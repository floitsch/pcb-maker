// Copyright (C) 2026 Toit contributors.

//! The machine's memory ledger: one JSON file that every route, layout
//! and KiCad check on the machine reserves its memory in before it
//! allocates, so that runs started side by side see what the others will
//! take, not only what they have taken so far.
//!
//! Until 2026-10-22 each run (and each harness) judged by the memory
//! available when it started; runs admitted together then grew into each
//! other and the OOM killer ended two rounds of them. The ledger holds a
//! reservation per process: its pid, when it reserved, its megabytes and
//! a label. A reservation of a process that is gone (its pid dead, or
//! reused by another process: the start time differs) is dropped on every
//! read, so a run killed by a signal or that panicked releases its
//! memory. Every update takes an exclusive flock on `<ledger>.lock`.
//!
//! A process (`arm`: the agent commands that route, lay out or check)
//! reserves before it allocates (`reserve_at_least`, `routers_side_by_side`,
//! `while_reserved`) and waits while the ledger's total and its request
//! exceed the machine's memory less a margin (`capacity_mb`). Its
//! reservation is a limit on itself too: a watchdog thread compares its
//! resident size with it, raises the reservation while the ledger has room
//! and otherwise ends the run with exit 4 and a verdict naming the
//! reservation, rather than leaving it to the OOM killer.
//!
//! Admission is ordered (since 2026-10-24, `order_allows`): a request
//! that waits is in the ledger with what it waits for and since when, and
//! a new request (a process that holds nothing yet, nor its harness) may
//! pass the request that has waited longest only with what still leaves
//! that one its room: while the oldest would fit now, room for both; while
//! it cannot fit, room for it beside every reservation that began holding
//! after it began waiting, so that it fits at the latest when the runs it
//! waits on end. Until then a large request waited while every smaller one
//! that came later took the memory as it freed (a designer-tier run: 3
//! rows in 75 minutes beside another agent's stream of small layouts). A
//! run that holds memory and grows is not held back (the oldest may be
//! waiting for it to end).
//!
//! The ledger drains for a starved waiter (since 2026-10-25,
//! `drain_allows`): once the first waiter has waited longer than its bound
//! (`drain_after`, default `DEFAULT_DRAIN_AFTER_SECONDS`), a new request
//! goes only if the waiter's room (the capacity less the reservations, and
//! the memory available less what the reservations still grow by and the
//! waiter's floor) still covers what it waits for after it, or if it is
//! expected to end before every run that holds memory (it does not delay
//! the drain). Until then a waiter held back by its harness's floor (the
//! nightly's designer boards of 17-27 GB, `--min-free-gb 10`) was passed
//! by a stream of 1-8 GB routes that never let the running set drain:
//! katia waited 33034 s. A request of the nightly (`priority`
//! `nightly`) goes first among the waiters and drains at once, without
//! the end exception: it is never passed once waiting.
//!
//! The drain's feasibility cap (since 2026-10-26, `blocked_by_large`):
//! while the waiter cannot fit before a run larger than
//! `SMALL_REQUEST_MB` ends (with every smaller run gone its room still
//! falls short), requests of at most that size go: they do not delay it.
//! The drain holds back the large ones until that run ends, then drains in
//! full. Until then the nightly's three designer boards of 23-28 GB
//! expected (with a floor of 10 GB, 33-38 GB of the 42 GB available on an
//! idle machine) ran one after the other, and each next one held every
//! other request back while the one before it ran.
//!
//! `PCB_MAKER_LEDGER` names the file (default
//! `$XDG_RUNTIME_DIR/pcb-maker-ledger.json`, else
//! `/tmp/pcb-maker-ledger.json`; `off`: no ledger),
//! `PCB_MAKER_LEDGER_MARGIN_MB` the margin (default 6144) and
//! `PCB_MAKER_LEDGER_TOTAL_MB` replaces the machine's memory less the
//! margin (tests). benchmarks/harness_common.py reads and writes the same
//! file.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// What a kicad-cli check takes beside the process (`kicad-cli pcb drc`,
/// 2.2 GB on any board: drc_cache.rs).
pub const KICAD_CHECK_MB: f64 = 2.2 * 1024.0;

/// The memory the ledger keeps free of reservations by default: other
/// work on the machine (editors, builds, the harnesses) is not in it.
pub const DEFAULT_MARGIN_MB: f64 = 6144.0;

/// How long the first waiter waits before newer requests are held to a
/// drain (`drain_allows`), unless its entry says otherwise
/// (`drain_after`; run.py's `--ledger-drain-after`).
pub const DEFAULT_DRAIN_AFTER_SECONDS: f64 = 600.0;

/// The priority of the nightly's requests (run.py's
/// `--ledger-priority nightly`): first among the waiters, never passed.
pub const PRIORITY_NIGHTLY: &str = "nightly";

/// The drain's feasibility cap (since 2026-10-26): while the waiter
/// cannot fit before a run larger than this ends (`blocked_by_large`),
/// requests of at most this many megabytes are not held back by the
/// drain (`drain_allows`).
pub const SMALL_REQUEST_MB: f64 = 4096.0;

/// How often a waiting process looks again, and how often it says so.
const WAIT_POLL: Duration = Duration::from_secs(2);
const WAIT_REPORT: Duration = Duration::from_secs(60);

/// How often the watchdog reads the resident size.
const WATCH_POLL: Duration = Duration::from_millis(200);

/// A reservation in the ledger.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Reservation {
    pub pid: u32,
    /// When the process first reserved (seconds since the epoch).
    pub started: f64,
    /// The process's start time in clock ticks after boot
    /// (/proc/<pid>/stat): a pid reused by another process is not this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ticks: Option<u64>,
    pub megabytes: f64,
    pub label: String,
    /// What the process waits to hold, while it waits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_mb: Option<f64>,
    /// When its wait began (seconds since the epoch), while it waits;
    /// `started` where absent (an older binary's entry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_since: Option<f64>,
    /// When it began holding memory (seconds since the epoch); `started`
    /// where absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_since: Option<f64>,
    /// While it waits: whether only the harness's floor of available
    /// memory holds it back (the ledger has its room). Newer requests then
    /// pass it as when it cannot fit (`order_allows`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub waits_on_floor: bool,
    /// While it waits: the floor of available memory its harness keeps
    /// beside it (`--min-free-gb`), for the drain (`drain_allows`); none
    /// where absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor_mb: Option<f64>,
    /// While it waits: how long it waits before newer requests are held to
    /// a drain (seconds; `DEFAULT_DRAIN_AFTER_SECONDS` where absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drain_after: Option<f64>,
    /// When the run is expected to end (seconds since the epoch; run.py's
    /// boards of a tier, from the tier's seconds), for the drain's end
    /// exception; unknown where absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_end: Option<f64>,
    /// `nightly` (`PRIORITY_NIGHTLY`): first among the waiters and never
    /// passed; none where absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Fields of newer writers, kept as they are when this binary rewrites
    /// the ledger.
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

impl Reservation {
    /// When its wait began (`waiting_since`, else `started`).
    pub fn waiting_since(&self) -> f64 {
        self.waiting_since.unwrap_or(self.started)
    }

    /// When it began holding memory (`held_since`, else `started`).
    pub fn held_since(&self) -> f64 {
        self.held_since.unwrap_or(self.started)
    }

    /// Whether it is the nightly's (`PRIORITY_NIGHTLY`).
    pub fn nightly(&self) -> bool {
        self.priority.as_deref() == Some(PRIORITY_NIGHTLY)
    }

    /// How long it waits before newer requests are held to a drain: 0 for
    /// the nightly's, else `drain_after` or the default.
    pub fn drain_after(&self) -> f64 {
        if self.nightly() {
            return 0.0;
        }
        self.drain_after
            .filter(|seconds| *seconds >= 0.0)
            .unwrap_or(DEFAULT_DRAIN_AFTER_SECONDS)
    }

    /// Its place among the waiters: the nightly's first, then by when the
    /// wait began, then by pid.
    pub fn order_key(&self) -> (u8, f64, u32) {
        (u8::from(!self.nightly()), self.waiting_since(), self.pid)
    }
}

#[derive(Serialize)]
struct LedgerFile {
    reservations: Vec<Reservation>,
}

/// The ledger file.
#[derive(Clone, Debug, PartialEq)]
pub struct Ledger {
    path: PathBuf,
}

/// The ledger's default place.
pub fn default_path() -> PathBuf {
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|directory| directory.is_dir())
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    directory.join("pcb-maker-ledger.json")
}

impl Ledger {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The ledger `PCB_MAKER_LEDGER` names (None: `off`).
    pub fn from_env() -> Option<Self> {
        match std::env::var("PCB_MAKER_LEDGER") {
            Ok(value) if value.trim() == "off" => None,
            Ok(value) if !value.trim().is_empty() => Some(Self::at(value.trim())),
            _ => Some(Self::at(default_path())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".lock");
        PathBuf::from(name)
    }

    /// Applies `change` to the live reservations under the ledger's lock
    /// and writes them back when they changed (the reservations of
    /// processes that are gone are dropped first).
    pub fn update<T>(&self, change: impl FnOnce(&mut Vec<Reservation>) -> T) -> io::Result<T> {
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.lock_path())?;
        let _held = Flock::exclusive(&lock)?;
        let stored = match fs::read_to_string(&self.path) {
            // Entry by entry: one that does not read is dropped, not the
            // others with it.
            Ok(text) => serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|value| value.get("reservations")?.as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|entry| serde_json::from_value::<Reservation>(entry).ok())
                .collect(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        let mut reservations: Vec<Reservation> = stored
            .iter()
            .filter(|reservation| alive(reservation.pid, reservation.start_ticks))
            .cloned()
            .collect();
        let outcome = change(&mut reservations);
        if reservations != stored {
            self.write(&reservations)?;
        }
        Ok(outcome)
    }

    /// The live reservations.
    pub fn live(&self) -> io::Result<Vec<Reservation>> {
        self.update(|reservations| reservations.clone())
    }

    fn write(&self, reservations: &[Reservation]) -> io::Result<()> {
        let text = serde_json::to_string_pretty(&LedgerFile {
            reservations: reservations.to_vec(),
        })
        .map_err(io::Error::other)?;
        let mut temporary = self.path.as_os_str().to_owned();
        temporary.push(format!(".{}.tmp", std::process::id()));
        let temporary = PathBuf::from(temporary);
        {
            let mut file = fs::File::create(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.write_all(b"\n")?;
        }
        fs::rename(&temporary, &self.path)
    }
}

/// An exclusive flock, released when dropped (no lock off Unix).
struct Flock<'a>(#[allow(dead_code)] &'a fs::File);

impl<'a> Flock<'a> {
    fn exclusive(file: &'a fs::File) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            loop {
                // SAFETY: flock takes a descriptor this File owns.
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                    break;
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
        Ok(Self(file))
    }
}

impl Drop for Flock<'_> {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            // SAFETY: as above.
            unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

/// The fields of /proc/<pid>/stat after the command (state first).
#[cfg(target_os = "linux")]
fn stat_fields(pid: u32) -> Option<Vec<String>> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = text.rsplit_once(')')?;
    Some(rest.split_whitespace().map(str::to_string).collect())
}

/// A process's start time in clock ticks after boot (Linux; else None).
pub fn start_ticks(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        stat_fields(pid)?.get(19)?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// Whether process `pid` lives (not a zombie) and, with `start_ticks`,
/// is the process that started then.
pub fn alive(pid: u32, ticks: Option<u64>) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Some(fields) = stat_fields(pid) else {
            return false;
        };
        if matches!(fields.first().map(String::as_str), Some("Z" | "X" | "x")) {
            return false;
        }
        ticks.is_none_or(|ticks| fields.get(19).and_then(|t| t.parse().ok()) == Some(ticks))
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let _ = ticks;
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
        || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, ticks);
        true
    }
}

/// The pids above this process up to init (Linux; else empty).
fn ancestors() -> Vec<u32> {
    let mut found = Vec::new();
    #[cfg(target_os = "linux")]
    {
        let mut pid = std::process::id();
        while pid > 1 {
            let Some(parent) = stat_fields(pid)
                .and_then(|fields| fields.get(1).and_then(|p| p.parse::<u32>().ok()))
            else {
                break;
            };
            if parent == 0 || found.contains(&parent) {
                break;
            }
            found.push(parent);
            pid = parent;
        }
    }
    found
}

/// A positive number of megabytes from the environment.
fn environment_mb(name: &str) -> Option<f64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|megabytes| megabytes.is_finite() && *megabytes > 0.0)
}

/// What the reservations may take together, in megabytes:
/// `PCB_MAKER_LEDGER_TOTAL_MB`, else the machine's memory less the margin
/// (`PCB_MAKER_LEDGER_MARGIN_MB`, default `DEFAULT_MARGIN_MB`). None when
/// the machine's memory is unknown (no waiting then).
pub fn capacity_mb() -> Option<f64> {
    if let Some(total) = environment_mb("PCB_MAKER_LEDGER_TOTAL_MB") {
        return Some(total);
    }
    let margin = std::env::var("PCB_MAKER_LEDGER_MARGIN_MB")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|megabytes| megabytes.is_finite() && *megabytes >= 0.0)
        .unwrap_or(DEFAULT_MARGIN_MB);
    crate::board_router::memory_total_mb().map(|total| (total - margin).max(0.0))
}

/// Whether process `me` (holding `held_mb`, started at `started`) may hold
/// `wanted_mb` now: the others' reservations and `wanted_mb` fit
/// `capacity`; or no other process holds memory (a request larger than
/// the capacity alone runs alone); or every other process that holds
/// memory waits too and `me` is the oldest of them that hold memory (two
/// runs that each wait to grow would wait for ever: the oldest goes on,
/// into the margin). Reservations of `ancestors` (a harness, a free
/// dimension's parent) wait for `me` and are left out of the last two.
pub fn may_hold(
    reservations: &[Reservation],
    me: u32,
    held_mb: f64,
    started: f64,
    wanted_mb: f64,
    capacity: Option<f64>,
    ancestors: &[u32],
) -> bool {
    let others: Vec<&Reservation> = reservations.iter().filter(|r| r.pid != me).collect();
    let reserved: f64 = others.iter().map(|r| r.megabytes).sum();
    if capacity.is_none_or(|capacity| reserved + wanted_mb <= capacity) {
        return true;
    }
    let holders: Vec<&&Reservation> = others
        .iter()
        .filter(|r| r.megabytes > 0.0 && !ancestors.contains(&r.pid))
        .collect();
    if holders.is_empty() {
        return true;
    }
    held_mb > 0.0
        && holders.iter().all(|r| r.waiting_mb.is_some())
        && holders.iter().all(|r| (r.started, r.pid) > (started, me))
}

/// Ordered admission: whether a new request of `wanted_mb` (a process
/// that holds nothing yet, nor any harness above it), waiting since
/// `since` (now when it has not waited) as process `me`, may be admitted
/// beside `others` (every reservation but its own) without passing the
/// request that has waited longest. No older waiter, or no capacity
/// known: yes. While the oldest waiter fits now (it has not polled yet),
/// only with room for both; while it cannot fit (or only the floor of
/// available memory holds it back, `waits_on_floor`), only with room for
/// it beside the reservations that began holding after it began waiting
/// (`held_since`) and this request: once the runs it waits on end, it
/// fits. A waiter larger than the capacity runs alone: no newer request
/// passes it. The first waiter is the nightly's where one waits
/// (`first_waiter`); a binary's own request is never the nightly's.
pub fn order_allows(
    others: &[&Reservation],
    me: u32,
    since: f64,
    wanted_mb: f64,
    capacity: Option<f64>,
) -> bool {
    let Some(capacity) = capacity else {
        return true;
    };
    let Some(oldest) = first_waiter(others) else {
        return true;
    };
    if oldest.order_key() >= (1, since, me) {
        return true;
    }
    let wants = oldest.waiting_mb.unwrap_or(0.0);
    let rest = || others.iter().filter(|r| !std::ptr::eq(**r, oldest));
    let reserved: f64 = rest().map(|r| r.megabytes).sum();
    let fits_now = reserved + wants <= capacity || !rest().any(|r| r.megabytes > 0.0);
    if fits_now && !oldest.waits_on_floor {
        return reserved + wanted_mb + wants <= capacity;
    }
    let newer: f64 = rest()
        .filter(|r| r.megabytes > 0.0 && r.held_since() >= oldest.waiting_since())
        .map(|r| r.megabytes)
        .sum();
    newer + wanted_mb + wants <= capacity
}

/// The waiting request the others let go first (`Reservation::order_key`:
/// the nightly's, then the one that has waited longest, then the lower
/// pid); None when nothing waits.
pub fn first_waiter<'a>(others: &[&'a Reservation]) -> Option<&'a Reservation> {
    others
        .iter()
        .copied()
        .filter(|r| r.waiting_mb.is_some())
        .min_by(|a, b| {
            a.order_key()
                .partial_cmp(&b.order_key())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// The waiter a new request of process `me`, waiting since `since`, is
/// held to a drain for at `now`: the first waiter (`first_waiter`), when it
/// comes before the request and has waited its `drain_after` (the
/// nightly's at once).
pub fn drained_for<'a>(
    others: &[&'a Reservation],
    me: u32,
    since: f64,
    now: f64,
) -> Option<&'a Reservation> {
    let waiter = first_waiter(others)?;
    (waiter.order_key() < (1, since, me) && now - waiter.waiting_since() >= waiter.drain_after())
        .then_some(waiter)
}

/// The drain (since 2026-10-25): whether a new request of `wanted_mb`,
/// expected to end at `expected_end` (None: unknown), may go beside
/// `others` while `waiter` (`drained_for`) is drained for. Only if after
/// it the waiter's room still covers what the waiter waits for: the
/// capacity less every other reservation (None: unbounded), and
/// `available_mb` (the memory available less what the reservations still
/// grow by, `available_less_growth`; None: unknown, unbounded) less the
/// waiter's floor (`floor_mb`; a waiter that holds memory already counts
/// it back); so the reservations shrink toward the waiter's room as runs
/// end. Or, except for the nightly's waiter, if it is expected to end no
/// later than every run that holds memory (each with an expected end):
/// it does not delay the drain. Or (the feasibility cap, since
/// 2026-10-26) if it is small (at most `SMALL_REQUEST_MB`) and the waiter
/// cannot fit before a larger run ends anyway (`blocked_by_large`):
/// holding it back gains the waiter nothing (a waiter of 23-28 GB behind a
/// 28 GB run held every other agent's request for hours).
pub fn drain_allows(
    waiter: &Reservation,
    others: &[&Reservation],
    wanted_mb: f64,
    capacity: Option<f64>,
    available_mb: Option<f64>,
    expected_end: Option<f64>,
) -> bool {
    drain_allows_up_to(
        SMALL_REQUEST_MB,
        waiter,
        others,
        wanted_mb,
        capacity,
        available_mb,
        expected_end,
    )
}

/// `drain_allows` with `small_mb` for `SMALL_REQUEST_MB` (0: no cap, the
/// rule of 2026-10-25).
fn drain_allows_up_to(
    small_mb: f64,
    waiter: &Reservation,
    others: &[&Reservation],
    wanted_mb: f64,
    capacity: Option<f64>,
    available_mb: Option<f64>,
    expected_end: Option<f64>,
) -> bool {
    let wants = waiter.waiting_mb.unwrap_or(0.0);
    let rest = || others.iter().filter(|r| !std::ptr::eq(**r, waiter));
    if waiter_room(waiter, others, capacity, available_mb, wanted_mb, 0.0) >= wants {
        return true;
    }
    if wanted_mb <= small_mb && blocked_by_large(small_mb, waiter, others, capacity, available_mb) {
        return true;
    }
    if waiter.nightly() {
        return false;
    }
    let Some(end) = expected_end else {
        return false;
    };
    let mut holders = rest().filter(|r| r.megabytes > 0.0).peekable();
    holders.peek().is_some() && holders.all(|r| r.expected_end.is_some_and(|their| end <= their))
}

/// The drain's room for `waiter` beside `others` (but the waiter) after a
/// request of `wanted_mb`, with `freed_mb` of the reservations gone: the
/// smaller of the capacity less the reservations (None: unbounded) and
/// `available_mb` (None: unbounded) less the waiter's floor (what the
/// waiter holds counts back).
fn waiter_room(
    waiter: &Reservation,
    others: &[&Reservation],
    capacity: Option<f64>,
    available_mb: Option<f64>,
    wanted_mb: f64,
    freed_mb: f64,
) -> f64 {
    let reserved: f64 = others
        .iter()
        .filter(|r| !std::ptr::eq(**r, waiter))
        .map(|r| r.megabytes)
        .sum::<f64>()
        - freed_mb;
    let ledger_room = capacity.map_or(f64::INFINITY, |capacity| capacity - reserved - wanted_mb);
    let floor_room = available_mb.map_or(f64::INFINITY, |available| {
        available + freed_mb - wanted_mb - waiter.floor_mb.unwrap_or(0.0).max(0.0)
            + waiter.megabytes
    });
    ledger_room.min(floor_room)
}

/// Whether `waiter` cannot fit before a run larger than `small_mb`
/// (`SMALL_REQUEST_MB`) ends: such a run holds memory, and with every smaller run gone the
/// waiter's room (`waiter_room`: the capacity and the memory available,
/// each with the smaller runs' megabytes back) still does not cover what
/// it waits for. The small runs then do not delay it: the drain lets them
/// go (`drain_allows`) and holds back only the large ones until that run
/// ends; then it drains in full. benchmarks/harness_common.py,
/// `blocked_by_large`: the same rule.
pub fn blocked_by_large(
    small_mb: f64,
    waiter: &Reservation,
    others: &[&Reservation],
    capacity: Option<f64>,
    available_mb: Option<f64>,
) -> bool {
    let holders = || {
        others
            .iter()
            .filter(|r| !std::ptr::eq(**r, waiter) && r.megabytes > 0.0)
    };
    if !holders().any(|r| r.megabytes > small_mb) {
        return false;
    }
    let small: f64 = holders()
        .filter(|r| r.megabytes <= small_mb)
        .map(|r| r.megabytes)
        .sum();
    waiter_room(waiter, others, capacity, available_mb, 0.0, small)
        < waiter.waiting_mb.unwrap_or(0.0)
}

/// The memory available now (MemAvailable) less what the reservations in
/// `others` still grow by (each one's megabytes less its process's
/// resident size, at least 0), in megabytes; None when unknown (off
/// Linux).
pub fn available_less_growth(others: &[&Reservation]) -> Option<f64> {
    let available = crate::board_router::memory_available_mb()?;
    let growth: f64 = others
        .iter()
        .filter(|r| r.megabytes > 0.0)
        .map(|r| (r.megabytes - crate::board_router::resident_mb_of(r.pid)).max(0.0))
        .sum();
    Some(available - growth)
}

/// The sum of the reservations' megabytes.
pub fn total_mb(reservations: &[Reservation]) -> f64 {
    // A fold from +0: an empty sum of f64 is -0.
    reservations
        .iter()
        .fold(0.0, |total, r| total + r.megabytes)
}

/// This process's part in the ledger.
struct Own {
    ledger: Ledger,
    label: String,
    started: f64,
    start_ticks: Option<u64>,
    ancestors: Vec<u32>,
    /// When its current wait began, while it waits (`order_allows`).
    waiting_since: Option<f64>,
    /// When it began holding memory.
    held_since: Option<f64>,
    /// What the process itself may hold resident (its hard limit).
    base_mb: f64,
    /// What its children hold beside it (kicad-cli's checks).
    extra_mb: f64,
    /// Whether the ledger could not be written (said once; the run goes
    /// on without it).
    broken: bool,
    watching: bool,
}

static OWN: Mutex<Option<Own>> = Mutex::new(None);

/// What ends the run when it grows past its reservation and the ledger has
/// no room (the executable writes the verdict and exits 4).
static EXCEEDED: OnceLock<fn(&str)> = OnceLock::new();

/// Sets what ends the run when it grows past its reservation.
pub fn set_exceeded_handler(handler: fn(&str)) {
    let _ = EXCEEDED.set(handler);
}

fn own() -> std::sync::MutexGuard<'static, Option<Own>> {
    OWN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// Makes this process reserve through the ledger (`PCB_MAKER_LEDGER`, not
/// `off`) under `label`; until then reserving does nothing (the library's
/// callers and tests). Reserves nothing yet.
pub fn arm(label: &str) {
    let Some(ledger) = Ledger::from_env() else {
        return;
    };
    let pid = std::process::id();
    *own() = Some(Own {
        ledger,
        label: label.to_string(),
        started: now_seconds(),
        start_ticks: start_ticks(pid),
        ancestors: ancestors(),
        waiting_since: None,
        held_since: None,
        base_mb: 0.0,
        extra_mb: 0.0,
        broken: false,
        watching: false,
    });
}

/// Whether this process reserves through the ledger.
pub fn armed() -> bool {
    own().is_some()
}

/// This process's reservation (its own limit and its children's), when
/// armed.
pub fn reserved_mb() -> Option<(f64, f64)> {
    own().as_ref().map(|own| (own.base_mb, own.extra_mb))
}

/// What one try to hold `base_mb` and `extra_mb` found.
enum Try {
    Held,
    Waiting {
        reserved_mb: f64,
        others: usize,
        /// The older request it lets go first (`order_allows`), for the
        /// log.
        behind: Option<String>,
    },
    Unarmed,
}

/// Tries once to set this process's reservation to `base_mb` for itself
/// and `extra_mb` for its children; with `wait`, records the wish in the
/// ledger when it does not fit (`pcb-maker ledger` shows it).
fn try_hold(base_mb: f64, extra_mb: f64, wait: bool) -> Try {
    let mut guard = own();
    let Some(own) = guard.as_mut() else {
        return Try::Unarmed;
    };
    // The process's own limit only grows: the watchdog may have raised it
    // while the caller waited for less.
    let base_mb = base_mb.max(own.base_mb);
    if own.broken {
        own.base_mb = base_mb;
        own.extra_mb = extra_mb;
        return Try::Held;
    }
    let me = std::process::id();
    let wanted = base_mb + extra_mb;
    let capacity = capacity_mb();
    let held = own.base_mb + own.extra_mb;
    let now = now_seconds();
    let since = own.waiting_since.unwrap_or(now);
    // What the entry it replaces (a harness's reservation for this process,
    // or its own) says of the run beyond the memory: kept.
    let entry = |megabytes: f64,
                 waiting_mb: Option<f64>,
                 held_since: Option<f64>,
                 inherited: Option<&Reservation>| Reservation {
        pid: me,
        started: own.started,
        start_ticks: own.start_ticks,
        megabytes,
        label: own.label.clone(),
        waiting_mb,
        waiting_since: waiting_mb.map(|_| since),
        held_since: held_since.filter(|_| megabytes > 0.0),
        expected_end: inherited.and_then(|r| r.expected_end),
        priority: inherited.and_then(|r| r.priority.clone()),
        other: inherited.map(|r| r.other.clone()).unwrap_or_default(),
        ..Default::default()
    };
    let ancestors = &own.ancestors;
    let held_since = own.held_since;
    let outcome = own.ledger.update(|reservations| {
        // A harness's reservation for this process (re-pointed to its
        // pid), or one of a process above it: admitted already.
        let harness = reservations
            .iter()
            .filter(|r| (r.pid == me || ancestors.contains(&r.pid)) && r.megabytes > 0.0)
            .map(Reservation::held_since)
            .fold(None, |first: Option<f64>, at| {
                Some(first.map_or(at, |first| first.min(at)))
            });
        let fits = may_hold(
            reservations,
            me,
            held,
            own.started,
            wanted,
            capacity,
            ancestors,
        );
        let others: Vec<&Reservation> = reservations.iter().filter(|r| r.pid != me).collect();
        let reserved: f64 = others.iter().map(|r| r.megabytes).sum();
        let count = others.len();
        // Ordered admission and the drain for a new request only: a run
        // that holds memory grows as before (the oldest waiter may wait for
        // it to end). A binary does not know when it will end: in a drain
        // it goes by the waiter's room alone.
        let new_request = held <= 0.0 && harness.is_none();
        let drained = new_request
            .then(|| drained_for(&others, me, since, now))
            .flatten();
        let ordered = !new_request
            || (order_allows(&others, me, since, wanted, capacity)
                && drained.is_none_or(|waiter| {
                    drain_allows(
                        waiter,
                        &others,
                        wanted,
                        capacity,
                        available_less_growth(&others),
                        None,
                    )
                }));
        let behind = (fits && !ordered)
            .then(|| first_waiter(&others))
            .flatten()
            .map(|r| {
                format!(
                    "{} (pid {}, waiting {:.0} s for {:.0} MB{})",
                    r.label,
                    r.pid,
                    (now - r.waiting_since()).max(0.0),
                    r.waiting_mb.unwrap_or(0.0),
                    if drained.is_some() {
                        "; the ledger drains for it"
                    } else {
                        ""
                    }
                )
            });
        let fits = fits && ordered;
        let held_since = held_since.or(harness).unwrap_or(now);
        // A harness's reservation for this process gives way to the
        // process's own.
        let inherited = reservations.iter().find(|r| r.pid == me).cloned();
        reservations.retain(|r| r.pid != me);
        if fits {
            reservations.push(entry(wanted, None, Some(held_since), inherited.as_ref()));
        } else if wait || held > 0.0 {
            reservations.push(entry(
                held,
                wait.then_some(wanted),
                Some(held_since),
                inherited.as_ref(),
            ));
        }
        (fits, reserved, count, behind, held_since)
    });
    match outcome {
        Ok((true, _, _, _, held_since)) => {
            own.base_mb = base_mb;
            own.extra_mb = extra_mb;
            own.waiting_since = None;
            if wanted > 0.0 {
                own.held_since.get_or_insert(held_since);
            }
            Try::Held
        }
        Ok((false, reserved_mb, others, behind, _)) => {
            if wait {
                own.waiting_since = Some(since);
            }
            Try::Waiting {
                reserved_mb,
                others,
                behind,
            }
        }
        Err(error) => {
            eprintln!(
                "memory ledger: {} cannot be written ({error}); the run goes on without it",
                own.ledger.path.display()
            );
            own.broken = true;
            own.base_mb = base_mb;
            own.extra_mb = extra_mb;
            Try::Held
        }
    }
}

/// Holds `base_mb` and `extra_mb`, waiting while the ledger has no room,
/// and says so in the log (`why`: what the memory is for).
fn hold(base_mb: f64, extra_mb: f64, why: &str) {
    let started = Instant::now();
    let mut reported: Option<Instant> = None;
    loop {
        match try_hold(base_mb, extra_mb, true) {
            Try::Held | Try::Unarmed => break,
            Try::Waiting {
                reserved_mb,
                others,
                behind,
            } => {
                if reported.is_none_or(|at| at.elapsed() >= WAIT_REPORT) {
                    let capacity = capacity_mb().unwrap_or(f64::INFINITY);
                    eprintln!(
                        "memory ledger: waiting to reserve {:.0} MB for {why}: {reserved_mb:.0} MB of {capacity:.0} MB reserved by {others} other run{}{} (pcb-maker ledger lists them; waited {:.0} s)",
                        base_mb + extra_mb,
                        if others == 1 { "" } else { "s" },
                        behind.map_or(String::new(), |behind| format!(
                            "; behind an older request, {behind}"
                        )),
                        started.elapsed().as_secs_f64()
                    );
                    reported = Some(Instant::now());
                }
                std::thread::sleep(WAIT_POLL);
            }
        }
    }
    if reported.is_some() {
        eprintln!(
            "memory ledger: reserved {:.0} MB for {why} after waiting {:.0} s",
            base_mb + extra_mb,
            started.elapsed().as_secs_f64()
        );
    }
    start_watchdog();
}

/// Raises this process's own reservation to at least `megabytes` (its
/// resident size, the hard limit the watchdog keeps), waiting while the
/// ledger has no room. Never lowers it. Nothing when not armed.
pub fn reserve_at_least(megabytes: f64, why: &str) {
    let Some((base, extra)) = reserved_mb() else {
        return;
    };
    if megabytes <= base || !megabytes.is_finite() {
        return;
    }
    hold(megabytes.ceil(), extra, why);
}

/// How many of `wanted` routers of about `each_mb` each, with `fixed_mb`
/// beside them once, run side by side beside what the process holds now:
/// as many as the ledger has room for, at least one (waiting for that
/// one's room), with the reservation raised to hold them. `wanted` when
/// not armed.
pub fn routers_side_by_side(fixed_mb: f64, each_mb: f64, wanted: usize, why: &str) -> usize {
    let wanted = wanted.max(1);
    let Some((base, extra)) = reserved_mb() else {
        return wanted;
    };
    let resident = crate::board_router::resident_mb() + fixed_mb.max(0.0);
    let each = each_mb.max(1.0);
    let room = room_mb().map_or(f64::INFINITY, |room| room - extra);
    let count = (1..=wanted)
        .rev()
        .find(|count| (resident + *count as f64 * each).max(base) <= room.max(base))
        .unwrap_or(1);
    reserve_at_least(resident + count as f64 * each, why);
    count
}

/// The most this process could hold now without waiting: the capacity
/// less the others' reservations (None: no capacity known, or not armed).
fn room_mb() -> Option<f64> {
    let guard = own();
    let own = guard.as_ref()?;
    if own.broken {
        return None;
    }
    let capacity = capacity_mb()?;
    let me = std::process::id();
    let others = own
        .ledger
        .live()
        .ok()?
        .iter()
        .filter(|r| r.pid != me)
        .map(|r| r.megabytes)
        .sum::<f64>();
    Some(capacity - others)
}

/// Runs `work` with `megabytes` more reserved for a child process (a
/// kicad-cli check), waiting for the room first; released afterwards.
pub fn while_reserved<T>(megabytes: f64, why: &str, work: impl FnOnce() -> T) -> T {
    let Some((base, extra)) = reserved_mb() else {
        return work();
    };
    hold(base, extra + megabytes, why);
    let outcome = work();
    if let Some((base, extra)) = reserved_mb() {
        let _ = try_hold(base, (extra - megabytes).max(0.0), false);
    }
    outcome
}

/// Removes this process's reservation (at exit; a process that dies
/// without it loses it by the pid rule).
pub fn release() {
    let guard = own();
    if let Some(own) = guard.as_ref().filter(|own| !own.broken) {
        let me = std::process::id();
        let _ = own
            .ledger
            .update(|reservations| reservations.retain(|reservation| reservation.pid != me));
    }
}

/// Starts the thread that holds the process to its reservation (once).
fn start_watchdog() {
    {
        let mut guard = own();
        match guard.as_mut() {
            Some(own) if !own.watching => own.watching = true,
            _ => return,
        }
    }
    let _ = std::thread::Builder::new()
        .name("memory-ledger".into())
        .spawn(|| {
            loop {
                std::thread::sleep(WATCH_POLL);
                let Some((base, extra)) = reserved_mb() else {
                    return;
                };
                let resident = crate::board_router::resident_mb();
                if base <= 0.0 || resident < base * 0.9 {
                    continue;
                }
                // Growth the estimate did not foresee: more while the
                // ledger has room, the end of the run when it has none.
                let raised = (resident * 1.2).max(base * 1.2).ceil();
                if matches!(try_hold(raised, extra, false), Try::Held) || resident <= base {
                    continue;
                }
                let capacity = capacity_mb().unwrap_or(f64::INFINITY);
                let path = own()
                    .as_ref()
                    .map(|own| own.ledger.path.display().to_string())
                    .unwrap_or_default();
                let message = format!(
                    "memory: the process grew to {resident:.0} MB resident, past its reservation of {base:.0} MB in the memory ledger ({path}), and the ledger had no room for more ({capacity:.0} MB for every run; pcb-maker ledger lists them); rerun when fewer runs share the machine"
                );
                if let Some(handler) = EXCEEDED.get() {
                    handler(&message);
                }
                eprintln!("error: {message}");
                release();
                std::process::exit(crate::EXIT_INTERNAL as i32);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "pcb-maker-ledger-{name}-{}-{}",
            std::process::id(),
            now_seconds()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn reservation(pid: u32, megabytes: f64, label: &str) -> Reservation {
        Reservation {
            pid,
            started: now_seconds(),
            start_ticks: start_ticks(pid),
            megabytes,
            label: label.into(),
            ..Default::default()
        }
    }

    /// A pid no process has: past the kernel's limit.
    const DEAD: u32 = 4_194_304 + 7;

    #[test]
    fn reservations_of_dead_or_reused_pids_are_dropped_on_read() {
        let directory = scratch("stale");
        let ledger = Ledger::at(directory.join("ledger.json"));
        let me = std::process::id();
        ledger
            .update(|reservations| {
                reservations.push(reservation(me, 100.0, "this test"));
                reservations.push(reservation(DEAD, 200.0, "a dead run"));
            })
            .unwrap();
        // The dead pid's is in the file until the next read drops it.
        let text = fs::read_to_string(ledger.path()).unwrap();
        assert!(text.contains("a dead run"), "{text}");
        let live = ledger.live().unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].label, "this test");
        assert!(
            !fs::read_to_string(ledger.path())
                .unwrap()
                .contains("a dead run")
        );
        // The pid alive but another start time: a reused pid.
        if cfg!(target_os = "linux") {
            ledger
                .update(|reservations| {
                    let mut reused = reservation(me, 300.0, "a reused pid");
                    reused.start_ticks = reused.start_ticks.map(|ticks| ticks + 1);
                    reservations.push(reused);
                })
                .unwrap();
            let live = ledger.live().unwrap();
            assert_eq!(live.len(), 1, "{live:?}");
            assert!(!alive(DEAD, None));
            assert!(alive(me, start_ticks(me)));
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn concurrent_updates_under_the_lock_lose_nothing() {
        let directory = scratch("concurrent");
        let path = directory.join("ledger.json");
        let me = std::process::id();
        let threads: Vec<_> = (0..8)
            .map(|thread| {
                let ledger = Ledger::at(&path);
                std::thread::spawn(move || {
                    for step in 0..25 {
                        ledger
                            .update(|reservations| {
                                // A read, a pause and a write: without the
                                // lock another thread's write in between is
                                // lost.
                                let mut copy = reservations.clone();
                                std::thread::yield_now();
                                copy.push(reservation(me, 1.0, &format!("{thread}-{step}")));
                                *reservations = copy;
                            })
                            .unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let live = Ledger::at(&path).live().unwrap();
        assert_eq!(live.len(), 200);
        assert_eq!(total_mb(&live), 200.0);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_request_waits_while_the_total_exceeds_the_capacity() {
        let other = |pid: u32, megabytes: f64, started: f64, waiting: bool| Reservation {
            pid,
            started,
            start_ticks: None,
            megabytes,
            label: String::new(),
            waiting_mb: waiting.then_some(megabytes * 2.0),
            ..Default::default()
        };
        let me = 10;
        let reservations = vec![other(11, 3000.0, 5.0, false), other(12, 3000.0, 6.0, false)];
        // 6000 reserved: 2000 more fit 8000, 2001 do not.
        assert!(may_hold(
            &reservations,
            me,
            0.0,
            9.0,
            2000.0,
            Some(8000.0),
            &[]
        ));
        assert!(!may_hold(
            &reservations,
            me,
            0.0,
            9.0,
            2001.0,
            Some(8000.0),
            &[]
        ));
        // No capacity known: no waiting.
        assert!(may_hold(&reservations, me, 0.0, 9.0, 1e9, None, &[]));
        // A request larger than the capacity runs alone; an ancestor's
        // reservation (the harness's) does not stop it.
        assert!(may_hold(&[], me, 0.0, 9.0, 9000.0, Some(8000.0), &[]));
        assert!(may_hold(
            &[other(1, 500.0, 1.0, false)],
            me,
            0.0,
            9.0,
            9000.0,
            Some(8000.0),
            &[1]
        ));
        // This process's own entry does not count against it.
        assert!(may_hold(
            &[other(me, 7000.0, 9.0, false)],
            me,
            7000.0,
            9.0,
            8000.0,
            Some(8000.0),
            &[]
        ));
        // Two runs that hold memory and both wait to grow: the older goes on.
        let waiting = vec![other(11, 3000.0, 5.0, true)];
        assert!(may_hold(
            &waiting,
            me,
            3000.0,
            4.0,
            6000.0,
            Some(8000.0),
            &[]
        ));
        assert!(!may_hold(
            &waiting,
            me,
            3000.0,
            6.0,
            6000.0,
            Some(8000.0),
            &[]
        ));
        // One that holds nothing yet never goes past the capacity.
        assert!(!may_hold(&waiting, me, 0.0, 4.0, 6000.0, Some(8000.0), &[]));
        // Another holder that does not wait will finish: wait for it.
        let mixed = vec![other(11, 3000.0, 5.0, true), other(12, 1000.0, 7.0, false)];
        assert!(!may_hold(
            &mixed,
            me,
            3000.0,
            4.0,
            6000.0,
            Some(8000.0),
            &[]
        ));
    }

    fn refs(all: &[Reservation]) -> Vec<&Reservation> {
        all.iter().collect()
    }

    #[test]
    fn the_request_that_waited_longest_is_not_passed() {
        // Holders: 11 since 1 s (before the waiter), 12 since 20 s (after).
        let holder = |pid: u32, megabytes: f64, held_since: f64| Reservation {
            pid,
            started: held_since,
            megabytes,
            held_since: Some(held_since),
            ..Default::default()
        };
        let waiter = |pid: u32, wants: f64, since: f64| Reservation {
            pid,
            started: since,
            waiting_mb: Some(wants),
            waiting_since: Some(since),
            ..Default::default()
        };
        let capacity = Some(10_000.0);
        // No waiter, or no capacity known: as before.
        let holders = [holder(11, 6000.0, 1.0)];
        assert!(order_allows(&refs(&holders), 99, 30.0, 4000.0, capacity));
        let large = waiter(13, 8000.0, 10.0);
        let with_large = [holder(11, 6000.0, 1.0), large.clone()];
        assert!(order_allows(&refs(&with_large), 99, 30.0, 4000.0, None));
        // The large request (8000 MB) waits for 11's 6000 to end. A newer
        // request of 2000 fits the capacity (6000 + 2000) and leaves the
        // large one its room once 11 ends (2000 + 8000): it goes.
        assert!(order_allows(&refs(&with_large), 99, 30.0, 2000.0, capacity));
        // 2001 would leave it short when 11 ends: it waits.
        assert!(!order_allows(
            &refs(&with_large),
            99,
            30.0,
            2001.0,
            capacity
        ));
        // The newer runs admitted while it waits count: with 12's 1500
        // (holding since 20 s, after the wait began) only 500 more go.
        let crowded = [
            holder(11, 6000.0, 1.0),
            holder(12, 1500.0, 20.0),
            large.clone(),
        ];
        assert!(order_allows(&refs(&crowded), 99, 30.0, 500.0, capacity));
        assert!(!order_allows(&refs(&crowded), 99, 30.0, 501.0, capacity));
        // A request that waits longer than the large one goes by the
        // capacity alone (`may_hold`).
        assert!(order_allows(&refs(&crowded), 99, 5.0, 9000.0, capacity));
        // Once 11 ends the large one fits now: the next slot is its own,
        // and a newer request only goes with room for both.
        let freed = [holder(12, 1500.0, 20.0), large.clone()];
        assert!(order_allows(&refs(&freed), 99, 30.0, 500.0, capacity));
        assert!(!order_allows(&refs(&freed), 99, 30.0, 501.0, capacity));
        // Held back by its harness's floor of available memory only: newer
        // requests pass as while it cannot fit (they need not wait for a
        // floor the ledger cannot see).
        // (11's 1000 since before the wait: room for both 500, after 11
        // ends 1500.)
        let fits = [
            holder(11, 1000.0, 1.0),
            holder(12, 500.0, 20.0),
            large.clone(),
        ];
        assert!(!order_allows(&refs(&fits), 99, 30.0, 1000.0, capacity));
        let mut floored = large.clone();
        floored.waits_on_floor = true;
        let floor = [holder(11, 1000.0, 1.0), holder(12, 500.0, 20.0), floored];
        assert!(order_allows(&refs(&floor), 99, 30.0, 1500.0, capacity));
        assert!(!order_allows(&refs(&floor), 99, 30.0, 1501.0, capacity));
        // Of two waiters the older one is protected; equal times: the
        // lower pid.
        let two = [
            holder(11, 6000.0, 1.0),
            waiter(14, 3000.0, 12.0),
            large.clone(),
        ];
        assert!(!order_allows(&refs(&two), 99, 30.0, 2500.0, capacity));
        let tie = [holder(11, 7000.0, 1.0), waiter(13, 2000.0, 10.0)];
        assert!(!order_allows(&refs(&tie), 14, 10.0, 1001.0, capacity));
        assert!(order_allows(&refs(&tie), 12, 10.0, 1001.0, capacity));
        // A waiter larger than the capacity runs alone: nothing newer
        // passes it, also when nothing else holds memory.
        let huge = [holder(11, 1000.0, 1.0), waiter(13, 12_000.0, 10.0)];
        assert!(!order_allows(&refs(&huge), 99, 30.0, 1.0, capacity));
        let huge_alone = [waiter(13, 12_000.0, 10.0)];
        assert!(!order_allows(&refs(&huge_alone), 99, 30.0, 1.0, capacity));
        // A holder waiting to grow (3000 held, 9000 wanted) is the oldest
        // waiter: its own memory is part of what it waits for.
        let mut growing = holder(15, 3000.0, 1.0);
        growing.waiting_mb = Some(9000.0);
        growing.waiting_since = Some(8.0);
        let grow = [growing];
        assert!(order_allows(&refs(&grow), 99, 30.0, 1000.0, capacity));
        assert!(!order_allows(&refs(&grow), 99, 30.0, 1001.0, capacity));
        // Entries of older binaries (no waiting_since, held_since): their
        // `started` stands in.
        let old = [
            Reservation {
                pid: 11,
                started: 1.0,
                megabytes: 6000.0,
                ..Default::default()
            },
            Reservation {
                pid: 13,
                started: 10.0,
                waiting_mb: Some(8000.0),
                ..Default::default()
            },
        ];
        assert!(order_allows(&refs(&old), 99, 30.0, 2000.0, capacity));
        assert!(!order_allows(&refs(&old), 99, 30.0, 2001.0, capacity));
    }

    /// The machine of 2026-10-25, simulated second by second: a ledger of
    /// 56 000 MB, 30 000 MB available beside the reservations (each run
    /// resident at its reservation), a stream of binaries' requests of
    /// 2000 MB, one every 5 s, each running 60 s once admitted and looking
    /// every 2 s while it waits, and from `ARRIVES` s a harness's request
    /// of 19 000 MB that keeps a floor of 10 000 MB (katia), looking every
    /// 15 s. Returns how long the large request waited (None: it did not
    /// start within `horizon` seconds) and how many small ones ran.
    fn starve(drain_after: f64, horizon: u32) -> (Option<u32>, usize) {
        const CAPACITY: f64 = 56_000.0;
        const FREE: f64 = 30_000.0;
        const FLOOR: f64 = 10_000.0;
        const LARGE: f64 = 19_000.0;
        const SMALL: f64 = 2000.0;
        const ARRIVES: u32 = 300;
        let large_pid = 1;
        // (reservation, end of its run once it holds)
        let mut book: Vec<(Reservation, Option<u32>)> = Vec::new();
        let mut large_started = None;
        let mut small_ran = 0;
        let mut next_pid = 100;
        for t in 0..ARRIVES + horizon {
            let now = f64::from(t);
            book.retain(|(_, end)| end.is_none_or(|end| end > t));
            if t == ARRIVES {
                book.push((
                    Reservation {
                        pid: large_pid,
                        started: now,
                        label: "katia".into(),
                        waiting_mb: Some(LARGE),
                        waiting_since: Some(now),
                        waits_on_floor: true,
                        floor_mb: Some(FLOOR),
                        drain_after: Some(drain_after),
                        ..Default::default()
                    },
                    None,
                ));
            }
            if t.is_multiple_of(5) && t > 0 {
                next_pid += 1;
                book.push((
                    Reservation {
                        pid: next_pid,
                        started: now,
                        label: "route".into(),
                        waiting_mb: Some(SMALL),
                        waiting_since: Some(now),
                        ..Default::default()
                    },
                    None,
                ));
            }
            // Who looks this second, in the order of their pids.
            let lookers: Vec<u32> = book
                .iter()
                .filter(|(r, _)| r.waiting_mb.is_some())
                .filter(|(r, _)| {
                    let waited = t - r.waiting_since() as u32;
                    if r.pid == large_pid {
                        waited.is_multiple_of(15)
                    } else {
                        waited.is_multiple_of(2)
                    }
                })
                .map(|(r, _)| r.pid)
                .collect();
            for pid in lookers {
                let index = book.iter().position(|(r, _)| r.pid == pid).unwrap();
                let me = book[index].0.clone();
                let wanted = me.waiting_mb.unwrap();
                let others: Vec<&Reservation> = book
                    .iter()
                    .map(|(r, _)| r)
                    .filter(|r| r.pid != pid)
                    .collect();
                let reserved: f64 = others.iter().map(|r| r.megabytes).sum();
                let available = FREE - reserved;
                let admitted = if pid == large_pid {
                    // The harness's rule (harness_common.memory_room): the
                    // ledger and the floor, or nothing held.
                    (reserved + wanted <= CAPACITY && available - wanted >= FLOOR)
                        || !others.iter().any(|r| r.megabytes > 0.0)
                } else {
                    // A binary's (try_hold): the capacity, the order and
                    // the drain.
                    let since = me.waiting_since();
                    may_hold(
                        &book.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(),
                        pid,
                        0.0,
                        me.started,
                        wanted,
                        Some(CAPACITY),
                        &[],
                    ) && order_allows(&others, pid, since, wanted, Some(CAPACITY))
                        && drained_for(&others, pid, since, now).is_none_or(|waiter| {
                            drain_allows(
                                waiter,
                                &others,
                                wanted,
                                Some(CAPACITY),
                                Some(available),
                                None,
                            )
                        })
                };
                if admitted {
                    let entry = &mut book[index];
                    entry.0.megabytes = wanted;
                    entry.0.waiting_mb = None;
                    entry.0.waiting_since = None;
                    entry.0.held_since = Some(now);
                    if pid == large_pid {
                        large_started = Some(t - ARRIVES);
                        break;
                    } else {
                        small_ran += 1;
                        entry.1 = Some(t + 60);
                    }
                }
            }
            if large_started.is_some() {
                break;
            }
        }
        (large_started, small_ran)
    }

    #[test]
    fn a_stream_of_small_requests_starves_a_waiter_until_the_ledger_drains() {
        // Without the drain (the rule of 2026-10-24: a waiter held back by
        // its floor is passed as long as the capacity has its room) the
        // small ones keep about 12 runs and 24 000 MB going: the large one
        // never sees its 29 000 MB, in four hours.
        let (started, ran) = starve(f64::INFINITY, 4 * 3600);
        assert_eq!(started, None);
        assert!(ran > 2800, "{ran} small runs");
        // With the drain after 600 s: no newer request goes once 600 s have
        // passed (none leaves the large one its room), the runs end within
        // 60 s, the large one goes alone at its next look.
        let (started, ran) = starve(600.0, 4 * 3600);
        let started = started.expect("the large request starts");
        assert!(
            (600..=600 + 60 + 15).contains(&started),
            "started at {started} s"
        );
        // (60 before it began waiting, 119 until 600 s, none since.)
        assert_eq!(ran, 179, "{ran} small runs");
        // The default bound for an entry that names none.
        assert_eq!(
            Reservation::default().drain_after(),
            DEFAULT_DRAIN_AFTER_SECONDS
        );
    }

    /// The machine of 2026-10-25 22:30, second by second: a ledger of
    /// 58 066 MB, 42 000 MB available beside the reservations (each run
    /// resident at its reservation), a run of 8800 MB holding until 900 s,
    /// three nightly boards waiting from 0 s (27 876, 24 724 and 23 215 MB
    /// expected, a floor of 10 240 MB, looking every 15 s, each running
    /// 1500 s) and three agents' chains of binaries (2000 MB, 120 s each,
    /// the next one as one ends, looking every 2 s), for `horizon` seconds.
    /// `small_mb`: the drain's small-request bound (0: the rule of
    /// 2026-10-25). Returns the nightly boards' starts, the binaries'
    /// longest wait and how many ran. test_run.py's `tonight`: the same.
    fn tonight(small_mb: f64, horizon: u32) -> (Vec<u32>, u32, usize) {
        const CAPACITY: f64 = 58_066.0;
        const FREE: f64 = 42_000.0;
        const FLOOR: f64 = 10_240.0;
        // (reservation, end once it holds, the binaries' chain)
        let mut book: Vec<(Reservation, Option<u32>, Option<u32>)> = vec![(
            Reservation {
                pid: 2,
                megabytes: 8800.0,
                label: "layout".into(),
                held_since: Some(0.0),
                expected_end: Some(900.0),
                ..Default::default()
            },
            Some(900),
            None,
        )];
        for (pid, megabytes) in [(11, 27_876.0), (12, 24_724.0), (13, 23_215.0)] {
            book.push((
                Reservation {
                    pid,
                    label: "nightly".into(),
                    waiting_mb: Some(megabytes),
                    waiting_since: Some(0.0),
                    floor_mb: Some(FLOOR),
                    waits_on_floor: true,
                    priority: Some(PRIORITY_NIGHTLY.into()),
                    drain_after: Some(0.0),
                    ..Default::default()
                },
                None,
                None,
            ));
        }
        let (mut starts, mut longest, mut ran, mut next_pid) = (Vec::new(), 0, 0, 100);
        for t in 0..horizon {
            let now = f64::from(t);
            book.retain(|(_, end, _)| end.is_none_or(|end| end > t));
            for chain in 0..3 {
                if !book.iter().any(|(_, _, c)| *c == Some(chain)) {
                    next_pid += 1;
                    book.push((
                        Reservation {
                            pid: next_pid,
                            started: now,
                            label: "route".into(),
                            waiting_mb: Some(2000.0),
                            waiting_since: Some(now),
                            ..Default::default()
                        },
                        None,
                        Some(chain),
                    ));
                }
            }
            let lookers: Vec<u32> = book
                .iter()
                .filter(|(r, _, _)| r.waiting_mb.is_some())
                .filter(|(r, _, _)| {
                    (t - r.waiting_since() as u32).is_multiple_of(if r.nightly() { 15 } else { 2 })
                })
                .map(|(r, _, _)| r.pid)
                .collect();
            for pid in lookers {
                let index = book.iter().position(|(r, _, _)| r.pid == pid).unwrap();
                let me = book[index].0.clone();
                let (wanted, since, nightly) =
                    (me.waiting_mb.unwrap(), me.waiting_since(), me.nightly());
                let others: Vec<&Reservation> = book
                    .iter()
                    .map(|(r, _, _)| r)
                    .filter(|r| r.pid != pid)
                    .collect();
                let reserved: f64 = others.iter().map(|r| r.megabytes).sum();
                let available = FREE - reserved;
                let fits = if nightly {
                    // The harness's rule (harness_common.memory_room).
                    (reserved + wanted <= CAPACITY && available - wanted >= FLOOR)
                        || !others.iter().any(|r| r.megabytes > 0.0)
                } else {
                    reserved + wanted <= CAPACITY || !others.iter().any(|r| r.megabytes > 0.0)
                };
                let end = nightly.then(|| now + 1500.0);
                // A binary's request is never the nightly's (order_allows
                // and drained_for rank it after them); the harness's
                // nightly boards rank by their priority: the first of them
                // is behind no one.
                let first = first_waiter(&book.iter().map(|(r, _, _)| r).collect::<Vec<_>>())
                    .is_some_and(|r| r.pid == pid);
                let admitted = fits
                    && (first || order_allows(&others, pid, since, wanted, Some(CAPACITY)))
                    && (first
                        || drained_for(&others, pid, since, now).is_none_or(|waiter| {
                            drain_allows_up_to(
                                small_mb,
                                waiter,
                                &others,
                                wanted,
                                Some(CAPACITY),
                                Some(available),
                                end,
                            )
                        }));
                if admitted {
                    let entry = &mut book[index];
                    entry.0.megabytes = wanted;
                    entry.0.waiting_mb = None;
                    entry.0.waiting_since = None;
                    entry.0.held_since = Some(now);
                    entry.0.expected_end = end;
                    entry.1 = Some(t + if nightly { 1500 } else { 120 });
                    if nightly {
                        starts.push(t);
                    } else {
                        longest = longest.max(t - since as u32);
                        ran += 1;
                    }
                }
            }
        }
        (starts, longest, ran)
    }

    #[test]
    fn small_requests_go_while_the_waiter_waits_for_a_large_run_to_end() {
        // The rule of 2026-10-25: a binary waits up to 3900 s (from the
        // first board's end to the third board's start every request is
        // held: each board needs 33-38 GB available, the machine has 42).
        assert_eq!(tonight(0.0, 4 * 3600), (vec![900, 2400, 3900], 3900, 264));
        // The feasibility cap: the binaries go beside the large runs (the
        // next board cannot fit before the running one ends) and wait at
        // most one of their own runs (the full drain before each board);
        // the boards start 60 s later.
        assert_eq!(
            tonight(SMALL_REQUEST_MB, 4 * 3600),
            (vec![960, 2460, 3960], 120, 348)
        );
    }

    #[test]
    fn a_waiter_blocked_by_a_large_run_lets_small_requests_go() {
        let holder = |pid: u32, megabytes: f64| Reservation {
            pid,
            megabytes,
            held_since: Some(1.0),
            ..Default::default()
        };
        let waiter = Reservation {
            pid: 13,
            started: 10.0,
            waiting_mb: Some(8000.0),
            waiting_since: Some(10.0),
            floor_mb: Some(2000.0),
            waits_on_floor: true,
            priority: Some(PRIORITY_NIGHTLY.into()),
            ..Default::default()
        };
        let book = [holder(11, 5000.0), holder(12, 1000.0), waiter.clone()];
        let others = refs(&book);
        // 6000 available: with the small run gone 7000 less the floor is
        // 5000, short of 8000: the large run must end first.
        assert!(blocked_by_large(
            SMALL_REQUEST_MB,
            &book[2],
            &others,
            Some(20_000.0),
            Some(6000.0)
        ));
        assert!(drain_allows(
            &book[2],
            &others,
            SMALL_REQUEST_MB,
            Some(20_000.0),
            Some(6000.0),
            None
        ));
        assert!(!drain_allows(
            &book[2],
            &others,
            SMALL_REQUEST_MB + 1.0,
            Some(20_000.0),
            Some(6000.0),
            None
        ));
        // 9000 available: with the small run gone the waiter fits (10 000
        // less the floor): the drain holds the small requests back too.
        assert!(!blocked_by_large(
            SMALL_REQUEST_MB,
            &book[2],
            &others,
            Some(20_000.0),
            Some(9000.0)
        ));
        assert!(!drain_allows(
            &book[2],
            &others,
            1000.0,
            Some(20_000.0),
            Some(9000.0),
            None
        ));
        // Only small runs hold memory: the drain empties the machine.
        let small = [holder(12, 1000.0), waiter];
        assert!(!blocked_by_large(
            SMALL_REQUEST_MB,
            &small[1],
            &refs(&small),
            Some(20_000.0),
            Some(1000.0)
        ));
    }

    #[test]
    fn a_drain_admits_what_leaves_the_waiter_its_room_or_ends_first() {
        let holder = |pid: u32, megabytes: f64, end: Option<f64>| Reservation {
            pid,
            started: 1.0,
            megabytes,
            held_since: Some(1.0),
            expected_end: end,
            ..Default::default()
        };
        let waiter = Reservation {
            pid: 13,
            started: 10.0,
            waiting_mb: Some(8000.0),
            waiting_since: Some(10.0),
            floor_mb: Some(2000.0),
            waits_on_floor: true,
            ..Default::default()
        };
        let capacity = Some(20_000.0);
        let book = [holder(11, 4000.0, Some(900.0)), waiter.clone()];
        let others = refs(&book);
        // Not drained before its 600 s, nor for a request older than it.
        assert!(drained_for(&others, 99, 30.0, 609.0).is_none());
        assert!(drained_for(&others, 99, 5.0, 700.0).is_none());
        let drained = drained_for(&others, 99, 30.0, 610.0).expect("drained");
        assert_eq!(drained.pid, 13);
        // Available 12 000 MB beside the reservations: a request of 2000
        // leaves 12 000 - 2000 - 2000 (floor) = 8000, the waiter's room.
        assert!(drain_allows(
            drained,
            &others,
            2000.0,
            capacity,
            Some(12_000.0),
            None
        ));
        assert!(!drain_allows(
            drained,
            &others,
            2001.0,
            capacity,
            Some(12_000.0),
            None
        ));
        // The capacity counts as well: 20 000 - 4000 - 8001 < 8000.
        assert!(!drain_allows(
            drained, &others, 8001.0, capacity, None, None
        ));
        // A request expected to end before every run that holds memory
        // does not delay the drain: it goes; after one, or with a holder
        // whose end is unknown, it waits.
        assert!(drain_allows(
            drained,
            &others,
            5000.0,
            capacity,
            Some(12_000.0),
            Some(900.0)
        ));
        assert!(!drain_allows(
            drained,
            &others,
            5000.0,
            capacity,
            Some(12_000.0),
            Some(901.0)
        ));
        let unknown = [
            holder(11, 4000.0, Some(900.0)),
            holder(12, 100.0, None),
            waiter.clone(),
        ];
        let unknown_refs = refs(&unknown);
        assert!(!drain_allows(
            &unknown[2],
            &unknown_refs,
            5000.0,
            capacity,
            Some(12_000.0),
            Some(800.0)
        ));
        // Nothing holds memory: no end to come before.
        let alone = [waiter.clone()];
        assert!(!drain_allows(
            &alone[0],
            &refs(&alone),
            5000.0,
            capacity,
            Some(5000.0),
            Some(800.0)
        ));
        // A waiter that holds memory and waits to grow counts what it holds
        // back on the floor's side (its resident memory is not available).
        let mut growing = waiter.clone();
        growing.megabytes = 3000.0;
        let grow = [holder(11, 4000.0, None), growing];
        assert!(drain_allows(
            &grow[1],
            &refs(&grow),
            2000.0,
            capacity,
            Some(9000.0),
            None
        ));
        assert!(!drain_allows(
            &grow[1],
            &refs(&grow),
            2001.0,
            capacity,
            Some(9000.0),
            None
        ));
    }

    #[test]
    fn the_nightly_s_request_goes_first_and_is_never_passed() {
        let holder = Reservation {
            pid: 11,
            started: 1.0,
            megabytes: 4000.0,
            held_since: Some(1.0),
            expected_end: Some(900.0),
            ..Default::default()
        };
        let waiter = |pid: u32, since: f64, nightly: bool| Reservation {
            pid,
            started: since,
            waiting_mb: Some(8000.0),
            waiting_since: Some(since),
            priority: nightly.then(|| PRIORITY_NIGHTLY.to_string()),
            ..Default::default()
        };
        // An older request and a nightly one that began waiting later: the
        // nightly's is the first waiter.
        let book = [
            holder.clone(),
            waiter(12, 5.0, false),
            waiter(13, 20.0, true),
        ];
        let others = refs(&book);
        assert_eq!(first_waiter(&others).map(|r| r.pid), Some(13));
        // It drains at once (a second after it began waiting), and the end
        // exception does not hold for it.
        let drained = drained_for(&others, 99, 21.0, 21.0).expect("drained at once");
        assert_eq!(drained.pid, 13);
        assert!(!drain_allows(
            drained,
            &others,
            2000.0,
            Some(20_000.0),
            Some(9000.0),
            Some(100.0)
        ));
        assert!(drain_allows(
            drained,
            &others,
            1000.0,
            Some(20_000.0),
            Some(9000.0),
            Some(100.0)
        ));
        // The older request is behind it in the order: it is a newer
        // request to the nightly's, which cannot fit now (4000 + 8000 of
        // 11 000): room for it beside the runs since 20 s (none) and this
        // request.
        assert!(!order_allows(&others, 12, 5.0, 3001.0, Some(11_000.0)));
        assert!(order_allows(&others, 12, 5.0, 3000.0, Some(11_000.0)));
        assert_eq!(waiter(1, 0.0, true).drain_after(), 0.0);
    }

    #[test]
    fn the_new_fields_and_those_of_newer_writers_survive_a_rewrite() {
        let directory = scratch("fields");
        let path = directory.join("ledger.json");
        let me = std::process::id();
        let text = serde_json::json!({"reservations": [{
            "pid": me, "started": 1.0, "start_ticks": start_ticks(me), "megabytes": 100.0,
            "label": "run.py: katia (expected peak)", "expected_end": 900.5, "priority": "nightly",
            "floor_mb": 10240.0, "drain_after": 300.0, "a_later_field": [1, 2]
        }]});
        fs::write(&path, text.to_string()).unwrap();
        let ledger = Ledger::at(&path);
        let live = ledger.live().unwrap();
        assert_eq!(live[0].expected_end, Some(900.5));
        assert!(live[0].nightly());
        assert_eq!(live[0].floor_mb, Some(10240.0));
        assert_eq!(live[0].drain_after, Some(300.0));
        // A change elsewhere rewrites the file: every field stays.
        ledger
            .update(|reservations| reservations.push(reservation(me, 5.0, "another")))
            .unwrap();
        let stored: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let first = &stored["reservations"][0];
        assert_eq!(first["a_later_field"], serde_json::json!([1, 2]));
        assert_eq!(first["priority"], "nightly");
        assert_eq!(first["expected_end"], 900.5);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_capacity_is_the_machines_memory_less_the_margin() {
        // Read without the variables set (cargo runs tests side by side:
        // only read here).
        if std::env::var_os("PCB_MAKER_LEDGER_TOTAL_MB").is_none()
            && std::env::var_os("PCB_MAKER_LEDGER_MARGIN_MB").is_none()
            && let Some(total) = crate::board_router::memory_total_mb()
        {
            assert_eq!(capacity_mb(), Some((total - DEFAULT_MARGIN_MB).max(0.0)));
        }
    }
}

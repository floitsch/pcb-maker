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
//! resident size with it and raises the reservation while the ledger has
//! room; past it without room, it waits in the ledger for the room as of
//! its admission (since 2026-10-27) and ends the run with exit 4 and a
//! verdict naming the reservation, rather than leaving it to the OOM
//! killer, only when the memory runs short (`overgrown_ends`). A harness's
//! reservation handed to the process (`granted_mb`) is the least of its
//! own (since 2026-10-27, `granted_of`).
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
//! The ledger drains only for a waiter it could start (since 2026-10-26,
//! `drain_state`): a waiter counts for the drain only while, with every
//! reservation that began holding since it began waiting gone, its room
//! under its own floor (`floor_mb`; `DEFAULT_FLOOR_MB` for an entry of an
//! older writer, which has no `refreshed`) and the reservations' growth
//! covers what it waits for, while its wait is within its bound
//! (`drain_max`, default `DEFAULT_DRAIN_MAX_SECONDS`), and while its
//! writer rewrites its entry (`refreshed`, at most `STALE_AFTER_SECONDS`
//! ago; none: unknown, live). Otherwise newer requests pass it by ordered
//! admission alone; it keeps its place for when room appears. Until then
//! an old nightly harness (356c8db, before the drain) waiting 27 hours for
//! 24.7 GB with its own floor of 10 GB, which neither the ledger's room
//! nor the drain could start, held every request of more than 4 GB back
//! for hours and every other one for minutes.
//!
//! Short requests never wait behind a drain (since 2026-10-26,
//! `short_request`, `short_allows`): a process of a short command (`arm`:
//! every armed command but a route or a layout, which hold their memory
//! for minutes; `PCB_MAKER_LEDGER_SHORT` 0 or 1 overrides) that asks for at
//! most `SHORT_OWN_MB` of its own beside at most one KiCad check goes as
//! soon as the capacity has its room and the memory available less what
//! the reservations still grow by covers it, whatever the order and the
//! drain say. Its reservation is marked `short` and counts as gone in the
//! order's and the drain's sums for the others (it ends within seconds):
//! it never passes a waiter there. Until then a 1.5 s check of a fresh
//! agent's board waited 657 s behind a harness's 3 GB board that its floor
//! of 10 GB held back for 4300 s.
//!
//! A harness's floor from measured headroom (since 2026-10-27,
//! `measured_floor_mb`): run.py's floor of available memory beside the
//! reservations' growth is `FLOOR_KEEP_MB` plus the 0.95 quantile of how
//! far the memory outside the ledger rose within ten minutes (the
//! harnesses' samples in `<ledger>.headroom.json`) plus the board's
//! tier's overshoot of its estimate, `DEFAULT_FLOOR_MB` without samples.
//! Until then it was 10 GB by opinion, and a 3 GB board waited 4371 s
//! with 9.4 GB beside the growth. Binaries keep no floor.
//!
//! The capacity from the machine (since 2026-10-27, `machine_capacity_mb`):
//! MemTotal less the memory outside the ledger (the headroom samples'
//! median, `DEFAULT_OUTSIDE_SHARE` of MemTotal without fresh samples), at
//! most MemTotal less the margin; a binary's new request keeps the
//! harnesses' floor beside it (`binary_floor_allows`: the measured one,
//! `FLOOR_KEEP_MB` for a short request). Until then the capacity was
//! MemTotal less the margin, 11-21 GB more than the machine could give
//! beside the memory outside the ledger, and binaries admitted by it alone.
//! Every kicad-cli that loads a board runs inside its process's
//! reservation (`kicad_check`): reserved up front by a command whose memory
//! is its checks, at the first check by any other (`arm_on_demand`), and
//! a process of a harness's board runs inside the board's reservation
//! (`PCB_MAKER_LEDGER_INSIDE`).
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

/// How long a waiter's wait may hold newer requests to a drain, unless its
/// entry says otherwise (`drain_max`; run.py's `--ledger-drain-max`):
/// past it the drain for it ends (`drain_state`).
pub const DEFAULT_DRAIN_MAX_SECONDS: f64 = 7200.0;

/// A waiting entry whose writer has not rewritten it (`refreshed`) for
/// longer than this is stale: never drained for (harnesses rewrite theirs
/// every 15 s, binaries every 2 s).
pub const STALE_AFTER_SECONDS: f64 = 120.0;

/// The floor of available memory assumed for a waiter whose entry names
/// none and comes from an older writer (no `refreshed`): run.py's
/// `--min-free-gb 10` beside other agents.
pub const DEFAULT_FLOOR_MB: f64 = 10240.0;

/// The most a short request (`short_request`, since 2026-10-26) holds for
/// its own process; its children may hold one KiCad check
/// (`KICAD_CHECK_MB`) beside it.
pub const SHORT_OWN_MB: f64 = 2048.0;

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
    /// While it waits: how long its wait may hold newer requests to a
    /// drain (seconds; `DEFAULT_DRAIN_MAX_SECONDS` where absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drain_max: Option<f64>,
    /// While it waits: when its writer last rewrote the entry (seconds
    /// since the epoch); unknown where absent (an older writer's entry:
    /// assumed live).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed: Option<f64>,
    /// When the run is expected to end (seconds since the epoch; run.py's
    /// boards of a tier, from the tier's seconds), for the drain's end
    /// exception; unknown where absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_end: Option<f64>,
    /// `nightly` (`PRIORITY_NIGHTLY`): first among the waiters and never
    /// passed; none where absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// A short request's (`short_request`): it ends within seconds and
    /// counts as gone in the order's and the drain's sums for the others
    /// (`order_allows`, `drain_state`, `drain_allows`); its megabytes count
    /// for the capacity as any reservation's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub short: bool,
    /// The part of the megabytes held for KiCad's checks (kicad-cli, the
    /// process's children; since 2026-10-27): reserved up front by a
    /// command that checks, or while a check runs. None where absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_mb: Option<f64>,
    /// Whether a KiCad check runs now (`pcb-maker ledger`: `running`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub checking: bool,
    /// The harness's process whose reservation this one runs inside
    /// (`PCB_MAKER_LEDGER_INSIDE`, since 2026-10-27): the megabytes are only
    /// what the process needs beyond it. None where absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside: Option<u32>,
    /// What a harness reserved for this process and handed to it (run.py's
    /// gate moves a board's reservation to the board's binary; since
    /// 2026-10-27): the process's own limit is at least this, so that its
    /// reservation is the one the harness admitted, not only its own
    /// estimate. None where absent (an older harness's hand-over is
    /// recognised by its label: `granted_of`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_mb: Option<f64>,
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

    /// How long its wait may hold newer requests to a drain: `drain_max`
    /// or the default.
    pub fn drain_max(&self) -> f64 {
        self.drain_max
            .filter(|seconds| *seconds >= 0.0)
            .unwrap_or(DEFAULT_DRAIN_MAX_SECONDS)
    }

    /// The floor of available memory its harness keeps beside it, for
    /// whether the ledger could start it (`drain_state`): `floor_mb`; for
    /// an entry of an older writer (no `refreshed`) without one,
    /// `DEFAULT_FLOOR_MB`; else none.
    pub fn assumed_floor_mb(&self) -> f64 {
        match (self.floor_mb, self.refreshed) {
            (Some(floor), _) => floor.max(0.0),
            (None, None) => DEFAULT_FLOOR_MB,
            (None, Some(_)) => 0.0,
        }
    }

    /// Its place among the waiters: the nightly's first, then by when the
    /// wait began, then by pid.
    pub fn order_key(&self) -> (u8, f64, u32) {
        (u8::from(!self.nightly()), self.waiting_since(), self.pid)
    }
}

/// What a harness reserved for the process of `entry` and handed to it
/// (since 2026-10-27): the entry's `granted_mb`, else, for the hand-over of
/// a harness before it (an entry under the process's pid that the process
/// did not write: another `label` or `started` than its own), the entry's
/// megabytes; 0 for the process's own entry without one.
pub fn granted_of(entry: &Reservation, label: &str, started: f64) -> f64 {
    if let Some(granted) = entry.granted_mb {
        return granted.max(0.0);
    }
    if entry.label != label || !same_start(entry.started, started) {
        return entry.megabytes.max(0.0);
    }
    0.0
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

/// Without fresh headroom samples, the share of the machine's memory the
/// capacity assumes outside the ledger (since 2026-10-27,
/// `machine_capacity_mb`): on a machine of 62.7 GB 15.7 GB, below the
/// 16.7 GB the night of 2026-10-26/27 measured at its 5th percentile; on
/// one of 16 or 24 GB the margin (`DEFAULT_MARGIN_MB`) is the smaller
/// bound.
pub const DEFAULT_OUTSIDE_SHARE: f64 = 0.25;

/// What the reservations may take together on a machine of `total_mb`
/// (since 2026-10-27): MemTotal less the memory outside the ledger, the
/// median of the headroom samples (`summary`'s `outside_mb`) when they are
/// fresh (`headroom_fresh`), else `DEFAULT_OUTSIDE_SHARE` of MemTotal; at
/// most MemTotal less `margin_mb` (the static bound, 56.7 GB here). With
/// what it is made of. Until then it was the static bound alone, 11-21 GB
/// more than the machine could give beside the 16.7-27.5 GB outside the
/// ledger. benchmarks/harness_common.py, `machine_capacity_mb`: the same.
pub fn machine_capacity_mb(
    total_mb: f64,
    margin_mb: f64,
    summary: Option<&HeadroomSummary>,
    now: f64,
) -> (f64, String) {
    let bound = (total_mb - margin_mb).max(0.0);
    let measured = summary
        .filter(|summary| headroom_fresh(summary, now))
        .and_then(|summary| summary.outside_mb.map(|outside| (outside, summary.samples)));
    let (outside, how) = match measured {
        Some((outside, count)) => (
            outside.max(0.0),
            format!("the median of {count} headroom samples"),
        ),
        None => (
            total_mb * DEFAULT_OUTSIDE_SHARE,
            format!(
                "{:.0} % of MemTotal without fresh headroom samples",
                DEFAULT_OUTSIDE_SHARE * 100.0
            ),
        ),
    };
    let capacity = (total_mb - outside).max(0.0).min(bound);
    (
        capacity,
        format!(
            "MemTotal {:.1} GB less {:.1} GB outside the ledger ({how}); at most {:.1} GB, MemTotal less the margin",
            total_mb / 1024.0,
            outside / 1024.0,
            bound / 1024.0
        ),
    )
}

/// How long a process keeps the headroom summary it read for the capacity
/// and its floor before it reads the file again.
const HEADROOM_REREAD: Duration = Duration::from_secs(30);

/// The headroom samples' summary beside the ledger `PCB_MAKER_LEDGER`
/// names, read at most every `HEADROOM_REREAD`.
fn headroom_now() -> Option<HeadroomSummary> {
    static READ: Mutex<Option<(Instant, PathBuf, Option<HeadroomSummary>)>> = Mutex::new(None);
    let ledger = Ledger::from_env()?;
    let mut read = READ.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((at, path, summary)) = read.as_ref()
        && path == ledger.path()
        && at.elapsed() < HEADROOM_REREAD
    {
        return summary.clone();
    }
    let summary = ledger.headroom();
    *read = Some((Instant::now(), ledger.path().to_path_buf(), summary.clone()));
    summary
}

/// What the reservations may take together, in megabytes, and what it is
/// made of: `PCB_MAKER_LEDGER_TOTAL_MB`, else `machine_capacity_mb` of the
/// machine's memory, its margin (`PCB_MAKER_LEDGER_MARGIN_MB`, default
/// `DEFAULT_MARGIN_MB`) and the headroom samples beside the ledger. None
/// when the machine's memory is unknown (no waiting then).
pub fn capacity_described() -> Option<(f64, String)> {
    if let Some(total) = environment_mb("PCB_MAKER_LEDGER_TOTAL_MB") {
        return Some((total, "PCB_MAKER_LEDGER_TOTAL_MB".into()));
    }
    let margin = std::env::var("PCB_MAKER_LEDGER_MARGIN_MB")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|megabytes| megabytes.is_finite() && *megabytes >= 0.0)
        .unwrap_or(DEFAULT_MARGIN_MB);
    let total = crate::board_router::memory_total_mb()?;
    Some(machine_capacity_mb(
        total,
        margin,
        headroom_now().as_ref(),
        now_seconds(),
    ))
}

/// What the reservations may take together, in megabytes
/// (`capacity_described`).
pub fn capacity_mb() -> Option<f64> {
    capacity_described().map(|(capacity, _)| capacity)
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
/// (`first_waiter`); a binary's own request is never the nightly's. A
/// short request's reservation (`short`) counts as gone: it ends within
/// seconds and has not passed the oldest (the capacity counts it,
/// `may_hold`).
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
    let rest = || {
        others
            .iter()
            .filter(|r| !std::ptr::eq(**r, oldest) && !r.short)
    };
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
/// comes before the request and the ledger drains for it (`drain_state`:
/// it has waited its `drain_after`, the nightly's at once, and the ledger
/// could start it). `capacity` and `available_mb` (the memory available
/// less what the reservations still grow by, `available_less_growth`):
/// None, unknown, unbounded.
pub fn drained_for<'a>(
    others: &[&'a Reservation],
    me: u32,
    since: f64,
    now: f64,
    capacity: Option<f64>,
    available_mb: Option<f64>,
) -> Option<&'a Reservation> {
    let waiter = first_waiter(others)?;
    (waiter.order_key() < (1, since, me)
        && drain_state(waiter, others, now, capacity, available_mb) == Drain::Drains)
        .then_some(waiter)
}

/// Whether the ledger drains for a waiter (`drain_state`), and if not, why.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drain {
    /// It has not waited its `drain_after` yet.
    NotYet,
    /// Newer requests are held to the drain (`drain_allows`).
    Drains,
    /// Its writer has not rewritten its entry for this many seconds (more
    /// than `STALE_AFTER_SECONDS`).
    Stale(f64),
    /// It has waited longer than its bound (`drain_max`, seconds).
    PastBound(f64),
    /// Even with every reservation since it began waiting gone, its room
    /// (megabytes) under its floor falls short of what it waits for.
    CannotStart(f64),
}

impl Drain {
    /// What it means, for `pcb-maker ledger` and the waiting lines.
    pub fn describe(&self) -> String {
        match self {
            Drain::NotYet => "not drained for yet".into(),
            Drain::Drains => "the ledger drains for it".into(),
            Drain::Stale(seconds) => {
                format!("not drained for: its entry was last rewritten {seconds:.0} s ago (stale)")
            }
            Drain::PastBound(bound) => {
                format!("not drained for: it has waited past the drain's bound of {bound:.0} s")
            }
            Drain::CannotStart(room) => format!(
                "not drained for: with every run since its wait began gone its room would be {room:.0} MB (the capacity, and the memory available less its floor), too little to start it"
            ),
        }
    }
}

/// Whether the ledger drains for `waiter` beside `others` at `now` (since
/// 2026-10-26): once it has waited its `drain_after`, unless its entry is
/// stale (`refreshed` more than `STALE_AFTER_SECONDS` ago; absent:
/// unknown, live), its wait is past its `drain_max`, or the ledger could
/// not start it even with every reservation that began holding since its
/// wait began gone (`held_since`): its room (`waiter_room`: the capacity
/// less the older reservations, and `available_mb` with the newer ones'
/// megabytes back less its floor, `Reservation::assumed_floor_mb`) short
/// of what it waits for. Then newer requests pass it by ordered admission
/// alone (`order_allows`); it keeps its place. Until then a drain for an
/// old harness's waiter of 24.7 GB with a floor of 10 GB that could not
/// start held every request of more than 4 GB back for hours.
/// benchmarks/harness_common.py, `drain_state`: the same rule.
pub fn drain_state(
    waiter: &Reservation,
    others: &[&Reservation],
    now: f64,
    capacity: Option<f64>,
    available_mb: Option<f64>,
) -> Drain {
    let waited = now - waiter.waiting_since();
    if waited < waiter.drain_after() {
        return Drain::NotYet;
    }
    if let Some(at) = waiter.refreshed
        && now - at > STALE_AFTER_SECONDS
    {
        return Drain::Stale(now - at);
    }
    if waited > waiter.drain_max() {
        return Drain::PastBound(waiter.drain_max());
    }
    let newer: f64 = others
        .iter()
        .filter(|r| {
            !std::ptr::eq(**r, waiter)
                && r.megabytes > 0.0
                && (r.short || r.held_since() >= waiter.waiting_since())
        })
        .map(|r| r.megabytes)
        .sum();
    let room = waiter_room(
        waiter,
        others,
        capacity,
        available_mb,
        0.0,
        newer,
        waiter.assumed_floor_mb(),
    );
    if room < waiter.waiting_mb.unwrap_or(0.0) {
        Drain::CannotStart(room)
    } else {
        Drain::Drains
    }
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
/// 28 GB run held every other agent's request for hours). A short
/// request's reservation (`short`) counts as gone (it ends within seconds).
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
    let rest = || {
        others
            .iter()
            .filter(|r| !std::ptr::eq(**r, waiter) && !r.short)
    };
    let floor = waiter.floor_mb.unwrap_or(0.0).max(0.0);
    if waiter_room(
        waiter,
        others,
        capacity,
        available_mb,
        wanted_mb,
        short_held_mb(waiter, others),
        floor,
    ) >= wants
    {
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
/// `available_mb` (None: unbounded) less the waiter's floor `floor_mb`
/// (what the waiter holds counts back).
fn waiter_room(
    waiter: &Reservation,
    others: &[&Reservation],
    capacity: Option<f64>,
    available_mb: Option<f64>,
    wanted_mb: f64,
    freed_mb: f64,
    floor_mb: f64,
) -> f64 {
    let reserved: f64 = others
        .iter()
        .filter(|r| !std::ptr::eq(**r, waiter))
        .map(|r| r.megabytes)
        .sum::<f64>()
        - freed_mb;
    let ledger_room = capacity.map_or(f64::INFINITY, |capacity| capacity - reserved - wanted_mb);
    let floor_room = available_mb.map_or(f64::INFINITY, |available| {
        available + freed_mb - wanted_mb - floor_mb + waiter.megabytes
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
            .filter(|r| !std::ptr::eq(**r, waiter) && r.megabytes > 0.0 && !r.short)
    };
    if !holders().any(|r| r.megabytes > small_mb) {
        return false;
    }
    let small: f64 = holders()
        .filter(|r| r.megabytes <= small_mb)
        .map(|r| r.megabytes)
        .sum::<f64>()
        + short_held_mb(waiter, others);
    let floor = waiter.floor_mb.unwrap_or(0.0).max(0.0);
    waiter_room(waiter, others, capacity, available_mb, 0.0, small, floor)
        < waiter.waiting_mb.unwrap_or(0.0)
}

/// What the short requests' reservations (`short`) in `others` but
/// `waiter` hold: counted as gone in the drain's rooms.
fn short_held_mb(waiter: &Reservation, others: &[&Reservation]) -> f64 {
    others
        .iter()
        .filter(|r| !std::ptr::eq(**r, waiter) && r.short && r.megabytes > 0.0)
        .map(|r| r.megabytes)
        .sum()
}

/// Whether a process of a short command (`arm`) asking for `base_mb` of
/// its own and `extra_mb` for its children makes a short request (since
/// 2026-10-26): at most `SHORT_OWN_MB` of its own beside at most one
/// KiCad check (`KICAD_CHECK_MB`; a check of any board, 1.5 s on a small
/// one).
pub fn short_request(base_mb: f64, extra_mb: f64) -> bool {
    base_mb <= SHORT_OWN_MB && extra_mb <= KICAD_CHECK_MB
}

/// Whether a short request (`short_request`) of `wanted_mb` goes whatever
/// the order and the drain say (since 2026-10-26): the memory for it is
/// there, the memory available less what the reservations still grow by
/// (`available_less_growth`; None: unknown, there) covers it (a binary
/// keeps no floor of its own). The capacity must have its room as well
/// (`may_hold`). Until then a 1.5 s check waited 657 s behind a drain for
/// a harness's board that its own floor held back.
/// benchmarks/harness_common.py, `try_admit` with `short`: the same rule.
pub fn short_allows(wanted_mb: f64, available_mb: Option<f64>) -> bool {
    available_mb.is_none_or(|available| available >= wanted_mb)
}

/// Whether the agent command of class `short` (route and layout: long,
/// every other armed command: short) makes short requests:
/// `PCB_MAKER_LEDGER_SHORT` 1 or 0 overrides.
pub fn short_class(short: bool) -> bool {
    short_override(
        std::env::var("PCB_MAKER_LEDGER_SHORT").ok().as_deref(),
        short,
    )
}

/// `short_class` with the variable's value: `1` short, `0` long, anything
/// else (or unset) the command's own class `short`.
fn short_override(value: Option<&str>, short: bool) -> bool {
    match value.map(str::trim) {
        Some("1") => true,
        Some("0") => false,
        _ => short,
    }
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
    /// Whether the command is short (`arm`, `short_class`): its new
    /// requests within `short_request` go by `short_allows`.
    short: bool,
    /// Whether it holds as a short request (its entry's `short`).
    held_short: bool,
    /// What the KiCad checks running now take of `extra_mb`.
    checks_mb: f64,
    /// The harness's reservation it runs inside (`PCB_MAKER_LEDGER_INSIDE`:
    /// its pid and `started`).
    inside: Option<(u32, f64)>,
    /// What a harness reserved for it and handed to it (`granted_of`): its
    /// own limit is at least this.
    granted_mb: f64,
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
/// callers and tests). Reserves nothing yet. `short`: the command holds
/// its memory for seconds (every armed agent command but a route and a
/// layout; `PCB_MAKER_LEDGER_SHORT` overrides, `short_class`): its new
/// requests within `short_request` never wait behind the order or a drain
/// (`short_allows`).
pub fn arm(label: &str, short: bool) {
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
        short: short_class(short),
        held_short: false,
        checks_mb: 0.0,
        inside: inside_key(std::env::var("PCB_MAKER_LEDGER_INSIDE").ok().as_deref()),
        granted_mb: 0.0,
    });
}

/// The reservation `PCB_MAKER_LEDGER_INSIDE` names (`<pid> <started>`, as
/// run.py's MemoryGate writes it for the binaries of a board it holds the
/// memory of): its pid and `started`.
fn inside_key(value: Option<&str>) -> Option<(u32, f64)> {
    let mut words = value?.split_whitespace();
    let pid = words.next()?.parse().ok()?;
    let started = words
        .next()?
        .parse::<f64>()
        .ok()
        .filter(|t| t.is_finite())?;
    words.next().is_none().then_some((pid, started))
}

/// Whether two `started` times are the same (a millisecond apart at most:
/// the harness writes them as JSON and in `PCB_MAKER_LEDGER_INSIDE`).
fn same_start(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

/// The label a process arms under when its first KiCad check comes
/// (`arm_on_demand`, `kicad_check`).
static ON_DEMAND: Mutex<Option<String>> = Mutex::new(None);

/// Makes this process arm under `label` as a short command when it first
/// runs a KiCad check (`kicad_check`; since 2026-10-27): the executable
/// says so for every command, so that no kicad-cli it starts runs outside
/// a reservation (a fab export, a netlist import, a research command). The
/// library's callers and tests do not: their checks reserve nothing.
pub fn arm_on_demand(label: &str) {
    *ON_DEMAND
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(label.to_string());
}

/// Reserves one KiCad check (`KICAD_CHECK_MB`) for this process's
/// children up front (since 2026-10-27): a command whose memory is its
/// checks (`drc-kicad-board`, `place-kicad-board`, `inspect-routing-run
/// --native-check`, a command armed on demand) holds it from before its
/// work until it ends, and its checks run inside it (`while_reserved`).
/// Nothing when not armed or when held already.
pub fn reserve_check_up_front() {
    let Some((base, extra)) = reserved_mb() else {
        return;
    };
    if extra < KICAD_CHECK_MB {
        hold(base, KICAD_CHECK_MB, "KiCad's check (reserved up front)");
    }
}

/// Runs `work`, a kicad-cli process that loads a board or a schematic
/// (DRC, zone fill, an export, a netlist), inside this process's
/// reservation (`while_reserved` of `KICAD_CHECK_MB`); a process armed on
/// demand (`arm_on_demand`) arms first and reserves the check up front.
/// Until 2026-10-27 the fab exports, the netlist import and the research
/// commands ran kicad-cli outside any reservation (2.1-2.2 GB each in the
/// headroom samples). `kicad-cli version` (64 MB) needs none.
pub fn kicad_check<T>(work: impl FnOnce() -> T) -> T {
    if !armed() {
        let label = ON_DEMAND
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(label) = label {
            arm(&label, true);
            reserve_check_up_front();
        }
    }
    while_reserved(KICAD_CHECK_MB, "KiCad's check", work)
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
        /// What of the floor holds it back (`binary_floor_allows`), for
        /// the log.
        floor: Option<String>,
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
    let capacity = capacity_mb();
    let now = now_seconds();
    let since = own.waiting_since.unwrap_or(now);
    let checking = own.checks_mb > 0.0;
    let inside_key = own.inside;
    // What the entry it replaces (a harness's reservation for this process,
    // or its own) says of the run beyond the memory: kept.
    #[allow(clippy::too_many_arguments)]
    let entry = |megabytes: f64,
                 check_mb: f64,
                 waiting_mb: Option<f64>,
                 held_since: Option<f64>,
                 short: bool,
                 inside: Option<u32>,
                 granted: f64,
                 inherited: Option<&Reservation>| Reservation {
        pid: me,
        started: own.started,
        start_ticks: own.start_ticks,
        megabytes,
        label: own.label.clone(),
        waiting_mb,
        waiting_since: waiting_mb.map(|_| since),
        held_since: held_since.filter(|_| megabytes > 0.0),
        refreshed: waiting_mb.map(|_| now),
        expected_end: inherited.and_then(|r| r.expected_end),
        priority: inherited.and_then(|r| r.priority.clone()),
        short: short && megabytes > 0.0,
        check_mb: (check_mb > 0.0).then_some(check_mb),
        checking,
        inside,
        granted_mb: (granted > 0.0).then_some(granted),
        other: inherited.map(|r| r.other.clone()).unwrap_or_default(),
        ..Default::default()
    };
    let ancestors = &own.ancestors;
    let held_since = own.held_since;
    let (short_class, held_short) = (own.short, own.held_short);
    let held_check = own.extra_mb;
    let (held_base, own_granted) = (own.base_mb, own.granted_mb);
    let (label, started) = (own.label.as_str(), own.started);
    let outcome = own.ledger.update(|reservations| {
        // What a harness reserved for this process and handed to it (run.py's
        // gate moves the board's reservation to its binary; `granted_of`):
        // the process's own limit is at least that (since 2026-10-27). Until
        // then the process's first reservation replaced it with its own
        // estimate: run.py admitted Quanta75 with 2.4 GB, its layout
        // replaced that with 1331 MB of its own (1405 when it began
        // routing; the night's held 1415), and ended with exit 4 at 1474 MB
        // resident while the room it had given back was taken.
        let granted = reservations
            .iter()
            .find(|r| r.pid == me)
            .map_or(0.0, |r| granted_of(r, label, started))
            .max(own_granted);
        let base_mb = base_mb.max(granted);
        let wanted = base_mb + extra_mb;
        // What it holds: the hand-over counts as its own.
        let held = held_base.max(granted) + held_check;
        // The harness's reservation this process runs inside
        // (`PCB_MAKER_LEDGER_INSIDE`): only what it needs beyond it is its
        // own entry's.
        let inside = inside_key.and_then(|(pid, started)| {
            reservations
                .iter()
                .find(|r| r.pid == pid && r.pid != me && same_start(r.started, started) && r.megabytes > 0.0)
                .map(|r| (r.pid, r.megabytes, r.held_since()))
        });
        let inside_mb = inside.map_or(0.0, |(_, megabytes, _)| megabytes);
        let own_wanted = (wanted - inside_mb).max(0.0);
        let own_held = (held - inside_mb).max(0.0);
        // A harness's reservation for this process (re-pointed to its
        // pid), or one of a process above it, or the one it runs inside:
        // admitted already.
        let harness = reservations
            .iter()
            .filter(|r| (r.pid == me || ancestors.contains(&r.pid)) && r.megabytes > 0.0)
            .map(Reservation::held_since)
            .chain(inside.map(|(_, _, at)| at))
            .fold(None, |first: Option<f64>, at| {
                Some(first.map_or(at, |first| first.min(at)))
            });
        let fits = may_hold(
            reservations,
            me,
            held,
            own.started,
            own_wanted,
            capacity,
            ancestors,
        );
        // Less than it holds always goes (a check's memory given back).
        let fits = fits || (held > 0.0 && wanted <= held);
        let others: Vec<&Reservation> = reservations.iter().filter(|r| r.pid != me).collect();
        let reserved: f64 = others.iter().map(|r| r.megabytes).sum();
        let count = others.len();
        // Ordered admission, the drain and the floor for a new request
        // only: a run that holds memory grows as before (the oldest waiter
        // may wait for it to end). A binary does not know when it will end:
        // in a drain it goes by the waiter's room alone.
        let new_request = held <= 0.0 && harness.is_none();
        // (The memory available is read for a new request only.)
        let available = new_request.then(|| available_less_growth(&others)).flatten();
        let drained = new_request
            .then(|| drained_for(&others, me, since, now, capacity, available))
            .flatten();
        let ordered = !new_request
            || (order_allows(&others, me, since, wanted, capacity)
                && drained.is_none_or(|waiter| {
                    drain_allows(waiter, &others, wanted, capacity, available, None)
                }));
        // A short request goes whatever the order and the drain say once
        // the memory for it is there (since 2026-10-26).
        let short = new_request && short_class && short_request(base_mb, extra_mb);
        // (Held back only by the order or the drain: a waiter waits, and
        // the memory available was read.)
        let ordered = ordered || (short && short_allows(wanted, available));
        // A new request keeps the harnesses' floor beside it (since
        // 2026-10-27): the measured one, a short request `FLOOR_KEEP_MB`.
        // Alone (no other run holds memory) it goes, as a harness's.
        let floor = new_request.then(|| {
            if short {
                (FLOOR_KEEP_MB, "the 2.0 GB to keep".to_string())
            } else {
                measured_floor_mb(headroom_now().as_ref(), 0.0, now)
            }
        });
        let alone = !others
            .iter()
            .any(|r| r.megabytes > 0.0 && !ancestors.contains(&r.pid));
        let floor_ok = alone || binary_floor_allows(wanted, available, floor.as_ref().map(|f| f.0));
        let floor_text = (!floor_ok).then(|| {
            let (floor, how) = floor.clone().unwrap_or_default();
            format!(
                "the memory available less what the runs still grow by, {:.1} GB, would leave less than the floor of {:.1} GB ({how})",
                available.unwrap_or(0.0) / 1024.0,
                floor / 1024.0
            )
        });
        let short = short || (held_short && held > 0.0);
        let behind = (fits && !ordered)
            .then(|| first_waiter(&others))
            .flatten()
            .map(|r| {
                let drain = match drain_state(r, &others, now, capacity, available) {
                    Drain::NotYet => String::new(),
                    state => format!("; {}", state.describe()),
                };
                format!(
                    "{} (pid {}, waiting {:.0} s for {:.0} MB{drain})",
                    r.label,
                    r.pid,
                    (now - r.waiting_since()).max(0.0),
                    r.waiting_mb.unwrap_or(0.0),
                )
            });
        let waits_on_floor = fits && ordered && !floor_ok;
        let fits = fits && ordered && floor_ok;
        let held_since = held_since.or(harness).unwrap_or(now);
        // A harness's reservation for this process gives way to the
        // process's own, at least as large (`granted`).
        let inherited = reservations.iter().find(|r| r.pid == me).cloned();
        reservations.retain(|r| r.pid != me);
        let inside_pid = inside.map(|(pid, _, _)| pid);
        if fits {
            reservations.push(entry(
                own_wanted,
                extra_mb,
                None,
                Some(held_since),
                short,
                inside_pid,
                granted,
                inherited.as_ref(),
            ));
        } else if wait || held > 0.0 {
            let mut waiting = entry(
                own_held,
                held_check,
                wait.then_some(wanted),
                Some(held_since),
                held_short,
                inside_pid,
                granted,
                inherited.as_ref(),
            );
            if wait && new_request {
                waiting.floor_mb = floor.as_ref().map(|f| f.0);
                waiting.waits_on_floor = waits_on_floor;
            }
            reservations.push(waiting);
        }
        let outcome = (fits, reserved, count, behind, held_since, short, floor_text);
        (outcome, granted, base_mb)
    });
    match outcome {
        Ok(((true, _, _, _, held_since, short, _), granted, base_mb)) => {
            own.granted_mb = granted;
            own.base_mb = base_mb;
            own.extra_mb = extra_mb;
            own.waiting_since = None;
            own.held_short = short && base_mb + extra_mb > 0.0;
            if base_mb + extra_mb > 0.0 {
                own.held_since.get_or_insert(held_since);
            }
            Try::Held
        }
        Ok(((false, reserved_mb, others, behind, held_since, _, floor), granted, _)) => {
            // The hand-over is held as its own (the waiting entry keeps it).
            own.granted_mb = granted;
            if granted > own.base_mb {
                own.base_mb = granted;
                own.held_since.get_or_insert(held_since);
            }
            if wait {
                own.waiting_since = Some(since);
            }
            Try::Waiting {
                reserved_mb,
                others,
                behind,
                floor,
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

/// Whether a binary's new request of `wanted_mb` keeps `floor_mb` (since
/// 2026-10-27; None: none) of the memory available less what the
/// reservations still grow by (`available_mb`; None: unknown, kept) beside
/// it: the harnesses' rule (harness_common.memory_room), which binaries
/// before it did not keep, so that a burst of direct calls could take the
/// machine below the floor the harnesses kept.
pub fn binary_floor_allows(
    wanted_mb: f64,
    available_mb: Option<f64>,
    floor_mb: Option<f64>,
) -> bool {
    match (available_mb, floor_mb) {
        (Some(available), Some(floor)) => available - wanted_mb >= floor,
        _ => true,
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
                floor,
            } => {
                if reported.is_none_or(|at| at.elapsed() >= WAIT_REPORT) {
                    let capacity = capacity_mb().unwrap_or(f64::INFINITY);
                    eprintln!(
                        "memory ledger: waiting to reserve {:.0} MB for {why}: {reserved_mb:.0} MB of {capacity:.0} MB reserved by {others} other run{}{}{} (pcb-maker ledger lists them; waited {:.0} s)",
                        base_mb + extra_mb,
                        if others == 1 { "" } else { "s" },
                        behind.map_or(String::new(), |behind| format!(
                            "; behind an older request, {behind}"
                        )),
                        floor.map_or(String::new(), |floor| format!("; {floor}")),
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
    let live = own.ledger.live().ok()?;
    let others = live
        .iter()
        .filter(|r| r.pid != me)
        .map(|r| r.megabytes)
        .sum::<f64>();
    // The reservation it runs inside is its own room too.
    let inside = own.inside.map_or(0.0, |(pid, started)| {
        live.iter()
            .filter(|r| r.pid == pid && r.pid != me && same_start(r.started, started))
            .map(|r| r.megabytes)
            .sum()
    });
    Some(capacity - others + inside)
}

/// Runs `work` with `megabytes` reserved for a child process (a kicad-cli
/// check): inside what the process holds for its children already (a
/// check reserved up front, `reserve_check_up_front`) where that has room
/// beside the checks running, else with the rest raised first (waiting for
/// the room) and released afterwards. The entry says a check runs
/// (`checking`; `pcb-maker ledger`).
pub fn while_reserved<T>(megabytes: f64, why: &str, work: impl FnOnce() -> T) -> T {
    let Some((base, extra)) = reserved_mb() else {
        return work();
    };
    let running = own().as_ref().map_or(0.0, |own| own.checks_mb);
    let raise = (megabytes - (extra - running).max(0.0)).max(0.0);
    if raise > 0.0 {
        hold(base, extra + raise, why);
    }
    set_checking(megabytes);
    let outcome = work();
    set_checking(-megabytes);
    if let Some((base, extra)) = reserved_mb() {
        let _ = try_hold(base, (extra - raise).max(0.0), false);
    }
    outcome
}

/// Adds `megabytes` to the checks running now and rewrites the entry when
/// whether one runs changed (a check's start without a raise).
fn set_checking(megabytes: f64) {
    let changed = {
        let mut guard = own();
        let Some(own) = guard.as_mut() else {
            return;
        };
        let before = own.checks_mb > 0.0;
        own.checks_mb = (own.checks_mb + megabytes).max(0.0);
        before != (own.checks_mb > 0.0) && megabytes > 0.0
    };
    if changed && let Some((base, extra)) = reserved_mb() {
        let _ = try_hold(base, extra, false);
    }
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

/// How long a run that grew past its reservation waits in the ledger for
/// the room to raise it before it ends with exit 4, unless the memory
/// available less what the other runs still grow by leaves the 2 GB to
/// keep (`FLOOR_KEEP_MB`; `overgrown_ends`, since 2026-10-27): the drain's
/// bound before it begins (`DEFAULT_DRAIN_AFTER_SECONDS`).
pub const GROWTH_WAIT_SECONDS: f64 = DEFAULT_DRAIN_AFTER_SECONDS;

/// Whether a run past its reservation that has waited `waited` seconds for
/// the room to raise it ends now (since 2026-10-27), and why: when the
/// memory available (`available_mb`, MemAvailable) is below
/// `FLOOR_KEEP_MB` (the kernel reclaims hard and its OOM killer would end
/// a process, perhaps another one), at once; when it has waited
/// `bound` seconds and the memory available less what the other runs
/// still grow by (`available_less_growth_mb`) is below `FLOOR_KEEP_MB` or
/// unknown (the others growing into their reservations would bring the
/// kernel there). Otherwise it goes on past its reservation, waiting in
/// the ledger: the others' admissions see its memory in MemAvailable.
/// None: it goes on.
pub fn overgrown_ends(
    waited: f64,
    available_mb: Option<f64>,
    available_less_growth_mb: Option<f64>,
    bound: f64,
) -> Option<String> {
    if let Some(available) = available_mb.filter(|available| *available < FLOOR_KEEP_MB) {
        return Some(format!(
            "the memory available fell to {available:.0} MB, below the {FLOOR_KEEP_MB:.0} MB to keep (the kernel would otherwise end a process)"
        ));
    }
    if waited < bound {
        return None;
    }
    match available_less_growth_mb {
        Some(view) if view >= FLOOR_KEEP_MB => None,
        Some(view) => Some(format!(
            "no room came in {waited:.0} s, and the memory available less what the other runs still grow by, {view:.0} MB, is below the {FLOOR_KEEP_MB:.0} MB to keep"
        )),
        None => Some(format!(
            "no room came in {waited:.0} s (the memory available is unknown)"
        )),
    }
}

/// The memory available less what the other runs in the ledger still grow
/// by (`available_less_growth`), for the watchdog; None when unknown.
fn others_available_less_growth() -> Option<f64> {
    let live = {
        let guard = own();
        let own = guard.as_ref()?;
        own.ledger.live().ok()?
    };
    let me = std::process::id();
    let others: Vec<&Reservation> = live.iter().filter(|r| r.pid != me).collect();
    available_less_growth(&others)
}

/// Starts the thread that holds the process to its reservation (once).
///
/// A run that grows to nine tenths of its reservation asks for more (1.2
/// times its resident size); one that has grown past it and finds no room
/// waits for the room in the ledger (since 2026-10-27): its entry waits for
/// the raise as of when it began holding (`waiting_since`: its request is
/// as old as its admission, so newer requests leave it its room and the
/// ledger drains for it, `order_allows`, `drain_state`), it asks again
/// every `WAIT_POLL` and runs on meanwhile; it ends with exit 4 only when
/// `overgrown_ends` says so. Until then it ended at once: Quanta75 in the
/// nightly of 2026-10-27, 59 MB past its 1415 MB, with 21 runs in the
/// ledger that ended within minutes.
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
            // Since when the process is past its reservation without room,
            // and when it last asked.
            let mut over: Option<Instant> = None;
            let mut asked = Instant::now();
            loop {
                std::thread::sleep(WATCH_POLL);
                let Some((base, extra)) = reserved_mb() else {
                    return;
                };
                let resident = crate::board_router::resident_mb();
                if base <= 0.0 || resident < base * 0.9 {
                    if let Some(at) = over.take() {
                        // Back within its reservation: the entry no longer
                        // waits.
                        let _ = try_hold(base, extra, false);
                        eprintln!(
                            "memory ledger: back within the reservation of {base:.0} MB after {:.0} s past it",
                            at.elapsed().as_secs_f64()
                        );
                    }
                    continue;
                }
                let past = resident > base;
                if let Some(at) = over.filter(|_| past) {
                    // Waiting: the kernel's margin every poll, the ledger
                    // every `WAIT_POLL`.
                    let available = crate::board_router::memory_available_mb();
                    if available.is_some_and(|available| available < FLOOR_KEEP_MB) {
                        let reason = overgrown_ends(at.elapsed().as_secs_f64(), available, None, f64::INFINITY);
                        exceeded(resident, base, Some((at, reason.unwrap_or_default())));
                    }
                    if asked.elapsed() < WAIT_POLL {
                        continue;
                    }
                }
                asked = Instant::now();
                // Growth the estimate did not foresee: more while the
                // ledger has room; past the reservation, waiting for it.
                let raised = (resident * 1.2).max(base * 1.2).ceil();
                if past {
                    // Its request is as old as its admission.
                    if let Some(own) = own().as_mut()
                        && own.waiting_since.is_none()
                    {
                        own.waiting_since = own.held_since;
                    }
                }
                if matches!(try_hold(raised, extra, past), Try::Held) {
                    if let Some(at) = over.take() {
                        eprintln!(
                            "memory ledger: reserved {:.0} MB after {:.0} s past the reservation of {base:.0} MB",
                            raised + extra,
                            at.elapsed().as_secs_f64()
                        );
                    }
                    continue;
                }
                if !past {
                    continue;
                }
                let at = *over.get_or_insert_with(|| {
                    let capacity = capacity_mb().unwrap_or(f64::INFINITY);
                    eprintln!(
                        "memory ledger: the process grew to {resident:.0} MB resident, past its reservation of {base:.0} MB, and the ledger has no room for more now ({capacity:.0} MB for every run; pcb-maker ledger lists them): it runs on and waits for the room (ending only when the memory available falls below {:.1} GB, or after {GROWTH_WAIT_SECONDS:.0} s when the memory available less what the other runs still grow by does)",
                        FLOOR_KEEP_MB / 1024.0
                    );
                    Instant::now()
                });
                let waited = at.elapsed().as_secs_f64();
                if waited < GROWTH_WAIT_SECONDS {
                    continue;
                }
                if let Some(reason) = overgrown_ends(
                    waited,
                    crate::board_router::memory_available_mb(),
                    others_available_less_growth(),
                    GROWTH_WAIT_SECONDS,
                ) {
                    exceeded(resident, base, Some((at, reason)));
                }
            }
        });
}

/// Ends the run that grew to `resident` MB past its reservation of `base`
/// MB (exit 4, the verdict saying so); `waited`: since when it waited for
/// room and why it ends.
fn exceeded(resident: f64, base: f64, waited: Option<(Instant, String)>) -> ! {
    let capacity = capacity_mb().unwrap_or(f64::INFINITY);
    let path = own()
        .as_ref()
        .map(|own| own.ledger.path.display().to_string())
        .unwrap_or_default();
    let waited = waited.map_or(String::new(), |(at, reason)| {
        format!(
            "; it waited {:.0} s for room and ended because {reason}",
            at.elapsed().as_secs_f64()
        )
    });
    let message = format!(
        "memory: the process grew to {resident:.0} MB resident, past its reservation of {base:.0} MB in the memory ledger ({path}), and the ledger had no room for more ({capacity:.0} MB for every run; pcb-maker ledger lists them){waited}; rerun when fewer runs share the machine"
    );
    if let Some(handler) = EXCEEDED.get() {
        handler(&message);
    }
    eprintln!("error: {message}");
    release();
    std::process::exit(crate::EXIT_INTERNAL as i32);
}

/// The memory available a harness's measured floor keeps beside every
/// run's expected peak (since 2026-10-27, `measured_floor_mb`): below it
/// the kernel reclaims and swaps hard.
pub const FLOOR_KEEP_MB: f64 = 2048.0;

/// How far ahead of a sample the outside memory's rise is measured
/// (`outside_rises`).
pub const HEADROOM_WINDOW_SECONDS: f64 = 600.0;

/// The quantile of the outside memory's rises the floor covers.
pub const HEADROOM_QUANTILE: f64 = 0.95;

/// How many samples the floor needs before it is measured (an hour at
/// one every 10 s), and how old the newest may be.
pub const HEADROOM_MIN_SAMPLES: usize = 360;
pub const HEADROOM_MAX_AGE_SECONDS: f64 = 24.0 * 3600.0;

/// What the floor reads of the ledger's headroom samples
/// (`<ledger>.headroom.json`, `summary`; benchmarks/harness_common.py,
/// `headroom_summary`, writes it).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HeadroomSummary {
    /// The newest sample's time (seconds since the epoch).
    #[serde(default)]
    pub updated: Option<f64>,
    #[serde(default)]
    pub samples: usize,
    #[serde(default)]
    pub span_seconds: Option<f64>,
    /// The `HEADROOM_QUANTILE` of the outside memory's rises within
    /// `HEADROOM_WINDOW_SECONDS` of a sample.
    #[serde(default)]
    pub outside_growth_mb: Option<f64>,
    /// The median memory outside the ledger.
    #[serde(default)]
    pub outside_mb: Option<f64>,
    /// The 0.95 quantile of the runs' memory above their reservations.
    #[serde(default)]
    pub over_mb: Option<f64>,
    /// The least MemAvailable seen.
    #[serde(default)]
    pub available_min_mb: Option<f64>,
}

impl Ledger {
    /// The headroom samples beside the ledger (`<ledger>.headroom.json`).
    pub fn headroom_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".headroom.json");
        PathBuf::from(name)
    }

    /// The samples' summary (None: no file, or it does not read).
    pub fn headroom(&self) -> Option<HeadroomSummary> {
        let text = fs::read_to_string(self.headroom_path()).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        serde_json::from_value(value.get("summary")?.clone()).ok()
    }
}

/// The `quantile` of `values` by nearest rank (the smallest value with at
/// least that share at or below it); None for none.
pub fn nearest_rank(values: &[f64], quantile: f64) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((quantile * sorted.len() as f64).ceil() as usize).max(1);
    sorted.get(rank - 1).copied()
}

/// For each sample (time, memory outside the ledger; by time) the most
/// the outside memory rose within `window_seconds` after it (at least 0);
/// a sample with no later one within the window has none.
pub fn outside_rises(samples: &[(f64, f64)], window_seconds: f64) -> Vec<f64> {
    let mut rises = Vec::new();
    for (index, &(at, outside)) in samples.iter().enumerate() {
        let highest = samples[index + 1..]
            .iter()
            .take_while(|(later, _)| later - at <= window_seconds)
            .map(|&(_, megabytes)| megabytes)
            .reduce(f64::max);
        if let Some(highest) = highest {
            rises.push((highest - outside).max(0.0));
        }
    }
    rises
}

/// Whether the samples' `summary` is fresh: at least `HEADROOM_MIN_SAMPLES`
/// samples whose newest is at most `HEADROOM_MAX_AGE_SECONDS` old (the
/// floor and the capacity read only such).
pub fn headroom_fresh(summary: &HeadroomSummary, now: f64) -> bool {
    summary.samples >= HEADROOM_MIN_SAMPLES
        && now - summary.updated.unwrap_or(0.0) <= HEADROOM_MAX_AGE_SECONDS
}

/// A harness's floor of available memory from measured headroom (since
/// 2026-10-27): `FLOOR_KEEP_MB` + the outside memory's growth (`summary`'s
/// `outside_growth_mb`) + `overshoot_mb`, the board's tier's 0.95 quantile
/// of measured peak less declared (estimates.json, `overshoot`; at least
/// 0); `DEFAULT_FLOOR_MB` without `HEADROOM_MIN_SAMPLES` samples whose
/// newest is at most `HEADROOM_MAX_AGE_SECONDS` old. With what it is made
/// of. Until then the floor was 10 GB by opinion: on the evening of
/// 2026-10-26 it held a 3 GB board for 4371 s with 9.4 GB beside the
/// reservations' growth. benchmarks/harness_common.py,
/// `measured_floor_mb`: the same rule (run.py's floor unless
/// `--min-free-gb` is given).
pub fn measured_floor_mb(
    summary: Option<&HeadroomSummary>,
    overshoot_mb: f64,
    now: f64,
) -> (f64, String) {
    let count = summary.map_or(0, |summary| summary.samples);
    let growth = summary
        .filter(|summary| headroom_fresh(summary, now))
        .and_then(|summary| summary.outside_growth_mb);
    match growth {
        Some(growth) => {
            let (growth, overshoot) = (growth.max(0.0), overshoot_mb.max(0.0));
            (
                FLOOR_KEEP_MB + growth + overshoot,
                format!(
                    "measured: {:.1} GB to keep + {:.1} GB outside growth + {:.1} GB overshoot ({count} samples)",
                    FLOOR_KEEP_MB / 1024.0,
                    growth / 1024.0,
                    overshoot / 1024.0
                ),
            )
        }
        _ => (
            DEFAULT_FLOOR_MB,
            format!(
                "the default {:.0} GB ({count} samples)",
                DEFAULT_FLOOR_MB / 1024.0
            ),
        ),
    }
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
                        && drained_for(&others, pid, since, now, Some(CAPACITY), Some(available))
                            .is_none_or(|waiter| {
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
                let end = nightly.then_some(now + 1500.0);
                // A binary's request is never the nightly's (order_allows
                // and drained_for rank it after them); the harness's
                // nightly boards rank by their priority: the first of them
                // is behind no one.
                let first = first_waiter(&book.iter().map(|(r, _, _)| r).collect::<Vec<_>>())
                    .is_some_and(|r| r.pid == pid);
                let admitted = fits
                    && (first || order_allows(&others, pid, since, wanted, Some(CAPACITY)))
                    && (first
                        || drained_for(&others, pid, since, now, Some(CAPACITY), Some(available))
                            .is_none_or(|waiter| {
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
        assert!(drained_for(&others, 99, 30.0, 609.0, capacity, None).is_none());
        assert!(drained_for(&others, 99, 5.0, 700.0, capacity, None).is_none());
        let drained = drained_for(&others, 99, 30.0, 610.0, capacity, None).expect("drained");
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

    /// The ledger of 2026-10-26 (58 066 MB): the old nightly harness's
    /// waiter (356c8db: no `floor_mb`, no `refreshed`, `waits_on_floor`) of
    /// 24 724 MB waiting since 0 s, 26 GB reserved by two runs (the smaller
    /// 3 GB, holding for another hour), 31 000 MB available less the
    /// reservations' growth, and a request of 7 GB (katia).
    #[test]
    fn the_ledger_drains_only_for_a_waiter_it_could_start() {
        let holder = |pid: u32, megabytes: f64, held_since: f64, end: f64| Reservation {
            pid,
            started: held_since,
            megabytes,
            held_since: Some(held_since),
            expected_end: Some(end),
            ..Default::default()
        };
        let waiter = Reservation {
            pid: 4_134_524,
            started: 0.0,
            label: "run.py: Goga64__ULK__ULK_sl_PG1316s (expected peak)".into(),
            waiting_mb: Some(24_724.0),
            waiting_since: Some(0.0),
            waits_on_floor: true,
            ..Default::default()
        };
        let (capacity, available, katia, now) = (Some(58_066.0), Some(31_000.0), 7168.0, 1200.0);
        // An old entry without a floor: run.py's 10 GB assumed.
        assert_eq!(waiter.assumed_floor_mb(), DEFAULT_FLOOR_MB);
        // The runs began holding before its wait began.
        let book = [
            holder(11, 3072.0, -600.0, now + 3600.0),
            holder(12, 23_552.0, -600.0, now + 7200.0),
            waiter.clone(),
        ];
        let others = refs(&book);
        // The drain of 2026-10-25 held the 7 GB request back: after it the
        // waiter's room (58 066 - 26 624 - 7168, 31 000 - 7168) falls short
        // of 24 724; it is larger than the cap and ends after the 3 GB run.
        assert!(!drain_allows(
            &book[2],
            &others,
            katia,
            capacity,
            available,
            Some(now + 4000.0)
        ));
        // Now: with every run since its wait began gone (none) its room
        // under its floor is 31 000 - 10 240 = 20 760 MB: the ledger
        // cannot start it, the drain ends, and the 7 GB request goes by
        // ordered admission (no newer run: 7168 + 24 724 of 58 066).
        assert_eq!(
            drain_state(&book[2], &others, now, capacity, available),
            Drain::CannotStart(20_760.0)
        );
        assert!(drained_for(&others, 99, now, now, capacity, available).is_none());
        assert!(order_allows(&others, 99, now, katia, capacity));
        // The live ledger's case: every run began after its wait (24 GB).
        // With them gone it would start (31 000 + 24 576 - 10 240): the
        // ledger drains for it and holds the 7 GB request back ...
        let book = [
            holder(11, 3072.0, 600.0, now + 3600.0),
            holder(12, 21_504.0, 600.0, now + 7200.0),
            waiter.clone(),
        ];
        let others = refs(&book);
        assert_eq!(
            drain_state(&book[2], &others, now, capacity, available),
            Drain::Drains
        );
        let drained = drained_for(&others, 99, now, now, capacity, available).expect("drained");
        assert!(!drain_allows(
            drained, &others, katia, capacity, available, None
        ));
        // ... for at most 2 hours of its wait; then the 7 GB request goes
        // by ordered admission (24 576 + 7168 + 24 724 of 58 066), and the
        // waiter keeps its place: 16 GB (ducktop2) still waits.
        let later = DEFAULT_DRAIN_MAX_SECONDS + 1.0;
        assert_eq!(
            drain_state(&book[2], &others, later, capacity, available),
            Drain::PastBound(DEFAULT_DRAIN_MAX_SECONDS)
        );
        assert!(drained_for(&others, 99, later, later, capacity, available).is_none());
        assert!(order_allows(&others, 99, later, katia, capacity));
        assert!(!order_allows(&others, 99, later, 16_384.0, capacity));
        // An entry's own bound.
        let patient = Reservation {
            drain_max: Some(10_000.0),
            ..waiter.clone()
        };
        assert_eq!(
            drain_state(&patient, &others, later, capacity, available),
            Drain::Drains
        );
        // A new writer's entry (`refreshed`) names its floor or has none,
        // and one not rewritten for more than 120 s is stale.
        let fresh = Reservation {
            refreshed: Some(now - STALE_AFTER_SECONDS),
            ..waiter.clone()
        };
        assert_eq!(fresh.assumed_floor_mb(), 0.0);
        assert_eq!(
            drain_state(&fresh, &others, now, capacity, available),
            Drain::Drains
        );
        let stale = Reservation {
            refreshed: Some(now - STALE_AFTER_SECONDS - 1.0),
            ..waiter.clone()
        };
        assert_eq!(
            drain_state(&stale, &others, now, capacity, available),
            Drain::Stale(STALE_AFTER_SECONDS + 1.0)
        );
        // Before its drain_after nothing drains.
        assert_eq!(
            drain_state(&book[2], &others, 599.0, capacity, available),
            Drain::NotYet
        );
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
        let drained = drained_for(&others, 99, 21.0, 21.0, None, None).expect("drained at once");
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
            "floor_mb": 10240.0, "drain_after": 300.0, "drain_max": 900.0, "refreshed": 2.5,
            "a_later_field": [1, 2]
        }]});
        fs::write(&path, text.to_string()).unwrap();
        let ledger = Ledger::at(&path);
        let live = ledger.live().unwrap();
        assert_eq!(live[0].expected_end, Some(900.5));
        assert!(live[0].nightly());
        assert_eq!(live[0].floor_mb, Some(10240.0));
        assert_eq!(live[0].drain_after, Some(300.0));
        assert_eq!(live[0].drain_max, Some(900.0));
        assert_eq!(live[0].refreshed, Some(2.5));
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
        assert_eq!(first["refreshed"], 2.5);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_capacity_is_at_most_the_machines_memory_less_the_margin() {
        // Read without the variables set (cargo runs tests side by side:
        // only read here): the machine's rule with the live ledger's
        // samples, bounded by MemTotal less the margin.
        if std::env::var_os("PCB_MAKER_LEDGER_TOTAL_MB").is_none()
            && std::env::var_os("PCB_MAKER_LEDGER_MARGIN_MB").is_none()
            && let Some(total) = crate::board_router::memory_total_mb()
        {
            let capacity = capacity_mb().unwrap();
            assert!((0.0..=(total - DEFAULT_MARGIN_MB).max(0.0)).contains(&capacity));
        }
    }

    /// The evening of 2026-10-26 (21:39), second by second from the
    /// fresh agent's check (0 s): a ledger of 58 066 MB, 44 000 MB
    /// available beside the reservations (each run resident at its
    /// reservation; the logs: MemAvailable 30-31 GB less 18-21 GB of
    /// growth beside 30-34 GB reserved, about 19 GB outside the ledger),
    /// eight agents' runs of 3875 MB that began holding during the waiter's
    /// wait and end one every 120 s from 600 s, and run.py's board of
    /// 3004 MB (pid 3598969's stonedDiscord, `--min-free-gb 10`) waiting
    /// since -4312 s and looking every 15 s. The check (2253 MB: KiCad's
    /// 2.2 GB) arrives at 0 s, other agents' checks every 20 s from 10 s
    /// (each 2 s), an agent's route of 2000 MB at 30 s (300 s); the
    /// binaries look every 2 s. `short_rule`: short requests pass
    /// (`short_allows`). Returns the check's wait, the longest wait of the
    /// other checks, and the starts of the board and the route.
    /// test_run.py's `evening`: the same.
    fn evening(short_rule: bool) -> (u32, u32, u32, u32) {
        const CAPACITY: f64 = 58_066.0;
        const FREE: f64 = 44_000.0;
        const FLOOR: f64 = 10_240.0;
        const CHECK: f64 = KICAD_CHECK_MB;
        const WAITER: u32 = 1;
        const ROUTE: u32 = 60;
        let horizon: i64 = 1800;
        // (reservation, end once it holds, short)
        let mut book: Vec<(Reservation, Option<i64>, bool)> = (0..8)
            .map(|i| {
                (
                    Reservation {
                        pid: 11 + i,
                        megabytes: 3875.0,
                        label: "layout".into(),
                        held_since: Some(-1000.0),
                        ..Default::default()
                    },
                    Some(600 + 120 * i64::from(i)),
                    false,
                )
            })
            .collect();
        book.push((
            Reservation {
                pid: WAITER,
                started: -4312.0,
                label: "run.py: stonedDiscord__nonSNES__SNSP-CPU-01 (expected peak)".into(),
                waiting_mb: Some(3004.0),
                waiting_since: Some(-4312.0),
                waits_on_floor: true,
                floor_mb: Some(FLOOR),
                drain_after: Some(600.0),
                refreshed: Some(-1.0),
                ..Default::default()
            },
            None,
            false,
        ));
        let (mut check_wait, mut longest, mut waiter_start, mut route_start) =
            (None, 0, None, None);
        let mut next_pid = 100;
        for t in 0..horizon {
            let now = t as f64;
            book.retain(|(_, end, _)| end.is_none_or(|end| end > t));
            let arrive = |pid: u32, megabytes: f64, label: &str| Reservation {
                pid,
                started: now,
                label: label.into(),
                waiting_mb: Some(megabytes),
                waiting_since: Some(now),
                refreshed: Some(now),
                ..Default::default()
            };
            if t == 0 {
                book.push((arrive(50, CHECK, "drc-kicad-board"), None, true));
            }
            if t >= 10 && (t - 10) % 20 == 0 {
                next_pid += 1;
                book.push((arrive(next_pid, CHECK, "drc-kicad-board"), None, true));
            }
            if t == 30 {
                book.push((arrive(ROUTE, 2000.0, "route-kicad-board"), None, false));
            }
            let lookers: Vec<u32> = book
                .iter()
                .filter(|(r, _, _)| r.waiting_mb.is_some())
                .filter(|(r, _, _)| {
                    let every = if r.pid == WAITER { 15 } else { 2 };
                    ((now - r.waiting_since()) as i64) % every == 0
                })
                .map(|(r, _, _)| r.pid)
                .collect();
            for pid in lookers {
                let index = book.iter().position(|(r, _, _)| r.pid == pid).unwrap();
                book[index].0.refreshed = Some(now);
                let (me, short) = (book[index].0.clone(), book[index].2);
                let (wanted, since) = (me.waiting_mb.unwrap(), me.waiting_since());
                let others: Vec<&Reservation> = book
                    .iter()
                    .map(|(r, _, _)| r)
                    .filter(|r| r.pid != pid)
                    .collect();
                let reserved: f64 = others.iter().map(|r| r.megabytes).sum();
                let available = FREE - reserved;
                let admitted = if pid == WAITER {
                    // The harness's rule (harness_common.memory_room).
                    (reserved + wanted <= CAPACITY && available - wanted >= FLOOR)
                        || !others.iter().any(|r| r.megabytes > 0.0)
                } else {
                    // A binary's (try_hold).
                    let fits =
                        reserved + wanted <= CAPACITY || !others.iter().any(|r| r.megabytes > 0.0);
                    let ordered = order_allows(&others, pid, since, wanted, Some(CAPACITY))
                        && drained_for(&others, pid, since, now, Some(CAPACITY), Some(available))
                            .is_none_or(|waiter| {
                                drain_allows(
                                    waiter,
                                    &others,
                                    wanted,
                                    Some(CAPACITY),
                                    Some(available),
                                    None,
                                )
                            });
                    let passes = short_rule
                        && short
                        && short_request(0.0, wanted)
                        && short_allows(wanted, Some(available));
                    fits && (ordered || passes)
                };
                if admitted {
                    let entry = &mut book[index];
                    entry.0.megabytes = wanted;
                    entry.0.waiting_mb = None;
                    entry.0.waiting_since = None;
                    entry.0.held_since = Some(now);
                    entry.0.short = short_rule && short;
                    let waited = (now - since) as u32;
                    entry.1 = Some(
                        t + match pid {
                            WAITER => 3600,
                            ROUTE => 300,
                            _ => 2,
                        },
                    );
                    match pid {
                        WAITER => waiter_start = Some(t as u32),
                        ROUTE => route_start = Some(t as u32),
                        50 => check_wait = Some(waited),
                        _ => longest = longest.max(waited),
                    }
                }
            }
        }
        (
            check_wait.unwrap(),
            longest,
            waiter_start.unwrap(),
            route_start.unwrap(),
        )
    }

    #[test]
    fn short_requests_never_wait_behind_a_drain() {
        // As on the evening: the board waits for its floor (44 000 - 31 000
        // - 3004 < 10 240) and the drain holds every new request for it
        // (with the runs since its wait began gone it could start; no run
        // is larger than 4 GB) until the first run ends at 600 s: the check
        // waits 600 s, the other checks up to 592 s, the route goes at
        // 606 s behind them, the board at its next look (608 s).
        assert_eq!(evening(false), (600, 592, 608, 606));
        // Short requests pass: every check goes at once (13 000 MB
        // available less growth), the board starts as before, the route at
        // 600 s (no check queued ahead of it; the checks count as gone for
        // its order and drain). test_run.py: the same numbers.
        assert_eq!(evening(true), (0, 0, 608, 600));
    }

    #[test]
    fn a_short_request_needs_the_memory_and_never_passes_in_the_bookkeeping() {
        assert!(short_request(0.0, KICAD_CHECK_MB));
        assert!(short_request(SHORT_OWN_MB, KICAD_CHECK_MB));
        assert!(!short_request(SHORT_OWN_MB + 1.0, 0.0));
        assert!(!short_request(0.0, KICAD_CHECK_MB + 1.0));
        // The memory available less growth must cover it; unknown: there.
        assert!(short_allows(2253.0, Some(2253.0)));
        assert!(!short_allows(2253.0, Some(2252.0)));
        assert!(short_allows(2253.0, None));
        // The command's class and its override.
        assert!(short_override(None, true));
        assert!(!short_override(None, false));
        assert!(short_override(Some(" 1"), false));
        assert!(!short_override(Some("0"), true));
        assert!(short_override(Some("yes"), true));
        // A short run beside a waiter: the order and the drain for a route
        // read the ledger as without it (its megabytes back), the
        // capacity counts it.
        let holder = Reservation {
            pid: 11,
            megabytes: 4000.0,
            held_since: Some(20.0),
            ..Default::default()
        };
        let check = Reservation {
            pid: 12,
            megabytes: 2253.0,
            held_since: Some(700.0),
            short: true,
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
        let without = [holder.clone(), waiter.clone()];
        let with = [holder.clone(), check.clone(), waiter.clone()];
        let (capacity, now) = (Some(14_500.0), 700.0);
        for wanted in [1000.0, 2000.0, 2500.0, 2600.0] {
            assert_eq!(
                order_allows(&refs(&with), 99, now, wanted, capacity),
                order_allows(&refs(&without), 99, now, wanted, capacity),
                "{wanted}"
            );
            // 12 000 available less growth without the check: 2000 leave
            // the waiter 12 000 - 2000 - 2000 = 8000.
            let w = refs(&with);
            let drained = drained_for(&w, 99, now, now, capacity, Some(12_000.0 - 2253.0)).unwrap();
            let o = refs(&without);
            let drained_o = drained_for(&o, 99, now, now, capacity, Some(12_000.0)).unwrap();
            assert_eq!(
                drain_allows(drained, &w, wanted, capacity, Some(12_000.0 - 2253.0), None),
                drain_allows(drained_o, &o, wanted, capacity, Some(12_000.0), None),
                "{wanted}"
            );
        }
        assert!(order_allows(&refs(&with), 99, now, 2500.0, capacity));
        assert!(!order_allows(&refs(&with), 99, now, 2501.0, capacity));
        // The same reservation not short counts: 4000 + 2253 + 1000 + 8000
        // is more than 14 500.
        let counted = [
            holder,
            Reservation {
                short: false,
                ..check
            },
            waiter,
        ];
        assert!(!order_allows(&refs(&counted), 99, now, 1000.0, capacity));
        // The flag survives a rewrite and is left out where false.
        let text = serde_json::to_string(&with[1]).unwrap();
        assert!(text.contains("\"short\":true"), "{text}");
        assert!(!serde_json::to_string(&with[0]).unwrap().contains("short"));
    }

    /// The floor from measured headroom (since 2026-10-27): test_run.py's
    /// `MeasuredFloor`, the same numbers.
    #[test]
    fn the_floor_is_measured_from_the_headroom_samples() {
        let samples = [
            (0.0, 19_000.0),
            (10.0, 19_500.0),
            (300.0, 22_400.0),
            (700.0, 19_100.0),
            (1400.0, 19_000.0),
        ];
        let rises = outside_rises(&samples, 600.0);
        assert_eq!(rises, vec![3400.0, 2900.0, 0.0]);
        assert_eq!(nearest_rank(&rises, 0.95), Some(3400.0));
        assert_eq!(nearest_rank(&rises, 0.5), Some(2900.0));
        assert_eq!(nearest_rank(&[], 0.95), None);
        let now = 1_793_000_000.0;
        let summary = HeadroomSummary {
            samples: 360,
            updated: Some(now - 60.0),
            outside_growth_mb: Some(3123.0),
            ..Default::default()
        };
        assert_eq!(measured_floor_mb(Some(&summary), 91.0, now).0, 5262.0);
        assert_eq!(measured_floor_mb(Some(&summary), -500.0, now).0, 5171.0);
        let few = HeadroomSummary {
            samples: 359,
            ..summary.clone()
        };
        assert_eq!(measured_floor_mb(Some(&few), 91.0, now).0, DEFAULT_FLOOR_MB);
        let old = HeadroomSummary {
            updated: Some(now - 86_401.0),
            ..summary.clone()
        };
        assert_eq!(measured_floor_mb(Some(&old), 91.0, now).0, DEFAULT_FLOOR_MB);
        assert_eq!(
            measured_floor_mb(None, 91.0, now),
            (DEFAULT_FLOOR_MB, "the default 10 GB (0 samples)".into())
        );
        assert_eq!(
            measured_floor_mb(Some(&summary), 91.0, now).1,
            "measured: 2.0 GB to keep + 3.0 GB outside growth + 0.1 GB overshoot (360 samples)"
        );
        // The summary as harness_common.record_headroom writes it, beside
        // the ledger.
        let directory = scratch("headroom");
        let ledger = Ledger::at(directory.join("ledger.json"));
        assert_eq!(ledger.headroom(), None);
        fs::write(
            ledger.headroom_path(),
            r#"{"summary": {"updated": 1011.0, "samples": 2, "span_seconds": 11.0, "outside_growth_mb": 1000, "outside_mb": 19000, "over_mb": 0, "available_min_mb": 30000, "newer": 1}, "samples": [[1000.0, 30000, 19000, 0, 0, 0]]}"#,
        )
        .unwrap();
        let read = ledger.headroom().unwrap();
        assert_eq!((read.samples, read.outside_growth_mb), (2, Some(1000.0)));
        assert_eq!(
            ledger.headroom_path().file_name().unwrap(),
            "ledger.json.headroom.json"
        );
        let _ = fs::remove_dir_all(directory);
    }

    /// A harness's boards replayed against the machine as it was without
    /// them (since 2026-10-27; test_run.py's `MeasuredFloor.replay`: the
    /// same). `knots`: (time, the floor's view, MemAvailable less growth,
    /// and MemAvailable of the machine without these boards, in MB), each
    /// holding until the next; `boards`: (since, expected MB, seconds,
    /// resident MB: its measured peak). A board looks every `poll` s from
    /// `since` and starts when the view less the expected peaks of the
    /// boards running less its own leaves `floor_mb`; it then holds its
    /// expected peak in the view and its measured peak resident for its
    /// seconds. Returns the starts (None: not within `horizon`), the seconds
    /// waited in all, how many started and the least MemAvailable.
    /// `capacity` (since 2026-10-27): the ledger's capacity and, per knot,
    /// what the other runs reserved; a board then starts only when those,
    /// the boards running and its own fit it as well.
    fn floor_replay(
        knots: &[(u32, f64, f64)],
        boards: &[(u32, f64, u32, f64)],
        floor_mb: f64,
        horizon: u32,
        poll: u32,
        capacity: Option<(f64, &[f64])>,
    ) -> (Vec<Option<u32>>, u32, usize, f64) {
        let mut starts: Vec<Option<u32>> = vec![None; boards.len()];
        let mut lowest = f64::INFINITY;
        let mut knot = 0;
        for t in 0..horizon {
            while knot + 1 < knots.len() && knots[knot + 1].0 <= t {
                knot += 1;
            }
            let (_, view, available) = knots[knot];
            let room = capacity.map_or(f64::INFINITY, |(capacity, others)| capacity - others[knot]);
            let (mut reserved, mut resident) = (0.0, 0.0);
            for (&(_, mb, seconds, peak), start) in boards.iter().zip(&starts) {
                if start.is_some_and(|start| start <= t && t < start + seconds) {
                    reserved += mb;
                    resident += peak;
                }
            }
            for (index, &(since, mb, _, peak)) in boards.iter().enumerate() {
                if starts[index].is_none()
                    && t >= since
                    && (t - since) % poll == 0
                    && view - reserved - mb >= floor_mb
                    && reserved + mb <= room
                {
                    starts[index] = Some(t);
                    reserved += mb;
                    resident += peak;
                }
            }
            lowest = lowest.min(available - resident);
        }
        let waited = boards
            .iter()
            .zip(&starts)
            .map(|(board, start)| start.unwrap_or(horizon) - board.0)
            .sum();
        let started = starts.iter().filter(|start| start.is_some()).count();
        (starts, waited, started, lowest)
    }

    /// The measured floor of a layout board (2026-10-27: `FLOOR_KEEP_MB` +
    /// 6886, the 0.95 quantile of the outside memory's rises within 10
    /// minutes over three hours of samples, + 56, the layout kind's
    /// overshoot in estimates.json).
    const MEASURED_FLOOR_MB: f64 = FLOOR_KEEP_MB + 6886.0 + 56.0;

    /// The evening of 2026-10-26 (20:37, build/pour6-dense-off-x1844.err):
    /// run.py's four boards (protoesp, video, DSP-ADAU1452, SNSP-CPU-01:
    /// expected 587, 4389, 1717, 3004 MB; measured 1205, 1731, 1850,
    /// 1632 s and peaks of 232, 2439, 1033, 1108 MB; in the order they
    /// started) waited from 0 with `--min-free-gb 10`. The machine without
    /// them, from the log's lines (MemAvailable less growth with the boards
    /// that ran added back at their expected peaks, MemAvailable with their
    /// measured peaks); between two lines at most what kept the boards that
    /// still waited out under 10 GB. test_run.py's
    /// `test_the_evening_replayed_with_the_measured_floor`: the same.
    const EVENING_KNOTS: [(u32, f64, f64); 5] = [
        (0, 9626.0, 30822.0),
        (2210, 12539.0, 35328.0),
        (3364, 15770.0, 33102.0),
        (4235, 16486.0, 35617.0),
        (4371, 19558.0, 35626.0),
    ];
    const EVENING_BOARDS: [(u32, f64, u32, f64); 4] = [
        (0, 587.0, 1205, 232.0),
        (0, 4389.0, 1731, 2439.0),
        (0, 1717.0, 1850, 1033.0),
        (0, 3004.0, 1632, 1108.0),
    ];

    fn last_end(starts: &[Option<u32>], boards: &[(u32, f64, u32, f64)]) -> Option<u32> {
        starts
            .iter()
            .zip(boards)
            .map(|(start, board)| start.map(|start| start + board.2))
            .max()
            .flatten()
    }

    #[test]
    fn the_evening_replayed_with_the_measured_floor() {
        // With 10 GB the replay gives the evening's starts (2210, 3364,
        // 4235, 4371 s, at the next look): 14 220 s waited, the last board
        // ends at 6095 s (22:18 was 6045 s after 20:37).
        let (starts, waited, _, lowest) =
            floor_replay(&EVENING_KNOTS, &EVENING_BOARDS, 10_240.0, 9000, 15, None);
        assert_eq!(starts, [Some(2220), Some(3375), Some(4245), Some(4380)]);
        assert_eq!(
            (waited, last_end(&starts, &EVENING_BOARDS)),
            (14_220, Some(6095))
        );
        // The measured floor: protoesp at once, DSP at 2220, SNSP-CPU-01 at
        // 4245: 9840 s waited, the last ends at 5877 s; the least
        // MemAvailable 28.9 GB (29.7 with 10 GB), above 2 GB.
        let (measured, measured_waited, _, measured_lowest) = floor_replay(
            &EVENING_KNOTS,
            &EVENING_BOARDS,
            MEASURED_FLOOR_MB,
            9000,
            15,
            None,
        );
        assert_eq!(measured, [Some(0), Some(3375), Some(2220), Some(4245)]);
        assert_eq!(
            (measured_waited, last_end(&measured, &EVENING_BOARDS)),
            (9840, Some(5877))
        );
        assert_eq!(
            (lowest.round(), measured_lowest.round()),
            (30_431.0, 29_630.0)
        );
        assert!(measured_lowest > FLOOR_KEEP_MB);
    }

    /// The night of 2026-10-26/27 (23:39 from 0; the samples of
    /// experiments/ledger-floor/sampler.py, replay.py): the first hour's
    /// fourteen boards of more than 3 GB that only the floor of 10 GB held
    /// back (since, expected MB, measured seconds, measured peak of the
    /// process tree); the machine without them in steps of 120 s, each the
    /// least view and MemAvailable within it. test_run.py's
    /// `test_tonight_replayed_with_the_measured_floor`: the same.
    const TONIGHT_KNOTS: [(u32, f64, f64); 30] = [
        (0, 11162.0, 38620.0),
        (120, 22621.0, 38300.0),
        (240, 29525.0, 37852.0),
        (360, 33072.0, 40831.0),
        (480, 30588.0, 39844.0),
        (600, 28889.0, 39164.0),
        (720, 28736.0, 38054.0),
        (840, 24076.0, 39841.0),
        (960, 31215.0, 38906.0),
        (1080, 38250.0, 43455.0),
        (1200, 33154.0, 38319.0),
        (1320, 31957.0, 37410.0),
        (1440, 34188.0, 39803.0),
        (1560, 37543.0, 43173.0),
        (1680, 33445.0, 38991.0),
        (1800, 34963.0, 39539.0),
        (1920, 34827.0, 39380.0),
        (2040, 37082.0, 40980.0),
        (2160, 30387.0, 40701.0),
        (2280, 29594.0, 38188.0),
        (2400, 27644.0, 34748.0),
        (2520, 25104.0, 32919.0),
        (2640, 21239.0, 33207.0),
        (2760, 15488.0, 33819.0),
        (2880, 19348.0, 33042.0),
        (3000, 15891.0, 32708.0),
        (3120, 16819.0, 33132.0),
        (3240, 16410.0, 30462.0),
        (3360, 11184.0, 30540.0),
        (3480, 13777.0, 31384.0),
    ];
    const TONIGHT_BOARDS: [(u32, f64, u32, f64); 14] = [
        (81, 8982.0, 914, 4388.0),
        (81, 8285.0, 729, 4412.0),
        (81, 7733.0, 1789, 2988.0),
        (81, 7202.0, 654, 4010.0),
        (81, 6729.0, 3039, 4293.0),
        (81, 6359.0, 1944, 3955.0),
        (81, 5650.0, 2799, 2513.0),
        (860, 3584.0, 2780, 2364.0),
        (81, 4308.0, 1039, 2558.0),
        (81, 4262.0, 1584, 2559.0),
        (837, 4063.0, 683, 832.0),
        (877, 4002.0, 2148, 3096.0),
        (1044, 3888.0, 411, 4590.0),
        (1028, 3584.0, 2241, 1417.0),
    ];

    #[test]
    fn tonight_replayed_with_the_measured_floor() {
        // 10 GB: 9 of the 14 start within the hour, 20 766 s waited; the
        // measured floor: 10, 18 376 s; the least MemAvailable 20.0 and
        // 17.5 GB, above 2 GB.
        let (_, waited, started, lowest) =
            floor_replay(&TONIGHT_KNOTS, &TONIGHT_BOARDS, 10_240.0, 3600, 15, None);
        assert_eq!((started, waited, lowest.round()), (9, 20_766, 20_495.0));
        let (_, waited, started, lowest) = floor_replay(
            &TONIGHT_KNOTS,
            &TONIGHT_BOARDS,
            MEASURED_FLOOR_MB,
            3600,
            15,
            None,
        );
        assert_eq!((started, waited, lowest.round()), (10, 18_376, 17_957.0));
        assert!(lowest > FLOOR_KEEP_MB);
    }

    /// This machine (2026-10-27): MemTotal 64 210 MB (62.7 GB), and the
    /// median memory outside the ledger the night of 2026-10-26/27
    /// measured, 18.1 GB (`outside_mb`), and its 5th percentile, 16.7 GB.
    const MEM_TOTAL_MB: f64 = 64_210.0;
    const OUTSIDE_P50_MB: f64 = 18.1 * 1024.0;
    const OUTSIDE_P5_MB: f64 = 16.7 * 1024.0;

    #[test]
    fn the_capacity_is_the_machines_memory_less_what_is_outside_the_ledger() {
        let now = 1_793_000_000.0;
        let summary = HeadroomSummary {
            samples: 1091,
            updated: Some(now - 60.0),
            outside_mb: Some(OUTSIDE_P50_MB),
            outside_growth_mb: Some(6886.0),
            ..Default::default()
        };
        // Measured: 64 210 - 18 534 = 45 676 MB (44.6 GB), not the static
        // bound's 58 066 (56.7 GB).
        let (capacity, how) =
            machine_capacity_mb(MEM_TOTAL_MB, DEFAULT_MARGIN_MB, Some(&summary), now);
        assert_eq!(capacity.round(), 45_676.0);
        assert_eq!(
            how,
            "MemTotal 62.7 GB less 18.1 GB outside the ledger (the median of 1091 headroom samples); at most 56.7 GB, MemTotal less the margin"
        );
        // Without fresh samples (too few, a day old, none): 25 % of
        // MemTotal outside, 48 158 MB (47.0 GB).
        let default = (MEM_TOTAL_MB * 0.75).round();
        for stale in [
            HeadroomSummary {
                samples: 359,
                ..summary.clone()
            },
            HeadroomSummary {
                updated: Some(now - 86_401.0),
                ..summary.clone()
            },
            HeadroomSummary {
                outside_mb: None,
                ..summary.clone()
            },
        ] {
            assert_eq!(
                machine_capacity_mb(MEM_TOTAL_MB, DEFAULT_MARGIN_MB, Some(&stale), now)
                    .0
                    .round(),
                default
            );
        }
        let (capacity, how) = machine_capacity_mb(MEM_TOTAL_MB, DEFAULT_MARGIN_MB, None, now);
        assert_eq!(capacity.round(), default);
        assert!(how.contains("25 % of MemTotal without fresh headroom samples"));
        // Another machine without samples: 16 GB and 24 GB keep the margin
        // (10 and 18 GB), 32 GB a quarter (24 GB), 128 GB a quarter (96 GB).
        for (total, expected) in [
            (16_384.0, 10_240.0),
            (24_576.0, 18_432.0),
            (32_768.0, 24_576.0),
            (131_072.0, 98_304.0),
        ] {
            assert_eq!(
                machine_capacity_mb(total, DEFAULT_MARGIN_MB, None, now).0,
                expected
            );
        }
        // Little outside (an idle machine): the static bound.
        let idle = HeadroomSummary {
            outside_mb: Some(2000.0),
            ..summary.clone()
        };
        assert_eq!(
            machine_capacity_mb(MEM_TOTAL_MB, DEFAULT_MARGIN_MB, Some(&idle), now).0,
            MEM_TOTAL_MB - DEFAULT_MARGIN_MB
        );
        // More outside than the machine: nothing.
        let full = HeadroomSummary {
            outside_mb: Some(70_000.0),
            ..summary
        };
        assert_eq!(
            machine_capacity_mb(MEM_TOTAL_MB, DEFAULT_MARGIN_MB, Some(&full), now).0,
            0.0
        );
    }

    #[test]
    fn a_binary_keeps_the_floor_and_runs_inside_a_harness_s_reservation() {
        // A new request of 3000 MB with 11 000 available beside the growth
        // keeps a floor of 8000, not one of 8990 (the measured one); a
        // short one keeps 2048; unknown memory or no floor: it goes.
        assert!(binary_floor_allows(3000.0, Some(11_000.0), Some(8000.0)));
        assert!(!binary_floor_allows(3000.0, Some(11_000.0), Some(8990.0)));
        assert!(binary_floor_allows(
            2253.0,
            Some(4301.0),
            Some(FLOOR_KEEP_MB)
        ));
        assert!(!binary_floor_allows(
            2253.0,
            Some(4300.0),
            Some(FLOOR_KEEP_MB)
        ));
        assert!(binary_floor_allows(3000.0, None, Some(8990.0)));
        assert!(binary_floor_allows(3000.0, Some(0.0), None));
        // The harness's reservation named by PCB_MAKER_LEDGER_INSIDE.
        assert_eq!(
            inside_key(Some("4134524 1793000000.123456")),
            Some((4_134_524, 1_793_000_000.123_456))
        );
        for bad in [
            None,
            Some(""),
            Some("12"),
            Some("x 1.0"),
            Some("12 inf"),
            Some("12 1.0 3"),
        ] {
            assert_eq!(inside_key(bad), None);
        }
        assert!(same_start(1_793_000_000.123_456, 1_793_000_000.123_457));
        assert!(!same_start(1_793_000_000.0, 1_793_000_000.01));
        // A check's entry as pcb-maker ledger and older binaries read it.
        let entry = Reservation {
            pid: 7,
            megabytes: 0.0,
            check_mb: Some(KICAD_CHECK_MB),
            checking: true,
            inside: Some(5),
            ..Default::default()
        };
        let text = serde_json::to_value(&entry).unwrap();
        assert_eq!(text["check_mb"], KICAD_CHECK_MB);
        assert_eq!(text["checking"], true);
        assert_eq!(text["inside"], 5);
        let back: Reservation = serde_json::from_value(text).unwrap();
        assert_eq!(back, entry);
        let plain = serde_json::to_value(Reservation::default()).unwrap();
        assert!(plain.get("check_mb").is_none() && plain.get("checking").is_none());
    }

    /// What the other runs reserved at the evening's knots (the log's
    /// lines: 34.5, 29.8, 28.5, 32.0 and 30.6 GB, less the four boards'
    /// reservations that ran then: protoesp 0.6 at 3364 s, video 4.3 at 4235
    /// and with DSP 1.7 at 4371).
    const EVENING_OTHERS_MB: [f64; 5] = [35_328.0, 30_515.0, 28_570.0, 28_365.0, 25_190.0];

    #[test]
    fn the_evening_and_tonight_replayed_with_the_capacity_from_the_machine() {
        // The capacity from the machine, 45 676 MB, beside the other runs'
        // reservations: the replays start the same boards at the same
        // times as with the floor alone, for both floors; the least
        // MemAvailable is the same (above 2 GB).
        let capacity = MEM_TOTAL_MB - OUTSIDE_P50_MB;
        for floor in [10_240.0, MEASURED_FLOOR_MB] {
            let alone = floor_replay(&EVENING_KNOTS, &EVENING_BOARDS, floor, 9000, 15, None);
            let with = floor_replay(
                &EVENING_KNOTS,
                &EVENING_BOARDS,
                floor,
                9000,
                15,
                Some((capacity, &EVENING_OTHERS_MB)),
            );
            assert_eq!(with, alone);
            assert!(with.3 > FLOOR_KEEP_MB);
        }
        // Tonight's knots do not carry what the others reserved: at most
        // MemTotal less the outside memory less the floor's view (a holder's
        // growth counts its own resident memory, its tree is outside the
        // outside memory), here with the outside memory at its 5th
        // percentile, 16.7 GB (more reserved: the capacity is the tighter).
        let others: Vec<f64> = TONIGHT_KNOTS
            .iter()
            .map(|&(_, view, _)| MEM_TOTAL_MB - OUTSIDE_P5_MB - view)
            .collect();
        for floor in [10_240.0, MEASURED_FLOOR_MB] {
            let alone = floor_replay(&TONIGHT_KNOTS, &TONIGHT_BOARDS, floor, 3600, 15, None);
            let with = floor_replay(
                &TONIGHT_KNOTS,
                &TONIGHT_BOARDS,
                floor,
                3600,
                15,
                Some((capacity, &others)),
            );
            assert_eq!(with, alone);
            assert!(with.3 > FLOOR_KEEP_MB);
        }
        // The capacity holds a board the floor admits back only with the
        // outside memory below its median less the floor (18.1 - 8.8 =
        // 9.3 GB): at 9 GB the measured floor's night changes.
        let low: Vec<f64> = TONIGHT_KNOTS
            .iter()
            .map(|&(_, view, _)| MEM_TOTAL_MB - 9.0 * 1024.0 - view)
            .collect();
        assert_ne!(
            floor_replay(
                &TONIGHT_KNOTS,
                &TONIGHT_BOARDS,
                MEASURED_FLOOR_MB,
                3600,
                15,
                Some((capacity, &low)),
            ),
            floor_replay(
                &TONIGHT_KNOTS,
                &TONIGHT_BOARDS,
                MEASURED_FLOOR_MB,
                3600,
                15,
                None
            )
        );
    }

    #[test]
    fn a_harness_s_hand_over_is_the_least_the_process_holds() {
        // run.py's gate hands the board's reservation to its binary
        // (harness_common.Ledger.move): `granted_mb` says what it reserved.
        let mut moved = Reservation {
            pid: 7,
            started: 100.0,
            megabytes: 2458.0,
            label: "run.py: Quanta75 (expected peak)".into(),
            granted_mb: Some(2458.0),
            ..Default::default()
        };
        assert_eq!(
            granted_of(&moved, "layout-kicad-board Quanta75", 200.0),
            2458.0
        );
        // A harness before it: no field; the entry is not the process's
        // own (another label or another start), so its megabytes.
        moved.granted_mb = None;
        assert_eq!(
            granted_of(&moved, "layout-kicad-board Quanta75", 200.0),
            2458.0
        );
        assert_eq!(
            granted_of(&moved, "run.py: Quanta75 (expected peak)", 200.0),
            2458.0
        );
        // The process's own entry without a grant: none.
        let own = Reservation {
            label: "layout-kicad-board Quanta75".into(),
            started: 200.0,
            megabytes: 1331.0,
            ..moved.clone()
        };
        assert_eq!(granted_of(&own, "layout-kicad-board Quanta75", 200.0), 0.0);
        // Its own entry after the hand-over keeps the grant.
        let kept = Reservation {
            granted_mb: Some(2458.0),
            megabytes: 2458.0,
            ..own.clone()
        };
        assert_eq!(
            granted_of(&kept, "layout-kicad-board Quanta75", 200.0),
            2458.0
        );
        let text = serde_json::to_value(&kept).unwrap();
        assert_eq!(text["granted_mb"], 2458.0);
        assert_eq!(serde_json::from_value::<Reservation>(text).unwrap(), kept);
        assert!(
            serde_json::to_value(&own)
                .unwrap()
                .get("granted_mb")
                .is_none()
        );
    }

    #[test]
    fn a_run_past_its_reservation_waits_for_room_and_ends_only_short_of_memory() {
        // Within the bound it goes on, whatever the others still grow by,
        // as long as the kernel has its 2 GB.
        assert_eq!(
            overgrown_ends(10.0, Some(26_700.0), Some(7_600.0), 600.0),
            None
        );
        assert_eq!(
            overgrown_ends(599.0, Some(2_048.0), Some(-5_000.0), 600.0),
            None
        );
        assert_eq!(overgrown_ends(599.0, None, None, 600.0), None);
        // Below the 2 GB to keep it ends at once.
        let ends = overgrown_ends(1.0, Some(2_047.0), Some(30_000.0), 600.0).unwrap();
        assert!(
            ends.contains("fell to 2047 MB, below the 2048 MB to keep"),
            "{ends}"
        );
        // Past the bound it ends when the others' growth would take the
        // machine there (or nothing is known), else it goes on.
        assert_eq!(
            overgrown_ends(600.0, Some(26_700.0), Some(2_048.0), 600.0),
            None
        );
        let ends = overgrown_ends(600.0, Some(26_700.0), Some(2_047.0), 600.0).unwrap();
        assert!(ends.contains("no room came in 600 s"), "{ends}");
        assert!(overgrown_ends(601.0, None, None, 600.0).is_some());
        assert_eq!(GROWTH_WAIT_SECONDS, DEFAULT_DRAIN_AFTER_SECONDS);
    }

    #[test]
    fn a_run_past_its_reservation_waits_in_order_as_of_its_admission() {
        // The nightly of 2026-10-27 in small: a 45 200 MB ledger, 33 800
        // reserved by the runs; the layout (pid 30) admitted at 1000 s
        // holds 1415 MB and, past it, waits for 1769 MB (its entry as the
        // watchdog writes it: `waiting_since` its `held_since`); a board
        // of the harness (pid 40) has waited since 1100 s for 2458 MB, and
        // a new request (pid 50) of 2000 MB arrives at 1200 s.
        let capacity = Some(45_200.0);
        let holder = |pid: u32, megabytes: f64, since: f64| Reservation {
            pid,
            started: since,
            held_since: Some(since),
            megabytes,
            label: format!("run {pid}"),
            ..Default::default()
        };
        let grower = Reservation {
            waiting_mb: Some(1769.0),
            waiting_since: Some(1000.0),
            refreshed: Some(1690.0),
            ..holder(30, 1415.0, 1000.0)
        };
        let board = Reservation {
            pid: 40,
            started: 1100.0,
            megabytes: 0.0,
            label: "run.py: board (expected peak)".into(),
            waiting_mb: Some(2458.0),
            waiting_since: Some(1100.0),
            refreshed: Some(1200.0),
            floor_mb: Some(6_450.0),
            ..Default::default()
        };
        let older = holder(11, 30_000.0, 10.0);
        let newer = holder(12, 2_385.0, 1050.0);
        let all = [older.clone(), newer.clone(), grower.clone(), board.clone()];
        let others = refs(&all);
        // It comes first among the waiters: its request is as old as its
        // admission, older than the board's wait.
        assert_eq!(first_waiter(&others).map(|r| r.pid), Some(30));
        // 33 800 reserved: the new request of 2000 MB fits the capacity
        // (35 800), and the grower's 1769 beside it too (37 569): it goes.
        assert!(order_allows(&others, 50, 1200.0, 2000.0, capacity));
        // With 43 300 reserved it would take the grower's room (41 885 and
        // its 1769 leave 1546 MB): it waits.
        let full = [
            Reservation {
                megabytes: 39_500.0,
                ..older.clone()
            },
            newer.clone(),
            grower.clone(),
            board.clone(),
        ];
        assert!(!order_allows(&refs(&full), 50, 1200.0, 2000.0, capacity));
        // It has waited past the drain's `drain_after` (since 1000 s): the
        // ledger drains for it while it could start once the runs since
        // its admission end.
        assert_eq!(
            drain_state(&grower, &refs(&full), 1700.0, capacity, Some(20_000.0)),
            Drain::Drains
        );
        // The grower itself goes by the capacity alone (may_hold): 1769
        // beside 41 885 fit, beside 44 000 not until a run ends.
        assert!(may_hold(&full, 30, 1415.0, 1000.0, 1769.0, capacity, &[]));
        let fuller = [
            Reservation {
                megabytes: 41_615.0,
                ..older.clone()
            },
            newer,
            grower,
            board,
        ];
        assert!(!may_hold(
            &fuller,
            30,
            1415.0,
            1000.0,
            1769.0,
            capacity,
            &[]
        ));
    }
}

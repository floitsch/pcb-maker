// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The memory profile (`PCB_ROUTER_MEM_PROFILE=1`): where a process's
//! peak resident size goes.
//!
//! Three sources, all printed to stderr as `mem-profile:` lines between
//! the run's own log lines (so that a line's place in the log says what
//! the process was doing):
//!
//! - a sampler thread reads the resident size (/proc/self/statm) every
//!   10 ms and prints each new high (`rss high`, every 2 % or 32 MB) with
//!   the phase, the live heap and the routers alive with their parts;
//! - the counting allocator (`CountingAllocator`, the binary's global
//!   allocator: the system's, which counts the live heap's bytes while
//!   the profile is on; one relaxed load an allocation otherwise);
//! - the routers' parts (`Router::memory_parts`, registered at their
//!   build and after each route or iteration, dropped with the router),
//!   and the phases the caller marks (`phase`).
//!
//! At the end (`finish`) the peak: its time, phase, live heap and the
//! routers' parts then. Nothing of it changes what a run computes.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static COUNTING: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK_LIVE: AtomicIsize = AtomicIsize::new(0);
static NEXT_ROUTER: AtomicU64 = AtomicU64::new(1);

/// The system allocator, counting the live heap while the profile is on.
pub struct CountingAllocator;

#[inline(always)]
fn count(delta: isize) {
    if COUNTING.load(Relaxed) {
        let live = LIVE.fetch_add(delta, Relaxed) + delta;
        if delta > 0 {
            PEAK_LIVE.fetch_max(live, Relaxed);
        }
    }
}

// SAFETY: every call is the system allocator's; the counters are atomics.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            count(layout.size() as isize);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            count(layout.size() as isize);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        count(-(layout.size() as isize));
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(pointer, layout, size) };
        if !moved.is_null() {
            count(size as isize - layout.size() as isize);
        }
        moved
    }
}

/// Whether the profile is on (`PCB_ROUTER_MEM_PROFILE` set and not 0).
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("PCB_ROUTER_MEM_PROFILE").is_ok_and(|value| !value.is_empty() && value != "0")
    })
}

const MB: f64 = 1024.0 * 1024.0;

struct State {
    started: Instant,
    phase: String,
    /// Router id -> its parts: (name, bytes, shared key: a part with a key
    /// counts once for all routers that hold it).
    routers: BTreeMap<u64, Vec<(&'static str, usize, Option<usize>)>>,
    printed_mb: f64,
    peak: Option<Peak>,
}

#[derive(Clone)]
struct Peak {
    rss_mb: f64,
    seconds: f64,
    phase: String,
    live_mb: f64,
    routers: String,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(State {
            started: Instant::now(),
            phase: "start".into(),
            routers: BTreeMap::new(),
            printed_mb: 0.0,
            peak: None,
        })
    })
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    state().lock().unwrap_or_else(|poison| poison.into_inner())
}

/// The process's resident size in megabytes (Linux; 0 elsewhere).
pub fn resident_mb() -> f64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|text| text.split_whitespace().nth(1)?.parse::<f64>().ok())
        .map_or(0.0, |pages| pages * 4096.0 / MB)
}

/// The live heap in megabytes (while the profile is on).
pub fn live_mb() -> f64 {
    LIVE.load(Relaxed) as f64 / MB
}

/// The routers' parts summed (a shared part once), as text.
fn routers_text(state: &State) -> String {
    let mut sums: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for parts in state.routers.values() {
        for (name, bytes, shared) in parts {
            if shared.is_none_or(|key| seen.insert((*name, key))) {
                *sums.entry(name).or_default() += bytes;
            }
        }
    }
    let total: usize = sums.values().sum();
    let mut sorted: Vec<_> = sums.into_iter().collect();
    sorted.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
    let mut text = format!(
        "{} routers {:.0} MB",
        state.routers.len(),
        total as f64 / MB
    );
    for (name, bytes) in sorted {
        if bytes as f64 >= 0.5 * MB {
            let _ = write!(text, ", {name} {:.0}", bytes as f64 / MB);
        }
    }
    text
}

/// Starts the profile (the counting and the sampler) when it is on; once,
/// first thing in `main`.
pub fn start() {
    if !enabled() {
        return;
    }
    COUNTING.store(true, Relaxed);
    drop(lock());
    std::thread::Builder::new()
        .name("mem-profile".into())
        .spawn(|| {
            loop {
                sample();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        })
        .ok();
}

fn sample() {
    let rss = resident_mb();
    let mut state = lock();
    if state.peak.as_ref().is_none_or(|peak| rss > peak.rss_mb) {
        let peak = Peak {
            rss_mb: rss,
            seconds: state.started.elapsed().as_secs_f64(),
            phase: state.phase.clone(),
            live_mb: live_mb(),
            routers: routers_text(&state),
        };
        if rss >= state.printed_mb * 1.02 && rss >= state.printed_mb + 32.0 {
            state.printed_mb = rss;
            eprintln!(
                "mem-profile: rss high {:.0} MB at {:.1} s ({}): heap {:.0} MB (peak {:.0}); {}",
                peak.rss_mb,
                peak.seconds,
                peak.phase,
                peak.live_mb,
                PEAK_LIVE.load(Relaxed) as f64 / MB,
                peak.routers
            );
        }
        state.peak = Some(peak);
    }
}

/// Marks a phase of the run (the sampler's label) and prints the resident
/// size, the live heap and the routers then.
pub fn phase(name: &str) {
    if !enabled() {
        return;
    }
    let mut state = lock();
    state.phase = name.to_string();
    eprintln!(
        "mem-profile: phase {name} at {:.1} s: rss {:.0} MB, heap {:.0} MB; {}",
        state.started.elapsed().as_secs_f64(),
        resident_mb(),
        live_mb(),
        routers_text(&state)
    );
}

/// Prints a measure (`what`: bytes) beside the phases.
pub fn note(what: &str, bytes: usize) {
    if enabled() {
        eprintln!("mem-profile: {what} {:.1} MB", bytes as f64 / MB);
    }
}

/// Prints the peak; at the end of the run.
pub fn finish() {
    if !enabled() {
        return;
    }
    sample();
    let state = lock();
    if let Some(peak) = &state.peak {
        eprintln!(
            "mem-profile: peak rss {:.0} MB at {:.1} s of {:.1} s ({}): heap {:.0} MB then (peak heap {:.0}); {}",
            peak.rss_mb,
            peak.seconds,
            state.started.elapsed().as_secs_f64(),
            peak.phase,
            peak.live_mb,
            PEAK_LIVE.load(Relaxed) as f64 / MB,
            peak.routers
        );
    }
}

/// A router's entry in the profile: registered with its parts, gone with
/// the router. A copy registers the parts of its original until it
/// reports its own.
pub struct Token(u64);

impl Default for Token {
    fn default() -> Self {
        Self(NEXT_ROUTER.fetch_add(1, Relaxed))
    }
}

impl Clone for Token {
    fn clone(&self) -> Self {
        let copy = Self::default();
        if enabled() {
            let mut state = lock();
            if let Some(parts) = state.routers.get(&self.0).cloned() {
                state.routers.insert(copy.0, parts);
            }
        }
        copy
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        if enabled() {
            lock().routers.remove(&self.0);
        }
    }
}

impl Token {
    /// Registers the router's parts now.
    pub(crate) fn update(&self, parts: Vec<(&'static str, usize, Option<usize>)>) {
        lock().routers.insert(self.0, parts);
    }
}

fn shape_bytes(shape: &crate::geometry::Shape) -> usize {
    match shape {
        crate::geometry::Shape::Polygon { points } => points.capacity() * 16,
        crate::geometry::Shape::Union { parts } => {
            parts.capacity() * std::mem::size_of::<crate::geometry::Shape>()
                + parts.iter().map(shape_bytes).sum::<usize>()
        }
        _ => 0,
    }
}

/// A lowered board's heap, estimated from its lists' capacities (the
/// memory profile).
pub fn board_bytes(board: &crate::board::Board) -> usize {
    let points = |list: &Vec<crate::geometry::Point>| list.capacity() * 16;
    board.obstacles.capacity() * std::mem::size_of::<crate::board::Obstacle>()
        + board
            .obstacles
            .iter()
            .map(|obstacle| shape_bytes(&obstacle.shape) + obstacle.label.capacity())
            .sum::<usize>()
        + board.nets.capacity() * std::mem::size_of::<crate::board::Net>()
        + board
            .nets
            .iter()
            .map(|net| {
                net.name.capacity()
                    + net.terminals.capacity() * std::mem::size_of::<crate::board::Terminal>()
                    + net
                        .terminals
                        .iter()
                        .map(|terminal| {
                            terminal.label.capacity()
                                + terminal.contact.as_ref().map_or(0, shape_bytes)
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
        + board
            .planes
            .iter()
            .map(|plane| {
                std::mem::size_of::<crate::board::Plane>()
                    + points(&plane.polygon)
                    + plane.excluded.iter().map(points).sum::<usize>()
                    + plane.above.iter().map(points).sum::<usize>()
            })
            .sum::<usize>()
        + points(&board.outline)
        + board.pad_spokes.len() * 96
        + board.terminal_copper.capacity() * 32
}

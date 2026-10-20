// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Wall-clock cuts and the wall guard.

/// What the wall clock cut short in this process (`note_wall_cut`), one
/// bit per kind (`WALL_CUTS`).
pub(super) static WALL_CUT_BITS: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// The kinds of wall-clock cut, in bit order.
pub const WALL_CUTS: [&str; 6] = [
    "deadline",
    "negotiation guard",
    "budget guard",
    "ladder",
    "placement",
    "layout",
];

/// Records that the wall clock, not the work, stopped `what` (one of
/// `WALL_CUTS`): the result then depends on how fast the machine ran, and
/// the run says so (its log once per kind, its report's `cut`). Everything
/// else in a run is decided by work and is the same for the same input
/// and thread count.
pub fn note_wall_cut(what: &str, detail: std::fmt::Arguments) {
    let Some(bit) = WALL_CUTS.iter().position(|kind| *kind == what) else {
        return;
    };
    let before = WALL_CUT_BITS.fetch_or(1 << bit, std::sync::atomic::Ordering::Relaxed);
    if before & (1 << bit) == 0 {
        eprintln!(
            "wall guard: the wall clock cut the run short ({what}: {detail}); its result depends on the machine's load"
        );
    }
}

/// The kinds of wall-clock cut this process saw (`note_wall_cut`).
pub fn wall_cuts() -> Vec<String> {
    let bits = WALL_CUT_BITS.load(std::sync::atomic::Ordering::Relaxed);
    WALL_CUTS
        .iter()
        .enumerate()
        .filter(|(bit, _)| bits & (1 << bit) != 0)
        .map(|(_, kind)| kind.to_string())
        .collect()
}

/// Whether the wall clock guards the work budgets (always, unless
/// `PCB_ROUTER_NO_WALL_GUARD` is set: for measuring the router's speed
/// under valgrind, where the clock runs fifty times slower than the work,
/// so that a run's routes do not depend on how fast it ran; and for
/// checking that a run is deterministic on a machine too busy for its
/// budget: the layout then sets no wall-clock deadline either).
pub fn wall_guard() -> bool {
    static GUARDED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *GUARDED.get_or_init(|| std::env::var_os("PCB_ROUTER_NO_WALL_GUARD").is_none())
}

// Copyright (C) 2026 Toit contributors.

//! The pairs' plan's work model: the obstacles its checks scan and test,
//! counted per thread, in nanoseconds of the reference machine.

use pcb_router as core;

thread_local! {
    /// The obstacles the pairs' code looked at on this thread: those of
    /// board-wide scans (a bounding box or a label each) and those of a
    /// launch's near lists (a distance each), `plan_pairs`' work model
    /// (`PLAN_SCAN_NANOS`, `PLAN_TEST_NANOS`).
    pub(super) static PLAN_TESTS: std::cell::Cell<[u64; 2]> = const { std::cell::Cell::new([0, 0]) };
}

/// Counts `count` obstacles of a board-wide scan (`PLAN_TESTS`).
pub(super) fn scanned(count: usize) {
    PLAN_TESTS.with(|tests| {
        let [scans, near] = tests.get();
        tests.set([scans + count as u64, near]);
    });
}

/// Counts `count` obstacles tested by distance (`PLAN_TESTS`).
pub(super) fn tested(count: usize) {
    PLAN_TESTS.with(|tests| {
        let [scans, near] = tests.get();
        tests.set([scans, near + count as u64]);
    });
}

/// The obstacles scanned and tested on this thread so far (`PLAN_TESTS`).
pub(super) fn tests_so_far() -> [u64; 2] {
    PLAN_TESTS.with(std::cell::Cell::get)
}

/// An obstacle of a board-wide scan and one tested by distance (or an
/// outline edge), in nanoseconds of the reference machine (the work model
/// of `plan_pairs`, as `LOWERING_EXPRESSION_NANOS` is the lowering's).
/// Measured 2026-11-04 (one plan, the time over the rate's scale): right_io
/// 0.15 M scanned and 6.8 M tested in 0.24 s of the reference, 35 ns a
/// test; left_io 0.52 M and 21.5 M in 0.67 s, 31 ns; jetson 7.9 M and 42.3
/// M in 0.90 s, about 20 ns (its tests leave sooner: more obstacles on
/// other layers). The model is off by -20, -10 and +35 % on them
/// (docs/router.md, "The layout's own routes").
pub(crate) const PLAN_SCAN_NANOS: f64 = 4.0;
pub(crate) const PLAN_TEST_NANOS: f64 = 28.0;

/// The modelled work of the obstacles scanned and tested since `before`
/// (`tests_so_far`), in seconds at this process's rate.
pub(super) fn plan_work_since(before: [u64; 2]) -> f64 {
    let [scans, near] = tests_so_far();
    ((scans - before[0]) as f64 * PLAN_SCAN_NANOS + (near - before[1]) as f64 * PLAN_TEST_NANOS)
        / 1.0e9
        * core::router::reference_time_scale()
}

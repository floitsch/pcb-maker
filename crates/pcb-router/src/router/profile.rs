// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Where a run's time and work go.

use super::*;

/// Wall seconds per phase of a run and how the negotiation's batches went,
/// for the verbose `profile:` line (where a board's time goes, and how
/// much of it uses one thread).
#[derive(Clone, Default)]
pub(super) struct Profile {
    pub(super) prepare: f64,
    pub(super) negotiation: f64,
    pub(super) conflicts: f64,
    pub(super) refresh_pour: f64,
    pub(super) replan: f64,
    pub(super) resolve: f64,
    pub(super) cleanup: f64,
    pub(super) via_reduction: f64,
    pub(super) thermal_spokes: f64,
    pub(super) stitching: f64,
    pub(super) emit: f64,
    /// Batches of one net (routed on this thread) and their seconds.
    pub(super) single_batches: u64,
    pub(super) single_seconds: f64,
    /// Batches of several nets (routed on the thread pool): how many, the
    /// nets in them, their wall seconds and the sum of the nets' own
    /// seconds (the work a single thread would have taken).
    pub(super) parallel_batches: u64,
    pub(super) parallel_nets: u64,
    pub(super) parallel_seconds: f64,
    pub(super) parallel_net_seconds: f64,
    /// The pour analysis (`analyze_pours`, in nanoseconds: it runs on
    /// `&self`), wherever it is called from.
    pub(super) analysis_nanos: Arc<std::sync::atomic::AtomicU64>,
    pub(super) analysis_calls: Arc<std::sync::atomic::AtomicU64>,
    /// Per phase (`PHASES`): the search work, the pour analyses' work
    /// and wall seconds, the phase's wall seconds.
    pub(super) phase_search: [f64; PHASES.len()],
    pub(super) phase_pour_work: [f64; PHASES.len()],
    pub(super) phase_pour_wall: [f64; PHASES.len()],
    pub(super) phase_wall: [f64; PHASES.len()],
    /// Per phase: the search expansions (`phase_search` without the
    /// division, for the `profile rate:` line).
    pub(super) phase_expansions: [u64; PHASES.len()],
    /// With `PCB_ROUTER_SEARCH_RATE`: nanoseconds inside `search` and the
    /// expansions done there (it runs on `&self`, also side by side).
    pub(super) search_nanos: Arc<std::sync::atomic::AtomicU64>,
    pub(super) search_expansions: Arc<std::sync::atomic::AtomicU64>,
}

/// Whether `search` times itself (`PCB_ROUTER_SEARCH_RATE`, for
/// calibrating `REFERENCE_EXPANSIONS_PER_SECOND`: the `profile rate:` line).
pub(super) fn search_rate_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PCB_ROUTER_SEARCH_RATE").is_some())
}

pub(super) static BUILD_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(super) static BUILD_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The builds' modelled work in nanoseconds (`BuildCounts::work_seconds`).
pub(super) static BUILD_WORK_NANOS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Routers built (`Router::new`: lattice, obstacles, pours) in this
/// process and their seconds, summed over threads.
pub fn build_totals() -> (u64, f64) {
    use std::sync::atomic::Ordering::Relaxed;
    (
        BUILD_CALLS.load(Relaxed),
        BUILD_NANOS.load(Relaxed) as f64 / 1.0e9,
    )
}

/// Over the process (every router, every thread; routers side by side
/// add up): per phase (`PHASES`), its wall nanoseconds, its search work and
/// its pour analyses' work (nanoseconds of an idle machine), and the
/// preparation's wall nanoseconds (escapes, skeletons, plane stubs, the
/// global plan): `process_accounting`.
static PROCESS_PHASES: [[std::sync::atomic::AtomicU64; 3]; PHASES.len()] =
    [const { [const { std::sync::atomic::AtomicU64::new(0) }; 3] }; PHASES.len()];
static PROCESS_PREPARE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Notes `seconds` of preparation (`process_accounting`).
pub(super) fn note_prepare(seconds: f64) {
    PROCESS_PREPARE.fetch_add(
        (seconds * 1.0e9) as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// Where this process's routers spent their wall clock and their work,
/// summed over the routers and threads (the ladder's `wall accounting:`
/// line, board-router.json's `wall_accounting`).
#[derive(Clone, Debug, Default)]
pub struct ProcessAccounting {
    /// Routers built (lattices, obstacles, pours) and their seconds.
    pub builds: u64,
    pub build_seconds: f64,
    /// Their work as modelled (`RoutingResult::build_work`).
    pub build_work_seconds: f64,
    /// The preparation before the negotiation (escapes, skeletons, plane
    /// stubs, the global plan), in seconds.
    pub prepare_seconds: f64,
    /// Per phase (`PHASES`): its name, wall seconds, search work and pour
    /// work (seconds of an idle machine).
    pub phases: Vec<(&'static str, f64, f64, f64)>,
}

/// This process's `ProcessAccounting` so far.
pub fn process_accounting() -> ProcessAccounting {
    use std::sync::atomic::Ordering::Relaxed;
    let (builds, build_seconds) = build_totals();
    ProcessAccounting {
        builds,
        build_seconds,
        build_work_seconds: BUILD_WORK_NANOS.load(Relaxed) as f64 / 1.0e9,
        prepare_seconds: PROCESS_PREPARE.load(Relaxed) as f64 / 1.0e9,
        phases: PHASES
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let [wall, search, pour] = &PROCESS_PHASES[index];
                (
                    *name,
                    wall.load(Relaxed) as f64 / 1.0e9,
                    search.load(Relaxed) as f64 / 1.0e9,
                    pour.load(Relaxed) as f64 / 1.0e9,
                )
            })
            .collect(),
    }
}

/// Adds a phase's share to the process's (`process_accounting`).
pub(super) fn note_phase(phase: usize, wall: f64, search: f64, pour: f64) {
    use std::sync::atomic::Ordering::Relaxed;
    let [total_wall, total_search, total_pour] = &PROCESS_PHASES[phase];
    total_wall.fetch_add((wall.max(0.0) * 1.0e9) as u64, Relaxed);
    total_search.fetch_add((search.max(0.0) * 1.0e9) as u64, Relaxed);
    total_pour.fetch_add((pour.max(0.0) * 1.0e9) as u64, Relaxed);
}

/// The phases the `profile work:` line splits the work into.
pub(super) const PHASES: [&str; 7] = [
    "negotiation",
    "resolve",
    "clean-up",
    "via reduction",
    "thermal spokes",
    "stitching",
    "manual spokes",
];

/// A phase's start for the profile: search work, pour work, pour wall
/// nanoseconds, wall clock, search expansions.
pub(super) type PhaseMark = (f64, f64, u64, std::time::Instant, u64);

impl Profile {
    /// Per phase: wall seconds, search work, pour analyses' wall seconds
    /// and work.
    pub(super) fn work_line(&self) -> String {
        let phases: Vec<String> = PHASES
            .iter()
            .enumerate()
            .map(|(index, name)| {
                format!(
                    "{name} {:.1}s wall, {:.1}s search work, pour {:.1}s wall, {:.1}s work",
                    self.phase_wall[index],
                    self.phase_search[index],
                    self.phase_pour_wall[index],
                    self.phase_pour_work[index],
                )
            })
            .collect();
        format!("profile work: {}", phases.join("; "))
    }

    /// The search's rate (`REFERENCE_EXPANSIONS_PER_SECOND` measured): the phases'
    /// expansions over their wall seconds without the pour analyses' (which
    /// count by their own model), and with `PCB_ROUTER_SEARCH_RATE` the
    /// expansions over the seconds inside `search` alone.
    pub(super) fn rate_line(&self) -> String {
        use std::sync::atomic::Ordering::Relaxed;
        let expansions: u64 = self.phase_expansions.iter().sum();
        let wall: f64 =
            self.phase_wall.iter().sum::<f64>() - self.phase_pour_wall.iter().sum::<f64>();
        let pour_wall: f64 = self.phase_pour_wall.iter().sum();
        let pour_work: f64 = self.phase_pour_work.iter().sum();
        let inside = self.search_nanos.load(Relaxed) as f64 / 1.0e9;
        let inside_expansions = self.search_expansions.load(Relaxed);
        let rate = |count: u64, seconds: f64| {
            if seconds > 0.0 {
                count as f64 / seconds
            } else {
                0.0
            }
        };
        format!(
            "profile rate: phases {expansions} expansions in {wall:.3}s without pour analyses ({:.0}/s, rate {:.0}); pour {pour_work:.3}s work in {pour_wall:.3}s wall; inside the search {inside_expansions} expansions in {inside:.3}s ({:.0}/s)",
            rate(expansions, wall),
            expansions_per_second(),
            rate(inside_expansions, inside),
        )
    }

    pub(super) fn line(&self, search: f64, stamp: f64) -> String {
        let analysis = self
            .analysis_nanos
            .load(std::sync::atomic::Ordering::Relaxed) as f64
            / 1.0e9;
        format!(
            "profile: prepare {:.1}s, negotiation {:.1}s (search {:.1}s, stamp {:.1}s, conflicts and history {:.1}s, pour refresh {:.1}s, replan {:.1}s), resolve {:.1}s, clean-up {:.1}s, via reduction {:.1}s, thermal spokes {:.1}s, stitching {:.1}s, emit {:.1}s; pour analysis {:.1}s in {} calls; batches: {} single ({:.1}s), {} parallel with {} nets ({:.1}s wall, {:.1}s of net time)",
            self.prepare,
            self.negotiation,
            search,
            stamp,
            self.conflicts,
            self.refresh_pour,
            self.replan,
            self.resolve,
            self.cleanup,
            self.via_reduction,
            self.thermal_spokes,
            self.stitching,
            self.emit,
            analysis,
            self.analysis_calls
                .load(std::sync::atomic::Ordering::Relaxed),
            self.single_batches,
            self.single_seconds,
            self.parallel_batches,
            self.parallel_nets,
            self.parallel_seconds,
            self.parallel_net_seconds,
        )
    }
}

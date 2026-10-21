// Copyright (C) 2026 Toit contributors.

//! The attempt ladder of `route_kicad_board` as explicit steps (`Ladder`):
//! the rung probes and the probed rung's continuation (`probes`), the
//! rungs of each pitch with their seed retry (`rungs`), the best rung's
//! seed rounds (`seed_rounds`), the narrow-signal and minimum-clearance
//! steps (`steps`). The bookkeeping (the ladder's work, the work kept for
//! the narrow-signal step, the work bound of route mode, the best board,
//! KiCad's check gate) is the struct's state; what routes, checks and
//! measures memory is a `LadderRunner`, so that the transitions are tested
//! with a scripted runner (ladder_tests.rs) while route.rs's runner routes.
//!
//! Every attempt runs isolated (`isolate`): a panic in a probe, a rung, a
//! seed retry or round, the probed rung's continuation or a step is
//! recorded (`failed`, the report's `attempts_failed`) and the ladder goes
//! on with the best board it holds. Until 2026-10-21 a panic in one
//! router thread ended the run (exit 4) and lost every board before it.

use super::*;
use pcb_router as core;

/// A rung of the ladder: (connect pours, plane skeleton, exclusive planes,
/// plane stubs).
pub(crate) type Rung = (bool, bool, bool, bool);

/// An attempt's open count (the router's and KiCad's unconnected items)
/// and KiCad's starved thermals.
pub(crate) type Opens = (usize, usize);

/// Where the probed rung's continuation stops (`LadderRunner::resume`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResumeLimits {
    /// The run's work budget and what is left of it (router `run_work`).
    pub run_work: (f64, f64),
    /// The router's work deadline, when the ladder sets one.
    pub deadline_work: Option<f64>,
    /// The negotiation's work from where the probe stopped.
    pub negotiation: f64,
}

/// What a router did, as route mode's ladder charges it: its search
/// expansions, and the work of its pour analyses and fill probes and of
/// its build (seconds of an idle machine: pcb-router's `Router::pour_work`
/// and `Router::build_work`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RouterWork {
    pub expansions: u64,
    pub pour: f64,
    pub build: f64,
}

/// What the ladder routes and checks with. route.rs's `KiCadRunner` routes
/// the board; the tests' runner returns scripted results.
pub(crate) trait LadderRunner: Sync {
    /// A rung's board, lowered once for its probe.
    type Board: Clone + Send + Sync;
    /// A probing router.
    type Probe: Send;
    /// A continued router's routing.
    type Routed: Send;

    /// The board of one rung as its probe routes it.
    fn lower(&self, attempt: &KiCadBoardRouterConfig, connect: bool)
    -> Result<Self::Board, String>;
    /// A lowered board's copper layers and connections.
    fn size(&self, board: &Self::Board) -> (usize, usize);
    /// A router for a probe, built (`quiet`: without its log lines).
    fn build(
        &self,
        board: &Self::Board,
        attempt: &KiCadBoardRouterConfig,
        quiet: bool,
    ) -> Self::Probe;
    /// A built router's lattice maps in megabytes (its memory report).
    fn maps_mb(&self, probe: &Self::Probe) -> Option<f64>;
    /// Negotiates `work` seconds of work; the nets left unfinished.
    fn probe(&self, probe: &mut Self::Probe, work: f64, mode: usize) -> usize;
    /// What the router did so far, as the ladder charges it.
    fn work(&self, probe: &Self::Probe) -> RouterWork;
    /// The work of lowering `board` (`lower::lowering_work`).
    fn lowering_work(&self, board: &Self::Board) -> f64;
    /// The router logs as `attempt` says (a probe beside others was quiet).
    fn speak(&self, probe: &mut Self::Probe, attempt: &KiCadBoardRouterConfig);
    /// Goes on from where the probe stopped, within `limits`; the routing
    /// and the router's work in all. The router is gone after it.
    fn resume(&self, probe: Self::Probe, limits: ResumeLimits) -> (Self::Routed, RouterWork);
    /// Writes a continued router's routing as the project in `directory`.
    fn write(
        &self,
        board: &Self::Board,
        routed: Self::Routed,
        attempt: &KiCadBoardRouterConfig,
        directory: &Path,
        routing_seconds: f64,
    ) -> Result<KiCadBoardRouterResult, String>;
    /// Routes one attempt into `directory` (which does not exist).
    fn route(
        &self,
        directory: &Path,
        attempt: &KiCadBoardRouterConfig,
        connect: bool,
    ) -> Result<KiCadBoardRouterResult, String>;
    /// KiCad's check of an attempt routed without it (`check_attempt`).
    fn check(&self, result: &mut KiCadBoardRouterResult, directory: &Path) -> Result<(), String>;
    /// KiCad's starved thermals in `directory`'s report.
    fn starved(&self, directory: &Path) -> usize;
    /// The memory limits now (how many routers run side by side).
    fn memory_limits(&self) -> MemoryLimits;
}

/// The message of a caught panic.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| text.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

/// Runs one attempt of the ladder: its value, or the panic's message.
pub(crate) fn isolate<T>(attempt: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(attempt))
        .map_err(|payload| panic_message(payload.as_ref()))
}

/// The test hook `PCB_MAKER_TEST_PANIC_ATTEMPT=<kind>[:<n>|:all]`: the
/// n-th (from 1, default 1) attempt of `kind` (probe, continuation, rung,
/// seeds, step) the ladder starts panics, in the order they start, or
/// every one of them (`all`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TestPanic {
    pub kind: String,
    /// From 1; 0: every attempt of the kind.
    pub nth: usize,
}

/// The test hook's environment variable (`TestPanic`).
pub(crate) const TEST_PANIC_ATTEMPT: &str = "PCB_MAKER_TEST_PANIC_ATTEMPT";

impl TestPanic {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let (kind, nth) = match value.split_once(':') {
            Some((kind, "all")) => (kind, 0),
            Some((kind, nth)) => (kind, nth.trim().parse().ok().filter(|nth| *nth > 0)?),
            None => (value, 1),
        };
        let kind = kind.trim();
        (!kind.is_empty()).then(|| Self {
            kind: kind.into(),
            nth,
        })
    }

    pub(crate) fn from_env() -> Option<Self> {
        Self::parse(&std::env::var(TEST_PANIC_ATTEMPT).ok()?)
    }
}

/// Panics as the test hook asked (`TestPanic`).
fn test_panic(kind: &str) {
    panic!("{TEST_PANIC_ATTEMPT}: the {kind} attempt panics");
}

/// The rungs of the ladder for a board (`pours`: whether it has pours,
/// `two_layers`): pours connected, with the plane skeleton, the plane
/// stubs, pour nets as tracks, as the configuration allows.
pub(crate) fn ladder_rungs(
    config: &KiCadBoardRouterConfig,
    has_pours: bool,
    two_layers: bool,
) -> Vec<Rung> {
    // The attempt ladder: pours connected, then connected with a fixed plane
    // skeleton, then pour nets as tracks; each first on the regular lattice
    // and then on finer ones. It stops at the first clean board and
    // otherwise keeps the one with the fewest opens.
    // A skeleton helps a two-layer board whose pours get shredded by the
    // signals; with inner planes the pours stay whole and the fixed tree
    // only blocks vias.
    let skeleton = config.plane_skeleton.unwrap_or(false);
    // (connect pours, plane skeleton, exclusive planes). On four or more
    // layers, inner planes are first kept free of signals (they stay whole:
    // ColdFire 13 open against 43), then opened to them (video needs the
    // room: 9 open against 110), then pour nets go as tracks.
    let exclusive = config.exclusive_planes;
    // Rungs: (connect pours, plane skeleton, exclusive planes, plane
    // stubs). The stub rung (a via next to every surface pad of a pour
    // net before the signals route; boards with inner planes) runs when
    // the plain pour connection left pour-net pads open, like the
    // skeleton rung: it saves those pads on some boards (Framework
    // mainboard half, GND 7 -> 0) and takes room on others (ColdFire).
    let stubs = config.fixed_plane_stubs;
    match (has_pours, config.pours) {
        (false, _) | (true, KiCadPourMode::Tracks) => vec![(false, false, false, false)],
        (true, KiCadPourMode::Connect) => vec![(
            true,
            skeleton,
            exclusive.unwrap_or(false),
            stubs.unwrap_or(false),
        )],
        (true, KiCadPourMode::Auto) if skeleton => {
            vec![(true, true, false, false), (false, false, false, false)]
        }
        (true, KiCadPourMode::Auto) if two_layers => {
            vec![
                (true, false, false, false),
                (true, true, false, false),
                (false, false, false, false),
            ]
        }
        (true, KiCadPourMode::Auto) => {
            let mut modes = match exclusive {
                Some(true) => vec![(true, false, true, false)],
                Some(false) => vec![(true, false, false, false)],
                None => vec![(true, false, true, false), (true, false, false, false)],
            };
            if stubs != Some(false) {
                modes.push((true, false, false, true));
            }
            modes.push((false, false, false, false));
            modes
        }
    }
}

/// The ladder's lattices: the regular one (None), then each finer pitch
/// of `refine_pitches_mm` (0.075 and 0.05 mm) below the regular one's.
pub(crate) fn ladder_pitches(config: &KiCadBoardRouterConfig) -> Vec<Option<Vec<f64>>> {
    let refine = config
        .refine_pitches_mm
        .clone()
        .unwrap_or_else(|| vec![0.075, 0.05]);
    let mut pitches: Vec<Option<Vec<f64>>> = vec![None];
    let coarsest = core_config(config)
        .pitches
        .iter()
        .copied()
        .fold(0.0, f64::max);
    for pitch in refine {
        if pitch < coarsest {
            pitches.push(Some(vec![pitch]));
        }
    }
    pitches
}

/// How an attempt's line names an open count KiCad did not check.
fn by_router(judged: &Option<Opens>) -> &'static str {
    if judged.is_none() {
        " (router's count, not checked)"
    } else {
        ""
    }
}

/// An attempt's routing work, from its search expansions.
fn work_of(expansions: u64) -> f64 {
    expansions as f64 / core::router::expansions_per_second()
}

/// An attempt's work beyond its search as the ladder counts it
/// (`Ladder::extra_charge`): all of it (`charges_all`), its manual spokes
/// (`charges_spokes`), or none.
fn beyond_search(result: &KiCadBoardRouterResult, charges_all: bool, charges_spokes: bool) -> f64 {
    if charges_all {
        result.pour_work_seconds + result.build_work_seconds + result.lowering_work_seconds
    } else if charges_spokes {
        result.manual_spokes_work_seconds
    } else {
        0.0
    }
}

/// Whether the ladder goes on after a rung.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Next,
    Stop,
}

/// One pitch's plan, decided before its rungs run.
struct PitchPlan {
    /// Every rung's negotiation (None: the configuration's own).
    negotiation: Option<f64>,
    /// Route mode: the rungs' work deadline (`rung_share`).
    route_deadline: Option<f64>,
    /// Whether the rungs after the first have their share.
    share_fixed: bool,
    /// What is left of the ladder's budget as the pitch starts.
    left: f64,
    /// What is left of the work deadline as the pitch starts.
    work_left: f64,
}

/// What the narrow-signal and minimum-clearance steps have (`steps`).
struct StepWork {
    /// What a layout's ladder kept for them (`reserve`).
    kept: f64,
    /// Route mode: what was kept for them (`route_reserve`).
    route_kept: f64,
    /// `kept` less the step's own check.
    kept_routing: f64,
}

/// The attempt ladder's state (see the module).
pub(crate) struct Ladder<'a> {
    config: &'a KiCadBoardRouterConfig,
    output_directory: &'a Path,
    /// The seed rounds' and the steps' directory.
    scratch: PathBuf,
    pub(crate) modes: Vec<Rung>,
    pub(crate) pitches: Vec<Option<Vec<f64>>>,
    has_pours: bool,
    pour_nets: Vec<String>,
    copper_layers: usize,
    /// `refine_budget_seconds`: what a finer pitch's attempt may take.
    budget: f64,
    pub(crate) ladder_budget: f64,
    pub(crate) started: std::time::Instant,
    /// The wall clock's margin beyond the work before the guard counts.
    wall_margin: f64,
    pub(crate) memory_at_start: MemoryLimits,
    /// The machine's memory: no finer pitch whose router would take more
    /// than 0.4 of it.
    memory_total_mb: Option<f64>,
    /// Whether KiCad checks the attempts (`CheckGate`).
    checking: bool,
    /// What one check counts as work (none in route mode).
    check_charge: f64,
    pub(crate) checks_charged: crate::check_work::CheckTally,
    gate: CheckGate,
    /// The ladder's work so far, in seconds of an idle machine.
    pub(crate) work: f64,
    /// Route mode: the most work the ladder does (`ladder_work_bound`).
    pub(crate) work_bound: Option<f64>,
    /// Route mode: the work kept for the narrow-signal step.
    route_reserve: f64,
    route_reserve_wanted: bool,
    route_reserve_set: bool,
    /// A layout's ladder: the work kept for the narrow-signal step.
    reserve: f64,
    reserve_wanted: bool,
    reserve_share: f64,
    reserve_cap: f64,
    reserve_set: bool,
    /// What the ladder's first attempt negotiated (its budget, in work).
    first_negotiation: f64,
    /// Route mode: whether an attempt's manual spokes count in the
    /// ladder's work beside its search (`result_work`; not under
    /// `PCB_LADDER_UNCHARGED_SPOKES`, the count before 2026-10-22).
    charges_spokes: bool,
    /// Route mode: whether all of an attempt's work counts (`result_work`:
    /// its search, every pour analysis and fill probe, its lattice's build
    /// and its board's lowering; not under `PCB_LADDER_UNCHARGED_POUR`,
    /// the count before, search and manual spokes).
    charges_all: bool,
    /// Route mode, all work charged: whether a share of the bound becomes
    /// a router's limits in search work by the routers' share of search so
    /// far (`search_fraction`; not under `PCB_LADDER_SEARCH_SHARES`).
    converts_shares: bool,
    /// The ladder's work by kind (`work` is their sum).
    pub(crate) charged: KiCadWorkCharges,
    /// Route mode: the work of the ladder's first attempt up to the end of
    /// its first negotiation iteration (0: none yet), which a fresh router
    /// does whatever its work deadline: no step starts with less left
    /// (not under `PCB_LADDER_NO_FIRST_ITERATION_GATE`).
    first_iteration: f64,
    /// What the last attempt took, in work, and its pitch (0: none yet).
    last_attempt_work: f64,
    last_attempt_pitch: f64,
    /// Routing seconds and pitch of the slowest attempt so far, to project
    /// the cost of a finer lattice.
    slowest: Option<(f64, f64)>,
    /// The first attempt's lattice maps and pitch: routers side by side
    /// (rungs, seeds) are sized from them (`MemoryLimits`), and a finer
    /// pitch whose router does not fit is not tried.
    router_maps: Option<(f64, f64)>,
    extra_rung_tried: bool,
    /// The rung the probe continued: the regular ladder skips it.
    probed_mode: Option<usize>,
    probe_complete: bool,
    /// Whether the best attempt's rung had its seed retry.
    best_seeded: bool,
    /// The best board so far, with its open count.
    pub(crate) best: Option<(Opens, KiCadBoardRouterResult)>,
    /// The configuration of the best attempt so far (and whether its pours
    /// connected), for the narrow-signal retry at the end.
    best_attempt: Option<(KiCadBoardRouterConfig, bool)>,
    /// Steps that did not run, with why: the report's `skipped` when the
    /// board stays incomplete.
    pub(crate) skipped: Vec<String>,
    /// Attempts that panicked, each with its message (`attempts_failed`).
    pub(crate) failed: Vec<String>,
    /// The test hook (`TestPanic`) and the attempts started by kind.
    pub(crate) panic_at: Option<TestPanic>,
    started_by_kind: std::collections::BTreeMap<&'static str, usize>,
}

impl<'a> Ladder<'a> {
    /// The ladder for a board with pours on `pour_nets` and
    /// `copper_layers` layers, with the memory limits as it starts.
    pub(crate) fn new(
        config: &'a KiCadBoardRouterConfig,
        output_directory: &'a Path,
        pour_nets: Vec<String>,
        copper_layers: usize,
        memory_at_start: MemoryLimits,
        memory_total_mb: Option<f64>,
    ) -> Self {
        let has_pours = !pour_nets.is_empty();
        let checking = !config.skip_native_verification;
        // What one check counts as work (none in route mode), and the checks
        // counted so far: every check run, whether the DRC cache answered it
        // or not, so that a warm run ends where a cold one does.
        let check_charge = if checking {
            config.kicad_check_work.unwrap_or(0.0)
        } else {
            0.0
        };
        let ladder_budget = config.ladder_budget_seconds.unwrap_or(1200.0);
        // Route mode (no work deadline): the ladder's whole work (its probes,
        // rungs, seeds, the probed rung's continuation and the steps after,
        // each router's clean-up and polish included) stays within its budget
        // and a tolerance (`ladder_work_bound`): every router stops at its
        // share of what is left under it (router `deadline_work`), and none
        // starts with too little left (`bound_allows_attempt`). Until
        // 2026-10-21 a 30 s budget meant 60 s of negotiation a rung and 75 s
        // a probe: the intake's clean boards spent a median 53 s and the
        // probed ones 120 s, and 279 runs were cut by the wall guard. A
        // layout's ladder (`deadline_work`) is bounded by the layout's end as
        // before.
        let work_bound = config
            .deadline_work
            .is_none()
            .then(|| ladder_work_bound(ladder_budget));
        let ladder = Self {
            config,
            output_directory,
            scratch: output_directory.with_extension("attempt"),
            modes: ladder_rungs(config, has_pours, copper_layers == 2),
            pitches: ladder_pitches(config),
            has_pours,
            pour_nets,
            copper_layers,
            budget: config.refine_budget_seconds.unwrap_or(240.0),
            ladder_budget,
            started: std::time::Instant::now(),
            // Without a deadline (route mode) the wall clock guards the work
            // at `GUARD` times it; with one, the deadline is the only wall
            // guard. The guard takes over only when the wall clock is ahead
            // by more than a twentieth of the budget: the work counts only
            // search expansions, so the wall clock is always ahead of it at
            // the start of the ladder and after KiCad's checks, and every
            // route-mode run reported a "ladder" cut (0 s of wall clock for
            // 0 s of work) that changed nothing.
            wall_margin: 0.05 * ladder_budget,
            memory_at_start,
            memory_total_mb,
            checking,
            check_charge,
            checks_charged: crate::check_work::CheckTally::new(check_charge),
            gate: CheckGate {
                top: config.kicad_check_top,
                counts: config
                    .kicad_kept
                    .map(|(measure, _)| vec![measure])
                    .unwrap_or_default(),
                kept: config.kicad_kept.map(|(_, quality)| quality),
                ..CheckGate::default()
            },
            work: 0.0,
            work_bound,
            // Route mode: where the narrow-signal (or minimum-clearance)
            // step applies, once the ladder's first attempt leaves
            // connections open, up to that attempt's work is kept for the
            // step (`route_step_reserve`); the rungs and seeds after it see
            // what is left without it. A complete board releases it. On a
            // 30 s budget facade60's first attempt took 20 s, its two other
            // rungs 4 s each and nothing, and its step (23 to 5 open on
            // x1231's overrun) did not run.
            route_reserve: 0.0,
            route_reserve_wanted: work_bound.is_some() && narrow_step_applies(config),
            route_reserve_set: false,
            // The final narrow-signal (or minimum-clearance) step comes after
            // every rung and seed round; with a work deadline (the layout's
            // ladder, whose deadline is barely beyond its budget) the rungs
            // used to take all of it (the A13 module: 126 open to 17 by the
            // step in one run, never run in the next). Once the first
            // attempt tells what an attempt costs, a share of that is kept
            // back for the step: the rungs and seed rounds see the work left
            // without it. Zero when the step does not apply or there is no
            // work deadline.
            reserve: 0.0,
            reserve_wanted: config.deadline_work.is_some() && narrow_step_applies(config),
            reserve_share: std::env::var("PCB_NARROW_RESERVE")
                .ok()
                .and_then(|value| value.parse::<f64>().ok())
                .or(config.narrow_reserve_share)
                .unwrap_or(0.75),
            reserve_cap: config
                .deadline_work
                .map_or(0.0, |left| (0.4 * left).min(600.0)),
            reserve_set: false,
            first_negotiation: 0.0,
            charges_spokes: work_bound.is_some()
                && std::env::var_os("PCB_LADDER_UNCHARGED_SPOKES").is_none(),
            charges_all: work_bound.is_some()
                && std::env::var_os("PCB_LADDER_UNCHARGED_SPOKES").is_none()
                && std::env::var_os("PCB_LADDER_UNCHARGED_POUR").is_none(),
            converts_shares: work_bound.is_some()
                && std::env::var_os("PCB_LADDER_UNCHARGED_SPOKES").is_none()
                && std::env::var_os("PCB_LADDER_UNCHARGED_POUR").is_none()
                && std::env::var_os("PCB_LADDER_SEARCH_SHARES").is_none(),
            charged: KiCadWorkCharges::default(),
            first_iteration: 0.0,
            // No attempt starts with less than the last one's work left
            // before the run's nominal end (an attempt checks the deadline
            // only in its negotiation; MokyaLora's last rung started at
            // 1560 s and ran to 1890 s, past the harness's limit).
            last_attempt_work: 120.0,
            last_attempt_pitch: 0.0,
            slowest: None,
            router_maps: None,
            extra_rung_tried: false,
            probed_mode: None,
            probe_complete: false,
            best_seeded: false,
            best: None,
            best_attempt: None,
            skipped: Vec::new(),
            failed: Vec::new(),
            panic_at: None,
            started_by_kind: Default::default(),
        };
        if let Some(bound) = work_bound {
            eprintln!(
                "ladder: a budget of {ladder_budget:.0} s of work, at most {bound:.0} s with its tolerance ({:.0} %); wall guard {}",
                LADDER_WORK_TOLERANCE * 100.0,
                match config.deadline_seconds {
                    Some(seconds) if config.deadline.is_some() => {
                        let stretch = crate::work_rate::work_rate().wall_stretch(false);
                        if stretch > 1.0 {
                            format!(
                                "at {:.0} s of wall clock (deadline_seconds {seconds:.0} stretched {stretch:.2} times: the pinned rate over the machine's)",
                                seconds * stretch
                            )
                        } else {
                            format!("at {seconds:.0} s of wall clock (deadline_seconds)")
                        }
                    }
                    _ if config.deadline.is_some() => "at the caller's deadline".to_string(),
                    _ if core::router::wall_guard() => format!(
                        "at {} times the work, beyond a margin of {:.0} s",
                        core::router::guard(),
                        ladder.wall_margin
                    ),
                    _ => "off".to_string(),
                }
            );
        }
        ladder
    }

    /// Runs the ladder's steps in order: the rung probes and the probed
    /// rung's continuation, the rungs of every pitch, the best rung's seed
    /// rounds, the narrow-signal and minimum-clearance steps. The best
    /// board is then in `best` (None only when every attempt failed).
    pub(crate) fn run<R: LadderRunner>(&mut self, runner: &R) -> Result<(), String> {
        self.probes(runner)?;
        self.rungs(runner)?;
        self.clear_rung_directories()?;
        self.seed_rounds(runner)?;
        self.steps(runner)
    }

    /// The ladder's work as the budget counts it: the work, or the wall
    /// clock over `GUARD` once it is ahead beyond the margin (route mode).
    /// The ladder's budget is spent in work (search expansions in seconds
    /// of an idle machine), so that a busy machine routes the same way; the
    /// wall clock counts only beyond `GUARD` times that. KiCad's checks of
    /// the attempts count in it too, at a modelled cost per check that
    /// depends only on the board (`kicad_check_work`), so that a check the
    /// DRC cache answers counts as one it does not.
    fn spent(&self, work: f64) -> f64 {
        if self.config.deadline.is_some() || !core::router::wall_guard() {
            work
        } else {
            let wall = self.started.elapsed().as_secs_f64();
            if wall / core::router::guard() > work + self.wall_margin {
                core::router::note_wall_cut(
                    "ladder",
                    format_args!("{wall:.0} s of wall clock for {work:.0} s of the ladder's work"),
                );
                wall / core::router::guard()
            } else {
                work
            }
        }
    }

    /// Route mode: what is left under the work bound.
    pub(crate) fn bound_left(&self) -> Option<f64> {
        self.work_bound
            .map(|bound| (bound - self.spent(self.work)).max(0.0))
    }

    /// Route mode: what is left under the bound for rungs and seeds (the
    /// narrow-signal step's reserve not counted).
    pub(crate) fn rung_left(&self) -> Option<f64> {
        self.bound_left()
            .map(|left| (left - self.route_reserve).max(0.0))
    }

    /// Route mode: keeps work for the narrow-signal step once the first
    /// attempt (of `first_work`) left connections open.
    fn keep_for_route_step(&mut self, first_work: f64) {
        let kept = route_step_reserve(first_work, self.bound_left().unwrap_or(0.0));
        self.route_reserve = kept;
        eprintln!(
            "ladder: {kept:.1} s of work kept for the narrow-signal step ({:.1} s left under the bound, the first attempt took {first_work:.1} s)",
            self.bound_left().unwrap_or(0.0)
        );
    }

    /// Whether the bound leaves room for another rung or seed round
    /// (always, in a layout's ladder).
    fn bound_allows(&self) -> bool {
        self.rung_left()
            .is_none_or(|left| bound_allows_attempt(left, self.ladder_budget))
    }

    /// Whether the wall clock passed the deadline (noted as a cut).
    pub(crate) fn past_deadline(&self) -> bool {
        let config = self.config;
        let past = config
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline);
        if past {
            core::router::note_wall_cut(
                "deadline",
                format_args!(
                    "the ladder at {:.0} s of its work{}",
                    self.work,
                    config
                        .deadline_seconds
                        .map(|seconds| format!(
                            ", the wall guard at {seconds:.0} s (deadline_seconds)"
                        ))
                        .unwrap_or_default()
                ),
            );
        }
        past
    }

    /// The work left before the work deadline (a layout's; infinite in
    /// route mode).
    fn work_left(&self) -> f64 {
        self.config
            .deadline_work
            .map_or(f64::INFINITY, |left| left - self.work)
    }

    /// An attempt's work as the ladder counts it: its search expansions
    /// and, in route mode, the rest of what it did (`extra_charge`).
    fn result_work(&self, result: &KiCadBoardRouterResult) -> f64 {
        work_of(result.expansions) + self.extra_charge(result)
    }

    /// The part of `result_work` beyond the search: in route mode every
    /// pour analysis and fill probe of the attempt (its negotiation's pour
    /// refresh, the finishing phases' pour checks, the manual spokes), its
    /// lattice's build and its board's lowering, each modelled from what
    /// it counted (`charges_all`, since 2026-10-22: brickpico's via
    /// reduction did 8.5 s of pour work beside 25 s of search, ChengminLi
    /// built 15 lattices in 40 thread seconds); under
    /// `PCB_LADDER_UNCHARGED_POUR` its manual spokes alone (pour analyses
    /// and fill probes after the negotiation, which no work deadline
    /// ends: Vortac's panel spent 30 s of them in every attempt beside 5 s
    /// of search, and the wall guard cut every run).
    fn extra_charge(&self, result: &KiCadBoardRouterResult) -> f64 {
        beyond_search(result, self.charges_all, self.charges_spokes)
    }

    /// Counts work of each kind in the ladder's (`charged`, `work`).
    fn charge(&mut self, search: f64, pour: f64, build: f64, lowering: f64) {
        self.charged.search_seconds += search;
        self.charged.pour_seconds += pour;
        self.charged.lattice_seconds += build;
        self.charged.lowering_seconds += lowering;
        self.work += search + pour + build + lowering;
    }

    /// Counts an attempt's work `times` (seeds side by side count as
    /// that many attempts).
    fn charge_result(&mut self, result: &KiCadBoardRouterResult, times: f64) {
        let search = work_of(result.expansions) * times;
        if self.charges_all {
            self.charge(
                search,
                result.pour_work_seconds * times,
                result.build_work_seconds * times,
                result.lowering_work_seconds * times,
            );
        } else {
            self.charge(search, self.extra_charge(result) * times, 0.0, 0.0);
        }
    }

    /// Route mode, all work charged: the share of the routers' work so far
    /// that was search (`search_fraction`), 1 otherwise.
    fn search_share(&self) -> f64 {
        if self.converts_shares {
            let charged = &self.charged;
            search_fraction(
                charged.search_seconds,
                charged.search_seconds + charged.pour_seconds + charged.lattice_seconds,
            )
        } else {
            1.0
        }
    }

    /// A share of the ladder's work (`charged`, seconds of all the work
    /// charged) as a router's limit in its search work: the router stops
    /// on its search alone, and its pour analyses and build come on top
    /// in the proportion they had so far. Unchanged before the first
    /// search, in a layout's ladder and under `PCB_LADDER_SEARCH_SHARES`.
    fn router_work(&self, charged: f64) -> f64 {
        charged * self.search_share()
    }

    /// Logs a router's limits as the ladder gave them (`router_work`).
    fn note_shares(&self, what: &str, deadline: Option<f64>, negotiation: Option<f64>) {
        if self.converts_shares {
            eprintln!(
                "ladder: {what}: work deadline {}, negotiation {} (search work, {:.3} of the routers' work so far)",
                deadline.map_or("none".to_string(), |deadline| format!("{deadline:.2} s")),
                negotiation.map_or("its own".to_string(), |seconds| format!("{seconds:.2} s")),
                self.search_share()
            );
        }
    }

    /// Route mode: notes the ladder's first attempt's first negotiation
    /// iteration (`first_iteration`): its search and, with all work
    /// charged, its lattice's build and its board's lowering before it.
    fn note_first_iteration(&mut self, result: &KiCadBoardRouterResult) {
        if self.work_bound.is_some()
            && self.first_iteration == 0.0
            && std::env::var_os("PCB_LADDER_NO_FIRST_ITERATION_GATE").is_none()
        {
            self.first_iteration = work_of(result.first_iteration_expansions)
                + if self.charges_all {
                    result.build_work_seconds + result.lowering_work_seconds
                } else {
                    0.0
                };
        }
    }

    /// An attempt's check comes on top of its routing (`check_charge`).
    fn no_time_for_another(&self) -> bool {
        self.work_left() - self.reserve < self.last_attempt_work.max(120.0) + self.check_charge
    }

    /// Counts `checks` KiCad checks as work.
    pub(crate) fn charge_checks(&mut self, checks: usize) {
        let work = self.checks_charged.add(checks);
        self.charged.kicad_check_seconds += work;
        self.work += work;
    }

    /// An attempt's configuration as it routes: without KiCad's check,
    /// which `judge` runs when it matters.
    fn unchecked(&self, attempt: &KiCadBoardRouterConfig) -> KiCadBoardRouterConfig {
        let mut attempt = attempt.clone();
        attempt.skip_native_verification = attempt.skip_native_verification || self.checking;
        attempt
    }

    /// The best board's quality as the check gate compares attempts.
    fn quality(&self) -> Option<CheckedQuality> {
        self.best
            .as_ref()
            .map(|(opens, result)| (*opens, result.vias, result.length_mm))
    }

    /// Unconnected items as the router and, when asked, KiCad see them,
    /// and separately the starved thermals (pads KiCad does not count as
    /// connected to their pour): those rank attempts but, being a matter
    /// of zone settings as often as of routing, do not call for further
    /// rungs.
    fn open<R: LadderRunner>(
        runner: &R,
        result: &KiCadBoardRouterResult,
        directory: &Path,
    ) -> Opens {
        (
            result.unconnected_terminals
                + result.internal_violations.len()
                + result
                    .native
                    .as_ref()
                    .map_or(0, |native| native.selected_net_unconnected_items),
            runner.starved(directory),
        )
    }

    /// An attempt's (open, starved) as KiCad checked it, or None when its
    /// check was skipped because it cannot win (`CheckGate`; its router
    /// count, a lower bound of its open count, is then what it is ranked
    /// by).
    fn judge<R: LadderRunner>(
        &mut self,
        runner: &R,
        result: &mut KiCadBoardRouterResult,
        directory: &Path,
        best: Option<CheckedQuality>,
    ) -> Result<Option<Opens>, String> {
        if !self.checking {
            return Ok(Some(Self::open(runner, result, directory)));
        }
        let count = router_open(result);
        // Past the deadline no further attempt starts: only whether this
        // one beats the kept boards matters.
        let finishing = self.past_deadline();
        if !self
            .gate
            .wants(count, result.vias, result.length_mm, best, finishing)
        {
            self.gate.skipped += 1;
            CHECKS_SKIPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let winnable =
                best.is_none_or(|best| could_win(count, result.vias, result.length_mm, best));
            eprintln!(
                "board check: KiCad skipped, the attempt {} (router count {count}, {} vias; best {}{})",
                if winnable && !finishing {
                    "is not among the best by the router's measures"
                } else {
                    "cannot win"
                },
                result.vias,
                best.map_or("none".to_string(), |((open, starved), vias, _)| format!(
                    "{open} open, {starved} starved, {vias} vias"
                )),
                if finishing { ", past the deadline" } else { "" }
            );
            return Ok(None);
        }
        runner.check(result, directory)?;
        self.charge_checks(1);
        self.gate.record((count, result.vias, result.length_mm));
        Ok(Some(Self::open(runner, result, directory)))
    }

    /// `judge`, isolated: None when judging panicked (recorded as `name`'s
    /// failure), else the judgement.
    fn judge_isolated<R: LadderRunner>(
        &mut self,
        runner: &R,
        name: &str,
        result: &mut KiCadBoardRouterResult,
        directory: &Path,
        best: Option<CheckedQuality>,
    ) -> Result<Option<Option<Opens>>, String> {
        let judged = isolate(|| self.judge(runner, result, directory, best));
        match self.survived(name, judged) {
            Some(judged) => Ok(Some(judged?)),
            None => Ok(None),
        }
    }

    /// An isolated attempt's value, or None when it panicked: then the
    /// failure is recorded and logged.
    fn survived<T>(&mut self, name: &str, outcome: Result<T, String>) -> Option<T> {
        match outcome {
            Ok(value) => Some(value),
            Err(message) => {
                self.fail(name, &message);
                None
            }
        }
    }

    /// Records that the attempt `name` panicked with `message`.
    fn fail(&mut self, name: &str, message: &str) {
        eprintln!(
            "ladder: {name} failed (panic: {message}); the ladder goes on with {}",
            if self.best.is_some() {
                "its best board"
            } else {
                "the next attempt"
            }
        );
        self.failed.push(format!("{name}: panic: {message}"));
    }

    /// Whether the test hook makes the attempt of `kind` starting now
    /// panic (`TestPanic`); counts the attempt.
    fn panic_due(&mut self, kind: &'static str) -> bool {
        let started = self.started_by_kind.entry(kind).or_insert(0);
        *started += 1;
        self.panic_at
            .as_ref()
            .is_some_and(|hook| hook.kind == kind && (hook.nth == 0 || hook.nth == *started))
    }

    /// How many routers of `maps_mb` of maps each run side by side, at
    /// most `wanted`, and why, for the log.
    fn side_by_side_for<R: LadderRunner>(
        runner: &R,
        maps_mb: f64,
        wanted: usize,
    ) -> (usize, String) {
        let limits = runner.memory_limits();
        let (count, limit) = limits.routers_that_fit(maps_mb, wanted);
        (count, fit_note(maps_mb, limit))
    }

    /// A rung's name for the log and the failures.
    fn rung_name(&self, mode: usize) -> String {
        let (connect, skeleton, exclusive, plane_stubs) = self.modes[mode];
        format!(
            "pours={}{}",
            self.pours_name(connect, skeleton),
            if exclusive {
                " (exclusive planes)"
            } else if plane_stubs {
                " (plane stubs)"
            } else {
                ""
            }
        )
    }

    /// The report's `pours` of an attempt.
    fn pours_name(&self, connect: bool, skeleton: bool) -> &'static str {
        if !self.has_pours {
            "none"
        } else if connect && skeleton {
            "connect+skeleton"
        } else if connect {
            "connect"
        } else {
            "tracks"
        }
    }

    /// The configuration of rung `mode` before any budget.
    fn rung_config(&self, mode: usize) -> KiCadBoardRouterConfig {
        let (_, skeleton, exclusive, plane_stubs) = self.modes[mode];
        let mut attempt = self.config.clone();
        attempt.plane_skeleton = Some(skeleton);
        attempt.exclusive_planes = Some(exclusive);
        attempt.fixed_plane_stubs = Some(plane_stubs);
        if exclusive && attempt.exclusive_layers.is_none() {
            attempt.exclusive_layers = Some(if self.copper_layers >= 6 { 2 } else { 1 });
        }
        attempt
    }

    /// Notes the first attempt's open count: the work kept for the
    /// narrow-signal step (`reserve` in a layout, `route_reserve` in route
    /// mode), once an attempt with `work` left connections open.
    fn keep_reserves(&mut self, opens: Opens, work: f64, negotiation: f64) {
        if self.reserve_wanted && !self.reserve_set && opens.0 > 0 {
            self.reserve_set = true;
            self.first_negotiation = negotiation;
            self.reserve = reserve_with_check(
                narrow_step_reserve(work, self.reserve_share, self.reserve_cap),
                self.check_charge,
            );
            eprintln!(
                "ladder: {:.0} s of work kept for the narrow-signal step ({:.0} s left, the attempt took {:.0} s)",
                self.reserve,
                self.work_left(),
                work
            );
        }
        if self.route_reserve_wanted && !self.route_reserve_set && opens.0 > 0 {
            self.route_reserve_set = true;
            self.keep_for_route_step(work);
        }
    }

    /// Large multilayer boards: no fixed split of the budget suits them all
    /// (one needs 600 s of negotiation in its first rung, another the last
    /// rungs). Every rung negotiates briefly first; the one with the fewest
    /// unfinished nets gets the rest of the budget and goes on from where
    /// its probe stopped. Only the leading router is kept (memory).
    #[allow(clippy::type_complexity)]
    pub(crate) fn probes<R: LadderRunner>(&mut self, runner: &R) -> Result<(), String> {
        let config = self.config;
        if !(config.probe_ladder.unwrap_or(true)
            && self.modes.len() >= 3
            && config.negotiation_seconds.is_none())
        {
            return Ok(());
        }
        let probe_seconds = config.probe_seconds.unwrap_or(75.0);
        let mut leader: Option<(usize, usize, R::Probe, R::Board, KiCadBoardRouterConfig)> = None;
        // The rungs' boards, in rung order.
        let mut rungs: Vec<(usize, R::Board, KiCadBoardRouterConfig)> = Vec::new();
        for mode in 0..self.modes.len() {
            let attempt = self.rung_config(mode);
            let board = runner.lower(&attempt, self.modes[mode].0)?;
            if self.charges_all {
                self.charge(0.0, 0.0, 0.0, runner.lowering_work(&board));
            }
            let (layers, connections) = runner.size(&board);
            if mode == 0 && (layers < 4 || connections < 120) {
                break;
            }
            rungs.push((mode, board, attempt));
        }
        // Route mode: the probes together take at most `PROBE_SHARE` of
        // the budget (in a layout, `probe_seconds` each as before).
        let probe_work = if self.work_bound.is_some() && !rungs.is_empty() {
            let each = bounded_probe_seconds(probe_seconds, self.ladder_budget, rungs.len());
            if each < probe_seconds {
                eprintln!(
                    "ladder: probes of {each:.1} s of work each ({:.0} % of the {:.0} s budget over {} rungs)",
                    PROBE_SHARE * 100.0,
                    self.ladder_budget,
                    rungs.len()
                );
            }
            each
        } else {
            probe_seconds
        };
        if !rungs.is_empty()
            && (self.past_deadline() || self.work_left() < probe_seconds * 3.0 + 180.0)
        {
            eprintln!("deadline too near for the ladder: no attempts");
            self.skipped.push(
                "the rung probes did not run: too little work left before the deadline".into(),
            );
            rungs.clear();
        }
        // The probes run side by side, each a router of its own: as many
        // at once as fit half the memory available (the first router
        // tells its size). The outcome is the one of probing one rung
        // after the other: the fewest unfinished nets lead, a tie goes to
        // the earlier rung, a rung that finishes everything ends the
        // probing, and only the probes up to it count as work. A probe's
        // budget is work, so its outcome does not depend on what runs
        // beside it.
        let mut first: Option<(R::Probe, f64)> = None;
        // The first router's lattice maps (its memory report).
        let mut router_mb_estimate: Option<f64> = None;
        let mut next = 0;
        while next < rungs.len() {
            // A probe takes its seconds (more on a busy machine), the
            // continuation and the board's check more: no probe starts
            // without that much wall clock left (PolyKybd right's ladder
            // started at 1500 s and the harness killed the board at
            // 1800 s).
            if leader.is_some()
                && (self.past_deadline() || self.work_left() < probe_seconds * 2.0 + 120.0)
            {
                break;
            }
            if router_mb_estimate.is_none() {
                let started = std::time::Instant::now();
                let (mode, board, attempt) = &rungs[next];
                let name = format!("the probe of rung {}", self.rung_name(*mode));
                let built = isolate(|| {
                    let router = runner.build(board, attempt, false);
                    let megabytes = runner.maps_mb(&router).unwrap_or(1.0);
                    (router, megabytes)
                });
                let Some((router, megabytes)) = self.survived(&name, built) else {
                    next += 1;
                    continue;
                };
                // The leading router stays in memory while the next probe
                // runs: under a limit of the process's own that two do not
                // fit (decided by the limit, not by what is free), the
                // rungs run one after the other in the regular ladder.
                if let Some(limit) = self.memory_at_start.too_small_for(megabytes, 2) {
                    eprintln!(
                        "ladder: no rung probes: two routers of about {:.0} MB each ({megabytes:.0} MB of maps) do not fit {} of {:.0} MB",
                        limit.router_mb(megabytes),
                        limit.source,
                        limit.limit_mb
                    );
                    drop(router);
                    rungs.clear();
                    break;
                }
                first = Some((router, started.elapsed().as_secs_f64()));
                router_mb_estimate = Some(megabytes);
            }
            // The search scratches and the via reduction's copy come on
            // top of the maps a new router holds (`router_peak_mb`).
            let (fitting, why) = Self::side_by_side_for(
                runner,
                router_mb_estimate.unwrap_or(1.0),
                rungs.len() - next,
            );
            let side_by_side = fitting
                // A cap for experiments (1: one after the other).
                .min(
                    std::env::var("PCB_PROBES_SIDE_BY_SIDE")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(usize::MAX),
                );
            eprintln!(
                "ladder: {side_by_side} of {} probes side by side ({why})",
                rungs.len() - next
            );
            let wave = &rungs[next..next + side_by_side];
            let hooks: Vec<bool> = wave.iter().map(|_| self.panic_due("probe")).collect();
            let first_router = first.take();
            let probed: Vec<Result<(usize, usize, R::Probe, f64), String>> =
                std::thread::scope(|scope| {
                    let mut first_router = first_router;
                    let handles: Vec<_> = wave
                        .iter()
                        .zip(&hooks)
                        .enumerate()
                        .map(|(index, ((mode, board, attempt), &panics))| {
                            let built = first_router.take();
                            scope.spawn(move || {
                                isolate(|| {
                                    let started = std::time::Instant::now();
                                    // One probe's lines in the log; the
                                    // others' would interleave with them.
                                    let (mut router, built_seconds) = built.unwrap_or_else(|| {
                                        (runner.build(board, attempt, index != 0), 0.0)
                                    });
                                    if panics {
                                        test_panic("probe");
                                    }
                                    let unfinished = runner.probe(&mut router, probe_work, *mode);
                                    (
                                        *mode,
                                        unfinished,
                                        router,
                                        built_seconds + started.elapsed().as_secs_f64(),
                                    )
                                })
                            })
                        })
                        .collect();
                    handles
                        .into_iter()
                        .map(|handle| {
                            handle
                                .join()
                                .unwrap_or_else(|payload| Err(panic_message(payload.as_ref())))
                        })
                        .collect()
                });
            let mut out_of_work = false;
            for (outcome, (wave_mode, board, attempt)) in probed.into_iter().zip(wave) {
                // As one probe after the other: a probe the work left would
                // not have started counts for nothing, however many ran side
                // by side (that number follows the memory available).
                if leader.is_some() && self.work_left() < probe_seconds * 2.0 + 120.0 {
                    out_of_work = true;
                    break;
                }
                let name = format!("the probe of rung {}", self.rung_name(*wave_mode));
                let Some((mode, unfinished, mut router, seconds)) = self.survived(&name, outcome)
                else {
                    continue;
                };
                let probed = runner.work(&router);
                if self.charges_all {
                    self.charge(work_of(probed.expansions), probed.pour, probed.build, 0.0);
                } else {
                    self.charge(work_of(probed.expansions), 0.0, 0.0, 0.0);
                }
                eprintln!(
                    "probe {}: {unfinished} nets unfinished after {:.0} s{}",
                    self.probe_name(mode),
                    seconds,
                    if side_by_side > 1 {
                        format!(" ({side_by_side} side by side)")
                    } else {
                        String::new()
                    }
                );
                if leader.as_ref().is_none_or(|(best, ..)| unfinished < *best) {
                    runner.speak(&mut router, attempt);
                    leader = Some((unfinished, mode, router, board.clone(), attempt.clone()));
                }
                // A rung that already finishes everything needs no rival
                // (the probes beside it after it count for nothing).
                if leader.as_ref().is_some_and(|(best, ..)| *best == 0) {
                    break;
                }
            }
            next += side_by_side;
            if out_of_work || leader.as_ref().is_some_and(|(best, ..)| *best == 0) {
                break;
            }
        }
        if let Some((_, mode, router, board, attempt)) = leader {
            self.continue_probe(
                runner,
                mode,
                router,
                board,
                attempt,
                probe_seconds,
                router_mb_estimate,
            )?;
        }
        Ok(())
    }

    /// A probe's name in its line: the pours as connected or as tracks
    /// (with the planes exclusive or stubbed).
    fn probe_name(&self, mode: usize) -> String {
        let (connect, _, exclusive, plane_stubs) = self.modes[mode];
        format!(
            "pours={}{}",
            if connect { "connect" } else { "tracks" },
            if exclusive {
                " (exclusive planes)"
            } else if plane_stubs {
                " (plane stubs)"
            } else {
                ""
            }
        )
    }

    /// The leading probe goes on with the rest of the budget from where it
    /// stopped; its board is the ladder's first.
    #[allow(clippy::too_many_arguments)]
    fn continue_probe<R: LadderRunner>(
        &mut self,
        runner: &R,
        mode: usize,
        router: R::Probe,
        board: R::Board,
        attempt: KiCadBoardRouterConfig,
        probe_seconds: f64,
        maps_mb: Option<f64>,
    ) -> Result<(), String> {
        let config = self.config;
        let output_directory = self.output_directory;
        let started = std::time::Instant::now();
        let remaining = self
            .bound_left()
            .unwrap_or_else(|| (self.ladder_budget - self.spent(self.work)).max(60.0));
        let probe_work = runner.work(&router);
        let probed = probe_work.expansions;
        // The router's own probe is in its work already.
        let run_work = (
            self.ladder_budget,
            self.router_work(self.ladder_budget - self.spent(self.work)) + work_of(probed),
        );
        // It stops where the run's work ends, not where the wall clock does
        // (the deadline stays as the guard against load).
        let deadline_work = if config.deadline_work.is_some() {
            Some(work_of(probed) + (self.work_left() - self.check_charge).max(0.0))
        } else {
            // Route mode: it stops where the ladder's bound ends.
            self.bound_left()
                .map(|left| work_of(probed) + self.router_work(left))
        };
        let resumed = if self.work_bound.is_some() {
            self.router_work(bounded_negotiation(remaining / 3.0, remaining))
        } else {
            (remaining / 3.0).clamp(60.0, 900.0)
        };
        let name = format!(
            "the continuation of the probed rung {}",
            self.rung_name(mode)
        );
        self.note_shares(&name, deadline_work, Some(resumed));
        let panics = self.panic_due("continuation");
        let outcome = isolate(|| {
            if panics {
                test_panic("continuation");
            }
            runner.resume(
                router,
                ResumeLimits {
                    run_work,
                    deadline_work,
                    negotiation: resumed,
                },
            )
        });
        let Some((routed, resumed_work)) = self.survived(&name, outcome) else {
            return Ok(());
        };
        let router_expansions = resumed_work.expansions;
        if self.charges_all {
            self.charge(
                work_of(router_expansions - probed),
                resumed_work.pour - probe_work.pour,
                0.0,
                0.0,
            );
        } else {
            self.charge(work_of(router_expansions - probed), 0.0, 0.0, 0.0);
        }
        if output_directory.exists() {
            fs::remove_dir_all(output_directory).map_err(|error| error.to_string())?;
        }
        let (connect, skeleton, exclusive, plane_stubs) = self.modes[mode];
        let unchecked = self.unchecked(&attempt);
        let written = isolate(|| {
            runner.write(
                &board,
                routed,
                &unchecked,
                output_directory,
                started.elapsed().as_secs_f64(),
            )
        });
        let Some(written) = self.survived(&name, written) else {
            return Ok(());
        };
        let mut result = written?;
        // The probe's work and its own are counted: with all work charged,
        // its pour analyses (the manual spokes' among them) too; else its
        // manual spokes.
        if !self.charges_all {
            let spokes = self.extra_charge(&result);
            self.charge(0.0, spokes, 0.0, 0.0);
        }
        self.note_first_iteration(&result);
        result.pours = self.pours_name(connect, skeleton).into();
        result.maps_mb = maps_mb.unwrap_or(0.0);
        self.router_maps = Some((result.maps_mb, result.grid_pitch_mm));
        let Some(judged) =
            self.judge_isolated(runner, &name, &mut result, output_directory, None)?
        else {
            return Ok(());
        };
        let opens = judged.unwrap_or((router_open(&result), 0));
        eprintln!(
            "attempt pours={}{} pitch=regular (probed, continued): {} open{}, {} starved, {} vias, {:.1} s",
            result.pours,
            if exclusive {
                " (exclusive planes)"
            } else if plane_stubs {
                " (plane stubs)"
            } else {
                ""
            },
            opens.0,
            by_router(&judged),
            opens.1,
            result.vias,
            self.started.elapsed().as_secs_f64()
        );
        // The whole router's work, its probe's included (its board's
        // lowering came with the probes).
        let attempt_work = self.result_work(&result);
        self.slowest = Some((attempt_work, result.grid_pitch_mm));
        self.last_attempt_work = attempt_work;
        self.last_attempt_pitch = result.grid_pitch_mm;
        self.keep_reserves(opens, attempt_work, probe_seconds + resumed);
        self.probe_complete = opens.0 == 0;
        self.best = Some((opens, result));
        self.best_attempt = Some((attempt.clone(), connect));
        self.probed_mode = Some(mode);
        Ok(())
    }

    /// The rungs on every pitch, until a board is clean or the budget,
    /// the bound or a finer pitch's projection stops them.
    pub(crate) fn rungs<R: LadderRunner>(&mut self, runner: &R) -> Result<(), String> {
        let pitches = self.pitches.clone();
        for pitch in &pitches {
            if self.probe_complete {
                continue;
            }
            if self.pitch_skipped(pitch) {
                break;
            }
            if self.pitch(runner, pitch)? == Flow::Stop {
                break;
            }
        }
        Ok(())
    }

    /// Whether a finer `pitch` is not tried (logged, and noted in
    /// `skipped`): its projected work exceeds the refinement budget or
    /// what is left under the bound, or its router does not fit a memory
    /// limit of the process's own or the machine. Every memory decision
    /// goes by the router's estimate from the first attempt's maps against
    /// a fixed limit, never by what the process happens to hold (until
    /// 2026-10-21 the machine's check used the process's peak resident
    /// size, which follows how many probes ran side by side).
    pub(crate) fn pitch_skipped(&mut self, pitch: &Option<Vec<f64>>) -> bool {
        let budget = self.budget;
        if let (Some(pitch), Some((seconds, previous))) = (pitch, self.slowest) {
            let projected = seconds * (previous / pitch[0]).powi(2);
            if projected > budget {
                eprintln!(
                    "skipping pitch {:?}: projected {projected:.0} s exceeds the {budget:.0} s refinement budget",
                    pitch
                );
                self.skipped.push(format!(
                    "the finer lattice ({} mm) was not tried: projected {projected:.0} s of routing exceeds refine_budget_seconds ({budget:.0} s)",
                    pitch[0]
                ));
                return true;
            }
        }
        // Route mode: a finer lattice whose attempt would take more than is
        // left under the ladder's bound is not started.
        if let (Some(pitch), Some((seconds, previous)), Some(left)) =
            (pitch, self.slowest, self.rung_left())
        {
            let projected = seconds * (previous / pitch[0]).powi(2);
            if projected > left {
                eprintln!(
                    "skipping pitch {:?}: projected {projected:.0} s exceeds the {left:.0} s left of the ladder's bound",
                    pitch
                );
                self.skipped.push(format!(
                    "the finer lattice ({} mm) was not tried: projected {projected:.0} s of routing exceeds the {left:.0} s left of the ladder's work bound (ladder_budget_seconds)",
                    pitch[0]
                ));
                return true;
            }
        }
        // A limit of the process's own (its address space, its cgroup, an
        // explicit one) one router of the finer lattice does not fit.
        if let (Some(pitch), Some((maps, previous))) = (pitch, self.router_maps) {
            let projected = maps * (previous / pitch[0]).powi(2);
            if let Some(limit) = self.memory_at_start.too_small_for(projected, 1) {
                eprintln!(
                    "skipping pitch {:?}: a router of about {:.0} MB ({projected:.0} MB of maps) does not fit {} of {:.0} MB",
                    pitch,
                    limit.router_mb(projected),
                    limit.source,
                    limit.limit_mb
                );
                self.skipped.push(format!(
                    "the finer lattice ({} mm) was not tried: a router of about {:.0} MB does not fit {} of {:.0} MB",
                    pitch[0],
                    limit.router_mb(projected),
                    limit.source,
                    limit.limit_mb
                ));
                return true;
            }
            // Against the machine's memory, not what is available now: the
            // attempts may not depend on what else runs (jiran-ble-lite:
            // 3299 x 1048 nodes at 0.1 mm, 7 GB a router at 0.05 mm, four
            // seeds of it took 31 GB and the machine's last free byte).
            let router = router_peak_mb(projected, MemoryKind::Resident);
            if self
                .memory_total_mb
                .is_some_and(|total| router > total * 0.4)
            {
                eprintln!(
                    "skipping pitch {:?}: a router of about {:.1} GB does not fit the machine's memory",
                    pitch,
                    router / 1024.0
                );
                self.skipped.push(format!(
                    "the finer lattice ({} mm) was not tried: a router of about {:.1} GB does not fit the machine's memory",
                    pitch[0],
                    router / 1024.0
                ));
                return true;
            }
        }
        false
    }

    /// The configuration of rung `mode` on `pitch` with the pitch's plan.
    fn rung_attempt(
        &self,
        mode: usize,
        pitch: &Option<Vec<f64>>,
        plan: &PitchPlan,
    ) -> KiCadBoardRouterConfig {
        let mut attempt = self.rung_config(mode);
        if let Some(seconds) = plan.negotiation {
            attempt.negotiation_seconds = Some(seconds);
        }
        attempt.run_work = Some((self.ladder_budget, plan.left));
        // Every rung of the pitch stops where the run's work ends as
        // the pitch starts (decided before they run, as the budget),
        // less the work kept for the narrow-signal step.
        // Its check follows: the router stops where that still fits.
        attempt.router_deadline_work = self
            .config
            .deadline_work
            .map(|_| {
                if self.reserve > 0.0 {
                    deadline_before_reserve(plan.work_left - self.check_charge, self.reserve, 1)
                } else {
                    (plan.work_left - self.check_charge).max(0.0)
                }
            })
            .or(plan.route_deadline);
        if let Some(pitch) = pitch {
            attempt.grid_pitches_mm = Some(pitch.clone());
        }
        attempt
    }

    /// Where rung `mode`'s attempt routes.
    fn rung_directory(&self, mode: usize) -> PathBuf {
        self.output_directory.with_extension(format!("rung{mode}"))
    }

    /// The rungs of one pitch.
    fn pitch<R: LadderRunner>(
        &mut self,
        runner: &R,
        pitch: &Option<Vec<f64>>,
    ) -> Result<Flow, String> {
        let config = self.config;
        let output_directory = self.output_directory;
        let budget = self.budget;
        let ladder_budget = self.ladder_budget;
        let modes = self.modes.clone();
        // Every rung of this pitch gets the same budget, decided before
        // any of them runs (an equal share of what is left of the ladder's
        // budget; an attempt spends up to about three times its
        // negotiation in repair and polish, so negotiation gets a third of
        // the share: zpn_devboard's two rungs took 1300 s and the two that
        // complete such boards never ran). A rung's outcome then depends
        // on nothing that runs before or beside it, and the rungs after
        // the first run side by side, as many as fit the memory; they are
        // judged in rung order as one after another, a rung the order
        // would not have reached counting for nothing.
        let negotiation = config.negotiation_seconds.is_none().then(|| {
            if let Some(left) = self.rung_left() {
                self.router_work(bounded_negotiation(left / modes.len() as f64 / 3.0, left))
            } else {
                let remaining = (ladder_budget - self.spent(self.work)).max(0.0);
                (remaining / modes.len() as f64 / 3.0).clamp(60.0, 900.0)
            }
        });
        // Route mode: each rung's work deadline (`rung_share`): the
        // ladder's first attempt has all that is left under the bound, the
        // rungs of a pitch after it an even share of what is left when the
        // first of them starts, decided before they run.
        let mut plan = PitchPlan {
            negotiation,
            route_deadline: None,
            share_fixed: false,
            left: self.router_work((ladder_budget - self.spent(self.work)).max(0.0)),
            work_left: self.work_left().max(0.0),
        };
        // A probed rung continued with what it had is not run again; the
        // other rungs still get their turn when it left connections open.
        let candidates: Vec<usize> = (0..modes.len())
            .filter(|mode| !(pitch.is_none() && self.probed_mode == Some(*mode)))
            .collect();
        // Each rung's outcome: its result, or the panic's message.
        let mut ran: Vec<Option<Result<Result<KiCadBoardRouterResult, String>, String>>> =
            (0..modes.len()).map(|_| None).collect();
        for (position, &mode) in candidates.iter().enumerate() {
            let (connect, skeleton, exclusive, plane_stubs) = modes[mode];
            // With work kept for the narrow-signal step, the rung's work
            // as projected from the last attempt (on a finer lattice, by
            // the square of the pitch ratio: DSP-ADAU1452's 0.05 mm rung
            // after its 0.075 mm rungs) must leave all of it.
            let short_of_work = if self.work_bound.is_some() {
                !self.bound_allows()
            } else if self.reserve > 0.0 {
                let cost = pitch.as_ref().map_or(self.last_attempt_work, |pitch| {
                    projected_attempt_work(
                        self.last_attempt_work,
                        self.last_attempt_pitch,
                        pitch[0],
                    )
                });
                !fits_before_reserve(self.work_left() - self.check_charge, self.reserve, cost)
            } else {
                self.no_time_for_another()
            };
            if self.best.is_some() && (self.past_deadline() || short_of_work) {
                if self.work_bound.is_some() && short_of_work {
                    eprintln!(
                        "ladder: its work bound is reached ({:.0} s left of {:.0} s, {:.0} s of it kept for the narrow-signal step): no further attempts",
                        self.bound_left().unwrap_or(0.0),
                        self.work_bound.unwrap_or(0.0),
                        self.route_reserve
                    );
                    self.skipped.push(format!(
                        "further rungs of the attempt ladder did not run: its work bound ({:.0} s, ladder_budget_seconds and {:.0} %) was reached",
                        self.work_bound.unwrap_or(0.0),
                        LADDER_WORK_TOLERANCE * 100.0
                    ));
                } else {
                    eprintln!("deadline reached: no further attempts");
                    self.skipped.push("further rungs of the attempt ladder did not run: the run's end was near (its deadline, or too little work left for another attempt)".into());
                }
                return Ok(Flow::Stop);
            }
            // The skeleton and the plane stubs only help when the plain
            // pour connection left pads of a pour net open; elsewhere they
            // just take room.
            if ((skeleton && !config.plane_skeleton.unwrap_or(false))
                || (plane_stubs && !config.fixed_plane_stubs.unwrap_or(false)))
                && self.best.as_ref().is_some_and(|(_, result)| {
                    result.pours == "connect"
                        && !result.nets.iter().any(|net| {
                            net.unconnected_terminals > 0
                                && self.pour_nets.contains(&net.connection)
                        })
                })
            {
                if self.rung_directory(mode).exists() {
                    fs::remove_dir_all(self.rung_directory(mode))
                        .map_err(|error| error.to_string())?;
                }
                continue;
            }
            if self.work_bound.is_some() {
                if self.best.is_none() {
                    plan.route_deadline = self.bound_left().map(|left| self.router_work(left));
                } else if !plan.share_fixed {
                    plan.share_fixed = true;
                    plan.route_deadline = self.rung_left().map(|left| {
                        self.router_work(rung_share(left, candidates.len() - position))
                    });
                }
            }
            let attempt = self.rung_attempt(mode, pitch, &plan);
            if ran[mode].is_none() {
                self.note_shares(
                    &format!("the rung {}", self.rung_name(mode)),
                    attempt.router_deadline_work,
                    attempt.negotiation_seconds,
                );
                // The first rung of a pitch without a board yet runs alone
                // (it often completes the board); after it, the rest run
                // side by side, each a router of this pitch and a KiCad
                // check.
                let side_by_side =
                    self.rungs_side_by_side(runner, candidates.len() - position, pitch);
                let wave: Vec<usize> = candidates[position..position + side_by_side].to_vec();
                for (other, result) in self.rung_wave(runner, &wave, pitch, &plan) {
                    ran[other] = Some(result);
                }
            }
            let directory = self.rung_directory(mode);
            let name = format!(
                "the rung {} pitch={}",
                self.rung_name(mode),
                pitch
                    .as_ref()
                    .map_or("regular".to_string(), |pitch| format!("{:?}", pitch))
            );
            let outcome = ran[mode].take().expect("the rung ran");
            let Some(routed) = self.survived(&name, outcome) else {
                if directory.exists() {
                    fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
                }
                continue;
            };
            let mut result = routed?;
            result.pours = self.pours_name(connect, skeleton).into();
            let quality = self.quality();
            let Some(mut judged) =
                self.judge_isolated(runner, &name, &mut result, &directory, quality)?
            else {
                if directory.exists() {
                    fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
                }
                continue;
            };
            let mut opens = judged.unwrap_or((router_open(&result), 0));
            if self.router_maps.is_none() && result.maps_mb > 0.0 {
                self.router_maps = Some((result.maps_mb, result.grid_pitch_mm));
            }
            // What one attempt at this pitch costs, for projecting finer
            // pitches; a seed retry does not change it.
            // In work, as the ladder's budget.
            let attempt_seconds = self.result_work(&result);
            self.charge_result(&result, 1.0);
            self.note_first_iteration(&result);
            // Open connections often depend on the order: route the same
            // attempt a few perturbed ways and keep the better board.
            let retry = config.retry_seeds.unwrap_or(4);
            // Four seeds in parallel cost about twice the attempt; on a
            // board whose attempt takes minutes that time is better spent
            // on the next rung (MIDAS-MK2: two retries, 740 s, both worse).
            // The seeds wait while other rungs remain: another way of
            // connecting the pours often finishes the board (olimex-c3,
            // stickhub and interf-u spent 40-130 s on seeds of rungs a
            // later one beat); the best rung gets them at the end.
            let mut seeds_tried = false;
            if opens.0 > 0
                && retry > 0
                && attempt.seeds.unwrap_or(1) <= 1
                && attempt.first_seed.unwrap_or(0) == 0
                && attempt_seconds <= budget / 2.0
                && mode + 1 == modes.len()
                && !self.past_deadline()
                // The seeds count as `retry` attempts of work: not out of
                // the work kept for the narrow-signal step.
                && (self.reserve == 0.0 || self.work_left() - self.check_charge - self.reserve >= retry as f64 * attempt_seconds)
                // Route mode: nor beyond the ladder's bound.
                && self.rung_left().is_none_or(|left| left >= retry as f64 * attempt_seconds)
            {
                seeds_tried = true;
                (result, opens, judged) = self.seed_retry(
                    runner,
                    &name,
                    (&attempt, connect, &directory, retry),
                    (result, opens, judged),
                )?;
            }
            eprintln!(
                "attempt pours={}{} pitch={}: {} open{}, {} starved, {} vias, {:.1} s",
                result.pours,
                if exclusive {
                    " (exclusive planes)"
                } else if plane_stubs {
                    " (plane stubs)"
                } else {
                    ""
                },
                pitch
                    .as_ref()
                    .map_or("regular".to_string(), |pitch| format!("{:?}", pitch)),
                opens.0,
                by_router(&judged),
                opens.1,
                result.vias,
                result.routing_seconds
            );
            self.last_attempt_work = attempt_seconds;
            self.last_attempt_pitch = result.grid_pitch_mm;
            self.keep_reserves(
                opens,
                attempt_seconds,
                attempt.negotiation_seconds.unwrap_or(900.0),
            );
            let seconds = attempt_seconds;
            let used = (attempt_seconds, result.grid_pitch_mm);
            if self.slowest.is_none_or(|(seconds, _)| used.0 > seconds) {
                self.slowest = Some(used);
            }
            // Equally open boards: fewer vias, then less copper. A board
            // KiCad did not check cannot beat the best (`CheckGate`).
            let better = self.best.as_ref().is_none_or(|(best_opens, best_result)| {
                judged.is_some()
                    && (opens, result.vias, result.length_mm)
                        .partial_cmp(&(*best_opens, best_result.vias, best_result.length_mm))
                        .is_some_and(|order| order.is_lt())
            });
            if better {
                if output_directory.exists() {
                    fs::remove_dir_all(output_directory).map_err(|error| error.to_string())?;
                }
                fs::rename(&directory, output_directory).map_err(|error| error.to_string())?;
                self.best = Some((opens, result));
                self.best_attempt = Some((attempt.clone(), connect));
                self.best_seeded = seeds_tried || attempt.seeds.unwrap_or(1) > 1;
            } else if directory.exists() {
                fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
            }
            // A complete board needs no narrow-signal step: what was kept
            // for it is the next rung's again (a clean skeleton rung).
            let complete = self.best.as_ref().is_some_and(|(opens, _)| opens.0 == 0);
            if complete && self.reserve > 0.0 {
                eprintln!(
                    "ladder: the board is complete: the {:.0} s kept for the narrow-signal step are released",
                    self.reserve
                );
                self.reserve = 0.0;
            }
            if complete && self.route_reserve > 0.0 {
                eprintln!(
                    "ladder: the board is complete: the {:.0} s kept for the narrow-signal step are released",
                    self.route_reserve
                );
                self.route_reserve = 0.0;
            }
            // Another way of connecting the pours is always worth trying
            // while connections are open (a complete board matters more
            // than the time; the finer pitches below are what the budget
            // limits). A clean board from the plane skeleton still gets the
            // next rung when time allows: the fixed tree often costs many
            // vias that routing the pour nets as tracks does not.
            let skeleton_clean = opens.0 == 0 && connect && skeleton && !self.extra_rung_tried;
            if skeleton_clean && mode + 1 < modes.len() && seconds <= budget / 2.0 {
                self.extra_rung_tried = true;
                continue;
            }
            if opens.0 == 0 || self.extra_rung_tried {
                return Ok(Flow::Stop);
            }
            if self.spent(self.work) > ladder_budget {
                eprintln!("ladder budget of {ladder_budget:.0} s used: no further attempts");
                self.skipped.push(format!(
                    "further rungs of the attempt ladder did not run: its budget of {ladder_budget:.0} s of work was used (ladder_budget_seconds)"
                ));
                return Ok(Flow::Stop);
            }
        }
        Ok(Flow::Next)
    }

    /// How many rungs of `pitch` run side by side, of `wanted`: the first
    /// rung of a pitch without a board yet runs alone (it often completes
    /// the board); after it, the rest run side by side, each a router of
    /// this pitch and a KiCad check, as many as fit the memory.
    fn rungs_side_by_side<R: LadderRunner>(
        &self,
        runner: &R,
        wanted: usize,
        pitch: &Option<Vec<f64>>,
    ) -> usize {
        if self.best.is_none() {
            1
        } else {
            match self.router_maps {
                Some((maps, previous)) => {
                    let each = maps
                        * (previous / pitch.as_ref().map_or(previous, |pitch| pitch[0])).powi(2);
                    let (count, why) = Self::side_by_side_for(runner, each, wanted);
                    eprintln!("ladder: {count} of {wanted} rungs side by side ({why})");
                    count
                }
                // No maps known (an engine without a memory report): one
                // after the other.
                None => 1,
            }
        }
    }

    /// Routes the rungs `wave` of `pitch` side by side, each in a thread
    /// of its own and isolated: each rung's result, or its panic's
    /// message.
    #[allow(clippy::type_complexity)]
    fn rung_wave<R: LadderRunner>(
        &mut self,
        runner: &R,
        wave: &[usize],
        pitch: &Option<Vec<f64>>,
        plan: &PitchPlan,
    ) -> Vec<(
        usize,
        Result<Result<KiCadBoardRouterResult, String>, String>,
    )> {
        let mut jobs = Vec::new();
        for &other in wave {
            let attempt = self.unchecked(&self.rung_attempt(other, pitch, plan));
            let panics = self.panic_due("rung");
            jobs.push((
                other,
                attempt,
                self.rung_directory(other),
                self.modes[other].0,
                panics,
            ));
        }
        std::thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .into_iter()
                .map(|(other, attempt, directory, connect, panics)| {
                    (
                        other,
                        scope.spawn(move || {
                            isolate(|| {
                                if directory.exists() {
                                    fs::remove_dir_all(&directory)
                                        .map_err(|error| error.to_string())?;
                                }
                                if panics {
                                    test_panic("rung");
                                }
                                runner.route(&directory, &attempt, connect)
                            })
                        }),
                    )
                })
                .collect();
            handles
                .into_iter()
                .map(|(other, handle)| {
                    (
                        other,
                        handle
                            .join()
                            .unwrap_or_else(|payload| Err(panic_message(payload.as_ref()))),
                    )
                })
                .collect()
        })
    }

    /// The rung's seed retry: `attempt` (whose pours connect or not, routed
    /// in `directory`) again with `retry` seeds side by side; the better of
    /// the two boards, its open count and KiCad's judgement (the rung's
    /// own when the seeds lose or fail).
    #[allow(clippy::type_complexity)]
    fn seed_retry<R: LadderRunner>(
        &mut self,
        runner: &R,
        name: &str,
        (attempt, connect, directory, retry): (&KiCadBoardRouterConfig, bool, &Path, usize),
        (mut result, mut opens, mut judged): (KiCadBoardRouterResult, Opens, Option<Opens>),
    ) -> Result<(KiCadBoardRouterResult, Opens, Option<Opens>), String> {
        let output_directory = self.output_directory;
        let mut seeded = attempt.clone();
        seeded.seeds = Some(retry);
        seeded.seeds_side_by_side = Some(self.seeds_that_fit(runner, retry, result.grid_pitch_mm));
        seeded.first_seed = Some(1);
        // Their work counts `retry` times: each stops at its share
        // of what is left beyond the reserve.
        if self.reserve > 0.0 {
            seeded.router_deadline_work = Some(deadline_before_reserve(
                self.work_left() - self.check_charge,
                self.reserve,
                retry,
            ));
        } else if let Some(left) = self.rung_left() {
            seeded.router_deadline_work = Some(self.router_work(rung_share(left, retry)));
        }
        let retry_directory = output_directory.with_extension("seeds");
        if retry_directory.exists() {
            fs::remove_dir_all(&retry_directory).map_err(|error| error.to_string())?;
        }
        let seeds_name = format!("{name} again with {retry} seeds");
        let panics = self.panic_due("seeds");
        let unchecked = self.unchecked(&seeded);
        let other = isolate(|| {
            if panics {
                test_panic("seeds");
            }
            runner.route(&retry_directory, &unchecked, connect)
        });
        let judged_other = match self.survived(&seeds_name, other) {
            None => None,
            Some(other) => {
                let mut other = other?;
                other.pours = result.pours.clone();
                // The seeds run side by side: about `retry` times
                // one's work.
                self.charge_result(&other, retry as f64);
                let quality = self.quality();
                self.judge_isolated(runner, &seeds_name, &mut other, &retry_directory, quality)?
                    .map(|judged| (other, judged))
            }
        };
        match judged_other {
            None => {
                if retry_directory.exists() {
                    fs::remove_dir_all(&retry_directory).map_err(|error| error.to_string())?;
                }
            }
            Some((mut other, other_judged)) => {
                let other_opens = other_judged.unwrap_or((router_open(&other), 0));
                eprintln!(
                    "attempt again with {retry} seeds: {} open{}, {} vias, {:.1} s",
                    other_opens.0,
                    by_router(&other_judged),
                    other.vias,
                    other.routing_seconds
                );
                // Boards KiCad checked compare as before; one it
                // did not check cannot beat the best so far, so it
                // never displaces one it did (and which of two
                // unchecked ones stays does not matter: neither
                // beats the best).
                let other_better = match (judged.is_some(), other_judged.is_some()) {
                    (true, true) => (other_opens, other.vias, other.length_mm)
                        .partial_cmp(&(opens, result.vias, result.length_mm))
                        .is_some_and(|order| order.is_lt()),
                    (false, true) => true,
                    (_, false) => false,
                };
                if other_better {
                    fs::remove_dir_all(directory).map_err(|error| error.to_string())?;
                    fs::rename(&retry_directory, directory).map_err(|error| error.to_string())?;
                    other.routing_seconds += result.routing_seconds;
                    result = other;
                    opens = other_opens;
                    judged = other_judged;
                } else {
                    fs::remove_dir_all(&retry_directory).map_err(|error| error.to_string())?;
                }
            }
        }
        Ok((result, opens, judged))
    }

    /// Seeds run side by side, each a router of this pitch, as many at
    /// once as fit the memory (`MemoryLimits`); how many seeds route does
    /// not depend on it.
    fn seeds_that_fit<R: LadderRunner>(&self, runner: &R, wanted: usize, pitch_mm: f64) -> usize {
        let Some((maps, previous)) = self.router_maps else {
            return 1;
        };
        let each = maps * (previous / pitch_mm).powi(2);
        let (count, why) = Self::side_by_side_for(runner, each, wanted);
        if count < wanted {
            eprintln!("ladder: {count} of {wanted} seeds side by side ({why})");
        }
        count
    }

    /// Rungs that ran beside one the order stopped at.
    fn clear_rung_directories(&self) -> Result<(), String> {
        for mode in 0..self.modes.len() {
            let directory = self.rung_directory(mode);
            if directory.exists() {
                fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    /// The best rung's deferred seed retries: rounds of fresh seeds while
    /// connections stay open and the ladder budget pays for them, keeping
    /// one attempt's worth for the narrow-signal step after (an attempt
    /// over half the refine budget used to get none: eurorack-pmod ended
    /// 4 open with 700 s of its budget left; three seeds took it to 2).
    pub(crate) fn seed_rounds<R: LadderRunner>(&mut self, runner: &R) -> Result<(), String> {
        let config = self.config;
        let output_directory = self.output_directory;
        let scratch = self.scratch.clone();
        let mut next_seed = 1u64;
        for _round in 0..3 {
            let Some((opens, best_result)) = self.best.as_ref() else {
                break;
            };
            let Some((attempt, connect)) = self.best_attempt.as_ref() else {
                break;
            };
            let opens = *opens;
            let connect = *connect;
            let attempt = attempt.clone();
            let (best_pitch, best_pours) = (best_result.grid_pitch_mm, best_result.pours.clone());
            let cost = self.result_work(best_result).max(1.0);
            // A round's seeds and its one check.
            let mut affordable =
                ((self.ladder_budget - self.spent(self.work) - cost - self.check_charge) / cost)
                    .floor()
                    .max(0.0) as usize;
            if self.reserve > 0.0 {
                affordable = affordable.min(
                    ((self.work_left() - self.check_charge - self.reserve) / cost)
                        .floor()
                        .max(0.0) as usize,
                );
            }
            if let Some(left) = self.rung_left() {
                affordable = affordable.min((left / cost).floor().max(0.0) as usize);
            }
            let retry = config.retry_seeds.unwrap_or(4).min(affordable);
            if opens.0 == 0
                || self.best_seeded
                || retry == 0
                || self.past_deadline()
                || self.no_time_for_another()
                || !self.bound_allows()
            {
                break;
            }
            // As many at once as fit the memory (`MemoryLimits`); how many
            // seeds route does not depend on it.
            let side_by_side = self.router_maps.map_or(1, |(maps, previous)| {
                let each = maps * (previous / best_pitch).powi(2);
                let (count, why) = Self::side_by_side_for(runner, each, retry);
                if count < retry {
                    eprintln!("ladder: {count} of {retry} seeds side by side ({why})");
                }
                count
            });
            let mut seeded = attempt;
            seeded.seeds = Some(retry);
            seeded.seeds_side_by_side = Some(side_by_side);
            seeded.first_seed = Some(next_seed);
            if self.reserve > 0.0 {
                seeded.router_deadline_work = Some(deadline_before_reserve(
                    self.work_left() - self.check_charge,
                    self.reserve,
                    retry,
                ));
            } else if let Some(left) = self.rung_left() {
                seeded.router_deadline_work = Some(self.router_work(rung_share(left, retry)));
            }
            next_seed += retry as u64;
            if scratch.exists() {
                fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
            }
            let quality = self.quality();
            let name = format!(
                "the best rung's seed round from seed {} ({retry} seeds)",
                next_seed - retry as u64
            );
            let panics = self.panic_due("seeds");
            let unchecked = self.unchecked(&seeded);
            let other = isolate(|| {
                if panics {
                    test_panic("seeds");
                }
                runner.route(&scratch, &unchecked, connect)
            });
            let Some(other) = self.survived(&name, other) else {
                if scratch.exists() {
                    fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
                }
                continue;
            };
            let mut other = other?;
            other.pours = best_pours;
            self.charge_result(&other, retry as f64);
            let Some(other_judged) =
                self.judge_isolated(runner, &name, &mut other, &scratch, quality)?
            else {
                if scratch.exists() {
                    fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
                }
                continue;
            };
            let other_opens = other_judged.unwrap_or((router_open(&other), 0));
            eprintln!(
                "best rung again with {retry} seeds: {} open{}, {} starved, {} vias, {:.1} s",
                other_opens.0,
                by_router(&other_judged),
                other_opens.1,
                other.vias,
                other.routing_seconds
            );
            let (best_vias, best_length) = self
                .best
                .as_ref()
                .map_or((0, 0.0), |(_, best)| (best.vias, best.length_mm));
            let better = other_judged.is_some()
                && (other_opens, other.vias, other.length_mm)
                    .partial_cmp(&(opens, best_vias, best_length))
                    .is_some_and(|order| order.is_lt());
            if better {
                fs::remove_dir_all(output_directory).map_err(|error| error.to_string())?;
                fs::rename(&scratch, output_directory).map_err(|error| error.to_string())?;
                self.best = Some((other_opens, other));
            } else {
                fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    /// The step's work left for its routing (a layout's ladder: its check
    /// comes on top).
    fn routing_left(&self) -> f64 {
        self.work_left() - self.check_charge
    }

    /// Whether a step has the work it needs. The work kept for the step
    /// is its own now. With it, the step needs half of it still there (the
    /// last rung may have run over its share), and at least 180 s (three
    /// times the negotiation's floor: jetson's ladder kept 129 s with 70
    /// left, and a step of 60 s negotiation took 232 s and left 268 open),
    /// and negotiates in a third of what is left, so that it ends in the
    /// work deadline whatever the machine's load; without it, the step
    /// needs an attempt's work as before (route mode: no work deadline).
    fn step_has_time(&self, kept: &StepWork) -> bool {
        if kept.kept > 0.0 {
            self.routing_left() >= narrow_step_minimum(kept.kept_routing)
        } else if self.work_bound.is_some() {
            self.bound_left().is_some_and(|left| {
                route_step_has_time(left, kept.route_kept, self.ladder_budget)
                    && first_iteration_fits(left, self.first_iteration)
            })
        } else {
            !self.no_time_for_another()
        }
    }

    /// A step's budget and work deadline.
    fn step_budget(&self, kept: &StepWork, step: &mut KiCadBoardRouterConfig) {
        if kept.kept > 0.0 {
            step.negotiation_seconds = Some(narrow_step_negotiation_floored(
                self.routing_left(),
                step.negotiation_seconds,
                self.first_negotiation,
                kept.kept_routing,
            ));
            step.run_work = Some((self.ladder_budget, self.routing_left().max(0.0)));
            // The attempt it varies may carry its pitch's work deadline,
            // taken when that pitch started: the step's is what is left now.
            step.router_deadline_work = Some(self.routing_left().max(0.0));
        } else if let Some(left) = self.bound_left() {
            // Route mode: within what is left of the ladder's bound, its
            // negotiation at most half of that.
            step.negotiation_seconds = Some(
                step.negotiation_seconds
                    .unwrap_or(900.0)
                    .min(self.router_work(0.5 * left)),
            );
            step.router_deadline_work = Some(self.router_work(left));
        }
    }

    /// Why a step for a board with connections open does not run, in the
    /// order its conditions are checked.
    fn blocked(&self, kept: &StepWork) -> Option<&'static str> {
        if self.spent(self.work) >= 1.5 * self.ladder_budget {
            Some("the ladder's budget was used (ladder_budget_seconds)")
        } else if self.past_deadline() {
            Some("the deadline was reached")
        } else if !self.step_has_time(kept) {
            Some(if kept.kept > 0.0 {
                "too little work was left of what was kept for it (half of that, at least 180 s)"
            } else if self.bound_left().is_some_and(|left| {
                route_step_has_time(left, kept.route_kept, self.ladder_budget)
                    && !first_iteration_fits(left, self.first_iteration)
            }) {
                "its first negotiation iteration would not fit the work left under the ladder's work bound (the ladder's first attempt took longer for its first)"
            } else if self.work_bound.is_some() {
                "too little work was left under the ladder's work bound (ladder_budget_seconds and its tolerance)"
            } else {
                "too little work was left for another attempt"
            })
        } else {
            None
        }
    }

    /// Whether the best board is still open (and a best attempt is known).
    fn open_best(&self) -> bool {
        self.best_attempt.is_some() && self.best.as_ref().is_some_and(|(opens, _)| opens.0 > 0)
    }

    /// The narrow-signal step, then the minimum-clearance step, each on the
    /// best attempt while connections stay open.
    pub(crate) fn steps<R: LadderRunner>(&mut self, runner: &R) -> Result<(), String> {
        let config = self.config;
        let rules = || {
            config
                .connection_rules
                .values()
                .chain(config.default_rules.iter())
        };
        let mut narrow_step_relaxed = false;
        let kept = self.reserve;
        self.reserve = 0.0;
        // Route mode: what was kept for the step is its own now.
        let route_kept = self.route_reserve;
        self.route_reserve = 0.0;
        // The step's own check is in what was kept: its routing has the rest
        // (and of what is left, what its check leaves).
        let kept = StepWork {
            kept,
            route_kept,
            kept_routing: (kept - self.check_charge).max(0.0),
        };
        if let Some(narrow) = config.narrow_signal_mm
            && self.open_best()
            && config.use_narrow_signals.is_none()
            && rules().any(|rules| rules.trace_width_mm > narrow + 1.0e-9)
            && let Some(why) = self.blocked(&kept)
        {
            eprintln!(
                "the narrow-signal step did not run: {why} ({:.1} s of work left)",
                self.bound_left().unwrap_or_else(|| self.work_left())
            );
            self.skipped.push(format!(
                "the narrow-signal step (signals at {narrow} mm) did not run: {why}"
            ));
        }
        // Connections still open at the class widths: the best attempt once
        // more with signal nets at the project's smallest predefined width
        // (designers often keep the default class at a power width and draw
        // signals narrower).
        if let Some(narrow) = config.narrow_signal_mm
            && self.open_best()
            && config.use_narrow_signals.is_none()
            && self.spent(self.work) < 1.5 * self.ladder_budget
            && !self.past_deadline()
            && self.step_has_time(&kept)
            && rules().any(|rules| rules.trace_width_mm > narrow + 1.0e-9)
        {
            let (attempt, connect) = self.best_attempt.clone().expect("open_best");
            let mut narrowed = attempt;
            narrowed.use_narrow_signals = Some(true);
            self.step_budget(&kept, &mut narrowed);
            self.note_shares(
                "the narrow-signal step",
                narrowed.router_deadline_work,
                narrowed.negotiation_seconds,
            );
            // A project that takes clearance findings as warnings gets its
            // minimum clearance in the same attempt (the A13 module's
            // designer drew 0.1016 tracks 0.149 from balls in classes of
            // 0.2).
            if let Some(relaxed) = config.relaxed_clearance_mm {
                narrowed.relaxed_clearance_mm = None;
                for rules in narrowed
                    .connection_rules
                    .values_mut()
                    .chain(narrowed.default_rules.iter_mut())
                {
                    rules.clearance_mm = rules.clearance_mm.min(relaxed);
                }
                narrow_step_relaxed = true;
            }
            let (charges_all, charges_spokes) = (self.charges_all, self.charges_spokes);
            let line = format!(
                "attempt with signals at {narrow} mm{}",
                config
                    .relaxed_clearance_mm
                    .map(|relaxed| format!(" and clearance {relaxed} mm"))
                    .unwrap_or_default()
            );
            self.step(
                runner,
                "the narrow-signal step",
                narrowed,
                connect,
                |opens, by, result| {
                    format!(
                        "{line}: {} open{by}, {} starved, {} vias, {:.1} s, work {:.0} s",
                        opens.0,
                        opens.1,
                        result.vias,
                        result.routing_seconds,
                        work_of(result.expansions)
                            + beyond_search(result, charges_all, charges_spokes)
                    )
                },
                true,
            )?;
        }
        // Still open, and the project takes clearance findings as warnings:
        // the best attempt once more at the board's minimum clearance,
        // which is what the designer held to.
        if let Some(relaxed) = config.relaxed_clearance_mm
            && self.open_best()
            && !narrow_step_relaxed
            && rules().any(|rules| rules.clearance_mm > relaxed + 1.0e-9)
            && let Some(why) = self.blocked(&kept)
        {
            self.skipped.push(format!(
                "the minimum-clearance step (clearance {relaxed} mm) did not run: {why}"
            ));
        }
        if let Some(relaxed) = config.relaxed_clearance_mm
            && self.open_best()
            && !narrow_step_relaxed
            && self.spent(self.work) < 1.5 * self.ladder_budget
            && !self.past_deadline()
            && self.step_has_time(&kept)
            && rules().any(|rules| rules.clearance_mm > relaxed + 1.0e-9)
        {
            let (attempt, connect) = self.best_attempt.clone().expect("open_best");
            let mut relaxed_attempt = attempt;
            self.step_budget(&kept, &mut relaxed_attempt);
            relaxed_attempt.relaxed_clearance_mm = None;
            for rules in relaxed_attempt
                .connection_rules
                .values_mut()
                .chain(relaxed_attempt.default_rules.iter_mut())
            {
                rules.clearance_mm = rules.clearance_mm.min(relaxed);
            }
            self.step(runner, "the minimum-clearance step", relaxed_attempt, connect, |opens, by, result| {
                format!(
                    "attempt at the minimum clearance {relaxed} mm: {} open{by}, {} starved, {} vias, {:.1} s",
                    opens.0,
                    opens.1,
                    result.vias,
                    result.routing_seconds
                )
            }, false)?;
        }
        Ok(())
    }

    /// One step: `attempt` routed in the scratch directory, kept when it
    /// beats the best board (`keeps_attempt`: it is the best attempt then).
    #[allow(clippy::too_many_arguments)]
    fn step<R: LadderRunner>(
        &mut self,
        runner: &R,
        name: &str,
        attempt: KiCadBoardRouterConfig,
        connect: bool,
        line: impl FnOnce(Opens, &str, &KiCadBoardRouterResult) -> String,
        keeps_attempt: bool,
    ) -> Result<(), String> {
        let scratch = self.scratch.clone();
        let output_directory = self.output_directory;
        if scratch.exists() {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
        let panics = self.panic_due("step");
        let unchecked = self.unchecked(&attempt);
        let routed = isolate(|| {
            if panics {
                test_panic("step");
            }
            runner.route(&scratch, &unchecked, connect)
        });
        let Some(routed) = self.survived(name, routed) else {
            if scratch.exists() {
                fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
            }
            return Ok(());
        };
        let mut result = routed?;
        self.charge_result(&result, 1.0);
        result.pours = self
            .best
            .as_ref()
            .map(|(_, best)| best.pours.clone())
            .unwrap_or_default();
        let quality = self.quality();
        let Some(judged) = self.judge_isolated(runner, name, &mut result, &scratch, quality)?
        else {
            if scratch.exists() {
                fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
            }
            return Ok(());
        };
        let opens = judged.unwrap_or((router_open(&result), 0));
        eprintln!("{}", line(opens, by_router(&judged), &result));
        let better = judged.is_some()
            && self.best.as_ref().is_some_and(|(best_opens, best_result)| {
                (opens, result.vias, result.length_mm)
                    .partial_cmp(&(*best_opens, best_result.vias, best_result.length_mm))
                    .is_some_and(|order| order.is_lt())
            });
        if better {
            fs::remove_dir_all(output_directory).map_err(|error| error.to_string())?;
            fs::rename(&scratch, output_directory).map_err(|error| error.to_string())?;
            self.best = Some((opens, result));
            if keeps_attempt {
                self.best_attempt = Some((attempt, connect));
            }
        } else {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "ladder_tests.rs"]
mod tests;

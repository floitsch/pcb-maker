// Copyright (C) 2026 Toit contributors.

//! The attempt ladder's transitions with a scripted runner (`Fake`): the
//! probes and their leader, the rungs and their seed retry, the finer
//! pitches, the seed rounds, the narrow-signal step, with the reserve and
//! the work bound, and the fault isolation of every attempt.

use super::*;
use std::sync::Mutex;

/// What the scripted runner was asked to do, in order.
#[derive(Clone, Debug, PartialEq)]
enum Call {
    Probe {
        mode: usize,
        work: f64,
    },
    Resume {
        negotiation: f64,
        deadline_work: Option<f64>,
    },
    Route(Routed),
}

/// One routed attempt as the runner saw it.
#[derive(Clone, Debug, PartialEq)]
struct Routed {
    rung: &'static str,
    /// None: the regular lattice.
    pitch: Option<f64>,
    seeds: usize,
    first_seed: u64,
    narrow: bool,
    deadline_work: Option<f64>,
    negotiation: Option<f64>,
}

/// An attempt's open connections and work in seconds (at most its work
/// deadline, as a router stops there).
#[derive(Clone, Copy, Debug)]
struct Outcome {
    open: usize,
    work: f64,
}

/// A probing router of the scripted runner.
struct FakeProbe {
    mode: usize,
    /// Its static maps' key (`statics_key_of`).
    key: u128,
    expansions: u64,
    pour: f64,
    /// Its limit counts its pour work (`limits_count_all`).
    counts_all: bool,
}

/// A scripted attempt's outcome by what it routes (None: it panics).
type Script = Box<dyn Fn(&Routed) -> Option<Outcome> + Sync>;

/// The scripted runner: a board of `layers` and `connections`, the probes'
/// unfinished nets by rung, the continued probe's outcome and every other
/// attempt's (None: the attempt panics, as a router bug would).
struct Fake {
    layers: usize,
    connections: usize,
    probes: Vec<usize>,
    /// A probe whose router panics (the runner's own, not the test hook).
    panicking_probe: Option<usize>,
    continued: Outcome,
    route: Script,
    calls: Mutex<Vec<Call>>,
    /// Every routed attempt's manual spokes' work and the work of its
    /// first negotiation iteration (its report's).
    spokes: f64,
    first_iteration: f64,
    /// Every routed attempt's pour work (a probe's: `probe_pour`), every
    /// router's build and every lowering's work.
    pour: f64,
    probe_pour: f64,
    /// Per rung, a probe's pour work instead of `probe_pour` (empty: none).
    probe_pours: Vec<f64>,
    build: f64,
    lowering: f64,
    /// The part of `build` routers of the same static maps share
    /// (`SharedBuild::statics`, keyed by `statics_key_of`).
    shared_statics: f64,
    /// The memory limits as the runner measures them (how many routers
    /// run side by side).
    limits: MemoryLimits,
}

/// The scripted runner's static maps' key of a rung on a lattice: the
/// exclusive planes act on the router, not on the maps (the connect
/// rung's), every other rung and pitch has its own.
fn statics_key_of(attempt: &KiCadBoardRouterConfig, connect: bool) -> u128 {
    let rung = match rung_of(attempt, connect) {
        "exclusive" => "connect",
        rung => rung,
    };
    let pitch = attempt
        .grid_pitches_mm
        .as_ref()
        .map_or(0, |pitches| (pitches[0] * 1.0e4).round() as u128);
    let index = ["connect", "skeleton", "stubs", "tracks"]
        .iter()
        .position(|other| *other == rung)
        .unwrap_or(9) as u128;
    1 + index * 100_000 + pitch
}

impl Fake {
    fn new(route: impl Fn(&Routed) -> Option<Outcome> + Sync + 'static) -> Self {
        Self {
            layers: 2,
            connections: 50,
            probes: Vec::new(),
            panicking_probe: None,
            continued: Outcome { open: 0, work: 1.0 },
            route: Box::new(route),
            calls: Mutex::new(Vec::new()),
            spokes: 0.0,
            first_iteration: 0.0,
            pour: 0.0,
            probe_pour: 0.0,
            probe_pours: Vec::new(),
            build: 0.0,
            lowering: 0.0,
            shared_statics: 0.0,
            limits: machine(),
        }
    }

    /// A routed or probing router's shared parts (`RouterWork::shared`).
    fn shared(&self, key: u128) -> core::router::SharedBuild {
        core::router::SharedBuild {
            statics: (self.shared_statics > 0.0).then_some((key, self.shared_statics)),
            masks: None,
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn routes(&self) -> Vec<Routed> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Route(routed) => Some(routed),
                _ => None,
            })
            .collect()
    }
}

/// The rung an attempt's configuration names.
fn rung_of(attempt: &KiCadBoardRouterConfig, connect: bool) -> &'static str {
    if !connect {
        "tracks"
    } else if attempt.exclusive_planes == Some(true) {
        "exclusive"
    } else if attempt.fixed_plane_stubs == Some(true) {
        "stubs"
    } else if attempt.plane_skeleton == Some(true) {
        "skeleton"
    } else {
        "connect"
    }
}

/// A routed board's report with `open` connections and `work` seconds of
/// work on a lattice of `pitch` mm.
fn report(open: usize, work: f64, pitch: f64) -> KiCadBoardRouterResult {
    KiCadBoardRouterResult {
        board_id: "t".into(),
        routable_connections: 50,
        routed_connections: 50 - open.min(50),
        unconnected_terminals: open,
        vias: 10 + open,
        length_mm: 100.0,
        grid_pitch_mm: pitch,
        grid_nodes: [100, 100],
        iterations: 1,
        searches: 1,
        expansions: (work * core::router::expansions_per_second()).round() as u64,
        lowering_seconds: 0.0,
        routing_seconds: work,
        internal_verification_seconds: 0.0,
        native_verification_seconds: 0.0,
        internal_violations: Vec::new(),
        native: None,
        pours: String::new(),
        nets: Vec::new(),
        congestion: Vec::new(),
        grid_origin: [0.0, 0.0],
        pin_swaps: None,
        diagnostics: KiCadRoutingDiagnostics::default(),
        cut: Vec::new(),
        work_seconds: 0.0,
        kicad_check_work_seconds: 0.0,
        kicad_checks_charged: 0,
        skipped: Vec::new(),
        neck_repair: None,
        rules: KiCadRulesSummary::default(),
        maps_mb: 100.0,
        work_bound_seconds: None,
        memory_limits: String::new(),
        attempts_failed: Vec::new(),
        island_stubs: 0,
        manual_spokes_work_seconds: 0.0,
        first_iteration_expansions: 0,
        pour_work_seconds: 0.0,
        build_work_seconds: 0.0,
        shared_build: Default::default(),
        lowering_work_seconds: 0.0,
        wall_accounting: None,
        kept_copper: None,
        diff_pairs: None,
        warnings: Vec::new(),
        pour_quality: Vec::new(),
        pour_loss: None,
        negotiation_stop: String::new(),
        attempts: Vec::new(),
    }
}

/// Writes an attempt's directory as a router would.
fn write_board(directory: &Path, what: &str) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    fs::write(directory.join("board"), what).map_err(|error| error.to_string())
}

impl LadderRunner for Fake {
    /// The board's static maps' key.
    type Board = u128;
    type Probe = FakeProbe;
    type Routed = Outcome;
    type Hold = u128;

    fn lower(&self, attempt: &KiCadBoardRouterConfig, connect: bool) -> Result<u128, String> {
        Ok(statics_key_of(attempt, connect))
    }

    fn size(&self, _: &u128) -> (usize, usize) {
        (self.layers, self.connections)
    }

    fn build(&self, board: &u128, attempt: &KiCadBoardRouterConfig, _: bool) -> FakeProbe {
        FakeProbe {
            mode: usize::MAX,
            key: *board,
            expansions: 0,
            pour: 0.0,
            counts_all: attempt.limits_count_all,
        }
    }

    fn hold(&self, board: &u128, _: &KiCadBoardRouterConfig) -> u128 {
        *board
    }

    fn hold_key(&self, hold: &u128) -> Option<u128> {
        Some(*hold)
    }

    fn statics_key(&self, board: &u128, _: &KiCadBoardRouterConfig) -> Option<u128> {
        Some(*board)
    }

    fn maps_mb(&self, _: &FakeProbe) -> Option<f64> {
        Some(100.0)
    }

    fn statics_mb(&self, _: &FakeProbe) -> Option<f64> {
        Some(40.0)
    }

    fn probe(&self, probe: &mut FakeProbe, work: f64, mode: usize) -> usize {
        self.calls.lock().unwrap().push(Call::Probe { mode, work });
        if self.panicking_probe == Some(mode) {
            panic!("index out of bounds in the probe of rung {mode}");
        }
        probe.mode = mode;
        // A limit of all its work: its pour work comes out of it.
        let pour = self
            .probe_pours
            .get(mode)
            .copied()
            .unwrap_or(self.probe_pour);
        let search = if probe.counts_all { work - pour } else { work };
        probe.expansions = (search * core::router::expansions_per_second()).round() as u64;
        probe.pour = pour;
        self.probes[mode]
    }

    fn work(&self, probe: &FakeProbe) -> RouterWork {
        RouterWork {
            expansions: probe.expansions,
            pour: probe.pour,
            build: self.build,
            shared: self.shared(probe.key),
        }
    }

    fn lowering_work(&self, _: &u128) -> f64 {
        self.lowering
    }

    fn speak(&self, _: &mut FakeProbe, _: &KiCadBoardRouterConfig) {}

    fn resume(&self, probe: FakeProbe, limits: ResumeLimits) -> (Outcome, RouterWork) {
        self.calls.lock().unwrap().push(Call::Resume {
            negotiation: limits.negotiation,
            deadline_work: limits.deadline_work,
        });
        let probed = probe.expansions as f64 / core::router::expansions_per_second();
        let work = self.continued.work.min(
            limits
                .deadline_work
                .map_or(f64::INFINITY, |end| end - probed),
        );
        let expansions = probe.expansions + (work * core::router::expansions_per_second()) as u64;
        (
            self.continued,
            RouterWork {
                expansions,
                pour: probe.pour + self.pour,
                build: self.build,
                shared: self.shared(probe.key),
            },
        )
    }

    fn write(
        &self,
        _: &u128,
        routed: Outcome,
        _: &KiCadBoardRouterConfig,
        directory: &Path,
        _: f64,
    ) -> Result<KiCadBoardRouterResult, String> {
        write_board(directory, "continued")?;
        let mut result = report(routed.open, routed.work, 0.1);
        result.pour_work_seconds = self.probe_pour + self.pour;
        result.build_work_seconds = self.build;
        Ok(result)
    }

    fn route_lowered(
        &self,
        directory: &Path,
        attempt: &KiCadBoardRouterConfig,
        connect: bool,
        board: u128,
    ) -> Result<KiCadBoardRouterResult, String> {
        assert_eq!(board, statics_key_of(attempt, connect));
        self.route(directory, attempt, connect)
    }

    fn route(
        &self,
        directory: &Path,
        attempt: &KiCadBoardRouterConfig,
        connect: bool,
    ) -> Result<KiCadBoardRouterResult, String> {
        let routed = Routed {
            rung: rung_of(attempt, connect),
            pitch: attempt.grid_pitches_mm.as_ref().map(|pitches| pitches[0]),
            seeds: attempt.seeds.unwrap_or(1),
            first_seed: attempt.first_seed.unwrap_or(0),
            narrow: attempt.use_narrow_signals == Some(true),
            deadline_work: attempt.router_deadline_work,
            negotiation: attempt.negotiation_seconds,
        };
        self.calls.lock().unwrap().push(Call::Route(routed.clone()));
        let Some(outcome) = (self.route)(&routed) else {
            panic!("a router assertion in rung {}", routed.rung);
        };
        let work = outcome
            .work
            .min(routed.deadline_work.unwrap_or(f64::INFINITY));
        write_board(directory, routed.rung)?;
        let mut result = report(outcome.open, work, routed.pitch.unwrap_or(0.1));
        result.manual_spokes_work_seconds = self.spokes;
        result.pour_work_seconds = self.pour + self.spokes;
        result.build_work_seconds = self.build;
        result.shared_build = self.shared(statics_key_of(attempt, connect));
        result.lowering_work_seconds = self.lowering;
        result.first_iteration_expansions =
            (self.first_iteration.min(work) * core::router::expansions_per_second()).round() as u64;
        Ok(result)
    }

    fn check(&self, _: &mut KiCadBoardRouterResult, _: &Path) -> Result<(), String> {
        Ok(())
    }

    fn starved(&self, _: &Path) -> usize {
        0
    }

    fn memory_limits(&self) -> MemoryLimits {
        self.limits.clone()
    }
}

/// A machine of 64 GB with 32 GB available and no limit of the process's
/// own.
fn machine() -> MemoryLimits {
    MemoryLimits::resolve(&MemoryInputs {
        virtual_mb: 400.0,
        resident_mb: 100.0,
        total_mb: Some(64000.0),
        available_mb: Some(32000.0),
        ..MemoryInputs::default()
    })
}

/// Route mode on a budget of `budget` s, without KiCad's check.
fn route_mode(budget: f64) -> KiCadBoardRouterConfig {
    KiCadBoardRouterConfig {
        skip_native_verification: true,
        ladder_budget_seconds: Some(budget),
        ..KiCadBoardRouterConfig::default()
    }
}

/// `config` with signals narrower than the classes (the narrow-signal
/// step applies).
fn with_narrow_step(mut config: KiCadBoardRouterConfig) -> KiCadBoardRouterConfig {
    config.default_rules = Some(KiCadConnectionRoutingRules {
        trace_width_mm: 0.25,
        clearance_mm: 0.2,
        via_size_mm: 0.6,
        via_drill_mm: 0.3,
    });
    config.narrow_signal_mm = Some(0.15);
    config
}

/// A scratch output directory of the test `name`.
fn output(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("pcb-maker-ladder-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.join("out")
}

/// The ladder of a board with pours on `layers` layers.
fn ladder<'a>(config: &'a KiCadBoardRouterConfig, output: &'a Path, layers: usize) -> Ladder<'a> {
    Ladder::new(
        config,
        output,
        vec!["GND".into()],
        layers,
        machine(),
        Some(64000.0),
    )
}

/// What the ladder kept: its open count and the attempt's directory.
fn kept(ladder: &Ladder, output: &Path) -> (usize, String) {
    (
        ladder.best.as_ref().expect("a board").0.0,
        fs::read_to_string(output.join("board")).unwrap(),
    )
}

#[test]
fn the_rungs_follow_the_board_layers_and_pour_mode() {
    let auto = KiCadBoardRouterConfig::default();
    assert_eq!(
        ladder_rungs(&auto, false, false),
        vec![(false, false, false, false)]
    );
    assert_eq!(
        ladder_rungs(&auto, true, true),
        vec![
            (true, false, false, false),
            (true, true, false, false),
            (false, false, false, false)
        ]
    );
    assert_eq!(
        ladder_rungs(&auto, true, false),
        vec![
            (true, false, true, false),
            (true, false, false, false),
            (true, false, false, true),
            (false, false, false, false)
        ]
    );
    let tracks = KiCadBoardRouterConfig {
        pours: KiCadPourMode::Tracks,
        ..KiCadBoardRouterConfig::default()
    };
    assert_eq!(ladder_rungs(&tracks, true, false).len(), 1);
    // The default lattices are 0.127 to 0.1 mm: both finer pitches.
    assert_eq!(
        ladder_pitches(&auto),
        vec![None, Some(vec![0.075]), Some(vec![0.05])]
    );
}

#[test]
fn the_test_hook_names_a_kind_and_which_attempt_of_it() {
    assert_eq!(
        TestPanic::parse("probe:2"),
        Some(TestPanic {
            kind: "probe".into(),
            nth: 2
        })
    );
    assert_eq!(TestPanic::parse("rung").map(|hook| hook.nth), Some(1));
    assert_eq!(TestPanic::parse("rung:all").map(|hook| hook.nth), Some(0));
    assert_eq!(TestPanic::parse("rung:0"), None);
    assert_eq!(TestPanic::parse(""), None);
    assert!(
        isolate(|| panic!("boom {}", 1))
            .unwrap_err()
            .contains("boom 1")
    );
    assert_eq!(isolate(|| 7), Ok(7));
}

/// A four-layer board with enough connections is probed: every rung
/// negotiates briefly, the one with the fewest unfinished nets continues,
/// and a complete continuation ends the ladder (no rung, no seeds, no
/// step).
#[test]
fn the_probes_lead_and_a_complete_continuation_finishes() {
    let config = with_narrow_step(route_mode(100.0));
    let out = output("probes");
    let mut fake = Fake::new(|_| Some(Outcome { open: 5, work: 5.0 }));
    fake.layers = 4;
    fake.connections = 200;
    fake.probes = vec![30, 10, 20, 40];
    fake.continued = Outcome {
        open: 0,
        work: 20.0,
    };
    let mut ladder = ladder(&config, &out, 4);
    ladder.run(&fake).unwrap();
    let calls = fake.calls();
    // The probes share half the budget: 12.5 s each over four rungs.
    let mut probes: Vec<usize> = calls
        .iter()
        .filter_map(|call| match call {
            Call::Probe { mode, work } => {
                assert_eq!(*work, 12.5);
                Some(*mode)
            }
            _ => None,
        })
        .collect();
    // Side by side: in any order.
    probes.sort();
    assert_eq!(probes, vec![0, 1, 2, 3]);
    let Call::Resume {
        negotiation,
        deadline_work,
    } = calls[4].clone()
    else {
        panic!("the leader continues: {calls:?}");
    };
    // A third of what is left under the bound (105 - 50) at least 60 s,
    // but at most half of it, and the router stops at the bound (its
    // probe's work counted in it).
    assert!((negotiation - 27.5).abs() < 1e-6, "{negotiation}");
    assert!((deadline_work.unwrap() - (12.5 + 55.0)).abs() < 1e-6);
    assert_eq!(calls.len(), 5, "{calls:?}");
    assert_eq!(kept(&ladder, &out), (0, "continued".into()));
    assert!((ladder.work - 70.0).abs() < 1e-6, "{}", ladder.work);
    assert!(ladder.failed.is_empty());
}

/// A panic in one probe is recorded and the ladder goes on: the next best
/// probe leads, its continuation is the board.
#[test]
fn a_panic_in_one_probe_is_recorded_and_the_next_leads() {
    let config = route_mode(100.0);
    for hooked in [true, false] {
        let out = output(if hooked { "probe-hook" } else { "probe-router" });
        let mut fake = Fake::new(|_| Some(Outcome { open: 5, work: 5.0 }));
        fake.layers = 4;
        fake.connections = 200;
        fake.probes = vec![30, 10, 20, 40];
        fake.continued = Outcome {
            open: 2,
            work: 20.0,
        };
        let mut ladder = ladder(&config, &out, 4);
        if hooked {
            // The second probe, rung 1's (the leader without the panic).
            ladder.panic_at = TestPanic::parse("probe:2");
        } else {
            fake.panicking_probe = Some(1);
        }
        ladder.run(&fake).unwrap();
        assert_eq!(ladder.failed.len(), 1, "{:?}", ladder.failed);
        assert!(
            ladder.failed[0].starts_with("the probe of rung pours=connect: panic: "),
            "{:?}",
            ladder.failed
        );
        assert!(
            ladder.failed[0].contains(if hooked {
                TEST_PANIC_ATTEMPT
            } else {
                "index out of bounds"
            }),
            "{:?}",
            ladder.failed
        );
        // Rung 2 (20 unfinished) led and continued; the regular rungs
        // other than it ran after (it left 2 open).
        let routes = fake.routes();
        assert!(
            routes
                .iter()
                .all(|routed| routed.rung != "stubs" || routed.pitch.is_some())
        );
        assert!(ladder.best.is_some());
    }
}

/// The continuation's panic leaves no board from the probes: the regular
/// rungs all run, the probed one too.
#[test]
fn a_panic_in_the_continuation_falls_back_to_the_rungs() {
    let config = route_mode(100.0);
    let out = output("continuation");
    let mut fake = Fake::new(|routed| {
        Some(Outcome {
            open: if routed.rung == "tracks" { 0 } else { 3 },
            work: 4.0,
        })
    });
    fake.layers = 4;
    fake.connections = 200;
    fake.probes = vec![30, 10, 20, 40];
    let mut ladder = ladder(&config, &out, 4);
    ladder.panic_at = TestPanic::parse("continuation");
    ladder.run(&fake).unwrap();
    assert_eq!(ladder.failed.len(), 1);
    assert!(ladder.failed[0].starts_with("the continuation of the probed rung pours=connect"));
    let mut rungs: Vec<&str> = fake.routes().iter().map(|routed| routed.rung).collect();
    // The first alone, the others side by side in any order.
    rungs[1..].sort();
    // The rungs after the first run side by side; the plane stubs' is
    // then not judged (they only count when a pour net's pads stayed
    // open, none here: the report lists no nets).
    assert_eq!(rungs, vec!["exclusive", "connect", "stubs", "tracks"]);
    assert_eq!(kept(&ladder, &out), (0, "tracks".into()));
}

/// The regular ladder in route mode: the first rung alone with all that
/// is left, its work kept for the narrow-signal step once it leaves
/// connections open, the other rungs with an even share, the last rung's
/// seed retry, the finer pitches while their projection fits, the step
/// last with what was kept; the work stays within the bound.
#[test]
fn route_mode_rungs_seeds_and_the_narrow_step_stay_within_the_bound() {
    let config = with_narrow_step(route_mode(100.0));
    let out = output("bound");
    let fake = Fake::new(|routed| {
        Some(if routed.narrow {
            Outcome { open: 1, work: 6.0 }
        } else if routed.seeds > 1 {
            Outcome { open: 4, work: 3.0 }
        } else {
            Outcome {
                open: if routed.rung == "tracks" { 6 } else { 5 },
                work: 10.0,
            }
        })
    });
    let mut ladder = ladder(&config, &out, 2);
    ladder.run(&fake).unwrap();
    let routes = fake.routes();
    let bound = ladder_work_bound(100.0);
    // The first rung has all that is left under the bound.
    assert_eq!(routes[0].rung, "connect");
    assert_eq!(routes[0].pitch, None);
    assert_eq!(routes[0].deadline_work, Some(bound));
    // Its work (10 s) is kept for the step: the next rung (the skeleton
    // is skipped, no pour pad was open) shares the rest.
    assert_eq!(routes[1].rung, "tracks");
    assert!((routes[1].deadline_work.unwrap() - (bound - 10.0 - 10.0)).abs() < 1e-6);
    // The last rung's seed retry, four seeds sharing what is left.
    assert_eq!((routes[2].seeds, routes[2].first_seed), (4, 1));
    // The step is the last attempt, with what was kept.
    let step = routes.last().unwrap();
    assert!(step.narrow, "{routes:?}");
    assert!(
        routes[..routes.len() - 1]
            .iter()
            .all(|routed| !routed.narrow)
    );
    assert!(step.deadline_work.unwrap() >= 10.0 - 1e-9);
    // Every router had a work deadline, and the ladder's work stays
    // within the bound.
    assert!(routes.iter().all(|routed| routed.deadline_work.is_some()));
    assert!(ladder.work <= bound + 1e-6, "{}", ladder.work);
    // The tracks rung's seeds (4 open) beat the connect rung (5): the
    // step varies that attempt.
    assert_eq!(kept(&ladder, &out), (1, "tracks".into()));
    assert!(ladder.failed.is_empty());
}

/// Route mode: an attempt's manual spokes count in the ladder's work
/// beside its search, and the rungs after it share what is left of that.
#[test]
fn route_mode_counts_the_manual_spokes() {
    let config = route_mode(100.0);
    let out = output("spokes");
    let mut fake = Fake::new(|routed| {
        Some(Outcome {
            open: if routed.rung == "tracks" { 6 } else { 5 },
            work: 10.0,
        })
    });
    fake.spokes = 4.0;
    let mut ladder = ladder(&config, &out, 2);
    ladder.run(&fake).unwrap();
    let routes = fake.routes();
    let bound = ladder_work_bound(100.0);
    assert_eq!(routes[0].deadline_work, Some(bound));
    // The first attempt counted 10 s of search and 4 s of spokes (pour
    // work, all of it charged); the next rung's share of what is left is
    // search work in the proportion of the first attempt's, 10 of 14.
    assert_eq!(routes[1].rung, "tracks");
    assert!(
        (routes[1].deadline_work.unwrap() - (bound - 14.0) * 10.0 / 14.0).abs() < 1e-6,
        "{routes:?}"
    );
    assert!(ladder.work <= bound + 1e-6, "{}", ladder.work);
}

/// Route mode: every kind of an attempt's work counts in the ladder's
/// (its search, its pour analyses, its lattice's build, its board's
/// lowering), and a share of what is left is a router's limit in search
/// work by the share of search in the routers' work so far.
#[test]
fn route_mode_charges_all_the_work() {
    let config = route_mode(100.0);
    let out = output("charges");
    let mut fake = Fake::new(|routed| {
        Some(Outcome {
            open: if routed.rung == "tracks" { 6 } else { 5 },
            work: 10.0,
        })
    });
    fake.pour = 3.0;
    fake.build = 1.0;
    fake.lowering = 0.5;
    let mut ladder = ladder(&config, &out, 2);
    ladder.run(&fake).unwrap();
    let routes = fake.routes();
    let bound = ladder_work_bound(100.0);
    // The first attempt has all of the bound but the first rung's
    // lowering for the probes (which a two-layer board does not get):
    // nothing is known of its proportions yet.
    assert_eq!(routes[0].deadline_work, Some(bound - 0.5));
    // It counted 10 + 3 + 1 + 0.5 s; of its routers' 14 s (the lowerings
    // apart) 10 were search.
    assert_eq!(routes[1].rung, "tracks");
    assert!(
        (routes[1].deadline_work.unwrap() - (bound - 15.0) * 10.0 / 14.0).abs() < 1e-6,
        "{routes:?}"
    );
    let charged = ladder.charged;
    let attempts = routes.iter().map(|routed| routed.seeds as f64).sum::<f64>();
    assert!(
        (charged.pour_seconds - 3.0 * attempts).abs() < 1e-6,
        "{charged:?}"
    );
    assert!(
        (charged.lattice_seconds - attempts).abs() < 1e-6,
        "{charged:?}"
    );
    // One lowering an attempt (seeds share theirs) and the probes' first.
    assert!(
        (charged.lowering_seconds - 0.5 * (routes.len() as f64 + 1.0)).abs() < 1e-6,
        "{charged:?}"
    );
    assert_eq!(charged.kicad_check_seconds, 0.0);
    let sum = charged.search_seconds
        + charged.pour_seconds
        + charged.lattice_seconds
        + charged.lowering_seconds;
    assert!(
        (ladder.work - sum).abs() < 1e-6,
        "{} {charged:?}",
        ladder.work
    );
}

/// Route mode: the probes count their pour analyses, their lattices and
/// the rungs' lowerings, and the leader's continuation stops where the
/// bound ends in search work by the probes' proportion.
#[test]
fn route_mode_probes_charge_their_pour_work_and_lattices() {
    let config = with_narrow_step(route_mode(100.0));
    let out = output("probe-charges");
    let mut fake = Fake::new(|_| Some(Outcome { open: 5, work: 5.0 }));
    fake.layers = 4;
    fake.connections = 200;
    fake.probes = vec![30, 10, 20, 40];
    fake.probe_pour = 2.5;
    fake.build = 0.5;
    fake.lowering = 0.25;
    fake.pour = 1.0;
    fake.continued = Outcome {
        open: 0,
        work: 20.0,
    };
    let mut ladder = ladder(&config, &out, 4);
    ladder.run(&fake).unwrap();
    let calls = fake.calls();
    let Call::Resume {
        negotiation,
        deadline_work,
    } = calls[4].clone()
    else {
        panic!("the leader continues: {calls:?}");
    };
    // Four lowerings, four probes of 12.5 s of search (their limits are
    // search work: the ranking compares equal search), 2.5 s of pour work
    // and a lattice of 0.5 s each: 1 + 50 + 10 + 2 = 63 s.
    assert!(calls[..4].iter().all(|call| matches!(
        call,
        Call::Probe { work, .. } if (work - 12.5).abs() < 1e-6
    )));
    // The continuation builds no lattice: of its own probe's search and
    // pour work, 12.5 of 15 s were search.
    let left = 105.0 - 63.0;
    let share = 12.5 / 15.0;
    assert!(
        (deadline_work.unwrap() - (12.5 + left * share)).abs() < 1e-6,
        "{deadline_work:?}"
    );
    assert!(
        (negotiation - 0.5 * left * share).abs() < 1e-6,
        "{negotiation}"
    );
    // The continuation counts its own search and pour work (the probe's
    // were counted with the probes, its lattice too).
    let continued = (20.0f64).min(left * share);
    assert!(
        (ladder.work - (63.0 + continued + 1.0)).abs() < 1e-6,
        "{} {:?}",
        ladder.work,
        ladder.charged
    );
    assert_eq!(kept(&ladder, &out), (0, "continued".into()));
}

/// Route mode: the probed rung's continuation takes its share in search
/// work by its own probe's proportion of search and pour work: a leader
/// routing its pours as tracks did none, whatever the connected rungs'
/// probes beside it did (Vortac: a third of their work).
#[test]
fn route_mode_continuation_goes_by_its_own_probe() {
    let config = with_narrow_step(route_mode(100.0));
    let out = output("continuation-own");
    let mut fake = Fake::new(|_| Some(Outcome { open: 5, work: 5.0 }));
    fake.layers = 4;
    fake.connections = 200;
    // The tracks rung (the last) leads.
    fake.probes = vec![30, 20, 40, 10];
    fake.probe_pours = vec![6.0, 6.0, 6.0, 0.0];
    fake.build = 0.5;
    fake.continued = Outcome {
        open: 0,
        work: 20.0,
    };
    let mut ladder = ladder(&config, &out, 4);
    ladder.run(&fake).unwrap();
    let calls = fake.calls();
    let Call::Resume { deadline_work, .. } = calls[4].clone() else {
        panic!("the leader continues: {calls:?}");
    };
    // Four probes of 12.5 s of search, 18 s of pour work, four lattices:
    // 50 + 18 + 2 = 70 s; the leader did no pour work: its share is
    // search work as it is.
    let left = 105.0 - 70.0;
    assert!(
        (deadline_work.unwrap() - (12.5 + left)).abs() < 1e-6,
        "{deadline_work:?}"
    );
}

/// Route mode: no step starts with less left under the bound than the
/// first attempt's first negotiation iteration took, which a fresh router
/// does whatever its work deadline.
#[test]
fn route_mode_steps_need_room_for_a_first_iteration() {
    let config = with_narrow_step(route_mode(30.0));
    let out = output("first-iteration");
    let mut fake = Fake::new(|routed| {
        Some(if routed.narrow {
            Outcome { open: 1, work: 3.0 }
        } else {
            Outcome {
                open: 5,
                work: 24.0,
            }
        })
    });
    fake.first_iteration = 17.0;
    let mut ladder = ladder(&config, &out, 2);
    ladder.run(&fake).unwrap();
    let routes = fake.routes();
    assert!(routes.iter().all(|routed| !routed.narrow), "{routes:?}");
    assert!(
        ladder
            .skipped
            .iter()
            .any(|why| why.contains("first negotiation iteration")),
        "{:?}",
        ladder.skipped
    );
    // With a first iteration that fits, the step runs.
    let out = output("first-iteration-fits");
    let mut fake = Fake::new(|routed| {
        Some(if routed.narrow {
            Outcome { open: 1, work: 3.0 }
        } else {
            Outcome {
                open: 5,
                work: 24.0,
            }
        })
    });
    fake.first_iteration = 2.0;
    let mut fitting = super::ladder::Ladder::new(
        &config,
        &out,
        vec!["GND".into()],
        2,
        machine(),
        Some(64000.0),
    );
    fitting.run(&fake).unwrap();
    assert!(fake.routes().iter().any(|routed| routed.narrow));
}

/// Route mode: a step needs a fifth of the ladder's first attempt's work
/// left under the bound (`step_cost_fits`), whatever its first iteration
/// takes: with a second left of 28 s, one iteration of a fresh router
/// leaves hundreds of nets in conflict (iorodeo, ducktop2, tuxkeychain).
#[test]
fn route_mode_steps_need_a_fifth_of_the_first_attempt() {
    let config = with_narrow_step(route_mode(30.0));
    let run = |first: f64, name: &str| {
        let out = output(name);
        let mut fake = Fake::new(move |routed| {
            Some(if routed.narrow {
                Outcome { open: 1, work: 3.0 }
            } else {
                Outcome {
                    open: 5,
                    work: first,
                }
            })
        });
        fake.first_iteration = 0.5;
        let mut ladder = ladder(&config, &out, 2);
        ladder.run(&fake).unwrap();
        let narrow = fake.routes().iter().any(|routed| routed.narrow);
        (narrow, ladder.skipped.clone())
    };
    // 31.5 - 28 = 3.5 s left, under a fifth of 28 s.
    let (narrow, skipped) = run(28.0, "step-cost-short");
    assert!(!narrow, "{skipped:?}");
    assert!(
        skipped.iter().any(|why| why.contains("a fifth")),
        "{skipped:?}"
    );
    // 31.5 - 24 = 7.5 s left, over a fifth of 24 s.
    let (narrow, skipped) = run(24.0, "step-cost-fits");
    assert!(narrow, "{skipped:?}");
}

/// A panic in a rung that runs beside others loses that rung only: the
/// board the ladder held stays, the other rungs are judged.
#[test]
fn a_panic_in_a_rung_keeps_the_best_board() {
    let config = route_mode(100.0);
    let out = output("rung");
    let fake = Fake::new(|routed| match routed.rung {
        "tracks" => None,
        _ => Some(Outcome {
            open: if routed.pitch.is_some() { 1 } else { 3 },
            work: 5.0,
        }),
    });
    let mut ladder = ladder(&config, &out, 2);
    ladder.run(&fake).unwrap();
    assert!(!ladder.failed.is_empty());
    assert!(
        ladder
            .failed
            .iter()
            .all(|failure| failure.starts_with("the rung pours=tracks")
                && failure.contains("a router assertion in rung tracks")),
        "{:?}",
        ladder.failed
    );
    // The finer pitch's connect rung is the best: 1 open.
    assert_eq!(kept(&ladder, &out), (1, "connect".into()));
    // No rung directory stays behind.
    for mode in 0..3 {
        assert!(!out.with_extension(format!("rung{mode}")).exists());
    }
}

/// Every attempt failing leaves no board: route_kicad_board then ends
/// with an error (exit 4) that lists the failures.
#[test]
fn every_attempt_failing_leaves_no_board() {
    let config = route_mode(30.0);
    let out = output("all");
    let fake = Fake::new(|_| None);
    let mut ladder = ladder(&config, &out, 2);
    ladder.run(&fake).unwrap();
    assert!(ladder.best.is_none());
    // The regular pitch's connect rung and each finer pitch's (no board,
    // so each pitch's first rung runs alone and fails).
    assert_eq!(ladder.failed.len(), fake.routes().len());
    assert!(ladder.failed.len() >= 3, "{:?}", ladder.failed);
}

/// A panic in the seed retry or a seed round keeps the rung's board.
#[test]
fn a_panic_in_the_seeds_keeps_the_rungs_board() {
    let config = route_mode(100.0);
    let out = output("seeds");
    let fake = Fake::new(|routed| (routed.seeds == 1).then_some(Outcome { open: 4, work: 5.0 }));
    let mut ladder = ladder(&config, &out, 2);
    ladder.run(&fake).unwrap();
    assert!(!ladder.failed.is_empty());
    assert!(
        ladder.failed[0].contains("again with 4 seeds: panic: "),
        "{:?}",
        ladder.failed
    );
    assert_eq!(kept(&ladder, &out).0, 4);
    assert!(!out.with_extension("seeds").exists());
}

/// The layout's ladder (a work deadline): once the first attempt leaves
/// connections open a share of its work is kept for the narrow-signal
/// step (at least 240 s); no rung starts into it, their deadlines end
/// where it begins, and the step negotiates at least what the first
/// attempt did.
#[test]
fn a_layouts_ladder_keeps_its_reserve_for_the_step() {
    let mut config = with_narrow_step(route_mode(1000.0));
    config.deadline_work = Some(1500.0);
    let out = output("reserve");
    let fake = Fake::new(|routed| {
        Some(if routed.narrow {
            Outcome {
                open: 0,
                work: 100.0,
            }
        } else {
            Outcome {
                open: 3,
                work: 400.0,
            }
        })
    });
    let mut ladder = ladder(&config, &out, 2);
    ladder.run(&fake).unwrap();
    let routes = fake.routes();
    // 0.75 of the first attempt's 400 s: 300 s kept.
    assert_eq!(routes[0].deadline_work, Some(1500.0));
    assert_eq!(routes.len(), 3, "{routes:?}");
    // The next rung stops where the reserve begins (of the work left as
    // its pitch started); the finer pitch is projected beyond the
    // refinement budget.
    assert_eq!(routes[1].deadline_work, Some(1500.0 - 300.0));
    assert!(
        ladder
            .skipped
            .iter()
            .any(|why| why.contains("exceeds refine_budget_seconds"))
    );
    let step = routes.last().unwrap();
    assert!(step.narrow, "{routes:?}");
    // The first attempt's negotiation was a third of its rungs' share of
    // the budget; the step's is at least that.
    assert!(step.negotiation.unwrap() >= routes[0].negotiation.unwrap() - 1e-6);
    assert_eq!(kept(&ladder, &out), (0, "connect".into()));
}

/// The narrow-signal attempt a layout runs alone (its step applied, so
/// it keeps no reserve of its own): its seeds stop at their share of the
/// work left, and with `keep_for_continuation` they leave the attempt
/// after it its reserve. Before 2026-10-23 the seeds had no deadline:
/// DSP-ADAU1452 had 742 s left, its first attempt took 192 s, three
/// seeds ran to about 425 s each and the ladder charged 1470 s.
#[test]
fn a_layouts_narrow_attempt_alone_ends_within_its_work() {
    let mut config = with_narrow_step(route_mode(808.0));
    config.use_narrow_signals = Some(true);
    config.pours = KiCadPourMode::Connect;
    config.refine_pitches_mm = Some(Vec::new());
    config.negotiation_seconds = Some(501.0);
    config.deadline_work = Some(742.0);
    assert!(!narrow_step_applies(&config));
    for keep in [false, true] {
        config.keep_for_continuation = keep;
        let out = output(&format!("narrow-alone-{keep}"));
        let fake = Fake::new(|routed| {
            Some(if routed.first_seed > 0 {
                Outcome {
                    open: 70,
                    work: 425.0,
                }
            } else {
                Outcome {
                    open: 92,
                    work: 192.0,
                }
            })
        });
        let mut ladder = ladder(&config, &out, 4);
        ladder.run(&fake).unwrap();
        let routes = fake.routes();
        // Two seeds without the reserve (unbounded: 192 + 2 x 425 s), one
        // with it.
        let seeds = routes.last().unwrap().seeds;
        assert_eq!(seeds, if keep { 1 } else { 2 }, "{routes:?}");
        for routed in &routes {
            assert!(routed.deadline_work.is_some(), "{routes:?}");
        }
        assert!(
            ladder.work <= 742.0 + 1.0e-6,
            "{} ({routes:?})",
            ladder.work
        );
        if keep {
            assert!(
                742.0 - ladder.work >= NARROW_RESERVE_FLOOR,
                "{} ({routes:?})",
                ladder.work
            );
        }
    }
}

/// A finer pitch whose router (from the first attempt's maps) does not
/// fit the machine is not tried: decided by the maps and the machine's
/// memory, a fixed number, never by the process's peak.
#[test]
fn a_finer_pitch_that_does_not_fit_the_machine_is_skipped_by_its_maps() {
    let config = route_mode(1000.0);
    let out = output("machine");
    let fake = Fake::new(|_| Some(Outcome { open: 2, work: 5.0 }));
    // 100 MB of maps at 0.1 mm: about 0.94 GB a router at 0.075 mm, 1.28
    // GB at 0.05 mm (`router_peak_mb`); a machine of 3 GB takes routers
    // to 1.2 GB.
    let mut ladder = Ladder::new(
        &config,
        &out,
        vec!["GND".into()],
        2,
        machine(),
        Some(3000.0),
    );
    ladder.run(&fake).unwrap();
    let pitches: Vec<Option<f64>> = fake
        .routes()
        .iter()
        .filter(|routed| routed.seeds == 1)
        .map(|routed| routed.pitch)
        .collect();
    assert!(pitches.contains(&Some(0.075)), "{pitches:?}");
    assert!(!pitches.contains(&Some(0.05)), "{pitches:?}");
    assert!(
        ladder
            .skipped
            .iter()
            .any(|why| why.contains("0.05 mm) was not tried")
                && why.contains("does not fit the machine's memory")),
        "{:?}",
        ladder.skipped
    );
}

/// A machine where one router fits at a time (`routers_that_fit` gives 1).
fn one_at_a_time() -> MemoryLimits {
    MemoryLimits::resolve(&MemoryInputs {
        virtual_mb: 400.0,
        resident_mb: 100.0,
        total_mb: Some(64000.0),
        available_mb: Some(1500.0),
        ..MemoryInputs::default()
    })
}

/// The ladder's charged work, routes and kept board with the memory
/// limits `limits` (how many routers run side by side).
fn charged_with(
    config: &KiCadBoardRouterConfig,
    name: &str,
    layers: usize,
    probes: bool,
    limits: MemoryLimits,
) -> (KiCadWorkCharges, f64, Vec<Routed>, (usize, String)) {
    // (A rung that ran in a wave and that the order then skipped is in
    // the routes but charged nothing.)
    let out = output(name);
    let mut fake = Fake::new(|routed| {
        Some(if routed.narrow {
            Outcome { open: 1, work: 4.0 }
        } else if routed.seeds > 1 {
            Outcome { open: 4, work: 3.0 }
        } else {
            Outcome {
                open: if routed.rung == "tracks" { 6 } else { 5 },
                work: 6.0,
            }
        })
    });
    fake.layers = layers;
    if probes {
        fake.connections = 200;
        fake.probes = vec![30, 10, 20, 40];
        fake.continued = Outcome { open: 3, work: 8.0 };
        fake.probe_pour = 0.5;
    }
    fake.pour = 1.0;
    fake.build = 1.0;
    fake.shared_statics = 0.6;
    fake.lowering = 0.25;
    fake.limits = limits;
    let mut ladder = ladder(config, &out, layers);
    ladder.run(&fake).unwrap();
    (
        ladder.charged,
        ladder.work,
        fake.routes(),
        kept(&ladder, &out),
    )
}

/// Route mode: the work the ladder charges, and so every share after it,
/// is the same whether its rungs, probes and seeds run one at a time or
/// side by side (which follows the memory available): a router charges
/// its own maps and each shared part of a build (the static maps, the
/// pours' masks) is charged once a phase, by the first router of its key
/// in the rungs' order; the ladder holds the maps for the phase so that
/// what it charges once is built once; a rung's board is lowered once
/// whether it runs alone or in a wave. Until 2026-10-21 a router that
/// found its maps in the process charged its own, one that did not the
/// whole build, and a wave's hold lowered its rungs a second time.
#[test]
fn route_mode_charges_the_same_one_at_a_time_and_side_by_side() {
    for (layers, probes) in [(4, false), (4, true), (2, false)] {
        let config = with_narrow_step(route_mode(100.0));
        let name = format!("side-by-side-{layers}-{probes}");
        let alone = charged_with(
            &config,
            &format!("{name}-1"),
            layers,
            probes,
            one_at_a_time(),
        );
        let wide = charged_with(&config, &format!("{name}-3"), layers, probes, machine());
        // The charges, the work and the kept board are the same; so is
        // every router's limit (a wave also routes a rung the order then
        // skips: it counts for nothing).
        assert_eq!(
            (alone.0, alone.1, &alone.3),
            (wide.0, wide.1, &wide.3),
            "{layers} layers, probes {probes}"
        );
        assert!(
            alone.2.iter().all(|routed| wide.2.contains(routed)),
            "{:?} {:?}",
            alone.2,
            wide.2
        );
        let (charged, work, routes, _) = alone;
        // Rungs sharing maps charged them once: less than a whole build
        // a router.
        let routers = routes.iter().map(|routed| routed.seeds as f64).sum::<f64>()
            + if probes { 4.0 } else { 0.0 };
        assert!(
            charged.lattice_seconds < routers - 1e-6,
            "{charged:?} {routes:?}"
        );
        assert!(
            charged.lattice_seconds >= routers * 0.4 - 1e-6,
            "{charged:?}"
        );
        // One lowering a rung's route, a seed round's or a step's (and
        // each probe's board).
        let lowerings = routes.len() as f64 + if probes { 4.0 } else { 1.0 };
        assert!(
            (charged.lowering_seconds - 0.25 * lowerings).abs() < 1e-6,
            "{charged:?} {routes:?}"
        );
        assert!(work > 0.0);
    }
}

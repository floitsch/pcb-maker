// Copyright (C) 2026 Toit contributors.

//! `route_kicad_board`: the attempt ladder, its seeds and the router's
//! configuration, and the finish of the board kept.

use super::*;
use pcb_router as core;

/// Routes every connection of `<source>/<board_id>.kicad_pcb` and writes the
/// complete project to `output`.
pub fn route_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
) -> Result<KiCadBoardRouterResult, String> {
    let with_deadline;
    let config = match (config.deadline, config.deadline_seconds) {
        (None, Some(seconds)) => {
            // Stretched where a pinned rate is faster than the machine
            // (`KiCadWorkRate::wall_stretch`).
            let stretch = crate::work_rate::work_rate().wall_stretch(false);
            with_deadline = KiCadBoardRouterConfig {
                deadline: Some(
                    std::time::Instant::now()
                        + std::time::Duration::from_secs_f64(seconds.max(0.0) * stretch),
                ),
                ..config.clone()
            };
            &with_deadline
        }
        _ => config,
    };
    // Requested net classes set their nets' rules.
    let with_classes;
    let config = if config.net_classes.is_empty() {
        config
    } else {
        let board = source_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board)
            .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        with_classes = crate::net_classes::with_net_classes(
            config,
            &parse(&text).map_err(crate::verdict::input_error)?,
        )
        .map_err(crate::verdict::input_error)?;
        &with_classes
    };
    // The minimum connection width floors every width, also those of a
    // configuration file or of requested net classes.
    let floored;
    let config = if config.width_floor() > 0.0 {
        let mut copy = config.clone();
        copy.apply_width_floor();
        floored = copy;
        &floored
    } else {
        config
    };
    // With interchangeable pins, the ladder routes a copy with the pins
    // assigned; every attempt copies its project from there.
    // Requested pours and net classes go into a copy of the project that the
    // attempts use.
    let poured_directory = output_directory.with_extension("poured");
    let (planned, strip) = {
        let board = source_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board)
            .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        let pcb =
            crate::canonical_order::parse_board(&text).map_err(crate::verdict::input_error)?;
        // `strip`: the board's own tracks and vias go before anything
        // reads the board (by default they are kept, `kept`).
        let strip =
            config.existing_tracks == KiCadExistingTracks::Strip && count_tracks(&pcb) != [0, 0, 0];
        (
            planned_pours(&pcb, config).map_err(crate::verdict::input_error)?,
            strip,
        )
    };
    if config.add_pours.is_empty() && !planned.is_empty() {
        eprintln!(
            "automatic planes: {}",
            planned
                .iter()
                .map(|pour| format!("{} on {}", pour.net, pour.layers.join(", ")))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    let prepared = !planned.is_empty() || !config.net_classes.is_empty() || strip;
    let source_directory = if !prepared {
        source_directory
    } else {
        if poured_directory.exists() {
            fs::remove_dir_all(&poured_directory).map_err(|error| error.to_string())?;
        }
        copy_directory_tree(source_directory, &poured_directory)?;
        let board = poured_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board)
            .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        let mut pcb = crate::canonical_order::parse_board(&text)?;
        if strip {
            eprintln!(
                "existing tracks: {} track segments, arcs and vias removed (strip)",
                strip_tracks(&mut pcb)
            );
        }
        add_pour_zones(&mut pcb, &planned).map_err(crate::verdict::input_error)?;
        fs::write(&board, format!("{}\n", encode(&pcb)))
            .map_err(|error| format!("failed to write {}: {error}", board.display()))?;
        if !config.net_classes.is_empty() {
            crate::net_classes::write_net_classes(
                &poured_directory,
                board_id,
                &pcb,
                &config.net_classes,
            )?;
        }
        poured_directory.as_path()
    };
    let swapped_directory = output_directory.with_extension("swapped");
    let (source_directory, pin_swaps) = match &config.pin_swaps {
        Some(path) => {
            let spec = read_pin_swap_spec(&source_directory.join(path))
                .map_err(crate::verdict::input_error)?;
            let result = swap_kicad_pins(
                source_directory,
                board_id,
                &swapped_directory,
                &spec,
                &config.pin_swap,
            )?;
            eprintln!(
                "pin swaps: ratsnest {:.0} mm with {} crossings -> {:.0} mm with {} crossings, {} pins changed",
                result.before.length_mm,
                result.before.crossings,
                result.after.length_mm,
                result.after.crossings,
                result.changes.len()
            );
            (swapped_directory.as_path(), Some(result))
        }
        None => (source_directory, None),
    };
    // The topological engine makes one attempt, pour nets as tracks.
    if config.engine.as_deref() == Some("topological") {
        let mut report =
            route_kicad_board_once(source_directory, board_id, output_directory, config, false)?;
        report.pin_swaps = pin_swaps;
        report.pours = "topological (pour nets as tracks)".into();
        return Ok(report);
    }
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let parsed = crate::canonical_order::parse_board(&source)?;
    let layer_table = LayerTable::from_pcb(&parsed)?;
    let pour_nets: Vec<String> = pours(&parsed, &layer_table)?
        .into_iter()
        .map(|pour| pour.net)
        .collect();
    let memory_at_start = MemoryLimits::read(config.memory_mb);
    eprintln!(
        "memory limits: {} (the process holds {:.0} MB resident)",
        memory_at_start.describe(),
        resident_mb()
    );
    let kept_tracks = count_tracks(&parsed);
    if kept_tracks != [0, 0, 0] {
        eprintln!(
            "existing tracks: {} segments, {} arcs and {} vias kept; the router routes what they leave open (--strip routes from nothing)",
            kept_tracks[0], kept_tracks[1], kept_tracks[2]
        );
    }
    let runner = KiCadRunner {
        kept: std::sync::OnceLock::new(),
        lowering_work: lowering_work(&parsed),
        parsed: &parsed,
        layer_names: layer_table.names.clone(),
        source_directory,
        board_id,
        memory_mb: config.memory_mb,
    };
    // The ladder publishes each best attempt into a staging directory
    // beside the output, which becomes the output only when the run is done
    // (the report written last, then one rename): a run killed meanwhile
    // (the OOM killer, a signal) leaves no board-router.json in the output,
    // so that its best attempt so far cannot pass for a result (review
    // 2026-10-21, H1: three such rows had `exit: -9` and a result).
    let final_directory = output_directory;
    let staging = output_directory.with_extension("partial");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
    }
    let output_directory = staging.as_path();
    let mut ladder = Ladder::new(
        config,
        output_directory,
        pour_nets,
        layer_table.names.len(),
        memory_at_start,
        memory_total_mb(),
    );
    ladder.panic_at = TestPanic::from_env();
    ladder.run(&runner)?;
    let Some((final_open, mut result)) = ladder.best.take() else {
        return Err(if ladder.failed.is_empty() {
            "no attempt of the ladder produced a board".to_string()
        } else {
            format!(
                "no attempt of the ladder produced a board: {} failed ({})",
                ladder.failed.len(),
                ladder.failed.join("; ")
            )
        });
    };
    result.pin_swaps = pin_swaps;
    // The connections the board's own copper made, from whichever
    // lowering ran (an attempt's own or the ladder's).
    let kept = result
        .kept_copper
        .take()
        .or_else(|| runner.kept.get().cloned());
    if kept_tracks != [0, 0, 0] {
        let mut kept = kept.unwrap_or_default();
        [kept.segments, kept.arcs, kept.vias] = kept_tracks;
        result.kept_copper = Some(kept);
    }
    if prepared {
        fs::remove_dir_all(&poured_directory).map_err(|error| error.to_string())?;
    }
    if config.pin_swaps.is_some() {
        fs::remove_dir_all(&swapped_directory).map_err(|error| error.to_string())?;
    }
    // Pads KiCad finds starved, solid (`solid_starved_thermals`, on unless
    // false). Also the pads whose spokes reach only an isolated island of
    // fill (no spoke count in the finding): solid, the pad joins the piece
    // and the finding goes (kicad-cli 10.0.7 on mackerel-30-proto's route:
    // R14.1, U2.42, U2.43 and U11.61 on In1.Cu and In2.Cu, four starved
    // thermals to none, nothing else changed).
    if config.solid_starved_thermals != Some(false) && !config.skip_native_verification {
        let checks = solidify_starved_thermals(&mut result, output_directory, board_id)?;
        ladder.charge_checks(checks);
    }
    // The neck repair improves a checked board; past the deadline it is
    // skipped (up to six more KiCad checks, uncounted as work).
    if result.native.is_some() && !ladder.past_deadline() {
        let checks = repair_necks(&mut result, output_directory, board_id, config, None)?;
        ladder.charge_checks(checks);
    }
    result.cut = core::router::wall_cuts();
    result.work_seconds = ladder.work;
    result.work_bound_seconds = ladder.work_bound;
    result.memory_limits = ladder.memory_at_start.describe();
    if let Some(bound) = ladder.work_bound {
        eprintln!(
            "ladder: {:.1} s of work of its bound of {bound:.0} s (a budget of {:.0} s)",
            ladder.work, ladder.ladder_budget
        );
    }
    let accounting = KiCadWallAccounting::now(
        ladder.started.elapsed().as_secs_f64(),
        ladder.work,
        ladder.charged,
    );
    eprintln!("{}", accounting.line());
    result.wall_accounting = Some(accounting);
    result.kicad_checks_charged = ladder.checks_charged.checks;
    result.kicad_check_work_seconds = ladder.checks_charged.work();
    result.attempts_failed = std::mem::take(&mut ladder.failed);
    // What did not run matters only to a board left incomplete.
    if final_open.0 > 0 || final_open.1 > 0 {
        result.skipped = std::mem::take(&mut ladder.skipped);
    }
    // How whole the pours stayed, after every decision (it counts as no
    // work). A layout's own ladders (`kicad_kept` set) leave it to the
    // layout, which measures the board it keeps.
    // The source board's parse goes first: the measure parses and lowers
    // the board written (jetson: a 25 MB file, 240 MB parsed; the measure
    // was the run's peak with both held).
    drop(runner);
    drop(parsed);
    drop(source);
    if config.kicad_kept.is_none() {
        result.pour_quality = measured_pours(output_directory, board_id, config);
    }
    let report_path = output_directory.join("board-router.json");
    crate::write_atomically(
        &report_path,
        serde_json::to_string_pretty(&result)
            .map_err(|error| error.to_string())?
            .as_bytes(),
    )?;
    // The finished project takes the output's place in one rename.
    if final_directory.exists() {
        fs::remove_dir_all(final_directory).map_err(|error| error.to_string())?;
    }
    fs::rename(output_directory, final_directory).map_err(|error| {
        format!(
            "failed to move {} to {}: {error}",
            output_directory.display(),
            final_directory.display()
        )
    })?;
    Ok(result)
}

/// The ladder's runner on a KiCad board (`LadderRunner`): the rungs'
/// boards lowered from `parsed`, every attempt routed from the project in
/// `source_directory`.
struct KiCadRunner<'a> {
    parsed: &'a Expr,
    /// What the first lowering kept of the board's own copper
    /// (`keep_existing_copper`).
    kept: std::sync::OnceLock<KiCadKeptCopper>,
    layer_names: Vec<String>,
    source_directory: &'a Path,
    board_id: &'a str,
    memory_mb: Option<f64>,
    /// The work of one lowering of `parsed` (`lowering_work`).
    lowering_work: f64,
}

impl LadderRunner for KiCadRunner<'_> {
    type Board = core::Board;
    type Probe = core::router::Router;
    type Routed = core::RoutingResult;
    type Hold = core::router::StaticMapsHold;

    fn lower(
        &self,
        attempt: &KiCadBoardRouterConfig,
        connect: bool,
    ) -> Result<core::Board, String> {
        let mut board = lower(self.parsed, attempt, connect)?.board;
        // The board's own tracks and vias: what they join is connected.
        let kept = keep_existing_copper(&mut board);
        let _ = self.kept.set(kept);
        Ok(board)
    }

    fn size(&self, board: &core::Board) -> (usize, usize) {
        (
            board.layer_count,
            board
                .nets
                .iter()
                .map(|net| net.terminals.len().saturating_sub(1))
                .sum(),
        )
    }

    fn build(
        &self,
        board: &core::Board,
        attempt: &KiCadBoardRouterConfig,
        quiet: bool,
    ) -> core::router::Router {
        let mut config = core_config(attempt);
        config.verbose = config.verbose && !quiet;
        core::router::Router::new(board, &config)
    }

    fn hold(
        &self,
        board: &core::Board,
        attempt: &KiCadBoardRouterConfig,
    ) -> core::router::StaticMapsHold {
        core::router::hold_static_maps(board, &core_config(attempt))
    }

    fn hold_key(&self, hold: &core::router::StaticMapsHold) -> Option<u128> {
        hold.key()
    }

    fn statics_key(&self, board: &core::Board, attempt: &KiCadBoardRouterConfig) -> Option<u128> {
        core::router::static_maps_key(board, &core_config(attempt))
    }

    fn maps_mb(&self, router: &core::router::Router) -> Option<f64> {
        maps_mb_of(&router.memory_report())
    }

    fn statics_mb(&self, router: &core::router::Router) -> Option<f64> {
        statics_mb_of(&router.memory_report())
    }

    fn probe(&self, router: &mut core::router::Router, work: f64, mode: usize) -> usize {
        let unfinished = router.probe(work);
        // Experiment hook (the ladder's judge, docs/congestion-model.md):
        // the probe's other statistics.
        if std::env::var_os("PCB_LADDER_STATS").is_some() {
            let labels = router.congestion_labels();
            let nets = labels.net_routable.len();
            let routable = |net: usize| labels.net_routable[net];
            let incomplete = (0..nets)
                .filter(|net| routable(*net) && !labels.net_complete[*net])
                .count();
            let conflicted = (0..nets)
                .filter(|net| routable(*net) && labels.net_conflicted[*net])
                .count();
            // Nets conflicted or incomplete; the rest of `unfinished` is
            // pour pads the pour does not reach yet.
            let either = (0..nets)
                .filter(|net| {
                    routable(*net) && (labels.net_conflicted[*net] || !labels.net_complete[*net])
                })
                .count();
            let stranded = unfinished.saturating_sub(either);
            eprintln!(
                "ladder stats: rung {mode} unfinished {unfinished} conflicted {conflicted} incomplete {incomplete} nets {either} stranded {stranded}"
            );
        }
        unfinished
    }

    fn work(&self, router: &core::router::Router) -> RouterWork {
        RouterWork {
            expansions: router.expansions(),
            pour: router.pour_work(),
            build: router.build_work(),
            shared: router.shared_build(),
        }
    }

    fn lowering_work(&self, _: &core::Board) -> f64 {
        self.lowering_work
    }

    fn speak(&self, router: &mut core::router::Router, attempt: &KiCadBoardRouterConfig) {
        router.config_mut().verbose = core_config(attempt).verbose;
    }

    fn resume(
        &self,
        mut router: core::router::Router,
        limits: ResumeLimits,
    ) -> (core::RoutingResult, RouterWork) {
        // The continuation's limits are search work (the ladder's shares
        // by the routers' proportion of search), whatever the probe's.
        router.config_mut().limits_count_all = false;
        router.config_mut().run_work = Some(limits.run_work);
        if let Some(deadline) = limits.deadline_work {
            router.config_mut().deadline_work = Some(deadline);
        }
        let routed = router.resume(limits.negotiation);
        let work = self.work(&router);
        (routed, work)
    }

    fn write(
        &self,
        board: &core::Board,
        routed: core::RoutingResult,
        attempt: &KiCadBoardRouterConfig,
        directory: &Path,
        routing_seconds: f64,
    ) -> Result<KiCadBoardRouterResult, String> {
        write_attempt(
            self.parsed.clone(),
            board,
            routed,
            &self.layer_names,
            self.source_directory,
            self.board_id,
            directory,
            attempt,
            [0.0, routing_seconds],
        )
    }

    fn route(
        &self,
        directory: &Path,
        attempt: &KiCadBoardRouterConfig,
        connect: bool,
    ) -> Result<KiCadBoardRouterResult, String> {
        route_kicad_board_once(
            self.source_directory,
            self.board_id,
            directory,
            attempt,
            connect,
        )
    }

    fn route_lowered(
        &self,
        directory: &Path,
        attempt: &KiCadBoardRouterConfig,
        connect: bool,
        board: core::Board,
    ) -> Result<KiCadBoardRouterResult, String> {
        route_kicad_board_lowered(
            self.source_directory,
            self.board_id,
            directory,
            attempt,
            connect,
            Some((board, self.lowering_work)),
        )
    }

    fn check(&self, result: &mut KiCadBoardRouterResult, directory: &Path) -> Result<(), String> {
        check_attempt(result, directory, self.board_id)
    }

    fn starved(&self, directory: &Path) -> usize {
        starved_thermals(directory)
    }

    fn memory_limits(&self) -> MemoryLimits {
        MemoryLimits::read(self.memory_mb)
    }

    fn pour_loss(&self, directory: &Path, attempt: &KiCadBoardRouterConfig) -> Option<f64> {
        match measure_pours(directory, self.board_id, attempt) {
            Ok(pours) if !pours.is_empty() => Some(pour_loss(&pours)),
            Ok(_) => None,
            Err(error) => {
                eprintln!("pours not measured for the ranking: {error}");
                None
            }
        }
    }
}

/// Routes `board` with seeds `first..first + count` in parallel (seed 0 is
/// the deterministic order) and returns the best result, with the largest
/// router's lattice maps in megabytes (`route_measured`).
pub(super) fn route_seeds(
    board: &core::Board,
    config: &core::Config,
    first: u64,
    count: usize,
    side_by_side: usize,
) -> (core::RoutingResult, f64) {
    // In waves of `side_by_side` (memory), in seed order.
    let seeds: Vec<u64> = (first..first + count.max(1) as u64).collect();
    // The seeds' static maps, held before they start side by side: built
    // once (or held by the ladder already), and shared (the ladder
    // charges the kept seed's own maps once a seed and the shared parts
    // of its build once, `Ladder::lattice_charge`).
    let _held = core::router::hold_static_maps(board, config);
    let mut results: Vec<(Option<u64>, core::RoutingResult)> = Vec::new();
    let mut maps_mb = 0.0f64;
    for wave in seeds.chunks(side_by_side.max(1)) {
        let routed: Vec<(Option<u64>, (core::RoutingResult, f64))> = std::thread::scope(|scope| {
            let handles: Vec<_> = wave
                .iter()
                .map(|&index| {
                    let mut seeded = config.clone();
                    seeded.seed = (index > 0).then_some(index);
                    seeded.verbose = config.verbose && index == first;
                    scope.spawn(move || (seeded.seed, route_measured(board, &seeded)))
                })
                .collect();
            handles
                .into_iter()
                // A seed's panic is the round's, with its own message.
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
                })
                .collect()
        });
        for (seed, (result, maps)) in routed {
            maps_mb = maps_mb.max(maps);
            results.push((seed, result));
        }
    }
    let key = |result: &core::RoutingResult| {
        let open: usize = result
            .status
            .iter()
            .map(|status| match status {
                core::NetStatus::Partial {
                    unconnected_terminals,
                } => *unconnected_terminals,
                core::NetStatus::Unreachable => 1,
                _ => 0,
            })
            .sum::<usize>()
            + core::verify(board, &result.routes).len();
        let vias: usize = result.routes.iter().map(|route| route.vias.len()).sum();
        let length: f64 = result
            .routes
            .iter()
            .flat_map(|route| route.segments.iter())
            .map(|segment| distance_squared(segment.start, segment.end).sqrt())
            .sum();
        (open, vias, length)
    };
    let scored: Vec<_> = results
        .into_iter()
        .map(|(seed, result)| (key(&result), seed, result))
        .collect();
    for (score, seed, _) in &scored {
        eprintln!(
            "seed {seed:?}: {} open, {} vias, {:.0} mm",
            score.0, score.1, score.2
        );
    }
    let best = scored
        .into_iter()
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .expect("at least one seed")
        .2;
    (best, maps_mb)
}

/// The core router configuration for a KiCad configuration.
pub(crate) fn core_config(config: &KiCadBoardRouterConfig) -> core::Config {
    let mut router_config = core::Config {
        verbose: true,
        deadline: config.deadline,
        deadline_work: config.router_deadline_work,
        net_rules: config.net_rules.clone(),
        limits_count_all: config.limits_count_all,
        ..core::Config::default()
    };
    if let Some(via_cost) = config.via_cost_mm {
        router_config.via_cost = via_cost;
    }
    if let Some(strict) = config.strict_via_budget {
        router_config.strict_via_budget = strict;
    }
    router_config.run_work = config.run_work;
    if let Some(seconds) = config.cleanup_seconds {
        router_config.cleanup_seconds = router_config.cleanup_seconds.min(seconds);
    }
    if let Some(against) = config.against_direction_cost {
        router_config.against_direction = against;
    }
    if let Some(iterations) = config.maximum_iterations {
        router_config.max_iterations = iterations;
    }
    if let Some(pitches) = &config.grid_pitches_mm {
        router_config.pitches = pitches.clone();
    }
    if let Some(weight) = config.heuristic_weight {
        router_config.heuristic_weight = weight;
    }
    if let Some(cap) = config.present_cap {
        router_config.present_cap = cap;
    }
    if let Some(factor) = config.present_factor {
        router_config.present_factor = factor;
    }
    if let Some(growth) = config.present_growth {
        router_config.present_growth = growth;
    }
    if let Some(drop) = config.stall_drop {
        router_config.stall_drop = drop;
    }
    if let Some(stall) = config.stall_at_cap {
        router_config.stall_at_cap = stall;
    }
    if let Some(seconds) = config.negotiation_seconds {
        // A budget given in seconds is turned into work (search
        // expansions), so that results do not depend on the machine's
        // load; the clock stays as a guard.
        router_config.negotiation_expansions = core::router::negotiation_expansions(seconds);
        router_config.negotiation_seconds = seconds * core::router::guard();
        // The polish after it may not take longer than the negotiation was
        // allowed.
        router_config.via_reduction_seconds = seconds.min(300.0);
    } else {
        // The router's default budget, in work too: left in seconds it was
        // the only limit of an attempt the ladder repeats from a probed
        // rung (its seeds, the narrow-signal and relaxed-clearance steps),
        // and video's narrow-signal attempt stopped at 900 s of wall clock
        // wherever the load had brought it (1124 s of work on one run).
        let seconds = router_config.negotiation_seconds;
        router_config.negotiation_expansions = core::router::negotiation_expansions(seconds);
        router_config.negotiation_seconds = seconds * core::router::guard();
    }
    if let Some(length) = config.escape_stub_mm {
        router_config.escape_stub_mm = length;
    }
    if let Some(fixed) = config.fixed_plane_stubs {
        router_config.fixed_plane_stubs = fixed;
    }
    if let Some(cost) = config.thermal_guard_cost {
        router_config.thermal_guard_cost = cost;
    }
    if let Some(global) = config.global_routing {
        router_config.global_routing = global;
    }
    if let Some(present) = config.via_reduction_present {
        router_config.via_reduction_present = present;
    }
    if let Some(exclusive) = config.exclusive_planes {
        router_config.exclusive_planes = exclusive;
    }
    if let Some(islands) = config.pour_islands {
        router_config.pour_islands = islands;
    }
    if let Some(kicad_spokes) = config.kicad_spokes {
        router_config.kicad_spokes = kicad_spokes;
    }
    if let Some(manual_spokes) = config.manual_spokes {
        router_config.manual_spokes = manual_spokes;
    }
    if let Some(reach) = config.neck_reach_mm {
        router_config.neck_reach = reach;
    }
    if let Some(cost) = config.plane_cut_cost {
        router_config.plane_cut_cost = cost;
    }
    if let Some(weight) = config.pour_integrity {
        router_config.pour_integrity = weight;
    }
    if let Some(cost) = config.cleanup_via_cost_mm {
        router_config.cleanup_via_cost = cost;
    }
    if let Some(passes) = config.cleanup_passes {
        router_config.cleanup_passes = passes;
    }
    if let Some(cost) = config.bend_cost_mm {
        router_config.bend_cost = cost;
    }
    if let Some(rounds) = config.via_reduction_rounds {
        router_config.via_reduction_rounds = rounds;
    }
    if let Some(skeleton) = config.plane_skeleton {
        router_config.plane_skeleton = skeleton;
    }
    if let Some(bias) = config.skeleton_bias {
        router_config.skeleton_bias = bias;
    }
    router_config
}

/// Writes `pcb` (with its copper) as the project in `output_directory`,
/// verifies it natively unless told not to, and builds the report.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_routed_board(
    pcb: &Expr,
    board: &core::Board,
    result: &core::RoutingResult,
    nets: Vec<KiCadBoardRouterNet>,
    layer_names: &[String],
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
    timings: [f64; 2],
) -> Result<KiCadBoardRouterResult, String> {
    let verify_started = std::time::Instant::now();
    let violations = core::verify(board, &result.routes);
    let verify_seconds = verify_started.elapsed().as_secs_f64();
    if output_directory.exists() {
        return Err(format!(
            "output directory {} already exists",
            output_directory.display()
        ));
    }
    copy_directory_tree(source_directory, output_directory)?;
    let output_board = output_directory.join(format!("{board_id}.kicad_pcb"));
    fs::write(&output_board, format!("{}\n", encode(pcb)))
        .map_err(|error| format!("failed to write {}: {error}", output_board.display()))?;
    let native_started = std::time::Instant::now();
    let native = if config.skip_native_verification {
        None
    } else {
        Some(verify_materialized_rung(output_directory, board_id)?)
    };
    let native_verification_seconds = native_started.elapsed().as_secs_f64();
    let drc_cache = crate::drc_cache_counts();
    let (lowerings, lowering_total) = lowering_totals();
    let (builds, build_total) = core::router::build_totals();
    eprintln!(
        "board check: internal verification {verify_seconds:.1}s, KiCad {} (lowering {:.1}s, routing {:.1}s); drc_cache: {{calls {}, hits {}}}; so far {lowerings} lowerings {lowering_total:.1}s, {builds} lattices {build_total:.1}s",
        if native.is_some() {
            format!("{native_verification_seconds:.1}s")
        } else {
            "not run".to_string()
        },
        timings[0],
        timings[1],
        drc_cache.calls,
        drc_cache.hits
    );
    let routable: Vec<_> = nets.iter().filter(|net| net.terminals >= 2).collect();
    let report = KiCadBoardRouterResult {
        board_id: board_id.into(),
        routable_connections: routable.len(),
        routed_connections: routable.iter().filter(|net| net.status == "routed").count(),
        unconnected_terminals: nets.iter().map(|net| net.unconnected_terminals).sum(),
        vias: nets.iter().map(|net| net.vias).sum(),
        length_mm: nets.iter().map(|net| net.length_mm).sum(),
        grid_pitch_mm: result.grid.pitch,
        grid_nodes: [result.grid.nx, result.grid.ny],
        iterations: result.iterations,
        searches: result.searches,
        expansions: result.expansions,
        lowering_seconds: timings[0],
        routing_seconds: timings[1],
        internal_verification_seconds: 0.0,
        native_verification_seconds,
        internal_violations: violations
            .iter()
            .map(|violation| KiCadBoardRouterViolation {
                connection: board.nets[violation.net as usize].name.clone(),
                other: violation.other.clone(),
                layer: layer_names[violation.layer].clone(),
                at: violation.at,
                required_mm: violation.required,
                actual_mm: violation.actual,
            })
            .collect(),
        native,
        pours: String::new(),
        nets,
        congestion: result.congestion.clone(),
        grid_origin: result.grid.origin,
        pin_swaps: None,
        diagnostics: KiCadRoutingDiagnostics {
            starved_thermals: starved_thermal_diagnostics(output_directory),
            ..diagnostics(board, result, layer_names)
        },
        cut: Vec::new(),
        work_seconds: 0.0,
        kicad_check_work_seconds: 0.0,
        kicad_checks_charged: 0,
        skipped: Vec::new(),
        neck_repair: None,
        rules: KiCadRulesSummary::of(board, config),
        maps_mb: 0.0,
        work_bound_seconds: None,
        memory_limits: String::new(),
        attempts_failed: Vec::new(),
        island_stubs: result.island_stubs,
        manual_spokes_work_seconds: result.manual_spokes_work,
        first_iteration_expansions: result.first_iteration_expansions,
        pour_work_seconds: result.pour_work,
        build_work_seconds: result.build_work,
        shared_build: result.shared_build,
        lowering_work_seconds: 0.0,
        wall_accounting: None,
        kept_copper: None,
        warnings: input_warnings(pcb),
        pour_quality: Vec::new(),
        pour_loss: None,
    };
    let report_path = output_directory.join("board-router.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(report)
}

/// Lays tracks over KiCad's connection_width necks on the finished board
/// in `directory` (`necks::repair_connection_width`) and, when the board
/// changed, takes its new native check and starved thermals.
/// Returns the KiCad runs it made: its DRC checks and, when it repaired
/// anything, KiCad's fill of a scratch copy (each a check's work,
/// `check_work`). `max_checks`: the KiCad runs its work may take (a
/// layout's work left over a check's work); with fewer than
/// `NECK_MIN_CHECKS` the repair does not run, and the note says so.
pub(crate) fn repair_necks(
    result: &mut KiCadBoardRouterResult,
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
    max_checks: Option<usize>,
) -> Result<usize, String> {
    if config.skip_native_verification {
        return Ok(0);
    }
    let calls = crate::drc_cache_counts().calls;
    let mut fills = 0;
    if let Some(repair) =
        crate::necks::repair_connection_width(directory, board_id, config, max_checks)?
    {
        fills = repair.kicad_fills;
        if repair.kept {
            result.native = Some(verify_materialized_rung(directory, board_id)?);
            result.diagnostics.starved_thermals = starved_thermal_diagnostics(directory);
        }
        result.neck_repair = Some(repair);
    }
    let checks = crate::drc_cache_counts().calls.saturating_sub(calls) as usize;
    NECK_CHECKS.fetch_add(checks, std::sync::atomic::Ordering::Relaxed);
    Ok(checks + fills)
}

/// KiCad checks the neck repair ran in this process (`repair_necks`).
static NECK_CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many KiCad checks the neck repair ran in this process.
pub(crate) fn neck_checks() -> usize {
    NECK_CHECKS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Number of `starved_thermal` findings in the directory's native DRC report.
pub(crate) fn starved_thermals(directory: &Path) -> usize {
    let Ok(text) = fs::read_to_string(directory.join("drc.json")) else {
        return 0;
    };
    let Ok(report) = serde_json::from_str::<serde_json::Value>(&text) else {
        return 0;
    };
    report["violations"].as_array().map_or(0, |violations| {
        violations
            .iter()
            .filter(|violation| violation["type"].as_str() == Some("starved_thermal"))
            .count()
    })
}

/// `solid_starved_thermals` on a checked board: the pads KiCad finds
/// starved get `(zone_connect 2)`, the board is checked again and kept when
/// `solid_kept`, else restored. Returns the KiCad checks it ran (0 or 1).
/// The layout runs it once, on the board it keeps (its own attempts'
/// comparisons count the starved thermals as KiCad found them).
pub(crate) fn solidify_starved_thermals(
    result: &mut KiCadBoardRouterResult,
    output_directory: &Path,
    board_id: &str,
) -> Result<usize, String> {
    let mut checks = 0;
    let starved: Vec<(String, String)> = result
        .diagnostics
        .starved_thermals
        .iter()
        .filter_map(|starved| {
            let text = &starved.pad;
            let number = text.split("ad ").nth(1)?.split(" [").next()?.to_string();
            let reference = text.split(" of ").nth(1)?.split(" on ").next()?.to_string();
            Some((reference, number))
        })
        .collect();
    if !starved.is_empty() {
        let board_path = output_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board_path)
            .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
        let mut pcb = parse(&text)?;
        let mut changed = 0;
        if let Expr::List(items) = &mut pcb {
            for footprint in items
                .iter_mut()
                .filter(|item| item.head() == Some("footprint"))
            {
                let reference = footprint_reference(footprint)
                    .unwrap_or_default()
                    .to_string();
                let Expr::List(children) = footprint else {
                    continue;
                };
                for pad in children
                    .iter_mut()
                    .filter(|child| child.head() == Some("pad"))
                {
                    let number = pad
                        .children()
                        .get(1)
                        .and_then(Expr::atom)
                        .unwrap_or("")
                        .to_string();
                    if !starved.iter().any(|(r, n)| *r == reference && *n == number) {
                        continue;
                    }
                    if let Expr::List(fields) = pad {
                        fields.retain(|field| field.head() != Some("zone_connect"));
                        fields.push(Expr::List(vec![
                            Expr::Atom("zone_connect".into()),
                            Expr::Atom("2".into()),
                        ]));
                        changed += 1;
                    }
                }
            }
        }
        if changed > 0 {
            // The checked board and its reports, to go back to: a solid
            // pad changes the fill, which can cost more than it gains
            // (k30-SBC's J1.14, a 0.41 mm card-connector pad set solid:
            // its starved thermal gone, but J1.39 beside it
            // unconnected twice).
            let mut kept: Vec<(std::path::PathBuf, Vec<u8>)> = Vec::new();
            for entry in fs::read_dir(output_directory).map_err(|error| error.to_string())? {
                let path = entry.map_err(|error| error.to_string())?.path();
                if path.is_file() {
                    let bytes = fs::read(&path)
                        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
                    kept.push((path, bytes));
                }
            }
            let before = result.native.clone();
            let starved_before = result.diagnostics.starved_thermals.clone();
            fs::write(&board_path, format!("{}\n", encode(&pcb)))
                .map_err(|error| format!("failed to write {}: {error}", board_path.display()))?;
            let after = verify_materialized_rung(output_directory, board_id)?;
            checks = 1;
            // Kept when KiCad finds no more unconnected items and fewer
            // findings (the starved thermals gone, nothing new).
            let better = solid_kept(before.as_ref(), &after);
            if better {
                result.native = Some(after);
                result.diagnostics.starved_thermals = starved_thermal_diagnostics(output_directory);
                eprintln!(
                    "{changed} starved pads connected solid: {} starved thermals left",
                    result.diagnostics.starved_thermals.len()
                );
            } else {
                for entry in fs::read_dir(output_directory).map_err(|error| error.to_string())? {
                    let path = entry.map_err(|error| error.to_string())?.path();
                    if path.is_file() && !kept.iter().any(|(kept, _)| *kept == path) {
                        fs::remove_file(&path).map_err(|error| {
                            format!("failed to remove {}: {error}", path.display())
                        })?;
                    }
                }
                for (path, bytes) in &kept {
                    fs::write(path, bytes)
                        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
                }
                result.native = before;
                result.diagnostics.starved_thermals = starved_before;
                eprintln!(
                    "{changed} starved pads connected solid: not kept ({} unconnected, {} findings; before {}, {})",
                    after.selected_net_unconnected_items,
                    after.drc_design_violations,
                    result
                        .native
                        .as_ref()
                        .map_or(0, |native| native.selected_net_unconnected_items),
                    result
                        .native
                        .as_ref()
                        .map_or(0, |native| native.drc_design_violations),
                );
            }
        }
    }
    Ok(checks)
}

/// Whether the board with its starved pads solid is kept: KiCad finds no
/// more unconnected items on it and fewer findings than on the board
/// before (the starved thermals gone, nothing new in their place).
pub(super) fn solid_kept(
    before: Option<&crate::verification::VerificationReport>,
    after: &crate::verification::VerificationReport,
) -> bool {
    before.is_none_or(|before| {
        after.selected_net_unconnected_items <= before.selected_net_unconnected_items
            && after.drc_design_violations < before.drc_design_violations
    })
}

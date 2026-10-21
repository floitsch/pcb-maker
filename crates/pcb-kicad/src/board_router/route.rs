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
    let planned = {
        let board = source_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board)
            .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        planned_pours(
            &crate::canonical_order::parse_board(&text).map_err(crate::verdict::input_error)?,
            config,
        )
        .map_err(crate::verdict::input_error)?
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
    let prepared = !planned.is_empty() || !config.net_classes.is_empty();
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
    let runner = KiCadRunner {
        lowering_work: lowering_work(&parsed),
        parsed: &parsed,
        layer_names: layer_table.names.clone(),
        source_directory,
        board_id,
        memory_mb: config.memory_mb,
    };
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
    if prepared {
        fs::remove_dir_all(&poured_directory).map_err(|error| error.to_string())?;
    }
    if config.pin_swaps.is_some() {
        fs::remove_dir_all(&swapped_directory).map_err(|error| error.to_string())?;
    }
    if config.solid_starved_thermals == Some(true) && !config.skip_native_verification {
        let starved: Vec<(String, String)> = result
            .diagnostics
            .starved_thermals
            .iter()
            .filter(|starved| starved.spokes.is_some())
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
                fs::write(&board_path, format!("{}\n", encode(&pcb))).map_err(|error| {
                    format!("failed to write {}: {error}", board_path.display())
                })?;
                result.native = Some(verify_materialized_rung(output_directory, board_id)?);
                ladder.charge_checks(1);
                result.diagnostics.starved_thermals = starved_thermal_diagnostics(output_directory);
                eprintln!(
                    "{changed} starved pads connected solid: {} starved thermals left",
                    result.diagnostics.starved_thermals.len()
                );
            }
        }
    }
    // The neck repair improves a checked board; past the deadline it is
    // skipped (up to six more KiCad checks, uncounted as work).
    if result.native.is_some() && !ladder.past_deadline() {
        let checks = repair_necks(&mut result, output_directory, board_id, config)?;
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
    let report_path = output_directory.join("board-router.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(result)
}

/// The ladder's runner on a KiCad board (`LadderRunner`): the rungs'
/// boards lowered from `parsed`, every attempt routed from the project in
/// `source_directory`.
struct KiCadRunner<'a> {
    parsed: &'a Expr,
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
        Ok(lower(self.parsed, attempt, connect)?.board)
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
    ) -> (core::router::StaticMapsHold, f64) {
        core::router::hold_static_maps(board, &core_config(attempt))
    }

    fn maps_mb(&self, router: &core::router::Router) -> Option<f64> {
        maps_mb_of(&router.memory_report())
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

    fn check(&self, result: &mut KiCadBoardRouterResult, directory: &Path) -> Result<(), String> {
        check_attempt(result, directory, self.board_id)
    }

    fn starved(&self, directory: &Path) -> usize {
        starved_thermals(directory)
    }

    fn memory_limits(&self) -> MemoryLimits {
        MemoryLimits::read(self.memory_mb)
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
    // once, their work spread over the seeds (the ladder counts the kept
    // seed's work once a seed).
    let (_held, statics) = core::router::hold_static_maps(board, config);
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
    let mut best = scored
        .into_iter()
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .expect("at least one seed")
        .2;
    best.build_work += statics / seeds.len() as f64;
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
        router_config.negotiation_expansions =
            (seconds * core::router::expansions_per_second()) as u64;
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
        router_config.negotiation_expansions =
            (seconds * core::router::expansions_per_second()) as u64;
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
        lowering_work_seconds: 0.0,
        wall_accounting: None,
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
/// `check_work`).
pub(crate) fn repair_necks(
    result: &mut KiCadBoardRouterResult,
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
) -> Result<usize, String> {
    if config.skip_native_verification {
        return Ok(0);
    }
    let calls = crate::drc_cache_counts().calls;
    let mut fills = 0;
    if let Some(repair) = crate::necks::repair_connection_width(directory, board_id, config)? {
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

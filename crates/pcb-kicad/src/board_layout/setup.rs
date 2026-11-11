// Copyright (C) 2026 Toit contributors.

//! What a layout reads from its start to its end (`LayoutContext`) and the
//! work it has booked (`WorkLedger`), both set up before the placement.

use super::*;
use std::time::Instant;

/// The layout's wall-clock guard: it looks at the router configuration's
/// deadline.
pub(super) type LayoutStop = WallStop<Box<dyn Fn() -> bool>>;

/// What every stage of a layout reads: the inputs, the router
/// configuration of its routes, the cost of a KiCad check, the end of its
/// work and its wall-clock guard.
pub(super) struct LayoutContext<'a> {
    pub source_directory: &'a Path,
    pub board_id: &'a str,
    pub output_directory: &'a Path,
    pub config: &'a KiCadBoardLayoutConfig,
    /// The router configuration of every route of the layout: the net
    /// classes' rules, the wall-clock deadline, the work of a KiCad check
    /// and the pairs' widths; from the race on also the lines at the pin
    /// rows where the race took them (`pin_lines`, set by the flow).
    pub router_config: KiCadBoardRouterConfig,
    /// The work one KiCad check counts (`check_work`; 0 without checks).
    pub check_seconds: f64,
    /// The layout's end in work, its nominal end (`total_seconds`).
    pub work_deadline: f64,
    /// When the layout started (its wall clock).
    pub started: Instant,
    pub stop: LayoutStop,
    /// `placed/` and the placed board in it.
    pub placed_directory: PathBuf,
    pub placed_board: PathBuf,
    /// The first route's seeds (the router setting `seeds`, at least 1).
    pub seeds: u64,
}

impl LayoutContext<'_> {
    /// The wall clock since the layout started, in seconds.
    pub(super) fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Whether the wall clock has passed the layout's deadline.
    pub(super) fn wall_passed(&self) -> bool {
        self.router_config
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    /// Every router of the layout stops where the layout's work ends: its
    /// own work so far plus what is left of the layout's after `work`.
    pub(super) fn arm(&self, router: &mut core::router::Router, work: f64) {
        let own = router.work();
        router.config_mut().deadline_work = Some(own + (self.work_deadline - work).max(0.0));
    }

    /// The project file beside the source board.
    pub(super) fn project_file(&self) -> PathBuf {
        self.source_directory
            .join(format!("{}.kicad_pro", self.board_id))
    }
}

/// The work a layout has booked, in seconds of an idle machine, and the
/// KiCad checks it counted.
pub(super) struct WorkLedger {
    /// The budget counts work (placement work and search expansions), so
    /// that a busy or a slower machine lays the board out the same way.
    pub work: f64,
    /// The placement's work counts its longest seed (the budget's
    /// measure); its seeds run in a pool, and the clock sees them as the
    /// reference machine runs them (`pool_work_seconds`, from the seeds'
    /// work alone): the difference counts toward the deadline in work
    /// (`work_deadline`), not toward the budgets (video: 193 s of placement
    /// work, 630 s of wall clock in waves; its first route ran into the
    /// wall-clock deadline at a point that depended on the load). Fixed
    /// once the placement ends.
    pub pool_excess: f64,
    /// KiCad's checks inside the layout, each at `check_seconds`.
    pub checks: crate::check_work::CheckTally,
}

impl WorkLedger {
    /// The work toward the deadline: the work and the pool's excess.
    pub(super) fn clock(&self) -> f64 {
        self.work + self.pool_excess
    }

    /// What is left before `deadline` (the layout's end in work).
    pub(super) fn left(&self, deadline: f64) -> f64 {
        deadline - self.work - self.pool_excess
    }
}

/// Checks the output directory and the settings, creates the directory and
/// sets up what every stage reads: the router configuration (net classes,
/// the wall-clock deadline, a KiCad check's work, the pairs), the work's
/// end and the wall-clock guard. Hands on the context and an empty ledger.
pub(super) fn prepare<'a>(
    source_directory: &'a Path,
    board_id: &'a str,
    output_directory: &'a Path,
    config: &'a KiCadBoardLayoutConfig,
    router_config: &KiCadBoardRouterConfig,
) -> Result<(LayoutContext<'a>, WorkLedger), String> {
    if output_directory.exists() {
        return Err(format!(
            "output directory {} already exists",
            output_directory.display()
        ));
    }
    // The search over sizes is the command's (each size a layout of its
    // own): a layout at one size has none.
    if config.free_dimension.is_some() {
        return Err(crate::exit::input_error(
            "free_dimension: the layout-kicad-board command searches the sizes; a single layout takes none",
        ));
    }
    fs::create_dir_all(output_directory)
        .map_err(|error| format!("failed to create {}: {error}", output_directory.display()))?;
    // The budget counts work (`WorkLedger`); the wall clock only guards
    // against running far past it.
    let started = Instant::now();
    // Requested net classes set their nets' rules (and the spacing).
    let with_classes;
    let router_config = if router_config.net_classes.is_empty() {
        router_config
    } else {
        let board = source_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board)
            .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        with_classes = crate::net_classes::with_net_classes(router_config, &parse(&text)?)
            .map_err(crate::exit::input_error)?;
        &with_classes
    };
    let check_seconds = kicad_check_seconds(source_directory, board_id, router_config)?;
    let checks = crate::check_work::CheckTally::new(check_seconds);
    // Every route of the layout ends by a wall-clock guard past its total
    // budget: on a much busier or slower machine the work would run on
    // past what the agent waits for.
    // (None under `PCB_ROUTER_NO_WALL_GUARD`: the work alone decides.)
    // Stretched where a pinned rate is faster than the machine
    // (`KiCadWorkRate::wall_stretch`, the search's or the placer's).
    let guard_seconds = wall_guard_seconds(config.total_seconds, config.wall_guard)?
        * crate::work_rate::work_rate().wall_stretch(true);
    let with_deadline = KiCadBoardRouterConfig {
        deadline: router_config.deadline.or(core::router::wall_guard()
            .then(|| started + std::time::Duration::from_secs_f64(guard_seconds))),
        kicad_check_work: Some(check_seconds),
        ..router_config.clone()
    };
    // Differential pairs (`diff_pairs`): every route of the layout plans
    // them (`lower_for_route`: the outline's sizing, the first route, the
    // placement race's probes, the moves) and the board it writes finishes
    // them (`finish_pairs`), as route mode's attempts do, so that the
    // board kept is a coupled attempt whichever stage made it; the pairs'
    // nets route at the pairs' width (`with_pairs`). Until 2026-11-04 only
    // the final ladder planned them, and where it did not run or its
    // attempt was not kept the first route stood uncoupled (jetson on both
    // seeds, ducktop2's left_io on seed 1).
    let router_config = if crate::board_router::pairs_planned(&with_deadline) {
        crate::board_router::with_pairs(&with_deadline)
    } else {
        with_deadline
    };
    // The layout's end in work, its nominal end: every router of the
    // layout stops there (`arm`), the polish and the final ladder are
    // planned up to it, so that where a run stops does not depend on the
    // machine's load. The wall-clock deadline `wall_guard` times it is the
    // margin for load (and for what the work does not count); a run that
    // reaches it first says so (`cut`). With both at 1.1 times the budget,
    // the wall clock decided every crowded board under a sweep's load
    // (video: work 1373 s of 1650 when the deadline cut its repair).
    let work_deadline = config.total_seconds.max(0.0);
    let deadline = router_config.deadline;
    let stop: LayoutStop = WallStop::new(Box::new(move || {
        deadline.is_some_and(|deadline| Instant::now() >= deadline)
    }));
    let placed_directory = output_directory.join("placed");
    let placed_board = placed_directory.join(format!("{board_id}.kicad_pcb"));
    let seeds = router_config.seeds.unwrap_or(1).max(1) as u64;
    Ok((
        LayoutContext {
            source_directory,
            board_id,
            output_directory,
            config,
            router_config,
            check_seconds,
            work_deadline,
            started,
            stop,
            placed_directory,
            placed_board,
            seeds,
        },
        WorkLedger {
            work: 0.0,
            pool_excess: 0.0,
            checks,
        },
    ))
}

/// The work one KiCad check inside the layout counts: the router config's
/// `kicad_check_work`, else modelled from the source board (`check_work`);
/// 0 where no check runs.
fn kicad_check_seconds(
    source_directory: &Path,
    board_id: &str,
    router_config: &KiCadBoardRouterConfig,
) -> Result<f64, String> {
    // KiCad's checks inside the layout (the board kept, the ladder's
    // attempts, the neck repair) count as work at a cost modelled from the
    // board alone (`check_work`), the same whether the DRC cache answers
    // a check or not: 10 to 50 s of kicad-cli each when uncached, which the
    // wall-clock guard used to see and the work did not.
    Ok(if router_config.skip_native_verification {
        0.0
    } else {
        match router_config.kicad_check_work {
            Some(seconds) => seconds,
            None => {
                let board = source_directory.join(format!("{board_id}.kicad_pcb"));
                let text = fs::read_to_string(&board)
                    .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
                let inputs = crate::check_work::check_inputs(&parse(&text)?);
                // Modelled on the reference: on a machine the work rates
                // find slower, a check takes longer by as much.
                let seconds =
                    crate::check_work::check_seconds(inputs) * core::router::reference_time_scale();
                eprintln!(
                    "layout: a KiCad check counts {seconds:.1} s of work ({} hatched zones, {:.0} hatch holes, {} Edge.Cuts items)",
                    inputs.hatched_zones, inputs.hatch_holes, inputs.edge_items
                );
                seconds
            }
        }
    })
}

// Copyright (C) 2026 Toit contributors.

//! The layout's report: `board-layout.json` and the closing line of the
//! log.

use super::candidates::Kept;
use super::race::RaceOutcome;
use super::setup::{LayoutContext, WorkLedger};
use super::*;

/// The first route as the report has it.
pub(super) struct FirstRoute {
    /// Its wall clock, from the lowering of the kept placement on.
    pub seconds: f64,
    /// The work up to its end (the seed pool's excess included).
    pub work_seconds: f64,
    pub open: usize,
    pub vias: usize,
    pub length_mm: f64,
}

impl FirstRoute {
    /// The first route `result` of open terminals `open`, ended at
    /// `ledger`'s work, started at `started`.
    pub(super) fn measure(
        result: &core::RoutingResult,
        open: usize,
        started: std::time::Instant,
        ledger: &WorkLedger,
    ) -> Self {
        Self {
            seconds: started.elapsed().as_secs_f64(),
            work_seconds: ledger.clock(),
            open,
            vias: result
                .routes
                .iter()
                .map(|route| route.vias.len())
                .sum::<usize>(),
            length_mm: result
                .routes
                .iter()
                .flat_map(|route| route.segments.iter())
                .map(|segment| distance_squared(segment.start, segment.end).sqrt())
                .sum::<f64>(),
        }
    }
}

/// Everything the stages hand the report besides the context and the
/// ledger.
pub(super) struct ReportParts<'a> {
    pub placement: &'a KiCadBoardPlacerResult,
    pub kept: &'a Kept,
    pub outline_sizing: Vec<KiCadOutlineTrial>,
    pub pin_swaps: Option<KiCadPinSwapResult>,
    pub placement_digest: String,
    pub race: RaceOutcome,
    pub first_route: FirstRoute,
    pub moves: Vec<KiCadBoardLayoutMove>,
    pub move_descent: Option<crate::move_descent::KiCadMoveDescentReport>,
    pub labels: crate::labels::KiCadLabelReport,
    /// The work up to the final ladder.
    pub layout_work: f64,
    pub final_ladder_budget_seconds: Option<f64>,
    pub routed: KiCadBoardRouterResult,
}

/// The layout's result assembled from the stages' parts (the lines the
/// race decided noted on the board kept, which had them from the router
/// config), written to `board-layout.json` and summed up in the log.
pub(super) fn finish_report(
    ctx: &LayoutContext,
    ledger: &WorkLedger,
    parts: ReportParts,
) -> Result<KiCadBoardLayoutResult, String> {
    let ReportParts {
        placement,
        kept,
        outline_sizing,
        pin_swaps,
        placement_digest,
        race,
        first_route,
        moves,
        move_descent,
        labels,
        layout_work,
        final_ladder_budget_seconds,
        mut routed,
    } = parts;
    let config = ctx.config;
    let problem = &kept.problem;
    // Lines the race decided: the board kept (the layout's own or the
    // final ladder's, which had them from the router config) says why.
    if let Some(why) = &race.lines_why {
        routed.pin_lines = Some(why.clone());
    }
    let routed_work = routed.work_seconds;
    let kicad_check_work_seconds = ledger.checks.work();
    let layout = KiCadBoardLayoutResult {
        board_id: ctx.board_id.into(),
        work_seconds: layout_work,
        kicad_check_seconds: ctx.check_seconds,
        kicad_checks_charged: ledger.checks.checks,
        kicad_check_work_seconds,
        placement_seconds: placement.seconds,
        first_route_seconds: first_route.seconds,
        first_route_work_seconds: first_route.work_seconds,
        total_seconds: config.total_seconds,
        ledger_wait_seconds: (crate::ledger::waited_seconds() * 10.0).round() / 10.0,
        first_unconnected_terminals: first_route.open,
        first_vias: first_route.vias,
        first_length_mm: first_route.length_mm,
        stopped_after_first_route: config.stop_after_first_route,
        placement_digest,
        final_ladder_budget_seconds,
        pin_lines: race.lines_why,
        moves,
        move_descent,
        outline_sizing,
        placement_race: race.placement_race,
        congestion: race.congestion,
        race: race.report,
        pad_exits: race.pad_exits,
        labels,
        quality: crate::quality::score_kicad_board(
            &ctx.output_directory.join("result"),
            ctx.board_id,
        )
        .ok(),
        pin_swaps,
        constraints: constraint_report(
            &problem.problem,
            &problem.problem.poses,
            &problem.references,
        )
        .into_iter()
        .chain(crate::board_placer::pair_row_statuses(
            &problem.pair_rows,
            &problem.problem,
            &problem.problem.poses,
            &problem.references,
        ))
        .collect(),
        routed,
        cut: core::router::wall_cuts(),
    };
    let report_path = ctx.output_directory.join("board-layout.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&layout).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    log_done(ctx, ledger, &layout, routed_work, kicad_check_work_seconds);
    Ok(layout)
}

/// The layout's closing lines: its wall clock and work, the work rates,
/// the ledger's wait, KiCad's checks and what the deadline or
/// `stop_after_first_route` skipped.
fn log_done(
    ctx: &LayoutContext,
    ledger: &WorkLedger,
    layout: &KiCadBoardLayoutResult,
    routed_work: f64,
    kicad_check_work_seconds: f64,
) {
    let work = ledger.work;
    let check_seconds = ctx.check_seconds;
    let stop = &ctx.stop;
    let drc_cache = crate::drc_cache_counts();
    eprintln!(
        "layout: done ({:.0} s, work {work:.0} s, {:.0} s in all; work over wall {} at {:.4e} expansions/s, {:.4e} placer units/s, {}{}); KiCad checks {} ({} from the DRC cache, {} of the neck repair), {} skipped by the router's count; check work {} x {check_seconds:.1} s = {kicad_check_work_seconds:.0} s{}",
        ctx.elapsed(),
        work + routed_work,
        crate::verdict::work_wall_ratio(Some(work + routed_work), ctx.elapsed())
            .map_or("-".to_string(), |ratio| format!("{ratio:.2}")),
        core::router::expansions_per_second(),
        placer::work_per_second(),
        crate::work_rate::work_rate().source,
        if layout.ledger_wait_seconds > 0.0 {
            format!(
                "; {:.0} s of the wall clock waiting in the memory ledger",
                layout.ledger_wait_seconds
            )
        } else {
            String::new()
        },
        drc_cache.calls,
        drc_cache.hits,
        crate::board_router::neck_checks(),
        crate::board_router::checks_skipped(),
        ledger.checks.checks,
        if stop.fired() {
            format!(
                "; past the deadline: {}",
                stop.skipped()
                    .iter()
                    .map(|step| format!("{step:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        }
    );
    if ctx.config.stop_after_first_route {
        eprintln!(
            "layout: stopped after the first route: {} skipped",
            stop.ended_skipped()
                .iter()
                .map(|step| format!("{step:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

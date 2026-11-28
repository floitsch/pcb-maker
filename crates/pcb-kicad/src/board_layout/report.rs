// Copyright (C) 2026 Toit contributors.

//! The layout's report: `board-layout.json` and the closing line of the
//! log.

use super::race::RaceOutcome;
use super::setup::{LayoutContext, WorkLedger};
use super::*;
use sha2::{Digest, Sha256};

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
    pub problem: &'a crate::board_placer::LoweredPlacement,
    pub relaxation: &'a placer::Relaxation,
    pub outline_sizing: Vec<KiCadOutlineTrial>,
    pub pin_swaps: Option<KiCadPinSwapResult>,
    pub placement_digest: String,
    pub race: RaceOutcome,
    pub first_route: FirstRoute,
    pub moves: Vec<KiCadBoardLayoutMove>,
    pub labels: crate::labels::KiCadLabelReport,
    /// The work up to the final ladder.
    pub layout_work: f64,
    pub final_ladder_budget_seconds: Option<f64>,
    pub routed: KiCadBoardRouterResult,
    pub route_reuse_placement: Option<KiCadRouteReusePlacement>,
    pub prepublished_fill: Option<crate::FillPublication>,
}

/// The layout's result assembled from the stages' parts (the lines the
/// race decided noted on the board kept, which had them from the router
/// config), written to `board-layout.json` and summed up in the log.
pub(super) fn finish_report(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    parts: ReportParts,
) -> Result<KiCadBoardLayoutResult, String> {
    let ReportParts {
        placement,
        problem,
        relaxation,
        outline_sizing,
        pin_swaps,
        placement_digest,
        race,
        first_route,
        moves,
        labels,
        layout_work,
        final_ladder_budget_seconds,
        mut routed,
        route_reuse_placement,
        mut prepublished_fill,
    } = parts;
    let config = ctx.config;
    let work_before_finish = ledger.work;
    let plane_escape = recover_plane_escape(ctx, ledger, &mut routed)?;
    // The repairs from KiCad's report (`tuning.post_route_repairs`): each
    // stage returns None at once with them off.
    let mut local_open_repair = if prepublished_fill.is_none() {
        super::additive_repair::recover(ctx, ledger, &mut routed)?
    } else {
        None
    };
    let via_detour = if prepublished_fill.is_none() {
        super::via_detour::recover(ctx, ledger, &mut routed)?
    } else {
        None
    };
    let terminal_launch_retry =
        recover_terminal_launches(ctx, ledger, &mut routed, &mut prepublished_fill)?;
    // Lines the race decided: the board kept (the layout's own or the
    // final ladder's, which had them from the router config) says why.
    if let Some(why) = &race.lines_why {
        routed.pin_lines = Some(why.clone());
    }
    let reserve =
        config.plane_escape_budget.unwrap_or(0.0) + config.terminal_launch_budget.unwrap_or(0.0);
    let fill_deadline = ctx.work_deadline + reserve;
    let mut fill_publication = prepublished_fill.or_else(|| {
        publish_fill(
            ctx,
            ledger,
            &routed,
            fill_deadline,
            &ctx.output_directory.join("result"),
            None,
        )
    });
    if let Some(fill) = &fill_publication {
        ledger.work += ledger.checks.add(fill.native_checks_charged);
    }
    if let Some(repair) = &mut local_open_repair {
        super::additive_repair::finalize_retry(
            ctx,
            ledger,
            repair,
            fill_publication.as_ref(),
            fill_deadline,
        );
    }
    let generated_pad_via_recovery =
        super::generated_pad_via::recover(ctx, ledger, &mut routed, &mut fill_publication)?;
    let unchanged_hatch_width_repair =
        super::unchanged_hatch::recover(ctx, ledger, &mut routed, &mut fill_publication)?;
    let mut constraints = constraints(problem, relaxation);
    let body_near_grid_retry = super::near_copper::recover(
        ctx,
        ledger,
        problem,
        &mut constraints,
        &mut routed,
        &mut fill_publication,
    )?;
    let moved_after_routing = body_near_grid_retry
        .as_ref()
        .is_some_and(|retry| retry.status == "accepted");
    let prepared_row_escape_advice = if ctx.router_config.prepared_row_escape_pads.is_empty()
        && routed
            .native
            .as_ref()
            .is_some_and(|native| (2..=4).contains(&native.selected_net_unconnected_items))
        && !ctx.wall_passed()
    {
        let result = ctx.output_directory.join("result");
        native_report::read_kicad_report(&result.join("drc.json"))
            .ok()
            .zip(
                fs::read_to_string(result.join(format!("{}.kicad_pcb", ctx.board_id)))
                    .ok()
                    .and_then(|text| crate::parse(&text).ok()),
            )
            .and_then(|(drc, pcb)| {
                pcb_kicad_route::open_repair::prepared_row_escape_advice(
                    &pcb,
                    &drc,
                    &ctx.router_config,
                )
            })
    } else {
        None
    };
    let terminal_launch_advice = if prepared_row_escape_advice.is_none()
        && config.terminal_launch_budget.is_none()
        && config.plane_escape_budget.is_none()
        && config.reuse_routes.is_none()
        && !config.stop_after_first_route
        && (config.save_zone_fills || fill_publication.is_none())
        && !ctx.wall_passed()
        && !ctx.router_config.skip_native_verification
        && ctx.router_config.tuning.engine.as_deref() != Some("topological")
        && let Some(native) = routed.native.as_ref()
    {
        pcb_kicad_route::open_repair::advise_terminal_launches(
            &ctx.placed_board,
            &ctx.output_directory
                .join("result")
                .join(format!("{}.kicad_pcb", ctx.board_id)),
            native,
            &routed.diagnostics.dead_pads,
            &ctx.router_config,
            ctx.check_seconds,
        )
    } else {
        None
    };
    let routed_work = routed.work_seconds;
    let kicad_check_work_seconds = ledger.checks.work();
    let layout = KiCadBoardLayoutResult {
        board_id: ctx.board_id.into(),
        work_seconds: layout_work + (ledger.work - work_before_finish),
        kicad_check_seconds: ctx.check_seconds,
        kicad_checks_charged: ledger.checks.checks,
        kicad_check_work_seconds,
        fill_publication,
        local_open_repair,
        generated_pad_via_recovery,
        body_near_grid_retry,
        unchanged_hatch_width_repair,
        via_detour,
        plane_escape,
        plane_escape_reserved_seconds: config.plane_escape_budget,
        terminal_launch_retry,
        terminal_launch_advice,
        prepared_row_escape_advice,
        placement_seconds: placement.seconds,
        first_route_seconds: first_route.seconds,
        first_route_work_seconds: first_route.work_seconds,
        total_seconds: config.total_seconds + reserve,
        ledger_wait_seconds: (crate::ledger::waited_seconds() * 10.0).round() / 10.0,
        first_unconnected_terminals: first_route.open,
        first_vias: first_route.vias,
        first_length_mm: first_route.length_mm,
        stopped_after_first_route: config.stop_after_first_route,
        placement_digest,
        final_ladder_budget_seconds,
        pin_lines: race.lines_why,
        moves,
        outline_sizing,
        placement_race: race.placement_race,
        congestion: race.congestion,
        race: race.report,
        pad_exits: if moved_after_routing {
            None
        } else {
            race.pad_exits
        },
        labels,
        quality: crate::quality::score_kicad_board(
            &ctx.output_directory.join("result"),
            ctx.board_id,
        )
        .ok(),
        pin_swaps,
        constraints,
        route_reuse_placement,
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

fn recover_plane_escape(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    routed: &mut KiCadBoardRouterResult,
) -> Result<Option<crate::PlaneEscapeRepair>, String> {
    let Some(budget) = ctx.config.plane_escape_budget else {
        return Ok(None);
    };
    let skipped = |reason, detail| {
        Ok(Some(crate::PlaneEscapeRepair::skipped(
            budget, reason, detail,
        )))
    };
    if ctx.config.stop_after_first_route {
        return skipped("first_route_measurement", None);
    }
    if ctx.wall_passed() {
        return skipped("wall_guard_expired", None);
    }
    if routed.native.as_ref().is_none_or(|report| {
        report.selected_net_unconnected_items != 1 || report.schematic_parity_issues != 0
    }) {
        return skipped("requires_one_native_open_and_clean_parity", None);
    }
    let directory = ctx.output_directory.join("result");
    let trial = ctx.output_directory.join("plane-escape");
    let mut config = ctx.router_config.clone();
    // The first pass can end with unused work; the explicit retry cap
    // alone controls the reroute's ladder recipe.
    let ordinary_left = ledger.left(ctx.work_deadline) - routed.work_seconds;
    let allowance = budget + ordinary_left.min(0.0);
    if allowance <= 0.0 {
        return skipped("ordinary_layout_used_repair_reserve", None);
    }
    config.ladder_budget_seconds = Some(allowance);
    config.diff_pairs_by_need = true;
    let report = crate::repair_kicad_plane_escape(
        &directory,
        &ctx.placed_directory,
        ctx.board_id,
        &trial,
        &config,
    );
    let mut report = match report {
        Ok(report) => report,
        Err(error) => {
            eprintln!("plane escape skipped: {error}");
            return skipped("repair_input_or_check_failed", Some(error));
        }
    };
    ledger
        .checks
        .add(report.native_checks + report.route_native_checks_charged);
    ledger.work += report.work_seconds - report.route_work_seconds;
    let counted_routes = routed.work_seconds + report.route_work_seconds;
    if let Some(mut recovered) = report.routed.take() {
        if let Some(reuse) = routed.route_reuse.clone() {
            super::reuse::write_evidence(&trial, &mut recovered, reuse)?;
        }
        let previous = crate::fs_util::temporary_next_to(&directory);
        fs::rename(&directory, &previous).map_err(|error| error.to_string())?;
        if let Err(error) = fs::rename(&trial, &directory) {
            fs::rename(&previous, &directory).map_err(|restore| {
                format!("could not adopt verified plane escape: {error}; restore: {restore}")
            })?;
            return Err(format!("could not adopt verified plane escape: {error}"));
        }
        let _ = fs::remove_dir_all(previous);
        *routed = recovered;
    }
    routed.work_seconds = counted_routes;
    eprintln!(
        "plane escape: {} ({}), {} to {} native opens with refill, {:.2} modeled seconds of {:.2}",
        report.status,
        report.reason,
        report.opens_before,
        report.opens_after,
        report.work_seconds,
        report.budget_seconds
    );
    Ok(Some(report))
}

fn recover_terminal_launches(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    routed: &mut KiCadBoardRouterResult,
    prepublished_fill: &mut Option<crate::FillPublication>,
) -> Result<Option<pcb_kicad_route::open_repair::TerminalLaunchRetry>, String> {
    use pcb_kicad_route::open_repair::{TerminalLaunchRetry, retry_terminal_launches};

    let Some(reserve) = ctx.config.terminal_launch_budget else {
        return Ok(None);
    };
    if ctx.config.stop_after_first_route || ctx.wall_passed() || prepublished_fill.is_some() {
        return Ok(Some(TerminalLaunchRetry::skipped(
            reserve,
            "ordinary_layout_only",
            &ctx.config.terminal_launch_pads,
        )));
    }
    let selected = ctx.output_directory.join("result");
    let output = ctx.output_directory.join("terminal-launch");
    let ordinary_left = ledger.left(ctx.work_deadline) - routed.work_seconds;
    let fill_reserve = if ctx.config.save_zone_fills || !ctx.config.terminal_launch_pads.is_empty()
    {
        2.0 * ctx.check_seconds
    } else {
        0.0
    };
    let available = (reserve + ordinary_left.min(0.0) - fill_reserve).max(0.0);
    let mut repair = retry_terminal_launches(
        &selected,
        &ctx.placed_board,
        &output,
        ctx.board_id,
        routed,
        &ctx.router_config,
        available,
        ctx.check_seconds,
        &ctx.config.terminal_launch_pads,
    );
    ledger
        .checks
        .add(repair.native_checks + repair.reroute_native_checks);
    ledger.work += repair.work_seconds;
    let mut verified_fill = None;
    let mut accepted = repair.accepted.take();
    if let Some((candidate, _)) = &accepted
        && !ctx.config.terminal_launch_pads.is_empty()
    {
        match verify_terminal_launch_saved_board(ctx, ledger, &selected, candidate, &output) {
            Ok(fill) => verified_fill = fill,
            Err(reason) => {
                repair.status = "no_improvement";
                repair.reason = format!("selected_tip_retry_saved_board_not_verified: {reason}");
                repair.opens_after = repair.opens_before;
                accepted = None;
            }
        }
    }
    if let Some((candidate, mut recovered)) = accepted {
        // Retry work is already booked above. Keep the ordinary route's
        // work for the layout's existing fill and finishing accounting.
        recovered.work_seconds = routed.work_seconds;
        write_typed_json(&candidate.join("board-router.json"), &recovered)?;
        let previous = crate::fs_util::temporary_next_to(&selected);
        fs::rename(&selected, &previous).map_err(|error| error.to_string())?;
        if let Err(error) = fs::rename(&candidate, &selected) {
            fs::rename(&previous, &selected).map_err(|restore| {
                format!("could not adopt terminal launch retry: {error}; restore: {restore}")
            })?;
            return Err(format!("could not adopt terminal launch retry: {error}"));
        }
        let _ = fs::remove_dir_all(previous);
        let previous_repair_report = candidate.join("open-repair.json");
        if let Ok(relative) = previous_repair_report.strip_prefix(ctx.output_directory)
            && let Some(trial) = repair
                .local_trials
                .iter_mut()
                .find(|trial| trial.report == relative)
        {
            trial.report = PathBuf::from("result/open-repair.json");
        }
        *routed = recovered;
        if verified_fill.is_some() {
            *prepublished_fill = verified_fill;
        }
    }
    eprintln!(
        "layout: terminal launch retry {} ({}), {} to {} native opens, {:.2} of {:.2} modeled seconds",
        repair.status,
        repair.reason,
        repair.opens_before,
        repair.opens_after,
        repair.work_seconds,
        repair.budget_seconds
    );
    if output.is_dir() {
        write_typed_json(&output.join("terminal-launch.json"), &repair)?;
    }
    Ok(Some(repair))
}

fn verify_terminal_launch_saved_board(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    selected: &Path,
    candidate: &Path,
    trial: &Path,
) -> Result<Option<crate::FillPublication>, String> {
    use crate::fill_handoff::layout::{FillRefillEvidence, project_inputs};
    use crate::fill_handoff::{compare_board_reports, has_fillable_zones, publish_layout_fill};
    use pcb_kicad_check::native_report;

    if project_inputs(candidate)? != project_inputs(selected)? {
        return Err("the reroute changed the project inputs".into());
    }
    let board = candidate.join(format!("{}.kicad_pcb", ctx.board_id));
    let text = fs::read_to_string(&board).map_err(|error| error.to_string())?;
    let pcb = parse(&text)?;
    let refill = native_report::read_kicad_report(&trial.join("drc-after-launch.json"))?;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let expired = || ctx.wall_passed() || std::time::Instant::now() >= end;
    if !has_fillable_zones(&pcb) {
        ledger.work += ledger.checks.add(1);
        let stored = native_report::check_kept_route_input(
            &board,
            &trial.join("drc-after-launch-stored.json"),
            true,
            false,
            expired,
        )?;
        compare_board_reports(&pcb, &refill, &refill, &stored)?;
        return Ok(None);
    }
    if !ctx.config.save_zone_fills {
        return Err("save_zone_fills is disabled for this layout".into());
    }
    let fill = publish_layout_fill(
        candidate,
        ctx.board_id,
        pcb,
        format!("{:x}", Sha256::digest(text.as_bytes())),
        FillRefillEvidence::CompletedRoute(&refill),
        false,
        expired,
    );
    if fill.status == "saved" {
        Ok(Some(fill))
    } else {
        ledger.work += ledger.checks.add(fill.native_checks_charged);
        Err(fill
            .reason
            .unwrap_or_else(|| "verification was deferred".into()))
    }
}

pub(super) fn publish_fill(
    ctx: &LayoutContext,
    ledger: &WorkLedger,
    routed: &KiCadBoardRouterResult,
    fill_deadline: f64,
    project: &Path,
    reused_refill: Option<&pcb_kicad_check::verification::CheckedRefill>,
) -> Option<crate::FillPublication> {
    use crate::fill_handoff::layout::FillRefillEvidence;
    use crate::fill_handoff::{FillPublication, has_fillable_zones, publish_layout_fill};
    let board = project.join(format!("{}.kicad_pcb", ctx.board_id));
    let text = match fs::read_to_string(&board) {
        Ok(text) => text,
        Err(error) => {
            return Some(FillPublication::skipped(format!(
                "read selected PCB: {error}"
            )));
        }
    };
    let pcb = match parse(&text) {
        Ok(pcb) => pcb,
        Err(error) => return Some(FillPublication::skipped(error)),
    };
    if !has_fillable_zones(&pcb) {
        return None;
    }
    if !ctx.config.save_zone_fills {
        return Some(FillPublication::skipped(
            "save_zone_fills is disabled for this layout",
        ));
    }
    if ctx.config.stop_after_first_route {
        return Some(FillPublication::skipped(
            "the first-route measurement ends before fill publication",
        ));
    }
    if routed.native.is_none() {
        return Some(FillPublication::skipped(
            "the selected board has no native verification",
        ));
    }
    let remaining_work = ledger.left(fill_deadline) - routed.work_seconds;
    let current_layout_refill = routed
        .checked_layout_refill
        .as_ref()
        .filter(|receipt| receipt.from_ordinary_layout() && receipt.matches(&board));
    let checks_needed = if current_layout_refill.is_some() {
        1.0
    } else {
        2.0
    };
    let retry = || {
        vec![
            "pcb-maker".into(),
            "drc-kicad-board".into(),
            ctx.output_directory.join("result").display().to_string(),
            ctx.board_id.into(),
            ctx.output_directory
                .join("filled-drc.json")
                .display()
                .to_string(),
            "--save-filled".into(),
            ctx.output_directory.join("filled").display().to_string(),
        ]
    };
    if remaining_work < checks_needed * ctx.check_seconds || ctx.wall_passed() {
        let reason = if routed.checked_layout_refill.is_some() && current_layout_refill.is_none() {
            "the selected board or project changed after its native refill, and the remaining work did not cover a fresh refill and reopen"
        } else if current_layout_refill.is_some() {
            "the layout's remaining budget did not cover reopening the checked fill"
        } else {
            "the layout's remaining budget did not cover refill and stored-board verification"
        };
        let mut fill = FillPublication::skipped(reason);
        fill.retry_argv = retry();
        return Some(fill);
    }
    if pcb_kicad_file::check_repairs::needs_repair(pcb.clone()) {
        return Some(FillPublication::skipped(
            "KiCad's routing verifier must first repair the refill input",
        ));
    }
    let source_hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    let evidence = current_layout_refill.map_or(
        FillRefillEvidence::Layout(reused_refill),
        FillRefillEvidence::CheckedLayout,
    );
    let mut fill = publish_layout_fill(
        project,
        ctx.board_id,
        pcb,
        source_hash,
        evidence,
        remaining_work >= (checks_needed + 2.0) * ctx.check_seconds,
        || ctx.wall_passed(),
    );
    if current_layout_refill.is_some() && fill.status != "saved" {
        fill.retry_argv = retry();
    }
    Some(fill)
}

pub(super) fn constraints(
    problem: &crate::board_placer::LoweredPlacement,
    relaxation: &placer::Relaxation,
) -> Vec<KiCadConstraintStatus> {
    let mut statuses = crate::placement_constraints::constraint_report_at(
        problem,
        &problem.problem.poses,
        crate::placement_constraints::near_geometry_spacing(relaxation),
    );
    if !problem.pair_rows.is_empty() {
        let stored_poses: Vec<_> = problem
            .source_at
            .iter()
            .map(|at| placer::Pose {
                position: [at[0], at[1]],
                angle: at[2],
            })
            .collect();
        statuses.extend(crate::board_placer::pair_row_statuses(
            &problem.pair_rows,
            &problem.problem,
            &stored_poses,
            &problem.references,
        ));
    }
    statuses
}

pub(super) fn constraints_at(
    problem: &crate::board_placer::LoweredPlacement,
    poses: &[placer::Pose],
    diagnostic_spacing: Option<f64>,
) -> Vec<KiCadConstraintStatus> {
    crate::placement_constraints::constraint_report_at(problem, poses, diagnostic_spacing)
        .into_iter()
        .chain(crate::board_placer::pair_row_statuses(
            &problem.pair_rows,
            &problem.problem,
            poses,
            &problem.references,
        ))
        .collect()
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

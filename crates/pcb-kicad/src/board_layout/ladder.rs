// Copyright (C) 2026 Toit contributors.

//! After the layout's own board is written: the final ladder (route mode's
//! ladder on the final placement, or the narrow-signal attempt and the
//! attempt after it), the starved thermals made solid and the necks
//! repaired on the board kept.

use super::moves::FirstRouteJudgement;
use super::placement::{Copper, write_placement_without_copper};
use super::setup::{LayoutContext, WorkLedger};
use super::*;

/// The board kept as the final steps improve it.
struct KeptBoard {
    routed: KiCadBoardRouterResult,
    /// Whether starved pads were made solid on it (by a check, or by the
    /// model where no check runs).
    solidified: bool,
    /// Whether the board kept has had its try at solid pads (the try after
    /// the ladder runs only on a board the ladder kept).
    solid_tried: bool,
}

/// What is still open on a board in KiCad's eyes and the router's: open
/// terminals, the router's own violations and KiCad's unconnected items.
fn open(result: &KiCadBoardRouterResult) -> usize {
    result.unconnected_terminals
        + result.internal_violations.len()
        + result
            .native
            .as_ref()
            .map_or(0, |native| native.selected_net_unconnected_items)
}

/// A board's quality for the final steps: what is open (`open`), then the
/// thermals KiCad finds starved in `directory`'s check.
pub(super) fn quality(result: &KiCadBoardRouterResult, directory: &Path) -> (usize, usize) {
    (
        open(result),
        crate::board_router::starved_thermals(directory),
    )
}

/// Whether KiCad checked a ladder's board (or no check runs at all).
fn checked(ctx: &LayoutContext, result: &KiCadBoardRouterResult) -> bool {
    result.native.is_some() || ctx.router_config.skip_native_verification
}

/// The board kept as a ladder's attempts compare against it: the router's
/// count, vias and length, and its quality.
fn kept_standing(
    routed: &KiCadBoardRouterResult,
    quality: (usize, usize),
) -> ((usize, usize, f64), (usize, usize)) {
    (
        (
            crate::board_router::router_open(routed),
            routed.vias,
            routed.length_mm,
        ),
        quality,
    )
}

/// The steps after the layout's own board (`routed`, of quality
/// `current`, written from `pcb`): if something is still open (in KiCad's
/// eyes too), the final placement routed with route mode's whole ladder
/// (pours as tracks, finer pitches, more seeds) and the better board kept.
/// If only thermals are starved (pads KiCad does not count as joined to
/// their pour), the starved pads go solid first and, when that leaves any
/// starved or open, the pour nets are routed as tracks. Then the board
/// kept has its starved pads made solid and its necks repaired. Hands on
/// the board kept and the ladder's budget (where it ran).
pub(super) fn final_steps(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    pcb: &Expr,
    routed: KiCadBoardRouterResult,
    current: (usize, usize),
    judgement: &FirstRouteJudgement,
    copper: &Copper,
) -> Result<(KiCadBoardRouterResult, Option<f64>), String> {
    let router_config = &ctx.router_config;
    // Past the wall-clock guard the ladder would only lower the board and
    // ask KiCad again (link ended 106 s past it, 44 s before the
    // benchmark's limit).
    // A ladder needs a few minutes of wall clock for a probe, the board
    // and KiCad's check; started with less it runs past the harness's
    // limit (the laptop motherboard: killed at 1800 s without a board).
    let past_guard = ledger.left(ctx.work_deadline) < 240.0 || !ctx.stop.allows(LayoutStep::Ladder);
    // A board far from complete where the narrow-signal step applies gets
    // only the step: one attempt with the first route's pours, signals at
    // the project's narrowest width (and its minimum clearance where the
    // project rates clearance findings as warnings), with most of what is
    // left. The class-width ladder before it used to take that work and
    // leave the step too little or nothing, and none of its attempts beat
    // the first route on these boards (x833: module 82 unconnected, DSP
    // 121, vaio_re 138, jetson 25 without a ladder; the step alone 17, 82,
    // 35, 18).
    let direct_narrow = judgement.narrow_applies && judgement.many_open && current.0 > 0;
    // Only thermals starved (KiCad and the router find every connection):
    // the starved pads go solid first, one KiCad check, and the ladder
    // (pours as tracks) runs only when that leaves something starved or
    // open. The ladder first used to run whole on these boards and keep
    // none of its attempts (the 0xCB 1337 panel: 6 attempts, 596 s, each
    // left 1 or 2 open), and then the solid pads fixed the board anyway.
    let thermals_only = !past_guard
        && current.0 == 0
        && current.1 > 0
        && copper.connect
        && router_config.pours == KiCadPourMode::Auto;
    let mut board = KeptBoard {
        routed,
        solidified: false,
        solid_tried: false,
    };
    // The checks of the solid pads tried before the ladder (counted with
    // the board's work after it).
    let (current, solid_first_work) =
        solid_thermals_first(ctx, ledger, &mut board, current, thermals_only)?;
    let fallback_config = ladder_config(
        ctx,
        ledger,
        past_guard,
        direct_narrow,
        current,
        copper.connect,
    );
    let final_ladder_budget_seconds = fallback_config
        .as_ref()
        .map(|_| ledger.left(ctx.work_deadline).max(0.0));
    if let Some(fallback_config) = fallback_config {
        run_final_ladder(
            ctx,
            ledger,
            pcb,
            fallback_config,
            current,
            direct_narrow,
            &mut board,
        )?;
    }
    board.routed.work_seconds += solid_first_work;
    finish_kept_board(ctx, ledger, &mut board)?;
    Ok((board.routed, final_ladder_budget_seconds))
}

/// With only thermals starved (`thermals_only`), the starved pads made
/// solid before the ladder (one KiCad check each round), logged. Hands on
/// the board's quality after it and the work of its checks (counted, not
/// yet booked).
fn solid_thermals_first(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    board: &mut KeptBoard,
    current: (usize, usize),
    thermals_only: bool,
) -> Result<((usize, usize), f64), String> {
    if !(thermals_only
        && ctx.router_config.solid_starved_thermals != Some(false)
        && board.routed.native.is_some()
        && ctx.stop.allows(LayoutStep::NeckRepair))
    {
        return Ok((current, 0.0));
    }
    let result_directory = ctx.output_directory.join("result");
    let checks = crate::board_router::solidify_starved_thermals(
        &mut board.routed,
        &result_directory,
        ctx.board_id,
    )?;
    board.solidified = checks > 0;
    board.solid_tried = true;
    let solid_first_work = ledger.checks.add(checks);
    let before = current;
    let current = quality(&board.routed, &result_directory);
    eprintln!(
        "layout: only thermals starved: the starved pads solid first: {} open, {} starved ({} open, {} starved before){}",
        current.0,
        current.1,
        before.0,
        before.1,
        if current == (0, 0) {
            ""
        } else {
            "; the ladder next"
        }
    );
    Ok((current, solid_first_work))
}

/// The router configuration of the final ladder, or `None` where none
/// runs (past the guard, or nothing open or starved): with
/// `direct_narrow` the narrow-signal attempt alone with most of the work
/// left; with connections open the layout's own configuration; with only
/// thermals starved the pours as tracks.
fn ladder_config(
    ctx: &LayoutContext,
    ledger: &WorkLedger,
    past_guard: bool,
    direct_narrow: bool,
    current: (usize, usize),
    connect: bool,
) -> Option<KiCadBoardRouterConfig> {
    let router_config = &ctx.router_config;
    if past_guard {
        None
    } else if direct_narrow {
        let ladder_left = ledger.left(ctx.work_deadline);
        let mut narrow = crate::board_router::narrow_step_config(router_config);
        narrow.pours = if connect {
            KiCadPourMode::Connect
        } else {
            KiCadPourMode::Tracks
        };
        narrow.refine_pitches_mm = Some(Vec::new());
        let (negotiation, cleanup, keeps) = narrow_with_continuation(ladder_left);
        narrow.negotiation_seconds = Some(negotiation);
        narrow.cleanup_seconds = Some(cleanup);
        // It leaves the attempt after it its share (the other pour mode:
        // DSP-ADAU1452 73 to 30 open by it).
        narrow.keep_for_continuation = keeps;
        eprintln!(
            "layout: {ladder_left:.0} s of work left: the narrow-signal attempt instead of the ladder (pours {}, negotiation {negotiation:.0} s, clean-up {cleanup:.0} s)",
            if connect { "connected" } else { "as tracks" },
        );
        Some(narrow)
    } else if current.0 > 0 {
        Some(router_config.clone())
    } else if current.1 > 0 && connect && router_config.pours == KiCadPourMode::Auto {
        let mut tracks = router_config.clone();
        tracks.pours = KiCadPourMode::Tracks;
        Some(tracks)
    } else {
        None
    }
}

/// The final ladder from scratch on the final placement (`pcb` written to
/// `placed/` without copper; the start from the router's state,
/// `final_route_from_state`, was removed on 2026-11-04): route mode's
/// flow in `result-ladder/`, kept in `result/` where KiCad checked it and
/// finds it better than `current`; after the narrow-signal attempt run
/// alone, the attempt after it. Their work is the board's whether kept or
/// not.
fn run_final_ladder(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    pcb: &Expr,
    fallback_config: KiCadBoardRouterConfig,
    current: (usize, usize),
    direct_narrow: bool,
    board: &mut KeptBoard,
) -> Result<(), String> {
    let result_directory = ctx.output_directory.join("result");
    let fallback_directory = ctx.output_directory.join("result-ladder");
    // The placement file already carries the final poses.
    write_placement_without_copper(ctx, pcb)?;
    let mut fallback_config = fallback_config;
    let left = (ctx.config.total_seconds - ledger.work).max(120.0);
    // The work left, or less where the router config says so: until
    // 2026-10-21 at most route mode's default of 1200 s, so a layout
    // of 3000 s (the fence's two layers) ended its ladder with half
    // its budget unused.
    fallback_config.ladder_budget_seconds = Some(
        fallback_config
            .ladder_budget_seconds
            .unwrap_or(left)
            .min(left),
    );
    // The ladder decides whether another attempt fits in work, up to
    // the layout's nominal end.
    fallback_config.deadline_work = Some(ledger.left(ctx.work_deadline).max(0.0));
    // KiCad checks only the ladder's attempts that could beat the board
    // kept, at most the two best by the router's count, the board kept
    // among them (`CheckGate`).
    fallback_config.kicad_check_top = Some(KICAD_CHECK_TOP);
    // The starved pads go solid once, on the board the layout keeps
    // (below): its attempts are compared as KiCad found them.
    fallback_config.solid_starved_thermals = Some(false);
    // The work of the attempts after the layout's own board (the ladder
    // from scratch, the narrow-signal attempt and what follows it)
    // counts whether the layout keeps them or not.
    let mut after_work = 0.0;
    let mut best_quality = current;
    // The project the attempts route.
    let attempt_source = ctx.placed_directory.clone();
    let mut fallback_config = fallback_config.clone();
    fallback_config.kicad_kept = Some(kept_standing(&board.routed, best_quality));
    let fallback = route_kicad_board(
        &ctx.placed_directory,
        ctx.board_id,
        &fallback_directory,
        &fallback_config,
    )?;
    // (Its checks are in its work already.)
    ledger.checks.add(fallback.kicad_checks_charged);
    after_work += fallback.work_seconds;
    // The pour mode of the ladder's last attempt (the continuation varies
    // it).
    let fallback_pours = fallback.pours.clone();
    let fallback_quality = quality(&fallback, &fallback_directory);
    // A ladder board KiCad did not check (past the deadline, it could
    // not beat the board kept) is never kept.
    if checked(ctx, &fallback) && fallback_quality < best_quality {
        fs::remove_dir_all(&result_directory).map_err(|error| error.to_string())?;
        fs::rename(&fallback_directory, &result_directory).map_err(|error| error.to_string())?;
        board.routed = fallback;
        best_quality = fallback_quality;
        board.solid_tried = false;
    } else {
        fs::remove_dir_all(&fallback_directory).map_err(|error| error.to_string())?;
    }
    if direct_narrow {
        run_continuation(
            ctx,
            ledger,
            &fallback_config,
            &fallback_pours,
            &attempt_source,
            &mut after_work,
            best_quality,
            board,
        )?;
    }
    // Work spent is work spent: the ladder's (and the attempts after
    // the narrow-signal attempt) count whether the layout keeps their
    // board or not (koeg in v16: 410 s reported, its unkept ladder ran
    // 1100 s more of wall clock).
    board.routed.work_seconds = after_work;
    Ok(())
}

/// After the narrow-signal attempt run alone, the work it left (Hub ended
/// at 577 of 1500 s) goes to one more attempt when there is enough of it:
/// the attempt again in the other pour mode (as tracks after connected,
/// and the other way round; the only continuation that gained), in
/// `result-continuation/`, kept where KiCad checked it and finds it better
/// than `best_quality`. Where it does not run with connections open, the
/// board says why. Its work joins `after_work`.
#[allow(
    clippy::too_many_arguments,
    reason = "the ladder's state is threaded through by reference"
)]
fn run_continuation(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    fallback_config: &KiCadBoardRouterConfig,
    fallback_pours: &str,
    attempt_source: &Path,
    after_work: &mut f64,
    best_quality: (usize, usize),
    board: &mut KeptBoard,
) -> Result<(), String> {
    let result_directory = ctx.output_directory.join("result");
    let left = ledger.left(ctx.work_deadline) - *after_work;
    let wall = !ctx.stop.allows(LayoutStep::Continuation);
    let continuation_config = if narrow_continuation_runs(left, best_quality.0, wall) {
        let mut other = fallback_config.clone();
        // Nothing comes after it.
        other.keep_for_continuation = false;
        other.pours = if fallback_pours == "tracks" {
            KiCadPourMode::Connect
        } else {
            KiCadPourMode::Tracks
        };
        let (negotiation, cleanup) = direct_narrow_budget(left);
        other.negotiation_seconds = Some(negotiation);
        other.cleanup_seconds = Some(cleanup);
        eprintln!(
            "layout: {left:.0} s of work left after the narrow-signal attempt: it again with pours {} (negotiation {negotiation:.0} s, clean-up {cleanup:.0} s)",
            if other.pours == KiCadPourMode::Tracks {
                "as tracks"
            } else {
                "connected"
            }
        );
        Some(other)
    } else {
        if best_quality.0 > 0 {
            let why = if wall {
                "the wall clock passed the layout's guard".to_string()
            } else {
                format!("{left:.0} s of work left, under {NARROW_CONTINUATION_FLOOR:.0} s")
            };
            eprintln!("layout: no attempt after the narrow-signal attempt: {why}");
            board.routed.skipped.push(format!(
                "the attempt after the narrow-signal attempt (the other pour mode) did not run: {why}"
            ));
        }
        None
    };
    let Some(mut continuation_config) = continuation_config else {
        return Ok(());
    };
    // The work left, as the ladder's (until 2026-10-21 at most
    // 1200 s), or less where the router config says so.
    continuation_config.ladder_budget_seconds = Some(
        ctx.router_config
            .ladder_budget_seconds
            .map_or(left, |budget| budget.min(left))
            .max(0.0),
    );
    continuation_config.deadline_work = Some(left.max(0.0));
    continuation_config.kicad_check_top = Some(KICAD_CHECK_TOP);
    continuation_config.solid_starved_thermals = Some(false);
    continuation_config.kicad_kept = Some(kept_standing(&board.routed, best_quality));
    let continuation_directory = ctx.output_directory.join("result-continuation");
    let continuation = route_kicad_board(
        attempt_source,
        ctx.board_id,
        &continuation_directory,
        &continuation_config,
    )?;
    ledger.checks.add(continuation.kicad_checks_charged);
    *after_work += continuation.work_seconds;
    let continuation_quality = quality(&continuation, &continuation_directory);
    eprintln!(
        "layout: the attempt after the narrow-signal attempt: {} open, {} starved, {} vias, work {:.0} s ({} open, {} starved before)",
        continuation_quality.0,
        continuation_quality.1,
        continuation.vias,
        continuation.work_seconds,
        best_quality.0,
        best_quality.1
    );
    if checked(ctx, &continuation) && continuation_quality < best_quality {
        fs::remove_dir_all(&result_directory).map_err(|error| error.to_string())?;
        fs::rename(&continuation_directory, &result_directory)
            .map_err(|error| error.to_string())?;
        board.routed = continuation;
        board.solid_tried = false;
    } else {
        fs::remove_dir_all(&continuation_directory).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The board kept, finished: its starved pads solid (`solid_starved_thermals`,
/// on unless false; kept only when KiCad finds the board better), its
/// connection_width necks repaired, how whole its pours stayed measured,
/// and `board-router.json` written again where any of that changed it.
/// The checks count as the board's work.
fn finish_kept_board(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    board: &mut KeptBoard,
) -> Result<(), String> {
    let router_config = &ctx.router_config;
    let routed = &mut board.routed;
    let result_directory = ctx.output_directory.join("result");
    // The starved pads of the board kept, solid. Once, here: on an
    // attempt it made the layout prefer eternal-keypad's continuation (its
    // starved thermals gone, a short the attempts' comparison does not
    // weigh) to the board it kept before. Not again on a board that had
    // its try before the ladder (thermals only).
    if !board.solid_tried
        && router_config.solid_starved_thermals != Some(false)
        && routed.native.is_some()
        && ctx.stop.allows(LayoutStep::NeckRepair)
    {
        let checks = crate::board_router::solidify_starved_thermals(
            routed,
            &result_directory,
            ctx.board_id,
        )?;
        board.solidified |= checks > 0;
        routed.work_seconds += ledger.checks.add(checks);
    }
    // KiCad's connection_width necks on the board kept (a no-op when the
    // route mode ladder above has repaired it already). It improves the
    // board: past the deadline it is skipped, and its checks stay within
    // the layout's work left (`neck_check_bound`; they used to be charged
    // after it, past `total_seconds` by up to eight checks).
    if (routed.native.is_some() || router_config.skip_native_verification)
        && ctx.stop.allows(LayoutStep::NeckRepair)
    {
        let left = ledger.left(ctx.work_deadline) - routed.work_seconds;
        let checks = crate::board_router::repair_necks(
            routed,
            &result_directory,
            ctx.board_id,
            router_config,
            neck_check_bound(left, ctx.check_seconds),
        )?;
        // Its checks count as work (after every decision: only the tally).
        routed.work_seconds += ledger.checks.add(checks);
    }
    // How whole the pours stayed on the board kept (a report: after every
    // decision, counted as no work).
    let measured = crate::board_router::measured_board(
        &result_directory,
        ctx.board_id,
        router_config,
        router_config.skip_native_verification
            && router_config.solid_starved_thermals != Some(false),
    );
    routed.pour_quality = measured.pours;
    routed.predicted_starved_thermals = measured.starved;
    board.solidified |= !measured.solidified.is_empty();
    routed.solid_by_model = measured.solidified;
    if board.solidified
        || !routed.pour_quality.is_empty()
        || routed
            .neck_repair
            .as_ref()
            .is_some_and(|repair| repair.kept)
    {
        let report_path = result_directory.join("board-router.json");
        fs::write(
            &report_path,
            serde_json::to_string_pretty(&*routed).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    }
    Ok(())
}

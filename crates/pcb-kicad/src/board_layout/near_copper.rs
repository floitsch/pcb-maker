// Copyright (C) 2026 Florian Loitsch.

//! Verify a one-step placement correction without rerouting saved copper.

use super::setup::{LayoutContext, WorkLedger};
use super::*;
use crate::fill_handoff::{counts, has_fillable_zones};
use crate::placement_retry::{
    KiCadPlacementRetryEvidence, RetryNativeCheck, verify_body_near_poses,
};
use pcb_kicad_file::footprint_bodies::write_footprint_pose;
use pcb_kicad_place::placement_constraints::{BodyNearGridStep, body_near_grid_steps};
use serde_json::json;

const MAX_TRIALS: usize = 2;
const CHECKS_PER_TRIAL: usize = 2;

#[derive(Clone, Debug, Serialize)]
pub struct LayoutBodyNearGridRetry {
    pub status: &'static str,
    pub reason: Option<String>,
    pub step: Option<BodyNearGridStep>,
    pub native_checks: usize,
    pub work_seconds: f64,
}

pub(super) fn recover(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    problem: &crate::board_placer::LoweredPlacement,
    constraints: &mut Vec<KiCadConstraintStatus>,
    routed: &mut KiCadBoardRouterResult,
    fill: &mut Option<crate::FillPublication>,
) -> Result<Option<LayoutBodyNearGridRetry>, String> {
    let mut missed = constraints
        .iter()
        .filter(|row| !row.requirement_satisfied());
    if ctx.config.stop_after_first_route
        || !ctx.config.save_zone_fills
        || routed.route_reuse.is_some()
        || !routed.native.as_ref().is_some_and(|native| {
            native.schematic_parity_checked && native.selected_net_unconnected_items == 0
        })
        || !missed.next().is_some_and(|row| row.body_near_pin.is_some())
        || missed.next().is_some()
    {
        return Ok(None);
    }
    let mut outcome = LayoutBodyNearGridRetry {
        status: "skipped",
        reason: None,
        step: None,
        native_checks: 0,
        work_seconds: 0.0,
    };
    let available = ledger.left(ctx.work_deadline) - routed.work_seconds;
    let charge = CHECKS_PER_TRIAL as f64 * ctx.check_seconds;
    if ctx.wall_passed() || available < charge {
        outcome.reason = Some("insufficient_remaining_work_or_wall_time".into());
        return Ok(Some(outcome));
    }
    let started = placer::work();
    let steps = body_near_grid_steps(problem, ctx.config.body_near_inferred_spacing);
    outcome.work_seconds =
        placer::work().saturating_sub(started) as f64 / placer::work_per_second();
    ledger.work += outcome.work_seconds;
    if steps.is_empty() {
        outcome.reason = Some("no_eligible_adjacent_pose_on_selected_grid".into());
        return Ok(Some(outcome));
    }
    let result = ctx.output_directory.join("result");
    let before =
        match KiCadPlacementRetryEvidence::read_with_fill(&result, ctx.board_id, &json!(fill)) {
            Ok(before)
                if before
                    .as_stored
                    .as_ref()
                    .is_some_and(|proof| proof.opens == 0) =>
            {
                before
            }
            Ok(_) => {
                outcome.reason = Some("selected_stored_copper_is_unverified".into());
                return Ok(Some(outcome));
            }
            Err(error) => {
                outcome.reason = Some(error);
                return Ok(Some(outcome));
            }
        };
    let source = result.join(format!("{}.kicad_pcb", ctx.board_id));
    for step in steps.into_iter().take(MAX_TRIALS) {
        if ctx.wall_passed() || available - outcome.work_seconds < charge {
            outcome.reason = Some("insufficient_remaining_work_or_wall_time".into());
            break;
        }
        let mut poses = problem.problem.poses.clone();
        let [x, y, angle] = step.at;
        let pose = placer::Pose {
            position: [x, y],
            angle,
        };
        poses[step.part_index] = pose;
        let trial_constraints = super::report::constraints_at(problem, &poses, None);
        if trial_constraints
            .iter()
            .any(|row| !row.requirement_satisfied())
        {
            continue;
        }
        outcome.step = Some(step);
        let step = outcome.step.as_ref().unwrap();
        let project = fs_util::temporary_next_to(&result);
        let mut checks = 0;
        let trial = check_step(ctx, &result, &project, &before, step, pose, &mut checks);
        outcome.native_checks += checks;
        let work = ledger.checks.add(checks);
        outcome.work_seconds += work;
        ledger.work += work;
        let (selected, verified_fill) = match trial {
            Ok(result) => result,
            Err(error) => {
                let _ = fs::remove_dir_all(project);
                outcome.status = "declined";
                outcome.reason = Some(error);
                continue;
            }
        };
        let installed = (|| {
            if ctx.wall_passed() || fs_util::file_sha256(&source)? != before.board_sha256 {
                return Err("selected board or deadline changed during the trial".into());
            }
            let mut checked = routed.clone();
            checked.native = Some(selected);
            checked.closed_reuse_drc = None;
            checked.model_precedes_body_near_grid_retry = true;
            write_typed_json(&project.join("board-router.json"), &checked)?;
            super::placement_evidence::publish_grid_step(
                &project,
                ctx.board_id,
                problem,
                &poses,
                step,
                &trial_constraints,
            )?;
            Ok::<_, String>(checked)
        })();
        let checked = match installed {
            Ok(checked) => checked,
            Err(error) => {
                let _ = fs::remove_dir_all(project);
                outcome.status = "declined";
                outcome.reason = Some(error);
                break;
            }
        };
        let previous = fs_util::temporary_next_to(&result);
        fs::rename(&result, &previous).map_err(|error| error.to_string())?;
        if let Err(error) = fs::rename(&project, &result) {
            fs::rename(&previous, &result).map_err(|restore| {
                format!("could not adopt verified near-grid move: {error}; restore: {restore}")
            })?;
            return Err(format!("could not adopt verified near-grid move: {error}"));
        }
        let _ = fs::remove_dir_all(previous);
        *routed = checked;
        *fill = verified_fill;
        *constraints = trial_constraints;
        outcome.status = "accepted";
        outcome.reason = None;
        eprintln!(
            "layout: {} body to {} {:.6} to {:.6} mm after one grid step, saved tracks and vias retained",
            step.part, step.pin_of, step.before_mm, step.distance_mm,
        );
        return Ok(Some(outcome));
    }
    if outcome.reason.is_none() {
        outcome.reason = Some("adjacent_poses_miss_another_constraint".into());
    }
    Ok(Some(outcome))
}

fn check_step(
    ctx: &LayoutContext,
    source: &Path,
    trial: &Path,
    before: &KiCadPlacementRetryEvidence,
    step: &BodyNearGridStep,
    pose: placer::Pose,
    checks: &mut usize,
) -> Result<
    (
        verification::VerificationReport,
        Option<crate::FillPublication>,
    ),
    String,
> {
    fs_util::copy_directory_tree(source, trial)?;
    let name = format!("{}.kicad_pcb", ctx.board_id);
    let board = trial.join(&name);
    let source_board = source.join(&name);
    let mut pcb = crate::parse(&fs::read_to_string(&board).map_err(|error| error.to_string())?)?;
    let Expr::List(items) = &mut pcb else {
        return Err("expected a KiCad PCB".into());
    };
    let mut footprints = items.iter_mut().filter(|item| {
        item.head() == Some("footprint")
            && crate::kicad_nets::footprint_reference(item).as_deref() == Some(&step.part)
    });
    let footprint = footprints.next().ok_or("the trial footprint is missing")?;
    if footprints.next().is_some()
        || footprint.child("zone").is_some()
        || board_owns_edge_cut(footprint)
    {
        return Err("the trial footprint is ambiguous or owns board geometry".into());
    }
    write_footprint_pose(footprint, pose)?;
    fs_util::write_atomically(&board, format!("{}\n", crate::encode(&pcb)).as_bytes())?;
    verify_body_near_poses(&source_board, &board, &step.part, step.before_at, step.at)?;
    let stopped = || ctx.wall_passed();
    let drc_path = trial.join("drc.json");
    let check_refilled = |drc: &serde_json::Value| {
        RetryNativeCheck::measure(drc)
            .qualifies_body_near_against(&before.refilled)
            .map_err(str::to_owned)?;
        write_typed_json(&drc_path, drc)
    };
    let fill = if has_fillable_zones(&pcb) {
        use crate::fill_handoff::layout::{FillRefillEvidence, publish_layout_fill};
        let hash = fs_util::file_sha256(&board)?;
        let fill = publish_layout_fill(
            trial,
            ctx.board_id,
            pcb,
            hash,
            FillRefillEvidence::GuardedTrial(&check_refilled),
            false,
            stopped,
        );
        *checks = fill.native_checks_charged;
        if fill.status != "saved" {
            return Err(fill
                .reason
                .unwrap_or_else(|| "candidate fill not verified".into()));
        }
        Some(fill)
    } else {
        for (output, refill) in [
            (&drc_path, true),
            (&trial.join("drc-saved-fill.json"), false),
        ] {
            *checks += 1;
            let native =
                native_report::check_kept_route_input(&board, output, true, refill, stopped)?;
            native_report::validate_drc_report(&native, &board)?;
            if !counts(&native).possibly_capped_types.is_empty() {
                return Err("native check may have reached a finding cap".into());
            }
            if refill {
                check_refilled(&native)?;
            } else {
                RetryNativeCheck::measure(&native)
                    .qualifies_body_near_against(before.as_stored.as_ref().unwrap())
                    .map_err(str::to_owned)?;
            }
        }
        None
    };
    KiCadPlacementRetryEvidence::read_with_fill(trial, ctx.board_id, &json!(fill))?
        .qualifies_body_near_against(before)
        .map_err(str::to_owned)?;
    let native = verification::record_verified_additive_board(trial, ctx.board_id, true)?;
    Ok((native, fill))
}

fn board_owns_edge_cut(footprint: &Expr) -> bool {
    footprint
        .children()
        .iter()
        .any(|item| pcb_kicad_file::kicad_sexpr::form_atom(item, "layer", 1) == Some("Edge.Cuts"))
}

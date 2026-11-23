// Copyright (C) 2026 Florian Loitsch.

//! Keep the placement report and playback beside the final placement they describe.

use super::race::RaceOutcome;
use super::setup::LayoutContext;
use super::*;
use crate::board_placer::{
    KiCadBusGroupsReport, LoweredPlacement, footprint_mounting_side, length_rule_report,
};
use crate::kicad_nets::footprint_uuid;
use pcb_kicad_place::placement_constraints::BodyNearGridStep;

pub(super) enum Selection<'a> {
    Routed {
        race: &'a RaceOutcome,
        kept_moves: usize,
    },
    Reused,
}

pub(super) struct Evidence {
    json: Vec<u8>,
    html: String,
}

/// Assemble the selected report and preserve the original placement before
/// the final ladder can replace its board in `placed/`.
pub(super) fn capture(
    ctx: &LayoutContext,
    initial: &KiCadBoardPlacerResult,
    pcb: &Expr,
    lowered: &LoweredPlacement,
    relaxation: &placer::Relaxation,
    selection: Selection<'_>,
) -> Result<Option<Evidence>, String> {
    let problem = &lowered.problem;
    let mut footprints = initial.footprints.clone();
    let board_footprints = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"));
    if board_footprints.clone().count() != footprints.len() {
        return Err("selected placement and initial report have different footprint counts".into());
    }
    let (race, kept_moves, source, archive) = match selection {
        Selection::Routed { race, kept_moves } => {
            (Some(race), kept_moves, "layout", "initial-placement")
        }
        Selection::Reused => (None, 0, "route_reuse", "reuse/initial-placement"),
    };
    let mut changed = race.is_some_and(|race| race.pool_index != 0);
    for (index, (footprint, row)) in board_footprints.zip(&mut footprints).enumerate() {
        if lowered.references.get(index) != Some(&row.reference)
            || footprint_uuid(footprint) != row.uuid
        {
            return Err(format!(
                "selected placement cannot identify report row for {}",
                row.reference
            ));
        }
        let at = form_at(footprint)?;
        let side = footprint_mounting_side(footprint);
        changed |= row.at != at || row.side != side;
        row.at = at;
        row.side = side;
        let component = &problem.components[index];
        let pose = placer::Pose {
            position: [at[0], at[1]],
            angle: at[2],
        };
        row.fixed = component.fixed;
        row.body_center = component.center(pose);
        row.body_half = component.half_extent(pose.angle);
    }
    if !changed {
        return Ok(None);
    }

    let mut selected = initial.clone();
    selected.footprints = footprints;
    selected.wirelength_final_mm = problem.wirelength(&problem.poses);
    selected.constraints = report::constraints(lowered, relaxation);
    selected.length_rules = length_rule_report(lowered, &problem.poses);
    selected.room = crate::placement_room::room(problem, &lowered.references, false);
    selected.utilization = selected.room.utilization;
    selected.tight_bodies = relaxation.tight;
    selected.spacing_mm = relaxation.spacing;
    selected.grid_mm = relaxation.grid;
    selected.halo_scale = relaxation.halo_scale;
    selected.small_halo_scale = relaxation.small_halo_scale;
    selected.edge_inset = relaxation.edge_inset;
    selected.edge_copper = relaxation.edge_copper;
    selected.edge_rule_mm = relaxation.edge_rule;
    selected.courtyard_spacing = relaxation.courtyard_spacing;
    selected.pad_clearance = relaxation.pad_clearance;
    selected.parts_on_marks = relaxation
        .on_marks
        .iter()
        .map(|&index| lowered.references[index].clone())
        .collect();
    selected.pad_exits = race
        .filter(|_| kept_moves == 0)
        .and_then(|race| race.pad_exits.clone());
    let alternative = race.and_then(|race| race.alternative_index);
    if let Some(index) = alternative {
        let seed = &initial.alternative_seed_metrics[index];
        selected.seed_kept = seed.seed;
        selected.global_iterations = seed.global_iterations;
        selected.global_overflow = seed.global_overflow;
        selected.wirelength_source_mm = seed.wirelength_source_mm;
        selected.wirelength_global_mm = seed.wirelength_global_mm;
        selected.wirelength_legal_mm = seed.wirelength_legal_mm;
        selected.skeleton_race.clone_from(&seed.skeleton_race);
        selected.panel_copies.clone_from(&seed.panel_copies);
        selected.sparse_legalization = seed.sparse_legalization;
        selected
            .pad_objective_composition
            .clone_from(&seed.pad_objective_composition);
    }
    if race.is_none() {
        selected.skeleton_race = None;
        selected.panel_copies = None;
        selected.sparse_legalization = None;
        selected.pad_objective_composition = None;
    }
    selected.bus_groups = ctx.config.placer.tuning.bus_groups.as_ref().map(|groups| {
        KiCadBusGroupsReport::of(
            problem,
            &groups.settings(),
            &problem.poses,
            &lowered.references,
        )
    });
    let mut json = serde_json::to_value(&selected).map_err(|error| error.to_string())?;
    if race.is_none() {
        let fields = json
            .as_object_mut()
            .expect("the placement report is an object");
        for key in [
            "seed_kept",
            "global_iterations",
            "global_overflow",
            "wirelength_source_mm",
            "wirelength_global_mm",
            "wirelength_legal_mm",
        ] {
            fields.remove(key);
        }
    }
    json["initial_placement_report"] = format!("../{archive}/board-placer.json").into();
    json["selected_placement"] = serde_json::json!({
        "source": source,
        "kept_moves": kept_moves,
    });
    if let Some(race) = race {
        json["selected_placement"]["pool_index"] = race.pool_index.into();
    }
    let json = serde_json::to_vec_pretty(&json).map_err(|error| error.to_string())?;
    let frame = placer::Frame {
        iteration: 0,
        overflow: 0.0,
        wirelength: selected.wirelength_final_mm,
        poses: problem.poses.clone(),
    };
    let html = placer::view::playback_html(
        problem,
        &[frame],
        &format!("{} selected placement", ctx.board_id),
    );

    let original = ctx.output_directory.join(archive);
    fs::create_dir(&original).map_err(|error| error.to_string())?;
    for name in [
        format!("{}.kicad_pcb", ctx.board_id),
        "board-placer.json".into(),
        "placement.html".into(),
    ] {
        fs::copy(ctx.placed_directory.join(&name), original.join(name))
            .map_err(|error| error.to_string())?;
    }
    Ok(Some(Evidence { json, html }))
}

pub(super) fn publish(
    ctx: &LayoutContext,
    pcb: &Expr,
    evidence: Option<Evidence>,
) -> Result<(), String> {
    placement::write_placement_without_copper(ctx, pcb)?;
    if let Some(Evidence { json, html }) = evidence {
        for directory in [&ctx.placed_directory, &ctx.output_directory.join("result")] {
            write_atomically(&directory.join("board-placer.json"), &json)?;
            write_atomically(&directory.join("placement.html"), html.as_bytes())?;
        }
    }
    Ok(())
}

/// Publish the current placement beside the verified routed board. The
/// ordinary `placed/` project still identifies the exact pre-grid placement.
pub(super) fn publish_grid_step(
    directory: &Path,
    board_id: &str,
    lowered: &LoweredPlacement,
    poses: &[placer::Pose],
    step: &BodyNearGridStep,
    constraints: &[KiCadConstraintStatus],
) -> Result<(), String> {
    let path = directory.join("board-placer.json");
    let mut report: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let footprints = report["footprints"]
        .as_array_mut()
        .ok_or("missing footprint report")?;
    let mut matching = footprints
        .iter_mut()
        .filter(|row| row["reference"] == step.part);
    let row = matching.next().ok_or("missing moved footprint report")?;
    if matching.next().is_some() {
        return Err("ambiguous moved footprint report".into());
    }
    let pose = poses[step.part_index];
    let problem = &lowered.problem;
    row["at"] = serde_json::json!(step.at);
    row["body_center"] = serde_json::json!(problem.components[step.part_index].center(pose));
    report["constraints"] = serde_json::json!(constraints);
    let wirelength = problem.wirelength(poses);
    report["wirelength_final_mm"] = serde_json::json!(wirelength);
    report["length_rules"] = serde_json::json!(length_rule_report(lowered, poses));
    report
        .as_object_mut()
        .ok_or("invalid placement report")?
        .remove("pad_exits");
    report["selected_placement"]["source"] = "body_near_grid_retry".into();
    report["selected_placement"]["pre_grid_placement"] = "../placed/board-placer.json".into();
    write_typed_json(&path, &report)?;
    let frame = placer::Frame {
        iteration: 0,
        overflow: 0.0,
        wirelength,
        poses: poses.to_vec(),
    };
    let html =
        placer::view::playback_html(problem, &[frame], &format!("{board_id} selected placement"));
    write_atomically(&directory.join("placement.html"), html.as_bytes())
}

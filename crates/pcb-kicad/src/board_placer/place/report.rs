// Copyright (C) 2026 Toit contributors.

//! board-placer.json: the placement said, with the room, the parts
//! without a legal place and the hints for them.

use super::*;

/// The placement's report: the picked placement, its relaxation, room
/// and hints, the parts without a place, the constraints and the race's
/// work.
pub(super) fn placement_report(
    context: &PlaceContext,
    pcb: &Expr,
    lowered: &LoweredPlacement,
    sub_boards: Option<crate::sub_boards::KiCadSubBoardsReport>,
    picked: Picked,
    written: WrittenPoses,
) -> KiCadBoardPlacerResult {
    let config = &context.config;
    let Picked {
        placement,
        seed_kept,
        work_seconds,
        seeds_placed,
        work,
        skeleton_race,
        panel_copies,
        alternatives,
        alternative_exits,
    } = picked;
    let WrittenPoses { footprints, uuids } = written;
    let names = |indices: &[usize]| -> Vec<String> {
        indices
            .iter()
            .map(|index| lowered.references[*index].clone())
            .collect()
    };
    // How full each side is, in the room's numbers (`placement_room::room`:
    // the movable parts' bodies with spacing over the usable area, the
    // outline less its edge band, its cutouts and the fixed parts' bodies),
    // and what to change when parts did not fit. Until 2026-11-04 the hint
    // and `utilization` counted every component (fixed parts, the board's
    // marks, rule areas, a panel's kept-clear frames, each with spacing)
    // over the outline's gross area: trial 4's ULK panel was "99 % full"
    // beside room.utilization 0.135.
    let problem = &lowered.problem;
    let room = crate::placement_room::room(
        problem,
        &lowered.references,
        !placement.unplaced.is_empty() || !placement.illegal.is_empty(),
    );
    let utilization = room.utilization;
    let repeated_references = repeated_references(&lowered.references, uuids.len());
    let unplaced_parts = unplaced_parts(
        problem,
        &lowered.references,
        &uuids,
        &repeated_references,
        &placement,
    );
    let hints = placement_hints(&placement, &room, &unplaced_parts);
    KiCadBoardPlacerResult {
        alternatives,
        alternative_exits,
        tight_bodies: placement.relaxation.tight,
        spacing_mm: placement.relaxation.spacing,
        grid_mm: placement.relaxation.grid,
        halo_scale: placement.relaxation.halo_scale,
        small_halo_scale: placement.relaxation.small_halo_scale,
        edge_inset: placement.relaxation.edge_inset,
        edge_copper: placement.relaxation.edge_copper,
        edge_rule_mm: placement.relaxation.edge_rule,
        courtyard_spacing: placement.relaxation.courtyard_spacing,
        pad_clearance: placement.relaxation.pad_clearance,
        parts_on_marks: placement
            .relaxation
            .on_marks
            .iter()
            .map(|index| lowered.references[*index].clone())
            .collect(),
        on_marks: placement.relaxation.on_marks.clone(),
        attached_marks: lowered.attached_report.clone(),
        array_rooms: lowered.array_rooms.clone(),
        length_rules: super::lower::length_rule_report(lowered, &placement.poses),
        attached_mark_pairs: lowered.attached_marks.clone(),
        halos: placement.relaxation.halos.clone(),
        utilization,
        hints,
        room,
        overfull: placement.overfull,
        board_id: context.board_id.into(),
        components: lowered.problem.components.len(),
        movable: placed_count(&lowered.problem),
        nets: lowered.problem.net_weights.len(),
        global_iterations: placement.global_iterations,
        global_overflow: placement.global_overflow,
        wirelength_source_mm: placement.wirelength_initial,
        wirelength_global_mm: placement.wirelength_global,
        wirelength_legal_mm: placement.wirelength_legal,
        wirelength_final_mm: placement.wirelength_final,
        unplaced: names(&placement.unplaced),
        illegal: names(&placement.illegal),
        unplaced_parts,
        constraints: constraint_report(&lowered.problem, &placement.poses, &lowered.references)
            .into_iter()
            .chain(pair_row_statuses(
                &lowered.pair_rows,
                &lowered.problem,
                &placement.poses,
                &lowered.references,
            ))
            .collect(),
        constraint_warnings: lowered.constraint_warnings.clone(),
        outline_mm: context.outline_size,
        edges_chosen: context.edges_chosen.clone(),
        seconds: context.started.elapsed().as_secs_f64(),
        work_seconds,
        pool_work_seconds: work.pool as f64 / core::work_per_second(),
        seeds_work_seconds: work.seeds as f64 / core::work_per_second(),
        seeds: seeds_placed,
        seeds_run: work.run as usize,
        seed_kept,
        footprints,
        bus_groups: config.bus_groups.as_ref().map(|bus_groups| {
            KiCadBusGroupsReport::of(
                &lowered.problem,
                &bus_groups.settings(),
                &placement.poses,
                &lowered.references,
            )
        }),
        panel_copies,
        sub_boards,
        pad_exits: placement
            .exits
            .as_ref()
            .map(|outcome| exits::exits_report(outcome, &lowered.references, pcb)),
        skeleton_race,
    }
}

/// Writes the report as `board-placer.json` into the output directory.
pub(super) fn write_report(
    context: &PlaceContext,
    result: &KiCadBoardPlacerResult,
) -> Result<(), String> {
    let report_path = context.output_directory.join("board-placer.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))
}

/// The references that more than one of the board's first `footprints`
/// parts (its footprints) carries.
pub(super) fn repeated_references(references: &[String], footprints: usize) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut repeated = BTreeSet::new();
    for reference in &references[..footprints.min(references.len())] {
        if !seen.insert(reference.as_str()) {
            repeated.insert(reference.clone());
        }
    }
    repeated
}

/// What to change when parts found no legal place (none when all did):
/// the sides fuller than the room's target, the constraints that hold a
/// part without a place, a board too small for every part, else more
/// room; sorted, each once.
pub(super) fn placement_hints(
    placement: &core::Placement,
    room: &crate::placement_room::KiCadRoom,
    unplaced_parts: &[KiCadUnplacedPart],
) -> Vec<String> {
    let mut hints = Vec::new();
    if !placement.unplaced.is_empty() || !placement.illegal.is_empty() {
        for (side, (share, usable)) in ["front", "back"]
            .iter()
            .zip(room.utilization.iter().zip(room.usable_mm2))
        {
            if *share > crate::placement_room::TARGET_UTILIZATION {
                hints.push(room_hint(side, *share, usable));
            }
        }
        // Fixed parts are only illegal where a part that found no place
        // lies on them; the moving parts are the ones to talk about, each
        // with the constraints that hold it (a repeated reference with
        // its uuid).
        for part in unplaced_parts {
            if !part.held_by.is_empty() {
                hints.push(format!(
                    "{} is held by {}: loosen it (a larger max_mm or region)",
                    part.label,
                    part.held_by.join(", ")
                ));
            }
        }
        if placement.overfull {
            hints.push(
                "even the tightest bodies need more area than a side has: no placement fits every part; room.suggested_outline_mm is an outline that holds them"
                    .into(),
            );
        }
        if hints.is_empty() {
            hints.push(
                "no free spot fits the part's courtyard with the copper clearance around it: give the board more room"
                    .into(),
            );
        }
        hints.sort();
        hints.dedup();
    }
    hints
}

/// The placement hint for a side fuller than the room's target
/// (`placement_room::TARGET_UTILIZATION`), in the room's terms: `share`
/// is `room.utilization` of the side, `usable` its `room.usable_mm2`.
pub(crate) fn room_hint(side: &str, share: f64, usable: f64) -> String {
    if usable <= 0.0 {
        return format!(
            "the {side} side has no usable area left (room.usable_mm2 0: the edge band, the cutouts and the fixed parts' bodies cover the outline): enlarge the outline, move parts to the other side, or fix fewer parts"
        );
    }
    format!(
        "the {side} side is {:.0} % full (room.utilization {share:.3}: the movable parts' bodies with spacing over its {usable:.0} mm2 usable, the outline less its edge band, its cutouts and the fixed parts): enlarge the outline, move parts to the other side, or fix fewer parts",
        share * 100.0
    )
}

/// The movable parts without a legal place, each once (unplaced first),
/// with their uuids and the constraints that hold them.
pub(super) fn unplaced_parts(
    problem: &core::Problem,
    references: &[String],
    uuids: &[Option<String>],
    repeated: &BTreeSet<String>,
    placement: &core::Placement,
) -> Vec<KiCadUnplacedPart> {
    let constraints = &problem.constraints;
    let mut seen = BTreeSet::new();
    let mut parts = Vec::new();
    // The illegal parts' reasons, judged at the placement's level.
    let relaxed = (!placement.illegal.is_empty()).then(|| {
        let mut relaxed = problem.clone();
        placement.relaxation.apply(&mut relaxed);
        relaxed
    });
    let reason = |index: usize| -> Option<String> {
        let relaxed = relaxed.as_ref()?;
        let poses = &placement.poses;
        // The pair tests book no placement work (`is_legal` would).
        Some(
            (0..relaxed.components.len())
                .filter(|other| *other != index)
                .find_map(|other| {
                    core::legal::conflict_kind(relaxed, poses, index, poses[index], other)
                        .map(|kind| format!("{kind} with {}", references[other]))
                })
                .unwrap_or_else(|| "the outline or a hard constraint".into()),
        )
    };
    for (index, status) in placement
        .unplaced
        .iter()
        .map(|index| (*index, "unplaced"))
        .chain(placement.illegal.iter().map(|index| (*index, "illegal")))
    {
        // Fixed parts and the board's marks are illegal only where a part
        // that found no place lies on them.
        if problem.components[index].fixed || index >= uuids.len() || !seen.insert(index) {
            continue;
        }
        let mut held_by = Vec::new();
        for (part, edge, max) in &constraints.edges {
            if *part == index {
                held_by.push(format!("edge {} within {max:.2} mm", edge.name()));
            }
        }
        for (part, region) in &constraints.regions {
            if *part == index {
                held_by.push(format!(
                    "region x {:.2}..{:.2}, y {:.2}..{:.2}",
                    region[0], region[2], region[1], region[3]
                ));
            }
        }
        for (part, edge, _) in &constraints.overhangs {
            if *part == index {
                held_by.push(format!("overhang {}", edge.name()));
            }
        }
        let pose = placement.poses[index];
        let reference = &references[index];
        let label = match &uuids[index] {
            Some(uuid) if repeated.contains(reference) => format!("{reference} (uuid {uuid})"),
            _ => reference.clone(),
        };
        parts.push(KiCadUnplacedPart {
            reference: reference.clone(),
            label,
            uuid: uuids[index].clone(),
            status,
            at: [pose.position[0], pose.position[1], pose.angle],
            held_by,
            reason: if status == "illegal" {
                reason(index)
            } else {
                None
            },
        });
    }
    parts
}

/// The parts the placer placed: the free ones and a row's other parts,
/// which move with its first (fixed in the problem only because their
/// leader carries them; trial 5's right_io counted its six row followers
/// as "kept where they are", 17 where 11 were fixed).
pub(crate) fn placed_count(problem: &core::Problem) -> usize {
    let free = problem
        .components
        .iter()
        .filter(|component| !component.fixed)
        .count();
    let carried = problem
        .constraints
        .followers
        .iter()
        .filter(|follower| {
            problem
                .components
                .get(follower.leader)
                .is_some_and(|leader| !leader.fixed)
        })
        .count();
    free + carried
}

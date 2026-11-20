// Copyright (C) 2026 Toit contributors.

//! Which parts the problem holds where the input put them, the user's
//! constraints (with the pair rows), the slots a part carries, the board
//! marks a part owns, and the warnings about fixed parts.

use super::*;
use crate::placement_constraints::{HeldOverlapGeometry, held_body_overlap, overlap_warning};
use pcb_placer as core;

fn fixed_by_config(config: &KiCadBoardPlacerConfig, reference: &str) -> bool {
    config.fixed.iter().any(|fixed| fixed == reference)
        || config
            .fixed_patterns
            .iter()
            .any(|pattern| glob_matches(pattern, reference))
}

/// Fixes the footprints the rules hold: those `fixed` names or a
/// `fixed_patterns` glob matches, and (unless `free` names them) the
/// locked ones, those without pins, and those on or beyond the edge, in a
/// keepout of the board (`keepouts`) or, for a connector, near the edge.
pub(super) fn fix_default_parts(
    context: &LowerContext,
    builder: &mut ProblemBuilder,
    keepouts: &[pcb_router::geometry::Aabb],
) {
    let config = context.config;
    let problem = &mut builder.problem;
    for index in 0..builder.footprints.count() {
        let reference = &builder.references[index];
        let component = &problem.components[index];
        let pose = problem.poses[index];
        let (center, half) = (component.center(pose), component.half_extent(pose.angle));
        let reach = config.tuning.edge_keep_mm;
        let in_keepout = keepouts.iter().any(|keepout| {
            center[0] + half[0] + reach > keepout.minimum[0]
                && center[0] - half[0] - reach < keepout.maximum[0]
                && center[1] + half[1] + reach > keepout.minimum[1]
                && center[1] - half[1] - reach < keepout.maximum[1]
        });
        let on_edge = !core::legal::body_inside_outline(problem, index, problem.poses[index])
            || in_keepout
            || (builder.footprints.connector[index]
                && core::legal::body_near_outline(
                    problem,
                    index,
                    problem.poses[index],
                    config.tuning.edge_keep_mm,
                ));
        let default_fixed =
            builder.locked[index] || problem.components[index].pins.is_empty() || on_edge;
        problem.components[index].fixed = fixed_by_config(config, reference)
            || (default_fixed && !config.free.contains(reference));
    }
}

/// Applies the user's constraints and the pair rows (`pair_rows`, added to
/// the user's rows); returns the pair rows and the constraints' warnings.
pub(super) fn apply_user_constraints(
    context: &LowerContext,
    builder: &mut ProblemBuilder,
) -> Result<(Vec<PairRow>, Vec<String>), String> {
    let config = context.config;
    let user_constraints = context.user_constraints;
    if let Some(constraints) = user_constraints {
        crate::placement_constraints::validate_near_pads(context.pcb, constraints)?;
    }
    let footprint_count = builder.footprints.count();
    let footprints = &builder.footprints;
    let configured_fixed: Vec<_> = builder.references[..footprint_count]
        .iter()
        .enumerate()
        .filter_map(|(index, reference)| fixed_by_config(config, reference).then_some(index))
        .collect();
    // A cold netlist can have every part parked just off the outline.
    // Check pair-row eligibility after the user's mobility rules release it.
    let warnings = match &config.constraints {
        Some(KiCadConstraintsSource::Inline(constraints)) => apply_constraints(
            &mut builder.problem,
            &builder.references,
            &footprints.pad_offsets,
            &footprints.pad_boxes,
            &footprints.keepout_boxes,
            &footprints.mouths,
            &builder.locked,
            &footprints.uuids,
            constraints,
            config.constraint_weight,
            &configured_fixed,
        )?,
        None => Vec::new(),
        Some(KiCadConstraintsSource::File(path)) => {
            return Err(format!(
                "constraints file {} was not resolved",
                path.display()
            ));
        }
    };
    // A differential pair's two series parts as a row of two
    // (`pair_rows`): only acts on otherwise free footprints.
    let pair_rows = if let Some(rows) = &config.placed_pair_rows {
        rows.clone()
    } else if config.pair_rows_on() {
        let explicitly_held = builder.references[..footprint_count]
            .iter()
            .zip(&builder.locked)
            .enumerate()
            .map(|(index, (reference, locked))| {
                *locked
                    || configured_fixed.binary_search(&index).is_ok()
                    || user_constraints.is_some_and(|constraints| {
                        constraints
                            .module_nonoverlap
                            .iter()
                            .any(|group| group.parts.contains(reference))
                    })
            })
            .collect::<Vec<_>>();
        implicit_pair_rows(
            &builder.problem,
            &builder.references[..footprint_count],
            (&footprints.library_names, &footprints.pad_nets),
            &explicitly_held,
            user_constraints,
            &config.pair_geometry,
            config.router_rules.as_deref(),
        )
    } else {
        Vec::new()
    };
    if !pair_rows.is_empty() {
        builder.problem.constraints.relation_weight = config.constraint_weight;
        crate::placement_constraints::apply_inferred_pair_rows(
            &mut builder.problem,
            &builder.references,
            &footprints.pad_boxes,
            &pair_rows
                .iter()
                .map(|pair| pair.row.clone())
                .collect::<Vec<_>>(),
            user_constraints,
        )?;
    }
    if let Some(constraints) = user_constraints {
        crate::placement_constraints::apply_module_envelopes(
            context.pcb,
            &mut builder.problem,
            &builder.references[..footprint_count],
            &constraints.module_nonoverlap,
        )?;
    }
    Ok((pair_rows, warnings))
}

/// A footprint's own slots (`Component::slots`) move with it: a movable
/// part's slot at its input pose is no hole of the board, and the part
/// carries it (its cutout among `owned_cutouts` occupies neither side). A
/// fixed part's at its input pose stays the cutout it was, and the part
/// carries none.
pub(super) fn carry_slots(builder: &mut ProblemBuilder, owned_cutouts: &[(usize, usize)]) {
    // (As before: bodies may reach over a held part's slot, copper keeps
    // the edge margin from it.)
    let footprint_count = builder.footprints.count();
    let source_at = &builder.source_at;
    let problem = &mut builder.problem;
    let leader_of = |problem: &core::Problem, part: usize| {
        problem
            .constraints
            .followers
            .iter()
            .find(|follower| follower.part == part)
            .map_or(part, |follower| follower.leader)
    };
    // A fixed part a `place` entry put elsewhere carries its slots too.
    let carried = |problem: &core::Problem, part: usize| {
        let leader = leader_of(problem, part);
        let pose = problem.poses[leader];
        let at = source_at[leader];
        !problem.components[leader].fixed
            || (pose.position[0] - at[0]).abs() > 1.0e-6
            || (pose.position[1] - at[1]).abs() > 1.0e-6
            || (pose.angle - at[2]).abs() > 1.0e-6
    };
    for &(cutout, owner) in owned_cutouts {
        if owner < footprint_count && carried(problem, owner) {
            problem.components[cutout].side = core::Side::Neither;
        }
    }
    for index in 0..footprint_count {
        if !carried(problem, index) {
            problem.components[index].slots.clear();
        }
    }
}

/// The warnings for fixed parts with pins whose bodies overlap on a side
/// (neither hollow).
pub(super) fn fixed_overlap_warnings(
    context: &LowerContext,
    builder: &ProblemBuilder,
) -> Vec<String> {
    // Fixed parts on top of each other place and route without a word,
    // and the board comes back with opens: say so (fence issue E). Checked
    // after the constraints, which fix and free parts (move_all frees a
    // stack fresh from a netlist that the default rules had fixed).
    let problem = &builder.problem;
    let references = &builder.references;
    let footprints: Vec<_> = context.footprints().collect();
    let mut geometry = BTreeMap::new();
    let mut warnings = Vec::new();
    let fixed: Vec<usize> = (0..builder.footprints.count())
        .filter(|index| {
            problem.components[*index].fixed && !problem.components[*index].pins.is_empty()
        })
        .collect();
    for (a, first) in fixed.iter().enumerate() {
        for second in &fixed[a + 1..] {
            let (ca, cb) = (&problem.components[*first], &problem.components[*second]);
            // A hollow part's body admits other parts (`hollow`:
            // framework_mobo_lefthalf's J1 around M1's pads); the
            // input check judges their pads (`held_parts_overlapping`).
            if !ca.side.collides(cb.side) || !ca.hollow.is_empty() || !cb.hollow.is_empty() {
                continue;
            }
            let (pa, pb) = (problem.poses[*first], problem.poses[*second]);
            let (center_a, half_a) = (ca.center(pa), ca.half_extent(pa.angle));
            let (center_b, half_b) = (cb.center(pb), cb.half_extent(pb.angle));
            let overlap = (0..2).all(|axis| {
                (center_a[axis] - center_b[axis]).abs() < half_a[axis] + half_b[axis] - 0.05
            });
            if overlap {
                for (index, pose) in [(*first, pa), (*second, pb)] {
                    geometry
                        .entry(index)
                        .or_insert_with(|| HeldOverlapGeometry::at_pose(footprints[index], pose));
                }
                if let Some(reason) =
                    held_body_overlap(geometry[first].as_ref(), geometry[second].as_ref())
                {
                    warnings.push(overlap_warning(
                        &references[*first],
                        &references[*second],
                        reason,
                    ));
                }
            }
        }
    }
    warnings
}

/// Gives the board marks on movable parts to those parts (`attach_marks`,
/// or the config's `attached_marks`): such a mark occupies neither side
/// and neither yields nor keeps a halo free. Returns the attachments
/// (place in `marks`, part) and their report lines.
pub(super) fn attach_marks_to_parts(
    context: &LowerContext,
    builder: &mut ProblemBuilder,
    marks: &[(usize, BoardMark)],
) -> (Vec<(usize, usize)>, Vec<String>) {
    // A mark on a movable part at its input position belongs to that part
    // (a connector's pin names inside its courtyard): no obstacle. Taken
    // from the input once (`attached_marks` in the config carries it to
    // the lowerings of the placed board).
    let problem = &mut builder.problem;
    let attached = match &context.config.attached_marks {
        Some(attached) => attached.clone(),
        None => attach_marks(problem, builder.footprints.count(), marks),
    };
    let mut attached_report = Vec::new();
    for &(place, part) in &attached {
        let Some((index, mark)) = marks.get(place) else {
            continue;
        };
        problem.components[*index].side = core::Side::Neither;
        problem.constraints.yielding.retain(|other| other != index);
        problem.constraints.halo_free.retain(|other| other != index);
        attached_report.push(format!("{}: {}", mark.key(), builder.references[part]));
    }
    (attached, attached_report)
}

/// The warnings for fixed parts whose bodies lie on board marks of their
/// side, one per part.
pub(super) fn fixed_on_marks_warnings(
    builder: &ProblemBuilder,
    marks: &[(usize, BoardMark)],
) -> Vec<String> {
    // A fixed part on a mark of the input (a header's pin names inside its
    // courtyard, a test pad under its name): both stay as the input put
    // them; said, not refused.
    // One warning per fixed part, however many marks it lies on (the
    // OpenESC panel's two frames lay on 20 texts and graphics: twenty
    // lines ahead of the one that mattered).
    let problem = &builder.problem;
    let references = &builder.references;
    let mut on_marks: Vec<(usize, Vec<String>)> = Vec::new();
    for &(index, ref mark) in marks {
        let mark_side = problem.components[index].side;
        for (part, _) in references
            .iter()
            .enumerate()
            .take(builder.footprints.count())
        {
            let component = &problem.components[part];
            if !component.fixed || !component.side.collides(mark_side) {
                continue;
            }
            let pose = problem.poses[part];
            let (center, half) = (component.center(pose), component.half_extent(pose.angle));
            let body = [
                center[0] - half[0],
                center[1] - half[1],
                center[0] + half[0],
                center[1] + half[1],
            ];
            if mark
                .boxes
                .iter()
                .any(|b| b[0] < body[2] && body[0] < b[2] && b[1] < body[3] && body[1] < b[3])
            {
                let what = format!(
                    "{} ({} at {:.2}, {:.2})",
                    mark.what, mark.layer, mark.at[0], mark.at[1]
                );
                match on_marks.iter_mut().find(|(other, _)| *other == part) {
                    Some((_, marks)) => marks.push(what),
                    None => on_marks.push((part, vec![what])),
                }
            }
        }
    }
    on_marks
        .into_iter()
        .map(|(part, marks)| fixed_on_marks_warning(&references[part], &marks))
        .collect()
}

/// The warning for a fixed part on marks of the input (`marks`: each
/// "text \"+\" (F.SilkS at 62.13, 66.20)"), the first three named.
pub fn fixed_on_marks_warning(reference: &str, marks: &[String]) -> String {
    let shown = 3;
    match marks {
        [mark] => format!(
            "fixed part {reference} lies on the board's {mark}: both stay where the input put them"
        ),
        _ => format!(
            "fixed part {reference} lies on {} of the board's texts and silkscreen graphics ({}{}): all stay where the input put them",
            marks.len(),
            marks[..marks.len().min(shown)].join("; "),
            if marks.len() > shown {
                format!("; {} more", marks.len() - shown)
            } else {
                String::new()
            }
        ),
    }
}

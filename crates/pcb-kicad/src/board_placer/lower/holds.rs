// Copyright (C) 2026 Toit contributors.

//! Which parts the problem holds where the input put them, the user's
//! constraints (with the pair rows), the slots a part carries, the board
//! marks a part owns, and the warnings about fixed parts.

use super::*;
use pcb_placer as core;

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
        let reach = config.edge_keep_mm;
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
                    config.edge_keep_mm,
                ));
        let default_fixed =
            builder.locked[index] || problem.components[index].pins.is_empty() || on_edge;
        problem.components[index].fixed = config.fixed.contains(reference)
            || config
                .fixed_patterns
                .iter()
                .any(|pattern| glob_matches(pattern, reference))
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
    let footprint_count = builder.footprints.count();
    let footprints = &builder.footprints;
    // A differential pair's two series parts as a row of two
    // (`pair_rows`): added to the user's rows, where any pair has them.
    let pair_rows = if config.pair_rows_on() {
        implicit_pair_rows(
            &builder.problem,
            &builder.references[..footprint_count],
            &footprints.library_names,
            &footprints.pad_nets,
            user_constraints,
            &config.pair_geometry,
        )
    } else {
        Vec::new()
    };
    let with_pairs = (!pair_rows.is_empty()).then(|| {
        let mut constraints = user_constraints.cloned().unwrap_or_else(|| {
            crate::placement_constraints::KiCadPlacementConstraints {
                version: 1,
                ..Default::default()
            }
        });
        constraints
            .row
            .extend(pair_rows.iter().map(|pair_row| pair_row.row.clone()));
        constraints
    });
    let apply = |problem: &mut core::Problem, constraints| {
        apply_constraints(
            problem,
            &builder.references,
            &footprints.pad_offsets,
            &footprints.pad_boxes,
            &footprints.keepout_boxes,
            &footprints.mouths,
            &builder.locked,
            &footprints.uuids,
            constraints,
            config.constraint_weight,
        )
    };
    let warnings = match (&config.constraints, &with_pairs) {
        (_, Some(constraints)) => apply(&mut builder.problem, constraints)?,
        (Some(KiCadConstraintsSource::Inline(constraints)), None) => {
            apply(&mut builder.problem, constraints)?
        }
        (None, None) => Vec::new(),
        (Some(KiCadConstraintsSource::File(path)), None) => {
            return Err(format!(
                "constraints file {} was not resolved",
                path.display()
            ));
        }
    };
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
pub(super) fn fixed_overlap_warnings(builder: &ProblemBuilder) -> Vec<String> {
    // Fixed parts on top of each other place and route without a word,
    // and the board comes back with opens: say so (fence issue E). Checked
    // after the constraints, which fix and free parts (move_all frees a
    // stack fresh from a netlist that the default rules had fixed).
    let problem = &builder.problem;
    let references = &builder.references;
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
                warnings.push(format!(
                    "fixed parts {} and {} overlap",
                    references[*first], references[*second]
                ));
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
pub(crate) fn fixed_on_marks_warning(reference: &str, marks: &[String]) -> String {
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

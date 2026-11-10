// Copyright (C) 2026 Toit contributors.

//! `move_all` and `fixed`: which parts the placer moves once the other
//! constraints are applied, and the refusal of the constraints no
//! placement can keep.

use super::*;

/// Applies `move_all`: every part moves but locked ones, row members,
/// artwork and parts nobody can name; mounting holes no constraint names
/// go to the corners, then to the middle of the edges.
pub(super) fn apply_move_all(
    problem: &mut core::Problem,
    parts: &Parts,
    fixed: &[String],
    constrained: &BTreeSet<usize>,
    pinned: &[usize],
) {
    // Parts without nets move too: from a netlist they sit in the stack
    // with the rest. Mounting holes nobody placed go to the corners, as
    // designers put them, then to the middle of the edges.
    let named: BTreeSet<usize> = fixed
        .iter()
        .flat_map(|pattern| parts.matching(pattern))
        .collect();
    let mut spots = [
        vec![Edge::Top, Edge::Left],
        vec![Edge::Top, Edge::Right],
        vec![Edge::Bottom, Edge::Left],
        vec![Edge::Bottom, Edge::Right],
        vec![Edge::Top],
        vec![Edge::Bottom],
        vec![Edge::Left],
        vec![Edge::Right],
    ]
    .into_iter();
    for index in 0..parts.footprints {
        // Artwork (a logo) occupies nothing and stays.
        let component = &problem.components[index];
        if parts.locked[index]
            || parts.leaders.contains_key(&index)
            || component.side == core::Side::Neither
        {
            continue;
        }
        // A part without nets and without a reference (a keyboard's
        // case holes, drawn by hand) is something no constraint can
        // name, so nobody can say where it goes: it stays. A netlist
        // gives every part a reference.
        if parts.references[index].is_empty() && component.pins.is_empty() {
            continue;
        }
        let mounting_hole = component.pins.is_empty() && component.has_holes();
        if mounting_hole
            && !constrained.contains(&index)
            && !named.contains(&index)
            && !pinned.contains(&index)
        {
            for side in spots.next().unwrap_or_default() {
                problem.constraints.edges.push((index, side, 1.0 + 1.0e-3));
            }
        }
        problem.components[index].fixed = false;
    }
}

/// Frees the parts the constraints say where to put (`constrained`) and
/// the parts sent to a side: they move.
pub(super) fn free_constrained(
    problem: &mut core::Problem,
    parts: &Parts,
    constraints: &KiCadPlacementConstraints,
    mut constrained: BTreeSet<usize>,
) {
    // A part sent to a side is placed there.
    for pattern in constraints.back.iter().chain(&constraints.front) {
        constrained.extend(parts.matching(pattern));
    }
    for index in constrained {
        problem.components[index].fixed = false;
    }
}

/// With an outline from the constraints, frees the parts with pins that
/// lie entirely off the board (`bounds`): they are unplaced, not
/// deliberately hanging over the edge.
pub(super) fn free_parts_off_the_outline(
    problem: &mut core::Problem,
    footprints: usize,
    outline: &KiCadOutlineConstraint,
    bounds: [f64; 4],
) {
    // On a polygon, a part whose centre lies off it (in an L's notch)
    // too.
    let polygonal = outline.polygonal() && problem.outline.len() >= 3;
    for index in 0..footprints {
        let component = &problem.components[index];
        let pose = problem.poses[index];
        let (center, half) = (component.center(pose), component.half_extent(pose.angle));
        let off = center[0] + half[0] <= bounds[0]
            || center[0] - half[0] >= bounds[2]
            || center[1] + half[1] <= bounds[1]
            || center[1] - half[1] >= bounds[3]
            || (polygonal && !point_in_polygon(center, &problem.outline));
        if off && !component.pins.is_empty() {
            problem.components[index].fixed = false;
        }
    }
}

/// Fixes the parts `place` put at a pose (`pinned`) and those `fixed`
/// names; a `fixed` glob that matches nothing is refused.
pub(super) fn fix_pinned_and_fixed(
    problem: &mut core::Problem,
    parts: &Parts,
    pinned: &[usize],
    fixed: &[String],
) -> Result<(), String> {
    for index in pinned {
        problem.components[*index].fixed = true;
    }
    for pattern in fixed {
        let matched = parts.matching(pattern);
        if matched.is_empty() {
            return Err(format!("fixed pattern {pattern:?} matches no part"));
        }
        for index in matched {
            problem.components[index].fixed = true;
        }
    }
    Ok(())
}

/// Refuses the constraints no placement can keep, all of them at once:
/// those collected (`cannot`), then a row's or an edge's parts the outline
/// cannot hold; and notes the room the board would need.
pub(super) fn refuse_what_cannot_be_kept(
    problem: &core::Problem,
    references: &[String],
    constraints: &KiCadPlacementConstraints,
    user_rows: &[(usize, [f64; 2])],
    mut cannot: Vec<String>,
) -> Result<(), String> {
    // A row or an edge's parts the outline cannot hold. Not for an
    // outline sized from the parts: the layout grows it while parts do
    // not fit.
    let automatic = constraints
        .outline
        .as_ref()
        .is_some_and(KiCadOutlineConstraint::automatic);
    let outlined = if automatic {
        Vec::new()
    } else {
        crate::placement_room::outline_holds(problem, references, user_rows)
    };
    if !cannot.is_empty() || !outlined.is_empty() {
        crate::placement_room::note_room(crate::placement_room::room(
            problem,
            references,
            !outlined.is_empty(),
        ));
        cannot.extend(outlined);
        return Err(cannot.join("; "));
    }
    Ok(())
}

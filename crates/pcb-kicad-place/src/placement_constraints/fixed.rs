// Copyright (C) 2026 Toit contributors.

//! `move_all` and `fixed`: which parts the placer moves once the other
//! constraints are applied, and the refusal of the constraints no
//! placement can keep.

use super::*;

/// Applies `move_all`: every part moves but locked or explicitly fixed ones, row members,
/// artwork and parts nobody can name; mounting holes no constraint names
/// go to the corners, then to the middle of the edges.
pub(super) fn apply_move_all(
    problem: &mut core::Problem,
    parts: &Parts,
    fixed: &[String],
    constrained: &BTreeSet<usize>,
    pinned: &[usize],
    configured_fixed: &[usize],
) {
    // Parts without nets move too: from a netlist they sit in the stack
    // with the rest. Mounting holes nobody placed go to the corners, as
    // designers put them, then to the middle of the edges.
    let named: BTreeSet<usize> = fixed
        .iter()
        .flat_map(|pattern| parts.matching(pattern))
        .chain(configured_fixed.iter().copied())
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
    let mut hole_spots = Vec::new();
    for index in 0..parts.footprints {
        // Artwork (a logo) occupies nothing and stays.
        let component = &problem.components[index];
        if parts.locked[index]
            || named.contains(&index)
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
            && !pinned.contains(&index)
            && let Some(sides) = spots.next()
        {
            hole_spots.push((index, sides));
        }
        problem.components[index].fixed = false;
    }
    let held: Vec<_> = problem
        .components
        .iter()
        .enumerate()
        .filter(|(part, component)| {
            component.fixed || pinned.contains(part) || named.contains(part)
        })
        .map(|(part, _)| part)
        .collect();
    for (index, sides) in hole_spots {
        apply_inferred_hole_edges(problem, index, &sides, &held);
    }
}

/// Keep the default corner when it has room for a hole. If the outline
/// or held parts cover it, try either adjoining edge before leaving the
/// hole free. User-authored edges never enter this choice.
fn apply_inferred_hole_edges(
    problem: &mut core::Problem,
    index: usize,
    preferred: &[Edge],
    held: &[usize],
) {
    const REACH: f64 = 1.001;
    let first = problem.constraints.edges.len();
    for sides in std::iter::once(preferred).chain(
        (preferred.len() == 2)
            .then(|| [&preferred[..1], &preferred[1..]])
            .into_iter()
            .flatten(),
    ) {
        problem
            .constraints
            .edges
            .extend(sides.iter().map(|&side| (index, side, REACH)));
        if hole_edge_has_room(problem, index, sides, held) {
            return;
        }
        problem.constraints.edges.truncate(first);
    }
}

fn hole_edge_has_room(
    problem: &core::Problem,
    index: usize,
    sides: &[Edge],
    held: &[usize],
) -> bool {
    let bounds = problem.bounds();
    let component = &problem.components[index];
    component
        .angle_options
        .iter()
        .copied()
        .chain([problem.poses[index].angle])
        .any(|angle| {
            let half = component.half_extent(angle);
            let offsets = core::constraints::copper_edge_offsets(problem, index, angle);
            let mut axes = [[0.0; 3]; 2];
            for axis in 0..2 {
                let (near, far) = if axis == 0 {
                    (Edge::Left, Edge::Right)
                } else {
                    (Edge::Top, Edge::Bottom)
                };
                axes[axis] = if sides.contains(&near) {
                    [0.1, 0.55, 0.99].map(|gap| bounds[axis] + offsets[axis] + half[axis] + gap)
                } else if sides.contains(&far) {
                    [0.1, 0.55, 0.99]
                        .map(|gap| bounds[axis + 2] - offsets[axis + 2] - half[axis] - gap)
                } else {
                    [0.25, 0.5, 0.75]
                        .map(|ratio| bounds[axis] + ratio * (bounds[axis + 2] - bounds[axis]))
                };
            }
            axes[0].into_iter().any(|x| {
                axes[1].into_iter().any(|y| {
                    let pose = core::Pose {
                        position: component.position_for_center([x, y], angle),
                        angle,
                    };
                    core::legal::is_legal(
                        problem,
                        &problem.poses,
                        index,
                        pose,
                        held.iter().copied(),
                    )
                })
            })
        })
}

/// Authored position constraints can move a locked part. A face-only
/// constraint can flip it, but its origin remains held.
pub(super) fn free_constrained(
    problem: &mut core::Problem,
    parts: &Parts,
    constraints: &KiCadPlacementConstraints,
    mut constrained: BTreeSet<usize>,
) {
    for pattern in constraints.back.iter().chain(&constraints.front) {
        constrained.extend(
            parts
                .matching(pattern)
                .into_iter()
                .filter(|&index| !parts.locked[index]),
        );
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

/// Fixes authored poses, explicit placer holds and constraints-file
/// `fixed` matches. A constraints glob that matches nothing is refused.
pub(super) fn fix_pinned_and_fixed(
    problem: &mut core::Problem,
    parts: &Parts,
    pinned: &[usize],
    fixed: &[String],
    configured_fixed: &[usize],
) -> Result<(), String> {
    for index in pinned.iter().chain(configured_fixed) {
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

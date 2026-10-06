// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Exact legality, legalization of a global placement, and
//! wirelength-driven detailed placement.

use crate::constraints;
use crate::problem::{Point, Pose, Problem, point_in_polygon};

#[derive(Clone, Copy, Debug)]
struct Rect {
    center: Point,
    half: Point,
    round: bool,
}

fn rect(problem: &Problem, index: usize, pose: Pose) -> Rect {
    let component = &problem.components[index];
    let half = component.half_extent(pose.angle);
    Rect {
        center: component.center(pose),
        half: [half[0] + component.halo, half[1] + component.halo],
        round: component.round,
    }
}

fn overlaps(a: Rect, b: Rect, spacing: f64) -> bool {
    let delta = [
        (a.center[0] - b.center[0]).abs(),
        (a.center[1] - b.center[1]).abs(),
    ];
    match (a.round, b.round) {
        (false, false) => {
            delta[0] < a.half[0] + b.half[0] + spacing - 1.0e-9
                && delta[1] < a.half[1] + b.half[1] + spacing - 1.0e-9
        }
        (true, true) => {
            delta[0].hypot(delta[1]) < a.half[0] + b.half[0] + spacing - 1.0e-9
        }
        _ => {
            let (disc, body) = if a.round { (a, b) } else { (b, a) };
            let outside = [
                (delta[0] - body.half[0]).max(0.0),
                (delta[1] - body.half[1]).max(0.0),
            ];
            outside[0].hypot(outside[1]) < disc.half[0] + spacing - 1.0e-9
        }
    }
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let orient = |p: Point, q: Point, r: Point| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (d1, d2, d3, d4) = (orient(c, d, a), orient(c, d, b), orient(a, b, c), orient(a, b, d));
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

fn inside_outline(problem: &Problem, body: Rect) -> bool {
    let corners = [
        [body.center[0] - body.half[0], body.center[1] - body.half[1]],
        [body.center[0] + body.half[0], body.center[1] - body.half[1]],
        [body.center[0] + body.half[0], body.center[1] + body.half[1]],
        [body.center[0] - body.half[0], body.center[1] + body.half[1]],
    ];
    if !corners.iter().all(|corner| point_in_polygon(*corner, &problem.outline)) {
        return false;
    }
    let outline = &problem.outline;
    for index in 0..outline.len() {
        let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
        for side in 0..4 {
            if segments_cross(a, b, corners[side], corners[(side + 1) % 4]) {
                return false;
            }
        }
    }
    true
}

/// The body alone, grown by `margin`, for tests against the board edge.
fn bare(problem: &Problem, index: usize, pose: Pose, margin: f64) -> Rect {
    let component = &problem.components[index];
    let half = component.half_extent(pose.angle);
    Rect {
        center: component.center(pose),
        half: [half[0] + margin, half[1] + margin],
        round: component.round,
    }
}

/// Whether the part keeps the edge margin inside the outline; an
/// overhanging part only with the share of its body that belongs on the
/// board.
fn on_board(problem: &Problem, index: usize, pose: Pose) -> bool {
    let component = &problem.components[index];
    if problem.constraints.edge_copper && !component.pads.is_empty() {
        let mut touching = [false; 4];
        for (part, edge, _) in &problem.constraints.edges {
            if *part == index {
                touching[match edge {
                    constraints::Edge::Left => 0,
                    constraints::Edge::Top => 1,
                    constraints::Edge::Right => 2,
                    constraints::Edge::Bottom => 3,
                }] = true;
            }
        }
        if touching.iter().any(|touch| *touch) {
            // Copper on the constrained side may reach the edge; elsewhere it
            // keeps the copper clearance (the edge margin is room for
            // routing, which a tab does not need).
            let margins = touching.map(|touch| {
                if touch { problem.constraints.copper_edge - 1.0e-6 } else { problem.min_spacing.min(problem.edge_margin) }
            });
            return component.pad_boxes(pose).iter().all(|(center, half)| {
                let grown = [
                    center[0] - half[0] - margins[0],
                    center[1] - half[1] - margins[1],
                    center[0] + half[0] + margins[2],
                    center[1] + half[1] + margins[3],
                ];
                inside_outline(
                    problem,
                    Rect {
                        center: [(grown[0] + grown[2]) / 2.0, (grown[1] + grown[3]) / 2.0],
                        half: [(grown[2] - grown[0]) / 2.0, (grown[3] - grown[1]) / 2.0],
                        round: false,
                    },
                )
            });
        }
    }
    if problem.constraints.is_empty() {
        let margin = problem.components[index].edge_margin(problem.edge_margin);
        return inside_outline(problem, bare(problem, index, pose, margin));
    }
    let inner = constraints::inner_box(problem, index, pose).unwrap_or_else(|| {
        let (center, half) = (
            problem.components[index].center(pose),
            problem.components[index].half_extent(pose.angle),
        );
        [center[0] - half[0], center[1] - half[1], center[0] + half[0], center[1] + half[1]]
    });
    // A side against a constrained edge may touch it: shrink by a hair so
    // the outline test does not sit on the boundary.
    let margins = constraints::side_margins(problem, index, pose.angle).map(|margin| {
        if margin > 0.0 { margin } else { -1.0e-6 }
    });
    let grown = [
        inner[0] - margins[0],
        inner[1] - margins[1],
        inner[2] + margins[2],
        inner[3] + margins[3],
    ];
    if grown[0] > grown[2] || grown[1] > grown[3] {
        return false;
    }
    inside_outline(
        problem,
        Rect {
            center: [(grown[0] + grown[2]) / 2.0, (grown[1] + grown[3]) / 2.0],
            half: [(grown[2] - grown[0]) / 2.0, (grown[3] - grown[1]) / 2.0],
            round: false,
        },
    )
}

/// Whether the body itself (without any margin) lies inside the outline.
pub fn body_inside_outline(problem: &Problem, index: usize, pose: Pose) -> bool {
    inside_outline(problem, bare(problem, index, pose, 0.0))
}

/// Whether the body grown by `reach` on every side touches the outline.
pub fn body_near_outline(problem: &Problem, index: usize, pose: Pose, reach: f64) -> bool {
    !inside_outline(problem, bare(problem, index, pose, reach))
}

/// Whether component `index` at `pose` is inside the board and clear of all
/// components in `others` (indices with their poses taken from `poses`).
pub fn is_legal(
    problem: &Problem,
    poses: &[Pose],
    index: usize,
    pose: Pose,
    others: impl Iterator<Item = usize>,
) -> bool {
    let body = rect(problem, index, pose);
    if !problem.components[index].fixed
        && (!on_board(problem, index, pose) || !constraints::hard_ok(problem, index, pose))
    {
        return false;
    }
    let side = problem.components[index].side;
    for other in others {
        if other == index {
            continue;
        }
        // Two fixed parts are the designer's responsibility.
        if problem.components[index].fixed && problem.components[other].fixed {
            continue;
        }
        let other_side = problem.components[other].side;
        if side.collides(other_side) {
            let (mine, theirs) = (&problem.components[index], &problem.components[other]);
            if mine.copper_only || theirs.copper_only {
                if copper_meets_cutout(problem, index, pose, other, poses[other]) {
                    return false;
                }
                continue;
            }
            let hollow = mine.hollow_for(theirs) || theirs.hollow_for(mine);
            if if hollow {
                hollow_overlap(problem, index, pose, other, poses[other])
            } else {
                if problem.constraints.linked(index, other) {
                    // Parts tied together meet without their halos.
                    let bare = |part: usize, pose: Pose| {
                        let rect = rect(problem, part, pose);
                        let halo = problem.components[part].halo;
                        Rect { half: [rect.half[0] - halo, rect.half[1] - halo], ..rect }
                    };
                    overlaps(bare(index, pose), bare(other, poses[other]), problem.spacing)
                } else {
                    overlaps(body, rect(problem, other, poses[other]), problem.spacing)
                }
            } {
                return false;
            }
        } else if side.opposite(other_side) && far_side_overlap(problem, index, pose, other, poses[other]) {
            return false;
        }
    }
    true
}

/// Whether a part's copper comes closer than the edge margin to a cutout
/// (either way round); its body may reach over the hole.
fn copper_meets_cutout(problem: &Problem, a: usize, pose_a: Pose, b: usize, pose_b: Pose) -> bool {
    let (hole, pose_hole, part, pose_part) = if problem.components[a].copper_only {
        (a, pose_a, b, pose_b)
    } else {
        (b, pose_b, a, pose_a)
    };
    if problem.components[part].copper_only {
        return false;
    }
    let hole = rect(problem, hole, pose_hole);
    let margin = problem.edge_margin;
    problem.components[part].pad_boxes(pose_part).into_iter().any(|(center, half)| {
        overlaps(
            Rect {
                center,
                half: [half[0] + margin, half[1] + margin],
                round: false,
            },
            hole,
            0.0,
        )
    })
}

/// Whether two parts on the same side meet where one of them is hollow:
/// only its blocking boxes count.
fn hollow_overlap(problem: &Problem, a: usize, pose_a: Pose, b: usize, pose_b: Pose) -> bool {
    let blocking = |index: usize, pose: Pose, other: usize| -> Vec<Rect> {
        let component = &problem.components[index];
        if !component.hollow_for(&problem.components[other]) {
            return vec![rect(problem, index, pose)];
        }
        component
            .blocking_boxes(&problem.components[other], pose)
            .into_iter()
            .map(|(center, half)| Rect {
                center,
                half,
                round: false,
            })
            .collect()
    };
    let theirs = blocking(b, pose_b, a);
    blocking(a, pose_a, b)
        .into_iter()
        .any(|mine| theirs.iter().any(|other| overlaps(mine, *other, problem.spacing)))
}

/// Whether a part's holes and pads on the far side meet the other part's
/// body there (either way round), for parts on opposite sides.
fn far_side_overlap(problem: &Problem, a: usize, pose_a: Pose, b: usize, pose_b: Pose) -> bool {
    let meets = |holes: usize, pose_holes: Pose, body: usize, pose_body: Pose| {
        let component = &problem.components[holes];
        if component.far_side.is_empty() {
            return false;
        }
        let far = component.far_boxes(pose_holes);
        if problem.far_side_pads_only {
            // Copper against copper only.
            let pads = problem.components[body].pad_boxes(pose_body);
            return far.iter().any(|(center, half)| {
                pads.iter().any(|(pad_center, pad_half)| {
                    overlaps(
                        Rect { center: *center, half: *half, round: false },
                        Rect { center: *pad_center, half: *pad_half, round: false },
                        problem.min_spacing,
                    )
                })
            });
        }
        let target = rect(problem, body, pose_body);
        far.into_iter().any(|(center, half)| {
            overlaps(
                Rect {
                    center,
                    half,
                    round: false,
                },
                target,
                problem.spacing,
            )
        })
    };
    meets(a, pose_a, b, pose_b) || meets(b, pose_b, a, pose_a)
}

pub fn illegal_components(problem: &Problem, poses: &[Pose]) -> Vec<usize> {
    (0..problem.components.len())
        .filter(|index| {
            !is_legal(problem, poses, *index, poses[*index], 0..problem.components.len())
        })
        .collect()
}


/// Moves every movable component to the nearest legal, grid-snapped position,
/// largest bodies first. Returns the components that found no position.
pub fn legalize(problem: &Problem, poses: &mut [Pose]) -> Vec<usize> {
    legalize_first(problem, poses, &[])
}

/// `legalize`, placing the parts of `first` before all others (the ones a
/// previous pass could not place: the big parts, placed first, can leave
/// them no room).
pub fn legalize_first(problem: &Problem, poses: &mut [Pose], first: &[usize]) -> Vec<usize> {
    legalize_keeping(problem, poses, first, &[])
}

/// `legalize_first`, with the parts of `keep` left at their poses (legal
/// already: a looser level of the same placement) and placed before all
/// others.
pub fn legalize_keeping(problem: &Problem, poses: &mut [Pose], first: &[usize], keep: &[usize]) -> Vec<usize> {
    let count = problem.components.len();
    let mut placed: Vec<usize> = (0..count)
        .filter(|index| problem.components[*index].fixed || keep.contains(index))
        .collect();
    let mut order: Vec<usize> = (0..count)
        .filter(|index| !problem.components[*index].fixed && !keep.contains(index))
        .collect();
    order.sort_by(|a, b| {
        let area = |index: &usize| {
            let size = problem.components[*index].body_size;
            size[0] * size[1]
        };
        let rank = |index: &usize| first.iter().position(|part| part == index).unwrap_or(usize::MAX);
        rank(a).cmp(&rank(b)).then(area(b).total_cmp(&area(a))).then(a.cmp(b))
    });
    let bounds = problem.bounds();
    let step = if problem.grid > 0.0 { problem.grid } else { 0.25 };
    let mut rings = (((bounds[2] - bounds[0]).max(bounds[3] - bounds[1])) / step).ceil() as i64 + 1;
    // A fine grid is a last resort after the coarse one searched the whole
    // board: it only looks near where the part wants to be (a part with no
    // spot scanned millions of positions otherwise).
    if step < 0.3 {
        rings = rings.min((15.0 / step).ceil() as i64);
    }
    let mut failed = Vec::new();
    for index in order {
        let component = &problem.components[index];
        let wanted = poses[index];
        let wanted_center = component.center(wanted);
        let mut best: Option<(f64, Pose)> = None;
        for angle in &component.angle_options {
            // Rotating about the body centre keeps the part where global
            // placement wanted it; prefer the global angle on ties.
            let penalty = if (angle - wanted.angle).abs() < 1.0e-9 {
                0.0
            } else {
                step * step
            };
            let ideal = component.position_for_center(wanted_center, *angle);
            let origin = constraints::snap_position(problem, index, ideal, *angle);
            'search: for ring in 0..=rings {
                // Once a legal spot exists, only slightly farther rings can
                // still hold a closer Euclidean match.
                if let Some((distance, _)) = best
                    && ((ring - 2).max(0) as f64 * step).powi(2) > distance
                {
                    break 'search;
                }
                for dy in -ring..=ring {
                    for dx in -ring..=ring {
                        if dx.abs().max(dy.abs()) != ring {
                            continue;
                        }
                        let pose = Pose {
                            position: [origin[0] + dx as f64 * step, origin[1] + dy as f64 * step],
                            angle: *angle,
                        };
                        let distance = (pose.position[0] - ideal[0]).powi(2)
                            + (pose.position[1] - ideal[1]).powi(2)
                            + penalty;
                        if best.is_some_and(|(best, _)| best <= distance) {
                            continue;
                        }
                        if is_legal(problem, poses, index, pose, placed.iter().copied()) {
                            best = Some((distance, pose));
                        }
                    }
                }
            }
        }
        match best {
            Some((_, pose)) => poses[index] = pose,
            None => failed.push(index),
        }
        placed.push(index);
    }
    failed
}

/// Places each part of `failed` where it displaces the least area of
/// movable parts (never a fixed one), then finds the displaced parts new
/// spots; keeps that only if all of them find one. A part whose spot is
/// taken by others finds no room by searching around them (katia's back
/// side: the designer's spots for its controllers existed, held by parts
/// placed first). Returns the parts still without room.
pub fn evict_for(problem: &Problem, poses: &mut [Pose], failed: &[usize]) -> Vec<usize> {
    let count = problem.components.len();
    let bounds = problem.bounds();
    let step = if problem.grid > 0.0 { problem.grid.max(0.5) } else { 0.5 };
    let mut still = Vec::new();
    let mut placed: Vec<usize> = (0..count).filter(|index| !failed.contains(index)).collect();
    for &index in failed {
        let component = &problem.components[index];
        let fixed: Vec<usize> = placed.iter().copied().filter(|other| problem.components[*other].fixed).collect();
        let movable: Vec<usize> = placed.iter().copied().filter(|other| !problem.components[*other].fixed).collect();
        // The spot whose displaced movable parts are smallest in area.
        let mut best: Option<(f64, Pose, Vec<usize>)> = None;
        let mut y = bounds[1];
        while y <= bounds[3] {
            let mut x = bounds[0];
            while x <= bounds[2] {
                for angle in &component.angle_options {
                    let pose = Pose {
                        position: constraints::snap_position(problem, index, [x, y], *angle),
                        angle: *angle,
                    };
                    if !is_legal(problem, poses, index, pose, fixed.iter().copied()) {
                        continue;
                    }
                    let mut area = 0.0;
                    let mut displaced = Vec::new();
                    for &other in &movable {
                        if !is_legal(problem, poses, index, pose, std::iter::once(other)) {
                            let size = problem.components[other].body_size;
                            area += size[0] * size[1];
                            displaced.push(other);
                            if best.as_ref().is_some_and(|(best, _, _)| area >= *best) {
                                break;
                            }
                        }
                    }
                    if best.as_ref().is_none_or(|(best, _, _)| area < *best) {
                        best = Some((area, pose, displaced));
                    }
                }
                x += step;
            }
            y += step;
        }
        let Some((_, pose, displaced)) = best else {
            still.push(index);
            continue;
        };
        let saved: Vec<Pose> = poses.to_vec();
        poses[index] = pose;
        let mut settled: Vec<usize> = placed.iter().copied().filter(|other| !displaced.contains(other)).collect();
        settled.push(index);
        let mut all = true;
        for &moved in &displaced {
            let component = &problem.components[moved];
            let mut spot: Option<(f64, Pose)> = None;
            let wanted = poses[moved].position;
            let mut y = bounds[1];
            while y <= bounds[3] {
                let mut x = bounds[0];
                while x <= bounds[2] {
                    for angle in &component.angle_options {
                        let candidate = Pose {
                            position: constraints::snap_position(problem, moved, [x, y], *angle),
                            angle: *angle,
                        };
                        let distance = (candidate.position[0] - wanted[0]).powi(2) + (candidate.position[1] - wanted[1]).powi(2);
                        if spot.is_some_and(|(best, _)| best <= distance) {
                            continue;
                        }
                        if is_legal(problem, poses, moved, candidate, settled.iter().copied()) {
                            spot = Some((distance, candidate));
                        }
                    }
                    x += step;
                }
                y += step;
            }
            match spot {
                Some((_, candidate)) => {
                    poses[moved] = candidate;
                    settled.push(moved);
                }
                None => {
                    all = false;
                    break;
                }
            }
        }
        if all {
            placed.push(index);
        } else {
            poses.copy_from_slice(&saved);
            still.push(index);
        }
    }
    still
}

/// For every part that misses a near or relative constraint, searches legal
/// positions around what the relation wants and takes the one with the
/// least relation penalty plus wirelength, if that beats where it is.
/// Returns the number of parts moved.
pub fn repair_relations(problem: &Problem, poses: &mut [Pose]) -> usize {
    let constraints = &problem.constraints;
    if constraints.relations.is_empty() {
        return 0;
    }
    let count = problem.components.len();
    let step = if problem.grid > 0.0 { problem.grid } else { 0.25 };
    let mut moved = 0;
    for relation in &constraints.relations {
        let Some(index) = relation.parts()[0] else {
            continue;
        };
        if problem.components[index].fixed
            || constraints::relation_violation(problem, poses, relation) <= 0.0
        {
            continue;
        }
        let component = &problem.components[index];
        let score = |poses: &[Pose]| {
            constraints::relation_penalty(problem, poses, Some(index)) + problem.wirelength(poses)
        };
        let original = poses[index];
        let mut best = (score(poses), original);
        let target = constraints::relation_target(problem, poses, relation);
        let reach = component.body_size[0].max(component.body_size[1]) + 6.0;
        let rings = (reach / step).ceil() as i64;
        for angle in &component.angle_options {
            let ideal = component.position_for_center(target, *angle);
            let origin = constraints::snap_position(problem, index, ideal, *angle);
            for ring in 0..=rings {
                for dy in -ring..=ring {
                    for dx in -ring..=ring {
                        if dx.abs().max(dy.abs()) != ring {
                            continue;
                        }
                        let pose = Pose {
                            position: [origin[0] + dx as f64 * step, origin[1] + dy as f64 * step],
                            angle: *angle,
                        };
                        if !is_legal(problem, poses, index, pose, 0..count) {
                            continue;
                        }
                        poses[index] = pose;
                        let candidate = score(poses);
                        poses[index] = original;
                        if candidate < best.0 - 1.0e-9 {
                            best = (candidate, pose);
                        }
                    }
                }
            }
        }
        if best.1 != original {
            poses[index] = best.1;
            moved += 1;
        }
    }
    moved
}

/// Turns parts of the same kind (same body and pin count, same side) to
/// their kind's most common angle where that stays legal, costs at most
/// `tolerance` mm of wire per part and misses no soft constraint more:
/// consistent orientation eases assembly and inspection. Returns the number
/// of parts turned.
pub fn align_orientations(problem: &Problem, poses: &mut [Pose], tolerance: f64) -> usize {
    let count = problem.components.len();
    let key = |index: usize| {
        let component = &problem.components[index];
        (
            (component.body_size[0] * 100.0).round() as i64,
            (component.body_size[1] * 100.0).round() as i64,
            component.pins.len(),
            component.side as u8,
        )
    };
    let mut kinds: std::collections::BTreeMap<(i64, i64, usize, u8), Vec<usize>> = Default::default();
    for index in 0..count {
        let component = &problem.components[index];
        if !component.fixed && component.pins.len() >= 2 && component.angle_options.len() > 1 {
            kinds.entry(key(index)).or_default().push(index);
        }
    }
    let angle_key = |angle: f64| (angle.rem_euclid(360.0).round() as i64) % 360;
    let mut turned = 0;
    for members in kinds.values().filter(|members| members.len() >= 2) {
        let mut counts: std::collections::BTreeMap<i64, usize> = Default::default();
        for index in members {
            *counts.entry(angle_key(poses[*index].angle)).or_default() += 1;
        }
        let (&modal, _) = counts.iter().max_by_key(|(angle, count)| (**count, -**angle)).unwrap();
        for &index in members {
            if angle_key(poses[index].angle) == modal {
                continue;
            }
            let component = &problem.components[index];
            let Some(&angle) = component.angle_options.iter().find(|option| angle_key(**option) == modal) else {
                continue;
            };
            let center = component.center(poses[index]);
            let position = constraints::snap_position(problem, index, component.position_for_center(center, angle), angle);
            let pose = Pose { position, angle };
            if !is_legal(problem, poses, index, pose, 0..count) {
                continue;
            }
            let before = (problem.wirelength(poses), constraints::relation_penalty(problem, poses, Some(index)));
            let original = poses[index];
            poses[index] = pose;
            let after = (problem.wirelength(poses), constraints::relation_penalty(problem, poses, Some(index)));
            if after.0 <= before.0 + tolerance && after.1 <= before.1 + 1.0e-9 {
                turned += 1;
            } else {
                poses[index] = original;
            }
        }
    }
    turned
}

/// Local search on a legal placement: every move keeps it legal and strictly
/// reduces the weighted half-perimeter wirelength.
pub fn refine(problem: &Problem, poses: &mut [Pose], passes: usize) -> usize {
    let count = problem.components.len();
    let nets = problem.net_weights.len();
    let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nets];
    for (index, component) in problem.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            members[description.net].push((index, pin));
        }
    }
    let incident: Vec<Vec<usize>> = problem
        .components
        .iter()
        .map(|component| {
            let mut nets: Vec<usize> = component.pins.iter().map(|pin| pin.net).collect();
            nets.sort_unstable();
            nets.dedup();
            nets
        })
        .collect();
    let cost = |poses: &[Pose], nets: &[usize]| -> f64 {
        nets.iter()
            .map(|net| {
                let mut bounds = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for (other, pin) in &members[*net] {
                    let at = problem.components[*other]
                        .pin_position(&problem.components[*other].pins[*pin], poses[*other]);
                    bounds[0] = bounds[0].min(at[0]);
                    bounds[1] = bounds[1].min(at[1]);
                    bounds[2] = bounds[2].max(at[0]);
                    bounds[3] = bounds[3].max(at[1]);
                }
                if members[*net].len() < 2 {
                    0.0
                } else {
                    problem.net_weights[*net] * ((bounds[2] - bounds[0]) + (bounds[3] - bounds[1]))
                }
            })
            .sum()
    };

    let mut improvements = 0;
    for _ in 0..passes {
        let mut improved = false;
        for index in 0..count {
            let component = &problem.components[index];
            if component.fixed || component.pins.is_empty() {
                continue;
            }
            // Where the other pins of this component's nets pull it.
            let mut target = [0.0; 2];
            let mut weight = 0.0;
            for net in &incident[index] {
                for (other, pin) in &members[*net] {
                    if *other == index {
                        continue;
                    }
                    let at = problem.components[*other]
                        .pin_position(&problem.components[*other].pins[*pin], poses[*other]);
                    let pull = problem.net_weights[*net] / (members[*net].len() - 1) as f64;
                    target[0] += pull * at[0];
                    target[1] += pull * at[1];
                    weight += pull;
                }
            }
            if weight == 0.0 {
                continue;
            }
            target = [target[0] / weight, target[1] / weight];
            let original = poses[index];
            let score = |poses: &[Pose]| {
                cost(poses, &incident[index]) + constraints::relation_penalty(problem, poses, Some(index))
            };
            let mut best = (score(poses), original);
            let center = component.center(original);
            // Besides the pull of its nets, a part that misses a relation
            // is drawn towards what it relates to.
            let mut targets = vec![target];
            for relation in &problem.constraints.relations {
                if relation.parts()[0] != Some(index)
                    || constraints::relation_violation(problem, poses, relation) <= 0.0
                {
                    continue;
                }
                targets.push(constraints::relation_target(problem, poses, relation));
            }
            for (angle, target) in component
                .angle_options
                .iter()
                .flat_map(|angle| targets.iter().map(move |target| (angle, *target)))
            {
                for fraction in [1.0, 0.75, 0.5, 0.25, 0.125, 0.0] {
                    let wanted = [
                        center[0] + fraction * (target[0] - center[0]),
                        center[1] + fraction * (target[1] - center[1]),
                    ];
                    let position = component.position_for_center(wanted, *angle);
                    let pose = Pose {
                        position: constraints::snap_position(problem, index, position, *angle),
                        angle: *angle,
                    };
                    if pose == original {
                        continue;
                    }
                    poses[index] = pose;
                    let candidate = score(poses);
                    if candidate < best.0 - 1.0e-9 && is_legal(problem, poses, index, pose, 0..count)
                    {
                        best = (candidate, pose);
                    }
                }
            }
            poses[index] = best.1;
            if best.1 != original {
                improved = true;
                improvements += 1;
            }
        }

        // Exchange equally sized parts.
        for a in 0..count {
            for b in a + 1..count {
                let (first, second) = (&problem.components[a], &problem.components[b]);
                if first.fixed
                    || second.fixed
                    || first.side != second.side
                    || first.round != second.round
                    || !first.angle_options.contains(&poses[b].angle)
                    || !second.angle_options.contains(&poses[a].angle)
                    || (first.body_size[0] - second.body_size[0]).abs() > 1.0e-6
                    || (first.body_size[1] - second.body_size[1]).abs() > 1.0e-6
                    || (first.body_center[0] - second.body_center[0]).abs() > 1.0e-6
                    || (first.body_center[1] - second.body_center[1]).abs() > 1.0e-6
                {
                    continue;
                }
                let mut nets = incident[a].clone();
                nets.extend(&incident[b]);
                nets.sort_unstable();
                nets.dedup();
                let pair = |poses: &[Pose]| {
                    cost(poses, &nets)
                        + constraints::relation_penalty(problem, poses, Some(a))
                        + constraints::relation_penalty(problem, poses, Some(b))
                };
                let before = pair(poses);
                poses.swap(a, b);
                if pair(poses) < before - 1.0e-9
                    && is_legal(problem, poses, a, poses[a], 0..count)
                    && is_legal(problem, poses, b, poses[b], 0..count)
                {
                    improved = true;
                    improvements += 1;
                } else {
                    poses.swap(a, b);
                }
            }
        }
        if !improved {
            break;
        }
    }
    improvements
}

/// Parts placed by their copper at an edge (a card edge in its tab) have
/// little play along that edge: each with less than 2 mm moves to the
/// middle of the stretch where it stays legal, away from the tab's sides.
pub fn center_edge_copper(problem: &Problem, poses: &mut [Pose]) {
    if !problem.constraints.edge_copper {
        return;
    }
    let step = if problem.grid > 0.0 { problem.grid } else { 0.05 };
    let everyone = 0..problem.components.len();
    for (part, edge, _) in problem.constraints.edges.clone() {
        if problem.components[part].fixed || problem.components[part].pads.is_empty() {
            continue;
        }
        let pose = poses[part];
        if !is_legal(problem, poses, part, pose, everyone.clone()) {
            continue;
        }
        // Along the edge: x for the top and bottom, y for the sides.
        let axis = match edge {
            constraints::Edge::Top | constraints::Edge::Bottom => 0,
            constraints::Edge::Left | constraints::Edge::Right => 1,
        };
        let reach = |direction: f64| {
            let mut last = 0.0;
            for count in 1..=200 {
                let mut moved = pose;
                moved.position[axis] += direction * step * count as f64;
                if !is_legal(problem, poses, part, moved, everyone.clone()) {
                    break;
                }
                last = direction * step * count as f64;
            }
            last
        };
        let (low, high) = (reach(-1.0), reach(1.0));
        if high - low > 2.0 {
            continue;
        }
        let middle = ((low + high) / 2.0 / step).round() * step;
        let mut centered = pose;
        centered.position[axis] += middle;
        if middle != 0.0 && is_legal(problem, poses, part, centered, everyone.clone()) {
            poses[part] = centered;
        }
    }
}

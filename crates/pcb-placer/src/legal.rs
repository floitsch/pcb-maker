// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Exact legality, legalization of a global placement, and
//! wirelength-driven detailed placement.

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

/// Whether the body itself (without any margin) lies inside the outline.
pub fn body_inside_outline(problem: &Problem, index: usize, pose: Pose) -> bool {
    inside_outline(problem, bare(problem, index, pose, 0.0))
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
    if !problem.components[index].fixed && !inside_outline(problem, bare(problem, index, pose, problem.edge_margin)) {
        return false;
    }
    let side = problem.components[index].side;
    for other in others {
        if other == index || !side.collides(problem.components[other].side) {
            continue;
        }
        // Two fixed parts are the designer's responsibility.
        if problem.components[index].fixed && problem.components[other].fixed {
            continue;
        }
        if overlaps(body, rect(problem, other, poses[other]), problem.spacing) {
            return false;
        }
    }
    true
}

pub fn illegal_components(problem: &Problem, poses: &[Pose]) -> Vec<usize> {
    (0..problem.components.len())
        .filter(|index| {
            !is_legal(problem, poses, *index, poses[*index], 0..problem.components.len())
        })
        .collect()
}

fn snap(value: f64, grid: f64) -> f64 {
    if grid > 0.0 {
        (value / grid).round() * grid
    } else {
        value
    }
}

/// Moves every movable component to the nearest legal, grid-snapped position,
/// largest bodies first. Returns the components that found no position.
pub fn legalize(problem: &Problem, poses: &mut [Pose]) -> Vec<usize> {
    let count = problem.components.len();
    let mut placed: Vec<usize> = (0..count)
        .filter(|index| problem.components[*index].fixed)
        .collect();
    let mut order: Vec<usize> = (0..count)
        .filter(|index| !problem.components[*index].fixed)
        .collect();
    order.sort_by(|a, b| {
        let area = |index: &usize| {
            let size = problem.components[*index].body_size;
            size[0] * size[1]
        };
        area(b).total_cmp(&area(a)).then(a.cmp(b))
    });
    let bounds = problem.bounds();
    let step = if problem.grid > 0.0 { problem.grid } else { 0.25 };
    let rings = (((bounds[2] - bounds[0]).max(bounds[3] - bounds[1])) / step).ceil() as i64 + 1;
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
            let origin = [snap(ideal[0], problem.grid), snap(ideal[1], problem.grid)];
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
            let mut best = (cost(poses, &incident[index]), original);
            let center = component.center(original);
            for angle in &component.angle_options {
                for fraction in [1.0, 0.75, 0.5, 0.25, 0.125, 0.0] {
                    let wanted = [
                        center[0] + fraction * (target[0] - center[0]),
                        center[1] + fraction * (target[1] - center[1]),
                    ];
                    let position = component.position_for_center(wanted, *angle);
                    let pose = Pose {
                        position: [snap(position[0], problem.grid), snap(position[1], problem.grid)],
                        angle: *angle,
                    };
                    if pose == original {
                        continue;
                    }
                    poses[index] = pose;
                    let candidate = cost(poses, &incident[index]);
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
                let before = cost(poses, &nets);
                poses.swap(a, b);
                if cost(poses, &nets) < before - 1.0e-9
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

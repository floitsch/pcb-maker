// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The orientation search's canonical start (`Config::orientation_search`).
//!
//! The placer turns parts in global placement (`global::choose_rotations`),
//! in the annealer (quarter turns), the legaliser and the refinement, but
//! every one of those steps started from the angles the board held and
//! walked the angle options in an order relative to them: the same part
//! at 0 degrees (an import from a schematic) and at the designer's 90
//! degrees (the layout tier's stack) drew other random turns and broke
//! ties the other way, so the placement depended on the angles it was
//! handed. Here every movable part starts at the first of its options in
//! a fixed order, so the search finds the same placement from any start.

use crate::problem::{Pose, Problem};

/// An angle's place in the canonical order: [0, 360).
pub fn angle_key(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(360.0);
    if (wrapped - 360.0).abs() < 1.0e-9 {
        0.0
    } else {
        wrapped
    }
}

/// Sorts angle options into the canonical order (by `angle_key`) and drops
/// repeats (within a millionth of a degree).
pub fn sort_angles(options: &mut Vec<f64>) {
    options.sort_by(|a, b| angle_key(*a).total_cmp(&angle_key(*b)));
    options.dedup_by(|a, b| (angle_key(*a) - angle_key(*b)).abs() < 1.0e-6);
}

/// Puts the problem in the canonical start: every part's angle options in
/// the canonical order, and every movable part that may turn at its first
/// option, turned about its body centre. Fixed parts keep their poses, a
/// part with one option (a `rotation` constraint) has it already, and a
/// row's leader keeps its angle: the row's body is built in the leader's
/// frame at the angle it had. Returns the number of parts turned.
pub fn canonical_start(problem: &mut Problem) -> usize {
    let leaders: std::collections::BTreeSet<usize> = problem
        .constraints
        .followers
        .iter()
        .map(|follower| follower.leader)
        .collect();
    let mut turned = 0;
    for index in 0..problem.components.len() {
        sort_angles(&mut problem.components[index].angle_options);
        let component = &problem.components[index];
        if component.fixed || component.angle_options.len() < 2 || leaders.contains(&index) {
            continue;
        }
        let pose = problem.poses[index];
        let angle = component.angle_options[0];
        if pose.angle == angle {
            continue;
        }
        let center = component.center(pose);
        problem.poses[index] = Pose {
            position: component.position_for_center(center, angle),
            angle,
        };
        turned += 1;
    }
    let mut poses = problem.poses.clone();
    problem.sync_followers(&mut poses);
    problem.poses = poses;
    turned
}

/// Escape demand (`Problem::escape`): how much of the room in front of
/// part `index`'s dense sides (at `poses[index]`) other bodies take. Per
/// side, its demand times the share of a band as deep as the demand's
/// tracks need that `bodies` (every part's box, the part's own skipped;
/// only parts on a colliding side) cover; the board edge is free room.
pub fn escape_blocking(
    problem: &Problem,
    escape: &crate::escape::Escape,
    members: &[Vec<(usize, usize)>],
    poses: &[Pose],
    index: usize,
    bodies: &[(crate::problem::Point, crate::problem::Point)],
) -> f64 {
    let mut exits = Vec::new();
    crate::escape::exits(problem, escape, members, poses, index, &mut exits);
    let demand = crate::escape::demand(&exits);
    if demand.iter().all(|count| *count == 0) {
        return 0.0;
    }
    let (center, half) = crate::escape::body_box(problem, index, poses[index]);
    let low = [center[0] - half[0], center[1] - half[1]];
    let high = [center[0] + half[0], center[1] + half[1]];
    let side = problem.components[index].side;
    let mut blocked = 0.0;
    for (direction, count) in demand.iter().enumerate() {
        if *count == 0 {
            continue;
        }
        let depth = (*count as f64 * escape.track_room()).max(escape.pitch);
        let band = match direction {
            0 => [low[0] - depth, low[1], low[0], high[1]],
            1 => [high[0], low[1], high[0] + depth, high[1]],
            2 => [low[0], low[1] - depth, high[0], low[1]],
            _ => [low[0], high[1], high[0], high[1] + depth],
        };
        let area = (band[2] - band[0]) * (band[3] - band[1]);
        if area <= 0.0 {
            continue;
        }
        let mut covered = 0.0;
        for (other, (other_center, other_half)) in bodies.iter().enumerate() {
            if other == index || !problem.components[other].side.collides(side) {
                continue;
            }
            let width = (band[2].min(other_center[0] + other_half[0])
                - band[0].max(other_center[0] - other_half[0]))
            .max(0.0);
            let height = (band[3].min(other_center[1] + other_half[1])
                - band[1].max(other_center[1] - other_half[1]))
            .max(0.0);
            covered += width * height;
        }
        crate::add_work(bodies.len() as u64);
        blocked += *count as f64 * (covered / area).min(1.0);
    }
    blocked
}

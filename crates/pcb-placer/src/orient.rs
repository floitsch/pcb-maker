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

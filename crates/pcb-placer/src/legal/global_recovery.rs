// Copyright (C) 2026 Florian Loitsch.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Try a sparse subset of the placement grid for parts left without a seat.

use super::{
    Apart, Buckets, illegal_components, legalize_keeping_until, region_window, ring_steps,
    window_rings, window_steps,
};
use crate::constraints;
use crate::{Pose, Problem, add_work, work};
use std::time::Instant;

const SAMPLE_SPACING_MM: f64 = 2.0;

/// Seat the failed parts around the existing placement, on the same grid
/// and under the same geometry and constraints. The failed parts' tentative
/// poses do not reserve space. Commit only after the whole group fits.
pub(crate) fn recover_unplaced(
    problem: &Problem,
    poses: &mut [Pose],
    failed: &[usize],
    stop: impl Fn() -> bool,
) -> bool {
    let mut pending = vec![false; poses.len()];
    for &index in failed {
        pending[index] = true;
    }
    let mut buckets = Buckets::new(problem);
    for (index, &pose) in poses.iter().enumerate() {
        if !pending[index] {
            buckets.insert(problem, index, pose);
        }
    }
    let mut trial = poses.to_vec();
    let mut apart = Apart::new(problem);
    let mut neighbors = Vec::new();
    let grid = if problem.grid > 0.0 {
        problem.grid
    } else {
        0.25
    };
    let step = (SAMPLE_SPACING_MM / grid).ceil().max(1.0) * grid;
    let bounds = problem.bounds();
    for &index in failed {
        let component = &problem.components[index];
        let wanted = poses[index];
        let center = component.center(wanted);
        let relations: Vec<_> = problem
            .constraints
            .relations
            .iter()
            .filter(|relation| relation.involves(index))
            .map(|relation| {
                (
                    relation,
                    constraints::relation_violation(problem, poses, relation),
                )
            })
            .collect();
        let mut best = None::<(f64, Pose)>;
        for &angle in &component.angle_options {
            let origin = constraints::snap_position(
                problem,
                index,
                component.position_for_center(center, angle),
                angle,
            );
            let origin_center = component.center(Pose {
                position: origin,
                angle,
            });
            let region = region_window(problem, index, angle).map_or(bounds, |region| {
                [
                    region[0].max(bounds[0]),
                    region[1].max(bounds[1]),
                    region[2].min(bounds[2]),
                    region[3].min(bounds[3]),
                ]
            });
            if region[0] > region[2] || region[1] > region[3] {
                continue;
            }
            let window = window_steps(Some(region), origin_center, step).expect("bounded search");
            let (nearest, farthest) = window_rings(window);
            for ring in nearest..=farthest {
                if best.is_some_and(|(score, _)| {
                    ((ring as f64 * step - grid).max(0.0)).powi(2) > score
                }) {
                    break;
                }
                for (dx, dy) in ring_steps(ring, window) {
                    if stop() {
                        return false;
                    }
                    add_work(1);
                    let pose = Pose {
                        position: [origin[0] + dx as f64 * step, origin[1] + dy as f64 * step],
                        angle,
                    };
                    let here = component.center(pose);
                    let turn = if angle == wanted.angle {
                        0.0
                    } else {
                        grid * grid
                    };
                    let score =
                        (here[0] - center[0]).powi(2) + (here[1] - center[1]).powi(2) + turn;
                    if best.is_some_and(|(best, _)| score >= best) {
                        continue;
                    }
                    buckets.near(problem, index, pose, &mut neighbors);
                    if !apart.is_legal(problem, &trial, index, pose, neighbors.iter().copied()) {
                        continue;
                    }
                    trial[index] = pose;
                    add_work(constraints::relation_work(problem, index));
                    if relations.iter().all(|&(relation, before)| {
                        constraints::relation_violation(problem, &trial, relation) <= before
                    }) {
                        best = Some((score, pose));
                    }
                }
            }
        }
        let Some((_, pose)) = best else {
            return false;
        };
        trial[index] = pose;
        buckets.insert(problem, index, pose);
    }
    poses.copy_from_slice(&trial);
    true
}

/// If nearby recovery cannot open a seat, let one failed part choose
/// first and reseat the others on a subset of the current placement grid.
/// Validate the result with the original problem, including its authored
/// relations. The converged grid and geometry of the level stay in force.
pub(crate) fn repack_one_unplaced(
    problem: &Problem,
    poses: &mut [Pose],
    failed: &[usize],
    limit: u64,
    deadline: Option<Instant>,
) -> Option<usize> {
    let &[first] = failed else {
        return None;
    };
    let expired = || work() >= limit || deadline.is_some_and(|end| Instant::now() >= end);
    if problem.components[first].fixed || !(problem.grid > 0.0 && problem.grid < 0.3) || expired() {
        return None;
    }
    let mut coarse = problem.clone();
    coarse.grid = (0.5 / problem.grid).ceil() * problem.grid;
    let mut trial = poses.to_vec();
    let remaining = legalize_keeping_until(&coarse, &mut trial, failed, &[], Some(limit), deadline);
    if !remaining.is_empty() || expired() {
        return None;
    }
    problem.sync_followers(&mut trial);
    if problem
        .components
        .iter()
        .enumerate()
        .any(|(index, part)| part.fixed && trial[index] != poses[index])
        || !illegal_components(problem, &trial).is_empty()
        || constraints::report(problem, &trial)
            .into_iter()
            .zip(constraints::report(problem, poses))
            .any(|(after, before)| after.violation > before.violation + 1e-6)
        || constraints::report_kept(problem, &trial)
            .into_iter()
            .any(|status| !status.satisfied)
        || expired()
    {
        return None;
    }
    let moved = trial
        .iter()
        .zip(poses.iter())
        .enumerate()
        .filter(|(index, (a, b))| *index != first && a != b)
        .count();
    poses.copy_from_slice(&trial);
    Some(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legal::{is_legal, tests::pair};

    fn blocked_start() -> (Problem, Vec<Pose>) {
        let (mut problem, mut poses) = pair(0.0, 0.0);
        problem.grid = 0.635;
        problem.components[0].fixed = true;
        problem.components[0].body_size = [35.0, 22.0];
        problem.components[1].angle_options = vec![90.0];
        problem.constraints.regions.push((1, [1.0, 1.0, 39.0, 8.0]));
        poses[0].position = [20.0, 20.0];
        poses[1].position = [20.0, 22.0];
        poses[1].angle = 90.0;
        (problem, poses)
    }

    #[test]
    fn recovers_a_seat_far_from_a_blocked_pose_on_the_existing_grid() {
        let (problem, mut poses) = blocked_start();
        let held = poses[0];
        assert!(!is_legal(&problem, &poses, 1, poses[1], [0].into_iter()));
        assert!(recover_unplaced(&problem, &mut poses, &[1], || false));
        assert_eq!(poses[0], held);
        assert_eq!(poses[1].angle, 90.0);
        assert!(poses[1].position[1] < 8.0);
        assert!(is_legal(&problem, &poses, 1, poses[1], [0].into_iter()));
        for coordinate in poses[1].position {
            let steps = coordinate / problem.grid;
            assert!((steps - steps.round()).abs() < 1.0e-10);
        }
    }

    #[test]
    fn failed_or_interrupted_recovery_keeps_every_pose() {
        let (mut problem, mut poses) = blocked_start();
        problem.components.push(problem.components[1].clone());
        problem
            .constraints
            .regions
            .push((2, [17.0, 17.0, 23.0, 23.0]));
        poses.push(poses[1]);
        let original = poses.clone();
        assert!(!recover_unplaced(&problem, &mut poses, &[1, 2], || false));
        assert_eq!(poses, original);
        let before = crate::work();
        assert!(!recover_unplaced(&problem, &mut poses, &[1], || {
            crate::work() > before
        }));
        assert_eq!(poses, original);
    }

    #[test]
    fn recovery_keeps_an_authored_pad_distance() {
        let (mut problem, mut poses) = blocked_start();
        let relation = constraints::Relation::PadNear {
            source: constraints::Anchor::Point(1, [0.0, 0.0]),
            anchor: constraints::Anchor::Point(0, [0.0, 0.0]),
            max: Some(3.0),
            pad_numbers: None,
        };
        assert_eq!(
            constraints::relation_violation(&problem, &poses, &relation),
            0.0
        );
        problem.constraints.relations.push(relation);
        let original = poses.clone();
        assert!(!recover_unplaced(&problem, &mut poses, &[1], || false));
        assert_eq!(poses, original);
    }

    fn occupied_seat() -> (Problem, Vec<Pose>) {
        let (mut problem, mut poses) = pair(0.0, 0.0);
        for part in &mut problem.components {
            part.body_size = [4.0, 4.0];
        }
        let mut held = problem.components[0].clone();
        held.fixed = true;
        problem.components.push(held);
        poses[1].position = [20.0, 20.0];
        poses.push(Pose {
            position: [30.0, 30.0],
            angle: 0.0,
        });
        problem.poses = poses.clone();
        problem
            .constraints
            .regions
            .push((1, [7.5, 7.5, 12.5, 12.5]));
        (problem, poses)
    }

    #[test]
    fn priority_scan_can_reseat_an_incumbent_on_the_original_grid() {
        let (problem, mut poses) = occupied_seat();
        let held = poses[2];
        assert!(!recover_unplaced(&problem, &mut poses, &[1], || false));
        assert_eq!(
            repack_one_unplaced(&problem, &mut poses, &[1], work() + 30_000_000, None),
            Some(1)
        );
        assert_eq!(poses[2], held);
        assert!(illegal_components(&problem, &poses).is_empty());
        assert!(
            poses
                .iter()
                .all(|pose| pose.position.into_iter().all(|value| {
                    let on_original_grid = value / problem.grid;
                    (on_original_grid.round() - on_original_grid).abs() < 1e-9
                }))
        );
    }

    #[test]
    fn priority_scan_preserves_authored_distances_and_obeys_its_budget() {
        let (mut problem, mut poses) = occupied_seat();
        let original = poses.clone();
        assert_eq!(
            repack_one_unplaced(&problem, &mut poses, &[1], work(), None),
            None
        );
        problem
            .constraints
            .relations
            .push(constraints::Relation::PadNear {
                source: constraints::Anchor::Point(0, [0.0, 0.0]),
                anchor: constraints::Anchor::Point(2, [-20.0, -20.0]),
                max: Some(0.2),
                pad_numbers: None,
            });
        assert_eq!(
            repack_one_unplaced(&problem, &mut poses, &[1], work() + 30_000_000, None),
            None
        );
        assert_eq!(poses, original);
    }
}

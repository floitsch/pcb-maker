// Copyright (C) 2026 Florian Loitsch.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Refine a legal placement towards the requested pad centre at subgrid resolution.

use super::{Apart, body_spacing, constraints, rect};
use crate::{Pose, Problem, constraints::Relation};

/// Resolve a contact to one micron and leave that much slack for serialization.
const CONTACT_SLACK_MM: f64 = 0.001;
const SEARCH_STEPS: usize = 16;
const AXIS_CONTACTS: usize = 12;

/// Closest coordinate projections onto neighbouring body faces at each
/// allowed orientation. A face and a neighbouring face can define a corner
/// that sliding towards the pad from the original pose could never reach.
fn face_contacts(
    problem: &Problem,
    poses: &[Pose],
    relation: &Relation,
    index: usize,
    distance: f64,
) -> Vec<(f64, Pose)> {
    let component = &problem.components[index];
    let mut candidates = Vec::new();
    let mut work = 0;
    for &angle in &component.angle_options {
        if crate::out_of_work(work) {
            break;
        }
        let goal = constraints::relation_target_at(problem, poses, relation, angle);
        let origin = component.position_for_center(goal, angle);
        let own = rect(
            problem,
            index,
            Pose {
                position: origin,
                angle,
            },
        );
        let mut axes = [vec![0.0], vec![0.0]];
        for (other, &pose) in poses.iter().enumerate() {
            work += 1;
            if index == other || !component.side.collides(problem.components[other].side) {
                continue;
            }
            let other_rect = rect(problem, other, pose);
            let gap = body_spacing(problem, index, other) + CONTACT_SLACK_MM;
            let reach: [f64; 2] = std::array::from_fn(|i| own.half[i] + other_rect.half[i] + gap);
            let delta: [f64; 2] = std::array::from_fn(|i| other_rect.center[i] - own.center[i]);
            if (0..2).any(|i| delta[i].abs() > reach[i] + distance) {
                continue;
            }
            for i in 0..2 {
                axes[i].extend([delta[i] - reach[i], delta[i] + reach[i]]);
            }
        }
        for axis in &mut axes {
            axis.sort_by(|a, b| a.abs().total_cmp(&b.abs()).then(a.total_cmp(b)));
            axis.dedup_by(|a, b| (*a - *b).abs() < CONTACT_SLACK_MM);
            axis.truncate(AXIS_CONTACTS);
        }
        for &x in &axes[0] {
            for &y in &axes[1] {
                work += 1;
                let gap = x.hypot(y);
                if gap < distance - 1e-9 {
                    candidates.push((
                        gap,
                        Pose {
                            position: [origin[0] + x, origin[1] + y],
                            angle,
                        },
                    ));
                }
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    crate::add_work(work);
    candidates
}

pub(crate) fn refine_pad_near(
    problem: &Problem,
    poses: &mut [Pose],
    unplaced: &[usize],
    include_limits: bool,
) {
    if !problem.constraints.relations.iter().any(|relation| {
        matches!(relation, Relation::PadNear { max, .. } if max.is_none() || include_limits)
    }) {
        return;
    }
    let mut apart = Apart::new(problem);
    let count = problem.components.len();
    for relation in &problem.constraints.relations {
        let Relation::PadNear {
            source,
            anchor,
            max,
            ..
        } = relation
        else {
            continue;
        };
        if max.is_some() && !include_limits {
            continue;
        }
        let index = source.part();
        if crate::out_of_work(0) {
            break;
        }
        if !constraints::pad_near_refinable(problem, *source, *anchor)
            || unplaced.contains(&index)
            || unplaced.contains(&anchor.part())
            || !apart.is_legal(problem, poses, index, poses[index], 0..count)
        {
            continue;
        }
        let other_relations: Vec<_> = problem
            .constraints
            .relations
            .iter()
            .filter(|other| other.involves(index) && !std::ptr::eq(*other, relation))
            .map(|other| {
                (
                    other,
                    constraints::raw_relation_violation(problem, poses, other),
                )
            })
            .collect();
        let component = &problem.components[index];
        let relation_work = constraints::relation_work(problem, index);
        let original = poses[index];
        if max.is_none() {
            let distance = constraints::raw_relation_violation(problem, poses, relation);
            for (_, candidate) in face_contacts(problem, poses, relation, index, distance) {
                if crate::out_of_work(0) {
                    break;
                }
                if !apart.is_legal(problem, poses, index, candidate, 0..count) {
                    continue;
                }
                poses[index] = candidate;
                crate::add_work(relation_work);
                if other_relations.iter().all(|(other, prior)| {
                    constraints::raw_relation_violation(problem, poses, other) <= prior + 1.0e-9
                }) {
                    break;
                }
                poses[index] = original;
            }
        }
        let current = poses[index];
        let target = component.position_for_center(
            constraints::relation_target_at(problem, poses, relation, current.angle),
            current.angle,
        );

        // Following the gradient reaches the first nearby obstruction.
        // Trying each axis separately lets the part slide along a flat
        // courtyard or the edge of a keepout.
        for _ in 0..3 {
            let initial = poses[index];
            let [x, y] = initial.position;
            let current = constraints::raw_relation_violation(problem, poses, relation);
            if current <= 0.0 {
                break;
            }
            let mut best = (current, initial);
            for goal in [target, [target[0], y], [x, target[1]]] {
                let delta = [goal[0] - x, goal[1] - y];
                let length = delta[0].hypot(delta[1]);
                if length <= CONTACT_SLACK_MM {
                    continue;
                }
                let at = |fraction: f64| Pose {
                    position: [x + fraction * delta[0], y + fraction * delta[1]],
                    ..initial
                };
                let mut fits = |position| {
                    if crate::out_of_work(0)
                        || !apart.is_legal(problem, poses, index, position, 0..count)
                    {
                        return false;
                    }
                    poses[index] = position;
                    crate::add_work(relation_work);
                    let respects_relations = other_relations.iter().all(|(other, prior)| {
                        constraints::raw_relation_violation(problem, poses, other) <= prior + 1.0e-9
                    });
                    poses[index] = initial;
                    respects_relations
                };
                let candidate = if fits(at(1.0)) {
                    at(1.0)
                } else {
                    let mut low = 0.0;
                    let mut high = 1.0;
                    for _ in 0..SEARCH_STEPS {
                        if (high - low) * length <= CONTACT_SLACK_MM {
                            break;
                        }
                        let middle = (low + high) / 2.0;
                        if fits(at(middle)) {
                            low = middle;
                        } else {
                            high = middle;
                        }
                    }
                    at((low - CONTACT_SLACK_MM / length).max(0.0))
                };
                // Confirm the final, backed-off point as a ray can cross
                // more than one disjoint legal interval.
                if fits(candidate) {
                    poses[index] = candidate;
                    let violation = constraints::raw_relation_violation(problem, poses, relation);
                    poses[index] = initial;
                    if violation < best.0 - 1.0e-9 {
                        best = (violation, candidate);
                    }
                }
            }
            if best.1 == initial {
                break;
            }
            poses[index] = best.1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::Anchor;

    fn related_pair(gap: f64) -> (Problem, Vec<Pose>) {
        let (mut problem, mut poses) = super::super::tests::pair(0.0, gap);
        problem.components[0].fixed = true;
        poses[1].position[1] += 0.35;
        problem.constraints.relations.push(Relation::PadNear {
            source: Anchor::Point(1, [-0.7, 0.0]),
            anchor: Anchor::Point(0, [0.7, 0.0]),
            max: Some(0.0),
            pad_numbers: None,
        });
        problem.constraints.link_pairs();
        (problem, poses)
    }

    #[test]
    fn refinement_aligns_the_pads_at_a_legal_courtyard_gap() {
        let (problem, mut poses) = related_pair(0.63);
        let held = poses[0];
        refine_pad_near(&problem, &mut poses, &[], true);
        assert_eq!(poses[0], held);
        assert_eq!(poses[1].angle, 0.0);
        assert!(super::super::is_legal(&problem, &poses, 1, poses[1], 0..2));
        assert!((12.2..12.203).contains(&poses[1].position[0]), "{poses:?}");
        assert!((poses[1].position[1] - 10.0).abs() < 0.002, "{poses:?}");
    }

    #[test]
    fn refinement_preserves_other_relations_and_skips_unplaced_parts() {
        let (mut problem, mut poses) = related_pair(0.63);
        let apart = Relation::Apart {
            part: 1,
            anchor: 0,
            min: 0.51,
        };
        problem.constraints.relations.push(apart.clone());
        let before = poses.clone();
        refine_pad_near(&problem, &mut poses, &[1], true);
        assert_eq!(poses, before);
        refine_pad_near(&problem, &mut poses, &[], true);
        assert_eq!(
            constraints::raw_relation_violation(&problem, &poses, &apart),
            0.0
        );
        assert!((12.51..12.513).contains(&poses[1].position[0]), "{poses:?}");
    }

    #[test]
    fn refinement_reaches_the_requested_limit_inside_reporting_tolerance() {
        let (mut problem, mut poses) = related_pair(0.63);
        let Relation::PadNear { max, .. } = &mut problem.constraints.relations[0] else {
            unreachable!()
        };
        *max = Some(1.25);
        let relation = &problem.constraints.relations[0];
        assert_eq!(
            constraints::relation_violation(&problem, &poses, relation),
            0.0
        );
        assert!(constraints::raw_relation_violation(&problem, &poses, relation) > 0.0);
        refine_pad_near(&problem, &mut poses, &[], true);
        assert_eq!(
            constraints::raw_relation_violation(&problem, &poses, relation),
            0.0
        );
    }

    #[test]
    fn a_pad_distance_objective_rotates_an_eligible_source_to_face_the_target() {
        let (mut problem, mut poses) = related_pair(0.63);
        problem.components[1].angle_options = vec![0.0, 180.0];
        poses[1].angle = 180.0;
        let before = poses.clone();
        refine_pad_near(&problem, &mut poses, &[], false);
        assert_eq!(poses, before, "a numeric limit stays opt-in");

        let Relation::PadNear { max, .. } = &mut problem.constraints.relations[0] else {
            unreachable!()
        };
        *max = None;
        refine_pad_near(&problem, &mut poses, &[], false);
        assert_eq!(poses[0], before[0]);
        assert_eq!(poses[1].angle, 0.0);
        assert!(super::super::is_legal(&problem, &poses, 1, poses[1], 0..2));
        let status = constraints::report(&problem, &poses).pop().unwrap();
        assert_eq!(status.kind, "pad_near_objective");
        assert!(status.satisfied);
        assert!((0.8..0.803).contains(&status.distance.unwrap()));
    }

    #[test]
    fn refinement_eligibility_excludes_held_sources_and_rigidly_linked_parts() {
        use crate::constraints::{Edge, Follower, pad_near_refinable};

        let (mut problem, _) = related_pair(0.63);
        let source = Anchor::Point(1, [-0.7, 0.0]);
        let anchor = Anchor::Point(0, [0.7, 0.0]);
        assert!(pad_near_refinable(&problem, source, anchor));
        assert!(!pad_near_refinable(&problem, anchor, source));
        assert!(!pad_near_refinable(&problem, source, source));
        problem.constraints.edges.push((1, Edge::Left, 0.0));
        assert!(!pad_near_refinable(&problem, source, anchor));
        problem.constraints.edges.clear();
        for (part, leader) in [(0, 1), (1, 0)] {
            problem.constraints.followers = vec![Follower {
                part,
                leader,
                offset: [3.0, 0.0],
                angle: 0.0,
            }];
            assert!(!pad_near_refinable(&problem, source, anchor));
        }
    }
}

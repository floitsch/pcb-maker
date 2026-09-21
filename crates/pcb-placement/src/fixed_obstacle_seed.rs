// Copyright (C) 2026 Toit contributors.

//! Nearest sampled seed legal against an occupied union of footprints. This is
//! deliberately not a continuous nearest-point or infeasibility solver.
use super::*;
use std::collections::BinaryHeap;

const PITCH_MM: f64 = 0.25;

#[derive(Clone, Copy, Debug)]
struct Candidate {
    distance: f64,
    x: usize,
    y: usize,
    position: Vec2,
}
impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max heap; reverse the complete deterministic order.
        other
            .distance
            .total_cmp(&self.distance)
            .then_with(|| other.position.x.total_cmp(&self.position.x))
            .then_with(|| other.position.y.total_cmp(&self.position.y))
    }
}

fn axis_samples(
    low: f64,
    high: f64,
    origin: f64,
    seed: f64,
    locked: bool,
    budget: usize,
) -> Result<Vec<f64>, String> {
    if low > high || !low.is_finite() || !high.is_finite() {
        return Err("fixed-obstacle seed projection has an empty center domain".into());
    }
    if locked {
        return if seed >= low - FEASIBILITY_TOLERANCE_MM && seed <= high + FEASIBILITY_TOLERANCE_MM
        {
            Ok(vec![seed])
        } else {
            Err("fixed-obstacle seed projection axis lock lies outside its center domain".into())
        };
    }
    let first = ((low - origin) / PITCH_MM).ceil();
    let last = ((high - origin) / PITCH_MM).floor();
    // Bound candidate-domain setup as well as geometry queries. This rejects
    // oversized sampling domains explicitly, without a layout impossibility claim.
    if last - first > budget as f64 {
        return Err("fixed-obstacle seed projection sampled domain exceeds its work budget".into());
    }
    let mut values = vec![low, high];
    for index in (first as i64)..=(last as i64) {
        let value = origin + index as f64 * PITCH_MM;
        if value >= low && value <= high {
            values.push(value);
        }
    }
    values.sort_by(|a, b| a.total_cmp(b));
    values.dedup_by(|a, b| *a == *b);
    values.sort_by(|a, b| {
        (a - seed)
            .powi(2)
            .total_cmp(&(b - seed).powi(2))
            .then_with(|| a.total_cmp(b))
    });
    Ok(values)
}

/// The original seed is checked first. Each later lattice/boundary sample is
/// visited once in distance order by a lazy row frontier, without sorting the
/// Cartesian product. Optional largest-first insertion adds accepted free
/// footprints to that union. Each actual footprint-pair query spends the caller's
/// shared budget. Changes commit together only after every free seed succeeds.
#[cfg(test)]
fn project(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    poses: &mut [PlacementPose],
    pair_checks: &mut usize,
    maximum_pair_checks: usize,
) -> Result<(), String> {
    project_with_insertion(
        problem,
        components,
        poses,
        pair_checks,
        maximum_pair_checks,
        false,
    )
}

#[cfg(test)]
fn project_with_insertion(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    poses: &mut [PlacementPose],
    pair_checks: &mut usize,
    maximum_pair_checks: usize,
    largest_first: bool,
) -> Result<(), String> {
    project_with_work(
        problem,
        components,
        poses,
        pair_checks,
        maximum_pair_checks,
        largest_first,
        &mut PlacementSeedBroadPhaseWork::default(),
    )
}

pub(super) fn project_with_work(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    poses: &mut [PlacementPose],
    pair_checks: &mut usize,
    maximum_pair_checks: usize,
    largest_first: bool,
    broad_phase_work: &mut PlacementSeedBroadPhaseWork,
) -> Result<(), String> {
    project_with_intervals(
        problem,
        components,
        poses,
        pair_checks,
        maximum_pair_checks,
        largest_first,
        false,
        broad_phase_work,
    )
}

pub(super) fn project_with_intervals(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    poses: &mut [PlacementPose],
    pair_checks: &mut usize,
    maximum_pair_checks: usize,
    largest_first: bool,
    skip_collision_intervals: bool,
    broad_phase_work: &mut PlacementSeedBroadPhaseWork,
) -> Result<(), String> {
    project_with_pad_gap(
        problem,
        components,
        poses,
        pair_checks,
        maximum_pair_checks,
        largest_first,
        skip_collision_intervals,
        0.0,
        broad_phase_work,
    )
}

pub(super) fn project_with_pad_gap(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    poses: &mut [PlacementPose],
    pair_checks: &mut usize,
    maximum_pair_checks: usize,
    largest_first: bool,
    skip_collision_intervals: bool,
    extra_pad_gap: f64,
    broad_phase_work: &mut PlacementSeedBroadPhaseWork,
) -> Result<(), String> {
    if !extra_pad_gap.is_finite()
        || extra_pad_gap < 0.0
        || !(problem.rules.clearance + extra_pad_gap).is_finite()
    {
        return Err(
            "seed copper clearance plus extra pad gap must be finite and nonnegative".into(),
        );
    }
    let (_, anchors) = harmonic_anchor_poses(problem, components)?;
    let mut occupied = (0..components.len())
        .filter(|&i| anchors[i])
        .collect::<Vec<_>>();
    if occupied.is_empty() && !largest_first {
        return Ok(());
    }
    let original = poses.to_vec();
    let prepared = components
        .iter()
        .zip(&original)
        .map(|(component, pose)| {
            geometry::prepare_projection_geometry_with_pad_gap(
                component,
                pose.rotation_degrees,
                problem.rules.clearance,
                extra_pad_gap,
            )
        })
        .collect::<Vec<_>>();
    let mut projected = original.clone();
    let mut order = (0..components.len())
        .filter(|&i| !anchors[i])
        .collect::<Vec<_>>();
    if largest_first {
        let area = |index: usize| {
            let (_, size) = placement_envelope(components[index]);
            let extent = rotated_extent(size, original[index].rotation_degrees);
            extent.x * extent.y * 4.0
        };
        order.sort_by(|&a, &b| {
            area(b)
                .total_cmp(&area(a))
                .then_with(|| components[a].id.cmp(&components[b].id))
        });
    }
    for index in order {
        let component = components[index];
        let seed = original[index].position;
        let mut obstacle_order = occupied.clone();
        obstacle_order.sort_by(|&a, &b| {
            vector_length(sub(projected[a].position, seed))
                .total_cmp(&vector_length(sub(projected[b].position, seed)))
                .then_with(|| components[a].id.cmp(&components[b].id))
        });
        let allowed = component
            .constraints
            .region
            .map_or(problem.board.bounds, |region| Rect {
                min: Vec2::new(
                    problem.board.bounds.min.x.max(region.min.x),
                    problem.board.bounds.min.y.max(region.min.y),
                ),
                max: Vec2::new(
                    problem.board.bounds.max.x.min(region.max.x),
                    problem.board.bounds.max.y.min(region.max.y),
                ),
            });
        let (low, high) = geometry::position_limits(problem, component, &original[index], allowed);
        let pair_bounds = obstacle_order
            .iter()
            .map(|&obstacle| {
                geometry::prepare_pair_projection_bounds(&prepared[index], &prepared[obstacle])
            })
            .collect::<Vec<_>>();
        let legal = |position: Vec2,
                     pair_checks: &mut usize,
                     broad_phase_work: &mut PlacementSeedBroadPhaseWork|
         -> Result<bool, String> {
            let mut pose = original[index].clone();
            pose.position = position;
            for (&obstacle, bounds) in obstacle_order.iter().zip(&pair_bounds) {
                broad_phase_work.comparisons += 1;
                if bounds.certainly_disjoint(position, projected[obstacle].position) {
                    broad_phase_work.rejected_disjoint += 1;
                    continue;
                }
                if *pair_checks >= maximum_pair_checks {
                    return Err(format!(
                        "fixed-obstacle seed projection exhausted its {maximum_pair_checks} shared pair-check budget at {}",
                        component.id
                    ));
                }
                *pair_checks += 1;
                let overlap = if extra_pad_gap == 0.0
                    || (component.placement_geometry.is_none()
                        && components[obstacle].placement_geometry.is_none())
                {
                    // Preserve the original legacy/explicit geometry dispatch.
                    body_overlap_correction(
                        component,
                        &pose,
                        components[obstacle],
                        &projected[obstacle],
                        problem.rules.clearance,
                    )
                } else {
                    geometry::overlap_correction_with_pad_gap(
                        component,
                        &pose,
                        components[obstacle],
                        &projected[obstacle],
                        problem.rules.clearance,
                        extra_pad_gap,
                    )
                };
                if overlap.is_some() {
                    return Ok(false);
                }
            }
            Ok(true)
        };
        if seed.x >= low.x
            && seed.x <= high.x
            && seed.y >= low.y
            && seed.y <= high.y
            && legal(seed, pair_checks, broad_phase_work)?
        {
            if largest_first {
                occupied.push(index);
            }
            continue;
        }
        let xs = axis_samples(
            low.x,
            high.x,
            problem.board.bounds.min.x,
            seed.x,
            component.constraints.movement == Movement::Vertical,
            maximum_pair_checks,
        )?;
        let ys = axis_samples(
            low.y,
            high.y,
            problem.board.bounds.min.y,
            seed.y,
            component.constraints.movement == Movement::Horizontal,
            maximum_pair_checks,
        )?;
        let candidate = |x: usize, y: usize| {
            let position = Vec2::new(xs[x], ys[y]);
            Candidate {
                distance: (position.x - seed.x).powi(2) + (position.y - seed.y).powi(2),
                x,
                y,
                position,
            }
        };
        let mut collision_pairs = Vec::new();
        if skip_collision_intervals {
            for &obstacle in &obstacle_order {
                spend_certificate(&component.id, pair_checks, maximum_pair_checks)?;
                broad_phase_work.interval_preparations += 1;
                collision_pairs.push(geometry::prepare_collision_intervals(
                    &prepared[index],
                    &prepared[obstacle],
                    projected[obstacle].position,
                ));
            }
        }
        // A not-yet-initialized row enters at its smallest raw candidate key.
        // This lower bound must be expanded even when every sample is skipped.
        let mut row_samples: Vec<Option<Vec<usize>>> = vec![None; xs.len()];
        let mut row_heads = vec![0usize; xs.len()];
        let minimum_y = ys.iter().copied().min_by(f64::total_cmp).unwrap();
        let mut row_order = (0..xs.len())
            .map(|x| {
                let mut bound = candidate(x, 0);
                // A virtual y smaller than every real y makes this a true complete-
                // key lower bound even when adding dx² rounds distinct dy² to a tie.
                bound.position.y = minimum_y;
                bound
            })
            .collect::<Vec<_>>();
        // Adding the nearest dy² can round distinct dx² to the same total.
        // Use the actual complete key, not the original dx²-only x order.
        row_order.sort_by(|a, b| b.cmp(a));
        let mut next_row = 1;
        let mut frontier = BinaryHeap::new();
        frontier.push(if skip_collision_intervals {
            row_order[0]
        } else {
            candidate(0, 0)
        });
        let mut found = None;
        while let Some(sample) = frontier.pop() {
            if skip_collision_intervals && row_samples[sample.x].is_none() {
                let mut intervals = Vec::new();
                for pair in &collision_pairs {
                    if pair.is_empty() {
                        continue;
                    }
                    spend_certificate(&component.id, pair_checks, maximum_pair_checks)?;
                    broad_phase_work.row_certificates += 1;
                    intervals.extend(pair.row_intervals(xs[sample.x], low.y, high.y));
                }
                let mut allowed = uncovered_samples(&ys, intervals);
                // The same rounded-total tie can occur between y samples in
                // one row; preserve the full key after adding this row's dx².
                allowed.sort_by(|&a, &b| candidate(sample.x, b).cmp(&candidate(sample.x, a)));
                broad_phase_work.skipped_samples += ys.len() - allowed.len();
                if let Some(&y) = allowed.first() {
                    frontier.push(candidate(sample.x, y));
                }
                row_samples[sample.x] = Some(allowed);
                if next_row < row_order.len() {
                    frontier.push(row_order[next_row]);
                    next_row += 1;
                }
                continue;
            }
            // An in-domain unchanged seed has already been queried.
            if sample.position != seed && legal(sample.position, pair_checks, broad_phase_work)? {
                found = Some(sample.position);
                break;
            }
            if skip_collision_intervals {
                row_heads[sample.x] += 1;
                if let Some(&y) = row_samples[sample.x]
                    .as_ref()
                    .unwrap()
                    .get(row_heads[sample.x])
                {
                    frontier.push(candidate(sample.x, y));
                }
            } else {
                if sample.y + 1 < ys.len() {
                    frontier.push(candidate(sample.x, sample.y + 1));
                }
                if sample.y == 0 && sample.x + 1 < xs.len() {
                    frontier.push(candidate(sample.x + 1, 0));
                }
            }
        }
        projected[index].position = found.ok_or_else(|| format!(
            "fixed-obstacle seed projection found no legal sampled center for {}; this does not establish continuous infeasibility", component.id))?;
        if largest_first {
            occupied.push(index);
        }
    }
    poses.clone_from_slice(&projected);
    Ok(())
}

fn spend_certificate(component: &str, checks: &mut usize, maximum: usize) -> Result<(), String> {
    if *checks >= maximum {
        return Err(format!(
            "fixed-obstacle seed projection exhausted its {maximum} shared pair-check budget at {component}"
        ));
    }
    *checks += 1;
    Ok(())
}

/// Interval coordinates are sorted geometrically; sample indices retain the
/// original distance/tie order. Touching open intervals do not cover their seam.
fn uncovered_samples(ys: &[f64], mut intervals: Vec<(f64, f64)>) -> Vec<usize> {
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    let mut union: Vec<(f64, f64)> = Vec::new();
    for interval in intervals {
        if let Some(last) = union.last_mut()
            && interval.0 < last.1
        {
            last.1 = last.1.max(interval.1);
        } else {
            union.push(interval);
        }
    }
    ys.iter()
        .enumerate()
        .filter_map(|(i, &y)| {
            let after = union.partition_point(|interval| interval.0 < y);
            (after == 0 || y >= union[after - 1].1).then_some(i)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Problem {
        serde_json::from_value(serde_json::json!({
            "schema_version":1,
            "board":{"bounds":{"min":{"x":0,"y":0},"max":{"x":30,"y":20}}},
            "rules":{"clearance":0.2},
            "components":[
                {"id":"A","position":{"x":10,"y":10},"size":{"x":4,"y":4},
                 "constraints":{"movement":"fixed","rotation":"fixed"},"pins":[]},
                {"id":"B","position":{"x":15,"y":10},"size":{"x":4,"y":4},
                 "constraints":{"movement":"fixed","rotation":"fixed"},"pins":[]},
                {"id":"M","position":{"x":12.5,"y":10},"size":{"x":2,"y":2},
                 "constraints":{"movement":"free","rotation":"fixed"},"pins":[]}
            ],"nets":[]
        }))
        .unwrap()
    }

    #[test]
    fn virtual_row_key_bounds_rounded_distance_ties() {
        let seed = Vec2::ZERO;
        let x = 100_000_000.0_f64;
        let ys = [0.0, -0.25, 0.25, -0.5, 0.5];
        let raw = Candidate {
            distance: x.powi(2),
            x: 0,
            y: 0,
            position: Vec2::new(x, 0.0),
        };
        let bound = Candidate {
            position: Vec2::new(x, -0.5),
            ..raw
        };
        let mut actual = ys
            .iter()
            .enumerate()
            .map(|(y, &py)| Candidate {
                distance: (x - seed.x).powi(2) + (py - seed.y).powi(2),
                x: 0,
                y,
                position: Vec2::new(x, py),
            })
            .collect::<Vec<_>>();
        actual.sort_by(|a, b| b.cmp(a));
        assert_eq!(actual[0].position.y, -0.5);
        assert!(
            raw < actual[0],
            "the old raw head is not a complete-key lower bound"
        );
        assert!(actual.iter().all(|candidate| bound >= *candidate));
    }

    #[test]
    fn open_interval_filter_keeps_distance_order_and_touching_seams() {
        let ys = vec![0.0, -0.25, 0.25, -0.5, 0.5, -1.0, 1.0, -1.125, 1.125];
        let kept = uncovered_samples(&ys, vec![(-1.0, 0.0), (0.0, 1.0), (-0.75, -0.5)]);
        assert_eq!(kept, vec![0, 5, 6, 7, 8]);
    }

    #[test]
    fn interval_rows_match_exhaustive_sample_order_with_axis_locks_and_boundaries() {
        for movement in [Movement::Free, Movement::Horizontal, Movement::Vertical] {
            for seed in [
                Vec2::new(12.5, 10.0),
                Vec2::new(11.13, 9.87),
                Vec2::new(15.0, 10.0),
            ] {
                let mut problem = fixture();
                problem.components[2].position = seed;
                problem.components[2].constraints.movement = movement;
                let components = sorted_components(&problem);
                let original = declared_poses(&problem);
                let mut reference = original.clone();
                let mut actual = original.clone();
                let mut old_checks = 0;
                let mut new_checks = 0;
                project_with_work(
                    &problem,
                    &components,
                    &mut reference,
                    &mut old_checks,
                    100_000,
                    true,
                    &mut PlacementSeedBroadPhaseWork::default(),
                )
                .unwrap();
                let mut work = PlacementSeedBroadPhaseWork::default();
                project_with_intervals(
                    &problem,
                    &components,
                    &mut actual,
                    &mut new_checks,
                    100_000,
                    true,
                    true,
                    &mut work,
                )
                .unwrap();
                assert_eq!(actual, reference, "movement {movement:?}, seed {seed:?}");
                // Independent oracle: sort the complete Cartesian sample set
                // by the public nearest-candidate key, not by the old lazy heap.
                let (lo, hi) = geometry::position_limits(
                    &problem,
                    components[2],
                    &original[2],
                    problem.board.bounds,
                );
                let xs = axis_samples(
                    lo.x,
                    hi.x,
                    problem.board.bounds.min.x,
                    seed.x,
                    movement == Movement::Vertical,
                    100_000,
                )
                .unwrap();
                let ys = axis_samples(
                    lo.y,
                    hi.y,
                    problem.board.bounds.min.y,
                    seed.y,
                    movement == Movement::Horizontal,
                    100_000,
                )
                .unwrap();
                let mut all = Vec::new();
                for (x, &px) in xs.iter().enumerate() {
                    for (y, &py) in ys.iter().enumerate() {
                        all.push(Candidate {
                            distance: (px - seed.x).powi(2) + (py - seed.y).powi(2),
                            x,
                            y,
                            position: Vec2::new(px, py),
                        });
                    }
                }
                all.sort_by(|a, b| b.cmp(a));
                let expected = all
                    .into_iter()
                    .find(|sample| {
                        let mut pose = original[2].clone();
                        pose.position = sample.position;
                        (0..2).all(|i| {
                            body_overlap_correction(
                                components[2],
                                &pose,
                                components[i],
                                &original[i],
                                problem.rules.clearance,
                            )
                            .is_none()
                        })
                    })
                    .unwrap();
                assert_eq!(
                    actual[2].position, expected.position,
                    "complete-sort oracle"
                );

                assert!(work.skipped_samples > 0);
                assert!(work.row_certificates > 0);
                let mut failed = original.clone();
                let mut checks = 0;
                assert!(
                    project_with_intervals(
                        &problem,
                        &components,
                        &mut failed,
                        &mut checks,
                        2,
                        true,
                        true,
                        &mut PlacementSeedBroadPhaseWork::default()
                    )
                    .is_err()
                );
                assert!(checks <= 2);
                assert_eq!(failed, original);
            }
        }
    }

    #[test]
    fn interval_search_escapes_large_occupied_interior_without_point_enumeration() {
        let mut problem = fixture();
        problem.board.bounds.max = Vec2::new(100.0, 80.0);
        problem.components.remove(1);
        problem.components[0].position = Vec2::new(50.0, 40.0);
        problem.components[0].size = Vec2::new(70.0, 50.0);
        problem.components[1].position = Vec2::new(50.0, 40.0);
        let components = sorted_components(&problem);
        let mut old = declared_poses(&problem);
        let mut new = old.clone();
        let mut old_checks = 0;
        let mut new_checks = 0;
        let mut work = PlacementSeedBroadPhaseWork::default();
        project_with_work(
            &problem,
            &components,
            &mut old,
            &mut old_checks,
            100_000,
            true,
            &mut PlacementSeedBroadPhaseWork::default(),
        )
        .unwrap();
        project_with_intervals(
            &problem,
            &components,
            &mut new,
            &mut new_checks,
            100_000,
            true,
            true,
            &mut work,
        )
        .unwrap();
        assert_eq!(new, old);
        assert!(
            old_checks > new_checks * 10,
            "old {old_checks}, new {new_checks}"
        );
        assert!(work.skipped_samples > 10_000);
        validate_placement_exclusions(&problem, &new).unwrap();
    }

    #[test]
    fn disjoint_certificates_are_counted_separately_from_exact_pair_budget() {
        let mut problem = fixture();
        problem.components[2].position = Vec2::new(26.0, 16.0);
        let original = declared_poses(&problem);
        let mut poses = original.clone();
        let mut checks = 0;
        let mut broad = PlacementSeedBroadPhaseWork::default();
        project_with_work(
            &problem,
            &sorted_components(&problem),
            &mut poses,
            &mut checks,
            1,
            false,
            &mut broad,
        )
        .unwrap();
        assert_eq!(poses, original);
        assert_eq!(checks, 0);
        assert_eq!(broad.comparisons, 2);
        assert_eq!(broad.rejected_disjoint, 2);
    }

    #[test]
    fn union_projection_escapes_narrow_gap_and_is_order_independent() {
        let mut problem = fixture();
        let components = sorted_components(&problem);
        let original = declared_poses(&problem);
        let mut poses = original.clone();
        let mut checks = 0;
        project(&problem, &components, &mut poses, &mut checks, 10_000).unwrap();
        assert_eq!(poses[0], original[0]);
        assert_eq!(poses[1], original[1]);
        assert_eq!(poses[2].position, Vec2::new(12.5, 6.75));
        validate_placement_exclusions(&problem, &poses).unwrap();
        assert!(checks > 2 && checks < 10_000);
        let expected_checks = checks;
        problem.components.reverse();
        let mut reversed = declared_poses(&problem);
        let mut checks = 0;
        project(
            &problem,
            &sorted_components(&problem),
            &mut reversed,
            &mut checks,
            10_000,
        )
        .unwrap();
        assert_eq!(poses, reversed);
        assert_eq!(checks, expected_checks);
        let mut exhausted = poses.clone();
        let error = legalize_harmonic_poses(
            &problem,
            &sorted_components(&problem),
            &mut exhausted,
            1,
            expected_checks,
            expected_checks,
            None,
            false,
            0,
            None,
        )
        .unwrap_err();
        assert!(error.contains("pair-check budget"));
    }

    #[test]
    fn projection_obeys_axis_and_region_constraints_and_rolls_back_at_budget() {
        let mut problem = fixture();
        problem.components[2].constraints.movement = Movement::Horizontal;
        let original = declared_poses(&problem);
        let mut poses = original.clone();
        let mut checks = 0;
        project(
            &problem,
            &sorted_components(&problem),
            &mut poses,
            &mut checks,
            10_000,
        )
        .unwrap();
        assert_eq!(poses[2].position, Vec2::new(6.75, 10.0));
        problem.components[2].constraints.region = Some(Rect {
            min: Vec2::new(11.0, 8.0),
            max: Vec2::new(14.0, 12.0),
        });
        let mut poses = original.clone();
        let error = project(
            &problem,
            &sorted_components(&problem),
            &mut poses,
            &mut 0,
            10_000,
        )
        .unwrap_err();
        assert!(error.contains("no legal sampled center"));
        assert_eq!(poses, original);
        let mut checks = 0;
        let error = project(
            &problem,
            &sorted_components(&problem),
            &mut poses,
            &mut checks,
            1,
        )
        .unwrap_err();
        assert!(
            error.contains("work budget") || error.contains("shared pair-check budget"),
            "{error}"
        );
        assert_eq!(checks, 1);
        assert_eq!(poses, original);
    }

    #[test]
    fn sequential_insertion_resolves_competing_free_bodies_in_one_stable_order() {
        let mut problem = fixture();
        let mut small = problem.components[2].clone();
        small.id = "N".into();
        small.size = Vec2::new(1.0, 1.0);
        problem.components.push(small);
        let original = declared_poses(&problem);
        let mut independent = original.clone();
        project(
            &problem,
            &sorted_components(&problem),
            &mut independent,
            &mut 0,
            10_000,
        )
        .unwrap();
        assert!(validate_placement_exclusions(&problem, &independent).is_err());
        let mut inserted = original.clone();
        let mut checks = 0;
        project_with_insertion(
            &problem,
            &sorted_components(&problem),
            &mut inserted,
            &mut checks,
            10_000,
            true,
        )
        .unwrap();
        validate_placement_exclusions(&problem, &inserted).unwrap();
        assert_eq!(inserted[0], original[0]);
        assert_eq!(inserted[1], original[1]);
        assert_eq!(inserted[2], independent[2]); // The largest free body gets first choice.
        let expected_checks = checks;
        problem.components.reverse();
        let mut reversed = declared_poses(&problem);
        let mut checks = 0;
        project_with_insertion(
            &problem,
            &sorted_components(&problem),
            &mut reversed,
            &mut checks,
            10_000,
            true,
        )
        .unwrap();
        assert_eq!(inserted, reversed);
        assert_eq!(checks, expected_checks);
        // With no anchors, insertion must still unpack mutually overlapping free seeds.
        problem
            .components
            .retain(|c| c.constraints.movement != Movement::Fixed);
        let mut all_free = declared_poses(&problem);
        project_with_insertion(
            &problem,
            &sorted_components(&problem),
            &mut all_free,
            &mut 0,
            10_000,
            true,
        )
        .unwrap();
        validate_placement_exclusions(&problem, &all_free).unwrap();
    }

    #[test]
    fn failed_later_insertion_rolls_back_earlier_choices_without_new_budget() {
        let mut problem = fixture();
        let mut first_only = declared_poses(&problem);
        let mut first_checks = 0;
        project_with_insertion(
            &problem,
            &sorted_components(&problem),
            &mut first_only,
            &mut first_checks,
            10_000,
            true,
        )
        .unwrap();
        let mut small = problem.components[2].clone();
        small.id = "N".into();
        small.size = Vec2::new(1.0, 1.0);
        problem.components.push(small);
        let original = declared_poses(&problem);
        let mut poses = original.clone();
        let mut checks = 0;
        let error = project_with_insertion(
            &problem,
            &sorted_components(&problem),
            &mut poses,
            &mut checks,
            first_checks,
            true,
        )
        .unwrap_err();
        assert!(error.contains("shared pair-check budget"));
        assert_eq!(checks, first_checks);
        assert_eq!(poses, original);
        problem.components[3].constraints.region = Some(Rect {
            min: Vec2::new(12.0, 9.5),
            max: Vec2::new(13.0, 10.5),
        });
        let mut checks = 0;
        let error = project_with_insertion(
            &problem,
            &sorted_components(&problem),
            &mut poses,
            &mut checks,
            10_000,
            true,
        )
        .unwrap_err();
        assert!(error.contains("no legal sampled center"));
        assert!(checks > first_checks);
        assert_eq!(poses, original);
    }

    #[test]
    fn inserted_opposite_side_bodies_can_share_space_but_shared_layer_pads_cannot() {
        let mut problem: Problem = serde_json::from_value(serde_json::json!({
            "schema_version":1,
            "board":{"bounds":{"min":{"x":0,"y":0},"max":{"x":20,"y":20}},"layers":[{"id":"top"},{"id":"bottom"}]},
            "rules":{"clearance":0.2},
            "components":[
                {"id":"A","position":{"x":10,"y":10},"size":{"x":2,"y":2},
                 "constraints":{"movement":"free","rotation":"fixed"},
                 "placement_geometry":{"body":{"kind":"rect","size":{"x":2,"y":2}},"body_layers":["top"],"clearance":0},
                 "pins":[{"id":"1","offset":{"x":5,"y":5},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.2}}]}]},
                {"id":"M","position":{"x":10,"y":10},"size":{"x":2,"y":2},
                 "constraints":{"movement":"free","rotation":"fixed"},
                 "placement_geometry":{"body":{"kind":"rect","size":{"x":2,"y":2}},"body_layers":["bottom"],"clearance":0},
                 "pins":[{"id":"1","offset":{"x":5,"y":5},"pads":[{"layer":"bottom","shape":{"kind":"circle","diameter":0.2}}]}]}
            ],"nets":[]
        })).unwrap();
        let original = declared_poses(&problem);
        let mut poses = original.clone();
        project_with_insertion(
            &problem,
            &sorted_components(&problem),
            &mut poses,
            &mut 0,
            10_000,
            true,
        )
        .unwrap();
        assert_eq!(poses, original);
        problem.components[1].pins[0].pads[0].layer = "top".into();
        project_with_insertion(
            &problem,
            &sorted_components(&problem),
            &mut poses,
            &mut 0,
            10_000,
            true,
        )
        .unwrap();
        assert_eq!(poses[0], original[0]);
        assert_ne!(poses[1], original[1]);
        validate_placement_exclusions(&problem, &poses).unwrap();
    }

    #[test]
    fn native_round_body_and_offset_pad_use_actual_geometry() {
        let mut problem: Problem = serde_json::from_value(serde_json::json!({
            "schema_version":1,
            "board":{"bounds":{"min":{"x":0,"y":0},"max":{"x":20,"y":20}},"layers":[{"id":"top"}]},
            "rules":{"clearance":0.2},
            "components":[
                {"id":"A","position":{"x":10,"y":10},"size":{"x":4,"y":4},
                 "constraints":{"movement":"fixed","rotation":"fixed"},
                 "placement_geometry":{"body":{"kind":"circle","diameter":4},"clearance":0},"pins":[]},
                {"id":"M","position":{"x":12,"y":12},"size":{"x":1,"y":1},
                 "constraints":{"movement":"free","rotation":"fixed"},
                 "placement_geometry":{"body":{"kind":"rect","size":{"x":1,"y":1}},"clearance":0},"pins":[]}
            ],"nets":[]
        })).unwrap();
        let original = declared_poses(&problem);
        let mut poses = original.clone();
        project(
            &problem,
            &sorted_components(&problem),
            &mut poses,
            &mut 0,
            10_000,
        )
        .unwrap();
        assert_eq!(poses, original); // Legal by the circular corner, inside its AABB.
        problem.components[0].pins = serde_json::from_value(serde_json::json!([
            {"id":"1","offset":{"x":2,"y":2},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":1}}]}
        ])).unwrap();
        project(
            &problem,
            &sorted_components(&problem),
            &mut poses,
            &mut 0,
            10_000,
        )
        .unwrap();
        assert_eq!(poses[0], original[0]);
        assert_ne!(poses[1], original[1]); // The outlying native pad must stay clear.
        validate_placement_exclusions(&problem, &poses).unwrap();
    }

    #[test]
    fn exact_boundaries_are_samples_and_all_free_input_spends_no_checks() {
        let axis = axis_samples(1.1, 1.2, 0.0, 1.16, false, 10).unwrap();
        assert_eq!(axis, vec![1.2, 1.1]);
        let mut problem = fixture();
        for component in &mut problem.components {
            component.constraints.movement = Movement::Free;
        }
        let original = declared_poses(&problem);
        let mut poses = original.clone();
        let mut checks = 0;
        project(
            &problem,
            &sorted_components(&problem),
            &mut poses,
            &mut checks,
            1,
        )
        .unwrap();
        assert_eq!(poses, original);
        assert_eq!(checks, 0);
    }

    #[test]
    fn seed_pad_reservation_preserves_zero_mode_and_close_anchors_with_atomic_budget() {
        let legacy = fixture();
        let mut legacy_results = Vec::new();
        for extra in [0.0, 10.0] {
            let mut poses = declared_poses(&legacy);
            let mut checks = 0;
            let mut work = PlacementSeedBroadPhaseWork::default();
            project_with_pad_gap(
                &legacy,
                &sorted_components(&legacy),
                &mut poses,
                &mut checks,
                100_000,
                true,
                true,
                extra,
                &mut work,
            )
            .unwrap();
            legacy_results.push((poses, checks, serde_json::to_value(work).unwrap()));
        }
        assert_eq!(legacy_results[0], legacy_results[1]);
        let mut problem = fixture();
        for component in &mut problem.components {
            component.size = Vec2::new(0.2, 0.2);
            component.placement_geometry = Some(
                serde_json::from_value(serde_json::json!({
                    "body":{"kind":"rect","size":{"x":0.2,"y":0.2}},
                    "clearance":0,"body_layers":["top"]
                }))
                .unwrap(),
            );
            component.pins = serde_json::from_value(serde_json::json!([
                {"id":"1","offset":{"x":0,"y":0},"pads":[
                    {"layer":"top","shape":{"kind":"rect","size":{"x":1,"y":1}}}
                ]}
            ]))
            .unwrap();
        }
        problem.components[0].position = Vec2::new(5.0, 5.0);
        problem.components[1].position = Vec2::new(6.5, 5.0);
        problem.components[2].position = Vec2::new(5.75, 5.25);
        let before = serde_json::to_value(&problem).unwrap();
        let original = declared_poses(&problem);
        let components = sorted_components(&problem);
        let mut old = original.clone();
        let mut zero = original.clone();
        let (mut old_checks, mut zero_checks) = (0, 0);
        let (mut old_work, mut zero_work) = (
            PlacementSeedBroadPhaseWork::default(),
            PlacementSeedBroadPhaseWork::default(),
        );
        project_with_intervals(
            &problem,
            &components,
            &mut old,
            &mut old_checks,
            100_000,
            true,
            true,
            &mut old_work,
        )
        .unwrap();
        project_with_pad_gap(
            &problem,
            &components,
            &mut zero,
            &mut zero_checks,
            100_000,
            true,
            true,
            0.0,
            &mut zero_work,
        )
        .unwrap();
        assert_eq!(zero, old);
        assert_eq!(zero_checks, old_checks);
        assert_eq!(
            serde_json::to_value(zero_work).unwrap(),
            serde_json::to_value(old_work).unwrap()
        );
        // Their true gap is legal, but fixed anchors need not satisfy an
        // artificial free-placement reservation.
        assert!(
            geometry::overlap_correction_with_pad_gap(
                components[0],
                &original[0],
                components[1],
                &original[1],
                0.2,
                0.6
            )
            .is_some()
        );
        let mut reserved = Vec::new();
        for intervals in [false, true] {
            let mut poses = original.clone();
            let mut checks = 0;
            project_with_pad_gap(
                &problem,
                &components,
                &mut poses,
                &mut checks,
                100_000,
                true,
                intervals,
                0.6,
                &mut PlacementSeedBroadPhaseWork::default(),
            )
            .unwrap();
            assert_eq!(&poses[..2], &original[..2]);
            for anchor in 0..2 {
                assert!(
                    geometry::overlap_correction_with_pad_gap(
                        components[2],
                        &poses[2],
                        components[anchor],
                        &poses[anchor],
                        0.2,
                        0.6
                    )
                    .is_none()
                );
            }
            validate_placement_exclusions(&problem, &poses).unwrap();
            reserved.push(poses);
        }
        assert_eq!(reserved[0], reserved[1]);
        let mut failed = original.clone();
        let mut checks = 0;
        assert!(
            project_with_pad_gap(
                &problem,
                &components,
                &mut failed,
                &mut checks,
                1,
                true,
                true,
                0.6,
                &mut PlacementSeedBroadPhaseWork::default()
            )
            .is_err()
        );
        assert_eq!(failed, original);
        assert!(checks <= 1);
        assert_eq!(serde_json::to_value(&problem).unwrap(), before);
    }
}

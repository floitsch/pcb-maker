// Copyright (C) 2026 Florian Loitsch.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use super::*;
use crate::constraints::Edge;

/// Look along the held outline side and across its permitted reach before
/// reopening the ordinary placement. All seated footprints stay put.
pub(crate) fn seat_edge_slots(problem: &Problem, poses: &mut [Pose], failed: &mut Vec<usize>) {
    let constraints = &problem.constraints;
    let authored = constraints
        .relations
        .len()
        .saturating_sub(constraints.automatic);
    let eligible: Vec<_> = failed
        .iter()
        .filter_map(|&part| {
            let mut edges = constraints.edges.iter().filter(|row| row.0 == part);
            let &(_, edge, reach) = edges.next()?;
            if edges.next().is_some()
                || problem.components[part].fixed
                || constraints.overhang(part).is_some()
                || constraints
                    .followers
                    .iter()
                    .any(|row| row.part == part || row.leader == part)
                || constraints.relations[..authored]
                    .iter()
                    .any(|row| row.involves(part))
            {
                return None;
            }
            Some((part, edge, reach))
        })
        .collect();
    if eligible.is_empty() {
        return;
    }
    let mut work = PackingWork {
        start: crate::work(),
        pending: 0,
        candidates: 0,
    };
    let mut apart = Apart::new(problem);
    let mut buckets = Buckets::new(problem);
    for (index, &pose) in poses.iter().enumerate() {
        if problem.components[index].fixed || !failed.contains(&index) {
            buckets.insert(problem, index, pose);
        }
    }
    let mut neighbours = Vec::new();
    for (part, edge, reach) in eligible {
        if work.exhausted() {
            break;
        }
        let [x, y] = poses[part].position;
        let mut best = None::<(f64, Pose)>;
        edge_slots(problem, part, poses[part], edge, reach, |candidate| {
            if work.exhausted() {
                return false;
            }
            work.candidates += 1;
            work.pending += SPOT_WORK as u64;
            let [a, b] = candidate.position;
            let distance = (a - x).powi(2) + (b - y).powi(2);
            if best.is_some_and(|(known, _)| known <= distance)
                || !fits_board(
                    problem,
                    part,
                    candidate,
                    &mut work.pending,
                    apart.board.as_ref(),
                )
            {
                return true;
            }
            buckets.near(problem, part, candidate, &mut neighbours);
            if apart.clear_of(
                problem,
                poses,
                part,
                candidate,
                neighbours.iter().copied(),
                &mut work.pending,
            ) {
                best = Some((distance, candidate));
            }
            true
        });
        if let Some((_, candidate)) = best {
            poses[part] = candidate;
            buckets.insert(problem, part, candidate);
            failed.retain(|&index| index != part);
        }
    }
    crate::add_work(work.pending);
}

/// An exterior connector may cross the artificial seam between copies if
/// no seat was found within its inferred cell. Keep its centre inside
/// the cell's depth and at least a quarter of its span on the cell's edge.
/// Any authored regions remain in force.
pub(crate) fn connector_cell_window(
    problem: &Problem,
    part: usize,
    pose: Pose,
    index: usize,
) -> Option<Problem> {
    let (_, edge, _) = problem.constraints.edges.iter().find(|row| row.0 == part)?;
    let along = usize::from(matches!(edge, Edge::Left | Edge::Right));
    let (owner, mut region) = *problem.constraints.regions.get(index)?;
    if owner != part {
        return None;
    }
    let half = problem.components[part].half_extent(pose.angle);
    for axis in 0..2 {
        let growth = if axis == along { 1.5 } else { 1.0 } * half[axis];
        region[axis] -= growth;
        region[axis + 2] += growth;
    }
    let mut trial = problem.clone();
    trial.constraints.regions[index].1 = region;
    Some(trial)
}

fn edge_slots(
    problem: &Problem,
    part: usize,
    pose: Pose,
    edge: Edge,
    reach: f64,
    mut visit: impl FnMut(Pose) -> bool,
) {
    let component = &problem.components[part];
    let slot = match edge {
        Edge::Left => 0,
        Edge::Top => 1,
        Edge::Right => 2,
        Edge::Bottom => 3,
    };
    let axis = slot % 2;
    let along = 1 - axis;
    let inward = if slot < 2 { 1.0 } else { -1.0 };
    let board = problem.bounds();
    let window = region_window(problem, part, pose.angle).unwrap_or(board);
    let half = component.half_extent(pose.angle);
    let offset = constraints::copper_edge_offsets(problem, part, pose.angle)[slot];
    let signed = constraints::copper_edge_offsets_signed(problem, part, pose.angle)[slot];
    let near = if problem.constraints.edge_copper {
        signed
    } else {
        offset
    };
    let step = if problem.grid > 0.0 {
        problem.grid.min(EDGE_PACK_STEP)
    } else {
        EDGE_PACK_STEP
    };
    let boundary = if along == 0 {
        [[board[0], board[slot]], [board[2], board[slot]]]
    } else {
        [[board[slot], board[1]], [board[slot], board[3]]]
    };
    if !visit(pose) {
        return;
    }
    let mut sides = problem
        .constraints
        .sides
        .iter()
        .filter(|row| row.0 == edge)
        .peekable();
    let fallback = (sides.peek().is_none()).then_some((edge, boundary));
    for &(_, [a, b]) in sides.chain(fallback.iter()) {
        let span = (b[along] - a[along]).abs();
        if span <= 1.0e-9 {
            continue;
        }
        let begin = a[along].min(b[along]);
        let end = a[along].max(b[along]);
        let low = (begin - half[along]).max(window[along]);
        let high = (end + half[along]).min(window[along + 2]);
        if low > high {
            continue;
        }
        let edge_at = |t: f64| {
            let proportion = ((t - a[along]) / (b[along] - a[along])).clamp(0.0, 1.0);
            a[axis] + proportion * (b[axis] - a[axis])
        };
        let mut tangent = low;
        loop {
            let first = edge_at(tangent - half[along]);
            let last = edge_at(tangent + half[along]);
            let edge_line = if inward > 0.0 {
                first.max(last)
            } else {
                first.min(last)
            };
            // Spacing measured from this exact segment, allowing the
            // same signed position at which KiCad-ready pads can sit.
            let mut shift = near;
            let far = offset + reach;
            loop {
                let mut center = [0.0; 2];
                center[along] = tangent;
                center[axis] = edge_line + inward * (half[axis] + shift);
                let candidate = Pose {
                    position: component.position_for_center(center, pose.angle),
                    angle: pose.angle,
                };
                if !visit(candidate) {
                    return;
                }
                if shift >= far {
                    break;
                }
                shift = (shift + step).min(far);
            }
            if tangent >= high {
                break;
            }
            tangent = (tangent + step).min(high);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legal::tests::pair;

    #[test]
    fn an_inward_slot_at_a_notched_edge_preserves_the_held_footprint() {
        let (mut problem, _) = pair(0.0, 0.0);
        problem.outline = vec![
            [0.0, 0.0],
            [6.0, 0.0],
            [6.0, 2.0],
            [12.0, 2.0],
            [12.0, 0.0],
            [16.0, 0.0],
            [16.0, 10.0],
            [0.0, 10.0],
        ];
        problem.constraints.sides = constraints::outline_sides(&[problem.outline.clone()]);
        problem.constraints.edges = vec![(0, Edge::Top, 2.61)];
        problem.constraints.regions = vec![(0, [6.0, 2.0, 12.0, 8.0])];
        problem.spacing = 0.5;
        problem.grid = 0.1;
        for component in &mut problem.components {
            component.pads.clear();
        }
        problem.components[0].body_size = [2.0, 2.0];
        problem.components[1].body_size = [6.0, 2.0];
        problem.components[1].fixed = true;
        let original = vec![
            Pose {
                position: [9.0, 3.0],
                angle: 0.0
            };
            2
        ];
        let mut poses = original.clone();
        let mut failed = vec![0];
        seat_edge_slots(&problem, &mut poses, &mut failed);
        assert!(failed.is_empty(), "{poses:?}");
        assert_eq!(poses[1], original[1]);
        assert_eq!(poses[0].angle, original[0].angle);
        assert!(poses[0].position[1] >= 5.5);
        assert!(crate::legal::illegal_components(&problem, &poses).is_empty());

        problem.constraints.edges[0].2 = 2.49;
        let mut poses = original.clone();
        let mut failed = vec![0];
        seat_edge_slots(&problem, &mut poses, &mut failed);
        assert_eq!(failed, [0]);
        assert_eq!(poses, original);
    }

    #[test]
    fn extending_the_inferred_cell_keeps_the_authored_region() {
        let (mut problem, _) = pair(0.0, 0.0);
        problem.components[0].body_size = [2.0, 2.0];
        problem.components[0].pads.clear();
        problem.constraints.edges = vec![(0, Edge::Top, 9.0)];
        problem.constraints.regions = vec![(0, [6.0, 2.0, 12.0, 8.0]), (0, [6.0, 2.0, 12.0, 8.0])];
        let pose = Pose {
            position: [12.1, 4.0],
            angle: 0.0,
        };
        let mut trial = connector_cell_window(&problem, 0, pose, 1).unwrap();
        assert_eq!(trial.constraints.regions[0], problem.constraints.regions[0]);
        assert!(!constraints::hard_ok(&trial, 0, pose));
        trial.constraints.regions.remove(0);
        assert!(constraints::hard_ok(&trial, 0, pose));
    }
}

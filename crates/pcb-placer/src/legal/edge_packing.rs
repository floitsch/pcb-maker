// Copyright (C) 2026 Florian Loitsch.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Pack movable parts along a shared edge when eviction cannot seat one.

use super::{Apart, Buckets, SPOT_WORK, fits_board, held_work, in_window, region_window};
use crate::{Pose, Problem, constraints};

mod slots;
pub(crate) use slots::{connector_cell_window, seat_edge_slots};

/// The step along an edge where the problem has no grid (mm).
const EDGE_PACK_STEP: f64 = 0.1;
/// The maximum work and number of candidates for a packing pass. Some
/// candidates fall outside the search window and book no geometry work.
const EDGE_PACK_WORK_LIMIT: u64 = 150_000_000;
const EDGE_PACK_CANDIDATE_LIMIT: u64 = 160_000;

struct PackingWork {
    start: u64,
    pending: u64,
    candidates: u64,
}

impl PackingWork {
    fn exhausted(&self) -> bool {
        self.candidates >= EDGE_PACK_CANDIDATE_LIMIT
            || crate::work() - self.start + self.pending >= EDGE_PACK_WORK_LIMIT
            || crate::out_of_work(self.pending)
    }
}

/// For each part of `failed` that an edge holds (still without room after
/// the eviction, which moves single parts only): the movable parts held
/// at that edge whose bodies may meet it, packed along the edge with it
/// (`pack_edge`), before it is given up. The molecules' panels hold four
/// 17.5 mm connectors at a 76.2 mm edge, 6.2 mm to spare: the legaliser
/// seated the first 5.5 mm from the edge's end and the fourth found no
/// room, though the designer's spots exist. Returns the parts still
/// without room, in `failed`'s order. Books its work only where it seats a
/// part: a packing that seats none changes nothing but the work count,
/// and booked it shifted every later decision a budget makes on the
/// boards where it was tried and failed (a part held at an edge without
/// room at some level is common: 20-odd of the sweep's boards at seed 1;
/// the attempt costs a few edge scans).
pub(crate) fn compact_edges(problem: &Problem, poses: &mut [Pose], failed: &[usize]) -> Vec<usize> {
    let edges = &problem.constraints.edges;
    if !failed
        .iter()
        .any(|index| edges.iter().any(|(part, _, _)| part == index))
    {
        return failed.to_vec();
    }
    let mut work = PackingWork {
        start: crate::work(),
        pending: 0,
        candidates: 0,
    };
    let mut unplaced = failed.to_vec();
    let mut apart = Apart::new(problem);
    for &index in failed {
        for &(_, edge, _) in edges.iter().filter(|(part, _, _)| *part == index) {
            if work.exhausted() {
                break;
            }
            if let Some(packed) = pack_edge(
                problem, poses, index, edge, &unplaced, &mut apart, &mut work,
            ) {
                poses.copy_from_slice(&packed);
                unplaced.retain(|part| *part != index);
                break;
            }
        }
    }
    if unplaced.len() < failed.len() {
        crate::add_work(work.pending);
    } else {
        // `Buckets::near` books its own.
        crate::add_work_back(crate::work() - work.start);
    }
    unplaced
}

/// The parts held at `edge` that may meet part `index` (movable, on a side
/// its body meets, seated: not in `unplaced`), it among them, packed
/// along the edge in their order there (`pack_in_order`): the others in
/// the order of their body centres along it, `index` in the slot its own
/// pose's centre gives it, else in the nearest slot that packs (a
/// connector whose own spot lies where the edge constraint cannot be
/// kept: the 7x5 molecule's J9 belongs on the wave's lowest segment, at
/// the far end of the others). None where no slot packs.
fn pack_edge(
    problem: &Problem,
    poses: &[Pose],
    index: usize,
    edge: constraints::Edge,
    unplaced: &[usize],
    apart: &mut Apart,
    work: &mut PackingWork,
) -> Option<Vec<Pose>> {
    let side = problem.components[index].side;
    let mut others: Vec<usize> = problem
        .constraints
        .edges
        .iter()
        .filter(|(part, held, _)| {
            *held == edge
                && *part != index
                && !problem.components[*part].fixed
                && !unplaced.contains(part)
                && problem.components[*part].side.collides(side)
        })
        .map(|(part, _, _)| *part)
        .collect();
    others.sort_unstable();
    others.dedup();
    let axis = match edge {
        constraints::Edge::Left | constraints::Edge::Right => 1,
        constraints::Edge::Top | constraints::Edge::Bottom => 0,
    };
    let center = |part: usize| problem.components[part].center(poses[part])[axis];
    others.sort_by(|a, b| center(*a).total_cmp(&center(*b)).then(a.cmp(b)));
    let own = center(index);
    let natural = others.partition_point(|part| center(*part) <= own);
    let mut slots: Vec<usize> = (0..=others.len()).collect();
    slots.sort_by_key(|slot| (slot.abs_diff(natural), *slot));
    for slot in slots {
        if work.exhausted() {
            break;
        }
        let mut members = others.clone();
        members.insert(slot, index);
        let mut wanted: Vec<f64> = members.iter().map(|part| center(*part)).collect();
        let before = if slot > 0 {
            wanted[slot - 1]
        } else {
            f64::NEG_INFINITY
        };
        let after = wanted.get(slot + 1).copied().unwrap_or(f64::INFINITY);
        wanted[slot] = own.clamp(before, after);
        if let Some(packed) = pack_in_order(
            problem, poses, index, &members, &wanted, axis, unplaced, apart, work,
        ) {
            return Some(packed);
        }
    }
    None
}

/// `members` (`index` among them) packed along the edge axis `axis` in
/// their order: each keeps its angle and its distance from the edge
/// (`index` flush with it), and takes the legal spot nearest `wanted`
/// within what the others leave, so the earlier ones slide only as far
/// as the later need. Three passes: every part as early as it goes
/// (`first`), as late (`last`, from the end), then each as near its
/// wanted spot as lies between the one before and its latest; where that
/// last pass fails, the early packing. None where they do not fit in
/// that order.
#[allow(clippy::too_many_arguments)]
fn pack_in_order(
    problem: &Problem,
    poses: &[Pose],
    index: usize,
    members: &[usize],
    wanted: &[f64],
    axis: usize,
    unplaced: &[usize],
    apart: &mut Apart,
    work: &mut PackingWork,
) -> Option<Vec<Pose>> {
    let bounds = problem.bounds();
    let (low, high) = (bounds[axis], bounds[axis + 2]);
    let step = if problem.grid > 0.0 {
        problem.grid
    } else {
        EDGE_PACK_STEP
    };
    let pass = |order: &mut dyn Iterator<Item = usize>,
                window: &dyn Fn(usize, f64) -> (f64, f64, f64),
                apart: &mut Apart,
                work: &mut PackingWork|
     -> Option<(Vec<Pose>, Vec<f64>)> {
        if work.exhausted() {
            return None;
        }
        let mut scratch = poses.to_vec();
        let mut buckets = Buckets::new(problem);
        for (other, pose) in poses.iter().enumerate() {
            if !members.contains(&other) && !unplaced.contains(&other) {
                buckets.insert(problem, other, *pose);
            }
        }
        let mut along = vec![f64::NAN; members.len()];
        let mut previous = f64::NAN;
        let mut near = Vec::new();
        for slot in order {
            let part = members[slot];
            let (from, to, target) = window(slot, previous);
            let pose = seat_along(
                problem,
                &scratch,
                part,
                poses[part],
                part == index,
                axis,
                [from, to, target],
                step,
                &mut buckets,
                apart,
                &mut near,
                work,
            )?;
            scratch[part] = pose;
            buckets.insert(problem, part, pose);
            along[slot] = problem.components[part].center(pose)[axis];
            previous = along[slot];
        }
        Some((scratch, along))
    };
    let (first, _) = pass(
        &mut (0..members.len()),
        &|_, previous| {
            let from = if previous.is_nan() { low } else { previous };
            (from, high, from)
        },
        apart,
        work,
    )?;
    let Some((_, last)) = pass(
        &mut (0..members.len()).rev(),
        &|_, previous| {
            let to = if previous.is_nan() { high } else { previous };
            (low, to, to)
        },
        apart,
        work,
    ) else {
        return Some(first);
    };
    let nearest = pass(
        &mut (0..members.len()),
        &|slot, previous| {
            let from = if previous.is_nan() { low } else { previous };
            let to = last[slot].max(from);
            (from, to, wanted[slot].clamp(from, to))
        },
        apart,
        work,
    );
    Some(nearest.map_or(first, |(packed, _)| packed))
}

/// The legal pose of part `part` nearest `window[2]` along the edge axis
/// `axis` (body centre between `window[0]` and `window[1]`), at `pose`'s
/// angle and distance from the edge (`flush`: flush with the edge that
/// holds it; else every angle it may take is tried after its own), in
/// steps of `step`, against the parts in `buckets` (at `poses`). None:
/// no spot.
#[allow(clippy::too_many_arguments)]
fn seat_along(
    problem: &Problem,
    poses: &[Pose],
    part: usize,
    pose: Pose,
    flush: bool,
    axis: usize,
    window: [f64; 3],
    step: f64,
    buckets: &mut Buckets,
    apart: &mut Apart,
    near: &mut Vec<usize>,
    work: &mut PackingWork,
) -> Option<Pose> {
    let component = &problem.components[part];
    let [from, to, target] = window;
    if from.is_nan() || to.is_nan() || from > to {
        return None;
    }
    let mut angles = vec![pose.angle];
    if flush {
        angles.extend(
            component
                .angle_options
                .iter()
                .copied()
                .filter(|angle| *angle != pose.angle),
        );
    }
    let start = component.center(pose);
    let steps = ((to - from) / step).ceil() as i64 + 1;
    for angle in angles {
        let regions = region_window(problem, part, angle);
        for ring in 0..=steps {
            for sign in [-1.0, 1.0] {
                if work.exhausted() {
                    return None;
                }
                work.candidates += 1;
                if ring == 0 && sign > 0.0 {
                    continue;
                }
                let t = target + sign * ring as f64 * step;
                if t < from - 1.0e-9 || t > to + 1.0e-9 {
                    continue;
                }
                let mut center = start;
                center[axis] = t;
                if flush {
                    center = constraints::flush_center(problem, part, center, angle);
                }
                let at = component.position_for_center(center, angle);
                let candidate = Pose {
                    position: constraints::snap_position(problem, part, at, angle),
                    angle,
                };
                if !in_window(regions, component.center(candidate)) {
                    continue;
                }
                work.pending += SPOT_WORK as u64 + held_work(problem, part);
                if !fits_board(
                    problem,
                    part,
                    candidate,
                    &mut work.pending,
                    apart.board.as_ref(),
                ) {
                    continue;
                }
                buckets.near(problem, part, candidate, near);
                near.retain(|other| *other != part);
                if apart.clear_of(
                    problem,
                    poses,
                    part,
                    candidate,
                    near.iter().copied(),
                    &mut work.pending,
                ) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::tests::{pair, resistor};
    use super::super::{evict_for, illegal_components, is_legal};
    use super::*;

    /// `count` connectors 17.5 mm long held at the right edge of a board
    /// 77.2 mm tall (76.2 mm inside the edge margin), the first 5.5 mm
    /// from the top's margin, each 0.5 mm below the one before.
    fn edge_row(count: usize) -> (Problem, Vec<Pose>) {
        let (mut problem, _) = pair(0.0, 0.0);
        problem.outline = vec![[0.0, 0.0], [30.0, 0.0], [30.0, 77.2], [0.0, 77.2]];
        problem.spacing = 0.5;
        let mut connector = resistor(0.0);
        connector.body_size = [2.0, 17.5];
        connector.pads = vec![[-0.5, -8.5, 0.5, 8.5]];
        problem.components = vec![connector; count];
        let poses: Vec<Pose> = (0..count)
            .map(|index| Pose {
                position: [28.5, 0.5 + 5.5 + 8.75 + 18.0 * index as f64],
                angle: 0.0,
            })
            .collect();
        problem.constraints.edges = (0..count)
            .map(|index| (index, constraints::Edge::Right, 0.52))
            .collect();
        problem.poses = poses.clone();
        (problem, poses)
    }

    #[test]
    fn parts_held_at_an_edge_are_packed_along_it_to_make_room() {
        // The molecules' panels: four 17.5 mm connectors on a 76.2 mm
        // edge, the first 5.5 mm from its end; the fourth found no room,
        // and the eviction (one part at a time) none either.
        let (problem, poses) = edge_row(4);
        assert!(!is_legal(&problem, &poses, 3, poses[3], 0..3));
        let mut evicted = poses.clone();
        assert_eq!(evict_for(&problem, &mut evicted, &[3]), vec![3]);
        let mut packed = poses.clone();
        assert!(compact_edges(&problem, &mut packed, &[3]).is_empty());
        assert!(
            illegal_components(&problem, &packed).is_empty(),
            "{packed:?}"
        );
        // In their order, each as near its own spot as the room allows:
        // the first slid up, by less than the 5.5 mm the others needed
        // all of.
        for index in 1..4 {
            assert!(packed[index].position[1] > packed[index - 1].position[1]);
        }
        let slid = poses[0].position[1] - packed[0].position[1];
        assert!(slid > 0.0 && slid < 5.5, "{slid}");
        // Five do not fit: nothing moves, and no work is booked.
        let (problem, poses) = edge_row(5);
        let mut packed = poses.clone();
        let before = crate::work();
        assert_eq!(compact_edges(&problem, &mut packed, &[4]), vec![4]);
        assert_eq!(packed, poses);
        assert_eq!(
            crate::work(),
            before,
            "a packing that seats none books no work"
        );
        // A part no edge holds: nothing to pack, no work.
        let (mut problem, poses) = edge_row(4);
        problem.constraints.edges.pop();
        let mut packed = poses.clone();
        let before = crate::work();
        assert_eq!(compact_edges(&problem, &mut packed, &[3]), vec![3]);
        assert_eq!(crate::work(), before);
    }

    #[test]
    fn packing_an_edge_respects_the_remaining_work_budget() {
        let (problem, poses) = edge_row(4);
        let mut packed = poses.clone();
        let before = crate::work();
        let _budget = crate::StopGuard::set(Some(before + 1));
        assert_eq!(compact_edges(&problem, &mut packed, &[3]), vec![3]);
        assert_eq!(packed, poses);
        assert_eq!(crate::work(), before);
    }

    #[test]
    fn a_part_held_at_an_edge_takes_another_slot_where_its_own_cannot_be_kept() {
        // The 7x5 molecule's J9: its own spot lay past the others along
        // the edge, where its constraint cannot be kept (there the wave's
        // edge is higher); its place is before them. Here a region keeps
        // the stuck part in the top 30 mm, its pose lies past the other.
        let (mut problem, mut poses) = edge_row(2);
        problem
            .constraints
            .regions
            .push((1, [0.0, 0.0, 30.0, 30.0]));
        poses[0].position[1] = 40.0;
        poses[1].position[1] = 70.0;
        let mut packed = poses.clone();
        assert!(compact_edges(&problem, &mut packed, &[1]).is_empty());
        assert!(
            illegal_components(&problem, &packed).is_empty(),
            "{packed:?}"
        );
        assert!(packed[1].position[1] < packed[0].position[1]);
    }
}

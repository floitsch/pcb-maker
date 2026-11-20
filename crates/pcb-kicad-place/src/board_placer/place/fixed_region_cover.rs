// Copyright (C) 2026 Florian Loitsch.

//! Prove a region infeasible from a union of fixed same-face bodies.

use pcb_placer as core;
use std::collections::BTreeSet;

use super::KiCadPlacementRegionRotation;

const MARGIN: f64 = 2.0e-6;
const MAX_BOXES: usize = 32;

/// Indices of fixed footprints sufficient to obstruct every permitted
/// centre at every fitting rotation. Unsupported shapes contribute no box.
pub(super) fn blockers(
    problem: &core::Problem,
    poses: &[core::Pose],
    index: usize,
    rotations: &[&KiCadPlacementRegionRotation],
    uuids: &[Option<String>],
) -> BTreeSet<usize> {
    let part = &problem.components[index];
    if rotations.is_empty()
        || part.round
        || part.copper_only
        || part.fixed
        || !matches!(part.side, core::Side::Front | core::Side::Back)
        || part.halo < 0.0
        || problem.spacing < 0.0
    {
        return BTreeSet::new();
    }
    let fixed = problem
        .components
        .iter()
        .enumerate()
        .filter(|&(other, body)| {
            body.fixed
                && uuids.get(other).is_some_and(Option::is_some)
                && body.halo >= 0.0
                && part.side.collides(body.side)
                && !body.copper_only
                && !part.hollow_for(body)
                && !body.hollow_for(part)
                && problem.constraints.frames.pass(index, other).is_none()
                && !problem.constraints.lies_on(index, other)
        })
        .filter_map(|(other, body)| {
            let pose = poses[other];
            if !body.round && pose.angle % 90.0 != 0.0 {
                return None;
            }
            let half = if body.round {
                // The inscribed square may establish a collision. A disc's
                // circumscribing box would produce false corner collisions.
                [body.body_size[0] * 0.5 * std::f64::consts::FRAC_1_SQRT_2; 2]
            } else {
                body.half_extent(pose.angle)
            };
            Some((other, body.center(pose), half))
        })
        .collect::<Vec<_>>();
    let mut proof = BTreeSet::new();
    for rotation in rotations {
        if rotation.angle_deg % 90.0 != 0.0 {
            return BTreeSet::new();
        }
        let Some([x0, y0, x1, y1]) = rotation.center_bounds_mm else {
            return BTreeSet::new();
        };
        // Enlarge the centre domain by more than hard_ok's tolerance and
        // shrink collision bounds so their closed edges also prove overlap.
        let domain = [x0 - MARGIN, y0 - MARGIN, x1 + MARGIN, y1 + MARGIN];
        let [dx, dy] = rotation.body_size_mm.map(|size| size * 0.5 - MARGIN);
        let mut boxes = fixed
            .iter()
            .filter_map(|&(other, [x, y], [hx, hy])| {
                let clipped = [
                    domain[0].max(x - hx - dx),
                    domain[1].max(y - hy - dy),
                    domain[2].min(x + hx + dx),
                    domain[3].min(y + hy + dy),
                ];
                (clipped[0] < clipped[2] && clipped[1] < clipped[3]).then_some((other, clipped))
            })
            .collect::<Vec<_>>();
        // Searching a subset can miss a proof, but cannot invent one.
        let area = |b: [f64; 4]| (b[2] - b[0]) * (b[3] - b[1]);
        boxes.sort_by(|a, b| area(b.1).total_cmp(&area(a.1)).then(a.0.cmp(&b.0)));
        boxes.truncate(MAX_BOXES);
        let Some(used) = covers(domain, &boxes) else {
            return BTreeSet::new();
        };
        proof.extend(used);
    }
    proof
}

/// Divide the rectangle into x slabs. Active rectangles span a slab in
/// full, so greedily covering its y interval also covers its boundaries.
fn covers(domain: [f64; 4], boxes: &[(usize, [f64; 4])]) -> Option<BTreeSet<usize>> {
    let mut cuts = vec![domain[0], domain[2]];
    cuts.extend(boxes.iter().flat_map(|(_, b)| [b[0], b[2]]));
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let mut used = BTreeSet::new();
    for slab in cuts.windows(2) {
        let mut y = domain[1];
        while y < domain[3] {
            let (id, bounds) = boxes
                .iter()
                .filter(|(_, b)| b[0] <= slab[0] && b[2] >= slab[1] && b[1] <= y && b[3] > y)
                .max_by(|a, b| a.1[3].total_cmp(&b.1[3]))?;
            used.insert(*id);
            y = bounds[3];
        }
    }
    Some(used)
}

// Copyright (C) 2026 Toit contributors.

//! A link's copper: its centreline offset into the pair's two tracks and
//! vias, and the checks between the pair's own two nets.

use super::*;
use pcb_router as core;

/// At a layer change the pair's two vias move apart along the bisector of
/// the runs' normals by up to 1 / this (a right-angle turn).
pub(super) const VIA_TURN: f64 = 0.7;

/// One link's copper, offset from its centreline.
#[derive(Clone, Default)]
pub(super) struct LinkCopper {
    /// Per polarity (0 positive, 1 negative): segments (layer, start, end)
    /// and vias.
    pub segments: [Vec<(usize, Point, Point)>; 2],
    pub vias: [Vec<Point>; 2],
    pub coupled: f64,
    pub crossed: bool,
    /// Crossovers in its leads (`Crossover`).
    pub crossovers: usize,
    /// Per polarity: the coupled run's tracks among `segments` (the rest
    /// are stubs, jogs and crossovers), each along the run from the first
    /// end, its partner on its right where the sign is 1 (left -1).
    pub run: [std::ops::Range<usize>; 2],
    /// Whether it gives way to the links on their routes (where their
    /// copper meets, it alone is refused): its centreline is not the
    /// router's route as it is (`centrelines`: moved where the router did
    /// not check the room), or its stubs are still to route
    /// (`repair_stubs`).
    pub moved: bool,
}

/// One side of a centreline polyline, `offset` to its left (`side` 1) or
/// right (-1): mitred on the inside of a bend where both segments are long
/// enough, bevelled on the outside (it stays inside the envelope).
pub(super) fn offset_line(points: &[Point], side: f64, offset: f64) -> Vec<Point> {
    let mut out = Vec::new();
    let count = points.len();
    if count < 2 {
        return out;
    }
    let normal = |i: usize| scale(left(unit(sub(points[i + 1], points[i]))), side);
    out.push(add(points[0], scale(normal(0), offset)));
    for i in 1..count - 1 {
        let (n1, n2) = (normal(i - 1), normal(i));
        let (u1, u2) = (
            unit(sub(points[i], points[i - 1])),
            unit(sub(points[i + 1], points[i])),
        );
        let inner = cross(u1, u2) * side > 0.0;
        let denominator = 1.0 + dot(n1, n2);
        let miter = scale(add(n1, n2), offset / denominator.max(EPSILON));
        // How far along each segment the miter point lies from the corner.
        let along = norm(sub(miter, scale(n1, offset)));
        if inner
            && denominator > 0.3
            && along <= core::geometry::distance(points[i], points[i - 1])
            && along <= core::geometry::distance(points[i + 1], points[i])
        {
            out.push(add(points[i], miter));
        } else {
            out.push(add(points[i], scale(n1, offset)));
            out.push(add(points[i], scale(n2, offset)));
        }
    }
    out.push(add(points[count - 1], scale(normal(count - 2), offset)));
    out
}

/// The spreads `finish_pairs` tries for a link's vias where those of
/// `Pair::via_offset` come too close to the other net (`own_vias_clear`).
pub(super) const VIA_SPREADS: [f64; 3] = [1.5, 2.0, 3.0];

/// Whether each via of a link's copper keeps from the other net's tracks
/// and vias what the copper check asks between the pair's two nets (the
/// via's copper the gap, below the clearance; its hole the hole clearance;
/// hole to hole).
pub(super) fn own_vias_clear(copper: &LinkCopper, pair: &Pair) -> bool {
    let near = pair.gap.min(pair.clearance) - EPSILON;
    let (radius, drill) = (pair.via_diameter / 2.0, pair.via_drill / 2.0);
    (0..2).all(|polarity| {
        copper.vias[polarity].iter().all(|&via| {
            copper.segments[1 - polarity].iter().all(|&(_, from, to)| {
                let distance =
                    core::geometry::point_segment_distance(via, from, to) - pair.width / 2.0;
                distance - radius >= near && distance - drill >= pair.hole_clearance - EPSILON
            }) && copper.vias[1 - polarity].iter().all(|&other| {
                let distance = core::geometry::distance(via, other);
                distance - 2.0 * radius >= near
                    && distance - radius - drill >= pair.hole_clearance - EPSILON
                    && distance - 2.0 * drill >= pair.hole_to_hole - EPSILON
            })
        })
    })
}

/// Whether a link's two nets keep from each other what the copper check
/// asks between them (`refused_groups`: the pair's gap where it is below
/// the clearance), tracks on one layer and vias (`own_vias_clear`).
pub(super) fn own_copper_clear(copper: &LinkCopper, pair: &Pair) -> bool {
    let near = pair.width + pair.gap.min(pair.clearance) - EPSILON;
    own_vias_clear(copper, pair)
        && copper.segments[0].iter().all(|&(layer, a, b)| {
            copper.segments[1]
                .iter()
                .filter(|(other, ..)| *other == layer)
                .all(|&(_, c, d)| core::geometry::segment_segment_distance(a, b, c, d) >= near)
        })
}

/// The copper of a link from its path: the two offset tracks per layer
/// run and a via of each net side by side at each layer change; the
/// positive net on `side` of the run (1 left, -1 right), the vias
/// `spread` times as far from the centreline as `Pair::via_offset`.
pub(super) fn link_copper(
    path: &[(Point, usize)],
    pair: &Pair,
    side: f64,
    spread: f64,
) -> Option<LinkCopper> {
    // Runs on one layer, split at the layer changes (the vias).
    let mut runs: Vec<(usize, Vec<Point>)> = Vec::new();
    for &(point, layer) in path {
        match runs.last_mut() {
            Some((known, points)) if *known == layer => points.push(point),
            _ => runs.push((layer, vec![point])),
        }
    }
    let runs: Vec<(usize, Vec<Point>)> = runs
        .into_iter()
        .map(|(layer, points)| (layer, simplified(&points)))
        .collect();
    let directions: Vec<Option<(Point, Point)>> = runs
        .iter()
        .map(|(_, points)| {
            (points.len() >= 2).then(|| {
                (
                    unit(sub(points[1], points[0])),
                    unit(sub(points[points.len() - 1], points[points.len() - 2])),
                )
            })
        })
        .collect();
    directions.iter().flatten().next()?;
    let mut copper = LinkCopper::default();
    let d = pair.offset();
    let dv = pair.via_offset() * spread;
    for (polarity, sign) in [(0, side), (1, -side)] {
        let mut previous_end: Option<(Point, usize)> = None;
        let mut incoming: Option<Point> = None;
        for (index, (layer, points)) in runs.iter().enumerate() {
            let line = offset_line(points, sign, d);
            if index > 0 {
                // The via between the previous run and this one.
                let at = points[0];
                let outgoing = directions[index..]
                    .iter()
                    .flatten()
                    .next()
                    .map(|(first, _)| *first)
                    .or(incoming)?;
                let into = incoming.unwrap_or(outgoing);
                let mut bisector = add(left(into), left(outgoing));
                if norm(bisector) < 0.2 {
                    bisector = left(into);
                }
                // Turned at the via, the vias move out along the bisector
                // so that across either run they are as far apart as
                // straight (up to a right angle).
                let half_turn_cosine = dot(unit(bisector), left(into)).abs().max(VIA_TURN);
                let via = add(at, scale(unit(bisector), sign * dv / half_turn_cosine));
                copper.vias[polarity].push(via);
                if let Some((from, from_layer)) = previous_end
                    && core::geometry::distance(from, via) > 1.0e-6
                {
                    copper.segments[polarity].push((from_layer, from, via));
                }
                if let Some(&to) = line.first()
                    && core::geometry::distance(to, via) > 1.0e-6
                {
                    copper.segments[polarity].push((*layer, via, to));
                }
                if line.is_empty() {
                    previous_end = Some((via, *layer));
                }
            }
            for pair_of_points in line.windows(2) {
                let length = core::geometry::distance(pair_of_points[0], pair_of_points[1]);
                if length > 1.0e-6 {
                    copper.segments[polarity].push((*layer, pair_of_points[0], pair_of_points[1]));
                    if polarity == 0 {
                        copper.coupled += length;
                    }
                }
            }
            if let Some(&last_point) = line.last() {
                previous_end = Some((last_point, *layer));
            }
            if let Some((_, last_direction)) = directions[index] {
                incoming = Some(last_direction);
            }
        }
    }
    (!copper.segments[0].is_empty() && !copper.segments[1].is_empty()).then_some(copper)
}

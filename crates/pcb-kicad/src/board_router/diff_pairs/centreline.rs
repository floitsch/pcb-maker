// Copyright (C) 2026 Toit contributors.

//! A link's centreline from its route: the path through its segments and
//! vias, and the variants `finish_pairs` tries (without hairpins, with
//! the vias on straight pieces).

use super::*;
use pcb_router as core;

/// A point of a route (nanometres) on a layer: `link_path`'s graph node.
type PathNode = ((i64, i64), usize);

/// A link's path from its net's route: (point, layer) from the launch at
/// `start` to the one at `end`, through its segments and vias.
pub(super) fn link_path(
    route: &core::NetRoute,
    start: Point,
    end: Point,
) -> Option<Vec<(Point, usize)>> {
    let key = |point: Point| {
        (
            (point[0] * 1.0e6).round() as i64,
            (point[1] * 1.0e6).round() as i64,
        )
    };
    let mut points: BTreeMap<(i64, i64), Point> = BTreeMap::new();
    let mut edges: BTreeMap<PathNode, Vec<PathNode>> = BTreeMap::new();
    let mut layers_at: BTreeMap<(i64, i64), BTreeSet<usize>> = BTreeMap::new();
    for segment in &route.segments {
        let (a, b) = (key(segment.start), key(segment.end));
        points.insert(a, segment.start);
        points.insert(b, segment.end);
        edges
            .entry((a, segment.layer))
            .or_default()
            .push((b, segment.layer));
        edges
            .entry((b, segment.layer))
            .or_default()
            .push((a, segment.layer));
        layers_at.entry(a).or_default().insert(segment.layer);
        layers_at.entry(b).or_default().insert(segment.layer);
    }
    for via in &route.vias {
        let at = key(via.at);
        let Some(layers) = layers_at.get(&at).cloned() else {
            continue;
        };
        for &from in &layers {
            for &to in &layers {
                if from != to {
                    edges.entry((at, from)).or_default().push((at, to));
                }
            }
        }
    }
    let nearest = |target: Point| {
        edges
            .keys()
            .min_by(|a, b| {
                core::geometry::distance(points[&a.0], target)
                    .total_cmp(&core::geometry::distance(points[&b.0], target))
            })
            .copied()
    };
    let (from, to) = (nearest(start)?, nearest(end)?);
    if core::geometry::distance(points[&from.0], start) > 0.5
        || core::geometry::distance(points[&to.0], end) > 0.5
    {
        return None;
    }
    // Breadth first over the tree.
    let mut previous: BTreeMap<PathNode, PathNode> = BTreeMap::new();
    let mut queue = std::collections::VecDeque::from([from]);
    previous.insert(from, from);
    while let Some(node) = queue.pop_front() {
        if node == to {
            break;
        }
        for next in edges.get(&node).into_iter().flatten() {
            if !previous.contains_key(next) {
                previous.insert(*next, node);
                queue.push_back(*next);
            }
        }
    }
    previous.get(&to)?;
    let mut path = vec![to];
    while *path.last()? != from {
        path.push(previous[path.last()?]);
    }
    path.reverse();
    Some(
        path.into_iter()
            .map(|(point, layer)| (points[&point], layer))
            .collect(),
    )
}

/// A link's path without its hairpins, which the offset tracks cannot
/// follow apart: a point of one layer where the path turns back by more
/// than a right angle goes, and two turns the same way with less than
/// `short` between them become one at the lines' meeting point, where that
/// lies within `short` of both (the first and last two points, the run's
/// lead, stay).
pub(super) fn despiked(path: &[(Point, usize)], short: f64) -> Vec<(Point, usize)> {
    let mut out = path.to_vec();
    let same_layer = |points: &[(Point, usize)]| points.windows(2).all(|w| w[0].1 == w[1].1);
    loop {
        let count = out.len();
        let spike = (2..count.saturating_sub(2)).find(|&i| {
            let (a, b, c) = (out[i - 1], out[i], out[i + 1]);
            same_layer(&out[i - 1..=i + 1])
                && core::geometry::distance(a.0, b.0) > 1.0e-6
                && core::geometry::distance(b.0, c.0) > 1.0e-6
                && dot(unit(sub(b.0, a.0)), unit(sub(c.0, b.0))) < -1.0e-3
        });
        if let Some(i) = spike {
            out.remove(i);
            continue;
        }
        // A short piece between two turns the same way: A B C D, B to C
        // short.
        let merged = (2..count.saturating_sub(3)).find_map(|i| {
            let (a, b, c, e) = (out[i - 1].0, out[i].0, out[i + 1].0, out[i + 2].0);
            if !same_layer(&out[i - 1..=i + 2]) || core::geometry::distance(b, c) >= short {
                return None;
            }
            let (u1, u2, u3) = (unit(sub(b, a)), unit(sub(c, b)), unit(sub(e, c)));
            let (t1, t2) = (cross(u1, u2), cross(u2, u3));
            if t1 * t2 <= 0.0 || t1.abs() < 1.0e-6 || t2.abs() < 1.0e-6 {
                return None;
            }
            // Where line A B meets line C D.
            let denominator = cross(u1, u3);
            if denominator.abs() < 1.0e-6 {
                return None;
            }
            let along = cross(sub(c, b), u3) / denominator;
            let meet = add(b, scale(u1, along));
            (along > 0.0
                && core::geometry::distance(meet, b) <= short
                && core::geometry::distance(meet, c) <= short
                && dot(sub(meet, a), u1) > 0.0
                && dot(sub(e, meet), u3) > 0.0)
                .then_some((i, meet))
        });
        match merged {
            Some((i, meet)) => {
                out[i].0 = meet;
                out.remove(i + 1);
            }
            None => return out,
        }
    }
}

/// The centrelines `finish_pairs` tries for a link's copper, in order: the
/// route as it is, without its hairpins and short jogs (`despiked`), with
/// its layer changes moved off its turns (`vias_on_straights`), both, then
/// with longer jogs straightened; no two alike. The first is the router's
/// own; the others were not checked by it (`LinkCopper::moved`).
pub(super) fn centrelines(path: &[(Point, usize)], pair: &Pair) -> Vec<Vec<(Point, usize)>> {
    let shift = 2.0 * pair.via_offset() + pair.width;
    let moved = vias_on_straights(path, shift);
    let d = pair.offset();
    let mut out: Vec<Vec<(Point, usize)>> = Vec::new();
    for candidate in [
        path.to_vec(),
        despiked(path, 2.5 * d),
        moved.clone(),
        despiked(&moved, 2.5 * d),
        despiked(path, 5.0 * d),
        despiked(&moved, 5.0 * d),
        despiked(path, 10.0 * d),
    ] {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

/// A link's centreline with each layer change in a turn moved `shift`
/// back along the segment before it (or on along the one after it, where
/// that is longer), so that the two vias stand on a straight piece and the
/// turn is on one layer: at a via in a turn the offset tracks of the two
/// layers meet the vias across (jetson's PCIE1_TX0: a via of one net 0.14
/// mm into the other net's track). Turns closer to the ends than the lead
/// (the first and last two points) and segments too short stay as they
/// are.
pub(super) fn vias_on_straights(path: &[(Point, usize)], shift: f64) -> Vec<(Point, usize)> {
    let mut out: Vec<(Point, usize)> = Vec::with_capacity(path.len() + 4);
    let count = path.len();
    let mut index = 0;
    while index < count {
        let (point, layer) = path[index];
        let change = index >= 2
            && index + 3 < count
            && path[index + 1].1 != layer
            && core::geometry::distance(path[index + 1].0, point) < 1.0e-6;
        if !change {
            out.push(path[index]);
            index += 1;
            continue;
        }
        let (before, after) = (path[index - 1].0, path[index + 2].0);
        let next_layer = path[index + 1].1;
        let (back, on) = (
            core::geometry::distance(before, point),
            core::geometry::distance(point, after),
        );
        let (incoming, outgoing) = (unit(sub(point, before)), unit(sub(after, point)));
        let straight = back < EPSILON || on < EPSILON || dot(incoming, outgoing) > 0.999;
        if straight || back.max(on) < shift + EPSILON {
            out.push(path[index]);
            index += 1;
            continue;
        }
        if back >= on {
            let moved = sub(point, scale(incoming, shift));
            out.push((moved, layer));
            out.push((moved, next_layer));
            out.push((point, next_layer));
        } else {
            let moved = add(point, scale(outgoing, shift));
            out.push((point, layer));
            out.push((moved, layer));
            out.push((moved, next_layer));
        }
        // Past the via's second point.
        index += 2;
    }
    out
}

/// A polyline's points without repeats and without points in a straight
/// line between their neighbours.
pub(super) fn simplified(points: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::new();
    for &point in points {
        if out
            .last()
            .is_some_and(|last| core::geometry::distance(*last, point) < 1.0e-6)
        {
            continue;
        }
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            if cross(sub(b, a), sub(point, b)).abs() < 1.0e-9 && dot(sub(b, a), sub(point, b)) > 0.0
            {
                out.pop();
            }
        }
        out.push(point);
    }
    out
}

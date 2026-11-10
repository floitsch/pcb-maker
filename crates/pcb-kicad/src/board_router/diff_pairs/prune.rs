// Copyright (C) 2026 Toit contributors.

//! What the pairs' copper leaves dangling goes: a failed link's stub, any
//! track or via that joins nothing, a repaired stub's old way.

use super::*;
use pcb_router as core;

/// Removes what is left of a failed link's stub on `net`'s route: from
/// the stub's end (its copper `lead`, `reach` around it) along the tracks
/// whose end there touches nothing else of the net (no other track, via,
/// pad or kept copper), up to the first junction (KiCad's
/// `track_dangling`, an error in some projects).
pub(super) fn prune_stub(
    board: &core::Board,
    route: &mut core::NetRoute,
    net: core::NetId,
    lead: (Point, Point),
    reach: f64,
) {
    let own: Vec<&core::Obstacle> = board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.net == Some(net) && obstacle.kind == core::ObstacleKind::Copper)
        .collect();
    // The dangling end at the stub's end.
    let mut at: Option<(Point, usize)> =
        route
            .segments
            .iter()
            .enumerate()
            .find_map(|(index, segment)| {
                [segment.start, segment.end]
                    .into_iter()
                    .find(|&end| {
                        core::geometry::point_segment_distance(end, lead.0, lead.1)
                            <= reach + 1.0e-6
                            && !end_touches(route, &own, index, end, segment.layer)
                    })
                    .map(|end| (end, segment.layer))
            });
    while let Some((point, layer)) = at.take() {
        let Some(index) = route.segments.iter().position(|segment| {
            segment.layer == layer
                && (core::geometry::distance(segment.start, point) < 1.0e-6
                    || core::geometry::distance(segment.end, point) < 1.0e-6)
        }) else {
            break;
        };
        if end_touches(route, &own, index, point, layer) {
            break;
        }
        let segment = route.segments.remove(index);
        let other = if core::geometry::distance(segment.start, point) < 1.0e-6 {
            segment.end
        } else {
            segment.start
        };
        let alone = !route.segments.iter().any(|candidate| {
            candidate.layer == layer
                && core::geometry::point_segment_distance(other, candidate.start, candidate.end)
                    <= (candidate.width + segment.width) / 2.0 + 1.0e-6
        });
        if alone {
            at = past_via(route, &own, other);
            continue;
        }
        // The next track of the chain, if `other` is its end and touches
        // nothing but it.
        let next = route.segments.iter().enumerate().filter(|(_, candidate)| {
            candidate.layer == layer
                && core::geometry::point_segment_distance(other, candidate.start, candidate.end)
                    <= (candidate.width + segment.width) / 2.0 + 1.0e-6
        });
        let next: Vec<(usize, &core::Segment)> = next.collect();
        if let [(index, candidate)] = next[..]
            && (core::geometry::distance(candidate.start, other) < 1.0e-6
                || core::geometry::distance(candidate.end, other) < 1.0e-6)
            && !end_touches(route, &own, index, other, layer)
        {
            at = Some((other, layer));
        }
    }
}

/// Whether a track end of `width` at `point` overlaps other copper of
/// the net (as the router joins copper: `joins_copper`): `route`'s other
/// tracks on `layer` and vias, the net's copper `own`.
/// The tracks that meet the track's other end do not count (KiCad finds
/// a short track's end dangling though its copper overlaps the next).
fn end_touches(
    route: &core::NetRoute,
    own: &[&core::Obstacle],
    index: usize,
    point: Point,
    layer: usize,
) -> bool {
    let half = route
        .segments
        .get(index)
        .map_or(0.0, |segment| segment.width / 2.0);
    let far = route.segments.get(index).map(|segment| {
        if core::geometry::distance(segment.start, point) < 1.0e-6 {
            segment.end
        } else {
            segment.start
        }
    });
    let meets_far = |segment: &core::Segment| {
        far.is_some_and(|far| {
            core::geometry::distance(segment.start, far) < 1.0e-6
                || core::geometry::distance(segment.end, far) < 1.0e-6
        })
    };
    route.segments.iter().enumerate().any(|(other, segment)| {
        other != index
            && !meets_far(segment)
            && segment.layer == layer
            && core::geometry::point_segment_distance(point, segment.start, segment.end)
                <= segment.width / 2.0 + half + 1.0e-6
    }) || route
        .vias
        .iter()
        .any(|via| core::geometry::distance(via.at, point) <= via.diameter / 2.0 + half + 1.0e-6)
        || own.iter().any(|obstacle| {
            obstacle.layers & (1 << layer) != 0
                && obstacle.shape.distance_to_point(point) <= half + 1.0e-6
        })
}

/// Where a pruned track's far end `other` joins nothing else on its layer:
/// a via there that joins nothing else of the net but one track on another
/// layer goes too, and the chain goes on along that track (a stub that
/// changed layers: KiCad's via_dangling); the next end to prune, if any.
fn past_via(
    route: &mut core::NetRoute,
    own: &[&core::Obstacle],
    other: Point,
) -> Option<(Point, usize)> {
    let via = route
        .vias
        .iter()
        .position(|via| core::geometry::distance(via.at, other) < 1.0e-6)?;
    let (at_via, diameter) = (route.vias[via].at, route.vias[via].diameter);
    if own
        .iter()
        .any(|obstacle| obstacle.shape.distance_to_point(at_via) <= diameter / 2.0 + 1.0e-6)
        || route.vias.iter().enumerate().any(|(index, other_via)| {
            index != via
                && core::geometry::distance(other_via.at, at_via)
                    <= (other_via.diameter + diameter) / 2.0 + 1.0e-6
        })
    {
        return None;
    }
    let meeting: Vec<&core::Segment> = route
        .segments
        .iter()
        .filter(|candidate| {
            core::geometry::point_segment_distance(at_via, candidate.start, candidate.end)
                <= (candidate.width + diameter) / 2.0 + 1.0e-6
        })
        .collect();
    match meeting[..] {
        [] => {
            route.vias.remove(via);
            None
        }
        [next]
            if core::geometry::distance(next.start, at_via) < 1.0e-6
                || core::geometry::distance(next.end, at_via) < 1.0e-6 =>
        {
            let next_layer = next.layer;
            route.vias.remove(via);
            Some((at_via, next_layer))
        }
        _ => None,
    }
}

/// The pieces of copper `route` and `own` (a net's pads and kept copper)
/// make, as the router joins copper (overlap), without the segment
/// `without`.
pub(super) fn pieces(
    route: &core::NetRoute,
    own: &[&core::Obstacle],
    without: Option<usize>,
) -> usize {
    let (o, n) = (own.len(), route.segments.len());
    let total = o + n + route.vias.len();
    let mut parent: Vec<usize> = (0..total).collect();
    let joined = |a: usize, b: usize| -> bool {
        let segment = |index: usize| &route.segments[index - o];
        let via = |index: usize| &route.vias[index - o - n];
        match (a < o, a >= o && a < o + n, b < o, b >= o && b < o + n) {
            (true, _, true, _) => false,
            (true, _, _, true) | (_, true, true, _) => {
                let (obstacle, track) = if a < o {
                    (own[a], segment(b))
                } else {
                    (own[b], segment(a))
                };
                obstacle.layers & (1 << track.layer) != 0
                    && obstacle.shape.distance_to_segment(track.start, track.end)
                        <= track.width / 2.0 + 1.0e-6
            }
            (true, _, _, false) | (_, false, true, _) => {
                let (obstacle, hole) = if a < o {
                    (own[a], via(b))
                } else {
                    (own[b], via(a))
                };
                obstacle.shape.distance_to_point(hole.at) <= hole.diameter / 2.0 + 1.0e-6
            }
            (_, true, _, true) => {
                let (x, y) = (segment(a), segment(b));
                x.layer == y.layer
                    && core::geometry::segment_segment_distance(x.start, x.end, y.start, y.end)
                        <= (x.width + y.width) / 2.0 + 1.0e-6
            }
            (_, true, _, false) | (_, false, _, true) => {
                let (track, hole) = if a < o + n {
                    (segment(a), via(b))
                } else {
                    (segment(b), via(a))
                };
                core::geometry::point_segment_distance(hole.at, track.start, track.end)
                    <= (track.width + hole.diameter) / 2.0 + 1.0e-6
            }
            (_, false, _, false) => false,
        }
    };
    let skip = without.map(|index| o + index);
    for a in 0..total {
        for b in a + 1..total {
            if Some(a) != skip && Some(b) != skip && joined(a, b) {
                let (x, y) = (root(&mut parent, a), root(&mut parent, b));
                if x != y {
                    parent[x.max(y)] = x.min(y);
                }
            }
        }
    }
    (0..total)
        .filter(|&item| Some(item) != skip && root(&mut parent, item) == item)
        .count()
}

/// Removes `net`'s tracks with an end that joins nothing else of the net
/// (no track, via, pad or kept copper its copper overlaps there, but the
/// tracks meeting its other end), and its vias that join no copper (a via
/// with copper on one layer stays: a zone, a designer's teardrop, may hold
/// it), until none is left; also a track no longer than its
/// width whose end lies in no other copper (KiCad finds such an end
/// dangling though it overlaps the next track) where removing it splits no
/// piece of the net's copper.
pub(super) fn prune_dangling(board: &core::Board, route: &mut core::NetRoute, net: core::NetId) {
    let own: Vec<&core::Obstacle> = board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.net == Some(net) && obstacle.kind == core::ObstacleKind::Copper)
        .collect();
    let near = |a: Point, b: Point| core::geometry::distance(a, b) < 1.0e-6;
    // Whether the end `point` of segment `index` (its other end `far`) is
    // held: by overlap, or (`inside`) by lying in the other track's copper.
    let held = |route: &core::NetRoute, index: usize, point: Point, far: Point, inside: bool| {
        let segment = &route.segments[index];
        let half = segment.width / 2.0;
        route.segments.iter().enumerate().any(|(other, candidate)| {
            other != index
                && candidate.layer == segment.layer
                && !(near(candidate.start, far) || near(candidate.end, far))
                && core::geometry::point_segment_distance(point, candidate.start, candidate.end)
                    <= candidate.width / 2.0 + if inside { 0.0 } else { half } + 1.0e-6
        }) || route.vias.iter().any(|via| {
            core::geometry::distance(via.at, point) <= via.diameter / 2.0 + half + 1.0e-6
        }) || own.iter().any(|obstacle| {
            obstacle.layers & (1 << segment.layer) != 0
                && obstacle.shape.distance_to_point(point) <= half + 1.0e-6
        })
    };
    let ends =
        |segment: &core::Segment| [(segment.start, segment.end), (segment.end, segment.start)];
    loop {
        let dangling = (0..route.segments.len()).find(|&index| {
            ends(&route.segments[index])
                .into_iter()
                .any(|(point, far)| !held(route, index, point, far, false))
        });
        if let Some(index) = dangling {
            route.segments.remove(index);
            continue;
        }
        let nub = (0..route.segments.len()).find(|&index| {
            let segment = &route.segments[index];
            core::geometry::distance(segment.start, segment.end) <= segment.width
                && ends(segment)
                    .into_iter()
                    .any(|(point, far)| !held(route, index, point, far, true))
                && pieces(route, &own, Some(index)) <= pieces(route, &own, None)
        });
        if let Some(index) = nub {
            route.segments.remove(index);
            continue;
        }
        let lonely = route.vias.iter().position(|via| {
            let mut layers: core::LayerMask = 0;
            for segment in &route.segments {
                if core::geometry::point_segment_distance(via.at, segment.start, segment.end)
                    <= (segment.width + via.diameter) / 2.0 + 1.0e-6
                {
                    layers |= 1 << segment.layer;
                }
            }
            for obstacle in &own {
                if obstacle.shape.distance_to_point(via.at) <= via.diameter / 2.0 + 1.0e-6 {
                    layers |= obstacle.layers;
                }
            }
            layers == 0
        });
        match lonely {
            Some(index) => {
                route.vias.remove(index);
            }
            None => break,
        }
    }
}

/// After a stub repaired (`repair_stubs`) from the pad at `pad`: the pad's
/// old tracks (not the stub's, `new`) that now close a loop go, one at a
/// time, each only where the net's copper stays in as many pieces, and
/// then what joins nothing (`prune_dangling`): the pad keeps one way to
/// the run, not also its old route to the link's other end or through
/// other pads.
pub(super) fn cut_old_connection(
    board: &core::Board,
    route: &mut core::NetRoute,
    net: core::NetId,
    pad: Point,
    new: &[core::Segment],
) {
    // The net's kept copper (the links' tracks and vias) as tracks and
    // vias, so that the pieces join along it; its pads and other copper as
    // they are.
    let mut kept = core::NetRoute::default();
    let mut own: Vec<&core::Obstacle> = Vec::new();
    for obstacle in board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.net == Some(net) && obstacle.kind == core::ObstacleKind::Copper)
    {
        match (&obstacle.shape, obstacle.label.as_str()) {
            (&core::Shape::Capsule { start, end, radius }, "existing track")
                if obstacle.layers.count_ones() == 1 =>
            {
                kept.segments.push(core::Segment {
                    layer: obstacle.layers.trailing_zeros() as usize,
                    start,
                    end,
                    width: 2.0 * radius,
                });
            }
            (&core::Shape::Circle { center, radius }, "existing via") => {
                kept.vias.push(core::Via {
                    at: center,
                    diameter: 2.0 * radius,
                    drill: 0.0,
                    span: None,
                });
            }
            _ => own.push(obstacle),
        }
    }
    let at_pad: Vec<&core::Obstacle> = own
        .iter()
        .copied()
        .filter(|obstacle| obstacle.shape.distance_to_point(pad) <= 1.0e-6)
        .collect();
    let joined_with_kept = |route: &core::NetRoute| {
        let mut all = route.clone();
        all.segments.extend(kept.segments.iter().cloned());
        all.vias.extend(kept.vias.iter().cloned());
        all
    };
    let mut cut = false;
    loop {
        let all = joined_with_kept(route);
        let before = pieces(&all, &own, None);
        let loop_at_pad = (0..route.segments.len()).find(|&index| {
            let segment = &route.segments[index];
            !new.contains(segment)
                && at_pad.iter().any(|obstacle| {
                    obstacle.layers & (1 << segment.layer) != 0
                        && [segment.start, segment.end].into_iter().any(|end| {
                            obstacle.shape.distance_to_point(end) <= segment.width / 2.0 + 1.0e-6
                        })
                })
                && pieces(&all, &own, Some(index)) <= before
        });
        match loop_at_pad {
            Some(index) => {
                route.segments.remove(index);
                cut = true;
            }
            None => break,
        }
    }
    if cut {
        prune_dangling(board, route, net);
    }
}

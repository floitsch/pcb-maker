// Copyright (C) 2026 Toit contributors.

//! What the pairs' copper leaves dangling goes: a failed link's stub, any
//! track or via that joins nothing, a repaired stub's old way.

use super::*;
use pcb_router as core;

/// Removes what is left of a failed link's stub on `net`'s route: from
/// the stub's end (its copper `lead`, `reach` around it) along the tracks
/// whose end there touches nothing else of the net (no other track, via,
/// pad or kept copper), up to the first junction (KiCad's
/// `track_dangling`, an error in some projects) or the first track other
/// copper of the net meets along its length.
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
        // A track the net's other copper meets along its length (the
        // fallback's route ending on the stub) stays: removing it would
        // leave the net's copper in more pieces.
        if end_touches(route, &own, index, point, layer)
            || pieces(route, &own, Some(index)) > pieces(route, &own, None)
        {
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
    let skip = without.map(|index| o + index);
    for a in 0..total {
        for b in a + 1..total {
            if Some(a) != skip && Some(b) != skip && copper_touches(route, own, a, b) {
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

fn copper_touches(route: &core::NetRoute, own: &[&core::Obstacle], a: usize, b: usize) -> bool {
    let (a, b) = (a.min(b), a.max(b));
    let (o, n) = (own.len(), route.segments.len());
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
}

/// Cut one redundant ordinary branch of a generated two-terminal pair net.
/// The kept pair copper is protected. Accept the trial only if every
/// remaining conductor and both original pads still form one piece.
pub(super) fn prune_parallel_branch(
    board: &core::Board,
    input_obstacles: usize,
    route: &mut core::NetRoute,
    net: core::NetId,
) -> bool {
    const MAX_ORDINARY_SEGMENTS: usize = 64;
    const MAX_TOTAL_ITEMS: usize = 160;
    let [first, second] = board.nets[net as usize].terminals.as_slice() else {
        return false;
    };
    if first.pad == second.pad
        || route.segments.is_empty()
        || route.segments.len() > MAX_ORDINARY_SEGMENTS
        || route.vias.iter().any(|via| via.span.is_some())
        || board.planes.iter().any(|plane| plane.net == net)
    {
        return false;
    }

    // Exclude nets with any authored copper besides the original two pads.
    // In particular, never prune tracks/vias taken from an input project.
    let original = &board.obstacles[..input_obstacles];
    let own: Vec<_> = original
        .iter()
        .filter(|obstacle| obstacle.net == Some(net) && obstacle.kind == core::ObstacleKind::Copper)
        .collect();
    if own.len() != 2
        || ![first.pad, second.pad].into_iter().all(|pad| {
            original
                .get(pad)
                .is_some_and(|obstacle| own.iter().any(|own| std::ptr::eq(*own, obstacle)))
        })
    {
        return false;
    }

    let Some(pair) = kept_pair_copper(board, input_obstacles, net) else {
        return false;
    };
    let all = |ordinary: &core::NetRoute| {
        let mut joined = ordinary.clone();
        joined.segments.extend(pair.segments.iter().cloned());
        joined.vias.extend(pair.vias.iter().cloned());
        joined
    };
    let joined = all(route);
    let count = |route: &core::NetRoute, without| {
        let n = route.segments.len() + route.vias.len() + own.len();
        tested(n.saturating_mul(n.saturating_sub(1)) / 2);
        pieces(route, &own, without)
    };
    if pair.segments.is_empty()
        || joined.segments.len() + joined.vias.len() > MAX_TOTAL_ITEMS
        || count(&joined, None) != 1
        || pieces(&pair, &[], None) != 1
    {
        return false;
    }
    let mut by_length: Vec<_> = route
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| (index, core::geometry::distance(segment.start, segment.end)))
        .collect();
    by_length.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let Some((index, _)) = by_length
        .into_iter()
        .find(|(index, _)| count(&joined, Some(*index)) == 1)
    else {
        return false;
    };

    let Some(trial) = connected_launches(route, &joined, &own, index) else {
        return false;
    };
    if count(&all(&trial), None) != 1 {
        return false;
    }
    *route = trial;
    true
}

fn kept_pair_copper(
    board: &core::Board,
    input_obstacles: usize,
    net: core::NetId,
) -> Option<core::NetRoute> {
    let mut pair = core::NetRoute::default();
    for obstacle in board.obstacles[input_obstacles..]
        .iter()
        .filter(|obstacle| obstacle.net == Some(net) && obstacle.kind == core::ObstacleKind::Copper)
    {
        match (&obstacle.shape, obstacle.label.as_str()) {
            (&core::Shape::Capsule { start, end, radius }, "existing track")
                if obstacle.layers.count_ones() == 1 =>
            {
                pair.segments.push(core::Segment {
                    layer: obstacle.layers.trailing_zeros() as usize,
                    start,
                    end,
                    width: 2.0 * radius,
                });
            }
            (&core::Shape::Circle { center, radius }, "existing via")
                if obstacle.layers == board.all_layers() =>
            {
                pair.vias.push(core::Via {
                    at: center,
                    diameter: 2.0 * radius,
                    drill: 0.0,
                    span: None,
                });
            }
            _ => return None,
        }
    }
    Some(pair)
}

/// Remove an ordinary leaf whose tip lies inside a parallel generated
/// track of the same net. Fallback can leave that overlap where it rejoins
/// a partly coupled pair, and KiCad can flag the overlapping tip.
pub(super) fn prune_fallback_overlap(
    board: &core::Board,
    input_obstacles: usize,
    route: &mut core::NetRoute,
    net: core::NetId,
) -> bool {
    const MAX_COPPER_ITEMS: usize = 160;
    const CONTACT_SLACK: f64 = 1.0e-6;
    if route.segments.is_empty()
        || route.vias.iter().any(|via| via.span.is_some())
        || board.planes.iter().any(|plane| plane.net == net)
    {
        return false;
    }
    let Some(pair) = kept_pair_copper(board, input_obstacles, net) else {
        return false;
    };
    if pair.segments.is_empty() {
        return false;
    }
    let own: Vec<_> = board.obstacles[..input_obstacles]
        .iter()
        .filter(|obstacle| obstacle.net == Some(net) && obstacle.kind == core::ObstacleKind::Copper)
        .collect();
    let total =
        own.len() + route.segments.len() + route.vias.len() + pair.segments.len() + pair.vias.len();
    if total > MAX_COPPER_ITEMS {
        return false;
    }
    let generated = || route.segments.iter().chain(&pair.segments).enumerate();
    let contacts_tip = |excluded: [usize; 2], track: &core::Segment, point: Point| {
        own.iter().any(|obstacle| {
            obstacle.layers & (1 << track.layer) != 0
                && obstacle.shape.distance_to_point(point) <= track.width / 2.0 + CONTACT_SLACK
        }) || route.vias.iter().chain(&pair.vias).any(|via| {
            norm(sub(via.at, point)) <= (via.diameter + track.width) / 2.0 + CONTACT_SLACK
        }) || generated().any(|(other, segment)| {
            !excluded.contains(&other)
                && segment.layer == track.layer
                && core::geometry::point_segment_distance(point, segment.start, segment.end)
                    <= (segment.width + track.width) / 2.0 + CONTACT_SLACK
        })
    };
    let mut joined = None;
    let mut candidates = route.segments.iter().enumerate();
    let Some((index, _)) = candidates.find(|&(index, track)| {
        tested(route.segments.len() + pair.segments.len());
        let direction = sub(track.end, track.start);
        let cover = |(other, kept): (usize, &core::Segment)| {
            let along = sub(kept.end, kept.start);
            if other == index
                || kept.layer != track.layer
                || norm(along) <= norm(direction) + CONTACT_SLACK
                || kept.width + CONTACT_SLACK < track.width
                || cross(unit(along), unit(direction)).abs() > CONTACT_SLACK
                || [track.start, track.end].into_iter().any(|point| {
                    core::geometry::point_segment_distance(point, kept.start, kept.end)
                        > kept.width / 2.0 + CONTACT_SLACK
                })
            {
                return false;
            }
            tested(2 * total);
            [track.start, track.end]
                .into_iter()
                .any(|point| !contacts_tip([index, other], track, point))
        };
        if !generated().any(cover) {
            return false;
        }
        let (all, count) = joined.get_or_insert_with(|| {
            let mut all = route.clone();
            all.segments.extend_from_slice(&pair.segments);
            all.vias.extend_from_slice(&pair.vias);
            tested(total * (total - 1) / 2);
            let count = pieces(&all, &own, None);
            (all, count)
        });
        tested(total * (total - 1) / 2);
        pieces(all, &own, Some(index)) == *count
    }) else {
        return false;
    };
    route.segments.remove(index);
    true
}

/// Find a separate path from each pad to one end of the preserved run.
/// Unused generated copper can be dropped even if it still touches a live
/// stub at one end. Neither path can reach its launch through the other
/// pad or the middle of the coupled run.
fn connected_launches(
    ordinary: &core::NetRoute,
    joined: &core::NetRoute,
    own: &[&core::Obstacle],
    cut: usize,
) -> Option<core::NetRoute> {
    let count = own.len();
    let ordinary_segments = count..count + ordinary.segments.len();
    let pair_start = ordinary_segments.end;
    let pair_end = count + joined.segments.len() - 1;
    let ordinary_vias = pair_end + 1..pair_end + 1 + ordinary.vias.len();
    let items = pair_end + 1 + joined.vias.len();
    let cut = count + cut;
    let steps = |id: usize| {
        if ordinary_segments.contains(&id) {
            let track = &ordinary.segments[id - count];
            Some(core::geometry::distance(track.start, track.end))
        } else {
            ordinary_vias.contains(&id).then_some(1.0e-6)
        }
    };
    let search = |source: usize, goal: usize| {
        let mut distance = vec![f64::INFINITY; items];
        let mut parent = vec![None; items];
        let mut seen = vec![false; items];
        distance[source] = 0.0;
        loop {
            let current = (0..items)
                .filter(|&id| !seen[id] && distance[id].is_finite())
                .min_by(|&a, &b| distance[a].total_cmp(&distance[b]))?;
            if current == goal {
                let mut path = BTreeSet::new();
                let mut node = current;
                while let Some(previous) = parent[node] {
                    if node != goal {
                        path.insert(node);
                    }
                    node = previous;
                }
                return Some(path);
            }
            seen[current] = true;
            for next in (0..items).filter(|&id| !seen[id] && id != cut) {
                let Some(step) = (if next == goal { Some(0.0) } else { steps(next) }) else {
                    continue;
                };
                tested(1);
                let path = distance[current] + step;
                if path < distance[next] && centreline_contact(joined, own, current, next) {
                    distance[next] = path;
                    parent[next] = Some(current);
                }
            }
        }
    };
    let attachment = |pads: [usize; 2]| {
        let first = search(pads[0], pair_start)?;
        let second = search(pads[1], pair_end)?;
        first
            .is_disjoint(&second)
            .then(|| first.union(&second).copied().collect::<BTreeSet<_>>())
    };
    let kept = attachment([0, 1]).or_else(|| attachment([1, 0]))?;
    Some(core::NetRoute {
        segments: ordinary
            .segments
            .iter()
            .cloned()
            .enumerate()
            .filter_map(|(index, item)| kept.contains(&(count + index)).then_some(item))
            .collect(),
        vias: ordinary
            .vias
            .iter()
            .cloned()
            .enumerate()
            .filter_map(|(index, item)| {
                kept.contains(&(ordinary_vias.start + index))
                    .then_some(item)
            })
            .collect(),
    })
}

/// The launch must still have a centreline connection through each
/// ordinary piece. Copper overlap alone can make a small launch connector
/// look redundant, even though dropping it leaves a seam in the trace.
fn centreline_contact(route: &core::NetRoute, own: &[&core::Obstacle], a: usize, b: usize) -> bool {
    const SLACK_MM: f64 = 1.0e-6;
    let (a, b) = (a.min(b), a.max(b));
    let pads = own.len();
    let track_end = pads + route.segments.len();
    if a < pads {
        return (pads..track_end).contains(&b) && {
            let pad = own[a];
            let track = &route.segments[b - pads];
            pad.layers & (1 << track.layer) != 0
                && pad.shape.distance_to_segment(track.start, track.end) <= SLACK_MM
        };
    }
    if a >= track_end {
        return false;
    }
    let track = &route.segments[a - pads];
    if b >= track_end {
        let via = &route.vias[b - track_end];
        return core::geometry::point_segment_distance(via.at, track.start, track.end)
            <= via.diameter / 2.0 + SLACK_MM;
    }
    let other = &route.segments[b - pads];
    track.layer == other.layer
        && [[track, other], [other, track]].into_iter().any(|[a, b]| {
            [a.start, a.end]
                .into_iter()
                .any(|end| core::geometry::point_segment_distance(end, b.start, b.end) <= SLACK_MM)
        })
}

/// Removes `net`'s tracks with an end that joins nothing else of the net
/// (no track, via, pad or kept copper its copper overlaps there, but the
/// tracks meeting its other end) where that splits no piece of the net's
/// copper, and its vias that join no copper (a via
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
        // Never a track whose removal splits the net's copper (another
        // track ends on it), though one of its ends joins nothing.
        let whole = pieces(route, &own, None);
        let dangling = (0..route.segments.len()).find(|&index| {
            ends(&route.segments[index])
                .into_iter()
                .any(|(point, far)| !held(route, index, point, far, false))
                && pieces(route, &own, Some(index)) <= whole
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
/// other pads. Returns whether a short tip of the replaced launch was
/// also removed.
pub(super) fn cut_old_connection(
    board: &core::Board,
    route: &mut core::NetRoute,
    net: core::NetId,
    pad: Point,
    new: &[core::Segment],
) -> bool {
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
    let mut cut_ends = Vec::new();
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
                let removed = route.segments.remove(index);
                cut_ends.extend([removed.start, removed.end].map(|end| (removed.layer, end)));
            }
            None => break,
        }
    }
    if cut_ends.is_empty() {
        return false;
    }
    prune_dangling(board, route, net);
    let joined = joined_with_kept(route);
    let tip = obsolete_launch_tip(route, &joined, &own, new, &cut_ends);
    if let Some(index) = tip {
        route.segments.remove(index);
    }
    tip.is_some()
}

/// One end of a short segment can keep overlapping the replacement track
/// after the old path is cut, though neither centerline reaches its former
/// joint. Remove only that old tip, with the replacement launch protected.
fn obsolete_launch_tip(
    ordinary: &core::NetRoute,
    joined: &core::NetRoute,
    own: &[&core::Obstacle],
    replacement: &[core::Segment],
    cut_ends: &[(usize, Point)],
) -> Option<usize> {
    let count = own.len() + joined.segments.len() + joined.vias.len();
    if count > 160 {
        return None;
    }
    let near = |a, b| core::geometry::distance(a, b) <= EPSILON;
    let trial_pieces = |without| {
        tested(count.saturating_mul(count.saturating_sub(1)) / 2);
        pieces(joined, own, without)
    };
    ordinary
        .segments
        .iter()
        .enumerate()
        .find_map(|(index, track)| {
            if replacement.contains(track)
                || core::geometry::distance(track.start, track.end) > 2.0 * track.width + EPSILON
                || own.iter().any(|pad| {
                    pad.layers & (1 << track.layer) != 0
                        && pad.shape.distance_to_segment(track.start, track.end)
                            <= track.width / 2.0 + EPSILON
                })
                || joined.vias.iter().any(|via| {
                    core::geometry::point_segment_distance(via.at, track.start, track.end)
                        <= (track.width + via.diameter) / 2.0 + EPSILON
                })
            {
                return None;
            }
            let endpoint_touches = |point, center_only| {
                joined.segments.iter().enumerate().any(|(other, segment)| {
                    other != index
                        && segment.layer == track.layer
                        && core::geometry::point_segment_distance(point, segment.start, segment.end)
                            <= segment.width / 2.0
                                + if center_only { 0.0 } else { track.width / 2.0 }
                                + EPSILON
                })
            };
            let is_tip = [(track.start, track.end), (track.end, track.start)]
                .into_iter()
                .any(|(end, other)| {
                    cut_ends
                        .iter()
                        .any(|&(layer, cut)| layer == track.layer && near(end, cut))
                        && !endpoint_touches(end, true)
                        && endpoint_touches(other, false)
                });
            (is_tip && trial_pieces(Some(index)) == trial_pieces(None)).then_some(index)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_cut_tip_is_removed_only_when_its_replacement_holds() {
        let segment = |start, end| core::Segment {
            layer: 0,
            start,
            end,
            width: 0.1,
        };
        let tip = segment([3.0, 2.04], [3.19, 2.09]);
        let live = segment([2.0, 2.0], [5.0, 2.0]);
        let route = core::NetRoute {
            segments: vec![tip.clone(), live.clone()],
            ..Default::default()
        };
        let cuts = [(0, tip.end)];
        assert_eq!(
            obsolete_launch_tip(&route, &route, &[], std::slice::from_ref(&live), &cuts),
            Some(0)
        );
        assert_eq!(
            obsolete_launch_tip(&route, &route, &[], &[tip.clone(), live.clone()], &cuts),
            None
        );
        assert_eq!(
            obsolete_launch_tip(&route, &route, &[], std::slice::from_ref(&live), &[]),
            None
        );
        let disconnected_trial = core::NetRoute {
            segments: vec![
                tip.clone(),
                live.clone(),
                segment([3.19, 2.17], [4.0, 2.17]),
            ],
            ..Default::default()
        };
        assert_eq!(
            obsolete_launch_tip(
                &disconnected_trial,
                &disconnected_trial,
                &[],
                &[live],
                &cuts
            ),
            None
        );
    }
}

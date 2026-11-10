// Copyright (C) 2026 Toit contributors.

//! A kept link's stubs after the route: the stub's path, whether it joins
//! the run, the stubs straightened and joined to the run's ends.

use super::*;
use pcb_router as core;

/// The segments of a route's stub from `pad` to `stub_end` (its track
/// touches the end's copper, a track of `width` along the lead: the
/// route may end anywhere on it, `Board::terminal_copper`): one layer, no
/// branch, no via; and the layer.
pub(super) fn stub_path(
    route: &core::NetRoute,
    pad: Point,
    (stub_end, lead_end): (Point, Point),
    width: f64,
) -> Option<(usize, Vec<usize>)> {
    let key = |point: Point| {
        (
            (point[0] * 1.0e6).round() as i64,
            (point[1] * 1.0e6).round() as i64,
        )
    };
    let path = link_path(route, pad, stub_end)?;
    if debug() {
        eprintln!("diff pair stub path {path:?}");
    }
    let (first, last) = (path[0], path[path.len() - 1]);
    let reach = (width
        + route
            .segments
            .iter()
            .map(|segment| segment.width)
            .fold(0.0, f64::max))
        / 2.0;
    if core::geometry::distance(first.0, pad) > 1.0e-6
        || core::geometry::point_segment_distance(last.0, stub_end, lead_end) > reach + 1.0e-6
        || path.iter().any(|(_, layer)| *layer != first.1)
        || path.len() < 2
    {
        return None;
    }
    let mut degree: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    for segment in route
        .segments
        .iter()
        .filter(|segment| segment.layer == first.1)
    {
        *degree.entry(key(segment.start)).or_default() += 1;
        *degree.entry(key(segment.end)).or_default() += 1;
    }
    let vias: BTreeSet<(i64, i64)> = route.vias.iter().map(|via| key(via.at)).collect();
    let ends_free = degree.get(&key(last.0)) == Some(&1) && !vias.contains(&key(last.0));
    if !ends_free
        || path[1..path.len() - 1]
            .iter()
            .any(|(point, _)| degree.get(&key(*point)) != Some(&2) || vias.contains(&key(*point)))
    {
        return None;
    }
    let on_path: BTreeSet<((i64, i64), (i64, i64))> = path
        .windows(2)
        .flat_map(|step| {
            let (a, b) = (key(step[0].0), key(step[1].0));
            [(a, b), (b, a)]
        })
        .collect();
    let stub: Vec<usize> = route
        .segments
        .iter()
        .enumerate()
        .filter(|(_, segment)| {
            segment.layer == first.1 && on_path.contains(&(key(segment.start), key(segment.end)))
        })
        .map(|(index, _)| index)
        .collect();
    (stub.len() + 1 == path.len()).then_some((first.1, stub))
}

/// Whether `net`'s route joins the pad at `pad` (its copper on the board)
/// to a stub's end (its copper `lead`, `reach` around it): the copper the
/// route's tracks and vias overlap, followed from the pad (as the router
/// joins copper), and through the net's other pads and kept copper on the
/// way (a stub that passes an ESD diode's pad or a series part's pad on
/// its way to the run: right_io's ETH_MDI pairs through U501 and U502),
/// but not through the pad at `avoid` (the link's other end: a route from
/// the pad to the other end's pad and on to this end's stub would make
/// the run a loop beside it). A link whose stubs do not reach both its
/// ends would leave its run an island (the router may end a pad's route
/// on the other end of the link, merged into one terminal, or leave it
/// open).
pub(super) fn stub_joined(
    board: &core::Board,
    route: &core::NetRoute,
    net: core::NetId,
    (pad, avoid): (Point, Point),
    lead: (Point, Point),
    reach: f64,
) -> bool {
    let own: Vec<&core::Obstacle> = board
        .obstacles
        .iter()
        .filter(|obstacle| {
            obstacle.net == Some(net)
                && obstacle.kind == core::ObstacleKind::Copper
                && obstacle.shape.distance_to_point(avoid) > 1.0e-6
        })
        .collect();
    let starts: Vec<bool> = own
        .iter()
        .map(|obstacle| obstacle.shape.distance_to_point(pad) <= 1.0e-6)
        .collect();
    let segments = &route.segments;
    let (count, vias) = (segments.len(), route.vias.len());
    // Items: the segments, the vias, then the net's own copper.
    let touches_copper = |item: usize, obstacle: &core::Obstacle| {
        if item < count {
            let segment = &segments[item];
            obstacle.layers & (1 << segment.layer) != 0
                && [segment.start, segment.end].into_iter().any(|end| {
                    obstacle.shape.distance_to_point(end) <= segment.width / 2.0 + 1.0e-6
                })
        } else {
            let via = &route.vias[item - count];
            obstacle.shape.distance_to_point(via.at) <= via.diameter / 2.0 + 1.0e-6
        }
    };
    let joined = |a: usize, b: usize| {
        let (a, b) = (a.min(b), a.max(b));
        if b >= count + vias {
            return a < count + vias && touches_copper(a, own[b - count - vias]);
        }
        match (a < count, b < count) {
            (true, true) => {
                let (x, y) = (&segments[a], &segments[b]);
                let keep = (x.width + y.width) / 2.0 + 1.0e-6;
                x.layer == y.layer
                    && ([x.start, x.end].into_iter().any(|end| {
                        core::geometry::point_segment_distance(end, y.start, y.end) <= keep
                    }) || [y.start, y.end].into_iter().any(|end| {
                        core::geometry::point_segment_distance(end, x.start, x.end) <= keep
                    }))
            }
            (true, false) => {
                let (segment, via) = (&segments[a], &route.vias[b - count]);
                core::geometry::point_segment_distance(via.at, segment.start, segment.end)
                    <= (segment.width + via.diameter) / 2.0 + 1.0e-6
            }
            _ => false,
        }
    };
    let at_end = |item: usize| {
        item < count
            && [segments[item].start, segments[item].end]
                .into_iter()
                .any(|end| {
                    core::geometry::point_segment_distance(end, lead.0, lead.1) <= reach + 1.0e-6
                })
    };
    let total = count + vias + own.len();
    let mut seen = vec![false; total];
    let mut queue: Vec<usize> = (0..own.len())
        .filter(|&index| starts[index])
        .map(|index| count + vias + index)
        .collect();
    for &item in &queue {
        seen[item] = true;
    }
    while let Some(item) = queue.pop() {
        if at_end(item) {
            return true;
        }
        for (other, known) in seen.iter_mut().enumerate() {
            if !*known && joined(item, other) {
                *known = true;
                queue.push(other);
            }
        }
    }
    if debug() {
        let nearest = segments
            .iter()
            .flat_map(|segment| [segment.start, segment.end])
            .map(|end| core::geometry::point_segment_distance(end, lead.0, lead.1))
            .fold(f64::INFINITY, f64::min);
        eprintln!(
            "diff pair stub of {} from {pad:?} does not reach {lead:?} (from the pad {} of {} items, nearest track end {nearest:.3} mm, reach {reach:.3})",
            board.nets[net as usize].name,
            seen.iter().filter(|seen| **seen).count(),
            total
        );
    }
    false
}

/// Whether the pads' order at `end` of a link crosses the stubs' ends'
/// (the stubs must cross there).
pub(super) fn end_crossed(link: &LinkPlan, end: usize) -> bool {
    let outward = sub(link.launches[end], link.starts[end]);
    let pads = cross(outward, sub(link.pads[end][0], link.pads[end][1]));
    let ends = cross(outward, sub(link.ends[end][0], link.ends[end][1]));
    pads * ends < 0.0
}

/// A kept link's stub that ends elsewhere on its end's copper (the router
/// may end it anywhere there) gets a track to the end itself, where the
/// run's jog starts: else the jog's end touches only the stub's copper,
/// not its end, and KiCad finds it dangling.
pub(super) fn join_stub_ends(
    result: &mut core::RoutingResult,
    pairs: &[Pair],
    plan_link: &LinkPlan,
) {
    let pair = &pairs[plan_link.pair];
    for end in (0..2).filter(|&end| !plan_link.fixed[end]) {
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            let (at, lead_end) = plan_link.stub_end(end, polarity, pair.width);
            let route = &mut result.routes[net as usize];
            let reach = (pair.width
                + route
                    .segments
                    .iter()
                    .map(|segment| segment.width)
                    .fold(0.0, f64::max))
                / 2.0;
            let ends: Vec<(f64, Point, usize, f64)> = route
                .segments
                .iter()
                .flat_map(|segment| {
                    [segment.start, segment.end].map(|point| (point, segment.layer, segment.width))
                })
                .filter(|(point, _, _)| {
                    core::geometry::point_segment_distance(*point, at, lead_end) <= reach + 1.0e-6
                })
                .map(|(point, layer, width)| {
                    (core::geometry::distance(point, at), point, layer, width)
                })
                .collect();
            if ends.iter().any(|(distance, ..)| *distance < 1.0e-6) {
                continue;
            }
            if let Some(&(_, point, layer, width)) = ends.iter().min_by(|a, b| a.0.total_cmp(&b.0))
            {
                route.segments.push(core::Segment {
                    layer,
                    start: point,
                    end: at,
                    width: width.min(pair.width),
                });
            }
        }
    }
}

/// Replaces each stub of a kept link (the pair's net's route from a pad to
/// its stub's end, one layer, no branch) by a straight track where that
/// keeps every clearance (`refused_groups`).
pub(super) fn straighten_stubs(
    board: &core::Board,
    result: &mut core::RoutingResult,
    pairs: &[Pair],
    plan_link: &LinkPlan,
) {
    let pair = &pairs[plan_link.pair];
    for end in (0..2).filter(|&end| !plan_link.fixed[end]) {
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            let (pad, stub_end) = (plan_link.pads[end][polarity], plan_link.ends[end][polarity]);
            let Some((layer, stub)) = stub_path(
                &result.routes[net as usize],
                pad,
                plan_link.stub_end(end, polarity, pair.width),
                pair.width,
            ) else {
                continue;
            };
            let route = &result.routes[net as usize];
            let first = (pad, layer);
            // The routed stub's narrowest width, at most the pair's (the
            // stubs' ends are placed for it).
            let width = stub
                .iter()
                .map(|&index| route.segments[index].width)
                .fold(pair.width, f64::min);
            // The stub out of the routes while the straight one is checked.
            let segments = &mut result.routes[net as usize].segments;
            let mut removed = Vec::new();
            let mut index = 0;
            segments.retain(|segment| {
                index += 1;
                let keep = !stub.contains(&(index - 1));
                if !keep {
                    removed.push(segment.clone());
                }
                keep
            });
            let mut straight: [Vec<(usize, Point, Point)>; 2] = Default::default();
            straight[polarity].push((first.1, pad, stub_end));
            let refused = refused_groups_at(
                board,
                &result.routes,
                pairs,
                &[Group {
                    pair: plan_link.pair,
                    segments: straight,
                    vias: Default::default(),
                    run: Default::default(),
                    moved: false,
                }],
                width,
            );
            if debug() {
                eprintln!(
                    "diff pair stub {pad:?} -> {stub_end:?} of {}: {:?}",
                    board.nets[net as usize].name, refused[0]
                );
            }
            let segments = &mut result.routes[net as usize].segments;
            if refused[0].is_none() {
                segments.push(core::Segment {
                    layer: first.1,
                    start: pad,
                    end: stub_end,
                    width,
                });
            } else {
                segments.extend(removed);
            }
        }
    }
}

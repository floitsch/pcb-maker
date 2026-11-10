// Copyright (C) 2026 Toit contributors.

//! The diagnostics of `PCB_ROUTER_PAIRS_WHY` and `PCB_ROUTER_PAIRS_REACH`:
//! trial routes of a stub with what might block it taken away, and the
//! geometry of a stub that does not reach its run.

use super::*;
use pcb_router as core;

/// What a diagnosis trial takes off the board (`trial_reaches`): by the
/// obstacle's index and the obstacle.
pub(super) type Gone<'a> = dyn Fn(usize, &core::Obstacle) -> bool + 'a;

/// A trial for the diagnosis (`PCB_ROUTER_PAIRS_WHY`): whether `net`
/// routes from `terminal` to `target` alone on `board` (other nets with no
/// terminals, no pours), in a window around the two where it lies inside
/// the board, with the obstacles `gone` takes moved off the board.
pub(super) fn trial_reaches(
    board: &core::Board,
    net: core::NetId,
    (terminal, target): (&core::Terminal, &core::Terminal),
    gone: &Gone<'_>,
    config: &KiCadBoardRouterConfig,
) -> bool {
    let mut trial = board.clone();
    trial.planes.clear();
    for other in &mut trial.nets {
        other.terminals.clear();
    }
    trial.merged_pads.retain(|merged| merged.net != net);
    trial.terminal_copper.retain(|extra| extra.net != net);
    trial.nets[net as usize].terminals = vec![terminal.clone(), target.clone()];
    trial.attachment = core::Attachment::default();
    let (pad, to) = (terminal.anchor, target.anchor);
    let margin = 3.0;
    let (low, high) = (
        [pad[0].min(to[0]) - margin, pad[1].min(to[1]) - margin],
        [pad[0].max(to[0]) + margin, pad[1].max(to[1]) + margin],
    );
    let window = vec![low, [high[0], low[1]], high, [low[0], high[1]]];
    if window
        .iter()
        .all(|&corner| core::geometry::point_in_polygon(corner, &board.outline))
    {
        trial.outline = window;
    }
    for (index, obstacle) in trial.obstacles.iter_mut().enumerate() {
        if obstacle.net != Some(net) && gone(index, obstacle) {
            obstacle.shape = core::Shape::Circle {
                center: [1.0e6, 1.0e6],
                radius: 0.01,
            };
        }
    }
    let mut router_config = core_config(config);
    router_config.verbose = false;
    router_config.deadline_work = Some(2.0);
    router_config.run_work = None;
    core::route(&trial, &router_config).status[net as usize] == core::NetStatus::Routed
}

/// `PCB_ROUTER_PAIRS_WHY`: what keeps the stub of `polarity` at `end` of
/// a link from its pad to its stub's end in the attempt's state: the pad
/// routed to the end alone on the planned board with the attempt's routes
/// (`routes`: the board's nets, then the links'), and again with what might
/// block it taken away in turn: the other nets' routes, the link's own
/// route, the partner's routes, the plan's copper (the leads, the stubs'
/// ends, the carried stubs). The first that frees it is named.
pub(super) fn stub_blocked(
    board: &core::Board,
    (routes, link_routes): (&[core::NetRoute], &[core::NetRoute]),
    planned_from: usize,
    link: &LinkPlan,
    (end, polarity): (usize, usize),
    nets: [core::NetId; 2],
    config: &KiCadBoardRouterConfig,
) -> String {
    let (net, partner) = (nets[polarity], nets[1 - polarity]);
    let pad = link.pads[end][polarity];
    let Some(terminal) = board.nets[net as usize]
        .terminals
        .iter()
        .find(|terminal| core::geometry::distance(terminal.anchor, pad) < 1.0e-6)
        .cloned()
    else {
        return "the pad is no terminal".into();
    };
    let target = link.runs[end][polarity].clone();
    let mut all: Vec<core::NetRoute> = routes.to_vec();
    all.extend(link_routes.iter().cloned());
    let base_count = board.obstacles.len();
    let trial = with_routes(board, &all);
    let kind = |index: usize, obstacle: &core::Obstacle| -> usize {
        let route = index >= base_count;
        if route && obstacle.net == Some(link.net) {
            2
        } else if route && obstacle.net == Some(partner) {
            3
        } else if route {
            1
        } else if index >= planned_from {
            4
        } else {
            0
        }
    };
    let names = [
        "routes with the attempt's copper",
        "freed without the other nets' routes",
        "freed without the link's route",
        "freed without the partner's routes",
        "freed without the plan's copper",
    ];
    for (step, name) in names.iter().enumerate() {
        let gone = |index: usize, obstacle: &core::Obstacle| {
            let k = kind(index, obstacle);
            k != 0 && k <= step
        };
        if trial_reaches(&trial, net, (&terminal, &target), &gone, config) {
            return (*name).to_string();
        }
    }
    "not even on the bare board".into()
}

/// `PCB_ROUTER_PAIRS_REACH`: per stub the router is to route (a link end
/// whose stubs the link does not carry), whether its pad reaches its stub's
/// end on the planned board alone (no route yet: the pads, the keepouts and
/// the plan's copper), and where it does not, what frees it: the link's own
/// lead, its partner's copper of the link (the stub's end, a crossover),
/// the other links' plan copper; else nothing does.
pub(super) fn plan_reach_check(
    board: &core::Board,
    plan: &PairPlan,
    config: &KiCadBoardRouterConfig,
) {
    for link in &plan.links {
        let pair = &plan.pairs[link.pair];
        let nets = [pair.positive, pair.negative];
        for end in (0..2).filter(|&end| !link.fixed[end]) {
            for polarity in 0..2 {
                let (net, partner) = (nets[polarity], nets[1 - polarity]);
                let pad = link.pads[end][polarity];
                let terminal = board.nets[net as usize]
                    .terminals
                    .iter()
                    .find(|terminal| core::geometry::distance(terminal.anchor, pad) < 1.0e-6)
                    .cloned();
                let Some(terminal) = terminal else {
                    continue;
                };
                let target = &link.runs[end][polarity];
                let planned = |index: usize| index >= plan.obstacles;
                let steps: [(&str, &Gone<'_>); 5] = [
                    ("reaches", &|_, _| false),
                    ("freed without the link's lead", &|index, obstacle| {
                        planned(index) && obstacle.net == Some(link.net)
                    }),
                    (
                        "freed without the partner's plan copper",
                        &|index, obstacle| {
                            planned(index)
                                && (obstacle.net == Some(link.net) || obstacle.net == Some(partner))
                        },
                    ),
                    ("freed without the other links' plan copper", &|index, _| {
                        planned(index)
                    }),
                    (
                        "freed without the board's other pads",
                        &|index, obstacle| planned(index) || obstacle.net != Some(partner),
                    ),
                ];
                let verdict = steps
                    .iter()
                    .find(|(_, gone)| trial_reaches(board, net, (&terminal, target), *gone, config))
                    .map_or("not even alone with its partner", |(name, _)| *name);
                // The plan's own check (`stub_reaches`) on the planned board.
                let near = obstacles_near(
                    board,
                    pad,
                    core::geometry::distance(pad, target.anchor) + REACH_MARGIN + 1.0,
                );
                let grid = stub_reaches(
                    board,
                    &near,
                    pair,
                    (&terminal, polarity),
                    target.anchor,
                    target.layers,
                    &[],
                );
                let start = link.starts[end];
                let outward = unit(sub(link.launches[end], start));
                // What lies near the straight line from the pad to the end.
                let mut near: Vec<String> = Vec::new();
                if !verdict.starts_with("reaches") {
                    let layer = target.layers;
                    for obstacle in &board.obstacles {
                        if obstacle.net == Some(net) || obstacle.layers & layer == 0 {
                            continue;
                        }
                        let gap = obstacle
                            .shape
                            .distance_to_segment(link.stubs[end][polarity], target.anchor)
                            - pair.width / 2.0;
                        if gap < pair.clearance + 0.05 {
                            near.push(format!(
                                "{} [{}] {:.3}",
                                obstacle.label,
                                obstacle
                                    .net
                                    .map_or("-".to_string(), |owner| if owner == partner {
                                        "partner".to_string()
                                    } else if owner == link.net {
                                        "link".to_string()
                                    } else {
                                        board
                                            .nets
                                            .get(owner as usize)
                                            .map_or("?".into(), |n| n.name.clone())
                                    }),
                                gap
                            ));
                        }
                    }
                }
                let local = |point: Point| {
                    let v = sub(point, start);
                    format!("({:.2}, {:.2})", dot(v, outward), cross(outward, v))
                };
                eprintln!(
                    "pairs reach: {} link {} end {} ({} end, {:.3} mm): {verdict}; grid {}; along/across the run from its start: pad {}, partner's pad {}, stub end {}, partner's end {}; near the straight stub: {near:?}",
                    board.nets[net as usize].name,
                    link.link + 1,
                    ["a", "b"][end],
                    if end_crossed(link, end) {
                        "crossed"
                    } else {
                        "straight"
                    },
                    core::geometry::distance(pad, target.anchor),
                    if grid { "reaches" } else { "does not" },
                    local(pad),
                    local(link.pads[end][1 - polarity]),
                    local(target.anchor),
                    local(link.ends[end][1 - polarity]),
                );
            }
        }
    }
}

/// `PCB_ROUTER_PAIRS_WHY`: why the stub of `net` (`polarity`) at `end` of
/// a link does not reach its end of the run (`None`: it does): the pad,
/// the stub's end, the run's start and direction, and what the copper the
/// pad's route belongs to reaches (the pads of the net joined through, the
/// link's other end), and what the stub's end is joined to.
pub(super) fn stub_why(
    board: &core::Board,
    result: &core::RoutingResult,
    plan_link: &LinkPlan,
    end: usize,
    polarity: usize,
    net: core::NetId,
    (width, reach): (f64, f64),
) -> Option<String> {
    let route = &result.routes[net as usize];
    let pad = plan_link.pads[end][polarity];
    let lead = plan_link.stub_end(end, polarity, width);
    let other_lead = plan_link.stub_end(1 - end, polarity, width);
    if stub_joined(
        board,
        route,
        net,
        (pad, plan_link.pads[1 - end][polarity]),
        lead,
        reach,
    ) {
        return None;
    }
    let pads: Vec<&core::Obstacle> = board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.net == Some(net) && obstacle.kind == core::ObstacleKind::Copper)
        .collect();
    let (segments, vias) = (&route.segments, &route.vias);
    let (count, total) = (segments.len(), segments.len() + vias.len() + pads.len());
    let touches = |a: usize, b: usize| -> bool {
        let (a, b) = (a.min(b), a.max(b));
        let segment = |index: usize| &segments[index];
        if b < count {
            let (x, y) = (segment(a), segment(b));
            let keep = (x.width + y.width) / 2.0 + 1.0e-6;
            x.layer == y.layer
                && core::geometry::segment_segment_distance(x.start, x.end, y.start, y.end) <= keep
        } else if b < count + vias.len() {
            let via = &vias[b - count];
            if a < count {
                let x = segment(a);
                core::geometry::point_segment_distance(via.at, x.start, x.end)
                    <= (x.width + via.diameter) / 2.0 + 1.0e-6
            } else {
                core::geometry::distance(via.at, vias[a - count].at) < 1.0e-6
            }
        } else {
            let obstacle = pads[b - count - vias.len()];
            if a < count {
                let x = segment(a);
                obstacle.layers & (1 << x.layer) != 0
                    && obstacle.shape.distance_to_segment(x.start, x.end) <= x.width / 2.0 + 1.0e-6
            } else if a < count + vias.len() {
                let via = &vias[a - count];
                obstacle.shape.distance_to_point(via.at) <= via.diameter / 2.0 + 1.0e-6
            } else {
                false
            }
        }
    };
    let mut parent: Vec<usize> = (0..total).collect();
    for a in 0..total {
        for b in a + 1..total {
            if touches(a, b) {
                let (x, y) = (root(&mut parent, a), root(&mut parent, b));
                if x != y {
                    parent[x.max(y)] = x.min(y);
                }
            }
        }
    }
    let at = |item: usize, lead: (Point, Point)| {
        item < count
            && [segments[item].start, segments[item].end]
                .into_iter()
                .any(|point| {
                    core::geometry::point_segment_distance(point, lead.0, lead.1) <= reach + 1.0e-6
                })
    };
    let pad_items: Vec<usize> = (0..pads.len())
        .filter(|&index| pads[index].shape.distance_to_point(pad) <= 1.0e-6)
        .map(|index| count + vias.len() + index)
        .collect();
    let component = |item: usize, parent: &mut Vec<usize>| {
        let home = root(parent, item);
        (0..total)
            .filter(|&other| root(parent, other) == home)
            .collect::<Vec<usize>>()
    };
    let describe = |items: &[usize]| {
        let mut labels: Vec<String> = items
            .iter()
            .filter(|&&item| item >= count + vias.len())
            .map(|&item| pads[item - count - vias.len()].label.clone())
            .collect();
        labels.dedup();
        let tracks = items.iter().filter(|&&item| item < count).count();
        let via_count = items
            .iter()
            .filter(|&&item| item >= count && item < count + vias.len())
            .count();
        let own_end = items.iter().any(|&item| at(item, lead));
        let other_end = items.iter().any(|&item| at(item, other_lead));
        format!(
            "{tracks} tracks, {via_count} vias, pads [{}]{}{}",
            labels.join(" "),
            if own_end { ", its end" } else { "" },
            if other_end {
                ", the link's other end"
            } else {
                ""
            }
        )
    };
    let from_pad = pad_items
        .first()
        .map(|&item| component(item, &mut parent))
        .unwrap_or_default();
    let at_end: Vec<usize> = (0..count).filter(|&item| at(item, lead)).collect();
    let end_side = at_end
        .first()
        .map(|&item| describe(&component(item, &mut parent)))
        .unwrap_or_else(|| "no track".into());
    let nearest = segments
        .iter()
        .flat_map(|segment| [segment.start, segment.end])
        .map(|point| core::geometry::point_segment_distance(point, lead.0, lead.1))
        .fold(f64::INFINITY, f64::min);
    let start = plan_link.starts[end];
    let outward = unit(sub(plan_link.launches[end], start));
    let towards = sub(lead.0, pad);
    Some(format!(
        "status {:?}; pad ({:.3}, {:.3}) -> stub end ({:.3}, {:.3}) {:.3} mm, {:.3} along the run's direction ({:.3}, {:.3}) and {:.3} across; run start ({:.3}, {:.3}); the pad's copper: {}; the end's: {}; nearest track end {:.3} mm (reach {:.3})",
        result.status[net as usize],
        pad[0],
        pad[1],
        lead.0[0],
        lead.0[1],
        norm(towards),
        dot(towards, outward),
        outward[0],
        outward[1],
        cross(outward, towards),
        start[0],
        start[1],
        if pad_items.is_empty() {
            "no pad found".into()
        } else {
            describe(&from_pad)
        },
        end_side,
        nearest,
        reach
    ))
}

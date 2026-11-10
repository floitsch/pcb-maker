// Copyright (C) 2026 Toit contributors.

//! The stubs a kept link lacks, routed after the attempt.

use super::*;
use pcb_router as core;

/// A stub routed after the attempt (`repair_stubs`): its net, its pad and
/// its tracks.
pub(super) type RoutedStub = (core::NetId, Point, Vec<core::Segment>);

/// The stubs a link lacks (`lacking`: per candidate, (end, polarity)),
/// for each candidate whose copper holds (`refused` none): each routed
/// from its pad to its stub's end on a board where every route and every
/// such candidate's copper is kept, the pair's net with those two
/// terminals only (its other terminals and its merged pads out: a route
/// from the pad that ends at the link's other end, or at another pad, is
/// what left the stub out). A net lacking two stubs routes them in two
/// rounds. Returns per candidate whose every stub routed and now reaches
/// its end (`stub_joined`): the stubs added (net, pad, segments), already
/// in `result`; the rest's added copper is taken out again. And the work.
pub(super) fn repair_stubs(
    board: &core::Board,
    result: &mut core::RoutingResult,
    pairs: &[Pair],
    (candidates, links): (&[(usize, usize, LinkCopper)], &[&LinkPlan]),
    lacking: &[Vec<(usize, usize)>],
    refused: &[Option<String>],
    config: &KiCadBoardRouterConfig,
) -> (BTreeMap<usize, Vec<RoutedStub>>, f64) {
    let wanted: Vec<usize> = (0..candidates.len())
        .filter(|&position| refused[position].is_none() && !lacking[position].is_empty())
        .collect();
    let mut base = with_routes(board, &result.routes);
    // Every candidate's copper whose check held, as copper of its nets.
    for (position, (index, _, copper)) in candidates.iter().enumerate() {
        if refused[position].is_some() {
            continue;
        }
        let pair = &pairs[*index];
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            for &(layer, start, end) in &copper.segments[polarity] {
                base.obstacles.push(core::Obstacle {
                    shape: core::Shape::Capsule {
                        start,
                        end,
                        radius: pair.width / 2.0,
                    },
                    layers: 1 << layer,
                    kind: core::ObstacleKind::Copper,
                    net: Some(net),
                    clearance: 0.0,
                    clearance_override: None,
                    plated: false,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: "existing track".into(),
                });
            }
            for &at in &copper.vias[polarity] {
                for (kind, radius, label) in [
                    (
                        core::ObstacleKind::Copper,
                        pair.via_diameter / 2.0,
                        "existing via",
                    ),
                    (
                        core::ObstacleKind::Hole,
                        pair.via_drill / 2.0,
                        "existing via hole",
                    ),
                ] {
                    base.obstacles.push(core::Obstacle {
                        shape: core::Shape::Circle { center: at, radius },
                        layers: board.all_layers(),
                        kind,
                        net: Some(net),
                        clearance: 0.0,
                        clearance_override: None,
                        plated: false,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: label.into(),
                    });
                }
            }
        }
    }
    let mut todo: Vec<(usize, usize, usize)> = wanted
        .iter()
        .flat_map(|&position| {
            lacking[position]
                .iter()
                .map(move |&(end, polarity)| (position, end, polarity))
        })
        .collect();
    let mut failed: BTreeSet<usize> = BTreeSet::new();
    let mut added: BTreeMap<usize, Vec<(RoutedStub, Vec<core::Via>)>> = BTreeMap::new();
    let mut work = 0.0;
    let budget = work_budget(config.ladder_budget_seconds.unwrap_or(1200.0));
    let router_config = {
        let mut router_config = core_config(config);
        router_config.verbose = false;
        router_config.deadline_work = Some(budget);
        router_config.run_work = None;
        router_config
    };
    while !todo.is_empty() {
        // One stub per net a round.
        let mut round: BTreeMap<core::NetId, (usize, usize, usize)> = BTreeMap::new();
        let mut later = Vec::new();
        for item in todo {
            let (position, _, polarity) = item;
            if failed.contains(&position) {
                continue;
            }
            let pair = &pairs[candidates[position].0];
            let net = [pair.positive, pair.negative][polarity];
            if let std::collections::btree_map::Entry::Vacant(entry) = round.entry(net) {
                entry.insert(item);
            } else {
                later.push(item);
            }
        }
        todo = later;
        if round.is_empty() {
            break;
        }
        let mut sub = base.clone();
        for net in &mut sub.nets {
            net.terminals.clear();
        }
        let nets: BTreeSet<core::NetId> = round.keys().copied().collect();
        sub.merged_pads.retain(|merged| !nets.contains(&merged.net));
        sub.terminal_copper
            .retain(|extra| !nets.contains(&extra.net));
        for (&net, &(position, end, polarity)) in &round {
            let pair = &pairs[candidates[position].0];
            let link = links[position];
            let pad = link.pads[end][polarity];
            // The pad's terminal, or the pad merged into another (its own
            // terminal here).
            let terminal = board.nets[net as usize]
                .terminals
                .iter()
                .find(|terminal| core::geometry::distance(terminal.anchor, pad) < 1.0e-6)
                .cloned()
                .or_else(|| {
                    board
                        .merged_pads
                        .iter()
                        .filter(|merged| merged.net == net)
                        .find(|merged| {
                            board.obstacles[merged.pad].shape.distance_to_point(pad) <= 1.0e-6
                        })
                        .map(|merged| core::Terminal {
                            anchor: pad,
                            layers: board.obstacles[merged.pad].layers,
                            pad: merged.pad,
                            contact: None,
                            label: board.obstacles[merged.pad].label.clone(),
                        })
                });
            let Some(terminal) = terminal else {
                failed.insert(position);
                continue;
            };
            let (at, along) = link.stub_end(end, polarity, pair.width);
            let layers = link.runs[end][polarity].layers;
            sub.obstacles.push(core::Obstacle {
                shape: core::Shape::Capsule {
                    start: at,
                    end: along,
                    radius: pair.width / 2.0,
                },
                layers,
                kind: core::ObstacleKind::Copper,
                net: Some(net),
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: "pair stub end".into(),
            });
            sub.nets[net as usize].terminals = vec![
                terminal,
                core::Terminal {
                    anchor: at,
                    layers,
                    pad: sub.obstacles.len() - 1,
                    contact: Some(core::Shape::Circle {
                        center: at,
                        radius: LAUNCH_CONTACT,
                    }),
                    label: "pair stub end".into(),
                },
            ];
        }
        // The other nets' pours stay as they are: a stub that cuts one
        // pays for it (`Config::pour_integrity`), as in the attempt (the
        // atmega32u4 board's GND pour on B.Cu split by a stub crossing
        // under its run, pairs5).
        let routed = core::route(&sub, &router_config);
        work += routed.expansions as f64 / core::router::expansions_per_second()
            + routed.build_work
            + routed.pour_work;
        for (&net, &(position, _, _)) in &round {
            if failed.contains(&position) {
                continue;
            }
            if routed.status[net as usize] != core::NetStatus::Routed {
                if debug() {
                    eprintln!(
                        "diff pair stub repair of {}: {:?}, terminals {:?}",
                        board.nets[net as usize].name,
                        routed.status[net as usize],
                        sub.nets[net as usize]
                            .terminals
                            .iter()
                            .map(|terminal| (terminal.anchor, terminal.layers, &terminal.label))
                            .collect::<Vec<_>>()
                    );
                }
                failed.insert(position);
                continue;
            }
            let (end, polarity) = (round[&net].1, round[&net].2);
            let from = &routed.routes[net as usize];
            added.entry(position).or_default().push((
                (
                    net,
                    links[position].pads[end][polarity],
                    from.segments.clone(),
                ),
                from.vias.clone(),
            ));
            // The next round keeps it.
            if todo.is_empty() {
                continue;
            }
            let mut kept = core::NetRoute::default();
            kept.segments.clone_from(&from.segments);
            kept.vias.clone_from(&from.vias);
            let mut routes = vec![core::NetRoute::default(); base.nets.len()];
            routes[net as usize] = kept;
            base = with_routes(&base, &routes);
        }
    }
    let mut done = BTreeMap::new();
    for position in wanted {
        let stubs = added.remove(&position).unwrap_or_default();
        let complete = !failed.contains(&position);
        for ((net, _, segments), vias) in &stubs {
            let route = &mut result.routes[*net as usize];
            route.segments.extend(segments.iter().cloned());
            route.vias.extend(vias.iter().cloned());
        }
        let pair = &pairs[candidates[position].0];
        let link = links[position];
        let joined = complete
            && (0..2).filter(|&end| !link.fixed[end]).all(|end| {
                [(0, pair.positive), (1, pair.negative)]
                    .into_iter()
                    .all(|(polarity, net)| {
                        let route = &result.routes[net as usize];
                        let reach = (pair.width
                            + route
                                .segments
                                .iter()
                                .map(|segment| segment.width)
                                .fold(0.0, f64::max))
                            / 2.0;
                        stub_joined(
                            board,
                            route,
                            net,
                            (link.pads[end][polarity], link.pads[1 - end][polarity]),
                            link.stub_end(end, polarity, pair.width),
                            reach,
                        )
                    })
            });
        if joined {
            done.insert(position, stubs.into_iter().map(|(stub, _)| stub).collect());
        } else {
            for ((net, _, segments), vias) in &stubs {
                let route = &mut result.routes[*net as usize];
                route.segments.retain(|segment| !segments.contains(segment));
                route.vias.retain(|via| !vias.contains(via));
            }
        }
        if debug() || pairs_why() {
            eprintln!(
                "pairs why: stubs of {} link {}: {}",
                board.nets[pair.positive as usize].name,
                link.link + 1,
                if joined {
                    "routed after the attempt"
                } else if complete {
                    "routed, still not reaching the run"
                } else {
                    "not routed after the attempt"
                }
            );
        }
    }
    (done, work)
}

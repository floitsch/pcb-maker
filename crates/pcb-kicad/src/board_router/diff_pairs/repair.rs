// Copyright (C) 2026 Toit contributors.

//! The stubs a kept link lacks, routed after the attempt (`repair_stubs`).

use super::*;
use pcb_router as core;

/// A stub routed after the attempt (`repair_stubs`): its net, its pad and
/// its tracks.
pub(super) type RoutedStub = (core::NetId, Point, Vec<core::Segment>);

/// A stub to route: its candidate, its end and its polarity.
type StubItem = (usize, usize, usize);

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
    let base = kept_board(board, &result.routes, pairs, candidates, refused);
    let mut todo: Vec<StubItem> = wanted
        .iter()
        .flat_map(|&position| {
            lacking[position]
                .iter()
                .map(move |&(end, polarity)| (position, end, polarity))
        })
        .collect();
    let budget = work_budget(config.ladder_budget_seconds.unwrap_or(1200.0));
    let router_config = {
        let mut router_config = core_config(config);
        router_config.verbose = false;
        router_config.deadline_work = Some(budget);
        router_config.run_work = None;
        router_config
    };
    let mut repair = StubRepair {
        board,
        pairs,
        candidates,
        links,
        base,
        router_config,
        failed: BTreeSet::new(),
        added: BTreeMap::new(),
        work: 0.0,
    };
    while !todo.is_empty() {
        let round;
        (round, todo) = repair.next_round(todo);
        if round.is_empty() {
            break;
        }
        repair.route_round(&round, !todo.is_empty());
    }
    let work = repair.work;
    (repair.settle(result, wanted), work)
}

/// `board` with `routes` and every candidate's copper whose check held
/// (`refused` none) as copper of its nets: what a repaired stub keeps
/// clear of.
fn kept_board(
    board: &core::Board,
    routes: &[core::NetRoute],
    pairs: &[Pair],
    candidates: &[(usize, usize, LinkCopper)],
    refused: &[Option<String>],
) -> core::Board {
    let mut base = with_routes(board, routes);
    // Every candidate's copper whose check held, as copper of its nets.
    for (position, (index, _, copper)) in candidates.iter().enumerate() {
        if refused[position].is_some() {
            continue;
        }
        let pair = &pairs[*index];
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            for &(layer, start, end) in &copper.segments[polarity] {
                base.obstacles.push(net_copper(
                    core::Shape::Capsule {
                        start,
                        end,
                        radius: pair.width / 2.0,
                    },
                    1 << layer,
                    core::ObstacleKind::Copper,
                    net,
                    "existing track".into(),
                ));
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
                    base.obstacles.push(net_copper(
                        core::Shape::Circle { center: at, radius },
                        board.all_layers(),
                        kind,
                        net,
                        label.into(),
                    ));
                }
            }
        }
    }
    base
}

/// The stub repair's state (`repair_stubs`): the board with every route and
/// held candidate's copper kept (`base`, growing by each round's stubs
/// while more rounds follow), the candidates with a stub that failed, the
/// stubs routed per candidate (with their vias) and the routes' work.
struct StubRepair<'a> {
    board: &'a core::Board,
    pairs: &'a [Pair],
    candidates: &'a [(usize, usize, LinkCopper)],
    links: &'a [&'a LinkPlan],
    base: core::Board,
    router_config: core::Config,
    failed: BTreeSet<usize>,
    added: BTreeMap<usize, Vec<(RoutedStub, Vec<core::Via>)>>,
    work: f64,
}

impl StubRepair<'_> {
    /// One stub per net a round: the round (by net) and the stubs left for
    /// later; those of a failed candidate go.
    fn next_round(&self, todo: Vec<StubItem>) -> (BTreeMap<core::NetId, StubItem>, Vec<StubItem>) {
        let mut round: BTreeMap<core::NetId, StubItem> = BTreeMap::new();
        let mut later = Vec::new();
        for item in todo {
            let (position, _, polarity) = item;
            if self.failed.contains(&position) {
                continue;
            }
            let pair = &self.pairs[self.candidates[position].0];
            let net = [pair.positive, pair.negative][polarity];
            if let std::collections::btree_map::Entry::Vacant(entry) = round.entry(net) {
                entry.insert(item);
            } else {
                later.push(item);
            }
        }
        (round, later)
    }

    /// Routes a round's stubs (`round_board`), each routed one added to its
    /// candidate's stubs and, where more rounds follow (`more`), kept on
    /// the base board; a stub that does not route fails its candidate.
    fn route_round(&mut self, round: &BTreeMap<core::NetId, StubItem>, more: bool) {
        let sub = self.round_board(round);
        // The other nets' pours stay as they are: a stub that cuts one
        // pays for it (`Config::pour_integrity`), as in the attempt (the
        // atmega32u4 board's GND pour on B.Cu split by a stub crossing
        // under its run, pairs5).
        let routed = core::route(&sub, &self.router_config);
        self.work += routed.expansions as f64 / core::router::expansions_per_second()
            + routed.build_work
            + routed.pour_work;
        for (&net, &(position, _, _)) in round {
            if self.failed.contains(&position) {
                continue;
            }
            if routed.status[net as usize] != core::NetStatus::Routed {
                if debug() {
                    eprintln!(
                        "diff pair stub repair of {}: {:?}, terminals {:?}",
                        self.board.nets[net as usize].name,
                        routed.status[net as usize],
                        sub.nets[net as usize]
                            .terminals
                            .iter()
                            .map(|terminal| (terminal.anchor, terminal.layers, &terminal.label))
                            .collect::<Vec<_>>()
                    );
                }
                self.failed.insert(position);
                continue;
            }
            let (end, polarity) = (round[&net].1, round[&net].2);
            let from = &routed.routes[net as usize];
            self.added.entry(position).or_default().push((
                (
                    net,
                    self.links[position].pads[end][polarity],
                    from.segments.clone(),
                ),
                from.vias.clone(),
            ));
            // The next round keeps it.
            if !more {
                continue;
            }
            let mut kept = core::NetRoute::default();
            kept.segments.clone_from(&from.segments);
            kept.vias.clone_from(&from.vias);
            let mut routes = vec![core::NetRoute::default(); self.base.nets.len()];
            routes[net as usize] = kept;
            self.base = with_routes(&self.base, &routes);
        }
    }

    /// The board a round routes on: the base board with no terminals but
    /// each stub's net's pad and its stub's end (its copper a track width
    /// along the lead), the round's nets' merged pads and terminal copper
    /// out. A stub whose pad is no terminal fails its candidate.
    fn round_board(&mut self, round: &BTreeMap<core::NetId, StubItem>) -> core::Board {
        let mut sub = self.base.clone();
        for net in &mut sub.nets {
            net.terminals.clear();
        }
        let nets: BTreeSet<core::NetId> = round.keys().copied().collect();
        sub.merged_pads.retain(|merged| !nets.contains(&merged.net));
        sub.terminal_copper
            .retain(|extra| !nets.contains(&extra.net));
        for (&net, &(position, end, polarity)) in round {
            let pair = &self.pairs[self.candidates[position].0];
            let link = self.links[position];
            let pad = link.pads[end][polarity];
            let Some(terminal) = pad_terminal(self.board, net, pad) else {
                self.failed.insert(position);
                continue;
            };
            let (at, along) = link.stub_end(end, polarity, pair.width);
            let layers = link.runs[end][polarity].layers;
            sub.obstacles.push(net_copper(
                core::Shape::Capsule {
                    start: at,
                    end: along,
                    radius: pair.width / 2.0,
                },
                layers,
                core::ObstacleKind::Copper,
                net,
                "pair stub end".into(),
            ));
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
        sub
    }

    /// Per candidate `wanted`: its stubs added to `result`; kept where all
    /// routed and each of its ends' stubs now reaches the run
    /// (`stub_joined`), else taken out again. The kept candidates' stubs.
    fn settle(
        mut self,
        result: &mut core::RoutingResult,
        wanted: Vec<usize>,
    ) -> BTreeMap<usize, Vec<RoutedStub>> {
        let mut done = BTreeMap::new();
        for position in wanted {
            let stubs = self.added.remove(&position).unwrap_or_default();
            let complete = !self.failed.contains(&position);
            for ((net, _, segments), vias) in &stubs {
                let route = &mut result.routes[*net as usize];
                route.segments.extend(segments.iter().cloned());
                route.vias.extend(vias.iter().cloned());
            }
            let pair = &self.pairs[self.candidates[position].0];
            let link = self.links[position];
            let joined = complete
                && (0..2).filter(|&end| !link.fixed[end]).all(|end| {
                    [(0, pair.positive), (1, pair.negative)]
                        .into_iter()
                        .all(|(polarity, net)| {
                            stub_joined(
                                self.board,
                                &result.routes[net as usize],
                                net,
                                (link.pads[end][polarity], link.pads[1 - end][polarity]),
                                link.stub_end(end, polarity, pair.width),
                                stub_reach(pair, result, net),
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
                    self.board.nets[pair.positive as usize].name,
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
        done
    }
}

/// The terminal of `net`'s pad at `pad` on `board`, or the pad merged into
/// another (its own terminal here).
fn pad_terminal(board: &core::Board, net: core::NetId, pad: Point) -> Option<core::Terminal> {
    board.nets[net as usize]
        .terminals
        .iter()
        .find(|terminal| core::geometry::distance(terminal.anchor, pad) < 1.0e-6)
        .cloned()
        .or_else(|| {
            board
                .merged_pads
                .iter()
                .filter(|merged| merged.net == net)
                .find(|merged| board.obstacles[merged.pad].shape.distance_to_point(pad) <= 1.0e-6)
                .map(|merged| core::Terminal {
                    anchor: pad,
                    layers: board.obstacles[merged.pad].layers,
                    pad: merged.pad,
                    contact: None,
                    label: board.obstacles[merged.pad].label.clone(),
                })
        })
}

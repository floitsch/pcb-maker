// Copyright (C) 2026 Toit contributors.

//! The links' copper after the route (`finish_pairs`' first half): per
//! routed link its centreline offset into the pair's copper, the
//! alternatives where that is refused, and the stubs it lacks
//! (`Finish::link_candidates`); then the copper of every link checked at
//! once, the alternatives in turn (`check_candidates`).

use super::*;
use pcb_router as core;

/// The links whose copper is checked (`Finish::link_candidates`): per
/// candidate its pair and link number with its copper, its plan, its
/// alternative coppers (other centrelines whose two nets keep the pair's
/// gap), and the stubs (end, polarity) that do not reach the run.
#[derive(Default)]
pub(super) struct Candidates<'a> {
    pub coppers: Vec<(usize, usize, LinkCopper)>,
    pub links: Vec<&'a LinkPlan>,
    pub alternatives: Vec<Vec<LinkCopper>>,
    pub lacking: Vec<Vec<(usize, usize)>>,
}

/// `PCB_ROUTER_PAIRS_WHY`: the hard repair's record and the dead pads of
/// the pairs' nets, before the links' are named for them.
#[derive(Default)]
pub(super) struct WhyRecord {
    pub repairs: BTreeMap<core::NetId, core::RepairEntry>,
    pub dead: Vec<String>,
}

impl WhyRecord {
    /// The record of `result`'s diagnostics, where `PCB_ROUTER_PAIRS_WHY`
    /// asks for it (else empty): of the board's nets before the links'
    /// (`nets`).
    pub(super) fn of(result: &core::RoutingResult, nets: usize) -> Self {
        let repairs: BTreeMap<core::NetId, core::RepairEntry> = if pairs_why() {
            result
                .diagnostics
                .repair
                .iter()
                .filter(|entry| (entry.net as usize) < nets)
                .map(|entry| (entry.net, entry.clone()))
                .collect()
        } else {
            BTreeMap::new()
        };
        let dead: Vec<String> = if pairs_why() {
            result
                .diagnostics
                .dead_pads
                .iter()
                .map(|dead| format!("{}:{}", dead.net, dead.label))
                .collect()
        } else {
            Vec::new()
        };
        WhyRecord { repairs, dead }
    }
}

/// Why a link's net was not routed, from the router's diagnostics: a
/// launch with no legal node of the link's class (`dead_pads`: the
/// envelope does not fit there, between pads or in another net's pour); no
/// path even through other nets' copper when the negotiation ended (the
/// link's envelope or via disc finds no way past the fixed copper, or its
/// searches gave up); in conflict then and the hard repair, which treats
/// every other net's copper as fixed, found no room (the negotiation
/// ejected it); or the hard repair did not search for it (the attempt the
/// ladder kept was judged hopeless, which runs no hard repair, or the
/// repair's budget or the attempt's work deadline ran out before it;
/// Diagnostics, `repair`). The verdict's `not_routed` counts them.
pub(super) fn not_routed_cause(diagnostics: &core::Diagnostics, net: core::NetId) -> &'static str {
    if let Some(entry) = diagnostics.released.iter().find(|entry| entry.net == net) {
        return if entry.reason == core::router::NO_PATH {
            "given up in the negotiation: no path"
        } else {
            "given up in the negotiation: no room"
        };
    }
    if diagnostics.dead_pads.iter().any(|dead| dead.net == net) {
        return "a launch has no legal node";
    }
    match diagnostics.repair.iter().find(|entry| entry.net == net) {
        Some(entry) if entry.skipped == "hopeless" => {
            "no hard repair: the attempt was judged hopeless"
        }
        Some(entry) if !entry.skipped.is_empty() => "the hard repair's budget ran out before it",
        Some(entry) if !entry.complete && !entry.conflicted => "no path in the negotiation",
        Some(_) => "the hard repair found no room",
        None => "lost after the repair",
    }
}

impl<'a> Finish<'a> {
    /// Per link of the plan: not routed, its cause counted and its pair
    /// open; else its copper (`link_coppers`) and the stubs it lacks
    /// (`missing_stubs`) as a candidate, or its pair open where its route
    /// has no clean copper.
    pub(super) fn link_candidates(
        &mut self,
        result: &core::RoutingResult,
        causes: &BTreeMap<core::NetId, &'static str>,
        why: &WhyRecord,
    ) -> Candidates<'a> {
        let plan = self.plan;
        let mut candidates = Candidates::default();
        for plan_link in &plan.links {
            let (index, link) = (plan_link.pair, plan_link.link);
            let pair = &plan.pairs[index];
            let offset = (plan_link.net as usize) - plan.nets;
            if self.link_status.get(offset) != Some(&core::NetStatus::Routed) {
                self.not_routed(plan_link, offset, causes);
                continue;
            }
            let path = run_path(&self.link_routes[offset], plan_link);
            if debug() {
                eprintln!("diff pair link {} path {:?}", plan_link.net, path);
            }
            let (copper, alternative) = link_coppers(path.as_ref(), pair, plan_link);
            let missing = missing_stubs(&self.restored, result, pair, plan_link);
            let stubs_joined = missing.is_empty();
            if copper.is_some() && !stubs_joined && debug() {
                eprintln!(
                    "diff pair link {} stub fails: crossed {}, fixed {:?}",
                    self.board.nets[plan_link.net as usize].name,
                    plan_link.crossed,
                    plan_link.fixed
                );
            }
            if copper.is_some() && !stubs_joined && pairs_why() {
                self.open_stubs_why(result, plan_link, why);
            }
            // Its copper is checked with the others; the stubs it lacks are
            // routed after that (`repair_stubs`).
            match copper {
                Some(mut copper) => {
                    // A link whose stubs are still to route gives way too.
                    copper.moved |= !missing.is_empty();
                    candidates.coppers.push((index, link, copper));
                    candidates.alternatives.push(alternative);
                    candidates.lacking.push(missing);
                    candidates.links.push(plan_link);
                }
                None => {
                    self.entries[index].why.push(format!(
                        "link {}: the coupled route has no clean path",
                        link + 1
                    ));
                    self.open_pairs.insert(index);
                }
            }
        }
        candidates
    }

    /// A link not routed (its net's status at `offset` among the links'):
    /// its cause counted, given up or not, and its pair open.
    fn not_routed(
        &mut self,
        plan_link: &LinkPlan,
        offset: usize,
        causes: &BTreeMap<core::NetId, &'static str>,
    ) {
        if debug() {
            eprintln!(
                "diff pair link {} ({}) not routed: {:?}, launches {:?}",
                plan_link.net,
                self.board.nets[plan_link.net as usize].name,
                self.link_status.get(offset),
                plan_link.launches
            );
        }
        let cause = causes.get(&plan_link.net).copied().unwrap_or("");
        *self.report.not_routed.entry(cause.to_string()).or_default() += 1;
        self.report.given_up += usize::from(self.given_up.contains(&plan_link.net));
        self.entries[plan_link.pair].why.push(format!(
            "link {}: the coupled run was not routed ({cause})",
            plan_link.link + 1
        ));
        self.open_pairs.insert(plan_link.pair);
    }

    /// `PCB_ROUTER_PAIRS_WHY`: per stub of a link the router was to route
    /// that does not reach the run, its geometry (`stub_why`) and, where
    /// its net is open, what keeps its pad from its stub's end
    /// (`stub_blocked`) with the hard repair's record and the dead pads.
    fn open_stubs_why(&self, result: &core::RoutingResult, plan_link: &LinkPlan, why: &WhyRecord) {
        let plan = self.plan;
        let (board, link) = (self.board, plan_link.link);
        let pair = &plan.pairs[plan_link.pair];
        for end in (0..2).filter(|&end| !plan_link.fixed[end]) {
            for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
                let line = stub_why(
                    &self.restored,
                    result,
                    plan_link,
                    end,
                    polarity,
                    net,
                    (pair.width, stub_reach(pair, result, net)),
                );
                if line.is_some() && result.status[net as usize] != core::NetStatus::Routed {
                    let blocked = stub_blocked(
                        board,
                        (&result.routes, &self.link_routes),
                        plan.obstacles,
                        plan_link,
                        (end, polarity),
                        [pair.positive, pair.negative],
                        self.config,
                    );
                    let repair = why.repairs.get(&net).map_or_else(
                        || "no repair entry".to_string(),
                        |entry| {
                            format!(
                                "complete {}, conflicted {}, skipped '{}'",
                                entry.complete, entry.conflicted, entry.skipped
                            )
                        },
                    );
                    let dead: Vec<&String> = why
                        .dead
                        .iter()
                        .filter(|dead| {
                            dead.starts_with(&format!("{net}:"))
                                || dead.starts_with(&format!("{}:", plan_link.net))
                        })
                        .collect();
                    eprintln!(
                        "pairs why: open {} link {} end {} ({} end): {blocked}; repair: {repair}; dead pads {dead:?}",
                        board.nets[net as usize].name,
                        link + 1,
                        ["a", "b"][end],
                        if end_crossed(plan_link, end) {
                            "crossed"
                        } else {
                            "straight"
                        },
                    );
                }
                if let Some(line) = line {
                    eprintln!(
                        "pairs why: stub {} link {} end {} ({}): {line}",
                        board.nets[net as usize].name,
                        link + 1,
                        ["a", "b"][end],
                        if plan_link.crossed {
                            "crossed"
                        } else {
                            "straight"
                        },
                    );
                }
            }
        }
    }
}

/// A routed link's run from its route (`link_path`): from its start (the
/// stubs' ends beside it) to the route's first node past the launch (the
/// route's own stub to the launch goes), and the same at its end; half the
/// lead straight along the launch's direction at each end.
pub(super) fn run_path(
    route: &core::NetRoute,
    plan_link: &LinkPlan,
) -> Option<Vec<(Point, usize)>> {
    let [at_a, at_b] = plan_link.launches;
    let [start_a, start_b] = plan_link.starts;
    link_path(route, at_a, at_b).map(|mut path| {
        if path.len() >= 3
            && path[0].1 == path[1].1
            && core::geometry::distance(path[0].0, at_a) < 1.0e-6
        {
            path.remove(0);
        }
        // On a route of two pieces on one layer (launch a, a node, launch
        // b) only end a's stub goes: the guard keeps two of the route's
        // nodes. End b then keeps its launch, which lies on its lead's
        // line beyond the half point below, so the run holds b's whole
        // lead straight where it cuts from a's half point to the node.
        // Both are tracks along the route, checked like any run
        // (`check_candidates`); dropping b's launch too would cut that
        // corner and change the copper, so the asymmetry stays (it met
        // none of the 304 runs of twelve pair boards' designer routes,
        // 2026-11-11).
        let count = path.len();
        if count >= 3
            && path[count - 1].1 == path[count - 2].1
            && core::geometry::distance(path[count - 1].0, at_b) < 1.0e-6
        {
            path.pop();
        }
        // Half the lead straight along the launch's direction, so that
        // the run's tracks start beside the stubs' ends.
        let (first, last) = (path[0], path[path.len() - 1]);
        let half = |start: Point, at: Point| scale(add(start, at), 0.5);
        path.insert(0, (half(start_a, at_a), first.1));
        path.insert(0, (start_a, first.1));
        path.push((half(start_b, at_b), last.1));
        path.push((start_b, last.1));
        path
    })
}

/// A link's copper along `path` (`link_copper`, its vias `spread` times as
/// far apart) with its ends: the jogs from the stubs' ends to the run's
/// tracks (or a crossover's copper), the stubs the link carries, and the
/// crossovers' vias.
fn link_copper_at(
    path: &[(Point, usize)],
    pair: &Pair,
    plan_link: &LinkPlan,
    spread: f64,
) -> Option<LinkCopper> {
    link_copper(path, pair, plan_link.side, spread).map(|mut copper| {
        copper.crossed = plan_link.crossed;
        // The jogs from the stubs' ends to the run's tracks.
        for (polarity, segments) in copper.segments.iter_mut().enumerate() {
            let [from, to] = [plan_link.ends[0][polarity], plan_link.ends[1][polarity]];
            let length = segments.len();
            let mut front = 0;
            // A crossover's copper from the stubs' ends to the run,
            // else a jog.
            if let Some(crossing) = &plan_link.crossovers[0] {
                segments.splice(0..0, crossing.segments[polarity].iter().copied());
                front += crossing.segments[polarity].len();
            } else if let Some(&(layer, tip, _)) = segments.first()
                && core::geometry::distance(tip, from) > 1.0e-6
            {
                segments.insert(0, (layer, from, tip));
                front += 1;
            }
            if let Some(crossing) = &plan_link.crossovers[1] {
                segments.extend(
                    crossing.segments[polarity]
                        .iter()
                        .rev()
                        .map(|&(layer, from, to)| (layer, to, from)),
                );
            } else if let Some(&(layer, _, tip)) = segments.last()
                && core::geometry::distance(tip, to) > 1.0e-6
            {
                segments.push((layer, tip, to));
            }
            // The stubs the link carries, from the pads.
            if plan_link.fixed[0]
                && let Some(&(layer, _, _)) = segments.first()
            {
                segments.insert(0, (layer, plan_link.stubs[0][polarity], from));
                front += 1;
            }
            if plan_link.fixed[1]
                && let Some(&(layer, _, _)) = segments.last()
            {
                segments.push((layer, to, plan_link.stubs[1][polarity]));
            }
            copper.run[polarity] = front..front + length;
        }
        for crossing in plan_link.crossovers.iter().flatten() {
            for polarity in 0..2 {
                copper.vias[polarity].extend(crossing.vias[polarity].iter().copied());
            }
            copper.crossovers += 1;
        }
        copper
    })
}

/// `link_copper_at`, where a via of one net comes too close to the other
/// net's copper (a layer change in a sharp turn) with the vias further out
/// (`VIA_SPREADS`).
fn spread_link_copper(
    path: &[(Point, usize)],
    pair: &Pair,
    plan_link: &LinkPlan,
) -> Option<LinkCopper> {
    let first = link_copper_at(path, pair, plan_link, 1.0);
    if first
        .as_ref()
        .is_none_or(|copper| own_vias_clear(copper, pair))
    {
        return first;
    }
    VIA_SPREADS
        .iter()
        .filter_map(|&spread| link_copper_at(path, pair, plan_link, spread))
        .find(|copper| own_vias_clear(copper, pair))
        .or(first)
}

/// A routed link's copper along its run `path` and the alternatives: the
/// route as it is, without its hairpins (the route turning back by more
/// than a right angle, which the offset tracks cannot follow apart) and
/// short jogs (`despiked`), with its layer changes moved off its turns
/// (`vias_on_straights`: a via pair placed in a turn lands on the
/// partner's track of the other layer), both, then with longer jogs
/// straightened (`centrelines`): the route as it is is the copper, the
/// others whose two nets keep the pair's gap from each other
/// (`own_copper_clear`) the alternatives, in turn, where that is refused.
fn link_coppers(
    path: Option<&Vec<(Point, usize)>>,
    pair: &Pair,
    plan_link: &LinkPlan,
) -> (Option<LinkCopper>, Vec<LinkCopper>) {
    let centrelines = path.map(|path| centrelines(path, pair)).unwrap_or_default();
    // The route as it is with its vias where the router put them
    // first; every other copper (the vias spread out, `build`, or
    // another centreline) moved.
    let mut built: Vec<LinkCopper> = Vec::new();
    for (position, line) in centrelines.iter().enumerate() {
        if position == 0
            && let Some(copper) = link_copper_at(line, pair, plan_link, 1.0)
        {
            built.push(copper);
        }
        if let Some(mut copper) = spread_link_copper(line, pair, plan_link) {
            copper.moved = true;
            if position > 0
                || built.first().is_none_or(|first| {
                    first.segments != copper.segments || first.vias != copper.vias
                })
            {
                built.push(copper);
            }
        }
    }
    // The route as it is first, as before (where it holds, nothing
    // changes); then the moved ones whose nets keep their gap.
    let mut built = built.into_iter();
    let first = built.next();
    match first {
        Some(copper) if !copper.moved => (
            Some(copper),
            built
                .filter(|copper| own_copper_clear(copper, pair))
                .collect::<Vec<LinkCopper>>(),
        ),
        first => {
            let mut clean: Vec<LinkCopper> = first
                .into_iter()
                .chain(built)
                .filter(|copper| own_copper_clear(copper, pair))
                .collect();
            let copper = (!clean.is_empty()).then(|| clean.remove(0));
            (copper, clean)
        }
    }
}

/// How far from a stub's end `net`'s route may end and still reach it:
/// half the pair's width and the route's widest track.
pub(super) fn stub_reach(pair: &Pair, result: &core::RoutingResult, net: core::NetId) -> f64 {
    (pair.width
        + result.routes[net as usize]
            .segments
            .iter()
            .map(|segment| segment.width)
            .fold(0.0, f64::max))
        / 2.0
}

/// The stubs (end, polarity) of a link's ends the router routes that do
/// not reach their end of the run (`stub_joined`): each pad's stub must
/// (else the run is an island and the pad open).
fn missing_stubs(
    restored: &core::Board,
    result: &core::RoutingResult,
    pair: &Pair,
    plan_link: &LinkPlan,
) -> Vec<(usize, usize)> {
    (0..2)
        .filter(|&end| !plan_link.fixed[end])
        .flat_map(|end| [(end, 0), (end, 1)])
        .filter(|&(end, polarity)| {
            let net = [pair.positive, pair.negative][polarity];
            !stub_joined(
                restored,
                &result.routes[net as usize],
                net,
                (
                    plan_link.pads[end][polarity],
                    plan_link.pads[1 - end][polarity],
                ),
                plan_link.stub_end(end, polarity, pair.width),
                stub_reach(pair, result, net),
            )
        })
        .collect()
}

/// A candidate's copper as a group of the check (`refused_groups`).
fn group_of(index: usize, copper: &LinkCopper) -> Group {
    Group {
        pair: index,
        segments: copper.segments.clone(),
        vias: copper.vias.clone(),
        run: copper.run.clone(),
        moved: copper.moved,
    }
}

/// Per candidate why its copper is refused (`None`: kept): every
/// candidate's copper checked at once against `restored` and the routes
/// (`refused_groups`); a refused one with another clean copper
/// (`centrelines`) is checked again with it, beside every other link's,
/// until one holds or none is left (the copper that holds replaces the
/// candidate's).
pub(super) fn check_candidates(
    restored: &core::Board,
    routes: &[core::NetRoute],
    pairs: &[Pair],
    candidates: &mut Candidates,
) -> Vec<Option<String>> {
    let groups: Vec<Group> = candidates
        .coppers
        .iter()
        .map(|(index, _, copper)| group_of(*index, copper))
        .collect();
    let mut refused = refused_groups(restored, routes, pairs, &groups);
    // A refused link with another clean copper (`centrelines`) is checked
    // again with it, beside every other link's, until one holds or none is
    // left.
    loop {
        let swapped: Vec<bool> = refused
            .iter()
            .zip(&candidates.alternatives)
            .map(|(why, other)| why.is_some() && !other.is_empty())
            .collect();
        if !swapped.contains(&true) {
            break;
        }
        let second: Vec<Group> = candidates
            .coppers
            .iter()
            .zip(&candidates.alternatives)
            .zip(&swapped)
            .map(|(((index, _, copper), other), &swap)| {
                let copper = if swap { &other[0] } else { copper };
                group_of(*index, copper)
            })
            .collect();
        let again = refused_groups(restored, routes, pairs, &second);
        for position in 0..candidates.coppers.len() {
            if !swapped[position] {
                continue;
            }
            let option = candidates.alternatives[position].remove(0);
            if again[position].is_none() {
                refused[position] = None;
                candidates.coppers[position].2 = option;
            }
        }
    }
    refused
}

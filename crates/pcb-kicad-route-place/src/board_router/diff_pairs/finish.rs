// Copyright (C) 2026 Toit contributors.

//! After the route (`finish_pairs`): the board as lowered restored, the
//! links' copper made and checked (candidates.rs), the stubs a kept link
//! lacks routed, the kept links' copper on the board, the failed links'
//! stubs removed, the fallback, the skew's meanders, the return vias and
//! the report.

use super::*;
use pcb_router as core;

/// What `finish_pairs` reads and builds: the plan and the routed board it
/// planned, the board as lowered (`restored`, the kept copper added), the
/// links' routes and status split off the result, the links given up, the
/// report and its pairs' entries, and the pairs a link left open.
pub(super) struct Finish<'a> {
    pub plan: &'a PairPlan,
    pub board: &'a core::Board,
    pub config: &'a KiCadBoardRouterConfig,
    pub entries: Vec<KiCadDiffPair>,
    pub report: KiCadDiffPairs,
    pub restored: core::Board,
    pub link_routes: Vec<core::NetRoute>,
    pub link_status: Vec<core::NetStatus>,
    pub given_up: BTreeSet<core::NetId>,
    pub open_pairs: BTreeSet<usize>,
}

/// The kept links (`Finish::keep_links`): the copper out, per kept link its
/// run (`KeptRun`) and its plan.
struct Kept<'a> {
    copper: PairCopper,
    runs: Vec<KeptRun>,
    links: Vec<&'a LinkPlan>,
}

/// A routed board with its pairs' links as nets (`plan_pairs`) turned back
/// into the board as lowered: each link's route offset into the pair's
/// tracks and vias from one run start to the other (where the pair's nets'
/// stubs end) and checked (`refused_groups`, against everything routed);
/// what holds is kept copper of the returned board (`existing track`,
/// `existing via`) and in the returned `PairCopper`; the links' nets and
/// the stubs' end terminals are gone from the board and the result; what
/// the links left open is routed for the pairs' nets with everything else
/// kept (the fallback, its work in the report). `None`: the board has no
/// plan.
pub fn finish_pairs(
    board: &core::Board,
    result: &mut core::RoutingResult,
    config: &KiCadBoardRouterConfig,
    heights: &[f64],
) -> Option<(core::Board, PairCopper, KiCadDiffPairs)> {
    let plan = plan_of(board)?;
    let plan: &PairPlan = &plan;
    let mut finish = Finish::new(plan, board, result, config);
    let why = WhyRecord::of(result, plan.nets);
    // Why each link not routed was not (`not_routed_cause`), before the
    // diagnostics name the pairs' nets for the links'.
    let causes: BTreeMap<core::NetId, &'static str> = plan
        .links
        .iter()
        .filter(|link| {
            finish.link_status.get(link.net as usize - plan.nets) != Some(&core::NetStatus::Routed)
        })
        .map(|link| (link.net, not_routed_cause(&result.diagnostics, link.net)))
        .collect();
    // The links the negotiation gave up: their pairs' nets routed to the
    // pads without them (no stubs to remove).
    finish.given_up = result
        .diagnostics
        .released
        .iter()
        .map(|entry| entry.net)
        .collect();
    name_links_for_pairs(result, plan);
    // The links' copper.
    let mut candidates = finish.link_candidates(result, &causes, &why);
    let mut refused = check_candidates(
        &finish.restored,
        &result.routes,
        &plan.pairs,
        &mut candidates,
    );
    let repaired = finish.repair_lacking(result, &candidates, &mut refused);
    finish.report.stubs_routed += repaired.values().map(Vec::len).sum::<usize>();
    let kept = finish.keep_links(candidates, &refused);
    let kept_nets: BTreeSet<core::NetId> = kept.links.iter().map(|link| link.net).collect();
    finish.remove_failed_stubs(result, &kept_nets);
    // A repaired stub's pad keeps one way to the run: its old route
    // elsewhere (to the link's other end, through other pads) goes where
    // the stub makes it a loop (`cut_old_connection`).
    for stubs in repaired.values() {
        for (net, pad, new) in stubs {
            cut_old_connection(
                &finish.restored,
                &mut result.routes[*net as usize],
                *net,
                *pad,
                new,
            );
        }
    }
    // The kept links' stubs straight where that holds (the router's run
    // parallel out of the pads, which KiCad takes for a coupled piece at
    // the pads' pitch).
    for plan_link in &kept.links {
        straighten_stubs(&finish.restored, result, &plan.pairs, plan_link);
        join_stub_ends(result, &plan.pairs, plan_link);
    }
    finish.close_given_up(result, &kept_nets);
    finish.fall_back(result, &kept_nets);
    let Kept {
        copper: mut copper_out,
        runs: kept_runs,
        ..
    } = kept;
    finish.match_skews(result, &kept_runs, &mut copper_out, heights);
    finish.return_vias(result, &kept_runs, &copper_out);
    // A pair's net whose pads the links joined all is routed, not trivial.
    for (net, original) in &plan.originals {
        let net = *net as usize;
        if original.terminals.len() >= 2 && result.status[net] == core::NetStatus::Trivial {
            result.status[net] = core::NetStatus::Routed;
        }
    }
    let report = finish.totals();
    Some((finish.restored, copper_out, report))
}

/// The router's diagnostics of the links' nets named for their pairs'
/// (the positive net), the conflicted nets sorted without repeats.
fn name_links_for_pairs(result: &mut core::RoutingResult, plan: &PairPlan) {
    let pair_net = |net: core::NetId| {
        if (net as usize) < plan.nets {
            net
        } else {
            plan.links
                .iter()
                .find(|link| link.net == net)
                .map_or(0, |link| plan.pairs[link.pair].positive)
        }
    };
    for dead in &mut result.diagnostics.dead_pads {
        dead.net = pair_net(dead.net);
    }
    for entry in &mut result.diagnostics.repair {
        entry.net = pair_net(entry.net);
    }
    for net in &mut result.diagnostics.conflicted {
        *net = pair_net(*net);
    }
    result.diagnostics.conflicted.sort_unstable();
    result.diagnostics.conflicted.dedup();
}

impl<'a> Finish<'a> {
    /// The board as lowered (`plan`'s counts of nets, obstacles and
    /// classes, the pairs' nets, merged pads and terminal copper as they
    /// were), and the links' routes and status split off `result`.
    fn new(
        plan: &'a PairPlan,
        board: &'a core::Board,
        result: &mut core::RoutingResult,
        config: &'a KiCadBoardRouterConfig,
    ) -> Self {
        let entries = plan.entries.clone();
        let report = KiCadDiffPairs {
            found: plan.pairs.len(),
            ..KiCadDiffPairs::default()
        };
        // The board as lowered.
        let mut restored = board.clone();
        restored.attachment = core::Attachment::default();
        restored.nets.truncate(plan.nets);
        restored.obstacles.truncate(plan.obstacles);
        restored.classes.truncate(plan.classes);
        for (net, original) in &plan.originals {
            restored.nets[*net as usize] = original.clone();
        }
        restored.merged_pads = plan.merged_pads.clone();
        restored.terminal_copper = plan.terminal_copper.clone();
        let link_routes: Vec<core::NetRoute> =
            result.routes.split_off(plan.nets.min(result.routes.len()));
        let link_status: Vec<core::NetStatus> =
            result.status.split_off(plan.nets.min(result.status.len()));
        Finish {
            plan,
            board,
            config,
            entries,
            report,
            restored,
            link_routes,
            link_status,
            given_up: BTreeSet::new(),
            open_pairs: BTreeSet::new(),
        }
    }

    /// The stubs a candidate whose copper holds lacks: routed from the pad
    /// to the stub's end with everything else kept (`repair_stubs`); where
    /// one does not route, the link falls back as before (its refusal
    /// `refused`). Per candidate repaired: its stubs.
    fn repair_lacking(
        &mut self,
        result: &mut core::RoutingResult,
        candidates: &Candidates,
        refused: &mut [Option<String>],
    ) -> BTreeMap<usize, Vec<RoutedStub>> {
        let plan = self.plan;
        let lacking = &candidates.lacking;
        let mut repaired: BTreeMap<usize, Vec<RoutedStub>> = BTreeMap::new();
        if !(0..candidates.coppers.len())
            .any(|position| refused[position].is_none() && !lacking[position].is_empty())
        {
            return repaired;
        }
        let (done, work) = repair_stubs(
            &self.restored,
            result,
            &plan.pairs,
            (&candidates.coppers, &candidates.links),
            lacking,
            refused,
            self.config,
        );
        self.report.work_seconds += work;
        for position in 0..candidates.coppers.len() {
            if refused[position].is_some() || lacking[position].is_empty() {
                continue;
            }
            match done.get(&position) {
                Some(stubs) => {
                    // A pair's net left open by the attempt: the fallback
                    // completes it around the link.
                    let pair = &plan.pairs[candidates.coppers[position].0];
                    if [pair.positive, pair.negative]
                        .iter()
                        .any(|&net| result.status[net as usize] != core::NetStatus::Routed)
                    {
                        self.open_pairs.insert(candidates.coppers[position].0);
                    }
                    repaired.insert(position, stubs.clone());
                }
                None => refused[position] = Some("a pad's stub does not reach the run".into()),
            }
        }
        repaired
    }

    /// The candidates whose copper holds: their copper kept on the restored
    /// board (`existing track`, `existing via`, its hole with crossovers)
    /// and in the copper out, counted in their pairs' entries; the refused
    /// ones' pairs open, their reasons in the entries.
    fn keep_links(&mut self, candidates: Candidates<'a>, refused: &[Option<String>]) -> Kept<'a> {
        let plan = self.plan;
        let mut kept = Kept {
            copper: PairCopper {
                items: Vec::new(),
                patterns: Vec::new(),
            },
            // Per kept link: its pair, its copper's items (positive,
            // negative) and run ranges, and the positive net's side.
            runs: Vec::new(),
            links: Vec::new(),
        };
        for (position, (index, link, copper)) in candidates.coppers.into_iter().enumerate() {
            if let Some(why) = &refused[position] {
                self.entries[index]
                    .why
                    .push(format!("link {}: {why}", link + 1));
                self.open_pairs.insert(index);
                continue;
            }
            kept.links.push(candidates.links[position]);
            kept.runs.push((
                index,
                [kept.copper.items.len(), kept.copper.items.len() + 1],
                copper.run.clone(),
                candidates.links[position].side,
            ));
            let pair = &plan.pairs[index];
            for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
                self.keep_copper(pair, net, &copper, polarity);
                kept.copper.items.push((
                    self.restored.nets[net as usize].name.clone(),
                    pair.width,
                    pair.via_diameter,
                    pair.via_drill,
                    copper.segments[polarity].clone(),
                    copper.vias[polarity].clone(),
                ));
            }
            self.entries[index].links_coupled += 1;
            self.entries[index].coupled_mm += copper.coupled;
            self.report.crossed += copper.crossed as usize;
            self.report.crossovers += copper.crossovers;
        }
        kept
    }

    /// A kept link's copper of `polarity` (net `net`) as kept copper of the
    /// restored board: its tracks, its vias and, with crossovers, their
    /// holes (the fallback routes beside them).
    fn keep_copper(&mut self, pair: &Pair, net: core::NetId, copper: &LinkCopper, polarity: usize) {
        let restored = &mut self.restored;
        for &(layer, start, end) in &copper.segments[polarity] {
            restored.obstacles.push(net_copper(
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
            restored.obstacles.push(net_copper(
                core::Shape::Circle {
                    center: at,
                    radius: pair.via_diameter / 2.0,
                },
                restored.all_layers(),
                core::ObstacleKind::Copper,
                net,
                "existing via".into(),
            ));
            // Its drill keeps the hole clearance from other nets'
            // copper (the fallback routes beside it).
            if !crossovers() {
                continue;
            }
            restored.obstacles.push(net_copper(
                core::Shape::Circle {
                    center: at,
                    radius: pair.via_drill / 2.0,
                },
                restored.all_layers(),
                core::ObstacleKind::Hole,
                net,
                "existing via hole".into(),
            ));
        }
    }

    /// The links neither kept nor given up in the negotiation, of the plan.
    fn failed_links<'b>(
        &'b self,
        kept_nets: &'b BTreeSet<core::NetId>,
    ) -> impl Iterator<Item = &'a LinkPlan> + 'b {
        let plan = self.plan;
        plan.links
            .iter()
            .filter(|link| !kept_nets.contains(&link.net) && !self.given_up.contains(&link.net))
    }

    /// The stubs of the links not kept go (the fallback routes the pads
    /// anew).
    fn remove_failed_stubs(
        &self,
        result: &mut core::RoutingResult,
        kept_nets: &BTreeSet<core::NetId>,
    ) {
        for plan_link in self.failed_links(kept_nets) {
            let pair = &self.plan.pairs[plan_link.pair];
            for end in (0..2).filter(|&end| !plan_link.fixed[end]) {
                for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
                    let route = &mut result.routes[net as usize];
                    if let Some((_, stub)) = stub_path(
                        route,
                        plan_link.pads[end][polarity],
                        plan_link.stub_end(end, polarity, pair.width),
                        pair.width,
                    ) {
                        let mut index = 0;
                        route.segments.retain(|_| {
                            index += 1;
                            !stub.contains(&(index - 1))
                        });
                    }
                }
            }
        }
    }

    /// A pair whose links not kept were all given up in the negotiation,
    /// and whose nets the attempt routed without them, needs no fallback.
    fn close_given_up(&mut self, result: &core::RoutingResult, kept_nets: &BTreeSet<core::NetId>) {
        let (plan, given_up) = (self.plan, &self.given_up);
        self.open_pairs.retain(|&index| {
            let pair = &plan.pairs[index];
            let routed = [pair.positive, pair.negative]
                .iter()
                .all(|&net| result.status[net as usize] == core::NetStatus::Routed);
            !routed
                || plan.links.iter().any(|link| {
                    link.pair == index
                        && !kept_nets.contains(&link.net)
                        && !given_up.contains(&link.net)
                })
        });
    }

    /// The fallback: the pairs' nets with a connection a link left open,
    /// routed with everything else kept (its work the report's); then what
    /// is left of the failed links' stubs that joins nothing, and any
    /// track or via of a fallen-back pair's nets that joins nothing at one
    /// end (a stub's piece the prune from its end did not reach: one that
    /// ended on the other end's copper, a pad exit left when the stub went;
    /// KiCad's track_dangling and via_dangling).
    fn fall_back(&mut self, result: &mut core::RoutingResult, kept_nets: &BTreeSet<core::NetId>) {
        if self.open_pairs.is_empty() {
            return;
        }
        let plan = self.plan;
        self.report.work_seconds = route_fallback(
            &self.restored,
            result,
            &plan.pairs,
            &self.open_pairs,
            self.config,
        );
        // What is left of the failed links' stubs that joins nothing.
        for plan_link in self.failed_links(kept_nets) {
            let pair = &plan.pairs[plan_link.pair];
            for end in (0..2).filter(|&end| !plan_link.fixed[end]) {
                for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
                    let route = &mut result.routes[net as usize];
                    let reach = (pair.width
                        + route
                            .segments
                            .iter()
                            .map(|segment| segment.width)
                            .fold(0.0, f64::max))
                        / 2.0;
                    prune_stub(
                        &self.restored,
                        route,
                        net,
                        plan_link.stub_end(end, polarity, pair.width),
                        reach,
                    );
                }
            }
        }
        for &index in &self.open_pairs {
            let pair = &plan.pairs[index];
            for net in [pair.positive, pair.negative] {
                prune_dangling(&self.restored, &mut result.routes[net as usize], net);
            }
        }
    }

    /// The skew: per pair with a kept link, the shorter net's meander
    /// (`match_skew`), counted in the report.
    fn match_skews(
        &mut self,
        result: &core::RoutingResult,
        kept_runs: &[KeptRun],
        copper_out: &mut PairCopper,
        heights: &[f64],
    ) {
        let plan = self.plan;
        for (index, entry) in self.entries.iter_mut().enumerate() {
            if skew_matching() && kept_runs.iter().any(|(owner, ..)| *owner == index) {
                match_skew(
                    &mut self.restored,
                    result,
                    &plan.pairs,
                    index,
                    kept_runs,
                    copper_out,
                    entry,
                    heights,
                );
                self.report.meanders += (entry.meander_mm > 0.0) as usize;
            }
        }
    }

    /// The return vias beside the kept links' vias (`place_return_vias`),
    /// where they are on and the board has a reference net; else only the
    /// pairs' vias counted.
    fn return_vias(
        &mut self,
        result: &mut core::RoutingResult,
        kept_runs: &[KeptRun],
        copper_out: &PairCopper,
    ) {
        if return_vias_on(self.config)
            && let Some(reference) = return_net(&self.restored, self.config)
        {
            self.report.return_net = self.restored.nets[reference as usize].name.clone();
            place_return_vias(
                &self.restored,
                result,
                &self.plan.pairs,
                kept_runs,
                copper_out,
                &mut self.entries,
                reference,
                self.config,
            );
        } else {
            for &(index, items, ..) in kept_runs {
                self.entries[index].pair_vias += items
                    .iter()
                    .map(|&item| copper_out.items[item].5.len())
                    .sum::<usize>();
            }
        }
    }

    /// The report: its pairs' entries summed and counted by how coupled
    /// they are; the summary lines of `PCB_ROUTER_PAIRS_WHY` and
    /// `PCB_ROUTER_DIFF_PAIRS_DEBUG`.
    fn totals(&mut self) -> KiCadDiffPairs {
        let mut report = std::mem::take(&mut self.report);
        let entries = std::mem::take(&mut self.entries);
        for entry in &entries {
            report.links += entry.links;
            report.links_coupled += entry.links_coupled;
            report.coupled_mm += entry.coupled_mm;
            report.pair_vias += entry.pair_vias;
            report.return_vias += entry.return_vias;
            if entry.links > 0 && entry.links_coupled == entry.links {
                report.coupled += 1;
            } else if entry.links_coupled > 0 {
                report.partly_coupled += 1;
            } else {
                report.fallen_back += 1;
            }
        }
        report.pairs = entries;
        if pairs_why() {
            let count = |text: &str| {
                report
                    .pairs
                    .iter()
                    .flat_map(|pair| &pair.why)
                    .filter(|why| why.contains(text))
                    .count()
            };
            eprintln!(
                "pairs why: written: {} of {} links coupled, pairs {} / {} / {}, stubs routed {}, a pad's stub {}, copper {}",
                report.links_coupled,
                report.links,
                report.coupled,
                report.partly_coupled,
                report.fallen_back,
                report.stubs_routed,
                count("stub does not reach"),
                count("its copper is")
            );
        }
        if debug() {
            eprintln!(
                "diff pairs written: {} of {} links coupled, fallback {:.2} s of work for {} pairs",
                report.links_coupled,
                report.links,
                report.work_seconds,
                self.open_pairs.len()
            );
        }
        report
    }
}

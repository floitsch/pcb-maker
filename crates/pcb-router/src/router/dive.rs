// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The crossing dive (`Config::crossing_dive`, off by default): a move of
//! the trouble repair for two nets of one fine-pitch row whose targets lie
//! on opposite sides (their routes must cross) where the one left open has
//! a straight way out closed by the other's copper at its tip (a via just
//! beyond it, a track turning across it). On one layer the two cannot both
//! leave; one of them has to dive, and in a 0.4 or 0.5 mm row a via fits
//! only beyond the neighbours' tips (docs/router.md, "Fan-out before
//! negotiation (2026-10-28)", the diagnosis).
//!
//! The move tries both assignments: the diver loses its branches at the
//! row's pad and takes a fixed stub from the pad straight out to one of the
//! via sites the fan-out computes for that pad (`fanout_plan`, the sites
//! beyond the neighbours' runs) and a via there; the nets the stub then
//! overlaps lose the branches it crosses; the other net of the pair (the
//! one that stays on the layer) is routed again first, hard, then the other
//! nets displaced, then the diver from its via, as the trouble exchange
//! routes its victims. A trial counts where the diver's route takes the
//! via. The assignment kept is the one that leaves the nets involved with
//! the fewest open terminals, fewer than before and none in conflict; else
//! the board is as it was. Every search is the router's own, on the work
//! clock.

use super::fanout::{DensePad, FanoutPlan};
use super::trouble::{Exchange, Saved, TroubleSettings, TroubleWork, clean, open_now, open_saved};
use super::*;

/// `Config::crossing_dive`'s default.
pub const CROSSING_DIVE_DEFAULT: bool = false;

/// `PCB_ROUTER_CROSSING_DIVE` when set: overrides `Config::crossing_dive`
/// (`0` or empty off, anything else on).
pub fn crossing_dive_override() -> Option<bool> {
    let value = std::env::var("PCB_ROUTER_CROSSING_DIVE").ok()?;
    Some(!(value.is_empty() || value == "0"))
}

/// The most dive trials for one trouble-maker (pairs, assignments and via
/// sites together).
const DIVE_TRIALS: usize = 8;

/// A row neighbour lies at most this far along the row from the pad
/// (centre to centre), mm: two or three pads of a 0.4 to 0.5 mm row.
const ROW_REACH: f64 = 1.5;

/// A crossing pair at a fine-pitch row: `open`, a dense pad (an index of
/// the fan-out plan's pads) of a net not complete; `other`, a dense pad of
/// another net in the same row (part, layer and outward side) whose copper
/// closes one of `open`'s straight ways out and whose target lies on the
/// other side of `open`'s along the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CrossingPair {
    pub(super) open: usize,
    pub(super) other: usize,
}

/// The fan-out plan (the dense pads, their ways out and via sites, on the
/// statics) and its pads by net and by row, for the dives.
pub(super) struct DivePlan {
    pub(super) plan: FanoutPlan,
    by_net: HashMap<NetId, Vec<usize>>,
    by_row: HashMap<(String, usize, usize), Vec<usize>>,
}

impl DivePlan {
    pub(super) fn new(plan: FanoutPlan) -> Self {
        let mut by_net: HashMap<NetId, Vec<usize>> = HashMap::new();
        let mut by_row: HashMap<(String, usize, usize), Vec<usize>> = HashMap::new();
        for (index, pad) in plan.pads.iter().enumerate() {
            by_net.entry(pad.net).or_default().push(index);
            if let Some((part, outward)) = &pad.row {
                by_row
                    .entry((part.clone(), pad.layer, *outward))
                    .or_default()
                    .push(index);
            }
        }
        Self {
            plan,
            by_net,
            by_row,
        }
    }
}

/// The coordinate along a row that leaves along `outward` (an index of
/// `DIRECTIONS`).
fn along_row(outward: usize, point: crate::geometry::Point) -> f64 {
    let (dx, dy) = DIRECTIONS[outward];
    point[0] * -(dy as f64) + point[1] * dx as f64
}

impl Router {
    fn pad_anchor(&self, pad: &DensePad) -> crate::geometry::Point {
        self.board.nets[pad.net as usize].terminals[pad.terminal].anchor
    }

    /// Whether `pad`'s terminal is open: its net is not complete and no
    /// branch of the net ends at it (a net's first terminal counts as
    /// connected in its flags even where no route left it).
    fn pad_open(&self, pad: &DensePad) -> bool {
        let state = &self.nets[pad.net as usize];
        let terminal = pad.terminal as u16;
        !state.complete
            && !state
                .branches
                .iter()
                .any(|branch| branch.start_terminal == terminal || branch.end_terminal == terminal)
    }

    fn pad_label(&self, pad: &DensePad) -> &str {
        &self.board.nets[pad.net as usize].terminals[pad.terminal].label
    }

    /// What closes `pad`'s straight ways out (its runs along the axes,
    /// without a via): per run, the nets of `nets` whose copper comes
    /// closer to the run than both half widths and the clearance (a via by
    /// its radius), and whether by a via; into `blocks` as (run, net, via).
    /// Returns the nodes looked at (the work).
    fn tip_blocks(
        &self,
        pad: &DensePad,
        nets: &mut dyn Iterator<Item = NetId>,
        blocks: &mut Vec<(usize, NetId, bool)>,
    ) -> u64 {
        let runs: Vec<(usize, &super::fanout::ExitRun)> = pad
            .runs
            .iter()
            .enumerate()
            .filter(|(_, run)| run.direction % 2 == 0 && !run.via)
            .collect();
        if runs.is_empty() {
            return 0;
        }
        // The runs' box, grown by the furthest reach (a via of 1 mm).
        let grow = pad.half_width + pad.clearance.max(0.5) + 1.0;
        let (mut x0, mut y0, mut x1, mut y1) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for (_, run) in &runs {
            x0 = x0.min(run.start[0].min(run.end[0]) - grow);
            y0 = y0.min(run.start[1].min(run.end[1]) - grow);
            x1 = x1.max(run.start[0].max(run.end[0]) + grow);
            y1 = y1.max(run.start[1].max(run.end[1]) + grow);
        }
        let mut work = 0u64;
        for net in nets {
            if net == pad.net {
                continue;
            }
            let class = self.board.nets[net as usize].class;
            let rules = self.board.classes[class];
            let clearance = pad.clearance.max(rules.clearance) - 1.0e-9;
            for branch in &self.nets[net as usize].branches {
                for (index, node) in branch.nodes.iter().enumerate() {
                    work += 1;
                    let is_via = index > 0
                        && branch.nodes[index - 1].cell == node.cell
                        && branch.nodes[index - 1].layer != node.layer;
                    if !is_via && node.layer as usize != pad.layer {
                        continue;
                    }
                    let point = self.grid.center_of(node.cell as usize);
                    if point[0] < x0 || point[0] > x1 || point[1] < y0 || point[1] > y1 {
                        continue;
                    }
                    let reach = pad.half_width
                        + clearance
                        + if is_via {
                            rules.via_diameter / 2.0
                        } else {
                            self.width_on(self.node_class(net, node.cell), pad.layer) / 2.0
                        };
                    for (run_index, run) in &runs {
                        if crate::geometry::point_segment_distance(point, run.start, run.end)
                            < reach
                        {
                            match blocks
                                .iter_mut()
                                .find(|(r, n, _)| r == run_index && *n == net)
                            {
                                Some(entry) => entry.2 |= is_via,
                                None => blocks.push((*run_index, net, is_via)),
                            }
                        }
                    }
                }
            }
        }
        work
    }

    /// The pads of `dive`'s row of pad `index` within `ROW_REACH` along the
    /// row, of other nets, nearest first.
    fn row_neighbours(&self, dive: &DivePlan, index: usize) -> Vec<usize> {
        let pad = &dive.plan.pads[index];
        let Some((part, outward)) = &pad.row else {
            return Vec::new();
        };
        let Some(members) = dive.by_row.get(&(part.clone(), pad.layer, *outward)) else {
            return Vec::new();
        };
        let here = along_row(*outward, self.pad_anchor(pad));
        let mut near: Vec<(f64, usize)> = members
            .iter()
            .copied()
            .filter(|other| dive.plan.pads[*other].net != pad.net)
            .map(|other| {
                let at = along_row(*outward, self.pad_anchor(&dive.plan.pads[other]));
                ((at - here).abs(), other)
            })
            .filter(|(distance, _)| *distance <= ROW_REACH)
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        near.into_iter().map(|(_, other)| other).collect()
    }

    /// Whether the targets of the pads `a` and `b` of one row lie on
    /// opposite sides of each other along the row (their tracks, leaving
    /// on one layer side by side, would cross).
    fn targets_cross(&self, a: &DensePad, b: &DensePad) -> bool {
        let (Some((_, outward)), Some(target_a), Some(target_b)) = (&a.row, a.target, b.target)
        else {
            return false;
        };
        let pads =
            along_row(*outward, self.pad_anchor(a)) - along_row(*outward, self.pad_anchor(b));
        let targets = along_row(*outward, target_a) - along_row(*outward, target_b);
        pads * targets < -1.0e-9
    }

    /// The crossing pairs of `net` (its open dense pads, nearest neighbour
    /// first) and the work of finding them (nodes looked at).
    pub(super) fn crossing_pairs(&self, dive: &DivePlan, net: NetId) -> (Vec<CrossingPair>, u64) {
        let mut pairs = Vec::new();
        let mut work = 0;
        for &open in dive.by_net.get(&net).map_or(&[][..], Vec::as_slice) {
            let pad = &dive.plan.pads[open];
            if !self.pad_open(pad) {
                continue;
            }
            for other in self.row_neighbours(dive, open) {
                let neighbour = &dive.plan.pads[other];
                if !self.targets_cross(pad, neighbour) {
                    continue;
                }
                let mut blocks = Vec::new();
                work += self.tip_blocks(pad, &mut std::iter::once(neighbour.net), &mut blocks);
                if !blocks.is_empty() {
                    pairs.push(CrossingPair { open, other });
                }
            }
        }
        (pairs, work)
    }

    /// `PCB_ROUTER_TROUBLE_WHY`: every open dense pad (`pad_open`) of a signal net still open,
    /// its straight ways out and what closes each (the net, by a via or a
    /// track, and whether that net has a pad in the row whose target
    /// crosses this one's: a crossing pair); open nets without a dense pad
    /// are named too. Analysis only: no work is charged.
    pub(super) fn crossing_census(&self, dive: &DivePlan, order: &[NetId], label: &str) {
        let (mut open_nets, mut pads, mut boxed, mut paired, mut no_way) = (0, 0, 0, 0, 0);
        let mut without = 0;
        for &net in order {
            let state = &self.nets[net as usize];
            if !state.routable || !state.plane.is_empty() || state.complete {
                continue;
            }
            open_nets += 1;
            let name = &self.board.nets[net as usize].name;
            let indices = dive.by_net.get(&net).map_or(&[][..], Vec::as_slice);
            if !indices
                .iter()
                .any(|index| self.pad_open(&dive.plan.pads[*index]))
            {
                without += 1;
                eprintln!("crossing census {label}: {name}: open, no open dense pad");
                continue;
            }
            for &index in indices {
                let pad = &dive.plan.pads[index];
                if !self.pad_open(pad) {
                    continue;
                }
                pads += 1;
                let ways: Vec<usize> = pad
                    .runs
                    .iter()
                    .enumerate()
                    .filter(|(_, run)| run.direction % 2 == 0 && !run.via)
                    .map(|(run, _)| run)
                    .collect();
                if ways.is_empty() {
                    no_way += 1;
                    eprintln!(
                        "crossing census {label}: {} ({name}) layer {}: no straight way out",
                        self.pad_label(pad),
                        pad.layer
                    );
                    continue;
                }
                let mut blocks = Vec::new();
                self.tip_blocks(
                    pad,
                    &mut (0..self.nets.len() as NetId).filter(|other| {
                        let other = &self.nets[*other as usize];
                        other.plane.is_empty() || !other.branches.is_empty()
                    }),
                    &mut blocks,
                );
                let closed = ways
                    .iter()
                    .filter(|way| blocks.iter().any(|(run, _, _)| run == *way))
                    .count();
                if closed == ways.len() {
                    boxed += 1;
                }
                let neighbours = self.row_neighbours(dive, index);
                let mut nets: Vec<NetId> = blocks.iter().map(|(_, net, _)| *net).collect();
                nets.sort_unstable();
                nets.dedup();
                let mut pair = false;
                let named: Vec<String> = nets
                    .iter()
                    .map(|other| {
                        let via = blocks.iter().any(|(_, net, via)| net == other && *via);
                        let in_row = neighbours
                            .iter()
                            .find(|neighbour| dive.plan.pads[**neighbour].net == *other);
                        let row = match in_row {
                            Some(neighbour) => {
                                let crossing = self.targets_cross(pad, &dive.plan.pads[*neighbour]);
                                pair |= crossing;
                                format!(
                                    ", row {}{}",
                                    self.pad_label(&dive.plan.pads[*neighbour]),
                                    if crossing { ", crossing" } else { "" }
                                )
                            }
                            None => String::new(),
                        };
                        format!(
                            "{} {}{row}",
                            self.board.nets[*other as usize].name,
                            if via { "via" } else { "track" }
                        )
                    })
                    .collect();
                if pair {
                    paired += 1;
                }
                eprintln!(
                    "crossing census {label}: {} ({name}) layer {}: {closed} of {} ways closed by [{}]{}",
                    self.pad_label(pad),
                    pad.layer,
                    ways.len(),
                    named.join("; "),
                    if pair { "; a crossing pair" } else { "" }
                );
            }
        }
        eprintln!(
            "crossing census {label}: {open_nets} signal nets open ({without} without an open dense pad), {pads} open dense pads of theirs, {boxed} with every straight way closed, {paired} in a crossing pair, {no_way} without a straight way out"
        );
    }

    /// The diver's stub for `pad`'s way out `run` (a via site): the lattice
    /// nodes from the last of the pad's terminal nodes on the run's line to
    /// the site on the pad's layer, and a via there to another layer (the
    /// furthest first) where the branch is legal on the statics.
    fn dive_stub(&self, pad: &DensePad, run: usize) -> Option<Branch> {
        let run = &pad.runs[run];
        let anchor = self.pad_anchor(pad);
        let centre = self.grid.nearest_node(anchor)?;
        let site = self.grid.nearest_node(run.end)?;
        let own: HashSet<Node> = self.nets[pad.net as usize].terminal_nodes[pad.terminal]
            .iter()
            .copied()
            .collect();
        let (dx, dy) = DIRECTIONS[run.direction];
        let (mut x, mut y) = {
            let (x, y) = self.grid.xy(centre);
            (x as i64, y as i64)
        };
        let layer = pad.layer as u8;
        let mut nodes: Vec<Node> = Vec::new();
        for _ in 0..4096 {
            if x < 0 || y < 0 || x >= self.grid.nx as i64 || y >= self.grid.ny as i64 {
                return None;
            }
            let cell = self.grid.index(x as usize, y as usize) as u32;
            let node = Node { layer, cell };
            if own.contains(&node) {
                nodes.clear();
            }
            nodes.push(node);
            if cell as usize == site {
                break;
            }
            x += dx as i64;
            y += dy as i64;
        }
        if nodes.last()?.cell as usize != site || !own.contains(&nodes[0]) || nodes.len() < 2 {
            return None;
        }
        let mut others: Vec<usize> = (0..self.board.layer_count)
            .filter(|other| *other != pad.layer)
            .collect();
        others.sort_by_key(|other| (std::cmp::Reverse(other.abs_diff(pad.layer)), *other));
        for other in others {
            let mut path = nodes.clone();
            path.push(Node {
                layer: other as u8,
                cell: site as u32,
            });
            let branch = Branch {
                nodes: path,
                start_terminal: pad.terminal as u16,
                end_terminal: FREE_END,
            };
            if self.branch_is_legal(pad.net, &branch) {
                return Some(branch);
            }
        }
        None
    }

    /// Removes `net`'s branches that end at its terminal `terminal` and
    /// what hangs on them alone (a junction left without its host); the
    /// fixed branches stay.
    fn drop_pad_branches(&mut self, net: NetId, terminal: usize) {
        let terminal = terminal as u16;
        let state = &self.nets[net as usize];
        let branches = &state.branches;
        let mut keep: Vec<bool> = branches
            .iter()
            .enumerate()
            .map(|(index, branch)| {
                index < state.fixed
                    || (branch.start_terminal != terminal && branch.end_terminal != terminal)
            })
            .collect();
        loop {
            let hosts = junction_hosts(branches, &keep);
            let mut changed = false;
            for (index, branch) in branches.iter().enumerate() {
                if !keep[index] || index < state.fixed {
                    continue;
                }
                let dangling = (branch.start_terminal == NO_TERMINAL
                    && !hosts.contains_key(&branch.nodes[0]))
                    || (branch.end_terminal == NO_TERMINAL
                        && !hosts.contains_key(branch.nodes.last().unwrap()));
                if dangling {
                    keep[index] = false;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut keep = keep.into_iter();
        let state = &mut self.nets[net as usize];
        state.branches.retain(|_| keep.next().unwrap());
        state.connected.fill(false);
        state.complete = false;
        self.stamp(net);
    }

    /// Routes `net` again hard: on its corridors first with `local` (as
    /// the trouble exchange's victims), else in full.
    fn dive_route(&mut self, net: NetId, local: bool, work: &mut TroubleWork) {
        let started = self.budget_work();
        self.nets[net as usize].hopeless_searches = 0;
        if local {
            self.hard_scope = HardScope::Corridors;
        }
        self.route_net_seq(net, 0.0, true, 4.0);
        self.hard_scope = HardScope::Full;
        self.stamp(net);
        work.routed += 1;
        work.victims += self.budget_work() - started;
    }

    /// One trial: the dense pad `diver_pad` dives at its via site `run`,
    /// `stayer` (the other net of the pair) stays on the layer; `trouble`
    /// is the open net the move is for. Returns the nets changed as they
    /// were (`trouble` first) with the board as the trial left it, or,
    /// restored, why not.
    #[allow(clippy::too_many_arguments)]
    fn dive_trial(
        &mut self,
        dive: &DivePlan,
        diver_pad: usize,
        run: usize,
        stayer: NetId,
        trouble: NetId,
        order: &[NetId],
        settings: TroubleSettings,
        work: &mut TroubleWork,
    ) -> Result<Vec<Saved>, String> {
        let pad = dive.plan.pads[diver_pad].clone();
        let diver = pad.net;
        let Some(stub) = self.dive_stub(&pad, run) else {
            return Err("no legal stub to the via site".into());
        };
        let conflicted_before: HashSet<NetId> = order
            .iter()
            .copied()
            .filter(|net| !self.conflicts(*net).is_empty())
            .collect();
        let mut saved = vec![self.save_net(trouble)];
        saved.push(self.save_net(if diver == trouble { stayer } else { diver }));
        let limit = settings.victim_limit(saved[0].4);
        self.drop_pad_branches(diver, pad.terminal);
        self.nets[diver as usize].branches.push(stub.clone());
        self.stamp(diver);
        // The nets the stub overlaps lose the branches it crosses; what its
        // own conflicts still name (a wider class's clearance) goes whole.
        let victims: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| {
                *net != diver
                    && !conflicted_before.contains(net)
                    && !self.conflicts(*net).is_empty()
            })
            .collect();
        let refusal = if victims.len() > limit {
            Some(format!(
                "{} victims, more than allowed ({limit})",
                victims.len()
            ))
        } else {
            victims
                .iter()
                .find(|victim| {
                    !settings.pour_victims && !self.nets[**victim as usize].plane.is_empty()
                })
                .map(|pour| {
                    format!(
                        "a pour net among the victims ({})",
                        self.board.nets[*pour as usize].name
                    )
                })
        };
        if let Some(reason) = refusal {
            self.restore_nets(saved);
            return Err(reason);
        }
        let mut victims = victims;
        for victim in &victims {
            if !saved.iter().any(|entry| entry.0 == *victim) {
                saved.push(self.save_net(*victim));
            }
            self.rip_up_conflicted(*victim);
        }
        if !self.conflicts(diver).is_empty() {
            let others: Vec<NetId> = self
                .blockers(diver)
                .into_iter()
                .filter(|other| *other != diver)
                .collect();
            let pour = others.iter().find(|other| {
                !settings.pour_victims && !self.nets[**other as usize].plane.is_empty()
            });
            if let Some(pour) = pour {
                let reason = format!(
                    "a pour net at the via site ({})",
                    self.board.nets[*pour as usize].name
                );
                self.restore_nets(saved);
                return Err(reason);
            }
            for other in others {
                if !saved.iter().any(|entry| entry.0 == other) {
                    saved.push(self.save_net(other));
                }
                if !victims.contains(&other) {
                    victims.push(other);
                }
                self.rip_up(other);
            }
            if !self.conflicts(diver).is_empty() || victims.len() > limit {
                self.restore_nets(saved);
                return Err("fixed copper at the via site, or too many victims".into());
            }
        }
        // The net that stays on the layer first, then the others displaced,
        // then the diver from its via.
        let mut routed: Vec<NetId> = Vec::new();
        if victims.contains(&stayer) || !self.nets[stayer as usize].complete {
            routed.push(stayer);
        }
        routed.extend(
            victims
                .iter()
                .copied()
                .filter(|victim| *victim != stayer && *victim != diver),
        );
        routed.push(diver);
        for net in &routed {
            self.dive_route(*net, settings.local, work);
        }
        if settings.local {
            let started = self.budget_work();
            for net in &routed {
                let before = saved
                    .iter()
                    .find(|entry| entry.0 == *net)
                    .map_or(0, |entry| entry.4);
                let open = self.open_measure(*net);
                let pair = *net == diver || *net == stayer;
                if open > before || (pair && open > 0) {
                    self.nets[*net as usize].hopeless_searches = 0;
                    self.hard_scope = HardScope::Window;
                    self.route_net_seq(*net, 0.0, true, 1.0);
                    self.hard_scope = HardScope::Full;
                    self.stamp(*net);
                    work.retries += 1;
                }
            }
            work.retried += self.budget_work() - started;
        }
        // Where the diver's route does not take its via (the stayer's
        // reroute freed its way on the layer), the stub goes, or its part
        // beyond the furthest node the route joins: no copper hangs off
        // the pad unused. The trial is judged as it stands.
        let state = &self.nets[diver as usize];
        let joined = state
            .branches
            .iter()
            .filter(|branch| branch.nodes != stub.nodes)
            .flat_map(|branch| [branch.nodes[0], *branch.nodes.last().unwrap()])
            .filter_map(|node| stub.nodes.iter().position(|stub_node| *stub_node == node))
            .max();
        if joined != Some(stub.nodes.len() - 1) {
            let state = &mut self.nets[diver as usize];
            if let Some(at) = state
                .branches
                .iter()
                .position(|branch| branch.nodes == stub.nodes)
            {
                match joined {
                    Some(last) if last > 0 => state.branches[at].nodes.truncate(last + 1),
                    _ => {
                        state.branches.remove(at);
                    }
                }
                self.stamp(diver);
            }
        }
        // A conflict one side of a pair of classes sees (as in the
        // exchange): the nets in conflict lose those branches and are
        // routed again hard, once.
        let conflicted: Vec<NetId> = saved
            .iter()
            .map(|entry| entry.0)
            .filter(|net| !self.conflicts(*net).is_empty())
            .collect();
        for net in conflicted {
            self.rip_up_conflicted(net);
            self.dive_route(net, false, work);
        }
        Ok(saved)
    }

    /// The dives for the crossing pairs of the open net `trouble`: each
    /// pair's two assignments at each of the diver's via sites
    /// (`DIVE_TRIALS` at most, within the pass's work), the best kept (see
    /// the module). Returns what was done and why.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn crossing_dives(
        &mut self,
        dive: &DivePlan,
        trouble: NetId,
        order: &[NetId],
        settings: TroubleSettings,
        clock: &(f64, std::time::Instant),
        budget: f64,
        work: &mut TroubleWork,
    ) -> (Exchange, String) {
        let started = self.budget_work();
        let (pairs, found) = self.crossing_pairs(dive, trouble);
        self.scratch.expansions += found.div_ceil(8);
        if pairs.is_empty() {
            work.dive_work += self.budget_work() - started;
            return (Exchange::default(), "no crossing pair".into());
        }
        let name = |router: &Self, pad: usize| {
            let pad = &dive.plan.pads[pad];
            format!(
                "{} ({})",
                router.pad_label(pad),
                router.board.nets[pad.net as usize].name
            )
        };
        let mut trials = 0;
        let mut reasons: Vec<String> = Vec::new();
        // The best trial: open after, the nets as they were, as it left them, and what it was.
        let mut best: Option<(usize, Vec<Saved>, Vec<Saved>, String)> = None;
        'pairs: for pair in pairs {
            let (open_net, other_net) = (
                dive.plan.pads[pair.open].net,
                dive.plan.pads[pair.other].net,
            );
            for (diver_pad, stayer) in [(pair.open, other_net), (pair.other, open_net)] {
                let sites: Vec<usize> = dive.plan.pads[diver_pad]
                    .runs
                    .iter()
                    .enumerate()
                    .filter(|(_, run)| run.via)
                    .map(|(index, _)| index)
                    .collect();
                if sites.is_empty() {
                    reasons.push(format!("{} has no via site", name(self, diver_pad)));
                }
                for run in sites {
                    if trials >= DIVE_TRIALS {
                        break 'pairs;
                    }
                    if self.spent(clock, budget) {
                        reasons.push("out of work".into());
                        break 'pairs;
                    }
                    trials += 1;
                    work.dive_trials += 1;
                    let site = dive.plan.pads[diver_pad].runs[run].end;
                    let what = format!(
                        "{} dives at ({:.2}, {:.2})",
                        name(self, diver_pad),
                        site[0],
                        site[1]
                    );
                    match self
                        .dive_trial(dive, diver_pad, run, stayer, trouble, order, settings, work)
                    {
                        Ok(saved) => {
                            let before = open_saved(&saved);
                            let after = open_now(self, &saved);
                            let is_clean = clean(self, &saved);
                            reasons.push(format!(
                                "{what}: {before} open to {after}, clean {is_clean}"
                            ));
                            if is_clean
                                && after < before
                                && best.as_ref().is_none_or(|(open, ..)| after < *open)
                            {
                                let left: Vec<Saved> =
                                    saved.iter().map(|entry| self.save_net(entry.0)).collect();
                                best = Some((after, saved.clone(), left, what));
                            }
                            self.restore_nets(saved);
                        }
                        Err(reason) => reasons.push(format!("{what}: {reason}")),
                    }
                }
            }
        }
        let outcome = match best {
            Some((after, saved, left, what)) => {
                let before = open_saved(&saved);
                self.restore_nets(left);
                if open_now(self, &saved) == after && clean(self, &saved) {
                    work.dives_kept += 1;
                    (
                        Exchange {
                            kept: true,
                            closed: before - after,
                            displaced: saved.len() - 1,
                        },
                        format!(
                            "kept: {what}, {before} open to {after} ({} nets changed); {}",
                            saved.len(),
                            reasons.join("; ")
                        ),
                    )
                } else {
                    self.restore_nets(saved);
                    (
                        Exchange::default(),
                        format!("refused: {what} did not replay; {}", reasons.join("; ")),
                    )
                }
            }
            None => (
                Exchange::default(),
                format!("refused: {}", reasons.join("; ")),
            ),
        };
        work.dive_work += self.budget_work() - started;
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Net, Obstacle, ObstacleKind, RuleClass, Terminal};
    use crate::geometry::Shape;

    fn obstacle(
        shape: Shape,
        net: Option<NetId>,
        kind: ObstacleKind,
        layers: u32,
        label: &str,
    ) -> Obstacle {
        Obstacle {
            shape,
            layers,
            kind,
            net,
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: label.into(),
        }
    }

    fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
        Shape::Polygon {
            points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        }
    }

    /// The pad of the row whose net is open (U1.2, net N2) and its crossing
    /// neighbour's (U1.3, net N3).
    const OPEN: NetId = 2;
    const OTHER: NetId = 3;

    /// Two layers, a row of six 0.3 mm pads at 0.5 mm pitch on the top
    /// (U1.0 to U1.5 at x 4.0 to 6.5, nets N0 to N5), each net to a pad at
    /// the top wall in the row's order but N2 and N3, whose targets
    /// cross (N2's at x 6.5, N3's at 4.0). Walls on both layers below the
    /// row and above the far pads: every net's track splits the space
    /// between them, so N2 and N3 cannot both stay on the top. With
    /// `enclosed`, N2's far pad is walled in on both layers (no route
    /// reaches it).
    fn crossing_row(enclosed: bool) -> Router {
        let class = RuleClass {
            trace_width: 0.2,
            clearance: 0.2,
            via_diameter: 0.6,
            via_drill: 0.3,
            layers: 0,
        };
        let targets = [1.0, 2.5, 6.5, 4.0, 8.0, 9.5];
        let mut obstacles = Vec::new();
        let mut nets = Vec::new();
        let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal {
            anchor: at,
            layers: 1,
            pad,
            contact: None,
            label: label.into(),
        };
        for (index, far) in targets.iter().enumerate() {
            let x = 4.0 + 0.5 * index as f64;
            let net = index as NetId;
            obstacles.push(obstacle(
                rectangle(x - 0.15, 4.5, x + 0.15, 5.5),
                Some(net),
                ObstacleKind::Copper,
                1,
                &format!("U1.{index}"),
            ));
            let row = obstacles.len() - 1;
            obstacles.push(obstacle(
                Shape::Circle {
                    center: [*far, 9.0],
                    radius: 0.2,
                },
                Some(net),
                ObstacleKind::Copper,
                1,
                &format!("J{index}.1"),
            ));
            nets.push(Net {
                name: format!("N{index}"),
                class: 0,
                terminals: vec![
                    terminal(row, [x, 5.0], &format!("U1.{index}")),
                    terminal(row + 1, [*far, 9.0], &format!("J{index}.1")),
                ],
            });
        }
        // The part's other row, under the wall (no routes there): the part's
        // centre lies below the row, so the row leaves upwards.
        for index in 0..6 {
            let x = 4.0 + 0.5 * index as f64;
            obstacles.push(obstacle(
                rectangle(x - 0.15, 0.5, x + 0.15, 1.5),
                Some(nets.len() as NetId),
                ObstacleKind::Copper,
                1,
                &format!("U1.{}", index + 6),
            ));
            nets.push(Net {
                name: format!("D{index}"),
                class: 0,
                terminals: vec![terminal(
                    obstacles.len() - 1,
                    [x, 1.0],
                    &format!("U1.{}", index + 6),
                )],
            });
        }
        let mut walls = vec![(0.0, 0.0, 11.0, 4.3), (0.0, 9.4, 11.0, 10.0)];
        if enclosed {
            walls.extend([
                (5.8, 8.0, 6.05, 9.4),
                (6.95, 8.0, 7.2, 9.4),
                (5.8, 8.0, 7.2, 8.55),
            ]);
        }
        for (x0, y0, x1, y1) in walls {
            obstacles.push(obstacle(
                rectangle(x0, y0, x1, y1),
                None,
                ObstacleKind::Keepout,
                3,
                "wall",
            ));
        }
        let board = Board {
            layer_count: 2,
            outline: vec![[0.0, 0.0], [11.0, 0.0], [11.0, 10.0], [0.0, 10.0]],
            edge_clearance: 0.1,
            hole_clearance: 0.25,
            hole_to_hole: 0.25,
            classes: vec![class],
            neck_width: 0.2,
            obstacles,
            nets,
            planes: Vec::new(),
            solid_pads: Vec::new(),
            isolated_pads: Vec::new(),
            solid_pad_obstacles: Vec::new(),
            isolated_pad_obstacles: Vec::new(),
            pad_spokes: Default::default(),
            merged_pads: Vec::new(),
            no_via_pads: Vec::new(),
            fill_passes: Vec::new(),
            pin_row_gaps: Vec::new(),
            terminal_copper: Vec::new(),
            attachment: Default::default(),
        };
        let config = Config {
            pitches: vec![0.1],
            crossing_dive: true,
            ..Config::default()
        };
        Router::new(&board, &config)
    }

    /// Lattice nodes on the top from `points[0]` through the others, by
    /// axis and diagonal steps.
    fn walk(router: &Router, points: &[[f64; 2]]) -> Vec<Node> {
        let mut at = router.grid.xy(router.grid.nearest_node(points[0]).unwrap());
        let mut nodes = vec![Node {
            layer: 0,
            cell: router.grid.index(at.0, at.1) as u32,
        }];
        for point in &points[1..] {
            let to = router.grid.xy(router.grid.nearest_node(*point).unwrap());
            while at != to {
                at = (
                    (at.0 as i64 + (to.0 as i64 - at.0 as i64).signum()) as usize,
                    (at.1 as i64 + (to.1 as i64 - at.1 as i64).signum()) as usize,
                );
                nodes.push(Node {
                    layer: 0,
                    cell: router.grid.index(at.0, at.1) as u32,
                });
            }
        }
        nodes
    }

    /// The row routed as the negotiation might leave it: N3 leaves U1.3 up
    /// and turns left across U1.2's tip (0.4 mm beyond it) to its far pad,
    /// the outer nets are routed hard, and N2, boxed in, stays open.
    fn routed_with_n2_open(router: &mut Router) {
        let nodes = walk(
            router,
            &[[5.5, 5.0], [5.5, 5.9], [5.0, 5.9], [4.0, 6.9], [4.0, 9.0]],
        );
        let branch = Branch {
            nodes,
            start_terminal: 0,
            end_terminal: 1,
        };
        assert!(router.branch_is_legal(OTHER, &branch));
        let state = &mut router.nets[OTHER as usize];
        state.branches = vec![branch];
        state.connected = vec![true, true];
        state.complete = true;
        router.stamp(OTHER);
        for net in [1, 0, 4, 5, OPEN] {
            router.route_net_seq(net, 0.0, true, 4.0);
            router.stamp(net);
        }
        for net in [0, 1, 3, 4, 5] {
            assert!(router.nets[net].complete, "N{net} routed");
        }
        assert!(!router.nets[OPEN as usize].complete, "N2 boxed in");
        for net in 0..6 {
            assert!(router.conflicts(net).is_empty());
        }
    }

    fn vias(router: &Router, net: NetId) -> Vec<crate::geometry::Point> {
        router.nets[net as usize]
            .branches
            .iter()
            .flat_map(|branch| {
                branch
                    .nodes
                    .windows(2)
                    .filter(|pair| pair[0].cell == pair[1].cell)
                    .map(|pair| router.grid.center_of(pair[0].cell as usize))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn dive(router: &mut Router) -> (Exchange, String, usize) {
        let plan = DivePlan::new(router.fanout_plan());
        router.crossing_census(&plan, &[0, 1, 2, 3, 4, 5], "test");
        let (pairs, _) = router.crossing_pairs(&plan, OPEN);
        let order: Vec<NetId> = (0..6).collect();
        let settings = TroubleSettings::from_env();
        let clock = router.clock();
        let mut work = TroubleWork::default();
        let (exchange, reason) =
            router.crossing_dives(&plan, OPEN, &order, settings, &clock, 1.0e9, &mut work);
        (exchange, reason, pairs.len())
    }

    /// N2 and N3 cross at the row: the move finds the pair, one of the two
    /// takes a via beyond the row's tips (the other none), and every net is
    /// complete without a conflict.
    #[test]
    fn a_crossing_pair_is_resolved_by_one_dive() {
        let mut router = crossing_row(false);
        routed_with_n2_open(&mut router);
        let (exchange, reason, pairs) = dive(&mut router);
        assert_eq!(pairs, 1, "{reason}");
        assert!(exchange.kept, "{reason}");
        assert_eq!(exchange.closed, 1, "{reason}");
        for net in 0..6 {
            assert!(router.nets[net].complete, "N{net}: {reason}");
            assert!(
                router.conflicts(net as NetId).is_empty(),
                "N{net}: {reason}"
            );
        }
        let (open_vias, other_vias) = (vias(&router, OPEN), vias(&router, OTHER));
        assert!(
            open_vias.is_empty() != other_vias.is_empty(),
            "exactly one dives: {open_vias:?} {other_vias:?}; {reason}"
        );
        // The diver's first via lies beyond the row's tips (5.5) by more
        // than a via's radius and clearance.
        let first = open_vias.first().or(other_vias.first()).unwrap();
        assert!(first[1] > 6.0, "{first:?}; {reason}");
    }

    /// The control: N2's far pad is walled in, so no dive closes it; the
    /// trials run and the board is as it was, branch for branch.
    #[test]
    fn a_dive_that_closes_nothing_keeps_the_state() {
        let mut router = crossing_row(true);
        routed_with_n2_open(&mut router);
        let before: Vec<Vec<Vec<Node>>> = router
            .nets
            .iter()
            .map(|state| state.branches.iter().map(|b| b.nodes.clone()).collect())
            .collect();
        let complete: Vec<bool> = router.nets.iter().map(|state| state.complete).collect();
        let (exchange, reason, pairs) = dive(&mut router);
        assert_eq!(pairs, 1, "{reason}");
        assert!(!exchange.kept, "{reason}");
        assert!(reason.contains("dives at"), "trials ran: {reason}");
        let after: Vec<Vec<Vec<Node>>> = router
            .nets
            .iter()
            .map(|state| state.branches.iter().map(|b| b.nodes.clone()).collect())
            .collect();
        assert_eq!(before, after);
        let now: Vec<bool> = router.nets.iter().map(|state| state.complete).collect();
        assert_eq!(complete, now);
        for net in 0..6 {
            assert!(router.conflicts(net).is_empty());
        }
    }
}

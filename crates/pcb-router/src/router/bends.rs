// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The bend round: jogs and staircases taken out by rerouting.
//!
//! The negotiation leaves sidesteps in the middle of runs: a 45-degree
//! jog costs two `bend_cost` (0.1 mm per 45 degrees) and 0.04 mm of
//! length, so any history or occupancy above about 0.3 mm on a lane moves
//! a track over and back. The straightening only tries the shortest
//! octilinear paths between a run's corners. This round, after the via
//! reduction, reroutes each complete signal net with a jog or a staircase
//! alone against all other copper (the clean-up's search: no history, no
//! layer preference, exact) at `bend_round_cost` per 45 degrees, and keeps
//! the new route only where the net stays complete and out of conflict,
//! has no via more, is at most `bend_round_length` longer and has fewer
//! jogs and staircases (`keep_bend_round`). A renegotiation of those nets
//! at the raised cost (the via reduction's twin) was measured first and
//! is no use: every round grew the vias (docs/router.md, "The bend round").

use super::straighten::{direction_of, jogs_and_staircases};
use super::*;

/// The bend round checks the pour and spoke counts after this many
/// reroutes kept (each check is a pour analysis of every pour net).
const BEND_ROUND_CHECK: usize = 8;

/// What the bend round compares before and after, for a net or a board.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct BendTally {
    /// Open terminals and conflicted nets (`quality`; for a net 1 if it
    /// is incomplete or in conflict).
    pub open: usize,
    pub vias: usize,
    /// Lattice length (mm).
    pub length: f64,
    pub jogs: usize,
    pub staircases: usize,
    /// Pin-row crossings: entries into another net's pin-row gap (every
    /// gap, `cleanup_only` too; `row_crossings`).
    pub crossings: usize,
}

/// Whether a reroute of the bend round is kept: nothing opens, no via is
/// added, the length grows by at most `length_share` of what it was, no
/// staircase and no pin-row crossing is added and the jogs and staircases
/// together fall.
pub(super) fn keep_bend_round(before: &BendTally, after: &BendTally, length_share: f64) -> bool {
    after.open <= before.open
        && after.vias <= before.vias
        && after.crossings <= before.crossings
        && after.length <= before.length * (1.0 + length_share) + 1.0e-9
        && after.staircases <= before.staircases
        && after.jogs + after.staircases < before.jogs + before.staircases
}

/// What the bend round did, for the verbose line.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct BendRound {
    pub nets: usize,
    pub tried: usize,
    pub kept: usize,
    /// Reroutes taken back for stranding pour pads or starving thermals.
    pub taken_back: usize,
    pub work: f64,
    pub budget: f64,
}

impl Router {
    /// The jogs and staircases of a branch's runs (one layer each),
    /// summed.
    pub(super) fn branch_jogs(&self, branch: &Branch) -> (usize, usize) {
        let nx = self.grid.nx as i64;
        let mut directions = Vec::new();
        let (mut jogs, mut staircases) = (0, 0);
        let mut flush = |directions: &mut Vec<u8>| {
            let (j, s) = jogs_and_staircases(directions, self.grid.pitch);
            jogs += j;
            staircases += s;
            directions.clear();
        };
        for pair in branch.nodes.windows(2) {
            let (a, b) = (pair[0].cell as i64, pair[1].cell as i64);
            match direction_of(b % nx - a % nx, b / nx - a / nx) {
                Some(direction) if pair[0].layer == pair[1].layer => directions.push(direction),
                _ => flush(&mut directions),
            }
        }
        flush(&mut directions);
        (jogs, staircases)
    }

    /// A net's pin-row crossings: the times its branches enter another
    /// net's pin-row gap (a run of nodes in one gap is one).
    pub(super) fn row_crossings(&self, net: NetId) -> usize {
        self.row_crossings_of(net, &self.nets[net as usize].branches)
    }

    /// `row_crossings` of `net` with these branches.
    pub(super) fn row_crossings_of(&self, net: NetId, branches: &[Branch]) -> usize {
        let mut crossings = 0;
        for branch in branches {
            let mut last = None;
            for node in &branch.nodes {
                let gap = self.pin_row_foreign_entry(net, node.layer as usize, node.cell as usize);
                if gap.is_some() && gap != last {
                    crossings += 1;
                }
                last = gap;
            }
        }
        crossings
    }

    /// The jogs and staircases of a net's branches.
    pub(super) fn net_jogs(&self, net: NetId) -> (usize, usize) {
        self.nets[net as usize]
            .branches
            .iter()
            .map(|branch| self.branch_jogs(branch))
            .fold((0, 0), |sum, (j, s)| (sum.0 + j, sum.1 + s))
    }

    /// The nets the bend round reroutes: the complete signal nets (no
    /// pour: its stubs and skeleton hold the pour's pads) with a jog or a
    /// staircase outside their fixed branches, the most per millimetre of
    /// the net first (a cut round takes those).
    pub(super) fn bend_round_nets(&self, order: &[NetId]) -> Vec<NetId> {
        let mut nets: Vec<(NetId, f64)> = order
            .iter()
            .copied()
            .filter_map(|net| {
                let state = &self.nets[net as usize];
                if !state.routable || !state.complete || !state.plane.is_empty() {
                    return None;
                }
                let steps: usize = state.branches[state.fixed..]
                    .iter()
                    .map(|branch| {
                        let (jogs, staircases) = self.branch_jogs(branch);
                        jogs + staircases
                    })
                    .sum();
                (steps > 0).then(|| (net, steps as f64 / self.net_tally(net).length.max(1.0)))
            })
            .collect();
        // The densest first: a reroute's work grows with the net's
        // length, its gain with the steps (sorted by the count alone, the
        // long nets took mackerel-30-proto's budget: 12 kept of 40).
        nets.sort_by(|a, b| b.1.total_cmp(&a.1));
        nets.into_iter().map(|(net, _)| net).collect()
    }

    /// The board's tally.
    pub(super) fn bend_tally(&self) -> BendTally {
        let (open, vias, length) = self.quality();
        let (mut jogs, mut staircases, mut crossings) = (0, 0, 0);
        for (net, state) in self.nets.iter().enumerate() {
            if !state.routable {
                continue;
            }
            let (j, s) = self.net_jogs(net as NetId);
            jogs += j;
            staircases += s;
            crossings += self.row_crossings(net as NetId);
        }
        BendTally {
            open,
            vias,
            length,
            jogs,
            staircases,
            crossings,
        }
    }

    /// One net's tally (open: 1 if incomplete or in conflict).
    pub(super) fn net_tally(&self, net: NetId) -> BendTally {
        let state = &self.nets[net as usize];
        let nx = self.grid.nx as i64;
        let (mut vias, mut length) = (0, 0.0);
        for branch in &state.branches {
            for pair in branch.nodes.windows(2) {
                if pair[0].cell == pair[1].cell {
                    vias += 1;
                } else {
                    let dx = (pair[0].cell as i64 % nx - pair[1].cell as i64 % nx).abs();
                    let dy = (pair[0].cell as i64 / nx - pair[1].cell as i64 / nx).abs();
                    length += self.grid.pitch * ((dx * dx + dy * dy) as f64).sqrt();
                }
            }
        }
        let (jogs, staircases) = self.net_jogs(net);
        BendTally {
            open: usize::from(!state.complete || !self.conflicts(net).is_empty()),
            vias,
            length,
            jogs,
            staircases,
            crossings: self.row_crossings(net),
        }
    }

    /// Per pour net (`pour_net_ids`), its pads off the main piece and its
    /// spoke islands: what the stitching adds vias for (`stitch_pours`;
    /// `pour_stranded` counts the pads alone: with it, Castor's kept
    /// reroutes took the board from 240 to 244 vias at the stitching).
    pub(super) fn pour_islands(&self) -> Vec<usize> {
        self.pour_net_ids()
            .into_iter()
            .map(|net| {
                let (pours, mut parent, main, spoke_islands) =
                    self.analyze_pours_full(net, &|_| false);
                let terminal_base = pours.pieces + 1;
                (0..self.nets[net as usize].terminal_nodes.len())
                    .filter(|terminal| find(&mut parent, terminal_base + terminal) != main)
                    .count()
                    + spoke_islands.len()
            })
            .collect()
    }

    /// A net's branches put back as they were (complete, stamped).
    pub(super) fn put_back(&mut self, net: NetId, branches: Vec<Branch>) {
        let state = &mut self.nets[net as usize];
        state.branches = branches;
        state.connected.fill(true);
        state.complete = true;
        self.stamp(net);
    }

    /// The bend round (see the module's comment), once the board is
    /// complete, within `bend_round_share` of the route's work so far (and
    /// a third of what is left before the work deadline). Every
    /// `BEND_ROUND_CHECK` reroutes kept and at the end, a pour net with
    /// more islands (`pour_islands`) or more pads the spoke model finds
    /// starved than before the round takes back the reroute that grew it
    /// and all after it; if the board's tally then fails
    /// `keep_bend_round`, all go back.
    pub(super) fn reduce_bends(&mut self, order: &[NetId]) -> BendRound {
        let mut round = BendRound::default();
        if !self.config.bend_round || self.config.bend_round_cost <= self.config.bend_cost {
            return round;
        }
        let before = self.bend_tally();
        if before.open > 0 || before.jogs + before.staircases == 0 {
            return round;
        }
        let route_work = self.budget_work();
        let mut budget = self.config.bend_round_share * route_work;
        if let Some(limit) = self.config.deadline_work {
            budget = budget.min((limit - self.deadline_used()).max(0.0) / 3.0);
        }
        round.budget = budget;
        let started = self.clock();
        if budget <= 0.0 || self.spent(&started, budget) || self.past_wall_deadline() {
            return round;
        }
        let pending = self.bend_round_nets(order);
        round.nets = pending.len();
        if pending.is_empty() {
            return round;
        }
        let checked = self.budget_work();
        let stranded_before = if self.config.pour_aware_polish {
            self.pour_islands()
        } else {
            Vec::new()
        };
        let starved_before = self.starved_pads();
        // The reroutes stop where what is left would not pay for a check
        // and its halving (about four checks).
        let check = self.budget_work() - checked;
        let reroutes = budget - 4.0 * check;
        let (was_cleanup, bend_cost) = (self.cleanup, self.config.bend_cost);
        self.cleanup = true;
        self.cleanup_rows = true;
        self.config.bend_cost = self.config.bend_round_cost;
        // The reroutes kept, in order: (net, old branches, new branches).
        // Each was routed against the ones before it, so a prefix of them
        // is a state the round passed through; taking back one from the
        // middle would leave the later ones routed around copper that is
        // not there and over copper that is back (0xCB panel: two nets in
        // conflict).
        let mut changed: Vec<(NetId, Vec<Branch>, Vec<Branch>)> = Vec::new();
        // The reroutes up to here strand no pour pad and starve no
        // thermal more than before the round.
        let mut settled = 0;
        let grown = |router: &Self| {
            router
                .pour_islands()
                .iter()
                .zip(&stranded_before)
                .any(|(after, before)| after > before)
                || router.starved_pads() > starved_before
        };
        let mut pending = pending.iter().copied();
        loop {
            let net = pending.next();
            if let Some(net) = net.filter(|_| !self.spent(&started, reroutes)) {
                round.tried += 1;
                let net_before = self.net_tally(net);
                let saved = self.nets[net as usize].branches.clone();
                self.rip_up(net);
                self.route_net_seq(net, 0.0, true, 1.0);
                self.stamp(net);
                let net_after = self.net_tally(net);
                if keep_bend_round(&net_before, &net_after, self.config.bend_round_length) {
                    let new = self.nets[net as usize].branches.clone();
                    changed.push((net, saved, new));
                } else {
                    self.put_back(net, saved);
                }
                if changed.len() < settled + BEND_ROUND_CHECK {
                    continue;
                }
            }
            // The pour and spoke counts every `BEND_ROUND_CHECK` reroutes
            // kept and at the end: if one grew, the shortest prefix that
            // grows it ends with the culprit (found by halving); it and
            // what came after go back, last first, and the round goes on
            // from there.
            if changed.len() > settled && grown(self) {
                let (mut low, mut high) = (settled, changed.len());
                while high - low > 1 {
                    let middle = (low + high) / 2;
                    for (index, (net, old, new)) in changed.iter().enumerate().skip(settled).rev() {
                        let branches = if index < middle { new } else { old };
                        if self.nets[*net as usize].branches != *branches {
                            self.put_back(*net, branches.clone());
                        }
                    }
                    if grown(self) {
                        high = middle;
                    } else {
                        low = middle;
                    }
                }
                while changed.len() > low {
                    let (net, old, _) = changed.pop().unwrap();
                    self.put_back(net, old);
                    round.taken_back += 1;
                }
                for (net, _, new) in &changed[settled..] {
                    if self.nets[*net as usize].branches != *new {
                        self.put_back(*net, new.clone());
                    }
                }
            }
            settled = changed.len();
            if net.is_none() || self.spent(&started, reroutes) {
                break;
            }
        }
        self.cleanup = was_cleanup;
        self.cleanup_rows = false;
        self.config.bend_cost = bend_cost;
        let after = self.bend_tally();
        if !changed.is_empty() && !keep_bend_round(&before, &after, self.config.bend_round_length) {
            round.taken_back += changed.len();
            while let Some((net, old, _)) = changed.pop() {
                self.put_back(net, old);
            }
        }
        round.kept = changed.len();
        round.work = self.budget_work() - started.0;
        if self.config.verbose {
            let after = self.bend_tally();
            eprintln!(
                "bend round at {:.2} mm: {} nets with jogs, {} rerouted, {} kept, {} taken back; open {} -> {}, vias {} -> {}, length {:.1} -> {:.1} mm, jogs {} -> {}, staircases {} -> {}, pin-row crossings {} -> {}; {:.1} s of work of a budget of {:.1} s ({:.1} s of the route's work before it)",
                self.config.bend_round_cost,
                round.nets,
                round.tried,
                round.kept,
                round.taken_back,
                before.open,
                after.open,
                before.vias,
                after.vias,
                before.length,
                after.length,
                before.jogs,
                after.jogs,
                before.staircases,
                after.staircases,
                before.crossings,
                after.crossings,
                round.work,
                round.budget,
                route_work,
            );
        }
        round
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(open: usize, vias: usize, length: f64, jogs: usize, staircases: usize) -> BendTally {
        BendTally {
            open,
            vias,
            length,
            jogs,
            staircases,
            crossings: 2,
        }
    }

    #[test]
    fn a_bend_round_is_kept_only_when_it_takes_steps_out_and_costs_nothing_else() {
        let before = tally(0, 10, 1000.0, 20, 5);
        // Fewer jogs, the same vias, 0.5 % longer: kept.
        assert!(keep_bend_round(&before, &tally(0, 10, 1005.0, 15, 5), 0.01));
        // Fewer staircases, a jog more (a staircase became a jog): kept.
        assert!(keep_bend_round(&before, &tally(0, 9, 990.0, 21, 3), 0.01));
        // A connection opens.
        assert!(!keep_bend_round(&before, &tally(1, 10, 990.0, 10, 0), 0.01));
        // A via more.
        assert!(!keep_bend_round(&before, &tally(0, 11, 990.0, 10, 0), 0.01));
        // 2 % longer.
        assert!(!keep_bend_round(
            &before,
            &tally(0, 10, 1020.0, 10, 0),
            0.01
        ));
        // A staircase more, whatever the jogs.
        assert!(!keep_bend_round(&before, &tally(0, 10, 990.0, 5, 6), 0.01));
        // A pin-row crossing more, whatever the steps.
        let crossing = BendTally {
            crossings: 3,
            ..tally(0, 10, 990.0, 10, 0)
        };
        assert!(!keep_bend_round(&before, &crossing, 0.01));
        // One fewer: kept.
        let fewer = BendTally {
            crossings: 1,
            ..tally(0, 10, 990.0, 10, 0)
        };
        assert!(keep_bend_round(&before, &fewer, 0.01));
        // No fewer steps in all.
        assert!(!keep_bend_round(&before, &tally(0, 10, 990.0, 21, 4), 0.01));
        assert!(!keep_bend_round(&before, &before, 0.01));
    }

    #[test]
    fn a_jog_is_a_chain_of_two_returning_bends_and_a_staircase_of_three_or_more() {
        // East, one step north-east, east: a jog.
        assert_eq!(jogs_and_staircases(&[0, 0, 0, 1, 0, 0, 0], 0.1), (1, 0));
        // East, north-east, east, north-east, east (each within 2 mm): a
        // staircase of four bends.
        assert_eq!(jogs_and_staircases(&[0, 0, 1, 0, 0, 1, 0, 0], 0.1), (0, 1));
        // A corner cut at 45 degrees and a plain corner: neither.
        assert_eq!(jogs_and_staircases(&[0, 0, 1, 1, 2, 2], 0.1), (0, 0));
        assert_eq!(jogs_and_staircases(&[0, 0, 2, 2], 0.1), (0, 0));
        // Two sidesteps 3 mm apart: two jogs.
        let mut apart = vec![0, 0, 1, 0];
        apart.extend([0; 30]);
        apart.extend([7, 0, 0]);
        assert_eq!(jogs_and_staircases(&apart, 0.1), (2, 0));
        // A sidestep 2.5 mm long is no jog (its bends are too far apart).
        let mut wide = vec![0, 0];
        wide.extend([1; 18]);
        wide.extend([0, 0]);
        assert_eq!(jogs_and_staircases(&wide, 0.1), (0, 0));
    }
}

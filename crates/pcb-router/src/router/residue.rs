// Copyright (C) 2026 Toit contributors.

//! The residue renegotiated in a window (`Config::residue_window`): after
//! the hard repair and the trouble-makers' repair, the connections still
//! open (at most `RESIDUE_MAX_NETS` nets) are taken up together where
//! they are. Per window (the box of an open net's open pads and the pads
//! of its tree nearest them, grown by a margin; overlapping boxes merged)
//! every net with copper in the box loses the branches that enter it
//! (signal nets and nets routed as tracks alike, the supplies of a
//! pours-as-tracks attempt among them; a pour net's copper stays), the
//! negotiation's history inside the box is reset, and those nets with the
//! open ones are negotiated again jointly (`negotiate_from`, from the
//! price's start value) and repaired hard. The round is kept only when
//! the open terminals drop with no net more open than before, no
//! conflict left and no pour pad cut off from its main piece (the
//! router's own count); otherwise everything is restored and the window
//! grows (`RESIDUE_MARGINS_MM`). The single exchanges of the trouble
//! repair move one net at a time; here several give way at once
//! (docs/router-ceiling.md, "Design 1", part 2; docs/router.md, "The
//! residue in a window").
//!
//! Its work is the attempt's (search work of the same router) within a
//! limit of its own: `RESIDUE_SHARE` of what the attempt's work deadline
//! leaves, at least what the hard repair and the trouble repair took.

use super::*;

/// The most open nets the residue takes up: beyond, the board is not a
/// residue away from complete.
pub const RESIDUE_MAX_NETS: usize = 12;

/// The margins a window's box grows by around its pads, round after
/// failed round.
pub const RESIDUE_MARGINS_MM: [f64; 3] = [2.0, 4.0, 8.0];

/// The residue's work limit, as a share of what the attempt's work
/// deadline leaves (`residue_budget`).
pub const RESIDUE_SHARE: f64 = 1.0 / 3.0;

/// `PCB_ROUTER_RESIDUE`: unset, the configuration decides; `0` or empty
/// off; anything else on.
pub fn residue_override() -> Option<bool> {
    let value = std::env::var("PCB_ROUTER_RESIDUE").ok()?;
    Some(!(value.trim().is_empty() || value.trim() == "0"))
}

/// One window of the residue as it went (`Diagnostics::residue`).
#[derive(Clone, Debug, PartialEq)]
pub struct ResidueWindow {
    /// The open nets it was for.
    pub nets: Vec<NetId>,
    /// Its last round's box (mm): x0, y0, x1, y1.
    pub bounds: [f64; 4],
    /// Rounds run (each a larger margin).
    pub rounds: usize,
    /// Nets its last round ripped up (the open ones among them).
    pub ripped: usize,
    /// The board's open terminals before and after (after: as kept, or
    /// the same as before when restored).
    pub open_before: usize,
    pub open_after: usize,
    pub kept: bool,
    /// Its work, seconds of search and pour work.
    pub work: f64,
}

/// The boxes of the open nets (each net's points: its open pads and the
/// pads of its tree nearest them) grown by `margin`, overlapping ones
/// merged: the windows, each with its nets in the order given.
pub(super) fn residue_boxes(
    open: &[(NetId, Vec<crate::geometry::Point>)],
    margin: f64,
) -> Vec<(Vec<NetId>, crate::geometry::Aabb)> {
    let mut windows: Vec<(Vec<NetId>, crate::geometry::Aabb)> = Vec::new();
    for (net, points) in open {
        let Some(first) = points.first() else {
            continue;
        };
        let mut bounds = crate::geometry::Aabb {
            minimum: *first,
            maximum: *first,
        };
        for point in points {
            bounds = bounds.union(crate::geometry::Aabb {
                minimum: *point,
                maximum: *point,
            });
        }
        windows.push((vec![*net], bounds.inflated(margin)));
    }
    // Merge until no two overlap (a merged box may reach a third).
    loop {
        let mut merged = false;
        'outer: for a in 0..windows.len() {
            for b in a + 1..windows.len() {
                if windows[a].1.intersects(windows[b].1) {
                    let (nets, bounds) = windows.remove(b);
                    windows[a].0.extend(nets);
                    windows[a].1 = windows[a].1.union(bounds);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            return windows;
        }
    }
}

/// Whether a round is kept: per net the open terminals before and after
/// (in the same order), no net more open than before, fewer open in all,
/// no conflict left and no pour pad newly cut off from its main piece.
pub(super) fn keep_residue(
    before: &[usize],
    after: &[usize],
    conflicted: usize,
    pours_whole: bool,
) -> bool {
    conflicted == 0
        && pours_whole
        && before.len() == after.len()
        && before
            .iter()
            .zip(after)
            .all(|(before, after)| after <= before)
        && after.iter().sum::<usize>() < before.iter().sum::<usize>()
}

impl Router {
    /// The nets the residue routes again and owns (routable, not given
    /// up, without a pour of their own).
    fn residue_routable(&self, net: NetId) -> bool {
        let state = &self.nets[net as usize];
        state.routable && !state.released && state.plane.is_empty()
    }

    /// The open nets the residue takes up, each with its points: its open
    /// pads that have a node (a pad no track enters is not the residue's)
    /// and, per open pad, the nearest pad of its tree (else of the net).
    pub(super) fn residue_targets(
        &self,
        order: &[NetId],
    ) -> Vec<(NetId, Vec<crate::geometry::Point>)> {
        let mut targets = Vec::new();
        for &net in order {
            let state = &self.nets[net as usize];
            if !self.residue_routable(net) || state.complete {
                continue;
            }
            let terminals = &self.board.nets[net as usize].terminals;
            if terminals.len() < 2 {
                continue;
            }
            let mut points = Vec::new();
            for (index, terminal) in terminals.iter().enumerate() {
                if state.connected.get(index).copied().unwrap_or(false)
                    || state
                        .terminal_nodes
                        .get(index)
                        .is_none_or(|nodes| nodes.is_empty())
                {
                    continue;
                }
                points.push(terminal.anchor);
                let nearest = |connected_only: bool| {
                    terminals
                        .iter()
                        .enumerate()
                        .filter(|(other, _)| {
                            *other != index
                                && (!connected_only
                                    || state.connected.get(*other).copied().unwrap_or(false))
                        })
                        .map(|(_, other)| other.anchor)
                        .min_by(|a, b| {
                            crate::geometry::distance(*a, terminal.anchor)
                                .total_cmp(&crate::geometry::distance(*b, terminal.anchor))
                        })
                };
                if let Some(near) = nearest(true).or_else(|| nearest(false)) {
                    points.push(near);
                }
            }
            if !points.is_empty() {
                targets.push((net, points));
            }
        }
        targets
    }

    /// The node range of `bounds` (mm) on the lattice.
    pub(super) fn residue_range(
        &self,
        bounds: crate::geometry::Aabb,
    ) -> (usize, usize, usize, usize) {
        self.grid
            .node_range(bounds)
            .unwrap_or((0, 0, self.grid.nx - 1, self.grid.ny - 1))
    }

    /// Removes the branches of `net` with a node in `range` (x0, y0, x1,
    /// y1 in nodes) and those left dangling at a junction on them; the
    /// fixed branches stay. Whether it removed any.
    pub(super) fn rip_up_in(&mut self, net: NetId, range: (usize, usize, usize, usize)) -> bool {
        let nx = self.grid.nx;
        let inside = |cell: u32| {
            let (x, y) = (cell as usize % nx, cell as usize / nx);
            x >= range.0 && x <= range.2 && y >= range.1 && y <= range.3
        };
        let state = &self.nets[net as usize];
        let fixed = state.fixed;
        let mut keep: Vec<bool> = state
            .branches
            .iter()
            .enumerate()
            .map(|(index, branch)| {
                index < fixed || !branch.nodes.iter().any(|node| inside(node.cell))
            })
            .collect();
        if keep.iter().all(|kept| *kept) {
            return false;
        }
        loop {
            let hosts = junction_hosts(&state.branches, &keep);
            let mut changed = false;
            for (index, branch) in state.branches.iter().enumerate() {
                if !keep[index] || index < fixed {
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
        true
    }

    /// The negotiation's history inside `range` back to nothing (the
    /// nodes' and their search records', and the tiles' it touches).
    fn reset_history_in(&mut self, range: (usize, usize, usize, usize)) {
        let layers = self.board.layer_count;
        let sets = self.hot.len().checked_div(layers).unwrap_or(0);
        for layer in 0..self.history.len() {
            for y in range.1..=range.3 {
                for x in range.0..=range.2 {
                    let cell = self.grid.index(x, y);
                    self.history[layer][cell] = 0.0;
                    if layer < layers {
                        for set in 0..sets {
                            if let Some(hot) = self.hot[set * layers + layer].get_mut(cell) {
                                hot.history = 0.0;
                            }
                        }
                    }
                }
            }
        }
        for tiles in &mut self.tile_history {
            for ty in range.1 / TILE..=range.3 / TILE {
                for tx in range.0 / TILE..=range.2 / TILE {
                    if let Some(value) = tiles.get_mut(ty * self.tiles_x + tx) {
                        *value = 0.0;
                    }
                }
            }
        }
    }

    /// The open terminals of every net the residue may change, in
    /// `order`'s order.
    fn residue_open(&self, order: &[NetId]) -> Vec<usize> {
        order
            .iter()
            .map(|net| {
                let state = &self.nets[*net as usize];
                if state.routable && !state.released && state.plane.is_empty() {
                    state
                        .connected
                        .iter()
                        .filter(|connected| !**connected)
                        .count()
                } else {
                    0
                }
            })
            .collect()
    }

    /// The residue's work limit: `RESIDUE_SHARE` of what the work deadline
    /// leaves (the negotiation's work without one), at least `repairs`
    /// (the hard and the trouble repair's work), never beyond the deadline.
    fn residue_budget(&self, repairs: f64) -> f64 {
        match self.config.deadline_work {
            Some(limit) => {
                let left = (limit - self.deadline_used()).max(0.0);
                (left * RESIDUE_SHARE).max(repairs).min(left)
            }
            None => self.negotiation_seconds.max(repairs),
        }
    }

    /// The residue in windows (see the module), within its work limit
    /// (`residue_budget` of `repairs`). Returns the open terminals it
    /// closed.
    pub(super) fn renegotiate_residue(&mut self, order: &[NetId], repairs: f64) -> usize {
        self.residue_windows.clear();
        let targets = self.residue_targets(order);
        if targets.is_empty() || self.past_deadline() {
            return 0;
        }
        if targets.len() > RESIDUE_MAX_NETS {
            if self.config.verbose {
                eprintln!(
                    "residue: {} nets open, more than {RESIDUE_MAX_NETS}: not taken up",
                    targets.len()
                );
            }
            return 0;
        }
        let budget = self.residue_budget(repairs);
        let clock = self.clock();
        let why = self.config.verbose || std::env::var_os("PCB_ROUTER_DEBUG").is_some();
        let mut closed = 0;
        for (nets, _) in residue_boxes(&targets, RESIDUE_MARGINS_MM[0]) {
            let window_started = self.budget_work();
            let mut record = ResidueWindow {
                nets: nets.clone(),
                bounds: [0.0; 4],
                rounds: 0,
                ripped: 0,
                open_before: 0,
                open_after: 0,
                kept: false,
                work: 0.0,
            };
            for margin in RESIDUE_MARGINS_MM {
                if self.spent(&clock, budget) {
                    break;
                }
                // The window's nets still open, their points now.
                let own: Vec<(NetId, Vec<crate::geometry::Point>)> = self
                    .residue_targets(order)
                    .into_iter()
                    .filter(|(net, _)| nets.contains(net))
                    .collect();
                if own.is_empty() {
                    break;
                }
                let bounds = residue_boxes(&own, margin)
                    .into_iter()
                    .map(|(_, bounds)| bounds)
                    .reduce(|a, b| a.union(b))
                    .expect("a box per open net");
                record.rounds += 1;
                record.bounds = [
                    bounds.minimum[0],
                    bounds.minimum[1],
                    bounds.maximum[0],
                    bounds.maximum[1],
                ];
                let left = (budget - (self.budget_work() - clock.0)).max(0.0);
                let (before, after, ripped, kept) = self.residue_round(order, &own, bounds, left);
                record.ripped = ripped;
                record.open_before = before;
                record.open_after = if kept { after } else { before };
                if why {
                    let names: Vec<&str> = nets
                        .iter()
                        .map(|net| self.board.nets[*net as usize].name.as_str())
                        .collect();
                    eprintln!(
                        "residue window [{}] margin {margin} mm ({:.2},{:.2})-({:.2},{:.2}): {ripped} nets ripped up, {before} open to {after}, {}",
                        names.join(", "),
                        bounds.minimum[0],
                        bounds.minimum[1],
                        bounds.maximum[0],
                        bounds.maximum[1],
                        if kept { "kept" } else { "restored" }
                    );
                }
                if kept {
                    record.kept = true;
                    closed += before - after;
                    break;
                }
            }
            record.work = self.budget_work() - window_started;
            if record.rounds > 0 {
                self.residue_windows.push(record);
            }
        }
        if self.config.verbose && !self.residue_windows.is_empty() {
            eprintln!(
                "residue: {closed} open terminals closed in {} windows ({} kept), {:.1} s of work of its {budget:.1} s",
                self.residue_windows.len(),
                self.residue_windows
                    .iter()
                    .filter(|window| window.kept)
                    .count(),
                self.budget_work() - clock.0
            );
        }
        closed
    }

    /// One round of a window (`renegotiate_residue`) for the open nets
    /// `own` in `bounds`, within `left` of work: the board's open
    /// terminals before and after, the nets ripped up, whether it was
    /// kept (else everything is as before).
    fn residue_round(
        &mut self,
        order: &[NetId],
        own: &[(NetId, Vec<crate::geometry::Point>)],
        bounds: crate::geometry::Aabb,
        left: f64,
    ) -> (usize, usize, usize, bool) {
        let range = self.residue_range(bounds);
        let before = self.residue_open(order);
        let stranded_before = self.pour_stranded();
        let snapshot = self.lean_snapshot();
        let saved = (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
            self.config.abandon_hopeless,
            self.negotiation_seconds,
            self.present_reached,
            self.last_conflicted.clone(),
            self.hopeless,
        );
        // The open nets first, then every net with copper in the box.
        let mut pending: Vec<NetId> = own.iter().map(|(net, _)| *net).collect();
        for net in pending.clone() {
            self.rip_up_in(net, range);
        }
        for &net in order {
            if pending.contains(&net) || !self.residue_routable(net) {
                continue;
            }
            if self.rip_up_in(net, range) {
                pending.push(net);
            }
        }
        let ripped = pending.len();
        self.reset_history_in(range);
        self.config.negotiation_expansions = ((left * expansions_per_second()) as u64).max(1);
        self.config.negotiation_seconds = self.config.negotiation_seconds.min(left * guard());
        // The board as a whole is not hopeless for a window's sake.
        self.config.abandon_hopeless = false;
        self.in_residue = true;
        self.negotiate_from(order, pending, self.config.present_factor as f32);
        self.resolve_remaining(order);
        self.in_residue = false;
        (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
            self.config.abandon_hopeless,
            self.negotiation_seconds,
            self.present_reached,
            self.last_conflicted,
            self.hopeless,
        ) = saved;
        let after = self.residue_open(order);
        let conflicted = order
            .iter()
            .filter(|net| !self.conflicts(**net).is_empty())
            .count();
        let stranded_after = self.pour_stranded();
        let pours_whole = stranded_after
            .iter()
            .zip(&stranded_before)
            .all(|(after, before)| after <= before);
        let kept = keep_residue(&before, &after, conflicted, pours_whole);
        let (open_before, open_after) = (before.iter().sum(), after.iter().sum());
        if !kept {
            self.restore_lean(snapshot);
        }
        (open_before, open_after, ripped, kept)
    }
}

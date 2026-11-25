// Copyright (C) 2026 Florian Loitsch.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Why the terminals a finish leaves open stay open (`OpenReason`,
//! `Diagnostics::open`): recorded where the finish decides (the last
//! negotiation's board-wide searches, the hard repair's searches and
//! budget, each later phase's before and after), read when the result is
//! made. A record, not an input: nothing reads it to route, and recording
//! adds no search of the run. Where a static wall is (`static_walls`) is
//! found when the result is made, for the nets that end open, by a test
//! that counts no work.

use super::corridors::StaticWall;
use super::*;

/// Where a terminal's static wall is (`Router::static_walls`): the
/// terminals it is at (indices in the net) and what walls them.
type WallAt = Option<(Vec<usize>, StaticWall)>;

impl Router {
    /// Whether `net` has something to connect: not trivial, not given up
    /// (`statuses`).
    fn has_connections(&self, net: usize) -> bool {
        let state = &self.nets[net];
        !state.released && (self.board.nets[net].terminals.len() >= 2 || !state.plane.is_empty())
    }

    /// At a finish's start: every terminal without a reason, but those
    /// open where the last negotiation's board-wide search did not reach
    /// them (`NetState::unreachable`: `OpenReason::NoPathStatic`).
    pub(super) fn open_reasons_start(&mut self) {
        self.open_reasons = self
            .nets
            .iter()
            .map(|state| {
                (0..state.connected.len())
                    .map(|terminal| {
                        if state.unreachable.get(terminal).copied().unwrap_or(false)
                            && !state.connected[terminal]
                        {
                            OpenReason::NoPathStatic
                        } else {
                            OpenReason::Unknown
                        }
                    })
                    .collect()
            })
            .collect();
    }

    /// Per terminal of every net, in net order: whether it is open now
    /// (not `connected`, or its net has nothing to connect).
    pub(super) fn open_flags(&self) -> Vec<bool> {
        let mut flags = Vec::new();
        for (net, state) in self.nets.iter().enumerate() {
            let counted = self.has_connections(net);
            flags.extend(
                state
                    .connected
                    .iter()
                    .map(|connected| !counted || !connected),
            );
        }
        flags
    }

    /// After a phase: the terminals joined in `before` (`open_flags`) and
    /// open now were left open by `phase` (`OpenReason::Lost`); with
    /// `unknown_only`, only those without a reason yet.
    pub(super) fn note_lost(&mut self, before: &[bool], phase: &'static str, unknown_only: bool) {
        if self.open_reasons.len() != self.nets.len() {
            return;
        }
        let mut index = 0;
        for net in 0..self.nets.len() {
            let counted = self.has_connections(net);
            let state = &self.nets[net];
            for (terminal, connected) in state.connected.iter().enumerate() {
                let was_open = before.get(index).copied().unwrap_or(true);
                index += 1;
                if !counted {
                    continue;
                }
                let Some(reason) = self.open_reasons[net].get_mut(terminal) else {
                    continue;
                };
                if !connected && !was_open && (!unknown_only || *reason == OpenReason::Unknown) {
                    *reason = OpenReason::Lost(phase);
                }
            }
        }
    }

    /// After the pour stitching (`stitch_pour_nets`): the terminals it
    /// left open as `note_lost` finds them, but those of `apart` (per
    /// pour net, the terminals its first analysis found off the main
    /// piece, before the stitching changed anything): the pour model
    /// counted those joined and the stitching did not join them
    /// (`OpenReason::PourApartBeforeStitching`), it did not lose them.
    pub(super) fn note_stitching(&mut self, before: &[bool], apart: &[(usize, Vec<usize>)]) {
        self.note_lost(before, "pour_stitching", false);
        if self.open_reasons.len() != self.nets.len() {
            return;
        }
        for (net, terminals) in apart {
            for &terminal in terminals {
                if let Some(reason) = self.open_reasons[*net].get_mut(terminal)
                    && *reason == OpenReason::Lost("pour_stitching")
                {
                    *reason = OpenReason::PourApartBeforeStitching;
                }
            }
        }
    }

    /// After the hard repair (`resolve_remaining`): the terminals still
    /// open, without a reason, of the nets it did not search
    /// (`repair_skipped`: `OpenReason::RepairSkipped`) and of those it
    /// searched without finding every path (`repair_failed`:
    /// `OpenReason::NoPathHard`).
    pub(super) fn note_repair(&mut self) {
        if self.open_reasons.len() != self.nets.len() {
            return;
        }
        let outcomes: Vec<(NetId, OpenReason)> = self
            .repair_skipped
            .iter()
            .map(|&(net, why)| (net, OpenReason::RepairSkipped(why)))
            .chain(
                self.repair_failed
                    .iter()
                    .filter(|(_, scope)| !scope.is_empty())
                    .map(|&(net, scope)| (net, OpenReason::NoPathHard(scope))),
            )
            .collect();
        for (net, outcome) in outcomes {
            let state = &self.nets[net as usize];
            for (terminal, reason) in self.open_reasons[net as usize].iter_mut().enumerate() {
                if !state.connected[terminal] && *reason == OpenReason::Unknown {
                    *reason = outcome;
                }
            }
        }
    }

    /// Where the wall is for the terminals of `net` recorded
    /// `NoPathStatic` (a board-wide search from the net's tree did not
    /// reach them): per terminal of the net, the terminals whose pocket
    /// the lattice closes and what walls them (`static_wall`). The
    /// terminal's own pocket, else the tree's (the net's terminals without
    /// that record) where every one of its terminals is in a closed
    /// pocket: then the wall is at the tree, not at the terminals it did
    /// not reach. None where neither is closed.
    /// Computed for the result's report, for the nets that end open with
    /// such a terminal: no search of the run reads it.
    fn static_walls(&self, net: usize) -> Vec<WallAt> {
        let count = self.nets[net].connected.len();
        let recorded = self.open_reasons.get(net);
        let unreachable = |terminal: usize| {
            recorded.and_then(|reasons| reasons.get(terminal)) == Some(&OpenReason::NoPathStatic)
        };
        let mut walls: Vec<Option<Option<StaticWall>>> = vec![None; count];
        let mut wall_of = |terminal: usize| -> Option<StaticWall> {
            walls[terminal]
                .get_or_insert_with(|| self.static_wall(net as NetId, terminal))
                .clone()
        };
        // The tree's wall: every terminal of the tree in a closed pocket.
        let tree: Vec<usize> = (0..count)
            .filter(|terminal| !unreachable(*terminal))
            .collect();
        let mut tree_wall: WallAt = None;
        if !tree.is_empty() && tree.len() < count {
            let mut merged = StaticWall {
                leaves: true,
                by: Vec::new(),
            };
            let mut closed = true;
            for &terminal in &tree {
                let Some(wall) = wall_of(terminal) else {
                    closed = false;
                    break;
                };
                merged.leaves &= wall.leaves;
                merged.by.extend(wall.by);
            }
            if closed {
                merged.by.sort();
                merged.by.dedup();
                if merged.leaves {
                    merged.by.clear();
                }
                tree_wall = Some((tree, merged));
            }
        }
        (0..count)
            .map(|terminal| {
                if !unreachable(terminal) {
                    return None;
                }
                wall_of(terminal)
                    .map(|wall| (vec![terminal], wall))
                    .or_else(|| tree_wall.clone())
            })
            .collect()
    }

    /// The open terminals of a result of `status` (`Diagnostics::open`):
    /// a `Partial` net's unconnected terminals, or those the copper check
    /// left `apart` (per net its terminals), and every terminal of an
    /// `Unreachable` net. A terminal without a lattice node is
    /// `NoAccess` whatever was recorded. A terminal recorded
    /// `NoPathStatic` names the terminals the wall is at and what walls
    /// them where a pad's pocket is closed (`static_walls`), and is
    /// `NoLatticeExit` where the rules leave a way out of that pocket.
    pub(super) fn open_terminals(
        &self,
        status: &[NetStatus],
        apart: &[(usize, Vec<usize>)],
    ) -> Vec<OpenTerminal> {
        let mut open = Vec::new();
        for (net, status) in status.iter().enumerate() {
            let state = &self.nets[net];
            let description = &self.board.nets[net];
            let no_access = |terminal: usize| {
                state.terminal_nodes[terminal].is_empty()
                    && !state.on_plane.get(terminal).copied().unwrap_or(false)
            };
            let mut walls: Option<Vec<WallAt>> = None;
            let mut push = |terminal: usize, reason: OpenReason| {
                let mut entry = OpenTerminal {
                    net: net as NetId,
                    terminal,
                    label: description.terminals[terminal].label.clone(),
                    reason: if no_access(terminal) {
                        OpenReason::NoAccess
                    } else {
                        reason
                    },
                    walled: Vec::new(),
                    wall: Vec::new(),
                };
                if entry.reason == OpenReason::NoPathStatic
                    && let Some((walled, wall)) = walls
                        .get_or_insert_with(|| self.static_walls(net))
                        .get(terminal)
                        .cloned()
                        .flatten()
                {
                    entry.walled = walled
                        .iter()
                        .map(|other| description.terminals[*other].label.clone())
                        .collect();
                    if wall.leaves {
                        entry.reason = OpenReason::NoLatticeExit;
                    } else {
                        entry.wall = wall.by;
                    }
                }
                open.push(entry);
            };
            match status {
                NetStatus::Unreachable => {
                    for terminal in 0..description.terminals.len() {
                        push(terminal, OpenReason::NetNotRoutable);
                    }
                }
                NetStatus::Partial { .. } => {
                    if let Some((_, terminals)) = apart.iter().find(|(other, _)| *other == net) {
                        for &terminal in terminals {
                            push(terminal, OpenReason::CopperCheck);
                        }
                        continue;
                    }
                    for (terminal, connected) in state.connected.iter().enumerate() {
                        if !connected {
                            let recorded = self
                                .open_reasons
                                .get(net)
                                .and_then(|reasons| reasons.get(terminal))
                                .copied()
                                .unwrap_or(OpenReason::Unknown);
                            push(terminal, recorded);
                        }
                    }
                }
                NetStatus::Trivial | NetStatus::Routed => {}
            }
        }
        open
    }

    /// Per entry of `nets` (a net and its conflicts by occupancy map,
    /// `conflict_maps`), the other nets with a stamp on its first
    /// `CONFLICT_PARTNERS` conflicts (the trouble repair's `blockers`, on
    /// fewer cells), sorted, at most that many: one pass over every net's
    /// stamps for all of them.
    pub(super) fn conflict_partners(&self, nets: &[(NetId, Vec<(u32, u32)>)]) -> Vec<Vec<NetId>> {
        let mut wanted: FastMap<(u32, u32), Vec<usize>> = FastMap::default();
        for (index, (_, cells)) in nets.iter().enumerate() {
            for cell in cells.iter().take(CONFLICT_PARTNERS) {
                wanted.entry(*cell).or_default().push(index);
            }
        }
        let mut partners: Vec<Vec<NetId>> = vec![Vec::new(); nets.len()];
        if wanted.is_empty() {
            return partners;
        }
        for (other, state) in self.nets.iter().enumerate() {
            let other = other as NetId;
            for stamp in state.stamped.iter() {
                let Some(entries) = wanted.get(&stamp) else {
                    continue;
                };
                for &index in entries {
                    if nets[index].0 != other && !partners[index].contains(&other) {
                        partners[index].push(other);
                    }
                }
            }
        }
        for list in &mut partners {
            list.sort_unstable();
            list.truncate(CONFLICT_PARTNERS);
        }
        partners
    }
}

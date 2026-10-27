// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The coarse probe's ends (pcb-kicad's `probe-coarse-placement`, a
//! research measure; docs/placer.md, "A coarse whole first route as the
//! race probe (2026-10-27)"): after `probe`, the board as it stands once
//! the nets in conflict lose their conflicted branches (the hard repair's
//! rip-up without its searches), or the first route's finish (the hard
//! repair and the trouble repair as configured, no polish).

use super::*;

impl Router {
    /// The negotiation iterations so far and why the last negotiation
    /// ended (`RoutingResult::negotiation_stop`).
    pub fn negotiation_progress(&self) -> (usize, &'static str) {
        (self.iterations, self.negotiation_stop)
    }

    /// After `probe`: every net in conflict loses its conflicted branches,
    /// the most conflicted first and the others judged again after each
    /// (`resolve_remaining`'s rip-up, without its hard searches), and the
    /// standing then: no net is left in conflict, so its open terminals
    /// are the board's (`board_open_terminals`) and the pour pads off
    /// their pour. Returns the nets ripped and the standing.
    pub fn rip_up_standing(&mut self) -> (usize, ProbeStanding) {
        let order = self.routing_order();
        let mut removed: Vec<NetId> = Vec::new();
        loop {
            let worst = order
                .iter()
                .filter(|net| !removed.contains(*net))
                .map(|net| (self.conflicts(*net).len(), *net))
                .filter(|(count, _)| *count > 0)
                .max();
            let Some((_, net)) = worst else {
                break;
            };
            self.nets[net as usize].stuck = 0;
            self.rip_up_conflicted(net);
            removed.push(net);
        }
        (removed.len(), self.probe_standing())
    }

    /// After `probe`: the finish of the layout's first route (the hard
    /// repair, the trouble repair where `Config::trouble_repair` has it,
    /// the pours stitched; no clean-up, no via reduction), within the
    /// router's work deadline (`Config::deadline_work`).
    pub fn finish_probe(&mut self) -> RoutingResult {
        let order = self.routing_order();
        self.finish_polished(&order, false)
    }
}

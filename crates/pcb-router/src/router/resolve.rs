// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The hard repair after the negotiation.

use super::*;

impl Router {
    /// Makes the board conflict free: nets still in conflict are ripped up
    /// (most contested first) and reinserted treating all other copper as
    /// hard obstacles.
    pub(super) fn resolve_remaining(&mut self, order: &[NetId]) {
        let mut removed = Vec::new();
        loop {
            let worst = order
                .iter()
                .filter(|net| !removed.contains(*net))
                .map(|net| (self.conflicts(*net).len(), *net))
                .filter(|(count, _)| *count > 0)
                .max();
            let Some((count, net)) = worst else {
                break;
            };
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                let state = &self.nets[net as usize];
                eprintln!(
                    "resolve {}: {count} conflicts, {} of {} terminals connected, conflicted branches ripped up",
                    self.board.nets[net as usize].name,
                    state.connected.iter().filter(|c| **c).count(),
                    state.connected.len(),
                );
            }
            // The negotiation's whole rip-up every eighth iteration in
            // conflict (`rip_up_whole`) is not the repair's business.
            self.nets[net as usize].stuck = 0;
            self.rip_up_conflicted(net);
            removed.push(net);
        }
        // Incomplete nets without conflicts keep what they have: the hard
        // search joins the rest to it. Ripped up, a big net's hard reroute
        // starts again from one pad among copper that no longer gives way
        // and can end far shorter (SNSP-CPU-01's GND: 147 of 199 pads
        // negotiated, 21 after the hard reroute).
        for net in order {
            if !removed.contains(net) && !self.nets[*net as usize].complete {
                if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                    let state = &self.nets[*net as usize];
                    eprintln!(
                        "resolve {}: incomplete, {} of {} terminals connected, {} branches, kept",
                        self.board.nets[*net as usize].name,
                        state.connected.iter().filter(|c| **c).count(),
                        state.connected.len(),
                        state.branches.len(),
                    );
                }
                removed.push(*net);
            }
        }
        // A differential pair's links (pcb-kicad's nets with "pair launch"
        // terminals, route mode's `diff_pairs`) first, and searched even
        // where the attempt is judged hopeless (within the budget a
        // repair has otherwise): a link left open costs its pair a
        // fallback. Without links the order is the routing order.
        let links: Vec<bool> = self
            .board
            .nets
            .iter()
            .map(|net| {
                net.terminals
                    .first()
                    .is_some_and(|terminal| terminal.label.starts_with("pair launch"))
            })
            .collect();
        let link = |net: NetId| links[net as usize];
        removed.sort_by_key(|net| (!link(*net), order.iter().position(|other| other == net)));
        let debug = std::env::var_os("PCB_ROUTER_DEBUG").is_some();
        // The hard reroute is a repair after negotiation: on a rung judged
        // hopeless it is not worth its time (the ladder moves on), and it
        // never takes longer than the negotiation did (at least 60 s): a
        // failing hard search sweeps four ever larger windows, 9 s a net on
        // a 4.8M-node board, 960 s for 107 nets. Nets it does not reach stay
        // ripped up (open, as they were in conflict).
        let budget = if self.hopeless {
            0.0
        } else {
            self.negotiation_seconds.max(60.0)
        };
        let started = std::time::Instant::now();
        let clock = self.clock();
        let mut skipped = 0;
        self.repair_skipped.clear();
        let link_budget = self.negotiation_seconds.max(60.0);
        for net in &removed {
            let budget = if link(*net) { link_budget } else { budget };
            if self.spent(&clock, budget) {
                skipped += 1;
                let why = if self.past_work_deadline() {
                    "work_deadline"
                } else if self.hopeless && !link(*net) {
                    "hopeless"
                } else {
                    "budget"
                };
                self.repair_skipped.push((*net, why));
                continue;
            }
            self.route_net_seq(*net, 0.0, true, 4.0);
            self.stamp(*net);
            if debug {
                let state = &self.nets[*net as usize];
                eprintln!(
                    "resolve {}: rerouted hard, {} of {} terminals connected, {} branches",
                    self.board.nets[*net as usize].name,
                    state.connected.iter().filter(|c| **c).count(),
                    state.connected.len(),
                    state.branches.len()
                );
            }
        }
        let rerouted = started.elapsed().as_secs_f64();
        let mut forced = 0;
        for net in removed.iter().copied() {
            if !self.nets[net as usize].complete && !self.spent(&clock, 2.0 * budget) {
                forced += 1;
                if self.force_connect(net, order) {
                    continue;
                }
            }
        }
        if self.config.verbose && !removed.is_empty() {
            eprintln!(
                "resolve: {} nets rerouted hard in {rerouted:.1} s ({skipped} left open by the budget), {forced} forced in {:.1} s",
                removed.len() - skipped,
                started.elapsed().as_secs_f64() - rerouted
            );
        }
    }
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Copper fixed before the negotiation: plane skeletons, plane stubs.

use super::*;

impl Router {
    /// Routes every pour net that has no copper yet as a plain tree, biased
    /// onto the layer its pours cover most, against all copper already on
    /// the board. The tree stays fixed for the rest of the run.
    pub(super) fn route_skeletons(&mut self, order: &[NetId]) {
        if !self.config.plane_skeleton {
            return;
        }
        let layers = self.board.layer_count;
        for net in order {
            let state = &self.nets[*net as usize];
            if state.plane.is_empty()
                || state.fixed > 0
                || !state.branches.is_empty()
                || self.board.nets[*net as usize].terminals.len() < 2
            {
                continue;
            }
            let started = std::time::Instant::now();
            let layer = (0..layers)
                .max_by_key(|layer| state.plane[*layer].iter().filter(|inside| **inside).count())
                .unwrap_or(0);
            self.layer_bias = vec![self.config.skeleton_bias as f32; layers];
            self.layer_bias[layer] = 1.0;
            let mut scratch = std::mem::take(&mut self.scratch);
            let mut net_state = std::mem::take(&mut self.nets[*net as usize]);
            let plane = std::mem::take(&mut net_state.plane);
            self.route_net(&mut scratch, *net, &mut net_state, 0.0, true, 4.0);
            net_state.plane = plane;
            net_state.fixed = net_state.branches.len();
            net_state.blocked = false;
            self.nets[*net as usize] = net_state;
            self.scratch = scratch;
            self.layer_bias.clear();
            self.stamp(*net);
            if self.config.verbose {
                let state = &self.nets[*net as usize];
                let vias = state
                    .branches
                    .iter()
                    .map(|branch| {
                        branch
                            .nodes
                            .windows(2)
                            .filter(|pair| pair[0].cell == pair[1].cell)
                            .count()
                    })
                    .sum::<usize>();
                eprintln!(
                    "skeleton {} on layer {layer}: {} branches, {vias} vias, complete {}, {:.2}s",
                    self.board.nets[*net as usize].name,
                    state.fixed,
                    state.complete,
                    started.elapsed().as_secs_f64()
                );
            }
        }
    }

    /// Connects every pad of a pour net to its plane on the empty board (a
    /// stub and a via next to the pad, as a designer places them) and keeps
    /// those connections fixed: signals route around them instead of
    /// pushing them off the via sites next to the pads.
    pub(super) fn fix_plane_stubs(&mut self, order: &[NetId]) {
        if !self.config.fixed_plane_stubs {
            return;
        }
        let started = std::time::Instant::now();
        let pour_nets: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| {
                let state = &self.nets[*net as usize];
                !state.plane.is_empty() && state.fixed == 0 && state.branches.is_empty()
            })
            .collect();
        if pour_nets.is_empty() {
            return;
        }
        // On the empty board every pad touches its layer's pour; the pours
        // on the outer layers are what the signals cut to pieces later. So
        // only the inner planes count here: a pad on an outer layer has to
        // reach one through a via, as a designer places one next to every
        // ground pad. Boards without an inner plane are left alone.
        let layers = self.board.layer_count;
        let mut saved = Vec::new();
        for net in &pour_nets {
            let state = &mut self.nets[*net as usize];
            let inner = (1..layers.saturating_sub(1)).any(|layer| !state.plane[layer].is_empty());
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                let covered: Vec<usize> = (0..layers)
                    .filter(|layer| !state.plane[*layer].is_empty())
                    .collect();
                eprintln!(
                    "  plane stubs {}: pour layers {covered:?}, inner {inner}",
                    self.board.nets[*net as usize].name
                );
            }
            if !inner {
                continue;
            }
            saved.push((*net, state.plane.clone(), state.on_plane.clone()));
            state.plane[0] = Vec::new();
            state.plane[layers - 1] = Vec::new();
            for (index, nodes) in state.terminal_nodes.iter().enumerate() {
                state.on_plane[index] = nodes.iter().any(|node| {
                    state
                        .plane
                        .get(node.layer as usize)
                        .is_some_and(|mask| !mask.is_empty() && mask[node.cell as usize])
                });
            }
        }
        if saved.is_empty() {
            return;
        }
        let pour_nets: Vec<NetId> = saved.iter().map(|(net, _, _)| *net).collect();
        // The pour nets negotiate among themselves (their vias compete for
        // the same spots next to alternating pads), then stay.
        // At most a minute: of work where the wall clock guards (route
        // mode; a minute of wall clock made it depend on the load), as
        // before under a caller's deadline (the layout: stall alone).
        let limits = (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
        );
        if self.wall_guard() {
            let minute = (60.0 * expansions_per_second()) as u64;
            self.config.negotiation_expansions = if limits.1 == 0 {
                minute
            } else {
                limits.1.min(minute)
            };
            self.config.negotiation_seconds = limits.0.min(60.0 * guard());
        } else {
            self.config.negotiation_seconds = limits.0.min(60.0);
        }
        self.negotiate(&pour_nets, pour_nets.clone());
        (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
        ) = limits;
        for (net, plane, on_plane) in saved {
            let state = &mut self.nets[net as usize];
            state.plane = plane;
            state.on_plane = on_plane;
        }
        for net in &pour_nets {
            // Only what is in conflict with nothing stays fixed: the
            // branches still contested (ColdFire's alternating GND and
            // +3.3V pads, whose vias compete for the same spots) go, and
            // those pads are left to the stitching at the end.
            let keep = self.unconflicted_branches(*net);
            let mut keep = keep.into_iter();
            let state = &mut self.nets[*net as usize];
            state.branches.retain(|_| keep.next().unwrap());
            state.fixed = state.branches.len();
            state.blocked = false;
            state.complete = false;
            self.stamp(*net);
            let state = &mut self.nets[*net as usize];
            if self.config.verbose {
                eprintln!(
                    "plane stubs {}: {} fixed branches, {} of {} terminals connected, {:.2}s",
                    self.board.nets[*net as usize].name,
                    state.fixed,
                    state.connected.iter().filter(|done| **done).count(),
                    state.connected.len(),
                    started.elapsed().as_secs_f64()
                );
            }
        }
    }
}

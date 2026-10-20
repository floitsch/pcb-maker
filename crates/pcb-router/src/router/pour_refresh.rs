// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Pour pads the signals cut off during the negotiation.

use super::*;

impl Router {
    /// Per pour net (in net order): how many of its pads the pour and the
    /// net's copper leave off the main piece. The polish keeps a change
    /// only if none of these grows (`pour_aware_polish`).
    pub(super) fn pour_stranded(&self) -> Vec<usize> {
        self.pour_net_ids()
            .into_iter()
            .map(|net| self.pour_stranded_of(net))
            .collect()
    }

    /// The pour nets `pour_stranded` counts, in its order.
    pub(super) fn pour_net_ids(&self) -> Vec<NetId> {
        (0..self.nets.len() as NetId)
            .filter(|net| {
                self.nets[*net as usize].routable && !self.nets[*net as usize].plane.is_empty()
            })
            .collect()
    }

    pub(super) fn pour_stranded_of(&self, net: NetId) -> usize {
        let (pours, mut parent, main) = self.analyze_pours(net);
        let terminal_base = pours.pieces + 1;
        (0..self.nets[net as usize].terminal_nodes.len())
            .filter(|terminal| find(&mut parent, terminal_base + terminal) != main)
            .count()
    }

    /// Node bounds (x0, y0, x1, y1) of a pour net's planes on all layers.
    pub(super) fn plane_bounds(&self, net: NetId) -> Option<(usize, usize, usize, usize)> {
        let mut bounds: Option<(usize, usize, usize, usize)> = None;
        for mask in &self.nets[net as usize].plane {
            for (cell, inside) in mask.iter().enumerate() {
                if *inside {
                    let (x, y) = self.grid.xy(cell);
                    bounds = Some(bounds.map_or((x, y, x, y), |(x0, y0, x1, y1)| {
                        (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
                    }));
                }
            }
        }
        bounds
    }

    /// Which pads of pour net `net` its pour no longer joins to the main
    /// piece (the signals cut the pour between them): those leave
    /// `on_plane`, so the net routes them to the main piece (only its
    /// nodes are targets from then on). A pad whose piece the pour joins
    /// again (the signals moved) is back on the plane and loses its stub:
    /// the vias of the early, crowded iterations must not stay (ESC mini's
    /// exclusive-plane rung: 113 open with them against 81). Returns how
    /// many pads were cut off newly; the net then needs rerouting.
    pub(super) fn refresh_pour(&mut self, net: NetId) -> usize {
        let state = &self.nets[net as usize];
        if state.plane.is_empty() || !state.routable {
            return 0;
        }
        let (pours, mut parent, main) = self.analyze_pours(net);
        let state = &self.nets[net as usize];
        let terminal_base = pours.pieces + 1;
        let terminal_count = state.terminal_nodes.len();
        let touches = |state: &NetState, terminal: usize| {
            state.terminal_nodes[terminal].iter().any(|node| {
                state
                    .plane
                    .get(node.layer as usize)
                    .is_some_and(|mask| !mask.is_empty() && mask[node.cell as usize])
            })
        };
        let newly: Vec<usize> = (0..terminal_count)
            .filter(|terminal| {
                state.on_plane[*terminal] && find(&mut parent, terminal_base + terminal) != main
            })
            .collect();
        // Pads cut off before whose own piece joins the main one again
        // without any stub (a pad's branch to the plane): through the pour
        // and the net's other branches only.
        let is_stub = |index: usize| {
            let branch = &state.branches[index];
            (branch.end_terminal == PLANE_TERMINAL && branch.start_terminal < FREE_END)
                || (branch.start_terminal == PLANE_TERMINAL && branch.end_terminal < FREE_END)
        };
        let rejoined: Vec<usize> = if (0..terminal_count)
            .any(|terminal| !state.on_plane[terminal] && touches(state, terminal))
        {
            // Without the stubs the pour has pieces of its own (the stubs'
            // nodes are fill no more), so the terminals follow its pieces,
            // not the full analysis's (ASH-ART Qfwfq: index 143 of 143).
            let (pours_without, mut without, main_without) =
                self.analyze_pours_skipping(net, &is_stub);
            let base_without = pours_without.pieces + 1;
            (0..terminal_count)
                .filter(|terminal| !state.on_plane[*terminal] && touches(state, *terminal))
                .filter(|terminal| find(&mut without, base_without + terminal) == main_without)
                .collect()
        } else {
            Vec::new()
        };
        // While any pad has to reach the main piece by a branch, only the
        // main piece is a target (the nearest pour node may be an island);
        // the piece moves with the signals, so it is found again each time.
        let any_off = !newly.is_empty()
            || state
                .on_plane
                .iter()
                .enumerate()
                .any(|(terminal, on)| !on && !rejoined.contains(&terminal));
        let target: Vec<Vec<bool>> = if any_off {
            (0..self.board.layer_count)
                .map(|layer| {
                    pours.label[layer]
                        .iter()
                        .map(|piece| *piece != 0 && find(&mut parent, *piece as usize) == main)
                        .collect()
                })
                .collect()
        } else {
            Vec::new()
        };
        let state = &mut self.nets[net as usize];
        state.plane_target = target;
        if !rejoined.is_empty() {
            for terminal in &rejoined {
                state.on_plane[*terminal] = true;
            }
            // The stubs those pads needed go, and with them any branch
            // left dangling at a junction on one of them (katia and the
            // Telemetry board panicked on such a junction without its
            // host); what remains is restamped.
            let fixed = state.fixed;
            let mut keep: Vec<bool> = state
                .branches
                .iter()
                .enumerate()
                .map(|(index, branch)| {
                    index < fixed
                        || !((branch.end_terminal == PLANE_TERMINAL
                            && rejoined.contains(&(branch.start_terminal as usize)))
                            || (branch.start_terminal == PLANE_TERMINAL
                                && rejoined.contains(&(branch.end_terminal as usize))))
                })
                .collect();
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
            state.branches.retain(|_| keep.next().unwrap());
            self.stamp(net);
        }
        let state = &mut self.nets[net as usize];
        if newly.is_empty() {
            return 0;
        }
        for terminal in &newly {
            state.on_plane[*terminal] = false;
            state.connected[*terminal] = false;
        }
        state.complete = false;
        newly.len()
    }
}

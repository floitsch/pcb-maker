// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The clean-up: each net rerouted alone, kept when cheaper.

use super::*;

impl Router {
    /// Length plus via cost of a net's branches, in millimetres.
    pub(super) fn geometric_cost(&self, net: NetId) -> f64 {
        self.geometric_cost_of(net, &self.nets[net as usize].branches)
    }

    /// `geometric_cost` of `net` with these branches.
    pub(super) fn geometric_cost_of(&self, net: NetId, branches: &[Branch]) -> f64 {
        let nx = self.grid.nx as i64;
        let mut cost = 0.0;
        for branch in branches {
            for pair in branch.nodes.windows(2) {
                if pair[0].cell == pair[1].cell {
                    cost += if self.nets[net as usize].plane.is_empty() {
                        self.config.cleanup_via_cost
                    } else {
                        self.config.plane_via_cost
                    };
                } else {
                    let dx = (pair[0].cell as i64 % nx - pair[1].cell as i64 % nx).abs();
                    let dy = (pair[0].cell as i64 / nx - pair[1].cell as i64 / nx).abs();
                    cost += self.grid.pitch * ((dx * dx + dy * dy) as f64).sqrt();
                }
            }
        }
        cost
    }

    /// Reroutes each complete net alone against all other copper and keeps
    /// the result when it is strictly cheaper. Returns the improvements.
    pub(super) fn clean_up(&mut self, order: &[NetId]) -> usize {
        self.cleanup = true;
        let mut improvements = 0;
        let mut pour_stranded = self
            .config
            .pour_aware_polish
            .then(|| self.pour_stranded())
            .filter(|counts| !counts.is_empty());
        let mut pour_check_seconds = 0.0;
        let mut pour_rejected = 0;
        let mut pour_analyses = 0usize;
        let (pour_nets, pour_bounds) = if pour_stranded.is_some() {
            let nets = self.pour_net_ids();
            let bounds: Vec<_> = nets.iter().map(|net| self.plane_bounds(*net)).collect();
            (nets, bounds)
        } else {
            (Vec::new(), Vec::new())
        };
        // How far a stamp reaches from its node, generously: the widest
        // via and track of any class with the largest clearances, twice.
        let pour_reach = {
            let widest = self
                .board
                .classes
                .iter()
                .map(|class| class.via_diameter + class.trace_width + class.clearance)
                .fold(0.0f64, f64::max)
                + self.board.hole_clearance.max(self.board.hole_to_hole);
            (2.0 * widest / self.grid.pitch).ceil() as usize + 3
        };
        let started = self.clock();
        let pour_work_before = self.pour_work_seconds();
        let budget = self
            .config
            .cleanup_seconds
            .min(self.negotiation_seconds.max(60.0));
        'passes: for _ in 0..self.config.cleanup_passes {
            let mut improved = false;
            let complete: Vec<NetId> = order
                .iter()
                .copied()
                .filter(|net| self.nets[*net as usize].complete)
                .collect();
            // Nets whose windows do not overlap are cleaned up at once; a
            // new route that leaves its window is not kept.
            let batches: Vec<Vec<NetId>> = if self.config.parallel {
                self.batches(&complete, 1.0)
            } else {
                complete.iter().map(|net| vec![*net]).collect()
            };
            for batch in batches {
                if self.spent(&started, budget) {
                    if self.config.verbose {
                        eprintln!("clean-up budget of {budget:.0} s used");
                    }
                    break 'passes;
                }
                let before: Vec<f64> = batch.iter().map(|net| self.geometric_cost(*net)).collect();
                let saved: Vec<Vec<Branch>> = batch
                    .iter()
                    .map(|net| self.nets[*net as usize].branches.clone())
                    .collect();
                for net in &batch {
                    self.rip_up(*net);
                }
                let mut inside = vec![true; batch.len()];
                if batch.len() == 1 {
                    self.route_net_seq(batch[0], 0.0, true, 1.0);
                } else {
                    let states = self.grid.cells() * self.board.layer_count;
                    let cells = self.grid.cells();
                    let this: &Self = self;
                    let routed: Vec<(NetState, bool, u64)> = batch
                        .par_iter()
                        .map(|net| {
                            with_thread_scratch(states, cells, |scratch| {
                                let mut net_state = this.nets[*net as usize].clone();
                                if !net_state.stamped.is_empty() {
                                    this.mask_own(scratch, *net, &net_state);
                                }
                                let before = scratch.expansions;
                                this.route_net(scratch, *net, &mut net_state, 0.0, true, 1.0);
                                let expansions = scratch.expansions - before;
                                scratch.unmask_own();
                                // Batch windows are kept two tiles apart:
                                // routes inside their own window stay clear
                                // of each other.
                                let window = this.window(*net, 1.0);
                                let within = net_state
                                    .branches
                                    .iter()
                                    .flat_map(|branch| &branch.nodes)
                                    .all(|node| {
                                        let (x, y) = this.grid.xy(node.cell as usize);
                                        x >= window.0
                                            && x <= window.2
                                            && y >= window.1
                                            && y <= window.3
                                    });
                                (net_state, within, expansions)
                            })
                        })
                        .collect();
                    for (index, (net_state, within, expansions)) in routed.into_iter().enumerate() {
                        self.nets[batch[index] as usize] = net_state;
                        inside[index] = within;
                        // The work of the threads counts on the shared clock.
                        self.scratch.expansions += expansions;
                    }
                }
                let mut accepted: Vec<bool> = Vec::with_capacity(batch.len());
                for (index, net) in batch.iter().enumerate() {
                    let state = &self.nets[*net as usize];
                    let better = inside[index]
                        && state.complete
                        && self.geometric_cost(*net) < before[index] - 1.0e-6;
                    if !better {
                        let state = &mut self.nets[*net as usize];
                        state.branches = saved[index].clone();
                        state.connected.fill(true);
                        state.complete = true;
                    }
                    accepted.push(better);
                    self.stamp(*net);
                }
                // A shorter route that cuts a pour's pad off its main piece
                // is no improvement (k30's +5V: 54 pads stranded at the
                // stitching after the polish, 3 without it).
                if let Some(stranded) = pour_stranded.as_mut()
                    && accepted.iter().any(|kept| *kept)
                {
                    let checked = std::time::Instant::now();
                    // Only pour nets whose planes come within a stamp's
                    // reach of the copper that changed can count otherwise
                    // than before; the others keep their count.
                    let mut changed: Option<(usize, usize, usize, usize)> = None;
                    for (index, net) in batch.iter().enumerate() {
                        if !accepted[index] {
                            continue;
                        }
                        for node in saved[index]
                            .iter()
                            .chain(&self.nets[*net as usize].branches)
                            .flat_map(|branch| &branch.nodes)
                        {
                            let (x, y) = self.grid.xy(node.cell as usize);
                            changed = Some(changed.map_or((x, y, x, y), |(x0, y0, x1, y1)| {
                                (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
                            }));
                        }
                    }
                    let now: Vec<usize> = pour_nets
                        .iter()
                        .zip(&pour_bounds)
                        .zip(stranded.iter())
                        .map(|((net, bounds), before)| {
                            let near = batch
                                .iter()
                                .enumerate()
                                .any(|(index, other)| accepted[index] && other == net)
                                || match (changed, bounds) {
                                    (Some((x0, y0, x1, y1)), &Some((px0, py0, px1, py1))) => {
                                        x0 <= px1 + pour_reach
                                            && px0 <= x1 + pour_reach
                                            && y0 <= py1 + pour_reach
                                            && py0 <= y1 + pour_reach
                                    }
                                    _ => false,
                                };
                            if near {
                                pour_analyses += 1;
                                self.pour_stranded_of(*net)
                            } else {
                                *before
                            }
                        })
                        .collect();
                    pour_check_seconds += checked.elapsed().as_secs_f64();
                    if now
                        .iter()
                        .zip(stranded.iter())
                        .any(|(now, before)| now > before)
                    {
                        for (index, net) in batch.iter().enumerate() {
                            if accepted[index] {
                                self.unstamp(*net);
                                let state = &mut self.nets[*net as usize];
                                state.branches = saved[index].clone();
                                state.connected.fill(true);
                                state.complete = true;
                                self.stamp(*net);
                                accepted[index] = false;
                            }
                        }
                        pour_rejected += 1;
                    } else {
                        *stranded = now;
                    }
                }
                for kept in accepted {
                    if kept {
                        improved = true;
                        improvements += 1;
                    }
                }
            }
            if !improved {
                break;
            }
        }
        self.cleanup = false;
        if self.config.verbose && pour_stranded.is_some() {
            eprintln!(
                "clean-up: {pour_rejected} batches kept out for stranding pour pads, {pour_check_seconds:.1} s of pour checks ({pour_analyses} analyses); {:.1} s of work of a budget of {budget:.0} s, {:.1} s of it pour analyses, in {:.1} s of wall clock",
                self.budget_work() - started.0,
                self.pour_work_seconds() - pour_work_before,
                started.1.elapsed().as_secs_f64()
            );
        }
        improvements
    }
}

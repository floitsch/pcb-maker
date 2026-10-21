// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Via reduction.

use super::*;

impl Router {
    /// Resolves what negotiation left, cleans up, stitches pours and
    /// materializes the copper.
    /// (open terminals, vias, lattice length) of the current state.
    pub(super) fn quality(&self) -> (usize, usize, f64) {
        let mut open = 0;
        let mut vias = 0;
        let mut length = 0.0;
        for (net, state) in self.nets.iter().enumerate() {
            if !state.routable {
                continue;
            }
            open += state.connected.iter().filter(|c| !**c).count();
            if !self.conflicts(net as NetId).is_empty() {
                open += 1;
            }
            let nx = self.grid.nx as i64;
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
        }
        (open, vias, length)
    }

    /// Renegotiates the nets that have vias with a higher via cost, from the
    /// converged state. A round is kept only if nothing opens and the via
    /// count drops.
    pub(super) fn reduce_vias(&mut self, order: &[NetId]) {
        // Fewer vias are worth nothing while connections are open, and the
        // renegotiation would take as long as the one that failed.
        if self.quality().0 > 0 {
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                for net in order {
                    let conflicts = self.conflicts(*net);
                    if !conflicts.is_empty() {
                        let spots: Vec<String> = conflicts
                            .iter()
                            .take(6)
                            .map(|(layer, cell)| {
                                let at = self.grid.center_of(*cell as usize);
                                format!("L{layer}({:.2},{:.2})", at[0], at[1])
                            })
                            .collect();
                        eprintln!(
                            "no via reduction: {} conflicts at {}",
                            self.board.nets[*net as usize].name,
                            spots.join(" ")
                        );
                    }
                }
            }
            return;
        }
        let mut budget = (self.config.via_reduction_budget * self.negotiation_seconds)
            .max(60.0)
            .min(self.config.via_reduction_seconds);
        if let Some((total, left_at_start)) = self.config.run_work {
            let left = left_at_start - self.work_seconds();
            if left > total / 2.0 {
                // At most 300 s: doubling the quick tier's via-reduction
                // time for 100 vias was too much; k30-SBC's 182 s round fits.
                let mut allowance = (left / 3.0).min(300.0);
                // A third of what is left before the deadline: of its work
                // when the caller counts it (the wall clock left would make
                // the budget depend on the machine's load), else of its
                // wall clock.
                if let Some(limit) = self.config.deadline_work {
                    allowance = allowance.min((limit - self.work_seconds()).max(0.0) / 3.0);
                } else if let Some(deadline) = self.config.deadline {
                    allowance = allowance.min(
                        deadline
                            .saturating_duration_since(std::time::Instant::now())
                            .as_secs_f64()
                            / 3.0,
                    );
                }
                if allowance > budget && self.config.verbose {
                    eprintln!(
                        "via reduction: {left:.0}s of the run's {total:.0}s left, a budget of {allowance:.0}s"
                    );
                }
                budget = budget.max(allowance);
            }
        }
        let started = self.clock();
        let strict = self.config.strict_via_budget;
        let mut last_round: Option<f64> = None;
        for round in 0..self.config.via_reduction_rounds {
            if self.spent(&started, budget) {
                if self.config.verbose {
                    eprintln!(
                        "via reduction: budget of {budget:.0}s used, stopping before round {round}"
                    );
                }
                break;
            }
            let left = budget - (self.budget_work() - started.0);
            if strict && let Some(cost) = last_round.filter(|cost| *cost > left) {
                if self.config.verbose {
                    eprintln!(
                        "via reduction: round {round} would take about {cost:.0}s of the {left:.0}s left of {budget:.0}s: stopping"
                    );
                }
                break;
            }
            let round_started = self.budget_work();
            let before = self.quality();
            let stranded_before = if self.config.pour_aware_via_reduction {
                self.pour_stranded()
            } else {
                Vec::new()
            };
            let starved_before = if self.config.via_reduction_keeps_spokes {
                self.starved_pads()
            } else {
                0
            };
            let pending: Vec<NetId> = order
                .iter()
                .copied()
                .filter(|net| {
                    self.nets[*net as usize].branches.iter().any(|branch| {
                        branch
                            .nodes
                            .windows(2)
                            .any(|pair| pair[0].cell == pair[1].cell)
                    })
                })
                .collect();
            if pending.is_empty() {
                break;
            }
            let snapshot = self.lean_snapshot();
            // After the snapshot: a revert restores the configuration too.
            let limits = (
                self.config.negotiation_seconds,
                self.config.negotiation_expansions,
                self.config.cleanup_seconds,
            );
            if strict {
                // The round's negotiation stops where the budget ends (a
                // cut round that leaves something open is reverted below).
                self.config.negotiation_expansions =
                    ((left * expansions_per_second()) as u64).max(1);
                self.config.negotiation_seconds =
                    self.config.negotiation_seconds.min(left * guard());
            }
            let via_cost = self.config.via_cost;
            let weight = self.config.heuristic_weight;
            self.config.via_cost =
                via_cost * self.config.via_reduction_factor.powi(round as i32 + 1);
            self.config.heuristic_weight = weight.max(self.config.via_reduction_weight);
            for net in &pending {
                self.rip_up(*net);
            }
            // The rerouted nets find their way around the settled ones
            // rather than through them.
            self.negotiate_from(
                order,
                pending.clone(),
                self.config.via_reduction_present as f32,
            );
            self.resolve_remaining(order);
            self.config.heuristic_weight = weight;
            if strict {
                // The clean-up gets what the negotiation left.
                let left = (budget - (self.budget_work() - started.0)).max(0.0);
                self.config.cleanup_seconds = self.config.cleanup_seconds.min(left);
            }
            // Only the nets this round touched can have got worse.
            let touched: Vec<NetId> = order
                .iter()
                .copied()
                .filter(|net| {
                    self.nets[*net as usize].reroutes > snapshot.nets[*net as usize].reroutes
                        || pending.contains(net)
                })
                .collect();
            self.clean_up(&touched);
            (
                self.config.negotiation_seconds,
                self.config.negotiation_expansions,
                self.config.cleanup_seconds,
            ) = limits;
            last_round = Some(self.budget_work() - round_started);
            self.config.via_cost = via_cost;
            let after = self.quality();
            let stranded_after = if self.config.pour_aware_via_reduction {
                self.pour_stranded()
            } else {
                Vec::new()
            };
            let starved_after = if self.config.via_reduction_keeps_spokes {
                self.starved_pads()
            } else {
                0
            };
            let keep = after.0 <= before.0
                && after.1 < before.1
                && stranded_after
                    .iter()
                    .zip(&stranded_before)
                    .all(|(after, before)| after <= before)
                && starved_after <= starved_before;
            if self.config.verbose {
                eprintln!(
                    "via reduction round {round}: {:?} -> {:?}, {starved_before} -> {starved_after} starved pads, {}",
                    before,
                    after,
                    if keep { "kept" } else { "reverted" }
                );
            }
            if !keep {
                // Restore, but still try the next (more expensive) round.
                self.restore_lean(snapshot);
            }
        }
    }

    /// A copy of the router to go back to, without what a negotiation
    /// leaves as it is (the static maps, the search scratch, the stamp
    /// marks, which are clear between stamps) or what is rebuilt from the
    /// rest (`hot`): a whole copy doubled the peak memory of every board
    /// (Castor: 1.39 GB for 0.51 GB of maps).
    pub(super) fn lean_snapshot(&mut self) -> Router {
        let statics = std::mem::take(&mut self.statics);
        let scratch = std::mem::take(&mut self.scratch);
        let hot = std::mem::take(&mut self.hot);
        let stamp_mark = std::mem::take(&mut self.stamp_mark);
        let snapshot = self.clone();
        self.statics = statics;
        self.scratch = scratch;
        self.hot = hot;
        self.stamp_mark = stamp_mark;
        snapshot
    }

    /// Goes back to a `lean_snapshot` taken since the static maps last
    /// changed.
    pub(super) fn restore_lean(&mut self, snapshot: Router) {
        let statics = std::mem::take(&mut self.statics);
        let scratch = std::mem::take(&mut self.scratch);
        let stamp_mark = std::mem::take(&mut self.stamp_mark);
        // The time spent stays spent.
        let profile = std::mem::take(&mut self.profile);
        let (search_seconds, stamp_seconds) = (self.search_seconds, self.stamp_seconds);
        *self = snapshot;
        self.profile = profile;
        (self.search_seconds, self.stamp_seconds) = (search_seconds, stamp_seconds);
        self.statics = statics;
        self.scratch = scratch;
        self.stamp_mark = stamp_mark;
        self.build_hot();
    }
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The negotiation's batches of nets with disjoint windows.

use super::*;

impl Router {
    /// Groups `pending` into batches of nets whose search windows (with a
    /// tile of margin) do not overlap, keeping the given order inside each
    /// batch. Pour nets and nets without a bounded window get a batch each.
    pub(super) fn batches(&self, pending: &[NetId], growth: f64) -> Vec<Vec<NetId>> {
        let full = (0, 0, self.grid.nx - 1, self.grid.ny - 1);
        let mut batches: Vec<(Vec<NetId>, Vec<CellWindow>)> = Vec::new();
        for net in pending {
            let window = self.window(*net, growth);
            let alone = window == full || !self.nets[*net as usize].plane.is_empty();
            let margin = TILE * 2;
            let rect = (
                window.0.saturating_sub(margin),
                window.1.saturating_sub(margin),
                window.2 + margin,
                window.3 + margin,
            );
            let overlaps = |other: &(usize, usize, usize, usize)| {
                rect.0 <= other.2 && rect.2 >= other.0 && rect.1 <= other.3 && rect.3 >= other.1
            };
            let slot = if alone {
                None
            } else {
                batches
                    .iter()
                    .position(|(members, rects)| members.len() < 64 && !rects.iter().any(overlaps))
            };
            match slot {
                Some(index) if !alone => {
                    batches[index].0.push(*net);
                    batches[index].1.push(rect);
                }
                _ => {
                    let rects = if alone { vec![full] } else { vec![rect] };
                    batches.push((vec![*net], rects));
                }
            }
        }
        batches.into_iter().map(|(members, _)| members).collect()
    }

    /// Routes one batch of the negotiation (see `batches`): rips up the
    /// conflicted branches of its nets, routes them (on the thread pool
    /// when there are several: their windows do not overlap), and stamps
    /// them.
    pub(super) fn route_batch(&mut self, batch: &[NetId], present: f32, growth: f64) {
        let search_started = std::time::Instant::now();
        for net in batch {
            self.rip_up_conflicted(*net);
        }
        // What a cut severs, from the fill without the batch's copper (its
        // old routes must not make their own corridors look cut already).
        self.refresh_sever();
        if batch.len() == 1 || !self.config.parallel {
            let single_started = std::time::Instant::now();
            for net in batch {
                self.route_net_seq(*net, present, false, growth);
            }
            self.profile.single_batches += 1;
            self.profile.single_seconds += single_started.elapsed().as_secs_f64();
        } else {
            // The nets of a batch do not overlap: routing them against
            // the same occupancy is the same as routing them in turn.
            let parallel_started = std::time::Instant::now();
            let pool = self.lend_scratches();
            let this: &Self = self;
            let routed: Vec<(NetId, NetState, [u64; 5], f64)> = batch
                .par_iter()
                .map(|net| {
                    pool.with(|scratch| {
                        let net_started = std::time::Instant::now();
                        let before = scratch.counters();
                        let mut net_state = this.nets[*net as usize].clone();
                        if !net_state.stamped.is_empty() {
                            this.mask_own(scratch, *net, &net_state);
                        }
                        this.route_net(scratch, *net, &mut net_state, present, false, growth);
                        scratch.unmask_own();
                        let after = scratch.counters();
                        let mut delta = [0u64; 5];
                        for (index, value) in delta.iter_mut().enumerate() {
                            *value = after[index] - before[index];
                        }
                        (*net, net_state, delta, net_started.elapsed().as_secs_f64())
                    })
                })
                .collect();
            self.return_scratches(pool);
            self.profile.parallel_batches += 1;
            self.profile.parallel_nets += batch.len() as u64;
            self.profile.parallel_seconds += parallel_started.elapsed().as_secs_f64();
            for (net, net_state, counters, seconds) in routed {
                self.profile.parallel_net_seconds += seconds;
                self.nets[net as usize] = net_state;
                self.scratch.searches += counters[0];
                self.scratch.expansions += counters[1];
                self.scratch.open_searches += counters[2];
                self.scratch.open_expansions += counters[3];
                self.scratch.failed_searches += counters[4];
            }
        }
        for net in batch {
            self.nets[*net as usize].reroutes += 1;
        }
        self.search_seconds += search_started.elapsed().as_secs_f64();
        let stamp_started = std::time::Instant::now();
        for net in batch {
            self.stamp(*net);
        }
        self.stamp_seconds += stamp_started.elapsed().as_secs_f64();
    }
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The negotiation (PathFinder style).

use super::*;

impl Router {
    /// Negotiated congestion over `pending` (which grows to whatever those
    /// nets conflict with) until the board is conflict free or stalls.
    pub(super) fn negotiate(&mut self, order: &[NetId], pending: Vec<NetId>) {
        self.negotiate_from(order, pending, self.config.present_factor as f32);
    }

    /// `negotiate`, with sharing priced at `present` from the start.
    pub(super) fn negotiate_from(
        &mut self,
        order: &[NetId],
        mut pending: Vec<NetId>,
        present: f32,
    ) {
        let started = std::time::Instant::now();
        let profile_started = started;
        // The via reduction negotiates too: its time is its own phase.
        let nested = self.in_via_reduction;
        let expansions_before = self.scratch.expansions;
        let work_before = self.work_seconds();
        let phase_mark = self.phase_mark();
        let mut present = present;
        let mut best_conflicted = usize::MAX;
        let mut stalled = 0;
        for state in &mut self.nets {
            state.hopeless_searches = 0;
            state.unreachable.clear();
        }
        // Past the caller's wall-clock deadline a router that has negotiated
        // before (a resume, a move's trial, the via reduction) keeps its
        // routes as they are and goes on to finish: an iteration of a big
        // board takes a minute or more under load, and after the deadline
        // only the result is wanted (PolyKybd left in v16: three trials of
        // 60-200 s each after the deadline, killed at 1800 s). A fresh
        // router still negotiates once: without it there is no board.
        let finishing = self.iterations > 0 && self.past_wall_deadline();
        if finishing && self.config.verbose {
            eprintln!("negotiation skipped: past the deadline, the routes stay as they are");
        }
        for iteration in 0..if finishing {
            0
        } else {
            self.config.max_iterations
        } {
            self.iterations += 1;
            let growth = 1.0 + iteration as f64 / 6.0;
            let batches = self.batches(&pending, growth);
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                let sizes: Vec<String> = batches
                    .iter()
                    .map(|members| members.len().to_string())
                    .collect();
                eprintln!(
                    "  {} nets in {} batches: {:?}",
                    pending.len(),
                    batches.len(),
                    sizes
                );
            }
            for batch in &batches {
                self.route_batch(batch, present, growth);
            }
            let conflicts_started = std::time::Instant::now();
            let mut conflicted = Vec::new();
            for net in order {
                let conflicts = self.conflicts(*net);
                if std::env::var_os("PCB_ROUTER_DEBUG").is_some()
                    && (order.len() <= 3 || iteration % 10 == 9)
                    && !conflicts.is_empty()
                {
                    let spots: Vec<String> = conflicts
                        .iter()
                        .take(8)
                        .map(|(layer, cell)| {
                            let at = self.grid.center_of(*cell as usize);
                            let map = self.map_index(self.node_class(*net, *cell), *layer) as u32;
                            let others: Vec<&str> = (0..self.nets.len())
                                .filter(|other| *other != *net as usize)
                                .filter(|other| self.nets[*other].stamped.contains(&(map, *cell)))
                                .map(|other| self.board.nets[other].name.as_str())
                                .collect();
                            format!(
                                "L{layer}({:.2},{:.2})x{}[{}]",
                                at[0],
                                at[1],
                                self.occupancy[map as usize][*cell as usize],
                                others.join(",")
                            )
                        })
                        .collect();
                    eprintln!(
                        "  {} conflicts at {}",
                        self.board.nets[*net as usize].name,
                        spots.join(" ")
                    );
                }
                if conflicts.is_empty() {
                    self.nets[*net as usize].stuck = 0;
                } else {
                    self.nets[*net as usize].stuck += 1;
                }
                if conflicts.is_empty()
                    && (self.nets[*net as usize].complete || self.nets[*net as usize].blocked)
                {
                    continue;
                }
                let mut tiles_hit: HashSet<(usize, usize)> = HashSet::new();
                let layers = self.board.layer_count;
                for (layer, cell) in conflicts {
                    self.history[layer][cell as usize] += self.config.history_increment as f32;
                    if layer < layers {
                        for class in 0..self.statics.len() {
                            if let Some(hot) =
                                self.hot[class * layers + layer].get_mut(cell as usize)
                            {
                                hot.history += self.config.history_increment as f32;
                            }
                        }
                    }
                    let cell = cell as usize;
                    tiles_hit.insert((
                        layer,
                        (cell / self.grid.nx / TILE) * self.tiles_x + (cell % self.grid.nx) / TILE,
                    ));
                }
                for (layer, tile) in tiles_hit {
                    self.tile_history[layer][tile] += self.config.history_increment as f32;
                }
                conflicted.push(*net);
            }
            self.profile.conflicts += conflicts_started.elapsed().as_secs_f64();
            if let Some(hook) = self.frame_hook.clone() {
                hook(self.iterations, &self.snapshot());
            }
            if self.config.verbose {
                eprintln!(
                    "iteration {iteration}: rerouted {}, conflicted {}, present {present:.2}, {:.2}s (search {:.2}s, stamp {:.2}s, {} searches, {}M expansions; {} open searches with {}M, {} failed)",
                    pending.len(),
                    conflicted.len(),
                    started.elapsed().as_secs_f64(),
                    self.search_seconds,
                    self.stamp_seconds,
                    self.scratch.searches,
                    self.scratch.expansions / 1_000_000,
                    self.scratch.open_searches,
                    self.scratch.open_expansions / 1_000_000,
                    self.scratch.failed_searches
                );
            }
            // Pads of a pour net that the signals cut off from the main
            // piece this iteration are routed to it from the next one on.
            // Progress is measured without them: the pads come and go with
            // the signals, and a new low by their count kept ESC mini's
            // connect rung negotiating 80 iterations instead of 53.
            let really_conflicted = conflicted.len();
            if self.config.pour_islands && iteration >= 2 {
                let refresh_started = std::time::Instant::now();
                let mut stranded = 0;
                for net in order {
                    let newly = self.refresh_pour(*net);
                    stranded += newly;
                    if newly > 0 && !conflicted.contains(net) {
                        conflicted.push(*net);
                    }
                }
                self.profile.refresh_pour += refresh_started.elapsed().as_secs_f64();
                if self.config.verbose && stranded > 0 {
                    eprintln!(
                        "  {stranded} pour pads cut off from their main piece, routed from now on ({:.2}s)",
                        refresh_started.elapsed().as_secs_f64()
                    );
                }
            }
            self.last_conflicted = conflicted.clone();
            if conflicted.is_empty() {
                break;
            }
            // The plan did not survive the lattice where nets conflict:
            // those tiles lose room on the graph and the nets are planned
            // again around them.
            if self.global.is_some() && iteration % 2 == 1 {
                let replan_started = std::time::Instant::now();
                self.replan(&conflicted);
                self.profile.replan += replan_started.elapsed().as_secs_f64();
            }
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() && iteration % 10 == 9 {
                // The most contested tiles: where history piled up.
                let mut tiles: Vec<(f32, usize, usize)> = Vec::new();
                for (layer, history) in self.tile_history.iter().enumerate() {
                    for (tile, value) in history.iter().enumerate() {
                        if *value > 0.0 {
                            tiles.push((*value, layer, tile));
                        }
                    }
                }
                tiles.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                let hot: Vec<String> = tiles
                    .iter()
                    .take(8)
                    .map(|(value, layer, tile)| {
                        let x = (tile % self.tiles_x) as f64 * TILE as f64 * self.grid.pitch
                            + self.grid.origin[0];
                        let y = (tile / self.tiles_x) as f64 * TILE as f64 * self.grid.pitch
                            + self.grid.origin[1];
                        format!("L{layer}({x:.0},{y:.0}):{value:.0}")
                    })
                    .collect();
                eprintln!("  hottest tiles: {}", hot.join(" "));
                let names: Vec<String> = conflicted
                    .iter()
                    .take(24)
                    .map(|net| {
                        let state = &self.nets[*net as usize];
                        format!(
                            "{}{}",
                            self.board.nets[*net as usize].name,
                            if state.complete { "" } else { "!" }
                        )
                    })
                    .collect();
                eprintln!("  conflicted (! = incomplete): {}", names.join(" "));
                if conflicted.len() <= 4 || iteration % 10 == 9 {
                    for net in conflicted.iter().take(4) {
                        let terminals: Vec<String> = self.board.nets[*net as usize]
                            .terminals
                            .iter()
                            .map(|t| format!("({:.2},{:.2})", t.anchor[0], t.anchor[1]))
                            .collect();
                        eprintln!(
                            "    {} terminals {}",
                            self.board.nets[*net as usize].name,
                            terminals.join(" ")
                        );
                    }
                    self.dump_region(&conflicted);
                }
            }
            let at_cap = present >= self.config.present_cap as f32;
            let progress =
                if at_cap && self.config.stall_drop > 0.0 && best_conflicted != usize::MAX {
                    let needed =
                        ((best_conflicted as f64 * self.config.stall_drop).ceil() as usize).max(2);
                    really_conflicted + needed <= best_conflicted
                } else {
                    really_conflicted < best_conflicted
                };
            if progress {
                best_conflicted = really_conflicted;
                stalled = 0;
            } else {
                // The low to beat stays where progress was last made: small
                // steps add up until they make a real drop.
                stalled += 1;
            }
            pending = conflicted;
            present =
                (present * self.config.present_growth as f32).min(self.config.present_cap as f32);
            // At the price cap only history still moves anything: give up
            // sooner there.
            let patience = if present >= self.config.present_cap as f32 {
                self.config.stall_at_cap
            } else {
                self.config.stall_patience
            };
            // Work, not seconds, when the caller gives it: the same board
            // then routes the same way however busy the machine is; the
            // seconds stay as the last guard.
            let worked_out = self.config.negotiation_expansions > 0
                && self.scratch.expansions - expansions_before > self.config.negotiation_expansions;
            if stalled > patience || worked_out || self.past_work_deadline() {
                break;
            }
            if self.wall_guard()
                && started.elapsed().as_secs_f64() > self.config.negotiation_seconds
            {
                note_wall_cut(
                    "negotiation guard",
                    format_args!(
                        "{:.0} s of wall clock, {:.0} s of work",
                        started.elapsed().as_secs_f64(),
                        (self.scratch.expansions - expansions_before) as f64
                            / EXPANSIONS_PER_SECOND
                    ),
                );
                break;
            }
            if self.past_wall_deadline() {
                break;
            }
            // Nets without any path are not congestion; when a quarter of
            // the board has none after the first iterations, this rung of
            // the ladder cannot succeed and the next one should get the
            // time (video's exclusive-plane rung: 275 open searches from
            // iteration 0, 895 s).
            if iteration >= 1 && self.config.abandon_hopeless {
                let (routable, pathless) = self.nets.iter().filter(|state| state.routable).fold(
                    (0usize, 0usize),
                    |(routable, pathless), state| {
                        (
                            routable + 1,
                            pathless + usize::from(!state.complete && state.blocked),
                        )
                    },
                );
                if pathless * 4 > routable {
                    if self.config.verbose {
                        eprintln!("hopeless: {pathless} of {routable} nets have no path at all");
                    }
                    self.hopeless = true;
                    break;
                }
            }
        }
        // In work seconds: the budgets of the repair, the clean-up and the
        // via reduction follow from it.
        self.negotiation_seconds = self.work_seconds() - work_before;
        self.present_reached = present;
        if !nested {
            self.profile.negotiation += profile_started.elapsed().as_secs_f64();
            self.phase_charge(0, phase_mark);
        }
    }
}

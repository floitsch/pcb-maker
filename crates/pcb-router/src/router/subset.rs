// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The trouble-maker probe (a research measure, docs/placer.md, "The
//! trouble-maker probe (2026-10-26)"): a set of nets negotiated alone, the
//! board's other nets present only as a background (fixed escape stubs out
//! of their pads, or a cost from the global route's tile usage), so that a
//! placement's difficulty is measured in a fraction of a whole-board probe.

use super::*;

/// One net of a subset probe.
#[derive(Clone, Debug)]
pub struct SubsetNet {
    pub net: NetId,
    /// Search expansions spent on the net in the probe.
    pub expansions: u64,
    /// How often the negotiation routed it.
    pub reroutes: usize,
    pub complete: bool,
    /// Its nodes inside another net's clearance zone at the end.
    pub conflict_cells: usize,
    /// Terminals not connected to the net (one for an unreachable net).
    pub open_terminals: usize,
}

/// What a subset probe left (`Router::probe_subset`).
#[derive(Clone, Debug, Default)]
pub struct SubsetStanding {
    /// The open terminals of the set, plus one for every complete net of
    /// it still in conflict (as `probe_standing` counts the board).
    pub open_terminals: usize,
    /// Nets of the set incomplete or in conflict.
    pub unfinished: usize,
    /// Nets of the set in conflict.
    pub conflicted: usize,
    /// Their conflict cells.
    pub conflict_cells: usize,
    pub iterations: usize,
    /// Why the negotiation ended (`RoutingResult::negotiation_stop`).
    pub stop: &'static str,
    /// The negotiation's search expansions.
    pub expansions: u64,
    pub nets: Vec<SubsetNet>,
}

impl Router {
    /// Fixes a straight stub of up to `length_mm` out of every pad of the
    /// routable nets outside `subset` (pour nets left out: their pads reach
    /// the pour), towards the nearest other pad of the net along the
    /// larger of its two offsets, as the copper the net will need there at
    /// least. Stops before blocked nodes and copper already stamped.
    /// Returns the stubs fixed.
    pub fn stamp_background_stubs(&mut self, subset: &[NetId], length_mm: f64) -> usize {
        let steps = (length_mm / self.grid.pitch).round().max(1.0) as i64;
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        let mut in_set = vec![false; self.nets.len()];
        for net in subset {
            in_set[*net as usize] = true;
        }
        let mut fixed = 0;
        for net in 0..self.nets.len() as NetId {
            let state = &self.nets[net as usize];
            let terminals = &self.board.nets[net as usize].terminals;
            if in_set[net as usize]
                || !state.routable
                || !state.plane.is_empty()
                || !state.branches.is_empty()
                || terminals.len() < 2
            {
                continue;
            }
            let mut branches = Vec::new();
            for (index, terminal) in terminals.iter().enumerate() {
                let Some(start) = state.terminal_nodes[index].first().copied() else {
                    continue;
                };
                // The nearest other pad says which way the copper leaves.
                let Some(target) = terminals
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != index)
                    .map(|(_, other)| other.anchor)
                    .min_by(|a, b| {
                        crate::geometry::distance(terminal.anchor, *a)
                            .total_cmp(&crate::geometry::distance(terminal.anchor, *b))
                    })
                else {
                    continue;
                };
                let at = self.grid.center_of(start.cell as usize);
                let (dx, dy) = (target[0] - at[0], target[1] - at[1]);
                let (step_x, step_y) = if dx.abs() >= dy.abs() {
                    (if dx >= 0.0 { 1 } else { -1 }, 0)
                } else {
                    (0, if dy >= 0.0 { 1 } else { -1 })
                };
                let layer = start.layer as usize;
                let mut path = vec![start];
                let (mut x, mut y) = (start.cell as i64 % nx, start.cell as i64 / nx);
                for _ in 0..steps {
                    x += step_x;
                    y += step_y;
                    if x < 0 || y < 0 || x >= nx || y >= ny {
                        break;
                    }
                    let cell = y * nx + x;
                    let class = self.node_class(net, cell as u32);
                    if !self.statics[class].trace_allowed(layer, cell as usize, net)
                        || self
                            .occupancy
                            .get(self.map_index(class, layer), cell as usize)
                            > 0
                    {
                        break;
                    }
                    path.push(Node {
                        layer: layer as u8,
                        cell: cell as u32,
                    });
                }
                if path.len() >= 2 {
                    branches.push(Branch {
                        nodes: path,
                        start_terminal: index as u16,
                        end_terminal: FREE_END,
                    });
                }
            }
            if branches.is_empty() {
                continue;
            }
            fixed += branches.len();
            let state = &mut self.nets[net as usize];
            state.branches = branches;
            state.fixed = state.branches.len();
            self.stamp(net);
        }
        fixed
    }

    /// Adds `cost(layer, point)` (a node's centre) to every node's history
    /// on the copper layers, and its mean per router tile to the tiles'
    /// history (the corridor planner's), as a standing price of the
    /// copper the board's other nets will need there.
    pub fn add_background_cost(&mut self, cost: &dyn Fn(usize, [f64; 2]) -> f32) {
        let layers = self.board.layer_count;
        let nx = self.grid.nx;
        for layer in 0..layers {
            let mut tile_sum = vec![0.0f32; self.tiles_x * self.tiles_y];
            let mut tile_count = vec![0u32; self.tiles_x * self.tiles_y];
            for cell in 0..self.grid.cells() {
                let value = cost(layer, self.grid.center_of(cell));
                if value <= 0.0 {
                    continue;
                }
                self.history[layer][cell] += value;
                let tile = (cell / nx / TILE) * self.tiles_x + (cell % nx) / TILE;
                if let Some(sum) = tile_sum.get_mut(tile) {
                    *sum += value;
                    tile_count[tile] += 1;
                }
            }
            for (tile, sum) in tile_sum.iter().enumerate() {
                if tile_count[tile] > 0 {
                    self.tile_history[layer][tile] += sum / (TILE * TILE) as f32;
                }
            }
        }
        self.build_hot();
    }

    /// Negotiates the nets of `subset` alone (in the routing order) for at
    /// most `seconds` of work, as `probe` negotiates the board, and reports
    /// how far they got. The other nets are not routed: what they leave of
    /// the board is whatever `stamp_background_stubs` or
    /// `add_background_cost` put there before.
    pub fn probe_subset(&mut self, subset: &[NetId], seconds: f64) -> SubsetStanding {
        self.ensure_buses();
        let mut in_set = vec![false; self.nets.len()];
        for net in subset {
            if (*net as usize) < in_set.len() {
                in_set[*net as usize] = true;
            }
        }
        let order: Vec<NetId> = self
            .routing_order()
            .into_iter()
            .filter(|net| in_set[*net as usize])
            .collect();
        let started = std::time::Instant::now();
        self.fix_escapes(&order);
        self.route_skeletons(&order);
        self.fix_plane_stubs(&order);
        self.profile.prepare += started.elapsed().as_secs_f64();
        let limit = (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
        );
        self.config.negotiation_seconds = seconds * guard();
        self.config.negotiation_expansions = negotiation_expansions(seconds);
        let expansions_before = self.scratch.expansions;
        let iterations_before = self.iterations;
        self.in_probe = true;
        self.negotiate(&order, order.clone());
        self.in_probe = false;
        (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
        ) = limit;
        let statuses = self.statuses();
        let mut standing = SubsetStanding {
            iterations: self.iterations - iterations_before,
            stop: self.negotiation_stop,
            expansions: self.scratch.expansions - expansions_before,
            ..SubsetStanding::default()
        };
        for net in &order {
            let state = &self.nets[*net as usize];
            let conflict_cells = self.conflicts(*net).len();
            let open = match statuses[*net as usize] {
                NetStatus::Partial {
                    unconnected_terminals,
                } => unconnected_terminals,
                NetStatus::Unreachable => 1,
                _ => 0,
            };
            standing.open_terminals += open + usize::from(state.complete && conflict_cells > 0);
            standing.unfinished += usize::from(!state.complete || conflict_cells > 0);
            standing.conflicted += usize::from(conflict_cells > 0);
            standing.conflict_cells += conflict_cells;
            standing.nets.push(SubsetNet {
                net: *net,
                expansions: state.expansions,
                reroutes: state.reroutes,
                complete: state.complete,
                conflict_cells,
                open_terminals: open,
            });
        }
        standing
    }
}

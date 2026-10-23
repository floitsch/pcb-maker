// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Global routing: every net planned on the tile graph first.

use super::*;

impl Router {
    /// Builds the tile graph and negotiates every net's plan on it (pour
    /// nets are left to the lattice).
    pub(super) fn plan_globally(&mut self, order: &[NetId]) {
        let started = std::time::Instant::now();
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        // The reference class: the one most nets use.
        let mut counts = vec![0usize; self.board.classes.len()];
        for net in order {
            counts[self.board.nets[*net as usize].class] += 1;
        }
        let reference = (0..counts.len())
            .max_by_key(|&class| counts[class])
            .unwrap_or(0);
        let rules = self.board.classes[reference];
        let free: Vec<Vec<bool>> = (0..layers)
            .map(|layer| {
                self.statics[reference].trace[layer]
                    .iter()
                    .map(|&value| value == crate::grid::FREE16)
                    .collect()
            })
            .collect();
        let via_free: Vec<bool> = self.statics[reference]
            .via_blocked
            .iter()
            .map(|blocked| !blocked)
            .collect();
        let tiles_x = self.tiles_x;
        let tiles = self.tiles_x * self.tiles_y;
        let tile_of = |cell: u32| {
            (cell as usize / self.grid.nx / TILE) * tiles_x + (cell as usize % self.grid.nx) / TILE
        };
        let nets: Vec<crate::global::GlobalNet> = (0..self.board.nets.len())
            .map(|net| {
                let state = &self.nets[net];
                if !state.routable || !state.plane.is_empty() {
                    return crate::global::GlobalNet::default();
                }
                let class = self.board.classes[self.board.nets[net].class];
                crate::global::GlobalNet {
                    terminals: state
                        .terminal_nodes
                        .iter()
                        .map(|nodes| {
                            let mut list: Vec<usize> = nodes
                                .iter()
                                .map(|node| node.layer as usize * tiles + tile_of(node.cell))
                                .collect();
                            list.sort_unstable();
                            list.dedup();
                            list
                        })
                        .collect(),
                    demand: ((class.trace_width + class.clearance)
                        / (rules.trace_width + rules.clearance)) as f32,
                    ..Default::default()
                }
            })
            .collect();
        let inputs = crate::global::Inputs {
            nx: self.grid.nx,
            ny: self.grid.ny,
            tile: TILE,
            pitch: self.grid.pitch,
            layers,
            free: free.iter().map(Vec::as_slice).collect(),
            via_free: &via_free,
            room: rules.trace_width + rules.clearance,
            via_cost: self.config.via_cost,
        };
        let mut plan = crate::global::GlobalPlan::new(&inputs, nets);
        let planned: Vec<usize> = order
            .iter()
            .map(|net| *net as usize)
            .filter(|&net| {
                plan.nets[net]
                    .terminals
                    .iter()
                    .filter(|terminal| !terminal.is_empty())
                    .count()
                    > 1
            })
            .collect();
        let overflow = plan.negotiate(&planned, 40);
        let _ = cells;
        if self.config.verbose {
            eprintln!(
                "global plan: {} nets on {} x {} tiles x {layers} layers, overflow {overflow:.1} tracks, {:.2}s",
                planned.len(),
                self.tiles_x,
                self.tiles_y,
                started.elapsed().as_secs_f64()
            );
        }
        self.global = Some(plan);
    }

    /// Teaches the plan where `conflicted` nets collide on the lattice and
    /// plans them again.
    pub(super) fn replan(&mut self, conflicted: &[NetId]) {
        let tiles_x = self.tiles_x;
        let nx = self.grid.nx;
        let mut hits: Vec<(usize, usize)> = Vec::new();
        for net in conflicted {
            for (layer, cell) in self.conflicts(*net) {
                let cell = cell as usize;
                hits.push((layer, (cell / nx / TILE) * tiles_x + (cell % nx) / TILE));
            }
        }
        hits.sort_unstable();
        hits.dedup();
        let Some(plan) = self.global.as_mut() else {
            return;
        };
        for (layer, tile) in hits {
            if layer < plan.layers {
                plan.learn(layer, tile, 0.5);
            }
        }
        for net in conflicted {
            let net = *net as usize;
            if plan.nets[net].terminals.is_empty() {
                continue;
            }
            plan.rip_up(net);
            plan.route(net, 4.0);
        }
    }

    /// Nets in routing order: pour nets first (their connections are local
    /// stubs and vias that must claim the sites next to their pads), then
    /// small nets by span, then the large ones (with `bus_bundles` the
    /// buses' members before them).
    pub(super) fn routing_order(&self) -> Vec<NetId> {
        let mut order: Vec<NetId> = (0..self.board.nets.len() as NetId)
            .filter(|net| self.nets[*net as usize].routable)
            .collect();
        let span = |router: &Self, net: NetId| {
            let (x0, y0, x1, y1) = router.window(net, 0.0);
            (x1 - x0) + (y1 - y0)
        };
        let seed = self.config.seed.or_else(experiment_seed);
        let mut state = seed.unwrap_or(0) ^ 0x5DEE_CE66;
        let jitter: Vec<f64> = (0..self.board.nets.len())
            .map(|_| match seed {
                Some(_) => 0.6 + 0.8 * unit_noise(&mut state),
                None => 1.0,
            })
            .collect();
        // Bus bundles (off: every net is `(1, 0, 0)`): the members after
        // the pour nets, bus by bus, in their order.
        // `PCB_ROUTER_BUS_NO_ORDER` (an ablation): the usual order.
        let keep_order = std::env::var_os("PCB_ROUTER_BUS_NO_ORDER").is_some();
        let bus_rank = |net: NetId| match self.bus_member(net).filter(|_| !keep_order) {
            Some((bus, position)) => (0, bus, position),
            None => (1, 0, 0),
        };
        order.sort_by_key(|net| {
            (
                self.nets[*net as usize].plane.is_empty(),
                bus_rank(*net),
                self.board.nets[*net as usize].terminals.len() > 8,
                (span(self, *net) as f64 * jitter[*net as usize]) as u64,
                *net,
            )
        });
        order
    }
}

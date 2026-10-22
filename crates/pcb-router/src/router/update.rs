// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Changes from outside: a moved board, existing copper, rerouting.

use super::*;

impl Router {
    /// Replaces the board (same nets and rule classes, other fixed copper or
    /// terminal positions) and reroutes only what the change invalidated:
    /// nets whose pads moved and nets whose copper now collides with fixed
    /// objects. Returns the number of nets rerouted. Everything else keeps
    /// its copper, occupancy, and history.
    pub fn update(&mut self, board: &Board) -> Result<usize, String> {
        let (board, neck_of, class_rules) = Self::with_neck_classes(board, &self.config);
        let board = &board;
        let same_rules = match (&class_rules, &self.class_rules) {
            (None, None) => true,
            (Some(new), Some(old)) => {
                new.class_group == old.class_group
                    && new.class_floor == old.class_floor
                    && new.resolved.net_group == old.resolved.net_group
            }
            _ => false,
        };
        if neck_of != self.neck_of || !same_rules {
            return Err("incremental update needs the same nets and rules".into());
        }
        if board.nets.len() != self.board.nets.len()
            || board.classes != self.board.classes
            || board.layer_count != self.board.layer_count
            || board.nets.iter().zip(&self.board.nets).any(|(new, old)| {
                new.name != old.name || new.terminals.len() != old.terminals.len()
            })
        {
            return Err("incremental update needs the same nets and rules".into());
        }
        let previous: Vec<NetState> = std::mem::take(&mut self.nets);
        // Occupancy is rebuilt from the retained branches below.
        for map in &mut self.occupancy {
            map.fill(0);
        }
        for map in &mut self.tile_claimed {
            map.fill(0);
        }
        self.board = board.clone();
        self.probe_shapes = Arc::default();
        self.class_rules = class_rules;
        self.searched = searched_classes(&self.board, &self.neck_of);
        self.statics_shared = false;
        self.statics_key = None;
        self.statics = Arc::new(
            (0..self.board.classes.len())
                .map(|class| {
                    StaticMaps::build_with_rules(
                        &self.board,
                        &self.grid,
                        class,
                        self.searched[class],
                        self.class_rules.as_deref(),
                    )
                })
                .collect(),
        );
        let layers = self.board.layer_count;
        let tiles = self.tiles_x * self.tiles_y;
        self.tile_routable = vec![vec![0u32; tiles]; self.board.classes.len() * layers];
        for class in 0..self.board.classes.len() {
            for layer in 0..layers {
                for (cell, value) in self.statics[class].trace[layer].iter().enumerate() {
                    if *value != crate::grid::BLOCKED16 {
                        self.tile_routable[class * layers + layer][(cell / self.grid.nx / TILE)
                            * self.tiles_x
                            + (cell % self.grid.nx) / TILE] += 1;
                    }
                }
            }
        }
        self.nets = (0..self.board.nets.len())
            .map(|net| self.prepare_net(net as NetId))
            .collect();
        self.guard = self.thermal_guards();
        self.covered = vec![Vec::new(); layers];
        self.tile_covered = vec![vec![0.0; tiles]; layers];
        self.mark_covered();
        self.mark_via_cells();
        self.build_hot();

        let mut rerouted = 0;
        for (net, old) in previous.into_iter().enumerate() {
            let id = net as NetId;
            let same_terminals = old.terminal_nodes == self.nets[net].terminal_nodes;
            let legal = same_terminals
                && old
                    .branches
                    .iter()
                    .all(|branch| self.branch_is_legal(id, branch));
            if legal && !old.branches.is_empty() {
                let state = &mut self.nets[net];
                state.branches = old.branches;
                state.connected = old.connected;
                state.complete = old.complete;
                state.reroutes = old.reroutes;
                state.fixed = old.fixed;
                self.stamp(id);
            } else if self.nets[net].routable {
                rerouted += 1;
            }
        }
        Ok(rerouted)
    }

    /// Routes whatever is unrouted or in conflict after `update`, then
    /// finishes as `run` does. Cheap when little changed. Without `polish`
    /// the per-net cleanup pass is skipped (for trials).
    pub fn reroute(&mut self, polish: bool) -> RoutingResult {
        let order = self.routing_order();
        self.fix_escapes(&order);
        self.route_skeletons(&order);
        let pending: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| {
                let state = &self.nets[*net as usize];
                !state.complete || !self.conflicts(*net).is_empty()
            })
            .collect();
        // PCB_ROUTER_TIMING: where a reroute's time goes (layout moves).
        let timing = std::env::var_os("PCB_ROUTER_TIMING").is_some();
        let (started, pending_count, iterations_before) =
            (std::time::Instant::now(), pending.len(), self.iterations);
        // At the price of sharing the negotiation reached, as `resume`: a
        // price started low again undid what was settled (the layout's
        // polish turned eurorack's 21 open into 42, PolyKybd's 33 into
        // 108, without any deadline).
        let present = self.present_reached.max(self.config.present_factor as f32);
        self.negotiate_from(&order, pending, present);
        // The nets rerouted may cut a pour's fill where another net's plane
        // stub ends; negotiation refreshes the pours only from its third
        // iteration, which a reroute may not reach (USB_Cable_Tester: a
        // moved capacitor's signals ran over /SHLD's plane end, a dangling
        // track in KiCad). Pads cut off so are routed to the main piece.
        if self.config.pour_islands {
            let stranded: Vec<NetId> = order
                .iter()
                .copied()
                .filter(|net| self.refresh_pour(*net) > 0)
                .collect();
            if !stranded.is_empty() {
                if self.config.verbose {
                    eprintln!(
                        "reroute: {} pour nets with pads cut off, routed again",
                        stranded.len()
                    );
                }
                self.negotiate_from(&order, stranded, present);
            }
        }
        let negotiated = started.elapsed().as_secs_f64();
        let result = self.finish_polished(&order, polish);
        if timing {
            eprintln!(
                "reroute: {pending_count} pending, {} iterations, negotiation {negotiated:.1} s, finish {:.1} s",
                self.iterations - iterations_before,
                started.elapsed().as_secs_f64() - negotiated
            );
        }
        result
    }

    /// Routes one net with the router's own scratch (sequential callers).
    pub(super) fn route_net_seq(
        &mut self,
        net: NetId,
        present: f32,
        hard: bool,
        window_growth: f64,
    ) {
        let mut scratch = std::mem::take(&mut self.scratch);
        let mut net_state = std::mem::take(&mut self.nets[net as usize]);
        // Fixed branches stay stamped through a rip-up; the search must
        // not take its own for another net's.
        if !net_state.stamped.is_empty() {
            self.mask_own(&mut scratch, net, &net_state);
        }
        self.route_net(
            &mut scratch,
            net,
            &mut net_state,
            present,
            hard,
            window_growth,
        );
        scratch.unmask_own();
        self.nets[net as usize] = net_state;
        self.scratch = scratch;
    }

    /// Writes the pour map of every pour net into `directory`, one file
    /// per net and layer (`<net>-L<layer>.pour`): a JSON header line (net,
    /// layer, nx, ny, origin, pitch, pieces), then one flag byte per node
    /// (row-major, x fastest) and one little-endian u32 piece label per
    /// node. Flags: 1 inside the pour's polygon (less higher-priority
    /// pours), 2 the static map lets the fill in, 4 no other net's routed
    /// copper stamps the node, 8 inside a thermal ring, 16 free (what the
    /// pieces are labelled from), 32 a node of the net's own branches.
    /// For overlaying KiCad's fill (experiments/pour-fidelity).
    /// For `dump_pours` on a routed board: the pour nets' existing tracks
    /// and vias (fixed obstacles of their own net) become branches, so the
    /// pour model sees them as copper of the net in its pieces, as it sees
    /// its own routes. Branch ends join the branches they meet; no stamping (the obstacles
    /// already hold their nodes for the net). Analysis only.
    pub fn adopt_existing_copper(&mut self) {
        let layers = self.board.layer_count;
        for obstacle in &self.board.obstacles {
            let Some(net) = obstacle.net else {
                continue;
            };
            if obstacle.kind != crate::board::ObstacleKind::Copper
                || !(obstacle.label == "existing track" || obstacle.label == "existing via")
                || self.nets[net as usize].plane.is_empty()
            {
                continue;
            }
            let mut nodes: Vec<Node> = Vec::new();
            match &obstacle.shape {
                crate::geometry::Shape::Capsule { start, end, .. } => {
                    let length = crate::geometry::distance(*start, *end);
                    let steps = ((length / (self.grid.pitch / 4.0)).ceil() as usize).max(1);
                    for layer in (0..layers).filter(|layer| obstacle.layers & (1 << layer) != 0) {
                        for step in 0..=steps {
                            let t = step as f64 / steps as f64;
                            let at = [
                                start[0] + (end[0] - start[0]) * t,
                                start[1] + (end[1] - start[1]) * t,
                            ];
                            if let Some(cell) = self.grid.nearest_node(at) {
                                let node = Node {
                                    layer: layer as u8,
                                    cell: cell as u32,
                                };
                                if nodes.last() != Some(&node) {
                                    nodes.push(node);
                                }
                            }
                        }
                    }
                }
                shape => {
                    let bounds = shape.aabb();
                    let center = [
                        (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                        (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
                    ];
                    if let Some(cell) = self.grid.nearest_node(center) {
                        for layer in (0..layers).filter(|layer| obstacle.layers & (1 << layer) != 0)
                        {
                            nodes.push(Node {
                                layer: layer as u8,
                                cell: cell as u32,
                            });
                        }
                    }
                }
            }
            if nodes.is_empty() {
                continue;
            }
            // A track's end on a pad of the net ends on that pad: KiCad's
            // connectivity joins a track to a pad where the copper at its
            // end touches the pad's, and its DRC counts only such tracks as spokes
            // (Castor's U11.5: a segment starting 0.15 mm off the pad's end,
            // within half a gap of it, hangs on the pad's exit stub and does
            // not count). Other ends join the branches they meet (KiCad's
            // tracks and vias connect where they touch).
            let mut ends = [NO_TERMINAL, NO_TERMINAL];
            if let crate::geometry::Shape::Capsule { start, end, radius } = &obstacle.shape {
                for (slot, point) in [*start, *end].into_iter().enumerate() {
                    let on = |pad: usize| {
                        let pad = &self.board.obstacles[pad];
                        pad.layers & obstacle.layers != 0
                            && pad.shape.distance_to_point(point) <= *radius + 1.0e-6
                    };
                    // A padstack's other shapes are their terminal's pad.
                    if let Some(terminal) = self.board.nets[net as usize]
                        .terminals
                        .iter()
                        .position(|terminal| on(terminal.pad))
                        .or_else(|| {
                            self.board
                                .merged_pads
                                .iter()
                                .find(|merged| {
                                    merged.padstack && merged.net == net && on(merged.pad)
                                })
                                .map(|merged| merged.host)
                        })
                    {
                        ends[slot] = terminal as u16;
                    }
                }
            }
            if let crate::geometry::Shape::Capsule { start, end, radius } = &obstacle.shape {
                let index = self.nets[net as usize].branches.len();
                self.nets[net as usize]
                    .adopted
                    .insert(index, (*start, *end, *radius));
            }
            self.nets[net as usize].branches.push(Branch {
                nodes,
                start_terminal: ends[0],
                end_terminal: ends[1],
            });
        }
    }
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Thermal guards around the pads that join a pour by spokes.

use super::*;

impl Router {
    /// Per pad connecting to a pour of its net: the layer, the net and the
    /// nodes of each of its four spokes' corridors, straight out from the
    /// pad along its axes (diagonally for a round pad, as KiCad draws them),
    /// across the gap and the spoke's length.
    /// Each corridor lists the spoke's own strip (the bridge's width) first;
    /// the fourth element says how many cells that is.
    pub(super) fn spoke_corridors(&self) -> Vec<SpokeCorridors> {
        let mut corridors = Vec::new();
        for plane in &self.board.planes {
            if plane.thermal_reach <= 0.0 {
                continue;
            }
            for terminal in &self.board.nets[plane.net as usize].terminals {
                if terminal.layers & (1 << plane.layer) == 0
                    || !crate::geometry::point_in_polygon(terminal.anchor, &plane.polygon)
                {
                    continue;
                }
                let shape = &self.board.obstacles[terminal.pad].shape;
                let bounds = shape.aabb();
                // The guards keep the corridors along the pad's box: in
                // the pad's own frame they cost Hub's
                // route 6 connections (29 to 35 open; turned parts guarded
                // diagonally), for no fewer starved pads.
                let (_, anchor, geometry, _) =
                    self.spoke_geometry_in(terminal.pad, terminal.anchor, 0.0, false);
                let bridge = (plane.thermal_reach - plane.thermal_gap).max(0.0);
                let width = bridge / 2.0
                    + self.board.classes[self.board.nets[plane.net as usize].class].clearance
                    + 0.1;
                let Some((x0, y0, x1, y1)) = self
                    .grid
                    .node_range(bounds.inflated(plane.thermal_reach + width))
                else {
                    continue;
                };
                let mut cores: [Vec<usize>; 4] = Default::default();
                let mut margins: [Vec<usize>; 4] = Default::default();
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let center = self.grid.center(x, y);
                        if shape.distance_to_point(center) >= plane.thermal_reach {
                            continue;
                        }
                        let offset = [center[0] - anchor[0], center[1] - anchor[1]];
                        for (index, (direction, edge, _)) in geometry.iter().enumerate() {
                            let edge = *edge;
                            let along = offset[0] * direction[0] + offset[1] * direction[1];
                            let across =
                                (offset[0] * direction[1] - offset[1] * direction[0]).abs();
                            if along > edge - 1.0e-9
                                && along < edge + plane.thermal_reach
                                && across < width
                            {
                                if across <= bridge / 2.0 + self.grid.pitch / 2.0 {
                                    cores[index].push(self.grid.index(x, y));
                                } else {
                                    margins[index].push(self.grid.index(x, y));
                                }
                            }
                        }
                    }
                }
                let core_counts = [
                    cores[0].len(),
                    cores[1].len(),
                    cores[2].len(),
                    cores[3].len(),
                ];
                let mut cells: [Vec<usize>; 4] = Default::default();
                for index in 0..4 {
                    cells[index] = std::mem::take(&mut cores[index]);
                    cells[index].append(&mut margins[index]);
                }
                corridors.push((plane.layer, plane.net, cells, core_counts));
            }
        }
        corridors
    }

    /// Nodes around pads that connect to a pour of their net on that layer:
    /// their spokes' corridors (other nets may pass the pad's corners, not
    /// its spokes: Sisu's GND resistors starved at one spoke of two).
    pub(super) fn thermal_guards(&self) -> Vec<Vec<u32>> {
        let mut guard = vec![Vec::new(); self.board.layer_count];
        for (layer, net, cells, _) in self.spoke_corridors() {
            if guard[layer].is_empty() {
                guard[layer] = vec![0; self.grid.cells()];
            }
            for corridor in &cells {
                for cell in corridor {
                    guard[layer][*cell] = crate::grid::owner(net);
                }
            }
        }
        // Planes without thermal reach still get their (empty) layer map.
        for plane in &self.board.planes {
            if plane.thermal_reach > 0.0 && guard[plane.layer].is_empty() {
                guard[plane.layer] = vec![0; self.grid.cells()];
            }
        }
        guard
    }

    /// Pads joined to a pour of their net by thermal spokes that have
    /// fewer than two of their spoke corridors free of other nets' copper
    /// (as `free_thermal_spokes` counts them): what KiCad reports as
    /// starved thermals.
    pub(super) fn starved_pads(&self) -> usize {
        let corridors = self.spoke_corridors();
        if corridors.is_empty() {
            return 0;
        }
        let layers = self.board.layer_count;
        let mut occupants: HashMap<(usize, usize), Vec<NetId>> = HashMap::new();
        for (net, state) in self.nets.iter().enumerate() {
            for branch in &state.branches {
                for (index, node) in branch.nodes.iter().enumerate() {
                    let via = branch
                        .nodes
                        .get(index + 1)
                        .is_some_and(|next| next.cell == node.cell && next.layer != node.layer);
                    let on: Vec<usize> = if via {
                        (0..layers).collect()
                    } else {
                        vec![node.layer as usize]
                    };
                    for layer in on {
                        let entry = occupants.entry((layer, node.cell as usize)).or_default();
                        if !entry.contains(&(net as NetId)) {
                            entry.push(net as NetId);
                        }
                    }
                }
            }
        }
        corridors
            .iter()
            .filter(|(layer, net, lists, _)| {
                let blocked = lists
                    .iter()
                    .filter(|list| {
                        list.iter().any(|cell| {
                            occupants
                                .get(&(*layer, *cell))
                                .is_some_and(|nets| nets.iter().any(|other| other != net))
                        })
                    })
                    .count();
                lists.iter().filter(|list| !list.is_empty()).count() - blocked < 2
            })
            .count()
    }

    /// Pads left with fewer than two of their four spoke corridors free of
    /// other nets' copper are starved thermals to KiCad (Sisu: Y1 and R70
    /// at one spoke of two). The nets in their corridors are routed again,
    /// the guard made all but impassable; a new route is kept when it is
    /// complete and crosses fewer corridor nodes. Returns the nets moved.
    pub(super) fn free_thermal_spokes(&mut self) -> usize {
        let corridors = self.spoke_corridors();
        if corridors.is_empty() {
            return 0;
        }
        let layers = self.board.layer_count;
        // Which nets' copper lies on each node (a via on every layer).
        let occupants = |router: &Self| -> HashMap<(usize, usize), Vec<NetId>> {
            let mut map: HashMap<(usize, usize), Vec<NetId>> = HashMap::new();
            for (net, state) in router.nets.iter().enumerate() {
                for branch in &state.branches {
                    for (index, node) in branch.nodes.iter().enumerate() {
                        let via = branch
                            .nodes
                            .get(index + 1)
                            .is_some_and(|next| next.cell == node.cell && next.layer != node.layer);
                        let on: Vec<usize> = if via {
                            (0..layers).collect()
                        } else {
                            vec![node.layer as usize]
                        };
                        for layer in on {
                            let entry = map.entry((layer, node.cell as usize)).or_default();
                            if !entry.contains(&(net as NetId)) {
                                entry.push(net as NetId);
                            }
                        }
                    }
                }
            }
            map
        };
        let starved = |router: &Self| -> (Vec<NetId>, Vec<(usize, usize)>) {
            let map = occupants(router);
            let mut offenders = Vec::new();
            let mut nodes = Vec::new();
            for (layer, net, lists, _) in &corridors {
                let blocked: Vec<&Vec<usize>> = lists
                    .iter()
                    .filter(|list| {
                        list.iter().any(|cell| {
                            map.get(&(*layer, *cell))
                                .is_some_and(|nets| nets.iter().any(|other| other != net))
                        })
                    })
                    .collect();
                if lists.iter().filter(|list| !list.is_empty()).count() - blocked.len() >= 2 {
                    continue;
                }
                for list in blocked {
                    for cell in list {
                        nodes.push((*layer, *cell));
                        for other in map.get(&(*layer, *cell)).into_iter().flatten() {
                            if other != net && !offenders.contains(other) {
                                offenders.push(*other);
                            }
                        }
                    }
                }
            }
            (offenders, nodes)
        };
        let (offenders, nodes) = starved(self);
        if offenders.is_empty() {
            return 0;
        }
        let corridor_nodes: std::collections::HashSet<(usize, usize)> = nodes.into_iter().collect();
        let crossings = |router: &Self, net: NetId| -> usize {
            router.nets[net as usize]
                .branches
                .iter()
                .flat_map(|branch| &branch.nodes)
                .filter(|node| corridor_nodes.contains(&(node.layer as usize, node.cell as usize)))
                .count()
        };
        let guard_cost = self.config.thermal_guard_cost;
        self.config.thermal_guard_cost = 1.0e4;
        self.cleanup = true;
        let mut moved = 0;
        // Budgeted like the clean-up: a hard reroute per offending net on a
        // big lattice adds up (PolyKybd right: 435 s and 369 s in one
        // layout, and the harness killed the board).
        let clock = self.clock();
        let budget = self
            .config
            .cleanup_seconds
            .min(self.negotiation_seconds.max(60.0));
        let mut skipped = 0;
        for net in offenders {
            if self.spent(&clock, budget) {
                skipped += 1;
                continue;
            }
            let state = &self.nets[net as usize];
            if !state.routable || !state.complete || !state.plane.is_empty() {
                continue;
            }
            let before = crossings(self, net);
            let saved = state.branches.clone();
            self.rip_up(net);
            self.route_net_seq(net, 0.0, true, 1.0);
            if self.nets[net as usize].complete && crossings(self, net) < before {
                moved += 1;
            } else {
                let state = &mut self.nets[net as usize];
                state.branches = saved;
                state.connected.fill(true);
                state.complete = true;
            }
            self.stamp(net);
        }
        self.cleanup = false;
        self.config.thermal_guard_cost = guard_cost;
        if self.config.verbose {
            eprintln!(
                "thermal spokes: {moved} nets moved out of starved pads' spoke corridors ({skipped} left by the budget)"
            );
        }
        moved
    }
}

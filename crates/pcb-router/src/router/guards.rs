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
    pub(super) fn thermal_guards(&self) -> Vec<Vec<u16>> {
        let mut guard = vec![Vec::new(); self.board.layer_count];
        for (layer, net, cells, _) in self.spoke_corridors() {
            if guard[layer].is_empty() {
                guard[layer] = vec![0; self.grid.cells()];
            }
            for corridor in &cells {
                for cell in corridor {
                    guard[layer][*cell] = crate::grid::owner16(net);
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
        // With the spoke model a move is kept only when the pour nets'
        // starved spokes go down: a corridor the model still finds the
        // pad two spokes beside is no starved thermal, and the detour at
        // the guard's cost was not free (Squantor's 10x10 alubox: 14 nets
        // moved, 38 vias to 100, for four pads the spoke repairs fix).
        let pour_nets: Vec<NetId> = if self.config.kicad_spokes {
            (0..self.nets.len() as NetId)
                .filter(|net| !self.nets[*net as usize].plane.is_empty())
                .collect()
        } else {
            Vec::new()
        };
        let mut short = self.pour_spokes_short(&pour_nets);
        for net in offenders {
            if self.spent(&clock, budget) {
                skipped += 1;
                continue;
            }
            let state = &self.nets[net as usize];
            if !state.routable || !state.complete || !state.plane.is_empty() {
                continue;
            }
            if !pour_nets.is_empty() && short == 0 {
                break;
            }
            let before = crossings(self, net);
            let saved = state.branches.clone();
            self.rip_up(net);
            self.route_net_seq(net, 0.0, true, 1.0);
            let better = self.nets[net as usize].complete
                && crossings(self, net) < before
                && (pour_nets.is_empty() || {
                    self.stamp(net);
                    let after = self.pour_spokes_short(&pour_nets);
                    let fewer = after < short;
                    if fewer {
                        short = after;
                    }
                    fewer
                });
            if better {
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

    /// Starved spokes of the pour nets as the spoke model counts them
    /// (`spokes_short` summed over the pads of every net with a pour).
    fn pour_spokes_short(&self, nets: &[NetId]) -> usize {
        nets.iter()
            .map(|net| {
                let mut report = Vec::new();
                self.analyze_pours_reporting(*net, &|_| false, Some(&mut report));
                report.iter().map(Self::spokes_short).sum::<usize>()
            })
            .sum()
    }

    /// Pads the spoke model (`kicad_spokes`) finds starved where other
    /// nets' copper cuts a spoke: in the spoke's strip, or within the
    /// fill's clearance of where it would land (74th's ESP32-H2 C1.2, two
    /// spokes on the empty board and one after the signals ran a loop
    /// around it; KiCad's DRC counted the one). The nets with copper there
    /// are routed again with the strips nearly closed to them (the thermal
    /// guard), each kept when it is complete and the pour nets' starved
    /// spokes go down; otherwise it is put back. Unlike
    /// `free_thermal_spokes`, whose corridors run along the pad's box
    /// whatever KiCad draws, the strips are the spokes that fail in the
    /// model. Returns the nets moved.
    pub(super) fn free_lost_spokes(&mut self) -> usize {
        if !self.config.kicad_spokes || !self.config.lost_spoke_repair {
            return 0;
        }
        let pour_nets: Vec<NetId> = (0..self.nets.len() as NetId)
            .filter(|net| {
                !self.nets[*net as usize].plane.is_empty()
                    && self
                        .board
                        .planes
                        .iter()
                        .any(|plane| plane.net == *net && plane.thermal_gap > 0.0)
            })
            .collect();
        if pour_nets.is_empty() {
            return 0;
        }
        // Per failed spoke: its layer, its net, the strip's axis and how
        // near other copper may come to it.
        let mut strips: Vec<(
            usize,
            NetId,
            crate::geometry::Point,
            crate::geometry::Point,
            f64,
        )> = Vec::new();
        for &net in &pour_nets {
            let mut report = Vec::new();
            self.analyze_pours_reporting(net, &|_| false, Some(&mut report));
            let state = &self.nets[net as usize];
            for record in report
                .iter()
                .filter(|record| Self::spokes_short(record) > 0)
            {
                let layer = record.layer;
                let planes = || {
                    self.board
                        .planes
                        .iter()
                        .filter(move |plane| plane.net == net && plane.layer == layer)
                };
                let gap = planes()
                    .map(|plane| plane.thermal_gap)
                    .fold(0.0f64, f64::max);
                let bridge = planes()
                    .map(|plane| (plane.thermal_reach - plane.thermal_gap).max(0.0))
                    .fold(0.0f64, f64::max);
                let brush = self.board.classes[state.plane_class[layer]];
                let (_, anchor, geometry, minor) =
                    self.spoke_geometry(record.pad, record.anchor, gap + brush.trace_width / 2.0);
                let width = bridge.min(minor);
                if width + 1.0e-9 < brush.trace_width {
                    continue;
                }
                for (index, (direction, edge, tip)) in geometry.into_iter().enumerate() {
                    if !matches!(
                        record.outcomes[index],
                        SpokeOutcome::NoFill | SpokeOutcome::Blocked
                    ) {
                        continue;
                    }
                    let at = |along: f64| {
                        [
                            anchor[0] + direction[0] * along,
                            anchor[1] + direction[1] * along,
                        ]
                    };
                    strips.push((
                        layer,
                        net,
                        at(edge),
                        at(tip + brush.clearance),
                        width / 2.0 + brush.clearance + brush.trace_width / 2.0,
                    ));
                }
            }
        }
        if strips.is_empty() {
            return 0;
        }
        // The other nets with copper on a strip (a via on every layer).
        let mut offenders: Vec<NetId> = Vec::new();
        for (other, state) in self.nets.iter().enumerate() {
            let other = other as NetId;
            if !state.routable || !state.complete || !state.plane.is_empty() {
                continue;
            }
            let half = self.board.classes[self.board.nets[other as usize].class].trace_width / 2.0;
            let via = self.board.classes[self.board.nets[other as usize].class].via_diameter / 2.0;
            let cuts = state.branches.iter().any(|branch| {
                branch.nodes.iter().enumerate().any(|(index, node)| {
                    let is_via = branch
                        .nodes
                        .get(index + 1)
                        .is_some_and(|next| next.cell == node.cell && next.layer != node.layer)
                        || (index > 0
                            && branch.nodes[index - 1].cell == node.cell
                            && branch.nodes[index - 1].layer != node.layer);
                    let center = self.grid.center_of(node.cell as usize);
                    strips.iter().any(|(layer, net, a, b, reach)| {
                        *net != other
                            && (is_via || node.layer as usize == *layer)
                            && crate::geometry::point_segment_distance(center, *a, *b)
                                < reach + if is_via { via } else { half }
                    })
                })
            });
            if cuts {
                offenders.push(other);
            }
        }
        if offenders.is_empty() {
            return 0;
        }
        // The strips closed to every net but their own.
        let saved_guard = self.guard.clone();
        for (layer, net, a, b, reach) in &strips {
            if self.guard[*layer].is_empty() {
                self.guard[*layer] = vec![0; self.grid.cells()];
            }
            let bounds = crate::geometry::Aabb {
                minimum: [a[0].min(b[0]), a[1].min(b[1])],
                maximum: [a[0].max(b[0]), a[1].max(b[1])],
            }
            .inflated(*reach);
            let Some((x0, y0, x1, y1)) = self.grid.node_range(bounds) else {
                continue;
            };
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let center = self.grid.center(x, y);
                    if crate::geometry::point_segment_distance(center, *a, *b) < *reach {
                        self.guard[*layer][self.grid.index(x, y)] = crate::grid::owner16(*net);
                    }
                }
            }
        }
        self.build_hot();
        let guard_cost = self.config.thermal_guard_cost;
        self.config.thermal_guard_cost = 1.0e4;
        self.cleanup = true;
        let clock = self.clock();
        let budget = self
            .config
            .cleanup_seconds
            .min(self.negotiation_seconds.max(60.0));
        let mut short = self.pour_spokes_short(&pour_nets);
        let mut moved = 0;
        let mut skipped = 0;
        for net in offenders {
            if short == 0 {
                break;
            }
            if self.spent(&clock, budget) {
                skipped += 1;
                continue;
            }
            let saved = self.nets[net as usize].branches.clone();
            self.rip_up(net);
            self.route_net_seq(net, 0.0, true, 1.0);
            self.stamp(net);
            let after = if self.nets[net as usize].complete {
                self.pour_spokes_short(&pour_nets)
            } else {
                usize::MAX
            };
            if after < short {
                short = after;
                moved += 1;
            } else {
                let state = &mut self.nets[net as usize];
                state.branches = saved;
                state.connected.fill(true);
                state.complete = true;
                self.stamp(net);
            }
        }
        self.cleanup = false;
        self.config.thermal_guard_cost = guard_cost;
        self.guard = saved_guard;
        self.build_hot();
        if self.config.verbose {
            eprintln!(
                "lost spokes: {moved} nets moved off starved pads' spokes ({skipped} left by the budget, {short} spokes short)"
            );
        }
        moved
    }
}

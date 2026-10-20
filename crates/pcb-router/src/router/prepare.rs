// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A net's search state: its terminals' nodes, its pour masks, its classes.

use super::*;

impl Router {
    /// The legal lattice nodes inside each terminal's pad copper.
    pub(super) fn prepare_net(&self, net: NetId) -> NetState {
        let description = &self.board.nets[net as usize];
        let statics = &self.statics[description.class];
        let mut terminal_nodes = Vec::new();
        let mut terminal_reach = Vec::new();
        let mut escapes: HashMap<Node, (Vec<u32>, f64, Vec<crate::geometry::Point>)> =
            HashMap::new();
        for terminal in &description.terminals {
            let pad = &self.board.obstacles[terminal.pad];
            let contact = terminal.contact.as_ref().unwrap_or(&pad.shape);
            let mut nodes = Vec::new();
            if let Some((x0, y0, x1, y1)) = self.grid.node_range(contact.aabb()) {
                for layer in 0..self.board.layer_count {
                    if terminal.layers & (1 << layer) == 0 {
                        continue;
                    }
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let cell = self.grid.index(x, y);
                            if statics.trace_allowed(layer, cell, net)
                                && well_inside(contact, self.grid.center(x, y))
                            {
                                nodes.push(Node {
                                    layer: layer as u8,
                                    cell: cell as u32,
                                });
                            }
                        }
                    }
                }
            }
            if nodes.is_empty() {
                self.escape_nodes(net, terminal, &mut nodes, &mut escapes);
            }
            let reach = nodes
                .iter()
                .map(|node| {
                    crate::geometry::distance(
                        terminal.anchor,
                        self.grid.center_of(node.cell as usize),
                    ) as f32
                })
                .fold(0.0, f32::max);
            terminal_reach.push(reach);
            terminal_nodes.push(nodes);
        }
        // Neck zones around pads narrower than the net's track plus its
        // clearance (a fine-pitch part's), where the net routes at the neck
        // width.
        let mut neck_zones = Vec::new();
        if self.neck_of[description.class].is_some() {
            let rules = self.board.classes[description.class];
            for terminal in &description.terminals {
                let bounds = self.board.obstacles[terminal.pad].shape.aabb();
                let narrow = (bounds.maximum[0] - bounds.minimum[0])
                    .min(bounds.maximum[1] - bounds.minimum[1]);
                if narrow < rules.trace_width + rules.clearance {
                    let reach = crate::geometry::Aabb {
                        minimum: terminal.anchor,
                        maximum: terminal.anchor,
                    }
                    .inflated(self.config.neck_reach);
                    if let Some(range) = self.grid.node_range(reach) {
                        neck_zones.push(range);
                    }
                }
            }
        }
        let (plane, plane_class) = self.pour_masks(net, true);
        let on_plane: Vec<bool> = terminal_nodes
            .iter()
            .enumerate()
            .map(|(index, nodes)| {
                // A pad set to no zone connection never joins a pour.
                self.board
                    .isolated_pads
                    .binary_search(&description.terminals[index].label)
                    .is_err()
                    && nodes.iter().any(|node| {
                        plane
                            .get(node.layer as usize)
                            .is_some_and(|mask| !mask.is_empty() && mask[node.cell as usize])
                    })
            })
            .collect();
        let has_plane = !plane.is_empty();
        if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
            for (index, nodes) in terminal_nodes.iter().enumerate() {
                if nodes.is_empty() && !on_plane[index] {
                    let terminal = &description.terminals[index];
                    let pad = &self.board.obstacles[terminal.pad];
                    let (mut inside, mut allowed) = (0, 0);
                    if let Some((x0, y0, x1, y1)) = self.grid.node_range(pad.shape.aabb()) {
                        for layer in 0..self.board.layer_count {
                            if terminal.layers & (1 << layer) == 0 {
                                continue;
                            }
                            for y in y0..=y1 {
                                for x in x0..=x1 {
                                    let cell = self.grid.index(x, y);
                                    inside +=
                                        well_inside(&pad.shape, self.grid.center(x, y)) as usize;
                                    allowed += statics.trace_allowed(layer, cell, net) as usize;
                                }
                            }
                        }
                    }
                    let near = pad.shape.aabb().inflated(1.0);
                    for other in &self.board.obstacles {
                        if other.net != Some(net)
                            && other.layers & terminal.layers != 0
                            && other.shape.aabb().intersects(near)
                        {
                            eprintln!(
                                "  blocked by {:?} net {:?} {:?}",
                                other.kind,
                                other.net,
                                other.shape.aabb()
                            );
                        }
                    }
                    eprintln!(
                        "dead pad {} of {} at {:?} layers {:#b}: shape {:?}, {inside} nodes well inside, {allowed} allowed",
                        terminal.label,
                        description.name,
                        terminal.anchor,
                        terminal.layers,
                        pad.shape.aabb(),
                    );
                }
            }
        }
        // Pads without any access are reported; the rest is still routed.
        let live = terminal_nodes
            .iter()
            .zip(&on_plane)
            .filter(|(nodes, on_plane)| **on_plane || !nodes.is_empty())
            .count();
        let routable = live >= 2 || (has_plane && live >= 1);
        NetState {
            escapes,
            plane,
            plane_class,
            on_plane,
            connected: vec![false; description.terminals.len()],
            terminal_nodes,
            terminal_reach,
            routable,
            neck_zones,
            ..Default::default()
        }
    }

    /// Per layer, the nodes where a pour of `net` may fill (its polygon
    /// less higher-priority pours, where its brush's static map lets it;
    /// empty for a layer without one; no layers without any pour), and
    /// the brush class per layer. `connected_only`: only the pours pads
    /// connect through (the routing's); else every pour of the net (what
    /// KiCad fills on a board whose pour nets are routed as tracks).
    pub(super) fn pour_masks(
        &self,
        net: NetId,
        connected_only: bool,
    ) -> (Vec<Vec<bool>>, Vec<usize>) {
        let description = &self.board.nets[net as usize];
        let mut plane: Vec<Vec<bool>> = Vec::new();
        let mut plane_class = vec![description.class; self.board.layer_count];
        for pour in self
            .board
            .planes
            .iter()
            .filter(|pour| pour.net == net && (pour.connect || !connected_only))
        {
            if plane.is_empty() {
                plane = vec![Vec::new(); self.board.layer_count];
            }
            plane_class[pour.layer] = pour.class;
            let statics = &self.statics[pour.class];
            if plane[pour.layer].is_empty() {
                plane[pour.layer] = vec![false; self.grid.cells()];
            }
            let Some((x0, y0, x1, y1)) = self.grid.node_range(
                crate::geometry::Shape::Polygon {
                    points: pour.polygon.clone(),
                }
                .aabb(),
            ) else {
                continue;
            };
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let cell = self.grid.index(x, y);
                    let center = self.grid.center(x, y);
                    if statics.fill_allowed(pour.layer, cell, net)
                        && crate::geometry::point_in_polygon(center, &pour.polygon)
                        && !pour
                            .excluded
                            .iter()
                            .any(|other| crate::geometry::point_in_polygon(center, other))
                    {
                        plane[pour.layer][cell] = true;
                    }
                }
            }
        }
        (plane, plane_class)
    }

    /// The rule class `net` routes with at `cell`: its neck class inside
    /// its neck zones, else its own.
    pub(super) fn class_at(&self, net: NetId, state: &NetState, cell: u32) -> usize {
        let class = self.board.nets[net as usize].class;
        match self.neck_of[class] {
            Some(neck) if !state.neck_zones.is_empty() => {
                let (x, y) = self.grid.xy(cell as usize);
                zone_class(class, neck, &state.neck_zones, x, y)
            }
            _ => class,
        }
    }

    pub(super) fn node_class(&self, net: NetId, cell: u32) -> usize {
        self.class_at(net, &self.nets[net as usize], cell)
    }

    /// Track width of `net` at `node`.
    pub(super) fn width_at(&self, net: NetId, cell: u32) -> f64 {
        self.board.classes[self.node_class(net, cell)].trace_width
    }
}

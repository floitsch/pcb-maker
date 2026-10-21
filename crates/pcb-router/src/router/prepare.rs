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
        let mut counts = [0u64; 2];
        // A padstack pad's copper on the layers its stack names
        // (`MergedPad::padstack`): there the terminal is that shape.
        let stacks: Vec<&crate::board::MergedPad> = self
            .board
            .merged_pads
            .iter()
            .filter(|merged| merged.padstack && merged.net == net)
            .collect();
        for (index, terminal) in description.terminals.iter().enumerate() {
            let pad = &self.board.obstacles[terminal.pad];
            let own = terminal.contact.as_ref().unwrap_or(&pad.shape);
            let mut nodes = Vec::new();
            for layer in 0..self.board.layer_count {
                if terminal.layers & (1 << layer) == 0 {
                    continue;
                }
                let contact = match stacks.iter().find(|merged| {
                    merged.host == index
                        && merged.layers & (1 << layer) != 0
                        && pad.layers & (1 << layer) == 0
                }) {
                    Some(merged) => &self.board.obstacles[merged.pad].shape,
                    None => own,
                };
                let Some((x0, y0, x1, y1)) = self.grid.node_range(contact.aabb()) else {
                    continue;
                };
                let tested = ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;
                counts[0] += tested;
                counts[1] += tested * 5 * crate::grid::shape_corners(contact);
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
        let (plane, plane_class, mask_counts, masks_found) = self.prepared_pour_masks(net);
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
            prepare_counts: counts,
            mask_counts,
            masks_found,
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
        let (plane, plane_class, _) = self.pour_masks_counted(net, connected_only);
        (plane, plane_class)
    }

    /// `pour_masks`, with what it took counted (`BuildCounts`): the nodes
    /// it tested and the polygon edges those tests went through.
    pub(super) fn pour_masks_counted(
        &self,
        net: NetId,
        connected_only: bool,
    ) -> (Vec<Vec<bool>>, Vec<usize>, [u64; 2]) {
        compute_pour_masks(&self.board, &self.statics, &self.grid, net, connected_only)
    }

    /// `pour_masks_counted(net, true)` as the router's preparation takes
    /// it, and whether it was found: the masks `hold_static_maps` computed
    /// for routers of the same static maps and pours (`POUR_MASKS`: the
    /// rung probes of the connected pours lower one board), or computed.
    /// Their counts are the same either way (the canonical price: what a
    /// router counts does not depend on what the process holds).
    pub(super) fn prepared_pour_masks(
        &self,
        net: NetId,
    ) -> (Vec<Vec<bool>>, Vec<usize>, [u64; 2], bool) {
        if let Some(key) = self.statics_key {
            let key = pour_masks_key(key, &self.board);
            let found = {
                let mut cache = POUR_MASKS
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                cache.retain(|(_, masks)| masks.strong_count() > 0);
                cache
                    .iter()
                    .find(|(other, _)| *other == key)
                    .and_then(|(_, masks)| masks.upgrade())
            };
            if let Some(masks) = found
                && let Some((_, mask)) = masks.iter().find(|(other, _)| *other == net)
            {
                return (mask.0.clone(), mask.1.clone(), mask.2, true);
            }
        }
        let (plane, plane_class, counts) = self.pour_masks_counted(net, true);
        (plane, plane_class, counts, false)
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

/// `Router::pour_masks_counted`, from the board, the static maps and the
/// lattice alone (`hold_static_maps` computes them before routers exist).
pub(super) fn compute_pour_masks(
    board: &Board,
    statics: &[StaticMaps],
    grid: &Grid,
    net: NetId,
    connected_only: bool,
) -> (Vec<Vec<bool>>, Vec<usize>, [u64; 2]) {
    let mut counts = [0u64; 2];
    let description = &board.nets[net as usize];
    let mut plane: Vec<Vec<bool>> = Vec::new();
    let mut plane_class = vec![description.class; board.layer_count];
    for pour in board
        .planes
        .iter()
        .filter(|pour| pour.net == net && (pour.connect || !connected_only))
    {
        if plane.is_empty() {
            plane = vec![Vec::new(); board.layer_count];
        }
        plane_class[pour.layer] = pour.class;
        let statics = &statics[pour.class];
        if plane[pour.layer].is_empty() {
            plane[pour.layer] = vec![false; grid.cells()];
        }
        let Some((x0, y0, x1, y1)) = grid.node_range(
            crate::geometry::Shape::Polygon {
                points: pour.polygon.clone(),
            }
            .aabb(),
        ) else {
            continue;
        };
        // KiCad's fill keeps its brush, half the minimum width, inside
        // the zone's outline: a node closer to the outline holds no
        // brush centre, and a strip of them along the edge joined
        // pieces KiCad's fill keeps apart (mothenjoyer69's GND_B: a
        // column of nodes 0.1 mm inside the outline, a 0.22 mm brush).
        // Per zone, so that zones of the net that overlap fill across
        // each other's outlines.
        let half = board.classes[pour.class].trace_width / 2.0;
        let mut inside: Vec<(usize, bool)> = Vec::new();
        let corners =
            pour.polygon.len() + pour.excluded.iter().map(|other| other.len()).sum::<usize>();
        counts[0] += ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let cell = grid.index(x, y);
                let center = grid.center(x, y);
                let allowed = statics.fill_allowed(pour.layer, cell, net);
                if allowed {
                    counts[1] += corners as u64;
                }
                if allowed
                    && crate::geometry::point_in_polygon(center, &pour.polygon)
                    && !pour
                        .excluded
                        .iter()
                        .any(|other| crate::geometry::point_in_polygon(center, other))
                {
                    inside.push((cell, true));
                }
            }
        }
        // So with the regions other zones take (`excluded`), kept at
        // their clearance as the fill probe keeps them
        // (`Board::excluded_clearance`).
        if half > 0.0 {
            let mut near_edge: std::collections::HashSet<usize> = std::collections::HashSet::new();
            let outlines = std::iter::once((&pour.polygon, half)).chain(
                pour.excluded
                    .iter()
                    .map(|other| (other, half + board.excluded_clearance(pour, other))),
            );
            for (polygon, reach) in outlines {
                for (a, b) in crate::geometry::polygon_edges(polygon) {
                    let bounds = crate::geometry::Aabb {
                        minimum: [a[0].min(b[0]), a[1].min(b[1])],
                        maximum: [a[0].max(b[0]), a[1].max(b[1])],
                    }
                    .inflated(reach);
                    let Some((ex0, ey0, ex1, ey1)) = grid.node_range(bounds) else {
                        continue;
                    };
                    let nodes = ((ex1 - ex0 + 1) * (ey1 - ey0 + 1)) as u64;
                    counts[0] += nodes;
                    counts[1] += nodes;
                    for y in ey0..=ey1 {
                        for x in ex0..=ex1 {
                            if crate::geometry::point_segment_distance(grid.center(x, y), a, b)
                                < reach - 1.0e-6
                            {
                                near_edge.insert(grid.index(x, y));
                            }
                        }
                    }
                }
            }
            counts[0] += inside.len() as u64;
            for (cell, keep) in &mut inside {
                *keep = !near_edge.contains(cell);
            }
        }
        for (cell, keep) in inside {
            if keep {
                plane[pour.layer][cell] = true;
            }
        }
    }
    (plane, plane_class, counts)
}

/// One net's connected pours' masks: per layer the nodes, the brush class
/// per layer, and what computing them counted (`compute_pour_masks`).
pub(super) type PourMask = (Vec<Vec<bool>>, Vec<usize>, [u64; 2]);

/// The masks of every net of a board's connected pours.
pub(super) type PourMaskSet = Vec<(NetId, Arc<PourMask>)>;

/// The pour masks `hold_static_maps` computed, by the static maps' and the
/// pours' key (`pour_masks_key`), while a hold keeps them (weak entries:
/// until 2026-10-21 the last four keys were kept for the process's life;
/// now nothing outlives the holds, `StaticMapsHold`).
pub(super) type PourMasks = Vec<(u128, std::sync::Weak<PourMaskSet>)>;

pub(super) static POUR_MASKS: std::sync::Mutex<PourMasks> = std::sync::Mutex::new(Vec::new());

/// The bytes of a mask set (`held_maps_bytes`).
pub(super) fn mask_set_bytes(set: &PourMaskSet) -> usize {
    set.iter()
        .map(|(_, mask)| {
            mask.0.iter().map(|layer| layer.capacity()).sum::<usize>()
                + mask.1.capacity() * std::mem::size_of::<usize>()
        })
        .sum()
}

/// What the pour masks are computed from beyond the static maps (their
/// key, `statics`): every pour's net, layer, class, connection, outline
/// and the regions it leaves out (a mask keeps clear of other nets' pours
/// of the same outline, `Board::excluded_clearance`).
pub(super) fn pour_masks_key(statics: u128, board: &Board) -> u128 {
    use std::hash::Hasher as _;
    let mut hashers = [
        std::collections::hash_map::DefaultHasher::new(),
        std::collections::hash_map::DefaultHasher::new(),
    ];
    hashers[1].write_u64(0x9e37_79b9_7f4a_7c15);
    let text = format!(
        "{statics} {:?}",
        board
            .planes
            .iter()
            .map(|plane| (
                plane.net,
                plane.layer,
                plane.class,
                plane.connect,
                &plane.polygon,
                &plane.excluded
            ))
            .collect::<Vec<_>>()
    );
    for hasher in &mut hashers {
        hasher.write(text.as_bytes());
    }
    ((hashers[0].finish() as u128) << 64) | hashers[1].finish() as u128
}

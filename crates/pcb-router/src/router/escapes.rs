// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Escape stubs out of the pads.

use super::*;

impl Router {
    /// Access for a pad too narrow to hold a lattice node: nearby legal
    /// nodes that a straight stub from the pad centre reaches without coming
    /// too close to any fixed copper of another net.
    pub(super) fn escape_nodes(
        &self,
        net: NetId,
        terminal: &crate::board::Terminal,
        nodes: &mut Vec<Node>,
        escapes: &mut HashMap<Node, (Vec<u32>, f64, Vec<crate::geometry::Point>)>,
    ) {
        const REACH: f64 = 2.0;
        const WANTED: usize = 12;
        let class = self.board.classes[self.board.nets[net as usize].class];
        let statics = &self.statics[self.board.nets[net as usize].class];
        let neck_width = self.neck_width_of(self.board.nets[net as usize].class);
        let widths = if neck_width < class.trace_width {
            vec![class.trace_width, neck_width]
        } else {
            vec![class.trace_width]
        };
        let bounds = crate::geometry::Aabb {
            minimum: terminal.anchor,
            maximum: terminal.anchor,
        }
        .inflated(REACH);
        let Some((x0, y0, x1, y1)) = self.grid.node_range(bounds) else {
            return;
        };
        for layer in 0..self.board.layer_count {
            if terminal.layers & (1 << layer) == 0 {
                continue;
            }
            let mut candidates: Vec<(f64, usize)> = Vec::new();
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let cell = self.grid.index(x, y);
                    let distance =
                        crate::geometry::distance(terminal.anchor, self.grid.center(x, y));
                    if distance <= REACH && statics.trace_allowed(layer, cell, net) {
                        candidates.push((distance, cell));
                    }
                }
            }
            candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let mut accepted = 0;
            // Exit points along the pad's long axis, for bent stubs.
            let pad = self.board.obstacles[terminal.pad].shape.aabb();
            let extent = [
                pad.maximum[0] - pad.minimum[0],
                pad.maximum[1] - pad.minimum[1],
            ];
            let axis = if extent[0] >= extent[1] {
                [1.0, 0.0]
            } else {
                [0.0, 1.0]
            };
            let half_length = extent[0].max(extent[1]) / 2.0;
            let exits: Vec<crate::geometry::Point> = [1.0, -1.0]
                .iter()
                .flat_map(|sign| {
                    (0..8).map(move |step| {
                        let d = half_length + 0.05 + step as f64 * 0.1;
                        [
                            terminal.anchor[0] + sign * axis[0] * d,
                            terminal.anchor[1] + sign * axis[1] * d,
                        ]
                    })
                })
                .collect();
            // A pad straddling the board edge (connector shell, card-edge
            // finger) cannot host copper at its centre: the stub then starts
            // at the nearest point along the pad's long axis that keeps the
            // edge clearance.
            let narrowest = *widths.last().unwrap();
            let mut starts = vec![terminal.anchor];
            if !self.stub_keeps_edge(terminal.anchor, terminal.anchor, narrowest) {
                let shape = &self.board.obstacles[terminal.pad].shape;
                let mut interior: Vec<crate::geometry::Point> = Vec::new();
                for step in 1.. {
                    let d = step as f64 * 0.1;
                    if d > half_length {
                        break;
                    }
                    for sign in [1.0, -1.0] {
                        let point = [
                            terminal.anchor[0] + sign * axis[0] * d,
                            terminal.anchor[1] + sign * axis[1] * d,
                        ];
                        if well_inside(shape, point)
                            && self.stub_keeps_edge(point, point, narrowest)
                        {
                            interior.push(point);
                        }
                    }
                }
                starts = interior;
                if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                    eprintln!(
                        "edge escape for {} at {:?}: {} interior starts",
                        terminal.label,
                        terminal.anchor,
                        starts.len()
                    );
                }
            }
            let candidate_count = candidates.len();
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() && !candidates.is_empty() {
                // What blocks the narrowest stub to the nearest candidates.
                for (distance, cell) in candidates.iter().take(3) {
                    let center = self.grid.center_of(*cell);
                    let blockers: Vec<String> = self
                        .board
                        .obstacles
                        .iter()
                        .filter(|obstacle| {
                            obstacle.net != Some(net)
                                && obstacle.layers & (1 << layer) != 0
                                && obstacle.blocks_tracks
                                && {
                                    let required = narrowest / 2.0
                                        + SAFETY
                                        + match obstacle.kind {
                                            crate::board::ObstacleKind::Copper => {
                                                self.board.copper_clearance(&class, obstacle)
                                            }
                                            crate::board::ObstacleKind::Keepout => {
                                                obstacle.clearance
                                            }
                                            crate::board::ObstacleKind::Hole => {
                                                self.board.hole_clearance.max(obstacle.clearance)
                                            }
                                        };
                                    obstacle.shape.distance_to_segment(terminal.anchor, center)
                                        < required
                                }
                        })
                        .map(|obstacle| {
                            format!(
                                "{} ({:.3})",
                                obstacle.label,
                                obstacle.shape.distance_to_segment(terminal.anchor, center)
                            )
                        })
                        .collect();
                    eprintln!(
                        "  escape {} candidate at {:?} ({distance:.3} mm): stub of {narrowest} blocked by {blockers:?}",
                        terminal.label, center
                    );
                }
            }
            for (_, cell) in candidates.into_iter().take(600) {
                let center = self.grid.center_of(cell);
                let mut found: Option<(f64, Vec<crate::geometry::Point>)> = None;
                'widths: for width in widths.iter().copied() {
                    for start in &starts {
                        if self.stub_is_clear(net, layer, *start, center, width)
                            && self.stub_keeps_edge(*start, center, width)
                        {
                            found = Some((width, vec![*start, center]));
                            break 'widths;
                        }
                    }
                    for exit in &exits {
                        if crate::geometry::distance(*exit, center) > 1.5 * self.grid.pitch + 0.15 {
                            continue;
                        }
                        if self.stub_is_clear(net, layer, terminal.anchor, *exit, width)
                            && self.stub_keeps_edge(terminal.anchor, *exit, width)
                            && self.stub_is_clear(net, layer, *exit, center, width)
                            && self.stub_keeps_edge(*exit, center, width)
                        {
                            found = Some((width, vec![terminal.anchor, *exit, center]));
                            break 'widths;
                        }
                    }
                }
                let Some((width, polyline)) = found else {
                    continue;
                };
                let mut cells: Vec<u32> = Vec::new();
                for pair in polyline.windows(2) {
                    let length = crate::geometry::distance(pair[0], pair[1]);
                    let samples = (length / (self.grid.pitch / 2.0)).ceil().max(1.0) as usize;
                    for index in 0..=samples {
                        let t = index as f64 / samples as f64;
                        if let Some(cell) = self.grid.nearest_node([
                            pair[0][0] + t * (pair[1][0] - pair[0][0]),
                            pair[0][1] + t * (pair[1][1] - pair[0][1]),
                        ]) {
                            cells.push(cell as u32);
                        }
                    }
                }
                cells.dedup();
                let node = Node {
                    layer: layer as u8,
                    cell: cell as u32,
                };
                // One escape per node: two pads of a net side by side (a
                // QFN's two VIN pins) reaching the same node would share
                // it, and only the later stub would be kept; a route
                // through it then joined both pads with one pad's copper
                // missing (Sisu's U9 pad 11).
                if escapes.contains_key(&node) {
                    continue;
                }
                escapes.insert(node, (cells, width, polyline));
                nodes.push(node);
                accepted += 1;
                if accepted == WANTED {
                    break;
                }
            }
            if accepted == 0 && std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                eprintln!(
                    "no escape for {} at {:?} layer {layer}: {candidate_count} candidates, {} stub starts",
                    terminal.label,
                    terminal.anchor,
                    starts.len()
                );
            }
        }
    }

    /// Whether the stub behind escape node `node` is free of other nets.
    pub(super) fn escape_is_free(
        &self,
        scratch: &Scratch,
        net_state: &NetState,
        net: NetId,
        node: Node,
    ) -> bool {
        let Some((cells, _, _)) = net_state.escapes.get(&node) else {
            return true;
        };
        let class = self.board.nets[net as usize].class;
        let map = self.map_index(class, node.layer as usize);
        let nx = self.grid.nx as i64;
        cells.iter().all(|cell| {
            (-1..=1).all(|dy| {
                (-1..=1).all(|dx| {
                    let target = *cell as i64 + dy * nx + dx;
                    target < 0
                        || target as usize >= self.grid.cells()
                        || self.occupancy_seen(scratch, class, map, target as usize) == 0
                })
            })
        })
    }

    /// A pad-centre stub is cosmetic (the track already ends inside the pad),
    /// so it is only emitted when it is exactly legal against fixed objects.
    pub(super) fn stub_is_clear(
        &self,
        net: NetId,
        layer: usize,
        start: crate::geometry::Point,
        end: crate::geometry::Point,
        width: f64,
    ) -> bool {
        let class_index = self.board.nets[net as usize].class;
        let half_width = width / 2.0;
        self.board.obstacles.iter().all(|obstacle| {
            if obstacle.net == Some(net)
                || obstacle.layers & (1 << layer) == 0
                || !obstacle.blocks_tracks
            {
                return true;
            }
            // A pad inside a mask opening leaves it by its stub: the only
            // way out, as the designer's tracks took (Sisu's J_LCD1 opening
            // over its eight pads keeps everything else out). A rule area
            // forbids tracks outright (Sisu's module keepout).
            if obstacle.kind == crate::board::ObstacleKind::Keepout
                && obstacle.label.starts_with("solder mask opening")
                && obstacle.shape.contains(start)
            {
                return true;
            }
            // Stubs are exact geometry: an exact fit passes, as in KiCad
            // (a 0.2 track out of a 0.4 pitch QFN's 0.2 pad keeps exactly
            // 0.2 from both neighbours, RP2040's U15).
            let required = half_width - STUB_FIT
                + match obstacle.kind {
                    crate::board::ObstacleKind::Copper => {
                        self.obstacle_clearance(class_index, crate::rules::TRACK, obstacle, layer)
                    }
                    crate::board::ObstacleKind::Keepout => obstacle.clearance,
                    crate::board::ObstacleKind::Hole => {
                        self.board.hole_clearance.max(obstacle.clearance)
                    }
                };
            let stub = crate::geometry::Aabb {
                minimum: [start[0].min(end[0]), start[1].min(end[1])],
                maximum: [start[0].max(end[0]), start[1].max(end[1])],
            };
            !stub.intersects(obstacle.shape.aabb().inflated(required))
                || obstacle.shape.distance_to_segment(start, end) >= required
        })
    }

    /// Whether a stub of `width` from `start` to `end` lies inside the board
    /// and keeps the edge clearance from the outline. Cutouts are obstacles
    /// and are checked by `stub_is_clear`.
    pub(super) fn stub_keeps_edge(
        &self,
        start: crate::geometry::Point,
        end: crate::geometry::Point,
        width: f64,
    ) -> bool {
        let outline = &self.board.outline;
        if outline.is_empty() {
            return true;
        }
        if !crate::geometry::point_in_polygon(start, outline)
            || !crate::geometry::point_in_polygon(end, outline)
        {
            return false;
        }
        let required = width / 2.0 + self.board.edge_clearance + SAFETY;
        let bounds = crate::geometry::Aabb {
            minimum: [start[0].min(end[0]), start[1].min(end[1])],
            maximum: [start[0].max(end[0]), start[1].max(end[1])],
        }
        .inflated(required);
        crate::geometry::polygon_edges(outline).all(|(a, b)| {
            let edge = crate::geometry::Aabb {
                minimum: [a[0].min(b[0]), a[1].min(b[1])],
                maximum: [a[0].max(b[0]), a[1].max(b[1])],
            };
            !bounds.intersects(edge)
                || crate::geometry::segment_segment_distance(start, end, a, b) >= required
        })
    }
}

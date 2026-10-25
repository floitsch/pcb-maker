// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Escape stubs out of the pads.

use super::*;

/// `PCB_ROUTER_EXACT_ESCAPES` when set: overrides
/// `Config::exact_escape_stubs` (`0` or empty off, anything else on).
pub fn exact_escapes_override() -> Option<bool> {
    let value = std::env::var("PCB_ROUTER_EXACT_ESCAPES").ok()?;
    Some(!(value.is_empty() || value == "0"))
}

impl Router {
    /// Access for a pad too narrow to hold a lattice node: nearby legal
    /// nodes that a straight stub from the pad centre reaches without coming
    /// too close to any fixed copper of another net.
    pub(super) fn escape_nodes(
        &self,
        net: NetId,
        terminal: &crate::board::Terminal,
        nodes: &mut Vec<Node>,
        escapes: &mut HashMap<Node, Escape>,
    ) {
        const REACH: f64 = 2.0;
        const WANTED: usize = 12;
        let class = self.board.classes[self.board.nets[net as usize].class];
        let statics = &self.statics[self.board.nets[net as usize].class];
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
            // The class's track on this layer, else its neck.
            let net_class = self.board.nets[net as usize].class;
            let track_width = self.width_on(net_class, layer);
            let neck_width = self.neck_width_on(net_class, layer);
            let widths = if neck_width < track_width {
                vec![track_width, neck_width]
            } else {
                vec![track_width]
            };
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
            let mut found_here: Vec<(Node, Escape)> = Vec::new();
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
            // First the straight stubs along a lattice line (the pad's
            // centre line on a lattice aligned with it): what a bent stub
            // reaches, the lattice reaches from the straight stub's node,
            // and a stub off the lattice's lines stamps with the snapping
            // margin (`Stamps::stub_to_trace`), which in a fine-pitch row
            // (0.4 mm pitch, 0.2 mm tracks and clearance) covers the
            // neighbouring pads' stubs: a conflict no negotiation
            // resolves. Bent and oblique stubs only where no straight one
            // leaves.
            let passes: &[bool] = if self.config.exact_escape_stubs {
                &[true, false]
            } else {
                &[false]
            };
            for &straight in passes {
                for (_, cell) in candidates.iter().copied().take(600) {
                    let center = self.grid.center_of(cell);
                    let mut found: Option<(f64, Vec<crate::geometry::Point>)> = None;
                    'widths: for width in widths.iter().copied() {
                        for start in &starts {
                            if straight && !Self::stub_on_lattice(&[*start, center]) {
                                continue;
                            }
                            if self.stub_is_clear(net, layer, *start, center, width)
                                && self.stub_keeps_edge(*start, center, width)
                            {
                                found = Some((width, vec![*start, center]));
                                break 'widths;
                            }
                        }
                        if straight {
                            continue;
                        }
                        for exit in &exits {
                            if crate::geometry::distance(*exit, center)
                                > 1.5 * self.grid.pitch + 0.15
                            {
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
                    let cells = self
                        .lattice_stub_cells(&polyline)
                        .filter(|_| self.config.exact_escape_stubs)
                        .unwrap_or_else(|| self.sampled_stub_cells(&polyline));
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
                    found_here.push((node, (cells, width, polyline)));
                    accepted += 1;
                    if accepted == WANTED {
                        break;
                    }
                }
                if accepted > 0 {
                    break;
                }
            }
            for (node, escape) in found_here.drain(..) {
                escapes.insert(node, escape);
                nodes.push(node);
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
        // A stub along a lattice line is lattice copper: its nodes are
        // all there is to test, as for a branch.
        if self.config.exact_escape_stubs && Self::stub_on_lattice(&net_state.escapes[&node].2) {
            return cells
                .iter()
                .all(|cell| self.occupancy_seen(scratch, class, map, *cell as usize) == 0);
        }
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

    /// Whether an escape stub runs along a lattice line: one straight
    /// segment from the pad centre to its node (the polyline's end),
    /// parallel to an axis. Such a stub is a piece of a lattice run (the
    /// nodes from the pad centre's to the escape node and the axis steps
    /// between them), so it stamps and is tested as exactly as a branch's
    /// nodes are; a stub off the lattice's lines keeps the snapping margin
    /// (`Stamps::stub_to_trace`). A fine-pitch row on a lattice aligned
    /// with its pad centres (0.4 mm pitch, 0.2 mm pads, track and
    /// clearance: the centre line is legal only to the nanometre, so every
    /// pad leaves by a stub) has neighbouring stubs exactly 0.4 mm apart,
    /// which the margin made a conflict that never went away.
    pub(super) fn stub_on_lattice(polyline: &[crate::geometry::Point]) -> bool {
        const ON_LINE: f64 = 1.0e-7;
        polyline.len() == 2
            && ((polyline[0][0] - polyline[1][0]).abs() < ON_LINE
                || (polyline[0][1] - polyline[1][1]).abs() < ON_LINE)
    }

    /// The nodes of a stub along a lattice line (`stub_on_lattice`): from
    /// the first node at or beyond the stub's start (away from its end, so
    /// that the nodes cover the whole stub) to the end node.
    fn lattice_stub_cells(&self, polyline: &[crate::geometry::Point]) -> Option<Vec<u32>> {
        if !Self::stub_on_lattice(polyline) {
            return None;
        }
        let end = self.grid.nearest_node(polyline[1])?;
        let (x, y) = self.grid.xy(end);
        let axis = if (polyline[0][0] - polyline[1][0]).abs() < 1.0e-7 {
            1
        } else {
            0
        };
        let offset = polyline[0][axis] - polyline[1][axis];
        let steps = (offset.abs() / self.grid.pitch - 1.0e-9).ceil().max(0.0) as i64;
        let sign = if offset < 0.0 { -1 } else { 1 };
        let mut cells = Vec::with_capacity(steps as usize + 1);
        for step in (0..=steps).rev() {
            let (cx, cy) = if axis == 0 {
                (x as i64 + sign * step, y as i64)
            } else {
                (x as i64, y as i64 + sign * step)
            };
            if cx < 0 || cy < 0 || cx >= self.grid.nx as i64 || cy >= self.grid.ny as i64 {
                continue;
            }
            cells.push(self.grid.index(cx as usize, cy as usize) as u32);
        }
        Some(cells)
    }

    /// The nearest nodes of samples half a pitch apart along a stub off the
    /// lattice's lines (stamped with the snapping margin).
    fn sampled_stub_cells(&self, polyline: &[crate::geometry::Point]) -> Vec<u32> {
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
        cells
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

/// What an escape stub off the lattice's lines keeps away, relative to
/// its escape node (`end`): `trace`, the nodes where another net's
/// centreline may not be (closer than `trace_radius` to the stub, or the
/// nearer node of an axis step that passes that close: a step's nearest
/// point to a stub's off-lattice vertex may lie between its nodes); `via`, the nodes
/// where its via may not be (closer than `via_radius`); `diagonal`, per
/// orientation (as `Stamps::diagonal_block`: 0 the step (+1, +1), 1 the
/// step (+1, -1), at its start), the diagonal steps passing closer than
/// `trace_radius`. Exact: a distance of exactly the radius is legal, as
/// the lattice's discs have it. Until 2026-10-25 such stubs stamped the
/// discs grown by the snapping error (three quarters of a pitch and a
/// sagitta), which at 0.5 mm pitch with 0.2 mm tracks and clearance
/// (an USON's 0.3 mm pads on a 0.1 mm lattice) covered the neighbouring
/// pads' stubs: a conflict no negotiation resolves.
pub(super) struct StubFootprint {
    pub(super) trace: Vec<(i32, i32)>,
    pub(super) via: Vec<(i32, i32)>,
    pub(super) diagonal: [Vec<(i32, i32)>; 2],
}

pub(super) fn stub_footprint(
    grid: &crate::grid::Grid,
    polyline: &[crate::geometry::Point],
    end: u32,
    trace_radius: f64,
    via_radius: f64,
) -> StubFootprint {
    const EXACT: f64 = 1.0e-9;
    let (ex, ey) = grid.xy(end as usize);
    let (ex, ey) = (ex as i64, ey as i64);
    let reach = trace_radius.max(via_radius);
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    for point in polyline {
        for axis in 0..2 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
        }
    }
    let index = |value: f64, axis: usize| (value - grid.origin[axis]) / grid.pitch;
    let margin = (reach / grid.pitch).ceil() as i64 + 1;
    let x0 = index(minimum[0], 0).floor() as i64 - margin;
    let x1 = index(maximum[0], 0).ceil() as i64 + margin;
    let y0 = index(minimum[1], 1).floor() as i64 - margin;
    let y1 = index(maximum[1], 1).ceil() as i64 + margin;
    let at = |x: i64, y: i64| {
        [
            grid.origin[0] + x as f64 * grid.pitch,
            grid.origin[1] + y as f64 * grid.pitch,
        ]
    };
    let to_point = |point: crate::geometry::Point| {
        polyline
            .windows(2)
            .map(|pair| crate::geometry::point_segment_distance(point, pair[0], pair[1]))
            .fold(f64::INFINITY, f64::min)
    };
    let to_step = |a: crate::geometry::Point, b: crate::geometry::Point| {
        polyline
            .windows(2)
            .map(|pair| crate::geometry::segment_segment_distance(a, b, pair[0], pair[1]))
            .fold(f64::INFINITY, f64::min)
    };
    let mut trace: Vec<(i32, i32)> = Vec::new();
    let mut via = Vec::new();
    let mut diagonal = [Vec::new(), Vec::new()];
    let relative = |x: i64, y: i64| ((x - ex) as i32, (y - ey) as i32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let node = at(x, y);
            let distance = to_point(node);
            if distance < trace_radius - EXACT {
                trace.push(relative(x, y));
            }
            if distance < via_radius - EXACT {
                via.push(relative(x, y));
            }
            // An axis step that passes too close although neither node
            // does: its nearer node (a search reads the node it steps
            // to, so either end keeps the step out; the farther one may
            // hold another net's centreline legally).
            for (dx, dy) in [(1, 0), (0, 1)] {
                let other = at(x + dx, y + dy);
                if to_step(node, other) < trace_radius - EXACT {
                    trace.push(if distance <= to_point(other) {
                        relative(x, y)
                    } else {
                        relative(x + dx, y + dy)
                    });
                }
            }
            for (orientation, dy) in [(0usize, 1i64), (1, -1)] {
                if to_step(node, at(x + 1, y + dy)) < trace_radius - EXACT {
                    diagonal[orientation].push(relative(x, y));
                }
            }
        }
    }
    trace.sort_unstable();
    trace.dedup();
    StubFootprint {
        trace,
        via,
        diagonal,
    }
}

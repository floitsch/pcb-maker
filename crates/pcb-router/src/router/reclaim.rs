// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The fill a pour reclaims inside another net's zone of a higher priority.

use super::*;

impl Router {
    /// Per layer (empty where nothing is reclaimed), the lattice nodes
    /// inside a region the pours of `net` leave to another net's zone of a
    /// higher priority (`Plane::excluded`) where that zone's fill does not
    /// come within the zones' clearance: brush centres of this net's fill.
    ///
    /// KiCad 10's filler knocks a higher zone of another net out of this
    /// one by its fill grown by the clearance, not by its outline
    /// (zone_filler.cpp, `buildDifferentNetZoneClearances` with
    /// `ZONE::TransformShapeToPolygon`, which grows the filled polygons):
    /// around this net's own copper inside that zone, which the higher
    /// zone keeps its clearance from, this net's fill reaches the copper
    /// (anthracite's J107.1 on In2.Cu: a GNDPWR via inside the +5V zone
    /// holds a disc of GNDPWR fill that the pad's spoke lands in), and
    /// gaps the higher zone cannot fill are this net's (backplane's J4.A1
    /// on F.Cu inside VIN's zone: one spoke into such a gap).
    ///
    /// Only near this net's copper inside the region (its pads, vias and
    /// tracks there, `RECLAIM_REACH` around them): what the DRC's spokes
    /// and pieces see. The higher zone's fill is its brush centres (its
    /// static fill map, its outline, the routed copper of other nets in
    /// its brush's occupancy); islands it removes are not taken out.
    pub(super) fn reclaimed_fill(&self, net: NetId) -> Vec<Vec<bool>> {
        let layers = self.board.layer_count;
        let mut result: Vec<Vec<bool>> = vec![Vec::new(); layers];
        if std::env::var_os("PCB_ROUTER_NO_RECLAIM").is_some() {
            return result;
        }
        let state = &self.nets[net as usize];
        let description = &self.board.nets[net as usize];
        let cells = self.grid.cells();
        let pitch = self.grid.pitch;
        for layer in 0..layers {
            if state.plane.get(layer).is_none_or(|mask| mask.is_empty()) {
                continue;
            }
            let class = state.plane_class[layer];
            let half = self.board.classes[class].trace_width / 2.0;
            for plane in self
                .board
                .planes
                .iter()
                .filter(|plane| plane.net == net && plane.layer == layer)
            {
                for excluded in &plane.excluded {
                    let Some(other) = self.board.planes.iter().find(|other| {
                        other.net != net
                            && other.layer == layer
                            && other.polygon.as_slice() == excluded.as_slice()
                    }) else {
                        // A keepout's region: nothing fills it.
                        continue;
                    };
                    let inside = |point: crate::geometry::Point| {
                        crate::geometry::point_in_polygon(point, excluded)
                            && crate::geometry::point_in_polygon(point, &plane.polygon)
                    };
                    // The windows: this net's copper inside the region.
                    let mut boxes: Vec<crate::geometry::Aabb> = Vec::new();
                    for terminal in &description.terminals {
                        if terminal.layers & (1 << layer) != 0 && inside(terminal.anchor) {
                            boxes.push(
                                self.board.obstacles[terminal.pad]
                                    .shape
                                    .aabb()
                                    .inflated(RECLAIM_REACH),
                            );
                        }
                    }
                    for branch in &state.branches {
                        for (index, node) in branch.nodes.iter().enumerate() {
                            let via = branch.nodes.get(index + 1).is_some_and(|next| {
                                next.cell == node.cell && next.layer != node.layer
                            }) || (index > 0
                                && branch.nodes[index - 1].cell == node.cell
                                && branch.nodes[index - 1].layer != node.layer);
                            if node.layer as usize != layer && !via {
                                continue;
                            }
                            let center = self.grid.center_of(node.cell as usize);
                            if inside(center) {
                                let reach = if via { RECLAIM_REACH } else { 2.0 * pitch };
                                boxes.push(crate::geometry::Aabb {
                                    minimum: [center[0] - reach, center[1] - reach],
                                    maximum: [center[0] + reach, center[1] + reach],
                                });
                            }
                        }
                    }
                    if boxes.is_empty() {
                        continue;
                    }
                    let mut window = vec![false; cells];
                    let mut candidates: Vec<usize> = Vec::new();
                    for bounds in &boxes {
                        let Some((x0, y0, x1, y1)) = self.grid.node_range(*bounds) else {
                            continue;
                        };
                        for y in y0..=y1 {
                            for x in x0..=x1 {
                                let cell = self.grid.index(x, y);
                                if !window[cell] {
                                    window[cell] = true;
                                    candidates.push(cell);
                                }
                            }
                        }
                    }
                    candidates.sort_unstable();
                    // The higher zone's brush centres, as asked.
                    let other_class = other.class;
                    let other_map = self.map_index(other_class, layer);
                    let other_own = OwnStamps::new(
                        &self.nets[other.net as usize].stamped,
                        &[other_map],
                        cells,
                    );
                    let mut other_fill: FastMap<usize, bool> = FastMap::default();
                    let mut other_at = |cell: usize| -> bool {
                        *other_fill.entry(cell).or_insert_with(|| {
                            let center = self.grid.center_of(cell);
                            crate::geometry::point_in_polygon(center, &other.polygon)
                                && !other
                                    .excluded
                                    .iter()
                                    .any(|region| crate::geometry::point_in_polygon(center, region))
                                && self.statics[other_class].fill_allowed(layer, cell, other.net)
                                && self.occupancy.get(other_map, cell) as usize
                                    == other_own.contains(other_map, cell) as usize
                        })
                    };
                    // This net's brush centre keeps the clearance from that
                    // fill: the gap plus both brushes' halves from its
                    // centres.
                    let gap = self.board.excluded_clearance(plane, excluded);
                    let radius = gap + half + self.board.classes[other_class].trace_width / 2.0;
                    let steps = (radius / pitch).ceil() as i64;
                    let mut offsets: Vec<(i64, i64)> = Vec::new();
                    for dy in -steps..=steps {
                        for dx in -steps..=steps {
                            if ((dx * dx + dy * dy) as f64).sqrt() * pitch < radius - 1.0e-9 {
                                offsets.push((dx, dy));
                            }
                        }
                    }
                    let (nx, ny) = (self.grid.nx as i64, self.grid.ny as i64);
                    for cell in candidates {
                        let center = self.grid.center_of(cell);
                        if !inside(center) || !self.statics[class].fill_allowed(layer, cell, net) {
                            continue;
                        }
                        let (x, y) = self.grid.xy(cell);
                        let blocked = offsets.iter().any(|(dx, dy)| {
                            let (ox, oy) = (x as i64 + dx, y as i64 + dy);
                            ox >= 0
                                && oy >= 0
                                && ox < nx
                                && oy < ny
                                && other_at(self.grid.index(ox as usize, oy as usize))
                        });
                        if !blocked {
                            if result[layer].is_empty() {
                                result[layer] = vec![false; cells];
                            }
                            result[layer][cell] = true;
                        }
                    }
                }
            }
        }
        result
    }
}

/// How far around this net's pads and vias inside another net's zone of a
/// higher priority its reclaimed fill is looked for (mm): past a pad's
/// thermal gap and a spoke's end, and over a via beside it.
const RECLAIM_REACH: f64 = 1.2;

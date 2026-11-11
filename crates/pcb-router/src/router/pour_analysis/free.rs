// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The pour analysis's fill: the nodes a net's pour keeps free and the
//! pieces they form.

use super::*;

/// What the analysis knows of the net's fill: per layer the free nodes,
/// the fill reclaimed in other nets' zones and the thermal rings' pads.
pub(super) struct Fill {
    pub(super) free: Vec<Vec<bool>>,
    pub(super) reclaimed: Vec<Vec<bool>>,
    pub(super) rings: Vec<Vec<(usize, f64)>>,
}

impl Router {
    /// The net's own stamps on its pour maps (the class map and both
    /// diagonal maps of each poured layer).
    pub(super) fn own_pour_stamps(&self, state: &NetState) -> OwnStamps {
        let layers = self.board.layer_count;
        let maps: Vec<usize> = (0..layers)
            .filter(|layer| !state.plane[*layer].is_empty())
            .flat_map(|layer| {
                let class = state.plane_class[layer];
                [
                    self.map_index(class, layer),
                    self.diagonal_map(class, layer, 0),
                    self.diagonal_map(class, layer, 1),
                ]
            })
            .collect();
        OwnStamps::new(&state.stamped, &maps, self.grid.cells())
    }

    /// The nodes of `net`'s pour no other net's copper claims (`own`: the
    /// net's own stamps), with the fill it reclaims, less its thermal
    /// rings, knockouts and via reliefs, and with its own tracks but those
    /// `skip` names.
    pub(super) fn free_fill(
        &self,
        net: NetId,
        own: &OwnStamps,
        skip: &dyn Fn(usize) -> bool,
    ) -> Fill {
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        let mut free: Vec<Vec<bool>> = (0..layers)
            .map(|layer| {
                let mask = &state.plane[layer];
                if mask.is_empty() {
                    return Vec::new();
                }
                let map = self.map_index(state.plane_class[layer], layer);
                // `own.contains` per node, its map's bits looked up once.
                let own_bits = own.bits_of(map);
                let mut free = vec![false; mask.len()];
                for cell in crate::pour::occupied_cells(mask) {
                    free[cell] = self.occupancy.get(map, cell) as usize
                        == ((own_bits[cell / 64] >> (cell % 64)) & 1) as usize;
                }
                free
            })
            .collect();
        // Fill this net reclaims inside other nets' zones of a higher
        // priority, where their fill leaves room (`reclaimed_fill`).
        let reclaimed = self.reclaimed_fill(net);
        for (layer, mask) in reclaimed.iter().enumerate() {
            if mask.is_empty() || free[layer].is_empty() {
                continue;
            }
            let map = self.map_index(state.plane_class[layer], layer);
            for cell in crate::pour::occupied_cells(mask) {
                if self.occupancy.get(map, cell) as usize == own.contains(map, cell) as usize {
                    free[layer][cell] = true;
                }
            }
        }
        let rings = self.thermal_ring_pads(net);
        self.cut_thermal_rings(&rings, &mut free);
        self.cut_thermal_rings(&self.isolated_knockouts(net), &mut free);
        self.cut_via_reliefs(net, &mut free);
        // The net's own tracks are copper of the net, and the fill joins
        // them anywhere, inside a pad's thermal ring too (a plane stub ends
        // in the ring: Castor's U7.3 read as stranded with "1 branch, 0
        // pieces" and was rerouted and stitched for nothing).
        for (index, branch) in state.branches.iter().enumerate() {
            if skip(index) {
                continue;
            }
            for node in &branch.nodes {
                let layer = node.layer as usize;
                if free[layer].is_empty() {
                    continue;
                }
                let cell = node.cell as usize;
                let map = self.map_index(state.plane_class[layer], layer);
                if (state.plane[layer][cell]
                    || reclaimed[layer].get(cell).copied().unwrap_or(false))
                    && self.occupancy.get(map, cell) as usize == own.contains(map, cell) as usize
                {
                    free[layer][cell] = true;
                }
            }
        }
        Fill {
            free,
            reclaimed,
            rings,
        }
    }

    /// The pieces of the free fill, labelled with the diagonal steps the
    /// brush can take.
    pub(super) fn fill_pieces(
        &self,
        net: NetId,
        own: &OwnStamps,
        fill: &Fill,
    ) -> crate::pour::PourMap {
        let state = &self.nets[net as usize];
        let rings = &fill.rings;
        // Two free nodes diagonal to each other whose common neighbours are
        // both taken join when the brush can go straight from one to the
        // other: no other net's copper, fixed or routed, and no thermal ring
        // comes closer to the step than to the nodes. A neck of KiCad's
        // fill running diagonally to the lattice holds no two nodes side by
        // side; labelled 4-connected, k30-SBC's +5V plane on In1 fell into
        // 674 pieces where KiCad's fill has 178 outlines (with the diagonal
        // steps 183; GND on F.Cu 420, 298 and 306).
        let diagonal =
            |layer: usize, start: usize, orientation: usize| {
                let class = state.plane_class[layer];
                let direction = if orientation == 0 { 1 } else { 7 };
                if !self.statics[class].edge_allowed(layer, start, direction, net) {
                    return false;
                }
                let map = self.diagonal_map(class, layer, orientation);
                if self.occupancy.get(map, start) as usize != own.contains(map, start) as usize {
                    return false;
                }
                let Some(end) = self.grid.neighbor(start, direction) else {
                    return false;
                };
                let (a, b) = (self.grid.center_of(start), self.grid.center_of(end));
                let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                !rings[layer].iter().any(|(pad, reach)| {
                    let shape = &self.board.obstacles[*pad].shape;
                    shape.aabb().inflated(*reach + self.grid.pitch).intersects(
                        crate::geometry::Aabb {
                            minimum: middle,
                            maximum: middle,
                        },
                    ) && shape.distance_to_segment(a, b) < *reach
                })
            };
        crate::pour::PourMap::build(&self.grid, &fill.free, &diagonal)
    }
}

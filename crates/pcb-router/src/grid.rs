// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The routing lattice and the static (never changing during a run) maps
//! derived from pads, keepouts, holes and the outline.

use crate::board::{Board, ClassId, NetId, ObstacleKind};
use crate::geometry::{Aabb, Point, Shape, polygon_edges};

/// Positions are written to the board in nanometres; keep that much slack
/// so rounding can never turn a tangent into a violation.
pub const SAFETY: f64 = 2.0e-6;

pub const FREE: u32 = 0;
pub const BLOCKED: u32 = u32::MAX;

pub fn owner(net: NetId) -> u32 {
    net + 1
}

/// Unit steps for the eight planar directions, counter-clockwise from +x.
pub const DIRECTIONS: [(i32, i32); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

#[derive(Clone, Debug)]
pub struct Grid {
    pub origin: Point,
    pub pitch: f64,
    pub nx: usize,
    pub ny: usize,
}

impl Grid {
    pub fn cells(&self) -> usize {
        self.nx * self.ny
    }

    pub fn index(&self, x: usize, y: usize) -> usize {
        y * self.nx + x
    }

    pub fn xy(&self, index: usize) -> (usize, usize) {
        (index % self.nx, index / self.nx)
    }

    pub fn center(&self, x: usize, y: usize) -> Point {
        [
            self.origin[0] + x as f64 * self.pitch,
            self.origin[1] + y as f64 * self.pitch,
        ]
    }

    pub fn center_of(&self, index: usize) -> Point {
        let (x, y) = self.xy(index);
        self.center(x, y)
    }

    pub fn neighbor(&self, index: usize, direction: usize) -> Option<usize> {
        let (x, y) = self.xy(index);
        let (dx, dy) = DIRECTIONS[direction];
        let nx = x as i64 + dx as i64;
        let ny = y as i64 + dy as i64;
        (nx >= 0 && ny >= 0 && (nx as usize) < self.nx && (ny as usize) < self.ny)
            .then(|| self.index(nx as usize, ny as usize))
    }

    /// Inclusive node range whose centres can lie inside `bounds`.
    pub fn node_range(&self, bounds: Aabb) -> Option<(usize, usize, usize, usize)> {
        let low = |value: f64, origin: f64| ((value - origin) / self.pitch).ceil();
        let high = |value: f64, origin: f64| ((value - origin) / self.pitch).floor();
        let x0 = low(bounds.minimum[0], self.origin[0]).max(0.0);
        let y0 = low(bounds.minimum[1], self.origin[1]).max(0.0);
        let x1 = high(bounds.maximum[0], self.origin[0]).min(self.nx as f64 - 1.0);
        let y1 = high(bounds.maximum[1], self.origin[1]).min(self.ny as f64 - 1.0);
        (x0 <= x1 && y0 <= y1).then(|| (x0 as usize, y0 as usize, x1 as usize, y1 as usize))
    }

    pub fn nearest_node(&self, point: Point) -> Option<usize> {
        let x = ((point[0] - self.origin[0]) / self.pitch).round();
        let y = ((point[1] - self.origin[1]) / self.pitch).round();
        (x >= 0.0 && y >= 0.0 && (x as usize) < self.nx && (y as usize) < self.ny)
            .then(|| self.index(x as usize, y as usize))
    }

    /// Picks the candidate pitch and lattice phase that puts the most pad
    /// anchors exactly on nodes. Through-hole boards live on an imperial
    /// lattice, and a one-track channel between two pads is only usable when
    /// a node row falls in its centre.
    pub fn choose(board: &Board, candidates: &[f64]) -> Self {
        let anchors: Vec<Point> = board
            .nets
            .iter()
            .flat_map(|net| net.terminals.iter().map(|terminal| terminal.anchor))
            .collect();
        let mut bounds = Aabb {
            minimum: [f64::INFINITY; 2],
            maximum: [f64::NEG_INFINITY; 2],
        };
        for point in &board.outline {
            for axis in 0..2 {
                bounds.minimum[axis] = bounds.minimum[axis].min(point[axis]);
                bounds.maximum[axis] = bounds.maximum[axis].max(point[axis]);
            }
        }
        let mut best: Option<(f64, f64, [f64; 2])> = None;
        for &pitch in candidates {
            let mut phase = [0.0; 2];
            let mut score = 0.0;
            for axis in 0..2 {
                let mut histogram = std::collections::BTreeMap::<i64, usize>::new();
                for anchor in &anchors {
                    let residue = anchor[axis].rem_euclid(pitch);
                    *histogram.entry((residue * 1.0e4).round() as i64).or_default() += 1;
                }
                // Residues wrap around: 0 and `pitch` are the same phase.
                let wrap = (pitch * 1.0e4).round() as i64;
                let mut merged = std::collections::BTreeMap::<i64, usize>::new();
                for (residue, count) in histogram {
                    *merged.entry(residue % wrap).or_default() += count;
                }
                let (residue, count) = merged
                    .into_iter()
                    .max_by_key(|(residue, count)| (*count, -*residue))
                    .unwrap_or((0, 0));
                phase[axis] = residue as f64 / 1.0e4;
                score += count as f64 / anchors.len().max(1) as f64;
            }
            if best.is_none_or(|(best_score, _, _)| score > best_score + 1e-9) {
                best = Some((score, pitch, phase));
            }
        }
        let (_, pitch, phase) = best.unwrap_or((0.0, candidates[0], [0.0; 2]));
        let mut origin = [0.0; 2];
        for axis in 0..2 {
            let steps = ((bounds.minimum[axis] - phase[axis]) / pitch).floor();
            origin[axis] = phase[axis] + steps * pitch;
        }
        let nx = ((bounds.maximum[0] - origin[0]) / pitch).ceil() as usize + 1;
        let ny = ((bounds.maximum[1] - origin[1]) / pitch).ceil() as usize + 1;
        Self {
            origin,
            pitch,
            nx,
            ny,
        }
    }
}

/// Where a trace centreline or a via of one rule class may be, considering
/// only immovable objects.
#[derive(Clone, Debug)]
pub struct StaticMaps {
    /// Per layer and node: `FREE`, `BLOCKED`, or the only net allowed there.
    pub trace: Vec<Vec<u32>>,
    /// Per layer and node: directions whose unit step passes too close to an
    /// obstacle although both end nodes are legal.
    pub edge_block: Vec<Vec<u8>>,
    /// The only net exempt from `edge_block` (its own pad), or `BLOCKED`.
    pub edge_owner: Vec<Vec<u32>>,
    /// Per node: a via of this class is forbidden.
    pub via_blocked: Vec<bool>,
}

fn claim(cell: &mut u32, net: Option<NetId>) {
    let value = net.map_or(BLOCKED, owner);
    if *cell == FREE {
        *cell = value;
    } else if *cell != value {
        *cell = BLOCKED;
    }
}

impl StaticMaps {
    pub fn build(board: &Board, grid: &Grid, class: ClassId) -> Self {
        let rules = board.classes[class];
        let cells = grid.cells();
        let half_width = rules.trace_width / 2.0;
        let via_radius = rules.via_diameter / 2.0;
        let step = grid.pitch * std::f64::consts::SQRT_2;

        let mut inside = vec![false; cells];
        for y in 0..grid.ny {
            let line = grid.origin[1] + y as f64 * grid.pitch;
            let mut crossings: Vec<f64> = polygon_edges(&board.outline)
                .filter(|(a, b)| (a[1] > line) != (b[1] > line))
                .map(|(a, b)| a[0] + (line - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
                .collect();
            crossings.sort_by(f64::total_cmp);
            for span in crossings.chunks_exact(2) {
                let x0 = ((span[0] - grid.origin[0]) / grid.pitch).ceil().max(0.0) as usize;
                let x1 = ((span[1] - grid.origin[0]) / grid.pitch).floor();
                if x1 < 0.0 {
                    continue;
                }
                for x in x0..=(x1 as usize).min(grid.nx - 1) {
                    inside[grid.index(x, y)] = true;
                }
            }
        }

        let mut base_trace: Vec<u32> = inside
            .iter()
            .map(|inside| if *inside { FREE } else { BLOCKED })
            .collect();
        let mut via_blocked: Vec<bool> = inside.iter().map(|inside| !inside).collect();
        let trace_edge = half_width + board.edge_clearance + SAFETY;
        let via_edge = via_radius + board.edge_clearance + SAFETY;
        // A unit step between two legal nodes can cut an outline corner by
        // at most the sagitta of that chord.
        let edge_margin = step * step / (8.0 * trace_edge.max(grid.pitch));
        for (a, b) in polygon_edges(&board.outline) {
            let reach = trace_edge.max(via_edge) + edge_margin;
            let bounds = Aabb {
                minimum: [a[0].min(b[0]), a[1].min(b[1])],
                maximum: [a[0].max(b[0]), a[1].max(b[1])],
            }
            .inflated(reach);
            let Some((x0, y0, x1, y1)) = grid.node_range(bounds) else {
                continue;
            };
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let distance =
                        crate::geometry::point_segment_distance(grid.center(x, y), a, b);
                    let index = grid.index(x, y);
                    if distance < trace_edge + edge_margin {
                        base_trace[index] = BLOCKED;
                    }
                    if distance < via_edge {
                        via_blocked[index] = true;
                    }
                }
            }
        }

        let mut trace = vec![base_trace; board.layer_count];
        let mut edge_block = vec![vec![0u8; cells]; board.layer_count];
        let mut edge_owner = vec![vec![FREE; cells]; board.layer_count];

        for obstacle in &board.obstacles {
            let (trace_reach, via_reach) = match obstacle.kind {
                ObstacleKind::Copper => {
                    let clearance = board.copper_clearance(&rules, obstacle);
                    (half_width + clearance, via_radius + clearance)
                }
                ObstacleKind::Keepout => (half_width, via_radius),
                ObstacleKind::Hole => {
                    let clearance = board.hole_clearance.max(obstacle.clearance);
                    (
                        half_width + clearance,
                        (via_radius + clearance).max(rules.via_drill / 2.0 + board.hole_to_hole),
                    )
                }
            };
            let trace_reach = trace_reach + SAFETY;
            let via_reach = via_reach + SAFETY;
            let margin = step * step / (8.0 * trace_reach);
            let bounds = obstacle.shape.aabb();

            if obstacle.blocks_vias
                && let Some((x0, y0, x1, y1)) = grid.node_range(bounds.inflated(via_reach))
            {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        if obstacle.shape.distance_to_point(grid.center(x, y)) < via_reach {
                            via_blocked[grid.index(x, y)] = true;
                        }
                    }
                }
            }
            if !obstacle.blocks_tracks {
                continue;
            }
            let Some((x0, y0, x1, y1)) = grid.node_range(bounds.inflated(trace_reach + margin))
            else {
                continue;
            };
            for layer in 0..board.layer_count {
                if obstacle.layers & (1 << layer) == 0 {
                    continue;
                }
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let index = grid.index(x, y);
                        let center = grid.center(x, y);
                        let distance = obstacle.shape.distance_to_point(center);
                        if distance < trace_reach {
                            claim(&mut trace[layer][index], obstacle.net);
                        } else if distance < trace_reach + margin {
                            mark_tight_edges(
                                grid,
                                &obstacle.shape,
                                obstacle.net,
                                trace_reach,
                                index,
                                &mut edge_block[layer],
                                &mut edge_owner[layer],
                            );
                        }
                    }
                }
            }
        }
        Self {
            trace,
            edge_block,
            edge_owner,
            via_blocked,
        }
    }

    /// Whether `net` may have a trace centreline on this node.
    pub fn trace_allowed(&self, layer: usize, index: usize, net: NetId) -> bool {
        let value = self.trace[layer][index];
        value == FREE || value == owner(net)
    }

    /// Whether `net` may take the unit step leaving `index` in `direction`.
    pub fn edge_allowed(&self, layer: usize, index: usize, direction: usize, net: NetId) -> bool {
        self.edge_block[layer][index] & (1 << direction) == 0
            || self.edge_owner[layer][index] == owner(net)
    }
}

fn mark_tight_edges(
    grid: &Grid,
    shape: &Shape,
    net: Option<NetId>,
    reach: f64,
    index: usize,
    edge_block: &mut [u8],
    edge_owner: &mut [u32],
) {
    let center = grid.center_of(index);
    for direction in 0..8 {
        let Some(neighbor) = grid.neighbor(index, direction) else {
            continue;
        };
        if shape.distance_to_segment(center, grid.center_of(neighbor)) < reach {
            edge_block[index] |= 1 << direction;
            claim(&mut edge_owner[index], net);
            edge_block[neighbor] |= 1 << ((direction + 4) % 8);
            claim(&mut edge_owner[neighbor], net);
        }
    }
}

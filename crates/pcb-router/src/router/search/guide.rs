// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The search's guide: its step costs per layer and its heuristic.

/// The coarse blocks' side in nodes (`coarse_distances`).
pub(super) const BLOCK: usize = 8;

/// The A* heuristic of one search (a lower bound on the cost left, times
/// the weight) and the step costs per layer and direction it bounds.
pub(super) struct Guide<'a> {
    plane_target: bool,
    /// The target points with their reach, when the caller has a short
    /// list of them.
    points: Option<&'a [(i32, i32, f32)]>,
    /// Without points: blocks to the nearest target's block
    /// (`coarse_distances`), `blocks_x` a row.
    coarse: Vec<f32>,
    blocks_x: usize,
    /// Per layer, a step's cost in each of the eight directions.
    pub(super) direction_cost: Vec<[f32; 8]>,
    /// The least cost over the layers of a horizontal, diagonal and
    /// vertical step.
    any_horizontal: f32,
    any_diagonal: f32,
    any_vertical: f32,
    via_cost: f32,
    weight: f32,
}

impl<'a> Guide<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        points: Option<&'a [(i32, i32, f32)]>,
        plane_target: bool,
        target_cells: impl Iterator<Item = (usize, usize)>,
        (nx, ny): (usize, usize),
        direction_cost: Vec<[f32; 8]>,
        via_cost: f32,
        weight: f32,
    ) -> Self {
        let blocks_x = nx.div_ceil(BLOCK);
        let blocks_y = ny.div_ceil(BLOCK);
        // Without a short list of target points, guide the search with an
        // exact octile distance transform over coarse blocks.
        let coarse: Vec<f32> = if points.is_some() || plane_target {
            Vec::new()
        } else {
            coarse_distances(target_cells, blocks_x, blocks_y)
        };
        let minimum = |direction: usize| {
            direction_cost
                .iter()
                .map(|costs| costs[direction])
                .fold(f32::INFINITY, f32::min)
        };
        let (any_horizontal, any_diagonal, any_vertical) = (minimum(0), minimum(1), minimum(2));
        Self {
            plane_target,
            points,
            coarse,
            blocks_x,
            direction_cost,
            any_horizontal,
            any_diagonal,
            any_vertical,
            via_cost,
            weight,
        }
    }

    /// The estimate of the cost left from node `(x, y)` on `layer`.
    pub(super) fn heuristic(&self, layer: usize, x: i32, y: i32) -> f32 {
        if self.plane_target {
            // A pour is everywhere; the nearest free spot is close.
            return 0.0;
        }
        let Some(points) = self.points else {
            let blocks = self.coarse[(y as usize / BLOCK) * self.blocks_x + x as usize / BLOCK];
            return (blocks - std::f32::consts::SQRT_2).max(0.0)
                * BLOCK as f32
                * self.any_horizontal.min(self.any_vertical)
                * self.weight;
        };
        let costs = &self.direction_cost[layer];
        let mut best = f32::INFINITY;
        for (tx, ty, reach) in points {
            let dx = (x - tx).abs() as f32;
            let dy = (y - ty).abs() as f32;
            let both = dx.min(dy);
            // Stay on this layer, or pay a via for the cheapest mix.
            let same_layer = both * costs[1] + (dx - both) * costs[0] + (dy - both) * costs[2];
            let any_layer = self.via_cost
                + both * self.any_diagonal
                + (dx - both) * self.any_horizontal
                + (dy - both) * self.any_vertical;
            best = best.min(same_layer.min(any_layer) - reach * 2.0);
        }
        best.max(0.0) * self.weight
    }
}

/// The exact octile distance, in blocks, from each block of
/// `blocks_x` x `blocks_y` to the nearest block holding a target cell
/// (two passes of a chamfer transform).
pub(super) fn coarse_distances(
    target_cells: impl Iterator<Item = (usize, usize)>,
    blocks_x: usize,
    blocks_y: usize,
) -> Vec<f32> {
    let mut distance = vec![f32::INFINITY; blocks_x * blocks_y];
    for (x, y) in target_cells {
        distance[(y / BLOCK) * blocks_x + x / BLOCK] = 0.0;
    }
    let diagonal = std::f32::consts::SQRT_2;
    let relax = |x: usize, y: usize, dx: i64, dy: i64, distance: &mut Vec<f32>| {
        let sx = x as i64 + dx;
        let sy = y as i64 + dy;
        if sx < 0 || sy < 0 || sx >= blocks_x as i64 || sy >= blocks_y as i64 {
            return;
        }
        let step = if dx != 0 && dy != 0 { diagonal } else { 1.0 };
        let candidate = distance[sy as usize * blocks_x + sx as usize] + step;
        if candidate < distance[y * blocks_x + x] {
            distance[y * blocks_x + x] = candidate;
        }
    };
    for y in 0..blocks_y {
        for x in 0..blocks_x {
            for (dx, dy) in [(-1, 0), (-1, -1), (0, -1), (1, -1)] {
                relax(x, y, dx, dy, &mut distance);
            }
        }
    }
    for y in (0..blocks_y).rev() {
        for x in (0..blocks_x).rev() {
            for (dx, dy) in [(1, 0), (1, 1), (0, 1), (-1, 1)] {
                relax(x, y, dx, dy, &mut distance);
            }
        }
    }
    distance
}

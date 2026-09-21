// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Connectivity of copper pours on the routing lattice.
//!
//! Tracks of other nets cut a pour into pieces. This module labels the solid
//! pieces, works out which terminals and which of the net's own tracks and
//! vias touch which piece, and names the main piece. The router uses it to
//! stitch islands to the main piece with vias.

use crate::grid::Grid;

/// Per layer and node: the pour's brush (its minimum width, keeping the pour's
/// clearance) fits here, so there is solid pour copper.
pub struct PourMap {
    pub solid: Vec<Vec<bool>>,
    /// Per layer and node: piece label, 0 for none. Labels are shared
    /// across layers and start at 1.
    pub label: Vec<Vec<u32>>,
    pub pieces: usize,
}

impl PourMap {
    pub fn build(grid: &Grid, free: &[Vec<bool>]) -> Self {
        let (nx, ny) = (grid.nx, grid.ny);
        // `free` already says that the pour's brush fits, so every free node
        // is solid copper; the rim is left out to keep neighbour access safe.
        let mut solid = Vec::new();
        for layer in free {
            let mut result = vec![false; layer.len()];
            if !layer.is_empty() {
                for y in 1..ny.saturating_sub(1) {
                    for x in 1..nx.saturating_sub(1) {
                        result[y * nx + x] = layer[y * nx + x];
                    }
                }
            }
            solid.push(result);
        }
        let mut label: Vec<Vec<u32>> = solid.iter().map(|layer| vec![0; layer.len()]).collect();
        let mut pieces = 0;
        let mut stack = Vec::new();
        for layer in 0..solid.len() {
            for start in 0..solid[layer].len() {
                if !solid[layer][start] || label[layer][start] != 0 {
                    continue;
                }
                pieces += 1;
                label[layer][start] = pieces as u32;
                stack.push(start);
                while let Some(cell) = stack.pop() {
                    for next in [cell - 1, cell + 1, cell - nx, cell + nx] {
                        if solid[layer][next] && label[layer][next] == 0 {
                            label[layer][next] = pieces as u32;
                            stack.push(next);
                        }
                    }
                }
            }
        }
        Self {
            solid,
            label,
            pieces,
        }
    }

    /// The piece nearest to `cell` on `layer` within `reach` nodes.
    pub fn piece_near(&self, grid: &Grid, layer: usize, cell: usize, reach: usize) -> Option<u32> {
        let labels = &self.label[layer];
        if labels.is_empty() {
            return None;
        }
        let (x, y) = grid.xy(cell);
        let mut best: Option<(usize, u32)> = None;
        for ty in y.saturating_sub(reach)..=(y + reach).min(grid.ny - 1) {
            for tx in x.saturating_sub(reach)..=(x + reach).min(grid.nx - 1) {
                let piece = labels[ty * grid.nx + tx];
                if piece == 0 {
                    continue;
                }
                let distance = (tx as i64 - x as i64).pow(2) + (ty as i64 - y as i64).pow(2);
                if best.is_none_or(|(best, _)| (distance as usize) < best) {
                    best = Some((distance as usize, piece));
                }
            }
        }
        best.map(|(_, piece)| piece)
    }

    /// Whether every node within `reach` of `cell` is solid on `layer`.
    pub fn solid_around(&self, grid: &Grid, layer: usize, cell: usize, reach: usize) -> bool {
        let solid = &self.solid[layer];
        if solid.is_empty() {
            return false;
        }
        let (x, y) = grid.xy(cell);
        if x < reach || y < reach || x + reach >= grid.nx || y + reach >= grid.ny {
            return false;
        }
        (y - reach..=y + reach).all(|ty| (x - reach..=x + reach).all(|tx| solid[ty * grid.nx + tx]))
    }
}

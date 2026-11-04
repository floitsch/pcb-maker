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
    /// Labels the pieces of `free`. Side by side nodes join; diagonal
    /// neighbours join when `diagonal(layer, start, orientation)` lets the
    /// brush take the step from `start` (orientation 0: towards +x +y, 1:
    /// towards +x -y), asked only where both common neighbours are not
    /// free (otherwise they join through one of those).
    pub fn build(
        grid: &Grid,
        free: &[Vec<bool>],
        diagonal: &dyn Fn(usize, usize, usize) -> bool,
    ) -> Self {
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
            // This layer's maps, looked up once rather than per neighbour.
            let solid = &solid[layer];
            let label = &mut label[layer];
            for start in 0..solid.len() {
                if !solid[start] || label[start] != 0 {
                    continue;
                }
                pieces += 1;
                label[start] = pieces as u32;
                stack.push(start);
                while let Some(cell) = stack.pop() {
                    for next in [cell - 1, cell + 1, cell - nx, cell + nx] {
                        if solid[next] && label[next] == 0 {
                            label[next] = pieces as u32;
                            stack.push(next);
                        }
                    }
                    // Diagonal neighbours, through the step's start and
                    // orientation as the occupancy's diagonal maps name it.
                    for (dx, dy) in [(1i64, 1i64), (1, -1), (-1, 1), (-1, -1)] {
                        let next = (cell as i64 + dy * nx as i64 + dx) as usize;
                        if !solid[next] || label[next] != 0 {
                            continue;
                        }
                        let corner_x = (cell as i64 + dx) as usize;
                        let corner_y = (cell as i64 + dy * nx as i64) as usize;
                        if solid[corner_x] || solid[corner_y] {
                            continue;
                        }
                        let (start, orientation) = if dx > 0 {
                            (cell, (dy < 0) as usize)
                        } else {
                            (next, (dy > 0) as usize)
                        };
                        if diagonal(layer, start, orientation) {
                            label[next] = pieces as u32;
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

    /// The pieces within `reach` nodes of `cell` on `layer`, each with its
    /// node nearest `cell` (in label order).
    pub fn pieces_near(
        &self,
        grid: &Grid,
        layer: usize,
        cell: usize,
        reach: usize,
    ) -> Vec<(u32, usize)> {
        let labels = &self.label[layer];
        if labels.is_empty() {
            return Vec::new();
        }
        let (x, y) = grid.xy(cell);
        let mut best: Vec<(u32, usize, usize)> = Vec::new();
        for ty in y.saturating_sub(reach)..=(y + reach).min(grid.ny - 1) {
            for tx in x.saturating_sub(reach)..=(x + reach).min(grid.nx - 1) {
                let index = ty * grid.nx + tx;
                let piece = labels[index];
                if piece == 0 {
                    continue;
                }
                let distance =
                    ((tx as i64 - x as i64).pow(2) + (ty as i64 - y as i64).pow(2)) as usize;
                match best.iter_mut().find(|(other, _, _)| *other == piece) {
                    Some(entry) if distance < entry.2 => *entry = (piece, index, distance),
                    Some(_) => {}
                    None => best.push((piece, index, distance)),
                }
            }
        }
        best.sort_unstable();
        best.into_iter()
            .map(|(piece, index, _)| (piece, index))
            .collect()
    }

    /// `piece_near`, its window visited nearest first (`NearOrder`) up to
    /// the first labelled node: the same piece, the same tie-break.
    pub fn piece_near_ordered(
        &self,
        grid: &Grid,
        layer: usize,
        cell: usize,
        order: &NearOrder,
    ) -> Option<u32> {
        let labels = &self.label[layer];
        if labels.is_empty() {
            return None;
        }
        let (x, y) = grid.xy(cell);
        let (x, y) = (x as i64, y as i64);
        let (nx, ny) = (grid.nx as i64, grid.ny as i64);
        for &(dx, dy) in &order.offsets {
            let (tx, ty) = (x + dx as i64, y + dy as i64);
            if tx < 0 || ty < 0 || tx >= nx || ty >= ny {
                continue;
            }
            let piece = labels[(ty * nx + tx) as usize];
            if piece != 0 {
                return Some(piece);
            }
        }
        None
    }

    /// `pieces_near` where the window can hold only pieces of `among` (a
    /// sorted list holding every piece the window has): the window visited
    /// nearest first (`NearOrder`), each piece's first node its nearest,
    /// until every piece of `among` was met. The same pieces and nodes.
    pub fn pieces_near_ordered(
        &self,
        grid: &Grid,
        layer: usize,
        cell: usize,
        order: &NearOrder,
        among: &[u32],
    ) -> Vec<(u32, usize)> {
        let labels = &self.label[layer];
        let mut found: Vec<(u32, usize)> = Vec::new();
        if labels.is_empty() || among.is_empty() {
            return found;
        }
        let (x, y) = grid.xy(cell);
        let (x, y) = (x as i64, y as i64);
        let (nx, ny) = (grid.nx as i64, grid.ny as i64);
        for &(dx, dy) in &order.offsets {
            let (tx, ty) = (x + dx as i64, y + dy as i64);
            if tx < 0 || ty < 0 || tx >= nx || ty >= ny {
                continue;
            }
            let index = (ty * nx + tx) as usize;
            let piece = labels[index];
            if piece == 0 || found.iter().any(|(other, _)| *other == piece) {
                continue;
            }
            found.push((piece, index));
            if found.len() == among.len() {
                break;
            }
        }
        found.sort_unstable();
        found
    }

    /// The pieces with a node in the box `x0..=x1` by `y0..=y1` (clipped to
    /// the lattice) on `layer`, sorted: what `pieces_near_ordered` may find
    /// around the nodes whose windows lie in the box.
    pub fn pieces_in_box(
        &self,
        grid: &Grid,
        layer: usize,
        (x0, y0, x1, y1): (i64, i64, i64, i64),
    ) -> Vec<u32> {
        let labels = &self.label[layer];
        let mut pieces: Vec<u32> = Vec::new();
        if labels.is_empty() {
            return pieces;
        }
        let (x0, y0) = (x0.max(0) as usize, y0.max(0) as usize);
        let x1 = x1.min(grid.nx as i64 - 1);
        let y1 = y1.min(grid.ny as i64 - 1);
        if x1 < x0 as i64 || y1 < y0 as i64 {
            return pieces;
        }
        let (x1, y1) = (x1 as usize, y1 as usize);
        let mut last = 0;
        for ty in y0..=y1 {
            for &piece in &labels[ty * grid.nx + x0..=ty * grid.nx + x1] {
                if piece != 0 && piece != last {
                    last = piece;
                    if !pieces.contains(&piece) {
                        pieces.push(piece);
                    }
                }
            }
        }
        pieces.sort_unstable();
        pieces
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

/// The nodes of a square window of `reach` nodes around a node as
/// offsets, nearest first and nodes as far in the window's row-major order
/// (`dy`, then `dx`): a row-major scan keeping a strictly nearer node
/// keeps, of the nodes nearest, the first in this order too
/// (`PourMap::piece_near_ordered`, `PourMap::pieces_near_ordered`).
pub struct NearOrder {
    pub reach: usize,
    offsets: Vec<(i32, i32)>,
}

impl NearOrder {
    pub fn new(reach: usize) -> Self {
        let reach_i = reach as i32;
        let mut offsets: Vec<(i32, i32)> = (-reach_i..=reach_i)
            .flat_map(|dy| (-reach_i..=reach_i).map(move |dx| (dx, dy)))
            .collect();
        offsets.sort_by_key(|&(dx, dy)| (dx as i64 * dx as i64 + dy as i64 * dy as i64, dy, dx));
        Self { reach, offsets }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pour map of one layer, `nx` by `ny`, labels from a small
    /// generator: runs of pieces 1 to 4 and holes.
    fn map(nx: usize, ny: usize, seed: u64) -> (Grid, PourMap) {
        let mut state = seed | 1;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut label = vec![0u32; nx * ny];
        let mut piece = 0;
        for value in label.iter_mut() {
            if next() % 7 == 0 {
                piece = (next() % 5) as u32;
            }
            *value = piece;
        }
        let grid = Grid {
            origin: [0.0, 0.0],
            pitch: 0.1,
            nx,
            ny,
        };
        let pours = PourMap {
            solid: vec![Vec::new()],
            label: vec![label],
            pieces: 4,
        };
        (grid, pours)
    }

    #[test]
    fn the_ordered_windows_find_what_the_scans_find() {
        for (nx, ny, seed) in [(23, 17, 1), (40, 9, 7), (9, 31, 11), (64, 64, 3)] {
            let (grid, pours) = map(nx, ny, seed);
            for reach in [0, 1, 2, 5, 16] {
                let order = NearOrder::new(reach);
                for cell in 0..nx * ny {
                    assert_eq!(
                        pours.piece_near_ordered(&grid, 0, cell, &order),
                        pours.piece_near(&grid, 0, cell, reach),
                        "piece_near at {cell}, reach {reach}"
                    );
                    let (x, y) = grid.xy(cell);
                    let r = reach as i64;
                    let among = pours.pieces_in_box(
                        &grid,
                        0,
                        (x as i64 - r, y as i64 - r, x as i64 + r, y as i64 + r),
                    );
                    let expected = pours.pieces_near(&grid, 0, cell, reach);
                    assert_eq!(
                        pours.pieces_near_ordered(&grid, 0, cell, &order, &among),
                        expected,
                        "pieces_near at {cell}, reach {reach}"
                    );
                    // A wider set of pieces than the window holds: the
                    // whole window is visited, the same answer.
                    assert_eq!(
                        pours.pieces_near_ordered(&grid, 0, cell, &order, &[1, 2, 3, 4]),
                        expected
                    );
                }
            }
        }
    }

    #[test]
    fn the_order_is_nearest_first_then_row_major() {
        let order = NearOrder::new(2);
        assert_eq!(order.offsets.len(), 25);
        assert_eq!(order.offsets[0], (0, 0));
        // At distance 1: above, left, right, below (row-major).
        assert_eq!(&order.offsets[1..5], &[(0, -1), (-1, 0), (1, 0), (0, 1)]);
        assert_eq!(&order.offsets[5..9], &[(-1, -1), (1, -1), (-1, 1), (1, 1)]);
    }
}

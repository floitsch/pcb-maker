// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A uniform bucket grid over the board: which parts lie near a box, so
//! that a legality test or an overlap sum looks at those parts only.

use crate::problem::Point;

pub(crate) struct Grid {
    origin: Point,
    cell: f64,
    columns: usize,
    rows: usize,
    cells: Vec<Vec<usize>>,
    seen: Vec<u32>,
    stamp: u32,
}

impl Grid {
    /// A grid over `bounds` (boxes beyond it fall into the border cells)
    /// for items `0..items`.
    pub(crate) fn new(bounds: [f64; 4], cell: f64, items: usize) -> Self {
        let columns = (((bounds[2] - bounds[0]) / cell).ceil() as usize).max(1) + 1;
        let rows = (((bounds[3] - bounds[1]) / cell).ceil() as usize).max(1) + 1;
        Grid {
            origin: [bounds[0], bounds[1]],
            cell,
            columns,
            rows,
            cells: vec![Vec::new(); columns * rows],
            seen: vec![0; items],
            stamp: 0,
        }
    }

    fn range(&self, center: Point, half: Point) -> (usize, usize, usize, usize) {
        let clamp = |value: f64, count: usize| {
            if value.is_nan() { 0 } else { (value.floor().max(0.0) as usize).min(count - 1) }
        };
        (
            clamp((center[0] - half[0] - self.origin[0]) / self.cell, self.columns),
            clamp((center[1] - half[1] - self.origin[1]) / self.cell, self.rows),
            clamp((center[0] + half[0] - self.origin[0]) / self.cell, self.columns),
            clamp((center[1] + half[1] - self.origin[1]) / self.cell, self.rows),
        )
    }

    pub(crate) fn insert(&mut self, item: usize, center: Point, half: Point) {
        let (x0, y0, x1, y1) = self.range(center, half);
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.cells[y * self.columns + x].push(item);
            }
        }
    }

    /// Takes out an item inserted with this box.
    pub(crate) fn remove(&mut self, item: usize, center: Point, half: Point) {
        let (x0, y0, x1, y1) = self.range(center, half);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let cell = &mut self.cells[y * self.columns + x];
                if let Some(position) = cell.iter().position(|other| *other == item) {
                    cell.swap_remove(position);
                }
            }
        }
    }

    /// The items whose boxes may meet this one, each once; returns the
    /// cells looked at (work).
    pub(crate) fn query(&mut self, center: Point, half: Point, out: &mut Vec<usize>) -> usize {
        out.clear();
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.seen.iter_mut().for_each(|seen| *seen = 0);
            self.stamp = 1;
        }
        let (x0, y0, x1, y1) = self.range(center, half);
        for y in y0..=y1 {
            for x in x0..=x1 {
                for &item in &self.cells[y * self.columns + x] {
                    if self.seen[item] != self.stamp {
                        self.seen[item] = self.stamp;
                        out.push(item);
                    }
                }
            }
        }
        (x1 - x0 + 1) * (y1 - y0 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_what_is_near_once_and_forgets_what_left() {
        let mut grid = Grid::new([0.0, 0.0, 10.0, 10.0], 2.0, 3);
        grid.insert(0, [1.0, 1.0], [0.5, 0.5]);
        grid.insert(1, [5.0, 5.0], [3.0, 3.0]);
        grid.insert(2, [9.0, 9.0], [0.5, 0.5]);
        let mut near = Vec::new();
        grid.query([4.0, 4.0], [0.5, 0.5], &mut near);
        assert_eq!(near, vec![1]);
        grid.query([1.0, 1.0], [8.5, 8.5], &mut near);
        near.sort_unstable();
        assert_eq!(near, vec![0, 1, 2]);
        grid.remove(1, [5.0, 5.0], [3.0, 3.0]);
        grid.query([4.0, 4.0], [0.5, 0.5], &mut near);
        assert!(near.is_empty());
        // Beyond the bounds: the border cells.
        grid.insert(1, [-20.0, 30.0], [1.0, 1.0]);
        grid.query([-1.0, 11.0], [0.1, 0.1], &mut near);
        assert_eq!(near, vec![1]);
    }
}

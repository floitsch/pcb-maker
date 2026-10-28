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
    /// The answers of `query` since an item last went in or out, by
    /// their cells (`with_answers`), or None.
    answers: Option<Answers>,
}

/// `Grid::query`'s answers by the cells a query looks at, while no item
/// goes into or out of those cells: the same cells then hold the same
/// items in the same order, so a query of the same cells has the same
/// answer (an item going in or out forgets the answers of the cells it
/// meets, `forget`). The
/// legaliser's spot search, the refinement's candidates and the eviction
/// query a grid that holds still for one part's spots, steps of 0.25 to
/// 0.5 mm in cells of 2.5 mm: most spots look at the cells of a spot
/// before them (the bucket queries were 27 % of protoesp's legalisation's
/// instructions, video's spots look at 60 cells for 1.7 parts,
/// 2026-10-28).
struct Answers {
    /// Direct-mapped by the cells' key: (key, generation, start, length)
    /// into `items`.
    slots: Vec<(u64, u32, u32, u32)>,
    items: Vec<usize>,
    /// The answers' generation: a slot of another is empty.
    generation: u32,
    /// The slots filled in this generation (some more than once).
    live: Vec<u32>,
    /// Every answer kept checked against a scan (debug builds and
    /// `PCB_PLACER_CHECK_OVERLAP`).
    check: bool,
}

/// The answers kept: a power of two.
const ANSWER_SLOTS: usize = 4096;

/// More items than this kept start the answers again: an answer whose
/// slot another took is kept anew, so the items grow with the misses
/// (at 2^20, half a megabyte a grid became eight: protoesp's peak 67 MB
/// went to 76, 2026-10-28).
const ANSWER_ITEMS: usize = 1 << 16;

impl Answers {
    fn new() -> Self {
        Answers {
            slots: vec![(0, 0, 0, 0); ANSWER_SLOTS],
            items: Vec::new(),
            generation: 1,
            live: Vec::new(),
            check: cfg!(debug_assertions) || std::env::var_os("PCB_PLACER_CHECK_OVERLAP").is_some(),
        }
    }

    /// Forgets the answers of the cells that meet `range` (an item went
    /// into or out of those cells; the others hold what they held). Since
    /// 2026-10-28 an item going in or out forgot every answer: the
    /// legaliser placed a part and the next part's search started anew.
    fn forget(&mut self, range: (usize, usize, usize, usize)) {
        let (x0, y0, x1, y1) = (
            range.0 as u64,
            range.1 as u64,
            range.2 as u64,
            range.3 as u64,
        );
        let generation = self.generation;
        for &slot in &self.live {
            let entry = &mut self.slots[slot as usize];
            if entry.1 != generation {
                continue;
            }
            let key = entry.0;
            let (kx0, ky0, kx1, ky1) = (
                key & 0xFFFF,
                (key >> 16) & 0xFFFF,
                (key >> 32) & 0xFFFF,
                key >> 48,
            );
            if kx0 <= x1 && x0 <= kx1 && ky0 <= y1 && y0 <= ky1 {
                entry.1 = 0;
            }
        }
    }

    /// Forgets every answer.
    fn clear(&mut self) {
        self.items.clear();
        self.live.clear();
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.slots.iter_mut().for_each(|slot| *slot = (0, 0, 0, 0));
            self.generation = 1;
        }
    }

    fn slot(key: u64) -> usize {
        (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - ANSWER_SLOTS.trailing_zeros())) as usize
    }
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
            answers: None,
        }
    }

    /// The grid with `query`'s answers kept while no item goes into or
    /// out of their cells (`Answers`): the same answers, each found once. A grid of more
    /// than 2^16 columns or rows keeps none.
    pub(crate) fn with_answers(mut self) -> Self {
        if self.columns <= 1 << 16 && self.rows <= 1 << 16 {
            self.answers = Some(Answers::new());
        }
        self
    }

    fn range(&self, center: Point, half: Point) -> (usize, usize, usize, usize) {
        let clamp = |value: f64, count: usize| cell_index(value).min(count - 1);
        (
            clamp(
                (center[0] - half[0] - self.origin[0]) / self.cell,
                self.columns,
            ),
            clamp(
                (center[1] - half[1] - self.origin[1]) / self.cell,
                self.rows,
            ),
            clamp(
                (center[0] + half[0] - self.origin[0]) / self.cell,
                self.columns,
            ),
            clamp(
                (center[1] + half[1] - self.origin[1]) / self.cell,
                self.rows,
            ),
        )
    }

    pub(crate) fn insert(&mut self, item: usize, center: Point, half: Point) {
        let (x0, y0, x1, y1) = self.range(center, half);
        if let Some(answers) = self.answers.as_mut() {
            answers.forget((x0, y0, x1, y1));
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.cells[y * self.columns + x].push(item);
            }
        }
    }

    /// Takes out an item inserted with this box.
    pub(crate) fn remove(&mut self, item: usize, center: Point, half: Point) {
        let (x0, y0, x1, y1) = self.range(center, half);
        if let Some(answers) = self.answers.as_mut() {
            answers.forget((x0, y0, x1, y1));
        }
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
        let range = self.range(center, half);
        let (x0, y0, x1, y1) = range;
        let cells = (x1 - x0 + 1) * (y1 - y0 + 1);
        let Some(answers) = self.answers.as_mut() else {
            self.scan(range, out);
            return cells;
        };
        let key = (x0 as u64) | (y0 as u64) << 16 | (x1 as u64) << 32 | (y1 as u64) << 48;
        let slot = Answers::slot(key);
        let (kept, generation, start, length) = answers.slots[slot];
        if generation == answers.generation && kept == key {
            out.clear();
            out.extend_from_slice(&answers.items[start as usize..(start + length) as usize]);
            if answers.check {
                let mut scanned = Vec::new();
                self.scan(range, &mut scanned);
                assert_eq!(*out, scanned, "kept answer of cells {range:?}");
            }
            return cells;
        }
        self.scan(range, out);
        let answers = self.answers.as_mut().expect("answers kept");
        if answers.items.len() + out.len() > ANSWER_ITEMS || answers.live.len() >= ANSWER_ITEMS {
            answers.clear();
        }
        let start = answers.items.len() as u32;
        answers.items.extend_from_slice(out);
        answers.slots[slot] = (key, answers.generation, start, out.len() as u32);
        answers.live.push(slot as u32);
        cells
    }

    /// The items of the cells `range` (`range`'s), each once, in the
    /// order of a row-major scan of the cells and each cell's list.
    fn scan(&mut self, range: (usize, usize, usize, usize), out: &mut Vec<usize>) {
        out.clear();
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.seen.iter_mut().for_each(|seen| *seen = 0);
            self.stamp = 1;
        }
        let (x0, y0, x1, y1) = range;
        let (stamp, seen) = (self.stamp, &mut self.seen);
        for y in y0..=y1 {
            let row = y * self.columns;
            for cell in &self.cells[row + x0..=row + x1] {
                // Without a branch on whether the item was seen (a part
                // spanning several cells is met in each; the test and its
                // load were most of the query's samples on MegaDrive): the
                // item goes into the free slot past the end and counts only
                // when new. The same items in the same order as a `push` of
                // the new ones. The length stays in a register for the
                // cell (the vector's own `spare_capacity_mut` and `set_len`
                // per item were a third of the query's instructions).
                out.reserve(cell.len());
                let slots = out.as_mut_ptr();
                let mut length = out.len();
                for &item in cell {
                    let new = seen[item] != stamp;
                    seen[item] = stamp;
                    // SAFETY: `reserve` left room for the cell's items past
                    // `length`, which grows by at most one an item.
                    unsafe { slots.add(length).write(item) };
                    length += new as usize;
                }
                // SAFETY: the slots below `length` were written (above, or
                // before this cell).
                unsafe { out.set_len(length) };
            }
        }
    }
}

/// The cell a coordinate in cell units falls into, before the clamp to the
/// grid's last cell: `value.floor().max(0.0) as usize` (0 for NaN), the
/// same for every input without the call into libm that `floor` is on
/// x86-64's baseline (1 to 2.6 % of a placement's instructions,
/// 2026-10-28): a cast truncates towards zero and takes negatives and NaN
/// to 0, which is the floor's answer after the `max` for every value.
pub(crate) fn cell_index(value: f64) -> usize {
    value as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_index_is_the_floor_clamped_at_zero() {
        let before = |value: f64| {
            if value.is_nan() {
                0
            } else {
                value.floor().max(0.0) as usize
            }
        };
        let mut values = vec![
            0.0,
            -0.0,
            0.5,
            -0.5,
            1.0,
            -1.0,
            0.999_999_999_999_999_9,
            -1.0e-300,
            1.0e-300,
            f64::MIN_POSITIVE,
            f64::EPSILON,
            1.0 - f64::EPSILON,
            4_503_599_627_370_495.5,
            9.0e15,
            1.0e19,
            1.0e300,
            -1.0e300,
            f64::MAX,
            f64::MIN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            -f64::NAN,
        ];
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..100_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            values.push((state >> 11) as f64 / (1u64 << 53) as f64 * 2000.0 - 1000.0);
            values.push(f64::from_bits(state));
        }
        for value in values {
            assert_eq!(cell_index(value), before(value), "{value:e}");
        }
    }

    #[test]
    fn kept_answers_are_the_scans_answers() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for round in 0..20 {
            let items = 60;
            let bounds = [0.0, 0.0, 20.0 + round as f64, 15.0];
            let mut plain = Grid::new(bounds, 2.5, items);
            let mut kept = Grid::new(bounds, 2.5, items).with_answers();
            assert!(kept.answers.is_some());
            let mut boxes: Vec<Option<(Point, Point)>> = vec![None; items];
            let (mut a, mut b) = (Vec::new(), Vec::new());
            for _ in 0..4000 {
                let roll = next();
                if roll < 0.08 {
                    // An item goes in or out (or moves: out, then in).
                    let item = (next() * items as f64) as usize;
                    if let Some((center, half)) = boxes[item].take() {
                        plain.remove(item, center, half);
                        kept.remove(item, center, half);
                    }
                    if next() < 0.7 {
                        let center = [next() * 30.0 - 5.0, next() * 25.0 - 5.0];
                        let half = [next() * 3.0, next() * 3.0];
                        plain.insert(item, center, half);
                        kept.insert(item, center, half);
                        boxes[item] = Some((center, half));
                    }
                } else {
                    // Queries on a lattice of spots, as the searches ask.
                    let center = [
                        (next() * 60.0).floor() * 0.5 - 3.0,
                        (next() * 40.0).floor() * 0.5 - 3.0,
                    ];
                    let half = [0.5 + (next() * 3.0).floor(), 0.5 + (next() * 2.0).floor()];
                    let cells = plain.query(center, half, &mut a);
                    assert_eq!(kept.query(center, half, &mut b), cells);
                    assert_eq!(a, b);
                }
            }
        }
    }

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

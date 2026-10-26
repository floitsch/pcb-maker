// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The dynamic occupancy: per map and cell, how many nets forbid the map's
//! class there, four bits a count.

use super::*;

/// The count four bits hold at most: a cell of this count keeps the rest
/// in `Occupancy::overflow`.
const SATURATED: u8 = 15;

/// Per occupancy map (`Router::occupancy`) and cell, how many nets forbid
/// that class there. Four bits a count, two cells a byte (the even cell in
/// the low half; a byte a count until 2026-10-26, two until 2026-10-23:
/// video's four probes held 530 MB of counts, a sixth of their peak): a
/// count is how many nets' stamps cover the cell, a handful where nets
/// overlap and 0 almost everywhere. Counts of 15 or more keep 15 in their
/// half and the rest in `overflow`, so every count is exact.
#[derive(Clone, Default)]
pub(super) struct Occupancy {
    maps: Vec<Vec<u8>>,
    /// Cells a map.
    cells: usize,
    /// Per (map, cell) of a saturated count: the count beyond 15.
    overflow: FastMap<(u32, u32), u32>,
}

/// The bit position of `cell`'s half in its byte.
#[inline(always)]
fn shift_of(cell: usize) -> u32 {
    ((cell & 1) * 4) as u32
}

impl Occupancy {
    pub(super) fn new(maps: usize, cells: usize) -> Self {
        Self {
            maps: (0..maps).map(|_| vec![0; cells.div_ceil(2)]).collect(),
            cells,
            overflow: FastMap::default(),
        }
    }

    /// How many maps there are.
    pub(super) fn len(&self) -> usize {
        self.maps.len()
    }

    /// The count at `cell` of `map`.
    #[inline(always)]
    pub(super) fn get(&self, map: usize, cell: usize) -> u16 {
        let value = (self.maps[map][cell >> 1] >> shift_of(cell)) & SATURATED;
        if value != SATURATED {
            value as u16
        } else {
            self.saturated(map, cell)
        }
    }

    #[cold]
    #[inline(never)]
    fn saturated(&self, map: usize, cell: usize) -> u16 {
        let extra = self
            .overflow
            .get(&(map as u32, cell as u32))
            .copied()
            .unwrap_or(0);
        (SATURATED as u32 + extra) as u16
    }

    /// Counts one more net at `cell` of `map`; whether the cell was free.
    #[inline(always)]
    pub(super) fn increment(&mut self, map: usize, cell: usize) -> bool {
        let shift = shift_of(cell);
        let byte = &mut self.maps[map][cell >> 1];
        let value = (*byte >> shift) & SATURATED;
        if value != SATURATED {
            *byte += 1 << shift;
            value == 0
        } else {
            *self.overflow.entry((map as u32, cell as u32)).or_insert(0) += 1;
            false
        }
    }

    /// Counts one net less at `cell` of `map`; whether the cell is free now.
    #[inline(always)]
    pub(super) fn decrement(&mut self, map: usize, cell: usize) -> bool {
        let shift = shift_of(cell);
        let byte = &mut self.maps[map][cell >> 1];
        if (*byte >> shift) & SATURATED == SATURATED
            && let Some(extra) = self.overflow.get_mut(&(map as u32, cell as u32))
        {
            *extra -= 1;
            if *extra == 0 {
                self.overflow.remove(&(map as u32, cell as u32));
            }
            return false;
        }
        *byte -= 1 << shift;
        (*byte >> shift) & SATURATED == 0
    }

    /// Asks for the cache line of `cell` of `map` early.
    #[inline(always)]
    pub(super) fn prefetch(&self, map: usize, cell: usize) {
        prefetch(&self.maps[map][cell >> 1]);
    }

    /// `prefetch_index` of `cell` of `map` (a cell past the map's end
    /// prefetches nothing useful and faults on nothing).
    #[inline(always)]
    pub(super) fn prefetch_index(&self, map: usize, cell: usize) {
        prefetch_index(&self.maps[map], cell >> 1);
    }

    /// Every count back to 0.
    pub(super) fn clear(&mut self) {
        for map in &mut self.maps {
            map.fill(0);
        }
        self.overflow.clear();
    }

    /// Every count, map after map (the fingerprint of every map).
    pub(super) fn values(&self) -> impl Iterator<Item = u16> + '_ {
        (0..self.maps.len())
            .flat_map(move |map| (0..self.cells).map(move |cell| self.get(map, cell)))
    }

    pub(super) fn bytes(&self) -> usize {
        self.maps.iter().map(Vec::capacity).sum::<usize>()
            + self.overflow.capacity() * std::mem::size_of::<((u32, u32), u32)>()
    }

    /// The counts beyond 15 kept apart (the memory profile).
    pub(super) fn overflowing(&self) -> usize {
        self.overflow.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_beyond_a_byte_are_exact() {
        let mut occupancy = Occupancy::new(2, 4);
        for count in 1..=600u16 {
            assert_eq!(occupancy.increment(1, 2), count == 1);
            assert_eq!(occupancy.get(1, 2), count);
        }
        assert_eq!(occupancy.get(0, 2), 0);
        assert_eq!(occupancy.get(1, 3), 0);
        for count in (0..600u16).rev() {
            assert_eq!(occupancy.decrement(1, 2), count == 0);
            assert_eq!(occupancy.get(1, 2), count);
        }
        assert!(occupancy.overflow.is_empty());
    }

    #[test]
    fn neighbouring_cells_count_apart() {
        // Two cells share a byte: each counts on its own, past the
        // saturation too, and the last cell of an odd map has its half.
        let mut occupancy = Occupancy::new(3, 5);
        let mut counts = [[0u16; 5]; 3];
        let mut state = 0x2545_f491_4f6c_dd1du64;
        for step in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let (map, cell) = ((state % 3) as usize, ((state >> 8) % 5) as usize);
            // Up more than down early, then down to empty.
            let up = if step < 12_000 {
                !(state >> 20).is_multiple_of(3)
            } else {
                false
            };
            if up {
                assert_eq!(occupancy.increment(map, cell), counts[map][cell] == 0);
                counts[map][cell] += 1;
            } else if counts[map][cell] > 0 {
                counts[map][cell] -= 1;
                assert_eq!(occupancy.decrement(map, cell), counts[map][cell] == 0);
            }
            for (map, row) in counts.iter().enumerate() {
                for (cell, count) in row.iter().enumerate() {
                    assert_eq!(occupancy.get(map, cell), *count);
                }
            }
        }
        assert_eq!(
            occupancy.values().collect::<Vec<_>>(),
            counts.iter().flatten().copied().collect::<Vec<_>>()
        );
        assert!(counts.iter().flatten().all(|count| *count == 0));
        assert!(occupancy.overflow.is_empty());
    }
}

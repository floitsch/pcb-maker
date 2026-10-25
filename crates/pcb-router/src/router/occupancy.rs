// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The dynamic occupancy: per map and cell, how many nets forbid the map's
//! class there, a byte a count.

use super::*;

/// The count a byte holds at most: a cell of this count keeps the rest in
/// `Occupancy::overflow`.
const SATURATED: u8 = u8::MAX;

/// Per occupancy map (`Router::occupancy`) and cell, how many nets forbid
/// that class there. A byte a count (two until 2026-10-23: katia 741 MB):
/// a count is how many nets' stamps cover the cell, a handful where nets
/// overlap and 0 almost everywhere. Counts of 255 or more keep 255 in
/// their byte and the rest in `overflow`, so every count is exact.
#[derive(Clone, Default)]
pub(super) struct Occupancy {
    maps: Vec<Vec<u8>>,
    /// Per (map, cell) of a saturated byte: the count beyond 255.
    overflow: FastMap<(u32, u32), u32>,
}

impl Occupancy {
    pub(super) fn new(maps: usize, cells: usize) -> Self {
        Self {
            maps: vec![vec![0; cells]; maps],
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
        let value = self.maps[map][cell];
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
        let value = &mut self.maps[map][cell];
        if *value != SATURATED {
            *value += 1;
            *value == 1
        } else {
            *self.overflow.entry((map as u32, cell as u32)).or_insert(0) += 1;
            false
        }
    }

    /// Counts one net less at `cell` of `map`; whether the cell is free now.
    #[inline(always)]
    pub(super) fn decrement(&mut self, map: usize, cell: usize) -> bool {
        let value = &mut self.maps[map][cell];
        if *value == SATURATED
            && let Some(extra) = self.overflow.get_mut(&(map as u32, cell as u32))
        {
            *extra -= 1;
            if *extra == 0 {
                self.overflow.remove(&(map as u32, cell as u32));
            }
            return false;
        }
        *value -= 1;
        *value == 0
    }

    /// Asks for the cache line of `cell` of `map` early.
    #[inline(always)]
    pub(super) fn prefetch(&self, map: usize, cell: usize) {
        prefetch(&self.maps[map][cell]);
    }

    /// `prefetch` of `cell` of `map` where there is such a cell.
    #[inline(always)]
    pub(super) fn prefetch_index(&self, map: usize, cell: usize) {
        if let Some(values) = self.maps.get(map) {
            prefetch_index(values, cell);
        }
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
            .flat_map(move |map| (0..self.maps[map].len()).map(move |cell| self.get(map, cell)))
    }

    pub(super) fn bytes(&self) -> usize {
        self.maps.iter().map(Vec::capacity).sum::<usize>()
            + self.overflow.capacity() * std::mem::size_of::<((u32, u32), u32)>()
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
}

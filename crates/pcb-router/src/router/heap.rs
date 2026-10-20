// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The search's open list.

pub(super) fn heap_key(estimate: f32, state: usize) -> u64 {
    ((estimate.to_bits() as u64) << 32) | state as u64
}

/// Four keys whose parent is one node, on one half of a cache line.
#[derive(Clone, Copy)]
#[repr(align(32))]
pub(super) struct KeyGroup([u64; 4]);

/// No key: fills the slots of a group past the heap's end, so that a
/// sift takes the least of four without counting them.
pub(super) const NO_KEY: u64 = u64::MAX;

/// A min-heap of search keys (`heap_key`: the estimate's bits, as
/// non-negative floats order as their bits, above the state, compared as
/// one word), four children a node: half the levels of a binary heap. The
/// children of a node are an aligned group on half a cache line, and a
/// sift finds the least of them without branches (since 2026-10-15; the
/// branches mispredicted). It lives in the search scratch, so that its
/// buffer is not grown again for every search (with the four-way layout
/// 9-13 % less CPU per routed board, k30-SBC and Sisu, identical routes).
/// Keys order totally (a key holds its state; equal keys are the same
/// entry twice), so the pops are those of any other min-heap.
#[derive(Default)]
pub(super) struct KeyHeap {
    /// Slot `q` is `groups[q / 4].0[q % 4]`. The root is slot 3, the
    /// children of slot `p` are the group `p - 2` (slots `4p - 8` to
    /// `4p - 5`), so that siblings never straddle a cache line. Slots 0 to
    /// 2 and those past the end hold `NO_KEY`.
    pub(super) groups: Vec<KeyGroup>,
    pub(super) len: usize,
}

impl KeyHeap {
    pub(super) fn clear(&mut self) {
        self.groups.clear();
        self.len = 0;
    }

    #[inline(always)]
    pub(super) fn slot(&mut self, q: usize) -> &mut u64 {
        &mut self.groups[q >> 2].0[q & 3]
    }

    #[inline(always)]
    pub(super) fn push(&mut self, key: u64) {
        let mut q = 3 + self.len;
        if self.groups.len() <= q >> 2 {
            self.groups.push(KeyGroup([NO_KEY; 4]));
        }
        self.len += 1;
        while q > 3 {
            let parent = (q >> 2) + 2;
            let above = *self.slot(parent);
            if above <= key {
                break;
            }
            *self.slot(q) = above;
            q = parent;
        }
        *self.slot(q) = key;
    }

    #[inline(always)]
    pub(super) fn pop(&mut self) -> Option<u64> {
        if self.len == 0 {
            return None;
        }
        let top = *self.slot(3);
        self.len -= 1;
        let end = 3 + self.len;
        let last = std::mem::replace(self.slot(end), NO_KEY);
        if self.len == 0 {
            return Some(top);
        }
        let mut q = 3;
        loop {
            if 4 * q - 8 >= end {
                break;
            }
            // The least of the four children without branches (`NO_KEY`
            // past the end; the first of equal keys, as any order would
            // pop the same keys).
            let [a, b, c, d] = self.groups[q - 2].0;
            let low = if b < a { 1 } else { 0 };
            let high = if d < c { 3 } else { 2 };
            let (first, second) = (a.min(b), c.min(d));
            let (least, which) = if second < first {
                (second, high)
            } else {
                (first, low)
            };
            if least >= last {
                break;
            }
            *self.slot(q) = least;
            q = 4 * q - 8 + which;
        }
        *self.slot(q) = last;
        Some(top)
    }
}

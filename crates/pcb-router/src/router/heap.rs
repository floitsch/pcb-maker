// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The search's open lists.

use super::SearchNode;

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
/// branches mispredicted; with the four-way layout 9-13 % less CPU per
/// routed board, k30-SBC and Sisu, identical routes). The tile planner's
/// and the bundles' searches use it; the router's search uses
/// `SearchHeap`, the same heap with a decrease-key (since 2026-10-27).
/// Keys order totally (a key holds its state; equal keys are the same
/// entry twice), so the pops are those of any other min-heap.
///
/// A pop leaves its slot empty (`hole`) and the next push fills it, sifting
/// the new key down from the root (since 2026-10-25). The search pops a
/// node and then pushes its neighbours, whose estimates are near the
/// least: they stop a level or two below the root, where the heap's last
/// key, moved to the root by a plain pop, sank to the bottom through every
/// level (the sift's loads were 10 to 15 % of the route's CPU;
/// docs/profiling.md). A pop with the root still empty fills it as a
/// plain pop does. The heap pops the same keys either way.
#[derive(Default)]
pub(super) struct KeyHeap {
    /// Slot `q` is `groups[q / 4].0[q % 4]`. The root is slot 3, the
    /// children of slot `p` are the group `p - 2` (slots `4p - 8` to
    /// `4p - 5`), so that siblings never straddle a cache line. Slots 0 to
    /// 2 and those past the end hold `NO_KEY`.
    pub(super) groups: Vec<KeyGroup>,
    /// The keys held. With `hole` they lie in slots 4 to `3 + len`, the
    /// root empty; otherwise in slots 3 to `2 + len`.
    pub(super) len: usize,
    /// Whether the root is empty (the last pop's), to be filled by the
    /// next push or pop.
    hole: bool,
}

impl KeyHeap {
    #[inline(always)]
    pub(super) fn slot(&mut self, q: usize) -> &mut u64 {
        &mut self.groups[q >> 2].0[q & 3]
    }

    #[inline(always)]
    pub(super) fn push(&mut self, key: u64) {
        if self.hole {
            // The root is empty: the new key goes there and down.
            self.hole = false;
            self.len += 1;
            self.sift_down(key, 3 + self.len);
            return;
        }
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
        if self.hole {
            // Nothing was pushed since the last pop: the last key fills
            // the root, as a plain pop does.
            self.hole = false;
            if self.len == 0 {
                return None;
            }
            let end = 3 + self.len;
            let last = std::mem::replace(self.slot(end), NO_KEY);
            self.sift_down(last, end);
        }
        if self.len == 0 {
            return None;
        }
        let top = *self.slot(3);
        self.len -= 1;
        self.hole = true;
        Some(top)
    }

    /// Puts `key` into the empty root and sifts it down among the slots
    /// before `end`.
    #[inline(always)]
    fn sift_down(&mut self, key: u64, end: usize) {
        let mut q = 3;
        loop {
            if 4 * q - 8 >= end {
                break;
            }
            // The four children's own children, the next level's groups
            // (4q - 10 to 4q - 7, 128 bytes on two or three cache lines):
            // asked for while this level's least is chosen, so that the
            // sift's loads overlap instead of waiting one after the other.
            let grandchildren = 4 * q - 10;
            for group in [grandchildren, grandchildren + 2, grandchildren + 3] {
                super::prefetch_index(&self.groups, group);
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
            if least >= key {
                break;
            }
            *self.slot(q) = least;
            q = 4 * q - 8 + which;
        }
        *self.slot(q) = key;
    }
}

/// The search's open list: the four-way heap of `KeyHeap` (the same
/// layout, the empty root a pop leaves, the prefetches) with each queued
/// node's slot kept in its `SearchNode::slot`, so that a better key for a
/// node already queued moves its entry up (`decrease`) instead of queueing
/// a second one that is popped stale later (since 2026-10-27: 39 to 41 % of
/// the pops were stale, and a stale pop left the next pop to sift the
/// heap's last key down through every level; the third router round,
/// docs/profiling.md). A key is lowered only for its own state (`heap_key`:
/// the estimate's bits above the state, so a lower cost through the same
/// heuristic gives a key no greater), and the heap holds a state's least
/// key, which the lazy heap popped first, so the nodes expand in the same
/// order.
#[derive(Default)]
pub(super) struct SearchHeap {
    /// As `KeyHeap::groups`.
    groups: Vec<KeyGroup>,
    pub(super) len: usize,
    hole: bool,
}

impl SearchHeap {
    pub(super) fn bytes(&self) -> usize {
        self.groups.capacity() * std::mem::size_of::<KeyGroup>()
    }

    pub(super) fn clear(&mut self) {
        self.groups.clear();
        self.len = 0;
        self.hole = false;
    }

    #[inline(always)]
    fn slot(&mut self, q: usize) -> &mut u64 {
        &mut self.groups[q >> 2].0[q & 3]
    }

    /// The key in slot `q` (the tests).
    #[cfg(test)]
    pub(super) fn key_at(&self, q: usize) -> u64 {
        self.groups[q >> 2].0[q & 3]
    }

    /// Puts `key` into slot `q` and records the slot in its state's node.
    #[inline(always)]
    fn place(&mut self, q: usize, key: u64, nodes: &mut [SearchNode]) {
        *self.slot(q) = key;
        nodes[(key & u32::MAX as u64) as usize].slot = q as u32;
    }

    /// Queues a state not queued (nor expanded) in this search.
    #[inline(always)]
    pub(super) fn push(&mut self, key: u64, nodes: &mut [SearchNode]) {
        if self.hole {
            self.hole = false;
            self.len += 1;
            self.sift_down(key, 3 + self.len, nodes);
            return;
        }
        let q = 3 + self.len;
        if self.groups.len() <= q >> 2 {
            self.groups.push(KeyGroup([NO_KEY; 4]));
        }
        self.len += 1;
        self.sift_up(q, key, nodes);
    }

    /// Lowers the key of a queued state to `key`, which is at most its
    /// key now.
    #[inline(always)]
    pub(super) fn decrease(&mut self, key: u64, nodes: &mut [SearchNode]) {
        let q = nodes[(key & u32::MAX as u64) as usize].slot as usize;
        debug_assert!(*self.slot(q) & u32::MAX as u64 == key & u32::MAX as u64);
        debug_assert!(*self.slot(q) >= key);
        self.sift_up(q, key, nodes);
    }

    /// Moves `key` up from slot `q` (which it takes over) to its place;
    /// while the root is empty, no higher than the root's children (the
    /// next push or pop fills the root from them).
    #[inline(always)]
    fn sift_up(&mut self, mut q: usize, key: u64, nodes: &mut [SearchNode]) {
        let floor = if self.hole { 7 } else { 3 };
        while q > floor {
            let parent = (q >> 2) + 2;
            let above = *self.slot(parent);
            if above <= key {
                break;
            }
            self.place(q, above, nodes);
            q = parent;
        }
        self.place(q, key, nodes);
    }

    /// The least key; its node's slot is left for the caller (which marks
    /// the node closed).
    #[inline(always)]
    pub(super) fn pop(&mut self, nodes: &mut [SearchNode]) -> Option<u64> {
        if self.hole {
            self.hole = false;
            if self.len == 0 {
                return None;
            }
            let end = 3 + self.len;
            let last = std::mem::replace(self.slot(end), NO_KEY);
            self.sift_down(last, end, nodes);
        }
        if self.len == 0 {
            return None;
        }
        let top = *self.slot(3);
        self.len -= 1;
        self.hole = true;
        Some(top)
    }

    /// The key the next pop takes unless a lesser one is pushed first
    /// (`NO_KEY` when the heap is empty): the root, or the least of its
    /// children while it is empty. A hint for prefetching what the next
    /// expansion reads.
    #[inline(always)]
    pub(super) fn next_hint(&self) -> u64 {
        if !self.hole {
            return self.groups.first().map_or(NO_KEY, |group| group.0[3]);
        }
        let Some(&KeyGroup([a, b, c, d])) = self.groups.get(1) else {
            return NO_KEY;
        };
        a.min(b).min(c.min(d))
    }

    /// As `KeyHeap::sift_down`, recording the slots.
    #[inline(always)]
    fn sift_down(&mut self, key: u64, end: usize, nodes: &mut [SearchNode]) {
        let mut q = 3;
        loop {
            if 4 * q - 8 >= end {
                break;
            }
            let grandchildren = 4 * q - 10;
            for group in [grandchildren, grandchildren + 2, grandchildren + 3] {
                super::prefetch_index(&self.groups, group);
            }
            let [a, b, c, d] = self.groups[q - 2].0;
            let low = if b < a { 1 } else { 0 };
            let high = if d < c { 3 } else { 2 };
            let (first, second) = (a.min(b), c.min(d));
            let (least, which) = if second < first {
                (second, high)
            } else {
                (first, low)
            };
            if least >= key {
                break;
            }
            self.place(q, least, nodes);
            q = 4 * q - 8 + which;
        }
        self.place(q, key, nodes);
    }
}

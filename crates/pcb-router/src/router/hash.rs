// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Fast hashers for the search's maps.

use super::*;

/// Hashes a lattice state index by one multiplication, folded so that the
/// low bits (the table's bucket) depend on all of them.
#[derive(Clone, Copy, Default)]
pub(super) struct StateHasher(u64);

impl std::hash::Hasher for StateHasher {
    fn finish(&self) -> u64 {
        self.0 ^ (self.0 >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0.rotate_left(8) ^ *byte as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.0 = (value as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

/// A multiply-add word hasher (rustc's FxHash, second version) for hot maps
/// of small integer keys, where SipHash cost more than the lookups: only
/// for maps whose iteration order does not matter (SipHash's order was
/// random per run, so no result may follow it anyway).
#[derive(Clone, Copy, Default)]
pub(crate) struct FastHasher(u64);

impl FastHasher {
    #[inline]
    pub(super) fn add(&mut self, word: u64) {
        self.0 = self
            .0
            .wrapping_add(word)
            .wrapping_mul(0xf135_7aea_2e62_a9c5);
    }
}

impl std::hash::Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0.rotate_left(26)
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
        self.add(bytes.len() as u64);
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(value as u64);
    }

    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.add(value as u64);
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(value as u64);
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    #[inline]
    fn write_i64(&mut self, value: i64) {
        self.add(value as u64);
    }
}

/// A hash map on `FastHasher`.
pub(crate) type FastMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<FastHasher>>;

/// Lattice states (`Router::state`) of a few nodes, with a terminal each.
pub(super) type StateMap = HashMap<u32, u16, std::hash::BuildHasherDefault<StateHasher>>;

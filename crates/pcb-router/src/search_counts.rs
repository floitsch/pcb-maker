// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Where the lattice searches went: the counters (`CorridorTotals`) are
//! kept for the process, every router and thread: the searches inside a
//! corridor, a window and the whole board, the negotiation's and the hard
//! searches' apart, and the board-wide fallbacks refused after two had
//! failed. The verdict reports them (`corridors`). Until 2026-11-04 they
//! also counted the capacitated corridors' learning (`capacitated_corridors`,
//! off, removed; docs/handover.md, "Removed experiments (2026-11-04)").

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// The kinds of lattice search the counters tell apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchKind {
    /// Inside a corridor (the tile planner's or the global route's).
    Corridor = 0,
    /// The plain window.
    Window = 1,
    /// The whole board.
    Board = 2,
}

const KINDS: usize = 3;

/// Per mode (negotiation, hard) and kind: searches, found, expansions.
static SEARCHES: [[[AtomicU64; 3]; KINDS]; 2] =
    [const { [const { [const { AtomicU64::new(0) }; 3] }; KINDS] }; 2];

/// Board-wide fallbacks refused because two had failed already
/// (`hopeless_searches`).
static HOPELESS_SKIPS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn note_search(hard: bool, kind: SearchKind, found: bool, expansions: u64) {
    let slot = &SEARCHES[hard as usize][kind as usize];
    slot[0].fetch_add(1, Relaxed);
    slot[1].fetch_add(found as u64, Relaxed);
    slot[2].fetch_add(expansions, Relaxed);
}

pub(crate) fn note_hopeless_skip() {
    HOPELESS_SKIPS.fetch_add(1, Relaxed);
}

/// One kind of search, summed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SearchTotals {
    pub searches: u64,
    pub found: u64,
    pub expansions: u64,
}

/// The process's corridor counters (`corridor_totals`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CorridorTotals {
    /// Per kind (`SearchKind` order), the negotiation's searches
    /// (`soft`) and the hard searches' (the repair, the trouble repair).
    pub soft: [SearchTotals; KINDS],
    pub hard: [SearchTotals; KINDS],
    /// Board-wide fallbacks refused after two board-wide failures.
    pub hopeless_skips: u64,
}

/// This process's corridor counters so far.
pub fn corridor_totals() -> CorridorTotals {
    let read = |mode: usize| {
        let mut totals = [SearchTotals::default(); KINDS];
        for (kind, slot) in totals.iter_mut().enumerate() {
            let counters = &SEARCHES[mode][kind];
            *slot = SearchTotals {
                searches: counters[0].load(Relaxed),
                found: counters[1].load(Relaxed),
                expansions: counters[2].load(Relaxed),
            };
        }
        totals
    };
    CorridorTotals {
        soft: read(0),
        hard: read(1),
        hopeless_skips: HOPELESS_SKIPS.load(Relaxed),
    }
}

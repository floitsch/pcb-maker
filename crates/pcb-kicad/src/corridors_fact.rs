// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The verdict's `corridors`: where the run's lattice searches went
//! (corridors, windows, the board; the negotiation's and the hard
//! searches' apart). Summed over every router of the process, the
//! ladder's attempts and probes alike. Until 2026-11-04 also what the
//! capacitated corridors' learning did (`capacitated_corridors`, off,
//! removed; docs/handover.md, "Removed experiments (2026-11-04)").

use pcb_router::search_counts::{CorridorTotals, SearchTotals, corridor_totals};
use serde::{Deserialize, Serialize};

/// One kind of search: how many, how many found a path, their work.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct KiCadSearchCount {
    pub searches: u64,
    pub found: u64,
    pub work_seconds: f64,
}

/// The searches of one mode by kind.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct KiCadSearchKinds {
    /// Inside a corridor (the tile planner's or the global route's).
    pub corridor: KiCadSearchCount,
    /// The plain window.
    pub window: KiCadSearchCount,
    /// The whole board.
    pub board: KiCadSearchCount,
}

/// The verdict's `corridors`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct KiCadCorridors {
    /// The negotiation's searches (the via reduction's too).
    pub negotiation: KiCadSearchKinds,
    /// The hard searches (the repair, the trouble repair, the spoke
    /// repairs).
    pub hard: KiCadSearchKinds,
    /// Window and board searches refused after two board-wide failures.
    pub hopeless_skips: u64,
}

impl KiCadCorridors {
    /// The process's counters so far.
    pub fn now() -> Self {
        Self::of(
            &corridor_totals(),
            pcb_router::router::expansions_per_second(),
        )
    }

    /// `totals` with expansions at `rate` a second.
    pub fn of(totals: &CorridorTotals, rate: f64) -> Self {
        let seconds = |expansions: u64| expansions as f64 / rate;
        let count = |total: &SearchTotals| KiCadSearchCount {
            searches: total.searches,
            found: total.found,
            work_seconds: seconds(total.expansions),
        };
        let kinds = |totals: &[SearchTotals; 3]| KiCadSearchKinds {
            corridor: count(&totals[0]),
            window: count(&totals[1]),
            board: count(&totals[2]),
        };
        Self {
            negotiation: kinds(&totals.soft),
            hard: kinds(&totals.hard),
            hopeless_skips: totals.hopeless_skips,
        }
    }
}

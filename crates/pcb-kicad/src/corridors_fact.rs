// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The verdict's `corridors`: where the run's lattice searches went
//! (corridors, the capacitated corridors' whole band, windows, the board;
//! the negotiation's and the hard searches' apart) and what the
//! capacitated corridors' learning did (`capacitated_corridors`;
//! docs/router.md, "Capacitated corridors (2026-10-24)"). Summed over
//! every router of the process, the ladder's attempts and probes alike.

use pcb_router::capacitated::{CorridorTotals, Learn, SearchTotals, corridor_totals};
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
    /// Inside a corridor: the tile planner's, or (switch on) the planner's
    /// inside the capacitated corridor.
    pub corridor: KiCadSearchCount,
    /// Inside the whole capacitated corridor (switch on).
    pub band: KiCadSearchCount,
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
    /// Capacitated plans built (one per router with the switch on) and
    /// their work.
    pub plans_built: u64,
    pub plan_work_seconds: f64,
    /// Searches that found nothing inside their capacitated corridor.
    pub misses: u64,
    /// Boundaries and via sites a miss lowered by a track.
    pub reductions: u64,
    /// Boundaries lowered for nets kept in conflict.
    pub conflict_reductions: u64,
    /// Boundaries a miss named that the tracks routed across kept up.
    pub floor_held: u64,
    /// Misses with no boundary of the route between reached and
    /// unreached tiles (the failure inside a tile).
    pub no_boundary: u64,
    /// Global re-plans, the nets they routed again, their work.
    pub replans: u64,
    pub replanned_nets: u64,
    pub replan_work_seconds: f64,
    /// Nets routed again in a new corridor after a re-plan.
    pub retries: u64,
    /// Searches that went to the window and the board: no global path
    /// under the learned capacities; nothing learned from the miss.
    pub fallbacks_no_path: u64,
    pub fallbacks_nothing_learned: u64,
    /// Nets still missing when their batch's retries ran out.
    pub deferred: u64,
    /// The work of counting the tracks across the boundaries (floors).
    pub floor_work_seconds: f64,
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
        let kinds = |totals: &[SearchTotals; 4]| KiCadSearchKinds {
            corridor: count(&totals[0]),
            band: count(&totals[1]),
            window: count(&totals[2]),
            board: count(&totals[3]),
        };
        Self {
            negotiation: kinds(&totals.soft),
            hard: kinds(&totals.hard),
            plans_built: totals.get(Learn::PlansBuilt),
            plan_work_seconds: seconds(totals.get(Learn::PlanExpansions)),
            misses: totals.get(Learn::Misses),
            reductions: totals.get(Learn::Reductions),
            conflict_reductions: totals.get(Learn::ConflictReductions),
            floor_held: totals.get(Learn::FloorHeld),
            no_boundary: totals.get(Learn::NoBoundary),
            replans: totals.get(Learn::Replans),
            replanned_nets: totals.get(Learn::ReplannedNets),
            replan_work_seconds: seconds(totals.get(Learn::ReplanExpansions)),
            retries: totals.get(Learn::Retries),
            fallbacks_no_path: totals.get(Learn::FallbackNoPath),
            fallbacks_nothing_learned: totals.get(Learn::FallbackNothingLearned),
            deferred: totals.get(Learn::Deferred),
            floor_work_seconds: seconds(totals.get(Learn::FloorExpansions)),
            hopeless_skips: totals.get(Learn::HopelessSkips),
        }
    }
}

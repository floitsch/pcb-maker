// Copyright (C) 2026 Toit contributors.

//! Whole-board routing through the in-memory `pcb-router` core.
//!
//! The board is parsed once, lowered once, routed entirely in memory, checked
//! by the core's exact verifier, and written once. Native KiCad verification
//! is a single final gate, never part of the routing loop.

use super::*;

mod attempt;
mod budget;
mod config;
mod diff_pairs;
mod geometry;
mod kept;
mod ladder;
mod lower;
mod memory;
mod pour_plan;
mod pour_quality;
mod result;
mod route;
#[cfg(test)]
mod tests;
mod tools;

pub(crate) use self::diff_pairs::{measure_uncoupled, route_diff_pairs};
pub(crate) use self::diff_pairs::{with_pairs, work_budget as pairs_work_budget};
pub(crate) use self::kept::{count_tracks, keep_existing_copper, strip_tracks};
pub(crate) use self::{attempt::*, budget::*, geometry::*, ladder::*, lower::*, memory::*};
pub use self::{
    config::*,
    diff_pairs::{KiCadDiffPair, KiCadDiffPairs},
    kept::KiCadKeptCopper,
    pour_plan::*,
    pour_quality::*,
    result::*,
    route::*,
    tools::*,
};

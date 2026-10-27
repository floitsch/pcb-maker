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
mod lengths;
mod lower;
mod meander;
mod memory;
mod pour_plan;
mod pour_quality;
mod result;
mod route;
#[cfg(test)]
mod tests;
mod tools;

pub(crate) use self::diff_pairs::{
    finish_pairs, measure_uncoupled, plan_pairs, with_pairs, write_pair_copper,
};
pub(crate) use self::kept::{count_tracks, keep_existing_copper, strip_tracks};
pub(crate) use self::lengths::{
    finish_lengths, match_lengths, with_lengths, write_length_patterns,
};
pub(crate) use self::meander::copper_heights;
pub(crate) use self::{attempt::*, budget::*, geometry::*, ladder::*, lower::*, memory::*};
pub use self::{
    config::*,
    diff_pairs::{KiCadDiffPair, KiCadDiffPairs},
    kept::KiCadKeptCopper,
    lengths::KiCadNetLength,
    pour_plan::*,
    pour_quality::*,
    result::*,
    route::*,
    tools::*,
};

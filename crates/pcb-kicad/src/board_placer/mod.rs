// Copyright (C) 2026 Toit contributors.

//! Whole-board component placement through the `pcb-placer` core.

use super::*;
use crate::placement_constraints::{
    HeldParts, KEEPOUT_NAME, KiCadConstraintStatus, KiCadConstraintsSource, apply_constraints,
    apply_keepouts, apply_outline, apply_sides, constraint_report, held_parts, resolve_constraints,
};

mod bodies;
mod config;
mod courtyards;
mod decoupling;
mod edges;
mod guidance;
mod legality;
mod lower;
mod marks;
mod place;
mod result;
mod rules;
mod seed_race;
#[cfg(test)]
mod tests;
mod trace;

pub(crate) use self::guidance::{attribution, reference_index};
pub(crate) use self::{bodies::*, lower::*, marks::*, rules::*, seed_race::*};
pub use self::{config::*, guidance::KiCadGlobalGuidance, legality::*, place::*, result::*};
use self::{courtyards::*, decoupling::*, edges::*, trace::*};

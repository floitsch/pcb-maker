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
mod exits;
mod guidance;
mod legality;
mod lower;
mod marks;
mod pair_rows;
mod panel;
mod place;
mod result;
mod rules;
mod seed_race;
#[cfg(test)]
mod tests;
mod trace;

#[cfg(test)]
pub(crate) use self::exits::closed_pad_reason;
pub(crate) use self::exits::pad_exits_settings;
pub(crate) use self::guidance::{attribution, reference_index};
#[cfg(test)]
pub(crate) use self::place::room_hint;
pub(crate) use self::{bodies::*, lower::*, marks::*, pair_rows::*, rules::*, seed_race::*};
pub use self::{
    config::*,
    exits::{
        KiCadClosedPad, KiCadExitMove, KiCadPadExits, KiCadPadExitsCensus, KiCadPadExitsReport,
        pad_exits_kicad_board, pad_exits_override,
    },
    guidance::KiCadGlobalGuidance,
    legality::*,
    panel::{KiCadPanelCellClass, KiCadPanelCopiesReport, KiCadPanelCopy},
    place::*,
    result::*,
};
use self::{courtyards::*, decoupling::*, edges::*, trace::*};

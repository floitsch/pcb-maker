// Copyright (C) 2026 Toit contributors.

//! Whole-board component placement through the `pcb-placer` core.

use super::*;
use crate::placement_constraints::{
    HeldParts, KEEPOUT_NAME, KiCadConstraintStatus, KiCadConstraintsSource, apply_constraints,
    apply_keepouts, apply_outline, apply_sides, constraint_report, held_parts, resolve_constraints,
};

mod config;
mod courtyards;
mod decoupling;
mod edges;
mod exits;
mod guidance;
mod lower;
mod marks;
mod pair_rows;
mod panel;
mod place;
mod result;
mod rules;
mod seed_race;
mod skeleton;
#[cfg(test)]
mod tests;
mod trace;

pub use self::courtyards::courtyard_slabs;
pub use self::exits::closed_pad_reason;
pub use self::exits::pad_exits_settings;
pub use self::guidance::{attribution, reference_index};
pub use self::place::room_hint;
pub use self::{
    config::*,
    exits::{
        KiCadClosedPad, KiCadExitMove, KiCadPadExits, KiCadPadExitsCensus, KiCadPadExitsReport,
        pad_exits_kicad_board, pad_exits_override,
    },
    guidance::KiCadGlobalGuidance,
    panel::{KiCadPanelCellClass, KiCadPanelCopiesReport, KiCadPanelCopy},
    place::*,
    result::*,
    skeleton::{KiCadSkeletonRace, KiCadSkeletonRaceReport},
};
use self::{courtyards::*, decoupling::*, edges::*, skeleton::passive_mask, trace::*};
pub use self::{lower::*, pair_rows::*, rules::*};
pub(crate) use self::{marks::*, seed_race::*};
use crate::board_router::largest_clearance;
use crate::footprint_bodies::*;

// Copyright (C) 2026 Toit contributors.

//! A KiCad board placed: everything between the board file and
//! pcb-placer's problem, both ways.
//!
//! - `board_placer`: the lowering to a `pcb_placer::Problem` (on the
//!   router side's lowering: the layer table, the router config, pours,
//!   BGA array rooms, length rules), `place_kicad_board` and its seed race,
//!   panels of copies, the skeleton race, decoupling, edges, pair rows,
//!   board marks, pad exits and the global route the escape demand
//!   measures with.
//! - `placement_constraints` (constraints.json), `placement_room`,
//!   `sub_boards` (the placer's half of a panel of different boards),
//!   `modules` (the breakaway modules of one board).
//! - `labels` and `silk_fields`: texts moved off copper and silkscreen
//!   fields placed, after a placement.
//!
//! The footprint bodies the placer and the router side both read are
//! pcb-kicad-file's `footprint_bodies`. The layout flow, the verdict and
//! the reports sit above, in pcb-kicad.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
// The tests' scratch directories.
#[cfg(test)]
use std::env;

// The KiCad files, their geometry and how a run exits (pcb-kicad-file).
use pcb_kicad_file::{
    canonical_order, exit, flip, footprint_bodies, geometry, kicad_nets, kicad_sexpr,
    local_clearance, outline, outline_pieces, part_motion,
};
use pcb_kicad_file::{copper_geometry::*, fs_util::*, geometry::*, kicad_nets::*, kicad_sexpr::*};
// A board routed (pcb-kicad-route).
use pcb_kicad_route::{KiCadBoardRouterConfig, board_router, net_classes, project_rules, quality};
// What the tests read from below.
#[cfg(test)]
use pcb_kicad_check::native_report;
#[cfg(test)]
use pcb_kicad_file::connection_rules::KiCadConnectionRoutingRules;
#[cfg(test)]
use pcb_kicad_route::{KiCadViaInPad, KiCadViaInPadEntry};

pub mod board_placer;
pub mod labels;
pub mod modules;
pub mod placement_constraints;
pub mod placement_room;
pub mod silk_fields;
pub mod sub_boards;

pub use board_placer::{
    KiCadBoardPlacerConfig, KiCadBoardPlacerResult, KiCadClosedPad, KiCadExitMove, KiCadPadExits,
    KiCadPadExitsCensus, KiCadPadExitsReport, KiCadPanelCellClass, KiCadPanelCopiesReport,
    KiCadPanelCopy, KiCadPlacedFootprint, KiCadPlacerTuning, pad_exits_kicad_board,
    pad_exits_override, place_kicad_board,
};
pub use labels::KiCadLabelReport;
pub use modules::describe_modules;
pub use placement_constraints::{
    KiCadConstraintStatus, KiCadConstraintsSource, KiCadPlacementConstraints, keepout_warnings,
    overlap_warnings, placement_input_warnings, read_placement_constraints,
};
pub use placement_room::{KiCadRoom, TARGET_UTILIZATION, noted_room};
pub use silk_fields::KiCadSilkscreenReport;
pub use sub_boards::describe_sub_boards;

// The fixture test of the placement on tests/fixtures/f1 that needs the
// crate's insides.
#[cfg(test)]
#[path = "tests/fixtures.rs"]
mod fixture_tests;

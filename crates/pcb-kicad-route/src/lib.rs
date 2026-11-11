// Copyright (C) 2026 Toit contributors.

//! A KiCad board routed: everything between the board file and
//! pcb-router's problem, both ways.
//!
//! - The project's rules (`project_rules`, `.kicad_pro` and `.kicad_dru`
//!   lowered to router rules; `net_classes`).
//! - `board_router`: the lowering to a `pcb_router::Board`, the attempt
//!   ladder of `route_kicad_board`, differential pairs, lengths and
//!   meanders, BGA fan-outs, vias in pads and via spans, pours, writing
//!   the copper back and the router's result; `necks` (connection widths
//!   and their repair), `pin_swap`, `check_work` (what a KiCad check is
//!   charged).
//! - `quality` with `aesthetics` and `cost` (`score-kicad-board`): the
//!   lowering reads its net-name heuristics and its census of vias in
//!   pads.
//!
//! The placer side (pcb-kicad-place) builds on this crate's lowering; the
//! layout flow, the verdict and the reports sit above, in pcb-kicad.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    time::Instant,
};

// The KiCad files, their geometry and how a run exits (pcb-kicad-file).
use pcb_kicad_file::{
    board_stats, canonical_order, exit, footprint_bodies, geometry, kicad_nets, kicad_sexpr,
    local_clearance, outline, outline_pieces,
};
use pcb_kicad_file::{
    board_stats::*, copper_emit::*, copper_geometry::*, fs_util::*, geometry::*, kicad_nets::*,
    kicad_sexpr::*,
};
// KiCad's checks and the machine (pcb-kicad-check).
use pcb_kicad_check::native_report::remove_if_present;
use pcb_kicad_check::{ledger, native_report, proc_memory, verification, work_rate};

pub mod aesthetics;
pub mod board_router;
pub mod check_work;
pub mod cost;
pub mod necks;
pub mod net_classes;
pub mod pin_swap;
pub mod project_rules;
pub mod quality;

// What the route flow reads from below without a path.
use pcb_kicad_check::drc_cache::drc_cache_counts;
use pcb_kicad_check::verification::{VerificationReport, verify_materialized_rung};
use pcb_kicad_file::connection_rules::KiCadConnectionRoutingRules;
use pcb_router::mem_profile;

pub use board_router::{
    KiCadBoardRouterConfig, KiCadBoardRouterNet, KiCadBoardRouterResult, KiCadBoardRouterViolation,
    KiCadClosedPads, KiCadDiffPair, KiCadDiffPairRule, KiCadDiffPairs, KiCadExistingTracks,
    KiCadKeepWhole, KiCadKeptCopper, KiCadNetLength, KiCadPourMode, KiCadPourQuality,
    KiCadViaInPad, KiCadViaInPadEntry, KiCadViaInPadFootprint, KiCadViaInPadReport,
    KiCadViaSpanEntry, admit_length_rule_layers, admit_reference_via_spans,
    admit_reference_vias_in_pads, blind_vias_override, dump_kicad_pour_map,
    keep_whole_input_warnings, measure_pours, route_kicad_board, write_kicad_board_without_tracks,
    write_kicad_board_without_tracks_except,
};
pub use cost::{KiCadCost, KiCadFabQuote};
pub use necks::{NeckRepair, repair_kicad_necks};
pub use net_classes::KiCadNetClassRequest;
pub use pin_swap::{
    KiCadPinChange, KiCadPinSwapConfig, KiCadPinSwapEstimate, KiCadPinSwapResult, KiCadPinSwapSpec,
    read_pin_swap_spec, swap_kicad_pins, swap_kicad_pins_in_place,
};
pub use project_rules::{
    KiCadLayerGeometryRule, KiCadProjectMinimums, KiCadRuleOverride, preflight_overrides,
    resolve_project_rules, router_config_over_project,
};
pub use quality::{KiCadDecoupling, KiCadQualityReport, score_kicad_board};

/// Sets the allocator up for the router's large maps (glibc: blocks of a
/// MiB or more are unmapped when freed; `board_router::configure_malloc`).
/// Call once, first thing in `main`.
pub fn configure_allocator() {
    board_router::configure_malloc();
}

// The fixture tests on tests/fixtures/f1 that need the crate's insides
// (under tests/, as test sources: tools/config-reference's index of
// environment variables leaves out their PCB_UPDATE_GOLDEN).
#[cfg(test)]
#[path = "tests/fixtures.rs"]
mod fixture_tests;

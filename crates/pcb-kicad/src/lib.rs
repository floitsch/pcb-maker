// Copyright (C) 2026 Toit contributors.

//! The top of the KiCad side: `layout-kicad-board` and what judges and
//! reports a run. `board_layout` lays a board out (outline sizing, the
//! placement, the race of placements, the moves, the final ladder, the
//! report; `congestion_rank` pre-ranks the race's candidates) and
//! `free_dimension` tries several sizes of one edge; `verdict` judges a
//! run (`judge`, `route_facts`, the next actions); `describe`,
//! `class_view`, `run_diagnosis` (`inspect-routing-run`), `silkscreen`,
//! `fab_export` and `fab_release` are the other agent commands' reports
//! and outputs.
//!
//! The crates below, each depending only on those below it:
//! pcb-kicad-file (KiCad's files as data), pcb-kicad-check (KiCad's
//! checks, the memory ledger, the work rate), pcb-kicad-route (a board
//! routed) and pcb-kicad-place (a board placed, on the router side's
//! lowering). This crate re-exports what the binary uses
//! of them, so that `src/` reads one API; the netlist import
//! (pcb-kicad-import) and the measurement tools (pcb-kicad-lab, which
//! uses this crate) are named directly. docs/architecture.md has the map.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};
// The tests' scratch directories.
#[cfg(test)]
use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
};

// KiCad's files as data and how a run exits (pcb-kicad-file).
use board_stats::*;
pub use board_stats::{
    KiCadBoardStatistics, KiCadConnectionSummary, KiCadCopperLayerStatistics,
    KiCadPhysicalCopperStatistics, inspect_kicad_board, inspect_kicad_routable_connections,
};
pub use connection_rules::KiCadConnectionRoutingRules;
pub use copper_emit::{
    KiCadCopperStripReport, KiCadRouteBranch, KiCadRoutePoint, SupplementalSegment,
    SupplementalVia, write_kicad_board_without_copper,
};
pub use corridors_fact::{KiCadCorridors, KiCadSearchCount, KiCadSearchKinds};
pub use exit::{
    EXIT_COMPLETE, EXIT_CUT, EXIT_INCOMPLETE, EXIT_INTERNAL, EXIT_USAGE, input_error,
    is_input_error, noted_input_actions,
};
pub use fs_util::check_board_id;
pub use fs_util::write_atomically;
use fs_util::*;
use geometry::*;
use kicad_nets::*;
use kicad_sexpr::*;
use pcb_kicad_file::{
    board_stats, canonical_order, connection_rules, copper_emit, corridors_fact, exit,
    footprint_bodies, fs_util, geometry, kicad_nets, kicad_sexpr, outline, outline_pieces,
};

// KiCad's checks and the machine (pcb-kicad-check).
pub use drc_cache::{DrcCacheCounts, drc_cache_counts};
pub use native_report::{
    KICAD_REQUIREMENT, check_kicad_cli, erc_design_issues, is_erc_library_warning,
    is_library_metadata_warning,
};
use native_report::{remove_if_present, run_kicad_report};
pub use pcb_kicad_check::ledger;
use pcb_kicad_check::{drc_cache, native_report, proc_memory, verification, work_rate};
pub use verification::{
    KICAD_VERIFICATION_CACHE_ENV, KICAD_VERIFICATION_CACHE_EVENTS_ENV,
    KiCadVerificationCacheStatus, KiCadVerificationCacheTrace, VerificationReport,
    kicad_verifier_implementation_sha256, verify_materialized_rung,
};
pub use work_rate::{
    KiCadMeasuredRate, KiCadWorkRate, WORK_RATE_VARIABLE, calibrate as calibrate_work_rate,
    calibrate_reference_by_default, measure as measure_work_rate, parse_work_rate, work_rate,
};

// A board routed (pcb-kicad-route).
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
pub use pcb_kicad_route::configure_allocator;
use pcb_kicad_route::{
    board_router, check_work, cost, necks, net_classes, pin_swap, project_rules, quality,
};
/// The memory profile (`PCB_ROUTER_MEM_PROFILE`): the counting allocator,
/// the sampler, the phases (docs/profiling.md, "Memory").
pub use pcb_router::mem_profile;
pub use pin_swap::{
    KiCadPinChange, KiCadPinSwapConfig, KiCadPinSwapEstimate, KiCadPinSwapResult, KiCadPinSwapSpec,
    read_pin_swap_spec, swap_kicad_pins, swap_kicad_pins_in_place,
};
pub use project_rules::{
    KiCadLayerGeometryRule, KiCadProjectMinimums, KiCadRuleOverride, preflight_overrides,
    resolve_project_rules, router_config_over_project,
};
pub use quality::{KiCadDecoupling, KiCadQualityReport, score_kicad_board};

// A board placed (pcb-kicad-place).
pub use board_placer::{
    KiCadBoardPlacerConfig, KiCadBoardPlacerResult, KiCadClosedPad, KiCadExitMove, KiCadPadExits,
    KiCadPadExitsCensus, KiCadPadExitsReport, KiCadPanelCellClass, KiCadPanelCopiesReport,
    KiCadPanelCopy, KiCadPlacedFootprint, pad_exits_kicad_board, pad_exits_override,
    place_kicad_board,
};
pub use labels::KiCadLabelReport;
use pcb_kicad_place::{
    board_placer, labels, placement_constraints, placement_room, silk_fields, sub_boards,
};
pub use placement_constraints::{
    KiCadConstraintStatus, KiCadConstraintsSource, KiCadPlacementConstraints, keepout_warnings,
    overlap_warnings, placement_input_warnings, read_placement_constraints,
};
pub use placement_room::{KiCadRoom, TARGET_UTILIZATION, noted_room};
pub use silk_fields::KiCadSilkscreenReport;
pub use sub_boards::describe_sub_boards;

// This crate: the layout flow, the verdict and the reports.
pub mod board_layout;
mod class_view;
pub mod congestion_rank;
mod describe;
mod fab_export;
mod fab_release;
pub mod free_dimension;
mod run_diagnosis;
mod silkscreen;
mod verdict;
pub use board_layout::{
    KiCadBoardLayoutConfig, KiCadBoardLayoutMove, KiCadBoardLayoutResult, KiCadOutlineTrial,
    KiCadRaceProbe, KiCadRaceProbeScore, KiCadRaceReport, KiCadSilkscreenConfig, KiCadSwappable,
    layout_kicad_board, placement_check_config,
};
pub use class_view::{
    KiCadClassLayer, KiCadClassLayers, KiCadClassRow, KiCadControlledImpedance, KiCadDesignerWidth,
    note_designer_class_layers, note_designer_widths,
};
pub use describe::{KiCadBoardDescription, KiCadSwapCandidate, describe_kicad_board};
pub use fab_export::{KiCadFabExport, export_kicad_fab};
pub use fab_release::{
    KiCadFabDrc, KiCadFabErc, KiCadFabFile, KiCadFabOptions, KiCadFabProfile, KiCadFabRelease,
    KiCadFabVerdictLines, fab_kicad_board, fab_verdict_lines,
};
pub use free_dimension::{
    KiCadFreeDimension, KiCadFreeDimensionPlan, KiCadFreeDimensionRun, KiCadFreeDimensionSearch,
    KiCadFreeDimensionStep,
};
pub use run_diagnosis::{
    KiCadRunDiagnosis, KiCadRunFiles, KiCadRunInspection, inspect_routing_run, run_files,
};
pub use silkscreen::repair_kicad_silkscreen;
pub use verdict::{
    Apart, DrcFinding, DrcTally, INPUT_KINDS, KiCadInputItems, KiCadReference, KiCadRunFacts,
    KiCadRunStatus, KiCadVerdict, board_items, custom_rules_line, decide, drc_finding,
    error_verdict, footprint_findings_note, footprint_own_unreferenced, input_findings_note,
    input_items, input_own, is_cosmetic, judge, kept_parts_actions, kicad_drc_report,
    load_drc_tally, read_drc_tally, reference_findings, room_action, route_facts,
    set_build_version, split_findings, tally_drc, unreferenced_apart, wall_cuts,
    with_input_observations,
};

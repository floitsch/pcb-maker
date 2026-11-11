// Copyright (C) 2026 Toit contributors.

//! The KiCad side of pcb-maker: boards and projects read and written as
//! KiCad's S-expressions (kicad_sexpr, kicad_nets, copper_emit), their
//! geometry (geometry, copper_geometry, outline, roundrect_pad), the
//! adapters that lower a board to the placer and the router and write the
//! result back (board_placer, board_router, board_layout), KiCad's own
//! checks (verification, native_report, drc_cache, verdict) and the agent
//! commands' other tools (netlist import, fab export, pin swaps, ...).
//! `lab` holds the measurement tools the research commands run.

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

pub use connection_rules::KiCadConnectionRoutingRules;
pub use corridors_fact::{KiCadCorridors, KiCadSearchCount, KiCadSearchKinds};
// The KiCad files, their geometry and how a run exits (pcb-kicad-file).
pub use necks::{NeckRepair, repair_kicad_necks};
use pcb_kicad_file::{
    board_stats, canonical_order, connection_rules, copper_emit, copper_geometry, corridors_fact,
    exit, flip, fs_util, geometry, kicad_nets, kicad_sexpr, outline, outline_pieces,
};
// KiCad's checks and the machine (pcb-kicad-check).
pub use drc_cache::{DrcCacheCounts, drc_cache_counts};
pub use fs_util::write_atomically;
pub use pcb_kicad_check::ledger;
use pcb_kicad_check::{drc_cache, native_report, proc_memory, verification, work_rate};
mod silkscreen;
pub use native_report::{
    KICAD_REQUIREMENT, check_kicad_cli, erc_design_issues, is_erc_library_warning,
    is_library_metadata_warning,
};
use native_report::{remove_if_present, run_kicad_report};
pub use silkscreen::repair_kicad_silkscreen;
// A board routed and placed (pcb-kicad-route-place).
use pcb_kicad_route_place::{
    board_placer, board_router, check_work, cost, labels, necks, net_classes, pin_swap,
    placement_constraints, placement_room, project_rules, quality, silk_fields, sub_boards,
};
mod board_layout;
mod congestion_rank;
pub mod lab;
pub use board_layout::{
    KiCadBoardLayoutConfig, KiCadBoardLayoutMove, KiCadBoardLayoutResult, KiCadOutlineTrial,
    KiCadRaceProbe, KiCadRaceProbeScore, KiCadRaceReport, KiCadSilkscreenConfig, KiCadSwappable,
    layout_kicad_board, placement_check_config,
};
pub use board_placer::{
    KiCadBoardPlacerConfig, KiCadBoardPlacerResult, KiCadClosedPad, KiCadExitMove, KiCadPadExits,
    KiCadPadExitsCensus, KiCadPadExitsReport, KiCadPanelCellClass, KiCadPanelCopiesReport,
    KiCadPanelCopy, KiCadPlacedFootprint, pad_exits_kicad_board, pad_exits_override,
    place_kicad_board,
};
mod verdict;
pub use exit::{
    EXIT_COMPLETE, EXIT_CUT, EXIT_INCOMPLETE, EXIT_INTERNAL, EXIT_USAGE, input_error,
    is_input_error, noted_input_actions,
};
pub use verdict::{
    Apart, DrcFinding, DrcTally, INPUT_KINDS, KiCadInputItems, KiCadReference, KiCadRunFacts,
    KiCadRunStatus, KiCadVerdict, board_items, custom_rules_line, decide, drc_finding,
    error_verdict, footprint_findings_note, footprint_own_unreferenced, input_findings_note,
    input_items, input_own, is_cosmetic, judge, kept_parts_actions, kicad_drc_report,
    load_drc_tally, read_drc_tally, reference_findings, room_action, route_facts,
    set_build_version, split_findings, tally_drc, unreferenced_apart, wall_cuts,
    with_input_observations,
};
pub use work_rate::{
    KiCadMeasuredRate, KiCadWorkRate, WORK_RATE_VARIABLE, calibrate as calibrate_work_rate,
    calibrate_reference_by_default, measure as measure_work_rate, parse_work_rate, work_rate,
};
mod class_view;
mod describe;
pub use cost::{KiCadCost, KiCadFabQuote};
pub use labels::KiCadLabelReport;
pub use net_classes::KiCadNetClassRequest;
pub use quality::{KiCadDecoupling, KiCadQualityReport, score_kicad_board};
pub use silk_fields::KiCadSilkscreenReport;
mod fab_export;
pub use fab_export::{KiCadFabExport, export_kicad_fab};
mod fab_release;
pub use class_view::{
    KiCadClassLayer, KiCadClassLayers, KiCadClassRow, KiCadControlledImpedance, KiCadDesignerWidth,
    note_designer_class_layers, note_designer_widths,
};
pub use describe::{KiCadBoardDescription, KiCadSwapCandidate, describe_kicad_board};
pub use fab_release::{
    KiCadFabDrc, KiCadFabErc, KiCadFabFile, KiCadFabOptions, KiCadFabProfile, KiCadFabRelease,
    KiCadFabVerdictLines, fab_kicad_board, fab_verdict_lines,
};
pub use placement_constraints::{
    KiCadConstraintStatus, KiCadConstraintsSource, KiCadPlacementConstraints, keepout_warnings,
    overlap_warnings, placement_input_warnings, read_placement_constraints,
};
pub use placement_room::{KiCadRoom, TARGET_UTILIZATION, noted_room};
pub mod free_dimension;
pub use free_dimension::{
    KiCadFreeDimension, KiCadFreeDimensionPlan, KiCadFreeDimensionRun, KiCadFreeDimensionSearch,
    KiCadFreeDimensionStep,
};
pub use pin_swap::{
    KiCadPinChange, KiCadPinSwapConfig, KiCadPinSwapEstimate, KiCadPinSwapResult, KiCadPinSwapSpec,
    read_pin_swap_spec, swap_kicad_pins, swap_kicad_pins_in_place,
};
pub use sub_boards::describe_sub_boards;
mod run_diagnosis;
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
pub use project_rules::{
    KiCadLayerGeometryRule, KiCadProjectMinimums, KiCadRuleOverride, preflight_overrides,
    resolve_project_rules, router_config_over_project,
};
pub use run_diagnosis::{
    KiCadRunDiagnosis, KiCadRunFiles, KiCadRunInspection, inspect_routing_run, run_files,
};

pub use pcb_kicad_route_place::configure_allocator;

/// The memory profile (`PCB_ROUTER_MEM_PROFILE`): the counting allocator,
/// the sampler, the phases (docs/profiling.md, "Memory").
pub use pcb_router::mem_profile;

// Split out of lib.rs on 2026-10-21.
use board_stats::*;
pub use board_stats::{
    KiCadBoardStatistics, KiCadConnectionSummary, KiCadCopperLayerStatistics,
    KiCadPhysicalCopperStatistics, inspect_kicad_board, inspect_kicad_routable_connections,
};
pub use copper_emit::{
    KiCadCopperStripReport, KiCadRouteBranch, KiCadRoutePoint, SupplementalSegment,
    SupplementalVia, write_kicad_board_without_copper,
};
use copper_geometry::*;
pub use fs_util::check_board_id;
use fs_util::*;
use geometry::*;
use kicad_nets::*;
use kicad_sexpr::*;
pub use verification::{
    KICAD_VERIFICATION_CACHE_ENV, KICAD_VERIFICATION_CACHE_EVENTS_ENV,
    KiCadVerificationCacheStatus, KiCadVerificationCacheTrace, VerificationReport,
    kicad_verifier_implementation_sha256, verify_materialized_rung,
};

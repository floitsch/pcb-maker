// Copyright (C) 2026 Toit contributors.

//! The KiCad side of pcb-maker: boards and projects read and written as
//! KiCad's S-expressions (kicad_sexpr, kicad_nets, copper_emit), their
//! geometry (geometry, copper_geometry, outline, roundrect_pad), the
//! adapters that lower a board to the placer and the router and write the
//! result back (board_placer, board_router, board_layout), KiCad's own
//! checks (verification, native_report, drc_cache, verdict) and the agent
//! commands' other tools (netlist import, fab export, pin swaps, ...).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

mod connection_rules;
mod corridors_fact;
pub use connection_rules::KiCadConnectionRoutingRules;
pub use corridors_fact::{KiCadCorridors, KiCadSearchCount, KiCadSearchKinds};
mod local_clearance;
mod native_report;
mod necks;
pub use necks::{NeckRepair, repair_kicad_necks};
mod check_work;
mod drc_cache;
mod kicad_config;
pub use drc_cache::{DrcCacheCounts, drc_cache_counts, write_atomically};
mod check_repairs;
mod electrical_terminals;
#[cfg(test)]
mod geometry_queries;
mod roundrect_pad;
mod silkscreen;
mod verification_preview;
pub use native_report::{
    KICAD_REQUIREMENT, check_kicad_cli, erc_design_issues, is_erc_library_warning,
    is_library_metadata_warning,
};
use native_report::{ReportKind, remove_if_present, run_kicad_report, validate_report};
pub use silkscreen::repair_kicad_silkscreen;
mod board_layout;
mod congestion_rank;
mod congestion_samples;
mod global_route;
mod move_descent;
mod trouble_probe;
pub use board_layout::{
    KiCadBoardLayoutConfig, KiCadBoardLayoutMove, KiCadBoardLayoutResult, KiCadOutlineTrial,
    KiCadRaceProbe, KiCadRaceProbeScore, KiCadRaceReport, KiCadSilkscreenConfig, KiCadSwappable,
    layout_kicad_board,
};
pub use congestion_rank::{KiCadCongestionScore, score_congestion_sample, score_kicad_congestion};
pub use congestion_samples::{
    KiCadCongestionJob, KiCadCongestionSampleReport, KiCadCongestionVariant, KiCadPerturbation,
    export_congestion_samples,
};
pub use global_route::{KiCadGlobalRouteReport, KiCadProbeReport, global_route_kicad_board};
pub use move_descent::{
    KiCadDescentMove, KiCadMoveDescent, KiCadMoveDescentReport, KiCadMovesFact,
};
pub use trouble_probe::{
    KiCadTroubleBackground, KiCadTroubleNet, KiCadTroubleProbeConfig, KiCadTroubleProbeReport,
    KiCadTroubleSource, probe_trouble_placement,
};
mod board_placer;
mod canonical_order;
pub use board_placer::{
    KiCadBoardPlacerConfig, KiCadBoardPlacerResult, KiCadLegalityReport, KiCadPanelCellClass,
    KiCadPanelCopiesReport, KiCadPanelCopy, KiCadPlacedFootprint, legality_kicad_board,
    place_kicad_board,
};
mod board_router;
mod verdict;
pub use verdict::{
    DrcFinding, DrcTally, EXIT_COMPLETE, EXIT_CUT, EXIT_INCOMPLETE, EXIT_INTERNAL, EXIT_USAGE,
    KiCadReference, KiCadRunFacts, KiCadRunStatus, KiCadVerdict, decide, drc_finding,
    error_verdict, footprint_findings_note, footprint_own_unreferenced, input_error, is_cosmetic,
    is_input_error, judge, kicad_drc_report, load_drc_tally, noted_input_actions, read_drc_tally,
    reference_findings, room_action, route_facts, set_build_version, split_findings, tally_drc,
    wall_cuts,
};
mod work_rate;
pub use work_rate::{
    KiCadMeasuredRate, KiCadWorkRate, WORK_RATE_VARIABLE, calibrate as calibrate_work_rate,
    measure as measure_work_rate, parse_work_rate, work_rate,
};
mod constraints_template;
mod describe;
mod flip;
mod net_classes;
pub use net_classes::KiCadNetClassRequest;
mod aesthetics;
mod quality;
pub use quality::{KiCadDecoupling, KiCadQualityReport, score_kicad_board};
mod cost;
pub use cost::{KiCadCost, KiCadFabQuote};
mod labels;
mod silk_fields;
pub use labels::KiCadLabelReport;
pub use silk_fields::KiCadSilkscreenReport;
mod fab_export;
pub use fab_export::{KiCadFabExport, export_kicad_fab};
mod fab_release;
pub use fab_release::{
    KiCadFabDrc, KiCadFabErc, KiCadFabFile, KiCadFabOptions, KiCadFabProfile, KiCadFabRelease,
    fab_kicad_board,
};
mod board_footprints;
mod key_matrix;
pub use key_matrix::{KiCadStarterHalf, KiCadStarterReport};
mod netlist_import;
pub use constraints_template::constraints_template;
pub use describe::{KiCadBoardDescription, KiCadSwapCandidate, describe_kicad_board};
pub use netlist_import::{KiCadNetlistImport, import_kicad_netlist, import_kicad_netlist_from};
mod placement_ablation;
mod placement_constraints;
pub use placement_ablation::{
    KiCadAblationConfig, KiCadAblationGroup, KiCadAblationReport, KiCadAblationVariant,
    ablate_kicad_placement,
};
mod placement_room;
pub use placement_constraints::{
    KiCadConstraintStatus, KiCadConstraintsSource, KiCadPlacementConstraints, keepout_warnings,
    overlap_warnings, placement_input_warnings, read_placement_constraints,
};
pub use placement_room::{KiCadRoom, TARGET_UTILIZATION, noted_room};
pub mod free_dimension;
pub mod ledger;
mod outline;
pub use free_dimension::{
    KiCadFreeDimension, KiCadFreeDimensionPlan, KiCadFreeDimensionRun, KiCadFreeDimensionSearch,
    KiCadFreeDimensionStep,
};
mod pin_swap;
pub use pin_swap::{
    KiCadPinChange, KiCadPinSwapConfig, KiCadPinSwapEstimate, KiCadPinSwapResult, KiCadPinSwapSpec,
    read_pin_swap_spec, swap_kicad_pins, swap_kicad_pins_in_place,
};
mod shrink;
pub use shrink::{KiCadShrinkReport, shrink_kicad_board};
mod project_rules;
mod run_diagnosis;
pub use board_router::{
    KiCadBoardRouterConfig, KiCadBoardRouterNet, KiCadBoardRouterResult, KiCadBoardRouterViolation,
    KiCadDiffPair, KiCadDiffPairRule, KiCadDiffPairs, KiCadExistingTracks, KiCadKeepWhole,
    KiCadKeptCopper, KiCadNetLength, KiCadPourMode, KiCadPourQuality, bench_kicad_search,
    dump_kicad_pour_map, keep_whole_input_warnings, measure_pours, route_kicad_board,
    write_kicad_board_without_tracks,
};
pub use project_rules::{
    KiCadProjectMinimums, KiCadRuleOverride, preflight_overrides, resolve_project_rules,
    router_config_over_project,
};
pub use run_diagnosis::{
    KiCadRunDiagnosis, KiCadRunFiles, KiCadRunInspection, inspect_routing_run, run_files,
};

/// Sets the allocator up for the router's large maps (glibc: blocks of a
/// MiB or more are unmapped when freed; `board_router::configure_malloc`).
/// Call once, first thing in `main`.
pub fn configure_allocator() {
    board_router::configure_malloc();
}

/// The memory profile (`PCB_ROUTER_MEM_PROFILE`): the counting allocator,
/// the sampler, the phases (docs/profiling.md, "Memory").
pub use pcb_router::mem_profile;

// Split out of lib.rs on 2026-10-21.
mod kicad_sexpr;
use kicad_sexpr::*;
mod kicad_nets;
use kicad_nets::*;
mod geometry;
use geometry::*;
mod copper_geometry;
use copper_geometry::*;
mod copper_emit;
use copper_emit::*;
pub use copper_emit::{
    KiCadCopperStripReport, KiCadRouteBranch, KiCadRoutePoint, SupplementalSegment,
    SupplementalVia, write_kicad_board_without_copper,
};
mod route_graph;
use route_graph::*;
mod board_stats;
use board_stats::*;
pub use board_stats::{
    KiCadBoardStatistics, KiCadConnectionSummary, KiCadCopperLayerStatistics,
    KiCadPhysicalCopperStatistics, inspect_kicad_board, inspect_kicad_routable_connections,
};
mod verification;
pub use verification::{
    KICAD_VERIFICATION_CACHE_ENV, KICAD_VERIFICATION_CACHE_EVENTS_ENV,
    KiCadVerificationCacheStatus, KiCadVerificationCacheTrace, VerificationReport,
    kicad_verifier_implementation_sha256, verify_materialized_rung,
};
mod fs_util;
pub use fs_util::check_board_id;
use fs_util::*;
mod layer_render;
pub use layer_render::{KiCadLayerRenderConfig, KiCadLayerRenderReport, render_board_layers};
mod footprint_links;
pub use footprint_links::{
    KiCadFootprintLinkNormalizationReport, KiCadFootprintLinkUpdate,
    KiCadSchematicLinkNormalization, normalize_kicad_footprint_links,
};

// The fixture tests on tests/fixtures/f1 that need the crate's insides
// (under tests/, as test sources: tools/config-reference's index of
// environment variables leaves out their PCB_UPDATE_GOLDEN).
#[cfg(test)]
#[path = "tests/fixtures.rs"]
mod fixture_tests;

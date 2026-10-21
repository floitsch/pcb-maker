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
pub use connection_rules::KiCadConnectionRoutingRules;
mod local_clearance;
mod native_report;
mod necks;
pub use necks::{NeckRepair, repair_kicad_necks};
mod check_work;
mod drc_cache;
mod kicad_config;
pub use drc_cache::{DrcCacheCounts, drc_cache_counts};
mod electrical_terminals;
#[cfg(test)]
mod geometry_queries;
mod roundrect_pad;
mod silkscreen;
mod verification_preview;
use native_report::{ReportKind, remove_if_present, run_kicad_report, validate_report};
pub use native_report::{erc_design_issues, is_erc_library_warning, is_library_metadata_warning};
pub use silkscreen::repair_kicad_silkscreen;
mod board_layout;
mod congestion_rank;
mod congestion_samples;
pub use board_layout::{
    KiCadBoardLayoutConfig, KiCadBoardLayoutMove, KiCadBoardLayoutResult, KiCadOutlineTrial,
    KiCadSwappable, layout_kicad_board,
};
pub use congestion_rank::{KiCadCongestionScore, score_congestion_sample, score_kicad_congestion};
pub use congestion_samples::{
    KiCadCongestionJob, KiCadCongestionSampleReport, KiCadCongestionVariant, KiCadPerturbation,
    export_congestion_samples,
};
mod board_placer;
mod canonical_order;
pub use board_placer::{
    KiCadBoardPlacerConfig, KiCadBoardPlacerResult, KiCadPlacedFootprint, place_kicad_board,
};
mod board_router;
mod verdict;
pub use verdict::{
    DrcTally, EXIT_COMPLETE, EXIT_CUT, EXIT_INCOMPLETE, EXIT_INTERNAL, EXIT_USAGE, KiCadReference,
    KiCadRunFacts, KiCadRunStatus, KiCadVerdict, decide, error_verdict, input_error, is_cosmetic,
    is_input_error, judge, kicad_drc_report, read_drc_tally, reference_findings, room_action,
    route_facts, split_findings, tally_drc, wall_cuts,
};
mod work_rate;
pub use work_rate::{
    KiCadMeasuredRate, KiCadWorkRate, WORK_RATE_VARIABLE, calibrate as calibrate_work_rate,
    measure as measure_work_rate, parse_work_rate, work_rate,
};
mod describe;
mod flip;
mod net_classes;
pub use net_classes::KiCadNetClassRequest;
mod quality;
pub use quality::{KiCadQualityReport, score_kicad_board};
mod cost;
pub use cost::{KiCadCost, KiCadFabQuote};
mod labels;
pub use labels::KiCadLabelReport;
mod fab_export;
pub use fab_export::{KiCadFabExport, export_kicad_fab};
mod board_footprints;
mod key_matrix;
pub use key_matrix::{KiCadStarterHalf, KiCadStarterReport};
mod netlist_import;
pub use describe::{KiCadBoardDescription, KiCadSwapCandidate, describe_kicad_board};
pub use netlist_import::{KiCadNetlistImport, import_kicad_netlist, import_kicad_netlist_from};
mod placement_constraints;
mod placement_room;
pub use placement_constraints::{
    KiCadConstraintStatus, KiCadConstraintsSource, KiCadPlacementConstraints,
    read_placement_constraints,
};
pub use placement_room::{KiCadRoom, TARGET_UTILIZATION, noted_room};
mod outline;
mod pin_swap;
pub use pin_swap::{
    KiCadPinChange, KiCadPinSwapConfig, KiCadPinSwapEstimate, KiCadPinSwapResult, KiCadPinSwapSpec,
    read_pin_swap_spec, swap_kicad_pins, swap_kicad_pins_in_place,
};
mod shrink;
pub use shrink::{KiCadShrinkReport, shrink_kicad_board};
mod project_rules;
pub use board_router::{
    KiCadBoardRouterConfig, KiCadBoardRouterNet, KiCadBoardRouterResult, KiCadBoardRouterViolation,
    KiCadPourMode, bench_kicad_search, dump_kicad_pour_map, route_kicad_board,
    write_kicad_board_without_tracks,
};
pub use project_rules::resolve_project_rules;

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
use fs_util::*;
mod layer_render;
pub use layer_render::{KiCadLayerRenderConfig, KiCadLayerRenderReport, render_board_layers};
mod footprint_links;
pub use footprint_links::{
    KiCadFootprintLinkNormalizationReport, KiCadFootprintLinkUpdate,
    KiCadSchematicLinkNormalization, normalize_kicad_footprint_links,
};

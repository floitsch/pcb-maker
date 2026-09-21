// Copyright (C) 2026 Toit contributors.

use pcb_core::{Bounds, Mobility, Vec2};
use pcb_engine::{
    Backend, CopperAttachmentInput, CopperBodyInput, CopperBodySeparationMinimum,
    CopperBodySeparationPair, CopperPointRef, CopperPolylineInput, CopperPolylineMobility,
    CopperRepairRequest, CopperSegmentBodyPair, CopperSeparationPair, CopperSharedPointInput,
    CpuReferenceBackend, SolverConfig, compile_copper_repair,
};
use pcb_grid_router::{
    DutAStar, GridFloodFill, GridPosition, GridReachabilityChecker, GridReachabilityOutcome,
    GridRouteOutcome, GridRouteRequest, GridRouter, route_with_obstacle_distances,
};
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

mod connection_order;
mod connection_rules;
pub use connection_rules::KiCadConnectionRoutingRules;
mod grid_alignment;
mod local_clearance;
mod native_report;
mod pad_layer_contacts;
mod electrical_terminals;
use electrical_terminals::ElectricalTerminal;
mod terminal_access;
pub use terminal_access::inspect_kicad_terminal_access;
mod routing_cut;
pub use routing_cut::{KiCadRoutingCutConfig, KiCadRoutingCutReport, inspect_kicad_routing_cut};
mod joint_relaxation;
pub use joint_relaxation::{KiCadJointRelaxationConfig, KiCadJointSolverConfig, KiCadJointRelaxationStatus, KiCadJointRelaxationResult, relax_kicad_joint_routes};
mod roundrect_pad;
mod pad_pair;
pub use pad_pair::{KiCadPadReference, KiCadPadPairRoutingConfig, KiCadPadPairRouteProposal, propose_kicad_pad_pair_route};
mod island_bridge;
pub use island_bridge::{KiCadIslandBridgeConfig, KiCadIslandBridgeResult, bridge_kicad_open_connections};
mod route_obstructions;
pub use route_obstructions::{KiCadRouteObstructionContact, KiCadRouteObstructionReport, KiCadRouteObstructionStatus, inspect_kicad_route_obstructions};
mod partial_ripup;
mod restoration_recovery;
mod compound_recovery;
pub use compound_recovery::{KiCadCompoundRecoveryConfig, KiCadCompoundRecoveryResult, KiCadSequentialCompoundEvidence};
pub use restoration_recovery::{KiCadRestorationRecoveryConfig, KiCadRestorationEvidence, KiCadChangedRouteReceipt};
mod route_failure;
pub use route_failure::{
    KiCadRouteError, KiCadRouteSearchFailure, KiCadRouteSearchFailureKind, KiCadRouteTerminalContext,
};
mod coupled_vias;
#[cfg(test)]
mod route_union_tests;
mod routing_cost;
mod routing_demand;
pub use routing_demand::{KiCadDemandAlternative, KiCadRoutingDemand};
mod routing_grid;
#[cfg(test)]
mod rule_area_tests;
mod silkscreen;
mod verification_preview;
mod via_discovery;
pub use coupled_vias::{KiCadCoupledViaConfig, refine_kicad_vias};
use native_report::{ReportKind, remove_if_present, run_kicad_report, validate_report};
pub use native_report::{drc_design_issues, is_library_metadata_warning};
pub use silkscreen::repair_kicad_silkscreen;
pub use via_discovery::{KiCadViaDiscoveryConfig, discover_kicad_via_opportunities};
mod adaptive_routing;
mod semantic_template;
mod sequential_order_search;
mod sequential_resume;
mod sequential_router;
pub use adaptive_routing::{
    KiCadAdaptiveInitialOrder, KiCadAdaptiveRoutingConfig, KiCadAdaptiveRoutingResult,
    route_kicad_board_adaptively,
};

pub use connection_order::*;
pub use sequential_order_search::*;
pub use sequential_resume::{
    KiCadSequentialResumeConfig, KiCadSequentialResumeEvidence, resume_kicad_board_sequentially,
};
pub use sequential_router::*;

pub use semantic_template::{
    SemanticKiCadPose, SemanticKiCadTemplateConfig, SemanticKiCadTemplateReport,
    write_semantic_kicad_ladder_template,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LadderDeclaration {
    pub schema_version: u32,
    pub board_id: String,
    pub schematic_template: PathBuf,
    pub pcb_template: PathBuf,
    #[serde(default)]
    pub project_template: Option<PathBuf>,
    pub connections: Vec<String>,
    #[serde(default)]
    pub route_vertex_overrides: Vec<RouteVertexOverride>,
    #[serde(default)]
    pub supplemental_segments: Vec<SupplementalSegment>,
    #[serde(default)]
    pub supplemental_vias: Vec<SupplementalVia>,
    #[serde(default)]
    pub replacement_route_candidates: Vec<PathBuf>,
    /// Declarative post-processing policies that replace selected routed
    /// connections with zones while optionally retaining a sparse subset of
    /// their vias as zone-to-zone stitching candidates.
    #[serde(default)]
    pub connection_zone_replacements: Vec<KiCadGroundZoneConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteVertexOverride {
    pub connection: String,
    pub from: [f64; 2],
    pub to: [f64; 2],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupplementalSegment {
    pub connection: String,
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub width: f64,
    pub layer: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupplementalVia {
    pub connection: String,
    pub at: [f64; 2],
    pub size: f64,
    pub drill: f64,
    pub layers: [String; 2],
}

/// How a multi-terminal connection acquires branch sources. Two-terminal
/// connections are identical under both policies.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadMultiTerminalRoutingPolicy {
    /// Route every non-root terminal independently from one deterministic
    /// terminal. This is the historical control.
    #[default]
    RootedStar,
    /// Route the first branch from the root, then attach later terminals to
    /// already-routed same-net copper. Shared vias and trunks are legal.
    SharedCopperTree,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadTreeAttachmentObjective {
    #[default]
    LengthThenVias,
    ViasThenLength,
    RouterCost,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadTreeAttachmentSearch {
    #[default]
    RankedSamples,
    /// One router-cost search seeded by every represented tree contact.
    MultiSource,
}

impl KiCadTreeAttachmentSearch {
    fn is_ranked_samples(&self) -> bool {
        *self == Self::RankedSamples
    }
}

/// Where materialized copper stops at a terminal pad. Grid search still uses
/// the deterministic pad anchor under both policies.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadTerminalContactPolicy {
    /// Preserve the historical center-to-center materialization control.
    #[default]
    PadAnchor,
    /// Remove the portion of the searched path that is already inside the pad.
    FirstPadContact,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadGridRouteConfig {
    /// Optional soft forecasts of other nets' future copper. Never changes
    /// geometry, design rules or native admission.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_demand: Option<KiCadRoutingDemand>,
    /// Complete resolved net-class geometry when nonempty. Retained in route
    /// candidates for deterministic replay and foreign-net clearance checks.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub connection_rules: BTreeMap<String, KiCadConnectionRoutingRules>,
    pub resolution_mm: f64,
    #[serde(skip_serializing_if = "KiCadGridAlignment::is_board_origin")]
    pub grid_alignment: KiCadGridAlignment,
    #[serde(skip_serializing_if = "KiCadGridHeuristic::is_geometric")]
    pub heuristic: KiCadGridHeuristic,
    pub trace_width_mm: f64,
    pub clearance_mm: f64,
    pub edge_clearance_mm: f64,
    pub hole_to_hole_clearance_mm: f64,
    pub via_size_mm: f64,
    pub via_drill_mm: f64,
    pub straight_cost: u32,
    pub diagonal_cost: u32,
    pub bend_cost: u32,
    pub via_cost: u32,
    /// Optional straight-track-equivalent millimetres per layer transition.
    /// When set, overrides via_cost and scales with grid resolution. None
    /// preserves the historical integer cost for existing experiments.
    pub via_cost_mm: Option<f64>,
    pub max_expansions: u32,
    /// Prove physical-grid connectivity before running the direction-aware
    /// cost search. This is conservative: reachable cases still run A*.
    /// ObstacleDistances includes its own reachability calculation and skips
    /// this separate flood fill; its work is reported as heuristic_expansions.
    pub reachability_preflight: bool,
    pub multi_terminal_routing: KiCadMultiTerminalRoutingPolicy,
    #[serde(skip_serializing_if = "KiCadTreeAttachmentSearch::is_ranked_samples")]
    pub tree_attachment_search: KiCadTreeAttachmentSearch,
    pub maximum_tree_attachment_searches: usize,
    /// Minimum physical separation between searched attachment sources on the
    /// same layer. Zero retains every ranked grid point.
    pub tree_attachment_minimum_spacing_mm: f64,
    pub tree_attachment_objective: KiCadTreeAttachmentObjective,
    pub terminal_contact_policy: KiCadTerminalContactPolicy,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadGridHeuristic {
    #[default]
    Geometric,
    ObstacleDistances,
}

impl KiCadGridHeuristic {
    fn is_geometric(&self) -> bool {
        *self == Self::Geometric
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadGridAlignment {
    #[default]
    BoardOrigin,
    /// Shift one grid axis into a narrow pad gap ranked by terminal detour.
    /// This is an alternate routing policy, not exhaustive gap coverage.
    NearestNarrowPadGap,
}

impl KiCadGridAlignment {
    fn is_board_origin(&self) -> bool {
        *self == Self::BoardOrigin
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadGridAlignmentEvidence {
    pub footprint: String,
    pub layer: String,
    pub axis: usize,
    pub center_band_mm: [f64; 2],
    pub passage: [[f64; 2]; 2],
    pub detour_mm: f64,
    pub offset_mm: f64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadRouteShorteningStrategy {
    #[default]
    GreedyLineOfSight,
    VisibilityShortestPath,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadRouteShorteningConfig {
    pub strategy: KiCadRouteShorteningStrategy,
    pub maximum_visibility_tests: usize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadRouteRelaxationStrategy {
    #[default]
    ProjectedTraceTension,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadRouteBroadPhase {
    #[default]
    Exhaustive,
    UniformGrid,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadSharedCopperPolicy {
    #[default]
    IndependentBranches,
    SharedPoints,
    SharedRouteGraph,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadViaMobilityPolicy {
    /// Preserve the original via-free adapter contract.
    #[default]
    Reject,
    /// Split branches into same-layer particle runs and pin every existing via
    /// with a shared fixed anchor. Via insertion, removal, and motion remain
    /// separate future experiments.
    Fixed,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadPlacementClearancePolicy {
    #[default]
    Disabled,
    /// Do not let a movable footprint consume any of the signed separation
    /// between its initial courtyard rectangle and another footprint's.
    PreserveInitialCourtyard,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadBranchSelectionPolicy {
    /// Relax exactly the branches named by `branch_indices`.
    #[default]
    Explicit,
    /// Treat `branch_indices` as seeds. For each hop, add the branches at the
    /// nearest shared physical-copper junction when walking from each seed's
    /// leaf toward its root.
    NearestSharedJunction,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadRouteRelaxationConfig {
    pub strategy: KiCadRouteRelaxationStrategy,
    pub broad_phase: KiCadRouteBroadPhase,
    pub shared_copper: KiCadSharedCopperPolicy,
    pub via_mobility: KiCadViaMobilityPolicy,
    pub placement_clearance: KiCadPlacementClearancePolicy,
    pub branch_selection: KiCadBranchSelectionPolicy,
    pub branch_indices: Vec<usize>,
    pub branch_neighborhood_hops: usize,
    pub maximum_selected_branches: usize,
    pub maximum_branch_contact_tests: usize,
    pub steps: usize,
    pub timestep: f64,
    pub projection_iterations: usize,
    pub trace_tension_strength: f64,
    pub maximum_trace_tension_step_mm: f64,
    pub constraint_guard_mm: f64,
    pub broad_phase_cell_size_mm: f64,
    pub broad_phase_motion_margin_mm: f64,
    pub maximum_constraint_pairs: usize,
    pub minimum_length_improvement_mm: f64,
    pub movable_footprints: Vec<KiCadMovableFootprintConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadMovableFootprintConfig {
    pub reference: String,
    pub translate_x: bool,
    pub translate_y: bool,
    pub rotate: bool,
    pub inverse_mass: f64,
    pub rotation_mobility: f64,
    pub maximum_translation_mm: f64,
    pub maximum_rotation_degrees: f64,
}

impl Default for KiCadMovableFootprintConfig {
    fn default() -> Self {
        Self {
            reference: String::new(),
            translate_x: true,
            translate_y: true,
            rotate: true,
            inverse_mass: 1.0,
            rotation_mobility: 1.0,
            maximum_translation_mm: 2.0,
            maximum_rotation_degrees: 45.0,
        }
    }
}

impl Default for KiCadRouteRelaxationConfig {
    fn default() -> Self {
        Self {
            strategy: KiCadRouteRelaxationStrategy::ProjectedTraceTension,
            broad_phase: KiCadRouteBroadPhase::Exhaustive,
            shared_copper: KiCadSharedCopperPolicy::IndependentBranches,
            via_mobility: KiCadViaMobilityPolicy::Reject,
            placement_clearance: KiCadPlacementClearancePolicy::Disabled,
            branch_selection: KiCadBranchSelectionPolicy::Explicit,
            branch_indices: Vec::new(),
            branch_neighborhood_hops: 1,
            maximum_selected_branches: 64,
            maximum_branch_contact_tests: 1_000_000,
            steps: 16,
            timestep: 0.05,
            projection_iterations: 8,
            trace_tension_strength: 4.0,
            maximum_trace_tension_step_mm: 0.2,
            constraint_guard_mm: 0.001,
            broad_phase_cell_size_mm: 2.0,
            broad_phase_motion_margin_mm: 1.0,
            maximum_constraint_pairs: 100_000,
            minimum_length_improvement_mm: 1.0e-6,
            movable_footprints: Vec::new(),
        }
    }
}

impl Default for KiCadRouteShorteningConfig {
    fn default() -> Self {
        Self {
            strategy: KiCadRouteShorteningStrategy::GreedyLineOfSight,
            maximum_visibility_tests: 1_000_000,
        }
    }
}

impl Default for KiCadGridRouteConfig {
    fn default() -> Self {
        Self {
            routing_demand: None,
            connection_rules: BTreeMap::new(),
            resolution_mm: 0.25,
            grid_alignment: KiCadGridAlignment::BoardOrigin,
            heuristic: KiCadGridHeuristic::Geometric,
            trace_width_mm: 0.25,
            clearance_mm: 0.2,
            edge_clearance_mm: 0.5,
            hole_to_hole_clearance_mm: 0.25,
            via_size_mm: 0.8,
            via_drill_mm: 0.4,
            straight_cost: 100,
            diagonal_cost: 141,
            bend_cost: 20,
            via_cost: 800,
            via_cost_mm: None,
            max_expansions: 2_000_000,
            reachability_preflight: false,
            multi_terminal_routing: KiCadMultiTerminalRoutingPolicy::RootedStar,
            tree_attachment_search: KiCadTreeAttachmentSearch::RankedSamples,
            maximum_tree_attachment_searches: 1,
            tree_attachment_minimum_spacing_mm: 0.0,
            tree_attachment_objective: KiCadTreeAttachmentObjective::LengthThenVias,
            terminal_contact_policy: KiCadTerminalContactPolicy::PadAnchor,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadTreeAttachmentAttemptStatus {
    Found,
    NoPath,
    BudgetExhausted,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadTreeAttachmentAttempt {
    pub branch_index: usize,
    pub rank: usize,
    pub target_terminal: [f64; 2],
    /// The ranked tree point supplied as the router start.
    #[serde(default)]
    pub searched_source: [f64; 2],
    #[serde(default)]
    pub searched_source_layer: String,
    /// The actual join point after trimming every redundant tree prefix.
    pub source: [f64; 2],
    pub source_layer: String,
    pub status: KiCadTreeAttachmentAttemptStatus,
    pub cost: Option<u32>,
    pub expansions: u32,
    pub retained_length_mm: Option<f64>,
    pub via_count: Option<usize>,
    pub selected: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRouteCandidate {
    pub schema_version: u32,
    pub connection: String,
    pub router: String,
    pub config: KiCadGridRouteConfig,
    pub terminals: Vec<[f64; 2]>,
    pub grid_origin: [f64; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_alignment_evidence: Option<KiCadGridAlignmentEvidence>,
    pub grid_size: [usize; 2],
    pub cost: u32,
    #[serde(default)]
    pub reachability_expansions: u32,
    /// Reverse-distance preparation work, separate from the A* budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heuristic_expansions: Option<u32>,
    pub expansions: u32,
    pub branches: Vec<KiCadRouteBranch>,
    #[serde(default)]
    pub tree_attachment_attempts: Vec<KiCadTreeAttachmentAttempt>,
    pub supplemental_segments: Vec<SupplementalSegment>,
    pub supplemental_vias: Vec<SupplementalVia>,
    #[serde(default)]
    pub footprint_placements: Vec<KiCadFootprintPlacement>,
    #[serde(default)]
    pub reference_placements: Vec<KiCadReferencePlacement>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadFootprintPlacement {
    pub reference: String,
    /// Pose expected in the board to which this candidate is applied.
    /// Angles use KiCad's clockwise-positive board convention.
    pub source_at: [f64; 3],
    pub at: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadReferencePlacement {
    pub reference: String,
    /// Stable KiCad identity of the footprint's visible Reference property.
    pub uuid: String,
    /// Component-local pose expected before applying this candidate.
    pub source_at: [f64; 3],
    /// Component-local replacement pose. Angles retain KiCad convention.
    pub at: [f64; 3],
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadReferenceRelaxationEvidence {
    pub schema_version: u32,
    pub algorithm: String,
    pub iteration: usize,
    pub phase: usize,
    pub silk_overlap_findings: usize,
    pub selected_reference_fields: usize,
    pub unique_alternate_actions: usize,
    pub duplicate_of_phase: Option<usize>,
    pub placements: Vec<KiCadReferencePlacement>,
    pub exact_kicad_validation_required: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadReferenceActionDuplicate {
    pub phase: usize,
    pub duplicate_of_phase: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadReferenceActionAttempt {
    pub id: String,
    pub phase: usize,
    pub artifact_directory: PathBuf,
    pub candidate: KiCadRouteCandidate,
    pub relaxation: KiCadReferenceRelaxationEvidence,
    pub verification: VerificationReport,
    pub route_quality: KiCadRouteQuality,
    pub reference_displacement_mm: f64,
    pub selected: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadReferenceActionPortfolioEvidence {
    pub schema_version: u32,
    pub algorithm: String,
    pub board_id: String,
    pub connection: String,
    pub source_artifact_directory: PathBuf,
    pub source_verification: VerificationReport,
    pub generated_actions: usize,
    pub unique_actions: usize,
    pub duplicate_actions: Vec<KiCadReferenceActionDuplicate>,
    pub attempted_actions: usize,
    pub exact_complete_actions: usize,
    pub selected_source: bool,
    pub selected_attempt: Option<usize>,
    pub selected_phase: Option<usize>,
    pub complete: bool,
    pub selection_rule: String,
    pub attempts: Vec<KiCadReferenceActionAttempt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadViaActionSearchConfig {
    /// Stable physical identity within the immutable source candidate.
    pub target_at: [f64; 2],
    pub target_layers: [String; 2],
    /// Each named adjacent layer produces one remove-and-merge action.
    pub removal_layers: Vec<String>,
    /// Deterministic lattice used for relocation proposals.
    pub relocation_radius_mm: f64,
    pub relocation_step_mm: f64,
    pub maximum_relocation_candidates: usize,
    /// When present, replaces the blind relocation lattice with a bounded
    /// exact-geometry probe that retains only improving feasible-frontier
    /// points for native KiCad evaluation.
    #[serde(default)]
    pub feasible_frontier: Option<KiCadViaFeasibleFrontierConfig>,
    /// Optional remove-then-repair portfolio. Direct removals remain separate
    /// controls; each named layer/resolution pair adds one bounded local A*
    /// action around the merged run.
    #[serde(default)]
    pub local_reroute: Option<KiCadViaLocalRerouteSearchConfig>,
    pub maximum_actions: usize,
    pub maximum_affected_branches: usize,
    /// Soft score = canonical physical length + this value per physical via.
    pub via_penalty_mm: f64,
    pub minimum_score_improvement_mm: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadViaFeasibleFrontierConfig {
    /// Rays are distributed around the local physical-copper descent vector.
    pub angular_samples: usize,
    /// Exact analytic checks along each ray before boundary refinement.
    pub radial_samples: usize,
    /// Bisections used when adjacent radial samples change feasibility.
    pub boundary_refinement_steps: usize,
    /// Pull a refined boundary point into its feasible interval so that the
    /// native gate is not asked to decide a floating-point tangent.
    pub boundary_inset_mm: f64,
    /// Greedy score-ordered non-maximum suppression keeps native evaluations
    /// from being spent on numerically distinct points at one boundary.
    pub minimum_candidate_spacing_mm: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadViaLocalRerouteSearchConfig {
    pub merged_layers: Vec<String>,
    pub resolutions_mm: Vec<f64>,
    pub window_margin_mm: f64,
    pub maximum_grid_states: usize,
    pub maximum_expansions: u32,
    pub maximum_exact_edge_retries: usize,
    /// Optional counterfactual search over bounded combinations of semantic
    /// blockers. These trials diagnose causal cuts; suppressed obstacles are
    /// never removed from a selectable/native board candidate.
    #[serde(default)]
    pub blocker_cut: Option<KiCadViaBlockerCutSearchConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadViaBlockerCutSearchConfig {
    pub maximum_ranked_blockers: usize,
    pub maximum_cut_size: usize,
    pub maximum_trials: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KiCadViaTopologyAction {
    Remove {
        merged_layer: String,
    },
    RemoveAndReroute {
        merged_layer: String,
        resolution_mm: f64,
        window_margin_mm: f64,
    },
    Relocate {
        to: [f64; 2],
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadViaActionStatus {
    Proposed,
    Unsupported,
    Duplicate,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaActionAttempt {
    pub id: String,
    pub action: KiCadViaTopologyAction,
    pub status: KiCadViaActionStatus,
    pub detail: String,
    pub affected_branches: Vec<usize>,
    pub artifact_directory: Option<PathBuf>,
    pub candidate: Option<KiCadRouteCandidate>,
    pub duplicate_of: Option<usize>,
    pub analytic_geometry_clear: bool,
    pub analytic_blockers: Vec<String>,
    pub local_reroute: Option<KiCadViaLocalRerouteEvidence>,
    pub diagnostic: Option<KiCadViaLocalRerouteDiagnosticArtifacts>,
    pub verification: Option<VerificationReport>,
    pub route_quality: Option<KiCadRouteQuality>,
    pub score_mm: Option<f64>,
    pub selected: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaLocalRerouteEvidence {
    pub algorithm: String,
    pub resolution_mm: f64,
    pub window_margin_mm: f64,
    pub maximum_grid_states: usize,
    pub maximum_expansions_per_branch: u32,
    pub maximum_exact_edge_retries_per_branch: usize,
    pub total_searches: usize,
    pub total_expansions: u64,
    pub total_exact_edge_retries: usize,
    pub branches: Vec<KiCadViaLocalRerouteBranchEvidence>,
    pub failed_branch: Option<KiCadViaLocalRerouteFailureEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker_cut: Option<KiCadViaBlockerCutAnalysisEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaLocalRerouteBranchEvidence {
    pub branch_index: usize,
    pub start: [f64; 2],
    pub finish: [f64; 2],
    pub layer: String,
    pub window_bounds: [f64; 4],
    pub grid_size: [usize; 2],
    pub searches: usize,
    pub expansions: u64,
    pub exact_edge_retries: usize,
    pub cost: u32,
    pub path_points: usize,
    pub path_length_mm: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadViaLocalRerouteFailureKind {
    NoPath,
    BudgetExhausted,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaLocalRerouteBlockedRun {
    pub row: usize,
    pub first_column: usize,
    pub last_column: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadViaLocalRerouteBlockerKind {
    FootprintPad,
    ForeignNetCopper,
    RuleArea,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaLocalRerouteBlockerEvidence {
    pub kind: KiCadViaLocalRerouteBlockerKind,
    /// Stable semantic identity: a footprint reference or foreign net name.
    pub object: String,
    /// Every foreign net represented by the grouped obstacle. Footprint pads
    /// are grouped by component so one component can expose several nets.
    pub nets: Vec<String>,
    pub footprint: Option<String>,
    /// True only for footprint obstacles carrying `Router_Movable=yes`.
    /// Foreign copper is identified as rerouteable topology, not mislabeled
    /// as a movable component.
    pub component_movable: bool,
    pub frontier_hits: usize,
    pub frontier_centroid: [f64; 2],
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct KiCadViaLocalRerouteBlockerRef {
    pub kind: KiCadViaLocalRerouteBlockerKind,
    pub object: String,
    pub component_movable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadViaBlockerCutTrialStatus {
    Found,
    NoPath,
    BudgetExhausted,
    Unsupported,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaBlockerCutTrialEvidence {
    pub index: usize,
    /// The trial whose failed frontier exposed `added_blocker`. `None` means
    /// the blocker came from the original failed local route.
    pub parent_trial: Option<usize>,
    pub added_blocker: KiCadViaLocalRerouteBlockerRef,
    pub suppressed: Vec<KiCadViaLocalRerouteBlockerRef>,
    pub status: KiCadViaBlockerCutTrialStatus,
    pub detail: String,
    pub searches: usize,
    pub expansions: u64,
    pub exact_edge_retries: usize,
    pub path_cost: Option<u32>,
    pub path_length_mm: Option<f64>,
    pub path: Vec<KiCadRoutePoint>,
    pub failure: Option<Box<KiCadViaLocalRerouteFailureEvidence>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaBlockerCutAnalysisEvidence {
    pub algorithm: String,
    pub counterfactual_only: bool,
    pub selectable: bool,
    pub maximum_ranked_blockers: usize,
    pub maximum_cut_size: usize,
    pub maximum_trials: usize,
    pub ranked_blockers: Vec<KiCadViaLocalRerouteBlockerRef>,
    pub generated_cut_sets: usize,
    pub attempted_trials: usize,
    pub trials_truncated: bool,
    pub total_searches: usize,
    pub total_expansions: u64,
    pub total_exact_edge_retries: usize,
    pub minimum_sufficient_cut_size: Option<usize>,
    pub sufficient_trials: Vec<usize>,
    pub trials: Vec<KiCadViaBlockerCutTrialEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaLocalRerouteFailureEvidence {
    pub branch_index: usize,
    pub kind: KiCadViaLocalRerouteFailureKind,
    pub detail: String,
    pub start: [f64; 2],
    pub finish: [f64; 2],
    pub layer: String,
    pub resolution_mm: f64,
    pub obstacle_inflate_mm: f64,
    pub window_bounds: [f64; 4],
    pub grid_origin: [f64; 2],
    pub grid_size: [usize; 2],
    pub grid_states: usize,
    pub blocked_states: usize,
    pub blocked_runs: Vec<KiCadViaLocalRerouteBlockedRun>,
    pub searches: usize,
    pub expansions: u64,
    pub exact_edge_retries: usize,
    pub blocked_frontier_states: usize,
    pub blocked_frontier_runs: Vec<KiCadViaLocalRerouteBlockedRun>,
    pub attributed_frontier_states: usize,
    pub unattributed_frontier_states: usize,
    pub blockers: Vec<KiCadViaLocalRerouteBlockerEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaLocalRerouteDiagnosticArtifacts {
    pub candidate: PathBuf,
    pub evidence: PathBuf,
    pub front_svg: PathBuf,
    pub back_svg: PathBuf,
    pub front_png: Option<PathBuf>,
    pub back_png: Option<PathBuf>,
    pub blocker_cut_trials: Vec<KiCadViaBlockerCutDiagnosticArtifacts>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaBlockerCutDiagnosticArtifacts {
    pub trial: usize,
    pub status: KiCadViaBlockerCutTrialStatus,
    pub front_svg: PathBuf,
    pub back_svg: PathBuf,
    pub front_png: Option<PathBuf>,
    pub back_png: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaActionPortfolioEvidence {
    pub schema_version: u32,
    pub algorithm: String,
    pub board_id: String,
    pub connection: String,
    pub source_artifact_directory: PathBuf,
    pub source_verification: VerificationReport,
    pub source_quality: KiCadRouteQuality,
    pub source_score_mm: f64,
    pub target_at: [f64; 2],
    pub target_layers: [String; 2],
    pub affected_branches: Vec<usize>,
    pub relocation_generation: KiCadViaRelocationGenerationEvidence,
    pub generated_actions: usize,
    pub unique_actions: usize,
    pub attempted_actions: usize,
    pub exact_complete_actions: usize,
    pub selected_source: bool,
    pub selected_attempt: Option<usize>,
    pub complete: bool,
    pub selection_rule: String,
    pub attempts: Vec<KiCadViaActionAttempt>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaRelocationGenerationEvidence {
    pub method: String,
    pub descent_direction: Option<[f64; 2]>,
    pub angular_directions: usize,
    pub radial_samples_per_direction: usize,
    pub boundary_refinement_steps: usize,
    pub minimum_candidate_spacing_mm: f64,
    pub analytic_probes: usize,
    pub analytically_clear_probes: usize,
    pub feasibility_transitions: usize,
    pub boundary_refinement_probes: usize,
    pub improving_feasible_points: usize,
    pub suppressed_near_duplicates: usize,
    pub retained_relocations: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KiCadRouteQuality {
    /// Length of the canonical physical copper graph after splitting and
    /// deduplicating same-net overlap.
    pub length_mm: f64,
    /// Segment count in that canonical physical graph.
    pub segments: usize,
    /// Sum/count of serialized KiCad track objects before graph
    /// canonicalization. These expose overlap and representation debt rather
    /// than silently charging it as extra physical copper.
    pub stored_track_length_mm: f64,
    pub stored_segments: usize,
    pub overlapping_track_length_mm: f64,
    pub vias: usize,
    pub branch_via_transitions: usize,
    pub maximum_branch_vias: usize,
    pub via_cluster_threshold_mm: f64,
    pub minimum_via_spacing_mm: Option<f64>,
    pub close_via_pairs: usize,
    pub clustered_vias: usize,
    pub branch_bends: usize,
    pub branch_points: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRouteShorteningEvidence {
    pub schema_version: u32,
    pub connection: String,
    pub algorithm: String,
    pub source_router: String,
    pub source_cost: u32,
    pub source_expansions: u32,
    pub strategy: KiCadRouteShorteningStrategy,
    pub maximum_visibility_tests: usize,
    pub visibility_tests: usize,
    pub attempted_shortcuts: usize,
    pub visible_shortcuts: usize,
    pub accepted_shortcuts: usize,
    pub removed_branch_points: usize,
    pub status: KiCadRouteShorteningStatus,
    pub detail: String,
    pub before: KiCadRouteQuality,
    pub proposal: KiCadRouteQuality,
    pub after: KiCadRouteQuality,
    pub exact_kicad_validation_required: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadRouteShorteningStatus {
    Accepted,
    NoImprovement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadRouteRelaxationStatus {
    Accepted,
    NoImprovement,
    Unsupported,
    InvalidProposal,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadRouteRelaxationWork {
    pub frames: usize,
    pub constraint_projections: u64,
    pub scalar_rows: u64,
    pub trace_tension_edge_integrations: u64,
    pub compiled_segment_pairs: usize,
    pub compiled_segment_body_pairs: usize,
    pub compiled_body_body_pairs: usize,
    pub fixed_capsule_obstacles: usize,
    pub fixed_rectangular_obstacles: usize,
    pub layer_relevant_obstacle_pairs: usize,
    pub broad_phase_candidate_pairs: usize,
    pub retained_obstacle_pairs: usize,
    pub culled_obstacle_pairs: usize,
    pub broad_phase_cells: usize,
    pub broad_phase_cell_references: usize,
    pub broad_phase_cell_queries: usize,
    pub engine_particles: usize,
    pub selected_layer_runs: usize,
    pub fixed_via_anchors: usize,
    pub fixed_via_anchor_members: usize,
    pub fixed_context_anchors: usize,
    pub fixed_context_anchor_members: usize,
    pub shared_point_groups: usize,
    pub shared_point_members: usize,
    pub mixed_mobility_shared_point_groups: usize,
    pub route_graph_contact_points: usize,
    pub route_graph_inserted_points: usize,
    pub tension_edges: usize,
    pub deduplicated_tension_edges: usize,
    pub deduplicated_segment_pairs: usize,
    pub deduplicated_segment_body_pairs: usize,
    pub deduplicated_body_body_pairs: usize,
    pub terminal_attachment_bodies: usize,
    pub terminal_attachments: usize,
    pub placement_clearance_bodies: usize,
    pub placement_clearance_pairs: usize,
    pub maximum_frame_displacement_mm: f64,
    pub maximum_constraint_residual_mm: f64,
    pub final_constraint_residual_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadFootprintMotionEvidence {
    pub reference: String,
    pub source_at: [f64; 3],
    pub proposal_at: [f64; 3],
    pub translation_mm: f64,
    pub rotation_degrees: f64,
    pub within_configured_limits: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRelaxedBranchEvidence {
    pub branch_index: usize,
    pub before: Vec<KiCadRoutePoint>,
    pub proposal: Vec<KiCadRoutePoint>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRouteRelaxationEvidence {
    pub schema_version: u32,
    pub connection: String,
    pub algorithm: String,
    pub source_router: String,
    pub backend: String,
    pub status: KiCadRouteRelaxationStatus,
    pub detail: String,
    pub branch_selection: KiCadBranchSelectionPolicy,
    pub seed_branches: Vec<usize>,
    pub selected_branches: Vec<usize>,
    pub branch_neighborhood_hops: usize,
    pub maximum_selected_branches: usize,
    pub maximum_branch_contact_tests: usize,
    pub branch_contact_tests: usize,
    pub broad_phase: KiCadRouteBroadPhase,
    pub shared_copper: KiCadSharedCopperPolicy,
    pub via_mobility: KiCadViaMobilityPolicy,
    pub placement_clearance: KiCadPlacementClearancePolicy,
    pub broad_phase_cell_size_mm: f64,
    pub broad_phase_motion_margin_mm: f64,
    pub broad_phase_envelope_respected: bool,
    pub steps: usize,
    pub projection_iterations: usize,
    pub trace_tension_strength: f64,
    pub maximum_trace_tension_step_mm: f64,
    pub constraint_guard_mm: f64,
    pub minimum_length_improvement_mm: f64,
    pub maximum_point_motion_mm: f64,
    pub footprint_motions: Vec<KiCadFootprintMotionEvidence>,
    pub initial_geometry_clear: bool,
    pub proposal_geometry_clear: bool,
    pub proposal_blockers: Vec<String>,
    pub proposed_branches: Vec<KiCadRelaxedBranchEvidence>,
    pub before: KiCadRouteQuality,
    pub proposal: Option<KiCadRouteQuality>,
    pub after: KiCadRouteQuality,
    pub work: KiCadRouteRelaxationWork,
    pub exact_kicad_validation_required: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRouteBranch {
    pub start_terminal: [f64; 2],
    pub finish_terminal: [f64; 2],
    pub cost: u32,
    pub expansions: u32,
    pub path: Vec<KiCadRoutePoint>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRoutePoint {
    pub at: [f64; 2],
    pub layer: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Expr {
    Atom(String),
    List(Vec<Expr>),
}

impl Expr {
    fn head(&self) -> Option<&str> {
        match self {
            Self::List(items) => items.first().and_then(Self::atom),
            Self::Atom(_) => None,
        }
    }

    fn atom(&self) -> Option<&str> {
        match self {
            Self::Atom(atom) => Some(unquote(atom)),
            Self::List(_) => None,
        }
    }

    fn child(&self, head: &str) -> Option<&Expr> {
        match self {
            Self::List(items) => items.iter().find(|item| item.head() == Some(head)),
            Self::Atom(_) => None,
        }
    }

    fn children(&self) -> &[Expr] {
        match self {
            Self::List(items) => items,
            Self::Atom(_) => &[],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MaterializationReport {
    pub board_id: String,
    pub rung: usize,
    pub selected_connections: Vec<String>,
    pub schematic_wires: usize,
    pub schematic_no_connects: usize,
    pub pcb_segments: usize,
    pub pcb_arcs: usize,
    pub pcb_vias: usize,
    #[serde(default)]
    pub pcb_zones: usize,
    pub output_directory: PathBuf,
}

/// Cheap structural measurements read directly from a KiCad board. These are
/// suitable for benchmark provenance and cold-input checks. The length is the
/// sum of stored straight segments, not canonical physical-copper union.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KiCadBoardStatistics {
    pub footprints: usize,
    /// Distinct non-empty net names referenced anywhere in the board. This
    /// supports both ordinary KiCad `(net id "name")` forms and pcb-maker's
    /// name-addressed `(net "name")` forms.
    pub total_named_nets_including_no_connects: usize,
    pub segments: usize,
    pub arcs: usize,
    pub vias: usize,
    /// All top-level KiCad zone objects, including non-copper rule areas.
    pub zones: usize,
    /// Zones that describe copper pours rather than rule-area keepouts.
    #[serde(default)]
    pub copper_zones: usize,
    /// Non-copper zone objects carrying a top-level `(keepout ...)` form.
    #[serde(default)]
    pub rule_areas: usize,
    /// Top-level graphic objects placed directly on a copper layer. Footprint
    /// copper primitives and pads are fixed component geometry, not counted
    /// here.
    #[serde(default)]
    pub copper_graphics: usize,
    pub stored_segment_length_mm: f64,
    #[serde(default)]
    pub physical_copper: KiCadPhysicalCopperStatistics,
    /// Order-independent digest of top-level tracks, arcs, vias, and zones,
    /// excluding KiCad object UUIDs/timestamps. Raw board hashes remain useful
    /// provenance, while this digest identifies equivalent copper decisions.
    pub copper_geometry_sha256: String,
    /// Order-independent reference/position/rotation digest for every
    /// footprint. This proves two route-only competitors received the same
    /// fixed placement without depending on KiCad UUIDs or file order.
    pub component_placement_sha256: String,
    /// The same pose digest after coordinates and angles are rounded to the
    /// 0.0001 mm/degree precision used by the Specctra exchange path.
    pub component_placement_100nm_sha256: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadPhysicalCopperStatistics {
    pub schema_version: u32,
    /// Union length of straight same-net tracks after splitting every contact
    /// and deduplicating collinear overlap.
    pub canonical_straight_track_length_mm: f64,
    /// Circular centerline length of serialized KiCad arc tracks.
    pub arc_track_length_mm: f64,
    /// Straight physical union plus arc centerlines. This is an exact
    /// centerline union only when `centerline_union_exact` is true.
    pub physical_centerline_length_mm: f64,
    pub centerline_union_exact: bool,
    pub overlapping_straight_track_length_mm: f64,
    /// Sum of canonical straight edge length times maximum covering width,
    /// plus serialized arc length times width. It excludes end caps and does
    /// not subtract crossing or zone overlap.
    pub centerline_weighted_track_area_mm2: f64,
    pub via_projected_outer_area_mm2: f64,
    pub via_drill_area_mm2: f64,
    pub layer_weighted_via_annulus_area_mm2: f64,
    /// Shoelace area of refilled `filled_polygon` contours.
    pub filled_zone_area_mm2: f64,
    /// Track area + layer-weighted via annuli + filled zone polygons. This is
    /// a primitive sum, not a planar union.
    pub primitive_copper_area_mm2: f64,
    pub board_outline_area_mm2: Option<f64>,
    pub used_copper_layers: usize,
    pub layers: Vec<KiCadCopperLayerStatistics>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadCopperLayerStatistics {
    pub layer: String,
    pub stored_straight_track_length_mm: f64,
    pub canonical_straight_track_length_mm: f64,
    pub arc_track_length_mm: f64,
    pub physical_centerline_length_mm: f64,
    pub centerline_weighted_track_area_mm2: f64,
    pub vias_spanning_layer: usize,
    pub via_annulus_area_mm2: f64,
    pub filled_zone_area_mm2: f64,
    pub primitive_copper_area_mm2: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KiCadCopperStripReport {
    pub schema_version: u32,
    pub removed_segments: usize,
    pub removed_arcs: usize,
    pub removed_vias: usize,
    pub removed_copper_zones: usize,
    #[serde(default)]
    pub removed_copper_graphics: usize,
    pub retained_rule_areas: usize,
    pub before: KiCadBoardStatistics,
    pub after: KiCadBoardStatistics,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadConnectionSummary {
    pub connection: String,
    pub distinct_pad_centers: usize,
    /// Omitted when identical to geometric centers, including historical journals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub electrical_terminal_count: Option<usize>,
}

impl KiCadConnectionSummary {
    pub fn electrical_terminal_count(&self) -> usize {
        self.electrical_terminal_count.unwrap_or(self.distinct_pad_centers)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadFootprintLinkUpdate {
    #[serde(default)]
    pub schematic_path: String,
    pub reference: String,
    pub schematic_before: String,
    pub board_link: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadSchematicLinkNormalization {
    pub schematic_path: String,
    pub before_sha256: String,
    pub after_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadFootprintLinkNormalizationReport {
    pub schema_version: u32,
    pub board_id: String,
    pub updated_references: Vec<KiCadFootprintLinkUpdate>,
    pub schematic_before_sha256: String,
    pub schematic_after_sha256: String,
    pub board_sha256: String,
    pub copper_geometry_sha256: String,
    pub component_placement_sha256: String,
    #[serde(default)]
    pub schematic_files: Vec<KiCadSchematicLinkNormalization>,
}

/// Synchronizes footprint links throughout the referenced schematic hierarchy
/// to the embedded footprint links in an already materialized board. This is
/// intended for copied, pinned legacy projects whose library nicknames were
/// renamed after the board was laid out. It changes no board geometry,
/// placement, net, or copper decision.
pub fn normalize_kicad_footprint_links(
    directory: &Path,
    board_id: &str,
) -> Result<KiCadFootprintLinkNormalizationReport, String> {
    let board_path = directory.join(format!("{board_id}.kicad_pcb"));
    let schematic_path = directory.join(format!("{board_id}.kicad_sch"));
    let board_source = fs::read_to_string(&board_path)
        .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let board = parse(&board_source)
        .map_err(|error| format!("failed to parse {}: {error}", board_path.display()))?;
    let board_statistics = board_statistics(&board)?;
    let mut board_links = BTreeMap::new();
    for footprint in board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let Some(reference) = footprint_reference(footprint)
            .filter(|value| !value.is_empty() && !value.contains('*') && !value.starts_with('#'))
        else {
            continue;
        };
        let link = footprint
            .children()
            .get(1)
            .and_then(Expr::atom)
            .ok_or_else(|| format!("board footprint {reference:?} has no library link"))?
            .to_string();
        if let Some(previous) = board_links.insert(reference.clone(), link.clone())
            && previous != link
        {
            return Err(format!(
                "board has conflicting footprint links for reference {reference:?}"
            ));
        }
    }

    let project_root = fs::canonicalize(directory)
        .map_err(|error| format!("failed to resolve {}: {error}", directory.display()))?;
    let root_schematic = fs::canonicalize(&schematic_path)
        .map_err(|error| format!("failed to resolve {}: {error}", schematic_path.display()))?;
    let mut pending = VecDeque::from([root_schematic.clone()]);
    let mut visited = BTreeSet::new();
    let mut updates = BTreeMap::new();
    let mut schematic_files = Vec::new();
    while let Some(current_path) = pending.pop_front() {
        if !current_path.starts_with(&project_root) {
            return Err(format!(
                "referenced schematic {} escapes project {}",
                current_path.display(),
                project_root.display()
            ));
        }
        if !visited.insert(current_path.clone()) {
            continue;
        }
        let relative = current_path
            .strip_prefix(&project_root)
            .expect("project containment was checked")
            .to_string_lossy()
            .into_owned();
        let before_sha256 = file_sha256(&current_path)?;
        let schematic_source = fs::read_to_string(&current_path)
            .map_err(|error| format!("failed to read {}: {error}", current_path.display()))?;
        let mut schematic = parse(&schematic_source)
            .map_err(|error| format!("failed to parse {}: {error}", current_path.display()))?;
        let Expr::List(items) = &mut schematic else {
            return Err(format!("{} root is not a list", current_path.display()));
        };
        if root_head(items) != Some("kicad_sch") {
            return Err(format!(
                "{} is not a kicad_sch document",
                current_path.display()
            ));
        }
        for sheet_file in items
            .iter()
            .filter(|item| item.head() == Some("sheet"))
            .filter_map(|sheet| schematic_property(sheet, "Sheetfile"))
        {
            let child = Path::new(&sheet_file);
            if child.is_absolute() {
                return Err(format!(
                    "referenced schematic path {sheet_file:?} is absolute"
                ));
            }
            let child_path =
                fs::canonicalize(current_path.parent().unwrap().join(child)).map_err(|error| {
                    format!(
                        "failed to resolve child schematic {sheet_file:?} from {}: {error}",
                        current_path.display()
                    )
                })?;
            if !child_path.starts_with(&project_root) {
                return Err(format!(
                    "referenced schematic {} escapes project {}",
                    child_path.display(),
                    project_root.display()
                ));
            }
            pending.push_back(child_path);
        }
        let mut changed = false;
        for symbol in items
            .iter_mut()
            .filter(|item| item.head() == Some("symbol"))
        {
            let Some(reference) = schematic_property(symbol, "Reference") else {
                continue;
            };
            let Some(board_link) = board_links.get(&reference) else {
                continue;
            };
            let Some(before) = schematic_property(symbol, "Footprint") else {
                continue;
            };
            if before == *board_link {
                continue;
            }
            let Expr::List(symbol_items) = symbol else {
                unreachable!("symbol form must be a list")
            };
            let property = symbol_items
                .iter_mut()
                .find(|item| {
                    item.head() == Some("property")
                        && item.children().get(1).and_then(Expr::atom) == Some("Footprint")
                })
                .expect("Footprint property was read above");
            let Expr::List(property_items) = property else {
                unreachable!("property form must be a list")
            };
            property_items[2] = Expr::Atom(quote(board_link));
            changed = true;
            updates.insert(
                (relative.clone(), reference.clone()),
                KiCadFootprintLinkUpdate {
                    schematic_path: relative.clone(),
                    reference,
                    schematic_before: before,
                    board_link: board_link.clone(),
                },
            );
        }
        if changed {
            fs::write(&current_path, format!("{}\n", encode(&schematic))).map_err(|error| {
                format!(
                    "failed to write normalized {}: {error}",
                    current_path.display()
                )
            })?;
        }
        schematic_files.push(KiCadSchematicLinkNormalization {
            schematic_path: relative,
            before_sha256,
            after_sha256: file_sha256(&current_path)?,
        });
    }
    let root_audit = schematic_files
        .iter()
        .find(|audit| root_schematic == project_root.join(&audit.schematic_path))
        .expect("root schematic is always audited");
    Ok(KiCadFootprintLinkNormalizationReport {
        schema_version: 2,
        board_id: board_id.to_string(),
        updated_references: updates.into_values().collect(),
        schematic_before_sha256: root_audit.before_sha256.clone(),
        schematic_after_sha256: root_audit.after_sha256.clone(),
        board_sha256: file_sha256(&board_path)?,
        copper_geometry_sha256: board_statistics.copper_geometry_sha256,
        component_placement_sha256: board_statistics.component_placement_sha256,
        schematic_files,
    })
}

fn schematic_property(node: &Expr, name: &str) -> Option<String> {
    node.children().iter().find_map(|item| {
        (item.head() == Some("property") && item.children().get(1)?.atom()? == name)
            .then(|| item.children().get(2)?.atom().map(str::to_owned))
            .flatten()
    })
}

pub fn inspect_kicad_board(path: &Path) -> Result<KiCadBoardStatistics, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let pcb =
        parse(&source).map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!("{} is not a kicad_pcb document", path.display()));
    }
    board_statistics(&pcb)
}

/// Rewrites a KiCad board without inherited routed copper while retaining the
/// placement, pads, net declarations, board outline, non-copper graphics, and
/// rule-area keepouts. This is the cold-start adapter for reproducible routing
/// studies; it removes copper pours and top-level copper graphics because both
/// are inherited layout decisions.
pub fn write_kicad_board_without_copper(
    source: &Path,
    destination: &Path,
) -> Result<KiCadCopperStripReport, String> {
    let source_text = fs::read_to_string(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
    let mut pcb = parse(&source_text)
        .map_err(|error| format!("failed to parse {}: {error}", source.display()))?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!("{} is not a kicad_pcb document", source.display()));
    }
    let before = board_statistics(&pcb)?;
    let Expr::List(items) = &mut pcb else {
        unreachable!("kicad_pcb root must be a list")
    };
    items.retain(|item| match item.head() {
        Some("segment" | "arc" | "via") => false,
        Some("zone") => is_rule_area(item),
        _ => !is_top_level_copper_graphic(item),
    });
    let after = board_statistics(&pcb)?;
    if before.component_placement_sha256 != after.component_placement_sha256
        || before.component_placement_100nm_sha256 != after.component_placement_100nm_sha256
    {
        return Err("cold-board rewrite changed component placement".into());
    }
    if after.segments != 0
        || after.arcs != 0
        || after.vias != 0
        || after.copper_zones != 0
        || after.copper_graphics != 0
    {
        return Err("cold-board rewrite retained routed copper".into());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    fs::write(destination, format!("{}\n", encode(&pcb)))
        .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
    Ok(KiCadCopperStripReport {
        schema_version: 2,
        removed_segments: before.segments,
        removed_arcs: before.arcs,
        removed_vias: before.vias,
        removed_copper_zones: before.copper_zones,
        removed_copper_graphics: before.copper_graphics,
        retained_rule_areas: after.rule_areas,
        before,
        after,
    })
}

/// Returns named electrical connections with at least two electrical pad
/// contacts, in KiCad net-declaration order. This is the route inventory for a
/// full-connectivity cold board; net IDs are deliberately not exposed as
/// semantic connection names.
pub fn inspect_kicad_routable_connections(
    path: &Path,
) -> Result<Vec<KiCadConnectionSummary>, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let pcb =
        parse(&source).map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!("{} is not a kicad_pcb document", path.display()));
    }
    let mut connection_order = Vec::new();
    let mut seen = BTreeSet::new();
    for net in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("net"))
    {
        if let Some(name) = net_name_from_form(net).map(normalize_net)
            && !name.is_empty()
            && !name.bytes().all(|byte| byte.is_ascii_digit())
            && !name.starts_with("unconnected-(")
            && seen.insert(name.to_string())
        {
            connection_order.push(name.to_string());
        }
    }
    let mut centers = BTreeMap::<String, BTreeMap<Point, ([bool; 2], bool)>>::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let footprint_at = form_at(footprint)?;
        for pad in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            let Some(connection) = node_net(pad).map(normalize_net).filter(|name| {
                !name.is_empty()
                    && !name.bytes().all(|byte| byte.is_ascii_digit())
                    && !name.starts_with("unconnected-(")
            }) else {
                continue;
            };
            if seen.insert(connection.to_string()) {
                connection_order.push(connection.to_string());
            }
            let pad_at = form_at(pad)?;
            let offset = rotate_vector([pad_at[0], pad_at[1]], -footprint_at[2]);
            let center = [footprint_at[0] + offset[0], footprint_at[1] + offset[1]];
            let group = centers.entry(connection.to_string()).or_default()
                .entry(Point(
                    (center[0] * 1_000_000.0).round() as i64,
                    (center[1] * 1_000_000.0).round() as i64,
                )).or_insert(([false; 2], false));
            let layers = pad_copper_layers(pad);
            group.0[0] |= layers[0];
            group.0[1] |= layers[1];
            group.1 |= layers == [true, true] && electrical_terminals::plated_pad(pad);
        }
    }
    Ok(connection_order
        .into_iter()
        .filter_map(|connection| {
            let distinct_pad_centers = centers.get(&connection).map_or(0, BTreeMap::len);
            let electrical_terminal_count: usize = centers.get(&connection).into_iter()
                .flat_map(|groups| groups.values())
                .map(|(layers, plated)| electrical_terminals::contact_layers(*layers, *plated).len())
                .sum();
            (electrical_terminal_count >= 2).then_some(KiCadConnectionSummary {
                connection,
                distinct_pad_centers,
                electrical_terminal_count: (electrical_terminal_count != distinct_pad_centers)
                    .then_some(electrical_terminal_count),
            })
        })
        .collect())
}

fn board_statistics(pcb: &Expr) -> Result<KiCadBoardStatistics, String> {
    let mut stored_segment_length_mm = 0.0;
    for item in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("segment"))
    {
        let start = form_xy(item, "start")?;
        let end = form_xy(item, "end")?;
        stored_segment_length_mm += (end[0] - start[0]).hypot(end[1] - start[1]);
    }
    let mut named_nets = BTreeSet::new();
    collect_named_nets(pcb, &mut named_nets);
    let zones = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
        .collect::<Vec<_>>();
    let rule_areas = zones.iter().filter(|zone| is_rule_area(zone)).count();
    Ok(KiCadBoardStatistics {
        footprints: count_head(pcb, "footprint"),
        total_named_nets_including_no_connects: named_nets.len(),
        segments: count_head(pcb, "segment"),
        arcs: count_head(pcb, "arc"),
        vias: count_head(pcb, "via"),
        zones: zones.len(),
        copper_zones: zones.len() - rule_areas,
        rule_areas,
        copper_graphics: pcb
            .children()
            .iter()
            .filter(|item| is_top_level_copper_graphic(item))
            .count(),
        stored_segment_length_mm,
        physical_copper: physical_copper_statistics(pcb, stored_segment_length_mm)?,
        copper_geometry_sha256: copper_geometry_sha256(pcb),
        component_placement_sha256: component_placement_sha256(pcb)?,
        component_placement_100nm_sha256: component_placement_quantized_sha256(pcb)?,
    })
}

fn is_rule_area(zone: &Expr) -> bool {
    zone.head() == Some("zone") && zone.child("keepout").is_some()
}

fn is_top_level_copper_graphic(item: &Expr) -> bool {
    matches!(
        item.head(),
        Some(
            "gr_text"
                | "gr_text_box"
                | "gr_line"
                | "gr_rect"
                | "gr_arc"
                | "gr_poly"
                | "gr_curve"
                | "dimension"
        )
    ) && form_atom(item, "layer", 1).is_some_and(|layer| layer.ends_with(".Cu"))
}

#[derive(Default)]
struct MeasuredCopperLayer {
    stored_straight_track_length_mm: f64,
    canonical_straight_track_length_mm: f64,
    arc_track_length_mm: f64,
    centerline_weighted_track_area_mm2: f64,
    vias_spanning_layer: usize,
    via_annulus_area_mm2: f64,
    filled_zone_area_mm2: f64,
}

fn physical_copper_statistics(
    pcb: &Expr,
    stored_segment_length_mm: f64,
) -> Result<KiCadPhysicalCopperStatistics, String> {
    let copper_layers = board_copper_layers(pcb);
    let mut layers = copper_layers
        .iter()
        .cloned()
        .map(|layer| (layer, MeasuredCopperLayer::default()))
        .collect::<BTreeMap<_, _>>();
    let mut branches_by_net = BTreeMap::<String, Vec<(KiCadRouteBranch, f64)>>::new();
    for (ordinal, item) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("segment"))
        .enumerate()
    {
        let start = form_xy(item, "start")?;
        let end = form_xy(item, "end")?;
        let width = form_f64(item, "width", 1)?;
        if !width.is_finite() || width <= 0.0 || start == end {
            return Err(format!("segment {ordinal} has invalid geometry"));
        }
        let layer = form_atom(item, "layer", 1)
            .ok_or_else(|| format!("segment {ordinal} has no layer"))?
            .to_string();
        let net = node_net(item)
            .map(normalize_net)
            .filter(|net| !net.is_empty())
            .ok_or_else(|| format!("segment {ordinal} has no named net"))?
            .to_string();
        let length = distance_squared(start, end).sqrt();
        layers
            .entry(layer.clone())
            .or_default()
            .stored_straight_track_length_mm += length;
        branches_by_net.entry(net).or_default().push((
            KiCadRouteBranch {
                start_terminal: start,
                finish_terminal: end,
                cost: 0,
                expansions: 0,
                path: vec![
                    KiCadRoutePoint {
                        at: start,
                        layer: layer.clone(),
                    },
                    KiCadRoutePoint { at: end, layer },
                ],
            },
            width,
        ));
    }

    let mut canonical_straight_track_length_mm = 0.0;
    let mut centerline_weighted_track_area_mm2 = 0.0;
    for branches in branches_by_net.into_values() {
        let mut route_branches = branches
            .iter()
            .map(|(branch, _)| branch.clone())
            .collect::<Vec<_>>();
        let selected = (0..route_branches.len()).collect::<Vec<_>>();
        normalize_selected_route_graph(&mut route_branches, &selected);
        let mut edges = BTreeMap::<String, ([f64; 2], [f64; 2], String, f64)>::new();
        for (branch_index, branch) in route_branches.iter().enumerate() {
            let width = branches[branch_index].1;
            for pair in branch.path.windows(2) {
                if pair[0].layer != pair[1].layer || pair[0].at == pair[1].at {
                    continue;
                }
                let (start, end) = ordered_points(pair[0].at, pair[1].at);
                let key = format!(
                    "{}:{:.9}:{:.9}:{:.9}:{:.9}",
                    pair[0].layer, start[0], start[1], end[0], end[1]
                );
                edges
                    .entry(key)
                    .and_modify(|edge| edge.3 = edge.3.max(width))
                    .or_insert((start, end, pair[0].layer.clone(), width));
            }
        }
        for (_, (start, end, layer, width)) in edges {
            let length = distance_squared(start, end).sqrt();
            canonical_straight_track_length_mm += length;
            centerline_weighted_track_area_mm2 += length * width;
            let layer = layers.entry(layer).or_default();
            layer.canonical_straight_track_length_mm += length;
            layer.centerline_weighted_track_area_mm2 += length * width;
        }
    }

    let mut arc_track_length_mm = 0.0;
    let mut arc_count = 0;
    for (ordinal, item) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("arc"))
        .enumerate()
    {
        let length = track_arc_length(item)
            .map_err(|error| format!("failed to measure track arc {ordinal}: {error}"))?;
        let width = form_f64(item, "width", 1)?;
        if !width.is_finite() || width <= 0.0 {
            return Err(format!("track arc {ordinal} has invalid width"));
        }
        let layer = form_atom(item, "layer", 1)
            .ok_or_else(|| format!("track arc {ordinal} has no layer"))?
            .to_string();
        arc_count += 1;
        arc_track_length_mm += length;
        centerline_weighted_track_area_mm2 += length * width;
        let layer = layers.entry(layer).or_default();
        layer.arc_track_length_mm += length;
        layer.centerline_weighted_track_area_mm2 += length * width;
    }

    let mut via_projected_outer_area_mm2 = 0.0;
    let mut via_drill_area_mm2 = 0.0;
    let mut layer_weighted_via_annulus_area_mm2 = 0.0;
    for (ordinal, item) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("via"))
        .enumerate()
    {
        let diameter = form_f64(item, "size", 1)?;
        let drill = form_f64(item, "drill", 1)?;
        if !diameter.is_finite()
            || !drill.is_finite()
            || diameter <= 0.0
            || drill <= 0.0
            || drill >= diameter
        {
            return Err(format!("via {ordinal} has invalid diameter/drill"));
        }
        let outer_area = std::f64::consts::PI * (diameter / 2.0).powi(2);
        let drill_area = std::f64::consts::PI * (drill / 2.0).powi(2);
        let annulus_area = outer_area - drill_area;
        via_projected_outer_area_mm2 += outer_area;
        via_drill_area_mm2 += drill_area;
        let spanned = via_spanned_layers(item, &copper_layers)?;
        layer_weighted_via_annulus_area_mm2 += annulus_area * spanned.len() as f64;
        for layer in spanned {
            let layer = layers.entry(layer).or_default();
            layer.vias_spanning_layer += 1;
            layer.via_annulus_area_mm2 += annulus_area;
        }
    }

    let mut filled_zone_area_mm2 = 0.0;
    for zone in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
    {
        for polygon in zone
            .children()
            .iter()
            .filter(|item| item.head() == Some("filled_polygon"))
        {
            let layer = form_atom(polygon, "layer", 1)
                .or_else(|| form_atom(zone, "layer", 1))
                .ok_or_else(|| "filled zone polygon has no layer".to_string())?
                .to_string();
            let area = polygon_area(polygon)?;
            filled_zone_area_mm2 += area;
            layers.entry(layer).or_default().filled_zone_area_mm2 += area;
        }
    }

    let physical_centerline_length_mm = canonical_straight_track_length_mm + arc_track_length_mm;
    let primitive_copper_area_mm2 = centerline_weighted_track_area_mm2
        + layer_weighted_via_annulus_area_mm2
        + filled_zone_area_mm2;
    let mut limitations = vec![
        "area is a primitive sum: track end caps and overlaps between tracks, vias, and zones are not unioned"
            .into(),
    ];
    if arc_count > 0 {
        limitations.push(
            "arc centerlines are measured exactly but are not unioned against straight or other arc tracks"
                .into(),
        );
    }
    let mut layer_statistics = layers
        .into_iter()
        .map(|(layer, measured)| {
            let physical_centerline_length_mm =
                measured.canonical_straight_track_length_mm + measured.arc_track_length_mm;
            let primitive_copper_area_mm2 = measured.centerline_weighted_track_area_mm2
                + measured.via_annulus_area_mm2
                + measured.filled_zone_area_mm2;
            KiCadCopperLayerStatistics {
                layer,
                stored_straight_track_length_mm: measured.stored_straight_track_length_mm,
                canonical_straight_track_length_mm: measured.canonical_straight_track_length_mm,
                arc_track_length_mm: measured.arc_track_length_mm,
                physical_centerline_length_mm,
                centerline_weighted_track_area_mm2: measured.centerline_weighted_track_area_mm2,
                vias_spanning_layer: measured.vias_spanning_layer,
                via_annulus_area_mm2: measured.via_annulus_area_mm2,
                filled_zone_area_mm2: measured.filled_zone_area_mm2,
                primitive_copper_area_mm2,
            }
        })
        .collect::<Vec<_>>();
    layer_statistics.sort_by(|left, right| {
        let index = |layer: &str| {
            copper_layers
                .iter()
                .position(|candidate| candidate == layer)
                .unwrap_or(usize::MAX)
        };
        index(&left.layer)
            .cmp(&index(&right.layer))
            .then_with(|| left.layer.cmp(&right.layer))
    });
    let used_copper_layers = layer_statistics
        .iter()
        .filter(|layer| {
            layer.physical_centerline_length_mm > 0.0
                || layer.vias_spanning_layer > 0
                || layer.filled_zone_area_mm2 > 0.0
        })
        .count();
    Ok(KiCadPhysicalCopperStatistics {
        schema_version: 1,
        canonical_straight_track_length_mm,
        arc_track_length_mm,
        physical_centerline_length_mm,
        centerline_union_exact: arc_count == 0,
        overlapping_straight_track_length_mm: (stored_segment_length_mm
            - canonical_straight_track_length_mm)
            .max(0.0),
        centerline_weighted_track_area_mm2,
        via_projected_outer_area_mm2,
        via_drill_area_mm2,
        layer_weighted_via_annulus_area_mm2,
        filled_zone_area_mm2,
        primitive_copper_area_mm2,
        board_outline_area_mm2: board_outline_area(pcb)?,
        used_copper_layers,
        layers: layer_statistics,
        limitations,
    })
}

fn ordered_points(left: [f64; 2], right: [f64; 2]) -> ([f64; 2], [f64; 2]) {
    if left[0].total_cmp(&right[0]).is_lt()
        || (left[0] == right[0] && left[1].total_cmp(&right[1]).is_le())
    {
        (left, right)
    } else {
        (right, left)
    }
}

fn board_copper_layers(pcb: &Expr) -> Vec<String> {
    let mut layers = Vec::new();
    let push = |layers: &mut Vec<String>, layer: &str| {
        if layer.ends_with(".Cu") && !layers.iter().any(|existing| existing == layer) {
            layers.push(layer.to_string());
        }
    };
    if let Some(stack) = pcb.child("layers") {
        for entry in stack.children().iter().skip(1) {
            if let Some(layer) = entry.children().get(1).and_then(Expr::atom) {
                push(&mut layers, layer);
            }
        }
    }
    for item in pcb.children() {
        if matches!(item.head(), Some("segment" | "arc" | "zone"))
            && let Some(layer) = form_atom(item, "layer", 1)
        {
            push(&mut layers, layer);
        }
        if item.head() == Some("via")
            && let Some(via_layers) = item.child("layers")
        {
            for layer in via_layers.children().iter().skip(1).filter_map(Expr::atom) {
                if layer != "*.Cu" {
                    push(&mut layers, layer);
                }
            }
        }
    }
    layers
}

fn via_spanned_layers(item: &Expr, copper_layers: &[String]) -> Result<Vec<String>, String> {
    let declared = item
        .child("layers")
        .ok_or_else(|| "via has no layers".to_string())?
        .children()
        .iter()
        .skip(1)
        .filter_map(Expr::atom)
        .collect::<Vec<_>>();
    if declared.contains(&"*.Cu") {
        if copper_layers.is_empty() {
            return Err("through via uses *.Cu but the board has no copper stack".into());
        }
        return Ok(copper_layers.to_vec());
    }
    if declared.len() == 2 {
        let start = copper_layers
            .iter()
            .position(|layer| layer == declared[0])
            .ok_or_else(|| format!("via references unknown copper layer {:?}", declared[0]))?;
        let end = copper_layers
            .iter()
            .position(|layer| layer == declared[1])
            .ok_or_else(|| format!("via references unknown copper layer {:?}", declared[1]))?;
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        return Ok(copper_layers[start..=end].to_vec());
    }
    if declared.is_empty() {
        return Err("via has an empty layer span".into());
    }
    declared
        .into_iter()
        .map(|layer| {
            copper_layers
                .iter()
                .find(|candidate| candidate.as_str() == layer)
                .cloned()
                .ok_or_else(|| format!("via references unknown copper layer {layer:?}"))
        })
        .collect()
}

fn track_arc_length(item: &Expr) -> Result<f64, String> {
    let start = form_xy(item, "start")?;
    let middle = form_xy(item, "mid")?;
    let end = form_xy(item, "end")?;
    let denominator = 2.0
        * (start[0] * (middle[1] - end[1])
            + middle[0] * (end[1] - start[1])
            + end[0] * (start[1] - middle[1]));
    if !denominator.is_finite() || denominator.abs() <= 1.0e-12 {
        return Err("start/mid/end are collinear".into());
    }
    let start_norm = start[0] * start[0] + start[1] * start[1];
    let middle_norm = middle[0] * middle[0] + middle[1] * middle[1];
    let end_norm = end[0] * end[0] + end[1] * end[1];
    let center = [
        (start_norm * (middle[1] - end[1])
            + middle_norm * (end[1] - start[1])
            + end_norm * (start[1] - middle[1]))
            / denominator,
        (start_norm * (end[0] - middle[0])
            + middle_norm * (start[0] - end[0])
            + end_norm * (middle[0] - start[0]))
            / denominator,
    ];
    let radius = distance_squared(start, center).sqrt();
    if !radius.is_finite() || radius <= 0.0 {
        return Err("arc has invalid radius".into());
    }
    let angle = |left: [f64; 2], right: [f64; 2]| {
        let left = [left[0] - center[0], left[1] - center[1]];
        let right = [right[0] - center[0], right[1] - center[1]];
        ((left[0] * right[0] + left[1] * right[1]) / (radius * radius))
            .clamp(-1.0, 1.0)
            .acos()
    };
    Ok(radius * (angle(start, middle) + angle(middle, end)))
}

fn polygon_area(polygon: &Expr) -> Result<f64, String> {
    let points = polygon
        .child("pts")
        .ok_or_else(|| "filled polygon has no pts".to_string())?
        .children()
        .iter()
        .skip(1)
        .filter(|point| point.head() == Some("xy"))
        .map(|point| {
            let coordinate = |index: usize| {
                point
                    .children()
                    .get(index)
                    .and_then(Expr::atom)
                    .ok_or_else(|| "xy point is missing a coordinate".to_string())?
                    .parse::<f64>()
                    .map_err(|error| format!("invalid xy coordinate: {error}"))
            };
            Ok::<[f64; 2], String>([coordinate(1)?, coordinate(2)?])
        })
        .collect::<Result<Vec<_>, String>>()?;
    if points.len() < 3 {
        return Err("filled polygon has fewer than three points".into());
    }
    let twice_area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
        .sum::<f64>();
    Ok(twice_area.abs() / 2.0)
}

fn board_outline_area(pcb: &Expr) -> Result<Option<f64>, String> {
    Ok(board_outline(pcb)?.map(|outline| outline.area()))
}

#[cfg(test)]
fn rectangular_board_outline_area(pcb: &Expr) -> Result<Option<f64>, String> {
    let Some(outline) = board_outline(pcb)? else {
        return Ok(None);
    };
    let bounds_area =
        (outline.bounds[2] - outline.bounds[0]) * (outline.bounds[3] - outline.bounds[1]);
    Ok(((outline.area() - bounds_area).abs() <= 1.0e-6).then_some(bounds_area))
}

#[cfg(test)]
fn rectangular_board_bounds(pcb: &Expr) -> Result<Option<[f64; 4]>, String> {
    let Some(outline) = board_outline(pcb)? else {
        return Ok(None);
    };
    let bounds_area =
        (outline.bounds[2] - outline.bounds[0]) * (outline.bounds[3] - outline.bounds[1]);
    Ok(((outline.area() - bounds_area).abs() <= 1.0e-6).then_some(outline.bounds))
}

fn board_outline(pcb: &Expr) -> Result<Option<BoardOutline>, String> {
    let edge_shapes = pcb
        .children()
        .iter()
        .filter(|item| {
            matches!(
                item.head(),
                Some("gr_rect" | "gr_line" | "gr_arc" | "gr_poly" | "gr_curve")
            ) && form_atom(item, "layer", 1) == Some("Edge.Cuts")
        })
        .collect::<Vec<_>>();
    if edge_shapes.len() == 1 && edge_shapes[0].head() == Some("gr_rect") {
        let start = form_xy(edge_shapes[0], "start")?;
        let end = form_xy(edge_shapes[0], "end")?;
        return Ok(Some(BoardOutline::rectangle([
            start[0].min(end[0]),
            start[1].min(end[1]),
            start[0].max(end[0]),
            start[1].max(end[1]),
        ])?));
    }
    if edge_shapes.len() < 4
        || edge_shapes
            .iter()
            .any(|shape| shape.head() != Some("gr_line"))
    {
        return Ok(None);
    }
    let mut segments = Vec::with_capacity(edge_shapes.len());
    for shape in edge_shapes {
        let start = form_xy(shape, "start")?;
        let end = form_xy(shape, "end")?;
        if distance_squared(start, end) <= 1.0e-18 {
            return Ok(None);
        }
        segments.push((start, end));
    }
    let point = |coordinate: [f64; 2]| {
        Point(
            (coordinate[0] * 1_000_000.0).round() as i64,
            (coordinate[1] * 1_000_000.0).round() as i64,
        )
    };
    let segments = segments
        .into_iter()
        .map(|(start, end)| (point(start), point(end)))
        .collect::<Vec<_>>();
    let mut adjacency: BTreeMap<Point, Vec<Point>> = BTreeMap::new();
    for (start, end) in segments {
        adjacency.entry(start).or_default().push(end);
        adjacency.entry(end).or_default().push(start);
    }
    if adjacency.len() < 3 || adjacency.values().any(|neighbors| neighbors.len() != 2) {
        return Ok(None);
    }
    let start = *adjacency
        .keys()
        .next()
        .expect("non-empty outline adjacency");
    let mut previous = None;
    let mut current = start;
    let mut points = Vec::with_capacity(adjacency.len());
    loop {
        points.push([
            current.0 as f64 / 1_000_000.0,
            current.1 as f64 / 1_000_000.0,
        ]);
        let neighbors = &adjacency[&current];
        let next = if previous == Some(neighbors[0]) {
            neighbors[1]
        } else {
            neighbors[0]
        };
        previous = Some(current);
        current = next;
        if current == start {
            break;
        }
        if points.len() >= adjacency.len() {
            return Ok(None);
        }
    }
    if points.len() != adjacency.len() {
        return Ok(None);
    }
    Ok(Some(BoardOutline::new(points)?))
}

fn component_placement_records(pcb: &Expr) -> Result<Vec<(String, [f64; 3])>, String> {
    let mut records = Vec::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let declared_reference =
            footprint_reference(footprint).filter(|reference| !reference.is_empty());
        let footprint_name = footprint
            .children()
            .get(1)
            .and_then(Expr::atom)
            .unwrap_or("unnamed-footprint");
        let diagnostic_name = declared_reference.as_deref().unwrap_or(footprint_name);
        let at = footprint
            .child("at")
            .ok_or_else(|| format!("footprint {diagnostic_name:?} has no at form"))?;
        let fields = at.children();
        let number = |index: usize, label: &str| {
            fields
                .get(index)
                .and_then(Expr::atom)
                .ok_or_else(|| format!("footprint {diagnostic_name:?} has no {label}"))?
                .parse::<f64>()
                .map_err(|error| {
                    format!("footprint {diagnostic_name:?} has invalid {label}: {error}")
                })
        };
        let x = number(1, "x position")?;
        let y = number(2, "y position")?;
        let rotation = fields
            .get(3)
            .and_then(Expr::atom)
            .unwrap_or("0")
            .parse::<f64>()
            .map_err(|error| {
                format!("footprint {diagnostic_name:?} has invalid rotation: {error}")
            })?
            .rem_euclid(360.0);
        let reference = declared_reference
            .unwrap_or_else(|| format!("@anonymous:{footprint_name}:{x:.9}:{y:.9}:{rotation:.9}"));
        records.push((reference, [x, y, rotation]));
    }
    records.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(records)
}

fn component_placement_sha256(pcb: &Expr) -> Result<String, String> {
    let records = component_placement_records(pcb)?;
    let mut hash = Sha256::new();
    for (reference, pose) in records {
        hash.update((reference.len() as u64).to_le_bytes());
        hash.update(reference.as_bytes());
        for value in pose {
            let bits = if value == 0.0 {
                0.0_f64.to_bits()
            } else {
                value.to_bits()
            };
            hash.update(bits.to_le_bytes());
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn component_placement_quantized_sha256(pcb: &Expr) -> Result<String, String> {
    let records = component_placement_records(pcb)?;
    let mut hash = Sha256::new();
    for (reference, pose) in records {
        hash.update((reference.len() as u64).to_le_bytes());
        hash.update(reference.as_bytes());
        for value in pose {
            let quantized = (value * 10_000.0).round() as i64;
            hash.update(quantized.to_le_bytes());
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn copper_geometry_sha256(pcb: &Expr) -> String {
    let mut records: Vec<_> = pcb
        .children()
        .iter()
        .filter(|item| {
            matches!(item.head(), Some("segment" | "arc" | "via" | "zone"))
                || is_top_level_copper_graphic(item)
        })
        .filter_map(without_volatile_identity)
        .map(|item| encode(&item))
        .collect();
    records.sort();
    let mut hash = Sha256::new();
    for record in records {
        hash.update((record.len() as u64).to_le_bytes());
        hash.update(record.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn without_volatile_identity(expression: &Expr) -> Option<Expr> {
    if matches!(expression.head(), Some("uuid" | "tstamp")) {
        return None;
    }
    Some(match expression {
        Expr::Atom(atom) => Expr::Atom(atom.clone()),
        Expr::List(items) => {
            Expr::List(items.iter().filter_map(without_volatile_identity).collect())
        }
    })
}

fn collect_named_nets(root: &Expr, named_nets: &mut BTreeSet<String>) {
    if root.head() == Some("net")
        && let Some(name) = net_name_from_form(root)
        && !name.is_empty()
        && name.parse::<u32>().is_err()
    {
        named_nets.insert(normalize_net(name).to_string());
    }
    for child in root.children() {
        collect_named_nets(child, named_nets);
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct VerificationReport {
    pub board_id: String,
    pub complete: bool,
    pub erc_violations: usize,
    pub drc_design_violations: usize,
    pub schematic_parity_issues: usize,
    pub selected_net_unconnected_items: usize,
    pub intentional_no_connect_groups: usize,
    pub library_metadata_warnings: usize,
}

pub const KICAD_VERIFICATION_CACHE_ENV: &str = "PCB_MAKER_KICAD_VERIFICATION_CACHE";
pub const KICAD_VERIFICATION_CACHE_EVENTS_ENV: &str = "PCB_MAKER_KICAD_VERIFICATION_CACHE_EVENTS";
const KICAD_VERIFICATION_CACHE_SCHEMA_VERSION: u32 = 1;
const KICAD_VERIFICATION_POLICY: &str =
    "verify-materialized-rung-v1:erc-all:drc-all:schematic-parity:intentional-no-connect-groups";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadVerificationCacheStatus {
    Hit,
    Miss,
    Bypassed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadVerificationCacheTrace {
    pub schema_version: u32,
    pub status: KiCadVerificationCacheStatus,
    pub key: Option<String>,
    pub input_sha256: Option<String>,
    pub kicad_cli_version: String,
    pub verifier_sha256: String,
    pub policy: String,
    pub refill_zones: bool,
    pub cache_root: PathBuf,
    pub cache_entry: Option<PathBuf>,
    pub elapsed_micros: u64,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct KiCadVerificationCacheEntry {
    schema_version: u32,
    key: String,
    input_sha256: String,
    kicad_cli_version: String,
    verifier_sha256: String,
    policy: String,
    refill_zones: bool,
    erc_sha256: String,
    drc_sha256: String,
    report: VerificationReport,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadLayerRenderConfig {
    pub width: u32,
    pub height: u32,
    pub quality: String,
    pub background: String,
    /// Include component models and markings. Comparisons default to hiding
    /// these so they cannot obscure copper; pads and board geometry remain.
    pub show_components: bool,
    /// Add outline-only silkscreen geometry to a disposable render input so
    /// native 3D renders expose non-physical rule-area constraints.
    pub show_rule_areas: bool,
    /// Label and outline copper zones on the disposable render input. This is
    /// opt-in because plane-heavy production boards would otherwise be noisy.
    pub show_copper_zones: bool,
}

impl Default for KiCadLayerRenderConfig {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 750,
            quality: "basic".into(),
            background: "opaque".into(),
            show_components: false,
            show_rule_areas: true,
            show_copper_zones: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadLayerRenderReport {
    pub front: PathBuf,
    pub back: PathBuf,
    pub config: KiCadLayerRenderConfig,
    pub rule_area_overlays: usize,
    pub copper_zone_overlays: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadZonePadConnection {
    Thermal,
    Solid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadGroundZoneConfig {
    pub connection: String,
    pub layers: Vec<String>,
    pub board_inset_mm: f64,
    pub clearance_mm: f64,
    pub minimum_thickness_mm: f64,
    pub thermal_gap_mm: f64,
    pub thermal_bridge_width_mm: f64,
    pub pad_connection: KiCadZonePadConnection,
    pub retain_existing_vias: bool,
    /// When retaining vias, keep only the first target-net via in each board-
    /// anchored square cell. `None` retains the complete existing via set.
    #[serde(default)]
    pub via_thinning_cell_size_mm: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadGroundZoneEvidence {
    pub schema_version: u32,
    pub algorithm: String,
    pub board_id: String,
    pub connection: String,
    pub source_directory: PathBuf,
    pub output_directory: PathBuf,
    pub board_bounds: [f64; 4],
    pub zone_bounds: [f64; 4],
    pub layers: Vec<String>,
    pub pad_connection: KiCadZonePadConnection,
    pub via_thinning_cell_size_mm: Option<f64>,
    pub removed_segments: usize,
    pub removed_arcs: usize,
    pub removed_vias: usize,
    pub retained_vias: usize,
    pub removed_zones: usize,
    pub modified_pads: usize,
    pub added_zones: usize,
    pub fill_authority: String,
    pub source_verification: VerificationReport,
    pub proposal_verification: VerificationReport,
    pub complete: bool,
    pub front_render: PathBuf,
    pub back_render: PathBuf,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KiCadGroundZoneReplacement {
    pub board_bounds: [f64; 4],
    pub zone_bounds: [f64; 4],
    pub removed_segments: usize,
    pub removed_arcs: usize,
    pub removed_vias: usize,
    pub retained_vias: usize,
    pub removed_zones: usize,
    pub modified_pads: usize,
    pub added_zones: usize,
}

/// Renders deterministic top and bottom journal images through KiCad's native
/// board renderer. These images are inspection evidence, not a replacement
/// for ERC, DRC, parity, or connectivity verification.
pub fn render_board_layers(
    pcb_path: &Path,
    output_prefix: &Path,
    config: &KiCadLayerRenderConfig,
) -> Result<KiCadLayerRenderReport, String> {
    validate_layer_render_config(config)?;
    if !pcb_path.is_file() {
        return Err(format!("KiCad board {} does not exist", pcb_path.display()));
    }
    if let Some(parent) = output_prefix
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create render directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let front = PathBuf::from(format!("{}-front.png", output_prefix.display()));
    let back = PathBuf::from(format!("{}-back.png", output_prefix.display()));
    let render_input = PathBuf::from(format!(
        "{}-inspection-{}-{}.kicad_pcb",
        output_prefix.display(),
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?
            .as_nanos()
    ));
    let overlay_counts =
        if !config.show_components || config.show_rule_areas || config.show_copper_zones {
            prepare_layer_render_board(pcb_path, &render_input, config)?
        } else {
            KiCadLayerRenderOverlayCounts::default()
        };
    let overlay_count = overlay_counts.rule_areas + overlay_counts.copper_zones;
    let disposable_input = !config.show_components || overlay_count != 0;
    let rendered_pcb = if !disposable_input {
        pcb_path
    } else {
        render_input.as_path()
    };
    let render_result = run_kicad_render(rendered_pcb, &front, "top", config)
        .and_then(|()| run_kicad_render(rendered_pcb, &back, "bottom", config));
    if disposable_input {
        let cleanup_result = fs::remove_file(&render_input).map_err(|error| {
            format!(
                "failed to remove disposable render input {}: {error}",
                render_input.display()
            )
        });
        render_result?;
        cleanup_result?;
    } else {
        render_result?;
    }
    Ok(KiCadLayerRenderReport {
        front,
        back,
        config: config.clone(),
        rule_area_overlays: overlay_counts.rule_areas,
        copper_zone_overlays: overlay_counts.copper_zones,
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct KiCadLayerRenderOverlayCounts {
    rule_areas: usize,
    copper_zones: usize,
}

fn prepare_layer_render_board(
    source: &Path,
    destination: &Path,
    config: &KiCadLayerRenderConfig,
) -> Result<KiCadLayerRenderOverlayCounts, String> {
    let source_text = fs::read_to_string(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
    let mut pcb = parse(&source_text)
        .map_err(|error| format!("failed to parse {}: {error}", source.display()))?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!("{} is not a kicad_pcb document", source.display()));
    }
    let mut overlays = Vec::new();
    let mut counts = KiCadLayerRenderOverlayCounts::default();
    for (zone_index, zone) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
        .enumerate()
    {
        let rule_area = is_rule_area(zone);
        if (rule_area && !config.show_rule_areas) || (!rule_area && !config.show_copper_zones) {
            continue;
        }
        let Ok(layer_mask) = rule_area_layer_mask(zone) else {
            continue;
        };
        let Ok(points) = rule_area_polygon_points(zone) else {
            continue;
        };
        for (layer_index, enabled) in layer_mask.into_iter().enumerate() {
            if enabled {
                let layer = if layer_index == 0 {
                    "F.SilkS"
                } else {
                    "B.SilkS"
                };
                overlays.push(zone_render_outline_expr(
                    &points, layer, zone_index, rule_area,
                ));
                if rule_area {
                    counts.rule_areas += 1;
                } else {
                    overlays.push(copper_zone_render_label_expr(
                        &points,
                        layer,
                        zone_index,
                        node_net(zone)
                            .or_else(|| form_atom(zone, "net_name", 1))
                            .unwrap_or("unnamed"),
                    ));
                    counts.copper_zones += 1;
                }
            }
        }
    }
    if overlays.is_empty() && config.show_components {
        return Ok(counts);
    }
    let Expr::List(items) = &mut pcb else {
        unreachable!("kicad_pcb root must be a list")
    };
    if !config.show_components {
        // Only the disposable visualization copy loses graphics. Footprints,
        // pads, copper, holes, outline and source project stay intact.
        let is_marking = |item: &Expr| {
            matches!(
                form_atom(item, "layer", 1),
                Some("F.SilkS" | "B.SilkS" | "F.Fab" | "B.Fab")
            )
        };
        items.retain(|item| !is_marking(item));
        for item in items.iter_mut() {
            if matches!(item.head(), Some("footprint" | "module")) {
                let Expr::List(children) = item else {
                    unreachable!()
                };
                children.retain(|child| child.head() != Some("model") && !is_marking(child));
            }
        }
    }
    items.extend(overlays);
    fs::write(destination, format!("{}\n", encode(&pcb))).map_err(|error| {
        format!(
            "failed to write disposable render input {}: {error}",
            destination.display()
        )
    })?;
    Ok(counts)
}

fn zone_render_outline_expr(
    points: &[[f64; 2]],
    layer: &str,
    index: usize,
    rule_area: bool,
) -> Expr {
    let mut point_items = vec![Expr::Atom("pts".into())];
    point_items.extend(
        points
            .iter()
            .copied()
            .map(|point| coordinate_form("xy", point)),
    );
    Expr::List(vec![
        Expr::Atom("gr_poly".into()),
        Expr::List(point_items),
        Expr::List(vec![
            Expr::Atom("stroke".into()),
            Expr::List(vec![Expr::Atom("width".into()), Expr::Atom("0.4".into())]),
            Expr::List(vec![
                Expr::Atom("type".into()),
                Expr::Atom(if rule_area { "dash" } else { "solid" }.into()),
            ]),
        ]),
        Expr::List(vec![Expr::Atom("fill".into()), Expr::Atom("no".into())]),
        string_form("layer", layer),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(quote(&deterministic_uuid_for(&format!(
                "pcb-maker:zone-render-outline:{rule_area}:{layer}:{index}:{points:?}"
            )))),
        ]),
    ])
}

fn copper_zone_render_label_expr(
    points: &[[f64; 2]],
    layer: &str,
    index: usize,
    net_name: &str,
) -> Expr {
    let centroid = points.iter().fold([0.0, 0.0], |sum, point| {
        [sum[0] + point[0], sum[1] + point[1]]
    });
    let centroid = [
        centroid[0] / points.len() as f64,
        centroid[1] / points.len() as f64,
    ];
    let mut effects = vec![
        Expr::Atom("effects".into()),
        Expr::List(vec![
            Expr::Atom("font".into()),
            Expr::List(vec![
                Expr::Atom("size".into()),
                Expr::Atom("1.2".into()),
                Expr::Atom("1.2".into()),
            ]),
            Expr::List(vec![
                Expr::Atom("thickness".into()),
                Expr::Atom("0.2".into()),
            ]),
        ]),
    ];
    if layer == "B.SilkS" {
        effects.push(Expr::List(vec![
            Expr::Atom("justify".into()),
            Expr::Atom("mirror".into()),
        ]));
    }
    Expr::List(vec![
        Expr::Atom("gr_text".into()),
        Expr::Atom(quote(&format!("COPPER ZONE {net_name}"))),
        coordinate_form("at", centroid),
        string_form("layer", layer),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(quote(&deterministic_uuid_for(&format!(
                "pcb-maker:copper-zone-render-label:{layer}:{index}:{net_name}:{centroid:?}"
            )))),
        ]),
        Expr::List(effects),
    ])
}

fn validate_layer_render_config(config: &KiCadLayerRenderConfig) -> Result<(), String> {
    if !(1..=8192).contains(&config.width) || !(1..=8192).contains(&config.height) {
        return Err("KiCad render width and height must be within 1..=8192".into());
    }
    if !matches!(
        config.quality.as_str(),
        "basic" | "high" | "user" | "job_settings"
    ) {
        return Err(format!(
            "unsupported KiCad render quality {:?}",
            config.quality
        ));
    }
    if !matches!(
        config.background.as_str(),
        "default" | "transparent" | "opaque"
    ) {
        return Err(format!(
            "unsupported KiCad render background {:?}",
            config.background
        ));
    }
    Ok(())
}

fn run_kicad_render(
    pcb_path: &Path,
    output_path: &Path,
    side: &str,
    config: &KiCadLayerRenderConfig,
) -> Result<(), String> {
    let result = Command::new("kicad-cli")
        .args([
            "pcb",
            "render",
            "--output",
            &output_path.to_string_lossy(),
            "--width",
            &config.width.to_string(),
            "--height",
            &config.height.to_string(),
            "--side",
            side,
            "--background",
            &config.background,
            "--quality",
            &config.quality,
            &pcb_path.to_string_lossy(),
        ])
        .output()
        .map_err(|error| format!("failed to run kicad-cli PCB renderer: {error}"))?;
    if !result.status.success() || !output_path.is_file() {
        return Err(format!(
            "kicad-cli failed to render {side} image {} with {}: {}{}",
            output_path.display(),
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(())
}

fn validate_ground_zone_config(config: &KiCadGroundZoneConfig) -> Result<(), String> {
    let layers = config.layers.iter().collect::<BTreeSet<_>>();
    if config.connection.is_empty()
        || config.layers.is_empty()
        || config.layers.len() != layers.len()
        || config
            .layers
            .iter()
            .any(|layer| copper_layer_index(layer).is_none())
        || !config.board_inset_mm.is_finite()
        || config.board_inset_mm <= 0.0
        || !config.clearance_mm.is_finite()
        || config.clearance_mm <= 0.0
        || !config.minimum_thickness_mm.is_finite()
        || config.minimum_thickness_mm <= 0.0
        || !config.thermal_gap_mm.is_finite()
        || config.thermal_gap_mm <= 0.0
        || !config.thermal_bridge_width_mm.is_finite()
        || config.thermal_bridge_width_mm <= 0.0
        || config.via_thinning_cell_size_mm.is_some_and(|cell_size| {
            !config.retain_existing_vias || !cell_size.is_finite() || cell_size <= 0.0
        })
    {
        return Err("invalid KiCad ground-zone configuration".into());
    }
    Ok(())
}

fn ground_zone_expr(
    connection: &str,
    layer: &str,
    bounds: [f64; 4],
    config: &KiCadGroundZoneConfig,
) -> Expr {
    let identity = format!("pcb-maker:ground-zone:{connection}:{layer}:{bounds:?}");
    Expr::List(vec![
        Expr::Atom("zone".into()),
        net_expr(&format!("/{connection}")),
        string_form("net_name", &format!("/{connection}")),
        string_form("layer", layer),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(quote(&deterministic_uuid_for(&identity))),
        ]),
        Expr::List(vec![
            Expr::Atom("hatch".into()),
            Expr::Atom("edge".into()),
            Expr::Atom("0.5".into()),
        ]),
        Expr::List(vec![
            Expr::Atom("connect_pads".into()),
            Expr::List(vec![
                Expr::Atom("clearance".into()),
                Expr::Atom(config.clearance_mm.to_string()),
            ]),
        ]),
        Expr::List(vec![
            Expr::Atom("min_thickness".into()),
            Expr::Atom(config.minimum_thickness_mm.to_string()),
        ]),
        Expr::List(vec![
            Expr::Atom("fill".into()),
            Expr::Atom("yes".into()),
            Expr::List(vec![
                Expr::Atom("thermal_gap".into()),
                Expr::Atom(config.thermal_gap_mm.to_string()),
            ]),
            Expr::List(vec![
                Expr::Atom("thermal_bridge_width".into()),
                Expr::Atom(config.thermal_bridge_width_mm.to_string()),
            ]),
        ]),
        Expr::List(vec![
            Expr::Atom("polygon".into()),
            Expr::List(vec![
                Expr::Atom("pts".into()),
                coordinate_form("xy", [bounds[0], bounds[1]]),
                coordinate_form("xy", [bounds[2], bounds[1]]),
                coordinate_form("xy", [bounds[2], bounds[3]]),
                coordinate_form("xy", [bounds[0], bounds[3]]),
            ]),
        ]),
    ])
}

fn replace_routed_connection_with_zones(
    pcb: &mut Expr,
    config: &KiCadGroundZoneConfig,
) -> Result<KiCadGroundZoneReplacement, String> {
    validate_ground_zone_config(config)?;
    let model = KiCadRoutingModel::from_pcb(pcb, &config.connection)?;
    let zone_bounds = [
        model.bounds[0] + config.board_inset_mm,
        model.bounds[1] + config.board_inset_mm,
        model.bounds[2] - config.board_inset_mm,
        model.bounds[3] - config.board_inset_mm,
    ];
    if zone_bounds[0] >= zone_bounds[2] || zone_bounds[1] >= zone_bounds[3] {
        return Err("ground-zone inset leaves no board area".into());
    }
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut removed = [0_usize; 4];
    let mut retained_vias = 0;
    let mut occupied_via_cells = BTreeSet::new();
    let mut invalid_target_via = false;
    items.retain(|item| {
        let target = node_net(item)
            .map(normalize_net)
            .is_some_and(|net| net == config.connection);
        let index = match item.head() {
            Some("segment") => Some(0),
            Some("arc") => Some(1),
            Some("via") => Some(2),
            Some("zone") => Some(3),
            _ => None,
        };
        if target && item.head() == Some("via") && config.retain_existing_vias {
            let retain = config.via_thinning_cell_size_mm.is_none_or(|cell_size| {
                let at = match form_xy(item, "at") {
                    Ok(at) => at,
                    Err(_) => {
                        invalid_target_via = true;
                        return false;
                    }
                };
                let cell = (
                    ((at[0] - model.bounds[0]) / cell_size).floor() as i64,
                    ((at[1] - model.bounds[1]) / cell_size).floor() as i64,
                );
                occupied_via_cells.insert(cell)
            });
            if retain {
                retained_vias += 1;
            } else {
                removed[2] += 1;
            }
            retain
        } else if target && let Some(index) = index {
            removed[index] += 1;
            false
        } else {
            true
        }
    });
    if invalid_target_via {
        return Err("target-net via is missing a valid coordinate".into());
    }
    let mut modified_pads = 0;
    if matches!(config.pad_connection, KiCadZonePadConnection::Solid) {
        for footprint in items
            .iter_mut()
            .filter(|item| item.head() == Some("footprint"))
        {
            let Expr::List(parts) = footprint else {
                continue;
            };
            for pad in parts.iter_mut().filter(|part| part.head() == Some("pad")) {
                if !node_net(pad)
                    .map(normalize_net)
                    .is_some_and(|net| net == config.connection)
                {
                    continue;
                }
                let Expr::List(pad_parts) = pad else {
                    continue;
                };
                pad_parts.retain(|part| part.head() != Some("zone_connect"));
                pad_parts.push(Expr::List(vec![
                    Expr::Atom("zone_connect".into()),
                    Expr::Atom("2".into()),
                ]));
                modified_pads += 1;
            }
        }
    }
    for layer in &config.layers {
        items.push(ground_zone_expr(
            &config.connection,
            layer,
            zone_bounds,
            config,
        ));
    }
    Ok(KiCadGroundZoneReplacement {
        board_bounds: model.bounds,
        zone_bounds,
        removed_segments: removed[0],
        removed_arcs: removed[1],
        removed_vias: removed[2],
        retained_vias,
        removed_zones: removed[3],
        modified_pads,
        added_zones: config.layers.len(),
    })
}

/// Replaces one routed connection with rectangular zones in a generated board
/// artifact. The caller remains responsible for refilling the zones and
/// exact-gating the resulting project through KiCad.
pub fn apply_kicad_connection_zone_replacement(
    board_path: &Path,
    config: &KiCadGroundZoneConfig,
) -> Result<KiCadGroundZoneReplacement, String> {
    if !board_path.is_file() {
        return Err(format!(
            "KiCad board {} does not exist",
            board_path.display()
        ));
    }
    validate_ground_zone_config(config)?;
    let board_source = fs::read_to_string(board_path)
        .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let mut pcb = parse(&board_source)
        .map_err(|error| format!("failed to parse {}: {error}", board_path.display()))?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!(
            "{} is not a kicad_pcb document",
            board_path.display()
        ));
    }
    let replacement = replace_routed_connection_with_zones(&mut pcb, config)?;
    let temporary_path = PathBuf::from(format!(
        "{}.pcb-maker-zone-{}-{}",
        board_path.display(),
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?
            .as_nanos()
    ));
    fs::write(&temporary_path, format!("{}\n", encode(&pcb))).map_err(|error| {
        format!(
            "failed to write zone-replacement board {}: {error}",
            temporary_path.display()
        )
    })?;
    if let Err(error) = fs::rename(&temporary_path, board_path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!(
            "failed to commit zone-replacement board {}: {error}",
            board_path.display()
        ));
    }
    Ok(replacement)
}

/// Replaces one routed connection with rectangular copper zones in a copied
/// complete-board transaction. KiCad's refilled-zone DRC and connectivity
/// report is the acceptance authority; the source directory is never edited.
pub fn replace_kicad_connection_with_zones(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadGroundZoneConfig,
) -> Result<KiCadGroundZoneEvidence, String> {
    if !source_directory.is_dir()
        || board_id.is_empty()
        || output_directory.exists()
        || output_directory.as_os_str().is_empty()
    {
        return Err("invalid KiCad ground-zone source, board ID, or existing output".into());
    }
    validate_ground_zone_config(config)?;
    if let Some(parent) = output_directory
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::create_dir(output_directory).map_err(|error| error.to_string())?;
    let source_name = PathBuf::from("source");
    let proposal_name = PathBuf::from("proposal");
    let source_copy = output_directory.join(&source_name);
    let proposal = output_directory.join(&proposal_name);
    copy_directory_tree(source_directory, &source_copy)?;
    copy_directory_tree(source_directory, &proposal)?;
    let source_verification = verify_materialized_rung(&source_copy, board_id)?;
    let board_name = format!("{board_id}.kicad_pcb");
    let proposal_board = proposal.join(&board_name);
    let board_source = fs::read_to_string(&proposal_board).map_err(|error| error.to_string())?;
    let mut pcb = parse(&board_source)?;
    let replacement = replace_routed_connection_with_zones(&mut pcb, config)?;
    fs::write(&proposal_board, format!("{}\n", encode(&pcb))).map_err(|error| error.to_string())?;
    let proposal_verification = verify_materialized_rung_with_zone_refill(&proposal, board_id)?;
    let render_prefix = proposal.join("ground-zone");
    let render = render_board_layers(
        &proposal_board,
        &render_prefix,
        &KiCadLayerRenderConfig::default(),
    )?;
    Ok(KiCadGroundZoneEvidence {
        schema_version: 1,
        algorithm: "testing-esp32-duts-rectangular-ground-zone-transaction-v1".into(),
        board_id: board_id.into(),
        connection: config.connection.clone(),
        source_directory: source_name,
        output_directory: proposal_name.clone(),
        board_bounds: replacement.board_bounds,
        zone_bounds: replacement.zone_bounds,
        layers: config.layers.clone(),
        pad_connection: config.pad_connection,
        via_thinning_cell_size_mm: config.via_thinning_cell_size_mm,
        removed_segments: replacement.removed_segments,
        removed_arcs: replacement.removed_arcs,
        removed_vias: replacement.removed_vias,
        retained_vias: replacement.retained_vias,
        removed_zones: replacement.removed_zones,
        modified_pads: replacement.modified_pads,
        added_zones: replacement.added_zones,
        fill_authority: "kicad-cli pcb drc --refill-zones".into(),
        source_verification,
        complete: proposal_verification.complete,
        proposal_verification,
        front_render: proposal_name.join(
            render
                .front
                .file_name()
                .ok_or_else(|| "front ground-zone render has no file name".to_string())?,
        ),
        back_render: proposal_name.join(
            render
                .back
                .file_name()
                .ok_or_else(|| "back ground-zone render has no file name".to_string())?,
        ),
    })
}

fn diagnostic_svg_number(value: f64) -> String {
    let mut formatted = format!("{value:.6}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    formatted
}

fn escape_diagnostic_svg(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn push_diagnostic_obstacle_svg(body: &mut String, obstacle: &CopperObstacle) {
    push_diagnostic_geometry_svg(body, &obstacle.geometry);
}

fn push_diagnostic_geometry_svg(body: &mut String, geometry: &ObstacleGeometry) {
    match geometry {
        ObstacleGeometry::Circle { center, radius } => body.push_str(&format!(
            r##"<circle cx="{}" cy="{}" r="{}" fill="#d44d5c" fill-opacity="0.28" stroke="#ff7b88" stroke-width="0.07"/>"##,
            diagnostic_svg_number(center[0]),
            diagnostic_svg_number(center[1]),
            diagnostic_svg_number(*radius)
        )),
        ObstacleGeometry::Segment { start, end, radius } => body.push_str(&format!(
            r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="#d44d5c" stroke-opacity="0.38" stroke-width="{}" stroke-linecap="round"/>"##,
            diagnostic_svg_number(start[0]),
            diagnostic_svg_number(start[1]),
            diagnostic_svg_number(end[0]),
            diagnostic_svg_number(end[1]),
            diagnostic_svg_number(radius * 2.0)
        )),
        ObstacleGeometry::Rectangle {
            center,
            half_size,
            angle_degrees,
        } => body.push_str(&format!(
            r##"<g transform="translate({} {}) rotate({})"><rect x="{}" y="{}" width="{}" height="{}" fill="#d44d5c" fill-opacity="0.28" stroke="#ff7b88" stroke-width="0.07"/></g>"##,
            diagnostic_svg_number(center[0]),
            diagnostic_svg_number(center[1]),
            diagnostic_svg_number(*angle_degrees),
            diagnostic_svg_number(-half_size[0]),
            diagnostic_svg_number(-half_size[1]),
            diagnostic_svg_number(half_size[0] * 2.0),
            diagnostic_svg_number(half_size[1] * 2.0)
        )),
        ObstacleGeometry::Polygon { points } => body.push_str(&format!(
            r##"<polygon points="{}" fill="#d44d5c" fill-opacity="0.28" stroke="#ff7b88" stroke-width="0.07"/>"##,
            points
                .iter()
                .map(|point| format!(
                    "{},{}",
                    diagnostic_svg_number(point[0]),
                    diagnostic_svg_number(point[1])
                ))
                .collect::<Vec<_>>()
                .join(" ")
        )),
        ObstacleGeometry::Union { parts } => {
            for part in parts {
                push_diagnostic_geometry_svg(body, part);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_via_local_reroute_diagnostic_svg(
    title: &str,
    model: &KiCadRoutingModel,
    candidate: &KiCadRouteCandidate,
    failure: &KiCadViaLocalRerouteFailureEvidence,
    layer_index: usize,
    show_search_overlay: bool,
    counterfactual_path: Option<&[KiCadRoutePoint]>,
    suppressed: &[KiCadViaLocalRerouteBlockerRef],
) -> String {
    let width = model.bounds[2] - model.bounds[0];
    let height = model.bounds[3] - model.bounds[1];
    let margin = width.max(height) * 0.03 + 1.0;
    let view_x = model.bounds[0] - margin;
    let view_y = model.bounds[1] - margin;
    let view_width = width + margin * 2.0;
    let view_height = height + margin * 2.0;
    let pixel_width = 1200_u32;
    let pixel_height = ((pixel_width as f64 * view_height / view_width).round() as u32).max(300);
    let layer_name = if layer_index == 0 { "F.Cu" } else { "B.Cu" };
    let active_layer = copper_layer_index(&failure.layer) == Some(layer_index);
    let copper = if layer_index == 0 {
        "#4db6ff"
    } else {
        "#ffbd59"
    };
    let mut body = String::new();
    body.push_str(&format!(
        r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#09131e"/>"##,
        diagnostic_svg_number(view_x),
        diagnostic_svg_number(view_y),
        diagnostic_svg_number(view_width),
        diagnostic_svg_number(view_height)
    ));
    body.push_str(&format!(
        r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#102638" stroke="#a7bac8" stroke-width="0.15"/>"##,
        diagnostic_svg_number(model.bounds[0]),
        diagnostic_svg_number(model.bounds[1]),
        diagnostic_svg_number(width),
        diagnostic_svg_number(height)
    ));
    for obstacle in &model.obstacles {
        if obstacle.layers[layer_index] {
            push_diagnostic_obstacle_svg(&mut body, obstacle);
        }
    }
    for segment in candidate
        .supplemental_segments
        .iter()
        .filter(|segment| segment.layer == layer_name)
    {
        body.push_str(&format!(
            r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="{}" stroke-linecap="round"/>"##,
            diagnostic_svg_number(segment.start[0]),
            diagnostic_svg_number(segment.start[1]),
            diagnostic_svg_number(segment.end[0]),
            diagnostic_svg_number(segment.end[1]),
            copper,
            diagnostic_svg_number(segment.width.max(0.08))
        ));
    }
    for via in &candidate.supplemental_vias {
        if via.layers.iter().any(|layer| layer == layer_name) {
            body.push_str(&format!(
                r##"<circle cx="{}" cy="{}" r="{}" fill="#09131e" stroke="#ffe08a" stroke-width="0.13"/>"##,
                diagnostic_svg_number(via.at[0]),
                diagnostic_svg_number(via.at[1]),
                diagnostic_svg_number(via.size * 0.5)
            ));
        }
    }
    let [window_min_x, window_min_y, window_max_x, window_max_y] = failure.window_bounds;
    body.push_str(&format!(
        r##"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="#f8e16c" stroke-width="0.16" stroke-dasharray="0.55 0.3"/>"##,
        diagnostic_svg_number(window_min_x),
        diagnostic_svg_number(window_min_y),
        diagnostic_svg_number(window_max_x - window_min_x),
        diagnostic_svg_number(window_max_y - window_min_y)
    ));
    if active_layer && show_search_overlay {
        let half = failure.resolution_mm * 0.5;
        for run in &failure.blocked_runs {
            let x = failure.grid_origin[0] + run.first_column as f64 * failure.resolution_mm - half;
            let y = failure.grid_origin[1] + run.row as f64 * failure.resolution_mm - half;
            let run_width = (run.last_column - run.first_column + 1) as f64 * failure.resolution_mm;
            body.push_str(&format!(
                r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#ff334f" fill-opacity="0.20"/>"##,
                diagnostic_svg_number(x),
                diagnostic_svg_number(y),
                diagnostic_svg_number(run_width),
                diagnostic_svg_number(failure.resolution_mm)
            ));
        }
        for run in &failure.blocked_frontier_runs {
            let x = failure.grid_origin[0] + run.first_column as f64 * failure.resolution_mm - half;
            let y = failure.grid_origin[1] + run.row as f64 * failure.resolution_mm - half;
            let run_width = (run.last_column - run.first_column + 1) as f64 * failure.resolution_mm;
            body.push_str(&format!(
                r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#67e8f9" fill-opacity="0.88"/>"##,
                diagnostic_svg_number(x),
                diagnostic_svg_number(y),
                diagnostic_svg_number(run_width),
                diagnostic_svg_number(failure.resolution_mm)
            ));
        }
        for blocker in failure.blockers.iter().take(12) {
            let color = match (blocker.kind, blocker.component_movable) {
                (KiCadViaLocalRerouteBlockerKind::FootprintPad, true) => "#7cf29a",
                (KiCadViaLocalRerouteBlockerKind::FootprintPad, false) => "#ff9d76",
                (KiCadViaLocalRerouteBlockerKind::ForeignNetCopper, _) => "#c4a7ff",
                (KiCadViaLocalRerouteBlockerKind::RuleArea, _) => "#ff5cc8",
            };
            let label = format!("{} ({})", blocker.object, blocker.frontier_hits);
            body.push_str(&format!(
                r##"<g data-semantic-blocker="{}"><circle cx="{}" cy="{}" r="{}" fill="#09131e" fill-opacity="0.82" stroke="{}" stroke-width="0.18"/><text x="{}" y="{}" fill="{}" font-size="0.50" font-family="monospace">{}</text></g>"##,
                escape_diagnostic_svg(&blocker.object),
                diagnostic_svg_number(blocker.frontier_centroid[0]),
                diagnostic_svg_number(blocker.frontier_centroid[1]),
                diagnostic_svg_number((failure.resolution_mm * 1.35).max(0.24)),
                color,
                diagnostic_svg_number(blocker.frontier_centroid[0] + 0.28),
                diagnostic_svg_number(blocker.frontier_centroid[1] - 0.28),
                color,
                escape_diagnostic_svg(&label)
            ));
        }
    }
    if active_layer {
        body.push_str(&format!(
            r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="#ffffff" stroke-width="0.16" stroke-dasharray="0.42 0.28"/>"##,
            diagnostic_svg_number(failure.start[0]),
            diagnostic_svg_number(failure.start[1]),
            diagnostic_svg_number(failure.finish[0]),
            diagnostic_svg_number(failure.finish[1])
        ));
        for (point, color) in [(failure.start, "#75f59a"), (failure.finish, "#ffeb74")] {
            body.push_str(&format!(
                r##"<circle cx="{}" cy="{}" r="{}" fill="#09131e" stroke="{}" stroke-width="0.20"/>"##,
                diagnostic_svg_number(point[0]),
                diagnostic_svg_number(point[1]),
                diagnostic_svg_number((failure.resolution_mm * 0.9).max(0.18)),
                color
            ));
        }
    }
    if let Some(path) = counterfactual_path {
        for pair in path
            .windows(2)
            .filter(|pair| copper_layer_index(&pair[0].layer) == Some(layer_index))
        {
            body.push_str(&format!(
                r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="#62f59a" stroke-width="0.32" stroke-linecap="round"/>"##,
                diagnostic_svg_number(pair[0].at[0]),
                diagnostic_svg_number(pair[0].at[1]),
                diagnostic_svg_number(pair[1].at[0]),
                diagnostic_svg_number(pair[1].at[1])
            ));
        }
    }
    let suppressed_summary = suppressed
        .iter()
        .map(|blocker| blocker.object.as_str())
        .collect::<Vec<_>>()
        .join("+");
    let summary = if active_layer && counterfactual_path.is_some() {
        format!("counterfactual path found after suppressing {suppressed_summary}")
    } else if active_layer && show_search_overlay {
        let top = failure.blockers.first().map_or_else(
            || "none".into(),
            |blocker| format!("{}:{}", blocker.object, blocker.frontier_hits),
        );
        format!(
            "{:?}: {} expansions, {} blocked states, {} frontier cells, {} semantic blockers, top {}",
            failure.kind,
            failure.expansions,
            failure.blocked_states,
            failure.blocked_frontier_states,
            failure.blockers.len(),
            top
        )
    } else if active_layer {
        format!("counterfactual trial unsupported after suppressing {suppressed_summary}")
    } else {
        format!("search overlay is on {}", failure.layer)
    };
    body.push_str(&format!(
        r##"<text x="{}" y="{}" fill="#f3f7fa" font-size="0.72" font-family="monospace">{} — {} — {}</text>"##,
        diagnostic_svg_number(model.bounds[0]),
        diagnostic_svg_number(model.bounds[1] - margin * 0.35),
        escape_diagnostic_svg(title),
        layer_name,
        escape_diagnostic_svg(&summary)
    ));
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{pixel_width}" height="{pixel_height}" viewBox="{} {} {} {}" role="img" aria-label="{}"><title>{}</title>{}</svg>"##,
        diagnostic_svg_number(view_x),
        diagnostic_svg_number(view_y),
        diagnostic_svg_number(view_width),
        diagnostic_svg_number(view_height),
        escape_diagnostic_svg(title),
        escape_diagnostic_svg(title),
        body
    )
}

fn convert_diagnostic_svg(svg: &Path, png: &Path) -> Option<PathBuf> {
    Command::new("rsvg-convert")
        .arg(svg)
        .arg("-o")
        .arg(png)
        .status()
        .ok()
        .filter(|status| status.success() && png.is_file())
        .map(|_| PathBuf::from(png.file_name().expect("diagnostic PNG has a file name")))
}

fn write_via_local_reroute_diagnostic(
    artifact_directory: &Path,
    model: &KiCadRoutingModel,
    candidate: &KiCadRouteCandidate,
    evidence: &KiCadViaLocalRerouteEvidence,
) -> Result<KiCadViaLocalRerouteDiagnosticArtifacts, String> {
    let failure = evidence
        .failed_branch
        .as_ref()
        .ok_or_else(|| "local-reroute diagnostic requires failed-branch evidence".to_string())?;
    fs::create_dir(artifact_directory).map_err(|error| error.to_string())?;
    let candidate_name = PathBuf::from("diagnostic-candidate.json");
    let evidence_name = PathBuf::from("local-reroute.json");
    let front_svg_name = PathBuf::from("diagnostic-front.svg");
    let back_svg_name = PathBuf::from("diagnostic-back.svg");
    let front_png_name = PathBuf::from("diagnostic-front.png");
    let back_png_name = PathBuf::from("diagnostic-back.png");
    write_pretty_json(&artifact_directory.join(&candidate_name), candidate)?;
    write_pretty_json(&artifact_directory.join(&evidence_name), evidence)?;
    let title = format!(
        "branch {} {:?} {} r{:.3}",
        failure.branch_index, failure.kind, failure.layer, failure.resolution_mm
    );
    fs::write(
        artifact_directory.join(&front_svg_name),
        render_via_local_reroute_diagnostic_svg(
            &title,
            model,
            candidate,
            failure,
            0,
            true,
            None,
            &[],
        ),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        artifact_directory.join(&back_svg_name),
        render_via_local_reroute_diagnostic_svg(
            &title,
            model,
            candidate,
            failure,
            1,
            true,
            None,
            &[],
        ),
    )
    .map_err(|error| error.to_string())?;
    let front_png = convert_diagnostic_svg(
        &artifact_directory.join(&front_svg_name),
        &artifact_directory.join(&front_png_name),
    );
    let back_png = convert_diagnostic_svg(
        &artifact_directory.join(&back_svg_name),
        &artifact_directory.join(&back_png_name),
    );
    let mut blocker_cut_trials = Vec::new();
    if let Some(blocker_cut) = &evidence.blocker_cut {
        let cut_directory_name = PathBuf::from("blocker-cuts");
        let cut_directory = artifact_directory.join(&cut_directory_name);
        fs::create_dir(&cut_directory).map_err(|error| error.to_string())?;
        for trial in &blocker_cut.trials {
            let stem = format!("trial-{:03}", trial.index);
            let front_svg_file = PathBuf::from(format!("{stem}-front.svg"));
            let back_svg_file = PathBuf::from(format!("{stem}-back.svg"));
            let front_png_file = PathBuf::from(format!("{stem}-front.png"));
            let back_png_file = PathBuf::from(format!("{stem}-back.png"));
            let front_svg = cut_directory_name.join(&front_svg_file);
            let back_svg = cut_directory_name.join(&back_svg_file);
            let suppressed = trial
                .suppressed
                .iter()
                .map(|blocker| blocker.object.as_str())
                .collect::<Vec<_>>()
                .join("+");
            let trial_title = format!(
                "counterfactual trial {} {:?}: suppress {}",
                trial.index, trial.status, suppressed
            );
            let filtered_model = local_reroute_model_without_blockers(model, &trial.suppressed);
            let trial_failure = trial.failure.as_deref().unwrap_or(failure);
            let show_search_overlay = trial.failure.is_some();
            let path = (!trial.path.is_empty()).then_some(trial.path.as_slice());
            fs::write(
                artifact_directory.join(&front_svg),
                render_via_local_reroute_diagnostic_svg(
                    &trial_title,
                    &filtered_model,
                    candidate,
                    trial_failure,
                    0,
                    show_search_overlay,
                    path,
                    &trial.suppressed,
                ),
            )
            .map_err(|error| error.to_string())?;
            fs::write(
                artifact_directory.join(&back_svg),
                render_via_local_reroute_diagnostic_svg(
                    &trial_title,
                    &filtered_model,
                    candidate,
                    trial_failure,
                    1,
                    show_search_overlay,
                    path,
                    &trial.suppressed,
                ),
            )
            .map_err(|error| error.to_string())?;
            let front_png = convert_diagnostic_svg(
                &cut_directory.join(&front_svg_file),
                &cut_directory.join(&front_png_file),
            )
            .map(|name| cut_directory_name.join(name));
            let back_png = convert_diagnostic_svg(
                &cut_directory.join(&back_svg_file),
                &cut_directory.join(&back_png_file),
            )
            .map(|name| cut_directory_name.join(name));
            blocker_cut_trials.push(KiCadViaBlockerCutDiagnosticArtifacts {
                trial: trial.index,
                status: trial.status,
                front_svg,
                back_svg,
                front_png,
                back_png,
            });
        }
    }
    Ok(KiCadViaLocalRerouteDiagnosticArtifacts {
        candidate: candidate_name,
        evidence: evidence_name,
        front_svg: front_svg_name,
        back_svg: back_svg_name,
        front_png,
        back_png,
        blocker_cut_trials,
    })
}

pub fn load_declaration(path: &Path) -> Result<LadderDeclaration, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut declaration: LadderDeclaration = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if declaration.schema_version != 1 {
        return Err(format!(
            "unsupported ladder schema version {}; expected 1",
            declaration.schema_version
        ));
    }
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    declaration.schematic_template = resolve(base, &declaration.schematic_template);
    declaration.pcb_template = resolve(base, &declaration.pcb_template);
    declaration.project_template = declaration
        .project_template
        .as_ref()
        .map(|path| resolve(base, path));
    declaration.replacement_route_candidates = declaration
        .replacement_route_candidates
        .iter()
        .map(|path| resolve(base, path))
        .collect();
    let mut unique = BTreeSet::new();
    for connection in &declaration.connections {
        if connection.is_empty() || !unique.insert(connection.clone()) {
            return Err(format!(
                "connection names must be non-empty and unique: {connection:?}"
            ));
        }
    }
    for route_override in &declaration.route_vertex_overrides {
        if !unique.contains(&route_override.connection) {
            return Err(format!(
                "route override references undeclared connection {:?}",
                route_override.connection
            ));
        }
        if route_override
            .from
            .iter()
            .chain(route_override.to.iter())
            .any(|value| !value.is_finite())
        {
            return Err(format!(
                "route override for {:?} contains a non-finite coordinate",
                route_override.connection
            ));
        }
    }
    for segment in &declaration.supplemental_segments {
        if !unique.contains(&segment.connection) {
            return Err(format!(
                "supplemental segment references undeclared connection {:?}",
                segment.connection
            ));
        }
        if segment
            .start
            .iter()
            .chain(segment.end.iter())
            .chain([&segment.width])
            .any(|value| !value.is_finite())
            || segment.width <= 0.0
            || segment.start == segment.end
            || segment.layer.is_empty()
        {
            return Err(format!(
                "supplemental segment for {:?} has invalid geometry",
                segment.connection
            ));
        }
    }
    for via in &declaration.supplemental_vias {
        if !unique.contains(&via.connection) {
            return Err(format!(
                "supplemental via references undeclared connection {:?}",
                via.connection
            ));
        }
        if via
            .at
            .iter()
            .chain([&via.size, &via.drill])
            .any(|value| !value.is_finite())
            || via.size <= 0.0
            || via.drill <= 0.0
            || via.drill >= via.size
            || via.layers.iter().any(String::is_empty)
            || via.layers[0] == via.layers[1]
        {
            return Err(format!(
                "supplemental via for {:?} has invalid geometry",
                via.connection
            ));
        }
    }
    let mut candidate_connections = BTreeSet::new();
    for path in &declaration.replacement_route_candidates {
        let candidate = read_route_candidate(path)?;
        if !unique.contains(&candidate.connection) {
            return Err(format!(
                "replacement route {} references undeclared connection {:?}",
                path.display(),
                candidate.connection
            ));
        }
        if !candidate_connections.insert(candidate.connection.clone()) {
            return Err(format!(
                "multiple replacement candidates target connection {:?}",
                candidate.connection
            ));
        }
        if declaration
            .supplemental_segments
            .iter()
            .any(|segment| segment.connection == candidate.connection)
            || declaration
                .supplemental_vias
                .iter()
                .any(|via| via.connection == candidate.connection)
        {
            return Err(format!(
                "replacement candidate {:?} conflicts with inline supplemental copper",
                candidate.connection
            ));
        }
    }
    let mut zone_connections = BTreeSet::new();
    for config in &declaration.connection_zone_replacements {
        validate_ground_zone_config(config)?;
        if !unique.contains(&config.connection) {
            return Err(format!(
                "zone replacement references undeclared connection {:?}",
                config.connection
            ));
        }
        if !zone_connections.insert(config.connection.clone()) {
            return Err(format!(
                "multiple zone replacements target connection {:?}",
                config.connection
            ));
        }
    }
    Ok(declaration)
}

fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

pub fn materialize_rung(
    declaration: &LadderDeclaration,
    rung: usize,
    output_root: &Path,
) -> Result<MaterializationReport, String> {
    if rung > declaration.connections.len() {
        return Err(format!(
            "rung {rung} exceeds {} declared connections",
            declaration.connections.len()
        ));
    }
    let selected: BTreeSet<String> = declaration.connections[..rung].iter().cloned().collect();
    let schematic_source =
        fs::read_to_string(&declaration.schematic_template).map_err(|error| {
            format!(
                "failed to read schematic template {}: {error}",
                declaration.schematic_template.display()
            )
        })?;
    let pcb_source = fs::read_to_string(&declaration.pcb_template).map_err(|error| {
        format!(
            "failed to read PCB template {}: {error}",
            declaration.pcb_template.display()
        )
    })?;
    let schematic_template = parse(&schematic_source)?;
    let schematic_fields = schematic_component_fields(&schematic_template);
    let schematic = filter_schematic(schematic_template, &selected)?;
    let pcb_template = parse(&pcb_source)?;
    let replacement_routes = declaration
        .replacement_route_candidates
        .iter()
        .map(|path| read_route_candidate(path))
        .collect::<Result<Vec<_>, String>>()?;

    let directory = output_root.join(format!(
        "{rung:02}-{}",
        rung_name(rung, &declaration.connections)
    ));
    fs::create_dir_all(&directory)
        .map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
    let stem = &declaration.board_id;
    fs::write(
        directory.join(format!("{stem}.kicad_sch")),
        format!("{}\n", encode(&schematic)),
    )
    .map_err(|error| format!("failed to write schematic: {error}"))?;
    let schematic_path = directory.join(format!("{stem}.kicad_sch"));
    let netlist_path = directory.join(format!("{stem}.net"));
    export_netlist(&schematic_path, &netlist_path)?;
    let pad_nets = read_pad_nets(&netlist_path)?;
    let mut pcb = filter_pcb(
        pcb_template,
        &selected,
        &schematic_fields,
        &pad_nets,
        PcbRouteEdits {
            vertex_overrides: &declaration.route_vertex_overrides,
            supplemental_segments: &declaration.supplemental_segments,
            supplemental_vias: &declaration.supplemental_vias,
            replacement_routes: &replacement_routes,
        },
    )?;
    for config in declaration
        .connection_zone_replacements
        .iter()
        .filter(|config| selected.contains(&config.connection))
    {
        replace_routed_connection_with_zones(&mut pcb, config)?;
    }
    fs::write(
        directory.join(format!("{stem}.kicad_pcb")),
        format!("{}\n", encode(&pcb)),
    )
    .map_err(|error| format!("failed to write PCB: {error}"))?;
    let project = match &declaration.project_template {
        Some(path) => fs::read_to_string(path).map_err(|error| {
            format!(
                "failed to read project template {}: {error}",
                path.display()
            )
        })?,
        None => "{}\n".to_string(),
    };
    fs::write(directory.join(format!("{stem}.kicad_pro")), project)
        .map_err(|error| format!("failed to write project: {error}"))?;
    write_project_libraries(&schematic, &pcb, &directory)?;

    let report = MaterializationReport {
        board_id: declaration.board_id.clone(),
        rung,
        selected_connections: declaration.connections[..rung].to_vec(),
        schematic_wires: count_head(&schematic, "wire"),
        schematic_no_connects: count_head(&schematic, "no_connect"),
        pcb_segments: count_head(&pcb, "segment"),
        pcb_arcs: count_head(&pcb, "arc"),
        pcb_vias: count_head(&pcb, "via"),
        pcb_zones: count_head(&pcb, "zone"),
        output_directory: directory.clone(),
    };
    fs::write(
        directory.join("materialization.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| format!("failed to write materialization report: {error}"))?;
    Ok(report)
}

pub fn verify_materialized_rung(
    directory: &Path,
    board_id: &str,
) -> Result<VerificationReport, String> {
    verification_preview::capture(directory, board_id, || {
        remove_if_present(&directory.join("verification.json"))?;
        let pcb_path = directory.join(format!("{board_id}.kicad_pcb"));
        let pcb_source = fs::read_to_string(&pcb_path)
            .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
        let pcb = parse(&pcb_source)?;
        verify_materialized_rung_with_options(directory, board_id, count_head(&pcb, "zone") > 0)
    })
}

pub fn verify_materialized_rung_with_cache(
    directory: &Path,
    board_id: &str,
    cache_root: &Path,
) -> Result<VerificationReport, String> {
    verification_preview::capture(directory, board_id, || {
        remove_if_present(&directory.join("verification.json"))?;
        let pcb_path = directory.join(format!("{board_id}.kicad_pcb"));
        let pcb_source = fs::read_to_string(&pcb_path)
            .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
        let pcb = parse(&pcb_source)?;
        verify_materialized_rung_cached(
            directory,
            board_id,
            count_head(&pcb, "zone") > 0,
            cache_root,
        )
    })
}

pub fn verify_materialized_rung_with_zone_refill(
    directory: &Path,
    board_id: &str,
) -> Result<VerificationReport, String> {
    verification_preview::capture(directory, board_id, || {
        verify_materialized_rung_with_options(directory, board_id, true)
    })
}

fn verify_materialized_rung_with_options(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<VerificationReport, String> {
    if let Some(cache_root) = env::var_os(KICAD_VERIFICATION_CACHE_ENV) {
        verify_materialized_rung_cached(directory, board_id, refill_zones, Path::new(&cache_root))
    } else {
        verify_materialized_rung_uncached(directory, board_id, refill_zones)
    }
}

fn verify_materialized_rung_uncached(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let schematic = directory.join(format!("{board_id}.kicad_sch"));
    let pcb = directory.join(format!("{board_id}.kicad_pcb"));
    let erc_path = directory.join("erc.json");
    let drc_path = directory.join("drc.json");
    run_kicad_report(
        &[
            "sch",
            "erc",
            "--format",
            "json",
            "--severity-all",
            "--output",
        ],
        &erc_path,
        &schematic,
    )?;
    let mut drc_arguments = vec![
        "pcb",
        "drc",
        "--format",
        "json",
        "--severity-all",
        "--schematic-parity",
    ];
    if refill_zones {
        drc_arguments.push("--refill-zones");
    }
    drc_arguments.push("--output");
    run_kicad_report(&drc_arguments, &drc_path, &pcb)?;

    let erc: serde_json::Value = read_json(&erc_path)?;
    let drc: serde_json::Value = read_json(&drc_path)?;
    write_verification_report(directory, board_id, &erc, &drc)
}

/// Route-only admission reuses the frozen schematic ERC report after proving
/// that the schematic bytes are unchanged. PCB DRC still runs with schematic
/// parity enabled, so copper clearance, connectivity, netlist parity, and
/// footprint changes remain native-gated for every transaction.
fn validate_route_only_erc(
    directory: &Path,
    board_id: &str,
    expected_erc_input_sha256: &str,
    frozen_erc: &serde_json::Value,
) -> Result<(), String> {
    validate_report(
        frozen_erc,
        ReportKind::Erc,
        &directory.join(format!("{board_id}.kicad_sch")),
    )?;
    let erc_input_sha256 = kicad_erc_input_sha256(directory)?;
    if erc_input_sha256 != expected_erc_input_sha256 {
        return Err(format!(
            "route-only native admission expected ERC inputs {expected_erc_input_sha256}, found {erc_input_sha256}"
        ));
    }
    Ok(())
}

pub(crate) fn verify_materialized_route_only(
    directory: &Path,
    board_id: &str,
    expected_erc_input_sha256: &str,
    frozen_erc: &serde_json::Value,
) -> Result<VerificationReport, String> {
    verification_preview::capture(directory, board_id, || {
        remove_if_present(&directory.join("verification.json"))?;
        validate_route_only_erc(directory, board_id, expected_erc_input_sha256, frozen_erc)?;
        let pcb_path = directory.join(format!("{board_id}.kicad_pcb"));
        let pcb_source = fs::read_to_string(&pcb_path)
            .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
        let pcb = parse(&pcb_source)?;
        let erc_path = directory.join("erc.json");
        let drc_path = directory.join("drc.json");
        write_typed_json(&erc_path, frozen_erc)?;
        let mut drc_arguments = vec![
            "pcb",
            "drc",
            "--format",
            "json",
            "--severity-all",
            "--schematic-parity",
        ];
        if count_head(&pcb, "zone") > 0 {
            drc_arguments.push("--refill-zones");
        }
        drc_arguments.push("--output");
        run_kicad_report(&drc_arguments, &drc_path, &pcb_path)?;
        let drc: serde_json::Value = read_json(&drc_path)?;
        write_verification_report(directory, board_id, frozen_erc, &drc)
    })
}

fn write_verification_report(
    directory: &Path,
    board_id: &str,
    erc: &serde_json::Value,
    drc: &serde_json::Value,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let report = assess_verification_reports(board_id, erc, drc)?;
    write_typed_json(&directory.join("verification.json"), &report)?;
    Ok(report)
}

fn assess_verification_reports(
    board_id: &str,
    erc: &serde_json::Value,
    drc: &serde_json::Value,
) -> Result<VerificationReport, String> {
    validate_report(
        erc,
        ReportKind::Erc,
        Path::new(&format!("{board_id}.kicad_sch")),
    )?;
    validate_report(
        drc,
        ReportKind::Drc,
        Path::new(&format!("{board_id}.kicad_pcb")),
    )?;
    let erc_violations = erc["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|sheet| sheet["violations"].as_array().map_or(0, Vec::len))
        .sum();
    let schematic_parity_issues = drc["schematic_parity"].as_array().map_or(0, Vec::len);
    let mut drc_design_violations = 0;
    let mut library_metadata_warnings = 0;
    for violation in drc["violations"].as_array().into_iter().flatten() {
        if is_library_metadata_warning(violation) {
            library_metadata_warnings += 1;
        } else {
            drc_design_violations += 1;
        }
    }
    let mut intentional_no_connect_groups = 0;
    let mut selected_net_unconnected_items = 0;
    for violation in drc["unconnected_items"].as_array().into_iter().flatten() {
        if is_intentional_no_connect_group(violation) {
            intentional_no_connect_groups += 1;
        } else {
            selected_net_unconnected_items += 1;
        }
    }
    let complete = erc_violations == 0
        && drc_design_violations == 0
        && schematic_parity_issues == 0
        && selected_net_unconnected_items == 0;
    let report = VerificationReport {
        board_id: board_id.to_string(),
        complete,
        erc_violations,
        drc_design_violations,
        schematic_parity_issues,
        selected_net_unconnected_items,
        intentional_no_connect_groups,
        library_metadata_warnings,
    };
    Ok(report)
}

enum KiCadCacheInput {
    Cacheable(String),
    Bypassed(String),
}

fn verify_materialized_rung_cached(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
    cache_root: &Path,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let started = Instant::now();
    let kicad_cli_version = kicad_cli_version()?;
    let verifier_sha256 = kicad_verifier_implementation_sha256();
    let input_sha256 = match kicad_cache_input_sha256(directory)? {
        KiCadCacheInput::Cacheable(hash) => hash,
        KiCadCacheInput::Bypassed(detail) => {
            let report = verify_materialized_rung_uncached(directory, board_id, refill_zones)?;
            write_kicad_cache_trace(
                directory,
                &KiCadVerificationCacheTrace {
                    schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
                    status: KiCadVerificationCacheStatus::Bypassed,
                    key: None,
                    input_sha256: None,
                    kicad_cli_version,
                    verifier_sha256,
                    policy: KICAD_VERIFICATION_POLICY.into(),
                    refill_zones,
                    cache_root: cache_root.to_path_buf(),
                    cache_entry: None,
                    elapsed_micros: started.elapsed().as_micros() as u64,
                    detail,
                },
            )?;
            return Ok(report);
        }
    };
    let key = {
        let mut hash = Sha256::new();
        hash.update(KICAD_VERIFICATION_CACHE_SCHEMA_VERSION.to_le_bytes());
        hash.update(KICAD_VERIFICATION_POLICY.as_bytes());
        hash.update([u8::from(refill_zones)]);
        hash.update((kicad_cli_version.len() as u64).to_le_bytes());
        hash.update(kicad_cli_version.as_bytes());
        hash.update(verifier_sha256.as_bytes());
        hash.update(input_sha256.as_bytes());
        format!("{:x}", hash.finalize())
    };
    fs::create_dir_all(cache_root).map_err(|error| {
        format!(
            "failed to create KiCad verification cache {}: {error}",
            cache_root.display()
        )
    })?;
    let cache_entry = cache_root.join(&key);
    if cache_entry.is_dir() {
        let entry: KiCadVerificationCacheEntry =
            read_typed_json(&cache_entry.join("cache-entry.json"))?;
        if entry.schema_version != KICAD_VERIFICATION_CACHE_SCHEMA_VERSION
            || entry.key != key
            || entry.input_sha256 != input_sha256
            || entry.kicad_cli_version != kicad_cli_version
            || entry.verifier_sha256 != verifier_sha256
            || entry.policy != KICAD_VERIFICATION_POLICY
            || entry.refill_zones != refill_zones
            || entry.report.board_id != board_id
        {
            return Err(format!(
                "KiCad verification cache entry {} does not match its key",
                cache_entry.display()
            ));
        }
        let cached_erc = cache_entry.join("erc.json");
        let cached_drc = cache_entry.join("drc.json");
        if file_sha256(&cached_erc)? != entry.erc_sha256
            || file_sha256(&cached_drc)? != entry.drc_sha256
        {
            return Err(format!(
                "KiCad verification cache entry {} has corrupted reports",
                cache_entry.display()
            ));
        }
        let assessed = assess_verification_reports(
            board_id,
            &read_json(&cached_erc)?,
            &read_json(&cached_drc)?,
        )?;
        if assessed != entry.report {
            return Err(format!(
                "KiCad verification cache entry {} has an inconsistent assessment",
                cache_entry.display()
            ));
        }
        fs::copy(&cached_erc, directory.join("erc.json"))
            .map_err(|error| format!("failed to restore cached ERC report: {error}"))?;
        fs::copy(&cached_drc, directory.join("drc.json"))
            .map_err(|error| format!("failed to restore cached DRC report: {error}"))?;
        write_typed_json(&directory.join("verification.json"), &entry.report)?;
        write_kicad_cache_trace(
            directory,
            &KiCadVerificationCacheTrace {
                schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
                status: KiCadVerificationCacheStatus::Hit,
                key: Some(key),
                input_sha256: Some(input_sha256),
                kicad_cli_version,
                verifier_sha256,
                policy: KICAD_VERIFICATION_POLICY.into(),
                refill_zones,
                cache_root: cache_root.to_path_buf(),
                cache_entry: Some(cache_entry),
                elapsed_micros: started.elapsed().as_micros() as u64,
                detail: "restored exact ERC, DRC, and interpreted verification reports".into(),
            },
        )?;
        return Ok(entry.report);
    }

    let report = verify_materialized_rung_uncached(directory, board_id, refill_zones)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes UNIX epoch: {error}"))?
        .as_nanos();
    let temporary = cache_root.join(format!(".{key}.{}.{}.tmp", std::process::id(), nonce));
    fs::create_dir(&temporary).map_err(|error| {
        format!(
            "failed to create temporary cache entry {}: {error}",
            temporary.display()
        )
    })?;
    let erc = directory.join("erc.json");
    let drc = directory.join("drc.json");
    fs::copy(&erc, temporary.join("erc.json"))
        .map_err(|error| format!("failed to cache ERC report: {error}"))?;
    fs::copy(&drc, temporary.join("drc.json"))
        .map_err(|error| format!("failed to cache DRC report: {error}"))?;
    fs::copy(
        directory.join("verification.json"),
        temporary.join("verification.json"),
    )
    .map_err(|error| format!("failed to cache verification report: {error}"))?;
    let entry = KiCadVerificationCacheEntry {
        schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
        key: key.clone(),
        input_sha256: input_sha256.clone(),
        kicad_cli_version: kicad_cli_version.clone(),
        verifier_sha256: verifier_sha256.clone(),
        policy: KICAD_VERIFICATION_POLICY.into(),
        refill_zones,
        erc_sha256: file_sha256(&erc)?,
        drc_sha256: file_sha256(&drc)?,
        report: report.clone(),
    };
    write_typed_json(&temporary.join("cache-entry.json"), &entry)?;
    if let Err(error) = fs::rename(&temporary, &cache_entry) {
        if cache_entry.is_dir() {
            fs::remove_dir_all(&temporary).map_err(|cleanup| {
                format!(
                    "cache entry race ({error}) and failed to remove {}: {cleanup}",
                    temporary.display()
                )
            })?;
        } else {
            return Err(format!(
                "failed to publish cache entry {}: {error}",
                cache_entry.display()
            ));
        }
    }
    write_kicad_cache_trace(
        directory,
        &KiCadVerificationCacheTrace {
            schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
            status: KiCadVerificationCacheStatus::Miss,
            key: Some(key),
            input_sha256: Some(input_sha256),
            kicad_cli_version,
            verifier_sha256,
            policy: KICAD_VERIFICATION_POLICY.into(),
            refill_zones,
            cache_root: cache_root.to_path_buf(),
            cache_entry: Some(cache_entry),
            elapsed_micros: started.elapsed().as_micros() as u64,
            detail: "ran native KiCad ERC/DRC and published an immutable cache entry".into(),
        },
    )?;
    Ok(report)
}

/// Identifies the native-verification implementation independently of the
/// router executable that happens to call it.
///
/// The cache intentionally invalidates conservatively after any change to this
/// crate or the resolved Rust dependency graph. Changes confined to placement
/// and routing crates retain native KiCad verification hits when their emitted
/// KiCad inputs are byte-identical.
pub fn kicad_verifier_implementation_sha256() -> String {
    let mut hash = Sha256::new();
    hash.update(b"pcb-maker-kicad-verifier-implementation-v1");
    hash.update(include_bytes!("lib.rs"));
    hash.update(include_bytes!("native_report.rs"));
    hash.update(include_bytes!("../../../Cargo.lock"));
    format!("{:x}", hash.finalize())
}

fn kicad_cli_version() -> Result<String, String> {
    let output = Command::new("kicad-cli")
        .arg("version")
        .output()
        .map_err(|error| format!("failed to identify kicad-cli: {error}"))?;
    if !output.status.success() {
        return Err(format!("kicad-cli version exited with {}", output.status));
    }
    String::from_utf8(output.stdout)
        .map(|version| version.trim().to_string())
        .map_err(|error| format!("kicad-cli version was not UTF-8: {error}"))
}

fn kicad_cache_input_sha256(directory: &Path) -> Result<KiCadCacheInput, String> {
    for table in ["sym-lib-table", "fp-lib-table"] {
        let path = directory.join(table);
        if !path.is_file() {
            continue;
        }
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        for tail in source.split("(uri \"").skip(1) {
            let Some(uri) = tail.split('"').next() else {
                return Ok(KiCadCacheInput::Bypassed(format!(
                    "could not parse library URI in {table}"
                )));
            };
            if !uri.starts_with("${KIPRJMOD}/") {
                return Ok(KiCadCacheInput::Bypassed(format!(
                    "{table} references external library {uri:?}"
                )));
            }
        }
    }
    let mut files = Vec::new();
    collect_kicad_cache_inputs(directory, directory, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err(format!(
            "{} contains no KiCad design inputs",
            directory.display()
        ));
    }
    let mut hash = Sha256::new();
    hash.update(b"pcb-maker-kicad-verification-input-v1");
    for (relative, absolute) in files {
        let relative = relative.as_os_str().as_encoded_bytes();
        let contents = fs::read(&absolute)
            .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative);
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(contents);
    }
    Ok(KiCadCacheInput::Cacheable(format!("{:x}", hash.finalize())))
}

fn collect_kicad_cache_inputs(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "KiCad verification cache rejects symlink {}",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            collect_kicad_cache_inputs(root, &entry.path(), files)?;
            continue;
        }
        if !file_type.is_file() || !is_kicad_cache_input(&entry.path()) {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_path_buf();
        files.push((relative, entry.path()));
    }
    Ok(())
}

fn is_kicad_cache_input(path: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    if matches!(name, Some("sym-lib-table" | "fp-lib-table")) {
        return true;
    }
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("kicad_sch" | "kicad_pcb" | "kicad_pro" | "kicad_sym" | "kicad_mod" | "kicad_dru")
    )
}

fn kicad_erc_input_sha256(directory: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_kicad_cache_inputs(directory, directory, &mut files)?;
    files.retain(|(relative, _)| {
        let name = relative.file_name().and_then(|name| name.to_str());
        name == Some("sym-lib-table")
            || matches!(
                relative
                    .extension()
                    .and_then(|extension| extension.to_str()),
                Some("kicad_sch" | "kicad_sym" | "kicad_pro" | "kicad_dru")
            )
    });
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err(format!(
            "{} contains no KiCad ERC inputs",
            directory.display()
        ));
    }
    let mut hash = Sha256::new();
    hash.update(b"pcb-maker-kicad-erc-input-v1");
    for (relative, absolute) in files {
        let relative = relative.as_os_str().as_encoded_bytes();
        let contents = fs::read(&absolute)
            .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative);
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(contents);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let contents =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(contents)))
}

fn write_typed_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, format!("{serialized}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn read_typed_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn write_kicad_cache_trace(
    directory: &Path,
    trace: &KiCadVerificationCacheTrace,
) -> Result<(), String> {
    write_typed_json(&directory.join("verification-cache.json"), trace)?;
    if let Some(event_path) = env::var_os(KICAD_VERIFICATION_CACHE_EVENTS_ENV) {
        let event_path = PathBuf::from(event_path);
        if let Some(parent) = event_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create cache event directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let mut events = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&event_path)
            .map_err(|error| {
                format!(
                    "failed to open cache event log {}: {error}",
                    event_path.display()
                )
            })?;
        let serialized = serde_json::to_string(trace).map_err(|error| error.to_string())?;
        writeln!(events, "{serialized}").map_err(|error| {
            format!(
                "failed to append cache event log {}: {error}",
                event_path.display()
            )
        })?;
    }
    Ok(())
}

type GridTree = BTreeMap<(usize, usize, usize), GridPosition>;

fn grid_position_key(position: GridPosition) -> (usize, usize, usize) {
    (position.layer, position.y, position.x)
}

fn grid_position_at(position: GridPosition, origin: [f64; 2], resolution_mm: f64) -> [f64; 2] {
    [
        origin[0] + position.x as f64 * resolution_mm,
        origin[1] + position.y as f64 * resolution_mm,
    ]
}

fn tree_attachment_lower_bound(
    source: GridPosition,
    target: GridPosition,
    config: &KiCadGridRouteConfig,
) -> u64 {
    let dx = source.x.abs_diff(target.x) as u64;
    let dy = source.y.abs_diff(target.y) as u64;
    let diagonal = dx.min(dy);
    let straight = dx.max(dy) - diagonal;
    diagonal
        .saturating_mul(u64::from(config.diagonal_cost))
        .saturating_add(straight.saturating_mul(u64::from(config.straight_cost)))
        .saturating_add(if source.layer != target.layer {
            u64::from(config.via_cost_units())
        } else {
            0
        })
}

fn ranked_tree_attachment_sources(
    tree: &GridTree,
    target: GridPosition,
    config: &KiCadGridRouteConfig,
) -> Vec<GridPosition> {
    let mut sources: Vec<_> = tree.values().copied().collect();
    sources.sort_by_key(|source| {
        (
            tree_attachment_lower_bound(*source, target, config),
            source.layer,
            source.y,
            source.x,
        )
    });
    let mut retained: Vec<GridPosition> = Vec::new();
    for source in sources {
        let sufficiently_distinct = retained.iter().all(|prior| {
            if prior.layer != source.layer {
                return true;
            }
            let dx = prior.x.abs_diff(source.x) as f64 * config.resolution_mm;
            let dy = prior.y.abs_diff(source.y) as f64 * config.resolution_mm;
            dx.hypot(dy) + f64::EPSILON >= config.tree_attachment_minimum_spacing_mm
        });
        if sufficiently_distinct {
            retained.push(source);
            if retained.len() == config.maximum_tree_attachment_searches {
                break;
            }
        }
    }
    retained
}

fn trim_grid_path_to_last_tree_contact(
    path: &[GridPosition],
    tree: &GridTree,
) -> Vec<GridPosition> {
    let last_contact = path
        .iter()
        .rposition(|position| tree.contains_key(&grid_position_key(*position)))
        .unwrap_or(0);
    path[last_contact..].to_vec()
}

fn tree_attachment_is_preferred(
    candidate: (u32, f64, usize, usize),
    current: (u32, f64, usize, usize),
    objective: KiCadTreeAttachmentObjective,
) -> bool {
    let (candidate_cost, candidate_length, candidate_vias, candidate_rank) = candidate;
    let (current_cost, current_length, current_vias, current_rank) = current;
    match objective {
        KiCadTreeAttachmentObjective::LengthThenVias => candidate_length
            .total_cmp(&current_length)
            .then_with(|| candidate_vias.cmp(&current_vias))
            .then_with(|| candidate_cost.cmp(&current_cost))
            .then_with(|| candidate_rank.cmp(&current_rank))
            .is_lt(),
        KiCadTreeAttachmentObjective::ViasThenLength => candidate_vias
            .cmp(&current_vias)
            .then_with(|| candidate_length.total_cmp(&current_length))
            .then_with(|| candidate_cost.cmp(&current_cost))
            .then_with(|| candidate_rank.cmp(&current_rank))
            .is_lt(),
        KiCadTreeAttachmentObjective::RouterCost => candidate_cost
            .cmp(&current_cost)
            .then_with(|| candidate_length.total_cmp(&current_length))
            .then_with(|| candidate_vias.cmp(&current_vias))
            .then_with(|| candidate_rank.cmp(&current_rank))
            .is_lt(),
    }
}

struct KiCadTreeRouteTrial {
    rank: usize,
    evidence_index: Option<usize>,
    source: GridPosition,
    cost: u32,
    expansions: u32,
    grid_path: Vec<GridPosition>,
    path: Vec<KiCadRoutePoint>,
    segments: Vec<SupplementalSegment>,
    vias: Vec<SupplementalVia>,
}

impl KiCadTreeRouteTrial {
    fn length_mm(&self) -> f64 {
        self.segments
            .iter()
            .map(|segment| distance_squared(segment.start, segment.end).sqrt())
            .sum()
    }

    fn selection_key(&self) -> (u32, f64, usize, usize) {
        (self.cost, self.length_mm(), self.vias.len(), self.rank)
    }
}

/// Routes one connection in an already materialized PCB.
///
/// This is an adapter around the imported DUT A* kernel, not the higher-level
/// conflict-action search. Multi-terminal nets use a deterministic rooted-star
/// baseline with independently bounded branches. The function intentionally
/// returns declaration fragments and evidence without modifying either the
/// board or its declaration.
pub fn route_materialized_connection(
    pcb_path: &Path,
    connection: &str,
    config: &KiCadGridRouteConfig,
) -> Result<KiCadRouteCandidate, String> {
    route_materialized_connection_detailed(pcb_path, connection, config)
        .map_err(|error| error.to_string())
}

/// Machine-actionable counterpart of `route_materialized_connection`.
pub fn route_materialized_connection_detailed(
    pcb_path: &Path,
    connection: &str,
    config: &KiCadGridRouteConfig,
) -> Result<KiCadRouteCandidate, KiCadRouteError> {
    route_materialized_connection_with_yielding_detailed(pcb_path, connection, &[], config)
}

pub(crate) fn materialized_connection_terminal_count(
    pcb_path: &Path,
    connection: &str,
) -> Result<usize, String> {
    let source = fs::read_to_string(pcb_path)
        .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
    let pcb = parse(&source)?;
    Ok(KiCadRoutingModel::from_pcb(&pcb, connection)?
        .electrical_terminals()
        .len())
}

/// Routes a diagnostic counterfactual in which selected foreign connections'
/// tracks and vias yield. Their pads remain fixed obstacles.
///
/// A returned route is not an admissible board: every yielding connection
/// would have to be rerouted and the complete board natively gated in one
/// transaction. This seam exists to identify which retained routes close a
/// local topology, not to silently rip them up.
pub fn route_materialized_connection_with_yielding_connections(
    pcb_path: &Path,
    connection: &str,
    yielding_connections: &[String],
    config: &KiCadGridRouteConfig,
) -> Result<KiCadRouteCandidate, String> {
    route_materialized_connection_with_yielding_detailed(
        pcb_path,
        connection,
        yielding_connections,
        config,
    )
    .map_err(|error| error.to_string())
}

fn route_materialized_connection_with_yielding_detailed(
    pcb_path: &Path,
    connection: &str,
    yielding_connections: &[String],
    config: &KiCadGridRouteConfig,
) -> Result<KiCadRouteCandidate, KiCadRouteError> {
    route_materialized_connection_inspected(pcb_path, connection, yielding_connections, config, None)
}

fn route_materialized_connection_inspected(
    pcb_path: &Path,
    connection: &str,
    yielding_connections: &[String],
    config: &KiCadGridRouteConfig,
    inspector: Option<&mut routing_cut::Inspector>,
) -> Result<KiCadRouteCandidate, KiCadRouteError> {
    route_materialized_connection_selected(pcb_path, connection, yielding_connections, config, inspector, None)
}

fn route_materialized_connection_selected(
    pcb_path: &Path,
    connection: &str,
    yielding_connections: &[String],
    config: &KiCadGridRouteConfig,
    mut inspector: Option<&mut routing_cut::Inspector>,
    selected_pair: Option<&KiCadPadPairRoutingConfig>,
) -> Result<KiCadRouteCandidate, KiCadRouteError> {
    let resolved_config = connection_rules::resolve(config, connection)?;
    let config = &resolved_config;
    validate_grid_route_config(config)?;
    if yielding_connections
        .iter()
        .any(|yielding| yielding == connection)
    {
        return Err("the target connection cannot yield to itself".into());
    }
    let source = fs::read_to_string(pcb_path)
        .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
    let pcb = parse(&source)?;
    let mut model = KiCadRoutingModel::from_pcb(&pcb, connection)?;
    connection_rules::apply(&mut model, config)?;
    let terminal_contacts = if let Some(pair) = selected_pair {
        pad_pair::select_terminals(&pcb, &mut model, pair)?
    } else {
        model.electrical_terminals()
    };
    let terminals: Vec<_> = terminal_contacts.iter().map(|contact| contact.at).collect();
    let yielding: BTreeSet<_> = yielding_connections.iter().map(String::as_str).collect();
    if !yielding.is_empty() {
        model
            .obstacles
            .retain(|obstacle| retain_obstacle_when_connections_yield(obstacle, &yielding));
    }
    let trace_radius = config.trace_width_mm / 2.0;
    let origin = [
        model.bounds[0] + config.edge_clearance_mm + trace_radius,
        model.bounds[1] + config.edge_clearance_mm + trace_radius,
    ];
    let ([min_x, min_y], grid_alignment_evidence) =
        grid_alignment::aligned_origin(&model, config, origin);
    let max_x = model.bounds[2] - config.edge_clearance_mm - trace_radius;
    let max_y = model.bounds[3] - config.edge_clearance_mm - trace_radius;
    if min_x >= max_x || min_y >= max_y {
        return Err("board has no routing area after applying edge clearance".into());
    }
    let width = (((max_x - min_x) / config.resolution_mm).floor() as usize) + 1;
    let height = (((max_y - min_y) / config.resolution_mm).floor() as usize) + 1;
    let plane = width
        .checked_mul(height)
        .ok_or_else(|| "routing grid dimensions overflow".to_string())?;
    let state_count = plane
        .checked_mul(2)
        .ok_or_else(|| "routing grid state count overflow".to_string())?;
    // Grid nodes represent exact centerline points. Check every transition
    // against continuous obstacle geometry below instead of inflating all
    // obstacles by a half-cell diagonal. The old raster margin could erase a
    // physically legal narrow channel even when an edge through it was safe.
    let trace_inflate = trace_radius + config.clearance_mm;
    let via_inflate = config.via_size_mm / 2.0 + config.clearance_mm;
    let trace_edge_inflate = trace_radius + config.edge_clearance_mm;
    let via_edge_inflate = config.via_size_mm / 2.0 + config.edge_clearance_mm;
    let obstacle_grid = KiCadObstacleGrid::new(&model, 4.0, config.clearance_mm);
    let mut blocked = vec![false; state_count];
    let (trace_enter_costs, mut via_enter_costs) =
        routing_demand::costs(config, connection, [min_x, min_y], [width, height]);
    for layer in 0..2 {
        for y in 0..height {
            for x in 0..width {
                let point = [
                    min_x + x as f64 * config.resolution_mm,
                    min_y + y as f64 * config.resolution_mm,
                ];
                let state = layer * plane + y * width + x;
                blocked[state] = !model
                    .outline
                    .contains_point_with_clearance(point, trace_edge_inflate)
                    || obstacle_grid.any_contains(
                        &model,
                        layer,
                        point,
                        trace_inflate,
                        CopperQueryKind::Trace,
                    );
                // A through-via checks both layers and the same holes from
                // either endpoint. Store one geometric answer on both layers.
                if layer != 0 {
                    continue;
                }
                let via_outside = !model
                    .outline
                    .contains_point_with_clearance(point, via_edge_inflate);
                let via_collision =
                    obstacle_grid.any_contains(&model, 0, point, via_inflate, CopperQueryKind::Via)
                        || obstacle_grid.any_contains(
                            &model,
                            1,
                            point,
                            via_inflate,
                            CopperQueryKind::Via,
                        )
                        || model.target_holes.iter().any(|hole| {
                            hole.contains(
                                point,
                                config.via_drill_mm / 2.0 + config.hole_to_hole_clearance_mm,
                            )
                        });
                if via_outside || via_collision {
                    via_enter_costs[state] = u32::MAX;
                    via_enter_costs[state + plane] = u32::MAX;
                }
            }
        }
    }
    let blocked_planar_transitions = {
        let outline_points = routing_grid::OutlinePointCache::new(
            &model.outline,
            width,
            height,
            [min_x, min_y],
            config.resolution_mm,
            trace_edge_inflate,
        );
        // The outline and trace-edge clearance are identical on both copper layers.
        // Only obstacle queries below depend on the current layer.
        let outline_mask =
            routing_grid::planar_transition_mask(width, height, 1, |_layer, start, end| {
                !outline_points.contains_segment(start, end)
            });
        routing_grid::planar_transition_mask(width, height, 2, |layer, start, end| {
            routing_grid::edge_mask_at(&outline_mask, width, start, end)
                || obstacle_grid.any_intersects_segment(
                    &model,
                    layer,
                    outline_points.at(start),
                    outline_points.at(end),
                    trace_inflate,
                )
        })
    };

    let mut terminal_access_recoveries = 0usize;
    let mut grid_terminal_options = terminal_contacts
        .iter()
        .map(|contact| {
            let terminal = contact.at;
            let mut grid = match snapped_grid_position(terminal, [min_x, min_y], config, width, height) {
                Ok(grid) => grid,
                Err(error) => {
                    if config.terminal_contact_policy == KiCadTerminalContactPolicy::FirstPadContact {
                        let options = terminal_access::copper_options(
                            &model,
                            contact,
                            [min_x, min_y],
                            [width, height],
                            config,
                            |p| blocked[p.layer * plane + p.y * width + p.x],
                        )?;
                        if !options.is_empty() {
                            terminal_access_recoveries += 1;
                            return Ok(options);
                        }
                    }
                    return Err(error);
                }
            };
            let layers = contact.layers;
            grid.layer = preferred_terminal_layer(layers).ok_or_else(|| {
                format!("connection {connection:?} terminal pad has no supported copper layer")
            })?;
            let mut options = vec![grid];
            if contact.layers == [true, true] {
                options.push(GridPosition {
                    layer: 1 - grid.layer,
                    ..grid
                });
            }
            Ok(options)
        })
        .collect::<Result<Vec<_>, String>>()?;
    for (contact, options) in terminal_contacts.iter().zip(&mut grid_terminal_options) {
        let snapped = options[0];
        let layers: Vec<_> = options
            .iter()
            .map(|option| copper_layer_name(option.layer))
            .collect();
        for terminal in options.iter() {
            let state = terminal.layer * plane + terminal.y * width + terminal.x;
            if blocked[state]
                && config.terminal_contact_policy == KiCadTerminalContactPolicy::FirstPadContact
            {
                let at = grid_position_at(*terminal, [min_x, min_y], config.resolution_mm);
                if !obstacle_grid.any_contains(
                    &model,
                    terminal.layer,
                    at,
                    trace_radius + config.clearance_mm,
                    CopperQueryKind::Trace,
                ) && model
                    .outline
                    .contains_point_with_clearance(at, trace_radius + config.edge_clearance_mm)
                {
                    blocked[state] = false;
                }
            }
        }
        options
            .retain(|terminal| !blocked[terminal.layer * plane + terminal.y * width + terminal.x]);
        if options.is_empty()
            && config.terminal_contact_policy == KiCadTerminalContactPolicy::FirstPadContact
        {
            *options = terminal_access::copper_options(
                &model,
                contact,
                [min_x, min_y],
                [width, height],
                config,
                |p| blocked[p.layer * plane + p.y * width + p.x],
            )?;
            if !options.is_empty() {
                terminal_access_recoveries += 1;
            }
        }
        if options.is_empty() {
            let at = grid_position_at(snapped, [min_x, min_y], config.resolution_mm);
            return Err(format!(
                "connection {connection:?} terminal snaps to occupied grid cells at ({}, {}) = ({:.4}, {:.4}) on every electrically connected pad layer {layers:?}",
                snapped.x, snapped.y, at[0], at[1]
            ).into());
        }
    }
    let grid_terminals: Vec<_> = grid_terminal_options
        .iter()
        .map(|options| options[0])
        .collect();
    let mut router = DutAStar;
    let mut reachability_checker = GridFloodFill;
    let obstacle_guided = config.heuristic == KiCadGridHeuristic::ObstacleDistances;
    let base_router_name = if obstacle_guided {
        "obstacle-distance-astar-v1"
    } else {
        router.name()
    };
    let shared_tree = config.multi_terminal_routing
        == KiCadMultiTerminalRoutingPolicy::SharedCopperTree
        && grid_terminals.len() > 2;
    let multi_source_tree =
        shared_tree && config.tree_attachment_search == KiCadTreeAttachmentSearch::MultiSource;
    let mut router_name = if shared_tree {
        format!("{base_router_name}-shared-copper-tree-v1")
    } else {
        base_router_name.to_string()
    };
    if !yielding.is_empty() {
        router_name.push_str("-yielding-counterfactual-v1");
    }
    if terminal_access_recoveries > 0 {
        router_name.push_str("-pad-copper-access-v1");
    }
    if multi_source_tree {
        router_name.push_str("-multi-source-v1");
    }
    if config.reachability_preflight && !obstacle_guided {
        router_name.push_str("-reachability-preflight-v1");
    }
    let root = rooted_star_terminal(&grid_terminals);
    let mut pending: Vec<_> = (0..grid_terminals.len())
        .filter(|terminal| *terminal != root)
        .collect();
    pending.sort_by_key(|terminal| {
        (
            grid_distance(grid_terminals[root], grid_terminals[*terminal]),
            *terminal,
        )
    });
    let mut branches: Vec<KiCadRouteBranch> = Vec::with_capacity(pending.len());
    let mut segments = Vec::new();
    let mut vias = Vec::new();
    let mut segment_keys = BTreeSet::new();
    let mut via_keys = BTreeSet::new();
    let mut total_cost = 0_u32;
    let mut total_reachability_expansions = 0_u32;
    let mut total_heuristic_expansions = 0_u32;
    let mut total_expansions = 0_u32;
    let mut tree = GridTree::new();
    let mut tree_attachment_attempts = Vec::new();
    for (branch_index, terminal) in pending.into_iter().enumerate() {
        let sources = if shared_tree && branch_index > 0 {
            if multi_source_tree {
                // Keep one request; all other tree contacts become equal-cost
                // starts, including both sides of proven plated pad contacts.
                tree.values().copied().collect()
            } else {
                ranked_tree_attachment_sources(&tree, grid_terminals[terminal], config)
            }
        } else {
            vec![grid_terminals[root]]
        };
        let sources = if multi_source_tree && branch_index > 0 {
            let mut expanded: GridTree = sources
                .into_iter()
                .map(|source| (grid_position_key(source), source))
                .collect();
            for source in tree.values() {
                let at = grid_position_at(*source, [min_x, min_y], config.resolution_mm);
                let other = GridPosition {
                    layer: 1 - source.layer,
                    ..*source
                };
                if pad_layer_contacts::plated_copper_at(&model, at)
                    && !blocked[other.layer * plane + other.y * width + other.x]
                {
                    expanded.insert(grid_position_key(other), other);
                }
            }
            expanded.into_values().collect::<Vec<_>>()
        } else {
            sources
        };
        let all_tree_starts = (multi_source_tree && branch_index > 0).then(|| sources.clone());
        let mut trials = Vec::new();
        let mut budget_exhausted_searches = 0;
        for (rank, source) in sources.into_iter().enumerate() {
            if all_tree_starts.is_some() && rank > 0 {
                break;
            }
            let alternate_starts = if let Some(starts) = &all_tree_starts {
                starts[1..].to_vec()
            } else if !shared_tree || branch_index == 0 {
                grid_terminal_options[root][1..].to_vec()
            } else {
                let at = grid_position_at(source, [min_x, min_y], config.resolution_mm);
                let other = GridPosition {
                    layer: 1 - source.layer,
                    ..source
                };
                if pad_layer_contacts::plated_copper_at(&model, at)
                    && !blocked[other.layer * plane + other.y * width + other.x]
                {
                    vec![other]
                } else {
                    Vec::new()
                }
            };
            let request = GridRouteRequest {
                width,
                height,
                layers: 2,
                start: source,
                finish: grid_terminals[terminal],
                alternate_starts,
                alternate_finishes: grid_terminal_options[terminal][1..].to_vec(),
                max_expansions: config.max_expansions,
                straight_cost: config.straight_cost,
                diagonal_cost: config.diagonal_cost,
                bend_cost: config.bend_cost,
                via_cost: config.via_cost_units(),
                allow_vias: true,
                blocked: blocked.clone(),
                blocked_planar_transitions: blocked_planar_transitions.clone(),
                trace_enter_costs: trace_enter_costs.clone(),
                via_enter_costs: via_enter_costs.clone(),
            };
            if let Some(inspector) = inspector.as_deref_mut()
                && inspector.branch == branch_index
            {
                inspector.capture(&request, &model, config, [min_x, min_y])?;
                return Err("routing cut inspection stopped before branch search".into());
            }
            if config.reachability_preflight && !obstacle_guided {
                let reachability = reachability_checker.check(&request)?;
                let reachability_expansions = match reachability {
                    GridReachabilityOutcome::Reachable { expansions }
                    | GridReachabilityOutcome::Unreachable { expansions } => expansions,
                };
                total_reachability_expansions =
                    total_reachability_expansions.saturating_add(reachability_expansions);
                if matches!(reachability, GridReachabilityOutcome::Unreachable { .. }) {
                    if shared_tree && branch_index > 0 {
                        tree_attachment_attempts.push(KiCadTreeAttachmentAttempt {
                            branch_index,
                            rank,
                            target_terminal: terminals[terminal],
                            searched_source: grid_position_at(
                                source,
                                [min_x, min_y],
                                config.resolution_mm,
                            ),
                            searched_source_layer: copper_layer_name(source.layer).into(),
                            source: grid_position_at(source, [min_x, min_y], config.resolution_mm),
                            source_layer: copper_layer_name(source.layer).into(),
                            status: KiCadTreeAttachmentAttemptStatus::NoPath,
                            cost: None,
                            expansions: 0,
                            retained_length_mm: None,
                            via_count: None,
                            selected: false,
                        });
                    }
                    continue;
                }
            }
            let outcome = if obstacle_guided {
                let (outcome, prepared) = route_with_obstacle_distances(&request)?;
                total_heuristic_expansions = total_heuristic_expansions.saturating_add(prepared);
                outcome
            } else {
                router.route(&request)?
            };
            if matches!(&outcome, GridRouteOutcome::BudgetExhausted { .. }) {
                budget_exhausted_searches += 1;
            }
            let outcome_expansions = match &outcome {
                GridRouteOutcome::Found { expansions, .. }
                | GridRouteOutcome::NoPath { expansions, .. }
                | GridRouteOutcome::BudgetExhausted { expansions, .. } => *expansions,
            };
            total_expansions = total_expansions.saturating_add(outcome_expansions);
            let evidence_index = (shared_tree && branch_index > 0).then(|| {
                let index = tree_attachment_attempts.len();
                tree_attachment_attempts.push(KiCadTreeAttachmentAttempt {
                    branch_index,
                    rank,
                    target_terminal: terminals[terminal],
                    searched_source: grid_position_at(source, [min_x, min_y], config.resolution_mm),
                    searched_source_layer: copper_layer_name(source.layer).into(),
                    source: grid_position_at(source, [min_x, min_y], config.resolution_mm),
                    source_layer: copper_layer_name(source.layer).into(),
                    status: match &outcome {
                        GridRouteOutcome::Found { .. } => KiCadTreeAttachmentAttemptStatus::Found,
                        GridRouteOutcome::NoPath { .. } => KiCadTreeAttachmentAttemptStatus::NoPath,
                        GridRouteOutcome::BudgetExhausted { .. } => {
                            KiCadTreeAttachmentAttemptStatus::BudgetExhausted
                        }
                    },
                    cost: None,
                    expansions: outcome_expansions,
                    retained_length_mm: None,
                    via_count: None,
                    selected: false,
                });
                index
            });
            let GridRouteOutcome::Found {
                path,
                cost,
                expansions,
            } = outcome
            else {
                continue;
            };
            let grid_path = if shared_tree && branch_index > 0 {
                trim_grid_path_to_last_tree_contact(&path, &tree)
            } else {
                path
            };
            let source_at = if !shared_tree || branch_index == 0 {
                terminals[root]
            } else {
                grid_position_at(grid_path[0], [min_x, min_y], config.resolution_mm)
            };
            let start_pad = (!shared_tree || branch_index == 0)
                .then(|| {
                    terminal_copper_geometry_at(&model, terminals[root], grid_path[0].layer)
                })
                .flatten();
            let finish_pad = terminal_copper_geometry_at(
                &model,
                terminals[terminal],
                grid_path.last().expect("found path").layer,
            );
            let (path, trial_segments, trial_vias) = materialize_grid_path(
                connection,
                &grid_path,
                [source_at, terminals[terminal]],
                [start_pad.as_ref(), finish_pad.as_ref()],
                [min_x, min_y],
                config,
            );
            let trial = KiCadTreeRouteTrial {
                rank,
                evidence_index,
                source: grid_path[0],
                cost,
                expansions,
                grid_path,
                path,
                segments: trial_segments,
                vias: trial_vias,
            };
            if let Some(index) = trial.evidence_index {
                tree_attachment_attempts[index].source = source_at;
                tree_attachment_attempts[index].source_layer =
                    copper_layer_name(trial.source.layer).into();
                tree_attachment_attempts[index].cost = Some(cost);
                tree_attachment_attempts[index].retained_length_mm = Some(trial.length_mm());
                tree_attachment_attempts[index].via_count = Some(trial.vias.len());
            }
            trials.push(trial);
        }
        let selected_index = trials
            .iter()
            .enumerate()
            .fold(None, |selected: Option<usize>, (index, candidate)| {
                if selected.is_none_or(|current| {
                    tree_attachment_is_preferred(
                        candidate.selection_key(),
                        trials[current].selection_key(),
                        config.tree_attachment_objective,
                    )
                }) {
                    Some(index)
                } else {
                    selected
                }
            })
            .ok_or_else(|| {
                let kind = if budget_exhausted_searches > 0 {
                    KiCadRouteSearchFailureKind::SearchBudgetExhausted
                } else {
                    KiCadRouteSearchFailureKind::GridDisconnected
                };
                KiCadRouteError::Search(KiCadRouteSearchFailure {
                    kind,
                    connection: connection.into(),
                    branch: branch_index,
                    astar_expansions: total_expansions,
                    reachability_expansions: total_reachability_expansions,
                    heuristic_expansions: obstacle_guided.then_some(total_heuristic_expansions),
                    terminal_context: Some(KiCadRouteTerminalContext {
                        root_mm: terminals[root],
                        reached_mm: std::iter::once(terminals[root])
                            .chain(branches.iter().map(|branch| branch.finish_terminal))
                            .collect(),
                        pending_mm: terminals[terminal],
                    }),
                })
            })?;
        let selected = trials.swap_remove(selected_index);
        if let Some(index) = selected.evidence_index {
            tree_attachment_attempts[index].selected = true;
        }
        let KiCadTreeRouteTrial {
            source,
            cost,
            expansions,
            grid_path,
            path,
            segments: selected_segments,
            vias: selected_vias,
            ..
        } = selected;
        insert_materialized_grid_path_contacts(
            &mut tree,
            &grid_path,
            &path,
            [min_x, min_y],
            config.resolution_mm,
        );
        for via in &selected_vias {
            reserve_via_hole_spacing(
                &mut via_enter_costs,
                width,
                height,
                [min_x, min_y],
                via.at,
                config,
                0.0,
            );
        }
        for segment in selected_segments {
            if segment_keys.insert(supplemental_segment_key(&segment)) {
                segments.push(segment);
            }
        }
        for via in selected_vias {
            if via_keys.insert(supplemental_via_key(&via)) {
                vias.push(via);
            }
        }
        total_cost = total_cost.saturating_add(cost);
        branches.push(KiCadRouteBranch {
            start_terminal: if !shared_tree || branch_index == 0 {
                terminals[root]
            } else {
                grid_position_at(source, [min_x, min_y], config.resolution_mm)
            },
            finish_terminal: terminals[terminal],
            cost,
            expansions,
            path,
        });
    }
    if shared_tree {
        // Later branches can attach to the interior of an earlier simplified
        // segment. Publish explicit shared vertices in both the route graph
        // and native copper; otherwise KiCad can flag the branch endpoint as
        // dangling despite electrical contact with the unsplit parent track.
        let selected = (0..branches.len()).collect::<Vec<_>>();
        normalize_selected_route_graph(&mut branches, &selected);
        (segments, vias) = materialize_candidate_branches(connection, &branches, config)?;
    }
    Ok(KiCadRouteCandidate {
        schema_version: 2,
        connection: connection.to_string(),
        router: router_name,
        config: config.clone(),
        terminals: terminals,
        grid_origin: [min_x, min_y],
        grid_alignment_evidence,
        grid_size: [width, height],
        cost: total_cost,
        reachability_expansions: total_reachability_expansions,
        heuristic_expansions: obstacle_guided.then_some(total_heuristic_expansions),
        expansions: total_expansions,
        branches,
        tree_attachment_attempts,
        supplemental_segments: segments,
        supplemental_vias: vias,
        footprint_placements: Vec::new(),
        reference_placements: Vec::new(),
    })
}

fn retain_obstacle_when_connections_yield(
    obstacle: &CopperObstacle,
    yielding: &BTreeSet<&str>,
) -> bool {
    obstacle.kind != KiCadViaLocalRerouteBlockerKind::ForeignNetCopper
        || !obstacle
            .net
            .as_deref()
            .is_some_and(|net| yielding.contains(net))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadYieldingConnectionDiagnosisConfig {
    /// Inspect the failed branch's reachable boundary before choosing removals.
    /// Advisory and opt-in; fall back to the declared hints on inspection error.
    pub prioritize_boundary_blockers: bool,
    /// Advisory order for foreign routed nets. Unknown names are ignored;
    /// every remaining net is retained in deterministic lexical order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub preferred_yielding_connections: Vec<String>,
    /// Start at pairs when single-net coverage has already been measured.
    pub minimum_yielding_connections: usize,
    /// Maximum number of retained foreign connections removed together in a
    /// counterfactual. This diagnostic currently caps the value at two.
    pub maximum_yielding_connections: usize,
    pub maximum_trials: usize,
    pub routing: KiCadGridRouteConfig,
}

impl Default for KiCadYieldingConnectionDiagnosisConfig {
    fn default() -> Self {
        Self {
            prioritize_boundary_blockers: false,
            preferred_yielding_connections: Vec::new(),
            minimum_yielding_connections: 1,
            maximum_yielding_connections: 1,
            maximum_trials: 128,
            routing: KiCadGridRouteConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadYieldingConnectionTrial {
    pub ordinal: usize,
    pub yielding_connections: Vec<String>,
    pub route_found: bool,
    pub reachability_rejected: bool,
    pub reachability_expansions: u32,
    pub route_expansions: Option<u32>,
    /// Advisory wall time for this complete counterfactual route call.
    pub elapsed_micros: u64,
    pub candidate: Option<KiCadRouteCandidate>,
    pub quality: Option<KiCadRouteQuality>,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_failure: Option<KiCadRouteSearchFailure>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadYieldingConnectionDiagnosis {
    pub schema_version: u32,
    pub target_connection: String,
    pub board: PathBuf,
    pub base_route_found: bool,
    pub base_reachability_rejected: bool,
    pub base_reachability_expansions: u32,
    pub base_route_expansions: Option<u32>,
    pub base_elapsed_micros: u64,
    pub base_candidate: Option<KiCadRouteCandidate>,
    pub base_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_route_failure: Option<KiCadRouteSearchFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_cut: Option<KiCadRoutingCutReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_cut_error: Option<String>,
    pub foreign_routed_connections: Vec<String>,
    pub yielding_connection_order: Vec<String>,
    pub minimum_yielding_connections: usize,
    pub maximum_yielding_connections: usize,
    pub maximum_trials: usize,
    pub truncated: bool,
    pub successful_counterfactuals: usize,
    pub total_reachability_expansions: u64,
    pub total_route_expansions: u64,
    pub elapsed_micros: u64,
    pub trials: Vec<KiCadYieldingConnectionTrial>,
    pub interpretation: String,
}

fn yielding_combinations(
    order: &[String],
    minimum: usize,
    maximum: usize,
    limit: usize,
) -> (Vec<Vec<String>>, usize) {
    let mut combinations = Vec::new();
    if minimum == 1 {
        for connection in order.iter().take(limit) {
            combinations.push(vec![connection.clone()]);
        }
    }
    if maximum == 2 {
        'pairs: for left in 0..order.len() {
            for right in left + 1..order.len() {
                if combinations.len() == limit {
                    break 'pairs;
                }
                combinations.push(vec![order[left].clone(), order[right].clone()]);
            }
        }
    }
    let total = (if minimum == 1 { order.len() } else { 0 })
        + if maximum == 2 {
            order.len().saturating_mul(order.len().saturating_sub(1)) / 2
        } else {
            0
        };
    (combinations, total)
}

fn yielding_priority_order(foreign: &[String], preferred: &[String]) -> Vec<String> {
    let mut remaining: BTreeMap<_, _> = foreign
        .iter()
        .map(|name| (normalize_net(name), name.clone()))
        .collect();
    let mut result = Vec::with_capacity(foreign.len());
    for name in preferred {
        if let Some(original) = remaining.remove(normalize_net(name)) {
            result.push(original);
        }
    }
    result.extend(remaining.into_values());
    result
}

#[cfg(test)]
mod yielding_priority_tests {
    use super::*;

    #[test]
    fn pair_only_coverage_skips_singles_and_keeps_the_declared_trial_bound() {
        let order = vec!["C".into(), "A".into(), "B".into()];
        let (pairs, total) = yielding_combinations(&order, 2, 2, 2);
        assert_eq!(pairs, vec![vec!["C", "A"], vec!["C", "B"]]);
        assert_eq!(total, 3);
        let (mixed, total) = yielding_combinations(&order, 1, 2, 4);
        assert_eq!(mixed, vec![vec!["C"], vec!["A"], vec!["B"], vec!["C", "A"]]);
        assert_eq!(total, 6);
        let (singles, total) = yielding_combinations(&order, 1, 1, 9);
        assert_eq!(singles, vec![vec!["C"], vec!["A"], vec!["B"]]);
        assert_eq!(total, 3);
        let (empty, total) = yielding_combinations(&order[..1], 2, 2, 4);
        assert!(empty.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn hints_reorder_without_dropping_or_inventing_foreign_nets() {
        let foreign = vec!["A".into(), "B".into(), "C".into()];
        assert_eq!(yielding_priority_order(&foreign, &[]), foreign);
        assert_eq!(
            yielding_priority_order(&foreign, &["/C".into(), "unknown".into(), "C".into()]),
            vec!["C", "A", "B"]
        );
        assert_eq!(
            yielding_priority_order(&foreign, &["B".into(), "/A".into()]),
            vec!["B", "A", "C"]
        );
    }
}

/// Tries bounded one- and two-connection rip-up counterfactuals for a target
/// that cannot be routed against fixed retained copper.
///
/// The result is causal search guidance only. It never applies a candidate or
/// claims completeness because the yielded connections are absent from the
/// counterfactual obstacle model.
pub fn diagnose_kicad_yielding_connections(
    pcb_path: &Path,
    target_connection: &str,
    config: &KiCadYieldingConnectionDiagnosisConfig,
) -> Result<KiCadYieldingConnectionDiagnosis, String> {
    let diagnosis_started = Instant::now();
    if config.minimum_yielding_connections == 0
        || config.minimum_yielding_connections > config.maximum_yielding_connections
        || config.maximum_yielding_connections == 0
        || config.maximum_yielding_connections > 2
        || config.maximum_trials == 0
    {
        return Err(
            "yielding diagnosis requires 1 <= minimum <= maximum <= 2 and a positive trial bound"
                .into(),
        );
    }
    validate_grid_route_config(&config.routing)?;
    let source = fs::read_to_string(pcb_path)
        .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
    let pcb = parse(&source)?;
    let model = KiCadRoutingModel::from_pcb(&pcb, target_connection)?;
    let foreign_routed_connections: Vec<_> = model
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.kind == KiCadViaLocalRerouteBlockerKind::ForeignNetCopper)
        .filter_map(|obstacle| obstacle.net.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let base_started = Instant::now();
    let mut base_route_failure = None;
    let (base_candidate, base_error, base_route_expansions, base_reachability_expansions) =
        match route_materialized_connection_detailed(pcb_path, target_connection, &config.routing) {
            Ok(candidate) => {
                let expansions = candidate.expansions;
                let reachability_expansions = candidate.reachability_expansions;
                (
                    Some(candidate),
                    None,
                    Some(expansions),
                    reachability_expansions,
                )
            }
            Err(error) => {
                base_route_failure = error.search_failure().cloned();
                let expansions = base_route_failure.as_ref().map(|f| f.astar_expansions);
                let reachability_expansions = base_route_failure
                    .as_ref()
                    .map_or(0, |f| f.reachability_expansions);
                (
                    None,
                    Some(error.to_string()),
                    expansions,
                    reachability_expansions,
                )
            }
        };
    let base_elapsed_micros = base_started.elapsed().as_micros() as u64;

    let mut routing_cut = None;
    let mut routing_cut_error = None;
    let mut hints = Vec::new();
    if config.prioritize_boundary_blockers && base_candidate.is_none() {
        if let Some(failure) = &base_route_failure
            && failure.kind == KiCadRouteSearchFailureKind::GridDisconnected
        {
            let inspection = KiCadRoutingCutConfig {
                branch: failure.branch,
                routing: config.routing.clone(),
                ..Default::default()
            };
            match inspect_kicad_routing_cut(pcb_path, target_connection, &inspection) {
                Ok(cut) => {
                    hints = cut.yielding_connection_priority();
                    routing_cut = Some(cut);
                }
                Err(error) => routing_cut_error = Some(error),
            }
        } else {
            routing_cut_error = Some("no grid-disconnected branch failure is available for boundary inspection".into());
        }
    }
    hints.extend(config.preferred_yielding_connections.iter().cloned());
    let yielding_connection_order = yielding_priority_order(&foreign_routed_connections, &hints);

    let (combinations, total_combinations) = yielding_combinations(
        &yielding_connection_order,
        config.minimum_yielding_connections,
        config.maximum_yielding_connections,
        config.maximum_trials,
    );
    let truncated = combinations.len() < total_combinations;
    let mut trials = Vec::with_capacity(combinations.len());
    for (ordinal, yielding_connections) in combinations.into_iter().enumerate() {
        let trial_started = Instant::now();
        match route_materialized_connection_with_yielding_detailed(
            pcb_path,
            target_connection,
            &yielding_connections,
            &config.routing,
        ) {
            Ok(candidate) => {
                let quality = route_candidate_quality(&candidate)?;
                trials.push(KiCadYieldingConnectionTrial {
                    ordinal,
                    yielding_connections,
                    route_found: true,
                    reachability_rejected: false,
                    reachability_expansions: candidate.reachability_expansions,
                    route_expansions: Some(candidate.expansions),
                    elapsed_micros: trial_started.elapsed().as_micros() as u64,
                    candidate: Some(candidate),
                    quality: Some(quality),
                    error: None,
                    route_failure: None,
                });
            }
            Err(error) => {
                let route_failure = error.search_failure().cloned();
                let route_expansions = route_failure.as_ref().map(|f| f.astar_expansions);
                let reachability_expansions = route_failure
                    .as_ref()
                    .map_or(0, |f| f.reachability_expansions);
                trials.push(KiCadYieldingConnectionTrial {
                    ordinal,
                    yielding_connections,
                    route_found: false,
                    reachability_rejected: route_failure
                        .as_ref()
                        .is_some_and(|f| f.kind == KiCadRouteSearchFailureKind::GridDisconnected),
                    reachability_expansions,
                    route_expansions,
                    elapsed_micros: trial_started.elapsed().as_micros() as u64,
                    candidate: None,
                    quality: None,
                    error: Some(error.to_string()),
                    route_failure,
                });
            }
        }
    }
    let successful_counterfactuals = trials.iter().filter(|trial| trial.route_found).count();
    let total_route_expansions = base_route_expansions
        .into_iter()
        .chain(trials.iter().filter_map(|trial| trial.route_expansions))
        .map(u64::from)
        .sum();
    let total_reachability_expansions = u64::from(base_reachability_expansions)
        + trials
            .iter()
            .map(|trial| u64::from(trial.reachability_expansions))
            .sum::<u64>();
    Ok(KiCadYieldingConnectionDiagnosis {
        schema_version: 5,
        target_connection: target_connection.to_string(),
        board: pcb_path.to_path_buf(),
        base_route_found: base_candidate.is_some(),
        base_reachability_rejected: base_route_failure.as_ref().is_some_and(|f| f.kind == KiCadRouteSearchFailureKind::GridDisconnected),
        base_reachability_expansions,
        base_route_expansions,
        base_elapsed_micros,
        base_candidate,
        base_error,
        base_route_failure,
        routing_cut,
        routing_cut_error,
        foreign_routed_connections,
        yielding_connection_order,
        minimum_yielding_connections: config.minimum_yielding_connections,
        maximum_yielding_connections: config.maximum_yielding_connections,
        maximum_trials: config.maximum_trials,
        truncated,
        successful_counterfactuals,
        total_reachability_expansions,
        total_route_expansions,
        elapsed_micros: diagnosis_started.elapsed().as_micros() as u64,
        trials,
        interpretation: "counterfactual only: yielded copper must be rerouted and the whole board must pass KiCad before any action can commit".into(),
    })
}

fn route_failure_expansions(error: &str) -> Option<u32> {
    let work = error.rsplit_once(" after ")?.1;
    work.split_whitespace().next()?.parse().ok()
}

fn route_failure_reachability_expansions(error: &str) -> u32 {
    error
        .split_once("reachability preflight used ")
        .and_then(|(_, work)| work.split_whitespace().next())
        .and_then(|work| work.parse().ok())
        .unwrap_or(0)
}

/// Imports one already-routed KiCad connection as a durable route candidate.
///
/// The importer canonicalizes same-layer contacts, then requires one connected
/// acyclic copper graph with uniform trace/via dimensions. It emits a
/// deterministic root-to-terminal paths, split into branches at plated pad
/// contacts. Pad barrels connect these branches without becoming drilled vias.
/// Cycles, dangling copper, and ambiguous pad contacts fail closed rather than
/// being hidden by a guessed spanning tree.
pub fn import_route_candidate_from_pcb(
    pcb_path: &Path,
    connection: &str,
) -> Result<KiCadRouteCandidate, String> {
    let source = fs::read_to_string(pcb_path)
        .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
    let pcb = parse(&source)?;
    import_route_candidate_from_expr(&pcb, connection)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ImportedCopperNode {
    x: u64,
    y: u64,
    layer: usize,
}

impl ImportedCopperNode {
    fn new(at: [f64; 2], layer: usize) -> Self {
        let bits = |value: f64| {
            if value == 0.0 {
                0.0_f64.to_bits()
            } else {
                value.to_bits()
            }
        };
        Self {
            x: bits(at[0]),
            y: bits(at[1]),
            layer,
        }
    }

    fn at(self) -> [f64; 2] {
        [f64::from_bits(self.x), f64::from_bits(self.y)]
    }

    fn layer_name(self) -> &'static str {
        if self.layer == 0 { "F.Cu" } else { "B.Cu" }
    }
}

fn imported_edge_key(
    left: ImportedCopperNode,
    right: ImportedCopperNode,
) -> (ImportedCopperNode, ImportedCopperNode) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn add_imported_edge(
    graph: &mut BTreeMap<ImportedCopperNode, Vec<(ImportedCopperNode, usize)>>,
    edges: &mut BTreeMap<(ImportedCopperNode, ImportedCopperNode), usize>,
    left: ImportedCopperNode,
    right: ImportedCopperNode,
) -> Result<(), String> {
    if left == right {
        return Err("target copper contains a zero-length canonical edge".into());
    }
    let key = imported_edge_key(left, right);
    if edges.contains_key(&key) {
        return Ok(());
    }
    let edge = edges.len();
    edges.insert(key, edge);
    graph.entry(left).or_default().push((right, edge));
    graph.entry(right).or_default().push((left, edge));
    Ok(())
}

fn split_imported_segments_at_vias(branches: &mut [KiCadRouteBranch], via_locations: &[[f64; 2]]) {
    for branch in branches {
        let start = branch.path[0].clone();
        let finish = branch.path[1].clone();
        let mut points = vec![start.at, finish.at];
        points.extend(
            via_locations
                .iter()
                .copied()
                .filter(|point| route_point_on_segment(*point, start.at, finish.at)),
        );
        points.sort_by(|left, right| {
            route_segment_parameter(*left, start.at, finish.at)
                .total_cmp(&route_segment_parameter(*right, start.at, finish.at))
        });
        points.dedup_by(|left, right| {
            distance_squared(*left, *right) <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM
        });
        branch.path = points
            .into_iter()
            .map(|at| KiCadRoutePoint {
                at,
                layer: start.layer.clone(),
            })
            .collect();
    }
}

fn import_route_candidate_from_expr(
    pcb: &Expr,
    connection: &str,
) -> Result<KiCadRouteCandidate, String> {
    let model = KiCadRoutingModel::from_pcb(pcb, connection)?;
    let terminal_contacts = model.electrical_terminals();
    let terminals: Vec<_> = terminal_contacts.iter().map(|contact| contact.at).collect();
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut trace_width_mm: Option<f64> = None;
    let mut via_size_mm: Option<f64> = None;
    let mut via_drill_mm: Option<f64> = None;
    let mut segment_branches = Vec::new();
    let mut via_locations = Vec::new();
    let require_uniform = |slot: &mut Option<f64>, value: f64, dimension: &str| {
        if !value.is_finite() || value <= 0.0 {
            return Err(format!("target copper has invalid {dimension} {value}"));
        }
        if slot.is_some_and(|existing| (existing - value).abs() > 1.0e-9) {
            return Err(format!(
                "KiCad copper importer currently requires uniform {dimension}"
            ));
        }
        *slot = Some(value);
        Ok(())
    };
    for item in items {
        if !node_net(item).is_some_and(|net| normalize_net(net) == connection) {
            continue;
        }
        match item.head() {
            Some("segment") => {
                let layer_name = form_atom(item, "layer", 1)
                    .ok_or_else(|| "target segment has no layer".to_string())?;
                copper_layer_index(layer_name)
                    .ok_or_else(|| format!("unsupported target segment layer {layer_name:?}"))?;
                let start = form_xy(item, "start")?;
                let end = form_xy(item, "end")?;
                if start == end {
                    return Err("target copper contains a zero-length segment".into());
                }
                require_uniform(
                    &mut trace_width_mm,
                    form_f64(item, "width", 1)?,
                    "trace width",
                )?;
                segment_branches.push(KiCadRouteBranch {
                    start_terminal: start,
                    finish_terminal: end,
                    cost: 0,
                    expansions: 0,
                    path: vec![
                        KiCadRoutePoint {
                            at: start,
                            layer: layer_name.into(),
                        },
                        KiCadRoutePoint {
                            at: end,
                            layer: layer_name.into(),
                        },
                    ],
                });
            }
            Some("via") => {
                let layers = item
                    .child("layers")
                    .ok_or_else(|| "target via has no layers".to_string())?;
                let layer_names = layers
                    .children()
                    .iter()
                    .skip(1)
                    .filter_map(Expr::atom)
                    .collect::<Vec<_>>();
                if layer_names != ["F.Cu", "B.Cu"] && layer_names != ["B.Cu", "F.Cu"] {
                    return Err(format!(
                        "KiCad copper importer currently supports only F.Cu/B.Cu vias, found {layer_names:?}"
                    ));
                }
                require_uniform(&mut via_size_mm, form_f64(item, "size", 1)?, "via size")?;
                require_uniform(&mut via_drill_mm, form_f64(item, "drill", 1)?, "via drill")?;
                let at = form_xy(item, "at")?;
                via_locations.push(at);
            }
            Some("arc" | "zone") => {
                return Err(format!(
                    "KiCad copper importer does not yet support target {} objects",
                    item.head().unwrap_or("copper")
                ));
            }
            _ => {}
        }
    }
    if segment_branches.is_empty() && via_locations.is_empty() {
        return Err(format!(
            "connection {connection:?} has no routed copper to import"
        ));
    }

    split_imported_segments_at_vias(&mut segment_branches, &via_locations);
    let selected_segments = (0..segment_branches.len()).collect::<Vec<_>>();
    normalize_selected_route_graph(&mut segment_branches, &selected_segments);
    let mut graph = BTreeMap::<ImportedCopperNode, Vec<(ImportedCopperNode, usize)>>::new();
    let mut edges = BTreeMap::<(ImportedCopperNode, ImportedCopperNode), usize>::new();
    for branch in &segment_branches {
        for pair in branch.path.windows(2) {
            let layer =
                copper_layer_index(&pair[0].layer).expect("imported segment layer was validated");
            add_imported_edge(
                &mut graph,
                &mut edges,
                ImportedCopperNode::new(pair[0].at, layer),
                ImportedCopperNode::new(pair[1].at, layer),
            )?;
        }
    }
    via_locations.sort_by(|left, right| {
        left[0]
            .total_cmp(&right[0])
            .then_with(|| left[1].total_cmp(&right[1]))
    });
    via_locations.dedup();
    for at in &via_locations {
        add_imported_edge(
            &mut graph,
            &mut edges,
            ImportedCopperNode::new(*at, 0),
            ImportedCopperNode::new(*at, 1),
        )?;
    }

    let mut terminal_nodes = Vec::with_capacity(terminals.len());
    let mut pad_node_replacements = BTreeMap::new();
    let mut pad_bridges = BTreeMap::new();
    let mut terminal_owners = BTreeMap::new();
    for contact in &terminal_contacts {
        let terminal = &contact.at;
        let pads = model
            .terminal_pads
            .iter()
            .filter(|pad| distance_squared(pad.at, *terminal) <= 1.0e-12)
            .collect::<Vec<_>>();
        let matches = graph
            .keys()
            .copied()
            .filter(|node| {
                contact.layers[node.layer] && pads.iter().any(|pad| {
                    pad.layers[node.layer]
                        && pad
                            .geometry
                            .contains(node.at(), trace_width_mm.unwrap_or(0.0) / 2.0)
                })
            })
            .collect::<BTreeSet<_>>();
        if matches.is_empty() {
            return Err(format!(
                "terminal pad at {terminal:?} does not overlap canonical copper"
            ));
        }
        let layers = matches
            .iter()
            .map(|node| node.layer)
            .collect::<BTreeSet<_>>();
        if layers.len() > 1
            && !pads
                .iter()
                .any(|pad| pad.plated_through_hole && pad.layers == [true, true])
        {
            return Err(format!(
                "terminal pad at {terminal:?} has multilayer contacts without a plated through-hole pad"
            ));
        }
        // Contract contacts separately on each copper layer. The pad's barrel
        // is an electrical graph edge, never a candidate track or drilled via.
        let mut representatives = Vec::new();
        for layer in layers {
            let layer_matches = matches
                .iter()
                .copied()
                .filter(|node| node.layer == layer)
                .collect::<BTreeSet<_>>();
            let center_node = ImportedCopperNode::new(*terminal, layer);
            let via_contacts = layer_matches
                .iter()
                .copied()
                .filter(|node| {
                    graph[node]
                        .iter()
                        .any(|(neighbor, _)| neighbor.layer != node.layer)
                })
                .collect::<Vec<_>>();
            if via_contacts.len() > 1 {
                return Err(format!(
                    "terminal pad at {terminal:?} contains multiple via contacts and cannot be contracted without choosing topology"
                ));
            }
            let representative = if let Some(via) = via_contacts.first() {
                *via
            } else if layer_matches.contains(&center_node) {
                center_node
            } else {
                *layer_matches
                    .iter()
                    .next()
                    .expect("a terminal layer has matches")
            };
            if let Some(previous) = terminal_owners.insert(representative, *terminal) {
                return Err(format!(
                    "terminal pads at {previous:?} and {terminal:?} resolve to the same copper node"
                ));
            }
            representatives.push(representative);
            for node in layer_matches {
                if let Some(previous) = pad_node_replacements.insert(node, representative)
                    && previous != representative
                {
                    return Err(format!(
                        "terminal pad at {terminal:?} overlaps another terminal's copper contacts"
                    ));
                }
            }
        }
        terminal_nodes.push(representatives[0]);
        if representatives.len() == 2 {
            pad_bridges.insert(
                imported_edge_key(representatives[0], representatives[1]),
                *terminal,
            );
        }
    }
    let original_edges = edges.keys().copied().collect::<Vec<_>>();
    graph.clear();
    edges.clear();
    for (left, right) in original_edges {
        let left = pad_node_replacements.get(&left).copied().unwrap_or(left);
        let right = pad_node_replacements.get(&right).copied().unwrap_or(right);
        if left != right {
            add_imported_edge(&mut graph, &mut edges, left, right)?;
        }
    }
    for &(left, right) in pad_bridges.keys() {
        if edges.contains_key(&imported_edge_key(left, right)) {
            return Err(
                "a physical via duplicates a plated pad connection; topology is ambiguous".into(),
            );
        }
        add_imported_edge(&mut graph, &mut edges, left, right)?;
    }

    let root = terminal_nodes[0];
    let mut parents = BTreeMap::<ImportedCopperNode, Option<(ImportedCopperNode, usize)>>::new();
    parents.insert(root, None);
    let mut queue = VecDeque::from([root]);
    while let Some(node) = queue.pop_front() {
        let parent_edge = parents[&node].map(|(_, edge)| edge);
        for &(neighbor, edge) in &graph[&node] {
            if parent_edge == Some(edge) {
                continue;
            }
            if parents.contains_key(&neighbor) {
                return Err(format!(
                    "target copper graph contains a cycle through {:?} on {}",
                    neighbor.at(),
                    neighbor.layer_name()
                ));
            }
            parents.insert(neighbor, Some((node, edge)));
            queue.push_back(neighbor);
        }
    }
    if parents.len() != graph.len() {
        return Err(format!(
            "target copper graph is disconnected: reached {} of {} nodes",
            parents.len(),
            graph.len()
        ));
    }

    let mut used_edges = BTreeSet::new();
    let mut branches = Vec::new();
    let mut emitted_paths = BTreeSet::new();
    for (terminal_index, &finish_node) in terminal_nodes.iter().enumerate().skip(1) {
        let mut nodes = vec![finish_node];
        let mut current = finish_node;
        while current != root {
            let Some((parent, edge)) = parents[&current] else {
                return Err("target terminal is not connected to the selected root".into());
            };
            used_edges.insert(edge);
            current = parent;
            nodes.push(current);
        }
        nodes.reverse();
        let mut start = 0;
        let mut start_terminal = terminals[0];
        // Split at pad barrels: independently anchored branches retain the
        // original contact coordinates on each layer, even when they differ.
        for end in 0..nodes.len() {
            let bridge_terminal = nodes.get(end + 1).and_then(|&next| {
                pad_bridges
                    .get(&imported_edge_key(nodes[end], next))
                    .copied()
            });
            if bridge_terminal.is_none() && end + 1 != nodes.len() {
                continue;
            }
            let finish_terminal = bridge_terminal.unwrap_or(terminals[terminal_index]);
            let path = &nodes[start..=end];
            if path.len() > 1 && emitted_paths.insert(path.to_vec()) {
                branches.push(KiCadRouteBranch {
                    start_terminal,
                    finish_terminal,
                    cost: 0,
                    expansions: 0,
                    path: path
                        .iter()
                        .map(|node| KiCadRoutePoint {
                            at: node.at(),
                            layer: node.layer_name().into(),
                        })
                        .collect(),
                });
            }
            start = end + 1;
            start_terminal = finish_terminal;
        }
    }
    if used_edges.len() != edges.len() {
        return Err(format!(
            "target copper graph has dangling material not needed by any terminal: root paths use {} of {} canonical edges",
            used_edges.len(),
            edges.len()
        ));
    }

    let defaults = KiCadGridRouteConfig::default();
    let config = KiCadGridRouteConfig {
        // A via-only connection has no active trace-width rule to recover.
        trace_width_mm: trace_width_mm.unwrap_or(defaults.trace_width_mm),
        via_size_mm: via_size_mm.unwrap_or(defaults.via_size_mm),
        via_drill_mm: via_drill_mm.unwrap_or(defaults.via_drill_mm),
        ..defaults
    };
    validate_grid_route_config(&config)?;
    let (supplemental_segments, supplemental_vias) =
        materialize_candidate_branches(connection, &branches, &config)?;
    let canonical_segments = edges
        .keys()
        .filter(|(left, right)| left.layer == right.layer)
        .count();
    let canonical_vias = edges.len() - canonical_segments - pad_bridges.len();
    if supplemental_segments.len() != canonical_segments
        || supplemental_vias.len() != canonical_vias
    {
        return Err(format!(
            "target copper does not round-trip canonically: {canonical_segments} segment edge(s)/{canonical_vias} via edge(s) became {}/{}",
            supplemental_segments.len(),
            supplemental_vias.len()
        ));
    }
    let width = (((model.bounds[2] - model.bounds[0]) / config.resolution_mm).ceil() as usize)
        .saturating_add(1);
    let height = (((model.bounds[3] - model.bounds[1]) / config.resolution_mm).ceil() as usize)
        .saturating_add(1);
    Ok(KiCadRouteCandidate {
        schema_version: 2,
        connection: connection.to_string(),
        router: "kicad-board-copper-tree-import-v2".into(),
        config,
        terminals: terminals,
        grid_origin: [model.bounds[0], model.bounds[1]],
        grid_alignment_evidence: None,
        heuristic_expansions: None,
        grid_size: [width, height],
        cost: 0,
        reachability_expansions: 0,
        expansions: 0,
        branches,
        tree_attachment_attempts: Vec::new(),
        supplemental_segments,
        supplemental_vias,
        footprint_placements: Vec::new(),
        reference_placements: Vec::new(),
    })
}

fn reserve_via_hole_spacing(
    via_enter_costs: &mut [u32],
    width: usize,
    height: usize,
    origin: [f64; 2],
    via: [f64; 2],
    config: &KiCadGridRouteConfig,
    safety: f64,
) {
    let forbidden_radius = config.via_drill_mm + config.hole_to_hole_clearance_mm + safety;
    let plane = width * height;
    for y in 0..height {
        for x in 0..width {
            let point = [
                origin[0] + x as f64 * config.resolution_mm,
                origin[1] + y as f64 * config.resolution_mm,
            ];
            if distance_squared(point, via) < forbidden_radius.powi(2) {
                via_enter_costs[y * width + x] = u32::MAX;
                via_enter_costs[plane + y * width + x] = u32::MAX;
            }
        }
    }
}

/// Replaces one connection's routed copper in a PCB with a persisted route
/// candidate. This is intended for temporary exact validation before a route
/// is promoted into the ladder declaration.
pub fn apply_route_candidate(
    pcb_path: &Path,
    candidate_path: &Path,
    output_path: &Path,
) -> Result<(), String> {
    apply_route_candidate_with_yielding_connections(pcb_path, candidate_path, &[], output_path)
}

/// Applies a diagnostic route while removing selected foreign connections'
/// copper from the output board.
///
/// The resulting board is intentionally incomplete until every yielded
/// connection has been rerouted. Callers must retain it as a provisional
/// artifact and run the full native gate after all routes are restored.
pub fn apply_route_candidate_with_yielding_connections(
    pcb_path: &Path,
    candidate_path: &Path,
    yielding_connections: &[String],
    output_path: &Path,
) -> Result<(), String> {
    let pcb_source = fs::read_to_string(pcb_path)
        .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
    let candidate = read_route_candidate(candidate_path)?;
    if yielding_connections
        .iter()
        .any(|yielding| yielding == &candidate.connection)
    {
        return Err("the applied target connection cannot yield to itself".into());
    }
    let yielding: BTreeSet<_> = yielding_connections.iter().map(String::as_str).collect();
    let mut pcb = parse(&pcb_source)?;
    let copper_net_names = unambiguous_pad_net_names(&pcb)?;
    apply_footprint_placements(&mut pcb, &candidate.footprint_placements)?;
    apply_reference_placements(&mut pcb, &candidate.reference_placements)?;
    // Parsing the routing model also proves the candidate still names a net
    // with at least two pads in this exact board revision.
    KiCadRoutingModel::from_pcb(&pcb, &candidate.connection)?;
    let Expr::List(items) = &mut pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| {
        !matches!(item.head(), Some("segment" | "arc" | "via" | "zone"))
            || !node_net(item).is_some_and(|net| {
                let net = normalize_net(net);
                net == candidate.connection || yielding.contains(net)
            })
    });
    items.extend(
        candidate
            .supplemental_segments
            .iter()
            .map(supplemental_segment_expr),
    );
    items.extend(
        candidate
            .supplemental_vias
            .iter()
            .map(supplemental_via_expr),
    );
    canonicalize_copper_net_names(&mut pcb, &copper_net_names);
    if let Some(parent) = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    fs::write(output_path, format!("{}\n", encode(&pcb)))
        .map_err(|error| format!("failed to write {}: {error}", output_path.display()))
}

/// Adds source-checked alternate local poses for visible footprint reference
/// fields named by native KiCad `silk_overlap` findings. The returned
/// candidate remains provisional until reapplied to a complete board and
/// accepted by the native gate.
pub fn relax_reference_fields_from_drc(
    pcb_path: &Path,
    candidate_path: &Path,
    drc_path: &Path,
    iteration: usize,
) -> Result<(KiCadRouteCandidate, KiCadReferenceRelaxationEvidence), String> {
    if iteration == 0 {
        return Err("KiCad reference-field relaxation iteration must be positive".into());
    }
    let pcb_source = fs::read_to_string(pcb_path)
        .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
    let mut pcb = parse(&pcb_source)?;
    let mut candidate = read_route_candidate(candidate_path)?;
    apply_footprint_placements(&mut pcb, &candidate.footprint_placements)?;
    apply_reference_placements(&mut pcb, &candidate.reference_placements)?;
    let drc = read_json(drc_path)?;

    let mut selected = BTreeMap::<String, String>::new();
    let mut silk_overlap_findings = 0;
    for violation in drc["violations"].as_array().into_iter().flatten() {
        if violation["type"].as_str() != Some("silk_overlap") {
            continue;
        }
        silk_overlap_findings += 1;
        for item in violation["items"].as_array().into_iter().flatten() {
            let Some(reference) = item["description"]
                .as_str()
                .and_then(|description| description.strip_prefix("Reference field of "))
            else {
                continue;
            };
            let Some(uuid) = item["uuid"].as_str() else {
                continue;
            };
            selected.insert(uuid.to_string(), reference.to_string());
        }
    }

    let mut current_fields = BTreeMap::<String, (String, [f64; 3])>::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = footprint_reference(footprint).unwrap_or_default();
        for property in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("property"))
        {
            if property.children().get(1).and_then(Expr::atom) != Some("Reference")
                || form_atom(property, "layer", 1) != Some("F.SilkS")
                || property.child("hide").is_some()
            {
                continue;
            }
            let Some(uuid) = form_atom(property, "uuid", 1) else {
                continue;
            };
            if selected.contains_key(uuid) {
                current_fields.insert(uuid.to_string(), (reference.clone(), form_at(property)?));
            }
        }
    }
    let missing = selected
        .keys()
        .filter(|uuid| !current_fields.contains_key(*uuid))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "KiCad DRC names reference field UUID(s) absent from the source board: {}",
            missing.join(", ")
        ));
    }

    let signatures = (1..=4)
        .map(|phase| reference_action_signature(&current_fields, phase))
        .collect::<Vec<_>>();
    let unique_alternate_actions = signatures.iter().cloned().collect::<BTreeSet<_>>().len();
    let phase = (iteration - 1) % 4 + 1;
    let duplicate_of_phase =
        (1..phase).find(|earlier| signatures[*earlier - 1] == signatures[phase - 1]);

    let mut placements = candidate
        .reference_placements
        .iter()
        .map(|placement| (placement.uuid.clone(), placement.clone()))
        .collect::<BTreeMap<_, _>>();
    for (uuid, drc_reference) in selected {
        let (reference, current) = &current_fields[&uuid];
        if reference != &drc_reference {
            return Err(format!(
                "KiCad DRC calls reference UUID {uuid} {drc_reference}, but the source board calls it {reference}"
            ));
        }
        let source_at = placements
            .get(&uuid)
            .map_or(*current, |placement| placement.source_at);
        placements.insert(
            uuid.clone(),
            KiCadReferencePlacement {
                reference: reference.clone(),
                uuid,
                source_at,
                at: alternate_reference_pose(*current, iteration),
            },
        );
    }
    candidate.reference_placements = placements.into_values().collect();
    candidate.reference_placements.sort_by(|left, right| {
        left.reference
            .cmp(&right.reference)
            .then_with(|| left.uuid.cmp(&right.uuid))
    });
    let evidence = KiCadReferenceRelaxationEvidence {
        schema_version: 1,
        algorithm: "testing-esp32-duts-reference-alternates-v1".into(),
        iteration,
        phase,
        silk_overlap_findings,
        selected_reference_fields: current_fields.len(),
        unique_alternate_actions,
        duplicate_of_phase,
        placements: candidate.reference_placements.clone(),
        exact_kicad_validation_required: true,
    };
    Ok((candidate, evidence))
}

/// Enumerates the predecessor reference-field actions, removes equivalent
/// candidates, runs every unique candidate through a fresh native KiCad gate,
/// and retains the complete attempt directories for inspection.
///
/// The source directory is never modified. The output directory must not
/// already exist, which prevents a new portfolio from silently mixing with
/// stale attempts. Selection prefers canonical route length, via count,
/// reference displacement, and finally the predecessor phase order. Native
/// completeness is a hard prerequisite, not part of that soft score.
pub fn search_reference_field_actions(
    source_directory: &Path,
    board_id: &str,
    candidate_path: &Path,
    output_directory: &Path,
) -> Result<
    (
        Option<KiCadRouteCandidate>,
        KiCadReferenceActionPortfolioEvidence,
    ),
    String,
> {
    if board_id.is_empty() {
        return Err("KiCad reference-action portfolio board ID must not be empty".into());
    }
    if !source_directory.is_dir() {
        return Err(format!(
            "KiCad reference-action source directory {} does not exist",
            source_directory.display()
        ));
    }
    if output_directory.exists() {
        return Err(format!(
            "KiCad reference-action output directory {} already exists",
            output_directory.display()
        ));
    }
    let source_directory = fs::canonicalize(source_directory).map_err(|error| {
        format!(
            "failed to canonicalize KiCad source directory {}: {error}",
            source_directory.display()
        )
    })?;
    let output_parent = output_directory
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let output_parent = fs::canonicalize(output_parent).map_err(|error| {
        format!(
            "KiCad reference-action output parent {} must already exist: {error}",
            output_parent.display()
        )
    })?;
    let output_name = output_directory.file_name().ok_or_else(|| {
        format!(
            "KiCad reference-action output {} has no directory name",
            output_directory.display()
        )
    })?;
    let output_directory = output_parent.join(output_name);
    if output_directory.starts_with(&source_directory) {
        return Err("KiCad reference-action output must not be inside its source directory".into());
    }
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    if !source_board.is_file() {
        return Err(format!(
            "KiCad reference-action source board {} does not exist",
            source_board.display()
        ));
    }
    let source_candidate = read_route_candidate(candidate_path)?;
    fs::create_dir(&output_directory).map_err(|error| {
        format!(
            "failed to create KiCad reference-action output {}: {error}",
            output_directory.display()
        )
    })?;
    let source_artifact_name = "source-candidate";
    let source_artifact_directory = output_directory.join(source_artifact_name);
    copy_directory_tree(&source_directory, &source_artifact_directory)?;
    let source_candidate_path = source_artifact_directory.join("candidate.json");
    write_pretty_json(&source_candidate_path, &source_candidate)?;
    let source_artifact_board = source_artifact_directory.join(format!("{board_id}.kicad_pcb"));
    apply_route_candidate(
        &source_artifact_board,
        &source_candidate_path,
        &source_artifact_board,
    )?;
    let source_verification = verify_materialized_rung(&source_artifact_directory, board_id)?;

    if source_verification.complete {
        return Ok((
            Some(source_candidate.clone()),
            KiCadReferenceActionPortfolioEvidence {
                schema_version: 1,
                algorithm: "native-kicad-reference-action-portfolio-v1".into(),
                board_id: board_id.into(),
                connection: source_candidate.connection.clone(),
                source_artifact_directory: PathBuf::from(source_artifact_name),
                source_verification,
                generated_actions: 0,
                unique_actions: 0,
                duplicate_actions: Vec::new(),
                attempted_actions: 0,
                exact_complete_actions: 0,
                selected_source: true,
                selected_attempt: None,
                selected_phase: None,
                complete: true,
                selection_rule: "native completeness, canonical route length, via count, total reference displacement, predecessor phase".into(),
                attempts: Vec::new(),
            },
        ));
    }

    let authoritative_drc = source_artifact_directory.join("drc.json");
    let mut attempts = Vec::new();
    let mut duplicate_actions = Vec::new();
    let mut generated_actions = 0;
    let mut unique_actions = 0;
    for phase in 1..=4 {
        let (candidate, relaxation) = relax_reference_fields_from_drc(
            &source_board,
            candidate_path,
            &authoritative_drc,
            phase,
        )?;
        if relaxation.selected_reference_fields == 0 {
            break;
        }
        generated_actions = 4;
        if let Some(duplicate_of_phase) = relaxation.duplicate_of_phase {
            duplicate_actions.push(KiCadReferenceActionDuplicate {
                phase,
                duplicate_of_phase,
            });
            continue;
        }
        unique_actions = relaxation.unique_alternate_actions;
        let id = format!("phase-{phase:02}");
        let artifact_name = format!("attempt-{id}");
        let artifact_directory = output_directory.join(&artifact_name);
        copy_directory_tree(&source_directory, &artifact_directory)?;
        let attempt_candidate_path = artifact_directory.join("candidate.json");
        write_pretty_json(&attempt_candidate_path, &candidate)?;
        let attempt_board = artifact_directory.join(format!("{board_id}.kicad_pcb"));
        apply_route_candidate(&attempt_board, &attempt_candidate_path, &attempt_board)?;
        let verification = verify_materialized_rung(&artifact_directory, board_id)?;
        let route_quality = route_candidate_quality(&candidate)?;
        let reference_displacement_mm = candidate
            .reference_placements
            .iter()
            .map(|placement| {
                let dx = placement.at[0] - placement.source_at[0];
                let dy = placement.at[1] - placement.source_at[1];
                dx.hypot(dy)
            })
            .sum();
        attempts.push(KiCadReferenceActionAttempt {
            id,
            phase,
            artifact_directory: PathBuf::from(artifact_name),
            candidate,
            relaxation,
            verification,
            route_quality,
            reference_displacement_mm,
            selected: false,
        });
    }

    let selected_attempt = attempts
        .iter()
        .enumerate()
        .filter(|(_, attempt)| attempt.verification.complete)
        .min_by(|(_, left), (_, right)| {
            left.route_quality
                .length_mm
                .total_cmp(&right.route_quality.length_mm)
                .then_with(|| left.route_quality.vias.cmp(&right.route_quality.vias))
                .then_with(|| {
                    left.reference_displacement_mm
                        .total_cmp(&right.reference_displacement_mm)
                })
                .then_with(|| left.phase.cmp(&right.phase))
        })
        .map(|(index, _)| index);
    if let Some(selected_attempt) = selected_attempt {
        attempts[selected_attempt].selected = true;
    }
    let selected_candidate = selected_attempt.map(|index| attempts[index].candidate.clone());
    let exact_complete_actions = attempts
        .iter()
        .filter(|attempt| attempt.verification.complete)
        .count();
    let selected_phase = selected_attempt.map(|index| attempts[index].phase);
    let complete = selected_attempt.is_some();
    let evidence = KiCadReferenceActionPortfolioEvidence {
        schema_version: 1,
        algorithm: "native-kicad-reference-action-portfolio-v1".into(),
        board_id: board_id.into(),
        connection: source_candidate.connection,
        source_artifact_directory: PathBuf::from(source_artifact_name),
        source_verification,
        generated_actions,
        unique_actions,
        duplicate_actions,
        attempted_actions: attempts.len(),
        exact_complete_actions,
        selected_source: false,
        selected_attempt,
        selected_phase,
        complete,
        selection_rule: "native completeness, canonical route length, via count, total reference displacement, predecessor phase".into(),
        attempts,
    };
    Ok((selected_candidate, evidence))
}

#[derive(Clone, Debug)]
struct KiCadBranchTopologyPoint {
    at: [f64; 2],
    was_via: bool,
}

#[derive(Clone, Debug)]
struct KiCadBranchTopology {
    points: Vec<KiCadBranchTopologyPoint>,
    segment_layers: Vec<String>,
    start_layer: String,
    finish_layer: String,
}

fn branch_topology(path: &[KiCadRoutePoint]) -> Result<KiCadBranchTopology, String> {
    let Some(first) = path.first() else {
        return Err("route branch has no points".into());
    };
    let finish_layer = path
        .last()
        .expect("the branch has a first point")
        .layer
        .clone();
    let mut points = vec![KiCadBranchTopologyPoint {
        at: first.at,
        was_via: false,
    }];
    let mut segment_layers = Vec::new();
    for (segment, pair) in path.windows(2).enumerate() {
        if pair[0].layer != pair[1].layer {
            if pair[0].at != pair[1].at {
                return Err(format!(
                    "route branch layer transition {segment} changes coordinate"
                ));
            }
            continue;
        }
        if pair[0].at == pair[1].at {
            return Err(format!("route branch segment {segment} has zero length"));
        }
        if points.last().expect("non-empty topology").at != pair[0].at {
            return Err(format!(
                "route branch segment {segment} is discontinuous after collapsing vias"
            ));
        }
        segment_layers.push(pair[0].layer.clone());
        points.push(KiCadBranchTopologyPoint {
            at: pair[1].at,
            was_via: false,
        });
    }
    if segment_layers.len() + 1 != points.len() || segment_layers.is_empty() {
        return Err("route branch has no geometric copper segment".into());
    }
    for point in 1..points.len() - 1 {
        points[point].was_via = segment_layers[point - 1] != segment_layers[point];
    }
    Ok(KiCadBranchTopology {
        points,
        segment_layers,
        start_layer: first.layer.clone(),
        finish_layer,
    })
}

fn topology_path(topology: &KiCadBranchTopology) -> Vec<KiCadRoutePoint> {
    let mut path = vec![KiCadRoutePoint {
        at: topology.points[0].at,
        layer: topology.start_layer.clone(),
    }];
    if topology.start_layer != topology.segment_layers[0] {
        path.push(KiCadRoutePoint {
            at: topology.points[0].at,
            layer: topology.segment_layers[0].clone(),
        });
    }
    for (segment, layer) in topology.segment_layers.iter().enumerate() {
        if path.last().expect("non-empty path").layer != *layer {
            path.push(KiCadRoutePoint {
                at: topology.points[segment].at,
                layer: layer.clone(),
            });
        }
        path.push(KiCadRoutePoint {
            at: topology.points[segment + 1].at,
            layer: layer.clone(),
        });
    }
    if path.last().expect("non-empty path").layer != topology.finish_layer {
        path.push(KiCadRoutePoint {
            at: topology.points.last().expect("non-empty topology").at,
            layer: topology.finish_layer.clone(),
        });
    }
    path
}

fn target_layer_pair_matches(left: &str, right: &str, target: &[String; 2]) -> bool {
    (left == target[0] && right == target[1]) || (left == target[1] && right == target[0])
}

#[derive(Clone, Debug)]
struct KiCadViaRemovalSpan {
    start: [f64; 2],
    finish: [f64; 2],
    layer: String,
}

#[derive(Debug)]
enum KiCadViaLocalRerouteError {
    Plain(String),
    Search(Box<KiCadViaLocalRerouteFailureEvidence>),
}

impl From<String> for KiCadViaLocalRerouteError {
    fn from(detail: String) -> Self {
        Self::Plain(detail)
    }
}

impl From<&str> for KiCadViaLocalRerouteError {
    fn from(detail: &str) -> Self {
        Self::Plain(detail.into())
    }
}

#[derive(Debug)]
struct KiCadViaActionApplicationError {
    detail: String,
    affected_branches: Vec<usize>,
    local_reroute: Option<Box<KiCadViaLocalRerouteEvidence>>,
    diagnostic_candidate: Option<Box<KiCadRouteCandidate>>,
}

impl From<String> for KiCadViaActionApplicationError {
    fn from(detail: String) -> Self {
        Self {
            detail,
            affected_branches: Vec::new(),
            local_reroute: None,
            diagnostic_candidate: None,
        }
    }
}

impl From<&str> for KiCadViaActionApplicationError {
    fn from(detail: &str) -> Self {
        Self::from(detail.to_string())
    }
}

// KiCad stores integer nanometres. A displayed physical coordinate must match
// its grid-generated JSON coordinate despite binary floating-point roundoff.
fn same_native_position(a: [f64; 2], b: [f64; 2]) -> bool {
    (0..2).all(|i| {
        a[i].is_finite()
            && b[i].is_finite()
            && (a[i] * 1_000_000.0).round() == (b[i] * 1_000_000.0).round()
    })
}

fn remove_physical_via_from_branch_with_span(
    branch: &mut KiCadRouteBranch,
    target_at: [f64; 2],
    target_layers: &[String; 2],
    merged_layer: &str,
) -> Result<Option<KiCadViaRemovalSpan>, String> {
    let mut topology = branch_topology(&branch.path)?;
    let matches = (1..topology.points.len() - 1)
        .filter(|&point| {
            same_native_position(topology.points[point].at, target_at)
                && topology.points[point].was_via
                && target_layer_pair_matches(
                    &topology.segment_layers[point - 1],
                    &topology.segment_layers[point],
                    target_layers,
                )
        })
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err("target via occurs more than once in one branch".into());
    }
    let Some(point) = matches.first().copied() else {
        return Ok(None);
    };
    let left_layer = topology.segment_layers[point - 1].clone();
    let right_layer = topology.segment_layers[point].clone();
    if merged_layer != left_layer && merged_layer != right_layer {
        return Err(format!(
            "merged layer {merged_layer:?} is not incident to target via"
        ));
    }
    let mut run_start = point - 1;
    while run_start > 0 && topology.segment_layers[run_start - 1] == left_layer {
        run_start -= 1;
    }
    let mut run_end = point + 1;
    while run_end < topology.segment_layers.len() && topology.segment_layers[run_end] == right_layer
    {
        run_end += 1;
    }
    topology.segment_layers[run_start..run_end].fill(merged_layer.to_string());
    topology.points.remove(point);
    topology.segment_layers.remove(point);
    // `seam` is the boundary which used to contain the target via. Track it
    // through redundant neighboring-via removal, then expand to the maximal
    // same-layer run created by the merge.
    let mut seam = point;

    let mut candidate = 1;
    while candidate + 1 < topology.points.len() {
        if topology.points[candidate].was_via
            && topology.segment_layers[candidate - 1] == topology.segment_layers[candidate]
        {
            topology.points.remove(candidate);
            topology.segment_layers.remove(candidate);
            if candidate < seam {
                seam -= 1;
            }
        } else {
            candidate += 1;
        }
    }
    if seam == 0 || seam >= topology.points.len() {
        return Err("via removal lost its internal merged-run seam".into());
    }
    let mut first_segment = seam - 1;
    while first_segment > 0 && topology.segment_layers[first_segment - 1] == merged_layer {
        first_segment -= 1;
    }
    let mut segment_after_last = seam;
    while segment_after_last < topology.segment_layers.len()
        && topology.segment_layers[segment_after_last] == merged_layer
    {
        segment_after_last += 1;
    }
    let span = KiCadViaRemovalSpan {
        start: topology.points[first_segment].at,
        finish: topology.points[segment_after_last].at,
        layer: merged_layer.to_string(),
    };
    if span.start == span.finish {
        return Err("via removal produced a zero-length merged run".into());
    }
    branch.path = topology_path(&topology);
    Ok(Some(span))
}

fn remove_physical_via_from_branch(
    branch: &mut KiCadRouteBranch,
    target_at: [f64; 2],
    target_layers: &[String; 2],
    merged_layer: &str,
) -> Result<bool, String> {
    Ok(
        remove_physical_via_from_branch_with_span(branch, target_at, target_layers, merged_layer)?
            .is_some(),
    )
}

fn relocate_physical_via_in_branch(
    branch: &mut KiCadRouteBranch,
    target_at: [f64; 2],
    target_layers: &[String; 2],
    to: [f64; 2],
) -> Result<bool, String> {
    let matches = branch
        .path
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| {
            same_native_position(pair[0].at, target_at)
                && same_native_position(pair[1].at, target_at)
                && pair[0].layer != pair[1].layer
                && target_layer_pair_matches(&pair[0].layer, &pair[1].layer, target_layers)
        })
        .map(|(transition, _)| transition)
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err("target via occurs more than once in one branch".into());
    }
    let Some(transition) = matches.first().copied() else {
        return Ok(false);
    };
    if transition == 0 || transition + 2 >= branch.path.len() {
        return Err("target via is not an internal free route point".into());
    }
    if to == branch.path[transition - 1].at || to == branch.path[transition + 2].at {
        return Err("relocated via would create a zero-length copper segment".into());
    }
    branch.path[transition].at = to;
    branch.path[transition + 1].at = to;
    Ok(true)
}

fn replace_merged_branch_run(
    branch: &mut KiCadRouteBranch,
    span: &KiCadViaRemovalSpan,
    replacement: &[KiCadRoutePoint],
) -> Result<(), String> {
    if replacement.len() < 2
        || replacement
            .first()
            .is_none_or(|point| point.at != span.start || point.layer != span.layer)
        || replacement
            .last()
            .is_none_or(|point| point.at != span.finish || point.layer != span.layer)
        || replacement.iter().any(|point| point.layer != span.layer)
    {
        return Err("local reroute does not match its merged-run endpoints and layer".into());
    }
    let mut topology = branch_topology(&branch.path)?;
    let mut matches = Vec::new();
    for start in 0..topology.points.len() {
        if topology.points[start].at != span.start {
            continue;
        }
        for finish in start + 1..topology.points.len() {
            if topology.points[finish].at == span.finish
                && topology.segment_layers[start..finish]
                    .iter()
                    .all(|layer| layer == &span.layer)
            {
                matches.push((start, finish));
            }
        }
    }
    if matches.len() != 1 {
        return Err(format!(
            "merged run {:?} -> {:?} on {} occurs {} times in the edited branch",
            span.start,
            span.finish,
            span.layer,
            matches.len()
        ));
    }
    let (start, finish) = matches[0];
    let start_point = topology.points[start].clone();
    let finish_point = topology.points[finish].clone();
    let mut points = Vec::with_capacity(replacement.len());
    points.push(start_point);
    points.extend(replacement[1..replacement.len() - 1].iter().map(|point| {
        KiCadBranchTopologyPoint {
            at: point.at,
            was_via: false,
        }
    }));
    points.push(finish_point);
    let replacement_segments = vec![span.layer.clone(); points.len() - 1];
    topology.points.splice(start..=finish, points);
    topology
        .segment_layers
        .splice(start..finish, replacement_segments);
    branch.path = topology_path(&topology);
    Ok(())
}

fn local_grid_point(position: GridPosition, origin: [f64; 2], resolution_mm: f64) -> [f64; 2] {
    [
        origin[0] + position.x as f64 * resolution_mm,
        origin[1] + position.y as f64 * resolution_mm,
    ]
}

fn blocked_grid_runs(
    blocked: &[bool],
    width: usize,
    height: usize,
) -> Vec<KiCadViaLocalRerouteBlockedRun> {
    let mut runs = Vec::new();
    for row in 0..height {
        let mut column = 0;
        while column < width {
            if !blocked[row * width + column] {
                column += 1;
                continue;
            }
            let first_column = column;
            while column + 1 < width && blocked[row * width + column + 1] {
                column += 1;
            }
            runs.push(KiCadViaLocalRerouteBlockedRun {
                row,
                first_column,
                last_column: column,
            });
            column += 1;
        }
    }
    runs
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LocalRerouteBlockerKey {
    kind: KiCadViaLocalRerouteBlockerKind,
    object: String,
    footprint: Option<String>,
    component_movable: bool,
}

#[derive(Default)]
struct LocalRerouteBlockerAccumulator {
    nets: BTreeSet<String>,
    frontier_hits: usize,
    sum: [f64; 2],
}

impl LocalRerouteBlockerAccumulator {
    fn record(&mut self, point: [f64; 2], nets: BTreeSet<String>) {
        self.nets.extend(nets);
        self.frontier_hits += 1;
        self.sum[0] += point[0];
        self.sum[1] += point[1];
    }
}

fn local_reroute_blockers(
    model: &KiCadRoutingModel,
    layer: usize,
    origin: [f64; 2],
    resolution_mm: f64,
    obstacle_inflate_mm: f64,
    clearance_mm: f64,
    blocked_frontier: &[GridPosition],
) -> (Vec<KiCadViaLocalRerouteBlockerEvidence>, usize) {
    let mut hits = BTreeMap::<LocalRerouteBlockerKey, LocalRerouteBlockerAccumulator>::new();
    let mut attributed_frontier_states = 0;
    for &position in blocked_frontier {
        if position.layer != 0 {
            continue;
        }
        let point = local_grid_point(position, origin, resolution_mm);
        let mut matched = BTreeMap::<LocalRerouteBlockerKey, BTreeSet<String>>::new();
        for obstacle in &model.obstacles {
            if !obstacle.blocks_tracks
                || !obstacle.layers[layer]
                || !obstacle
                    .geometry
                    .contains(point, obstacle.inflate(obstacle_inflate_mm, clearance_mm))
            {
                continue;
            }
            let key = LocalRerouteBlockerKey {
                kind: obstacle.kind,
                object: obstacle.object.clone(),
                footprint: obstacle.footprint.clone(),
                component_movable: obstacle.component_movable,
            };
            if let Some(net) = &obstacle.net {
                matched.entry(key).or_default().insert(net.clone());
            } else {
                matched.entry(key).or_default();
            }
        }
        if !matched.is_empty() {
            attributed_frontier_states += 1;
        }
        for (key, nets) in matched {
            hits.entry(key).or_default().record(point, nets);
        }
    }
    let mut blockers = hits
        .into_iter()
        .map(|(key, accumulator)| KiCadViaLocalRerouteBlockerEvidence {
            kind: key.kind,
            object: key.object,
            nets: accumulator.nets.into_iter().collect(),
            footprint: key.footprint,
            component_movable: key.component_movable,
            frontier_hits: accumulator.frontier_hits,
            frontier_centroid: [
                accumulator.sum[0] / accumulator.frontier_hits as f64,
                accumulator.sum[1] / accumulator.frontier_hits as f64,
            ],
        })
        .collect::<Vec<_>>();
    blockers.sort_by(|left, right| {
        right
            .frontier_hits
            .cmp(&left.frontier_hits)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.object.cmp(&right.object))
    });
    (blockers, attributed_frontier_states)
}

#[allow(clippy::too_many_arguments)]
fn local_reroute_failure_evidence(
    kind: KiCadViaLocalRerouteFailureKind,
    detail: String,
    request: &GridRouteRequest,
    model: &KiCadRoutingModel,
    layer: usize,
    origin: [f64; 2],
    resolution_mm: f64,
    obstacle_inflate_mm: f64,
    clearance_mm: f64,
    branch_index: usize,
    span: &KiCadViaRemovalSpan,
    searches: usize,
    expansions: u64,
    exact_edge_retries: usize,
    blocked_frontier: Vec<GridPosition>,
) -> KiCadViaLocalRerouteFailureEvidence {
    let (blockers, attributed_frontier_states) = local_reroute_blockers(
        model,
        layer,
        origin,
        resolution_mm,
        obstacle_inflate_mm,
        clearance_mm,
        &blocked_frontier,
    );
    let mut frontier_grid = vec![false; request.width * request.height];
    for point in blocked_frontier {
        if point.layer == 0 && point.x < request.width && point.y < request.height {
            frontier_grid[point.y * request.width + point.x] = true;
        }
    }
    let blocked_frontier_states = frontier_grid.iter().filter(|blocked| **blocked).count();
    let blocked_frontier_runs = blocked_grid_runs(&frontier_grid, request.width, request.height);
    KiCadViaLocalRerouteFailureEvidence {
        branch_index,
        kind,
        detail,
        start: span.start,
        finish: span.finish,
        layer: span.layer.clone(),
        resolution_mm,
        obstacle_inflate_mm,
        window_bounds: [
            origin[0],
            origin[1],
            origin[0] + (request.width - 1) as f64 * resolution_mm,
            origin[1] + (request.height - 1) as f64 * resolution_mm,
        ],
        grid_origin: origin,
        grid_size: [request.width, request.height],
        grid_states: request.width * request.height,
        blocked_states: request.blocked.iter().filter(|blocked| **blocked).count(),
        blocked_runs: blocked_grid_runs(&request.blocked, request.width, request.height),
        searches,
        expansions,
        exact_edge_retries,
        blocked_frontier_states,
        blocked_frontier_runs,
        attributed_frontier_states,
        unattributed_frontier_states: blocked_frontier_states
            .saturating_sub(attributed_frontier_states),
        blockers,
    }
}

fn route_local_merged_run(
    model: &KiCadRoutingModel,
    route_config: &KiCadGridRouteConfig,
    search_config: &KiCadViaLocalRerouteSearchConfig,
    resolution_mm: f64,
    branch_index: usize,
    span: &KiCadViaRemovalSpan,
) -> Result<(Vec<KiCadRoutePoint>, KiCadViaLocalRerouteBranchEvidence), KiCadViaLocalRerouteError> {
    let layer = copper_layer_index(&span.layer)
        .ok_or_else(|| format!("unsupported local reroute layer {:?}", span.layer))?;
    let trace_radius = route_config.trace_width_mm / 2.0;
    let edge_margin = route_config.edge_clearance_mm + trace_radius;
    let board_bounds = [
        model.bounds[0] + edge_margin,
        model.bounds[1] + edge_margin,
        model.bounds[2] - edge_margin,
        model.bounds[3] - edge_margin,
    ];
    let requested = [
        span.start[0].min(span.finish[0]) - search_config.window_margin_mm,
        span.start[1].min(span.finish[1]) - search_config.window_margin_mm,
        span.start[0].max(span.finish[0]) + search_config.window_margin_mm,
        span.start[1].max(span.finish[1]) + search_config.window_margin_mm,
    ];
    let minimum = [
        requested[0].max(board_bounds[0]),
        requested[1].max(board_bounds[1]),
    ];
    let maximum = [
        requested[2].min(board_bounds[2]),
        requested[3].min(board_bounds[3]),
    ];
    if minimum[0] >= maximum[0] || minimum[1] >= maximum[1] {
        return Err("local reroute window has no routing area".into());
    }
    // Align the local origin with the first endpoint. This avoids an
    // artificial connector at one side and makes imported 0.125 mm geometry
    // exact whenever the chosen resolution divides the endpoint delta.
    let origin = [
        span.start[0] - ((span.start[0] - minimum[0]) / resolution_mm).floor() * resolution_mm,
        span.start[1] - ((span.start[1] - minimum[1]) / resolution_mm).floor() * resolution_mm,
    ];
    let width = (((maximum[0] - origin[0]) / resolution_mm).floor() as usize) + 1;
    let height = (((maximum[1] - origin[1]) / resolution_mm).floor() as usize) + 1;
    let state_count = width
        .checked_mul(height)
        .ok_or_else(|| "local reroute grid dimensions overflow".to_string())?;
    if state_count == 0 || state_count > search_config.maximum_grid_states {
        return Err(format!(
            "local reroute needs {state_count} grid states, exceeding maximum_grid_states={}",
            search_config.maximum_grid_states
        )
        .into());
    }
    let safety = resolution_mm * 2.0_f64.sqrt() / 2.0;
    let obstacle_inflate = trace_radius + route_config.clearance_mm + safety;
    let mut blocked = vec![false; state_count];
    for y in 0..height {
        for x in 0..width {
            let point = [
                origin[0] + x as f64 * resolution_mm,
                origin[1] + y as f64 * resolution_mm,
            ];
            blocked[y * width + x] = model.obstacles.iter().any(|obstacle| {
                obstacle.blocks_tracks
                    && obstacle.layers[layer]
                    && obstacle.geometry.contains(
                        point,
                        obstacle.inflate(obstacle_inflate, route_config.clearance_mm),
                    )
            });
        }
    }
    let grid_config = KiCadGridRouteConfig {
        resolution_mm,
        ..route_config.clone()
    };
    let start = snapped_grid_position(span.start, origin, &grid_config, width, height)?;
    let finish = snapped_grid_position(span.finish, origin, &grid_config, width, height)?;
    if !route_segment_is_clear(span.start, span.start, layer, model, route_config)
        || !route_segment_is_clear(span.finish, span.finish, layer, model, route_config)
    {
        return Err("local reroute endpoint is not geometrically clear".into());
    }
    blocked[start.y * width + start.x] = false;
    blocked[finish.y * width + finish.x] = false;
    let mut request = GridRouteRequest {
        width,
        height,
        layers: 1,
        start,
        finish,
        max_expansions: search_config.maximum_expansions,
        alternate_starts: Vec::new(),
        alternate_finishes: Vec::new(),
        straight_cost: route_config.straight_cost,
        diagonal_cost: route_config.diagonal_cost,
        bend_cost: route_config.bend_cost,
        via_cost: route_config.via_cost_units(),
        allow_vias: false,
        blocked,
        blocked_planar_transitions: vec![false; state_count * 8],
        trace_enter_costs: vec![0; state_count],
        via_enter_costs: vec![u32::MAX; state_count],
    };
    let mut router = DutAStar;
    let mut searches = 0;
    let mut expansions = 0_u64;
    let mut exact_edge_retries = 0;
    let (grid_path, cost) = loop {
        searches += 1;
        match router.route(&request)? {
            GridRouteOutcome::Found {
                path,
                cost,
                expansions: work,
            } => {
                expansions += u64::from(work);
                let invalid = path.windows(2).find_map(|pair| {
                    let first = local_grid_point(pair[0], origin, resolution_mm);
                    let second = local_grid_point(pair[1], origin, resolution_mm);
                    (!route_segment_is_clear(first, second, layer, model, route_config))
                        .then_some((pair[0], pair[1]))
                });
                if let Some((first, second)) = invalid {
                    if exact_edge_retries >= search_config.maximum_exact_edge_retries {
                        return Err(format!(
                            "local reroute exhausted {} exact-edge retries after {expansions} expansions",
                            search_config.maximum_exact_edge_retries
                        )
                        .into());
                    }
                    request.block_planar_transition(first, second)?;
                    exact_edge_retries += 1;
                    continue;
                }
                let first = local_grid_point(path[0], origin, resolution_mm);
                let last = local_grid_point(
                    *path.last().expect("A* returned a non-empty path"),
                    origin,
                    resolution_mm,
                );
                if !route_segment_is_clear(span.start, first, layer, model, route_config)
                    || !route_segment_is_clear(last, span.finish, layer, model, route_config)
                {
                    return Err(format!(
                        "local reroute grid-to-endpoint connector is not exact-clear at {resolution_mm:.6} mm"
                    )
                    .into());
                }
                break (path, cost);
            }
            GridRouteOutcome::NoPath {
                expansions: work,
                blocked_frontier,
            } => {
                expansions += u64::from(work);
                let detail = format!(
                    "local reroute found no path after {searches} search(es) and {expansions} expansions"
                );
                return Err(KiCadViaLocalRerouteError::Search(Box::new(
                    local_reroute_failure_evidence(
                        KiCadViaLocalRerouteFailureKind::NoPath,
                        detail,
                        &request,
                        model,
                        layer,
                        origin,
                        resolution_mm,
                        obstacle_inflate,
                        route_config.clearance_mm,
                        branch_index,
                        span,
                        searches,
                        expansions,
                        exact_edge_retries,
                        blocked_frontier,
                    ),
                )));
            }
            GridRouteOutcome::BudgetExhausted {
                expansions: work,
                blocked_frontier,
            } => {
                expansions += u64::from(work);
                let detail = format!(
                    "local reroute exhausted its search budget after {searches} search(es) and {expansions} expansions"
                );
                return Err(KiCadViaLocalRerouteError::Search(Box::new(
                    local_reroute_failure_evidence(
                        KiCadViaLocalRerouteFailureKind::BudgetExhausted,
                        detail,
                        &request,
                        model,
                        layer,
                        origin,
                        resolution_mm,
                        obstacle_inflate,
                        route_config.clearance_mm,
                        branch_index,
                        span,
                        searches,
                        expansions,
                        exact_edge_retries,
                        blocked_frontier,
                    ),
                )));
            }
        }
    };
    let mut points = grid_path
        .iter()
        .copied()
        .map(|point| (local_grid_point(point, origin, resolution_mm), layer))
        .collect::<Vec<_>>();
    if points.first().is_none_or(|point| point.0 != span.start) {
        points.insert(0, (span.start, layer));
    }
    if points.last().is_none_or(|point| point.0 != span.finish) {
        points.push((span.finish, layer));
    }
    let points = simplify_route_points(points);
    let path_length_mm = points
        .windows(2)
        .map(|pair| distance_squared(pair[0].0, pair[1].0).sqrt())
        .sum();
    let path = points
        .into_iter()
        .map(|(at, _)| KiCadRoutePoint {
            at,
            layer: span.layer.clone(),
        })
        .collect::<Vec<_>>();
    Ok((
        path.clone(),
        KiCadViaLocalRerouteBranchEvidence {
            branch_index,
            start: span.start,
            finish: span.finish,
            layer: span.layer.clone(),
            window_bounds: [
                origin[0],
                origin[1],
                origin[0] + (width - 1) as f64 * resolution_mm,
                origin[1] + (height - 1) as f64 * resolution_mm,
            ],
            grid_size: [width, height],
            searches,
            expansions,
            exact_edge_retries,
            cost,
            path_points: path.len(),
            path_length_mm,
        },
    ))
}

fn local_reroute_blocker_ref(
    blocker: &KiCadViaLocalRerouteBlockerEvidence,
) -> KiCadViaLocalRerouteBlockerRef {
    KiCadViaLocalRerouteBlockerRef {
        kind: blocker.kind,
        object: blocker.object.clone(),
        component_movable: blocker.component_movable,
    }
}

fn local_reroute_model_without_blockers(
    model: &KiCadRoutingModel,
    suppressed: &[KiCadViaLocalRerouteBlockerRef],
) -> KiCadRoutingModel {
    let mut filtered = model.clone();
    filtered.obstacles.retain(|obstacle| {
        !suppressed
            .iter()
            .any(|blocker| blocker.kind == obstacle.kind && blocker.object == obstacle.object)
    });
    filtered
}

#[derive(Clone, Debug)]
struct PendingBlockerCutTrial {
    parent_trial: Option<usize>,
    added_blocker: KiCadViaLocalRerouteBlockerRef,
    suppressed: Vec<KiCadViaLocalRerouteBlockerRef>,
}

fn enqueue_blocker_cut_children(
    pending: &mut VecDeque<PendingBlockerCutTrial>,
    seen: &mut BTreeSet<Vec<KiCadViaLocalRerouteBlockerRef>>,
    parent_trial: Option<usize>,
    suppressed: &[KiCadViaLocalRerouteBlockerRef],
    failure: &KiCadViaLocalRerouteFailureEvidence,
    config: &KiCadViaBlockerCutSearchConfig,
) {
    if suppressed.len() >= config.maximum_cut_size {
        return;
    }
    for blocker in failure
        .blockers
        .iter()
        .take(config.maximum_ranked_blockers)
        .map(local_reroute_blocker_ref)
    {
        if suppressed.contains(&blocker) {
            continue;
        }
        let mut child = suppressed.to_vec();
        child.push(blocker.clone());
        child.sort();
        if seen.insert(child.clone()) {
            pending.push_back(PendingBlockerCutTrial {
                parent_trial,
                added_blocker: blocker,
                suppressed: child,
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn analyze_local_reroute_blocker_cuts(
    model: &KiCadRoutingModel,
    route_config: &KiCadGridRouteConfig,
    search_config: &KiCadViaLocalRerouteSearchConfig,
    cut_config: &KiCadViaBlockerCutSearchConfig,
    resolution_mm: f64,
    branch_index: usize,
    span: &KiCadViaRemovalSpan,
    initial_failure: &KiCadViaLocalRerouteFailureEvidence,
) -> KiCadViaBlockerCutAnalysisEvidence {
    let ranked_blockers = initial_failure
        .blockers
        .iter()
        .take(cut_config.maximum_ranked_blockers)
        .map(local_reroute_blocker_ref)
        .collect::<Vec<_>>();
    let mut plain_search_config = search_config.clone();
    plain_search_config.blocker_cut = None;
    let mut pending = VecDeque::new();
    let mut seen = BTreeSet::new();
    enqueue_blocker_cut_children(
        &mut pending,
        &mut seen,
        None,
        &[],
        initial_failure,
        cut_config,
    );
    let mut trials = Vec::new();
    let mut total_searches = 0;
    let mut total_expansions = 0;
    let mut total_exact_edge_retries = 0;
    let mut minimum_sufficient_cut_size = None;
    let mut sufficient_trials = Vec::new();

    while trials.len() < cut_config.maximum_trials {
        let Some(pending_trial) = pending.pop_front() else {
            break;
        };
        let index = trials.len();
        let filtered_model = local_reroute_model_without_blockers(model, &pending_trial.suppressed);
        let outcome = route_local_merged_run(
            &filtered_model,
            route_config,
            &plain_search_config,
            resolution_mm,
            branch_index,
            span,
        );
        let trial = match outcome {
            Ok((path, evidence)) => {
                total_searches += evidence.searches;
                total_expansions += evidence.expansions;
                total_exact_edge_retries += evidence.exact_edge_retries;
                if minimum_sufficient_cut_size
                    .is_none_or(|size| pending_trial.suppressed.len() < size)
                {
                    minimum_sufficient_cut_size = Some(pending_trial.suppressed.len());
                    sufficient_trials.clear();
                }
                if minimum_sufficient_cut_size == Some(pending_trial.suppressed.len()) {
                    sufficient_trials.push(index);
                }
                KiCadViaBlockerCutTrialEvidence {
                    index,
                    parent_trial: pending_trial.parent_trial,
                    added_blocker: pending_trial.added_blocker,
                    suppressed: pending_trial.suppressed,
                    status: KiCadViaBlockerCutTrialStatus::Found,
                    detail: "counterfactual local route found; suppressed obstacles were not applied to a board candidate".into(),
                    searches: evidence.searches,
                    expansions: evidence.expansions,
                    exact_edge_retries: evidence.exact_edge_retries,
                    path_cost: Some(evidence.cost),
                    path_length_mm: Some(evidence.path_length_mm),
                    path,
                    failure: None,
                }
            }
            Err(KiCadViaLocalRerouteError::Search(failure)) => {
                total_searches += failure.searches;
                total_expansions += failure.expansions;
                total_exact_edge_retries += failure.exact_edge_retries;
                enqueue_blocker_cut_children(
                    &mut pending,
                    &mut seen,
                    Some(index),
                    &pending_trial.suppressed,
                    &failure,
                    cut_config,
                );
                let status = match failure.kind {
                    KiCadViaLocalRerouteFailureKind::NoPath => {
                        KiCadViaBlockerCutTrialStatus::NoPath
                    }
                    KiCadViaLocalRerouteFailureKind::BudgetExhausted => {
                        KiCadViaBlockerCutTrialStatus::BudgetExhausted
                    }
                };
                KiCadViaBlockerCutTrialEvidence {
                    index,
                    parent_trial: pending_trial.parent_trial,
                    added_blocker: pending_trial.added_blocker,
                    suppressed: pending_trial.suppressed,
                    status,
                    detail: failure.detail.clone(),
                    searches: failure.searches,
                    expansions: failure.expansions,
                    exact_edge_retries: failure.exact_edge_retries,
                    path_cost: None,
                    path_length_mm: None,
                    path: Vec::new(),
                    failure: Some(failure),
                }
            }
            Err(KiCadViaLocalRerouteError::Plain(detail)) => KiCadViaBlockerCutTrialEvidence {
                index,
                parent_trial: pending_trial.parent_trial,
                added_blocker: pending_trial.added_blocker,
                suppressed: pending_trial.suppressed,
                status: KiCadViaBlockerCutTrialStatus::Unsupported,
                detail,
                searches: 0,
                expansions: 0,
                exact_edge_retries: 0,
                path_cost: None,
                path_length_mm: None,
                path: Vec::new(),
                failure: None,
            },
        };
        trials.push(trial);
    }
    let generated_cut_sets = seen.len();
    KiCadViaBlockerCutAnalysisEvidence {
        algorithm: "conflict-guided-semantic-blocker-cut-bfs-v1".into(),
        counterfactual_only: true,
        selectable: false,
        maximum_ranked_blockers: cut_config.maximum_ranked_blockers,
        maximum_cut_size: cut_config.maximum_cut_size,
        maximum_trials: cut_config.maximum_trials,
        ranked_blockers,
        generated_cut_sets,
        attempted_trials: trials.len(),
        trials_truncated: !pending.is_empty(),
        total_searches,
        total_expansions,
        total_exact_edge_retries,
        minimum_sufficient_cut_size,
        sufficient_trials,
        trials,
    }
}

fn collect_local_reroute_evidence(
    search_config: &KiCadViaLocalRerouteSearchConfig,
    resolution_mm: f64,
    branches: Vec<KiCadViaLocalRerouteBranchEvidence>,
    failed_branch: Option<KiCadViaLocalRerouteFailureEvidence>,
) -> KiCadViaLocalRerouteEvidence {
    let failed_searches = failed_branch.as_ref().map_or(0, |failure| failure.searches);
    let failed_expansions = failed_branch
        .as_ref()
        .map_or(0, |failure| failure.expansions);
    let failed_retries = failed_branch
        .as_ref()
        .map_or(0, |failure| failure.exact_edge_retries);
    KiCadViaLocalRerouteEvidence {
        algorithm: "testing-esp32-duts-bounded-single-layer-astar-semantic-frontier-v2".into(),
        resolution_mm,
        window_margin_mm: search_config.window_margin_mm,
        maximum_grid_states: search_config.maximum_grid_states,
        maximum_expansions_per_branch: search_config.maximum_expansions,
        maximum_exact_edge_retries_per_branch: search_config.maximum_exact_edge_retries,
        total_searches: branches.iter().map(|branch| branch.searches).sum::<usize>()
            + failed_searches,
        total_expansions: branches.iter().map(|branch| branch.expansions).sum::<u64>()
            + failed_expansions,
        total_exact_edge_retries: branches
            .iter()
            .map(|branch| branch.exact_edge_retries)
            .sum::<usize>()
            + failed_retries,
        branches,
        failed_branch,
        blocker_cut: None,
    }
}

fn apply_remove_and_local_reroute(
    source: &KiCadRouteCandidate,
    model: &KiCadRoutingModel,
    config: &KiCadViaActionSearchConfig,
    search_config: &KiCadViaLocalRerouteSearchConfig,
    merged_layer: &str,
    resolution_mm: f64,
) -> Result<
    (
        KiCadRouteCandidate,
        Vec<usize>,
        KiCadViaLocalRerouteEvidence,
    ),
    KiCadViaActionApplicationError,
> {
    let mut candidate = source.clone();
    let mut edits = Vec::new();
    for (branch_index, branch) in candidate.branches.iter_mut().enumerate() {
        if let Some(span) = remove_physical_via_from_branch_with_span(
            branch,
            config.target_at,
            &config.target_layers,
            merged_layer,
        )? {
            edits.push((branch_index, span));
        }
    }
    if edits.is_empty() {
        return Err("target physical via does not occur in the source candidate".into());
    }
    if edits.len() > config.maximum_affected_branches {
        return Err(format!(
            "target via affects {} branches, exceeding maximum_affected_branches={}",
            edits.len(),
            config.maximum_affected_branches
        )
        .into());
    }
    let affected = edits
        .iter()
        .map(|(branch_index, _)| *branch_index)
        .collect::<Vec<_>>();
    let (segments, vias) = materialize_candidate_branches(
        &candidate.connection,
        &candidate.branches,
        &candidate.config,
    )?;
    candidate.supplemental_segments = segments;
    candidate.supplemental_vias = vias;
    pad_layer_contacts::normalize_endpoint_contacts(&mut candidate, model)?;
    let diagnostic_candidate = candidate.clone();
    let mut branch_evidence = Vec::with_capacity(edits.len());
    for (branch_index, span) in &edits {
        let routed = route_local_merged_run(
            model,
            &candidate.config,
            search_config,
            resolution_mm,
            *branch_index,
            span,
        );
        let (replacement, evidence) = match routed {
            Ok(result) => result,
            Err(KiCadViaLocalRerouteError::Plain(detail)) => {
                return Err(KiCadViaActionApplicationError {
                    detail,
                    affected_branches: affected,
                    local_reroute: None,
                    diagnostic_candidate: Some(Box::new(diagnostic_candidate)),
                });
            }
            Err(KiCadViaLocalRerouteError::Search(failure)) => {
                let detail = failure.detail.clone();
                let blocker_cut = search_config.blocker_cut.as_ref().map(|cut_config| {
                    analyze_local_reroute_blocker_cuts(
                        model,
                        &candidate.config,
                        search_config,
                        cut_config,
                        resolution_mm,
                        *branch_index,
                        span,
                        &failure,
                    )
                });
                let mut evidence = collect_local_reroute_evidence(
                    search_config,
                    resolution_mm,
                    branch_evidence,
                    Some(*failure),
                );
                evidence.blocker_cut = blocker_cut;
                return Err(KiCadViaActionApplicationError {
                    detail,
                    affected_branches: affected,
                    local_reroute: Some(Box::new(evidence)),
                    diagnostic_candidate: Some(Box::new(diagnostic_candidate)),
                });
            }
        };
        replace_merged_branch_run(&mut candidate.branches[*branch_index], span, &replacement)?;
        candidate.branches[*branch_index].cost = evidence.cost;
        candidate.branches[*branch_index].expansions =
            u32::try_from(evidence.expansions).unwrap_or(u32::MAX);
        branch_evidence.push(evidence);
    }
    let (segments, vias) = materialize_candidate_branches(
        &candidate.connection,
        &candidate.branches,
        &candidate.config,
    )?;
    candidate.supplemental_segments = segments;
    candidate.supplemental_vias = vias;
    candidate.cost = branch_evidence
        .iter()
        .fold(0_u32, |total, branch| total.saturating_add(branch.cost));
    candidate.expansions = branch_evidence.iter().fold(0_u32, |total, branch| {
        total.saturating_add(u32::try_from(branch.expansions).unwrap_or(u32::MAX))
    });
    candidate.router = "testing-esp32-duts-local-via-removal-reroute-v1".into();
    let evidence =
        collect_local_reroute_evidence(search_config, resolution_mm, branch_evidence, None);
    Ok((candidate, affected, evidence))
}

fn apply_via_topology_action(
    source: &KiCadRouteCandidate,
    config: &KiCadViaActionSearchConfig,
    action: &KiCadViaTopologyAction,
) -> Result<(KiCadRouteCandidate, Vec<usize>), String> {
    let mut candidate = source.clone();
    let mut affected = Vec::new();
    for (branch_index, branch) in candidate.branches.iter_mut().enumerate() {
        let changed = match action {
            KiCadViaTopologyAction::Remove { merged_layer } => remove_physical_via_from_branch(
                branch,
                config.target_at,
                &config.target_layers,
                merged_layer,
            )?,
            KiCadViaTopologyAction::RemoveAndReroute { .. } => {
                return Err("remove-and-reroute requires a routing model".into());
            }
            KiCadViaTopologyAction::Relocate { to } => relocate_physical_via_in_branch(
                branch,
                config.target_at,
                &config.target_layers,
                *to,
            )?,
        };
        if changed {
            affected.push(branch_index);
        }
    }
    if affected.is_empty() {
        return Err("target physical via does not occur in the source candidate".into());
    }
    if affected.len() > config.maximum_affected_branches {
        return Err(format!(
            "target via affects {} branches, exceeding maximum_affected_branches={}",
            affected.len(),
            config.maximum_affected_branches
        ));
    }
    let (segments, vias) = materialize_candidate_branches(
        &candidate.connection,
        &candidate.branches,
        &candidate.config,
    )?;
    candidate.supplemental_segments = segments;
    candidate.supplemental_vias = vias;
    candidate.router = "layout-trace-via-topology-action-v1".into();
    Ok((candidate, affected))
}

fn apply_via_search_action(
    source: &KiCadRouteCandidate,
    model: &KiCadRoutingModel,
    config: &KiCadViaActionSearchConfig,
    action: &KiCadViaTopologyAction,
) -> Result<
    (
        KiCadRouteCandidate,
        Vec<usize>,
        Option<KiCadViaLocalRerouteEvidence>,
    ),
    KiCadViaActionApplicationError,
> {
    match action {
        KiCadViaTopologyAction::RemoveAndReroute {
            merged_layer,
            resolution_mm,
            window_margin_mm,
        } => {
            let search_config = config.local_reroute.as_ref().ok_or_else(|| {
                KiCadViaActionApplicationError::from(
                    "remove-and-reroute action has no local_reroute configuration",
                )
            })?;
            if (*window_margin_mm - search_config.window_margin_mm).abs() > 1.0e-12
                || !search_config
                    .merged_layers
                    .iter()
                    .any(|layer| layer == merged_layer)
                || !search_config
                    .resolutions_mm
                    .iter()
                    .any(|resolution| (*resolution - *resolution_mm).abs() <= 1.0e-12)
            {
                return Err("remove-and-reroute action is outside its configured frontier".into());
            }
            let (mut candidate, affected, evidence) = apply_remove_and_local_reroute(
                source,
                model,
                config,
                search_config,
                merged_layer,
                *resolution_mm,
            )?;
            pad_layer_contacts::normalize_endpoint_contacts(&mut candidate, model)?;
            Ok((candidate, affected, Some(evidence)))
        }
        _ => {
            let (mut candidate, affected) = apply_via_topology_action(source, config, action)
                .map_err(KiCadViaActionApplicationError::from)?;
            pad_layer_contacts::normalize_endpoint_contacts(&mut candidate, model)?;
            Ok((candidate, affected, None))
        }
    }
}

fn local_reroute_via_action_candidates(
    config: &KiCadViaActionSearchConfig,
) -> Vec<(String, KiCadViaTopologyAction)> {
    let Some(local) = &config.local_reroute else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    for layer in &local.merged_layers {
        for &resolution_mm in &local.resolutions_mm {
            actions.push((
                format!(
                    "remove-reroute-{}-r{resolution_mm:.6}",
                    layer.replace('.', "-")
                ),
                KiCadViaTopologyAction::RemoveAndReroute {
                    merged_layer: layer.clone(),
                    resolution_mm,
                    window_margin_mm: local.window_margin_mm,
                },
            ));
        }
    }
    actions
}

fn lattice_via_action_candidates(
    config: &KiCadViaActionSearchConfig,
) -> Vec<(String, KiCadViaTopologyAction)> {
    let mut actions = config
        .removal_layers
        .iter()
        .map(|layer| {
            (
                format!("remove-{}", layer.replace('.', "-")),
                KiCadViaTopologyAction::Remove {
                    merged_layer: layer.clone(),
                },
            )
        })
        .collect::<Vec<_>>();
    if config.maximum_relocation_candidates == 0 {
        return actions;
    }
    let lattice_radius = (config.relocation_radius_mm / config.relocation_step_mm).floor() as i32;
    let mut offsets = Vec::new();
    for y in -lattice_radius..=lattice_radius {
        for x in -lattice_radius..=lattice_radius {
            if x == 0 && y == 0 {
                continue;
            }
            let offset = [
                x as f64 * config.relocation_step_mm,
                y as f64 * config.relocation_step_mm,
            ];
            let distance = offset[0].hypot(offset[1]);
            if distance <= config.relocation_radius_mm + 1.0e-9 {
                offsets.push((distance, offset));
            }
        }
    }
    offsets.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1[0].total_cmp(&right.1[0]))
            .then_with(|| left.1[1].total_cmp(&right.1[1]))
    });
    for (index, (_, offset)) in offsets
        .into_iter()
        .take(config.maximum_relocation_candidates)
        .enumerate()
    {
        let to = [
            config.target_at[0] + offset[0],
            config.target_at[1] + offset[1],
        ];
        actions.push((
            format!("relocate-{index:03}"),
            KiCadViaTopologyAction::Relocate { to },
        ));
    }
    actions
}

#[derive(Clone, Debug)]
struct FeasibleRelocationPoint {
    to: [f64; 2],
    score_mm: f64,
}

fn via_relocation_descent_direction(
    source: &KiCadRouteCandidate,
    config: &KiCadViaActionSearchConfig,
) -> Result<Option<[f64; 2]>, String> {
    let mut physical_edges = BTreeMap::<String, [f64; 2]>::new();
    for branch in &source.branches {
        for (transition, pair) in branch.path.windows(2).enumerate() {
            if !same_native_position(pair[0].at, config.target_at)
                || !same_native_position(pair[1].at, config.target_at)
                || pair[0].layer == pair[1].layer
                || !target_layer_pair_matches(&pair[0].layer, &pair[1].layer, &config.target_layers)
            {
                continue;
            }
            if transition == 0 || transition + 2 >= branch.path.len() {
                return Err("target via is not an internal free route point".into());
            }
            for (anchor, layer) in [
                (branch.path[transition - 1].at, &pair[0].layer),
                (branch.path[transition + 2].at, &pair[1].layer),
            ] {
                physical_edges.insert(
                    format!(
                        "{layer}:{:.9}:{:.9}:{:.9}:{:.9}",
                        anchor[0], anchor[1], config.target_at[0], config.target_at[1]
                    ),
                    anchor,
                );
            }
        }
    }
    if physical_edges.is_empty() {
        return Err("target physical via does not occur in the source candidate".into());
    }
    let mut gradient = [0.0, 0.0];
    for anchor in physical_edges.values() {
        let offset = [
            config.target_at[0] - anchor[0],
            config.target_at[1] - anchor[1],
        ];
        let distance = offset[0].hypot(offset[1]);
        if distance <= 1.0e-12 {
            return Err("target via has a zero-length incident copper edge".into());
        }
        gradient[0] += offset[0] / distance;
        gradient[1] += offset[1] / distance;
    }
    let magnitude = gradient[0].hypot(gradient[1]);
    Ok((magnitude > 1.0e-12).then(|| [-gradient[0] / magnitude, -gradient[1] / magnitude]))
}

fn assess_via_relocation(
    source: &KiCadRouteCandidate,
    model: &KiCadRoutingModel,
    config: &KiCadViaActionSearchConfig,
    to: [f64; 2],
) -> Result<(bool, f64), String> {
    let action = KiCadViaTopologyAction::Relocate { to };
    let (candidate, affected_branches) = apply_via_topology_action(source, config, &action)?;
    let mut blockers = selected_geometry_blockers(&candidate, &affected_branches, model);
    blockers.extend(candidate_via_blockers(&candidate, model));
    let quality = route_candidate_quality(&candidate)?;
    Ok((
        blockers.is_empty(),
        quality.length_mm + config.via_penalty_mm * quality.vias as f64,
    ))
}

fn feasible_frontier_via_action_candidates(
    source: &KiCadRouteCandidate,
    model: &KiCadRoutingModel,
    config: &KiCadViaActionSearchConfig,
    frontier: &KiCadViaFeasibleFrontierConfig,
    source_score_mm: f64,
) -> Result<
    (
        Vec<(String, KiCadViaTopologyAction)>,
        KiCadViaRelocationGenerationEvidence,
    ),
    String,
> {
    let Some(descent_direction) = via_relocation_descent_direction(source, config)? else {
        return Ok((
            Vec::new(),
            KiCadViaRelocationGenerationEvidence {
                method: "analytic-feasible-radial-frontier-v1".into(),
                descent_direction: None,
                angular_directions: frontier.angular_samples,
                radial_samples_per_direction: frontier.radial_samples,
                boundary_refinement_steps: frontier.boundary_refinement_steps,
                minimum_candidate_spacing_mm: frontier.minimum_candidate_spacing_mm,
                analytic_probes: 0,
                analytically_clear_probes: 0,
                feasibility_transitions: 0,
                boundary_refinement_probes: 0,
                improving_feasible_points: 0,
                suppressed_near_duplicates: 0,
                retained_relocations: 0,
            },
        ));
    };
    let base_angle = descent_direction[1].atan2(descent_direction[0]);
    let radial_step = config.relocation_radius_mm / frontier.radial_samples as f64;
    let mut analytic_probes = 0;
    let mut analytically_clear_probes = 0;
    let mut feasibility_transitions = 0;
    let mut boundary_refinement_probes = 0;
    let mut points = BTreeMap::<String, FeasibleRelocationPoint>::new();

    let mut retain = |to: [f64; 2], clear: bool, score_mm: f64| {
        if !clear
            || score_mm + config.minimum_score_improvement_mm > source_score_mm
            || to == config.target_at
        {
            return;
        }
        let key = format!("{:.9}:{:.9}", to[0], to[1]);
        points
            .entry(key)
            .and_modify(|point| point.score_mm = point.score_mm.min(score_mm))
            .or_insert(FeasibleRelocationPoint { to, score_mm });
    };

    for direction_index in 0..frontier.angular_samples {
        let angle = base_angle
            + std::f64::consts::TAU * direction_index as f64 / frontier.angular_samples as f64;
        let direction = [angle.cos(), angle.sin()];
        let mut prior_t = 0.0;
        let mut prior_clear = true;
        for radial_index in 1..=frontier.radial_samples {
            let t = radial_step * radial_index as f64;
            let to = [
                config.target_at[0] + direction[0] * t,
                config.target_at[1] + direction[1] * t,
            ];
            let (clear, score_mm) = assess_via_relocation(source, model, config, to)?;
            analytic_probes += 1;
            analytically_clear_probes += usize::from(clear);
            retain(to, clear, score_mm);

            if clear != prior_clear {
                feasibility_transitions += 1;
                let mut low_t = prior_t;
                let mut low_clear = prior_clear;
                let mut high_t = t;
                for _ in 0..frontier.boundary_refinement_steps {
                    let mid_t = (low_t + high_t) / 2.0;
                    let mid = [
                        config.target_at[0] + direction[0] * mid_t,
                        config.target_at[1] + direction[1] * mid_t,
                    ];
                    let (mid_clear, mid_score) = assess_via_relocation(source, model, config, mid)?;
                    analytic_probes += 1;
                    boundary_refinement_probes += 1;
                    analytically_clear_probes += usize::from(mid_clear);
                    retain(mid, mid_clear, mid_score);
                    if mid_clear == low_clear {
                        low_t = mid_t;
                        low_clear = mid_clear;
                    } else {
                        high_t = mid_t;
                    }
                }
                let feasible_t = if low_clear {
                    (low_t - frontier.boundary_inset_mm).max(0.0)
                } else {
                    (high_t + frontier.boundary_inset_mm).min(config.relocation_radius_mm)
                };
                if feasible_t > 0.0 {
                    let feasible = [
                        config.target_at[0] + direction[0] * feasible_t,
                        config.target_at[1] + direction[1] * feasible_t,
                    ];
                    let (feasible_clear, feasible_score) =
                        assess_via_relocation(source, model, config, feasible)?;
                    analytic_probes += 1;
                    analytically_clear_probes += usize::from(feasible_clear);
                    retain(feasible, feasible_clear, feasible_score);
                }
            }
            prior_t = t;
            prior_clear = clear;
        }
    }

    let improving_feasible_points = points.len();
    let mut points = points.into_values().collect::<Vec<_>>();
    points.sort_by(|left, right| {
        left.score_mm
            .total_cmp(&right.score_mm)
            .then_with(|| left.to[0].total_cmp(&right.to[0]))
            .then_with(|| left.to[1].total_cmp(&right.to[1]))
    });
    let mut diverse_points = Vec::<FeasibleRelocationPoint>::new();
    let mut suppressed_near_duplicates = 0;
    for point in points {
        if diverse_points.iter().any(|retained| {
            distance_squared(point.to, retained.to) < frontier.minimum_candidate_spacing_mm.powi(2)
        }) {
            suppressed_near_duplicates += 1;
        } else {
            diverse_points.push(point);
        }
    }
    diverse_points.truncate(config.maximum_relocation_candidates);
    let retained_relocations = diverse_points.len();
    let actions = diverse_points
        .into_iter()
        .enumerate()
        .map(|(index, point)| {
            (
                format!("frontier-relocate-{index:03}"),
                KiCadViaTopologyAction::Relocate { to: point.to },
            )
        })
        .collect();
    Ok((
        actions,
        KiCadViaRelocationGenerationEvidence {
            method: "analytic-feasible-radial-frontier-v1".into(),
            descent_direction: Some(descent_direction),
            angular_directions: frontier.angular_samples,
            radial_samples_per_direction: frontier.radial_samples,
            boundary_refinement_steps: frontier.boundary_refinement_steps,
            minimum_candidate_spacing_mm: frontier.minimum_candidate_spacing_mm,
            analytic_probes,
            analytically_clear_probes,
            feasibility_transitions,
            boundary_refinement_probes,
            improving_feasible_points,
            suppressed_near_duplicates,
            retained_relocations,
        },
    ))
}

fn via_action_candidates(
    source: &KiCadRouteCandidate,
    model: &KiCadRoutingModel,
    config: &KiCadViaActionSearchConfig,
    source_score_mm: f64,
) -> Result<
    (
        Vec<(String, KiCadViaTopologyAction)>,
        KiCadViaRelocationGenerationEvidence,
    ),
    String,
> {
    if let Some(frontier) = &config.feasible_frontier {
        let (relocations, evidence) = feasible_frontier_via_action_candidates(
            source,
            model,
            config,
            frontier,
            source_score_mm,
        )?;
        let mut actions = config
            .removal_layers
            .iter()
            .map(|layer| {
                (
                    format!("remove-{}", layer.replace('.', "-")),
                    KiCadViaTopologyAction::Remove {
                        merged_layer: layer.clone(),
                    },
                )
            })
            .collect::<Vec<_>>();
        actions.extend(local_reroute_via_action_candidates(config));
        actions.extend(relocations);
        Ok((actions, evidence))
    } else {
        let mut actions = lattice_via_action_candidates(config);
        let retained_relocations = actions.len().saturating_sub(config.removal_layers.len());
        actions.extend(local_reroute_via_action_candidates(config));
        Ok((
            actions,
            KiCadViaRelocationGenerationEvidence {
                method: "deterministic-lattice-v1".into(),
                descent_direction: None,
                angular_directions: 0,
                radial_samples_per_direction: 0,
                boundary_refinement_steps: 0,
                minimum_candidate_spacing_mm: 0.0,
                analytic_probes: 0,
                analytically_clear_probes: 0,
                feasibility_transitions: 0,
                boundary_refinement_probes: 0,
                improving_feasible_points: retained_relocations,
                suppressed_near_duplicates: 0,
                retained_relocations,
            },
        ))
    }
}

fn validate_via_action_config(config: &KiCadViaActionSearchConfig) -> Result<(), String> {
    if !config.target_at[0].is_finite()
        || !config.target_at[1].is_finite()
        || config.target_layers[0] == config.target_layers[1]
        || config
            .target_layers
            .iter()
            .any(|layer| copper_layer_index(layer).is_none())
        || config.maximum_actions == 0
        || config.maximum_actions > 1_000
        || config.maximum_affected_branches == 0
        || config.maximum_affected_branches > 1_000
        || !config.via_penalty_mm.is_finite()
        || config.via_penalty_mm < 0.0
        || !config.minimum_score_improvement_mm.is_finite()
        || config.minimum_score_improvement_mm < 0.0
        || !config.relocation_step_mm.is_finite()
        || config.relocation_step_mm < 0.0
    {
        return Err("invalid KiCad via-action configuration".into());
    }
    let removal_layers = config.removal_layers.iter().collect::<BTreeSet<_>>();
    if removal_layers.len() != config.removal_layers.len()
        || config
            .removal_layers
            .iter()
            .any(|layer| layer != &config.target_layers[0] && layer != &config.target_layers[1])
    {
        return Err("via-action removal_layers must be unique incident target layers".into());
    }
    if config.maximum_relocation_candidates > 0
        && (!config.relocation_radius_mm.is_finite() || config.relocation_radius_mm <= 0.0)
    {
        return Err("via-action relocation search must have a positive radius".into());
    }
    if config.maximum_relocation_candidates > 0
        && config.feasible_frontier.is_none()
        && (!config.relocation_step_mm.is_finite()
            || config.relocation_step_mm <= 0.0
            || config.relocation_step_mm > config.relocation_radius_mm)
    {
        return Err("via-action relocation lattice must have a positive bounded step".into());
    }
    if let Some(frontier) = &config.feasible_frontier
        && (!(1..=360).contains(&frontier.angular_samples)
            || !(1..=1_000).contains(&frontier.radial_samples)
            || frontier.boundary_refinement_steps > 64
            || !frontier.boundary_inset_mm.is_finite()
            || frontier.boundary_inset_mm <= 0.0
            || frontier.boundary_inset_mm
                >= config.relocation_radius_mm / frontier.radial_samples as f64
            || !frontier.minimum_candidate_spacing_mm.is_finite()
            || frontier.minimum_candidate_spacing_mm <= 0.0
            || frontier.minimum_candidate_spacing_mm > config.relocation_radius_mm)
    {
        return Err("invalid analytic-feasible via relocation frontier configuration".into());
    }
    let local_actions = if let Some(local) = &config.local_reroute {
        let layers = local.merged_layers.iter().collect::<BTreeSet<_>>();
        let resolutions = local
            .resolutions_mm
            .iter()
            .map(|resolution| format!("{resolution:.12}"))
            .collect::<BTreeSet<_>>();
        if local.merged_layers.is_empty()
            || local.resolutions_mm.is_empty()
            || layers.len() != local.merged_layers.len()
            || resolutions.len() != local.resolutions_mm.len()
            || local
                .merged_layers
                .iter()
                .any(|layer| layer != &config.target_layers[0] && layer != &config.target_layers[1])
            || local
                .resolutions_mm
                .iter()
                .any(|resolution| !resolution.is_finite() || *resolution <= 0.0)
            || !local.window_margin_mm.is_finite()
            || local.window_margin_mm <= 0.0
            || local.maximum_grid_states == 0
            || local.maximum_grid_states > 100_000_000
            || local.maximum_expansions == 0
            || local.maximum_exact_edge_retries > 100_000
        {
            return Err("invalid bounded via-removal local-reroute configuration".into());
        }
        if let Some(cut) = &local.blocker_cut
            && (cut.maximum_ranked_blockers == 0
                || cut.maximum_ranked_blockers > 64
                || cut.maximum_cut_size == 0
                || cut.maximum_cut_size > 8
                || cut.maximum_trials == 0
                || cut.maximum_trials > 10_000)
        {
            return Err("invalid bounded semantic blocker-cut configuration".into());
        }
        local
            .merged_layers
            .len()
            .checked_mul(local.resolutions_mm.len())
            .ok_or_else(|| "via local-reroute action count overflow".to_string())?
    } else {
        0
    };
    let configured_actions = config
        .removal_layers
        .len()
        .checked_add(config.maximum_relocation_candidates)
        .and_then(|count| count.checked_add(local_actions))
        .ok_or_else(|| "via-action configured proposal count overflow".to_string())?;
    if configured_actions > config.maximum_actions {
        return Err("via-action configured proposals exceed maximum_actions".into());
    }
    Ok(())
}

fn candidate_digest(candidate: &KiCadRouteCandidate) -> Result<String, String> {
    let bytes = serde_json::to_vec(candidate).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn candidate_via_blockers(
    candidate: &KiCadRouteCandidate,
    model: &KiCadRoutingModel,
) -> Vec<String> {
    let mut blockers = Vec::new();
    let radius = candidate.config.via_size_mm / 2.0;
    let obstacle_inflate = radius + candidate.config.clearance_mm;
    let hole_inflate =
        candidate.config.via_drill_mm / 2.0 + candidate.config.hole_to_hole_clearance_mm;
    for (via_index, via) in candidate.supplemental_vias.iter().enumerate() {
        let outside = !model
            .outline
            .contains_point_with_clearance(via.at, radius + candidate.config.edge_clearance_mm);
        if outside {
            blockers.push(format!("via {via_index} violates board-edge clearance"));
        }
        for (obstacle_index, obstacle) in model.obstacles.iter().enumerate() {
            if obstacle.blocks_vias
                && (obstacle.layers[0] || obstacle.layers[1])
                && obstacle.geometry.contains(
                    via.at,
                    obstacle.inflate(obstacle_inflate, candidate.config.clearance_mm),
                )
            {
                blockers.push(format!(
                    "via {via_index} at {:?} intersects obstacle {obstacle_index}",
                    via.at
                ));
            }
        }
        for (hole_index, hole) in model.target_holes.iter().enumerate() {
            if hole.contains(via.at, hole_inflate) {
                blockers.push(format!(
                    "via {via_index} at {:?} violates target-hole spacing against hole {hole_index}",
                    via.at
                ));
            }
        }
    }
    blockers
}

/// Ports layout-trace's discrete remove/relocate-via lifecycle into immutable
/// KiCad route candidates, then native-gates every unique complete-board
/// attempt. The physical coordinate/layer pair is the stable handle within the
/// source candidate; all root-to-terminal branch occurrences are edited
/// atomically. The source remains selectable and no proposal commits without
/// the full KiCad completion policy.
pub fn search_via_topology_actions(
    source_directory: &Path,
    board_id: &str,
    candidate_path: &Path,
    output_directory: &Path,
    config: &KiCadViaActionSearchConfig,
) -> Result<(Option<KiCadRouteCandidate>, KiCadViaActionPortfolioEvidence), String> {
    validate_via_action_config(config)?;
    if board_id.is_empty() || !source_directory.is_dir() || output_directory.exists() {
        return Err("invalid KiCad via-action source, board ID, or existing output".into());
    }
    let source_directory = fs::canonicalize(source_directory).map_err(|error| error.to_string())?;
    let output_parent = output_directory
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let output_parent = fs::canonicalize(output_parent).map_err(|error| error.to_string())?;
    let output_name = output_directory
        .file_name()
        .ok_or_else(|| "KiCad via-action output has no directory name".to_string())?;
    let output_directory = output_parent.join(output_name);
    if output_directory.starts_with(&source_directory) {
        return Err("KiCad via-action output must not be inside its source directory".into());
    }
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    if !source_board.is_file() {
        return Err(format!(
            "source board {} does not exist",
            source_board.display()
        ));
    }
    let source_candidate = read_route_candidate(candidate_path)?;
    let source_quality = route_candidate_quality(&source_candidate)?;
    let source_score_mm =
        source_quality.length_mm + config.via_penalty_mm * source_quality.vias as f64;
    let pcb_source = fs::read_to_string(&source_board).map_err(|error| error.to_string())?;
    let mut pcb = parse(&pcb_source)?;
    apply_footprint_placements(&mut pcb, &source_candidate.footprint_placements)?;
    apply_reference_placements(&mut pcb, &source_candidate.reference_placements)?;
    let model = KiCadRoutingModel::from_pcb_with_config(
        &pcb,
        &source_candidate.connection,
        &source_candidate.config,
    )?;

    fs::create_dir(&output_directory).map_err(|error| error.to_string())?;
    let source_artifact_name = "source-candidate";
    let source_artifact_directory = output_directory.join(source_artifact_name);
    copy_directory_tree(&source_directory, &source_artifact_directory)?;
    let source_artifact_candidate = source_artifact_directory.join("candidate.json");
    write_pretty_json(&source_artifact_candidate, &source_candidate)?;
    let source_artifact_board = source_artifact_directory.join(format!("{board_id}.kicad_pcb"));
    apply_route_candidate(
        &source_artifact_board,
        &source_artifact_candidate,
        &source_artifact_board,
    )?;
    let source_verification = verify_materialized_rung(&source_artifact_directory, board_id)?;

    let (action_specs, relocation_generation) =
        via_action_candidates(&source_candidate, &model, config, source_score_mm)?;
    let generated_actions = action_specs.len();
    let mut attempts = Vec::new();
    let mut unique_digests = BTreeMap::<String, usize>::new();
    let mut affected_union = BTreeSet::new();
    for (id, action) in action_specs {
        let attempt_index = attempts.len();
        let (candidate, affected_branches, local_reroute) =
            match apply_via_search_action(&source_candidate, &model, config, &action) {
                Ok(result) => result,
                Err(error) => {
                    let KiCadViaActionApplicationError {
                        detail,
                        affected_branches,
                        local_reroute,
                        diagnostic_candidate,
                    } = error;
                    let local_reroute = local_reroute.map(|evidence| *evidence);
                    let diagnostic_candidate = diagnostic_candidate.map(|candidate| *candidate);
                    affected_union.extend(affected_branches.iter().copied());
                    let (artifact_directory, diagnostic) =
                        if let (Some(evidence), Some(candidate)) =
                            (local_reroute.as_ref(), diagnostic_candidate.as_ref())
                        {
                            if evidence.failed_branch.is_some() {
                                let artifact_name = format!("attempt-{attempt_index:03}-{id}");
                                let artifact_path = output_directory.join(&artifact_name);
                                let diagnostic = write_via_local_reroute_diagnostic(
                                    &artifact_path,
                                    &model,
                                    candidate,
                                    evidence,
                                )?;
                                (Some(PathBuf::from(artifact_name)), Some(diagnostic))
                            } else {
                                (None, None)
                            }
                        } else {
                            (None, None)
                        };
                    attempts.push(KiCadViaActionAttempt {
                        id,
                        action,
                        status: KiCadViaActionStatus::Unsupported,
                        detail,
                        affected_branches,
                        artifact_directory,
                        candidate: None,
                        duplicate_of: None,
                        analytic_geometry_clear: false,
                        analytic_blockers: Vec::new(),
                        local_reroute,
                        diagnostic,
                        verification: None,
                        route_quality: None,
                        score_mm: None,
                        selected: false,
                    });
                    continue;
                }
            };
        affected_union.extend(affected_branches.iter().copied());
        let digest = candidate_digest(&candidate)?;
        if let Some(&duplicate_of) = unique_digests.get(&digest) {
            attempts.push(KiCadViaActionAttempt {
                id,
                action,
                status: KiCadViaActionStatus::Duplicate,
                detail: format!("candidate is byte-equivalent to attempt {duplicate_of}"),
                affected_branches,
                artifact_directory: None,
                candidate: None,
                duplicate_of: Some(duplicate_of),
                analytic_geometry_clear: false,
                analytic_blockers: Vec::new(),
                local_reroute,
                diagnostic: None,
                verification: None,
                route_quality: None,
                score_mm: None,
                selected: false,
            });
            continue;
        }
        unique_digests.insert(digest, attempt_index);
        let mut analytic_blockers =
            selected_geometry_blockers(&candidate, &affected_branches, &model);
        analytic_blockers.extend(candidate_via_blockers(&candidate, &model));
        let analytic_geometry_clear = analytic_blockers.is_empty();
        let route_quality = route_candidate_quality(&candidate)?;
        let score_mm = route_quality.length_mm + config.via_penalty_mm * route_quality.vias as f64;
        let artifact_name = format!("attempt-{attempt_index:03}-{id}");
        let artifact_directory = output_directory.join(&artifact_name);
        copy_directory_tree(&source_directory, &artifact_directory)?;
        let attempt_candidate_path = artifact_directory.join("candidate.json");
        write_pretty_json(&attempt_candidate_path, &candidate)?;
        let attempt_board = artifact_directory.join(format!("{board_id}.kicad_pcb"));
        apply_route_candidate(&attempt_board, &attempt_candidate_path, &attempt_board)?;
        let verification = verify_materialized_rung(&artifact_directory, board_id)?;
        attempts.push(KiCadViaActionAttempt {
            id,
            action,
            status: KiCadViaActionStatus::Proposed,
            detail: if verification.complete {
                "candidate passed the native KiCad completion policy".into()
            } else {
                "candidate failed the native KiCad completion policy".into()
            },
            affected_branches,
            artifact_directory: Some(PathBuf::from(artifact_name)),
            candidate: Some(candidate),
            duplicate_of: None,
            analytic_geometry_clear,
            analytic_blockers,
            local_reroute,
            diagnostic: None,
            verification: Some(verification),
            route_quality: Some(route_quality),
            score_mm: Some(score_mm),
            selected: false,
        });
    }
    let unique_actions = attempts
        .iter()
        .filter(|attempt| attempt.status == KiCadViaActionStatus::Proposed)
        .count();
    let exact_complete_actions = attempts
        .iter()
        .filter(|attempt| {
            attempt
                .verification
                .as_ref()
                .is_some_and(|verification| verification.complete)
        })
        .count();
    let selected_attempt = attempts
        .iter()
        .enumerate()
        .filter(|(_, attempt)| {
            attempt
                .verification
                .as_ref()
                .is_some_and(|verification| verification.complete)
                && attempt.score_mm.is_some_and(|score| {
                    score + config.minimum_score_improvement_mm <= source_score_mm
                })
        })
        .min_by(|(_, left), (_, right)| {
            left.score_mm
                .expect("selected attempt has a score")
                .total_cmp(&right.score_mm.expect("selected attempt has a score"))
                .then_with(|| left.id.cmp(&right.id))
        })
        .map(|(index, _)| index);
    if let Some(index) = selected_attempt {
        attempts[index].selected = true;
    }
    let selected_source = selected_attempt.is_none() && source_verification.complete;
    let selected_candidate = selected_attempt
        .and_then(|index| attempts[index].candidate.clone())
        .or_else(|| selected_source.then(|| source_candidate.clone()));
    let complete = selected_candidate.is_some();
    Ok((
        selected_candidate,
        KiCadViaActionPortfolioEvidence {
            schema_version: 6,
            algorithm: "native-kicad-layout-trace-via-action-portfolio-v6".into(),
            board_id: board_id.into(),
            connection: source_candidate.connection,
            source_artifact_directory: PathBuf::from(source_artifact_name),
            source_verification,
            source_quality,
            source_score_mm,
            target_at: config.target_at,
            target_layers: config.target_layers.clone(),
            affected_branches: affected_union.into_iter().collect(),
            relocation_generation,
            generated_actions,
            unique_actions,
            attempted_actions: attempts.len(),
            exact_complete_actions,
            selected_source,
            selected_attempt,
            complete,
            selection_rule: format!(
                "native completeness, canonical physical length + {:.6} mm per physical via, action ID",
                config.via_penalty_mm
            ),
            attempts,
        },
    ))
}

fn copy_directory_tree(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        return Err(format!(
            "refusing to overwrite KiCad action artifact {}",
            destination.display()
        ));
    }
    fs::create_dir(destination).map_err(|error| {
        format!(
            "failed to create KiCad action artifact {}: {error}",
            destination.display()
        )
    })?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to inspect KiCad artifact {}: {error}",
                entry.path().display()
            )
        })?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory_tree(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target).map_err(|error| {
                format!(
                    "failed to copy KiCad artifact {} to {}: {error}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        } else {
            return Err(format!(
                "KiCad reference-action source contains unsupported non-file artifact {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn write_pretty_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    fs::write(
        path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

mod connection_insertion;
pub use connection_insertion::*;

fn reference_action_signature(
    fields: &BTreeMap<String, (String, [f64; 3])>,
    iteration: usize,
) -> Vec<(String, [u64; 3])> {
    fields
        .iter()
        .map(|(uuid, (_, pose))| {
            (
                uuid.clone(),
                alternate_reference_pose(*pose, iteration).map(f64::to_bits),
            )
        })
        .collect()
}

fn alternate_reference_pose(source: [f64; 3], iteration: usize) -> [f64; 3] {
    let [x, y, angle] = source;
    let mut pose = match (iteration - 1) % 4 {
        0 => [-x, -y, angle],
        1 => [-y, x, angle],
        2 => [y, -x, angle],
        _ => [x, -y, angle],
    };
    for value in &mut pose {
        if *value == 0.0 {
            *value = 0.0;
        }
    }
    pose
}

fn apply_footprint_placements(
    pcb: &mut Expr,
    placements: &[KiCadFootprintPlacement],
) -> Result<(), String> {
    let mut by_reference = BTreeMap::new();
    for placement in placements {
        if placement.reference.is_empty()
            || by_reference
                .insert(placement.reference.as_str(), placement)
                .is_some()
            || placement
                .source_at
                .iter()
                .chain(placement.at.iter())
                .any(|value| !value.is_finite())
        {
            return Err("route candidate has an invalid or duplicate footprint placement".into());
        }
    }
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut applied = BTreeSet::new();
    for footprint in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        let Some(reference) = footprint_reference(footprint) else {
            continue;
        };
        let Some(placement) = by_reference.get(reference.as_str()) else {
            continue;
        };
        let current = form_at(footprint)?;
        if current
            .iter()
            .zip(placement.source_at)
            .any(|(current, source)| (current - source).abs() > 1.0e-6)
        {
            return Err(format!(
                "footprint {} is at {:?}, but route candidate expects source pose {:?}",
                placement.reference, current, placement.source_at
            ));
        }
        set_form_at(footprint, placement.at)?;
        applied.insert(reference);
    }
    if applied.len() != placements.len() {
        let missing = placements
            .iter()
            .filter(|placement| !applied.contains(&placement.reference))
            .map(|placement| placement.reference.as_str())
            .collect::<Vec<_>>();
        return Err(format!(
            "route candidate references missing footprint(s): {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

fn apply_reference_placements(
    pcb: &mut Expr,
    placements: &[KiCadReferencePlacement],
) -> Result<(), String> {
    let mut by_uuid = BTreeMap::new();
    for placement in placements {
        if placement.reference.is_empty()
            || placement.uuid.is_empty()
            || by_uuid.insert(placement.uuid.as_str(), placement).is_some()
            || placement
                .source_at
                .iter()
                .chain(placement.at.iter())
                .any(|value| !value.is_finite())
        {
            return Err("route candidate has an invalid or duplicate reference placement".into());
        }
    }
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut applied = BTreeSet::new();
    for footprint in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let Expr::List(footprint_items) = footprint else {
            unreachable!("footprint form is a list")
        };
        for property in footprint_items
            .iter_mut()
            .filter(|item| item.head() == Some("property"))
        {
            if property.children().get(1).and_then(Expr::atom) != Some("Reference") {
                continue;
            }
            let Some(uuid) = form_atom(property, "uuid", 1) else {
                continue;
            };
            let Some(placement) = by_uuid.get(uuid) else {
                continue;
            };
            if placement.reference != reference {
                return Err(format!(
                    "reference-field UUID {} belongs to {}, but route candidate names {}",
                    placement.uuid, reference, placement.reference
                ));
            }
            let current = form_at(property)?;
            if current
                .iter()
                .zip(placement.source_at)
                .any(|(current, source)| (current - source).abs() > 1.0e-6)
            {
                return Err(format!(
                    "reference field {} ({}) is at {:?}, but route candidate expects source pose {:?}",
                    placement.reference, placement.uuid, current, placement.source_at
                ));
            }
            set_form_at(property, placement.at)?;
            applied.insert(placement.uuid.clone());
        }
    }
    if applied.len() != placements.len() {
        let missing = placements
            .iter()
            .filter(|placement| !applied.contains(&placement.uuid))
            .map(|placement| format!("{} ({})", placement.reference, placement.uuid))
            .collect::<Vec<_>>();
        return Err(format!(
            "route candidate references missing reference field(s): {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

fn set_form_at(node: &mut Expr, at: [f64; 3]) -> Result<(), String> {
    let Expr::List(items) = node else {
        return Err("cannot set at form on an atom".into());
    };
    let Some(Expr::List(values)) = items.iter_mut().find(|item| item.head() == Some("at")) else {
        return Err("node has no at form".into());
    };
    values.truncate(4);
    while values.len() < 4 {
        values.push(Expr::Atom("0".into()));
    }
    values[1] = Expr::Atom(at[0].to_string());
    values[2] = Expr::Atom(at[1].to_string());
    values[3] = Expr::Atom(at[2].to_string());
    Ok(())
}

pub fn read_route_candidate(path: &Path) -> Result<KiCadRouteCandidate, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let candidate: KiCadRouteCandidate = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if candidate.schema_version != 2 {
        return Err(format!(
            "unsupported KiCad route-candidate schema {} in {}; expected 2",
            candidate.schema_version,
            path.display()
        ));
    }
    validate_grid_route_config(&candidate.config)?;
    if (candidate.supplemental_segments.is_empty() && candidate.supplemental_vias.is_empty())
        || candidate.terminals.len() < 2
        || candidate
            .supplemental_segments
            .iter()
            .any(|segment| segment.connection != candidate.connection)
        || candidate
            .supplemental_vias
            .iter()
            .any(|via| via.connection != candidate.connection)
    {
        return Err(format!(
            "route candidate {} has inconsistent or empty copper",
            path.display()
        ));
    }
    Ok(candidate)
}

/// Measures canonical physical copper plus branch-shape complexity.
///
/// Physical length and segment count split every same-layer contact and
/// deduplicate overlapping same-net fragments. Stored track-object length and
/// count remain explicit so representation debt cannot masquerade as routing
/// improvement. Bend and point counts intentionally use logical branches.
pub fn route_candidate_quality(
    candidate: &KiCadRouteCandidate,
) -> Result<KiCadRouteQuality, String> {
    let stored_track_length_mm = candidate
        .supplemental_segments
        .iter()
        .map(|segment| distance_squared(segment.start, segment.end).sqrt())
        .sum::<f64>();
    let stored_segments = candidate.supplemental_segments.len();
    let (physical_segments, physical_vias) = materialize_candidate_branches(
        &candidate.connection,
        &candidate.branches,
        &candidate.config,
    )?;
    let via_cluster_threshold_mm = candidate.config.via_size_mm + candidate.config.clearance_mm;
    let mut minimum_via_spacing_mm: Option<f64> = None;
    let mut close_via_pairs = 0;
    let mut clustered = vec![false; physical_vias.len()];
    for left in 0..physical_vias.len() {
        for right in (left + 1)..physical_vias.len() {
            let spacing = distance_squared(physical_vias[left].at, physical_vias[right].at).sqrt();
            minimum_via_spacing_mm =
                Some(minimum_via_spacing_mm.map_or(spacing, |minimum| minimum.min(spacing)));
            if spacing <= via_cluster_threshold_mm {
                close_via_pairs += 1;
                clustered[left] = true;
                clustered[right] = true;
            }
        }
    }
    let branch_vias: Vec<_> = candidate
        .branches
        .iter()
        .map(|branch| branch_via_count(&branch.path))
        .collect();
    let length_mm = physical_segments
        .iter()
        .map(|segment| distance_squared(segment.start, segment.end).sqrt())
        .sum::<f64>();
    Ok(KiCadRouteQuality {
        length_mm,
        segments: physical_segments.len(),
        stored_track_length_mm,
        stored_segments,
        overlapping_track_length_mm: stored_track_length_mm - length_mm,
        vias: physical_vias.len(),
        branch_via_transitions: branch_vias.iter().sum(),
        maximum_branch_vias: branch_vias.into_iter().max().unwrap_or(0),
        via_cluster_threshold_mm,
        minimum_via_spacing_mm,
        close_via_pairs,
        clustered_vias: clustered.into_iter().filter(|clustered| *clustered).count(),
        branch_bends: candidate
            .branches
            .iter()
            .map(|branch| branch_bend_count(&branch.path))
            .sum(),
        branch_points: candidate
            .branches
            .iter()
            .map(|branch| branch.path.len())
            .sum(),
    })
}

/// Greedily removes unnecessary same-layer branch points using conservative
/// continuous collision checks. Layer changes and via locations are fixed.
/// The result is only a proposal: callers must apply it to a complete rung and
/// run the exact KiCad verification gate before promotion.
pub fn shorten_route_candidate(
    pcb_path: &Path,
    candidate_path: &Path,
) -> Result<(KiCadRouteCandidate, KiCadRouteShorteningEvidence), String> {
    shorten_route_candidate_with_config(
        pcb_path,
        candidate_path,
        &KiCadRouteShorteningConfig::default(),
    )
}

/// Shortens a retained KiCad route using the selected bounded processor.
///
/// Both processors preserve the existing vertex order, layer runs, and via
/// locations. `VisibilityShortestPath` finds the minimum-length path through
/// the retained vertices; it does not move vertices or claim global geometric
/// optimality. The returned proposal still requires the full KiCad gate.
pub fn shorten_route_candidate_with_config(
    pcb_path: &Path,
    candidate_path: &Path,
    shortening_config: &KiCadRouteShorteningConfig,
) -> Result<(KiCadRouteCandidate, KiCadRouteShorteningEvidence), String> {
    if shortening_config.maximum_visibility_tests == 0 {
        return Err("maximum_visibility_tests must be positive".into());
    }
    let pcb_source = fs::read_to_string(pcb_path)
        .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
    let mut pcb = parse(&pcb_source)?;
    let mut candidate = read_route_candidate(candidate_path)?;
    apply_footprint_placements(&mut pcb, &candidate.footprint_placements)?;
    apply_reference_placements(&mut pcb, &candidate.reference_placements)?;
    let source_candidate = candidate.clone();
    let model =
        KiCadRoutingModel::from_pcb_with_config(&pcb, &candidate.connection, &candidate.config)?;
    let before = route_candidate_quality(&candidate)?;
    let mut attempted_shortcuts = 0;
    let mut visible_shortcuts = 0;
    let mut accepted_shortcuts = 0;
    let mut visibility_tests = 0;
    for branch in &mut candidate.branches {
        let (path, attempted, visible, accepted) = shorten_branch_path_with_strategy(
            &branch.path,
            &model,
            &candidate.config,
            shortening_config,
            &mut visibility_tests,
        )?;
        branch.path = path;
        attempted_shortcuts += attempted;
        visible_shortcuts += visible;
        accepted_shortcuts += accepted;
    }
    let (segments, vias) = materialize_candidate_branches(
        &candidate.connection,
        &candidate.branches,
        &candidate.config,
    )?;
    candidate.supplemental_segments = segments;
    candidate.supplemental_vias = vias;
    let algorithm = match shortening_config.strategy {
        KiCadRouteShorteningStrategy::GreedyLineOfSight => "conservative-line-of-sight-v1",
        KiCadRouteShorteningStrategy::VisibilityShortestPath => {
            "retained-visibility-shortest-path-v1"
        }
    };
    let source_router = std::mem::replace(&mut candidate.router, algorithm.to_string());
    let proposal = route_candidate_quality(&candidate)?;
    let removed_branch_points = before.branch_points.saturating_sub(proposal.branch_points);
    let improves = proposal.length_mm + 1.0e-9 < before.length_mm;
    let (candidate, status, detail, after) = if improves {
        (
            candidate,
            KiCadRouteShorteningStatus::Accepted,
            "proposal improved canonical physical copper length".into(),
            proposal.clone(),
        )
    } else {
        (
            source_candidate,
            KiCadRouteShorteningStatus::NoImprovement,
            "proposal did not improve canonical physical copper length".into(),
            before.clone(),
        )
    };
    let evidence = KiCadRouteShorteningEvidence {
        schema_version: 1,
        connection: candidate.connection.clone(),
        algorithm: algorithm.into(),
        source_router,
        source_cost: candidate.cost,
        source_expansions: candidate.expansions,
        strategy: shortening_config.strategy,
        maximum_visibility_tests: shortening_config.maximum_visibility_tests,
        visibility_tests,
        attempted_shortcuts,
        visible_shortcuts,
        accepted_shortcuts,
        removed_branch_points,
        status,
        detail,
        before,
        proposal,
        after,
        exact_kicad_validation_required: true,
    };
    Ok((candidate, evidence))
}

#[derive(Debug, Default)]
struct RelaxationObstacleSelection {
    selected_runs_by_obstacle: BTreeMap<usize, Vec<usize>>,
    layer_relevant_pairs: usize,
    candidate_pairs: usize,
    retained_pairs: usize,
    broad_phase_cells: usize,
    broad_phase_cell_references: usize,
    broad_phase_cell_queries: usize,
}

#[derive(Clone, Debug)]
struct RelaxationLayerRun {
    id: String,
    branch_index: usize,
    start_point: usize,
    end_point: usize,
    layer: usize,
}

impl RelaxationLayerRun {
    fn len(&self) -> usize {
        self.end_point - self.start_point + 1
    }
}

fn selected_layer_runs(
    candidate: &KiCadRouteCandidate,
    selected: &[usize],
) -> Result<Vec<RelaxationLayerRun>, String> {
    let mut runs = Vec::new();
    for &branch_index in selected {
        let branch = &candidate.branches[branch_index];
        let mut start_point = 0;
        for point_index in 0..branch.path.len() {
            if copper_layer_index(&branch.path[point_index].layer).is_none() {
                return Err(format!(
                    "selected branch {branch_index} uses unsupported layer {:?}",
                    branch.path[point_index].layer
                ));
            }
            let transition = point_index + 1 < branch.path.len()
                && branch.path[point_index].layer != branch.path[point_index + 1].layer;
            if !transition {
                continue;
            }
            if branch.path[point_index].at != branch.path[point_index + 1].at {
                return Err(format!(
                    "selected branch {branch_index} layer transition {point_index} changes coordinate"
                ));
            }
            if point_index == start_point {
                return Err(format!(
                    "selected branch {branch_index} has a layer run without a copper segment"
                ));
            }
            let layer = copper_layer_index(&branch.path[start_point].layer)
                .expect("the selected layer was validated");
            runs.push(RelaxationLayerRun {
                id: format!("selected-branch-{branch_index}-run-{}", runs.len()),
                branch_index,
                start_point,
                end_point: point_index,
                layer,
            });
            start_point = point_index + 1;
        }
        if branch.path.len() - 1 == start_point {
            return Err(format!(
                "selected branch {branch_index} has a layer run without a copper segment"
            ));
        }
        let layer = copper_layer_index(&branch.path[start_point].layer)
            .expect("the selected layer was validated");
        runs.push(RelaxationLayerRun {
            id: format!("selected-branch-{branch_index}-run-{}", runs.len()),
            branch_index,
            start_point,
            end_point: branch.path.len() - 1,
            layer,
        });
    }
    Ok(runs)
}

fn merged_shared_points(raw_groups: Vec<Vec<CopperPointRef>>) -> Vec<CopperSharedPointInput> {
    let mut components = Vec::<BTreeSet<(String, usize)>>::new();
    for group in raw_groups {
        let mut merged = group
            .into_iter()
            .map(|member| (member.polyline, member.point_index))
            .collect::<BTreeSet<_>>();
        if merged.len() < 2 {
            continue;
        }
        let mut index = 0;
        while index < components.len() {
            if components[index]
                .iter()
                .any(|member| merged.contains(member))
            {
                merged.extend(components.remove(index));
            } else {
                index += 1;
            }
        }
        components.push(merged);
    }
    components.sort_by(|left, right| left.iter().next().cmp(&right.iter().next()));
    components
        .into_iter()
        .enumerate()
        .map(|(group, members)| CopperSharedPointInput {
            id: format!("selected-route-point-{group}"),
            members: members
                .into_iter()
                .map(|(polyline, point_index)| CopperPointRef {
                    polyline,
                    point_index,
                })
                .collect(),
        })
        .collect()
}

fn select_relaxation_obstacles(
    candidate: &KiCadRouteCandidate,
    model: &KiCadRoutingModel,
    runs: &[RelaxationLayerRun],
    config: &KiCadRouteRelaxationConfig,
    excluded_footprints: &BTreeSet<String>,
) -> RelaxationObstacleSelection {
    let mut selection = RelaxationObstacleSelection::default();
    let grid = (config.broad_phase == KiCadRouteBroadPhase::UniformGrid).then(|| {
        KiCadObstacleGrid::new(
            model,
            config.broad_phase_cell_size_mm,
            candidate.config.clearance_mm,
        )
    });
    if let Some(grid) = &grid {
        selection.broad_phase_cells = grid.cells.len();
        selection.broad_phase_cell_references = grid.cell_references;
    }
    let collision_and_motion_margin = candidate.config.trace_width_mm / 2.0
        + candidate.config.clearance_mm
        + config.constraint_guard_mm
        + config.broad_phase_motion_margin_mm;

    for (run_index, run) in runs.iter().enumerate() {
        let branch = &candidate.branches[run.branch_index];
        let envelopes = branch.path[run.start_point..=run.end_point]
            .windows(2)
            .map(|pair| {
                GeometryAabb::around_segment(pair[0].at, pair[1].at, collision_and_motion_margin)
            })
            .collect::<Vec<_>>();
        selection.layer_relevant_pairs += model
            .obstacles
            .iter()
            .filter(|obstacle| {
                obstacle.blocks_tracks
                    && obstacle.layers[run.layer]
                    && !obstacle
                        .footprint
                        .as_ref()
                        .is_some_and(|reference| excluded_footprints.contains(reference))
            })
            .count();

        let candidates = if let Some(grid) = &grid {
            let mut candidates = BTreeSet::new();
            for envelope in &envelopes {
                let (found, cell_queries) = grid.query(run.layer, *envelope);
                candidates.extend(found);
                selection.broad_phase_cell_queries += cell_queries;
            }
            candidates
        } else {
            model
                .obstacles
                .iter()
                .enumerate()
                .filter(|(_, obstacle)| {
                    obstacle.blocks_tracks
                        && obstacle.layers[run.layer]
                        && !obstacle
                            .footprint
                            .as_ref()
                            .is_some_and(|reference| excluded_footprints.contains(reference))
                })
                .map(|(index, _)| index)
                .collect()
        };
        selection.candidate_pairs += candidates.len();
        for obstacle_index in candidates {
            let obstacle = &model.obstacles[obstacle_index];
            if !obstacle.blocks_tracks
                || obstacle
                    .footprint
                    .as_ref()
                    .is_some_and(|reference| excluded_footprints.contains(reference))
            {
                continue;
            }
            let retained = config.broad_phase == KiCadRouteBroadPhase::Exhaustive
                || envelopes.iter().any(|envelope| {
                    envelope.intersects(obstacle.query_bounds(candidate.config.clearance_mm))
                });
            if retained {
                selection
                    .selected_runs_by_obstacle
                    .entry(obstacle_index)
                    .or_default()
                    .push(run_index);
                selection.retained_pairs += 1;
            }
        }
    }
    selection
}

#[derive(Clone, Copy, Debug)]
struct SelectedRouteSegment {
    branch_index: usize,
    segment_index: usize,
    start: [f64; 2],
    end: [f64; 2],
}

#[derive(Clone, Copy, Debug, Default)]
struct RouteGraphNormalization {
    contact_points: usize,
    inserted_points: usize,
}

const ROUTE_CONTACT_EPSILON_MM: f64 = 1.0e-9;

fn route_cross(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[1] - left[1] * right[0]
}

fn route_subtract(left: [f64; 2], right: [f64; 2]) -> [f64; 2] {
    [left[0] - right[0], left[1] - right[1]]
}

fn route_point_on_segment(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> bool {
    let direction = route_subtract(end, start);
    let offset = route_subtract(point, start);
    let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
    if length_squared <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM {
        return distance_squared(point, start)
            <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM;
    }
    let scale = length_squared.sqrt().max(1.0);
    if route_cross(direction, offset).abs() > ROUTE_CONTACT_EPSILON_MM * scale {
        return false;
    }
    let projection = offset[0] * direction[0] + offset[1] * direction[1];
    projection >= -ROUTE_CONTACT_EPSILON_MM
        && projection <= length_squared + ROUTE_CONTACT_EPSILON_MM
}

fn route_segment_parameter(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let direction = route_subtract(end, start);
    let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
    if length_squared == 0.0 {
        0.0
    } else {
        let offset = route_subtract(point, start);
        (offset[0] * direction[0] + offset[1] * direction[1]) / length_squared
    }
}

/// Returns the physical contact points shared by two same-layer segments.
///
/// Collinear overlap is represented by its endpoint vertices. Proper
/// crossings get one computed point, while endpoint/T contacts retain the
/// exact stored endpoint coordinate so later bitwise grouping is stable.
fn route_segment_contacts(
    first_start: [f64; 2],
    first_end: [f64; 2],
    second_start: [f64; 2],
    second_end: [f64; 2],
) -> Vec<[f64; 2]> {
    let mut contacts = Vec::new();
    for point in [first_start, first_end, second_start, second_end] {
        if route_point_on_segment(point, first_start, first_end)
            && route_point_on_segment(point, second_start, second_end)
            && !contacts.iter().any(|existing| {
                distance_squared(*existing, point)
                    <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM
            })
        {
            contacts.push(point);
        }
    }
    if !contacts.is_empty() {
        return contacts;
    }

    let first_direction = route_subtract(first_end, first_start);
    let second_direction = route_subtract(second_end, second_start);
    let denominator = route_cross(first_direction, second_direction);
    let scale = (distance_squared(first_start, first_end)
        * distance_squared(second_start, second_end))
    .sqrt()
    .max(1.0);
    if denominator.abs() <= ROUTE_CONTACT_EPSILON_MM * scale {
        return contacts;
    }
    let between_starts = route_subtract(second_start, first_start);
    let first_parameter = route_cross(between_starts, second_direction) / denominator;
    let second_parameter = route_cross(between_starts, first_direction) / denominator;
    let contact_range = -ROUTE_CONTACT_EPSILON_MM..=1.0 + ROUTE_CONTACT_EPSILON_MM;
    if contact_range.contains(&first_parameter) && contact_range.contains(&second_parameter) {
        contacts.push([
            first_start[0] + first_parameter * first_direction[0],
            first_start[1] + first_parameter * first_direction[1],
        ]);
    }
    contacts
}

/// Inserts explicit vertices wherever selected same-layer routes physically
/// touch. The operation preserves the initial copper geometry but turns
/// collinear overlap, T contacts, and crossings into a graph that the generic
/// shared-particle compiler can represent without duplicate physical edges.
fn normalize_selected_route_graph(
    branches: &mut [KiCadRouteBranch],
    selected: &[usize],
) -> RouteGraphNormalization {
    normalize_selected_route_graph_against(branches, selected, selected)
}

fn normalize_selected_route_graph_against(
    branches: &mut [KiCadRouteBranch],
    selected: &[usize],
    context: &[usize],
) -> RouteGraphNormalization {
    let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
    let original_paths = context
        .iter()
        .map(|&branch_index| (branch_index, branches[branch_index].path.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut segments = Vec::new();
    let mut cuts = BTreeMap::<(usize, usize), Vec<[f64; 2]>>::new();
    for (&branch_index, path) in &original_paths {
        for (segment_index, pair) in path.windows(2).enumerate() {
            if pair[0].layer != pair[1].layer || pair[0].at == pair[1].at {
                continue;
            }
            segments.push(SelectedRouteSegment {
                branch_index,
                segment_index,
                start: pair[0].at,
                end: pair[1].at,
            });
            if selected_set.contains(&branch_index) {
                cuts.insert((branch_index, segment_index), vec![pair[0].at, pair[1].at]);
            }
        }
    }

    let mut contact_keys = BTreeSet::new();
    for first in 0..segments.len() {
        for second in (first + 1)..segments.len() {
            let first_segment = segments[first];
            let second_segment = segments[second];
            let first_path = &original_paths[&first_segment.branch_index];
            let second_path = &original_paths[&second_segment.branch_index];
            if first_path[first_segment.segment_index].layer
                != second_path[second_segment.segment_index].layer
            {
                continue;
            }
            let contacts = route_segment_contacts(
                first_segment.start,
                first_segment.end,
                second_segment.start,
                second_segment.end,
            );
            // Ordinary adjacent segments only share their existing vertex.
            // A backtrack shares an interval: split its endpoints just like
            // overlaps between different branches so physical edges dedupe.
            if first_segment.branch_index == second_segment.branch_index
                && first_segment
                    .segment_index
                    .abs_diff(second_segment.segment_index)
                    == 1
                && contacts.len() <= 1
            {
                continue;
            }
            for contact in contacts {
                let mut touches_selected = false;
                if let Some(segment_cuts) =
                    cuts.get_mut(&(first_segment.branch_index, first_segment.segment_index))
                {
                    segment_cuts.push(contact);
                    touches_selected = true;
                }
                if let Some(segment_cuts) =
                    cuts.get_mut(&(second_segment.branch_index, second_segment.segment_index))
                {
                    segment_cuts.push(contact);
                    touches_selected = true;
                }
                if touches_selected {
                    contact_keys.insert((
                        contact[0].to_bits(),
                        contact[1].to_bits(),
                        first_path[first_segment.segment_index].layer.clone(),
                    ));
                }
            }
        }
    }

    let mut inserted_points = 0;
    for &branch_index in selected {
        let original = &original_paths[&branch_index];
        // A terminal already reached by the tree can have a single contact
        // point and no new segment. Preserve that connectivity evidence.
        if original.len() < 2 {
            continue;
        }
        let mut normalized = Vec::new();
        for (segment_index, pair) in original.windows(2).enumerate() {
            let Some(segment_cuts) = cuts.get(&(branch_index, segment_index)) else {
                if normalized.is_empty() {
                    normalized.push(pair[0].clone());
                }
                normalized.push(pair[1].clone());
                continue;
            };
            let mut segment_cuts = segment_cuts.clone();
            segment_cuts.sort_by(|left, right| {
                route_segment_parameter(*left, pair[0].at, pair[1].at)
                    .total_cmp(&route_segment_parameter(*right, pair[0].at, pair[1].at))
                    .then_with(|| left[0].total_cmp(&right[0]))
                    .then_with(|| left[1].total_cmp(&right[1]))
            });
            segment_cuts.dedup_by(|left, right| {
                distance_squared(*left, *right)
                    <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM
            });
            for (cut_index, at) in segment_cuts.into_iter().enumerate() {
                if segment_index > 0 && cut_index == 0 {
                    continue;
                }
                normalized.push(KiCadRoutePoint {
                    at,
                    layer: pair[0].layer.clone(),
                });
            }
        }
        inserted_points += normalized.len().saturating_sub(original.len());
        branches[branch_index].path = normalized;
    }
    RouteGraphNormalization {
        contact_points: contact_keys.len(),
        inserted_points,
    }
}

#[derive(Debug)]
struct RelaxationBranchSelection {
    selected: Vec<usize>,
    contact_tests: usize,
}

fn nearest_unselected_branch_neighbors(
    candidate: &KiCadRouteCandidate,
    source_index: usize,
    selected: &BTreeSet<usize>,
    contact_tests: &mut usize,
    maximum_contact_tests: usize,
) -> Result<Vec<usize>, String> {
    let source = &candidate.branches[source_index];
    let mut nearest_by_branch = BTreeMap::<usize, f64>::new();
    let mut distance_from_leaf = 0.0;

    for source_pair in source.path.windows(2).rev() {
        let source_length = distance_squared(source_pair[0].at, source_pair[1].at).sqrt();
        if source_pair[0].layer == source_pair[1].layer && source_length > 0.0 {
            for (target_index, target) in candidate.branches.iter().enumerate() {
                if selected.contains(&target_index) {
                    continue;
                }
                for target_pair in target.path.windows(2) {
                    if target_pair[0].layer != source_pair[0].layer
                        || target_pair[1].layer != source_pair[0].layer
                        || target_pair[0].at == target_pair[1].at
                    {
                        continue;
                    }
                    *contact_tests += 1;
                    if *contact_tests > maximum_contact_tests {
                        return Err(format!(
                            "nearest-shared-junction selection exceeded maximum_branch_contact_tests={maximum_contact_tests}"
                        ));
                    }
                    for contact in route_segment_contacts(
                        source_pair[0].at,
                        source_pair[1].at,
                        target_pair[0].at,
                        target_pair[1].at,
                    ) {
                        let distance = distance_from_leaf
                            + distance_squared(contact, source_pair[1].at).sqrt();
                        nearest_by_branch
                            .entry(target_index)
                            .and_modify(|nearest| *nearest = nearest.min(distance))
                            .or_insert(distance);
                    }
                }
            }
        }
        distance_from_leaf += source_length;
    }

    let Some(nearest_distance) = nearest_by_branch.values().copied().min_by(f64::total_cmp) else {
        return Ok(Vec::new());
    };
    Ok(nearest_by_branch
        .into_iter()
        .filter_map(|(branch, distance)| {
            ((distance - nearest_distance).abs() <= ROUTE_CONTACT_EPSILON_MM).then_some(branch)
        })
        .collect())
}

fn select_relaxation_branches(
    candidate: &KiCadRouteCandidate,
    config: &KiCadRouteRelaxationConfig,
) -> Result<RelaxationBranchSelection, (String, usize)> {
    let mut seeds = config.branch_indices.clone();
    seeds.sort_unstable();
    for &index in &seeds {
        if index >= candidate.branches.len() {
            return Err((format!("selected branch {index} does not exist"), 0));
        }
    }
    if config.branch_selection == KiCadBranchSelectionPolicy::Explicit {
        return Ok(RelaxationBranchSelection {
            selected: seeds,
            contact_tests: 0,
        });
    }

    let mut selected = seeds.iter().copied().collect::<BTreeSet<_>>();
    let mut frontier = selected.clone();
    let mut contact_tests = 0;
    for _ in 0..config.branch_neighborhood_hops {
        let mut additions = BTreeSet::new();
        for &source_index in &frontier {
            let neighbors = nearest_unselected_branch_neighbors(
                candidate,
                source_index,
                &selected,
                &mut contact_tests,
                config.maximum_branch_contact_tests,
            )
            .map_err(|detail| (detail, contact_tests))?;
            additions.extend(neighbors);
        }
        additions.retain(|index| !selected.contains(index));
        if selected.len() + additions.len() > config.maximum_selected_branches {
            return Err((
                format!(
                    "nearest-shared-junction selection would choose {} branches, exceeding maximum_selected_branches={}",
                    selected.len() + additions.len(),
                    config.maximum_selected_branches
                ),
                contact_tests,
            ));
        }
        if additions.is_empty() {
            break;
        }
        selected.extend(additions.iter().copied());
        frontier = additions;
    }
    Ok(RelaxationBranchSelection {
        selected: selected.into_iter().collect(),
        contact_tests,
    })
}

/// Moves selected branch interiors with the continuous engine while keeping
/// their KiCad pad endpoints fixed. With `via_mobility = fixed`, multilayer
/// branches are lowered into same-layer particle runs joined by fixed shared
/// anchors at the existing vias.
///
/// The adapter lowers every layer-relevant non-target circle, capsule, and
/// rectangle from the same routing model used by grid routing and retained
/// shortcutting. The returned candidate is still provisional until it is
/// applied to a complete project and accepted by native KiCad
/// ERC/DRC/parity/connectivity checks.
pub fn relax_route_candidate(
    pcb_path: &Path,
    candidate_path: &Path,
    relaxation_config: &KiCadRouteRelaxationConfig,
) -> Result<
    (
        KiCadRouteCandidate,
        Option<KiCadRouteCandidate>,
        KiCadRouteRelaxationEvidence,
    ),
    String,
> {
    validate_route_relaxation_config(relaxation_config)?;
    let algorithm = route_relaxation_algorithm(relaxation_config);
    let pcb_source = fs::read_to_string(pcb_path)
        .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
    let mut pcb = parse(&pcb_source)?;
    let candidate = read_route_candidate(candidate_path)?;
    apply_footprint_placements(&mut pcb, &candidate.footprint_placements)?;
    apply_reference_placements(&mut pcb, &candidate.reference_placements)?;
    let model =
        KiCadRoutingModel::from_pcb_with_config(&pcb, &candidate.connection, &candidate.config)?;
    let footprint_infos = footprint_infos(&pcb)?;
    let movable_footprints = relaxation_config
        .movable_footprints
        .iter()
        .map(|config| (config.reference.clone(), config))
        .collect::<BTreeMap<_, _>>();
    if movable_footprints.len() != relaxation_config.movable_footprints.len() {
        return Err("KiCad route relaxation movable_footprints must be unique".into());
    }
    for (reference, config) in &movable_footprints {
        let info = footprint_infos
            .get(reference)
            .ok_or_else(|| format!("unknown movable footprint {reference}"))?;
        if !info.router_movable {
            return Err(format!(
                "footprint {reference} is not explicitly marked Router_Movable=yes"
            ));
        }
        validate_movable_footprint_config(config)?;
    }
    let before = route_candidate_quality(&candidate)?;
    let source_router = candidate.router.clone();

    let branch_selection = match select_relaxation_branches(&candidate, relaxation_config) {
        Ok(selection) => selection,
        Err((detail, contact_tests)) => {
            let mut seeds = relaxation_config.branch_indices.clone();
            seeds.sort_unstable();
            return Ok(unsupported_route_relaxation(
                candidate,
                before,
                source_router,
                relaxation_config,
                seeds,
                contact_tests,
                detail,
            ));
        }
    };
    let selected = branch_selection.selected;
    let branch_contact_tests = branch_selection.contact_tests;
    for &index in &selected {
        let branch = &candidate.branches[index];
        if branch.path.len() < 3 {
            return Ok(unsupported_route_relaxation(
                candidate,
                before,
                source_router,
                relaxation_config,
                selected,
                branch_contact_tests,
                format!("selected branch {index} has no movable interior point"),
            ));
        }
        let has_layer_transition = branch
            .path
            .windows(2)
            .any(|pair| pair[0].layer != pair[1].layer);
        if has_layer_transition && relaxation_config.via_mobility == KiCadViaMobilityPolicy::Reject
        {
            return Ok(unsupported_route_relaxation(
                candidate,
                before,
                source_router,
                relaxation_config,
                selected,
                branch_contact_tests,
                format!("selected branch {index} contains a layer transition or via"),
            ));
        }
        for (point_index, point) in branch.path.iter().enumerate() {
            if copper_layer_index(&point.layer).is_none() {
                let detail = format!(
                    "selected branch {index} point {point_index} uses unsupported layer {:?}",
                    point.layer
                );
                return Ok(unsupported_route_relaxation(
                    candidate,
                    before,
                    source_router,
                    relaxation_config,
                    selected,
                    branch_contact_tests,
                    detail,
                ));
            }
        }
        if let Some((transition, _)) = branch
            .path
            .windows(2)
            .enumerate()
            .find(|(_, pair)| pair[0].layer != pair[1].layer && pair[0].at != pair[1].at)
        {
            return Ok(unsupported_route_relaxation(
                candidate,
                before,
                source_router,
                relaxation_config,
                selected,
                branch_contact_tests,
                format!("selected branch {index} layer transition {transition} changes coordinate"),
            ));
        }
    }

    let trace_radius = candidate.config.trace_width_mm / 2.0;
    let edge_margin = candidate.config.edge_clearance_mm + trace_radius;
    let engine_bounds = [
        model.bounds[0] + edge_margin,
        model.bounds[1] + edge_margin,
        model.bounds[2] - edge_margin,
        model.bounds[3] - edge_margin,
    ];
    if engine_bounds[0] >= engine_bounds[2] || engine_bounds[1] >= engine_bounds[3] {
        return Err("KiCad route relaxation edge clearance leaves no engine bounds".into());
    }

    let initial_geometry_clear =
        selected_geometry_blockers(&candidate, &selected, &model).is_empty();
    let mut engine_candidate = candidate.clone();
    let route_graph_normalization = if relaxation_config.shared_copper
        == KiCadSharedCopperPolicy::SharedRouteGraph
    {
        let context = (0..engine_candidate.branches.len()).collect::<Vec<_>>();
        normalize_selected_route_graph_against(&mut engine_candidate.branches, &selected, &context)
    } else {
        RouteGraphNormalization::default()
    };
    let layer_runs = match selected_layer_runs(&engine_candidate, &selected) {
        Ok(runs) => runs,
        Err(detail) => {
            return Ok(unsupported_route_relaxation(
                candidate,
                before,
                source_router,
                relaxation_config,
                selected,
                branch_contact_tests,
                detail,
            ));
        }
    };
    let mut polylines = Vec::new();
    let mut point_refs = BTreeMap::<(usize, usize), CopperPointRef>::new();
    for run in &layer_runs {
        let branch = &engine_candidate.branches[run.branch_index];
        let moves_terminal = [branch.path.first(), branch.path.last()]
            .into_iter()
            .flatten()
            .filter_map(|point| terminal_pad_at(&model, point.at))
            .any(|terminal| movable_footprints.contains_key(&terminal.footprint));
        polylines.push(CopperPolylineInput {
            id: run.id.clone(),
            points: branch.path[run.start_point..=run.end_point]
                .iter()
                .enumerate()
                .map(|(point, value)| {
                    engine_vec2(
                        value.at,
                        &format!("branch {} run point {point}", run.branch_index),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
            width: engine_f32(candidate.config.trace_width_mm, "trace width")?,
            tension_weight: 1.0,
            mobility: if moves_terminal {
                CopperPolylineMobility::All
            } else {
                CopperPolylineMobility::Interior
            },
        });
        for point_index in run.start_point..=run.end_point {
            point_refs.insert(
                (run.branch_index, point_index),
                CopperPointRef {
                    polyline: run.id.clone(),
                    point_index: point_index - run.start_point,
                },
            );
        }
    }
    let mut coincident_members = BTreeMap::<(u64, u64, String), Vec<CopperPointRef>>::new();
    for &branch_index in &selected {
        for (point_index, point) in engine_candidate.branches[branch_index]
            .path
            .iter()
            .enumerate()
        {
            coincident_members
                .entry((
                    point.at[0].to_bits(),
                    point.at[1].to_bits(),
                    point.layer.clone(),
                ))
                .or_default()
                .push(point_refs[&(branch_index, point_index)].clone());
        }
    }
    let mut raw_shared_groups = if matches!(
        relaxation_config.shared_copper,
        KiCadSharedCopperPolicy::SharedPoints | KiCadSharedCopperPolicy::SharedRouteGraph
    ) {
        coincident_members
            .into_values()
            .filter(|members| members.len() > 1)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
    let mut exact_fixed_points = BTreeSet::<(usize, usize)>::new();
    let mut fixed_context_members = BTreeMap::<(u64, u64, String), Vec<CopperPointRef>>::new();
    if matches!(
        relaxation_config.shared_copper,
        KiCadSharedCopperPolicy::SharedPoints | KiCadSharedCopperPolicy::SharedRouteGraph
    ) {
        for &branch_index in &selected {
            for (point_index, point) in engine_candidate.branches[branch_index]
                .path
                .iter()
                .enumerate()
            {
                let touches_unselected = engine_candidate
                    .branches
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| !selected_set.contains(index))
                    .flat_map(|(_, branch)| branch.path.windows(2))
                    .any(|pair| {
                        pair[0].layer == point.layer
                            && pair[1].layer == point.layer
                            && route_point_on_segment(point.at, pair[0].at, pair[1].at)
                    });
                if touches_unselected {
                    exact_fixed_points.insert((branch_index, point_index));
                    fixed_context_members
                        .entry((
                            point.at[0].to_bits(),
                            point.at[1].to_bits(),
                            point.layer.clone(),
                        ))
                        .or_default()
                        .push(point_refs[&(branch_index, point_index)].clone());
                }
            }
        }
    }
    let fixed_context_anchors = fixed_context_members.len();
    let fixed_context_anchor_members = fixed_context_members.values().map(Vec::len).sum::<usize>();
    for (anchor_index, ((x, y, _), mut members)) in fixed_context_members.into_iter().enumerate() {
        let id = format!("fixed-context-anchor-{anchor_index}");
        let at = engine_vec2([f64::from_bits(x), f64::from_bits(y)], &id)?;
        polylines.push(CopperPolylineInput {
            id: id.clone(),
            points: vec![at, at],
            width: engine_f32(candidate.config.trace_width_mm, "context anchor width")?,
            tension_weight: 0.0,
            mobility: CopperPolylineMobility::Fixed,
        });
        members.push(CopperPointRef {
            polyline: id,
            point_index: 0,
        });
        raw_shared_groups.push(members);
    }
    let mut via_members = BTreeMap::<(u64, u64), Vec<CopperPointRef>>::new();
    for &branch_index in &selected {
        let branch = &engine_candidate.branches[branch_index];
        for (point_index, pair) in branch.path.windows(2).enumerate() {
            if pair[0].layer == pair[1].layer {
                continue;
            }
            exact_fixed_points
                .extend([(branch_index, point_index), (branch_index, point_index + 1)]);
            via_members
                .entry((pair[0].at[0].to_bits(), pair[0].at[1].to_bits()))
                .or_default()
                .extend([
                    point_refs[&(branch_index, point_index)].clone(),
                    point_refs[&(branch_index, point_index + 1)].clone(),
                ]);
        }
    }
    let fixed_via_anchors = via_members.len();
    let fixed_via_anchor_members = via_members.values().map(Vec::len).sum::<usize>();
    for (anchor_index, ((x, y), mut members)) in via_members.into_iter().enumerate() {
        let id = format!("fixed-via-anchor-{anchor_index}");
        let at = engine_vec2([f64::from_bits(x), f64::from_bits(y)], &id)?;
        polylines.push(CopperPolylineInput {
            id: id.clone(),
            points: vec![at, at],
            width: engine_f32(candidate.config.trace_width_mm, "via anchor width")?,
            tension_weight: 0.0,
            mobility: CopperPolylineMobility::Fixed,
        });
        members.push(CopperPointRef {
            polyline: id,
            point_index: 0,
        });
        raw_shared_groups.push(members);
    }
    let shared_points = merged_shared_points(raw_shared_groups);

    let excluded_footprints = movable_footprints.keys().cloned().collect::<BTreeSet<_>>();
    let obstacle_selection = select_relaxation_obstacles(
        &engine_candidate,
        &model,
        &layer_runs,
        relaxation_config,
        &excluded_footprints,
    );

    let mut bodies = Vec::new();
    let mut attachments = Vec::new();
    let mut terminal_body_ids = BTreeMap::<String, String>::new();
    let mut attached_shared_endpoints = BTreeSet::<(u64, u64, String)>::new();
    for &branch_index in &selected {
        let branch = &engine_candidate.branches[branch_index];
        let branch_moves_terminal = [branch.path.first(), branch.path.last()]
            .into_iter()
            .flatten()
            .filter_map(|point| terminal_pad_at(&model, point.at))
            .any(|terminal| movable_footprints.contains_key(&terminal.footprint));
        if !branch_moves_terminal {
            continue;
        }
        for point_index in [0, branch.path.len() - 1] {
            let point = &branch.path[point_index];
            let terminal = terminal_pad_at(&model, point.at).ok_or_else(|| {
                format!(
                    "branch {branch_index} endpoint {:?} is not owned by a target pad",
                    point.at
                )
            })?;
            let info = footprint_infos.get(&terminal.footprint).ok_or_else(|| {
                format!("missing footprint information for {}", terminal.footprint)
            })?;
            let body_id = terminal_body_ids
                .entry(info.reference.clone())
                .or_insert_with(|| format!("terminal-footprint:{}", info.reference))
                .clone();
            if !bodies
                .iter()
                .any(|body: &CopperBodyInput| body.id == body_id)
            {
                let mobility = movable_footprints
                    .get(&info.reference)
                    .map(|config| Mobility {
                        inverse_mass: config.inverse_mass as f32,
                        rotation_mobility: config.rotation_mobility as f32,
                        translate_x: config.translate_x,
                        translate_y: config.translate_y,
                        rotate: config.rotate,
                    })
                    .unwrap_or(Mobility::FIXED);
                bodies.push(CopperBodyInput {
                    id: body_id.clone(),
                    position: engine_vec2(footprint_body_center(info), &body_id)?,
                    angle_radians: engine_f32(
                        -info.at[2].to_radians(),
                        &format!("{body_id} angle"),
                    )?,
                    size: engine_vec2(info.body_size, &format!("{body_id} size"))?,
                    mobility,
                });
            }
            let shared_key = (
                point.at[0].to_bits(),
                point.at[1].to_bits(),
                body_id.clone(),
            );
            if relaxation_config.shared_copper != KiCadSharedCopperPolicy::IndependentBranches
                && !attached_shared_endpoints.insert(shared_key)
            {
                continue;
            }
            attachments.push(CopperAttachmentInput {
                polyline: point_refs[&(branch_index, point_index)].polyline.clone(),
                point_index: point_refs[&(branch_index, point_index)].point_index,
                body: body_id,
                local_position: engine_vec2(
                    [
                        terminal.local_position[0] - info.body_center_local[0],
                        terminal.local_position[1] - info.body_center_local[1],
                    ],
                    &format!("{} terminal", terminal.footprint),
                )?,
            });
        }
    }
    let terminal_attachment_bodies = terminal_body_ids.len();
    let mut body_body_separations = Vec::new();
    let mut placement_clearance_bodies = 0;
    if relaxation_config.placement_clearance
        == KiCadPlacementClearancePolicy::PreserveInitialCourtyard
    {
        let mut placement_pairs = Vec::<(String, String)>::new();
        for reference in movable_footprints.keys() {
            for other in footprint_infos.keys() {
                if reference == other
                    || (movable_footprints.contains_key(other) && reference > other)
                    || !footprint_motion_envelopes_can_meet(
                        &footprint_infos[reference],
                        movable_footprints[reference],
                        &footprint_infos[other],
                        movable_footprints.get(other).copied(),
                    )
                {
                    continue;
                }
                placement_pairs.push((reference.clone(), other.clone()));
            }
        }
        let placement_references = placement_pairs
            .iter()
            .flat_map(|(first, second)| [first.clone(), second.clone()])
            .collect::<BTreeSet<_>>();
        placement_clearance_bodies = placement_references.len();
        for reference in &placement_references {
            let info = &footprint_infos[reference];
            let body_id = terminal_body_ids
                .entry(reference.clone())
                .or_insert_with(|| format!("terminal-footprint:{reference}"))
                .clone();
            if !bodies
                .iter()
                .any(|body: &CopperBodyInput| body.id == body_id)
            {
                let mobility = movable_footprints
                    .get(reference)
                    .map(|config| Mobility {
                        inverse_mass: config.inverse_mass as f32,
                        rotation_mobility: config.rotation_mobility as f32,
                        translate_x: config.translate_x,
                        translate_y: config.translate_y,
                        rotate: config.rotate,
                    })
                    .unwrap_or(Mobility::FIXED);
                bodies.push(CopperBodyInput {
                    id: body_id.clone(),
                    position: engine_vec2(footprint_body_center(info), &body_id)?,
                    angle_radians: engine_f32(
                        -info.at[2].to_radians(),
                        &format!("{body_id} angle"),
                    )?,
                    size: engine_vec2(info.body_size, &format!("{body_id} size"))?,
                    mobility,
                });
            }
        }
        for (first, second) in placement_pairs {
            body_body_separations.push(CopperBodySeparationPair {
                first: terminal_body_ids[&first].clone(),
                second: terminal_body_ids[&second].clone(),
                minimum: CopperBodySeparationMinimum::PreserveInitial,
            });
        }
    }
    let placement_clearance_pairs = body_body_separations.len();
    let mut separations = Vec::new();
    let mut body_separations = Vec::new();
    let mut fixed_capsule_obstacles = 0;
    let mut fixed_rectangular_obstacles = 0;
    for (&obstacle_index, active_selected_runs) in &obstacle_selection.selected_runs_by_obstacle {
        let obstacle = &model.obstacles[obstacle_index];
        let clearance = engine_f32(
            obstacle.clearance(candidate.config.clearance_mm)
                + relaxation_config.constraint_guard_mm,
            "guarded obstacle clearance",
        )?;
        let active_selected_ids = active_selected_runs
            .iter()
            .map(|index| &layer_runs[*index].id)
            .collect::<Vec<_>>();
        match &obstacle.geometry {
            ObstacleGeometry::Circle { center, radius } => {
                let id = format!("fixed-circle-{obstacle_index}");
                let center = engine_vec2(*center, &id)?;
                polylines.push(CopperPolylineInput {
                    id: id.clone(),
                    points: vec![center, center],
                    width: engine_f32(*radius * 2.0, &format!("{id} diameter"))?,
                    tension_weight: 0.0,
                    mobility: CopperPolylineMobility::Fixed,
                });
                for selected_id in &active_selected_ids {
                    separations.push(CopperSeparationPair {
                        first: (*selected_id).clone(),
                        second: id.clone(),
                        clearance,
                    });
                }
                fixed_capsule_obstacles += 1;
            }
            ObstacleGeometry::Segment { start, end, radius } => {
                let id = format!("fixed-capsule-{obstacle_index}");
                polylines.push(CopperPolylineInput {
                    id: id.clone(),
                    points: vec![engine_vec2(*start, &id)?, engine_vec2(*end, &id)?],
                    width: engine_f32(*radius * 2.0, &format!("{id} width"))?,
                    tension_weight: 0.0,
                    mobility: CopperPolylineMobility::Fixed,
                });
                for selected_id in &active_selected_ids {
                    separations.push(CopperSeparationPair {
                        first: (*selected_id).clone(),
                        second: id.clone(),
                        clearance,
                    });
                }
                fixed_capsule_obstacles += 1;
            }
            ObstacleGeometry::Rectangle {
                center,
                half_size,
                angle_degrees,
            } => {
                let id = format!("fixed-rectangle-{obstacle_index}");
                bodies.push(CopperBodyInput {
                    id: id.clone(),
                    position: engine_vec2(*center, &id)?,
                    angle_radians: engine_f32(angle_degrees.to_radians(), &format!("{id} angle"))?,
                    size: engine_vec2(
                        [half_size[0] * 2.0, half_size[1] * 2.0],
                        &format!("{id} size"),
                    )?,
                    mobility: Mobility::FIXED,
                });
                for selected_id in &active_selected_ids {
                    body_separations.push(CopperSegmentBodyPair {
                        polyline: (*selected_id).clone(),
                        body: id.clone(),
                        clearance,
                    });
                }
                fixed_rectangular_obstacles += 1;
            }
            ObstacleGeometry::Polygon { .. } | ObstacleGeometry::Union { .. } => {
                // Continuous repair presently supports analytic circles,
                // capsules, and rectangles. A bounding box is conservative:
                // it can reject motion but cannot admit copper through a
                // custom pad that the exact native gate would later reject.
                let bounds = obstacle.geometry.aabb();
                let id = format!("fixed-compound-{obstacle_index}");
                let center = [
                    (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                    (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
                ];
                let size = [
                    bounds.maximum[0] - bounds.minimum[0],
                    bounds.maximum[1] - bounds.minimum[1],
                ];
                bodies.push(CopperBodyInput {
                    id: id.clone(),
                    position: engine_vec2(center, &id)?,
                    angle_radians: 0.0,
                    size: engine_vec2(size, &format!("{id} size"))?,
                    mobility: Mobility::FIXED,
                });
                for selected_id in &active_selected_ids {
                    body_separations.push(CopperSegmentBodyPair {
                        polyline: (*selected_id).clone(),
                        body: id.clone(),
                        clearance,
                    });
                }
                fixed_rectangular_obstacles += 1;
            }
        }
    }

    let request = CopperRepairRequest {
        bounds: Bounds::new(
            engine_vec2(
                [engine_bounds[0], engine_bounds[1]],
                "engine bounds minimum",
            )?,
            engine_vec2(
                [engine_bounds[2], engine_bounds[3]],
                "engine bounds maximum",
            )?,
        ),
        bodies,
        polylines,
        shared_points,
        attachments,
        separations,
        body_separations,
        body_body_separations,
        maximum_segment_pairs: relaxation_config.maximum_constraint_pairs,
    };
    let mut compiled = compile_copper_repair(&request)?;
    let compiled_segment_pairs = compiled.segment_pairs;
    let compiled_segment_body_pairs = compiled.segment_body_pairs;
    let compiled_body_body_pairs = compiled.body_body_pairs;
    let engine_particles = compiled.world.particles.len();
    let shared_point_groups = compiled.shared_point_groups;
    let shared_point_members = compiled.shared_point_members;
    let mixed_mobility_shared_point_groups = compiled.mixed_mobility_shared_point_groups;
    let tension_edges = compiled.tension_edges;
    let deduplicated_tension_edges = compiled.deduplicated_tension_edges;
    let deduplicated_segment_pairs = compiled.deduplicated_segment_pairs;
    let deduplicated_segment_body_pairs = compiled.deduplicated_segment_body_pairs;
    let deduplicated_body_body_pairs = compiled.deduplicated_body_body_pairs;
    let terminal_attachments = request.attachments.len();
    let mut backend = CpuReferenceBackend::new();
    let backend_name = backend.name().to_string();
    let solver = SolverConfig {
        timestep: engine_f32(relaxation_config.timestep, "relaxation timestep")?,
        projection_iterations: relaxation_config.projection_iterations,
        field_width: 8,
        field_height: 8,
        field_smoothing_steps: 0,
        target_density: 0.0,
        field_strength: 0.0,
        maximum_field_step: 0.2,
        trace_tension_strength: engine_f32(
            relaxation_config.trace_tension_strength,
            "trace tension strength",
        )?,
        maximum_trace_tension_step: engine_f32(
            relaxation_config.maximum_trace_tension_step_mm,
            "maximum trace tension step",
        )?,
    };
    let mut work = KiCadRouteRelaxationWork {
        compiled_segment_pairs,
        compiled_segment_body_pairs,
        compiled_body_body_pairs,
        fixed_capsule_obstacles,
        fixed_rectangular_obstacles,
        layer_relevant_obstacle_pairs: obstacle_selection.layer_relevant_pairs,
        broad_phase_candidate_pairs: obstacle_selection.candidate_pairs,
        retained_obstacle_pairs: obstacle_selection.retained_pairs,
        culled_obstacle_pairs: obstacle_selection
            .layer_relevant_pairs
            .saturating_sub(obstacle_selection.retained_pairs),
        broad_phase_cells: obstacle_selection.broad_phase_cells,
        broad_phase_cell_references: obstacle_selection.broad_phase_cell_references,
        broad_phase_cell_queries: obstacle_selection.broad_phase_cell_queries,
        engine_particles,
        selected_layer_runs: layer_runs.len(),
        fixed_via_anchors,
        fixed_via_anchor_members,
        fixed_context_anchors,
        fixed_context_anchor_members,
        shared_point_groups,
        shared_point_members,
        mixed_mobility_shared_point_groups,
        route_graph_contact_points: route_graph_normalization.contact_points,
        route_graph_inserted_points: route_graph_normalization.inserted_points,
        tension_edges,
        deduplicated_tension_edges,
        deduplicated_segment_pairs,
        deduplicated_segment_body_pairs,
        deduplicated_body_body_pairs,
        terminal_attachment_bodies,
        terminal_attachments,
        placement_clearance_bodies,
        placement_clearance_pairs,
        ..KiCadRouteRelaxationWork::default()
    };
    for step in 0..relaxation_config.steps {
        let frame = backend.step(&mut compiled.world, &solver, step as u64);
        work.frames += 1;
        work.constraint_projections += frame.metrics.constraint_projections;
        work.scalar_rows += frame.metrics.constraint_scalar_rows;
        work.trace_tension_edge_integrations += frame.metrics.trace_tension_edges;
        work.maximum_frame_displacement_mm = work
            .maximum_frame_displacement_mm
            .max(f64::from(frame.metrics.max_displacement));
        work.maximum_constraint_residual_mm = work
            .maximum_constraint_residual_mm
            .max(f64::from(frame.metrics.max_constraint_residual));
        work.final_constraint_residual_mm = f64::from(frame.metrics.max_constraint_residual);
    }

    let solved: BTreeMap<_, _> = compiled.polylines().into_iter().collect();
    let solved_bodies = compiled
        .body_poses()
        .into_iter()
        .map(|pose| (pose.id.clone(), pose))
        .collect::<BTreeMap<_, _>>();
    let mut proposal_candidate = engine_candidate.clone();
    let mut maximum_point_motion_mm = 0.0_f64;
    for run in &layer_runs {
        let id = &run.id;
        let solved_points = solved
            .get(id)
            .ok_or_else(|| format!("engine omitted selected polyline {id}"))?;
        if solved_points.len() != run.len() {
            return Err(format!(
                "engine changed point count for selected branch {} layer run",
                run.branch_index
            ));
        }
        for (local_point, solved_point) in solved_points.iter().enumerate() {
            let point = run.start_point + local_point;
            if exact_fixed_points.contains(&(run.branch_index, point)) {
                continue;
            }
            let original = &engine_candidate.branches[run.branch_index].path[point];
            if solved_point.x.to_bits() == (original.at[0] as f32).to_bits()
                && solved_point.y.to_bits() == (original.at[1] as f32).to_bits()
            {
                continue;
            }
            let at = [f64::from(solved_point.x), f64::from(solved_point.y)];
            maximum_point_motion_mm =
                maximum_point_motion_mm.max(distance_squared(original.at, at).sqrt());
            proposal_candidate.branches[run.branch_index].path[point].at = at;
        }
    }
    for &index in &selected {
        let original = &engine_candidate.branches[index];
        if original
            .path
            .first()
            .and_then(|point| terminal_pad_at(&model, point.at))
            .is_some_and(|terminal| movable_footprints.contains_key(&terminal.footprint))
        {
            proposal_candidate.branches[index].start_terminal =
                proposal_candidate.branches[index].path[0].at;
        }
        if original
            .path
            .last()
            .and_then(|point| terminal_pad_at(&model, point.at))
            .is_some_and(|terminal| movable_footprints.contains_key(&terminal.footprint))
        {
            proposal_candidate.branches[index].finish_terminal = proposal_candidate.branches[index]
                .path
                .last()
                .expect("validated branch path")
                .at;
        }
    }
    proposal_candidate.terminals = proposal_candidate
        .branches
        .iter()
        .flat_map(|branch| [branch.start_terminal, branch.finish_terminal])
        .collect();
    proposal_candidate.terminals.sort_by(|left, right| {
        left[0]
            .total_cmp(&right[0])
            .then_with(|| left[1].total_cmp(&right[1]))
    });
    proposal_candidate.terminals.dedup();

    let mut footprint_motions = Vec::new();
    let mut placement_limits_respected = true;
    let mut proposed_placements = candidate.footprint_placements.clone();
    for (reference, config) in &movable_footprints {
        let info = &footprint_infos[reference];
        let body_id = terminal_body_ids.get(reference).ok_or_else(|| {
            format!("movable footprint {reference} is not attached to a selected branch")
        })?;
        let pose = &solved_bodies[body_id];
        let engine_angle_degrees = f64::from(pose.angle_radians).to_degrees();
        let body_center_offset = rotate_vector(info.body_center_local, engine_angle_degrees);
        let proposal_at = [
            f64::from(pose.position.x) - body_center_offset[0],
            f64::from(pose.position.y) - body_center_offset[1],
            -engine_angle_degrees,
        ];
        let translation_mm =
            distance_squared([info.at[0], info.at[1]], [proposal_at[0], proposal_at[1]]).sqrt();
        let rotation_degrees = angle_delta_degrees(info.at[2], proposal_at[2]).abs();
        let within_configured_limits = translation_mm <= config.maximum_translation_mm + 1.0e-6
            && rotation_degrees <= config.maximum_rotation_degrees + 1.0e-6;
        placement_limits_respected &= within_configured_limits;
        footprint_motions.push(KiCadFootprintMotionEvidence {
            reference: reference.clone(),
            source_at: info.at,
            proposal_at,
            translation_mm,
            rotation_degrees,
            within_configured_limits,
        });
        if let Some(existing) = proposed_placements
            .iter_mut()
            .find(|placement| placement.reference == *reference)
        {
            existing.at = proposal_at;
        } else {
            proposed_placements.push(KiCadFootprintPlacement {
                reference: reference.clone(),
                source_at: info.at,
                at: proposal_at,
            });
        }
    }
    proposed_placements.sort_by(|left, right| left.reference.cmp(&right.reference));
    proposal_candidate.footprint_placements = proposed_placements;
    let (segments, vias) = materialize_candidate_branches(
        &proposal_candidate.connection,
        &proposal_candidate.branches,
        &proposal_candidate.config,
    )?;
    proposal_candidate.supplemental_segments = segments;
    proposal_candidate.supplemental_vias = vias;
    let proposed_branches = selected
        .iter()
        .map(|&branch_index| KiCadRelaxedBranchEvidence {
            branch_index,
            before: candidate.branches[branch_index].path.clone(),
            proposal: proposal_candidate.branches[branch_index].path.clone(),
        })
        .collect();
    let mut proposal_pcb = parse(&pcb_source)?;
    apply_footprint_placements(&mut proposal_pcb, &proposal_candidate.footprint_placements)?;
    apply_reference_placements(&mut proposal_pcb, &proposal_candidate.reference_placements)?;
    let proposal_model = KiCadRoutingModel::from_pcb_with_config(
        &proposal_pcb,
        &candidate.connection,
        &proposal_candidate.config,
    )?;
    let proposal_blockers =
        selected_geometry_blockers(&proposal_candidate, &selected, &proposal_model);
    let proposal_geometry_clear = proposal_blockers.is_empty();
    let proposal = route_candidate_quality(&proposal_candidate)?;
    let returned_proposal = proposal_candidate.clone();
    let broad_phase_envelope_respected = relaxation_config.broad_phase
        == KiCadRouteBroadPhase::Exhaustive
        || maximum_point_motion_mm <= relaxation_config.broad_phase_motion_margin_mm + 1.0e-6;
    let improves =
        proposal.length_mm + relaxation_config.minimum_length_improvement_mm <= before.length_mm;
    let (candidate, status, detail, after) = if !placement_limits_respected {
        (
            candidate,
            KiCadRouteRelaxationStatus::InvalidProposal,
            "continuous proposal exceeded a configured footprint motion limit".into(),
            before.clone(),
        )
    } else if !broad_phase_envelope_respected {
        (
            candidate,
            KiCadRouteRelaxationStatus::InvalidProposal,
            format!(
                "continuous proposal exceeded the configured {:.6} mm broad-phase motion envelope",
                relaxation_config.broad_phase_motion_margin_mm
            ),
            before.clone(),
        )
    } else if !proposal_geometry_clear {
        (
            candidate,
            KiCadRouteRelaxationStatus::InvalidProposal,
            "continuous proposal failed the independent analytic KiCad obstacle check".into(),
            before.clone(),
        )
    } else if !improves {
        (
            candidate,
            KiCadRouteRelaxationStatus::NoImprovement,
            format!(
                "proposal did not improve length by the configured {:.6} mm",
                relaxation_config.minimum_length_improvement_mm
            ),
            before.clone(),
        )
    } else {
        proposal_candidate.router = algorithm.into();
        (
            proposal_candidate,
            KiCadRouteRelaxationStatus::Accepted,
            "proposal passed the analytic obstacle gate and improved route length".into(),
            proposal.clone(),
        )
    };
    let evidence = KiCadRouteRelaxationEvidence {
        schema_version: 1,
        connection: candidate.connection.clone(),
        algorithm: algorithm.into(),
        source_router,
        backend: backend_name,
        status,
        detail,
        branch_selection: relaxation_config.branch_selection,
        seed_branches: relaxation_config.branch_indices.clone(),
        selected_branches: selected,
        branch_neighborhood_hops: relaxation_config.branch_neighborhood_hops,
        maximum_selected_branches: relaxation_config.maximum_selected_branches,
        maximum_branch_contact_tests: relaxation_config.maximum_branch_contact_tests,
        branch_contact_tests,
        broad_phase: relaxation_config.broad_phase,
        shared_copper: relaxation_config.shared_copper,
        via_mobility: relaxation_config.via_mobility,
        placement_clearance: relaxation_config.placement_clearance,
        broad_phase_cell_size_mm: relaxation_config.broad_phase_cell_size_mm,
        broad_phase_motion_margin_mm: relaxation_config.broad_phase_motion_margin_mm,
        broad_phase_envelope_respected,
        steps: relaxation_config.steps,
        projection_iterations: relaxation_config.projection_iterations,
        trace_tension_strength: relaxation_config.trace_tension_strength,
        maximum_trace_tension_step_mm: relaxation_config.maximum_trace_tension_step_mm,
        constraint_guard_mm: relaxation_config.constraint_guard_mm,
        minimum_length_improvement_mm: relaxation_config.minimum_length_improvement_mm,
        maximum_point_motion_mm,
        footprint_motions,
        initial_geometry_clear,
        proposal_geometry_clear,
        proposal_blockers,
        proposed_branches,
        before,
        proposal: Some(proposal),
        after,
        work,
        exact_kicad_validation_required: true,
    };
    Ok((candidate, Some(returned_proposal), evidence))
}

fn unsupported_route_relaxation(
    candidate: KiCadRouteCandidate,
    before: KiCadRouteQuality,
    source_router: String,
    config: &KiCadRouteRelaxationConfig,
    selected_branches: Vec<usize>,
    branch_contact_tests: usize,
    detail: String,
) -> (
    KiCadRouteCandidate,
    Option<KiCadRouteCandidate>,
    KiCadRouteRelaxationEvidence,
) {
    let evidence = KiCadRouteRelaxationEvidence {
        schema_version: 1,
        connection: candidate.connection.clone(),
        algorithm: route_relaxation_algorithm(config).into(),
        source_router,
        backend: "not-run".into(),
        status: KiCadRouteRelaxationStatus::Unsupported,
        detail,
        branch_selection: config.branch_selection,
        seed_branches: config.branch_indices.clone(),
        selected_branches,
        branch_neighborhood_hops: config.branch_neighborhood_hops,
        maximum_selected_branches: config.maximum_selected_branches,
        maximum_branch_contact_tests: config.maximum_branch_contact_tests,
        branch_contact_tests,
        broad_phase: config.broad_phase,
        shared_copper: config.shared_copper,
        via_mobility: config.via_mobility,
        placement_clearance: config.placement_clearance,
        broad_phase_cell_size_mm: config.broad_phase_cell_size_mm,
        broad_phase_motion_margin_mm: config.broad_phase_motion_margin_mm,
        broad_phase_envelope_respected: false,
        steps: config.steps,
        projection_iterations: config.projection_iterations,
        trace_tension_strength: config.trace_tension_strength,
        maximum_trace_tension_step_mm: config.maximum_trace_tension_step_mm,
        constraint_guard_mm: config.constraint_guard_mm,
        minimum_length_improvement_mm: config.minimum_length_improvement_mm,
        maximum_point_motion_mm: 0.0,
        footprint_motions: Vec::new(),
        initial_geometry_clear: false,
        proposal_geometry_clear: false,
        proposal_blockers: Vec::new(),
        proposed_branches: Vec::new(),
        before: before.clone(),
        proposal: None,
        after: before,
        work: KiCadRouteRelaxationWork::default(),
        exact_kicad_validation_required: true,
    };
    (candidate, None, evidence)
}

fn route_relaxation_algorithm(config: &KiCadRouteRelaxationConfig) -> &'static str {
    match config.via_mobility {
        KiCadViaMobilityPolicy::Reject => "projected-trace-tension-v1",
        KiCadViaMobilityPolicy::Fixed => "projected-trace-tension-fixed-via-v2",
    }
}

fn validate_route_relaxation_config(config: &KiCadRouteRelaxationConfig) -> Result<(), String> {
    if config.branch_indices.is_empty() {
        return Err("KiCad route relaxation requires at least one branch index".into());
    }
    let mut branch_indices = config.branch_indices.clone();
    branch_indices.sort_unstable();
    if branch_indices.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("KiCad route relaxation branch_indices must be unique".into());
    }
    if config.branch_indices.len() > config.maximum_selected_branches {
        return Err("KiCad route relaxation seeds exceed maximum_selected_branches".into());
    }
    if config.branch_selection == KiCadBranchSelectionPolicy::NearestSharedJunction
        && config.shared_copper != KiCadSharedCopperPolicy::SharedRouteGraph
    {
        return Err(
            "nearest_shared_junction branch selection requires shared_copper=shared_route_graph"
                .into(),
        );
    }
    if !(1..=10_000).contains(&config.steps)
        || !(1..=1_000).contains(&config.projection_iterations)
        || !(1..=64).contains(&config.branch_neighborhood_hops)
        || config.maximum_selected_branches == 0
        || config.maximum_branch_contact_tests == 0
        || config.maximum_constraint_pairs == 0
        || !config.timestep.is_finite()
        || !(1.0e-6..=1.0).contains(&config.timestep)
        || !config.trace_tension_strength.is_finite()
        || !(0.0..=100.0).contains(&config.trace_tension_strength)
        || !config.maximum_trace_tension_step_mm.is_finite()
        || !(1.0e-6..=10.0).contains(&config.maximum_trace_tension_step_mm)
        || !config.constraint_guard_mm.is_finite()
        || !(0.0..=1.0).contains(&config.constraint_guard_mm)
        || !config.broad_phase_cell_size_mm.is_finite()
        || !(0.01..=1_000.0).contains(&config.broad_phase_cell_size_mm)
        || !config.broad_phase_motion_margin_mm.is_finite()
        || !(0.0..=1_000.0).contains(&config.broad_phase_motion_margin_mm)
        || (config.broad_phase == KiCadRouteBroadPhase::UniformGrid
            && config.broad_phase_motion_margin_mm == 0.0)
        || !config.minimum_length_improvement_mm.is_finite()
        || config.minimum_length_improvement_mm < 0.0
    {
        return Err("invalid KiCad route relaxation configuration".into());
    }
    Ok(())
}

fn validate_movable_footprint_config(config: &KiCadMovableFootprintConfig) -> Result<(), String> {
    if config.reference.is_empty()
        || (!config.translate_x && !config.translate_y && !config.rotate)
        || !config.inverse_mass.is_finite()
        || config.inverse_mass < 0.0
        || !config.rotation_mobility.is_finite()
        || config.rotation_mobility < 0.0
        || !config.maximum_translation_mm.is_finite()
        || config.maximum_translation_mm < 0.0
        || !config.maximum_rotation_degrees.is_finite()
        || !(0.0..=180.0).contains(&config.maximum_rotation_degrees)
    {
        return Err(format!(
            "invalid movable-footprint configuration for {:?}",
            config.reference
        ));
    }
    Ok(())
}

fn terminal_pad_at(model: &KiCadRoutingModel, at: [f64; 2]) -> Option<&KiCadTerminalPad> {
    model
        .terminal_pads
        .iter()
        .find(|terminal| distance_squared(terminal.at, at) <= 1.0e-12)
}

fn terminal_copper_layers_at(model: &KiCadRoutingModel, at: [f64; 2]) -> Option<[bool; 2]> {
    let mut found = false;
    let mut layers = [false, false];
    for terminal in &model.terminal_pads {
        if distance_squared(terminal.at, at) <= 1.0e-12 {
            found = true;
            layers[0] |= terminal.layers[0];
            layers[1] |= terminal.layers[1];
        }
    }
    found.then_some(layers)
}

fn terminal_copper_geometry_at(
    model: &KiCadRoutingModel,
    at: [f64; 2],
    layer: usize,
) -> Option<ObstacleGeometry> {
    let mut parts = model
        .terminal_pads
        .iter()
        .filter(|terminal| terminal.layers[layer] && distance_squared(terminal.at, at) <= 1.0e-12)
        .map(|terminal| terminal.geometry.clone())
        .collect::<Vec<_>>();
    match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(ObstacleGeometry::Union { parts }),
    }
}

fn angle_delta_degrees(from: f64, to: f64) -> f64 {
    (to - from + 180.0).rem_euclid(360.0) - 180.0
}

fn engine_f32(value: f64, label: &str) -> Result<f32, String> {
    let converted = value as f32;
    if !value.is_finite() || !converted.is_finite() {
        return Err(format!("{label} cannot be represented by the engine"));
    }
    Ok(converted)
}

fn engine_vec2(value: [f64; 2], label: &str) -> Result<Vec2, String> {
    Ok(Vec2::new(
        engine_f32(value[0], &format!("{label} x"))?,
        engine_f32(value[1], &format!("{label} y"))?,
    ))
}

fn rooted_star_terminal(terminals: &[GridPosition]) -> usize {
    (0..terminals.len())
        .min_by_key(|candidate| {
            (
                terminals
                    .iter()
                    .copied()
                    .map(|terminal| grid_distance(terminals[*candidate], terminal))
                    .sum::<usize>(),
                *candidate,
            )
        })
        .expect("routing model has at least two terminals")
}

fn grid_distance(left: GridPosition, right: GridPosition) -> usize {
    left.x.abs_diff(right.x).max(left.y.abs_diff(right.y))
}

fn insert_materialized_grid_path_contacts(
    tree: &mut GridTree,
    grid_path: &[GridPosition],
    materialized_path: &[KiCadRoutePoint],
    origin: [f64; 2],
    resolution_mm: f64,
) {
    for (index, position) in grid_path.iter().copied().enumerate() {
        let at = grid_position_at(position, origin, resolution_mm);
        let layer = copper_layer_name(position.layer);
        // A moved endpoint can still be a retained via or lie on its anchor
        // connector. Check actual same-layer copper before discarding it.
        let endpoint = index == 0 || index + 1 == grid_path.len();
        let represented = !endpoint || materialized_path.iter().any(|point| {
            point.layer == layer && distance_squared(at, point.at) <= 1.0e-12
        }) || materialized_path.windows(2).any(|pair| {
            pair[0].layer == layer && pair[1].layer == layer
                && route_point_on_segment(at, pair[0].at, pair[1].at)
        });
        if represented {
            tree.insert(grid_position_key(position), position);
        }
    }
}

fn supplemental_segment_key(segment: &SupplementalSegment) -> String {
    let (start, end) = if segment.start[0].total_cmp(&segment.end[0]).is_lt()
        || (segment.start[0] == segment.end[0]
            && segment.start[1].total_cmp(&segment.end[1]).is_le())
    {
        (segment.start, segment.end)
    } else {
        (segment.end, segment.start)
    };
    format!(
        "{}:{:.9}:{:.9}:{:.9}:{:.9}:{:.9}",
        segment.layer, start[0], start[1], end[0], end[1], segment.width
    )
}

fn supplemental_via_key(via: &SupplementalVia) -> String {
    format!(
        "{:.9}:{:.9}:{:.9}:{:.9}:{}:{}",
        via.at[0], via.at[1], via.size, via.drill, via.layers[0], via.layers[1]
    )
}

fn validate_grid_route_config(config: &KiCadGridRouteConfig) -> Result<(), String> {
    connection_rules::validate(config)?;
    if let Some(demand) = &config.routing_demand {
        routing_demand::check(demand)?;
    }
    if let Some(mm) = config.via_cost_mm {
        let units = mm * f64::from(config.straight_cost) / config.resolution_mm;
        if !mm.is_finite() || mm < 0.0 || !units.is_finite() || units.ceil() > f64::from(u32::MAX) {
            return Err(
                "via_cost_mm must be nonnegative and representable at this grid resolution".into(),
            );
        }
    }
    let dimensions = [
        config.resolution_mm,
        config.trace_width_mm,
        config.clearance_mm,
        config.edge_clearance_mm,
        config.hole_to_hole_clearance_mm,
        config.via_size_mm,
        config.via_drill_mm,
        config.tree_attachment_minimum_spacing_mm,
    ];
    if dimensions
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
        || config.resolution_mm == 0.0
        || config.trace_width_mm == 0.0
        || config.via_size_mm == 0.0
        || config.via_drill_mm == 0.0
        || config.via_drill_mm >= config.via_size_mm
        || config.straight_cost == 0
        || config.diagonal_cost == 0
        || config.max_expansions == 0
        || config.maximum_tree_attachment_searches == 0
    {
        return Err("invalid KiCad grid-routing configuration".into());
    }
    if config.multi_terminal_routing == KiCadMultiTerminalRoutingPolicy::RootedStar
        && (config.tree_attachment_search != KiCadTreeAttachmentSearch::RankedSamples
            || config.maximum_tree_attachment_searches != 1
            || config.tree_attachment_minimum_spacing_mm != 0.0
            || config.tree_attachment_objective != KiCadTreeAttachmentObjective::LengthThenVias)
    {
        return Err(
            "tree-attachment controls require shared_copper_tree multi-terminal routing".into(),
        );
    }
    if config.tree_attachment_search == KiCadTreeAttachmentSearch::MultiSource
        && (config.tree_attachment_objective != KiCadTreeAttachmentObjective::RouterCost
            || config.maximum_tree_attachment_searches != 1
            || config.tree_attachment_minimum_spacing_mm != 0.0)
    {
        return Err("multi_source tree attachment requires router_cost, one search and zero sampling spacing".into());
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct BoardOutline {
    points: Vec<[f64; 2]>,
    bounds: [f64; 4],
}

impl BoardOutline {
    fn rectangle(bounds: [f64; 4]) -> Result<Self, String> {
        Self::new(vec![
            [bounds[0], bounds[1]],
            [bounds[2], bounds[1]],
            [bounds[2], bounds[3]],
            [bounds[0], bounds[3]],
        ])
    }

    fn new(points: Vec<[f64; 2]>) -> Result<Self, String> {
        if points.len() < 3
            || points
                .iter()
                .flatten()
                .any(|coordinate| !coordinate.is_finite())
        {
            return Err("board outline has invalid polygon coordinates".into());
        }
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for point in &points {
            bounds[0] = bounds[0].min(point[0]);
            bounds[1] = bounds[1].min(point[1]);
            bounds[2] = bounds[2].max(point[0]);
            bounds[3] = bounds[3].max(point[1]);
        }
        let outline = Self { points, bounds };
        if outline.area() <= 1.0e-12 {
            return Err("board outline polygon has zero area".into());
        }
        Ok(outline)
    }

    fn area(&self) -> f64 {
        self.points
            .iter()
            .zip(self.points.iter().cycle().skip(1))
            .take(self.points.len())
            .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
            .sum::<f64>()
            .abs()
            / 2.0
    }

    fn contains_point_with_clearance(&self, point: [f64; 2], clearance: f64) -> bool {
        point_in_polygon(point, &self.points)
            && self.edges().all(|(start, end)| {
                point_segment_distance_squared(point, start, end) + 1.0e-18 >= clearance * clearance
            })
    }

    fn contains_segment_with_clearance(
        &self,
        start: [f64; 2],
        end: [f64; 2],
        clearance: f64,
    ) -> bool {
        self.contains_point_with_clearance(start, clearance)
            && self.contains_point_with_clearance(end, clearance)
            && self.segment_clears_boundary(start, end, clearance)
    }

    fn segment_clears_boundary(&self, start: [f64; 2], end: [f64; 2], clearance: f64) -> bool {
        self.edges().all(|(edge_start, edge_end)| {
            segment_segment_distance_squared(start, end, edge_start, edge_end) + 1.0e-18
                >= clearance * clearance
        })
    }

    fn edges(&self) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
        self.points
            .iter()
            .copied()
            .zip(self.points.iter().copied().cycle().skip(1))
            .take(self.points.len())
    }
}

#[derive(Clone, Debug)]
struct KiCadRoutingModel {
    bounds: [f64; 4],
    outline: BoardOutline,
    terminals: Vec<[f64; 2]>,
    terminal_pads: Vec<KiCadTerminalPad>,
    obstacles: Vec<CopperObstacle>,
    target_holes: Vec<ObstacleGeometry>,
}

#[derive(Clone, Debug)]
struct KiCadTerminalPad {
    at: [f64; 2],
    footprint: String,
    local_position: [f64; 2],
    layers: [bool; 2],
    plated_through_hole: bool,
    geometry: ObstacleGeometry,
}

#[derive(Clone, Debug)]
struct KiCadFootprintInfo {
    reference: String,
    at: [f64; 3],
    body_center_local: [f64; 2],
    body_size: [f64; 2],
    router_movable: bool,
}

#[derive(Clone, Debug)]
struct CopperObstacle {
    source_uuid: Option<String>,
    layers: [bool; 2],
    // Native rule areas constrain these independently. Ordinary copper blocks
    // both; a via-only keepout must not become a wall for planar routing.
    blocks_tracks: bool,
    blocks_vias: bool,
    local_clearance_mm: f64,
    geometry: ObstacleGeometry,
    kind: KiCadViaLocalRerouteBlockerKind,
    object: String,
    net: Option<String>,
    footprint: Option<String>,
    component_movable: bool,
}

#[derive(Clone, Copy)]
enum CopperQueryKind {
    Trace,
    Via,
}

impl CopperObstacle {
    fn blocks(&self, kind: CopperQueryKind) -> bool {
        match kind {
            CopperQueryKind::Trace => self.blocks_tracks,
            CopperQueryKind::Via => self.blocks_vias,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ObstacleGeometry {
    Circle {
        center: [f64; 2],
        radius: f64,
    },
    Segment {
        start: [f64; 2],
        end: [f64; 2],
        radius: f64,
    },
    Rectangle {
        center: [f64; 2],
        half_size: [f64; 2],
        angle_degrees: f64,
    },
    Polygon {
        points: Vec<[f64; 2]>,
    },
    Union {
        parts: Vec<ObstacleGeometry>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GeometryAabb {
    minimum: [f64; 2],
    maximum: [f64; 2],
}

impl GeometryAabb {
    fn around_segment(start: [f64; 2], end: [f64; 2], inflate: f64) -> Self {
        Self {
            minimum: [
                start[0].min(end[0]) - inflate,
                start[1].min(end[1]) - inflate,
            ],
            maximum: [
                start[0].max(end[0]) + inflate,
                start[1].max(end[1]) + inflate,
            ],
        }
    }

    fn intersects(self, other: Self) -> bool {
        self.minimum[0] <= other.maximum[0]
            && self.maximum[0] >= other.minimum[0]
            && self.minimum[1] <= other.maximum[1]
            && self.maximum[1] >= other.minimum[1]
    }
}

#[derive(Debug)]
struct KiCadObstacleGrid {
    origin: [f64; 2],
    cell_size_mm: f64,
    clearance_mm: f64,
    cells: BTreeMap<(usize, i32, i32), Vec<usize>>,
    cell_references: usize,
}

impl KiCadObstacleGrid {
    fn new(model: &KiCadRoutingModel, cell_size_mm: f64, clearance_mm: f64) -> Self {
        let mut cells = BTreeMap::<(usize, i32, i32), Vec<usize>>::new();
        let mut cell_references = 0;
        for (obstacle_index, obstacle) in model.obstacles.iter().enumerate() {
            let bounds = obstacle.query_bounds(clearance_mm);
            let ([minimum_x, minimum_y], [maximum_x, maximum_y]) =
                cell_range(bounds, model.bounds, cell_size_mm);
            for layer in 0..2 {
                if !obstacle.layers[layer] {
                    continue;
                }
                for y in minimum_y..=maximum_y {
                    for x in minimum_x..=maximum_x {
                        cells.entry((layer, x, y)).or_default().push(obstacle_index);
                        cell_references += 1;
                    }
                }
            }
        }
        Self {
            origin: [model.bounds[0], model.bounds[1]],
            cell_size_mm,
            clearance_mm,
            cells,
            cell_references,
        }
    }

    fn query(&self, layer: usize, bounds: GeometryAabb) -> (BTreeSet<usize>, usize) {
        let minimum_x = ((bounds.minimum[0] - self.origin[0]) / self.cell_size_mm).floor() as i32;
        let minimum_y = ((bounds.minimum[1] - self.origin[1]) / self.cell_size_mm).floor() as i32;
        let maximum_x = ((bounds.maximum[0] - self.origin[0]) / self.cell_size_mm).floor() as i32;
        let maximum_y = ((bounds.maximum[1] - self.origin[1]) / self.cell_size_mm).floor() as i32;
        let mut candidates = BTreeSet::new();
        let mut cell_queries = 0;
        for y in minimum_y..=maximum_y {
            for x in minimum_x..=maximum_x {
                cell_queries += 1;
                if let Some(indices) = self.cells.get(&(layer, x, y)) {
                    candidates.extend(indices.iter().copied());
                }
            }
        }
        (candidates, cell_queries)
    }

    fn any_contains(
        &self,
        model: &KiCadRoutingModel,
        layer: usize,
        point: [f64; 2],
        inflate: f64,
        kind: CopperQueryKind,
    ) -> bool {
        let bounds = GeometryAabb {
            minimum: [point[0] - inflate, point[1] - inflate],
            maximum: [point[0] + inflate, point[1] + inflate],
        };
        let minimum_x = ((bounds.minimum[0] - self.origin[0]) / self.cell_size_mm).floor() as i32;
        let minimum_y = ((bounds.minimum[1] - self.origin[1]) / self.cell_size_mm).floor() as i32;
        let maximum_x = ((bounds.maximum[0] - self.origin[0]) / self.cell_size_mm).floor() as i32;
        let maximum_y = ((bounds.maximum[1] - self.origin[1]) / self.cell_size_mm).floor() as i32;
        for y in minimum_y..=maximum_y {
            for x in minimum_x..=maximum_x {
                let Some(indices) = self.cells.get(&(layer, x, y)) else {
                    continue;
                };
                if indices.iter().any(|index| {
                    model.obstacles[*index].blocks(kind)
                        && model.obstacles[*index].geometry.contains(
                            point,
                            model.obstacles[*index].inflate(inflate, self.clearance_mm),
                        )
                }) {
                    return true;
                }
            }
        }
        false
    }

    fn any_intersects_segment(
        &self,
        model: &KiCadRoutingModel,
        layer: usize,
        start: [f64; 2],
        end: [f64; 2],
        inflate: f64,
    ) -> bool {
        let bounds = GeometryAabb::around_segment(start, end, inflate);
        let minimum_x = ((bounds.minimum[0] - self.origin[0]) / self.cell_size_mm).floor() as i32;
        let minimum_y = ((bounds.minimum[1] - self.origin[1]) / self.cell_size_mm).floor() as i32;
        let maximum_x = ((bounds.maximum[0] - self.origin[0]) / self.cell_size_mm).floor() as i32;
        let maximum_y = ((bounds.maximum[1] - self.origin[1]) / self.cell_size_mm).floor() as i32;
        for y in minimum_y..=maximum_y {
            for x in minimum_x..=maximum_x {
                let Some(indices) = self.cells.get(&(layer, x, y)) else {
                    continue;
                };
                if indices.iter().any(|index| {
                    model.obstacles[*index].blocks_tracks
                        && model.obstacles[*index].geometry.intersects_segment(
                            start,
                            end,
                            model.obstacles[*index].inflate(inflate, self.clearance_mm),
                        )
                }) {
                    return true;
                }
            }
        }
        false
    }
}

fn cell_range(
    bounds: GeometryAabb,
    board_bounds: [f64; 4],
    cell_size_mm: f64,
) -> ([i32; 2], [i32; 2]) {
    let cell = |coordinate: f64, origin: f64| ((coordinate - origin) / cell_size_mm).floor() as i32;
    (
        [
            cell(bounds.minimum[0], board_bounds[0]),
            cell(bounds.minimum[1], board_bounds[1]),
        ],
        [
            cell(bounds.maximum[0], board_bounds[0]),
            cell(bounds.maximum[1], board_bounds[1]),
        ],
    )
}

impl ObstacleGeometry {
    fn aabb(&self) -> GeometryAabb {
        match self {
            Self::Circle { center, radius } => GeometryAabb {
                minimum: [center[0] - radius, center[1] - radius],
                maximum: [center[0] + radius, center[1] + radius],
            },
            Self::Segment { start, end, radius } => {
                GeometryAabb::around_segment(*start, *end, *radius)
            }
            Self::Rectangle {
                center,
                half_size,
                angle_degrees,
            } => {
                let radians = angle_degrees.to_radians();
                let extent = [
                    radians.cos().abs() * half_size[0] + radians.sin().abs() * half_size[1],
                    radians.sin().abs() * half_size[0] + radians.cos().abs() * half_size[1],
                ];
                GeometryAabb {
                    minimum: [center[0] - extent[0], center[1] - extent[1]],
                    maximum: [center[0] + extent[0], center[1] + extent[1]],
                }
            }
            Self::Polygon { points } => polygon_aabb(points),
            Self::Union { parts } => parts
                .iter()
                .map(Self::aabb)
                .reduce(|left, right| GeometryAabb {
                    minimum: [
                        left.minimum[0].min(right.minimum[0]),
                        left.minimum[1].min(right.minimum[1]),
                    ],
                    maximum: [
                        left.maximum[0].max(right.maximum[0]),
                        left.maximum[1].max(right.maximum[1]),
                    ],
                })
                .expect("compound pad geometry has an anchor"),
        }
    }

    fn contains(&self, point: [f64; 2], inflate: f64) -> bool {
        match self {
            Self::Circle { center, radius } => {
                distance_squared(point, *center) < (radius + inflate).powi(2)
            }
            Self::Segment { start, end, radius } => {
                point_segment_distance_squared(point, *start, *end) < (radius + inflate).powi(2)
            }
            Self::Rectangle {
                center,
                half_size,
                angle_degrees,
            } => {
                let local = rotate_vector(
                    [point[0] - center[0], point[1] - center[1]],
                    -*angle_degrees,
                );
                point_intersects_rounded_rectangle(local, *half_size, inflate)
            }
            Self::Polygon { points } => point_intersects_inflated_polygon(point, points, inflate),
            Self::Union { parts } => parts.iter().any(|part| part.contains(point, inflate)),
        }
    }

    fn intersects_segment(&self, start: [f64; 2], end: [f64; 2], inflate: f64) -> bool {
        match self {
            Self::Circle { center, radius } => {
                point_segment_distance_squared(*center, start, end) < (radius + inflate).powi(2)
            }
            Self::Segment {
                start: obstacle_start,
                end: obstacle_end,
                radius,
            } => {
                segment_segment_distance_squared(start, end, *obstacle_start, *obstacle_end)
                    < (radius + inflate).powi(2)
            }
            Self::Rectangle {
                center,
                half_size,
                angle_degrees,
            } => {
                let local_start = rotate_vector(
                    [start[0] - center[0], start[1] - center[1]],
                    -*angle_degrees,
                );
                let local_end =
                    rotate_vector([end[0] - center[0], end[1] - center[1]], -*angle_degrees);
                segment_intersects_rounded_rectangle(local_start, local_end, *half_size, inflate)
            }
            Self::Polygon { points } => {
                segment_intersects_inflated_polygon(start, end, points, inflate)
            }
            Self::Union { parts } => parts
                .iter()
                .any(|part| part.intersects_segment(start, end, inflate)),
        }
    }
}

fn polygon_aabb(points: &[[f64; 2]]) -> GeometryAabb {
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    for point in points {
        for axis in 0..2 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
        }
    }
    GeometryAabb { minimum, maximum }
}

fn point_intersects_inflated_polygon(point: [f64; 2], polygon: &[[f64; 2]], inflate: f64) -> bool {
    if point_in_polygon(point, polygon) {
        return true;
    }
    polygon
        .windows(2)
        .any(|edge| point_segment_distance_squared(point, edge[0], edge[1]) < inflate.powi(2))
        || point_segment_distance_squared(point, polygon[polygon.len() - 1], polygon[0])
            < inflate.powi(2)
}

fn segment_intersects_inflated_polygon(
    start: [f64; 2],
    end: [f64; 2],
    polygon: &[[f64; 2]],
    inflate: f64,
) -> bool {
    point_in_polygon(start, polygon)
        || point_in_polygon(end, polygon)
        || polygon.windows(2).any(|edge| {
            segment_segment_distance_squared(start, end, edge[0], edge[1]) < inflate.powi(2)
        })
        || segment_segment_distance_squared(start, end, polygon[polygon.len() - 1], polygon[0])
            < inflate.powi(2)
}

fn point_in_polygon(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let first = polygon[current];
        let second = polygon[previous];
        if point_segment_distance_squared(point, first, second) <= 1.0e-18 {
            return true;
        }
        if (first[1] > point[1]) != (second[1] > point[1])
            && point[0]
                < (second[0] - first[0]) * (point[1] - first[1]) / (second[1] - first[1]) + first[0]
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn point_intersects_rounded_rectangle(point: [f64; 2], half_size: [f64; 2], inflate: f64) -> bool {
    let outside = [
        (point[0].abs() - half_size[0]).max(0.0),
        (point[1].abs() - half_size[1]).max(0.0),
    ];
    (outside[0] == 0.0 && outside[1] == 0.0)
        || outside[0] * outside[0] + outside[1] * outside[1] < inflate * inflate
}

fn segment_intersects_rounded_rectangle(
    start: [f64; 2],
    end: [f64; 2],
    half_size: [f64; 2],
    inflate: f64,
) -> bool {
    if segment_intersects_axis_aligned_rectangle(start, end, half_size) {
        return true;
    }
    let corners = [
        [-half_size[0], -half_size[1]],
        [half_size[0], -half_size[1]],
        [half_size[0], half_size[1]],
        [-half_size[0], half_size[1]],
    ];
    (0..4).any(|edge| {
        segment_segment_distance_squared(start, end, corners[edge], corners[(edge + 1) % 4])
            < inflate * inflate
    })
}

impl KiCadRoutingModel {
    fn from_pcb_with_config(
        pcb: &Expr,
        connection: &str,
        config: &KiCadGridRouteConfig,
    ) -> Result<Self, String> {
        let mut model = Self::from_pcb(pcb, connection)?;
        connection_rules::apply(&mut model, config)?;
        Ok(model)
    }

    fn from_pcb(pcb: &Expr, connection: &str) -> Result<Self, String> {
        let Expr::List(items) = pcb else {
            return Err("PCB root is not a list".into());
        };
        if root_head(items) != Some("kicad_pcb") {
            return Err("expected kicad_pcb root".into());
        }
        let outline = board_outline(pcb)?;
        let mut terminals = Vec::new();
        let mut terminal_pads = Vec::new();
        let mut obstacles = Vec::new();
        let mut target_holes = Vec::new();
        for item in items {
            match item.head() {
                Some("footprint") => {
                    collect_footprint_copper(
                        item,
                        connection,
                        &mut terminals,
                        &mut terminal_pads,
                        &mut obstacles,
                        &mut target_holes,
                    )?;
                }
                Some("segment") => {
                    let net = node_net(item).map(normalize_net).map(str::to_owned);
                    if net.as_deref() == Some(connection) {
                        continue;
                    }
                    let Some(layer) = form_atom(item, "layer", 1).and_then(copper_layer_index)
                    else {
                        continue;
                    };
                    let object = net.clone().unwrap_or_else(|| "unconnected-copper".into());
                    obstacles.push(CopperObstacle {
                        source_uuid: form_atom(item, "uuid", 1).map(str::to_owned),
                        blocks_tracks: true,
                        blocks_vias: true,
                        local_clearance_mm: 0.0,
                        layers: layer_mask(layer),
                        geometry: ObstacleGeometry::Segment {
                            start: form_xy(item, "start")?,
                            end: form_xy(item, "end")?,
                            radius: form_f64(item, "width", 1)? / 2.0,
                        },
                        kind: KiCadViaLocalRerouteBlockerKind::ForeignNetCopper,
                        object,
                        net,
                        footprint: None,
                        component_movable: false,
                    });
                }
                Some("via") => {
                    let net = node_net(item).map(normalize_net).map(str::to_owned);
                    if net.as_deref() == Some(connection) {
                        continue;
                    }
                    let object = net.clone().unwrap_or_else(|| "unconnected-copper".into());
                    obstacles.push(CopperObstacle {
                        source_uuid: form_atom(item, "uuid", 1).map(str::to_owned),
                        blocks_tracks: true,
                        blocks_vias: true,
                        local_clearance_mm: 0.0,
                        layers: [true, true],
                        geometry: ObstacleGeometry::Circle {
                            center: form_xy(item, "at")?,
                            radius: form_f64(item, "size", 1)? / 2.0,
                        },
                        kind: KiCadViaLocalRerouteBlockerKind::ForeignNetCopper,
                        object,
                        net,
                        footprint: None,
                        component_movable: false,
                    });
                }
                Some("arc")
                    if !node_net(item).is_some_and(|net| normalize_net(net) == connection) =>
                {
                    return Err(format!(
                        "KiCad grid adapter does not yet rasterize non-target {} objects",
                        item.head().unwrap_or("copper")
                    ));
                }
                Some("zone") if is_rule_area(item) => {
                    let blocks_tracks = item
                        .child("keepout")
                        .and_then(|keepout| keepout.child("tracks"))
                        .and_then(|tracks| tracks.children().get(1))
                        .and_then(Expr::atom)
                        == Some("not_allowed");
                    let blocks_vias = item
                        .child("keepout")
                        .and_then(|keepout| keepout.child("vias"))
                        .and_then(|vias| vias.children().get(1))
                        .and_then(Expr::atom)
                        == Some("not_allowed");
                    if blocks_tracks || blocks_vias {
                        obstacles.push(CopperObstacle {
                            source_uuid: form_atom(item, "uuid", 1).map(str::to_owned),
                            blocks_tracks,
                            blocks_vias,
                            local_clearance_mm: 0.0,
                            layers: rule_area_layer_mask(item)?,
                            geometry: ObstacleGeometry::Polygon {
                                points: rule_area_polygon_points(item)?,
                            },
                            kind: KiCadViaLocalRerouteBlockerKind::RuleArea,
                            object: form_atom(item, "name", 1)
                                .unwrap_or("unnamed-rule-area")
                                .to_string(),
                            net: None,
                            footprint: None,
                            component_movable: false,
                        });
                    }
                }
                // Filled zones are yielding copper: KiCad withdraws their fill
                // around foreign pads, vias, and traces when it refills the
                // board. Treating the zone polygon as a hard obstacle would
                // block nearly the complete board. Its non-yielding pads and
                // retained stitching vias are already lowered above.
                Some("zone") => {}
                _ => {}
            }
        }
        let outline = outline.ok_or_else(|| {
            "KiCad grid adapter requires one closed Edge.Cuts outline encoded as a gr_rect or an unambiguous loop of gr_line objects"
                .to_string()
        })?;
        let bounds = outline.bounds;
        terminals.sort_by(|left, right| {
            left[0]
                .total_cmp(&right[0])
                .then_with(|| left[1].total_cmp(&right[1]))
        });
        terminals.dedup();
        terminal_pads.sort_by(|left, right| {
            left.at[0]
                .total_cmp(&right.at[0])
                .then_with(|| left.at[1].total_cmp(&right.at[1]))
                .then_with(|| left.footprint.cmp(&right.footprint))
        });
        let model = Self {
            bounds,
            outline,
            terminals,
            terminal_pads,
            obstacles,
            target_holes,
        };
        let contacts = model.electrical_terminals().len();
        if contacts < 2 {
            return Err(format!(
                "connection {connection:?} has {contacts} distinct electrical terminals; at least two are required"
            ));
        }
        Ok(model)
    }
}

fn rule_area_layer_mask(zone: &Expr) -> Result<[bool; 2], String> {
    let mut mask = [false; 2];
    if let Some(layer) = form_atom(zone, "layer", 1) {
        let index = copper_layer_index(layer)
            .ok_or_else(|| format!("unsupported rule-area layer {layer:?}"))?;
        mask[index] = true;
    } else if let Some(layers) = zone.child("layers") {
        for layer in layers.children().iter().skip(1).filter_map(Expr::atom) {
            match layer {
                "F&B.Cu" | "*.Cu" => mask = [true, true],
                "F.Cu" => mask[0] = true,
                "B.Cu" => mask[1] = true,
                _ => return Err(format!("unsupported rule-area layer {layer:?}")),
            }
        }
    }
    if mask == [false, false] {
        Err("rule area has no supported copper layer".into())
    } else {
        Ok(mask)
    }
}

fn rule_area_polygon_points(zone: &Expr) -> Result<Vec<[f64; 2]>, String> {
    let points = zone
        .child("polygon")
        .and_then(|polygon| polygon.child("pts"))
        .ok_or_else(|| "rule area has no polygon points".to_string())?
        .children()
        .iter()
        .skip(1)
        .filter(|point| point.head() == Some("xy"))
        .map(|point| {
            let coordinate = |index: usize| {
                point
                    .children()
                    .get(index)
                    .and_then(Expr::atom)
                    .ok_or_else(|| "rule-area xy point is missing a coordinate".to_string())?
                    .parse::<f64>()
                    .map_err(|error| format!("invalid rule-area xy coordinate: {error}"))
            };
            Ok::<[f64; 2], String>([coordinate(1)?, coordinate(2)?])
        })
        .collect::<Result<Vec<_>, _>>()?;
    if points.len() < 3 {
        Err("rule area polygon has fewer than three points".into())
    } else {
        Ok(points)
    }
}

fn collect_footprint_copper(
    footprint: &Expr,
    connection: &str,
    terminals: &mut Vec<[f64; 2]>,
    terminal_pads: &mut Vec<KiCadTerminalPad>,
    obstacles: &mut Vec<CopperObstacle>,
    target_holes: &mut Vec<ObstacleGeometry>,
) -> Result<(), String> {
    let footprint_at = form_at(footprint)?;
    let reference = footprint_reference(footprint).unwrap_or_default();
    let component_movable = footprint_router_movable(footprint);
    for pad in footprint
        .children()
        .iter()
        .filter(|item| item.head() == Some("pad"))
    {
        let pad_at = form_at(pad)?;
        // KiCad's board coordinates have Y pointing down: positive footprint
        // angles therefore rotate local pad coordinates clockwise.
        let center_offset = rotate_vector([pad_at[0], pad_at[1]], -footprint_at[2]);
        let center = [
            footprint_at[0] + center_offset[0],
            footprint_at[1] + center_offset[1],
        ];
        let layers = pad_copper_layers(pad);
        let size = form_xy(pad, "size")?;
        let shape = pad.children().get(3).and_then(Expr::atom).unwrap_or("rect");
        // KiCad's drill offset displaces copper relative to the drill/pad
        // anchor. Pad angles in board files are absolute, and Y points down.
        let copper_offset = match pad.child("drill").and_then(|d| d.child("offset")) {
            Some(offset) => rotate_vector(
                [
                    expression_coordinate(offset, 1, "pad copper offset x")?,
                    expression_coordinate(offset, 2, "pad copper offset y")?,
                ],
                -pad_at[2],
            ),
            None => [0.0, 0.0],
        };
        let copper_center = [center[0] + copper_offset[0], center[1] + copper_offset[1]];
        let anchor_geometry = match shape {
            "roundrect" => roundrect_pad::geometry(pad, copper_center, size, -pad_at[2])?,
            "circle" => ObstacleGeometry::Circle {
                center: copper_center,
                radius: size[0].max(size[1]) / 2.0,
            },
            "oval" => {
                // Pad angles in a board file are already absolute, rather
                // than relative to the footprint angle.
                let angle = -pad_at[2];
                let (half_length, radius, local_end) = if size[0] >= size[1] {
                    ((size[0] - size[1]) / 2.0, size[1] / 2.0, [1.0, 0.0])
                } else {
                    ((size[1] - size[0]) / 2.0, size[0] / 2.0, [0.0, 1.0])
                };
                let offset = rotate_vector(
                    [local_end[0] * half_length, local_end[1] * half_length],
                    angle,
                );
                ObstacleGeometry::Segment {
                    start: [copper_center[0] - offset[0], copper_center[1] - offset[1]],
                    end: [copper_center[0] + offset[0], copper_center[1] + offset[1]],
                    radius,
                }
            }
            _ => ObstacleGeometry::Rectangle {
                center: copper_center,
                half_size: [size[0] / 2.0, size[1] / 2.0],
                angle_degrees: -pad_at[2],
            },
        };
        let geometry = if shape == "custom" {
            custom_pad_geometry(pad, copper_center, pad_at[2], anchor_geometry)?
        } else {
            anchor_geometry
        };
        let net = node_net(pad).map(normalize_net).map(str::to_owned);
        if net.as_deref() == Some(connection) {
            terminals.push(center);
            terminal_pads.push(KiCadTerminalPad {
                at: center,
                footprint: reference.clone(),
                local_position: [pad_at[0], pad_at[1]],
                layers,
                plated_through_hole: electrical_terminals::plated_pad(pad),
                geometry,
            });
            if let Some(drill) = pad.child("drill") {
                let diameter = drill
                    .children()
                    .iter()
                    .skip(1)
                    .filter_map(Expr::atom)
                    .filter_map(|value| value.parse::<f64>().ok())
                    .fold(0.0_f64, f64::max);
                if diameter > 0.0 {
                    target_holes.push(ObstacleGeometry::Circle {
                        center,
                        radius: diameter / 2.0,
                    });
                }
            }
            continue;
        }
        if !layers[0] && !layers[1] {
            continue;
        }
        obstacles.push(CopperObstacle {
            source_uuid: form_atom(pad, "uuid", 1).map(str::to_owned),
            blocks_tracks: true,
            blocks_vias: true,
            local_clearance_mm: local_clearance::pad_clearance(pad, footprint)?,
            layers,
            geometry,
            kind: KiCadViaLocalRerouteBlockerKind::FootprintPad,
            object: if reference.is_empty() {
                "unowned-pad".into()
            } else {
                reference.clone()
            },
            net,
            footprint: (!reference.is_empty()).then(|| reference.clone()),
            component_movable,
        });
    }
    Ok(())
}

fn custom_pad_geometry(
    pad: &Expr,
    center: [f64; 2],
    pad_angle_degrees: f64,
    anchor: ObstacleGeometry,
) -> Result<ObstacleGeometry, String> {
    let mut parts = vec![anchor];
    let primitives = pad
        .child("primitives")
        .ok_or_else(|| "custom pad has no primitives form".to_string())?;
    for primitive in primitives.children().iter().skip(1) {
        if primitive.head() != Some("gr_poly")
            || form_atom(primitive, "fill", 1) != Some("yes")
            || form_atom(primitive, "width", 1) != Some("0")
        {
            return Err(format!(
                "KiCad grid adapter does not yet lower custom-pad primitive {} unless it is a zero-width filled polygon",
                primitive.head().unwrap_or("unknown")
            ));
        }
        let points_form = primitive
            .child("pts")
            .ok_or_else(|| "custom-pad polygon has no pts form".to_string())?;
        let mut points = Vec::new();
        for point in points_form.children().iter().skip(1) {
            if point.head() != Some("xy") {
                return Err("custom-pad polygon contains a non-xy point".into());
            }
            let local = [
                expression_coordinate(point, 1, "custom-pad x")?,
                expression_coordinate(point, 2, "custom-pad y")?,
            ];
            let offset = rotate_vector(local, -pad_angle_degrees);
            points.push([center[0] + offset[0], center[1] + offset[1]]);
        }
        if points.len() < 3 {
            return Err("custom-pad polygon has fewer than three points".into());
        }
        parts.push(ObstacleGeometry::Polygon { points });
    }
    if parts.len() == 1 {
        return Err("custom pad has no supported copper primitives".into());
    }
    Ok(ObstacleGeometry::Union { parts })
}

fn expression_coordinate(node: &Expr, index: usize, label: &str) -> Result<f64, String> {
    node.children()
        .get(index)
        .and_then(Expr::atom)
        .ok_or_else(|| format!("{label} is missing"))?
        .parse::<f64>()
        .map_err(|error| format!("invalid {label}: {error}"))
}

fn footprint_infos(pcb: &Expr) -> Result<BTreeMap<String, KiCadFootprintInfo>, String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut result = BTreeMap::new();
    for footprint in items.iter().filter(|item| item.head() == Some("footprint")) {
        let Some(reference) = footprint_reference(footprint).filter(|value| !value.is_empty())
        else {
            continue;
        };
        let at = form_at(footprint)?;
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for graphic in footprint.children() {
            if graphic.head() == Some("fp_rect")
                && form_atom(graphic, "layer", 1) == Some("F.CrtYd")
            {
                let start = form_xy(graphic, "start")?;
                let end = form_xy(graphic, "end")?;
                for point in [start, end] {
                    for axis in 0..2 {
                        minimum[axis] = minimum[axis].min(point[axis]);
                        maximum[axis] = maximum[axis].max(point[axis]);
                    }
                }
            }
        }
        if !(0..2).all(|axis| {
            minimum[axis].is_finite() && maximum[axis].is_finite() && maximum[axis] > minimum[axis]
        }) {
            minimum = [f64::INFINITY; 2];
            maximum = [f64::NEG_INFINITY; 2];
            for pad in footprint
                .children()
                .iter()
                .filter(|item| item.head() == Some("pad"))
            {
                let pad_at = form_at(pad)?;
                let size = form_xy(pad, "size")?;
                for axis in 0..2 {
                    minimum[axis] = minimum[axis].min(pad_at[axis] - size[axis] / 2.0);
                    maximum[axis] = maximum[axis].max(pad_at[axis] + size[axis] / 2.0);
                }
            }
        }
        for axis in 0..2 {
            if !minimum[axis].is_finite() || !maximum[axis].is_finite() {
                minimum[axis] = -0.005;
                maximum[axis] = 0.005;
            }
        }
        let router_movable = footprint_router_movable(footprint);
        let info = KiCadFootprintInfo {
            reference: reference.clone(),
            at,
            body_center_local: [
                (minimum[0] + maximum[0]) / 2.0,
                (minimum[1] + maximum[1]) / 2.0,
            ],
            body_size: [
                (maximum[0] - minimum[0]).max(0.01),
                (maximum[1] - minimum[1]).max(0.01),
            ],
            router_movable,
        };
        if result.insert(reference.clone(), info).is_some() {
            return Err(format!(
                "PCB contains duplicate footprint reference {reference}"
            ));
        }
    }
    Ok(result)
}

fn footprint_router_movable(footprint: &Expr) -> bool {
    footprint.children().iter().any(|item| {
        item.head() == Some("property")
            && item.children().get(1).and_then(Expr::atom) == Some("Router_Movable")
            && item
                .children()
                .get(2)
                .and_then(Expr::atom)
                .is_some_and(|value| value.eq_ignore_ascii_case("yes"))
    })
}

fn pad_copper_layers(pad: &Expr) -> [bool; 2] {
    let Some(layers) = pad.child("layers") else {
        return [false, false];
    };
    let names: Vec<_> = layers
        .children()
        .iter()
        .skip(1)
        .filter_map(Expr::atom)
        .collect();
    [
        names.iter().any(|name| matches!(*name, "F.Cu" | "*.Cu")),
        names.iter().any(|name| matches!(*name, "B.Cu" | "*.Cu")),
    ]
}

fn preferred_terminal_layer(layers: [bool; 2]) -> Option<usize> {
    match layers {
        [true, _] => Some(0),
        [false, true] => Some(1),
        [false, false] => None,
    }
}

fn copper_layer_index(layer: &str) -> Option<usize> {
    match layer {
        "F.Cu" => Some(0),
        "B.Cu" => Some(1),
        _ => None,
    }
}

fn layer_mask(layer: usize) -> [bool; 2] {
    [layer == 0, layer == 1]
}

fn form_atom<'a>(node: &'a Expr, head: &str, index: usize) -> Option<&'a str> {
    node.child(head)?.children().get(index)?.atom()
}

fn form_f64(node: &Expr, head: &str, index: usize) -> Result<f64, String> {
    form_atom(node, head, index)
        .ok_or_else(|| format!("{head} form is missing coordinate {index}"))?
        .parse::<f64>()
        .map_err(|error| format!("invalid {head} coordinate: {error}"))
}

fn form_xy(node: &Expr, head: &str) -> Result<[f64; 2], String> {
    Ok([form_f64(node, head, 1)?, form_f64(node, head, 2)?])
}

fn form_at(node: &Expr) -> Result<[f64; 3], String> {
    let at = node
        .child("at")
        .ok_or_else(|| "copper item has no at form".to_string())?;
    Ok([
        form_f64(node, "at", 1)?,
        form_f64(node, "at", 2)?,
        at.children()
            .get(3)
            .and_then(Expr::atom)
            .unwrap_or("0")
            .parse::<f64>()
            .map_err(|error| format!("invalid at rotation: {error}"))?,
    ])
}

fn rotate_vector(vector: [f64; 2], degrees: f64) -> [f64; 2] {
    let radians = degrees.to_radians();
    let cosine = radians.cos();
    let sine = radians.sin();
    [
        vector[0] * cosine - vector[1] * sine,
        vector[0] * sine + vector[1] * cosine,
    ]
}

fn footprint_body_center(info: &KiCadFootprintInfo) -> [f64; 2] {
    let offset = rotate_vector(info.body_center_local, -info.at[2]);
    [info.at[0] + offset[0], info.at[1] + offset[1]]
}

fn footprint_motion_envelopes_can_meet(
    first: &KiCadFootprintInfo,
    first_motion: &KiCadMovableFootprintConfig,
    second: &KiCadFootprintInfo,
    second_motion: Option<&KiCadMovableFootprintConfig>,
) -> bool {
    let body_radius = |info: &KiCadFootprintInfo| {
        ((info.body_size[0] / 2.0).powi(2) + (info.body_size[1] / 2.0).powi(2)).sqrt()
    };
    let sweep = |info: &KiCadFootprintInfo, motion: &KiCadMovableFootprintConfig| {
        let rotation_sweep = 2.0
            * body_radius(info)
            * (motion.maximum_rotation_degrees.to_radians() / 2.0)
                .sin()
                .abs();
        motion.maximum_translation_mm + rotation_sweep
    };
    let first_center = footprint_body_center(first);
    let second_center = footprint_body_center(second);
    let circle_gap = distance_squared(first_center, second_center).sqrt()
        - body_radius(first)
        - body_radius(second);
    let reachable_motion =
        sweep(first, first_motion) + second_motion.map_or(0.0, |motion| sweep(second, motion));
    circle_gap <= reachable_motion + 1.0e-9
}

fn distance_squared(left: [f64; 2], right: [f64; 2]) -> f64 {
    (left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2)
}

fn point_segment_distance_squared(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let segment = [end[0] - start[0], end[1] - start[1]];
    let length_squared = segment[0].powi(2) + segment[1].powi(2);
    if length_squared <= f64::EPSILON {
        return distance_squared(point, start);
    }
    let projection = (((point[0] - start[0]) * segment[0] + (point[1] - start[1]) * segment[1])
        / length_squared)
        .clamp(0.0, 1.0);
    distance_squared(
        point,
        [
            start[0] + projection * segment[0],
            start[1] + projection * segment[1],
        ],
    )
}

fn segment_segment_distance_squared(
    first_start: [f64; 2],
    first_end: [f64; 2],
    second_start: [f64; 2],
    second_end: [f64; 2],
) -> f64 {
    if line_segments_intersect(first_start, first_end, second_start, second_end) {
        return 0.0;
    }
    point_segment_distance_squared(first_start, second_start, second_end)
        .min(point_segment_distance_squared(
            first_end,
            second_start,
            second_end,
        ))
        .min(point_segment_distance_squared(
            second_start,
            first_start,
            first_end,
        ))
        .min(point_segment_distance_squared(
            second_end,
            first_start,
            first_end,
        ))
}

fn line_segments_intersect(
    first_start: [f64; 2],
    first_end: [f64; 2],
    second_start: [f64; 2],
    second_end: [f64; 2],
) -> bool {
    fn cross(origin: [f64; 2], left: [f64; 2], right: [f64; 2]) -> f64 {
        (left[0] - origin[0]) * (right[1] - origin[1])
            - (left[1] - origin[1]) * (right[0] - origin[0])
    }
    fn contains(start: [f64; 2], end: [f64; 2], point: [f64; 2]) -> bool {
        point[0] >= start[0].min(end[0]) - 1e-9
            && point[0] <= start[0].max(end[0]) + 1e-9
            && point[1] >= start[1].min(end[1]) - 1e-9
            && point[1] <= start[1].max(end[1]) + 1e-9
    }

    let first_side_a = cross(first_start, first_end, second_start);
    let first_side_b = cross(first_start, first_end, second_end);
    let second_side_a = cross(second_start, second_end, first_start);
    let second_side_b = cross(second_start, second_end, first_end);
    if ((first_side_a > 1e-9 && first_side_b < -1e-9)
        || (first_side_a < -1e-9 && first_side_b > 1e-9))
        && ((second_side_a > 1e-9 && second_side_b < -1e-9)
            || (second_side_a < -1e-9 && second_side_b > 1e-9))
    {
        return true;
    }
    (first_side_a.abs() <= 1e-9 && contains(first_start, first_end, second_start))
        || (first_side_b.abs() <= 1e-9 && contains(first_start, first_end, second_end))
        || (second_side_a.abs() <= 1e-9 && contains(second_start, second_end, first_start))
        || (second_side_b.abs() <= 1e-9 && contains(second_start, second_end, first_end))
}

fn segment_intersects_axis_aligned_rectangle(
    start: [f64; 2],
    end: [f64; 2],
    half_size: [f64; 2],
) -> bool {
    let delta = [end[0] - start[0], end[1] - start[1]];
    let mut minimum = 0.0_f64;
    let mut maximum = 1.0_f64;
    for axis in 0..2 {
        if delta[axis].abs() <= f64::EPSILON {
            if start[axis].abs() <= half_size[axis] {
                continue;
            }
            return false;
        }
        let mut enter = (-half_size[axis] - start[axis]) / delta[axis];
        let mut leave = (half_size[axis] - start[axis]) / delta[axis];
        if enter > leave {
            std::mem::swap(&mut enter, &mut leave);
        }
        minimum = minimum.max(enter);
        maximum = maximum.min(leave);
        if minimum > maximum {
            return false;
        }
    }
    true
}

fn snapped_grid_position(
    terminal: [f64; 2],
    origin: [f64; 2],
    config: &KiCadGridRouteConfig,
    width: usize,
    height: usize,
) -> Result<GridPosition, String> {
    let x = ((terminal[0] - origin[0]) / config.resolution_mm).round() as isize;
    let y = ((terminal[1] - origin[1]) / config.resolution_mm).round() as isize;
    if x < 0 || y < 0 || x >= width as isize || y >= height as isize {
        return Err(format!(
            "terminal ({:.4}, {:.4}) lies outside the routing grid",
            terminal[0], terminal[1]
        ));
    }
    Ok(GridPosition {
        x: x as usize,
        y: y as usize,
        layer: 0,
    })
}

fn materialize_grid_path(
    connection: &str,
    grid_path: &[GridPosition],
    terminals: [[f64; 2]; 2],
    terminal_pads: [Option<&ObstacleGeometry>; 2],
    origin: [f64; 2],
    config: &KiCadGridRouteConfig,
) -> (
    Vec<KiCadRoutePoint>,
    Vec<SupplementalSegment>,
    Vec<SupplementalVia>,
) {
    let mut points: Vec<([f64; 2], usize)> = grid_path
        .iter()
        .map(|point| {
            (
                [
                    origin[0] + point.x as f64 * config.resolution_mm,
                    origin[1] + point.y as f64 * config.resolution_mm,
                ],
                point.layer,
            )
        })
        .collect();
    if let Some(first) = points.first().copied()
        && first.0 != terminals[0]
    {
        points.insert(0, (terminals[0], first.1));
    }
    if let Some(last) = points.last().copied()
        && last.0 != terminals[1]
    {
        points.push((terminals[1], last.1));
    }
    if config.terminal_contact_policy == KiCadTerminalContactPolicy::FirstPadContact {
        if let Some(pad) = terminal_pads[0] {
            trim_path_start_to_first_pad_contact(&mut points, pad);
        }
        if let Some(pad) = terminal_pads[1] {
            points.reverse();
            trim_path_start_to_first_pad_contact(&mut points, pad);
            points.reverse();
        }
    }
    points = simplify_route_points(points);
    let path = points
        .iter()
        .map(|(at, layer)| KiCadRoutePoint {
            at: *at,
            layer: copper_layer_name(*layer).to_string(),
        })
        .collect();
    let mut segments = Vec::new();
    let mut vias = Vec::new();
    for pair in points.windows(2) {
        let (start, start_layer) = pair[0];
        let (end, end_layer) = pair[1];
        if start_layer == end_layer {
            if start != end {
                segments.push(SupplementalSegment {
                    connection: connection.to_string(),
                    start,
                    end,
                    width: config.trace_width_mm,
                    layer: copper_layer_name(start_layer).to_string(),
                });
            }
        } else {
            vias.push(SupplementalVia {
                connection: connection.to_string(),
                at: start,
                size: config.via_size_mm,
                drill: config.via_drill_mm,
                layers: ["F.Cu".into(), "B.Cu".into()],
            });
        }
    }
    (path, segments, vias)
}

fn trim_path_start_to_first_pad_contact(
    points: &mut Vec<([f64; 2], usize)>,
    pad: &ObstacleGeometry,
) {
    let Some(first) = points.first().copied() else {
        return;
    };
    if !pad.contains(first.0, 0.0) {
        return;
    }
    let Some(outside_index) = points.iter().position(|point| !pad.contains(point.0, 0.0)) else {
        return;
    };
    let inside = points[outside_index - 1];
    let outside = points[outside_index];
    if points[..=outside_index]
        .windows(2)
        .any(|pair| pair[0].1 != pair[1].1)
    {
        // Clipping must not remove a via anywhere in the discarded pad tail.
        // Pad geometry alone does not establish connectivity between layers;
        // an SMD terminal may need an in-pad via to reach this planar segment.
        return;
    }
    let contact = pad_boundary_contact(inside.0, outside.0, pad);
    points.drain(..outside_index);
    points.insert(0, (contact, outside.1));
}

fn pad_boundary_contact(inside: [f64; 2], outside: [f64; 2], pad: &ObstacleGeometry) -> [f64; 2] {
    let mut inside_fraction = 0.0;
    let mut outside_fraction = 1.0;
    for _ in 0..52 {
        let middle = (inside_fraction + outside_fraction) / 2.0;
        let point = [
            inside[0] + (outside[0] - inside[0]) * middle,
            inside[1] + (outside[1] - inside[1]) * middle,
        ];
        if pad.contains(point, 0.0) {
            inside_fraction = middle;
        } else {
            outside_fraction = middle;
        }
    }
    [
        inside[0] + (outside[0] - inside[0]) * inside_fraction,
        inside[1] + (outside[1] - inside[1]) * inside_fraction,
    ]
}

fn shorten_branch_path_with_strategy(
    path: &[KiCadRoutePoint],
    model: &KiCadRoutingModel,
    config: &KiCadGridRouteConfig,
    shortening_config: &KiCadRouteShorteningConfig,
    visibility_tests: &mut usize,
) -> Result<(Vec<KiCadRoutePoint>, usize, usize, usize), String> {
    if path.len() < 2 {
        return Err("route branch must contain at least two points".into());
    }
    for point in path {
        if copper_layer_index(&point.layer).is_none() {
            return Err(format!(
                "unsupported route-candidate layer {:?}",
                point.layer
            ));
        }
    }
    match shortening_config.strategy {
        KiCadRouteShorteningStrategy::GreedyLineOfSight => shorten_branch_path_greedy(
            path,
            model,
            config,
            shortening_config.maximum_visibility_tests,
            visibility_tests,
        ),
        KiCadRouteShorteningStrategy::VisibilityShortestPath => shorten_branch_path_visibility(
            path,
            model,
            config,
            shortening_config.maximum_visibility_tests,
            visibility_tests,
        ),
    }
}

fn shorten_branch_path_greedy(
    path: &[KiCadRoutePoint],
    model: &KiCadRoutingModel,
    config: &KiCadGridRouteConfig,
    maximum_visibility_tests: usize,
    visibility_tests: &mut usize,
) -> Result<(Vec<KiCadRoutePoint>, usize, usize, usize), String> {
    let mut output = vec![path[0].clone()];
    let mut current = 0;
    let mut attempted = 0;
    let mut visible = 0;
    let mut accepted = 0;
    while current + 1 < path.len() {
        if path[current + 1].layer != path[current].layer {
            output.push(path[current + 1].clone());
            current += 1;
            continue;
        }
        let mut run_end = current + 1;
        while run_end + 1 < path.len() && path[run_end + 1].layer == path[current].layer {
            run_end += 1;
        }
        let layer = copper_layer_index(&path[current].layer).expect("validated copper layer");
        let mut chosen = current + 1;
        for candidate in ((current + 2)..=run_end).rev() {
            consume_visibility_test(visibility_tests, maximum_visibility_tests)?;
            attempted += 1;
            if route_segment_is_clear(path[current].at, path[candidate].at, layer, model, config) {
                visible += 1;
                chosen = candidate;
                accepted += 1;
                break;
            }
        }
        output.push(path[chosen].clone());
        current = chosen;
    }
    Ok((output, attempted, visible, accepted))
}

#[derive(Clone, Copy, Debug)]
struct RetainedPathCost {
    length_mm: f64,
    segments: usize,
}

fn shorten_branch_path_visibility(
    path: &[KiCadRoutePoint],
    model: &KiCadRoutingModel,
    config: &KiCadGridRouteConfig,
    maximum_visibility_tests: usize,
    visibility_tests: &mut usize,
) -> Result<(Vec<KiCadRoutePoint>, usize, usize, usize), String> {
    let mut output = Vec::new();
    let mut attempted_shortcuts = 0;
    let mut visible_shortcuts = 0;
    let mut accepted_shortcuts = 0;
    let mut run_start = 0;
    while run_start < path.len() {
        let mut run_end = run_start;
        while run_end + 1 < path.len() && path[run_end + 1].layer == path[run_start].layer {
            run_end += 1;
        }
        let run = &path[run_start..=run_end];
        if run.len() == 1 {
            output.push(run[0].clone());
        } else {
            let layer = copper_layer_index(&run[0].layer).expect("validated copper layer");
            let mut best = vec![None; run.len()];
            let mut predecessor = vec![None; run.len()];
            best[0] = Some(RetainedPathCost {
                length_mm: 0.0,
                segments: 0,
            });
            for finish in 1..run.len() {
                for start in 0..finish {
                    let Some(prefix) = best[start] else {
                        continue;
                    };
                    consume_visibility_test(visibility_tests, maximum_visibility_tests)?;
                    let is_shortcut = finish > start + 1;
                    if is_shortcut {
                        attempted_shortcuts += 1;
                    }
                    if !route_segment_is_clear(run[start].at, run[finish].at, layer, model, config)
                    {
                        continue;
                    }
                    if is_shortcut {
                        visible_shortcuts += 1;
                    }
                    let proposal = RetainedPathCost {
                        length_mm: prefix.length_mm
                            + distance_squared(run[start].at, run[finish].at).sqrt(),
                        segments: prefix.segments + 1,
                    };
                    let replace = best[finish].is_none_or(|current| {
                        proposal.length_mm < current.length_mm - 1.0e-9
                            || ((proposal.length_mm - current.length_mm).abs() <= 1.0e-9
                                && (proposal.segments < current.segments
                                    || (proposal.segments == current.segments
                                        && predecessor[finish]
                                            .is_none_or(|previous| start < previous))))
                    });
                    if replace {
                        best[finish] = Some(proposal);
                        predecessor[finish] = Some(start);
                    }
                }
            }
            if best[run.len() - 1].is_none() {
                return Err(format!(
                    "retained visibility graph has no path through layer {:?}",
                    run[0].layer
                ));
            }
            let mut selected = vec![run.len() - 1];
            while let Some(previous) = predecessor[*selected.last().expect("non-empty path")] {
                selected.push(previous);
            }
            selected.reverse();
            accepted_shortcuts += selected
                .windows(2)
                .filter(|pair| pair[1] > pair[0] + 1)
                .count();
            output.extend(selected.into_iter().map(|index| run[index].clone()));
        }
        run_start = run_end + 1;
    }
    Ok((
        output,
        attempted_shortcuts,
        visible_shortcuts,
        accepted_shortcuts,
    ))
}

fn consume_visibility_test(
    visibility_tests: &mut usize,
    maximum_visibility_tests: usize,
) -> Result<(), String> {
    if *visibility_tests >= maximum_visibility_tests {
        return Err(format!(
            "retained route shortening exhausted its {maximum_visibility_tests} visibility-test budget"
        ));
    }
    *visibility_tests += 1;
    Ok(())
}

fn route_segment_is_clear(
    start: [f64; 2],
    end: [f64; 2],
    layer: usize,
    model: &KiCadRoutingModel,
    config: &KiCadGridRouteConfig,
) -> bool {
    let trace_radius = config.trace_width_mm / 2.0;
    let edge_margin = config.edge_clearance_mm + trace_radius;
    model
        .outline
        .contains_segment_with_clearance(start, end, edge_margin)
        && !model.obstacles.iter().any(|obstacle| {
            obstacle.blocks_tracks
                && obstacle.layers[layer]
                && obstacle.geometry.intersects_segment(
                    start,
                    end,
                    trace_radius + obstacle.clearance(config.clearance_mm),
                )
        })
}

fn selected_geometry_blockers(
    candidate: &KiCadRouteCandidate,
    selected: &[usize],
    model: &KiCadRoutingModel,
) -> Vec<String> {
    const MAX_REPORTED_BLOCKERS: usize = 128;
    let trace_radius = candidate.config.trace_width_mm / 2.0;
    let edge_margin = candidate.config.edge_clearance_mm + trace_radius;
    let obstacle_inflate = trace_radius + candidate.config.clearance_mm;
    let mut blockers = Vec::new();
    for &branch_index in selected {
        let branch = &candidate.branches[branch_index];
        for (segment_index, pair) in branch.path.windows(2).enumerate() {
            let Some(layer) = copper_layer_index(&pair[0].layer) else {
                blockers.push(format!(
                    "branch {branch_index} segment {segment_index} uses unsupported layer {:?}",
                    pair[0].layer
                ));
                continue;
            };
            if pair[0].layer != pair[1].layer {
                if pair[0].at != pair[1].at {
                    blockers.push(format!(
                        "branch {branch_index} segment {segment_index} changes copper layers and coordinates"
                    ));
                }
                continue;
            }
            if !model
                .outline
                .contains_segment_with_clearance(pair[0].at, pair[1].at, edge_margin)
            {
                blockers.push(format!(
                    "branch {branch_index} segment {segment_index} violates board-edge clearance"
                ));
            }
            for (obstacle_index, obstacle) in model.obstacles.iter().enumerate() {
                if obstacle.blocks_tracks
                    && obstacle.layers[layer]
                    && obstacle.geometry.intersects_segment(
                        pair[0].at,
                        pair[1].at,
                        obstacle.inflate(obstacle_inflate, candidate.config.clearance_mm),
                    )
                {
                    let kind = match &obstacle.geometry {
                        ObstacleGeometry::Circle { .. } => "circle",
                        ObstacleGeometry::Segment { .. } => "capsule",
                        ObstacleGeometry::Rectangle { .. } => "rectangle",
                        ObstacleGeometry::Polygon { .. } => "polygon",
                        ObstacleGeometry::Union { .. } => "compound",
                    };
                    blockers.push(format!(
                        "branch {branch_index} segment {segment_index} ({:?} -> {:?}) intersects {kind} obstacle {obstacle_index} {:?} on {}",
                        pair[0].at,
                        pair[1].at,
                        obstacle.geometry,
                        pair[0].layer
                    ));
                    if blockers.len() >= MAX_REPORTED_BLOCKERS {
                        return blockers;
                    }
                }
            }
        }
    }
    blockers
}

fn materialize_candidate_branches(
    connection: &str,
    branches: &[KiCadRouteBranch],
    config: &KiCadGridRouteConfig,
) -> Result<(Vec<SupplementalSegment>, Vec<SupplementalVia>), String> {
    let mut normalized_branches = branches.to_vec();
    let selected = (0..normalized_branches.len()).collect::<Vec<_>>();
    normalize_selected_route_graph(&mut normalized_branches, &selected);
    let mut segments = Vec::new();
    let mut vias = Vec::new();
    let mut segment_keys = BTreeSet::new();
    let mut via_keys = BTreeSet::new();
    for branch in &normalized_branches {
        for pair in branch.path.windows(2) {
            let start_layer = copper_layer_index(&pair[0].layer)
                .ok_or_else(|| format!("unsupported route-candidate layer {:?}", pair[0].layer))?;
            let end_layer = copper_layer_index(&pair[1].layer)
                .ok_or_else(|| format!("unsupported route-candidate layer {:?}", pair[1].layer))?;
            if start_layer == end_layer {
                if pair[0].at == pair[1].at {
                    continue;
                }
                let segment = SupplementalSegment {
                    connection: connection.to_string(),
                    start: pair[0].at,
                    end: pair[1].at,
                    width: config.trace_width_mm,
                    layer: pair[0].layer.clone(),
                };
                if segment_keys.insert(supplemental_segment_key(&segment)) {
                    segments.push(segment);
                }
            } else {
                if pair[0].at != pair[1].at {
                    return Err(
                        "a route-candidate layer change must keep the same coordinate".into(),
                    );
                }
                let via = SupplementalVia {
                    connection: connection.to_string(),
                    at: pair[0].at,
                    size: config.via_size_mm,
                    drill: config.via_drill_mm,
                    layers: ["F.Cu".into(), "B.Cu".into()],
                };
                if via_keys.insert(supplemental_via_key(&via)) {
                    vias.push(via);
                }
            }
        }
    }
    if segments.is_empty() && vias.is_empty() {
        return Err("shortened route candidate has no copper".into());
    }
    Ok((segments, vias))
}

fn branch_bend_count(path: &[KiCadRoutePoint]) -> usize {
    path.windows(3)
        .filter(|points| {
            if points[0].layer != points[1].layer || points[1].layer != points[2].layer {
                return false;
            }
            let left = [
                points[1].at[0] - points[0].at[0],
                points[1].at[1] - points[0].at[1],
            ];
            let right = [
                points[2].at[0] - points[1].at[0],
                points[2].at[1] - points[1].at[1],
            ];
            (left[0] * right[1] - left[1] * right[0]).abs() > 1e-9
                || left[0] * right[0] + left[1] * right[1] <= 0.0
        })
        .count()
}

fn branch_via_count(path: &[KiCadRoutePoint]) -> usize {
    path.windows(2)
        .filter(|points| points[0].layer != points[1].layer)
        .count()
}

fn simplify_route_points(mut points: Vec<([f64; 2], usize)>) -> Vec<([f64; 2], usize)> {
    points.dedup();
    let mut output: Vec<([f64; 2], usize)> = Vec::new();
    for point in points {
        while output.len() >= 2 {
            let first = output[output.len() - 2];
            let middle = output[output.len() - 1];
            if first.1 != middle.1 || middle.1 != point.1 {
                break;
            }
            let left = [middle.0[0] - first.0[0], middle.0[1] - first.0[1]];
            let right = [point.0[0] - middle.0[0], point.0[1] - middle.0[1]];
            if (left[0] * right[1] - left[1] * right[0]).abs() > 1e-9
                || left[0] * right[0] + left[1] * right[1] <= 0.0
            {
                break;
            }
            output.pop();
        }
        output.push(point);
    }
    output
}

fn copper_layer_name(layer: usize) -> &'static str {
    match layer {
        0 => "F.Cu",
        1 => "B.Cu",
        _ => unreachable!("the KiCad adapter currently has two copper layers"),
    }
}

fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn is_intentional_no_connect_group(violation: &serde_json::Value) -> bool {
    let Some(items) = violation["items"].as_array() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    let mut component = None;
    for item in items {
        let Some(description) = item["description"].as_str() else {
            return false;
        };
        let Some(net_start) = description.find("[unconnected-(") else {
            return false;
        };
        let Some(net_end) = description[net_start..].find("] of ") else {
            return false;
        };
        let after_net = &description[net_start + net_end + 5..];
        let Some(reference) = after_net.split(" on ").next() else {
            return false;
        };
        match component {
            None => component = Some(reference),
            Some(expected) if expected == reference => {}
            Some(_) => return false,
        }
    }
    true
}

fn rung_name(rung: usize, connections: &[String]) -> String {
    if rung == 0 {
        "empty".to_string()
    } else {
        connections[rung - 1]
            .to_ascii_lowercase()
            .replace(|character: char| !character.is_ascii_alphanumeric(), "-")
    }
}

struct PcbRouteEdits<'a> {
    vertex_overrides: &'a [RouteVertexOverride],
    supplemental_segments: &'a [SupplementalSegment],
    supplemental_vias: &'a [SupplementalVia],
    replacement_routes: &'a [KiCadRouteCandidate],
}

fn filter_pcb(
    mut root: Expr,
    selected: &BTreeSet<String>,
    schematic_fields: &BTreeMap<String, BTreeMap<String, String>>,
    pad_nets: &BTreeMap<(String, String), PadInfo>,
    route_edits: PcbRouteEdits<'_>,
) -> Result<Expr, String> {
    let mut placement_by_reference = BTreeMap::<String, KiCadFootprintPlacement>::new();
    for placement in route_edits
        .replacement_routes
        .iter()
        .filter(|candidate| selected.contains(&candidate.connection))
        .flat_map(|candidate| &candidate.footprint_placements)
    {
        if let Some(existing) = placement_by_reference.get(&placement.reference) {
            if existing != placement {
                return Err(format!(
                    "replacement routes disagree about footprint {} placement",
                    placement.reference
                ));
            }
        } else {
            placement_by_reference.insert(placement.reference.clone(), placement.clone());
        }
    }
    apply_footprint_placements(
        &mut root,
        &placement_by_reference.into_values().collect::<Vec<_>>(),
    )?;
    let mut reference_by_uuid = BTreeMap::<String, KiCadReferencePlacement>::new();
    for placement in route_edits
        .replacement_routes
        .iter()
        .filter(|candidate| selected.contains(&candidate.connection))
        .flat_map(|candidate| &candidate.reference_placements)
    {
        if let Some(existing) = reference_by_uuid.get(&placement.uuid) {
            if existing != placement {
                return Err(format!(
                    "replacement routes disagree about reference field {} ({}) placement",
                    placement.reference, placement.uuid
                ));
            }
        } else {
            reference_by_uuid.insert(placement.uuid.clone(), placement.clone());
        }
    }
    apply_reference_placements(
        &mut root,
        &reference_by_uuid.into_values().collect::<Vec<_>>(),
    )?;
    let Expr::List(items) = &mut root else {
        return Err("PCB root is not a list".into());
    };
    if root_head(items) != Some("kicad_pcb") {
        return Err("expected kicad_pcb root".into());
    }
    let mut output = Vec::with_capacity(items.len());
    let replaced: BTreeSet<_> = route_edits
        .replacement_routes
        .iter()
        .filter(|candidate| selected.contains(&candidate.connection))
        .map(|candidate| candidate.connection.as_str())
        .collect();
    for mut item in std::mem::take(items) {
        match item.head() {
            Some("segment" | "arc" | "via") => {
                if node_net(&item).is_some_and(|net| {
                    selected.contains(normalize_net(net)) && !replaced.contains(normalize_net(net))
                }) {
                    apply_route_vertex_overrides(&mut item, route_edits.vertex_overrides)?;
                    output.push(item);
                }
            }
            Some("zone") if is_rule_area(&item) => output.push(item),
            Some("zone") => {
                if node_net(&item).is_some_and(|net| {
                    selected.contains(normalize_net(net)) && !replaced.contains(normalize_net(net))
                }) {
                    output.push(item);
                }
            }
            Some("footprint") => {
                assign_pad_nets(&mut item, selected, pad_nets);
                prepare_footprint_for_parity(&mut item, schematic_fields);
                output.push(item);
            }
            _ => output.push(item),
        }
    }
    output.extend(
        route_edits
            .supplemental_segments
            .iter()
            .filter(|segment| selected.contains(&segment.connection))
            .map(supplemental_segment_expr),
    );
    output.extend(
        route_edits
            .supplemental_vias
            .iter()
            .filter(|via| selected.contains(&via.connection))
            .map(supplemental_via_expr),
    );
    for candidate in route_edits
        .replacement_routes
        .iter()
        .filter(|candidate| selected.contains(&candidate.connection))
    {
        output.extend(
            candidate
                .supplemental_segments
                .iter()
                .map(supplemental_segment_expr),
        );
        output.extend(
            candidate
                .supplemental_vias
                .iter()
                .map(supplemental_via_expr),
        );
    }
    *items = output;
    Ok(root)
}

fn supplemental_via_expr(via: &SupplementalVia) -> Expr {
    let identity = format!(
        "pcb-maker:supplemental-via:{}:{:.9}:{:.9}:{:.9}:{:.9}:{}:{}",
        via.connection, via.at[0], via.at[1], via.size, via.drill, via.layers[0], via.layers[1]
    );
    Expr::List(vec![
        Expr::Atom("via".into()),
        coordinate_form("at", via.at),
        Expr::List(vec![
            Expr::Atom("size".into()),
            Expr::Atom(via.size.to_string()),
        ]),
        Expr::List(vec![
            Expr::Atom("drill".into()),
            Expr::Atom(via.drill.to_string()),
        ]),
        Expr::List(vec![
            Expr::Atom("layers".into()),
            Expr::Atom(quote(&via.layers[0])),
            Expr::Atom(quote(&via.layers[1])),
        ]),
        net_expr(&format!("/{}", via.connection)),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(quote(&deterministic_uuid_for(&identity))),
        ]),
    ])
}

/// Connection IDs omit one hierarchical slash, but native net names do not.
/// Pads identify the exact spelling to use for emitted/retained copper. Reject
/// aliases with two distinct pad nets rather than silently shorting them.
fn unambiguous_pad_net_names(pcb: &Expr) -> Result<BTreeMap<String, Expr>, String> {
    let mut names: BTreeMap<String, Expr> = BTreeMap::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|n| n.head() == Some("footprint"))
    {
        for pad in footprint
            .children()
            .iter()
            .filter(|n| n.head() == Some("pad"))
        {
            let Some(name) = node_net(pad).filter(|name| !name.is_empty()) else {
                continue;
            };
            let connection = normalize_net(name).to_string();
            let atom = pad.child("net").unwrap().children().last().unwrap().clone();
            if let Some(previous) = names.get(&connection) {
                if previous.atom() != Some(name) {
                    return Err(format!(
                        "ambiguous native net identity {connection:?}: pads name both {:?} and {name:?}",
                        previous.atom().unwrap()
                    ));
                }
            } else {
                names.insert(connection, atom);
            }
        }
    }
    Ok(names)
}

fn canonicalize_copper_net_names(pcb: &mut Expr, names: &BTreeMap<String, Expr>) {
    let Expr::List(items) = pcb else {
        return;
    };
    for item in items {
        if !matches!(item.head(), Some("segment" | "arc" | "via" | "zone")) {
            continue;
        }
        let Some(name) = node_net(item)
            .and_then(|name| names.get(normalize_net(name)))
            .cloned()
        else {
            continue;
        };
        let Expr::List(parts) = item else {
            continue;
        };
        for part in parts {
            if let Some(head @ ("net" | "net_name")) = part.head() {
                // Preserve the source atom, including escaping, rather than
                // quoting its already escaped contents a second time.
                *part = Expr::List(vec![Expr::Atom(head.into()), name.clone()]);
            }
        }
    }
}

fn supplemental_segment_expr(segment: &SupplementalSegment) -> Expr {
    let identity = format!(
        "pcb-maker:supplemental:{}:{:.9}:{:.9}:{:.9}:{:.9}:{:.9}:{}",
        segment.connection,
        segment.start[0],
        segment.start[1],
        segment.end[0],
        segment.end[1],
        segment.width,
        segment.layer
    );
    Expr::List(vec![
        Expr::Atom("segment".into()),
        coordinate_form("start", segment.start),
        coordinate_form("end", segment.end),
        Expr::List(vec![
            Expr::Atom("width".into()),
            Expr::Atom(segment.width.to_string()),
        ]),
        string_form("layer", &segment.layer),
        net_expr(&format!("/{}", segment.connection)),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(quote(&deterministic_uuid_for(&identity))),
        ]),
    ])
}

fn coordinate_form(head: &str, coordinate: [f64; 2]) -> Expr {
    Expr::List(vec![
        Expr::Atom(head.into()),
        Expr::Atom(coordinate[0].to_string()),
        Expr::Atom(coordinate[1].to_string()),
    ])
}

fn apply_route_vertex_overrides(
    item: &mut Expr,
    route_vertex_overrides: &[RouteVertexOverride],
) -> Result<(), String> {
    if item.head() != Some("segment") {
        return Ok(());
    }
    let net = node_net(item)
        .map(normalize_net)
        .ok_or_else(|| "selected segment has no named net".to_string())?
        .to_string();
    let Expr::List(parts) = item else {
        return Ok(());
    };
    for route_override in route_vertex_overrides
        .iter()
        .filter(|route_override| route_override.connection == net)
    {
        for endpoint in parts
            .iter_mut()
            .filter(|part| matches!(part.head(), Some("start" | "end")))
        {
            let Expr::List(coordinates) = endpoint else {
                continue;
            };
            let Some(x) = coordinates
                .get(1)
                .and_then(Expr::atom)
                .and_then(|value| value.parse::<f64>().ok())
            else {
                continue;
            };
            let Some(y) = coordinates
                .get(2)
                .and_then(Expr::atom)
                .and_then(|value| value.parse::<f64>().ok())
            else {
                continue;
            };
            if (x - route_override.from[0]).abs() <= 1e-9
                && (y - route_override.from[1]).abs() <= 1e-9
            {
                coordinates[1] = Expr::Atom(route_override.to[0].to_string());
                coordinates[2] = Expr::Atom(route_override.to[1].to_string());
            }
        }
    }
    Ok(())
}

fn normalize_net(net: &str) -> &str {
    net.strip_prefix('/').unwrap_or(net)
}

fn node_net(node: &Expr) -> Option<&str> {
    net_name_from_form(node.child("net")?)
}

fn assign_pad_nets(
    node: &mut Expr,
    selected: &BTreeSet<String>,
    pad_nets: &BTreeMap<(String, String), PadInfo>,
) {
    let reference = footprint_reference(node).unwrap_or_default();
    let Expr::List(items) = node else { return };
    for item in items.iter_mut() {
        if item.head() == Some("pad") {
            let Expr::List(pad_items) = item else {
                continue;
            };
            let existing_net = pad_items
                .iter()
                .find(|part| part.head() == Some("net"))
                .and_then(node_net_from_form)
                .map(str::to_string);
            pad_items
                .retain(|part| !matches!(part.head(), Some("net" | "pinfunction" | "pintype")));
            if let Some(net) = existing_net.filter(|net| selected.contains(normalize_net(net))) {
                pad_items.push(net_expr(&net));
            } else if !reference.is_empty()
                && let Some(number) = pad_items.get(1).and_then(Expr::atom)
                && !number.is_empty()
                && let Some(info) = pad_nets.get(&(reference.clone(), number.to_string()))
            {
                pad_items.push(net_expr(&info.net));
            }
            if !reference.is_empty()
                && let Some(number) = pad_items.get(1).and_then(Expr::atom)
                && let Some(info) = pad_nets.get(&(reference.clone(), number.to_string()))
            {
                if !info.pin_function.is_empty() {
                    pad_items.push(string_form("pinfunction", &info.pin_function));
                }
                if !info.pin_type.is_empty() {
                    pad_items.push(string_form("pintype", &info.pin_type));
                }
            }
        }
    }
}

fn net_expr(name: &str) -> Expr {
    Expr::List(vec![
        Expr::Atom("net".into()),
        Expr::Atom(format!("\"{name}\"")),
    ])
}

fn string_form(head: &str, value: &str) -> Expr {
    Expr::List(vec![Expr::Atom(head.into()), Expr::Atom(quote(value))])
}

fn export_netlist(schematic: &Path, output: &Path) -> Result<(), String> {
    let result = Command::new("kicad-cli")
        .args([
            "sch",
            "export",
            "netlist",
            "--format",
            "kicadsexpr",
            "--output",
        ])
        .arg(output)
        .arg(schematic)
        .output()
        .map_err(|error| format!("failed to run kicad-cli netlist export: {error}"))?;
    if !result.status.success() {
        return Err(format!(
            "kicad-cli netlist export failed with {}: {}{}",
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct PadInfo {
    net: String,
    pin_function: String,
    pin_type: String,
}

fn read_pad_nets(path: &Path) -> Result<BTreeMap<(String, String), PadInfo>, String> {
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read exported netlist {}: {error}",
            path.display()
        )
    })?;
    let root = parse(&source)?;
    let nets = root
        .child("nets")
        .ok_or_else(|| "exported KiCad netlist has no nets section".to_string())?;
    let mut result = BTreeMap::new();
    for net in nets
        .children()
        .iter()
        .filter(|item| item.head() == Some("net"))
    {
        let Some(name) = net
            .child("name")
            .and_then(|form| form.children().get(1))
            .and_then(Expr::atom)
        else {
            continue;
        };
        for node in net
            .children()
            .iter()
            .filter(|item| item.head() == Some("node"))
        {
            let Some(reference) = node
                .child("ref")
                .and_then(|form| form.children().get(1))
                .and_then(Expr::atom)
            else {
                continue;
            };
            let Some(pin) = node
                .child("pin")
                .and_then(|form| form.children().get(1))
                .and_then(Expr::atom)
            else {
                continue;
            };
            let pin_function = node
                .child("pinfunction")
                .and_then(|form| form.children().get(1))
                .and_then(Expr::atom)
                .unwrap_or_default();
            let pin_type = node
                .child("pintype")
                .and_then(|form| form.children().get(1))
                .and_then(Expr::atom)
                .unwrap_or_default();
            result.insert(
                (reference.to_string(), pin.to_string()),
                PadInfo {
                    net: name.to_string(),
                    pin_function: pin_function.to_string(),
                    pin_type: pin_type.to_string(),
                },
            );
        }
    }
    Ok(result)
}

fn footprint_reference(node: &Expr) -> Option<String> {
    node.children().iter().find_map(|item| {
        if item.head() != Some("property") || item.children().get(1)?.atom()? != "Reference" {
            return None;
        }
        Some(item.children().get(2)?.atom()?.to_string())
    })
}

fn schematic_component_fields(root: &Expr) -> BTreeMap<String, BTreeMap<String, String>> {
    root.children()
        .iter()
        .filter(|item| item.head() == Some("symbol"))
        .filter_map(|symbol| {
            let fields: BTreeMap<String, String> = symbol
                .children()
                .iter()
                .filter_map(|item| {
                    if item.head() != Some("property") {
                        return None;
                    }
                    Some((
                        item.children().get(1)?.atom()?.to_string(),
                        item.children().get(2)?.atom()?.to_string(),
                    ))
                })
                .collect();
            Some((fields.get("Reference")?.clone(), fields))
        })
        .collect()
}

fn prepare_footprint_for_parity(
    footprint: &mut Expr,
    schematic_fields: &BTreeMap<String, BTreeMap<String, String>>,
) {
    let reference = footprint_reference(footprint).unwrap_or_default();
    let Expr::List(items) = footprint else { return };
    if reference.is_empty()
        && let Some(Expr::List(attributes)) =
            items.iter_mut().find(|item| item.head() == Some("attr"))
        && !attributes
            .iter()
            .any(|item| item.atom() == Some("board_only"))
    {
        attributes.push(Expr::Atom("board_only".into()));
    }
    if let Some(fields) = schematic_fields.get(&reference) {
        for item in items
            .iter_mut()
            .filter(|item| item.head() == Some("property"))
        {
            let Expr::List(property) = item else {
                continue;
            };
            let Some(name) = property.get(1).and_then(Expr::atom) else {
                continue;
            };
            let Some(value) = fields.get(name) else {
                continue;
            };
            if let Some(destination) = property.get_mut(2) {
                *destination = Expr::Atom(quote(value));
            }
        }
    }

    let mut pad_counts = BTreeMap::<String, usize>::new();
    for pad in items.iter().filter(|item| item.head() == Some("pad")) {
        if let Some(number) = pad.children().get(1).and_then(Expr::atom) {
            *pad_counts.entry(number.to_string()).or_default() += 1;
        }
    }
    if pad_counts.values().any(|count| *count > 1)
        && let Some(Expr::List(setting)) = items
            .iter_mut()
            .find(|item| item.head() == Some("duplicate_pad_numbers_are_jumpers"))
        && let Some(value) = setting.get_mut(1)
    {
        *value = Expr::Atom("yes".into());
    }
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn node_net_from_form(node: &Expr) -> Option<&str> {
    net_name_from_form(node)
}

fn net_name_from_form(node: &Expr) -> Option<&str> {
    if node.head() != Some("net") {
        return None;
    }
    let fields = node.children();
    match fields.len() {
        3.. => fields.get(2)?.atom(),
        2 => fields.get(1)?.atom(),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Point(i64, i64);

fn filter_schematic(mut root: Expr, selected: &BTreeSet<String>) -> Result<Expr, String> {
    let Expr::List(items) = &mut root else {
        return Err("schematic root is not a list".into());
    };
    if root_head(items) != Some("kicad_sch") {
        return Err("expected kicad_sch root".into());
    }

    let wires: Vec<(usize, Point, Point)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| wire_points(item).map(|(a, b)| (index, a, b)))
        .collect();
    let labels: Vec<(usize, String, Point)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| label_data(item).map(|(name, at)| (index, name, at)))
        .collect();
    let all_label_points: BTreeSet<Point> = labels.iter().map(|(_, _, point)| *point).collect();
    let hidden_label_points: BTreeSet<Point> = items
        .iter()
        .filter(|item| {
            label_data(item).is_some()
                && item
                    .child("effects")
                    .and_then(|effects| effects.child("hide"))
                    .is_some()
        })
        .filter_map(at_point)
        .collect();
    let mut incident: BTreeMap<Point, Vec<usize>> = BTreeMap::new();
    for (wire_index, a, b) in &wires {
        incident.entry(*a).or_default().push(*wire_index);
        incident.entry(*b).or_default().push(*wire_index);
    }

    let mut retained_wires = BTreeSet::new();
    let mut queue = VecDeque::new();
    for (_, name, point) in &labels {
        if selected.contains(name) {
            queue.push_back(*point);
        }
    }
    while let Some(point) = queue.pop_front() {
        for wire_index in incident.get(&point).into_iter().flatten() {
            if !retained_wires.insert(*wire_index) {
                continue;
            }
            let (_, a, b) = wires
                .iter()
                .find(|(index, _, _)| index == wire_index)
                .expect("indexed wire");
            queue.push_back(*a);
            queue.push_back(*b);
        }
    }

    let retained_points: BTreeSet<Point> = retained_wires
        .iter()
        .flat_map(|index| {
            let (_, a, b) = wires
                .iter()
                .find(|(wire_index, _, _)| wire_index == index)
                .expect("indexed wire");
            [*a, *b]
        })
        .collect();
    let existing_no_connects: BTreeSet<Point> = items.iter().filter_map(no_connect_point).collect();
    let pin_points: BTreeSet<Point> = incident
        .iter()
        .filter_map(|(point, edges)| {
            (edges.len() == 1
                && (!all_label_points.contains(point) || hidden_label_points.contains(point)))
            .then_some(*point)
        })
        .collect();

    let mut output = Vec::with_capacity(items.len() + pin_points.len());
    for (index, item) in std::mem::take(items).into_iter().enumerate() {
        match item.head() {
            Some("wire") if retained_wires.contains(&index) => output.push(item),
            Some("wire") => {}
            Some("label" | "global_label" | "hierarchical_label") => {
                if label_data(&item).is_some_and(|(name, _)| selected.contains(&name)) {
                    output.push(item);
                }
            }
            Some("junction") => {
                if at_point(&item).is_some_and(|point| retained_points.contains(&point)) {
                    output.push(item);
                }
            }
            _ => output.push(item),
        }
    }
    for point in pin_points {
        if retained_points.contains(&point) || existing_no_connects.contains(&point) {
            continue;
        }
        output.push(no_connect_expr(point));
    }
    *items = output;
    Ok(root)
}

fn wire_points(node: &Expr) -> Option<(Point, Point)> {
    if node.head() != Some("wire") {
        return None;
    }
    let points: Vec<Point> = node
        .child("pts")?
        .children()
        .iter()
        .filter_map(xy_point)
        .collect();
    (points.len() == 2).then(|| (points[0], points[1]))
}

fn xy_point(node: &Expr) -> Option<Point> {
    if node.head() != Some("xy") {
        return None;
    }
    point_from_numbers(
        node.children().get(1)?.atom()?,
        node.children().get(2)?.atom()?,
    )
}

fn at_point(node: &Expr) -> Option<Point> {
    let at = node.child("at")?;
    point_from_numbers(at.children().get(1)?.atom()?, at.children().get(2)?.atom()?)
}

fn point_from_numbers(x: &str, y: &str) -> Option<Point> {
    Some(Point(
        (x.parse::<f64>().ok()? * 1_000_000.0).round() as i64,
        (y.parse::<f64>().ok()? * 1_000_000.0).round() as i64,
    ))
}

fn label_data(node: &Expr) -> Option<(String, Point)> {
    if !matches!(
        node.head(),
        Some("label" | "global_label" | "hierarchical_label")
    ) {
        return None;
    }
    Some((node.children().get(1)?.atom()?.to_string(), at_point(node)?))
}

fn no_connect_point(node: &Expr) -> Option<Point> {
    (node.head() == Some("no_connect"))
        .then(|| at_point(node))
        .flatten()
}

fn no_connect_expr(point: Point) -> Expr {
    let x = format_number(point.0);
    let y = format_number(point.1);
    Expr::List(vec![
        Expr::Atom("no_connect".into()),
        Expr::List(vec![Expr::Atom("at".into()), Expr::Atom(x), Expr::Atom(y)]),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(format!("\"{}\"", deterministic_uuid(point))),
        ]),
    ])
}

fn format_number(value: i64) -> String {
    let integer = value / 1_000_000;
    let fraction = value.unsigned_abs() % 1_000_000;
    if fraction == 0 {
        integer.to_string()
    } else {
        format!("{integer}.{fraction:06}")
            .trim_end_matches('0')
            .to_string()
    }
}

fn deterministic_uuid(point: Point) -> String {
    deterministic_uuid_for(&format!("pcb-maker:no-connect:{},{}", point.0, point.1))
}

fn deterministic_uuid_for(identity: &str) -> String {
    let digest = Sha256::digest(identity);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn count_head(root: &Expr, head: &str) -> usize {
    root.children()
        .iter()
        .filter(|item| item.head() == Some(head))
        .count()
}

fn write_project_libraries(schematic: &Expr, pcb: &Expr, directory: &Path) -> Result<(), String> {
    let lib_symbols = schematic
        .child("lib_symbols")
        .ok_or_else(|| "schematic has no embedded lib_symbols".to_string())?;
    let mut symbol_library = vec![
        Expr::Atom("kicad_symbol_lib".into()),
        Expr::List(vec![
            Expr::Atom("version".into()),
            Expr::Atom("20231120".into()),
        ]),
        Expr::List(vec![
            Expr::Atom("generator".into()),
            Expr::Atom("\"pcb-maker\"".into()),
        ]),
        Expr::List(vec![
            Expr::Atom("generator_version".into()),
            Expr::Atom("\"0.1\"".into()),
        ]),
    ];
    symbol_library.extend(
        lib_symbols
            .children()
            .iter()
            .filter(|item| item.head() == Some("symbol"))
            .cloned(),
    );
    fs::write(
        directory.join("Generated.kicad_sym"),
        format!("{}\n", encode(&Expr::List(symbol_library))),
    )
    .map_err(|error| format!("failed to write symbol library: {error}"))?;
    fs::write(
        directory.join("sym-lib-table"),
        "(sym_lib_table (version 7) (lib (name \"Generated\")(type \"KiCad\")(uri \"${KIPRJMOD}/Generated.kicad_sym\")(options \"\")(descr \"Generated by pcb-maker\")))\n",
    )
    .map_err(|error| format!("failed to write symbol library table: {error}"))?;

    let footprint_directory = directory.join("Generated.pretty");
    fs::create_dir_all(&footprint_directory)
        .map_err(|error| format!("failed to create footprint library: {error}"))?;
    let mut written = BTreeSet::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let Some(link) = footprint.children().get(1).and_then(Expr::atom) else {
            continue;
        };
        let Some(name) = link.strip_prefix("Generated:") else {
            continue;
        };
        if !written.insert(name.to_string()) {
            continue;
        }
        let mut library_footprint = footprint.clone();
        clean_library_footprint(&mut library_footprint, name);
        fs::write(
            footprint_directory.join(format!("{name}.kicad_mod")),
            format!("{}\n", encode(&library_footprint)),
        )
        .map_err(|error| format!("failed to write footprint {name}: {error}"))?;
    }
    fs::write(
        directory.join("fp-lib-table"),
        "(fp_lib_table (version 7) (lib (name \"Generated\")(type \"KiCad\")(uri \"${KIPRJMOD}/Generated.pretty\")(options \"\")(descr \"Generated by pcb-maker\")))\n",
    )
    .map_err(|error| format!("failed to write footprint library table: {error}"))?;
    Ok(())
}

fn clean_library_footprint(footprint: &mut Expr, name: &str) {
    let Expr::List(items) = footprint else { return };
    if let Some(value) = items.get_mut(1) {
        *value = Expr::Atom(format!("\"{name}\""));
    }
    items.retain(|item| !matches!(item.head(), Some("at" | "path" | "uuid")));
    for item in items {
        if item.head() == Some("pad") {
            let Expr::List(parts) = item else { continue };
            parts.retain(|part| part.head() != Some("net"));
        }
    }
}

fn root_head(items: &[Expr]) -> Option<&str> {
    items.first()?.atom()
}

fn parse(source: &str) -> Result<Expr, String> {
    let mut parser = Parser { source, offset: 0 };
    let expression = parser.expression()?;
    parser.whitespace();
    if parser.offset != source.len() {
        return Err(format!("trailing input at byte {}", parser.offset));
    }
    Ok(expression)
}

struct Parser<'a> {
    source: &'a str,
    offset: usize,
}

impl Parser<'_> {
    fn expression(&mut self) -> Result<Expr, String> {
        self.whitespace();
        match self.peek() {
            Some('(') => self.list(),
            Some(_) => self.atom(),
            None => Err("unexpected end of input".into()),
        }
    }

    fn list(&mut self) -> Result<Expr, String> {
        self.bump();
        let mut items = Vec::new();
        loop {
            self.whitespace();
            match self.peek() {
                Some(')') => {
                    self.bump();
                    return Ok(Expr::List(items));
                }
                Some(_) => items.push(self.expression()?),
                None => return Err("unterminated list".into()),
            }
        }
    }

    fn atom(&mut self) -> Result<Expr, String> {
        let start = self.offset;
        if self.peek() == Some('"') {
            self.bump();
            let mut escaped = false;
            while let Some(character) = self.bump() {
                if character == '"' && !escaped {
                    return Ok(Expr::Atom(self.source[start..self.offset].to_string()));
                }
                escaped = character == '\\' && !escaped;
                if character != '\\' {
                    escaped = false;
                }
            }
            return Err("unterminated string".into());
        }
        while let Some(character) = self.peek() {
            if character.is_whitespace() || matches!(character, '(' | ')') {
                break;
            }
            self.bump();
        }
        if self.offset == start {
            Err(format!("expected atom at byte {}", self.offset))
        } else {
            Ok(Expr::Atom(self.source[start..self.offset].to_string()))
        }
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.offset += character.len_utf8();
        Some(character)
    }
}

fn unquote(atom: &str) -> &str {
    atom.strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(atom)
}

fn encode(expression: &Expr) -> String {
    let mut output = String::new();
    encode_into(expression, 0, &mut output);
    output
}

fn encode_into(expression: &Expr, depth: usize, output: &mut String) {
    match expression {
        Expr::Atom(atom) => output.push_str(atom),
        Expr::List(items) => {
            output.push('(');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    if matches!(item, Expr::List(_)) {
                        output.push('\n');
                        output.push_str(&"  ".repeat(depth + 1));
                    } else {
                        output.push(' ');
                    }
                }
                encode_into(item, depth + 1, output);
            }
            output.push(')');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_cache_hashes_design_inputs_and_ignores_reports() {
        assert_eq!(kicad_verifier_implementation_sha256().len(), 64);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-kicad-cache-input-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("board.kicad_pcb"), "(kicad_pcb)\n").unwrap();
        fs::write(directory.join("board.kicad_pro"), "{}\n").unwrap();
        fs::write(
            directory.join("sym-lib-table"),
            r#"(sym_lib_table (lib (uri "${KIPRJMOD}/Generated.kicad_sym")))"#,
        )
        .unwrap();
        let digest = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        fs::write(directory.join("erc.json"), "generated report\n").unwrap();
        let after_report = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        assert_eq!(digest, after_report);
        fs::write(directory.join("board.kicad_pcb"), "(kicad_pcb (zone))\n").unwrap();
        let after_board = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        assert_ne!(digest, after_board);

        fs::write(
            directory.join("fp-lib-table"),
            r#"(fp_lib_table (lib (uri "/external/library.pretty")))"#,
        )
        .unwrap();
        assert!(matches!(
            kicad_cache_input_sha256(&directory).unwrap(),
            KiCadCacheInput::Bypassed(reason) if reason.contains("external library")
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn route_only_erc_digest_covers_subsheets_but_ignores_copper() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-kicad-erc-input-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("board.kicad_sch"), "root\n").unwrap();
        fs::write(directory.join("child.kicad_sch"), "child\n").unwrap();
        fs::write(directory.join("board.kicad_pro"), "{}\n").unwrap();
        fs::write(directory.join("board.kicad_pcb"), "copper-a\n").unwrap();
        let digest = kicad_erc_input_sha256(&directory).unwrap();
        fs::write(directory.join("board.kicad_pcb"), "copper-b\n").unwrap();
        assert_eq!(digest, kicad_erc_input_sha256(&directory).unwrap());
        fs::write(directory.join("child.kicad_sch"), "changed-child\n").unwrap();
        assert_ne!(digest, kicad_erc_input_sha256(&directory).unwrap());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn route_only_erc_guard_rejects_changed_schematic_project_and_wrong_report() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-route-only-erc-guard-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("board.kicad_sch"), "root\n").unwrap();
        fs::write(directory.join("child.kicad_sch"), "child\n").unwrap();
        fs::write(directory.join("board.kicad_pro"), "{}\n").unwrap();
        fs::write(directory.join("board.kicad_pcb"), "copper-a\n").unwrap();
        let digest = kicad_erc_input_sha256(&directory).unwrap();
        let report = serde_json::json!({
            "$schema": "https://schemas.kicad.org/erc.v1.json",
            "source": "board.kicad_sch", "date": "2026-09-09T00:00:00",
            "kicad_version": "10.0.6", "coordinate_units": "mm",
            "included_severities": ["error", "warning", "exclusion"],
            "sheets": [{"path": "/", "uuid_path": "/root", "violations": []}]
        });
        let check = |report: &serde_json::Value| {
            validate_route_only_erc(&directory, "board", &digest, report)
        };
        assert!(check(&report).is_ok());
        fs::write(directory.join("board.kicad_pcb"), "routed-copper\n").unwrap();
        assert!(check(&report).is_ok());
        for (file, original) in [
            ("board.kicad_sch", "root\n"),
            ("child.kicad_sch", "child\n"),
            ("board.kicad_pro", "{}\n"),
        ] {
            fs::write(directory.join(file), format!("{original}changed\n")).unwrap();
            assert!(
                check(&report).unwrap_err().contains("expected ERC inputs"),
                "{file}"
            );
            fs::write(directory.join(file), original).unwrap();
            assert!(check(&report).is_ok());
        }
        let mut wrong = report.clone();
        wrong["source"] = serde_json::json!("other.kicad_sch");
        assert!(check(&wrong).is_err());
        assert!(check(&serde_json::json!({})).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn board_statistics_find_declared_and_name_addressed_nets() {
        let pcb = parse(
            r#"(kicad_pcb
                (net 0 "")
                (net 1 "SIGNAL")
                (footprint "test" (at 0 0)
                  (property "Reference" "J1")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net 1 "SIGNAL"))
                  (pad "2" smd rect (at 0 1) (size 1 1)
                    (layers "F.Cu") (net "unconnected-(J1-Pad2)")))
                (segment (start 0 0) (end 3 4) (width 0.25)
                  (layer "F.Cu") (net 1))
                (via (at 3 4) (size 0.7) (drill 0.3)
                  (layers "F.Cu" "B.Cu") (net "SIGNAL")))"#,
        )
        .unwrap();

        let statistics = board_statistics(&pcb).unwrap();
        assert_eq!(statistics.footprints, 1);
        assert_eq!(statistics.total_named_nets_including_no_connects, 2);
        assert_eq!(statistics.segments, 1);
        assert_eq!(statistics.vias, 1);
        assert_eq!(statistics.stored_segment_length_mm, 5.0);
        assert_eq!(
            statistics
                .physical_copper
                .canonical_straight_track_length_mm,
            5.0
        );
        assert_eq!(statistics.physical_copper.used_copper_layers, 2);
        assert!(statistics.physical_copper.centerline_union_exact);
        assert!(
            (statistics
                .physical_copper
                .centerline_weighted_track_area_mm2
                - 1.25)
                .abs()
                < 1.0e-12
        );
        assert_eq!(statistics.copper_geometry_sha256.len(), 64);
        assert_eq!(statistics.component_placement_sha256.len(), 64);
        assert_eq!(statistics.component_placement_100nm_sha256.len(), 64);
    }

    #[test]
    fn cold_board_rewrite_removes_copper_but_retains_rule_areas_and_placement() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-strip-copper-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        fs::write(
            &board,
            r#"(kicad_pcb
                (net 0 "")
                (net 1 "SIGNAL")
                (footprint "test" (at 12 34 90)
                  (property "Reference" "J1")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net 1 "SIGNAL")))
                (segment (start 12 34) (end 14 34) (width 0.25)
                  (layer "F.Cu") (net 1))
                (arc (start 14 34) (mid 15 35) (end 14 36) (width 0.25)
                  (layer "F.Cu") (net 1))
                (via (at 14 36) (size 0.7) (drill 0.3)
                  (layers "F.Cu" "B.Cu") (net 1))
                (gr_text "COPPER" (at 8 8) (layer "F.Cu"))
                (gr_text "SILK" (at 8 9) (layer "F.SilkS"))
                (zone (net 1) (net_name "SIGNAL") (layer "B.Cu")
                  (polygon (pts (xy 0 0) (xy 2 0) (xy 2 2) (xy 0 2))))
                (zone (net 0) (net_name "") (layers "F&B.Cu")
                  (keepout (tracks not_allowed) (vias not_allowed))
                  (polygon (pts (xy 4 4) (xy 6 4) (xy 6 6) (xy 4 6)))))"#,
        )
        .unwrap();

        let report = write_kicad_board_without_copper(&board, &board).unwrap();
        assert_eq!(report.removed_segments, 1);
        assert_eq!(report.removed_arcs, 1);
        assert_eq!(report.removed_vias, 1);
        assert_eq!(report.removed_copper_zones, 1);
        assert_eq!(report.removed_copper_graphics, 1);
        assert_eq!(report.retained_rule_areas, 1);
        assert_eq!(report.before.zones, 2);
        assert_eq!(report.before.copper_zones, 1);
        assert_eq!(report.before.rule_areas, 1);
        assert_eq!(report.after.zones, 1);
        assert_eq!(report.after.copper_zones, 0);
        assert_eq!(report.after.rule_areas, 1);
        assert_eq!(report.before.copper_graphics, 1);
        assert_eq!(report.after.copper_graphics, 0);
        assert_eq!(report.before.footprints, report.after.footprints);
        assert_eq!(
            report.before.component_placement_sha256,
            report.after.component_placement_sha256
        );
        let stripped = fs::read_to_string(&board).unwrap();
        assert!(stripped.contains("(keepout"));
        assert!(stripped.contains("SILK"));
        assert!(!stripped.contains("COPPER"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn progressive_materialization_retains_netless_rule_areas() {
        let pcb = parse(
            r#"(kicad_pcb
                (net 0 "")
                (net 1 "SIGNAL")
                (zone (net 0) (net_name "") (layers "F&B.Cu")
                  (keepout (tracks not_allowed) (vias not_allowed))
                  (polygon (pts (xy 4 4) (xy 6 4) (xy 6 6) (xy 4 6))))
                (zone (net 1) (net_name "SIGNAL") (layer "F.Cu")
                  (polygon (pts (xy 0 0) (xy 2 0) (xy 2 2) (xy 0 2)))))"#,
        )
        .unwrap();
        let selected = BTreeSet::new();
        let schematic_fields = BTreeMap::new();
        let pad_nets = BTreeMap::new();
        let segments = Vec::new();
        let vias = Vec::new();
        let candidates = Vec::new();
        let filtered = filter_pcb(
            pcb,
            &selected,
            &schematic_fields,
            &pad_nets,
            PcbRouteEdits {
                vertex_overrides: &[],
                supplemental_segments: &segments,
                supplemental_vias: &vias,
                replacement_routes: &candidates,
            },
        )
        .unwrap();
        let statistics = board_statistics(&filtered).unwrap();
        assert_eq!(statistics.zones, 1);
        assert_eq!(statistics.rule_areas, 1);
        assert_eq!(statistics.copper_zones, 0);
    }

    #[test]
    fn ordinary_numeric_kicad_nets_expose_names_and_routable_pad_counts() {
        let numeric = parse(r#"(segment (net 7 "SIGNAL"))"#).unwrap();
        assert_eq!(node_net(&numeric), Some("SIGNAL"));

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-connection-inventory-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        fs::write(
            &board,
            r#"(kicad_pcb
                (net 0 "")
                (net 2 "SECOND")
                (net 1 "FIRST")
                (footprint "a" (at 10 20 90)
                  (property "Reference" "A")
                  (pad "1" thru_hole circle (at 1 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net 2 "SECOND"))
                  (pad "2" thru_hole circle (at 1 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net 2 "SECOND"))
                  (pad "3" thru_hole circle (at 0 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net 1 "FIRST"))
                  (pad "4" thru_hole circle (at 2 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net "THIRD")))
                (footprint "b" (at 30 40)
                  (property "Reference" "B")
                  (pad "1" thru_hole circle (at 0 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net 2 "SECOND"))
                  (pad "2" thru_hole circle (at 3 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net "THIRD"))))"#,
        )
        .unwrap();

        assert_eq!(
            inspect_kicad_routable_connections(&board).unwrap(),
            vec![
                KiCadConnectionSummary {
                    connection: "SECOND".into(),
                    distinct_pad_centers: 2,
                    electrical_terminal_count: None,
                },
                KiCadConnectionSummary {
                    connection: "THIRD".into(),
                    distinct_pad_centers: 2,
                    electrical_terminal_count: None,
                }
            ]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn footprint_link_normalization_changes_only_schematic_library_links() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-footprint-link-normalization-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        let schematic = directory.join("board.kicad_sch");
        fs::write(
            &board,
            r#"(kicad_pcb
                (footprint "Local:R" (at 4 5 90)
                  (property "Reference" "R1")
                  (pad "1" thru_hole circle (at 0 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask")))
                (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        fs::write(
            &schematic,
            r#"(kicad_sch
                (symbol (lib_id "Device:R") (at 1 2 0)
                  (property "Reference" "R1")
                  (property "Footprint" "Device:R")))"#,
        )
        .unwrap();
        let board_before = fs::read(&board).unwrap();

        let report = normalize_kicad_footprint_links(&directory, "board").unwrap();

        assert_eq!(report.updated_references.len(), 1);
        assert_eq!(report.schema_version, 2);
        assert_eq!(report.schematic_files.len(), 1);
        assert_eq!(
            report.updated_references[0].schematic_path,
            "board.kicad_sch"
        );
        assert_eq!(report.updated_references[0].reference, "R1");
        assert_eq!(report.updated_references[0].schematic_before, "Device:R");
        assert_eq!(report.updated_references[0].board_link, "Local:R");
        assert_ne!(
            report.schematic_before_sha256,
            report.schematic_after_sha256
        );
        assert_eq!(fs::read(&board).unwrap(), board_before);
        assert!(
            fs::read_to_string(&schematic)
                .unwrap()
                .contains("(property \"Footprint\" \"Local:R\")")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn footprint_link_normalization_traverses_referenced_schematic_hierarchy() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-hierarchical-footprint-link-normalization-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("board.kicad_pcb"),
            r#"(kicad_pcb
                (footprint "Local:R" (at 4 5) (property "Reference" "R1"))
                (footprint "Local:C" (at 6 5) (property "Reference" "C2"))
                (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        fs::write(
            directory.join("board.kicad_sch"),
            r#"(kicad_sch
                (symbol (property "Reference" "R1") (property "Footprint" "Device:R"))
                (sheet (property "Sheetfile" "child.kicad_sch")))"#,
        )
        .unwrap();
        fs::write(
            directory.join("child.kicad_sch"),
            r#"(kicad_sch
                (symbol (property "Reference" "C2") (property "Footprint" "Device:C")))"#,
        )
        .unwrap();
        let board_before = fs::read(directory.join("board.kicad_pcb")).unwrap();

        let report = normalize_kicad_footprint_links(&directory, "board").unwrap();

        assert_eq!(report.updated_references.len(), 2);
        assert_eq!(report.schematic_files.len(), 2);
        assert_eq!(
            fs::read(directory.join("board.kicad_pcb")).unwrap(),
            board_before
        );
        assert!(
            fs::read_to_string(directory.join("child.kicad_sch"))
                .unwrap()
                .contains("(property \"Footprint\" \"Local:C\")")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn physical_copper_metrics_deduplicate_tracks_and_measure_layers() {
        let pcb = parse(
            r#"(kicad_pcb
                (layers
                  (0 "F.Cu" signal)
                  (31 "B.Cu" signal))
                (gr_rect (start 0 0) (end 10 5) (layer "Edge.Cuts"))
                (segment (start 0 0) (end 4 0) (width 0.2)
                  (layer "F.Cu") (net "SIGNAL"))
                (segment (start 0 0) (end 2 0) (width 0.4)
                  (layer "F.Cu") (net "SIGNAL"))
                (arc (start 1 0) (mid 0.7071067811865476 0.7071067811865476)
                  (end 0 1) (width 0.2) (layer "B.Cu") (net "ARC"))
                (via (at 2 0) (size 0.7) (drill 0.3)
                  (layers "F.Cu" "B.Cu") (net "SIGNAL"))
                (zone (net "PLANE") (layer "B.Cu")
                  (filled_polygon (layer "B.Cu")
                    (pts (xy 5 1) (xy 7 1) (xy 7 3) (xy 5 3)))))"#,
        )
        .unwrap();

        let statistics = board_statistics(&pcb).unwrap();
        let physical = &statistics.physical_copper;

        assert!((physical.canonical_straight_track_length_mm - 4.0).abs() < 1.0e-12);
        assert!((physical.overlapping_straight_track_length_mm - 2.0).abs() < 1.0e-12);
        assert!((physical.arc_track_length_mm - std::f64::consts::FRAC_PI_2).abs() < 1.0e-9);
        assert!(!physical.centerline_union_exact);
        assert!(
            (physical.centerline_weighted_track_area_mm2 - (1.2 + 0.1 * std::f64::consts::PI))
                .abs()
                < 1.0e-9
        );
        assert!((physical.filled_zone_area_mm2 - 4.0).abs() < 1.0e-12);
        assert_eq!(physical.board_outline_area_mm2, Some(50.0));
        assert_eq!(physical.used_copper_layers, 2);
        assert_eq!(physical.layers.len(), 2);
        assert_eq!(physical.layers[0].layer, "F.Cu");
        assert_eq!(physical.layers[0].vias_spanning_layer, 1);
        assert_eq!(physical.layers[1].layer, "B.Cu");
        assert_eq!(physical.layers[1].vias_spanning_layer, 1);
        assert!((physical.layers[1].filled_zone_area_mm2 - 4.0).abs() < 1.0e-12);
    }

    #[test]
    fn four_closed_edge_lines_define_the_same_rectangular_bounds_and_area() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_line (start 10 20) (end 30 20) (layer "Edge.Cuts"))
                (gr_line (start 30 40) (end 10 40) (layer "Edge.Cuts"))
                (gr_line (start 10 40) (end 10 20) (layer "Edge.Cuts"))
                (gr_line (start 30 20) (end 30 40) (layer "Edge.Cuts")))"#,
        )
        .unwrap();

        assert_eq!(
            rectangular_board_bounds(&pcb).unwrap(),
            Some([10.0, 20.0, 30.0, 40.0])
        );
        assert_eq!(rectangular_board_outline_area(&pcb).unwrap(), Some(400.0));
    }

    #[test]
    fn collinearly_split_edge_lines_still_define_a_rectangle() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_line (start 10 20) (end 17 20) (layer "Edge.Cuts"))
                (gr_line (start 17 20) (end 30 20) (layer "Edge.Cuts"))
                (gr_line (start 30 20) (end 30 40) (layer "Edge.Cuts"))
                (gr_line (start 30 40) (end 10 40) (layer "Edge.Cuts"))
                (gr_line (start 10 40) (end 10 20) (layer "Edge.Cuts")))"#,
        )
        .unwrap();

        assert_eq!(
            rectangular_board_bounds(&pcb).unwrap(),
            Some([10.0, 20.0, 30.0, 40.0])
        );
        assert_eq!(rectangular_board_outline_area(&pcb).unwrap(), Some(400.0));
    }

    #[test]
    fn non_convex_closed_line_outline_keeps_exact_area_and_blocks_corner_cutting() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts"))
                (gr_line (start 10 0) (end 10 10) (layer "Edge.Cuts"))
                (gr_line (start 10 10) (end 6 10) (layer "Edge.Cuts"))
                (gr_line (start 6 10) (end 6 14) (layer "Edge.Cuts"))
                (gr_line (start 6 14) (end 4 14) (layer "Edge.Cuts"))
                (gr_line (start 4 14) (end 4 10) (layer "Edge.Cuts"))
                (gr_line (start 4 10) (end 0 10) (layer "Edge.Cuts"))
                (gr_line (start 0 10) (end 0 0) (layer "Edge.Cuts")))"#,
        )
        .unwrap();

        let outline = board_outline(&pcb).unwrap().unwrap();
        assert_eq!(outline.bounds, [0.0, 0.0, 10.0, 14.0]);
        assert!((outline.area() - 108.0).abs() < 1.0e-12);
        assert_eq!(board_outline_area(&pcb).unwrap(), Some(108.0));
        assert_eq!(rectangular_board_bounds(&pcb).unwrap(), None);
        assert!(outline.contains_point_with_clearance([5.0, 12.0], 0.1));
        assert!(!outline.contains_point_with_clearance([2.0, 12.0], 0.1));
        assert!(outline.contains_segment_with_clearance([5.0, 9.0], [5.0, 12.0], 0.1));
        assert!(!outline.contains_segment_with_clearance([2.0, 9.0], [5.0, 12.0], 0.1));
    }

    #[test]
    fn copper_geometry_digest_ignores_object_identity_and_order() {
        let first = parse(
            r#"(kicad_pcb
                (segment (start 0 0) (end 1 0) (width 0.25)
                  (layer "F.Cu") (net "SIGNAL") (uuid "first-segment"))
                (via (at 1 0) (size 0.7) (drill 0.3)
                  (layers "F.Cu" "B.Cu") (net "SIGNAL") (uuid "first-via")))"#,
        )
        .unwrap();
        let second = parse(
            r#"(kicad_pcb
                (via (at 1 0) (size 0.7) (drill 0.3)
                  (layers "F.Cu" "B.Cu") (net "SIGNAL") (uuid "other-via"))
                (segment (start 0 0) (end 1 0) (width 0.25)
                  (layer "F.Cu") (net "SIGNAL") (uuid "other-segment")))"#,
        )
        .unwrap();

        assert_eq!(
            board_statistics(&first).unwrap().copper_geometry_sha256,
            board_statistics(&second).unwrap().copper_geometry_sha256
        );
    }

    #[test]
    fn component_placement_digest_ignores_order_but_detects_motion() {
        let board = |a_x: f64, reverse: bool| {
            let a = format!(
                r#"(footprint "test" (at {a_x} 2 15)
                    (property "Reference" "A") (uuid "a"))"#
            );
            let b = r#"(footprint "test" (at 3 4 90)
                    (property "Reference" "B") (uuid "b"))"#;
            parse(&format!(
                "(kicad_pcb {} {})",
                if reverse { b } else { &a },
                if reverse { &a } else { b }
            ))
            .unwrap()
        };
        let first = board(1.0, false);
        let reordered = board(1.0, true);
        let moved = board(1.1, false);

        let digest = |pcb| board_statistics(pcb).unwrap().component_placement_sha256;
        assert_eq!(digest(&first), digest(&reordered));
        assert_ne!(digest(&first), digest(&moved));
    }

    fn import_fixture(extra_copper: &str) -> Expr {
        parse(&format!(
            r#"(kicad_pcb
                (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
                (footprint "test" (at 2 10)
                  (property "Reference" "J1")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (footprint "test" (at 18 10)
                  (property "Reference" "J2")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (segment (start 2.4 10) (end 5 10) (width 0.25)
                  (layer "F.Cu") (net "/TARGET"))
                (via (at 5 10) (size 0.7) (drill 0.35)
                  (layers "F.Cu" "B.Cu") (net "/TARGET"))
                (segment (start 5 10) (end 15 10) (width 0.25)
                  (layer "B.Cu") (net "/TARGET"))
                (via (at 15 10) (size 0.7) (drill 0.35)
                  (layers "F.Cu" "B.Cu") (net "/TARGET"))
                (segment (start 15 10) (end 17.6 10) (width 0.25)
                  (layer "F.Cu") (net "/TARGET"))
                {extra_copper})"#
        ))
        .unwrap()
    }

    #[test]
    fn imports_simple_board_copper_chain_with_fixed_layer_transitions() {
        let candidate = import_route_candidate_from_expr(&import_fixture(""), "TARGET").unwrap();

        assert_eq!(candidate.router, "kicad-board-copper-tree-import-v2");
        assert_eq!(candidate.supplemental_segments.len(), 3);
        assert_eq!(candidate.supplemental_vias.len(), 2);
        assert_eq!(candidate.config.trace_width_mm, 0.25);
        assert_eq!(candidate.config.via_size_mm, 0.7);
        assert_eq!(candidate.config.via_drill_mm, 0.35);
        assert_eq!(
            candidate.branches[0]
                .path
                .iter()
                .map(|point| (point.at, point.layer.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ([2.4, 10.0], "F.Cu"),
                ([5.0, 10.0], "F.Cu"),
                ([5.0, 10.0], "B.Cu"),
                ([15.0, 10.0], "B.Cu"),
                ([15.0, 10.0], "F.Cu"),
                ([17.6, 10.0], "F.Cu"),
            ]
        );
    }

    #[test]
    fn imports_a_multilayer_tree_with_a_shared_via_junction() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
                (footprint "test" (at 2 10)
                  (property "Reference" "J1")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (footprint "test" (at 18 6)
                  (property "Reference" "J2")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (footprint "test" (at 18 14)
                  (property "Reference" "J3")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (segment (start 2.4 10) (end 8 10) (width 0.25)
                  (layer "F.Cu") (net "/TARGET"))
                (via (at 8 10) (size 0.7) (drill 0.35)
                  (layers "F.Cu" "B.Cu") (net "/TARGET"))
                (segment (start 8 10) (end 12 10) (width 0.25)
                  (layer "B.Cu") (net "/TARGET"))
                (via (at 12 10) (size 0.7) (drill 0.35)
                  (layers "F.Cu" "B.Cu") (net "/TARGET"))
                (segment (start 12 10) (end 17.6 6) (width 0.25)
                  (layer "F.Cu") (net "/TARGET"))
                (segment (start 12 10) (end 17.6 14) (width 0.25)
                  (layer "F.Cu") (net "/TARGET")))"#,
        )
        .unwrap();

        let candidate = import_route_candidate_from_expr(&pcb, "TARGET").unwrap();

        assert_eq!(candidate.terminals.len(), 3);
        assert_eq!(candidate.branches.len(), 2);
        assert_eq!(candidate.supplemental_segments.len(), 4);
        assert_eq!(candidate.supplemental_vias.len(), 2);
        assert_eq!(
            candidate.branches[0].path[..5],
            candidate.branches[1].path[..5]
        );
        assert_eq!(candidate.branches[0].path[4].at, [12.0, 10.0]);
        assert_eq!(candidate.branches[0].path[4].layer, "F.Cu");
    }

    #[test]
    fn pad_copper_offsets_rotate_independently_of_drill_anchors() {
        for (angle, expected) in [(0.0, [5.0, 5.4]), (90.0, [5.4, 5.0])] {
            let pcb = parse(&format!(
                r#"(kicad_pcb
                (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
                (footprint "test" (at 5 5) (property "Reference" "Q")
                    (pad "1" thru_hole rect (at 0 0 {angle}) (size 1.1 1.8)
                        (drill 0.75 (offset 0 0.4)) (layers "*.Cu" "*.Mask") (net "/TARGET")))
                (footprint "test" (at 15 15) (property "Reference" "J")
                    (pad "1" thru_hole circle (at 0 0) (size 2 2)
                        (drill 1) (layers "*.Cu" "*.Mask") (net "/TARGET"))))"#
            ))
            .unwrap();
            let model = KiCadRoutingModel::from_pcb(&pcb, "TARGET").unwrap();
            let pad = model
                .terminal_pads
                .iter()
                .find(|p| p.footprint == "Q")
                .unwrap();
            assert_eq!(pad.at, [5.0, 5.0]);
            let ObstacleGeometry::Rectangle { center, .. } = pad.geometry else {
                panic!("expected rectangle")
            };
            assert!(distance_squared(center, expected) < 1e-20);
            assert!(model.target_holes.iter().any(|h| matches!(h,
                ObstacleGeometry::Circle { center, radius } if *center == [5.0, 5.0] && *radius == 0.375)));
        }
    }

    #[test]
    fn imports_plated_pad_branches_without_inventing_vias_or_copper() {
        let pcb = parse(include_str!("../tests/fixtures/plated-branch.kicad_pcb")).unwrap();
        let candidate = import_route_candidate_from_expr(&pcb, "TARGET").unwrap();
        assert_eq!(candidate.terminals.len(), 3);
        assert_eq!(candidate.branches.len(), 2);
        assert_eq!(candidate.branches[0].finish_terminal, [8.0, 10.0]);
        assert_eq!(candidate.branches[1].start_terminal, [8.0, 10.0]);
        assert_eq!(candidate.branches[0].path.last().unwrap().at, [7.8, 10.0]);
        assert_eq!(candidate.branches[1].path[0].at, [8.2, 10.0]);
        assert_eq!(candidate.supplemental_vias.len(), 1);
        assert_eq!(candidate.supplemental_vias[0].at, [12.0, 10.0]);
        assert_eq!(candidate.supplemental_segments.len(), 3);
        let source = board_statistics(&pcb).unwrap();
        let quality = route_candidate_quality(&candidate).unwrap();
        assert_eq!(quality.vias, 1);
        assert!((quality.length_mm - source.stored_segment_length_mm).abs() < 1e-9);
        // Rematerialization by route actions must retain exactly the physical
        // copper, without needing an importer-specific flag or board context.
        let (segments, vias) =
            materialize_candidate_branches("TARGET", &candidate.branches, &candidate.config)
                .unwrap();
        assert_eq!(segments.len(), 3);
        assert_eq!(vias.len(), 1);
    }

    #[test]
    fn board_copper_import_does_not_bridge_coincident_smd_pads() {
        let source = include_str!("../tests/fixtures/plated-branch.kicad_pcb")
            .replace("thru_hole circle", "smd circle")
            .replace("(drill 0.8)", "")
            .replace("(layers \"*.Cu\" \"*.Mask\") (net \"/TARGET\")",
                "(layers \"F.Cu\") (net \"/TARGET\")) (pad \"2\" smd circle (at 0 0) (size 2 2) (layers \"B.Cu\") (net \"/TARGET\")");
        let error =
            import_route_candidate_from_expr(&parse(&source).unwrap(), "TARGET").unwrap_err();
        assert!(
            error.contains("target copper graph is disconnected"),
            "{error}"
        );
    }

    #[test]
    fn board_copper_import_rejects_a_cycle_completed_by_a_pad_barrel() {
        let source = include_str!("../tests/fixtures/plated-branch.kicad_pcb");
        let mut pcb = parse(source).unwrap();
        let edge = parse(
            r#"(segment (start 7.8 10) (end 12 10) (width 0.25) (layer "F.Cu") (net "/TARGET"))"#,
        )
        .unwrap();
        if let Expr::List(children) = &mut pcb {
            children.push(edge);
        }
        let error = import_route_candidate_from_expr(&pcb, "TARGET").unwrap_err();
        assert!(error.contains("cycle"), "{error}");
    }

    #[test]
    fn board_copper_import_rejects_a_dangling_branch() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
                (footprint "test" (at 2 10)
                  (property "Reference" "J1")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (footprint "test" (at 18 10)
                  (property "Reference" "J2")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (segment (start 2.4 10) (end 10 10) (width 0.25)
                  (layer "F.Cu") (net "/TARGET"))
                (segment (start 10 10) (end 17.6 10) (width 0.25)
                  (layer "F.Cu") (net "/TARGET"))
                (segment (start 10 10) (end 10 12) (width 0.25)
                  (layer "F.Cu") (net "/TARGET")))"#,
        )
        .unwrap();

        let error = import_route_candidate_from_expr(&pcb, "TARGET").unwrap_err();

        assert!(error.contains("dangling material"));
    }

    #[test]
    fn board_copper_import_rejects_a_cycle() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
                (footprint "test" (at 2 10)
                  (property "Reference" "J1")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (footprint "test" (at 18 10)
                  (property "Reference" "J2")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (segment (start 2.4 10) (end 2.4 8) (width 0.25)
                  (layer "F.Cu") (net "/TARGET"))
                (segment (start 2.4 8) (end 17.6 8) (width 0.25)
                  (layer "F.Cu") (net "/TARGET"))
                (segment (start 17.6 8) (end 17.6 10) (width 0.25)
                  (layer "F.Cu") (net "/TARGET"))
                (segment (start 2.4 10) (end 2.4 12) (width 0.25)
                  (layer "F.Cu") (net "/TARGET"))
                (segment (start 2.4 12) (end 17.6 12) (width 0.25)
                  (layer "F.Cu") (net "/TARGET"))
                (segment (start 17.6 12) (end 17.6 10) (width 0.25)
                  (layer "F.Cu") (net "/TARGET")))"#,
        )
        .unwrap();

        let error = import_route_candidate_from_expr(&pcb, "TARGET").unwrap_err();

        assert!(error.contains("cycle"));
    }

    #[test]
    fn overlapping_shared_point_groups_are_merged_for_via_junctions() {
        let point = |polyline: &str, point_index| CopperPointRef {
            polyline: polyline.into(),
            point_index,
        };
        let groups = merged_shared_points(vec![
            vec![point("front-a", 2), point("via-anchor", 0)],
            vec![point("via-anchor", 0), point("back-a", 0)],
            vec![point("front-a", 2), point("front-b", 1)],
        ]);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 4);
        assert_eq!(groups[0].members[0].polyline, "back-a");
        assert_eq!(groups[0].members[3].polyline, "via-anchor");
    }

    #[test]
    fn copper_application_uses_exact_pad_net_spelling() {
        let mut pcb = parse(
            r#"(kicad_pcb
            (footprint "test"
              (pad "1" smd (net "GND"))
              (pad "2" smd (net "/DATA"))
              (pad "3" smd (net "N\"Q")))
            (segment (net "/GND"))
            (via (net "/GND"))
            (arc (net "/DATA"))
            (zone (net "/GND") (net_name "/GND"))
            (segment (net "/N\"Q"))
            (segment (net "ORPHAN")))"#,
        )
        .unwrap();
        let names = unambiguous_pad_net_names(&pcb).unwrap();
        canonicalize_copper_net_names(&mut pcb, &names);
        let copper = pcb
            .children()
            .iter()
            .filter(|n| matches!(n.head(), Some("segment" | "via" | "arc" | "zone")))
            .collect::<Vec<_>>();
        assert_eq!(
            copper
                .iter()
                .map(|n| node_net(n).unwrap())
                .collect::<Vec<_>>(),
            vec!["GND", "GND", "/DATA", "GND", r#"N\"Q"#, "ORPHAN"]
        );
        assert_eq!(
            copper[3].child("net_name").unwrap().children()[1].atom(),
            Some("GND")
        );
        // Raw escaped atoms must survive a complete text round trip.
        let reparsed = parse(&encode(&pcb)).unwrap();
        assert_eq!(encode(&reparsed), encode(&pcb));
    }

    #[test]
    fn copper_application_rejects_colliding_normalized_pad_names() {
        let pcb = parse(
            r#"(kicad_pcb (footprint "test"
            (pad "1" smd (net "SIGNAL")) (pad "2" smd (net "/SIGNAL"))))"#,
        )
        .unwrap();
        assert!(
            unambiguous_pad_net_names(&pcb)
                .unwrap_err()
                .contains("ambiguous native net identity")
        );
    }

    #[test]
    fn parser_round_trips_strings_and_nested_lists() {
        let source = "(root (name \"a b\\\"c\") (xy 1.25 -2))";
        assert_eq!(
            parse(&encode(&parse(source).unwrap())).unwrap(),
            parse(source).unwrap()
        );
    }

    #[test]
    fn uuid_is_stable_and_well_formed() {
        assert_eq!(deterministic_uuid(Point(1, 2)).len(), 36);
        assert_eq!(
            deterministic_uuid(Point(1, 2)),
            deterministic_uuid(Point(1, 2))
        );
    }

    #[test]
    fn routing_model_uses_kicad_clockwise_footprint_rotation() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
                (footprint "test" (at 10 10 90)
                  (pad "1" smd rect (at -2 1 90) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (footprint "test" (at 15 15 -90)
                  (pad "1" smd rect (at -1 0 270) (size 1 1)
                    (layers "F.Cu") (net "/TARGET"))))"#,
        )
        .unwrap();
        let model = KiCadRoutingModel::from_pcb(&pcb, "TARGET").unwrap();
        assert_eq!(model.terminals, vec![[11.0, 12.0], [15.0, 14.0]]);
    }

    #[test]
    fn routing_model_treats_foreign_zones_as_yielding_but_keeps_their_vias() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
                (footprint "A" (at 2 2)
                  (pad "1" smd circle (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (footprint "B" (at 18 18)
                  (pad "1" smd circle (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (zone (net "/GND") (net_name "/GND") (layer "F.Cu"))
                (via (at 10 10) (size 0.8) (drill 0.4)
                  (layers "F.Cu" "B.Cu") (net "/GND")))"#,
        )
        .unwrap();
        let model = KiCadRoutingModel::from_pcb(&pcb, "TARGET").unwrap();
        assert_eq!(model.terminals, vec![[2.0, 2.0], [18.0, 18.0]]);
        assert_eq!(model.obstacles.len(), 1);
        assert_eq!(model.obstacles[0].object, "GND");
    }

    #[test]
    fn routing_model_lowers_track_blocking_rule_areas_as_fixed_obstacles() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
                (footprint "A" (at 2 10)
                  (pad "1" smd circle (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (footprint "B" (at 18 10)
                  (pad "1" smd circle (at 0 0) (size 1 1)
                    (layers "F.Cu") (net "/TARGET")))
                (zone (layer "F.Cu") (name "WALL")
                  (keepout (tracks not_allowed) (vias not_allowed))
                  (polygon (pts (xy 8 0) (xy 12 0) (xy 12 8) (xy 8 8)))))"#,
        )
        .unwrap();
        let model = KiCadRoutingModel::from_pcb(&pcb, "TARGET").unwrap();
        assert_eq!(model.obstacles.len(), 1);
        assert_eq!(
            model.obstacles[0].kind,
            KiCadViaLocalRerouteBlockerKind::RuleArea
        );
        assert_eq!(model.obstacles[0].layers, [true, false]);
        assert_eq!(model.obstacles[0].object, "WALL");
        assert!(model.obstacles[0].geometry.contains([10.0, 4.0], 0.0));
        assert!(!model.obstacles[0].geometry.contains([10.0, 10.0], 0.0));
    }

    #[test]
    fn exact_edge_checks_preserve_a_legal_narrow_rule_area_channel() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-exact-grid-edge-channel-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        fs::write(
            &board,
            r#"(kicad_pcb
                (gr_rect (start 10 10) (end 50 40) (layer "Edge.Cuts"))
                (footprint "test" (at 13 25)
                  (property "Reference" "J1")
                  (pad "1" smd circle (at 0 0) (size 1.2 1.2)
                    (layers "F.Cu") (net "/TARGET")))
                (footprint "test" (at 47 25)
                  (property "Reference" "J2")
                  (pad "1" smd circle (at 0 0) (size 1.2 1.2)
                    (layers "F.Cu") (net "/TARGET")))
                (zone (layers "F.Cu" "B.Cu") (name "UPPER")
                  (keepout (tracks not_allowed) (vias not_allowed))
                  (polygon (pts (xy 27.5 10.5) (xy 32.5 10.5)
                                (xy 32.5 29.7) (xy 27.5 29.7))))
                (zone (layers "F.Cu" "B.Cu") (name "LOWER")
                  (keepout (tracks not_allowed) (vias not_allowed))
                  (polygon (pts (xy 27.5 31.5) (xy 32.5 31.5)
                                (xy 32.5 39.5) (xy 27.5 39.5)))))"#,
        )
        .unwrap();
        let candidate = route_materialized_connection(
            &board,
            "TARGET",
            &KiCadGridRouteConfig {
                resolution_mm: 0.25,
                trace_width_mm: 0.8,
                clearance_mm: 0.4,
                edge_clearance_mm: 0.4,
                reachability_preflight: true,
                terminal_contact_policy: KiCadTerminalContactPolicy::FirstPadContact,
                ..KiCadGridRouteConfig::default()
            },
        )
        .unwrap();
        assert_eq!(candidate.branches.len(), 1);
        assert!(
            candidate
                .branches
                .iter()
                .flat_map(|branch| &branch.path)
                .any(|point| point.at[1] > 30.5 && point.at[1] < 30.7)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn asymmetric_courtyard_keeps_its_local_center_offset() {
        let pcb = parse(
            r#"(kicad_pcb
                (footprint "test" (at 10 20 90)
                  (property "Reference" "R1")
                  (fp_rect (start -1 -2) (end 3 4)
                    (stroke (width 0.05) (type default))
                    (fill none) (layer "F.CrtYd"))))"#,
        )
        .unwrap();
        let infos = footprint_infos(&pcb).unwrap();
        let info = &infos["R1"];
        assert_eq!(info.body_center_local, [1.0, 1.0]);
        assert_eq!(info.body_size, [4.0, 6.0]);
        let center = footprint_body_center(info);
        assert!((center[0] - 11.0).abs() < 1.0e-12);
        assert!((center[1] - 19.0).abs() < 1.0e-12);
    }

    #[test]
    fn placement_clearance_prunes_unreachable_footprints() {
        let footprint = |reference: &str, x| KiCadFootprintInfo {
            reference: reference.into(),
            at: [x, 5.0, 0.0],
            body_center_local: [0.0, 0.0],
            body_size: [2.0, 2.0],
            router_movable: true,
        };
        let motion = KiCadMovableFootprintConfig {
            maximum_translation_mm: 2.0,
            maximum_rotation_degrees: 0.0,
            ..KiCadMovableFootprintConfig::default()
        };
        let movable = footprint("R1", 5.0);
        assert!(footprint_motion_envelopes_can_meet(
            &movable,
            &motion,
            &footprint("R2", 8.0),
            None,
        ));
        assert!(!footprint_motion_envelopes_can_meet(
            &movable,
            &motion,
            &footprint("R3", 20.0),
            None,
        ));
    }

    #[test]
    fn footprint_placement_application_checks_the_source_pose() {
        let mut pcb = parse(
            r#"(kicad_pcb
                (footprint "test"
                    (layer "F.Cu")
                    (at 10 20 30)
                    (property "Reference" "R1" (at 0 0 30))))"#,
        )
        .unwrap();
        let placement = KiCadFootprintPlacement {
            reference: "R1".into(),
            source_at: [10.0, 20.0, 30.0],
            at: [11.0, 19.0, 35.0],
        };

        apply_footprint_placements(&mut pcb, std::slice::from_ref(&placement)).unwrap();
        let footprint = pcb
            .children()
            .iter()
            .find(|item| item.head() == Some("footprint"))
            .unwrap();
        assert_eq!(form_at(footprint).unwrap(), placement.at);

        let error = apply_footprint_placements(&mut pcb, &[placement]).unwrap_err();
        assert!(error.contains("expects source pose"));
    }

    #[test]
    fn reference_field_placement_is_uuid_and_source_pose_checked() {
        let mut pcb = parse(
            r#"(kicad_pcb
                (footprint "test"
                    (layer "F.Cu")
                    (at 10 20 0)
                    (property "Reference" "R2"
                        (at 0 -2.2 0)
                        (layer "F.SilkS")
                        (uuid "reference-r2"))))"#,
        )
        .unwrap();
        let placement = KiCadReferencePlacement {
            reference: "R2".into(),
            uuid: "reference-r2".into(),
            source_at: [0.0, -2.2, 0.0],
            at: [2.2, 0.0, 0.0],
        };

        apply_reference_placements(&mut pcb, std::slice::from_ref(&placement)).unwrap();
        let property = pcb
            .children()
            .iter()
            .find(|item| item.head() == Some("footprint"))
            .unwrap()
            .children()
            .iter()
            .find(|item| item.head() == Some("property"))
            .unwrap();
        assert_eq!(form_at(property).unwrap(), placement.at);

        let error = apply_reference_placements(&mut pcb, &[placement]).unwrap_err();
        assert!(error.contains("expects source pose"));
    }

    #[test]
    fn reference_alternates_match_the_imported_predecessor_policy() {
        let source = [1.0, -2.0, 30.0];
        assert_eq!(alternate_reference_pose(source, 1), [-1.0, 2.0, 30.0]);
        assert_eq!(alternate_reference_pose(source, 2), [2.0, 1.0, 30.0]);
        assert_eq!(alternate_reference_pose(source, 3), [-2.0, -1.0, 30.0]);
        assert_eq!(alternate_reference_pose(source, 4), [1.0, 2.0, 30.0]);
        assert_eq!(alternate_reference_pose(source, 5), [-1.0, 2.0, 30.0]);
        assert_eq!(
            alternate_reference_pose([0.0, -2.2, 0.0], 1),
            alternate_reference_pose([0.0, -2.2, 0.0], 4)
        );
    }

    #[test]
    fn continuous_obstacle_check_rejects_crossing_shortcuts() {
        let obstacle = ObstacleGeometry::Circle {
            center: [5.0, 5.0],
            radius: 0.5,
        };
        assert!(obstacle.intersects_segment([1.0, 5.0], [9.0, 5.0], 0.2));
        assert!(!obstacle.intersects_segment([1.0, 7.0], [9.0, 7.0], 0.2));
    }

    #[test]
    fn custom_polygon_geometry_is_not_reduced_to_its_anchor() {
        let pad = parse(
            r#"(pad "2" smd custom
                (at 0 0)
                (size 0.3 0.3)
                (primitives
                    (gr_poly
                        (pts (xy 0.5 -0.75) (xy -0.65 -0.75) (xy -0.15 0)
                             (xy -0.65 0.75) (xy 0.5 0.75))
                        (width 0)
                        (fill yes))))"#,
        )
        .unwrap();
        let geometry = custom_pad_geometry(
            &pad,
            [10.0, 20.0],
            0.0,
            ObstacleGeometry::Rectangle {
                center: [10.0, 20.0],
                half_size: [0.15, 0.15],
                angle_degrees: 0.0,
            },
        )
        .unwrap();
        assert!(geometry.contains([10.45, 20.5], 0.0));
        assert!(!geometry.contains([9.0, 20.7], 0.0));
        assert_eq!(geometry.aabb().minimum, [9.35, 19.25]);
        assert_eq!(geometry.aabb().maximum, [10.5, 20.75]);
    }

    #[test]
    fn first_pad_contact_removes_only_the_inside_tail() {
        let pad = ObstacleGeometry::Rectangle {
            center: [0.0, 0.0],
            half_size: [1.0, 0.5],
            angle_degrees: 0.0,
        };
        let mut points = vec![
            ([0.0, 0.0], 1),
            ([0.5, 0.0], 1),
            ([1.5, 0.0], 1),
            ([3.0, 0.0], 1),
        ];
        trim_path_start_to_first_pad_contact(&mut points, &pad);
        assert_eq!(points.len(), 3);
        assert!((points[0].0[0] - 1.0).abs() < 1.0e-12);
        assert_eq!(points[0].0[1], 0.0);
        assert_eq!(points[0].1, 1);
        assert_eq!(points[1], ([1.5, 0.0], 1));
    }

    #[test]
    fn first_pad_contact_preserves_in_pad_via_at_either_path_end() {
        let pad = ObstacleGeometry::Rectangle {
            center: [0.0, 0.0],
            half_size: [1.0, 0.5],
            angle_degrees: 0.0,
        };
        let config = KiCadGridRouteConfig {
            resolution_mm: 0.5,
            terminal_contact_policy: KiCadTerminalContactPolicy::FirstPadContact,
            ..KiCadGridRouteConfig::default()
        };
        let mut grid_path = vec![
            GridPosition {
                x: 0,
                y: 0,
                layer: 0,
            },
            GridPosition {
                x: 1,
                y: 0,
                layer: 0,
            },
            GridPosition {
                x: 1,
                y: 0,
                layer: 1,
            },
            GridPosition {
                x: 3,
                y: 0,
                layer: 1,
            },
            GridPosition {
                x: 6,
                y: 0,
                layer: 1,
            },
        ];
        for reversed in [false, true] {
            let (terminals, pads) = if reversed {
                grid_path.reverse();
                ([[3.0, 0.0], [0.0, 0.0]], [None, Some(&pad)])
            } else {
                ([[0.0, 0.0], [3.0, 0.0]], [Some(&pad), None])
            };
            let (path, segments, vias) = materialize_grid_path(
                "SMD",
                &grid_path,
                terminals,
                pads,
                [0.0, 0.0],
                &config,
            );
            assert_eq!(vias.len(), 1);
            assert_eq!(vias[0].at, [0.5, 0.0]);
            let terminal = if reversed { path.last() } else { path.first() }.unwrap();
            assert_eq!(terminal.at, [0.0, 0.0]);
            assert_eq!(terminal.layer, "F.Cu");
            assert!(segments.iter().any(|segment| segment.layer == "F.Cu"));
            assert!(segments.iter().any(|segment| segment.layer == "B.Cu"));
        }
    }

    #[test]
    fn route_graph_normalization_splits_collinear_overlap_and_interior_contacts() {
        let point = |at| KiCadRoutePoint {
            at,
            layer: "F.Cu".into(),
        };
        let branch = |path: Vec<[f64; 2]>| KiCadRouteBranch {
            start_terminal: path[0],
            finish_terminal: *path.last().unwrap(),
            cost: 0,
            expansions: 0,
            path: path.into_iter().map(point).collect(),
        };
        let mut candidate = KiCadRouteCandidate {
            schema_version: 2,
            connection: "SHARED".into(),
            router: "test".into(),
            config: KiCadGridRouteConfig::default(),
            terminals: Vec::new(),
            grid_origin: [0.0, 0.0],
            grid_alignment_evidence: None,
            heuristic_expansions: None,
            grid_size: [1, 1],
            cost: 0,
            reachability_expansions: 0,
            expansions: 0,
            branches: vec![
                branch(vec![[0.0, 0.0], [4.0, 0.0], [5.0, 1.0]]),
                branch(vec![[0.0, 0.0], [2.0, 0.0], [5.0, 2.0]]),
                branch(vec![[-1.0, -1.0], [1.0, 0.0], [1.0, 1.0]]),
            ],
            tree_attachment_attempts: Vec::new(),
            supplemental_segments: Vec::new(),
            supplemental_vias: Vec::new(),
            footprint_placements: Vec::new(),
            reference_placements: Vec::new(),
        };

        let work = normalize_selected_route_graph(&mut candidate.branches, &[0, 1, 2]);

        assert_eq!(work.contact_points, 3);
        assert_eq!(work.inserted_points, 3);
        assert_eq!(
            candidate.branches[0]
                .path
                .iter()
                .map(|point| point.at)
                .collect::<Vec<_>>(),
            vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [4.0, 0.0], [5.0, 1.0]]
        );
        assert_eq!(
            candidate.branches[1]
                .path
                .iter()
                .map(|point| point.at)
                .collect::<Vec<_>>(),
            vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [5.0, 2.0]]
        );
    }

    #[test]
    fn nearest_shared_junction_selection_grows_one_local_bundle_per_hop() {
        let point = |at| KiCadRoutePoint {
            at,
            layer: "F.Cu".into(),
        };
        let branch = |path: Vec<[f64; 2]>| KiCadRouteBranch {
            start_terminal: path[0],
            finish_terminal: *path.last().unwrap(),
            cost: 0,
            expansions: 0,
            path: path.into_iter().map(point).collect(),
        };
        let candidate = KiCadRouteCandidate {
            schema_version: 2,
            connection: "TREE".into(),
            router: "fixture".into(),
            config: KiCadGridRouteConfig::default(),
            terminals: Vec::new(),
            grid_origin: [0.0, 0.0],
            grid_alignment_evidence: None,
            heuristic_expansions: None,
            grid_size: [1, 1],
            cost: 0,
            reachability_expansions: 0,
            expansions: 0,
            branches: vec![
                branch(vec![[0.0, 0.0], [3.0, 0.0], [5.0, 0.0], [7.0, 2.0]]),
                branch(vec![[0.0, 0.0], [3.0, 0.0], [5.0, 0.0], [7.0, -2.0]]),
                branch(vec![[0.0, 0.0], [3.0, 0.0], [3.0, 3.0]]),
                branch(vec![[0.0, 0.0], [-2.0, 2.0]]),
            ],
            tree_attachment_attempts: Vec::new(),
            supplemental_segments: Vec::new(),
            supplemental_vias: Vec::new(),
            footprint_placements: Vec::new(),
            reference_placements: Vec::new(),
        };
        let config = KiCadRouteRelaxationConfig {
            shared_copper: KiCadSharedCopperPolicy::SharedRouteGraph,
            branch_selection: KiCadBranchSelectionPolicy::NearestSharedJunction,
            branch_indices: vec![0],
            branch_neighborhood_hops: 1,
            maximum_selected_branches: 3,
            ..KiCadRouteRelaxationConfig::default()
        };

        let one_hop = select_relaxation_branches(&candidate, &config).unwrap();
        assert_eq!(one_hop.selected, vec![0, 1]);
        assert!(one_hop.contact_tests > 0);

        let two_hops = select_relaxation_branches(
            &candidate,
            &KiCadRouteRelaxationConfig {
                branch_neighborhood_hops: 2,
                ..config.clone()
            },
        )
        .unwrap();
        assert_eq!(two_hops.selected, vec![0, 1, 2]);

        let (detail, contact_tests) = select_relaxation_branches(
            &candidate,
            &KiCadRouteRelaxationConfig {
                maximum_selected_branches: 1,
                ..config
            },
        )
        .unwrap_err();
        assert!(detail.contains("exceeding maximum_selected_branches=1"));
        assert!(contact_tests > 0);
    }

    #[test]
    fn via_topology_edit_round_trips_and_removes_a_whole_redundant_excursion() {
        let point = |at, layer: &str| KiCadRoutePoint {
            at,
            layer: layer.into(),
        };
        let path = vec![
            point([0.0, 0.0], "F.Cu"),
            point([2.0, 0.0], "F.Cu"),
            point([2.0, 0.0], "B.Cu"),
            point([4.0, 1.0], "B.Cu"),
            point([6.0, 0.0], "B.Cu"),
            point([6.0, 0.0], "F.Cu"),
            point([8.0, 0.0], "F.Cu"),
        ];
        assert_eq!(topology_path(&branch_topology(&path).unwrap()), path);
        let mut branch = KiCadRouteBranch {
            start_terminal: [0.0, 0.0],
            finish_terminal: [8.0, 0.0],
            cost: 0,
            expansions: 0,
            path,
        };

        assert!(
            remove_physical_via_from_branch(
                &mut branch,
                [2.000000000000001, 0.0],
                &["F.Cu".into(), "B.Cu".into()],
                "F.Cu",
            )
            .unwrap()
        );
        assert_eq!(
            branch.path,
            vec![
                point([0.0, 0.0], "F.Cu"),
                point([4.0, 1.0], "F.Cu"),
                point([8.0, 0.0], "F.Cu"),
            ]
        );
    }

    #[test]
    fn remove_then_local_reroute_repairs_the_new_chord_and_reduces_vias() {
        let point = |at, layer: &str| KiCadRoutePoint {
            at,
            layer: layer.into(),
        };
        let mut candidate = KiCadRouteCandidate {
            schema_version: 2,
            connection: "SIGNAL".into(),
            router: "fixture".into(),
            config: KiCadGridRouteConfig {
                resolution_mm: 0.5,
                ..KiCadGridRouteConfig::default()
            },
            terminals: vec![[1.0, 1.0], [9.0, 5.0]],
            grid_origin: [0.0, 0.0],
            grid_alignment_evidence: None,
            heuristic_expansions: None,
            grid_size: [1, 1],
            cost: 0,
            reachability_expansions: 0,
            expansions: 0,
            branches: vec![KiCadRouteBranch {
                start_terminal: [1.0, 1.0],
                finish_terminal: [9.0, 5.0],
                cost: 0,
                expansions: 0,
                path: vec![
                    point([1.0, 1.0], "F.Cu"),
                    point([3.0, 1.0], "F.Cu"),
                    point([3.0, 1.0], "B.Cu"),
                    point([3.0, 5.0], "B.Cu"),
                    point([7.0, 5.0], "B.Cu"),
                    point([7.0, 5.0], "F.Cu"),
                    point([9.0, 5.0], "F.Cu"),
                ],
            }],
            tree_attachment_attempts: Vec::new(),
            supplemental_segments: Vec::new(),
            supplemental_vias: Vec::new(),
            footprint_placements: Vec::new(),
            reference_placements: Vec::new(),
        };
        let (segments, vias) = materialize_candidate_branches(
            &candidate.connection,
            &candidate.branches,
            &candidate.config,
        )
        .unwrap();
        candidate.supplemental_segments = segments;
        candidate.supplemental_vias = vias;
        assert_eq!(candidate.supplemental_vias.len(), 2);
        let model = KiCadRoutingModel {
            bounds: [0.0, 0.0, 10.0, 7.0],
            outline: BoardOutline::rectangle([0.0, 0.0, 10.0, 7.0]).unwrap(),
            terminals: candidate.terminals.clone(),
            terminal_pads: Vec::new(),
            obstacles: vec![CopperObstacle {
                source_uuid: None,
                blocks_tracks: true,
                blocks_vias: true,
                local_clearance_mm: 0.0,
                layers: [true, false],
                geometry: ObstacleGeometry::Circle {
                    center: [2.0, 3.0],
                    radius: 0.5,
                },
                kind: KiCadViaLocalRerouteBlockerKind::ForeignNetCopper,
                object: "BLOCK".into(),
                net: Some("BLOCK".into()),
                footprint: None,
                component_movable: false,
            }],
            target_holes: Vec::new(),
        };
        let config = KiCadViaActionSearchConfig {
            target_at: [3.0, 1.0],
            target_layers: ["F.Cu".into(), "B.Cu".into()],
            removal_layers: vec!["F.Cu".into()],
            relocation_radius_mm: 0.0,
            relocation_step_mm: 0.0,
            maximum_relocation_candidates: 0,
            feasible_frontier: None,
            local_reroute: Some(KiCadViaLocalRerouteSearchConfig {
                merged_layers: vec!["F.Cu".into()],
                resolutions_mm: vec![0.5],
                window_margin_mm: 1.0,
                maximum_grid_states: 10_000,
                maximum_expansions: 100_000,
                maximum_exact_edge_retries: 64,
                blocker_cut: Some(KiCadViaBlockerCutSearchConfig {
                    maximum_ranked_blockers: 4,
                    maximum_cut_size: 2,
                    maximum_trials: 8,
                }),
            }),
            maximum_actions: 2,
            maximum_affected_branches: 1,
            via_penalty_mm: 3.0,
            minimum_score_improvement_mm: 1.0e-6,
        };
        validate_via_action_config(&config).unwrap();
        let (direct, affected) = apply_via_topology_action(
            &candidate,
            &config,
            &KiCadViaTopologyAction::Remove {
                merged_layer: "F.Cu".into(),
            },
        )
        .unwrap();
        assert!(!selected_geometry_blockers(&direct, &affected, &model).is_empty());

        let action = KiCadViaTopologyAction::RemoveAndReroute {
            merged_layer: "F.Cu".into(),
            resolution_mm: 0.5,
            window_margin_mm: 1.0,
        };
        let (repaired, affected, evidence) =
            apply_via_search_action(&candidate, &model, &config, &action).unwrap();
        assert_eq!(affected, vec![0]);
        assert!(selected_geometry_blockers(&repaired, &affected, &model).is_empty());
        assert!(candidate_via_blockers(&repaired, &model).is_empty());
        assert!(repaired.supplemental_vias.is_empty());
        let evidence = evidence.expect("remove-and-reroute retains work evidence");
        assert_eq!(evidence.branches[0].start, [1.0, 1.0]);
        assert_eq!(evidence.branches[0].finish, [9.0, 5.0]);
        assert!(evidence.total_expansions > 0);
        assert!(evidence.branches[0].path_points > 2);
        assert!(evidence.failed_branch.is_none());

        let blocked_model = KiCadRoutingModel {
            bounds: [0.0, 0.0, 10.0, 7.0],
            outline: BoardOutline::rectangle([0.0, 0.0, 10.0, 7.0]).unwrap(),
            terminals: candidate.terminals.clone(),
            terminal_pads: Vec::new(),
            obstacles: vec![CopperObstacle {
                source_uuid: None,
                blocks_tracks: true,
                blocks_vias: true,
                local_clearance_mm: 0.0,
                layers: [true, false],
                geometry: ObstacleGeometry::Rectangle {
                    center: [5.0, 3.5],
                    half_size: [0.5, 3.5],
                    angle_degrees: 0.0,
                },
                kind: KiCadViaLocalRerouteBlockerKind::FootprintPad,
                object: "WALL".into(),
                net: Some("BLOCK".into()),
                footprint: Some("WALL".into()),
                component_movable: true,
            }],
            target_holes: Vec::new(),
        };
        let error = apply_via_search_action(&candidate, &blocked_model, &config, &action)
            .expect_err("the full-height wall must retain a no-path diagnostic");
        assert_eq!(error.affected_branches, vec![0]);
        let failed = error
            .local_reroute
            .as_ref()
            .and_then(|evidence| evidence.failed_branch.as_ref())
            .expect("no-path action retains structured grid evidence");
        assert_eq!(failed.kind, KiCadViaLocalRerouteFailureKind::NoPath);
        assert!(failed.expansions > 0);
        assert!(failed.blocked_states > 0);
        assert!(!failed.blocked_runs.is_empty());
        assert!(failed.blocked_frontier_states > 0);
        assert!(!failed.blocked_frontier_runs.is_empty());
        assert_eq!(
            failed.attributed_frontier_states,
            failed.blocked_frontier_states
        );
        assert_eq!(failed.unattributed_frontier_states, 0);
        assert_eq!(failed.blockers.len(), 1);
        assert_eq!(
            failed.blockers[0].kind,
            KiCadViaLocalRerouteBlockerKind::FootprintPad
        );
        assert_eq!(failed.blockers[0].object, "WALL");
        assert_eq!(failed.blockers[0].nets, ["BLOCK"]);
        assert!(failed.blockers[0].component_movable);
        assert_eq!(
            failed.blockers[0].frontier_hits,
            failed.blocked_frontier_states
        );
        let blocker_cut = error
            .local_reroute
            .as_ref()
            .and_then(|evidence| evidence.blocker_cut.as_ref())
            .expect("configured failures retain causal blocker-cut evidence");
        assert!(blocker_cut.counterfactual_only);
        assert!(!blocker_cut.selectable);
        assert_eq!(blocker_cut.minimum_sufficient_cut_size, Some(1));
        assert_eq!(blocker_cut.trials.len(), 1);
        assert_eq!(
            blocker_cut.trials[0].status,
            KiCadViaBlockerCutTrialStatus::Found
        );
        assert_eq!(blocker_cut.trials[0].added_blocker.object, "WALL");
        assert!(!blocker_cut.trials[0].path.is_empty());
        let diagnostic_candidate = error
            .diagnostic_candidate
            .as_ref()
            .expect("no-path action retains its topology-only candidate");
        let front = render_via_local_reroute_diagnostic_svg(
            "reduced no-path",
            &blocked_model,
            diagnostic_candidate,
            failed,
            0,
            true,
            None,
            &[],
        );
        let back = render_via_local_reroute_diagnostic_svg(
            "reduced no-path",
            &blocked_model,
            diagnostic_candidate,
            failed,
            1,
            true,
            None,
            &[],
        );
        assert!(front.contains("blocked states"));
        assert!(front.contains("#67e8f9"));
        assert!(front.contains("data-semantic-blocker=\"WALL\""));
        assert!(front.contains("WALL ("));
        assert!(back.contains("search overlay is on F.Cu"));

        // WALL_B is hidden behind WALL_A at the original frontier. The
        // second failure must expose it before the sufficient pair exists.
        let sequential_model = KiCadRoutingModel {
            bounds: [0.0, 0.0, 10.0, 7.0],
            outline: BoardOutline::rectangle([0.0, 0.0, 10.0, 7.0]).unwrap(),
            terminals: candidate.terminals.clone(),
            terminal_pads: Vec::new(),
            obstacles: [("WALL_A", 4.0), ("WALL_B", 6.0)]
                .into_iter()
                .map(|(object, x)| CopperObstacle {
                    source_uuid: None,
                    blocks_tracks: true,
                    blocks_vias: true,
                    local_clearance_mm: 0.0,
                    layers: [true, false],
                    geometry: ObstacleGeometry::Rectangle {
                        center: [x, 3.5],
                        half_size: [0.35, 3.5],
                        angle_degrees: 0.0,
                    },
                    kind: KiCadViaLocalRerouteBlockerKind::FootprintPad,
                    object: object.into(),
                    net: Some("BLOCK".into()),
                    footprint: Some(object.into()),
                    component_movable: true,
                })
                .collect(),
            target_holes: Vec::new(),
        };
        let sequential_error =
            apply_via_search_action(&candidate, &sequential_model, &config, &action)
                .expect_err("both sequential walls keep the real candidate blocked");
        let sequential_cut = sequential_error
            .local_reroute
            .as_ref()
            .and_then(|evidence| evidence.blocker_cut.as_ref())
            .expect("sequential walls retain blocker-cut evidence");
        assert_eq!(sequential_cut.ranked_blockers.len(), 1);
        assert_eq!(sequential_cut.ranked_blockers[0].object, "WALL_A");
        assert_eq!(sequential_cut.minimum_sufficient_cut_size, Some(2));
        let sufficient = &sequential_cut.trials[sequential_cut.sufficient_trials[0]];
        assert_eq!(sufficient.parent_trial, Some(0));
        assert_eq!(sufficient.added_blocker.object, "WALL_B");
        assert_eq!(
            sufficient
                .suppressed
                .iter()
                .map(|blocker| blocker.object.as_str())
                .collect::<Vec<_>>(),
            ["WALL_A", "WALL_B"]
        );
    }

    #[test]
    fn via_relocation_updates_every_serialized_side_but_not_endpoints() {
        let point = |at, layer: &str| KiCadRoutePoint {
            at,
            layer: layer.into(),
        };
        let mut branch = KiCadRouteBranch {
            start_terminal: [0.0, 0.0],
            finish_terminal: [4.0, 0.0],
            cost: 0,
            expansions: 0,
            path: vec![
                point([0.0, 0.0], "F.Cu"),
                point([2.0, 0.0], "F.Cu"),
                point([2.0, 0.0], "B.Cu"),
                point([4.0, 0.0], "B.Cu"),
            ],
        };
        assert!(
            relocate_physical_via_in_branch(
                &mut branch,
                [2.0, 0.0],
                &["F.Cu".into(), "B.Cu".into()],
                [2.0, 1.0],
            )
            .unwrap()
        );
        assert_eq!(branch.path[0].at, branch.start_terminal);
        assert_eq!(branch.path[1].at, [2.0, 1.0]);
        assert_eq!(branch.path[2].at, [2.0, 1.0]);
        assert_eq!(branch.path[3].at, branch.finish_terminal);
    }

    #[test]
    fn feasible_via_frontier_spends_native_candidates_only_on_clear_improvements() {
        let point = |at, layer: &str| KiCadRoutePoint {
            at,
            layer: layer.into(),
        };
        let mut candidate = KiCadRouteCandidate {
            schema_version: 2,
            connection: "SIGNAL".into(),
            router: "fixture".into(),
            config: KiCadGridRouteConfig::default(),
            terminals: vec![[2.0, 2.0], [9.0, 2.0]],
            grid_origin: [0.0, 0.0],
            grid_alignment_evidence: None,
            heuristic_expansions: None,
            grid_size: [1, 1],
            cost: 0,
            reachability_expansions: 0,
            expansions: 0,
            branches: vec![KiCadRouteBranch {
                start_terminal: [2.0, 2.0],
                finish_terminal: [9.0, 2.0],
                cost: 0,
                expansions: 0,
                path: vec![
                    point([2.0, 2.0], "F.Cu"),
                    point([5.0, 6.0], "F.Cu"),
                    point([5.0, 6.0], "B.Cu"),
                    point([9.0, 2.0], "B.Cu"),
                ],
            }],
            tree_attachment_attempts: Vec::new(),
            supplemental_segments: Vec::new(),
            supplemental_vias: Vec::new(),
            footprint_placements: Vec::new(),
            reference_placements: Vec::new(),
        };
        let (segments, vias) = materialize_candidate_branches(
            &candidate.connection,
            &candidate.branches,
            &candidate.config,
        )
        .unwrap();
        candidate.supplemental_segments = segments;
        candidate.supplemental_vias = vias;
        let model = KiCadRoutingModel {
            bounds: [0.0, 0.0, 12.0, 10.0],
            outline: BoardOutline::rectangle([0.0, 0.0, 12.0, 10.0]).unwrap(),
            terminals: candidate.terminals.clone(),
            terminal_pads: Vec::new(),
            obstacles: vec![CopperObstacle {
                source_uuid: None,
                blocks_tracks: true,
                blocks_vias: true,
                local_clearance_mm: 0.0,
                layers: [false, true],
                geometry: ObstacleGeometry::Circle {
                    center: [5.5, 4.5],
                    radius: 0.2,
                },
                kind: KiCadViaLocalRerouteBlockerKind::ForeignNetCopper,
                object: "BLOCK".into(),
                net: Some("BLOCK".into()),
                footprint: None,
                component_movable: false,
            }],
            target_holes: Vec::new(),
        };
        let config = KiCadViaActionSearchConfig {
            target_at: [5.0, 6.0],
            target_layers: ["F.Cu".into(), "B.Cu".into()],
            removal_layers: Vec::new(),
            relocation_radius_mm: 2.0,
            relocation_step_mm: 0.0,
            maximum_relocation_candidates: 4,
            feasible_frontier: Some(KiCadViaFeasibleFrontierConfig {
                angular_samples: 16,
                radial_samples: 8,
                boundary_refinement_steps: 8,
                boundary_inset_mm: 0.001,
                minimum_candidate_spacing_mm: 0.1,
            }),
            local_reroute: None,
            maximum_actions: 4,
            maximum_affected_branches: 1,
            via_penalty_mm: 3.0,
            minimum_score_improvement_mm: 1.0e-6,
        };
        validate_via_action_config(&config).unwrap();
        let source_quality = route_candidate_quality(&candidate).unwrap();
        let source_score =
            source_quality.length_mm + config.via_penalty_mm * source_quality.vias as f64;
        let (actions, evidence) =
            via_action_candidates(&candidate, &model, &config, source_score).unwrap();
        let (repeated, _) =
            via_action_candidates(&candidate, &model, &config, source_score).unwrap();

        assert_eq!(actions, repeated);
        assert!(!actions.is_empty());
        assert!(actions.len() <= config.maximum_relocation_candidates);
        assert_eq!(evidence.method, "analytic-feasible-radial-frontier-v1");
        assert!(evidence.analytic_probes > actions.len());
        assert!(evidence.feasibility_transitions > 0);
        for (_, action) in actions {
            let KiCadViaTopologyAction::Relocate { to } = action else {
                panic!("frontier emitted a non-relocation action");
            };
            let (clear, score) = assess_via_relocation(&candidate, &model, &config, to).unwrap();
            assert!(clear);
            assert!(score + config.minimum_score_improvement_mm <= source_score);
        }
    }

    #[test]
    fn route_quality_separates_stored_track_sum_from_physical_copper_union() {
        let branch = |end| KiCadRouteBranch {
            start_terminal: [0.0, 0.0],
            finish_terminal: end,
            cost: 0,
            expansions: 0,
            path: vec![
                KiCadRoutePoint {
                    at: [0.0, 0.0],
                    layer: "F.Cu".into(),
                },
                KiCadRoutePoint {
                    at: end,
                    layer: "F.Cu".into(),
                },
            ],
        };
        let segment = |end| SupplementalSegment {
            connection: "SHARED".into(),
            start: [0.0, 0.0],
            end,
            width: 0.25,
            layer: "F.Cu".into(),
        };
        let candidate = KiCadRouteCandidate {
            schema_version: 2,
            connection: "SHARED".into(),
            router: "test".into(),
            config: KiCadGridRouteConfig::default(),
            terminals: vec![[0.0, 0.0], [4.0, 0.0], [2.0, 0.0]],
            grid_origin: [0.0, 0.0],
            grid_alignment_evidence: None,
            heuristic_expansions: None,
            grid_size: [1, 1],
            cost: 0,
            reachability_expansions: 0,
            expansions: 0,
            branches: vec![branch([4.0, 0.0]), branch([2.0, 0.0])],
            tree_attachment_attempts: Vec::new(),
            supplemental_segments: vec![segment([4.0, 0.0]), segment([2.0, 0.0])],
            supplemental_vias: Vec::new(),
            footprint_placements: Vec::new(),
            reference_placements: Vec::new(),
        };

        let quality = route_candidate_quality(&candidate).unwrap();

        assert_eq!(quality.length_mm, 4.0);
        assert_eq!(quality.segments, 2);
        assert_eq!(quality.stored_track_length_mm, 6.0);
        assert_eq!(quality.stored_segments, 2);
        assert_eq!(quality.overlapping_track_length_mm, 2.0);
    }

    #[test]
    fn rectangular_clearance_uses_euclidean_rounded_corners() {
        let half_size = [0.5, 0.5];
        assert!(point_intersects_rounded_rectangle(
            [0.0, 0.0],
            half_size,
            0.0
        ));
        assert!(point_intersects_rounded_rectangle(
            [0.7, 0.7],
            half_size,
            0.3
        ));
        assert!(!point_intersects_rounded_rectangle(
            [0.72, 0.72],
            half_size,
            0.3
        ));
        assert!(!segment_intersects_rounded_rectangle(
            [0.72, 0.72],
            [1.0, 1.0],
            half_size,
            0.3
        ));
    }

    #[test]
    fn shared_tree_attachment_ranking_accounts_for_layer_transition_cost() {
        let config = KiCadGridRouteConfig {
            multi_terminal_routing: KiCadMultiTerminalRoutingPolicy::SharedCopperTree,
            maximum_tree_attachment_searches: 3,
            ..KiCadGridRouteConfig::default()
        };
        let positions = [
            GridPosition {
                x: 0,
                y: 0,
                layer: 0,
            },
            GridPosition {
                x: 5,
                y: 5,
                layer: 0,
            },
            GridPosition {
                x: 5,
                y: 5,
                layer: 1,
            },
        ];
        let tree: GridTree = positions
            .into_iter()
            .map(|position| (grid_position_key(position), position))
            .collect();
        let target = GridPosition {
            x: 6,
            y: 5,
            layer: 1,
        };

        let ranked = ranked_tree_attachment_sources(&tree, target, &config);

        assert_eq!(ranked[0], positions[2]);
        assert_eq!(ranked[1], positions[1]);
        assert_eq!(ranked[2], positions[0]);
    }

    #[test]
    fn terminal_layer_prefers_front_for_through_holes_and_honors_bottom_smd() {
        assert_eq!(preferred_terminal_layer([true, true]), Some(0));
        assert_eq!(preferred_terminal_layer([true, false]), Some(0));
        assert_eq!(preferred_terminal_layer([false, true]), Some(1));
        assert_eq!(preferred_terminal_layer([false, false]), None);
    }

    #[test]
    fn shared_tree_attachment_ranking_can_sample_spatially_distinct_sources() {
        let config = KiCadGridRouteConfig {
            resolution_mm: 0.25,
            multi_terminal_routing: KiCadMultiTerminalRoutingPolicy::SharedCopperTree,
            maximum_tree_attachment_searches: 3,
            tree_attachment_minimum_spacing_mm: 1.0,
            ..KiCadGridRouteConfig::default()
        };
        let positions = [
            GridPosition {
                x: 10,
                y: 10,
                layer: 0,
            },
            GridPosition {
                x: 11,
                y: 10,
                layer: 0,
            },
            GridPosition {
                x: 14,
                y: 10,
                layer: 0,
            },
            GridPosition {
                x: 10,
                y: 10,
                layer: 1,
            },
        ];
        let tree: GridTree = positions
            .into_iter()
            .map(|position| (grid_position_key(position), position))
            .collect();
        let target = GridPosition {
            x: 10,
            y: 9,
            layer: 0,
        };

        let ranked = ranked_tree_attachment_sources(&tree, target, &config);

        assert_eq!(ranked, vec![positions[0], positions[2], positions[3]]);
    }

    #[test]
    fn shared_tree_path_trims_every_redundant_prefix_contact() {
        let position = |x| GridPosition { x, y: 0, layer: 0 };
        let tree: GridTree = [position(0), position(2)]
            .into_iter()
            .map(|point| (grid_position_key(point), point))
            .collect();
        let path = [position(0), position(1), position(2), position(3)];

        assert_eq!(
            trim_grid_path_to_last_tree_contact(&path, &tree),
            vec![position(2), position(3)]
        );
    }

    #[test]
    fn route_normalization_preserves_an_already_connected_terminal() {
        let mut branches = vec![KiCadRouteBranch {
            start_terminal: [2.0, 3.0],
            finish_terminal: [2.0, 3.0],
            cost: 0,
            expansions: 0,
            path: vec![KiCadRoutePoint {
                at: [2.0, 3.0],
                layer: "F.Cu".into(),
            }],
        }];
        normalize_selected_route_graph(&mut branches, &[0]);
        assert_eq!(branches[0].path.len(), 1);
        assert_eq!(branches[0].path[0].at, [2.0, 3.0]);
        assert_eq!(branches[0].path[0].layer, "F.Cu");
    }

    #[test]
    fn generated_shared_tree_serializes_interior_contacts_as_vertices() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-tree-junction-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        let mut source =
            String::from(r#"(kicad_pcb (gr_rect (start 0 0) (end 24 24) (layer "Edge.Cuts"))"#);
        for (index, (x, y)) in [(2, 2), (18, 18), (2, 18), (18, 2), (10, 7), (7, 10)]
            .into_iter()
            .enumerate()
        {
            source.push_str(&format!(
                r#"(footprint "test" (at {x} {y})
              (property "Reference" "J{index}")
              (pad "1" smd circle (at 0 0) (size 1 1)
                (layers "F.Cu") (net "/TARGET")))"#
            ));
        }
        source.push(')');
        fs::write(&board, source).unwrap();
        for search in [
            KiCadTreeAttachmentSearch::RankedSamples,
            KiCadTreeAttachmentSearch::MultiSource,
        ] {
            let candidate = route_materialized_connection(
                &board,
                "TARGET",
                &KiCadGridRouteConfig {
                    resolution_mm: 0.5,
                    multi_terminal_routing: KiCadMultiTerminalRoutingPolicy::SharedCopperTree,
                    tree_attachment_search: search,
                    tree_attachment_objective: KiCadTreeAttachmentObjective::RouterCost,
                    terminal_contact_policy: KiCadTerminalContactPolicy::FirstPadContact,
                    ..KiCadGridRouteConfig::default()
                },
            )
            .unwrap();
            assert_eq!(candidate.branches.len(), 5);
            let mut contacts = 0;
            for (i, first) in candidate.supplemental_segments.iter().enumerate() {
                for second in &candidate.supplemental_segments[i + 1..] {
                    if first.layer != second.layer {
                        continue;
                    }
                    for contact in
                        route_segment_contacts(first.start, first.end, second.start, second.end)
                    {
                        contacts += 1;
                        let endpoint = |segment: &SupplementalSegment| {
                            distance_squared(contact, segment.start)
                                .min(distance_squared(contact, segment.end))
                                < 1.0e-12
                        };
                        assert!(
                            endpoint(first) && endpoint(second),
                            "unsplit contact at {contact:?}"
                        );
                    }
                }
            }
            assert!(contacts > 0);
            assert_eq!(
                candidate
                    .tree_attachment_attempts
                    .iter()
                    .filter(|a| a.selected)
                    .count(),
                4
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn shared_tree_excludes_a_snapped_endpoint_that_copper_does_not_reach() {
        let positions = [0, 1, 2].map(|x| GridPosition { x, y: 0, layer: 0 });
        let materialized = [
            KiCadRoutePoint {
                at: [0.1, 0.0],
                layer: "F.Cu".into(),
            },
            KiCadRoutePoint {
                at: [2.0, 0.0],
                layer: "F.Cu".into(),
            },
        ];
        let mut tree = GridTree::new();

        insert_materialized_grid_path_contacts(
            &mut tree,
            &positions,
            &materialized,
            [0.0, 0.0],
            1.0,
        );

        assert!(!tree.contains_key(&grid_position_key(positions[0])));
        assert!(tree.contains_key(&grid_position_key(positions[1])));
        assert!(tree.contains_key(&grid_position_key(positions[2])));
    }

    #[test]
    fn shared_tree_attachment_objective_is_explicit() {
        let short_two_vias = (100, 5.0, 2, 0);
        let long_no_vias = (200, 6.0, 0, 1);
        assert!(tree_attachment_is_preferred(
            short_two_vias,
            long_no_vias,
            KiCadTreeAttachmentObjective::LengthThenVias
        ));
        assert!(tree_attachment_is_preferred(
            long_no_vias,
            short_two_vias,
            KiCadTreeAttachmentObjective::ViasThenLength
        ));
        assert!(tree_attachment_is_preferred(
            short_two_vias,
            long_no_vias,
            KiCadTreeAttachmentObjective::RouterCost
        ));
    }

    #[test]
    fn multi_source_attachment_requires_an_explicit_cost_search_contract() {
        let mut config = KiCadGridRouteConfig::default();
        assert!(
            serde_json::to_value(&config)
                .unwrap()
                .get("tree_attachment_search")
                .is_none()
        );
        config.tree_attachment_search = KiCadTreeAttachmentSearch::MultiSource;
        assert!(validate_grid_route_config(&config).is_err());
        config.multi_terminal_routing = KiCadMultiTerminalRoutingPolicy::SharedCopperTree;
        assert!(validate_grid_route_config(&config).is_err());
        config.tree_attachment_objective = KiCadTreeAttachmentObjective::RouterCost;
        assert!(validate_grid_route_config(&config).is_ok());
        config.maximum_tree_attachment_searches = 8;
        assert!(validate_grid_route_config(&config).is_err());
        config.maximum_tree_attachment_searches = 1;
        config.tree_attachment_minimum_spacing_mm = 1.0;
        assert!(validate_grid_route_config(&config).is_err());
    }

    #[test]
    fn uniform_grid_broad_phase_is_layer_aware_and_culls_remote_obstacles() {
        let path = vec![
            KiCadRoutePoint {
                at: [1.0, 1.0],
                layer: "F.Cu".into(),
            },
            KiCadRoutePoint {
                at: [2.0, 1.5],
                layer: "F.Cu".into(),
            },
            KiCadRoutePoint {
                at: [3.0, 1.0],
                layer: "F.Cu".into(),
            },
        ];
        let candidate = KiCadRouteCandidate {
            schema_version: 1,
            connection: "TARGET".into(),
            router: "fixture".into(),
            config: KiCadGridRouteConfig::default(),
            terminals: vec![[1.0, 1.0], [3.0, 1.0]],
            grid_origin: [0.0, 0.0],
            grid_alignment_evidence: None,
            heuristic_expansions: None,
            grid_size: [40, 40],
            cost: 0,
            reachability_expansions: 0,
            expansions: 0,
            branches: vec![KiCadRouteBranch {
                start_terminal: [1.0, 1.0],
                finish_terminal: [3.0, 1.0],
                cost: 0,
                expansions: 0,
                path,
            }],
            tree_attachment_attempts: Vec::new(),
            supplemental_segments: Vec::new(),
            supplemental_vias: Vec::new(),
            footprint_placements: Vec::new(),
            reference_placements: Vec::new(),
        };
        let model = KiCadRoutingModel {
            bounds: [0.0, 0.0, 10.0, 10.0],
            outline: BoardOutline::rectangle([0.0, 0.0, 10.0, 10.0]).unwrap(),
            terminals: candidate.terminals.clone(),
            terminal_pads: Vec::new(),
            obstacles: vec![
                CopperObstacle {
                    source_uuid: None,
                    blocks_tracks: true,
                    blocks_vias: true,
                    local_clearance_mm: 0.0,
                    layers: [true, false],
                    geometry: ObstacleGeometry::Circle {
                        center: [3.8, 1.0],
                        radius: 0.2,
                    },
                    kind: KiCadViaLocalRerouteBlockerKind::ForeignNetCopper,
                    object: "F_NEAR".into(),
                    net: Some("F_NEAR".into()),
                    footprint: None,
                    component_movable: false,
                },
                CopperObstacle {
                    source_uuid: None,
                    blocks_tracks: true,
                    blocks_vias: true,
                    local_clearance_mm: 0.0,
                    layers: [false, true],
                    geometry: ObstacleGeometry::Circle {
                        center: [2.0, 1.0],
                        radius: 0.2,
                    },
                    kind: KiCadViaLocalRerouteBlockerKind::ForeignNetCopper,
                    object: "B_NEAR".into(),
                    net: Some("B_NEAR".into()),
                    footprint: None,
                    component_movable: false,
                },
                CopperObstacle {
                    source_uuid: None,
                    blocks_tracks: true,
                    blocks_vias: true,
                    local_clearance_mm: 0.0,
                    layers: [true, false],
                    geometry: ObstacleGeometry::Rectangle {
                        center: [8.0, 8.0],
                        half_size: [0.5, 0.5],
                        angle_degrees: 0.0,
                    },
                    kind: KiCadViaLocalRerouteBlockerKind::FootprintPad,
                    object: "FAR".into(),
                    net: None,
                    footprint: None,
                    component_movable: false,
                },
            ],
            target_holes: Vec::new(),
        };
        let obstacle_grid = KiCadObstacleGrid::new(&model, 1.0, 0.0);
        for layer in 0..2 {
            for point in [[2.0, 1.0], [3.8, 1.0], [4.5, 1.0], [8.0, 8.0]] {
                for inflate in [0.0, 0.3, 0.8] {
                    let exhaustive = model.obstacles.iter().any(|obstacle| {
                        obstacle.layers[layer] && obstacle.geometry.contains(point, inflate)
                    });
                    assert_eq!(
                        obstacle_grid.any_contains(
                            &model,
                            layer,
                            point,
                            inflate,
                            CopperQueryKind::Trace
                        ),
                        exhaustive,
                        "broad phase differed at layer {layer}, point {point:?}, inflation {inflate}"
                    );
                }
            }
        }
        let config = KiCadRouteRelaxationConfig {
            broad_phase: KiCadRouteBroadPhase::UniformGrid,
            branch_indices: vec![0],
            broad_phase_cell_size_mm: 1.0,
            broad_phase_motion_margin_mm: 1.0,
            ..KiCadRouteRelaxationConfig::default()
        };
        let runs = selected_layer_runs(&candidate, &[0]).unwrap();
        let selection =
            select_relaxation_obstacles(&candidate, &model, &runs, &config, &BTreeSet::new());
        assert_eq!(selection.layer_relevant_pairs, 2);
        assert_eq!(selection.retained_pairs, 1);
        assert_eq!(selection.selected_runs_by_obstacle.len(), 1);
        assert_eq!(selection.selected_runs_by_obstacle[&0], vec![0]);
        assert!(selection.candidate_pairs < selection.layer_relevant_pairs);
        assert!(selection.broad_phase_cell_queries > 0);
    }

    #[test]
    fn native_layer_render_config_is_fail_closed() {
        assert!(validate_layer_render_config(&KiCadLayerRenderConfig::default()).is_ok());
        let legacy: KiCadLayerRenderConfig = serde_json::from_str(
            r#"{"width": 1000, "height": 750, "quality": "basic", "background": "opaque"}"#,
        )
        .unwrap();
        assert!(legacy.show_rule_areas);
        assert!(!legacy.show_copper_zones);
        let invalid = KiCadLayerRenderConfig {
            width: 0,
            quality: "cinematic".into(),
            ..KiCadLayerRenderConfig::default()
        };
        assert_eq!(
            validate_layer_render_config(&invalid).unwrap_err(),
            "KiCad render width and height must be within 1..=8192"
        );
        let invalid = KiCadLayerRenderConfig {
            quality: "cinematic".into(),
            ..KiCadLayerRenderConfig::default()
        };
        assert!(
            validate_layer_render_config(&invalid)
                .unwrap_err()
                .contains("unsupported KiCad render quality")
        );
    }

    #[test]
    fn render_overlay_exposes_rule_areas_without_mutating_source() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-kicad-rule-area-render-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let source_path = directory.join("source.kicad_pcb");
        let render_path = directory.join("render.kicad_pcb");
        let source = r#"(kicad_pcb
          (zone (layer "F.Cu")
            (keepout (tracks not_allowed) (vias not_allowed))
            (polygon (pts (xy 1 1) (xy 4 1) (xy 4 4) (xy 1 4))))
          (zone (layers "F&B.Cu")
            (keepout (tracks not_allowed) (vias not_allowed))
            (polygon (pts (xy 5 1) (xy 8 1) (xy 8 4) (xy 5 4))))
          (zone (layer "B.Cu")
            (keepout (tracks not_allowed) (vias not_allowed))
            (polygon (pts (xy 9 1) (xy 12 1) (xy 12 4) (xy 9 4))))
          (zone (net_name "/GND") (layer "F.Cu")
            (polygon (pts (xy 1 5) (xy 12 5) (xy 12 8) (xy 1 8)))))"#;
        fs::write(&source_path, source).unwrap();

        let config = KiCadLayerRenderConfig {
            show_copper_zones: true,
            ..KiCadLayerRenderConfig::default()
        };
        assert_eq!(
            prepare_layer_render_board(&source_path, &render_path, &config).unwrap(),
            KiCadLayerRenderOverlayCounts {
                rule_areas: 4,
                copper_zones: 1
            }
        );
        assert_eq!(fs::read_to_string(&source_path).unwrap(), source);
        let rendered = parse(&fs::read_to_string(&render_path).unwrap()).unwrap();
        let overlays = rendered
            .children()
            .iter()
            .filter(|item| item.head() == Some("gr_poly"))
            .collect::<Vec<_>>();
        assert_eq!(overlays.len(), 5);
        assert_eq!(
            overlays
                .iter()
                .filter(|item| form_atom(item, "layer", 1) == Some("F.SilkS"))
                .count(),
            3
        );
        assert_eq!(
            overlays
                .iter()
                .filter(|item| form_atom(item, "layer", 1) == Some("B.SilkS"))
                .count(),
            2
        );
        assert!(overlays.iter().all(|item| {
            item.child("fill").and_then(|fill| fill.children().get(1))
                == Some(&Expr::Atom("no".into()))
        }));
        let labels = rendered
            .children()
            .iter()
            .filter(|item| item.head() == Some("gr_text"))
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), 1);
        assert_eq!(
            labels[0].children().get(1).and_then(Expr::atom),
            Some("COPPER ZONE /GND")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ground_zone_replacement_is_bounded_and_thins_vias_by_board_cell() {
        let mut pcb = parse(
            r#"(kicad_pcb
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "A" (layer "F.Cu") (at 2 5)
                (property "Reference" "A" (at 0 0 0))
                (pad "1" smd circle (at 0 0) (size 1 1)
                  (layers "F.Cu") (net "/GND")))
              (footprint "B" (layer "F.Cu") (at 18 5)
                (property "Reference" "B" (at 0 0 0))
                (pad "1" smd circle (at 0 0) (size 1 1)
                  (layers "F.Cu") (net "/GND")))
              (segment (start 2 5) (end 18 5) (width 0.25)
                (layer "F.Cu") (net "/GND"))
              (via (at 4 4) (size 0.8) (drill 0.4)
                (layers "F.Cu" "B.Cu") (net "/GND"))
              (via (at 4.5 4.5) (size 0.8) (drill 0.4)
                (layers "F.Cu" "B.Cu") (net "/GND"))
              (via (at 10 5) (size 0.8) (drill 0.4)
                (layers "F.Cu" "B.Cu") (net "/GND")))"#,
        )
        .unwrap();
        let config = KiCadGroundZoneConfig {
            connection: "GND".into(),
            layers: vec!["F.Cu".into(), "B.Cu".into()],
            board_inset_mm: 0.5,
            clearance_mm: 0.3,
            minimum_thickness_mm: 0.25,
            thermal_gap_mm: 0.3,
            thermal_bridge_width_mm: 0.3,
            pad_connection: KiCadZonePadConnection::Solid,
            retain_existing_vias: true,
            via_thinning_cell_size_mm: Some(2.0),
        };
        let replacement = replace_routed_connection_with_zones(&mut pcb, &config).unwrap();
        assert_eq!(replacement.board_bounds, [0.0, 0.0, 20.0, 10.0]);
        assert_eq!(replacement.zone_bounds, [0.5, 0.5, 19.5, 9.5]);
        assert_eq!(replacement.removed_segments, 1);
        assert_eq!(replacement.removed_arcs, 0);
        assert_eq!(replacement.removed_vias, 1);
        assert_eq!(replacement.retained_vias, 2);
        assert_eq!(replacement.removed_zones, 0);
        assert_eq!(replacement.modified_pads, 2);
        assert_eq!(replacement.added_zones, 2);
        assert_eq!(count_head(&pcb, "segment"), 0);
        assert_eq!(count_head(&pcb, "via"), 2);
        assert_eq!(count_head(&pcb, "zone"), 2);
        let encoded = encode(&pcb);
        assert_eq!(encoded.matches("(zone_connect 2)").count(), 2);
        assert!(encoded.contains("(net_name \"/GND\")"));
    }

    #[test]
    fn line_of_sight_shortener_removes_points_but_preserves_vias() {
        let model = KiCadRoutingModel {
            bounds: [0.0, 0.0, 10.0, 10.0],
            outline: BoardOutline::rectangle([0.0, 0.0, 10.0, 10.0]).unwrap(),
            terminals: vec![[1.0, 2.0], [9.0, 8.0]],
            terminal_pads: Vec::new(),
            obstacles: Vec::new(),
            target_holes: Vec::new(),
        };
        let path = vec![
            KiCadRoutePoint {
                at: [1.0, 2.0],
                layer: "F.Cu".into(),
            },
            KiCadRoutePoint {
                at: [4.0, 3.0],
                layer: "F.Cu".into(),
            },
            KiCadRoutePoint {
                at: [5.0, 5.0],
                layer: "F.Cu".into(),
            },
            KiCadRoutePoint {
                at: [5.0, 5.0],
                layer: "B.Cu".into(),
            },
            KiCadRoutePoint {
                at: [7.0, 7.0],
                layer: "B.Cu".into(),
            },
            KiCadRoutePoint {
                at: [9.0, 8.0],
                layer: "B.Cu".into(),
            },
        ];
        let mut visibility_tests = 0;
        let (shortened, _, _, accepted) = shorten_branch_path_with_strategy(
            &path,
            &model,
            &KiCadGridRouteConfig::default(),
            &KiCadRouteShorteningConfig::default(),
            &mut visibility_tests,
        )
        .unwrap();
        assert_eq!(accepted, 2);
        assert_eq!(shortened.len(), 4);
        assert_eq!(shortened[1].at, shortened[2].at);
        assert_ne!(shortened[1].layer, shortened[2].layer);
    }

    #[test]
    fn visibility_shortest_path_beats_the_furthest_visible_greedy_choice() {
        let model = KiCadRoutingModel {
            bounds: [0.0, 0.0, 10.0, 10.0],
            outline: BoardOutline::rectangle([0.0, 0.0, 10.0, 10.0]).unwrap(),
            terminals: vec![[1.0, 1.0], [9.0, 1.0]],
            terminal_pads: Vec::new(),
            obstacles: vec![CopperObstacle {
                source_uuid: None,
                blocks_tracks: true,
                blocks_vias: true,
                local_clearance_mm: 0.0,
                layers: [true, false],
                geometry: ObstacleGeometry::Circle {
                    center: [5.0, 1.0],
                    radius: 0.5,
                },
                kind: KiCadViaLocalRerouteBlockerKind::ForeignNetCopper,
                object: "BLOCK".into(),
                net: Some("BLOCK".into()),
                footprint: None,
                component_movable: false,
            }],
            target_holes: Vec::new(),
        };
        let path: Vec<_> = [[1.0, 1.0], [5.0, 2.0], [5.0, 4.0], [9.0, 1.0]]
            .into_iter()
            .map(|at| KiCadRoutePoint {
                at,
                layer: "F.Cu".into(),
            })
            .collect();
        let grid_config = KiCadGridRouteConfig::default();
        let mut greedy_work = 0;
        let (greedy, _, _, _) = shorten_branch_path_with_strategy(
            &path,
            &model,
            &grid_config,
            &KiCadRouteShorteningConfig::default(),
            &mut greedy_work,
        )
        .unwrap();
        let mut shortest_work = 0;
        let (shortest, attempted, visible, accepted) = shorten_branch_path_with_strategy(
            &path,
            &model,
            &grid_config,
            &KiCadRouteShorteningConfig {
                strategy: KiCadRouteShorteningStrategy::VisibilityShortestPath,
                maximum_visibility_tests: 100,
            },
            &mut shortest_work,
        )
        .unwrap();
        let length = |candidate: &[KiCadRoutePoint]| {
            candidate
                .windows(2)
                .map(|pair| distance_squared(pair[0].at, pair[1].at).sqrt())
                .sum::<f64>()
        };
        assert_eq!(
            greedy.iter().map(|point| point.at).collect::<Vec<_>>(),
            vec![[1.0, 1.0], [5.0, 4.0], [9.0, 1.0]]
        );
        assert_eq!(
            shortest.iter().map(|point| point.at).collect::<Vec<_>>(),
            vec![[1.0, 1.0], [5.0, 2.0], [9.0, 1.0]]
        );
        assert!(length(&shortest) < length(&greedy));
        assert_eq!(attempted, 3);
        assert_eq!(visible, 2);
        assert_eq!(accepted, 1);
        assert!(shortest_work > greedy_work);
    }

    #[test]
    fn visibility_shortening_budget_is_fail_closed() {
        let model = KiCadRoutingModel {
            bounds: [0.0, 0.0, 10.0, 10.0],
            outline: BoardOutline::rectangle([0.0, 0.0, 10.0, 10.0]).unwrap(),
            terminals: vec![[1.0, 1.0], [9.0, 1.0]],
            terminal_pads: Vec::new(),
            obstacles: Vec::new(),
            target_holes: Vec::new(),
        };
        let path: Vec<_> = [[1.0, 1.0], [3.0, 2.0], [6.0, 2.0], [9.0, 1.0]]
            .into_iter()
            .map(|at| KiCadRoutePoint {
                at,
                layer: "F.Cu".into(),
            })
            .collect();
        let mut work = 0;
        let error = shorten_branch_path_with_strategy(
            &path,
            &model,
            &KiCadGridRouteConfig::default(),
            &KiCadRouteShorteningConfig {
                strategy: KiCadRouteShorteningStrategy::VisibilityShortestPath,
                maximum_visibility_tests: 2,
            },
            &mut work,
        )
        .unwrap_err();
        assert_eq!(work, 2);
        assert!(error.contains("exhausted its 2 visibility-test budget"));
    }

    #[test]
    fn yielding_counterfactual_removes_only_named_foreign_copper() {
        assert_eq!(
            route_failure_expansions(
                "router found no path for \"TARGET\" branch 0 after 77 expansions"
            ),
            Some(77)
        );
        assert_eq!(
            route_failure_expansions(
                "router found no path for \"TARGET\" branch 0 after 2000001 aggregate expansions"
            ),
            Some(2_000_001)
        );
        assert_eq!(route_failure_expansions("unrelated failure"), None);
        let obstacle = |kind, object: &str, net: Option<&str>| CopperObstacle {
            source_uuid: None,
            blocks_tracks: true,
            blocks_vias: true,
            local_clearance_mm: 0.0,
            layers: [true, false],
            geometry: ObstacleGeometry::Circle {
                center: [1.0, 1.0],
                radius: 0.2,
            },
            kind,
            object: object.into(),
            net: net.map(str::to_string),
            footprint: None,
            component_movable: false,
        };
        let yielding = BTreeSet::from(["BLOCKING"]);
        assert!(!retain_obstacle_when_connections_yield(
            &obstacle(
                KiCadViaLocalRerouteBlockerKind::ForeignNetCopper,
                "BLOCKING",
                Some("BLOCKING")
            ),
            &yielding
        ));
        assert!(retain_obstacle_when_connections_yield(
            &obstacle(
                KiCadViaLocalRerouteBlockerKind::ForeignNetCopper,
                "OTHER",
                Some("OTHER")
            ),
            &yielding
        ));
        assert!(retain_obstacle_when_connections_yield(
            &obstacle(
                KiCadViaLocalRerouteBlockerKind::FootprintPad,
                "R1",
                Some("BLOCKING")
            ),
            &yielding
        ));
    }
}

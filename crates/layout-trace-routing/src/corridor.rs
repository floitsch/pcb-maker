// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, VecDeque};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use serde::{Deserialize, Serialize};
use spade::handles::FixedVertexHandle;
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

use crate::geometry::{
    EPSILON, add, cross, dot, length, normalized_or, scale, segment_distance, sub,
};
use crate::model::{Rect, Vec2};
use crate::topology::{CrossingDirection, CutCrossing, GateDirection, GateKey, HomotopyWord};

// Segment/triangle intersections are parameterized to [0, 1], so this
// tolerance is independent of board scale. It rejects numerical point touches
// while retaining any terminal-cell interval with meaningful forward extent.
const TERMINAL_INTERVAL_PARAMETER_EPSILON: f64 = 1.0e-8;
// Projection evidence is diagnostic, not an alternate copy of the raw graph.
// Keep it useful on large boards without allowing one endpoint to serialize an
// unbounded list of triangles.
const MAX_TERMINAL_PROJECTION_CANDIDATES: usize = 16;
const MIN_BOUNDARY_EDGE_LENGTH: f64 = EPSILON * 32.0;
const BOUNDARY_INTERSECTION_PARAMETER_EPSILON: f64 = 1.0e-10;

#[derive(Clone, Debug, Serialize)]
pub struct CorridorObstacle {
    pub id: String,
    pub polygon: Vec<Vec2>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CorridorBuildError {
    InvalidBoardBounds,
    DuplicateObstacleId {
        obstacle: String,
    },
    PolygonHasTooFewVertices {
        obstacle: String,
        vertices: usize,
    },
    NonFiniteBoundaryVertex {
        owner: String,
        vertex: usize,
    },
    NonFiniteBoundaryEdge {
        owner: String,
        edge: usize,
    },
    NearDegenerateBoundaryEdge {
        owner: String,
        edge: usize,
        length_bits: u64,
    },
    NearDegenerateBoundaryIntersection {
        first_owner: String,
        first_edge: usize,
        second_owner: String,
        second_edge: usize,
    },
    TriangulationInsertion {
        feature: String,
        error: String,
    },
    ConstraintInsertionPanicked {
        owner: String,
        edge: usize,
    },
    ConstraintInsertionEmpty {
        owner: String,
        edge: usize,
    },
    ConstraintInsertionInvalid {
        owner: String,
        edge: usize,
    },
}

impl fmt::Display for CorridorBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "corridor graph build failed: {self:?}")
    }
}

impl std::error::Error for CorridorBuildError {}

#[derive(Clone, Debug, Serialize)]
pub struct CorridorGraph {
    pub revision: u64,
    pub placement_revision: u64,
    pub layer: String,
    pub board: Rect,
    pub obstacles: Vec<CorridorObstacle>,
    pub cut_basis: CutBasis,
    pub cells: Vec<CorridorCell>,
    pub gates: Vec<CorridorGate>,
    pub semantic: SemanticCorridorGraph,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CorridorCell {
    /// Disposable ID, valid only within `CorridorGraph::revision`.
    pub id: usize,
    pub triangle: [Vec2; 3],
    pub center: Vec2,
    /// Stable obstacle/board owners of the triangle vertices. Polygon corner
    /// indices are deliberately removed.
    pub boundary_features: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CorridorGate {
    /// Disposable ID, valid only within `CorridorGraph::revision`.
    pub id: usize,
    pub key: GateKey,
    pub cells: [usize; 2],
    pub segment: [Vec2; 2],
    /// Portal-length capacity used by the Liu et al. baseline.
    pub nominal_width: f64,
    pub clearance_model: GateClearanceModel,
}

/// Reduced topology/capacity graph derived from the raw triangle dual.
///
/// IDs are revision-local. `key` and `fingerprint` are stable while obstacle
/// ownership and connectivity remain unchanged.
#[derive(Clone, Debug, Serialize)]
pub struct SemanticCorridorGraph {
    pub revision: u64,
    pub placement_revision: u64,
    pub layer: String,
    pub fingerprint: String,
    pub regions: Vec<SemanticCorridorRegion>,
    pub passages: Vec<SemanticCorridorPassage>,
    pub raw_cell_to_region: Vec<usize>,
    pub raw_gate_to_passage: Vec<Option<usize>>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SemanticCorridorRegion {
    pub id: usize,
    pub key: String,
    pub boundary_features: Vec<String>,
    pub raw_cells: Vec<usize>,
    pub center: Vec2,
    pub area: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SemanticCorridorPassage {
    pub id: usize,
    pub key: String,
    pub regions: [usize; 2],
    pub limiting_features: [String; 2],
    pub topology_cuts: Vec<String>,
    pub raw_gates: Vec<usize>,
    pub representative_gate: usize,
    pub segment: [Vec2; 2],
    pub nominal_width: f64,
}

/// Deterministic work performed by the current whole-layer corridor rebuild.
///
/// This counts constructed records, not allocator activity or elapsed time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CorridorRebuildWork {
    pub raw_cells_constructed: usize,
    pub raw_gates_constructed: usize,
    pub semantic_regions_reduced: usize,
    pub semantic_passages_reduced: usize,
}

/// Stable semantic identity retained across a corridor rebuild.
///
/// `unaffected` means that none of the record's blocker owners belongs to the
/// caller-supplied changed-obstacle set. Geometry stability is checked
/// separately: a surviving key alone is not evidence that its center, width,
/// or representative portal can safely be reused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CorridorIdentitySurvival {
    pub before: usize,
    pub after: usize,
    pub affected_before: usize,
    pub affected_after: usize,
    pub unaffected_before: usize,
    pub unaffected_keys_survived: usize,
    pub unaffected_geometry_stable: usize,
    pub removed_keys: usize,
    pub added_keys: usize,
    /// After-records which are changed-owner records, new identities, or have
    /// changed geometry despite a surviving stable key.
    pub conservative_refresh_after: usize,
    /// After-records whose key, blocker ownership, and relevant geometry all
    /// survived. This is diagnostic reuse potential, not a reuse certificate.
    pub reuse_candidates_after: usize,
}

/// Compare two independently rebuilt graphs after a declared obstacle change.
///
/// The function deliberately does not mutate or splice either graph. The raw
/// CDT and cut basis are global constructions, and downstream embeddings are
/// tied to the graph placement revision. Until those dependencies have local
/// validity certificates, this report is the safe boundary for measuring
/// blocker-owned locality without admitting stale geometry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CorridorRebuildDelta {
    pub layer: String,
    pub changed_obstacles: Vec<String>,
    pub full_rebuild_work: CorridorRebuildWork,
    pub regions: CorridorIdentitySurvival,
    pub passages: CorridorIdentitySurvival,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateClearanceModel {
    /// Useful as a topological-routing baseline, but not yet a certificate for
    /// arbitrary-clearance motion through the two incident triangles.
    PortalLengthBaseline,
}

#[derive(Clone, Debug)]
pub struct RouteEmbeddingRequest<'a> {
    pub net: &'a str,
    pub from_component: &'a str,
    pub to_component: &'a str,
    pub physical_width: f64,
    pub clearance: f64,
    pub from: Vec2,
    pub from_toward: Vec2,
    pub to: Vec2,
    pub to_toward: Vec2,
    pub reference: &'a [Vec2],
    pub target_homotopy: Option<&'a HomotopyWord>,
    pub persistent_basis_matches: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteCorridorEmbedding {
    pub net: String,
    /// Stable identity of one maximal same-layer run within the electrical net.
    pub route_run: String,
    pub run_index: usize,
    pub start_point: usize,
    pub end_point: usize,
    pub complete_route: bool,
    pub layer: String,
    pub corridor_revision: u64,
    pub placement_revision: u64,
    pub required_width: f64,
    /// Distinguishes a cheap topological/centerline witness from a copper-width
    /// certificate. Both reserve the declared width during joint allocation;
    /// only `full_width_certificate` is final routing evidence.
    pub purpose: EmbeddingPurpose,
    pub cells: Vec<usize>,
    pub gates: Vec<usize>,
    pub semantic_regions: Vec<usize>,
    pub semantic_passages: Vec<usize>,
    pub polyline: Vec<Vec2>,
    pub method: EmbeddingMethod,
    pub geometry_method: GeometryMethod,
    pub route_class_verified: bool,
    /// The run's topology is certified in the current corridor epoch, even
    /// when it is not itself a complete persistent pin-to-pin route class.
    pub run_class_verified: bool,
    pub basis_mapping_required: bool,
    pub reference_homotopy: HomotopyWord,
    pub embedded_homotopy: HomotopyWord,
    pub epoch_homotopy_verified: bool,
    pub clearance: ClearanceCertificate,
    pub repair: Option<RouteRepairEvidence>,
    /// Typed evidence for attributing a geometrically certified route to the
    /// disposable corridor graph. This is present when endpoint selection or
    /// certified-reference projection was attempted; it never substitutes for
    /// a successful raw cell/gate chain.
    pub projection: Option<RouteProjectionEvidence>,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RouteProjectionEvidence {
    pub source: RouteTerminalProjectionEvidence,
    pub target: RouteTerminalProjectionEvidence,
    pub ordinary_attempt: Option<RawProjectionAttemptEvidence>,
    pub gap_retry_attempt: Option<RawProjectionAttemptEvidence>,
    pub failure_reasons: Vec<RouteProjectionFailureReason>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RouteTerminalProjectionEvidence {
    /// Sorted, bounded raw-cell IDs considered by ordinary terminal selection.
    pub direct_candidate_cells: Vec<usize>,
    pub direct_candidate_count: usize,
    pub direct_selected_cell: Option<usize>,
    pub direct_selection: DirectTerminalSelection,
    /// Sorted, bounded raw-cell IDs at the earliest positive-measure interval
    /// after an endpoint-exempt gap.
    pub gap_anchor_candidate_cells: Vec<usize>,
    pub gap_anchor_candidate_count: usize,
    /// Bounded interval parameters on the terminal-to-toward segment. Point
    /// touches are absent; `span_parameter` is strictly positive.
    pub gap_anchor_intervals: Vec<TerminalIntervalEvidence>,
    /// Sorted, bounded semantic regions represented by those candidates.
    pub gap_anchor_regions: Vec<usize>,
    pub gap_anchor_region_count: usize,
    pub gap_anchor_selected_cell: Option<usize>,
    pub gap_anchor_outcome: GapAnchorOutcome,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TerminalIntervalEvidence {
    pub cell: usize,
    pub start_parameter: f64,
    pub span_parameter: f64,
    pub semantic_region: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectTerminalSelection {
    ContainsTerminal,
    FirstPositiveInterval,
    NearestFallback,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GapAnchorOutcome {
    KeepDirectNoGap,
    Unique,
    Missing,
    AmbiguousRegions,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RawProjectionAttemptEvidence {
    pub source_cell: usize,
    pub target_cell: usize,
    pub outcome: RawProjectionAttemptOutcome,
    /// Whether the two selected cells are connected in the complete raw dual,
    /// independent of this polyline.
    pub source_target_graph_connected: bool,
    /// Cells with positive-measure overlap with the polyline. Endpoint cells
    /// forced into the touched-cell fallback are not included in this count.
    pub positive_overlap_cell_count: usize,
    pub source_reachable_touched_cell_count: usize,
    pub target_reachable_touched_cell_count: usize,
    pub source_target_touched_connected: bool,
    /// Bounded positive-measure portions of the polyline covered by no raw
    /// free-space triangle.
    pub uncovered_intervals: Vec<PolylineProgressIntervalEvidence>,
    pub uncovered_interval_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PolylineProgressIntervalEvidence {
    pub segment_index: usize,
    pub start_parameter: f64,
    pub end_parameter: f64,
    pub covering_obstacles: Vec<UncoveredIntervalObstacleEvidence>,
    pub covering_obstacle_count: usize,
    /// True only when at least one exact obstacle covers the representative
    /// midpoint and every covering obstacle is explicitly exempt for one of
    /// this route's terminals.
    pub exclusively_endpoint_exempt: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UncoveredIntervalObstacleEvidence {
    pub obstacle: String,
    pub endpoint_exemption: EndpointExemptionMatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointExemptionMatch {
    From,
    To,
    Both,
    Neither,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RawProjectionAttemptOutcome {
    Connected,
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteProjectionFailureReason {
    SourceDirectCellMissing,
    TargetDirectCellMissing,
    OrdinaryRawChainDisconnected,
    SourceGapAnchorMissing,
    TargetGapAnchorMissing,
    SourceGapAnchorAmbiguous,
    TargetGapAnchorAmbiguous,
    GapRetryUnchanged,
    GapRetryRawChainDisconnected,
}

impl RouteCorridorEmbedding {
    pub fn usable_centerline_seed(&self) -> bool {
        self.purpose == EmbeddingPurpose::CenterlineSeed
            && self.failure.is_none()
            && self.clearance.certified
            && (self.epoch_homotopy_verified
                || (self.method == EmbeddingMethod::CertifiedReferenceContinuation
                    && matches!(
                        self.geometry_method,
                        GeometryMethod::CertifiedReference
                            | GeometryMethod::CertifiedReferenceGapAnchorRetry
                    )))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingPurpose {
    FullWidthCertificate,
    CenterlineSeed,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteRepairEvidence {
    pub method: RepairMethod,
    pub outcome: RepairOutcome,
    pub nodes: Vec<RepairNode>,
    pub edges: Vec<RepairEdge>,
    pub rejected_edges: usize,
    /// Undirected node pairs whose full clearance and cut signatures were
    /// actually evaluated. With lazy visibility this is deliberately distinct
    /// from both the implicit all-pairs graph and the admitted `edges`.
    pub evaluated_edge_candidates: usize,
    /// Stable exhaustive-equivalent pair-by-feature work charged by the lazy
    /// graph. Internal spatial pruning deliberately does not change this
    /// public budget contract.
    pub visibility_geometry_units: usize,
    /// Versioned algorithm and work-accounting evidence for the visibility
    /// search. `explored_states` remains the legacy heap-pop count; new
    /// consumers should use these explicit counters and budget semantics.
    pub search_work: RouteRepairSearchWork,
    pub explored_states: usize,
    /// Exact reason the deterministic alternative search stopped. Consumers
    /// which certify a bounded first-K frontier must inspect this field rather
    /// than infer termination from `search_complete` and result count.
    pub termination: AlternativeSearchTermination,
    /// True only when the visibility search stopped at its hard state ceiling;
    /// `search_complete == false` can also mean enough alternatives were found.
    #[serde(default)]
    pub state_limit_reached: bool,
    /// True when actual lazy visibility construction reached a deterministic
    /// work ceiling. Any already-materialized graph is bounded partial evidence
    /// and cannot support a geometric-infeasibility claim.
    #[serde(default)]
    pub visibility_graph_limit_reached: bool,
    pub alternatives: Vec<RepairAlternative>,
    pub selected_alternative: Option<usize>,
    pub search_complete: bool,
    pub limitations: Vec<String>,
}

/// Deterministic work ceilings for one bounded route-repair call.
///
/// Settled states bound the final visibility-search attempt, whether that is a
/// node-only first-witness search or an explicit homotopy-product search.
/// Visibility limits are cumulative across a discarded Euclidean-heuristic
/// attempt and its zero-heuristic fallback.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouteRepairWorkBudget {
    pub settled_states: usize,
    pub visibility_pair_evaluations: usize,
    pub visibility_geometry_units: usize,
}

impl Default for RouteRepairWorkBudget {
    fn default() -> Self {
        Self {
            settled_states: DEFAULT_ROUTE_REPAIR_SETTLED_STATE_BUDGET,
            visibility_pair_evaluations: MAX_REPAIR_VISIBILITY_PAIR_EVALUATIONS,
            visibility_geometry_units: MAX_REPAIR_VISIBILITY_GEOMETRY_UNITS,
        }
    }
}

/// Cumulative visibility work performed by a bounded route-repair call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RouteRepairVisibilityWork {
    pub pair_evaluation_limit: usize,
    pub pair_evaluations: usize,
    pub geometry_unit_limit: usize,
    pub geometry_units: usize,
    pub attempts: usize,
}

/// Heavy route evidence paired with whole-call visibility work accounting.
#[derive(Clone, Debug, Serialize)]
pub struct BoundedRouteRepairEvidence {
    pub evidence: RouteRepairEvidence,
    pub visibility_work: RouteRepairVisibilityWork,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairSearchContract {
    /// One cheapest label per visibility node. The exact homotopy word is
    /// computed and certified only after a geometric target witness settles.
    NodeOnlyEuclideanAstarV1,
    /// Product state `(visibility node, homotopy word)`, retained for an exact
    /// required word and explicit route-family enumeration.
    CanonicalEuclideanAstarV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairSearchBudgetKind {
    SettledStates,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RouteRepairSearchWork {
    pub contract: RepairSearchContract,
    pub budget_kind: RepairSearchBudgetKind,
    /// Effective number of canonical goal families requested from this
    /// search. Persistent exact-word embedding always records one.
    pub goal_limit: usize,
    pub budget_limit: usize,
    pub budget_consumed: usize,
    /// Includes stale entries. Every queued non-root label comes from one
    /// accepted relaxation, so this remains bounded by `accepted_labels + 1`
    /// even though stale pops do not consume the settled-state budget.
    pub heap_pops: usize,
    pub stale_pops: usize,
    pub settled_states: usize,
    pub expanded_states: usize,
    /// Bounded naturally by `settled_states * maximum visibility degree`;
    /// goal states consume budget but are not expanded.
    pub relaxed_edges: usize,
    pub accepted_labels: usize,
    pub heuristic_evaluations: usize,
    pub heuristic_fallback_to_zero: bool,
    pub goals_settled: usize,
    pub boundary_goals: usize,
    pub threshold_extra_pops: usize,
    pub threshold_complete: bool,
    pub exact_cost_hop_path_comparisons: usize,
    pub lexicographic_improvements: usize,
    pub identical_path_ties: usize,
    pub materialized_path_elements: usize,
    pub output_materialized_path_elements: usize,
    pub maximum_path_depth: usize,
    pub path_arena_nodes: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlternativeSearchTermination {
    SearchExhausted,
    /// The versioned canonical search completed every open state at or below
    /// the Kth certified goal's exact cost threshold, then returned the
    /// canonical first-K subset without exhausting more expensive states.
    FamilyLimitReached,
    /// The settled-state budget in `RouteRepairSearchWork` was consumed before
    /// search exhaustion or completion of a reached K boundary.
    StateLimitReached,
    VisibilityGraphLimitReached,
}

pub const FIRST_K_ROUTE_CLASS_FAMILY_LIMIT: usize = 3;
/// Hard public ceiling for bounded planner exploration. Production normally
/// stops at eight and may use the remaining range only for context-guided
/// deepening of variables named by an exact bounded-infeasibility witness.
pub const MAX_EXPLORATORY_ROUTE_CLASS_FAMILY_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteClassFamilyLimitError {
    pub requested: usize,
    pub minimum: usize,
    pub maximum: usize,
}

impl std::fmt::Display for RouteClassFamilyLimitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "route-class family limit {} is outside the supported range {}..={}",
            self.requested, self.minimum, self.maximum
        )
    }
}

impl std::error::Error for RouteClassFamilyLimitError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairMethod {
    ClearanceOffsetVisibilityGraph,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairOutcome {
    Repaired,
    SearchExhausted,
    ProjectionFailed,
}

#[derive(Clone, Debug, Serialize)]
pub struct RepairNode {
    pub id: usize,
    pub position: Vec2,
    pub kind: RepairNodeKind,
    pub feature: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairNodeKind {
    Source,
    Target,
    ClearanceCorner,
}

#[derive(Clone, Debug, Serialize)]
pub struct RepairEdge {
    pub from: usize,
    pub to: usize,
    pub minimum_clearance: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct RepairAlternative {
    pub polyline: Vec<Vec2>,
    pub length: f64,
    pub homotopy: HomotopyWord,
    pub minimum_clearance: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CutBasis {
    pub revision: u64,
    pub placement_revision: u64,
    /// Connected components of forbidden space.  Overlapping pads, package
    /// bodies, and keepouts are one topological obstacle even though exact
    /// clearance continues to inspect every member polygon independently.
    pub components: Vec<TopologyObstacleComponent>,
    pub cuts: Vec<TopologyCut>,
    /// Coordinate-free discrete forest identity for cheap search dedup only.
    /// It is not sufficient to persist a route word across placement motion.
    pub structural_fingerprint: String,
    /// Exact beam-basis epoch, including every endpoint as canonical f64 bits.
    /// Equality is necessary, but not sufficient, for persistence: a current
    /// route/terminal embedding must still certify its crossing events.
    pub fingerprint: String,
    pub complete: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TopologyObstacleComponent {
    pub id: String,
    pub members: Vec<String>,
    pub board_attached: bool,
    pub board_contacts: Vec<BoardBoundarySide>,
    /// Board-attached forbidden space belongs to the exterior and therefore
    /// has no hole generator.
    pub generator: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardBoundarySide {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Debug, Serialize)]
pub struct TopologyCut {
    pub generator: String,
    pub source_component: String,
    pub segment: [Vec2; 2],
    pub target: TopologyCutTarget,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TopologyCutTarget {
    BoardBottom,
    Component { component: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct ClearanceCertificate {
    pub model: ClearanceModel,
    pub required_radius: f64,
    pub certified: bool,
    pub minimum_clearance: f64,
    pub failures: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClearanceModel {
    ExactPolylineAgainstPolygonalObstacles,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingMethod {
    /// The current engine geometry is already an exact-clearance witness. It is
    /// projected onto the disposable graph without changing the geometry.
    CertifiedReferenceContinuation,
    ClearanceVisibilityRepair,
    ReferenceBiasedShortestPath,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryMethod {
    CertifiedReference,
    /// The unchanged certified reference needed endpoint anchors after an
    /// endpoint-exempt free-space gap because ordinary raw projection failed.
    CertifiedReferenceGapAnchorRetry,
    ClearanceVisibilityGraph,
    ClearanceInsetFunnel,
    CorridorUnionShortcut,
    CellCentersFallback,
}

#[derive(Clone)]
struct InputVertex {
    point: Point2<f64>,
    feature: String,
}

impl spade::HasPosition for InputVertex {
    type Scalar = f64;

    fn position(&self) -> Point2<Self::Scalar> {
        self.point
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BoundaryPointKey {
    x_bits: u64,
    y_bits: u64,
}

impl BoundaryPointKey {
    fn new(point: Vec2) -> Self {
        Self {
            x_bits: canonical_coordinate_bits(point.x),
            y_bits: canonical_coordinate_bits(point.y),
        }
    }

    fn feature(self) -> String {
        format!("intersection:{:016x}:{:016x}", self.x_bits, self.y_bits)
    }
}

#[derive(Clone, Debug)]
struct BoundaryEdgeInput {
    board: bool,
    owner: String,
    edge: usize,
    start: Vec2,
    end: Vec2,
}

#[derive(Clone, Debug)]
struct AtomicBoundaryEdge {
    board: bool,
    owner: String,
    edge: usize,
    ordinal: usize,
    start: Vec2,
    end: Vec2,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BoundaryPreparationWork {
    input_edges: usize,
    atomic_edges: usize,
    duplicate_atomic_edges: usize,
    shared_endpoint_pairs: usize,
    collinear_overlap_pairs: usize,
}

pub fn build_corridor_graph(
    layer: &str,
    board: Rect,
    obstacles: &[CorridorObstacle],
    placement_revision: u64,
    revision: u64,
) -> Result<CorridorGraph, CorridorBuildError> {
    let (canonical_obstacles, atomic_boundaries, point_features, boundary_work) =
        prepare_boundary_constraints(board, obstacles)?;
    let cut_basis = build_cut_basis(board, &canonical_obstacles, placement_revision, revision);
    let triangulation = build_incremental_cdt(&atomic_boundaries, &point_features)?;

    let mut cells = Vec::new();
    let mut face_to_cell = HashMap::new();
    for face in triangulation.inner_faces() {
        let triangle = face.positions().map(point_from_spade);
        let center = triangle_center(triangle);
        if canonical_obstacles
            .iter()
            .any(|obstacle| point_in_convex_polygon(center, &obstacle.polygon))
        {
            continue;
        }
        let mut boundary_features = face
            .adjacent_edges()
            .into_iter()
            .map(|edge| boundary_feature_owner(&edge.vertices()[0].data().feature))
            .collect::<Vec<_>>();
        boundary_features.sort();
        boundary_features.dedup();
        let id = cells.len();
        face_to_cell.insert(face.fix().index(), id);
        cells.push(CorridorCell {
            id,
            triangle,
            center,
            boundary_features,
        });
    }

    let mut gates = Vec::new();
    for face in triangulation.inner_faces() {
        let Some(&from_cell) = face_to_cell.get(&face.fix().index()) else {
            continue;
        };
        for edge in face.adjacent_edges() {
            if edge.as_undirected().is_constraint_edge() {
                continue;
            }
            let Some(other_face) = edge.rev().face().as_inner() else {
                continue;
            };
            let Some(&to_cell) = face_to_cell.get(&other_face.fix().index()) else {
                continue;
            };
            if from_cell >= to_cell {
                continue;
            }
            let [from_vertex, to_vertex] = edge.vertices();
            let segment = edge.positions().map(point_from_spade);
            let nominal_width = length(sub(segment[1], segment[0]));
            gates.push(CorridorGate {
                id: gates.len(),
                key: GateKey::new(
                    layer,
                    from_vertex.data().feature.clone(),
                    to_vertex.data().feature.clone(),
                    GateDirection::Forward,
                ),
                cells: [from_cell, to_cell],
                segment,
                nominal_width,
                clearance_model: GateClearanceModel::PortalLengthBaseline,
            });
        }
    }

    let semantic = reduce_corridor_graph(
        layer,
        placement_revision,
        revision,
        &cut_basis,
        &cells,
        &gates,
    );
    Ok(CorridorGraph {
        revision,
        placement_revision,
        layer: layer.into(),
        board,
        obstacles: canonical_obstacles,
        cut_basis,
        cells,
        gates,
        semantic,
        warnings: {
            vec![
                "gate capacity uses raw CDT portal length as an incomplete search filter; accepted embedding polylines receive a separate exact clearance certificate".into(),
                format!(
                    "incremental split CDT retained all {} declared boundary edges as {} atomic constraints ({} duplicate atomic edges merged, {} shared-endpoint pairs, {} collinear-overlap pairs)",
                    boundary_work.input_edges,
                    boundary_work.atomic_edges,
                    boundary_work.duplicate_atomic_edges,
                    boundary_work.shared_endpoint_pairs,
                    boundary_work.collinear_overlap_pairs,
                ),
            ]
        },
    })
}

fn prepare_boundary_constraints(
    board: Rect,
    obstacles: &[CorridorObstacle],
) -> Result<
    (
        Vec<CorridorObstacle>,
        Vec<AtomicBoundaryEdge>,
        BTreeMap<BoundaryPointKey, String>,
        BoundaryPreparationWork,
    ),
    CorridorBuildError,
> {
    if ![board.min.x, board.min.y, board.max.x, board.max.y]
        .into_iter()
        .all(f64::is_finite)
        || board.min.x >= board.max.x
        || board.min.y >= board.max.y
    {
        return Err(CorridorBuildError::InvalidBoardBounds);
    }
    let mut canonical_obstacles = obstacles.to_vec();
    canonical_obstacles.sort_by(|first, second| first.id.cmp(&second.id));
    for pair in canonical_obstacles.windows(2) {
        if pair[0].id == pair[1].id {
            return Err(CorridorBuildError::DuplicateObstacleId {
                obstacle: pair[0].id.clone(),
            });
        }
    }

    let board_corners = [
        Vec2::new(board.min.x, board.min.y),
        Vec2::new(board.max.x, board.min.y),
        Vec2::new(board.max.x, board.max.y),
        Vec2::new(board.min.x, board.max.y),
    ];
    let mut edges = Vec::new();
    let mut feature_candidates = BTreeMap::<BoundaryPointKey, BTreeSet<String>>::new();
    push_boundary_edges(
        &mut edges,
        &mut feature_candidates,
        true,
        "board",
        &board_corners,
    )?;
    for obstacle in &canonical_obstacles {
        if obstacle.polygon.len() < 3 {
            return Err(CorridorBuildError::PolygonHasTooFewVertices {
                obstacle: obstacle.id.clone(),
                vertices: obstacle.polygon.len(),
            });
        }
        push_boundary_edges(
            &mut edges,
            &mut feature_candidates,
            false,
            &format!("component:{}", obstacle.id),
            &obstacle.polygon,
        )?;
    }

    let mut work = BoundaryPreparationWork {
        input_edges: edges.len(),
        ..BoundaryPreparationWork::default()
    };
    let mut split_points = edges
        .iter()
        .map(|edge| vec![edge.start, edge.end])
        .collect::<Vec<_>>();
    for first in 0..edges.len() {
        for second in (first + 1)..edges.len() {
            let (before_second, at_second) = split_points.split_at_mut(second);
            inspect_boundary_edge_pair(
                &edges[first],
                &edges[second],
                &mut before_second[first],
                &mut at_second[0],
                &mut work,
            )?;
        }
    }

    let mut atomic = Vec::new();
    for (edge, mut points) in edges.into_iter().zip(split_points) {
        let direction = sub(edge.end, edge.start);
        let denominator = dot(direction, direction);
        points.sort_by(|first, second| {
            boundary_edge_parameter(edge.start, direction, denominator, *first)
                .total_cmp(&boundary_edge_parameter(
                    edge.start,
                    direction,
                    denominator,
                    *second,
                ))
                .then_with(|| BoundaryPointKey::new(*first).cmp(&BoundaryPointKey::new(*second)))
        });
        points.dedup_by(|first, second| {
            BoundaryPointKey::new(*first) == BoundaryPointKey::new(*second)
        });
        for (ordinal, pair) in points.windows(2).enumerate() {
            let edge_length = length(sub(pair[1], pair[0]));
            if edge_length <= MIN_BOUNDARY_EDGE_LENGTH {
                return Err(CorridorBuildError::NearDegenerateBoundaryEdge {
                    owner: edge.owner.clone(),
                    edge: edge.edge,
                    length_bits: edge_length.to_bits(),
                });
            }
            atomic.push(AtomicBoundaryEdge {
                board: edge.board,
                owner: edge.owner.clone(),
                edge: edge.edge,
                ordinal,
                start: pair[0],
                end: pair[1],
            });
        }
    }
    atomic.sort_by(|first, second| {
        (!first.board)
            .cmp(&(!second.board))
            .then_with(|| first.owner.cmp(&second.owner))
            .then_with(|| first.edge.cmp(&second.edge))
            .then_with(|| first.ordinal.cmp(&second.ordinal))
    });
    let mut seen_atomic = BTreeSet::new();
    atomic.retain(|edge| {
        let key = canonical_boundary_edge_key(edge.start, edge.end);
        if seen_atomic.insert(key) {
            true
        } else {
            work.duplicate_atomic_edges += 1;
            false
        }
    });
    work.atomic_edges = atomic.len();

    for edge in &atomic {
        for point in [edge.start, edge.end] {
            feature_candidates
                .entry(BoundaryPointKey::new(point))
                .or_default();
        }
    }
    let point_features = feature_candidates
        .into_iter()
        .map(|(point, features)| {
            let feature = if features.len() == 1 {
                features.into_iter().next().unwrap()
            } else {
                point.feature()
            };
            (point, feature)
        })
        .collect();
    Ok((canonical_obstacles, atomic, point_features, work))
}

fn push_boundary_edges(
    edges: &mut Vec<BoundaryEdgeInput>,
    feature_candidates: &mut BTreeMap<BoundaryPointKey, BTreeSet<String>>,
    board: bool,
    owner: &str,
    polygon: &[Vec2],
) -> Result<(), CorridorBuildError> {
    for (index, &point) in polygon.iter().enumerate() {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(CorridorBuildError::NonFiniteBoundaryVertex {
                owner: owner.to_owned(),
                vertex: index,
            });
        }
        feature_candidates
            .entry(BoundaryPointKey::new(point))
            .or_default()
            .insert(format!("{owner}:corner:{index}"));
    }
    for index in 0..polygon.len() {
        let start = polygon[index];
        let end = polygon[(index + 1) % polygon.len()];
        let edge_length = length(sub(end, start));
        if !edge_length.is_finite() {
            return Err(CorridorBuildError::NonFiniteBoundaryEdge {
                owner: owner.to_owned(),
                edge: index,
            });
        }
        if edge_length <= MIN_BOUNDARY_EDGE_LENGTH {
            return Err(CorridorBuildError::NearDegenerateBoundaryEdge {
                owner: owner.to_owned(),
                edge: index,
                length_bits: edge_length.to_bits(),
            });
        }
        edges.push(BoundaryEdgeInput {
            board,
            owner: owner.to_owned(),
            edge: index,
            start,
            end,
        });
    }
    Ok(())
}

fn inspect_boundary_edge_pair(
    first: &BoundaryEdgeInput,
    second: &BoundaryEdgeInput,
    first_splits: &mut Vec<Vec2>,
    second_splits: &mut Vec<Vec2>,
    work: &mut BoundaryPreparationWork,
) -> Result<(), CorridorBuildError> {
    let first_points = [first.start, first.end];
    let second_points = [second.start, second.end];
    let shares_endpoint = first_points.iter().any(|first| {
        second_points
            .iter()
            .any(|second| BoundaryPointKey::new(*first) == BoundaryPointKey::new(*second))
    });
    if shares_endpoint {
        work.shared_endpoint_pairs += 1;
    }

    let first_delta = sub(first.end, first.start);
    let second_delta = sub(second.end, second.start);
    let denominator = cross(first_delta, second_delta);
    let length_scale = length(first_delta) * length(second_delta);
    let offset = sub(second.start, first.start);
    let collinear = denominator.abs() <= EPSILON * length_scale
        && cross(first_delta, offset).abs()
            <= EPSILON * length(first_delta) * length(offset).max(1.0);
    if collinear {
        let first_denominator = dot(first_delta, first_delta);
        let second_parameters = second_points.map(|point| {
            boundary_edge_parameter(first.start, first_delta, first_denominator, point)
        });
        let overlap_start = 0.0_f64.max(second_parameters[0].min(second_parameters[1]));
        let overlap_end = 1.0_f64.min(second_parameters[0].max(second_parameters[1]));
        if overlap_end - overlap_start > MIN_BOUNDARY_EDGE_LENGTH / length(first_delta) {
            work.collinear_overlap_pairs += 1;
        }
        for point in second_points {
            if point_on_boundary_segment(point, first.start, first.end) {
                first_splits.push(point);
            }
        }
        for point in first_points {
            if point_on_boundary_segment(point, second.start, second.end) {
                second_splits.push(point);
            }
        }
        return Ok(());
    }
    if denominator.abs() <= EPSILON * length_scale {
        return Ok(());
    }

    let first_parameter = cross(offset, second_delta) / denominator;
    let second_parameter = cross(offset, first_delta) / denominator;
    if !(-BOUNDARY_INTERSECTION_PARAMETER_EPSILON..=1.0 + BOUNDARY_INTERSECTION_PARAMETER_EPSILON)
        .contains(&first_parameter)
        || !(-BOUNDARY_INTERSECTION_PARAMETER_EPSILON
            ..=1.0 + BOUNDARY_INTERSECTION_PARAMETER_EPSILON)
            .contains(&second_parameter)
    {
        return Ok(());
    }

    let near_first_endpoint = first_parameter <= BOUNDARY_INTERSECTION_PARAMETER_EPSILON
        || first_parameter >= 1.0 - BOUNDARY_INTERSECTION_PARAMETER_EPSILON;
    let near_second_endpoint = second_parameter <= BOUNDARY_INTERSECTION_PARAMETER_EPSILON
        || second_parameter >= 1.0 - BOUNDARY_INTERSECTION_PARAMETER_EPSILON;
    if near_first_endpoint || near_second_endpoint {
        let mut resolved = false;
        for point in first_points {
            if point_on_boundary_segment(point, second.start, second.end) {
                second_splits.push(point);
                resolved = true;
            }
        }
        for point in second_points {
            if point_on_boundary_segment(point, first.start, first.end) {
                first_splits.push(point);
                resolved = true;
            }
        }
        if !resolved && !shares_endpoint {
            return Err(CorridorBuildError::NearDegenerateBoundaryIntersection {
                first_owner: first.owner.clone(),
                first_edge: first.edge,
                second_owner: second.owner.clone(),
                second_edge: second.edge,
            });
        }
    }
    Ok(())
}

fn build_incremental_cdt(
    atomic_boundaries: &[AtomicBoundaryEdge],
    point_features: &BTreeMap<BoundaryPointKey, String>,
) -> Result<ConstrainedDelaunayTriangulation<InputVertex>, CorridorBuildError> {
    let mut triangulation = ConstrainedDelaunayTriangulation::<InputVertex>::new();
    for boundary in atomic_boundaries {
        let from = ensure_boundary_vertex(&mut triangulation, boundary.start, point_features)?;
        let to = ensure_boundary_vertex(&mut triangulation, boundary.end, point_features)?;
        let result = catch_unwind(AssertUnwindSafe(|| {
            triangulation.add_constraint_and_split(from, to, |point| InputVertex {
                point,
                feature: BoundaryPointKey {
                    x_bits: canonical_coordinate_bits(point.x),
                    y_bits: canonical_coordinate_bits(point.y),
                }
                .feature(),
            })
        }));
        let constraint_edges =
            result.map_err(|_| CorridorBuildError::ConstraintInsertionPanicked {
                owner: boundary.owner.clone(),
                edge: boundary.edge,
            })?;
        if constraint_edges.is_empty() {
            return Err(CorridorBuildError::ConstraintInsertionEmpty {
                owner: boundary.owner.clone(),
                edge: boundary.edge,
            });
        }
        if constraint_edges.iter().any(|edge| {
            !triangulation
                .directed_edge(*edge)
                .as_undirected()
                .is_constraint_edge()
        }) {
            return Err(CorridorBuildError::ConstraintInsertionInvalid {
                owner: boundary.owner.clone(),
                edge: boundary.edge,
            });
        }
    }
    Ok(triangulation)
}

fn ensure_boundary_vertex(
    triangulation: &mut ConstrainedDelaunayTriangulation<InputVertex>,
    point: Vec2,
    point_features: &BTreeMap<BoundaryPointKey, String>,
) -> Result<FixedVertexHandle, CorridorBuildError> {
    let position = Point2::new(point.x, point.y);
    if let Some(existing) = triangulation.locate_vertex(position) {
        return Ok(existing.fix());
    }
    let key = BoundaryPointKey::new(point);
    let feature = point_features
        .get(&key)
        .cloned()
        .unwrap_or_else(|| key.feature());
    triangulation
        .insert(InputVertex {
            point: position,
            feature: feature.clone(),
        })
        .map_err(|error| CorridorBuildError::TriangulationInsertion {
            feature,
            error: error.to_string(),
        })
}

fn canonical_boundary_edge_key(start: Vec2, end: Vec2) -> (BoundaryPointKey, BoundaryPointKey) {
    let start = BoundaryPointKey::new(start);
    let end = BoundaryPointKey::new(end);
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

fn canonical_coordinate_bits(value: f64) -> u64 {
    if value == 0.0 { 0 } else { value.to_bits() }
}

fn boundary_edge_parameter(start: Vec2, delta: Vec2, denominator: f64, point: Vec2) -> f64 {
    dot(sub(point, start), delta) / denominator
}

fn point_on_boundary_segment(point: Vec2, start: Vec2, end: Vec2) -> bool {
    let delta = sub(end, start);
    let offset = sub(point, start);
    let segment_length = length(delta);
    cross(delta, offset).abs() <= EPSILON * segment_length * length(offset).max(1.0)
        && dot(offset, delta) >= -EPSILON
        && dot(sub(point, end), delta) <= EPSILON
}

fn polygon_boundaries_intersect(first: &[Vec2], second: &[Vec2]) -> bool {
    first.iter().enumerate().any(|(first_index, &first_start)| {
        let first_end = first[(first_index + 1) % first.len()];
        second
            .iter()
            .enumerate()
            .any(|(second_index, &second_start)| {
                let second_end = second[(second_index + 1) % second.len()];
                segment_distance(first_start, first_end, second_start, second_end) <= EPSILON
            })
    })
}

fn boundary_feature_owner(feature: &str) -> String {
    feature
        .rsplit_once(":corner:")
        .map(|(owner, _)| owner)
        .unwrap_or(feature)
        .to_owned()
}

fn triangle_area(triangle: [Vec2; 3]) -> f64 {
    cross(sub(triangle[1], triangle[0]), sub(triangle[2], triangle[0])).abs() * 0.5
}

#[derive(Debug)]
struct DisjointSets {
    parent: Vec<usize>,
}

impl DisjointSets {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
        }
    }

    fn find(&mut self, item: usize) -> usize {
        let parent = self.parent[item];
        if parent != item {
            self.parent[item] = self.find(parent);
        }
        self.parent[item]
    }

    fn union(&mut self, first: usize, second: usize) {
        let first = self.find(first);
        let second = self.find(second);
        if first != second {
            let (keep, replace) = if first < second {
                (first, second)
            } else {
                (second, first)
            };
            self.parent[replace] = keep;
        }
    }
}

#[derive(Debug)]
struct RegionCandidate {
    boundary_features: Vec<String>,
    raw_cells: Vec<usize>,
    center: Vec2,
    area: f64,
}

fn gate_topology_cuts(
    gate: &CorridorGate,
    cells: &[CorridorCell],
    cut_basis: &CutBasis,
) -> Vec<String> {
    let transition = [cells[gate.cells[0]].center, cells[gate.cells[1]].center];
    let mut cuts = cut_basis
        .cuts
        .iter()
        .filter_map(
            |cut| match segment_cut_crossing(transition[0], transition[1], cut) {
                CutIntersection::Crossing { .. } | CutIntersection::Ambiguous => {
                    Some(cut.generator.clone())
                }
                CutIntersection::None => None,
            },
        )
        .collect::<Vec<_>>();
    cuts.sort();
    cuts.dedup();
    cuts
}

fn reduce_corridor_graph(
    layer: &str,
    placement_revision: u64,
    revision: u64,
    cut_basis: &CutBasis,
    cells: &[CorridorCell],
    gates: &[CorridorGate],
) -> SemanticCorridorGraph {
    let gate_cuts = gates
        .iter()
        .map(|gate| gate_topology_cuts(gate, cells, cut_basis))
        .collect::<Vec<_>>();
    let mut sets = DisjointSets::new(cells.len());
    for gate in gates {
        if cells[gate.cells[0]].boundary_features == cells[gate.cells[1]].boundary_features {
            sets.union(gate.cells[0], gate.cells[1]);
        }
    }

    let mut cells_by_root = BTreeMap::<usize, Vec<usize>>::new();
    for cell in cells {
        cells_by_root
            .entry(sets.find(cell.id))
            .or_default()
            .push(cell.id);
    }
    let mut candidates = cells_by_root
        .into_values()
        .map(|raw_cells| {
            let boundary_features = cells[raw_cells[0]].boundary_features.clone();
            let area = raw_cells
                .iter()
                .map(|&cell| triangle_area(cells[cell].triangle))
                .sum::<f64>();
            let weighted_center = raw_cells.iter().fold(Vec2::ZERO, |sum, &cell| {
                add(
                    sum,
                    scale(cells[cell].center, triangle_area(cells[cell].triangle)),
                )
            });
            RegionCandidate {
                boundary_features,
                raw_cells,
                center: if area <= EPSILON {
                    Vec2::ZERO
                } else {
                    scale(weighted_center, 1.0 / area)
                },
                area,
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|first, second| {
        first
            .boundary_features
            .cmp(&second.boundary_features)
            .then_with(|| first.center.x.total_cmp(&second.center.x))
            .then_with(|| first.center.y.total_cmp(&second.center.y))
    });

    let mut signature_ordinals = BTreeMap::<Vec<String>, usize>::new();
    let mut raw_cell_to_region = vec![0; cells.len()];
    let mut regions = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let ordinal = signature_ordinals
            .entry(candidate.boundary_features.clone())
            .and_modify(|ordinal| *ordinal += 1)
            .or_insert(0);
        let id = regions.len();
        for &cell in &candidate.raw_cells {
            raw_cell_to_region[cell] = id;
        }
        regions.push(SemanticCorridorRegion {
            id,
            key: format!(
                "{}|{}|{}",
                layer,
                candidate.boundary_features.join("&"),
                *ordinal
            ),
            boundary_features: candidate.boundary_features,
            raw_cells: candidate.raw_cells,
            center: candidate.center,
            area: candidate.area,
        });
    }

    type PassageGroupKey = (usize, usize, String, String, String);
    let mut passage_groups = BTreeMap::<PassageGroupKey, Vec<usize>>::new();
    for gate in gates {
        let first_region = raw_cell_to_region[gate.cells[0]];
        let second_region = raw_cell_to_region[gate.cells[1]];
        if first_region == second_region && gate_cuts[gate.id].is_empty() {
            continue;
        }
        let mut features = [
            boundary_feature_owner(&gate.key.boundary_feature_a),
            boundary_feature_owner(&gate.key.boundary_feature_b),
        ];
        features.sort();
        let regions = if first_region <= second_region {
            [first_region, second_region]
        } else {
            [second_region, first_region]
        };
        passage_groups
            .entry((
                regions[0],
                regions[1],
                features[0].clone(),
                features[1].clone(),
                gate_cuts[gate.id].join("&"),
            ))
            .or_default()
            .push(gate.id);
    }

    let mut raw_gate_to_passage = vec![None; gates.len()];
    let mut passages = Vec::with_capacity(passage_groups.len());
    for ((first_region, second_region, first_feature, second_feature, cuts), mut raw_gates) in
        passage_groups
    {
        raw_gates.sort_unstable();
        let representative_gate = *raw_gates
            .iter()
            .min_by(|&&first, &&second| {
                gates[first]
                    .nominal_width
                    .total_cmp(&gates[second].nominal_width)
                    .then_with(|| first.cmp(&second))
            })
            .unwrap();
        let id = passages.len();
        for &gate in &raw_gates {
            raw_gate_to_passage[gate] = Some(id);
        }
        let topology_cuts = if cuts.is_empty() {
            Vec::new()
        } else {
            cuts.split('&').map(str::to_owned).collect()
        };
        let key = format!(
            "{}<->{}|{}&{}|{}",
            regions[first_region].key,
            regions[second_region].key,
            first_feature,
            second_feature,
            cuts
        );
        passages.push(SemanticCorridorPassage {
            id,
            key,
            regions: [first_region, second_region],
            limiting_features: [first_feature, second_feature],
            topology_cuts,
            raw_gates,
            representative_gate,
            segment: gates[representative_gate].segment,
            nominal_width: gates[representative_gate].nominal_width,
        });
    }

    let fingerprint = regions
        .iter()
        .map(|region| format!("r:{}", region.key))
        .chain(passages.iter().map(|passage| format!("p:{}", passage.key)))
        .collect::<Vec<_>>()
        .join("|");
    SemanticCorridorGraph {
        revision,
        placement_revision,
        layer: layer.into(),
        fingerprint,
        regions,
        passages,
        raw_cell_to_region,
        raw_gate_to_passage,
        warnings: vec![
            "semantic passages aggregate raw CDT portals by stable boundary ownership; passage width remains a conservative, uncertified raw-portal baseline".into(),
        ],
    }
}

fn feature_owned_by_changed_obstacle(feature: &str, changed: &BTreeSet<&str>) -> bool {
    changed.contains(feature)
        || feature
            .strip_prefix("component:")
            .is_some_and(|owner| changed.contains(owner))
}

fn region_owned_by_changed_obstacle(
    region: &SemanticCorridorRegion,
    changed: &BTreeSet<&str>,
) -> bool {
    region
        .boundary_features
        .iter()
        .any(|feature| feature_owned_by_changed_obstacle(feature, changed))
}

fn passage_owned_by_changed_obstacle(
    graph: &SemanticCorridorGraph,
    passage: &SemanticCorridorPassage,
    cut_basis: &CutBasis,
    changed: &BTreeSet<&str>,
) -> bool {
    passage
        .limiting_features
        .iter()
        .any(|feature| feature_owned_by_changed_obstacle(feature, changed))
        || passage.topology_cuts.iter().any(|generator| {
            cut_basis.components.iter().any(|component| {
                component.generator.as_deref() == Some(generator.as_str())
                    && component
                        .members
                        .iter()
                        .any(|member| changed.contains(member.as_str()))
            })
        })
        || passage
            .regions
            .iter()
            .any(|&region| region_owned_by_changed_obstacle(&graph.regions[region], changed))
}

fn same_point(first: Vec2, second: Vec2) -> bool {
    length(sub(first, second)) <= EPSILON
}

fn same_region_geometry(first: &SemanticCorridorRegion, second: &SemanticCorridorRegion) -> bool {
    same_point(first.center, second.center) && (first.area - second.area).abs() <= EPSILON
}

fn same_passage_geometry(
    first: &SemanticCorridorPassage,
    second: &SemanticCorridorPassage,
) -> bool {
    let same_segment = (same_point(first.segment[0], second.segment[0])
        && same_point(first.segment[1], second.segment[1]))
        || (same_point(first.segment[0], second.segment[1])
            && same_point(first.segment[1], second.segment[0]));
    same_segment && (first.nominal_width - second.nominal_width).abs() <= EPSILON
}

fn identity_survival<T>(
    before: &[T],
    after: &[T],
    key: impl Fn(&T) -> &str,
    affected_before: impl Fn(&T) -> bool,
    affected_after: impl Fn(&T) -> bool,
    same_geometry: impl Fn(&T, &T) -> bool,
) -> CorridorIdentitySurvival {
    let before_by_key = before
        .iter()
        .map(|item| (key(item), item))
        .collect::<BTreeMap<_, _>>();
    let after_by_key = after
        .iter()
        .map(|item| (key(item), item))
        .collect::<BTreeMap<_, _>>();
    let affected_before_count = before.iter().filter(|item| affected_before(item)).count();
    let affected_after_count = after.iter().filter(|item| affected_after(item)).count();
    let unaffected_before = before.len() - affected_before_count;
    let unaffected_keys_survived = before
        .iter()
        .filter(|item| !affected_before(item) && after_by_key.contains_key(key(item)))
        .count();
    let unaffected_geometry_stable = before
        .iter()
        .filter(|item| {
            !affected_before(item)
                && after_by_key
                    .get(key(item))
                    .is_some_and(|other| same_geometry(item, other))
        })
        .count();
    let removed_keys = before_by_key
        .keys()
        .filter(|key| !after_by_key.contains_key(*key))
        .count();
    let added_keys = after_by_key
        .keys()
        .filter(|key| !before_by_key.contains_key(*key))
        .count();
    let conservative_refresh_after = after
        .iter()
        .filter(|item| {
            affected_after(item)
                || before_by_key
                    .get(key(item))
                    .is_none_or(|old| !same_geometry(old, item))
        })
        .count();
    CorridorIdentitySurvival {
        before: before.len(),
        after: after.len(),
        affected_before: affected_before_count,
        affected_after: affected_after_count,
        unaffected_before,
        unaffected_keys_survived,
        unaffected_geometry_stable,
        removed_keys,
        added_keys,
        conservative_refresh_after,
        reuse_candidates_after: after.len() - conservative_refresh_after,
    }
}

pub fn analyze_corridor_rebuild(
    before: &CorridorGraph,
    after: &CorridorGraph,
    changed_obstacles: &[String],
) -> Result<CorridorRebuildDelta, String> {
    if before.layer != after.layer {
        return Err(format!(
            "cannot compare corridor layers {} and {}",
            before.layer, after.layer
        ));
    }
    let known_obstacles = before
        .obstacles
        .iter()
        .chain(&after.obstacles)
        .map(|obstacle| obstacle.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut changed_obstacles = changed_obstacles.to_vec();
    changed_obstacles.sort();
    changed_obstacles.dedup();
    if let Some(unknown) = changed_obstacles
        .iter()
        .find(|obstacle| !known_obstacles.contains(obstacle.as_str()))
    {
        return Err(format!("changed corridor obstacle {unknown} is unknown"));
    }
    let changed = changed_obstacles
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let regions = identity_survival(
        &before.semantic.regions,
        &after.semantic.regions,
        |region| region.key.as_str(),
        |region| region_owned_by_changed_obstacle(region, &changed),
        |region| region_owned_by_changed_obstacle(region, &changed),
        same_region_geometry,
    );
    let passages = identity_survival(
        &before.semantic.passages,
        &after.semantic.passages,
        |passage| passage.key.as_str(),
        |passage| {
            passage_owned_by_changed_obstacle(
                &before.semantic,
                passage,
                &before.cut_basis,
                &changed,
            )
        },
        |passage| {
            passage_owned_by_changed_obstacle(&after.semantic, passage, &after.cut_basis, &changed)
        },
        same_passage_geometry,
    );
    Ok(CorridorRebuildDelta {
        layer: before.layer.clone(),
        changed_obstacles,
        full_rebuild_work: CorridorRebuildWork {
            raw_cells_constructed: after.cells.len(),
            raw_gates_constructed: after.gates.len(),
            semantic_regions_reduced: after.semantic.regions.len(),
            semantic_passages_reduced: after.semantic.passages.len(),
        },
        regions,
        passages,
    })
}

fn build_cut_basis(
    board: Rect,
    obstacles: &[CorridorObstacle],
    placement_revision: u64,
    revision: u64,
) -> CutBasis {
    let working_components = connected_forbidden_components(board, obstacles);
    let components = working_components
        .iter()
        .map(|component| component.record.clone())
        .collect::<Vec<_>>();
    let mut cuts = Vec::<TopologyCut>::new();
    let mut warnings = Vec::new();
    let mut used_anchors = Vec::<f64>::new();

    for (component_index, component) in working_components.iter().enumerate() {
        let Some(generator) = component.record.generator.clone() else {
            continue;
        };
        if let Some((segment, target)) = vertical_cut_for_component(
            board,
            obstacles,
            &working_components,
            component_index,
            &used_anchors,
        ) {
            used_anchors.push(segment[0].x);
            cuts.push(TopologyCut {
                generator,
                source_component: component.record.id.clone(),
                segment,
                target,
            });
        } else {
            warnings.push(format!(
                "could not construct a distinct nondegenerate vertical cut for forbidden component {}",
                component.record.id
            ));
        }
    }

    warnings.extend(certify_vertical_cut_forest(
        board,
        obstacles,
        &working_components,
        &cuts,
    ));
    warnings.sort();
    warnings.dedup();
    let structural_fingerprint = cut_forest_structural_fingerprint(&components, &cuts);
    let fingerprint = cut_forest_exact_fingerprint(&structural_fingerprint, &cuts);
    CutBasis {
        revision,
        placement_revision,
        complete: warnings.is_empty(),
        components,
        cuts,
        structural_fingerprint,
        fingerprint,
        warnings,
    }
}

#[derive(Clone, Debug)]
struct WorkingTopologyComponent {
    record: TopologyObstacleComponent,
    obstacle_indices: Vec<usize>,
}

fn connected_forbidden_components(
    board: Rect,
    obstacles: &[CorridorObstacle],
) -> Vec<WorkingTopologyComponent> {
    let mut sets = DisjointSets::new(obstacles.len());
    for first in 0..obstacles.len() {
        for second in (first + 1)..obstacles.len() {
            if forbidden_polygons_connected(&obstacles[first].polygon, &obstacles[second].polygon) {
                sets.union(first, second);
            }
        }
    }
    let mut by_root = BTreeMap::<usize, Vec<usize>>::new();
    for obstacle_index in 0..obstacles.len() {
        by_root
            .entry(sets.find(obstacle_index))
            .or_default()
            .push(obstacle_index);
    }
    let mut components = by_root
        .into_values()
        .map(|mut obstacle_indices| {
            obstacle_indices
                .sort_by(|&first, &second| obstacles[first].id.cmp(&obstacles[second].id));
            let members = obstacle_indices
                .iter()
                .map(|&index| obstacles[index].id.clone())
                .collect::<Vec<_>>();
            let board_contacts = obstacle_indices
                .iter()
                .flat_map(|&index| polygon_board_contacts(board, &obstacles[index].polygon))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let board_attached = !board_contacts.is_empty();
            let id = membership_identity("forbidden", &members);
            let generator = (!board_attached).then(|| membership_identity("generator", &members));
            WorkingTopologyComponent {
                record: TopologyObstacleComponent {
                    id,
                    members,
                    board_attached,
                    board_contacts,
                    generator,
                },
                obstacle_indices,
            }
        })
        .collect::<Vec<_>>();
    components.sort_by(|first, second| first.record.id.cmp(&second.record.id));
    components
}

fn membership_identity(kind: &str, members: &[String]) -> String {
    let framed = members
        .iter()
        .map(|member| format!("{}:{member}", member.len()))
        .collect::<Vec<_>>()
        .join("|");
    format!("{kind}[{framed}]")
}

fn forbidden_polygons_connected(first: &[Vec2], second: &[Vec2]) -> bool {
    polygon_boundaries_intersect(first, second)
        || first
            .first()
            .is_some_and(|&point| point_in_convex_polygon(point, second))
        || second
            .first()
            .is_some_and(|&point| point_in_convex_polygon(point, first))
}

fn polygon_board_contacts(board: Rect, polygon: &[Vec2]) -> Vec<BoardBoundarySide> {
    let mut contacts = BTreeSet::new();
    for point in polygon {
        if point.x <= board.min.x + EPSILON {
            contacts.insert(BoardBoundarySide::Left);
        }
        if point.x >= board.max.x - EPSILON {
            contacts.insert(BoardBoundarySide::Right);
        }
        if point.y <= board.min.y + EPSILON {
            contacts.insert(BoardBoundarySide::Top);
        }
        if point.y >= board.max.y - EPSILON {
            contacts.insert(BoardBoundarySide::Bottom);
        }
    }
    contacts.into_iter().collect()
}

fn component_x_bounds(
    obstacles: &[CorridorObstacle],
    component: &WorkingTopologyComponent,
) -> Option<(f64, f64)> {
    let minimum = component
        .obstacle_indices
        .iter()
        .flat_map(|&index| obstacles[index].polygon.iter())
        .map(|point| point.x)
        .min_by(f64::total_cmp)?;
    let maximum = component
        .obstacle_indices
        .iter()
        .flat_map(|&index| obstacles[index].polygon.iter())
        .map(|point| point.x)
        .max_by(f64::total_cmp)?;
    Some((minimum, maximum))
}

fn vertical_boundary_intersections(polygon: &[Vec2], x: f64) -> Vec<f64> {
    let mut intersections = Vec::new();
    for index in 0..polygon.len() {
        let from = polygon[index];
        let to = polygon[(index + 1) % polygon.len()];
        if (to.x - from.x).abs() <= EPSILON
            || x < from.x.min(to.x) - EPSILON
            || x > from.x.max(to.x) + EPSILON
        {
            continue;
        }
        let parameter = (x - from.x) / (to.x - from.x);
        if (-EPSILON..=1.0 + EPSILON).contains(&parameter) {
            intersections.push(from.y + parameter * (to.y - from.y));
        }
    }
    intersections.sort_by(f64::total_cmp);
    intersections.dedup_by(|first, second| (*first - *second).abs() <= EPSILON);
    intersections
}

fn component_lower_boundary(
    obstacles: &[CorridorObstacle],
    component: &WorkingTopologyComponent,
    x: f64,
) -> Option<f64> {
    component
        .obstacle_indices
        .iter()
        .flat_map(|&index| vertical_boundary_intersections(&obstacles[index].polygon, x))
        .max_by(f64::total_cmp)
}

fn component_first_boundary_below(
    obstacles: &[CorridorObstacle],
    component: &WorkingTopologyComponent,
    x: f64,
    below: f64,
) -> Option<f64> {
    component
        .obstacle_indices
        .iter()
        .flat_map(|&index| vertical_boundary_intersections(&obstacles[index].polygon, x))
        .filter(|&y| y > below + EPSILON)
        .min_by(f64::total_cmp)
}

fn nondegenerate_vertical_anchor(obstacles: &[CorridorObstacle], x: f64) -> bool {
    obstacles.iter().all(|obstacle| {
        obstacle
            .polygon
            .iter()
            .all(|point| (point.x - x).abs() > EPSILON * 8.0)
    })
}

fn stable_anchor_offset(identity: &str, candidate_count: usize) -> usize {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in identity.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (hash % candidate_count as u64) as usize
}

fn vertical_cut_at_anchor(
    board: Rect,
    obstacles: &[CorridorObstacle],
    components: &[WorkingTopologyComponent],
    source_index: usize,
    x: f64,
) -> Option<([Vec2; 2], TopologyCutTarget)> {
    let source_y = component_lower_boundary(obstacles, &components[source_index], x)?;
    let mut target_y = board.max.y;
    let mut target = TopologyCutTarget::BoardBottom;
    for (component_index, component) in components.iter().enumerate() {
        if component_index == source_index {
            continue;
        }
        let Some(y) = component_first_boundary_below(obstacles, component, x, source_y) else {
            continue;
        };
        if y < target_y - EPSILON
            || ((y - target_y).abs() <= EPSILON
                && matches!(&target, TopologyCutTarget::Component { component: current } if component.record.id.as_str() < current.as_str()))
        {
            target_y = y;
            target = TopologyCutTarget::Component {
                component: component.record.id.clone(),
            };
        }
    }
    (target_y > source_y + EPSILON)
        .then_some(([Vec2::new(x, source_y), Vec2::new(x, target_y)], target))
}

fn vertical_cut_for_component(
    board: Rect,
    obstacles: &[CorridorObstacle],
    components: &[WorkingTopologyComponent],
    source_index: usize,
    used_anchors: &[f64],
) -> Option<([Vec2; 2], TopologyCutTarget)> {
    let component = &components[source_index];
    let (minimum_x, maximum_x) = component_x_bounds(obstacles, component)?;
    if maximum_x <= minimum_x + EPSILON * 32.0 {
        return None;
    }
    // There is no fixed candidate ceiling. Every forbidden vertex x and every
    // previously selected anchor contributes one excluded coordinate; the
    // open intervals between those finitely many coordinates form a complete
    // deterministic candidate set for a distinct non-vertex anchor.
    let mut excluded = obstacles
        .iter()
        .flat_map(|obstacle| obstacle.polygon.iter().map(|point| point.x))
        .chain(used_anchors.iter().copied())
        .filter(|x| *x > minimum_x && *x < maximum_x)
        .collect::<Vec<_>>();
    excluded.sort_by(f64::total_cmp);
    excluded.dedup_by(|first, second| (*first - *second).abs() <= EPSILON * 16.0);
    let mut boundaries = Vec::with_capacity(excluded.len() + 2);
    boundaries.push(minimum_x);
    boundaries.extend(excluded);
    boundaries.push(maximum_x);
    let preferred_fraction =
        (stable_anchor_offset(&component.record.id, 65_521) as f64 + 0.5) / 65_521.0;
    let preferred_x = minimum_x + preferred_fraction * (maximum_x - minimum_x);
    let mut candidates = boundaries
        .windows(2)
        .filter(|bounds| bounds[1] > bounds[0] + EPSILON * 32.0)
        .map(|bounds| (bounds[0] + bounds[1]) * 0.5)
        .collect::<Vec<_>>();
    candidates.push(preferred_x);
    candidates.sort_by(|first, second| {
        (*first - preferred_x)
            .abs()
            .total_cmp(&(*second - preferred_x).abs())
            .then_with(|| first.total_cmp(second))
    });
    candidates.dedup_by(|first, second| (*first - *second).abs() <= EPSILON * 16.0);
    for x in candidates {
        if used_anchors
            .iter()
            .any(|used| (used - x).abs() <= EPSILON * 16.0)
            || !nondegenerate_vertical_anchor(obstacles, x)
        {
            continue;
        }
        if let Some(cut) = vertical_cut_at_anchor(board, obstacles, components, source_index, x) {
            return Some(cut);
        }
    }
    None
}

fn cut_forest_structural_fingerprint(
    components: &[TopologyObstacleComponent],
    cuts: &[TopologyCut],
) -> String {
    let component_records = components
        .iter()
        .map(|component| {
            let contacts = component
                .board_contacts
                .iter()
                .map(|side| match side {
                    BoardBoundarySide::Left => "L",
                    BoardBoundarySide::Right => "R",
                    BoardBoundarySide::Top => "T",
                    BoardBoundarySide::Bottom => "B",
                })
                .collect::<Vec<_>>()
                .join("");
            format!(
                "C{}{}{}{}",
                fingerprint_field(&component.id),
                if component.board_attached { "R" } else { "H" },
                fingerprint_field(&contacts),
                fingerprint_field(component.generator.as_deref().unwrap_or(""))
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let parent_records = cuts
        .iter()
        .map(|cut| {
            let target = match &cut.target {
                TopologyCutTarget::BoardBottom => "B".to_owned(),
                TopologyCutTarget::Component { component } => {
                    format!("C{}", fingerprint_field(component))
                }
            };
            format!("P{}>{target}", fingerprint_field(&cut.source_component))
        })
        .collect::<Vec<_>>()
        .join("");
    // Coordinates are deliberately absent, but left-to-right beam order is a
    // discrete arrangement relation.  If two anchors exchange order, the old
    // word basis cannot silently survive even when their abstract parents do.
    let mut anchor_order = cuts.iter().collect::<Vec<_>>();
    anchor_order.sort_by(|first, second| {
        first.segment[0]
            .x
            .total_cmp(&second.segment[0].x)
            .then_with(|| first.generator.cmp(&second.generator))
    });
    let anchor_records = anchor_order
        .iter()
        .map(|cut| fingerprint_field(&cut.generator))
        .collect::<Vec<_>>()
        .join("");
    format!(
        "vertical-cut-forest/v2|components={component_records}|parents={parent_records}|anchor-order={anchor_records}"
    )
}

fn cut_forest_exact_fingerprint(structural_fingerprint: &str, cuts: &[TopologyCut]) -> String {
    let beam_records = cuts
        .iter()
        .map(|cut| {
            format!(
                "E{}:{:016x}:{:016x}:{:016x}:{:016x}",
                fingerprint_field(&cut.generator),
                cut.segment[0].x.to_bits(),
                cut.segment[0].y.to_bits(),
                cut.segment[1].x.to_bits(),
                cut.segment[1].y.to_bits(),
            )
        })
        .collect::<Vec<_>>()
        .join("");
    format!(
        "vertical-cut-forest-exact/v2|structural={}|beams={beam_records}",
        fingerprint_field(structural_fingerprint)
    )
}

fn fingerprint_field(value: &str) -> String {
    format!("{}:{value}", value.len())
}

fn point_on_component_boundary(
    point: Vec2,
    obstacles: &[CorridorObstacle],
    component: &WorkingTopologyComponent,
) -> bool {
    component.obstacle_indices.iter().any(|&obstacle_index| {
        let polygon = &obstacles[obstacle_index].polygon;
        (0..polygon.len()).any(|index| {
            segment_distance(
                point,
                point,
                polygon[index],
                polygon[(index + 1) % polygon.len()],
            ) <= EPSILON * 8.0
        })
    })
}

fn certify_vertical_cut_forest(
    board: Rect,
    obstacles: &[CorridorObstacle],
    components: &[WorkingTopologyComponent],
    cuts: &[TopologyCut],
) -> Vec<String> {
    let mut failures = Vec::new();
    let expected_members = obstacles
        .iter()
        .map(|obstacle| obstacle.id.clone())
        .collect::<BTreeSet<_>>();
    let actual_members = components
        .iter()
        .flat_map(|component| component.record.members.iter().cloned())
        .collect::<Vec<_>>();
    if actual_members.iter().cloned().collect::<BTreeSet<_>>() != expected_members
        || actual_members.len() != expected_members.len()
    {
        failures.push("cut forest components do not cover each obstacle exactly once".to_owned());
    }
    let component_by_id = components
        .iter()
        .map(|component| (component.record.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    if component_by_id.len() != components.len() {
        failures.push("cut forest component identities are not unique".to_owned());
    }
    let generators = components
        .iter()
        .filter_map(|component| component.record.generator.as_deref())
        .collect::<BTreeSet<_>>();
    let interior_count = components
        .iter()
        .filter(|component| !component.record.board_attached)
        .count();
    if generators.len() != interior_count {
        failures.push("cut forest generator identities are not unique".to_owned());
    }
    let mut cut_by_source = BTreeMap::<&str, &TopologyCut>::new();
    for cut in cuts {
        if cut_by_source
            .insert(cut.source_component.as_str(), cut)
            .is_some()
        {
            failures.push(format!(
                "forbidden component {} has more than one cut",
                cut.source_component
            ));
        }
    }
    for component in components {
        if !component
            .record
            .members
            .windows(2)
            .all(|members| members[0].as_str() < members[1].as_str())
        {
            failures.push(format!(
                "component {} member provenance is not strictly sorted",
                component.record.id
            ));
        }
        let certified_contacts = component
            .obstacle_indices
            .iter()
            .flat_map(|&index| polygon_board_contacts(board, &obstacles[index].polygon))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if component.record.board_contacts != certified_contacts
            || component.record.board_attached != !certified_contacts.is_empty()
        {
            failures.push(format!(
                "component {} has an invalid board-contact set",
                component.record.id
            ));
        }
        if component.record.board_attached == component.record.generator.is_some() {
            failures.push(format!(
                "component {} has an invalid exterior/generator role",
                component.record.id
            ));
        }
        match (
            component.record.board_attached,
            cut_by_source.get(component.record.id.as_str()),
        ) {
            (true, Some(_)) => failures.push(format!(
                "board-attached component {} must not emit a cut",
                component.record.id
            )),
            (false, None) => failures.push(format!(
                "interior component {} must emit exactly one cut",
                component.record.id
            )),
            _ => {}
        }
    }
    for (index, cut) in cuts.iter().enumerate() {
        let Some(source) = component_by_id.get(cut.source_component.as_str()) else {
            failures.push(format!(
                "cut generator {} names an unknown source component",
                cut.generator
            ));
            continue;
        };
        if source.record.generator.as_deref() != Some(cut.generator.as_str()) {
            failures.push(format!(
                "cut {} does not match its component generator",
                cut.generator
            ));
        }
        if (cut.segment[0].x - cut.segment[1].x).abs() > EPSILON
            || cut.segment[1].y <= cut.segment[0].y + EPSILON
        {
            failures.push(format!(
                "cut {} is not a positive vertical beam",
                cut.generator
            ));
        }
        if !nondegenerate_vertical_anchor(obstacles, cut.segment[0].x) {
            failures.push(format!("cut {} has a degenerate anchor", cut.generator));
        }
        if !point_on_component_boundary(cut.segment[0], obstacles, source) {
            failures.push(format!(
                "cut {} source anchor is not on its component boundary",
                cut.generator
            ));
        }
        match &cut.target {
            TopologyCutTarget::BoardBottom => {
                if (cut.segment[1].y - board.max.y).abs() > EPSILON * 8.0 {
                    failures.push(format!(
                        "cut {} claims the board but does not end on its bottom boundary",
                        cut.generator
                    ));
                }
            }
            TopologyCutTarget::Component { component } => {
                let Some(target) = component_by_id.get(component.as_str()) else {
                    failures.push(format!(
                        "cut {} targets unknown component {component}",
                        cut.generator
                    ));
                    continue;
                };
                if component == &cut.source_component {
                    failures.push(format!("cut {} is self-parented", cut.generator));
                }
                if !point_on_component_boundary(cut.segment[1], obstacles, target) {
                    failures.push(format!(
                        "cut {} target is not on component {component}",
                        cut.generator
                    ));
                }
            }
        }
        if cuts[..index]
            .iter()
            .any(|other| (other.segment[0].x - cut.segment[0].x).abs() <= EPSILON * 16.0)
        {
            failures.push(format!("cut {} reuses a vertical anchor", cut.generator));
        }
        let source_index = components
            .iter()
            .position(|component| component.record.id == cut.source_component)
            .unwrap();
        match vertical_cut_at_anchor(board, obstacles, components, source_index, cut.segment[0].x) {
            Some((expected_segment, expected_target))
                if expected_target == cut.target
                    && (expected_segment[0].y - cut.segment[0].y).abs() <= EPSILON * 8.0
                    && (expected_segment[1].y - cut.segment[1].y).abs() <= EPSILON * 8.0 => {}
            _ => failures.push(format!(
                "cut {} is not the obstacle-free beam to the first lower target",
                cut.generator
            )),
        }
    }

    for source in components
        .iter()
        .filter(|component| !component.record.board_attached)
    {
        let mut seen = BTreeSet::new();
        let mut current = source.record.id.as_str();
        loop {
            if !seen.insert(current) {
                failures.push(format!(
                    "cut forest cycle is reachable from {}",
                    source.record.id
                ));
                break;
            }
            let Some(cut) = cut_by_source.get(current) else {
                failures.push(format!(
                    "cut parent chain from {} does not reach the exterior",
                    source.record.id
                ));
                break;
            };
            match &cut.target {
                TopologyCutTarget::BoardBottom => break,
                TopologyCutTarget::Component { component } => {
                    let Some(parent) = component_by_id.get(component.as_str()) else {
                        break;
                    };
                    if parent.record.board_attached {
                        break;
                    }
                    current = parent.record.id.as_str();
                }
            }
        }
    }
    failures
}

pub fn embed_route(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
) -> RouteCorridorEmbedding {
    embed_route_internal(graph, request, false)
}

fn embed_route_internal(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
    allow_any_homotopy: bool,
) -> RouteCorridorEmbedding {
    let required_width = request.physical_width + 2.0 * request.clearance;
    let required_radius = request.physical_width * 0.5 + request.clearance;
    let excluded = route_endpoint_exemptions(request.from_component, request.to_component);
    let from_exemptions = endpoint_obstacle_exemptions(request.from_component);
    let to_exemptions = endpoint_obstacle_exemptions(request.to_component);
    let reference_signature = polyline_signature(request.reference, &graph.cut_basis, &excluded);
    let target_homotopy = if request.persistent_basis_matches {
        request
            .target_homotopy
            .cloned()
            .unwrap_or_else(|| reference_signature.word.clone())
    } else {
        reference_signature.word.clone()
    };
    let base = || RouteCorridorEmbedding {
        net: request.net.into(),
        route_run: request.net.into(),
        run_index: 0,
        start_point: 0,
        end_point: request.reference.len().saturating_sub(1),
        complete_route: true,
        layer: graph.layer.clone(),
        corridor_revision: graph.revision,
        placement_revision: graph.placement_revision,
        required_width,
        purpose: EmbeddingPurpose::FullWidthCertificate,
        cells: Vec::new(),
        gates: Vec::new(),
        semantic_regions: Vec::new(),
        semantic_passages: Vec::new(),
        polyline: Vec::new(),
        method: EmbeddingMethod::ReferenceBiasedShortestPath,
        geometry_method: GeometryMethod::CellCentersFallback,
        route_class_verified: false,
        run_class_verified: false,
        basis_mapping_required: false,
        reference_homotopy: reference_signature.word.clone(),
        embedded_homotopy: HomotopyWord::empty(),
        epoch_homotopy_verified: false,
        clearance: ClearanceCertificate {
            model: ClearanceModel::ExactPolylineAgainstPolygonalObstacles,
            required_radius,
            certified: false,
            minimum_clearance: 0.0,
            failures: vec!["no embedding polyline was produced".into()],
        },
        repair: None,
        projection: None,
        failure: None,
    };

    let source = terminal_cell(graph, request.from, request.from_toward);
    if source.is_none() {
        let source_projection = terminal_projection(graph, request.from, request.from_toward);
        let target_projection = terminal_projection(graph, request.to, request.to_toward);
        let mut failure_reasons = vec![RouteProjectionFailureReason::SourceDirectCellMissing];
        if target_projection.evidence.direct_selected_cell.is_none() {
            failure_reasons.push(RouteProjectionFailureReason::TargetDirectCellMissing);
        }
        return RouteCorridorEmbedding {
            failure: Some("no free corridor cell is adjacent to the source terminal".into()),
            projection: Some(RouteProjectionEvidence {
                source: source_projection.evidence,
                target: target_projection.evidence,
                ordinary_attempt: None,
                gap_retry_attempt: None,
                failure_reasons,
            }),
            ..base()
        };
    }
    let source = source.expect("source direct selection checked above");
    let target = terminal_cell(graph, request.to, request.to_toward);
    if target.is_none() {
        let source_projection = terminal_projection(graph, request.from, request.from_toward);
        let target_projection = terminal_projection(graph, request.to, request.to_toward);
        return RouteCorridorEmbedding {
            failure: Some("no free corridor cell is adjacent to the target terminal".into()),
            projection: Some(RouteProjectionEvidence {
                source: source_projection.evidence,
                target: target_projection.evidence,
                ordinary_attempt: None,
                gap_retry_attempt: None,
                failure_reasons: vec![RouteProjectionFailureReason::TargetDirectCellMissing],
            }),
            ..base()
        };
    }
    let target = target.expect("target direct selection checked above");

    // The continuous engine's current route is valuable geometric evidence.
    // If it is already legal in the requested class, do not let an incomplete
    // raw-mesh search replace it with a worse representative. Project the
    // witness onto the disposable graph instead; search is reserved for an
    // invalid witness.
    let reference_clearance = certify_polyline_clearance(
        graph,
        request.reference,
        required_radius,
        request.from_component,
        request.to_component,
    );
    let reference_matches_target = !reference_signature.ambiguous
        && (allow_any_homotopy || reference_signature.word == target_homotopy);
    if reference_clearance.certified && reference_matches_target {
        let epoch_homotopy_verified = graph.cut_basis.complete;
        let route_class_verified = graph.cut_basis.complete
            && request.persistent_basis_matches
            && request.target_homotopy.is_some();
        let projected = project_certified_reference_to_corridor(graph, &request, source, target);
        if let Some((cells, gates)) = projected.traversal {
            let (semantic_regions, semantic_passages) = semantic_projection(graph, &cells, &gates);
            return RouteCorridorEmbedding {
                cells,
                gates,
                semantic_regions,
                semantic_passages,
                polyline: request.reference.to_vec(),
                method: EmbeddingMethod::CertifiedReferenceContinuation,
                geometry_method: if projected.used_gap_anchor_retry {
                    GeometryMethod::CertifiedReferenceGapAnchorRetry
                } else {
                    GeometryMethod::CertifiedReference
                },
                route_class_verified,
                reference_homotopy: reference_signature.word.clone(),
                embedded_homotopy: reference_signature.word.clone(),
                epoch_homotopy_verified,
                clearance: reference_clearance,
                projection: projected.evidence,
                ..base()
            };
        }
        return RouteCorridorEmbedding {
            polyline: request.reference.to_vec(),
            method: EmbeddingMethod::CertifiedReferenceContinuation,
            geometry_method: GeometryMethod::CertifiedReference,
            route_class_verified,
            reference_homotopy: reference_signature.word.clone(),
            embedded_homotopy: reference_signature.word.clone(),
            epoch_homotopy_verified,
            clearance: reference_clearance,
            projection: projected.evidence,
            failure: Some(
                "the certified current route could not be projected onto the disposable corridor graph by either ordinary endpoint sectors or the fail-closed endpoint-gap retry; geometry was deliberately left unchanged"
                    .into(),
            ),
            ..base()
        };
    }

    // Centerline seeding deliberately uses the sparse corridor graph first.
    // Constructing the all-corners visibility graph is quadratic in footprint
    // vertices and defeats the point of a cheap, disposable initial witness.
    let repair_evidence = if required_radius <= EPSILON * 32.0 && required_width <= EPSILON * 64.0 {
        None
    } else {
        let mut repair_result =
            repair_route_visibility(graph, &request, required_radius, &target_homotopy);
        if let Some(selected) = repair_result.selected.take() {
            let embedded_signature =
                polyline_signature(&selected.polyline, &graph.cut_basis, &excluded);
            let epoch_homotopy_verified = graph.cut_basis.complete
                && !reference_signature.ambiguous
                && !embedded_signature.ambiguous
                && reference_signature.word == embedded_signature.word;
            let route_class_verified = graph.cut_basis.complete
                && request.persistent_basis_matches
                && request.target_homotopy.is_some()
                && !embedded_signature.ambiguous
                && embedded_signature.word == target_homotopy;
            let repaired_source = terminal_cell(graph, selected.polyline[0], selected.polyline[1]);
            let repaired_target = terminal_cell(
                graph,
                *selected.polyline.last().unwrap(),
                selected.polyline[selected.polyline.len() - 2],
            );
            if let (Some(repaired_source), Some(repaired_target)) =
                (repaired_source, repaired_target)
                && let Some((cells, gates)) = project_polyline_to_corridor(
                    graph,
                    &selected.polyline,
                    repaired_source,
                    repaired_target,
                )
            {
                let (semantic_regions, semantic_passages) =
                    semantic_projection(graph, &cells, &gates);
                return RouteCorridorEmbedding {
                    cells,
                    gates,
                    semantic_regions,
                    semantic_passages,
                    polyline: selected.polyline,
                    method: EmbeddingMethod::ClearanceVisibilityRepair,
                    geometry_method: GeometryMethod::ClearanceVisibilityGraph,
                    route_class_verified,
                    reference_homotopy: reference_signature.word.clone(),
                    embedded_homotopy: embedded_signature.word,
                    epoch_homotopy_verified,
                    clearance: selected.clearance,
                    repair: Some(repair_result.evidence),
                    ..base()
                };
            }
            repair_result.evidence.outcome = RepairOutcome::ProjectionFailed;
            repair_result.evidence.selected_alternative = None;
        }
        Some(repair_result.evidence)
    };

    let source_segment = [request.from, graph.cells[source].center];
    if !segment_has_clearance(graph, source_segment, required_radius, &from_exemptions) {
        return RouteCorridorEmbedding {
            failure: Some("the source terminal cannot reach its selected corridor cell at the required clearance".into()),
            repair: repair_evidence,
            ..base()
        };
    }

    let mut adjacency = vec![Vec::<(usize, usize)>::new(); graph.cells.len()];
    for gate in &graph.gates {
        if gate.nominal_width + EPSILON < required_width {
            continue;
        }
        adjacency[gate.cells[0]].push((gate.cells[1], gate.id));
        adjacency[gate.cells[1]].push((gate.cells[0], gate.id));
    }

    let source_signature = polyline_signature(&source_segment, &graph.cut_basis, &excluded);
    if source_signature.ambiguous && !allow_any_homotopy {
        return RouteCorridorEmbedding {
            failure: Some("the source-to-cell segment touches a topology cut ambiguously".into()),
            repair: repair_evidence,
            ..base()
        };
    }
    let source_key = SearchKey {
        cell: source,
        word: if allow_any_homotopy {
            HomotopyWord::empty()
        } else {
            source_signature.word
        },
    };
    let maximum_word_length = target_homotopy.crossings().len() + 4;
    let mut distances = HashMap::<SearchKey, f64>::new();
    let mut previous = HashMap::<SearchKey, (SearchKey, usize)>::new();
    let mut heap = BinaryHeap::new();
    distances.insert(source_key.clone(), 0.0);
    heap.push(QueueState {
        cost: 0.0,
        key: source_key.clone(),
    });
    let mut accepted = None;
    let mut visited_states = 0usize;

    while let Some(QueueState { cost, key }) = heap.pop() {
        visited_states += 1;
        if visited_states > 20_000 {
            break;
        }
        if cost > distances.get(&key).copied().unwrap_or(f64::INFINITY) {
            continue;
        }
        if key.cell == target {
            let final_segment = [graph.cells[target].center, request.to];
            if segment_has_clearance(graph, final_segment, required_radius, &to_exemptions) {
                let final_signature =
                    polyline_signature(&final_segment, &graph.cut_basis, &excluded);
                if !final_signature.ambiguous
                    && (allow_any_homotopy
                        || key
                            .word
                            .extended(final_signature.word.crossings().iter().cloned())
                            == target_homotopy)
                {
                    accepted = Some(key);
                    break;
                }
            }
        }
        for &(next, gate_id) in &adjacency[key.cell] {
            let gate = &graph.gates[gate_id];
            let midpoint = scale(add(gate.segment[0], gate.segment[1]), 0.5);
            let transition = [graph.cells[key.cell].center, graph.cells[next].center];
            if !segment_has_clearance(graph, transition, required_radius, &[]) {
                continue;
            }
            let next_word = if allow_any_homotopy {
                HomotopyWord::empty()
            } else {
                let transition_signature =
                    polyline_signature(&transition, &graph.cut_basis, &excluded);
                if transition_signature.ambiguous {
                    continue;
                }
                key.word
                    .extended(transition_signature.word.crossings().iter().cloned())
            };
            if next_word.crossings().len() > maximum_word_length {
                continue;
            }
            let next_key = SearchKey {
                cell: next,
                word: next_word,
            };
            let step = length(sub(graph.cells[next].center, graph.cells[key.cell].center));
            let reference_penalty = point_polyline_distance(midpoint, request.reference) * 0.2;
            let next_cost = cost + step + reference_penalty;
            if next_cost < distances.get(&next_key).copied().unwrap_or(f64::INFINITY) {
                distances.insert(next_key.clone(), next_cost);
                previous.insert(next_key.clone(), (key.clone(), gate_id));
                heap.push(QueueState {
                    cost: next_cost,
                    key: next_key,
                });
            }
        }
    }

    let Some(mut cursor) = accepted else {
        return RouteCorridorEmbedding {
            failure: Some(format!(
                "no clearance-certified corridor path with nominal capacity {:.4} matched epoch homotopy {:?}",
                required_width,
                target_homotopy.crossings()
            )),
            repair: repair_evidence,
            ..base()
        };
    };

    let mut cells = vec![cursor.cell];
    let mut gates = Vec::new();
    while cursor != source_key {
        let (parent, gate) = previous
            .get(&cursor)
            .cloned()
            .expect("reachable state must have a predecessor");
        gates.push(gate);
        cells.push(parent.cell);
        cursor = parent;
    }
    cells.reverse();
    gates.reverse();
    let mut center_polyline = Vec::with_capacity(cells.len() + 2);
    center_polyline.push(request.from);
    center_polyline.extend(cells.iter().map(|&cell| graph.cells[cell].center));
    center_polyline.push(request.to);
    let funnel_polyline = string_pull_portals(
        graph,
        &cells,
        &gates,
        request.from,
        request.to,
        required_radius,
    );
    let funnel_signature = polyline_signature(&funnel_polyline, &graph.cut_basis, &excluded);
    let funnel_clearance = certify_polyline_clearance(
        graph,
        &funnel_polyline,
        required_radius,
        request.from_component,
        request.to_component,
    );
    let use_funnel = !funnel_signature.ambiguous
        && (allow_any_homotopy || funnel_signature.word == target_homotopy)
        && funnel_clearance.certified;
    let shortcut_polyline = shortcut_within_corridor(
        graph,
        &center_polyline,
        &cells,
        &graph.cut_basis,
        &excluded,
        required_radius,
        request.from_component,
        request.to_component,
    );
    let shortcut_signature = polyline_signature(&shortcut_polyline, &graph.cut_basis, &excluded);
    let shortcut_clearance = certify_polyline_clearance(
        graph,
        &shortcut_polyline,
        required_radius,
        request.from_component,
        request.to_component,
    );
    let use_shortcut = !shortcut_signature.ambiguous
        && (allow_any_homotopy || shortcut_signature.word == target_homotopy)
        && shortcut_clearance.certified;
    let (polyline, embedded_signature, clearance, geometry_method) = if use_funnel {
        (
            funnel_polyline,
            funnel_signature,
            funnel_clearance,
            GeometryMethod::ClearanceInsetFunnel,
        )
    } else if use_shortcut {
        (
            shortcut_polyline,
            shortcut_signature,
            shortcut_clearance,
            GeometryMethod::CorridorUnionShortcut,
        )
    } else {
        let signature = polyline_signature(&center_polyline, &graph.cut_basis, &excluded);
        let clearance = certify_polyline_clearance(
            graph,
            &center_polyline,
            required_radius,
            request.from_component,
            request.to_component,
        );
        (
            center_polyline,
            signature,
            clearance,
            GeometryMethod::CellCentersFallback,
        )
    };
    let epoch_homotopy_verified = graph.cut_basis.complete
        && !embedded_signature.ambiguous
        && (allow_any_homotopy
            || (!reference_signature.ambiguous
                && reference_signature.word == embedded_signature.word));
    let route_class_verified = graph.cut_basis.complete
        && request.persistent_basis_matches
        && request.target_homotopy.is_some()
        && !embedded_signature.ambiguous
        && embedded_signature.word == target_homotopy;
    let failure = (!clearance.certified)
        .then(|| "the reconstructed corridor polyline failed exact clearance certification".into());
    let (semantic_regions, semantic_passages) = semantic_projection(graph, &cells, &gates);

    RouteCorridorEmbedding {
        cells,
        gates,
        semantic_regions,
        semantic_passages,
        polyline,
        geometry_method,
        route_class_verified,
        reference_homotopy: reference_signature.word.clone(),
        embedded_homotopy: embedded_signature.word,
        epoch_homotopy_verified,
        clearance,
        repair: repair_evidence,
        failure,
        ..base()
    }
}

/// Produce a zero-clearance centerline witness while retaining the route's
/// full declared demand for later corridor allocation. This is initialization
/// evidence, never a full-width correctness certificate.
pub fn embed_route_centerline_seed(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
) -> RouteCorridorEmbedding {
    let allocation_width = request.physical_width + 2.0 * request.clearance;
    let seed_request = RouteEmbeddingRequest {
        physical_width: 0.0,
        // A mathematical zero-width line must still not pass through an
        // obstacle. Use a numerical topology epsilon so boundary contact is
        // rejected without introducing meaningful copper width.
        clearance: EPSILON * 16.0,
        target_homotopy: None,
        persistent_basis_matches: false,
        ..request
    };
    // Product-graph routing preserves the reference homotopy on small corridor
    // alphabets. On footprint-scale graphs that state space can grow
    // exponentially, so the seed is allowed to choose a provisional class;
    // the coordinator still owns the later certified discrete decision.
    const MAX_SEED_TOPOLOGY_CUTS: usize = 32;
    let allow_any_homotopy = graph.cut_basis.cuts.len() > MAX_SEED_TOPOLOGY_CUTS;
    let mut embedding = embed_route_internal(graph, seed_request, allow_any_homotopy);
    if embedding.clearance.certified
        && embedding.method == EmbeddingMethod::CertifiedReferenceContinuation
        && embedding.cells.is_empty()
        && !embedding.polyline.is_empty()
    {
        // A safe centerline is useful engine initialization even when the
        // deliberately degraded mesh cannot attribute it to corridor cells.
        // It contributes no passage allocation until a later rebuild can
        // project it, and remains visibly tagged as seed-only evidence.
        embedding.failure = None;
    }
    embedding.required_width = allocation_width;
    embedding.purpose = EmbeddingPurpose::CenterlineSeed;
    embedding.route_class_verified = false;
    embedding.run_class_verified = false;
    embedding
}

/// Enumerate a small set of exact-clearance route-class witnesses without
/// constraining the search to the currently persisted homotopy word.
///
/// This is planner evidence, not an embedding decision. The caller must choose
/// a class explicitly and run the ordinary embedding/validation pipeline on
/// the resulting candidate.
pub fn enumerate_route_class_alternatives(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
) -> RouteRepairEvidence {
    first_k_clearance_offset_visibility_dijkstra(graph, request)
}

/// Enumerate a caller-selected bounded first-K frontier of exact-clearance
/// route classes.
///
/// This exploratory API remains deliberately small and fail-closed. It is
/// intended for producers which may need to discard otherwise valid geometry
/// that cannot be mapped into their richer discrete representation. It never
/// changes persistent exact-word embedding, whose effective goal limit is one.
pub fn enumerate_route_class_alternatives_with_limit(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
    goal_limit: usize,
) -> Result<RouteRepairEvidence, RouteClassFamilyLimitError> {
    enumerate_route_class_alternatives_with_limit_and_budget(
        graph,
        request,
        goal_limit,
        DEFAULT_ROUTE_REPAIR_SETTLED_STATE_BUDGET,
    )
}

/// Enumerate a bounded first-K frontier with a caller-selected settled-state
/// budget for the final canonical search attempt.
///
/// This is not a whole-call work or wall-clock limit. Lazy visibility keeps
/// its independent pair/geometry ceilings, and a heuristic-inconsistency
/// fallback may discard one attempt and restart with the same budget. The
/// returned [`RouteRepairSearchWork`] describes only the final attempt.
/// A zero budget is valid and deterministically returns incomplete evidence
/// without settling a product-graph state.
pub fn enumerate_route_class_alternatives_with_limit_and_budget(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
    goal_limit: usize,
    settled_state_budget: usize,
) -> Result<RouteRepairEvidence, RouteClassFamilyLimitError> {
    if !(1..=MAX_EXPLORATORY_ROUTE_CLASS_FAMILY_LIMIT).contains(&goal_limit) {
        return Err(RouteClassFamilyLimitError {
            requested: goal_limit,
            minimum: 1,
            maximum: MAX_EXPLORATORY_ROUTE_CLASS_FAMILY_LIMIT,
        });
    }
    let required_radius = request.physical_width * 0.5 + request.clearance;
    let exploratory = RouteEmbeddingRequest {
        target_homotopy: None,
        persistent_basis_matches: false,
        ..request
    };
    Ok(repair_route_visibility_with_goal_limit(
        graph,
        &exploratory,
        required_radius,
        &HomotopyWord::empty(),
        settled_state_budget,
        goal_limit,
    )
    .evidence)
}

/// Enumerate a bounded first-K frontier with cumulative visibility-work
/// ceilings in addition to the final-search settled-state ceiling.
///
/// Unlike [`enumerate_route_class_alternatives_with_limit_and_budget`], the
/// visibility limits cover both an initial Euclidean-heuristic attempt and a
/// zero-heuristic retry if numerical inconsistency forces one. Node creation,
/// endpoint preparation, candidate materialization, and elapsed time are not
/// covered by these counters. A zero visibility limit is valid and returns
/// typed incomplete evidence before evaluating a visibility pair.
pub fn enumerate_route_class_alternatives_with_limit_and_work_budget(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
    goal_limit: usize,
    work_budget: RouteRepairWorkBudget,
) -> Result<BoundedRouteRepairEvidence, RouteClassFamilyLimitError> {
    if !(1..=MAX_EXPLORATORY_ROUTE_CLASS_FAMILY_LIMIT).contains(&goal_limit) {
        return Err(RouteClassFamilyLimitError {
            requested: goal_limit,
            minimum: 1,
            maximum: MAX_EXPLORATORY_ROUTE_CLASS_FAMILY_LIMIT,
        });
    }
    let required_radius = request.physical_width * 0.5 + request.clearance;
    let exploratory = RouteEmbeddingRequest {
        target_homotopy: None,
        persistent_basis_matches: false,
        ..request
    };
    let result = repair_route_visibility_with_work_budget(
        graph,
        &exploratory,
        required_radius,
        &HomotopyWord::empty(),
        work_budget,
        goal_limit,
    );
    Ok(BoundedRouteRepairEvidence {
        evidence: result.evidence,
        visibility_work: result.visibility_work,
    })
}

/// Enumerate the deterministic first three distinct route classes popped by
/// clearance-offset visibility Dijkstra.
///
/// Completeness is deliberately relative to this named bounded generator, not
/// to every geometrically realizable route class. `FamilyLimitReached` means
/// the exact K-cost threshold was completed; `SearchExhausted` means the whole
/// bounded product graph was exhausted. Either hard complexity limit is
/// incomplete evidence.
pub fn first_k_clearance_offset_visibility_dijkstra(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
) -> RouteRepairEvidence {
    let required_radius = request.physical_width * 0.5 + request.clearance;
    let exploratory = RouteEmbeddingRequest {
        target_homotopy: None,
        persistent_basis_matches: false,
        ..request
    };
    repair_route_visibility_with_goal_limit(
        graph,
        &exploratory,
        required_radius,
        &HomotopyWord::empty(),
        MAX_REPAIR_SETTLED_STATES,
        FIRST_K_ROUTE_CLASS_FAMILY_LIMIT,
    )
    .evidence
}

/// Find one shortest geometric visibility witness without multiplying nodes
/// by homotopy words.
///
/// This is the default bounded operation for an unconstrained reroute. Every
/// admitted edge still receives its exact cut signature, and the settled
/// target polyline is exact-clearance certified before its homotopy word is
/// computed and published. Call the explicit route-class enumeration APIs
/// when more than one topological family is actually required.
pub fn first_clearance_offset_visibility_path_with_budget(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
    settled_state_budget: usize,
) -> RouteRepairEvidence {
    let required_radius = request.physical_width * 0.5 + request.clearance;
    let unconstrained = RouteEmbeddingRequest {
        target_homotopy: None,
        persistent_basis_matches: false,
        ..request
    };
    repair_route_visibility_with_search_goal_limit::<true>(
        graph,
        &unconstrained,
        required_radius,
        &HomotopyWord::empty(),
        RepairHeuristic::Euclidean,
        RouteRepairWorkBudget {
            settled_states: settled_state_budget,
            ..RouteRepairWorkBudget::default()
        },
        1,
        false,
        RepairVisibilitySearchStrategy::FirstGeometricWitness,
    )
    .evidence
}

/// One-witness geometric search with cumulative visibility-work ceilings.
pub fn first_clearance_offset_visibility_path_with_work_budget(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
    work_budget: RouteRepairWorkBudget,
) -> BoundedRouteRepairEvidence {
    let required_radius = request.physical_width * 0.5 + request.clearance;
    let unconstrained = RouteEmbeddingRequest {
        target_homotopy: None,
        persistent_basis_matches: false,
        ..request
    };
    let result = repair_route_visibility_with_search_goal_limit::<true>(
        graph,
        &unconstrained,
        required_radius,
        &HomotopyWord::empty(),
        RepairHeuristic::Euclidean,
        work_budget,
        1,
        true,
        RepairVisibilitySearchStrategy::FirstGeometricWitness,
    );
    BoundedRouteRepairEvidence {
        evidence: result.evidence,
        visibility_work: result.visibility_work,
    }
}

/// Find a shortest exact-clearance witness while treating provisional copper
/// reservations as obstacles. The disposable obstacles affect geometry only:
/// they do not enter the persistent topology alphabet or corridor graph.
pub fn enumerate_dynamic_obstacle_avoidance(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
    removed_obstacle_ids: &[String],
    temporary_obstacles: &[CorridorObstacle],
) -> RouteRepairEvidence {
    enumerate_dynamic_obstacle_avoidance_with_limit_and_budget(
        graph,
        request,
        removed_obstacle_ids,
        temporary_obstacles,
        FIRST_K_ROUTE_CLASS_FAMILY_LIMIT,
        DEFAULT_ROUTE_REPAIR_SETTLED_STATE_BUDGET,
    )
    .expect("the built-in dynamic-obstacle family limit is valid")
}

/// Find one shortest geometric witness around disposable obstacles without a
/// homotopy-product state space.
pub fn first_dynamic_obstacle_avoidance_with_budget(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
    removed_obstacle_ids: &[String],
    temporary_obstacles: &[CorridorObstacle],
    settled_state_budget: usize,
) -> RouteRepairEvidence {
    let planning_graph =
        dynamic_obstacle_planning_graph(graph, removed_obstacle_ids, temporary_obstacles);
    first_clearance_offset_visibility_path_with_budget(
        &planning_graph,
        request,
        settled_state_budget,
    )
}

/// Dynamic one-witness search with cumulative visibility-work ceilings.
pub fn first_dynamic_obstacle_avoidance_with_work_budget(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
    removed_obstacle_ids: &[String],
    temporary_obstacles: &[CorridorObstacle],
    work_budget: RouteRepairWorkBudget,
) -> BoundedRouteRepairEvidence {
    let planning_graph =
        dynamic_obstacle_planning_graph(graph, removed_obstacle_ids, temporary_obstacles);
    first_clearance_offset_visibility_path_with_work_budget(&planning_graph, request, work_budget)
}

/// Bounded dynamic-obstacle search with the same first-K and settled-state
/// contract as ordinary route-class enumeration. The detached planning graph
/// receives an ephemeral cut basis so alternatives around temporary obstacles
/// remain distinct without publishing those cuts as persistent topology.
pub fn enumerate_dynamic_obstacle_avoidance_with_limit_and_budget(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
    removed_obstacle_ids: &[String],
    temporary_obstacles: &[CorridorObstacle],
    goal_limit: usize,
    settled_state_budget: usize,
) -> Result<RouteRepairEvidence, RouteClassFamilyLimitError> {
    let planning_graph =
        dynamic_obstacle_planning_graph(graph, removed_obstacle_ids, temporary_obstacles);
    enumerate_route_class_alternatives_with_limit_and_budget(
        &planning_graph,
        request,
        goal_limit,
        settled_state_budget,
    )
}

/// Dynamic-obstacle search with cumulative visibility and settled-state
/// limits. Disposable obstacles enter only the detached ephemeral alphabet,
/// never the persistent corridor graph's alphabet.
pub fn enumerate_dynamic_obstacle_avoidance_with_limit_and_work_budget(
    graph: &CorridorGraph,
    request: RouteEmbeddingRequest<'_>,
    removed_obstacle_ids: &[String],
    temporary_obstacles: &[CorridorObstacle],
    goal_limit: usize,
    work_budget: RouteRepairWorkBudget,
) -> Result<BoundedRouteRepairEvidence, RouteClassFamilyLimitError> {
    let planning_graph =
        dynamic_obstacle_planning_graph(graph, removed_obstacle_ids, temporary_obstacles);
    enumerate_route_class_alternatives_with_limit_and_work_budget(
        &planning_graph,
        request,
        goal_limit,
        work_budget,
    )
}

fn dynamic_obstacle_planning_graph(
    graph: &CorridorGraph,
    removed_obstacle_ids: &[String],
    temporary_obstacles: &[CorridorObstacle],
) -> CorridorGraph {
    let mut planning_graph = graph.clone();
    planning_graph
        .obstacles
        .retain(|obstacle| !removed_obstacle_ids.contains(&obstacle.id));
    planning_graph
        .obstacles
        .extend_from_slice(temporary_obstacles);
    planning_graph.cut_basis = build_cut_basis(
        planning_graph.board,
        &planning_graph.obstacles,
        planning_graph.placement_revision,
        planning_graph.revision,
    );
    planning_graph
}

struct VisibilityRepairResult {
    evidence: RouteRepairEvidence,
    selected: Option<SelectedRepair>,
    visibility_work: RouteRepairVisibilityWork,
    #[cfg(test)]
    ordered_node_paths: Vec<Vec<usize>>,
    #[cfg(test)]
    ordered_search_cost_bits: Vec<u64>,
}

struct SelectedRepair {
    polyline: Vec<Vec2>,
    clearance: ClearanceCertificate,
}

// Visibility is materialized on demand. Pair work covers only candidates that
// are actually requested, while geometry work charges every such pair the
// full obstacle-plus-cut feature count. Keeping that stable logical charge
// independent of the internal broad phase preserves bounded-search evidence.
pub const MAX_REPAIR_VISIBILITY_PAIR_EVALUATIONS: usize = 131_072;
pub const MAX_REPAIR_VISIBILITY_GEOMETRY_UNITS: usize = 25_000_000;
/// Settled-state budget used by all legacy/default route-repair APIs.
///
/// This limits the final product-graph search attempt, not lazy-visibility
/// construction, total retries, elapsed time, or the whole planner call.
pub const DEFAULT_ROUTE_REPAIR_SETTLED_STATE_BUDGET: usize = 50_000;
const MAX_REPAIR_SETTLED_STATES: usize = DEFAULT_ROUTE_REPAIR_SETTLED_STATE_BUDGET;

fn empty_route_repair_search_work(goal_limit: usize) -> RouteRepairSearchWork {
    RouteRepairSearchWork {
        contract: RepairSearchContract::CanonicalEuclideanAstarV1,
        budget_kind: RepairSearchBudgetKind::SettledStates,
        goal_limit,
        budget_limit: MAX_REPAIR_SETTLED_STATES,
        budget_consumed: 0,
        heap_pops: 0,
        stale_pops: 0,
        settled_states: 0,
        expanded_states: 0,
        relaxed_edges: 0,
        accepted_labels: 0,
        heuristic_evaluations: 0,
        heuristic_fallback_to_zero: false,
        goals_settled: 0,
        boundary_goals: 0,
        threshold_extra_pops: 0,
        threshold_complete: false,
        exact_cost_hop_path_comparisons: 0,
        lexicographic_improvements: 0,
        identical_path_ties: 0,
        materialized_path_elements: 0,
        output_materialized_path_elements: 0,
        maximum_path_depth: 1,
        path_arena_nodes: 0,
    }
}

#[derive(Clone)]
struct VisibilityEdge {
    to: usize,
    length: f64,
    word: HomotopyWord,
    reference_penalty: Option<f64>,
}

enum LazyVisibilityExpansionError {
    WorkLimit(String),
    HeuristicInconsistent,
}

// Repair visibility repeatedly asks the same immutable obstacle and cut set
// about different candidate segments. Keep a repair-local deterministic BVH
// rather than paying one full geometry scan for every lazy visibility pair.
// The exact polygon and cut predicates remain authoritative at the leaves.
const REPAIR_VISIBILITY_BVH_LEAF_SIZE: usize = 8;

#[derive(Clone, Copy)]
struct RepairSpatialAabb {
    min: Vec2,
    max: Vec2,
}

impl RepairSpatialAabb {
    fn from_points(points: &[Vec2]) -> Option<Self> {
        let first = *points.first()?;
        if !first.x.is_finite() || !first.y.is_finite() {
            return None;
        }
        let mut bounds = Self {
            min: first,
            max: first,
        };
        for &point in &points[1..] {
            if !point.x.is_finite() || !point.y.is_finite() {
                return None;
            }
            bounds.min.x = bounds.min.x.min(point.x);
            bounds.min.y = bounds.min.y.min(point.y);
            bounds.max.x = bounds.max.x.max(point.x);
            bounds.max.y = bounds.max.y.max(point.y);
        }
        Some(bounds)
    }

    fn union(self, other: Self) -> Self {
        Self {
            min: Vec2::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Vec2::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }

    fn expanded(self, amount: f64) -> Self {
        Self {
            min: Vec2::new(self.min.x - amount, self.min.y - amount),
            max: Vec2::new(self.max.x + amount, self.max.y + amount),
        }
    }

    fn intersects(self, other: Self) -> bool {
        self.min.x <= other.max.x
            && other.min.x <= self.max.x
            && self.min.y <= other.max.y
            && other.min.y <= self.max.y
    }

    fn centroid_axis(self, axis: usize) -> f64 {
        if axis == 0 {
            self.min.x + (self.max.x - self.min.x) * 0.5
        } else {
            self.min.y + (self.max.y - self.min.y) * 0.5
        }
    }

    fn segment_distance_lower_bound(self, segment: [Vec2; 2]) -> f64 {
        let inside = |point: Vec2| {
            point.x >= self.min.x
                && point.x <= self.max.x
                && point.y >= self.min.y
                && point.y <= self.max.y
        };
        if inside(segment[0]) || inside(segment[1]) {
            return 0.0;
        }
        let corners = [
            self.min,
            Vec2::new(self.max.x, self.min.y),
            self.max,
            Vec2::new(self.min.x, self.max.y),
        ];
        let distance = (0..4)
            .map(|edge| {
                segment_distance(
                    segment[0],
                    segment[1],
                    corners[edge],
                    corners[(edge + 1) % 4],
                )
            })
            .fold(f64::INFINITY, f64::min);
        // `segment_distance` is the exact narrow-phase primitive, but this
        // value is only a pruning lower bound. Bias it downward by a
        // coordinate-scaled guard so roundoff can retain extra candidates but
        // can never make the broad phase certify geometry on its own.
        let coordinate_scale = [
            segment[0].x,
            segment[0].y,
            segment[1].x,
            segment[1].y,
            self.min.x,
            self.min.y,
            self.max.x,
            self.max.y,
        ]
        .into_iter()
        .map(f64::abs)
        .fold(1.0, f64::max);
        (distance - EPSILON * 16.0 * coordinate_scale).max(0.0)
    }
}

#[derive(Clone, Copy)]
struct RepairBvhEntry {
    primitive: usize,
    bounds: RepairSpatialAabb,
}

#[derive(Clone, Copy)]
enum RepairBvhNodeKind {
    Leaf { start: usize, len: usize },
    Branch { left: usize, right: usize },
}

struct RepairBvhNode {
    bounds: RepairSpatialAabb,
    kind: RepairBvhNodeKind,
}

#[derive(Default)]
struct RepairBvh {
    nodes: Vec<RepairBvhNode>,
    primitives: Vec<usize>,
    root: Option<usize>,
}

impl RepairBvh {
    fn build(mut entries: Vec<RepairBvhEntry>) -> Self {
        if entries.is_empty() {
            return Self::default();
        }
        let mut result = Self::default();
        result.root = Some(Self::build_node(
            &mut entries,
            &mut result.nodes,
            &mut result.primitives,
        ));
        result
    }

    fn build_node(
        entries: &mut [RepairBvhEntry],
        nodes: &mut Vec<RepairBvhNode>,
        primitives: &mut Vec<usize>,
    ) -> usize {
        let bounds = entries[1..]
            .iter()
            .fold(entries[0].bounds, |bounds, entry| {
                bounds.union(entry.bounds)
            });
        if entries.len() <= REPAIR_VISIBILITY_BVH_LEAF_SIZE {
            entries.sort_by_key(|entry| entry.primitive);
            let start = primitives.len();
            primitives.extend(entries.iter().map(|entry| entry.primitive));
            let node = nodes.len();
            nodes.push(RepairBvhNode {
                bounds,
                kind: RepairBvhNodeKind::Leaf {
                    start,
                    len: entries.len(),
                },
            });
            return node;
        }

        let centroid_bounds = entries[1..].iter().fold(
            RepairSpatialAabb {
                min: Vec2::new(
                    entries[0].bounds.centroid_axis(0),
                    entries[0].bounds.centroid_axis(1),
                ),
                max: Vec2::new(
                    entries[0].bounds.centroid_axis(0),
                    entries[0].bounds.centroid_axis(1),
                ),
            },
            |mut centroid_bounds, entry| {
                let x = entry.bounds.centroid_axis(0);
                let y = entry.bounds.centroid_axis(1);
                centroid_bounds.min.x = centroid_bounds.min.x.min(x);
                centroid_bounds.min.y = centroid_bounds.min.y.min(y);
                centroid_bounds.max.x = centroid_bounds.max.x.max(x);
                centroid_bounds.max.y = centroid_bounds.max.y.max(y);
                centroid_bounds
            },
        );
        let x_extent = centroid_bounds.max.x - centroid_bounds.min.x;
        let y_extent = centroid_bounds.max.y - centroid_bounds.min.y;
        let axis = usize::from(y_extent > x_extent);
        entries.sort_by(|first, second| {
            first
                .bounds
                .centroid_axis(axis)
                .total_cmp(&second.bounds.centroid_axis(axis))
                .then_with(|| first.primitive.cmp(&second.primitive))
        });
        let middle = entries.len() / 2;
        let (left_entries, right_entries) = entries.split_at_mut(middle);
        let left = Self::build_node(left_entries, nodes, primitives);
        let right = Self::build_node(right_entries, nodes, primitives);
        let node = nodes.len();
        nodes.push(RepairBvhNode {
            bounds,
            kind: RepairBvhNodeKind::Branch { left, right },
        });
        node
    }
}

struct RepairVisibilitySpatialIndex {
    obstacles: RepairBvh,
    cuts: RepairBvh,
    obstacle_geometry_valid: bool,
    cut_geometry_valid: bool,
}

impl RepairVisibilitySpatialIndex {
    fn build(graph: &CorridorGraph) -> Self {
        let mut obstacle_geometry_valid = true;
        let obstacles = graph
            .obstacles
            .iter()
            .enumerate()
            .filter_map(|(primitive, obstacle)| {
                if obstacle.polygon.len() < 3 {
                    obstacle_geometry_valid = false;
                    return None;
                }
                let Some(bounds) = RepairSpatialAabb::from_points(&obstacle.polygon) else {
                    obstacle_geometry_valid = false;
                    return None;
                };
                Some(RepairBvhEntry { primitive, bounds })
            })
            .collect();
        let mut cut_geometry_valid = true;
        let cuts = graph
            .cut_basis
            .cuts
            .iter()
            .enumerate()
            .filter_map(|(primitive, cut)| {
                let Some(bounds) = RepairSpatialAabb::from_points(&cut.segment) else {
                    cut_geometry_valid = false;
                    return None;
                };
                let cut_length = length(sub(cut.segment[1], cut.segment[0]));
                if !cut_length.is_finite() || cut_length <= EPSILON {
                    cut_geometry_valid = false;
                    return None;
                }
                Some(RepairBvhEntry {
                    primitive,
                    bounds: bounds.expanded(EPSILON * cut_length + EPSILON),
                })
            })
            .collect();
        Self {
            obstacles: RepairBvh::build(obstacles),
            cuts: RepairBvh::build(cuts),
            obstacle_geometry_valid,
            cut_geometry_valid,
        }
    }
}

#[derive(Default)]
struct RepairSpatialQueryScratch {
    cut_candidates: Vec<usize>,
    #[cfg(test)]
    exact_cut_tests: usize,
}

struct LazyVisibilityGraph {
    adjacency: Vec<Vec<VisibilityEdge>>,
    expanded_nodes: Vec<bool>,
    evidence_edges: Vec<RepairEdge>,
    rejected_edges: usize,
    pair_evaluations: usize,
    geometry_units: usize,
    geometry_features: usize,
    pair_evaluation_limit: usize,
    geometry_unit_limit: usize,
    spatial_query_scratch: RepairSpatialQueryScratch,
}

impl LazyVisibilityGraph {
    fn new(
        node_count: usize,
        geometry_features: usize,
        pair_evaluation_limit: usize,
        geometry_unit_limit: usize,
    ) -> Self {
        Self {
            adjacency: vec![Vec::new(); node_count],
            expanded_nodes: vec![false; node_count],
            evidence_edges: Vec::new(),
            rejected_edges: 0,
            pair_evaluations: 0,
            geometry_units: 0,
            geometry_features,
            pair_evaluation_limit,
            geometry_unit_limit,
            spatial_query_scratch: RepairSpatialQueryScratch::default(),
        }
    }

    fn expand<const PRECOMPUTE: bool>(
        &mut self,
        node: usize,
        nodes: &[RepairNode],
        graph: &CorridorGraph,
        spatial_index: &RepairVisibilitySpatialIndex,
        request: &RouteEmbeddingRequest<'_>,
        required_radius: f64,
        excluded: &[&str],
        heuristic: &[f64],
    ) -> Result<(), LazyVisibilityExpansionError> {
        if self.expanded_nodes[node] {
            return Ok(());
        }
        for other in 0..nodes.len() {
            if other == node || self.expanded_nodes[other] {
                continue;
            }
            if self.pair_evaluations >= self.pair_evaluation_limit {
                return Err(LazyVisibilityExpansionError::WorkLimit(format!(
                    "lazy visibility reached the bounded work ceiling of {} evaluated corner pairs",
                    self.pair_evaluation_limit
                )));
            }
            if self.geometry_units.saturating_add(self.geometry_features) > self.geometry_unit_limit
            {
                return Err(LazyVisibilityExpansionError::WorkLimit(format!(
                    "lazy visibility reached the bounded geometry-work ceiling of {} pair-by-feature units after {} evaluated corner pairs",
                    self.geometry_unit_limit, self.pair_evaluations
                )));
            }
            self.pair_evaluations += 1;
            self.geometry_units = self.geometry_units.saturating_add(self.geometry_features);

            let mut exemptions = Vec::with_capacity(4);
            if node == 0 || other == 0 {
                exemptions.extend(endpoint_obstacle_exemptions(request.from_component));
            }
            if node == 1 || other == 1 {
                exemptions.extend(endpoint_obstacle_exemptions(request.to_component));
            }
            let segment = [nodes[node].position, nodes[other].position];
            let (minimum_clearance, _) =
                segment_clearance_indexed(graph, spatial_index, segment, &exemptions);
            if minimum_clearance + EPSILON < required_radius {
                self.rejected_edges += 1;
                continue;
            }
            let signature = polyline_signature_indexed(
                &segment,
                &graph.cut_basis,
                spatial_index,
                excluded,
                &mut self.spatial_query_scratch,
            );
            if signature.ambiguous {
                self.rejected_edges += 1;
                continue;
            }
            let edge_length = length(sub(segment[1], segment[0]));
            if heuristic[node].total_cmp(&(edge_length + heuristic[other])) == Ordering::Greater
                || heuristic[other].total_cmp(&(edge_length + heuristic[node])) == Ordering::Greater
            {
                return Err(LazyVisibilityExpansionError::HeuristicInconsistent);
            }
            let reference_penalty = PRECOMPUTE.then(|| {
                let midpoint = scale(add(segment[0], segment[1]), 0.5);
                point_polyline_distance(midpoint, request.reference) * 0.1
            });
            self.adjacency[node].push(VisibilityEdge {
                to: other,
                length: edge_length,
                word: signature.word.clone(),
                reference_penalty,
            });
            self.adjacency[other].push(VisibilityEdge {
                to: node,
                length: edge_length,
                word: signature.word.reversed(),
                reference_penalty,
            });
            self.evidence_edges.push(RepairEdge {
                from: node.min(other),
                to: node.max(other),
                minimum_clearance,
            });
        }
        self.expanded_nodes[node] = true;
        self.adjacency[node].sort_by_key(|edge| edge.to);
        Ok(())
    }
}

struct RepairCandidate {
    alternative: RepairAlternative,
    clearance: ClearanceCertificate,
    node_path: Vec<usize>,
    search_cost: f64,
    hops: usize,
}

#[derive(Clone, Copy)]
struct RepairPathNode {
    parent: Option<usize>,
    node: usize,
}

struct RepairPathArena {
    nodes: Vec<RepairPathNode>,
    candidate_scratch: Vec<usize>,
    incumbent_scratch: Vec<usize>,
}

impl RepairPathArena {
    fn new(source: usize) -> Self {
        Self {
            nodes: vec![RepairPathNode {
                parent: None,
                node: source,
            }],
            candidate_scratch: Vec::new(),
            incumbent_scratch: Vec::new(),
        }
    }

    fn accept(&mut self, parent: usize, node: usize) -> usize {
        let handle = self.nodes.len();
        self.nodes.push(RepairPathNode {
            parent: Some(parent),
            node,
        });
        handle
    }

    fn fill_path(nodes: &[RepairPathNode], mut handle: usize, output: &mut Vec<usize>) {
        output.clear();
        loop {
            let entry = nodes[handle];
            output.push(entry.node);
            let Some(parent) = entry.parent else {
                break;
            };
            handle = parent;
        }
        output.reverse();
    }

    fn materialize(&self, handle: usize) -> Vec<usize> {
        let mut result = Vec::new();
        Self::fill_path(&self.nodes, handle, &mut result);
        result
    }

    fn compare_candidate(
        &mut self,
        parent: usize,
        node: usize,
        incumbent: usize,
        work: &mut RouteRepairSearchWork,
    ) -> Ordering {
        Self::fill_path(&self.nodes, parent, &mut self.candidate_scratch);
        self.candidate_scratch.push(node);
        Self::fill_path(&self.nodes, incumbent, &mut self.incumbent_scratch);
        work.materialized_path_elements +=
            self.candidate_scratch.len() + self.incumbent_scratch.len();
        self.candidate_scratch.cmp(&self.incumbent_scratch)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RepairSearchKey {
    node: usize,
    word: HomotopyWord,
}

#[derive(Clone, Copy)]
struct RepairSearchLabel {
    cost: f64,
    hops: usize,
    path: usize,
    generation: u64,
}

#[derive(Clone)]
struct RepairQueueState {
    estimated_total: f64,
    cost: f64,
    hops: usize,
    generation: u64,
    key: RepairSearchKey,
}

impl PartialEq for RepairQueueState {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for RepairQueueState {}

impl PartialOrd for RepairQueueState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RepairQueueState {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .estimated_total
            .total_cmp(&self.estimated_total)
            .then_with(|| other.cost.total_cmp(&self.cost))
            .then_with(|| other.hops.cmp(&self.hops))
            .then_with(|| other.key.node.cmp(&self.key.node))
            .then_with(|| compare_homotopy_words(&other.key.word, &self.key.word))
            .then_with(|| other.generation.cmp(&self.generation))
    }
}

fn compare_homotopy_words(first: &HomotopyWord, second: &HomotopyWord) -> Ordering {
    first
        .crossings()
        .iter()
        .zip(second.crossings())
        .find_map(|(first, second)| {
            let obstacle = first.obstacle.cmp(&second.obstacle);
            if obstacle != Ordering::Equal {
                return Some(obstacle);
            }
            let direction_rank = |direction| match direction {
                CrossingDirection::Positive => 0u8,
                CrossingDirection::Negative => 1u8,
            };
            let direction = direction_rank(first.direction).cmp(&direction_rank(second.direction));
            (direction != Ordering::Equal).then_some(direction)
        })
        .unwrap_or_else(|| first.crossings().len().cmp(&second.crossings().len()))
}

fn canonical_nonnegative(value: f64) -> Option<f64> {
    if !value.is_finite() || value < 0.0 {
        None
    } else if value == 0.0 {
        Some(0.0)
    } else {
        Some(value)
    }
}

#[derive(Clone, Copy)]
enum RepairHeuristic {
    Euclidean,
    Zero,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepairVisibilitySearchStrategy {
    FirstGeometricWitness,
    ProductRouteFamilies,
}

fn repair_heuristic(
    nodes: &[RepairNode],
    adjacency: &[Vec<VisibilityEdge>],
    mode: RepairHeuristic,
    work: &mut RouteRepairSearchWork,
) -> Vec<f64> {
    let mut heuristic = match mode {
        RepairHeuristic::Euclidean => nodes
            .iter()
            // A tiny downward scale keeps the Euclidean lower bound useful
            // while absorbing ordinary IEEE-754 triangle-inequality roundoff.
            // The directed exact check below remains authoritative and falls
            // back to zero if even this conservative value is inconsistent.
            .map(|node| length(sub(node.position, nodes[1].position)) * (1.0 - 16.0 * f64::EPSILON))
            .collect::<Vec<_>>(),
        RepairHeuristic::Zero => vec![0.0; nodes.len()],
    };
    work.heuristic_evaluations = heuristic.len();
    heuristic[1] = 0.0;
    let invalid = heuristic
        .iter()
        .any(|value| canonical_nonnegative(*value).is_none());
    let inconsistent = !invalid
        && adjacency.iter().enumerate().any(|(from, edges)| {
            edges.iter().any(|edge| {
                canonical_nonnegative(edge.length).is_none()
                    || heuristic[from].total_cmp(&(edge.length + heuristic[edge.to]))
                        == Ordering::Greater
            })
        });
    if invalid || inconsistent {
        heuristic.fill(0.0);
        work.heuristic_fallback_to_zero = true;
    }
    heuristic
}

fn compare_goal_candidates(first: &RepairCandidate, second: &RepairCandidate) -> Ordering {
    first
        .search_cost
        .total_cmp(&second.search_cost)
        .then_with(|| {
            compare_homotopy_words(&first.alternative.homotopy, &second.alternative.homotopy)
        })
        .then_with(|| first.hops.cmp(&second.hops))
        .then_with(|| first.node_path.cmp(&second.node_path))
}

fn compare_final_candidates(first: &RepairCandidate, second: &RepairCandidate) -> Ordering {
    first
        .alternative
        .length
        .total_cmp(&second.alternative.length)
        .then_with(|| {
            first
                .alternative
                .homotopy
                .crossings()
                .len()
                .cmp(&second.alternative.homotopy.crossings().len())
        })
        .then_with(|| {
            compare_homotopy_words(&first.alternative.homotopy, &second.alternative.homotopy)
        })
        .then_with(|| first.hops.cmp(&second.hops))
        .then_with(|| first.node_path.cmp(&second.node_path))
}

fn canonical_label_wins(
    existing: Option<RepairSearchLabel>,
    next_cost: f64,
    next_hops: usize,
    parent_path: usize,
    next_node: usize,
    path_arena: &mut RepairPathArena,
    work: &mut RouteRepairSearchWork,
) -> bool {
    let Some(existing) = existing else {
        return true;
    };
    match next_cost.total_cmp(&existing.cost) {
        Ordering::Less => true,
        Ordering::Greater => false,
        Ordering::Equal => match next_hops.cmp(&existing.hops) {
            Ordering::Less => true,
            Ordering::Greater => false,
            Ordering::Equal => {
                work.exact_cost_hop_path_comparisons += 1;
                match path_arena.compare_candidate(parent_path, next_node, existing.path, work) {
                    Ordering::Less => {
                        work.lexicographic_improvements += 1;
                        true
                    }
                    Ordering::Equal => {
                        work.identical_path_ties += 1;
                        false
                    }
                    Ordering::Greater => false,
                }
            }
        },
    }
}

fn repair_route_visibility(
    graph: &CorridorGraph,
    request: &RouteEmbeddingRequest<'_>,
    required_radius: f64,
    target_homotopy: &HomotopyWord,
) -> VisibilityRepairResult {
    repair_route_visibility_with_reference_penalties::<true>(
        graph,
        request,
        required_radius,
        target_homotopy,
        if request.target_homotopy.is_none() {
            RepairVisibilitySearchStrategy::FirstGeometricWitness
        } else {
            RepairVisibilitySearchStrategy::ProductRouteFamilies
        },
    )
}

fn repair_route_visibility_with_goal_limit(
    graph: &CorridorGraph,
    request: &RouteEmbeddingRequest<'_>,
    required_radius: f64,
    target_homotopy: &HomotopyWord,
    settled_state_budget: usize,
    exploratory_goal_limit: usize,
) -> VisibilityRepairResult {
    repair_route_visibility_with_search_goal_limit::<true>(
        graph,
        request,
        required_radius,
        target_homotopy,
        RepairHeuristic::Euclidean,
        RouteRepairWorkBudget {
            settled_states: settled_state_budget,
            ..RouteRepairWorkBudget::default()
        },
        exploratory_goal_limit,
        false,
        RepairVisibilitySearchStrategy::ProductRouteFamilies,
    )
}

fn repair_route_visibility_with_work_budget(
    graph: &CorridorGraph,
    request: &RouteEmbeddingRequest<'_>,
    required_radius: f64,
    target_homotopy: &HomotopyWord,
    work_budget: RouteRepairWorkBudget,
    exploratory_goal_limit: usize,
) -> VisibilityRepairResult {
    repair_route_visibility_with_search_goal_limit::<true>(
        graph,
        request,
        required_radius,
        target_homotopy,
        RepairHeuristic::Euclidean,
        work_budget,
        exploratory_goal_limit,
        true,
        RepairVisibilitySearchStrategy::ProductRouteFamilies,
    )
}

fn repair_route_visibility_with_reference_penalties<const PRECOMPUTE: bool>(
    graph: &CorridorGraph,
    request: &RouteEmbeddingRequest<'_>,
    required_radius: f64,
    target_homotopy: &HomotopyWord,
    strategy: RepairVisibilitySearchStrategy,
) -> VisibilityRepairResult {
    repair_route_visibility_with_search_goal_limit::<PRECOMPUTE>(
        graph,
        request,
        required_radius,
        target_homotopy,
        RepairHeuristic::Euclidean,
        RouteRepairWorkBudget {
            settled_states: MAX_REPAIR_SETTLED_STATES,
            ..RouteRepairWorkBudget::default()
        },
        FIRST_K_ROUTE_CLASS_FAMILY_LIMIT,
        false,
        strategy,
    )
}

#[cfg(test)]
fn repair_route_visibility_with_search<const PRECOMPUTE: bool>(
    graph: &CorridorGraph,
    request: &RouteEmbeddingRequest<'_>,
    required_radius: f64,
    target_homotopy: &HomotopyWord,
    heuristic_mode: RepairHeuristic,
    settled_state_budget: usize,
) -> VisibilityRepairResult {
    repair_route_visibility_with_search_goal_limit::<PRECOMPUTE>(
        graph,
        request,
        required_radius,
        target_homotopy,
        heuristic_mode,
        RouteRepairWorkBudget {
            settled_states: settled_state_budget,
            ..RouteRepairWorkBudget::default()
        },
        FIRST_K_ROUTE_CLASS_FAMILY_LIMIT,
        false,
        RepairVisibilitySearchStrategy::ProductRouteFamilies,
    )
}

fn repair_route_visibility_with_search_goal_limit<const PRECOMPUTE: bool>(
    graph: &CorridorGraph,
    request: &RouteEmbeddingRequest<'_>,
    required_radius: f64,
    target_homotopy: &HomotopyWord,
    heuristic_mode: RepairHeuristic,
    work_budget: RouteRepairWorkBudget,
    exploratory_goal_limit: usize,
    cumulative_visibility_budget: bool,
    strategy: RepairVisibilitySearchStrategy,
) -> VisibilityRepairResult {
    let spatial_index = RepairVisibilitySpatialIndex::build(graph);
    repair_route_visibility_with_search_goal_limit_indexed::<PRECOMPUTE>(
        graph,
        &spatial_index,
        request,
        required_radius,
        target_homotopy,
        heuristic_mode,
        work_budget,
        exploratory_goal_limit,
        cumulative_visibility_budget,
        strategy,
    )
}

fn repair_route_visibility_with_search_goal_limit_indexed<const PRECOMPUTE: bool>(
    graph: &CorridorGraph,
    spatial_index: &RepairVisibilitySpatialIndex,
    request: &RouteEmbeddingRequest<'_>,
    required_radius: f64,
    target_homotopy: &HomotopyWord,
    heuristic_mode: RepairHeuristic,
    work_budget: RouteRepairWorkBudget,
    exploratory_goal_limit: usize,
    cumulative_visibility_budget: bool,
    strategy: RepairVisibilitySearchStrategy,
) -> VisibilityRepairResult {
    let family_limit = match strategy {
        RepairVisibilitySearchStrategy::FirstGeometricWitness => 1,
        RepairVisibilitySearchStrategy::ProductRouteFamilies => {
            if request.persistent_basis_matches {
                1
            } else {
                exploratory_goal_limit
            }
        }
    };
    let excluded = route_endpoint_exemptions(request.from_component, request.to_component);
    let mut nodes = vec![
        RepairNode {
            id: 0,
            position: request.from,
            kind: RepairNodeKind::Source,
            feature: Some(request.from_component.into()),
        },
        RepairNode {
            id: 1,
            position: request.to,
            kind: RepairNodeKind::Target,
            feature: Some(request.to_component.into()),
        },
    ];
    for obstacle in &graph.obstacles {
        if excluded.contains(&obstacle.id.as_str()) {
            continue;
        }
        for corner in clearance_offset_corners(&obstacle.polygon, required_radius) {
            if segment_clearance_indexed(graph, spatial_index, [corner, corner], &[]).0 + EPSILON
                < required_radius
                || nodes
                    .iter()
                    .any(|node| length(sub(node.position, corner)) <= 1.0e-7)
            {
                continue;
            }
            nodes.push(RepairNode {
                id: nodes.len(),
                position: corner,
                kind: RepairNodeKind::ClearanceCorner,
                feature: Some(obstacle.id.clone()),
            });
        }
    }

    let geometry_features = graph
        .obstacles
        .len()
        .saturating_add(graph.cut_basis.cuts.len());
    let mut visibility = LazyVisibilityGraph::new(
        nodes.len(),
        geometry_features,
        work_budget.visibility_pair_evaluations,
        work_budget.visibility_geometry_units,
    );

    let source_key = RepairSearchKey {
        node: 0,
        word: HomotopyWord::empty(),
    };
    let mut work = empty_route_repair_search_work(family_limit);
    work.contract = match strategy {
        RepairVisibilitySearchStrategy::FirstGeometricWitness => {
            RepairSearchContract::NodeOnlyEuclideanAstarV1
        }
        RepairVisibilitySearchStrategy::ProductRouteFamilies => {
            RepairSearchContract::CanonicalEuclideanAstarV1
        }
    };
    work.budget_limit = work_budget.settled_states;
    // Every lazily admitted edge uses its exact Euclidean length plus a
    // nonnegative reference penalty. The conservative straight-line bound is
    // therefore consistent without materializing adjacency first.
    let heuristic = repair_heuristic(&nodes, &visibility.adjacency, heuristic_mode, &mut work);
    let mut path_arena = RepairPathArena::new(0);
    let source_label = RepairSearchLabel {
        cost: 0.0,
        hops: 0,
        path: 0,
        generation: 0,
    };
    let mut labels = HashMap::<RepairSearchKey, RepairSearchLabel>::new();
    let mut heap = BinaryHeap::new();
    labels.insert(source_key.clone(), source_label);
    heap.push(RepairQueueState {
        estimated_total: heuristic[0],
        cost: 0.0,
        hops: 0,
        generation: 0,
        key: source_key.clone(),
    });
    let maximum_word_length = if request.persistent_basis_matches {
        target_homotopy.crossings().len() + 4
    } else {
        8
    };
    let mut generation = 0u64;
    let mut state_limit_reached = false;
    let mut visibility_graph_limitation = None::<String>;
    let mut goal_candidates = HashMap::<HomotopyWord, RepairCandidate>::new();
    let mut cutoff = None::<f64>;
    let mut threshold_started = false;
    let mut search_complete = true;
    let mut termination = AlternativeSearchTermination::SearchExhausted;
    while let Some(next) = heap.peek() {
        if cutoff
            .is_some_and(|boundary| next.estimated_total.total_cmp(&boundary) == Ordering::Greater)
        {
            work.threshold_complete = true;
            search_complete = false;
            termination = AlternativeSearchTermination::FamilyLimitReached;
            break;
        }
        let RepairQueueState {
            cost,
            hops,
            generation: queued_generation,
            key,
            ..
        } = heap.pop().unwrap();
        work.heap_pops += 1;
        if threshold_started {
            work.threshold_extra_pops += 1;
        }
        let Some(label) = labels.get(&key).copied() else {
            work.stale_pops += 1;
            continue;
        };
        if label.generation != queued_generation {
            work.stale_pops += 1;
            continue;
        }
        debug_assert_eq!(label.cost.to_bits(), cost.to_bits());
        debug_assert_eq!(label.hops, hops);
        if work.settled_states >= work_budget.settled_states {
            state_limit_reached = true;
            search_complete = false;
            termination = AlternativeSearchTermination::StateLimitReached;
            break;
        }
        work.settled_states += 1;
        work.budget_consumed = work.settled_states;
        let target_state_matches = match strategy {
            RepairVisibilitySearchStrategy::FirstGeometricWitness => true,
            RepairVisibilitySearchStrategy::ProductRouteFamilies => {
                !request.persistent_basis_matches || key.word == *target_homotopy
            }
        };
        if key.node == 1 && target_state_matches {
            work.goals_settled += 1;
            let node_path = path_arena.materialize(label.path);
            work.output_materialized_path_elements += node_path.len();
            let polyline = node_path
                .iter()
                .map(|&node| nodes[node].position)
                .collect::<Vec<_>>();
            let clearance = certify_polyline_clearance_indexed(
                graph,
                spatial_index,
                &polyline,
                required_radius,
                request.from_component,
                request.to_component,
            );
            let signature = polyline_signature_indexed(
                &polyline,
                &graph.cut_basis,
                spatial_index,
                &excluded,
                &mut visibility.spatial_query_scratch,
            );
            let signature_matches_search = match strategy {
                RepairVisibilitySearchStrategy::FirstGeometricWitness => true,
                RepairVisibilitySearchStrategy::ProductRouteFamilies => {
                    signature.word == key.word
                        && (!request.persistent_basis_matches || signature.word == *target_homotopy)
                }
            };
            if clearance.certified && !signature.ambiguous && signature_matches_search {
                goal_candidates.insert(
                    signature.word.clone(),
                    RepairCandidate {
                        alternative: RepairAlternative {
                            length: polyline_length(&polyline),
                            polyline,
                            homotopy: signature.word,
                            minimum_clearance: clearance.minimum_clearance,
                        },
                        clearance,
                        node_path,
                        search_cost: label.cost,
                        hops: label.hops,
                    },
                );
                if goal_candidates.len() >= family_limit {
                    let mut ordered = goal_candidates.values().collect::<Vec<_>>();
                    ordered.sort_by(|first, second| compare_goal_candidates(first, second));
                    cutoff = Some(ordered[family_limit - 1].search_cost);
                    threshold_started = true;
                }
            }
            continue;
        }
        match visibility.expand::<PRECOMPUTE>(
            key.node,
            &nodes,
            graph,
            spatial_index,
            request,
            required_radius,
            &excluded,
            &heuristic,
        ) {
            Ok(()) => {}
            Err(LazyVisibilityExpansionError::WorkLimit(limitation)) => {
                visibility_graph_limitation = Some(limitation);
                search_complete = false;
                termination = AlternativeSearchTermination::VisibilityGraphLimitReached;
                break;
            }
            Err(LazyVisibilityExpansionError::HeuristicInconsistent) => {
                debug_assert!(matches!(heuristic_mode, RepairHeuristic::Euclidean));
                let fallback_budget = if cumulative_visibility_budget {
                    RouteRepairWorkBudget {
                        settled_states: work_budget.settled_states,
                        visibility_pair_evaluations: work_budget
                            .visibility_pair_evaluations
                            .saturating_sub(visibility.pair_evaluations),
                        visibility_geometry_units: work_budget
                            .visibility_geometry_units
                            .saturating_sub(visibility.geometry_units),
                    }
                } else {
                    work_budget
                };
                let mut fallback =
                    repair_route_visibility_with_search_goal_limit_indexed::<PRECOMPUTE>(
                        graph,
                        spatial_index,
                        request,
                        required_radius,
                        target_homotopy,
                        RepairHeuristic::Zero,
                        fallback_budget,
                        exploratory_goal_limit,
                        cumulative_visibility_budget,
                        strategy,
                    );
                fallback.evidence.search_work.heuristic_fallback_to_zero = true;
                if cumulative_visibility_budget {
                    fallback.visibility_work.pair_evaluation_limit =
                        work_budget.visibility_pair_evaluations;
                    fallback.visibility_work.pair_evaluations = fallback
                        .visibility_work
                        .pair_evaluations
                        .saturating_add(visibility.pair_evaluations);
                    fallback.visibility_work.geometry_unit_limit =
                        work_budget.visibility_geometry_units;
                    fallback.visibility_work.geometry_units = fallback
                        .visibility_work
                        .geometry_units
                        .saturating_add(visibility.geometry_units);
                    fallback.visibility_work.attempts += 1;
                }
                return fallback;
            }
        }
        work.expanded_states += 1;
        for edge in &visibility.adjacency[key.node] {
            work.relaxed_edges += 1;
            let next_word = match strategy {
                RepairVisibilitySearchStrategy::FirstGeometricWitness => HomotopyWord::empty(),
                RepairVisibilitySearchStrategy::ProductRouteFamilies => {
                    let next_word = key.word.extended(edge.word.crossings().iter().cloned());
                    if next_word.crossings().len() > maximum_word_length {
                        continue;
                    }
                    next_word
                }
            };
            let next_key = RepairSearchKey {
                node: edge.to,
                word: next_word,
            };
            let reference_penalty = if PRECOMPUTE {
                edge.reference_penalty
                    .expect("precomputed lazy edge carries its reference penalty")
            } else {
                let midpoint = scale(add(nodes[key.node].position, nodes[edge.to].position), 0.5);
                point_polyline_distance(midpoint, request.reference) * 0.1
            };
            let Some(edge_length) = canonical_nonnegative(edge.length) else {
                continue;
            };
            let Some(reference_penalty) = canonical_nonnegative(reference_penalty) else {
                continue;
            };
            let Some(next_cost) =
                canonical_nonnegative(label.cost + edge_length + reference_penalty)
            else {
                continue;
            };
            let Some(estimated_total) = canonical_nonnegative(next_cost + heuristic[edge.to])
            else {
                continue;
            };
            let next_hops = label.hops + 1;
            let accept = canonical_label_wins(
                labels.get(&next_key).copied(),
                next_cost,
                next_hops,
                label.path,
                edge.to,
                &mut path_arena,
                &mut work,
            );
            if !accept {
                continue;
            }
            let path = path_arena.accept(label.path, edge.to);
            generation += 1;
            let candidate = RepairSearchLabel {
                cost: next_cost,
                hops: next_hops,
                path,
                generation,
            };
            labels.insert(next_key.clone(), candidate);
            work.accepted_labels += 1;
            work.maximum_path_depth = work.maximum_path_depth.max(next_hops + 1);
            heap.push(RepairQueueState {
                estimated_total,
                cost: next_cost,
                hops: next_hops,
                generation,
                key: next_key,
            });
        }
    }
    if termination == AlternativeSearchTermination::SearchExhausted && cutoff.is_some() {
        work.threshold_complete = true;
    }
    work.path_arena_nodes = path_arena.nodes.len();
    if let Some(boundary) = cutoff {
        work.boundary_goals = goal_candidates
            .values()
            .filter(|candidate| candidate.search_cost.total_cmp(&boundary) == Ordering::Equal)
            .count();
    }
    let mut alternatives = goal_candidates.into_values().collect::<Vec<_>>();
    alternatives.sort_by(compare_goal_candidates);
    alternatives.truncate(family_limit);
    alternatives.sort_by(compare_final_candidates);
    let selected = alternatives.first().map(|candidate| SelectedRepair {
        polyline: candidate.alternative.polyline.clone(),
        clearance: candidate.clearance.clone(),
    });
    let outcome = if selected.is_some() {
        RepairOutcome::Repaired
    } else {
        RepairOutcome::SearchExhausted
    };
    #[cfg(test)]
    let ordered_node_paths = alternatives
        .iter()
        .map(|candidate| candidate.node_path.clone())
        .collect();
    #[cfg(test)]
    let ordered_search_cost_bits = alternatives
        .iter()
        .map(|candidate| candidate.search_cost.to_bits())
        .collect();
    let explored_states = work.heap_pops;
    let visibility_graph_limit_reached = visibility_graph_limitation.is_some();
    visibility
        .evidence_edges
        .sort_by_key(|edge| (edge.from, edge.to));
    let mut limitations = vec![
        "offset-corner visibility is exact-certified but conservative around rounded corners and overlapping expanded obstacles"
            .into(),
        "search_exhausted is not a proof of geometric infeasibility".into(),
    ];
    if let Some(limitation) = visibility_graph_limitation {
        limitations.push(limitation);
        limitations.push(
            "visibility work was materialized lazily; a bounded partial graph is incomplete evidence"
                .into(),
        );
    }
    VisibilityRepairResult {
        evidence: RouteRepairEvidence {
            method: RepairMethod::ClearanceOffsetVisibilityGraph,
            outcome,
            nodes,
            edges: visibility.evidence_edges,
            rejected_edges: visibility.rejected_edges,
            evaluated_edge_candidates: visibility.pair_evaluations,
            visibility_geometry_units: visibility.geometry_units,
            search_work: work,
            explored_states,
            termination,
            state_limit_reached,
            visibility_graph_limit_reached,
            alternatives: alternatives
                .into_iter()
                .map(|candidate| candidate.alternative)
                .collect(),
            selected_alternative: selected.as_ref().map(|_| 0),
            search_complete,
            limitations,
        },
        selected,
        visibility_work: RouteRepairVisibilityWork {
            pair_evaluation_limit: work_budget.visibility_pair_evaluations,
            pair_evaluations: visibility.pair_evaluations,
            geometry_unit_limit: work_budget.visibility_geometry_units,
            geometry_units: visibility.geometry_units,
            attempts: 1,
        },
        #[cfg(test)]
        ordered_node_paths,
        #[cfg(test)]
        ordered_search_cost_bits,
    }
}

fn clearance_offset_corners(polygon: &[Vec2], required_radius: f64) -> Vec<Vec2> {
    if polygon.len() < 3 {
        return Vec::new();
    }
    let signed_area = (0..polygon.len())
        .map(|index| cross(polygon[index], polygon[(index + 1) % polygon.len()]))
        .sum::<f64>()
        * 0.5;
    let outward_normal = |edge: Vec2| {
        let normal = if signed_area >= 0.0 {
            Vec2::new(edge.y, -edge.x)
        } else {
            Vec2::new(-edge.y, edge.x)
        };
        normalized_or(normal, Vec2::ZERO)
    };
    // Stay measurably outside the exact boundary. The final certificate still
    // uses `required_radius`; this construction margin only prevents polygon
    // offset and segment-distance roundoff from deleting tangent edges.
    let construction_margin = required_radius.max(1.0) * 1.0e-3;
    let offset = required_radius + construction_margin;
    let base = (0..polygon.len())
        .filter_map(|index| {
            let previous = polygon[(index + polygon.len() - 1) % polygon.len()];
            let corner = polygon[index];
            let next = polygon[(index + 1) % polygon.len()];
            let previous_normal = outward_normal(sub(corner, previous));
            let next_normal = outward_normal(sub(next, corner));
            let bisector = normalized_or(add(previous_normal, next_normal), Vec2::ZERO);
            let denominator = dot(bisector, previous_normal).min(dot(bisector, next_normal));
            (denominator > 1.0e-6).then(|| add(corner, scale(bisector, offset / denominator)))
        })
        .collect::<Vec<_>>();
    // A radial topology cut often meets an offset polygon exactly at a corner.
    // Slide disposable waypoints slightly along the offset boundary so the cut
    // is crossed in an edge interior and receives a well-defined sign.
    (0..base.len())
        .map(|index| {
            let tangent = normalized_or(
                sub(
                    base[(index + 1) % base.len()],
                    base[(index + base.len() - 1) % base.len()],
                ),
                Vec2::ZERO,
            );
            add(base[index], scale(tangent, construction_margin * 0.2))
        })
        .collect()
}

fn polyline_length(polyline: &[Vec2]) -> f64 {
    polyline
        .windows(2)
        .map(|segment| length(sub(segment[1], segment[0])))
        .sum()
}

fn semantic_projection(
    graph: &CorridorGraph,
    cells: &[usize],
    gates: &[usize],
) -> (Vec<usize>, Vec<usize>) {
    let semantic_regions = cells
        .iter()
        .map(|&cell| graph.semantic.raw_cell_to_region[cell])
        .fold(Vec::new(), |mut reduced, region| {
            if reduced.last() != Some(&region) {
                reduced.push(region);
            }
            reduced
        });
    let semantic_passages = gates
        .iter()
        .filter_map(|&gate| graph.semantic.raw_gate_to_passage[gate])
        .fold(Vec::new(), |mut reduced, passage| {
            if reduced.last() != Some(&passage) {
                reduced.push(passage);
            }
            reduced
        });
    (semantic_regions, semantic_passages)
}

/// Attribute an already-certified polyline to the raw triangulation without
/// changing its geometry. Gate crossings at the same polyline position are
/// processed as a small local graph: this handles the common case where a route
/// passes exactly through a triangulation vertex.
fn project_polyline_to_corridor(
    graph: &CorridorGraph,
    polyline: &[Vec2],
    source: usize,
    target: usize,
) -> Option<(Vec<usize>, Vec<usize>)> {
    project_polyline_by_gate_events(graph, polyline, source, target)
        .or_else(|| project_polyline_by_touched_cells(graph, polyline, source, target))
}

fn raw_projection_attempt_evidence(
    graph: &CorridorGraph,
    polyline: &[Vec2],
    source: usize,
    target: usize,
    from_component: &str,
    to_component: &str,
    outcome: RawProjectionAttemptOutcome,
) -> RawProjectionAttemptEvidence {
    let positive_overlap = graph
        .cells
        .iter()
        .map(|cell| {
            polyline.windows(2).any(|segment| {
                segment_triangle_interval([segment[0], segment[1]], cell.triangle)
                    .is_some_and(|(start, end)| end - start > TERMINAL_INTERVAL_PARAMETER_EPSILON)
            })
        })
        .collect::<Vec<_>>();
    let positive_overlap_cell_count = positive_overlap.iter().filter(|&&hit| hit).count();
    let mut touched = positive_overlap.clone();
    if let Some(source_touched) = touched.get_mut(source) {
        *source_touched = true;
    }
    if let Some(target_touched) = touched.get_mut(target) {
        *target_touched = true;
    }

    let mut adjacency = vec![Vec::<usize>::new(); graph.cells.len()];
    for gate in &graph.gates {
        adjacency[gate.cells[0]].push(gate.cells[1]);
        adjacency[gate.cells[1]].push(gate.cells[0]);
    }
    for neighbors in &mut adjacency {
        neighbors.sort_unstable();
        neighbors.dedup();
    }
    let graph_reachable = reachable_raw_cells(&adjacency, source, None);
    let source_touched_reachable = reachable_raw_cells(&adjacency, source, Some(&touched));
    let target_touched_reachable = reachable_raw_cells(&adjacency, target, Some(&touched));

    let mut uncovered_intervals = Vec::new();
    let mut uncovered_interval_count = 0usize;
    let from_exemptions = endpoint_obstacle_exemptions(from_component);
    let to_exemptions = endpoint_obstacle_exemptions(to_component);
    for (segment_index, segment) in polyline.windows(2).enumerate() {
        if length(sub(segment[1], segment[0])) <= EPSILON {
            continue;
        }
        let mut covered = graph
            .cells
            .iter()
            .filter_map(|cell| segment_triangle_interval([segment[0], segment[1]], cell.triangle))
            .filter(|(start, end)| end - start > TERMINAL_INTERVAL_PARAMETER_EPSILON)
            .map(|(start, end)| (start.clamp(0.0, 1.0), end.clamp(0.0, 1.0)))
            .collect::<Vec<_>>();
        covered.sort_by(|first, second| {
            first
                .0
                .total_cmp(&second.0)
                .then_with(|| first.1.total_cmp(&second.1))
        });
        let mut cursor = 0.0_f64;
        for (start, end) in covered {
            if start > cursor + TERMINAL_INTERVAL_PARAMETER_EPSILON {
                uncovered_interval_count += 1;
                if uncovered_intervals.len() < MAX_TERMINAL_PROJECTION_CANDIDATES {
                    uncovered_intervals.push(uncovered_interval_evidence(
                        graph,
                        [segment[0], segment[1]],
                        segment_index,
                        cursor,
                        start,
                        &from_exemptions,
                        &to_exemptions,
                    ));
                }
            }
            cursor = cursor.max(end);
        }
        if cursor < 1.0 - TERMINAL_INTERVAL_PARAMETER_EPSILON {
            uncovered_interval_count += 1;
            if uncovered_intervals.len() < MAX_TERMINAL_PROJECTION_CANDIDATES {
                uncovered_intervals.push(uncovered_interval_evidence(
                    graph,
                    [segment[0], segment[1]],
                    segment_index,
                    cursor,
                    1.0,
                    &from_exemptions,
                    &to_exemptions,
                ));
            }
        }
    }

    RawProjectionAttemptEvidence {
        source_cell: source,
        target_cell: target,
        outcome,
        source_target_graph_connected: graph_reachable.get(target).copied().unwrap_or(false),
        positive_overlap_cell_count,
        source_reachable_touched_cell_count: source_touched_reachable
            .iter()
            .filter(|&&reached| reached)
            .count(),
        target_reachable_touched_cell_count: target_touched_reachable
            .iter()
            .filter(|&&reached| reached)
            .count(),
        source_target_touched_connected: source_touched_reachable
            .get(target)
            .copied()
            .unwrap_or(false),
        uncovered_intervals,
        uncovered_interval_count,
    }
}

fn uncovered_interval_evidence(
    graph: &CorridorGraph,
    segment: [Vec2; 2],
    segment_index: usize,
    start_parameter: f64,
    end_parameter: f64,
    from_exemptions: &[&str],
    to_exemptions: &[&str],
) -> PolylineProgressIntervalEvidence {
    let parameter = (start_parameter + end_parameter) * 0.5;
    let midpoint = add(
        scale(segment[0], 1.0 - parameter),
        scale(segment[1], parameter),
    );
    let mut covering_obstacles = graph
        .obstacles
        .iter()
        .filter(|obstacle| point_in_convex_polygon(midpoint, &obstacle.polygon))
        .map(|obstacle| {
            let from = from_exemptions.contains(&obstacle.id.as_str());
            let to = to_exemptions.contains(&obstacle.id.as_str());
            UncoveredIntervalObstacleEvidence {
                obstacle: obstacle.id.clone(),
                endpoint_exemption: match (from, to) {
                    (true, true) => EndpointExemptionMatch::Both,
                    (true, false) => EndpointExemptionMatch::From,
                    (false, true) => EndpointExemptionMatch::To,
                    (false, false) => EndpointExemptionMatch::Neither,
                },
            }
        })
        .collect::<Vec<_>>();
    covering_obstacles.sort_by(|first, second| first.obstacle.cmp(&second.obstacle));
    let covering_obstacle_count = covering_obstacles.len();
    let exclusively_endpoint_exempt = !covering_obstacles.is_empty()
        && covering_obstacles
            .iter()
            .all(|obstacle| obstacle.endpoint_exemption != EndpointExemptionMatch::Neither);
    covering_obstacles.truncate(MAX_TERMINAL_PROJECTION_CANDIDATES);
    PolylineProgressIntervalEvidence {
        segment_index,
        start_parameter,
        end_parameter,
        covering_obstacles,
        covering_obstacle_count,
        exclusively_endpoint_exempt,
    }
}

fn reachable_raw_cells(
    adjacency: &[Vec<usize>],
    source: usize,
    allowed: Option<&[bool]>,
) -> Vec<bool> {
    let mut reached = vec![false; adjacency.len()];
    if source >= adjacency.len()
        || allowed.is_some_and(|allowed| !allowed.get(source).copied().unwrap_or(false))
    {
        return reached;
    }
    reached[source] = true;
    let mut queue = VecDeque::from([source]);
    while let Some(cell) = queue.pop_front() {
        for &next in &adjacency[cell] {
            if reached[next]
                || allowed.is_some_and(|allowed| !allowed.get(next).copied().unwrap_or(false))
            {
                continue;
            }
            reached[next] = true;
            queue.push_back(next);
        }
    }
    reached
}

/// Preserve the ordinary terminal-sector projection exactly. Only after that
/// projection fails may a certified, unchanged witness retry with anchors at
/// the first positive-measure free-space intervals beyond endpoint-exempt
/// gaps. The boolean records whether that retry supplied the accepted raw
/// witness, so serialized diagnostics do not hide the fallback.
struct CertifiedReferenceProjection {
    traversal: Option<(Vec<usize>, Vec<usize>)>,
    used_gap_anchor_retry: bool,
    evidence: Option<RouteProjectionEvidence>,
}

fn project_certified_reference_to_corridor(
    graph: &CorridorGraph,
    request: &RouteEmbeddingRequest<'_>,
    source: usize,
    target: usize,
) -> CertifiedReferenceProjection {
    if let Some(traversal) = project_polyline_to_corridor(graph, request.reference, source, target)
    {
        return CertifiedReferenceProjection {
            traversal: Some(traversal),
            used_gap_anchor_retry: false,
            evidence: None,
        };
    }
    let source_projection = terminal_projection(graph, request.from, request.from_toward);
    let target_projection = terminal_projection(graph, request.to, request.to_toward);
    let mut evidence = RouteProjectionEvidence {
        source: source_projection.evidence,
        target: target_projection.evidence,
        ordinary_attempt: Some(raw_projection_attempt_evidence(
            graph,
            request.reference,
            source,
            target,
            request.from_component,
            request.to_component,
            RawProjectionAttemptOutcome::Disconnected,
        )),
        gap_retry_attempt: None,
        failure_reasons: vec![RouteProjectionFailureReason::OrdinaryRawChainDisconnected],
    };

    match source_projection.gap_anchor {
        GapAnchor::Missing => evidence
            .failure_reasons
            .push(RouteProjectionFailureReason::SourceGapAnchorMissing),
        GapAnchor::AmbiguousRegions => evidence
            .failure_reasons
            .push(RouteProjectionFailureReason::SourceGapAnchorAmbiguous),
        GapAnchor::NoGapKeepLegacy | GapAnchor::Unique(_) => {}
    }
    match target_projection.gap_anchor {
        GapAnchor::Missing => evidence
            .failure_reasons
            .push(RouteProjectionFailureReason::TargetGapAnchorMissing),
        GapAnchor::AmbiguousRegions => evidence
            .failure_reasons
            .push(RouteProjectionFailureReason::TargetGapAnchorAmbiguous),
        GapAnchor::NoGapKeepLegacy | GapAnchor::Unique(_) => {}
    }
    if evidence.failure_reasons.iter().any(|reason| {
        matches!(
            reason,
            RouteProjectionFailureReason::SourceGapAnchorMissing
                | RouteProjectionFailureReason::TargetGapAnchorMissing
                | RouteProjectionFailureReason::SourceGapAnchorAmbiguous
                | RouteProjectionFailureReason::TargetGapAnchorAmbiguous
        )
    }) {
        return CertifiedReferenceProjection {
            traversal: None,
            used_gap_anchor_retry: false,
            evidence: Some(evidence),
        };
    }

    let Some((retry_source, retry_target)) = resolve_gap_anchor_pair(
        source,
        target,
        source_projection.gap_anchor,
        target_projection.gap_anchor,
    ) else {
        evidence
            .failure_reasons
            .push(RouteProjectionFailureReason::GapRetryUnchanged);
        return CertifiedReferenceProjection {
            traversal: None,
            used_gap_anchor_retry: false,
            evidence: Some(evidence),
        };
    };
    let traversal =
        project_polyline_to_corridor(graph, request.reference, retry_source, retry_target);
    let outcome = if traversal.is_some() {
        RawProjectionAttemptOutcome::Connected
    } else {
        evidence
            .failure_reasons
            .push(RouteProjectionFailureReason::GapRetryRawChainDisconnected);
        RawProjectionAttemptOutcome::Disconnected
    };
    evidence.gap_retry_attempt = Some(raw_projection_attempt_evidence(
        graph,
        request.reference,
        retry_source,
        retry_target,
        request.from_component,
        request.to_component,
        outcome,
    ));
    CertifiedReferenceProjection {
        traversal,
        used_gap_anchor_retry: outcome == RawProjectionAttemptOutcome::Connected,
        evidence: Some(evidence),
    }
}

fn resolve_gap_anchor_pair(
    original_source: usize,
    original_target: usize,
    retry_source: GapAnchor,
    retry_target: GapAnchor,
) -> Option<(usize, usize)> {
    let source = match retry_source {
        GapAnchor::NoGapKeepLegacy => original_source,
        GapAnchor::Unique(cell) => cell,
        GapAnchor::Missing | GapAnchor::AmbiguousRegions => return None,
    };
    let target = match retry_target {
        GapAnchor::NoGapKeepLegacy => original_target,
        GapAnchor::Unique(cell) => cell,
        GapAnchor::Missing | GapAnchor::AmbiguousRegions => return None,
    };
    ((source, target) != (original_source, original_target)).then_some((source, target))
}

fn project_polyline_by_gate_events(
    graph: &CorridorGraph,
    polyline: &[Vec2],
    source: usize,
    target: usize,
) -> Option<(Vec<usize>, Vec<usize>)> {
    if polyline.len() < 2 {
        return None;
    }
    let mut events = Vec::<(f64, usize)>::new();
    for (segment_index, segment) in polyline.windows(2).enumerate() {
        if length(sub(segment[1], segment[0])) <= EPSILON {
            continue;
        }
        for gate in &graph.gates {
            if let Some(parameter) = segment_intersection_parameter(
                segment[0],
                segment[1],
                gate.segment[0],
                gate.segment[1],
            ) {
                events.push((segment_index as f64 + parameter, gate.id));
            }
        }
    }
    events.sort_by(|first, second| {
        first
            .0
            .total_cmp(&second.0)
            .then_with(|| first.1.cmp(&second.1))
    });

    let mut cells = vec![source];
    let mut gates = Vec::new();
    let mut current = source;
    let mut event_index = 0;
    while event_index < events.len() {
        let progress = events[event_index].0;
        let mut event_gates = Vec::new();
        while event_index < events.len() && (events[event_index].0 - progress).abs() <= 1.0e-7 {
            if event_gates.last() != Some(&events[event_index].1) {
                event_gates.push(events[event_index].1);
            }
            event_index += 1;
        }
        event_gates.sort_unstable();
        event_gates.dedup();

        let after = point_after_polyline_progress(polyline, progress);
        let mut desired = graph
            .cells
            .iter()
            .filter(|cell| point_in_triangle(after, cell.triangle))
            .map(|cell| cell.id)
            .collect::<Vec<_>>();
        desired.sort_unstable();
        desired.dedup();
        if desired.is_empty() && progress >= polyline.len() as f64 - 2.0 - 1.0e-7 {
            desired.push(target);
        }

        let Some(transitions) = event_transition_path(graph, current, &desired, &event_gates)
        else {
            // A tangential touch need not change cells. Ignore it if the
            // current cell still contains the route immediately afterwards.
            if desired.contains(&current) {
                continue;
            }
            return None;
        };
        for (gate, next) in transitions {
            gates.push(gate);
            cells.push(next);
            current = next;
        }
    }

    (current == target).then_some((cells, gates))
}

/// Degenerate attribution fallback for a route lying along a CDT edge. Choose
/// a deterministic connected chain through triangles which overlap the route
/// for positive length. Cells touching only at an isolated vertex are excluded
/// so they cannot create artificial shortcuts.
///
/// TODO: replace this progressless raw-cell BFS with the bounded,
/// progress-ordered semantic automaton described in `docs/corridors.md` before
/// broadening its authority. In particular, never make a failed traversal mean
/// empty passage use or add cells which have no positive-length route overlap.
fn project_polyline_by_touched_cells(
    graph: &CorridorGraph,
    polyline: &[Vec2],
    source: usize,
    target: usize,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let touched = graph
        .cells
        .iter()
        .map(|cell| {
            cell.id == source
                || cell.id == target
                || polyline.windows(2).any(|segment| {
                    segment_triangle_interval([segment[0], segment[1]], cell.triangle)
                        .is_some_and(|(start, end)| end - start > 1.0e-8)
                })
        })
        .collect::<Vec<_>>();
    let mut adjacency = vec![Vec::<(usize, usize)>::new(); graph.cells.len()];
    for gate in &graph.gates {
        if touched[gate.cells[0]] && touched[gate.cells[1]] {
            adjacency[gate.cells[0]].push((gate.cells[1], gate.id));
            adjacency[gate.cells[1]].push((gate.cells[0], gate.id));
        }
    }
    for neighbors in &mut adjacency {
        neighbors.sort_by_key(|&(cell, gate)| (gate, cell));
    }

    let mut queue = VecDeque::from([source]);
    let mut previous = vec![None::<(usize, usize)>; graph.cells.len()];
    previous[source] = Some((source, usize::MAX));
    while let Some(cell) = queue.pop_front() {
        if cell == target {
            break;
        }
        for &(next, gate) in &adjacency[cell] {
            if previous[next].is_none() {
                previous[next] = Some((cell, gate));
                queue.push_back(next);
            }
        }
    }
    previous[target]?;
    let mut cursor = target;
    let mut cells = vec![target];
    let mut gates = Vec::new();
    while cursor != source {
        let (parent, gate) = previous[cursor]?;
        gates.push(gate);
        cells.push(parent);
        cursor = parent;
    }
    cells.reverse();
    gates.reverse();
    Some((cells, gates))
}

fn event_transition_path(
    graph: &CorridorGraph,
    source: usize,
    targets: &[usize],
    event_gates: &[usize],
) -> Option<Vec<(usize, usize)>> {
    if targets.contains(&source) {
        return Some(Vec::new());
    }
    let mut queue = VecDeque::from([source]);
    let mut previous = HashMap::<usize, (usize, usize)>::new();
    let mut reached = None;
    while let Some(cell) = queue.pop_front() {
        for &gate_id in event_gates {
            let gate = &graph.gates[gate_id];
            let next = if gate.cells[0] == cell {
                gate.cells[1]
            } else if gate.cells[1] == cell {
                gate.cells[0]
            } else {
                continue;
            };
            if next == source || previous.contains_key(&next) {
                continue;
            }
            previous.insert(next, (cell, gate_id));
            if targets.contains(&next) {
                reached = Some(next);
                break;
            }
            queue.push_back(next);
        }
        if reached.is_some() {
            break;
        }
    }
    let mut cursor = reached?;
    let mut transitions = Vec::new();
    while cursor != source {
        let (parent, gate) = previous[&cursor];
        transitions.push((gate, cursor));
        cursor = parent;
    }
    transitions.reverse();
    Some(transitions)
}

fn point_after_polyline_progress(polyline: &[Vec2], progress: f64) -> Vec2 {
    let maximum = (polyline.len() - 1) as f64;
    let after = (progress + 1.0e-6).min(maximum);
    let segment = (after.floor() as usize).min(polyline.len() - 2);
    let parameter = (after - segment as f64).clamp(0.0, 1.0);
    add(
        scale(polyline[segment], 1.0 - parameter),
        scale(polyline[segment + 1], parameter),
    )
}

fn segment_intersection_parameter(
    route_from: Vec2,
    route_to: Vec2,
    gate_from: Vec2,
    gate_to: Vec2,
) -> Option<f64> {
    let route = sub(route_to, route_from);
    let gate = sub(gate_to, gate_from);
    let denominator = cross(route, gate);
    if denominator.abs() <= EPSILON {
        return None;
    }
    let offset = sub(gate_from, route_from);
    let route_parameter = cross(offset, gate) / denominator;
    let gate_parameter = cross(offset, route) / denominator;
    ((-EPSILON..=1.0 + EPSILON).contains(&route_parameter)
        && (-EPSILON..=1.0 + EPSILON).contains(&gate_parameter))
    .then_some(route_parameter.clamp(0.0, 1.0))
}

fn string_pull_portals(
    graph: &CorridorGraph,
    cells: &[usize],
    gates: &[usize],
    from: Vec2,
    to: Vec2,
    portal_inset: f64,
) -> Vec<Vec2> {
    if gates.is_empty() {
        return vec![from, to];
    }
    let mut portals = Vec::<[Vec2; 2]>::with_capacity(gates.len() + 2);
    portals.push([from, from]);
    for (index, &gate_id) in gates.iter().enumerate() {
        let gate = &graph.gates[gate_id];
        let gate_direction = normalized_or(sub(gate.segment[1], gate.segment[0]), Vec2::ZERO);
        let safe_segment = [
            add(gate.segment[0], scale(gate_direction, portal_inset)),
            add(gate.segment[1], scale(gate_direction, -portal_inset)),
        ];
        let origin = graph.cells[cells[index]].center;
        let direction = sub(graph.cells[cells[index + 1]].center, origin);
        let first_side = cross(direction, sub(safe_segment[0], origin));
        let second_side = cross(direction, sub(safe_segment[1], origin));
        portals.push(if first_side >= second_side {
            [safe_segment[0], safe_segment[1]]
        } else {
            [safe_segment[1], safe_segment[0]]
        });
    }
    portals.push([to, to]);

    let mut result = vec![from];
    let mut apex = from;
    let mut left = from;
    let mut right = from;
    let mut left_index = 0usize;
    let mut right_index = 0usize;
    let mut index = 1usize;
    while index < portals.len() {
        let new_left = portals[index][0];
        let new_right = portals[index][1];

        if twice_triangle_area(apex, right, new_right) <= 0.0 {
            if nearly_same(apex, right) || twice_triangle_area(apex, left, new_right) > 0.0 {
                right = new_right;
                right_index = index;
            } else {
                result.push(left);
                apex = left;
                let apex_index = left_index;
                left = apex;
                right = apex;
                left_index = apex_index;
                right_index = apex_index;
                index = apex_index + 1;
                continue;
            }
        }

        if twice_triangle_area(apex, left, new_left) >= 0.0 {
            if nearly_same(apex, left) || twice_triangle_area(apex, right, new_left) < 0.0 {
                left = new_left;
                left_index = index;
            } else {
                result.push(right);
                apex = right;
                let apex_index = right_index;
                left = apex;
                right = apex;
                left_index = apex_index;
                right_index = apex_index;
                index = apex_index + 1;
                continue;
            }
        }
        index += 1;
    }
    if !nearly_same(*result.last().unwrap(), to) {
        result.push(to);
    }
    result
}

fn twice_triangle_area(first: Vec2, second: Vec2, third: Vec2) -> f64 {
    cross(sub(second, first), sub(third, first))
}

fn nearly_same(first: Vec2, second: Vec2) -> bool {
    length(sub(first, second)) <= EPSILON
}

#[allow(clippy::too_many_arguments)]
fn shortcut_within_corridor(
    graph: &CorridorGraph,
    points: &[Vec2],
    cells: &[usize],
    basis: &CutBasis,
    excluded: &[&str],
    required_radius: f64,
    from_component: &str,
    to_component: &str,
) -> Vec<Vec2> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let last = points.len() - 1;
    let mut result = vec![points[0]];
    let mut from_index = 0usize;
    while from_index < last {
        let mut chosen = from_index + 1;
        for to_index in ((from_index + 2)..=last).rev() {
            let first_cell = from_index.saturating_sub(1);
            let last_cell = if to_index == last {
                cells.len() - 1
            } else {
                to_index - 1
            };
            if !segment_covered_by_cells(
                [points[from_index], points[to_index]],
                &cells[first_cell..=last_cell]
                    .iter()
                    .map(|&cell| graph.cells[cell].triangle)
                    .collect::<Vec<_>>(),
            ) {
                continue;
            }
            let mut exempt = Vec::with_capacity(2);
            if from_index == 0 {
                exempt.push(from_component);
            }
            if to_index == last {
                exempt.push(to_component);
            }
            if !segment_has_clearance(
                graph,
                [points[from_index], points[to_index]],
                required_radius,
                &exempt,
            ) {
                continue;
            }
            let original = polyline_signature(&points[from_index..=to_index], basis, excluded);
            let shortcut =
                polyline_signature(&[points[from_index], points[to_index]], basis, excluded);
            if !original.ambiguous && !shortcut.ambiguous && original.word == shortcut.word {
                chosen = to_index;
                break;
            }
        }
        result.push(points[chosen]);
        from_index = chosen;
    }
    result
}

fn segment_covered_by_cells(segment: [Vec2; 2], triangles: &[[Vec2; 3]]) -> bool {
    let mut intervals = triangles
        .iter()
        .filter_map(|&triangle| segment_triangle_interval(segment, triangle))
        .collect::<Vec<_>>();
    intervals.sort_by(|first, second| first.0.total_cmp(&second.0));
    let mut covered_until = 0.0;
    for (start, end) in intervals {
        if start > covered_until + EPSILON {
            return false;
        }
        covered_until = covered_until.max(end);
        if covered_until >= 1.0 - EPSILON {
            return true;
        }
    }
    false
}

fn segment_triangle_interval(segment: [Vec2; 2], triangle: [Vec2; 3]) -> Option<(f64, f64)> {
    let orientation = cross(sub(triangle[1], triangle[0]), sub(triangle[2], triangle[0]));
    let orientation_sign = if orientation >= 0.0 { 1.0 } else { -1.0 };
    let delta = sub(segment[1], segment[0]);
    let mut minimum = 0.0_f64;
    let mut maximum = 1.0_f64;
    for index in 0..3 {
        let edge = sub(triangle[(index + 1) % 3], triangle[index]);
        let initial = orientation_sign * cross(edge, sub(segment[0], triangle[index]));
        let slope = orientation_sign * cross(edge, delta);
        if slope.abs() <= EPSILON {
            if initial < -EPSILON {
                return None;
            }
            continue;
        }
        let boundary = (-EPSILON - initial) / slope;
        if slope > 0.0 {
            minimum = minimum.max(boundary);
        } else {
            maximum = maximum.min(boundary);
        }
        if minimum > maximum + EPSILON {
            return None;
        }
    }
    Some((minimum.clamp(0.0, 1.0), maximum.clamp(0.0, 1.0)))
}

#[derive(Clone, Debug)]
struct SignatureResult {
    word: HomotopyWord,
    ambiguous: bool,
}

/// Certify a polyline's signed cut word against one exact cut-basis epoch.
/// Endpoint obstacle exemptions use the same semantics as corridor embedding.
/// A cut touch/overlap is rejected because it does not define a stable side.
pub fn certify_route_homotopy(
    polyline: &[Vec2],
    basis: &CutBasis,
    from_obstacle: &str,
    to_obstacle: &str,
) -> Result<HomotopyWord, ()> {
    let excluded = route_endpoint_exemptions(from_obstacle, to_obstacle);
    let signature = polyline_signature(polyline, basis, &excluded);
    if signature.ambiguous {
        Err(())
    } else {
        Ok(signature.word)
    }
}

fn endpoint_obstacle_exemptions(obstacle: &str) -> Vec<&str> {
    let mut exemptions = vec![obstacle];
    if let Some(pad) = obstacle.strip_prefix("pad:")
        && let Some((component, _)) = pad.split_once(':')
    {
        // Component-body keepouts often overlap their own SMD lands. Copper
        // must be able to leave the connected pad; this does not exempt any
        // unrelated pad or component.
        exemptions.push(component);
    }
    exemptions
}

fn route_endpoint_exemptions<'a>(from: &'a str, to: &'a str) -> Vec<&'a str> {
    let mut exemptions = endpoint_obstacle_exemptions(from);
    exemptions.extend(endpoint_obstacle_exemptions(to));
    exemptions.sort_unstable();
    exemptions.dedup();
    exemptions
}

fn polyline_signature(
    polyline: &[Vec2],
    basis: &CutBasis,
    excluded_obstacles: &[&str],
) -> SignatureResult {
    let mut events = Vec::<(usize, f64, CutCrossing)>::new();
    let mut ambiguous = false;
    let exempt_components = basis
        .components
        .iter()
        .filter(|component| {
            component
                .members
                .iter()
                .any(|member| excluded_obstacles.contains(&member.as_str()))
        })
        .map(|component| component.id.as_str())
        .collect::<BTreeSet<_>>();
    for (segment_index, segment) in polyline.windows(2).enumerate() {
        for cut in &basis.cuts {
            if exempt_components.contains(cut.source_component.as_str()) {
                continue;
            }
            match segment_cut_crossing(segment[0], segment[1], cut) {
                CutIntersection::Crossing { route_t, crossing } => {
                    events.push((segment_index, route_t, crossing));
                }
                CutIntersection::Ambiguous => ambiguous = true,
                CutIntersection::None => {}
            }
        }
    }
    events.sort_by(|first, second| {
        first
            .0
            .cmp(&second.0)
            .then_with(|| first.1.total_cmp(&second.1))
    });
    SignatureResult {
        word: HomotopyWord::reduced(events.into_iter().map(|(_, _, crossing)| crossing)),
        ambiguous,
    }
}

fn collect_intersecting_repair_primitives(
    bvh: &RepairBvh,
    node_index: usize,
    query: RepairSpatialAabb,
    output: &mut Vec<usize>,
) {
    let node = &bvh.nodes[node_index];
    if !node.bounds.intersects(query) {
        return;
    }
    match node.kind {
        RepairBvhNodeKind::Leaf { start, len } => {
            output.extend_from_slice(&bvh.primitives[start..start + len]);
        }
        RepairBvhNodeKind::Branch { left, right } => {
            collect_intersecting_repair_primitives(bvh, left, query, output);
            collect_intersecting_repair_primitives(bvh, right, query, output);
        }
    }
}

fn polyline_signature_indexed(
    polyline: &[Vec2],
    basis: &CutBasis,
    spatial_index: &RepairVisibilitySpatialIndex,
    excluded_obstacles: &[&str],
    scratch: &mut RepairSpatialQueryScratch,
) -> SignatureResult {
    #[cfg(test)]
    {
        scratch.exact_cut_tests = 0;
    }
    if !spatial_index.cut_geometry_valid
        || polyline
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return SignatureResult {
            word: HomotopyWord::empty(),
            ambiguous: true,
        };
    }
    let exempt_components = basis
        .components
        .iter()
        .filter(|component| {
            component
                .members
                .iter()
                .any(|member| excluded_obstacles.contains(&member.as_str()))
        })
        .map(|component| component.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut events = Vec::<(usize, f64, CutCrossing)>::new();
    let mut ambiguous = false;
    for (segment_index, segment) in polyline.windows(2).enumerate() {
        let route = [segment[0], segment[1]];
        let Some(route_bounds) = RepairSpatialAabb::from_points(&route) else {
            ambiguous = true;
            continue;
        };
        let route_length = length(sub(route[1], route[0]));
        if !route_length.is_finite() {
            ambiguous = true;
            continue;
        }
        scratch.cut_candidates.clear();
        if let Some(root) = spatial_index.cuts.root {
            collect_intersecting_repair_primitives(
                &spatial_index.cuts,
                root,
                route_bounds.expanded(EPSILON * route_length + EPSILON),
                &mut scratch.cut_candidates,
            );
        }
        // BVH traversal order is an implementation detail. Restore exact cut
        // basis order before invoking the narrow phase so equal-parameter
        // crossing events retain the exhaustive algorithm's stable ordering.
        scratch.cut_candidates.sort_unstable();
        scratch.cut_candidates.dedup();
        for &cut_index in &scratch.cut_candidates {
            let cut = &basis.cuts[cut_index];
            if exempt_components.contains(cut.source_component.as_str()) {
                continue;
            }
            #[cfg(test)]
            {
                scratch.exact_cut_tests += 1;
            }
            match segment_cut_crossing(route[0], route[1], cut) {
                CutIntersection::Crossing { route_t, crossing } => {
                    events.push((segment_index, route_t, crossing));
                }
                CutIntersection::Ambiguous => ambiguous = true,
                CutIntersection::None => {}
            }
        }
    }
    events.sort_by(|first, second| {
        first
            .0
            .cmp(&second.0)
            .then_with(|| first.1.total_cmp(&second.1))
    });
    SignatureResult {
        word: HomotopyWord::reduced(events.into_iter().map(|(_, _, crossing)| crossing)),
        ambiguous,
    }
}

enum CutIntersection {
    None,
    Crossing { route_t: f64, crossing: CutCrossing },
    Ambiguous,
}

fn segment_cut_crossing(from: Vec2, to: Vec2, cut: &TopologyCut) -> CutIntersection {
    let route = sub(to, from);
    let cut_vector = sub(cut.segment[1], cut.segment[0]);
    let denominator = cross(route, cut_vector);
    if denominator.abs() <= EPSILON {
        return if segment_distance(from, to, cut.segment[0], cut.segment[1]) <= EPSILON {
            CutIntersection::Ambiguous
        } else {
            CutIntersection::None
        };
    }
    let offset = sub(cut.segment[0], from);
    let route_t = cross(offset, cut_vector) / denominator;
    let cut_t = cross(offset, route) / denominator;
    if !(-EPSILON..=1.0 + EPSILON).contains(&route_t)
        || !(-EPSILON..=1.0 + EPSILON).contains(&cut_t)
    {
        return CutIntersection::None;
    }
    if route_t <= EPSILON || route_t >= 1.0 - EPSILON || cut_t <= EPSILON || cut_t >= 1.0 - EPSILON
    {
        return CutIntersection::Ambiguous;
    }
    CutIntersection::Crossing {
        route_t,
        crossing: CutCrossing {
            obstacle: cut.generator.clone(),
            direction: if cross(cut_vector, route) > 0.0 {
                CrossingDirection::Positive
            } else {
                CrossingDirection::Negative
            },
        },
    }
}

fn segment_has_clearance(
    graph: &CorridorGraph,
    segment: [Vec2; 2],
    required_radius: f64,
    exempt_obstacles: &[&str],
) -> bool {
    segment_clearance(graph, segment, exempt_obstacles).0 + EPSILON >= required_radius
}

fn segment_clearance(
    graph: &CorridorGraph,
    segment: [Vec2; 2],
    exempt_obstacles: &[&str],
) -> (f64, Option<String>) {
    let board_clearance = [segment[0], segment[1]]
        .into_iter()
        .map(|point| {
            (point.x - graph.board.min.x)
                .min(graph.board.max.x - point.x)
                .min(point.y - graph.board.min.y)
                .min(graph.board.max.y - point.y)
        })
        .fold(f64::INFINITY, f64::min);
    let mut minimum = board_clearance;
    let mut limiting = Some("board boundary".into());
    for obstacle in &graph.obstacles {
        if exempt_obstacles.contains(&obstacle.id.as_str()) {
            continue;
        }
        let clearance = segment_obstacle_clearance(segment, obstacle);
        if clearance < minimum {
            minimum = clearance;
            limiting = Some(format!("component {}", obstacle.id));
        }
    }
    (minimum, limiting)
}

fn segment_obstacle_clearance(segment: [Vec2; 2], obstacle: &CorridorObstacle) -> f64 {
    let midpoint = scale(add(segment[0], segment[1]), 0.5);
    let mut clearance = if point_in_convex_polygon(midpoint, &obstacle.polygon) {
        0.0
    } else {
        f64::INFINITY
    };
    for index in 0..obstacle.polygon.len() {
        clearance = clearance.min(segment_distance(
            segment[0],
            segment[1],
            obstacle.polygon[index],
            obstacle.polygon[(index + 1) % obstacle.polygon.len()],
        ));
    }
    clearance
}

fn visit_repair_obstacle_bvh(
    graph: &CorridorGraph,
    spatial_index: &RepairVisibilitySpatialIndex,
    node_index: usize,
    segment: [Vec2; 2],
    exempt_obstacles: &[&str],
    minimum: &mut f64,
    limiting_obstacle: &mut Option<usize>,
    exact_obstacle_tests: &mut usize,
) {
    let node = &spatial_index.obstacles.nodes[node_index];
    if node.bounds.segment_distance_lower_bound(segment) > *minimum {
        return;
    }
    match node.kind {
        RepairBvhNodeKind::Leaf { start, len } => {
            for &obstacle_index in
                &spatial_index.obstacles.primitives[start..start.saturating_add(len)]
            {
                let obstacle = &graph.obstacles[obstacle_index];
                if exempt_obstacles.contains(&obstacle.id.as_str()) {
                    continue;
                }
                *exact_obstacle_tests += 1;
                let clearance = segment_obstacle_clearance(segment, obstacle);
                let earlier_equal_obstacle = clearance == *minimum
                    && limiting_obstacle
                        .as_ref()
                        .is_some_and(|current| obstacle_index < *current);
                if clearance < *minimum || earlier_equal_obstacle {
                    *minimum = clearance;
                    *limiting_obstacle = Some(obstacle_index);
                }
            }
        }
        RepairBvhNodeKind::Branch { left, right } => {
            let left_distance = spatial_index.obstacles.nodes[left]
                .bounds
                .segment_distance_lower_bound(segment);
            let right_distance = spatial_index.obstacles.nodes[right]
                .bounds
                .segment_distance_lower_bound(segment);
            let (first, second) = if left_distance.total_cmp(&right_distance) != Ordering::Greater {
                (left, right)
            } else {
                (right, left)
            };
            visit_repair_obstacle_bvh(
                graph,
                spatial_index,
                first,
                segment,
                exempt_obstacles,
                minimum,
                limiting_obstacle,
                exact_obstacle_tests,
            );
            visit_repair_obstacle_bvh(
                graph,
                spatial_index,
                second,
                segment,
                exempt_obstacles,
                minimum,
                limiting_obstacle,
                exact_obstacle_tests,
            );
        }
    }
}

fn segment_clearance_indexed(
    graph: &CorridorGraph,
    spatial_index: &RepairVisibilitySpatialIndex,
    segment: [Vec2; 2],
    exempt_obstacles: &[&str],
) -> (f64, Option<String>) {
    let mut exact_obstacle_tests = 0;
    segment_clearance_indexed_with_work(
        graph,
        spatial_index,
        segment,
        exempt_obstacles,
        &mut exact_obstacle_tests,
    )
}

fn segment_clearance_indexed_with_work(
    graph: &CorridorGraph,
    spatial_index: &RepairVisibilitySpatialIndex,
    segment: [Vec2; 2],
    exempt_obstacles: &[&str],
    exact_obstacle_tests: &mut usize,
) -> (f64, Option<String>) {
    *exact_obstacle_tests = 0;
    let valid_segment = segment
        .iter()
        .all(|point| point.x.is_finite() && point.y.is_finite());
    let valid_board = graph.board.min.x.is_finite()
        && graph.board.min.y.is_finite()
        && graph.board.max.x.is_finite()
        && graph.board.max.y.is_finite()
        && graph.board.min.x <= graph.board.max.x
        && graph.board.min.y <= graph.board.max.y;
    if !valid_segment || !valid_board || !spatial_index.obstacle_geometry_valid {
        return (
            f64::NEG_INFINITY,
            Some("invalid non-finite repair geometry".into()),
        );
    }
    let mut minimum = [segment[0], segment[1]]
        .into_iter()
        .map(|point| {
            (point.x - graph.board.min.x)
                .min(graph.board.max.x - point.x)
                .min(point.y - graph.board.min.y)
                .min(graph.board.max.y - point.y)
        })
        .fold(f64::INFINITY, f64::min);
    let mut limiting_obstacle = None;
    if let Some(root) = spatial_index.obstacles.root {
        visit_repair_obstacle_bvh(
            graph,
            spatial_index,
            root,
            segment,
            exempt_obstacles,
            &mut minimum,
            &mut limiting_obstacle,
            exact_obstacle_tests,
        );
    }
    let limiting = limiting_obstacle.map_or_else(
        || Some("board boundary".into()),
        |index| Some(format!("component {}", graph.obstacles[index].id)),
    );
    (minimum, limiting)
}

/// Certify one exact polyline against the board and the graph's polygonal
/// obstacles at `required_radius`. Connected terminal obstacles retain the
/// same whole-route escape exemption used by ordinary corridor embedding.
///
/// This performs no route search: callers can use it as deterministic,
/// candidate-local evidence that a provisional centerline already has room
/// to expand to its target radius in a selected obstacle set.
pub fn certify_polyline_clearance(
    graph: &CorridorGraph,
    polyline: &[Vec2],
    required_radius: f64,
    from_component: &str,
    to_component: &str,
) -> ClearanceCertificate {
    let mut minimum_clearance = f64::INFINITY;
    let mut failures = Vec::new();
    // Connected terminal pads and the route are one copper object. A legal
    // escape may follow, wrap around, or re-enter its own pad away from the
    // first/last polyline segment, so both endpoint obstacles are exempt along
    // the whole witness. The independent exact validator uses the same rule.
    let exempt = route_endpoint_exemptions(from_component, to_component);
    for (index, segment) in polyline.windows(2).enumerate() {
        let (clearance, limiting) = segment_clearance(graph, [segment[0], segment[1]], &exempt);
        minimum_clearance = minimum_clearance.min(clearance);
        if clearance + EPSILON < required_radius {
            failures.push(format!(
                "segment {index} has clearance {clearance:.4}, requires {required_radius:.4}; limited by {}",
                limiting.unwrap_or_else(|| "unknown geometry".into())
            ));
        }
    }
    ClearanceCertificate {
        model: ClearanceModel::ExactPolylineAgainstPolygonalObstacles,
        required_radius,
        certified: failures.is_empty(),
        minimum_clearance,
        failures,
    }
}

fn certify_polyline_clearance_indexed(
    graph: &CorridorGraph,
    spatial_index: &RepairVisibilitySpatialIndex,
    polyline: &[Vec2],
    required_radius: f64,
    from_component: &str,
    to_component: &str,
) -> ClearanceCertificate {
    let mut minimum_clearance = f64::INFINITY;
    let mut failures = Vec::new();
    let exempt = route_endpoint_exemptions(from_component, to_component);
    for (index, segment) in polyline.windows(2).enumerate() {
        let (clearance, limiting) =
            segment_clearance_indexed(graph, spatial_index, [segment[0], segment[1]], &exempt);
        minimum_clearance = minimum_clearance.min(clearance);
        if clearance + EPSILON < required_radius {
            failures.push(format!(
                "segment {index} has clearance {clearance:.4}, requires {required_radius:.4}; limited by {}",
                limiting.unwrap_or_else(|| "unknown geometry".into())
            ));
        }
    }
    ClearanceCertificate {
        model: ClearanceModel::ExactPolylineAgainstPolygonalObstacles,
        required_radius,
        certified: failures.is_empty(),
        minimum_clearance,
        failures,
    }
}

fn point_from_spade(point: Point2<f64>) -> Vec2 {
    Vec2::new(point.x, point.y)
}

fn triangle_center(triangle: [Vec2; 3]) -> Vec2 {
    scale(add(add(triangle[0], triangle[1]), triangle[2]), 1.0 / 3.0)
}

fn point_in_convex_polygon(point: Vec2, polygon: &[Vec2]) -> bool {
    let mut sign = 0.0;
    for index in 0..polygon.len() {
        let side = crate::geometry::cross(
            sub(polygon[(index + 1) % polygon.len()], polygon[index]),
            sub(point, polygon[index]),
        );
        if side.abs() <= EPSILON {
            continue;
        }
        if sign == 0.0 {
            sign = side;
        } else if sign * side < 0.0 {
            return false;
        }
    }
    true
}

fn point_in_triangle(point: Vec2, triangle: [Vec2; 3]) -> bool {
    let mut sign = 0.0;
    for index in 0..3 {
        let side = crate::geometry::cross(
            sub(triangle[(index + 1) % 3], triangle[index]),
            sub(point, triangle[index]),
        );
        if side.abs() <= EPSILON {
            continue;
        }
        if sign == 0.0 {
            sign = side;
        } else if sign * side < 0.0 {
            return false;
        }
    }
    true
}

fn terminal_cell(graph: &CorridorGraph, terminal: Vec2, toward: Vec2) -> Option<usize> {
    let direction = normalized_or(sub(toward, terminal), Vec2::new(1.0, 0.0));
    graph
        .cells
        .iter()
        .filter(|cell| point_in_triangle(terminal, cell.triangle))
        .max_by(|first, second| {
            let first_direction = normalized_or(sub(first.center, terminal), direction);
            let second_direction = normalized_or(sub(second.center, terminal), direction);
            dot(first_direction, direction).total_cmp(&dot(second_direction, direction))
        })
        .map(|cell| cell.id)
        .or_else(|| {
            // Pad-centered terminals are not inside any free triangle. Choose
            // the first triangle reached along the actual trace direction,
            // rather than an arbitrary nearest triangle on the pad boundary.
            graph
                .cells
                .iter()
                .filter_map(|cell| {
                    segment_triangle_interval([terminal, toward], cell.triangle)
                        .filter(|(start, end)| end - start > EPSILON)
                        .map(|(start, _)| (start, cell))
                })
                .min_by(|(first_start, first), (second_start, second)| {
                    first_start
                        .total_cmp(second_start)
                        .then_with(|| {
                            length(sub(first.center, terminal))
                                .total_cmp(&length(sub(second.center, terminal)))
                        })
                        .then_with(|| first.id.cmp(&second.id))
                })
                .map(|(_, cell)| cell.id)
        })
        .or_else(|| {
            graph
                .cells
                .iter()
                .min_by(|first, second| {
                    length(sub(first.center, terminal))
                        .total_cmp(&length(sub(second.center, terminal)))
                })
                .map(|cell| cell.id)
        })
}

#[derive(Clone, Debug)]
struct TerminalProjection {
    evidence: RouteTerminalProjectionEvidence,
    gap_anchor: GapAnchor,
}

fn terminal_projection(graph: &CorridorGraph, terminal: Vec2, toward: Vec2) -> TerminalProjection {
    let selected = terminal_cell(graph, terminal, toward);
    let mut direct_candidates = graph
        .cells
        .iter()
        .filter(|cell| point_in_triangle(terminal, cell.triangle))
        .map(|cell| cell.id)
        .collect::<Vec<_>>();
    let direct_selection = if !direct_candidates.is_empty() {
        DirectTerminalSelection::ContainsTerminal
    } else {
        direct_candidates = graph
            .cells
            .iter()
            .filter(|cell| {
                segment_triangle_interval([terminal, toward], cell.triangle)
                    .is_some_and(|(start, end)| end - start > EPSILON)
            })
            .map(|cell| cell.id)
            .collect();
        if !direct_candidates.is_empty() {
            DirectTerminalSelection::FirstPositiveInterval
        } else if let Some(selected) = selected {
            direct_candidates.push(selected);
            DirectTerminalSelection::NearestFallback
        } else {
            DirectTerminalSelection::Missing
        }
    };
    direct_candidates.sort_unstable();
    direct_candidates.dedup();
    let direct_candidate_count = direct_candidates.len();
    direct_candidates.truncate(MAX_TERMINAL_PROJECTION_CANDIDATES);

    let gap = terminal_gap_anchor_projection(graph, terminal, toward);
    TerminalProjection {
        evidence: RouteTerminalProjectionEvidence {
            direct_candidate_cells: direct_candidates,
            direct_candidate_count,
            direct_selected_cell: selected,
            direct_selection,
            gap_anchor_candidate_cells: gap.candidate_cells,
            gap_anchor_candidate_count: gap.candidate_count,
            gap_anchor_intervals: gap.intervals,
            gap_anchor_regions: gap.regions,
            gap_anchor_region_count: gap.region_count,
            gap_anchor_selected_cell: match gap.anchor {
                GapAnchor::Unique(cell) => Some(cell),
                GapAnchor::NoGapKeepLegacy | GapAnchor::Missing | GapAnchor::AmbiguousRegions => {
                    None
                }
            },
            gap_anchor_outcome: match gap.anchor {
                GapAnchor::NoGapKeepLegacy => GapAnchorOutcome::KeepDirectNoGap,
                GapAnchor::Unique(_) => GapAnchorOutcome::Unique,
                GapAnchor::Missing => GapAnchorOutcome::Missing,
                GapAnchor::AmbiguousRegions => GapAnchorOutcome::AmbiguousRegions,
            },
        },
        gap_anchor: gap.anchor,
    }
}

/// Retry-only endpoint anchor for a certified witness whose ordinary
/// disposable-mesh projection failed. It skips numerical point touches and an
/// endpoint-exempt forbidden-space gap, but refuses to choose between semantic
/// regions at the first positive-measure free-space interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GapAnchor {
    NoGapKeepLegacy,
    Unique(usize),
    Missing,
    AmbiguousRegions,
}

#[derive(Clone, Debug)]
struct GapAnchorProjection {
    anchor: GapAnchor,
    candidate_cells: Vec<usize>,
    candidate_count: usize,
    intervals: Vec<TerminalIntervalEvidence>,
    regions: Vec<usize>,
    region_count: usize,
}

#[cfg(test)]
fn terminal_cell_after_free_space_gap(
    graph: &CorridorGraph,
    terminal: Vec2,
    toward: Vec2,
) -> GapAnchor {
    terminal_gap_anchor_projection(graph, terminal, toward).anchor
}

fn terminal_gap_anchor_projection(
    graph: &CorridorGraph,
    terminal: Vec2,
    toward: Vec2,
) -> GapAnchorProjection {
    let intervals = graph
        .cells
        .iter()
        .filter_map(|cell| {
            segment_triangle_interval([terminal, toward], cell.triangle)
                .filter(|(start, end)| end - start > TERMINAL_INTERVAL_PARAMETER_EPSILON)
                .map(|(start, end)| (start, end - start, cell))
        })
        .collect::<Vec<_>>();
    if intervals
        .iter()
        .any(|(start, _, _)| *start <= TERMINAL_INTERVAL_PARAMETER_EPSILON)
    {
        let immediate = intervals
            .iter()
            .filter(|(start, _, _)| *start <= TERMINAL_INTERVAL_PARAMETER_EPSILON)
            .map(|(start, span, cell)| TerminalIntervalEvidence {
                cell: cell.id,
                start_parameter: *start,
                span_parameter: *span,
                semantic_region: graph.semantic.raw_cell_to_region.get(cell.id).copied(),
            })
            .collect::<Vec<_>>();
        return gap_anchor_projection(GapAnchor::NoGapKeepLegacy, immediate);
    }
    let Some(earliest_start) = intervals
        .iter()
        .map(|(start, _, _)| *start)
        .min_by(f64::total_cmp)
    else {
        return gap_anchor_projection(GapAnchor::Missing, Vec::new());
    };
    let earliest = intervals
        .iter()
        .filter(|(start, _, _)| *start <= earliest_start + TERMINAL_INTERVAL_PARAMETER_EPSILON)
        .collect::<Vec<_>>();
    let region_set = earliest
        .iter()
        .map(|(_, _, cell)| graph.semantic.raw_cell_to_region.get(cell.id).copied())
        .collect::<Option<BTreeSet<_>>>();
    let candidate_intervals = earliest
        .iter()
        .map(|(start, span, cell)| TerminalIntervalEvidence {
            cell: cell.id,
            start_parameter: *start,
            span_parameter: *span,
            semantic_region: graph.semantic.raw_cell_to_region.get(cell.id).copied(),
        })
        .collect::<Vec<_>>();
    let Some(regions) = region_set else {
        return gap_anchor_projection(GapAnchor::Missing, candidate_intervals);
    };
    if regions.len() != 1 {
        return gap_anchor_projection(GapAnchor::AmbiguousRegions, candidate_intervals);
    }
    let anchor = earliest
        .into_iter()
        .min_by(
            |(first_start, first_span, first), (second_start, second_span, second)| {
                first_start
                    .total_cmp(second_start)
                    .then_with(|| second_span.total_cmp(first_span))
                    .then_with(|| first.id.cmp(&second.id))
            },
        )
        .map(|(_, _, cell)| cell.id)
        .map_or(GapAnchor::Missing, GapAnchor::Unique);
    gap_anchor_projection(anchor, candidate_intervals)
}

fn gap_anchor_projection(
    anchor: GapAnchor,
    mut intervals: Vec<TerminalIntervalEvidence>,
) -> GapAnchorProjection {
    intervals.sort_by(|first, second| {
        first
            .start_parameter
            .total_cmp(&second.start_parameter)
            .then_with(|| second.span_parameter.total_cmp(&first.span_parameter))
            .then_with(|| first.cell.cmp(&second.cell))
    });
    let mut candidate_cells = intervals
        .iter()
        .map(|interval| interval.cell)
        .collect::<Vec<_>>();
    candidate_cells.sort_unstable();
    candidate_cells.dedup();
    let mut regions = intervals
        .iter()
        .filter_map(|interval| interval.semantic_region)
        .collect::<Vec<_>>();
    regions.sort_unstable();
    regions.dedup();
    let candidate_count = candidate_cells.len();
    let region_count = regions.len();
    candidate_cells.truncate(MAX_TERMINAL_PROJECTION_CANDIDATES);
    regions.truncate(MAX_TERMINAL_PROJECTION_CANDIDATES);
    intervals.truncate(MAX_TERMINAL_PROJECTION_CANDIDATES);
    GapAnchorProjection {
        anchor,
        candidate_cells,
        candidate_count,
        intervals,
        regions,
        region_count,
    }
}

fn point_polyline_distance(point: Vec2, polyline: &[Vec2]) -> f64 {
    polyline
        .windows(2)
        .map(|segment| point_segment_distance(point, segment[0], segment[1]))
        .fold(f64::INFINITY, f64::min)
}

fn point_segment_distance(point: Vec2, from: Vec2, to: Vec2) -> f64 {
    let delta = sub(to, from);
    let denominator = dot(delta, delta);
    if denominator <= EPSILON {
        return length(sub(point, from));
    }
    let t = (dot(sub(point, from), delta) / denominator).clamp(0.0, 1.0);
    length(sub(point, add(from, scale(delta, t))))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SearchKey {
    cell: usize,
    word: HomotopyWord,
}

#[derive(Clone)]
struct QueueState {
    cost: f64,
    key: SearchKey,
}

impl PartialEq for QueueState {
    fn eq(&self, other: &Self) -> bool {
        self.cost.total_cmp(&other.cost) == Ordering::Equal && self.key == other.key
    }
}

impl Eq for QueueState {}

impl PartialOrd for QueueState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueueState {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.key.cell.cmp(&self.key.cell))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct CanonicalTestEdge {
        first: usize,
        second: usize,
        length: f64,
        word: HomotopyWord,
        penalty: f64,
    }

    #[derive(Clone, Debug)]
    struct CanonicalTestFamily {
        word: HomotopyWord,
        path: Vec<usize>,
        hops: usize,
        geometric_length: f64,
        search_cost: f64,
    }

    struct CanonicalTestSearch {
        families: Vec<CanonicalTestFamily>,
        work: RouteRepairSearchWork,
        termination: AlternativeSearchTermination,
        certified: bool,
    }

    fn test_crossing(obstacle: &str, direction: CrossingDirection) -> HomotopyWord {
        HomotopyWord::reduced([CutCrossing {
            obstacle: obstacle.into(),
            direction,
        }])
    }

    fn compare_test_goals(first: &CanonicalTestFamily, second: &CanonicalTestFamily) -> Ordering {
        first
            .search_cost
            .total_cmp(&second.search_cost)
            .then_with(|| compare_homotopy_words(&first.word, &second.word))
            .then_with(|| first.hops.cmp(&second.hops))
            .then_with(|| first.path.cmp(&second.path))
    }

    fn compare_test_final(first: &CanonicalTestFamily, second: &CanonicalTestFamily) -> Ordering {
        first
            .geometric_length
            .total_cmp(&second.geometric_length)
            .then_with(|| {
                first
                    .word
                    .crossings()
                    .len()
                    .cmp(&second.word.crossings().len())
            })
            .then_with(|| compare_homotopy_words(&first.word, &second.word))
            .then_with(|| first.hops.cmp(&second.hops))
            .then_with(|| first.path.cmp(&second.path))
    }

    fn canonical_test_search(
        positions: &[Vec2],
        edges: &[CanonicalTestEdge],
        heuristic_mode: RepairHeuristic,
        family_limit: usize,
        maximum_word_length: usize,
        target_word: Option<&HomotopyWord>,
        settled_state_budget: usize,
    ) -> CanonicalTestSearch {
        let nodes = positions
            .iter()
            .enumerate()
            .map(|(id, &position)| RepairNode {
                id,
                position,
                kind: if id == 0 {
                    RepairNodeKind::Source
                } else if id == 1 {
                    RepairNodeKind::Target
                } else {
                    RepairNodeKind::ClearanceCorner
                },
                feature: None,
            })
            .collect::<Vec<_>>();
        let mut adjacency = vec![Vec::<VisibilityEdge>::new(); nodes.len()];
        let mut lookup = HashMap::<(usize, usize), (usize, HomotopyWord)>::new();
        for (id, edge) in edges.iter().enumerate() {
            adjacency[edge.first].push(VisibilityEdge {
                to: edge.second,
                length: edge.length,
                word: edge.word.clone(),
                reference_penalty: Some(edge.penalty),
            });
            adjacency[edge.second].push(VisibilityEdge {
                to: edge.first,
                length: edge.length,
                word: edge.word.reversed(),
                reference_penalty: Some(edge.penalty),
            });
            lookup.insert((edge.first, edge.second), (id, edge.word.clone()));
            lookup.insert((edge.second, edge.first), (id, edge.word.reversed()));
        }
        for adjacent in &mut adjacency {
            adjacent.sort_by_key(|edge| edge.to);
        }
        let mut work = empty_route_repair_search_work(family_limit);
        work.budget_limit = settled_state_budget;
        let heuristic = repair_heuristic(&nodes, &adjacency, heuristic_mode, &mut work);
        let source_key = RepairSearchKey {
            node: 0,
            word: HomotopyWord::empty(),
        };
        let mut arena = RepairPathArena::new(0);
        let mut labels = HashMap::from([(
            source_key.clone(),
            RepairSearchLabel {
                cost: 0.0,
                hops: 0,
                path: 0,
                generation: 0,
            },
        )]);
        let mut heap = BinaryHeap::new();
        heap.push(RepairQueueState {
            estimated_total: heuristic[0],
            cost: 0.0,
            hops: 0,
            generation: 0,
            key: source_key,
        });
        let mut generation = 0u64;
        let mut goals = HashMap::<HomotopyWord, CanonicalTestFamily>::new();
        let mut cutoff = None::<f64>;
        let mut threshold_started = false;
        let mut termination = AlternativeSearchTermination::SearchExhausted;
        while let Some(next) = heap.peek() {
            if cutoff.is_some_and(|boundary| {
                next.estimated_total.total_cmp(&boundary) == Ordering::Greater
            }) {
                work.threshold_complete = true;
                termination = AlternativeSearchTermination::FamilyLimitReached;
                break;
            }
            let queued = heap.pop().unwrap();
            work.heap_pops += 1;
            if threshold_started {
                work.threshold_extra_pops += 1;
            }
            let Some(label) = labels.get(&queued.key).copied() else {
                work.stale_pops += 1;
                continue;
            };
            if label.generation != queued.generation {
                work.stale_pops += 1;
                continue;
            }
            if work.settled_states >= settled_state_budget {
                termination = AlternativeSearchTermination::StateLimitReached;
                break;
            }
            work.settled_states += 1;
            work.budget_consumed = work.settled_states;
            if queued.key.node == 1 && target_word.is_none_or(|target| *target == queued.key.word) {
                work.goals_settled += 1;
                let path = arena.materialize(label.path);
                work.output_materialized_path_elements += path.len();
                let mut replay_word = HomotopyWord::empty();
                let mut replay_cost = 0.0;
                let mut geometric_length = 0.0;
                let mut replay_valid = true;
                for pair in path.windows(2) {
                    let Some((edge_id, addition)) = lookup.get(&(pair[0], pair[1])) else {
                        replay_valid = false;
                        break;
                    };
                    let edge = &edges[*edge_id];
                    replay_word = replay_word.extended(addition.crossings().iter().cloned());
                    replay_cost = replay_cost + edge.length + edge.penalty;
                    geometric_length += edge.length;
                }
                replay_valid &= replay_word == queued.key.word
                    && replay_cost.to_bits() == label.cost.to_bits()
                    && target_word.is_none_or(|target| *target == replay_word);
                if replay_valid {
                    goals.insert(
                        queued.key.word.clone(),
                        CanonicalTestFamily {
                            word: queued.key.word,
                            path,
                            hops: label.hops,
                            geometric_length,
                            search_cost: label.cost,
                        },
                    );
                    if goals.len() >= family_limit {
                        let mut ordered = goals.values().collect::<Vec<_>>();
                        ordered.sort_by(|first, second| compare_test_goals(first, second));
                        cutoff = Some(ordered[family_limit - 1].search_cost);
                        threshold_started = true;
                    }
                }
                continue;
            }
            work.expanded_states += 1;
            for edge in &adjacency[queued.key.node] {
                work.relaxed_edges += 1;
                let next_word = queued
                    .key
                    .word
                    .extended(edge.word.crossings().iter().cloned());
                if next_word.crossings().len() > maximum_word_length {
                    continue;
                }
                let next_key = RepairSearchKey {
                    node: edge.to,
                    word: next_word,
                };
                let penalty = edge
                    .reference_penalty
                    .expect("test edge carries its penalty");
                let Some(next_cost) = canonical_nonnegative(label.cost + edge.length + penalty)
                else {
                    continue;
                };
                let Some(estimated_total) = canonical_nonnegative(next_cost + heuristic[edge.to])
                else {
                    continue;
                };
                let next_hops = label.hops + 1;
                if !canonical_label_wins(
                    labels.get(&next_key).copied(),
                    next_cost,
                    next_hops,
                    label.path,
                    edge.to,
                    &mut arena,
                    &mut work,
                ) {
                    continue;
                }
                let path = arena.accept(label.path, edge.to);
                generation += 1;
                labels.insert(
                    next_key.clone(),
                    RepairSearchLabel {
                        cost: next_cost,
                        hops: next_hops,
                        path,
                        generation,
                    },
                );
                work.accepted_labels += 1;
                work.maximum_path_depth = work.maximum_path_depth.max(next_hops + 1);
                heap.push(RepairQueueState {
                    estimated_total,
                    cost: next_cost,
                    hops: next_hops,
                    generation,
                    key: next_key,
                });
            }
        }
        if termination == AlternativeSearchTermination::SearchExhausted && cutoff.is_some() {
            work.threshold_complete = true;
        }
        work.path_arena_nodes = arena.nodes.len();
        if let Some(boundary) = cutoff {
            work.boundary_goals = goals
                .values()
                .filter(|goal| goal.search_cost.total_cmp(&boundary) == Ordering::Equal)
                .count();
        }
        let certified = goals
            .values()
            .all(|goal| goal.path.len() == goal.hops + 1 && goal.search_cost.is_finite());
        let mut families = goals.into_values().collect::<Vec<_>>();
        families.sort_by(compare_test_goals);
        families.truncate(family_limit);
        families.sort_by(compare_test_final);
        CanonicalTestSearch {
            families,
            work,
            termination,
            certified,
        }
    }

    fn assert_canonical_test_search_equal(
        dijkstra: &CanonicalTestSearch,
        astar: &CanonicalTestSearch,
    ) {
        assert_eq!(dijkstra.certified, astar.certified);
        assert_eq!(dijkstra.families.len(), astar.families.len());
        for (dijkstra, astar) in dijkstra.families.iter().zip(&astar.families) {
            assert_eq!(dijkstra.word, astar.word);
            assert_eq!(dijkstra.path, astar.path);
            assert_eq!(dijkstra.hops, astar.hops);
            assert_eq!(
                dijkstra.geometric_length.to_bits(),
                astar.geometric_length.to_bits()
            );
            assert_eq!(dijkstra.search_cost.to_bits(), astar.search_cost.to_bits());
        }
        for result in [dijkstra, astar] {
            match result.termination {
                AlternativeSearchTermination::SearchExhausted => {}
                AlternativeSearchTermination::FamilyLimitReached => {
                    assert!(result.work.threshold_complete)
                }
                termination => panic!("unexpected incomplete oracle termination {termination:?}"),
            }
        }
    }

    fn board() -> Rect {
        Rect {
            min: Vec2::ZERO,
            max: Vec2::new(20.0, 10.0),
        }
    }

    fn regular_polygon(center: Vec2, diameter: f64, vertices: usize) -> Vec<Vec2> {
        (0..vertices)
            .map(|index| {
                let angle = std::f64::consts::TAU * index as f64 / vertices as f64;
                add(
                    center,
                    Vec2::new(angle.cos() * diameter * 0.5, angle.sin() * diameter * 0.5),
                )
            })
            .collect()
    }

    fn rectangle_obstacle(id: impl Into<String>, min: Vec2, max: Vec2) -> CorridorObstacle {
        CorridorObstacle {
            id: id.into(),
            polygon: vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)],
        }
    }

    fn assert_sampled_segments_have_no_obstacle_free_coverage_holes(
        graph: &CorridorGraph,
        segments: &[[Vec2; 2]],
    ) {
        for (segment_index, segment) in segments.iter().enumerate() {
            let mut covered = graph
                .cells
                .iter()
                .filter_map(|cell| segment_triangle_interval(*segment, cell.triangle))
                .filter(|(start, end)| end - start > TERMINAL_INTERVAL_PARAMETER_EPSILON)
                .collect::<Vec<_>>();
            covered.sort_by(|first, second| {
                first
                    .0
                    .total_cmp(&second.0)
                    .then_with(|| first.1.total_cmp(&second.1))
            });
            let mut cursor = 0.0_f64;
            for (start, end) in covered {
                if start > cursor + TERMINAL_INTERVAL_PARAMETER_EPSILON {
                    let parameter = (cursor + start) * 0.5;
                    let point = add(
                        scale(segment[0], 1.0 - parameter),
                        scale(segment[1], parameter),
                    );
                    assert!(
                        graph
                            .obstacles
                            .iter()
                            .any(|obstacle| point_in_convex_polygon(point, &obstacle.polygon)),
                        "segment {segment_index} has obstacle-free uncovered interval [{cursor}, {start}] at {point:?}"
                    );
                }
                cursor = cursor.max(end);
            }
            if cursor < 1.0 - TERMINAL_INTERVAL_PARAMETER_EPSILON {
                let parameter = (cursor + 1.0) * 0.5;
                let point = add(
                    scale(segment[0], 1.0 - parameter),
                    scale(segment[1], parameter),
                );
                assert!(
                    graph
                        .obstacles
                        .iter()
                        .any(|obstacle| point_in_convex_polygon(point, &obstacle.polygon)),
                    "segment {segment_index} has obstacle-free uncovered interval [{cursor}, 1] at {point:?}"
                );
            }
        }
    }

    fn segments_cross_interior(first: [Vec2; 2], second: [Vec2; 2]) -> bool {
        let first_delta = sub(first[1], first[0]);
        let second_delta = sub(second[1], second[0]);
        let denominator = cross(first_delta, second_delta);
        if denominator.abs() <= EPSILON {
            return false;
        }
        let offset = sub(second[0], first[0]);
        let first_parameter = cross(offset, second_delta) / denominator;
        let second_parameter = cross(offset, first_delta) / denominator;
        (EPSILON..1.0 - EPSILON).contains(&first_parameter)
            && (EPSILON..1.0 - EPSILON).contains(&second_parameter)
    }

    #[test]
    fn board_without_obstacles_has_a_dual_gate() {
        let graph = build_corridor_graph("top", board(), &[], 3, 7).unwrap();
        assert_eq!(graph.cells.len(), 2);
        assert_eq!(graph.gates.len(), 1);
        assert_eq!(graph.semantic.regions.len(), 1);
        assert!(graph.semantic.passages.is_empty());
        assert_eq!(
            graph.cut_basis.structural_fingerprint,
            "vertical-cut-forest/v2|components=|parents=|anchor-order="
        );
        assert!(
            graph
                .cut_basis
                .fingerprint
                .starts_with("vertical-cut-forest-exact/v2|")
        );
        assert_eq!(graph.placement_revision, 3);
        assert_eq!(graph.revision, 7);
    }

    #[test]
    fn ordinary_visibility_repair_does_not_hit_the_graph_limit() {
        let graph = build_corridor_graph("top", board(), &[], 0, 1).unwrap();
        let reference = [Vec2::new(1.0, 5.0), Vec2::new(19.0, 5.0)];
        let evidence = enumerate_route_class_alternatives(
            &graph,
            RouteEmbeddingRequest {
                net: "normal",
                from_component: "source",
                to_component: "target",
                physical_width: 0.4,
                clearance: 0.2,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
        );

        assert!(!evidence.visibility_graph_limit_reached);
        assert_eq!(
            evidence.termination,
            AlternativeSearchTermination::SearchExhausted
        );
        assert_eq!(
            serde_json::to_value(&evidence).unwrap()["visibility_graph_limit_reached"].as_bool(),
            Some(false)
        );
        let search = &serde_json::to_value(&evidence).unwrap()["search_work"];
        assert_eq!(search["contract"], "canonical_euclidean_astar_v1");
        assert_eq!(search["budget_kind"], "settled_states");
        assert_eq!(
            search["goal_limit"].as_u64(),
            Some(FIRST_K_ROUTE_CLASS_FAMILY_LIMIT as u64)
        );
        assert_eq!(
            search["budget_limit"].as_u64(),
            Some(MAX_REPAIR_SETTLED_STATES as u64)
        );
        assert_eq!(
            search["heap_pops"].as_u64(),
            Some(evidence.explored_states as u64)
        );
    }

    #[test]
    fn node_only_search_exhausts_an_isolated_target_with_at_most_one_settle_per_node() {
        let graph = build_corridor_graph(
            "top",
            board(),
            &[rectangle_obstacle(
                "target-cage",
                Vec2::new(8.0, 3.0),
                Vec2::new(12.0, 7.0),
            )],
            0,
            1,
        )
        .unwrap();
        let reference = [Vec2::new(1.0, 5.0), Vec2::new(10.0, 5.0)];
        let evidence = first_clearance_offset_visibility_path_with_budget(
            &graph,
            RouteEmbeddingRequest {
                net: "isolated-target",
                from_component: "source-terminal",
                to_component: "isolated-target-terminal",
                physical_width: 0.4,
                clearance: 0.2,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
            50_000,
        );

        assert_eq!(
            evidence.search_work.contract,
            RepairSearchContract::NodeOnlyEuclideanAstarV1
        );
        assert_eq!(evidence.search_work.goal_limit, 1);
        assert_eq!(
            evidence.termination,
            AlternativeSearchTermination::SearchExhausted
        );
        assert!(!evidence.state_limit_reached);
        assert!(evidence.alternatives.is_empty());
        assert!(
            evidence.search_work.settled_states <= evidence.nodes.len(),
            "node-only search settled {} states for {} visibility nodes",
            evidence.search_work.settled_states,
            evidence.nodes.len()
        );
    }

    #[test]
    fn node_only_first_witness_is_clearance_and_posthoc_homotopy_certified() {
        let graph = build_corridor_graph(
            "top",
            board(),
            &[rectangle_obstacle(
                "middle",
                Vec2::new(8.0, 2.0),
                Vec2::new(12.0, 8.0),
            )],
            0,
            1,
        )
        .unwrap();
        let reference = [Vec2::new(1.0, 5.0), Vec2::new(19.0, 5.0)];
        let evidence = first_clearance_offset_visibility_path_with_budget(
            &graph,
            RouteEmbeddingRequest {
                net: "first-witness",
                from_component: "source",
                to_component: "target",
                physical_width: 0.4,
                clearance: 0.2,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
            50_000,
        );

        assert_eq!(
            evidence.search_work.contract,
            RepairSearchContract::NodeOnlyEuclideanAstarV1
        );
        assert_eq!(evidence.alternatives.len(), 1);
        assert_eq!(evidence.selected_alternative, Some(0));
        assert!(evidence.search_work.settled_states <= evidence.nodes.len());
        let witness = &evidence.alternatives[0];
        assert!(witness.polyline.len() > 2, "the obstacle requires a detour");
        assert!(witness.minimum_clearance + EPSILON >= 0.4);
        assert_eq!(
            certify_route_homotopy(&witness.polyline, &graph.cut_basis, "source", "target")
                .unwrap(),
            witness.homotopy
        );
    }

    #[test]
    fn configurable_exploratory_family_limit_is_bounded_and_preserves_legacy_k_three() {
        let graph = build_corridor_graph(
            "top",
            board(),
            &[
                rectangle_obstacle("lower", Vec2::new(6.0, 1.0), Vec2::new(9.0, 6.0)),
                rectangle_obstacle("upper", Vec2::new(11.0, 4.0), Vec2::new(14.0, 9.0)),
            ],
            0,
            1,
        )
        .unwrap();
        let reference = [Vec2::new(1.0, 5.0), Vec2::new(19.0, 5.0)];
        let request = RouteEmbeddingRequest {
            net: "configurable-family-limit",
            from_component: "source",
            to_component: "target",
            physical_width: 0.5,
            clearance: 0.2,
            from: reference[0],
            from_toward: reference[1],
            to: reference[1],
            to_toward: reference[0],
            reference: &reference,
            target_homotopy: None,
            persistent_basis_matches: false,
        };

        let legacy = enumerate_route_class_alternatives(&graph, request.clone());
        let explicit_three = enumerate_route_class_alternatives_with_limit(
            &graph,
            request.clone(),
            FIRST_K_ROUTE_CLASS_FAMILY_LIMIT,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_vec(&legacy).unwrap(),
            serde_json::to_vec(&explicit_three).unwrap(),
            "the configurable API changed the legacy K=3 contract"
        );
        assert_eq!(explicit_three.search_work.goal_limit, 3);
        let explicit_default_budget = enumerate_route_class_alternatives_with_limit_and_budget(
            &graph,
            request.clone(),
            FIRST_K_ROUTE_CLASS_FAMILY_LIMIT,
            DEFAULT_ROUTE_REPAIR_SETTLED_STATE_BUDGET,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_vec(&explicit_three).unwrap(),
            serde_json::to_vec(&explicit_default_budget).unwrap(),
            "the explicit default work budget changed serialized evidence"
        );

        for budget in [0, 1] {
            let bounded = enumerate_route_class_alternatives_with_limit_and_budget(
                &graph,
                request.clone(),
                1,
                budget,
            )
            .unwrap();
            assert_eq!(
                bounded.termination,
                AlternativeSearchTermination::StateLimitReached
            );
            assert!(bounded.state_limit_reached);
            assert_eq!(bounded.search_work.budget_limit, budget);
            assert_eq!(bounded.search_work.budget_consumed, budget);
            assert_eq!(bounded.search_work.settled_states, budget);
            assert!(bounded.alternatives.is_empty());
        }

        let first_six =
            enumerate_route_class_alternatives_with_limit(&graph, request.clone(), 6).unwrap();
        let second_six =
            enumerate_route_class_alternatives_with_limit(&graph, request.clone(), 6).unwrap();
        assert_eq!(first_six.search_work.goal_limit, 6);
        assert_eq!(first_six.alternatives.len(), 6);
        assert_eq!(
            serde_json::to_vec(&first_six).unwrap(),
            serde_json::to_vec(&second_six).unwrap(),
            "K=6 evidence must remain deterministic"
        );
        assert!(first_six.search_work.threshold_complete);
        assert_eq!(
            first_six.termination,
            AlternativeSearchTermination::FamilyLimitReached
        );

        for invalid in [0, MAX_EXPLORATORY_ROUTE_CLASS_FAMILY_LIMIT + 1] {
            let error =
                enumerate_route_class_alternatives_with_limit(&graph, request.clone(), invalid)
                    .unwrap_err();
            assert_eq!(error.requested, invalid);
            assert_eq!(error.minimum, 1);
            assert_eq!(error.maximum, MAX_EXPLORATORY_ROUTE_CLASS_FAMILY_LIMIT);
            assert!(error.to_string().contains("supported range"));
        }
    }

    #[test]
    fn threshold_completion_canonicalizes_the_equal_k_boundary() {
        let positions = [
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(1.9, 0.0),
            Vec2::new(1.5, 0.0),
            Vec2::new(1.0, 0.0),
        ];
        let edges = [
            CanonicalTestEdge {
                first: 0,
                second: 2,
                length: 1.9,
                word: test_crossing("A", CrossingDirection::Positive),
                penalty: 0.0,
            },
            CanonicalTestEdge {
                first: 2,
                second: 1,
                length: 0.1,
                word: HomotopyWord::empty(),
                penalty: 0.0,
            },
            CanonicalTestEdge {
                first: 0,
                second: 3,
                length: 1.5,
                word: test_crossing("B", CrossingDirection::Positive),
                penalty: 0.0,
            },
            CanonicalTestEdge {
                first: 3,
                second: 1,
                length: 0.5,
                word: HomotopyWord::empty(),
                penalty: 0.0,
            },
            CanonicalTestEdge {
                first: 0,
                second: 4,
                length: 1.0,
                word: test_crossing("C", CrossingDirection::Positive),
                penalty: 0.0,
            },
            CanonicalTestEdge {
                first: 4,
                second: 1,
                length: 1.0,
                word: HomotopyWord::empty(),
                penalty: 0.0,
            },
        ];
        let dijkstra = canonical_test_search(
            &positions,
            &edges,
            RepairHeuristic::Zero,
            2,
            4,
            None,
            MAX_REPAIR_SETTLED_STATES,
        );
        let astar = canonical_test_search(
            &positions,
            &edges,
            RepairHeuristic::Euclidean,
            2,
            4,
            None,
            MAX_REPAIR_SETTLED_STATES,
        );

        assert_canonical_test_search_equal(&dijkstra, &astar);
        assert!(astar.work.threshold_complete);
        assert_eq!(astar.work.boundary_goals, 3);
        assert_eq!(
            astar
                .families
                .iter()
                .map(|family| family.word.crossings()[0].obstacle.as_str())
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
    }

    #[test]
    fn canonical_state_labels_fix_same_state_and_sub_epsilon_cost_counterexamples() {
        let empty = HomotopyWord::empty();
        let same_state_positions = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.5, 0.0),
            Vec2::new(-0.2, 0.0),
        ];
        let same_state_edges =
            [(0, 2, 1.0), (2, 1, 1.0), (0, 4, 0.2), (4, 1, 1.8)].map(|(first, second, length)| {
                CanonicalTestEdge {
                    first,
                    second,
                    length,
                    word: HomotopyWord::empty(),
                    penalty: 0.0,
                }
            });
        let dijkstra = canonical_test_search(
            &same_state_positions,
            &same_state_edges,
            RepairHeuristic::Zero,
            1,
            2,
            Some(&empty),
            MAX_REPAIR_SETTLED_STATES,
        );
        let astar = canonical_test_search(
            &same_state_positions,
            &same_state_edges,
            RepairHeuristic::Euclidean,
            1,
            2,
            Some(&empty),
            MAX_REPAIR_SETTLED_STATES,
        );
        assert_canonical_test_search_equal(&dijkstra, &astar);
        assert_eq!(astar.families[0].path, [0, 2, 1]);
        assert!(astar.work.exact_cost_hop_path_comparisons > 0);
        assert!(astar.work.materialized_path_elements > 0);

        let epsilon_positions = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(-0.2, 0.0),
        ];
        let epsilon_edges = [(0, 2, 1.0), (2, 1, 1.0000000005), (0, 3, 0.2), (3, 1, 1.8)].map(
            |(first, second, length)| CanonicalTestEdge {
                first,
                second,
                length,
                word: HomotopyWord::empty(),
                penalty: 0.0,
            },
        );
        let epsilon_dijkstra = canonical_test_search(
            &epsilon_positions,
            &epsilon_edges,
            RepairHeuristic::Zero,
            1,
            2,
            Some(&empty),
            MAX_REPAIR_SETTLED_STATES,
        );
        let epsilon_astar = canonical_test_search(
            &epsilon_positions,
            &epsilon_edges,
            RepairHeuristic::Euclidean,
            1,
            2,
            Some(&empty),
            MAX_REPAIR_SETTLED_STATES,
        );
        assert_canonical_test_search_equal(&epsilon_dijkstra, &epsilon_astar);
        assert_eq!(epsilon_astar.families[0].path, [0, 3, 1]);
        assert_eq!(
            epsilon_astar.families[0].search_cost.to_bits(),
            2.0f64.to_bits()
        );
    }

    #[test]
    fn canonical_labels_are_well_founded_on_a_zero_cost_word_cycle() {
        let positions = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 0.0),
        ];
        let edges = [
            CanonicalTestEdge {
                first: 0,
                second: 2,
                length: 0.0,
                word: test_crossing("A", CrossingDirection::Positive),
                penalty: 0.0,
            },
            CanonicalTestEdge {
                first: 2,
                second: 3,
                length: 0.0,
                word: test_crossing("B", CrossingDirection::Positive),
                penalty: 0.0,
            },
            CanonicalTestEdge {
                first: 3,
                second: 0,
                length: 0.0,
                word: HomotopyWord::empty(),
                penalty: 0.0,
            },
            CanonicalTestEdge {
                first: 0,
                second: 1,
                length: 1.0,
                word: HomotopyWord::empty(),
                penalty: 0.0,
            },
            CanonicalTestEdge {
                first: 2,
                second: 1,
                length: 1.0,
                word: HomotopyWord::empty(),
                penalty: 0.0,
            },
            CanonicalTestEdge {
                first: 3,
                second: 1,
                length: 1.0,
                word: HomotopyWord::empty(),
                penalty: 0.0,
            },
        ];
        let dijkstra = canonical_test_search(
            &positions,
            &edges,
            RepairHeuristic::Zero,
            4,
            4,
            None,
            MAX_REPAIR_SETTLED_STATES,
        );
        let astar = canonical_test_search(
            &positions,
            &edges,
            RepairHeuristic::Euclidean,
            4,
            4,
            None,
            MAX_REPAIR_SETTLED_STATES,
        );
        assert_canonical_test_search_equal(&dijkstra, &astar);
        assert!(astar.certified);
        assert_eq!(astar.families.len(), 4);
        assert!(astar.work.settled_states < 500);
    }

    #[test]
    fn settled_state_budget_is_explicit_and_does_not_count_stale_entries() {
        let positions = [
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(1.0, 0.0),
        ];
        let edges = [
            CanonicalTestEdge {
                first: 0,
                second: 2,
                length: 1.0,
                word: HomotopyWord::empty(),
                penalty: 0.0,
            },
            CanonicalTestEdge {
                first: 2,
                second: 1,
                length: 1.0,
                word: HomotopyWord::empty(),
                penalty: 0.0,
            },
        ];
        let result = canonical_test_search(
            &positions,
            &edges,
            RepairHeuristic::Euclidean,
            1,
            0,
            None,
            1,
        );
        assert_eq!(
            result.termination,
            AlternativeSearchTermination::StateLimitReached
        );
        assert_eq!(
            result.work.budget_kind,
            RepairSearchBudgetKind::SettledStates
        );
        assert_eq!(result.work.budget_limit, 1);
        assert_eq!(result.work.budget_consumed, 1);
        assert_eq!(result.work.settled_states, 1);
        assert!(!result.work.threshold_complete);
    }

    #[test]
    fn eager_reference_penalties_are_bit_identical_to_repeated_production_search() {
        let graph = build_corridor_graph(
            "top",
            board(),
            &[
                rectangle_obstacle("middle-lower", Vec2::new(7.0, 1.5), Vec2::new(10.0, 6.5)),
                rectangle_obstacle("middle-upper", Vec2::new(11.0, 3.5), Vec2::new(14.0, 8.5)),
            ],
            0,
            1,
        )
        .unwrap();
        let source = Vec2::new(1.0, 5.0);
        let target = Vec2::new(19.0, 5.0);
        let references = [
            vec![source, target],
            // Both duplicate pairs are zero-length reference segments.
            vec![
                source,
                source,
                Vec2::new(10.0, 5.0),
                Vec2::new(10.0, 5.0),
                target,
            ],
            // No reference segment at all: every edge receives infinity.
            vec![source],
            // Reference traversal direction is deliberately unrelated to the
            // source-to-target direction used by the visibility search.
            vec![target, Vec2::new(14.0, 8.0), Vec2::new(6.0, 2.0), source],
            (0..=32)
                .map(|index| {
                    let t = index as f64 / 32.0;
                    Vec2::new(
                        source.x + (target.x - source.x) * t,
                        5.0 + ((index * 7) % 11) as f64 * 0.37 - 1.85,
                    )
                })
                .collect::<Vec<_>>(),
            vec![source, Vec2::new(10.0, 5.0 + 1.0e-12), target],
            vec![source, Vec2::new(10.0, 5.0 - 1.0e-12), target],
        ];

        for strategy in [
            RepairVisibilitySearchStrategy::FirstGeometricWitness,
            RepairVisibilitySearchStrategy::ProductRouteFamilies,
        ] {
            for (reference_index, reference) in references.iter().enumerate() {
                let request = RouteEmbeddingRequest {
                    net: "reference-penalty-equivalence",
                    from_component: "source",
                    to_component: "target",
                    physical_width: 0.5,
                    clearance: 0.2,
                    from: source,
                    from_toward: target,
                    to: target,
                    to_toward: source,
                    reference,
                    target_homotopy: None,
                    persistent_basis_matches: false,
                };
                let eager = repair_route_visibility_with_reference_penalties::<true>(
                    &graph,
                    &request,
                    request.physical_width * 0.5 + request.clearance,
                    &HomotopyWord::empty(),
                    strategy,
                );
                let repeated = repair_route_visibility_with_reference_penalties::<false>(
                    &graph,
                    &request,
                    request.physical_width * 0.5 + request.clearance,
                    &HomotopyWord::empty(),
                    strategy,
                );

                assert_eq!(
                    serde_json::to_vec(&eager.evidence).unwrap(),
                    serde_json::to_vec(&repeated.evidence).unwrap(),
                    "ordered family evidence changed for {strategy:?}, reference {reference:?}"
                );
                assert_eq!(eager.ordered_node_paths, repeated.ordered_node_paths);
                assert_eq!(
                    eager.ordered_search_cost_bits,
                    repeated.ordered_search_cost_bits
                );
                assert_eq!(
                    eager
                        .evidence
                        .alternatives
                        .iter()
                        .map(|alternative| alternative.length.to_bits())
                        .collect::<Vec<_>>(),
                    repeated
                        .evidence
                        .alternatives
                        .iter()
                        .map(|alternative| alternative.length.to_bits())
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    eager
                        .evidence
                        .alternatives
                        .iter()
                        .flat_map(|alternative| &alternative.polyline)
                        .flat_map(|point| [point.x.to_bits(), point.y.to_bits()])
                        .collect::<Vec<_>>(),
                    repeated
                        .evidence
                        .alternatives
                        .iter()
                        .flat_map(|alternative| &alternative.polyline)
                        .flat_map(|point| [point.x.to_bits(), point.y.to_bits()])
                        .collect::<Vec<_>>()
                );

                // The first, straight reference is the minimal deterministic
                // witness that product-state search reaches the geometric
                // target under more than one homotopy word. Node-only search
                // deliberately publishes only one geometric witness.
                if strategy == RepairVisibilitySearchStrategy::ProductRouteFamilies
                    && reference_index == 0
                {
                    let target_words = eager
                        .evidence
                        .alternatives
                        .iter()
                        .map(|alternative| &alternative.homotopy)
                        .collect::<Vec<_>>();
                    assert!(
                        target_words.iter().enumerate().any(|(index, word)| {
                            target_words[..index].iter().any(|other| other != word)
                        }),
                        "the product-search regression must reach one geometric target under multiple homotopy words"
                    );
                }
            }
        }
    }

    #[test]
    fn production_visibility_astar_matches_the_canonical_dijkstra_oracle() {
        let graph = build_corridor_graph(
            "top",
            board(),
            &[
                rectangle_obstacle("lower", Vec2::new(6.0, 1.0), Vec2::new(9.0, 6.0)),
                rectangle_obstacle("upper", Vec2::new(11.0, 4.0), Vec2::new(14.0, 9.0)),
            ],
            0,
            1,
        )
        .unwrap();
        let source = Vec2::new(1.0, 5.0);
        let target = Vec2::new(19.0, 5.0);
        let references = [
            vec![source, target],
            vec![source, source, Vec2::new(10.0, 5.0), target, target],
            vec![target, Vec2::new(12.0, 8.0), Vec2::new(7.0, 2.0), source],
            (0..=16)
                .map(|index| {
                    let t = index as f64 / 16.0;
                    Vec2::new(
                        source.x + (target.x - source.x) * t,
                        5.0 + ((index * 5) % 9) as f64 * 0.31 - 1.24,
                    )
                })
                .collect::<Vec<_>>(),
        ];
        for reference in &references {
            let request = RouteEmbeddingRequest {
                net: "canonical-astar-production-oracle",
                from_component: "source",
                to_component: "target",
                physical_width: 0.5,
                clearance: 0.2,
                from: source,
                from_toward: target,
                to: target,
                to_toward: source,
                reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            };
            let radius = request.physical_width * 0.5 + request.clearance;
            let astar = repair_route_visibility_with_search::<true>(
                &graph,
                &request,
                radius,
                &HomotopyWord::empty(),
                RepairHeuristic::Euclidean,
                MAX_REPAIR_SETTLED_STATES,
            );
            let dijkstra = repair_route_visibility_with_search::<true>(
                &graph,
                &request,
                radius,
                &HomotopyWord::empty(),
                RepairHeuristic::Zero,
                MAX_REPAIR_SETTLED_STATES,
            );
            assert_eq!(
                serde_json::to_vec(&astar.evidence.alternatives).unwrap(),
                serde_json::to_vec(&dijkstra.evidence.alternatives).unwrap(),
                "canonical family output changed for reference {reference:?}"
            );
            assert_eq!(astar.ordered_node_paths, dijkstra.ordered_node_paths);
            assert_eq!(
                astar.ordered_search_cost_bits,
                dijkstra.ordered_search_cost_bits
            );
            for result in [&astar, &dijkstra] {
                assert!(!result.evidence.state_limit_reached);
                match result.evidence.termination {
                    AlternativeSearchTermination::SearchExhausted => {
                        assert!(result.evidence.search_complete)
                    }
                    AlternativeSearchTermination::FamilyLimitReached => {
                        assert!(result.evidence.search_work.threshold_complete);
                        assert!(!result.evidence.search_complete);
                    }
                    termination => {
                        panic!("unexpected incomplete oracle termination {termination:?}")
                    }
                }
            }
        }
    }

    #[test]
    fn persistent_exact_word_search_completes_after_one_canonical_goal() {
        let graph = build_corridor_graph(
            "top",
            board(),
            &[
                rectangle_obstacle("left", Vec2::new(6.0, 1.0), Vec2::new(9.0, 6.0)),
                rectangle_obstacle("right", Vec2::new(11.0, 4.0), Vec2::new(14.0, 9.0)),
            ],
            0,
            1,
        )
        .unwrap();
        let reference = [Vec2::new(1.0, 5.0), Vec2::new(19.0, 5.0)];
        let exploratory_request = RouteEmbeddingRequest {
            net: "persistent-k-one",
            from_component: "source",
            to_component: "target",
            physical_width: 0.5,
            clearance: 0.2,
            from: reference[0],
            from_toward: reference[1],
            to: reference[1],
            to_toward: reference[0],
            reference: &reference,
            target_homotopy: None,
            persistent_basis_matches: false,
        };
        let radius = exploratory_request.physical_width * 0.5 + exploratory_request.clearance;
        let exploratory =
            repair_route_visibility(&graph, &exploratory_request, radius, &HomotopyWord::empty());
        let target_word = exploratory.evidence.alternatives[0].homotopy.clone();
        let persistent_request = RouteEmbeddingRequest {
            target_homotopy: Some(&target_word),
            persistent_basis_matches: true,
            ..exploratory_request
        };
        let persistent = repair_route_visibility(&graph, &persistent_request, radius, &target_word);

        assert_eq!(persistent.evidence.alternatives.len(), 1);
        assert_eq!(persistent.evidence.alternatives[0].homotopy, target_word);
        assert_eq!(
            persistent.evidence.termination,
            AlternativeSearchTermination::FamilyLimitReached
        );
        assert!(persistent.evidence.search_work.threshold_complete);
        assert!(!persistent.evidence.search_complete);
        assert_eq!(persistent.evidence.search_work.goal_limit, 1);
        assert_eq!(persistent.evidence.search_work.boundary_goals, 1);
    }

    #[test]
    fn deterministic_small_graph_fuzz_matches_canonical_dijkstra() {
        fn next_random(state: &mut u64) -> u64 {
            *state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            *state
        }

        fn generated_word(value: u64) -> HomotopyWord {
            let crossing = |obstacle: &str, direction| CutCrossing {
                obstacle: obstacle.into(),
                direction,
            };
            match value % 6 {
                0 => HomotopyWord::empty(),
                1 => HomotopyWord::reduced([crossing("A", CrossingDirection::Positive)]),
                2 => HomotopyWord::reduced([crossing("A", CrossingDirection::Negative)]),
                3 => HomotopyWord::reduced([crossing("B", CrossingDirection::Positive)]),
                4 => HomotopyWord::reduced([
                    crossing("A", CrossingDirection::Positive),
                    crossing("A", CrossingDirection::Negative),
                ]),
                _ => HomotopyWord::reduced([
                    crossing("A", CrossingDirection::Positive),
                    crossing("B", CrossingDirection::Negative),
                ]),
            }
        }

        for seed in 0..96u64 {
            let mut random = seed ^ 0xA57A_5A17_D15C_A11E;
            let node_count = 2 + (next_random(&mut random) % 6) as usize;
            let positions = (0..node_count)
                .map(|_| {
                    Vec2::new(
                        (next_random(&mut random) % 7) as f64 - 2.0,
                        (next_random(&mut random) % 7) as f64 - 2.0,
                    )
                })
                .collect::<Vec<_>>();
            let mut edges = Vec::new();
            for first in 0..node_count {
                for second in (first + 1)..node_count {
                    if next_random(&mut random) % 100 >= 42 {
                        continue;
                    }
                    let detour = match next_random(&mut random) % 5 {
                        0 => 2.5e-10,
                        1 => 7.5e-10,
                        2 => 0.25,
                        _ => 0.0,
                    };
                    edges.push(CanonicalTestEdge {
                        first,
                        second,
                        length: length(sub(positions[first], positions[second])) + detour,
                        word: generated_word(next_random(&mut random)),
                        penalty: if seed % 4 == 0 {
                            0.0
                        } else {
                            (next_random(&mut random) % 3) as f64 * 0.05
                        },
                    });
                }
            }
            if edges.is_empty() && seed % 3 != 0 {
                edges.push(CanonicalTestEdge {
                    first: 0,
                    second: 1,
                    length: length(sub(positions[0], positions[1])),
                    word: generated_word(next_random(&mut random)),
                    penalty: 0.0,
                });
            }
            let target_word = match seed % 3 {
                0 => None,
                1 => Some(HomotopyWord::empty()),
                _ => Some(test_crossing("UNREACHABLE", CrossingDirection::Positive)),
            };
            let family_limit = if target_word.is_some() {
                1
            } else {
                1 + (seed % 3) as usize
            };
            let maximum_word_length = (seed % 5) as usize;
            let dijkstra = canonical_test_search(
                &positions,
                &edges,
                RepairHeuristic::Zero,
                family_limit,
                maximum_word_length,
                target_word.as_ref(),
                MAX_REPAIR_SETTLED_STATES,
            );
            let astar = canonical_test_search(
                &positions,
                &edges,
                RepairHeuristic::Euclidean,
                family_limit,
                maximum_word_length,
                target_word.as_ref(),
                MAX_REPAIR_SETTLED_STATES,
            );
            assert_canonical_test_search_equal(&dijkstra, &astar);
            assert!(dijkstra.certified, "seed {seed} Dijkstra replay failed");
            assert!(astar.certified, "seed {seed} A* replay failed");
        }
    }

    #[test]
    fn lazy_visibility_certifies_a_local_route_beyond_the_old_node_ceiling() {
        let large_board = Rect {
            min: Vec2::ZERO,
            max: Vec2::new(1_000.0, 1_000.0),
        };
        let mut graph = build_corridor_graph("top", large_board, &[], 0, 1).unwrap();
        graph.obstacles = (0..160)
            .map(|index| {
                let x = 100.0 + (index % 16) as f64 * 10.0;
                let y = 100.0 + (index / 16) as f64 * 10.0;
                CorridorObstacle {
                    id: format!("obstacle-{index:03}"),
                    polygon: vec![
                        Vec2::new(x, y),
                        Vec2::new(x + 1.0, y),
                        Vec2::new(x + 1.0, y + 1.0),
                        Vec2::new(x, y + 1.0),
                    ],
                }
            })
            .collect();
        let reference = [Vec2::new(10.0, 10.0), Vec2::new(990.0, 10.0)];
        let target = HomotopyWord::empty();
        let result = repair_route_visibility(
            &graph,
            &RouteEmbeddingRequest {
                net: "local-large-graph",
                from_component: "source",
                to_component: "target",
                physical_width: 0.2,
                clearance: 0.1,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: Some(&target),
                persistent_basis_matches: true,
            },
            0.2,
            &target,
        );
        let evidence = result.evidence;

        assert_eq!(evidence.nodes.len(), 2 + 160 * 4);
        assert_eq!(evidence.outcome, RepairOutcome::Repaired);
        assert!(!evidence.visibility_graph_limit_reached);
        assert_eq!(
            evidence.termination,
            AlternativeSearchTermination::FamilyLimitReached
        );
        assert!(evidence.search_work.threshold_complete);
        assert!(!evidence.search_complete);
        assert_eq!(evidence.alternatives.len(), 1);
        assert_eq!(evidence.alternatives[0].polyline, reference);
        let evaluated_pairs = evidence.evaluated_edge_candidates;
        assert_eq!(
            evaluated_pairs,
            evidence.edges.len() + evidence.rejected_edges
        );
        let full_pair_count = evidence.nodes.len() * (evidence.nodes.len() - 1) / 2;
        assert_eq!(evaluated_pairs, evidence.nodes.len() - 1);
        assert!(evaluated_pairs < full_pair_count / 100);
        assert!(evaluated_pairs <= MAX_REPAIR_VISIBILITY_PAIR_EVALUATIONS);
        assert_eq!(
            evidence.visibility_geometry_units,
            evaluated_pairs * graph.obstacles.len()
        );
        assert!(evidence.visibility_geometry_units <= MAX_REPAIR_VISIBILITY_GEOMETRY_UNITS);
    }

    #[test]
    fn lazy_visibility_stops_at_actual_work_budget_with_typed_incomplete_evidence() {
        let large_board = Rect {
            min: Vec2::ZERO,
            max: Vec2::new(1_000.0, 1_000.0),
        };
        let mut graph = build_corridor_graph("top", large_board, &[], 0, 1).unwrap();
        graph.obstacles = (0..160)
            .map(|index| {
                let x = 100.0 + (index % 16) as f64 * 10.0;
                let y = 100.0 + (index / 16) as f64 * 10.0;
                CorridorObstacle {
                    id: format!("obstacle-{index:03}"),
                    polygon: vec![
                        Vec2::new(x, y),
                        Vec2::new(x + 1.0, y),
                        Vec2::new(x + 1.0, y + 1.0),
                        Vec2::new(x, y + 1.0),
                    ],
                }
            })
            .collect();
        let reference = [Vec2::new(10.0, 10.0), Vec2::new(990.0, 10.0)];
        let evidence = enumerate_route_class_alternatives(
            &graph,
            RouteEmbeddingRequest {
                net: "bounded-large-graph",
                from_component: "source",
                to_component: "target",
                physical_width: 0.2,
                clearance: 0.1,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
        );

        assert!(evidence.visibility_graph_limit_reached);
        assert_eq!(
            evidence.termination,
            AlternativeSearchTermination::VisibilityGraphLimitReached
        );
        assert!(!evidence.search_complete);
        assert!(evidence.explored_states > 0);
        assert!(!evidence.edges.is_empty());
        let evaluated_pairs = evidence.evaluated_edge_candidates;
        assert_eq!(
            evaluated_pairs,
            evidence.edges.len() + evidence.rejected_edges
        );
        assert_eq!(evaluated_pairs, MAX_REPAIR_VISIBILITY_PAIR_EVALUATIONS);
        assert!(evidence.limitations.iter().any(|limitation| {
            limitation.contains("bounded partial graph is incomplete evidence")
        }));
    }

    #[test]
    fn repair_spatial_index_matches_exhaustive_clearance_and_limiting_feature() {
        let obstacles = [
            rectangle_obstacle("first", Vec2::new(4.0, 2.0), Vec2::new(6.0, 4.0)),
            rectangle_obstacle("second", Vec2::new(4.0, 6.0), Vec2::new(6.0, 8.0)),
            rectangle_obstacle("remote", Vec2::new(14.0, 3.0), Vec2::new(16.0, 7.0)),
        ];
        let graph = build_corridor_graph("top", board(), &obstacles, 0, 1).unwrap();
        let spatial_index = RepairVisibilitySpatialIndex::build(&graph);

        // The first two obstacles are equidistant from this segment. The
        // indexed traversal must retain exhaustive obstacle-order tie
        // semantics even when the BVH visits the other leaf first.
        let tied = [Vec2::new(2.0, 5.0), Vec2::new(8.0, 5.0)];
        let exhaustive = segment_clearance(&graph, tied, &[]);
        let indexed = segment_clearance_indexed(&graph, &spatial_index, tied, &[]);
        assert_eq!(indexed.0.to_bits(), exhaustive.0.to_bits());
        assert_eq!(indexed.1, exhaustive.1);

        let mut state = 0x6a09_e667_f3bc_c909_u64;
        for sample in 0..512 {
            let mut next_coordinate = |extent: f64| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let unit = (state >> 11) as f64 / ((1_u64 << 53) - 1) as f64;
                unit * extent
            };
            let segment = [
                Vec2::new(next_coordinate(20.0), next_coordinate(10.0)),
                Vec2::new(next_coordinate(20.0), next_coordinate(10.0)),
            ];
            let exemptions: &[&str] = if sample % 3 == 0 { &["first"] } else { &[] };
            let exhaustive = segment_clearance(&graph, segment, exemptions);
            let indexed = segment_clearance_indexed(&graph, &spatial_index, segment, exemptions);
            assert_eq!(
                indexed.0.to_bits(),
                exhaustive.0.to_bits(),
                "minimum clearance differs for sample {sample}: {segment:?}"
            );
            assert_eq!(
                indexed.1, exhaustive.1,
                "limiting feature differs for sample {sample}: {segment:?}"
            );
        }
    }

    #[test]
    fn repair_spatial_index_matches_exhaustive_cut_words_and_ambiguity() {
        let obstacles = [
            rectangle_obstacle("left", Vec2::new(4.0, 2.0), Vec2::new(6.0, 7.0)),
            rectangle_obstacle("right", Vec2::new(12.0, 3.0), Vec2::new(14.0, 8.0)),
        ];
        let graph = build_corridor_graph("top", board(), &obstacles, 0, 1).unwrap();
        assert!(graph.cut_basis.complete, "{:?}", graph.cut_basis.warnings);
        let spatial_index = RepairVisibilitySpatialIndex::build(&graph);
        let mut scratch = RepairSpatialQueryScratch::default();
        let mut state = 0xbb67_ae85_84ca_a73b_u64;
        for sample in 0..512 {
            let mut next_coordinate = |extent: f64| {
                state = state
                    .wrapping_mul(2_862_933_555_777_941_757)
                    .wrapping_add(3_037_000_493);
                let unit = (state >> 11) as f64 / ((1_u64 << 53) - 1) as f64;
                unit * extent
            };
            let polyline = [
                Vec2::new(next_coordinate(20.0), next_coordinate(10.0)),
                Vec2::new(next_coordinate(20.0), next_coordinate(10.0)),
                Vec2::new(next_coordinate(20.0), next_coordinate(10.0)),
            ];
            let excluded: &[&str] = if sample % 5 == 0 { &["left"] } else { &[] };
            let exhaustive = polyline_signature(&polyline, &graph.cut_basis, excluded);
            let indexed = polyline_signature_indexed(
                &polyline,
                &graph.cut_basis,
                &spatial_index,
                excluded,
                &mut scratch,
            );
            assert_eq!(
                indexed.word, exhaustive.word,
                "cut word differs for sample {sample}: {polyline:?}"
            );
            assert_eq!(
                indexed.ambiguous, exhaustive.ambiguous,
                "ambiguity differs for sample {sample}: {polyline:?}"
            );
        }
    }

    #[test]
    fn repair_spatial_index_prunes_remote_obstacles_and_cuts_before_exact_tests() {
        let large_board = Rect {
            min: Vec2::ZERO,
            max: Vec2::new(1_000.0, 1_000.0),
        };
        let mut graph = build_corridor_graph("top", large_board, &[], 0, 1).unwrap();
        graph.obstacles = (0..256)
            .map(|index| {
                let x = 100.0 + (index % 16) as f64 * 4.0;
                let y = 100.0 + (index / 16) as f64 * 4.0;
                rectangle_obstacle(
                    format!("remote-{index:03}"),
                    Vec2::new(x, y),
                    Vec2::new(x + 1.0, y + 1.0),
                )
            })
            .collect();
        graph.cut_basis.cuts = (0..128)
            .map(|index| TopologyCut {
                generator: format!("remote-cut-{index:03}"),
                source_component: format!("remote-component-{index:03}"),
                segment: [
                    Vec2::new(100.0 + index as f64 * 2.0, 100.0),
                    Vec2::new(100.0 + index as f64 * 2.0, 200.0),
                ],
                target: TopologyCutTarget::BoardBottom,
            })
            .collect();
        let spatial_index = RepairVisibilitySpatialIndex::build(&graph);
        let route = [Vec2::new(10.0, 10.0), Vec2::new(20.0, 10.0)];
        let mut exact_obstacle_tests = usize::MAX;
        let indexed = segment_clearance_indexed_with_work(
            &graph,
            &spatial_index,
            route,
            &[],
            &mut exact_obstacle_tests,
        );
        let exhaustive = segment_clearance(&graph, route, &[]);
        assert_eq!(indexed.0.to_bits(), exhaustive.0.to_bits());
        assert_eq!(indexed.1, exhaustive.1);
        assert_eq!(exact_obstacle_tests, 0);

        let mut scratch = RepairSpatialQueryScratch::default();
        let indexed =
            polyline_signature_indexed(&route, &graph.cut_basis, &spatial_index, &[], &mut scratch);
        let exhaustive = polyline_signature(&route, &graph.cut_basis, &[]);
        assert_eq!(indexed.word, exhaustive.word);
        assert_eq!(indexed.ambiguous, exhaustive.ambiguous);
        assert_eq!(scratch.exact_cut_tests, 0);
    }

    #[test]
    fn repair_cut_index_retains_parameter_and_parallel_tolerance_touches() {
        let mut graph = build_corridor_graph("top", board(), &[], 0, 1).unwrap();
        graph.cut_basis.cuts = vec![TopologyCut {
            generator: "test-generator".into(),
            source_component: "test-component".into(),
            segment: [Vec2::new(10.0, 2.0), Vec2::new(10.0, 8.0)],
            target: TopologyCutTarget::BoardBottom,
        }];
        let spatial_index = RepairVisibilitySpatialIndex::build(&graph);
        let mut scratch = RepairSpatialQueryScratch::default();
        let routes = [
            // The line intersection lies just beyond the route endpoint but
            // within `segment_cut_crossing`'s parameter tolerance.
            [Vec2::new(0.0, 5.0), Vec2::new(10.0 - EPSILON * 5.0, 5.0)],
            // Parallel and just outside the raw cut AABB, but close enough to
            // be an ambiguous touch under the exact predicate.
            [
                Vec2::new(10.0 + EPSILON * 0.5, 3.0),
                Vec2::new(10.0 + EPSILON * 0.5, 7.0),
            ],
        ];
        for route in routes {
            let exhaustive = polyline_signature(&route, &graph.cut_basis, &[]);
            let indexed = polyline_signature_indexed(
                &route,
                &graph.cut_basis,
                &spatial_index,
                &[],
                &mut scratch,
            );
            assert!(exhaustive.ambiguous);
            assert_eq!(indexed.word, exhaustive.word);
            assert_eq!(indexed.ambiguous, exhaustive.ambiguous);
        }
    }

    #[test]
    fn invalid_repair_spatial_geometry_fails_closed() {
        let mut graph = build_corridor_graph("top", board(), &[], 0, 1).unwrap();
        graph.obstacles.push(CorridorObstacle {
            id: "invalid".into(),
            polygon: vec![Vec2::new(f64::NAN, 1.0), Vec2::new(2.0, 2.0)],
        });
        graph.cut_basis.cuts.push(TopologyCut {
            generator: "invalid".into(),
            source_component: "invalid".into(),
            segment: [Vec2::new(f64::NAN, 0.0), Vec2::new(1.0, 1.0)],
            target: TopologyCutTarget::BoardBottom,
        });
        let spatial_index = RepairVisibilitySpatialIndex::build(&graph);
        let route = [Vec2::new(1.0, 1.0), Vec2::new(19.0, 9.0)];
        let clearance = segment_clearance_indexed(&graph, &spatial_index, route, &[]);
        assert_eq!(clearance.0, f64::NEG_INFINITY);
        let signature = polyline_signature_indexed(
            &route,
            &graph.cut_basis,
            &spatial_index,
            &[],
            &mut RepairSpatialQueryScratch::default(),
        );
        assert!(signature.ambiguous);
    }

    #[test]
    fn semantic_graph_ignores_circle_tessellation_detail() {
        let graphs = [8, 16, 32].map(|vertices| {
            build_corridor_graph(
                "top",
                board(),
                &[CorridorObstacle {
                    id: "round-pad".into(),
                    polygon: regular_polygon(Vec2::new(10.0, 5.0), 3.0, vertices),
                }],
                0,
                1,
            )
            .unwrap()
        });
        assert!(graphs[0].cells.len() < graphs[1].cells.len());
        assert!(graphs[1].cells.len() < graphs[2].cells.len());
        assert_eq!(
            graphs[0].semantic.fingerprint,
            graphs[1].semantic.fingerprint
        );
        assert_eq!(
            graphs[1].semantic.fingerprint,
            graphs[2].semantic.fingerprint
        );
    }

    #[test]
    fn semantic_graph_survives_small_obstacle_motion() {
        let graph = |center| {
            build_corridor_graph(
                "top",
                board(),
                &[CorridorObstacle {
                    id: "round-pad".into(),
                    polygon: regular_polygon(center, 3.0, 16),
                }],
                0,
                1,
            )
            .unwrap()
        };
        let before = graph(Vec2::new(10.0, 5.0));
        let after = graph(Vec2::new(10.02, 5.01));
        assert_eq!(before.semantic.fingerprint, after.semantic.fingerprint);
    }

    #[test]
    fn one_component_move_preserves_unaffected_semantic_identities() {
        let locality_board = Rect {
            min: Vec2::ZERO,
            max: Vec2::new(80.0, 80.0),
        };
        let mut before_obstacles = Vec::new();
        for row in 0..3 {
            for column in 0..3 {
                before_obstacles.push(CorridorObstacle {
                    id: if row == 1 && column == 1 {
                        "movable".into()
                    } else {
                        format!("fixed-{row}-{column}")
                    },
                    polygon: regular_polygon(
                        Vec2::new(15.0 + column as f64 * 20.0, 15.0 + row as f64 * 20.0),
                        2.0,
                        12,
                    ),
                });
            }
        }
        let mut after_obstacles = before_obstacles.clone();
        for point in &mut after_obstacles[4].polygon {
            *point = add(*point, Vec2::new(0.02, 0.01));
        }
        let before = build_corridor_graph("top", locality_board, &before_obstacles, 4, 10).unwrap();
        let after = build_corridor_graph("top", locality_board, &after_obstacles, 5, 11).unwrap();
        let report = analyze_corridor_rebuild(&before, &after, &["movable".into()]).unwrap();
        assert_eq!(report.changed_obstacles, vec!["movable".to_owned()]);
        assert_eq!(
            report.full_rebuild_work,
            CorridorRebuildWork {
                raw_cells_constructed: after.cells.len(),
                raw_gates_constructed: after.gates.len(),
                semantic_regions_reduced: after.semantic.regions.len(),
                semantic_passages_reduced: after.semantic.passages.len(),
            }
        );
        assert!(report.regions.affected_after > 0);
        assert!(report.regions.unaffected_before > 0);
        // The raw global CDT is disposable: a local obstacle motion may flip a
        // small number of remote triangles and therefore semantic identities.
        // This is a measured locality diagnostic, never a reuse certificate.
        assert!(
            report.regions.unaffected_keys_survived * 5 >= report.regions.unaffected_before * 4,
            "small local motion must preserve at least 80% of remote region identities"
        );
        assert!(
            report.regions.unaffected_geometry_stable * 5 >= report.regions.unaffected_before * 4,
            "small local motion must preserve stable geometry for at least 80% of remote regions"
        );
        assert!(report.passages.affected_after > 0);
        assert!(report.passages.unaffected_before > 0);
        assert!(
            report.passages.unaffected_keys_survived * 5 >= report.passages.unaffected_before * 4,
            "small local motion must preserve at least 80% of remote passage identities"
        );
        assert!(
            report.passages.unaffected_geometry_stable * 5 >= report.passages.unaffected_before * 4,
            "small local motion must preserve stable geometry for at least 80% of remote passages"
        );
        assert!(report.passages.reuse_candidates_after > 0);
        assert!(
            report.passages.conservative_refresh_after < report.passages.after,
            "the diagnostic fixture must expose work avoided by blocker-local reuse"
        );
        let encoded = serde_json::to_value(&report).unwrap();
        assert_eq!(encoded["layer"], "top");
        assert_eq!(encoded["changed_obstacles"][0], "movable");
        assert!(encoded["regions"]["unaffected_keys_survived"].is_number());
        assert!(encoded["passages"]["reuse_candidates_after"].is_number());
    }

    #[test]
    fn corridor_rebuild_delta_rejects_unknown_changed_obstacles() {
        let graph = build_corridor_graph("top", board(), &[], 0, 1).unwrap();
        assert_eq!(
            analyze_corridor_rebuild(&graph, &graph, &["missing".into()]),
            Err("changed corridor obstacle missing is unknown".into())
        );
    }

    #[test]
    fn corridor_locality_scaling_diagnostic_is_deterministic() {
        #[derive(serde::Serialize)]
        struct Fraction {
            numerator: usize,
            denominator: usize,
            value: f64,
        }

        impl Fraction {
            fn new(numerator: usize, denominator: usize) -> Self {
                Self {
                    numerator,
                    denominator,
                    value: if denominator == 0 {
                        0.0
                    } else {
                        numerator as f64 / denominator as f64
                    },
                }
            }
        }

        #[derive(serde::Serialize)]
        struct ScaleRecord {
            obstacle_count: usize,
            moved_obstacle: String,
            full_rebuild_work: CorridorRebuildWork,
            region_unaffected_geometry_survival: Fraction,
            region_full_rebuild_reuse_candidates: Fraction,
            passage_unaffected_geometry_survival: Fraction,
            passage_full_rebuild_reuse_candidates: Fraction,
            delta: CorridorRebuildDelta,
        }

        let large_board = Rect {
            min: Vec2::ZERO,
            max: Vec2::new(80.0, 80.0),
        };
        let mut records = Vec::new();
        for side in [3usize, 5, 7] {
            let mut before_obstacles = Vec::new();
            for row in 0..side {
                for column in 0..side {
                    before_obstacles.push(CorridorObstacle {
                        id: format!("blocker-{row:02}-{column:02}"),
                        polygon: regular_polygon(
                            Vec2::new(
                                10.0 + column as f64 * 10.0,
                                10.0 + row as f64 * 10.0 + column as f64 * 0.17,
                            ),
                            2.0,
                            8,
                        ),
                    });
                }
            }
            let moved_index = (side / 2) * side + side / 2;
            let moved_obstacle = before_obstacles[moved_index].id.clone();
            let mut after_obstacles = before_obstacles.clone();
            for point in &mut after_obstacles[moved_index].polygon {
                *point = add(*point, Vec2::new(0.02, 0.01));
            }
            let before = build_corridor_graph(
                "top",
                large_board,
                &before_obstacles,
                20 + side as u64,
                40 + side as u64,
            )
            .unwrap();
            let after = build_corridor_graph(
                "top",
                large_board,
                &after_obstacles,
                21 + side as u64,
                41 + side as u64,
            )
            .unwrap();
            let repeated_after = build_corridor_graph(
                "top",
                large_board,
                &after_obstacles,
                21 + side as u64,
                41 + side as u64,
            )
            .unwrap();
            let changed = vec![moved_obstacle.clone()];
            let delta = analyze_corridor_rebuild(&before, &after, &changed).unwrap();
            let repeated = analyze_corridor_rebuild(&before, &repeated_after, &changed).unwrap();

            assert_eq!(
                delta, repeated,
                "side {side} rebuild delta changed between repeats"
            );
            assert_eq!(delta.changed_obstacles, changed);
            assert_eq!(
                delta.full_rebuild_work,
                CorridorRebuildWork {
                    raw_cells_constructed: after.cells.len(),
                    raw_gates_constructed: after.gates.len(),
                    semantic_regions_reduced: after.semantic.regions.len(),
                    semantic_passages_reduced: after.semantic.passages.len(),
                }
            );
            assert!(
                delta.regions.unaffected_geometry_stable <= delta.regions.unaffected_keys_survived
            );
            assert!(
                delta.passages.unaffected_geometry_stable
                    <= delta.passages.unaffected_keys_survived
            );
            assert!(delta.regions.reuse_candidates_after <= delta.regions.after);
            assert!(delta.passages.reuse_candidates_after <= delta.passages.after);

            records.push(ScaleRecord {
                obstacle_count: before_obstacles.len(),
                moved_obstacle,
                full_rebuild_work: delta.full_rebuild_work.clone(),
                region_unaffected_geometry_survival: Fraction::new(
                    delta.regions.unaffected_geometry_stable,
                    delta.regions.unaffected_before,
                ),
                region_full_rebuild_reuse_candidates: Fraction::new(
                    delta.regions.reuse_candidates_after,
                    delta.regions.after,
                ),
                passage_unaffected_geometry_survival: Fraction::new(
                    delta.passages.unaffected_geometry_stable,
                    delta.passages.unaffected_before,
                ),
                passage_full_rebuild_reuse_candidates: Fraction::new(
                    delta.passages.reuse_candidates_after,
                    delta.passages.after,
                ),
                delta,
            });
        }
        let output = serde_json::json!({
            "schema_version": 1,
            "kind": "corridor_locality_scaling",
            "timing_is_a_gate": false,
            "records": records,
        });
        assert_eq!(output["records"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn semantic_graph_keeps_topology_crossings_after_region_contraction() {
        let graph = build_corridor_graph(
            "top",
            board(),
            &[CorridorObstacle {
                id: "middle".into(),
                polygon: regular_polygon(Vec2::new(10.0, 5.0), 3.0, 16),
            }],
            0,
            1,
        )
        .unwrap();
        let generator = &graph.cut_basis.cuts[0].generator;
        assert!(
            graph
                .semantic
                .passages
                .iter()
                .any(|passage| passage.topology_cuts.len() == 1
                    && passage.topology_cuts[0] == *generator)
        );
    }

    #[test]
    fn pad_centered_terminals_can_exit_their_own_obstacles() {
        let obstacles = [
            CorridorObstacle {
                id: "source".into(),
                polygon: regular_polygon(Vec2::new(2.0, 5.0), 1.4, 16),
            },
            CorridorObstacle {
                id: "target".into(),
                polygon: regular_polygon(Vec2::new(18.0, 5.0), 1.4, 16),
            },
        ];
        let graph = build_corridor_graph("top", board(), &obstacles, 0, 1).unwrap();
        let reference = [Vec2::new(2.0, 5.0), Vec2::new(18.0, 5.0)];
        let embedding = embed_route(
            &graph,
            RouteEmbeddingRequest {
                net: "pad-to-pad",
                from_component: "source",
                to_component: "target",
                physical_width: 0.3,
                clearance: 0.2,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
        );
        assert!(embedding.failure.is_none(), "{:?}", embedding.failure);
        assert!(!embedding.semantic_regions.is_empty());
        assert_eq!(
            embedding.method,
            EmbeddingMethod::CertifiedReferenceContinuation
        );
        assert_eq!(
            embedding.geometry_method,
            GeometryMethod::CertifiedReference
        );
        assert!(
            embedding.projection.is_none(),
            "ordinary successful projection must not retain hot-path diagnostics"
        );
        assert_eq!(embedding.polyline, reference);
    }

    #[test]
    fn route_embedding_respects_full_width_capacity() {
        let obstacle = CorridorObstacle {
            id: "middle".into(),
            polygon: vec![
                Vec2::new(8.0, 2.0),
                Vec2::new(12.0, 2.0),
                Vec2::new(12.0, 8.0),
                Vec2::new(8.0, 8.0),
            ],
        };
        let graph = build_corridor_graph("top", board(), &[obstacle], 0, 1).unwrap();
        assert!(graph.cells.iter().all(|cell| !point_in_convex_polygon(
            cell.center,
            &[
                Vec2::new(8.0, 2.0),
                Vec2::new(12.0, 2.0),
                Vec2::new(12.0, 8.0),
                Vec2::new(8.0, 8.0),
            ]
        )));
        let reference = [Vec2::new(1.0, 5.0), Vec2::new(19.0, 5.0)];
        let embedding = embed_route(
            &graph,
            RouteEmbeddingRequest {
                net: "n",
                from_component: "source",
                to_component: "target",
                physical_width: 0.5,
                clearance: 0.2,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
        );
        assert!(embedding.failure.is_none(), "{:?}", embedding.failure);
        assert!(!embedding.gates.is_empty());
        assert!(!embedding.route_class_verified);
        assert!(embedding.epoch_homotopy_verified);
        assert_eq!(embedding.reference_homotopy, embedding.embedded_homotopy);
        assert!(embedding.clearance.certified, "{:?}", embedding.clearance);
        assert!((embedding.required_width - 0.9).abs() < EPSILON);
        assert_eq!(embedding.method, EmbeddingMethod::ClearanceVisibilityRepair);
        let repair = embedding.repair.as_ref().unwrap();
        assert_eq!(repair.outcome, RepairOutcome::Repaired);
        assert!(!repair.nodes.is_empty());
        assert!(!repair.edges.is_empty());
        assert!(!repair.alternatives.is_empty());
    }

    #[test]
    fn retry_anchor_ignores_point_touch_and_requires_one_earliest_semantic_region() {
        let mut graph = build_corridor_graph("top", board(), &[], 0, 1).unwrap();
        graph.cells = vec![
            CorridorCell {
                id: 0,
                triangle: [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(-1.0, -1.0),
                    Vec2::new(-1.0, 1.0),
                ],
                center: Vec2::new(-2.0 / 3.0, 0.0),
                boundary_features: Vec::new(),
            },
            CorridorCell {
                id: 1,
                triangle: [
                    Vec2::new(4.0, -1.0),
                    Vec2::new(4.0, 1.0),
                    Vec2::new(8.0, 0.0),
                ],
                center: Vec2::new(16.0 / 3.0, 0.0),
                boundary_features: Vec::new(),
            },
            CorridorCell {
                id: 2,
                triangle: [
                    Vec2::new(4.0, -1.0),
                    Vec2::new(4.0, 1.0),
                    Vec2::new(6.0, 0.0),
                ],
                center: Vec2::new(14.0 / 3.0, 0.0),
                boundary_features: Vec::new(),
            },
            CorridorCell {
                id: 3,
                triangle: [
                    Vec2::new(4.0, -1.0),
                    Vec2::new(4.0, 1.0),
                    Vec2::new(8.0, 0.0),
                ],
                center: Vec2::new(16.0 / 3.0, 0.0),
                boundary_features: Vec::new(),
            },
        ];
        graph.semantic.raw_cell_to_region = vec![0, 1, 1, 1];

        assert_eq!(
            terminal_cell_after_free_space_gap(&graph, Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)),
            GapAnchor::Unique(1),
            "cell 0 is only a numerical point touch; equal-entry cells 1 and 3 tie by stable ID after the longer interval wins"
        );
        assert_eq!(
            terminal_cell_after_free_space_gap(&graph, Vec2::new(10.0, 0.0), Vec2::new(0.0, 0.0)),
            GapAnchor::Unique(1),
            "the same interval rule must anchor the reversed target heading"
        );

        graph.semantic.raw_cell_to_region[3] = 2;
        let ambiguous = terminal_projection(&graph, Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0));
        assert_eq!(
            ambiguous.gap_anchor,
            GapAnchor::AmbiguousRegions,
            "positive earliest intervals in different semantic regions are ambiguous and must fail closed"
        );
        assert_eq!(
            ambiguous.evidence.gap_anchor_outcome,
            GapAnchorOutcome::AmbiguousRegions
        );
        assert_eq!(ambiguous.evidence.gap_anchor_candidate_cells, vec![1, 2, 3]);
        assert_eq!(ambiguous.evidence.gap_anchor_candidate_count, 3);
        assert_eq!(ambiguous.evidence.gap_anchor_regions, vec![1, 2]);
        assert_eq!(ambiguous.evidence.gap_anchor_region_count, 2);
        assert_eq!(ambiguous.evidence.gap_anchor_intervals.len(), 3);
        assert!(
            ambiguous
                .evidence
                .gap_anchor_intervals
                .iter()
                .all(|interval| interval.start_parameter > 0.0
                    && interval.span_parameter > TERMINAL_INTERVAL_PARAMETER_EPSILON)
        );

        graph.gates.clear();
        let reference = [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)];
        let source = terminal_cell(&graph, reference[0], reference[1]).unwrap();
        let target = terminal_cell(&graph, reference[1], reference[0]).unwrap();
        let projected = project_certified_reference_to_corridor(
            &graph,
            &RouteEmbeddingRequest {
                net: "ambiguous-gap",
                from_component: "source",
                to_component: "target",
                physical_width: 0.1,
                clearance: 0.0,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
            source,
            target,
        );
        assert!(projected.traversal.is_none());
        assert!(
            projected
                .evidence
                .unwrap()
                .failure_reasons
                .contains(&RouteProjectionFailureReason::SourceGapAnchorAmbiguous)
        );
    }

    #[test]
    fn mixed_ambiguous_and_unique_gap_anchors_abort_certified_retry() {
        assert_eq!(
            resolve_gap_anchor_pair(5, 17, GapAnchor::AmbiguousRegions, GapAnchor::Unique(17)),
            None
        );
        assert_eq!(
            resolve_gap_anchor_pair(5, 17, GapAnchor::Unique(9), GapAnchor::NoGapKeepLegacy),
            Some((9, 17))
        );
    }

    #[test]
    fn terminal_projection_reports_both_missing_direct_endpoints() {
        let mut graph = build_corridor_graph("top", board(), &[], 0, 1).unwrap();
        graph.cells.clear();
        graph.gates.clear();
        graph.semantic.raw_cell_to_region.clear();
        graph.semantic.raw_gate_to_passage.clear();
        let reference = [Vec2::new(1.0, 5.0), Vec2::new(19.0, 5.0)];
        let embedding = embed_route(
            &graph,
            RouteEmbeddingRequest {
                net: "missing-terminals",
                from_component: "source",
                to_component: "target",
                physical_width: 0.1,
                clearance: 0.0,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
        );
        let projection = embedding.projection.expect("typed terminal evidence");
        assert_eq!(
            projection.source.direct_selection,
            DirectTerminalSelection::Missing
        );
        assert_eq!(
            projection.target.direct_selection,
            DirectTerminalSelection::Missing
        );
        assert_eq!(
            projection.failure_reasons,
            vec![
                RouteProjectionFailureReason::SourceDirectCellMissing,
                RouteProjectionFailureReason::TargetDirectCellMissing,
            ]
        );
        assert!(projection.ordinary_attempt.is_none());
    }

    #[test]
    fn certified_projection_reports_missing_gap_anchors_separately() {
        let mut graph = build_corridor_graph("top", board(), &[], 0, 1).unwrap();
        graph.cells = vec![
            CorridorCell {
                id: 0,
                triangle: [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(3.0, 0.0),
                    Vec2::new(0.0, 3.0),
                ],
                center: Vec2::new(1.0, 1.0),
                boundary_features: Vec::new(),
            },
            CorridorCell {
                id: 1,
                triangle: [
                    Vec2::new(8.0, 0.0),
                    Vec2::new(10.0, 0.0),
                    Vec2::new(10.0, 3.0),
                ],
                center: Vec2::new(28.0 / 3.0, 1.0),
                boundary_features: Vec::new(),
            },
        ];
        graph.gates.clear();
        graph.semantic.raw_cell_to_region = vec![0, 0];
        graph.semantic.raw_gate_to_passage.clear();
        let reference = [Vec2::new(1.0, 5.0), Vec2::new(9.0, 5.0)];
        let embedding = embed_route(
            &graph,
            RouteEmbeddingRequest {
                net: "missing-gap-anchors",
                from_component: "source",
                to_component: "target",
                physical_width: 0.1,
                clearance: 0.0,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
        );
        let projection = embedding.projection.expect("typed projection evidence");
        assert_eq!(
            projection.source.direct_selection,
            DirectTerminalSelection::NearestFallback
        );
        assert_eq!(
            projection.source.gap_anchor_outcome,
            GapAnchorOutcome::Missing
        );
        assert_eq!(
            projection.target.gap_anchor_outcome,
            GapAnchorOutcome::Missing
        );
        assert!(
            projection
                .failure_reasons
                .contains(&RouteProjectionFailureReason::SourceGapAnchorMissing)
        );
        assert!(
            projection
                .failure_reasons
                .contains(&RouteProjectionFailureReason::TargetGapAnchorMissing)
        );
        assert!(projection.gap_retry_attempt.is_none());
    }

    #[test]
    fn certified_projection_distinguishes_a_disconnected_raw_cell_chain() {
        let mut graph = build_corridor_graph("top", board(), &[], 0, 1).unwrap();
        graph.cells = vec![
            CorridorCell {
                id: 0,
                triangle: [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(3.0, 0.0),
                    Vec2::new(0.0, 3.0),
                ],
                center: Vec2::new(1.0, 1.0),
                boundary_features: Vec::new(),
            },
            CorridorCell {
                id: 1,
                triangle: [
                    Vec2::new(8.0, 0.0),
                    Vec2::new(10.0, 0.0),
                    Vec2::new(10.0, 3.0),
                ],
                center: Vec2::new(28.0 / 3.0, 1.0),
                boundary_features: Vec::new(),
            },
        ];
        graph.gates.clear();
        graph.semantic.raw_cell_to_region = vec![0, 0];
        graph.semantic.raw_gate_to_passage.clear();
        let reference = [Vec2::new(1.0, 1.0), Vec2::new(9.0, 1.0)];
        let embedding = embed_route(
            &graph,
            RouteEmbeddingRequest {
                net: "disconnected-projection",
                from_component: "source",
                to_component: "target",
                physical_width: 0.1,
                clearance: 0.0,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
        );
        assert!(embedding.clearance.certified);
        assert!(embedding.failure.is_some());
        let projection = embedding.projection.expect("typed projection evidence");
        let ordinary = projection.ordinary_attempt.as_ref().unwrap();
        assert_eq!(ordinary.source_cell, 0);
        assert_eq!(ordinary.target_cell, 1);
        assert_eq!(ordinary.outcome, RawProjectionAttemptOutcome::Disconnected);
        assert!(!ordinary.source_target_graph_connected);
        assert!(!ordinary.source_target_touched_connected);
        assert!(ordinary.uncovered_interval_count > 0);
        assert!(projection.gap_retry_attempt.is_none());
        assert_eq!(
            projection.failure_reasons,
            vec![
                RouteProjectionFailureReason::OrdinaryRawChainDisconnected,
                RouteProjectionFailureReason::GapRetryUnchanged,
            ]
        );
    }

    #[test]
    fn successful_visibility_repair_is_independent_of_terminal_heading() {
        let obstacles = [
            CorridorObstacle {
                id: "source".into(),
                polygon: regular_polygon(Vec2::new(2.0, 5.0), 1.4, 8),
            },
            CorridorObstacle {
                id: "middle".into(),
                polygon: vec![
                    Vec2::new(8.0, 2.0),
                    Vec2::new(12.0, 2.0),
                    Vec2::new(12.0, 8.0),
                    Vec2::new(8.0, 8.0),
                ],
            },
            CorridorObstacle {
                id: "target".into(),
                polygon: regular_polygon(Vec2::new(18.0, 5.0), 1.4, 8),
            },
        ];
        let graph = build_corridor_graph("top", board(), &obstacles, 0, 1).unwrap();
        let reference = [Vec2::new(2.0, 5.0), Vec2::new(18.0, 5.0)];
        let toward_route_heading = reference[1];
        let toward_board_edge_heading = Vec2::new(2.0, 9.0);
        assert_ne!(
            terminal_cell(&graph, reference[0], toward_route_heading),
            terminal_cell(&graph, reference[0], toward_board_edge_heading),
            "the regression headings must exercise different preliminary terminal cells"
        );
        let embed_with_heading = |from_toward| {
            embed_route(
                &graph,
                RouteEmbeddingRequest {
                    net: "n",
                    from_component: "source",
                    to_component: "target",
                    physical_width: 0.5,
                    clearance: 0.2,
                    from: reference[0],
                    from_toward,
                    to: reference[1],
                    to_toward: reference[0],
                    reference: &reference,
                    target_homotopy: None,
                    persistent_basis_matches: false,
                },
            )
        };

        let toward_route = embed_with_heading(toward_route_heading);
        let toward_board_edge = embed_with_heading(toward_board_edge_heading);
        assert_eq!(
            toward_route.method,
            EmbeddingMethod::ClearanceVisibilityRepair
        );
        assert_eq!(
            toward_board_edge.method,
            EmbeddingMethod::ClearanceVisibilityRepair
        );
        assert_eq!(
            toward_route.repair.as_ref().unwrap().outcome,
            RepairOutcome::Repaired
        );
        assert_eq!(
            toward_board_edge.repair.as_ref().unwrap().outcome,
            RepairOutcome::Repaired
        );

        // `from_toward` selects only the preliminary disposable terminal cell.
        // A successful visibility repair is computed from the fixed endpoints,
        // reference, clearance, and route class, then reprojected using the
        // repaired polyline's own endpoint tangent. Once that repair succeeds,
        // retrying more terminal headings cannot produce different evidence.
        assert_eq!(
            serde_json::to_value(&toward_route).unwrap(),
            serde_json::to_value(&toward_board_edge).unwrap()
        );
    }

    #[test]
    fn centerline_seed_routes_at_zero_width_but_reserves_declared_width() {
        let obstacle = CorridorObstacle {
            id: "middle".into(),
            polygon: vec![
                Vec2::new(8.0, 2.0),
                Vec2::new(12.0, 2.0),
                Vec2::new(12.0, 8.0),
                Vec2::new(8.0, 8.0),
            ],
        };
        let graph = build_corridor_graph("top", board(), &[obstacle], 0, 1).unwrap();
        let reference = [Vec2::new(1.0, 5.0), Vec2::new(19.0, 5.0)];
        let embedding = embed_route_centerline_seed(
            &graph,
            RouteEmbeddingRequest {
                net: "n",
                from_component: "source",
                to_component: "target",
                physical_width: 0.5,
                clearance: 0.2,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
        );
        assert!(embedding.failure.is_none(), "{:?}", embedding.failure);
        assert_eq!(embedding.purpose, EmbeddingPurpose::CenterlineSeed);
        assert!((embedding.required_width - 0.9).abs() < EPSILON);
        assert!(embedding.clearance.certified);
        assert!(embedding.clearance.required_radius <= EPSILON * 16.0);
        assert!(embedding.repair.is_none());
        assert!(!embedding.route_class_verified);
    }

    #[test]
    fn incremental_cdt_partitions_overlapping_body_and_pads() {
        let obstacles = vec![
            rectangle_obstacle("body", Vec2::new(6.0, 2.0), Vec2::new(14.0, 8.0)),
            rectangle_obstacle("pad:left", Vec2::new(4.0, 4.0), Vec2::new(8.0, 6.0)),
            rectangle_obstacle("pad:top", Vec2::new(9.0, 1.0), Vec2::new(11.0, 4.0)),
            // This pad partially overlaps the body's y=2 boundary. The
            // shared interval must be atomized once, not omitted.
            rectangle_obstacle("pad:collinear", Vec2::new(10.0, 2.0), Vec2::new(16.0, 4.0)),
        ];
        let graph = build_corridor_graph("top", board(), &obstacles, 0, 1).unwrap();

        assert_sampled_segments_have_no_obstacle_free_coverage_holes(
            &graph,
            &[
                [Vec2::new(0.5, 1.5), Vec2::new(19.5, 1.5)],
                [Vec2::new(0.5, 3.0), Vec2::new(19.5, 3.0)],
                [Vec2::new(0.5, 5.0), Vec2::new(19.5, 5.0)],
                [Vec2::new(0.5, 7.0), Vec2::new(19.5, 7.0)],
                [Vec2::new(5.0, 0.5), Vec2::new(5.0, 9.5)],
                [Vec2::new(9.5, 0.5), Vec2::new(9.5, 9.5)],
                [Vec2::new(12.0, 0.5), Vec2::new(12.0, 9.5)],
                [Vec2::new(15.0, 0.5), Vec2::new(15.0, 9.5)],
            ],
        );

        for cell in &graph.cells {
            for weights in [
                [1.0 / 3.0; 3],
                [0.6, 0.2, 0.2],
                [0.2, 0.6, 0.2],
                [0.2, 0.2, 0.6],
            ] {
                let sample = add(
                    add(
                        scale(cell.triangle[0], weights[0]),
                        scale(cell.triangle[1], weights[1]),
                    ),
                    scale(cell.triangle[2], weights[2]),
                );
                assert!(
                    !graph
                        .obstacles
                        .iter()
                        .any(|obstacle| { point_in_convex_polygon(sample, &obstacle.polygon) }),
                    "free cell {} contains obstacle-interior sample {sample:?}",
                    cell.id
                );
            }
        }

        let mut constrained_polygons = graph
            .obstacles
            .iter()
            .map(|obstacle| (obstacle.id.as_str(), obstacle.polygon.as_slice()))
            .collect::<Vec<_>>();
        let board_polygon = [
            Vec2::new(graph.board.min.x, graph.board.min.y),
            Vec2::new(graph.board.max.x, graph.board.min.y),
            Vec2::new(graph.board.max.x, graph.board.max.y),
            Vec2::new(graph.board.min.x, graph.board.max.y),
        ];
        constrained_polygons.push(("board", &board_polygon));
        for gate in &graph.gates {
            for (owner, polygon) in &constrained_polygons {
                for index in 0..polygon.len() {
                    let boundary = [polygon[index], polygon[(index + 1) % polygon.len()]];
                    assert!(
                        !segments_cross_interior(gate.segment, boundary),
                        "gate {} crosses constrained boundary {}:{}",
                        gate.id,
                        owner,
                        index
                    );
                }
            }
        }

        let repeated = build_corridor_graph("top", board(), &obstacles, 0, 1).unwrap();
        let mut reordered = obstacles.clone();
        reordered.reverse();
        let reordered = build_corridor_graph("top", board(), &reordered, 0, 1).unwrap();
        assert_eq!(
            serde_json::to_vec(&graph).unwrap(),
            serde_json::to_vec(&repeated).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&graph).unwrap(),
            serde_json::to_vec(&reordered).unwrap()
        );
        assert_eq!(graph.semantic.fingerprint, repeated.semantic.fingerprint);
        assert_eq!(graph.semantic.fingerprint, reordered.semantic.fingerprint);
        assert_eq!(graph.cut_basis.fingerprint, repeated.cut_basis.fingerprint);
        assert_eq!(graph.cut_basis.fingerprint, reordered.cut_basis.fingerprint);
        assert!(graph.warnings.iter().any(|warning| {
            warning.contains("incremental split CDT")
                && warning.contains("collinear-overlap pairs")
                && !warning.contains("omitted")
        }));
    }

    #[test]
    fn boundary_preparation_handles_duplicate_shared_and_collinear_geometry() {
        let duplicates = [
            rectangle_obstacle("duplicate:a", Vec2::new(4.0, 3.0), Vec2::new(8.0, 7.0)),
            rectangle_obstacle("duplicate:b", Vec2::new(4.0, 3.0), Vec2::new(8.0, 7.0)),
        ];
        let (_, _, _, duplicate_work) = prepare_boundary_constraints(board(), &duplicates).unwrap();
        assert_eq!(duplicate_work.duplicate_atomic_edges, 4);
        assert!(duplicate_work.shared_endpoint_pairs >= 4);
        assert!(duplicate_work.collinear_overlap_pairs >= 4);
        build_corridor_graph("top", board(), &duplicates, 0, 1).unwrap();

        let touching = [
            rectangle_obstacle("touch:a", Vec2::new(2.0, 2.0), Vec2::new(5.0, 5.0)),
            rectangle_obstacle("touch:b", Vec2::new(5.0, 5.0), Vec2::new(8.0, 8.0)),
        ];
        let (_, _, _, touching_work) = prepare_boundary_constraints(board(), &touching).unwrap();
        // Eight same-polygon corner pairs plus four cross-polygon pairs meet
        // at the common corner.
        assert!(touching_work.shared_endpoint_pairs >= 12);
        build_corridor_graph("top", board(), &touching, 0, 1).unwrap();

        let collinear = [
            rectangle_obstacle("collinear:a", Vec2::new(3.0, 2.0), Vec2::new(10.0, 5.0)),
            rectangle_obstacle("collinear:b", Vec2::new(7.0, 2.0), Vec2::new(14.0, 4.0)),
        ];
        let (_, _, _, collinear_work) = prepare_boundary_constraints(board(), &collinear).unwrap();
        assert!(collinear_work.collinear_overlap_pairs >= 1);
        assert!(collinear_work.duplicate_atomic_edges >= 1);
        build_corridor_graph("top", board(), &collinear, 0, 1).unwrap();
    }

    #[test]
    fn invalid_boundary_geometry_returns_typed_errors() {
        let duplicate_ids = [
            rectangle_obstacle("same", Vec2::new(2.0, 2.0), Vec2::new(3.0, 3.0)),
            rectangle_obstacle("same", Vec2::new(4.0, 4.0), Vec2::new(5.0, 5.0)),
        ];
        assert_eq!(
            build_corridor_graph("top", board(), &duplicate_ids, 0, 1).unwrap_err(),
            CorridorBuildError::DuplicateObstacleId {
                obstacle: "same".into()
            }
        );

        let short_edge = CorridorObstacle {
            id: "short".into(),
            polygon: vec![
                Vec2::new(2.0, 2.0),
                Vec2::new(2.0 + MIN_BOUNDARY_EDGE_LENGTH * 0.5, 2.0),
                Vec2::new(4.0, 4.0),
            ],
        };
        assert!(matches!(
            build_corridor_graph("top", board(), &[short_edge], 0, 1),
            Err(CorridorBuildError::NearDegenerateBoundaryEdge {
                owner,
                edge: 0,
                ..
            }) if owner == "component:short"
        ));

        let non_finite = CorridorObstacle {
            id: "nan".into(),
            polygon: vec![
                Vec2::new(2.0, 2.0),
                Vec2::new(f64::NAN, 3.0),
                Vec2::new(4.0, 4.0),
            ],
        };
        assert!(matches!(
            build_corridor_graph("top", board(), &[non_finite], 0, 1),
            Err(CorridorBuildError::NonFiniteBoundaryVertex {
                owner,
                vertex: 1,
            }) if owner == "component:nan"
        ));
    }

    #[test]
    fn terminal_pad_escape_also_exempts_its_own_overlapping_body() {
        let obstacles = [
            CorridorObstacle {
                id: "source".into(),
                polygon: vec![
                    Vec2::new(2.0, 4.0),
                    Vec2::new(4.0, 4.0),
                    Vec2::new(4.0, 6.0),
                    Vec2::new(2.0, 6.0),
                ],
            },
            CorridorObstacle {
                id: "pad:source:1".into(),
                polygon: vec![
                    Vec2::new(3.5, 4.5),
                    Vec2::new(4.5, 4.5),
                    Vec2::new(4.5, 5.5),
                    Vec2::new(3.5, 5.5),
                ],
            },
            CorridorObstacle {
                id: "pad:target:1".into(),
                polygon: regular_polygon(Vec2::new(18.0, 5.0), 1.0, 16),
            },
        ];
        let graph = build_corridor_graph("top", board(), &obstacles, 0, 1).unwrap();
        let reference = [Vec2::new(4.0, 5.0), Vec2::new(18.0, 5.0)];
        let embedding = embed_route_centerline_seed(
            &graph,
            RouteEmbeddingRequest {
                net: "n",
                from_component: "pad:source:1",
                to_component: "pad:target:1",
                physical_width: 0.3,
                clearance: 0.2,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
        );
        assert!(embedding.failure.is_none(), "{:?}", embedding.failure);
    }

    #[test]
    fn dense_aligned_pads_receive_one_certified_beam_per_component() {
        let dense_board = Rect {
            min: Vec2::ZERO,
            max: Vec2::new(20.0, 1_000.0),
        };
        let obstacles = (0..300)
            .map(|index| {
                let x = 8.0;
                let y = 2.0 + index as f64 * 3.0;
                rectangle_obstacle(
                    format!("pad:{index:03}"),
                    Vec2::new(x, y),
                    Vec2::new(x + 1.2, y + 1.2),
                )
            })
            .collect::<Vec<_>>();
        let basis = build_cut_basis(dense_board, &obstacles, 4, 9);

        assert!(basis.complete, "{:?}", basis.warnings);
        assert_eq!(basis.components.len(), obstacles.len());
        assert_eq!(basis.cuts.len(), obstacles.len());
        assert!(
            basis
                .cuts
                .iter()
                .all(|cut| (cut.segment[0].x - cut.segment[1].x).abs() <= EPSILON)
        );
        for (index, cut) in basis.cuts.iter().enumerate() {
            assert!(
                basis.cuts[..index].iter().all(|other| {
                    (other.segment[0].x - cut.segment[0].x).abs() > EPSILON * 16.0
                })
            );
        }
    }

    #[test]
    fn overlap_touch_and_containment_form_one_forbidden_component() {
        let obstacles = [
            rectangle_obstacle("body", Vec2::new(7.0, 2.0), Vec2::new(13.0, 8.0)),
            rectangle_obstacle("contained-pad", Vec2::new(9.0, 4.0), Vec2::new(11.0, 6.0)),
            rectangle_obstacle("touching-pad", Vec2::new(13.0, 4.0), Vec2::new(15.0, 6.0)),
        ];
        let basis = build_cut_basis(board(), &obstacles, 0, 1);

        assert!(basis.complete, "{:?}", basis.warnings);
        assert_eq!(basis.components.len(), 1);
        assert_eq!(
            basis.components[0].members,
            vec![
                "body".to_owned(),
                "contained-pad".to_owned(),
                "touching-pad".to_owned()
            ]
        );
        assert_eq!(basis.cuts.len(), 1);
        assert_eq!(
            basis.components[0].generator.as_deref(),
            Some(basis.cuts[0].generator.as_str())
        );
    }

    #[test]
    fn singleton_generator_namespace_cannot_collide_with_a_raw_obstacle_id() {
        let obstacles = [
            rectangle_obstacle("x", Vec2::new(2.0, 2.0), Vec2::new(4.0, 4.0)),
            rectangle_obstacle("generator[1:x]", Vec2::new(14.0, 2.0), Vec2::new(16.0, 4.0)),
        ];
        let basis = build_cut_basis(board(), &obstacles, 0, 1);
        let generators = basis
            .cuts
            .iter()
            .map(|cut| cut.generator.as_str())
            .collect::<BTreeSet<_>>();

        assert!(basis.complete, "{:?}", basis.warnings);
        assert_eq!(generators.len(), 2);
        assert!(generators.contains("generator[1:x]"));
        assert!(generators.contains("generator[14:generator[1:x]]"));
    }

    #[test]
    fn board_attached_forbidden_space_is_an_exterior_root() {
        let obstacles = [
            rectangle_obstacle("interior", Vec2::new(8.0, 2.0), Vec2::new(12.0, 4.0)),
            rectangle_obstacle("edge-root", Vec2::new(6.0, 7.0), Vec2::new(14.0, 10.0)),
        ];
        let basis = build_cut_basis(board(), &obstacles, 0, 1);

        assert!(basis.complete, "{:?}", basis.warnings);
        let root = basis
            .components
            .iter()
            .find(|component| component.members.len() == 1 && component.members[0] == "edge-root")
            .unwrap();
        assert!(root.board_attached);
        assert_eq!(root.board_contacts, vec![BoardBoundarySide::Bottom]);
        assert!(root.generator.is_none());
        assert_eq!(basis.cuts.len(), 1);
        assert_eq!(
            basis.cuts[0].target,
            TopologyCutTarget::Component {
                component: root.id.clone()
            }
        );
    }

    #[test]
    fn small_motion_preserves_structure_but_invalidates_the_exact_route_epoch() {
        let before = [
            rectangle_obstacle("upper", Vec2::new(3.0, 2.0), Vec2::new(5.0, 4.0)),
            rectangle_obstacle("lower", Vec2::new(12.0, 6.0), Vec2::new(14.0, 8.0)),
        ];
        let mut after = before.clone();
        for point in &mut after[0].polygon {
            *point = add(*point, Vec2::new(0.01, 0.01));
        }
        let before_basis = build_cut_basis(board(), &before, 1, 1);
        let after_basis = build_cut_basis(board(), &after, 2, 2);

        assert!(before_basis.complete, "{:?}", before_basis.warnings);
        assert!(after_basis.complete, "{:?}", after_basis.warnings);
        assert_eq!(
            before_basis.structural_fingerprint,
            after_basis.structural_fingerprint
        );
        assert_ne!(before_basis.fingerprint, after_basis.fingerprint);
        let upper_component = before_basis
            .components
            .iter()
            .find(|component| component.members.iter().any(|member| member == "upper"))
            .unwrap()
            .id
            .clone();
        assert_ne!(
            before_basis
                .cuts
                .iter()
                .find(|cut| cut.source_component == upper_component)
                .unwrap()
                .segment,
            after_basis
                .cuts
                .iter()
                .find(|cut| cut.source_component == upper_component)
                .unwrap()
                .segment
        );
    }

    #[test]
    fn reparenting_changes_the_cut_forest_epoch() {
        let source = rectangle_obstacle("source", Vec2::new(8.0, 1.0), Vec2::new(12.0, 3.0));
        let beneath = rectangle_obstacle("beneath", Vec2::new(6.0, 6.0), Vec2::new(14.0, 8.0));
        let moved_aside = rectangle_obstacle("beneath", Vec2::new(15.0, 6.0), Vec2::new(18.0, 8.0));
        let before = build_cut_basis(board(), &[source.clone(), beneath], 1, 1);
        let after = build_cut_basis(board(), &[source, moved_aside], 2, 2);

        assert!(before.complete, "{:?}", before.warnings);
        assert!(after.complete, "{:?}", after.warnings);
        let source_component = before
            .components
            .iter()
            .find(|component| component.members.iter().any(|member| member == "source"))
            .unwrap()
            .id
            .clone();
        let before_source = before
            .cuts
            .iter()
            .find(|cut| cut.source_component == source_component)
            .unwrap();
        let after_source = after
            .cuts
            .iter()
            .find(|cut| cut.source_component == source_component)
            .unwrap();
        assert!(matches!(
            &before_source.target,
            TopologyCutTarget::Component { .. }
        ));
        assert_eq!(&after_source.target, &TopologyCutTarget::BoardBottom);
        assert_ne!(before.structural_fingerprint, after.structural_fingerprint);
        assert_ne!(before.fingerprint, after.fingerprint);
    }

    #[test]
    fn exchanging_vertical_anchor_order_changes_the_cut_forest_epoch() {
        let before = [
            rectangle_obstacle("a", Vec2::new(2.0, 2.0), Vec2::new(4.0, 4.0)),
            rectangle_obstacle("b", Vec2::new(14.0, 2.0), Vec2::new(16.0, 4.0)),
        ];
        let after = [
            rectangle_obstacle("a", Vec2::new(14.0, 2.0), Vec2::new(16.0, 4.0)),
            rectangle_obstacle("b", Vec2::new(2.0, 2.0), Vec2::new(4.0, 4.0)),
        ];
        let before = build_cut_basis(board(), &before, 1, 1);
        let after = build_cut_basis(board(), &after, 2, 2);

        assert!(before.complete, "{:?}", before.warnings);
        assert!(after.complete, "{:?}", after.warnings);
        assert_ne!(before.structural_fingerprint, after.structural_fingerprint);
        assert_ne!(before.fingerprint, after.fingerprint);
    }

    #[test]
    fn changing_board_contact_sides_changes_the_cut_forest_epoch() {
        let bottom_only =
            rectangle_obstacle("edge-root", Vec2::new(16.0, 8.0), Vec2::new(19.0, 10.0));
        let bottom_and_right =
            rectangle_obstacle("edge-root", Vec2::new(17.0, 8.0), Vec2::new(20.0, 10.0));
        let before = build_cut_basis(board(), &[bottom_only], 1, 1);
        let after = build_cut_basis(board(), &[bottom_and_right], 2, 2);

        assert!(before.complete, "{:?}", before.warnings);
        assert!(after.complete, "{:?}", after.warnings);
        assert_eq!(
            before.components[0].board_contacts,
            vec![BoardBoundarySide::Bottom]
        );
        assert_eq!(
            after.components[0].board_contacts,
            vec![BoardBoundarySide::Right, BoardBoundarySide::Bottom]
        );
        assert_ne!(before.structural_fingerprint, after.structural_fingerprint);
        assert_ne!(before.fingerprint, after.fingerprint);
    }

    #[test]
    fn beam_sweeping_a_fixed_route_changes_only_the_exact_basis_epoch() {
        let before_obstacle =
            rectangle_obstacle("moving", Vec2::new(8.0, 2.0), Vec2::new(10.0, 6.0));
        let mut after_obstacle = before_obstacle.clone();
        for point in &mut after_obstacle.polygon {
            point.x += 0.2;
        }
        let before = build_cut_basis(board(), &[before_obstacle], 1, 1);
        let after = build_cut_basis(board(), &[after_obstacle], 2, 2);
        let before_x = before.cuts[0].segment[0].x;
        let after_x = after.cuts[0].segment[0].x;
        let between = (before_x + after_x) * 0.5;
        let route = [Vec2::new(between, 8.0), Vec2::new(19.0, 8.0)];
        let before_word = polyline_signature(&route, &before, &[]);
        let after_word = polyline_signature(&route, &after, &[]);

        assert!(before.complete, "{:?}", before.warnings);
        assert!(after.complete, "{:?}", after.warnings);
        assert_eq!(before.structural_fingerprint, after.structural_fingerprint);
        assert_ne!(before.fingerprint, after.fingerprint);
        assert_ne!(before_word.word, after_word.word);
        assert!(!before_word.ambiguous);
        assert!(!after_word.ambiguous);
    }

    #[test]
    fn endpoint_exemption_applies_to_its_connected_forbidden_component() {
        let obstacles = [
            rectangle_obstacle("body", Vec2::new(1.0, 3.0), Vec2::new(4.0, 7.0)),
            rectangle_obstacle("pad:body:1", Vec2::new(3.5, 4.0), Vec2::new(4.5, 6.0)),
        ];
        let basis = build_cut_basis(board(), &obstacles, 0, 1);
        let route = [Vec2::new(0.5, 8.0), Vec2::new(19.5, 8.0)];
        let without_exemption = polyline_signature(&route, &basis, &[]);
        let with_exemption = polyline_signature(
            &route,
            &basis,
            &route_endpoint_exemptions("pad:body:1", "remote"),
        );

        assert!(basis.complete, "{:?}", basis.warnings);
        assert!(!without_exemption.word.crossings().is_empty() || without_exemption.ambiguous);
        assert!(with_exemption.word.crossings().is_empty());
        assert!(!with_exemption.ambiguous);
    }

    #[test]
    fn cut_word_distinguishes_the_two_sides_of_an_obstacle() {
        let obstacle = CorridorObstacle {
            id: "middle".into(),
            polygon: vec![
                Vec2::new(8.0, 2.0),
                Vec2::new(12.0, 2.0),
                Vec2::new(12.0, 8.0),
                Vec2::new(8.0, 8.0),
            ],
        };
        let graph = build_corridor_graph("top", board(), &[obstacle], 0, 1).unwrap();
        assert!(graph.cut_basis.complete);
        assert_eq!(graph.cut_basis.cuts.len(), 1);
        let above = polyline_signature(
            &[Vec2::new(1.0, 1.0), Vec2::new(19.0, 1.0)],
            &graph.cut_basis,
            &[],
        );
        let below = polyline_signature(
            &[Vec2::new(1.0, 9.0), Vec2::new(19.0, 9.0)],
            &graph.cut_basis,
            &[],
        );
        assert!(!above.ambiguous);
        assert!(!below.ambiguous);
        assert_ne!(above.word, below.word);
    }

    #[test]
    fn exact_certificate_rejects_a_route_too_close_to_board_edge() {
        let graph = build_corridor_graph("top", board(), &[], 0, 1).unwrap();
        let reference = [Vec2::new(1.0, 0.1), Vec2::new(19.0, 0.1)];
        let embedding = embed_route(
            &graph,
            RouteEmbeddingRequest {
                net: "edge",
                from_component: "source",
                to_component: "target",
                physical_width: 0.4,
                clearance: 0.3,
                from: reference[0],
                from_toward: reference[1],
                to: reference[1],
                to_toward: reference[0],
                reference: &reference,
                target_homotopy: None,
                persistent_basis_matches: false,
            },
        );
        assert!(embedding.failure.is_some());
        assert!(!embedding.clearance.certified);
        let repair = embedding.repair.as_ref().unwrap();
        assert_eq!(repair.outcome, RepairOutcome::SearchExhausted);
        assert!(repair.search_complete);
    }
}

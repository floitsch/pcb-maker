// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{
    Problem, Vec2,
    geometry::rotate_degrees,
    model::{Movement, PinRef, Rotation},
    topology::{CrossingDirection, CutCrossing, HomotopyWord, RouteClass, TerminalSector},
};
use layout_trace_routing::{
    context_correction::{
        ContextCorrectionLimits, ContextCorrectionOutcome, select_context_correction,
    },
    corridor::{
        AlternativeSearchTermination, CorridorGraph, EmbeddingPurpose, RouteEmbeddingRequest,
        RouteRepairWorkBudget, certify_polyline_clearance, embed_route,
        enumerate_route_class_alternatives_with_limit,
        enumerate_route_class_alternatives_with_limit_and_work_budget,
    },
    family_assignment::{
        FamilyAssignmentLimits, FamilyAssignmentOutcome, FamilyClearancePolicy,
        FamilySelectionRepairObligation, select_route_families,
        select_route_families_with_clearance_policy,
    },
    family_ir::{
        CorridorEpoch, CutDirection, CutEvent, CutWordCertificate, EXACT_VIA_CLEARANCE_MODEL,
        FIXED_CONTEXT_ANALYSIS_METHOD, FIXED_WITNESS_NO_SCALAR_CAPACITY_MODEL,
        FIXED_WITNESS_PAIR_ANALYSIS_METHOD, FamilyConflictKind, FamilyGenerationCertificate,
        FamilyPoint, FixedContextCertificate, MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION,
        PairAnalysisCertificate, PassageCapacityCertificate, PassageEnforcementMode,
        RequiredNullable, RouteFamily, RouteFamilyFeasibilityMode, RouteFamilyPortfolio,
        RouteFamilyRun, RouteFamilyVariable, RouteTransition, RouteTransitionKind,
        RouteViaGeometry, RouteWitness,
    },
    family_portfolio::{
        RouteFamilyCandidateRejection, RouteFamilyFixedContextInput, RouteFamilyPlanningOutcome,
        RouteFamilyPortfolioBuildOutcome, RouteFamilyPortfolioInput, RouteFamilyProductionWork,
        RouteFamilyRawFrontierEvidence, RouteFamilyRouteInput, RouteFamilyUnsupportedReason,
        build_and_select_route_families, build_and_select_route_families_with_work_budget,
    },
};
use pcb_core::{Bounds as EngineBounds, Frame, Mobility as EngineMobility, Vec2 as EngineVec2};
use pcb_engine::{
    Backend, CopperAttachmentInput, CopperBodyInput, CopperPolylineInput, CopperPolylineMobility,
    CopperRepairRequest, CopperSegmentBodyPair, CopperSeparationPair, CpuReferenceBackend, NoField,
    SolverConfig, compile_copper_repair,
};
use pcb_validate::{
    CANDIDATE_SCHEMA_VERSION, CandidateArtifact, ExactValidationAssessment, SolvedComponent,
    SolvedRouteGraph, SolvedRouteNode, SolvedRouteNodeKind, SolvedTrace, SolvedVia,
    validate_candidate,
};
use serde::{Deserialize, Serialize};

use crate::{
    CorridorAnalysisConfig, CorridorAnalysisResult, analyze_problem_corridors_at_poses,
    corridor_adapter::{compile_corridor_branches, component_pose_map, terminal_attachment},
};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteFamilyAnalysisConfig {
    pub corridor: CorridorAnalysisConfig,
    pub portfolio_id: String,
    pub geometry_revision: u64,
    pub maximum_assignment_nodes: u64,
    #[serde(default = "default_maximum_repair_component_branches")]
    pub maximum_repair_component_branches: usize,
    #[serde(default)]
    pub route_repair_work_budget: Option<RouteRepairWorkBudget>,
    #[serde(default)]
    pub context_correction_limits: ContextCorrectionLimits,
    #[serde(default)]
    pub continuous_obligation_repair: ContinuousObligationRepairConfig,
    #[serde(default)]
    pub seeded_via_families: SeededViaFamilyConfig,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeededViaFamilyConfig {
    pub enabled: bool,
    pub maximum_via_sites: usize,
    pub maximum_run_alternatives: usize,
    pub maximum_families_per_variable: usize,
    #[serde(default = "default_seeded_via_clearance_policy")]
    pub clearance_policy: FamilyClearancePolicy,
}

const fn default_seeded_via_clearance_policy() -> FamilyClearancePolicy {
    FamilyClearancePolicy::DiscreteSeparation
}

impl Default for SeededViaFamilyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            maximum_via_sites: 5,
            maximum_run_alternatives: 3,
            maximum_families_per_variable: 8,
            clearance_policy: default_seeded_via_clearance_policy(),
        }
    }
}

impl SeededViaFamilyConfig {
    fn check(self) -> Result<(), String> {
        if !(1..=16).contains(&self.maximum_via_sites) {
            return Err("seeded-via maximum_via_sites must be between 1 and 16".into());
        }
        if !(1..=16).contains(&self.maximum_run_alternatives) {
            return Err("seeded-via maximum_run_alternatives must be between 1 and 16".into());
        }
        if !(1..=64).contains(&self.maximum_families_per_variable) {
            return Err("seeded-via maximum_families_per_variable must be between 1 and 64".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuousObligationRepairConfig {
    pub steps: usize,
    pub projection_iterations: usize,
    pub maximum_segment_pairs: usize,
    #[serde(default = "default_include_body_contacts")]
    pub include_body_contacts: bool,
    #[serde(default = "default_numerical_clearance_margin_nm")]
    pub numerical_clearance_margin_nm: u32,
}

const fn default_include_body_contacts() -> bool {
    true
}

const fn default_numerical_clearance_margin_nm() -> u32 {
    1_000
}

impl Default for ContinuousObligationRepairConfig {
    fn default() -> Self {
        Self {
            steps: 8,
            projection_iterations: 16,
            maximum_segment_pairs: 100_000,
            include_body_contacts: default_include_body_contacts(),
            numerical_clearance_margin_nm: default_numerical_clearance_margin_nm(),
        }
    }
}

impl ContinuousObligationRepairConfig {
    fn check(self) -> Result<(), String> {
        if self.steps == 0 {
            return Err("continuous obligation repair steps must be positive".into());
        }
        if self.projection_iterations == 0 {
            return Err(
                "continuous obligation repair projection_iterations must be positive".into(),
            );
        }
        if self.maximum_segment_pairs == 0 {
            return Err(
                "continuous obligation repair maximum_segment_pairs must be positive".into(),
            );
        }
        if self.numerical_clearance_margin_nm > 1_000_000 {
            return Err(
                "continuous obligation repair numerical_clearance_margin_nm must not exceed 1000000"
                    .into(),
            );
        }
        Ok(())
    }
}

const fn default_maximum_repair_component_branches() -> usize {
    6
}

impl RouteFamilyAnalysisConfig {
    pub fn check(&self) -> Result<(), String> {
        self.corridor.check()?;
        if self.portfolio_id.is_empty() {
            return Err("route-family portfolio_id must not be empty".into());
        }
        if self.maximum_assignment_nodes == 0 {
            return Err("route-family maximum_assignment_nodes must be positive".into());
        }
        if !(2..=16).contains(&self.maximum_repair_component_branches) {
            return Err(
                "route-family maximum_repair_component_branches must be between 2 and 16".into(),
            );
        }
        self.continuous_obligation_repair.check()?;
        self.seeded_via_families.check()?;
        Ok(())
    }
}

impl Default for RouteFamilyAnalysisConfig {
    fn default() -> Self {
        Self {
            corridor: CorridorAnalysisConfig::default(),
            portfolio_id: "pcb-maker-route-families".into(),
            geometry_revision: 1,
            maximum_assignment_nodes: 100_000,
            maximum_repair_component_branches: default_maximum_repair_component_branches(),
            route_repair_work_budget: None,
            context_correction_limits: ContextCorrectionLimits::default(),
            continuous_obligation_repair: ContinuousObligationRepairConfig::default(),
            seeded_via_families: SeededViaFamilyConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteFamilyAnalysisStatus {
    Selected,
    ExactRejected,
    BoundedInfeasible,
    AssignmentBudgetExhausted,
    PortfolioUnavailable,
    AssignmentRejected,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteFamilyAnalysisEvidence {
    pub strategy: String,
    pub config: RouteFamilyAnalysisConfig,
    pub status: RouteFamilyAnalysisStatus,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteFamilyAnalysisResult {
    pub corridor: CorridorAnalysisResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<CandidateArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ExactValidationAssessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portfolio: Option<RouteFamilyPortfolio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignment: Option<FamilyAssignmentOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuous_repair: Option<ContinuousObligationRepairResult>,
    pub production_work: RouteFamilyProductionWork,
    pub evidence: RouteFamilyAnalysisEvidence,
}

/// Exact-gated lowering of an already generated and validated route-family
/// portfolio. This keeps external/new family generators behind the same
/// durable candidate boundary as the in-process same-layer producer.
#[derive(Clone, Debug, Serialize)]
pub struct RouteFamilyPortfolioMaterializationResult {
    pub portfolio: RouteFamilyPortfolio,
    pub assignment: FamilyAssignmentOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<CandidateArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ExactValidationAssessment>,
}

impl RouteFamilyPortfolioMaterializationResult {
    pub fn complete(&self) -> bool {
        self.validation
            .as_ref()
            .is_some_and(|validation| validation.complete)
    }
}

pub fn parse_route_family_portfolio(source: &str) -> Result<RouteFamilyPortfolio, String> {
    layout_trace_routing::family_ir::parse_and_validate_route_family_portfolio(source)
        .map_err(|error| error.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuousObligationRepairStatus {
    ExactComplete,
    Improved,
    RolledBack,
    NoObligations,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContinuousObligationRepairResult {
    pub status: ContinuousObligationRepairStatus,
    /// Transactional output: this is the proposal only when it passed the
    /// exact improvement gate, otherwise it is the unchanged input candidate.
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_validation: Option<ExactValidationAssessment>,
    pub obligation_count: usize,
    pub segment_pairs: usize,
    pub segment_body_pairs: usize,
    pub frames: Vec<Frame>,
    pub detail: String,
}

impl ContinuousObligationRepairResult {
    pub fn accepted(&self) -> bool {
        matches!(
            self.status,
            ContinuousObligationRepairStatus::ExactComplete
                | ContinuousObligationRepairStatus::Improved
        )
    }
}

impl RouteFamilyAnalysisResult {
    pub fn complete(&self) -> bool {
        self.evidence.status == RouteFamilyAnalysisStatus::Selected
            && self
                .validation
                .as_ref()
                .is_some_and(|result| result.complete)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteFamilyRepairStatus {
    ExactComplete,
    Improved,
    NoImprovement,
    NoEligibleConflict,
    BoundedInfeasible,
    AssignmentBudgetExhausted,
    PortfolioUnavailable,
    AssignmentRejected,
    ExactRejected,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteFamilyRepairEvidence {
    pub strategy: String,
    pub config: RouteFamilyAnalysisConfig,
    pub status: RouteFamilyRepairStatus,
    pub selected_branches: Vec<String>,
    pub component_edges: usize,
    pub original_selected_conflicts: usize,
    pub proposed_selected_conflicts: Option<usize>,
    pub original_findings: usize,
    pub proposed_findings: Option<usize>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteFamilyRepairResult {
    pub original_validation: ExactValidationAssessment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corridor: Option<CorridorAnalysisResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<CandidateArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ExactValidationAssessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portfolio: Option<RouteFamilyPortfolio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignment: Option<FamilyAssignmentOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_correction: Option<ContextCorrectionOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuous_repair: Option<ContinuousObligationRepairResult>,
    pub production_work: RouteFamilyProductionWork,
    pub evidence: RouteFamilyRepairEvidence,
}

impl RouteFamilyRepairResult {
    pub fn accepted(&self) -> bool {
        matches!(
            self.evidence.status,
            RouteFamilyRepairStatus::ExactComplete | RouteFamilyRepairStatus::Improved
        )
    }

    pub fn complete(&self) -> bool {
        self.evidence.status == RouteFamilyRepairStatus::ExactComplete
    }
}

struct OwnedRouteFamilyInput {
    branch: String,
    electrical_net: String,
    layer: String,
    end_layer: String,
    allowed_layers: Vec<String>,
    physical_width: f64,
    tension_weight: f64,
    clearance: f64,
    from_obstacle: String,
    to_obstacle: String,
    from: Vec2,
    to: Vec2,
    reference: Vec<Vec2>,
    from_terminal: PinRef,
    to_terminal: PinRef,
    from_node: String,
    to_node: String,
}

fn plan_route_families(
    input: RouteFamilyPortfolioInput<'_>,
    config: &RouteFamilyAnalysisConfig,
) -> RouteFamilyPlanningOutcome {
    let assignment_limits = FamilyAssignmentLimits {
        max_nodes: config.maximum_assignment_nodes,
    };
    if let Some(work_budget) = config.route_repair_work_budget {
        build_and_select_route_families_with_work_budget(input, assignment_limits, work_budget)
    } else {
        build_and_select_route_families(input, assignment_limits)
    }
}

fn terminal_supported_layers(
    problem: &Problem,
    terminal: &PinRef,
    allowed_layers: &[String],
) -> Result<Vec<String>, String> {
    let component = problem
        .components
        .iter()
        .find(|component| component.id == terminal.component)
        .ok_or_else(|| format!("unknown terminal component {}", terminal.component))?;
    let pin = component
        .pins
        .iter()
        .find(|pin| pin.id == terminal.pin)
        .ok_or_else(|| format!("unknown terminal {}.{}", terminal.component, terminal.pin))?;
    if pin.pads.is_empty() {
        return Ok(allowed_layers.to_vec());
    }
    Ok(allowed_layers
        .iter()
        .filter(|layer| pin.pads.iter().any(|pad| &pad.layer == *layer))
        .cloned()
        .collect())
}

fn route_endpoint_layers(
    problem: &Problem,
    from: &PinRef,
    to: &PinRef,
    preferred_layer: &str,
    allowed_layers: &[String],
    allow_via: bool,
) -> Result<(String, String), String> {
    let from_layers = terminal_supported_layers(problem, from, allowed_layers)?;
    let to_layers = terminal_supported_layers(problem, to, allowed_layers)?;
    if let Some(layer) = allowed_layers
        .iter()
        .find(|layer| from_layers.contains(layer) && to_layers.contains(layer))
    {
        return Ok((layer.clone(), layer.clone()));
    }
    if !allow_via {
        return Ok((preferred_layer.to_owned(), preferred_layer.to_owned()));
    }
    let from_layer = from_layers.first().cloned().ok_or_else(|| {
        format!(
            "terminal {}.{} has no pad on an allowed layer",
            from.component, from.pin
        )
    })?;
    let to_layer = to_layers.first().cloned().ok_or_else(|| {
        format!(
            "terminal {}.{} has no pad on an allowed layer",
            to.component, to.pin
        )
    })?;
    Ok((from_layer, to_layer))
}

#[derive(Clone)]
struct SeededRunPattern {
    layer: String,
    from_component: String,
    to_component: String,
    reference: Vec<Vec2>,
}

#[derive(Clone)]
struct SeededFamilyPattern {
    ordinal: usize,
    via: Option<(Vec2, f64)>,
    runs: Vec<SeededRunPattern>,
}

enum SeededRunSearchFailure {
    Budget(AlternativeSearchTermination),
    Rejected(String),
}

fn polyline_length(points: &[Vec2]) -> f64 {
    points
        .windows(2)
        .map(|pair| (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y))
        .sum()
}

fn split_polyline_at_distance(
    reference: &[Vec2],
    target: f64,
) -> Option<(Vec2, Vec<Vec2>, Vec<Vec2>)> {
    let mut traversed = 0.0;
    for (segment_index, pair) in reference.windows(2).enumerate() {
        let length = (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
        if length <= 1.0e-12 {
            continue;
        }
        if traversed + length + 1.0e-12 < target {
            traversed += length;
            continue;
        }
        let fraction = ((target - traversed) / length).clamp(0.0, 1.0);
        let point = Vec2::new(
            pair[0].x + (pair[1].x - pair[0].x) * fraction,
            pair[0].y + (pair[1].y - pair[0].y) * fraction,
        );
        let mut first = reference[..=segment_index].to_vec();
        if first.last().is_none_or(|last| *last != point) {
            first.push(point);
        }
        let mut second = vec![point];
        if point == pair[1] {
            second.extend_from_slice(&reference[segment_index + 2..]);
        } else {
            second.extend_from_slice(&reference[segment_index + 1..]);
        }
        if first.len() >= 2 && second.len() >= 2 {
            return Some((point, first, second));
        }
        return None;
    }
    None
}

fn seeded_via_splits(
    reference: &[Vec2],
    maximum_sites: usize,
) -> Vec<(Vec2, Vec<Vec2>, Vec<Vec2>)> {
    let total = polyline_length(reference);
    if total <= 1.0e-9 {
        return Vec::new();
    }
    let mut distances = Vec::new();
    let mut traversed = 0.0;
    for pair in reference.windows(2).take(reference.len().saturating_sub(2)) {
        traversed += (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
        if traversed > 1.0e-9 && traversed < total - 1.0e-9 {
            distances.push(traversed);
        }
    }
    for index in 1..=maximum_sites {
        distances.push(total * index as f64 / (maximum_sites + 1) as f64);
    }
    let mut selected = Vec::new();
    for distance in distances {
        if selected
            .iter()
            .any(|existing: &f64| (*existing - distance).abs() <= 1.0e-9 * total.max(1.0))
        {
            continue;
        }
        selected.push(distance);
        if selected.len() == maximum_sites {
            break;
        }
    }
    selected.sort_by(f64::total_cmp);
    selected
        .into_iter()
        .filter_map(|distance| split_polyline_at_distance(reference, distance))
        .collect()
}

fn passage_keys_for_embedding(
    graph: &CorridorGraph,
    embedding: &layout_trace_routing::corridor::RouteCorridorEmbedding,
) -> Option<Vec<String>> {
    let mut keys = BTreeSet::new();
    for passage in &embedding.semantic_passages {
        keys.insert(graph.semantic.passages.get(*passage)?.key.clone());
    }
    Some(keys.into_iter().collect())
}

fn seeded_embedding_request<'a>(
    branch: &'a str,
    run: &'a SeededRunPattern,
    physical_width: f64,
    clearance: f64,
    reference: &'a [Vec2],
    target_homotopy: Option<&'a HomotopyWord>,
) -> RouteEmbeddingRequest<'a> {
    RouteEmbeddingRequest {
        net: branch,
        from_component: &run.from_component,
        to_component: &run.to_component,
        physical_width,
        clearance,
        from: reference[0],
        from_toward: reference[1],
        to: reference[reference.len() - 1],
        to_toward: reference[reference.len() - 2],
        reference,
        target_homotopy,
        persistent_basis_matches: target_homotopy.is_some(),
    }
}

fn search_seeded_run(
    route: &OwnedRouteFamilyInput,
    pattern_ordinal: usize,
    run_ordinal: usize,
    run: &SeededRunPattern,
    graph: &CorridorGraph,
    config: &RouteFamilyAnalysisConfig,
    work: &mut RouteFamilyProductionWork,
) -> Result<Vec<layout_trace_routing::corridor::RouteCorridorEmbedding>, SeededRunSearchFailure> {
    let branch = route.branch.as_str();
    let maximum_alternatives = config.seeded_via_families.maximum_run_alternatives;
    work.family_searches += 1;
    work.family_search_cache_lookups += 1;
    work.family_search_new_executions += 1;
    work.raw_candidates_requested += maximum_alternatives as u64;
    let request = seeded_embedding_request(
        branch,
        run,
        route.physical_width,
        route.clearance,
        &run.reference,
        None,
    );
    let evidence = if let Some(work_budget) = config.route_repair_work_budget {
        enumerate_route_class_alternatives_with_limit_and_work_budget(
            graph,
            request,
            maximum_alternatives,
            work_budget,
        )
        .map(|bounded| bounded.evidence)
    } else {
        enumerate_route_class_alternatives_with_limit(graph, request, maximum_alternatives)
    }
    .map_err(|error| SeededRunSearchFailure::Rejected(error.to_string()))?;
    work.visibility_states_new += evidence.explored_states as u64;
    work.visibility_states += evidence.explored_states as u64;
    work.visibility_nodes += evidence.nodes.len() as u64;
    work.visibility_edge_candidates += evidence.evaluated_edge_candidates as u64;
    work.visibility_edges += evidence.edges.len() as u64;
    work.visibility_edges_rejected += evidence.rejected_edges as u64;
    work.visibility_geometry_units += evidence.visibility_geometry_units as u64;
    if matches!(
        evidence.termination,
        AlternativeSearchTermination::StateLimitReached
            | AlternativeSearchTermination::VisibilityGraphLimitReached
    ) {
        return Err(SeededRunSearchFailure::Budget(evidence.termination));
    }
    let mut certified = Vec::new();
    for (alternative_index, alternative) in evidence.alternatives.iter().enumerate() {
        work.raw_candidates_examined += 1;
        work.alternatives_reembedded += 1;
        let embedding = embed_route(
            graph,
            seeded_embedding_request(
                branch,
                run,
                route.physical_width,
                route.clearance,
                &alternative.polyline,
                Some(&alternative.homotopy),
            ),
        );
        let strict = embedding.failure.is_none()
            && embedding.purpose == EmbeddingPurpose::FullWidthCertificate
            && embedding.layer == run.layer
            && embedding.corridor_revision == graph.revision
            && embedding.placement_revision == graph.placement_revision
            && embedding.route_class_verified
            && embedding.epoch_homotopy_verified
            && embedding.clearance.certified
            && embedding.polyline.len() >= 2
            && embedding.polyline.first() == run.reference.first()
            && embedding.polyline.last() == run.reference.last()
            && passage_keys_for_embedding(graph, &embedding).is_some();
        if strict {
            certified.push(embedding);
        } else {
            work.raw_candidates_rejected += 1;
            work.rejection_evidence.push(RouteFamilyCandidateRejection {
                branch: branch.into(),
                raw_ordinal: alternative_index + 1,
                family_id: format!(
                    "seeded-via:{branch}:pattern-{pattern_ordinal}:run-{run_ordinal}"
                ),
                reason: "run_reembedding_not_strict".into(),
                detail: "one bounded run alternative failed current-epoch full-width reembedding"
                    .into(),
                projection: None,
            });
        }
    }
    if certified.is_empty() {
        Err(SeededRunSearchFailure::Rejected(format!(
            "pattern {pattern_ordinal} run {run_ordinal} produced no strict family"
        )))
    } else {
        Ok(certified)
    }
}

fn seeded_patterns(
    problem: &Problem,
    route: &OwnedRouteFamilyInput,
    graphs: &BTreeMap<&str, &CorridorGraph>,
    config: SeededViaFamilyConfig,
    work: &mut RouteFamilyProductionWork,
) -> Result<Vec<SeededFamilyPattern>, String> {
    if route.layer == route.end_layer {
        return Ok(vec![SeededFamilyPattern {
            ordinal: 0,
            via: None,
            runs: vec![SeededRunPattern {
                layer: route.layer.clone(),
                from_component: route.from_obstacle.clone(),
                to_component: route.to_obstacle.clone(),
                reference: route.reference.clone(),
            }],
        }]);
    }
    if !route.allowed_layers.contains(&route.layer)
        || !route.allowed_layers.contains(&route.end_layer)
    {
        return Err("selected endpoint layers are absent from the route's allowed layers".into());
    }
    let from_graph = graphs
        .get(route.layer.as_str())
        .ok_or_else(|| format!("missing corridor epoch for layer {}", route.layer))?;
    let to_graph = graphs
        .get(route.end_layer.as_str())
        .ok_or_else(|| format!("missing corridor epoch for layer {}", route.end_layer))?;
    let required_radius = problem.rules.via_diameter * 0.5 + problem.rules.clearance;
    let mut patterns = Vec::new();
    for (site_index, (site, first, second)) in
        seeded_via_splits(&route.reference, config.maximum_via_sites)
            .into_iter()
            .enumerate()
    {
        let point_witness = [site, site];
        let first_clearance =
            certify_polyline_clearance(from_graph, &point_witness, required_radius, "", "");
        let second_clearance =
            certify_polyline_clearance(to_graph, &point_witness, required_radius, "", "");
        if !first_clearance.certified || !second_clearance.certified {
            work.raw_candidates_rejected += 1;
            work.rejection_evidence.push(RouteFamilyCandidateRejection {
                branch: route.branch.clone(),
                raw_ordinal: site_index + 1,
                family_id: format!("seeded-via:{}:site-{site_index}", route.branch),
                reason: "via_clearance_not_certified".into(),
                detail: format!(
                    "via site ({:.6},{:.6}) needs radius {:.6}; layer minima are {:.6} and {:.6}",
                    site.x,
                    site.y,
                    required_radius,
                    first_clearance.minimum_clearance,
                    second_clearance.minimum_clearance,
                ),
                projection: None,
            });
            continue;
        }
        patterns.push(SeededFamilyPattern {
            ordinal: site_index,
            via: Some((
                site,
                first_clearance
                    .minimum_clearance
                    .min(second_clearance.minimum_clearance),
            )),
            runs: vec![
                SeededRunPattern {
                    layer: route.layer.clone(),
                    from_component: route.from_obstacle.clone(),
                    to_component: String::new(),
                    reference: first,
                },
                SeededRunPattern {
                    layer: route.end_layer.clone(),
                    from_component: String::new(),
                    to_component: route.to_obstacle.clone(),
                    reference: second,
                },
            ],
        });
    }
    if patterns.is_empty() {
        Err("no bounded seeded via site has exact annulus clearance on both layers".into())
    } else {
        Ok(patterns)
    }
}

fn seeded_via_unavailable(
    route: Option<String>,
    detail: impl Into<String>,
    work: RouteFamilyProductionWork,
) -> RouteFamilyPlanningOutcome {
    RouteFamilyPlanningOutcome::PortfolioUnavailable {
        outcome: RouteFamilyPortfolioBuildOutcome::Unsupported {
            route,
            reason: RouteFamilyUnsupportedReason::AlternativeCouldNotBeCertified,
            detail: detail.into(),
            work,
        },
    }
}

fn plan_seeded_via_route_families(
    problem: &Problem,
    routes: &[OwnedRouteFamilyInput],
    corridors: &[CorridorGraph],
    config: &RouteFamilyAnalysisConfig,
) -> RouteFamilyPlanningOutcome {
    let mut work = RouteFamilyProductionWork {
        portfolio_builds: 1,
        ..RouteFamilyProductionWork::default()
    };
    let graphs = corridors
        .iter()
        .map(|graph| (graph.layer.as_str(), graph))
        .collect::<BTreeMap<_, _>>();
    let mut variables = Vec::new();
    let mut passage_minima = BTreeMap::<(String, String), f64>::new();
    for route in routes {
        let patterns = match seeded_patterns(
            problem,
            route,
            &graphs,
            config.seeded_via_families,
            &mut work,
        ) {
            Ok(patterns) => patterns,
            Err(detail) => {
                return seeded_via_unavailable(Some(route.branch.clone()), detail, work);
            }
        };
        let mut pattern_frontiers = Vec::new();
        for pattern in patterns {
            let mut combinations = vec![Vec::new()];
            let mut rejected_pattern = None;
            for (run_ordinal, run) in pattern.runs.iter().enumerate() {
                let graph = graphs[run.layer.as_str()];
                let alternatives = match search_seeded_run(
                    route,
                    pattern.ordinal,
                    run_ordinal,
                    run,
                    graph,
                    config,
                    &mut work,
                ) {
                    Ok(alternatives) => alternatives,
                    Err(SeededRunSearchFailure::Budget(termination)) => {
                        return RouteFamilyPlanningOutcome::PortfolioUnavailable {
                            outcome: RouteFamilyPortfolioBuildOutcome::GenerationBudgetExhausted {
                                route: route.branch.clone(),
                                termination,
                                work,
                            },
                        };
                    }
                    Err(SeededRunSearchFailure::Rejected(detail)) => {
                        rejected_pattern = Some(detail);
                        break;
                    }
                };
                let mut expanded = Vec::new();
                for combination in &combinations {
                    for alternative in &alternatives {
                        let mut next = combination.clone();
                        next.push(alternative.clone());
                        expanded.push(next);
                        if expanded.len()
                            == config.seeded_via_families.maximum_families_per_variable
                        {
                            break;
                        }
                    }
                    if expanded.len() == config.seeded_via_families.maximum_families_per_variable {
                        break;
                    }
                }
                combinations = expanded;
            }
            if let Some(detail) = rejected_pattern {
                work.rejection_evidence.push(RouteFamilyCandidateRejection {
                    branch: route.branch.clone(),
                    raw_ordinal: pattern.ordinal + 1,
                    family_id: format!("seeded-via:{}:pattern-{}", route.branch, pattern.ordinal),
                    reason: "run_family_unavailable".into(),
                    detail,
                    projection: None,
                });
                continue;
            }
            let mut pattern_families = Vec::new();
            for (combination_ordinal, combination) in combinations.into_iter().enumerate() {
                let family_id = format!(
                    "family:seeded-via:{}:{:04}:{combination_ordinal:04}",
                    route.branch, pattern.ordinal
                );
                let mut point_index = 0;
                let mut runs = Vec::new();
                let mut cost = 0.0;
                for (run_ordinal, embedding) in combination.iter().enumerate() {
                    let graph = graphs[embedding.layer.as_str()];
                    let passage_keys = passage_keys_for_embedding(graph, embedding)
                        .expect("strict run has valid passage projection");
                    let start_point = point_index;
                    let end_point = start_point + embedding.polyline.len() - 1;
                    let transition_from_previous = if run_ordinal == 0 {
                        RequiredNullable::Null
                    } else {
                        let position = embedding.polyline[0];
                        let previous_layer = &combination[run_ordinal - 1].layer;
                        let kind = if previous_layer == &embedding.layer {
                            RouteTransitionKind::Continuous
                        } else {
                            RouteTransitionKind::Via
                        };
                        RequiredNullable::Value(RouteTransition {
                            kind,
                            position: FamilyPoint {
                                x: position.x,
                                y: position.y,
                            },
                            certified: true,
                            via: if kind == RouteTransitionKind::Via {
                                Some(RouteViaGeometry {
                                    diameter: problem.rules.via_diameter,
                                    drill: problem.rules.via_drill,
                                    clearance_model: EXACT_VIA_CLEARANCE_MODEL.into(),
                                    minimum_clearance: pattern
                                        .via
                                        .expect("layer-changing pattern has a via")
                                        .1,
                                })
                            } else {
                                None
                            },
                        })
                    };
                    cost += polyline_length(&embedding.polyline);
                    runs.push(RouteFamilyRun {
                        run_id: format!("{family_id}:run:{run_ordinal}"),
                        run_index: run_ordinal,
                        layer: embedding.layer.clone(),
                        start_point,
                        end_point,
                        corridor_revision: embedding.corridor_revision,
                        transition_from_previous,
                        witness: RouteWitness {
                            polyline: embedding
                                .polyline
                                .iter()
                                .map(|point| FamilyPoint {
                                    x: point.x,
                                    y: point.y,
                                })
                                .collect(),
                            placement_revision: config.corridor.placement_revision,
                            realizable: true,
                            clearance_certified: true,
                            clearance_model: "exact_polyline_against_polygonal_obstacles".into(),
                            minimum_clearance: embedding.clearance.minimum_clearance,
                            required_width: route.physical_width,
                        },
                        cut_word_certificate: CutWordCertificate {
                            cut_basis_fingerprint: graph.cut_basis.fingerprint.clone(),
                            basis_complete: true,
                            verified: true,
                            word: embedding
                                .embedded_homotopy
                                .crossings()
                                .iter()
                                .map(|crossing| CutEvent {
                                    obstacle: crossing.obstacle.clone(),
                                    direction: match crossing.direction {
                                        CrossingDirection::Positive => CutDirection::Positive,
                                        CrossingDirection::Negative => CutDirection::Negative,
                                    },
                                })
                                .collect(),
                        },
                        semantic_passage_keys: passage_keys,
                        passage_mapping_complete: true,
                    });
                    point_index = end_point;
                }
                if pattern.via.is_some() {
                    cost += problem.rules.via_diameter;
                }
                pattern_families.push(RouteFamily {
                    family_id,
                    placement_revision: config.corridor.placement_revision,
                    complete_route: true,
                    cost,
                    runs,
                });
            }
            pattern_frontiers.push(pattern_families);
        }
        let mut families = Vec::new();
        let maximum_depth = pattern_frontiers
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or_default();
        'frontier: for depth in 0..maximum_depth {
            for frontier in &pattern_frontiers {
                if let Some(family) = frontier.get(depth) {
                    families.push(family.clone());
                    if families.len() == config.seeded_via_families.maximum_families_per_variable {
                        break 'frontier;
                    }
                }
            }
        }
        if families.is_empty() {
            return seeded_via_unavailable(
                Some(route.branch.clone()),
                "bounded seeded-via patterns produced no complete certified family",
                work,
            );
        }
        work.families_certified += families.len() as u64;
        for family in &families {
            for run in &family.runs {
                for key in &run.semantic_passage_keys {
                    let minimum = passage_minima
                        .entry((run.layer.clone(), key.clone()))
                        .or_insert(f64::INFINITY);
                    *minimum = (*minimum).min(run.witness.minimum_clearance);
                }
            }
        }
        work.raw_frontiers.push(RouteFamilyRawFrontierEvidence {
            branch: route.branch.clone(),
            requested_raw_bound: config.seeded_via_families.maximum_families_per_variable,
            returned_raw_candidates: families.len(),
            termination: if families.len()
                == config.seeded_via_families.maximum_families_per_variable
            {
                AlternativeSearchTermination::FamilyLimitReached
            } else {
                AlternativeSearchTermination::SearchExhausted
            },
            admitted_family_count: families.len(),
        });
        work.certifiable_frontiers_completed += 1;
        variables.push(RouteFamilyVariable {
            branch: route.branch.clone(),
            electrical_net: route.electrical_net.clone(),
            required_width: route.physical_width,
            required_clearance: Some(route.clearance),
            families,
        });
    }
    variables.sort_by(|left, right| left.branch.cmp(&right.branch));
    let mut corridor_epochs = corridors
        .iter()
        .map(|graph| CorridorEpoch {
            layer: graph.layer.clone(),
            corridor_revision: graph.revision,
            semantic_fingerprint: graph.semantic.fingerprint.clone(),
            cut_basis_fingerprint: graph.cut_basis.fingerprint.clone(),
            cut_basis_complete: graph.cut_basis.complete,
        })
        .collect::<Vec<_>>();
    corridor_epochs.sort_by(|left, right| left.layer.cmp(&right.layer));
    let passage_certificates = passage_minima
        .into_iter()
        .map(
            |((layer, key), minimum_clearance)| PassageCapacityCertificate {
                corridor_revision: graphs[layer.as_str()].revision,
                layer,
                key,
                placement_revision: config.corridor.placement_revision,
                available_width: None,
                minimum_clearance,
                enforcement: Some(PassageEnforcementMode::FixedWitnessPairwise),
                capacity_model: FIXED_WITNESS_NO_SCALAR_CAPACITY_MODEL.into(),
                certified: true,
            },
        )
        .collect();
    let mut portfolio = RouteFamilyPortfolio {
        schema_version: MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into(),
        feasibility_mode: Some(RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise),
        portfolio_id: config.portfolio_id.clone(),
        placement_revision: config.corridor.placement_revision,
        geometry_revision: Some(config.geometry_revision),
        corridor_epochs,
        family_generation: FamilyGenerationCertificate {
            method:
                "bounded_seed_polyline_via_sites_plus_per_run_visibility/spatial_round_robin/v2"
                    .into(),
            max_families_per_variable: config.seeded_via_families.maximum_families_per_variable,
            search_complete: true,
            budget_exhausted: false,
        },
        passage_certificates,
        variables,
        fixed_context: Some(FixedContextCertificate {
            method: FIXED_CONTEXT_ANALYSIS_METHOD.into(),
            placement_revision: config.corridor.placement_revision,
            geometry_revision: config.geometry_revision,
            fixed_context_fingerprint: String::new(),
            search_complete: true,
            runs: Vec::new(),
        }),
        pair_analysis: PairAnalysisCertificate {
            method: FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into(),
            placement_revision: config.corridor.placement_revision,
            corridor_epoch_fingerprint: String::new(),
            candidate_set_fingerprint: String::new(),
            search_complete: true,
            analyzed_family_pairs: Some(0),
            fixed_context_fingerprint: Some(String::new()),
            analyzed_family_context_pairs: Some(0),
            context_conflicts: Vec::new(),
            conflicts: Vec::new(),
        },
    };
    if let Err(issues) = portfolio.rebuild_and_seal_fixed_witness_analysis() {
        return RouteFamilyPlanningOutcome::PortfolioUnavailable {
            outcome: RouteFamilyPortfolioBuildOutcome::InvalidProducedPortfolio { issues, work },
        };
    }
    work.family_pairs_analyzed = portfolio
        .pair_analysis
        .analyzed_family_pairs
        .unwrap_or_default();
    work.family_context_pairs_analyzed = portfolio
        .pair_analysis
        .analyzed_family_context_pairs
        .unwrap_or_default();
    work.assignment_searches = 1;
    let assignment = match select_route_families_with_clearance_policy(
        &portfolio,
        FamilyAssignmentLimits {
            max_nodes: config.maximum_assignment_nodes,
        },
        config.seeded_via_families.clearance_policy,
    ) {
        Ok(assignment) => assignment,
        Err(error) => {
            return RouteFamilyPlanningOutcome::AssignmentRejected {
                error: error.to_string(),
                production_work: work,
            };
        }
    };
    RouteFamilyPlanningOutcome::Assignment {
        portfolio,
        production_work: work,
        assignment,
    }
}

/// Discharge the continuous work explicitly retained by an exact family
/// selection. Family IDs are resolved here; the engine receives only copper
/// polylines and required pair separations.
pub fn repair_selected_family_obligations(
    problem: &Problem,
    candidate: &CandidateArtifact,
    portfolio: &RouteFamilyPortfolio,
    assignment: &FamilyAssignmentOutcome,
    config: ContinuousObligationRepairConfig,
) -> Result<ContinuousObligationRepairResult, String> {
    config.check()?;
    let baseline_validation = validate_candidate(problem, candidate)?;
    let FamilyAssignmentOutcome::Selected { selection, .. } = assignment else {
        return Err("continuous obligation repair requires a selected family assignment".into());
    };
    if selection.repair_obligations.is_empty() {
        return Ok(ContinuousObligationRepairResult {
            status: ContinuousObligationRepairStatus::NoObligations,
            candidate: candidate.clone(),
            validation: baseline_validation,
            proposed_validation: None,
            obligation_count: 0,
            segment_pairs: 0,
            segment_body_pairs: 0,
            frames: Vec::new(),
            detail: "selected family assignment has no continuous repair obligations".into(),
        });
    }

    let candidate_traces = candidate
        .traces
        .iter()
        .map(|trace| (trace.branch.as_str(), trace))
        .collect::<BTreeMap<_, _>>();
    let mut inputs = BTreeMap::<String, CopperPolylineInput>::new();
    let mut separations = Vec::new();
    for obligation in &selection.repair_obligations {
        match obligation {
            FamilySelectionRepairObligation::FamilyPair {
                left_family_id,
                right_family_id,
                kind,
                ..
            } => {
                require_copper_clearance(*kind)?;
                let (left_branch, left_family) =
                    selected_family(portfolio, selection, left_family_id)?;
                let (right_branch, right_family) =
                    selected_family(portfolio, selection, right_family_id)?;
                let left_trace = candidate_traces.get(left_branch).ok_or_else(|| {
                    format!("candidate lacks selected family branch {left_branch}")
                })?;
                let right_trace = candidate_traces.get(right_branch).ok_or_else(|| {
                    format!("candidate lacks selected family branch {right_branch}")
                })?;
                require_single_layer_family(left_family, &left_trace.layer)?;
                require_single_layer_family(right_family, &right_trace.layer)?;
                insert_trace_repair_input(
                    &mut inputs,
                    left_trace,
                    CopperPolylineMobility::Interior,
                )?;
                insert_trace_repair_input(
                    &mut inputs,
                    right_trace,
                    CopperPolylineMobility::Interior,
                )?;
                separations.push(CopperSeparationPair {
                    first: left_branch.to_owned(),
                    second: right_branch.to_owned(),
                    clearance: continuous_clearance(problem, config)?,
                });
            }
            FamilySelectionRepairObligation::FixedContext {
                family_id,
                context_id,
                kind,
                ..
            } => {
                require_copper_clearance(*kind)?;
                let (branch, family) = selected_family(portfolio, selection, family_id)?;
                let trace = candidate_traces
                    .get(branch)
                    .ok_or_else(|| format!("candidate lacks selected family branch {branch}"))?;
                require_single_layer_family(family, &trace.layer)?;
                insert_trace_repair_input(&mut inputs, trace, CopperPolylineMobility::Interior)?;
                let context = portfolio
                    .fixed_context
                    .as_ref()
                    .and_then(|certificate| {
                        certificate
                            .runs
                            .iter()
                            .find(|run| run.context_id == *context_id)
                    })
                    .ok_or_else(|| format!("portfolio lacks fixed context {context_id}"))?;
                if context.layer != trace.layer {
                    return Err(format!(
                        "repair obligation places {family_id} and {context_id} on different layers"
                    ));
                }
                let engine_id = format!("fixed-context:{context_id}");
                let context_input = CopperPolylineInput {
                    id: engine_id.clone(),
                    points: context
                        .polyline
                        .iter()
                        .map(|point| {
                            Ok(EngineVec2::new(
                                checked_engine_float(point.x, "fixed-context point")?,
                                checked_engine_float(point.y, "fixed-context point")?,
                            ))
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                    width: checked_engine_float(context.required_width, "fixed-context width")?,
                    tension_weight: 0.0,
                    mobility: CopperPolylineMobility::Fixed,
                };
                if inputs.insert(engine_id.clone(), context_input).is_some() {
                    return Err(format!("duplicate fixed-context obligation {context_id}"));
                }
                separations.push(CopperSeparationPair {
                    first: branch.to_owned(),
                    second: engine_id,
                    clearance: continuous_clearance(problem, config)?,
                });
            }
        }
    }
    let poses = component_pose_map(&candidate.components)?;
    let branch_intents = compile_corridor_branches(problem, &poses)?
        .into_iter()
        .map(|branch| (branch.id, (branch.from, branch.to)))
        .collect::<BTreeMap<_, _>>();
    let mut bodies = BTreeMap::<String, CopperBodyInput>::new();
    let mut attachments = Vec::new();
    for (branch_id, input) in &mut inputs {
        let Some(trace) = candidate_traces.get(branch_id.as_str()) else {
            continue;
        };
        let (from, to) = branch_intents
            .get(branch_id)
            .ok_or_else(|| format!("candidate branch {branch_id} has no route intent"))?;
        input.mobility = CopperPolylineMobility::All;
        for (point_index, terminal) in [(0, from), (trace.points.len() - 1, to)] {
            let pose = poses
                .get(terminal.component.as_str())
                .ok_or_else(|| format!("missing candidate pose {}", terminal.component))?;
            insert_continuous_body(
                &mut bodies,
                continuous_body_input(problem, &poses, &terminal.component)?,
            )?;
            let endpoint = trace.points[point_index];
            let local = rotate_degrees(
                Vec2::new(endpoint.x - pose.position.x, endpoint.y - pose.position.y),
                -pose.rotation_degrees,
            );
            attachments.push(CopperAttachmentInput {
                polyline: branch_id.clone(),
                point_index,
                body: terminal.component.clone(),
                local_position: EngineVec2::new(
                    checked_engine_float(local.x, "endpoint local position")?,
                    checked_engine_float(local.y, "endpoint local position")?,
                ),
            });
        }
    }
    let mut body_separations = Vec::new();
    if config.include_body_contacts {
        for branch_id in inputs.keys() {
            let Some(_) = candidate_traces.get(branch_id.as_str()) else {
                continue;
            };
            let (from, to) = branch_intents
                .get(branch_id)
                .ok_or_else(|| format!("candidate branch {branch_id} has no route intent"))?;
            for obstacle in problem
                .components
                .iter()
                .filter(|component| component.body_is_routing_keepout)
            {
                if obstacle.id == from.component || obstacle.id == to.component {
                    continue;
                }
                insert_continuous_body(
                    &mut bodies,
                    continuous_body_input(problem, &poses, &obstacle.id)?,
                )?;
                body_separations.push(CopperSegmentBodyPair {
                    polyline: branch_id.clone(),
                    body: obstacle.id.clone(),
                    clearance: continuous_clearance(problem, config)?,
                });
            }
        }
    }
    let request = CopperRepairRequest {
        bounds: EngineBounds::new(
            EngineVec2::new(
                checked_engine_float(problem.board.bounds.min.x, "board minimum")?,
                checked_engine_float(problem.board.bounds.min.y, "board minimum")?,
            ),
            EngineVec2::new(
                checked_engine_float(problem.board.bounds.max.x, "board maximum")?,
                checked_engine_float(problem.board.bounds.max.y, "board maximum")?,
            ),
        ),
        bodies: bodies.into_values().collect(),
        polylines: inputs.into_values().collect(),
        shared_points: Vec::new(),
        attachments,
        separations,
        body_separations,
        body_body_separations: Vec::new(),
        maximum_segment_pairs: config.maximum_segment_pairs,
    };
    let mut compiled = compile_copper_repair(&request)?;
    let segment_pairs = compiled.segment_pairs;
    let segment_body_pairs = compiled.segment_body_pairs;
    let mut backend = CpuReferenceBackend::with_field(NoField);
    let solver = SolverConfig {
        field_strength: 0.0,
        projection_iterations: config.projection_iterations,
        ..SolverConfig::default()
    };
    let mut frames = Vec::with_capacity(config.steps + 1);
    frames.push(backend.step(
        &mut compiled.world,
        &SolverConfig {
            projection_iterations: 0,
            ..solver
        },
        0,
    ));
    for step in 1..=config.steps {
        frames.push(backend.step(&mut compiled.world, &solver, step as u64));
    }
    let mut proposed = candidate.clone();
    let body_poses = compiled
        .body_poses()
        .into_iter()
        .map(|pose| (pose.id.clone(), pose))
        .collect::<BTreeMap<_, _>>();
    for component in &mut proposed.components {
        let Some(body) = body_poses.get(&component.id) else {
            continue;
        };
        let declared = problem
            .components
            .iter()
            .find(|declared| declared.id == component.id)
            .ok_or_else(|| format!("candidate contains unknown component {}", component.id))?;
        let engine_position = Vec2::new(body.position.x as f64, body.position.y as f64);
        component.position = match declared.constraints.movement {
            Movement::Fixed => component.position,
            Movement::Horizontal => Vec2::new(engine_position.x, component.position.y),
            Movement::Vertical => Vec2::new(component.position.x, engine_position.y),
            Movement::Free => engine_position,
        };
        if declared.constraints.rotation == Rotation::Free {
            component.rotation_degrees = (body.angle_radians as f64).to_degrees().rem_euclid(360.0);
        }
    }
    let mut proposed_traces = proposed
        .traces
        .iter_mut()
        .map(|trace| (trace.branch.clone(), trace))
        .collect::<BTreeMap<_, _>>();
    for (id, points) in compiled.polylines() {
        let Some(trace) = proposed_traces.get_mut(id.as_str()) else {
            continue;
        };
        trace.points = points
            .into_iter()
            .map(|point| Vec2::new(point.x as f64, point.y as f64))
            .collect();
    }
    drop(proposed_traces);
    let proposed_poses = component_pose_map(&proposed.components)?;
    let proposed_intents = compile_corridor_branches(problem, &proposed_poses)?
        .into_iter()
        .map(|branch| (branch.id, (branch.from, branch.to)))
        .collect::<BTreeMap<_, _>>();
    let mut graph_positions = BTreeMap::<String, Vec2>::new();
    for trace in &mut proposed.traces {
        let (from, to) = proposed_intents
            .get(&trace.branch)
            .ok_or_else(|| format!("candidate branch {} has no route intent", trace.branch))?;
        let (from_position, _) = terminal_attachment(problem, &proposed_poses, from, &trace.layer)?;
        let (to_position, _) = terminal_attachment(problem, &proposed_poses, to, &trace.layer)?;
        trace.points[0] = from_position;
        let last = trace.points.len() - 1;
        trace.points[last] = to_position;
        // Any continuous point or endpoint-body motion invalidates the old
        // corridor/cut-basis epoch. The exact candidate remains usable, but it
        // must not carry a stale family certificate fingerprint.
        trace.route_basis_fingerprint = None;
        graph_positions.insert(trace.from_node.clone(), from_position);
        graph_positions.insert(trace.to_node.clone(), to_position);
    }
    for graph in &mut proposed.route_graphs {
        for node in &mut graph.nodes {
            if let Some(&position) = graph_positions.get(&node.id) {
                node.position = position;
            }
        }
    }
    let proposed_validation = validate_candidate(problem, &proposed)?;
    let baseline_score = exact_repair_score(&baseline_validation);
    let proposed_score = exact_repair_score(&proposed_validation);
    let (status, accepted, detail) = if proposed_validation.complete {
        (
            ContinuousObligationRepairStatus::ExactComplete,
            true,
            format!(
                "continuous correction discharged {} obligation(s) and passed exact validation",
                selection.repair_obligations.len()
            ),
        )
    } else if proposed_score.0 <= baseline_score.0
        && proposed_score.1 <= baseline_score.1
        && proposed_score != baseline_score
    {
        (
            ContinuousObligationRepairStatus::Improved,
            true,
            format!(
                "continuous correction improved exact score from {baseline_score:?} to {proposed_score:?}"
            ),
        )
    } else {
        (
            ContinuousObligationRepairStatus::RolledBack,
            false,
            format!(
                "continuous correction did not improve exact score {baseline_score:?} (proposal {proposed_score:?}); retained original candidate"
            ),
        )
    };
    Ok(ContinuousObligationRepairResult {
        status,
        candidate: if accepted {
            proposed
        } else {
            candidate.clone()
        },
        validation: if accepted {
            proposed_validation.clone()
        } else {
            baseline_validation
        },
        proposed_validation: Some(proposed_validation),
        obligation_count: selection.repair_obligations.len(),
        segment_pairs,
        segment_body_pairs,
        frames,
        detail,
    })
}

fn exact_repair_score(validation: &ExactValidationAssessment) -> (usize, usize) {
    let trace_clearance = validation
        .geometry
        .violations
        .iter()
        .filter(|violation| violation.code == "trace_trace_clearance")
        .count();
    (trace_clearance, validation.violations.len())
}

fn require_copper_clearance(kind: FamilyConflictKind) -> Result<(), String> {
    if kind == FamilyConflictKind::CopperClearance {
        Ok(())
    } else {
        Err(format!(
            "continuous correction cannot discharge discrete {kind:?} obligation"
        ))
    }
}

fn selected_family<'a>(
    portfolio: &'a RouteFamilyPortfolio,
    selection: &layout_trace_routing::family_assignment::FamilySelection,
    family_id: &str,
) -> Result<(&'a str, &'a RouteFamily), String> {
    let mut found = None;
    for variable in &portfolio.variables {
        if let Some(family) = variable
            .families
            .iter()
            .find(|family| family.family_id == family_id)
        {
            if selection.choices.get(&variable.branch).map(String::as_str) != Some(family_id) {
                return Err(format!(
                    "repair obligation names unselected family {family_id}"
                ));
            }
            if found.replace((variable.branch.as_str(), family)).is_some() {
                return Err(format!("family ID {family_id} is ambiguous"));
            }
        }
    }
    found.ok_or_else(|| format!("portfolio lacks family {family_id}"))
}

fn require_single_layer_family(family: &RouteFamily, layer: &str) -> Result<(), String> {
    if family.runs.len() == 1 && family.runs[0].layer == layer {
        Ok(())
    } else {
        Err(format!(
            "continuous correction currently requires one same-layer run for family {}",
            family.family_id
        ))
    }
}

fn insert_trace_repair_input(
    inputs: &mut BTreeMap<String, CopperPolylineInput>,
    trace: &SolvedTrace,
    mobility: CopperPolylineMobility,
) -> Result<(), String> {
    if trace.points.len() < 2
        || trace.segment_layers.len() + 1 != trace.points.len()
        || trace
            .segment_layers
            .iter()
            .any(|layer| layer != &trace.layer)
        || !trace.vias.is_empty()
    {
        return Err(format!(
            "continuous correction currently requires same-layer via-free trace {}",
            trace.branch
        ));
    }
    let input = CopperPolylineInput {
        id: trace.branch.clone(),
        points: trace
            .points
            .iter()
            .map(|point| {
                Ok(EngineVec2::new(
                    checked_engine_float(point.x, "trace point")?,
                    checked_engine_float(point.y, "trace point")?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?,
        width: checked_engine_float(trace.width, "trace width")?,
        tension_weight: checked_engine_float(trace.tension_weight, "trace tension weight")?,
        mobility,
    };
    if let Some(existing) = inputs.get(&trace.branch) {
        if existing.points != input.points || existing.width != input.width {
            return Err(format!(
                "conflicting continuous inputs for trace {}",
                trace.branch
            ));
        }
    } else {
        inputs.insert(trace.branch.clone(), input);
    }
    Ok(())
}

fn checked_engine_float(value: f64, label: &str) -> Result<f32, String> {
    let converted = value as f32;
    if value.is_finite() && converted.is_finite() {
        Ok(converted)
    } else {
        Err(format!("{label} cannot be represented by the f32 engine"))
    }
}

fn continuous_body_input(
    problem: &Problem,
    poses: &BTreeMap<&str, &SolvedComponent>,
    component_id: &str,
) -> Result<CopperBodyInput, String> {
    let declared = problem
        .components
        .iter()
        .find(|component| component.id == component_id)
        .ok_or_else(|| format!("unknown continuous body {component_id}"))?;
    let pose = poses
        .get(component_id)
        .ok_or_else(|| format!("missing candidate pose {component_id}"))?;
    let (translate_x, translate_y) = match declared.constraints.movement {
        Movement::Fixed => (false, false),
        Movement::Horizontal => (true, false),
        Movement::Vertical => (false, true),
        Movement::Free => (true, true),
    };
    let rotate = declared.constraints.rotation == Rotation::Free;
    Ok(CopperBodyInput {
        id: declared.id.clone(),
        position: EngineVec2::new(
            checked_engine_float(pose.position.x, "component position")?,
            checked_engine_float(pose.position.y, "component position")?,
        ),
        angle_radians: checked_engine_float(
            pose.rotation_degrees.to_radians(),
            "component rotation",
        )?,
        size: EngineVec2::new(
            checked_engine_float(declared.size.x, "component size")?,
            checked_engine_float(declared.size.y, "component size")?,
        ),
        mobility: EngineMobility {
            inverse_mass: if translate_x || translate_y {
                checked_engine_float(problem.solver.component_mobility, "component mobility")?
            } else {
                0.0
            },
            rotation_mobility: if rotate {
                checked_engine_float(
                    problem.solver.rotation_mobility,
                    "component rotation mobility",
                )?
            } else {
                0.0
            },
            translate_x,
            translate_y,
            rotate,
        },
    })
}

fn insert_continuous_body(
    bodies: &mut BTreeMap<String, CopperBodyInput>,
    body: CopperBodyInput,
) -> Result<(), String> {
    if let Some(existing) = bodies.get(&body.id) {
        if existing.position != body.position
            || existing.angle_radians != body.angle_radians
            || existing.size != body.size
            || existing.mobility != body.mobility
        {
            return Err(format!(
                "conflicting continuous body inputs for {}",
                body.id
            ));
        }
    } else {
        bodies.insert(body.id.clone(), body);
    }
    Ok(())
}

fn continuous_clearance(
    problem: &Problem,
    config: ContinuousObligationRepairConfig,
) -> Result<f32, String> {
    checked_engine_float(
        problem.rules.clearance + f64::from(config.numerical_clearance_margin_nm) / 1_000_000.0,
        "board clearance plus numerical margin",
    )
}

pub fn analyze_problem_route_families_at_poses(
    problem: &Problem,
    components: &[SolvedComponent],
    config: &RouteFamilyAnalysisConfig,
) -> Result<RouteFamilyAnalysisResult, String> {
    config.check()?;
    let corridor = analyze_problem_corridors_at_poses(problem, components, &config.corridor)?;
    let poses = component_pose_map(components)?;
    let owned = compile_corridor_branches(problem, &poses)?
        .into_iter()
        .map(|branch| {
            let (start_layer, end_layer) = route_endpoint_layers(
                problem,
                &branch.from,
                &branch.to,
                &branch.layer,
                &branch.allowed_layers,
                config.seeded_via_families.enabled,
            )?;
            let (from, from_obstacle) =
                terminal_attachment(problem, &poses, &branch.from, &start_layer)?;
            let (to, to_obstacle) = terminal_attachment(problem, &poses, &branch.to, &end_layer)?;
            let mut reference = if branch.reference.len() >= 2 {
                branch.reference
            } else {
                vec![from, to]
            };
            reference[0] = from;
            let last = reference.len() - 1;
            reference[last] = to;
            let from_node = terminal_node_id(&branch.electrical_net, &branch.from);
            let to_node = terminal_node_id(&branch.electrical_net, &branch.to);
            Ok(OwnedRouteFamilyInput {
                branch: branch.id,
                electrical_net: branch.electrical_net,
                layer: start_layer,
                end_layer,
                allowed_layers: branch.allowed_layers,
                physical_width: branch.width,
                tension_weight: branch.tension_weight,
                clearance: problem.rules.clearance,
                from_obstacle,
                to_obstacle,
                from,
                to,
                reference,
                from_terminal: branch.from,
                to_terminal: branch.to,
                from_node,
                to_node,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let routes = owned
        .iter()
        .map(|route| {
            let last = route.reference.len() - 1;
            RouteFamilyRouteInput {
                branch_id: &route.branch,
                electrical_net: &route.electrical_net,
                layer: &route.layer,
                physical_width: route.physical_width,
                clearance: route.clearance,
                from_component: &route.from_obstacle,
                to_component: &route.to_obstacle,
                from: route.from,
                from_toward: route.reference[1],
                to: route.to,
                to_toward: route.reference[last - 1],
                reference: &route.reference,
            }
        })
        .collect::<Vec<_>>();
    let fixed_context = Vec::<RouteFamilyFixedContextInput>::new();
    let outcome = if owned.iter().any(|route| route.layer != route.end_layer) {
        plan_seeded_via_route_families(problem, &owned, &corridor.corridors, config)
    } else {
        plan_route_families(
            RouteFamilyPortfolioInput {
                portfolio_id: &config.portfolio_id,
                placement_revision: config.corridor.placement_revision,
                geometry_revision: config.geometry_revision,
                corridors: &corridor.corridors,
                routes: &routes,
                fixed_context: &fixed_context,
            },
            config,
        )
    };

    let (portfolio, assignment, production_work, status, detail) = match outcome {
        RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work,
            assignment,
        } => {
            let (status, detail) = match &assignment {
                FamilyAssignmentOutcome::Selected { selection, .. } => (
                    RouteFamilyAnalysisStatus::Selected,
                    format!(
                        "selected {} route families with {} repair obligation(s)",
                        selection.choices.len(),
                        selection.repair_obligations.len()
                    ),
                ),
                FamilyAssignmentOutcome::BoundedInfeasible { evidence, .. } => (
                    RouteFamilyAnalysisStatus::BoundedInfeasible,
                    format!(
                        "bounded admitted portfolio is infeasible; {} variable(s) implicated",
                        evidence.implicated_variables.len()
                    ),
                ),
                FamilyAssignmentOutcome::BudgetExhausted { .. } => (
                    RouteFamilyAnalysisStatus::AssignmentBudgetExhausted,
                    "exact family assignment exhausted its node budget".into(),
                ),
            };
            (
                Some(portfolio),
                Some(assignment),
                production_work,
                status,
                detail,
            )
        }
        RouteFamilyPlanningOutcome::AssignmentRejected {
            error,
            production_work,
        } => (
            None,
            None,
            production_work,
            RouteFamilyAnalysisStatus::AssignmentRejected,
            error,
        ),
        RouteFamilyPlanningOutcome::PortfolioUnavailable { outcome } => {
            let (detail, work) = unavailable_detail_and_work(outcome);
            (
                None,
                None,
                work,
                RouteFamilyAnalysisStatus::PortfolioUnavailable,
                detail,
            )
        }
    };
    let (candidate, validation, continuous_repair, status, detail) = if status
        == RouteFamilyAnalysisStatus::Selected
    {
        let portfolio_ref = portfolio.as_ref().expect("selected outcome has portfolio");
        let assignment_ref = assignment
            .as_ref()
            .expect("selected outcome has assignment");
        match materialize_selected_candidate(
            problem,
            components,
            &owned,
            portfolio_ref,
            assignment_ref,
        ) {
            Ok(candidate) => {
                let validation = validate_candidate(problem, &candidate)?;
                if validation.complete {
                    (Some(candidate), Some(validation), None, status, detail)
                } else {
                    match repair_selected_family_obligations(
                        problem,
                        &candidate,
                        portfolio_ref,
                        assignment_ref,
                        config.continuous_obligation_repair,
                    ) {
                        Ok(repair) => {
                            let repaired_candidate = repair.candidate.clone();
                            let repaired_validation = repair.validation.clone();
                            let repaired_status = if repaired_validation.complete {
                                RouteFamilyAnalysisStatus::Selected
                            } else {
                                RouteFamilyAnalysisStatus::ExactRejected
                            };
                            let repaired_detail = if repaired_validation.complete {
                                repair.detail.clone()
                            } else {
                                format!(
                                    "selected route families remain exact-rejected after continuous obligation handling: {}",
                                    repair.detail
                                )
                            };
                            (
                                Some(repaired_candidate),
                                Some(repaired_validation),
                                Some(repair),
                                repaired_status,
                                repaired_detail,
                            )
                        }
                        Err(error) => (
                            Some(candidate),
                            Some(validation),
                            None,
                            RouteFamilyAnalysisStatus::ExactRejected,
                            format!(
                                "selected route families are exact-invalid and the configured continuous obligation handler cannot process them: {error}"
                            ),
                        ),
                    }
                }
            }
            Err(error) => (
                None,
                None,
                None,
                RouteFamilyAnalysisStatus::ExactRejected,
                format!("selected route families could not materialize: {error}"),
            ),
        }
    } else {
        (None, None, None, status, detail)
    };
    Ok(RouteFamilyAnalysisResult {
        corridor,
        candidate,
        validation,
        portfolio,
        assignment,
        continuous_repair,
        production_work,
        evidence: RouteFamilyAnalysisEvidence {
            strategy: "layout-trace-route-family-planning-v1".into(),
            config: config.clone(),
            status,
            detail,
        },
    })
}

/// Repair the smallest bounded connected component of exact trace/trace
/// clearance conflicts. Copper outside the selected component is immutable
/// fixed context for family generation and assignment.
pub fn repair_candidate_route_families(
    problem: &Problem,
    candidate: &CandidateArtifact,
    config: &RouteFamilyAnalysisConfig,
) -> Result<RouteFamilyRepairResult, String> {
    config.check()?;
    let original_validation = validate_candidate(problem, candidate)?;
    let Some((selected_indices, component_edges)) = select_repair_component(
        candidate,
        &original_validation,
        config.maximum_repair_component_branches,
    ) else {
        return Ok(RouteFamilyRepairResult {
            original_validation: original_validation.clone(),
            corridor: None,
            candidate: None,
            validation: None,
            portfolio: None,
            assignment: None,
            context_correction: None,
            continuous_repair: None,
            production_work: RouteFamilyProductionWork::default(),
            evidence: RouteFamilyRepairEvidence {
                strategy: "layout-trace-fixed-context-component-repair-v1".into(),
                config: config.clone(),
                status: RouteFamilyRepairStatus::NoEligibleConflict,
                selected_branches: Vec::new(),
                component_edges: 0,
                original_selected_conflicts: 0,
                proposed_selected_conflicts: None,
                original_findings: original_validation.violations.len(),
                proposed_findings: None,
                detail: format!(
                    "no connected trace/trace conflict component with 2..={} eligible same-layer branches",
                    config.maximum_repair_component_branches
                ),
            },
        });
    };
    let selected = selected_indices.iter().copied().collect::<BTreeSet<_>>();
    let selected_branches = selected_indices
        .iter()
        .map(|index| candidate.traces[*index].branch.clone())
        .collect::<Vec<_>>();
    let selected_branch_set = selected_branches
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let original_selected_conflicts =
        selected_trace_conflicts(&original_validation, &selected_branch_set);
    let corridor =
        analyze_problem_corridors_at_poses(problem, &candidate.components, &config.corridor)?;
    let poses = component_pose_map(&candidate.components)?;
    let declared = compile_corridor_branches(problem, &poses)?
        .into_iter()
        .map(|branch| (branch.id.clone(), branch))
        .collect::<BTreeMap<_, _>>();
    let mut owned = Vec::new();
    for &trace_index in &selected_indices {
        let trace = &candidate.traces[trace_index];
        let branch = declared.get(&trace.branch).ok_or_else(|| {
            format!(
                "candidate trace {} has no declared route intent",
                trace.branch
            )
        })?;
        let (from, from_obstacle) =
            terminal_attachment(problem, &poses, &branch.from, &trace.layer)?;
        let (to, to_obstacle) = terminal_attachment(problem, &poses, &branch.to, &trace.layer)?;
        let mut reference = trace.points.clone();
        reference[0] = from;
        let last = reference.len() - 1;
        reference[last] = to;
        owned.push(OwnedRouteFamilyInput {
            branch: trace.branch.clone(),
            electrical_net: trace.electrical_net.clone(),
            layer: trace.layer.clone(),
            end_layer: trace.layer.clone(),
            allowed_layers: vec![trace.layer.clone()],
            physical_width: trace.width,
            tension_weight: trace.tension_weight,
            clearance: problem.rules.clearance,
            from_obstacle,
            to_obstacle,
            from,
            to,
            reference,
            from_terminal: branch.from.clone(),
            to_terminal: branch.to.clone(),
            from_node: trace.from_node.clone(),
            to_node: trace.to_node.clone(),
        });
    }
    owned.sort_by(|left, right| left.branch.cmp(&right.branch));
    let routes = owned
        .iter()
        .map(|route| {
            let last = route.reference.len() - 1;
            RouteFamilyRouteInput {
                branch_id: &route.branch,
                electrical_net: &route.electrical_net,
                layer: &route.layer,
                physical_width: route.physical_width,
                clearance: route.clearance,
                from_component: &route.from_obstacle,
                to_component: &route.to_obstacle,
                from: route.from,
                from_toward: route.reference[1],
                to: route.to,
                to_toward: route.reference[last - 1],
                reference: &route.reference,
            }
        })
        .collect::<Vec<_>>();
    let fixed_context = fixed_context_from_candidate(problem, candidate, &selected)?;
    let portfolio_id = format!(
        "{}:repair:{}",
        config.portfolio_id,
        selected_branches.join("+")
    );
    let outcome = plan_route_families(
        RouteFamilyPortfolioInput {
            portfolio_id: &portfolio_id,
            placement_revision: config.corridor.placement_revision,
            geometry_revision: config.geometry_revision,
            corridors: &corridor.corridors,
            routes: &routes,
            fixed_context: &fixed_context,
        },
        config,
    );
    let (portfolio, assignment, production_work, failed_status, failed_detail) = match outcome {
        RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work,
            assignment,
        } => {
            let (status, detail) = match &assignment {
                FamilyAssignmentOutcome::Selected { selection, .. } => (
                    None,
                    format!(
                        "selected {} replacement families with {} repair obligation(s)",
                        selection.choices.len(),
                        selection.repair_obligations.len()
                    ),
                ),
                FamilyAssignmentOutcome::BoundedInfeasible { evidence, .. } => (
                    Some(RouteFamilyRepairStatus::BoundedInfeasible),
                    format!(
                        "bounded fixed-context portfolio is infeasible; {} variable(s) implicated",
                        evidence.implicated_variables.len()
                    ),
                ),
                FamilyAssignmentOutcome::BudgetExhausted { .. } => (
                    Some(RouteFamilyRepairStatus::AssignmentBudgetExhausted),
                    "fixed-context family assignment exhausted its node budget".into(),
                ),
            };
            (
                Some(portfolio),
                Some(assignment),
                production_work,
                status,
                detail,
            )
        }
        RouteFamilyPlanningOutcome::AssignmentRejected {
            error,
            production_work,
        } => (
            None,
            None,
            production_work,
            Some(RouteFamilyRepairStatus::AssignmentRejected),
            error,
        ),
        RouteFamilyPlanningOutcome::PortfolioUnavailable { outcome } => {
            let (detail, work) = unavailable_detail_and_work(outcome);
            (
                None,
                None,
                work,
                Some(RouteFamilyRepairStatus::PortfolioUnavailable),
                detail,
            )
        }
    };
    if let Some(status) = failed_status {
        let context_correction = if status == RouteFamilyRepairStatus::BoundedInfeasible {
            let eligible_context = candidate
                .traces
                .iter()
                .enumerate()
                .filter(|(index, trace)| {
                    !selected.contains(index) && trace_is_repair_eligible(trace)
                })
                .map(|(_, trace)| trace.branch.clone())
                .collect::<BTreeSet<_>>();
            portfolio
                .as_ref()
                .map(|portfolio| {
                    select_context_correction(
                        portfolio,
                        &eligible_context,
                        config.context_correction_limits,
                    )
                    .map_err(|error| error.to_string())
                })
                .transpose()?
        } else {
            None
        };
        let failed_detail = match &context_correction {
            Some(ContextCorrectionOutcome::Selected { selection, .. }) => format!(
                "{failed_detail}; exact context correction selected promotion of {}",
                selection.promoted_context_branches.join(", ")
            ),
            Some(ContextCorrectionOutcome::NoCorrectionWithinBound { .. }) => format!(
                "{failed_detail}; no fixed-context correction exists within the configured promotion bound"
            ),
            Some(ContextCorrectionOutcome::NoPromotionNeeded { .. }) => format!(
                "{failed_detail}; context solver found no promotion necessary in this admitted portfolio"
            ),
            Some(ContextCorrectionOutcome::BudgetExhausted { .. }) => {
                format!("{failed_detail}; context-correction search exhausted its node budget")
            }
            None => failed_detail,
        };
        return Ok(RouteFamilyRepairResult {
            original_validation: original_validation.clone(),
            corridor: Some(corridor),
            candidate: None,
            validation: None,
            portfolio,
            assignment,
            context_correction,
            continuous_repair: None,
            production_work,
            evidence: RouteFamilyRepairEvidence {
                strategy: "layout-trace-fixed-context-component-repair-v1".into(),
                config: config.clone(),
                status,
                selected_branches,
                component_edges,
                original_selected_conflicts,
                proposed_selected_conflicts: None,
                original_findings: original_validation.violations.len(),
                proposed_findings: None,
                detail: failed_detail,
            },
        });
    }
    let portfolio_ref = portfolio
        .as_ref()
        .ok_or_else(|| "selected repair lacks a portfolio".to_string())?;
    let assignment_ref = assignment
        .as_ref()
        .ok_or_else(|| "selected repair lacks an assignment".to_string())?;
    let replacement =
        match materialize_selected_traces(problem, &owned, portfolio_ref, assignment_ref) {
            Ok(replacement) => replacement,
            Err(error) => {
                return Ok(RouteFamilyRepairResult {
                    original_validation: original_validation.clone(),
                    corridor: Some(corridor),
                    candidate: None,
                    validation: None,
                    portfolio,
                    assignment,
                    context_correction: None,
                    continuous_repair: None,
                    production_work,
                    evidence: RouteFamilyRepairEvidence {
                        strategy: "layout-trace-fixed-context-component-repair-v1".into(),
                        config: config.clone(),
                        status: RouteFamilyRepairStatus::ExactRejected,
                        selected_branches,
                        component_edges,
                        original_selected_conflicts,
                        proposed_selected_conflicts: None,
                        original_findings: original_validation.violations.len(),
                        proposed_findings: None,
                        detail: format!("selected repair could not materialize: {error}"),
                    },
                });
            }
        };
    let mut proposed = candidate.clone();
    proposed
        .traces
        .retain(|trace| !selected_branch_set.contains(trace.branch.as_str()));
    proposed.traces.extend(replacement);
    proposed
        .traces
        .sort_by(|left, right| left.branch.cmp(&right.branch));
    let initial_proposed_validation = validate_candidate(problem, &proposed)?;
    let has_continuous_obligations = matches!(
        assignment_ref,
        FamilyAssignmentOutcome::Selected { selection, .. }
            if !selection.repair_obligations.is_empty()
    );
    let continuous_repair = if !initial_proposed_validation.complete && has_continuous_obligations {
        Some(repair_selected_family_obligations(
            problem,
            &proposed,
            portfolio_ref,
            assignment_ref,
            config.continuous_obligation_repair,
        )?)
    } else {
        None
    };
    let (proposed, proposed_validation) = match &continuous_repair {
        Some(repair) => (repair.candidate.clone(), repair.validation.clone()),
        None => (proposed, initial_proposed_validation),
    };
    let proposed_selected_conflicts =
        selected_trace_conflicts(&proposed_validation, &selected_branch_set);
    let original_score = (
        original_selected_conflicts,
        original_validation.violations.len(),
    );
    let proposed_score = (
        proposed_selected_conflicts,
        proposed_validation.violations.len(),
    );
    let continuous_detail = continuous_repair
        .as_ref()
        .map(|repair| format!("; continuous obligation pass: {}", repair.detail))
        .unwrap_or_default();
    let (status, detail) = if proposed_validation.complete {
        (
            RouteFamilyRepairStatus::ExactComplete,
            format!(
                "fixed-context component replacement passes the independent exact gate{continuous_detail}"
            ),
        )
    } else if proposed_score.0 <= original_score.0
        && proposed_score.1 <= original_score.1
        && proposed_score != original_score
    {
        (
            RouteFamilyRepairStatus::Improved,
            format!(
                "fixed-context component replacement improved exact score from {:?} to {:?}; unresolved findings remain{continuous_detail}",
                original_score, proposed_score,
            ),
        )
    } else {
        (
            RouteFamilyRepairStatus::NoImprovement,
            format!(
                "fixed-context component replacement did not improve exact score {:?} (proposal {:?}){continuous_detail}",
                original_score, proposed_score,
            ),
        )
    };
    Ok(RouteFamilyRepairResult {
        original_validation: original_validation.clone(),
        corridor: Some(corridor),
        candidate: Some(proposed),
        validation: Some(proposed_validation.clone()),
        portfolio,
        assignment,
        context_correction: None,
        continuous_repair,
        production_work,
        evidence: RouteFamilyRepairEvidence {
            strategy: "layout-trace-fixed-context-component-repair-v1".into(),
            config: config.clone(),
            status,
            selected_branches,
            component_edges,
            original_selected_conflicts,
            proposed_selected_conflicts: Some(proposed_selected_conflicts),
            original_findings: original_validation.violations.len(),
            proposed_findings: Some(proposed_validation.violations.len()),
            detail,
        },
    })
}

fn trace_is_repair_eligible(trace: &SolvedTrace) -> bool {
    trace.points.len() >= 2
        && trace.vias.is_empty()
        && trace.segment_layers.len() + 1 == trace.points.len()
        && trace
            .segment_layers
            .iter()
            .all(|layer| layer == &trace.layer)
}

fn select_repair_component(
    candidate: &CandidateArtifact,
    validation: &ExactValidationAssessment,
    maximum_branches: usize,
) -> Option<(Vec<usize>, usize)> {
    let eligible = candidate
        .traces
        .iter()
        .enumerate()
        .filter(|(_, trace)| trace_is_repair_eligible(trace))
        .map(|(index, trace)| (trace.branch.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut adjacency = BTreeMap::<usize, BTreeSet<usize>>::new();
    let mut edges = BTreeSet::<(usize, usize)>::new();
    for violation in validation
        .geometry
        .violations
        .iter()
        .filter(|violation| violation.code == "trace_trace_clearance")
    {
        let mut indices = violation
            .objects
            .iter()
            .filter_map(|object| eligible.get(object.as_str()).copied())
            .collect::<Vec<_>>();
        indices.sort_unstable();
        indices.dedup();
        for left_position in 0..indices.len() {
            for &right in &indices[left_position + 1..] {
                let left = indices[left_position];
                if candidate.traces[left].electrical_net == candidate.traces[right].electrical_net
                    || candidate.traces[left].layer != candidate.traces[right].layer
                {
                    continue;
                }
                let edge = (left.min(right), left.max(right));
                if edges.insert(edge) {
                    adjacency.entry(left).or_default().insert(right);
                    adjacency.entry(right).or_default().insert(left);
                }
            }
        }
    }
    let mut unvisited = adjacency.keys().copied().collect::<BTreeSet<_>>();
    let mut components = Vec::new();
    while let Some(seed) = unvisited.pop_first() {
        let mut stack = vec![seed];
        let mut component = BTreeSet::from([seed]);
        while let Some(current) = stack.pop() {
            for &next in adjacency.get(&current).into_iter().flatten() {
                if component.insert(next) {
                    unvisited.remove(&next);
                    stack.push(next);
                }
            }
        }
        if component.len() < 2 || component.len() > maximum_branches {
            continue;
        }
        let edge_count = edges
            .iter()
            .filter(|(left, right)| component.contains(left) && component.contains(right))
            .count();
        let mut indices = component.into_iter().collect::<Vec<_>>();
        indices.sort_by(|left, right| {
            candidate.traces[*left]
                .branch
                .cmp(&candidate.traces[*right].branch)
        });
        components.push((indices, edge_count));
    }
    components.sort_by(|left, right| {
        left.0.len().cmp(&right.0.len()).then_with(|| {
            left.0
                .iter()
                .map(|index| candidate.traces[*index].branch.as_str())
                .cmp(
                    right
                        .0
                        .iter()
                        .map(|index| candidate.traces[*index].branch.as_str()),
                )
        })
    });
    components.into_iter().next()
}

fn fixed_context_from_candidate(
    problem: &Problem,
    candidate: &CandidateArtifact,
    selected: &BTreeSet<usize>,
) -> Result<Vec<RouteFamilyFixedContextInput>, String> {
    let mut context = Vec::new();
    for (trace_index, trace) in candidate.traces.iter().enumerate() {
        if selected.contains(&trace_index) {
            continue;
        }
        if trace.points.len() < 2 || trace.segment_layers.len() + 1 != trace.points.len() {
            return Err(format!(
                "fixed-context trace {} has invalid point/layer geometry",
                trace.branch
            ));
        }
        let mut start_segment = 0;
        while start_segment < trace.segment_layers.len() {
            let layer = &trace.segment_layers[start_segment];
            let mut end_segment = start_segment + 1;
            while end_segment < trace.segment_layers.len()
                && trace.segment_layers[end_segment] == *layer
            {
                end_segment += 1;
            }
            context.push(RouteFamilyFixedContextInput {
                context_id: format!("{}#run:{}", trace.branch, start_segment),
                branch: trace.branch.clone(),
                electrical_net: trace.electrical_net.clone(),
                layer: layer.clone(),
                physical_width: trace.width,
                clearance: problem.rules.clearance,
                points: trace.points[start_segment..=end_segment].to_vec(),
            });
            start_segment = end_segment;
        }
        for (via_index, via) in trace.vias.iter().enumerate() {
            for layer in BTreeSet::from([via.from_layer.as_str(), via.to_layer.as_str()]) {
                context.push(RouteFamilyFixedContextInput {
                    context_id: format!(
                        "{}#via:{}:point:{}:{}",
                        trace.branch, via_index, via.point_index, layer
                    ),
                    branch: trace.branch.clone(),
                    electrical_net: trace.electrical_net.clone(),
                    layer: layer.to_owned(),
                    physical_width: via.diameter,
                    clearance: problem.rules.clearance,
                    points: vec![via.position],
                });
            }
        }
    }
    Ok(context)
}

fn selected_trace_conflicts(
    validation: &ExactValidationAssessment,
    selected: &BTreeSet<&str>,
) -> usize {
    validation
        .geometry
        .violations
        .iter()
        .filter(|violation| {
            violation.code == "trace_trace_clearance"
                && violation
                    .objects
                    .iter()
                    .any(|object| selected.contains(object.as_str()))
        })
        .count()
}

pub fn materialize_route_family_portfolio_at_poses(
    problem: &Problem,
    components: &[SolvedComponent],
    portfolio: &RouteFamilyPortfolio,
    limits: FamilyAssignmentLimits,
) -> Result<RouteFamilyPortfolioMaterializationResult, String> {
    problem.check_schema()?;
    portfolio.validate().map_err(|issues| {
        let summary = issues
            .iter()
            .take(5)
            .map(|issue| format!("{} at {}", issue.code, issue.path))
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "route-family portfolio failed validation with {} issue(s): {summary}",
            issues.len()
        )
    })?;
    let placement = pcb_validate::validate_geometry(problem, components, &[], &[]);
    if !placement.complete {
        return Err(format!(
            "route-family portfolio placement failed exact geometry with {} finding(s)",
            placement.violations.len()
        ));
    }
    let assignment = select_route_families(portfolio, limits).map_err(|error| error.to_string())?;
    let FamilyAssignmentOutcome::Selected { selection, .. } = &assignment else {
        return Ok(RouteFamilyPortfolioMaterializationResult {
            portfolio: portfolio.clone(),
            assignment,
            candidate: None,
            validation: None,
        });
    };
    let poses = component_pose_map(components)?;
    let branches = compile_corridor_branches(problem, &poses)?
        .into_iter()
        .map(|branch| (branch.id.clone(), branch))
        .collect::<BTreeMap<_, _>>();
    let mut routes = Vec::new();
    for variable in &portfolio.variables {
        let family_id = selection
            .choices
            .get(&variable.branch)
            .ok_or_else(|| format!("selection lacks branch {}", variable.branch))?;
        let family = variable
            .families
            .iter()
            .find(|family| &family.family_id == family_id)
            .ok_or_else(|| {
                format!(
                    "selection names unknown family {family_id} for {}",
                    variable.branch
                )
            })?;
        let mut runs = family.runs.iter().collect::<Vec<_>>();
        runs.sort_by_key(|run| run.run_index);
        let first_layer = runs
            .first()
            .map(|run| run.layer.as_str())
            .ok_or_else(|| format!("selected family {family_id} has no runs"))?;
        let last_layer = runs.last().unwrap().layer.as_str();
        let branch = branches
            .get(&variable.branch)
            .ok_or_else(|| format!("portfolio contains undeclared branch {}", variable.branch))?;
        if branch.electrical_net != variable.electrical_net {
            return Err(format!(
                "portfolio branch {} changes electrical identity",
                variable.branch
            ));
        }
        let (from, from_obstacle) =
            terminal_attachment(problem, &poses, &branch.from, first_layer)?;
        let (to, to_obstacle) = terminal_attachment(problem, &poses, &branch.to, last_layer)?;
        let mut reference = if branch.reference.len() >= 2 {
            branch.reference.clone()
        } else {
            vec![from, to]
        };
        reference[0] = from;
        let last = reference.len() - 1;
        reference[last] = to;
        routes.push(OwnedRouteFamilyInput {
            branch: branch.id.clone(),
            electrical_net: branch.electrical_net.clone(),
            layer: first_layer.into(),
            end_layer: last_layer.into(),
            allowed_layers: branch.allowed_layers.clone(),
            physical_width: branch.width,
            tension_weight: branch.tension_weight,
            clearance: problem.rules.clearance,
            from_obstacle,
            to_obstacle,
            from,
            to,
            reference,
            from_terminal: branch.from.clone(),
            to_terminal: branch.to.clone(),
            from_node: terminal_node_id(&branch.electrical_net, &branch.from),
            to_node: terminal_node_id(&branch.electrical_net, &branch.to),
        });
    }
    routes.sort_by(|left, right| left.branch.cmp(&right.branch));
    let candidate =
        materialize_selected_candidate(problem, components, &routes, portfolio, &assignment)?;
    let validation = validate_candidate(problem, &candidate)?;
    Ok(RouteFamilyPortfolioMaterializationResult {
        portfolio: portfolio.clone(),
        assignment,
        candidate: Some(candidate),
        validation: Some(validation),
    })
}

fn materialize_selected_candidate(
    problem: &Problem,
    components: &[SolvedComponent],
    routes: &[OwnedRouteFamilyInput],
    portfolio: &RouteFamilyPortfolio,
    assignment: &FamilyAssignmentOutcome,
) -> Result<CandidateArtifact, String> {
    let route_by_branch = routes
        .iter()
        .map(|route| (route.branch.as_str(), route))
        .collect::<BTreeMap<_, _>>();
    let traces = materialize_selected_traces(problem, routes, portfolio, assignment)?;
    let route_graphs = build_route_graphs(&traces, &route_by_branch)?;
    let candidate = CandidateArtifact {
        schema_version: CANDIDATE_SCHEMA_VERSION,
        components: components.to_vec(),
        traces,
        route_graphs,
    };
    if candidate.traces.len()
        != problem.nets.len()
            + problem
                .electrical_nets
                .iter()
                .map(|net| net.terminals.len().saturating_sub(1))
                .sum::<usize>()
    {
        return Err("materialized route-family candidate does not cover every branch".into());
    }
    Ok(candidate)
}

fn materialize_selected_traces(
    problem: &Problem,
    routes: &[OwnedRouteFamilyInput],
    portfolio: &RouteFamilyPortfolio,
    assignment: &FamilyAssignmentOutcome,
) -> Result<Vec<SolvedTrace>, String> {
    let FamilyAssignmentOutcome::Selected { selection, .. } = assignment else {
        return Err("route-family assignment did not select choices".into());
    };
    let route_by_branch = routes
        .iter()
        .map(|route| (route.branch.as_str(), route))
        .collect::<BTreeMap<_, _>>();
    let mut traces = Vec::new();
    for variable in &portfolio.variables {
        let family_id = selection
            .choices
            .get(&variable.branch)
            .ok_or_else(|| format!("selection lacks branch {}", variable.branch))?;
        let family = variable
            .families
            .iter()
            .find(|family| &family.family_id == family_id)
            .ok_or_else(|| {
                format!(
                    "selection names unknown family {family_id} for {}",
                    variable.branch
                )
            })?;
        if !family.complete_route || family.runs.is_empty() {
            return Err(format!(
                "selected family {} is not a complete route",
                family.family_id
            ));
        }
        let route = route_by_branch
            .get(variable.branch.as_str())
            .ok_or_else(|| format!("portfolio contains unknown branch {}", variable.branch))?;
        let mut runs = family.runs.iter().collect::<Vec<_>>();
        runs.sort_by_key(|run| run.run_index);
        if runs.iter().map(|run| run.run_index).ne(0..runs.len()) {
            return Err(format!(
                "selected family {} has non-contiguous run indices",
                family.family_id
            ));
        }
        let mut points = Vec::new();
        let mut segment_layers = Vec::new();
        let mut vias = Vec::new();
        for (run_index, run) in runs.iter().enumerate() {
            if !run.witness.realizable
                || !run.witness.clearance_certified
                || !run.cut_word_certificate.basis_complete
                || !run.cut_word_certificate.verified
                || run.witness.polyline.len() < 2
            {
                return Err(format!(
                    "selected family {} run {} lacks a complete certified witness",
                    family.family_id, run.run_id
                ));
            }
            let expected_start = points.len().saturating_sub(1);
            if run.end_point <= run.start_point
                || run.start_point != expected_start
                || run.end_point - run.start_point + 1 != run.witness.polyline.len()
            {
                return Err(format!(
                    "selected family {} run {} point range does not match materialized geometry",
                    family.family_id, run.run_id
                ));
            }
            if run_index == 0 {
                if !matches!(run.transition_from_previous, RequiredNullable::Null) {
                    return Err(format!(
                        "selected family {} first run has a transition",
                        family.family_id
                    ));
                }
                points.extend(
                    run.witness
                        .polyline
                        .iter()
                        .map(|point| Vec2::new(point.x, point.y)),
                );
            } else {
                let previous = runs[run_index - 1];
                let RequiredNullable::Value(transition) = &run.transition_from_previous else {
                    return Err(format!(
                        "selected family {} run {} lacks a transition",
                        family.family_id, run.run_id
                    ));
                };
                let previous_end = previous.witness.polyline.last().unwrap();
                let current_start = run.witness.polyline.first().unwrap();
                if !transition.certified
                    || !family_points_match(previous_end, &transition.position)
                    || !family_points_match(current_start, &transition.position)
                {
                    return Err(format!(
                        "selected family {} run {} has an uncertified or disconnected transition",
                        family.family_id, run.run_id
                    ));
                }
                let expected_kind = if previous.layer == run.layer {
                    RouteTransitionKind::Continuous
                } else {
                    RouteTransitionKind::Via
                };
                if transition.kind != expected_kind {
                    return Err(format!(
                        "selected family {} run {} transition kind does not match its layers",
                        family.family_id, run.run_id
                    ));
                }
                if transition.kind == RouteTransitionKind::Via {
                    let via = transition.via.as_ref().ok_or_else(|| {
                        format!(
                            "selected family {} run {} lacks via geometry",
                            family.family_id, run.run_id
                        )
                    })?;
                    if !approximately_same(via.diameter, problem.rules.via_diameter)
                        || !approximately_same(via.drill, problem.rules.via_drill)
                    {
                        return Err(format!(
                            "selected family {} run {} via geometry does not match board rules",
                            family.family_id, run.run_id
                        ));
                    }
                    vias.push(SolvedVia {
                        position: Vec2::new(transition.position.x, transition.position.y),
                        from_layer: previous.layer.clone(),
                        to_layer: run.layer.clone(),
                        diameter: via.diameter,
                        drill: via.drill,
                        point_index: points.len() - 1,
                    });
                }
                points.extend(
                    run.witness
                        .polyline
                        .iter()
                        .skip(1)
                        .map(|point| Vec2::new(point.x, point.y)),
                );
            }
            segment_layers.extend(std::iter::repeat_n(
                run.layer.clone(),
                run.witness.polyline.len() - 1,
            ));
            if points.len() - 1 != run.end_point {
                return Err(format!(
                    "selected family {} run {} ended at the wrong point index",
                    family.family_id, run.run_id
                ));
            }
        }
        let first_run = runs[0];
        let homotopy = if runs.len() == 1 {
            HomotopyWord::reduced(first_run.cut_word_certificate.word.iter().map(|event| {
                CutCrossing {
                    obstacle: event.obstacle.clone(),
                    direction: match event.direction {
                        CutDirection::Positive => CrossingDirection::Positive,
                        CutDirection::Negative => CrossingDirection::Negative,
                    },
                }
            }))
        } else {
            // RouteClass is a legacy single-layer compatibility field.
            // Segment layers, vias, and per-run portfolio certificates are
            // authoritative for multi-run candidates.
            HomotopyWord::empty()
        };
        traces.push(SolvedTrace {
            branch: variable.branch.clone(),
            electrical_net: variable.electrical_net.clone(),
            net: variable.branch.clone(),
            from_node: route.from_node.clone(),
            to_node: route.to_node.clone(),
            width: route.physical_width,
            tension_weight: route.tension_weight,
            layer: first_run.layer.clone(),
            segment_layers,
            vias,
            points,
            route_class: RouteClass {
                layer: first_run.layer.clone(),
                from: TerminalSector {
                    component: route.from_terminal.component.clone(),
                    sector: route.from_terminal.pin.clone(),
                },
                to: TerminalSector {
                    component: route.to_terminal.component.clone(),
                    sector: route.to_terminal.pin.clone(),
                },
                homotopy,
            },
            route_basis_fingerprint: if runs.len() == 1 {
                Some(first_run.cut_word_certificate.cut_basis_fingerprint.clone())
            } else {
                None
            },
        });
    }
    traces.sort_by(|left, right| left.branch.cmp(&right.branch));
    Ok(traces)
}

fn family_points_match(
    left: &layout_trace_routing::family_ir::FamilyPoint,
    right: &layout_trace_routing::family_ir::FamilyPoint,
) -> bool {
    let x_scale = left.x.abs().max(right.x.abs()).max(1.0);
    let y_scale = left.y.abs().max(right.y.abs()).max(1.0);
    (left.x - right.x).abs() <= 1.0e-9 * x_scale && (left.y - right.y).abs() <= 1.0e-9 * y_scale
}

fn approximately_same(left: f64, right: f64) -> bool {
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= 1.0e-9 * scale
}

fn build_route_graphs(
    traces: &[SolvedTrace],
    routes: &BTreeMap<&str, &OwnedRouteFamilyInput>,
) -> Result<Vec<SolvedRouteGraph>, String> {
    let mut nets = BTreeMap::<String, (BTreeMap<String, SolvedRouteNode>, BTreeSet<String>)>::new();
    for trace in traces {
        let route = routes
            .get(trace.branch.as_str())
            .ok_or_else(|| format!("missing route intent for {}", trace.branch))?;
        let entry = nets.entry(trace.electrical_net.clone()).or_default();
        for (terminal, position) in [
            (&route.from_terminal, trace.points[0]),
            (&route.to_terminal, *trace.points.last().unwrap()),
        ] {
            let node_id = terminal_node_id(&trace.electrical_net, terminal);
            let node = entry
                .0
                .entry(node_id.clone())
                .or_insert_with(|| SolvedRouteNode {
                    id: node_id,
                    electrical_net: trace.electrical_net.clone(),
                    position,
                    incident_branches: Vec::new(),
                    kind: SolvedRouteNodeKind::Terminal {
                        component: terminal.component.clone(),
                        pin: terminal.pin.clone(),
                    },
                });
            node.incident_branches.push(trace.branch.clone());
            node.incident_branches.sort();
            node.incident_branches.dedup();
        }
        for (via_index, via) in trace.vias.iter().enumerate() {
            let node_id = format!(
                "via:{}/{}:{via_index}@{}",
                trace.electrical_net, trace.branch, via.point_index
            );
            if entry
                .0
                .insert(
                    node_id.clone(),
                    SolvedRouteNode {
                        id: node_id.clone(),
                        electrical_net: trace.electrical_net.clone(),
                        position: via.position,
                        incident_branches: vec![trace.branch.clone()],
                        kind: SolvedRouteNodeKind::Via {
                            point_index: via.point_index,
                            from_layer: via.from_layer.clone(),
                            to_layer: via.to_layer.clone(),
                            diameter: via.diameter,
                            drill: via.drill,
                        },
                    },
                )
                .is_some()
            {
                return Err(format!("duplicate route-family via node {node_id}"));
            }
        }
        entry.1.insert(trace.branch.clone());
    }
    Ok(nets
        .into_iter()
        .map(|(electrical_net, (nodes, branches))| SolvedRouteGraph {
            electrical_net,
            nodes: nodes.into_values().collect(),
            branches: branches.into_iter().collect(),
        })
        .collect())
}

fn terminal_node_id(net: &str, terminal: &PinRef) -> String {
    format!("terminal:{net}/{}.{}", terminal.component, terminal.pin)
}

fn unavailable_detail_and_work(
    outcome: RouteFamilyPortfolioBuildOutcome,
) -> (String, RouteFamilyProductionWork) {
    match outcome {
        RouteFamilyPortfolioBuildOutcome::Complete { work, .. } => (
            "internal planning boundary returned an unassigned complete portfolio".into(),
            work,
        ),
        RouteFamilyPortfolioBuildOutcome::Unsupported {
            route,
            reason,
            detail,
            work,
        } => (
            format!("unsupported route {route:?}: {reason:?}: {detail}"),
            work,
        ),
        RouteFamilyPortfolioBuildOutcome::GenerationBudgetExhausted {
            route,
            termination,
            work,
        } => (
            format!("family generation budget exhausted for {route}: {termination:?}"),
            work,
        ),
        RouteFamilyPortfolioBuildOutcome::NoFamily {
            route,
            termination,
            work,
        } => (
            format!("no certified family for {route}: {termination:?}"),
            work,
        ),
        RouteFamilyPortfolioBuildOutcome::CertifiableFrontierIncomplete {
            route,
            accepted,
            requested_raw_bound,
            termination,
            work,
        } => (
            format!(
                "certifiable frontier incomplete for {route}: accepted {accepted}/{requested_raw_bound}, termination={termination:?}"
            ),
            work,
        ),
        RouteFamilyPortfolioBuildOutcome::InvalidProducedPortfolio { issues, work } => (
            format!(
                "produced portfolio failed validation with {} issue(s)",
                issues.len()
            ),
            work,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected_power_slice() -> (
        Problem,
        CandidateArtifact,
        RouteFamilyPortfolio,
        FamilyAssignmentOutcome,
    ) {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/esp32-slices/power-west-capacitor-interaction.json"
        ))
        .unwrap();
        let components = problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        let solved = analyze_problem_route_families_at_poses(
            &problem,
            &components,
            &RouteFamilyAnalysisConfig::default(),
        )
        .unwrap();
        (
            problem,
            solved.candidate.unwrap(),
            solved.portfolio.unwrap(),
            solved.assignment.unwrap(),
        )
    }

    fn add_pair_obligation(
        assignment: &mut FamilyAssignmentOutcome,
        left_family_id: String,
        right_family_id: String,
    ) {
        let FamilyAssignmentOutcome::Selected { selection, .. } = assignment else {
            panic!("fixture must select families");
        };
        selection
            .repair_obligations
            .push(FamilySelectionRepairObligation::FamilyPair {
                left_family_id,
                right_family_id,
                kind: FamilyConflictKind::CopperClearance,
                evidence: layout_trace_routing::family_ir::ConflictEvidence {
                    layer: RequiredNullable::Value("top".into()),
                    passage_key: RequiredNullable::Null,
                    detail: "test-injected continuous shortfall".into(),
                },
            });
    }

    #[test]
    fn native_seeded_via_family_producer_is_exact_and_fail_closed() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/family-multi-run-via.json"
        ))
        .unwrap();
        let components = problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        let result = analyze_problem_route_families_at_poses(
            &problem,
            &components,
            &RouteFamilyAnalysisConfig::default(),
        )
        .unwrap();
        assert!(result.complete(), "{}", result.evidence.detail);
        let mut portfolio = result.portfolio.unwrap();
        assert_eq!(
            portfolio.schema_version,
            MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
        );
        assert_eq!(portfolio.variables[0].families.len(), 5);
        assert_eq!(result.production_work.family_searches, 10);
        let candidate = result.candidate.unwrap();
        assert_eq!(candidate.traces[0].segment_layers, ["top", "bottom"]);
        assert_eq!(candidate.traces[0].vias.len(), 1);
        assert_eq!(candidate.traces[0].vias[0].point_index, 1);
        assert!(candidate.route_graphs[0].nodes.iter().any(|node| {
            matches!(
                node.kind,
                SolvedRouteNodeKind::Via {
                    point_index: 1,
                    ref from_layer,
                    ref to_layer,
                    ..
                } if from_layer == "top" && to_layer == "bottom"
            )
        }));

        let mut wrong_transition = portfolio.clone();
        let RequiredNullable::Value(transition) =
            &mut wrong_transition.variables[0].families[0].runs[1].transition_from_previous
        else {
            unreachable!()
        };
        transition.kind = RouteTransitionKind::Continuous;
        wrong_transition.seal_pair_analysis_fingerprints().unwrap();
        assert!(wrong_transition.validate().is_err());

        let RequiredNullable::Value(transition) =
            &mut portfolio.variables[0].families[0].runs[1].transition_from_previous
        else {
            unreachable!()
        };
        transition.via.as_mut().unwrap().minimum_clearance = 0.1;
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        assert!(
            portfolio
                .validate()
                .unwrap_err()
                .iter()
                .any(|issue| { issue.code == "run_transition_via_clearance_shortfall" })
        );

        let zero_budget = RouteFamilyAnalysisConfig {
            route_repair_work_budget: Some(RouteRepairWorkBudget {
                settled_states: 0,
                visibility_pair_evaluations: 0,
                visibility_geometry_units: 0,
            }),
            ..RouteFamilyAnalysisConfig::default()
        };
        let bounded =
            analyze_problem_route_families_at_poses(&problem, &components, &zero_budget).unwrap();
        assert!(!bounded.complete());
        assert_eq!(
            bounded.evidence.status,
            RouteFamilyAnalysisStatus::PortfolioUnavailable
        );
        assert!(bounded.evidence.detail.contains("budget exhausted"));
        assert_eq!(bounded.production_work.family_searches, 1);
    }

    #[test]
    fn checked_in_v5_via_portfolio_is_exact_complete() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/family-multi-run-via.json"
        ))
        .unwrap();
        let mut portfolio: RouteFamilyPortfolio = serde_json::from_str(include_str!(
            "../../../benchmarks/small/family-multi-run-via.portfolio.json"
        ))
        .unwrap();
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        portfolio.validate().unwrap();
        let components = problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();

        let result = materialize_route_family_portfolio_at_poses(
            &problem,
            &components,
            &portfolio,
            FamilyAssignmentLimits::default(),
        )
        .unwrap();

        assert!(result.complete());
        let candidate = result.candidate.unwrap();
        assert_eq!(candidate.traces[0].segment_layers, ["top", "bottom"]);
        assert_eq!(candidate.traces[0].vias.len(), 1);
    }

    #[test]
    fn seeded_via_portfolio_rejects_blocked_midpoint_and_retains_spatial_alternatives() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/family-seeded-via-obstacle.json"
        ))
        .unwrap();
        let components = problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();

        let result = analyze_problem_route_families_at_poses(
            &problem,
            &components,
            &RouteFamilyAnalysisConfig::default(),
        )
        .unwrap();

        assert!(result.complete(), "{}", result.evidence.detail);
        let portfolio = result.portfolio.as_ref().unwrap();
        assert_eq!(portfolio.variables[0].families.len(), 8);
        let via_x = portfolio.variables[0]
            .families
            .iter()
            .map(|family| {
                let RequiredNullable::Value(transition) = &family.runs[1].transition_from_previous
                else {
                    unreachable!()
                };
                transition.position.x.to_bits()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(via_x.len(), 4);
        assert!(!via_x.contains(&10.0f64.to_bits()));
        assert!(
            result
                .production_work
                .rejection_evidence
                .iter()
                .any(|rejection| {
                    rejection.reason == "via_clearance_not_certified"
                        && rejection.detail.contains("(10.000000,6.000000)")
                })
        );
        assert_eq!(result.production_work.family_searches, 8);
        assert_ne!(
            result.candidate.as_ref().unwrap().traces[0].vias[0]
                .position
                .x,
            10.0
        );

        let mut disabled = RouteFamilyAnalysisConfig::default();
        disabled.seeded_via_families.enabled = false;
        let control =
            analyze_problem_route_families_at_poses(&problem, &components, &disabled).unwrap();
        assert!(!control.complete());
        assert_eq!(
            control.portfolio.as_ref().unwrap().schema_version,
            layout_trace_routing::family_ir::ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
        );
    }

    #[test]
    fn seeded_via_two_net_assignment_avoids_via_conflicts_and_fails_closed() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/family-seeded-via-two-net.json"
        ))
        .unwrap();
        let components = problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();

        let result = analyze_problem_route_families_at_poses(
            &problem,
            &components,
            &RouteFamilyAnalysisConfig::default(),
        )
        .unwrap();
        assert!(result.complete(), "{}", result.evidence.detail);
        assert_eq!(result.production_work.family_searches, 20);
        let portfolio = result.portfolio.as_ref().unwrap();
        assert_eq!(portfolio.variables.len(), 2);
        assert_eq!(portfolio.pair_analysis.analyzed_family_pairs, Some(40));
        assert!(portfolio.pair_analysis.conflicts.iter().any(|conflict| {
            conflict.kind == FamilyConflictKind::CopperClearance
                && conflict.evidence.detail.contains("first_primitive=via:")
                && conflict.evidence.detail.contains("second_primitive=via:")
        }));
        let FamilyAssignmentOutcome::Selected { selection, .. } =
            result.assignment.as_ref().unwrap()
        else {
            unreachable!()
        };
        assert!(selection.repair_obligations.is_empty());
        let via_x = result
            .candidate
            .as_ref()
            .unwrap()
            .traces
            .iter()
            .map(|trace| trace.vias[0].position.x.to_bits())
            .collect::<BTreeSet<_>>();
        assert_eq!(via_x.len(), 2);

        let mut one_site = RouteFamilyAnalysisConfig::default();
        one_site.seeded_via_families.maximum_via_sites = 1;
        let bounded =
            analyze_problem_route_families_at_poses(&problem, &components, &one_site).unwrap();
        assert_eq!(
            bounded.evidence.status,
            RouteFamilyAnalysisStatus::BoundedInfeasible
        );
        assert!(bounded.candidate.is_none());
        assert_eq!(bounded.production_work.family_searches, 4);

        let mut repair_policy = RouteFamilyAnalysisConfig::default();
        repair_policy.seeded_via_families.clearance_policy =
            FamilyClearancePolicy::RepairObligation;
        let unsupported_repair =
            analyze_problem_route_families_at_poses(&problem, &components, &repair_policy).unwrap();
        assert_eq!(
            unsupported_repair.evidence.status,
            RouteFamilyAnalysisStatus::ExactRejected
        );
        assert!(unsupported_repair.candidate.is_some());
        assert!(
            unsupported_repair
                .validation
                .as_ref()
                .is_some_and(|validation| !validation.complete)
        );
        assert!(
            unsupported_repair
                .evidence
                .detail
                .contains("continuous obligation handler cannot process")
        );
    }

    #[test]
    fn power_slice_gets_a_joint_exact_family_assignment() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/esp32-slices/power-west-capacitor-interaction.json"
        ))
        .unwrap();
        let components = problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        let config = RouteFamilyAnalysisConfig {
            route_repair_work_budget: Some(RouteRepairWorkBudget::default()),
            ..RouteFamilyAnalysisConfig::default()
        };
        let result =
            analyze_problem_route_families_at_poses(&problem, &components, &config).unwrap();
        assert!(result.complete(), "{}", result.evidence.detail);
        assert!(result.portfolio.is_some());
        assert!(matches!(
            result.assignment,
            Some(FamilyAssignmentOutcome::Selected { .. })
        ));
    }

    #[test]
    fn exhaustive_short_frontiers_produce_a_natural_obligation_solved_by_component_coupling() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/family-clearance-obligation.json"
        ))
        .unwrap();
        let components = problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        let result = analyze_problem_route_families_at_poses(
            &problem,
            &components,
            &RouteFamilyAnalysisConfig::default(),
        )
        .unwrap();
        let FamilyAssignmentOutcome::Selected { selection, .. } =
            result.assignment.as_ref().unwrap()
        else {
            panic!("the exhaustive one-family domains must be selected");
        };
        assert_eq!(selection.repair_obligations.len(), 1);
        assert!(matches!(
            selection.repair_obligations[0],
            FamilySelectionRepairObligation::FamilyPair {
                kind: FamilyConflictKind::CopperClearance,
                ..
            }
        ));
        let repair = result.continuous_repair.as_ref().unwrap();
        assert_eq!(
            repair.status,
            ContinuousObligationRepairStatus::ExactComplete
        );
        assert_eq!(repair.segment_pairs, 1);
        assert_eq!(repair.frames.len(), 9);
        assert_eq!(result.evidence.status, RouteFamilyAnalysisStatus::Selected);
        assert!(result.validation.as_ref().unwrap().complete);
        for id in ["A0", "B0"] {
            assert!(
                result
                    .candidate
                    .as_ref()
                    .unwrap()
                    .components
                    .iter()
                    .find(|component| component.id == id)
                    .unwrap()
                    .position
                    .y
                    < 2.999
            );
        }
    }

    #[test]
    fn body_neighborhood_contact_prevents_conflict_substitution() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/family-clearance-body-neighborhood.json"
        ))
        .unwrap();
        let components = problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();

        let coupled = analyze_problem_route_families_at_poses(
            &problem,
            &components,
            &RouteFamilyAnalysisConfig::default(),
        )
        .unwrap();
        assert!(coupled.complete(), "{}", coupled.evidence.detail);
        let repair = coupled.continuous_repair.as_ref().unwrap();
        assert_eq!(repair.segment_pairs, 1);
        assert_eq!(repair.segment_body_pairs, 2);
        assert!(repair.frames.iter().any(|frame| {
            frame
                .constraints
                .iter()
                .any(|constraint| constraint.family == "segment_body_clearance")
        }));
        let coupled_components = &coupled.candidate.as_ref().unwrap().components;
        assert!(
            coupled_components
                .iter()
                .find(|component| component.id == "A0")
                .unwrap()
                .position
                .y
                > 3.0009
        );
        assert!(
            coupled_components
                .iter()
                .find(|component| component.id == "A1")
                .unwrap()
                .position
                .y
                > 3.751
        );

        let ablated_config = RouteFamilyAnalysisConfig {
            continuous_obligation_repair: ContinuousObligationRepairConfig {
                include_body_contacts: false,
                ..ContinuousObligationRepairConfig::default()
            },
            ..RouteFamilyAnalysisConfig::default()
        };
        let ablated =
            analyze_problem_route_families_at_poses(&problem, &components, &ablated_config)
                .unwrap();
        assert_eq!(
            ablated.evidence.status,
            RouteFamilyAnalysisStatus::ExactRejected
        );
        assert_eq!(
            ablated
                .continuous_repair
                .as_ref()
                .unwrap()
                .segment_body_pairs,
            0
        );
        assert!(
            ablated
                .validation
                .as_ref()
                .unwrap()
                .geometry
                .violations
                .iter()
                .any(|violation| violation.code == "trace_obstacle_clearance")
        );
    }

    #[test]
    fn natural_obligation_with_fixed_endpoint_bodies_rolls_back() {
        let mut problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/family-clearance-obligation.json"
        ))
        .unwrap();
        for component in &mut problem.components {
            component.constraints.movement = Movement::Fixed;
        }
        let components = problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        let result = analyze_problem_route_families_at_poses(
            &problem,
            &components,
            &RouteFamilyAnalysisConfig::default(),
        )
        .unwrap();
        let repair = result.continuous_repair.as_ref().unwrap();
        assert_eq!(repair.status, ContinuousObligationRepairStatus::RolledBack);
        assert_eq!(
            result.evidence.status,
            RouteFamilyAnalysisStatus::ExactRejected
        );
        assert_eq!(result.validation.as_ref().unwrap().violations.len(), 1);
    }

    #[test]
    fn fixed_context_repair_path_runs_its_natural_continuous_obligation() {
        let movable_problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/family-clearance-obligation.json"
        ))
        .unwrap();
        let mut fixed_problem = movable_problem.clone();
        for component in &mut fixed_problem.components {
            component.constraints.movement = Movement::Fixed;
        }
        let components = fixed_problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        let conflicted = analyze_problem_route_families_at_poses(
            &fixed_problem,
            &components,
            &RouteFamilyAnalysisConfig::default(),
        )
        .unwrap()
        .candidate
        .unwrap();
        let repaired = repair_candidate_route_families(
            &movable_problem,
            &conflicted,
            &RouteFamilyAnalysisConfig::default(),
        )
        .unwrap();
        assert!(repaired.complete(), "{}", repaired.evidence.detail);
        let continuous = repaired.continuous_repair.as_ref().unwrap();
        assert_eq!(
            continuous.status,
            ContinuousObligationRepairStatus::ExactComplete
        );
        assert_eq!(continuous.obligation_count, 1);
        assert_eq!(continuous.frames.len(), 9);
        assert!(repaired.validation.as_ref().unwrap().complete);
    }

    #[test]
    fn fixed_context_repair_selects_only_the_connected_power_conflict() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/esp32-slices/power-west-capacitor-interaction.json"
        ))
        .unwrap();
        let components = problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        let solved = analyze_problem_route_families_at_poses(
            &problem,
            &components,
            &RouteFamilyAnalysisConfig::default(),
        )
        .unwrap();
        let mut conflicted = solved.candidate.unwrap();
        for trace in conflicted
            .traces
            .iter_mut()
            .filter(|trace| matches!(trace.branch.as_str(), "W_CA_G" | "W_CA_V"))
        {
            let from = trace.points[0];
            let to = *trace.points.last().unwrap();
            trace.points = vec![from, Vec2::new(9.0, 34.0), Vec2::new(10.0, 34.0), to];
            trace.segment_layers = vec!["top".into(); 3];
            trace.route_basis_fingerprint = None;
        }
        let before = validate_candidate(&problem, &conflicted).unwrap();
        assert!(
            before
                .geometry
                .violations
                .iter()
                .any(|violation| violation.code == "trace_trace_clearance")
        );
        let repaired = repair_candidate_route_families(
            &problem,
            &conflicted,
            &RouteFamilyAnalysisConfig::default(),
        )
        .unwrap();
        assert_eq!(
            repaired.evidence.selected_branches,
            vec![
                "W_CA_G".to_string(),
                "W_CA_V".to_string(),
                "W_CB_V".to_string()
            ]
        );
        assert!(repaired.accepted(), "{}", repaired.evidence.detail);
        assert_eq!(repaired.evidence.proposed_selected_conflicts, Some(0));
    }

    #[test]
    fn selected_family_pair_obligation_is_continuously_improved_and_exact_gated() {
        let (problem, mut candidate, portfolio, mut assignment) = selected_power_slice();
        let choices = match &assignment {
            FamilyAssignmentOutcome::Selected { selection, .. } => selection.choices.clone(),
            _ => unreachable!(),
        };
        let moving = candidate
            .traces
            .iter_mut()
            .find(|trace| trace.branch == "W_CB_G")
            .unwrap();
        moving.points[1] = Vec2::new(9.55, 36.7);
        moving.points[2] = Vec2::new(9.45, 35.6);
        moving.points[3] = Vec2::new(8.95, 34.5);
        moving.route_basis_fingerprint = None;
        let before = validate_candidate(&problem, &candidate).unwrap();
        assert!(exact_repair_score(&before).0 > 0);
        add_pair_obligation(
            &mut assignment,
            choices["W_CB_G"].clone(),
            choices["W_CA_V"].clone(),
        );
        let repaired = repair_selected_family_obligations(
            &problem,
            &candidate,
            &portfolio,
            &assignment,
            ContinuousObligationRepairConfig::default(),
        )
        .unwrap();
        assert!(repaired.accepted(), "{}", repaired.detail);
        assert!(exact_repair_score(&repaired.validation) < exact_repair_score(&before));
        assert_eq!(repaired.frames.len(), 9);
        assert!(repaired.segment_pairs > 0);
    }

    #[test]
    fn immobile_two_point_obligation_rolls_back_the_candidate() {
        let (mut problem, mut candidate, portfolio, mut assignment) = selected_power_slice();
        // A two-point trace is no longer intrinsically immobile: its endpoints
        // transfer clearance forces into their component bodies. Make the
        // intended negative control explicit by fixing those bodies.
        for component in &mut problem.components {
            component.constraints.movement = Movement::Fixed;
        }
        let choices = match &assignment {
            FamilyAssignmentOutcome::Selected { selection, .. } => selection.choices.clone(),
            _ => unreachable!(),
        };
        let moving = candidate
            .traces
            .iter_mut()
            .find(|trace| trace.branch == "W_CB_G")
            .unwrap();
        moving.points = vec![moving.points[0], *moving.points.last().unwrap()];
        moving.segment_layers = vec!["top".into()];
        moving.route_basis_fingerprint = None;
        let before = validate_candidate(&problem, &candidate).unwrap();
        assert!(exact_repair_score(&before).0 > 0);
        add_pair_obligation(
            &mut assignment,
            choices["W_CB_G"].clone(),
            choices["W_CA_V"].clone(),
        );
        let repaired = repair_selected_family_obligations(
            &problem,
            &candidate,
            &portfolio,
            &assignment,
            ContinuousObligationRepairConfig::default(),
        )
        .unwrap();
        assert_eq!(
            repaired.status,
            ContinuousObligationRepairStatus::RolledBack
        );
        assert_eq!(repaired.candidate, candidate);
        assert_eq!(repaired.validation, before);
    }
}

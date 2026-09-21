// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Strict production producer for bounded route-family portfolios.
//!
//! This slice accepts complete, via-free, same-layer branch variables. It
//! promotes real exact-clearance corridor alternatives into the validated V4
//! fixed-witness-plus-context IR, then the separate exact assignment pass
//! chooses one family per branch.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::copper_pair::{
    FixedCopperPairClassification, FixedCopperRoute, classify_prepared_copper_pair,
    prepare_fixed_copper_route,
};
use crate::corridor::{
    AlternativeSearchTermination, CorridorGraph, EmbeddingPurpose,
    FIRST_K_ROUTE_CLASS_FAMILY_LIMIT, MAX_EXPLORATORY_ROUTE_CLASS_FAMILY_LIMIT,
    RouteClassFamilyLimitError, RouteCorridorEmbedding, RouteEmbeddingRequest, RouteRepairEvidence,
    RouteRepairWorkBudget, embed_route, enumerate_route_class_alternatives_with_limit,
    enumerate_route_class_alternatives_with_limit_and_work_budget,
};
use crate::family_assignment::{
    FamilyAssignmentError, FamilyAssignmentLimits, FamilyAssignmentOutcome, FamilyAssignmentWork,
    select_route_families,
};
use crate::family_ir::{
    ConflictEvidence, CorridorEpoch, CutDirection, CutEvent, CutWordCertificate,
    FIXED_CONTEXT_ANALYSIS_METHOD, FIXED_WITNESS_NO_SCALAR_CAPACITY_MODEL,
    FIXED_WITNESS_PAIR_ANALYSIS_METHOD, FamilyConflict, FamilyConflictKind, FamilyContextConflict,
    FamilyGenerationCertificate, FamilyPoint, FixedContextCertificate, FixedContextRun,
    PairAnalysisCertificate, PassageCapacityCertificate, PassageEnforcementMode,
    ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION, RequiredNullable, RouteFamily,
    RouteFamilyFeasibilityMode, RouteFamilyPortfolio, RouteFamilyRun, RouteFamilyVariable,
    RouteWitness, ValidationIssue,
};
use crate::model::Vec2;
use crate::topology::CrossingDirection;

pub const MAX_FAMILIES_PER_VARIABLE: usize = FIRST_K_ROUTE_CLASS_FAMILY_LIMIT;
pub const RAW_FAMILY_CANDIDATE_LIMIT: usize = 8;
pub const TARGETED_RAW_FAMILY_CANDIDATE_LIMIT: usize = MAX_EXPLORATORY_ROUTE_CLASS_FAMILY_LIMIT;
/// Independent failsafe against accidentally admitting an unbounded number of
/// generators even when a degenerate portfolio makes the work projections
/// unusually cheap. Normal admission is governed by the work caps below.
pub const MAX_TARGETED_DEEPENING_VARIABLES_SAFETY: usize = 8;
pub const MAX_TARGETED_NEW_FAMILIES: usize = 32;
pub const MAX_TARGETED_FAMILY_PAIR_ANALYSES: u64 = 4_096;
pub const MAX_TARGETED_FAMILY_CONTEXT_PAIR_ANALYSES: u64 = 32_768;
pub const MAX_INCREMENTAL_PROMOTED_VARIABLES: usize = 3;
pub const MAX_INCREMENTAL_TOTAL_VARIABLES: usize = 6;
pub const FAMILY_GENERATION_METHOD: &str =
    "bounded_clearance_offset_visibility_dijkstra/raw_k=8/k3_or_exhaustive/v3";
pub const EXPANDED_FAMILY_GENERATION_METHOD: &str =
    "bounded_clearance_offset_visibility_dijkstra/raw_k=8/all_certifiable_k3_or_exhaustive/v4";
pub const TARGETED_FAMILY_GENERATION_METHOD: &str = "bounded_clearance_offset_visibility_dijkstra/raw_k=16/implicated_budget_admission/reconcile_raw8_identities/v5";
pub const PROMOTED_FAMILY_GENERATION_METHOD: &str =
    "incremental_fixed_context_promotion/reuse_exact_old_variables/generate_promoted_raw8/v1";
pub const INCREMENTAL_TARGETED_FAMILY_GENERATION_METHOD: &str =
    "incremental_fixed_context_promotion/reuse_unchanged_variables/deepen_implicated_raw16/v1";
pub const INCREMENTAL_PROMOTION_REUSE_CONTRACT: &str =
    "layout-trace.incremental-route-family-promotion/v1";

#[derive(Clone, Copy)]
struct PortfolioBounds {
    minimum_certified_per_variable: usize,
    maximum_certified_per_variable: usize,
    raw_candidate_limit: usize,
    generation_method: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SearchPointBits {
    x: u64,
    y: u64,
}

impl From<Vec2> for SearchPointBits {
    fn from(point: Vec2) -> Self {
        Self {
            x: point.x.to_bits(),
            y: point.y.to_bits(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AlternativeSearchKey {
    graph_digest: [u8; 32],
    net: String,
    from_component: String,
    to_component: String,
    physical_width: u64,
    clearance: u64,
    from: SearchPointBits,
    from_toward: SearchPointBits,
    to: SearchPointBits,
    to_toward: SearchPointBits,
    reference: Vec<SearchPointBits>,
    target_homotopy: Option<Vec<(String, u8)>>,
    persistent_basis_matches: bool,
    requested_bound: usize,
}

struct AlternativeSearchCacheEntry {
    /// Retained canonical graph bytes make a digest match independently
    /// checkable before replay, including in the theoretical collision case.
    graph_bytes: Arc<[u8]>,
    evidence: RouteRepairEvidence,
}

#[derive(Default)]
struct AlternativeSearchCache {
    graphs_by_layer: BTreeMap<String, ([u8; 32], Arc<[u8]>)>,
    entries: BTreeMap<AlternativeSearchKey, Vec<AlternativeSearchCacheEntry>>,
    work_budget: Option<RouteRepairWorkBudget>,
}

impl AlternativeSearchCache {
    fn new(graphs: &[CorridorGraph]) -> Self {
        let graphs_by_layer = graphs
            .iter()
            .map(|graph| {
                let bytes = Arc::<[u8]>::from(
                    serde_json::to_vec(graph)
                        .expect("serializable corridor graph used for exact cache identity"),
                );
                let digest: [u8; 32] = Sha256::digest(bytes.as_ref()).into();
                (graph.layer.clone(), (digest, bytes))
            })
            .collect();
        Self {
            graphs_by_layer,
            entries: BTreeMap::new(),
            work_budget: None,
        }
    }

    fn new_with_work_budget(graphs: &[CorridorGraph], work_budget: RouteRepairWorkBudget) -> Self {
        Self {
            work_budget: Some(work_budget),
            ..Self::new(graphs)
        }
    }

    fn key(
        &self,
        graph: &CorridorGraph,
        request: &RouteEmbeddingRequest<'_>,
        requested_bound: usize,
    ) -> (AlternativeSearchKey, Arc<[u8]>) {
        let (graph_digest, graph_bytes) = &self.graphs_by_layer[&graph.layer];
        (
            AlternativeSearchKey {
                graph_digest: *graph_digest,
                net: request.net.to_owned(),
                from_component: request.from_component.to_owned(),
                to_component: request.to_component.to_owned(),
                physical_width: request.physical_width.to_bits(),
                clearance: request.clearance.to_bits(),
                from: request.from.into(),
                from_toward: request.from_toward.into(),
                to: request.to.into(),
                to_toward: request.to_toward.into(),
                reference: request.reference.iter().copied().map(Into::into).collect(),
                target_homotopy: request.target_homotopy.map(|word| cut_word_key(word)),
                persistent_basis_matches: request.persistent_basis_matches,
                requested_bound,
            },
            graph_bytes.clone(),
        )
    }

    fn enumerate(
        &mut self,
        graph: &CorridorGraph,
        request: RouteEmbeddingRequest<'_>,
        requested_bound: usize,
    ) -> (
        Result<RouteRepairEvidence, RouteClassFamilyLimitError>,
        bool,
    ) {
        let (key, graph_bytes) = self.key(graph, &request, requested_bound);
        if let Some(entries) = self.entries.get(&key)
            && let Some(entry) = entries.iter().find(|entry| {
                Arc::ptr_eq(&entry.graph_bytes, &graph_bytes)
                    || entry.graph_bytes.as_ref() == graph_bytes.as_ref()
            })
        {
            return (Ok(entry.evidence.clone()), true);
        }
        let result = if let Some(work_budget) = self.work_budget {
            enumerate_route_class_alternatives_with_limit_and_work_budget(
                graph,
                request,
                requested_bound,
                work_budget,
            )
            .map(|bounded| bounded.evidence)
        } else {
            enumerate_route_class_alternatives_with_limit(graph, request, requested_bound)
        };
        if let Ok(evidence) = &result {
            self.entries
                .entry(key)
                .or_default()
                .push(AlternativeSearchCacheEntry {
                    graph_bytes,
                    evidence: evidence.clone(),
                });
        }
        (result, false)
    }
}

#[derive(Clone, Copy)]
struct TargetedAdmissionLimits {
    variable_safety: usize,
    new_families: usize,
    family_pairs: u64,
    family_context_pairs: u64,
}

const TARGETED_ADMISSION_LIMITS: TargetedAdmissionLimits = TargetedAdmissionLimits {
    variable_safety: MAX_TARGETED_DEEPENING_VARIABLES_SAFETY,
    new_families: MAX_TARGETED_NEW_FAMILIES,
    family_pairs: MAX_TARGETED_FAMILY_PAIR_ANALYSES,
    family_context_pairs: MAX_TARGETED_FAMILY_CONTEXT_PAIR_ANALYSES,
};

const INITIAL_BOUNDS: PortfolioBounds = PortfolioBounds {
    minimum_certified_per_variable: MAX_FAMILIES_PER_VARIABLE,
    maximum_certified_per_variable: MAX_FAMILIES_PER_VARIABLE,
    raw_candidate_limit: RAW_FAMILY_CANDIDATE_LIMIT,
    generation_method: FAMILY_GENERATION_METHOD,
};

const EXPANDED_BOUNDS: PortfolioBounds = PortfolioBounds {
    minimum_certified_per_variable: MAX_FAMILIES_PER_VARIABLE,
    maximum_certified_per_variable: RAW_FAMILY_CANDIDATE_LIMIT,
    raw_candidate_limit: RAW_FAMILY_CANDIDATE_LIMIT,
    generation_method: EXPANDED_FAMILY_GENERATION_METHOD,
};

const TARGETED_BOUNDS: PortfolioBounds = PortfolioBounds {
    minimum_certified_per_variable: MAX_FAMILIES_PER_VARIABLE,
    maximum_certified_per_variable: TARGETED_RAW_FAMILY_CANDIDATE_LIMIT,
    raw_candidate_limit: TARGETED_RAW_FAMILY_CANDIDATE_LIMIT,
    generation_method: TARGETED_FAMILY_GENERATION_METHOD,
};

#[derive(Clone, Debug)]
pub struct RouteFamilyRouteInput<'a> {
    pub branch_id: &'a str,
    pub electrical_net: &'a str,
    pub layer: &'a str,
    pub physical_width: f64,
    pub clearance: f64,
    pub from_component: &'a str,
    pub to_component: &'a str,
    pub from: Vec2,
    pub from_toward: Vec2,
    pub to: Vec2,
    pub to_toward: Vec2,
    pub reference: &'a [Vec2],
}

#[derive(Clone, Debug)]
pub struct RouteFamilyPortfolioInput<'a> {
    pub portfolio_id: &'a str,
    pub placement_revision: u64,
    pub geometry_revision: u64,
    pub corridors: &'a [CorridorGraph],
    pub routes: &'a [RouteFamilyRouteInput<'a>],
    pub fixed_context: &'a [RouteFamilyFixedContextInput],
}

#[derive(Clone, Debug)]
pub struct RouteFamilyFixedContextInput {
    pub context_id: String,
    pub branch: String,
    pub electrical_net: String,
    pub layer: String,
    pub physical_width: f64,
    pub clearance: f64,
    pub points: Vec<Vec2>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RouteFamilyProductionWork {
    /// Full bounded portfolio builds, including repeated K3/K8/K16 stages.
    pub portfolio_builds: u64,
    /// Exact family-assignment searches run against produced portfolios.
    pub assignment_searches: u64,
    /// Logical bounded enumeration requests. Existing reports retain this
    /// pre-cache meaning so structural work evidence remains comparable.
    pub family_searches: u64,
    pub family_search_cache_lookups: u64,
    pub family_search_cache_hits: u64,
    /// Reserved for a future exact bounded-search continuation. Exact-bound
    /// replay introduced here never increments it.
    pub family_search_resumes: u64,
    pub family_search_new_executions: u64,
    /// Explored-state evidence replayed from exact cache entries.
    pub visibility_states_reused: u64,
    /// Explored states consumed by searches actually executed in this call.
    pub visibility_states_new: u64,
    pub visibility_nodes: u64,
    /// Lazy visibility node pairs whose full geometry and cut signatures were
    /// considered, including both accepted and rejected pairs.
    pub visibility_edge_candidates: u64,
    /// Accepted edges among `visibility_edge_candidates`.
    pub visibility_edges: u64,
    pub visibility_edges_rejected: u64,
    pub visibility_geometry_units: u64,
    pub visibility_states: u64,
    pub alternatives_reembedded: u64,
    pub families_certified: u64,
    /// Producer-side classifications used to construct the sparse conflict
    /// certificate. Independent boundary-validation classifications are not
    /// included in this work counter.
    pub family_pairs_analyzed: u64,
    pub segment_pairs_analyzed: u64,
    pub family_context_pairs_analyzed: u64,
    pub context_segment_pairs_analyzed: u64,
    /// Family witnesses converted to prepared copper geometry. A family can
    /// appear more than once when a later bounded stage rebuilds a portfolio.
    pub family_witness_preparations: u64,
    /// Fixed-context witnesses converted to prepared copper geometry.
    pub context_witness_preparations: u64,
    /// Same-layer family-family classifications actually executed.
    pub family_pair_classifications: u64,
    /// Same-layer family-context classifications actually executed.
    pub family_context_classifications: u64,
    pub raw_candidates_requested: u64,
    pub raw_candidates_examined: u64,
    pub raw_candidates_rejected: u64,
    pub certifiable_frontiers_completed: u64,
    /// Exact prior variables copied without another corridor search.
    pub family_variables_reused: u64,
    /// Exact prior family objects retained. Families on selected variables are
    /// counted here only after their stable identities are re-generated.
    pub families_reused: u64,
    /// Complete old-old family-pair outcomes retained from a validated parent
    /// certificate without geometric reclassification.
    pub family_pair_outcomes_reused: u64,
    /// Complete old-family x retained-context outcomes retained from a
    /// validated parent certificate without geometric reclassification.
    pub family_context_pair_outcomes_reused: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incremental_reuse_contract: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incremental_parent_candidate_set_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incremental_parent_fixed_context_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incremental_result_candidate_set_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incremental_retained_branches: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incremental_promoted_branches: Vec<String>,
    /// Prior families observed again in the deeper frontier with the same
    /// relative order, stable identity, cut word, and geometry epoch before
    /// the exact prior objects were retained.
    pub prior_family_identities_verified: u64,
    pub retained_prior_family_ids: Vec<String>,
    pub observed_new_family_ids: Vec<String>,
    pub rejection_evidence: Vec<RouteFamilyCandidateRejection>,
    pub raw_frontiers: Vec<RouteFamilyRawFrontierEvidence>,
    pub planning_stages: Vec<RouteFamilyPlanningStageEvidence>,
    /// Inclusive wall time of portfolio builds. The narrower timings below
    /// are non-overlapping subsets except for uninstrumented build overhead.
    pub portfolio_build_elapsed_micros: u64,
    pub alternative_search_elapsed_micros: u64,
    pub alternative_reembedding_elapsed_micros: u64,
    pub copper_preparation_elapsed_micros: u64,
    pub family_pair_classification_elapsed_micros: u64,
    pub family_context_classification_elapsed_micros: u64,
    pub portfolio_finalize_elapsed_micros: u64,
    pub assignment_elapsed_micros: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteFamilyPlanningStageKind {
    InitialK3,
    ExpandedRaw8,
    TargetedRaw16,
    IncrementalPromotion,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteFamilyPlanningStageOutcome {
    Selected,
    BoundedInfeasible,
    AssignmentBudgetExhausted,
    NoAdmittedDomainGrowth,
    GenerationBudgetExhausted,
    NoFamily,
    CertifiableFrontierIncomplete,
    PortfolioRejected,
    NoEligibleImplicatedFrontier,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteFamilyTargetedDeepeningExhaustion {
    NoEligibleImplicatedFrontier,
    TargetVariableLimitReached,
    NewFamilyWorkLimitReached,
    FamilyPairWorkLimitReached,
    FamilyContextWorkLimitReached,
    GenerationStateLimitReached,
    VisibilityGraphLimitReached,
    AssignmentNodeLimitReached,
    CertifiableFrontierIncomplete,
    NoAdmittedDomainGrowth,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouteFamilyTargetedDeepeningEvidence {
    pub implicated_variables: Vec<String>,
    pub eligible_variables: Vec<String>,
    pub selected_variables: Vec<String>,
    pub deferred_variables: Vec<String>,
    pub prior_raw_candidate_limit: usize,
    pub target_raw_candidate_limit: usize,
    pub target_variable_limit: usize,
    pub new_family_limit: usize,
    pub family_pair_limit: u64,
    pub family_context_pair_limit: u64,
    #[serde(default)]
    pub projected_new_families: usize,
    pub projected_family_pairs: u64,
    pub projected_family_context_pairs: u64,
    pub reused_variables: u64,
    pub reused_families: u64,
    pub prior_family_identities_verified: u64,
    pub newly_certified_families: u64,
    pub retained_prior_family_ids: Vec<String>,
    pub observed_new_family_ids: Vec<String>,
    pub exhaustions: Vec<RouteFamilyTargetedDeepeningExhaustion>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouteFamilyAdmittedDomainEvidence {
    pub branch: String,
    pub admitted_family_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouteFamilyRawFrontierEvidence {
    pub branch: String,
    pub requested_raw_bound: usize,
    pub returned_raw_candidates: usize,
    pub termination: AlternativeSearchTermination,
    pub admitted_family_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouteFamilyPlanningStageEvidence {
    pub stage: RouteFamilyPlanningStageKind,
    pub minimum_certified_per_variable: usize,
    pub maximum_certified_per_variable: usize,
    pub raw_candidate_limit: usize,
    pub outcome: RouteFamilyPlanningStageOutcome,
    pub admitted_domains: Vec<RouteFamilyAdmittedDomainEvidence>,
    pub raw_frontiers: Vec<RouteFamilyRawFrontierEvidence>,
    pub assignment_work: FamilyAssignmentWork,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targeted_deepening: Option<RouteFamilyTargetedDeepeningEvidence>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouteFamilyCandidateRejection {
    pub branch: String,
    pub raw_ordinal: usize,
    pub family_id: String,
    pub reason: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteFamilyUnsupportedReason {
    EmptyPortfolioId,
    EmptyRouteSet,
    EmptyIdentity,
    InvalidRouteGeometry,
    InvalidWidthOrClearance,
    DuplicateBranch,
    CorridorLayerMissing,
    DuplicateCorridorLayer,
    CorridorEpochMismatch,
    IncompleteCutBasis,
    AlternativeCutWordDuplicated,
    AlternativeCouldNotBeCertified,
    AlternativePassageProjectionInvalid,
    UncertifiableExclusivePassageCapacity,
    UnmodeledCopperClearanceConflict,
    IncrementalPromotionCertificateMismatch,
}

#[derive(Clone, Debug)]
pub enum RouteFamilyPortfolioBuildOutcome {
    Complete {
        portfolio: RouteFamilyPortfolio,
        work: RouteFamilyProductionWork,
    },
    Unsupported {
        route: Option<String>,
        reason: RouteFamilyUnsupportedReason,
        detail: String,
        work: RouteFamilyProductionWork,
    },
    GenerationBudgetExhausted {
        route: String,
        termination: AlternativeSearchTermination,
        work: RouteFamilyProductionWork,
    },
    NoFamily {
        route: String,
        termination: AlternativeSearchTermination,
        work: RouteFamilyProductionWork,
    },
    CertifiableFrontierIncomplete {
        route: String,
        accepted: usize,
        requested_raw_bound: usize,
        termination: AlternativeSearchTermination,
        work: RouteFamilyProductionWork,
    },
    InvalidProducedPortfolio {
        issues: Vec<ValidationIssue>,
        work: RouteFamilyProductionWork,
    },
}

#[derive(Clone, Debug)]
pub enum RouteFamilyPlanningOutcome {
    Assignment {
        portfolio: RouteFamilyPortfolio,
        production_work: RouteFamilyProductionWork,
        assignment: FamilyAssignmentOutcome,
    },
    PortfolioUnavailable {
        outcome: RouteFamilyPortfolioBuildOutcome,
    },
    AssignmentRejected {
        error: String,
        production_work: RouteFamilyProductionWork,
    },
}

#[derive(Clone)]
struct CertifiedFamily {
    branch: String,
    electrical_net: String,
    required_clearance: f64,
    family: RouteFamily,
}

struct TargetedReusePlan<'a> {
    prior_portfolio: &'a RouteFamilyPortfolio,
    targeted_branches: &'a BTreeSet<String>,
    prior_raw_candidate_limit: usize,
}

struct ReconciledFamilyFrontier {
    families: Vec<RouteFamily>,
    retained_prior_family_ids: Vec<String>,
    observed_new_family_ids: Vec<String>,
}

fn reuse_exact_prior_family(
    prior: Option<&RouteFamily>,
    regenerated: &RouteFamily,
) -> Result<RouteFamily, &'static str> {
    let Some(prior) = prior else {
        return Err(
            "targeted search observed a prior family identity absent from the exact prior portfolio",
        );
    };
    let same_topological_certificate = prior.family_id == regenerated.family_id
        && prior.placement_revision == regenerated.placement_revision
        && prior.complete_route == regenerated.complete_route
        && prior.runs.len() == regenerated.runs.len()
        && prior
            .runs
            .iter()
            .zip(&regenerated.runs)
            .all(|(left, right)| {
                left.run_index == right.run_index
                    && left.layer == right.layer
                    && left.corridor_revision == right.corridor_revision
                    && left.transition_from_previous == right.transition_from_previous
                    && left.cut_word_certificate == right.cut_word_certificate
                    && left.witness.placement_revision == right.witness.placement_revision
                    && left.witness.required_width.to_bits()
                        == right.witness.required_width.to_bits()
            });
    if !same_topological_certificate {
        return Err(
            "targeted search did not reproduce the prior stable family identity, cut word, and geometry epoch",
        );
    }
    // K16 may choose a different full-width representative polyline for the
    // same certified route class. The already validated K8 object remains the
    // authoritative exact witness; never silently replace it during reuse.
    Ok(prior.clone())
}

fn reconcile_prior_family_frontier(
    prior: &[RouteFamily],
    regenerated: &[RouteFamily],
) -> Result<ReconciledFamilyFrontier, String> {
    let prior_ids = prior
        .iter()
        .map(|family| family.family_id.as_str())
        .collect::<BTreeSet<_>>();
    if prior_ids.len() != prior.len() {
        return Err("prior family frontier contains a duplicate stable identity".into());
    }
    let observed_prior = regenerated
        .iter()
        .filter(|family| prior_ids.contains(family.family_id.as_str()))
        .collect::<Vec<_>>();
    if observed_prior.len() != prior.len() {
        return Err(format!(
            "targeted frontier contains {} prior identities, expected the complete ordered set of {}",
            observed_prior.len(),
            prior.len()
        ));
    }
    let mut families = Vec::with_capacity(regenerated.len());
    let mut retained_prior_family_ids = Vec::with_capacity(prior.len());
    for (prior_family, regenerated_family) in prior.iter().zip(observed_prior) {
        let retained = reuse_exact_prior_family(Some(prior_family), regenerated_family)
            .map_err(str::to_owned)?;
        retained_prior_family_ids.push(retained.family_id.clone());
        families.push(retained);
    }
    let mut observed_new_family_ids = Vec::new();
    for family in regenerated {
        if prior_ids.contains(family.family_id.as_str()) {
            continue;
        }
        observed_new_family_ids.push(family.family_id.clone());
        families.push(family.clone());
    }
    Ok(ReconciledFamilyFrontier {
        families,
        retained_prior_family_ids,
        observed_new_family_ids,
    })
}

fn assignment_work(outcome: &FamilyAssignmentOutcome) -> &FamilyAssignmentWork {
    match outcome {
        FamilyAssignmentOutcome::Selected { work, .. }
        | FamilyAssignmentOutcome::BoundedInfeasible { work, .. }
        | FamilyAssignmentOutcome::BudgetExhausted { work, .. } => work,
    }
}

fn assignment_stage_outcome(outcome: &FamilyAssignmentOutcome) -> RouteFamilyPlanningStageOutcome {
    match outcome {
        FamilyAssignmentOutcome::Selected { .. } => RouteFamilyPlanningStageOutcome::Selected,
        FamilyAssignmentOutcome::BoundedInfeasible { .. } => {
            RouteFamilyPlanningStageOutcome::BoundedInfeasible
        }
        FamilyAssignmentOutcome::BudgetExhausted { .. } => {
            RouteFamilyPlanningStageOutcome::AssignmentBudgetExhausted
        }
    }
}

fn admitted_domains(portfolio: &RouteFamilyPortfolio) -> Vec<RouteFamilyAdmittedDomainEvidence> {
    let mut domains = portfolio
        .variables
        .iter()
        .map(|variable| RouteFamilyAdmittedDomainEvidence {
            branch: variable.branch.clone(),
            admitted_family_count: variable.families.len(),
        })
        .collect::<Vec<_>>();
    domains.sort_by(|left, right| left.branch.cmp(&right.branch));
    domains
}

fn push_stage(
    work: &mut RouteFamilyProductionWork,
    stage: RouteFamilyPlanningStageKind,
    bounds: PortfolioBounds,
    outcome: RouteFamilyPlanningStageOutcome,
    portfolio: Option<&RouteFamilyPortfolio>,
    raw_frontiers: &[RouteFamilyRawFrontierEvidence],
    assignment_work: FamilyAssignmentWork,
) {
    work.planning_stages.push(RouteFamilyPlanningStageEvidence {
        stage,
        minimum_certified_per_variable: bounds.minimum_certified_per_variable,
        maximum_certified_per_variable: bounds.maximum_certified_per_variable,
        raw_candidate_limit: bounds.raw_candidate_limit,
        outcome,
        admitted_domains: portfolio.map(admitted_domains).unwrap_or_default(),
        raw_frontiers: raw_frontiers.to_vec(),
        assignment_work,
        targeted_deepening: None,
    });
}

fn push_targeted_stage(
    work: &mut RouteFamilyProductionWork,
    outcome: RouteFamilyPlanningStageOutcome,
    portfolio: Option<&RouteFamilyPortfolio>,
    raw_frontiers: &[RouteFamilyRawFrontierEvidence],
    assignment_work: FamilyAssignmentWork,
    evidence: RouteFamilyTargetedDeepeningEvidence,
) {
    push_stage(
        work,
        RouteFamilyPlanningStageKind::TargetedRaw16,
        TARGETED_BOUNDS,
        outcome,
        portfolio,
        raw_frontiers,
        assignment_work,
    );
    work.planning_stages
        .last_mut()
        .expect("targeted stage was just appended")
        .targeted_deepening = Some(evidence);
}

fn targeted_deepening_evidence(
    assignment: &FamilyAssignmentOutcome,
    input: &RouteFamilyPortfolioInput<'_>,
    expanded_portfolio: &RouteFamilyPortfolio,
    expanded_frontiers: &[RouteFamilyRawFrontierEvidence],
) -> Option<RouteFamilyTargetedDeepeningEvidence> {
    let FamilyAssignmentOutcome::BoundedInfeasible { evidence, .. } = assignment else {
        return None;
    };
    let frontier_by_branch = expanded_frontiers
        .iter()
        .map(|frontier| (frontier.branch.as_str(), frontier))
        .collect::<BTreeMap<_, _>>();
    let domain_by_branch = expanded_portfolio
        .variables
        .iter()
        .map(|variable| (variable.branch.as_str(), variable.families.len()))
        .collect::<BTreeMap<_, _>>();
    let mut implicated_variables = evidence.implicated_variables.clone();
    implicated_variables.sort();
    implicated_variables.dedup();
    let eligible_variables = implicated_variables
        .iter()
        .filter(|branch| {
            frontier_by_branch
                .get(branch.as_str())
                .is_some_and(|frontier| {
                    frontier.requested_raw_bound == RAW_FAMILY_CANDIDATE_LIMIT
                        && frontier.returned_raw_candidates == RAW_FAMILY_CANDIDATE_LIMIT
                        && frontier.termination == AlternativeSearchTermination::FamilyLimitReached
                        && frontier.admitted_family_count == RAW_FAMILY_CANDIDATE_LIMIT
                })
                && domain_by_branch.get(branch.as_str()) == Some(&RAW_FAMILY_CANDIDATE_LIMIT)
        })
        .cloned()
        .collect::<Vec<_>>();
    let admission = admit_targeted_variables(
        input,
        expanded_portfolio,
        &eligible_variables,
        TARGETED_ADMISSION_LIMITS,
    );
    let mut exhaustions = Vec::new();
    if eligible_variables.is_empty() {
        exhaustions.push(RouteFamilyTargetedDeepeningExhaustion::NoEligibleImplicatedFrontier);
    }
    exhaustions.extend(admission.exhaustions);
    Some(RouteFamilyTargetedDeepeningEvidence {
        implicated_variables,
        eligible_variables,
        selected_variables: admission.selected_variables,
        deferred_variables: admission.deferred_variables,
        prior_raw_candidate_limit: RAW_FAMILY_CANDIDATE_LIMIT,
        target_raw_candidate_limit: TARGETED_RAW_FAMILY_CANDIDATE_LIMIT,
        target_variable_limit: TARGETED_ADMISSION_LIMITS.variable_safety,
        new_family_limit: MAX_TARGETED_NEW_FAMILIES,
        family_pair_limit: MAX_TARGETED_FAMILY_PAIR_ANALYSES,
        family_context_pair_limit: MAX_TARGETED_FAMILY_CONTEXT_PAIR_ANALYSES,
        projected_new_families: admission.projected_new_families,
        projected_family_pairs: admission.projected_family_pairs,
        projected_family_context_pairs: admission.projected_family_context_pairs,
        reused_variables: 0,
        reused_families: 0,
        prior_family_identities_verified: 0,
        newly_certified_families: 0,
        retained_prior_family_ids: Vec::new(),
        observed_new_family_ids: Vec::new(),
        exhaustions,
    })
}

struct TargetedAdmission {
    selected_variables: Vec<String>,
    deferred_variables: Vec<String>,
    projected_new_families: usize,
    projected_family_pairs: u64,
    projected_family_context_pairs: u64,
    exhaustions: Vec<RouteFamilyTargetedDeepeningExhaustion>,
}

fn targeted_new_family_upper_bound(
    portfolio: &RouteFamilyPortfolio,
    targeted_branches: &BTreeSet<String>,
) -> usize {
    portfolio
        .variables
        .iter()
        .filter(|variable| targeted_branches.contains(&variable.branch))
        .map(|variable| TARGETED_RAW_FAMILY_CANDIDATE_LIMIT.saturating_sub(variable.families.len()))
        .sum()
}

fn push_unique_exhaustion(
    exhaustions: &mut Vec<RouteFamilyTargetedDeepeningExhaustion>,
    exhaustion: RouteFamilyTargetedDeepeningExhaustion,
) {
    if !exhaustions.contains(&exhaustion) {
        exhaustions.push(exhaustion);
    }
}

fn admit_targeted_variables(
    input: &RouteFamilyPortfolioInput<'_>,
    portfolio: &RouteFamilyPortfolio,
    eligible_variables: &[String],
    limits: TargetedAdmissionLimits,
) -> TargetedAdmission {
    let mut selected_variables = Vec::new();
    let mut selected = BTreeSet::new();
    let mut deferred_variables = Vec::new();
    let mut exhaustions = Vec::new();

    for branch in eligible_variables {
        let mut candidate = selected.clone();
        candidate.insert(branch.clone());
        let projected_new_families = targeted_new_family_upper_bound(portfolio, &candidate);
        let (projected_family_pairs, projected_family_context_pairs) =
            targeted_pair_work_upper_bounds(input, portfolio, &candidate);
        let exhaustion = if candidate.len() > limits.variable_safety {
            Some(RouteFamilyTargetedDeepeningExhaustion::TargetVariableLimitReached)
        } else if projected_new_families > limits.new_families {
            Some(RouteFamilyTargetedDeepeningExhaustion::NewFamilyWorkLimitReached)
        } else if projected_family_pairs > limits.family_pairs {
            Some(RouteFamilyTargetedDeepeningExhaustion::FamilyPairWorkLimitReached)
        } else if projected_family_context_pairs > limits.family_context_pairs {
            Some(RouteFamilyTargetedDeepeningExhaustion::FamilyContextWorkLimitReached)
        } else {
            None
        };
        if let Some(exhaustion) = exhaustion {
            deferred_variables.push(branch.clone());
            push_unique_exhaustion(&mut exhaustions, exhaustion);
        } else {
            selected = candidate;
            selected_variables.push(branch.clone());
        }
    }

    let projected_new_families = targeted_new_family_upper_bound(portfolio, &selected);
    let (projected_family_pairs, projected_family_context_pairs) =
        targeted_pair_work_upper_bounds(input, portfolio, &selected);
    TargetedAdmission {
        selected_variables,
        deferred_variables,
        projected_new_families,
        projected_family_pairs,
        projected_family_context_pairs,
        exhaustions,
    }
}

fn record_targeted_work_success(
    evidence: &mut RouteFamilyTargetedDeepeningEvidence,
    work: &RouteFamilyProductionWork,
) {
    evidence.reused_variables = work.family_variables_reused;
    evidence.reused_families = work.families_reused;
    evidence.prior_family_identities_verified = work.prior_family_identities_verified;
    evidence.newly_certified_families = work.families_certified;
    evidence.retained_prior_family_ids = work.retained_prior_family_ids.clone();
    evidence.observed_new_family_ids = work.observed_new_family_ids.clone();
}

fn record_targeted_work(
    evidence: &mut RouteFamilyTargetedDeepeningEvidence,
    work: &RouteFamilyProductionWork,
    outcome: &RouteFamilyPortfolioBuildOutcome,
) {
    record_targeted_work_success(evidence, work);
    match outcome {
        RouteFamilyPortfolioBuildOutcome::GenerationBudgetExhausted {
            termination: AlternativeSearchTermination::StateLimitReached,
            ..
        } => evidence
            .exhaustions
            .push(RouteFamilyTargetedDeepeningExhaustion::GenerationStateLimitReached),
        RouteFamilyPortfolioBuildOutcome::GenerationBudgetExhausted {
            termination: AlternativeSearchTermination::VisibilityGraphLimitReached,
            ..
        } => evidence
            .exhaustions
            .push(RouteFamilyTargetedDeepeningExhaustion::VisibilityGraphLimitReached),
        RouteFamilyPortfolioBuildOutcome::CertifiableFrontierIncomplete { .. } => evidence
            .exhaustions
            .push(RouteFamilyTargetedDeepeningExhaustion::CertifiableFrontierIncomplete),
        _ => {}
    }
    if work.families_certified > MAX_TARGETED_NEW_FAMILIES as u64 {
        evidence
            .exhaustions
            .push(RouteFamilyTargetedDeepeningExhaustion::NewFamilyWorkLimitReached);
    }
}

fn canonical_parent_raw_frontiers(
    prior: &RouteFamilyPortfolio,
    evidence: &[RouteFamilyRawFrontierEvidence],
) -> Result<Vec<RouteFamilyRawFrontierEvidence>, String> {
    let variables = prior
        .variables
        .iter()
        .map(|variable| (variable.branch.as_str(), variable.families.len()))
        .collect::<BTreeMap<_, _>>();
    let mut matching = BTreeMap::<&str, Vec<&RouteFamilyRawFrontierEvidence>>::new();
    for frontier in evidence {
        let Some(&domain_size) = variables.get(frontier.branch.as_str()) else {
            return Err(format!(
                "parent raw-frontier evidence names unknown branch {}",
                frontier.branch
            ));
        };
        if !matches!(
            frontier.requested_raw_bound,
            RAW_FAMILY_CANDIDATE_LIMIT | TARGETED_RAW_FAMILY_CANDIDATE_LIMIT
        ) || frontier.returned_raw_candidates > frontier.requested_raw_bound
            || frontier.admitted_family_count > frontier.returned_raw_candidates
            || (frontier.termination == AlternativeSearchTermination::FamilyLimitReached
                && frontier.returned_raw_candidates != frontier.requested_raw_bound)
        {
            return Err(format!(
                "parent raw-frontier evidence for {} violates the bounded-search contract",
                frontier.branch
            ));
        }
        if frontier.admitted_family_count == domain_size {
            matching
                .entry(frontier.branch.as_str())
                .or_default()
                .push(frontier);
        }
    }

    let mut canonical = Vec::with_capacity(variables.len());
    for (branch, _) in variables {
        let candidates = matching.get(branch).ok_or_else(|| {
            format!("parent raw-frontier evidence has no record bound to retained domain {branch}")
        })?;
        let highest_bound = candidates
            .iter()
            .map(|frontier| frontier.requested_raw_bound)
            .max()
            .expect("matching frontier set is nonempty");
        let mut authoritative = candidates
            .iter()
            .filter(|frontier| frontier.requested_raw_bound == highest_bound)
            .copied();
        let selected = authoritative.next().expect("highest-bound frontier exists");
        if authoritative.any(|other| other != selected) {
            return Err(format!(
                "parent raw-frontier evidence for {branch} has ambiguous authoritative records"
            ));
        }
        canonical.push(selected.clone());
    }
    canonical.sort_by(|left, right| left.branch.cmp(&right.branch));
    Ok(canonical)
}

fn targeted_pair_work_upper_bounds(
    input: &RouteFamilyPortfolioInput<'_>,
    expanded_portfolio: &RouteFamilyPortfolio,
    targeted_branches: &BTreeSet<String>,
) -> (u64, u64) {
    let variables = expanded_portfolio
        .variables
        .iter()
        .map(|variable| {
            let count = if targeted_branches.contains(&variable.branch) {
                TARGETED_RAW_FAMILY_CANDIDATE_LIMIT
            } else {
                variable.families.len()
            } as u64;
            (variable.electrical_net.as_str(), count)
        })
        .collect::<Vec<_>>();
    let mut family_pairs = 0u64;
    for left in 0..variables.len() {
        for right in left + 1..variables.len() {
            if variables[left].0 != variables[right].0 {
                family_pairs = family_pairs
                    .saturating_add(variables[left].1.saturating_mul(variables[right].1));
            }
        }
    }
    let mut context_pairs = 0u64;
    for (electrical_net, family_count) in variables {
        let cross_net_context = input
            .fixed_context
            .iter()
            .filter(|context| context.electrical_net != electrical_net)
            .count() as u64;
        context_pairs =
            context_pairs.saturating_add(family_count.saturating_mul(cross_net_context));
    }
    (family_pairs, context_pairs)
}

fn merge_production_work(
    total: &mut RouteFamilyProductionWork,
    addition: &mut RouteFamilyProductionWork,
) {
    total.portfolio_builds += addition.portfolio_builds;
    total.assignment_searches += addition.assignment_searches;
    total.family_searches += addition.family_searches;
    total.family_search_cache_lookups += addition.family_search_cache_lookups;
    total.family_search_cache_hits += addition.family_search_cache_hits;
    total.family_search_resumes += addition.family_search_resumes;
    total.family_search_new_executions += addition.family_search_new_executions;
    total.visibility_states_reused += addition.visibility_states_reused;
    total.visibility_states_new += addition.visibility_states_new;
    total.visibility_nodes += addition.visibility_nodes;
    total.visibility_edge_candidates += addition.visibility_edge_candidates;
    total.visibility_edges += addition.visibility_edges;
    total.visibility_edges_rejected += addition.visibility_edges_rejected;
    total.visibility_geometry_units += addition.visibility_geometry_units;
    total.visibility_states += addition.visibility_states;
    total.alternatives_reembedded += addition.alternatives_reembedded;
    total.families_certified += addition.families_certified;
    total.family_pairs_analyzed += addition.family_pairs_analyzed;
    total.segment_pairs_analyzed += addition.segment_pairs_analyzed;
    total.family_context_pairs_analyzed += addition.family_context_pairs_analyzed;
    total.context_segment_pairs_analyzed += addition.context_segment_pairs_analyzed;
    total.family_witness_preparations += addition.family_witness_preparations;
    total.context_witness_preparations += addition.context_witness_preparations;
    total.family_pair_classifications += addition.family_pair_classifications;
    total.family_context_classifications += addition.family_context_classifications;
    total.raw_candidates_requested += addition.raw_candidates_requested;
    total.raw_candidates_examined += addition.raw_candidates_examined;
    total.raw_candidates_rejected += addition.raw_candidates_rejected;
    total.certifiable_frontiers_completed += addition.certifiable_frontiers_completed;
    total.family_variables_reused += addition.family_variables_reused;
    total.families_reused += addition.families_reused;
    total.family_pair_outcomes_reused += addition.family_pair_outcomes_reused;
    total.family_context_pair_outcomes_reused += addition.family_context_pair_outcomes_reused;
    if addition.incremental_reuse_contract.is_some() {
        total.incremental_reuse_contract = addition.incremental_reuse_contract.take();
        total.incremental_parent_candidate_set_fingerprint =
            addition.incremental_parent_candidate_set_fingerprint.take();
        total.incremental_parent_fixed_context_fingerprint =
            addition.incremental_parent_fixed_context_fingerprint.take();
        total.incremental_result_candidate_set_fingerprint =
            addition.incremental_result_candidate_set_fingerprint.take();
        total
            .incremental_retained_branches
            .append(&mut addition.incremental_retained_branches);
        total
            .incremental_promoted_branches
            .append(&mut addition.incremental_promoted_branches);
    }
    total.prior_family_identities_verified += addition.prior_family_identities_verified;
    total
        .retained_prior_family_ids
        .append(&mut addition.retained_prior_family_ids);
    total
        .observed_new_family_ids
        .append(&mut addition.observed_new_family_ids);
    total
        .rejection_evidence
        .append(&mut addition.rejection_evidence);
    total.raw_frontiers.append(&mut addition.raw_frontiers);
    total.portfolio_build_elapsed_micros += addition.portfolio_build_elapsed_micros;
    total.alternative_search_elapsed_micros += addition.alternative_search_elapsed_micros;
    total.alternative_reembedding_elapsed_micros += addition.alternative_reembedding_elapsed_micros;
    total.copper_preparation_elapsed_micros += addition.copper_preparation_elapsed_micros;
    total.family_pair_classification_elapsed_micros +=
        addition.family_pair_classification_elapsed_micros;
    total.family_context_classification_elapsed_micros +=
        addition.family_context_classification_elapsed_micros;
    total.portfolio_finalize_elapsed_micros += addition.portfolio_finalize_elapsed_micros;
    total.assignment_elapsed_micros += addition.assignment_elapsed_micros;
}

fn select_route_families_timed(
    portfolio: &RouteFamilyPortfolio,
    limits: FamilyAssignmentLimits,
    work: &mut RouteFamilyProductionWork,
) -> Result<FamilyAssignmentOutcome, FamilyAssignmentError> {
    let started = Instant::now();
    let outcome = select_route_families(portfolio, limits);
    work.assignment_searches += 1;
    work.assignment_elapsed_micros += started.elapsed().as_micros() as u64;
    outcome
}

fn build_outcome_work_mut(
    outcome: &mut RouteFamilyPortfolioBuildOutcome,
) -> &mut RouteFamilyProductionWork {
    match outcome {
        RouteFamilyPortfolioBuildOutcome::Complete { work, .. }
        | RouteFamilyPortfolioBuildOutcome::Unsupported { work, .. }
        | RouteFamilyPortfolioBuildOutcome::GenerationBudgetExhausted { work, .. }
        | RouteFamilyPortfolioBuildOutcome::NoFamily { work, .. }
        | RouteFamilyPortfolioBuildOutcome::CertifiableFrontierIncomplete { work, .. }
        | RouteFamilyPortfolioBuildOutcome::InvalidProducedPortfolio { work, .. } => work,
    }
}

fn unavailable_stage_outcome(
    outcome: &RouteFamilyPortfolioBuildOutcome,
) -> RouteFamilyPlanningStageOutcome {
    match outcome {
        RouteFamilyPortfolioBuildOutcome::GenerationBudgetExhausted { .. } => {
            RouteFamilyPlanningStageOutcome::GenerationBudgetExhausted
        }
        RouteFamilyPortfolioBuildOutcome::NoFamily { .. } => {
            RouteFamilyPlanningStageOutcome::NoFamily
        }
        RouteFamilyPortfolioBuildOutcome::CertifiableFrontierIncomplete { .. } => {
            RouteFamilyPlanningStageOutcome::CertifiableFrontierIncomplete
        }
        RouteFamilyPortfolioBuildOutcome::Unsupported { .. }
        | RouteFamilyPortfolioBuildOutcome::InvalidProducedPortfolio { .. }
        | RouteFamilyPortfolioBuildOutcome::Complete { .. } => {
            RouteFamilyPlanningStageOutcome::PortfolioRejected
        }
    }
}

fn unavailable_with_stage(
    mut outcome: RouteFamilyPortfolioBuildOutcome,
    stage: RouteFamilyPlanningStageKind,
    bounds: PortfolioBounds,
) -> RouteFamilyPlanningOutcome {
    let stage_outcome = unavailable_stage_outcome(&outcome);
    let work = build_outcome_work_mut(&mut outcome);
    let raw_frontiers = work.raw_frontiers.clone();
    push_stage(
        work,
        stage,
        bounds,
        stage_outcome,
        None,
        &raw_frontiers,
        FamilyAssignmentWork::default(),
    );
    RouteFamilyPlanningOutcome::PortfolioUnavailable { outcome }
}

pub fn build_and_select_route_families(
    input: RouteFamilyPortfolioInput<'_>,
    limits: FamilyAssignmentLimits,
) -> RouteFamilyPlanningOutcome {
    build_and_select_route_families_impl(input, limits, None)
}

/// Build and select a route-family portfolio with caller-controlled exact
/// visibility-search ceilings. This preserves every ordinary portfolio and
/// validation contract; only the bounded producer work limits change.
pub fn build_and_select_route_families_with_work_budget(
    input: RouteFamilyPortfolioInput<'_>,
    limits: FamilyAssignmentLimits,
    work_budget: RouteRepairWorkBudget,
) -> RouteFamilyPlanningOutcome {
    build_and_select_route_families_impl(input, limits, Some(work_budget))
}

fn build_and_select_route_families_impl(
    input: RouteFamilyPortfolioInput<'_>,
    limits: FamilyAssignmentLimits,
    work_budget: Option<RouteRepairWorkBudget>,
) -> RouteFamilyPlanningOutcome {
    let mut search_cache = work_budget.map_or_else(
        || AlternativeSearchCache::new(input.corridors),
        |budget| AlternativeSearchCache::new_with_work_budget(input.corridors, budget),
    );
    let (initial_portfolio, mut work) = match build_route_family_portfolio_with_plan(
        input.clone(),
        INITIAL_BOUNDS,
        None,
        &mut search_cache,
    ) {
        RouteFamilyPortfolioBuildOutcome::Complete { portfolio, work } => (portfolio, work),
        unavailable => {
            return unavailable_with_stage(
                unavailable,
                RouteFamilyPlanningStageKind::InitialK3,
                INITIAL_BOUNDS,
            );
        }
    };
    let initial_assignment =
        match select_route_families_timed(&initial_portfolio, limits, &mut work) {
            Ok(assignment) => assignment,
            Err(FamilyAssignmentError::InvalidPortfolio(issues)) => {
                let raw_frontiers = work.raw_frontiers.clone();
                push_stage(
                    &mut work,
                    RouteFamilyPlanningStageKind::InitialK3,
                    INITIAL_BOUNDS,
                    RouteFamilyPlanningStageOutcome::PortfolioRejected,
                    Some(&initial_portfolio),
                    &raw_frontiers,
                    FamilyAssignmentWork::default(),
                );
                return RouteFamilyPlanningOutcome::AssignmentRejected {
                    error: format!(
                        "validated producer output was rejected with {} issue(s)",
                        issues.len()
                    ),
                    production_work: work,
                };
            }
        };
    let initial_raw_frontiers = work.raw_frontiers.clone();
    push_stage(
        &mut work,
        RouteFamilyPlanningStageKind::InitialK3,
        INITIAL_BOUNDS,
        assignment_stage_outcome(&initial_assignment),
        Some(&initial_portfolio),
        &initial_raw_frontiers,
        assignment_work(&initial_assignment).clone(),
    );
    if !matches!(
        initial_assignment,
        FamilyAssignmentOutcome::BoundedInfeasible { .. }
    ) {
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio: initial_portfolio,
            production_work: work,
            assignment: initial_assignment,
        };
    }

    let (expanded_portfolio, expanded_work) = match build_route_family_portfolio_with_plan(
        input.clone(),
        EXPANDED_BOUNDS,
        None,
        &mut search_cache,
    ) {
        RouteFamilyPortfolioBuildOutcome::Complete { portfolio, work } => (portfolio, work),
        mut unavailable => {
            let outcome = unavailable_stage_outcome(&unavailable);
            let mut expanded_work = std::mem::take(build_outcome_work_mut(&mut unavailable));
            let expanded_raw_frontiers = expanded_work.raw_frontiers.clone();
            merge_production_work(&mut work, &mut expanded_work);
            push_stage(
                &mut work,
                RouteFamilyPlanningStageKind::ExpandedRaw8,
                EXPANDED_BOUNDS,
                outcome,
                None,
                &expanded_raw_frontiers,
                FamilyAssignmentWork::default(),
            );
            *build_outcome_work_mut(&mut unavailable) = work;
            return RouteFamilyPlanningOutcome::PortfolioUnavailable {
                outcome: unavailable,
            };
        }
    };
    let expanded_raw_frontiers = expanded_work.raw_frontiers.clone();
    let mut expanded_work = expanded_work;
    merge_production_work(&mut work, &mut expanded_work);
    if initial_portfolio.variables == expanded_portfolio.variables {
        work.planning_stages.push(RouteFamilyPlanningStageEvidence {
            stage: RouteFamilyPlanningStageKind::ExpandedRaw8,
            minimum_certified_per_variable: EXPANDED_BOUNDS.minimum_certified_per_variable,
            maximum_certified_per_variable: EXPANDED_BOUNDS.maximum_certified_per_variable,
            raw_candidate_limit: EXPANDED_BOUNDS.raw_candidate_limit,
            outcome: RouteFamilyPlanningStageOutcome::NoAdmittedDomainGrowth,
            admitted_domains: admitted_domains(&expanded_portfolio),
            raw_frontiers: expanded_raw_frontiers,
            assignment_work: FamilyAssignmentWork::default(),
            targeted_deepening: None,
        });
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio: initial_portfolio,
            production_work: work,
            assignment: initial_assignment,
        };
    }
    let expanded_assignment =
        match select_route_families_timed(&expanded_portfolio, limits, &mut work) {
            Ok(assignment) => assignment,
            Err(FamilyAssignmentError::InvalidPortfolio(issues)) => {
                push_stage(
                    &mut work,
                    RouteFamilyPlanningStageKind::ExpandedRaw8,
                    EXPANDED_BOUNDS,
                    RouteFamilyPlanningStageOutcome::PortfolioRejected,
                    Some(&expanded_portfolio),
                    &expanded_raw_frontiers,
                    FamilyAssignmentWork::default(),
                );
                return RouteFamilyPlanningOutcome::AssignmentRejected {
                    error: format!(
                        "validated expanded producer output was rejected with {} issue(s)",
                        issues.len()
                    ),
                    production_work: work,
                };
            }
        };
    push_stage(
        &mut work,
        RouteFamilyPlanningStageKind::ExpandedRaw8,
        EXPANDED_BOUNDS,
        assignment_stage_outcome(&expanded_assignment),
        Some(&expanded_portfolio),
        &expanded_raw_frontiers,
        assignment_work(&expanded_assignment).clone(),
    );
    if !matches!(
        expanded_assignment,
        FamilyAssignmentOutcome::BoundedInfeasible { .. }
    ) {
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio: expanded_portfolio,
            production_work: work,
            assignment: expanded_assignment,
        };
    }

    let mut targeted_evidence = targeted_deepening_evidence(
        &expanded_assignment,
        &input,
        &expanded_portfolio,
        &expanded_raw_frontiers,
    )
    .expect("bounded infeasibility always has exact implicated-variable evidence");
    if targeted_evidence.selected_variables.is_empty() {
        let outcome = if targeted_evidence.eligible_variables.is_empty() {
            RouteFamilyPlanningStageOutcome::NoEligibleImplicatedFrontier
        } else {
            RouteFamilyPlanningStageOutcome::GenerationBudgetExhausted
        };
        push_targeted_stage(
            &mut work,
            outcome,
            Some(&expanded_portfolio),
            &[],
            FamilyAssignmentWork::default(),
            targeted_evidence,
        );
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio: expanded_portfolio,
            production_work: work,
            assignment: expanded_assignment,
        };
    }

    let targeted_branches = targeted_evidence
        .selected_variables
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let (projected_family_pairs, projected_context_pairs) =
        targeted_pair_work_upper_bounds(&input, &expanded_portfolio, &targeted_branches);
    targeted_evidence.projected_family_pairs = projected_family_pairs;
    targeted_evidence.projected_family_context_pairs = projected_context_pairs;
    if projected_family_pairs > MAX_TARGETED_FAMILY_PAIR_ANALYSES
        || projected_context_pairs > MAX_TARGETED_FAMILY_CONTEXT_PAIR_ANALYSES
    {
        if projected_family_pairs > MAX_TARGETED_FAMILY_PAIR_ANALYSES {
            targeted_evidence
                .exhaustions
                .push(RouteFamilyTargetedDeepeningExhaustion::FamilyPairWorkLimitReached);
        }
        if projected_context_pairs > MAX_TARGETED_FAMILY_CONTEXT_PAIR_ANALYSES {
            targeted_evidence
                .exhaustions
                .push(RouteFamilyTargetedDeepeningExhaustion::FamilyContextWorkLimitReached);
        }
        push_targeted_stage(
            &mut work,
            RouteFamilyPlanningStageOutcome::GenerationBudgetExhausted,
            Some(&expanded_portfolio),
            &[],
            FamilyAssignmentWork::default(),
            targeted_evidence,
        );
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio: expanded_portfolio,
            production_work: work,
            assignment: expanded_assignment,
        };
    }
    let (targeted_portfolio, mut targeted_work) = match build_route_family_portfolio_with_plan(
        input,
        TARGETED_BOUNDS,
        Some(TargetedReusePlan {
            prior_portfolio: &expanded_portfolio,
            targeted_branches: &targeted_branches,
            prior_raw_candidate_limit: RAW_FAMILY_CANDIDATE_LIMIT,
        }),
        &mut search_cache,
    ) {
        RouteFamilyPortfolioBuildOutcome::Complete { portfolio, work } => (portfolio, work),
        mut unavailable => {
            let outcome = unavailable_stage_outcome(&unavailable);
            let mut stage_work = std::mem::take(build_outcome_work_mut(&mut unavailable));
            let targeted_raw_frontiers = stage_work.raw_frontiers.clone();
            targeted_evidence.reused_variables = stage_work.family_variables_reused;
            targeted_evidence.reused_families = stage_work.families_reused;
            targeted_evidence.prior_family_identities_verified =
                stage_work.prior_family_identities_verified;
            targeted_evidence.newly_certified_families = stage_work.families_certified;
            targeted_evidence.retained_prior_family_ids =
                stage_work.retained_prior_family_ids.clone();
            targeted_evidence.observed_new_family_ids = stage_work.observed_new_family_ids.clone();
            match &unavailable {
                RouteFamilyPortfolioBuildOutcome::GenerationBudgetExhausted {
                    termination: AlternativeSearchTermination::StateLimitReached,
                    ..
                } => targeted_evidence
                    .exhaustions
                    .push(RouteFamilyTargetedDeepeningExhaustion::GenerationStateLimitReached),
                RouteFamilyPortfolioBuildOutcome::GenerationBudgetExhausted {
                    termination: AlternativeSearchTermination::VisibilityGraphLimitReached,
                    ..
                } => targeted_evidence
                    .exhaustions
                    .push(RouteFamilyTargetedDeepeningExhaustion::VisibilityGraphLimitReached),
                RouteFamilyPortfolioBuildOutcome::CertifiableFrontierIncomplete { .. } => {
                    targeted_evidence.exhaustions.push(
                        RouteFamilyTargetedDeepeningExhaustion::CertifiableFrontierIncomplete,
                    );
                }
                _ => {}
            }
            if stage_work.families_certified > MAX_TARGETED_NEW_FAMILIES as u64 {
                targeted_evidence
                    .exhaustions
                    .push(RouteFamilyTargetedDeepeningExhaustion::NewFamilyWorkLimitReached);
            }
            merge_production_work(&mut work, &mut stage_work);
            push_targeted_stage(
                &mut work,
                outcome,
                None,
                &targeted_raw_frontiers,
                FamilyAssignmentWork::default(),
                targeted_evidence,
            );
            *build_outcome_work_mut(&mut unavailable) = work;
            return RouteFamilyPlanningOutcome::PortfolioUnavailable {
                outcome: unavailable,
            };
        }
    };
    let targeted_raw_frontiers = targeted_work.raw_frontiers.clone();
    targeted_evidence.reused_variables = targeted_work.family_variables_reused;
    targeted_evidence.reused_families = targeted_work.families_reused;
    targeted_evidence.prior_family_identities_verified =
        targeted_work.prior_family_identities_verified;
    targeted_evidence.newly_certified_families = targeted_work.families_certified;
    targeted_evidence.retained_prior_family_ids = targeted_work.retained_prior_family_ids.clone();
    targeted_evidence.observed_new_family_ids = targeted_work.observed_new_family_ids.clone();
    if targeted_work.families_certified > MAX_TARGETED_NEW_FAMILIES as u64 {
        targeted_evidence
            .exhaustions
            .push(RouteFamilyTargetedDeepeningExhaustion::NewFamilyWorkLimitReached);
        merge_production_work(&mut work, &mut targeted_work);
        push_targeted_stage(
            &mut work,
            RouteFamilyPlanningStageOutcome::GenerationBudgetExhausted,
            None,
            &targeted_raw_frontiers,
            FamilyAssignmentWork::default(),
            targeted_evidence,
        );
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio: expanded_portfolio,
            production_work: work,
            assignment: expanded_assignment,
        };
    }
    merge_production_work(&mut work, &mut targeted_work);
    if expanded_portfolio.variables == targeted_portfolio.variables {
        targeted_evidence
            .exhaustions
            .push(RouteFamilyTargetedDeepeningExhaustion::NoAdmittedDomainGrowth);
        push_targeted_stage(
            &mut work,
            RouteFamilyPlanningStageOutcome::NoAdmittedDomainGrowth,
            Some(&targeted_portfolio),
            &targeted_raw_frontiers,
            FamilyAssignmentWork::default(),
            targeted_evidence,
        );
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio: expanded_portfolio,
            production_work: work,
            assignment: expanded_assignment,
        };
    }
    let targeted_assignment =
        match select_route_families_timed(&targeted_portfolio, limits, &mut work) {
            Ok(assignment) => assignment,
            Err(FamilyAssignmentError::InvalidPortfolio(issues)) => {
                push_targeted_stage(
                    &mut work,
                    RouteFamilyPlanningStageOutcome::PortfolioRejected,
                    Some(&targeted_portfolio),
                    &targeted_raw_frontiers,
                    FamilyAssignmentWork::default(),
                    targeted_evidence,
                );
                return RouteFamilyPlanningOutcome::AssignmentRejected {
                    error: format!(
                        "validated targeted producer output was rejected with {} issue(s)",
                        issues.len()
                    ),
                    production_work: work,
                };
            }
        };
    if matches!(
        targeted_assignment,
        FamilyAssignmentOutcome::BudgetExhausted { .. }
    ) {
        targeted_evidence
            .exhaustions
            .push(RouteFamilyTargetedDeepeningExhaustion::AssignmentNodeLimitReached);
    }
    push_targeted_stage(
        &mut work,
        assignment_stage_outcome(&targeted_assignment),
        Some(&targeted_portfolio),
        &targeted_raw_frontiers,
        assignment_work(&targeted_assignment).clone(),
        targeted_evidence,
    );
    RouteFamilyPlanningOutcome::Assignment {
        portfolio: targeted_portfolio,
        production_work: work,
        assignment: targeted_assignment,
    }
}

pub fn build_route_family_portfolio(
    input: RouteFamilyPortfolioInput<'_>,
) -> RouteFamilyPortfolioBuildOutcome {
    build_route_family_portfolio_with_bounds(input, INITIAL_BOUNDS)
}

/// Expand a validated fixed-context portfolio by promoting new live route
/// branches. Old variables and their certified outcomes are retained exactly;
/// only promoted variables and old-new delta pairs are generated/classified.
/// The returned provisional assignment is still subject to the normal engine
/// proposal and final-admission boundaries.
pub fn build_and_select_promoted_route_families(
    input: RouteFamilyPortfolioInput<'_>,
    prior: &RouteFamilyPortfolio,
    prior_raw_frontiers: &[RouteFamilyRawFrontierEvidence],
    promoted_branches: &BTreeSet<String>,
    limits: FamilyAssignmentLimits,
) -> RouteFamilyPlanningOutcome {
    let rejected = |detail: String, work: RouteFamilyProductionWork| {
        RouteFamilyPlanningOutcome::PortfolioUnavailable {
            outcome: unsupported(
                None,
                RouteFamilyUnsupportedReason::IncrementalPromotionCertificateMismatch,
                detail,
                work,
            ),
        }
    };
    if let Err(issues) = prior.validate() {
        return RouteFamilyPlanningOutcome::PortfolioUnavailable {
            outcome: RouteFamilyPortfolioBuildOutcome::InvalidProducedPortfolio {
                issues,
                work: RouteFamilyProductionWork::default(),
            },
        };
    }
    if promoted_branches.is_empty() {
        return rejected(
            "incremental promotion requires at least one promoted branch".into(),
            RouteFamilyProductionWork::default(),
        );
    }
    if promoted_branches.len() > MAX_INCREMENTAL_PROMOTED_VARIABLES {
        return rejected(
            format!(
                "incremental promotion requested {} branches beyond the {}-branch ceiling",
                promoted_branches.len(),
                MAX_INCREMENTAL_PROMOTED_VARIABLES
            ),
            RouteFamilyProductionWork::default(),
        );
    }
    if prior.variables.len() + promoted_branches.len() > MAX_INCREMENTAL_TOTAL_VARIABLES {
        return rejected(
            format!(
                "incremental promotion would create {} variables beyond the {}-variable ceiling",
                prior.variables.len() + promoted_branches.len(),
                MAX_INCREMENTAL_TOTAL_VARIABLES
            ),
            RouteFamilyProductionWork::default(),
        );
    }
    if prior.placement_revision != input.placement_revision
        || prior.geometry_revision != Some(input.geometry_revision)
    {
        return rejected(
            "incremental promotion parent does not match the current geometry epoch".into(),
            RouteFamilyProductionWork::default(),
        );
    }

    let routes = input
        .routes
        .iter()
        .map(|route| (route.branch_id, route))
        .collect::<BTreeMap<_, _>>();
    if routes.len() != input.routes.len() {
        return rejected(
            "incremental promotion input duplicates a route branch".into(),
            RouteFamilyProductionWork::default(),
        );
    }
    let old_branches = prior
        .variables
        .iter()
        .map(|variable| variable.branch.as_str())
        .collect::<BTreeSet<_>>();
    if promoted_branches
        .iter()
        .any(|branch| old_branches.contains(branch.as_str()))
    {
        return rejected(
            "incremental promotion cannot promote an existing route-family variable".into(),
            RouteFamilyProductionWork::default(),
        );
    }
    let expected_branches = old_branches
        .iter()
        .copied()
        .chain(promoted_branches.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    if routes.keys().copied().collect::<BTreeSet<_>>() != expected_branches {
        return rejected(
            "incremental promotion input must contain exactly the old and promoted branches".into(),
            RouteFamilyProductionWork::default(),
        );
    }
    for variable in &prior.variables {
        let route = routes[variable.branch.as_str()];
        if variable.electrical_net != route.electrical_net
            || variable.required_width != route.physical_width
            || variable.required_clearance != Some(route.clearance)
        {
            return rejected(
                format!(
                    "retained variable {} does not match its current route contract",
                    variable.branch
                ),
                RouteFamilyProductionWork::default(),
            );
        }
        let retained_witness_mismatch = variable.families.iter().any(|family| {
            !family.complete_route
                || family.runs.is_empty()
                || family.runs.iter().any(|run| run.layer != route.layer)
                || family
                    .runs
                    .first()
                    .and_then(|run| run.witness.polyline.first())
                    .is_none_or(|point| point.x != route.from.x || point.y != route.from.y)
                || family
                    .runs
                    .last()
                    .and_then(|run| run.witness.polyline.last())
                    .is_none_or(|point| point.x != route.to.x || point.y != route.to.y)
        });
        if retained_witness_mismatch {
            return rejected(
                format!(
                    "retained variable {} does not match its live layer and endpoint contract",
                    variable.branch
                ),
                RouteFamilyProductionWork::default(),
            );
        }
    }
    let canonical_prior_frontiers = match canonical_parent_raw_frontiers(prior, prior_raw_frontiers)
    {
        Ok(frontiers) => frontiers,
        Err(detail) => return rejected(detail, RouteFamilyProductionWork::default()),
    };
    let current_graphs = input
        .corridors
        .iter()
        .map(|graph| (graph.layer.as_str(), graph))
        .collect::<BTreeMap<_, _>>();
    if current_graphs.len() != input.corridors.len() {
        return rejected(
            "incremental promotion input duplicates a corridor layer".into(),
            RouteFamilyProductionWork::default(),
        );
    }
    for epoch in &prior.corridor_epochs {
        let Some(graph) = current_graphs.get(epoch.layer.as_str()) else {
            return rejected(
                format!(
                    "retained layer {:?} has no current corridor graph",
                    epoch.layer
                ),
                RouteFamilyProductionWork::default(),
            );
        };
        if graph.placement_revision != input.placement_revision
            || graph.revision != epoch.corridor_revision
            || graph.semantic.fingerprint != epoch.semantic_fingerprint
            || graph.cut_basis.fingerprint != epoch.cut_basis_fingerprint
            || !graph.cut_basis.complete
        {
            return rejected(
                format!("retained layer {:?} changed corridor epoch", epoch.layer),
                RouteFamilyProductionWork::default(),
            );
        }
    }

    let Some(prior_context) = prior.fixed_context.as_ref() else {
        return rejected(
            "incremental promotion parent lacks fixed-context evidence".into(),
            RouteFamilyProductionWork::default(),
        );
    };
    let mut expected_context = input
        .fixed_context
        .iter()
        .map(|context| {
            fixed_context_run(context, input.placement_revision, input.geometry_revision)
        })
        .collect::<Vec<_>>();
    expected_context.sort_by(|left, right| left.context_id.cmp(&right.context_id));
    let mut retained_prior_context = prior_context
        .runs
        .iter()
        .filter(|run| !promoted_branches.contains(&run.branch))
        .cloned()
        .collect::<Vec<_>>();
    retained_prior_context.sort_by(|left, right| left.context_id.cmp(&right.context_id));
    if retained_prior_context != expected_context {
        return rejected(
            "current fixed context is not the exact parent context minus promoted branches".into(),
            RouteFamilyProductionWork::default(),
        );
    }
    if promoted_branches
        .iter()
        .any(|branch| !prior_context.runs.iter().any(|run| run.branch == *branch))
    {
        return rejected(
            "a promoted branch is absent from the validated parent fixed context".into(),
            RouteFamilyProductionWork::default(),
        );
    }

    let promoted_routes = input
        .routes
        .iter()
        .filter(|route| promoted_branches.contains(route.branch_id))
        .cloned()
        .collect::<Vec<_>>();
    let projected_new_families =
        promoted_routes.len() as u64 * EXPANDED_BOUNDS.maximum_certified_per_variable as u64;
    let old_family_count = prior
        .variables
        .iter()
        .map(|variable| variable.families.len() as u64)
        .sum::<u64>();
    let projected_delta_pairs = old_family_count.saturating_mul(projected_new_families);
    let projected_new_context_pairs =
        projected_new_families.saturating_mul(input.fixed_context.len() as u64);
    if projected_delta_pairs > MAX_TARGETED_FAMILY_PAIR_ANALYSES
        || projected_new_context_pairs > MAX_TARGETED_FAMILY_CONTEXT_PAIR_ANALYSES
    {
        return rejected(
            format!(
                "incremental promotion delta exceeds pair caps: projected_pairs={projected_delta_pairs}/{MAX_TARGETED_FAMILY_PAIR_ANALYSES}, projected_context_pairs={projected_new_context_pairs}/{MAX_TARGETED_FAMILY_CONTEXT_PAIR_ANALYSES}"
            ),
            RouteFamilyProductionWork::default(),
        );
    }
    let promoted_input = RouteFamilyPortfolioInput {
        portfolio_id: input.portfolio_id,
        placement_revision: input.placement_revision,
        geometry_revision: input.geometry_revision,
        corridors: input.corridors,
        routes: &promoted_routes,
        fixed_context: input.fixed_context,
    };
    let mut search_cache = AlternativeSearchCache::new(input.corridors);
    let (new_portfolio, mut work) = match build_route_family_portfolio_with_plan(
        promoted_input,
        EXPANDED_BOUNDS,
        None,
        &mut search_cache,
    ) {
        RouteFamilyPortfolioBuildOutcome::Complete { portfolio, work } => (portfolio, work),
        unavailable => {
            return RouteFamilyPlanningOutcome::PortfolioUnavailable {
                outcome: unavailable,
            };
        }
    };
    work.family_variables_reused = prior.variables.len() as u64;
    work.families_reused = prior
        .variables
        .iter()
        .map(|variable| variable.families.len() as u64)
        .sum();
    work.incremental_reuse_contract = Some(INCREMENTAL_PROMOTION_REUSE_CONTRACT.into());
    work.incremental_parent_candidate_set_fingerprint =
        Some(prior.pair_analysis.candidate_set_fingerprint.clone());
    work.incremental_parent_fixed_context_fingerprint =
        prior.pair_analysis.fixed_context_fingerprint.clone();
    work.incremental_retained_branches = prior
        .variables
        .iter()
        .map(|variable| variable.branch.clone())
        .collect();
    work.incremental_promoted_branches = promoted_branches.iter().cloned().collect();

    let old_families = certified_families(prior);
    let new_families = certified_families(&new_portfolio);
    let delta_conflicts =
        match analyze_family_cross_product(&old_families, &new_families, &mut work) {
            Ok(conflicts) => conflicts,
            Err((route, detail)) => {
                return rejected(format!("{route}: {detail}"), work);
            }
        };

    let expected_old_pairs = expected_family_pairs(&prior.variables);
    if prior.pair_analysis.analyzed_family_pairs != Some(expected_old_pairs) {
        return rejected(
            "parent old-old family-pair matrix is not complete".into(),
            work,
        );
    }
    let expected_parent_context_pairs =
        expected_family_context_pairs(&prior.variables, &prior_context.runs);
    if prior.pair_analysis.analyzed_family_context_pairs != Some(expected_parent_context_pairs) {
        return rejected(
            "parent old-context family matrix is not complete".into(),
            work,
        );
    }
    let retained_context_ids = expected_context
        .iter()
        .map(|run| run.context_id.as_str())
        .collect::<BTreeSet<_>>();
    let reused_context_pairs = expected_family_context_pairs(&prior.variables, &expected_context);
    work.family_pair_outcomes_reused = expected_old_pairs;
    work.family_context_pair_outcomes_reused = reused_context_pairs;

    let mut variables = prior.variables.clone();
    variables.extend(new_portfolio.variables.clone());
    variables.sort_by(|left, right| left.branch.cmp(&right.branch));
    let all_families = old_families
        .iter()
        .cloned()
        .chain(new_families.iter().cloned())
        .collect::<Vec<_>>();
    let graph_map = input
        .corridors
        .iter()
        .map(|graph| (graph.layer.as_str(), graph))
        .collect::<BTreeMap<_, _>>();
    let passage_certificates = build_fixed_witness_passage_certificates(
        &all_families,
        &graph_map,
        input.placement_revision,
    );
    let mut conflicts = prior.pair_analysis.conflicts.clone();
    conflicts.extend(new_portfolio.pair_analysis.conflicts.clone());
    conflicts.extend(delta_conflicts);
    conflicts.sort_by(|left, right| {
        (&left.left_family_id, &left.right_family_id)
            .cmp(&(&right.left_family_id, &right.right_family_id))
    });
    let mut context_conflicts = prior
        .pair_analysis
        .context_conflicts
        .iter()
        .filter(|conflict| retained_context_ids.contains(conflict.context_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    context_conflicts.extend(new_portfolio.pair_analysis.context_conflicts.clone());
    context_conflicts.sort_by(|left, right| {
        (&left.family_id, &left.context_id).cmp(&(&right.family_id, &right.context_id))
    });

    let mut corridor_epochs = prior.corridor_epochs.clone();
    for epoch in new_portfolio.corridor_epochs {
        if let Some(old) = corridor_epochs.iter().find(|old| old.layer == epoch.layer) {
            if old != &epoch {
                return rejected(
                    format!("promoted layer {:?} changed corridor epoch", epoch.layer),
                    work,
                );
            }
        } else {
            corridor_epochs.push(epoch);
        }
    }
    corridor_epochs.sort_by(|left, right| left.layer.cmp(&right.layer));
    let finalize_started = Instant::now();
    let mut portfolio = RouteFamilyPortfolio {
        schema_version: ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into(),
        feasibility_mode: Some(RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise),
        portfolio_id: input.portfolio_id.into(),
        placement_revision: input.placement_revision,
        geometry_revision: Some(input.geometry_revision),
        corridor_epochs,
        family_generation: FamilyGenerationCertificate {
            method: PROMOTED_FAMILY_GENERATION_METHOD.into(),
            max_families_per_variable: prior
                .family_generation
                .max_families_per_variable
                .max(EXPANDED_BOUNDS.maximum_certified_per_variable),
            search_complete: true,
            budget_exhausted: false,
        },
        passage_certificates,
        variables,
        fixed_context: new_portfolio.fixed_context,
        pair_analysis: PairAnalysisCertificate {
            method: FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into(),
            placement_revision: input.placement_revision,
            corridor_epoch_fingerprint: String::new(),
            candidate_set_fingerprint: String::new(),
            search_complete: true,
            analyzed_family_pairs: Some(expected_old_pairs + work.family_pairs_analyzed),
            fixed_context_fingerprint: Some(String::new()),
            analyzed_family_context_pairs: Some(
                reused_context_pairs + work.family_context_pairs_analyzed,
            ),
            context_conflicts,
            conflicts,
        },
    };
    if let Err(error) = portfolio.seal_pair_analysis_fingerprints() {
        work.portfolio_finalize_elapsed_micros += finalize_started.elapsed().as_micros() as u64;
        return rejected(
            format!("incremental promoted portfolio fingerprint failed: {error}"),
            work,
        );
    }
    work.incremental_result_candidate_set_fingerprint =
        Some(portfolio.pair_analysis.candidate_set_fingerprint.clone());
    if let Err(issues) = portfolio.validate() {
        work.portfolio_finalize_elapsed_micros += finalize_started.elapsed().as_micros() as u64;
        return RouteFamilyPlanningOutcome::PortfolioUnavailable {
            outcome: RouteFamilyPortfolioBuildOutcome::InvalidProducedPortfolio { issues, work },
        };
    }
    work.portfolio_finalize_elapsed_micros += finalize_started.elapsed().as_micros() as u64;
    let assignment = match select_route_families_timed(&portfolio, limits, &mut work) {
        Ok(assignment) => assignment,
        Err(FamilyAssignmentError::InvalidPortfolio(issues)) => {
            return RouteFamilyPlanningOutcome::AssignmentRejected {
                error: format!(
                    "validated incremental portfolio was rejected with {} issue(s)",
                    issues.len()
                ),
                production_work: work,
            };
        }
    };
    let raw_frontiers = work.raw_frontiers.clone();
    let mut combined_raw_frontiers = canonical_prior_frontiers;
    combined_raw_frontiers.extend(raw_frontiers.iter().cloned());
    let promoted_bounds = PortfolioBounds {
        minimum_certified_per_variable: MAX_FAMILIES_PER_VARIABLE,
        maximum_certified_per_variable: portfolio.family_generation.max_families_per_variable,
        raw_candidate_limit: EXPANDED_BOUNDS.raw_candidate_limit,
        generation_method: PROMOTED_FAMILY_GENERATION_METHOD,
    };
    push_stage(
        &mut work,
        RouteFamilyPlanningStageKind::IncrementalPromotion,
        promoted_bounds,
        assignment_stage_outcome(&assignment),
        Some(&portfolio),
        &raw_frontiers,
        assignment_work(&assignment).clone(),
    );
    if !matches!(
        assignment,
        FamilyAssignmentOutcome::BoundedInfeasible { .. }
    ) {
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work: work,
            assignment,
        };
    }

    let mut targeted_evidence =
        targeted_deepening_evidence(&assignment, &input, &portfolio, &combined_raw_frontiers)
            .expect("bounded incremental infeasibility has exact implicated-variable evidence");
    if targeted_evidence.selected_variables.is_empty() {
        let outcome = if targeted_evidence.eligible_variables.is_empty() {
            RouteFamilyPlanningStageOutcome::NoEligibleImplicatedFrontier
        } else {
            RouteFamilyPlanningStageOutcome::GenerationBudgetExhausted
        };
        push_targeted_stage(
            &mut work,
            outcome,
            Some(&portfolio),
            &[],
            FamilyAssignmentWork::default(),
            targeted_evidence,
        );
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work: work,
            assignment,
        };
    }

    let targeted_branches = targeted_evidence
        .selected_variables
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let (projected_family_pairs, projected_context_pairs) =
        targeted_pair_work_upper_bounds(&input, &portfolio, &targeted_branches);
    targeted_evidence.projected_family_pairs = projected_family_pairs;
    targeted_evidence.projected_family_context_pairs = projected_context_pairs;
    if projected_family_pairs > MAX_TARGETED_FAMILY_PAIR_ANALYSES
        || projected_context_pairs > MAX_TARGETED_FAMILY_CONTEXT_PAIR_ANALYSES
    {
        if projected_family_pairs > MAX_TARGETED_FAMILY_PAIR_ANALYSES {
            targeted_evidence
                .exhaustions
                .push(RouteFamilyTargetedDeepeningExhaustion::FamilyPairWorkLimitReached);
        }
        if projected_context_pairs > MAX_TARGETED_FAMILY_CONTEXT_PAIR_ANALYSES {
            targeted_evidence
                .exhaustions
                .push(RouteFamilyTargetedDeepeningExhaustion::FamilyContextWorkLimitReached);
        }
        push_targeted_stage(
            &mut work,
            RouteFamilyPlanningStageOutcome::GenerationBudgetExhausted,
            Some(&portfolio),
            &[],
            FamilyAssignmentWork::default(),
            targeted_evidence,
        );
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work: work,
            assignment,
        };
    }

    let targeted_routes = input
        .routes
        .iter()
        .filter(|route| targeted_branches.contains(route.branch_id))
        .cloned()
        .collect::<Vec<_>>();
    let targeted_input = RouteFamilyPortfolioInput {
        portfolio_id: input.portfolio_id,
        placement_revision: input.placement_revision,
        geometry_revision: input.geometry_revision,
        corridors: input.corridors,
        routes: &targeted_routes,
        fixed_context: input.fixed_context,
    };
    let (targeted_portfolio, mut targeted_work) = match build_route_family_portfolio_with_plan(
        targeted_input,
        TARGETED_BOUNDS,
        Some(TargetedReusePlan {
            prior_portfolio: &portfolio,
            targeted_branches: &targeted_branches,
            prior_raw_candidate_limit: RAW_FAMILY_CANDIDATE_LIMIT,
        }),
        &mut search_cache,
    ) {
        RouteFamilyPortfolioBuildOutcome::Complete { portfolio, work } => (portfolio, work),
        mut unavailable => {
            let outcome = unavailable_stage_outcome(&unavailable);
            let mut stage_work = std::mem::take(build_outcome_work_mut(&mut unavailable));
            let targeted_raw_frontiers = stage_work.raw_frontiers.clone();
            record_targeted_work(&mut targeted_evidence, &stage_work, &unavailable);
            merge_production_work(&mut work, &mut stage_work);
            push_targeted_stage(
                &mut work,
                outcome,
                None,
                &targeted_raw_frontiers,
                FamilyAssignmentWork::default(),
                targeted_evidence,
            );
            *build_outcome_work_mut(&mut unavailable) = work;
            return RouteFamilyPlanningOutcome::PortfolioUnavailable {
                outcome: unavailable,
            };
        }
    };
    let targeted_raw_frontiers = targeted_work.raw_frontiers.clone();
    record_targeted_work_success(&mut targeted_evidence, &targeted_work);
    if targeted_work.families_certified > MAX_TARGETED_NEW_FAMILIES as u64 {
        targeted_evidence
            .exhaustions
            .push(RouteFamilyTargetedDeepeningExhaustion::NewFamilyWorkLimitReached);
        merge_production_work(&mut work, &mut targeted_work);
        push_targeted_stage(
            &mut work,
            RouteFamilyPlanningStageOutcome::GenerationBudgetExhausted,
            None,
            &targeted_raw_frontiers,
            FamilyAssignmentWork::default(),
            targeted_evidence,
        );
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work: work,
            assignment,
        };
    }
    let prior_targeted_variables = portfolio
        .variables
        .iter()
        .filter(|variable| targeted_branches.contains(&variable.branch))
        .cloned()
        .collect::<Vec<_>>();
    if prior_targeted_variables == targeted_portfolio.variables {
        targeted_evidence
            .exhaustions
            .push(RouteFamilyTargetedDeepeningExhaustion::NoAdmittedDomainGrowth);
        merge_production_work(&mut work, &mut targeted_work);
        push_targeted_stage(
            &mut work,
            RouteFamilyPlanningStageOutcome::NoAdmittedDomainGrowth,
            Some(&portfolio),
            &targeted_raw_frontiers,
            FamilyAssignmentWork::default(),
            targeted_evidence,
        );
        return RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work: work,
            assignment,
        };
    }

    let targeted_portfolio = match merge_incremental_targeted_portfolio(
        &input,
        &portfolio,
        targeted_portfolio,
        &targeted_branches,
        &mut targeted_work,
    ) {
        Ok(portfolio) => portfolio,
        Err(detail) => {
            merge_production_work(&mut work, &mut targeted_work);
            return rejected(detail, work);
        }
    };
    // The merge retains every non-targeted variable and its certified
    // outcomes. Record the stage evidence only after those reuse counters have
    // been added, so the per-stage certificate agrees with the work that is
    // subsequently folded into the aggregate.
    record_targeted_work_success(&mut targeted_evidence, &targeted_work);
    merge_production_work(&mut work, &mut targeted_work);
    work.incremental_result_candidate_set_fingerprint = Some(
        targeted_portfolio
            .pair_analysis
            .candidate_set_fingerprint
            .clone(),
    );
    let targeted_assignment =
        match select_route_families_timed(&targeted_portfolio, limits, &mut work) {
            Ok(assignment) => assignment,
            Err(FamilyAssignmentError::InvalidPortfolio(issues)) => {
                push_targeted_stage(
                    &mut work,
                    RouteFamilyPlanningStageOutcome::PortfolioRejected,
                    Some(&targeted_portfolio),
                    &targeted_raw_frontiers,
                    FamilyAssignmentWork::default(),
                    targeted_evidence,
                );
                return RouteFamilyPlanningOutcome::AssignmentRejected {
                    error: format!(
                        "validated incremental targeted portfolio was rejected with {} issue(s)",
                        issues.len()
                    ),
                    production_work: work,
                };
            }
        };
    if matches!(
        targeted_assignment,
        FamilyAssignmentOutcome::BudgetExhausted { .. }
    ) {
        targeted_evidence
            .exhaustions
            .push(RouteFamilyTargetedDeepeningExhaustion::AssignmentNodeLimitReached);
    }
    push_targeted_stage(
        &mut work,
        assignment_stage_outcome(&targeted_assignment),
        Some(&targeted_portfolio),
        &targeted_raw_frontiers,
        assignment_work(&targeted_assignment).clone(),
        targeted_evidence,
    );
    RouteFamilyPlanningOutcome::Assignment {
        portfolio: targeted_portfolio,
        production_work: work,
        assignment: targeted_assignment,
    }
}

fn merge_incremental_targeted_portfolio(
    input: &RouteFamilyPortfolioInput<'_>,
    prior: &RouteFamilyPortfolio,
    targeted: RouteFamilyPortfolio,
    targeted_branches: &BTreeSet<String>,
    work: &mut RouteFamilyProductionWork,
) -> Result<RouteFamilyPortfolio, String> {
    let unchanged_variables = prior
        .variables
        .iter()
        .filter(|variable| !targeted_branches.contains(&variable.branch))
        .cloned()
        .collect::<Vec<_>>();
    let expected_targeted_branches = targeted_branches.iter().collect::<BTreeSet<_>>();
    let observed_targeted_branches = targeted
        .variables
        .iter()
        .map(|variable| &variable.branch)
        .collect::<BTreeSet<_>>();
    if observed_targeted_branches != expected_targeted_branches {
        return Err(
            "incremental targeted frontier does not contain exactly the selected variables".into(),
        );
    }
    for epoch in &targeted.corridor_epochs {
        if prior
            .corridor_epochs
            .iter()
            .find(|prior_epoch| prior_epoch.layer == epoch.layer)
            != Some(epoch)
        {
            return Err(format!(
                "incremental targeted layer {:?} changed corridor epoch",
                epoch.layer
            ));
        }
    }

    let unchanged_families = certified_families(prior)
        .into_iter()
        .filter(|family| !targeted_branches.contains(&family.branch))
        .collect::<Vec<_>>();
    let targeted_families = certified_families(&targeted);
    let delta_conflicts =
        analyze_family_cross_product(&unchanged_families, &targeted_families, work)
            .map_err(|(route, detail)| format!("{route}: {detail}"))?;

    let unchanged_family_ids = unchanged_families
        .iter()
        .map(|family| family.family.family_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut conflicts = prior
        .pair_analysis
        .conflicts
        .iter()
        .filter(|conflict| {
            unchanged_family_ids.contains(conflict.left_family_id.as_str())
                && unchanged_family_ids.contains(conflict.right_family_id.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    conflicts.extend(targeted.pair_analysis.conflicts.iter().cloned());
    conflicts.extend(delta_conflicts);
    conflicts.sort_by(|left, right| {
        (&left.left_family_id, &left.right_family_id)
            .cmp(&(&right.left_family_id, &right.right_family_id))
    });

    let mut context_conflicts = prior
        .pair_analysis
        .context_conflicts
        .iter()
        .filter(|conflict| unchanged_family_ids.contains(conflict.family_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    context_conflicts.extend(targeted.pair_analysis.context_conflicts.iter().cloned());
    context_conflicts.sort_by(|left, right| {
        (&left.family_id, &left.context_id).cmp(&(&right.family_id, &right.context_id))
    });

    let fixed_context = prior
        .fixed_context
        .clone()
        .ok_or_else(|| "incremental targeted parent lacks fixed-context evidence".to_owned())?;
    if targeted.fixed_context.as_ref() != Some(&fixed_context) {
        return Err("incremental targeted frontier changed the fixed-context certificate".into());
    }
    let reused_family_pairs = expected_family_pairs(&unchanged_variables);
    let reused_context_pairs =
        expected_family_context_pairs(&unchanged_variables, &fixed_context.runs);
    work.family_variables_reused += unchanged_variables.len() as u64;
    work.families_reused += unchanged_variables
        .iter()
        .map(|variable| variable.families.len() as u64)
        .sum::<u64>();
    work.family_pair_outcomes_reused += reused_family_pairs;
    work.family_context_pair_outcomes_reused += reused_context_pairs;

    let mut variables = unchanged_variables;
    variables.extend(targeted.variables);
    variables.sort_by(|left, right| left.branch.cmp(&right.branch));
    let all_families = unchanged_families
        .iter()
        .cloned()
        .chain(targeted_families.iter().cloned())
        .collect::<Vec<_>>();
    let graph_map = input
        .corridors
        .iter()
        .map(|graph| (graph.layer.as_str(), graph))
        .collect::<BTreeMap<_, _>>();
    let finalize_started = Instant::now();
    let passage_certificates = build_fixed_witness_passage_certificates(
        &all_families,
        &graph_map,
        input.placement_revision,
    );
    let mut portfolio = RouteFamilyPortfolio {
        schema_version: ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into(),
        feasibility_mode: Some(RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise),
        portfolio_id: input.portfolio_id.into(),
        placement_revision: input.placement_revision,
        geometry_revision: Some(input.geometry_revision),
        corridor_epochs: prior.corridor_epochs.clone(),
        family_generation: FamilyGenerationCertificate {
            method: INCREMENTAL_TARGETED_FAMILY_GENERATION_METHOD.into(),
            max_families_per_variable: prior
                .family_generation
                .max_families_per_variable
                .max(TARGETED_BOUNDS.maximum_certified_per_variable),
            search_complete: true,
            budget_exhausted: false,
        },
        passage_certificates,
        variables,
        fixed_context: Some(fixed_context),
        pair_analysis: PairAnalysisCertificate {
            method: FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into(),
            placement_revision: input.placement_revision,
            corridor_epoch_fingerprint: String::new(),
            candidate_set_fingerprint: String::new(),
            search_complete: true,
            analyzed_family_pairs: Some(reused_family_pairs + work.family_pairs_analyzed),
            fixed_context_fingerprint: Some(String::new()),
            analyzed_family_context_pairs: Some(
                reused_context_pairs + work.family_context_pairs_analyzed,
            ),
            context_conflicts,
            conflicts,
        },
    };
    let result = (|| {
        portfolio
            .seal_pair_analysis_fingerprints()
            .map_err(|error| format!("incremental targeted fingerprint failed: {error}"))?;
        portfolio.validate().map_err(|issues| {
            format!(
                "incremental targeted portfolio failed independent validation with {} issue(s): {issues:?}",
                issues.len()
            )
        })?;
        Ok(portfolio)
    })();
    work.portfolio_finalize_elapsed_micros += finalize_started.elapsed().as_micros() as u64;
    result
}

fn fixed_context_run(
    context: &RouteFamilyFixedContextInput,
    placement_revision: u64,
    geometry_revision: u64,
) -> FixedContextRun {
    FixedContextRun {
        context_id: context.context_id.clone(),
        branch: context.branch.clone(),
        electrical_net: context.electrical_net.clone(),
        layer: context.layer.clone(),
        required_width: context.physical_width,
        required_clearance: context.clearance,
        polyline: context.points.iter().copied().map(family_point).collect(),
        placement_revision,
        geometry_revision,
    }
}

fn certified_families(portfolio: &RouteFamilyPortfolio) -> Vec<CertifiedFamily> {
    portfolio
        .variables
        .iter()
        .flat_map(|variable| {
            variable
                .families
                .iter()
                .cloned()
                .map(|family| CertifiedFamily {
                    branch: variable.branch.clone(),
                    electrical_net: variable.electrical_net.clone(),
                    required_clearance: variable
                        .required_clearance
                        .expect("validated V4 variables carry required clearance"),
                    family,
                })
        })
        .collect()
}

fn expected_family_pairs(variables: &[RouteFamilyVariable]) -> u64 {
    variables
        .iter()
        .enumerate()
        .map(|(left, variable)| {
            variables[left + 1..]
                .iter()
                .filter(|right| right.electrical_net != variable.electrical_net)
                .map(|right| variable.families.len() as u64 * right.families.len() as u64)
                .sum::<u64>()
        })
        .sum()
}

fn expected_family_context_pairs(
    variables: &[RouteFamilyVariable],
    context: &[FixedContextRun],
) -> u64 {
    variables
        .iter()
        .map(|variable| {
            variable.families.len() as u64
                * context
                    .iter()
                    .filter(|run| run.electrical_net != variable.electrical_net)
                    .count() as u64
        })
        .sum()
}

fn build_route_family_portfolio_with_bounds(
    input: RouteFamilyPortfolioInput<'_>,
    bounds: PortfolioBounds,
) -> RouteFamilyPortfolioBuildOutcome {
    let mut search_cache = AlternativeSearchCache::new(input.corridors);
    build_route_family_portfolio_with_plan(input, bounds, None, &mut search_cache)
}

fn build_route_family_portfolio_with_plan(
    input: RouteFamilyPortfolioInput<'_>,
    bounds: PortfolioBounds,
    reuse: Option<TargetedReusePlan<'_>>,
    search_cache: &mut AlternativeSearchCache,
) -> RouteFamilyPortfolioBuildOutcome {
    let started = Instant::now();
    let mut outcome =
        build_route_family_portfolio_with_plan_impl(input, bounds, reuse, search_cache);
    let work = build_outcome_work_mut(&mut outcome);
    work.portfolio_builds += 1;
    work.portfolio_build_elapsed_micros += started.elapsed().as_micros() as u64;
    outcome
}

fn build_route_family_portfolio_with_plan_impl(
    input: RouteFamilyPortfolioInput<'_>,
    bounds: PortfolioBounds,
    reuse: Option<TargetedReusePlan<'_>>,
    search_cache: &mut AlternativeSearchCache,
) -> RouteFamilyPortfolioBuildOutcome {
    let mut work = RouteFamilyProductionWork::default();
    if input.portfolio_id.is_empty() {
        return unsupported(
            None,
            RouteFamilyUnsupportedReason::EmptyPortfolioId,
            "portfolio ID is empty",
            work,
        );
    }
    if input.routes.is_empty() {
        return unsupported(
            None,
            RouteFamilyUnsupportedReason::EmptyRouteSet,
            "at least one route is required",
            work,
        );
    }
    if let Some(reuse) = &reuse {
        if reuse.prior_portfolio.portfolio_id != input.portfolio_id
            || reuse.prior_portfolio.placement_revision != input.placement_revision
            || reuse.prior_portfolio.geometry_revision != Some(input.geometry_revision)
        {
            return unsupported(
                None,
                RouteFamilyUnsupportedReason::AlternativeCouldNotBeCertified,
                "targeted deepening prior portfolio does not match the current identity and geometry epoch",
                work,
            );
        }
    }

    let mut graphs = BTreeMap::<&str, &CorridorGraph>::new();
    for graph in input.corridors {
        if graph.placement_revision != input.placement_revision
            || graph.semantic.placement_revision != input.placement_revision
            || graph.semantic.revision != graph.revision
            || graph.semantic.layer != graph.layer
        {
            return unsupported(
                None,
                RouteFamilyUnsupportedReason::CorridorEpochMismatch,
                format!(
                    "corridor layer {:?} is stale or internally inconsistent",
                    graph.layer
                ),
                work,
            );
        }
        if !graph.cut_basis.complete
            || graph.cut_basis.placement_revision != input.placement_revision
            || graph.cut_basis.revision != graph.revision
        {
            return unsupported(
                None,
                RouteFamilyUnsupportedReason::IncompleteCutBasis,
                format!(
                    "corridor layer {:?} lacks a complete current cut basis",
                    graph.layer
                ),
                work,
            );
        }
        if graphs.insert(graph.layer.as_str(), graph).is_some() {
            return unsupported(
                None,
                RouteFamilyUnsupportedReason::DuplicateCorridorLayer,
                format!("corridor layer {:?} occurs more than once", graph.layer),
                work,
            );
        }
    }

    let mut ordered_routes = input.routes.iter().collect::<Vec<_>>();
    ordered_routes.sort_by(|left, right| {
        (left.electrical_net, left.branch_id).cmp(&(right.electrical_net, right.branch_id))
    });
    let mut branches = BTreeSet::new();
    for route in &ordered_routes {
        if route.branch_id.is_empty()
            || route.electrical_net.is_empty()
            || route.layer.is_empty()
            || route.from_component.is_empty()
            || route.to_component.is_empty()
        {
            return unsupported(
                Some(route.branch_id),
                RouteFamilyUnsupportedReason::EmptyIdentity,
                "route, net, layer, and endpoint component identities must be nonempty",
                work,
            );
        }
        if !branches.insert(route.branch_id) {
            return unsupported(
                Some(route.branch_id),
                RouteFamilyUnsupportedReason::DuplicateBranch,
                "route branch identity is duplicated",
                work,
            );
        }
        if route.reference.len() < 2
            || route.reference.iter().any(|point| !finite_point(*point))
            || !finite_point(route.from)
            || !finite_point(route.from_toward)
            || !finite_point(route.to)
            || !finite_point(route.to_toward)
            || route.reference.first() != Some(&route.from)
            || route.reference.last() != Some(&route.to)
        {
            return unsupported(
                Some(route.branch_id),
                RouteFamilyUnsupportedReason::InvalidRouteGeometry,
                "route reference must be finite, have two points, and match both terminals",
                work,
            );
        }
        if !route.physical_width.is_finite()
            || route.physical_width <= 0.0
            || !route.clearance.is_finite()
            || route.clearance < 0.0
        {
            return unsupported(
                Some(route.branch_id),
                RouteFamilyUnsupportedReason::InvalidWidthOrClearance,
                "physical width must be positive and clearance nonnegative",
                work,
            );
        }
        if !graphs.contains_key(route.layer) {
            return unsupported(
                Some(route.branch_id),
                RouteFamilyUnsupportedReason::CorridorLayerMissing,
                format!(
                    "no current corridor graph exists for layer {:?}",
                    route.layer
                ),
                work,
            );
        }
    }

    let mut context_ids = BTreeSet::new();
    for context in input.fixed_context {
        if context.context_id.is_empty()
            || context.branch.is_empty()
            || context.electrical_net.is_empty()
            || context.layer.is_empty()
        {
            return unsupported(
                Some(&context.branch),
                RouteFamilyUnsupportedReason::EmptyIdentity,
                "fixed context ID, branch, electrical net, and layer must be nonempty",
                work,
            );
        }
        if !context_ids.insert(context.context_id.as_str()) {
            return unsupported(
                Some(&context.branch),
                RouteFamilyUnsupportedReason::DuplicateBranch,
                "fixed context run identity is duplicated",
                work,
            );
        }
        if branches.contains(context.branch.as_str()) {
            return unsupported(
                Some(&context.branch),
                RouteFamilyUnsupportedReason::DuplicateBranch,
                "selected component branches must not be serialized as fixed outside copper",
                work,
            );
        }
        if !graphs.contains_key(context.layer.as_str()) {
            return unsupported(
                Some(&context.branch),
                RouteFamilyUnsupportedReason::CorridorLayerMissing,
                format!(
                    "fixed context layer {:?} has no current corridor graph",
                    context.layer
                ),
                work,
            );
        }
        if context.points.is_empty()
            || context.points.iter().any(|point| !finite_point(*point))
            || !context.physical_width.is_finite()
            || context.physical_width <= 0.0
            || !context.clearance.is_finite()
            || context.clearance < 0.0
        {
            return unsupported(
                Some(&context.branch),
                RouteFamilyUnsupportedReason::InvalidRouteGeometry,
                "fixed context requires nonempty finite same-layer geometry, positive width, and nonnegative clearance",
                work,
            );
        }
    }

    let prior_variables = reuse
        .as_ref()
        .map(|plan| {
            plan.prior_portfolio
                .variables
                .iter()
                .map(|variable| (variable.branch.as_str(), variable))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let mut certified = Vec::new();
    for route in ordered_routes {
        let prior_variable = prior_variables.get(route.branch_id).copied();
        if let Some(plan) = &reuse {
            let Some(prior_variable) = prior_variable else {
                return unsupported(
                    Some(route.branch_id),
                    RouteFamilyUnsupportedReason::AlternativeCouldNotBeCertified,
                    "targeted deepening prior portfolio is missing the route variable",
                    work,
                );
            };
            if prior_variable.electrical_net != route.electrical_net
                || prior_variable.required_width != route.physical_width
                || prior_variable.required_clearance != Some(route.clearance)
            {
                return unsupported(
                    Some(route.branch_id),
                    RouteFamilyUnsupportedReason::AlternativeCouldNotBeCertified,
                    "targeted deepening prior variable does not match the current route contract",
                    work,
                );
            }
            if !plan.targeted_branches.contains(route.branch_id) {
                work.family_variables_reused += 1;
                work.families_reused += prior_variable.families.len() as u64;
                certified.extend(prior_variable.families.iter().cloned().map(|family| {
                    CertifiedFamily {
                        branch: route.branch_id.to_owned(),
                        electrical_net: route.electrical_net.to_owned(),
                        required_clearance: route.clearance,
                        family,
                    }
                }));
                continue;
            }
        }
        let graph = graphs[route.layer];
        let request = request_for(route, route.reference, None, false);
        work.family_searches += 1;
        work.family_search_cache_lookups += 1;
        work.raw_candidates_requested += bounds.raw_candidate_limit as u64;
        let search_started = Instant::now();
        let (search, cache_hit) =
            search_cache.enumerate(graph, request, bounds.raw_candidate_limit);
        work.alternative_search_elapsed_micros += search_started.elapsed().as_micros() as u64;
        if cache_hit {
            work.family_search_cache_hits += 1;
        } else {
            work.family_search_new_executions += 1;
        }
        let evidence = match search {
            Ok(evidence) => evidence,
            Err(error) => {
                return unsupported(
                    Some(route.branch_id),
                    RouteFamilyUnsupportedReason::AlternativeCouldNotBeCertified,
                    format!("producer requested an invalid raw family bound: {error}"),
                    work,
                );
            }
        };
        if cache_hit {
            work.visibility_states_reused += evidence.explored_states as u64;
        } else {
            work.visibility_states_new += evidence.explored_states as u64;
        }
        work.visibility_nodes += evidence.nodes.len() as u64;
        work.visibility_edge_candidates += evidence.evaluated_edge_candidates as u64;
        work.visibility_edges += evidence.edges.len() as u64;
        work.visibility_edges_rejected += evidence.rejected_edges as u64;
        work.visibility_geometry_units += evidence.visibility_geometry_units as u64;
        work.visibility_states += evidence.explored_states as u64;
        let returned_raw_candidates = evidence.alternatives.len();
        match evidence.termination {
            AlternativeSearchTermination::StateLimitReached
            | AlternativeSearchTermination::VisibilityGraphLimitReached => {
                work.raw_frontiers.push(RouteFamilyRawFrontierEvidence {
                    branch: route.branch_id.to_owned(),
                    requested_raw_bound: bounds.raw_candidate_limit,
                    returned_raw_candidates,
                    termination: evidence.termination,
                    admitted_family_count: 0,
                });
                return RouteFamilyPortfolioBuildOutcome::GenerationBudgetExhausted {
                    route: route.branch_id.to_owned(),
                    termination: evidence.termination,
                    work,
                };
            }
            AlternativeSearchTermination::SearchExhausted
            | AlternativeSearchTermination::FamilyLimitReached => {}
        }
        if evidence.termination == AlternativeSearchTermination::FamilyLimitReached
            && evidence.alternatives.len() != bounds.raw_candidate_limit
        {
            return unsupported(
                Some(route.branch_id),
                RouteFamilyUnsupportedReason::AlternativeCouldNotBeCertified,
                "family-limit termination did not return the declared raw candidate frontier",
                work,
            );
        }
        if evidence.alternatives.is_empty() {
            work.raw_frontiers.push(RouteFamilyRawFrontierEvidence {
                branch: route.branch_id.to_owned(),
                requested_raw_bound: bounds.raw_candidate_limit,
                returned_raw_candidates,
                termination: evidence.termination,
                admitted_family_count: 0,
            });
            return RouteFamilyPortfolioBuildOutcome::NoFamily {
                route: route.branch_id.to_owned(),
                termination: evidence.termination,
                work,
            };
        }

        let mut seen_words = BTreeSet::new();
        let mut seen_family_ids = BTreeSet::new();
        let mut route_families = Vec::new();
        for (raw_index, alternative) in evidence.alternatives.into_iter().enumerate() {
            if route_families.len() == bounds.maximum_certified_per_variable {
                break;
            }
            work.raw_candidates_examined += 1;
            let word_key = cut_word_key(&alternative.homotopy);
            if !seen_words.insert(word_key) {
                return unsupported(
                    Some(route.branch_id),
                    RouteFamilyUnsupportedReason::AlternativeCutWordDuplicated,
                    "bounded generator returned the same target cut word twice",
                    work,
                );
            }
            work.alternatives_reembedded += 1;
            let reembedding_started = Instant::now();
            let embedding = embed_route(
                graph,
                request_for(
                    route,
                    &alternative.polyline,
                    Some(&alternative.homotopy),
                    true,
                ),
            );
            work.alternative_reembedding_elapsed_micros +=
                reembedding_started.elapsed().as_micros() as u64;
            let family_id = stable_family_id(route, &embedding);
            if seen_family_ids.contains(&family_id) {
                work.raw_candidates_rejected += 1;
                work.rejection_evidence.push(RouteFamilyCandidateRejection {
                    branch: route.branch_id.to_owned(),
                    raw_ordinal: raw_index + 1,
                    family_id,
                    reason: "duplicate_stable_family_id".into(),
                    detail: "a later raw candidate reduced to an already admitted stable family identity"
                        .into(),
                    projection: None,
                });
                continue;
            }
            if !embedding_is_strict(graph, route, &embedding) {
                work.raw_candidates_rejected += 1;
                work.rejection_evidence.push(RouteFamilyCandidateRejection {
                    branch: route.branch_id.to_owned(),
                    raw_ordinal: raw_index + 1,
                    family_id: family_id.clone(),
                    reason: "full_width_class_or_epoch_certification_failed".into(),
                    detail: format!(
                        "family_id={family_id}; bounded-frontier word {:?}; polyline={:?}; method={:?}; geometry_method={:?}; failure={:?}; complete={}; class_verified={}; epoch_verified={}; clearance_certified={}; minimum_clearance={}; raw_cells={:?}; raw_gates={:?}; semantic_regions={:?}; semantic_passages={:?}; capacity_binding={}",
                        alternative.homotopy,
                        embedding.polyline,
                        embedding.method,
                        embedding.geometry_method,
                        embedding.failure,
                        embedding.complete_route,
                        embedding.route_class_verified,
                        embedding.epoch_homotopy_verified,
                        embedding.clearance.certified,
                        embedding.clearance.minimum_clearance,
                        embedding.cells,
                        embedding.gates,
                        embedding.semantic_regions,
                        embedding.semantic_passages,
                        if embedding.semantic_passages.is_empty() {
                            "unavailable: no raw cell/gate projection exists, so neither direct stable passage keys nor a certified capacity key can be bound"
                        } else {
                            "semantic passage sequence present but strict embedding invariant failed elsewhere"
                        }
                    ),
                    projection: embedding
                        .projection
                        .as_ref()
                        .and_then(|projection| serde_json::to_value(projection).ok()),
                });
                continue;
            }
            let Some(passage_keys) = passage_keys(graph, &embedding) else {
                work.raw_candidates_rejected += 1;
                work.rejection_evidence.push(RouteFamilyCandidateRejection {
                    branch: route.branch_id.to_owned(),
                    raw_ordinal: raw_index + 1,
                    family_id,
                    reason: "semantic_passage_projection_invalid".into(),
                    detail: format!(
                        "certified alternative references invalid semantic passage indices {:?} in a graph with {} passages",
                        embedding.semantic_passages,
                        graph.semantic.passages.len()
                    ),
                    projection: embedding
                        .projection
                        .as_ref()
                        .and_then(|projection| serde_json::to_value(projection).ok()),
                });
                continue;
            };
            seen_family_ids.insert(family_id.clone());
            let family = RouteFamily {
                family_id: family_id.clone(),
                placement_revision: input.placement_revision,
                complete_route: true,
                cost: polyline_length(&embedding.polyline),
                runs: vec![RouteFamilyRun {
                    run_id: format!("{family_id}:run:0"),
                    run_index: 0,
                    layer: route.layer.to_owned(),
                    start_point: 0,
                    end_point: embedding.polyline.len() - 1,
                    corridor_revision: graph.revision,
                    transition_from_previous: RequiredNullable::Null,
                    witness: RouteWitness {
                        polyline: embedding
                            .polyline
                            .iter()
                            .copied()
                            .map(family_point)
                            .collect(),
                        placement_revision: input.placement_revision,
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
                }],
            };
            if reuse.is_none() {
                work.families_certified += 1;
            }
            route_families.push(CertifiedFamily {
                branch: route.branch_id.to_owned(),
                electrical_net: route.electrical_net.to_owned(),
                required_clearance: route.clearance,
                family,
            });
        }
        if let Some(plan) = &reuse {
            let prior_families = prior_variable
                .map(|variable| variable.families.as_slice())
                .unwrap_or_default();
            let regenerated = route_families
                .iter()
                .map(|family| family.family.clone())
                .collect::<Vec<_>>();
            let reconciled = match reconcile_prior_family_frontier(prior_families, &regenerated) {
                Ok(reconciled) => reconciled,
                Err(detail) => {
                    return unsupported(
                        Some(route.branch_id),
                        RouteFamilyUnsupportedReason::AlternativeCouldNotBeCertified,
                        format!(
                            "{detail}; prior_raw_bound={}",
                            plan.prior_raw_candidate_limit
                        ),
                        work,
                    );
                }
            };
            work.prior_family_identities_verified += prior_families.len() as u64;
            work.families_reused += prior_families.len() as u64;
            work.families_certified += reconciled.observed_new_family_ids.len() as u64;
            work.retained_prior_family_ids
                .extend(reconciled.retained_prior_family_ids.iter().cloned());
            work.observed_new_family_ids
                .extend(reconciled.observed_new_family_ids.iter().cloned());
            route_families = reconciled
                .families
                .into_iter()
                .map(|family| CertifiedFamily {
                    branch: route.branch_id.to_owned(),
                    electrical_net: route.electrical_net.to_owned(),
                    required_clearance: route.clearance,
                    family,
                })
                .collect();
        }
        work.raw_frontiers.push(RouteFamilyRawFrontierEvidence {
            branch: route.branch_id.to_owned(),
            requested_raw_bound: bounds.raw_candidate_limit,
            returned_raw_candidates,
            termination: evidence.termination,
            admitted_family_count: route_families.len(),
        });
        // Reaching K is sufficient but not necessary. Exhausting the entire
        // finite product graph with every returned raw class successfully
        // certified is stronger completeness evidence: there simply are
        // fewer than K distinct classes in this corridor epoch. Reject only
        // when a short frontier is budget-truncated or lost a raw class during
        // certification/reconciliation.
        let exhaustive_short_frontier = reuse.is_none()
            && evidence.termination == AlternativeSearchTermination::SearchExhausted
            && route_families.len() == returned_raw_candidates;
        if route_families.len() < bounds.minimum_certified_per_variable
            && !exhaustive_short_frontier
        {
            return RouteFamilyPortfolioBuildOutcome::CertifiableFrontierIncomplete {
                route: route.branch_id.to_owned(),
                accepted: route_families.len(),
                requested_raw_bound: bounds.raw_candidate_limit,
                termination: evidence.termination,
                work,
            };
        }
        work.certifiable_frontiers_completed += 1;
        // Preserve raw generator order; never post-filter by cost.
        certified.extend(route_families);
    }

    let finalize_started = Instant::now();
    let passage_certificates =
        build_fixed_witness_passage_certificates(&certified, &graphs, input.placement_revision);
    work.portfolio_finalize_elapsed_micros += finalize_started.elapsed().as_micros() as u64;
    let (conflicts, context_conflicts) =
        match analyze_pairs_and_context(&certified, input.fixed_context, &mut work) {
            Ok(conflicts) => conflicts,
            Err((route, detail)) => {
                return unsupported(
                    Some(&route),
                    RouteFamilyUnsupportedReason::UnmodeledCopperClearanceConflict,
                    detail,
                    work,
                );
            }
        };
    let finalize_started = Instant::now();
    let mut families_by_branch = BTreeMap::<String, (String, f64, f64, Vec<RouteFamily>)>::new();
    for item in certified {
        let entry = families_by_branch.entry(item.branch).or_insert_with(|| {
            (
                item.electrical_net,
                item.family.runs[0].witness.required_width,
                item.required_clearance,
                Vec::new(),
            )
        });
        entry.3.push(item.family);
    }
    let mut portfolio = RouteFamilyPortfolio {
        schema_version: ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into(),
        feasibility_mode: Some(RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise),
        portfolio_id: input.portfolio_id.to_owned(),
        placement_revision: input.placement_revision,
        geometry_revision: Some(input.geometry_revision),
        corridor_epochs: graphs
            .values()
            .filter(|graph| {
                input.routes.iter().any(|route| route.layer == graph.layer)
                    || input
                        .fixed_context
                        .iter()
                        .any(|context| context.layer == graph.layer)
            })
            .map(|graph| CorridorEpoch {
                layer: graph.layer.clone(),
                corridor_revision: graph.revision,
                semantic_fingerprint: graph.semantic.fingerprint.clone(),
                cut_basis_fingerprint: graph.cut_basis.fingerprint.clone(),
                cut_basis_complete: true,
            })
            .collect(),
        family_generation: FamilyGenerationCertificate {
            method: bounds.generation_method.into(),
            max_families_per_variable: bounds.maximum_certified_per_variable,
            search_complete: true,
            budget_exhausted: false,
        },
        passage_certificates,
        variables: families_by_branch
            .into_iter()
            .map(
                |(branch, (electrical_net, required_width, required_clearance, families))| {
                    RouteFamilyVariable {
                        branch,
                        electrical_net,
                        required_width,
                        required_clearance: Some(required_clearance),
                        families,
                    }
                },
            )
            .collect(),
        fixed_context: reuse.as_ref().map_or_else(
            || {
                Some(FixedContextCertificate {
                    method: FIXED_CONTEXT_ANALYSIS_METHOD.into(),
                    placement_revision: input.placement_revision,
                    geometry_revision: input.geometry_revision,
                    fixed_context_fingerprint: String::new(),
                    search_complete: true,
                    runs: input
                        .fixed_context
                        .iter()
                        .map(|context| FixedContextRun {
                            context_id: context.context_id.clone(),
                            branch: context.branch.clone(),
                            electrical_net: context.electrical_net.clone(),
                            layer: context.layer.clone(),
                            required_width: context.physical_width,
                            required_clearance: context.clearance,
                            polyline: context.points.iter().copied().map(family_point).collect(),
                            placement_revision: input.placement_revision,
                            geometry_revision: input.geometry_revision,
                        })
                        .collect(),
                })
            },
            |plan| plan.prior_portfolio.fixed_context.clone(),
        ),
        pair_analysis: PairAnalysisCertificate {
            method: FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into(),
            placement_revision: input.placement_revision,
            corridor_epoch_fingerprint: String::new(),
            candidate_set_fingerprint: String::new(),
            search_complete: true,
            analyzed_family_pairs: Some(work.family_pairs_analyzed),
            fixed_context_fingerprint: Some(String::new()),
            analyzed_family_context_pairs: Some(work.family_context_pairs_analyzed),
            context_conflicts,
            conflicts,
        },
    };
    if let Err(error) = portfolio.seal_pair_analysis_fingerprints() {
        work.portfolio_finalize_elapsed_micros += finalize_started.elapsed().as_micros() as u64;
        return unsupported(
            None,
            RouteFamilyUnsupportedReason::AlternativeCouldNotBeCertified,
            format!("produced portfolio could not be fingerprinted: {error}"),
            work,
        );
    }
    if let Err(issues) = portfolio.validate() {
        work.portfolio_finalize_elapsed_micros += finalize_started.elapsed().as_micros() as u64;
        return RouteFamilyPortfolioBuildOutcome::InvalidProducedPortfolio { issues, work };
    }
    work.portfolio_finalize_elapsed_micros += finalize_started.elapsed().as_micros() as u64;
    RouteFamilyPortfolioBuildOutcome::Complete { portfolio, work }
}

fn request_for<'a>(
    route: &'a RouteFamilyRouteInput<'a>,
    reference: &'a [Vec2],
    target_homotopy: Option<&'a crate::topology::HomotopyWord>,
    persistent_basis_matches: bool,
) -> RouteEmbeddingRequest<'a> {
    RouteEmbeddingRequest {
        net: route.electrical_net,
        from_component: route.from_component,
        to_component: route.to_component,
        physical_width: route.physical_width,
        clearance: route.clearance,
        from: route.from,
        // A generated family may leave either terminal in a different sector
        // than the input witness. Corridor projection must seed its terminal
        // cells from the witness being certified, not from the stale input
        // direction, or a geometrically valid alternative can fail raw-mesh
        // attribution before semantic passage mapping.
        from_toward: reference[1],
        to: route.to,
        to_toward: reference[reference.len() - 2],
        reference,
        target_homotopy,
        persistent_basis_matches,
    }
}

fn embedding_is_strict(
    graph: &CorridorGraph,
    route: &RouteFamilyRouteInput<'_>,
    embedding: &RouteCorridorEmbedding,
) -> bool {
    embedding.failure.is_none()
        && embedding.complete_route
        && embedding.layer == route.layer
        && embedding.corridor_revision == graph.revision
        && embedding.placement_revision == graph.placement_revision
        && embedding.purpose == EmbeddingPurpose::FullWidthCertificate
        && embedding.route_class_verified
        && embedding.epoch_homotopy_verified
        && embedding.clearance.certified
        && embedding.polyline.len() >= 2
        && embedding.polyline.first() == Some(&route.from)
        && embedding.polyline.last() == Some(&route.to)
}

fn passage_keys(graph: &CorridorGraph, embedding: &RouteCorridorEmbedding) -> Option<Vec<String>> {
    let mut keys = BTreeSet::new();
    for passage_id in &embedding.semantic_passages {
        keys.insert(graph.semantic.passages.get(*passage_id)?.key.clone());
    }
    Some(keys.into_iter().collect())
}

fn build_fixed_witness_passage_certificates(
    families: &[CertifiedFamily],
    graphs: &BTreeMap<&str, &CorridorGraph>,
    placement_revision: u64,
) -> Vec<PassageCapacityCertificate> {
    let mut passages = BTreeMap::<(String, String), f64>::new();
    for item in families {
        for run in &item.family.runs {
            for key in &run.semantic_passage_keys {
                let entry = passages
                    .entry((run.layer.clone(), key.clone()))
                    .or_insert(f64::INFINITY);
                *entry = (*entry).min(run.witness.minimum_clearance);
            }
        }
    }
    passages
        .into_iter()
        .map(
            |((layer, key), minimum_clearance)| PassageCapacityCertificate {
                corridor_revision: graphs[layer.as_str()].revision,
                layer,
                key,
                placement_revision,
                available_width: None,
                minimum_clearance,
                enforcement: Some(PassageEnforcementMode::FixedWitnessPairwise),
                capacity_model: FIXED_WITNESS_NO_SCALAR_CAPACITY_MODEL.into(),
                certified: true,
            },
        )
        .collect()
}

#[cfg(test)]
fn analyze_pairs(
    families: &[CertifiedFamily],
    work: &mut RouteFamilyProductionWork,
) -> Result<Vec<FamilyConflict>, (String, String)> {
    analyze_pairs_and_context(families, &[], work).map(|(conflicts, _)| conflicts)
}

fn analyze_pairs_and_context(
    families: &[CertifiedFamily],
    context: &[RouteFamilyFixedContextInput],
    work: &mut RouteFamilyProductionWork,
) -> Result<(Vec<FamilyConflict>, Vec<FamilyContextConflict>), (String, String)> {
    let route_inputs = families
        .iter()
        .map(|item| {
            let run = &item.family.runs[0];
            (
                run.witness
                    .polyline
                    .iter()
                    .map(|point| Vec2::new(point.x, point.y))
                    .collect::<Vec<_>>(),
                run.witness.required_width,
                item.required_clearance,
            )
        })
        .collect::<Vec<_>>();
    let preparation_started = Instant::now();
    let prepared = route_inputs
        .iter()
        .enumerate()
        .map(|(index, (points, width, clearance))| {
            prepare_fixed_copper_route(FixedCopperRoute {
                polyline: points,
                width: *width,
                clearance: *clearance,
            })
            .map_err(|error| {
                (
                    families[index].branch.clone(),
                    format!("fixed copper witness preparation failed closed: {error}"),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>();
    work.family_witness_preparations += route_inputs.len() as u64;
    work.copper_preparation_elapsed_micros += preparation_started.elapsed().as_micros() as u64;
    let prepared = prepared?;
    let context_preparation_started = Instant::now();
    let context_prepared = context
        .iter()
        .map(|run| {
            prepare_fixed_copper_route(FixedCopperRoute {
                polyline: &run.points,
                width: run.physical_width,
                clearance: run.clearance,
            })
            .map_err(|error| {
                (
                    run.branch.clone(),
                    format!("fixed context preparation failed closed: {error}"),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>();
    work.context_witness_preparations += context.len() as u64;
    work.copper_preparation_elapsed_micros +=
        context_preparation_started.elapsed().as_micros() as u64;
    let context_prepared = context_prepared?;
    let mut conflicts = Vec::new();
    for left_index in 0..families.len() {
        for right_index in left_index + 1..families.len() {
            let left = &families[left_index];
            let right = &families[right_index];
            if left.electrical_net == right.electrical_net {
                continue;
            }
            work.family_pairs_analyzed += 1;
            let left_run = &left.family.runs[0];
            let right_run = &right.family.runs[0];
            if left_run.layer != right_run.layer {
                continue;
            }
            let (left_id, right_id, first_route, second_route) =
                if left.family.family_id <= right.family.family_id {
                    (
                        &left.family.family_id,
                        &right.family.family_id,
                        &prepared[left_index],
                        &prepared[right_index],
                    )
                } else {
                    (
                        &right.family.family_id,
                        &left.family.family_id,
                        &prepared[right_index],
                        &prepared[left_index],
                    )
                };
            let classification_started = Instant::now();
            let classification = classify_prepared_copper_pair(first_route, second_route);
            work.family_pair_classifications += 1;
            work.family_pair_classification_elapsed_micros +=
                classification_started.elapsed().as_micros() as u64;
            let classification = classification.map_err(|error| {
                (
                    format!("{} + {}", left.branch, right.branch),
                    format!("fixed copper-pair classification failed closed: {error}"),
                )
            })?;
            let pair_evidence = classification.evidence();
            work.segment_pairs_analyzed += pair_evidence.segment_pairs_analyzed as u64;
            let kind = match classification {
                FixedCopperPairClassification::Clear(_) => continue,
                FixedCopperPairClassification::CenterlineIntersection(_) => {
                    FamilyConflictKind::CenterlineCrossing
                }
                FixedCopperPairClassification::ClearanceShortfall(_) => {
                    FamilyConflictKind::CopperClearance
                }
            };
            conflicts.push(FamilyConflict {
                left_family_id: left_id.clone(),
                right_family_id: right_id.clone(),
                kind,
                certified: true,
                evidence: ConflictEvidence {
                    layer: RequiredNullable::Value(left_run.layer.clone()),
                    passage_key: RequiredNullable::Null,
                    detail: format!(
                        "fixed full-width witnesses: first_family={left_id}, second_family={right_id}, first_segment={}, second_segment={}, centerline_distance={:.17}, required_distance={:.17}, segment_pairs_analyzed={}",
                        pair_evidence.first_segment_index,
                        pair_evidence.second_segment_index,
                        pair_evidence.minimum_centerline_distance,
                        pair_evidence.required_centerline_distance,
                        pair_evidence.segment_pairs_analyzed,
                    ),
                },
            });
        }
    }
    conflicts.sort_by(|left, right| {
        (&left.left_family_id, &left.right_family_id)
            .cmp(&(&right.left_family_id, &right.right_family_id))
    });
    let mut context_conflicts = Vec::new();
    for (family_index, family) in families.iter().enumerate() {
        let family_run = &family.family.runs[0];
        for (context_index, context_run) in context.iter().enumerate() {
            if family.electrical_net == context_run.electrical_net {
                continue;
            }
            work.family_context_pairs_analyzed += 1;
            if family_run.layer != context_run.layer {
                continue;
            }
            let classification_started = Instant::now();
            let classification = classify_prepared_copper_pair(
                &prepared[family_index],
                &context_prepared[context_index],
            );
            work.family_context_classifications += 1;
            work.family_context_classification_elapsed_micros +=
                classification_started.elapsed().as_micros() as u64;
            let classification = classification.map_err(|error| {
                (
                    format!("{} + {}", family.branch, context_run.branch),
                    format!("fixed family-context classification failed closed: {error}"),
                )
            })?;
            let pair_evidence = classification.evidence();
            work.context_segment_pairs_analyzed += pair_evidence.segment_pairs_analyzed as u64;
            let kind = match classification {
                FixedCopperPairClassification::Clear(_) => continue,
                FixedCopperPairClassification::CenterlineIntersection(_) => {
                    FamilyConflictKind::CenterlineCrossing
                }
                FixedCopperPairClassification::ClearanceShortfall(_) => {
                    FamilyConflictKind::CopperClearance
                }
            };
            let family_id = family.family.family_id.clone();
            let context_id = context_run.context_id.clone();
            context_conflicts.push(FamilyContextConflict {
                family_id: family_id.clone(),
                context_id: context_id.clone(),
                kind,
                certified: true,
                evidence: ConflictEvidence {
                    layer: RequiredNullable::Value(family_run.layer.clone()),
                    passage_key: RequiredNullable::Null,
                    detail: format!(
                        "fixed context copper: family={family_id}, context={context_id}, family_segment={}, context_segment={}, centerline_distance={:.17}, required_distance={:.17}, segment_pairs_analyzed={}",
                        pair_evidence.first_segment_index,
                        pair_evidence.second_segment_index,
                        pair_evidence.minimum_centerline_distance,
                        pair_evidence.required_centerline_distance,
                        pair_evidence.segment_pairs_analyzed,
                    ),
                },
            });
        }
    }
    context_conflicts.sort_by(|left, right| {
        (&left.family_id, &left.context_id).cmp(&(&right.family_id, &right.context_id))
    });
    Ok((conflicts, context_conflicts))
}

fn analyze_family_cross_product(
    old: &[CertifiedFamily],
    new: &[CertifiedFamily],
    work: &mut RouteFamilyProductionWork,
) -> Result<Vec<FamilyConflict>, (String, String)> {
    let old_points = old
        .iter()
        .map(|family| {
            family.family.runs[0]
                .witness
                .polyline
                .iter()
                .map(|point| Vec2::new(point.x, point.y))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let new_points = new
        .iter()
        .map(|family| {
            family.family.runs[0]
                .witness
                .polyline
                .iter()
                .map(|point| Vec2::new(point.x, point.y))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let old_preparation_started = Instant::now();
    let old_prepared = old
        .iter()
        .enumerate()
        .map(|(index, family)| {
            let run = &family.family.runs[0];
            prepare_fixed_copper_route(FixedCopperRoute {
                polyline: &old_points[index],
                width: run.witness.required_width,
                clearance: family.required_clearance,
            })
            .map_err(|error| {
                (
                    family.branch.clone(),
                    format!("fixed copper witness preparation failed closed: {error}"),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>();
    work.family_witness_preparations += old.len() as u64;
    work.copper_preparation_elapsed_micros += old_preparation_started.elapsed().as_micros() as u64;
    let old_prepared = old_prepared?;
    let new_preparation_started = Instant::now();
    let new_prepared = new
        .iter()
        .enumerate()
        .map(|(index, family)| {
            let run = &family.family.runs[0];
            prepare_fixed_copper_route(FixedCopperRoute {
                polyline: &new_points[index],
                width: run.witness.required_width,
                clearance: family.required_clearance,
            })
            .map_err(|error| {
                (
                    family.branch.clone(),
                    format!("fixed copper witness preparation failed closed: {error}"),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>();
    work.family_witness_preparations += new.len() as u64;
    work.copper_preparation_elapsed_micros += new_preparation_started.elapsed().as_micros() as u64;
    let new_prepared = new_prepared?;
    let mut conflicts = Vec::new();
    for (old_index, old_family) in old.iter().enumerate() {
        for (new_index, new_family) in new.iter().enumerate() {
            if old_family.electrical_net == new_family.electrical_net {
                continue;
            }
            work.family_pairs_analyzed += 1;
            let old_run = &old_family.family.runs[0];
            let new_run = &new_family.family.runs[0];
            if old_run.layer != new_run.layer {
                continue;
            }
            let (left_id, right_id, left_route, right_route) =
                if old_family.family.family_id <= new_family.family.family_id {
                    (
                        &old_family.family.family_id,
                        &new_family.family.family_id,
                        &old_prepared[old_index],
                        &new_prepared[new_index],
                    )
                } else {
                    (
                        &new_family.family.family_id,
                        &old_family.family.family_id,
                        &new_prepared[new_index],
                        &old_prepared[old_index],
                    )
                };
            let classification_started = Instant::now();
            let classification = classify_prepared_copper_pair(left_route, right_route);
            work.family_pair_classifications += 1;
            work.family_pair_classification_elapsed_micros +=
                classification_started.elapsed().as_micros() as u64;
            let classification = classification.map_err(|error| {
                (
                    format!("{} + {}", old_family.branch, new_family.branch),
                    format!("incremental copper-pair classification failed closed: {error}"),
                )
            })?;
            let pair_evidence = classification.evidence();
            work.segment_pairs_analyzed += pair_evidence.segment_pairs_analyzed as u64;
            let kind = match classification {
                FixedCopperPairClassification::Clear(_) => continue,
                FixedCopperPairClassification::CenterlineIntersection(_) => {
                    FamilyConflictKind::CenterlineCrossing
                }
                FixedCopperPairClassification::ClearanceShortfall(_) => {
                    FamilyConflictKind::CopperClearance
                }
            };
            conflicts.push(FamilyConflict {
                left_family_id: left_id.clone(),
                right_family_id: right_id.clone(),
                kind,
                certified: true,
                evidence: ConflictEvidence {
                    layer: RequiredNullable::Value(old_run.layer.clone()),
                    passage_key: RequiredNullable::Null,
                    detail: format!(
                        "fixed full-width witnesses: first_family={left_id}, second_family={right_id}, first_segment={}, second_segment={}, centerline_distance={:.17}, required_distance={:.17}, segment_pairs_analyzed={}",
                        pair_evidence.first_segment_index,
                        pair_evidence.second_segment_index,
                        pair_evidence.minimum_centerline_distance,
                        pair_evidence.required_centerline_distance,
                        pair_evidence.segment_pairs_analyzed,
                    ),
                },
            });
        }
    }
    conflicts.sort_by(|left, right| {
        (&left.left_family_id, &left.right_family_id)
            .cmp(&(&right.left_family_id, &right.right_family_id))
    });
    Ok(conflicts)
}

fn stable_family_id(
    route: &RouteFamilyRouteInput<'_>,
    embedding: &RouteCorridorEmbedding,
) -> String {
    let mut hash = Sha256::new();
    hash.update(b"layout-trace.route-family-id/v1\0");
    hash_field(&mut hash, route.electrical_net.as_bytes());
    hash_field(&mut hash, route.branch_id.as_bytes());
    hash_field(&mut hash, route.layer.as_bytes());
    for crossing in embedding.embedded_homotopy.crossings() {
        hash_field(&mut hash, crossing.obstacle.as_bytes());
        hash.update([match crossing.direction {
            CrossingDirection::Positive => 1,
            CrossingDirection::Negative => 2,
        }]);
    }
    let digest = hash.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    format!("family:{encoded}")
}

fn hash_field(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_le_bytes());
    hash.update(value);
}

fn cut_word_key(word: &crate::topology::HomotopyWord) -> Vec<(String, u8)> {
    word.crossings()
        .iter()
        .map(|crossing| {
            (
                crossing.obstacle.clone(),
                match crossing.direction {
                    CrossingDirection::Positive => 1,
                    CrossingDirection::Negative => 2,
                },
            )
        })
        .collect()
}

fn unsupported(
    route: Option<&str>,
    reason: RouteFamilyUnsupportedReason,
    detail: impl Into<String>,
    work: RouteFamilyProductionWork,
) -> RouteFamilyPortfolioBuildOutcome {
    RouteFamilyPortfolioBuildOutcome::Unsupported {
        route: route.map(str::to_owned),
        reason,
        detail: detail.into(),
        work,
    }
}

fn finite_point(point: Vec2) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

fn family_point(point: Vec2) -> FamilyPoint {
    FamilyPoint {
        x: point.x,
        y: point.y,
    }
}

fn polyline_length(points: &[Vec2]) -> f64 {
    points
        .windows(2)
        .map(|pair| {
            let dx = pair[1].x - pair[0].x;
            let dy = pair[1].y - pair[0].y;
            dx.hypot(dy)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corridor::{CorridorObstacle, build_corridor_graph};
    use crate::family_assignment::FamilyAssignmentOutcome;
    use crate::model::Rect;

    fn without_advisory_production_timing(
        mut work: RouteFamilyProductionWork,
    ) -> RouteFamilyProductionWork {
        work.portfolio_build_elapsed_micros = 0;
        work.alternative_search_elapsed_micros = 0;
        work.alternative_reembedding_elapsed_micros = 0;
        work.copper_preparation_elapsed_micros = 0;
        work.family_pair_classification_elapsed_micros = 0;
        work.family_context_classification_elapsed_micros = 0;
        work.portfolio_finalize_elapsed_micros = 0;
        work.assignment_elapsed_micros = 0;
        work
    }

    #[test]
    fn production_diagnostics_merge_across_repeated_stages() {
        let mut total = RouteFamilyProductionWork {
            portfolio_builds: 1,
            assignment_searches: 1,
            family_search_cache_lookups: 3,
            family_search_cache_hits: 1,
            family_search_new_executions: 2,
            visibility_states_reused: 7,
            visibility_states_new: 11,
            family_witness_preparations: 3,
            family_pair_classifications: 2,
            portfolio_build_elapsed_micros: 11,
            alternative_search_elapsed_micros: 7,
            assignment_elapsed_micros: 2,
            ..Default::default()
        };
        let mut addition = RouteFamilyProductionWork {
            portfolio_builds: 2,
            assignment_searches: 1,
            family_search_cache_lookups: 5,
            family_search_cache_hits: 2,
            family_search_resumes: 1,
            family_search_new_executions: 2,
            visibility_states_reused: 13,
            visibility_states_new: 17,
            family_witness_preparations: 5,
            family_pair_classifications: 4,
            portfolio_build_elapsed_micros: 13,
            alternative_search_elapsed_micros: 8,
            assignment_elapsed_micros: 3,
            ..Default::default()
        };
        merge_production_work(&mut total, &mut addition);
        assert_eq!(total.portfolio_builds, 3);
        assert_eq!(total.assignment_searches, 2);
        assert_eq!(total.family_search_cache_lookups, 8);
        assert_eq!(total.family_search_cache_hits, 3);
        assert_eq!(total.family_search_resumes, 1);
        assert_eq!(total.family_search_new_executions, 4);
        assert_eq!(total.visibility_states_reused, 20);
        assert_eq!(total.visibility_states_new, 28);
        assert_eq!(total.family_witness_preparations, 8);
        assert_eq!(total.family_pair_classifications, 6);
        assert_eq!(total.portfolio_build_elapsed_micros, 24);
        assert_eq!(total.alternative_search_elapsed_micros, 15);
        assert_eq!(total.assignment_elapsed_micros, 5);
    }

    #[test]
    fn exact_search_cache_replays_frontier_without_new_exploration() {
        let graph = build_corridor_graph("top", board(), &[], 2, 5).unwrap();
        let reference = [Vec2::new(1.0, 6.0), Vec2::new(19.0, 6.0)];
        let routes = [route("cached", "CACHE", &reference)];
        let corridors = [graph];
        let input = RouteFamilyPortfolioInput {
            portfolio_id: "cache-proof",
            placement_revision: 2,
            geometry_revision: 0,
            corridors: &corridors,
            routes: &routes,
            fixed_context: &[],
        };
        let mut cache = AlternativeSearchCache::new(&corridors);
        let mut first =
            build_route_family_portfolio_with_plan(input.clone(), INITIAL_BOUNDS, None, &mut cache);
        let first_work = build_outcome_work_mut(&mut first).clone();
        let mut replay =
            build_route_family_portfolio_with_plan(input, INITIAL_BOUNDS, None, &mut cache);
        let replay_work = build_outcome_work_mut(&mut replay);

        assert_eq!(first_work.family_search_cache_lookups, 1);
        assert_eq!(first_work.family_search_cache_hits, 0);
        assert_eq!(first_work.family_search_new_executions, 1);
        assert_eq!(replay_work.family_search_cache_lookups, 1);
        assert_eq!(replay_work.family_search_cache_hits, 1);
        assert_eq!(replay_work.family_search_new_executions, 0);
        assert_eq!(replay_work.family_search_resumes, 0);
        assert_eq!(replay_work.visibility_states_new, 0);
        assert_eq!(
            replay_work.visibility_states_reused,
            first_work.visibility_states_new
        );
        assert_eq!(replay_work.visibility_states, first_work.visibility_states);
        assert_eq!(replay_work.raw_frontiers, first_work.raw_frontiers);

        let request = request_for(&routes[0], &reference, None, false);
        let (bound_eight, _) = cache.key(&corridors[0], &request, 8);
        let (bound_sixteen, _) = cache.key(&corridors[0], &request, 16);
        assert_ne!(bound_eight, bound_sixteen);
    }

    fn board() -> Rect {
        Rect {
            min: Vec2::ZERO,
            max: Vec2::new(20.0, 12.0),
        }
    }

    fn route<'a>(
        branch_id: &'a str,
        net: &'a str,
        reference: &'a [Vec2],
    ) -> RouteFamilyRouteInput<'a> {
        RouteFamilyRouteInput {
            branch_id,
            electrical_net: net,
            layer: "top",
            physical_width: 0.4,
            clearance: 0.2,
            from_component: branch_id,
            to_component: branch_id,
            from: reference[0],
            from_toward: reference[1],
            to: *reference.last().unwrap(),
            to_toward: reference[reference.len() - 2],
            reference,
        }
    }

    fn planning_only_portfolio(branches: &[(&str, usize)]) -> RouteFamilyPortfolio {
        RouteFamilyPortfolio {
            schema_version: ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into(),
            feasibility_mode: Some(RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise),
            portfolio_id: "planning-only".into(),
            placement_revision: 1,
            geometry_revision: Some(1),
            corridor_epochs: Vec::new(),
            family_generation: FamilyGenerationCertificate {
                method: EXPANDED_FAMILY_GENERATION_METHOD.into(),
                max_families_per_variable: RAW_FAMILY_CANDIDATE_LIMIT,
                search_complete: true,
                budget_exhausted: false,
            },
            passage_certificates: Vec::new(),
            variables: branches
                .iter()
                .map(|(branch, count)| RouteFamilyVariable {
                    branch: (*branch).into(),
                    electrical_net: format!("net-{branch}"),
                    required_width: 0.4,
                    required_clearance: Some(0.2),
                    families: (0..*count)
                        .map(|ordinal| RouteFamily {
                            family_id: format!("{branch}/family-{ordinal}"),
                            placement_revision: 1,
                            complete_route: true,
                            cost: ordinal as f64,
                            runs: vec![RouteFamilyRun {
                                run_id: format!("{branch}/family-{ordinal}/run-0"),
                                run_index: 0,
                                layer: "top".into(),
                                start_point: 0,
                                end_point: 1,
                                corridor_revision: 1,
                                transition_from_previous: RequiredNullable::Null,
                                witness: RouteWitness {
                                    polyline: vec![
                                        FamilyPoint { x: 0.0, y: 0.0 },
                                        FamilyPoint { x: 1.0, y: 0.0 },
                                    ],
                                    placement_revision: 1,
                                    realizable: true,
                                    clearance_certified: true,
                                    clearance_model: "planning-test".into(),
                                    minimum_clearance: 1.0,
                                    required_width: 0.4,
                                },
                                cut_word_certificate: CutWordCertificate {
                                    cut_basis_fingerprint: "planning-cuts".into(),
                                    basis_complete: true,
                                    verified: true,
                                    word: Vec::new(),
                                },
                                semantic_passage_keys: Vec::new(),
                                passage_mapping_complete: true,
                            }],
                        })
                        .collect(),
                })
                .collect(),
            fixed_context: None,
            pair_analysis: PairAnalysisCertificate {
                method: FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into(),
                placement_revision: 1,
                corridor_epoch_fingerprint: String::new(),
                candidate_set_fingerprint: String::new(),
                search_complete: true,
                analyzed_family_pairs: Some(0),
                fixed_context_fingerprint: None,
                analyzed_family_context_pairs: None,
                context_conflicts: Vec::new(),
                conflicts: Vec::new(),
            },
        }
    }

    fn bounded_assignment(implicated_variables: &[&str]) -> FamilyAssignmentOutcome {
        FamilyAssignmentOutcome::BoundedInfeasible {
            work: FamilyAssignmentWork::default(),
            evidence: crate::family_assignment::FamilyAssignmentBoundedInfeasibilityEvidence {
                contract: "test-bounded-evidence".into(),
                max_families_per_variable: RAW_FAMILY_CANDIDATE_LIMIT,
                admitted_domains: Vec::new(),
                exhaustive_within_admitted_portfolio: true,
                candidate_set_fingerprint: "candidate-set".into(),
                portfolio_context_sha256: "context".into(),
                all_fixed_context_sha256: None,
                fixed_context_dependency_sha256: None,
                conflict_context_runs: Vec::new(),
                implicated_variables: implicated_variables
                    .iter()
                    .map(|branch| (*branch).to_owned())
                    .collect(),
                empty_domain_events: 1,
                witnesses: Vec::new(),
                witnesses_truncated: 0,
                canonical_sha256: "canonical".into(),
            },
        }
    }

    fn full_raw8_frontier(branch: &str) -> RouteFamilyRawFrontierEvidence {
        RouteFamilyRawFrontierEvidence {
            branch: branch.into(),
            requested_raw_bound: RAW_FAMILY_CANDIDATE_LIMIT,
            returned_raw_candidates: RAW_FAMILY_CANDIDATE_LIMIT,
            termination: AlternativeSearchTermination::FamilyLimitReached,
            admitted_family_count: RAW_FAMILY_CANDIDATE_LIMIT,
        }
    }

    fn planning_only_input<'a>(
        fixed_context: &'a [RouteFamilyFixedContextInput],
    ) -> RouteFamilyPortfolioInput<'a> {
        RouteFamilyPortfolioInput {
            portfolio_id: "planning-only",
            placement_revision: 1,
            geometry_revision: 1,
            corridors: &[],
            routes: &[],
            fixed_context,
        }
    }

    #[test]
    fn targeted_deepening_admits_a_third_variable_when_all_work_budgets_fit() {
        let portfolio = planning_only_portfolio(&[("z", 8), ("x", 8), ("c", 8), ("a", 8)]);
        let assignment = bounded_assignment(&["z", "x", "c", "a", "a"]);
        let input = planning_only_input(&[]);
        let mut x = full_raw8_frontier("x");
        x.termination = AlternativeSearchTermination::SearchExhausted;
        let frontiers = [
            full_raw8_frontier("z"),
            x,
            full_raw8_frontier("c"),
            full_raw8_frontier("a"),
        ];

        let evidence =
            targeted_deepening_evidence(&assignment, &input, &portfolio, &frontiers).unwrap();
        assert_eq!(evidence.implicated_variables, ["a", "c", "x", "z"]);
        assert_eq!(evidence.eligible_variables, ["a", "c", "z"]);
        assert_eq!(evidence.selected_variables, ["a", "c", "z"]);
        assert!(evidence.deferred_variables.is_empty());
        assert_eq!(evidence.target_variable_limit, 8);
        assert_eq!(evidence.new_family_limit, 32);
        assert_eq!(evidence.projected_new_families, 24);
        assert_eq!(evidence.projected_family_pairs, 1_152);
        assert_eq!(evidence.projected_family_context_pairs, 0);
        assert!(evidence.exhaustions.is_empty());

        let reversed_assignment = bounded_assignment(&["a", "c", "x", "z"]);
        let reversed_frontiers = frontiers.iter().rev().cloned().collect::<Vec<_>>();
        let reversed = targeted_deepening_evidence(
            &reversed_assignment,
            &input,
            &portfolio,
            &reversed_frontiers,
        )
        .unwrap();
        assert_eq!(evidence, reversed);
    }

    #[test]
    fn targeted_deepening_defers_variables_that_would_overflow_projected_pair_work() {
        let portfolio = planning_only_portfolio(&[("a", 8), ("b", 8), ("c", 8)]);
        let input = planning_only_input(&[]);
        let admission = admit_targeted_variables(
            &input,
            &portfolio,
            &["a".into(), "b".into(), "c".into()],
            TargetedAdmissionLimits {
                variable_safety: 8,
                new_families: 32,
                family_pairs: 400,
                family_context_pairs: u64::MAX,
            },
        );

        assert_eq!(admission.selected_variables, ["a"]);
        assert_eq!(admission.deferred_variables, ["b", "c"]);
        assert_eq!(admission.projected_new_families, 8);
        assert_eq!(admission.projected_family_pairs, 320);
        assert_eq!(
            admission.exhaustions,
            [RouteFamilyTargetedDeepeningExhaustion::FamilyPairWorkLimitReached]
        );
    }

    #[test]
    fn targeted_deepening_does_not_expand_partial_or_exhausted_frontiers() {
        let portfolio = planning_only_portfolio(&[("partial", 7), ("exhausted", 8)]);
        let assignment = bounded_assignment(&["partial", "exhausted"]);
        let input = planning_only_input(&[]);
        let mut partial = full_raw8_frontier("partial");
        partial.admitted_family_count = 7;
        let mut exhausted = full_raw8_frontier("exhausted");
        exhausted.termination = AlternativeSearchTermination::SearchExhausted;

        let evidence =
            targeted_deepening_evidence(&assignment, &input, &portfolio, &[partial, exhausted])
                .unwrap();
        assert!(evidence.eligible_variables.is_empty());
        assert!(evidence.selected_variables.is_empty());
        assert_eq!(
            evidence.exhaustions,
            [RouteFamilyTargetedDeepeningExhaustion::NoEligibleImplicatedFrontier]
        );
    }

    #[test]
    fn parent_frontier_binding_is_order_independent_and_fails_closed() {
        let portfolio = planning_only_portfolio(&[("a", 8)]);
        let mut shallow = full_raw8_frontier("a");
        shallow.admitted_family_count = 3;
        let complete = full_raw8_frontier("a");
        assert_eq!(
            canonical_parent_raw_frontiers(&portfolio, &[shallow.clone(), complete.clone()])
                .unwrap(),
            canonical_parent_raw_frontiers(&portfolio, &[complete.clone(), shallow]).unwrap()
        );
        assert!(canonical_parent_raw_frontiers(&portfolio, &[]).is_err());

        let mut ambiguous = complete.clone();
        ambiguous.termination = AlternativeSearchTermination::SearchExhausted;
        assert!(
            canonical_parent_raw_frontiers(&portfolio, &[complete.clone(), ambiguous]).is_err()
        );
        let mut unknown = complete;
        unknown.branch = "unknown".into();
        assert!(canonical_parent_raw_frontiers(&portfolio, &[unknown]).is_err());
    }

    #[test]
    fn targeted_identity_reconciliation_retains_exact_prior_certificate() {
        let prior = planning_only_portfolio(&[("route", 1)]).variables[0].families[0].clone();
        assert_eq!(
            reuse_exact_prior_family(Some(&prior), &prior).unwrap(),
            prior
        );

        let mut changed = prior.clone();
        changed.cost += 1.0;
        assert_eq!(
            reuse_exact_prior_family(Some(&prior), &changed).unwrap(),
            prior
        );
        changed.family_id.push_str("-different-class");
        assert!(reuse_exact_prior_family(Some(&prior), &changed).is_err());
        let mut changed_cut_epoch = prior.clone();
        changed_cut_epoch.runs[0]
            .cut_word_certificate
            .cut_basis_fingerprint
            .push_str("-changed");
        assert!(reuse_exact_prior_family(Some(&prior), &changed_cut_epoch).is_err());
        assert!(reuse_exact_prior_family(None, &prior).is_err());
    }

    #[test]
    fn targeted_frontier_reconciliation_accepts_only_complete_ordered_prior_subsequence() {
        let prior = planning_only_portfolio(&[("route", 3)]).variables[0]
            .families
            .clone();
        let mut new_a = prior[0].clone();
        new_a.family_id = "route/family-new-a".into();
        let mut new_b = prior[0].clone();
        new_b.family_id = "route/family-new-b".into();
        let interleaved = vec![
            prior[0].clone(),
            new_a.clone(),
            prior[1].clone(),
            new_b.clone(),
            prior[2].clone(),
        ];
        let reconciled = reconcile_prior_family_frontier(&prior, &interleaved).unwrap();
        assert_eq!(
            reconciled.retained_prior_family_ids,
            prior
                .iter()
                .map(|family| family.family_id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            reconciled.observed_new_family_ids,
            ["route/family-new-a", "route/family-new-b"]
        );
        assert_eq!(
            reconciled
                .families
                .iter()
                .map(|family| family.family_id.as_str())
                .collect::<Vec<_>>(),
            [
                "route/family-0",
                "route/family-1",
                "route/family-2",
                "route/family-new-a",
                "route/family-new-b",
            ]
        );

        assert!(
            reconcile_prior_family_frontier(&prior, &interleaved[..4]).is_err(),
            "missing prior identity must fail closed"
        );
        assert!(
            reconcile_prior_family_frontier(
                &prior,
                &[
                    prior[0].clone(),
                    prior[1].clone(),
                    prior[1].clone(),
                    prior[2].clone(),
                ],
            )
            .is_err(),
            "duplicate prior identity must fail closed"
        );
        assert!(
            reconcile_prior_family_frontier(
                &prior,
                &[prior[1].clone(), prior[0].clone(), prior[2].clone()],
            )
            .is_err(),
            "reordered prior identities must fail closed"
        );
    }

    #[test]
    fn targeted_pair_work_is_preflighted_before_any_deeper_search() {
        let portfolio = planning_only_portfolio(&[("a", 8), ("b", 8)]);
        let context = [
            RouteFamilyFixedContextInput {
                context_id: "outside-1".into(),
                branch: "outside-1".into(),
                electrical_net: "outside".into(),
                layer: "top".into(),
                physical_width: 0.4,
                clearance: 0.2,
                points: vec![Vec2::ZERO],
            },
            RouteFamilyFixedContextInput {
                context_id: "outside-2".into(),
                branch: "outside-2".into(),
                electrical_net: "outside".into(),
                layer: "top".into(),
                physical_width: 0.4,
                clearance: 0.2,
                points: vec![Vec2::ZERO],
            },
        ];
        let input = RouteFamilyPortfolioInput {
            portfolio_id: "planning-only",
            placement_revision: 1,
            geometry_revision: 1,
            corridors: &[],
            routes: &[],
            fixed_context: &context,
        };
        let targeted = BTreeSet::from(["a".to_owned()]);

        let (family_pairs, context_pairs) =
            targeted_pair_work_upper_bounds(&input, &portfolio, &targeted);
        assert_eq!(family_pairs, 16 * 8);
        assert_eq!(context_pairs, (16 + 8) * 2);
        assert!(family_pairs <= MAX_TARGETED_FAMILY_PAIR_ANALYSES);
        assert!(context_pairs <= MAX_TARGETED_FAMILY_CONTEXT_PAIR_ANALYSES);
    }

    #[test]
    fn exhausted_single_class_frontier_is_complete_without_padding_to_k3() {
        let graph = build_corridor_graph("top", board(), &[], 7, 11).unwrap();
        let reference = [Vec2::new(1.0, 3.0), Vec2::new(19.0, 3.0)];
        let routes = [route("a", "A", &reference)];
        let corridors = [graph];
        let outcome = build_and_select_route_families(
            RouteFamilyPortfolioInput {
                portfolio_id: "simple",
                placement_revision: 7,
                geometry_revision: 0,
                corridors: &corridors,
                routes: &routes,
                fixed_context: &[],
            },
            FamilyAssignmentLimits::default(),
        );
        let RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work: work,
            assignment,
        } = outcome
        else {
            panic!("an exhausted one-class frontier is complete, got {outcome:?}");
        };
        assert_eq!(portfolio.variables.len(), 1);
        assert_eq!(portfolio.variables[0].families.len(), 1);
        assert!(matches!(
            assignment,
            FamilyAssignmentOutcome::Selected { .. }
        ));
        assert_eq!(work.raw_candidates_examined, 1);
        assert_eq!(work.families_certified, 1);
        assert_eq!(work.certifiable_frontiers_completed, 1);
        assert_eq!(work.raw_frontiers.len(), 1);
        assert_eq!(work.raw_frontiers[0].requested_raw_bound, 8);
        assert_eq!(work.raw_frontiers[0].returned_raw_candidates, 1);
        assert_eq!(
            work.raw_frontiers[0].termination,
            AlternativeSearchTermination::SearchExhausted
        );
        assert_eq!(work.raw_frontiers[0].admitted_family_count, 1);
        assert_eq!(work.planning_stages.len(), 1);
        assert_eq!(
            work.planning_stages[0].stage,
            RouteFamilyPlanningStageKind::InitialK3
        );
        assert_eq!(
            work.planning_stages[0].outcome,
            RouteFamilyPlanningStageOutcome::Selected
        );
    }

    #[test]
    fn exhaustive_single_class_domains_can_prove_bounded_infeasibility() {
        let graph = build_corridor_graph("top", board(), &[], 0, 1).unwrap();
        let horizontal = [Vec2::new(1.0, 6.0), Vec2::new(19.0, 6.0)];
        let vertical = [Vec2::new(10.0, 1.0), Vec2::new(10.0, 11.0)];
        let routes = [
            route("horizontal", "H", &horizontal),
            route("vertical", "V", &vertical),
        ];
        let corridors = [graph];
        let outcome = build_and_select_route_families(
            RouteFamilyPortfolioInput {
                portfolio_id: "crossing",
                placement_revision: 0,
                geometry_revision: 0,
                corridors: &corridors,
                routes: &routes,
                fixed_context: &[],
            },
            FamilyAssignmentLimits::default(),
        );
        let RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work: work,
            assignment: FamilyAssignmentOutcome::BoundedInfeasible { evidence, .. },
        } = outcome
        else {
            panic!("the exhausted crossing domains must be bounded-infeasible, got {outcome:?}");
        };
        assert_eq!(portfolio.variables.len(), 2);
        assert!(
            portfolio
                .variables
                .iter()
                .all(|variable| variable.families.len() == 1)
        );
        assert!(evidence.exhaustive_within_admitted_portfolio);
        assert_eq!(evidence.implicated_variables, ["horizontal", "vertical"]);
        assert_eq!(work.family_pairs_analyzed, 2);
        assert_eq!(work.segment_pairs_analyzed, 2);
    }

    #[test]
    fn route_input_order_does_not_change_exhausted_short_frontiers() {
        let graph = build_corridor_graph("top", board(), &[], 2, 5).unwrap();
        let low = [Vec2::new(1.0, 2.0), Vec2::new(19.0, 2.0)];
        let high = [Vec2::new(1.0, 10.0), Vec2::new(19.0, 10.0)];
        let forward = [route("low", "LOW", &low), route("high", "HIGH", &high)];
        let reverse = [route("high", "HIGH", &high), route("low", "LOW", &low)];
        let corridors = [graph];
        let build = |routes: &[RouteFamilyRouteInput<'_>]| {
            build_route_family_portfolio(RouteFamilyPortfolioInput {
                portfolio_id: "stable",
                placement_revision: 2,
                geometry_revision: 0,
                corridors: &corridors,
                routes,
                fixed_context: &[],
            })
        };
        let evidence = |outcome| {
            let RouteFamilyPortfolioBuildOutcome::Complete { portfolio, work } = outcome else {
                panic!("expected complete exhausted frontiers, got {outcome:?}");
            };
            (portfolio, without_advisory_production_timing(work))
        };
        assert_eq!(evidence(build(&forward)), evidence(build(&reverse)));
    }

    #[test]
    fn same_net_branches_accept_independently_exhausted_short_frontiers() {
        let graph = build_corridor_graph("top", board(), &[], 0, 1).unwrap();
        let low = [Vec2::new(1.0, 2.0), Vec2::new(19.0, 2.0)];
        let high = [Vec2::new(1.0, 10.0), Vec2::new(19.0, 10.0)];
        let routes = [route("one", "TREE", &low), route("two", "TREE", &high)];
        let corridors = [graph];
        let outcome = build_route_family_portfolio(RouteFamilyPortfolioInput {
            portfolio_id: "tree",
            placement_revision: 0,
            geometry_revision: 0,
            corridors: &corridors,
            routes: &routes,
            fixed_context: &[],
        });
        let RouteFamilyPortfolioBuildOutcome::Complete { portfolio, work } = outcome else {
            panic!("both exhausted one-class frontiers are complete, got {outcome:?}");
        };
        assert_eq!(portfolio.variables.len(), 2);
        assert!(
            portfolio
                .variables
                .iter()
                .all(|variable| variable.families.len() == 1)
        );
        assert_eq!(work.family_searches, 2);
        assert_eq!(work.family_pairs_analyzed, 0);
    }

    #[test]
    fn obstacle_frontier_reembeds_every_emitted_class_at_full_width() {
        let obstacle = CorridorObstacle {
            id: "middle".into(),
            polygon: vec![
                Vec2::new(8.0, 3.0),
                Vec2::new(12.0, 3.0),
                Vec2::new(12.0, 9.0),
                Vec2::new(8.0, 9.0),
            ],
        };
        let graph = build_corridor_graph("top", board(), &[obstacle], 4, 9).unwrap();
        let reference = [Vec2::new(1.0, 6.0), Vec2::new(19.0, 6.0)];
        let routes = [route("around", "AROUND", &reference)];
        let corridors = [graph];
        let outcome = build_route_family_portfolio(RouteFamilyPortfolioInput {
            portfolio_id: "obstacle",
            placement_revision: 4,
            geometry_revision: 0,
            corridors: &corridors,
            routes: &routes,
            fixed_context: &[],
        });
        let RouteFamilyPortfolioBuildOutcome::Complete { portfolio, work } = outcome else {
            panic!("expected obstacle alternatives, got {outcome:?}");
        };
        assert!(!portfolio.variables[0].families.is_empty());
        assert_eq!(
            work.alternatives_reembedded,
            portfolio.variables[0].families.len() as u64
        );
        assert!(portfolio.variables[0].families.iter().all(|family| {
            let witness = &family.runs[0].witness;
            witness.clearance_certified && witness.required_width == 0.4
        }));
    }

    #[test]
    fn generated_family_uses_its_own_terminal_sectors_for_corridor_projection() {
        let obstacle = CorridorObstacle {
            id: "middle".into(),
            polygon: vec![
                Vec2::new(8.0, 3.0),
                Vec2::new(12.0, 3.0),
                Vec2::new(12.0, 9.0),
                Vec2::new(8.0, 9.0),
            ],
        };
        let graph = build_corridor_graph("top", board(), &[obstacle], 5, 10).unwrap();
        // The input leaves both terminals toward the lower corridor. The
        // shortest alternative is the straight upper route and therefore has
        // different terminal sectors. Reusing the input directions used to
        // select unrelated terminal cells and reject that valid family during
        // raw-to-semantic corridor projection.
        let reference = [
            Vec2::new(1.0, 10.0),
            Vec2::new(7.0, 2.0),
            Vec2::new(13.0, 2.0),
            Vec2::new(19.0, 10.0),
        ];
        let routes = [route("sector-change", "SECTOR_CHANGE", &reference)];
        let corridors = [graph];
        let outcome = build_route_family_portfolio(RouteFamilyPortfolioInput {
            portfolio_id: "terminal-sector-change",
            placement_revision: 5,
            geometry_revision: 0,
            corridors: &corridors,
            routes: &routes,
            fixed_context: &[],
        });
        let RouteFamilyPortfolioBuildOutcome::Complete { portfolio, work } = outcome else {
            panic!("expected both terminal-sector alternatives to certify, got {outcome:?}");
        };
        assert!(portfolio.variables[0].families.len() >= 2);
        assert_eq!(
            work.alternatives_reembedded,
            portfolio.variables[0].families.len() as u64
        );
    }

    #[test]
    fn v2_tree_branches_get_independent_exact_choices_and_complete_cross_net_analysis() {
        let make_family = |branch: &str,
                           electrical_net: &str,
                           ordinal: usize,
                           cost: f64,
                           points: [Vec2; 2],
                           passages: &[&str]| {
            let family_id = format!("{branch}/family-{ordinal}");
            CertifiedFamily {
                branch: branch.into(),
                electrical_net: electrical_net.into(),
                required_clearance: 0.2,
                family: RouteFamily {
                    family_id: family_id.clone(),
                    placement_revision: 7,
                    complete_route: true,
                    cost,
                    runs: vec![RouteFamilyRun {
                        run_id: format!("{family_id}/run-0"),
                        run_index: 0,
                        layer: "top".into(),
                        start_point: 0,
                        end_point: 1,
                        corridor_revision: 11,
                        transition_from_previous: RequiredNullable::Null,
                        witness: RouteWitness {
                            polyline: points.into_iter().map(family_point).collect(),
                            placement_revision: 7,
                            realizable: true,
                            clearance_certified: true,
                            clearance_model: "synthetic-exact-v1".into(),
                            minimum_clearance: 0.2,
                            required_width: 0.4,
                        },
                        cut_word_certificate: CutWordCertificate {
                            cut_basis_fingerprint: "cuts-v1".into(),
                            basis_complete: true,
                            verified: true,
                            word: Vec::new(),
                        },
                        semantic_passage_keys: passages
                            .iter()
                            .map(|passage| (*passage).to_owned())
                            .collect(),
                        passage_mapping_complete: true,
                    }],
                },
            }
        };
        let certified = vec![
            make_family(
                "vcc-a",
                "VCC",
                0,
                0.0,
                [Vec2::new(1.0, 2.0), Vec2::new(19.0, 2.0)],
                &["shared"],
            ),
            make_family(
                "vcc-b",
                "VCC",
                0,
                0.0,
                [Vec2::new(1.0, 4.0), Vec2::new(19.0, 4.0)],
                &["shared"],
            ),
            make_family(
                "signal",
                "SIGNAL",
                0,
                0.0,
                [Vec2::new(10.0, 1.0), Vec2::new(10.0, 5.0)],
                &[],
            ),
            make_family(
                "signal",
                "SIGNAL",
                1,
                1.0,
                [Vec2::new(1.0, 10.0), Vec2::new(19.0, 10.0)],
                &[],
            ),
        ];
        let mut production_work = RouteFamilyProductionWork::default();
        let conflicts = analyze_pairs(&certified, &mut production_work).unwrap();

        // There are two same-electrical VCC branches and two SIGNAL options:
        // only the 2 * 2 cross-electrical family pairs belong to the matrix.
        assert_eq!(production_work.family_pairs_analyzed, 4);
        assert_eq!(production_work.segment_pairs_analyzed, 4);
        assert_eq!(conflicts.len(), 2);
        assert!(conflicts.iter().all(|conflict| {
            conflict.left_family_id.starts_with("signal/")
                || conflict.right_family_id.starts_with("signal/")
        }));

        let mut portfolio = RouteFamilyPortfolio {
            schema_version: crate::family_ir::LEGACY_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into(),
            feasibility_mode: None,
            portfolio_id: "synthetic-v2-tree".into(),
            placement_revision: 7,
            geometry_revision: None,
            corridor_epochs: vec![CorridorEpoch {
                layer: "top".into(),
                corridor_revision: 11,
                semantic_fingerprint: "semantic-v1".into(),
                cut_basis_fingerprint: "cuts-v1".into(),
                cut_basis_complete: true,
            }],
            family_generation: FamilyGenerationCertificate {
                method: "synthetic-complete-v2".into(),
                max_families_per_variable: 2,
                search_complete: true,
                budget_exhausted: false,
            },
            passage_certificates: vec![PassageCapacityCertificate {
                layer: "top".into(),
                key: "shared".into(),
                placement_revision: 7,
                corridor_revision: 11,
                available_width: Some(0.8),
                minimum_clearance: 0.2,
                enforcement: None,
                capacity_model: "exclusive_exact_full_width_witness/conservative_per_branch_sum/v2"
                    .into(),
                certified: true,
            }],
            variables: ["vcc-a", "vcc-b", "signal"]
                .into_iter()
                .map(|branch| {
                    let families = certified
                        .iter()
                        .filter(|item| item.branch == branch)
                        .map(|item| item.family.clone())
                        .collect::<Vec<_>>();
                    RouteFamilyVariable {
                        branch: branch.into(),
                        electrical_net: if branch.starts_with("vcc") {
                            "VCC".into()
                        } else {
                            "SIGNAL".into()
                        },
                        required_width: 0.4,
                        required_clearance: None,
                        families,
                    }
                })
                .collect(),
            fixed_context: None,
            pair_analysis: PairAnalysisCertificate {
                method: "exhaustive_same_layer_segment_pairs/v1".into(),
                placement_revision: 7,
                corridor_epoch_fingerprint: String::new(),
                candidate_set_fingerprint: String::new(),
                search_complete: true,
                analyzed_family_pairs: None,
                fixed_context_fingerprint: None,
                analyzed_family_context_pairs: None,
                context_conflicts: vec![],
                conflicts,
            },
        };
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        portfolio.validate().unwrap();

        let FamilyAssignmentOutcome::Selected { selection, .. } =
            select_route_families(&portfolio, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("the synthetic complete V2 portfolio must be exactly assignable");
        };
        assert_eq!(selection.choices.len(), 3);
        assert_eq!(selection.choices["vcc-a"], "vcc-a/family-0");
        assert_eq!(selection.choices["vcc-b"], "vcc-b/family-0");
        assert_eq!(selection.choices["signal"], "signal/family-1");
        assert_eq!(selection.passage_usage[0].used_width, 0.8);
        assert_eq!(selection.passage_usage[0].remaining_width, 0.0);
    }

    #[test]
    fn fixed_witness_pairs_admit_clear_shared_passages_and_reject_positive_shortfalls() {
        let make = |branch: &str, net: &str, y: f64| {
            let points = [Vec2::new(0.0, y), Vec2::new(2.0, y)];
            let family_id = format!("{branch}/family-0");
            CertifiedFamily {
                branch: branch.into(),
                electrical_net: net.into(),
                required_clearance: 0.2,
                family: RouteFamily {
                    family_id: family_id.clone(),
                    placement_revision: 7,
                    complete_route: true,
                    cost: 2.0,
                    runs: vec![RouteFamilyRun {
                        run_id: format!("{family_id}/run-0"),
                        run_index: 0,
                        layer: "top".into(),
                        start_point: 0,
                        end_point: 1,
                        corridor_revision: 11,
                        transition_from_previous: RequiredNullable::Null,
                        witness: RouteWitness {
                            polyline: points.into_iter().map(family_point).collect(),
                            placement_revision: 7,
                            realizable: true,
                            clearance_certified: true,
                            clearance_model: "synthetic-fixed-witness".into(),
                            minimum_clearance: 0.2,
                            required_width: 0.4,
                        },
                        cut_word_certificate: CutWordCertificate {
                            cut_basis_fingerprint: "cuts-v1".into(),
                            basis_complete: true,
                            verified: true,
                            word: Vec::new(),
                        },
                        semantic_passage_keys: vec!["shared".into()],
                        passage_mapping_complete: true,
                    }],
                },
            }
        };

        let first = make("a", "A", 0.0);
        let exactly_clear = make("b", "B", 0.6);
        let mut work = RouteFamilyProductionWork::default();
        assert!(
            analyze_pairs(&[first.clone(), exactly_clear], &mut work)
                .unwrap()
                .is_empty()
        );
        assert_eq!(work.family_pairs_analyzed, 1);
        assert_eq!(work.segment_pairs_analyzed, 1);

        let too_close = make("c", "C", 0.59);
        let mut work = RouteFamilyProductionWork::default();
        let conflicts = analyze_pairs(&[first, too_close], &mut work).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, FamilyConflictKind::CopperClearance);
        assert!(conflicts[0].evidence.detail.contains("required_distance="));

        let lexically_late = make("z", "Z", 0.0);
        let lexically_early = make("a", "A", 0.0);
        let mut delta_work = RouteFamilyProductionWork::default();
        let delta = analyze_family_cross_product(
            &[lexically_late.clone()],
            &[lexically_early.clone()],
            &mut delta_work,
        )
        .unwrap();
        let mut full_work = RouteFamilyProductionWork::default();
        let full = analyze_pairs(&[lexically_early, lexically_late], &mut full_work).unwrap();
        assert_eq!(
            delta, full,
            "delta evidence must use canonical family order"
        );
    }

    #[test]
    fn fixed_context_analysis_counts_cross_net_layers_and_classifies_point_annuli() {
        let family_id = "branch/family-0".to_owned();
        let family = CertifiedFamily {
            branch: "branch".into(),
            electrical_net: "SIGNAL".into(),
            required_clearance: 0.2,
            family: RouteFamily {
                family_id: family_id.clone(),
                placement_revision: 7,
                complete_route: true,
                cost: 0.0,
                runs: vec![RouteFamilyRun {
                    run_id: format!("{family_id}/run-0"),
                    run_index: 0,
                    layer: "top".into(),
                    start_point: 0,
                    end_point: 1,
                    corridor_revision: 11,
                    transition_from_previous: RequiredNullable::Null,
                    witness: RouteWitness {
                        polyline: [Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.0)]
                            .into_iter()
                            .map(family_point)
                            .collect(),
                        placement_revision: 7,
                        realizable: true,
                        clearance_certified: true,
                        clearance_model: "synthetic-fixed-witness".into(),
                        minimum_clearance: 0.2,
                        required_width: 0.4,
                    },
                    cut_word_certificate: CutWordCertificate {
                        cut_basis_fingerprint: "cuts-v1".into(),
                        basis_complete: true,
                        verified: true,
                        word: Vec::new(),
                    },
                    semantic_passage_keys: Vec::new(),
                    passage_mapping_complete: true,
                }],
            },
        };
        let context = vec![
            RouteFamilyFixedContextInput {
                context_id: "same-net".into(),
                branch: "outside-same".into(),
                electrical_net: "SIGNAL".into(),
                layer: "top".into(),
                physical_width: 0.7,
                clearance: 0.2,
                points: vec![Vec2::new(1.0, 0.0)],
            },
            RouteFamilyFixedContextInput {
                context_id: "other-layer".into(),
                branch: "outside-bottom".into(),
                electrical_net: "GROUND".into(),
                layer: "bottom".into(),
                physical_width: 0.7,
                clearance: 0.2,
                points: vec![Vec2::new(1.0, 0.0)],
            },
            RouteFamilyFixedContextInput {
                context_id: "via-annulus".into(),
                branch: "outside-via".into(),
                electrical_net: "GROUND".into(),
                layer: "top".into(),
                physical_width: 0.7,
                clearance: 0.2,
                points: vec![Vec2::new(1.0, 0.0)],
            },
        ];
        let mut work = RouteFamilyProductionWork::default();
        let (pair_conflicts, context_conflicts) =
            analyze_pairs_and_context(&[family], &context, &mut work).unwrap();

        assert!(pair_conflicts.is_empty());
        assert_eq!(work.family_context_pairs_analyzed, 2);
        assert_eq!(work.context_segment_pairs_analyzed, 1);
        assert_eq!(context_conflicts.len(), 1);
        assert_eq!(context_conflicts[0].context_id, "via-annulus");
        assert_eq!(
            context_conflicts[0].kind,
            FamilyConflictKind::CenterlineCrossing
        );
    }

    #[test]
    fn producer_serializes_corridor_epochs_for_cross_layer_fixed_context() {
        let obstacle = CorridorObstacle {
            id: "middle".into(),
            polygon: vec![
                Vec2::new(8.0, 3.0),
                Vec2::new(12.0, 3.0),
                Vec2::new(12.0, 9.0),
                Vec2::new(8.0, 9.0),
            ],
        };
        let bottom = build_corridor_graph("bottom", board(), &[obstacle.clone()], 4, 9).unwrap();
        let top = build_corridor_graph("top", board(), &[obstacle], 4, 9).unwrap();
        let points = [Vec2::new(1.0, 6.0), Vec2::new(19.0, 6.0)];
        let mut bottom_route = route("selected-bottom", "SELECTED", &points);
        bottom_route.layer = "bottom";
        let routes = [bottom_route];
        let context = [RouteFamilyFixedContextInput {
            context_id: "fixed-top".into(),
            branch: "fixed-top".into(),
            electrical_net: "FIXED".into(),
            layer: "top".into(),
            physical_width: 0.4,
            clearance: 0.2,
            points: vec![Vec2::new(2.0, 2.0), Vec2::new(18.0, 2.0)],
        }];
        let corridors = [bottom, top];
        let RouteFamilyPortfolioBuildOutcome::Complete { portfolio, .. } =
            build_route_family_portfolio(RouteFamilyPortfolioInput {
                portfolio_id: "cross-layer-context",
                placement_revision: 4,
                geometry_revision: 2,
                corridors: &corridors,
                routes: &routes,
                fixed_context: &context,
            })
        else {
            panic!("cross-layer fixed context must produce a valid portfolio")
        };
        assert_eq!(
            portfolio
                .corridor_epochs
                .iter()
                .map(|epoch| epoch.layer.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["bottom", "top"])
        );
        portfolio.validate().unwrap();
    }

    fn incremental_promotion_fixture() -> (
        CorridorGraph,
        RouteFamilyPortfolio,
        Vec<RouteFamilyRawFrontierEvidence>,
        [Vec2; 2],
        [Vec2; 2],
        RouteFamilyFixedContextInput,
    ) {
        let obstacle = CorridorObstacle {
            id: "middle".into(),
            polygon: vec![
                Vec2::new(8.0, 3.0),
                Vec2::new(12.0, 3.0),
                Vec2::new(12.0, 9.0),
                Vec2::new(8.0, 9.0),
            ],
        };
        let graph = build_corridor_graph("top", board(), &[obstacle], 4, 9).unwrap();
        let old_points = [Vec2::new(1.0, 6.0), Vec2::new(19.0, 6.0)];
        let promoted_points = [Vec2::new(1.0, 5.0), Vec2::new(19.0, 5.0)];
        let promoted_context = RouteFamilyFixedContextInput {
            context_id: "promoted".into(),
            branch: "promoted".into(),
            electrical_net: "PROMOTED".into(),
            layer: "top".into(),
            physical_width: 0.4,
            clearance: 0.2,
            points: promoted_points.to_vec(),
        };
        let retained_context = RouteFamilyFixedContextInput {
            context_id: "retained".into(),
            branch: "retained".into(),
            electrical_net: "RETAINED".into(),
            layer: "top".into(),
            physical_width: 0.4,
            clearance: 0.2,
            points: vec![Vec2::new(4.0, 1.0), Vec2::new(4.0, 11.0)],
        };
        let old_routes = [route("old", "OLD", &old_points)];
        let corridors = [graph.clone()];
        let RouteFamilyPortfolioBuildOutcome::Complete { portfolio, work } =
            build_route_family_portfolio(RouteFamilyPortfolioInput {
                portfolio_id: "parent",
                placement_revision: 4,
                geometry_revision: 2,
                corridors: &corridors,
                routes: &old_routes,
                fixed_context: &[promoted_context, retained_context.clone()],
            })
        else {
            panic!("incremental parent fixture must certify")
        };
        (
            graph,
            portfolio,
            work.raw_frontiers,
            old_points,
            promoted_points,
            retained_context,
        )
    }

    #[test]
    fn incremental_promotion_reuses_old_domains_and_only_searches_new_branches() {
        let (graph, parent, parent_frontiers, old_points, promoted_points, retained_context) =
            incremental_promotion_fixture();
        let old_variable = parent.variables[0].clone();
        let routes = [
            route("old", "OLD", &old_points),
            route("promoted", "PROMOTED", &promoted_points),
        ];
        let corridors = [graph];
        let outcome = build_and_select_promoted_route_families(
            RouteFamilyPortfolioInput {
                portfolio_id: "expanded",
                placement_revision: 4,
                geometry_revision: 2,
                corridors: &corridors,
                routes: &routes,
                fixed_context: &[retained_context],
            },
            &parent,
            &parent_frontiers,
            &BTreeSet::from(["promoted".into()]),
            FamilyAssignmentLimits::default(),
        );
        let RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work,
            ..
        } = outcome
        else {
            panic!(
                "incremental expansion must produce a validated assignment portfolio, got {outcome:?}"
            )
        };
        assert_eq!(
            portfolio.variables.iter().find(|v| v.branch == "old"),
            Some(&old_variable)
        );
        assert_eq!(production_work.family_searches, 1);
        assert_eq!(production_work.family_variables_reused, 1);
        assert_eq!(
            production_work.families_reused,
            old_variable.families.len() as u64
        );
        assert_eq!(production_work.family_pair_outcomes_reused, 0);
        assert_eq!(
            production_work.family_context_pair_outcomes_reused,
            old_variable.families.len() as u64
        );
        assert_eq!(
            production_work.incremental_reuse_contract.as_deref(),
            Some(INCREMENTAL_PROMOTION_REUSE_CONTRACT)
        );
        assert_eq!(production_work.incremental_retained_branches, ["old"]);
        assert_eq!(production_work.incremental_promoted_branches, ["promoted"]);
        assert_eq!(
            production_work
                .incremental_parent_candidate_set_fingerprint
                .as_deref(),
            Some(parent.pair_analysis.candidate_set_fingerprint.as_str())
        );
        assert_eq!(
            production_work
                .incremental_result_candidate_set_fingerprint
                .as_deref(),
            Some(portfolio.pair_analysis.candidate_set_fingerprint.as_str())
        );
        portfolio.validate().unwrap();
    }

    #[test]
    fn incremental_promotion_selects_a_promoted_family_found_only_at_k16() {
        let obstacles = [3.0, 7.0, 11.0, 15.0]
            .into_iter()
            .enumerate()
            .map(|(index, x)| CorridorObstacle {
                id: format!("choice-{index}"),
                polygon: vec![
                    Vec2::new(x, 4.0),
                    Vec2::new(x + 2.0, 4.0),
                    Vec2::new(x + 2.0, 8.0),
                    Vec2::new(x, 8.0),
                ],
            })
            .collect::<Vec<_>>();
        let top = build_corridor_graph("top", board(), &obstacles, 4, 9).unwrap();
        let bottom_obstacles = [CorridorObstacle {
            id: "bottom-middle".into(),
            polygon: vec![
                Vec2::new(8.0, 3.0),
                Vec2::new(12.0, 3.0),
                Vec2::new(12.0, 9.0),
                Vec2::new(8.0, 9.0),
            ],
        }];
        let bottom = build_corridor_graph("bottom", board(), &bottom_obstacles, 4, 9).unwrap();
        let promoted_points = [Vec2::new(1.0, 5.0), Vec2::new(19.0, 5.0)];
        let mut promoted_route = [route("promoted", "PROMOTED", &promoted_points)];
        promoted_route[0].physical_width = 0.02;
        promoted_route[0].clearance = 0.005;
        let top_only = [top.clone()];
        let RouteFamilyPortfolioBuildOutcome::Complete {
            portfolio: full_frontier,
            ..
        } = build_route_family_portfolio_with_bounds(
            RouteFamilyPortfolioInput {
                portfolio_id: "k16-probe",
                placement_revision: 4,
                geometry_revision: 2,
                corridors: &top_only,
                routes: &promoted_route,
                fixed_context: &[],
            },
            TARGETED_BOUNDS,
        )
        else {
            panic!("the promoted-route probe must expose a complete K16 frontier")
        };
        let families = &full_frontier.variables[0].families;
        assert_eq!(families.len(), TARGETED_RAW_FAMILY_CANDIDATE_LIMIT);

        let points = |family: &RouteFamily| {
            family.runs[0]
                .witness
                .polyline
                .iter()
                .map(|point| Vec2::new(point.x, point.y))
                .collect::<Vec<_>>()
        };
        let family_polylines = families.iter().map(points).collect::<Vec<_>>();
        let distance_to_polyline = |point: Vec2, polyline: &[Vec2]| {
            polyline
                .windows(2)
                .map(|segment| {
                    crate::geometry::point_segment_distance(point, segment[0], segment[1])
                })
                .fold(f64::INFINITY, f64::min)
        };
        let (deep_family_ordinal, deep_family_id, blocker_points) = families
            .iter()
            .enumerate()
            .skip(RAW_FAMILY_CANDIDATE_LIMIT)
            .find_map(|(ordinal, candidate)| {
                let candidate_points = points(candidate);
                let mut blockers = Vec::new();
                for shallow in &families[..RAW_FAMILY_CANDIDATE_LIMIT] {
                    let shallow_points = points(shallow);
                    let best = shallow_points
                        .iter()
                        .copied()
                        .chain(shallow_points.windows(2).map(|segment| {
                            Vec2::new(
                                (segment[0].x + segment[1].x) * 0.5,
                                (segment[0].y + segment[1].y) * 0.5,
                            )
                        }))
                        .map(|point| (distance_to_polyline(point, &candidate_points), point))
                        .max_by(|left, right| left.0.total_cmp(&right.0))
                        .expect("a certified witness has at least two points");
                    if best.0 <= 0.05 {
                        return None;
                    }
                    blockers.push(best.1);
                }
                Some((ordinal, candidate.family_id.clone(), blockers))
            })
            .expect("one K9-K16 family must be geometrically separable from every K1-K8 witness");
        assert!(deep_family_ordinal >= RAW_FAMILY_CANDIDATE_LIMIT);
        assert!(
            families[..RAW_FAMILY_CANDIDATE_LIMIT]
                .iter()
                .all(|family| family.family_id != deep_family_id)
        );
        let blockers = blocker_points
            .into_iter()
            .enumerate()
            .map(|(index, point)| RouteFamilyFixedContextInput {
                context_id: format!("shallow-blocker-{index}"),
                branch: format!("shallow-blocker-{index}"),
                electrical_net: format!("BLOCKER-{index}"),
                layer: "top".into(),
                physical_width: 0.002,
                clearance: 0.001,
                points: vec![point],
            })
            .collect::<Vec<_>>();
        let prepared_family = |index: usize| {
            prepare_fixed_copper_route(FixedCopperRoute {
                polyline: &family_polylines[index],
                width: families[index].runs[0].witness.required_width,
                clearance: 0.005,
            })
            .unwrap()
        };
        for (index, blocker) in blockers.iter().enumerate() {
            let shallow = prepared_family(index);
            let blocker = prepare_fixed_copper_route(FixedCopperRoute {
                polyline: &blocker.points,
                width: blocker.physical_width,
                clearance: blocker.clearance,
            })
            .unwrap();
            assert!(
                !matches!(
                    classify_prepared_copper_pair(&shallow, &blocker).unwrap(),
                    FixedCopperPairClassification::Clear(_)
                ),
                "each K1-K8 witness must be blocked by exact copper classification"
            );
        }
        let deep = prepared_family(
            families
                .iter()
                .position(|family| family.family_id == deep_family_id)
                .unwrap(),
        );
        for blocker in &blockers {
            let blocker = prepare_fixed_copper_route(FixedCopperRoute {
                polyline: &blocker.points,
                width: blocker.physical_width,
                clearance: blocker.clearance,
            })
            .unwrap();
            assert!(
                matches!(
                    classify_prepared_copper_pair(&deep, &blocker).unwrap(),
                    FixedCopperPairClassification::Clear(_)
                ),
                "the selected K9-K16 witness must be clear of every exact blocker"
            );
        }

        let old_points = [Vec2::new(1.0, 6.0), Vec2::new(19.0, 6.0)];
        let mut old_route = route("old", "OLD", &old_points);
        old_route.layer = "bottom";
        let parent_routes = [old_route.clone()];
        let promoted_context = RouteFamilyFixedContextInput {
            context_id: "promoted".into(),
            branch: "promoted".into(),
            electrical_net: "PROMOTED".into(),
            layer: "top".into(),
            physical_width: 0.02,
            clearance: 0.005,
            points: promoted_points.to_vec(),
        };
        let mut parent_context = vec![promoted_context];
        parent_context.extend(blockers.iter().cloned());
        let corridors = [top, bottom];
        let RouteFamilyPortfolioBuildOutcome::Complete {
            portfolio: parent,
            work: parent_work,
        } = build_route_family_portfolio(RouteFamilyPortfolioInput {
            portfolio_id: "k16-parent",
            placement_revision: 4,
            geometry_revision: 2,
            corridors: &corridors,
            routes: &parent_routes,
            fixed_context: &parent_context,
        })
        else {
            panic!("the unrelated bottom-layer parent must certify")
        };
        let mut expanded_promoted = route("promoted", "PROMOTED", &promoted_points);
        expanded_promoted.physical_width = 0.02;
        expanded_promoted.clearance = 0.005;
        let expanded_routes = [old_route, expanded_promoted];
        let outcome = build_and_select_promoted_route_families(
            RouteFamilyPortfolioInput {
                portfolio_id: "k16-expanded",
                placement_revision: 4,
                geometry_revision: 2,
                corridors: &corridors,
                routes: &expanded_routes,
                fixed_context: &blockers,
            },
            &parent,
            &parent_work.raw_frontiers,
            &BTreeSet::from(["promoted".into()]),
            FamilyAssignmentLimits::default(),
        );
        let RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work,
            assignment: FamilyAssignmentOutcome::Selected { selection, .. },
        } = outcome
        else {
            panic!(
                "K16 incremental deepening must find the first unblocked family, got {outcome:?}"
            )
        };
        assert_eq!(selection.choices["promoted"], deep_family_id);
        assert!(
            portfolio
                .variables
                .iter()
                .find(|variable| variable.branch == "promoted")
                .is_some_and(|variable| variable.families.len() > RAW_FAMILY_CANDIDATE_LIMIT)
        );
        assert_eq!(production_work.family_searches, 2);
        assert!(production_work.planning_stages.iter().any(|stage| {
            stage.stage == RouteFamilyPlanningStageKind::TargetedRaw16
                && stage.outcome == RouteFamilyPlanningStageOutcome::Selected
        }));
        let targeted_evidence = production_work
            .planning_stages
            .iter()
            .find(|stage| stage.stage == RouteFamilyPlanningStageKind::TargetedRaw16)
            .and_then(|stage| stage.targeted_deepening.as_ref())
            .expect("selected K16 stage must carry targeted work evidence");
        let targeted_branches = targeted_evidence
            .selected_variables
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let unchanged_variables = portfolio
            .variables
            .iter()
            .filter(|variable| !targeted_branches.contains(variable.branch.as_str()))
            .collect::<Vec<_>>();
        let unchanged_family_count = unchanged_variables
            .iter()
            .map(|variable| variable.families.len() as u64)
            .sum::<u64>();
        let parent_family_count = parent
            .variables
            .iter()
            .map(|variable| variable.families.len() as u64)
            .sum::<u64>();
        assert_eq!(
            targeted_evidence.reused_variables,
            unchanged_variables.len() as u64
        );
        assert_eq!(
            targeted_evidence.reused_families,
            unchanged_family_count + targeted_evidence.prior_family_identities_verified
        );
        assert_eq!(
            production_work.family_variables_reused,
            parent.variables.len() as u64 + targeted_evidence.reused_variables
        );
        assert_eq!(
            production_work.families_reused,
            parent_family_count + targeted_evidence.reused_families
        );
        portfolio.validate().unwrap();
    }

    #[test]
    fn incremental_promotion_rejects_tampered_parent_conflict_certificate() {
        let (graph, mut parent, parent_frontiers, old_points, promoted_points, retained_context) =
            incremental_promotion_fixture();
        assert!(!parent.pair_analysis.context_conflicts.is_empty());
        parent.pair_analysis.context_conflicts.pop();
        parent.seal_pair_analysis_fingerprints().unwrap();
        let routes = [
            route("old", "OLD", &old_points),
            route("promoted", "PROMOTED", &promoted_points),
        ];
        let corridors = [graph];
        let outcome = build_and_select_promoted_route_families(
            RouteFamilyPortfolioInput {
                portfolio_id: "tampered",
                placement_revision: 4,
                geometry_revision: 2,
                corridors: &corridors,
                routes: &routes,
                fixed_context: &[retained_context],
            },
            &parent,
            &parent_frontiers,
            &BTreeSet::from(["promoted".into()]),
            FamilyAssignmentLimits::default(),
        );
        assert!(matches!(
            outcome,
            RouteFamilyPlanningOutcome::PortfolioUnavailable {
                outcome: RouteFamilyPortfolioBuildOutcome::InvalidProducedPortfolio { .. }
            }
        ));
    }

    #[test]
    fn incremental_promotion_rejects_missing_parent_frontier_before_search() {
        let (graph, parent, _, old_points, promoted_points, retained_context) =
            incremental_promotion_fixture();
        let routes = [
            route("old", "OLD", &old_points),
            route("promoted", "PROMOTED", &promoted_points),
        ];
        let corridors = [graph];
        let outcome = build_and_select_promoted_route_families(
            RouteFamilyPortfolioInput {
                portfolio_id: "missing-frontier",
                placement_revision: 4,
                geometry_revision: 2,
                corridors: &corridors,
                routes: &routes,
                fixed_context: &[retained_context],
            },
            &parent,
            &[],
            &BTreeSet::from(["promoted".into()]),
            FamilyAssignmentLimits::default(),
        );
        let RouteFamilyPlanningOutcome::PortfolioUnavailable {
            outcome: RouteFamilyPortfolioBuildOutcome::Unsupported { detail, work, .. },
        } = outcome
        else {
            panic!("missing parent frontier must fail closed")
        };
        assert!(detail.contains("no record bound to retained domain"));
        assert_eq!(work.family_searches, 0);
    }

    #[test]
    fn incremental_promotion_rejects_context_not_exactly_filtered_from_parent() {
        let (graph, parent, parent_frontiers, old_points, promoted_points, mut retained_context) =
            incremental_promotion_fixture();
        retained_context.points[0].x += 0.01;
        let routes = [
            route("old", "OLD", &old_points),
            route("promoted", "PROMOTED", &promoted_points),
        ];
        let corridors = [graph];
        let outcome = build_and_select_promoted_route_families(
            RouteFamilyPortfolioInput {
                portfolio_id: "context-tampered",
                placement_revision: 4,
                geometry_revision: 2,
                corridors: &corridors,
                routes: &routes,
                fixed_context: &[retained_context],
            },
            &parent,
            &parent_frontiers,
            &BTreeSet::from(["promoted".into()]),
            FamilyAssignmentLimits::default(),
        );
        let RouteFamilyPlanningOutcome::PortfolioUnavailable {
            outcome: RouteFamilyPortfolioBuildOutcome::Unsupported { detail, work, .. },
        } = outcome
        else {
            panic!("context tamper must fail closed before generation")
        };
        assert!(detail.contains("exact parent context minus promoted branches"));
        assert_eq!(work.family_searches, 0);
    }

    #[test]
    fn incremental_promotion_rejects_retained_layer_change_at_same_revision() {
        let (graph, parent, parent_frontiers, old_points, promoted_points, retained_context) =
            incremental_promotion_fixture();
        let mut routes = [
            route("old", "OLD", &old_points),
            route("promoted", "PROMOTED", &promoted_points),
        ];
        routes[0].layer = "bottom";
        let corridors = [graph];
        let outcome = build_and_select_promoted_route_families(
            RouteFamilyPortfolioInput {
                portfolio_id: "layer-tampered",
                placement_revision: 4,
                geometry_revision: 2,
                corridors: &corridors,
                routes: &routes,
                fixed_context: &[retained_context],
            },
            &parent,
            &parent_frontiers,
            &BTreeSet::from(["promoted".into()]),
            FamilyAssignmentLimits::default(),
        );
        let RouteFamilyPlanningOutcome::PortfolioUnavailable {
            outcome: RouteFamilyPortfolioBuildOutcome::Unsupported { detail, work, .. },
        } = outcome
        else {
            panic!("retained layer change must fail closed")
        };
        assert!(detail.contains("live layer and endpoint contract"));
        assert_eq!(work.family_searches, 0);
    }

    #[test]
    fn incremental_promotion_rejects_retained_endpoint_change_at_same_revision() {
        let (graph, parent, parent_frontiers, _, promoted_points, retained_context) =
            incremental_promotion_fixture();
        let shifted_old_points = [Vec2::new(1.01, 6.0), Vec2::new(19.0, 6.0)];
        let routes = [
            route("old", "OLD", &shifted_old_points),
            route("promoted", "PROMOTED", &promoted_points),
        ];
        let corridors = [graph];
        let outcome = build_and_select_promoted_route_families(
            RouteFamilyPortfolioInput {
                portfolio_id: "endpoint-tampered",
                placement_revision: 4,
                geometry_revision: 2,
                corridors: &corridors,
                routes: &routes,
                fixed_context: &[retained_context],
            },
            &parent,
            &parent_frontiers,
            &BTreeSet::from(["promoted".into()]),
            FamilyAssignmentLimits::default(),
        );
        let RouteFamilyPlanningOutcome::PortfolioUnavailable {
            outcome: RouteFamilyPortfolioBuildOutcome::Unsupported { detail, work, .. },
        } = outcome
        else {
            panic!("retained endpoint change must fail closed")
        };
        assert!(detail.contains("live layer and endpoint contract"));
        assert_eq!(work.family_searches, 0);
    }

    #[test]
    fn incremental_promotion_copies_complete_old_pair_matrix_and_tamper_fails_closed() {
        let obstacle = CorridorObstacle {
            id: "middle".into(),
            polygon: vec![
                Vec2::new(8.0, 3.0),
                Vec2::new(12.0, 3.0),
                Vec2::new(12.0, 9.0),
                Vec2::new(8.0, 9.0),
            ],
        };
        let graph = build_corridor_graph("top", board(), &[obstacle], 4, 9).unwrap();
        let old_a = [Vec2::new(1.0, 6.0), Vec2::new(19.0, 6.0)];
        let old_b = [Vec2::new(1.0, 6.0), Vec2::new(19.0, 6.0)];
        let promoted = [Vec2::new(1.0, 5.0), Vec2::new(19.0, 5.0)];
        let parent_routes = [route("old-a", "A", &old_a), route("old-b", "B", &old_b)];
        let promoted_context = RouteFamilyFixedContextInput {
            context_id: "promoted".into(),
            branch: "promoted".into(),
            electrical_net: "P".into(),
            layer: "top".into(),
            physical_width: 0.4,
            clearance: 0.2,
            points: promoted.to_vec(),
        };
        let corridors = [graph.clone()];
        let RouteFamilyPortfolioBuildOutcome::Complete {
            portfolio: parent,
            work: parent_work,
        } = build_route_family_portfolio(RouteFamilyPortfolioInput {
            portfolio_id: "pair-parent",
            placement_revision: 4,
            geometry_revision: 2,
            corridors: &corridors,
            routes: &parent_routes,
            fixed_context: &[promoted_context],
        })
        else {
            panic!("pair parent must certify")
        };
        let expected_reused_pairs = expected_family_pairs(&parent.variables);
        assert!(expected_reused_pairs > 0);
        let expanded_routes = [
            route("old-a", "A", &old_a),
            route("old-b", "B", &old_b),
            route("promoted", "P", &promoted),
        ];
        let build = |prior: &RouteFamilyPortfolio| {
            build_and_select_promoted_route_families(
                RouteFamilyPortfolioInput {
                    portfolio_id: "pair-expanded",
                    placement_revision: 4,
                    geometry_revision: 2,
                    corridors: &corridors,
                    routes: &expanded_routes,
                    fixed_context: &[],
                },
                prior,
                &parent_work.raw_frontiers,
                &BTreeSet::from(["promoted".into()]),
                FamilyAssignmentLimits::default(),
            )
        };
        let RouteFamilyPlanningOutcome::Assignment {
            portfolio,
            production_work,
            ..
        } = build(&parent)
        else {
            panic!("pair expansion must validate, got {:?}", build(&parent))
        };
        assert_eq!(production_work.family_searches, 1);
        assert_eq!(
            production_work.family_pair_outcomes_reused,
            expected_reused_pairs
        );
        assert!(parent.variables.iter().all(|old| {
            portfolio
                .variables
                .iter()
                .find(|new| new.branch == old.branch)
                == Some(old)
        }));

        let mut tampered = parent.clone();
        assert!(!tampered.pair_analysis.conflicts.is_empty());
        tampered.pair_analysis.conflicts.pop();
        tampered.seal_pair_analysis_fingerprints().unwrap();
        assert!(matches!(
            build(&tampered),
            RouteFamilyPlanningOutcome::PortfolioUnavailable {
                outcome: RouteFamilyPortfolioBuildOutcome::InvalidProducedPortfolio { .. }
            }
        ));
    }
}

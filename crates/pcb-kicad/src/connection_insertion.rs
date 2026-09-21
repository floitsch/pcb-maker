// Copyright (C) 2026 Toit contributors.

use super::*;
use std::time::Instant;
const TRANSACTION_SCHEMA_VERSION: u32 = 4;
const DIRECTORY_HASH_CONTRACT: &str = "pcb-maker-kicad-directory-v1";

/// Optional control used only after every inserted-connection route has
/// failed. It is deliberately named as a declared-target control: this is not
/// evidence that a global repair algorithm discovered the target layout.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadConnectionInsertionFallback {
    None,
    #[default]
    DeclaredTargetControl,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadLocalNativeAdmission {
    /// Native-gate every generated local candidate before ranking complete
    /// children. This is the control policy for algorithm comparisons.
    #[default]
    Exhaustive,
    /// Rank generated candidates by the same deterministic selection score,
    /// then native-gate them until the first complete child. Lower-ranked
    /// candidates remain explicit, unevaluated evidence.
    ScoreOrderedUntilComplete,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadUnroutedTargetGate {
    /// Gate the deliberately incomplete target before any route search.
    #[default]
    Eager,
    /// Defer this diagnostic gate until all ordinary routes fail. A complete
    /// child still receives the normal native admission gate.
    BeforeFallbackOrRollback,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadFutureReachabilityPolicy {
    /// Retain every native-complete child, then rank fewer unreachable future
    /// connections ahead of local route score.
    #[default]
    PreferMoreReachable,
    /// Do not admit a native-complete child when any checked future connection
    /// is raster-unreachable. This is an intentionally stricter experimental
    /// policy; the final KiCad gate remains unchanged.
    RequireAll,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadFutureReachabilityConfig {
    /// Maximum number of still-enabled declaration connections checked after
    /// the newly inserted connection. Zero is invalid; `None` disables the
    /// entire lookahead through the parent insertion config.
    pub maximum_connections: usize,
    pub policy: KiCadFutureReachabilityPolicy,
    pub routing: KiCadGridRouteConfig,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadFutureConnectionReachabilityTrial {
    pub ordinal: usize,
    pub declaration_index: usize,
    pub rung: usize,
    pub connection: String,
    pub route_found: bool,
    pub reachability_rejected: bool,
    pub reachability_expansions: u32,
    pub route_expansions: Option<u32>,
    pub candidate: Option<PathBuf>,
    pub error: Option<String>,
    pub elapsed_micros: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadFutureReachabilityEvidence {
    pub schema_version: u32,
    /// Retained beside the candidate rather than below it, so accepting the
    /// candidate cannot recursively copy diagnostic boards into later rungs.
    pub artifact_directory: PathBuf,
    pub policy: KiCadFutureReachabilityPolicy,
    pub maximum_connections: usize,
    pub available_connections: usize,
    pub checked_connections: usize,
    pub reachable_connections: usize,
    pub unreachable_connections: usize,
    pub all_checked_reachable: bool,
    pub total_reachability_expansions: u64,
    pub total_route_expansions: u64,
    pub elapsed_micros: u64,
    pub trials: Vec<KiCadFutureConnectionReachabilityTrial>,
    pub interpretation: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadOrderedLocalRefinementConfig {
    /// Always evaluate at least this many portfolio entries before deciding
    /// whether the ordered refinement has converged.
    pub minimum_portfolio_entries: usize,
    /// Continue to the next portfolio entry only when the latest generated
    /// candidate improves the previous best score by at least this percent.
    pub continue_if_score_improvement_percent_at_least: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadOrderedLocalRefinementEvidence {
    pub evaluated_before_stop: usize,
    pub portfolio_entries_available: usize,
    pub previous_best_score_mm: f64,
    pub latest_score_mm: f64,
    pub improvement_percent: f64,
    pub continue_threshold_percent: f64,
    pub resumed_after_native_rejection: bool,
    pub interpretation: String,
}

/// Evidence predicate for an expensive local routing mode. Predicates inspect
/// already-generated cheap candidates or structured search failures; they never replace native
/// admission or infer success from a surrogate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KiCadConditionalLocalRoutingTrigger {
    BestCandidateViasAtLeast {
        minimum_vias: usize,
    },
    /// Try an expensive search mode only when no earlier route was generated
    /// and at least one earlier local search exhausted its budget.
    SearchBudgetExhaustedWithoutCandidate,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadConditionalLocalRoutingConfig {
    pub trigger: KiCadConditionalLocalRoutingTrigger,
    pub routing: KiCadGridRouteConfig,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadConditionalLocalRoutingEvidence {
    pub ordinal: usize,
    pub trigger: KiCadConditionalLocalRoutingTrigger,
    pub routing_config_sha256: String,
    pub observed_best_score_mm: Option<f64>,
    pub observed_best_vias: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_budget_exhausted_attempts: Option<Vec<usize>>,
    pub activated: bool,
    pub attempt_ordinal: Option<usize>,
    pub interpretation: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadConnectionInsertionConfig {
    /// Maximum number of portfolio entries to evaluate. Every evaluated
    /// entry is retained, including failures and native rejections.
    pub local_candidate_cap: usize,
    pub local_routing_portfolio: Vec<KiCadGridRouteConfig>,
    /// Hard bound over the ordered conditional suffix. Zero disables it.
    pub conditional_local_candidate_cap: usize,
    /// Expensive route modes activated from retained cheap-candidate evidence.
    /// Entries run in order and may observe an earlier conditional result.
    pub conditional_local_routing_portfolio: Vec<KiCadConditionalLocalRoutingConfig>,
    pub local_native_admission: KiCadLocalNativeAdmission,
    pub unrouted_target_gate: KiCadUnroutedTargetGate,
    /// Optional convergence rule for portfolios ordered coarse-to-fine. If
    /// every generated prefix candidate fails native admission, all skipped
    /// entries are restored before topology fallback.
    pub ordered_local_refinement: Option<KiCadOrderedLocalRefinementConfig>,
    /// Optional bounded counterfactual routing of still-enabled connections
    /// against each native-complete child. It never substitutes for the final
    /// KiCad gate.
    pub future_reachability: Option<KiCadFutureReachabilityConfig>,
    /// Optional parent-derived repair after fixed-copper local routing fails.
    /// Historical declared targets remain a separate control and are never
    /// selected.
    pub single_connection_ripup: Option<KiCadSingleConnectionRipupConfig>,
    pub fallback: KiCadConnectionInsertionFallback,
    /// Physical vias receive this extra length when complete children are
    /// ranked. KiCad completeness is always the primary criterion.
    pub via_penalty_mm: f64,
}

impl Default for KiCadConnectionInsertionConfig {
    fn default() -> Self {
        Self {
            local_candidate_cap: 1,
            local_routing_portfolio: vec![KiCadGridRouteConfig::default()],
            conditional_local_candidate_cap: 0,
            conditional_local_routing_portfolio: Vec::new(),
            local_native_admission: KiCadLocalNativeAdmission::Exhaustive,
            unrouted_target_gate: KiCadUnroutedTargetGate::Eager,
            ordered_local_refinement: None,
            future_reachability: None,
            single_connection_ripup: None,
            fallback: KiCadConnectionInsertionFallback::DeclaredTargetControl,
            via_penalty_mm: 2.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadConnectionInsertionDisposition {
    Committed,
    RolledBack,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadConnectionInsertionAttemptScope {
    InsertedConnectionOnly,
    ConditionalLocalRefinement,
    SingleConnectionRipup,
    DeclaredTargetControl,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadConnectionInsertionAttemptStatus {
    Committed,
    CandidateGenerated,
    ControlComplete,
    RouteFailed,
    NativeRejected,
    FutureReachabilityRejected,
    ExecutionFailed,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadConnectionInsertionAttempt {
    pub ordinal: usize,
    pub scope: KiCadConnectionInsertionAttemptScope,
    pub status: KiCadConnectionInsertionAttemptStatus,
    pub artifact_directory: PathBuf,
    pub routing_config_sha256: Option<String>,
    pub route_cost: Option<u32>,
    pub route_expansions: Option<u32>,
    /// Produced by the router, never inferred from the error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_failure: Option<KiCadRouteSearchFailure>,
    /// Advisory wall time for route generation only. Never used for
    /// selection, hashing, or correctness.
    pub route_elapsed_micros: u64,
    pub route_quality: Option<KiCadRouteQuality>,
    pub score_mm: Option<f64>,
    pub native_admission_order: Option<usize>,
    pub native_gate_invoked: bool,
    /// Advisory wall time for applying and native-gating this attempt.
    pub native_gate_elapsed_micros: u64,
    pub verification: Option<VerificationReport>,
    pub future_reachability: Option<KiCadFutureReachabilityEvidence>,
    pub single_connection_ripup: Option<Box<KiCadSingleConnectionRipupResult>>,
    pub error: Option<String>,
    pub selected: bool,
    /// Advisory total retained-attempt wall time.
    pub elapsed_micros: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadConnectionInsertionEvidence {
    pub schema_version: u32,
    pub contract: String,
    pub board_id: String,
    pub source_rung: usize,
    pub target_rung: usize,
    pub inserted_connection: String,
    pub declaration_sha256: String,
    pub config_sha256: String,
    pub external_parent_sha256_before: String,
    pub external_parent_sha256_after: String,
    pub source_snapshot_sha256: String,
    pub selected_artifact_sha256: String,
    pub parent_unchanged: bool,
    pub rollback_exact: bool,
    pub inserted_connection_terminal_count: usize,
    pub local_candidates_generated: usize,
    pub local_portfolio_entries_declared: usize,
    pub local_portfolio_entries_equivalent_skipped: usize,
    pub local_portfolio_entries_available: usize,
    pub local_portfolio_entries_evaluated: usize,
    pub conditional_local_entries_declared: usize,
    pub conditional_local_entries_activated: usize,
    pub conditional_local_entries_skipped: usize,
    pub conditional_local_routing: Vec<KiCadConditionalLocalRoutingEvidence>,
    pub local_native_gates: usize,
    pub local_candidates_not_native_gated: usize,
    pub future_reachability_evaluations: usize,
    pub future_reachability_rejections: usize,
    pub future_reachability_expansions: u64,
    pub future_route_expansions: u64,
    pub ordered_local_refinement: Option<KiCadOrderedLocalRefinementEvidence>,
    pub source_verification: VerificationReport,
    pub source_verification_reused: bool,
    pub unrouted_target_gate: KiCadUnroutedTargetGate,
    pub unrouted_target_verification: Option<VerificationReport>,
    pub attempts: Vec<KiCadConnectionInsertionAttempt>,
    /// Advisory transaction wall time. Correctness and selection never depend
    /// on this value.
    pub elapsed_micros: u64,
    pub selection_rule: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadConnectionInsertionResult {
    pub disposition: KiCadConnectionInsertionDisposition,
    pub selected_directory: PathBuf,
    pub selected_candidate: Option<PathBuf>,
    pub evidence: KiCadConnectionInsertionEvidence,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadConnectionProgressionConfig {
    /// Hard bound on committed connection deltas in one progression run.
    pub maximum_connections: usize,
    /// Reuse the immediately preceding committed child's native report only
    /// when its selected directory hash is unchanged.
    pub reuse_committed_parent_verification: bool,
    pub insertion: KiCadConnectionInsertionConfig,
}

impl Default for KiCadConnectionProgressionConfig {
    fn default() -> Self {
        Self {
            maximum_connections: 8,
            reuse_committed_parent_verification: false,
            insertion: KiCadConnectionInsertionConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadConnectionProgressionTermination {
    Running,
    BoundReached,
    FinalRungReached,
    RolledBack,
    ExecutionFailed,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadConnectionProgressionStep {
    pub ordinal: usize,
    pub source_rung: usize,
    pub target_rung: usize,
    pub transaction_directory: PathBuf,
    pub result: Option<KiCadConnectionInsertionResult>,
    pub quality_comparison: Option<KiCadConnectionProgressionQualityComparison>,
    pub error: Option<String>,
    /// Advisory wall time for this independently retained transaction.
    pub elapsed_micros: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadConnectionProgressionQualityComparison {
    pub selected: KiCadRouteQuality,
    pub historical_declared: Option<KiCadRouteQuality>,
    pub selected_minus_historical_length_mm: Option<f64>,
    pub selected_minus_historical_length_percent: Option<f64>,
    pub selected_minus_historical_vias: Option<i64>,
    pub historical_control_error: Option<String>,
    pub interpretation: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadConnectionProgressionResult {
    pub schema_version: u32,
    pub board_id: String,
    pub initial_rung: usize,
    pub final_rung: usize,
    pub initial_directory: PathBuf,
    pub final_directory: PathBuf,
    pub termination: KiCadConnectionProgressionTermination,
    pub committed_connections: usize,
    pub attempted_transactions: usize,
    pub elapsed_micros: u64,
    pub total_route_expansions: u64,
    pub total_future_reachability_evaluations: usize,
    pub total_future_reachability_rejections: usize,
    pub total_future_reachability_expansions: u64,
    pub total_future_route_expansions: u64,
    pub total_local_portfolio_entries_declared: usize,
    pub total_local_portfolio_entries_equivalent_skipped: usize,
    pub total_local_portfolio_entries_available: usize,
    pub total_local_portfolio_entries_evaluated: usize,
    pub total_conditional_local_entries_declared: usize,
    pub total_conditional_local_entries_activated: usize,
    pub total_conditional_local_entries_skipped: usize,
    pub total_local_candidates_generated: usize,
    pub total_local_native_gates: usize,
    pub steps: Vec<KiCadConnectionProgressionStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadSingleConnectionRipupConfig {
    /// Stop after the first fully restored, natively admitted candidate when enabled.
    #[serde(skip_serializing_if = "KiCadSequentialPortfolioPolicy::is_best_score")]
    pub selection_policy: KiCadSequentialPortfolioPolicy,
    pub diagnosis: KiCadYieldingConnectionDiagnosisConfig,
    pub via_penalty_mm: f64,
    /// Insert an entirely unrouted target on an incomplete board. Native opens
    /// must decrease by its pad-tree count, with no new disconnections.
    pub allow_partial_progress: bool,
    /// Only applies to partial progress; complete layouts still require no
    /// native design findings. New annotations are never allowed.
    pub allow_existing_annotation_findings: bool,
    /// Bounded recovery when the yielded net cannot be restored directly.
    pub restoration: Option<KiCadRestorationRecoveryConfig>,
}

impl Default for KiCadSingleConnectionRipupConfig {
    fn default() -> Self {
        Self {
            selection_policy: KiCadSequentialPortfolioPolicy::BestScore,
            diagnosis: KiCadYieldingConnectionDiagnosisConfig::default(),
            via_penalty_mm: 2.0,
            allow_partial_progress: false,
            allow_existing_annotation_findings: false,
            restoration: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadSingleConnectionRipupStatus {
    Complete,
    Progress,
    ProvisionalGateRejected,
    RerouteFailed,
    NativeRejected,
    ExecutionFailed,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadSingleConnectionRipupAttempt {
    pub ordinal: usize,
    pub yielding_connection: String,
    pub status: KiCadSingleConnectionRipupStatus,
    pub artifact_directory: PathBuf,
    pub target_candidate: PathBuf,
    pub provisional_board: Option<PathBuf>,
    pub provisional_directory: Option<PathBuf>,
    pub rerouted_candidate: Option<PathBuf>,
    pub source_yielding_quality: Option<KiCadRouteQuality>,
    pub target_quality: KiCadRouteQuality,
    pub target_route_cost: u32,
    pub target_route_expansions: u32,
    pub rerouted_quality: Option<KiCadRouteQuality>,
    pub rerouted_route_cost: Option<u32>,
    pub rerouted_route_expansions: Option<u32>,
    pub combined_score_mm: Option<f64>,
    /// Full-board score for partial insertion, affected-net score for the
    /// legacy complete-board repair mode.
    pub selection_score_mm: Option<f64>,
    pub provisional_verification: Option<VerificationReport>,
    pub final_verification: Option<VerificationReport>,
    /// Optional counterfactual evidence added by the enclosing connection
    /// transaction after this independently native-complete repair exists.
    pub future_reachability: Option<KiCadFutureReachabilityEvidence>,
    pub error: Option<String>,
    pub selected: bool,
    pub changed_routes: BTreeMap<String, KiCadChangedRouteReceipt>,
    pub restoration: Option<KiCadRestorationEvidence>,
    pub restoration_skip_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadSingleConnectionRipupResult {
    #[serde(skip_serializing_if = "KiCadSequentialPortfolioPolicy::is_best_score")]
    pub selection_policy: KiCadSequentialPortfolioPolicy,
    /// Successful, unprotected diagnosis trials left unmaterialized after admission.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped_diagnosis_trials: Vec<usize>,
    pub schema_version: u32,
    pub board_id: String,
    pub target_connection: String,
    pub source_directory: PathBuf,
    pub source_sha256: String,
    pub source_unchanged: bool,
    pub baseline_verification: VerificationReport,
    pub expected_unconnected_items: Option<usize>,
    pub diagnosis: KiCadYieldingConnectionDiagnosis,
    pub attempts: Vec<KiCadSingleConnectionRipupAttempt>,
    pub total_route_expansions: u64,
    pub selected_attempt: Option<usize>,
    pub selected_directory: Option<PathBuf>,
    pub selection_rule: String,
    pub restoration_invocations: usize,
    pub protected_connections: Vec<String>,
}

/// Tries the smallest parent-derived topology repair after fixed-copper local
/// insertion fails: route the new target while one old connection yields,
/// then reroute that exact old connection around the target and native-gate
/// the complete board.
pub fn repair_kicad_with_single_connection_ripup(
    source_directory: &Path,
    board_id: &str,
    target_connection: &str,
    output_root: &Path,
    config: &KiCadSingleConnectionRipupConfig,
) -> Result<KiCadSingleConnectionRipupResult, String> {
    restoration_recovery::validate(config)?;
    repair_with_restoration_budget(
        source_directory,
        board_id,
        target_connection,
        output_root,
        config,
        &mut restoration_recovery::Budget::default(),
        0,
        &[],
    )
}

pub(super) fn repair_with_restoration_budget(
    source_directory: &Path,
    board_id: &str,
    target_connection: &str,
    output_root: &Path,
    config: &KiCadSingleConnectionRipupConfig,
    budget: &mut restoration_recovery::Budget,
    depth: usize,
    protected: &[String],
) -> Result<KiCadSingleConnectionRipupResult, String> {
    let initial_invocations = budget.invocations;
    if config.diagnosis.maximum_yielding_connections != 1 {
        return Err("single-connection rip-up requires maximum_yielding_connections = 1".into());
    }
    if !config.via_penalty_mm.is_finite() || config.via_penalty_mm < 0.0 {
        return Err("via_penalty_mm must be finite and non-negative".into());
    }
    if output_root.exists() {
        return Err(format!(
            "refusing to overwrite KiCad single-rip-up experiment {}",
            output_root.display()
        ));
    }
    let source_before = directory_sha256(source_directory)?;
    fs::create_dir_all(output_root).map_err(|error| {
        format!(
            "failed to create KiCad single-rip-up experiment {}: {error}",
            output_root.display()
        )
    })?;
    let source_snapshot = output_root.join("source-unrouted-target");
    copy_directory_tree(source_directory, &source_snapshot)?;
    if directory_sha256(&source_snapshot)? != source_before {
        return Err("single-rip-up source snapshot is not byte-identical".into());
    }
    let baseline_directory = output_root.join("baseline-verification");
    copy_directory_tree(&source_snapshot, &baseline_directory)?;
    let baseline_erc_input_sha256 = kicad_erc_input_sha256(&baseline_directory)?;
    let baseline_verification = verify_materialized_rung(&baseline_directory, board_id)?;
    let baseline_erc = read_json(&baseline_directory.join("erc.json"))?;
    let source_board = source_snapshot.join(format!("{board_id}.kicad_pcb"));
    let baseline_drc = read_json(&baseline_directory.join("drc.json"))?;
    let expected_unconnected_items = if config.allow_partial_progress {
        if !adaptive_routing::native_progress_admissible(
            &baseline_verification,
            &baseline_drc,
            &baseline_drc,
            config.allow_existing_annotation_findings,
        )? {
            return Err("partial rip-up source failed the native progress gate".into());
        }
        Some(partial_ripup::expected_opens(
            &source_board,
            target_connection,
            &baseline_verification,
        )?)
    } else {
        validate_unrouted_target_gate(&baseline_verification)?;
        None
    };
    let diagnosis =
        diagnose_kicad_yielding_connections(&source_board, target_connection, &config.diagnosis)?;
    write_pretty_json(&output_root.join("diagnosis.json"), &diagnosis)?;

    let mut attempts = Vec::new();
    let mut complete = Vec::new();
    let mut skipped_diagnosis_trials = Vec::new();
    for diagnostic_trial in diagnosis.trials.iter().filter(|trial| trial.route_found) {
        let Some(target_candidate) = diagnostic_trial.candidate.as_ref() else {
            continue;
        };
        let Some(yielding_connection) = diagnostic_trial.yielding_connections.first() else {
            continue;
        };
        if protected
            .iter()
            .any(|net| normalize_net(net) == normalize_net(yielding_connection))
        {
            continue;
        }
        if config.selection_policy == KiCadSequentialPortfolioPolicy::OrderedFirstAdmitted
            && !complete.is_empty()
        {
            skipped_diagnosis_trials.push(diagnostic_trial.ordinal);
            continue;
        }
        let ordinal = attempts.len();
        let trial_directory = output_root.join(format!(
            "attempt-{ordinal:03}-yield-{}",
            filesystem_label(yielding_connection)
        ));
        copy_directory_tree(&source_snapshot, &trial_directory)?;
        let target_candidate_path = trial_directory.join("target-candidate.json");
        write_pretty_json(&target_candidate_path, target_candidate)?;
        let trial_board = trial_directory.join(format!("{board_id}.kicad_pcb"));
        let source_yielding_quality =
            import_route_candidate_from_pcb(&source_board, yielding_connection)
                .and_then(|candidate| route_candidate_quality(&candidate))
                .ok();
        let mut attempt = KiCadSingleConnectionRipupAttempt {
            ordinal,
            yielding_connection: yielding_connection.clone(),
            status: KiCadSingleConnectionRipupStatus::ExecutionFailed,
            artifact_directory: trial_directory.clone(),
            target_candidate: target_candidate_path.clone(),
            provisional_board: None,
            provisional_directory: None,
            rerouted_candidate: None,
            source_yielding_quality,
            target_quality: diagnostic_trial
                .quality
                .clone()
                .ok_or_else(|| "successful diagnosis has no route quality".to_string())?,
            target_route_cost: target_candidate.cost,
            target_route_expansions: target_candidate.expansions,
            rerouted_quality: None,
            rerouted_route_cost: None,
            rerouted_route_expansions: None,
            combined_score_mm: None,
            selection_score_mm: None,
            provisional_verification: None,
            final_verification: None,
            future_reachability: None,
            error: None,
            selected: false,
            changed_routes: BTreeMap::new(),
            restoration: None,
            restoration_skip_reason: None,
        };
        match apply_route_candidate_with_yielding_connections(
            &trial_board,
            &target_candidate_path,
            std::slice::from_ref(yielding_connection),
            &trial_board,
        ) {
            Err(error) => attempt.error = Some(error),
            Ok(()) => match verify_materialized_route_only(
                &trial_directory,
                board_id,
                &baseline_erc_input_sha256,
                &baseline_erc,
            ) {
                Err(error) => attempt.error = Some(error),
                Ok(provisional) => {
                    attempt.provisional_verification = Some(provisional.clone());
                    let provisional_board =
                        trial_directory.join("provisional-target-only.kicad_pcb");
                    fs::copy(&trial_board, &provisional_board).map_err(|error| {
                        format!(
                            "failed to preserve provisional board {}: {error}",
                            provisional_board.display()
                        )
                    })?;
                    attempt.provisional_board = Some(provisional_board);
                    // Preserve the native report and combined render before
                    // the same trial directory receives restored copper.
                    let provisional_directory =
                        output_root.join(format!("provisional-{ordinal:03}"));
                    copy_directory_tree(&trial_directory, &provisional_directory)?;
                    attempt.provisional_directory = Some(provisional_directory);
                    let provisional_ok = if config.allow_partial_progress {
                        adaptive_routing::native_progress_admissible(
                            &provisional,
                            &read_json(&trial_directory.join("drc.json"))?,
                            &baseline_drc,
                            config.allow_existing_annotation_findings,
                        )? && partial_ripup::preserved(
                            &source_snapshot,
                            &trial_directory,
                            board_id,
                            target_connection,
                            yielding_connection,
                        )?
                    } else {
                        !(provisional.complete
                            || provisional.erc_violations != 0
                            || provisional.drc_design_violations != 0
                            || provisional.schematic_parity_issues != 0
                            || provisional.selected_net_unconnected_items == 0)
                    };
                    if !provisional_ok {
                        attempt.status = KiCadSingleConnectionRipupStatus::ProvisionalGateRejected;
                        attempt.error = Some(format!(
                            "target-only provisional artifact did not isolate the yielded connection: {}",
                            verification_summary(&provisional)
                        ));
                    } else {
                        let mut reroute_config = config.diagnosis.routing.clone();
                        routing_demand::remove_completed(
                            &mut reroute_config,
                            &[target_connection.to_string()],
                        );
                        match route_materialized_connection(
                            &trial_board,
                            yielding_connection,
                            &reroute_config,
                        ) {
                            Err(error) => {
                                attempt.status = KiCadSingleConnectionRipupStatus::RerouteFailed;
                                attempt.rerouted_route_expansions =
                                    route_failure_expansions(&error);
                                attempt.error = Some(error);
                            }
                            Ok(rerouted) => {
                                let rerouted_path =
                                    trial_directory.join("yielded-reroute-candidate.json");
                                write_pretty_json(&rerouted_path, &rerouted)?;
                                attempt.rerouted_candidate = Some(rerouted_path.clone());
                                attempt.rerouted_route_cost = Some(rerouted.cost);
                                attempt.rerouted_route_expansions = Some(rerouted.expansions);
                                let quality = route_candidate_quality(&rerouted)?;
                                let score = attempt.target_quality.length_mm
                                    + quality.length_mm
                                    + (attempt.target_quality.vias + quality.vias) as f64
                                        * config.via_penalty_mm;
                                attempt.rerouted_quality = Some(quality);
                                attempt.combined_score_mm = Some(score);
                                attempt.selection_score_mm = Some(score);
                                match apply_route_candidate(
                                    &trial_board,
                                    &rerouted_path,
                                    &trial_board,
                                ) {
                                    Err(error) => attempt.error = Some(error),
                                    Ok(()) => {
                                        match verify_materialized_route_only(
                                            &trial_directory,
                                            board_id,
                                            &baseline_erc_input_sha256,
                                            &baseline_erc,
                                        ) {
                                            Err(error) => attempt.error = Some(error),
                                            Ok(verification) => {
                                                attempt.final_verification =
                                                    Some(verification.clone());
                                                let admissible = if let Some(expected) =
                                                    expected_unconnected_items
                                                {
                                                    partial_ripup::final_admissible(
                                                        &verification,
                                                        &read_json(
                                                            &trial_directory.join("drc.json"),
                                                        )?,
                                                        &baseline_drc,
                                                        expected,
                                                        config.allow_existing_annotation_findings,
                                                    )? && partial_ripup::preserved(
                                                        &source_snapshot,
                                                        &trial_directory,
                                                        board_id,
                                                        target_connection,
                                                        yielding_connection,
                                                    )?
                                                } else {
                                                    verification.complete
                                                };
                                                if admissible {
                                                    attempt.status = if verification.complete {
                                                        KiCadSingleConnectionRipupStatus::Complete
                                                    } else {
                                                        KiCadSingleConnectionRipupStatus::Progress
                                                    };
                                                    let selection_score =
                                                        if config.allow_partial_progress {
                                                            let stats =
                                                                inspect_kicad_board(&trial_board)?;
                                                            stats
                                                                .physical_copper
                                                                .physical_centerline_length_mm
                                                                + stats.vias as f64
                                                                    * config.via_penalty_mm
                                                        } else {
                                                            score
                                                        };
                                                    attempt.selection_score_mm =
                                                        Some(selection_score);
                                                    complete.push((ordinal, selection_score));
                                                } else {
                                                    attempt.status =
                                                    KiCadSingleConnectionRipupStatus::NativeRejected;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            },
        }
        if matches!(
            attempt.status,
            KiCadSingleConnectionRipupStatus::Complete | KiCadSingleConnectionRipupStatus::Progress
        ) {
            attempt.changed_routes.insert(
                normalize_net(target_connection).to_string(),
                restoration_recovery::receipt(&attempt.target_candidate)?,
            );
            attempt.changed_routes.insert(
                normalize_net(yielding_connection).to_string(),
                restoration_recovery::receipt(
                    attempt
                        .rerouted_candidate
                        .as_ref()
                        .ok_or("admitted repair missing restored candidate")?,
                )?,
            );
        } else if attempt.status == KiCadSingleConnectionRipupStatus::RerouteFailed {
            restoration_recovery::recover(
                &source_snapshot,
                board_id,
                target_connection,
                &baseline_drc,
                expected_unconnected_items,
                config,
                budget,
                depth,
                protected,
                &mut attempt,
            )?;
            if matches!(
                attempt.status,
                KiCadSingleConnectionRipupStatus::Complete
                    | KiCadSingleConnectionRipupStatus::Progress
            ) {
                complete.push((
                    ordinal,
                    attempt
                        .selection_score_mm
                        .ok_or("nested repair missing whole-board score")?,
                ));
            }
        }
        write_pretty_json(&trial_directory.join("attempt.json"), &attempt)?;
        attempts.push(attempt);
    }
    complete.sort_by(|left, right| {
        (attempts[right.0].status == KiCadSingleConnectionRipupStatus::Complete)
            .cmp(&(attempts[left.0].status == KiCadSingleConnectionRipupStatus::Complete))
            .then_with(|| left.1.total_cmp(&right.1))
            .then_with(|| left.0.cmp(&right.0))
    });
    let selected_attempt = complete.first().map(|(ordinal, _)| *ordinal);
    if let Some(selected) = selected_attempt {
        attempts[selected].selected = true;
    }
    for attempt in &attempts {
        write_pretty_json(&attempt.artifact_directory.join("attempt.json"), attempt)?;
    }
    let total_route_expansions = diagnosis.total_route_expansions
        + attempts
            .iter()
            .filter_map(|attempt| attempt.rerouted_route_expansions)
            .map(u64::from)
            .sum::<u64>()
        + attempts
            .iter()
            .filter_map(|a| a.restoration.as_ref())
            .map(|r| r.route_expansions)
            .sum::<u64>();
    let source_unchanged = directory_sha256(source_directory)? == source_before;
    if !source_unchanged {
        return Err(format!(
            "single-connection rip-up modified its source directory {}",
            source_directory.display()
        ));
    }
    let result = KiCadSingleConnectionRipupResult {
        selection_policy: config.selection_policy,
        skipped_diagnosis_trials,
        schema_version: TRANSACTION_SCHEMA_VERSION,
        board_id: board_id.to_string(),
        target_connection: target_connection.to_string(),
        source_directory: source_directory.to_path_buf(),
        source_sha256: source_before.clone(),
        source_unchanged,
        baseline_verification,
        expected_unconnected_items,
        diagnosis,
        total_route_expansions,
        selected_attempt,
        selected_directory: selected_attempt
            .map(|selected| attempts[selected].artifact_directory.clone()),
        attempts,
        selection_rule: if config.selection_policy
            == KiCadSequentialPortfolioPolicy::OrderedFirstAdmitted
        {
            "first candidate with every displaced net restored and native completeness, or explicitly enabled exact target connectivity progress with unchanged other opens/design; remaining eligible diagnosis trials are skipped".into()
        } else {
            format!(
                "native completeness, or explicitly enabled exact target connectivity progress with unchanged other opens/design; then {} physical copper + {:.6} mm per via, attempt ordinal",
                if config.allow_partial_progress {
                    "whole-board"
                } else {
                    "target + rerouted"
                },
                config.via_penalty_mm
            )
        },
        restoration_invocations: budget.invocations - initial_invocations,
        protected_connections: protected.to_vec(),
    };
    write_pretty_json(&output_root.join("ripup.json"), &result)?;
    Ok(result)
}

fn filesystem_label(value: &str) -> String {
    let label: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect();
    if label.is_empty() {
        "unnamed".into()
    } else {
        label
    }
}

/// Repeatedly applies [`insert_kicad_connection`] to its own committed child.
///
/// The run is explicitly bounded and stops at the first rollback or execution
/// failure. A partial directory is durable evidence, not a reason to restart
/// from a historical rung. `progression.json` is rewritten after every step so
/// an interrupted experiment still exposes its last completed transaction.
pub fn progress_kicad_connections(
    declaration: &LadderDeclaration,
    initial_parent_directory: &Path,
    output_root: &Path,
    config: &KiCadConnectionProgressionConfig,
) -> Result<KiCadConnectionProgressionResult, String> {
    if config.maximum_connections == 0 {
        return Err("maximum_connections must be greater than zero".into());
    }
    validate_insertion_config(&config.insertion)?;
    if output_root.exists() {
        return Err(format!(
            "refusing to overwrite KiCad connection progression {}",
            output_root.display()
        ));
    }
    let initial_report: MaterializationReport =
        read_typed_json(&initial_parent_directory.join("materialization.json"))?;
    validate_parent_report(declaration, &initial_report)?;
    fs::create_dir_all(output_root).map_err(|error| {
        format!(
            "failed to create KiCad connection progression {}: {error}",
            output_root.display()
        )
    })?;

    let initial_rung = initial_report.rung;
    let mut final_rung = initial_rung;
    let mut final_directory = initial_parent_directory.to_path_buf();
    let mut trusted_parent: Option<(String, VerificationReport)> = None;
    let mut steps = Vec::new();
    let mut termination = if final_rung == declaration.connections.len() {
        KiCadConnectionProgressionTermination::FinalRungReached
    } else {
        KiCadConnectionProgressionTermination::Running
    };

    for ordinal in 0..config.maximum_connections {
        if final_rung == declaration.connections.len() {
            termination = KiCadConnectionProgressionTermination::FinalRungReached;
            break;
        }
        let source_rung = final_rung;
        let target_rung = source_rung + 1;
        let transaction_directory = output_root.join(format!(
            "step-{ordinal:03}-rung-{source_rung:02}-to-{target_rung:02}"
        ));
        let step_started = Instant::now();
        let trusted_source = if config.reuse_committed_parent_verification {
            trusted_parent
                .as_ref()
                .map(|(directory_sha256, report)| TrustedSourceVerification {
                    directory_sha256,
                    report,
                })
        } else {
            None
        };
        match insert_kicad_connection_with_trusted_source(
            declaration,
            &final_directory,
            &transaction_directory,
            &config.insertion,
            trusted_source,
        ) {
            Ok(result) => {
                let disposition = result.disposition;
                let selected_directory = result.selected_directory.clone();
                let next_trusted_parent = if disposition
                    == KiCadConnectionInsertionDisposition::Committed
                    && config.reuse_committed_parent_verification
                {
                    result
                        .evidence
                        .attempts
                        .iter()
                        .find(|attempt| attempt.selected)
                        .and_then(|attempt| attempt.verification.clone())
                        .filter(|report| report.complete)
                        .map(|report| (result.evidence.selected_artifact_sha256.clone(), report))
                } else {
                    None
                };
                let quality_comparison = progression_quality_comparison(
                    declaration,
                    &transaction_directory,
                    target_rung,
                    &result,
                );
                steps.push(KiCadConnectionProgressionStep {
                    ordinal,
                    source_rung,
                    target_rung,
                    transaction_directory,
                    result: Some(result),
                    quality_comparison,
                    error: None,
                    elapsed_micros: step_started.elapsed().as_micros() as u64,
                });
                final_directory = selected_directory;
                if disposition == KiCadConnectionInsertionDisposition::Committed {
                    trusted_parent = next_trusted_parent;
                    final_rung = target_rung;
                    termination = if final_rung == declaration.connections.len() {
                        KiCadConnectionProgressionTermination::FinalRungReached
                    } else {
                        KiCadConnectionProgressionTermination::Running
                    };
                } else {
                    trusted_parent = None;
                    termination = KiCadConnectionProgressionTermination::RolledBack;
                }
            }
            Err(error) => {
                trusted_parent = None;
                steps.push(KiCadConnectionProgressionStep {
                    ordinal,
                    source_rung,
                    target_rung,
                    transaction_directory,
                    result: None,
                    quality_comparison: None,
                    error: Some(error),
                    elapsed_micros: step_started.elapsed().as_micros() as u64,
                });
                termination = KiCadConnectionProgressionTermination::ExecutionFailed;
            }
        }
        let checkpoint = build_progression_result(
            declaration,
            initial_rung,
            final_rung,
            initial_parent_directory,
            &final_directory,
            termination,
            &steps,
        );
        write_pretty_json(&output_root.join("progression.json"), &checkpoint)?;
        if matches!(
            termination,
            KiCadConnectionProgressionTermination::RolledBack
                | KiCadConnectionProgressionTermination::ExecutionFailed
                | KiCadConnectionProgressionTermination::FinalRungReached
        ) {
            break;
        }
    }

    if termination == KiCadConnectionProgressionTermination::Running {
        termination = KiCadConnectionProgressionTermination::BoundReached;
    }

    let result = build_progression_result(
        declaration,
        initial_rung,
        final_rung,
        initial_parent_directory,
        &final_directory,
        termination,
        &steps,
    );
    write_pretty_json(&output_root.join("progression.json"), &result)?;
    Ok(result)
}

fn progression_quality_comparison(
    declaration: &LadderDeclaration,
    transaction_directory: &Path,
    target_rung: usize,
    result: &KiCadConnectionInsertionResult,
) -> Option<KiCadConnectionProgressionQualityComparison> {
    if result.disposition != KiCadConnectionInsertionDisposition::Committed {
        return None;
    }
    let selected = result
        .evidence
        .attempts
        .iter()
        .find(|attempt| attempt.selected)?
        .route_quality
        .clone()?;
    let connection = &result.evidence.inserted_connection;
    let historical_board = transaction_directory
        .join("declared-target")
        .join(format!(
            "{target_rung:02}-{}",
            rung_name(target_rung, &declaration.connections)
        ))
        .join(format!("{}.kicad_pcb", declaration.board_id));
    match import_route_candidate_from_pcb(&historical_board, connection)
        .and_then(|candidate| route_candidate_quality(&candidate))
    {
        Ok(historical) => {
            let length_delta = selected.length_mm - historical.length_mm;
            let percent = if historical.length_mm > 0.0 {
                Some(length_delta / historical.length_mm * 100.0)
            } else {
                None
            };
            let via_delta = selected.vias as i64 - historical.vias as i64;
            Some(KiCadConnectionProgressionQualityComparison {
                selected,
                historical_declared: Some(historical),
                selected_minus_historical_length_mm: Some(length_delta),
                selected_minus_historical_length_percent: percent,
                selected_minus_historical_vias: Some(via_delta),
                historical_control_error: None,
                interpretation: "quality comparison only; historical geometry is not eligible for parent-preserving admission".into(),
            })
        }
        Err(error) => Some(KiCadConnectionProgressionQualityComparison {
            selected,
            historical_declared: None,
            selected_minus_historical_length_mm: None,
            selected_minus_historical_length_percent: None,
            selected_minus_historical_vias: None,
            historical_control_error: Some(error),
            interpretation: "historical route could not be imported for comparison; selected child remains independently native-gated".into(),
        }),
    }
}

fn build_progression_result(
    declaration: &LadderDeclaration,
    initial_rung: usize,
    final_rung: usize,
    initial_directory: &Path,
    final_directory: &Path,
    termination: KiCadConnectionProgressionTermination,
    steps: &[KiCadConnectionProgressionStep],
) -> KiCadConnectionProgressionResult {
    let committed_connections = steps
        .iter()
        .filter(|step| {
            step.result.as_ref().is_some_and(|result| {
                result.disposition == KiCadConnectionInsertionDisposition::Committed
            })
        })
        .count();
    let total_route_expansions = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .flat_map(|result| &result.evidence.attempts)
        .filter_map(|attempt| attempt.route_expansions)
        .map(u64::from)
        .sum();
    let total_future_reachability_evaluations = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.future_reachability_evaluations)
        .sum();
    let total_future_reachability_rejections = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.future_reachability_rejections)
        .sum();
    let total_future_reachability_expansions = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.future_reachability_expansions)
        .sum();
    let total_future_route_expansions = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.future_route_expansions)
        .sum();
    let total_local_portfolio_entries_declared = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.local_portfolio_entries_declared)
        .sum();
    let total_local_portfolio_entries_equivalent_skipped = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.local_portfolio_entries_equivalent_skipped)
        .sum();
    let total_local_portfolio_entries_available = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.local_portfolio_entries_available)
        .sum();
    let total_local_portfolio_entries_evaluated = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.local_portfolio_entries_evaluated)
        .sum();
    let total_conditional_local_entries_declared = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.conditional_local_entries_declared)
        .sum();
    let total_conditional_local_entries_activated = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.conditional_local_entries_activated)
        .sum();
    let total_conditional_local_entries_skipped = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.conditional_local_entries_skipped)
        .sum();
    let total_local_candidates_generated = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.local_candidates_generated)
        .sum();
    let total_local_native_gates = steps
        .iter()
        .flat_map(|step| step.result.iter())
        .map(|result| result.evidence.local_native_gates)
        .sum();
    KiCadConnectionProgressionResult {
        schema_version: TRANSACTION_SCHEMA_VERSION,
        board_id: declaration.board_id.clone(),
        initial_rung,
        final_rung,
        initial_directory: initial_directory.to_path_buf(),
        final_directory: final_directory.to_path_buf(),
        termination,
        committed_connections,
        attempted_transactions: steps.len(),
        elapsed_micros: steps.iter().map(|step| step.elapsed_micros).sum(),
        total_route_expansions,
        total_future_reachability_evaluations,
        total_future_reachability_rejections,
        total_future_reachability_expansions,
        total_future_route_expansions,
        total_local_portfolio_entries_declared,
        total_local_portfolio_entries_equivalent_skipped,
        total_local_portfolio_entries_available,
        total_local_portfolio_entries_evaluated,
        total_conditional_local_entries_declared,
        total_conditional_local_entries_activated,
        total_conditional_local_entries_skipped,
        total_local_candidates_generated,
        total_local_native_gates,
        steps: steps.to_vec(),
    }
}

#[derive(Clone, Debug)]
struct LocalRouteCandidate {
    attempt_index: usize,
    score_mm: f64,
    route_cost: u32,
    candidate_path: PathBuf,
    artifact_directory: PathBuf,
}

type CompleteConnectionCandidate = (usize, f64, u32, Option<PathBuf>, PathBuf);

fn ordered_refinement_improvement_percent(
    previous_best_score_mm: f64,
    latest_score_mm: f64,
) -> f64 {
    (previous_best_score_mm - latest_score_mm) / previous_best_score_mm * 100.0
}

fn ordered_refinement_stop_evidence(
    config: &KiCadOrderedLocalRefinementConfig,
    evaluated: usize,
    available: usize,
    previous_best_score_mm: Option<f64>,
    latest_score_mm: Option<f64>,
) -> Option<KiCadOrderedLocalRefinementEvidence> {
    if evaluated < config.minimum_portfolio_entries || evaluated >= available {
        return None;
    }
    let previous_best_score_mm = previous_best_score_mm?;
    let latest_score_mm = latest_score_mm?;
    if previous_best_score_mm <= 0.0 {
        return None;
    }
    let improvement_percent =
        ordered_refinement_improvement_percent(previous_best_score_mm, latest_score_mm);
    (improvement_percent < config.continue_if_score_improvement_percent_at_least).then(|| {
        KiCadOrderedLocalRefinementEvidence {
            evaluated_before_stop: evaluated,
            portfolio_entries_available: available,
            previous_best_score_mm,
            latest_score_mm,
            improvement_percent,
            continue_threshold_percent: config.continue_if_score_improvement_percent_at_least,
            resumed_after_native_rejection: false,
            interpretation: "ordered live prefix converged; skipped refinements remain recoverable until native admission succeeds".into(),
        }
    })
}

fn should_restore_skipped_refinements(
    complete_candidates: usize,
    evaluated: usize,
    available: usize,
) -> bool {
    complete_candidates == 0 && evaluated < available
}

fn effective_local_portfolio_indices(
    config: &KiCadConnectionInsertionConfig,
    terminal_count: usize,
    alignment_model: Option<&KiCadRoutingModel>,
) -> Result<Vec<usize>, String> {
    let declared = config
        .local_candidate_cap
        .min(config.local_routing_portfolio.len());
    let mut fingerprints = BTreeSet::new();
    let mut indices = Vec::new();
    for (index, routing) in config
        .local_routing_portfolio
        .iter()
        .take(declared)
        .enumerate()
    {
        let mut effective = routing.clone();
        if let Some(model) = alignment_model {
            let margin = effective.edge_clearance_mm + effective.trace_width_mm / 2.0;
            let origin = [model.bounds[0] + margin, model.bounds[1] + margin];
            let (aligned, _) = grid_alignment::aligned_origin(model, &effective, origin);
            if aligned == origin {
                effective.grid_alignment = KiCadGridAlignment::BoardOrigin;
            }
        }
        if terminal_count <= 2 {
            effective.multi_terminal_routing = KiCadMultiTerminalRoutingPolicy::RootedStar;
            effective.tree_attachment_search = KiCadTreeAttachmentSearch::RankedSamples;
            effective.maximum_tree_attachment_searches = 1;
            effective.tree_attachment_minimum_spacing_mm = 0.0;
            effective.tree_attachment_objective = KiCadTreeAttachmentObjective::LengthThenVias;
        }
        if fingerprints.insert(json_sha256(&effective)?) {
            indices.push(index);
        }
    }
    Ok(indices)
}

struct LocalRouteGenerationContext<'a> {
    hydrated_base: &'a Path,
    declared_target_report: &'a MaterializationReport,
    declaration: &'a LadderDeclaration,
    inserted_connection: &'a str,
    output_root: &'a Path,
    via_penalty_mm: f64,
}

fn generate_local_route_attempt(
    context: &LocalRouteGenerationContext<'_>,
    portfolio_index: usize,
    ordinal: usize,
    scope: KiCadConnectionInsertionAttemptScope,
    routing_config: &KiCadGridRouteConfig,
) -> Result<(KiCadConnectionInsertionAttempt, Option<LocalRouteCandidate>), String> {
    let attempt_started = Instant::now();
    let kind = match scope {
        KiCadConnectionInsertionAttemptScope::InsertedConnectionOnly => "local",
        KiCadConnectionInsertionAttemptScope::ConditionalLocalRefinement => "conditional-local",
        _ => return Err("local route generation received a non-local attempt scope".into()),
    };
    let trial = context
        .output_root
        .join(format!("attempt-{ordinal:03}-{kind}-{portfolio_index:03}"));
    copy_directory_tree(context.hydrated_base, &trial)?;
    write_materialization_report_for_directory(
        context.declared_target_report,
        &trial,
        &trial.join("materialization.json"),
    )?;
    let mut attempt = KiCadConnectionInsertionAttempt {
        ordinal,
        scope,
        status: KiCadConnectionInsertionAttemptStatus::RouteFailed,
        artifact_directory: trial.clone(),
        routing_config_sha256: Some(json_sha256(routing_config)?),
        route_cost: None,
        route_expansions: None,
        route_failure: None,
        route_elapsed_micros: 0,
        route_quality: None,
        score_mm: None,
        native_admission_order: None,
        native_gate_invoked: false,
        native_gate_elapsed_micros: 0,
        verification: None,
        future_reachability: None,
        single_connection_ripup: None,
        error: None,
        selected: false,
        elapsed_micros: 0,
    };
    let board_path = trial.join(format!("{}.kicad_pcb", context.declaration.board_id));
    let route_started = Instant::now();
    let local_candidate = match route_materialized_connection_detailed(
        &board_path,
        context.inserted_connection,
        routing_config,
    ) {
        Ok(candidate) => {
            attempt.route_cost = Some(candidate.cost);
            attempt.route_expansions = Some(candidate.expansions);
            let candidate_path = trial.join("inserted-route.json");
            write_pretty_json(&candidate_path, &candidate)?;
            match route_candidate_quality(&candidate) {
                Ok(quality) => {
                    let score_mm = quality.length_mm + quality.vias as f64 * context.via_penalty_mm;
                    attempt.status = KiCadConnectionInsertionAttemptStatus::CandidateGenerated;
                    attempt.route_quality = Some(quality);
                    attempt.score_mm = Some(score_mm);
                    Some(LocalRouteCandidate {
                        attempt_index: ordinal,
                        score_mm,
                        route_cost: candidate.cost,
                        candidate_path,
                        artifact_directory: trial.clone(),
                    })
                }
                Err(error) => {
                    attempt.status = KiCadConnectionInsertionAttemptStatus::ExecutionFailed;
                    attempt.error = Some(error);
                    None
                }
            }
        }
        Err(error) => {
            attempt.route_failure = error.search_failure().cloned();
            attempt.route_expansions = attempt
                .route_failure
                .as_ref()
                .map(|failure| failure.astar_expansions);
            attempt.error = Some(error.to_string());
            None
        }
    };
    attempt.route_elapsed_micros = route_started.elapsed().as_micros() as u64;
    attempt.elapsed_micros = attempt_started.elapsed().as_micros() as u64;
    write_pretty_json(&trial.join("attempt.json"), &attempt)?;
    Ok((attempt, local_candidate))
}

fn evaluate_future_connection_reachability(
    declaration: &LadderDeclaration,
    current_rung: usize,
    candidate_artifact_directory: &Path,
    evidence_directory: &Path,
    config: &KiCadFutureReachabilityConfig,
) -> Result<KiCadFutureReachabilityEvidence, String> {
    let started = Instant::now();
    let available_connections = declaration.connections.len().saturating_sub(current_rung);
    let checked_connections = available_connections.min(config.maximum_connections);
    fs::create_dir_all(evidence_directory).map_err(|error| {
        format!(
            "failed to create future-reachability directory {}: {error}",
            evidence_directory.display()
        )
    })?;
    let current_board =
        candidate_artifact_directory.join(format!("{}.kicad_pcb", declaration.board_id));
    let mut trials = Vec::with_capacity(checked_connections);
    for (ordinal, declaration_index) in
        (current_rung..current_rung + checked_connections).enumerate()
    {
        let trial_started = Instant::now();
        let connection = declaration.connections[declaration_index].clone();
        let rung = declaration_index + 1;
        let trial_root = evidence_directory.join(format!(
            "trial-{ordinal:03}-rung-{rung:02}-{}",
            filesystem_label(&connection)
        ));
        let report = materialize_rung(declaration, rung, &trial_root.join("declared-target"))?;
        let hydrated = trial_root.join("hydrated");
        copy_directory_tree(&report.output_directory, &hydrated)?;
        let hydrated_board = hydrated.join(format!("{}.kicad_pcb", declaration.board_id));
        hydrate_target_board_from_parent(
            &current_board,
            &report
                .output_directory
                .join(format!("{}.kicad_pcb", declaration.board_id)),
            &hydrated_board,
        )?;
        write_materialization_report_for_directory(
            &report,
            &hydrated,
            &hydrated.join("materialization.json"),
        )?;
        let trial =
            match route_materialized_connection(&hydrated_board, &connection, &config.routing) {
                Ok(candidate) => {
                    let candidate_path = hydrated.join("future-route.json");
                    write_pretty_json(&candidate_path, &candidate)?;
                    KiCadFutureConnectionReachabilityTrial {
                        ordinal,
                        declaration_index,
                        rung,
                        connection,
                        route_found: true,
                        reachability_rejected: false,
                        reachability_expansions: candidate.reachability_expansions,
                        route_expansions: Some(candidate.expansions),
                        candidate: Some(candidate_path),
                        error: None,
                        elapsed_micros: trial_started.elapsed().as_micros() as u64,
                    }
                }
                Err(error) => {
                    let route_expansions = route_failure_expansions(&error);
                    let reachability_expansions = route_failure_reachability_expansions(&error);
                    KiCadFutureConnectionReachabilityTrial {
                        ordinal,
                        declaration_index,
                        rung,
                        connection,
                        route_found: false,
                        reachability_rejected: config.routing.reachability_preflight
                            && route_expansions == Some(0),
                        reachability_expansions,
                        route_expansions,
                        candidate: None,
                        error: Some(error),
                        elapsed_micros: trial_started.elapsed().as_micros() as u64,
                    }
                }
            };
        write_pretty_json(&trial_root.join("trial.json"), &trial)?;
        trials.push(trial);
    }
    let reachable_connections = trials.iter().filter(|trial| trial.route_found).count();
    let unreachable_connections = checked_connections.saturating_sub(reachable_connections);
    let evidence = KiCadFutureReachabilityEvidence {
        schema_version: 1,
        artifact_directory: evidence_directory.to_path_buf(),
        policy: config.policy,
        maximum_connections: config.maximum_connections,
        available_connections,
        checked_connections,
        reachable_connections,
        unreachable_connections,
        all_checked_reachable: unreachable_connections == 0,
        total_reachability_expansions: trials
            .iter()
            .map(|trial| u64::from(trial.reachability_expansions))
            .sum(),
        total_route_expansions: trials
            .iter()
            .filter_map(|trial| trial.route_expansions)
            .map(u64::from)
            .sum(),
        elapsed_micros: started.elapsed().as_micros() as u64,
        trials,
        interpretation: "counterfactual only: each still-enabled connection is routed independently against the current native-complete child; future routes are neither applied nor treated as KiCad admission".into(),
    };
    write_pretty_json(
        &evidence_directory.join("future-reachability.json"),
        &evidence,
    )?;
    Ok(evidence)
}

fn future_reachability_admits(evidence: &KiCadFutureReachabilityEvidence) -> bool {
    evidence.policy != KiCadFutureReachabilityPolicy::RequireAll || evidence.all_checked_reachable
}

#[allow(clippy::too_many_arguments)]
fn admit_ungated_local_candidates(
    local_candidates: &[LocalRouteCandidate],
    attempts: &mut [KiCadConnectionInsertionAttempt],
    complete_candidates: &mut Vec<CompleteConnectionCandidate>,
    declaration: &LadderDeclaration,
    policy: KiCadLocalNativeAdmission,
    current_rung: usize,
    lookahead_root: &Path,
    future_reachability: Option<&KiCadFutureReachabilityConfig>,
) -> Result<(), String> {
    let mut admission_order = local_native_admission_order(local_candidates, policy);
    admission_order.retain(|index| {
        attempts[local_candidates[*index].attempt_index]
            .native_admission_order
            .is_none()
    });
    let admission_start = attempts
        .iter()
        .filter_map(|attempt| attempt.native_admission_order)
        .max()
        .map_or(0, |rank| rank + 1);
    let complete_before = complete_candidates.len();
    for (admission_offset, local_index) in admission_order.into_iter().enumerate() {
        let admission_rank = admission_start + admission_offset;
        let local = &local_candidates[local_index];
        let gate_started = Instant::now();
        let attempt = &mut attempts[local.attempt_index];
        attempt.native_admission_order = Some(admission_rank);
        let board_path = local
            .artifact_directory
            .join(format!("{}.kicad_pcb", declaration.board_id));
        match apply_route_candidate(&board_path, &local.candidate_path, &board_path) {
            Err(error) => {
                attempt.status = KiCadConnectionInsertionAttemptStatus::ExecutionFailed;
                attempt.error = Some(error);
            }
            Ok(()) => {
                attempt.native_gate_invoked = true;
                match verify_materialized_rung(&local.artifact_directory, &declaration.board_id) {
                    Err(error) => {
                        attempt.status = KiCadConnectionInsertionAttemptStatus::ExecutionFailed;
                        attempt.error = Some(error);
                    }
                    Ok(verification) => {
                        attempt.verification = Some(verification.clone());
                        if verification.complete {
                            let future = future_reachability
                                .map(|config| {
                                    evaluate_future_connection_reachability(
                                        declaration,
                                        current_rung,
                                        &local.artifact_directory,
                                        &lookahead_root.join(format!(
                                            "future-reachability-attempt-{:03}",
                                            local.attempt_index
                                        )),
                                        config,
                                    )
                                })
                                .transpose()?;
                            let admitted = future.as_ref().is_none_or(future_reachability_admits);
                            attempt.future_reachability = future;
                            if admitted {
                                attempt.status = KiCadConnectionInsertionAttemptStatus::Committed;
                                complete_candidates.push((
                                    local.attempt_index,
                                    local.score_mm,
                                    local.route_cost,
                                    Some(local.candidate_path.clone()),
                                    local.artifact_directory.clone(),
                                ));
                            } else {
                                attempt.status =
                                    KiCadConnectionInsertionAttemptStatus::FutureReachabilityRejected;
                                attempt.error = Some(
                                    "native-complete child failed required bounded future-connection reachability".into(),
                                );
                            }
                        } else {
                            attempt.status = KiCadConnectionInsertionAttemptStatus::NativeRejected;
                        }
                    }
                }
            }
        }
        let gate_elapsed = gate_started.elapsed().as_micros() as u64;
        attempt.native_gate_elapsed_micros = attempt
            .native_gate_elapsed_micros
            .saturating_add(gate_elapsed);
        attempt.elapsed_micros = attempt.elapsed_micros.saturating_add(gate_elapsed);
        write_pretty_json(
            &local.artifact_directory.join("attempt.json"),
            &attempts[local.attempt_index],
        )?;
        if policy == KiCadLocalNativeAdmission::ScoreOrderedUntilComplete
            && complete_candidates.len() > complete_before
            && future_reachability
                .is_none_or(|config| config.policy == KiCadFutureReachabilityPolicy::RequireAll)
        {
            break;
        }
    }
    Ok(())
}

fn ranked_local_candidate_indices(candidates: &[LocalRouteCandidate]) -> Vec<usize> {
    let mut order: Vec<_> = (0..candidates.len()).collect();
    order.sort_by(|left, right| {
        let left = &candidates[*left];
        let right = &candidates[*right];
        left.score_mm
            .total_cmp(&right.score_mm)
            .then_with(|| left.route_cost.cmp(&right.route_cost))
            .then_with(|| left.attempt_index.cmp(&right.attempt_index))
    });
    order
}

fn best_local_candidate_observation(
    candidates: &[LocalRouteCandidate],
    attempts: &[KiCadConnectionInsertionAttempt],
) -> Option<(f64, usize)> {
    let best = *ranked_local_candidate_indices(candidates).first()?;
    let candidate = &candidates[best];
    let quality = attempts
        .get(candidate.attempt_index)?
        .route_quality
        .as_ref()?;
    Some((candidate.score_mm, quality.vias))
}

fn budget_exhausted_attempt_ordinals(attempts: &[KiCadConnectionInsertionAttempt]) -> Vec<usize> {
    attempts
        .iter()
        .filter(|attempt| {
            attempt.status == KiCadConnectionInsertionAttemptStatus::RouteFailed
                && matches!(
                    attempt.scope,
                    KiCadConnectionInsertionAttemptScope::InsertedConnectionOnly
                        | KiCadConnectionInsertionAttemptScope::ConditionalLocalRefinement
                )
                && attempt.route_failure.as_ref().is_some_and(|failure| {
                    failure.kind == KiCadRouteSearchFailureKind::SearchBudgetExhausted
                })
        })
        .map(|attempt| attempt.ordinal)
        .collect()
}

fn conditional_local_trigger_decision(
    trigger: &KiCadConditionalLocalRoutingTrigger,
    terminal_count: usize,
    candidates: &[LocalRouteCandidate],
    attempts: &[KiCadConnectionInsertionAttempt],
) -> (bool, Option<f64>, Option<usize>, String) {
    let observation = best_local_candidate_observation(candidates, attempts);
    if matches!(
        trigger,
        KiCadConditionalLocalRoutingTrigger::SearchBudgetExhaustedWithoutCandidate
    ) {
        let exhausted = budget_exhausted_attempt_ordinals(attempts);
        let active = candidates.is_empty() && !exhausted.is_empty();
        return (
            active,
            observation.map(|value| value.0),
            observation.map(|value| value.1),
            if !candidates.is_empty() {
                "skipped: an earlier local route candidate exists".into()
            } else if exhausted.is_empty() {
                "skipped: no earlier local search reported structured budget exhaustion".into()
            } else {
                format!(
                    "activated: no route candidate; search budget exhausted in attempts {exhausted:?}"
                )
            },
        );
    }
    let Some((score_mm, vias)) = observation else {
        return (
            false,
            None,
            None,
            "skipped: no cheaper local route candidate produced quality evidence".into(),
        );
    };
    if terminal_count <= 2 {
        return (
            false,
            Some(score_mm),
            Some(vias),
            "skipped: conditional multi-terminal topology is inactive for a two-terminal connection"
                .into(),
        );
    }
    match trigger {
        KiCadConditionalLocalRoutingTrigger::BestCandidateViasAtLeast { minimum_vias } => {
            let active = vias >= *minimum_vias;
            (
                active,
                Some(score_mm),
                Some(vias),
                if active {
                    format!(
                        "activated: cheapest generated candidate has {vias} vias, meeting threshold {minimum_vias}"
                    )
                } else {
                    format!(
                        "skipped: cheapest generated candidate has {vias} vias, below threshold {minimum_vias}"
                    )
                },
            )
        }
        KiCadConditionalLocalRoutingTrigger::SearchBudgetExhaustedWithoutCandidate => {
            unreachable!("handled before candidate quality checks")
        }
    }
}

fn local_native_admission_order(
    candidates: &[LocalRouteCandidate],
    policy: KiCadLocalNativeAdmission,
) -> Vec<usize> {
    match policy {
        KiCadLocalNativeAdmission::Exhaustive => (0..candidates.len()).collect(),
        KiCadLocalNativeAdmission::ScoreOrderedUntilComplete => {
            ranked_local_candidate_indices(candidates)
        }
    }
}

/// Adds exactly the next declared connection to an immutable, natively valid
/// KiCad ladder artifact.
///
/// The source directory is never passed to KiCad or modified. A byte-exact
/// snapshot is made first, and all native verification happens in disposable
/// copies. The target schematic/pad connectivity is materialized from the
/// declaration, while component poses and existing copper come from the exact
/// parent board. A child is committed only after KiCad ERC, DRC, schematic
/// parity, and selected-net connectivity all pass.
pub fn insert_kicad_connection(
    declaration: &LadderDeclaration,
    parent_directory: &Path,
    output_root: &Path,
    config: &KiCadConnectionInsertionConfig,
) -> Result<KiCadConnectionInsertionResult, String> {
    insert_kicad_connection_with_trusted_source(
        declaration,
        parent_directory,
        output_root,
        config,
        None,
    )
}

struct TrustedSourceVerification<'a> {
    directory_sha256: &'a str,
    report: &'a VerificationReport,
}

fn insert_kicad_connection_with_trusted_source(
    declaration: &LadderDeclaration,
    parent_directory: &Path,
    output_root: &Path,
    config: &KiCadConnectionInsertionConfig,
    trusted_source: Option<TrustedSourceVerification<'_>>,
) -> Result<KiCadConnectionInsertionResult, String> {
    let transaction_started = Instant::now();
    validate_insertion_config(config)?;
    if output_root.exists() {
        return Err(format!(
            "refusing to overwrite KiCad connection transaction {}",
            output_root.display()
        ));
    }
    if !parent_directory.is_dir() {
        return Err(format!(
            "KiCad connection parent is not a directory: {}",
            parent_directory.display()
        ));
    }

    let parent_report_path = parent_directory.join("materialization.json");
    let parent_report: MaterializationReport = read_typed_json(&parent_report_path)?;
    validate_parent_report(declaration, &parent_report)?;
    let source_rung = parent_report.rung;
    let target_rung = source_rung + 1;
    let inserted_connection = declaration
        .connections
        .get(source_rung)
        .ok_or_else(|| "the parent is already the final declared ladder rung".to_string())?
        .clone();
    let parent_pcb = parent_directory.join(format!("{}.kicad_pcb", declaration.board_id));
    reject_existing_connection_copper(&parent_pcb, &inserted_connection)?;

    let parent_before = directory_sha256(parent_directory)?;
    fs::create_dir_all(output_root).map_err(|error| {
        format!(
            "failed to create KiCad connection transaction {}: {error}",
            output_root.display()
        )
    })?;

    let source_snapshot = output_root.join("source-parent");
    copy_directory_tree(parent_directory, &source_snapshot)?;
    let source_snapshot_hash = directory_sha256(&source_snapshot)?;
    if source_snapshot_hash != parent_before {
        return Err("source snapshot is not byte-identical to the external parent".into());
    }

    let source_verification_reused = trusted_source.is_some();
    let source_verification = if let Some(trusted) = trusted_source {
        if trusted.directory_sha256 != parent_before {
            return Err(
                "trusted progression parent hash does not match its selected artifact".into(),
            );
        }
        if trusted.report.board_id != declaration.board_id || !trusted.report.complete {
            return Err(
                "trusted progression parent report is not a complete matching board".into(),
            );
        }
        trusted.report.clone()
    } else {
        let source_verification_directory = output_root.join("source-verification");
        copy_directory_tree(&source_snapshot, &source_verification_directory)?;
        verify_materialized_rung(&source_verification_directory, &declaration.board_id)?
    };
    if !source_verification.complete {
        return Err(format!(
            "source rung {source_rung} is not natively complete: {}",
            verification_summary(&source_verification)
        ));
    }

    let declared_target_root = output_root.join("declared-target");
    let declared_target_report = materialize_rung(declaration, target_rung, &declared_target_root)?;
    let declared_target_directory = declared_target_report.output_directory.clone();

    let hydrated_base = output_root.join("unrouted-target");
    copy_directory_tree(&declared_target_directory, &hydrated_base)?;
    hydrate_target_board_from_parent(
        &parent_pcb,
        &declared_target_directory.join(format!("{}.kicad_pcb", declaration.board_id)),
        &hydrated_base.join(format!("{}.kicad_pcb", declaration.board_id)),
    )?;
    write_materialization_report_for_directory(
        &declared_target_report,
        &hydrated_base,
        &hydrated_base.join("materialization.json"),
    )?;

    let mut unrouted_target_verification = match config.unrouted_target_gate {
        KiCadUnroutedTargetGate::Eager => Some(verify_unrouted_target(
            &hydrated_base,
            output_root,
            &declaration.board_id,
        )?),
        KiCadUnroutedTargetGate::BeforeFallbackOrRollback => None,
    };

    let hydrated_board = hydrated_base.join(format!("{}.kicad_pcb", declaration.board_id));
    // Resolve board-dependent grid equivalence before rasterization/search.
    // Most portfolios do not request alignment and retain their original path.
    let alignment_model = if config
        .local_routing_portfolio
        .iter()
        .take(config.local_candidate_cap)
        .any(|r| r.grid_alignment != KiCadGridAlignment::BoardOrigin)
    {
        let source = fs::read_to_string(&hydrated_board)
            .map_err(|error| format!("failed to read {}: {error}", hydrated_board.display()))?;
        Some(KiCadRoutingModel::from_pcb(
            &parse(&source)?,
            &inserted_connection,
        )?)
    } else {
        None
    };
    let inserted_connection_terminal_count = if let Some(model) = &alignment_model {
        model.electrical_terminals().len()
    } else {
        materialized_connection_terminal_count(&hydrated_board, &inserted_connection)?
    };
    let local_portfolio_entries_declared = config
        .local_candidate_cap
        .min(config.local_routing_portfolio.len());
    let local_portfolio_indices = effective_local_portfolio_indices(
        config,
        inserted_connection_terminal_count,
        alignment_model.as_ref(),
    )?;
    let local_portfolio_entries_available = local_portfolio_indices.len();
    let local_portfolio_entries_equivalent_skipped =
        local_portfolio_entries_declared.saturating_sub(local_portfolio_entries_available);
    let conditional_local_entries_declared = config
        .conditional_local_candidate_cap
        .min(config.conditional_local_routing_portfolio.len());
    let mut attempts = Vec::new();
    let mut local_candidates = Vec::new();
    let mut conditional_local_routing = Vec::new();
    let mut candidates: Vec<CompleteConnectionCandidate> = Vec::new();
    let mut ordered_local_refinement = None;
    let mut best_score_mm: Option<f64> = None;
    let mut next_portfolio_index = 0;
    let local_generation = LocalRouteGenerationContext {
        hydrated_base: &hydrated_base,
        declared_target_report: &declared_target_report,
        declaration,
        inserted_connection: &inserted_connection,
        output_root,
        via_penalty_mm: config.via_penalty_mm,
    };
    while next_portfolio_index < local_portfolio_entries_available {
        let portfolio_index = local_portfolio_indices[next_portfolio_index];
        let ordinal = attempts.len();
        let (attempt, local_candidate) = generate_local_route_attempt(
            &local_generation,
            portfolio_index,
            ordinal,
            KiCadConnectionInsertionAttemptScope::InsertedConnectionOnly,
            &config.local_routing_portfolio[portfolio_index],
        )?;
        next_portfolio_index += 1;
        attempts.push(attempt);
        let Some(local_candidate) = local_candidate else {
            continue;
        };
        let latest_score_mm = local_candidate.score_mm;
        local_candidates.push(local_candidate);
        let previous_best_score_mm = best_score_mm;
        best_score_mm =
            Some(best_score_mm.map_or(latest_score_mm, |best| best.min(latest_score_mm)));
        if let Some(evidence) = config
            .ordered_local_refinement
            .as_ref()
            .and_then(|refinement| {
                ordered_refinement_stop_evidence(
                    refinement,
                    next_portfolio_index,
                    local_portfolio_entries_available,
                    previous_best_score_mm,
                    Some(latest_score_mm),
                )
            })
        {
            ordered_local_refinement = Some(evidence);
            break;
        }
    }

    let mut seen_routing_configs = config
        .local_routing_portfolio
        .iter()
        .take(local_portfolio_entries_declared)
        .map(json_sha256)
        .collect::<Result<BTreeSet<_>, _>>()?;
    for (conditional_index, conditional) in config
        .conditional_local_routing_portfolio
        .iter()
        .take(conditional_local_entries_declared)
        .enumerate()
    {
        let routing_config_sha256 = json_sha256(&conditional.routing)?;
        let (mut activated, observed_best_score_mm, observed_best_vias, mut interpretation) =
            conditional_local_trigger_decision(
                &conditional.trigger,
                inserted_connection_terminal_count,
                &local_candidates,
                &attempts,
            );
        if activated && !seen_routing_configs.insert(routing_config_sha256.clone()) {
            activated = false;
            interpretation =
                "skipped: conditional routing configuration duplicates an earlier route mode"
                    .into();
        }
        let mut evidence = KiCadConditionalLocalRoutingEvidence {
            ordinal: conditional_index,
            trigger: conditional.trigger.clone(),
            routing_config_sha256,
            observed_best_score_mm,
            observed_best_vias,
            observed_budget_exhausted_attempts: matches!(
                conditional.trigger,
                KiCadConditionalLocalRoutingTrigger::SearchBudgetExhaustedWithoutCandidate
            )
            .then(|| budget_exhausted_attempt_ordinals(&attempts)),
            activated,
            attempt_ordinal: None,
            interpretation,
        };
        if activated {
            let ordinal = attempts.len();
            let (attempt, local_candidate) = generate_local_route_attempt(
                &local_generation,
                conditional_index,
                ordinal,
                KiCadConnectionInsertionAttemptScope::ConditionalLocalRefinement,
                &conditional.routing,
            )?;
            evidence.attempt_ordinal = Some(ordinal);
            attempts.push(attempt);
            if let Some(local_candidate) = local_candidate {
                local_candidates.push(local_candidate);
            }
        }
        conditional_local_routing.push(evidence);
    }

    admit_ungated_local_candidates(
        &local_candidates,
        &mut attempts,
        &mut candidates,
        declaration,
        config.local_native_admission,
        target_rung,
        output_root,
        config.future_reachability.as_ref(),
    )?;

    // A convergence hint may reduce ordinary work, but it must never remove
    // routes from the recovery set. Restore the skipped suffix before any
    // topology-changing fallback when the generated prefix fails KiCad.
    if should_restore_skipped_refinements(
        candidates.len(),
        next_portfolio_index,
        local_portfolio_entries_available,
    ) {
        if let Some(evidence) = ordered_local_refinement.as_mut() {
            evidence.resumed_after_native_rejection = true;
            evidence.interpretation = "ordered live prefix converged but every prefix candidate failed native admission; restored the complete declared portfolio before topology fallback".into();
        }
        while next_portfolio_index < local_portfolio_entries_available {
            let portfolio_index = local_portfolio_indices[next_portfolio_index];
            let ordinal = attempts.len();
            let (attempt, local_candidate) = generate_local_route_attempt(
                &local_generation,
                portfolio_index,
                ordinal,
                KiCadConnectionInsertionAttemptScope::InsertedConnectionOnly,
                &config.local_routing_portfolio[portfolio_index],
            )?;
            next_portfolio_index += 1;
            attempts.push(attempt);
            if let Some(local_candidate) = local_candidate {
                local_candidates.push(local_candidate);
            }
        }
        admit_ungated_local_candidates(
            &local_candidates,
            &mut attempts,
            &mut candidates,
            declaration,
            config.local_native_admission,
            target_rung,
            output_root,
            config.future_reachability.as_ref(),
        )?;
    }

    if candidates.is_empty()
        && let Some(ripup_config) = config.single_connection_ripup.as_ref()
    {
        let ordinal = attempts.len();
        let trial = output_root.join(format!("attempt-{ordinal:03}-single-connection-ripup"));
        let attempt_started = Instant::now();
        let mut attempt = KiCadConnectionInsertionAttempt {
            ordinal,
            scope: KiCadConnectionInsertionAttemptScope::SingleConnectionRipup,
            status: KiCadConnectionInsertionAttemptStatus::ExecutionFailed,
            artifact_directory: trial.clone(),
            routing_config_sha256: Some(json_sha256(ripup_config)?),
            route_cost: None,
            route_expansions: None,
            route_failure: None,
            route_elapsed_micros: 0,
            route_quality: None,
            score_mm: None,
            native_admission_order: None,
            native_gate_invoked: false,
            native_gate_elapsed_micros: 0,
            verification: None,
            future_reachability: None,
            single_connection_ripup: None,
            error: None,
            selected: false,
            elapsed_micros: 0,
        };
        match repair_kicad_with_single_connection_ripup(
            &hydrated_base,
            &declaration.board_id,
            &inserted_connection,
            &trial,
            ripup_config,
        ) {
            Ok(mut repair) => {
                if unrouted_target_verification.is_none() {
                    unrouted_target_verification = Some(repair.baseline_verification.clone());
                }
                if let Some(future_config) = config.future_reachability.as_ref() {
                    for repair_attempt in repair.attempts.iter_mut().filter(|repair_attempt| {
                        repair_attempt.status == KiCadSingleConnectionRipupStatus::Complete
                    }) {
                        repair_attempt.future_reachability =
                            Some(evaluate_future_connection_reachability(
                                declaration,
                                target_rung,
                                &repair_attempt.artifact_directory,
                                &output_root.join(format!(
                                    "future-reachability-attempt-{ordinal:03}-ripup-{:03}",
                                    repair_attempt.ordinal
                                )),
                                future_config,
                            )?);
                    }
                    let mut ranked_repairs: Vec<_> = repair
                        .attempts
                        .iter()
                        .enumerate()
                        .filter(|(_, repair_attempt)| {
                            repair_attempt.status == KiCadSingleConnectionRipupStatus::Complete
                        })
                        .map(|(index, repair_attempt)| {
                            (
                                index,
                                repair_attempt
                                    .future_reachability
                                    .as_ref()
                                    .map_or(0, |evidence| evidence.unreachable_connections),
                                repair_attempt.combined_score_mm.unwrap_or(f64::INFINITY),
                                repair_attempt.ordinal,
                            )
                        })
                        .collect();
                    ranked_repairs.sort_by(|left, right| {
                        left.1
                            .cmp(&right.1)
                            .then_with(|| left.2.total_cmp(&right.2))
                            .then_with(|| left.3.cmp(&right.3))
                    });
                    repair.selected_attempt = ranked_repairs.first().map(|ranked| ranked.0);
                    repair.selected_directory = repair
                        .selected_attempt
                        .map(|selected| repair.attempts[selected].artifact_directory.clone());
                    for repair_attempt in &mut repair.attempts {
                        repair_attempt.selected = false;
                    }
                    if let Some(selected) = repair.selected_attempt {
                        repair.attempts[selected].selected = true;
                    }
                    repair.selection_rule = format!(
                        "native completeness, then fewer unreachable bounded future connections, target + rerouted physical copper + {:.6} mm per via, attempt ordinal",
                        ripup_config.via_penalty_mm
                    );
                    for repair_attempt in &repair.attempts {
                        write_pretty_json(
                            &repair_attempt.artifact_directory.join("attempt.json"),
                            repair_attempt,
                        )?;
                    }
                    write_pretty_json(&trial.join("ripup.json"), &repair)?;
                }
                if let Some(selected) = repair.selected_attempt {
                    let selected_repair = &repair.attempts[selected];
                    let selected_directory = repair
                        .selected_directory
                        .clone()
                        .ok_or_else(|| "selected rip-up action has no directory".to_string())?;
                    let score = selected_repair
                        .combined_score_mm
                        .ok_or_else(|| "selected rip-up action has no score".to_string())?;
                    let verification = selected_repair
                        .final_verification
                        .clone()
                        .ok_or_else(|| "selected rip-up action has no native gate".to_string())?;
                    attempt.score_mm = Some(score);
                    attempt.verification = Some(verification);
                    attempt.route_quality = Some(selected_repair.target_quality.clone());
                    attempt.route_cost = Some(
                        selected_repair
                            .target_route_cost
                            .saturating_add(selected_repair.rerouted_route_cost.unwrap_or(0)),
                    );
                    attempt.route_expansions =
                        Some(repair.total_route_expansions.min(u64::from(u32::MAX)) as u32);
                    let future = selected_repair.future_reachability.clone();
                    let admitted = future.as_ref().is_none_or(future_reachability_admits);
                    attempt.future_reachability = future;
                    if admitted {
                        attempt.status = KiCadConnectionInsertionAttemptStatus::Committed;
                        candidates.push((ordinal, score, 0, None, selected_directory));
                    } else {
                        attempt.status =
                            KiCadConnectionInsertionAttemptStatus::FutureReachabilityRejected;
                        attempt.error = Some(
                            "native-complete rip-up child failed required bounded future-connection reachability".into(),
                        );
                    }
                } else {
                    attempt.status = KiCadConnectionInsertionAttemptStatus::NativeRejected;
                    attempt.error = Some(
                        "no single-connection rip-up action restored a native-complete board"
                            .into(),
                    );
                }
                attempt.single_connection_ripup = Some(Box::new(repair));
            }
            Err(error) => {
                fs::create_dir_all(&trial).map_err(|create_error| {
                    format!(
                        "failed to retain rip-up execution failure {}: {create_error}",
                        trial.display()
                    )
                })?;
                attempt.error = Some(error);
            }
        }
        attempt.elapsed_micros = attempt_started.elapsed().as_micros() as u64;
        write_pretty_json(&trial.join("attempt.json"), &attempt)?;
        attempts.push(attempt);
    }

    if candidates.is_empty() && unrouted_target_verification.is_none() {
        unrouted_target_verification = Some(verify_unrouted_target(
            &hydrated_base,
            output_root,
            &declaration.board_id,
        )?);
    }

    if candidates.is_empty()
        && config.fallback == KiCadConnectionInsertionFallback::DeclaredTargetControl
    {
        let ordinal = attempts.len();
        let trial = output_root.join(format!("attempt-{ordinal:03}-declared-target-control"));
        let attempt_started = Instant::now();
        copy_directory_tree(&declared_target_directory, &trial)?;
        write_materialization_report_for_directory(
            &declared_target_report,
            &trial,
            &trial.join("materialization.json"),
        )?;
        let mut attempt = KiCadConnectionInsertionAttempt {
            ordinal,
            scope: KiCadConnectionInsertionAttemptScope::DeclaredTargetControl,
            status: KiCadConnectionInsertionAttemptStatus::ExecutionFailed,
            artifact_directory: trial.clone(),
            routing_config_sha256: None,
            route_cost: None,
            route_expansions: None,
            route_failure: None,
            route_elapsed_micros: 0,
            route_quality: None,
            score_mm: None,
            native_admission_order: None,
            native_gate_invoked: false,
            native_gate_elapsed_micros: 0,
            verification: None,
            future_reachability: None,
            single_connection_ripup: None,
            error: None,
            selected: false,
            elapsed_micros: 0,
        };
        attempt.native_gate_invoked = true;
        let gate_started = Instant::now();
        match verify_materialized_rung(&trial, &declaration.board_id) {
            Ok(verification) => {
                attempt.verification = Some(verification.clone());
                if verification.complete {
                    match import_route_candidate_from_pcb(
                        &trial.join(format!("{}.kicad_pcb", declaration.board_id)),
                        &inserted_connection,
                    )
                    .and_then(|candidate| {
                        let quality = route_candidate_quality(&candidate)?;
                        let score = quality.length_mm + quality.vias as f64 * config.via_penalty_mm;
                        Ok((candidate, quality, score))
                    }) {
                        Ok((candidate, quality, score)) => {
                            let candidate_path = trial.join("declared-target-route.json");
                            write_pretty_json(&candidate_path, &candidate)?;
                            // This proves that the declaration still contains a
                            // solvable historical target, but it is not a
                            // parent-preserving transformation. Never admit it
                            // as the selected child.
                            attempt.status = KiCadConnectionInsertionAttemptStatus::ControlComplete;
                            attempt.route_cost = Some(candidate.cost);
                            attempt.route_expansions = Some(candidate.expansions);
                            attempt.route_quality = Some(quality);
                            attempt.score_mm = Some(score);
                        }
                        Err(error) => attempt.error = Some(error),
                    }
                } else {
                    attempt.status = KiCadConnectionInsertionAttemptStatus::NativeRejected;
                }
            }
            Err(error) => attempt.error = Some(error),
        }
        attempt.native_gate_elapsed_micros = gate_started.elapsed().as_micros() as u64;
        attempt.elapsed_micros = attempt_started.elapsed().as_micros() as u64;
        write_pretty_json(&trial.join("attempt.json"), &attempt)?;
        attempts.push(attempt);
    }

    candidates.sort_by(|left, right| {
        let left_unreachable = attempts[left.0]
            .future_reachability
            .as_ref()
            .map_or(0, |evidence| evidence.unreachable_connections);
        let right_unreachable = attempts[right.0]
            .future_reachability
            .as_ref()
            .map_or(0, |evidence| evidence.unreachable_connections);
        left_unreachable
            .cmp(&right_unreachable)
            .then_with(|| left.1.total_cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.0.cmp(&right.0))
    });
    let (disposition, selected_directory, selected_candidate, rollback_exact) =
        if let Some((selected_attempt, _, _, candidate_path, selected_artifact)) =
            candidates.first()
        {
            attempts[*selected_attempt].selected = true;
            (
                KiCadConnectionInsertionDisposition::Committed,
                selected_artifact.clone(),
                candidate_path.clone(),
                false,
            )
        } else {
            let rollback = output_root.join("rollback");
            copy_directory_tree(&source_snapshot, &rollback)?;
            let exact = directory_sha256(&rollback)? == source_snapshot_hash;
            if !exact {
                return Err(
                    "rollback artifact is not byte-identical to the source snapshot".into(),
                );
            }
            (
                KiCadConnectionInsertionDisposition::RolledBack,
                rollback,
                None,
                true,
            )
        };

    // Persist the final selected flags in each retained attempt after ranking.
    for attempt in &attempts {
        write_pretty_json(&attempt.artifact_directory.join("attempt.json"), attempt)?;
    }
    let parent_after = directory_sha256(parent_directory)?;
    let parent_unchanged = parent_after == parent_before;
    if !parent_unchanged {
        return Err("external parent changed during the connection transaction".into());
    }
    let selected_artifact_hash = directory_sha256(&selected_directory)?;
    let local_candidates_generated = attempts
        .iter()
        .filter(|attempt| {
            matches!(
                attempt.scope,
                KiCadConnectionInsertionAttemptScope::InsertedConnectionOnly
                    | KiCadConnectionInsertionAttemptScope::ConditionalLocalRefinement
            ) && attempt.route_quality.is_some()
        })
        .count();
    let local_portfolio_entries_evaluated = next_portfolio_index;
    let conditional_local_entries_activated = conditional_local_routing
        .iter()
        .filter(|evidence| evidence.activated)
        .count();
    let conditional_local_entries_skipped =
        conditional_local_entries_declared.saturating_sub(conditional_local_entries_activated);
    let local_native_gates = attempts
        .iter()
        .filter(|attempt| {
            matches!(
                attempt.scope,
                KiCadConnectionInsertionAttemptScope::InsertedConnectionOnly
                    | KiCadConnectionInsertionAttemptScope::ConditionalLocalRefinement
            ) && attempt.native_gate_invoked
        })
        .count();
    let local_candidates_not_native_gated =
        local_candidates_generated.saturating_sub(local_native_gates);
    let mut future_evidence = Vec::new();
    for attempt in &attempts {
        if !matches!(
            attempt.scope,
            KiCadConnectionInsertionAttemptScope::SingleConnectionRipup
                | KiCadConnectionInsertionAttemptScope::DeclaredTargetControl
        ) && let Some(evidence) = attempt.future_reachability.as_ref()
        {
            future_evidence.push(evidence);
        }
        if let Some(repair) = attempt.single_connection_ripup.as_ref() {
            future_evidence.extend(
                repair
                    .attempts
                    .iter()
                    .filter_map(|repair_attempt| repair_attempt.future_reachability.as_ref()),
            );
        }
    }
    let future_reachability_evaluations = future_evidence.len();
    let future_reachability_rejections = future_evidence
        .iter()
        .filter(|evidence| !future_reachability_admits(evidence))
        .count();
    let future_reachability_expansions = future_evidence
        .iter()
        .map(|evidence| evidence.total_reachability_expansions)
        .sum();
    let future_route_expansions = future_evidence
        .iter()
        .map(|evidence| evidence.total_route_expansions)
        .sum();
    let local_admission_rule = match config.local_native_admission {
        KiCadLocalNativeAdmission::Exhaustive => {
            "native-gate every generated local candidate before selection"
        }
        KiCadLocalNativeAdmission::ScoreOrderedUntilComplete => {
            "score generated local candidates, then native-gate until the first complete child"
        }
    };
    let evidence = KiCadConnectionInsertionEvidence {
        schema_version: TRANSACTION_SCHEMA_VERSION,
        contract: "immutable parent; one declared connection; bounded base routing then evidence-triggered conditional routing; optional parent-derived single-old-net rip-up; KiCad-native admission; optional bounded counterfactual future reachability; declared targets are controls only; exact rollback; retain every attempt and conditional decision".into(),
        board_id: declaration.board_id.clone(),
        source_rung,
        target_rung,
        inserted_connection,
        declaration_sha256: json_sha256(declaration)?,
        config_sha256: json_sha256(config)?,
        external_parent_sha256_before: parent_before,
        external_parent_sha256_after: parent_after,
        source_snapshot_sha256: source_snapshot_hash,
        selected_artifact_sha256: selected_artifact_hash,
        parent_unchanged,
        rollback_exact,
        inserted_connection_terminal_count,
        local_candidates_generated,
        local_portfolio_entries_declared,
        local_portfolio_entries_equivalent_skipped,
        local_portfolio_entries_available,
        local_portfolio_entries_evaluated,
        conditional_local_entries_declared,
        conditional_local_entries_activated,
        conditional_local_entries_skipped,
        conditional_local_routing,
        local_native_gates,
        local_candidates_not_native_gated,
        future_reachability_evaluations,
        future_reachability_rejections,
        future_reachability_expansions,
        future_route_expansions,
        ordered_local_refinement,
        source_verification,
        source_verification_reused,
        unrouted_target_gate: config.unrouted_target_gate,
        unrouted_target_verification,
        attempts,
        elapsed_micros: transaction_started.elapsed().as_micros() as u64,
        selection_rule: format!(
            "{local_admission_rule}; KiCad-native completeness{}; then producer score (local target copper or rip-up target + restored copper) with {:.6} mm per physical via, route cost, attempt ordinal",
            config.future_reachability.as_ref().map_or("", |future| match future.policy {
                KiCadFutureReachabilityPolicy::PreferMoreReachable => ", fewer unreachable bounded future connections",
                KiCadFutureReachabilityPolicy::RequireAll => ", required all bounded future connections raster-reachable",
            }),
            config.via_penalty_mm
        ),
    };
    let result = KiCadConnectionInsertionResult {
        disposition,
        selected_directory,
        selected_candidate,
        evidence,
    };
    write_pretty_json(&output_root.join("transaction.json"), &result)?;
    Ok(result)
}

fn validate_insertion_config(config: &KiCadConnectionInsertionConfig) -> Result<(), String> {
    if config
        .single_connection_ripup
        .as_ref()
        .is_some_and(|c| c.allow_partial_progress)
    {
        return Err("declared connection insertion requires a complete child; partial rip-up is a separate routing action".into());
    }
    if config.local_candidate_cap == 0 {
        return Err("local_candidate_cap must be greater than zero".into());
    }
    if config.local_routing_portfolio.is_empty() {
        return Err("local_routing_portfolio must contain at least one configuration".into());
    }
    if config.conditional_local_candidate_cap > 0
        && config.conditional_local_routing_portfolio.is_empty()
    {
        return Err(
            "conditional_local_candidate_cap requires a conditional routing configuration".into(),
        );
    }
    if config.conditional_local_candidate_cap > 0 && config.ordered_local_refinement.is_some() {
        return Err(
            "conditional local routing and ordered local refinement cannot yet be combined".into(),
        );
    }
    for conditional in config
        .conditional_local_routing_portfolio
        .iter()
        .take(config.conditional_local_candidate_cap)
    {
        match conditional.trigger {
            KiCadConditionalLocalRoutingTrigger::BestCandidateViasAtLeast { minimum_vias: 0 } => {
                return Err("conditional local via threshold must be positive".into());
            }
            KiCadConditionalLocalRoutingTrigger::BestCandidateViasAtLeast { .. } => {}
            KiCadConditionalLocalRoutingTrigger::SearchBudgetExhaustedWithoutCandidate => {}
        }
        validate_grid_route_config(&conditional.routing)?;
    }
    if !config.via_penalty_mm.is_finite() || config.via_penalty_mm < 0.0 {
        return Err("via_penalty_mm must be finite and non-negative".into());
    }
    if let Some(refinement) = &config.ordered_local_refinement {
        let available = config
            .local_candidate_cap
            .min(config.local_routing_portfolio.len());
        if refinement.minimum_portfolio_entries == 0
            || refinement.minimum_portfolio_entries > available
            || !refinement
                .continue_if_score_improvement_percent_at_least
                .is_finite()
            || refinement.continue_if_score_improvement_percent_at_least < 0.0
        {
            return Err("invalid ordered local refinement configuration".into());
        }
    }
    if let Some(future) = &config.future_reachability {
        if future.maximum_connections == 0 {
            return Err("future reachability maximum_connections must be greater than zero".into());
        }
        validate_grid_route_config(&future.routing)?;
    }
    if let Some(ripup) = &config.single_connection_ripup {
        if ripup.diagnosis.maximum_yielding_connections != 1
            || ripup.diagnosis.maximum_trials == 0
            || !ripup.via_penalty_mm.is_finite()
            || ripup.via_penalty_mm < 0.0
        {
            return Err("invalid single-connection rip-up configuration".into());
        }
        validate_grid_route_config(&ripup.diagnosis.routing)?;
    }
    Ok(())
}

fn validate_parent_report(
    declaration: &LadderDeclaration,
    report: &MaterializationReport,
) -> Result<(), String> {
    if report.board_id != declaration.board_id {
        return Err(format!(
            "parent materialization board {:?} does not match declaration {:?}",
            report.board_id, declaration.board_id
        ));
    }
    if report.rung > declaration.connections.len() {
        return Err(format!(
            "parent rung {} exceeds {} declared connections",
            report.rung,
            declaration.connections.len()
        ));
    }
    if report.selected_connections != declaration.connections[..report.rung] {
        return Err("parent materialization is not the declared connection prefix".into());
    }
    Ok(())
}

fn validate_unrouted_target_gate(report: &VerificationReport) -> Result<(), String> {
    if report.complete
        || report.erc_violations != 0
        || report.drc_design_violations != 0
        || report.schematic_parity_issues != 0
        || report.selected_net_unconnected_items == 0
    {
        return Err(format!(
            "unrouted target did not isolate a connectivity-only delta: {}",
            verification_summary(report)
        ));
    }
    Ok(())
}

fn verify_unrouted_target(
    hydrated_base: &Path,
    output_root: &Path,
    board_id: &str,
) -> Result<VerificationReport, String> {
    let verification_directory = output_root.join("unrouted-target-verification");
    copy_directory_tree(hydrated_base, &verification_directory)?;
    let report = verify_materialized_rung(&verification_directory, board_id)?;
    validate_unrouted_target_gate(&report)?;
    Ok(report)
}

fn verification_summary(report: &VerificationReport) -> String {
    format!(
        "complete={}, ERC={}, DRC={}, parity={}, unconnected={}",
        report.complete,
        report.erc_violations,
        report.drc_design_violations,
        report.schematic_parity_issues,
        report.selected_net_unconnected_items
    )
}

fn reject_existing_connection_copper(pcb_path: &Path, connection: &str) -> Result<(), String> {
    let source = fs::read_to_string(pcb_path)
        .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
    let pcb = parse(&source)?;
    if pcb.children().iter().any(|item| {
        matches!(item.head(), Some("segment" | "arc" | "via" | "zone"))
            && node_net(item).is_some_and(|net| normalize_net(net) == connection)
    }) {
        return Err(format!(
            "parent already contains copper for next connection {connection:?}"
        ));
    }
    Ok(())
}

fn hydrate_target_board_from_parent(
    parent_path: &Path,
    target_path: &Path,
    output_path: &Path,
) -> Result<(), String> {
    let parent_source = fs::read_to_string(parent_path)
        .map_err(|error| format!("failed to read {}: {error}", parent_path.display()))?;
    let target_source = fs::read_to_string(target_path)
        .map_err(|error| format!("failed to read {}: {error}", target_path.display()))?;
    let mut parent = parse(&parent_source)?;
    let target = parse(&target_source)?;
    let target_metadata = collect_pad_metadata(&target)?;
    let mut consumed = BTreeSet::new();
    let Expr::List(parent_items) = &mut parent else {
        return Err("parent PCB root is not a list".into());
    };
    for footprint in parent_items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = footprint_reference(footprint)
            .ok_or_else(|| "parent footprint has no Reference property".to_string())?;
        let Expr::List(footprint_items) = footprint else {
            unreachable!();
        };
        for pad in footprint_items
            .iter_mut()
            .filter(|item| item.head() == Some("pad"))
        {
            let Expr::List(pad_items) = pad else {
                unreachable!();
            };
            let number = pad_items
                .get(1)
                .and_then(Expr::atom)
                .ok_or_else(|| format!("footprint {reference:?} has a pad without a number"))?
                .to_string();
            let key = (reference.clone(), number);
            let metadata = target_metadata.get(&key).ok_or_else(|| {
                format!("parent pad {key:?} is absent from the declared target board")
            })?;
            pad_items
                .retain(|part| !matches!(part.head(), Some("net" | "pinfunction" | "pintype")));
            pad_items.extend(metadata.iter().cloned());
            consumed.insert(key);
        }
    }
    if consumed.len() != target_metadata.len() {
        let missing: Vec<_> = target_metadata
            .keys()
            .filter(|key| !consumed.contains(*key))
            .take(8)
            .collect();
        return Err(format!(
            "declared target contains {} pad(s) absent from the parent; first missing: {missing:?}",
            target_metadata.len() - consumed.len()
        ));
    }
    fs::write(output_path, format!("{}\n", encode(&parent)))
        .map_err(|error| format!("failed to write {}: {error}", output_path.display()))
}

fn collect_pad_metadata(root: &Expr) -> Result<BTreeMap<(String, String), Vec<Expr>>, String> {
    let mut result = BTreeMap::new();
    for footprint in root
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = footprint_reference(footprint)
            .ok_or_else(|| "target footprint has no Reference property".to_string())?;
        for pad in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            let number = pad
                .children()
                .get(1)
                .and_then(Expr::atom)
                .ok_or_else(|| format!("footprint {reference:?} has a pad without a number"))?
                .to_string();
            let metadata: Vec<_> = pad
                .children()
                .iter()
                .filter(|part| matches!(part.head(), Some("net" | "pinfunction" | "pintype")))
                .cloned()
                .collect();
            let key = (reference.clone(), number);
            if let Some(previous) = result.insert(key.clone(), metadata.clone())
                && previous != metadata
            {
                return Err(format!(
                    "duplicate target pad {key:?} has conflicting metadata"
                ));
            }
        }
    }
    Ok(result)
}

fn write_materialization_report_for_directory(
    source: &MaterializationReport,
    directory: &Path,
    path: &Path,
) -> Result<(), String> {
    let mut report = source.clone();
    report.output_directory = directory.to_path_buf();
    write_pretty_json(path, &report)
}

fn read_typed_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn json_sha256(value: &impl Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    hash.update(bytes);
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) fn directory_sha256(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_directory_files(root, root, &mut files)?;
    files.sort_by(|left, right| {
        left.0
            .as_os_str()
            .as_encoded_bytes()
            .cmp(right.0.as_os_str().as_encoded_bytes())
    });
    let mut hash = Sha256::new();
    hash.update(DIRECTORY_HASH_CONTRACT.as_bytes());
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

fn collect_directory_files(
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
        if file_type.is_dir() {
            collect_directory_files(root, &entry.path(), files)?;
        } else if file_type.is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_path_buf();
            files.push((relative, entry.path()));
        } else {
            return Err(format!(
                "directory hash rejects non-file artifact {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ripup_selection_opt_in_preserves_legacy_serialization() {
        let legacy = serde_json::to_value(KiCadSingleConnectionRipupConfig::default()).unwrap();
        assert!(legacy.get("selection_policy").is_none());
        let mut explicit = legacy.clone();
        explicit["selection_policy"] = serde_json::json!("best_score");
        let defaulted: KiCadSingleConnectionRipupConfig = serde_json::from_value(explicit).unwrap();
        assert_eq!(serde_json::to_value(defaulted).unwrap(), legacy);
        let enabled: KiCadSingleConnectionRipupConfig = serde_json::from_value(
            serde_json::json!({"selection_policy": "ordered_first_admitted"}),
        )
        .unwrap();
        assert_eq!(
            enabled.selection_policy,
            KiCadSequentialPortfolioPolicy::OrderedFirstAdmitted
        );
        let sequential: KiCadSequentialRipupConfig = serde_json::from_value(
            serde_json::json!({"selection_policy": "ordered_first_admitted"}),
        )
        .unwrap();
        assert_eq!(sequential.selection_policy, enabled.selection_policy);
        assert!(
            serde_json::to_value(KiCadSequentialRipupConfig::default())
                .unwrap()
                .get("selection_policy")
                .is_none()
        );
    }

    #[test]
    fn default_transaction_has_a_bounded_local_portfolio() {
        let config = KiCadConnectionInsertionConfig::default();
        assert_eq!(config.local_candidate_cap, 1);
        assert_eq!(config.local_routing_portfolio.len(), 1);
        assert_eq!(config.conditional_local_candidate_cap, 0);
        assert!(config.conditional_local_routing_portfolio.is_empty());
        assert_eq!(
            config.local_native_admission,
            KiCadLocalNativeAdmission::Exhaustive
        );
        assert!(config.ordered_local_refinement.is_none());
        assert!(config.future_reachability.is_none());
        assert_eq!(
            config.fallback,
            KiCadConnectionInsertionFallback::DeclaredTargetControl
        );
        validate_insertion_config(&config).unwrap();
    }

    #[test]
    fn invalid_transaction_config_is_rejected() {
        let config = KiCadConnectionInsertionConfig {
            local_candidate_cap: 0,
            ..KiCadConnectionInsertionConfig::default()
        };
        assert!(validate_insertion_config(&config).is_err());
        let config = KiCadConnectionInsertionConfig {
            via_penalty_mm: f64::NAN,
            ..KiCadConnectionInsertionConfig::default()
        };
        assert!(validate_insertion_config(&config).is_err());
        let config = KiCadConnectionInsertionConfig {
            ordered_local_refinement: Some(KiCadOrderedLocalRefinementConfig {
                minimum_portfolio_entries: 2,
                continue_if_score_improvement_percent_at_least: 3.0,
            }),
            ..KiCadConnectionInsertionConfig::default()
        };
        assert!(validate_insertion_config(&config).is_err());
        let config = KiCadConnectionInsertionConfig {
            future_reachability: Some(KiCadFutureReachabilityConfig {
                maximum_connections: 0,
                policy: KiCadFutureReachabilityPolicy::RequireAll,
                routing: KiCadGridRouteConfig::default(),
            }),
            ..KiCadConnectionInsertionConfig::default()
        };
        assert!(validate_insertion_config(&config).is_err());
        let config = KiCadConnectionInsertionConfig {
            conditional_local_candidate_cap: 1,
            ..KiCadConnectionInsertionConfig::default()
        };
        assert!(validate_insertion_config(&config).is_err());
        let config = KiCadConnectionInsertionConfig {
            conditional_local_candidate_cap: 1,
            conditional_local_routing_portfolio: vec![KiCadConditionalLocalRoutingConfig {
                trigger: KiCadConditionalLocalRoutingTrigger::BestCandidateViasAtLeast {
                    minimum_vias: 0,
                },
                routing: KiCadGridRouteConfig::default(),
            }],
            ..KiCadConnectionInsertionConfig::default()
        };
        assert!(validate_insertion_config(&config).is_err());
    }

    #[test]
    fn ordered_refinement_improvement_is_directional() {
        assert_eq!(ordered_refinement_improvement_percent(100.0, 96.0), 4.0);
        assert_eq!(ordered_refinement_improvement_percent(100.0, 101.0), -1.0);
    }

    #[test]
    fn ordered_refinement_stops_only_after_a_small_live_improvement() {
        let config = KiCadOrderedLocalRefinementConfig {
            minimum_portfolio_entries: 2,
            continue_if_score_improvement_percent_at_least: 3.0,
        };
        assert!(ordered_refinement_stop_evidence(&config, 1, 3, Some(100.0), Some(99.0)).is_none());
        assert!(ordered_refinement_stop_evidence(&config, 2, 3, Some(100.0), Some(96.0)).is_none());
        let evidence = ordered_refinement_stop_evidence(&config, 2, 3, Some(100.0), Some(98.0))
            .expect("a two-percent improvement is below the threshold");
        assert_eq!(evidence.evaluated_before_stop, 2);
        assert_eq!(evidence.improvement_percent, 2.0);
        assert!(!evidence.resumed_after_native_rejection);
    }

    #[test]
    fn skipped_refinements_are_restored_only_when_admission_has_no_child() {
        assert!(should_restore_skipped_refinements(0, 2, 3));
        assert!(!should_restore_skipped_refinements(1, 2, 3));
        assert!(!should_restore_skipped_refinements(0, 3, 3));
    }

    #[test]
    fn local_native_admission_is_explicit_and_deterministically_ranked() {
        let candidate = |attempt_index, score_mm, route_cost| LocalRouteCandidate {
            attempt_index,
            score_mm,
            route_cost,
            candidate_path: PathBuf::from(format!("candidate-{attempt_index}.json")),
            artifact_directory: PathBuf::from(format!("attempt-{attempt_index}")),
        };
        let candidates = vec![
            candidate(0, 12.0, 100),
            candidate(1, 10.0, 200),
            candidate(2, 10.0, 100),
            candidate(3, 10.0, 100),
        ];
        assert_eq!(
            local_native_admission_order(&candidates, KiCadLocalNativeAdmission::Exhaustive),
            vec![0, 1, 2, 3]
        );
        assert_eq!(
            local_native_admission_order(
                &candidates,
                KiCadLocalNativeAdmission::ScoreOrderedUntilComplete
            ),
            vec![2, 3, 1, 0]
        );
    }

    #[test]
    fn conditional_refinement_uses_quality_or_structured_search_failure() {
        let quality = |vias| KiCadRouteQuality {
            length_mm: 10.0,
            segments: 1,
            stored_track_length_mm: 10.0,
            stored_segments: 1,
            overlapping_track_length_mm: 0.0,
            vias,
            branch_via_transitions: vias,
            maximum_branch_vias: vias,
            via_cluster_threshold_mm: 1.0,
            minimum_via_spacing_mm: None,
            close_via_pairs: 0,
            clustered_vias: 0,
            branch_bends: 0,
            branch_points: 2,
        };
        let attempt = |ordinal, vias| KiCadConnectionInsertionAttempt {
            ordinal,
            scope: KiCadConnectionInsertionAttemptScope::InsertedConnectionOnly,
            status: KiCadConnectionInsertionAttemptStatus::CandidateGenerated,
            artifact_directory: PathBuf::from(format!("attempt-{ordinal}")),
            routing_config_sha256: None,
            route_cost: Some(100),
            route_expansions: Some(10),
            route_failure: None,
            route_elapsed_micros: 0,
            route_quality: Some(quality(vias)),
            score_mm: Some(10.0 + vias as f64 * 2.0),
            native_admission_order: None,
            native_gate_invoked: false,
            native_gate_elapsed_micros: 0,
            verification: None,
            future_reachability: None,
            single_connection_ripup: None,
            error: None,
            selected: false,
            elapsed_micros: 0,
        };
        let attempts = vec![attempt(0, 5), attempt(1, 3)];
        let candidates = vec![
            LocalRouteCandidate {
                attempt_index: 0,
                score_mm: 20.0,
                route_cost: 100,
                candidate_path: PathBuf::from("candidate-0.json"),
                artifact_directory: PathBuf::from("attempt-0"),
            },
            LocalRouteCandidate {
                attempt_index: 1,
                score_mm: 16.0,
                route_cost: 100,
                candidate_path: PathBuf::from("candidate-1.json"),
                artifact_directory: PathBuf::from("attempt-1"),
            },
        ];
        let trigger =
            KiCadConditionalLocalRoutingTrigger::BestCandidateViasAtLeast { minimum_vias: 3 };

        let (active, score, vias, _) =
            conditional_local_trigger_decision(&trigger, 3, &candidates, &attempts);
        assert!(active);
        assert_eq!(score, Some(16.0));
        assert_eq!(vias, Some(3));

        let (active, _, _, reason) =
            conditional_local_trigger_decision(&trigger, 2, &candidates, &attempts);
        assert!(!active);
        assert!(reason.contains("two-terminal"));

        let trigger = KiCadConditionalLocalRoutingTrigger::SearchBudgetExhaustedWithoutCandidate;
        let mut failed = attempt(0, 0);
        failed.status = KiCadConnectionInsertionAttemptStatus::RouteFailed;
        failed.route_quality = None;
        failed.score_mm = None;
        failed.route_failure = Some(KiCadRouteSearchFailure {
            kind: KiCadRouteSearchFailureKind::SearchBudgetExhausted,
            connection: "NET".into(),
            branch: 0,
            astar_expansions: 101,
            reachability_expansions: 50,
            heuristic_expansions: None,
            terminal_context: None,
        });
        // Unlike topology refinement, search guidance is useful for two-pad
        // nets too. No generated route is required to activate it.
        for terminal_count in [2, 3] {
            assert!(
                conditional_local_trigger_decision(
                    &trigger,
                    terminal_count,
                    &[],
                    &[failed.clone()]
                )
                .0
            );
        }
        let mut mixed = attempts.clone();
        mixed.push(failed.clone());
        assert!(!conditional_local_trigger_decision(&trigger, 3, &candidates, &mixed).0);
        failed.route_failure.as_mut().unwrap().kind = KiCadRouteSearchFailureKind::GridDisconnected;
        assert!(!conditional_local_trigger_decision(&trigger, 2, &[], &[failed.clone()]).0);
        failed.route_failure = None;
        failed.error = Some("router exhausted its search budget for \"NET\" branch 0 after 101 aggregate expansions; reachability preflight used 50 expansions".into());
        assert!(!conditional_local_trigger_decision(&trigger, 2, &[], &[failed]).0);
        assert!(!conditional_local_trigger_decision(&trigger, 2, &[], &[]).0);
        let parsed: KiCadConditionalLocalRoutingTrigger =
            serde_json::from_str(r#"{"kind":"search_budget_exhausted_without_candidate"}"#)
                .unwrap();
        assert_eq!(parsed, trigger);
    }

    #[test]
    fn insertion_config_without_admission_field_remains_exhaustive() {
        let config: KiCadConnectionInsertionConfig =
            serde_json::from_str("{}").expect("parse backward-compatible config");
        assert_eq!(
            config.local_native_admission,
            KiCadLocalNativeAdmission::Exhaustive
        );
        assert!(config.future_reachability.is_none());
    }

    #[test]
    fn future_reachability_policy_is_explicitly_advisory_or_required() {
        let evidence = |policy, unreachable_connections| KiCadFutureReachabilityEvidence {
            schema_version: 1,
            artifact_directory: PathBuf::from("future-reachability"),
            policy,
            maximum_connections: 3,
            available_connections: 3,
            checked_connections: 3,
            reachable_connections: 3 - unreachable_connections,
            unreachable_connections,
            all_checked_reachable: unreachable_connections == 0,
            total_reachability_expansions: 0,
            total_route_expansions: 0,
            elapsed_micros: 0,
            trials: Vec::new(),
            interpretation: String::new(),
        };
        assert!(future_reachability_admits(&evidence(
            KiCadFutureReachabilityPolicy::PreferMoreReachable,
            1
        )));
        assert!(!future_reachability_admits(&evidence(
            KiCadFutureReachabilityPolicy::RequireAll,
            1
        )));
        assert!(future_reachability_admits(&evidence(
            KiCadFutureReachabilityPolicy::RequireAll,
            0
        )));
    }

    #[test]
    fn two_terminal_portfolio_skips_only_topology_equivalent_entries() {
        let rooted = KiCadGridRouteConfig::default();
        let mut shared = rooted.clone();
        shared.multi_terminal_routing = KiCadMultiTerminalRoutingPolicy::SharedCopperTree;
        shared.maximum_tree_attachment_searches = 8;
        shared.tree_attachment_minimum_spacing_mm = 2.0;
        shared.tree_attachment_objective = KiCadTreeAttachmentObjective::RouterCost;
        let mut finer = shared.clone();
        finer.resolution_mm = 0.125;
        let config = KiCadConnectionInsertionConfig {
            local_candidate_cap: 3,
            local_routing_portfolio: vec![rooted, shared, finer],
            ..KiCadConnectionInsertionConfig::default()
        };

        assert_eq!(
            effective_local_portfolio_indices(&config, 2, None).unwrap(),
            vec![0, 2]
        );
        assert_eq!(
            effective_local_portfolio_indices(&config, 3, None).unwrap(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn gap_portfolio_skips_unshifted_grids_but_keeps_a_distinct_phase() {
        let bounds = [0.0, 0.0, 12.0, 8.0];
        let mut model = KiCadRoutingModel {
            bounds,
            outline: BoardOutline::rectangle(bounds).unwrap(),
            terminals: vec![[2.0, 4.0], [10.0, 4.0]],
            terminal_pads: vec![],
            target_holes: vec![],
            obstacles: [-0.635, 0.635]
                .into_iter()
                .map(|dy| CopperObstacle {
                    source_uuid: None,
                    layers: [true, false],
                    blocks_tracks: true,
                    blocks_vias: true,
                    local_clearance_mm: 0.0,
                    geometry: ObstacleGeometry::Rectangle {
                        center: [6.0, 4.0 + dy],
                        half_size: [0.75, 0.45],
                        angle_degrees: 0.0,
                    },
                    kind: KiCadViaLocalRerouteBlockerKind::FootprintPad,
                    object: "U1".into(),
                    net: None,
                    footprint: Some("U1".into()),
                    component_movable: false,
                })
                .collect(),
        };
        let base = KiCadGridRouteConfig {
            resolution_mm: 0.05,
            trace_width_mm: 0.15,
            clearance_mm: 0.1,
            ..Default::default()
        };
        let mut aligned = base.clone();
        aligned.grid_alignment = KiCadGridAlignment::NearestNarrowPadGap;
        let config = KiCadConnectionInsertionConfig {
            local_candidate_cap: 2,
            local_routing_portfolio: vec![base, aligned],
            ..Default::default()
        };
        assert_eq!(
            effective_local_portfolio_indices(&config, 2, Some(&model)).unwrap(),
            vec![0, 1]
        );
        model.obstacles[0].local_clearance_mm = 0.2;
        assert_eq!(
            effective_local_portfolio_indices(&config, 2, Some(&model)).unwrap(),
            vec![0]
        );
        // With no board facts, preserve both declarations; do not assume that
        // a geometry-dependent policy is equivalent to the fixed grid.
        assert_eq!(
            effective_local_portfolio_indices(&config, 2, None).unwrap(),
            vec![0, 1]
        );
    }
}

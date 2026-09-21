// Copyright (C) 2026 Toit contributors.

use super::*;

const SEQUENTIAL_ROUTER_SCHEMA_VERSION: u32 = 8;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadSequentialRipupConfig {
    #[serde(skip_serializing_if = "KiCadSequentialPortfolioPolicy::is_best_score")]
    pub selection_policy: KiCadSequentialPortfolioPolicy,
    /// Optional pair displacement after ordinary and single-net recovery fail.
    pub compound_recovery: Option<KiCadCompoundRecoveryConfig>,
    pub prioritize_boundary_blockers: bool,
    pub maximum_invocations: usize,
    pub maximum_diagnosis_trials: usize,
    /// Repair uses this portfolio member with the current remaining-net demand.
    pub routing_config_index: usize,
    pub allow_existing_annotation_findings: bool,
    pub restoration: Option<KiCadRestorationRecoveryConfig>,
}

impl Default for KiCadSequentialRipupConfig {
    fn default() -> Self {
        Self {
            selection_policy: KiCadSequentialPortfolioPolicy::BestScore,
            compound_recovery: None,
            prioritize_boundary_blockers: false,
            maximum_invocations: 4,
            maximum_diagnosis_trials: 64,
            routing_config_index: 0,
            allow_existing_annotation_findings: false,
            restoration: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadSequentialPortfolioPolicy {
    /// Generate every candidate, then admit in completion-first score order.
    #[default]
    BestScore,
    /// Treat configurations as ordered fallbacks and stop at the first
    /// candidate that passes native admission.
    OrderedFirstAdmitted,
}

impl KiCadSequentialPortfolioPolicy {
    pub(crate) fn is_best_score(&self) -> bool {
        *self == Self::BestScore
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadSequentialRouterConfig {
    /// Empty selects deterministic KiCad net/pad discovery order. A non-empty
    /// order must name every discovered routable connection exactly once.
    pub connection_order: Vec<String>,
    /// Attempt each remaining net once after a typed ordinary routing failure.
    /// Disabled preserves the legacy contiguous committed-prefix contract.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub continue_after_routing_failure: bool,
    pub routing_portfolio: Vec<KiCadGridRouteConfig>,
    pub portfolio_policy: KiCadSequentialPortfolioPolicy,
    /// Map an obstacle-distance portfolio index to an earlier geometric index.
    /// Run the fallback only after that entry reports search-budget exhaustion.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub obstacle_distance_fallbacks: BTreeMap<usize, usize>,
    pub maximum_connections: usize,
    pub via_penalty_mm: f64,
    /// Optional bounded recovery after every ordinary portfolio entry fails.
    pub ripup: Option<KiCadSequentialRipupConfig>,
}

impl Default for KiCadSequentialRouterConfig {
    fn default() -> Self {
        Self {
            connection_order: Vec::new(),
            continue_after_routing_failure: false,
            routing_portfolio: vec![KiCadGridRouteConfig::default()],
            portfolio_policy: KiCadSequentialPortfolioPolicy::BestScore,
            obstacle_distance_fallbacks: BTreeMap::new(),
            maximum_connections: 1_024,
            via_penalty_mm: 2.0,
            ripup: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadSequentialRouterTermination {
    Complete,
    RoutingFailed,
    BoundReached,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadSequentialRouteAttempt {
    pub config_index: usize,
    pub candidate: Option<PathBuf>,
    pub quality: Option<KiCadRouteQuality>,
    pub score_mm: Option<f64>,
    pub expansions: u32,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_failure: Option<KiCadRouteSearchFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    pub native_admission: Option<KiCadSequentialNativeAdmission>,
    pub selected: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadSequentialNativeAdmission {
    pub verification_scope: String,
    pub erc_input_sha256: String,
    pub directory: PathBuf,
    pub expected_unconnected_items: usize,
    pub verification: VerificationReport,
    pub introduced_erc_violations: usize,
    pub introduced_drc_design_violations: usize,
    pub introduced_schematic_parity_issues: usize,
    pub complete: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadSequentialRouteStep {
    pub ordinal: usize,
    pub connection: String,
    pub artifact_directory: PathBuf,
    pub attempts: Vec<KiCadSequentialRouteAttempt>,
    pub selected_config_index: Option<usize>,
    #[serde(default)]
    pub repair: Option<KiCadSequentialRipupEvidence>,
    #[serde(default)]
    pub repair_skip_reason: Option<String>,
    pub board_sha256_after: Option<String>,
    pub elapsed_micros: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct KiCadSequentialRipupEvidence {
    pub directory: PathBuf,
    pub routing_config_index: usize,
    pub selected_attempt: Option<usize>,
    pub yielded_connection: Option<String>,
    pub target_quality: Option<KiCadRouteQuality>,
    pub yielded_quality: Option<KiCadRouteQuality>,
    pub route_expansions: u64,
    pub native_admission: Option<KiCadSequentialNativeAdmission>,
    pub selected: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub changed_routes: BTreeMap<String, KiCadChangedRouteReceipt>,
    #[serde(default)]
    pub restoration_invocations: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compound: Option<KiCadSequentialCompoundEvidence>,
}

impl KiCadSequentialRouteStep {
    pub fn committed(&self) -> bool {
        self.selected_config_index.is_some() || self.repair.as_ref().is_some_and(|r| r.selected)
    }

    pub fn selected_admission(&self) -> Option<&KiCadSequentialNativeAdmission> {
        self.attempts
            .iter()
            .find(|a| a.selected)
            .and_then(|a| a.native_admission.as_ref())
            .or_else(|| {
                self.repair
                    .as_ref()
                    .filter(|r| r.selected)
                    .and_then(|r| r.native_admission.as_ref())
            })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadSequentialRouterResult {
    pub schema_version: u32,
    pub contract: String,
    pub board_id: String,
    pub source_directory: PathBuf,
    pub source_board_sha256_before: String,
    pub source_board_sha256_after: String,
    pub source_board_unchanged: bool,
    pub output_directory: PathBuf,
    pub result_directory: PathBuf,
    pub result_board: PathBuf,
    pub discovered_connections: Vec<KiCadConnectionSummary>,
    pub native_source_baseline: VerificationReport,
    pub expected_source_unconnected_items: usize,
    pub connection_order: Vec<String>,
    pub portfolio_policy: KiCadSequentialPortfolioPolicy,
    pub maximum_connections: usize,
    pub completed_connections: usize,
    pub total_expansions: u64,
    pub termination: KiCadSequentialRouterTermination,
    pub steps: Vec<KiCadSequentialRouteStep>,
    pub final_statistics: KiCadBoardStatistics,
    /// Present only in schema 9: steps are attempted order, not a commit prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sweep: Option<KiCadSequentialSweepState>,
    #[serde(default)]
    pub resume: Option<KiCadSequentialResumeEvidence>,
}

/// Electrical identities partitioned by actual admission, in original order.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadSequentialSweepState {
    pub attempted_connections: usize,
    pub completed: Vec<String>,
    pub failed: Vec<String>,
    pub unattempted: Vec<String>,
    pub repair_invocations_used: usize,
    pub expected_unconnected_items: usize,
    pub halted_on_unclassified_failure: bool,
}

fn classified_sweep_failure(step: &KiCadSequentialRouteStep) -> bool {
    // Failed repair receipts still mix search, adapter and native failures.
    // Preserve the incumbent and stop rather than guess from error strings.
    if step.committed() || step.repair.is_some() {
        return false;
    }
    let attempted = step.attempts.iter().filter(|a| a.skip_reason.is_none());
    let mut count = 0;
    for attempt in attempted {
        count += 1;
        if attempt.candidate.is_some()
            || attempt.native_admission.is_some()
            || !attempt.route_failure.as_ref().is_some_and(|failure| {
                matches!(
                    failure.kind,
                    KiCadRouteSearchFailureKind::GridDisconnected
                        | KiCadRouteSearchFailureKind::SearchBudgetExhausted
                )
            })
        {
            return false;
        }
    }
    count > 0
}

fn sweep_state(
    order: &[String],
    discovered: &[KiCadConnectionSummary],
    steps: &[KiCadSequentialRouteStep],
    repair_invocations_used: usize,
) -> Result<KiCadSequentialSweepState, String> {
    if repair_invocations_used != steps.iter().filter(|s| s.repair.is_some()).count() {
        return Err("sweep repair inventory disagrees with the cumulative allowance".into());
    }
    let mut state = KiCadSequentialSweepState {
        attempted_connections: steps.len(),
        completed: Vec::new(),
        failed: Vec::new(),
        unattempted: Vec::new(),
        repair_invocations_used,
        expected_unconnected_items: 0,
        halted_on_unclassified_failure: steps
            .last()
            .is_some_and(|s| !s.committed() && !classified_sweep_failure(s)),
    };
    if steps.len() > order.len()
        || steps
            .iter()
            .enumerate()
            .any(|(i, s)| s.ordinal != i || s.connection != order[i])
    {
        return Err("sweep steps must follow original attempted order exactly once".into());
    }
    for (i, connection) in order.iter().enumerate() {
        if steps
            .get(i)
            .is_some_and(KiCadSequentialRouteStep::committed)
        {
            state.completed.push(connection.clone());
        } else {
            let summary = discovered
                .iter()
                .find(|s| s.connection == *connection)
                .ok_or("sweep identity missing from electrical inventory")?;
            state.expected_unconnected_items += summary.electrical_terminal_count() - 1;
            if i < steps.len() {
                state.failed.push(connection.clone());
            } else {
                state.unattempted.push(connection.clone());
            }
        }
    }
    Ok(state)
}

fn committed_identities(steps: &[KiCadSequentialRouteStep]) -> Vec<String> {
    steps
        .iter()
        .filter(|s| s.committed())
        .map(|s| s.connection.clone())
        .collect()
}

fn sweep_termination(
    total: usize,
    completed: usize,
    steps: &[KiCadSequentialRouteStep],
) -> KiCadSequentialRouterTermination {
    if completed == total {
        KiCadSequentialRouterTermination::Complete
    } else if steps.len() == total
        || steps
            .last()
            .is_some_and(|s| !s.committed() && !classified_sweep_failure(s))
    {
        KiCadSequentialRouterTermination::RoutingFailed
    } else {
        KiCadSequentialRouterTermination::BoundReached
    }
}

fn recently_changed_connections(steps: &[KiCadSequentialRouteStep]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for step in steps.iter().rev().filter(|step| step.committed()) {
        let mut changed = vec![step.connection.clone()];
        if let Some(repair) = step.repair.as_ref().filter(|repair| repair.selected) {
            if let Some(yielded) = &repair.yielded_connection {
                changed.push(yielded.clone());
            }
            changed.extend(repair.changed_routes.keys().cloned());
        }
        for connection in changed {
            if seen.insert(normalize_net(&connection).to_string()) {
                result.push(connection);
            }
        }
    }
    result
}

fn failed_route_attempt(
    config_index: usize,
    error: KiCadRouteError,
) -> KiCadSequentialRouteAttempt {
    let failure = error.search_failure().cloned();
    KiCadSequentialRouteAttempt {
        config_index,
        candidate: None,
        quality: None,
        score_mm: None,
        expansions: failure.as_ref().map_or(0, |f| f.astar_expansions),
        error: Some(error.to_string()),
        route_failure: failure,
        skip_reason: None,
        native_admission: None,
        selected: false,
    }
}

fn skipped_obstacle_fallback(
    config: &KiCadSequentialRouterConfig,
    config_index: usize,
    attempts: &[KiCadSequentialRouteAttempt],
) -> Option<KiCadSequentialRouteAttempt> {
    let source = *config.obstacle_distance_fallbacks.get(&config_index)?;
    if attempts.iter().any(|attempt| {
        attempt.config_index == source
            && attempt.route_failure.as_ref().is_some_and(|failure| {
                failure.kind == KiCadRouteSearchFailureKind::SearchBudgetExhausted
            })
    }) {
        return None;
    }
    Some(KiCadSequentialRouteAttempt {
        config_index,
        candidate: None,
        quality: None,
        score_mm: None,
        expansions: 0,
        error: None,
        route_failure: None,
        skip_reason: Some(format!(
            "portfolio entry {source} did not report search-budget exhaustion"
        )),
        native_admission: None,
        selected: false,
    })
}

pub fn route_kicad_board_sequentially(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadSequentialRouterConfig,
) -> Result<KiCadSequentialRouterResult, String> {
    route_with_checkpoint(source_directory, board_id, output_directory, config, None)
}

pub(super) fn route_with_checkpoint(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadSequentialRouterConfig,
    checkpoint: Option<(&Path, &KiCadSequentialRouterResult, bool)>,
) -> Result<KiCadSequentialRouterResult, String> {
    validate_sequential_config(config)?;
    if checkpoint.is_some() && config.continue_after_routing_failure {
        return Err(
            "sparse sweep routing cannot resume a legacy prefix; start from the cold source".into(),
        );
    }
    if output_directory.exists() {
        return Err(format!(
            "refusing to overwrite sequential KiCad route {}",
            output_directory.display()
        ));
    }
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    if !source_board.is_file() {
        return Err(format!(
            "source board {} does not exist",
            source_board.display()
        ));
    }
    let source_statistics = inspect_kicad_board(&source_board)?;
    if source_statistics.segments != 0
        || source_statistics.arcs != 0
        || source_statistics.vias != 0
        || source_statistics.copper_zones != 0
        || source_statistics.copper_graphics != 0
    {
        return Err("sequential KiCad router requires a zero-copper source board".into());
    }
    let discovered_connections = inspect_kicad_routable_connections(&source_board)?;
    let discovered = discovered_connections
        .iter()
        .map(|summary| summary.connection.clone())
        .collect::<BTreeSet<_>>();
    if discovered.is_empty() {
        return Err("cold board has no named connection with at least two pad centers".into());
    }
    let connection_order = if config.connection_order.is_empty() {
        discovered_connections
            .iter()
            .map(|summary| summary.connection.clone())
            .collect()
    } else {
        let configured = config
            .connection_order
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if configured.len() != config.connection_order.len() || configured != discovered {
            return Err(format!(
                "explicit connection_order must name each of the {} discovered connections exactly once",
                discovered.len()
            ));
        }
        config.connection_order.clone()
    };

    fs::create_dir_all(output_directory).map_err(|error| {
        format!(
            "failed to create sequential route {}: {error}",
            output_directory.display()
        )
    })?;
    write_json(&output_directory.join("config.json"), config)?;
    let result_directory = output_directory.join("result");
    copy_directory_tree(source_directory, &result_directory)?;
    let result_board = result_directory.join(format!("{board_id}.kicad_pcb"));
    let baseline_directory = output_directory.join("native-source-baseline");
    copy_directory_tree(source_directory, &baseline_directory)?;
    let native_source_baseline = verify_materialized_rung(&baseline_directory, board_id)?;
    commit_sequential_candidate(
        &baseline_directory,
        &baseline_directory.join(format!("{board_id}.kicad_pcb")),
        &result_directory,
        &result_board,
    )?;
    let expected_source_unconnected_items = discovered_connections
        .iter()
        .map(|connection| connection.electrical_terminal_count() - 1)
        .sum::<usize>();
    if native_source_baseline.selected_net_unconnected_items != expected_source_unconnected_items {
        return Err(format!(
            "native source reports {} unconnected items, but the pad inventory predicts {expected_source_unconnected_items}",
            native_source_baseline.selected_net_unconnected_items
        ));
    }
    let baseline_erc = read_json(&baseline_directory.join("erc.json"))?;
    let baseline_drc = read_json(&baseline_directory.join("drc.json"))?;
    let source_erc_input_sha256 = kicad_erc_input_sha256(source_directory)?;
    let source_board_sha256_before = file_sha256(&source_board)?;
    let mut steps = Vec::new();
    let mut completed_connections = 0;
    let mut connected_items = 0;
    let mut total_expansions = 0_u64;
    let mut repair_invocations = 0;
    let mut termination = KiCadSequentialRouterTermination::BoundReached;
    let mut resume = None;
    if let Some((directory, previous, allow_annotations)) = checkpoint {
        let (committed_steps, evidence) = sequential_resume::prepare(
            directory,
            previous,
            source_directory,
            output_directory,
            config,
            &connection_order,
            &baseline_drc,
            allow_annotations,
        )?;
        completed_connections = committed_steps.len();
        connected_items = expected_source_unconnected_items - evidence.initial_unconnected_items;
        total_expansions = previous.total_expansions;
        steps = committed_steps;
        commit_sequential_candidate(
            &evidence.snapshot_directory,
            &evidence
                .snapshot_directory
                .join(format!("{board_id}.kicad_pcb")),
            &result_directory,
            &result_board,
        )?;
        if completed_connections == connection_order.len() {
            termination = KiCadSequentialRouterTermination::Complete;
        }
        resume = Some(evidence);
    }
    let start_ordinal = completed_connections;
    let initial = build_result(
        board_id,
        source_directory,
        &source_board_sha256_before,
        output_directory,
        &result_directory,
        &result_board,
        &discovered_connections,
        &native_source_baseline,
        expected_source_unconnected_items,
        &connection_order,
        config.portfolio_policy,
        config.maximum_connections,
        completed_connections,
        total_expansions,
        termination,
        &steps,
        resume.as_ref(),
        config.continue_after_routing_failure,
        repair_invocations,
    )?;
    write_checkpoint(&output_directory.join("sequential-route.json"), &initial)?;

    for (ordinal, connection) in connection_order
        .iter()
        .take(config.maximum_connections)
        .enumerate()
        .skip(start_ordinal)
    {
        let started = Instant::now();
        let artifact_directory = output_directory.join(format!(
            "step-{ordinal:03}-{}",
            filesystem_connection_label(connection)
        ));
        fs::create_dir_all(&artifact_directory).map_err(|error| {
            format!("failed to create {}: {error}", artifact_directory.display())
        })?;
        let expected_reduction = discovered_connections
            .iter()
            .find(|summary| summary.connection == *connection)
            .expect("configured connection order was checked against discovery")
            .electrical_terminal_count()
            - 1;
        let expected_unconnected_items = expected_source_unconnected_items
            .checked_sub(connected_items + expected_reduction)
            .ok_or_else(|| "sequential connectivity accounting underflowed".to_string())?;
        let mut routing_portfolio = config.routing_portfolio.clone();
        for routing in &mut routing_portfolio {
            let committed = committed_identities(&steps);
            routing_demand::remove_completed(routing, &committed);
        }
        let mut attempts = Vec::with_capacity(routing_portfolio.len());
        let mut selected_config_index = None;
        let mut board_sha256_after = None;
        if config.portfolio_policy == KiCadSequentialPortfolioPolicy::OrderedFirstAdmitted {
            for (config_index, routing) in routing_portfolio.iter().enumerate() {
                if let Some(skipped) = skipped_obstacle_fallback(config, config_index, &attempts) {
                    attempts.push(skipped);
                    continue;
                }
                let candidate_path =
                    artifact_directory.join(format!("candidate-{config_index:02}.json"));
                let candidate = match route_materialized_connection_detailed(
                    &result_board,
                    connection,
                    routing,
                ) {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        let attempt = failed_route_attempt(config_index, error);
                        total_expansions += u64::from(attempt.expansions);
                        attempts.push(attempt);
                        continue;
                    }
                };
                let quality = route_candidate_quality(&candidate)?;
                let score = quality.length_mm + quality.vias as f64 * config.via_penalty_mm;
                total_expansions += u64::from(candidate.expansions);
                write_json(&candidate_path, &candidate)?;
                attempts.push(KiCadSequentialRouteAttempt {
                    config_index,
                    candidate: Some(candidate_path.clone()),
                    quality: Some(quality),
                    score_mm: Some(score),
                    expansions: candidate.expansions,
                    error: None,
                    route_failure: None,
                    skip_reason: None,
                    native_admission: None,
                    selected: false,
                });
                let native_directory =
                    artifact_directory.join(format!("native-candidate-{config_index:02}"));
                copy_directory_tree(&result_directory, &native_directory)?;
                let native_board = native_directory.join(format!("{board_id}.kicad_pcb"));
                apply_route_candidate(&result_board, &candidate_path, &native_board)?;
                let admission = verify_sequential_candidate(
                    &baseline_erc,
                    &baseline_drc,
                    &source_erc_input_sha256,
                    &native_directory,
                    board_id,
                    expected_unconnected_items,
                )?;
                let accepted = admission.complete;
                let attempt = attempts.last_mut().expect("attempt was just recorded");
                attempt.native_admission = Some(admission);
                if accepted {
                    commit_sequential_candidate(
                        &native_directory,
                        &native_board,
                        &result_directory,
                        &result_board,
                    )?;
                    attempt.selected = true;
                    selected_config_index = Some(config_index);
                    completed_connections += 1;
                    connected_items += expected_reduction;
                    board_sha256_after = Some(file_sha256(&result_board)?);
                    break;
                }
            }
        } else {
            let mut candidates = Vec::new();
            for (config_index, routing) in routing_portfolio.iter().enumerate() {
                if let Some(skipped) = skipped_obstacle_fallback(config, config_index, &attempts) {
                    attempts.push(skipped);
                    continue;
                }
                let candidate_path =
                    artifact_directory.join(format!("candidate-{config_index:02}.json"));
                match route_materialized_connection_detailed(&result_board, connection, routing) {
                    Ok(candidate) => {
                        let quality = route_candidate_quality(&candidate)?;
                        let score = quality.length_mm + quality.vias as f64 * config.via_penalty_mm;
                        total_expansions += u64::from(candidate.expansions);
                        write_json(&candidate_path, &candidate)?;
                        attempts.push(KiCadSequentialRouteAttempt {
                            config_index,
                            candidate: Some(candidate_path.clone()),
                            quality: Some(quality.clone()),
                            score_mm: Some(score),
                            expansions: candidate.expansions,
                            error: None,
                            route_failure: None,
                            skip_reason: None,
                            native_admission: None,
                            selected: false,
                        });
                        candidates.push((
                            score,
                            quality.vias,
                            quality.length_mm,
                            config_index,
                            candidate_path,
                        ));
                    }
                    Err(error) => {
                        let attempt = failed_route_attempt(config_index, error);
                        total_expansions += u64::from(attempt.expansions);
                        attempts.push(attempt);
                    }
                }
            }
            candidates.sort_by(|left, right| {
                left.0
                    .total_cmp(&right.0)
                    .then_with(|| left.1.cmp(&right.1))
                    .then_with(|| left.2.total_cmp(&right.2))
                    .then_with(|| left.3.cmp(&right.3))
            });
            for (_, _, _, config_index, candidate_path) in candidates {
                let native_directory =
                    artifact_directory.join(format!("native-candidate-{config_index:02}"));
                copy_directory_tree(&result_directory, &native_directory)?;
                let native_board = native_directory.join(format!("{board_id}.kicad_pcb"));
                apply_route_candidate(&result_board, &candidate_path, &native_board)?;
                let admission = verify_sequential_candidate(
                    &baseline_erc,
                    &baseline_drc,
                    &source_erc_input_sha256,
                    &native_directory,
                    board_id,
                    expected_unconnected_items,
                )?;
                let accepted = admission.complete;
                attempts[config_index].native_admission = Some(admission);
                if accepted {
                    commit_sequential_candidate(
                        &native_directory,
                        &native_board,
                        &result_directory,
                        &result_board,
                    )?;
                    attempts[config_index].selected = true;
                    selected_config_index = Some(config_index);
                    completed_connections += 1;
                    connected_items += expected_reduction;
                    board_sha256_after = Some(file_sha256(&result_board)?);
                    break;
                }
            }
        }
        let mut repair = None;
        let mut repair_skip_reason = None;
        if selected_config_index.is_none() {
            if let Some(recovery) = &config.ripup {
                if repair_invocations >= recovery.maximum_invocations {
                    repair_skip_reason = Some("maximum rip-up invocations reached".into());
                } else if completed_connections == 0 {
                    repair_skip_reason = Some("no earlier routed copper to yield".into());
                } else {
                    repair_invocations += 1;
                    let directory = artifact_directory.join("ripup");
                    let repair_config = KiCadSingleConnectionRipupConfig {
                        selection_policy: recovery.selection_policy,
                        diagnosis: KiCadYieldingConnectionDiagnosisConfig {
                            prioritize_boundary_blockers: recovery.prioritize_boundary_blockers,
                            preferred_yielding_connections: recently_changed_connections(&steps),
                            minimum_yielding_connections: 1,
                            maximum_yielding_connections: 1,
                            maximum_trials: recovery.maximum_diagnosis_trials,
                            routing: routing_portfolio[recovery.routing_config_index].clone(),
                        },
                        via_penalty_mm: config.via_penalty_mm,
                        allow_partial_progress: true,
                        allow_existing_annotation_findings: recovery
                            .allow_existing_annotation_findings,
                        restoration: recovery.restoration.clone(),
                    };
                    write_json(
                        &artifact_directory.join("ripup-config.json"),
                        &repair_config,
                    )?;
                    let mut evidence = KiCadSequentialRipupEvidence {
                        directory: directory.clone(),
                        routing_config_index: recovery.routing_config_index,
                        ..Default::default()
                    };
                    match repair_kicad_with_single_connection_ripup(
                        &result_directory,
                        board_id,
                        connection,
                        &directory,
                        &repair_config,
                    ) {
                        Err(error) => evidence.error = Some(error),
                        Ok(result) => {
                            evidence.restoration_invocations = result.restoration_invocations;
                            evidence.route_expansions = result.total_route_expansions;
                            total_expansions += result.total_route_expansions;
                            if let Some(index) = result.selected_attempt {
                                let chosen = &result.attempts[index];
                                let native_directory = &chosen.artifact_directory;
                                let admission = verify_sequential_candidate(
                                    &baseline_erc,
                                    &baseline_drc,
                                    &source_erc_input_sha256,
                                    native_directory,
                                    board_id,
                                    expected_unconnected_items,
                                )?;
                                evidence.selected_attempt = Some(index);
                                evidence.yielded_connection =
                                    Some(chosen.yielding_connection.clone());
                                evidence.target_quality = Some(chosen.target_quality.clone());
                                evidence.yielded_quality = chosen.rerouted_quality.clone();
                                evidence.changed_routes = chosen.changed_routes.clone();
                                if admission.complete {
                                    commit_sequential_candidate(
                                        native_directory,
                                        &native_directory.join(format!("{board_id}.kicad_pcb")),
                                        &result_directory,
                                        &result_board,
                                    )?;
                                    evidence.selected = true;
                                    completed_connections += 1;
                                    connected_items += expected_reduction;
                                    board_sha256_after = Some(file_sha256(&result_board)?);
                                } else {
                                    evidence.error = Some(
                                        "repaired board failed sequential native admission".into(),
                                    );
                                }
                                evidence.native_admission = Some(admission);
                            } else {
                                evidence.error = Some(
                                    "no diagnosed single-net repair restored target connectivity"
                                        .into(),
                                );
                            }
                        }
                    }
                    if !evidence.selected
                        && let Some(policy) = &recovery.compound_recovery
                    {
                        let directory = artifact_directory.join("compound-ripup");
                        write_json(&artifact_directory.join("compound-config.json"), policy)?;
                        let mut compound = KiCadSequentialCompoundEvidence {
                            directory: directory.clone(),
                            ..Default::default()
                        };
                        match compound_recovery::recover(
                            &result_directory,
                            board_id,
                            connection,
                            &directory,
                            &repair_config,
                            policy,
                        ) {
                            Err(error) => compound.error = Some(error),
                            Ok(result) => {
                                compound.selected_attempt = result.selected_attempt;
                                compound.restoration_repairs = result.restoration_repairs;
                                compound.nested_restoration_invocations =
                                    result.nested_restoration_invocations;
                                compound.route_expansions = result.total_route_expansions;
                                evidence.route_expansions += result.total_route_expansions;
                                total_expansions += result.total_route_expansions;
                                evidence.restoration_invocations +=
                                    result.nested_restoration_invocations;
                                if let Some(index) = result.selected_attempt {
                                    let chosen = &result.attempts[index];
                                    let native_directory = chosen
                                        .final_directory
                                        .as_ref()
                                        .ok_or("compound selection lacks final directory")?;
                                    let admission = verify_sequential_candidate(
                                        &baseline_erc,
                                        &baseline_drc,
                                        &source_erc_input_sha256,
                                        native_directory,
                                        board_id,
                                        expected_unconnected_items,
                                    )?;
                                    if admission.complete {
                                        commit_sequential_candidate(
                                            native_directory,
                                            &native_directory.join(format!("{board_id}.kicad_pcb")),
                                            &result_directory,
                                            &result_board,
                                        )?;
                                        evidence.selected_attempt = None;
                                        evidence.yielded_connection = None;
                                        evidence.yielded_quality = None;
                                        evidence.changed_routes = chosen.changed_routes.clone();
                                        evidence.target_quality = evidence
                                            .changed_routes
                                            .get(normalize_net(connection))
                                            .map(|r| r.quality.clone());
                                        evidence.selected = true;
                                        evidence.error = None;
                                        completed_connections += 1;
                                        connected_items += expected_reduction;
                                        board_sha256_after = Some(file_sha256(&result_board)?);
                                    } else {
                                        compound.error = Some(
                                            "compound result failed sequential admission".into(),
                                        );
                                    }
                                    evidence.native_admission = Some(admission);
                                } else {
                                    compound.error = Some(
                                        "no bounded compound repair restored target connectivity"
                                            .into(),
                                    );
                                }
                            }
                        }
                        if !evidence.selected
                            && let Some(error) = &compound.error
                        {
                            evidence.error = Some(format!("compound recovery failed: {error}"));
                        }
                        evidence.compound = Some(compound);
                    }
                    repair = Some(evidence);
                }
            }
        }
        let committed =
            selected_config_index.is_some() || repair.as_ref().is_some_and(|r| r.selected);
        steps.push(KiCadSequentialRouteStep {
            ordinal,
            connection: connection.clone(),
            artifact_directory,
            attempts,
            selected_config_index,
            repair,
            repair_skip_reason,
            board_sha256_after,
            elapsed_micros: started.elapsed().as_micros() as u64,
        });
        termination = if config.continue_after_routing_failure {
            sweep_termination(connection_order.len(), completed_connections, &steps)
        } else if committed {
            if completed_connections == connection_order.len() {
                KiCadSequentialRouterTermination::Complete
            } else {
                KiCadSequentialRouterTermination::BoundReached
            }
        } else {
            KiCadSequentialRouterTermination::RoutingFailed
        };
        let checkpoint = build_result(
            board_id,
            source_directory,
            &source_board_sha256_before,
            output_directory,
            &result_directory,
            &result_board,
            &discovered_connections,
            &native_source_baseline,
            expected_source_unconnected_items,
            &connection_order,
            config.portfolio_policy,
            config.maximum_connections,
            completed_connections,
            total_expansions,
            termination,
            &steps,
            resume.as_ref(),
            config.continue_after_routing_failure,
            repair_invocations,
        )?;
        write_checkpoint(&output_directory.join("sequential-route.json"), &checkpoint)?;
        if termination == KiCadSequentialRouterTermination::RoutingFailed {
            return Ok(checkpoint);
        }
    }
    let result = build_result(
        board_id,
        source_directory,
        &source_board_sha256_before,
        output_directory,
        &result_directory,
        &result_board,
        &discovered_connections,
        &native_source_baseline,
        expected_source_unconnected_items,
        &connection_order,
        config.portfolio_policy,
        config.maximum_connections,
        completed_connections,
        total_expansions,
        termination,
        &steps,
        resume.as_ref(),
        config.continue_after_routing_failure,
        repair_invocations,
    )?;
    write_checkpoint(&output_directory.join("sequential-route.json"), &result)?;
    Ok(result)
}

fn write_checkpoint(path: &Path, result: &KiCadSequentialRouterResult) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    write_json(&temporary, result)?;
    fs::rename(&temporary, path)
        .map_err(|e| format!("failed to commit checkpoint {}: {e}", path.display()))
}

pub(super) fn commit_sequential_candidate(
    native_directory: &Path,
    native_board: &Path,
    result_directory: &Path,
    result_board: &Path,
) -> Result<(), String> {
    fs::copy(native_board, result_board).map_err(|error| {
        format!(
            "failed to commit native candidate {} to {}: {error}",
            native_board.display(),
            result_board.display()
        )
    })?;
    for report in ["erc.json", "drc.json", "verification.json"] {
        let source = native_directory.join(report);
        let destination = result_directory.join(report);
        fs::copy(&source, &destination).map_err(|error| {
            format!(
                "failed to commit native report {} to {}: {error}",
                source.display(),
                destination.display()
            )
        })?;
    }
    verification_preview::copy_to(native_directory, result_directory);
    Ok(())
}

fn verify_sequential_candidate(
    baseline_erc: &serde_json::Value,
    baseline_drc: &serde_json::Value,
    source_erc_input_sha256: &str,
    candidate_directory: &Path,
    board_id: &str,
    expected_unconnected_items: usize,
) -> Result<KiCadSequentialNativeAdmission, String> {
    let verification = verify_materialized_route_only(
        candidate_directory,
        board_id,
        source_erc_input_sha256,
        baseline_erc,
    )?;
    let candidate_erc = read_json(&candidate_directory.join("erc.json"))?;
    let candidate_drc = read_json(&candidate_directory.join("drc.json"))?;
    let introduced_erc_violations = introduced_issue_count(
        schematic_erc_issues(baseline_erc),
        schematic_erc_issues(&candidate_erc),
    )?;
    let introduced_drc_design_violations = introduced_issue_count(
        drc_design_issues(baseline_drc),
        drc_design_issues(&candidate_drc),
    )?;
    let introduced_schematic_parity_issues = introduced_issue_count(
        report_array(baseline_drc, "schematic_parity"),
        report_array(&candidate_drc, "schematic_parity"),
    )?;
    let complete = introduced_erc_violations == 0
        && introduced_drc_design_violations == 0
        && introduced_schematic_parity_issues == 0
        && verification.selected_net_unconnected_items == expected_unconnected_items;
    Ok(KiCadSequentialNativeAdmission {
        verification_scope: "frozen_erc_input_sha256_plus_native_pcb_drc_with_schematic_parity; lib_footprint_mismatch warnings retained separately as library metadata"
            .into(),
        erc_input_sha256: source_erc_input_sha256.into(),
        directory: candidate_directory.to_path_buf(),
        expected_unconnected_items,
        verification,
        introduced_erc_violations,
        introduced_drc_design_violations,
        introduced_schematic_parity_issues,
        complete,
    })
}

fn schematic_erc_issues(report: &serde_json::Value) -> Vec<&serde_json::Value> {
    report["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|sheet| report_array(sheet, "violations"))
        .collect()
}

fn report_array<'a>(report: &'a serde_json::Value, key: &str) -> Vec<&'a serde_json::Value> {
    report[key].as_array().into_iter().flatten().collect()
}

fn introduced_issue_count(
    baseline: Vec<&serde_json::Value>,
    result: Vec<&serde_json::Value>,
) -> Result<usize, String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for issue in baseline {
        *counts.entry(issue_fingerprint(issue)?).or_default() += 1;
    }
    let mut introduced = 0;
    for issue in result {
        let fingerprint = issue_fingerprint(issue)?;
        match counts.get_mut(&fingerprint) {
            Some(count) if *count > 0 => *count -= 1,
            _ => introduced += 1,
        }
    }
    Ok(introduced)
}

fn issue_fingerprint(issue: &serde_json::Value) -> Result<String, String> {
    fn normalize(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Array(values) => {
                let mut normalized = values.iter().map(normalize).collect::<Vec<_>>();
                normalized.sort_by_key(|value| value.to_string());
                serde_json::Value::Array(normalized)
            }
            serde_json::Value::Object(values) => serde_json::Value::Object(
                values
                    .iter()
                    .filter(|(key, _)| key.as_str() != "uuid")
                    .map(|(key, value)| (key.clone(), normalize(value)))
                    .collect(),
            ),
            _ => value.clone(),
        }
    }
    serde_json::to_string(&normalize(issue)).map_err(|error| error.to_string())
}

pub(super) fn validate_sequential_config(
    config: &KiCadSequentialRouterConfig,
) -> Result<(), String> {
    if config.routing_portfolio.is_empty()
        || config.routing_portfolio.len() > 64
        || config.maximum_connections == 0
        || !config.via_penalty_mm.is_finite()
        || config.via_penalty_mm < 0.0
    {
        return Err("invalid bounded sequential KiCad router configuration".into());
    }
    for routing in &config.routing_portfolio {
        validate_grid_route_config(routing)?;
    }
    for (&fallback, &source) in &config.obstacle_distance_fallbacks {
        if source >= fallback || fallback >= config.routing_portfolio.len() {
            return Err(
                "obstacle-distance fallback must reference an earlier portfolio entry".into(),
            );
        }
        let mut expected = config.routing_portfolio[source].clone();
        if expected.heuristic != KiCadGridHeuristic::Geometric {
            return Err("obstacle-distance fallback source must use geometric guidance".into());
        }
        expected.heuristic = KiCadGridHeuristic::ObstacleDistances;
        if serde_json::to_value(&expected).map_err(|e| e.to_string())?
            != serde_json::to_value(&config.routing_portfolio[fallback])
                .map_err(|e| e.to_string())?
        {
            return Err("obstacle-distance fallback may change only the heuristic".into());
        }
    }
    if let Some(repair) = &config.ripup {
        if let Some(compound) = &repair.compound_recovery {
            compound_recovery::validate(compound)?;
        }
        restoration_recovery::validate(&KiCadSingleConnectionRipupConfig {
            restoration: repair.restoration.clone(),
            allow_partial_progress: true,
            ..Default::default()
        })?;
        if !(1..=64).contains(&repair.maximum_invocations)
            || !(1..=1024).contains(&repair.maximum_diagnosis_trials)
            || repair.routing_config_index >= config.routing_portfolio.len()
        {
            return Err("invalid bounded sequential rip-up configuration".into());
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_result(
    board_id: &str,
    source_directory: &Path,
    source_board_sha256_before: &str,
    output_directory: &Path,
    result_directory: &Path,
    result_board: &Path,
    discovered_connections: &[KiCadConnectionSummary],
    native_source_baseline: &VerificationReport,
    expected_source_unconnected_items: usize,
    connection_order: &[String],
    portfolio_policy: KiCadSequentialPortfolioPolicy,
    maximum_connections: usize,
    completed_connections: usize,
    total_expansions: u64,
    termination: KiCadSequentialRouterTermination,
    steps: &[KiCadSequentialRouteStep],
    resume: Option<&KiCadSequentialResumeEvidence>,
    continue_after_routing_failure: bool,
    repair_invocations: usize,
) -> Result<KiCadSequentialRouterResult, String> {
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source_board_sha256_after = file_sha256(&source_board)?;
    let sweep = if continue_after_routing_failure {
        let state = sweep_state(
            connection_order,
            discovered_connections,
            steps,
            repair_invocations,
        )?;
        let native: VerificationReport =
            serde_json::from_value(read_json(&result_directory.join("verification.json"))?)
                .map_err(|e| e.to_string())?;
        if state.completed.len() != completed_connections
            || state.expected_unconnected_items != native.selected_net_unconnected_items
        {
            return Err("sweep electrical inventory disagrees with admitted native board".into());
        }
        Some(state)
    } else {
        None
    };
    let mut report = KiCadSequentialRouterResult {
        schema_version: if sweep.is_some() { 9 } else { SEQUENTIAL_ROUTER_SCHEMA_VERSION },
        contract: "zero-copper immutable source; explicit or discovered deterministic net order; bounded portfolio per net; optional single-net recovery then bounded compound recovery; preserve all copper and geometry outside declared changed-route receipts; transactionally commit only after original-parent native ERC/DRC/parity and connectivity admission; checkpoint every commit; final complete-layout admission remains external"
            .into(),
        board_id: board_id.into(),
        source_directory: source_directory.to_path_buf(),
        source_board_sha256_before: source_board_sha256_before.into(),
        source_board_sha256_after: source_board_sha256_after.clone(),
        source_board_unchanged: source_board_sha256_after == source_board_sha256_before,
        output_directory: output_directory.to_path_buf(),
        result_directory: result_directory.to_path_buf(),
        result_board: result_board.to_path_buf(),
        discovered_connections: discovered_connections.to_vec(),
        native_source_baseline: native_source_baseline.clone(),
        expected_source_unconnected_items,
        connection_order: connection_order.to_vec(),
        portfolio_policy,
        maximum_connections,
        completed_connections,
        total_expansions,
        termination,
        steps: steps.to_vec(),
        final_statistics: inspect_kicad_board(result_board)?,
        resume: resume.cloned(),
        sweep,
    };
    if report.sweep.is_some() {
        report.contract.push_str("; one bounded sweep; steps retain every attempted identity in original order with sparse commits; failed and unattempted electrical groups remain pending; no external prefix resume");
    }
    Ok(report)
}

fn filesystem_connection_label(value: &str) -> String {
    let label = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if label.is_empty() {
        "unnamed".into()
    } else {
        label
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    fs::write(
        path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sweep_step(ordinal: usize, connection: &str, admitted: bool) -> KiCadSequentialRouteStep {
        KiCadSequentialRouteStep {
            ordinal,
            connection: connection.into(),
            artifact_directory: PathBuf::new(),
            attempts: if admitted {
                Vec::new()
            } else {
                vec![failed_route_attempt(
                    0,
                    KiCadRouteError::Search(KiCadRouteSearchFailure {
                        kind: KiCadRouteSearchFailureKind::GridDisconnected,
                        connection: connection.into(),
                        branch: 0,
                        astar_expansions: 0,
                        reachability_expansions: 10,
                        heuristic_expansions: None,
                        terminal_context: None,
                    }),
                )]
            },
            selected_config_index: admitted.then_some(0),
            repair: None,
            repair_skip_reason: None,
            board_sha256_after: admitted.then(|| "native-board".into()),
            elapsed_micros: 0,
        }
    }

    #[test]
    fn sparse_sweep_preserves_electrical_failures_and_bounded_tail_after_later_success() {
        let order = ["front-back", "later", "tail"].map(String::from);
        let discovered = vec![
            KiCadConnectionSummary {
                connection: order[0].clone(),
                distinct_pad_centers: 1,
                electrical_terminal_count: Some(2),
            },
            KiCadConnectionSummary {
                connection: order[1].clone(),
                distinct_pad_centers: 2,
                electrical_terminal_count: Some(3),
            },
            KiCadConnectionSummary {
                connection: order[2].clone(),
                distinct_pad_centers: 2,
                electrical_terminal_count: None,
            },
        ];
        let mut steps = vec![
            sweep_step(0, &order[0], false),
            sweep_step(1, &order[1], true),
        ];
        let state = sweep_state(&order, &discovered, &steps, 0).unwrap();
        assert_eq!(state.failed, [order[0].clone()]);
        assert_eq!(state.completed, [order[1].clone()]);
        assert_eq!(state.unattempted, [order[2].clone()]);
        assert_eq!(state.expected_unconnected_items, 2);
        assert_eq!(
            sweep_termination(3, 1, &steps),
            KiCadSequentialRouterTermination::BoundReached
        );
        steps.push(sweep_step(2, &order[2], true));
        assert_eq!(
            sweep_termination(3, 2, &steps),
            KiCadSequentialRouterTermination::RoutingFailed
        );
        assert_eq!(
            sweep_state(&order, &discovered, &steps, 0)
                .unwrap()
                .expected_unconnected_items,
            1
        );
        steps[0] = sweep_step(0, &order[0], true);
        assert_eq!(
            sweep_termination(3, 3, &steps),
            KiCadSequentialRouterTermination::Complete
        );
        assert_eq!(
            sweep_state(&order, &discovered, &steps, 0)
                .unwrap()
                .expected_unconnected_items,
            0
        );
        steps[2].connection = order[0].clone();
        assert!(sweep_state(&order, &discovered, &steps, 0).is_err());
    }

    #[test]
    fn sparse_sweep_requires_attempted_typed_failure_and_stops_on_failed_repair() {
        let mut step = sweep_step(0, "N", false);
        assert!(classified_sweep_failure(&step));
        step.attempts[0].route_failure.as_mut().unwrap().kind =
            KiCadRouteSearchFailureKind::SearchBudgetExhausted;
        assert!(classified_sweep_failure(&step));
        step.attempts[0].skip_reason = Some("not attempted".into());
        assert!(!classified_sweep_failure(&step));
        step.attempts[0].skip_reason = None;
        step.repair = Some(KiCadSequentialRipupEvidence::default());
        assert!(!classified_sweep_failure(&step));
        assert_eq!(
            sweep_termination(3, 0, &[step.clone()]),
            KiCadSequentialRouterTermination::RoutingFailed
        );
        step.repair = None;
        step.attempts.push(failed_route_attempt(
            1,
            KiCadRouteError::Other("adapter error".into()),
        ));
        assert!(!classified_sweep_failure(&step));
        step.attempts.clear();
        assert!(!classified_sweep_failure(&step));
        let failed = [sweep_step(0, "A", false), sweep_step(1, "B", false)];
        assert_eq!(
            sweep_termination(2, 0, &failed),
            KiCadSequentialRouterTermination::RoutingFailed
        );
    }

    #[test]
    fn sparse_sweep_keeps_failed_demand_and_cumulative_admitted_repair_receipts() {
        let mut steps = vec![
            sweep_step(0, "repaired", true),
            sweep_step(1, "failed", false),
            sweep_step(2, "later", true),
        ];
        steps[0].selected_config_index = None;
        steps[0].repair = Some(KiCadSequentialRipupEvidence {
            selected: true,
            ..Default::default()
        });
        let order = ["repaired", "failed", "later", "pending"].map(String::from);
        let discovered = order
            .iter()
            .map(|n| KiCadConnectionSummary {
                connection: n.clone(),
                distinct_pad_centers: 2,
                electrical_terminal_count: None,
            })
            .collect::<Vec<_>>();
        let state = sweep_state(&order, &discovered, &steps, 1).unwrap();
        assert_eq!(state.repair_invocations_used, 1);
        assert!(sweep_state(&order, &discovered, &steps, 0).is_err());
        let mut routing = KiCadGridRouteConfig::default();
        routing.routing_demand = Some(KiCadRoutingDemand {
            strength: 1.0,
            shoulder_mm: 1.0,
            alternatives: order
                .iter()
                .map(|n| KiCadDemandAlternative {
                    connection: n.clone(),
                    weight: 1.0,
                    trace_width_mm: 0.2,
                    clearance_mm: 0.2,
                    via_size_mm: 0.6,
                    paths: Vec::new(),
                })
                .collect(),
        });
        routing_demand::remove_completed(&mut routing, &committed_identities(&steps));
        assert_eq!(
            routing
                .routing_demand
                .unwrap()
                .alternatives
                .iter()
                .map(|a| a.connection.as_str())
                .collect::<Vec<_>>(),
            ["failed", "pending"]
        );
        let config = KiCadSequentialRouterConfig::default();
        assert!(!config.continue_after_routing_failure);
        assert!(
            serde_json::to_value(config)
                .unwrap()
                .get("continue_after_routing_failure")
                .is_none()
        );
    }

    #[test]
    fn sequential_admission_counts_design_changes_but_not_metadata_changes() {
        let source = serde_json::json!({"violations": [{
            "type": "lib_footprint_mismatch", "severity": "warning",
            "description": "old footprint mismatch", "items": []
        }]});
        let mut candidate = source.clone();
        candidate["violations"][0]["description"] = serde_json::json!("new footprint mismatch");
        let introduced = |candidate: &serde_json::Value| {
            introduced_issue_count(drc_design_issues(&source), drc_design_issues(candidate))
                .unwrap()
        };
        assert_eq!(introduced(&candidate), 0);
        candidate["violations"][0]["severity"] = serde_json::json!("error");
        assert_eq!(introduced(&candidate), 1);
        candidate["violations"][0]["type"] = serde_json::json!("clearance");
        candidate["violations"][0]["severity"] = serde_json::json!("warning");
        assert_eq!(introduced(&candidate), 1);
    }

    #[test]
    fn recent_diagnosis_order_includes_restored_nets_and_skips_uncommitted_steps() {
        let step = |name: &str, selected: bool| KiCadSequentialRouteStep {
            ordinal: 0,
            connection: name.into(),
            artifact_directory: PathBuf::new(),
            attempts: Vec::new(),
            selected_config_index: selected.then_some(0),
            repair: None,
            repair_skip_reason: None,
            board_sha256_after: None,
            elapsed_micros: 0,
        };
        let mut repaired = step("C", false);
        repaired.repair = Some(KiCadSequentialRipupEvidence {
            selected: true,
            yielded_connection: Some("/A".into()),
            ..Default::default()
        });
        let steps = vec![
            step("A", true),
            step("B", true),
            repaired,
            step("failed", false),
        ];
        assert_eq!(recently_changed_connections(&steps), vec!["C", "/A", "B"]);
    }

    #[test]
    fn obstacle_fallback_requires_a_matching_earlier_geometric_entry() {
        let mut config = KiCadSequentialRouterConfig::default();
        let mut guided = config.routing_portfolio[0].clone();
        guided.heuristic = KiCadGridHeuristic::ObstacleDistances;
        config.routing_portfolio.push(guided);
        config.obstacle_distance_fallbacks.insert(1, 0);
        validate_sequential_config(&config).unwrap();
        config.routing_portfolio[1].clearance_mm += 0.01;
        assert!(validate_sequential_config(&config).is_err());
        config.routing_portfolio[1] = config.routing_portfolio[0].clone();
        assert!(validate_sequential_config(&config).is_err());
        config.routing_portfolio[1].heuristic = KiCadGridHeuristic::ObstacleDistances;
        config.obstacle_distance_fallbacks.insert(1, 1);
        assert!(validate_sequential_config(&config).is_err());
        config.obstacle_distance_fallbacks = BTreeMap::from([(2, 0)]);
        assert!(validate_sequential_config(&config).is_err());
    }

    #[test]
    fn obstacle_fallback_uses_typed_failure_from_its_declared_source_only() {
        let mut config = KiCadSequentialRouterConfig::default();
        config.obstacle_distance_fallbacks.insert(2, 1);
        let failure = KiCadRouteSearchFailure {
            kind: KiCadRouteSearchFailureKind::SearchBudgetExhausted,
            connection: "N".into(),
            branch: 0,
            astar_expansions: 101,
            reachability_expansions: 202,
            heuristic_expansions: None,
            terminal_context: None,
        };
        let attempt = failed_route_attempt(1, KiCadRouteError::Search(failure.clone()));
        assert_eq!(attempt.expansions, 101);
        assert_eq!(
            attempt
                .route_failure
                .as_ref()
                .unwrap()
                .reachability_expansions,
            202
        );
        assert!(skipped_obstacle_fallback(&config, 2, &[attempt.clone()]).is_none());
        // An earlier, unrelated exhausted entry must not activate this retry.
        let unrelated = failed_route_attempt(0, KiCadRouteError::Search(failure.clone()));
        assert!(skipped_obstacle_fallback(&config, 2, &[unrelated]).is_some());
        // Neither the old human-readable error nor disconnection activates it.
        let opaque =
            failed_route_attempt(1, KiCadRouteError::Other(attempt.error.clone().unwrap()));
        assert!(skipped_obstacle_fallback(&config, 2, &[opaque]).is_some());
        let disconnected = failed_route_attempt(
            1,
            KiCadRouteError::Search(KiCadRouteSearchFailure {
                kind: KiCadRouteSearchFailureKind::GridDisconnected,
                ..failure
            }),
        );
        let skipped = skipped_obstacle_fallback(&config, 2, &[disconnected]).unwrap();
        assert_eq!(skipped.expansions, 0);
        assert!(skipped.error.is_none() && skipped.candidate.is_none() && !skipped.selected);
        assert!(skipped.skip_reason.is_some());
        let mut success = attempt;
        success.error = None;
        success.route_failure = None;
        success.candidate = Some(PathBuf::from("route.json"));
        assert!(skipped_obstacle_fallback(&config, 2, &[success]).is_some());
        assert!(skipped_obstacle_fallback(&config, 0, &[]).is_none());
    }

    #[test]
    fn legacy_attempts_deserialize_without_inventing_search_evidence() {
        let attempt: KiCadSequentialRouteAttempt = serde_json::from_str(r#"{
            "config_index":0,"candidate":null,"quality":null,"score_mm":null,
            "expansions":0,"error":"search budget exhausted","native_admission":null,"selected":false
        }"#).unwrap();
        assert!(attempt.route_failure.is_none() && attempt.skip_reason.is_none());
        let config: KiCadSequentialRouterConfig = serde_json::from_str("{}").unwrap();
        assert!(config.obstacle_distance_fallbacks.is_empty());
    }

    #[test]
    fn sequential_config_is_bounded_and_requires_a_portfolio() {
        let mut config = KiCadSequentialRouterConfig::default();
        assert_eq!(
            config.portfolio_policy,
            KiCadSequentialPortfolioPolicy::BestScore
        );
        validate_sequential_config(&config).unwrap();
        config.maximum_connections = 0;
        assert!(validate_sequential_config(&config).is_err());
        config.maximum_connections = 1;
        config.routing_portfolio.clear();
        assert!(validate_sequential_config(&config).is_err());

        let configured: KiCadSequentialRouterConfig =
            serde_json::from_str(r#"{"portfolio_policy":"ordered_first_admitted"}"#).unwrap();
        assert_eq!(
            configured.portfolio_policy,
            KiCadSequentialPortfolioPolicy::OrderedFirstAdmitted
        );
    }

    #[test]
    fn committing_candidate_keeps_result_board_and_native_reports_in_sync() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "pcb-maker-sequential-commit-{}-{nonce}",
            std::process::id()
        ));
        let native = directory.join("native");
        let result = directory.join("result");
        fs::create_dir_all(&native).unwrap();
        fs::create_dir_all(&result).unwrap();
        fs::write(native.join("board.kicad_pcb"), "new-board").unwrap();
        fs::write(result.join("board.kicad_pcb"), "old-board").unwrap();
        for report in ["erc.json", "drc.json", "verification.json"] {
            fs::write(native.join(report), format!("new-{report}")).unwrap();
            fs::write(result.join(report), format!("old-{report}")).unwrap();
        }

        commit_sequential_candidate(
            &native,
            &native.join("board.kicad_pcb"),
            &result,
            &result.join("board.kicad_pcb"),
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(result.join("board.kicad_pcb")).unwrap(),
            "new-board"
        );
        assert_eq!(
            fs::read_to_string(result.join("verification.json")).unwrap(),
            "new-verification.json"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ripup_recovery_requires_bounded_work_and_a_real_portfolio_member() {
        let mut config = KiCadSequentialRouterConfig::default();
        assert!(config.ripup.is_none());
        config.ripup = Some(KiCadSequentialRipupConfig::default());
        assert!(validate_sequential_config(&config).is_ok());
        config.ripup.as_mut().unwrap().maximum_invocations = 0;
        assert!(validate_sequential_config(&config).is_err());
        config.ripup.as_mut().unwrap().maximum_invocations = 1;
        config.ripup.as_mut().unwrap().maximum_diagnosis_trials = 0;
        assert!(validate_sequential_config(&config).is_err());
        config.ripup.as_mut().unwrap().maximum_diagnosis_trials = 64;
        config.ripup.as_mut().unwrap().routing_config_index = 1;
        assert!(validate_sequential_config(&config).is_err());
    }
}

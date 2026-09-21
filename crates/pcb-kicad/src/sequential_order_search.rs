// Copyright (C) 2026 Toit contributors.

use super::*;

const SEQUENTIAL_ORDER_SEARCH_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadSequentialOrderSearchConfig {
    pub sequential: KiCadSequentialRouterConfig,
    pub search: KiCadConnectionOrderSearchConfig,
    pub diagnosis: KiCadYieldingConnectionDiagnosisConfig,
}

impl KiCadSequentialOrderSearchConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.sequential.continue_after_routing_failure {
            return Err(
                "legacy order search requires a contiguous committed prefix, not a sparse sweep"
                    .into(),
            );
        }
        validate_sequential_config(&self.sequential)?;
        self.search.check()?;
        if self.diagnosis.minimum_yielding_connections == 0
            || self.diagnosis.minimum_yielding_connections
                > self.diagnosis.maximum_yielding_connections
            || self.diagnosis.maximum_yielding_connections == 0
            || self.diagnosis.maximum_yielding_connections > 2
            || self.diagnosis.maximum_trials == 0
        {
            return Err(
                "sequential order search diagnosis requires 1..=2 yielding connections and a positive trial bound"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct KiCadSequentialOrderTrial {
    pub ordinal: usize,
    pub parent_trial: Option<usize>,
    pub proposal: Option<KiCadConnectionOrderProposalEvidence>,
    pub directory: PathBuf,
    pub connection_order: Vec<String>,
    pub completed_connections: usize,
    pub remaining_unconnected_items: usize,
    pub complete: bool,
    pub score_mm: f64,
    pub selected: bool,
    pub sequential: KiCadSequentialRouterResult,
    pub diagnosis: Option<KiCadYieldingConnectionDiagnosis>,
}

#[derive(Debug, Serialize)]
pub struct KiCadSequentialOrderSearchResult {
    pub schema_version: u32,
    pub contract: String,
    pub source_directory: PathBuf,
    pub board_id: String,
    pub output_directory: PathBuf,
    pub config: KiCadSequentialOrderSearchConfig,
    pub selected_trial: usize,
    pub complete: bool,
    pub trials: Vec<KiCadSequentialOrderTrial>,
}

struct PendingOrderTrial {
    parent_trial: Option<usize>,
    connection_order: Option<Vec<String>>,
    proposal: Option<KiCadConnectionOrderProposalEvidence>,
}

/// Searches diagnosis-derived connection orders. Every trial starts from the
/// same zero-copper source; no placement or copper is inherited from a parent.
pub fn search_kicad_sequential_orders(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadSequentialOrderSearchConfig,
) -> Result<KiCadSequentialOrderSearchResult, String> {
    config.check()?;
    if output_directory.exists() {
        return Err(format!(
            "refusing to overwrite sequential order search {}",
            output_directory.display()
        ));
    }
    fs::create_dir_all(output_directory).map_err(|error| {
        format!(
            "failed to create sequential order search {}: {error}",
            output_directory.display()
        )
    })?;
    let mut queue = VecDeque::from([PendingOrderTrial {
        parent_trial: None,
        connection_order: (!config.sequential.connection_order.is_empty())
            .then(|| config.sequential.connection_order.clone()),
        proposal: None,
    }]);
    let mut queued_fingerprints = BTreeSet::new();
    if let Some(order) = queue[0].connection_order.as_ref() {
        queued_fingerprints.insert(connection_order_fingerprint(order));
    }
    let mut trials = Vec::new();

    while trials.len() < config.search.maximum_trials {
        let Some(pending) = queue.pop_front() else {
            break;
        };
        let ordinal = trials.len();
        let directory = output_directory.join(format!("trial-{ordinal:03}"));
        let mut sequential_config = config.sequential.clone();
        if let Some(order) = pending.connection_order {
            sequential_config.connection_order = order;
        }
        let sequential = route_kicad_board_sequentially(
            source_directory,
            board_id,
            &directory,
            &sequential_config,
        )?;
        let connection_order = sequential.connection_order.clone();
        queued_fingerprints.insert(connection_order_fingerprint(&connection_order));
        let remaining_unconnected_items = sequential_remaining_unconnected_items(&sequential)?;
        let complete = sequential.termination == KiCadSequentialRouterTermination::Complete;
        let score_mm = sequential
            .final_statistics
            .physical_copper
            .physical_centerline_length_mm
            + sequential.final_statistics.vias as f64 * config.search.via_penalty_mm;
        let diagnosis = if sequential.termination == KiCadSequentialRouterTermination::RoutingFailed
        {
            let failed = sequential
                .steps
                .last()
                .ok_or_else(|| "routing_failed trial has no failed step".to_string())?;
            Some(diagnose_kicad_yielding_connections(
                &sequential.result_board,
                &failed.connection,
                &config.diagnosis,
            )?)
        } else {
            None
        };
        let trial = KiCadSequentialOrderTrial {
            ordinal,
            parent_trial: pending.parent_trial,
            proposal: pending.proposal,
            directory: directory.clone(),
            connection_order: connection_order.clone(),
            completed_connections: sequential.completed_connections,
            remaining_unconnected_items,
            complete,
            score_mm,
            selected: false,
            sequential,
            diagnosis,
        };
        write_order_json(&directory.join("order-trial.json"), &trial)?;
        let children = diagnosed_children(&trial, config);
        trials.push(trial);
        if complete && config.search.stop_after_first_complete {
            break;
        }
        let mut retained = children
            .into_iter()
            .filter_map(|proposal| {
                let fingerprint = connection_order_fingerprint(&proposal.connection_order);
                queued_fingerprints
                    .insert(fingerprint)
                    .then_some(PendingOrderTrial {
                        parent_trial: Some(ordinal),
                        connection_order: Some(proposal.connection_order),
                        proposal: Some(proposal.evidence),
                    })
            })
            .collect::<Vec<_>>();
        match config.search.frontier_policy {
            KiCadConnectionOrderFrontierPolicy::BreadthFirst => queue.extend(retained),
            KiCadConnectionOrderFrontierPolicy::DepthFirst => {
                while let Some(child) = retained.pop() {
                    queue.push_front(child);
                }
            }
        }
    }
    if trials.is_empty() {
        return Err("sequential order search evaluated no trials".into());
    }
    let selected_trial = (0..trials.len())
        .min_by(|&left, &right| order_trial_key(&trials[left], &trials[right]))
        .expect("nonempty trial list");
    trials[selected_trial].selected = true;
    write_order_json(
        &trials[selected_trial].directory.join("order-trial.json"),
        &trials[selected_trial],
    )?;
    let result = KiCadSequentialOrderSearchResult {
        schema_version: SEQUENTIAL_ORDER_SEARCH_SCHEMA_VERSION,
        contract: "derive bounded children only from successful yielding counterfactuals; reroute every order independently from the identical zero-copper source; native-gate every retained connection; rank completion, then farther native connectivity, before route score".into(),
        source_directory: source_directory.to_path_buf(),
        board_id: board_id.into(),
        output_directory: output_directory.to_path_buf(),
        config: config.clone(),
        selected_trial,
        complete: trials[selected_trial].complete,
        trials,
    };
    write_order_json(
        &output_directory.join("sequential-order-search.json"),
        &result,
    )?;
    Ok(result)
}

fn diagnosed_children(
    trial: &KiCadSequentialOrderTrial,
    config: &KiCadSequentialOrderSearchConfig,
) -> Vec<KiCadConnectionOrderProposal> {
    let Some(diagnosis) = trial.diagnosis.as_ref() else {
        return Vec::new();
    };
    let mut children = Vec::new();
    let mut fingerprints = BTreeSet::new();
    for mode in &config.search.proposal_modes {
        for counterfactual in diagnosis.trials.iter().filter(|trial| trial.route_found) {
            for proposal in connection_order_proposals_for_yielding(
                &trial.connection_order,
                &diagnosis.target_connection,
                &counterfactual.yielding_connections,
                counterfactual.ordinal,
                *mode,
            ) {
                if fingerprints.insert(connection_order_fingerprint(&proposal.connection_order)) {
                    children.push(proposal);
                }
                if children.len() == config.search.maximum_children_per_failure {
                    return children;
                }
            }
        }
    }
    children
}

fn sequential_remaining_unconnected_items(
    result: &KiCadSequentialRouterResult,
) -> Result<usize, String> {
    let reductions = result
        .steps
        .iter()
        .filter(|step| step.committed())
        .map(|step| {
            result
                .discovered_connections
                .iter()
                .find(|summary| summary.connection == step.connection)
                .map(|summary| summary.electrical_terminal_count() - 1)
                .ok_or_else(|| format!("missing discovered connection {:?}", step.connection))
        })
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .sum::<usize>();
    result
        .expected_source_unconnected_items
        .checked_sub(reductions)
        .ok_or_else(|| "sequential order connectivity accounting underflowed".into())
}

fn order_trial_key(
    left: &KiCadSequentialOrderTrial,
    right: &KiCadSequentialOrderTrial,
) -> std::cmp::Ordering {
    right
        .complete
        .cmp(&left.complete)
        .then_with(|| right.completed_connections.cmp(&left.completed_connections))
        .then_with(|| {
            left.remaining_unconnected_items
                .cmp(&right.remaining_unconnected_items)
        })
        .then_with(|| left.score_mm.total_cmp(&right.score_mm))
        .then_with(|| left.ordinal.cmp(&right.ordinal))
}

fn write_order_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, format!("{serialized}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_search_config_rejects_unbounded_diagnosis() {
        let mut config = KiCadSequentialOrderSearchConfig {
            sequential: KiCadSequentialRouterConfig::default(),
            search: KiCadConnectionOrderSearchConfig {
                maximum_trials: 2,
                maximum_children_per_failure: 1,
                proposal_modes: vec![KiCadConnectionOrderProposalMode::MoveTargetBeforeYielding],
                frontier_policy: KiCadConnectionOrderFrontierPolicy::BreadthFirst,
                stop_after_first_complete: true,
                via_penalty_mm: 2.0,
            },
            diagnosis: KiCadYieldingConnectionDiagnosisConfig::default(),
        };
        config.check().unwrap();
        config.sequential.continue_after_routing_failure = true;
        assert!(config.check().unwrap_err().contains("sparse sweep"));
        config.sequential.continue_after_routing_failure = false;
        config.diagnosis.maximum_trials = 0;
        assert!(config.check().is_err());
    }
}

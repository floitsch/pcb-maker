// Copyright (C) 2026 Toit contributors.

//! Restore displaced nets with a shared bounded budget and enclosing admission.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadRestorationRecoveryConfig {
    pub maximum_invocations: usize,
    pub maximum_depth: usize,
    pub maximum_diagnosis_trials: usize,
    /// Optional search profile. Current demand and native connection rules
    /// are inherited from the enclosing repair, then completed demand is removed.
    pub routing: Option<KiCadGridRouteConfig>,
}

impl Default for KiCadRestorationRecoveryConfig {
    fn default() -> Self {
        Self {
            maximum_invocations: 1,
            maximum_depth: 1,
            maximum_diagnosis_trials: 64,
            routing: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadChangedRouteReceipt {
    pub candidate: PathBuf,
    pub candidate_sha256: String,
    pub quality: KiCadRouteQuality,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadRestorationEvidence {
    pub directory: PathBuf,
    pub depth: usize,
    pub invocation_ordinal: usize,
    pub selected_attempt: Option<usize>,
    pub route_expansions: u64,
    pub error: Option<String>,
    pub trigger_error: Option<String>,
}

#[derive(Default)]
pub(super) struct Budget {
    pub invocations: usize,
}

impl Budget {
    fn claim(
        &mut self,
        config: &KiCadRestorationRecoveryConfig,
        depth: usize,
    ) -> Result<usize, &'static str> {
        if depth >= config.maximum_depth {
            return Err("maximum restoration depth reached");
        }
        if self.invocations >= config.maximum_invocations {
            return Err("maximum restoration invocations reached");
        }
        let ordinal = self.invocations;
        self.invocations += 1;
        Ok(ordinal)
    }
}

pub(super) fn validate(config: &KiCadSingleConnectionRipupConfig) -> Result<(), String> {
    if let Some(recovery) = &config.restoration {
        if !config.allow_partial_progress {
            return Err("nested restoration requires partial-progress admission".into());
        }
        if !(1..=16).contains(&recovery.maximum_invocations)
            || !(1..=3).contains(&recovery.maximum_depth)
            || !(1..=1024).contains(&recovery.maximum_diagnosis_trials)
        {
            return Err("invalid bounded restoration recovery configuration".into());
        }
        if let Some(routing) = &recovery.routing {
            validate_grid_route_config(routing)?;
        }
    }
    Ok(())
}

pub(super) fn receipt(path: &Path) -> Result<KiCadChangedRouteReceipt, String> {
    Ok(KiCadChangedRouteReceipt {
        candidate: path.canonicalize().map_err(|e| e.to_string())?,
        candidate_sha256: file_sha256(path)?,
        quality: route_candidate_quality(&read_route_candidate(path)?)?,
    })
}

pub(super) fn check_receipt(net: &str, receipt: &KiCadChangedRouteReceipt) -> Result<(), String> {
    let candidate = read_route_candidate(&receipt.candidate)?;
    if normalize_net(&candidate.connection) != normalize_net(net)
        || file_sha256(&receipt.candidate)? != receipt.candidate_sha256
        || serde_json::to_value(route_candidate_quality(&candidate)?).map_err(|e| e.to_string())?
            != serde_json::to_value(&receipt.quality).map_err(|e| e.to_string())?
    {
        return Err("changed-route receipt disagrees with its candidate".into());
    }
    Ok(())
}

pub(super) fn recover(
    source: &Path,
    board_id: &str,
    target: &str,
    baseline_drc: &serde_json::Value,
    expected: Option<usize>,
    config: &KiCadSingleConnectionRipupConfig,
    budget: &mut Budget,
    depth: usize,
    protected: &[String],
    attempt: &mut KiCadSingleConnectionRipupAttempt,
) -> Result<(), String> {
    let Some(policy) = &config.restoration else {
        return Ok(());
    };
    let invocation = match budget.claim(policy, depth) {
        Ok(ordinal) => ordinal,
        Err(reason) => {
            attempt.restoration_skip_reason = Some(reason.into());
            return Ok(());
        }
    };
    let directory = attempt
        .artifact_directory
        .with_file_name(format!("restoration-{:03}", attempt.ordinal));
    let mut evidence = KiCadRestorationEvidence {
        directory: directory.clone(),
        depth: depth + 1,
        invocation_ordinal: invocation,
        selected_attempt: None,
        route_expansions: 0,
        error: None,
        trigger_error: attempt.error.clone(),
    };
    let outcome = (|| -> Result<(), String> {
        let mut child = config.clone();
        child.diagnosis.maximum_trials = policy.maximum_diagnosis_trials;
        child.diagnosis.routing = policy
            .routing
            .clone()
            .unwrap_or_else(|| config.diagnosis.routing.clone());
        child.diagnosis.routing.connection_rules =
            config.diagnosis.routing.connection_rules.clone();
        child.diagnosis.routing.routing_demand = config.diagnosis.routing.routing_demand.clone();
        let mut ancestors = protected.to_vec();
        ancestors.push(target.to_string());
        routing_demand::remove_completed(&mut child.diagnosis.routing, &ancestors);
        write_typed_json(&directory.with_extension("config.json"), &child)?;
        let provisional = attempt
            .provisional_directory
            .as_ref()
            .ok_or("restoration lacks provisional snapshot")?;
        let result = connection_insertion::repair_with_restoration_budget(
            provisional,
            board_id,
            &attempt.yielding_connection,
            &directory,
            &child,
            budget,
            depth + 1,
            &ancestors,
        )?;
        evidence.route_expansions = result.total_route_expansions;
        evidence.selected_attempt = result.selected_attempt;
        let chosen = result
            .selected_attempt
            .map(|index| &result.attempts[index])
            .ok_or("no nested repair restored every displaced net")?;
        let mut changed = chosen.changed_routes.clone();
        changed.insert(
            normalize_net(target).to_string(),
            receipt(&attempt.target_candidate)?,
        );
        for (net, row) in &changed {
            check_receipt(net, row)?;
        }
        let verification = chosen
            .final_verification
            .as_ref()
            .ok_or("nested selection has no native verification")?;
        let names: Vec<_> = changed.keys().map(String::as_str).collect();
        if !partial_ripup::final_admissible(
            verification,
            &read_json(&chosen.artifact_directory.join("drc.json"))?,
            baseline_drc,
            expected.ok_or("nested restoration lacks enclosing connectivity ledger")?,
            config.allow_existing_annotation_findings,
        )? || !partial_ripup::preserved_connections(
            source,
            &chosen.artifact_directory,
            board_id,
            &names,
        )? {
            return Err("nested repair failed enclosing native or fixed-context admission".into());
        }
        let board = format!("{board_id}.kicad_pcb");
        // Only the board and its verified reports/preview are promoted. Keep
        // parent and child candidate journals separate and immutable.
        sequential_router::commit_sequential_candidate(
            &chosen.artifact_directory,
            &chosen.artifact_directory.join(&board),
            &attempt.artifact_directory,
            &attempt.artifact_directory.join(&board),
        )?;
        let restored = changed
            .get(normalize_net(&attempt.yielding_connection))
            .ok_or("nested selection lacks displaced target receipt")?;
        attempt.rerouted_candidate = Some(restored.candidate.clone());
        attempt.rerouted_quality = Some(restored.quality.clone());
        attempt.combined_score_mm = Some(
            changed
                .values()
                .map(|r| r.quality.length_mm + r.quality.vias as f64 * config.via_penalty_mm)
                .sum(),
        );
        let stats = inspect_kicad_board(&attempt.artifact_directory.join(&board))?;
        attempt.selection_score_mm = Some(
            stats.physical_copper.physical_centerline_length_mm
                + stats.vias as f64 * config.via_penalty_mm,
        );
        attempt.changed_routes = changed;
        attempt.final_verification = Some(verification.clone());
        attempt.status = if verification.complete {
            KiCadSingleConnectionRipupStatus::Complete
        } else {
            KiCadSingleConnectionRipupStatus::Progress
        };
        attempt.error = None;
        Ok(())
    })();
    if let Err(error) = outcome {
        evidence.error = Some(error);
    }
    attempt.restoration = Some(evidence);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_is_shared_across_siblings_and_depth_cannot_reset_it() {
        let config = KiCadRestorationRecoveryConfig {
            maximum_invocations: 2,
            maximum_depth: 2,
            ..Default::default()
        };
        let mut budget = Budget::default();
        assert_eq!(budget.claim(&config, 0), Ok(0));
        assert_eq!(
            budget.claim(&config, 2),
            Err("maximum restoration depth reached")
        );
        assert_eq!(budget.claim(&config, 1), Ok(1));
        assert_eq!(
            budget.claim(&config, 0),
            Err("maximum restoration invocations reached")
        );
        assert_eq!(budget.invocations, 2);
    }

    #[test]
    fn nested_recovery_is_opt_in_and_requires_the_strict_progress_gate() {
        let mut config = KiCadSingleConnectionRipupConfig::default();
        assert!(validate(&config).is_ok());
        config.restoration = Some(Default::default());
        assert!(validate(&config).is_err());
        config.allow_partial_progress = true;
        assert!(validate(&config).is_ok());
        config.restoration.as_mut().unwrap().maximum_depth = 0;
        assert!(validate(&config).is_err());
    }
}

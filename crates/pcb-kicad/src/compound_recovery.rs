// Copyright (C) 2026 Toit contributors.

//! Bounded two-net displacement followed by transactional restoration.
//! Intermediate boards are evidence only; admission uses the original parent.
use super::*;
use super::connection_insertion::directory_sha256;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadCompoundRecoveryConfig {
    pub maximum_pair_trials: usize,
    pub maximum_pair_candidates: usize,
    pub maximum_restoration_repairs: usize,
    pub maximum_restoration_diagnosis_trials: usize,
    pub restoration: KiCadRestorationRecoveryConfig,
}

impl Default for KiCadCompoundRecoveryConfig {
    fn default() -> Self {
        Self {
            maximum_pair_trials: 32,
            maximum_pair_candidates: 2,
            maximum_restoration_repairs: 2,
            maximum_restoration_diagnosis_trials: 4,
            restoration: KiCadRestorationRecoveryConfig {
                maximum_diagnosis_trials: 4,
                ..Default::default()
            },
        }
    }
}

pub(super) fn validate(policy: &KiCadCompoundRecoveryConfig) -> Result<(), String> {
    if !(1..=4096).contains(&policy.maximum_pair_trials)
        || !(1..=16).contains(&policy.maximum_pair_candidates)
        || !(1..=32).contains(&policy.maximum_restoration_repairs)
        || !(1..=1024).contains(&policy.maximum_restoration_diagnosis_trials)
    {
        return Err("invalid bounded compound recovery configuration".into());
    }
    restoration_recovery::validate(&KiCadSingleConnectionRipupConfig {
        allow_partial_progress: true,
        restoration: Some(policy.restoration.clone()),
        ..Default::default()
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadCompoundStage {
    pub connection: String,
    pub directory: PathBuf,
    pub candidate: Option<PathBuf>,
    pub route_failure: Option<KiCadRouteSearchFailure>,
    pub error: Option<String>,
    pub verification: Option<VerificationReport>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadCompoundAttempt {
    pub ordinal: usize,
    pub pair_trial: usize,
    pub order: Vec<String>,
    pub provisional_directory: PathBuf,
    pub target_candidate: PathBuf,
    pub stages: Vec<KiCadCompoundStage>,
    pub restoration_directory: Option<PathBuf>,
    pub restoration_selected_attempt: Option<usize>,
    pub final_directory: Option<PathBuf>,
    pub final_verification: Option<VerificationReport>,
    pub changed_routes: BTreeMap<String, KiCadChangedRouteReceipt>,
    pub error: Option<String>,
    pub selected: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadCompoundRecoveryResult {
    pub schema_version: u32,
    pub board_id: String,
    pub target_connection: String,
    pub source_directory: PathBuf,
    pub source_snapshot: PathBuf,
    pub source_sha256: String,
    pub source_unchanged: bool,
    pub expected_unconnected_items: usize,
    pub diagnosis: KiCadYieldingConnectionDiagnosis,
    pub attempts: Vec<KiCadCompoundAttempt>,
    pub selected_attempt: Option<usize>,
    pub restoration_repairs: usize,
    pub nested_restoration_invocations: usize,
    pub total_route_expansions: u64,
    pub contract: &'static str,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct KiCadSequentialCompoundEvidence {
    pub directory: PathBuf,
    pub selected_attempt: Option<usize>,
    pub restoration_repairs: usize,
    pub nested_restoration_invocations: usize,
    pub route_expansions: u64,
    pub error: Option<String>,
}

pub(super) fn recover(
    source: &Path,
    board_id: &str,
    target: &str,
    output: &Path,
    base: &KiCadSingleConnectionRipupConfig,
    policy: &KiCadCompoundRecoveryConfig,
) -> Result<KiCadCompoundRecoveryResult, String> {
    validate(policy)?;
    if !base.allow_partial_progress {
        return Err("compound recovery requires partial-progress admission".into());
    }
    if output.exists() {
        return Err("refusing to overwrite compound recovery evidence".into());
    }
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let destination = output
        .parent()
        .ok_or("compound output has no parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(output.file_name().ok_or("invalid compound output")?);
    if destination.starts_with(&source) {
        return Err("compound output must be outside its source".into());
    }
    let before = directory_sha256(&source)?;
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let snapshot = output.join("source-unrouted-target");
    copy_directory_tree(&source, &snapshot)?;
    if directory_sha256(&snapshot)? != before {
        return Err("compound source snapshot differs".into());
    }
    let board = format!("{board_id}.kicad_pcb");
    let baseline = output.join("baseline-verification");
    copy_directory_tree(&snapshot, &baseline)?;
    let baseline_erc_input_sha256 = kicad_erc_input_sha256(&baseline)?;
    let native = verify_materialized_rung(&baseline, board_id)?;
    let baseline_erc = read_json(&baseline.join("erc.json"))?;
    let baseline_drc = read_json(&baseline.join("drc.json"))?;
    if !adaptive_routing::native_progress_admissible(
        &native,
        &baseline_drc,
        &baseline_drc,
        base.allow_existing_annotation_findings,
    )? {
        return Err("compound source failed native admission".into());
    }
    let expected = partial_ripup::expected_opens(&snapshot.join(&board), target, &native)?;
    let mut diagnosis_config = base.diagnosis.clone();
    // The initial pair family follows changed-route history. Fresh boundary
    // evidence is used when restoring a displaced net on the changed board.
    diagnosis_config.prioritize_boundary_blockers = false;
    diagnosis_config.minimum_yielding_connections = 2;
    diagnosis_config.maximum_yielding_connections = 2;
    diagnosis_config.maximum_trials = policy.maximum_pair_trials;
    write_typed_json(&output.join("diagnosis-config.json"), &diagnosis_config)?;
    let diagnosis =
        diagnose_kicad_yielding_connections(&snapshot.join(&board), target, &diagnosis_config)?;
    let mut candidates: Vec<_> = diagnosis
        .trials
        .iter()
        .filter(|t| t.route_found)
        .map(|t| t.ordinal)
        .collect();
    let score = |i: usize| {
        let q = diagnosis.trials[i]
            .quality
            .as_ref()
            .expect("successful diagnosis quality");
        q.length_mm + q.vias as f64 * base.via_penalty_mm
    };
    candidates.sort_by(|&a, &b| score(a).total_cmp(&score(b)).then(a.cmp(&b)));
    candidates.truncate(policy.maximum_pair_candidates);
    let mut result = KiCadCompoundRecoveryResult {
        schema_version: 1,
        board_id: board_id.into(),
        target_connection: target.into(),
        source_directory: source.clone(),
        source_snapshot: snapshot.clone(),
        source_sha256: before.clone(),
        source_unchanged: false,
        expected_unconnected_items: expected,
        total_route_expansions: diagnosis.total_route_expansions,
        diagnosis,
        attempts: Vec::new(),
        selected_attempt: None,
        restoration_repairs: 0,
        nested_restoration_invocations: 0,
        contract: "Bounded pair diagnosis follows recent changes; target candidates are ordered by declared length/via score. Try both restoration orders. The first displaced net must route directly; the last may use bounded nested boundary-guided repair. No intermediate is committed. Stop at the first result passing original-parent native connectivity and fixed-context checks; retain final route receipts for every changed net.",
    };
    write_typed_json(&output.join("compound.json"), &result)?;
    for trial_index in candidates {
        let trial = &result.diagnosis.trials[trial_index];
        let yielding = trial.yielding_connections.clone();
        if yielding.len() != 2 {
            return Err("pair diagnosis returned a non-pair".into());
        }
        let pair = output.join(format!("pair-{trial_index:03}"));
        fs::create_dir_all(&pair).map_err(|e| e.to_string())?;
        let candidate_path = pair.join("target-candidate.json");
        write_typed_json(
            &candidate_path,
            trial
                .candidate
                .as_ref()
                .ok_or("pair lacks route candidate")?,
        )?;
        let provisional = pair.join("provisional");
        copy_directory_tree(&snapshot, &provisional)?;
        apply_route_candidate_with_yielding_connections(
            &snapshot.join(&board),
            &candidate_path,
            &yielding,
            &provisional.join(&board),
        )?;
        let verification = verify_materialized_route_only(
            &provisional,
            board_id,
            &baseline_erc_input_sha256,
            &baseline_erc,
        )?;
        let allowed = [target, yielding[0].as_str(), yielding[1].as_str()];
        if !adaptive_routing::native_progress_admissible(
            &verification,
            &read_json(&provisional.join("drc.json"))?,
            &baseline_drc,
            base.allow_existing_annotation_findings,
        )? || !partial_ripup::preserved_connections(&snapshot, &provisional, board_id, &allowed)?
        {
            let ordinal = result.attempts.len();
            result.attempts.push(KiCadCompoundAttempt {
                ordinal,
                pair_trial: trial_index,
                order: Vec::new(),
                provisional_directory: provisional,
                target_candidate: candidate_path,
                stages: Vec::new(),
                restoration_directory: None,
                restoration_selected_attempt: None,
                final_directory: None,
                final_verification: None,
                changed_routes: BTreeMap::new(),
                error: Some("pair counterfactual failed native or fixed-context checks".into()),
                selected: false,
            });
            write_typed_json(&output.join("compound.json"), &result)?;
            continue;
        }
        for order in [
            yielding.clone(),
            vec![yielding[1].clone(), yielding[0].clone()],
        ] {
            let ordinal = result.attempts.len();
            let mut attempt = KiCadCompoundAttempt {
                ordinal,
                pair_trial: trial_index,
                order,
                provisional_directory: provisional.clone(),
                target_candidate: candidate_path.clone(),
                stages: Vec::new(),
                restoration_directory: None,
                restoration_selected_attempt: None,
                final_directory: None,
                final_verification: None,
                changed_routes: BTreeMap::new(),
                error: None,
                selected: false,
            };
            if let Err(error) = restore_order(
                &snapshot,
                board_id,
                target,
                output,
                base,
                policy,
                &baseline_drc,
                &baseline_erc_input_sha256,
                &baseline_erc,
                &mut result,
                &mut attempt,
            ) {
                attempt.error = Some(error);
            }
            if attempt.selected {
                result.selected_attempt = Some(ordinal);
            }
            result.attempts.push(attempt);
            write_typed_json(&output.join("compound.json"), &result)?;
            if result.selected_attempt.is_some() {
                break;
            }
        }
        if result.selected_attempt.is_some() {
            break;
        }
    }
    result.source_unchanged = directory_sha256(&source)? == before;
    write_typed_json(&output.join("compound.json"), &result)?;
    if !result.source_unchanged {
        return Err("compound recovery changed its source".into());
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn restore_order(
    source: &Path,
    board_id: &str,
    target: &str,
    output: &Path,
    base: &KiCadSingleConnectionRipupConfig,
    policy: &KiCadCompoundRecoveryConfig,
    baseline_drc: &serde_json::Value,
    baseline_erc_input_sha256: &str,
    baseline_erc: &serde_json::Value,
    result: &mut KiCadCompoundRecoveryResult,
    attempt: &mut KiCadCompoundAttempt,
) -> Result<(), String> {
    let directory = output.join(format!("attempt-{:03}", attempt.ordinal));
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let board = format!("{board_id}.kicad_pcb");
    let mut parent = attempt.provisional_directory.clone();
    let mut changed = BTreeMap::from([(
        normalize_net(target).to_string(),
        restoration_recovery::receipt(&attempt.target_candidate)?,
    )]);
    let mut routing = base.diagnosis.routing.clone();
    routing_demand::remove_completed(&mut routing, &[target.to_string()]);
    for (index, net) in attempt.order.iter().enumerate() {
        let stage = directory.join(format!("stage-{index}"));
        copy_directory_tree(&parent, &stage)?;
        let candidate = directory.join(format!("route-{index}.json"));
        let mut evidence = KiCadCompoundStage {
            connection: net.clone(),
            directory: stage.clone(),
            candidate: None,
            route_failure: None,
            error: None,
            verification: None,
        };
        let route = route_materialized_connection_detailed(&parent.join(&board), net, &routing);
        match route {
            Ok(candidate_value) => {
                result.total_route_expansions += u64::from(candidate_value.expansions);
                write_typed_json(&candidate, &candidate_value)?;
                apply_route_candidate(&parent.join(&board), &candidate, &stage.join(&board))?;
                evidence.candidate = Some(candidate.clone());
            }
            Err(error) => {
                evidence.route_failure = error.search_failure().cloned();
                result.total_route_expansions += u64::from(
                    evidence
                        .route_failure
                        .as_ref()
                        .map_or(0, |f| f.astar_expansions),
                );
                evidence.error = Some(error.to_string());
            }
        }
        let native = verify_materialized_route_only(
            &stage,
            board_id,
            baseline_erc_input_sha256,
            baseline_erc,
        )?;
        let valid = adaptive_routing::native_progress_admissible(
            &native,
            &read_json(&stage.join("drc.json"))?,
            baseline_drc,
            base.allow_existing_annotation_findings,
        )?;
        evidence.verification = Some(native);
        let routed = evidence.candidate.is_some();
        attempt.stages.push(evidence);
        if !valid {
            return Err("restoration stage failed native checks".into());
        }
        if routed {
            changed.insert(
                normalize_net(net).into(),
                restoration_recovery::receipt(&candidate)?,
            );
            parent = stage;
            routing_demand::remove_completed(&mut routing, std::slice::from_ref(net));
            continue;
        }
        if index == 0 {
            return Err("first displaced net did not route directly; try the other order".into());
        }
        if result.restoration_repairs >= policy.maximum_restoration_repairs {
            return Err("compound restoration-repair budget exhausted".into());
        }
        result.restoration_repairs += 1;
        let mut repair = base.clone();
        repair.diagnosis.prioritize_boundary_blockers = true;
        repair.diagnosis.minimum_yielding_connections = 1;
        repair.diagnosis.maximum_yielding_connections = 1;
        repair.diagnosis.maximum_trials = policy.maximum_restoration_diagnosis_trials;
        repair
            .diagnosis
            .preferred_yielding_connections
            .insert(0, target.into());
        repair.diagnosis.routing = routing.clone();
        repair.restoration = Some(policy.restoration.clone());
        let repair_directory = directory.join("restoration");
        write_typed_json(&directory.join("restoration-config.json"), &repair)?;
        attempt.restoration_directory = Some(repair_directory.clone());
        let restored = repair_kicad_with_single_connection_ripup(
            &stage,
            board_id,
            net,
            &repair_directory,
            &repair,
        )?;
        result.total_route_expansions += restored.total_route_expansions;
        result.nested_restoration_invocations += restored.restoration_invocations;
        attempt.restoration_selected_attempt = restored.selected_attempt;
        let selected = restored
            .selected_attempt
            .ok_or("bounded repair did not restore the displaced net")?;
        let chosen = &restored.attempts[selected];
        // Child receipts replace earlier versions, including a rerouted target.
        changed.extend(chosen.changed_routes.clone());
        parent = chosen.artifact_directory.clone();
    }
    for (net, receipt) in &changed {
        restoration_recovery::check_receipt(net, receipt)?;
    }
    let native =
        verify_materialized_route_only(&parent, board_id, baseline_erc_input_sha256, baseline_erc)?;
    let names: Vec<_> = changed.keys().map(String::as_str).collect();
    if !partial_ripup::final_admissible(
        &native,
        &read_json(&parent.join("drc.json"))?,
        baseline_drc,
        result.expected_unconnected_items,
        base.allow_existing_annotation_findings,
    )? || !partial_ripup::preserved_connections(source, &parent, board_id, &names)?
    {
        return Err("compound result failed original-parent admission".into());
    }
    attempt.changed_routes = changed;
    attempt.final_directory = Some(parent);
    attempt.final_verification = Some(native);
    attempt.selected = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compound_limits_are_finite_and_nested_limits_still_apply() {
        let mut p = KiCadCompoundRecoveryConfig::default();
        assert!(validate(&p).is_ok());
        p.maximum_restoration_repairs = 0;
        assert!(validate(&p).is_err());
        p.maximum_restoration_repairs = 2;
        p.maximum_pair_trials = 4097;
        assert!(validate(&p).is_err());
        p.maximum_pair_trials = 16;
        p.restoration.maximum_depth = 4;
        assert!(validate(&p).is_err());
    }
}

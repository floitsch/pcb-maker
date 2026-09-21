// Copyright (C) 2026 Toit contributors.

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadCoupledViaConfig {
    pub via_actions: KiCadViaActionSearchConfig,
    pub reroute: KiCadGridRouteConfig,
    pub maximum_coupled_trials: usize,
}

pub(super) fn fixed_design(mut pcb: Expr) -> Expr {
    if let Expr::List(items) = &mut pcb {
        items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    }
    pcb
}

fn quality_score(stats: &KiCadBoardStatistics, penalty: f64) -> f64 {
    stats.stored_segment_length_mm + stats.vias as f64 * penalty
}

/// Convert successful foreign-copper counterfactuals into complete routed
/// transactions. Missing nets are never accepted as improvements.
pub fn refine_kicad_vias(
    source: &Path,
    board_id: &str,
    candidate_path: &Path,
    output: &Path,
    config: &KiCadCoupledViaConfig,
) -> Result<serde_json::Value, String> {
    validate_via_action_config(&config.via_actions)?;
    validate_grid_route_config(&config.reroute)?;
    if config.maximum_coupled_trials == 0 || config.maximum_coupled_trials > 64 {
        return Err("maximum_coupled_trials must be within 1..=64".into());
    }
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let output = output
        .parent()
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(output.file_name().ok_or("output needs a name")?);
    if output.exists() || output.starts_with(&source) {
        return Err("output must be fresh and outside source".into());
    }
    let source_board = source.join(format!("{board_id}.kicad_pcb"));
    let source_bytes = fs::read(&source_board).map_err(|e| e.to_string())?;
    let original = inspect_kicad_board(&source_board)?;
    if original.arcs != 0 || original.copper_zones != 0 {
        return Err(
            "coupled via refinement currently scores straight-track boards without copper zones"
                .into(),
        );
    }
    let candidate = read_route_candidate(candidate_path)?;
    if !candidate.footprint_placements.is_empty() || !candidate.reference_placements.is_empty() {
        return Err(
            "via refinement requires a candidate with fixed footprint and reference poses".into(),
        );
    }
    fs::create_dir(&output).map_err(|e| e.to_string())?;
    write_pretty_json(&output.join("config.json"), config)?;
    // Importing a route may trim copper inside pads. The untouched original
    // remains the cost baseline and fallback, even when its candidate has a
    // different (electrically equivalent) serialization.
    let baseline = output.join("source");
    copy_directory_tree(&source, &baseline)?;
    let original_verification = verify_materialized_rung(&baseline, board_id)?;
    if !original_verification.complete {
        return Err("via refinement requires a native-complete original board".into());
    }
    let independent = output.join("independent");
    let (_, portfolio) = search_via_topology_actions(
        &baseline,
        board_id,
        candidate_path,
        &independent,
        &config.via_actions,
    )?;
    write_pretty_json(&independent.join("portfolio.json"), &portfolio)?;
    if !portfolio.source_verification.complete {
        return Err("via refinement requires a native-complete source candidate".into());
    }
    let baseline_board = baseline.join(format!("{board_id}.kicad_pcb"));
    let baseline_stats = inspect_kicad_board(&baseline_board)?;
    let fixed = fixed_design(parse(
        &fs::read_to_string(&baseline_board).map_err(|e| e.to_string())?,
    )?);
    if fixed
        != fixed_design(parse(
            &String::from_utf8(source_bytes.clone()).map_err(|e| e.to_string())?,
        )?)
    {
        return Err("source candidate changes fixed design".into());
    }
    let project = baseline.join(format!("{board_id}.kicad_pro"));
    let project_bytes = fs::read(&project).ok();
    let penalty = config.via_actions.via_penalty_mm;
    let mut selected = baseline.clone();
    let mut selected_stats = baseline_stats.clone();
    let mut selected_score = quality_score(&selected_stats, penalty);
    let model = KiCadRoutingModel::from_pcb_with_config(
        &parse(&fs::read_to_string(&baseline_board).map_err(|e| e.to_string())?)?,
        &candidate.connection,
        &candidate.config,
    )?;
    let mut attempts = Vec::new();
    // Native-complete independent changes also compete on whole-board cost.
    for attempt in &portfolio.attempts {
        if attempt.verification.as_ref().is_some_and(|v| v.complete) {
            if let Some(directory) = &attempt.artifact_directory {
                let directory = independent.join(directory);
                let stats = inspect_kicad_board(&directory.join(format!("{board_id}.kicad_pcb")))?;
                let score = quality_score(&stats, penalty);
                if score + config.via_actions.minimum_score_improvement_mm < selected_score {
                    selected = directory;
                    selected_stats = stats;
                    selected_score = score;
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    'actions: for attempt in &portfolio.attempts {
        let KiCadViaTopologyAction::RemoveAndReroute {
            merged_layer,
            resolution_mm,
            ..
        } = &attempt.action
        else {
            continue;
        };
        let Some(cut) = attempt
            .local_reroute
            .as_ref()
            .and_then(|l| l.blocker_cut.as_ref())
        else {
            continue;
        };
        for trial in &cut.trials {
            if trial.status != KiCadViaBlockerCutTrialStatus::Found
                || trial.suppressed.is_empty()
                || trial
                    .suppressed
                    .iter()
                    .any(|b| b.kind != KiCadViaLocalRerouteBlockerKind::ForeignNetCopper)
            {
                continue;
            }
            let nets = trial
                .suppressed
                .iter()
                .map(|b| b.object.clone())
                .collect::<Vec<_>>();
            if !seen.insert((merged_layer.clone(), resolution_mm.to_bits(), nets.clone())) {
                continue;
            }
            if attempts.len() >= config.maximum_coupled_trials {
                break 'actions;
            }
            let directory = output.join(format!("coupled-{:03}", attempts.len()));
            copy_directory_tree(&baseline, &directory)?;
            let board = directory.join(format!("{board_id}.kicad_pcb"));
            let filtered = local_reroute_model_without_blockers(&model, &trial.suppressed);
            let mut search = config
                .via_actions
                .local_reroute
                .clone()
                .ok_or("local reroute configuration missing")?;
            search.blocker_cut = None;
            let mut target_path_policy = "counterfactual_local_route";
            let work = (|| -> Result<(), String> {
                let (mut target, _, _) = apply_remove_and_local_reroute(
                    &candidate,
                    &filtered,
                    &config.via_actions,
                    &search,
                    merged_layer,
                    *resolution_mm,
                )
                .map_err(|e| e.detail)?;
                // A grid detour is not needed if the direct merged chord is
                // exact-clear after yielding the same diagnosed copper.
                if let Ok((direct, affected)) = apply_via_topology_action(
                    &candidate,
                    &config.via_actions,
                    &KiCadViaTopologyAction::Remove {
                        merged_layer: merged_layer.clone(),
                    },
                ) {
                    if selected_geometry_blockers(&direct, &affected, &filtered).is_empty()
                        && candidate_via_blockers(&direct, &filtered).is_empty()
                        && route_candidate_quality(&direct)?.length_mm
                            < route_candidate_quality(&target)?.length_mm
                    {
                        target = direct;
                        target_path_policy = "exact_clear_direct_chord";
                    }
                }
                let path = directory.join("target-candidate.json");
                write_pretty_json(&path, &target)?;
                apply_route_candidate_with_yielding_connections(&board, &path, &nets, &board)?;
                // The provisional board is rendered and explicitly remains
                // unselectable until every yielded connection is restored.
                verify_materialized_rung(&directory, board_id)?;
                let provisional = directory.join("provisional");
                fs::create_dir(&provisional).map_err(|e| e.to_string())?;
                verification_preview::copy_to(&directory, &provisional);
                fs::copy(&board, provisional.join(format!("{board_id}.kicad_pcb")))
                    .map_err(|e| e.to_string())?;
                for (i, net) in nets.iter().enumerate() {
                    let routed = route_materialized_connection(&board, net, &config.reroute)?;
                    let path = directory.join(format!("rerouted-{i:02}.json"));
                    write_pretty_json(&path, &routed)?;
                    apply_route_candidate(&board, &path, &board)?;
                }
                Ok(())
            })();
            let verification = verify_materialized_rung(&directory, board_id)?;
            let stats = inspect_kicad_board(&board)?;
            let preserved = fixed
                == fixed_design(parse(
                    &fs::read_to_string(&board).map_err(|e| e.to_string())?,
                )?)
                && fs::read(directory.join(format!("{board_id}.kicad_pro"))).ok() == project_bytes;
            let score = quality_score(&stats, penalty);
            let accepted = work.is_ok()
                && verification.complete
                && preserved
                && score + config.via_actions.minimum_score_improvement_mm < selected_score;
            if accepted {
                selected = directory.clone();
                selected_stats = stats.clone();
                selected_score = score;
            }
            attempts.push(
                serde_json::json!({"directory":directory,"yielded_connections":nets,
                "target_path_policy":target_path_policy,
                "merged_layer":merged_layer,"resolution_mm":resolution_mm,"error":work.err(),
                "native":verification,"fixed_design_and_project_preserved":preserved,
                "statistics":stats,"score_mm":score,"improved_incumbent":accepted}),
            );
            write_pretty_json(&output.join("attempts.json"), &attempts)?;
        }
    }
    if fs::read(&source_board).map_err(|e| e.to_string())? != source_bytes {
        return Err("source board changed".into());
    }
    copy_directory_tree(&selected, &output.join("result"))?;
    let report = serde_json::json!({"source":source,"result":output.join("result"),"selected":selected,
        "source_statistics":baseline_stats,"result_statistics":selected_stats,
        "via_penalty_mm":penalty,"source_score_mm":quality_score(&baseline_stats,penalty),
        "result_score_mm":selected_score,"attempts":attempts,"source_unchanged":true,
        "selection":"Native-complete whole boards only; stored track length plus explicit per-via penalty; all fixed design and project settings retained."});
    write_pretty_json(&output.join("refinement.json"), &report)?;
    Ok(report)
}

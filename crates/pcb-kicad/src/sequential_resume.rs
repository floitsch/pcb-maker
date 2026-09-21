// Copyright (C) 2026 Toit contributors.

//! Fresh native admission of immutable committed routing checkpoints.
use super::*;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadSequentialResumeConfig {
    pub sequential: KiCadSequentialRouterConfig,
    pub allow_existing_annotation_findings: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadSequentialResumeEvidence {
    pub checkpoint_directory: PathBuf,
    pub checkpoint_report_sha256: String,
    pub initial_completed_connections: usize,
    pub initial_unconnected_items: usize,
    pub retained_board_sha256: String,
    pub mutable_result_matches_commit: bool,
    pub snapshot_directory: PathBuf,
    pub verification: VerificationReport,
    pub contract: String,
}

pub fn resume_kicad_board_sequentially(
    checkpoint: &Path,
    output: &Path,
    config: &KiCadSequentialResumeConfig,
) -> Result<KiCadSequentialRouterResult, String> {
    let checkpoint = checkpoint.canonicalize().map_err(|e| e.to_string())?;
    let previous: KiCadSequentialRouterResult =
        serde_json::from_value(read_json(&checkpoint.join("sequential-route.json"))?)
            .map_err(|e| format!("invalid sequential checkpoint: {e}"))?;
    if previous.schema_version == 9
        || previous.sweep.is_some()
        || config.sequential.continue_after_routing_failure
    {
        return Err(
            "external resume does not support sparse sweep checkpoints; start a fresh sweep".into(),
        );
    }
    let source = previous
        .source_directory
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let output = output
        .parent()
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(
            output
                .file_name()
                .ok_or("resume output needs a directory name")?,
        );
    if output.starts_with(&checkpoint) || output.starts_with(&source) {
        return Err("resume output must be outside checkpoint and original source".into());
    }
    let mut sequential = config.sequential.clone();
    if sequential.connection_order.is_empty() {
        sequential.connection_order = previous.connection_order.clone();
    }
    if let Some(repair) = &mut sequential.ripup {
        repair.allow_existing_annotation_findings = config.allow_existing_annotation_findings;
    }
    sequential_router::route_with_checkpoint(
        &source,
        &previous.board_id,
        &output,
        &sequential,
        Some((
            &checkpoint,
            &previous,
            config.allow_existing_annotation_findings,
        )),
    )
}

fn prefix(
    previous: &KiCadSequentialRouterResult,
    order: &[String],
    maximum_connections: usize,
) -> Result<Vec<KiCadSequentialRouteStep>, String> {
    let count = previous.completed_connections;
    if !(3..=8).contains(&previous.schema_version)
        || count > previous.steps.len()
        || count > order.len()
        || maximum_connections < count
    {
        return Err("unsupported or inconsistent checkpoint prefix/bound".into());
    }
    validate_steps(&previous.steps, count, &previous.connection_order, order)?;
    Ok(previous.steps[..count].to_vec())
}

fn validate_steps(
    steps: &[KiCadSequentialRouteStep],
    count: usize,
    previous_order: &[String],
    order: &[String],
) -> Result<(), String> {
    if count > steps.len() || count > order.len() || steps.len() > count + 1 {
        return Err("checkpoint step count is inconsistent".into());
    }
    for (ordinal, step) in steps.iter().enumerate() {
        if step.ordinal != ordinal
            || ordinal >= previous_order.len()
            || step.connection != previous_order[ordinal]
            || (ordinal < count && step.connection != order[ordinal])
            || step.committed() != (ordinal < count)
        {
            return Err("checkpoint steps are not one contiguous committed prefix".into());
        }
        if ordinal < count {
            let ordinary = step.attempts.iter().filter(|a| a.selected).count();
            let repaired = usize::from(step.repair.as_ref().is_some_and(|r| r.selected));
            if ordinary + repaired != 1
                || step.board_sha256_after.is_none()
                || !step.selected_admission().is_some_and(|a| a.complete)
                || (ordinary == 1
                    && step
                        .attempts
                        .iter()
                        .find(|a| a.selected)
                        .map(|a| a.config_index)
                        != step.selected_config_index)
                || (repaired == 1 && step.selected_config_index.is_some())
            {
                return Err(
                    "checkpoint commit has contradictory or missing admission evidence".into(),
                );
            }
        } else if step.attempts.iter().any(|a| a.selected) || step.board_sha256_after.is_some() {
            return Err("uncommitted checkpoint tail claims selected copper".into());
        }
    }
    Ok(())
}

fn copper_is_committed(pcb: &Expr, completed: &BTreeSet<&str>) -> bool {
    let names = partial_ripup::net_names(pcb);
    pcb.children()
        .iter()
        .filter(|n| matches!(n.head(), Some("segment" | "arc" | "via")))
        .all(|n| {
            node_net(n).is_some_and(|name| {
                completed.contains(normalize_net(names.get(name).map_or(name, String::as_str)))
            })
        })
}

fn absolute(path: &mut PathBuf) -> Result<(), String> {
    *path = path
        .canonicalize()
        .map_err(|e| format!("missing checkpoint artifact {}: {e}", path.display()))?;
    Ok(())
}

fn canonicalize_step(step: &mut KiCadSequentialRouteStep) -> Result<(), String> {
    absolute(&mut step.artifact_directory)?;
    for attempt in &mut step.attempts {
        if let Some(path) = &mut attempt.candidate {
            absolute(path)?;
        }
        if let Some(admission) = &mut attempt.native_admission {
            absolute(&mut admission.directory)?;
        }
    }
    if let Some(repair) = &mut step.repair {
        for (net, receipt) in &mut repair.changed_routes {
            absolute(&mut receipt.candidate)?;
            restoration_recovery::check_receipt(net, receipt)?;
        }
        absolute(&mut repair.directory)?;
        if let Some(compound) = &mut repair.compound {
            absolute(&mut compound.directory)?;
        }
        if let Some(admission) = &mut repair.native_admission {
            absolute(&mut admission.directory)?;
        }
    }
    Ok(())
}

pub(super) fn prepare(
    checkpoint: &Path,
    previous: &KiCadSequentialRouterResult,
    source: &Path,
    output: &Path,
    config: &KiCadSequentialRouterConfig,
    order: &[String],
    baseline_drc: &serde_json::Value,
    allow_annotations: bool,
) -> Result<(Vec<KiCadSequentialRouteStep>, KiCadSequentialResumeEvidence), String> {
    let mut steps = prefix(previous, order, config.maximum_connections)?;
    let board_name = format!("{}.kicad_pcb", previous.board_id);
    let original = source.join(&board_name);
    let old_baseline = checkpoint.join("native-source-baseline");
    if !previous.source_board_unchanged
        || file_sha256(&original)? != previous.source_board_sha256_before
        || previous.source_board_sha256_before != previous.source_board_sha256_after
        || file_sha256(&old_baseline.join(&board_name))? != previous.source_board_sha256_before
        || kicad_erc_input_sha256(source)? != kicad_erc_input_sha256(&old_baseline)?
        || fs::read(source.join(format!("{}.kicad_pro", previous.board_id)))
            .map_err(|e| e.to_string())?
            != fs::read(old_baseline.join(format!("{}.kicad_pro", previous.board_id)))
                .map_err(|e| e.to_string())?
    {
        return Err("checkpoint original board, project or ERC inputs changed".into());
    }
    let discovered = inspect_kicad_routable_connections(&original)?;
    if serde_json::to_value(&discovered).map_err(|e| e.to_string())?
        != serde_json::to_value(&previous.discovered_connections).map_err(|e| e.to_string())?
    {
        return Err("checkpoint pad inventory changed".into());
    }
    let mut expected = discovered
        .iter()
        .map(|s| s.electrical_terminal_count() - 1)
        .sum::<usize>();
    if expected != previous.expected_source_unconnected_items {
        return Err("checkpoint original connectivity ledger is inconsistent".into());
    }
    for step in &mut steps {
        canonicalize_step(step)?;
        let admission = step
            .selected_admission()
            .ok_or("checkpoint commit lacks admission")?;
        expected = expected
            .checked_sub(
                discovered
                    .iter()
                    .find(|s| s.connection == step.connection)
                    .ok_or("checkpoint net missing from inventory")?
                    .electrical_terminal_count()
                    - 1,
            )
            .ok_or("checkpoint connectivity ledger underflowed")?;
        if admission.expected_unconnected_items != expected
            || admission.verification.selected_net_unconnected_items != expected
            || admission.erc_input_sha256 != kicad_erc_input_sha256(source)?
            || file_sha256(&admission.directory.join(&board_name))?
                != *step.board_sha256_after.as_ref().unwrap()
        {
            return Err(
                "checkpoint commit hash or native ledger disagrees with its artifacts".into(),
            );
        }
    }
    let retained = steps
        .last()
        .and_then(|s| s.selected_admission())
        .map(|a| a.directory.clone())
        .unwrap_or_else(|| old_baseline.clone());
    let snapshot = output.join("resume-source");
    copy_directory_tree(&retained, &snapshot)?;
    // Verify and render the current retained bytes, never trust a cached verdict.
    let verification = verify_materialized_rung(&snapshot, &previous.board_id)?;
    let retained_hash = file_sha256(&snapshot.join(&board_name))?;
    let expected_hash = steps
        .last()
        .and_then(|s| s.board_sha256_after.as_ref())
        .unwrap_or(&previous.source_board_sha256_before);
    let mutable_result_matches_commit = file_sha256(&previous.result_directory.join(&board_name))
        .is_ok_and(|hash| &hash == expected_hash);
    let pcb = parse(&fs::read_to_string(snapshot.join(&board_name)).map_err(|e| e.to_string())?)?;
    let original_pcb = parse(&fs::read_to_string(&original).map_err(|e| e.to_string())?)?;
    let completed: BTreeSet<_> = steps.iter().map(|s| s.connection.as_str()).collect();
    let copper_belongs_to_prefix = copper_is_committed(&pcb, &completed);
    if &retained_hash != expected_hash
        || coupled_vias::fixed_design(pcb) != coupled_vias::fixed_design(original_pcb)
        || !copper_belongs_to_prefix
        || kicad_erc_input_sha256(&snapshot)? != kicad_erc_input_sha256(source)?
        || fs::read(snapshot.join(format!("{}.kicad_pro", previous.board_id)))
            .map_err(|e| e.to_string())?
            != fs::read(source.join(format!("{}.kicad_pro", previous.board_id)))
                .map_err(|e| e.to_string())?
        || !partial_ripup::final_admissible(
            &verification,
            &read_json(&snapshot.join("drc.json"))?,
            baseline_drc,
            expected,
            allow_annotations,
        )?
    {
        return Err(
            "retained checkpoint failed fresh native, geometry, hash or connectivity admission"
                .into(),
        );
    }
    let evidence = KiCadSequentialResumeEvidence {
        checkpoint_directory: checkpoint.to_path_buf(),
        checkpoint_report_sha256: file_sha256(&checkpoint.join("sequential-route.json"))?,
        initial_completed_connections: steps.len(), initial_unconnected_items: expected,
        retained_board_sha256: retained_hash, mutable_result_matches_commit,
        snapshot_directory: snapshot, verification,
        contract: "reuse only a contiguous committed prefix after fresh native verification; preserve original board/project/ERC inputs and fixed geometry; no copper on uncommitted nets; uncommitted repair alternatives remain in the original run; new repair-invocation budget applies to this invocation; maximum_connections is the total prefix ceiling".into(),
    };
    Ok((steps, evidence))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(ordinal: usize, connection: &str, committed: bool) -> KiCadSequentialRouteStep {
        let native = VerificationReport {
            board_id: "test".into(),
            complete: false,
            erc_violations: 0,
            drc_design_violations: 0,
            schematic_parity_issues: 0,
            selected_net_unconnected_items: 1,
            intentional_no_connect_groups: 0,
            library_metadata_warnings: 0,
        };
        KiCadSequentialRouteStep {
            ordinal,
            connection: connection.into(),
            artifact_directory: PathBuf::new(),
            attempts: if committed {
                vec![KiCadSequentialRouteAttempt {
                    config_index: 0,
                    candidate: None,
                    quality: None,
                    score_mm: None,
                    expansions: 0,
                    error: None,
                    route_failure: None,
                    skip_reason: None,
                    selected: true,
                    native_admission: Some(KiCadSequentialNativeAdmission {
                        verification_scope: "test".into(),
                        erc_input_sha256: "test".into(),
                        directory: PathBuf::new(),
                        expected_unconnected_items: 1,
                        verification: native,
                        introduced_erc_violations: 0,
                        introduced_drc_design_violations: 0,
                        introduced_schematic_parity_issues: 0,
                        complete: true,
                    }),
                }]
            } else {
                vec![]
            },
            selected_config_index: committed.then_some(0),
            repair: None,
            repair_skip_reason: None,
            board_sha256_after: committed.then(|| "hash".into()),
            elapsed_micros: 0,
        }
    }

    #[test]
    fn resume_rejects_holes_reordered_commits_and_contradictory_receipts() {
        let order = ["A".into(), "B".into(), "C".into()];
        let mut steps = vec![step(0, "A", true), step(1, "B", false)];
        assert!(validate_steps(&steps, 1, &order, &order).is_ok());
        assert!(validate_steps(&steps, 2, &order, &order).is_err());
        assert!(validate_steps(&steps, 1, &order, &["B".into(), "A".into(), "C".into()]).is_err());
        steps[0].selected_config_index = Some(1);
        assert!(validate_steps(&steps, 1, &order, &order).is_err());
        steps[0].selected_config_index = Some(0);
        steps[0].board_sha256_after = None;
        assert!(validate_steps(&steps, 1, &order, &order).is_err());
        let steps = vec![step(0, "A", false), step(1, "B", true)];
        assert!(validate_steps(&steps, 1, &order, &order).is_err());
    }

    #[test]
    fn resume_rejects_copper_outside_the_committed_prefix_in_both_net_formats() {
        let numeric =
            parse(r#"(kicad_pcb (net 1 "/A") (net 2 "B") (segment (net 1)) (via (net 2)))"#)
                .unwrap();
        let named = parse(r#"(kicad_pcb (segment (net "/A")) (via (net "B")))"#).unwrap();
        for pcb in [numeric, named] {
            assert!(!copper_is_committed(&pcb, &BTreeSet::from(["A"])));
            assert!(copper_is_committed(&pcb, &BTreeSet::from(["A", "B"])));
        }
        assert!(!copper_is_committed(
            &parse("(kicad_pcb (segment (net 0)))").unwrap(),
            &BTreeSet::from(["A"])
        ));
    }
}

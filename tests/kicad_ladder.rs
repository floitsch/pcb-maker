// Copyright (C) 2026 Toit contributors.

use std::{path::PathBuf, process::Command};

#[test]
fn gnd_baseline_exposes_the_known_via_cluster_debt() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = pcb_kicad::read_route_candidate(
        &repository.join("benchmarks/esp32-c3-ladder/candidates/gnd-rung42.json"),
    )
    .expect("read persisted GND candidate");
    let quality = pcb_kicad::route_candidate_quality(&candidate).expect("measure GND candidate");
    assert_eq!(quality.vias, 128);
    assert_eq!(quality.branch_via_transitions, 128);
    assert_eq!(quality.maximum_branch_vias, 6);
    assert_eq!(quality.close_via_pairs, 85);
    assert_eq!(quality.clustered_vias, 91);
}

#[test]
fn native_dual_esp32_cold_prefix_rebuilds_first_two_rungs_from_zero_copper() {
    if !Command::new("kicad-cli")
        .arg("--version")
        .status()
        .is_ok_and(|status| status.success())
    {
        eprintln!("skipping KiCad integration test: kicad-cli is unavailable");
        return;
    }

    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = std::env::temp_dir().join(format!(
        "pcb-maker-dual-esp32-cold-prefix-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let status = Command::new(env!("CARGO_BIN_EXE_pcb-maker"))
        .arg("solve-semantic-kicad-prefix")
        .arg(repository.join("benchmarks/imported/layout-trace/dual-esp32-benchmark.json"))
        .arg("declared")
        .arg(repository.join("benchmarks/dual-esp32-ladder/template-config.json"))
        .arg("2")
        .arg(&output)
        .arg(repository.join("experiments/configs/dual-esp32-rung02-shared-tree.json"))
        .status()
        .expect("run cold dual ESP32 prefix");
    assert!(status.success());

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output.join("cold-prefix.json"))
            .expect("read cold-prefix manifest"),
    )
    .expect("parse cold-prefix manifest");
    assert_eq!(manifest["target_rung"].as_u64(), Some(2));
    assert_eq!(manifest["completed"].as_bool(), Some(true));
    assert_eq!(
        manifest["template"]["electrical_connections"].as_u64(),
        Some(2)
    );
    assert!(
        manifest["contract"]
            .as_str()
            .is_some_and(|contract| contract.contains("never consume a previously solved"))
    );
    assert_eq!(manifest["initial"]["rung"].as_u64(), Some(0));
    assert_eq!(
        manifest["initial"]["selected_connections"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(manifest["initial"]["pcb_segments"].as_u64(), Some(0));
    assert_eq!(manifest["initial"]["pcb_vias"].as_u64(), Some(0));
    assert_eq!(
        manifest["placement"]["evidence"]["routability_claimed"].as_bool(),
        Some(false)
    );
    assert_eq!(manifest["progression"]["initial_rung"].as_u64(), Some(0));
    assert_eq!(manifest["progression"]["final_rung"].as_u64(), Some(2));
    assert_eq!(
        manifest["progression"]["committed_connections"].as_u64(),
        Some(2)
    );

    let steps = manifest["progression"]["steps"]
        .as_array()
        .expect("cold progression steps");
    assert_eq!(steps.len(), 2);
    assert_eq!(
        steps[0]["result"]["evidence"]["inserted_connection"].as_str(),
        Some("SIG01_W_R")
    );
    assert_eq!(
        steps[1]["result"]["evidence"]["inserted_connection"].as_str(),
        Some("SIG01_AFTER_LINK")
    );
    assert!(
        steps[0]["result"]["evidence"]["source_verification"]["complete"]
            .as_bool()
            .is_some_and(|complete| complete)
    );
    let second_attempt = &steps[1]["result"]["evidence"]["attempts"][0];
    assert_eq!(second_attempt["selected"].as_bool(), Some(true));
    assert_eq!(
        second_attempt["verification"]["complete"].as_bool(),
        Some(true)
    );
    assert_eq!(second_attempt["route_quality"]["vias"].as_u64(), Some(0));
    assert!(
        second_attempt["route_quality"]["length_mm"]
            .as_f64()
            .is_some_and(|length| length < 58.0)
    );
    // On this cold placement, the pre-reference-fix executable takes 10,429
    // expansions and the repaired program takes 10,556. Keep a bounded work
    // regression; the historical 1,500 limit no longer fits this fixture.
    assert!(
        second_attempt["route_expansions"]
            .as_u64()
            .is_some_and(|work| work < 20_000)
    );

    for snapshot in [
        "source-problem.json",
        "active-prefix-problem.json",
        "placement-config.json",
        "source-template-config.json",
        "template-config.json",
        "insertion-config.json",
    ] {
        assert!(output.join(snapshot).is_file(), "missing {snapshot}");
    }
    let active_problem: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output.join("active-prefix-problem.json"))
            .expect("read active prefix problem"),
    )
    .expect("parse active prefix problem");
    assert_eq!(
        active_problem["components"].as_array().map(Vec::len),
        Some(43)
    );
    assert_eq!(active_problem["nets"].as_array().map(Vec::len), Some(3));
    let final_directory = PathBuf::from(
        manifest["final_directory"]
            .as_str()
            .expect("cold-prefix final directory"),
    );
    let candidate = pcb_kicad::read_route_candidate(&final_directory.join("inserted-route.json"))
        .expect("read second-rung shared-tree candidate");
    assert_eq!(candidate.branches.len(), 2);
    assert_eq!(candidate.tree_attachment_attempts.len(), 1);
    assert!(candidate.tree_attachment_attempts[0].selected);

    if std::env::var_os("PCB_MAKER_KEEP_TEST_ARTIFACTS").is_some() {
        eprintln!("retained cold-prefix artifacts at {}", output.display());
    } else {
        std::fs::remove_dir_all(output).expect("remove isolated dual ESP32 ladder output");
    }
}

#[test]
fn native_one_connection_transaction_commits_or_exactly_rolls_back() {
    if !Command::new("kicad-cli")
        .arg("--version")
        .status()
        .is_ok_and(|status| status.success())
    {
        eprintln!("skipping KiCad integration test: kicad-cli is unavailable");
        return;
    }

    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let declaration = pcb_kicad::load_declaration(
        &repository.join("benchmarks/esp32-c3-ladder/declaration.json"),
    )
    .expect("load ESP32 ladder declaration");
    let parent = repository.join("artifacts/esp32-c3-ladder/00-empty");
    let output = std::env::temp_dir().join(format!(
        "pcb-maker-kicad-connection-insertion-{}",
        std::process::id()
    ));

    let committed = pcb_kicad::insert_kicad_connection(
        &declaration,
        &parent,
        &output.join("commit"),
        &pcb_kicad::KiCadConnectionInsertionConfig::default(),
    )
    .expect("insert first connection");
    assert_eq!(
        committed.disposition,
        pcb_kicad::KiCadConnectionInsertionDisposition::Committed
    );
    assert!(committed.evidence.parent_unchanged);
    assert_eq!(committed.evidence.source_rung, 0);
    assert_eq!(committed.evidence.target_rung, 1);
    assert_eq!(committed.evidence.inserted_connection, "T_LOCAL_COMM0");
    assert_eq!(committed.evidence.attempts.len(), 1);
    let attempt = &committed.evidence.attempts[0];
    assert_eq!(
        attempt.status,
        pcb_kicad::KiCadConnectionInsertionAttemptStatus::Committed
    );
    assert!(attempt.selected);
    assert!(
        attempt
            .verification
            .as_ref()
            .is_some_and(|gate| gate.complete)
    );
    assert_eq!(
        attempt.route_quality.as_ref().map(|quality| quality.vias),
        Some(0)
    );
    assert!(attempt.route_expansions.is_some_and(|work| work < 1_000));

    let lazy_config = pcb_kicad::KiCadConnectionInsertionConfig {
        local_candidate_cap: 3,
        local_routing_portfolio: vec![
            pcb_kicad::KiCadGridRouteConfig::default(),
            pcb_kicad::KiCadGridRouteConfig {
                resolution_mm: 0.125,
                ..pcb_kicad::KiCadGridRouteConfig::default()
            },
            pcb_kicad::KiCadGridRouteConfig {
                resolution_mm: 0.1,
                ..pcb_kicad::KiCadGridRouteConfig::default()
            },
        ],
        local_native_admission: pcb_kicad::KiCadLocalNativeAdmission::ScoreOrderedUntilComplete,
        unrouted_target_gate: pcb_kicad::KiCadUnroutedTargetGate::BeforeFallbackOrRollback,
        ..pcb_kicad::KiCadConnectionInsertionConfig::default()
    };
    let lazy = pcb_kicad::insert_kicad_connection(
        &declaration,
        &parent,
        &output.join("lazy-score-ordered"),
        &lazy_config,
    )
    .expect("insert first connection with score-ordered native admission");
    assert_eq!(
        lazy.disposition,
        pcb_kicad::KiCadConnectionInsertionDisposition::Committed
    );
    assert_eq!(lazy.evidence.local_candidates_generated, 3);
    assert_eq!(lazy.evidence.local_native_gates, 1);
    assert_eq!(lazy.evidence.local_candidates_not_native_gated, 2);
    assert!(lazy.evidence.unrouted_target_verification.is_none());
    assert!(
        !output
            .join("lazy-score-ordered/unrouted-target-verification")
            .exists()
    );
    assert_eq!(lazy.evidence.attempts.len(), 3);
    assert_eq!(
        lazy.evidence.attempts[0].status,
        pcb_kicad::KiCadConnectionInsertionAttemptStatus::CandidateGenerated
    );
    assert_eq!(
        lazy.evidence.attempts[1].status,
        pcb_kicad::KiCadConnectionInsertionAttemptStatus::CandidateGenerated
    );
    assert_eq!(
        lazy.evidence.attempts[2].status,
        pcb_kicad::KiCadConnectionInsertionAttemptStatus::Committed
    );
    assert_eq!(lazy.evidence.attempts[2].native_admission_order, Some(0));
    assert!(lazy.evidence.attempts[2].native_gate_invoked);
    assert!(lazy.evidence.attempts[2].selected);
    assert!(
        lazy.evidence.attempts[2]
            .route_quality
            .as_ref()
            .is_some_and(|quality| quality.length_mm < 19.1)
    );

    let adaptive_resume_config = pcb_kicad::KiCadConnectionInsertionConfig {
        local_candidate_cap: 3,
        local_routing_portfolio: vec![
            pcb_kicad::KiCadGridRouteConfig {
                trace_width_mm: 0.05,
                ..pcb_kicad::KiCadGridRouteConfig::default()
            },
            pcb_kicad::KiCadGridRouteConfig {
                resolution_mm: 0.125,
                trace_width_mm: 0.05,
                ..pcb_kicad::KiCadGridRouteConfig::default()
            },
            pcb_kicad::KiCadGridRouteConfig {
                resolution_mm: 0.1,
                ..pcb_kicad::KiCadGridRouteConfig::default()
            },
        ],
        local_native_admission: pcb_kicad::KiCadLocalNativeAdmission::ScoreOrderedUntilComplete,
        ordered_local_refinement: Some(pcb_kicad::KiCadOrderedLocalRefinementConfig {
            minimum_portfolio_entries: 2,
            continue_if_score_improvement_percent_at_least: 100.0,
        }),
        fallback: pcb_kicad::KiCadConnectionInsertionFallback::None,
        unrouted_target_gate: pcb_kicad::KiCadUnroutedTargetGate::BeforeFallbackOrRollback,
        ..pcb_kicad::KiCadConnectionInsertionConfig::default()
    };
    let adaptive_resume = pcb_kicad::insert_kicad_connection(
        &declaration,
        &parent,
        &output.join("adaptive-resume"),
        &adaptive_resume_config,
    )
    .expect("restore the skipped suffix after native prefix rejection");
    assert_eq!(
        adaptive_resume.disposition,
        pcb_kicad::KiCadConnectionInsertionDisposition::Committed
    );
    assert_eq!(
        adaptive_resume.evidence.local_portfolio_entries_available,
        3
    );
    assert_eq!(
        adaptive_resume.evidence.local_portfolio_entries_evaluated,
        3
    );
    assert_eq!(adaptive_resume.evidence.local_native_gates, 3);
    assert!(
        adaptive_resume
            .evidence
            .ordered_local_refinement
            .as_ref()
            .is_some_and(|evidence| {
                evidence.evaluated_before_stop == 2 && evidence.resumed_after_native_rejection
            })
    );
    assert!(
        adaptive_resume.evidence.attempts[..2]
            .iter()
            .all(|attempt| {
                attempt.status == pcb_kicad::KiCadConnectionInsertionAttemptStatus::NativeRejected
            })
    );
    assert_eq!(
        adaptive_resume.evidence.attempts[2].status,
        pcb_kicad::KiCadConnectionInsertionAttemptStatus::Committed
    );
    assert!(adaptive_resume.evidence.attempts[2].selected);

    let rollback_config = pcb_kicad::KiCadConnectionInsertionConfig {
        local_routing_portfolio: vec![pcb_kicad::KiCadGridRouteConfig {
            max_expansions: 1,
            ..pcb_kicad::KiCadGridRouteConfig::default()
        }],
        fallback: pcb_kicad::KiCadConnectionInsertionFallback::None,
        ..pcb_kicad::KiCadConnectionInsertionConfig::default()
    };
    let rollback = pcb_kicad::insert_kicad_connection(
        &declaration,
        &parent,
        &output.join("rollback"),
        &rollback_config,
    )
    .expect("run bounded rollback control");
    assert_eq!(
        rollback.disposition,
        pcb_kicad::KiCadConnectionInsertionDisposition::RolledBack
    );
    assert!(rollback.evidence.parent_unchanged);
    assert!(rollback.evidence.rollback_exact);
    assert!(rollback.evidence.unrouted_target_verification.is_some());
    assert!(
        output
            .join("rollback/unrouted-target-verification")
            .is_dir()
    );
    assert_eq!(
        rollback.evidence.source_snapshot_sha256,
        rollback.evidence.selected_artifact_sha256
    );
    assert_eq!(
        rollback.evidence.attempts[0].status,
        pcb_kicad::KiCadConnectionInsertionAttemptStatus::RouteFailed
    );

    let control_config = pcb_kicad::KiCadConnectionInsertionConfig {
        fallback: pcb_kicad::KiCadConnectionInsertionFallback::DeclaredTargetControl,
        ..rollback_config
    };
    let control = pcb_kicad::insert_kicad_connection(
        &declaration,
        &parent,
        &output.join("control"),
        &control_config,
    )
    .expect("run historical-target feasibility control");
    assert_eq!(
        control.disposition,
        pcb_kicad::KiCadConnectionInsertionDisposition::RolledBack
    );
    assert!(control.evidence.rollback_exact);
    assert_eq!(control.evidence.attempts.len(), 2);
    assert_eq!(
        control.evidence.attempts[1].status,
        pcb_kicad::KiCadConnectionInsertionAttemptStatus::ControlComplete
    );
    assert!(!control.evidence.attempts[1].selected);

    let progression = pcb_kicad::progress_kicad_connections(
        &declaration,
        &parent,
        &output.join("progression"),
        &pcb_kicad::KiCadConnectionProgressionConfig {
            maximum_connections: 13,
            reuse_committed_parent_verification: true,
            insertion: pcb_kicad::KiCadConnectionInsertionConfig {
                single_connection_ripup: Some(
                    pcb_kicad::KiCadSingleConnectionRipupConfig::default(),
                ),
                ..pcb_kicad::KiCadConnectionInsertionConfig::default()
            },
        },
    )
    .expect("progress through the first fixed-copper failure and repair");
    assert_eq!(
        progression.termination,
        pcb_kicad::KiCadConnectionProgressionTermination::BoundReached
    );
    assert_eq!(progression.initial_rung, 0);
    assert_eq!(progression.final_rung, 13);
    assert_eq!(progression.committed_connections, 13);
    assert_eq!(progression.steps.len(), 13);
    assert!(
        !progression.steps[0]
            .result
            .as_ref()
            .expect("first progression result")
            .evidence
            .source_verification_reused
    );
    assert!(progression.steps[1..].iter().all(|step| {
        step.result
            .as_ref()
            .is_some_and(|result| result.evidence.source_verification_reused)
            && !step
                .transaction_directory
                .join("source-verification")
                .exists()
    }));
    assert!(progression.steps.iter().all(|step| {
        step.quality_comparison
            .as_ref()
            .and_then(|comparison| comparison.historical_declared.as_ref())
            .is_some()
    }));
    let repaired = progression.steps.last().expect("rung-13 repair step");
    let repaired_result = repaired.result.as_ref().expect("rung-13 result");
    assert_eq!(
        repaired_result.evidence.inserted_connection,
        "T_LOCAL_FLEX3"
    );
    assert_eq!(repaired_result.evidence.attempts.len(), 2);
    assert_eq!(
        repaired_result.evidence.attempts[0].status,
        pcb_kicad::KiCadConnectionInsertionAttemptStatus::RouteFailed
    );
    assert_eq!(
        repaired_result.evidence.attempts[1].scope,
        pcb_kicad::KiCadConnectionInsertionAttemptScope::SingleConnectionRipup
    );
    assert_eq!(
        repaired_result.evidence.attempts[1].status,
        pcb_kicad::KiCadConnectionInsertionAttemptStatus::Committed
    );
    assert!(repaired_result.evidence.attempts[1].selected);
    assert!(
        repaired_result.evidence.attempts[1]
            .single_connection_ripup
            .as_ref()
            .is_some_and(|repair| repair.selected_attempt == Some(0))
    );

    std::fs::remove_dir_all(output).expect("remove isolated connection-insertion output");
}

#[test]
fn native_shared_tree_insertion_reuses_copper_without_a_via_cluster() {
    if !Command::new("kicad-cli")
        .arg("--version")
        .status()
        .is_ok_and(|status| status.success())
    {
        eprintln!("skipping KiCad integration test: kicad-cli is unavailable");
        return;
    }

    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let declaration = pcb_kicad::load_declaration(
        &repository.join("benchmarks/esp32-c3-ladder/declaration.json"),
    )
    .expect("load ESP32 ladder declaration");
    let parent = repository.join("artifacts/esp32-c3-ladder/22-t-probe-xdata-1");
    let output = std::env::temp_dir().join(format!(
        "pcb-maker-kicad-shared-tree-insertion-{}",
        std::process::id()
    ));
    let routing = pcb_kicad::KiCadGridRouteConfig {
        resolution_mm: 0.25,
        multi_terminal_routing: pcb_kicad::KiCadMultiTerminalRoutingPolicy::SharedCopperTree,
        maximum_tree_attachment_searches: 1,
        tree_attachment_objective: pcb_kicad::KiCadTreeAttachmentObjective::ViasThenLength,
        ..pcb_kicad::KiCadGridRouteConfig::default()
    };
    let result = pcb_kicad::insert_kicad_connection(
        &declaration,
        &parent,
        &output,
        &pcb_kicad::KiCadConnectionInsertionConfig {
            local_candidate_cap: 1,
            local_routing_portfolio: vec![routing],
            ..pcb_kicad::KiCadConnectionInsertionConfig::default()
        },
    )
    .expect("insert XDATA as a shared copper tree");

    assert_eq!(
        result.disposition,
        pcb_kicad::KiCadConnectionInsertionDisposition::Committed
    );
    assert!(result.evidence.parent_unchanged);
    assert_eq!(result.evidence.source_rung, 22);
    assert_eq!(result.evidence.target_rung, 23);
    assert_eq!(result.evidence.inserted_connection, "XDATA");
    let attempt = &result.evidence.attempts[0];
    assert!(attempt.selected);
    assert!(
        attempt
            .verification
            .as_ref()
            .is_some_and(|gate| gate.complete)
    );
    let quality = attempt.route_quality.as_ref().expect("route quality");
    assert_eq!(quality.vias, 0);
    assert_eq!(quality.close_via_pairs, 0);
    assert_eq!(quality.clustered_vias, 0);
    assert!(quality.length_mm < 24.0);

    let candidate = pcb_kicad::read_route_candidate(
        result
            .selected_candidate
            .as_ref()
            .expect("selected route candidate"),
    )
    .expect("read selected shared-tree candidate");
    assert_eq!(candidate.branches.len(), 3);
    assert_eq!(candidate.tree_attachment_attempts.len(), 2);
    assert!(candidate.tree_attachment_attempts.iter().all(|attachment| {
        attachment.selected
            && attachment.status == pcb_kicad::KiCadTreeAttachmentAttemptStatus::Found
            && attachment.searched_source == attachment.source
            && attachment.searched_source_layer == attachment.source_layer
    }));

    std::fs::remove_dir_all(output).expect("remove isolated shared-tree output");
}

#[test]
fn rung7_board_copper_round_trips_and_fixed_via_relaxation_exact_passes() {
    if !Command::new("kicad-cli")
        .arg("--version")
        .status()
        .is_ok_and(|status| status.success())
    {
        eprintln!("skipping KiCad integration test: kicad-cli is unavailable");
        return;
    }

    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let declaration = repository.join("benchmarks/esp32-c3-ladder/declaration.json");
    let output =
        std::env::temp_dir().join(format!("pcb-maker-kicad-fixed-via-{}", std::process::id()));
    let binary = env!("CARGO_BIN_EXE_pcb-maker");
    let materialized = Command::new(binary)
        .args(["materialize-kicad-rung"])
        .arg(&declaration)
        .arg("7")
        .arg(&output)
        .status()
        .expect("materialize rung 7");
    assert!(materialized.success());
    let rung = output.join("07-t-local-flex0");
    let board = rung.join("esp32-c3.kicad_pcb");
    let imported = output.join("t-local-flex0-imported.json");
    let import = Command::new(binary)
        .args(["import-kicad-route-candidate"])
        .arg(&board)
        .arg("T_LOCAL_FLEX0")
        .arg(&imported)
        .status()
        .expect("import rung-7 copper");
    assert!(import.success());
    let imported_candidate =
        pcb_kicad::read_route_candidate(&imported).expect("read imported route candidate");
    assert_eq!(imported_candidate.supplemental_segments.len(), 4);
    assert_eq!(imported_candidate.supplemental_vias.len(), 2);

    let reject_config = output.join("reject-vias.json");
    std::fs::write(&reject_config, r#"{"branch_indices":[0]}"#)
        .expect("write reject control config");
    let reject_evidence = output.join("reject-evidence.json");
    let reject = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&board)
        .arg(&imported)
        .arg(output.join("reject-committed.json"))
        .arg(output.join("reject-proposal.json"))
        .arg(&reject_evidence)
        .arg(&reject_config)
        .status()
        .expect("run reject-via control");
    assert!(reject.success());
    let reject_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&reject_evidence).expect("read reject-via evidence"),
    )
    .expect("parse reject-via evidence");
    assert_eq!(reject_evidence["status"].as_str(), Some("unsupported"));
    assert_eq!(reject_evidence["via_mobility"].as_str(), Some("reject"));

    let fixed_config = output.join("fixed-vias.json");
    std::fs::write(
        &fixed_config,
        r#"{"branch_indices":[0],"via_mobility":"fixed"}"#,
    )
    .expect("write fixed-via config");
    let committed = output.join("fixed-via-committed.json");
    let proposal = output.join("fixed-via-proposal.json");
    let evidence_path = output.join("fixed-via-evidence.json");
    let relaxed = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&board)
        .arg(&imported)
        .arg(&committed)
        .arg(&proposal)
        .arg(&evidence_path)
        .arg(&fixed_config)
        .status()
        .expect("run fixed-via relaxation");
    assert!(relaxed.success());
    let evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&evidence_path).expect("read fixed-via evidence"),
    )
    .expect("parse fixed-via evidence");
    assert_eq!(evidence["status"].as_str(), Some("accepted"));
    assert_eq!(evidence["via_mobility"].as_str(), Some("fixed"));
    assert_eq!(evidence["work"]["selected_layer_runs"].as_u64(), Some(3));
    assert_eq!(evidence["work"]["fixed_via_anchors"].as_u64(), Some(2));
    assert_eq!(evidence["before"]["vias"].as_u64(), Some(2));
    assert_eq!(evidence["after"]["vias"].as_u64(), Some(2));
    assert!(
        evidence["after"]["length_mm"]
            .as_f64()
            .zip(evidence["before"]["length_mm"].as_f64())
            .is_some_and(|(after, before)| after < before)
    );

    let local_config = output.join("fixed-vias-local.json");
    std::fs::write(
        &local_config,
        r#"{"branch_indices":[0],"via_mobility":"fixed","broad_phase":"uniform_grid"}"#,
    )
    .expect("write local fixed-via config");
    let local_committed = output.join("fixed-via-local-committed.json");
    let local_evidence_path = output.join("fixed-via-local-evidence.json");
    let local = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&board)
        .arg(&imported)
        .arg(&local_committed)
        .arg(output.join("fixed-via-local-proposal.json"))
        .arg(&local_evidence_path)
        .arg(&local_config)
        .status()
        .expect("run local fixed-via relaxation");
    assert!(local.success());
    assert_eq!(
        std::fs::read(&local_committed).expect("read local fixed-via candidate"),
        std::fs::read(&committed).expect("read exhaustive fixed-via candidate")
    );
    let local_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&local_evidence_path).expect("read local fixed-via evidence"),
    )
    .expect("parse local fixed-via evidence");
    assert_eq!(local_evidence["status"].as_str(), Some("accepted"));
    assert_eq!(local_evidence["after"], evidence["after"]);
    assert!(
        local_evidence["work"]["constraint_projections"]
            .as_u64()
            .zip(evidence["work"]["constraint_projections"].as_u64())
            .is_some_and(|(local, exhaustive)| local * 50 < exhaustive)
    );

    let applied = Command::new(binary)
        .args(["apply-kicad-route-candidate"])
        .arg(&board)
        .arg(&committed)
        .arg(&board)
        .status()
        .expect("apply fixed-via result");
    assert!(applied.success());
    let verified = Command::new(binary)
        .args(["verify-kicad-rung"])
        .arg(&rung)
        .arg("esp32-c3")
        .status()
        .expect("verify fixed-via rung");
    assert!(verified.success());

    std::fs::remove_dir_all(output).expect("remove isolated fixed-via test output");
}

#[test]
fn rung41_tree_import_and_shared_via_motion_exact_passes() {
    if !Command::new("kicad-cli")
        .arg("--version")
        .status()
        .is_ok_and(|status| status.success())
    {
        eprintln!("skipping KiCad integration test: kicad-cli is unavailable");
        return;
    }

    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let declaration = repository.join("benchmarks/esp32-c3-ladder/declaration.json");
    let output = std::env::temp_dir().join(format!(
        "pcb-maker-kicad-tree-import-{}",
        std::process::id()
    ));
    let binary = env!("CARGO_BIN_EXE_pcb-maker");
    let materialized = Command::new(binary)
        .args(["materialize-kicad-rung"])
        .arg(&declaration)
        .arg("41")
        .arg(&output)
        .status()
        .expect("materialize rung 41");
    assert!(materialized.success());
    let rung = output.join("41-3v3-dut");
    let board = rung.join("esp32-c3.kicad_pcb");
    let imported = output.join("3v3-dut-imported.json");
    let import = Command::new(binary)
        .args(["import-kicad-route-candidate"])
        .arg(&board)
        .arg("3V3_DUT")
        .arg(&imported)
        .status()
        .expect("import rung-41 tree copper");
    assert!(import.success());
    let imported_candidate =
        pcb_kicad::read_route_candidate(&imported).expect("read imported tree candidate");
    assert_eq!(
        imported_candidate.router,
        "kicad-board-copper-tree-import-v2"
    );
    assert_eq!(imported_candidate.terminals.len(), 13);
    assert_eq!(imported_candidate.branches.len(), 12);
    assert_eq!(imported_candidate.supplemental_segments.len(), 44);
    assert_eq!(imported_candidate.supplemental_vias.len(), 12);

    let committed = output.join("shared-via-committed.json");
    let proposal = output.join("shared-via-proposal.json");
    let evidence_path = output.join("shared-via-evidence.json");
    let relaxed = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&board)
        .arg(&imported)
        .arg(&committed)
        .arg(&proposal)
        .arg(&evidence_path)
        .arg(repository.join("experiments/configs/kicad-fixed-via-shared-tree-zero-motion.json"))
        .status()
        .expect("lower shared-via tree through the continuous engine");
    assert!(relaxed.success());
    let imported_bytes = std::fs::read(&imported).expect("read imported tree bytes");
    assert_eq!(
        std::fs::read(&committed).expect("read committed tree bytes"),
        imported_bytes
    );
    assert_eq!(
        std::fs::read(&proposal).expect("read proposed tree bytes"),
        imported_bytes
    );
    let evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&evidence_path).expect("read shared-via evidence"),
    )
    .expect("parse shared-via evidence");
    assert_eq!(evidence["status"].as_str(), Some("no_improvement"));
    assert_eq!(evidence["maximum_point_motion_mm"].as_f64(), Some(0.0));
    assert_eq!(evidence["work"]["fixed_via_anchors"].as_u64(), Some(2));
    assert_eq!(
        evidence["work"]["fixed_via_anchor_members"].as_u64(),
        Some(8)
    );
    assert_eq!(evidence["work"]["fixed_context_anchors"].as_u64(), Some(11));

    let applied = Command::new(binary)
        .args(["apply-kicad-route-candidate"])
        .arg(&board)
        .arg(&proposal)
        .arg(&board)
        .status()
        .expect("apply shared-via no-op proposal");
    assert!(applied.success());
    let verified = Command::new(binary)
        .args(["verify-kicad-rung"])
        .arg(&rung)
        .arg("esp32-c3")
        .status()
        .expect("verify shared-via tree rung");
    assert!(verified.success());

    let materialized = Command::new(binary)
        .args(["materialize-kicad-rung"])
        .arg(&declaration)
        .arg("42")
        .arg(&output)
        .status()
        .expect("materialize future-aware rung 42");
    assert!(materialized.success());
    let future_rung = output.join("42-gnd");
    let future_board = future_rung.join("esp32-c3.kicad_pcb");

    let (via_selected, via_evidence) = pcb_kicad::search_via_topology_actions(
        &future_rung,
        "esp32-c3",
        &imported,
        &output.join("via-action-portfolio"),
        &pcb_kicad::KiCadViaActionSearchConfig {
            target_at: [63.125, 30.875],
            target_layers: ["F.Cu".into(), "B.Cu".into()],
            removal_layers: Vec::new(),
            relocation_radius_mm: 0.5,
            relocation_step_mm: 0.5,
            maximum_relocation_candidates: 2,
            feasible_frontier: None,
            local_reroute: None,
            maximum_actions: 2,
            maximum_affected_branches: 1,
            via_penalty_mm: 3.0,
            minimum_score_improvement_mm: 1.0e-6,
        },
    )
    .expect("native-gate a bounded via-relocation portfolio");
    assert!(via_selected.is_some());
    assert!(via_evidence.complete);
    assert_eq!(via_evidence.affected_branches, vec![8]);
    assert_eq!(via_evidence.generated_actions, 2);
    // The old routed GND tree blocked the first relocation. With declared
    // GND zones, both relocations are legal because KiCad refills the yielding
    // plane around the moved 3V3_DUT copper; retained GND vias stay obstacles.
    assert_eq!(via_evidence.exact_complete_actions, 2);
    assert_eq!(via_evidence.selected_attempt, Some(1));
    assert!(!via_evidence.selected_source);
    assert!(
        via_evidence.attempts[1]
            .route_quality
            .as_ref()
            .is_some_and(|quality| quality.length_mm < 236.61)
    );

    let (frontier_selected, frontier_evidence) = pcb_kicad::search_via_topology_actions(
        &future_rung,
        "esp32-c3",
        &imported,
        &output.join("via-feasible-frontier-portfolio"),
        &pcb_kicad::KiCadViaActionSearchConfig {
            target_at: [63.125, 30.875],
            target_layers: ["F.Cu".into(), "B.Cu".into()],
            removal_layers: Vec::new(),
            relocation_radius_mm: 1.0,
            relocation_step_mm: 0.0,
            maximum_relocation_candidates: 2,
            feasible_frontier: Some(pcb_kicad::KiCadViaFeasibleFrontierConfig {
                angular_samples: 16,
                radial_samples: 4,
                boundary_refinement_steps: 6,
                boundary_inset_mm: 0.005,
                minimum_candidate_spacing_mm: 0.1,
            }),
            local_reroute: None,
            maximum_actions: 2,
            maximum_affected_branches: 1,
            via_penalty_mm: 3.0,
            minimum_score_improvement_mm: 1.0e-6,
        },
    )
    .expect("native-gate a geometry-aware via-relocation frontier");
    assert!(frontier_selected.is_some());
    assert!(frontier_evidence.complete);
    assert_eq!(frontier_evidence.affected_branches, vec![8]);
    assert_eq!(frontier_evidence.generated_actions, 2);
    assert_eq!(frontier_evidence.exact_complete_actions, 2);
    assert_eq!(
        frontier_evidence.relocation_generation.method,
        "analytic-feasible-radial-frontier-v1"
    );
    assert!(frontier_evidence.relocation_generation.analytic_probes > 2);
    assert!(
        frontier_evidence
            .attempts
            .iter()
            .all(|attempt| attempt.analytic_geometry_clear)
    );
    assert!(
        frontier_evidence.attempts[frontier_evidence.selected_attempt.unwrap()]
            .route_quality
            .as_ref()
            .is_some_and(|quality| quality.length_mm < 236.61)
    );

    let (rerouted_selected, rerouted_evidence) = pcb_kicad::search_via_topology_actions(
        &future_rung,
        "esp32-c3",
        &imported,
        &output.join("via-local-reroute-portfolio"),
        &pcb_kicad::KiCadViaActionSearchConfig {
            target_at: [65.125, 25.625],
            target_layers: ["F.Cu".into(), "B.Cu".into()],
            removal_layers: Vec::new(),
            relocation_radius_mm: 0.0,
            relocation_step_mm: 0.0,
            maximum_relocation_candidates: 0,
            feasible_frontier: None,
            local_reroute: Some(pcb_kicad::KiCadViaLocalRerouteSearchConfig {
                merged_layers: vec!["F.Cu".into()],
                resolutions_mm: vec![0.25],
                window_margin_mm: 4.0,
                maximum_grid_states: 500_000,
                maximum_expansions: 2_000_000,
                maximum_exact_edge_retries: 256,
                blocker_cut: None,
            }),
            maximum_actions: 1,
            maximum_affected_branches: 1,
            via_penalty_mm: 3.0,
            minimum_score_improvement_mm: 1.0e-6,
        },
    )
    .expect("native-gate a private via removal with bounded local rerouting");
    assert!(rerouted_selected.is_some());
    assert!(rerouted_evidence.complete);
    assert_eq!(rerouted_evidence.affected_branches, vec![9]);
    assert_eq!(rerouted_evidence.generated_actions, 1);
    assert_eq!(rerouted_evidence.exact_complete_actions, 1);
    assert_eq!(rerouted_evidence.selected_attempt, Some(0));
    assert!(!rerouted_evidence.selected_source);
    let rerouted_attempt = &rerouted_evidence.attempts[0];
    assert!(
        rerouted_attempt
            .route_quality
            .as_ref()
            .is_some_and(|quality| quality.vias == 10 && quality.length_mm < 233.0)
    );
    assert!(
        rerouted_attempt
            .local_reroute
            .as_ref()
            .is_some_and(|work| work.total_searches == 1 && work.total_expansions > 1_000)
    );

    let (no_path_selected, no_path_evidence) = pcb_kicad::search_via_topology_actions(
        &future_rung,
        "esp32-c3",
        &imported,
        &output.join("via-local-reroute-no-path-portfolio"),
        &pcb_kicad::KiCadViaActionSearchConfig {
            target_at: [63.125, 30.875],
            target_layers: ["F.Cu".into(), "B.Cu".into()],
            removal_layers: Vec::new(),
            relocation_radius_mm: 0.0,
            relocation_step_mm: 0.0,
            maximum_relocation_candidates: 0,
            feasible_frontier: None,
            local_reroute: Some(pcb_kicad::KiCadViaLocalRerouteSearchConfig {
                merged_layers: vec!["F.Cu".into()],
                resolutions_mm: vec![0.5],
                window_margin_mm: 4.0,
                maximum_grid_states: 500_000,
                maximum_expansions: 2_000_000,
                maximum_exact_edge_retries: 256,
                blocker_cut: None,
            }),
            maximum_actions: 1,
            maximum_affected_branches: 1,
            via_penalty_mm: 3.0,
            minimum_score_improvement_mm: 1.0e-6,
        },
    )
    .expect("retain structured frontier evidence for a real no-path removal");
    assert!(no_path_selected.is_some());
    assert!(no_path_evidence.selected_source);
    assert_eq!(no_path_evidence.affected_branches, vec![8]);
    assert_eq!(no_path_evidence.generated_actions, 1);
    assert_eq!(no_path_evidence.exact_complete_actions, 0);
    let no_path_attempt = &no_path_evidence.attempts[0];
    assert_eq!(
        no_path_attempt.status,
        pcb_kicad::KiCadViaActionStatus::Unsupported
    );
    let failure = no_path_attempt
        .local_reroute
        .as_ref()
        .and_then(|evidence| evidence.failed_branch.as_ref())
        .expect("real no-path action retains its grid frontier");
    assert_eq!(
        failure.kind,
        pcb_kicad::KiCadViaLocalRerouteFailureKind::NoPath
    );
    assert!(failure.blocked_frontier_states > 100);
    assert!(!failure.blocked_frontier_runs.is_empty());
    assert_eq!(
        failure.attributed_frontier_states,
        failure.blocked_frontier_states
    );
    assert_eq!(failure.unattributed_frontier_states, 0);
    assert_eq!(failure.blockers[0].object, "J4");
    assert!(!failure.blockers[0].component_movable);
    assert!(failure.blockers.iter().any(|blocker| {
        blocker.kind == pcb_kicad::KiCadViaLocalRerouteBlockerKind::FootprintPad
            && blocker.object == "C4"
            && blocker.component_movable
    }));
    assert!(failure.blockers.iter().any(|blocker| {
        blocker.kind == pcb_kicad::KiCadViaLocalRerouteBlockerKind::ForeignNetCopper
            && blocker.object == "D_EN"
    }));
    let artifact = no_path_attempt
        .artifact_directory
        .as_ref()
        .expect("real no-path action retains diagnostic artifacts");
    assert!(
        output
            .join("via-local-reroute-no-path-portfolio")
            .join(artifact)
            .join("diagnostic-front.svg")
            .is_file()
    );
    let diagnostic_front = std::fs::read_to_string(
        output
            .join("via-local-reroute-no-path-portfolio")
            .join(artifact)
            .join("diagnostic-front.svg"),
    )
    .unwrap();
    assert!(diagnostic_front.contains("data-semantic-blocker=\"J4\""));
    assert!(diagnostic_front.contains("data-semantic-blocker=\"D_EN\""));

    let joint_committed = output.join("shared-via-motion-committed.json");
    let joint_proposal = output.join("shared-via-motion-proposal.json");
    let joint_evidence_path = output.join("shared-via-motion-evidence.json");
    let joint = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&future_board)
        .arg(&imported)
        .arg(&joint_committed)
        .arg(&joint_proposal)
        .arg(&joint_evidence_path)
        .arg(repository.join("experiments/configs/kicad-fixed-via-shared-tree-tension.json"))
        .status()
        .expect("relax the shared-via subtree jointly");
    assert!(joint.success());
    let joint_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&joint_evidence_path).expect("read joint-motion evidence"),
    )
    .expect("parse joint-motion evidence");
    assert_eq!(joint_evidence["status"].as_str(), Some("accepted"));
    assert_eq!(joint_evidence["after"]["vias"].as_u64(), Some(12));
    assert_eq!(
        joint_evidence["work"]["fixed_via_anchor_members"].as_u64(),
        Some(8)
    );
    assert_eq!(
        joint_evidence["work"]["fixed_context_anchors"].as_u64(),
        Some(11)
    );
    assert!(
        joint_evidence["after"]["length_mm"]
            .as_f64()
            .zip(joint_evidence["before"]["length_mm"].as_f64())
            .is_some_and(|(after, before)| after < before - 3.9)
    );

    let automatic_committed = output.join("shared-via-motion-automatic-committed.json");
    let automatic_proposal = output.join("shared-via-motion-automatic-proposal.json");
    let automatic_evidence_path = output.join("shared-via-motion-automatic-evidence.json");
    let automatic = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&future_board)
        .arg(&imported)
        .arg(&automatic_committed)
        .arg(&automatic_proposal)
        .arg(&automatic_evidence_path)
        .arg(
            repository
                .join("experiments/configs/kicad-fixed-via-shared-tree-auto-neighborhood.json"),
        )
        .status()
        .expect("select and relax the shared-via subtree automatically");
    assert!(automatic.success());
    let automatic_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&automatic_evidence_path)
            .expect("read automatic-selection evidence"),
    )
    .expect("parse automatic-selection evidence");
    assert_eq!(automatic_evidence["status"].as_str(), Some("accepted"));
    assert_eq!(
        automatic_evidence["branch_selection"].as_str(),
        Some("nearest_shared_junction")
    );
    assert_eq!(automatic_evidence["seed_branches"], serde_json::json!([1]));
    assert_eq!(
        automatic_evidence["selected_branches"],
        serde_json::json!([1, 2])
    );
    assert_eq!(
        automatic_evidence["branch_contact_tests"].as_u64(),
        Some(830)
    );
    assert_eq!(
        std::fs::read(&automatic_committed).expect("read automatic candidate"),
        std::fs::read(&joint_committed).expect("read manual candidate")
    );
    assert_eq!(
        std::fs::read(&automatic_proposal).expect("read automatic proposal"),
        std::fs::read(&joint_proposal).expect("read manual proposal")
    );

    let bounded_committed = output.join("shared-via-motion-bounded-committed.json");
    let bounded_proposal = output.join("shared-via-motion-bounded-proposal.json");
    let bounded_evidence_path = output.join("shared-via-motion-bounded-evidence.json");
    let bounded = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&future_board)
        .arg(&imported)
        .arg(&bounded_committed)
        .arg(&bounded_proposal)
        .arg(&bounded_evidence_path)
        .arg(repository.join("experiments/configs/kicad-fixed-via-shared-tree-auto-bounded.json"))
        .status()
        .expect("run bounded automatic-selection control");
    assert!(bounded.success());
    let bounded_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&bounded_evidence_path).expect("read bounded-selection evidence"),
    )
    .expect("parse bounded-selection evidence");
    assert_eq!(bounded_evidence["status"].as_str(), Some("unsupported"));
    assert_eq!(bounded_evidence["work"]["frames"].as_u64(), Some(0));
    assert!(
        bounded_evidence["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("maximum_selected_branches=1"))
    );
    assert_eq!(
        std::fs::read(&bounded_committed).expect("read bounded rollback"),
        imported_bytes
    );
    assert!(!bounded_proposal.exists());

    let exhaustive_committed = output.join("shared-via-motion-exhaustive-committed.json");
    let exhaustive_proposal = output.join("shared-via-motion-exhaustive-proposal.json");
    let exhaustive_evidence_path = output.join("shared-via-motion-exhaustive-evidence.json");
    let exhaustive = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&future_board)
        .arg(&imported)
        .arg(&exhaustive_committed)
        .arg(&exhaustive_proposal)
        .arg(&exhaustive_evidence_path)
        .arg(
            repository
                .join("experiments/configs/kicad-fixed-via-shared-tree-tension-exhaustive.json"),
        )
        .status()
        .expect("run exhaustive shared-via motion control");
    assert!(exhaustive.success());
    assert_eq!(
        std::fs::read(&exhaustive_committed).expect("read exhaustive shared-via candidate"),
        std::fs::read(&joint_committed).expect("read local shared-via candidate")
    );
    assert_eq!(
        std::fs::read(&exhaustive_proposal).expect("read exhaustive shared-via proposal"),
        std::fs::read(&joint_proposal).expect("read local shared-via proposal")
    );
    let exhaustive_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&exhaustive_evidence_path)
            .expect("read exhaustive shared-via evidence"),
    )
    .expect("parse exhaustive shared-via evidence");
    assert!(
        joint_evidence["work"]["constraint_projections"]
            .as_u64()
            .zip(exhaustive_evidence["work"]["constraint_projections"].as_u64())
            .is_some_and(|(local, exhaustive)| local * 10 < exhaustive)
    );

    let branch1_evidence_path = output.join("shared-via-branch1-evidence.json");
    let branch1 = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&future_board)
        .arg(&imported)
        .arg(output.join("shared-via-branch1-committed.json"))
        .arg(output.join("shared-via-branch1-proposal.json"))
        .arg(&branch1_evidence_path)
        .arg(repository.join("experiments/configs/kicad-fixed-via-shared-tree-branch1.json"))
        .status()
        .expect("run single-branch shared-via control");
    assert!(branch1.success());
    let branch1_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&branch1_evidence_path).expect("read branch-1 evidence"),
    )
    .expect("parse branch-1 evidence");
    assert_eq!(branch1_evidence["status"].as_str(), Some("accepted"));
    assert!(
        branch1_evidence["after"]["length_mm"]
            .as_f64()
            .zip(joint_evidence["after"]["length_mm"].as_f64())
            .is_some_and(|(branch1, joint)| branch1 > joint + 3.0)
    );

    let independent_committed = output.join("shared-via-independent-committed.json");
    let independent_proposal = output.join("shared-via-independent-proposal.json");
    let independent_evidence_path = output.join("shared-via-independent-evidence.json");
    let independent = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&future_board)
        .arg(&imported)
        .arg(&independent_committed)
        .arg(&independent_proposal)
        .arg(&independent_evidence_path)
        .arg(repository.join("experiments/configs/kicad-fixed-via-shared-tree-independent.json"))
        .status()
        .expect("run independent-particle shared-via control");
    assert!(independent.success());
    assert_eq!(
        std::fs::read(&independent_committed).expect("read independent rollback"),
        imported_bytes
    );
    let independent_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&independent_evidence_path)
            .expect("read independent-particle evidence"),
    )
    .expect("parse independent-particle evidence");
    assert_eq!(
        independent_evidence["status"].as_str(),
        Some("no_improvement")
    );
    assert!(
        independent_evidence["proposal"]["length_mm"]
            .as_f64()
            .zip(independent_evidence["before"]["length_mm"].as_f64())
            .is_some_and(|(proposal, before)| proposal > before + 50.0)
    );

    let applied = Command::new(binary)
        .args(["apply-kicad-route-candidate"])
        .arg(&future_board)
        .arg(&independent_proposal)
        .arg(&future_board)
        .status()
        .expect("apply rejected independent-particle proposal for native control");
    assert!(applied.success());
    let verified = Command::new(binary)
        .args(["verify-kicad-rung"])
        .arg(&future_rung)
        .arg("esp32-c3")
        .status()
        .expect("verify rejected independent-particle proposal");
    assert!(verified.success());

    for (directory, board) in [(&rung, &board), (&future_rung, &future_board)] {
        let applied = Command::new(binary)
            .args(["apply-kicad-route-candidate"])
            .arg(board)
            .arg(&joint_committed)
            .arg(board)
            .status()
            .expect("apply joint shared-via motion");
        assert!(applied.success());
        let verified = Command::new(binary)
            .args(["verify-kicad-rung"])
            .arg(directory)
            .arg("esp32-c3")
            .status()
            .expect("verify joint shared-via motion");
        assert!(verified.success());
    }

    std::fs::remove_dir_all(output).expect("remove isolated tree-import test output");
}

#[test]
fn every_promoted_connection_prefix_passes_the_kicad_gate() {
    if !Command::new("kicad-cli")
        .arg("--version")
        .status()
        .is_ok_and(|status| status.success())
    {
        eprintln!("skipping KiCad integration test: kicad-cli is unavailable");
        return;
    }

    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let declaration = repository.join("benchmarks/esp32-c3-ladder/declaration.json");
    let output =
        std::env::temp_dir().join(format!("pcb-maker-kicad-ladder-{}", std::process::id()));
    let binary = env!("CARGO_BIN_EXE_pcb-maker");

    for rung in 0..=42 {
        let materialized = Command::new(binary)
            .args(["materialize-kicad-rung"])
            .arg(&declaration)
            .arg(rung.to_string())
            .arg(&output)
            .status()
            .expect("run materializer");
        assert!(materialized.success(), "rung {rung} did not materialize");

        let directory = std::fs::read_dir(&output)
            .expect("read materialized output")
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{rung:02}-"))
            })
            .unwrap_or_else(|| panic!("materializer did not create rung {rung}"))
            .path();
        let verified = Command::new(binary)
            .args(["verify-kicad-rung"])
            .arg(directory)
            .arg("esp32-c3")
            .status()
            .expect("run KiCad verification gate");
        assert!(verified.success(), "rung {rung} did not verify");
    }

    std::fs::remove_dir_all(output).expect("remove isolated test output");
}

#[test]
fn future_aware_route_processors_improve_3v3_and_keep_later_rungs_valid() {
    if !Command::new("kicad-cli")
        .arg("--version")
        .status()
        .is_ok_and(|status| status.success())
    {
        eprintln!("skipping KiCad integration test: kicad-cli is unavailable");
        return;
    }

    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let declaration = repository.join("benchmarks/esp32-c3-ladder/declaration.json");
    let source_candidate =
        repository.join("benchmarks/esp32-c3-ladder/candidates/3v3-dut-rung41.json");
    let output =
        std::env::temp_dir().join(format!("pcb-maker-kicad-shortening-{}", std::process::id()));
    let binary = env!("CARGO_BIN_EXE_pcb-maker");

    for rung in [41, 42] {
        let materialized = Command::new(binary)
            .args(["materialize-kicad-rung"])
            .arg(&declaration)
            .arg(rung.to_string())
            .arg(&output)
            .status()
            .expect("run materializer");
        assert!(materialized.success(), "rung {rung} did not materialize");
    }

    let rung_41 = output.join("41-3v3-dut");
    let rung_42 = output.join("42-gnd");
    let greedy_candidate = output.join("3v3-dut-rung41-future-aware-greedy.json");
    let greedy_evidence_path = output.join("3v3-dut-rung41-future-aware-greedy-evidence.json");
    let shortened = Command::new(binary)
        .args(["shorten-kicad-route-candidate"])
        .arg(rung_42.join("esp32-c3.kicad_pcb"))
        .arg(&source_candidate)
        .arg(&greedy_candidate)
        .arg(&greedy_evidence_path)
        .status()
        .expect("run line-of-sight shortener");
    assert!(shortened.success(), "future-aware shortening failed");

    let greedy_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&greedy_evidence_path).expect("read shortening evidence"),
    )
    .expect("parse shortening evidence");
    assert_eq!(greedy_evidence["status"].as_str(), Some("no_improvement"));
    assert_eq!(greedy_evidence["after"], greedy_evidence["before"]);
    assert!(
        greedy_evidence["proposal"]["length_mm"]
            .as_f64()
            .zip(greedy_evidence["before"]["length_mm"].as_f64())
            .is_some_and(|(proposal, before)| proposal > before)
    );
    assert!(
        greedy_evidence["proposal"]["stored_track_length_mm"]
            .as_f64()
            .zip(greedy_evidence["before"]["stored_track_length_mm"].as_f64())
            .is_some_and(|(proposal, before)| proposal < before)
    );

    let shortest_candidate = output.join("3v3-dut-rung41-future-aware-shortest.json");
    let shortest_evidence_path = output.join("3v3-dut-rung41-future-aware-shortest-evidence.json");
    let shortest = Command::new(binary)
        .args(["shorten-kicad-route-candidate"])
        .arg(rung_42.join("esp32-c3.kicad_pcb"))
        .arg(&source_candidate)
        .arg(&shortest_candidate)
        .arg(&shortest_evidence_path)
        .arg(repository.join("experiments/configs/kicad-retained-visibility-shortest-path.json"))
        .status()
        .expect("run retained visibility shortest-path processor");
    assert!(shortest.success(), "future-aware shortest path failed");
    let shortest_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&shortest_evidence_path).expect("read shortest-path evidence"),
    )
    .expect("parse shortest-path evidence");
    assert_eq!(shortest_evidence["schema_version"].as_u64(), Some(1));
    assert_eq!(
        shortest_evidence["strategy"].as_str(),
        Some("visibility_shortest_path")
    );
    assert_eq!(shortest_evidence["visibility_tests"].as_u64(), Some(201));
    assert_eq!(shortest_evidence["status"].as_str(), Some("no_improvement"));
    assert_eq!(shortest_evidence["after"], greedy_evidence["after"]);
    assert_eq!(shortest_evidence["proposal"], greedy_evidence["proposal"]);
    assert_eq!(
        shortest_evidence["after"]["vias"],
        shortest_evidence["before"]["vias"]
    );

    let relaxed_candidate = output.join("3v3-dut-rung41-projected-tension.json");
    let relaxed_proposal = output.join("3v3-dut-rung41-projected-tension-proposal.json");
    let relaxed_evidence_path = output.join("3v3-dut-rung41-projected-tension-evidence.json");
    let relaxed = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(rung_42.join("esp32-c3.kicad_pcb"))
        .arg(&source_candidate)
        .arg(&relaxed_candidate)
        .arg(&relaxed_proposal)
        .arg(&relaxed_evidence_path)
        .arg(repository.join("experiments/configs/kicad-projected-tension-branch-0.json"))
        .status()
        .expect("run projected trace-tension processor");
    assert!(relaxed.success(), "projected trace tension failed");
    let relaxed_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&relaxed_evidence_path).expect("read relaxation evidence"),
    )
    .expect("parse relaxation evidence");
    assert_eq!(relaxed_evidence["status"].as_str(), Some("accepted"));
    assert_eq!(
        relaxed_evidence["initial_geometry_clear"].as_bool(),
        Some(true)
    );
    assert_eq!(
        relaxed_evidence["proposal_geometry_clear"].as_bool(),
        Some(true)
    );
    assert!(
        relaxed_evidence["after"]["length_mm"].as_f64()
            < shortest_evidence["after"]["length_mm"].as_f64()
    );
    assert!(
        relaxed_evidence["work"]["constraint_projections"]
            .as_u64()
            .is_some_and(|work| work > 0)
    );

    let local_candidate = output.join("3v3-dut-rung41-projected-tension-local.json");
    let local_proposal = output.join("3v3-dut-rung41-projected-tension-local-proposal.json");
    let local_evidence_path = output.join("3v3-dut-rung41-projected-tension-local-evidence.json");
    let local = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(rung_42.join("esp32-c3.kicad_pcb"))
        .arg(&source_candidate)
        .arg(&local_candidate)
        .arg(&local_proposal)
        .arg(&local_evidence_path)
        .arg(repository.join("experiments/configs/kicad-projected-tension-branch-0-local.json"))
        .status()
        .expect("run local projected trace-tension processor");
    assert!(local.success(), "local projected trace tension failed");
    let local_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&local_evidence_path).expect("read local relaxation evidence"),
    )
    .expect("parse local relaxation evidence");
    assert_eq!(local_evidence["status"].as_str(), Some("accepted"));
    assert_eq!(local_evidence["broad_phase"].as_str(), Some("uniform_grid"));
    assert_eq!(
        std::fs::read_to_string(&local_candidate).expect("read local candidate"),
        std::fs::read_to_string(&relaxed_candidate).expect("read exhaustive candidate")
    );
    assert_eq!(
        std::fs::read_to_string(&local_proposal).expect("read local proposal"),
        std::fs::read_to_string(&relaxed_proposal).expect("read exhaustive proposal")
    );
    let exhaustive_projections = relaxed_evidence["work"]["constraint_projections"]
        .as_u64()
        .expect("exhaustive projection count");
    let local_projections = local_evidence["work"]["constraint_projections"]
        .as_u64()
        .expect("local projection count");
    assert!(local_projections * 10 < exhaustive_projections);
    assert!(
        local_evidence["work"]["culled_obstacle_pairs"]
            .as_u64()
            .is_some_and(|culled| culled > 0)
    );

    let shared_point_candidate = output.join("3v3-dut-rung41-via-free-shared-points.json");
    let shared_point_proposal = output.join("3v3-dut-rung41-via-free-shared-points-proposal.json");
    let shared_point_evidence_path =
        output.join("3v3-dut-rung41-via-free-shared-points-evidence.json");
    let shared_point = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(rung_42.join("esp32-c3.kicad_pcb"))
        .arg(&source_candidate)
        .arg(&shared_point_candidate)
        .arg(&shared_point_proposal)
        .arg(&shared_point_evidence_path)
        .arg(
            repository.join("experiments/configs/kicad-projected-tension-via-free-local-wide.json"),
        )
        .status()
        .expect("run exact-point-only shared-copper control");
    assert!(
        shared_point.success(),
        "exact-point-only shared-copper control failed"
    );
    let shared_point_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&shared_point_evidence_path)
            .expect("read exact-point-only shared-copper evidence"),
    )
    .expect("parse exact-point-only shared-copper evidence");
    assert_eq!(
        shared_point_evidence["status"].as_str(),
        Some("no_improvement")
    );
    assert_eq!(
        shared_point_evidence["shared_copper"].as_str(),
        Some("shared_points")
    );
    assert!(
        shared_point_evidence["proposal"]["length_mm"]
            .as_f64()
            .zip(shared_point_evidence["before"]["length_mm"].as_f64())
            .is_some_and(|(proposal, before)| proposal > before)
    );

    let graph_exhaustive_candidate = output.join("3v3-dut-rung41-via-free-graph-exhaustive.json");
    let graph_exhaustive_proposal =
        output.join("3v3-dut-rung41-via-free-graph-exhaustive-proposal.json");
    let graph_exhaustive_evidence_path =
        output.join("3v3-dut-rung41-via-free-graph-exhaustive-evidence.json");
    let graph_exhaustive = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(rung_42.join("esp32-c3.kicad_pcb"))
        .arg(&source_candidate)
        .arg(&graph_exhaustive_candidate)
        .arg(&graph_exhaustive_proposal)
        .arg(&graph_exhaustive_evidence_path)
        .arg(
            repository
                .join("experiments/configs/kicad-projected-tension-via-free-graph-exhaustive.json"),
        )
        .status()
        .expect("run route-graph exhaustive processor");
    assert!(graph_exhaustive.success(), "route-graph exhaustive failed");
    let graph_exhaustive_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&graph_exhaustive_evidence_path)
            .expect("read route-graph exhaustive evidence"),
    )
    .expect("parse route-graph exhaustive evidence");
    assert_eq!(
        graph_exhaustive_evidence["status"].as_str(),
        Some("accepted")
    );
    assert_eq!(
        graph_exhaustive_evidence["shared_copper"].as_str(),
        Some("shared_route_graph")
    );
    assert_eq!(
        graph_exhaustive_evidence["work"]["route_graph_inserted_points"].as_u64(),
        Some(7)
    );
    assert_eq!(
        graph_exhaustive_evidence["work"]["route_graph_contact_points"].as_u64(),
        Some(9)
    );
    assert_eq!(
        graph_exhaustive_evidence["work"]["deduplicated_tension_edges"].as_u64(),
        Some(16)
    );
    assert_eq!(
        graph_exhaustive_evidence["work"]["mixed_mobility_shared_point_groups"].as_u64(),
        Some(3)
    );

    let graph_local_candidate = output.join("3v3-dut-rung41-via-free-graph-local.json");
    let graph_local_proposal = output.join("3v3-dut-rung41-via-free-graph-local-proposal.json");
    let graph_local_evidence_path =
        output.join("3v3-dut-rung41-via-free-graph-local-evidence.json");
    let graph_local = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(rung_42.join("esp32-c3.kicad_pcb"))
        .arg(&shortest_candidate)
        .arg(&graph_local_candidate)
        .arg(&graph_local_proposal)
        .arg(&graph_local_evidence_path)
        .arg(
            repository
                .join("experiments/configs/kicad-projected-tension-via-free-graph-local.json"),
        )
        .status()
        .expect("run route-graph local processor");
    assert!(graph_local.success(), "route-graph local failed");
    let graph_local_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&graph_local_evidence_path)
            .expect("read route-graph local evidence"),
    )
    .expect("parse route-graph local evidence");
    assert_eq!(graph_local_evidence["status"].as_str(), Some("accepted"));
    assert_eq!(
        std::fs::read_to_string(&graph_local_candidate).expect("read graph local candidate"),
        std::fs::read_to_string(&graph_exhaustive_candidate)
            .expect("read graph exhaustive candidate")
    );
    assert_eq!(
        std::fs::read_to_string(&graph_local_proposal).expect("read graph local proposal"),
        std::fs::read_to_string(&graph_exhaustive_proposal)
            .expect("read graph exhaustive proposal")
    );
    let graph_exhaustive_projections = graph_exhaustive_evidence["work"]["constraint_projections"]
        .as_u64()
        .expect("graph exhaustive projection count");
    let graph_local_projections = graph_local_evidence["work"]["constraint_projections"]
        .as_u64()
        .expect("graph local projection count");
    assert!(graph_local_projections * 10 < graph_exhaustive_projections);

    for directory in [rung_41, rung_42] {
        let board = directory.join("esp32-c3.kicad_pcb");
        let applied = Command::new(binary)
            .args(["apply-kicad-route-candidate"])
            .arg(&board)
            .arg(&graph_local_candidate)
            .arg(&board)
            .status()
            .expect("apply shortened route");
        assert!(applied.success(), "relaxed candidate did not apply");
        let verified = Command::new(binary)
            .args(["verify-kicad-rung"])
            .arg(&directory)
            .arg("esp32-c3")
            .status()
            .expect("run KiCad verification gate");
        assert!(
            verified.success(),
            "relaxed candidate did not pass {}",
            directory.display()
        );
    }

    std::fs::remove_dir_all(output).expect("remove isolated test output");
}

#[test]
fn trace_tension_can_move_a_real_rung_one_footprint_without_breaking_kicad() {
    if !Command::new("kicad-cli")
        .arg("--version")
        .status()
        .is_ok_and(|status| status.success())
    {
        eprintln!("skipping KiCad integration test: kicad-cli is unavailable");
        return;
    }

    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let declaration = repository.join("benchmarks/esp32-c3-ladder/declaration.json");
    let output = std::env::temp_dir().join(format!(
        "pcb-maker-kicad-component-coupling-{}",
        std::process::id()
    ));
    let binary = env!("CARGO_BIN_EXE_pcb-maker");

    let materialized = Command::new(binary)
        .args(["materialize-kicad-rung"])
        .arg(&declaration)
        .arg("1")
        .arg(&output)
        .status()
        .expect("materialize rung one");
    assert!(materialized.success());
    let rung = output.join("01-t-local-comm0");
    let board = rung.join("esp32-c3.kicad_pcb");
    let source = output.join("source.json");
    let routed = Command::new(binary)
        .args(["route-kicad-connection"])
        .arg(&board)
        .arg("T_LOCAL_COMM0")
        .arg(&source)
        .status()
        .expect("route rung-one connection");
    assert!(routed.success());

    let courtyard_candidate = output.join("courtyard.json");
    let courtyard_proposal = output.join("courtyard-proposal.json");
    let courtyard_evidence_path = output.join("courtyard-evidence.json");
    let relaxed = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&board)
        .arg(&source)
        .arg(&courtyard_candidate)
        .arg(&courtyard_proposal)
        .arg(&courtyard_evidence_path)
        .arg(repository.join("experiments/configs/kicad-rung1-r1-translation-courtyard.json"))
        .status()
        .expect("run courtyard-protected footprint relaxation");
    assert!(relaxed.success());
    let courtyard_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&courtyard_evidence_path).expect("read courtyard evidence"),
    )
    .expect("parse courtyard evidence");
    assert_eq!(courtyard_evidence["status"].as_str(), Some("accepted"));
    assert_eq!(
        courtyard_evidence["placement_clearance"].as_str(),
        Some("preserve_initial_courtyard")
    );
    assert_eq!(
        courtyard_evidence["work"]["compiled_body_body_pairs"].as_u64(),
        Some(2)
    );
    assert!(
        courtyard_evidence["footprint_motions"][0]["translation_mm"]
            .as_f64()
            .is_some_and(|motion| motion > 0.2)
    );
    assert!(
        courtyard_evidence["footprint_motions"][0]["proposal_at"][1]
            .as_f64()
            .is_some_and(|y| (y - 29.0).abs() < 1.0e-3)
    );

    let courtyard_output = output.join("courtyard-rung");
    let materialized = Command::new(binary)
        .args(["materialize-kicad-rung"])
        .arg(&declaration)
        .arg("1")
        .arg(&courtyard_output)
        .status()
        .expect("materialize courtyard-protected rung");
    assert!(materialized.success());
    let courtyard_rung = courtyard_output.join("01-t-local-comm0");
    let courtyard_board = courtyard_rung.join("esp32-c3.kicad_pcb");
    let applied = Command::new(binary)
        .args(["apply-kicad-route-candidate"])
        .arg(&courtyard_board)
        .arg(&courtyard_candidate)
        .arg(&courtyard_board)
        .status()
        .expect("apply courtyard-protected candidate");
    assert!(applied.success());
    let verified = Command::new(binary)
        .args(["verify-kicad-rung"])
        .arg(&courtyard_rung)
        .arg("esp32-c3")
        .status()
        .expect("verify courtyard-protected rung");
    assert!(verified.success());

    let valid_reference_portfolio = output.join("valid-reference-portfolio");
    let reference_searched = Command::new(binary)
        .args(["search-kicad-reference-fields"])
        .arg(&rung)
        .arg("esp32-c3")
        .arg(&courtyard_candidate)
        .arg(&valid_reference_portfolio)
        .status()
        .expect("avoid reference actions for an already valid candidate");
    assert!(reference_searched.success());
    let valid_reference_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(valid_reference_portfolio.join("portfolio.json"))
            .expect("read valid reference-action portfolio evidence"),
    )
    .expect("parse valid reference-action portfolio evidence");
    assert_eq!(
        valid_reference_evidence["selected_source"].as_bool(),
        Some(true)
    );
    assert_eq!(
        valid_reference_evidence["generated_actions"].as_u64(),
        Some(0)
    );
    assert_eq!(
        valid_reference_evidence["attempted_actions"].as_u64(),
        Some(0)
    );
    assert_eq!(
        std::fs::read(valid_reference_portfolio.join("selected-candidate.json"))
            .expect("read selected valid source"),
        std::fs::read(&courtyard_candidate).expect("read valid source candidate")
    );

    let free_candidate = output.join("free.json");
    let free_proposal = output.join("free-proposal.json");
    let free_evidence_path = output.join("free-evidence.json");
    let relaxed = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&board)
        .arg(&source)
        .arg(&free_candidate)
        .arg(&free_proposal)
        .arg(&free_evidence_path)
        .arg(repository.join("experiments/configs/kicad-rung1-r1-free-control.json"))
        .status()
        .expect("run free footprint relaxation");
    assert!(relaxed.success());
    let free_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&free_evidence_path).expect("read free evidence"),
    )
    .expect("parse free evidence");
    assert!(
        free_evidence["footprint_motions"][0]["rotation_degrees"]
            .as_f64()
            .is_some_and(|rotation| rotation > 1.0)
    );

    let free_output = output.join("free-rung");
    let materialized = Command::new(binary)
        .args(["materialize-kicad-rung"])
        .arg(&declaration)
        .arg("1")
        .arg(&free_output)
        .status()
        .expect("materialize free-rotation rung");
    assert!(materialized.success());
    let free_rung = free_output.join("01-t-local-comm0");
    let free_board = free_rung.join("esp32-c3.kicad_pcb");
    let applied = Command::new(binary)
        .args(["apply-kicad-route-candidate"])
        .arg(&free_board)
        .arg(&free_proposal)
        .arg(&free_board)
        .status()
        .expect("apply unprotected free-rotation candidate");
    assert!(applied.success());
    let verified = Command::new(binary)
        .args(["verify-kicad-rung"])
        .arg(&free_rung)
        .arg("esp32-c3")
        .status()
        .expect("verify unprotected free-rotation rung");
    assert!(!verified.success());

    let reference_portfolio = output.join("free-reference-portfolio");
    let reference_searched = Command::new(binary)
        .args(["search-kicad-reference-fields"])
        .arg(&rung)
        .arg("esp32-c3")
        .arg(&free_proposal)
        .arg(&reference_portfolio)
        .status()
        .expect("search the implicated reference-field actions");
    assert!(reference_searched.success());
    let reference_evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(reference_portfolio.join("portfolio.json"))
            .expect("read reference-action portfolio evidence"),
    )
    .expect("parse reference-action portfolio evidence");
    assert_eq!(reference_evidence["generated_actions"].as_u64(), Some(4));
    assert_eq!(reference_evidence["unique_actions"].as_u64(), Some(3));
    assert_eq!(reference_evidence["attempted_actions"].as_u64(), Some(3));
    assert_eq!(
        reference_evidence["exact_complete_actions"].as_u64(),
        Some(2)
    );
    assert_eq!(reference_evidence["selected_phase"].as_u64(), Some(2));
    assert_eq!(
        reference_evidence["duplicate_actions"][0]["phase"].as_u64(),
        Some(4)
    );
    assert_eq!(
        reference_evidence["duplicate_actions"][0]["duplicate_of_phase"].as_u64(),
        Some(1)
    );
    assert_eq!(
        reference_evidence["attempts"][0]["verification"]["complete"].as_bool(),
        Some(false)
    );
    assert_eq!(
        reference_evidence["attempts"][1]["verification"]["complete"].as_bool(),
        Some(true)
    );
    assert_eq!(
        reference_evidence["attempts"][2]["verification"]["complete"].as_bool(),
        Some(true)
    );
    assert_eq!(
        reference_evidence["attempts"][1]["selected"].as_bool(),
        Some(true)
    );
    let repaired_candidate = reference_portfolio.join("selected-candidate.json");

    let repaired_output = output.join("free-reference-rung");
    let materialized = Command::new(binary)
        .args(["materialize-kicad-rung"])
        .arg(&declaration)
        .arg("1")
        .arg(&repaired_output)
        .status()
        .expect("materialize reference-repaired rung");
    assert!(materialized.success());
    let repaired_rung = repaired_output.join("01-t-local-comm0");
    let repaired_board = repaired_rung.join("esp32-c3.kicad_pcb");
    let applied = Command::new(binary)
        .args(["apply-kicad-route-candidate"])
        .arg(&repaired_board)
        .arg(&repaired_candidate)
        .arg(&repaired_board)
        .status()
        .expect("apply reference-repaired candidate");
    assert!(applied.success());
    let verified = Command::new(binary)
        .args(["verify-kicad-rung"])
        .arg(&repaired_rung)
        .arg("esp32-c3")
        .status()
        .expect("verify reference-repaired rung");
    assert!(verified.success());

    let candidate = output.join("horizontal.json");
    let proposal = output.join("horizontal-proposal.json");
    let evidence_path = output.join("horizontal-evidence.json");
    let relaxed = Command::new(binary)
        .args(["relax-kicad-route-candidate"])
        .arg(&board)
        .arg(&source)
        .arg(&candidate)
        .arg(&proposal)
        .arg(&evidence_path)
        .arg(repository.join("experiments/configs/kicad-rung1-r1-horizontal-control.json"))
        .status()
        .expect("run footprint-coupled relaxation");
    assert!(relaxed.success());
    let evidence: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&evidence_path).expect("read coupling evidence"),
    )
    .expect("parse coupling evidence");
    assert_eq!(evidence["status"].as_str(), Some("accepted"));
    assert!(
        evidence["footprint_motions"][0]["translation_mm"]
            .as_f64()
            .is_some_and(|motion| motion > 0.2)
    );
    assert_eq!(
        evidence["footprint_motions"][0]["rotation_degrees"].as_f64(),
        Some(0.0)
    );

    let applied = Command::new(binary)
        .args(["apply-kicad-route-candidate"])
        .arg(&board)
        .arg(&candidate)
        .arg(&board)
        .status()
        .expect("apply footprint-coupled candidate");
    assert!(applied.success());
    let verified = Command::new(binary)
        .args(["verify-kicad-rung"])
        .arg(&rung)
        .arg("esp32-c3")
        .status()
        .expect("verify footprint-coupled rung");
    assert!(verified.success());

    let mut persisted_declaration: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&declaration).expect("read ladder declaration"),
    )
    .expect("parse ladder declaration");
    persisted_declaration["schematic_template"] = serde_json::Value::String(
        repository
            .join("benchmarks/imported/testing-esp32-duts/esp32-c3/esp32-c3.kicad_sch")
            .display()
            .to_string(),
    );
    persisted_declaration["pcb_template"] = serde_json::Value::String(
        repository
            .join("benchmarks/imported/testing-esp32-duts/esp32-c3/esp32-c3.kicad_pcb")
            .display()
            .to_string(),
    );
    persisted_declaration["replacement_route_candidates"] =
        serde_json::json!([repaired_candidate.display().to_string()]);
    let persisted_declaration_path = output.join("placement-declaration.json");
    std::fs::write(
        &persisted_declaration_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&persisted_declaration)
                .expect("serialize placement declaration")
        ),
    )
    .expect("write placement declaration");
    let persisted_output = output.join("persisted");
    let materialized = Command::new(binary)
        .args(["materialize-kicad-rung"])
        .arg(&persisted_declaration_path)
        .arg("1")
        .arg(&persisted_output)
        .status()
        .expect("materialize persisted placement candidate");
    assert!(materialized.success());
    let persisted_rung = persisted_output.join("01-t-local-comm0");
    let verified = Command::new(binary)
        .args(["verify-kicad-rung"])
        .arg(&persisted_rung)
        .arg("esp32-c3")
        .status()
        .expect("verify persisted footprint-coupled rung");
    assert!(verified.success());
    let persisted_board = std::fs::read_to_string(persisted_rung.join("esp32-c3.kicad_pcb"))
        .expect("read persisted board");
    assert!(
        persisted_board.contains("(at 52.2801628112793 29.047855377197266 1.7674922343103932)")
    );
    assert!(persisted_board.contains("(at 2.2 0 0)"));

    std::fs::remove_dir_all(output).expect("remove isolated coupling test output");
}

// Copyright (C) 2026 Toit contributors.

use std::{fs, path::PathBuf, process::Command};

#[test]
fn file_level_six_action_frontier_is_exact_and_viewable() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let problem = repository.join("benchmarks/small/conflict-action-orthogonal-crossing.json");
    let config = repository.join("experiments/configs/conflict-action-default.json");
    let output =
        std::env::temp_dir().join(format!("pcb-maker-conflict-actions-{}", std::process::id()));
    fs::create_dir_all(&output).expect("create isolated output");
    let result_path = output.join("result.json");
    let viewer_path = output.join("viewer.html");
    let binary = env!("CARGO_BIN_EXE_pcb-maker");

    let explored = Command::new(binary)
        .arg("explore-conflict-actions")
        .arg(&problem)
        .arg(&result_path)
        .arg(&config)
        .status()
        .expect("run conflict-action frontier");
    assert!(explored.success());

    let result: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&result_path).expect("read result"))
            .expect("parse result");
    assert_eq!(result["evidence"]["generated_actions"], 6);
    assert_eq!(result["evidence"]["exact_complete_actions"], 6);
    assert_eq!(result["evidence"]["selected_attempt"], 4);
    assert_eq!(
        result["attempts"][4]["action"]["id"],
        "A_WEST_EAST_via_bottom"
    );
    assert!(
        result["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|attempt| {
                attempt["validation"]["complete"] == true
                    && attempt["state_id"].as_str().is_some()
                    && attempt["score"]["estimated_total_cost_mm"]
                        .as_f64()
                        .is_some()
            })
    );

    let viewed = Command::new(binary)
        .arg("view-route")
        .arg(&problem)
        .arg(&result_path)
        .arg(&viewer_path)
        .status()
        .expect("render conflict-action viewer");
    assert!(viewed.success());
    let viewer = fs::read_to_string(viewer_path).expect("read viewer");
    assert!(viewer.contains("A_WEST_EAST_around_B_NORTH_SOUTH_north"));
    assert!(viewer.contains("A_WEST_EAST_via_bottom"));
    assert!(viewer.contains("\"state_id\":\"seed\""));
    assert!(viewer.contains("\"selected_attempt\":5"));
    assert!(viewer.contains("state_id:attempt.state_id"));
    assert!(viewer.contains("score:attempt.score"));

    fs::remove_dir_all(output).expect("remove isolated output");
}

#[test]
fn file_level_best_first_search_resolves_two_crossings_and_retains_the_state_graph() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let problem = repository.join("benchmarks/small/conflict-action-two-crossings.json");
    let config = repository.join("experiments/configs/conflict-action-best-first-default.json");
    let output = std::env::temp_dir().join(format!(
        "pcb-maker-best-first-conflict-actions-{}",
        std::process::id()
    ));
    fs::create_dir_all(&output).expect("create isolated output");
    let result_path = output.join("result.json");
    let viewer_path = output.join("viewer.html");
    let binary = env!("CARGO_BIN_EXE_pcb-maker");

    let searched = Command::new(binary)
        .arg("search-conflict-actions")
        .arg(&problem)
        .arg(&result_path)
        .arg(&config)
        .status()
        .expect("run best-first conflict-action search");
    assert!(searched.success());

    let result: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&result_path).expect("read result"))
            .expect("parse result");
    assert_eq!(result["evidence"]["expanded_states"], 13);
    assert_eq!(result["evidence"]["generated_transitions"], 84);
    assert_eq!(result["evidence"]["unique_states"], 49);
    assert_eq!(result["evidence"]["duplicate_states"], 36);
    assert_eq!(result["evidence"]["exact_complete_states"], 36);
    assert_eq!(result["evidence"]["selected_attempt"], 13);
    assert_eq!(result["attempts"][0]["state_id"], "seed");
    assert_eq!(result["attempts"][13]["depth"], 2);
    assert_eq!(
        result["attempts"][13]["action_history"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(result["validation"]["complete"], true);

    let viewed = Command::new(binary)
        .arg("view-route")
        .arg(&problem)
        .arg(&result_path)
        .arg(&viewer_path)
        .status()
        .expect("render best-first conflict-action viewer");
    assert!(viewed.success());
    let viewer = fs::read_to_string(viewer_path).expect("read viewer");
    assert!(viewer.contains("\"state_id\":\"seed\""));
    assert!(viewer.contains("\"selected_attempt\":13"));
    assert!(viewer.contains("\"outcome\":\"duplicate\""));

    fs::remove_dir_all(output).expect("remove isolated output");
}

#[test]
fn shared_trace_search_exposes_dead_ends_and_post_action_improvement() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let problem = repository.join("benchmarks/small/conflict-action-shared-trace.json");
    let config =
        repository.join("experiments/configs/conflict-action-shared-trace-vertex-pull.json");
    let output = std::env::temp_dir().join(format!(
        "pcb-maker-shared-trace-conflict-actions-{}",
        std::process::id()
    ));
    fs::create_dir_all(&output).expect("create isolated output");
    let result_path = output.join("result.json");
    let viewer_path = output.join("viewer.html");
    let binary = env!("CARGO_BIN_EXE_pcb-maker");

    let searched = Command::new(binary)
        .arg("search-conflict-actions")
        .arg(&problem)
        .arg(&result_path)
        .arg(&config)
        .status()
        .expect("run shared-trace conflict-action search");
    assert!(searched.success());
    let result: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&result_path).expect("read result"))
            .expect("parse result");
    assert_eq!(result["evidence"]["post_processor"], "exact-vertex-pull-v1");
    assert_eq!(result["evidence"]["exact_complete_states"], 12);
    assert_eq!(result["validation"]["complete"], true);
    assert!(result["attempts"].as_array().unwrap().iter().any(|state| {
        state["post_action"]["status"] == "improved"
            && state["post_action"]["output_trace_length_mm"]
                .as_f64()
                .unwrap()
                < state["post_action"]["input_trace_length_mm"]
                    .as_f64()
                    .unwrap()
    }));
    assert!(result["attempts"].as_array().unwrap().iter().any(|state| {
        state["expansion_error"]
            .as_str()
            .is_some_and(|error| error.contains("not one straight via-free segment"))
    }));

    let viewed = Command::new(binary)
        .arg("view-route")
        .arg(&problem)
        .arg(&result_path)
        .arg(&viewer_path)
        .status()
        .expect("render shared-trace conflict-action viewer");
    assert!(viewed.success());
    let viewer = fs::read_to_string(viewer_path).expect("read viewer");
    assert!(viewer.contains("exact-vertex-pull-v1"));
    assert!(viewer.contains("not one straight via-free segment"));
    assert!(viewer.contains("post_action:attempt.post_action"));

    fs::remove_dir_all(output).expect("remove isolated output");
}

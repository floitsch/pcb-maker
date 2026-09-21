// Copyright (C) 2026 Toit contributors.

use std::{fs, path::PathBuf, process::Command};

#[test]
fn clear_board_continuation_reaches_the_exact_target_and_renders_every_stage() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let problem = repository.join("benchmarks/small/board-continuation-clear.json");
    let config = repository.join("experiments/configs/board-continuation-default.json");
    let output = std::env::temp_dir().join(format!(
        "pcb-maker-board-continuation-clear-{}",
        std::process::id()
    ));
    fs::create_dir_all(&output).expect("create isolated output");
    let result_path = output.join("result.json");
    let image_prefix = output.join("stage");
    let binary = env!("CARGO_BIN_EXE_pcb-maker");

    let continued = Command::new(binary)
        .arg("continue-board")
        .arg(&problem)
        .arg(&result_path)
        .arg(&config)
        .arg(&image_prefix)
        .status()
        .expect("run clear continuation");
    assert!(continued.success());
    let result: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&result_path).expect("read result"))
            .expect("parse result");
    assert_eq!(result["evidence"]["target_complete"], true);
    assert_eq!(result["evidence"]["accepted_stages"], 6);
    assert_eq!(result["evidence"]["last_valid_scale"], 1.0);
    assert_eq!(result["attempts"].as_array().unwrap().len(), 6);
    assert_eq!(result["validation"]["complete"], true);
    for stage in 0..6 {
        for side in ["front", "back"] {
            assert!(
                output
                    .join(format!("stage-stage-{stage:02}-{side}.svg"))
                    .is_file()
            );
        }
    }
    fs::remove_dir_all(output).expect("remove isolated output");
}

#[test]
fn bottleneck_continuation_fails_closed_and_keeps_visual_failure_evidence() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let problem = repository.join("benchmarks/small/board-continuation-bottleneck.json");
    let config = repository.join("experiments/configs/board-continuation-default.json");
    let output = std::env::temp_dir().join(format!(
        "pcb-maker-board-continuation-bottleneck-{}",
        std::process::id()
    ));
    fs::create_dir_all(&output).expect("create isolated output");
    let result_path = output.join("result.json");
    let viewer_path = output.join("viewer.html");
    let image_prefix = output.join("stage");
    let binary = env!("CARGO_BIN_EXE_pcb-maker");

    let continued = Command::new(binary)
        .arg("continue-board")
        .arg(&problem)
        .arg(&result_path)
        .arg(&config)
        .arg(&image_prefix)
        .status()
        .expect("run bottleneck continuation");
    assert!(!continued.success());
    let result: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&result_path).expect("read result"))
            .expect("parse result");
    assert_eq!(result["evidence"]["target_complete"], false);
    assert_eq!(result["evidence"]["stopped_at_stage"], 4);
    assert_eq!(result["evidence"]["last_valid_scale"], 1.2);
    assert_eq!(result["evidence"]["accepted_stages"], 4);
    assert_eq!(result["attempts"][4]["accepted"], false);
    assert_eq!(result["validation"]["complete"], false);
    assert!(output.join("stage-stage-04-front.svg").is_file());
    assert!(
        fs::read_to_string(output.join("stage-stage-04-front.svg"))
            .unwrap()
            .contains("#ff6363")
    );

    let viewed = Command::new(binary)
        .arg("view-route")
        .arg(&problem)
        .arg(&result_path)
        .arg(&viewer_path)
        .status()
        .expect("render continuation viewer");
    assert!(viewed.success());
    let viewer = fs::read_to_string(viewer_path).expect("read viewer");
    assert!(viewer.contains("\"scale\":1.2"));
    assert!(viewer.contains("\"accepted\":false"));
    assert!(viewer.contains("attempt.board_bounds||data.problem.board.bounds"));
    fs::remove_dir_all(output).expect("remove isolated output");
}

#[test]
fn retained_continuation_repairs_signal_and_preserves_upper_at_every_shrink() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let problem = repository.join("benchmarks/small/board-continuation-local-repair.json");
    let config =
        repository.join("experiments/configs/board-continuation-retained-local-repair.json");
    let output = std::env::temp_dir().join(format!(
        "pcb-maker-board-continuation-retained-{}",
        std::process::id()
    ));
    fs::create_dir_all(&output).expect("create isolated output");
    let result_path = output.join("result.json");
    let viewer_path = output.join("viewer.html");
    let image_prefix = output.join("stage");
    let binary = env!("CARGO_BIN_EXE_pcb-maker");

    let continued = Command::new(binary)
        .arg("continue-board")
        .arg(&problem)
        .arg(&result_path)
        .arg(&config)
        .arg(&image_prefix)
        .status()
        .expect("run retained continuation");
    assert!(continued.success());
    let result: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&result_path).expect("read result"))
            .expect("parse result");
    assert_eq!(
        result["evidence"]["producer"],
        "retained-affine-local-grid-repair-v1"
    );
    assert_eq!(result["evidence"]["target_complete"], true);
    assert_eq!(result["evidence"]["affine_reuse_stages"], 5);
    assert_eq!(result["evidence"]["local_repair_stages"], 5);
    assert_eq!(result["evidence"]["total_searched_branches"], 7);
    for stage in 1..6 {
        assert_eq!(result["attempts"][stage]["rerouted_branches"][0], "SIGNAL");
        assert_eq!(result["attempts"][stage]["retained_branch_count"], 1);
        assert_eq!(
            result["attempts"][stage]["deformation_validation"]["complete"],
            false
        );
    }
    for stage in 0..6 {
        for side in ["front", "back"] {
            assert!(
                output
                    .join(format!("stage-stage-{stage:02}-{side}.svg"))
                    .is_file()
            );
        }
    }
    let viewed = Command::new(binary)
        .arg("view-route")
        .arg(&problem)
        .arg(&result_path)
        .arg(&viewer_path)
        .status()
        .expect("render retained continuation viewer");
    assert!(viewed.success());
    let viewer = fs::read_to_string(viewer_path).expect("read viewer");
    assert!(viewer.contains("deformation_exact"));
    assert!(viewer.contains("rerouted_branches"));
    assert!(viewer.contains("\"SIGNAL\""));
    fs::remove_dir_all(output).expect("remove isolated output");
}

#[test]
fn continuous_continuation_moves_a_component_and_exposes_motion_in_images_and_viewer() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let problem = repository.join("benchmarks/small/board-continuation-force-motion.json");
    let config = repository.join("experiments/configs/board-continuation-force-motion.json");
    let output = std::env::temp_dir().join(format!(
        "pcb-maker-board-continuation-motion-{}",
        std::process::id()
    ));
    fs::create_dir_all(&output).expect("create isolated output");
    let result_path = output.join("result.json");
    let viewer_path = output.join("viewer.html");
    let image_prefix = output.join("stage");
    let binary = env!("CARGO_BIN_EXE_pcb-maker");

    let continued = Command::new(binary)
        .arg("continue-board")
        .arg(&problem)
        .arg(&result_path)
        .arg(&config)
        .arg(&image_prefix)
        .status()
        .expect("run continuous continuation");
    assert!(continued.success());
    let result: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&result_path).expect("read result"))
            .expect("parse result");
    assert_eq!(result["evidence"]["target_complete"], true);
    assert_eq!(result["evidence"]["total_searched_branches"], 2);
    assert_eq!(result["evidence"]["local_repair_stages"], 0);
    assert_eq!(result["evidence"]["continuous_motion_exact_stages"], 2);
    assert_eq!(
        result["attempts"][4]["continuous_motion"]["status"],
        "exact_complete"
    );
    assert_eq!(
        result["attempts"][4]["continuous_motion"]["body_motion"][0]["component"],
        "YIELDING_BLOCKER"
    );
    for stage in 0..6 {
        for side in ["front", "back"] {
            let image = output.join(format!("stage-stage-{stage:02}-{side}.svg"));
            assert!(image.is_file());
            if stage >= 4 {
                assert!(fs::read_to_string(image).unwrap().contains("m=exact"));
            }
        }
    }

    let viewed = Command::new(binary)
        .arg("view-route")
        .arg(&problem)
        .arg(&result_path)
        .arg(&viewer_path)
        .status()
        .expect("render continuous continuation viewer");
    assert!(viewed.success());
    let viewer = fs::read_to_string(viewer_path).expect("read viewer");
    assert!(viewer.contains("continuous_motion_status"));
    assert!(viewer.contains("continuous_motion_body_motion"));
    assert!(viewer.contains("continuous_motion_selected_obstacles"));
    assert!(viewer.contains("YIELDING_BLOCKER"));
    fs::remove_dir_all(output).expect("remove isolated output");
}

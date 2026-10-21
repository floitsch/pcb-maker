// Copyright (C) 2026 Toit contributors.

//! Hostile and odd input files through the command line
//! (docs/reviews/2026-10-21-review-robustness.md): each ends with an exit
//! code and a message, never a crash, a panic or a write outside the
//! output directory. The files are in tests/fixtures/hostile/; they are
//! read as boards and rules, never run.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hostile")
        .join(name)
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-hostile-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// A project of the fixture `board` as `t.kicad_pcb` (and `rules` as its
/// custom rules, `t.kicad_dru`).
fn project(root: &Path, board: &str, rules: Option<&str>) -> PathBuf {
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    fs::copy(fixture(board), project.join("t.kicad_pcb")).unwrap();
    if let Some(rules) = rules {
        fs::copy(fixture(rules), project.join("t.kicad_dru")).unwrap();
    }
    project
}

/// `route-kicad-board` without KiCad's check, the work rates pinned, the
/// address space capped (so that a gate that let a huge lattice through
/// fails this test rather than the machine).
fn route(project: &Path, board_id: &str, out: &Path, environment: &[(&str, &str)]) -> Output {
    let binary = env!("CARGO_BIN_EXE_pcb-maker");
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("ulimit -v 6000000; exec \"$0\" \"$@\"")
        .arg(binary)
        .args(["route-kicad-board"])
        .arg(project)
        .arg(board_id)
        .arg(out)
        .arg("auto")
        .env("RAYON_NUM_THREADS", "2")
        .env("PCB_SKIP_NATIVE", "1")
        .env("PCB_WORK_RATE", "3.15e6,150e6")
        .env_remove("PCB_MAKER_TEST_PANIC")
        .env_remove("PCB_MAKER_MEMORY_MB");
    for (name, value) in environment {
        command.env(name, value);
    }
    command.output().unwrap()
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or_else(|| {
        panic!(
            "killed by a signal: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The route of `board` exits 2 with an error line holding every one of
/// `needles`.
fn refused(board: &str, needles: &[&str]) {
    let root = scratch(board.trim_end_matches(".kicad_pcb"));
    let project = project(&root, board, None);
    let output = route(&project, "t", &root.join("out"), &[]);
    let text = stderr(&output);
    assert_eq!(code(&output), 2, "{board}: {text}");
    let error = text
        .lines()
        .find(|line| line.starts_with("error: "))
        .unwrap_or_else(|| panic!("{board}: no error line: {text}"));
    for needle in needles {
        assert!(error.contains(needle), "{board}: {needle:?} in {error}");
    }
    assert!(!error.contains("panic"), "{board}: {error}");
    fs::remove_dir_all(root).unwrap();
}

/// The route of `board` (and `rules`) completes as far as the router can
/// tell (exit 1: KiCad did not check); its report.
fn routed(board: &str, rules: Option<&str>) -> serde_json::Value {
    let root = scratch(board.trim_end_matches(".kicad_pcb"));
    let project = project(&root, board, rules);
    let out = root.join("out");
    let output = route(&project, "t", &out, &[("PCB_ROUTER_SEED", "0")]);
    assert_eq!(code(&output), 1, "{board}: {}", stderr(&output));
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("board-router.json")).unwrap()).unwrap();
    assert_eq!(report["verdict"]["status"], "router_complete_unverified");
    fs::remove_dir_all(root).unwrap();
    report
}

#[test]
fn lists_nested_past_the_limit_are_an_input_error_not_a_stack_overflow() {
    // 20 000 levels: the recursive parser of before died of SIGSEGV.
    refused(
        "deep-nesting.kicad_pcb",
        &["nested deeper than 1000 levels"],
    );
}

#[test]
fn numbers_that_are_not_finite_are_an_input_error_naming_the_item() {
    refused(
        "nan-rule-area.kicad_pcb",
        &["a rule area's xy coordinate is NaN", "finite"],
    );
    refused(
        "inf-outline.kicad_pcb",
        &["a gr_line's start coordinate 1 is inf"],
    );
    refused(
        "overflow-coordinate.kicad_pcb",
        &["the footprint J3's at coordinate 1 is 18e999"],
    );
}

#[test]
fn an_outline_past_two_metres_is_an_input_error() {
    refused(
        "huge-outline.kicad_pcb",
        &["spans 10000.0 x 20.0 mm", "Edge.Cuts"],
    );
}

#[test]
fn a_lattice_the_memory_cannot_hold_is_refused_before_it_is_allocated() {
    let root = scratch("huge-lattice");
    let project = project(&root, "huge-lattice.kicad_pcb", None);
    let out = root.join("out");
    let output = route(&project, "t", &out, &[("PCB_MAKER_MEMORY_MB", "4000")]);
    let text = stderr(&output);
    assert_eq!(code(&output), 2, "{text}");
    assert!(text.contains("1900.0 x 1900.0 mm"), "{text}");
    assert!(text.contains("more than the"), "{text}");
    // The verdict says what to fix: the extent and the pitch.
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "usage_error");
    let action = verdict["next_actions"][0].as_str().unwrap();
    assert!(action.contains("1900.0 x 1900.0 mm"), "{action}");
    assert!(action.contains("0.127 mm"), "{action}");
    // The retry with a fixed board takes the same output directory.
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates/pcb-kicad/tests/fixtures/plated-branch.kicad_pcb"),
        project.join("t.kicad_pcb"),
    )
    .unwrap();
    let output = route(&project, "t", &out, &[]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_footprint_without_its_name_is_an_input_error() {
    refused(
        "short-footprint.kicad_pcb",
        &["a footprint form without its name"],
    );
}

#[test]
fn a_board_id_that_is_no_file_stem_is_refused() {
    let root = scratch("board-id");
    let project = project(&root, "net-four-fields.kicad_pcb", None);
    for id in ["../t", "~t", "$t", ".."] {
        let output = route(&project, id, &root.join("out"), &[]);
        assert_eq!(code(&output), 2, "{id}: {}", stderr(&output));
        assert!(stderr(&output).contains("the board id"), "{id}");
    }
    // A command that reads the board only.
    let output = Command::new(env!("CARGO_BIN_EXE_pcb-maker"))
        .args(["describe-kicad-board"])
        .arg(&project)
        .arg("../project/t")
        .output()
        .unwrap();
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("the board id"),
        "{}",
        stderr(&output)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_text_box_of_one_corner_on_a_mask_layer_is_left_out_with_a_warning() {
    // Until 2026-10-21 every route and layout of the board panicked.
    let report = routed("text-box-one-corner.kicad_pcb", None);
    let warnings = report["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0]
            .as_str()
            .unwrap()
            .contains("J1's text box has fewer than three corners")
    );
}

#[test]
fn copper_on_layers_the_stack_lacks_is_named_in_the_warnings() {
    let report = routed("unknown-layers.kicad_pcb", None);
    let warnings: Vec<&str> = report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].starts_with("1 segment on In3.Cu (the first at 3.000, 3.000)"));
    assert!(warnings[1].starts_with("1 via on In7.Cu"));
    // The verdict leads with them.
    assert_eq!(report["verdict"]["next_actions"][0], warnings[0]);
}

#[test]
fn a_net_form_of_four_fields_and_thirty_two_layers_route() {
    // `(net 1 "/TARGET" x)`: the name is the third field (the copper was
    // named "x", and a second pad's net panicked).
    let report = routed("net-four-fields.kicad_pcb", None);
    assert_eq!(report["unconnected_terminals"], 0);
    // 32 copper layers: every layer's bit, not none (`1 << 32`).
    let report = routed("32-layers.kicad_pcb", None);
    assert_eq!(report["unconnected_terminals"], 0);
}

#[test]
fn custom_rule_conditions_past_the_depth_limit_are_unmodelled_not_a_crash() {
    let report = routed(
        "net-four-fields.kicad_pcb",
        Some("deep-condition.kicad_dru"),
    );
    // 5 000 parentheses and 5 000 "!": refused; 2 000 alternatives: read.
    let rules = report["rules"]["custom_rules"].as_array().unwrap();
    let detail = |rule: &str| {
        rules
            .iter()
            .find(|entry| entry["rule"] == rule)
            .map(|entry| entry["detail"].as_str().unwrap_or("").to_string())
            .unwrap()
    };
    for rule in ["deep parentheses", "many nots"] {
        assert_eq!(
            detail(rule),
            "the condition does not parse: the condition nests deeper than 200 levels"
        );
    }
    assert!(!detail("a long chain").contains("does not parse"));
    let report = routed("net-four-fields.kicad_pcb", Some("deep-nesting.kicad_dru"));
    assert!(
        report
            .to_string()
            .contains("nested deeper than 1000 levels"),
        "{report}"
    );
}

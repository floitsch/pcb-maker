// Copyright (C) 2026 Toit contributors.

//! The command line's exit codes and verdicts (`pcb-maker --help`, "Exit
//! codes"): 0 complete, 1 incomplete, 2 usage or input error, 3 cut by the
//! wall clock, 4 internal error or panic.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pcb-maker"));
    command.env("RAYON_NUM_THREADS", "2").env_remove("PCB_MAKER_TEST_PANIC");
    command
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-cli-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// A project with the three-pad test board as `t.kicad_pcb`.
fn project(root: &Path) -> PathBuf {
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/pcb-kicad/tests/fixtures/plated-branch.kicad_pcb");
    fs::copy(fixture, project.join("t.kicad_pcb")).unwrap();
    project
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("exited")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn verdict(report: &Path) -> serde_json::Value {
    let report: serde_json::Value = serde_json::from_str(&fs::read_to_string(report).unwrap()).unwrap();
    report["verdict"].clone()
}

#[test]
fn help_lists_the_agent_commands_and_the_exit_codes() {
    let output = binary().arg("--help").output().unwrap();
    assert_eq!(code(&output), 0);
    let text = String::from_utf8_lossy(&output.stdout);
    for needle in ["route-kicad-board", "layout-kicad-board", "place-kicad-board", "Exit codes", "  3  incomplete and cut short"] {
        assert!(text.contains(needle), "{needle}");
    }
    for command in ["route-kicad-board", "layout-kicad-board", "place-kicad-board", "import-kicad-netlist"] {
        let output = binary().args([command, "--help"]).output().unwrap();
        assert_eq!(code(&output), 0, "{command}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(&format!("usage: pcb-maker {command} ")), "{command}");
    }
    assert_eq!(code(&binary().args(["help", "verify-kicad-rung"]).output().unwrap()), 0);
}

#[test]
fn usage_and_input_errors_exit_2_with_one_line() {
    let root = scratch("usage");
    let project = project(&root);
    let cases: Vec<Vec<String>> = vec![
        vec!["no-such-command".into()],
        vec!["route-kicad-board".into()],
        vec!["route-kicad-board".into(), root.join("missing").display().to_string(), "t".into(), root.join("out").display().to_string()],
        vec!["route-kicad-board".into(), project.display().to_string(), "nope".into(), root.join("out").display().to_string()],
        // The output directory exists.
        vec!["route-kicad-board".into(), project.display().to_string(), "t".into(), project.display().to_string()],
        vec!["layout-kicad-board".into(), project.display().to_string(), "t".into(), root.join("out").display().to_string(), "--bogus".into()],
        vec!["describe-kicad-board".into(), project.display().to_string()],
    ];
    for arguments in cases {
        let output = binary().args(&arguments).output().unwrap();
        assert_eq!(code(&output), 2, "{arguments:?}: {}", stderr(&output));
        assert_eq!(stderr(&output).lines().count(), 1, "{arguments:?}: {}", stderr(&output));
    }
    // An unreadable router config: nothing is written.
    fs::write(root.join("router.json"), "{not json").unwrap();
    let out = root.join("out");
    let output = binary()
        .args(["route-kicad-board", &project.display().to_string(), "t", &out.display().to_string(), &root.join("router.json").display().to_string()])
        .output()
        .unwrap();
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(!out.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_complete_route_exits_0_and_its_report_says_so() {
    let root = scratch("route");
    let project = project(&root);
    let out = root.join("out");
    let output = binary()
        .env("PCB_SKIP_NATIVE", "1")
        .args(["route-kicad-board", &project.display().to_string(), "t", &out.display().to_string(), "auto"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let verdict = verdict(&out.join("board-router.json"));
    assert_eq!(verdict["status"], "complete");
    assert_eq!(verdict["exit_code"], 0);
    let file: serde_json::Value = serde_json::from_str(&fs::read_to_string(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(file, verdict);
    assert_eq!(verdict["open_connections"], 0);
    assert_eq!(verdict["cut"], serde_json::json!([]));
    assert!(verdict["work_seconds"].as_f64().is_some());
    assert!(verdict["outputs"]["board"].as_str().is_some_and(|path| Path::new(path).is_absolute() && Path::new(path).exists()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn with_kicads_check_the_exit_code_is_the_verdicts() {
    if !Command::new("kicad-cli").arg("version").output().is_ok_and(|output| output.status.success()) {
        eprintln!("skipping: kicad-cli is unavailable");
        return;
    }
    let root = scratch("native");
    let project = project(&root);
    let out = root.join("out");
    let output = binary()
        .args(["route-kicad-board", &project.display().to_string(), "t", &out.display().to_string(), "auto"])
        .output()
        .unwrap();
    let verdict = verdict(&out.join("board-router.json"));
    assert_eq!(verdict["exit_code"].as_i64(), Some(code(&output) as i64), "{}", stderr(&output));
    assert_eq!(verdict["kicad_unconnected_items"], 0);
    assert!(verdict["outputs"]["drc"].as_str().is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_panic_exits_4_with_one_line_and_a_report() {
    let root = scratch("panic");
    let project = project(&root);
    let out = root.join("out");
    let output = binary()
        .env("PCB_MAKER_TEST_PANIC", "1")
        .env_remove("RUST_BACKTRACE")
        .args(["route-kicad-board", &project.display().to_string(), "t", &out.display().to_string(), "auto"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 4);
    let text = stderr(&output);
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.contains("PCB_MAKER_TEST_PANIC"), "{text}");
    assert!(!out.join("board-router.json").exists());
    let verdict: serde_json::Value = serde_json::from_str(&fs::read_to_string(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "internal_error");
    assert_eq!(verdict["exit_code"], 4);
    assert!(verdict["error"].as_str().is_some_and(|error| error.contains("panic")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_closed_stdout_ends_quietly_with_exit_0() {
    // `pcb-maker --help | head -1`: the reader closes the pipe early.
    let mut child = binary()
        .arg("--help")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
}

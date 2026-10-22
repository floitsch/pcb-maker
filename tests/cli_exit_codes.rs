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
    command
        .env("RAYON_NUM_THREADS", "2")
        .env_remove("PCB_MAKER_TEST_PANIC")
        .env_remove("PCB_WORK_RATE");
    command
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-cli-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// A project with the three-pad test board as `t.kicad_pcb`.
fn project(root: &Path) -> PathBuf {
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/pcb-kicad/tests/fixtures/plated-branch.kicad_pcb");
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
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(report).unwrap()).unwrap();
    report["verdict"].clone()
}

#[test]
fn help_lists_the_agent_commands_and_the_exit_codes() {
    let output = binary().arg("--help").output().unwrap();
    assert_eq!(code(&output), 0);
    let text = String::from_utf8_lossy(&output.stdout);
    for needle in [
        "route-kicad-board",
        "layout-kicad-board",
        "place-kicad-board",
        "Exit codes",
        "  3  incomplete and cut short",
    ] {
        assert!(text.contains(needle), "{needle}");
    }
    for command in [
        "route-kicad-board",
        "layout-kicad-board",
        "place-kicad-board",
        "import-kicad-netlist",
    ] {
        let output = binary().args([command, "--help"]).output().unwrap();
        assert_eq!(code(&output), 0, "{command}");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("usage: pcb-maker {command} ")),
            "{command}"
        );
    }
    assert_eq!(
        code(
            &binary()
                .args(["help", "verify-kicad-rung"])
                .output()
                .unwrap()
        ),
        0
    );
}

#[test]
fn version_prints_the_commit_and_the_build_date() {
    let output = binary().arg("--version").output().unwrap();
    assert_eq!(code(&output), 0);
    let text = String::from_utf8(output.stdout).unwrap();
    let words: Vec<&str> = text.split_whitespace().collect();
    assert_eq!(words.len(), 3, "{text:?}");
    assert_eq!(words[0], "pcb-maker");
    // The date is YYYY-MM-DD, the commit HEAD's (or "unknown" without git).
    assert!(
        words[2].len() == 10 && words[2].chars().filter(|c| *c == '-').count() == 2,
        "{text:?}"
    );
    let head = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
    match head {
        Some(head) => assert_eq!(words[1], head),
        None => assert_eq!(words[1], "unknown"),
    }
    assert_eq!(code(&binary().arg("-V").output().unwrap()), 0);
}

#[test]
fn usage_and_input_errors_exit_2_with_one_line() {
    let root = scratch("usage");
    let project = project(&root);
    let cases: Vec<Vec<String>> = vec![
        vec!["no-such-command".into()],
        vec!["route-kicad-board".into()],
        vec![
            "route-kicad-board".into(),
            root.join("missing").display().to_string(),
            "t".into(),
            root.join("out").display().to_string(),
        ],
        vec![
            "route-kicad-board".into(),
            project.display().to_string(),
            "nope".into(),
            root.join("out").display().to_string(),
        ],
        // The output directory exists.
        vec![
            "route-kicad-board".into(),
            project.display().to_string(),
            "t".into(),
            project.display().to_string(),
        ],
        vec![
            "layout-kicad-board".into(),
            project.display().to_string(),
            "t".into(),
            root.join("out").display().to_string(),
            "--bogus".into(),
        ],
        vec!["describe-kicad-board".into(), project.display().to_string()],
        // A board to take footprints from that is not there.
        vec![
            "import-kicad-netlist".into(),
            project.join("t.kicad_pcb").display().to_string(),
            root.join("import").display().to_string(),
            "t".into(),
            "--layers".into(),
            "2".into(),
            "--footprints-from".into(),
            root.join("missing.kicad_pcb").display().to_string(),
        ],
    ];
    for arguments in cases {
        let output = binary().args(&arguments).output().unwrap();
        assert_eq!(code(&output), 2, "{arguments:?}: {}", stderr(&output));
        assert_eq!(
            stderr(&output).lines().count(),
            1,
            "{arguments:?}: {}",
            stderr(&output)
        );
    }
    // An unreadable router config: nothing is written.
    fs::write(root.join("router.json"), "{not json").unwrap();
    let out = root.join("out");
    let output = binary()
        .args([
            "route-kicad-board",
            &project.display().to_string(),
            "t",
            &out.display().to_string(),
            &root.join("router.json").display().to_string(),
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(!out.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_route_kicad_did_not_check_is_never_complete() {
    let root = scratch("route");
    let project = project(&root);
    let out = root.join("out");
    let output = binary()
        .env("PCB_SKIP_NATIVE", "1")
        .env("PCB_ROUTER_SEED", "0")
        .args([
            "route-kicad-board",
            &project.display().to_string(),
            "t",
            &out.display().to_string(),
            "auto",
        ])
        .output()
        .unwrap();
    // The router's count finds every connection made, but without KiCad's
    // check that is no pass: exit 1, router_complete_unverified.
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    let verdict = verdict(&out.join("board-router.json"));
    assert_eq!(verdict["status"], "router_complete_unverified");
    assert_eq!(verdict["exit_code"], 1);
    assert_eq!(verdict["kicad_checked"], false);
    assert_eq!(verdict["kicad_unconnected_items"], serde_json::Value::Null);
    assert_eq!(verdict["clean_without_subtraction"], false);
    assert!(!verdict["reason"].as_str().unwrap().starts_with("complete"));
    let file: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(file, verdict);
    assert_eq!(verdict["open_connections"], 0);
    assert_eq!(verdict["cut"], serde_json::json!([]));
    assert!(verdict["work_seconds"].as_f64().is_some());
    // The rates were measured at the start, and recorded.
    assert_eq!(verdict["work_rate_source"], "measured");
    assert!(verdict["work_rate"].as_f64().is_some_and(|rate| rate > 0.0));
    assert_eq!(verdict["work_rate"], verdict["work_rate_measured"]);
    assert_eq!(
        verdict["placer_work_rate"],
        verdict["placer_work_rate_measured"]
    );
    assert!(verdict["work_wall_ratio"].as_f64().is_some());
    assert!(
        stderr(&output).contains("work rate: "),
        "{}",
        stderr(&output)
    );
    assert!(
        verdict["outputs"]["board"]
            .as_str()
            .is_some_and(|path| Path::new(path).is_absolute() && Path::new(path).exists())
    );
    // The switches that were set, and what the run repaired.
    assert_eq!(verdict["environment"]["PCB_SKIP_NATIVE"], "1");
    assert_eq!(verdict["environment"]["PCB_ROUTER_SEED"], "0");
    // The build that judged it, as `--version` prints it.
    let version = String::from_utf8(binary().arg("--version").output().unwrap().stdout).unwrap();
    assert_eq!(
        verdict["environment"]["pcb_maker_version"]
            .as_str()
            .map(|v| format!("pcb-maker {v}\n")),
        Some(version)
    );
    assert_eq!(verdict["repairs"]["neck_tracks"], 0);
    assert!(verdict["repairs"]["pour_mode"].is_string());
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("board-router.json")).unwrap()).unwrap();
    assert_eq!(report["repairs"], verdict["repairs"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_pinned_work_rate_is_applied_and_recorded() {
    let root = scratch("rate");
    let project = project(&root);
    let route = |out: &Path, rate: Option<&str>, config: &str| {
        let mut command = binary();
        command.env("PCB_SKIP_NATIVE", "1");
        if let Some(rate) = rate {
            command.env("PCB_WORK_RATE", rate);
        }
        command
            .args([
                "route-kicad-board",
                &project.display().to_string(),
                "t",
                &out.display().to_string(),
                config,
            ])
            .output()
            .unwrap()
    };
    // The variable: both rates as given, the measurement beside them.
    let out = root.join("variable");
    let output = route(&out, Some("3.15e6,150e6"), "auto");
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    let judged = verdict(&out.join("board-router.json"));
    assert_eq!(judged["work_rate"], 3.15e6);
    assert_eq!(judged["placer_work_rate"], 150.0e6);
    assert_eq!(judged["work_rate_source"], "PCB_WORK_RATE");
    assert!(
        judged["work_rate_measured"]
            .as_f64()
            .is_some_and(|rate| rate > 0.0)
    );
    assert_eq!(judged["environment"]["PCB_WORK_RATE"], "3.15e6,150e6");
    // The router settings, the placer's rate scaled as the search's.
    let config = root.join("router.json");
    fs::write(
        &config,
        r#"{"work_rate": 1.575e6, "skip_native_verification": true}"#,
    )
    .unwrap();
    let out = root.join("config");
    let output = route(&out, None, &config.display().to_string());
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    let judged = verdict(&out.join("board-router.json"));
    assert_eq!(judged["work_rate"], 1.575e6);
    assert_eq!(judged["placer_work_rate"], 75.0e6);
    assert_eq!(judged["work_rate_source"], "config");
    // A rate that is no rate is the input's error.
    let out = root.join("bad");
    let output = route(&out, Some("fast"), "auto");
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("PCB_WORK_RATE"),
        "{}",
        stderr(&output)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn with_kicads_check_the_exit_code_is_the_verdicts() {
    if !Command::new("kicad-cli")
        .arg("version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        eprintln!("skipping: kicad-cli is unavailable");
        return;
    }
    let root = scratch("native");
    let project = project(&root);
    let out = root.join("out");
    let output = binary()
        .args([
            "route-kicad-board",
            &project.display().to_string(),
            "t",
            &out.display().to_string(),
            "auto",
        ])
        .output()
        .unwrap();
    let verdict = verdict(&out.join("board-router.json"));
    assert_eq!(
        verdict["exit_code"].as_i64(),
        Some(code(&output) as i64),
        "{}",
        stderr(&output)
    );
    assert_eq!(verdict["kicad_unconnected_items"], 0);
    assert_eq!(verdict["kicad_checked"], true);
    assert!(verdict["new_findings_strict"].is_object());
    assert!(verdict["clean_without_subtraction"].is_boolean());
    assert!(verdict["outputs"]["drc"].as_str().is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_panic_exits_4_with_one_line_and_a_report() {
    let root = scratch("panic");
    let project = project(&root);
    let out = root.join("out");
    // KiCad's check off: the run starts without kicad-cli too.
    let output = binary()
        .env("PCB_MAKER_TEST_PANIC", "1")
        .env("PCB_SKIP_NATIVE", "1")
        .env_remove("RUST_BACKTRACE")
        .args([
            "route-kicad-board",
            &project.display().to_string(),
            "t",
            &out.display().to_string(),
            "auto",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 4);
    // One line besides the work rates' (measured before the run starts).
    let text = stderr(&output);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.starts_with("work rate: "))
        .collect();
    assert_eq!(lines.len(), 1, "{text}");
    assert!(text.contains("PCB_MAKER_TEST_PANIC"), "{text}");
    assert!(!out.join("board-router.json").exists());
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "internal_error");
    assert_eq!(verdict["exit_code"], 4);
    assert!(
        verdict["error"]
            .as_str()
            .is_some_and(|error| error.contains("panic"))
    );
    fs::remove_dir_all(root).unwrap();
}

/// The ladder's fault isolation (`PCB_MAKER_TEST_PANIC_ATTEMPT`): a panic
/// in the first rung is recorded and the finer pitch's rung gives the
/// board; the run is no internal error.
#[test]
fn a_panic_in_one_attempt_keeps_the_ladder_going() {
    let root = scratch("attempt-panic");
    let project = project(&root);
    let out = root.join("out");
    let output = binary()
        .env("PCB_SKIP_NATIVE", "1")
        .env("PCB_ROUTER_SEED", "0")
        .env("PCB_MAKER_TEST_PANIC_ATTEMPT", "rung")
        .env_remove("RUST_BACKTRACE")
        .args([
            "route-kicad-board",
            &project.display().to_string(),
            "t",
            &out.display().to_string(),
            "auto",
        ])
        .output()
        .unwrap();
    let text = stderr(&output);
    assert_eq!(code(&output), 1, "{text}");
    assert!(
        text.contains("failed (panic: PCB_MAKER_TEST_PANIC_ATTEMPT"),
        "{text}"
    );
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("board-router.json")).unwrap()).unwrap();
    let failed = report["attempts_failed"]
        .as_array()
        .expect("attempts_failed");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(
        failed[0]
            .as_str()
            .is_some_and(|failure| failure.starts_with("the rung pours=")
                && failure.contains("pitch=regular: panic: PCB_MAKER_TEST_PANIC_ATTEMPT")),
        "{failed:?}"
    );
    // The board is the finer lattice's.
    assert!(report["grid_pitch_mm"].as_f64().unwrap() < 0.1);
    let verdict = &report["verdict"];
    assert_eq!(verdict["status"], "router_complete_unverified");
    assert!(
        verdict["next_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().is_some_and(
                |action| action.starts_with("1 attempts of the router's ladder failed")
            )),
        "{verdict}"
    );
    assert_eq!(
        verdict["environment"]["PCB_MAKER_TEST_PANIC_ATTEMPT"],
        "rung"
    );
    fs::remove_dir_all(root).unwrap();
}

/// Every attempt panicking leaves no board: exit 4, the failures named.
#[test]
fn every_attempt_panicking_exits_4() {
    let root = scratch("attempts-panic");
    let project = project(&root);
    let out = root.join("out");
    let output = binary()
        .env("PCB_SKIP_NATIVE", "1")
        .env("PCB_MAKER_TEST_PANIC_ATTEMPT", "rung:all")
        .env_remove("RUST_BACKTRACE")
        .args([
            "route-kicad-board",
            &project.display().to_string(),
            "t",
            &out.display().to_string(),
            "auto",
        ])
        .output()
        .unwrap();
    let text = stderr(&output);
    assert_eq!(code(&output), 4, "{text}");
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "internal_error");
    assert!(
        verdict["error"].as_str().is_some_and(
            |error| error.contains("no attempt of the ladder produced a board: 3 failed")
        ),
        "{verdict}"
    );
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

/// A fake kicad-cli on the PATH that writes `report` for any DRC and
/// counts its DRC runs in `<directory>/runs`.
#[cfg(unix)]
fn fake_kicad(directory: &Path, report: &serde_json::Value, exit: u8) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = directory.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let program = bin.join("kicad-cli");
    let runs = directory.join("runs");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\nif [ \"$1\" = version ]; then echo 10.0.7; exit 0; fi\necho run >> {}\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = --output ]; then\n    shift\n    cat > \"$1\" <<'REPORT'\n{report}\nREPORT\n    exit {exit}\n  fi\n  shift\ndone\nexit 99\n",
            runs.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    bin
}

#[test]
#[cfg(unix)]
fn drc_exits_by_the_findings_that_count_and_caches_the_report() {
    use serde_json::json;
    let root = scratch("drc");
    let project = project(&root);
    let finding = |kind: &str, severity: &str| json!({"type": kind, "description": kind, "severity": severity, "items": []});
    let report = |violations: serde_json::Value, unconnected: serde_json::Value| {
        json!({"$schema": "https://schemas.kicad.org/drc.v1.json", "source": "t.kicad_pcb", "date": "2026-10-14T12:00:00",
               "kicad_version": "10.0.7", "coordinate_units": "mm", "included_severities": ["error", "warning", "exclusion"],
               "violations": violations, "unconnected_items": unconnected, "schematic_parity": []})
    };
    let cases = [
        ("clean", report(json!([]), json!([])), 0, 0),
        (
            "warnings and cosmetic errors only",
            report(
                json!([
                    finding("clearance", "warning"),
                    finding("silk_overlap", "error")
                ]),
                json!([]),
            ),
            0,
            0,
        ),
        (
            "an error",
            report(json!([finding("clearance", "error")]), json!([])),
            0,
            1,
        ),
        (
            "an unconnected item",
            report(json!([]), json!([finding("unconnected_items", "error")])),
            0,
            1,
        ),
        ("kicad-cli fails", report(json!([]), json!([])), 1, 4),
    ];
    for (index, (name, contents, fake_exit, expected)) in cases.into_iter().enumerate() {
        let case = root.join(format!("case-{index}"));
        let bin = fake_kicad(&case, &contents, fake_exit);
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = case.join("reports/drc.json");
        let run = || {
            binary()
                .env("PATH", &path)
                .env("PCB_MAKER_CACHE_DIR", case.join("cache"))
                .env_remove("PCB_MAKER_NO_CACHE")
                .args([
                    "drc-kicad-board",
                    &project.display().to_string(),
                    "t",
                    &out.display().to_string(),
                ])
                .output()
                .unwrap()
        };
        let output = run();
        assert_eq!(code(&output), expected, "{name}: {}", stderr(&output));
        if expected == 4 {
            assert_eq!(
                stderr(&output).lines().count(),
                1,
                "{name}: {}",
                stderr(&output)
            );
            assert!(!out.exists(), "{name}");
            continue;
        }
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(written, contents, "{name}");
        // The second check of the same board comes from the cache.
        let again = run();
        assert_eq!(code(&again), expected, "{name}");
        assert!(
            String::from_utf8_lossy(&again.stdout).contains("drc_cache: {calls 1, hits 1}"),
            "{name}"
        );
        assert_eq!(
            fs::read_to_string(case.join("runs"))
                .unwrap()
                .lines()
                .count(),
            1,
            "{name}"
        );
        assert!(
            !out.with_extension("project").exists(),
            "{name}: the scratch copy is gone"
        );
    }
    // Usage and input errors.
    let missing = root.join("missing").display().to_string();
    for arguments in [
        vec!["drc-kicad-board".to_string()],
        vec![
            "drc-kicad-board".into(),
            project.display().to_string(),
            "t".into(),
        ],
        vec![
            "drc-kicad-board".into(),
            missing,
            "t".into(),
            "r.json".into(),
        ],
        vec![
            "drc-kicad-board".into(),
            project.display().to_string(),
            "nope".into(),
            "r.json".into(),
        ],
        vec![
            "drc-kicad-board".into(),
            project.display().to_string(),
            "t".into(),
            "r.json".into(),
            "--bogus".into(),
        ],
    ] {
        let output = binary().args(&arguments).output().unwrap();
        assert_eq!(code(&output), 2, "{arguments:?}: {}", stderr(&output));
        assert_eq!(
            stderr(&output).lines().count(),
            1,
            "{arguments:?}: {}",
            stderr(&output)
        );
    }
    let output = binary()
        .args(["drc-kicad-board", "--help"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0);
    assert!(String::from_utf8_lossy(&output.stdout).contains("usage: pcb-maker drc-kicad-board "));
    fs::remove_dir_all(root).unwrap();
}

/// A DRC report of the test board (`t.kicad_pcb`) with `violations`.
#[cfg(unix)]
fn drc_report(violations: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"$schema": "https://schemas.kicad.org/drc.v1.json", "source": "t.kicad_pcb",
        "date": "2026-10-21T12:00:00", "kicad_version": "10.0.7", "coordinate_units": "mm",
        "included_severities": ["error", "warning", "exclusion"], "violations": violations,
        "unconnected_items": [], "schematic_parity": []})
}

/// A fake kicad-cli on the PATH for the neck repair: every DRC reports
/// `report` until the repair saves its refilled copy (`--save-board`, which
/// gets `filled` as KiCad's fill); every DRC after that fails, as a
/// kicad-cli that crashes or times out.
#[cfg(unix)]
fn crashing_kicad(directory: &Path, report: &serde_json::Value, filled: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = directory.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let program = bin.join("kicad-cli");
    let mark = directory.join("saved");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\nif [ \"$1\" = version ]; then echo 10.0.7; exit 0; fi\n\
             save=no\nfor argument in \"$@\"; do [ \"$argument\" = --save-board ] && save=yes; done\n\
             if [ $save = no ] && [ -e {mark} ]; then echo 'kicad-cli crashed' >&2; exit 3; fi\n\
             while [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = --output ]; then\n    shift\n    cat > \"$1\" <<'REPORT'\n{report}\nREPORT\n    shift\n    [ $save = yes ] && {{ cp {filled} \"$1\"; touch {mark}; }}\n    exit 0\n  fi\n  shift\ndone\nexit 99\n",
            mark = mark.display(),
            filled = filled.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    bin
}

#[test]
#[cfg(unix)]
fn a_kicad_failure_in_the_neck_repair_leaves_the_last_report() {
    use serde_json::json;
    let root = scratch("necks");
    let project = project(&root);
    // A minimum connection width, so that the repair runs.
    fs::write(
        project.join("t.kicad_pro"),
        json!({"board": {"design_settings": {"rules": {"min_connection": 0.127}}}}).to_string(),
    )
    .unwrap();
    // KiCad's fill as the repair's save gets it: two squares of /TARGET on
    // B.Cu joined by a bridge 0.06 mm wide.
    let square = |x0: f64, y0: f64, x1: f64, y1: f64| {
        format!(
            "(filled_polygon (layer \"B.Cu\") (pts (xy {x0} {y0}) (xy {x1} {y0}) (xy {x1} {y1}) (xy {x0} {y1})))"
        )
    };
    let zone = format!(
        "(zone (net \"/TARGET\") (layer \"B.Cu\") (uuid \"6a1c1a5e-0000-4000-8000-000000000001\") (polygon (pts (xy 2 2) (xy 9 2) (xy 9 6) (xy 2 6))) {} {} {})",
        square(3.0, 3.0, 5.0, 5.0),
        square(5.3, 3.0, 7.3, 5.0),
        square(4.95, 3.97, 5.35, 4.03)
    );
    let board = fs::read_to_string(project.join("t.kicad_pcb")).unwrap();
    let filled = root.join("filled.kicad_pcb");
    fs::write(
        &filled,
        format!(
            "{}\n  {zone})\n",
            board.trim_end().strip_suffix(')').unwrap()
        ),
    )
    .unwrap();
    let neck = json!({"type": "connection_width", "severity": "error",
        "description": "Copper connection too narrow (board setup constraints minimum connection width 0.1270 mm; actual 0.0600 mm) (B.Cu)",
        "items": [{"description": "Zone [/TARGET] on B.Cu", "pos": {"x": 5.1, "y": 4.0}, "uuid": "6a1c1a5e-0000-4000-8000-000000000001"}]});
    let report = drc_report(json!([neck]));
    let bin = crashing_kicad(&root, &report, &filled);
    let out = root.join("out");
    let output = binary()
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("PCB_MAKER_NO_CACHE", "1")
        .env_remove("PCB_SKIP_NATIVE")
        .env_remove("PCB_KICAD_NO_NECK_REPAIR")
        .args([
            "route-kicad-board",
            &project.display().to_string(),
            "t",
            &out.display().to_string(),
            "auto",
        ])
        .output()
        .unwrap();
    let text = stderr(&output);
    assert!(root.join("saved").exists(), "the repair ran: {text}");
    let necks: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("necks.json")).unwrap()).unwrap();
    assert!(necks["found"].as_u64().unwrap() >= 1, "{necks}");
    assert_eq!(necks["kept"], false, "{necks}");
    assert!(
        necks["note"]
            .as_str()
            .is_some_and(|note| note.contains("kicad-cli") && note.contains("as before the repair")),
        "{necks}"
    );
    // The board and KiCad's report on it are the ones before the repair:
    // the verdict rests on KiCad's check, not on the router's count.
    assert!(
        !fs::read_to_string(out.join("t.kicad_pcb"))
            .unwrap()
            .contains("(width 0.127)"),
        "no trial's repair track is left on the board"
    );
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("drc.json")).unwrap()).unwrap();
    assert_eq!(written, report);
    assert_eq!(code(&output), 1, "{text}");
    let verdict = verdict(&out.join("board-router.json"));
    assert_eq!(verdict["status"], "incomplete", "{verdict}");
    assert_eq!(verdict["kicad_checked"], true);
    assert_eq!(verdict["kicad_unconnected_items"], 0);
    assert_eq!(
        verdict["new_findings_strict"],
        json!({"connection_width": 1})
    );
    assert_eq!(verdict["repairs"]["neck_tracks"], 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn a_missing_report_after_kicads_check_is_an_internal_error() {
    let root = scratch("dropped");
    let project = project(&root);
    let report = drc_report(serde_json::json!([]));
    let bin = fake_kicad(&root, &report, 0);
    let out = root.join("out");
    let output = binary()
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("PCB_MAKER_CACHE_DIR", root.join("cache"))
        .env("PCB_MAKER_TEST_DROP_DRC", "1")
        .env_remove("PCB_SKIP_NATIVE")
        .args([
            "route-kicad-board",
            &project.display().to_string(),
            "t",
            &out.display().to_string(),
            "auto",
        ])
        .output()
        .unwrap();
    let text = stderr(&output);
    assert_eq!(code(&output), 4, "{text}");
    assert!(
        text.lines().last().is_some_and(|line| line
            .starts_with("error: KiCad checked the board but its report is missing or unreadable")),
        "{text}"
    );
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "internal_error");
    assert_eq!(verdict["exit_code"], 4);
    assert_eq!(verdict["kicad_checked"], false);
    assert!(
        verdict["error"]
            .as_str()
            .is_some_and(|error| error.contains("drc.json")),
        "{verdict}"
    );
    fs::remove_dir_all(root).unwrap();
}

/// A layout with a free dimension: input errors exit 2 before any layout,
/// and a search ends with the chosen size's layout as the result, the
/// table of every size beside it.
#[test]
fn a_free_dimension_searches_the_sizes_and_keeps_one_layout() {
    let root = scratch("free-dimension");
    let project = project(&root);
    let layout = |name: &str, config: serde_json::Value, extra: &[(&str, &str)]| {
        let path = root.join(format!("{name}.json"));
        fs::write(&path, config.to_string()).unwrap();
        let out = root.join(name);
        // The rates pinned and the wall guard off: two probes side by side
        // under load were cut by the wall clock (exit 3 for 1).
        let mut command = binary();
        command
            .env("PCB_WORK_RATE", "3.15e6,150e6")
            .env("PCB_ROUTER_NO_WALL_GUARD", "1");
        for (key, value) in extra {
            command.env(key, value);
        }
        let output = command
            .args([
                "layout-kicad-board",
                &project.display().to_string(),
                "t",
                &out.display().to_string(),
                "auto",
                &path.display().to_string(),
            ])
            .output()
            .unwrap();
        (output, out)
    };
    // An edge that is none, and sizes where the fixed J3 (at x 18 of a
    // 20 mm board) would end outside: input errors, one line, exit 2.
    // (KiCad's check off: these need no kicad-cli.)
    let skip = [("PCB_SKIP_NATIVE", "1")];
    for (name, config) in [
        (
            "edge",
            serde_json::json!({"free_dimension": {"edge": "middle", "sizes": [20]}}),
        ),
        (
            "stranded",
            serde_json::json!({"placer": {"fixed": ["J3"]},
                               "free_dimension": {"edge": "right", "sizes": [5, 10]}}),
        ),
    ] {
        let (output, _) = layout(name, config, &skip);
        assert_eq!(code(&output), 2, "{name}: {}", stderr(&output));
        assert_eq!(stderr(&output).lines().count(), 1, "{}", stderr(&output));
    }
    let (output, out) = layout(
        "stranded2",
        serde_json::json!({"placer": {"fixed": ["J3"]},
                           "free_dimension": {"edge": "right", "sizes": [5]}}),
        &skip,
    );
    assert!(
        stderr(&output).contains("fixed part J3"),
        "{}",
        stderr(&output)
    );
    // No size feasible: the verdict and the table still say why.
    let file: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(file["status"], "usage_error");
    assert_eq!(file["exit_code"], 2);
    assert!(
        file["next_actions"][0]
            .as_str()
            .unwrap()
            .contains("free-dimension.json lists each size's reasons"),
        "{file}"
    );
    let table: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("free-dimension.json")).unwrap())
            .unwrap();
    assert_eq!(table["runs"][0]["status"], "infeasible", "{table}");
    assert!(
        table["runs"][0]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("J3"),
        "{table}"
    );
    // Without KiCad's check no size is clean: the closest is kept (the
    // smallest of equals), its layout is the result.
    let (output, out) = layout(
        "search",
        serde_json::json!({"total_seconds": 30, "moves": 0,
                           "placer": {"fixed": ["J3"]},
                           "free_dimension": {"edge": "bottom", "sizes": [16, 20, 24],
                                              "probe_seconds": 2, "confirm_seconds": 6,
                                              "parallel": 2}}),
        &[("PCB_SKIP_NATIVE", "1"), ("PCB_ROUTER_SEED", "0")],
    );
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    let verdict = verdict(&out.join("board-layout.json"));
    assert_eq!(verdict["status"], "router_complete_unverified");
    let free = &verdict["free_dimension"];
    assert_eq!(free["chosen"], 16.0, "{free}");
    assert_eq!(free["clean"], false);
    assert!(
        free["reason"]
            .as_str()
            .unwrap()
            .starts_with("no size was clean: 16 mm came closest"),
        "{free}"
    );
    assert!(
        verdict["next_actions"][0]
            .as_str()
            .unwrap()
            .starts_with("no size from 16 to 24 mm was clean"),
        "{verdict}"
    );
    // The result is where a layout puts it, at the chosen size.
    let board = verdict["outputs"]["board"].as_str().unwrap();
    assert_eq!(Path::new(board), out.join("result/t.kicad_pcb"));
    let text = fs::read_to_string(board).unwrap();
    assert!(text.contains("(end 20 16)"), "{text}");
    let table: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("free-dimension.json")).unwrap())
            .unwrap();
    let runs = table["runs"].as_array().unwrap();
    // The probes, two side by side, then the closest with the rest.
    let kinds: Vec<(f64, &str, f64)> = runs
        .iter()
        .map(|run| {
            (
                run["size_mm"].as_f64().unwrap(),
                run["kind"].as_str().unwrap(),
                run["budget_seconds"].as_f64().unwrap(),
            )
        })
        .collect();
    // The closest gets all that is left of the 30 s, not confirm_seconds.
    let probes_work: f64 = runs[..3]
        .iter()
        .map(|run| run["work_seconds"].as_f64().unwrap())
        .sum();
    assert_eq!(
        kinds,
        vec![
            (16.0, "probe", 2.0),
            (20.0, "probe", 2.0),
            (24.0, "probe", 2.0),
            (16.0, "closest", 30.0 - probes_work)
        ],
        "{table}"
    );
    assert_eq!(
        runs.iter().filter(|run| run["chosen"] == true).count(),
        1,
        "{table}"
    );
    assert!(
        out.join("free-dimension/20mm-probe/input/t.kicad_pcb")
            .is_file()
    );
    fs::remove_dir_all(root).unwrap();
}

/// A board of three nets of two pads each: /A pre-routed on F.Cu, /B
/// pre-routed through two vias and B.Cu, /C open.
const PREROUTED: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6)) (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 30 20) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
  (footprint "test" (layer "F.Cu") (at 3 5)
    (property "Reference" "J1" (at -2 0) (layer "F.SilkS")
      (effects (font (size 1 1) (thickness 0.15))))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/A"))
    (pad "2" smd rect (at 0 5) (size 1 1) (layers "F.Cu") (net "/B"))
    (pad "3" smd rect (at 0 10) (size 1 1) (layers "F.Cu") (net "/C")))
  (footprint "test" (layer "F.Cu") (at 27 5)
    (property "Reference" "J2" (at 2 0) (layer "F.SilkS")
      (effects (font (size 1 1) (thickness 0.15))))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/A"))
    (pad "2" smd rect (at 0 5) (size 1 1) (layers "F.Cu") (net "/B"))
    (pad "3" smd rect (at 0 10) (size 1 1) (layers "F.Cu") (net "/C")))
  (segment (start 3 5) (end 27 5) (width 0.25) (layer "F.Cu") (net "/A"))
  (segment (start 3 10) (end 10 10) (width 0.25) (layer "F.Cu") (net "/B"))
  (via (at 10 10) (size 0.7) (drill 0.35) (layers "F.Cu" "B.Cu") (net "/B"))
  (segment (start 10 10) (end 20 10) (width 0.25) (layer "B.Cu") (net "/B"))
  (via (at 20 10) (size 0.7) (drill 0.35) (layers "F.Cu" "B.Cu") (net "/B"))
  (segment (start 20 10) (end 27 10) (width 0.25) (layer "F.Cu") (net "/B")))
"#;

/// The board's top-level tracks and vias, whitespace normalised.
fn copper_items(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut items = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let mut quoted = false;
    for (index, &byte) in bytes.iter().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b'(' if !quoted => {
                depth += 1;
                if depth == 2 {
                    start = index;
                }
            }
            b')' if !quoted => {
                if depth == 2 {
                    let item = text[start..=index]
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .replace("( ", "(")
                        .replace(" )", ")");
                    if item.starts_with("(segment") || item.starts_with("(via") {
                        items.push(item);
                    }
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    items
}

#[test]
fn route_keeps_the_boards_own_tracks_and_routes_what_is_open() {
    let root = scratch("prerouted");
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("t.kicad_pcb"), PREROUTED).unwrap();
    let route = |out: &Path, strip: bool| {
        let mut command = binary();
        command.env("PCB_SKIP_NATIVE", "1").args([
            "route-kicad-board",
            &project.display().to_string(),
            "t",
            &out.display().to_string(),
            "auto",
        ]);
        if strip {
            command.arg("--strip");
        }
        command.output().unwrap()
    };
    let kept = copper_items(PREROUTED);
    assert_eq!(kept.len(), 6);
    // By default the board's own tracks and vias stay as they are, and
    // only /C is routed.
    let out = root.join("keep");
    let output = route(&out, false);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    let judged = verdict(&out.join("board-router.json"));
    assert_eq!(judged["status"], "router_complete_unverified");
    assert_eq!(judged["open_connections"], 0);
    let routed = copper_items(&fs::read_to_string(out.join("t.kicad_pcb")).unwrap());
    for item in &kept {
        assert!(routed.contains(item), "{item} is gone: {routed:#?}");
    }
    let added: Vec<&String> = routed.iter().filter(|item| !kept.contains(item)).collect();
    assert!(!added.is_empty());
    assert!(
        added.iter().all(|item| item.contains("(net \"/C\")")),
        "{added:#?}"
    );
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("board-router.json")).unwrap()).unwrap();
    let copper = &report["kept_copper"];
    assert_eq!(copper["segments"], 4);
    assert_eq!(copper["vias"], 2);
    assert_eq!(copper["connections_made"], 2);
    assert_eq!(copper["nets_complete"], serde_json::json!(["A", "B"]));
    assert_eq!(report["routable_connections"], 1);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("kept: the board's 4 track segments"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    // --strip routes from nothing: the board's own items are gone and
    // every net is the router's.
    let out = root.join("strip");
    let output = route(&out, true);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert_eq!(
        verdict(&out.join("board-router.json"))["open_connections"],
        0
    );
    let routed = copper_items(&fs::read_to_string(out.join("t.kicad_pcb")).unwrap());
    assert!(
        routed.iter().all(|item| !kept.contains(item)),
        "{routed:#?}"
    );
    for net in ["/A", "/B", "/C"] {
        assert!(
            routed
                .iter()
                .any(|item| item.contains(&format!("(net \"{net}\")"))),
            "{net}"
        );
    }
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("board-router.json")).unwrap()).unwrap();
    assert!(report.get("kept_copper").is_none());
    assert_eq!(report["routable_connections"], 3);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_fab_release_is_gated_by_kicads_drc() {
    let root = scratch("fab");
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("t.kicad_pcb"), PREROUTED).unwrap();
    let fab = |out: &Path, extra: &[&str]| {
        binary()
            .args([
                "fab-kicad-board",
                &project.display().to_string(),
                "t",
                &out.display().to_string(),
            ])
            .args(extra)
            .output()
            .unwrap()
    };
    // Usage errors before anything runs.
    let output = fab(&root.join("bad"), &["--profile", "oshpark"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(stderr(&output).contains("unknown --profile"));
    assert_eq!(code(&fab(&project, &[])), 2);
    if !Command::new("kicad-cli")
        .arg("version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        eprintln!("skipping the release: kicad-cli is unavailable");
        fs::remove_dir_all(root).unwrap();
        return;
    }
    // /C is open: refused, nothing exported, the finding named.
    let out = root.join("refused");
    let output = fab(&out, &[]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "incomplete");
    assert_eq!(verdict["kicad_unconnected_items"], 1);
    assert_eq!(verdict["fab_release"]["released"], false);
    assert!(
        verdict["next_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("[/C]")),
        "{verdict}"
    );
    assert!(!out.join("gerbers").exists());
    assert!(out.join("fab-manifest.json").is_file());
    // --allow-findings releases anyway, and says so.
    let out = root.join("allowed");
    let output = fab(
        &out,
        &["--allow-findings", "--profile", "jlcpcb", "--no-step"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "complete");
    assert_eq!(verdict["fab_release"]["findings_allowed"], true);
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("fab-manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["released"], true);
    assert_eq!(manifest["drc"]["unconnected_items"], 1);
    assert_eq!(manifest["zip"], "t-gerbers.zip");
    let kinds: Vec<&str> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["kind"].as_str().unwrap())
        .collect();
    for kind in [
        "gerber",
        "drill",
        "drill_map",
        "placement",
        "bom",
        "netlist",
        "zip",
    ] {
        assert!(kinds.contains(&kind), "{kind}: {kinds:?}");
    }
    for file in manifest["files"].as_array().unwrap() {
        let path = out.join(file["path"].as_str().unwrap());
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            file["bytes"].as_u64().unwrap(),
            "{}",
            path.display()
        );
    }
    let cpl = fs::read_to_string(out.join("assembly/t-cpl-jlcpcb.csv")).unwrap();
    assert!(
        cpl.starts_with("Designator,Mid X,Mid Y,Layer,Rotation\n"),
        "{cpl}"
    );
    assert_eq!(cpl.lines().count(), 3, "{cpl}");
    assert_eq!(manifest["skipped"].as_array().unwrap().len(), 2);
    fs::remove_dir_all(root).unwrap();
}

/// The agent skill (skills/pcb-maker/SKILL.md) cannot go stale: every
/// command it names is one of the binary's agent commands and in
/// docs/agents.md, every agent command is in it, and its exit code table
/// is `--help`'s.
#[test]
fn the_agent_skill_matches_the_command_table_and_the_exit_codes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let skill = fs::read_to_string(root.join("skills/pcb-maker/SKILL.md")).unwrap();
    let guide = fs::read_to_string(root.join("docs/agents.md")).unwrap();
    // Frontmatter: name and description, as the agent CLIs read it.
    let front = skill
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map(|(front, _)| front)
        .expect("frontmatter");
    assert!(front.lines().any(|line| line == "name: pcb-maker"));
    assert!(
        front
            .lines()
            .any(|line| line.starts_with("description: ") && line.len() > 80)
    );
    let help = binary().arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&help.stdout).into_owned();
    let commands: Vec<String> = help
        .split("Commands for agents")
        .nth(1)
        .unwrap()
        .lines()
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect();
    assert!(commands.len() >= 13, "{commands:?}");
    // Command names: words of the form <verb>-kicad-<noun>.
    let named: std::collections::BTreeSet<String> = skill
        .split(|c: char| !(c.is_ascii_lowercase() || c == '-'))
        .filter(|word| word.contains("-kicad-"))
        .map(str::to_string)
        .collect();
    for name in &named {
        assert!(
            commands.contains(name),
            "the skill names {name}, which is no agent command"
        );
        assert!(
            guide.contains(name.as_str()),
            "the skill names {name}, which docs/agents.md does not"
        );
    }
    for command in &commands {
        assert!(named.contains(command), "the skill leaves out {command}");
    }
    // The exit code table: each code with its status, as --help says.
    for (exit, status, helped) in [
        (0, "`complete`", "complete"),
        (1, "`incomplete`", "incomplete"),
        (2, "`usage_error`", "usage or input error"),
        (3, "`cut`", "incomplete and cut short"),
        (4, "`internal_error`", "internal error"),
    ] {
        let row = skill
            .lines()
            .find(|line| line.starts_with(&format!("| {exit} | ")))
            .unwrap_or_else(|| panic!("no row for exit {exit}"));
        assert!(row.contains(status), "{row}");
        assert!(
            help.lines()
                .any(|line| line.starts_with(&format!("  {exit}  {helped}"))),
            "--help has no exit {exit} {helped}"
        );
        assert!(
            guide.contains(&format!("| {exit} | {status} |")),
            "docs/agents.md has no exit {exit} {status}"
        );
    }
    assert!(
        !skill
            .lines()
            .any(|line| line.starts_with("| 5 ") || line.starts_with("| 6 "))
    );
}

/// A kicad-cli that answers `version` with `version` and fails anything
/// else.
#[cfg(unix)]
fn versioned_kicad(directory: &Path, version: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = directory.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let program = bin.join("kicad-cli");
    fs::write(
        &program,
        format!("#!/bin/sh\nif [ \"$1\" = version ]; then echo {version}; exit 0; fi\nexit 1\n"),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    program
}

#[test]
#[cfg(unix)]
fn kicad_cli_is_checked_before_any_work() {
    let root = scratch("kicad-check");
    let project = project(&root);
    let missing = root.join("no-kicad-cli");
    let run = |program: &Path, skip: bool, arguments: &[String]| {
        let mut command = binary();
        command
            .env("PCB_KICAD_CLI", program)
            .env("PCB_ROUTER_SEED", "0")
            .args(arguments);
        if skip {
            command.env("PCB_SKIP_NATIVE", "1");
        } else {
            command.env_remove("PCB_SKIP_NATIVE");
        }
        command.output().unwrap()
    };
    let out = root.join("out");
    let route = vec![
        "route-kicad-board".to_string(),
        project.display().to_string(),
        "t".into(),
        out.display().to_string(),
        "auto".into(),
    ];
    // No kicad-cli: an input error before any work, its verdict naming
    // the requirement.
    let output = run(&missing, false, &route);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert_eq!(stderr(&output).lines().count(), 1, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("PCB_KICAD_CLI"),
        "{}",
        stderr(&output)
    );
    let file: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(file["status"], "usage_error");
    let action = file["next_actions"][0].as_str().unwrap();
    assert!(
        action.contains("KiCad 10") && action.contains("PATH"),
        "{action}"
    );
    // With KiCad's check off the route goes on (into the same directory,
    // which holds only that verdict), with a warning in its verdict.
    let output = run(&missing, true, &route);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    let verdict = verdict(&out.join("board-router.json"));
    assert_eq!(verdict["status"], "router_complete_unverified");
    assert!(
        verdict["next_actions"][0]
            .as_str()
            .unwrap()
            .contains("goes on without KiCad"),
        "{verdict}"
    );
    // A KiCad older than 10: an input error too.
    let old = versioned_kicad(&root.join("old"), "9.0.1");
    let output = run(
        &old,
        false,
        &[
            "layout-kicad-board".to_string(),
            project.display().to_string(),
            "t".into(),
            root.join("layout").display().to_string(),
        ],
    );
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("kicad-cli 9.0.1 is older than KiCad 10"),
        "{}",
        stderr(&output)
    );
    // The DRC and a schematic's import need it.
    let output = run(
        &missing,
        false,
        &[
            "drc-kicad-board".to_string(),
            project.display().to_string(),
            "t".into(),
            root.join("drc.json").display().to_string(),
        ],
    );
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    let schematic = root.join("t.kicad_sch");
    fs::write(&schematic, "(kicad_sch)").unwrap();
    let output = run(
        &missing,
        false,
        &[
            "import-kicad-netlist".to_string(),
            schematic.display().to_string(),
            root.join("import").display().to_string(),
            "t".into(),
        ],
    );
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    // A schematic KiCad cannot read is the input's fault: exit 2.
    let failing = versioned_kicad(&root.join("failing"), "10.0.7");
    let output = run(
        &failing,
        false,
        &[
            "import-kicad-netlist".to_string(),
            schematic.display().to_string(),
            root.join("import2").display().to_string(),
            "t".into(),
        ],
    );
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("sch export netlist failed"),
        "{}",
        stderr(&output)
    );
    fs::remove_dir_all(root).unwrap();
}

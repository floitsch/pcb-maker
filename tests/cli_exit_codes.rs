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
        .env_remove("PCB_MAKER_TEST_PANIC");
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
    assert!(
        verdict["outputs"]["board"]
            .as_str()
            .is_some_and(|path| Path::new(path).is_absolute() && Path::new(path).exists())
    );
    // The switches that were set, and what the run repaired.
    assert_eq!(verdict["environment"]["PCB_SKIP_NATIVE"], "1");
    assert_eq!(verdict["environment"]["PCB_ROUTER_SEED"], "0");
    assert_eq!(verdict["repairs"]["neck_tracks"], 0);
    assert!(verdict["repairs"]["pour_mode"].is_string());
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("board-router.json")).unwrap()).unwrap();
    assert_eq!(report["repairs"], verdict["repairs"]);
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
    let output = binary()
        .env("PCB_MAKER_TEST_PANIC", "1")
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
    let text = stderr(&output);
    assert_eq!(text.lines().count(), 1, "{text}");
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

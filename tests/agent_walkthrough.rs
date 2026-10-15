// Copyright (C) 2026 Toit contributors.

//! The flow docs/agents.md documents, end to end, as an agent runs it on a
//! new design: a KiCad project with a schematic (urchin, a split keyboard,
//! MIT; `crates/pcb-kicad/tests/fixtures/urchin`), imported with
//! `import-kicad-netlist`, constraints written (outline, rows of keys,
//! regions, `near` to a key of a row, `relative`, `edge`), checked with
//! `place-kicad-board` (a region too small first), laid out with
//! `layout-kicad-board` on a short work budget (`total_seconds`), and
//! judged by `drc-kicad-board`. It asserts the exit codes and the verdict
//! fields an agent reads, so that the walkthrough of 2026-10-15
//! (docs/handover.md) cannot rot.
//!
//! Needs `kicad-cli` (KiCad 9 or newer) on the PATH; without it the test
//! says so and passes. About 30 s.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pcb-maker"));
    // The work, not the machine's load, ends the run: a loaded test
    // machine must not turn exit 0 into exit 3.
    command
        .env("RAYON_NUM_THREADS", "2")
        .env("PCB_ROUTER_NO_WALL_GUARD", "1")
        .env_remove("PCB_SKIP_NATIVE")
        .env_remove("PCB_MAKER_TEST_PANIC");
    command
}

fn scratch() -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-walkthrough-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            copy_tree(&path, &to.join(entry.file_name()));
        } else {
            fs::copy(&path, to.join(entry.file_name())).unwrap();
        }
    }
}

fn run(arguments: &[&Path]) -> Output {
    let output = binary().args(arguments).output().unwrap();
    eprintln!(
        "$ pcb-maker {}\nexit {:?}\n{}",
        arguments.iter().map(|argument| argument.display().to_string()).collect::<Vec<_>>().join(" "),
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).lines().rev().take(5).collect::<Vec<_>>().join("\n")
    );
    output
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("exited")
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))).unwrap()
}

/// What an agent writes for this keyboard: both halves on one 230 x 95 mm
/// board, each half's 15 keys a 5 x 3 grid at the Choc pitch, its thumb
/// keys below the grid, every diode at its key, the controller's display
/// and reset button at the controller, the power switch on the top edge.
fn constraints(width: f64, height: f64) -> Value {
    let mut rows = Vec::new();
    let mut regions = Vec::new();
    let mut near = Vec::new();
    let mut relative = Vec::new();
    let mut edge = Vec::new();
    for (first, mcu, display, reset, power, battery, x) in [
        (1, "U1", "Display1", "RSW0", "SW_POWER0", ["Bat+0", "BatGND0"], [0.0, width / 2.0 - 3.0]),
        (18, "U2", "Display2", "RSW1", "SW_POWER1", ["Bat+1", "BatGND1"], [width / 2.0 + 3.0, width]),
    ] {
        let keys: Vec<String> = (first..first + 15).map(|n| format!("SW{n}")).collect();
        rows.push(json!({"parts": keys, "pitch_mm": 18, "columns": 5, "row_pitch_mm": 17}));
        let mut half = vec![keys[0].clone(), format!("SW{}", first + 15), format!("SW{}", first + 16)];
        half.extend((first..first + 17).map(|n| format!("D{n}")));
        half.extend([mcu, display, reset, power].map(String::from));
        half.extend(battery.map(String::from));
        regions.push(json!({"parts": half, "x": x, "y": [0, height]}));
        for n in first..first + 17 {
            // The keys of the grid are carried by the row: near may still
            // name them (docs/constraints.md, row).
            near.push(json!({"part": format!("D{n}"), "part_of": format!("SW{n}"), "max_mm": 3}));
        }
        near.push(json!({"part": display, "part_of": mcu, "max_mm": 3}));
        near.push(json!({"part": reset, "part_of": mcu, "max_mm": 5}));
        for part in battery {
            near.push(json!({"part": part, "part_of": power, "max_mm": 5}));
        }
        // Below the row's first key is below the whole grid.
        relative.push(json!({"part": format!("SW{}", first + 15), "below": keys[0], "max_gap_mm": 60}));
        relative.push(json!({"part": format!("SW{}", first + 16), "right_of": format!("SW{}", first + 15), "max_gap_mm": 3}));
        edge.push(json!({"part": power, "edge": "top", "max_mm": 1}));
    }
    json!({"version": 1, "move_all": true, "outline": {"width": width, "height": height},
           "row": rows, "region": regions, "near": near, "relative": relative, "edge": edge})
}

#[test]
fn the_documented_flow_lays_out_a_keyboard_from_its_schematic() {
    if Command::new("kicad-cli").arg("version").output().is_err() {
        eprintln!("kicad-cli is not on the PATH: the walkthrough needs KiCad; skipped");
        return;
    }
    let root = scratch();
    let design = root.join("design");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/pcb-kicad/tests/fixtures/urchin"), &design);
    let project = root.join("project");

    // 0. From the schematic: footprints from the project's library, the
    // starter constraints.json and layout.json next to the board.
    let output = run(&[Path::new("import-kicad-netlist"), &design.join("main.kicad_sch"), &project, Path::new("main")]);
    assert_eq!(code(&output), 0, "import");
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["footprints"], 80);
    assert_eq!(report["missing"], json!([]));
    // The controller's three pins on no net have no pad: listed, harmless.
    assert_eq!(report["unmatched_pins"].as_array().unwrap().len(), 2);
    for file in ["main.kicad_pcb", "main.kicad_sch", "main.kicad_pro", "constraints.json", "layout.json"] {
        assert!(project.join(file).exists(), "{file}");
    }
    assert_eq!(read(&project.join("constraints.json"))["move_all"], true);
    // The library's KiCad 6 texts became properties: no footprint keeps
    // the library's reference (KiCad read it over the import's).
    assert!(!fs::read_to_string(project.join("main.kicad_pcb")).unwrap().contains("fp_text reference"));

    // 1. Look at the board: no outline of its own yet.
    let output = run(&[Path::new("describe-kicad-board"), &project, Path::new("main")]);
    assert_eq!(code(&output), 0, "describe");
    let board: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(board["footprints"].as_array().unwrap().len(), 80);
    assert!(board["outline"].is_null() || board["outline"]["from_footprints"] == true, "{}", board["outline"]);

    // 2. Say what you want. A region too small for its part is said at
    // once (exit 2), not after every placement level was tried.
    let mut tight = constraints(230.0, 95.0);
    tight["region"].as_array_mut().unwrap().push(json!({"parts": ["Display1"], "x": [0, 3], "y": [0, 3]}));
    fs::write(project.join("constraints.json"), tight.to_string()).unwrap();
    let output = run(&[Path::new("place-kicad-board"), &project, Path::new("main"), &root.join("tight"), &project.join("layout.json")]);
    assert_eq!(code(&output), 2, "a region too small");
    assert!(String::from_utf8_lossy(&output.stderr).contains("region for Display1 cannot hold it"));

    // Room enough: the placement keeps every constraint, in seconds.
    fs::write(project.join("constraints.json"), constraints(230.0, 95.0).to_string()).unwrap();
    let placed = root.join("placed");
    let output = run(&[Path::new("place-kicad-board"), &project, Path::new("main"), &placed, &project.join("layout.json")]);
    assert_eq!(code(&output), 0, "placement");
    let placement = read(&placed.join("board-placer.json"));
    let statuses = placement["constraints"].as_array().unwrap();
    assert!(statuses.iter().all(|status| status["satisfied"] == true), "{statuses:?}");
    // A diode near a key the row carries is reported against that key.
    assert!(statuses.iter().any(|status| status["part"] == "D2" && status["other"] == "SW2"), "{statuses:?}");
    assert!(stdout_has_no_overlap_warnings(&output));

    // 3. Run, on a short work budget (the default is 1500 s).
    fs::write(project.join("layout.json"), json!({"placer": {"constraints": "constraints.json"}, "total_seconds": 30}).to_string()).unwrap();
    let out = root.join("layout");
    let output = run(&[Path::new("layout-kicad-board"), &project, Path::new("main"), &out, Path::new("auto"), &project.join("layout.json")]);
    assert_eq!(code(&output), 0, "layout");

    // 4. Read the verdict.
    let verdict = read(&out.join("verdict.json"));
    assert_eq!(verdict["status"], "complete", "{verdict}");
    assert_eq!(verdict["exit_code"], 0);
    assert_eq!(verdict["command"], "layout-kicad-board");
    assert_eq!(verdict["kicad_unconnected_items"], 0);
    assert_eq!(verdict["open_connections"], 0);
    assert_eq!(verdict["new_findings"], json!({}));
    assert_eq!(verdict["constraints_missed"], json!([]));
    assert_eq!(verdict["unplaced"], json!([]));
    assert_eq!(verdict["cut"], json!([]));
    assert_eq!(verdict["next_actions"], json!([]));
    assert!(verdict["skipped"].is_array());
    assert!(verdict["work_seconds"].as_f64().unwrap() > 0.0);
    assert!(verdict["drc_cache"]["calls"].as_u64().unwrap() >= 1);
    // The no-net pins above are KiCad's parity findings, nothing more
    // (181 when the library's references won).
    assert!(verdict["schematic_parity_issues"].as_u64().unwrap() <= 6, "{verdict}");
    for output in ["project", "board", "drc", "report", "verdict"] {
        let path = verdict["outputs"][output].as_str().unwrap_or_else(|| panic!("outputs.{output}"));
        assert!(Path::new(path).exists(), "{output}: {path}");
    }
    let layout = read(&out.join("board-layout.json"));
    assert_eq!(layout["verdict"], verdict);
    assert_eq!(layout["routed"]["routed_connections"], layout["routed"]["routable_connections"]);
    // The row the placer turned is judged where it lies (it was reported
    // as a region missed by the row's length).
    assert!(layout["constraints"].as_array().unwrap().iter().all(|status| status["satisfied"] == true), "{}", layout["constraints"]);

    // 5. KiCad's own DRC on the result passes.
    let output = run(&[Path::new("drc-kicad-board"), &out.join("result"), Path::new("main"), &root.join("drc.json")]);
    assert_eq!(code(&output), 0, "drc-kicad-board: {}", String::from_utf8_lossy(&output.stdout));
    assert!(String::from_utf8_lossy(&output.stdout).contains("0 unconnected items"));

    fs::remove_dir_all(&root).ok();
}

/// The placer warned that every part of a fresh import overlapped every
/// other, as fixed parts, before `move_all` freed them.
fn stdout_has_no_overlap_warnings(output: &Output) -> bool {
    let all = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    !String::from_utf8_lossy(&all).contains("overlap")
}

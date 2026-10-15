// Copyright (C) 2026 Toit contributors.

//! The flow docs/agents.md documents, end to end, as an agent runs it on a
//! new design: a KiCad project with a schematic (urchin, a split keyboard,
//! MIT; `crates/pcb-kicad/tests/fixtures/urchin`), imported with
//! `import-kicad-netlist`, constraints written (outline, rows of keys,
//! regions, `near` to a key of a row, `relative`, `edge`), checked with
//! `place-kicad-board` (a region too small first, then a board without
//! room: a fast no with the room as numbers), laid out with
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
    time::{Instant, SystemTime, UNIX_EPOCH},
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
    keyboard(json!({"width": width, "height": height}), width, [[0.0, height], [0.0, height]])
}

/// The keyboard's constraints on any `outline` `width` wide, each half's
/// region spanning `heights` (left, right).
fn keyboard(outline: Value, width: f64, heights: [[f64; 2]; 2]) -> Value {
    let mut rows = Vec::new();
    let mut regions = Vec::new();
    let mut near = Vec::new();
    let mut relative = Vec::new();
    let mut edge = Vec::new();
    for (first, mcu, display, reset, power, battery, x, y) in [
        (1, "U1", "Display1", "RSW0", "SW_POWER0", ["Bat+0", "BatGND0"], [0.0, width / 2.0 - 3.0], heights[0]),
        (18, "U2", "Display2", "RSW1", "SW_POWER1", ["Bat+1", "BatGND1"], [width / 2.0 + 3.0, width], heights[1]),
    ] {
        let keys: Vec<String> = (first..first + 15).map(|n| format!("SW{n}")).collect();
        rows.push(json!({"parts": keys, "pitch_mm": 18, "columns": 5, "row_pitch_mm": 17}));
        let mut half = vec![keys[0].clone(), format!("SW{}", first + 15), format!("SW{}", first + 16)];
        half.extend((first..first + 17).map(|n| format!("D{n}")));
        half.extend([mcu, display, reset, power].map(String::from));
        half.extend(battery.map(String::from));
        regions.push(json!({"parts": half, "x": x, "y": y}));
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
    json!({"version": 1, "move_all": true, "outline": outline,
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

    // A board without room is a fast no, with the room as numbers. On 80 x
    // 40 mm neither half's key grid (88 x 44 mm) fits the board, nor its
    // region: an input error at once (the placer ran its levels for over
    // twelve minutes before), its verdict with the outline to try.
    fs::write(project.join("constraints.json"), constraints(80.0, 40.0).to_string()).unwrap();
    let small = root.join("small");
    let started = Instant::now();
    let output = run(&[Path::new("place-kicad-board"), &project, Path::new("main"), &small, &project.join("layout.json")]);
    assert_eq!(code(&output), 2, "a board without room");
    assert!(started.elapsed().as_secs() < 60);
    assert!(String::from_utf8_lossy(&output.stderr).contains("the row of SW1 (15 parts) cannot fit the outline"));
    let verdict = read(&small.join("verdict.json"));
    assert_eq!(verdict["status"], "usage_error");
    let room = &verdict["room"];
    assert_eq!(room["outline_mm"], json!([80.0, 40.0]));
    // The keys are on the back: more than the target there.
    assert!(room["utilization"][1].as_f64().unwrap() > room["target_utilization"].as_f64().unwrap(), "{room}");
    assert!(room["missing_mm2"][1].as_f64().unwrap() > 0.0, "{room}");
    let suggested = room["suggested_outline_mm"].as_array().unwrap_or_else(|| panic!("{room}"));
    assert!(suggested[0].as_f64().unwrap() >= 88.0 && suggested[1].as_f64().unwrap() >= 88.0, "two 88 x 44 mm grids: {room}");
    assert!(room["limits"].as_array().unwrap().iter().any(|limit| limit.as_str().unwrap().contains("the row of SW1")), "{room}");
    assert!(verdict["next_actions"].as_array().unwrap().iter().any(|action| action.as_str().unwrap().starts_with("try an outline of")));
    // The same directory takes the retry (it holds only that verdict).
    // Without the rows, on 60 x 40 mm: the parts need more area than a
    // side has even with their tightest bodies; the placer says no at
    // once (exit 1), the parts it could not seat unplaced.
    fs::write(project.join("constraints.json"), json!({"version": 1, "move_all": true, "outline": {"width": 60, "height": 40}}).to_string())
        .unwrap();
    let started = Instant::now();
    let output = run(&[Path::new("place-kicad-board"), &project, Path::new("main"), &small, &project.join("layout.json")]);
    assert_eq!(code(&output), 1, "parts without room");
    assert!(started.elapsed().as_secs() < 60);
    let verdict = read(&small.join("verdict.json"));
    assert!(!verdict["unplaced"].as_array().unwrap().is_empty());
    let room = &verdict["room"];
    assert!(room["missing_mm2"][1].as_f64().unwrap() > 1000.0, "{room}");
    let suggested = room["suggested_outline_mm"].as_array().unwrap_or_else(|| panic!("{room}"));
    assert!(suggested[0].as_f64().unwrap() * suggested[1].as_f64().unwrap() > 60.0 * 40.0 / 0.7, "{room}");
    assert_eq!(read(&small.join("board-placer.json"))["overfull"], true);

    // Room enough: the placement keeps every constraint, in seconds, and
    // lists the constraints it keeps by construction (a side, the turns a
    // part may take) like the others.
    let mut kept = constraints(230.0, 95.0);
    kept["rotation"] = json!([{"part": "RSW0", "angles": [0, 180]}]);
    kept["back"] = json!(["Display2"]);
    kept["front"] = json!(["RSW1"]);
    fs::write(project.join("constraints.json"), kept.to_string()).unwrap();
    let placed = root.join("placed");
    let output = run(&[Path::new("place-kicad-board"), &project, Path::new("main"), &placed, &project.join("layout.json")]);
    assert_eq!(code(&output), 0, "placement");
    let placement = read(&placed.join("board-placer.json"));
    let statuses = placement["constraints"].as_array().unwrap();
    assert!(statuses.iter().all(|status| status["satisfied"] == true), "{statuses:?}");
    // A diode near a key the row carries is reported against that key.
    assert!(statuses.iter().any(|status| status["part"] == "D2" && status["other"] == "SW2"), "{statuses:?}");
    assert!(stdout_has_no_overlap_warnings(&output));
    for (kind, part) in [("rotation", "RSW0"), ("back", "Display2"), ("front", "RSW1")] {
        assert!(statuses.iter().any(|status| status["kind"] == kind && status["part"] == part && status["satisfied"] == true), "{kind} {part}");
    }
    // The room on a board with room: below the target on both sides, no
    // outline suggested, the smallest one within the board's.
    let room = &placement["room"];
    assert!(room["utilization"].as_array().unwrap().iter().all(|share| share.as_f64().unwrap() < 0.7), "{room}");
    assert_eq!(room["missing_mm2"], json!([0.0, 0.0]));
    assert!(room.get("suggested_outline_mm").is_none(), "{room}");
    fs::write(project.join("constraints.json"), constraints(230.0, 95.0).to_string()).unwrap();

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
    // Nothing to do; the parity findings are the controller's no-connect
    // pins, and the verdict says so.
    let actions = verdict["next_actions"].as_array().unwrap();
    assert!(actions.iter().all(|action| action.as_str().unwrap().starts_with("nothing to do")), "{actions:?}");
    assert!(verdict["skipped"].is_array());
    assert!(verdict["work_seconds"].as_f64().unwrap() > 0.0);
    assert!(verdict["drc_cache"]["calls"].as_u64().unwrap() >= 1);
    // The no-net pins above are KiCad's parity findings, nothing more
    // (181 when the library's references won).
    assert!(verdict["schematic_parity_issues"].as_u64().unwrap() <= 6, "{verdict}");
    let by_type: u64 = verdict["schematic_parity_by_type"].as_object().unwrap().values().map(|count| count.as_u64().unwrap()).sum();
    assert_eq!(by_type, verdict["schematic_parity_issues"].as_u64().unwrap(), "{verdict}");
    // The layout's room: the placement's, the board with room to spare.
    assert_eq!(verdict["room"]["outline_mm"], json!([230.0, 95.0]), "{verdict}");
    assert!(verdict["room"]["utilization"][1].as_f64().unwrap() < 0.7, "{verdict}");
    assert!(verdict["room"].get("suggested_outline_mm").is_none(), "{verdict}");
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

/// The keyboard on outlines that are not rectangles (docs/constraints.md,
/// outline): an L whose right half lies 30 mm lower (its power switch
/// held at the top keeps to the step, the nearest side facing up), and a
/// rectangle with round corners, a slot between the halves and a
/// triangular window. Each is laid out on a short budget; the result
/// passes, or the verdict names the parts that found no place. KiCad's
/// DRC checks the outline (closed, no copper too close to it).
#[test]
fn the_keyboard_lays_out_on_an_l_and_on_a_board_with_round_corners_and_cutouts() {
    if Command::new("kicad-cli").arg("version").output().is_err() {
        eprintln!("kicad-cli is not on the PATH: the walkthrough needs KiCad; skipped");
        return;
    }
    let root = scratch();
    let design = root.join("design");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/pcb-kicad/tests/fixtures/urchin"), &design);
    let project = root.join("project");
    let output = run(&[Path::new("import-kicad-netlist"), &design.join("main.kicad_sch"), &project, Path::new("main")]);
    assert_eq!(code(&output), 0, "import");
    fs::write(project.join("layout.json"), json!({"placer": {"constraints": "constraints.json"}, "total_seconds": 30}).to_string()).unwrap();
    let l_shape = json!({"x": 0, "y": 0, "polygon": [[0, 0], [115, 0], [115, 30], [230, 30], [230, 125], [0, 125]]});
    let cut = json!({"x": 0, "y": 0, "width": 230, "height": 95, "corner_radius": 5,
                     "cutouts": [{"x": [110, 120], "y": [20, 75]}, {"polygon": [[60, 80], [70, 80], [65, 88]]}]});
    for (name, outline, heights) in [("l-shape", l_shape, [[0.0, 125.0], [30.0, 125.0]]), ("cut", cut, [[0.0, 95.0], [0.0, 95.0]])] {
        fs::write(project.join("constraints.json"), keyboard(outline, 230.0, heights).to_string()).unwrap();
        let out = root.join(name);
        let output = run(&[Path::new("layout-kicad-board"), &project, Path::new("main"), &out, Path::new("auto"), &project.join("layout.json")]);
        let verdict = read(&out.join("verdict.json"));
        if code(&output) != 0 {
            // Not complete: only for parts without a place, and named.
            assert_eq!(code(&output), 1, "{name}: {verdict}");
            assert!(!verdict["unplaced"].as_array().unwrap().is_empty(), "{name}: {verdict}");
            continue;
        }
        assert_eq!(verdict["status"], "complete", "{name}: {verdict}");
        assert_eq!(verdict["constraints_missed"], json!([]), "{name}");
        assert_eq!(verdict["new_findings"], json!({}), "{name}");
        // The room counts the outline's own area: the L's 230 x 125 box
        // less its 115 x 30 notch, the other less its holes and corners.
        let usable = verdict["room"]["usable_mm2"][0].as_f64().unwrap();
        let area = if name == "l-shape" { 230.0 * 125.0 - 115.0 * 30.0 } else { 230.0 * 95.0 - 10.0 * 55.0 - 40.0 };
        assert!(usable < area && usable > 0.9 * area, "{name}: usable {usable} of {area}");
        let board = fs::read_to_string(out.join("result/main.kicad_pcb")).unwrap();
        let placement = read(&out.join("placed/board-placer.json"));
        let body = |reference: &str| {
            let footprint = placement["footprints"].as_array().unwrap().iter().find(|footprint| footprint["reference"] == reference).unwrap();
            let (center, half) = (&footprint["body_center"], &footprint["body_half"]);
            let at = |value: &Value, axis: usize| value[axis].as_f64().unwrap();
            [at(center, 0) - at(half, 0), at(center, 1) - at(half, 1), at(center, 0) + at(half, 0), at(center, 1) + at(half, 1)]
        };
        if name == "l-shape" {
            assert!(!board.contains("gr_rect") && !board.contains("gr_arc"));
            // The right half's power switch at the step, the left's at the top.
            let right = body("SW_POWER1");
            assert!((30.0..=31.0).contains(&right[1]) && right[0] >= 115.0, "SW_POWER1 at {right:?}");
            assert!(body("SW_POWER0")[1] <= 1.0);
        } else {
            assert_eq!(board.matches("(gr_arc").count(), 4);
            // No part's body in the slot.
            for footprint in placement["footprints"].as_array().unwrap() {
                let reference = footprint["reference"].as_str().unwrap();
                let b = body(reference);
                assert!(b[2] <= 110.0 || b[0] >= 120.0 || b[3] <= 20.0 || b[1] >= 75.0, "{reference} in the slot: {b:?}");
            }
        }
        let output = run(&[Path::new("drc-kicad-board"), &out.join("result"), Path::new("main"), &root.join(format!("drc-{name}.json"))]);
        assert_eq!(code(&output), 0, "drc-kicad-board {name}: {}", String::from_utf8_lossy(&output.stdout));
        let drc = read(&root.join(format!("drc-{name}.json")));
        let types: Vec<&str> = drc["violations"].as_array().unwrap().iter().map(|violation| violation["type"].as_str().unwrap()).collect();
        assert!(!types.iter().any(|kind| matches!(*kind, "invalid_outline" | "copper_edge_clearance")), "{name}: {types:?}");
    }
    fs::remove_dir_all(&root).ok();
}

/// The placer warned that every part of a fresh import overlapped every
/// other, as fixed parts, before `move_all` freed them.
fn stdout_has_no_overlap_warnings(output: &Output) -> bool {
    let all = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    !String::from_utf8_lossy(&all).contains("overlap")
}

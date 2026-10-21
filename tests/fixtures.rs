// Copyright (C) 2026 Toit contributors.

//! Fixture tests through the binary, on the committed fixture boards
//! (docs/handover.md, "The fixture boards"; review 2026-10-21, "Proposed
//! fixture tests"):
//!
//! - F1 (`tests/fixtures/f1`): 2 layers, 13 parts, 30 x 25 mm, KiCad 10's
//!   format written by hand: a QFN with an exposed pad and thermal vias, a
//!   SOT-223, an LED with 0.7 mm pads, a GND test pad alone on F.Cu, a net
//!   tie, a stacked front and back pad of one number, a jumper group, a
//!   pad with zone_connect none, a Margin rectangle, an unplated slot, a
//!   plated hole listing only `*.Mask`, a text box and a dimension on
//!   copper, a padstack, a net named "14", two zones of equal priority, a
//!   hatched zone, net classes (one by a regular expression) and a
//!   JLCPCB-style .kicad_dru with a pair rule.
//! - F2 (`tests/fixtures/f2`): F1 laid out with a free dimension.
//!
//! The tests, in the review's order: 1 no via in an SMD pad and no via hop
//! closer than hole_to_hole, 3 the rules end to end, 4 golden
//! fingerprints, 5 and 6 KiCad's connectivity and pours on the routed
//! board (skipped, with a message, without kicad-cli), 8 the free
//! dimension's clean path, 10 a plane pour and a keepout over a fixed
//! part. Tests 2, 7 and 9 need the crate's insides:
//! crates/pcb-kicad/src/tests/fixtures.rs.
//!
//! Every route pins the work rates (`PCB_WORK_RATE`), the memory
//! (`PCB_MAKER_MEMORY_MB`) and turns the wall guard off, so that the
//! machine's load cannot change a result. Golden files are in
//! `tests/fixtures/golden`; `PCB_UPDATE_GOLDEN=1` rewrites them.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

const RATE: &str = "3.15e6,150e6";

/// The short route of the fingerprint test: 2 s of ladder work, 6
/// negotiation iterations, without KiCad's check.
const ROUTER: &str =
    r#"{"ladder_budget_seconds": 2, "maximum_iterations": 6, "skip_native_verification": true}"#;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures() -> PathBuf {
    repository().join("tests/fixtures")
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-fixture-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// The binary with the rates and the memory pinned, the wall guard off
/// and the switches of the caller's environment that change a run
/// removed.
fn binary(threads: usize) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pcb-maker"));
    command
        .env("RAYON_NUM_THREADS", threads.to_string())
        .env("PCB_WORK_RATE", RATE)
        .env("PCB_MAKER_MEMORY_MB", "4096")
        .env("PCB_ROUTER_NO_WALL_GUARD", "1")
        .env("PCB_MAKER_NO_CACHE", "1");
    for variable in [
        "PCB_MAKER_TEST_PANIC",
        "PCB_SKIP_NATIVE",
        "PCB_ROUTER_FINGERPRINT",
        "PCB_ROUTER_FINGERPRINT_ALL",
        "PCB_ROUTER_SEED",
        "PCB_ROUTER_SEEDS",
        "PCB_ROUTER_NO_SHARED_STATICS",
        "PCB_KICAD_NO_NECK_REPAIR",
    ] {
        command.env_remove(variable);
    }
    command
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("exited")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(
        &fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display())),
    )
    .unwrap()
}

/// Whether kicad-cli is on the PATH; says so on stderr when not.
fn kicad_cli(test: &str) -> bool {
    let present = Command::new("kicad-cli")
        .arg("version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !present {
        eprintln!("SKIPPED {test}: kicad-cli is not on the PATH (needs KiCad 10)");
    }
    present
}

/// Compares `actual` with the golden file `name`, or writes it under
/// `PCB_UPDATE_GOLDEN=1`. A difference fails with the lines that differ
/// and how to accept them.
fn golden(name: &str, actual: &str) {
    let path = fixtures().join("golden").join(name);
    if std::env::var("PCB_UPDATE_GOLDEN").as_deref() == Ok("1") {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
        eprintln!("golden file written: {}", path.display());
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "golden file {} is missing; write it with PCB_UPDATE_GOLDEN=1",
            path.display()
        )
    });
    if expected != actual {
        let old: BTreeSet<&str> = expected.lines().collect();
        let new: BTreeSet<&str> = actual.lines().collect();
        let mut diff = String::new();
        for line in expected.lines().filter(|line| !new.contains(line)) {
            diff.push_str(&format!("- {line}\n"));
        }
        for line in actual.lines().filter(|line| !old.contains(line)) {
            diff.push_str(&format!("+ {line}\n"));
        }
        if diff.is_empty() {
            diff.push_str("(the same lines in another order)\n");
        }
        panic!(
            "{name} differs from its golden file ({}; - golden, + now):\n{diff}\
             If the change is intended, rewrite it with\n  \
             PCB_UPDATE_GOLDEN=1 cargo test --release -p pcb-maker --test fixtures\n\
             and commit the golden file's diff with the change.",
            path.display()
        );
    }
}

/// A copy of fixture `name`'s project in a scratch directory.
fn project(name: &str, root: &Path) -> PathBuf {
    let project = root.join(name);
    fs::create_dir_all(&project).unwrap();
    for entry in fs::read_dir(fixtures().join("f1")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), project.join(entry.file_name())).unwrap();
    }
    project
}

/// One route-kicad-board of F1.
struct Routed {
    root: PathBuf,
    output: Output,
    out: PathBuf,
}

impl Routed {
    fn report(&self) -> Value {
        read_json(&self.out.join("board-router.json"))
    }

    fn verdict(&self) -> Value {
        read_json(&self.out.join("verdict.json"))
    }

    fn board(&self) -> Node {
        parse(&fs::read_to_string(self.out.join("f1.kicad_pcb")).unwrap())
    }
}

impl Drop for Routed {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Routes F1 with `router` (JSON) at `threads` threads, `environment`
/// added; exit 0 or 1 asserted.
fn route(label: &str, router: &str, threads: usize, environment: &[(&str, &str)]) -> Routed {
    route_edited(label, router, threads, environment, |_| {})
}

/// `route` of F1 as `edit` changes its project directory first.
fn route_edited(
    label: &str,
    router: &str,
    threads: usize,
    environment: &[(&str, &str)],
    edit: impl Fn(&Path),
) -> Routed {
    let root = scratch(label);
    let source = project("f1", &root);
    edit(&source);
    let config = root.join("router.json");
    fs::write(&config, router).unwrap();
    let out = root.join("out");
    let mut command = binary(threads);
    for (key, value) in environment {
        command.env(key, value);
    }
    let output = command
        .arg("route-kicad-board")
        .arg(&source)
        .arg("f1")
        .arg(&out)
        .arg(&config)
        .args(["--reference", "none"])
        .output()
        .unwrap();
    assert!(
        matches!(code(&output), 0 | 1),
        "route-kicad-board of F1 ({label}) exited with {:?}:\n{}",
        output.status.code(),
        stderr(&output)
    );
    Routed { root, output, out }
}

// A reader of KiCad's S-expressions, enough for the boards the router
// writes: lists, atoms and quoted strings (unquoted).

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Atom(String),
    List(Vec<Node>),
}

impl Node {
    fn items(&self) -> &[Node] {
        match self {
            Node::List(items) => items,
            Node::Atom(_) => &[],
        }
    }

    fn head(&self) -> Option<&str> {
        self.items().first().and_then(Node::atom)
    }

    fn atom(&self) -> Option<&str> {
        match self {
            Node::Atom(text) => Some(text),
            Node::List(_) => None,
        }
    }

    /// The children with head `name`.
    fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.items()
            .iter()
            .filter(move |item| item.head() == Some(name))
    }

    fn child(&self, name: &str) -> Option<&Node> {
        self.items().iter().find(|item| item.head() == Some(name))
    }

    /// Atom `index` of child `name` (`(at 1 2)`: index 1 is 1).
    fn value(&self, name: &str, index: usize) -> Option<&str> {
        self.child(name)?.items().get(index)?.atom()
    }

    fn number(&self, name: &str, index: usize) -> Option<f64> {
        self.value(name, index)?.parse().ok()
    }

    fn point(&self, name: &str) -> [f64; 2] {
        [self.number(name, 1).unwrap(), self.number(name, 2).unwrap()]
    }

    /// The atoms of child `name` after its head.
    fn atoms(&self, name: &str) -> Vec<&str> {
        self.child(name)
            .map(|node| node.items().iter().skip(1).filter_map(Node::atom).collect())
            .unwrap_or_default()
    }
}

fn parse(text: &str) -> Node {
    let bytes = text.as_bytes();
    let mut stack: Vec<Vec<Node>> = vec![Vec::new()];
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => {
                stack.push(Vec::new());
                index += 1;
            }
            b')' => {
                let list = stack.pop().unwrap();
                stack.last_mut().unwrap().push(Node::List(list));
                index += 1;
            }
            b'"' => {
                let mut text = Vec::new();
                index += 1;
                while bytes[index] != b'"' {
                    if bytes[index] == b'\\' {
                        index += 1;
                    }
                    text.push(bytes[index]);
                    index += 1;
                }
                index += 1;
                stack
                    .last_mut()
                    .unwrap()
                    .push(Node::Atom(String::from_utf8(text).unwrap()));
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            _ => {
                let start = index;
                while index < bytes.len()
                    && !bytes[index].is_ascii_whitespace()
                    && !matches!(bytes[index], b'(' | b')')
                {
                    index += 1;
                }
                stack
                    .last_mut()
                    .unwrap()
                    .push(Node::Atom(text[start..index].to_string()));
            }
        }
    }
    let mut top = stack.pop().unwrap();
    assert_eq!(top.len(), 1, "one expression");
    top.pop().unwrap()
}

/// A pad's copper on the board: its net, label, copper layers and shape.
struct Pad {
    label: String,
    net: Option<String>,
    smd: bool,
    layers: Vec<String>,
    center: [f64; 2],
    /// Half width and height in the pad's own frame, its angle (degrees,
    /// KiCad's: counter-clockwise on screen) and its corner radius.
    half: [f64; 2],
    angle: f64,
    radius: f64,
}

impl Pad {
    /// The distance from `point` to the pad's copper (negative inside).
    fn distance(&self, point: [f64; 2]) -> f64 {
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let [dx, dy] = [point[0] - self.center[0], point[1] - self.center[1]];
        // Into the pad's frame (y down, the angle counter-clockwise).
        let local = [dx * cos - dy * sin, dx * sin + dy * cos];
        let inner = [
            (self.half[0] - self.radius).max(0.0),
            (self.half[1] - self.radius).max(0.0),
        ];
        let outside = [
            (local[0].abs() - inner[0]).max(0.0),
            (local[1].abs() - inner[1]).max(0.0),
        ];
        let inside = (local[0].abs() - inner[0])
            .max(local[1].abs() - inner[1])
            .min(0.0);
        (outside[0].hypot(outside[1]) + inside) - self.radius
    }

    /// The distance from the segment `a`-`b` to the pad's copper, measured
    /// at points 0.01 mm apart along it (exact enough for a test).
    fn distance_to_segment(&self, a: [f64; 2], b: [f64; 2]) -> f64 {
        let steps = ((a[0] - b[0]).hypot(a[1] - b[1]) / 0.01).ceil().max(1.0) as usize;
        (0..=steps)
            .map(|step| {
                let t = step as f64 / steps as f64;
                self.distance([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])])
            })
            .fold(f64::INFINITY, f64::min)
    }
}

/// Every pad with copper of a board.
fn pads(board: &Node) -> Vec<Pad> {
    let mut found = Vec::new();
    for footprint in board.all("footprint") {
        let at = footprint.point("at");
        let rotation = footprint.number("at", 3).unwrap_or(0.0);
        let reference = footprint
            .all("property")
            .find(|property| property.items().get(1).and_then(Node::atom) == Some("Reference"))
            .and_then(|property| property.items().get(2)?.atom())
            .unwrap_or("")
            .to_string();
        let (sin, cos) = rotation.to_radians().sin_cos();
        for pad in footprint.all("pad") {
            let items = pad.items();
            let number = items[1].atom().unwrap_or("");
            let kind = items[2].atom().unwrap_or("");
            let shape = items[3].atom().unwrap_or("");
            let layers: Vec<String> = pad
                .atoms("layers")
                .into_iter()
                .filter(|layer| layer.ends_with(".Cu"))
                .map(str::to_string)
                .collect();
            if layers.is_empty() {
                continue;
            }
            let local = pad.point("at");
            let size = pad.point("size");
            let half = [size[0] / 2.0, size[1] / 2.0];
            let radius = match shape {
                "circle" | "oval" => half[0].min(half[1]),
                "roundrect" => {
                    pad.number("roundrect_rratio", 1).unwrap_or(0.25) * size[0].min(size[1])
                }
                _ => 0.0,
            };
            found.push(Pad {
                label: format!("{reference}.{number}"),
                net: pad.value("net", 1).map(str::to_string),
                smd: matches!(kind, "smd" | "connect"),
                layers,
                center: [
                    at[0] + local[0] * cos + local[1] * sin,
                    at[1] - local[0] * sin + local[1] * cos,
                ],
                half,
                // A pad's angle in the file is its own on the board.
                angle: pad.number("at", 3).unwrap_or(0.0),
                radius,
            });
        }
    }
    found
}

/// The vias of a board: centre, diameter, drill, net.
fn vias(board: &Node) -> Vec<([f64; 2], f64, f64, String)> {
    board
        .all("via")
        .map(|via| {
            (
                via.point("at"),
                via.number("size", 1).unwrap(),
                via.number("drill", 1).unwrap(),
                via.value("net", 1).unwrap_or("").to_string(),
            )
        })
        .collect()
}

/// A track: start, end, width, layer, net.
type Track = ([f64; 2], [f64; 2], f64, String, String);

/// The tracks of a board.
fn tracks(board: &Node) -> Vec<Track> {
    board
        .all("segment")
        .map(|segment| {
            (
                segment.point("start"),
                segment.point("end"),
                segment.number("width", 1).unwrap(),
                segment.value("layer", 1).unwrap().to_string(),
                segment.value("net", 1).unwrap_or("").to_string(),
            )
        })
        .collect()
}

/// A net's routed copper as strokes: its copper layers, the ends and the
/// half width (a via: one point, every layer, its radius).
fn strokes(board: &Node, net: &str) -> Vec<(Vec<String>, [f64; 2], [f64; 2], f64)> {
    let mut found: Vec<_> = tracks(board)
        .into_iter()
        .filter(|track| track.4 == net)
        .map(|track| (vec![track.3], track.0, track.1, track.2 / 2.0))
        .collect();
    found.extend(
        vias(board)
            .into_iter()
            .filter(|via| via.3 == net)
            .map(|via| (vec!["*.Cu".to_string()], via.0, via.0, via.1 / 2.0)),
    );
    found
}

fn shares_layer(a: &[String], b: &[String]) -> bool {
    a.iter()
        .any(|layer| layer == "*.Cu" || b.iter().any(|other| other == layer || other == "*.Cu"))
}

fn segment_distance(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> f64 {
    fn point_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
        let ab = [b[0] - a[0], b[1] - a[1]];
        let length = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if length == 0.0 {
            0.0
        } else {
            (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length).clamp(0.0, 1.0)
        };
        (p[0] - a[0] - t * ab[0]).hypot(p[1] - a[1] - t * ab[1])
    }
    let cross = |o: [f64; 2], p: [f64; 2], q: [f64; 2]| {
        (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0])
    };
    let (d1, d2, d3, d4) = (
        cross(c, d, a),
        cross(c, d, b),
        cross(a, b, c),
        cross(a, b, d),
    );
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return 0.0;
    }
    point_segment(a, c, d)
        .min(point_segment(b, c, d))
        .min(point_segment(c, a, b))
        .min(point_segment(d, a, b))
}

/// F1's hole-to-hole spacing (its .kicad_pro's `min_hole_to_hole`).
const HOLE_TO_HOLE: f64 = 0.25;

#[test]
fn no_via_lies_in_an_smd_pad_and_no_via_hops_within_the_hole_spacing() {
    // Review 2026-10-21, fixture test 1 (the fence's 14 vias in LED pads;
    // Castor's two vias 0.25 mm apart): F1 routed in route mode, every via
    // checked against every SMD pad but the QFN's exposed pad (U1.17, where
    // vias are the designer's way out), and against the other vias of its
    // net.
    let routed = route("vias", ROUTER, 2, &[]);
    let board = routed.board();
    let vias = vias(&board);
    assert!(
        vias.len() >= 3,
        "F1's route uses vias (the stacked pad's back, the B.Cu pours): {} vias\n{}",
        vias.len(),
        stderr(&routed.output)
    );
    let pads = pads(&board);
    let mut found = Vec::new();
    for (center, size, _, net) in &vias {
        for pad in pads.iter().filter(|pad| pad.smd && pad.label != "U1.17") {
            let gap = pad.distance(*center) - size / 2.0;
            if gap < -1.0e-6 {
                found.push(format!(
                    "via of {net} at ({:.3}, {:.3}) in {} ({}): {:.3} mm into its copper",
                    center[0],
                    center[1],
                    pad.label,
                    pad.net.as_deref().unwrap_or("no net"),
                    -gap
                ));
            }
        }
    }
    assert!(found.is_empty(), "vias in SMD pads:\n{}", found.join("\n"));
    let mut hops = Vec::new();
    for (index, (a, _, drill_a, net_a)) in vias.iter().enumerate() {
        for (b, _, drill_b, net_b) in &vias[index + 1..] {
            let distance = (a[0] - b[0]).hypot(a[1] - b[1]);
            let needed = (drill_a + drill_b) / 2.0 + HOLE_TO_HOLE;
            if net_a == net_b && distance < needed - 1.0e-6 {
                hops.push(format!(
                    "{net_a}: vias at ({:.3}, {:.3}) and ({:.3}, {:.3}), {distance:.3} mm apart, holes {:.3} mm (at least {HOLE_TO_HOLE})",
                    a[0],
                    a[1],
                    b[0],
                    b[1],
                    distance - (drill_a + drill_b) / 2.0
                ));
            }
        }
    }
    assert!(
        hops.is_empty(),
        "vias of one net closer than hole_to_hole:\n{}",
        hops.join("\n")
    );
    // The thermal vias of the exposed pad are the footprint's, unchanged.
    let thermal = pads
        .iter()
        .filter(|pad| pad.label == "U1.17" && !pad.smd)
        .count();
    assert_eq!(thermal, 4);
    // With `via_in_pad: true` (filled and capped vias) the rule of before:
    // TP2, a 1.2 mm GND pad alone on F.Cu, takes its via to the B.Cu pour
    // inside its copper (without it the via sits beside the pad).
    let filled = route(
        "via-in-pad",
        r#"{"ladder_budget_seconds": 2, "maximum_iterations": 6, "skip_native_verification": true,
            "via_in_pad": true}"#,
        2,
        &[],
    );
    let board = filled.board();
    let in_pads = self::vias(&board);
    let tp2 = self::pads(&board)
        .into_iter()
        .find(|pad| pad.label == "TP2.1")
        .unwrap();
    assert!(
        in_pads
            .iter()
            .any(|(center, size, _, _)| tp2.distance(*center) - size / 2.0 < -1.0e-6),
        "via_in_pad: true admits a via in TP2's pad: {:?}",
        in_pads
    );
}

#[test]
fn the_project_rules_reach_the_route_through_the_cli() {
    // Review 2026-10-21, fixture test 3 (a dropped `net_rules` line in
    // core_config or main.rs's merge; AMLing2's and tuxkeychain's
    // findings): F1's .kicad_dru read by route-kicad-board's config
    // merge, board-router.json's rules against the golden list, and the
    // pair rule kept on the routed copper.
    let routed = route("rules", ROUTER, 2, &[]);
    let report = routed.report();
    let rules = serde_json::to_string_pretty(&report["rules"]).unwrap();
    golden("f1-rules.json", &format!("{rules}\n"));
    let statuses: BTreeMap<String, String> = report["rules"]["custom_rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rule| {
            (
                rule["rule"].as_str().unwrap().to_string(),
                rule["status"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(statuses["PA to PB"], "applied", "{statuses:?}");
    assert_eq!(statuses["Pad to Track clearance"], "no_effect");
    // The pair's copper keeps 0.4 mm on the board written: the tracks and
    // vias of each against the tracks, vias and pads of the other.
    let board = routed.board();
    let (pa, pb) = (strokes(&board, "PA"), strokes(&board, "PB"));
    assert!(
        pa.iter().any(|stroke| stroke.1 != stroke.2)
            && pb.iter().any(|stroke| stroke.1 != stroke.2),
        "PA and PB are routed\n{}",
        stderr(&routed.output)
    );
    let pads = pads(&board);
    let mut close = Vec::new();
    for (first, second, own, other) in [(&pa, &pb, "PA", "PB"), (&pb, &pa, "PB", "PA")] {
        for a in first {
            for b in second.iter().filter(|b| shares_layer(&a.0, &b.0)) {
                let gap = segment_distance(a.1, a.2, b.1, b.2) - a.3 - b.3;
                if gap < 0.4 - 1.0e-6 {
                    close.push(format!(
                        "{own} at ({:.3}, {:.3}) to {other} at ({:.3}, {:.3}): {gap:.4}",
                        a.1[0], a.1[1], b.1[0], b.1[1]
                    ));
                }
            }
            for pad in pads
                .iter()
                .filter(|pad| pad.net.as_deref() == Some(other) && shares_layer(&a.0, &pad.layers))
            {
                let gap = pad.distance_to_segment(a.1, a.2) - a.3;
                if gap < 0.4 - 1.0e-6 {
                    close.push(format!(
                        "{own} at ({:.3}, {:.3}) to {}: {gap:.4}",
                        a.1[0], a.1[1], pad.label
                    ));
                }
            }
        }
    }
    assert!(
        close.is_empty(),
        "the pair rule (0.4 mm) is not kept:\n{}",
        close.join("\n")
    );
    // F1's pair keeps 0.4 mm without the rule too: the rule reaches the
    // search only if a pair rule it cannot keep leaves the pair open.
    // At 3 mm a track of PA cannot leave U1.10 (1 mm from PB's U1.12).
    let strict = route_edited("rules-3mm", ROUTER, 2, &[], |project| {
        let rules = project.join("f1.kicad_dru");
        let text = fs::read_to_string(&rules).unwrap();
        assert!(text.contains("(min 0.4mm)"));
        fs::write(&rules, text.replace("(min 0.4mm)", "(min 3mm)")).unwrap();
    });
    let open: Vec<String> = strict.report()["nets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|net| net["status"] != "routed")
        .map(|net| net["connection"].as_str().unwrap().to_string())
        .collect();
    assert!(
        open.iter().any(|net| net == "PA" || net == "PB"),
        "a 3 mm pair rule leaves PA or PB open (the rule reached the router): open {open:?}"
    );
}

/// The sorted fingerprint lines of one route of F1 at `threads` threads.
fn fingerprints(threads: usize) -> Vec<String> {
    let routed = route(
        &format!("fingerprint-{threads}"),
        ROUTER,
        threads,
        &[("PCB_ROUTER_FINGERPRINT", "1")],
    );
    let mut lines: Vec<String> = stderr(&routed.output)
        .lines()
        .filter(|line| line.starts_with("fingerprint "))
        .map(str::to_string)
        .collect();
    lines.sort();
    lines
}

#[test]
fn f1s_fingerprints_are_the_golden_ones_at_1_and_4_threads() {
    // Review 2026-10-21, fixture test 4: tests/router_fingerprint.rs
    // checks that a route does not depend on the thread count; this one
    // that it does not change unnoticed. Every change of the router's
    // behaviour on F1 is a golden diff to acknowledge.
    let one = fingerprints(1);
    assert!(!one.is_empty(), "no fingerprint printed");
    let four = fingerprints(4);
    assert_eq!(one, four, "F1's fingerprints at 1 and 4 threads differ");
    golden("f1-fingerprints.txt", &(one.join("\n") + "\n"));
}

/// A DRC report's findings as lines: the type and severity of each
/// violation, sorted, and every unconnected item's pair.
fn drc_lines(report: &Value) -> Vec<String> {
    let items = |finding: &Value| -> String {
        let mut items: Vec<&str> = finding["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["description"].as_str())
            .collect();
        items.sort();
        items.join(" / ")
    };
    let mut lines: Vec<String> = report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|finding| {
            format!(
                "{} {}: {}",
                finding["severity"].as_str().unwrap_or(""),
                finding["type"].as_str().unwrap_or(""),
                items(finding)
            )
        })
        .collect();
    lines.extend(
        report["unconnected_items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|finding| format!("unconnected: {}", items(finding))),
    );
    lines.sort();
    lines
}

#[test]
fn kicad_agrees_with_the_routers_connectivity_on_f1() {
    // Review 2026-10-21, fixture test 5: complete by the router's count
    // is KiCad's 0 unconnected. F1 routed with KiCad's check: the
    // router's open connections and KiCad's unconnected items agree (both
    // none, or both some), and KiCad's unconnected items on the routed
    // board are the golden ones.
    if !kicad_cli("kicad_agrees_with_the_routers_connectivity_on_f1") {
        return;
    }
    let routed = route(
        "kicad",
        r#"{"ladder_budget_seconds": 2, "maximum_iterations": 6}"#,
        2,
        &[],
    );
    let verdict = routed.verdict();
    assert_eq!(verdict["kicad_checked"], true, "{verdict}");
    let router_open = verdict["open_connections"].as_u64().unwrap();
    let kicad_open = verdict["kicad_unconnected_items"].as_u64().unwrap();
    assert_eq!(
        router_open == 0,
        kicad_open == 0,
        "the router counts {router_open} open, KiCad {kicad_open} unconnected"
    );
    let report = read_json(&routed.out.join("drc.json"));
    let unconnected: Vec<String> = drc_lines(&report)
        .into_iter()
        .filter(|line| line.starts_with("unconnected"))
        .collect();
    let dead: Vec<String> = routed.report()["diagnostics"]["dead_pads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pad| {
            format!(
                "dead pad {} of {}",
                pad["pad"].as_str().unwrap(),
                pad["net"].as_str().unwrap()
            )
        })
        .collect();
    golden(
        "f1-routed-connectivity.txt",
        &format!(
            "router open {router_open}, KiCad unconnected {kicad_open}\n{}\n{}\n",
            dead.join("\n"),
            unconnected.join("\n")
        ),
    );
}

#[test]
fn the_routed_f1s_pours_are_kicads() {
    // Review 2026-10-21, fixture test 6: the routed F1's findings with
    // KiCad's refill (starved thermals, isolated copper, clearances)
    // against the golden DRC, and the verdict's starved thermals as
    // KiCad's report has them. The unrouted F1 too: what the designer's
    // board has before any route.
    if !kicad_cli("the_routed_f1s_pours_are_kicads") {
        return;
    }
    let root = scratch("unrouted");
    let source = project("f1", &root);
    let output = binary(2)
        .args([
            "drc-kicad-board",
            &source.display().to_string(),
            "f1",
            &root.join("drc.json").display().to_string(),
        ])
        .output()
        .unwrap();
    assert!(
        matches!(code(&output), 0 | 1),
        "drc-kicad-board: {}",
        stderr(&output)
    );
    let unrouted = drc_lines(&read_json(&root.join("drc.json")));
    let _ = fs::remove_dir_all(&root);
    let routed = route(
        "pours",
        r#"{"ladder_budget_seconds": 2, "maximum_iterations": 6}"#,
        2,
        &[],
    );
    let report = read_json(&routed.out.join("drc.json"));
    let lines = drc_lines(&report);
    let starved = lines
        .iter()
        .filter(|line| line.contains(" starved_thermal:"))
        .count();
    assert_eq!(
        routed.verdict()["starved_thermals"].as_u64().unwrap() as usize,
        starved
    );
    golden(
        "f1-drc.txt",
        &format!(
            "unrouted F1\n{}\n\nrouted F1 (route mode, 2 s)\n{}\n",
            unrouted.join("\n"),
            lines.join("\n")
        ),
    );
}

/// A fake kicad-cli for F2's free dimension: version 10.0.7, and a clean
/// DRC report for a board whose outline ends at y 22 or more (the bottom
/// edge's sizes 22 and 24), one unconnected item below. Counts its DRC
/// runs in `<directory>/runs`.
#[cfg(unix)]
fn sized_kicad(directory: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = directory.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let report = |unconnected: Value| {
        json!({"$schema": "https://schemas.kicad.org/drc.v1.json", "source": "f1.kicad_pcb",
               "date": "2026-10-21T12:00:00", "kicad_version": "10.0.7", "coordinate_units": "mm",
               "included_severities": ["error", "warning", "exclusion"], "violations": [],
               "unconnected_items": unconnected, "schematic_parity": []})
    };
    let clean = report(json!([]));
    let open = report(json!([{"type": "unconnected_items", "severity": "error",
        "description": "Missing connection between items", "items": []}]));
    let program = bin.join("kicad-cli");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = version ]; then echo 10.0.7; exit 0; fi\n\
             echo run >> {runs}\n\
             board=\n\
             for argument in \"$@\"; do case \"$argument\" in *.kicad_pcb) board=\"$argument\";; esac; done\n\
             size=$(tr -s ' \\t\\n' ' ' < \"$board\" | grep -o '(gr_rect (start 0 0) (end 30 [0-9.]*)' | head -n 1 | sed 's/.* //; s/)//')\n\
             while [ \"$#\" -gt 0 ]; do\n\
               if [ \"$1\" = --output ]; then\n\
                 shift\n\
                 if [ \"${{size%%.*}}\" -ge 22 ] 2>/dev/null; then\n\
                   cat > \"$1\" <<'REPORT'\n{clean}\nREPORT\n\
                 else\n\
                   cat > \"$1\" <<'REPORT'\n{open}\nREPORT\n\
                 fi\n\
                 exit 0\n\
               fi\n\
               shift\n\
             done\n\
             exit 99\n",
            runs = directory.join("runs").display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    bin
}

#[test]
#[cfg(unix)]
fn f2s_free_dimension_stops_at_the_first_clean_size() {
    // Review 2026-10-21, fixture test 8: the free dimension's clean path
    // through the binary (the CLI test in cli_exit_codes.rs has no size
    // clean). F2 is F1 with its bottom edge free from 18 to 24 mm, two
    // sizes side by side, and a fake kicad-cli clean from 22 mm: the
    // probes stop after the batch of 22, the confirmation lays out 22 mm
    // at a third of the budget, the result's outline is 22 mm with the
    // band's items moved, and the children run at the parent's rates.
    let root = scratch("free-dimension");
    let source = project("f1", &root);
    let bin = sized_kicad(&root);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = root.join("out");
    let output = binary(4)
        // The parent measures its rates and pins them for the children.
        .env_remove("PCB_WORK_RATE")
        .env("PATH", &path)
        .args([
            "layout-kicad-board",
            &source.display().to_string(),
            "f1",
            &out.display().to_string(),
            "auto",
            &fixtures().join("f2/layout.json").display().to_string(),
        ])
        .output()
        .unwrap();
    let text = stderr(&output);
    let verdict = read_json(&out.join("verdict.json"));
    let free = &verdict["free_dimension"];
    assert_eq!(free["chosen"], 22.0, "{free}\n{text}");
    assert_eq!(free["clean"], true, "{free}");
    assert_eq!(code(&output), 0, "{verdict}\n{text}");
    let table = read_json(&out.join("free-dimension.json"));
    let runs: Vec<(f64, String, f64, bool)> = table["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| {
            (
                run["size_mm"].as_f64().unwrap(),
                run["kind"].as_str().unwrap().to_string(),
                run["budget_seconds"].as_f64().unwrap(),
                run["chosen"] == true,
            )
        })
        .collect();
    // 18 mm would put the fixed J1 outside the outline: infeasible, no
    // run. 20 and 22 side by side: 22 is clean, the probes stop (24 is
    // never tried); 22 again at the confirmation's budget, a third of the
    // search's 60 s.
    assert_eq!(
        runs,
        vec![
            (18.0, "infeasible".to_string(), 0.0, false),
            (20.0, "probe".to_string(), 6.0, false),
            (22.0, "probe".to_string(), 6.0, false),
            (22.0, "confirm".to_string(), 20.0, true),
        ],
        "{table}"
    );
    assert_eq!(table["untried"], json!([24.0]), "{table}");
    assert!(
        root.join("runs").is_file(),
        "the layouts ran the fake kicad-cli"
    );
    // The result's outline is 22 mm high; the band's footprints, zones
    // and graphics moved up with the edge by 3 mm, the rest stayed.
    let board = parse(&fs::read_to_string(out.join("result/f1.kicad_pcb")).unwrap());
    let outline = board
        .all("gr_rect")
        .find(|rect| rect.value("layer", 1) == Some("Edge.Cuts"))
        .unwrap();
    assert_eq!(outline.point("end"), [30.0, 22.0]);
    let input = parse(
        &fs::read_to_string(out.join("free-dimension/22mm-confirm/input/f1.kicad_pcb")).unwrap(),
    );
    let footprint = |reference: &str| {
        input
            .all("footprint")
            .find(|footprint| {
                footprint
                    .all("property")
                    .any(|property| property.items().get(2).and_then(Node::atom) == Some(reference))
            })
            .unwrap()
            .point("at")
    };
    assert_eq!(footprint("R1"), [6.0, 19.5], "R1 moved with the edge");
    assert_eq!(footprint("TP1"), [14.0, 19.5]);
    assert_eq!(footprint("J1"), [2.5, 5.0], "J1 stays");
    let hatched = input
        .all("zone")
        .find(|zone| {
            zone.child("fill")
                .is_some_and(|fill| fill.value("mode", 1) == Some("hatch"))
        })
        .unwrap();
    let corners: Vec<[f64; 2]> = hatched
        .child("polygon")
        .unwrap()
        .child("pts")
        .unwrap()
        .all("xy")
        .map(|xy| {
            [
                xy.items()[1].atom().unwrap().parse().unwrap(),
                xy.items()[2].atom().unwrap().parse().unwrap(),
            ]
        })
        .collect();
    assert!(
        corners
            .iter()
            .any(|corner| (corner[1] - 21.5).abs() < 1.0e-9),
        "the hatched zone's bottom moved from 24.5 to 21.5: {corners:?}"
    );
    // The children ran at the parent's rates, pinned.
    let parent = (
        verdict["work_rate"].as_f64().unwrap(),
        verdict["placer_work_rate"].as_f64().unwrap(),
    );
    for size in ["20mm-probe", "22mm-probe"] {
        let child = read_json(&out.join(format!("free-dimension/{size}/layout/verdict.json")));
        assert_eq!(
            child["work_rate_source"], "PCB_WORK_RATE",
            "{size}: {child}"
        );
        assert_eq!(
            (
                child["work_rate"].as_f64().unwrap(),
                child["placer_work_rate"].as_f64().unwrap()
            ),
            parent,
            "{size}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_plane_pour_and_a_keepout_over_a_fixed_part() {
    // Review 2026-10-21, fixture test 10 (the fence report's plane pour
    // and keepout boxes): a keepout box over the fixed header J1 is
    // written as a rule area with footprints allowed and warned of first
    // in the verdict's next_actions; a GND plane on B.Cu (`add_pours`
    // plane: true, where the board has a GND zone already) writes one rule
    // area keeping tracks off the layer, and no track is routed there.
    let root = scratch("plane");
    let source = project("f1", &root);
    let placer = root.join("placer.json");
    fs::write(
        &placer,
        json!({"fixed": ["J1"], "fixed_patterns": ["*"],
               "constraints": {"version": 1, "keepout": [{"x": [0.5, 4], "y": [8, 11]}]}})
        .to_string(),
    )
    .unwrap();
    let placed = root.join("placed");
    let output = binary(2)
        .args([
            "place-kicad-board",
            &source.display().to_string(),
            "f1",
            &placed.display().to_string(),
            &placer.display().to_string(),
        ])
        .output()
        .unwrap();
    assert!(matches!(code(&output), 0 | 1), "{}", stderr(&output));
    let verdict = read_json(&placed.join("verdict.json"));
    let first = verdict["next_actions"][0].as_str().unwrap_or("");
    assert!(
        first.starts_with("keepout 1 ") && first.contains("J1"),
        "the keepout's warning comes first: {verdict}"
    );
    let board = parse(&fs::read_to_string(placed.join("f1.kicad_pcb")).unwrap());
    let areas: Vec<&Node> = board
        .all("zone")
        .filter(|zone| {
            zone.value("name", 1)
                .is_some_and(|name| name.starts_with("constraint keepout"))
        })
        .collect();
    assert_eq!(areas.len(), 1, "one keepout rule area");
    let keepout = areas[0].child("keepout").unwrap();
    assert_eq!(
        keepout.value("footprints", 1),
        Some("allowed"),
        "{keepout:?}"
    );
    // The plane on the placed board.
    let router = root.join("router.json");
    fs::write(
        &router,
        json!({"ladder_budget_seconds": 2, "maximum_iterations": 6,
               "skip_native_verification": true,
               "add_pours": [{"net": "GND", "layers": ["B.Cu"], "plane": true}]})
        .to_string(),
    )
    .unwrap();
    let out = root.join("routed");
    let output = binary(2)
        .args([
            "route-kicad-board",
            &placed.display().to_string(),
            "f1",
            &out.display().to_string(),
            &router.display().to_string(),
            "--reference",
            "none",
        ])
        .output()
        .unwrap();
    assert!(matches!(code(&output), 0 | 1), "{}", stderr(&output));
    let board = parse(&fs::read_to_string(out.join("f1.kicad_pcb")).unwrap());
    let planes: Vec<&Node> = board
        .all("zone")
        .filter(|zone| {
            zone.value("name", 1)
                .is_some_and(|name| name.starts_with("plane "))
        })
        .collect();
    assert_eq!(planes.len(), 1, "one plane rule area");
    assert_eq!(planes[0].value("name", 1), Some("plane GND B.Cu"));
    let on_back: Vec<String> = tracks(&board)
        .into_iter()
        .filter(|track| track.3 == "B.Cu")
        .map(|track| format!("{} from {:?} to {:?}", track.4, track.0, track.1))
        .collect();
    assert!(
        on_back.is_empty(),
        "tracks on the plane's layer: {on_back:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "fails on main at 20956b3: placement_constraints::HeldParts reads the placer config's fixed lists only, not constraints.json's fixed"]
fn a_keepout_over_a_part_fixed_by_the_constraints_file_is_warned_of() {
    // The same keepout as above, with the parts fixed by the constraints'
    // own `fixed` (docs/constraints.md's format) instead of the placer
    // config's lists: the placer keeps J1 where it is all the same, but
    // the rule area is written with footprints not allowed (KiCad then
    // reports J1) and nothing is warned of.
    let root = scratch("keepout-constraints");
    let source = project("f1", &root);
    let placer = root.join("placer.json");
    fs::write(
        &placer,
        json!({"constraints": {"version": 1, "fixed": ["*"],
                               "keepout": [{"x": [0.5, 4], "y": [8, 11]}]}})
        .to_string(),
    )
    .unwrap();
    let placed = root.join("placed");
    let output = binary(2)
        .args([
            "place-kicad-board",
            &source.display().to_string(),
            "f1",
            &placed.display().to_string(),
            &placer.display().to_string(),
        ])
        .output()
        .unwrap();
    assert!(matches!(code(&output), 0 | 1), "{}", stderr(&output));
    let verdict = read_json(&placed.join("verdict.json"));
    assert!(
        verdict["next_actions"][0]
            .as_str()
            .is_some_and(|first| first.starts_with("keepout 1 ") && first.contains("J1")),
        "the keepout's warning comes first: {verdict}"
    );
    let board = parse(&fs::read_to_string(placed.join("f1.kicad_pcb")).unwrap());
    let area = board
        .all("zone")
        .find(|zone| zone.value("name", 1) == Some("constraint keepout 1"))
        .unwrap();
    assert_eq!(
        area.child("keepout").unwrap().value("footprints", 1),
        Some("allowed")
    );
    fs::remove_dir_all(root).unwrap();
}

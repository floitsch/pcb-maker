// Copyright (C) 2026 Toit contributors.

use super::board_outline::{KiCadCutout, signed_area};
use super::held::{held_parts_overlapping, without_parts_placed_elsewhere};
use super::keepout::held_parts_in_keepouts;
use super::rows::apply_rows;
use super::*;

type PadNames = Vec<BTreeMap<String, [f64; 2]>>;

fn problem_only() -> core::Problem {
    problem().0
}

fn references() -> Vec<String> {
    problem().1
}

fn problem() -> (core::Problem, Vec<String>, PadNames) {
    let part = |fixed: bool| core::Component {
        name: String::new(),
        body_center: [0.0, 0.0],
        body_size: [2.0, 1.0],
        round: false,
        halo: 0.0,
        pins: Vec::new(),
        side: core::Side::Front,
        fixed,
        angle_options: vec![0.0, 90.0, 180.0, 270.0],
        far_side: Vec::new(),
        far_copper: Vec::new(),
        far_open: Default::default(),
        far_courtyard: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
        courtless: Vec::new(),
        slots: Vec::new(),
    };
    let problem = core::Problem {
        outline: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
        components: vec![part(true), part(false), part(false)],
        net_weights: Vec::new(),
        poses: vec![
            core::Pose {
                position: [10.0, 10.0],
                angle: 0.0
            };
            3
        ],
        spacing: 0.2,
        grid: 0.1,
        edge_margin: 0.5,
        min_spacing: 0.0,
        far_side_pads_only: false,
        escape: None,
        crossings: None,
        pieces: Vec::new(),
        constraints: Default::default(),
    };
    let references = vec!["J1".to_string(), "C1".to_string(), "C2".to_string()];
    let pads = vec![
        BTreeMap::from([("1".to_string(), [0.5, 0.0])]),
        BTreeMap::new(),
        BTreeMap::new(),
    ];
    (problem, references, pads)
}

fn apply(json: &str) -> Result<(core::Problem, Vec<String>), String> {
    let (mut problem, references, pads) = problem();
    let constraints: KiCadPlacementConstraints =
        serde_json::from_str(json).map_err(|e| e.to_string())?;
    let warnings = apply_constraints(
        &mut problem,
        &references,
        &pads,
        &[vec![[0.0, -0.5, 1.0, 0.5]], Vec::new(), Vec::new()],
        &[None, None, None],
        &[None, None, None],
        &[false, false, false],
        &[None, None, None],
        &constraints,
        10.0,
    )?;
    Ok((problem, warnings))
}

#[test]
fn move_all_sends_mounting_holes_nobody_placed_to_the_corners() {
    let run = |json: &str| {
        let (mut problem, references, pads) = problem();
        // J1 is a mounting hole: a hole, no nets.
        problem.components[0].far_side = vec![[-1.0, -1.0, 1.0, 1.0]];
        let constraints: KiCadPlacementConstraints = serde_json::from_str(json).unwrap();
        apply_constraints(
            &mut problem,
            &references,
            &pads,
            &[Vec::new(), Vec::new(), Vec::new()],
            &[None, None, None],
            &[None, None, None],
            &[false, false, false],
            &[None, None, None],
            &constraints,
            10.0,
        )
        .unwrap();
        problem
    };
    let problem = run(r#"{"version": 1, "move_all": true}"#);
    assert!(problem.components.iter().all(|component| !component.fixed));
    assert_eq!(
        problem.constraints.edges,
        vec![(0, Edge::Top, 1.0 + 1.0e-3), (0, Edge::Left, 1.0 + 1.0e-3)]
    );
    // Named anywhere, it goes where it is told.
    let problem = run(r#"{"version": 1, "move_all": true, "fixed": ["J1"]}"#);
    assert!(problem.components[0].fixed && problem.constraints.edges.is_empty());
    let problem =
        run(r#"{"version": 1, "move_all": true, "place": [{"part": "J1", "at": "bottom"}]}"#);
    assert_eq!(
        problem.constraints.edges,
        vec![(0, Edge::Bottom, 1.0 + 1.0e-3)]
    );
}

#[test]
fn a_repeated_reference_names_its_parts_in_turn() {
    // A panel's copies repeat references: C1 twice. The first edge
    // entry for C1 is about the first C1, the second about the second
    // (all on the first asked one part to touch two opposite edges).
    let (mut problem, mut references, pads) = problem();
    references[2] = "C1".to_string();
    let constraints: KiCadPlacementConstraints = serde_json::from_str(
        r#"{"version": 1,
            "edge": [{"part": "C1", "edge": "left"}, {"part": "C1", "edge": "right"}],
            "rotation": [{"part": "C1", "angle": 90}, {"part": "C1", "angle": 0}]}"#,
    )
    .unwrap();
    apply_constraints(
        &mut problem,
        &references,
        &pads,
        &[Vec::new(), Vec::new(), Vec::new()],
        &[None, None, None],
        &[None, None, None],
        &[false, false, false],
        &[None, None, None],
        &constraints,
        10.0,
    )
    .unwrap();
    let edges: Vec<(usize, Edge)> = problem
        .constraints
        .edges
        .iter()
        .map(|(part, edge, _)| (*part, *edge))
        .collect();
    assert_eq!(edges, vec![(1, Edge::Left), (2, Edge::Right)]);
    assert_eq!(problem.components[1].angle_options, vec![90.0]);
    assert_eq!(problem.components[2].angle_options, vec![0.0]);
}

#[test]
fn a_repeated_reference_entry_with_a_uuid_names_that_footprint() {
    // The molecule panels' tasks list edge entries only for the copies
    // at an edge, not in board order, and some copies have none: in
    // turn, the first entry bound the first copy. By uuid each entry
    // binds its own footprint, whatever the order; entries without one
    // still take the parts in turn (hand-written constraints), and a
    // uuid no such footprint has is an error.
    let (mut problem, mut references, pads) = problem();
    references[2] = "C1".to_string();
    let uuids = [
        Some("aaaa".to_string()),
        Some("b1".to_string()),
        Some("C2".to_string()),
    ];
    let constraints: KiCadPlacementConstraints = serde_json::from_str(
        r#"{"version": 1,
            "edge": [{"part": "C1", "edge": "top", "uuid": "c2"}, {"part": "C1", "edge": "left"},
                     {"part": "C1", "edge": "bottom", "uuid": "c2"}],
            "rotation": [{"part": "C1", "angle": 90, "uuid": "c2"}],
            "place": [{"part": "C1", "at": "right", "uuid": "b1"}]}"#,
    )
    .unwrap();
    apply_constraints(
        &mut problem,
        &references,
        &pads,
        &[Vec::new(), Vec::new(), Vec::new()],
        &[None, None, None],
        &[None, None, None],
        &[false, false, false],
        &uuids,
        &constraints,
        10.0,
    )
    .unwrap();
    let edges: Vec<(usize, Edge)> = problem
        .constraints
        .edges
        .iter()
        .map(|(part, edge, _)| (*part, *edge))
        .collect();
    assert_eq!(
        edges,
        vec![
            (1, Edge::Right),
            (2, Edge::Top),
            (1, Edge::Left),
            (2, Edge::Bottom)
        ]
    );
    assert_eq!(problem.components[2].angle_options, vec![90.0]);
    assert_ne!(problem.components[1].angle_options, vec![90.0]);
    // Written back as read: no uuid where the entry had none.
    let written = serde_json::to_string(&constraints.edge[1]).unwrap();
    assert!(!written.contains("uuid"), "{written}");

    let (mut fresh, mut references, pads) = super::tests::problem();
    references[2] = "C1".to_string();
    let unknown: KiCadPlacementConstraints = serde_json::from_str(
        r#"{"version": 1, "edge": [{"part": "C1", "edge": "top", "uuid": "aaaa"}]}"#,
    )
    .unwrap();
    let error = apply_constraints(
        &mut fresh,
        &references,
        &pads,
        &[Vec::new(), Vec::new(), Vec::new()],
        &[None, None, None],
        &[None, None, None],
        &[false, false, false],
        &uuids,
        &unknown,
        10.0,
    )
    .unwrap_err();
    assert!(error.contains("uuid \"aaaa\""), "{error}");
}

#[test]
fn constraints_resolve_parts_pins_and_globs() {
    let (problem, warnings) = apply(
        r#"{"version": 1,
            "edge": [{"part": "J1", "edge": "left", "flush": true}],
            "region": [{"parts": ["C*"], "x": [0, 10], "y": [0, 5]}],
            "near": [{"part": "C1", "pin_of": "J1:1", "max_mm": 2}],
            "relative": [{"part": "C2", "below": "C1"}],
            "rotation": [{"part": "C2", "angles": [90, 270]}]}"#,
    )
    .unwrap();
    assert!(warnings.is_empty());
    // J1 was fixed by default; an edge constraint frees it.
    assert!(!problem.components[0].fixed);
    assert_eq!(problem.constraints.edges.len(), 1);
    assert_eq!(problem.constraints.regions.len(), 2);
    assert_eq!(
        problem.constraints.relations[0],
        Relation::Near {
            part: 1,
            anchor: Anchor::Point(0, [0.5, 0.0]),
            max: 2.0
        }
    );
    assert!(matches!(
        problem.constraints.relations[1],
        Relation::Beside {
            part: 2,
            anchor: 1,
            side: Edge::Bottom,
            ..
        }
    ));
    assert_eq!(problem.components[2].angle_options, vec![90.0, 270.0]);
    assert_eq!(problem.poses[2].angle, 90.0);
}

#[test]
fn a_region_in_words_is_that_part_of_the_outline_and_a_comment_is_read_past() {
    // The board is 30 x 20 mm: the left half is x 0-15, the centre the
    // middle third both ways.
    let (problem, _) = apply(
        r#"{"version": 1, "comment": ["inferred", "decide the pitch"],
            "region": [{"parts": ["C1"], "at": "left-half"}, {"parts": ["C2"], "at": "center"}]}"#,
    )
    .unwrap();
    assert_eq!(problem.constraints.regions[0], (1, [0.0, 0.0, 15.0, 20.0]));
    let (_, centre) = problem.constraints.regions[1];
    assert!(
        (centre[0] - 10.0).abs() < 1e-9 && (centre[3] - 40.0 / 3.0).abs() < 1e-9,
        "{centre:?}"
    );
    for wrong in [
        r#"{"version": 1, "region": [{"parts": ["C1"], "at": "left"}]}"#,
        r#"{"version": 1, "region": [{"parts": ["C1"], "at": "left-half", "x": [0, 1], "y": [0, 1]}]}"#,
        r#"{"version": 1, "region": [{"parts": ["C1"], "x": [0, 1]}]}"#,
    ] {
        assert!(apply(wrong).is_err(), "{wrong}");
    }
}

#[test]
fn an_outline_without_size_is_sized_from_the_parts() {
    let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)) (gr_line (start 0 0) (end 1 0) (layer \"Edge.Cuts\")))").unwrap();
    let outline: KiCadOutlineConstraint = serde_json::from_str(r#"{"aspect": 2.0}"#).unwrap();
    // 200 mm2 of parts at the default factor 2 (two layers): 400 mm2,
    // 28.5 x 14.5 (whole half millimetres, rounded up).
    let size = apply_outline(&mut pcb, &outline, 200.0).unwrap();
    assert_eq!(size, [28.5, 14.5]);
    let edges: Vec<_> = pcb
        .children()
        .iter()
        .filter(|item| form_atom(item, "layer", 1) == Some("Edge.Cuts"))
        .collect();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].head(), Some("gr_rect"));
    let only_width: KiCadOutlineConstraint = serde_json::from_str(r#"{"width": 10}"#).unwrap();
    assert!(apply_outline(&mut pcb, &only_width, 1.0).is_err());
    // 4500 mm2 of parts, 9000 mm2 of board: 116 x 78 at 1.5, but both
    // sides stay within 100 mm (the flat price at JLCPCB and PCBWay).
    let automatic: KiCadOutlineConstraint = serde_json::from_str("{}").unwrap();
    assert_eq!(
        apply_outline(&mut pcb, &automatic, 4500.0).unwrap(),
        [100.0, 90.0]
    );
    // On four layers, 1.5 times the parts.
    let mut four = parse("(kicad_pcb (layers (0 \"F.Cu\" signal) (4 \"In1.Cu\" signal) (6 \"In2.Cu\" signal) (2 \"B.Cu\" signal)))").unwrap();
    assert_eq!(
        apply_outline(&mut four, &outline, 300.0).unwrap(),
        [30.0, 15.0]
    );
}

fn edge_cuts(pcb: &Expr) -> Vec<&Expr> {
    pcb.children()
        .iter()
        .filter(|item| form_atom(item, "layer", 1) == Some("Edge.Cuts"))
        .collect()
}

#[test]
fn outline_shapes_parse_and_mistakes_are_errors() {
    let parsed: KiCadPlacementConstraints = serde_json::from_str(
        r#"{"version": 1, "outline": {"polygon": [[0, 0], [40, 0], [40, 10], [20, 10], [20, 30], [0, 30]], "corner_radius": 1,
            "cutouts": [{"x": [2, 6], "y": [2, 6]}, {"polygon": [[5, 15], [10, 15], [5, 20]], "origin": "absolute"}]}}"#,
    )
    .unwrap();
    let outline = parsed.outline.unwrap();
    assert_eq!(outline.polygon.as_ref().unwrap().len(), 6);
    assert_eq!(outline.cutouts.len(), 2);
    assert!(outline.polygonal() && !outline.automatic());
    let pieces: KiCadOutlineConstraint =
        serde_json::from_str(r#"{"pieces": [{"width": 20, "height": 10}, {"polygon": [[0, 0], [10, 0], [0, 10]], "x": 25}]}"#).unwrap();
    assert!(pieces.polygonal() && !pieces.automatic());
    let rounded: KiCadOutlineConstraint =
        serde_json::from_str(r#"{"width": 20, "height": 10, "corner_radius": 2}"#).unwrap();
    assert!(!rounded.polygonal());
    assert!(
        serde_json::from_str::<KiCadOutlineConstraint>(r#"{"polygon": [[0, 0]], "radius": 1}"#)
            .is_err(),
        "unknown key"
    );
    assert!(
        serde_json::from_str::<KiCadCutout>(r#"{"x": [0, 1], "y": [0, 1], "diameter": 1}"#)
            .is_err(),
        "unknown key"
    );

    let error = |json: &str| {
        let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)))").unwrap();
        let outline: KiCadOutlineConstraint = serde_json::from_str(json).unwrap();
        apply_outline(&mut pcb, &outline, 100.0).unwrap_err()
    };
    assert!(error(r#"{"polygon": [[0, 0], [10, 0]]}"#).contains("three corners"));
    assert!(error(r#"{"polygon": [[0, 0], [10, 0], [20, 0]]}"#).contains("no area"));
    assert!(
        error(r#"{"polygon": [[0, 0], [10, 0], [0, 10], [12, 14]]}"#).contains("crosses itself")
    );
    assert!(error(r#"{"polygon": [[0, 0], [10, 0], [0, 10]], "width": 10}"#).contains("not both"));
    assert!(
        error(r#"{"polygon": [[0, 0], [10, 0], [0, 10]], "aspect": 2}"#)
            .contains("sized from the parts")
    );
    assert!(error(r#"{"width": 10, "height": 10, "corner_radius": -1}"#).contains("corner_radius"));
    assert!(
        error(r#"{"width": 10, "height": 10, "cutouts": [{"x": [8, 12], "y": [1, 2]}]}"#)
            .contains("does not lie inside")
    );
    assert!(error(r#"{"width": 10, "height": 10, "cutouts": [{"x": [1, 4], "y": [1, 4]}, {"x": [3, 6], "y": [3, 6]}]}"#).contains("cutouts may not overlap"));
    assert!(
        error(r#"{"width": 10, "height": 10, "cutouts": [{"x": [1, 4]}]}"#)
            .contains("x and y ranges or a polygon")
    );
    assert!(error(r#"{"cutouts": [{"x": [1, 4], "y": [1, 4]}]}"#).contains("need a sized outline"));
    assert!(
        error(r#"{"pieces": [{"width": 10, "height": 10}, {"width": 10, "height": 10, "x": 5}]}"#)
            .contains("pieces may not overlap")
    );
    assert!(
        error(r#"{"pieces": [{"width": 10, "height": 10}], "width": 5}"#)
            .contains("each piece carries its own shape")
    );
    assert!(error(r#"{"pieces": [{"x": 5}]}"#).contains("needs width and height or a polygon"));
}

#[test]
fn outline_shapes_are_written_as_edge_cuts_lines_and_arcs() {
    use crate::outline::board_loops;
    let area = |points: &[[f64; 2]]| signed_area(points).abs();
    // An L, its corner at (5, 5): six lines, read back as one loop.
    let mut pcb = parse("(kicad_pcb (gr_rect (start 0 0) (end 1 1) (layer \"Edge.Cuts\")) (footprint \"x\" (at 10 10)))").unwrap();
    let outline: KiCadOutlineConstraint = serde_json::from_str(
        r#"{"polygon": [[0, 0], [40, 0], [40, 10], [20, 10], [20, 30], [0, 30]], "x": 5, "y": 5}"#,
    )
    .unwrap();
    assert_eq!(
        apply_outline(&mut pcb, &outline, 0.0).unwrap(),
        [40.0, 30.0]
    );
    let items = edge_cuts(&pcb);
    assert_eq!(items.len(), 6);
    assert!(items.iter().all(|item| item.head() == Some("gr_line")));
    assert_eq!(form_xy(items[0], "start").unwrap(), [5.0, 5.0]);
    assert_eq!(form_xy(items[0], "end").unwrap(), [45.0, 5.0]);
    let loops = board_loops(&pcb).unwrap();
    assert_eq!(loops.piece_loops.len(), 1);
    assert!(loops.cutouts.is_empty());
    assert!((area(&loops.outline) - (40.0 * 10.0 + 20.0 * 20.0)).abs() < 1.0e-9);

    // A rectangle with round corners, a box cut out and a triangle cut
    // out (absolute): four arcs, four lines, a box and three lines.
    let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)))").unwrap();
    let outline: KiCadOutlineConstraint = serde_json::from_str(
        r#"{"width": 30, "height": 20, "x": 0, "y": 0, "corner_radius": 3,
            "cutouts": [{"x": [10, 20], "y": [5, 10]}, {"polygon": [[2, 12], [6, 12], [2, 16]], "origin": "absolute"}]}"#,
    )
    .unwrap();
    assert_eq!(
        apply_outline(&mut pcb, &outline, 0.0).unwrap(),
        [30.0, 20.0]
    );
    let items = edge_cuts(&pcb);
    let count = |head: &str| {
        items
            .iter()
            .filter(|item| item.head() == Some(head))
            .count()
    };
    assert_eq!((count("gr_arc"), count("gr_line")), (4, 4 + 4 + 3));
    // The top-left arc: from (0, 3) through the corner's diagonal to (3, 0).
    let arc = items
        .iter()
        .find(|item| item.head() == Some("gr_arc"))
        .unwrap();
    assert_eq!(form_xy(arc, "start").unwrap(), [0.0, 3.0]);
    assert_eq!(form_xy(arc, "end").unwrap(), [3.0, 0.0]);
    let mid = form_xy(arc, "mid").unwrap();
    let expected = 3.0 - 3.0 / 2f64.sqrt();
    assert!(
        (mid[0] - expected).abs() < 1.0e-6 && (mid[1] - expected).abs() < 1.0e-6,
        "{mid:?}"
    );
    let loops = board_loops(&pcb).unwrap();
    // (The arcs are read back as chords within 5 um.)
    let rounded = 30.0 * 20.0 - (4.0 - std::f64::consts::PI) * 9.0;
    assert!(
        (area(&loops.outline) - rounded).abs() < 0.1,
        "{}",
        area(&loops.outline)
    );
    assert_eq!(loops.cutouts.len(), 2);
    assert_eq!(loops.board_cutouts, vec![true, true]);
    let mut holes: Vec<f64> = loops.cutouts.iter().map(|cutout| area(cutout)).collect();
    holes.sort_by(f64::total_cmp);
    assert!(
        (holes[0] - 8.0).abs() < 1.0e-9 && (holes[1] - 50.0).abs() < 1.0e-9,
        "{holes:?}"
    );

    // Two pieces, a rectangle and a triangle 5 mm to its right.
    let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)))").unwrap();
    let outline: KiCadOutlineConstraint = serde_json::from_str(
        r#"{"x": 100, "y": 50, "pieces": [{"width": 20, "height": 10}, {"polygon": [[0, 0], [10, 0], [0, 10]], "x": 25, "y": 0}]}"#,
    )
    .unwrap();
    assert_eq!(
        apply_outline(&mut pcb, &outline, 0.0).unwrap(),
        [35.0, 10.0]
    );
    let loops = board_loops(&pcb).unwrap();
    assert_eq!(
        loops.pieces,
        vec![[100.0, 50.0, 120.0, 60.0], [125.0, 50.0, 135.0, 60.0]]
    );
    assert!((area(&loops.outline) - 250.0).abs() < 1.0e-9);

    // A rectangle without round corners or cutouts stays one gr_rect.
    let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)))").unwrap();
    let plain: KiCadOutlineConstraint =
        serde_json::from_str(r#"{"width": 30, "height": 20, "corner_radius": 0}"#).unwrap();
    apply_outline(&mut pcb, &plain, 0.0).unwrap();
    assert_eq!(
        edge_cuts(&pcb)
            .iter()
            .map(|item| item.head())
            .collect::<Vec<_>>(),
        vec![Some("gr_rect")]
    );
    // A radius larger than a side allows rounds that corner less.
    let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)))").unwrap();
    let slot: KiCadOutlineConstraint =
        serde_json::from_str(r#"{"width": 30, "height": 4, "corner_radius": 5}"#).unwrap();
    apply_outline(&mut pcb, &slot, 0.0).unwrap();
    let loops = board_loops(&pcb).unwrap();
    let stadium = 26.0 * 4.0 + std::f64::consts::PI * 4.0;
    assert!(
        (area(&loops.outline) - stadium).abs() < 0.1,
        "{}",
        area(&loops.outline)
    );
}

#[test]
fn fixed_wins_over_a_constraint_and_mistakes_are_errors() {
    let (problem, _) = apply(r#"{"version": 1, "fixed": ["C1"], "near": [{"part": "C1", "part_of": "C2", "max_mm": 1}]}"#).unwrap();
    assert!(problem.components[1].fixed);
    let (problem, _) =
        apply(r#"{"version": 1, "group": [{"parts": ["C*", "J1"], "around": "J1", "max_mm": 2}]}"#)
            .unwrap();
    assert_eq!(problem.constraints.relations.len(), 2);
    assert!(
        problem
            .constraints
            .relations
            .iter()
            .all(|relation| matches!(
                relation,
                Relation::Near { anchor: Anchor::Body(0), max, .. } if *max == 2.0
            ))
    );
    let (problem, _) = apply(r#"{"version": 1, "hollow": ["J1"]}"#).unwrap();
    assert_eq!(problem.components[0].hollow, vec![[0.0, -0.5, 1.0, 0.5]]);
    for bad in [
        r#"{"version": 1, "near": [{"part": "R9", "part_of": "C2", "max_mm": 1}]}"#,
        r#"{"version": 1, "near": [{"part": "C1", "pin_of": "J1:7", "max_mm": 1}]}"#,
        r#"{"version": 1, "near": [{"part": "C1", "max_mm": 1}]}"#,
        r#"{"version": 1, "edge": [{"part": "C1", "edge": "north"}]}"#,
        r#"{"version": 1, "relative": [{"part": "C1", "below": "C2", "above": "J1"}]}"#,
        r#"{"version": 1, "fixed": ["Q*"]}"#,
        r#"{"version": 1, "edge": [{"part": "C1", "edge": "top", "overhang": {"edge": "top"}}]}"#,
        r#"{"version": 1, "colour": "green"}"#,
        r#"{"version": 1, "hollow": ["U*"]}"#,
        r#"{"version": 1, "group": [{"parts": ["C1", "U9"], "max_mm": 2}]}"#,
        r#"{"version": 1, "hollow": ["C1"]}"#,
    ] {
        assert!(apply(bad).is_err(), "{bad} was accepted");
    }
}

#[test]
fn apart_keeps_every_named_part_from_every_other() {
    let (problem, _) =
        apply(r#"{"version": 1, "apart": [{"parts": ["C1"], "from": ["C*", "J1"], "min_mm": 5}]}"#)
            .unwrap();
    assert_eq!(
        problem.constraints.relations,
        vec![
            Relation::Apart {
                part: 1,
                anchor: 2,
                min: 5.0
            },
            Relation::Apart {
                part: 1,
                anchor: 0,
                min: 5.0
            }
        ]
    );
    for bad in [
        r#"{"version": 1, "apart": [{"parts": ["C1"], "from": ["U*"], "min_mm": 5}]}"#,
        r#"{"version": 1, "apart": [{"parts": ["C1"], "from": ["C2"], "min_mm": 0}]}"#,
    ] {
        assert!(apply(bad).is_err(), "{bad} was accepted");
    }
}

#[test]
fn place_takes_words_and_exact_poses() {
    let (problem, _) = apply(
        r#"{"version": 1, "device_front": "left",
            "place": [{"part": "C1", "at": "front"}, {"part": "C2", "at": "bottom-right"},
                      {"part": "J1", "x": 5, "y": 6, "angle": 90}]}"#,
    )
    .unwrap();
    let edges: Vec<(usize, Edge)> = problem
        .constraints
        .edges
        .iter()
        .map(|(part, edge, _)| (*part, *edge))
        .collect();
    assert_eq!(
        edges,
        vec![(1, Edge::Left), (2, Edge::Bottom), (2, Edge::Right)]
    );
    assert!(!problem.components[1].fixed && !problem.components[2].fixed);
    assert_eq!(
        problem.poses[0],
        core::Pose {
            position: [5.0, 6.0],
            angle: 90.0
        }
    );
    assert!(problem.components[0].fixed);
    // Listed as kept with every other constraint (board-layout.json),
    // and missed where the part is not at its pose.
    let references: Vec<String> = ["J1", "C1", "C2"].iter().map(|r| r.to_string()).collect();
    let report = constraint_report(&problem, &problem.poses, &references);
    let place = report
        .iter()
        .find(|status| status.kind == "place")
        .expect("the place entry with x and y");
    assert!(place.part == "J1" && place.satisfied, "{report:?}");
    let mut moved = problem.poses.clone();
    moved[0].position[0] += 1.0;
    let report = constraint_report(&problem, &moved, &references);
    let place = report.iter().find(|status| status.kind == "place").unwrap();
    assert!(!place.satisfied && (place.violation_mm - 1.0).abs() < 1e-9);
    let (problem, _) = apply(r#"{"version": 1, "place": [{"part": "C1", "at": "center"}, {"part": "C2", "at": "top-half"}]}"#).unwrap();
    let expected = [
        (1, [10.0, 20.0 / 3.0, 20.0, 40.0 / 3.0]),
        (2, [0.0, 0.0, 30.0, 10.0]),
    ];
    assert_eq!(problem.constraints.regions.len(), 2);
    for ((part, region), (want_part, want)) in problem.constraints.regions.iter().zip(expected) {
        assert_eq!(*part, want_part);
        assert!(
            region.iter().zip(want).all(|(a, b)| (a - b).abs() < 1.0e-9),
            "{region:?}"
        );
    }
    for bad in [
        r#"{"version": 1, "place": [{"part": "C1", "at": "front"}]}"#,
        r#"{"version": 1, "place": [{"part": "C1", "at": "somewhere"}]}"#,
        r#"{"version": 1, "place": [{"part": "C1", "at": "left", "x": 1, "y": 2}]}"#,
        r#"{"version": 1, "place": [{"part": "C1", "x": 1}]}"#,
        r#"{"version": 1, "device_front": "north", "place": [{"part": "C1", "at": "front"}]}"#,
    ] {
        assert!(apply(bad).is_err(), "{bad} was accepted");
    }
}

#[test]
fn a_near_on_a_panels_copy_anchors_in_the_parts_own_copy() {
    // Two copies' U1 (references repeat), and the second copy's C1 on
    // the second copy's net: its near to U1:1 holds to that copy's U1,
    // not the first U1 in board order (trial 5's panel of copies).
    let (mut problem, _, _) = problem();
    problem.components[0].pins = vec![core::Pin {
        offset: [0.5, 0.0],
        net: 0,
    }];
    problem.components[2].pins = vec![core::Pin {
        offset: [0.5, 0.0],
        net: 1,
    }];
    problem.components[2].fixed = true;
    problem.components[1].pins = vec![core::Pin {
        offset: [0.0, 0.0],
        net: 1,
    }];
    let references = vec!["U1".to_string(), "C1".to_string(), "U1".to_string()];
    let pads = vec![
        BTreeMap::from([("1".to_string(), [0.5, 0.0])]),
        BTreeMap::new(),
        BTreeMap::from([("1".to_string(), [0.5, 0.0])]),
    ];
    let constraints: KiCadPlacementConstraints = serde_json::from_str(
        r#"{"version": 1, "near": [{"part": "C1", "pin_of": "U1:1", "max_mm": 1}], "relative": [{"part": "C1", "below": "U1"}]}"#,
    )
    .unwrap();
    apply_constraints(
        &mut problem,
        &references,
        &pads,
        &[Vec::new(), Vec::new(), Vec::new()],
        &[None, None, None],
        &[None, None, None],
        &[false, false, false],
        &[None, None, None],
        &constraints,
        10.0,
    )
    .unwrap();
    assert_eq!(
        problem.constraints.relations[0],
        Relation::Near {
            part: 1,
            anchor: Anchor::Point(2, [0.5, 0.0]),
            max: 1.0
        }
    );
    assert!(matches!(
        problem.constraints.relations[1],
        Relation::Beside {
            part: 1,
            anchor: 2,
            ..
        }
    ));
}

#[test]
fn a_row_is_placed_as_its_first_part() {
    let (problem, _) =
        apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}]}"#).unwrap();
    // C1 carries C2 three millimetres to its right: one 5 x 1 mm body.
    assert_eq!(problem.components[1].body_size, [5.0, 1.0]);
    assert_eq!(problem.components[1].body_center, [1.5, 0.0]);
    assert!(problem.components[2].fixed && problem.components[2].side == core::Side::Neither);
    assert_eq!(
        problem.constraints.followers,
        vec![core::constraints::Follower {
            part: 2,
            leader: 1,
            offset: [3.0, 0.0],
            angle: 0.0
        }]
    );
    assert_eq!(problem.poses[2].position, [13.0, 10.0]);
    // Only half turns keep the row along its axis.
    assert_eq!(problem.components[1].angle_options, vec![0.0, 180.0]);
    // A row's other parts are placed, with it: J1 alone is kept where
    // it is (trial 5's right_io counted its six row followers as
    // "kept where they are").
    assert_eq!(crate::board_placer::placed_count(&problem), 2);
    // A pair row (`pair_rows`) turns any quarter turn.
    let mut problem = problem_only();
    let (_, _, _) = apply_rows(
        &mut problem,
        &references(),
        3,
        &[Vec::new(), Vec::new(), Vec::new()],
        &[KiCadRowConstraint {
            parts: vec!["C1".into(), "C2".into()],
            pitch_mm: 1.0,
            axis: Some("y".into()),
            columns: None,
            row_pitch_mm: None,
            pair: true,
        }],
    )
    .unwrap();
    assert_eq!(
        problem.components[1].angle_options,
        vec![0.0, 90.0, 180.0, 270.0]
    );
    // Two lines of one: C2 below C1.
    let (problem, _) =
        apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3, "columns": 1, "row_pitch_mm": 4}]}"#).unwrap();
    assert_eq!(problem.constraints.followers[0].offset, [0.0, 4.0]);
    assert_eq!(problem.components[1].body_size, [2.0, 5.0]);
    for bad in [
        r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}], "near": [{"part": "C2", "part_of": "J1", "max_mm": 1}]}"#,
        r#"{"version": 1, "row": [{"parts": ["C1"], "pitch_mm": 3}]}"#,
        r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 0}]}"#,
        r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3, "axis": "z"}]}"#,
        r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}, {"parts": ["C2", "J1"], "pitch_mm": 3}]}"#,
    ] {
        assert!(apply(bad).is_err(), "{bad} was accepted");
    }
}

/// A row whose pitch puts two members' pads on each other is refused
/// before the run (trial 5: two 0402 capacitors 1.82 mm long at a
/// 1 mm pitch came back shorted by the layout).
#[test]
fn a_row_whose_pitch_puts_pads_on_pads_is_refused() {
    let grid = |row: &str| {
        let (mut problem, references, pads) = problem();
        let row: KiCadRowConstraint = serde_json::from_str(row).unwrap();
        let pad = vec![[-1.0, -0.5, -0.4, 0.5], [0.4, -0.5, 1.0, 0.5]];
        apply_rows(
            &mut problem,
            &references,
            pads.len(),
            &[Vec::new(), pad.clone(), pad],
            &[row],
        )
        .map(|_| ())
    };
    let rows = |pitch: f64| {
        grid(&format!(
            r#"{{"parts": ["C1", "C2"], "pitch_mm": {pitch}}}"#
        ))
    };
    let error = rows(1.0).unwrap_err();
    assert!(
        error.contains("the pads of C1 and C2 lie on each other (neighbours in a line)")
            && error.contains("they need 2.00 mm along the board's x (the pads' span), so pitch_mm must be at least that (now 1)"),
        "{error}"
    );
    assert!(rows(2.0).is_ok());
    // Two lines of a grid too close: the setting is row_pitch_mm, the
    // span the pads' across the axis (trial 8's bank of capacitors
    // was told about pitch_mm).
    let error =
        grid(r#"{"parts": ["C1", "C2"], "pitch_mm": 3, "columns": 1, "row_pitch_mm": 0.5}"#)
            .unwrap_err();
    assert!(
        error.contains("(lines 1 and 2 of the grid)")
            && error.contains("they need 1.00 mm along the board's y (the pads' span), so row_pitch_mm must be at least that (now 0.5)"),
        "{error}"
    );
    let error = grid(r#"{"parts": ["C1", "C2"], "pitch_mm": 0.5, "columns": 1}"#).unwrap_err();
    assert!(
        error.contains("(row_pitch_mm is not given: it takes pitch_mm)"),
        "{error}"
    );
    assert!(
        grid(r#"{"parts": ["C1", "C2"], "pitch_mm": 3, "columns": 1, "row_pitch_mm": 1.5}"#)
            .is_ok()
    );
    // Pads of two nets keep the copper clearance: every part turned
    // alike, a neighbour's pad 2 faces pad 1 (trial 8's bank: GND and
    // +BATT 0.1 mm apart, 16 clearance findings).
    let netted = |pitch: f64| {
        let (mut problem, references, pads) = problem();
        problem.min_spacing = 0.2;
        for member in [1, 2] {
            problem.components[member].pins = vec![
                core::Pin {
                    offset: [-0.7, 0.0],
                    net: 0,
                },
                core::Pin {
                    offset: [0.7, 0.0],
                    net: 1,
                },
            ];
        }
        let row: KiCadRowConstraint = serde_json::from_str(&format!(
            r#"{{"parts": ["C1", "C2"], "pitch_mm": {pitch}}}"#
        ))
        .unwrap();
        let pad = vec![[-1.0, -0.5, -0.4, 0.5], [0.4, -0.5, 1.0, 0.5]];
        apply_rows(
            &mut problem,
            &references,
            pads.len(),
            &[Vec::new(), pad.clone(), pad],
            &[row],
        )
        .map(|_| ())
    };
    let error = netted(2.1).unwrap_err();
    assert!(
        error.contains("of two nets come closer than the copper clearance (0.2 mm)")
            && error.contains("they need 2.20 mm along the board's x (the pads' span and 0.2 mm between pads of two nets)"),
        "{error}"
    );
    assert!(netted(2.25).is_ok());
}

#[test]
fn constraints_kept_by_construction_are_reported() {
    // A side, the turns a part may take, a connector opening outwards:
    // the placer cannot break them; the report lists them as kept.
    let (mut problem, references, pads) = problem();
    let constraints: KiCadPlacementConstraints = serde_json::from_str(
        r#"{"version": 1, "front": ["C1"], "rotation": [{"part": "C2", "angles": [90, 270]}],
            "edge": [{"part": "C1", "edge": "left", "opening_outwards": true}]}"#,
    )
    .unwrap();
    apply_constraints(
        &mut problem,
        &references,
        &pads,
        &[Vec::new(), Vec::new(), Vec::new()],
        &[None, None, None],
        // C1 opens towards -x in its own frame.
        &[None, Some([-1.0, 0.0]), None],
        &[false, false, false],
        &[None, None, None],
        &constraints,
        10.0,
    )
    .unwrap();
    let kept = |poses: &[core::Pose]| -> Vec<(String, String, bool)> {
        constraint_report(&problem, poses, &references)
            .into_iter()
            .filter(|status| status.kind != "edge")
            .map(|status| (status.kind, status.part, status.satisfied))
            .collect()
    };
    let row =
        |kind: &str, part: &str, satisfied: bool| (kind.to_string(), part.to_string(), satisfied);
    assert_eq!(
        kept(&problem.poses),
        vec![
            row("rotation", "C2", true),
            row("opening_outwards", "C1", true),
            row("front", "C1", true)
        ]
    );
    // Turned another way, they would be missed.
    let mut poses = problem.poses.clone();
    poses[1].angle = 180.0;
    poses[2].angle = 0.0;
    assert_eq!(
        kept(&poses),
        vec![
            row("rotation", "C2", false),
            row("opening_outwards", "C1", false),
            row("front", "C1", true)
        ]
    );

    // A footprint `back` put on B.Cu whose pads are on the front (a key
    // socket under its switch) lies on the front: kept all the same.
    let (mut socket, references, pads) = self::problem();
    let constraints: KiCadPlacementConstraints =
        serde_json::from_str(r#"{"version": 1, "back": ["C1"]}"#).unwrap();
    assert_eq!(socket.components[1].side, core::Side::Front);
    apply_constraints(
        &mut socket,
        &references,
        &pads,
        &[Vec::new(), Vec::new(), Vec::new()],
        &[None, None, None],
        &[None, None, None],
        &[false, false, false],
        &[None, None, None],
        &constraints,
        10.0,
    )
    .unwrap();
    let statuses: Vec<(String, bool)> = constraint_report(&socket, &socket.poses, &references)
        .into_iter()
        .map(|status| (status.kind, status.satisfied))
        .collect();
    assert_eq!(statuses, vec![("back".to_string(), true)]);
}

#[test]
fn an_overhang_without_a_keepout_says_how_to_keep_the_antenna_at_the_edge() {
    // Trial 6: an ESP32 module's footprint without an antenna keepout.
    let error = apply(
        r#"{"version": 1, "edge": [{"part": "J1", "edge": "bottom", "flush": true, "overhang": {"edge": "bottom"}}]}"#,
    )
    .unwrap_err();
    assert!(
        error.contains("edge constraint for J1: overhang needs a keepout zone in the footprint (the part that sticks out), and this footprint has none: drop the overhang and keep the antenna end at the edge with the edge entry alone"),
        "{error}"
    );
}

#[test]
fn a_row_the_outline_cannot_hold_is_an_error_at_once() {
    // Two 2 x 1 mm parts 25 mm apart: 27 mm on a 30 mm board; 40 apart
    // do not fit, turned by a half or not (a row turns no further).
    assert!(apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 25}]}"#).is_ok());
    let error = apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 40}]}"#).unwrap_err();
    assert!(
        error
            .contains("the row of C1 (2 parts) cannot fit the outline: the row is 42.00 x 1.00 mm"),
        "{error}"
    );
    assert!(!error.contains("turned a quarter"), "{error}");
    // 28.5 apart: 30.5 mm as they stand, 29.5 turned a quarter each
    // (trial 6's jacks down a eurorack board): the refusal says how.
    let error =
        apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 28.5}]}"#).unwrap_err();
    assert!(
        error.contains("; its parts keep the angle they have on the board and turn with the row as one, and turned a quarter from its line they fit: give the row the other axis and C1 a rotation a quarter turn from its angle now"),
        "{error}"
    );
    // The room it would need is noted for the verdict (other tests may
    // note theirs in between: only that there is one).
    assert!(crate::placement_room::noted_room().is_some());
    // With a region too small as well, both are said.
    let error = apply(
        r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 40}], "region": [{"parts": ["J1"], "x": [0, 1], "y": [0, 0.5]}]}"#,
    )
    .unwrap_err();
    assert!(
        error.contains("region for J1 cannot hold it") && error.contains("the row of C1"),
        "{error}"
    );
}

#[test]
fn a_turned_row_keeps_its_members() {
    // A board placed before: the placer turned the row (C1 at 180
    // degrees, C2 three millimetres to its left). Lowered again, the
    // row runs the way it lies; it used to run to the right and pull
    // C2 there.
    let (mut problem, references, pads) = problem();
    problem.poses[1] = core::Pose {
        position: [10.0, 10.0],
        angle: 180.0,
    };
    problem.poses[2] = core::Pose {
        position: [7.0, 10.0],
        angle: 180.0,
    };
    let constraints: KiCadPlacementConstraints =
        serde_json::from_str(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}]}"#)
            .unwrap();
    apply_constraints(
        &mut problem,
        &references,
        &pads,
        &[Vec::new(), Vec::new(), Vec::new()],
        &[None, None, None],
        &[None, None, None],
        &[false, false, false],
        &[None, None, None],
        &constraints,
        10.0,
    )
    .unwrap();
    assert_eq!(problem.poses[2].position, [7.0, 10.0]);
    let center = problem.components[1].center(problem.poses[1]);
    assert!(
        (center[0] - 8.5).abs() < 1.0e-9 && (center[1] - 10.0).abs() < 1.0e-9,
        "{center:?}"
    );
}

#[test]
fn a_placed_row_that_runs_off_the_board_is_refused() {
    // Trial 6: the row's first part placed at the top, the row running
    // up from it off the board, accepted without a word.
    assert!(
        apply(
            r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}], "place": [{"part": "C1", "x": 5, "y": 10}]}"#
        )
        .is_ok()
    );
    let error = apply(
        r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}], "place": [{"part": "C1", "x": 29, "y": 10}]}"#,
    )
    .unwrap_err();
    assert!(
        error.contains("the row of C1 runs from where its place entry puts it (29.00, 10.00, 0 degrees) along its axis in that part's frame and reaches 3.0 mm beyond the right edge: its other parts would lie off the board"),
        "{error}"
    );
}

#[test]
fn a_row_turned_a_quarter_keeps_its_members() {
    // Trial 6: a row along x whose first part has rotation 90 (eight
    // jacks down a eurorack board), placed; the layout lowers the placed
    // board again. C1 at 90 degrees, C2 three millimetres above it.
    let (mut problem, references, pads) = problem();
    problem.poses[1] = core::Pose {
        position: [10.0, 10.0],
        angle: 90.0,
    };
    problem.poses[2] = core::Pose {
        position: [10.0, 7.0],
        angle: 90.0,
    };
    let constraints: KiCadPlacementConstraints =
        serde_json::from_str(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}]}"#)
            .unwrap();
    apply_constraints(
        &mut problem,
        &references,
        &pads,
        &[Vec::new(), Vec::new(), Vec::new()],
        &[None, None, None],
        &[None, None, None],
        &[false, false, false],
        &[None, None, None],
        &constraints,
        10.0,
    )
    .unwrap();
    // The row's body in its first part's frame: the two 2 x 1 mm parts
    // 3 mm apart along it, as when it was built from the stack.
    assert_eq!(problem.components[1].body_size, [5.0, 1.0]);
    let mut poses = problem.poses.clone();
    problem.sync_followers(&mut poses);
    let at = poses[2].position;
    assert!(
        (at[0] - 10.0).abs() < 1.0e-9 && (at[1] - 7.0).abs() < 1.0e-9,
        "{at:?}"
    );
}

#[test]
fn near_may_name_a_row_member_as_its_anchor() {
    // D1 near C2, which the row C1, C2 carries: the anchor is C2's
    // place in the row (its body, or its pad), on the row's first part.
    let (mut problem, mut references, mut pads) = problem();
    problem.components.push(problem.components[1].clone());
    problem.poses.push(core::Pose {
        position: [10.0, 10.0],
        angle: 0.0,
    });
    references.push("D1".into());
    pads[2].insert("1".into(), [0.5, 0.0]);
    pads.push(BTreeMap::new());
    let constraints: KiCadPlacementConstraints = serde_json::from_str(
        r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}],
            "near": [{"part": "D1", "part_of": "C2", "max_mm": 1}, {"part": "D1", "pin_of": "C2:1", "max_mm": 1}]}"#,
    )
    .unwrap();
    apply_constraints(
        &mut problem,
        &references,
        &pads,
        &[Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        &[None, None, None, None],
        &[None, None, None, None],
        &[false, false, false, false],
        &[None, None, None, None],
        &constraints,
        10.0,
    )
    .unwrap();
    assert_eq!(
        problem.constraints.relations,
        vec![
            Relation::Near {
                part: 3,
                anchor: Anchor::Area(1, 2, [3.0, 0.0], [1.0, 0.5]),
                max: 1.0
            },
            Relation::Near {
                part: 3,
                anchor: Anchor::Area(1, 2, [3.5, 0.0], [0.0, 0.0]),
                max: 1.0
            },
        ]
    );
    assert!(apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}], "near": [{"part": "C2", "part_of": "J1", "max_mm": 1}]}"#).is_err());
}

#[test]
fn side_constraints_flip_parts_onto_their_side() {
    let footprint = |reference: &str, layer: &str| {
        format!(
            "(footprint \"R\" (layer \"{layer}\") (at 5 5) (property \"Reference\" \"{reference}\" (at 0 -1 0) (layer \"F.SilkS\")) \
             (pad \"1\" smd rect (at 1 1) (size 1 1) (layers \"{layer}\")))"
        )
    };
    let board = format!(
        "(kicad_pcb {} {} {})",
        footprint("R1", "F.Cu"),
        footprint("R2", "F.Cu"),
        footprint("C1", "B.Cu")
    );
    let sides = |json: &str| -> Result<Vec<String>, String> {
        let mut pcb = parse(&board).unwrap();
        apply_sides(&mut pcb, &serde_json::from_str(json).unwrap())?;
        Ok(pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .map(|item| form_atom(item, "layer", 1).unwrap().to_string())
            .collect())
    };
    assert_eq!(
        sides(r#"{"version": 1, "back": ["R*"], "front": ["C1"]}"#).unwrap(),
        ["B.Cu", "B.Cu", "F.Cu"]
    );
    // A part already on its side stays as it is.
    assert_eq!(
        sides(r#"{"version": 1, "back": ["C1", "R2"]}"#).unwrap(),
        ["F.Cu", "B.Cu", "B.Cu"]
    );
    assert!(sides(r#"{"version": 1, "back": ["R*"], "front": ["R1"]}"#).is_err());
    assert!(sides(r#"{"version": 1, "back": ["U1"]}"#).is_err());
}

#[test]
fn keepouts_become_rule_areas_and_replace_earlier_ones() {
    let mut pcb = parse(
        "(kicad_pcb (gr_rect (start 10 20) (end 60 60) (stroke (width 0.05) (type solid)) (fill no) (layer \"Edge.Cuts\")))",
    )
    .unwrap();
    let keepouts = |json: &str| -> Vec<KiCadKeepoutConstraint> {
        serde_json::from_str::<KiCadPlacementConstraints>(json)
            .unwrap()
            .keepout
    };
    let zones = |pcb: &Expr| -> Vec<Expr> {
        pcb.children()
            .iter()
            .filter(|item| item.head() == Some("zone"))
            .cloned()
            .collect()
    };
    let held = &HeldParts::default();
    apply_keepouts(
        &mut pcb,
        &keepouts(r#"{"version": 1, "keepout": [{"x": [0, 5], "y": [2, 4], "copper": true}, {"x": [1, 2], "y": [1, 2], "side": "back"}]}"#),
        held,
    )
    .unwrap();
    let written = zones(&pcb);
    assert_eq!(written.len(), 2);
    // Relative to the outline's corner; copper keepouts block tracks on every layer.
    let text = encode(&written[0]).to_string();
    assert!(
        text.contains("(xy 10 22)") && text.contains("(xy 15 24)"),
        "{text}"
    );
    assert!(
        text.contains("\"*.Cu\"") && text.contains("(tracks not_allowed)"),
        "{text}"
    );
    let text = encode(&written[1]).to_string();
    assert!(
        text.contains("\"B.Cu\"")
            && text.contains("(tracks allowed)")
            && text.contains("(footprints not_allowed)")
    );
    // Applying again replaces them; nothing removes them all.
    apply_keepouts(
        &mut pcb,
        &keepouts(r#"{"version": 1, "keepout": [{"x": [0, 5], "y": [0, 5]}]}"#),
        held,
    )
    .unwrap();
    assert_eq!(zones(&pcb).len(), 1);
    apply_keepouts(&mut pcb, &[], held).unwrap();
    assert!(zones(&pcb).is_empty());
    for bad in [
        r#"{"version": 1, "keepout": [{"x": [0, 0], "y": [0, 5]}]}"#,
        r#"{"version": 1, "keepout": [{"x": [0, 1], "y": [0, 5], "side": "top"}]}"#,
        r#"{"version": 1, "keepout": [{"x": [0, 1], "y": [0, 5], "origin": "center"}]}"#,
    ] {
        assert!(
            apply_keepouts(&mut pcb, &keepouts(bad), held).is_err(),
            "{bad} was accepted"
        );
    }
}

#[test]
fn one_notion_of_a_held_part_covers_every_case() {
    // A 50 x 40 mm board; each part is held or not for one reason.
    let part = |reference: &str, at: [f64; 2], extra: &str, net: bool| {
        format!(
            r#"(footprint "p" (layer "F.Cu") (at {} {}) {extra} (property "Reference" "{reference}")
          (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
          (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu"){}))"#,
            at[0],
            at[1],
            if net { " (net 1 \"A\")" } else { "" }
        )
    };
    let header = r#"(footprint "hdr" (layer "F.Cu") (at 30 21.5) (property "Reference" "J1")
          (fp_rect (start -6 -1.2) (end 6 1.2) (layer "F.CrtYd"))
          (pad "1" thru_hole circle (at -3.81 0) (size 1.7 1.7) (drill 1) (layers "*.Cu") (net 1 "A"))
          (pad "2" thru_hole circle (at -1.27 0) (size 1.7 1.7) (drill 1) (layers "*.Cu") (net 1 "A"))
          (pad "3" thru_hole circle (at 1.27 0) (size 1.7 1.7) (drill 1) (layers "*.Cu") (net 1 "A"))
          (pad "4" thru_hole circle (at 3.81 0) (size 1.7 1.7) (drill 1) (layers "*.Cu") (net 1 "A")))"#;
    let parts = [
        part("R1", [30.0, 40.0], "", true),             // plain: moves
        part("F1", [20.0, 40.0], "", true),             // placer fixed
        part("P1", [25.0, 40.0], "", true),             // fixed_patterns
        part("C1", [35.0, 40.0], "", true),             // constraints fixed
        part("X1", [40.0, 40.0], "", true),             // place with x and y
        part("X2", [45.0, 40.0], "", true),             // place by a word
        part("L1", [30.0, 30.0], "(locked yes)", true), // locked
        part("L2", [35.0, 30.0], "(locked yes)", true), // locked, on an edge constraint
        part("N1", [40.0, 30.0], "", false),            // no net
        part("N2", [45.0, 30.0], "", false),            // no net, free
        part("E1", [11.0, 40.0], "", true),             // body over the outline
        header.to_string(),                             // a connector at the edge
    ];
    let text = format!(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 10 20) (end 60 60) (layer "Edge.Cuts")) {})"#,
        parts.join("\n")
    );
    let pcb = parse(&text).unwrap();
    let constraints: KiCadPlacementConstraints = serde_json::from_value(serde_json::json!({
        "version": 1,
        "fixed": ["C1"],
        "place": [{"part": "X1", "x": 30, "y": 20}, {"part": "X2", "at": "center"}],
        "edge": [{"part": "L2", "edge": "any"}]
    }))
    .unwrap();
    let config = KiCadBoardPlacerConfig {
        fixed: vec!["F1".into()],
        fixed_patterns: vec!["P*".into()],
        free: vec!["N2".into()],
        constraints: Some(KiCadConstraintsSource::Inline(constraints.clone())),
        ..Default::default()
    };
    let held_names = |held: &HeldParts| -> Vec<String> {
        pcb.children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .enumerate()
            .filter(|(index, _)| held.holds(*index))
            .map(|(_, footprint)| footprint_reference(footprint).unwrap_or_default())
            .collect()
    };
    assert_eq!(
        held_names(&held_parts(&pcb, &config).unwrap()),
        ["F1", "P1", "C1", "X1", "L1", "N1", "E1", "J1"]
    );
    // A row's members go with its leader: N1 (no net) behind the
    // movable R1 moves with it.
    let mut in_row = constraints.clone();
    in_row.row =
        serde_json::from_value(serde_json::json!([{"parts": ["R1", "N1"], "pitch_mm": 3}]))
            .unwrap();
    let row_config = KiCadBoardPlacerConfig {
        constraints: Some(KiCadConstraintsSource::Inline(in_row)),
        ..config.clone()
    };
    assert_eq!(
        held_names(&held_parts(&pcb, &row_config).unwrap()),
        ["F1", "P1", "C1", "X1", "L1", "E1", "J1"]
    );
    // move_all: every part is placed but locked ones and those
    // constraints.json fixes or places (docs/constraints.md); the
    // placer's own `fixed` list and the defaults go too.
    let mut all = constraints;
    all.move_all = true;
    let config = KiCadBoardPlacerConfig {
        constraints: Some(KiCadConstraintsSource::Inline(all)),
        ..config
    };
    assert_eq!(
        held_names(&held_parts(&pcb, &config).unwrap()),
        ["C1", "X1", "L1"]
    );
}

#[test]
fn a_keepout_over_a_fixed_part_allows_footprints_and_warns() {
    // J2 (fixed) reaches 1 mm into the first box; R1 (movable) lies in
    // the second; H1 (no net) is in the third, on the back only.
    let board = r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 10 20) (end 60 60) (layer "Edge.Cuts"))
        (footprint "conn" (layer "F.Cu") (at 20 25 90) (property "Reference" "J2")
          (fp_rect (start -1 -3) (end 1 3) (layer "F.CrtYd"))
          (pad "1" thru_hole circle (at 0 0) (size 1.7 1.7) (drill 1) (layers "*.Cu") (net 1 "A")))
        (footprint "r" (layer "F.Cu") (at 40 40) (property "Reference" "R1")
          (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
          (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "A")))
        (footprint "hole" (layer "B.Cu") (at 30 50) (property "Reference" "H1")
          (fp_circle (center 0 0) (end 2 0) (layer "B.CrtYd"))
          (pad "" np_thru_hole circle (at 0 0) (size 3 3) (drill 3) (layers "*.Cu"))))"#;
    let keepouts: Vec<KiCadKeepoutConstraint> = serde_json::from_str::<KiCadPlacementConstraints>(
        r#"{"version": 1, "keepout": [
            {"x": [12, 20], "y": [0, 4]},
            {"x": [25, 35], "y": [15, 25], "copper": true},
            {"x": [15, 25], "y": [25, 35], "side": "back"}]}"#,
    )
    .unwrap()
    .keepout;
    let mut pcb = parse(board).unwrap();
    let config = KiCadBoardPlacerConfig {
        fixed: vec!["J2".into()],
        ..Default::default()
    };
    let held = &held_parts(&pcb, &config).unwrap();
    // J2 at (20, 25) turned a quarter: its courtyard is x 17..23, y
    // 24..26 on the board; the first box (from the corner (10, 20)) x
    // 22..30, y 20..24 only touches it. H1 (no net: held) lies in the
    // third, R1 (movable) in the second.
    let inside = held_parts_in_keepouts(&pcb, &keepouts, held).unwrap();
    assert_eq!(
        inside,
        [Vec::<String>::new(), Vec::new(), vec!["H1".to_string()]]
    );
    let warnings = apply_keepouts(&mut pcb, &keepouts, held).unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].starts_with("keepout 3 ") && warnings[0].contains("(H1)"),
        "{}",
        warnings[0]
    );
    let zones: Vec<String> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
        .map(|zone| encode(zone).to_string())
        .collect();
    assert!(zones[0].contains("(footprints not_allowed)"));
    assert!(
        zones[1].contains("(footprints not_allowed)") && zones[1].contains("(tracks not_allowed)")
    );
    assert!(zones[2].contains("(footprints allowed)"), "{}", zones[2]);
    // A box reaching into the fixed J2's courtyard: footprints allowed;
    // copper stays out.
    let over: Vec<KiCadKeepoutConstraint> = serde_json::from_str::<KiCadPlacementConstraints>(
        r#"{"version": 1, "keepout": [{"x": [12, 20], "y": [0, 4.5], "copper": true}]}"#,
    )
    .unwrap()
    .keepout;
    let warnings = apply_keepouts(&mut pcb, &over, held).unwrap();
    assert!(
        warnings[0].contains("(J2)") && warnings[0].contains("tracks, vias and pours stay out")
    );
    let zone = pcb
        .children()
        .iter()
        .find(|item| item.head() == Some("zone"))
        .map(|zone| encode(zone).to_string())
        .unwrap();
    assert!(zone.contains("(footprints allowed)") && zone.contains("(tracks not_allowed)"));
    // Not fixed: the movable part is the placer's to move.
    let movable = KiCadBoardPlacerConfig {
        free: vec!["H1".into()],
        ..Default::default()
    };
    let movable = held_parts(&pcb, &movable).unwrap();
    assert!(
        apply_keepouts(&mut pcb, &over, &movable)
            .unwrap()
            .is_empty()
    );
    // Fixed by constraints.json: held as by the placer's own list (the
    // fence's issue 15 for that input).
    let constraints: KiCadPlacementConstraints =
        serde_json::from_str(r#"{"version": 1, "fixed": ["J2"]}"#).unwrap();
    let by_constraints = KiCadBoardPlacerConfig {
        free: vec!["H1".into()],
        constraints: Some(KiCadConstraintsSource::Inline(constraints)),
        ..Default::default()
    };
    let by_constraints = held_parts(&pcb, &by_constraints).unwrap();
    let warnings = apply_keepouts(&mut pcb, &over, &by_constraints).unwrap();
    assert!(warnings[0].contains("(J2)"), "{warnings:?}");
}

#[test]
fn the_input_check_warns_of_a_held_part_in_a_keepout_with_an_outline_too() {
    // H1 (no net: held) on the back at (30, 50), the outline's corner
    // at (10, 20): the box x 15..25, y 25..35 from the corner holds it.
    // The import writes an outline; the check applies it first, as
    // the placement does.
    let directory =
        std::env::temp_dir().join(format!("pcb-maker-keepout-warnings-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("b.kicad_pcb"),
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 10 20) (end 60 60) (layer "Edge.Cuts"))
        (footprint "hole" (layer "B.Cu") (at 30 50) (property "Reference" "H1")
          (fp_circle (center 0 0) (end 2 0) (layer "B.CrtYd"))
          (pad "" np_thru_hole circle (at 0 0) (size 3 3) (drill 3) (layers "*.Cu"))))"#,
    )
    .unwrap();
    for outline in ["", r#""outline": {"width": 50, "height": 40},"#] {
        let constraints: KiCadPlacementConstraints = serde_json::from_str(&format!(
            r#"{{"version": 1, {outline} "keepout": [{{"x": [15, 25], "y": [25, 35]}}]}}"#
        ))
        .unwrap();
        let config = KiCadBoardPlacerConfig {
            constraints: Some(KiCadConstraintsSource::Inline(constraints)),
            ..KiCadBoardPlacerConfig::default()
        };
        let warnings = keepout_warnings(&directory, "b", &config).unwrap();
        assert_eq!(warnings.len(), 1, "{outline}: {warnings:?}");
        assert!(warnings[0].contains("(H1)"), "{}", warnings[0]);
    }
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn fixed_parts_at_one_spot_are_warned_of() {
    // C1 and C2 fixed at one spot; C3 fixed beside them (courtyards
    // touch); R1 movable on top of C1; U1 on the back under C1 (its
    // courtyard on the other side, its pads on B.Cu).
    let part = |reference: &str, x: f64, y: f64, layer: &str| {
        format!(
            r#"(footprint "c" (layer "{layer}.Cu") (at {x} {y}) (property "Reference" "{reference}")
              (fp_rect (start -1 -0.5) (end 1 0.5) (layer "{layer}.CrtYd"))
              (pad "1" smd rect (at -0.5 0) (size 0.6 0.6) (layers "{layer}.Cu") (net "{reference}-A"))
              (pad "2" smd rect (at 0.5 0) (size 0.6 0.6) (layers "{layer}.Cu") (net "{reference}-B")))"#
        )
    };
    let board = format!(
        "(kicad_pcb (layers (0 \"F.Cu\" signal) (31 \"B.Cu\" signal)) (gr_rect (start 0 0) (end 30 20) (layer \"Edge.Cuts\")) {} {} {} {} {})",
        part("C1", 10.0, 10.0, "F"),
        part("C2", 10.0, 10.0, "F"),
        part("C3", 12.0, 10.0, "F"),
        part("R1", 10.0, 10.0, "F"),
        part("U1", 10.0, 10.0, "B"),
    );
    let pcb = parse(&board).unwrap();
    let fixed = [
        "C1".to_string(),
        "C2".to_string(),
        "C3".to_string(),
        "U1".to_string(),
    ];
    let config = KiCadBoardPlacerConfig {
        fixed: fixed.to_vec(),
        ..Default::default()
    };
    let held = held_parts(&pcb, &config).unwrap();
    let pairs = held_parts_overlapping(&pcb, &held).unwrap();
    assert_eq!(
        pairs,
        [(
            "C1".to_string(),
            "C2".to_string(),
            "courtyards and pads on F.Cu".to_string()
        )]
    );
    let warning = overlap_warning(&pairs[0].0, &pairs[0].1, &pairs[0].2);
    assert!(
        warning.starts_with("fixed parts C1 and C2 overlap"),
        "{warning}"
    );
    // Placed with x and y elsewhere, C2's input position is not where
    // it will be held: nothing to warn of here (the lowering checks it
    // where the entry puts it).
    let constraints: KiCadPlacementConstraints =
        serde_json::from_str(r#"{"version": 1, "place": [{"part": "C2", "x": 25, "y": 15}]}"#)
            .unwrap();
    let held = held_parts(&pcb, &config).unwrap();
    let held = without_parts_placed_elsewhere(&pcb, held, &constraints);
    assert!(held_parts_overlapping(&pcb, &held).unwrap().is_empty());
    // In a row whose first part is placed so, C2 goes with the row
    // (trial 6: eight jacks in a row led by a placed J4, 21 warnings).
    let constraints: KiCadPlacementConstraints = serde_json::from_str(
        r#"{"version": 1, "place": [{"part": "C3", "x": 25, "y": 15}], "row": [{"parts": ["C3", "C2"], "pitch_mm": 3}]}"#,
    )
    .unwrap();
    let held = held_parts(&pcb, &config).unwrap();
    let held = without_parts_placed_elsewhere(&pcb, held, &constraints);
    assert!(held_parts_overlapping(&pcb, &held).unwrap().is_empty());
    // Not fixed, C2 is the placer's to move: nothing to warn of.
    let fixed = ["C1".to_string(), "C3".to_string(), "U1".to_string()];
    let config = KiCadBoardPlacerConfig {
        fixed: fixed.to_vec(),
        ..Default::default()
    };
    let held = held_parts(&pcb, &config).unwrap();
    assert!(held_parts_overlapping(&pcb, &held).unwrap().is_empty());
}

#[test]
fn fixed_pads_whose_boxes_meet_but_not_their_copper_are_not_warned_of() {
    // framework_mobo_lefthalf: R31's 0603 pad under J1's oval
    // through-hole pad, 0.087 mm apart by KiCad, their boxes 0.2 mm
    // into each other. J2 moved 0.3 mm nearer: the copper meets.
    let board = |oval_x: f64| {
        format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (gr_rect (start 0 0) (end 30 20) (layer "Edge.Cuts"))
              (footprint "r" (layer "B.Cu") (at 10 10) (property "Reference" "R31")
                (pad "1" smd rect (at 0 0) (size 0.8 0.95) (layers "B.Cu") (net 1 "A")))
              (footprint "j" (layer "F.Cu") (at {oval_x} 10.675) (property "Reference" "J1")
                (pad "B6" thru_hole oval (at 0 0 90) (size 0.6 1.2) (drill oval 0.3 0.9) (layers "*.Cu") (net 2 "B"))))"#
        )
    };
    let config = KiCadBoardPlacerConfig {
        fixed: vec!["R31".to_string(), "J1".to_string()],
        ..Default::default()
    };
    // The 0603 pad's corner at (10.4, 10.475); the oval (radius 0.3,
    // its axis from x - 0.3 to x + 0.3 at y 10.675) has its box 0.05
    // mm past the corner on both axes, its copper 0.02 mm away at
    // x 10.95 (the end's centre at 10.65) and 0.1 mm into the pad at
    // x 10.6.
    let far = parse(&board(10.95)).unwrap();
    let held = held_parts(&far, &config).unwrap();
    assert!(
        held_parts_overlapping(&far, &held).unwrap().is_empty(),
        "boxes meet, copper apart"
    );
    let near = parse(&board(10.6)).unwrap();
    let held = held_parts(&near, &config).unwrap();
    assert_eq!(
        held_parts_overlapping(&near, &held).unwrap(),
        [(
            "J1".to_string(),
            "R31".to_string(),
            "pads on *.Cu".to_string()
        )]
    );
    // The same pads on one net: R31's pad 1 soldered onto J1's B5,
    // as the designer drew it.
    let joined = parse(&board(10.6).replace(r#"(net 2 "B")"#, r#"(net 1 "A")"#)).unwrap();
    let held = held_parts(&joined, &config).unwrap();
    assert!(held_parts_overlapping(&joined, &held).unwrap().is_empty());
}

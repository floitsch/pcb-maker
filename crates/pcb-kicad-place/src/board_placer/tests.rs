// Copyright (C) 2026 Toit contributors.

//! Unit tests of the placer's lowering, rules and races.

use super::*;
use pcb_placer as core;

#[test]
fn placement_seeds_are_judged_in_seed_order_whatever_the_threads() {
    let works = [50u64, 10, 70, 20, 30, 5, 40, 60, 25, 15, 35, 45];
    // One seed after the other: kept while the seeds before used less
    // than the limit, three at least.
    let sequential = |limit: Option<u64>| -> Vec<u64> {
        let mut kept = Vec::new();
        let mut used = 0;
        for (seed, work) in works.iter().enumerate() {
            if kept.len() >= 3 && limit.is_some_and(|limit| used >= limit) {
                break;
            }
            used += work;
            kept.push(seed as u64);
        }
        kept
    };
    for limit in [None, Some(1), Some(100), Some(150), Some(200), Some(10_000)] {
        let mut outcomes = Vec::new();
        for threads in [1, 2, 3, 4, 8, 16] {
            let ran = std::sync::Mutex::new(Vec::new());
            let race = race_seeds(
                works.len() as u64,
                3,
                limit,
                threads,
                |result: &(u64, u64)| result.1,
                |seed| {
                    // Later seeds finish first.
                    std::thread::sleep(std::time::Duration::from_micros(
                        300 * (works.len() as u64 - seed),
                    ));
                    ran.lock().unwrap().push(seed);
                    (seed, works[seed as usize])
                },
            );
            let kept: Vec<u64> = race.kept.iter().map(|(offset, _)| *offset).collect();
            assert_eq!(
                kept,
                sequential(limit),
                "limit {limit:?}, {threads} threads"
            );
            assert!(race.kept.iter().all(|(offset, (seed, _))| offset == seed));
            let mut ran = ran.into_inner().unwrap();
            ran.sort();
            assert_eq!(ran.len() as u64, race.run);
            outcomes.push((ran, race.pool_work));
        }
        assert!(
            outcomes.windows(2).all(|pair| pair[0] == pair[1]),
            "limit {limit:?}: {outcomes:?}"
        );
    }
    // Up to the limit: the kept seeds and no more ran.
    let race = race_seeds(
        works.len() as u64,
        3,
        Some(150),
        4,
        |result: &u64| *result,
        |seed| works[seed as usize],
    );
    assert_eq!(race.kept.len(), 4);
    assert_eq!(race.pool_work, 70 + 20);
    assert_eq!(pool_makespan(&[4, 4, 4], REFERENCE_THREADS), 4);
    assert_eq!(pool_makespan(&[3; 9], REFERENCE_THREADS), 6);
    assert_eq!(pool_makespan(&[10, 1, 1, 1, 1, 1, 1, 1, 1, 1], 8), 10);
}

#[test]
fn nets_named_for_a_switch_node_feedback_rf_or_crystal_are_kept_short() {
    for name in [
        "Net-(U9-FB)",
        "Net-(U1-SW)",
        "/RF/ANT",
        "Net-(J2-RF)",
        "Net-(U3-XOUT)",
        "/Supply/5V_BOOT",
    ] {
        assert!(keep_short(name), "{name}");
    }
    // A key switch, a supply, a signal that merely contains the letters.
    for name in [
        "Net-(SW1-Pad2)",
        "+3V3",
        "/SWDIO",
        "Net-(U4-COL0)",
        "/FBUS_TX",
    ] {
        assert!(!keep_short(name), "{name}");
    }
}

#[test]
fn a_copper_graphic_on_several_layers_bounds_the_edge_inset_too() {
    // A part whose courtyard reaches 2 mm past its pad on every side, with
    // a copper polygon 1 mm short of the courtyard on the left: listed
    // on one copper layer (KiCad 8) or with its mask (KiCad 9's
    // `layers`), the copper stays 1 mm inside the body, not 2.
    let inset = |layers: &str| {
        let pcb = parse(&format!(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
            (footprint "Tab" (layer "F.Cu") (at 20 10)
                (fp_rect (start -2.5 -2.5) (end 2.5 2.5) (layer "F.CrtYd"))
                (fp_poly (pts (xy -1.5 -0.5) (xy 0 -0.5) (xy 0 0.5) (xy -1.5 0.5)) {layers} (width 0))
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A"))))"#
        ))
        .unwrap();
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
        lowered.problem.components[0].edge_inset
    };
    assert!((inset(r#"(layer "F.Cu")"#) - 1.0).abs() < 1.0e-9);
    assert!((inset(r#"(layers "F.Cu" "F.Mask")"#) - 1.0).abs() < 1.0e-9);
    assert!((inset(r#"(layer "F.SilkS")"#) - 2.0).abs() < 1.0e-9);
}

#[test]
fn a_front_footprints_copper_on_the_back_blocks_back_parts_there() {
    // A netless logo on the front whose copper polygon lies on B.Cu
    // (Castor's LOGO), and a part with a courtyard whose B.Mask graphic
    // opens the back's mask; a back 0603 between two nets.
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
        (footprint "LOGO" (layer "F.Cu") (at 10 10)
            (fp_poly (pts (xy -2 -2) (xy 2 -2) (xy 2 2) (xy -2 2)) (layer "B.Cu") (width 0)))
        (footprint "Marked" (layer "F.Cu") (at 30 10)
            (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
            (fp_poly (pts (xy -3 -3) (xy 3 -3) (xy 3 3) (xy -3 3)) (layer "B.Mask") (width 0))
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
        (footprint "L_0603" (layer "B.Cu") (at 20 3)
            (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "B.CrtYd"))
            (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "B.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "B.Cu") (net 2 "B"))))"#,
    )
    .unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let problem = &lowered.problem;
    let logo = &problem.components[0];
    assert_eq!(
        logo.side,
        core::Side::Back,
        "artwork occupies the side of its copper"
    );
    assert_eq!(problem.components[1].side, core::Side::Front);
    assert_eq!(
        problem.components[1].far_copper,
        vec![[-3.0, -3.0, 3.0, 3.0]]
    );
    let part = 2;
    assert_eq!(problem.components[part].side, core::Side::Back);
    let mut poses = problem.poses.clone();
    assert!(!core::legal::illegal_components(problem, &poses).contains(&part));
    // Over the logo's copper, and under the other part's mask opening.
    for position in [[10.0, 10.5], [30.0, 12.0]] {
        poses[part].position = position;
        assert!(
            core::legal::illegal_components(problem, &poses).contains(&part),
            "{position:?}"
        );
    }
    // A front part may sit over the logo: its copper is on the back.
    let mut front = problem.components[part].clone();
    front.side = core::Side::Front;
    let mut flipped = problem.clone();
    flipped.components[part] = front;
    poses[part].position = [10.0, 10.5];
    assert!(!core::legal::illegal_components(&flipped, &poses).contains(&part));
}

#[test]
fn rule_areas_keep_out_what_they_forbid() {
    // A back rule area that allows footprints but forbids pads (d20),
    // and a fixed back module whose own keepout forbids footprints on
    // both sides (PixelWave's ESP32 antenna area).
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
        (zone (layers "B.Cu") (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints allowed))
            (polygon (pts (xy 2 2) (xy 20 2) (xy 20 28) (xy 2 28))))
        (footprint "Module" (layer "B.Cu") (at 40 10) (locked yes)
            (fp_rect (start -3 -3) (end 3 3) (layer "B.CrtYd"))
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu") (net 1 "A"))
            (zone (layers "F.Cu" "B.Cu") (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints not_allowed))
                (polygon (pts (xy 35 14) (xy 45 14) (xy 45 24) (xy 35 24)))))
        (footprint "Back" (layer "B.Cu") (at 30 3)
            (fp_rect (start -5 -0.7) (end 5 0.7) (layer "B.CrtYd"))
            (pad "1" smd rect (at -2.5 0) (size 0.8 0.9) (layers "B.Cu") (net 1 "A"))
            (pad "2" smd rect (at 2.5 0) (size 0.8 0.9) (layers "B.Cu") (net 2 "B")))
        (footprint "Front" (layer "F.Cu") (at 30 27)
            (fp_rect (start -1 -0.7) (end 1 0.7) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.5 0) (size 0.6 0.9) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.5 0) (size 0.6 0.9) (layers "F.Cu") (net 2 "B"))))"#,
    )
    .unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let problem = &lowered.problem;
    let (back, front) = (1, 2);
    let mut poses = problem.poses.clone();
    assert!(core::legal::illegal_components(problem, &poses).is_empty());
    let legal = |poses: &[core::Pose], part: usize| {
        !core::legal::illegal_components(problem, poses).contains(&part)
    };
    // Pads in the pad keepout: not legal; the body reaching over its
    // edge with the pads outside: legal.
    poses[back].position = [10.0, 15.0];
    assert!(!legal(&poses, back));
    poses[back].position = [23.5, 15.0];
    assert!(legal(&poses, back));
    // A front part is not on the pad keepout's layer.
    poses[front].position = [10.0, 15.0];
    assert!(legal(&poses, front));
    // The fixed module's keepout blocks both sides.
    poses[front].position = [40.0, 20.0];
    assert!(!legal(&poses, front));
    poses[back].position = [40.0, 20.0];
    assert!(!legal(&poses, back));
}

#[test]
fn board_texts_and_silkscreen_graphics_keep_parts_off() {
    // The fence's labels: a front text, a back text, a rectangle drawn on
    // the front silkscreen around a group, a frame around the whole board
    // (decoration, no obstacle), a hidden text, a copper text, and a fixed
    // test pad on a text (a warning). A front and a back part to move.
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
        (gr_text "GND" (at 10 10 0) (layer "F.SilkS") (effects (font (size 0.8 0.8) (thickness 0.13))))
        (gr_text "STACK" (at 10 20 0) (layer "B.SilkS") (effects (font (size 0.8 0.8) (thickness 0.13)) (justify mirror)))
        (gr_rect (start 30 5) (end 40 15) (stroke (width 0.15) (type solid)) (fill no) (layer "F.SilkS"))
        (gr_rect (start 0.5 0.5) (end 59.5 29.5) (stroke (width 0.15) (type solid)) (fill no) (layer "F.SilkS"))
        (gr_text "HIDDEN" (at 50 20 0) (layer "F.SilkS") (hide yes) (effects (font (size 1 1) (thickness 0.15))))
        (gr_text "+5V" (at 50 10 0) (layer "F.Cu") (effects (font (size 1 1) (thickness 0.15))))
        (footprint "TP" (layer "F.Cu") (at 20 25) (locked yes)
            (property "Reference" "TP1")
            (fp_rect (start -1.3 -1.3) (end 1.3 1.3) (layer "F.CrtYd"))
            (pad "1" smd circle (at 0 0) (size 1.5 1.5) (layers "F.Cu") (net 1 "A")))
        (gr_text "TEST" (at 20 26.5 0) (layer "F.SilkS") (effects (font (size 0.8 0.8) (thickness 0.13))))
        (footprint "R" (layer "F.Cu") (at 25 3)
            (property "Reference" "R1")
            (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 2 "B")))
        (footprint "R" (layer "B.Cu") (at 25 27)
            (property "Reference" "R2")
            (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "B.CrtYd"))
            (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "B.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "B.Cu") (net 2 "B"))))"#,
    )
    .unwrap();
    let config = KiCadBoardPlacerConfig::default();
    let lowered = lower_placement(&pcb, &config).unwrap();
    let problem = &lowered.problem;
    let (front, back) = (1, 2);
    let marks: Vec<&str> = lowered.references[3..].iter().map(String::as_str).collect();
    // Four texts (GND, STACK, +5V, TEST) and the rectangle; not the frame
    // nor the hidden text.
    assert_eq!(
        marks
            .iter()
            .filter(|name| matches!(**name, "silkscreen mark" | "copper text"))
            .count(),
        5,
        "{marks:?}"
    );
    let legal = |poses: &[core::Pose], part: usize| {
        !core::legal::illegal_components(problem, poses).contains(&part)
    };
    let mut poses = problem.poses.clone();
    assert!(legal(&poses, front) && legal(&poses, back));
    // On the front text, the copper text, the rectangle's side: not legal
    // for the front part; inside the rectangle, clear of its sides: legal.
    for (position, expected) in [
        ([10.0, 10.0], false),
        ([50.0, 10.0], false),
        ([30.0, 10.0], false),
        ([35.0, 10.0], true),
        ([10.0, 20.0], true),
    ] {
        poses[front].position = position;
        assert_eq!(legal(&poses, front), expected, "front at {position:?}");
    }
    poses[front] = problem.poses[front];
    // The back text keeps the back part off; the front's marks do not.
    for (position, expected) in [
        ([10.0, 20.0], false),
        ([10.0, 10.0], true),
        ([35.0, 10.0], true),
    ] {
        poses[back].position = position;
        assert_eq!(legal(&poses, back), expected, "back at {position:?}");
    }
    // A routing halo is room for tracks, which run under a silkscreen
    // text but not through a copper one: 1 mm from either (more than the
    // spacing, less than the halo).
    let mut haloed = problem.clone();
    haloed.components[front].halo = 2.0;
    let mut poses = problem.poses.clone();
    poses[front].position = [10.0, 12.23];
    assert!(!core::legal::illegal_components(&haloed, &poses).contains(&front));
    poses[front].position = [50.0, 12.36];
    assert!(core::legal::illegal_components(&haloed, &poses).contains(&front));
    // The fixed test pad on its name: a warning.
    assert!(
        lowered
            .constraint_warnings
            .iter()
            .any(|warning| warning.contains("TP1") && warning.contains("\"TEST\"")),
        "{:?}",
        lowered.constraint_warnings
    );
    // Without `keep_board_marks` the marks are no obstacles (a copper
    // text is decoration the polish moves).
    let off = lower_placement(
        &pcb,
        &KiCadBoardPlacerConfig {
            keep_board_marks: false,
            ..config
        },
    )
    .unwrap();
    let mut poses = off.problem.poses.clone();
    for position in [[10.0, 10.0], [50.0, 10.0], [30.0, 10.0]] {
        poses[front].position = position;
        assert!(
            !core::legal::illegal_components(&off.problem, &poses).contains(&front),
            "{position:?}"
        );
    }
}

#[test]
fn a_part_that_would_sit_under_a_board_label_is_placed_beside_it() {
    // Two fixed connectors and a resistor between them: every spot on the
    // line between them is as short, and a label covers its middle.
    let board = r#"(kicad_pcb (version 20240108)
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (net 0 "") (net 1 "A") (net 2 "B")
        (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
        (gr_text "ABCDEFGHIJKLMNO" (at 20 10 0) (layer "F.SilkS") (effects (font (size 2 2) (thickness 0.3))))
        (footprint "Lib:J" (layer "F.Cu") (at 3 10) (locked yes)
            (property "Reference" "J1")
            (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at 0 0) (size 1.5 1.5) (layers "F.Cu") (net 1 "A")))
        (footprint "Lib:J" (layer "F.Cu") (at 37 10) (locked yes)
            (property "Reference" "J2")
            (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at 0 0) (size 1.5 1.5) (layers "F.Cu") (net 2 "B")))
        (footprint "Lib:R" (layer "F.Cu") (at 20 3)
            (property "Reference" "R1")
            (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 2 "B"))))"#;
    let label = {
        let pcb = parse(board).unwrap();
        board_marks(&pcb, outline_box(&pcb)).unwrap()[0].bounds
    };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("pcb-maker-label-{}-{nonce}", std::process::id()));
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("board.kicad_pcb"), board).unwrap();
    let mut under = Vec::new();
    for keep in [true, false] {
        let config = KiCadBoardPlacerConfig {
            keep_board_marks: keep,
            tuning: KiCadPlacerTuning {
                placement_seeds: 1,
                ..Default::default()
            },
            ..KiCadBoardPlacerConfig::default()
        };
        let output = root.join(format!("placed-{keep}"));
        let result = place_kicad_board(&source, "board", &output, &config).unwrap();
        assert!(result.unplaced.is_empty() && result.illegal.is_empty());
        let placed = fs::read_to_string(output.join("board.kicad_pcb")).unwrap();
        let placed = parse(&placed).unwrap();
        // The label stays.
        let text = placed
            .children()
            .iter()
            .find(|item| item.head() == Some("gr_text"))
            .unwrap();
        assert_eq!(form_at(text).unwrap()[..2], [20.0, 10.0]);
        let r1 = placed
            .children()
            .iter()
            .find(|item| crate::kicad_nets::footprint_reference(item).as_deref() == Some("R1"))
            .unwrap();
        let at = form_at(r1).unwrap();
        let (sin, cos) = at[2].to_radians().sin_cos();
        let half = [
            1.5 * cos.abs() + 0.7 * sin.abs(),
            1.5 * sin.abs() + 0.7 * cos.abs(),
        ];
        under.push(
            at[0] - half[0] < label[2]
                && label[0] < at[0] + half[0]
                && at[1] - half[1] < label[3]
                && label[1] < at[1] + half[1],
        );
    }
    let _ = fs::remove_dir_all(&root);
    assert_eq!(under, [false, true], "R1 under the label (kept, not kept)");
}

#[test]
fn a_mark_on_a_movable_part_at_its_input_position_belongs_to_it() {
    // J1's pin names inside its courtyard (a connector the placement may
    // move): no obstacle. "TITLE" over free space and "TP" over a fixed
    // test pad: obstacles. "STACK" over two parts stacked at one origin
    // (a task's unplaced parts): an obstacle, the stack says nothing.
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
        (gr_text "1 2 3" (at 10 10 0) (layer "F.SilkS") (effects (font (size 0.8 0.8) (thickness 0.13))))
        (gr_text "TITLE" (at 30 25 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
        (gr_text "TP" (at 50 10 0) (layer "F.SilkS") (effects (font (size 0.8 0.8) (thickness 0.13))))
        (gr_text "STACK" (at 30 5 0) (layer "F.SilkS") (effects (font (size 0.8 0.8) (thickness 0.13))))
        (footprint "J" (layer "F.Cu") (at 10 10)
            (property "Reference" "J1")
            (fp_rect (start -4 -1.5) (end 4 1.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at -2.5 0) (size 1 1) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 2.5 0) (size 1 1) (layers "F.Cu") (net 2 "B")))
        (footprint "TP" (layer "F.Cu") (at 50 10) (locked yes)
            (property "Reference" "TP1")
            (fp_rect (start -1.3 -1.3) (end 1.3 1.3) (layer "F.CrtYd"))
            (pad "1" smd circle (at 0 0) (size 1.5 1.5) (layers "F.Cu") (net 1 "A")))
        (footprint "R" (layer "F.Cu") (at 30 5)
            (property "Reference" "R1")
            (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 2 "B")))
        (footprint "R" (layer "F.Cu") (at 30 5)
            (property "Reference" "R2")
            (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 2 "B"))))"#,
    )
    .unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    assert_eq!(
        lowered.attached_report,
        ["text \"1 2 3\" (F.SilkS at 10.00, 10.00): J1"]
    );
    let problem = &lowered.problem;
    let r1 = 2;
    let legal = |position: [f64; 2]| {
        let mut poses = problem.poses.clone();
        poses[r1].position = position;
        poses[r1 + 1].position = [5.0, 25.0];
        !core::legal::illegal_components(problem, &poses).contains(&r1)
    };
    // On J1's own names (J1 itself moved away): free; on the others: not.
    let mut moved = problem.poses.clone();
    moved[0].position = [10.0, 20.0];
    let mut on_names = moved.clone();
    on_names[r1].position = [10.0, 10.0];
    on_names[r1 + 1].position = [5.0, 25.0];
    assert!(!core::legal::illegal_components(problem, &on_names).contains(&r1));
    assert!(!legal([30.0, 25.0]));
    assert!(!legal([30.0, 5.0]));
    assert!(legal([40.0, 18.0]));
}

#[test]
fn board_marks_give_way_when_nothing_else_seats_a_part() {
    // A label across the whole of a small board: the resistor (off the
    // board in the input, so the label is not its own) has no spot off
    // it, so it lies there (and the placement says so).
    let board = r#"(kicad_pcb (version 20240108)
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (net 0 "") (net 1 "A") (net 2 "B")
        (gr_rect (start 0 0) (end 12 6) (layer "Edge.Cuts"))
        (gr_text "ABCDE" (at 6 3 0) (layer "F.SilkS") (effects (font (size 3 2) (thickness 0.3))))
        (footprint "Lib:R" (layer "F.Cu") (at 20 3)
            (property "Reference" "R1")
            (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 2 "B"))))"#;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("pcb-maker-yield-{}-{nonce}", std::process::id()));
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("board.kicad_pcb"), board).unwrap();
    let config = KiCadBoardPlacerConfig {
        free: vec!["R1".into()],
        tuning: KiCadPlacerTuning {
            placement_seeds: 1,
            ..Default::default()
        },
        ..KiCadBoardPlacerConfig::default()
    };
    let result = place_kicad_board(&source, "board", &root.join("placed"), &config).unwrap();
    let _ = fs::remove_dir_all(&root);
    assert!(result.unplaced.is_empty() && result.illegal.is_empty());
    assert_eq!(result.parts_on_marks, ["R1"]);
    // How full a side is, in the room's numbers (trial 4: the hint's 99 %
    // beside room.utilization 0.135).
    assert_eq!(result.utilization, result.room.utilization);
}

#[test]
fn the_full_side_hint_says_what_it_measures_in_the_rooms_terms() {
    // Trial 4's ULK panel: "the front side is 99 % full" counted every
    // component (fixed keys, the board's marks, the frames' keepouts) over
    // the outline's gross area, while room.utilization said 0.135. The
    // hint now is the room's number, and says what it is.
    let hint = super::place::room_hint("front", 0.923, 3080.0);
    assert!(
        hint.starts_with(
            "the front side is 92 % full (room.utilization 0.923: the movable parts' bodies with spacing over its 3080 mm2 usable, the outline less its edge band, its cutouts and the fixed parts)"
        ),
        "{hint}"
    );
    let none = super::place::room_hint("back", 99.0, 0.0);
    assert!(
        none.starts_with("the back side has no usable area left"),
        "{none}"
    );
}

#[test]
fn an_inner_layer_pad_keepout_keeps_pads_out() {
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
        (zone (layers "In1.Cu") (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints allowed))
            (polygon (pts (xy 2 2) (xy 10 2) (xy 10 10) (xy 2 10))))
        (footprint "J" (layer "F.Cu") (at 40 20) (attr through_hole)
            (fp_rect (start -2 -1) (end 2 1) (layer "F.CrtYd"))
            (pad "1" thru_hole circle (at -1 0) (size 1 1) (drill 0.6) (layers "*.Cu") (net 1 "A"))
            (pad "2" thru_hole circle (at 1 0) (size 1 1) (drill 0.6) (layers "*.Cu") (net 2 "B"))))"#,
    )
    .unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let problem = &lowered.problem;
    let mut poses = problem.poses.clone();
    assert!(core::legal::illegal_components(problem, &poses).is_empty());
    poses[0].position = [6.0, 6.0];
    assert!(core::legal::illegal_components(problem, &poses).contains(&0));
}

#[test]
fn a_movable_parts_pad_keepout_moves_with_it() {
    // MokyaLora's U.FL on the back: its own keepout forbids pads on
    // every layer (footprints allowed). Placed elsewhere, the keepout
    // goes with it, in the model and in the written board.
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
        (footprint "UFL" (layer "B.Cu") (at 10 10)
            (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "B.CrtYd"))
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu") (net 1 "A"))
            (zone (layers "F.Cu" "B.Cu") (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints allowed))
                (polygon (pts (xy 11 9) (xy 13 9) (xy 13 11) (xy 11 11)))))
        (footprint "R" (layer "F.Cu") (at 45 15)
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))))"#,
    )
    .unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let problem = &lowered.problem;
    let (ufl, front) = (0, 1);
    assert!(!problem.components[ufl].fixed);
    let mut poses = problem.poses.clone();
    poses[ufl] = core::Pose {
        position: [30.0, 15.0],
        angle: 90.0,
    };
    let legal = |poses: &[core::Pose]| core::legal::illegal_components(problem, poses).is_empty();
    // The keepout's first pad sits 2 mm along the U.FL's x axis: turned
    // by 90 degrees, 2 mm above it (y grows downwards).
    poses[front].position = [30.5, 13.0];
    assert!(!legal(&poses), "a front pad in the moved keepout");
    poses[front].position = [30.5, 18.0];
    assert!(legal(&poses), "the keepout is off the front part's pads");
    // The written board carries the keepout along.
    let mut footprint = pcb
        .children()
        .iter()
        .find(|item| item.head() == Some("footprint"))
        .unwrap()
        .clone();
    write_footprint_pose(&mut footprint, poses[ufl]).unwrap();
    let zone = footprint.child("zone").unwrap();
    let points: Vec<[f64; 2]> = zone
        .child("polygon")
        .and_then(|polygon| polygon.child("pts"))
        .unwrap()
        .children()
        .iter()
        .filter(|child| child.head() == Some("xy"))
        .map(|point| {
            [
                point.children()[1].atom().unwrap().parse().unwrap(),
                point.children()[2].atom().unwrap().parse().unwrap(),
            ]
        })
        .collect();
    assert_eq!(
        points,
        vec![[29.0, 14.0], [29.0, 12.0], [31.0, 12.0], [31.0, 14.0]]
    );
}

#[test]
fn parts_stay_on_the_piece_of_the_fixed_parts_they_join() {
    // Two pieces side by side (zpn_devboard: a board and an empty
    // breakaway); J1 is locked on the right one. R1 joins J1 through
    // its net, R2 through R1: both stay on the right piece. R3 joins
    // nothing fixed and may go anywhere.
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts"))
        (gr_rect (start 40 0) (end 90 30) (layer "Edge.Cuts"))
        (footprint "J" (layer "F.Cu") (at 60 15) (locked yes)
            (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
            (property "Reference" "J1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
        (footprint "R" (layer "F.Cu") (at 10 10)
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
            (property "Reference" "R1")
            (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B")))
        (footprint "R" (layer "F.Cu") (at 10 20)
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
            (property "Reference" "R2")
            (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))
            (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 3 "C")))
        (footprint "R" (layer "F.Cu") (at 20 20)
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
            (property "Reference" "R3")
            (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 4 "D"))
            (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 5 "E"))))"#,
    )
    .unwrap();
    let config = KiCadBoardPlacerConfig {
        keep_on_pieces: true,
        ..KiCadBoardPlacerConfig::default()
    };
    let lowered = lower_placement(&pcb, &config).unwrap();
    let problem = &lowered.problem;
    assert_eq!(problem.pieces.len(), 2);
    let right = *problem.pieces.iter().find(|piece| piece[0] > 35.0).unwrap();
    let mut regions = problem.constraints.regions.clone();
    regions.sort_by_key(|(part, _)| *part);
    assert_eq!(regions, vec![(1, right), (2, right)]);
    // On by default; `keep_on_pieces: false` switches it off.
    let lowered = lower_placement(
        &pcb,
        &KiCadBoardPlacerConfig {
            keep_on_pieces: false,
            ..KiCadBoardPlacerConfig::default()
        },
    )
    .unwrap();
    assert!(lowered.problem.constraints.regions.is_empty());
}

#[test]
fn a_shaped_outline_gives_the_placer_its_sides_pieces_and_solid_cutouts() {
    // Two pieces from the constraints: an L (its step at y = 10) and a
    // 20 mm square with a window. J1 is locked on the square; R1 joins
    // it by its net and stays on that piece (keep_on_pieces). J2 brings
    // a slot of its own (Edge.Cuts in the footprint): copper keeps off
    // it, bodies may reach over it, unlike the window.
    let mut pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (footprint "J" (layer "F.Cu") (at 80 15) (locked yes)
            (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
            (property "Reference" "J1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
        (footprint "J" (layer "F.Cu") (at 30 20) (locked yes)
            (fp_rect (start -2 -2) (end 2 2) (layer "F.CrtYd"))
            (fp_rect (start -0.5 -1) (end 0.5 1) (layer "Edge.Cuts"))
            (property "Reference" "J2")
            (pad "1" smd rect (at 1.5 0) (size 0.5 0.5) (layers "F.Cu") (net 3 "C")))
        (footprint "R" (layer "F.Cu") (at 10 10)
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
            (property "Reference" "R1")
            (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))))"#,
    )
    .unwrap();
    let constraints: KiCadPlacementConstraints = serde_json::from_str(
        r#"{"version": 1, "edge": [{"part": "R1", "edge": "top"}],
            "outline": {"x": 0, "y": 0, "pieces": [
                {"polygon": [[0, 0], [20, 0], [20, 10], [60, 10], [60, 30], [0, 30]]},
                {"width": 20, "height": 20, "x": 70, "y": 0, "cutouts": [{"x": [2, 6], "y": [2, 6]}]}]}}"#,
    )
    .unwrap();
    apply_outline(&mut pcb, constraints.outline.as_ref().unwrap(), 0.0).unwrap();
    let config = KiCadBoardPlacerConfig {
        constraints: Some(KiCadConstraintsSource::Inline(constraints.clone())),
        ..KiCadBoardPlacerConfig::default()
    };
    let lowered = lower_placement(&pcb, &config).unwrap();
    let problem = &lowered.problem;
    assert_eq!(
        problem.pieces,
        vec![[0.0, 0.0, 60.0, 30.0], [70.0, 0.0, 90.0, 20.0]]
    );
    // Two pieces' sides: six and four.
    assert_eq!(problem.constraints.sides.len(), 10);
    // The window keeps bodies off, J2's slot copper only.
    let holes: Vec<usize> = (0..problem.components.len())
        .filter(|index| problem.components[*index].copper_only)
        .collect();
    assert_eq!(holes.len(), 2);
    assert_eq!(problem.constraints.solid_cutouts.len(), 1);
    let window = &problem.components[problem.constraints.solid_cutouts[0]];
    assert_eq!(window.body_size, [4.0, 4.0]);
    assert_eq!(
        problem.poses[problem.constraints.solid_cutouts[0]].position,
        [74.0, 4.0]
    );
    // R1 is on J1's piece.
    let r1 = lowered
        .references
        .iter()
        .position(|reference| reference == "R1")
        .unwrap();
    assert!(
        problem
            .constraints
            .regions
            .contains(&(r1, [70.0, 0.0, 90.0, 20.0])),
        "{:?}",
        problem.constraints.regions
    );
    // A rectangle keeps the box's sides and has no solid cutouts.
    let rectangle: KiCadPlacementConstraints =
        serde_json::from_str(r#"{"version": 1, "outline": {"width": 60, "height": 30, "x": 0, "y": 0, "corner_radius": 2}}"#).unwrap();
    apply_outline(&mut pcb, rectangle.outline.as_ref().unwrap(), 0.0).unwrap();
    let config = KiCadBoardPlacerConfig {
        constraints: Some(KiCadConstraintsSource::Inline(rectangle)),
        ..KiCadBoardPlacerConfig::default()
    };
    let lowered = lower_placement(&pcb, &config).unwrap();
    assert!(lowered.problem.constraints.sides.is_empty());
    assert!(lowered.problem.constraints.solid_cutouts.is_empty());
}

#[test]
fn copies_of_a_tabbed_panel_keep_their_parts() {
    // One outline piece holding two copies (KiKit names their nets
    // Board_0-... and Board_1-...): each copy's J1 is locked, its R1
    // joins it through a net. Each R1 stays on its copy's side of the
    // line halfway between the copies' fixed parts.
    let copy = |index: usize, x: f64| {
        format!(
            r#"(footprint "J" (layer "F.Cu") (at {x} 15) (locked yes)
            (fp_rect (start -2 -2) (end 2 2) (layer "F.CrtYd"))
            (property "Reference" "J1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "Board_{index}-A")))
        (footprint "R" (layer "F.Cu") (at 50 15)
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
            (property "Reference" "R1")
            (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net "Board_{index}-A"))
            (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net "Board_{index}-B")))"#
        )
    };
    let board = |copies: &str| {
        parse(&format!(
            r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 100 30) (layer "Edge.Cuts"))
        {copies})"#
        ))
        .unwrap()
    };
    let pcb = board(&format!("{}\n{}", copy(0, 10.0), copy(1, 90.0)));
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let problem = &lowered.problem;
    assert!(problem.pieces.len() < 2);
    let mut regions = problem.constraints.regions.clone();
    regions.sort_by_key(|(part, _)| *part);
    // J1 bodies end at 12 and start at 88: the cut is at 50.
    assert_eq!(
        regions,
        vec![(1, [0.0, 0.0, 50.0, 30.0]), (3, [50.0, 0.0, 100.0, 30.0])]
    );
    // Each copy's parts start global placement at their region's centre.
    let mut starts = problem.constraints.starts.clone();
    starts.sort_by_key(|(part, _)| *part);
    assert_eq!(starts, vec![(1, [25.0, 15.0]), (3, [75.0, 15.0])]);
    // Copies whose fixed parts sit at their left end (SmartSpin2k's at
    // the top): the cut moves by half the difference of the board's
    // margins beyond them (3 mm left, 43 mm right: 20 mm) from halfway
    // (30) to 50, where the copies meet; it stays between the copies'
    // fixed parts (55 mm apart: 23 mm right of the first copy's).
    let regions_of = |first: f64, second: f64| {
        let pcb = board(&format!("{}\n{}", copy(0, first), copy(1, second)));
        let mut regions = lower_placement(&pcb, &KiCadBoardPlacerConfig::default())
            .unwrap()
            .problem
            .constraints
            .regions;
        regions.sort_by_key(|(part, _)| *part);
        regions
    };
    assert_eq!(
        regions_of(5.0, 55.0),
        vec![(1, [0.0, 0.0, 50.0, 30.0]), (3, [50.0, 0.0, 100.0, 30.0])]
    );
    assert_eq!(
        regions_of(5.0, 25.0),
        vec![(1, [0.0, 0.0, 23.0, 30.0]), (3, [23.0, 0.0, 100.0, 30.0])]
    );
    // Switched off, or a single copy, or copies whose nets join: none.
    let off = KiCadBoardPlacerConfig {
        tuning: KiCadPlacerTuning {
            keep_copies: false,
            ..Default::default()
        },
        ..KiCadBoardPlacerConfig::default()
    };
    assert!(
        lower_placement(&pcb, &off)
            .unwrap()
            .problem
            .constraints
            .regions
            .is_empty()
    );
    let single = board(&copy(0, 10.0));
    assert!(
        lower_placement(&single, &KiCadBoardPlacerConfig::default())
            .unwrap()
            .problem
            .constraints
            .regions
            .is_empty()
    );
    let shared = board(&format!("{}\n{}", copy(0, 10.0), copy(0, 90.0)));
    assert!(
        lower_placement(&shared, &KiCadBoardPlacerConfig::default())
            .unwrap()
            .problem
            .constraints
            .regions
            .is_empty()
    );
    // Parts that are not copies of each other (another reference) stay
    // free too: a board of two circuits is not a panel.
    let other = board(&format!(
        "{}\n{}",
        copy(0, 10.0),
        copy(1, 90.0).replace("\"R1\"", "\"R2\"")
    ));
    assert!(
        lower_placement(&other, &KiCadBoardPlacerConfig::default())
            .unwrap()
            .problem
            .constraints
            .regions
            .is_empty()
    );
}

#[test]
fn the_lowering_marks_the_poured_nets() {
    // GND has a zone; A and B do not.
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts"))
        (zone (net 3) (net_name "GND") (layer "B.Cu")
            (polygon (pts (xy 0 0) (xy 30 0) (xy 30 30) (xy 0 30))))
        (footprint "R" (layer "F.Cu") (at 10 10)
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
            (property "Reference" "R1")
            (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 3 "GND")))
        (footprint "R" (layer "F.Cu") (at 20 20)
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
            (property "Reference" "R2")
            (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))))"#,
    )
    .unwrap();
    let config = KiCadBoardPlacerConfig::default();
    let lowered = lower_placement(&pcb, &config).unwrap();
    let net_of = |reference: &str, pad: usize| {
        let index = lowered
            .references
            .iter()
            .position(|name| name == reference)
            .unwrap();
        lowered.problem.components[index].pins[pad].net
    };
    assert!(lowered.poured[net_of("R1", 1)], "{:?}", lowered.poured);
    assert!(!lowered.poured[net_of("R1", 0)] && !lowered.poured[net_of("R2", 1)]);
}

#[test]
fn a_row_of_shaped_parts_blocks_with_every_members_shape() {
    // Two L-shaped sockets as a row, 12 mm apart along x: the second
    // one's L blocks as the first one's does, its notch stays free.
    let socket = |reference: &str, x: f64| {
        format!(
            r#"(footprint "Socket" (layer "F.Cu") (at {x} 15)
            (property "Reference" "{reference}")
            (fp_poly (pts (xy -5 -5) (xy -1 -5) (xy -1 1) (xy 5 1) (xy 5 5) (xy -5 5)) (layer "F.CrtYd") (width 0.05))
            (pad "1" smd rect (at -3 3) (size 1 1) (layers "F.Cu") (net 1 "A")))"#
        )
    };
    let pcb = parse(&format!(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
        {} {}
        (footprint "R" (layer "F.Cu") (at 50 25)
            (property "Reference" "R1")
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))))"#,
        socket("J1", 10.0),
        socket("J2", 40.0),
    ))
    .unwrap();
    let constraints: crate::placement_constraints::KiCadPlacementConstraints =
        serde_json::from_str(r#"{"version": 1, "row": [{"parts": ["J1", "J2"], "pitch_mm": 12}]}"#)
            .unwrap();
    let config = KiCadBoardPlacerConfig {
        constraints: Some(KiCadConstraintsSource::Inline(constraints)),
        ..KiCadBoardPlacerConfig::default()
    };
    let lowered = lower_placement(&pcb, &config).unwrap();
    let problem = &lowered.problem;
    let mut poses = problem.poses.clone();
    let legal =
        |poses: &[core::Pose]| !core::legal::illegal_components(problem, poses).contains(&2);
    // J2 sits at x 22: in its notch legal, in its L not.
    poses[2].position = [22.0 + 2.5, 15.0 - 2.5];
    assert!(legal(&poses));
    poses[2].position = [22.0 - 3.0, 15.0 - 2.5];
    assert!(!legal(&poses));
}

#[test]
fn a_part_fits_in_the_notch_of_an_l_shaped_courtyard() {
    // A socket whose courtyard is an L (10 x 10 with its top-right
    // 6 x 6 quarter cut away), and a small part.
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
        (footprint "Socket" (layer "F.Cu") (at 20 15)
            (fp_poly (pts (xy -5 -5) (xy -1 -5) (xy -1 1) (xy 5 1) (xy 5 5) (xy -5 5)) (layer "F.CrtYd") (width 0.05))
            (pad "1" smd rect (at -3 3) (size 1 1) (layers "F.Cu") (net 1 "A")))
        (footprint "R" (layer "F.Cu") (at 45 15)
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))))"#,
    )
    .unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let problem = &lowered.problem;
    assert!(
        !problem.components[0].hollow.is_empty(),
        "the L blocks with its own shape"
    );
    let mut poses = problem.poses.clone();
    let legal =
        |poses: &[core::Pose]| !core::legal::illegal_components(problem, poses).contains(&1);
    // In the notch (local x 1..5, y -5..1): legal; in the L: not.
    poses[1].position = [20.0 + 2.5, 15.0 - 2.5];
    assert!(legal(&poses));
    poses[1].position = [20.0 - 3.0, 15.0 - 2.5];
    assert!(!legal(&poses));
}

#[test]
fn a_fixed_parts_mask_opening_keeps_other_parts_pads_out() {
    // SNSP-CPU-01's J2 (SNES_Library:RFModulator, at its angle): a
    // 12 x 5 courtyard around four pins, a filled F.Mask polygon of
    // 22 x 32 mm beside it and a smaller F.Mask rectangle. The courtyard
    // is far smaller than the body's box, so the part blocks with the
    // courtyard's shape and its pads, and the layout placed T1, C61,
    // C63, C64 and C66 under the polygon (KiCad: solder_mask_bridge for
    // every pad there). And an 0805 capacitor.
    let pcb = parse(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 120 80) (layer "Edge.Cuts"))
        (footprint "SNES_Library:RFModulator" (layer "F.Cu") (at 60 40 -90) (property "Reference" "J2")
            (fp_rect (start -3.429 -1.27) (end 3.81 6.9596) (stroke (width 0.1) (type solid)) (fill yes) (layer "F.Mask"))
            (fp_poly (pts (xy -10.922 38.862) (xy -10.922 6.604) (xy -3.429 6.6) (xy -3.429 6.9596) (xy -6.096 6.9596)
                (xy -6.096 10.668) (xy 6.096 10.668) (xy 6.096 6.9596) (xy 3.81 6.9596) (xy 3.81 6.604) (xy 11.43 6.604) (xy 11.43 38.862))
                (stroke (width 0.1) (type solid)) (fill yes) (layer "F.Mask"))
            (fp_rect (start -6 6.5) (end 6 11.5) (stroke (width 0.05) (type default)) (fill no) (layer "F.CrtYd"))
            (pad "1" thru_hole rect (at -3.81 8.89 270) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask") (net 1 "VCC"))
            (pad "2" thru_hole rect (at -1.27 8.89 270) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask") (net 2 "VIDEO"))
            (pad "3" thru_hole rect (at 1.27 8.89 270) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask") (net 3 "AUDIO"))
            (pad "4" thru_hole rect (at 3.81 8.89 270) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask") (net 4 "GND")))
        (footprint "C_0805" (layer "F.Cu") (at 100 60) (property "Reference" "C61")
            (fp_rect (start -1.7 -0.95) (end 1.7 0.95) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.95 0) (size 1 1.45) (layers "F.Cu" "F.Paste" "F.Mask") (net 2 "VIDEO"))
            (pad "2" smd rect (at 0.95 0) (size 1 1.45) (layers "F.Cu" "F.Paste" "F.Mask") (net 4 "GND"))))"#,
    )
    .unwrap();
    let config = KiCadBoardPlacerConfig {
        fixed: vec!["J2".into()],
        ..KiCadBoardPlacerConfig::default()
    };
    let mut placement = lower_placement(&pcb, &config).unwrap();
    let (j2, c) = (0, 1);
    assert_eq!(
        placement.references[..2],
        ["J2".to_string(), "C61".to_string()]
    );
    assert!(placement.problem.components[j2].fixed);
    assert!(
        !placement.problem.components[j2].hollow.is_empty(),
        "J2 blocks with its courtyard's shape"
    );
    // A point of J2's own frame on the board.
    let at = |problem: &core::Problem, local: [f64; 2]| {
        let pose = problem.poses[j2];
        let offset = problem.components[j2].offset(local, pose.angle);
        [pose.position[0] + offset[0], pose.position[1] + offset[1]]
    };
    for fixed in [true, false] {
        placement.problem.components[j2].fixed = fixed;
        let problem = &placement.problem;
        let mut poses = problem.poses.clone();
        let legal =
            |poses: &[core::Pose]| !core::legal::illegal_components(problem, poses).contains(&c);
        let seen = |poses: &[core::Pose]| core::anneal::overlaps(problem, poses)[c];
        // Where C61 was: deep inside the polygon, 13 mm from the
        // courtyard; and under the small rectangle beside the pins.
        for local in [[0.0, 25.0], [8.0, 15.0], [0.0, 2.0]] {
            poses[c].position = at(problem, local);
            assert!(
                !legal(&poses),
                "fixed {fixed}: under the opening at {local:?}"
            );
            assert!(
                seen(&poses) > 0.0,
                "fixed {fixed}: the annealer sees the opening at {local:?}"
            );
        }
        // Beside the polygon (its edge at local x 11.43): legal.
        poses[c].position = at(problem, [16.0, 25.0]);
        assert!(legal(&poses), "fixed {fixed}: beside the opening");
        assert_eq!(seen(&poses), 0.0);
    }
    // Off, the shape is the courtyard and the pads alone, as before.
    let off = lower_placement(
        &pcb,
        &KiCadBoardPlacerConfig {
            tuning: KiCadPlacerTuning {
                mask_openings: false,
                ..config.tuning.clone()
            },
            ..config.clone()
        },
    )
    .unwrap();
    let mut poses = off.problem.poses.clone();
    poses[c].position = at(&off.problem, [0.0, 25.0]);
    assert!(!core::legal::illegal_components(&off.problem, &poses).contains(&c));
}

#[test]
fn a_mask_opening_keeps_kicads_mask_margin_beyond_the_copper_clearance() {
    let board = |setup: &str| parse(&format!(r#"(kicad_pcb (setup {setup}))"#)).unwrap();
    let config = |clearance: f64, to_copper: Option<f64>| KiCadBoardPlacerConfig {
        copper_clearance_mm: Some(clearance),
        solder_mask_to_copper_mm: to_copper,
        ..KiCadBoardPlacerConfig::default()
    };
    // SNSP: everything 0, the clearance decides.
    assert_eq!(
        mask_opening_growth(&board("(pad_to_mask_clearance 0)"), &config(0.2, Some(0.0))),
        0.0
    );
    // eternal-keypad: a web of 0.25 and an expansion of 0.051 against
    // a clearance of 0.1995.
    let keypad = mask_opening_growth(
        &board("(pad_to_mask_clearance 0.051) (solder_mask_min_width 0.25)"),
        &config(0.1995, None),
    );
    assert!((keypad - 0.1015).abs() < 1.0e-9, "{keypad}");
    // A mask-to-copper clearance larger than the web and expansion.
    let covered = mask_opening_growth(
        &board("(pad_to_mask_clearance 0.05) (solder_mask_min_width 0.1)"),
        &config(0.2, Some(0.3)),
    );
    assert!((covered - 0.1).abs() < 1.0e-9, "{covered}");
}

#[test]
fn courtyard_less_parts_at_45_degrees_meet_by_their_turned_pads() {
    // Two 0402s without courtyards, turned by 45 degrees, side by side
    // across their long axes: the squares around their pads overlap,
    // the pads do not.
    let part = |x: f64, y: f64| {
        format!(
            r#"(footprint "R" (layer "F.Cu") (at {x} {y} 45)
            (pad "1" smd rect (at -0.5 0 45) (size 0.6 0.5) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.5 0 45) (size 0.6 0.5) (layers "F.Cu") (net 2 "B")))"#
        )
    };
    // KiCad turns by -45: the part's own y axis points to (sin 45, cos 45)
    // on the board, 1.2 mm along it.
    let step = 1.2 / 2f64.sqrt();
    let pcb = parse(&format!(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts")) {} {})"#,
        part(15.0, 15.0),
        part(15.0 + step, 15.0 + step)
    ))
    .unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let problem = &lowered.problem;
    assert!(
        !problem.components[0].hollow.is_empty(),
        "no courtyard: the pads block"
    );
    assert!(core::legal::illegal_components(problem, &problem.poses).is_empty());
    // Half as far apart, the pads meet.
    let mut poses = problem.poses.clone();
    poses[1].position = [15.0 + step / 2.0, 15.0 + step / 2.0];
    assert!(core::legal::illegal_components(problem, &poses).contains(&1));
}

#[test]
fn a_diode_sits_between_the_pads_of_a_socket_without_a_courtyard() {
    // A hot-swap socket without a courtyard (two pads 14 mm apart and
    // a centre hole), and a diode with one.
    let pcb = parse(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
        (footprint "Socket" (layer "F.Cu") (at 20 15)
            (pad "" np_thru_hole circle (at 0 0) (size 4 4) (drill 4) (layers "*.Cu" "*.Mask"))
            (pad "1" smd rect (at -7 -3) (size 2.5 2.5) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 7 -3) (size 2.5 2.5) (layers "F.Cu") (net 2 "B")))
        (footprint "D" (layer "F.Cu") (at 45 15)
            (fp_rect (start -1.5 -0.8) (end 1.5 0.8) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.9 0) (size 0.8 1) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.9 0) (size 0.8 1) (layers "F.Cu") (net 3 "C"))))"#,
    )
    .unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let problem = &lowered.problem;
    let mut poses = problem.poses.clone();
    let legal =
        |poses: &[core::Pose]| !core::legal::illegal_components(problem, poses).contains(&1);
    // Between the pads, clear of the hole: legal (KiCad checks copper).
    poses[1].position = [20.0, 15.0 - 4.0];
    assert!(legal(&poses));
    // On a pad or on the hole: not.
    poses[1].position = [20.0 - 7.0, 15.0 - 3.0];
    assert!(!legal(&poses));
    poses[1].position = [20.0, 15.0];
    assert!(!legal(&poses));
}

#[test]
fn pads_in_rows_of_pins_mean_a_body_fills_their_box() {
    let grid = |columns: &[f64], rows: &[f64]| -> Vec<[f64; 2]> {
        columns
            .iter()
            .flat_map(|x| rows.iter().map(move |y| [*x, *y]))
            .collect()
    };
    // A 2 x 3 header, an SOIC-8, a card slot's two rows.
    assert!(pads_in_pin_rows(&grid(&[-2.54, 0.0, 2.54], &[-1.27, 1.27])));
    assert!(pads_in_pin_rows(&grid(
        &[-2.7, 2.7],
        &[-1.905, -0.635, 0.635, 1.905]
    )));
    assert!(pads_in_pin_rows(&grid(
        &(0..31).map(|i| i as f64 * 2.54).collect::<Vec<_>>(),
        &[-5.08, 0.0]
    )));
    // A QFP's four sides of 0.5 mm pitch with an exposed pad.
    let mut quad: Vec<[f64; 2]> = (0..8)
        .map(|i| -1.75 + 0.5 * i as f64)
        .flat_map(|t| [[-4.0, t], [4.0, t], [t, -4.0], [t, 4.0]])
        .collect();
    quad.push([0.0, 0.0]);
    assert!(pads_in_pin_rows(&quad));
    // A key switch (ULK's PG1316: six pads over 14 mm), a hot-swap
    // socket, a two-pad part, a pad stacked on itself.
    assert!(!pads_in_pin_rows(&[
        [-6.35, -6.0],
        [-6.35, 6.0],
        [-2.5, 2.65],
        [2.5, 2.65],
        [6.35, -6.0],
        [6.35, 6.0]
    ]));
    assert!(!pads_in_pin_rows(&[
        [-7.56, -2.54],
        [-5.08, 0.0],
        [-3.81, -2.54],
        [0.0, 0.0],
        [2.54, -5.08],
        [5.08, 0.0],
        [6.29, -5.08]
    ]));
    assert!(!pads_in_pin_rows(&[[-0.5, 0.0], [0.5, 0.0]]));
    assert!(!pads_in_pin_rows(&[
        [0.0, 0.0],
        [0.0, 0.0],
        [1.0, 0.0],
        [1.0, 0.0]
    ]));
}

#[test]
fn a_courtyard_less_quad_blocks_inside_its_pad_ring() {
    // hackrf's LQFP without a courtyard: 4 x 6 pads of 0.5 mm pitch on
    // a 10 mm ring, and an SOT-23-sized part with a courtyard.
    let mut pads = String::new();
    for i in 0..6 {
        let t = -1.25 + 0.5 * i as f64;
        for (x, y, w, h) in [
            (-5.0, t, 1.2, 0.3),
            (5.0, t, 1.2, 0.3),
            (t, -5.0, 0.3, 1.2),
            (t, 5.0, 0.3, 1.2),
        ] {
            pads += &format!(
                r#"(pad "{i}" smd rect (at {x} {y}) (size {w} {h}) (layers "F.Cu") (net 1 "A"))"#
            );
        }
    }
    let pcb = parse(&format!(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
        (footprint "QFP" (layer "F.Cu") (at 20 15) {pads})
        (footprint "Q" (layer "F.Cu") (at 45 15)
            (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.95 1) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.95 1) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))
            (pad "3" smd rect (at 0 -1) (size 0.6 0.8) (layers "F.Cu") (net 3 "C"))))"#
    ))
    .unwrap();
    let inside = |config: &KiCadBoardPlacerConfig| {
        let lowered = lower_placement(&pcb, config).unwrap();
        let problem = &lowered.problem;
        let mut poses = problem.poses.clone();
        poses[1].position = [20.0, 15.0];
        !core::legal::illegal_components(problem, &poses).contains(&1)
    };
    assert!(
        !inside(&KiCadBoardPlacerConfig::default()),
        "the body fills the pad ring"
    );
    let off = KiCadBoardPlacerConfig {
        tuning: KiCadPlacerTuning {
            solid_pin_rows: false,
            ..Default::default()
        },
        ..KiCadBoardPlacerConfig::default()
    };
    assert!(inside(&off), "off: only the pads block");
}

#[test]
fn a_header_without_a_courtyard_keeps_parts_from_between_its_rows() {
    // Two 2 x 10 headers of 2.54 mm pitch whose rows lie 5.08 mm apart
    // (a card slot's): the second may not interleave with the first.
    let header = |reference: &str, y: f64| {
        let mut pads = String::new();
        for i in 0..10 {
            for row in [-2.54, 2.54] {
                pads += &format!(
                    r#"(pad "{i}" thru_hole circle (at {} {row}) (size 1.6 1.6) (drill 1) (layers "*.Cu" "*.Mask") (net 1 "A"))"#,
                    i as f64 * 2.54
                );
            }
        }
        format!(
            r#"(footprint "Slot" (layer "F.Cu") (at 10 {y}) (property "Reference" "{reference}") (attr through_hole) {pads})"#
        )
    };
    let pcb = parse(&format!(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 40) (layer "Edge.Cuts")) {} {})"#,
        header("J1", 10.0),
        header("J2", 30.0)
    ))
    .unwrap();
    let interleaved = |config: &KiCadBoardPlacerConfig| {
        let lowered = lower_placement(&pcb, config).unwrap();
        let problem = &lowered.problem;
        let mut poses = problem.poses.clone();
        // J2's top row between J1's rows (at y 10), its bottom row
        // below them.
        poses[1].position = [10.0, 10.0 + 2.54];
        !core::legal::illegal_components(problem, &poses).contains(&1)
    };
    assert!(!interleaved(&KiCadBoardPlacerConfig::default()));
    // Off, the two headers' bodies without courtyards (their pads' box)
    // still may not overlap (2026-11-04: `Component::courtless`).
    assert!(!interleaved(&KiCadBoardPlacerConfig {
        tuning: KiCadPlacerTuning {
            solid_pin_rows: false,
            ..Default::default()
        },
        ..KiCadBoardPlacerConfig::default()
    }));
}

#[test]
fn copper_keeps_off_the_edge_where_the_rule_is_zero() {
    // An edge connector held on the left edge of a board whose copper
    // edge clearance is zero: its pads may not touch the outline.
    let pcb = parse(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts"))
        (footprint "SMA" (layer "F.Cu") (at 20 15) (property "Reference" "P4")
            (pad "1" smd rect (at 2 0) (size 4 1.8) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 2 -3.5) (size 4 2.6) (layers "F.Cu") (net 2 "B"))
            (pad "2" smd rect (at 2 3.5) (size 4 2.6) (layers "F.Cu") (net 2 "B"))))"#,
    )
    .unwrap();
    let constraints: crate::placement_constraints::KiCadPlacementConstraints =
        serde_json::from_str(
            r#"{"version": 1, "edge": [{"part": "P4", "edge": "left", "max_mm": 1.5}]}"#,
        )
        .unwrap();
    let config = KiCadBoardPlacerConfig {
        constraints: Some(KiCadConstraintsSource::Inline(constraints)),
        copper_edge_clearance_mm: Some(0.0),
        ..KiCadBoardPlacerConfig::default()
    };
    let lowered = lower_placement(&pcb, &config).unwrap();
    let problem = &lowered.problem;
    assert!(problem.constraints.copper_edge >= 0.01);
    let offsets = core::constraints::copper_edge_offsets(problem, 0, 0.0);
    assert!(offsets[0] >= 0.01, "{offsets:?}");
}

#[test]
fn far_side_pads_keep_off_copper_and_leads_keep_off_bodies() {
    // An SMD part with pads on both sides (a reversible footprint) and
    // a through-hole part, both on the front; a part on the back.
    let board = |holes_error: bool| {
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
            (footprint "Reversible" (layer "F.Cu") (at 15 15) (attr smd)
                (fp_rect (start -2 -1) (end 2 1) (layer "F.CrtYd"))
                (pad "1" smd rect (at -1.5 0) (size 0.8 1) (layers "F.Cu") (net 1 "A"))
                (pad "1" smd rect (at -1.5 0) (size 0.8 1) (layers "B.Cu") (net 1 "A"))
                (pad "2" smd rect (at 1.5 0) (size 0.8 1) (layers "F.Cu") (net 2 "B"))
                (pad "2" smd rect (at 1.5 0) (size 0.8 1) (layers "B.Cu") (net 2 "B")))
            (footprint "Header" (layer "F.Cu") (at 40 15) (attr through_hole)
                (fp_rect (start -2 -1.5) (end 2 1.5) (layer "F.CrtYd"))
                (pad "1" thru_hole circle (at -1.27 0) (size 1.7 1.7) (drill 1) (layers "*.Cu" "*.Mask") (net 1 "A"))
                (pad "2" thru_hole circle (at 1.27 0) (size 1.7 1.7) (drill 1) (layers "*.Cu" "*.Mask") (net 2 "B")))
            (footprint "Back" (layer "B.Cu") (at 30 3)
                (fp_rect (start -3 -0.6) (end 3 0.6) (layer "B.CrtYd"))
                (pad "1" smd rect (at -2.5 0) (size 0.8 0.9) (layers "B.Cu") (net 1 "A"))
                (pad "2" smd rect (at 2.5 0) (size 0.8 0.9) (layers "B.Cu") (net 3 "C"))))"#,
        )
        .unwrap();
        let config = KiCadBoardPlacerConfig {
            holes_in_courtyards_error: Some(holes_error),
            ..Default::default()
        };
        lower_placement(&pcb, &config).unwrap()
    };
    let lowered = board(false);
    let problem = &lowered.problem;
    assert!(
        problem.components[0].far_side.is_empty() && problem.components[0].far_copper.len() == 2
    );
    assert_eq!(
        problem.components[1].far_side.len(),
        2,
        "the header's leads"
    );
    let mut poses = problem.poses.clone();
    let legal = |problem: &core::Problem, poses: &[core::Pose]| {
        !core::legal::illegal_components(problem, poses).contains(&2)
    };
    // The back part's body over the reversible part's back pads, its
    // own pads clear of them: not an error.
    poses[2].position = [15.0, 15.0];
    assert!(legal(problem, &poses));
    // Its pad on theirs: an error.
    poses[2].position = [15.0 + 1.0, 15.0];
    assert!(!legal(problem, &poses));
    // Over the header's leads: the leads stand out of the board.
    poses[2].position = [40.0, 15.0];
    assert!(!legal(problem, &poses));
}

#[test]
fn a_courtyard_drawn_as_several_shapes_keeps_them_apart() {
    let rectangle = |x0: f64, y0: f64, x1: f64, y1: f64| {
        [
            [x0, y0, x1, y0],
            [x1, y0, x1, y1],
            [x1, y1, x0, y1],
            [x0, y1, x0, y0],
        ]
        .iter()
        .map(|[a, b, c, d]| format!("(fp_line (start {a} {b}) (end {c} {d}) (layer \"F.CrtYd\"))"))
        .collect::<String>()
    };
    // An outline marking two connectors: two rectangles, lines in any order.
    let mut lines = rectangle(0.0, 0.0, 10.0, 5.0);
    lines.insert_str(0, &rectangle(40.0, 30.0, 50.0, 40.0));
    let footprint = parse(&format!(
        "(footprint \"Board\" (layer \"F.Cu\") (at 0 0) {lines})"
    ))
    .unwrap();
    let (mut shapes, malformed) = courtyard_shapes_on_side(&footprint, None).unwrap();
    assert!(!malformed);
    shapes.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(
        shapes,
        vec![[0.0, 0.0, 10.0, 5.0], [40.0, 30.0, 50.0, 40.0]]
    );
    // One rectangle is one shape: the body covers it.
    let footprint = parse(&format!(
        "(footprint \"Part\" (layer \"F.Cu\") (at 0 0) {})",
        rectangle(0.0, 0.0, 2.0, 1.0)
    ))
    .unwrap();
    assert_eq!(
        courtyard_shapes_on_side(&footprint, None).unwrap(),
        (Vec::new(), false)
    );
    // A zero-length line at a corner: KiCad cannot build it.
    let footprint = parse(&format!(
        "(footprint \"Part\" (layer \"F.Cu\") (at 0 0) {} (fp_line (start 2 1) (end 2 1) (layer \"F.CrtYd\")))",
        rectangle(0.0, 0.0, 2.0, 1.0)
    ))
    .unwrap();
    assert!(courtyard_shapes_on_side(&footprint, None).unwrap().1);
}

mod glob_tests {
    use super::glob_matches;

    #[test]
    fn globs_match_runs_single_characters_and_escaped_literals() {
        assert!(glob_matches("U*", "U12"));
        assert!(glob_matches("R?", "R1") && !glob_matches("R?", "R12"));
        assert!(glob_matches("REF**", "REF**_2"));
        assert!(
            glob_matches("REF\\*\\*", "REF**")
                && !glob_matches("REF\\*\\*", "REF**_2")
                && !glob_matches("REF\\*\\*", "REF12")
        );
    }
}

mod order_tests {
    use super::*;

    /// A small board in a given order: twelve parts on an 8-pin chip's
    /// nets, each with pads and graphics; `reversed` writes the footprints,
    /// and each footprint's pads and graphics, the other way round.
    fn board(reversed: bool) -> String {
        let order = |mut items: Vec<String>| {
            if reversed {
                items.reverse();
            }
            items.concat()
        };
        let part = |reference: &str,
                    uuid: &str,
                    at: [f64; 2],
                    pads: Vec<String>,
                    half: [f64; 2]| {
            let graphics = vec![
                format!(
                    r#"(fp_rect (start {} {}) (end {} {}) (layer "F.CrtYd") (uuid "{uuid}-c"))"#,
                    -half[0], -half[1], half[0], half[1]
                ),
                format!(
                    r#"(fp_line (start {} {}) (end {} {}) (layer "F.SilkS") (uuid "{uuid}-s1"))"#,
                    -half[0], -half[1], half[0], -half[1]
                ),
                format!(
                    r#"(fp_line (start {} {}) (end {} {}) (layer "F.SilkS") (uuid "{uuid}-s2"))"#,
                    -half[0], half[1], half[0], half[1]
                ),
                format!(
                    r#"(fp_line (start {} {}) (end {} {}) (layer "F.Fab") (uuid "{uuid}-f"))"#,
                    -half[0], 0.0, half[0], 0.0
                ),
            ];
            format!(
                r#"(footprint "Lib:{reference}" (layer "F.Cu") (uuid "{uuid}") (at {} {})
                    (property "Reference" "{reference}") (attr smd) {} {})"#,
                at[0],
                at[1],
                order(graphics),
                order(pads)
            )
        };
        let pad = |number: &str, x: f64, y: f64, net: usize| {
            format!(
                r#"(pad "{number}" smd rect (at {x} {y}) (size 0.6 0.9) (layers "F.Cu") (net {net} "N{net}"))"#
            )
        };
        let mut footprints = Vec::new();
        let chip: Vec<String> = (1..=8)
            .map(|pin| {
                let (x, y) = if pin <= 4 {
                    (-2.5, pin as f64 - 2.5)
                } else {
                    (2.5, 6.5 - pin as f64)
                };
                pad(&pin.to_string(), x, y, pin)
            })
            .collect();
        footprints.push(part("U1", "a1", [25.0, 20.0], chip, [3.5, 2.5]));
        for index in 1..=8 {
            let pads = vec![
                pad("1", -0.8, 0.0, index),
                pad("2", 0.8, 0.0, index % 8 + 1),
            ];
            footprints.push(part(
                &format!("R{index}"),
                &format!("b{index}"),
                [5.0 + 4.0 * index as f64, 35.0],
                pads,
                [1.5, 0.8],
            ));
        }
        for index in 1..=3 {
            let pads = vec![pad("1", -0.8, 0.0, index * 2), pad("2", 0.8, 0.0, 9)];
            footprints.push(part(
                &format!("C{index}"),
                &format!("c{index}"),
                [45.0, 5.0 + 4.0 * index as f64],
                pads,
                [1.5, 0.8],
            ));
        }
        format!(
            r#"(kicad_pcb (version 20240108)
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (net 0 "") {}
            (gr_rect (start 0 0) (end 50 40) (layer "Edge.Cuts"))
            {})"#,
            (1..=9)
                .map(|net| format!(r#"(net {net} "N{net}")"#))
                .collect::<String>(),
            order(footprints)
        )
    }

    #[test]
    fn the_lowering_does_not_depend_on_the_order_of_the_file() {
        let (forward, backward) = (board(false), board(true));
        assert_ne!(forward, backward);
        let config = KiCadBoardPlacerConfig::default();
        // As read, the parts come in the file's order.
        let raw = |text: &str| {
            lower_placement(&parse(text).unwrap(), &config)
                .unwrap()
                .references
        };
        assert_ne!(raw(&forward), raw(&backward));
        // In the canonical order the problems are the same.
        let lowered = |text: &str| {
            let lowered =
                lower_placement(&crate::canonical_order::parse_board(text).unwrap(), &config)
                    .unwrap();
            (lowered.references.clone(), format!("{:?}", lowered.problem))
        };
        let (references, problem) = lowered(&forward);
        assert_eq!(
            references,
            [
                "C1", "C2", "C3", "R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8", "U1"
            ]
        );
        assert_eq!((references, problem), lowered(&backward));
    }

    #[test]
    fn a_placement_does_not_depend_on_the_order_of_the_file() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("pcb-maker-order-{}-{nonce}", std::process::id()));
        let config = KiCadBoardPlacerConfig {
            tuning: KiCadPlacerTuning {
                placement_seeds: 2,
                ..Default::default()
            },
            ..KiCadBoardPlacerConfig::default()
        };
        let mut placed = Vec::new();
        for (name, reversed) in [("forward", false), ("backward", true)] {
            let source = root.join(format!("{name}-source"));
            fs::create_dir_all(&source).unwrap();
            fs::write(source.join("board.kicad_pcb"), board(reversed)).unwrap();
            let output = root.join(format!("{name}-placed"));
            let result = place_kicad_board(&source, "board", &output, &config).unwrap();
            assert!(
                result.unplaced.is_empty() && result.illegal.is_empty(),
                "{:?} {:?}",
                result.unplaced,
                result.illegal
            );
            placed.push((
                result.wirelength_final_mm,
                fs::read_to_string(output.join("board.kicad_pcb")).unwrap(),
            ));
        }
        let _ = fs::remove_dir_all(&root);
        // The same poses, and the same board written: the output is in the
        // canonical order whatever order the input was in (the writer's
        // courtyard order comes from the content: `source_order`).
        assert_eq!(placed[0].0, placed[1].0);
        assert!(placed[0].1 == placed[1].1, "the placed boards differ");
    }
}

#[test]
fn a_printed_antennas_copper_is_its_body() {
    // The Business-Cards' AE1 (TI CC2430DB inverted-F antenna): three small
    // pads, no courtyard, 18 mm of copper beside them. Its body was the
    // pads' box, so the layout hung the copper over the board edge
    // (copper_edge_clearance on WLED_Matrix and Batch_1).
    let pcb = parse(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
        (footprint "Inverted_F_Antenna" (layer "F.Cu") (at 30 15) (property "Reference" "AE1")
            (fp_poly (pts (xy 25.4 1.0) (xy 8.98 1.0) (xy 8.98 3.0) (xy 6.98 3.0) (xy 6.98 4.0) (xy 25.4 4.0))
                (stroke (width 0) (type solid)) (fill yes) (layer "F.Cu"))
            (fp_line (start 10.77 4.62) (end 9.71 3.56) (stroke (width 0.46) (type solid)) (layer "F.Cu"))
            (pad "1" smd roundrect (at 10.77 6.9 270) (size 1 0.46) (layers "F.Cu" "F.Mask") (roundrect_rratio 0.25) (net 1 "RF"))
            (pad "2" smd roundrect (at 7.41 7 270) (size 0.2 1.15) (layers "F.Cu" "F.Mask") (roundrect_rratio 0.25) (net 2 "GND"))
            (pad "2" smd roundrect (at 9.48 7 270) (size 0.2 1) (layers "F.Cu" "F.Mask") (roundrect_rratio 0.25) (net 2 "GND"))))"#,
    )
    .unwrap();
    let placement = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let body = &placement.problem.components[0];
    // The copper reaches from x 6.98 to 25.4 and y 1.0 to 4.0, the pads
    // to y 7.6: the body holds all of it.
    assert!(
        body.body_size[0] >= 25.4 - 6.98 - 1.0e-6,
        "{:?}",
        body.body_size
    );
    assert!(body.body_center[0] - body.body_size[0] / 2.0 <= 6.98 + 1.0e-6);
    assert!(body.body_center[1] - body.body_size[1] / 2.0 <= 1.0 + 1.0e-6);
}

#[test]
fn a_decoupling_capacitor_is_near_every_supply_pin_of_its_rail_with_decoupling_near() {
    // U1 with two +3V3 pins and a GND pin; C1 between +3V3 and GND.
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
        (footprint "QFN" (layer "F.Cu") (at 10 10) (property "Reference" "U1")
            (fp_rect (start -2 -2) (end 2 2) (layer "F.CrtYd"))
            (pad "1" smd rect (at -1.5 0) (size 0.3 0.8) (layers "F.Cu") (net 1 "+3V3") (pintype "power_in"))
            (pad "2" smd rect (at 1.5 0) (size 0.3 0.8) (layers "F.Cu") (net 1 "+3V3") (pintype "power_in"))
            (pad "3" smd rect (at 0 1.5) (size 0.8 0.3) (layers "F.Cu") (net 2 "GND"))
            (pad "4" smd rect (at 0 -1.5) (size 0.8 0.3) (layers "F.Cu") (net 3 "A")))
        (footprint "C_0402" (layer "F.Cu") (at 30 10) (property "Reference" "C1")
            (property "Value" "100nF")
            (fp_rect (start -0.9 -0.5) (end 0.9 0.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "+3V3"))
            (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 2 "GND"))))"#,
    )
    .unwrap();
    let relation = |config: &KiCadBoardPlacerConfig| {
        let lowered = lower_placement(&pcb, config).unwrap();
        let constraints = &lowered.problem.constraints;
        assert_eq!(constraints.follow_anchors, config.decoupling_near);
        match constraints.relations.last().unwrap() {
            core::constraints::Relation::NearAny { part, anchors, max } => {
                (*part, anchors.len(), *max)
            }
            other => panic!("{other:?}"),
        }
    };
    let off = KiCadBoardPlacerConfig::default();
    assert!(!off.decoupling_near);
    assert_eq!(relation(&off), (1, 1, 2.5));
    let on = KiCadBoardPlacerConfig {
        decoupling_near: true,
        ..KiCadBoardPlacerConfig::default()
    };
    assert_eq!(relation(&on), (1, 2, 1.0));
}

#[test]
fn a_capacitor_named_after_its_protector_is_pulled_to_its_ic_not_the_connector() {
    // J1 (USB, VBUS), U1 a TPD1S514 on VBUS, C1 "1u TPD1S514 stability"
    // between VBUS and GND (fresh-agent trial 5's ducktop2 right_io C1771),
    // U2 a regulator fed from VBUS: U1 is the protector pulled to J1; C1
    // decouples U2's supply pin, as the score says (`esd_protector`, one
    // rule for both).
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
        (footprint "USB" (layer "F.Cu") (at 2 10) (property "Reference" "J1") (property "Value" "USB_C")
            (fp_rect (start -1 -1) (end 1 2) (layer "F.CrtYd"))
            (pad "1" smd rect (at 0 0) (size 0.6 0.7) (layers "F.Cu") (net 1 "VBUS"))
            (pad "2" smd rect (at 0 1) (size 0.6 0.7) (layers "F.Cu") (net 2 "GND")))
        (footprint "SOT-23" (layer "F.Cu") (at 20 10) (property "Reference" "U1") (property "Value" "TPD1S514")
            (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at -1 0) (size 0.6 0.7) (layers "F.Cu") (net 1 "VBUS") (pintype "power_in"))
            (pad "2" smd rect (at 1 0) (size 0.6 0.7) (layers "F.Cu") (net 2 "GND"))
            (pad "3" smd rect (at 0 1) (size 0.6 0.7) (layers "F.Cu") (net 3 "VOUT")))
        (footprint "C_0603" (layer "F.Cu") (at 30 10) (property "Reference" "C1") (property "Value" "1u TPD1S514 stability")
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "VBUS"))
            (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 2 "GND")))
        (footprint "SOT-223" (layer "F.Cu") (at 36 10) (property "Reference" "U2") (property "Value" "AMS1117")
            (fp_rect (start -2 -2) (end 2 2) (layer "F.CrtYd"))
            (pad "1" smd rect (at -1 0) (size 0.6 0.7) (layers "F.Cu") (net 1 "VBUS") (pintype "power_in"))
            (pad "2" smd rect (at 1 0) (size 0.6 0.7) (layers "F.Cu") (net 2 "GND"))
            (pad "3" smd rect (at 0 1) (size 0.6 0.7) (layers "F.Cu") (net 4 "+3V3"))))"#,
    )
    .unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let pulls: Vec<(usize, Vec<usize>)> = lowered
        .problem
        .constraints
        .relations
        .iter()
        .filter_map(|relation| match relation {
            core::constraints::Relation::NearAny { part, anchors, .. } => Some((
                *part,
                anchors
                    .iter()
                    .map(|anchor| match anchor {
                        core::constraints::Anchor::Point(to, _) => *to,
                        other => panic!("{other:?}"),
                    })
                    .collect(),
            )),
            _ => None,
        })
        .collect();
    // C1 (2) near U2 (3), U1 near J1 (0); nothing pulls C1 to J1.
    assert!(pulls.contains(&(2, vec![3])), "{pulls:?}");
    assert!(pulls.contains(&(1, vec![0])), "{pulls:?}");
    assert!(
        !pulls.iter().any(|(part, to)| *part == 2 && to.contains(&0)),
        "{pulls:?}"
    );
}

#[test]
fn pads_keep_the_hole_clearance_from_a_non_plated_hole() {
    // A mouse bite (0.5 mm holes in a 0.7 mm courtyard) and a capacitor
    // whose courtyard may touch others (`tight_bodies`): OpenRX's panel,
    // where the hole clearance 0.2 less the copper clearance 0.09 left a
    // ring of 0.11 mm reaching 0.01 mm past the courtyard, and a pad came
    // 0.19 mm to a hole.
    let pcb = parse(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts"))
        (footprint "MouseBite" (layer "F.Cu") (at 20 15)
            (fp_rect (start -1.2 -0.35) (end 1.2 0.35) (layer "F.CrtYd"))
            (pad "" np_thru_hole circle (at -0.85 0) (size 0.5 0.5) (drill 0.5) (layers "*.Cu" "*.Mask"))
            (pad "" np_thru_hole circle (at 0 0) (size 0.5 0.5) (drill 0.5) (layers "*.Cu" "*.Mask"))
            (pad "" np_thru_hole circle (at 0.85 0) (size 0.5 0.5) (drill 0.5) (layers "*.Cu" "*.Mask")))
        (footprint "C" (layer "F.Cu") (at 10 10)
            (fp_rect (start -0.91 -0.46) (end 0.91 0.46) (layer "F.CrtYd"))
            (fp_rect (start -0.5 -0.25) (end 0.5 0.25) (layer "F.Fab"))
            (pad "1" smd rect (at -0.48 0) (size 0.56 0.62) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0.48 0) (size 0.56 0.62) (layers "F.Cu") (net 2 "B"))))"#,
    )
    .unwrap();
    let config = KiCadBoardPlacerConfig {
        copper_clearance_mm: Some(0.09),
        hole_clearance_mm: Some(0.2),
        tight_bodies: Some(true),
        ..KiCadBoardPlacerConfig::default()
    };
    let lowered = lower_placement(&pcb, &config).unwrap();
    // The bite keeps its rings: 0.25 + 0.11 mm about each hole.
    let bite = &lowered.problem.components[0];
    assert_eq!(bite.hole_rings.len(), 3);
    assert!((bite.hole_rings[1][3] - 0.36).abs() < 1.0e-9);
    // At the last level (tight bodies, courtyards touching where the
    // copper keeps the clearance), a pad under the middle hole keeps the
    // hole clearance from it.
    let mut problem = lowered.problem.clone();
    problem.spacing = problem.min_spacing;
    problem.grid = 0.01;
    problem.constraints.courtyard_spacing = true;
    for component in &mut problem.components {
        component.halo = 0.0;
        component.use_tight_body();
    }
    let mut poses = problem.poses.clone();
    // The capacitor's pad 1 under the hole at (20, 15): its top edge at
    // y - 0.31, the hole's bottom edge at 15.25.
    let at_gap = |gap: f64| [20.48, 15.25 + gap + 0.31];
    let legal =
        |poses: &[core::Pose]| !core::legal::illegal_components(&problem, poses).contains(&1);
    poses[1].position = at_gap(0.19);
    assert!(!legal(&poses), "0.19 mm from the hole is legal");
    poses[1].position = at_gap(0.201);
    let why = core::legal::conflict_kind(&problem, &poses, 1, poses[1], 0);
    assert!(
        legal(&poses),
        "0.201 mm from the hole is not legal: {why:?}"
    );
}

#[test]
fn a_holes_own_mask_margin_keeps_far_side_pads_out_of_its_opening() {
    // vaio_re's J1, a Pi Zero socket on the front: its 2.3 mm mounting
    // holes open the mask 1.625 mm around (`solder_mask_margin`) on both
    // sides, and KiCad 10.0.7 bridged a back part's pads under that
    // opening (solder_mask_bridge "NPTH pad of J1 || Pad 5 of U16").
    let lowered = |margin: &str| {
        let pcb = parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (setup (pad_to_mask_clearance 0))
            (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
            (footprint "Socket" (layer "F.Cu") (at 20 15) (attr through_hole)
                (property "Reference" "J1")
                (fp_rect (start -4 -4) (end 4 4) (layer "F.CrtYd"))
                (pad "" np_thru_hole circle (at 0 0) (size 2.3 2.3) (drill 2.3) (layers "*.Cu" "*.Mask") {margin}))
            (footprint "Reg" (layer "B.Cu") (at 50 15)
                (property "Reference" "U1")
                (fp_rect (start -1.6 -0.6) (end 1.6 0.6) (layer "B.CrtYd"))
                (pad "1" smd rect (at -1 0) (size 0.6 0.6) (layers "B.Cu" "B.Mask") (net 1 "A"))
                (pad "2" smd rect (at 1 0) (size 0.6 0.6) (layers "B.Cu" "B.Mask") (net 2 "B"))))"#
        ))
        .unwrap();
        lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap()
    };
    let index = |lowered: &LoweredPlacement, name: &str| {
        lowered
            .problem
            .components
            .iter()
            .position(|component| component.name == name)
            .unwrap()
    };
    let with = lowered("(solder_mask_margin 1.625)");
    let socket = &with.problem.components[index(&with, "J1")];
    // The hole's copper box, and its opening less the copper clearance
    // the far-side test adds: 1.15 + 1.625 - 0.2.
    assert_eq!(socket.far_copper.len(), 2, "{:?}", socket.far_copper);
    assert!((socket.far_copper[1][2] - 2.575).abs() < 1.0e-9);
    let without = lowered("");
    assert_eq!(
        without.problem.components[index(&without, "J1")]
            .far_copper
            .len(),
        1,
        "no margin of its own: the copper alone"
    );
    let legal_at = |lowered: &LoweredPlacement, y: f64| {
        let back = index(lowered, "U1");
        let mut poses = lowered.problem.poses.clone();
        poses[back].position = [20.0, y];
        !core::legal::illegal_components(&lowered.problem, &poses).contains(&back)
    };
    // U1's pads 0.55 mm from the hole's copper, inside its opening.
    assert!(!legal_at(&with, 17.0));
    assert!(legal_at(&without, 17.0));
    // 1.1 mm beyond the opening.
    assert!(legal_at(&with, 19.0));
}

#[test]
fn a_parts_copper_graphics_keep_the_edge_margin_from_a_cutout() {
    // osprey's AE1, a PCB antenna: its meander is a B.Cu polygon beyond
    // its pads, and the placement put it over the battery cutout's edge
    // (copper_edge_clearance 0.0 mm against 0.25). A cutout lets bodies
    // reach over it but keeps copper out: graphics as well as pads.
    let pcb = parse(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
        (gr_rect (start 30 10) (end 40 20) (layer "Edge.Cuts"))
        (footprint "Antenna" (layer "F.Cu") (at 10 25)
            (property "Reference" "AE1")
            (fp_rect (start -1 -1) (end 7 1) (layer "F.CrtYd"))
            (fp_poly (pts (xy 0.5 -0.5) (xy 6 -0.5) (xy 6 0.5) (xy 0.5 0.5)) (layer "F.Cu") (width 0) (fill solid))
            (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "RF"))))"#,
    )
    .unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let antenna = lowered
        .problem
        .components
        .iter()
        .position(|component| component.name == "AE1")
        .unwrap();
    assert_eq!(
        lowered.problem.components[antenna].copper_graphics,
        vec![[0.5, -0.5, 6.0, 0.5]]
    );
    let legal_at = |x: f64| {
        let mut poses = lowered.problem.poses.clone();
        poses[antenna].position = [x, 15.0];
        !core::legal::illegal_components(&lowered.problem, &poses).contains(&antenna)
    };
    // The pad 5 mm before the cutout, the meander over its edge.
    assert!(!legal_at(25.0));
    // The meander 4 mm before it.
    assert!(legal_at(20.0));
}

#[test]
fn a_fixed_part_on_many_marks_is_one_warning() {
    let one = super::lower::fixed_on_marks_warning(
        "J5",
        &["text \"Framework kVm shim v2\" (F.SilkS at 113.00, 83.42)".to_string()],
    );
    assert_eq!(
        one,
        "fixed part J5 lies on the board's text \"Framework kVm shim v2\" (F.SilkS at 113.00, 83.42): both stay where the input put them"
    );
    let marks: Vec<String> = (1..=5)
        .map(|n| format!("text \"{n}\" (F.SilkS at {n}.00, 1.00)"))
        .collect();
    let many = super::lower::fixed_on_marks_warning("U3", &marks);
    assert!(
        many.starts_with(
            "fixed part U3 lies on 5 of the board's texts and silkscreen graphics (text \"1\""
        ),
        "{many}"
    );
    assert!(many.contains("; 2 more): all stay"), "{many}");
}

/// A 5 x 5 ball grid at 0.5 mm pitch (0.3 mm balls) on the front at
/// `at`, locked: the centre ball C3 on `X`, B2 on `Y` (both enclosed),
/// the others unconnected; R1 on the front carries the nets' other ends,
/// and `back` (footprints on the back) joins them.
fn far_side_board(back: &str) -> Expr {
    let mut balls = String::new();
    for (row, letter) in ["A", "B", "C", "D", "E"].iter().enumerate() {
        for column in 0..5 {
            let number = format!("{letter}{}", column + 1);
            let net = match number.as_str() {
                "C3" => "X".to_string(),
                "B2" => "Y".to_string(),
                other => format!("unconnected-(U1-{other})"),
            };
            balls.push_str(&format!(
                r#"(pad "{number}" smd circle (at {} {}) (size 0.3 0.3) (layers "F.Cu" "F.Mask") (net "{net}"))"#,
                (column as f64 - 2.0) * 0.5,
                (row as f64 - 2.0) * 0.5
            ));
        }
    }
    parse(&format!(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
             (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
             (footprint "Package_BGA:BGA-25" (layer "F.Cu") (locked) (at 10 10)
                 (property "Reference" "U1")
                 (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd"))
                 {balls})
             (footprint "R" (layer "F.Cu") (at 30 10) (property "Reference" "R1")
                 (fp_rect (start -0.5 -0.5) (end 1.5 0.5) (layer "F.CrtYd"))
                 (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net "X"))
                 (pad "2" smd rect (at 1 0) (size 0.6 0.6) (layers "F.Cu") (net "Y")))
             {back})"#
    ))
    .unwrap()
}

/// The router's rules of `far_side_board`: U1 admitted (as the designer's
/// via in its pads would), the project's vias 0.35/0.2 and 0.8/0.4.
fn far_side_rules(admitted: bool) -> KiCadBoardRouterConfig {
    KiCadBoardRouterConfig {
        default_rules: Some(KiCadConnectionRoutingRules {
            trace_width_mm: 0.1,
            clearance_mm: 0.1,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }),
        neck_width_mm: Some(0.1),
        hole_clearance_mm: 0.1,
        hole_to_hole_clearance_mm: 0.2,
        via_in_pad: admitted.then(|| {
            KiCadViaInPad::Parts(vec![KiCadViaInPadEntry {
                part: Some("U1".into()),
                ..Default::default()
            }])
        }),
        project_rules: Some(crate::project_rules::KiCadProjectRulesReport {
            via_sizes: vec![[0.35, 0.2], [0.8, 0.4]],
            via_minimums: [0.25, 0.15, 0.05],
            ..Default::default()
        }),
        ..KiCadBoardRouterConfig::default()
    }
}

fn far_side_config(rules: Option<KiCadBoardRouterConfig>) -> KiCadBoardPlacerConfig {
    KiCadBoardPlacerConfig {
        copper_clearance_mm: Some(0.1),
        hole_clearance_mm: Some(0.1),
        router_rules: rules.map(std::sync::Arc::new),
        tuning: KiCadPlacerTuning {
            grid_mm: 0.1,
            ..Default::default()
        },
        ..KiCadBoardPlacerConfig::default()
    }
}

#[test]
fn a_part_on_the_far_side_of_an_admitted_bgas_via_in_pad_is_illegal_and_moves() {
    // An 0603 on the back right under U1's centre ball, its nets joined by
    // a resistor on the back elsewhere.
    let pcb = far_side_board(
        r#"(footprint "C_0603" (layer "B.Cu") (at 10 10) (property "Reference" "C1")
               (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "B.CrtYd"))
               (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "B.Cu") (net "P"))
               (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "B.Cu") (net "Q")))
           (footprint "R_0603" (layer "B.Cu") (at 20 10) (property "Reference" "R2")
               (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "B.CrtYd"))
               (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "B.Cu") (net "P"))
               (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "B.Cu") (net "Q")))"#,
    );
    let (bga, cap) = (0, 2);
    // Without the rule (or with U1 not admitted) the back under U1 is free.
    for rules in [None, Some(far_side_rules(false))] {
        let lowered = lower_placement(&pcb, &far_side_config(rules)).unwrap();
        let problem = &lowered.problem;
        assert!(problem.components[bga].far_side.is_empty());
        assert!(!core::legal::illegal_components(problem, &problem.poses).contains(&cap));
    }
    // Admitted: B2 and C3 (enclosed) each hold a 0.35/0.2 via, the largest
    // size that keeps its clearances among U1's balls; each is a far-side
    // box of half the via's diameter (the drill's hole clearance less the
    // copper clearance is less).
    let lowered = lower_placement(&pcb, &far_side_config(Some(far_side_rules(true)))).unwrap();
    let problem = &lowered.problem;
    let far = &problem.components[bga].far_side;
    assert_eq!(far.len(), 2, "{far:?}");
    for (got, center) in far.iter().zip([[-0.5, -0.5], [0.0, 0.0]]) {
        let want = [
            center[0] - 0.175,
            center[1] - 0.175,
            center[0] + 0.175,
            center[1] + 0.175,
        ];
        assert!(
            got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1.0e-9),
            "{got:?} against {want:?}"
        );
    }
    let mut poses = problem.poses.clone();
    let illegal = core::legal::illegal_components(problem, &poses);
    assert!(illegal.contains(&cap), "{illegal:?}");
    // The legaliser moves it off the vias; the front stays as it was.
    let failed = core::legal::legalize(problem, &mut poses);
    assert!(failed.is_empty(), "{failed:?}");
    assert!(core::legal::illegal_components(problem, &poses).is_empty());
    assert_ne!(poses[cap], problem.poses[cap]);
    assert_eq!(poses[bga], problem.poses[bga]);
    // The switch off: the same problem as without the rules.
    let mut off = far_side_config(Some(far_side_rules(true)));
    off.tuning.via_in_pad_far_side = false;
    let off = lower_placement(&pcb, &off).unwrap();
    let none = lower_placement(&pcb, &far_side_config(None)).unwrap();
    assert_eq!(format!("{:?}", off.problem), format!("{:?}", none.problem));
}

/// `far_side_board`'s rules with a `length` rule of maximum `max` over
/// net `X`.
fn length_rules(max: f64) -> KiCadBoardRouterConfig {
    KiCadBoardRouterConfig {
        length_matching: Some(true),
        length_rules: vec![crate::board_router::KiCadLengthRule {
            rule: "bus".into(),
            constraint: "length".into(),
            min: Some(max - 1.0),
            max: Some(max),
            nets: vec![crate::board_router::KiCadLengthMember {
                net: "X".into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..far_side_rules(false)
    }
}

#[test]
fn a_length_bounded_nets_pins_are_kept_within_its_maximum() {
    // R3 on X (U1's centre ball) and on three nets to J1, locked at the
    // board's right edge: its wire pulls it to J1, 27 mm from U1.
    let pcb = far_side_board(
        r#"(footprint "R" (layer "F.Cu") (at 30 5) (property "Reference" "R3")
               (fp_rect (start -0.5 -0.5) (end 3.5 0.5) (layer "F.CrtYd"))
               (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net "X"))
               (pad "2" smd rect (at 1 0) (size 0.6 0.6) (layers "F.Cu") (net "W1"))
               (pad "3" smd rect (at 2 0) (size 0.6 0.6) (layers "F.Cu") (net "W2"))
               (pad "4" smd rect (at 3 0) (size 0.6 0.6) (layers "F.Cu") (net "W3")))
           (footprint "J" (layer "F.Cu") (locked) (at 37 15) (property "Reference" "J1")
               (fp_rect (start -0.5 -0.5) (end 2.5 0.5) (layer "F.CrtYd"))
               (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net "W1"))
               (pad "2" smd rect (at 1 0) (size 0.6 0.6) (layers "F.Cu") (net "W2"))
               (pad "3" smd rect (at 2 0) (size 0.6 0.6) (layers "F.Cu") (net "W3")))"#,
    );
    let (bga, r1, r3) = (0, 1, 2);
    let lowered = lower_placement(&pcb, &far_side_config(Some(length_rules(15.0)))).unwrap();
    let problem = &lowered.problem;
    // X joins U1.C3 (fixed), R1.1 and R3.1: a relation for each two of
    // them, the movable part first (the smaller of two movable ones), all
    // automatic.
    let pairs: Vec<(usize, usize, f64, f64)> = problem
        .constraints
        .relations
        .iter()
        .map(|relation| match relation {
            core::constraints::Relation::Within {
                part,
                other,
                soft,
                max,
                ..
            } => (*part, *other, *soft, *max),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        pairs,
        [
            (r1, bga, 12.0, 15.0),
            (r3, bga, 12.0, 15.0),
            (r1, r3, 12.0, 15.0)
        ]
    );
    assert_eq!(problem.constraints.automatic, 3);
    assert_eq!(problem.constraints.relation_weight, 50.0);
    let relation = &problem.constraints.relations[1];
    // R3's pin 20 mm from U1's: 8 mm beyond the soft bound, 5 beyond the
    // maximum.
    let violation = |x: f64| {
        let mut poses = problem.poses.clone();
        poses[r3].position = [x, 10.0];
        core::constraints::relation_violation(problem, &poses, relation)
    };
    assert!((violation(30.0) - (8.0 + 20.0 * 5.0)).abs() < 1.0e-9);
    // Between the soft bound and the maximum only the distance beyond the
    // soft bound counts; within it nothing.
    assert!((violation(24.0) - 2.0).abs() < 1.0e-9);
    assert_eq!(violation(20.0), 0.0);
    let x_pin = |problem: &core::Problem, index: usize, poses: &[core::Pose]| {
        let component = &problem.components[index];
        let pin = component
            .pins
            .iter()
            .find(|pin| lowered.net_names[pin.net] == "X")
            .unwrap();
        component.pin_position(pin, poses[index])
    };
    // The refinement: without the rule R3 goes to J1, beyond the maximum;
    // with it R3 stays within.
    let none = lower_placement(&pcb, &far_side_config(None)).unwrap();
    assert!(none.problem.constraints.relations.is_empty());
    let mut free = none.problem.poses.clone();
    core::legal::refine(&none.problem, &mut free, 8);
    let mut bounded = problem.poses.clone();
    core::legal::refine(problem, &mut bounded, 8);
    let apart = |problem: &core::Problem, poses: &[core::Pose]| {
        core::constraints::octilinear(x_pin(problem, r3, poses), x_pin(problem, bga, poses))
    };
    assert!(apart(&none.problem, &free) > 15.0, "{:?}", free[r3]);
    assert!(apart(problem, &bounded) <= 15.0, "{:?}", bounded[r3]);
    // The switch off: the same problem as without the rule.
    let mut off = far_side_config(Some(length_rules(15.0)));
    off.length_maxima = false;
    let off = lower_placement(&pcb, &off).unwrap();
    assert_eq!(format!("{:?}", off.problem), format!("{:?}", none.problem));
}

#[test]
fn a_length_minimums_two_parts_are_held_apart_and_reported() {
    // J1 locked at the left; R5 joined to it by X (a length rule of 20 to
    // 22 mm) and by three more nets that pull it right beside J1.
    let pcb = parse(
        r#"(kicad_pcb (layers (0 "F.Cu") (31 "B.Cu") (44 "Edge.Cuts" user))
             (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
             (footprint "J" (layer "F.Cu") (locked) (at 5 10) (property "Reference" "J1")
                 (fp_rect (start -0.5 -0.5) (end 3.5 0.5) (layer "F.CrtYd"))
                 (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net "X"))
                 (pad "2" smd rect (at 1 0) (size 0.6 0.6) (layers "F.Cu") (net "W1"))
                 (pad "3" smd rect (at 2 0) (size 0.6 0.6) (layers "F.Cu") (net "W2"))
                 (pad "4" smd rect (at 3 0) (size 0.6 0.6) (layers "F.Cu") (net "W3")))
             (footprint "R" (layer "F.Cu") (at 10 12) (property "Reference" "R5")
                 (fp_rect (start -0.5 -0.5) (end 3.5 0.5) (layer "F.CrtYd"))
                 (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net "X"))
                 (pad "2" smd rect (at 1 0) (size 0.6 0.6) (layers "F.Cu") (net "W1"))
                 (pad "3" smd rect (at 2 0) (size 0.6 0.6) (layers "F.Cu") (net "W2"))
                 (pad "4" smd rect (at 3 0) (size 0.6 0.6) (layers "F.Cu") (net "W3"))))"#,
    )
    .unwrap();
    let mut rules = length_rules(22.0);
    rules.length_rules[0].min = Some(20.0);
    let (j1, r5) = (0, 1);
    let mut on = far_side_config(Some(rules.clone()));
    on.length_minima = true;
    let lowered = lower_placement(&pcb, &on).unwrap();
    let problem = &lowered.problem;
    // One relation: R5's X pin at least half the minimum (10 mm) from
    // J1's, within 0.8 of the maximum (17.6 mm).
    let relations: Vec<(usize, usize, f64, f64, f64)> = problem
        .constraints
        .relations
        .iter()
        .map(|relation| match relation {
            core::constraints::Relation::Within {
                part,
                other,
                soft,
                max,
                floor,
                ..
            } => (*part, *other, *soft, *max, *floor),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(relations, [(r5, j1, 0.8 * 22.0, 22.0, 10.0)]);
    let x_apart = |problem: &core::Problem, poses: &[core::Pose]| {
        let at = |index: usize| {
            let component = &problem.components[index];
            let pin = &component.pins[0];
            component.pin_position(pin, poses[index])
        };
        core::constraints::octilinear(at(r5), at(j1))
    };
    // The refinement: without the minimum R5 sits by J1, with it its X
    // pin keeps about 10 mm off (the floor's price
    // against the three other nets' pull).
    let mut maxima_only = far_side_config(Some(rules.clone()));
    maxima_only.length_minima = false;
    let maxima_only = lower_placement(&pcb, &maxima_only).unwrap();
    let mut near = maxima_only.problem.poses.clone();
    core::legal::refine(&maxima_only.problem, &mut near, 8);
    let mut apart = problem.poses.clone();
    core::legal::refine(problem, &mut apart, 8);
    assert!(
        x_apart(&maxima_only.problem, &near) < 10.0,
        "{:?}",
        near[r5]
    );
    assert!(
        x_apart(problem, &apart) >= 9.0
            && x_apart(problem, &apart) > x_apart(&maxima_only.problem, &near) + 3.0,
        "{:?}",
        apart[r5]
    );
    // board-placer.json's length_rules says what was held and where the
    // pins ended.
    let report = super::lower::length_rule_report(&lowered, &apart);
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].net, "X");
    assert_eq!(report[0].held, "both");
    assert_eq!(report[0].apart_mm, Some(10.0));
    assert_eq!(report[0].parts, ["J1", "R5"]);
    let close = super::lower::length_rule_report(&lowered, &near);
    assert!(!close[0].kept && close[0].note.contains("closer than half the minimum"));
    // Both switches off, or no rule: the problem without the relation.
    let none = lower_placement(&pcb, &far_side_config(None)).unwrap();
    let mut off = far_side_config(Some(rules));
    off.length_minima = false;
    off.length_maxima = false;
    let off = lower_placement(&pcb, &off).unwrap();
    assert_eq!(format!("{:?}", off.problem), format!("{:?}", none.problem));
    assert!(off.length_rules.is_empty());
}

/// A PG1316-like key without a courtyard (Goga64 ULK): pads in its
/// corners, a 16 mm fabrication square, a peg hole on Edge.Cuts.
fn courtless_key(reference: &str, x: f64, y: f64) -> String {
    format!(
        r#"(footprint "PG1316" (layer "F.Cu") (at {x} {y})
        (property "Reference" "{reference}")
        (fp_rect (start -8 -8) (end 8 8) (layer "F.Fab") (stroke (width 0.1)))
        (fp_circle (center -5.8 2.75) (end -5.3 2.75) (layer "Edge.Cuts") (stroke (width 0.05)))
        (pad "1" smd rect (at -2.5 2.65) (size 1 1) (layers "F.Cu") (net 1 "A"))
        (pad "2" smd rect (at 2.5 2.65) (size 1 1) (layers "F.Cu") (net 2 "B"))
        (pad "3" smd rect (at -6.35 -6) (size 1.3 2) (layers "F.Cu"))
        (pad "3" smd rect (at 6.35 6) (size 1.3 2) (layers "F.Cu")))"#
    )
}

#[test]
fn rows_of_keys_without_courtyards_may_not_lie_on_each_other_and_carry_their_slots() {
    // Two rows of two keys; the second row 1.27 mm below the first, 9.5
    // mm across (the trial's thumb row on the grid's last row).
    let pcb = parse(&format!(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 100 60) (layer "Edge.Cuts"))
        {} {} {} {})"#,
        courtless_key("SW1", 20.0, 20.0),
        courtless_key("SW2", 39.05, 20.0),
        courtless_key("SW3", 29.5, 21.27),
        courtless_key("SW4", 48.55, 21.27),
    ))
    .unwrap();
    let constraints: crate::placement_constraints::KiCadPlacementConstraints =
        serde_json::from_str(
            r#"{"version": 1, "row": [{"parts": ["SW1", "SW2"], "pitch_mm": 19.05}, {"parts": ["SW3", "SW4"], "pitch_mm": 19.05}]}"#,
        )
        .unwrap();
    let config = KiCadBoardPlacerConfig {
        constraints: Some(KiCadConstraintsSource::Inline(constraints)),
        ..KiCadBoardPlacerConfig::default()
    };
    let lowered = lower_placement(&pcb, &config).unwrap();
    let problem = &lowered.problem;
    // The row leaders carry each member's fabrication square and slot.
    assert_eq!(problem.components[0].courtless.len(), 2);
    assert_eq!(problem.components[0].slots.len(), 2);
    assert_eq!(
        problem.components[0].courtless[0],
        [-8.05, -8.05, 8.05, 8.05]
    );
    // The keys' own slots are no fixed holes of the board: they move.
    assert!(
        problem
            .components
            .iter()
            .filter(|component| component.copper_only)
            .all(|cutout| cutout.side == core::Side::Neither)
    );
    let illegal = core::legal::illegal_components(problem, &problem.poses);
    assert!(illegal.contains(&0) && illegal.contains(&2), "{illegal:?}");
    // (Each row's slot lies in the other's square as well.)
    assert!(matches!(
        core::legal::conflict_kind(problem, &problem.poses, 2, problem.poses[2], 0),
        Some("bodies overlap (no courtyard)" | "a slot of one meets the other")
    ));
    // A row below the other by a key's height is legal.
    let mut poses = problem.poses.clone();
    poses[2].position[1] = 20.0 + 16.1;
    problem.sync_followers(&mut poses);
    assert!(core::legal::illegal_components(problem, &poses).is_empty());
    // A placed board of held keys keeps its slots as cutouts, as before.
    let held = KiCadBoardPlacerConfig {
        fixed: ["SW1", "SW2", "SW3", "SW4"]
            .map(String::from)
            .into_iter()
            .collect(),
        ..KiCadBoardPlacerConfig::default()
    };
    let lowered = lower_placement(&pcb, &held).unwrap();
    assert!(
        lowered.problem.components[..4]
            .iter()
            .all(|c| c.slots.is_empty())
    );
    assert_eq!(
        lowered
            .problem
            .components
            .iter()
            .filter(|c| c.copper_only && c.side == core::Side::Both)
            .count(),
        4
    );
}

/// Net weights (`net_weight`; docs/placer.md, "Net weights").
mod net_weights {
    use super::*;

    #[test]
    fn the_weight_falls_with_the_pin_count() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        for pins in [0, 1, 2, 3, 4] {
            assert_eq!(net_weight(pins), 1.0, "{pins}");
        }
        assert!(close(net_weight(5), 0.75));
        assert!(close(net_weight(20), 3.0 / 19.0));
        // The removed weightings' key is no setting any more.
        let parse = |text: &str| serde_json::from_str::<KiCadBoardPlacerConfig>(text);
        assert!(parse(r#"{"net_weight": "steep"}"#).is_err());
    }

    /// Fourteen resistors: pad 1 of R1 to R12 on BUS (12 pins), pad 2 of
    /// R1 to R6 on +3V3 and of R7 to R12 on S (6 pins each), R13 and R14
    /// on a feedback net (kept short) and A.
    fn board() -> String {
        let nets = ["BUS", "+3V3", "S", "Net-(U1-FB)", "A"];
        let mut footprints = String::new();
        for index in 1..=14usize {
            let (first, second) = match index {
                1..=6 => (1, 2),
                7..=12 => (1, 3),
                _ => (4, 5),
            };
            let pad = |number: usize, x: f64, net: usize| {
                format!(
                    r#"(pad "{number}" smd rect (at {x} 0) (size 0.6 0.9) (layers "F.Cu") (net {net} "{}"))"#,
                    nets[net - 1]
                )
            };
            footprints += &format!(
                r#"(footprint "Lib:R" (layer "F.Cu") (uuid "r{index}") (at {} {})
                    (property "Reference" "R{index}") (attr smd)
                    (fp_rect (start -1.5 -0.8) (end 1.5 0.8) (layer "F.CrtYd") (uuid "r{index}-c"))
                    {} {})"#,
                5.0 + 6.0 * ((index - 1) % 7) as f64,
                8.0 + 16.0 * ((index - 1) / 7) as f64,
                pad(1, -0.8, first),
                pad(2, 0.8, second)
            );
        }
        format!(
            r#"(kicad_pcb (version 20240108)
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (net 0 "") {}
            (gr_rect (start 0 0) (end 50 32) (layer "Edge.Cuts"))
            {footprints})"#,
            nets.iter()
                .enumerate()
                .map(|(index, name)| format!(r#"(net {} "{name}")"#, index + 1))
                .collect::<String>()
        )
    }

    #[test]
    fn the_lowering_weights_each_net_once() {
        let pcb = parse(&board()).unwrap();
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
        for (name, weight) in [
            ("BUS", 3.0 / 11.0),
            ("+3V3", 0.6),
            ("S", 0.6),
            ("Net-(U1-FB)", 3.0),
            ("A", 1.0),
        ] {
            let net = lowered
                .net_names
                .iter()
                .position(|net| net == name)
                .unwrap_or_else(|| panic!("{name}: {:?}", lowered.net_names));
            let got = lowered.problem.net_weights[net];
            assert!(
                (got - weight).abs() < 1e-12,
                "{name}: {got} against {weight}"
            );
        }
    }
}

/// A 6 x 6 BGA at 0.8 mm (U1, locked, its courtyard 5.6 mm square) whose
/// balls join a connector row (J1, locked), A1 on GND and C3 on VDD, and
/// `parts` (KiCad footprints) besides.
fn array_room_board(parts: &str) -> Expr {
    array_room_board_with(parts, true)
}

/// `array_room_board`, the middle 2 x 2 balls (ring 2) on nets of their
/// own (no way out needed) unless `inner_nets`.
fn array_room_board_with(parts: &str, inner_nets: bool) -> Expr {
    let (mut balls, mut ends) = (String::new(), String::new());
    for row in 0..6 {
        for column in 0..6 {
            let number = format!("{}{}", (b'A' + row as u8) as char, column + 1);
            let inner = (2..4).contains(&row) && (2..4).contains(&column);
            let net = match number.as_str() {
                "A1" => "GND".to_string(),
                _ if inner && !inner_nets => format!("unconnected-(U1-{number})"),
                "C3" => "VDD".to_string(),
                other => format!("N{other}"),
            };
            balls.push_str(&format!(
                r#"(pad "{number}" smd circle (at {} {}) (size 0.35 0.35) (layers "F.Cu" "F.Mask") (net "{net}"))"#,
                (column as f64 - 2.5) * 0.8,
                (row as f64 - 2.5) * 0.8
            ));
            let end = if inner && !inner_nets {
                format!("E{number}")
            } else {
                net.clone()
            };
            ends.push_str(&format!(
                r#"(pad "{number}" smd rect (at {} 0) (size 0.4 0.4) (layers "F.Cu") (net "{end}"))"#,
                (row * 6 + column) as f64 * 0.5
            ));
        }
    }
    parse(&format!(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
             (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
             (footprint "Package_BGA:BGA-36" (layer "F.Cu") (locked) (at 10 10)
                 (property "Reference" "U1")
                 (fp_rect (start -2.8 -2.8) (end 2.8 2.8) (layer "F.CrtYd"))
                 {balls})
             (footprint "J" (layer "F.Cu") (locked) (at 30 27) (property "Reference" "J1")
                 (fp_rect (start -0.5 -0.5) (end 18 0.5) (layer "F.CrtYd"))
                 {ends})
             {parts})"#
    ))
    .unwrap()
}

/// A two-pad 0402 on `layer` between nets `a` and `b`.
fn two_pad(reference: &str, layer: &str, at: [f64; 2], a: &str, b: &str, locked: bool) -> String {
    let side = &layer[..1];
    format!(
        r#"(footprint "R_0402" (layer "{layer}") {} (at {} {}) (property "Reference" "{reference}")
             (fp_rect (start -0.9 -0.45) (end 0.9 0.45) (layer "{side}.CrtYd"))
             (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "{layer}") (net "{a}"))
             (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "{layer}") (net "{b}")))"#,
        if locked { "(locked)" } else { "" },
        at[0],
        at[1]
    )
}

/// `array_room_board`'s rules: tracks and clearances 0.15, the class via
/// 0.6/0.3, the project's 0.4/0.2 (the fan-out's in a diagonal gap).
fn array_room_rules() -> KiCadBoardRouterConfig {
    KiCadBoardRouterConfig {
        default_rules: Some(KiCadConnectionRoutingRules {
            trace_width_mm: 0.15,
            clearance_mm: 0.15,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }),
        hole_clearance_mm: 0.25,
        hole_to_hole_clearance_mm: 0.25,
        bga_fanout: Some(true),
        project_rules: Some(crate::project_rules::KiCadProjectRulesReport {
            via_sizes: vec![[0.4, 0.2]],
            via_minimums: [0.4, 0.2, 0.05],
            ..Default::default()
        }),
        ..KiCadBoardRouterConfig::default()
    }
}

#[test]
fn the_room_around_a_grid_array_keeps_its_bands_and_lets_only_its_decoupling_under_it() {
    // C1 decouples VDD (ball C3) to GND, R2 is another back part, R3 a
    // front part, FID1 a locked front part 4.2 mm off U1's centre on both axes.
    let parts = [
        two_pad("C1", "B.Cu", [40.0, 15.0], "VDD", "GND", false),
        two_pad("R2", "B.Cu", [45.0, 15.0], "NB2", "GND", false),
        two_pad("R3", "F.Cu", [50.0, 15.0], "NB3", "X", false),
        two_pad("FID1", "F.Cu", [14.2, 14.2], "NB4", "Y", true),
        two_pad("R4", "F.Cu", [55.0, 15.0], "X", "Y", false),
    ]
    .concat();
    let pcb = array_room_board(&parts);
    // No routing halos: the bodies' own gaps (the 0.5 mm spacing).
    let config = |on: bool| KiCadBoardPlacerConfig {
        bga_room: on,
        tuning: KiCadPlacerTuning {
            halo_scale: 0.0,
            ..far_side_config(Some(array_room_rules())).tuning.clone()
        },
        ..far_side_config(Some(array_room_rules()))
    };
    let off = lower_placement(&pcb, &config(false)).unwrap();
    let on = lower_placement(&pcb, &config(true)).unwrap();
    let index = |name: &str| on.references.iter().position(|r| r == name).unwrap();
    let (u1, c1, r2, r3) = (index("U1"), index("C1"), index("R2"), index("R3"));
    let (before, after) = (&off.problem.components[u1], &on.problem.components[u1]);
    // The ball field is the centres' box grown by half a pitch (2.4 mm
    // each way); the own side's band 2 x (0.15 + 0.15) + 0.6 + 0.3 = 1.5
    // mm beyond it: the body grows from 2.8 to 3.9 mm each way, the edge
    // inset by 1.1 mm.
    assert!(
        before
            .body_size
            .iter()
            .all(|side| (side - 5.6).abs() < 1.0e-9),
        "{:?}",
        before.body_size
    );
    assert!(
        after
            .body_size
            .iter()
            .all(|side| (side - 7.8).abs() < 1.0e-9),
        "{:?}",
        after.body_size
    );
    assert!(after.body_center.iter().all(|v| v.abs() < 1.0e-9));
    assert!((after.edge_inset - before.edge_inset - 1.1).abs() < 1.0e-9);
    // The far side: the fan-out's four via sites, then the room under the
    // array, the field and one ring's track and clearance (rings 0 and 1
    // leave on the surface, ring 2 is the deepest): 2.7 mm each way, which
    // lets C1 in.
    assert_eq!(before.far_side.len(), 4, "{:?}", before.far_side);
    assert_eq!(after.far_side[..4], before.far_side[..]);
    let room = after.far_side[4];
    assert!(
        room.iter()
            .zip([-2.7, -2.7, 2.7, 2.7])
            .all(|(a, b)| (a - b).abs() < 1.0e-9),
        "{room:?}"
    );
    assert_eq!(
        after.far_open,
        core::FarOpen {
            from: 4,
            parts: vec![c1]
        }
    );
    assert_eq!(before.far_open, core::FarOpen::default());
    assert_eq!(on.array_rooms.len(), 1, "{:?}", on.array_rooms);
    assert!(on.array_rooms[0].contains("only its decoupling C1"));
    assert!(
        on.array_rooms[0].contains("fixed parts left in its room: FID1"),
        "{:?}",
        on.array_rooms
    );
    assert!(off.array_rooms.is_empty());
    // Legality at the poses: under the array clear of the vias, C1 is
    // legal and R2 is not; over a via neither; R3 4.6 mm right of U1's
    // centre on the front is in the band only with the room.
    let legal = |lowered: &LoweredPlacement, part: usize, at: [f64; 2]| {
        let problem = &lowered.problem;
        let mut poses = problem.poses.clone();
        poses[part].position = at;
        let fid = index("FID1");
        core::legal::is_legal(
            problem,
            &poses,
            part,
            poses[part],
            (0..poses.len()).filter(|other| *other != part && *other != fid),
        )
    };
    let under = [10.0, 7.85];
    let over = [10.8, 9.2];
    assert!(legal(&off, c1, under) && legal(&off, r2, under));
    assert!(legal(&on, c1, under));
    assert!(!legal(&on, r2, under));
    assert!(!legal(&on, c1, over) && !legal(&off, c1, over));
    assert!(legal(&off, r3, [14.6, 10.0]));
    assert!(!legal(&on, r3, [14.6, 10.0]));
    // An array whose every ball of a net leaves on the surface (rings 0
    // and 1 here) keeps no room.
    let surface = array_room_board_with(&parts, false);
    let (off, on) = (
        lower_placement(&surface, &config(false)).unwrap(),
        lower_placement(&surface, &config(true)).unwrap(),
    );
    assert_eq!(format!("{:?}", off.problem), format!("{:?}", on.problem));
    assert!(on.array_rooms.is_empty());
    // Without a grid array the switch changes nothing.
    let plain = parse(&format!(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
             (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
             {parts})"#
    ))
    .unwrap();
    let (off, on) = (
        lower_placement(&plain, &config(false)).unwrap(),
        lower_placement(&plain, &config(true)).unwrap(),
    );
    assert_eq!(format!("{:?}", off.problem), format!("{:?}", on.problem));
    assert!(on.array_rooms.is_empty());
}

#[test]
fn an_array_whose_inner_balls_leave_by_vias_in_their_pads_keeps_no_room() {
    // `far_side_board`'s U1 (5 x 5 at 0.5 mm, MIDAS's case): a grid array
    // with inner balls on nets, but no via fits a diagonal gap, so its
    // enclosed balls take vias in their pads and the fan-out plans none.
    let pcb = far_side_board("");
    let rules = far_side_rules(true);
    let rooms = crate::board_router::array_rooms(&pcb, &rules);
    assert_eq!(rooms.len(), 1, "{rooms:?}");
    assert!(rooms[0].deepest >= rooms[0].surface_rings);
    assert!(crate::board_router::far_side_balls(&pcb, &rules).is_empty());
    let config = |on: bool| KiCadBoardPlacerConfig {
        bga_room: on,
        ..far_side_config(Some(rules.clone()))
    };
    let (off, on) = (
        lower_placement(&pcb, &config(false)).unwrap(),
        lower_placement(&pcb, &config(true)).unwrap(),
    );
    // The vias in pads keep the far side as before; nothing more.
    assert!(!on.problem.components[0].far_side.is_empty());
    assert_eq!(format!("{:?}", off.problem), format!("{:?}", on.problem));
    assert!(on.array_rooms.is_empty());
    // On by default.
    assert!(KiCadBoardPlacerConfig::default().bga_room);
}

/// A pair's AC-coupling capacitors (`pair_rows`) between a source
/// connector on the left and a destination connector on the right, both
/// capacitors standing turned a quarter (their pads up and down): the row
/// is placed and turned so that its source side faces the source, and
/// `constraints[]` says the pitch and that it faces the pair's ends.
#[test]
fn a_pair_row_faces_its_pairs_two_ends() {
    let board = r#"(kicad_pcb (version 20240108)
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (net 0 "") (net 1 "TX_P") (net 2 "TX_N") (net 3 "HDMI_P") (net 4 "HDMI_N")
        (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
        (footprint "Lib:J" (layer "F.Cu") (at 3 10) (locked yes)
            (property "Reference" "J1")
            (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at 0 -0.5) (size 1 0.4) (layers "F.Cu") (net 1 "TX_P"))
            (pad "2" smd rect (at 0 0.5) (size 1 0.4) (layers "F.Cu") (net 2 "TX_N")))
        (footprint "Lib:J" (layer "F.Cu") (at 37 10) (locked yes)
            (property "Reference" "J2")
            (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd"))
            (pad "1" smd rect (at 0 -0.5) (size 1 0.4) (layers "F.Cu") (net 3 "HDMI_P"))
            (pad "2" smd rect (at 0 0.5) (size 1 0.4) (layers "F.Cu") (net 4 "HDMI_N")))
        (footprint "Capacitor_SMD:C_0402_1005Metric" (layer "F.Cu") (at 20 5 90)
            (property "Reference" "C1")
            (fp_rect (start -0.91 -0.46) (end 0.91 0.46) (layer "F.CrtYd"))
            (pad "1" smd roundrect (at -0.48 0 90) (size 0.56 0.62) (layers "F.Cu") (net 1 "TX_P"))
            (pad "2" smd roundrect (at 0.48 0 90) (size 0.56 0.62) (layers "F.Cu") (net 3 "HDMI_P")))
        (footprint "Capacitor_SMD:C_0402_1005Metric" (layer "F.Cu") (at 25 15 90)
            (property "Reference" "C2")
            (fp_rect (start -0.91 -0.46) (end 0.91 0.46) (layer "F.CrtYd"))
            (pad "1" smd roundrect (at -0.48 0 90) (size 0.56 0.62) (layers "F.Cu") (net 2 "TX_N"))
            (pad "2" smd roundrect (at 0.48 0 90) (size 0.56 0.62) (layers "F.Cu") (net 4 "HDMI_N"))))"#;
    // The row's leader faces with the TX nets on one side, the HDMI nets
    // on the other.
    let lowered = lower_placement(
        &parse(board).unwrap(),
        &KiCadBoardPlacerConfig {
            pair_rows: Some(true),
            ..KiCadBoardPlacerConfig::default()
        },
    )
    .unwrap();
    let facing = &lowered.problem.constraints.facing;
    assert_eq!(facing.len(), 1);
    let c1 = lowered.references.iter().position(|r| r == "C1").unwrap();
    assert_eq!(facing[0].part, c1);
    let names = |nets: &[usize]| -> Vec<&str> {
        nets.iter()
            .map(|net| lowered.net_names[*net].as_str())
            .collect()
    };
    assert_eq!(names(&facing[0].nets[0]), ["TX_P", "TX_N"]);
    assert_eq!(names(&facing[0].nets[1]), ["HDMI_P", "HDMI_N"]);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("pcb-maker-pair-row-{}-{nonce}", std::process::id()));
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("board.kicad_pcb"), board).unwrap();
    let config = KiCadBoardPlacerConfig {
        pair_rows: Some(true),
        tuning: KiCadPlacerTuning {
            placement_seeds: 1,
            ..Default::default()
        },
        ..KiCadBoardPlacerConfig::default()
    };
    let output = root.join("placed");
    let result = place_kicad_board(&source, "board", &output, &config).unwrap();
    assert!(result.unplaced.is_empty() && result.illegal.is_empty());
    let row = result
        .constraints
        .iter()
        .find(|status| status.kind == "pair_row")
        .expect("a pair row");
    assert_eq!(
        (row.part.as_str(), row.other.as_deref()),
        ("C1", Some("C2"))
    );
    let detail = row.detail.as_deref().unwrap_or_default();
    assert!(
        detail.starts_with("pitch 0.93 mm") && detail.contains("faces the pair's two ends"),
        "{detail}"
    );
    // Each capacitor's TX pad lies left of its HDMI pad, the two side by
    // side across the row.
    let placed = parse(&fs::read_to_string(output.join("board.kicad_pcb")).unwrap()).unwrap();
    let pad_x = |reference: &str, number: &str| {
        let footprint = placed
            .children()
            .iter()
            .find(|item| crate::kicad_nets::footprint_reference(item).as_deref() == Some(reference))
            .unwrap();
        let at = form_at(footprint).unwrap();
        let pad = footprint
            .children()
            .iter()
            .find(|item| {
                item.head() == Some("pad")
                    && item.children().get(1).and_then(Expr::atom) == Some(number)
            })
            .unwrap();
        let local = form_at(pad).unwrap();
        let (sin, cos) = (-at[2]).to_radians().sin_cos();
        [
            at[0] + local[0] * cos - local[1] * sin,
            at[1] + local[0] * sin + local[1] * cos,
        ]
    };
    let _ = fs::remove_dir_all(&root);
    for reference in ["C1", "C2"] {
        let (tx, hdmi) = (pad_x(reference, "1"), pad_x(reference, "2"));
        assert!(
            tx[0] < hdmi[0] - 0.5,
            "{reference}: TX at {tx:?}, HDMI at {hdmi:?}"
        );
    }
    let (c1, c2) = (pad_x("C1", "1"), pad_x("C2", "1"));
    assert!((c1[0] - c2[0]).abs() < 0.01 && ((c1[1] - c2[1]).abs() - 0.93).abs() < 0.01);
}

/// A fine-pitch part U1 (three 0.3 x 1.0 mm pads at 0.5 mm pitch over an
/// exposed pad) at `angle`, J9's pad 0.15 mm above its middle pad's tip,
/// and each net's other pad far off.
fn walled_board(angle: f64) -> String {
    let far = |reference: &str, x: f64, y: f64, net: usize, name: &str| {
        format!(
            r#"(footprint "Lib:TP" (layer "F.Cu") (at {x} {y})
            (property "Reference" "{reference}")
            (pad "1" smd rect (at 0 0) (size 0.8 0.8) (layers "F.Cu") (net {net} "{name}")))"#
        )
    };
    format!(
        r#"(kicad_pcb (version 20240108)
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (net 0 "") (net 1 "A") (net 2 "B") (net 3 "C") (net 4 "G") (net 5 "X")
        (gr_rect (start 0 0) (end 40 40) (layer "Edge.Cuts"))
        (footprint "Lib:QFN" (layer "F.Cu") (at 20 20 {angle})
            (property "Reference" "U1")
            (pad "1" smd rect (at -0.5 0 {angle}) (size 0.3 1.0) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 0 0 {angle}) (size 0.3 1.0) (layers "F.Cu") (net 2 "B"))
            (pad "3" smd rect (at 0.5 0 {angle}) (size 0.3 1.0) (layers "F.Cu") (net 3 "C"))
            (pad "4" smd rect (at 0 1.0 {angle}) (size 1.6 0.5) (layers "F.Cu") (net 4 "G")))
        (footprint "Lib:R" (layer "F.Cu") (at 20 19.2)
            (property "Reference" "J9")
            (pad "1" smd rect (at 0 0) (size 0.8 0.3) (layers "F.Cu") (net 5 "X")))
        {} {} {} {} {})"#,
        far("R1", 3.0, 3.0, 1, "A"),
        far("R2", 37.0, 3.0, 2, "B"),
        far("R3", 3.0, 37.0, 3, "C"),
        far("R4", 37.0, 37.0, 4, "G"),
        far("R5", 20.0, 37.0, 5, "X"),
    )
}

#[test]
fn a_pad_another_parts_pad_walls_in_has_no_way_out_until_its_part_turns() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("pcb-maker-exits-{}-{nonce}", std::process::id()));
    let mut closed_at = Vec::new();
    for angle in [0.0, 90.0] {
        let source = root.join(format!("{angle}"));
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("board.kicad_pcb"), walled_board(angle)).unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(crate::KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let census = pad_exits_kicad_board(&source, "board", &config, None).unwrap();
        let middle: Vec<&KiCadClosedPad> = census
            .closed
            .iter()
            .filter(|pad| pad.pad == "U1.2")
            .collect();
        if let Some(pad) = middle.first() {
            // Another part's pad closes it: the placement's doing, with
            // the reason the place check and the verdict give.
            assert_eq!(pad.blocked_by[0], "J9.1", "{pad:?}");
            assert!(!pad.footprint_only);
            assert!(
                pad.reason
                    .starts_with("pad U1.2 has no way out (blocked by J9.1")
            );
        }
        assert!(census.work_seconds > 0.0);
        closed_at.push(!middle.is_empty());
    }
    let _ = fs::remove_dir_all(&root);
    assert_eq!(
        closed_at,
        [true, false],
        "closed at 0 degrees, open after the turn"
    );
}

/// The diagnostic start (`PCB_PLACER_START`) reads the designer's poses
/// by reference and the parts file by `REF#n`; the trace
/// (`PCB_PLACER_TRACE`) says each seed's number, whether it was kept, the
/// objective at every stage and every part's pose at every stage.
#[test]
fn the_start_reads_the_designers_poses_and_the_trace_is_well_formed() {
    let part = |reference: &str, x: f64, nets: (usize, usize)| {
        format!(
            r#"(footprint "R_0603" (layer "F.Cu") (at {x} 10)
            (property "Reference" "{reference}" (at 0 -1) (layer "F.SilkS"))
            (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "F.Cu") (net {} "N{}"))
            (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "F.Cu") (net {} "N{}")))"#,
            nets.0, nets.0, nets.1, nets.1
        )
    };
    let board = |xs: [f64; 3]| {
        format!(
            r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
        {} {} {})"#,
            part("R1", xs[0], (1, 2)),
            part("R2", xs[1], (2, 3)),
            part("R2", xs[2], (3, 1))
        )
    };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("pcb-maker-start-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let designer = root.join("designer.kicad_pcb");
    fs::write(&designer, board([8.0, 20.0, 32.0])).unwrap();
    let pcb = parse(&board([20.0, 20.0, 20.0])).unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let start = start_from(&lowered, &designer, Some("legalize"), None).unwrap();
    assert_eq!(start.stage, core::StartStage::Legalize);
    let xs: Vec<f64> = start.poses.iter().map(|pose| pose.position[0]).collect();
    assert_eq!(
        xs,
        vec![8.0, 20.0, 32.0],
        "by reference, repeated ones in order"
    );
    let names = root.join("parts.json");
    fs::write(&names, r#"["R2#1"]"#).unwrap();
    let only = start_from(&lowered, &designer, None, Some(&names)).unwrap();
    assert_eq!(
        (only.stage, only.parts),
        (core::StartStage::Global, vec![false, false, true])
    );
    fs::write(&names, r#"["R3#0"]"#).unwrap();
    assert!(start_from(&lowered, &designer, None, Some(&names)).is_err());
    assert!(start_from(&lowered, &designer, Some("anneal"), Some(&names)).is_err());
    assert!(start_from(&lowered, &designer, Some("somewhere"), None).is_err());

    let mut config = core::Config::new();
    config.start = Some(std::sync::Arc::new(start));
    let placement = core::place(&lowered.problem, &config);
    let trace = root.join("trace.json");
    trace_placements(&lowered, &[placement], &[7], &config, &trace).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&trace).unwrap()).unwrap();
    let seed = &json["seeds"][0];
    assert_eq!(
        (seed["seed"].as_u64(), seed["kept"].as_bool()),
        (Some(7), Some(true))
    );
    let stages: Vec<&str> = seed["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stage| stage.as_str().unwrap())
        .collect();
    assert_eq!(
        stages,
        vec!["start", "global", "annealed", "legal", "final"]
    );
    // The descent's history: none ran, so none is kept.
    assert!(
        seed["global_history"]["global"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for stage in &stages {
        for objective in ["objective", "objective_bare"] {
            let terms = &seed[objective][stage];
            assert_eq!(terms["wirelength"].as_array().unwrap().len(), 3, "{stage}");
            assert!(terms["overlap"].as_f64().is_some(), "{stage}");
        }
    }
    for stage in &stages {
        for at in ["global_overflow_at", "global_overflow_at_global"] {
            let overflow = seed[at][stage].as_f64().unwrap();
            assert!((0.0..=1.0).contains(&overflow), "{stage}: {overflow}");
        }
    }
    let parts = seed["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 3);
    for part in parts {
        let poses = part["poses"].as_array().unwrap();
        assert_eq!(poses.len(), stages.len());
        assert_eq!(poses[0].as_array().unwrap().len(), 3);
    }
    // The start's poses are the global stage's (none ran).
    assert_eq!(parts[0]["poses"][0][0].as_f64(), Some(8.0));
    assert_eq!(parts[0]["poses"][1][0].as_f64(), Some(8.0));
    // The level walk: the levels that ran, the last with no part left
    // without room; no race over skeletons ran.
    let walk = seed["walk"].as_array().unwrap();
    let last = walk.last().expect("a level ran");
    assert!(last[1].as_array().unwrap().is_empty(), "{walk:?}");
    assert!(last[2].as_f64().unwrap() >= 0.0);
    assert!(seed["skeleton"].is_null());
    let _ = fs::remove_dir_all(&root);
}

/// Every placer setting that moved into `tuning`, refused at the top level
/// with its new place, read there, written back as read; in a layout's
/// `placer` too.
#[test]
fn the_placer_settings_tuning_group_holds_every_moved_key() {
    use pcb_kicad_route::settings::{FieldNames, check_tuning_group, field_names};
    let mut top: &'static [&'static str] = &[];
    let _ = KiCadBoardPlacerConfig::deserialize(FieldNames(&mut top));
    assert!(top.contains(&"fixed") && top.contains(&"tuning"), "{top:?}");
    let tuning = field_names::<KiCadPlacerTuning>();
    let checked = check_tuning_group::<KiCadBoardPlacerConfig>("placer", top, tuning, &[]).unwrap();
    assert_eq!((checked.len(), tuning.len()), (23, 23));
}

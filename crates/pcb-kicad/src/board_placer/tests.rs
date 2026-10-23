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
            placement_seeds: 1,
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
        placement_seeds: 1,
        free: vec!["R1".into()],
        ..KiCadBoardPlacerConfig::default()
    };
    let result = place_kicad_board(&source, "board", &root.join("placed"), &config).unwrap();
    let _ = fs::remove_dir_all(&root);
    assert!(result.unplaced.is_empty() && result.illegal.is_empty());
    assert_eq!(result.parts_on_marks, ["R1"]);
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
        keep_copies: false,
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
fn routing_room_leaves_the_poured_nets_out_of_the_demand() {
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
    // Off by default; on, the demand leaves GND out.
    assert!(routing_room(&config, &lowered).is_none());
    let on = KiCadBoardPlacerConfig {
        routing_room: true,
        routing_room_share: 0.75,
        ..config
    };
    let room = routing_room(&on, &lowered).unwrap();
    assert_eq!(room.net_demand[net_of("R1", 1)], 0.0);
    assert_eq!(room.net_demand[net_of("R1", 0)], 1.0);
    assert_eq!(room.share, 0.75);
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
        let seen = |poses: &[core::Pose]| {
            core::anneal::overlaps(problem, poses, &core::anneal::AnnealConfig::default())[c]
        };
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
            mask_openings: false,
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
        solid_pin_rows: false,
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
    assert!(interleaved(&KiCadBoardPlacerConfig {
        solid_pin_rows: false,
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
            placement_seeds: 2,
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

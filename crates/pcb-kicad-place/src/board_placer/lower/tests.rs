// Copyright (C) 2026 Toit contributors.

//! Unit tests of the lowering's stages, each alone.

use super::footprint::{FootprintSides, angle_options, halo, pad_boxes, plated_hole_ring};
use super::obstacles::{
    add_board_rule_areas, add_cutouts, add_footprint_rule_areas, fixed_obstacle, local_box,
    outer_copper,
};
use super::*;
use pcb_placer as core;

fn close(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1.0e-9)
}

#[test]
fn move_all_preserves_configured_fixed_and_releases_explicit_free() {
    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 30 20) (layer "Edge.Cuts"))
        (footprint "U" (layer "F.Cu") (at 4 4 90)
            (property "Reference" "U1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")))
        (footprint "C" (layer "F.Cu") (at 8 8)
            (property "Reference" "C1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")))
        (footprint "U" (layer "F.Cu") (at 12 12) (locked yes)
            (property "Reference" "U2")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")))
        (footprint "U" (layer "F.Cu") (at 16 16) (locked yes)
            (property "Reference" "U3")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))))"#,
    )
    .unwrap();
    let constraints = serde_json::from_str(r#"{"version": 1, "move_all": true}"#).unwrap();
    let config = KiCadBoardPlacerConfig {
        fixed: vec!["U1".into()],
        fixed_patterns: vec!["C?".into()],
        free: vec!["U3".into()],
        constraints: Some(
            crate::placement_constraints::KiCadConstraintsSource::Inline(constraints),
        ),
        ..Default::default()
    };
    let lowered = lower_placement(&pcb, &config).unwrap();
    for (reference, position, angle, fixed) in [
        ("U1", [4.0, 4.0], 90.0, true),
        ("C1", [8.0, 8.0], 0.0, true),
        ("U2", [12.0, 12.0], 0.0, true),
        ("U3", [16.0, 16.0], 0.0, false),
    ] {
        let index = lowered
            .references
            .iter()
            .position(|name| name == reference)
            .unwrap();
        assert_eq!(
            lowered.problem.components[index].fixed, fixed,
            "{reference}"
        );
        assert_eq!(
            lowered.problem.poses[index].position, position,
            "{reference}"
        );
        assert_eq!(lowered.problem.poses[index].angle, angle, "{reference}");
    }
}

#[test]
fn only_a_held_full_outline_can_offer_its_pin_rows_as_individual_contacts() {
    let pads = [-5, 5]
        .into_iter()
        .flat_map(|y| {
            [-4, -2, 0, 2, 4].into_iter().map(move |x| {
                format!(r#"(pad "{x},{y}" smd rect (at {x} {y}) (size 0.8 0.8) (layers "F.Cu"))"#)
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    let check = |outline, extra, lock| {
        let pcb = parse(&format!(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
          {extra}
          (footprint "Carrier" (layer "F.Cu") (at 20 20) {lock}
            (property "Reference" "U1") (path "/carrier") {outline} {pads}))"#
        ))
        .unwrap();
        lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap()
    };
    let outline = r#"(fp_rect (start -5 -5) (end 5 5) (layer "Edge.Cuts"))"#;
    let lowered = check(outline, "", "(locked yes)");
    let [(part, boxes)] = lowered.perimeter_contact_boxes.as_slice() else {
        panic!()
    };
    assert_eq!((*part, boxes.len()), (0, 10));
    assert!(lowered.problem.components[0].hollow.is_empty());
    assert!(check(outline, "", "").perimeter_contact_boxes.is_empty());
    let slot = r#"(fp_circle (center 0 0) (end 0 1) (layer "Edge.Cuts"))"#;
    let board_outline = r#"(gr_rect (start 0 0) (end 40 40) (layer "Edge.Cuts"))"#;
    assert!(
        check(slot, board_outline, "(locked yes)")
            .perimeter_contact_boxes
            .is_empty()
    );
}

#[test]
fn a_declared_hollow_pad_field_admits_through_hole_parts_between_its_pads() {
    let pads = [-5, 5]
        .into_iter()
        .flat_map(|y| {
            [-4, -2, 0, 2, 4].into_iter().map(move |x| {
                format!(r#"(pad "{x},{y}" thru_hole circle (at {x} {y}) (size 1.2 1.2) (drill 0.8) (layers "*.Cu" "*.Mask"))"#)
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    let check = |hollow, physical_shape| {
        let pcb = parse(&format!(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 40 40) (layer "Edge.Cuts"))
            (footprint "PadField" (layer "F.Cu") (at 20 20) (locked yes)
              (property "Reference" "P1") {physical_shape} {pads})
            (footprint "Header" (layer "F.Cu") (at 10 10)
              (property "Reference" "J1")
              (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
              (pad "1" thru_hole circle (at 0 0) (size 1.5 1.5) (drill 1) (layers "*.Cu" "*.Mask")))
            (footprint "MountingHole" (layer "F.Cu") (at 24 20) (locked yes)
              (property "Reference" "H1")
              (pad "" np_thru_hole circle (at 0 0) (size 1 1) (drill 1) (layers "*.Cu" "*.Mask"))))"#
        ))
        .unwrap();
        let constraints = crate::placement_constraints::KiCadPlacementConstraints {
            hollow: if hollow { vec!["P1".into()] } else { vec![] },
            ..Default::default()
        };
        let config = KiCadBoardPlacerConfig {
            constraints: Some(KiCadConstraintsSource::Inline(constraints)),
            free: vec!["J1".into()],
            ..Default::default()
        };
        lower_placement(&pcb, &config).unwrap().problem
    };
    let fit = |problem: &core::Problem, position| {
        core::legal::is_legal(
            problem,
            &problem.poses,
            1,
            core::Pose {
                position,
                angle: 0.0,
            },
            [0, 2].into_iter(),
        )
    };
    let ordinary = check(false, "");
    assert!(!fit(&ordinary, [20.0, 20.0]));
    let grid = check(true, "");
    assert!(grid.components[0].holes_inside);
    assert_eq!(grid.components[0].hollow.len(), 10);
    assert_eq!(ordinary.components[0].far_side, grid.components[0].far_side);
    assert!(fit(&grid, [20.0, 20.0]));
    assert!(!fit(&grid, [20.0, 25.0]));
    assert!(!fit(&grid, [24.0, 20.0]));

    let enclosure = r#"(fp_rect (start -5 -6) (end 5 6) (layer "F.Fab"))"#;
    let enclosed = check(true, enclosure);
    assert!(!enclosed.components[0].holes_inside);
    assert!(!fit(&enclosed, [20.0, 20.0]));
}

#[test]
fn a_net_designers_keep_short_weighs_three_times_its_pins_weight() {
    let mut net_ids = BTreeMap::new();
    net_ids.insert("GND".to_string(), 0);
    net_ids.insert("Net-(U9-FB)".to_string(), 1);
    let part = |nets: &[usize]| core::Component {
        pins: nets
            .iter()
            .map(|&net| core::Pin {
                offset: [0.0, 0.0],
                net,
            })
            .collect(),
        ..fixed_obstacle("part", [1.0, 1.0], core::Side::Front)
    };
    // GND on five pins, the feedback net on two.
    let components = [part(&[0, 1]), part(&[0, 1]), part(&[0, 0, 0])];
    let weights = net_weights(&net_ids, &components);
    assert_eq!(weights, vec![net_weight(5), 3.0 * net_weight(2)]);
}

#[test]
fn a_part_turns_by_quarters_and_square_to_the_board_from_an_odd_angle() {
    let config = KiCadBoardPlacerConfig::default();
    let square = angle_options(&config, 90.0);
    assert_eq!(square.len(), 4);
    let odd = angle_options(&config, -30.0);
    assert_eq!(odd.len(), 8, "{odd:?}");
    for board in [0.0, 90.0, 180.0, 270.0] {
        assert!(odd.contains(&board), "{odd:?}");
    }
    let kept = KiCadBoardPlacerConfig {
        tuning: KiCadPlacerTuning {
            keep_rotation: true,
            ..Default::default()
        },
        ..KiCadBoardPlacerConfig::default()
    };
    assert_eq!(angle_options(&kept, -30.0), vec![-30.0]);
}

#[test]
fn a_face_only_constraint_keeps_each_native_locked_origin() {
    use crate::placement_constraints::{KiCadConstraintsSource, apply_sides};

    let board = r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
        (gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts"))
        (footprint "Bracket" (layer "F.Cu") (at 12 8 70) (locked yes)
            (property "Reference" "REF**")
            (pad "" np_thru_hole circle (at 2 0) (size 1 1) (drill 1) (layers "*.Cu" "*.Mask")))
        (footprint "Resistor" (layer "F.Cu") (at 20 10)
            (property "Reference" "REF**")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "A")))
        (footprint "Connector" (layer "F.Cu") (at 28 8 90) (locked yes)
            (property "Reference" "J1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "A"))))"#;
    let lower = |brief| {
        let mut pcb = parse(board).unwrap();
        let constraints = serde_json::from_str(brief).unwrap();
        apply_sides(&mut pcb, &constraints).unwrap();
        lower_placement(
            &pcb,
            &KiCadBoardPlacerConfig {
                constraints: Some(KiCadConstraintsSource::Inline(constraints)),
                ..Default::default()
            },
        )
        .unwrap()
    };
    for (brief, side, angle) in [
        (
            r#"{"version":1,"move_all":true,"front":["REF**"]}"#,
            core::Side::Front,
            70.0,
        ),
        (
            r#"{"version":1,"move_all":true,"back":["REF**"]}"#,
            core::Side::Back,
            -70.0,
        ),
    ] {
        let model = lower(brief);
        for (i, pose) in model.problem.poses.iter().enumerate() {
            if model.references[i] != "REF**" {
                continue;
            }
            let component = &model.problem.components[i];
            assert_eq!(component.side, side);
            match pose.position {
                [12.0, 8.0] => {
                    assert!(component.fixed);
                    assert_eq!(pose.angle.rem_euclid(360.0), f64::rem_euclid(angle, 360.0));
                }
                [20.0, 10.0] => assert!(!component.fixed),
                other => panic!("unexpected source origin {other:?}"),
            }
        }
    }
    let moved = lower(
        r#"{"version":1,"front":["J1"],
        "place":[{"part":"J1","x":24,"y":9,"angle":0,"origin":"absolute"}]}"#,
    );
    let j1 = moved
        .references
        .iter()
        .position(|reference| reference == "J1")
        .unwrap();
    assert_eq!(moved.problem.poses[j1].position, [24.0, 9.0]);
    assert_eq!(moved.problem.poses[j1].angle, 0.0);
    assert!(moved.problem.components[j1].fixed);
}

#[test]
fn pad_near_uses_mirrored_rotated_pad_centres_in_report_and_legal_repair() {
    use crate::placement_constraints::{KiCadConstraintsSource, apply_sides, constraint_report};

    let board = r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
        (gr_rect (start 0 0) (end 30 20) (layer "Edge.Cuts"))
        (footprint "U" (layer "F.Cu") (at 16 10 90)
            (property "Reference" "U1")
            (pad "3" smd rect (at 1 -1) (size 1 1) (layers "F.Cu")))
        (footprint "C" (layer "F.Cu") (at 9 9 270)
            (property "Reference" "C1")
            (pad "1" smd rect (at 1 2) (size 1 1) (layers "F.Cu"))))"#;
    let constraints = serde_json::from_str(
        r#"{
        "version": 1, "move_all": true, "fixed": ["U1"], "back": ["C1"],
        "rotation": [{"part": "C1", "angle": 90}],
        "near": [{"part": "C1", "pad": "1", "pin_of": "U1:3", "max_mm": 1}]
    }"#,
    )
    .unwrap();
    let mut pcb = parse(board).unwrap();
    apply_sides(&mut pcb, &constraints).unwrap();
    let config = KiCadBoardPlacerConfig {
        constraints: Some(KiCadConstraintsSource::Inline(constraints)),
        ..Default::default()
    };
    let lowered = lower_placement(&pcb, &config).unwrap();
    let mut poses = lowered.problem.poses.clone();
    let reports =
        |poses: &[core::Pose]| constraint_report(&lowered.problem, poses, &lowered.references);
    let report = reports(&poses)
        .into_iter()
        .find(|entry| entry.kind == "near")
        .unwrap();
    // Rotated U1:3 is at (15, 9). Flipping C1 mirrors its pad's Y from +2 to -2
    // and makes its angle +90 degrees, so C1:1 is at (7, 8).
    assert_eq!(report.distance_mm, Some(8.062));
    assert_eq!(report.violation_mm, 7.062);
    assert_eq!(
        core::legal::repair_relations(&lowered.problem, &mut poses),
        1
    );
    assert!(core::legal::illegal_components(&lowered.problem, &poses).is_empty());
    assert_eq!(poses[0], lowered.problem.poses[0]);
    assert_eq!(poses[1].angle, 90.0);
    assert!(
        reports(&poses)
            .into_iter()
            .find(|entry| entry.kind == "near")
            .unwrap()
            .satisfied
    );

    let duplicate = parse(&board.replace(
        r#"(pad "1" smd rect (at 1 2) (size 1 1) (layers "F.Cu"))"#,
        r#"(pad "1" smd rect (at 1 2) (size 1 1) (layers "F.Cu"))
           (pad "1" smd rect (at -1 2) (size 1 1) (layers "F.Cu"))"#,
    ))
    .unwrap();
    assert!(matches!(lower_placement(&duplicate, &config), Err(error)
        if error.contains("C1:1") && error.contains("multiple pad centres")));

    let objective = serde_json::json!({
        "version": 1,
        "move_all": true,
        "near": [{"part": "C1", "pad": "1", "pin_of": "U1:3", "minimize": true}]
    });
    let objective_config = KiCadBoardPlacerConfig {
        constraints: Some(KiCadConstraintsSource::Inline(
            serde_json::from_value(objective).unwrap(),
        )),
        ..Default::default()
    };
    assert!(
        matches!(lower_placement(&duplicate, &objective_config), Err(error)
        if error.contains("C1:1") && error.contains("multiple pad centres"))
    );
    let coincident = parse(&board.replace(
        r#"(pad "1" smd rect (at 1 2) (size 1 1) (layers "F.Cu"))"#,
        r#"(pad "1" smd rect (at 1 2) (size 1 1) (layers "F.Cu"))
           (pad "1" smd rect (at 1 2) (size 1 1) (layers "F.Cu"))"#,
    ))
    .unwrap();
    assert!(lower_placement(&coincident, &objective_config).is_ok());
}

#[test]
fn pad_near_reports_a_strict_limit_with_unrounded_distance_and_pad_ids() {
    use crate::placement_constraints::{
        KiCadConstraintStatus, KiCadConstraintsSource, constraint_report,
    };

    let pcb = parse(
        r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
        (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
        (footprint "U" (layer "F.Cu") (at 10 10)
            (property "Reference" "U1")
            (pad "3" smd rect (at 0 0) (size 1 1) (layers "F.Cu")))
        (footprint "C" (layer "F.Cu") (at 16 10)
            (property "Reference" "C1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))))"#,
    )
    .unwrap();
    let constraints = serde_json::from_str(
        r#"{
            "version": 1, "move_all": true, "fixed": ["U1"],
            "near": [
                {"part": "C1", "pad": "1", "pin_of": "U1:3", "max_mm": 6},
                {"part": "C1", "pin_of": "U1:3", "max_mm": 6},
                {"part": "C1", "pad": "1", "pin_of": "U1:3", "minimize": true}
            ]
        }"#,
    )
    .unwrap();
    let config = KiCadBoardPlacerConfig {
        constraints: Some(KiCadConstraintsSource::Inline(constraints)),
        ..Default::default()
    };
    let mut lowered = lower_placement(&pcb, &config).unwrap();
    let source = lowered
        .references
        .iter()
        .position(|name| name == "C1")
        .unwrap();
    let mut poses = lowered.problem.poses.clone();
    for (distance, strict) in [
        (6.005814, false),
        (5.999814, true),
        (6.0, true),
        (6.0000008, true),
        (6.0000012, false),
    ] {
        poses[source].position[0] = 10.0 + distance;
        let rows = constraint_report(&lowered.problem, &poses, &lowered.references);
        let near = rows
            .iter()
            .find(|status| status.pad_near.is_some())
            .unwrap();
        assert_eq!((near.satisfied, near.violation_mm), (true, 0.0));
        let pair = near.pad_near.as_ref().unwrap();
        assert_eq!((&*pair.pad, &*pair.pin_of), ("1", "U1:3"));
        assert_eq!(pair.max_mm, Some(6.0));
        assert!((pair.distance_mm - distance).abs() < 1.0e-12);
        assert!((pair.overage_mm.unwrap() - (distance - 6.0).max(0.0)).abs() < 1.0e-12);
        assert_eq!(pair.satisfied, Some(strict), "{distance}");
        assert!(pair.refinable);
        let json = serde_json::to_value(near).unwrap();
        if distance == 6.005814 {
            assert_eq!(json["distance_mm"], 6.006);
            assert_eq!(json["pad_near"]["satisfied"], false);
        }
        let body = rows
            .iter()
            .find(|status| status.kind == "near" && status.pad_near.is_none())
            .unwrap();
        assert!(
            serde_json::to_value(body)
                .unwrap()
                .get("pad_near")
                .is_none()
        );
        let objective = rows
            .iter()
            .find(|status| status.kind == "pad_near_objective")
            .unwrap();
        assert_eq!((objective.satisfied, objective.violation_mm), (true, 0.0));
        let evidence = serde_json::to_value(objective.pad_near.as_ref().unwrap()).unwrap();
        assert_eq!(evidence["pad"], "1");
        assert_eq!(evidence["pin_of"], "U1:3");
        assert_eq!(evidence["measured"], true);
        for finite_only in ["max_mm", "overage_mm", "satisfied"] {
            assert!(evidence.get(finite_only).is_none());
        }
    }
    lowered.problem.components[source].fixed = true;
    let rows = constraint_report(&lowered.problem, &poses, &lowered.references);
    assert!(
        !rows
            .iter()
            .find_map(|status| status.pad_near.as_ref())
            .unwrap()
            .refinable
    );

    let legacy: KiCadConstraintStatus = serde_json::from_str(
        r#"{"kind":"near","part":"C1","other":"U1","satisfied":true,"violation_mm":0,"distance_mm":6.006}"#,
    ).unwrap();
    assert!(legacy.pad_near.is_none());
    assert_eq!(legacy.distance_mm, Some(6.006));
}

#[test]
fn a_halo_grows_with_the_pins_up_to_its_maximum_unless_overridden() {
    let mut config = KiCadBoardPlacerConfig {
        tuning: KiCadPlacerTuning {
            pins_per_halo_track: 4.0,
            track_pitch_mm: 0.5,
            maximum_halo_mm: 2.0,
            halo_scale: 1.0,
            ..Default::default()
        },
        ..KiCadBoardPlacerConfig::default()
    };
    assert_eq!(halo(&config, "U1", 5), 1.0);
    assert_eq!(halo(&config, "U1", 100), 2.0);
    config.tuning.halo_overrides_mm.insert("U1".into(), 0.25);
    assert_eq!(halo(&config, "U1", 100), 0.25);
    config.tuning.pins_per_halo_track = 0.0;
    assert_eq!(halo(&config, "U2", 100), 0.0);
}

#[test]
fn a_part_without_smd_pads_whose_only_courtyard_is_on_the_back_sits_there() {
    let puck = parse(
        r#"(footprint "Puck" (layer "F.Cu") (at 0 0)
            (fp_rect (start -1 -1) (end 1 1) (layer "B.CrtYd"))
            (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.5) (layers "*.Cu")))"#,
    )
    .unwrap();
    let sides = FootprintSides::of(&puck);
    assert!(sides.back);
    assert_eq!((sides.own_prefix, sides.other_prefix), ("B.", "F."));
    assert!(sides.own_courtyard && !sides.other_courtyard);
    assert_eq!(sides.other_layers(), ("F.Cu", "F.Mask"));
    // With an SMD pad it stays on its footprint's side.
    let smd = parse(
        r#"(footprint "Pad" (layer "F.Cu") (at 0 0)
            (fp_rect (start -1 -1) (end 1 1) (layer "B.CrtYd"))
            (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")))"#,
    )
    .unwrap();
    let sides = FootprintSides::of(&smd);
    assert!(!sides.back);
    assert!(sides.both_courtyards());
}

#[test]
fn a_non_plated_hole_keeps_the_hole_clearance_and_an_offset_drill_widens_the_pad() {
    let config = KiCadBoardPlacerConfig {
        hole_clearance_mm: Some(0.5),
        copper_clearance_mm: Some(0.2),
        ..KiCadBoardPlacerConfig::default()
    };
    let footprint = parse(r#"(footprint "H" (layer "F.Cu") (at 0 0))"#).unwrap();
    let hole = parse(
        r#"(pad "" np_thru_hole circle (at 1 0) (size 2 2) (drill 2) (layers "*.Cu" "*.Mask"))"#,
    )
    .unwrap();
    let boxes = pad_boxes(
        &config,
        &hole,
        &footprint,
        "np_thru_hole",
        [1.0, 0.0, 0.0],
        [0.0; 3],
    )
    .unwrap();
    // The hole keeps 0.5 from copper, of which the clearance tests add 0.2.
    assert!((boxes.keep_away - 0.3).abs() < 1.0e-9);
    assert!(close(boxes.pad, [-0.3, -1.3, 2.3, 1.3]), "{:?}", boxes.pad);
    assert!(close(boxes.hole, [0.0, -1.0, 2.0, 1.0]), "{:?}", boxes.hole);
    let castellated = parse(
        r#"(pad "1" thru_hole oval (at 0 0) (size 2 1) (drill 0.8 (offset 0.5 0)) (layers "*.Cu"))"#,
    )
    .unwrap();
    let boxes = pad_boxes(
        &config,
        &castellated,
        &footprint,
        "thru_hole",
        [0.0; 3],
        [0.0; 3],
    )
    .unwrap();
    assert_eq!(boxes.keep_away, 0.0);
    assert!(close(boxes.pad, [-0.5, -0.5, 1.5, 0.5]), "{:?}", boxes.pad);
}

#[test]
fn a_plated_hole_adds_only_clearance_that_extends_past_the_copper() {
    let round = parse(r#"(pad "" thru_hole circle (drill 0.6))"#).unwrap();
    let at = [1.0, 2.0, 0.0];
    let extra = 0.20 - 0.09;
    let ring = plated_hole_ring(&round, at, 0.0, [0.7, 1.7, 1.3, 2.3], extra).unwrap();
    assert!(close(ring, [0.59, 1.59, 1.41, 2.41]));
    assert!(plated_hole_ring(&round, at, 0.0, [0.5, 1.5, 1.5, 2.5], extra).is_none());
    // An offset moves the copper while the rotated slot stays around its anchor.
    let slot = parse(r#"(pad "1" thru_hole oval (drill oval 1.6 0.6 (offset 0.3 0)))"#).unwrap();
    let ring =
        plated_hole_ring(&slot, [1.0, 2.0, 135.0], 45.0, [0.5, 1.4, 1.5, 3.0], extra).unwrap();
    assert!(close(ring, [0.59, 1.09, 1.41, 2.91]));
}

#[test]
fn a_board_graphic_covers_the_sides_of_its_copper_and_mask_layers() {
    let graphic = |layers: &str| {
        parse(&format!(
            "(gr_rect (start 0 0) (end 1 1) (layers {layers}))"
        ))
        .unwrap()
    };
    assert_eq!(outer_copper(&graphic(r#""F.Cu" "F.Mask""#)), vec![0]);
    assert_eq!(outer_copper(&graphic(r#""B.Mask" "F.Cu""#)), vec![0, 1]);
    assert_eq!(
        outer_copper(&graphic(r#""In1.Cu" "F.SilkS""#)),
        Vec::<usize>::new()
    );
    let single = parse(r#"(gr_line (start 0 0) (end 1 1) (layer "B.Cu"))"#).unwrap();
    assert_eq!(outer_copper(&single), vec![1]);
}

#[test]
fn a_movable_parts_keepout_turns_into_its_frame() {
    // A 4 x 2 keepout 3 mm right of a part turned a quarter.
    let keepout = fixed_obstacle("keepout", [4.0, 2.0], core::Side::Both);
    let pose = core::Pose {
        position: [13.0, 10.0],
        angle: 0.0,
    };
    let local = local_box(&keepout, pose, [10.0, 10.0, 90.0]);
    assert!(close(local, [-1.0, 1.0, 1.0, 5.0]), "{local:?}");
}

const BOARD: &str = r#"(kicad_pcb
    (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
    (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
    (gr_rect (start 30 5) (end 34 9) (layer "Edge.Cuts"))
    (zone (net 0) (net_name "") (layers "F.Cu" "B.Cu") (name "antenna")
        (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour allowed) (footprints not_allowed))
        (polygon (pts (xy 1 1) (xy 5 1) (xy 5 3) (xy 1 3))))
    (zone (net 0) (net_name "") (layers "F.Cu" "B.Cu") (name "constraint keepout 1")
        (keepout (tracks allowed) (vias allowed) (pads not_allowed) (copperpour allowed) (footprints allowed))
        (polygon (pts (xy 10 1) (xy 12 1) (xy 12 3) (xy 10 3))))
    (zone (net 0) (net_name "") (layers "F.Cu") (name "pads")
        (keepout (tracks allowed) (vias allowed) (pads not_allowed) (copperpour allowed) (footprints allowed))
        (polygon (pts (xy 20 1) (xy 22 1) (xy 22 3) (xy 20 3)))))"#;

#[test]
fn the_boards_rule_areas_are_obstacles_and_those_keeping_bodies_out_fix_parts() {
    let pcb = parse(BOARD).unwrap();
    let config = KiCadBoardPlacerConfig::default();
    let context = LowerContext::new(&pcb, &config).unwrap();
    let mut builder = ProblemBuilder::new(&context);
    let keepouts = add_board_rule_areas(&context, &mut builder).unwrap();
    // All three are obstacles; only the antenna's keeps bodies out (the
    // constraint keepout is not there on purpose, the pad area keeps
    // copper out only).
    assert_eq!(builder.references, vec!["keepout"; 3]);
    assert_eq!(builder.locked, vec![true; 3]);
    assert_eq!(builder.source_at, vec![[0.0; 3]; 3]);
    assert_eq!(keepouts.len(), 1);
    assert_eq!(keepouts[0].minimum, [1.0, 1.0]);
    assert_eq!(keepouts[0].maximum, [5.0, 3.0]);
    let components = &builder.problem.components;
    assert!(!components[0].copper_only && components[0].side == core::Side::Both);
    assert!(components[2].copper_only && components[2].side == core::Side::Front);
    assert_eq!(builder.problem.poses[2].position, [21.0, 2.0]);
    assert!(builder.problem.constraints.pad_rule_areas.is_empty());
}

#[test]
fn the_pad_rule_area_uses_polygon_legality_on_the_whole_active_stack() {
    let area = r#"(layers "F.Cu") (name "pads")"#;
    let both = BOARD.replace(area, r#"(layers "F&B.Cu") (name "pads")"#);
    let inner = both.replace(
        r#"(31 "B.Cu" signal)"#,
        r#"(2 "In1.Cu" signal) (31 "B.Cu" signal)"#,
    );
    for (text, expected) in [(&both, vec![2]), (&inner, Vec::new())] {
        let pcb = parse(text).unwrap();
        let config = KiCadBoardPlacerConfig::default();
        let context = LowerContext::new(&pcb, &config).unwrap();
        let mut builder = ProblemBuilder::new(&context);
        add_board_rule_areas(&context, &mut builder).unwrap();
        assert_eq!(builder.problem.constraints.pad_rule_areas, expected);
    }
}

#[test]
fn a_fixed_footprints_own_rule_area_is_a_locked_part_like_every_obstacle() {
    // A locked module with its antenna keepout: the keepout is the next
    // part, and the per-part lists keep the parts' length (its lock was
    // missing, `locked` one short of `components`).
    let pcb = parse(
        r#"(kicad_pcb
    (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
    (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
    (footprint "ESP32" (layer "F.Cu") (locked) (at 20 10)
        (fp_rect (start -3 -3) (end 3 3) (layer "F.CrtYd"))
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A"))
        (zone (net 0) (net_name "") (layers "F.Cu" "B.Cu") (name "antenna")
            (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour allowed) (footprints not_allowed))
            (polygon (pts (xy 17 4) (xy 23 4) (xy 23 6) (xy 17 6))))))"#,
    )
    .unwrap();
    let config = KiCadBoardPlacerConfig::default();
    let context = LowerContext::new(&pcb, &config).unwrap();
    let mut builder = ProblemBuilder::new(&context);
    for footprint in context.footprints() {
        let lowered =
            super::footprint::lower_footprint(&context, footprint, &mut builder.net_ids).unwrap();
        builder.push_footprint(lowered);
    }
    builder.problem.components[0].fixed = true;
    add_footprint_rule_areas(&context, &mut builder).unwrap();
    assert_eq!(
        builder.references,
        ["", "keepout"].map(String::from).to_vec()
    );
    assert_eq!(builder.locked, vec![true; 2]);
    assert_eq!(builder.problem.components.len(), 2);
    assert_eq!(builder.source_at.len(), 2);
}

#[test]
fn a_cutout_keeps_copper_out_and_keeps_bodies_out_only_under_an_outline_constraint() {
    let pcb = parse(BOARD).unwrap();
    let config = KiCadBoardPlacerConfig::default();
    let context = LowerContext::new(&pcb, &config).unwrap();
    let mut builder = ProblemBuilder::new(&context);
    let cutouts = add_cutouts(&context, &mut builder);
    assert_eq!(builder.references, vec!["cutout"]);
    let cutout = &builder.problem.components[0];
    assert!(cutout.copper_only && cutout.fixed);
    assert_eq!(cutout.side, core::Side::Both);
    assert_eq!(cutout.body_size, [4.0, 4.0]);
    assert_eq!(builder.problem.poses[0].position, [32.0, 7.0]);
    assert!(cutouts.solid.is_empty() && cutouts.owned.is_empty());
    let outlined = KiCadBoardPlacerConfig {
        constraints: Some(KiCadConstraintsSource::Inline(
            serde_json::from_value(serde_json::json!({
                "version": 1,
                "outline": {"width": 40, "height": 20}
            }))
            .unwrap(),
        )),
        ..KiCadBoardPlacerConfig::default()
    };
    let context = LowerContext::new(&pcb, &outlined).unwrap();
    let mut builder = ProblemBuilder::new(&context);
    assert_eq!(add_cutouts(&context, &mut builder).solid, vec![0]);
}

#[test]
fn fabrication_rails_keep_parts_off_unless_the_brief_uses_them() {
    // The narrow strip is joined through a perforated tab beside the
    // main board, not a sub-board. The same outline without holes is
    // ordinary board material.
    let board = |perforated| {
        let holes = if perforated {
            r#"(footprint "MouseBites" (layer "F.Cu") (at 0 10) (locked yes)
                (property "Reference" "MB1")
                (pad "" np_thru_hole circle (at 0 -0.75) (size 0.4 0.4) (drill 0.4) (layers "*.Cu" "*.Mask"))
                (pad "" np_thru_hole circle (at 0 -0.25) (size 0.4 0.4) (drill 0.4) (layers "*.Cu" "*.Mask"))
                (pad "" np_thru_hole circle (at 0 0.25) (size 0.4 0.4) (drill 0.4) (layers "*.Cu" "*.Mask"))
                (pad "" np_thru_hole circle (at 0 0.75) (size 0.4 0.4) (drill 0.4) (layers "*.Cu" "*.Mask")))"#
        } else {
            ""
        };
        parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_poly (pts (xy 0 0) (xy 40 0) (xy 40 20) (xy 0 20) (xy 0 11) (xy -1 11)
                (xy -1 20) (xy -3.4 20) (xy -3.4 0) (xy -1 0) (xy -1 9) (xy 0 9))
                (layer "Edge.Cuts") (width 0.05))
            {holes}
            (footprint "Cap" (layer "F.Cu") (at 20 10)
                (property "Reference" "C1")
                (fp_rect (start -0.5 -0.3) (end 0.5 0.3) (layer "F.CrtYd"))
                (pad "1" smd rect (at -0.3 0) (size 0.3 0.4) (layers "F.Cu" "F.Mask") (net 1 "GND"))
                (pad "2" smd rect (at 0.3 0) (size 0.3 0.4) (layers "F.Cu" "F.Mask") (net 2 "VCC")))
            (footprint "Cap" (layer "F.Cu") (at 30 10)
                (property "Reference" "C2")
                (fp_rect (start -0.5 -0.3) (end 0.5 0.3) (layer "F.CrtYd"))
                (pad "1" smd rect (at -0.3 0) (size 0.3 0.4) (layers "F.Cu" "F.Mask") (net 1 "GND"))
                (pad "2" smd rect (at 0.3 0) (size 0.3 0.4) (layers "F.Cu" "F.Mask") (net 2 "VCC"))))"#
        ))
        .unwrap()
    };
    let lower = |perforated, brief: serde_json::Value| {
        lower_placement(
            &board(perforated),
            &KiCadBoardPlacerConfig {
                constraints: Some(KiCadConstraintsSource::Inline(
                    serde_json::from_value(brief).unwrap(),
                )),
                tuning: KiCadPlacerTuning {
                    halo_scale: 0.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap()
    };
    let legal_at = |lowered: &LoweredPlacement, position| {
        let part = lowered
            .references
            .iter()
            .position(|name| name == "C1")
            .unwrap();
        let problem = &lowered.problem;
        assert!(!problem.components[part].fixed);
        core::legal::is_legal(
            problem,
            &problem.poses,
            part,
            core::Pose {
                position,
                angle: 90.0,
            },
            0..problem.components.len(),
        )
    };
    let unheld = serde_json::json!({"version": 1, "move_all": true});
    assert!(legal_at(&lower(false, unheld.clone()), [-2.2, 5.0]));
    let perforated = lower(true, unheld);
    assert!(!legal_at(&perforated, [-2.2, 5.0]));
    assert!(legal_at(&perforated, [10.0, 5.0]));

    let held = lower(
        true,
        serde_json::json!({
            "version": 1,
            "move_all": true,
            "region": [{"parts": ["C1"], "origin": "absolute", "x": [-3.3, -1.1], "y": [2.0, 7.0]}],
        }),
    );
    assert!(legal_at(&held, [-2.2, 5.0]));

    let anchored = lower(
        true,
        serde_json::json!({
            "version": 1,
            "move_all": true,
            "place": [{"part": "C1", "origin": "absolute", "x": -2.2, "y": 5.0, "angle": 90.0}],
        }),
    );
    assert!(
        !anchored
            .references
            .iter()
            .any(|name| name == "fabrication rail")
    );
}

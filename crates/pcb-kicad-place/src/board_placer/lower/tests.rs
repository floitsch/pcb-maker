// Copyright (C) 2026 Toit contributors.

//! Unit tests of the lowering's stages, each alone.

use super::footprint::{FootprintSides, angle_options, halo, pad_boxes};
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

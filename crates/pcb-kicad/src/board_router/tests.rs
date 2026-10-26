// Copyright (C) 2026 Toit contributors.

//! Unit tests: pour requests, the narrow-signal step's budget and the
//! check gate.

use super::*;
use pcb_router as core;

mod pour_request_tests {
    use super::*;

    #[test]
    fn multilayer_boards_without_pours_get_ground_and_rail_planes() {
        let board = |layers: &str, zone: &str| {
            let pad = |number: usize, net: &str| {
                format!(
                    r#"(pad "{number}" smd rect (at {number} 0) (size 0.5 0.5) (layers "F.Cu") (net "{net}"))"#
                )
            };
            let pads: String = (1..=4)
                .map(|n| pad(n, "GND"))
                .chain((5..=7).map(|n| pad(n, "+3V3")))
                .chain((8..=10).map(|n| pad(n, "/SIG")))
                .collect();
            parse(&format!(
                r#"(kicad_pcb (layers {layers} (25 "Edge.Cuts" user))
                  (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
                  (footprint "a" (at 5 5) {pads}) {zone})"#
            ))
            .unwrap()
        };
        let four = r#"(0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal)"#;
        let config = KiCadBoardRouterConfig::default();
        let planned = planned_pours(&board(four, ""), &config).unwrap();
        let summary: Vec<(String, Vec<String>)> = planned
            .into_iter()
            .map(|pour| (pour.net, pour.layers))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("GND".into(), vec!["In1.Cu".into()]),
                ("+3V3".into(), vec!["In2.Cu".into()])
            ]
        );
        // Two layers, an existing pour, or switched off: none.
        assert!(
            planned_pours(
                &board(r#"(0 "F.Cu" signal) (2 "B.Cu" signal)"#, ""),
                &config
            )
            .unwrap()
            .is_empty()
        );
        let zone =
            r#"(zone (net "GND") (layer "In1.Cu") (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10))))"#;
        assert!(
            planned_pours(&board(four, zone), &config)
                .unwrap()
                .is_empty()
        );
        let off = KiCadBoardRouterConfig {
            automatic_planes: Some(false),
            ..KiCadBoardRouterConfig::default()
        };
        assert!(planned_pours(&board(four, ""), &off).unwrap().is_empty());
    }

    #[test]
    fn exclusive_planes_keep_the_layers_nearest_the_outside_solid() {
        // A six-layer board whose ground pour covers every inner layer
        // (ESC mini): with a limit of two, In1 and In4 stay exclusive.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (8 "In3.Cu" signal) (10 "In4.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "a" (at 5 5) (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 0.5 0.5) (layers "F.Cu") (net "/SIG")))
              (footprint "b" (at 15 5) (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 0.5 0.5) (layers "F.Cu") (net "/SIG")))
              (zone (net "GND") (layers "In1.Cu" "In2.Cu" "In3.Cu" "In4.Cu") (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10) (xy 0 10)))))"#,
        )
        .unwrap();
        let exclusive = |config: &KiCadBoardRouterConfig| -> Vec<usize> {
            let board = lower(&pcb, config, true).unwrap().board;
            let mut layers: Vec<usize> = board
                .planes
                .iter()
                .filter(|plane| plane.exclusive)
                .map(|plane| plane.layer)
                .collect();
            layers.sort_unstable();
            layers
        };
        let rules = KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        };
        let all = KiCadBoardRouterConfig {
            default_rules: Some(rules.clone()),
            ..KiCadBoardRouterConfig::default()
        };
        assert_eq!(exclusive(&all), vec![1, 2, 3, 4]);
        let two = KiCadBoardRouterConfig {
            exclusive_layers: Some(2),
            ..all.clone()
        };
        assert_eq!(exclusive(&two), vec![1, 4]);
        let one = KiCadBoardRouterConfig {
            exclusive_layers: Some(1),
            ..all.clone()
        };
        assert_eq!(exclusive(&one), vec![1]);
    }

    #[test]
    fn a_zone_of_a_higher_priority_is_cut_from_the_nets_lower_zone_and_arcs_are_followed() {
        // Telemetry: GND zones along the round board's edge (priority 10,
        // drawn with arcs) over a whole-board GND zone (priority 1). KiCad
        // fills the lower zone less the higher one's outline; the arc side
        // is followed (it was left out, the outline a polygon through the
        // other corners).
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
              (footprint "a" (at 5 5) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")))
              (footprint "b" (at 15 15) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")))
              (zone (net "GND") (layers "F.Cu") (priority 1) (polygon (pts (xy 0 0) (xy 20 0) (xy 20 20) (xy 0 20))))
              (zone (net "GND") (layers "F.Cu") (priority 10)
                (polygon (pts (xy 20 0) (xy 10 0) (arc (start 10 0) (mid 17.071068 2.928932) (end 20 10))))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, true).unwrap().board;
        assert_eq!(board.planes.len(), 2);
        let (low, high) = (&board.planes[0], &board.planes[1]);
        assert_eq!(low.above.len(), 1);
        assert!(high.above.is_empty());
        // The corner between the arc and the board's corner is the higher
        // zone's; a point between the arc and its chord is not.
        assert!(core::geometry::point_in_polygon([19.0, 1.0], &low.above[0]));
        assert!(!core::geometry::point_in_polygon(
            [16.0, 4.0],
            &low.above[0]
        ));
        assert!(high.polygon.len() > 10);
    }

    #[test]
    fn of_two_zones_of_one_priority_the_larger_uuid_is_filled_first() {
        // USB-C 3-way splitter: +5V and GND both of priority 1 overlap on
        // F.Cu. KiCad orders them by uuid (ZONE::HigherPriority): the
        // larger one's fill stays, the other is cut around it; the pour
        // model filled GND across +5V's zone.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
              (footprint "a" (at 5 5) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "+5V")))
              (footprint "b" (at 15 15) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "+5V")))
              (zone (net "GND") (layers "F.Cu") (uuid "0f000000-0000-0000-0000-000000000000") (priority 1)
                (polygon (pts (xy 0 0) (xy 20 0) (xy 20 20) (xy 0 20))))
              (zone (net "+5V") (layers "F.Cu") (uuid "a0000000-0000-0000-0000-000000000000") (priority 1)
                (polygon (pts (xy 10 0) (xy 20 0) (xy 20 10) (xy 10 10)))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, true).unwrap().board;
        let plane = |name: &str| {
            board
                .planes
                .iter()
                .find(|plane| board.nets[plane.net as usize].name == name)
                .unwrap()
        };
        assert_eq!(plane("GND").excluded.len(), 1);
        assert!(plane("+5V").excluded.is_empty());
    }

    #[test]
    fn rule_areas_that_keep_fills_out_are_left_out_of_the_pours_on_their_layers() {
        // Sisu: rule areas that allow tracks and vias but no zone fill; the
        // pour model filled them where KiCad leaves them empty.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "a" (at 5 5) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.5) (layers "*.Cu") (net "GND")))
              (footprint "b" (at 15 5) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.5) (layers "*.Cu") (net "GND")))
              (zone (net "GND") (layers "In1.Cu" "In2.Cu") (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10) (xy 0 10))))
              (zone (layers "In2.Cu") (name "no fill")
                (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed))
                (polygon (pts (xy 8 2) (xy 12 2) (xy 12 8) (xy 8 8))))
              (zone (layers "In1.Cu") (name "no tracks")
                (keepout (tracks not_allowed) (vias allowed) (pads allowed) (copperpour allowed) (footprints allowed))
                (polygon (pts (xy 8 2) (xy 12 2) (xy 12 8) (xy 8 8)))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, true).unwrap().board;
        let excluded = |layer: usize| {
            board
                .planes
                .iter()
                .find(|plane| plane.layer == layer)
                .unwrap()
                .excluded
                .clone()
        };
        assert!(excluded(1).is_empty());
        assert_eq!(excluded(2).len(), 1);
        assert!(core::geometry::point_in_polygon(
            [10.0, 5.0],
            &excluded(2)[0]
        ));
        // Neither keeps a track out of In2; the other one keeps them out
        // of In1 as before.
        assert!(
            !board
                .obstacles
                .iter()
                .any(|obstacle| obstacle.label == "no fill")
        );
        assert!(
            board
                .obstacles
                .iter()
                .any(|obstacle| obstacle.label == "no tracks" && obstacle.blocks_tracks)
        );
    }

    #[test]
    fn fill_keepouts_off_copper_or_drawn_as_circles_do_not_refuse_the_board() {
        // TelemetryOnboard: rule areas that keep fills out, drawn as circles
        // (an outline of one arc) on F.SilkS and on F.Cu.
        let circle = |layer: &str, x: f64| {
            format!(
                r#"(zone (layer "{layer}") (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed))
                (polygon (pts (arc (start {x} 5) (mid {} 5) (end {x} 5)))))"#,
                x + 2.0
            )
        };
        let pcb = parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (37 "F.SilkS" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "a" (at 2 2) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.5) (layers "*.Cu") (net "GND")))
              (footprint "b" (at 18 8) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.5) (layers "*.Cu") (net "GND")))
              (zone (net "GND") (layer "F.Cu") (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10) (xy 0 10))))
              {} {})"#,
            circle("F.SilkS", 3.0),
            circle("F.Cu", 9.0)
        ))
        .unwrap();
        let rules = KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, true).unwrap().board;
        let excluded = &board
            .planes
            .iter()
            .find(|plane| plane.layer == 0)
            .unwrap()
            .excluded;
        assert_eq!(excluded.len(), 1);
        assert!(core::geometry::point_in_polygon([10.0, 5.0], &excluded[0]));
        assert!(!core::geometry::point_in_polygon([4.0, 5.0], &excluded[0]));
    }

    #[test]
    fn mask_openings_keep_copper_half_the_minimum_mask_width_away() {
        let pcb = parse(
            r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
              (setup (solder_mask_min_width 0.1))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "a" (at 12 5)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/A"))
                (pad "2" smd rect (at 4 0) (size 1 1) (layers "F.Cu") (net "/A")))
              (gr_rect (start 4 4) (end 6 6) (stroke (width 0) (type solid)) (fill yes) (layer "F.Mask")))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let opening = board
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label.starts_with("solder mask opening"))
            .unwrap();
        assert!(
            (opening.clearance - 0.06).abs() < 1.0e-9,
            "{}",
            opening.clearance
        );
    }

    #[test]
    fn mask_openings_keep_the_solder_mask_to_copper_clearance_and_their_expansion() {
        // KiCad's bridge test: copper within the project's solder-mask-to-
        // copper clearance plus the aperture's expansion (a graphic's own
        // margin, else the board's pad_to_mask_clearance) is exposed.
        // USB_Cable_Tester: 0.05 + 0.05 mm, a track 0.094 mm from a
        // battery holder's F.Mask polygon bridged it to the GND fill. The
        // web rule (half the minimum width, 0.075 mm here) stays a floor; a
        // text has no expansion.
        let lowered = |margin: &str, mask_to_copper: Option<f64>, text: bool| {
            let opening = if text {
                r#"(gr_text "X" (at 5 5 0) (layer "F.Mask") (effects (font (size 1 1) (thickness 0.15))) (render_cache "X" 0 (polygon (pts (xy 4.5 4.5) (xy 5.5 4.5) (xy 5.5 5.5) (xy 4.5 5.5)))))"#.to_string()
            } else {
                format!(
                    r#"(gr_rect (start 4 4) (end 6 6) (stroke (width 0) (type solid)) (fill yes) {margin} (layer "F.Mask"))"#
                )
            };
            let pcb = parse(&format!(
                r#"(kicad_pcb
                  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
                  (setup (pad_to_mask_clearance 0.05) (solder_mask_min_width 0.13))
                  (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
                  (footprint "a" (at 12 5)
                    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/A"))
                    (pad "2" smd rect (at 4 0) (size 1 1) (layers "F.Cu") (net "/A")))
                  {opening})"#
            ))
            .unwrap();
            let rules = KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            };
            let config = KiCadBoardRouterConfig {
                default_rules: Some(rules),
                solder_mask_to_copper_mm: mask_to_copper,
                ..KiCadBoardRouterConfig::default()
            };
            let board = lower(&pcb, &config, false).unwrap().board;
            board
                .obstacles
                .iter()
                .find(|obstacle| obstacle.label.starts_with("solder mask opening"))
                .unwrap()
                .clearance
        };
        let close = |value: f64, expected: f64| {
            assert!(
                (value - expected).abs() < 1.0e-9,
                "{value} against {expected}"
            )
        };
        close(lowered("", Some(0.05), false), 0.05 + 0.05);
        close(
            lowered("(solder_mask_margin 0.2)", Some(0.05), false),
            0.05 + 0.2,
        );
        close(lowered("", None, false), 0.075);
        close(lowered("", Some(0.05), true), 0.075);
    }

    #[test]
    fn a_pads_mask_aperture_keeps_the_mask_to_copper_clearance_and_an_untented_via_the_web() {
        // KiCad 10.0.7's bridge test around a pad of another net: covered
        // copper (a track, a tented via) within the solder-mask-to-copper
        // clearance plus the pad's mask expansion bridges; an untented via
        // within the minimum web plus both expansions. OpenESC 4in1's J1:
        // pads with a 0.102 mm margin, the project's 0.005 mm, classes of
        // 0.09 mm; tracks kept 0.105 mm and bridged.
        let lowered = |setup: &str, margin: &str, mask_to_copper: Option<f64>, clearance: f64| {
            let pcb = parse(&format!(
                r#"(kicad_pcb
                  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
                  (setup {setup})
                  (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
                  (footprint "J" (layer "B.Cu") (at 10 5) (property "Reference" "J1")
                    (pad "8" smd rect (at 0 0) (size 0.6 1.55) (layers "B.Cu" "B.Mask" "B.Paste") (net "/M4") {margin})
                    (pad "7" smd rect (at -1 0) (size 0.6 1.55) (layers "B.Cu" "B.Mask" "B.Paste") (net "/M3") {margin})
                    (pad "9" smd rect (at 1 0) (size 0.6 1.55) (layers "B.Cu") (net "/M3") {margin})))"#
            ))
            .unwrap();
            let rules = KiCadConnectionRoutingRules {
                trace_width_mm: 0.09,
                clearance_mm: clearance,
                via_size_mm: 0.45,
                via_drill_mm: 0.2,
            };
            let config = KiCadBoardRouterConfig {
                default_rules: Some(rules),
                solder_mask_to_copper_mm: mask_to_copper,
                ..KiCadBoardRouterConfig::default()
            };
            let board = lower(&pcb, &config, false).unwrap().board;
            let mut apertures: Vec<(String, f64, bool, bool, u32)> = board
                .obstacles
                .iter()
                .filter(|obstacle| obstacle.label.ends_with("mask aperture"))
                .map(|obstacle| {
                    (
                        obstacle.label.clone(),
                        obstacle.clearance,
                        obstacle.blocks_tracks,
                        obstacle.blocks_vias,
                        obstacle.layers,
                    )
                })
                .collect();
            apertures.sort_by(|a, b| a.0.cmp(&b.0).then(b.2.cmp(&a.2)));
            apertures
        };
        let close = |value: f64, expected: f64| {
            assert!(
                (value - expected).abs() < 1.0e-9,
                "{value} against {expected}"
            )
        };
        // J1: margin plus the project's clearance, for tracks and the
        // (tented) vias alike, on the back only; pad 9 has no mask opening.
        let j1 = lowered(
            "(pad_to_mask_clearance 0)",
            "(solder_mask_margin 0.102)",
            Some(0.005),
            0.09,
        );
        assert_eq!(j1.len(), 2, "{j1:?}");
        for (label, clearance, tracks, vias, layers) in &j1 {
            assert!(
                label.starts_with("J1.7") || label.starts_with("J1.8"),
                "{label}"
            );
            close(*clearance, 0.107);
            assert!(*tracks && *vias);
            assert_eq!(*layers, 0b10);
        }
        // Before, the margin alone: tracks at 0.105 mm passed.
        // The board's expansion is `pad_to_mask_clearance`.
        let board = lowered("(pad_to_mask_clearance 0.1)", "", Some(0.05), 0.1);
        assert_eq!(board.len(), 2);
        close(board[0].1, 0.15);
        // Untented vias are openings: the web plus both expansions (the
        // pad's and the board's for the via), apart from the tracks' rule.
        let untented = lowered(
            "(pad_to_mask_clearance 0.05) (solder_mask_min_width 0.1) (tenting none)",
            "",
            Some(0.0),
            0.04,
        );
        assert_eq!(untented.len(), 4, "{untented:?}");
        close(untented[0].1, 0.05);
        assert!(untented[0].2 && !untented[0].3);
        close(untented[1].1, 0.1 + 0.05 + 0.05);
        assert!(!untented[1].2 && untented[1].3);
        // KiCad 10's own form, the back side untented.
        let back = lowered(
            "(pad_to_mask_clearance 0.05) (solder_mask_min_width 0.1) (tenting (front yes) (back no))",
            "",
            Some(0.0),
            0.04,
        );
        assert_eq!(back.len(), 4);
        let front = lowered(
            "(pad_to_mask_clearance 0.05) (solder_mask_min_width 0.1) (tenting (front no) (back yes))",
            "",
            Some(0.0),
            0.04,
        );
        assert_eq!(front.len(), 2);
        // Within the copper clearance, nothing.
        assert!(lowered("(pad_to_mask_clearance 0.05)", "", Some(0.05), 0.2).is_empty());
    }

    #[test]
    fn below_an_error_the_pad_apertures_keep_the_rule_of_before() {
        // OpenESC rates solder_mask_bridge a warning: J1's apertures keep
        // the margin alone (0.102 mm), on both sides at once as before.
        let pcb = parse(
            r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
              (setup (pad_to_mask_clearance 0))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (gr_text "ESC" (at 5 5 0) (layer "B.Mask") (effects (font (face "Tokyo") (size 2.5 2.5) (thickness 0.3)))
                (render_cache "ESC" 0 (polygon (pts (xy 4.5 4.5) (xy 5.5 4.5) (xy 5.5 5.5) (xy 4.5 5.5)))))
              (footprint "J" (layer "B.Cu") (at 10 5) (property "Reference" "J1")
                (pad "8" smd rect (at 0 0) (size 0.6 1.55) (layers "B.Cu" "B.Mask" "B.Paste") (net "/M4") (solder_mask_margin 0.102))
                (pad "7" smd rect (at -1 0) (size 0.6 1.55) (layers "B.Cu" "B.Mask" "B.Paste") (net "/M3") (solder_mask_margin 0.102))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules {
            trace_width_mm: 0.09,
            clearance_mm: 0.09,
            via_size_mm: 0.45,
            via_drill_mm: 0.2,
        };
        let lowered = |severity: Option<&str>| {
            let config = KiCadBoardRouterConfig {
                default_rules: Some(rules.clone()),
                solder_mask_to_copper_mm: Some(0.005),
                solder_mask_bridge_severity: severity.map(str::to_owned),
                ..KiCadBoardRouterConfig::default()
            };
            let board = lower(&pcb, &config, false).unwrap().board;
            let apertures: Vec<f64> = board
                .obstacles
                .iter()
                .filter(|obstacle| obstacle.label.ends_with("mask aperture"))
                .map(|obstacle| obstacle.clearance)
                .collect();
            let text = board
                .obstacles
                .iter()
                .filter(|obstacle| obstacle.label == "solder mask opening (gr_text)")
                .count();
            (apertures, text)
        };
        let (warning, text) = lowered(Some("warning"));
        assert_eq!(warning.len(), 2);
        assert!(
            warning
                .iter()
                .all(|clearance| (clearance - 0.102).abs() < 1.0e-9),
            "{warning:?}"
        );
        assert_eq!(text, 1);
        let (error, text) = lowered(None);
        assert!(
            error
                .iter()
                .all(|clearance| (clearance - 0.107).abs() < 1.0e-9),
            "{error:?}"
        );
        assert_eq!(text, 2);
    }

    #[test]
    fn symbols_outside_ascii_reach_as_far_as_descenders() {
        // eurorack-pmod's "⇒⊕⊥∃⊢≢" on F.Mask at 315 degrees: KiCad's glyphs
        // reached 0.73 mm across the line, where a track bridged.
        let shapes = copper_graphic_shapes(
            &parse(r#"(gr_text "⇒⊕⊥∃⊢≢" (at 0 0 0) (layer "F.Mask") (effects (font (size 1 1) (thickness 0.15))))"#).unwrap(),
        )
        .unwrap();
        assert!(shapes[0].contains([0.0, 0.8]) && shapes[0].contains([0.0, -0.8]));
    }

    #[test]
    fn at_snsps_values_a_via_by_another_nets_pad_bridges_no_mask() {
        // SNSP-CPU-01: tented vias (front yes, back yes), no mask expansion,
        // no minimum web, no solder-mask-to-copper clearance, so KiCad's
        // bridge test needs copper to touch: a via of another net next to
        // a pad (0.8 mm via, a 2 mm THT pad, 0.2 mm clearance) is held by
        // the copper clearance alone, untented or not. Its 13 bridges in
        // route mode are pads of other parts placed under J2's F.Mask
        // polygon (C63, C64, C66) and the GND fill there.
        for tenting in ["(tenting (front yes) (back yes))", "(tenting none)"] {
            let pcb = parse(&format!(
                r#"(kicad_pcb
                  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
                  (setup (pad_to_mask_clearance 0) {tenting})
                  (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
                  (footprint "J" (layer "F.Cu") (at 5 5) (property "Reference" "J2")
                    (pad "1" thru_hole rect (at 0 0) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask") (net "VCC"))
                    (pad "4" thru_hole rect (at 2.54 0) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask") (net "GND"))))"#
            ))
            .unwrap();
            let rules = KiCadConnectionRoutingRules {
                trace_width_mm: 0.25,
                clearance_mm: 0.2,
                via_size_mm: 0.8,
                via_drill_mm: 0.4,
            };
            let config = KiCadBoardRouterConfig {
                default_rules: Some(rules),
                solder_mask_to_copper_mm: Some(0.0),
                ..KiCadBoardRouterConfig::default()
            };
            let board = lower(&pcb, &config, false).unwrap().board;
            assert!(
                !board
                    .obstacles
                    .iter()
                    .any(|obstacle| obstacle.label.ends_with("mask aperture"))
            );
        }
    }

    #[test]
    fn via_tenting_is_read_as_kicad_10_reads_it() {
        let tented =
            |setup: &str| vias_tented(&parse(&format!("(kicad_pcb (setup {setup}))")).unwrap());
        assert_eq!(tented(""), [true, true]);
        assert_eq!(tented("(tenting (front yes) (back yes))"), [true, true]);
        assert_eq!(tented("(tenting (front yes))"), [true, false]);
        assert_eq!(tented("(tenting front back)"), [true, true]);
        assert_eq!(tented("(tenting back)"), [false, true]);
        assert_eq!(tented("(tenting none)"), [false, false]);
        assert_eq!(tented("(pcbplotparams (viasonmask false))"), [true, true]);
        assert_eq!(tented("(pcbplotparams (viasonmask yes))"), [false, false]);
    }

    #[test]
    fn a_cached_text_on_a_mask_layer_also_opens_the_stroke_fonts_box() {
        // OpenESC 4in1-mini's "ESC" in Tokyo on B.Mask: KiCad drew it in a
        // stand-in face 2 mm wider than its cached glyphs.
        let text = |layer: &str| {
            format!(
                r#"(gr_text "ESC" (at 72.27 41.77 0) (layer "{layer}") (effects (font (face "Tokyo") (size 2.5 2.5) (thickness 0.3)) (justify left bottom mirror))
                  (render_cache "ESC" 0 (polygon (pts (xy 70.7 39.3) (xy 71.3 39.3) (xy 71.4 40.9) (xy 70.7 40.9)))))"#
            )
        };
        let mask = mask_graphic_shapes(&parse(&text("B.Mask")).unwrap()).unwrap();
        assert_eq!(mask.len(), 2);
        // The track at (68.17, 39.05) and the via at (69.47, 38.75) that
        // bridged were inside KiCad's glyphs.
        assert!(mask.iter().any(|shape| shape.contains([68.17, 39.05])));
        assert!(
            mask.iter()
                .any(|shape| shape.distance_to_point([69.47, 38.75]) < 0.225)
        );
        assert_eq!(
            copper_graphic_shapes(&parse(&text("B.Mask")).unwrap())
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_bottom_anchored_texts_descenders_hang_below_its_anchor() {
        // eurorack-pmod's "[apf.audio]" on F.Mask: KiCad's glyphs span x
        // 9.98 to 11.68 at (11.43, 57.15, 90), 0.25 mm past the anchor; a
        // track whose edge was at 11.549 bridged.
        let shapes = copper_graphic_shapes(
            &parse(r#"(gr_text "[apf.audio]" (at 11.43 57.15 90) (layer "F.Mask") (effects (font (size 1 0.85) (thickness 0.15)) (justify left bottom)))"#)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(shapes.len(), 1);
        for point in [[11.6765, 53.0], [9.98, 53.0], [11.0, 50.0]] {
            assert!(shapes[0].contains(point), "{point:?}");
        }
    }

    #[test]
    fn a_knockout_text_is_its_plate() {
        let text = parse(
            r#"(gr_text "0b" (at 0 0 0) (layer "F.Cu" knockout) (effects (font (size 7 7) (thickness 0.6)))
              (render_cache "0b" 0 (polygon (pts (xy 1 1) (xy 3 1) (xy 3 4) (xy 1 4))) (polygon (pts (xy 5 1) (xy 8 1) (xy 8 4) (xy 5 4)))))"#,
        )
        .unwrap();
        let shapes = copper_graphic_shapes(&text).unwrap();
        assert_eq!(shapes.len(), 1);
        let bounds = shapes[0].aabb();
        assert!((bounds.minimum[0] - 0.4).abs() < 1e-9 && (bounds.maximum[0] - 8.6).abs() < 1e-9);
        assert!((bounds.minimum[1] - 0.4).abs() < 1e-9 && (bounds.maximum[1] - 4.6).abs() < 1e-9);
        // Without the knockout, the glyphs alone.
        let plain = parse(
            r#"(gr_text "0b" (at 0 0 0) (layer "F.Cu") (effects (font (size 7 7) (thickness 0.6)))
              (render_cache "0b" 0 (polygon (pts (xy 1 1) (xy 3 1) (xy 3 4) (xy 1 4))) (polygon (pts (xy 5 1) (xy 8 1) (xy 8 4) (xy 5 4)))))"#,
        )
        .unwrap();
        assert_eq!(copper_graphic_shapes(&plain).unwrap().len(), 2);
    }

    #[test]
    fn a_polygon_counts_its_stroke_and_only_a_fill_it_has() {
        let shapes = |text: &str| copper_graphic_shapes(&parse(text).unwrap()).unwrap();
        let reaches = |shapes: &[core::Shape], point: [f64; 2]| {
            shapes.iter().any(|shape| shape.contains(point))
        };
        let stroked = shapes(
            r#"(fp_poly (pts (xy 0 0) (xy 4 0) (xy 4 4) (xy 0 4)) (stroke (width 0.1) (type solid)) (fill yes) (layer "F.Mask"))"#,
        );
        assert!(reaches(&stroked, [2.0, 2.0]));
        assert!(reaches(&stroked, [4.04, 2.0]));
        assert!(!reaches(&stroked, [4.06, 2.0]));
        let outline = shapes(
            r#"(gr_poly (pts (xy 0 0) (xy 4 0) (xy 4 4) (xy 0 4)) (stroke (width 0.2) (type solid)) (fill no) (layer "F.Mask"))"#,
        );
        assert!(!reaches(&outline, [2.0, 2.0]));
        assert!(reaches(&outline, [3.95, 2.0]));
    }

    #[test]
    fn mask_openings_belong_to_the_net_they_expose() {
        let lowered = |zone: &str| {
            let pcb = parse(&format!(
                r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "a" (at 5 5)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/A"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "/B")))
              (gr_rect (start 4.6 4.6) (end 5.4 5.4) (stroke (width 0) (type solid)) (fill yes) (layer "F.Mask"))
              (gr_rect (start 4.6 4.6) (end 7.4 5.4) (stroke (width 0) (type solid)) (fill yes) (layer "F.Mask"))
              (gr_text "LABEL" (at 14 5) (layer "F.Mask") (effects (font (size 1 1) (thickness 0.15)))) {zone})"#
            ))
            .unwrap();
            let rules = KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            };
            let config = KiCadBoardRouterConfig {
                default_rules: Some(rules),
                ..KiCadBoardRouterConfig::default()
            };
            lower(&pcb, &config, false).unwrap().board
        };
        let openings = |board: &core::Board| -> Vec<(Option<core::NetId>, String)> {
            board
                .obstacles
                .iter()
                .filter(|obstacle| obstacle.label.starts_with("solder mask opening"))
                .map(|obstacle| (obstacle.net, obstacle.label.clone()))
                .collect()
        };
        let board = lowered("");
        let net_b = board
            .nets
            .iter()
            .position(|net| net.name.ends_with('B'))
            .map(|index| index as core::NetId);
        let net_a = board
            .nets
            .iter()
            .position(|net| net.name.ends_with('A'))
            .map(|index| index as core::NetId);
        // The first rectangle (its fill; no stroke) is pad 1's; the
        // second spans two pads and keeps everything out; so does text
        // over bare board (two nets under it would bridge).
        let bare = openings(&board);
        assert_eq!(bare.len(), 3);
        assert!(
            bare[..1]
                .iter()
                .all(|(net, _)| net.is_some() && *net == net_a)
        );
        assert!(
            bare[1..2]
                .iter()
                .all(|(net, label)| net.is_none() && label.ends_with("(gr_rect)"))
        );
        assert_eq!(bare[2], (None, "solder mask opening (gr_text)".to_string()));
        // Over a pour, the text exposes the fill: it is the pour's net's.
        let zone = r#"(zone (net "/B") (layer "F.Cu") (polygon (pts (xy 10 0) (xy 20 0) (xy 20 10) (xy 10 10))))"#;
        let poured = openings(&lowered(zone));
        assert_eq!(poured.len(), 3);
        assert_eq!(
            poured[2],
            (net_b, "solder mask opening (gr_text)".to_string())
        );
    }

    #[test]
    fn a_mask_only_pad_opens_the_mask_by_its_expansion() {
        // hackrf's mask-only test point: a 0.762 mm pad on F.Mask alone,
        // the board's mask expansion 0.05 mm.
        let lowered = |zone: &str, config: KiCadBoardRouterConfig| {
            let pcb = parse(&format!(
                r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
              (setup (pad_to_mask_clearance 0.05))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "TP" (layer "F.Cu") (at 5 5) (property "Reference" "TP1")
                (pad "" smd circle (at 0 0) (size 0.762 0.762) (layers "F.Mask")))
              (footprint "a" (at 15 5)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/A"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "/B"))) {zone})"#
            ))
            .unwrap();
            let rules = KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            };
            let config = KiCadBoardRouterConfig {
                default_rules: Some(rules),
                ..config
            };
            lower(&pcb, &config, false).unwrap().board
        };
        let openings = |board: &core::Board| -> Vec<(Option<core::NetId>, f64, u32, String)> {
            board
                .obstacles
                .iter()
                .filter(|obstacle| obstacle.label.starts_with("solder mask opening"))
                .map(|obstacle| {
                    (
                        obstacle.net,
                        obstacle.clearance,
                        obstacle.layers,
                        obstacle.label.clone(),
                    )
                })
                .collect()
        };
        // Over bare board it is no net's: nothing goes under it, by the
        // expansion, and on the front only.
        let bare = openings(&lowered("", KiCadBoardRouterConfig::default()));
        assert_eq!(bare.len(), 1);
        assert_eq!(bare[0].0, None);
        assert!((bare[0].1 - 0.05).abs() < 1.0e-9, "{}", bare[0].1);
        assert_eq!(bare[0].2, 1);
        assert_eq!(bare[0].3, "solder mask opening (pad TP1.)");
        // Over a pour it is the pour's net's.
        let zone = r#"(zone (net "/B") (layer "F.Cu") (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10))))"#;
        let board = lowered(zone, KiCadBoardRouterConfig::default());
        let net_b = board
            .nets
            .iter()
            .position(|net| net.name.ends_with('B'))
            .map(|index| index as core::NetId);
        assert_eq!(openings(&board)[0].0, net_b);
        // Off: the pad is nothing to the router.
        let off = KiCadBoardRouterConfig {
            mask_only_pad_openings: Some(false),
            ..KiCadBoardRouterConfig::default()
        };
        assert!(openings(&lowered("", off)).is_empty());
    }

    #[test]
    fn graphics_on_several_layers_carry_their_net() {
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (gr_rect (start 2 2) (end 4 4) (stroke (width 0) (type default)) (fill yes) (layers "F.Cu" "F.Mask") (net "GND"))
          (footprint "a" (at 10 5)
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND"))
            (pad "2" smd rect (at 3 0) (size 1 1) (layers "F.Cu") (net "GND"))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let ground = board
            .nets
            .iter()
            .position(|net| net.name == "GND")
            .map(|index| index as core::NetId);
        assert!(ground.is_some());
        let copper: Vec<_> = board
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.label == "copper gr_rect")
            .collect();
        assert_eq!(copper.len(), 1);
        assert_eq!((copper[0].net, copper[0].layers), (ground, 1));
        let opening: Vec<_> = board
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.label.starts_with("solder mask opening"))
            .collect();
        assert_eq!(opening.len(), 1);
        assert_eq!(opening[0].net, ground);
    }

    #[test]
    fn each_pad_keeps_its_own_zone_connection_beside_its_labels() {
        // 74th's J3.S1: one label, SMD shield pads without a zone
        // connection, through-hole ones with the zone's (thermal); J4 is
        // solid for the footprint, but its pad 2 is thermal by its own.
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 30 10) (layer "Edge.Cuts"))
          (footprint "usb" (at 5 5) (property "Reference" "J3")
            (pad "S1" smd rect (at -2 0) (size 1 1) (layers "F.Cu") (net "GND") (zone_connect 0))
            (pad "S1" thru_hole circle (at 2 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu") (net "GND"))
            (pad "1" smd rect (at 0 2) (size 0.5 1) (layers "F.Cu") (net "/D")))
          (footprint "hdr" (at 20 5) (property "Reference" "J4") (zone_connect 2)
            (pad "1" thru_hole circle (at 0 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu") (net "GND"))
            (pad "2" thru_hole circle (at 2.54 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu") (net "/D") (zone_connect 1))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let pad = |label: &str, x: f64| {
            board
                .obstacles
                .iter()
                .position(|obstacle| {
                    obstacle.label == label && obstacle.kind == core::ObstacleKind::Copper && {
                        let bounds = obstacle.shape.aabb();
                        ((bounds.minimum[0] + bounds.maximum[0]) / 2.0 - x).abs() < 1e-6
                    }
                })
                .unwrap_or_else(|| panic!("{label} at {x}"))
        };
        // The labels as before: J3.S1 counts as isolated, J4.1 as solid.
        assert_eq!(board.isolated_pads, vec!["J3.S1".to_string()]);
        assert_eq!(board.solid_pads, vec!["J4.1".to_string()]);
        // Per pad: only the SMD shield pad is isolated, the through-hole
        // one is not listed (the zone's connection); J4.2 is thermal by
        // its own setting.
        assert_eq!(board.isolated_pad_obstacles, vec![pad("J3.S1", 3.0)]);
        assert_eq!(board.solid_pad_obstacles, vec![pad("J4.1", 20.0)]);
        assert!(!board.isolated_pad_obstacles.contains(&pad("J3.S1", 7.0)));
    }

    #[test]
    fn a_custom_pad_spokes_from_its_copper_box_at_its_anchor_angle() {
        // SNSP-CPU-01's P7.1: a custom pad with a circle anchor and a
        // polygon primitive (KiCad's default spoke angle 45 degrees by
        // its anchor). KiCad builds its spokes over the box of its copper
        // at orientation 0 from that box's centre, turned with the pad:
        // here the polygon reaches 1 mm right of the anchor, so the box is
        // 2 x 1 mm around (0.5, 0) in the pad's frame, (5, 4.5) on the
        // board for a pad at (5, 5) turned by 90 degrees (Y down).
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "jst" (at 5 5) (property "Reference" "P7")
            (pad "1" thru_hole custom (at 0 0 90) (size 1 1) (drill 0.6) (layers "*.Cu") (net "GND")
              (options (clearance outline) (anchor circle))
              (primitives (gr_poly (pts (xy -0.5 -0.5) (xy 1.5 -0.5) (xy 1.5 0.5) (xy -0.5 0.5)) (width 0) (fill yes))))
            (pad "2" thru_hole circle (at 4 0) (size 1 1) (drill 0.6) (layers "*.Cu") (net "GND"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let pad = board
            .obstacles
            .iter()
            .position(|obstacle| {
                obstacle.label == "P7.1" && obstacle.kind == core::ObstacleKind::Copper
            })
            .unwrap();
        let frame = board.pad_spokes[&pad];
        assert!((frame.spoke_angle - 45.0).abs() < 1e-9);
        assert!(!frame.circular);
        assert!((frame.half[0] - 1.0).abs() < 1e-6 && (frame.half[1] - 0.5).abs() < 1e-6);
        assert!(
            (frame.center[0] - 5.0).abs() < 1e-6 && (frame.center[1] - 4.5).abs() < 1e-6,
            "{:?}",
            frame.center
        );
    }

    #[test]
    fn overlapping_pads_of_one_number_are_one_terminal() {
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "a" (at 5 5) (property "Reference" "TP1")
            (pad "1" thru_hole roundrect (at 0 0) (size 1.9 4.5) (drill 1) (layers "*.Cu") (roundrect_rratio 0.25) (net "/W"))
            (pad "1" thru_hole circle (at 0.5 1.2) (size 0.6 0.6) (drill 0.4) (layers "*.Cu") (net "/W"))
            (pad "1" thru_hole circle (at -0.5 -1.2) (size 0.6 0.6) (drill 0.4) (layers "*.Cu") (net "/W")))
          (footprint "b" (at 15 5) (property "Reference" "TP2")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/W"))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let net = board
            .nets
            .iter()
            .find(|net| net.name.ends_with('W'))
            .unwrap();
        assert_eq!(net.terminals.len(), 2);
        // The large pad stays, and the terminal points at its copper.
        assert_eq!(net.terminals[0].anchor, [5.0, 5.0]);
        assert_eq!(
            board.obstacles[net.terminals[0].pad].kind,
            core::ObstacleKind::Copper
        );
    }

    #[test]
    fn a_net_named_by_a_number_is_routed() {
        // Soraxem's usb-dmx: nets named "14" and "0" (KiCad 9's net table,
        // codes 6 and 14). The lowering took a name of digits for a net
        // code, left both nets out and called the board complete; KiCad
        // found their pads unconnected. An unquoted number in a net form
        // of one field is a code (an old track's `(net 6)`), named by the
        // board's net table.
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (net 0 "")
          (net 6 "14")
          (net 14 "0")
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "a" (at 5 5) (property "Reference" "D1")
            (pad "3" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 6 "14"))
            (pad "4" smd rect (at 0 2) (size 1 1) (layers "F.Cu") (net 14 "0")))
          (footprint "b" (at 15 5) (property "Reference" "U3")
            (pad "18" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 6 "14"))
            (pad "19" smd rect (at 0 2) (size 1 1) (layers "F.Cu") (net 14 "0")))
          (segment (start 1 1) (end 2 1) (width 0.2) (layer "B.Cu") (net 6)))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        for name in ["14", "0"] {
            let net = board.nets.iter().find(|net| net.name == name).unwrap();
            assert_eq!(net.terminals.len(), 2, "{name}");
        }
        // The old track's (net 6) is net "14" by its code.
        let track = board
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label == "existing track")
            .unwrap();
        let fourteen = board.nets.iter().position(|net| net.name == "14").unwrap();
        assert_eq!(track.net, Some(fourteen as core::NetId));
    }

    #[test]
    fn a_stroke_with_a_net_is_a_terminal() {
        // justbuchanan's ledboard (KiCad 8): copper arcs on Net-(D1-K),
        // named by code. KiCad's connectivity wants them joined to the
        // net's pads; the lowering read no net and left them out. A line
        // drawn from a pad is one item with it.
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (net 0 "")
          (net 2 "Net-(D1-K)")
          (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
          (footprint "a" (at 5 5) (property "Reference" "D1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 2 "Net-(D1-K)")))
          (footprint "b" (at 35 5) (property "Reference" "R1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 2 "Net-(D1-K)")))
          (gr_arc (start 10 15) (mid 20 12) (end 30 15) (stroke (width 1) (type solid)) (layer "F.Cu") (net 2))
          (gr_line (start 35 5) (end 35 9) (stroke (width 0.3) (type solid)) (layer "F.Cu") (net 2)))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let id = board
            .nets
            .iter()
            .position(|net| net.name == "Net-(D1-K)")
            .unwrap();
        let net = &board.nets[id];
        let labels: Vec<&str> = net
            .terminals
            .iter()
            .map(|terminal| terminal.label.as_str())
            .collect();
        assert_eq!(labels, vec!["D1.1", "R1.1", "gr_arc 1"]);
        let arc = &net.terminals[2];
        assert!(board.obstacles[arc.pad].shape.contains(arc.anchor));
        assert_eq!(board.obstacles[arc.pad].net, Some(id as core::NetId));
    }

    #[test]
    fn stacked_pads_of_one_number_on_two_sides_are_two_terminals() {
        // Knox Key's USB-C P1: pad A1 on F.Cu and again on B.Cu at the
        // same place, no hole. KiCad's connectivity wants the two joined
        // by copper (a via); one terminal let a track on B.Cu stand for
        // both. Pads of one number that overlap on a shared layer stay one.
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "usb" (at 5 5) (property "Reference" "P1")
            (pad "A1" smd roundrect (at 0 0) (size 0.7 0.7) (layers "B.Cu" "B.Mask") (roundrect_rratio 0.16) (net "GND"))
            (pad "A1" smd roundrect (at 0 0) (size 0.7 0.7) (layers "F.Cu" "F.Mask") (roundrect_rratio 0.16) (net "GND"))
            (pad "A2" smd rect (at 2 0) (size 1.2 1.2) (layers "F.Cu" "F.Mask") (net "GND"))
            (pad "A2" smd rect (at 2.3 0) (size 0.5 0.5) (layers "F.Cu" "F.Mask") (net "GND")))
          (footprint "b" (at 15 5) (property "Reference" "TP2")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let net = board.nets.iter().find(|net| net.name == "GND").unwrap();
        let labels: Vec<&str> = net
            .terminals
            .iter()
            .map(|terminal| terminal.label.as_str())
            .collect();
        // Both A1 pads, one A2 (the small one inside the large), TP2.1.
        assert_eq!(labels, vec!["P1.A1", "P1.A1", "P1.A2", "TP2.1"]);
        assert_ne!(net.terminals[0].layers, net.terminals[1].layers);
        assert_eq!(board.merged_pads.len(), 1);
    }

    #[test]
    fn pads_of_one_number_joined_through_a_third_are_one_terminal() {
        // MSign's U1.21 and Sisu's U2.11: an exposed pad on F.Cu, its copy
        // on B.Cu and thermal vias between them. The two exposed pads share
        // no layer, but each overlaps the vias: one item to KiCad (kicad-cli
        // 10.0.7 on this board with a track from F.Cu's pad to TP1: no
        // unconnected item; without the vias "Pad 5 [GND] of U1 on B.Cu"
        // is unconnected from the track).
        let board_text = |vias: bool| {
            format!(
                r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "ep" (at 5 5) (property "Reference" "U1")
            (pad "5" smd rect (at 0 0) (size 1.7 1.7) (layers "F.Cu" "F.Mask") (net 1 "GND"))
            (pad "5" smd rect (at 0 0) (size 1.7 1.7) (layers "B.Cu") (net 1 "GND"))
            {})
          (footprint "tp" (at 15 5) (property "Reference" "TP1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu" "F.Mask") (net 1 "GND"))))"#,
                if vias {
                    r#"(pad "5" thru_hole circle (at -0.6 -0.6) (size 0.5 0.5) (drill 0.3) (layers "*.Cu") (net 1 "GND"))
            (pad "5" thru_hole circle (at 0.6 0.6) (size 0.5 0.5) (drill 0.3) (layers "*.Cu") (net 1 "GND"))"#
                } else {
                    ""
                }
            )
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let labels = |vias: bool| {
            let board = lower(&parse(&board_text(vias)).unwrap(), &config, false)
                .unwrap()
                .board;
            let net = board.nets.iter().find(|net| net.name == "GND").unwrap();
            let labels: Vec<String> = net
                .terminals
                .iter()
                .map(|terminal| terminal.label.clone())
                .collect();
            (labels, board.merged_pads.len())
        };
        assert_eq!(labels(true), (vec!["U1.5".into(), "TP1.1".into()], 3));
        assert_eq!(
            labels(false),
            (vec!["U1.5".into(), "U1.5".into(), "TP1.1".into()], 0)
        );
    }

    #[test]
    fn pads_of_a_jumper_group_are_one_terminal() {
        // KiCad 9's footprint jumpers: kicad-cli 10.0.7 on this board, a
        // track from SW1's right pad 1 to TP1, finds pad 2 and the left pad
        // 1 unconnected without jumpers; with
        // duplicate_pad_numbers_are_jumpers only pad 2; with
        // jumper_pad_groups ("1" "2") nothing.
        let board_text = |settings: &str| {
            format!(
                r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 30 10) (layer "Edge.Cuts"))
          (footprint "sw" (at 10 5) (property "Reference" "SW1") {settings}
            (pad "1" smd rect (at -2 0) (size 1 1) (layers "F.Cu" "F.Mask") (net 1 "GND"))
            (pad "1" smd rect (at 2 0) (size 1 1) (layers "F.Cu" "F.Mask") (net 1 "GND"))
            (pad "2" smd rect (at 0 2) (size 1 1) (layers "F.Cu" "F.Mask") (net 1 "GND")))
          (footprint "tp" (at 25 5) (property "Reference" "TP1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu" "F.Mask") (net 1 "GND"))))"#
            )
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let terminals = |settings: &str| {
            let board = lower(&parse(&board_text(settings)).unwrap(), &config, false)
                .unwrap()
                .board;
            let net = board.nets.iter().find(|net| net.name == "GND").unwrap();
            net.terminals.len()
        };
        assert_eq!(terminals(""), 4);
        assert_eq!(terminals("(duplicate_pad_numbers_are_jumpers yes)"), 3);
        assert_eq!(terminals(r#"(jumper_pad_groups ("1" "2"))"#), 2);
    }

    #[test]
    fn a_footprints_only_custom_pad_may_hold_its_nets_vias() {
        // d20_pcb's J2: a touch electrode, one custom pad (a face of the
        // die), soldered to nothing. A custom pad beside another pad of
        // its footprint is a land like any SMD pad.
        let board = |second_pad: &str| {
            let pcb = parse(&format!(
                r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "touch" (at 10 5) (property "Reference" "J2")
            (pad "1" smd custom (at 0 0) (size 1 1) (layers "F.Cu" "F.Mask") (net "/PAD")
              (options (clearance outline) (anchor rect))
              (primitives (gr_poly (pts (xy -3 -3) (xy 3 -3) (xy 3 3) (xy -3 3)) (width 0) (fill yes))))
            {second_pad})
          (footprint "R" (at 2 2) (property "Reference" "R1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/PAD"))))"#
            ))
            .unwrap();
            let config = KiCadBoardRouterConfig {
                default_rules: Some(KiCadConnectionRoutingRules {
                    trace_width_mm: 0.2,
                    clearance_mm: 0.2,
                    via_size_mm: 0.6,
                    via_drill_mm: 0.3,
                }),
                ..KiCadBoardRouterConfig::default()
            };
            let board = lower(&pcb, &config, false).unwrap().board;
            let listed = |label: &str| {
                board
                    .no_via_pads
                    .iter()
                    .any(|&index| board.obstacles[index].label == label)
            };
            (listed("J2.1"), listed("R1.1"))
        };
        assert_eq!(board(""), (false, true));
        assert_eq!(
            board(r#"(pad "2" smd rect (at 5 0) (size 1 1) (layers "F.Cu") (net "/PAD"))"#),
            (true, true)
        );
    }

    #[test]
    fn an_unfilled_circle_in_a_custom_pad_is_a_ring() {
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "dome" (at 10 5) (property "Reference" "SW1")
            (pad "1" smd circle (at 0 0) (size 4 4) (layers "F.Cu") (net "/A"))
            (pad "2" smd custom (at 2.75 0) (size 0.7 0.7) (layers "F.Cu") (net "/B")
              (options (clearance outline) (anchor circle))
              (primitives (gr_circle (center -2.75 0) (end 0 0) (width 0.7) (fill no))))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let ring = board
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label == "SW1.2")
            .unwrap();
        // The inner pad's centre is free of the ring; the ring itself is copper.
        assert!(ring.shape.distance_to_point([10.0, 5.0]) > 2.0);
        assert!(ring.shape.distance_to_point([12.75, 5.0]) < 1.0e-6);
    }

    #[test]
    fn other_nets_vias_keep_the_hole_clearance_from_a_plated_hole() {
        // jetson-nano-baseboard's J2 pad 1: a 0.55 drill in 0.75 copper,
        // clearance 0.125 and hole clearance 0.25. A 0.5 via of another
        // net 0.769 from the pad's centre keeps 0.144 from its copper but
        // only 0.244 from its hole (KiCad: hole_clearance, 0.2438). The
        // pad's own net is exempt.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts"))
              (footprint "dp" (at 5 5) (property "Reference" "J2")
                (pad "1" thru_hole circle (at 0 0) (size 0.75 0.75) (drill 0.55) (layers "*.Cu" "*.Mask") (net "GND"))
                (pad "2" thru_hole circle (at 0 -3) (size 0.75 0.75) (drill 0.55) (layers "*.Cu" "*.Mask") (net "3V3_SYS")))
              (footprint "c" (at 8 8) (property "Reference" "C1")
                (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "GND"))
                (pad "2" smd rect (at 1 0) (size 0.5 0.5) (layers "F.Cu") (net "3V3_SYS"))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules {
            trace_width_mm: 0.1,
            clearance_mm: 0.125,
            via_size_mm: 0.5,
            via_drill_mm: 0.15,
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules),
            hole_clearance_mm: 0.25,
            hole_to_hole_clearance_mm: 0.25,
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let net =
            |name: &str| board.nets.iter().position(|net| net.name == name).unwrap() as core::NetId;
        let (ground, supply) = (net("GND"), net("3V3_SYS"));
        let hole = board
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label == "J2.1 plated hole")
            .unwrap();
        assert!(hole.blocks_vias && hole.net == Some(ground));
        // The via's place on a lattice node: J2.1 at (5, 5), the via at
        // (-0.741, +0.205) from it as on jetson.
        let at = [5.0 - 0.741, 5.0 + 0.205];
        let pitch = 0.1;
        let grid = core::grid::Grid {
            origin: [at[0] - 42.0 * pitch, at[1] - 52.0 * pitch],
            pitch,
            nx: 99,
            ny: 99,
        };
        let node = grid.nearest_node(at).unwrap();
        assert!(core::geometry::distance(grid.center_of(node), at) < 1.0e-9);
        let class = board.nets[supply as usize].class;
        let maps = core::grid::StaticMaps::build(&board, &grid, class, true);
        assert!(
            !maps.via_allowed(node, supply),
            "another net's via within the hole clearance"
        );
        assert!(
            maps.via_allowed(node, ground),
            "the pad's own net keeps the copper rule"
        );
        // One node further out (0.866 from the centre, 0.775 needed) is free.
        assert!(maps.via_allowed(node - 1, supply));
        // The exact verifier reports the via against the hole.
        let via_at = |net: core::NetId| {
            let mut routes = vec![core::NetRoute::default(); board.nets.len()];
            routes[net as usize].vias.push(core::board::Via {
                at,
                diameter: 0.5,
                drill: 0.15,
            });
            core::verify(&board, &routes)
        };
        let found = via_at(supply);
        let hole_finding = found
            .iter()
            .find(|violation| violation.other == "J2.1 plated hole")
            .expect("a hole clearance violation");
        assert!((hole_finding.required - 0.25).abs() < 1.0e-9);
        assert!(
            (hole_finding.actual - 0.2439).abs() < 1.0e-3,
            "{}",
            hole_finding.actual
        );
        assert!(
            !via_at(ground)
                .iter()
                .any(|violation| violation.other.starts_with("J2.1"))
        );
    }

    #[test]
    fn starved_thermals_come_with_advice() {
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-starved-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("drc.json"),
            r#"{"violations": [
              {"type": "starved_thermal", "description": "Thermal relief connection to zone incomplete (layer F.Cu; zone min spoke count 2; actual 1)",
               "items": [{"description": "Zone [GND] on F.Cu", "pos": {"x": 1, "y": 2}}, {"description": "Pad 4 [GND] of U201 on F.Cu", "pos": {"x": 3.5, "y": 4}}]},
              {"type": "starved_thermal", "description": "Thermal relief connection to zone incomplete (layer F.Cu; 4 spokes connected to isolated island)",
               "items": [{"description": "PTH pad 4 [MCU_GND] of J102", "pos": {"x": 5, "y": 6}}]},
              {"type": "clearance", "description": "x", "items": []}]}"#,
        )
        .unwrap();
        let found = starved_thermal_diagnostics(&directory);
        fs::remove_dir_all(&directory).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(
            (
                found[0].layer.as_str(),
                found[0].spokes,
                found[0].needed,
                found[0].at
            ),
            ("F.Cu", Some(1), Some(2), [3.5, 4.0])
        );
        assert!(found[0].advice.contains("only 1 of 2 spokes"));
        assert!(found[1].advice.contains("not joined"));
        assert!(found[1].pad.contains("J102"));
    }

    #[test]
    fn requested_pours_cover_the_outline_once_per_net_and_layer() {
        let mut pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "a" (at 5 5) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/GND")))
          (zone (net "/GND") (layer "B.Cu") (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10)))))"#,
        )
        .unwrap();
        let request = |layers: &[&str]| KiCadPourRequest {
            net: "GND".into(),
            layers: layers.iter().map(|layer| layer.to_string()).collect(),
            clearance_mm: None,
            plane: false,
            keep_whole: Vec::new(),
        };
        add_pour_zones(&mut pcb, &[request(&["F.Cu", "B.Cu"])]).unwrap();
        let zones: Vec<_> = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("zone"))
            .collect();
        // B.Cu had one already; F.Cu got a new one over the outline.
        assert_eq!(zones.len(), 2);
        assert_eq!(form_atom(zones[1], "layer", 1), Some("F.Cu"));
        assert_eq!(node_net(zones[1]), Some("/GND"));
        let unknown = KiCadPourRequest {
            net: "VCC".into(),
            ..request(&["F.Cu"])
        };
        assert!(add_pour_zones(&mut pcb, &[unknown]).is_err());
        assert!(add_pour_zones(&mut pcb, &[request(&["In1.Cu"])]).is_err());
    }

    #[test]
    fn a_zone_on_a_layer_set_counts_as_existing_on_each_of_its_layers() {
        // The trial's board A (2026-10-26): GND on "F&B.Cu"; a request
        // for F.Cu wrote a second zone over it (zones_intersect).
        for layers in [
            r#"(layers "F&B.Cu")"#,
            r#"(layers "*.Cu")"#,
            r#"(layers "F.Cu" "B.Cu")"#,
        ] {
            let mut pcb = parse(&format!(
                r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "a" (at 5 5) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "GND")))
              (zone (net 1) (net_name "GND") {layers} (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10)))))"#
            ))
            .unwrap();
            let request = KiCadPourRequest {
                net: "GND".into(),
                layers: vec!["F.Cu".into(), "B.Cu".into()],
                clearance_mm: None,
                plane: false,
                keep_whole: Vec::new(),
            };
            add_pour_zones(&mut pcb, &[request]).unwrap();
            let zones = pcb
                .children()
                .iter()
                .filter(|item| item.head() == Some("zone"))
                .count();
            assert_eq!(zones, 1, "{layers}");
        }
    }

    #[test]
    fn keep_whole_regions_over_other_nets_pads_are_warned_of_naming_the_pads() {
        // U1 on F.Cu with a GND pad and two signal pads; a GND pour on F.Cu
        // kept whole under U1 would cut U1:2 and U1:3 off; on B.Cu nothing.
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 30 10) (layer "Edge.Cuts"))
          (footprint "u" (at 10 5) (property "Reference" "U1")
            (fp_rect (start -2 -2) (end 2 2) (layer "F.CrtYd"))
            (pad "1" smd rect (at -1 0) (size 0.5 0.5) (layers "F.Cu") (net "/GND"))
            (pad "2" smd rect (at 1 0) (size 0.5 0.5) (layers "F.Cu") (net "/A"))
            (pad "3" smd rect (at 1 1) (size 0.5 0.5) (layers "F.Cu") (net "/B"))
            (pad "4" smd rect (at 1 -1) (size 0.5 0.5) (layers "F.Cu") (net "/GND"))
            (pad "5" smd rect (at -1 1) (size 0.5 0.5) (layers "F.Cu") (net "/GND"))
            (pad "6" smd rect (at -1 -1) (size 0.5 0.5) (layers "F.Cu") (net "/GND")))
          (footprint "r" (at 11 5) (property "Reference" "R1")
            (pad "1" smd rect (at 0 0.5) (size 0.5 0.5) (layers "F.Cu") (net "/C"))))"#,
        )
        .unwrap();
        let request = |layer: &str| -> KiCadPourRequest {
            serde_json::from_str(&format!(
                r#"{{"net": "GND", "layers": ["{layer}"], "keep_whole": [{{"parts": ["U1"]}}]}}"#
            ))
            .unwrap()
        };
        let placed = keep_whole_warnings(&pcb, &[request("F.Cu")], true).unwrap();
        assert_eq!(placed.len(), 1);
        assert!(
            placed[0].contains("covers 3 pads") && placed[0].contains("U1:2, U1:3, R1:1"),
            "{}",
            placed[0]
        );
        // Before a layout's placement only U1's own pads count.
        let unplaced = keep_whole_warnings(&pcb, &[request("F.Cu")], false).unwrap();
        assert!(
            unplaced[0].contains("covers 2 pads of other nets on that layer (U1:2, U1:3)"),
            "{}",
            unplaced[0]
        );
        assert!(
            keep_whole_warnings(&pcb, &[request("B.Cu")], true)
                .unwrap()
                .is_empty()
        );
        // The pour advice's candidates: U1 on B.Cu, nothing on F.Cu.
        let layers = LayerTable::from_pcb(&pcb).unwrap();
        let pads = region_pads(&pcb, &layers).unwrap();
        let parts = keep_whole_parts(&pcb, &pads).unwrap();
        assert_eq!(
            keep_whole_candidates(&parts, &pads, "GND", 1),
            vec!["U1".to_string()]
        );
        assert!(keep_whole_candidates(&parts, &pads, "GND", 0).is_empty());
    }

    #[test]
    fn a_plane_request_keeps_tracks_off_its_layer_by_a_rule_area() {
        // The fence's four layers: GND's zone on In1.Cu exists; a plane
        // request adds the rule area alone, once.
        let mut pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "a" (at 5 5) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/GND"))
            (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "/A")))
          (footprint "b" (at 15 5) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/GND"))
            (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "/A")))
          (zone (net "/GND") (layer "In1.Cu") (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10) (xy 0 10)))))"#,
        )
        .unwrap();
        let plane = KiCadPourRequest {
            net: "GND".into(),
            layers: vec!["In1.Cu".into()],
            clearance_mm: None,
            plane: true,
            keep_whole: Vec::new(),
        };
        add_pour_zones(&mut pcb, std::slice::from_ref(&plane)).unwrap();
        add_pour_zones(&mut pcb, std::slice::from_ref(&plane)).unwrap();
        let zones: Vec<&Expr> = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("zone"))
            .collect();
        assert_eq!(zones.len(), 2);
        let text = encode(zones[1]).to_string();
        assert!(
            text.contains("(name \"plane GND In1.Cu\")")
                && text.contains("(tracks not_allowed)")
                && text.contains("(vias allowed)")
                && text.contains("(copperpour allowed)"),
            "{text}"
        );
        // The router keeps every track off In1.Cu, vias pass.
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, true).unwrap().board;
        let keepout = board
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label == "plane GND In1.Cu")
            .unwrap();
        assert!(keepout.blocks_tracks && !keepout.blocks_vias && keepout.layers == 0b10);
        // The plane's own fill still fills In1.Cu in the pour model, as
        // without the rule area (the rule area hid it before 2026-10-21).
        let report = |pcb: &Expr| {
            let board = lower(pcb, &config, true).unwrap().board;
            let mut router = core::router::Router::new(&board, &core_config(&config));
            router.adopt_existing_copper();
            router.pour_report()
        };
        let mut bare = pcb.clone();
        if let Expr::List(items) = &mut bare {
            items.retain(|item| form_atom(item, "name", 1) != Some("plane GND In1.Cu"));
        }
        let with_area = report(&pcb);
        assert!(with_area[0].inside_mm2 > 150.0, "{with_area:?}");
        assert_eq!(with_area, report(&bare));
    }

    #[test]
    fn keep_whole_keeps_other_nets_off_the_pour_under_a_part() {
        // A 2-layer board with GND poured on B.Cu; U1 at (10, 5) and a
        // box at the right end are to stay whole.
        let mut pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 30 10) (layer "Edge.Cuts"))
          (footprint "u" (at 10 5) (property "Reference" "U1")
            (fp_rect (start -2 -2) (end 2 2) (layer "F.CrtYd"))
            (pad "1" smd rect (at -1 0) (size 0.5 0.5) (layers "F.Cu") (net "/GND"))
            (pad "2" smd rect (at 1 0) (size 0.5 0.5) (layers "F.Cu") (net "/A")))
          (footprint "r" (at 3 5) (property "Reference" "R1")
            (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "/GND"))
            (pad "2" smd rect (at 1 0) (size 0.5 0.5) (layers "F.Cu") (net "/A"))))"#,
        )
        .unwrap();
        let request: KiCadPourRequest = serde_json::from_str(
            r#"{"net": "GND", "layers": ["B.Cu"], "keep_whole": [{"parts": ["U*"]}, {"x": [25, 28], "y": [2, 8]}]}"#,
        )
        .unwrap();
        add_pour_zones(&mut pcb, std::slice::from_ref(&request)).unwrap();
        // Twice: the regions of the earlier call give way.
        add_pour_zones(&mut pcb, std::slice::from_ref(&request)).unwrap();
        let areas: Vec<String> = pcb
            .children()
            .iter()
            .filter(|item| {
                item.head() == Some("zone")
                    && form_atom(item, "name", 1) == Some("keep whole GND B.Cu")
            })
            .map(|zone| encode(zone).to_string())
            .collect();
        assert_eq!(areas.len(), 2, "{areas:?}");
        let areas: Vec<String> = areas
            .iter()
            .map(|area| area.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        // U1's courtyard (8..12, 3..7) grown by 0.5 mm; KiCad checks
        // nothing there (it has no rule by net).
        assert!(
            areas[0].contains("(xy 7.5 2.5)")
                && areas[0].contains("(xy 12.5 7.5)")
                && areas[0].contains("(tracks allowed) (vias allowed)"),
            "{}",
            areas[0]
        );
        assert!(areas[1].contains("(xy 25 2)") && areas[1].contains("(xy 28 8)"));
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, true).unwrap().board;
        let net =
            |name: &str| board.nets.iter().position(|net| net.name == name).unwrap() as core::NetId;
        let (ground, signal) = (net("GND"), net("A"));
        let region = board
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label == "keep whole GND B.Cu")
            .unwrap();
        assert!(
            region.net == Some(ground)
                && region.blocks_tracks
                && region.blocks_vias
                && region.layers == 0b10
        );
        let pitch = 0.1;
        let grid = core::grid::Grid {
            origin: [0.05, 0.05],
            pitch,
            nx: 299,
            ny: 99,
        };
        let maps =
            core::grid::StaticMaps::build(&board, &grid, board.nets[signal as usize].class, true);
        // In the box at the right end: another net's track on B.Cu and its
        // via are out, GND's pass and its fill fills; F.Cu is free.
        let inside = grid.nearest_node([26.5, 5.0]).unwrap();
        assert!(!maps.trace_allowed(1, inside, signal));
        assert!(!maps.via_allowed(inside, signal));
        assert!(maps.trace_allowed(1, inside, ground));
        assert!(maps.via_allowed(inside, ground));
        assert!(maps.fill_allowed(1, inside, ground));
        assert!(maps.trace_allowed(0, inside, signal));
        // Outside: free for both.
        let outside = grid.nearest_node([20.0, 5.0]).unwrap();
        assert!(maps.trace_allowed(1, outside, signal) && maps.via_allowed(outside, signal));
        // An unknown part is an input error.
        let unknown: KiCadPourRequest = serde_json::from_str(
            r#"{"net": "GND", "layers": ["B.Cu"], "keep_whole": [{"parts": ["Q9"]}]}"#,
        )
        .unwrap();
        assert!(add_pour_zones(&mut pcb, &[unknown]).is_err());
    }

    #[test]
    fn smd_pads_but_an_exposed_pad_hold_no_via_unless_the_board_allows_vias_in_pads() {
        // A QFN-like part: four pins around a 2 mm exposed pad; a 0805
        // capacitor; a through-hole header pin; a SOT-223-like part whose
        // tab lies beside its pins (not exposed).
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 30 20) (layer "Edge.Cuts"))
              (footprint "qfn" (at 5 5) (property "Reference" "U1")
                (pad "1" smd rect (at -1.5 0) (size 0.6 0.3) (layers "F.Cu") (net "A"))
                (pad "2" smd rect (at 1.5 0) (size 0.6 0.3) (layers "F.Cu") (net "B"))
                (pad "3" smd rect (at 0 -1.5) (size 0.3 0.6) (layers "F.Cu") (net "A"))
                (pad "4" smd rect (at 0 1.5) (size 0.3 0.6) (layers "F.Cu") (net "B"))
                (pad "5" smd rect (at 0 0) (size 2 2) (layers "F.Cu") (net "GND")))
              (footprint "c" (at 15 5) (property "Reference" "C1")
                (pad "1" smd rect (at 0 0) (size 1 1.3) (layers "F.Cu") (net "GND"))
                (pad "2" smd rect (at 2 0) (size 1 1.3) (layers "F.Cu") (net "A")))
              (footprint "pin" (at 25 5) (property "Reference" "J1")
                (pad "1" thru_hole circle (at 0 0) (size 1.7 1.7) (drill 1) (layers "*.Cu" "*.Mask") (net "GND"))
                (pad "2" thru_hole circle (at 0 2.54) (size 1.7 1.7) (drill 1) (layers "*.Cu" "*.Mask") (net "B")))
              (footprint "sot223" (at 10 15) (property "Reference" "U2")
                (pad "1" smd rect (at -2.3 3.15) (size 1 1.5) (layers "F.Cu") (net "GND"))
                (pad "2" smd rect (at 0 3.15) (size 1 1.5) (layers "F.Cu") (net "A"))
                (pad "3" smd rect (at 2.3 3.15) (size 1 1.5) (layers "F.Cu") (net "B"))
                (pad "4" smd rect (at 0 -3.15) (size 3.5 1.5) (layers "F.Cu") (net "A"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let listed = |config: &KiCadBoardRouterConfig| -> Vec<String> {
            let board = lower(&pcb, config, false).unwrap().board;
            board
                .no_via_pads
                .iter()
                .map(|index| board.obstacles[*index].label.clone())
                .collect()
        };
        assert_eq!(
            listed(&config),
            [
                "U1.1", "U1.2", "U1.3", "U1.4", "C1.1", "C1.2", "U2.1", "U2.2", "U2.3", "U2.4"
            ]
        );
        let allowed = KiCadBoardRouterConfig {
            via_in_pad: Some(true),
            ..config
        };
        assert!(listed(&allowed).is_empty());
    }

    #[test]
    fn a_two_sided_or_split_exposed_pad_is_one_exposed_pad() {
        // U1: Sisu's U2.11, an exposed pad drawn twice at one place (F.Cu
        // and B.Cu, 1.2 x 2 mm) with a thermal via between, six pins
        // around. U2: an exposed pad split into four same-number pieces
        // of 1 x 1 mm (0.1 mm apart), with paste-only apertures that
        // carry no copper. U3: two same-number tabs far apart beside a
        // row of pins (a connector's shield pads): not exposed.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
              (footprint "dfn" (at 5 5) (property "Reference" "U1")
                (pad "1" smd rect (at -1 -0.65) (size 0.6 0.3) (layers "F.Cu") (net "A"))
                (pad "2" smd rect (at -1 0) (size 0.6 0.3) (layers "F.Cu") (net "B"))
                (pad "3" smd rect (at -1 0.65) (size 0.6 0.3) (layers "F.Cu") (net "A"))
                (pad "4" smd rect (at 1 0.65) (size 0.6 0.3) (layers "F.Cu") (net "B"))
                (pad "5" smd rect (at 1 0) (size 0.6 0.3) (layers "F.Cu") (net "A"))
                (pad "6" smd rect (at 1 -0.65) (size 0.6 0.3) (layers "F.Cu") (net "B"))
                (pad "11" smd rect (at 0 0) (size 1.2 2) (layers "F.Cu" "F.Paste" "F.Mask") (net "GND"))
                (pad "11" smd rect (at 0 0) (size 1.2 2) (layers "B.Cu") (net "GND"))
                (pad "11" thru_hole circle (at 0 0) (size 0.5 0.5) (drill 0.3) (layers "*.Cu") (net "GND")))
              (footprint "qfn" (at 15 5) (property "Reference" "U2")
                (pad "1" smd rect (at -2 0) (size 0.6 0.3) (layers "F.Cu") (net "A"))
                (pad "2" smd rect (at 2 0) (size 0.6 0.3) (layers "F.Cu") (net "B"))
                (pad "3" smd rect (at 0 -2) (size 0.3 0.6) (layers "F.Cu") (net "A"))
                (pad "4" smd rect (at 0 2) (size 0.3 0.6) (layers "F.Cu") (net "B"))
                (pad "5" smd rect (at -0.55 -0.55) (size 1 1) (layers "F.Cu" "F.Mask") (net "GND"))
                (pad "5" smd rect (at 0.55 -0.55) (size 1 1) (layers "F.Cu" "F.Mask") (net "GND"))
                (pad "5" smd rect (at -0.55 0.55) (size 1 1) (layers "F.Cu" "F.Mask") (net "GND"))
                (pad "5" smd rect (at 0.55 0.55) (size 1 1) (layers "F.Cu" "F.Mask") (net "GND"))
                (pad "" smd rect (at -0.5 -0.5) (size 0.8 0.8) (layers "F.Paste"))
                (pad "" smd rect (at 0.5 0.5) (size 0.8 0.8) (layers "F.Paste")))
              (footprint "usb" (at 30 10) (property "Reference" "U3")
                (pad "1" smd rect (at -1 0) (size 0.4 1.3) (layers "F.Cu") (net "A"))
                (pad "2" smd rect (at 0 0) (size 0.4 1.3) (layers "F.Cu") (net "B"))
                (pad "3" smd rect (at 1 0) (size 0.4 1.3) (layers "F.Cu") (net "A"))
                (pad "6" smd rect (at -4 2) (size 1.5 1.5) (layers "F.Cu") (net "GND"))
                (pad "6" smd rect (at 4 2) (size 1.5 1.5) (layers "F.Cu") (net "GND"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let listed: Vec<&str> = board
            .no_via_pads
            .iter()
            .map(|index| board.obstacles[*index].label.as_str())
            .collect();
        assert_eq!(
            listed,
            [
                "U1.1", "U1.2", "U1.3", "U1.4", "U1.5", "U1.6", "U2.1", "U2.2", "U2.3", "U2.4",
                "U3.1", "U3.2", "U3.3", "U3.6", "U3.6"
            ]
        );
    }
}

mod narrow_step_tests {
    use super::*;

    fn rules(width: f64, clearance: f64) -> KiCadConnectionRoutingRules {
        KiCadConnectionRoutingRules {
            trace_width_mm: width,
            clearance_mm: clearance,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }
    }

    #[test]
    fn the_step_applies_only_with_a_narrower_width_or_a_relaxed_clearance() {
        let plain = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.2, 0.2)),
            ..KiCadBoardRouterConfig::default()
        };
        assert!(!narrow_step_applies(&plain));
        // A menu width no narrower than every class: nothing to narrow.
        let same = KiCadBoardRouterConfig {
            narrow_signal_mm: Some(0.2),
            ..plain.clone()
        };
        assert!(!narrow_step_applies(&same));
        let narrower = KiCadBoardRouterConfig {
            narrow_signal_mm: Some(0.15),
            ..plain.clone()
        };
        assert!(narrow_step_applies(&narrower));
        // Already narrow (the step itself): it does not apply again.
        let narrowed = KiCadBoardRouterConfig {
            use_narrow_signals: Some(true),
            ..narrower.clone()
        };
        assert!(!narrow_step_applies(&narrowed));
        let relaxed = KiCadBoardRouterConfig {
            relaxed_clearance_mm: Some(0.1),
            ..plain.clone()
        };
        assert!(narrow_step_applies(&relaxed));
        let not_smaller = KiCadBoardRouterConfig {
            relaxed_clearance_mm: Some(0.2),
            ..plain.clone()
        };
        assert!(!narrow_step_applies(&not_smaller));
        // A wide class among narrow ones is enough.
        let mut classes = plain.clone();
        classes.default_rules = Some(rules(0.125, 0.2));
        classes
            .connection_rules
            .insert("/PWR".into(), rules(0.5, 0.2));
        classes.narrow_signal_mm = Some(0.125);
        assert!(narrow_step_applies(&classes));
    }

    #[test]
    fn the_step_needs_half_its_reserve_and_three_floors_of_negotiation() {
        // The A13 module: 278 s kept, about 600 s left: it runs.
        assert_eq!(narrow_step_minimum(278.0), 180.0);
        assert_eq!(narrow_step_minimum(600.0), 300.0);
        // Jetson: 129 s kept, 70 s left: it does not.
        assert!(70.0 < narrow_step_minimum(129.0));
    }

    #[test]
    fn the_step_negotiates_a_third_of_what_is_left() {
        assert_eq!(narrow_step_negotiation(450.0, None), 150.0);
        assert_eq!(narrow_step_negotiation(90.0, None), 60.0);
        assert_eq!(narrow_step_negotiation(6000.0, None), 900.0);
        // Never more than the attempt it varies had.
        assert_eq!(narrow_step_negotiation(450.0, Some(100.0)), 100.0);
        assert_eq!(narrow_step_negotiation(450.0, Some(200.0)), 150.0);
    }

    #[test]
    fn the_step_negotiates_at_least_what_the_first_attempt_did_within_the_reserve() {
        // A finer rung with 60 s of negotiation is the best attempt; the
        // first attempt negotiated 120 s, 300 s kept and 330 s left.
        assert_eq!(narrow_step_negotiation(330.0, Some(60.0)), 60.0);
        assert_eq!(
            narrow_step_negotiation_floored(330.0, Some(60.0), 120.0, 300.0),
            120.0
        );
        // Bounded by the reserve and by half of what is left.
        assert_eq!(
            narrow_step_negotiation_floored(1000.0, Some(60.0), 400.0, 250.0),
            250.0
        );
        assert_eq!(
            narrow_step_negotiation_floored(300.0, Some(60.0), 400.0, 600.0),
            150.0
        );
        // Never less than before.
        assert_eq!(
            narrow_step_negotiation_floored(900.0, None, 100.0, 300.0),
            300.0
        );
        assert_eq!(
            narrow_step_negotiation_floored(900.0, None, 0.0, 300.0),
            300.0
        );
    }

    #[test]
    fn the_floored_reserve_is_enough_for_the_step_to_run() {
        // TelemetryOnboard: a short first attempt kept 120 s before the
        // floor (2026-10-15), below the step's 180 s minimum; as a floor
        // it keeps 240 s.
        assert!(120.0 < narrow_step_minimum(120.0));
        let kept = narrow_step_reserve(100.0, 0.75, 600.0);
        assert_eq!(kept, NARROW_RESERVE_FLOOR);
        assert!(kept >= narrow_step_minimum(kept));
        // A share of the first attempt, at most the cap.
        assert_eq!(narrow_step_reserve(400.0, 0.75, 600.0), 300.0);
        assert_eq!(narrow_step_reserve(1000.0, 0.75, 600.0), 600.0);
        assert_eq!(narrow_step_reserve(400.0, 0.0, 600.0), 0.0);
        // A cap under the floor still keeps what the step can use...
        assert_eq!(narrow_step_reserve(100.0, 0.75, 200.0), 200.0);
        // ...and nothing when the step could never run on it.
        assert_eq!(narrow_step_reserve(100.0, 0.75, 150.0), 0.0);
        // Whatever the first attempt, a kept reserve lets the step run.
        for work in [10.0, 100.0, 320.0, 500.0, 2000.0] {
            for cap in [100.0, 179.0, 180.0, 240.0, 400.0, 600.0] {
                let kept = narrow_step_reserve(work, 0.75, cap);
                assert!(
                    kept == 0.0 || kept >= narrow_step_minimum(kept),
                    "{work} {cap}"
                );
            }
        }
    }

    #[test]
    fn no_attempt_starts_into_the_reserve() {
        // DSP-ADAU1452: 300 s kept, 450 s left, the last 0.075 mm rung took
        // 100 s: another rung at that pitch fits, one at 0.05 mm (225 s
        // projected) does not.
        let next = projected_attempt_work(100.0, 0.075, 0.075);
        assert_eq!(next, 100.0);
        assert!(fits_before_reserve(450.0, 300.0, next));
        let finer = projected_attempt_work(100.0, 0.075, 0.05);
        assert!((finer - 225.0).abs() < 1.0e-9);
        assert!(!fits_before_reserve(450.0, 300.0, finer));
        // A coarser or equal pitch is not scaled down.
        assert_eq!(projected_attempt_work(100.0, 0.075, 0.1), 100.0);
        // An attempt counts at least 120 s.
        assert!(!fits_before_reserve(400.0, 300.0, 30.0));
        assert!(fits_before_reserve(420.0, 300.0, 30.0));
        // The whole reserve, not half of it: 150 s left beyond half of a
        // 300 s reserve is not enough for a 130 s rung.
        assert!(!fits_before_reserve(300.0, 300.0, 130.0));
    }

    #[test]
    fn attempts_stop_where_the_reserve_begins() {
        assert_eq!(deadline_before_reserve(700.0, 300.0, 1), 400.0);
        // Four seeds side by side: their work counts four times.
        assert_eq!(deadline_before_reserve(700.0, 300.0, 4), 100.0);
        assert_eq!(deadline_before_reserve(200.0, 300.0, 1), 0.0);
        assert_eq!(deadline_before_reserve(700.0, 300.0, 0), 400.0);
    }

    #[test]
    fn the_reserve_covers_the_steps_own_check() {
        // A layout's ladder, checks counting 9 s each: the step keeps its
        // routing's floor and its check on top; nothing kept, no check.
        let check = 9.0;
        assert_eq!(reserve_with_check(0.0, check), 0.0);
        let kept = reserve_with_check(narrow_step_reserve(200.0, 0.75, 600.0), check);
        assert_eq!(kept, NARROW_RESERVE_FLOOR + check);
        // Every rung stops where the reserve begins, less its own check:
        // its routing, its check and the reserve fit in what was left.
        let left = 700.0;
        let deadline = deadline_before_reserve(left - check, kept, 1);
        assert!((deadline + check + kept - left).abs() < 1.0e-9);
        // So with only the reserve left the step runs: its routing has the
        // reserve less its check, which is its minimum.
        let (routing_left, kept_routing) = (kept - check, kept - check);
        assert!(routing_left >= narrow_step_minimum(kept_routing));
        // An attempt whose routing fits but whose check does not, does not
        // start: 130 s of a rung and a 9 s check in 135 s beyond the reserve.
        assert!(fits_before_reserve(135.0 + kept, kept, 130.0));
        assert!(!fits_before_reserve(135.0 + kept - check, kept, 130.0));
        // Without a charge (route mode) the arithmetic is the old one.
        assert_eq!(reserve_with_check(300.0, 0.0), 300.0);
        assert_eq!(
            deadline_before_reserve(left - 0.0, 300.0, 1),
            deadline_before_reserve(left, 300.0, 1)
        );
    }

    #[test]
    fn the_minimum_connection_floors_classes_the_narrow_width_and_the_neck() {
        let mut config = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.09, 0.09)),
            ..KiCadBoardRouterConfig::default()
        };
        config
            .connection_rules
            .insert("/PWR".into(), rules(0.5, 0.2));
        config.narrow_signal_mm = Some(0.11);
        config.neck_width_mm = Some(0.1);
        assert!(narrow_step_applies(&config));
        config.min_connection_mm = Some(0.16);
        config.apply_width_floor();
        assert_eq!(config.default_rules.as_ref().unwrap().trace_width_mm, 0.16);
        assert_eq!(config.connection_rules["/PWR"].trace_width_mm, 0.5);
        assert_eq!(config.narrow_signal_mm, Some(0.16));
        assert_eq!(config.neck_width_mm, Some(0.16));
        // The power class is still wider: the narrow step routes the signals
        // at the floor.
        assert!(narrow_step_applies(&config));
        // No floor, nothing changes.
        let mut plain = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.09, 0.09)),
            ..KiCadBoardRouterConfig::default()
        };
        plain.apply_width_floor();
        assert_eq!(plain.default_rules.as_ref().unwrap().trace_width_mm, 0.09);
    }

    #[test]
    fn a_routed_board_has_no_track_narrower_than_the_minimum_connection() {
        // Two 0.25 mm pads side by side force a neck-down; class, narrow
        // signal and neck widths are all below the floor.
        let pcb = parse(
            r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 12 8) (layer "Edge.Cuts"))
              (footprint "a" (at 3 4)
                (pad "1" smd rect (at 0 0) (size 0.25 0.8) (layers "F.Cu") (net "/A"))
                (pad "2" smd rect (at 0.5 0) (size 0.25 0.8) (layers "F.Cu") (net "/B")))
              (footprint "b" (at 9 4)
                (pad "1" smd rect (at 0 0) (size 0.8 0.8) (layers "F.Cu") (net "/A"))
                (pad "2" smd rect (at 0 2) (size 0.8 0.8) (layers "F.Cu") (net "/B"))))"#,
        )
        .unwrap();
        let mut config = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.3, 0.1)),
            ..KiCadBoardRouterConfig::default()
        };
        config.narrow_signal_mm = Some(0.1);
        config.use_narrow_signals = Some(true);
        config.neck_width_mm = Some(0.1);
        config.min_connection_mm = Some(0.15);
        let board = lower(&pcb, &config, false).unwrap().board;
        assert_eq!(board.neck_width, 0.15);
        for net in &board.nets {
            assert_eq!(board.classes[net.class].trace_width, 0.15, "{}", net.name);
        }
        let result = core::route(&board, &core_config(&config));
        let widths: Vec<f64> = result
            .routes
            .iter()
            .flat_map(|route| route.segments.iter().map(|segment| segment.width))
            .collect();
        assert!(!widths.is_empty());
        assert!(
            widths.iter().all(|width| *width >= 0.15 - 1.0e-12),
            "{widths:?}"
        );
        // Without the floor the same board routes at the narrow width.
        config.min_connection_mm = None;
        let board = lower(&pcb, &config, false).unwrap().board;
        assert!(
            board
                .nets
                .iter()
                .all(|net| board.classes[net.class].trace_width == 0.1)
        );
    }

    #[test]
    fn the_step_config_narrows_signals_and_relaxes_clearance_once() {
        let mut config = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.2, 0.2)),
            ..KiCadBoardRouterConfig::default()
        };
        config
            .connection_rules
            .insert("/PWR".into(), rules(0.5, 0.25));
        config.narrow_signal_mm = Some(0.125);
        config.relaxed_clearance_mm = Some(0.1);
        let step = narrow_step_config(&config);
        assert_eq!(step.use_narrow_signals, Some(true));
        assert_eq!(step.relaxed_clearance_mm, None);
        assert_eq!(step.default_rules.as_ref().unwrap().clearance_mm, 0.1);
        assert_eq!(step.connection_rules["/PWR"].clearance_mm, 0.1);
        // The widths stay the classes' (lowering narrows the signals).
        assert_eq!(step.connection_rules["/PWR"].trace_width_mm, 0.5);
        assert!(!narrow_step_applies(&step));
        // Only a relaxed clearance: no narrow width to use.
        let relaxed = KiCadBoardRouterConfig {
            narrow_signal_mm: None,
            ..config.clone()
        };
        let step = narrow_step_config(&relaxed);
        assert_eq!(step.use_narrow_signals, Some(false));
        assert_eq!(step.default_rules.as_ref().unwrap().clearance_mm, 0.1);
        assert!(!narrow_step_applies(&step));
    }
}

mod check_gate_tests {
    use super::*;

    #[test]
    fn a_router_count_above_the_best_open_count_cannot_win() {
        // The full open count is the router's plus KiCad's: no slack.
        let best = ((10, 0), 100, 500.0);
        assert!(could_win(9, 200, 900.0, best));
        assert!(!could_win(11, 10, 100.0, best));
        // A tie on the count: starved thermals, then vias and copper decide.
        assert!(could_win(10, 99, 900.0, best));
        assert!(could_win(10, 100, 499.0, best));
        assert!(!could_win(10, 100, 500.0, best));
        assert!(!could_win(10, 101, 100.0, best));
        assert!(could_win(10, 101, 100.0, ((10, 2), 100, 500.0)));
    }

    #[test]
    fn the_first_attempt_and_router_clean_ones_are_always_checked() {
        let gate = CheckGate {
            top: Some(2),
            counts: vec![(3, 10, 1.0), (4, 10, 1.0)],
            ..CheckGate::default()
        };
        assert!(gate.wants(50, 0, 0.0, None, false));
        assert!(gate.wants(0, 500, 9000.0, Some(((0, 1), 10, 10.0)), false));
    }

    #[test]
    fn route_mode_checks_every_attempt_that_could_win() {
        let mut gate = CheckGate::default();
        let best = Some(((20, 0), 100, 500.0));
        for count in [5, 6, 7, 19] {
            assert!(gate.wants(count, 0, 0.0, best, false));
            gate.record((count, 0, 0.0));
        }
        assert!(!gate.wants(21, 0, 0.0, best, false));
        assert_eq!(gate.checked, 4);
    }

    #[test]
    fn the_final_ladder_checks_only_the_two_best_by_the_routers_measures() {
        // The layout's board (router count 12) is the first measure.
        let mut gate = CheckGate {
            top: Some(2),
            counts: vec![(12, 100, 500.0)],
            kept: Some((20, 0)),
            ..CheckGate::default()
        };
        let best = Some(((30, 1), 100, 500.0));
        // The ladder's first attempt: checked whatever its count.
        assert!(gate.wants(25, 100, 500.0, None, false));
        gate.record((25, 100, 500.0));
        // Better than one of the two: checked.
        assert!(gate.wants(14, 100, 500.0, best, false));
        gate.record((14, 100, 500.0));
        // Now 12 and 14 lead: 15 is third, 13 second.
        assert!(!gate.wants(15, 100, 500.0, best, false));
        assert!(gate.wants(13, 100, 500.0, best, false));
        gate.record((13, 100, 500.0));
        // An equal measure does not displace the two best; fewer vias at
        // the same count do (MokyaLora: the narrow step's 333 vias against
        // the ladder's 339 at one open each).
        assert!(!gate.wants(13, 100, 500.0, best, false));
        assert!(gate.wants(13, 90, 600.0, best, false));
        assert!(gate.wants(11, 100, 500.0, best, false));
    }

    #[test]
    fn past_the_deadline_only_a_board_that_beats_both_is_checked() {
        let gate = CheckGate {
            top: Some(2),
            counts: vec![(12, 100, 500.0)],
            kept: Some((20, 0)),
            ..CheckGate::default()
        };
        // No best yet: the layout's kept board alone decides.
        assert!(gate.wants(19, 0, 0.0, None, true));
        assert!(!gate.wants(20, 0, 0.0, None, true));
        assert!(!gate.wants(25, 0, 0.0, None, true));
        // With a best, both: the top-two rule no longer applies (nothing
        // follows), the could-win rules do.
        let best = Some(((18, 0), 100, 500.0));
        assert!(gate.wants(17, 0, 0.0, best, true));
        assert!(!gate.wants(19, 0, 0.0, best, true));
        // A kept board with starved thermals loses a tie to a clean one.
        let starved = CheckGate {
            kept: Some((20, 3)),
            ..CheckGate::default()
        };
        assert!(starved.wants(20, 0, 0.0, None, true));
    }
}

mod memory_limit_tests {
    use super::*;

    fn inputs() -> MemoryInputs {
        MemoryInputs {
            virtual_mb: 400.0,
            resident_mb: 100.0,
            total_mb: Some(64000.0),
            available_mb: Some(20000.0),
            ..MemoryInputs::default()
        }
    }

    #[test]
    fn without_a_limit_of_its_own_the_machines_available_memory_decides() {
        let limits = MemoryLimits::resolve(&inputs());
        assert_eq!(limits.limits.len(), 1);
        assert!(limits.limits[0].shared);
        assert_eq!(limits.hard_free(), None);
        assert_eq!(limits.machine_available(), Some(20000.0));
        // Half of it for routers side by side.
        assert_eq!(limits.limits[0].usable_mb(), 10000.0);
        assert!(limits.too_small_for(5000.0, 2).is_none());
    }

    #[test]
    fn the_address_space_limit_counts_the_virtual_size() {
        let limits = MemoryLimits::resolve(&MemoryInputs {
            address_space_mb: Some(3072.0),
            ..inputs()
        });
        let limit = &limits.limits[0];
        assert_eq!(limit.kind, MemoryKind::AddressSpace);
        assert_eq!(limit.free_mb, 3072.0 - 400.0);
        assert_eq!(limits.hard_free(), Some(2672.0));
        // A router of 400 MB of maps: (1.0 x 400 + 760) x 1.25 = 1450 MB
        // of address space, one fits beside the 400 held, not two.
        assert_eq!(limit.router_mb(400.0), 1450.0);
        let (count, by) = limits.routers_that_fit(400.0, 4);
        assert_eq!(count, 1);
        assert_eq!(by.unwrap().source, limit.source);
        // Never fewer than one, never more than wanted.
        assert_eq!(limits.routers_that_fit(5000.0, 4).0, 1);
        assert_eq!(limits.routers_that_fit(1.0, 2).0, 2);
        assert_eq!(limits.routers_that_fit(1.0, 8).0, 2);
        // Fixed decisions go by the limit, not by what is free.
        assert!(limits.too_small_for(400.0, 2).is_none());
        assert!(limits.too_small_for(400.0, 3).is_some());
        assert!(limits.too_small_for(2000.0, 1).is_some());
    }

    #[test]
    fn the_tightest_of_every_limit_decides() {
        let limits = MemoryLimits::resolve(&MemoryInputs {
            address_space_mb: Some(12288.0),
            cgroup_v2: vec![(8192.0, 1000.0), (4096.0, 1500.0)],
            cgroup_v1: Some((6000.0, 500.0)),
            environment_mb: Some(5000.0),
            setting_mb: Some(4500.0),
            ..inputs()
        });
        let sources: Vec<&str> = limits.limits.iter().map(|limit| limit.source).collect();
        assert_eq!(
            sources,
            [
                "the address space limit (RLIMIT_AS)",
                "the cgroup's memory.max",
                "the cgroup's memory.limit_in_bytes (v1)",
                "the router setting memory_mb",
                "the machine's available memory (MemAvailable)",
            ]
        );
        // The cgroup level with the least free holds (4096 - 1500).
        assert_eq!(limits.limits[1].limit_mb, 4096.0);
        assert_eq!(limits.limits[1].free_mb, 2596.0);
        // The explicit limit is the smaller of the setting and the
        // environment, against the resident size.
        assert_eq!(limits.limits[3].free_mb, 4400.0);
        assert_eq!(limits.hard_free(), Some(2596.0));
        // A router of 100 MB of maps: (1.0 x 100 + 600) x 1.25 = 875 MB
        // resident; two fit the cgroup's 2596 MB.
        let (count, by) = limits.routers_that_fit(100.0, 8);
        assert_eq!(count, 2);
        assert_eq!(by.unwrap().source, "the cgroup's memory.max");
        assert!(
            limits
                .describe()
                .starts_with("the cgroup's memory.max 4096 MB, 2596 MB free")
        );
    }

    #[test]
    fn the_environment_limits_when_it_is_the_smaller() {
        let limits = MemoryLimits::resolve(&MemoryInputs {
            environment_mb: Some(3072.0),
            setting_mb: Some(4096.0),
            ..inputs()
        });
        assert_eq!(limits.limits[0].source, "PCB_MAKER_MEMORY_MB");
        assert_eq!(limits.limits[0].free_mb, 2972.0);
        assert_eq!(limits.limits[0].kind, MemoryKind::Resident);
    }

    #[test]
    fn cgroup_paths_are_read_for_both_versions() {
        let (v2, v1) = cgroup_paths("0::/user.slice/run.scope\n");
        assert_eq!(v2.as_deref(), Some("/user.slice/run.scope"));
        assert_eq!(v1, None);
        let (v2, v1) = cgroup_paths("12:cpu,cpuacct:/a\n9:memory:/batch/job\n0::/\n");
        assert_eq!(v2.as_deref(), Some("/"));
        assert_eq!(v1.as_deref(), Some("/batch/job"));
    }

    #[test]
    fn the_maps_come_from_the_routers_memory_report() {
        assert_eq!(
            maps_mb_of("memory: 244 MB in maps (232 bytes a node; 1102 x 1001 nodes)"),
            Some(244.0)
        );
        assert_eq!(maps_mb_of("lattice: pitch 0.1 mm"), None);
        // The router's own report still reads (its format is pcb-router's).
        let pcb = parse(
            r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 12 8) (layer "Edge.Cuts"))
              (footprint "a" (at 3 4)
                (pad "1" smd rect (at 0 0) (size 0.8 0.8) (layers "F.Cu") (net "/A")))
              (footprint "b" (at 9 4)
                (pad "1" smd rect (at 0 0) (size 0.8 0.8) (layers "F.Cu") (net "/A"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let mut quiet = core_config(&config);
        quiet.verbose = false;
        let report = core::router::Router::new(&board, &quiet).memory_report();
        assert!(maps_mb_of(&report).is_some(), "{report}");
        let (result, maps) = route_measured(&board, &quiet);
        assert!(maps >= 0.0);
        assert_eq!(result.routes.len(), board.nets.len());
    }
}

mod work_bound_tests {
    use super::*;

    #[test]
    fn the_refinement_budget_grows_with_the_ladders_budget_and_a_layouts_work_left() {
        // Route mode: 240 s up to the default ladder budget of 1200 s, a
        // fifth of a larger one.
        assert_eq!(default_refine_budget(30.0, None), 240.0);
        assert_eq!(default_refine_budget(1200.0, None), 240.0);
        assert_eq!(default_refine_budget(3000.0, None), 600.0);
        // A layout's ladder: 0.4 of the work left (the fence's two layers:
        // 2241 s left, 896 s, past its 0.075 mm lattice's 367 s).
        assert_eq!(default_refine_budget(1200.0, Some(300.0)), 240.0);
        assert!((default_refine_budget(1200.0, Some(2241.0)) - 896.4).abs() < 1.0e-9);
    }

    #[test]
    fn the_bound_is_the_budget_and_its_tolerance() {
        assert!((ladder_work_bound(30.0) - 31.5).abs() < 1.0e-9);
        assert!((ladder_work_bound(300.0) - 315.0).abs() < 1.0e-9);
    }

    #[test]
    fn probes_take_at_most_half_the_budget() {
        // The intake's 4-layer boards: four probes of 75 s on 30 s.
        assert_eq!(bounded_probe_seconds(75.0, 30.0, 4), 3.75);
        assert_eq!(bounded_probe_seconds(75.0, 300.0, 4), 37.5);
        // The default budget keeps the probes' 75 s.
        assert_eq!(bounded_probe_seconds(75.0, 1200.0, 4), 75.0);
    }

    #[test]
    fn a_negotiation_takes_at_most_half_of_what_is_left() {
        // 30 s: a third of an even share would be 3.3 s and the floor
        // 60 s; half of what is left wins.
        assert_eq!(bounded_negotiation(30.0 / 3.0 / 3.0, 31.5), 15.75);
        // A large budget keeps the share within 60 to 900 s.
        assert_eq!(
            bounded_negotiation(1200.0 / 3.0 / 3.0, 1260.0),
            1200.0 / 9.0
        );
        assert_eq!(bounded_negotiation(10.0, 1260.0), 60.0);
        assert_eq!(bounded_negotiation(5000.0, 20000.0), 900.0);
        assert_eq!(bounded_negotiation(10.0, -5.0), 0.0);
    }

    #[test]
    fn rungs_after_the_first_share_what_is_left() {
        // Their work deadlines together stay within what is left, however
        // many run side by side.
        let left = 20.0;
        let rungs = 3;
        assert!(rung_share(left, rungs) * rungs as f64 <= left + 1.0e-9);
        assert_eq!(rung_share(left, 0), left);
        assert_eq!(rung_share(-1.0, 2), 0.0);
    }

    #[test]
    fn the_narrow_step_keeps_what_the_first_attempt_took() {
        // facade60: 20 s for the first attempt, 9 s left: all of it.
        assert_eq!(route_step_reserve(20.0, 9.1), 9.1);
        // akita: 0.6 s of 30.9.
        assert_eq!(route_step_reserve(0.6, 30.9), 0.6);
        assert_eq!(route_step_reserve(5.0, -1.0), 0.0);
        // The step runs with half of what was kept, never with nothing.
        assert!(route_step_has_time(0.3, 0.6, 30.0));
        assert!(!route_step_has_time(0.2, 0.6, 30.0));
        assert!(!route_step_has_time(0.0, 0.0, 30.0));
        // Without a reserve, as any attempt.
        assert!(route_step_has_time(1.5, 0.0, 30.0));
        assert!(!route_step_has_time(1.0, 0.0, 30.0));
        // megaduck: 3.9 s left, the first attempt's first iteration 17.5 s.
        assert!(!first_iteration_fits(3.9, 17.5));
        assert!(first_iteration_fits(3.9, 3.0));
        assert!(first_iteration_fits(3.9, 0.0));
        // A step's realistic cost: a fifth of the first attempt's work.
        assert!(!step_cost_fits(1.0, 16.3));
        assert!(step_cost_fits(7.9, 23.6));
        assert!(step_cost_fits(1.0, 0.0));
    }

    #[test]
    fn no_attempt_starts_with_less_than_a_twentieth_of_the_budget() {
        assert!(bound_allows_attempt(1.5, 30.0));
        assert!(!bound_allows_attempt(1.4, 30.0));
        assert!(!bound_allows_attempt(0.0, 30.0));
    }

    #[test]
    fn a_share_becomes_search_work_by_the_routers_search_share() {
        // Before any search, or with nothing beside it: the share as is.
        assert_eq!(search_fraction(0.0, 0.0), 1.0);
        assert_eq!(search_fraction(10.0, 10.0), 1.0);
        assert_eq!(search_fraction(10.0, 9.0), 1.0);
        // brickpico: 32.4 s of search, 15.7 s of pour work, 0.9 s of
        // lattices.
        assert!((search_fraction(32.4, 49.0) - 32.4 / 49.0).abs() < 1.0e-12);
        // Never below a quarter (Vortac's probes: 15 s of search beside
        // 50 s of pour work and lattices).
        assert_eq!(search_fraction(15.0, 65.0), MIN_SEARCH_FRACTION);
    }

    #[test]
    fn a_lowering_is_priced_by_the_board_files_expressions() {
        let pcb = crate::kicad_sexpr::parse(
            "(kicad_pcb (version 20240108) (net 0 \"\") (net 1 \"GND\"))",
        )
        .unwrap();
        // Every list and every atom once: the root and its head, the
        // version's list and two atoms, each net's list and three atoms.
        assert_eq!(expression_count(&pcb), 13);
        let work = lowering_work(&pcb);
        assert!(
            (work
                - 13.0 * LOWERING_EXPRESSION_NANOS / 1.0e9
                    * pcb_router::router::reference_time_scale())
            .abs()
                < 1.0e-15
        );
        assert_eq!(lowering_work_of(0), 0.0);
    }
}

mod lowering_gap_tests {
    use super::*;

    fn rules(clearance: f64) -> KiCadConnectionRoutingRules {
        KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: clearance,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }
    }

    /// A track of net `name` from `start` to `end` on F.Cu, 0.2 wide.
    fn track(
        board: &core::Board,
        name: &str,
        start: [f64; 2],
        end: [f64; 2],
    ) -> Vec<core::Violation> {
        let net = board.nets.iter().position(|net| net.name == name).unwrap();
        let mut routes = vec![core::NetRoute::default(); board.nets.len()];
        routes[net].segments.push(core::board::Segment {
            layer: 0,
            start,
            end,
            width: 0.2,
        });
        core::verify(board, &routes)
    }

    #[test]
    fn unplated_slots_and_margin_graphics_keep_the_edge_clearance() {
        // KiCad 10.0.7's copper_edge_clearance (checked with kicad-cli on
        // the same shapes): an NPTH slot is board edge, a round NPTH hole
        // and a plated slot are not; a Margin graphic is edge with its
        // stroke, on the board or in a footprint; a text on Margin is not.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (45 "Margin" user))
              (gr_rect (start 0 0) (end 40 40) (layer "Edge.Cuts"))
              (gr_line (start 30 5) (end 30 15) (stroke (width 0.2) (type default)) (layer "Margin"))
              (gr_text "M" (at 35 35 0) (layer "Margin") (effects (font (size 1 1) (thickness 0.15))))
              (footprint "h" (at 10 10) (property "Reference" "H1")
                (pad "" np_thru_hole circle (at 0 0) (size 2 2) (drill 2) (layers "*.Cu" "*.Mask")))
              (footprint "h" (at 10 25) (property "Reference" "H2")
                (pad "" np_thru_hole oval (at 0 0) (size 3 1.5) (drill oval 3 1.5) (layers "*.Cu" "*.Mask")))
              (footprint "h" (at 20 10) (property "Reference" "H3")
                (pad "" thru_hole oval (at 0 0) (size 3.4 1.9) (drill oval 3 1.5) (layers "*.Cu" "*.Mask")))
              (footprint "h" (at 20 25) (property "Reference" "H4")
                (fp_line (start 3 -3) (end 3 3) (stroke (width 0.1) (type default)) (layer "Margin")))
              (footprint "c" (at 5 35) (property "Reference" "C1")
                (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "A"))
                (pad "2" smd rect (at 1 0) (size 0.5 0.5) (layers "F.Cu") (net "A"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.1)),
            edge_clearance_mm: 0.5,
            hole_clearance_mm: 0.25,
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let find = |label: &str| {
            board
                .obstacles
                .iter()
                .filter(|obstacle| obstacle.label == label)
                .collect::<Vec<_>>()
        };
        assert_eq!(find("H1. hole")[0].clearance, 0.0, "a round hole");
        assert_eq!(find("H2. hole")[0].clearance, 0.5, "an unplated slot");
        assert_eq!(find("H3. plated hole")[0].clearance, 0.0, "a plated slot");
        let margin = find("board margin (gr_line)");
        assert_eq!(margin.len(), 1);
        assert_eq!(margin[0].kind, core::ObstacleKind::Keepout);
        assert_eq!(margin[0].layers, 0b11);
        assert_eq!(margin[0].clearance, 0.5);
        assert_eq!(find("H4 margin fp_line").len(), 1);
        assert!(
            !board
                .obstacles
                .iter()
                .any(|obstacle| obstacle.label.contains("gr_text")),
            "a text on Margin is no edge"
        );
        // KiCad's findings, 0.2 mm each: a track 0.2 from the slot, 0.2
        // from the Margin line's stroke, 0.15 from the footprint's.
        let near = |violations: Vec<core::Violation>, label: &str| {
            violations
                .iter()
                .find(|violation| violation.other == label)
                .map(|violation| (violation.required, violation.actual))
        };
        let (required, actual) =
            near(track(&board, "A", [5.0, 26.05], [15.0, 26.05]), "H2. hole").unwrap();
        assert!((required - 0.5).abs() < 1.0e-9 && (actual - 0.2).abs() < 1.0e-9);
        let (required, actual) = near(
            track(&board, "A", [30.4, 5.0], [30.4, 15.0]),
            "board margin (gr_line)",
        )
        .unwrap();
        assert!((required - 0.5).abs() < 1.0e-9 && (actual - 0.2).abs() < 1.0e-9);
        assert!(
            near(
                track(&board, "A", [23.3, 20.0], [23.3, 30.0]),
                "H4 margin fp_line"
            )
            .is_some()
        );
        // The round hole keeps the hole clearance only: 0.2 from it fails
        // that, 0.3 passes.
        assert!(near(track(&board, "A", [5.0, 11.3], [15.0, 11.3]), "H1. hole").is_some());
        assert!(near(track(&board, "A", [5.0, 11.4], [15.0, 11.4]), "H1. hole").is_none());
    }

    #[test]
    fn copper_of_no_net_keeps_the_default_class_and_footprint_layers_lists_count() {
        // tuxkeychain's GRAPHIC3: a footprint polygon on (layers "B.Cu"
        // "B.Mask") was neither copper nor an opening; KiCad keeps the
        // Default class's clearance from copper of no net, also against a
        // net of a smaller class (jnftech's Value texts on F.Cu too).
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (38 "B.Mask" user) (39 "F.Mask" user))
              (gr_rect (start 0 0) (end 40 40) (layer "Edge.Cuts"))
              (gr_line (start 5 30) (end 15 30) (stroke (width 0.2) (type default)) (layer "F.Cu"))
              (footprint "Graphics" (layer "B.Cu") (at 20 20 180) (property "Reference" "GRAPHIC3")
                (fp_poly (pts (xy -1 -1) (xy 1 -1) (xy 1 1) (xy -1 1)) (stroke (width 0) (type solid)) (fill yes) (layers "B.Cu" "B.Mask")))
              (footprint "tp" (layer "F.Cu") (at 10 10 90) (property "Reference" "TP1" (at 0 -2 90) (layer "F.SilkS"))
                (property "Value" "VDD" (at 0 -1.143 90) (layer "F.Cu") (effects (font (size 0.5 0.5) (thickness 0.1))))
                (property "Footprint" "x" (at 0 0 0) (layer "F.Cu") (hide yes) (effects (font (size 0.5 0.5) (thickness 0.1))))
                (fp_text user "MASK" (at 0 2 0) (layer "F.Mask") (effects (font (size 0.5 0.5) (thickness 0.1))))
                (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "SIG")))
              (footprint "c" (at 5 35) (property "Reference" "C1")
                (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "SIG"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.2)),
            connection_rules: [("SIG".to_string(), rules(0.1))].into_iter().collect(),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let find = |label: &str| {
            board
                .obstacles
                .iter()
                .filter(|obstacle| obstacle.label == label)
                .collect::<Vec<_>>()
        };
        let logo = find("GRAPHIC3 copper fp_poly");
        assert_eq!(logo.len(), 1, "the polygon's fill, no stroke");
        assert_eq!(
            (logo[0].layers, logo[0].net, logo[0].clearance),
            (0b10, None, 0.2)
        );
        assert!(
            board
                .obstacles
                .iter()
                .any(|obstacle| obstacle.label == "solder mask opening (fp_poly)"
                    && obstacle.layers == 0b10),
            "and an opening on B.Mask"
        );
        let line = find("copper gr_line");
        assert_eq!((line[0].net, line[0].clearance), (None, 0.2));
        // The Value on F.Cu: 0.5 mm high at 180 degrees, 1.143 left of the
        // pad (the footprint at 90 turns its (0, -1.143) to (-1.143, 0)).
        let value = find("TP1 copper text");
        assert_eq!(value.len(), 1, "the hidden Footprint field is no copper");
        let bounds = value[0].shape.aabb();
        let middle = [
            (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
            (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
        ];
        assert!(core::geometry::distance(middle, [10.0 - 1.143, 10.0]) < 1.0e-6);
        assert!(bounds.maximum[0] - bounds.minimum[0] > bounds.maximum[1] - bounds.minimum[1]);
        assert!(
            board
                .obstacles
                .iter()
                .any(|obstacle| obstacle.label == "solder mask opening (TP1 text)"),
            "a footprint text on F.Mask opens it"
        );
        // SIG's class keeps 0.1; against the line of no net, Default's 0.2.
        let found = track(&board, "SIG", [5.0, 30.35], [15.0, 30.35]);
        let finding = found
            .iter()
            .find(|violation| violation.other == "copper gr_line")
            .unwrap();
        assert!((finding.required - 0.2).abs() < 1.0e-9);
        assert!((finding.actual - 0.15).abs() < 1.0e-9);
    }

    /// The violation of a track of `name` against the obstacle `label`:
    /// (required, actual).
    fn against(
        board: &core::Board,
        name: &str,
        start: [f64; 2],
        end: [f64; 2],
        label: &str,
    ) -> Option<(f64, f64)> {
        track(board, name, start, end)
            .into_iter()
            .filter(|violation| violation.other == label)
            .map(|violation| (violation.required, violation.actual))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    #[test]
    fn dimensions_and_text_boxes_on_copper_are_copper_of_no_net() {
        // KiCad 10.0.7 on the same board (kicad-cli, Default 0.2 mm): an
        // aligned and an orthogonal dimension measured from their lines'
        // centres (zero width) and the text's glyphs; a text box from its
        // outline grown by half its stroke, border or not, and its glyphs.
        let pcb = parse(
            r#"(kicad_pcb (version 20241229) (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 60 40) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
              (dimension (type aligned) (layer "F.Cu")
                (pts (xy 10 10) (xy 30 10)) (height 5)
                (gr_text "20.0000 mm" (at 20 13 0) (layer "F.Cu") (effects (font (size 1 1) (thickness 0.15))))
                (format (prefix "") (suffix "") (units 3) (units_format 1) (precision 4))
                (style (thickness 0.2) (arrow_length 1.27) (text_position_mode 2) (arrow_direction outward) (extension_height 0.58642) (extension_offset 0.5) (keep_text_aligned yes)))
              (dimension (type orthogonal) (layer "F.Cu")
                (pts (xy 40 30) (xy 50 25)) (height -3) (orientation 0)
                (gr_text "10.0000 mm" (at 45 35 0) (layer "F.Cu") (effects (font (size 1 1) (thickness 0.15))))
                (format (prefix "") (suffix "") (units 3) (units_format 1) (precision 4))
                (style (thickness 0.2) (arrow_length 1.27) (text_position_mode 2) (arrow_direction outward) (extension_height 0.58642) (extension_offset 0.5) (keep_text_aligned yes)))
              (gr_text_box "Hi" (start 10 20) (end 20 24) (margins 0.5 0.5 0.5 0.5) (layer "F.Cu")
                (effects (font (size 1 1) (thickness 0.15)) (justify left top)) (border no) (stroke (width 0.2) (type solid)))
              (footprint "t" (layer "F.Cu") (at 2 38) (property "Reference" "J1")
                (fp_text_box "Yo" (start 30 -10) (end 36 -7) (margins 0.5 0.5 0.5 0.5) (layer "F.Cu")
                  (effects (font (size 1 1) (thickness 0.15)) (justify left top)) (border yes) (stroke (width 0.2) (type solid)))
                (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "A")))
              (footprint "t" (layer "F.Cu") (at 58 38) (property "Reference" "J2")
                (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "A"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.2)),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let dimension = |start, end| against(&board, "A", start, end, "copper dimension");
        let close = |found: Option<(f64, f64)>, actual: f64| {
            let (required, measured) = found.expect("a finding");
            assert!((required - 0.2).abs() < 1.0e-9);
            assert!(
                (measured - actual).abs() < 1.0e-3,
                "{measured} against {actual}"
            );
        };
        // KiCad's actual distances: the crossbar 0.19, an extension line's
        // start (its offset) 0.15, the extension line 0.05, an arrow
        // 0.0636; the orthogonal one's second extension 0.10, its first
        // 0.05.
        close(dimension([14.0, 15.29], [17.0, 15.29]), 0.19);
        close(dimension([9.0, 10.25], [11.0, 10.25]), 0.15);
        close(dimension([9.85, 11.0], [9.85, 13.0]), 0.05);
        close(dimension([28.5, 15.75], [29.3, 15.75]), 0.0636);
        close(dimension([50.2, 25.6], [50.2, 26.2]), 0.10);
        close(dimension([39.85, 28.0], [39.85, 29.0]), 0.05);
        // Its text: KiCad's glyphs 0.0798 away; the box is nearer.
        let (_, text) = dimension([18.0, 12.2], [22.0, 12.2]).unwrap();
        assert!(text <= 0.0798);
        // Beside the lines, as KiCad: no finding 0.21 from the crossbar.
        assert!(dimension([14.0, 15.31], [17.0, 15.31]).is_none());
        // The text box: 0.05 past its stroke, also without a border.
        close(
            against(&board, "A", [12.0, 24.25], [18.0, 24.25], "copper text box"),
            0.05,
        );
        // The footprint's text box, in the footprint's frame: at (32, 28).
        assert!(
            against(
                &board,
                "A",
                [33.0, 28.25],
                [37.0, 28.25],
                "J1 copper text box"
            )
            .is_some()
        );
        assert!(
            against(
                &board,
                "A",
                [33.0, 27.5],
                [37.0, 27.5],
                "J1 copper text box"
            )
            .is_none()
        );
    }

    #[test]
    fn a_padstacks_back_copper_a_bare_side_opening_and_a_plated_hole_without_layers() {
        // adrianmarple's Raspberry Pi header: 1.2 mm on F.Cu, 1.778 on
        // B.Cu (KiCad 9's front_inner_back); KiCad bridged and shorted
        // B.Cu tracks 0.4 mm from the drill. c-lydia's U3: copper on B.Cu,
        // its opening on F.Mask: an F.Cu track under it bridged.
        let pcb = parse(
            r#"(kicad_pcb (version 20241229) (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
              (gr_rect (start 0 0) (end 40 40) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
              (footprint "h" (at 10 10) (property "Reference" "A1")
                (pad "1" thru_hole circle (at 0 0) (size 1.2 1.2) (drill 1) (layers "*.Cu" "*.Mask") (net "P")
                  (padstack (mode front_inner_back) (layer "Inner" (shape circle) (size 1.778 1.778)) (layer "B.Cu" (shape circle) (size 1.778 1.778)))))
              (footprint "h" (at 30 10) (property "Reference" "A2")
                (pad "1" thru_hole circle (at 0 0) (size 1.2 1.2) (drill 1) (layers "*.Cu" "*.Mask") (net "P")))
              (footprint "u" (at 20 30) (property "Reference" "U3")
                (pad "7" smd roundrect (at 0 0) (size 1.9 0.4) (layers "F.Mask" "B.Cu" "F.Paste") (roundrect_rratio 0.25) (net "Q")))
              (footprint "u" (at 30 30) (property "Reference" "U4")
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu" "B.Mask") (net "Q")))
              (footprint "m" (at 20 20) (property "Reference" "U1")
                (pad "" thru_hole oval (at 0 0) (size 1.2 2) (drill 0.8) (layers "*.Mask")))
              (footprint "c" (at 5 35) (property "Reference" "C1")
                (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "A"))
                (pad "2" smd rect (at 1 0) (size 0.5 0.5) (layers "F.Cu") (net "A"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.2)),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        // aaronds's DevKitC pins: a plated hole listing only *.Mask is
        // copper on every layer (KiCad shorted tracks on F.Cu, In2.Cu and
        // B.Cu to one).
        let pin = board
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label == "U1." && obstacle.kind == core::ObstacleKind::Copper)
            .expect("the pin's copper");
        assert_eq!(pin.layers, 0b11);
        let p = board.nets.iter().position(|net| net.name == "P").unwrap() as core::NetId;
        let back = board
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label == "A1.1 padstack")
            .expect("the back's copper");
        assert_eq!(back.layers, 0b10, "B.Cu only (no inner layers here)");
        assert_eq!(back.net, Some(p));
        assert!((back.shape.aabb().maximum[0] - 10.889).abs() < 1.0e-9);
        assert!(
            !board
                .obstacles
                .iter()
                .any(|obstacle| obstacle.label == "A2.1 padstack"),
            "a pad without a padstack is one shape"
        );
        // U3's pad opens F.Mask over F.Cu, where it has no copper: an
        // aperture of its net there.
        let q = board.nets.iter().position(|net| net.name == "Q").unwrap() as core::NetId;
        let aperture = board
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label == "U3.7 mask aperture")
            .expect("the front's aperture");
        assert_eq!((aperture.layers, aperture.net), (0b01, Some(q)));
    }

    #[test]
    fn a_padstacks_front_shape_is_copper_only_where_the_stack_names_no_other() {
        // dennisss's lens mounts: 4 mm on F.Cu over 3 mm on B.Cu. The
        // front shape is F.Cu's alone; B.Cu has the 3 mm one, merged into
        // the pad's terminal. A B.Cu track ending 1.8 mm from the centre
        // (in the front's circle, off the back's) does not reach the pad;
        // on F.Cu it does, and on B.Cu at 1.4 mm.
        let pcb = parse(
            r#"(kicad_pcb (version 20241229) (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 40 40) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
              (footprint "h" (at 10 10) (property "Reference" "A1")
                (pad "1" thru_hole circle (at 0 0) (size 4 4) (drill 1) (layers "*.Cu" "*.Mask") (net "P")
                  (padstack (mode front_inner_back) (layer "Inner" (shape circle) (size 3 3)) (layer "B.Cu" (shape circle) (size 3 3)))))
              (footprint "h" (at 30 10) (property "Reference" "A2")
                (pad "1" thru_hole circle (at 0 0) (size 1.2 1.2) (drill 0.8) (layers "*.Cu" "*.Mask") (net "P"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.2)),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let find = |label: &str| {
            board
                .obstacles
                .iter()
                .position(|obstacle| {
                    obstacle.label == label && obstacle.kind == core::ObstacleKind::Copper
                })
                .unwrap()
        };
        let (front, back) = (find("A1.1"), find("A1.1 padstack"));
        assert_eq!(
            board.obstacles[front].layers, 0b01,
            "the front shape on F.Cu only"
        );
        assert!((board.obstacles[front].shape.aabb().maximum[0] - 12.0).abs() < 1.0e-9);
        assert_eq!(board.obstacles[back].layers, 0b10);
        assert!((board.obstacles[back].shape.aabb().maximum[0] - 11.5).abs() < 1.0e-9);
        let p = board.nets.iter().position(|net| net.name == "P").unwrap();
        let host = board.nets[p]
            .terminals
            .iter()
            .position(|terminal| terminal.pad == front)
            .unwrap();
        assert_eq!(board.nets[p].terminals[host].layers, 0b11);
        assert!(board.merged_pads.iter().any(|merged| merged.padstack
            && merged.pad == back
            && merged.host == host
            && merged.layers == 0b10));
        let joined = |layer: usize, end: f64| {
            let route = core::NetRoute {
                segments: vec![core::Segment {
                    layer,
                    start: [30.0, 10.0],
                    end: [end, 10.0],
                    width: 0.2,
                }],
                vias: Vec::new(),
            };
            let components =
                pcb_router::verify::copper_components(&board, p as core::NetId, &route);
            components
                .iter()
                .all(|component| *component == components[0])
        };
        assert!(!joined(1, 11.8), "phantom front copper on B.Cu");
        assert!(joined(0, 11.8));
        assert!(joined(1, 11.4));
    }

    #[test]
    fn a_blind_via_is_copper_on_its_own_layers_and_a_hidden_text_box_is_none() {
        // A blind via F.Cu to In1.Cu, a buried one In1.Cu to In2.Cu, a
        // through via; a footprint's hidden text box on F.Cu and a shown
        // one; a board text box of start and end turned 90 degrees keeps
        // its corners (KiCad turns its text only).
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "In2.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 40 40) (layer "Edge.Cuts"))
              (via blind (at 5 5) (size 0.6) (drill 0.3) (layers "F.Cu" "In1.Cu") (net "A"))
              (via buried (at 10 5) (size 0.6) (drill 0.3) (layers "In2.Cu" "In1.Cu") (net "A"))
              (via (at 15 5) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "A"))
              (gr_text_box "T" (start 20 20) (end 30 22) (angle 90) (layer "F.Cu") (effects (font (size 1 1))))
              (footprint "t" (at 20 10) (property "Reference" "T1")
                (fp_text_box "hidden" (start -1 -1) (end 1 1) (layer "F.Cu") (hide yes) (effects (font (size 1 1))))
                (fp_text_box "shown" (start 4 -1) (end 6 1) (layer "F.Cu") (effects (font (size 1 1))))
                (pad "1" smd rect (at 10 0) (size 1 1) (layers "F.Cu") (net "A")))
              (footprint "t" (at 35 35) (property "Reference" "T2")
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "A"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.2)),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let via_layers: Vec<core::LayerMask> = board
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.label == "existing via")
            .map(|obstacle| obstacle.layers)
            .collect();
        assert_eq!(via_layers, [0b0011, 0b0110, 0b1111]);
        let boxes: Vec<pcb_router::geometry::Aabb> = board
            .obstacles
            .iter()
            .filter(|obstacle| {
                obstacle.label.contains("text box")
                    && matches!(obstacle.shape, core::Shape::Polygon { .. })
            })
            .map(|obstacle| obstacle.shape.aabb())
            .collect();
        assert_eq!(boxes.len(), 2, "the shown footprint box and the board's");
        assert!(
            boxes
                .iter()
                .any(|bounds| (bounds.minimum[0] - 24.0).abs() < 1.0e-9
                    && (bounds.maximum[0] - 26.0).abs() < 1.0e-9)
        );
        assert!(
            boxes
                .iter()
                .any(|bounds| (bounds.minimum[0] - 20.0).abs() < 1.0e-9
                    && (bounds.maximum[0] - 30.0).abs() < 1.0e-9
                    && (bounds.maximum[1] - 22.0).abs() < 1.0e-9)
        );
    }

    #[test]
    fn a_cached_copper_text_is_a_box_per_line_and_a_footprint_graphics_net_no_net() {
        // TomKeddie's SAO sign: KiCad draws a TrueType copper text from the
        // installed font, its glyphs up to 0.92 mm off the cached ones;
        // each cached line is copper as a box, the lines' gaps stay open.
        // A footprint's polygon of net A is copper of no net to KiCad
        // 10.0.7 (clearance findings against its own pad and A's track).
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 40 40) (layer "Edge.Cuts"))
              (gr_text "ab\ncd" (at 10 10 0) (layer "F.Cu") (effects (font (face "X") (size 1 1) (thickness 0.1)))
                (render_cache "ab\ncd" 0
                  (polygon (pts (xy 8 9) (xy 9 9) (xy 9 10) (xy 8 10)))
                  (polygon (pts (xy 11 9) (xy 12 9) (xy 12 10) (xy 11 10)))
                  (polygon (pts (xy 8 11.6) (xy 12 11.6) (xy 12 12.6) (xy 8 12.6)))))
              (footprint "ant" (at 30 30) (property "Reference" "AE1")
                (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "A"))
                (fp_poly (pts (xy 0 -0.25) (xy 5 -0.25) (xy 5 0.25) (xy 0 0.25)) (stroke (width 0) (type solid)) (fill yes) (layer "F.Cu") (net "A")))
              (footprint "c" (at 5 35) (property "Reference" "C1")
                (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "A"))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules(0.1)),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let text = |start, end| against(&board, "A", start, end, "copper gr_text");
        // Between the glyphs of the first line: inside its box.
        assert!(text([10.0, 8.0], [10.0, 10.0]).is_some());
        // Between the lines (boxes to 10.1 and from 11.5): clear.
        assert!(text([5.0, 10.8], [15.0, 10.8]).is_none());
        let polygon = board
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label == "AE1 copper fp_poly")
            .unwrap();
        assert_eq!(polygon.net, None);
    }
}

mod terminal_tests {
    use super::*;

    fn rules() -> KiCadBoardRouterConfig {
        KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.25,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        }
    }

    /// A net tie between nets A and B (KiCad's NetTie-2_SMD_Pad0.5mm, the
    /// footprint of neotron-pico's NT901 to NT912) and a pad of each net.
    fn net_tie_board(version: u32, tie: &str) -> String {
        format!(
            r#"(kicad_pcb (version {version})
          (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
          (net 0 "") (net 1 "A") (net 2 "B")
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "NetTie:NetTie-2_SMD_Pad0.5mm" (layer "F.Cu") (at 10 5)
            (property "Reference" "NT1") {tie}
            (fp_poly (pts (xy -0.5 -0.25) (xy 0.5 -0.25) (xy 0.5 0.25) (xy -0.5 0.25))
              (stroke (width 0) (type solid)) (fill solid) (layer "F.Cu"))
            (pad "1" smd circle (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd circle (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 2 "B")))
          (footprint "R" (layer "F.Cu") (at 3 5) (property "Reference" "R1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
          (footprint "R" (layer "F.Cu") (at 17 5) (property "Reference" "R2")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 2 "B"))))"#
        )
    }

    fn tie_copper(board: &core::Board) -> Vec<&core::Obstacle> {
        board
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.label == "NT1 copper fp_poly")
            .collect()
    }

    #[test]
    fn a_net_ties_pads_are_reached_by_their_own_nets() {
        // kicad-cli 10.0.7 on this board with tracks R1-NT1.1 (A) and
        // R2-NT1.2 (B) ending on the pads' centres, over the bridge: no
        // unconnected item, no finding. Only A's track: NT1.2 and R2
        // unconnected (a tie joins no nets for KiCad's connectivity: each
        // pad is an item of its own net). An A track ending on the
        // bridge's middle: clearance against NT1.2 only. Without
        // net_tie_pad_groups: clearance and shorting_items against the
        // polygon, from both tracks and both pads.
        let config = rules();
        let tied = lower(
            &parse(&net_tie_board(20240108, r#"(net_tie_pad_groups "1, 2")"#)).unwrap(),
            &config,
            false,
        )
        .unwrap()
        .board;
        // The tie joins no nets: A and B keep their two terminals each.
        for name in ["A", "B"] {
            let net = tied.nets.iter().find(|net| net.name == name).unwrap();
            assert_eq!(net.terminals.len(), 2, "{name}");
        }
        // Its bridge is copper of no net, cut back where the pads' nets end
        // their tracks: 0.2 clearance, half of 0.25 and the margin from
        // each pad's centre.
        let reach = 0.2 + 0.125 + 0.1;
        for obstacle in tie_copper(&tied) {
            assert_eq!(obstacle.net, None);
            for center in [[9.5, 5.0], [10.5, 5.0]] {
                assert!(obstacle.shape.distance_to_point(center) >= reach - 1.0e-9);
            }
        }
        // What is left (the middle) still keeps other nets clear.
        assert!(
            tie_copper(&tied)
                .iter()
                .any(|obstacle| obstacle.shape.contains([10.0, 5.0]))
        );
        let mut quiet = core_config(&config);
        quiet.verbose = false;
        let (result, _) = route_measured(&tied, &quiet);
        for (index, net) in tied.nets.iter().enumerate() {
            assert!(
                matches!(result.status[index], core::NetStatus::Routed),
                "{}: {:?}",
                net.name,
                result.status[index]
            );
        }
        // Without the tie the polygon is copper of no net on both pads, as
        // before: whole, and the pads are dead behind it.
        let untied = lower(
            &parse(&net_tie_board(20240108, "")).unwrap(),
            &config,
            false,
        )
        .unwrap()
        .board;
        let copper = tie_copper(&untied);
        assert_eq!(copper.len(), 1);
        assert!(copper[0].shape.contains([9.6, 5.0]));
        let (result, _) = route_measured(&untied, &quiet);
        assert!(
            result
                .status
                .iter()
                .all(|status| !matches!(status, core::NetStatus::Routed))
        );
    }

    #[test]
    fn a_net_ties_bridge_keeps_every_via_out() {
        // brown-studios/pdlc's NT1: a via of EN, one of the tie's own
        // nets, 0.164 mm from the bridge's copper where the bridge is
        // carved back for EN's tracks; KiCad's hole_clearance (0.25)
        // holds against the bridge for every net. The whole bridge keeps
        // vias out; tracks of the tie's nets still reach their pads.
        let config = rules();
        let tied = lower(
            &parse(&net_tie_board(20240108, r#"(net_tie_pad_groups "1, 2")"#)).unwrap(),
            &config,
            false,
        )
        .unwrap()
        .board;
        let bridge: Vec<&core::Obstacle> = tied
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.label == "NT1 copper fp_poly (vias)")
            .collect();
        assert_eq!(bridge.len(), 1);
        assert_eq!(bridge[0].net, None);
        assert!(bridge[0].blocks_vias && !bridge[0].blocks_tracks);
        for point in [[9.6, 5.0], [10.0, 5.0], [10.4, 5.0]] {
            assert!(bridge[0].shape.contains(point), "{point:?}");
        }
        // On the lattice no via of A or B lies where its drill comes
        // within the hole clearance of the bridge.
        let grid = core::grid::Grid::choose(&tied, &[0.1]);
        let a = tied.nets.iter().position(|net| net.name == "A").unwrap();
        let class = tied.nets[a].class;
        let statics = core::grid::StaticMaps::build(&tied, &grid, class, true);
        let rules = tied.classes[class];
        let keep = rules.via_drill / 2.0 + tied.hole_clearance;
        for y in 0..grid.ny {
            for x in 0..grid.nx {
                let center = grid.center(x, y);
                if bridge[0].shape.distance_to_point(center) < keep {
                    for net in 0..tied.nets.len() as u32 {
                        assert!(
                            !statics.via_allowed(grid.index(x, y), net),
                            "{center:?} net {net}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_kicad_6_footprint_tagged_net_tie_is_a_tie() {
        // kicad-cli 10.0.7: `(tags "net tie")` without net_tie_pad_groups
        // makes a tie of all its pads on a board before version 20220817
        // (no finding with both tracks), not at 20220817 or after
        // (shorting_items and clearance against the polygon); "net tie
        // short" too, "Net Tie" and "foo net tie" not.
        let config = rules();
        let cut = |version: u32, tags: &str| {
            let board = lower(
                &parse(&net_tie_board(version, &format!("(tags {tags:?})"))).unwrap(),
                &config,
                false,
            )
            .unwrap()
            .board;
            !tie_copper(&board)
                .iter()
                .any(|obstacle| obstacle.shape.contains([9.6, 5.0]))
        };
        assert!(cut(20211014, "net tie"));
        assert!(cut(20220815, "net tie short"));
        assert!(!cut(20220817, "net tie"));
        assert!(!cut(20211014, "Net Tie"));
        assert!(!cut(20211014, "foo net tie"));
    }
}

mod solid_starved_tests {
    use super::super::route::solid_kept;
    use crate::verification::VerificationReport;

    fn report(unconnected: usize, findings: usize) -> VerificationReport {
        VerificationReport {
            board_id: "b".into(),
            complete: false,
            erc_violations: 0,
            drc_design_violations: findings,
            schematic_parity_issues: 0,
            selected_net_unconnected_items: unconnected,
            intentional_no_connect_groups: 0,
            library_metadata_warnings: 0,
            erc_library_warnings: Default::default(),
        }
    }

    #[test]
    fn starved_pads_stay_solid_only_when_kicad_finds_the_board_better() {
        // kicad-cli 10.0.7 on the v18 routes (x1433): mackerel-30-proto's
        // four island-starved pads solid, 134 findings to 130 and no
        // unconnected item either way: kept. k30-SBC's four starved pads
        // solid: 114 findings to 110, but J1.39 beside J1.14 unconnected
        // twice: not kept.
        assert!(solid_kept(Some(&report(0, 134)), &report(0, 130)));
        assert!(!solid_kept(Some(&report(0, 114)), &report(2, 110)));
        assert!(!solid_kept(Some(&report(0, 10)), &report(0, 10)));
        assert!(solid_kept(None, &report(3, 10)));
    }

    /// A panel of two copies of U3, each an exposed pad 45 with two
    /// thermal vias of its number; the second copy turned by 90 degrees.
    fn panel() -> crate::kicad_sexpr::Expr {
        let copy = |x: f64, angle: f64| {
            format!(
                r#"(footprint "QFN" (layer "F.Cu") (at {x} 10 {angle})
                  (property "Reference" "U3")
                  (pad "45" smd rect (at 0 0) (size 2 2) (layers "F.Cu"))
                  (pad "45" thru_hole circle (at 0.5 0.5) (size 0.5 0.5) (drill 0.3) (layers "*.Cu"))
                  (pad "45" thru_hole circle (at -0.5 0.5) (size 0.5 0.5) (drill 0.3) (layers "*.Cu") (zone_connect 1))
                  (pad "1" smd rect (at -1.5 0) (size 0.3 0.8) (layers "F.Cu")))"#
            )
        };
        crate::kicad_sexpr::parse(&format!(
            "(kicad_pcb {} {})",
            copy(10.0, 0.0),
            copy(30.0, 90.0)
        ))
        .unwrap()
    }

    /// Per footprint, its pads' numbers and zone connections (None: the
    /// pad's own is not set).
    fn connections(pcb: &crate::kicad_sexpr::Expr) -> Vec<Vec<(String, Option<String>)>> {
        pcb.children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .map(|footprint| {
                footprint
                    .children()
                    .iter()
                    .filter(|item| item.head() == Some("pad"))
                    .map(|pad| {
                        (
                            pad.children()[1].atom().unwrap().to_string(),
                            pad.child("zone_connect")
                                .and_then(|form| form.children().get(1))
                                .and_then(|value| value.atom())
                                .map(str::to_string),
                        )
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_solid_step_connects_only_the_starved_pads_of_a_label() {
        use super::super::route::{SolidPad, set_pads_solid};
        let solid = |reference: &str, number: &str, at: &[[f64; 2]]| SolidPad {
            reference: reference.into(),
            number: number.into(),
            at: at.to_vec(),
        };
        let some = |value: &str| Some(value.to_string());
        // The second copy's via at (-0.5, 0.5), turned by 90 degrees (Y
        // down: clockwise on the screen), lies at (30.5, 10.5): only it.
        let mut pcb = panel();
        let change = set_pads_solid(&mut pcb, &[solid("U3", "45", &[[30.5, 10.5]])]);
        assert_eq!(change.pads, 1);
        assert!(change.whole.is_empty());
        let pads = connections(&pcb);
        assert_eq!(
            pads[0],
            vec![
                ("45".into(), None),
                ("45".into(), None),
                ("45".into(), some("1")),
                ("1".into(), None)
            ]
        );
        assert_eq!(
            pads[1],
            vec![
                ("45".into(), None),
                ("45".into(), None),
                ("45".into(), some("2")),
                ("1".into(), None)
            ]
        );
        // Two points: the first copy's exposed pad and the second's
        // other via (0.5, 0.5 turned: 30.5, 9.5).
        let mut pcb = panel();
        let change = set_pads_solid(&mut pcb, &[solid("U3", "45", &[[10.0, 10.0], [30.5, 9.5]])]);
        assert_eq!(change.pads, 2);
        let pads = connections(&pcb);
        assert_eq!(pads[0][0].1, some("2"));
        assert_eq!(pads[0][1].1, None);
        assert_eq!(pads[1][1].1, some("2"));
        assert_eq!(pads[1][2].1, some("1"));
        // A point no pad of the label lies at, or no point at all: the
        // whole label, every copy, as before 2026-10-26.
        for at in [vec![[20.0, 20.0]], Vec::new()] {
            let mut pcb = panel();
            let change = set_pads_solid(&mut pcb, &[solid("U3", "45", &at)]);
            assert_eq!(change.pads, 6);
            assert_eq!(change.whole, vec!["U3.45".to_string()]);
            for footprint in connections(&pcb) {
                for (number, connection) in footprint {
                    assert_eq!(connection.is_some_and(|c| c == "2"), number == "45");
                }
            }
        }
        // Another label is left alone.
        let mut pcb = panel();
        assert_eq!(
            set_pads_solid(&mut pcb, &[solid("U4", "45", &[[10.0, 10.0]])]).pads,
            0
        );
    }

    #[test]
    fn kicads_findings_name_their_pads_by_label_and_point() {
        use super::super::route::{SolidPad, solid_pads_of_findings};
        let finding = |pad: &str, at: [f64; 2]| super::super::KiCadStarvedThermal {
            pad: pad.into(),
            layer: "F.Cu".into(),
            at,
            spokes: Some(1),
            needed: Some(2),
            advice: String::new(),
        };
        let pads = solid_pads_of_findings(&[
            finding("Pad 45 [GND] of U3 on F.Cu", [30.5, 10.5]),
            finding("Through hole pad 45 [GND] of U3", [10.5, 10.5]),
            finding("Pad 45 [GND] of U3 on F.Cu", [30.5, 10.5]),
            finding("Pad 2 [GND] of C8 on B.Cu", [1.0, 2.0]),
        ]);
        assert_eq!(
            pads,
            vec![
                SolidPad {
                    reference: "U3".into(),
                    number: "45".into(),
                    at: vec![[30.5, 10.5], [10.5, 10.5]],
                },
                SolidPad {
                    reference: "C8".into(),
                    number: "2".into(),
                    at: vec![[1.0, 2.0]],
                },
            ]
        );
    }
}

/// Hostile and odd boards (docs/reviews/2026-10-21-review-robustness.md).
mod robustness_tests {
    use super::*;

    fn config() -> KiCadBoardRouterConfig {
        KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.25,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        }
    }

    fn hostile(name: &str) -> Expr {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/hostile")
            .join(name);
        parse(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn thirty_two_copper_layers_are_all_of_a_mask() {
        // `(1 << 32) - 1` was 0 in a release build: every through-hole pad,
        // via and cutout on no layer.
        let pcb = hostile("32-layers.kicad_pcb");
        let layers = LayerTable::from_pcb(&pcb).unwrap();
        assert_eq!(layers.len(), 32);
        assert_eq!(layers.all(), u32::MAX);
        assert_eq!(layers.mask_of(["*.Cu"].into_iter()).unwrap(), u32::MAX);
        assert_eq!(layers.mask_of(["B.Cu"].into_iter()).unwrap(), 1 << 31);
        let board = lower(&pcb, &config(), true).unwrap().board;
        assert!(
            board
                .obstacles
                .iter()
                .any(|obstacle| obstacle.layers == u32::MAX)
        );
        let two = LayerTable::from_pcb(&hostile("unknown-layers.kicad_pcb")).unwrap();
        assert_eq!(two.all(), 0b11);
    }

    #[test]
    fn a_text_box_of_one_corner_on_a_mask_layer_is_left_out() {
        let pcb = hostile("text-box-one-corner.kicad_pcb");
        assert!(lower(&pcb, &config(), true).is_ok());
        assert_eq!(
            input_warnings(&pcb),
            ["J1's text box has fewer than three corners: it is left out"]
        );
    }

    #[test]
    fn copper_on_layers_the_stack_lacks_is_warned_of() {
        let warnings = input_warnings(&hostile("unknown-layers.kicad_pcb"));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("1 segment on In3.Cu"), "{warnings:?}");
        assert!(warnings[1].contains("1 via on In7.Cu (the first at 5.000, 15.000)"));
        // An ordinary board: none.
        let plated =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plated-branch.kicad_pcb");
        let pcb = parse(&fs::read_to_string(plated).unwrap()).unwrap();
        assert!(input_warnings(&pcb).is_empty());
    }

    #[test]
    fn a_lattice_past_the_memory_is_refused_by_the_lowering() {
        let pcb = hostile("huge-lattice.kicad_pcb");
        let config = KiCadBoardRouterConfig {
            memory_mb: Some(2000.0),
            ..config()
        };
        let error = lower(&pcb, &config, true).err().unwrap();
        assert!(error.contains("1900.0 x 1900.0 mm"), "{error}");
        assert!(error.contains("2000 MB"), "{error}");
        assert!(crate::verdict::is_input_error(&error));
        assert!(
            crate::verdict::noted_input_actions()
                .iter()
                .any(|action| action.contains("coarsest pitch of 0.127 mm"))
        );
        // The same board at a sane size lowers.
        let plated =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plated-branch.kicad_pcb");
        let pcb = parse(&fs::read_to_string(plated).unwrap()).unwrap();
        assert!(lower(&pcb, &config, true).is_ok());
    }
}

mod pour_rank_tests {
    use super::*;

    fn pour(pieces: usize, largest_share: f64) -> KiCadPourQuality {
        KiCadPourQuality {
            pieces,
            largest_share,
            ..Default::default()
        }
    }

    #[test]
    fn fewer_pieces_and_a_larger_main_piece_rank_first() {
        // The fence at 46 mm: 37 pieces, the largest 8 % of the pour.
        let shredded = pour_loss(&[pour(37, 0.08)]);
        let whole = pour_loss(&[pour(4, 0.6)]);
        assert!((shredded - (0.92 + 0.36)).abs() < 1e-9);
        assert!(pour_rank(Some(whole)) < pour_rank(Some(shredded)));
        // Within a step of each other the vias decide.
        assert_eq!(
            pour_rank(Some(pour_loss(&[pour(3, 0.70)]))),
            pour_rank(Some(pour_loss(&[pour(3, 0.705)])))
        );
        // Not measured: no rank (as before the pours ranked).
        assert_eq!(pour_rank(None), 0);
        assert_eq!(pour_loss(&[]), 0.0);
    }
}

#[test]
fn pin_row_gaps_pair_adjacent_smd_pads_of_a_row() {
    // An SOIC-8's two rows at 1.27 mm (5.4 mm apart): three gaps a row,
    // none across; a 2.54 mm row is no pin row; two pads are none.
    let mut soic = Vec::new();
    for row in 0..2 {
        for pin in 0..4 {
            soic.push((
                [pin as f64 * 1.27, row as f64 * 5.4],
                1,
                Some((row * 4 + pin) as pcb_router::NetId),
            ));
        }
    }
    let gaps = super::lower::footprint_pin_row_gaps(&soic);
    assert_eq!(gaps.len(), 6);
    assert!(
        gaps.iter()
            .all(|gap| gap.a[1] == gap.b[1] && gap.layer == 0)
    );
    let header: Vec<_> = (0..4)
        .map(|pin| ([pin as f64 * 2.54, 0.0], 1, None))
        .collect();
    assert!(super::lower::footprint_pin_row_gaps(&header).is_empty());
    assert!(super::lower::footprint_pin_row_gaps(&soic[..2]).is_empty());
}

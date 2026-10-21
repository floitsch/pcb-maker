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
        // A router of 300 MB of maps: (2 x 300 + 550) x 1.25 = 1437.5 MB of
        // address space, one fits beside the 400 held, not two.
        assert_eq!(limit.router_mb(300.0), 1437.5);
        let (count, by) = limits.routers_that_fit(300.0, 4);
        assert_eq!(count, 1);
        assert_eq!(by.unwrap().source, limit.source);
        // Never fewer than one, never more than wanted.
        assert_eq!(limits.routers_that_fit(5000.0, 4).0, 1);
        assert_eq!(limits.routers_that_fit(1.0, 2).0, 2);
        assert_eq!(limits.routers_that_fit(1.0, 8).0, 3);
        // Fixed decisions go by the limit, not by what is free.
        assert!(limits.too_small_for(300.0, 2).is_none());
        assert!(limits.too_small_for(300.0, 3).is_some());
        assert!(limits.too_small_for(1000.0, 1).is_some());
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
        // A router of 100 MB of maps: (1.7 x 100 + 460) x 1.25 = 787.5 MB
        // resident; three fit the cgroup's 2596 MB.
        let (count, by) = limits.routers_that_fit(100.0, 8);
        assert_eq!(count, 3);
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
    }

    #[test]
    fn no_attempt_starts_with_less_than_a_twentieth_of_the_budget() {
        assert!(bound_allows_attempt(1.5, 30.0));
        assert!(!bound_allows_attempt(1.4, 30.0));
        assert!(!bound_allows_attempt(0.0, 30.0));
    }
}

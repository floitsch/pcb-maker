// Copyright (C) 2026 Toit contributors.

use std::collections::{BTreeMap, BTreeSet};

use super::condition::*;
use super::lower::*;
use super::*;

#[test]
fn conditions_past_the_depth_limit_are_refused_and_chains_stay_shallow() {
    // `((((` and `!!!!` by the thousand overflowed the stack until
    // 2026-10-21.
    for text in [
        format!(
            "{}A.NetName == 'X'{}",
            "(".repeat(100_000),
            ")".repeat(100_000)
        ),
        format!("{}(A.NetName == 'X')", "!".repeat(100_000)),
        format!("{}1", "-".repeat(100_000)),
        format!(
            "A.insideArea({}'X'{})",
            "(".repeat(10_000),
            ")".repeat(10_000)
        ),
    ] {
        let error = parse_condition(&text).unwrap_err();
        assert!(error.contains("nests deeper than 200 levels"), "{error}");
    }
    // Within the limit, as before.
    let nested = format!("{}A.NetName == 'X'{}", "(".repeat(150), ")".repeat(150));
    assert!(parse_condition(&nested).is_ok());
    // Ten thousand alternatives: a tree of depth 15, the same truth as
    // read left to right.
    let chain = (0..10_000)
        .map(|index| format!("A.NetName == 'N{index}'"))
        .collect::<Vec<_>>()
        .join(" || ");
    let ex = parse_condition(&chain).unwrap();
    fn depth(ex: &Ex) -> usize {
        match ex {
            Ex::Or(a, b) | Ex::And(a, b) => 1 + depth(a).max(depth(b)),
            _ => 1,
        }
    }
    assert_eq!(depth(&ex), 15);
    let mut leaves = Vec::new();
    net_leaves(&ex, &mut leaves, &mut false);
    assert_eq!(leaves.len(), 10_000);
    // Three operands nest as before: `(a || b) || c`.
    assert_eq!(
        parse_condition("A.Type == 'via' || A.Type == 'pad' || A.Type == 'track'").unwrap(),
        Ex::Or(
            Box::new(parse_condition("A.Type == 'via' || A.Type == 'pad'").unwrap()),
            Box::new(parse_condition("A.Type == 'track'").unwrap())
        )
    );
    // A rule file nested past the reader's limit.
    let deep = format!("(version 1){}{}", "(".repeat(5_000), ")".repeat(5_000));
    assert!(sexps(&deep).unwrap_err().contains("nested deeper than"));
    // A length past f64's range is no length.
    assert_eq!(length_mm("1e999mm"), None);
    assert_eq!(length_mm("0.2mm"), Some(0.2));
}

fn nets(entries: &[(&str, &[&str])]) -> Vec<NetFacts> {
    entries
        .iter()
        .map(|(name, classes)| NetFacts {
            name: name.to_string(),
            classes: classes.iter().map(|class| class.to_string()).collect(),
            diff_pair: None,
        })
        .collect()
}

fn board(entries: &[(&str, &[&str])]) -> BoardFacts {
    BoardFacts {
        nets: nets(entries),
        copper_layers: vec!["F.Cu".into(), "B.Cu".into()],
        pad_kinds: [Kind::SmdPad, Kind::PlatedPad, Kind::NpthPad].into(),
        current: vec![
            crate::KiCadConnectionRoutingRules {
                trace_width_mm: 0.1,
                clearance_mm: 0.1,
                via_size_mm: 0.4,
                via_drill_mm: 0.2,
            };
            entries.len()
        ],
        board_clearances: [0.0; 3],
        neck_width: 0.0,
        connection_floor_warnings: false,
        areas: Vec::new(),
    }
}

fn lowered(text: &str, board: &BoardFacts) -> Lowered {
    lower(&parse_rules(text).unwrap(), board, &BTreeMap::new())
}

#[test]
fn rule_files_parse_with_comments_units_and_escapes() {
    let rules = parse_rules(
        r#"(version 1)
# a comment (with a parenthesis
(rule "Track width, outer"
	(layer outer)
	(condition "A.Type == 'track' && A.NetName == \"/USB_D+\"")
	(constraint track_width (min 0.127mm) (opt 8mil) (max 0.01in)))
(rule HV (severity warning)
  (constraint clearance (min 2)) (constraint disallow via track))"#,
    )
    .unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].name, "Track width, outer");
    assert_eq!(rules[0].layer.as_deref(), Some("outer"));
    assert_eq!(
        rules[0].condition.as_deref(),
        Some("A.Type == 'track' && A.NetName == \"/USB_D+\"")
    );
    let width = &rules[0].constraints[0];
    assert_eq!(width.kind, "track_width");
    assert_eq!(width.min, Some(0.127));
    assert!((width.opt.unwrap() - 0.2032).abs() < 1e-9);
    assert!((width.max.unwrap() - 0.254).abs() < 1e-9);
    assert_eq!(rules[1].severity.as_deref(), Some("warning"));
    assert_eq!(rules[1].constraints[0].min, Some(2.0));
    assert_eq!(rules[1].constraints[1].words, ["via", "track"]);
}

#[test]
fn a_pad_clearance_on_some_layers_floors_the_placer() {
    // OpenESC's 2 oz outer copper: 0.16 mm on the outer layers only, a
    // pair rule of the router that no net's clearance shows; the
    // placer packed pads 0.09 mm apart without it (170 clearance
    // errors on 4in1-mini).
    let mut board = board(&[
        ("A", &["Default"]),
        ("B", &["Default"]),
        ("GND", &["Default"]),
    ]);
    board.copper_layers = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
        .map(String::from)
        .to_vec();
    let rules = lowered(
        r#"(version 1)
(rule "silkscreen over pad" (constraint silk_clearance (min 0.15mm)) (condition "A.Type == 'Pad'"))
(rule "2 oz outer copper, clearance" (constraint clearance (min 0.16mm)) (layer outer))"#,
        &board,
    );
    assert_eq!(rules.clearance_all, 0.0);
    assert_eq!(rules.clearance, [0.0, 0.0, 0.0]);
    assert_eq!(rules.pad_clearance_all, 0.16);
    assert_eq!(rules.pair_clearances.len(), 1);
    assert_eq!(rules.pair_clearances[0].layers, 0b1001);
    // On every layer: a floor for every net.
    let rules = lowered("(rule all (constraint clearance (min 0.2mm)))", &board);
    assert_eq!(rules.pad_clearance_all, 0.2);
    // A rule from pads to tracks only, or on one class's nets, is no
    // floor between every two parts' pads.
    let rules = lowered(
        "(rule pad_track (condition \"A.Type == 'Pad' && B.Type == 'Track'\") (constraint clearance (min 0.2mm)))",
        &board,
    );
    assert_eq!(rules.pad_clearance_all, 0.0);
    let rules = lowered(
        "(rule hv (condition \"A.NetName == 'GND'\") (constraint clearance (min 1.5mm)))",
        &board,
    );
    assert_eq!(rules.pad_clearance_all, 0.0);
    // A later rule that takes the pads back (KiCad applies the last).
    let rules = lowered(
        "(rule outer (layer outer) (constraint clearance (min 0.3mm)))\n(rule pads (condition \"A.Type == 'Pad' && B.Type == 'Pad'\") (constraint clearance (min 0.1mm)))",
        &board,
    );
    assert_eq!(rules.pad_clearance_all, 0.1);
}

#[test]
fn a_net_class_clearance_raises_that_class_only() {
    // KiCad's netclass comparison is case-insensitive.
    let board = board(&[
        ("HV+", &["HIGH_V"]),
        ("HV-", &["HIGH_V"]),
        ("SDA", &["Default"]),
    ]);
    let rules = lowered(
        "(rule hv (condition \"A.NetClass == 'high_v'\") (constraint clearance (min 1.5mm)))",
        &board,
    );
    assert_eq!(rules.clearance, [1.5, 1.5, 0.0]);
    assert_eq!(rules.clearance_all, 0.0);
    assert_eq!(rules.report[0].status, "applied");
}

#[test]
fn a_rule_on_every_pair_raises_every_net() {
    let board = board(&[("A", &["Default"]), ("B", &["Default"])]);
    let rules = lowered(
        "(rule all (condition \"A.NetClass == 'default' && B.NetClass == 'Default'\") (constraint clearance (min 0.205mm)))",
        &board,
    );
    assert_eq!(rules.clearance_all, 0.205);
    assert_eq!(rules.report[0].status, "applied");
}

#[test]
fn a_rule_between_two_classes_is_a_pair_rule_unless_one_is_every_other_net() {
    let board = board(&[
        ("HV", &["HV"]),
        ("GND_PRE", &["PRE"]),
        ("SDA", &["Default"]),
        ("SCL", &["Default"]),
    ]);
    let rules = lowered(
        "(rule iso (condition \"A.NetClass == 'HV' && B.NetClass == 'PRE'\") (constraint clearance (min 4mm)))",
        &board,
    );
    // One clearance a net would keep 4 mm from every net: a pair rule
    // between the two sets instead, every item kind, every layer,
    // both ways round.
    assert_eq!(rules.report[0].status, "applied");
    assert!(
        rules.report[0]
            .detail
            .contains("between sets of nets, 2 nets")
    );
    assert_eq!(rules.clearance, [0.0; 4]);
    assert_eq!(rules.clearance_all, 0.0);
    assert!(rules.unmodelled.is_empty());
    assert_eq!(rules.pair_clearances.len(), 2);
    for pair in &rules.pair_clearances {
        assert_eq!(pair.kinds, 0b1111_1111);
        assert_eq!(pair.layers, 0b11);
        assert_eq!(pair.clearance, 4.0);
    }
    type Sides = (Option<Vec<usize>>, Option<Vec<usize>>);
    let sides: BTreeSet<Sides> = rules
        .pair_clearances
        .iter()
        .map(|pair| (pair.first.clone(), pair.second.clone()))
        .collect();
    assert_eq!(
        sides,
        [
            (Some(vec![0]), Some(vec![1])),
            (Some(vec![1]), Some(vec![0]))
        ]
        .into()
    );
    // Against everything else, a class's rule is the class's.
    let rules = lowered(
        "(rule iso (condition \"A.NetClass == 'HV' && B.NetClass != 'HV'\") (constraint clearance (min 4mm)))",
        &board,
    );
    assert_eq!(rules.clearance, [4.0, 0.0, 0.0, 0.0]);
    assert!(rules.pair_clearances.is_empty());
    // Looser than the classes: nothing to keep.
    let rules = lowered(
        "(rule iso (condition \"A.NetClass == 'HV' && B.NetClass == 'PRE'\") (constraint clearance (min 0.05mm)))",
        &board,
    );
    assert_eq!(rules.report[0].status, "no_effect");
    assert!(rules.pair_clearances.is_empty());
}

#[test]
fn net_names_match_with_wildcards_and_net_identity() {
    let board = board(&[
        ("/USB_D+", &["Default"]),
        ("/USB_D-", &["Default"]),
        ("GND", &["Default"]),
    ]);
    let rules = lowered(
        "(rule usb (condition \"A.NetName == '/USB*' && A.Net != B.Net\") (constraint clearance (min 0.3)))",
        &board,
    );
    assert_eq!(rules.clearance, [0.3, 0.3, 0.0]);
    // A rule between items of one net never binds a clearance.
    let same = lowered(
        "(rule same (condition \"A.Net == B.Net\") (constraint clearance (min 0.3)))",
        &board,
    );
    assert_eq!(same.report[0].status, "no_effect");
    assert_eq!(same.clearance_all, 0.0);
}

#[test]
fn item_types_and_layers_are_pair_rules() {
    let board = board(&[("A", &["Default"]), ("B", &["Default"])]);
    let rules = lowered(
            "(rule tp (layer outer) (condition \"A.Type == 'pad' && B.Type == 'track' && A.Net != B.Net\") (constraint clearance (min 0.2)))
             (rule pp (condition \"A.Type == 'Pad' && B.Type == 'Pad'\") (constraint clearance (min 0.5)))
             (rule in (layer inner) (constraint clearance (min 0.9)))",
            &board,
        );
    // Pad to track alone (JLCPCB's): a pair rule of tracks against
    // pads, on every net (no side named), not a clearance a net would
    // also hold between tracks.
    assert_eq!(rules.clearance_all, 0.0);
    assert_eq!(rules.report[0].status, "applied");
    assert_eq!(
        rules.pair_clearances,
        [PairClearance {
            first: None,
            second: None,
            kinds: 1 << 2 | 1 << 3,
            layers: 0b11,
            clearance: 0.2,
        }]
    );
    let tracks = lowered(
        "(rule tt (layer outer) (condition \"A.Type == 'track' && B.Type == A.Type\") (constraint clearance (min 0.2)))",
        &board,
    );
    assert_eq!(tracks.clearance_all, 0.0);
    assert_eq!(tracks.report[0].status, "applied");
    assert_eq!(tracks.pair_clearances[0].kinds, 1);
    // Pad to pad is the placement's, not the router's.
    assert_eq!(rules.report[1].status, "no_effect");
    // A two-layer board has no inner layer.
    assert_eq!(rules.report[2].status, "skipped");
    // On four layers: the inner layers only.
    let mut four = board.clone();
    four.copper_layers = vec![
        "F.Cu".into(),
        "In1.Cu".into(),
        "In2.Cu".into(),
        "B.Cu".into(),
    ];
    let inner = lowered(
        "(rule in (layer inner) (constraint clearance (min 0.9)))",
        &four,
    );
    assert_eq!(inner.clearance_all, 0.0);
    assert_eq!(inner.report[0].status, "applied");
    assert_eq!(inner.pair_clearances[0].layers, 0b0110);
    assert_eq!(inner.pair_clearances[0].kinds, 0b1111_1111);
    // An SMD pad rule holds for SMD pads only (until 2026-11-07 for
    // plated pads too: the router had one pad kind).
    let smd = lowered(
        "(rule smd (condition \"A.Pad_Type == 'SMD' && B.Type == 'track'\") (constraint clearance (min 0.3)))",
        &board,
    );
    assert_eq!(smd.report[0].status, "applied");
    assert_eq!(smd.pair_clearances[0].kinds, 1 << 2);
}

#[test]
fn a_rule_on_plated_pads_leaves_smd_pads_alone() {
    // The laptop motherboard's "Pad to Track clearance": KiCad's
    // `isPlated()` holds for plated pads and vias, never for an SMD
    // pad, so its 0.4 mm WLCSP balls keep the class's 0.11 mm.
    let board = board(&[("A", &["Default"]), ("B", &["Default"])]);
    let rules = lowered(
        "(rule \"Pad to Track\" (constraint clearance (min 0.2mm)) (condition \"A.isPlated() && B.Type == 'track'\"))",
        &board,
    );
    assert_eq!(rules.report[0].status, "applied");
    assert!(!rules.report[0].detail.contains("alike"));
    assert_eq!(rules.pair_clearances.len(), 1);
    // Tracks against plated pads and against vias, vias against
    // tracks: not against SMD pads.
    use pcb_router::rules::{PAD, PLATED, TRACK, VIA, kind_bit};
    let kinds = rules.pair_clearances[0].kinds;
    assert_eq!(
        kinds,
        kind_bit(TRACK, PLATED) | kind_bit(TRACK, VIA) | kind_bit(VIA, TRACK)
    );
    assert_eq!(kinds & kind_bit(TRACK, PAD), 0);
}

#[test]
fn a_later_rule_takes_the_pairs_it_surely_matches() {
    let board = board(&[("A", &["Default"]), ("B", &["Default"]), ("HV", &["HV"])]);
    // JLCPCB's template: "Pad to Track" 0.2, then 0.127 on everything,
    // which KiCad applies (the last matching rule).
    let rules = lowered(
            "(rule pt (condition \"A.Type == 'Pad' && B.Type == 'Track'\") (constraint clearance (min 0.2)))
             (rule all (constraint clearance (min 0.127)))",
            &board,
        );
    assert_eq!(rules.report[0].status, "no_effect");
    assert!(rules.report[0].detail.contains("later rule"));
    assert!(rules.pair_clearances.is_empty());
    // The other way round, the pad rule is the last one for pads.
    let rules = lowered(
            "(rule all (constraint clearance (min 0.127)))
             (rule pt (condition \"A.Type == 'Pad' && B.Type == 'Track'\") (constraint clearance (min 0.2)))",
            &board,
        );
    assert_eq!(rules.report[1].status, "applied");
    // (The general rule then holds for every pair but tracks to pads.)
    assert_eq!(rules.pair_clearances.len(), 2);
    assert!(
        rules
            .pair_clearances
            .iter()
            .any(|pair| pair.clearance == 0.2 && pair.kinds == 1 << 2 | 1 << 3)
    );
    assert!(
        rules
            .pair_clearances
            .iter()
            .any(|pair| pair.clearance == 0.127 && pair.kinds == 0b1111_1111 & !(1 << 2 | 1 << 3))
    );
    // A later rule on some nets takes their pairs only: the general
    // 0.3 holds between A and B, not with HV.
    let rules = lowered(
        "(rule all (constraint clearance (min 0.3)))
             (rule hv (condition \"A.NetClass == 'HV'\") (constraint clearance (min 0.05)))",
        &board,
    );
    assert_eq!(rules.clearance_all, 0.0);
    assert_eq!(rules.report[0].status, "applied");
    let pair = &rules.pair_clearances[0];
    assert_eq!(pair.clearance, 0.3);
    assert_eq!(pair.first, Some(vec![0, 1]));
    assert_eq!(pair.second, Some(vec![0, 1]));
}

#[test]
fn a_clearance_inside_a_rule_area_holds_on_the_areas_layers() {
    // MokyaLora: 0.25 mm between tracks that both meet the rule area
    // TFT_ZONE (In3.Cu only) over classes of 0.1. Not lowered, the
    // router laid tracks 0.13 mm apart there (kicad-cli: four
    // clearance errors on In3.Cu, none on the other layers inside the
    // outline). Kept between every two tracks on the area's layer; a
    // missing area matches nothing; a rule that lowers the clearance
    // in an area changes nothing; an area beside an unknown of
    // another kind stays unmodelled.
    let mut board = board(&[("KEY_R5", &["Default"]), ("GPIO11", &["Default"])]);
    board.copper_layers = vec!["F.Cu".into(), "In3.Cu".into(), "B.Cu".into()];
    board.areas = vec![
        NamedArea {
            name: "TFT_ZONE".into(),
            uuid: "ae3cb847".into(),
            layers: vec!["In3.Cu".into()],
        },
        NamedArea {
            name: "MCU_ZONE".into(),
            uuid: "4a298381".into(),
            layers: vec!["B.Cu".into()],
        },
    ];
    let rules = lowered(
            "(rule Shrink_Clearance_In_MCU (constraint clearance (min 0.085mm))
               (condition \"A.intersectsArea('MCU_ZONE')\"))
             (rule TFT_ZONE_2W_Clearance (constraint clearance (min 0.25mm))
               (condition \"A.Type == 'Track' && B.Type == 'Track' && A.intersectsArea('TFT_ZONE') && B.intersectsArea('TFT_ZONE')\"))
             (rule gone (constraint clearance (min 1mm)) (condition \"A.intersectsArea('NOWHERE')\"))
             (rule mixed (constraint clearance (min 1mm))
               (condition \"A.intersectsArea('TFT_ZONE') && A.insideCourtyard('U1')\"))",
            &board,
        );
    let statuses: Vec<&str> = rules.report.iter().map(|r| r.status.as_str()).collect();
    assert_eq!(
        statuses,
        ["no_effect", "approximated", "no_effect", "unmodelled"]
    );
    assert_eq!(rules.clearance, [0.0, 0.0]);
    assert_eq!(rules.clearance_all, 0.0);
    assert_eq!(rules.pair_clearances.len(), 1);
    let pair = &rules.pair_clearances[0];
    assert_eq!(pair.clearance, 0.25);
    // Tracks against tracks (bit 0), on In3.Cu (index 1) alone.
    assert_eq!(pair.kinds, 1);
    assert_eq!(pair.layers, 0b010);
    assert!(
        rules.report[1].detail.contains("In3.Cu"),
        "{}",
        rules.report[1].detail
    );
    // By uuid, and an area on every copper layer.
    board.areas[0].layers = board.copper_layers.clone();
    let rules = lowered(
            "(rule tft (constraint clearance (min 0.25mm))
               (condition \"A.Type == 'Track' && B.Type == 'Track' && A.intersectsArea('ae3cb847') && B.intersectsArea('ae3cb847')\"))",
            &board,
        );
    assert_eq!(rules.pair_clearances.len(), 1);
    assert_eq!(rules.pair_clearances[0].layers, 0b111);
    assert_eq!(rules.report[0].status, "approximated");
}

#[test]
fn conditions_on_areas_or_parts_are_unmodelled_and_named() {
    let board = board(&[("L1", &["Default"]), ("FB", &["Default"])]);
    let rules = lowered(
            "(rule keep (condition \"A.insideCourtyard('L1') && B.Type == 'track'\") (constraint clearance (min 20mm)))
             (rule creep (constraint creepage (min 4mm)))
             (rule len (condition \"A.NetClass == 'DDR'\") (constraint length (min 10mm) (max 20mm)))",
            &board,
        );
    assert_eq!(rules.clearance_all, 0.0);
    assert_eq!(rules.clearance, [0.0, 0.0]);
    let statuses: Vec<&str> = rules.report.iter().map(|r| r.status.as_str()).collect();
    assert_eq!(statuses, ["unmodelled", "unmodelled", "unmodelled"]);
    assert!(rules.report[0].detail.contains("A.insidecourtyard()"));
    assert_eq!(rules.unmodelled.len(), 3);
    assert!(rules.unmodelled[1].starts_with("creepage ('creep', min 4 mm)"));
}

/// A minimum the router keeps more than anyway (at or below every
/// net's clearance, or the board setup's hole clearance) costs nothing
/// whatever its condition: ducktop2 right_io's intra-footprint rules
/// on `A.Reference` (trial 5) were reported as rules the router does
/// not keep. Above them a condition the router cannot read stays
/// unmodelled.
#[test]
fn a_minimum_below_what_the_router_keeps_has_no_effect_whatever_its_condition() {
    let mut board = board(&[("L1", &["Default"]), ("FB", &["Default"])]);
    board.board_clearances = [0.0, 0.25, 0.0];
    let rules = lowered(
            "(rule own (condition \"A.Reference == 'U41' && B.Reference == 'U41'\") (constraint clearance (min 0.1mm)))
             (rule raise (condition \"A.Reference == 'U41' && B.Reference == 'U41'\") (constraint clearance (min 0.12mm)))
             (rule port (condition \"A.memberOfFootprint('MK430') && B.memberOfFootprint('MK430')\") (constraint hole_clearance (min 0.15mm)))",
            &board,
        );
    let statuses: Vec<&str> = rules.report.iter().map(|r| r.status.as_str()).collect();
    assert_eq!(statuses, ["no_effect", "unmodelled", "no_effect"]);
    assert!(
        rules.report[0]
            .detail
            .contains("at or below every net's clearance"),
        "{}",
        rules.report[0].detail
    );
    assert!(
        rules.report[2]
            .detail
            .contains("at or below the board setup's 0.25 mm"),
        "{}",
        rules.report[2].detail
    );
    assert_eq!(rules.unmodelled.len(), 1);
}

#[test]
fn widths_vias_and_holes_lower_per_net_or_for_the_board() {
    let board = board(&[("VBUS", &["Power"]), ("SDA", &["Default"])]);
    let rules = lowered(
            "(rule pw (condition \"A.NetClass == 'Power'\") (constraint track_width (min 0.5mm) (opt 0.8mm)))
             (rule all (condition \"A.Type == 'track'\") (constraint track_width (min 0.15mm)))
             (rule via (condition \"A.Type == 'via'\") (constraint via_diameter (min 0.6)) (constraint hole_size (min 0.3) (max 6.3)))
             (rule ring (condition \"A.isPlated()\") (constraint annular_width (min 0.15)))
             (rule pads (condition \"A.Type == 'pad' && !A.isPlated()\") (constraint hole_size (min 0.5)))
             (rule edge (condition \"A.Type == 'track'\") (constraint edge_clearance (min 0.3mm)))
             (rule npth (condition \"!A.isPlated() && B.Type == 'track' && A.Net != B.Net\") (constraint hole_clearance (min 0.254)))
             (rule h2h (condition \"A.Net != B.Net\") (constraint hole_to_hole (min 0.5)))",
            &board,
        );
    assert_eq!(rules.width_min, [0.5, 0.15]);
    assert_eq!(rules.width_min_all, 0.15);
    assert_eq!(rules.width_opt, [Some(0.8), None]);
    assert_eq!(rules.via_diameter_min_all, 0.6);
    assert_eq!(rules.via_drill_min_all, 0.3);
    assert_eq!(rules.via_drill_max, [6.3, 6.3]);
    assert_eq!(rules.annulus_min_all, 0.15);
    assert_eq!(rules.edge_clearance, 0.3);
    assert_eq!(rules.hole_clearance, 0.254);
    // hole_to_hole is a warning by KiCad's default.
    assert_eq!(rules.hole_to_hole, 0.0);
    let status: BTreeMap<&str, &str> = rules
        .report
        .iter()
        .map(|r| (r.rule.as_str(), r.status.as_str()))
        .collect();
    assert_eq!(status["pads"], "no_effect");
    assert_eq!(status["h2h"], "skipped");
    let severities = BTreeMap::from([("hole_to_hole".to_string(), "error".to_string())]);
    let strict = lower(
        &parse_rules(
            "(rule h2h (condition \"A.Net != B.Net\") (constraint hole_to_hole (min 0.5)))",
        )
        .unwrap(),
        &board,
        &severities,
    );
    assert_eq!(strict.hole_to_hole, 0.5);
}

#[test]
fn track_widths_on_some_layers_lower_per_layer() {
    // OpenESC's "2 oz outer copper": 0.16 mm on F.Cu and B.Cu over
    // classes of 0.1, kept per layer (until 2026-10-24 on every layer,
    // `approximated`).
    let mut board = board(&[("A", &["Default"]), ("D+", &["USB"])]);
    board.copper_layers = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
        .map(String::from)
        .to_vec();
    let rules = lowered(
        "(rule \"2 oz outer copper, track width\" (constraint track_width (min 0.16mm)) (layer outer))",
        &board,
    );
    assert_eq!(rules.width_min, [0.0, 0.0]);
    assert_eq!(rules.width_min_all, 0.0);
    assert_eq!(
        rules.layer_width_min,
        [[0.16, 0.16], [0.0, 0.0], [0.0, 0.0], [0.16, 0.16]]
    );
    assert_eq!(rules.report[0].status, "applied");
    assert!(
        rules.report[0].detail.contains("on F.Cu, B.Cu (per layer)"),
        "{}",
        rules.report[0].detail
    );
    // A layer condition (Sisu's USB geometry on two inner layers),
    // with an opt and a max.
    let rules = lowered(
        "(rule usb (condition \"A.hasNetclass('USB') && (A.Layer == 'In1.Cu' || A.Layer == 'In2.Cu')\") (constraint track_width (min 0.1mm) (opt 0.1mm) (max 0.1mm)))",
        &board,
    );
    assert_eq!(rules.width_opt, [None, None]);
    assert_eq!(
        rules.layer_width_opt,
        [
            [None, None],
            [None, Some(0.1)],
            [None, Some(0.1)],
            [None, None]
        ]
    );
    assert_eq!(rules.layer_width_max[1], [f64::INFINITY, 0.1]);
    assert_eq!(rules.report[0].status, "applied");
    // A layer rule the classes keep (tuxkeychain's 0.127 mm outside
    // over classes and a neck of 0.13) changes nothing.
    let mut kept = board.clone();
    kept.neck_width = 0.13;
    kept.current
        .iter_mut()
        .for_each(|rules| rules.trace_width_mm = 0.2);
    let rules = lowered(
        "(rule outer (constraint track_width (min 0.127mm)) (layer outer))",
        &kept,
    );
    assert_eq!(rules.report[0].status, "no_effect");
    assert!(rules.layer_width_min.is_empty());
    // A layer clause that names every layer of the board is a rule on
    // every layer: per net, as before.
    let two = self::board(&[("A", &["Default"])]);
    let rules = lowered(
        "(rule outer (constraint track_width (min 0.16mm)) (layer outer))",
        &two,
    );
    assert!(rules.layer_width_min.is_empty());
    assert_eq!(rules.width_min_all, 0.16);
}

#[test]
fn a_connection_width_rule_floors_the_track_width() {
    // A custom connection width (KiCad checks the union of a net's
    // copper on a layer; the router's part is its tracks' width).
    let mut board = board(&[("A", &["Default"]), ("PWR", &["Power"])]);
    board.copper_layers = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
        .map(String::from)
        .to_vec();
    let errors: BTreeMap<String, String> =
        [("connection_width".to_string(), "error".to_string())].into();
    let lower_with = |text: &str, board: &BoardFacts, severities: &BTreeMap<String, String>| {
        lower(&parse_rules(text).unwrap(), board, severities)
    };
    // On the inner layers only: per layer, the necks with it.
    let inner =
        "(rule \"inner connections\" (layer inner) (constraint connection_width (min 0.15mm)))";
    let rules = lower_with(inner, &board, &errors);
    assert_eq!(
        rules.layer_width_min,
        [[0.0, 0.0], [0.15, 0.15], [0.15, 0.15], [0.0, 0.0]]
    );
    assert_eq!(rules.width_min_all, 0.0);
    assert_eq!(rules.report[0].status, "applied");
    assert!(
        rules.report[0]
            .detail
            .contains("connection width at least 0.15 mm"),
        "{}",
        rules.report[0].detail
    );
    assert!(rules.unmodelled.is_empty());
    // On every layer, for one class: per net.
    let power = "(rule power (condition \"A.NetClass == 'Power'\") (constraint connection_width (min 0.3mm)))";
    let rules = lower_with(power, &board, &errors);
    assert!(rules.layer_width_min.is_empty());
    assert_eq!(rules.width_min, [0.0, 0.3]);
    assert_eq!(rules.report[0].status, "applied");
    // KiCad's default severity is a warning: a floor only where the
    // mode floors warnings.
    let rules = lower_with(inner, &board, &BTreeMap::new());
    assert_eq!(rules.report[0].status, "skipped");
    assert!(rules.layer_width_min.is_empty());
    let mut floors = board.clone();
    floors.connection_floor_warnings = true;
    let rules = lower_with(inner, &floors, &BTreeMap::new());
    assert_eq!(rules.report[0].status, "applied");
    assert_eq!(rules.layer_width_min[1], [0.15, 0.15]);
    // A rule's own warning alike.
    let own = "(rule own (severity warning) (constraint connection_width (min 0.15mm)))";
    assert_eq!(lower_with(own, &board, &errors).report[0].status, "skipped");
    let rules = lower_with(own, &floors, &errors);
    assert_eq!(rules.report[0].status, "applied");
    assert_eq!(rules.width_min_all, 0.15);
    // The classes and the necks keep it: no effect.
    let mut kept = board.clone();
    kept.neck_width = 0.1;
    let rules = lower_with(
        "(rule narrow (constraint connection_width (min 0.05mm)))",
        &kept,
        &errors,
    );
    assert_eq!(rules.report[0].status, "no_effect");
}

#[test]
fn a_rule_the_classes_already_keep_has_no_effect() {
    let board = board(&[("A", &["Default"]), ("B", &["Default"])]);
    let rules = lowered(
            "(rule loose (condition \"A.Type == 'track' && B.Type == A.Type\") (constraint clearance (min 0.09mm)))
             (rule narrow (constraint track_width (min 0.09mm)))",
            &board,
        );
    assert_eq!(rules.report[0].status, "no_effect");
    // A board-wide minimum width still floors the neck-downs.
    assert_eq!(rules.report[1].status, "applied");
    assert_eq!(rules.width_min_all, 0.09);
}

#[test]
fn not_routing_rules_and_through_via_disallows_need_nothing() {
    let board = board(&[("A", &["Default"])]);
    let rules = lowered(
        "(rule silk (constraint silk_clearance (min 0.15)))
             (rule text (constraint text_height (min 1mm)))
             (rule vias (constraint disallow buried_via micro_via))
             (rule zone (constraint thermal_spoke_width (min 0.3)))",
        &board,
    );
    let statuses: Vec<&str> = rules.report.iter().map(|r| r.status.as_str()).collect();
    assert_eq!(
        statuses,
        ["not_routing", "not_routing", "no_effect", "not_routing"]
    );
    assert!(rules.unmodelled.is_empty());
}

#[test]
fn conditions_are_evaluated_both_ways_and_with_kleene_logic() {
    let ex = parse_condition("A.Type == 'pad' && B.Type == 'track'").unwrap();
    let net = NetFacts {
        name: "N".into(),
        classes: vec!["Default".into()],
        diff_pair: None,
    };
    let track = Item {
        kind: Kind::Track,
        net: Some(&net),
        layer: "F.Cu",
    };
    let pad = Item {
        kind: Kind::SmdPad,
        net: Some(&net),
        layer: "F.Cu",
    };
    assert_eq!(pair_truth(&ex, track, Some(pad), false), T3::True);
    assert_eq!(pair_truth(&ex, track, Some(track), false), T3::False);
    let unknown = parse_condition("A.Type == 'via' || A.insideArea('X')").unwrap();
    assert_eq!(pair_truth(&unknown, track, None, false), T3::Unknown);
    let decided = parse_condition("A.Type == 'pad' && A.insideArea('X')").unwrap();
    assert_eq!(pair_truth(&decided, track, None, false), T3::False);
    assert_eq!(
        parse_condition("A.Pad_Type == 'NPTH, mechanical' && B.Type == 'track'").unwrap(),
        Ex::And(
            Box::new(Ex::Compare(
                Compare::Equal,
                Box::new(Ex::Property(Side::A, "padtype".into())),
                Box::new(Ex::Text("NPTH, mechanical".into()))
            )),
            Box::new(Ex::Compare(
                Compare::Equal,
                Box::new(Ex::Property(Side::B, "type".into())),
                Box::new(Ex::Text("track".into()))
            ))
        )
    );
}

#[test]
fn recognition_notes_what_the_router_does_not_keep_and_passes_the_rest() {
    let board = board(&[("A", &["Default"]), ("B", &["HV"])]);
    let rules = parse_rules(
        r#"(version 1)
(rule silk (constraint silk_clearance (min 0.2)))
(rule soft (severity warning) (constraint clearance (min 0.3)))
(rule inner (layer In1.Cu) (constraint clearance (min 0.3)))
(rule broken (condition "A.NetClass ==") (constraint clearance (min 0.3)))
(rule vias (constraint disallow blind_via micro_via))
(rule creep (constraint creepage (min 2)))
(rule hv (layer outer) (condition "A.NetClass == 'HV'") (constraint clearance (min 0.3)))"#,
    )
    .unwrap();
    let mut lowered = Lowered::new(board.nets.len());
    let recognised: Vec<Option<Vec<&str>>> = rules
        .iter()
        .map(|rule| {
            recognise(
                &mut lowered,
                rule,
                &rule.constraints[0],
                &board,
                &BTreeMap::new(),
            )
            .map(|(layers, _)| layers)
        })
        .collect();
    assert_eq!(
        recognised,
        [
            None,
            None,
            None,
            None,
            None,
            None,
            Some(vec!["F.Cu", "B.Cu"])
        ]
    );
    // One note for each rule recognised as not kept; none yet for the
    // last, whose kind's lowering notes it.
    let statuses: Vec<(&str, &str)> = lowered
        .report
        .iter()
        .map(|note| (note.rule.as_str(), note.status.as_str()))
        .collect();
    assert_eq!(
        statuses,
        [
            ("silk", "not_routing"),
            ("soft", "skipped"),
            ("inner", "skipped"),
            ("broken", "unmodelled"),
            ("vias", "no_effect"),
            ("creep", "unmodelled"),
        ]
    );
}

#[test]
fn the_matcher_groups_nets_and_a_later_rule_takes_what_it_surely_matches() {
    let board = board(&[("A", &["HV"]), ("B", &["Default"]), ("C", &["Default"])]);
    let no_net = NetFacts::default();
    let hv = parse_condition("A.NetClass == 'HV'").unwrap();
    let matcher = Matcher::new(
        &board,
        &no_net,
        &[],
        "clearance",
        vec!["F.Cu", "B.Cu"],
        hv.clone(),
    );
    // HV's net apart from the two the condition cannot tell apart.
    assert_eq!(matcher.groups, [vec![1, 2], vec![0]]);
    let net = |index: usize| Some(&board.nets[index]);
    assert_eq!(matcher.truth_for(net(0), net(1), false), T3::True);
    assert_eq!(matcher.truth_for(net(1), net(2), false), T3::False);
    assert!(!matcher.shadowed_any.get());
    // A later rule on every item takes every pair this one matches.
    let every = Ex::Number(1.0);
    let later = [(&every, vec!["F.Cu", "B.Cu"])];
    let shadowed = Matcher::new(
        &board,
        &no_net,
        &later,
        "clearance",
        vec!["F.Cu", "B.Cu"],
        hv,
    );
    assert_eq!(shadowed.truth_for(net(0), net(1), false), T3::False);
    assert!(shadowed.shadowed_any.get());
}

#[test]
fn a_size_rule_changes_what_its_nets_keep_and_notes_the_necks_it_raises() {
    use super::sizes::{changes_current, neck_note};
    let mut board = board(&[("A", &["Default"]), ("B", &["Default"])]);
    board.neck_width = 0.1;
    let constraint = |kind: &str, min: Option<f64>, opt: Option<f64>| Constraint {
        kind: kind.into(),
        min,
        opt,
        max: None,
        words: Vec::new(),
    };
    // The nets' vias: 0.4 mm over a 0.2 mm drill, an annular ring of 0.1.
    let ring = constraint("annular_width", Some(0.1), None);
    assert!(!changes_current(&board, "annular_width", &ring, &[0, 1]));
    let ring = constraint("annular_width", Some(0.15), None);
    assert!(changes_current(&board, "annular_width", &ring, &[0]));
    // A track width's opt off the nets' 0.1 mm changes them.
    let opt = constraint("track_width", None, Some(0.2));
    assert!(changes_current(&board, "track_width", &opt, &[1]));
    // A minimum above the neck-down width raises it: the board's for a
    // rule on every net on every layer, else the nets'.
    let wide = constraint("track_width", Some(0.15), None);
    assert_eq!(
        neck_note(&board, &wide, true, true, false).as_deref(),
        Some("; the neck-down width 0.1 mm rises to it")
    );
    assert_eq!(
        neck_note(&board, &wide, true, false, false).as_deref(),
        Some("; their neck-downs rise from 0.1 mm to it")
    );
    assert_eq!(neck_note(&board, &wide, false, true, false), None);
    let narrow = constraint("track_width", Some(0.1), None);
    assert_eq!(neck_note(&board, &narrow, true, true, false), None);
}

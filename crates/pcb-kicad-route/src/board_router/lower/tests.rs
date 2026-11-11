// Copyright (C) 2026 Toit contributors.

//! Tests of the lowering's stages on their own: the classes of the nets,
//! the merged pads, the mask openings' nets, net ties, padstacks, holes and
//! via layers. The whole lowering is checked against F1's golden table
//! (crates/pcb-kicad-route/src/tests/fixtures.rs).

use super::footprint::{carve_net_tie, net_tie_groups, touch_electrode};
use super::merge::terminal_hosts;
use super::pad::{drill_shape, padstack_variants};
use super::*;
use pcb_router as core;
use std::collections::HashMap;

const FOUR: &str = r#"(0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal)"#;

fn rules(clearance: f64) -> KiCadConnectionRoutingRules {
    KiCadConnectionRoutingRules {
        trace_width_mm: 0.2,
        clearance_mm: clearance,
        via_size_mm: 0.6,
        via_drill_mm: 0.3,
    }
}

fn config() -> KiCadBoardRouterConfig {
    KiCadBoardRouterConfig {
        default_rules: Some(rules(0.2)),
        ..KiCadBoardRouterConfig::default()
    }
}

/// A 20 x 20 mm board of `layers` with `items`.
fn board(layers: &str, items: &str) -> Expr {
    parse(&format!(
        r#"(kicad_pcb (version 20240108) (layers {layers} (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts")) {items})"#
    ))
    .unwrap()
}

fn square(center: [f64; 2], half: f64) -> core::Shape {
    let [x, y] = center;
    core::Shape::Polygon {
        points: vec![
            [x - half, y - half],
            [x + half, y - half],
            [x + half, y + half],
            [x - half, y + half],
        ],
    }
}

fn copper(shape: core::Shape, layers: core::LayerMask, net: Option<core::NetId>) -> core::Obstacle {
    core::Obstacle {
        shape,
        layers,
        kind: core::ObstacleKind::Copper,
        net,
        clearance: 0.0,
        clearance_override: None,
        plated: false,
        blocks_tracks: true,
        blocks_vias: true,
        label: String::new(),
    }
}

fn terminal(label: &str, pad: usize, anchor: [f64; 2], layers: core::LayerMask) -> core::Terminal {
    core::Terminal {
        anchor,
        layers,
        pad,
        contact: None,
        label: label.into(),
    }
}

#[test]
fn nets_of_the_same_rules_share_a_class() {
    let pcb = board(FOUR, "");
    let config = KiCadBoardRouterConfig {
        connection_rules: [("/WIDE".to_string(), rules(0.3))].into(),
        ..config()
    };
    let ctx = Context::new(&pcb, &config, true).unwrap();
    let mut builder = Builder::default();
    let a = builder.net_id(&ctx, "/A").unwrap();
    let wide = builder.net_id(&ctx, "/WIDE").unwrap();
    let b = builder.net_id(&ctx, "/B").unwrap();
    assert_eq!((a, wide, b), (0, 1, 2));
    // A name seen before keeps its id.
    assert_eq!(builder.net_id(&ctx, "/WIDE").unwrap(), 1);
    let classes: Vec<usize> = builder.nets.iter().map(|net| net.class).collect();
    assert_eq!(classes, [0, 1, 0]);
    assert_eq!(builder.classes.len(), 2);
    assert_eq!(builder.classes[1].clearance, 0.3);
    // Every layer open: the class's layers are 0.
    assert_eq!(builder.classes[0].layers, 0);
    // Without default rules a net of no rule of its own is an error.
    let bare = KiCadBoardRouterConfig::default();
    let ctx = Context::new(&pcb, &bare, true).unwrap();
    assert!(Builder::default().net_id(&ctx, "/A").is_err());
}

#[test]
fn overlapping_pads_of_one_number_merge_into_the_largest() {
    let mut builder = Builder {
        obstacles: vec![
            // U1.9: an exposed pad, a thermal via inside it, its copy on
            // the back; U1.2 apart.
            copper(square([5.0, 5.0], 1.0), 1, Some(0)),
            copper(square([5.2, 5.0], 0.2), 1, Some(0)),
            copper(square([9.0, 5.0], 0.3), 1, Some(0)),
            copper(square([5.0, 5.0], 1.0), 2, Some(0)),
            // The thermal via's padstack copper on the back.
            copper(square([5.2, 5.0], 0.25), 2, Some(0)),
        ],
        nets: vec![core::Net {
            name: "GND".into(),
            class: 0,
            terminals: vec![
                terminal("U1.9", 1, [5.2, 5.0], 1),
                terminal("U1.9", 0, [5.0, 5.0], 1),
                terminal("U1.2", 2, [9.0, 5.0], 1),
                terminal("U1.9", 3, [5.0, 5.0], 2),
            ],
        }],
        padstack_copper: vec![(1, 4)],
        ..Builder::default()
    };
    builder.merge_pads();
    let pads: Vec<usize> = builder.nets[0]
        .terminals
        .iter()
        .map(|terminal| terminal.pad)
        .collect();
    // The back copy shares no layer with the front pads: a terminal of its
    // own, as KiCad's connectivity has it.
    assert_eq!(pads, [0, 2, 3]);
    let merged = |builder: &Builder| -> Vec<(usize, usize, bool)> {
        builder
            .merged_pads
            .iter()
            .map(|merged| (merged.host, merged.pad, merged.padstack))
            .collect()
    };
    assert_eq!(merged(&builder), [(0, 1, false)]);
    // The via's padstack copper joins the terminal its pad was merged into.
    builder.join_padstack_copper();
    assert_eq!(merged(&builder), [(0, 1, false), (0, 4, true)]);
    assert_eq!(builder.merged_pads[1].anchor, [5.2, 5.0]);
    assert_eq!(builder.merged_pads[1].layers, 2);
}

#[test]
fn the_pads_of_a_jumper_group_are_one_terminal() {
    let obstacles = vec![
        copper(square([2.0, 2.0], 0.3), 1, Some(0)),
        copper(square([4.0, 2.0], 0.5), 1, Some(0)),
        copper(square([6.0, 2.0], 0.3), 1, Some(0)),
    ];
    let net = core::Net {
        name: "/J".into(),
        class: 0,
        terminals: vec![
            terminal("JP1.1", 0, [2.0, 2.0], 1),
            terminal("JP1.2", 1, [4.0, 2.0], 1),
            terminal("R1.1", 2, [6.0, 2.0], 1),
        ],
    };
    let mut groups = HashMap::new();
    assert_eq!(terminal_hosts(&net, &obstacles, &groups), [0, 1, 2]);
    groups.insert(0, (1, "group 0".to_string()));
    groups.insert(1, (1, "group 0".to_string()));
    // The larger pad is the group's terminal.
    assert_eq!(terminal_hosts(&net, &obstacles, &groups), [1, 1, 2]);
}

#[test]
fn a_mask_opening_belongs_to_the_one_net_it_exposes() {
    let pcb = board(FOUR, "");
    let config = config();
    let ctx = Context::new(&pcb, &config, true).unwrap();
    let opening = |center: [f64; 2]| MaskOpening {
        shapes: vec![square(center, 0.5)],
        layer: 0,
        kind: "gr_rect".into(),
        margin: 0.0,
        expansion: 0.0,
    };
    let mut builder = Builder {
        obstacles: vec![
            copper(square([5.0, 5.0], 0.2), 1, Some(0)),
            copper(square([10.0, 5.0], 0.2), 1, Some(0)),
            copper(square([10.2, 5.0], 0.2), 1, Some(1)),
            // On the back: not under a front opening.
            copper(square([15.0, 5.0], 0.2), 8, Some(1)),
        ],
        mask_openings: vec![
            opening([5.0, 5.0]),
            opening([10.0, 5.0]),
            opening([15.0, 5.0]),
        ],
        ..Builder::default()
    };
    builder.place_mask_openings(&ctx, &[]);
    assert!(builder.mask_openings.is_empty());
    let placed: Vec<(Option<core::NetId>, core::ObstacleKind, core::LayerMask)> = builder.obstacles
        [4..]
        .iter()
        .map(|obstacle| (obstacle.net, obstacle.kind, obstacle.layers))
        .collect();
    assert_eq!(
        placed,
        [
            (Some(0), core::ObstacleKind::Keepout, 1),
            (None, core::ObstacleKind::Keepout, 1),
            (None, core::ObstacleKind::Keepout, 1),
        ]
    );
    assert_eq!(builder.obstacles[4].label, "solder mask opening (gr_rect)");
}

#[test]
fn a_net_tie_joins_its_groups_or_by_the_legacy_keyword_all_its_pads() {
    let footprint = |forms: &str| {
        parse(&format!(
            r#"(footprint "NetTie" {forms}
              (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu"))
              (pad "2" smd rect (at 1 0) (size 0.5 0.5) (layers "F.Cu"))
              (pad "2" smd rect (at 1 1) (size 0.5 0.5) (layers "F.Cu")))"#
        ))
        .unwrap()
    };
    let groups = footprint(r#"(net_tie_pad_groups "1, 2" "3")"#);
    assert_eq!(net_tie_groups(&groups, Some(20240108)), [["1", "2"]]);
    let tagged = footprint(r#"(tags "net tie")"#);
    assert_eq!(net_tie_groups(&tagged, Some(20220815)), [["1", "2"]]);
    // Not at 20220817 and after, nor by another wording.
    assert!(net_tie_groups(&tagged, Some(20220817)).is_empty());
    assert!(net_tie_groups(&footprint(r#"(tags "Net Tie")"#), Some(20220815)).is_empty());
    assert!(!touch_electrode(&tagged));
}

#[test]
fn a_net_tie_bridge_is_carved_around_its_pads() {
    let bridge = square([0.0, 0.0], 0.5);
    // No disc touches it: the bridge itself.
    let kept = carve_net_tie(bridge.clone(), &[([5.0, 0.0], 0.5)]).unwrap();
    assert!(matches!(kept, core::Shape::Polygon { .. }));
    // A disc over all of it leaves nothing.
    assert!(carve_net_tie(bridge.clone(), &[([0.0, 0.0], 2.0)]).is_none());
    // A disc at one end leaves the rest, not where the disc is.
    let carved = carve_net_tie(bridge, &[([-0.5, 0.0], 0.3)]).unwrap();
    assert!(carved.contains([0.41, 0.011]));
    assert!(!carved.contains([-0.41, 0.011]));
}

#[test]
fn a_padstack_draws_its_inner_and_back_layers_in_their_own_shapes() {
    let pcb = board(FOUR, "");
    let layers = LayerTable::from_pcb(&pcb).unwrap();
    let pad = parse(
        r#"(pad "1" thru_hole circle (at 0 0) (size 1.2 1.2) (drill 0.8 (offset 0 0))
          (layers "*.Cu" "*.Mask")
          (padstack (mode front_inner_back)
            (layer "Inner" (shape circle) (size 1 1))
            (layer "B.Cu" (shape rect) (size 1.8 1.8) (offset 0.1 0))))"#,
    )
    .unwrap();
    let variants = padstack_variants(&pad, &layers);
    let masks: Vec<core::LayerMask> = variants.iter().map(|(mask, _)| *mask).collect();
    assert_eq!(masks, [0b0110, 0b1000]);
    let back = &variants[1].1;
    assert_eq!(back.children()[3].atom(), Some("rect"));
    assert_eq!(form_f64(back, "size", 1).unwrap(), 1.8);
    // The layer's offset is the drill's.
    assert_eq!(
        form_f64(back.child("drill").unwrap(), "offset", 1).unwrap(),
        0.1
    );
    assert!(padstack_variants(&parse(r#"(pad "1" smd rect)"#).unwrap(), &layers).is_empty());
}

#[test]
fn holes_and_via_layers() {
    let pcb = board(FOUR, "");
    let layers = LayerTable::from_pcb(&pcb).unwrap();
    let via =
        |forms: &str| parse(&format!("(via {forms} (at 1 1) (size 0.6) (drill 0.3))")).unwrap();
    assert_eq!(
        via_layers(&via(r#"(layers "F.Cu" "B.Cu")"#), &layers),
        0b1111
    );
    assert_eq!(
        via_layers(&via(r#"blind (layers "F.Cu" "In1.Cu")"#), &layers),
        0b0011
    );
    assert_eq!(
        via_layers(&via(r#"buried (layers "In2.Cu" "In1.Cu")"#), &layers),
        0b0110
    );
    // A round drill is a circle, an oval one a capsule along its long axis
    // turned with the pad.
    let pad = |forms: &str| parse(&format!("(pad \"1\" thru_hole oval {forms})")).unwrap();
    let Some(core::Shape::Circle { center, radius }) =
        drill_shape(&pad("(at 0 0) (drill 0.8)"), [3.0, 4.0]).unwrap()
    else {
        panic!("a round drill is a circle");
    };
    assert_eq!((center, radius), ([3.0, 4.0], 0.4));
    let Some(core::Shape::Capsule { start, end, radius }) =
        drill_shape(&pad("(at 0 0 90) (drill oval 2 1)"), [0.0, 0.0]).unwrap()
    else {
        panic!("an oval drill is a capsule");
    };
    assert_eq!(radius, 0.5);
    assert!((start[0] - end[0]).abs() < 1.0e-9 && (start[1] - end[1]).abs() > 0.99);
    assert!(drill_shape(&pad("(at 0 0)"), [0.0, 0.0]).unwrap().is_none());
}

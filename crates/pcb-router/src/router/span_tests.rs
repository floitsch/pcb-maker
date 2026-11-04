// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Blind, buried and micro vias (`Board::via_spans`, `SpanModel`).

use super::*;
use crate::board::{Net, Obstacle, ObstacleKind, RuleClass, Terminal, ViaKind, ViaSpan};
use crate::geometry::Shape;

fn span(top: usize, bottom: usize, micro: bool) -> ViaSpan {
    ViaSpan {
        top,
        bottom,
        micro,
        size: None,
    }
}

/// Two SMD pads of net A on F.Cu either side of a wall of another net's
/// copper on F.Cu across the whole board, on `layers` copper layers; a
/// rule area keeping tracks and vias out on every layer but F.Cu and
/// In1.Cu over the whole board (as the planes of a dense board would):
/// no through via fits anywhere, a via of F.Cu and In1.Cu does. Net B is
/// the wall.
fn walled_board(layers: usize, spans: Vec<ViaSpan>) -> Board {
    let class = RuleClass {
        trace_width: 0.2,
        clearance: 0.2,
        via_diameter: 0.6,
        via_drill: 0.3,
        layers: 0,
    };
    let pad = |center: [f64; 2], net: NetId, label: &str| Obstacle {
        shape: Shape::rectangle(center, [0.3, 0.3], 0.0),
        layers: 1,
        kind: ObstacleKind::Copper,
        net: Some(net),
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks: true,
        blocks_vias: true,
        label: label.into(),
    };
    let lower = (2..layers).fold(0, |mask, layer| mask | 1 << layer);
    let obstacles = vec![
        pad([2.0, 5.0], 0, "A.1"),
        pad([8.0, 5.0], 0, "A.2"),
        Obstacle {
            shape: Shape::rectangle([5.0, 5.0], [0.2, 6.0], 0.0),
            layers: 1,
            kind: ObstacleKind::Copper,
            net: Some(1),
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: "wall".into(),
        },
        pad([5.0, 9.6], 1, "B.1"),
        Obstacle {
            shape: Shape::rectangle([5.0, 5.0], [6.0, 6.0], 0.0),
            layers: lower,
            kind: ObstacleKind::Keepout,
            net: None,
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: "lower layers".into(),
        },
    ];
    let terminal = |pad: usize, anchor: [f64; 2], label: &str| Terminal {
        anchor,
        layers: 1,
        pad,
        contact: None,
        label: label.into(),
    };
    Board {
        layer_count: layers,
        outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        edge_clearance: 0.1,
        hole_clearance: 0.2,
        hole_to_hole: 0.25,
        classes: vec![class],
        neck_width: 0.2,
        obstacles,
        nets: vec![
            Net {
                name: "A".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [2.0, 5.0], "A.1"),
                    terminal(1, [8.0, 5.0], "A.2"),
                ],
            },
            Net {
                name: "B".into(),
                class: 0,
                terminals: vec![terminal(3, [5.0, 9.6], "B.1")],
            },
        ],
        planes: Vec::new(),
        solid_pads: Vec::new(),
        isolated_pads: Vec::new(),
        solid_pad_obstacles: Vec::new(),
        isolated_pad_obstacles: Vec::new(),
        pad_spokes: Default::default(),
        merged_pads: Vec::new(),
        no_via_pads: Vec::new(),
        fill_passes: Vec::new(),
        pin_row_gaps: Vec::new(),
        terminal_copper: Vec::new(),
        attachment: Default::default(),
        via_spans: spans,
    }
}

#[test]
fn a_via_takes_the_shortest_allowed_span_that_holds_both_layers() {
    let mut board = walled_board(6, Vec::new());
    // Without spans every via is a through via.
    assert_eq!(board.via_span(0, 1), None);
    board.via_spans = vec![
        span(0, 1, true),
        span(0, 2, false),
        span(1, 4, false),
        span(4, 5, true),
    ];
    assert_eq!(board.via_span(0, 1), Some(span(0, 1, true)));
    assert_eq!(board.via_span(1, 0), Some(span(0, 1, true)));
    assert_eq!(board.via_span(0, 2), Some(span(0, 2, false)));
    assert_eq!(board.via_span(2, 3), Some(span(1, 4, false)));
    assert_eq!(board.via_span(1, 4), Some(span(1, 4, false)));
    assert_eq!(board.via_span(5, 4), Some(span(4, 5, true)));
    // No span holds F.Cu and In3.Cu: a through via.
    assert_eq!(board.via_span(0, 3), None);
    assert_eq!(board.via_span(0, 5), None);
    // The router's table: the layers pierced per pair, and through vias
    // for a net with a pour.
    let model = SpanModel::new(&board, 0).unwrap();
    assert_eq!(model.mask(0, 0, 1), 0b11);
    assert_eq!(model.mask(0, 3, 2), 0b11110);
    assert_eq!(model.mask(0, 0, 3), 0b111111);
    assert_eq!(model.chain_mask(0, 0b101), 0b111);
}

#[test]
fn a_spans_kind_and_validity_follow_the_stack() {
    assert_eq!(span(0, 1, true).kind(4), ViaKind::Micro);
    assert_eq!(span(0, 2, false).kind(4), ViaKind::Blind);
    assert_eq!(span(1, 3, false).kind(4), ViaKind::Blind);
    assert_eq!(span(1, 2, false).kind(4), ViaKind::Buried);
    assert_eq!(span(0, 3, false).kind(4), ViaKind::Through);
    assert!(span(0, 1, true).is_valid(4));
    assert!(span(1, 2, true).is_valid(4));
    // A micro via is one dielectric deep; a span is inside the stack and
    // short of it.
    assert!(!span(0, 2, true).is_valid(4));
    assert!(!span(0, 3, false).is_valid(4));
    assert!(!span(2, 4, false).is_valid(4));
    assert!(!span(2, 2, false).is_valid(4));
    assert_eq!(span(1, 2, false).mask(), 0b110);
}

#[test]
fn a_board_whose_lower_layers_are_closed_routes_over_the_wall_with_micro_vias_only_where_allowed() {
    let config = Config {
        pitches: vec![0.1],
        ..Config::default()
    };
    // Through vias only: no via fits, A stays open.
    let board = walled_board(4, Vec::new());
    let result = crate::route(&board, &config);
    assert_ne!(result.status[0], NetStatus::Routed);
    // F.Cu to In1.Cu allowed: over the wall on In1.Cu with two micro vias.
    let board = walled_board(4, vec![span(0, 1, true)]);
    let result = crate::route(&board, &config);
    assert_eq!(result.status[0], NetStatus::Routed);
    let vias = &result.routes[0].vias;
    assert!(vias.len() >= 2, "{vias:?}");
    assert!(vias.iter().all(|via| via.span == Some(span(0, 1, true))));
    assert!(
        result.routes[0]
            .segments
            .iter()
            .all(|segment| segment.layer <= 1)
    );
    assert!(crate::verify::verify(&board, &result.routes).is_empty());
}

/// A track of net B on In2.Cu straight over a blind via of A (F.Cu to
/// In1.Cu) is legal (KiCad: the via has neither copper nor a hole there);
/// on In1.Cu it is not, and a buried via of B (In1.Cu to In2.Cu) at the
/// same place meets A's on In1.Cu while one of In2.Cu and B.Cu does not.
#[test]
fn the_verifier_holds_a_via_of_a_span_to_its_own_layers() {
    let mut board = walled_board(
        4,
        vec![span(0, 1, false), span(1, 2, false), span(2, 3, false)],
    );
    // Without the rule area over the lower layers.
    board.obstacles.pop();
    let via = |at: [f64; 2], span: Option<ViaSpan>| crate::board::Via {
        at,
        diameter: 0.6,
        drill: 0.3,
        span,
    };
    let track = |layer: usize| crate::board::Segment {
        layer,
        start: [6.0, 2.0],
        end: [9.0, 2.0],
        width: 0.2,
    };
    let mut routes = vec![NetRoute::default(); 2];
    routes[0]
        .vias
        .push(via([7.5, 2.0], Some(span(0, 1, false))));
    routes[1].segments.push(track(2));
    assert!(crate::verify::verify(&board, &routes).is_empty());
    routes[1].segments[0] = track(1);
    assert!(!crate::verify::verify(&board, &routes).is_empty());
    routes[1].segments.clear();
    routes[1]
        .vias
        .push(via([7.5, 2.0], Some(span(1, 2, false))));
    assert!(!crate::verify::verify(&board, &routes).is_empty());
    routes[1].vias[0] = via([7.5, 2.0], Some(span(2, 3, false)));
    assert!(crate::verify::verify(&board, &routes).is_empty());
    // A through via meets every span.
    routes[1].vias[0] = via([7.8, 2.0], None);
    assert!(!crate::verify::verify(&board, &routes).is_empty());
    // And a via of a span joins its layers only: F.Cu to In3 and back
    // through two vias of In2.Cu and B.Cu are not joined, through vias
    // join them.
    let mut route = NetRoute::default();
    let segment = |layer: usize, start: [f64; 2], end: [f64; 2]| crate::board::Segment {
        layer,
        start,
        end,
        width: 0.2,
    };
    route.vias.push(via([4.0, 2.0], Some(span(2, 3, false))));
    route.vias.push(via([6.0, 2.0], Some(span(2, 3, false))));
    route.segments.push(segment(0, [2.0, 5.0], [4.0, 2.0]));
    route.segments.push(segment(3, [4.0, 2.0], [6.0, 2.0]));
    route.segments.push(segment(0, [6.0, 2.0], [8.0, 5.0]));
    let components = crate::verify::copper_components(&board, 0, &route);
    assert_ne!(components[0], components[1]);
    for via in &mut route.vias {
        via.span = None;
    }
    let components = crate::verify::copper_components(&board, 0, &route);
    assert_eq!(components[0], components[1]);
}

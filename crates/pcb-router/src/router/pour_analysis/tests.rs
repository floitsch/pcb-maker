// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use super::joins::{Joins, main_piece};
use super::*;

#[test]
fn joins_lay_out_pieces_then_terminals_then_branches() {
    let joins = Joins::new(3, 2, 4);
    assert_eq!(joins.terminal_base, 4);
    assert_eq!(joins.branch_base, 6);
    assert_eq!(joins.parent, (0..10).collect::<Vec<_>>());
}

#[test]
fn the_main_piece_holds_the_most_terminals() {
    // Pieces 1 and 2; terminals 0 to 2 at 3 to 5.
    let mut joins = Joins::new(2, 3, 0);
    joins.union(3, 1);
    joins.union(4, 2);
    joins.union(5, 2);
    let labels = vec![vec![1, 1, 1, 1, 2]];
    let main = main_piece(&mut joins.parent, &labels, joins.terminal_base, 3);
    assert_eq!(main, find(&mut joins.parent, 2));
}

#[test]
fn on_equal_votes_the_main_piece_has_the_most_fill() {
    let mut joins = Joins::new(2, 2, 0);
    joins.union(3, 1);
    joins.union(4, 2);
    // Piece 2 has more nodes than piece 1.
    let labels = vec![vec![1, 2, 0], vec![2, 2, 0]];
    let main = main_piece(&mut joins.parent, &labels, joins.terminal_base, 2);
    assert_eq!(main, find(&mut joins.parent, 2));
    // Equal fill too: the least root.
    let mut joins = Joins::new(2, 2, 0);
    joins.union(3, 1);
    joins.union(4, 2);
    let labels = vec![vec![1, 2]];
    let main = main_piece(&mut joins.parent, &labels, joins.terminal_base, 2);
    assert_eq!(main, 1);
}

#[test]
fn without_terminals_the_main_piece_is_zero() {
    let mut joins = Joins::new(2, 0, 1);
    assert_eq!(main_piece(&mut joins.parent, &[vec![1, 2]], 3, 0), 0);
}

use crate::board::{Hatch, Net, Obstacle, ObstacleKind, Plane, RuleClass, Terminal};
use crate::geometry::Shape;

fn copper(shape: Shape, net: NetId, label: &str) -> Obstacle {
    Obstacle {
        shape,
        layers: 1,
        kind: ObstacleKind::Copper,
        net: Some(net),
        clearance: 0.0,
        clearance_override: None,
        plated: false,
        blocks_tracks: true,
        blocks_vias: true,
        label: label.into(),
    }
}

fn rect(center: [f64; 2], half: [f64; 2]) -> Shape {
    Shape::Polygon {
        points: vec![
            [center[0] - half[0], center[1] - half[1]],
            [center[0] + half[0], center[1] - half[1]],
            [center[0] + half[0], center[1] + half[1]],
            [center[0] - half[0], center[1] + half[1]],
        ],
    }
}

fn circle(center: [f64; 2], radius: f64) -> Shape {
    Shape::Circle { center, radius }
}

fn terminal(pad: usize, anchor: [f64; 2], label: &str) -> Terminal {
    Terminal {
        anchor,
        layers: 1,
        pad,
        contact: None,
        label: label.into(),
    }
}

/// A GND pour over a 10 mm square on one layer (thermal gap and spoke
/// 0.5 mm, minimum width 0.25 mm), hatched when `hatch` is given.
fn pour(hatch: Option<Hatch>, excluded: Vec<Vec<[f64; 2]>>) -> Plane {
    Plane {
        net: 0,
        class: 0,
        layer: 0,
        polygon: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        excluded,
        above: Vec::new(),
        connect: true,
        thermal_reach: 1.0,
        thermal_gap: 0.5,
        exclusive: false,
        solid: false,
        hatch,
        island_min_area: Some(0.0),
    }
}

fn hatch() -> Hatch {
    Hatch {
        thickness: 0.3,
        gap: 2.0,
        orientation: 0.0,
        smoothing_level: 0,
        smoothing_value: 0.0,
        min_hole_area: 0.0,
        via_reliefs: true,
    }
}

fn one_net_board(obstacles: Vec<Obstacle>, terminals: Vec<Terminal>, plane: Plane) -> Board {
    Board {
        layer_count: 1,
        outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        edge_clearance: 0.1,
        hole_clearance: 0.25,
        hole_to_hole: 0.25,
        classes: vec![RuleClass {
            trace_width: 0.25,
            clearance: 0.2,
            via_diameter: 0.6,
            via_drill: 0.3,
            layers: 0,
        }],
        neck_width: 0.25,
        obstacles,
        nets: vec![Net {
            name: "GND".into(),
            class: 0,
            terminals,
        }],
        planes: vec![plane],
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
        via_spans: Vec::new(),
    }
}

fn router_of(board: &Board, measured: bool) -> Router {
    Router::new(
        board,
        &Config {
            pitches: vec![0.1],
            measured_thermals: measured,
            ..Config::default()
        },
    )
}

/// Whether the analysis puts terminals `a` and `b` of net 0 in one group.
fn joined(router: &Router, a: usize, b: usize) -> bool {
    let (pours, mut parent, _) = router.analyze_pours(0);
    let base = pours.pieces + 1;
    find(&mut parent, base + a) == find(&mut parent, base + b)
}

#[test]
fn a_vias_spoke_whose_end_lies_in_no_fill_joins_nothing() {
    // capybully's SW48.3 on B.Cu: a 0.4 mm strip set to solid whose end
    // pokes 0.1 mm out of a keepout, beside a via of its net in a hatched
    // zone. The via's +y spoke would end at 0.955 mm below its centre,
    // in the keepout: KiCad drops it (its end is in no fill), keeps the
    // other three, and leaves the strip unconnected. Without the keepout
    // the spoke's end lies on the strip, in fill, and joins it.
    let obstacles = |strip: bool| {
        let mut obstacles = vec![
            copper(rect([2.0, 2.0], [0.5, 0.5]), 0, "P.1"),
            copper(circle([5.0, 5.0], 0.3), 0, "existing via"),
        ];
        if strip {
            obstacles.push(copper(rect([5.0, 6.5], [0.2, 0.9]), 0, "S.3"));
        }
        obstacles
    };
    let keepout = vec![[3.0, 5.7], [7.0, 5.7], [7.0, 9.0], [3.0, 9.0]];
    let board = |excluded: Vec<Vec<[f64; 2]>>| {
        let mut board = one_net_board(
            obstacles(true),
            vec![
                terminal(0, [2.0, 2.0], "P.1"),
                terminal(2, [5.0, 6.5], "S.3"),
            ],
            pour(Some(hatch()), excluded),
        );
        board.solid_pads = vec!["S.3".into()];
        board.solid_pad_obstacles = vec![2];
        board
    };
    for measured in [false, true] {
        let open = router_of(&board(vec![keepout.clone()]), measured);
        assert!(!joined(&open, 0, 1), "measured {measured}");
        // The via's other spokes still join it to the fill: the -y
        // spoke's corridor is fill of the main piece.
        let (pours, mut parent, main) = open.analyze_pours(0);
        let label = |at: [f64; 2]| pours.label[0][open.grid.nearest_node(at).unwrap()];
        assert_ne!(label([5.0, 4.4]), 0);
        assert_eq!(find(&mut parent, label([5.0, 4.4]) as usize), main);
        // Nor does the cut corridor of the +y spoke hold fill.
        assert_eq!(label([5.0, 5.5]), 0);
        let closed = router_of(&board(Vec::new()), measured);
        assert!(joined(&closed, 0, 1), "measured {measured}");
    }
}

#[test]
fn a_pads_zone_connection_is_its_own_not_its_labels() {
    // poly_kybd_split72_left's H5.1: a large round pad with the zone's
    // thermal relief and a small pad of its number set to solid inside it
    // (merged into its terminal). The label is solid by the small pad;
    // the large pad keeps its relief, so the fill does not run over it
    // and C.2's spoke, whose end lies on it, lands in no fill. By the
    // label (the board without the per-pad lists) the fill covered it.
    let board = |per_pad: bool| {
        let mut board = one_net_board(
            vec![
                copper(circle([6.0, 5.0], 1.5), 0, "H.1"),
                copper(circle([6.0, 4.0], 0.3), 0, "H.1"),
                copper(rect([3.9, 5.0], [0.25, 0.25]), 0, "C.2"),
            ],
            vec![
                terminal(0, [6.0, 5.0], "H.1"),
                terminal(2, [3.9, 5.0], "C.2"),
            ],
            pour(None, Vec::new()),
        );
        board.merged_pads.push(crate::board::MergedPad {
            net: 0,
            host: 0,
            pad: 1,
            anchor: [6.0, 4.0],
            layers: 1,
            padstack: false,
        });
        board.solid_pads = vec!["H.1".into()];
        if per_pad {
            board.solid_pad_obstacles = vec![1];
        }
        board
    };
    let per_pad = board(true);
    assert_eq!(zone_connection(&per_pad, "H.1", 0), ZoneConnection::Thermal);
    assert_eq!(zone_connection(&per_pad, "H.1", 1), ZoneConnection::Solid);
    assert_eq!(
        zone_connection(&board(false), "H.1", 0),
        ZoneConnection::Solid
    );
    let plus_x = |board: &Board| {
        let router = router_of(board, false);
        assert_eq!(
            router.thermal_ring_pads(0)[0]
                .iter()
                .any(|(pad, _)| *pad == 0),
            board.solid_pad_obstacles == vec![1]
        );
        let mut report = Vec::new();
        router.analyze_pours_reporting(0, &|_| false, Some(&mut report));
        report
            .iter()
            .find(|record| record.pad == 2)
            .unwrap()
            .outcomes[0]
    };
    assert!(matches!(plus_x(&board(false)), SpokeOutcome::Good(_)));
    assert!(!matches!(plus_x(&per_pad), SpokeOutcome::Good(_)));
}

#[test]
fn fill_touching_a_merged_solid_pad_joins_its_terminal() {
    // capybully's SW39.3: two overlapping strips of one number on B.Cu,
    // both set to solid, merged into one terminal (the first). A keepout
    // covers the first and all but the end of the second, where the fill
    // touches it: KiCad's fill joins both, one item to its connectivity.
    let board = one_net_board(
        vec![
            copper(rect([2.0, 2.0], [0.5, 0.5]), 0, "P.1"),
            copper(rect([5.0, 5.0], [0.2, 2.0]), 0, "S.3"),
            copper(rect([5.0, 4.0], [0.2, 2.0]), 0, "S.3"),
        ],
        vec![
            terminal(0, [2.0, 2.0], "P.1"),
            terminal(1, [5.0, 5.0], "S.3"),
        ],
        pour(
            None,
            vec![vec![[3.0, 2.3], [7.0, 2.3], [7.0, 9.0], [3.0, 9.0]]],
        ),
    );
    let mut board = board;
    board.merged_pads.push(crate::board::MergedPad {
        net: 0,
        host: 1,
        pad: 2,
        anchor: [5.0, 4.0],
        layers: 1,
        padstack: false,
    });
    board.solid_pads = vec!["S.3".into()];
    board.solid_pad_obstacles = vec![1, 2];
    // In the measurement the second strip is solid by its own setting.
    assert!(joined(&router_of(&board, true), 0, 1));
}

#[test]
fn a_vias_spoke_ending_in_another_items_spoke_is_kept() {
    // le_chiffre's GND vias in a hatched zone of a 1.3 mm web, whose
    // spokes end 0.655 mm past the gap (beyond the via's own ring): a
    // via's +x spoke ends in the relief of the thermal pad P.2 beside it,
    // in no fill, but inside P.2's -x spoke. KiCad keeps a spoke whose end lies in another spoke
    // (fillCopperZone's second test), so the corridor between them holds
    // fill; without P.2 the via's spoke lands in the open fill all the
    // same.
    let board = |pad: bool| {
        let mut obstacles = vec![
            copper(rect([2.0, 2.0], [0.5, 0.5]), 0, "P.1"),
            copper(circle([5.0, 5.0], 0.3), 0, "existing via"),
        ];
        let mut terminals = vec![terminal(0, [2.0, 2.0], "P.1")];
        if pad {
            obstacles.push(copper(rect([7.0, 5.0], [0.25, 0.25]), 0, "P.2"));
            terminals.push(terminal(2, [7.0, 5.0], "P.2"));
        }
        let web = Hatch {
            thickness: 1.3,
            gap: 0.5,
            ..hatch()
        };
        one_net_board(obstacles, terminals, pour(Some(web), Vec::new()))
    };
    for pad in [true, false] {
        let router = router_of(&board(pad), false);
        let (pours, _, _) = router.analyze_pours(0);
        let label = |at: [f64; 2]| pours.label[0][router.grid.nearest_node(at).unwrap()];
        assert_ne!(label([5.7, 5.0]), 0, "pad {pad}");
        // Off the corridor the relief holds no fill.
        assert_eq!(label([5.55, 5.55]), 0, "pad {pad}");
    }
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The rules beyond the net classes (`crate::rules`, `Config::net_rules`).

use super::*;
use crate::board::{Net, Obstacle, ObstacleKind, RuleClass, Terminal};
use crate::geometry::{Shape, point_segment_distance};
use crate::rules::{ALL_KINDS, ClearanceRule, NetRules, PAD, TRACK, kind_bit};

fn pad(center: [f64; 2], radius: f64, layers: u32, net: Option<NetId>, label: &str) -> Obstacle {
    Obstacle {
        shape: Shape::Circle { center, radius },
        layers,
        kind: ObstacleKind::Copper,
        net,
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks: true,
        blocks_vias: true,
        label: label.into(),
    }
}

fn terminal(pad: usize, at: [f64; 2], layers: u32, label: &str) -> Terminal {
    Terminal {
        anchor: at,
        layers,
        pad,
        contact: None,
        label: label.into(),
    }
}

/// Nets `A` (pads at (1, 5) and (9, 5)) and `B` (pads at (1, 8) and (9,
/// 8)) on `layer_count` layers, and a pad of no net at (5, 5.55), 0.55 mm
/// above A's straight line.
fn two_nets(layer_count: usize) -> Board {
    let obstacles = vec![
        pad([1.0, 5.0], 0.2, 1, Some(0), "A.1"),
        pad([9.0, 5.0], 0.2, 1, Some(0), "A.2"),
        pad([1.0, 8.0], 0.2, 1, Some(1), "B.1"),
        pad([9.0, 8.0], 0.2, 1, Some(1), "B.2"),
        pad([5.0, 5.55], 0.2, 1, None, "U1.1"),
    ];
    Board {
        layer_count,
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
        neck_width: 0.15,
        obstacles,
        nets: vec![
            Net {
                name: "A".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [1.0, 5.0], 1, "A.1"),
                    terminal(1, [9.0, 5.0], 1, "A.2"),
                ],
            },
            Net {
                name: "B".into(),
                class: 0,
                terminals: vec![
                    terminal(2, [1.0, 8.0], 1, "B.1"),
                    terminal(3, [9.0, 8.0], 1, "B.2"),
                ],
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
    }
}

fn config(rules: Option<NetRules>) -> Config {
    Config {
        pitches: vec![0.1],
        net_rules: rules.map(Arc::new),
        ..Config::default()
    }
}

/// The closest a net's tracks come to `point`.
fn closest(result: &RoutingResult, net: usize, point: [f64; 2]) -> f64 {
    result.routes[net]
        .segments
        .iter()
        .map(|segment| point_segment_distance(point, segment.start, segment.end))
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_pad_to_track_rule_keeps_tracks_from_pads_only() {
    let board = two_nets(1);
    // The classes alone: A passes the pad within 0.8 mm of its centre
    // (0.525 needed).
    let plain = route(&board, &config(None));
    assert!(
        plain
            .status
            .iter()
            .all(|status| *status == NetStatus::Routed)
    );
    assert!(closest(&plain, 0, [5.0, 5.55]) < 0.8);
    // JLCPCB's "Pad to Track" 0.5 mm, every net: 0.125 + 0.5 + 0.2 from
    // the pad's centre.
    let rules = NetRules {
        clearances: vec![ClearanceRule {
            first: None,
            second: None,
            kinds: kind_bit(TRACK, PAD),
            layers: 0,
            clearance: 0.5,
        }],
        ..NetRules::default()
    };
    let ruled = route(&board, &config(Some(rules)));
    assert!(
        ruled
            .status
            .iter()
            .all(|status| *status == NetStatus::Routed)
    );
    assert!(closest(&ruled, 0, [5.0, 5.55]) >= 0.825 - 1.0e-6);
    // Tracks against tracks keep the classes' clearance: the rule did
    // not widen the stamps.
    let router = Router::new(
        &board,
        &config(Some(NetRules {
            clearances: vec![ClearanceRule {
                first: None,
                second: None,
                kinds: kind_bit(TRACK, PAD),
                layers: 0,
                clearance: 0.5,
            }],
            ..NetRules::default()
        })),
    );
    let base = Router::new(&board, &config(None));
    assert_eq!(
        router.stamps[0][0].trace_to_trace,
        base.stamps[0][0].trace_to_trace
    );
}

#[test]
fn a_rule_between_two_sets_of_nets_splits_their_classes() {
    let mut board = two_nets(2);
    // A third net C of the same class, not named by the rule.
    board
        .obstacles
        .push(pad([1.0, 2.0], 0.2, 1, Some(2), "C.1"));
    board
        .obstacles
        .push(pad([9.0, 2.0], 0.2, 1, Some(2), "C.2"));
    board.nets.push(Net {
        name: "C".into(),
        class: 0,
        terminals: vec![
            terminal(5, [1.0, 2.0], 1, "C.1"),
            terminal(6, [9.0, 2.0], 1, "C.2"),
        ],
    });
    let rules = NetRules {
        clearances: vec![ClearanceRule {
            first: Some(vec!["A".into()]),
            second: Some(vec!["B".into()]),
            kinds: ALL_KINDS,
            // Layer 1 only.
            layers: 0b10,
            clearance: 1.0,
        }],
        ..NetRules::default()
    };
    let router = Router::new(&board, &config(Some(rules)));
    let class = |net: usize| router.board.nets[net].class;
    // A, B and C each have a class of their own now (A's is the
    // original, the first net's).
    assert_eq!(class(0), 0);
    assert_ne!(class(0), class(1));
    assert_ne!(class(1), class(2));
    assert_ne!(class(0), class(2));
    let split = router.class_rules.as_ref().unwrap();
    assert_eq!(split.resolved.variants, 2);
    let classes = router.board.classes.len();
    let radius = |stamps: &Stamps| {
        stamps
            .trace_to_trace
            .iter()
            .map(|(dx, dy)| ((*dx * *dx + *dy * *dy) as f64).sqrt())
            .fold(0.0, f64::max)
            * 0.1
    };
    let on = |own: usize, other: usize, layer: usize| {
        &router.stamps[class(own)][split.resolved.layer_variant[layer] * classes + class(other)]
    };
    // A against B: the rule's 1.0 + 0.25 on layer 1, the classes' 0.45
    // on layer 0; A against C (and B against C) the classes' everywhere.
    assert!(radius(on(0, 1, 1)) > 1.2 && radius(on(0, 1, 1)) < 1.25);
    assert!(radius(on(1, 0, 1)) > 1.2);
    assert!(radius(on(0, 1, 0)) < 0.45);
    assert!(radius(on(0, 2, 1)) < 0.45);
    assert!(radius(on(1, 2, 1)) < 0.45);
    // Vias meet on every layer: the rule's everywhere.
    let via = |stamps: &Stamps| {
        stamps
            .via_to_via
            .iter()
            .map(|(dx, dy)| ((*dx * *dx + *dy * *dy) as f64).sqrt())
            .fold(0.0, f64::max)
            * 0.1
    };
    assert!(via(on(0, 1, 0)) > 1.5);
    assert!(via(on(0, 2, 0)) < 0.9);
}

#[test]
fn a_net_width_floor_holds_for_its_neck_too() {
    let board = two_nets(1);
    let rules = NetRules {
        width_floor: [("A".to_string(), 0.3)].into(),
        ..NetRules::default()
    };
    let router = Router::new(&board, &config(Some(rules)));
    let a = router.board.nets[0].class;
    let b = router.board.nets[1].class;
    assert_eq!(router.board.classes[a].trace_width, 0.3);
    // No neck narrower than its floor.
    assert_eq!(router.neck_of[a], None);
    assert_eq!(router.neck_width_of(a), 0.3);
    // B keeps the class width and the board's neck.
    assert_eq!(router.board.classes[b].trace_width, 0.25);
    let neck = router.neck_of[b].unwrap();
    assert_eq!(router.board.classes[neck].trace_width, 0.15);
    assert_eq!(router.neck_width_of(b), 0.15);
    let result = route(
        &board,
        &config(Some(NetRules {
            width_floor: [("A".to_string(), 0.3)].into(),
            ..NetRules::default()
        })),
    );
    assert!(
        result.routes[0]
            .segments
            .iter()
            .all(|segment| segment.width >= 0.3 - 1.0e-9)
    );
}

#[test]
fn a_via_keeps_the_hole_to_hole_spacing_from_its_own_nets_pad_hole() {
    let mut board = two_nets(2);
    board.classes[0] = RuleClass {
        trace_width: 0.2,
        clearance: 0.1,
        via_diameter: 0.4,
        via_drill: 0.2,
        layers: 0,
    };
    board.hole_to_hole = 0.5;
    board.hole_clearance = 0.2;
    // A through-hole pad of A at (5, 3): 1.3 copper, 1.0 drill.
    let copper = board.obstacles.len();
    board.obstacles.push(Obstacle {
        layers: 0b11,
        ..pad([5.0, 3.0], 0.65, 0b11, Some(0), "J3.5")
    });
    board.obstacles.push(Obstacle {
        kind: ObstacleKind::Hole,
        layers: 0b11,
        ..pad([5.0, 3.0], 0.5, 0b11, Some(0), "J3.5 plated hole")
    });
    board.nets[0]
        .terminals
        .push(terminal(copper, [5.0, 3.0], 0b11, "J3.5"));
    // A node 1.0 mm from the hole's centre: the copper rule (0.65 + 0.2
    // + 0.1) lets A's via there, the hole to hole (0.5 + 0.5 + 0.1) does
    // not; B's via never.
    let check = |rules: Option<NetRules>| {
        let router = Router::new(&board, &config(rules));
        let node = router.grid.nearest_node([6.0, 3.0]).unwrap();
        let at = router.grid.center_of(node);
        assert!((crate::geometry::distance(at, [5.0, 3.0]) - 1.0).abs() < 0.06);
        let statics = &router.statics[router.board.nets[0].class];
        (statics.via_allowed(node, 0), statics.via_allowed(node, 1))
    };
    assert_eq!(check(None), (true, false));
    assert_eq!(
        check(Some(NetRules {
            same_net_hole_to_hole: 0.5,
            ..NetRules::default()
        })),
        (false, false)
    );
}

#[test]
fn rules_that_split_nothing_route_as_without_them() {
    let board = two_nets(2);
    let plain = route(&board, &config(None));
    for rules in [
        NetRules::default(),
        NetRules {
            same_net_hole_to_hole: 0.25,
            ..NetRules::default()
        },
        // A rule the classes already keep.
        NetRules {
            clearances: vec![ClearanceRule {
                first: Some(vec!["A".into()]),
                second: None,
                kinds: ALL_KINDS,
                layers: 0,
                clearance: 0.1,
            }],
            ..NetRules::default()
        },
    ] {
        let ruled = route(&board, &config(Some(rules)));
        assert_eq!(plain.iterations, ruled.iterations);
        assert_eq!(plain.expansions, ruled.expansions);
        for (a, b) in plain.routes.iter().zip(&ruled.routes) {
            assert_eq!(a.segments, b.segments);
            assert_eq!(a.vias, b.vias);
        }
    }
}

#[test]
fn the_stub_check_holds_the_rules_beyond_the_classes() {
    // tuxkeychain: 0.127 mm between outer-layer tracks over classes of
    // 0.1. Two tracks 0.104 mm apart keep the classes but not the rule;
    // the verifier the clean-up of pad stubs asks (`verify_with_rules`)
    // sees it, `verify` (the classes) does not.
    let mut board = two_nets(1);
    board.classes[0].clearance = 0.1;
    let rules = NetRules {
        clearances: vec![ClearanceRule {
            first: None,
            second: None,
            kinds: kind_bit(TRACK, TRACK),
            layers: 0,
            clearance: 0.127,
        }],
        ..NetRules::default()
    };
    let (split, class_rules) = crate::rules::ClassRules::split(&board, &rules);
    let track = |y: f64| crate::board::Segment {
        layer: 0,
        start: [2.0, y],
        end: [4.0, y],
        width: 0.25,
    };
    let mut routes = vec![crate::board::NetRoute::default(); 2];
    routes[0].segments.push(track(3.0));
    routes[1].segments.push(track(3.0 + 0.25 + 0.104));
    assert!(crate::verify::verify(&split, &routes).is_empty());
    let found = crate::verify::verify_with_rules(&split, &routes, Some(&class_rules));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].other_segment, Some((1, 0)));
    assert!((found[0].required - 0.127).abs() < 1.0e-9);
    assert!((found[0].actual - 0.104).abs() < 1.0e-9);
    // At the rule's distance, nothing.
    routes[1].segments[0] = track(3.0 + 0.25 + 0.127);
    assert!(crate::verify::verify_with_rules(&split, &routes, Some(&class_rules)).is_empty());
    // A via of B as close: the rule is between tracks only.
    routes[1].segments.clear();
    routes[1].vias.push(crate::board::Via {
        at: [3.0, 3.0 + 0.125 + 0.3 + 0.104],
        diameter: 0.6,
        drill: 0.3,
    });
    assert!(crate::verify::verify_with_rules(&split, &routes, Some(&class_rules)).is_empty());
}

/// A 1.4 mm square SMD pad of A at (5, 3) on the front.
fn square_pad(board: &mut Board) -> usize {
    let index = board.obstacles.len();
    board.obstacles.push(Obstacle {
        shape: Shape::Polygon {
            points: vec![[4.3, 2.3], [5.7, 2.3], [5.7, 3.7], [4.3, 3.7]],
        },
        ..pad([5.0, 3.0], 0.7, 1, Some(0), "U1.EP")
    });
    index
}

#[test]
fn a_via_keeps_its_ring_out_of_a_listed_smd_pad_even_of_its_own_net() {
    let mut board = two_nets(2);
    let index = square_pad(&mut board);
    // Via 0.6 mm, clearance 0.2: the ring is 0.3 from the centre.
    let allowed = |board: &Board, x: f64| {
        let router = Router::new(board, &config(None));
        let node = router.grid.nearest_node([x, 3.0]).unwrap();
        assert!((router.grid.center_of(node)[0] - x).abs() < 1.0e-6);
        let statics = &router.statics[router.board.nets[0].class];
        (statics.via_allowed(node, 0), statics.via_allowed(node, 1))
    };
    // Not listed (the rule of before): A's vias inside the pad where they
    // fit wholly, B's nowhere near; just outside the pad A keeps the
    // clearance as from any copper.
    assert_eq!(allowed(&board, 5.0), (true, false));
    assert_eq!(allowed(&board, 6.0), (false, false));
    // Listed: no via inside, A's from where its ring stays outside (0.3
    // from the edge at 5.7, a node on: 6.1), B's at its clearance beyond
    // (6.2 and more, a node on: 6.3).
    board.no_via_pads = vec![index];
    assert_eq!(allowed(&board, 5.0), (false, false));
    assert_eq!(allowed(&board, 5.9), (false, false));
    assert_eq!(allowed(&board, 6.1), (true, false));
    assert_eq!(allowed(&board, 6.3), (true, true));
}

#[test]
fn a_touch_electrodes_mask_opening_keeps_its_vias_inside_the_pad_only() {
    // A's 1.4 mm pad at (5, 3) under a mask opening of A over it and 1 mm
    // beyond (d20_pcb's J2: the die's whole face is open). Inside the pad
    // A's via stays (copper of one net in one opening bridges nothing);
    // beside a listed pad, where the mask web would part the via from the
    // pad's solder, the opening keeps it out as before; so does another
    // footprint's opening (an exposed pad's mask windows).
    let allowed = |listed: bool, label: &str, x: f64| {
        let mut board = two_nets(2);
        let index = square_pad(&mut board);
        if listed {
            board.no_via_pads = vec![index];
        }
        board.obstacles.push(Obstacle {
            shape: Shape::Polygon {
                points: vec![[3.3, 1.3], [7.7, 1.3], [7.7, 4.7], [3.3, 4.7]],
            },
            kind: ObstacleKind::Keepout,
            clearance: 0.05,
            label: label.into(),
            ..pad([5.0, 3.0], 0.7, 1, Some(0), "")
        });
        let router = Router::new(&board, &config(None));
        let node = router.grid.nearest_node([x, 3.0]).unwrap();
        let statics = &router.statics[router.board.nets[0].class];
        (statics.via_allowed(node, 0), statics.via_allowed(node, 1))
    };
    let electrode = "solder mask opening (fp_poly of the touch electrode J2)";
    assert_eq!(allowed(false, electrode, 5.0), (true, false));
    assert_eq!(allowed(true, electrode, 5.0), (false, false));
    assert_eq!(allowed(true, electrode, 6.1), (false, false));
    assert_eq!(
        allowed(false, "solder mask opening (fp_poly)", 5.0),
        (false, false)
    );
}

#[test]
fn an_exposed_pads_own_vias_do_not_depend_on_the_order_of_the_pads() {
    // A listed pin of A 0.05 mm right of A's exposed pad (1.4 mm square,
    // not listed): a node inside the pad within the pin's via reach (5.3:
    // 0.45 from the pin, the via's ring 0.1 inside the pad's edge) holds
    // A's via whether the pin comes before the pad or after it.
    let allowed = |pin_first: bool| {
        let mut board = two_nets(2);
        let pin = Obstacle {
            shape: Shape::Polygon {
                points: vec![[5.75, 2.85], [6.25, 2.85], [6.25, 3.15], [5.75, 3.15]],
            },
            ..pad([6.0, 3.0], 0.25, 1, Some(0), "U1.1")
        };
        let pin_index = if pin_first {
            board.obstacles.push(pin);
            square_pad(&mut board);
            board.obstacles.len() - 2
        } else {
            square_pad(&mut board);
            board.obstacles.push(pin);
            board.obstacles.len() - 1
        };
        board.no_via_pads = vec![pin_index];
        let router = Router::new(&board, &config(None));
        let node = router.grid.nearest_node([5.3, 3.0]).unwrap();
        assert!((router.grid.center_of(node)[0] - 5.3).abs() < 1.0e-6);
        let statics = &router.statics[router.board.nets[0].class];
        (statics.via_allowed(node, 0), statics.via_allowed(node, 1))
    };
    assert_eq!(allowed(false), (true, false));
    assert_eq!(allowed(true), (true, false));
}

#[test]
fn a_padstack_terminal_is_reached_inside_each_layers_own_shape() {
    // A's pad at (1, 5) is 0.2 mm in radius on the front and 0.1 on the
    // back (a padstack's back shape, merged into the terminal): its back
    // nodes lie within the back's 0.1 mm, its front ones in the front's.
    let mut board = two_nets(2);
    board.obstacles[0].layers = 0b01;
    board.nets[0].terminals[0].layers = 0b11;
    let back = board.obstacles.len();
    board
        .obstacles
        .push(pad([1.0, 5.0], 0.1, 0b10, Some(0), "A.1 padstack"));
    board.merged_pads.push(crate::board::MergedPad {
        net: 0,
        host: 0,
        pad: back,
        anchor: [1.0, 5.0],
        layers: 0b10,
        padstack: true,
    });
    let router = Router::new(&board, &config(None));
    let state = router.prepare_net(0);
    let farthest = |layer: u8| {
        state.terminal_nodes[0]
            .iter()
            .filter(|node| node.layer == layer)
            .map(|node| {
                crate::geometry::distance([1.0, 5.0], router.grid.center_of(node.cell as usize))
            })
            .fold(0.0, f64::max)
    };
    assert!(
        farthest(0) > 0.1 + 1.0e-6,
        "front nodes in the 0.2 mm shape"
    );
    assert!(
        state.terminal_nodes[0].iter().any(|node| node.layer == 1),
        "the back is reached"
    );
    assert!(farthest(1) <= 0.1, "back nodes in the 0.1 mm shape");
}

#[test]
fn a_pad_of_a_pour_net_gets_its_via_beside_the_pad() {
    // A's pads on the front, its pour on the back of the whole board: the
    // pads reach it by vias, which stay out of the listed pads.
    let mut board = two_nets(2);
    let index = square_pad(&mut board);
    board.nets[0]
        .terminals
        .push(terminal(index, [5.0, 3.0], 1, "U1.EP"));
    board.no_via_pads = vec![0, 1, index];
    board.planes.push(crate::board::Plane {
        net: 0,
        class: 0,
        layer: 1,
        polygon: board.outline.clone(),
        excluded: Vec::new(),
        above: Vec::new(),
        connect: true,
        thermal_reach: 0.0,
        thermal_gap: 0.0,
        exclusive: false,
        solid: true,
        hatch: None,
    });
    let result = route(&board, &config(None));
    assert_eq!(result.status[0], NetStatus::Routed);
    let vias = &result.routes[0].vias;
    assert!(!vias.is_empty());
    for via in vias {
        for &pad in &board.no_via_pads {
            let distance = board.obstacles[pad].shape.distance_to_point(via.at);
            assert!(
                distance >= via.diameter / 2.0 - 1.0e-6,
                "a via at {:?} {distance:.3} mm from {}",
                via.at,
                board.obstacles[pad].label
            );
        }
    }
}

#[test]
fn a_through_hole_pad_holds_no_via_of_its_own_net() {
    let mut board = two_nets(2);
    let copper = board.obstacles.len();
    board
        .obstacles
        .push(pad([5.0, 3.0], 0.8, 0b11, Some(0), "J1.1"));
    board.obstacles.push(Obstacle {
        kind: ObstacleKind::Hole,
        ..pad([5.0, 3.0], 0.5, 0b11, Some(0), "J1.1 plated hole")
    });
    board.nets[0]
        .terminals
        .push(terminal(copper, [5.0, 3.0], 0b11, "J1.1"));
    let router = Router::new(&board, &config(None));
    let node = router.grid.nearest_node([5.0, 3.0]).unwrap();
    assert!(!router.statics[router.board.nets[0].class].via_allowed(node, 0));
}

#[test]
fn a_path_hopping_over_a_wall_keeps_its_two_vias_the_hole_spacing_apart() {
    // A's pads on the back (layer 1), a wall across the whole board on
    // the back that keeps tracks out, not vias: A goes through the front
    // for a short hop. Its two vias keep the drill plus hole to hole
    // (0.3 + 0.25) apart, as KiCad's hole_to_hole holds for one net too.
    let mut board = two_nets(2);
    board.obstacles.truncate(4);
    board.nets.truncate(1);
    for index in 0..2 {
        board.obstacles[index].layers = 0b10;
        board.nets[0].terminals[index].layers = 0b10;
    }
    board.obstacles.drain(2..);
    board.obstacles.push(Obstacle {
        shape: Shape::Polygon {
            points: vec![[4.95, -1.0], [5.05, -1.0], [5.05, 11.0], [4.95, 11.0]],
        },
        layers: 0b10,
        kind: ObstacleKind::Keepout,
        net: None,
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks: true,
        blocks_vias: false,
        label: "wall".into(),
    });
    let result = route(&board, &config(None));
    assert_eq!(result.status[0], NetStatus::Routed);
    let vias = &result.routes[0].vias;
    assert!(vias.len() >= 2, "{vias:?}");
    for (index, a) in vias.iter().enumerate() {
        for b in &vias[index + 1..] {
            let apart = crate::geometry::distance(a.at, b.at);
            assert!(apart >= 0.55 - 1.0e-6, "vias {apart:.3} mm apart: {vias:?}");
        }
    }
}

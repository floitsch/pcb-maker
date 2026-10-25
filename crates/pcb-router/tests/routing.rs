// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use pcb_router::{
    Board, Config, Net, NetStatus, Obstacle, ObstacleKind, Plane, RuleClass, Shape, Terminal,
    route, verify,
};

fn class() -> RuleClass {
    RuleClass {
        trace_width: 0.25,
        clearance: 0.2,
        via_diameter: 0.7,
        via_drill: 0.3,
        layers: 0,
    }
}

struct Builder {
    board: Board,
}

impl Builder {
    fn new(width: f64, height: f64, layers: usize) -> Self {
        Self {
            board: Board {
                layer_count: layers,
                outline: vec![[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]],
                edge_clearance: 0.2,
                hole_clearance: 0.25,
                hole_to_hole: 0.25,
                classes: vec![class()],
                neck_width: 0.25,
                obstacles: Vec::new(),
                nets: Vec::new(),
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
            },
        }
    }

    fn net(&mut self, name: &str, pads: &[([f64; 2], u32)]) {
        let id = self.board.nets.len() as u32;
        let mut terminals = Vec::new();
        for (index, (at, layers)) in pads.iter().enumerate() {
            self.board.obstacles.push(Obstacle {
                shape: Shape::Circle {
                    center: *at,
                    radius: 0.5,
                },
                layers: *layers,
                kind: ObstacleKind::Copper,
                net: Some(id),
                clearance: 0.0,
                clearance_override: None,
                blocks_tracks: true,
                blocks_vias: true,
                label: format!("{name}.{index}"),
            });
            terminals.push(Terminal {
                anchor: *at,
                layers: *layers,
                pad: self.board.obstacles.len() - 1,
                contact: None,
                label: format!("{name}.{index}"),
            });
        }
        self.board.nets.push(Net {
            name: name.into(),
            class: 0,
            terminals,
        });
    }
}

fn config() -> Config {
    Config {
        pitches: vec![0.1],
        ..Config::default()
    }
}

#[test]
fn crossing_nets_on_one_layer_need_two_layers() {
    let mut builder = Builder::new(20.0, 20.0, 2);
    builder.net("H", &[([2.0, 10.0], 0b11), ([18.0, 10.0], 0b11)]);
    builder.net("V", &[([10.0, 2.0], 0b11), ([10.0, 18.0], 0b11)]);
    let result = route(&builder.board, &config());
    assert_eq!(result.status, vec![NetStatus::Routed, NetStatus::Routed]);
    assert!(verify(&builder.board, &result.routes).is_empty());
    // Through-hole pads change layers for free, so no via is needed.
    assert_eq!(
        result
            .routes
            .iter()
            .map(|route| route.vias.len())
            .sum::<usize>(),
        0
    );
    let layers: Vec<_> = result
        .routes
        .iter()
        .map(|route| route.segments[0].layer)
        .collect();
    assert_ne!(layers[0], layers[1]);
}

#[test]
fn surface_pads_force_a_detour_or_vias_and_stay_legal() {
    let mut builder = Builder::new(20.0, 20.0, 2);
    builder.net("H", &[([2.0, 10.0], 0b01), ([18.0, 10.0], 0b01)]);
    builder.net("V", &[([10.0, 2.0], 0b01), ([10.0, 18.0], 0b01)]);
    let result = route(&builder.board, &config());
    assert_eq!(result.status, vec![NetStatus::Routed, NetStatus::Routed]);
    let violations = verify(&builder.board, &result.routes);
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn negotiation_shares_a_narrow_channel_fairly() {
    // A wall with one gap wide enough for exactly two traces, three nets
    // wanting through on a single layer: one must fail, two must succeed.
    let mut builder = Builder::new(30.0, 20.0, 1);
    let gap = 2.0 * 0.25 + 3.0 * 0.2 + 0.12;
    for (y0, y1) in [(0.0, 10.0 - gap / 2.0), (10.0 + gap / 2.0, 20.0)] {
        builder.board.obstacles.push(Obstacle {
            shape: Shape::rectangle([15.0, (y0 + y1) / 2.0], [0.5, (y1 - y0) / 2.0], 0.0),
            layers: 0b1,
            kind: ObstacleKind::Keepout,
            net: None,
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: "wall".into(),
        });
    }
    for (index, y) in [6.0, 10.0, 14.0].iter().enumerate() {
        builder.net(&format!("N{index}"), &[([3.0, *y], 0b1), ([27.0, *y], 0b1)]);
    }
    let result = route(&builder.board, &config());
    let routed = result
        .status
        .iter()
        .filter(|status| **status == NetStatus::Routed)
        .count();
    assert_eq!(routed, 2, "{:?}", result.status);
    let violations = verify(&builder.board, &result.routes);
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn multi_terminal_net_forms_one_tree() {
    let mut builder = Builder::new(30.0, 30.0, 2);
    builder.net(
        "GND",
        &[
            ([3.0, 3.0], 0b11),
            ([27.0, 3.0], 0b11),
            ([15.0, 27.0], 0b11),
            ([15.0, 15.0], 0b01),
        ],
    );
    builder.net("X", &[([3.0, 15.0], 0b01), ([27.0, 15.0], 0b01)]);
    let result = route(&builder.board, &config());
    assert_eq!(result.status, vec![NetStatus::Routed, NetStatus::Routed]);
    let violations = verify(&builder.board, &result.routes);
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn pour_connects_pads_with_stub_vias_only() {
    // A ground pour on the back. Through-hole ground pads touch it and need
    // nothing; front-only ground pads need one short stub and a via each.
    let mut builder = Builder::new(30.0, 20.0, 2);
    builder.net(
        "GND",
        &[
            ([3.0, 3.0], 0b11),
            ([27.0, 17.0], 0b11),
            ([10.0, 10.0], 0b01),
            ([20.0, 10.0], 0b01),
        ],
    );
    builder.net("SIG", &[([3.0, 10.0], 0b01), ([27.0, 10.0], 0b01)]);
    builder.board.planes.push(Plane {
        net: 0,
        class: 0,
        layer: 1,
        polygon: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
        excluded: Vec::new(),
        above: Vec::new(),
        connect: true,
        thermal_reach: 0.0,
        thermal_gap: 0.0,
        exclusive: false,
        solid: false,
        hatch: None,
    });
    let result = route(&builder.board, &config());
    assert_eq!(result.status, vec![NetStatus::Routed, NetStatus::Routed]);
    let violations = verify(&builder.board, &result.routes);
    assert!(violations.is_empty(), "{violations:?}");
    let ground = &result.routes[0];
    assert_eq!(ground.vias.len(), 2);
    let length: f64 = ground
        .segments
        .iter()
        .map(|s| ((s.start[0] - s.end[0]).powi(2) + (s.start[1] - s.end[1]).powi(2)).sqrt())
        .sum();
    assert!(length < 6.0, "ground stubs are {length} mm long");
}

#[test]
fn plane_skeleton_is_trimmed_back_to_what_the_pour_cannot_provide() {
    // Same board, routed with the skeleton: the fixed ground tree is routed
    // first, but every part of it that lies in solid pour is trimmed again,
    // so the result is the same two stub vias.
    let mut builder = Builder::new(30.0, 20.0, 2);
    builder.net(
        "GND",
        &[
            ([3.0, 3.0], 0b11),
            ([27.0, 17.0], 0b11),
            ([10.0, 10.0], 0b01),
            ([20.0, 10.0], 0b01),
        ],
    );
    builder.net("SIG", &[([3.0, 10.0], 0b01), ([27.0, 10.0], 0b01)]);
    builder.board.planes.push(Plane {
        net: 0,
        class: 0,
        layer: 1,
        polygon: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
        excluded: Vec::new(),
        above: Vec::new(),
        connect: true,
        thermal_reach: 0.0,
        thermal_gap: 0.0,
        exclusive: false,
        solid: false,
        hatch: None,
    });
    let config = Config {
        plane_skeleton: true,
        ..config()
    };
    let result = route(&builder.board, &config);
    assert_eq!(result.status, vec![NetStatus::Routed, NetStatus::Routed]);
    let violations = verify(&builder.board, &result.routes);
    assert!(violations.is_empty(), "{violations:?}");
    let ground = &result.routes[0];
    assert_eq!(ground.vias.len(), 2);
    let length: f64 = ground
        .segments
        .iter()
        .map(|s| ((s.start[0] - s.end[0]).powi(2) + (s.start[1] - s.end[1]).powi(2)).sqrt())
        .sum();
    assert!(length < 6.0, "ground stubs are {length} mm long");
}

#[test]
fn incremental_update_reroutes_only_what_a_change_touches() {
    let mut builder = Builder::new(40.0, 20.0, 2);
    builder.net("A", &[([3.0, 5.0], 0b01), ([37.0, 5.0], 0b01)]);
    builder.net("B", &[([3.0, 15.0], 0b01), ([37.0, 15.0], 0b01)]);
    let config = config();
    let mut router = pcb_router::router::Router::new(&builder.board, &config);
    let first = router.run_in_place();
    assert_eq!(first.status, vec![NetStatus::Routed, NetStatus::Routed]);

    // A keepout appears across net A's straight path; net B is untouched.
    let mut changed = builder.board.clone();
    changed.obstacles.push(Obstacle {
        shape: Shape::rectangle([20.0, 5.0], [0.5, 3.0], 0.0),
        layers: 0b11,
        kind: ObstacleKind::Keepout,
        net: None,
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks: true,
        blocks_vias: true,
        label: "wall".into(),
    });
    let rerouted = router.update(&changed).unwrap();
    assert_eq!(rerouted, 1);
    let second = router.reroute(true);
    assert_eq!(second.status, vec![NetStatus::Routed, NetStatus::Routed]);
    assert!(verify(&changed, &second.routes).is_empty());
    assert_eq!(second.routes[1].segments, first.routes[1].segments);
}

#[test]
fn a_local_reroute_routes_only_the_ripped_nets_against_the_rest() {
    let mut builder = Builder::new(40.0, 20.0, 2);
    builder.net("A", &[([3.0, 5.0], 0b01), ([37.0, 5.0], 0b01)]);
    builder.net("B", &[([3.0, 15.0], 0b01), ([37.0, 15.0], 0b01)]);
    let config = config();
    let mut router = pcb_router::router::Router::new(&builder.board, &config);
    let first = router.run_in_place();
    assert_eq!(first.status, vec![NetStatus::Routed, NetStatus::Routed]);
    // A wall across net A's path: A is ripped up and routed again around
    // it; B keeps its copper.
    let mut changed = builder.board.clone();
    changed.obstacles.push(Obstacle {
        shape: Shape::rectangle([20.0, 5.0], [0.5, 3.0], 0.0),
        layers: 0b11,
        kind: ObstacleKind::Keepout,
        net: None,
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks: true,
        blocks_vias: true,
        label: "wall".into(),
    });
    let ripped = router.update_nets(&changed).unwrap();
    assert_eq!(ripped, vec![0]);
    assert_eq!(router.board_open_terminals(), 2);
    assert_eq!(router.unconnected(), vec![(0, 0), (0, 1)]);
    assert_eq!(router.local_reroute(&ripped, 60.0), 0);
    let second = router.finish_as_is();
    assert_eq!(second.status, vec![NetStatus::Routed, NetStatus::Routed]);
    assert!(verify(&changed, &second.routes).is_empty());
    assert_eq!(second.routes[1].segments, first.routes[1].segments);
}

#[test]
fn a_moved_pad_leaves_the_rest_of_its_net_routed() {
    // Three pads of net A in a row; the last one moves down. The branch
    // between the first two stays, the moved pad is joined to it again.
    let mut builder = Builder::new(40.0, 20.0, 2);
    builder.net(
        "A",
        &[([3.0, 5.0], 0b01), ([20.0, 5.0], 0b01), ([37.0, 5.0], 0b01)],
    );
    let config = config();
    let mut router = pcb_router::router::Router::new(&builder.board, &config);
    let first = router.run_in_place();
    assert_eq!(first.status, vec![NetStatus::Routed]);
    let mut moved = builder.board.clone();
    moved.obstacles[2].shape = Shape::Circle {
        center: [37.0, 12.0],
        radius: 0.5,
    };
    moved.nets[0].terminals[2].anchor = [37.0, 12.0];
    assert_eq!(router.update_nets(&moved).unwrap(), vec![0]);
    assert_eq!(router.local_reroute(&[0], 60.0), 0);
    let second = router.finish_as_is();
    assert_eq!(second.status, vec![NetStatus::Routed]);
    assert!(verify(&moved, &second.routes).is_empty());
    // The copper between the first two pads is where it was.
    let between = |routes: &[pcb_router::NetRoute]| {
        routes[0]
            .segments
            .iter()
            .filter(|segment| segment.start[0].max(segment.end[0]) <= 20.5)
            .count()
    };
    assert!(between(&first.routes) > 0);
    assert_eq!(between(&second.routes), between(&first.routes));
}

#[test]
fn a_pad_straddling_the_board_edge_is_reached_from_inside() {
    // A connector shell pad hangs over the outline (the designer's choice,
    // KiCad flags the pad itself). The track that reaches it must still keep
    // the edge clearance: it starts inside the pad, away from the edge.
    // Only 0.71 mm of the pad is inside the board: no lattice node in it
    // keeps the 0.5 mm edge clearance with a 0.25 mm track, so the pad is
    // reached through an escape stub.
    let mut builder = Builder::new(20.0, 20.0, 2);
    builder.board.edge_clearance = 0.5;
    builder.net("lv", &[([10.0, 10.0], 0b11), ([10.0, 12.0], 0b11)]);
    let net = builder.board.nets.len() as u32 - 1;
    builder.board.obstacles.push(Obstacle {
        shape: Shape::Polygon {
            points: vec![[-1.25, 9.4], [0.71, 9.4], [0.71, 10.6], [-1.25, 10.6]],
        },
        layers: 0b11,
        kind: ObstacleKind::Copper,
        net: Some(net),
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks: true,
        blocks_vias: true,
        label: "shell".into(),
    });
    builder.board.nets[net as usize].terminals.push(Terminal {
        anchor: [-0.25, 10.0],
        layers: 0b11,
        pad: builder.board.obstacles.len() - 1,
        contact: None,
        label: "shell".into(),
    });
    let result = route(&builder.board, &config());
    assert_eq!(result.status, vec![NetStatus::Routed]);
    let violations = verify(&builder.board, &result.routes);
    assert!(violations.is_empty(), "{violations:?}");
    // The stub into the shell pad is real copper of the pad's net.
    assert!(
        result.routes[0]
            .segments
            .iter()
            .any(|segment| segment.start[0] < 0.71 || segment.end[0] < 0.71)
    );
}

#[test]
fn a_gap_of_exactly_two_clearances_holds_two_tracks_at_exactly_their_clearance() {
    // A wall with a gap that holds two 0.25 mm tracks only if they sit at
    // exactly the 0.2 mm clearance from each other and from the wall: the
    // lattice is 0.1 mm, so the tracks sit 0.45 mm apart. KiCad accepts a
    // clearance met exactly; the stamps must too.
    let mut builder = Builder::new(30.0, 20.0, 1);
    let gap = 2.0 * 0.25 + 3.0 * 0.2 + 0.05;
    let (bottom, top) = (10.0 - gap / 2.0, 10.0 + gap / 2.0);
    // Track centrelines at 10 - 0.225 and 10 + 0.225 keep exactly 0.2 from
    // each other; the wall sits 0.2 (+ 0.025) from their edges.
    for (y0, y1) in [(0.0, bottom), (top, 20.0)] {
        builder.board.obstacles.push(Obstacle {
            shape: Shape::rectangle([15.0, (y0 + y1) / 2.0], [0.5, (y1 - y0) / 2.0], 0.0),
            layers: 0b1,
            kind: ObstacleKind::Keepout,
            net: None,
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: "wall".into(),
        });
    }
    builder.net("A", &[([3.0, 9.55], 0b1), ([27.0, 9.55], 0b1)]);
    builder.net("B", &[([3.0, 10.45], 0b1), ([27.0, 10.45], 0b1)]);
    let mut config = config();
    config.pitches = vec![0.1];
    let result = route(&builder.board, &config);
    assert_eq!(
        result.status,
        vec![NetStatus::Routed, NetStatus::Routed],
        "{:?}",
        result.status
    );
    let violations = verify(&builder.board, &result.routes);
    assert!(violations.is_empty(), "{violations:?}");
    // Both went through the gap, not around (there is no way around).
    for route in &result.routes {
        assert!(
            route
                .segments
                .iter()
                .any(|segment| segment.start[0].min(segment.end[0]) < 15.0
                    && segment.start[0].max(segment.end[0]) > 15.0
                    && (segment.start[1] - 10.0).abs() < gap)
        );
    }
}

#[test]
fn diagonal_steps_keep_their_clearance_from_nodes_beside_them() {
    // Many nets crossing a small board on one layer diagonally past each
    // other's pads: whatever the router chooses, the exact verifier must
    // find nothing (the stamps and the corner-cell rule are exact).
    let mut builder = Builder::new(24.0, 24.0, 1);
    for index in 0..6 {
        let y = 3.0 + index as f64 * 3.3;
        builder.net(
            &format!("D{index}"),
            &[([2.0, y], 0b1), ([22.0, 24.0 - y], 0b1)],
        );
    }
    let result = route(&builder.board, &config());
    let violations = verify(&builder.board, &result.routes);
    assert!(violations.is_empty(), "{violations:?}");
    assert!(
        result
            .status
            .iter()
            .filter(|status| **status == NetStatus::Routed)
            .count()
            >= 4,
        "{:?}",
        result.status
    );
}

#[test]
fn parallel_diagonal_tracks_in_a_channel_keep_their_clearance() {
    // Two pad areas joined by a diagonal channel of the board wide enough
    // for three tracks at exactly their clearance (1.15 mm plus the edge
    // clearance on both sides). On a 0.127 mm lattice, diagonal tracks
    // five lattice steps apart are 0.449 mm apart: 1 µm too close. The
    // router must never take that spacing (it settles for two tracks
    // here), and whatever it routes verifies clean.
    let mut builder = Builder::new(30.0, 30.0, 1);
    let half = (1.15 + 2.0 * 0.2) / 2.0 * std::f64::consts::SQRT_2;
    builder.board.outline = vec![
        [0.0, 0.0],
        [6.0, 0.0],
        [6.0, 6.0 - half],
        [24.0 + half, 24.0],
        [30.0, 24.0],
        [30.0, 30.0],
        [24.0, 30.0],
        [24.0, 24.0 + half],
        [6.0 - half, 6.0],
        [0.0, 6.0],
    ];
    for index in 0..3 {
        let offset = index as f64 * 1.5;
        builder.net(
            &format!("P{index}"),
            &[([1.5, 1.5 + offset], 0b1), ([28.5, 25.5 + offset], 0b1)],
        );
    }
    let mut config = config();
    config.pitches = vec![0.127];
    let result = route(&builder.board, &config);
    let violations = verify(&builder.board, &result.routes);
    assert!(violations.is_empty(), "{violations:?}");
    let routed = result
        .status
        .iter()
        .filter(|status| **status == NetStatus::Routed)
        .count();
    assert!(routed >= 2, "{:?}", result.status);
}

/// A row of `count` fine-pitch pads (`width` x 0.875 mm, `pitch` apart,
/// centres from `first` along x), each its own net to a round pad 6 mm
/// above, 1.5 mm apart: the tracks fan out of the row. One layer, 0.2 mm
/// tracks and clearance.
fn fine_pitch_fanout(first: f64, pitch: f64, width: f64, count: usize) -> (Board, Config) {
    fine_pitch_row(first, pitch, width, count, 1.5)
}

/// `fine_pitch_fanout` with the round pads `spread` apart.
fn fine_pitch_row(
    first: f64,
    pitch: f64,
    width: f64,
    count: usize,
    spread: f64,
) -> (Board, Config) {
    let mut builder = Builder::new(30.0, 24.0, 1);
    builder.board.classes[0].trace_width = 0.2;
    builder.board.classes[0].clearance = 0.2;
    builder.board.neck_width = 0.2;
    builder.board.edge_clearance = 0.2;
    for index in 0..count {
        let x = first + index as f64 * pitch;
        let name = format!("N{index}");
        let far = 15.0 + (index as f64 - (count as f64 - 1.0) / 2.0) * spread;
        builder.net(&name, &[([far, 4.0], 0b1)]);
        let net = builder.board.nets.len() as u32 - 1;
        let target = builder.board.obstacles.len() - 1;
        builder.board.obstacles[target].shape = Shape::Circle {
            center: [far, 4.0],
            radius: (spread / 2.0 - 0.2).min(0.5),
        };
        builder.board.obstacles.push(Obstacle {
            shape: Shape::rectangle([x, 10.0], [width / 2.0, 0.4375], 0.0),
            layers: 0b1,
            kind: ObstacleKind::Copper,
            net: Some(net),
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: format!("U1.{index}"),
        });
        builder.board.nets[net as usize].terminals.push(Terminal {
            anchor: [x, 10.0],
            layers: 0b1,
            pad: builder.board.obstacles.len() - 1,
            contact: None,
            label: format!("U1.{index}"),
        });
    }
    let mut config = config();
    config.pitches = vec![0.1];
    (builder.board, config)
}

#[test]
fn a_fine_pitch_row_at_exactly_its_clearance_fans_out() {
    // RP2040's QFN-56: 0.2 mm pads 0.4 mm apart with 0.2 mm tracks and
    // clearance, the centre lines on the lattice. A track leaves each pad
    // straight along its centre, exactly 0.2 mm from both neighbours, so
    // every pad leaves by a stub along its lattice column. Stamped with
    // the snapping margin of an off-lattice stub, each stub covered its
    // neighbours' and the row never came out of conflict
    // (jfedor2/hid-remapper: 27 of 120 connections open at 30 s).
    let (board, config) = fine_pitch_fanout(12.9, 0.4, 0.2, 8);
    let result = route(&board, &config);
    assert!(
        result
            .status
            .iter()
            .all(|status| *status == NetStatus::Routed),
        "{:?}",
        result.status
    );
    let violations = verify(&board, &result.routes);
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn a_fine_pitch_row_off_the_lattice_routes_straight_and_clean() {
    // An USON's 0.3 mm pads 0.5 mm apart, their centres half a pitch off
    // the 0.1 mm lattice: the inner pads hold no legal node (both nodes
    // beside a centre keep exactly the clearance to a neighbour, which the
    // static maps' margin refuses), so each leaves by an oblique stub to a
    // node beside its centre line, and the tracks run straight on at the
    // pads' pitch. The stubs' measured footprints (`stub_footprint`) must
    // keep every clearance: the result verifies clean.
    let (board, config) = fine_pitch_row(13.75, 0.5, 0.3, 6, 0.5);
    let result = route(&board, &config);
    assert!(
        result
            .status
            .iter()
            .all(|status| *status == NetStatus::Routed),
        "{:?}",
        result.status
    );
    let violations = verify(&board, &result.routes);
    assert!(violations.is_empty(), "{violations:?}");
}

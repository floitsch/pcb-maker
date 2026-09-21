// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use pcb_router::{
    Plane,
    Board, Config, Net, NetStatus, Obstacle, ObstacleKind, RuleClass, Shape, Terminal, route,
    verify,
};

fn class() -> RuleClass {
    RuleClass {
        trace_width: 0.25,
        clearance: 0.2,
        via_diameter: 0.7,
        via_drill: 0.3,
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
                obstacles: Vec::new(),
                nets: Vec::new(),
                planes: Vec::new(),
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
                blocks_tracks: true,
                blocks_vias: true,
                label: format!("{name}.{index}"),
            });
            terminals.push(Terminal {
                anchor: *at,
                layers: *layers,
                pad: self.board.obstacles.len() - 1,
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
        result.routes.iter().map(|route| route.vias.len()).sum::<usize>(),
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
            blocks_tracks: true,
            blocks_vias: true,
            label: "wall".into(),
        });
    }
    for (index, y) in [6.0, 10.0, 14.0].iter().enumerate() {
        builder.net(
            &format!("N{index}"),
            &[([3.0, *y], 0b1), ([27.0, *y], 0b1)],
        );
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


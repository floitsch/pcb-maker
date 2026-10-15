// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The orientation search (`Config::orientation_search`): the placement
//! does not depend on the angles the parts start at, and a part turns to
//! the angle that shortens its wires.

use pcb_placer::orient::{angle_key, canonical_start, sort_angles};
use pcb_placer::{Component, Config, Pin, Pose, Problem, Side, place};

fn component(name: &str, size: [f64; 2], pins: Vec<Pin>, fixed: bool) -> Component {
    Component {
        name: name.into(),
        body_center: [0.0, 0.0],
        body_size: size,
        round: false,
        halo: 0.0,
        pins,
        side: if fixed { Side::Both } else { Side::Front },
        fixed,
        angle_options: if fixed { vec![0.0] } else { vec![0.0, 90.0, 180.0, 270.0] },
        far_side: Vec::new(),
        far_copper: Vec::new(),
        far_courtyard: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
    }
}

/// Two fixed connectors and a chain of two-pin parts and three-pin parts
/// (a transistor's pins on two sides) between them, every part at `angle`
/// of its index with its options listed from that angle on, as the board
/// lowering lists them from a footprint's angle.
fn board(angle: impl Fn(usize) -> f64) -> Problem {
    let parts = 10;
    let mut components = vec![
        component("J1", [4.0, 10.0], vec![Pin { offset: [0.0, 0.0], net: 0 }], true),
        component("J2", [4.0, 10.0], vec![Pin { offset: [0.0, 0.0], net: parts }], true),
    ];
    let mut poses = vec![
        Pose { position: [4.0, 25.0], angle: 0.0 },
        Pose { position: [56.0, 25.0], angle: 0.0 },
    ];
    let mut net_count = parts + 1;
    for index in 0..parts {
        let mut pins = vec![Pin { offset: [-2.0, 0.0], net: index }, Pin { offset: [2.0, 0.0], net: index + 1 }];
        if index % 3 == 1 {
            // A third pin off the axis, on a net of its own with the next part.
            pins.push(Pin { offset: [0.0, 1.0], net: net_count });
            net_count += 1;
        }
        let mut part = component(&format!("U{index}"), [6.0, 3.0], pins, false);
        let start = angle(index);
        part.angle_options = (0..4).map(|quarter| (start + 90.0 * quarter as f64).rem_euclid(360.0)).collect();
        components.push(part);
        poses.push(Pose { position: [30.0, 25.0], angle: start });
    }
    // The extra nets join the part after.
    for index in 0..parts {
        if index % 3 == 1 && index + 1 < parts {
            let net = components[index + 2].pins[2].net;
            components[index + 3].pins.push(Pin { offset: [0.0, -1.0], net });
        }
    }
    Problem {
        outline: vec![[0.0, 0.0], [60.0, 0.0], [60.0, 50.0], [0.0, 50.0]],
        components,
        net_weights: vec![1.0; net_count],
        poses,
        spacing: 0.5,
        grid: 0.5,
        edge_margin: 0.0,
        min_spacing: 0.0,
        far_side_pads_only: false,
        shared_halos: false,
        pieces: Vec::new(),
        constraints: Default::default(),
    }
}

/// The poses with angles in the canonical range, for comparison.
fn normalized(poses: &[Pose]) -> Vec<([f64; 2], f64)> {
    poses.iter().map(|pose| (pose.position, angle_key(pose.angle))).collect()
}

#[test]
fn angle_options_sort_into_one_order() {
    let mut options = vec![270.0, 0.0, 90.0, 180.0, -90.0, 360.0];
    sort_angles(&mut options);
    assert_eq!(options.iter().map(|angle| angle_key(*angle)).collect::<Vec<_>>(), vec![0.0, 90.0, 180.0, 270.0]);
    let mut odd = vec![30.0, 120.0, 210.0, 300.0, 0.0, 90.0, 180.0, 270.0];
    sort_angles(&mut odd);
    assert_eq!(odd, vec![0.0, 30.0, 90.0, 120.0, 180.0, 210.0, 270.0, 300.0]);
}

#[test]
fn the_canonical_start_turns_movable_parts_only() {
    let mut problem = board(|index| 90.0 * (index % 4) as f64);
    problem.components[2].angle_options = vec![270.0];
    problem.poses[2].angle = 270.0;
    let turned = canonical_start(&mut problem);
    // Parts at 90, 180 and 270 turn to 0; the one held at 270 stays, the
    // fixed connectors too.
    assert_eq!(turned, 7);
    assert_eq!(problem.poses[2].angle, 270.0);
    for index in 3..problem.components.len() {
        assert_eq!(problem.poses[index].angle, 0.0);
        assert_eq!(problem.components[index].angle_options, vec![0.0, 90.0, 180.0, 270.0]);
    }
    // Turned about the body centre (here the origin).
    assert_eq!(problem.poses[4].position, [30.0, 25.0]);
}

#[test]
fn the_placement_does_not_depend_on_the_starting_angles() {
    let at_zero = board(|_| 0.0);
    let turned = board(|index| 90.0 * ((index * 7 + 1) % 4) as f64);
    let config = Config::new();
    let first = place(&at_zero, &config);
    let second = place(&turned, &config);
    assert!(first.unplaced.is_empty() && first.illegal.is_empty());
    assert_eq!(normalized(&first.poses), normalized(&second.poses));
    assert_eq!(first.wirelength_final, second.wirelength_final);
    // Deterministic: the same start places the same way again.
    assert_eq!(first.poses, place(&at_zero, &config).poses);
    // Without the canonical start the angles the parts start at decide.
    let off = Config { orientation_search: false, ..Config::new() };
    assert_ne!(normalized(&place(&at_zero, &off).poses), normalized(&place(&turned, &off).poses));
}

#[test]
fn a_part_turns_to_the_angle_that_shortens_its_wires() {
    // One part between two connectors whose nets cross it: its first pin
    // (left in its own frame) belongs to the right connector's net, so it
    // shortens its wires turned by a half.
    for start in [0.0, 90.0, 180.0, 270.0] {
        let mut problem = board(|_| 0.0);
        problem.components.truncate(3);
        problem.poses.truncate(3);
        problem.components[2].pins = vec![Pin { offset: [-2.0, 0.0], net: 1 }, Pin { offset: [2.0, 0.0], net: 0 }];
        problem.components[1].pins[0].net = 1;
        problem.net_weights = vec![1.0; 2];
        problem.components[2].angle_options =
            (0..4).map(|quarter| (start + 90.0 * quarter as f64).rem_euclid(360.0)).collect();
        problem.poses[2].angle = start;
        let placement = place(&problem, &Config::new());
        assert_eq!(angle_key(placement.poses[2].angle), 180.0, "from {start}");
        assert!(placement.illegal.is_empty());
    }
}

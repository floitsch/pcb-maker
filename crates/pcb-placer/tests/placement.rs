// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use pcb_placer::{Component, Config, Pin, Pose, Problem, Side, place};

/// A chain of two-pin parts plus two fixed connectors at opposite ends.
fn chain(parts: usize) -> Problem {
    let mut components = Vec::new();
    let mut poses = Vec::new();
    let connector = |name: &str, net: usize| Component {
        name: name.into(),
        body_center: [0.0, 0.0],
        body_size: [4.0, 10.0],
        round: false,
        pins: vec![Pin {
            offset: [0.0, 0.0],
            net,
        }],
        side: Side::Both,
        fixed: true,
        angle_options: vec![0.0],
    };
    components.push(connector("J1", 0));
    poses.push(Pose {
        position: [4.0, 25.0],
        angle: 0.0,
    });
    components.push(connector("J2", parts));
    poses.push(Pose {
        position: [76.0, 25.0],
        angle: 0.0,
    });
    for index in 0..parts {
        components.push(Component {
            name: format!("R{index}"),
            body_center: [0.0, 0.0],
            body_size: [6.0, 3.0],
            round: false,
            pins: vec![
                Pin {
                    offset: [-2.0, 0.0],
                    net: index,
                },
                Pin {
                    offset: [2.0, 0.0],
                    net: index + 1,
                },
            ],
            side: Side::Front,
            fixed: false,
            angle_options: vec![0.0, 90.0, 180.0, 270.0],
        });
        // A deliberately bad start: everything piled in one corner.
        poses.push(Pose {
            position: [10.0, 45.0],
            angle: 0.0,
        });
    }
    Problem {
        outline: vec![[0.0, 0.0], [80.0, 0.0], [80.0, 50.0], [0.0, 50.0]],
        components,
        net_weights: vec![1.0; parts + 1],
        poses,
        spacing: 0.5,
        grid: 0.5,
    }
}

#[test]
fn chain_is_spread_legally_between_its_connectors() {
    let problem = chain(12);
    let placement = place(&problem, &Config::new());
    assert!(placement.unplaced.is_empty());
    assert!(placement.illegal.is_empty(), "{:?}", placement.illegal);
    // An ideal chain spans the 72 mm between the connectors once; allow
    // slack for body sizes and legalization.
    assert!(
        placement.wirelength_final < 130.0,
        "wirelength {}",
        placement.wirelength_final
    );
    // The fixed connectors did not move.
    assert_eq!(placement.poses[0], problem.poses[0]);
    assert_eq!(placement.poses[1], problem.poses[1]);
}

#[test]
fn dense_board_still_becomes_legal() {
    // 40 parts of 6x3 mm plus spacing on a 40x30 board: ~75 % utilization.
    let mut problem = chain(40);
    problem.outline = vec![[0.0, 0.0], [44.0, 0.0], [44.0, 34.0], [0.0, 34.0]];
    problem.poses[0].position = [3.0, 17.0];
    problem.poses[1].position = [41.0, 17.0];
    for pose in &mut problem.poses[2..] {
        pose.position = [22.0, 17.0];
    }
    let placement = place(&problem, &Config::new());
    assert!(placement.unplaced.is_empty(), "{:?}", placement.unplaced);
    assert!(placement.illegal.is_empty(), "{:?}", placement.illegal);
}

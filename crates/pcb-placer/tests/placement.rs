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
        halo: 0.0,
        pins: vec![Pin {
            offset: [0.0, 0.0],
            net,
        }],
        side: Side::Both,
        fixed: true,
        angle_options: vec![0.0],
        far_side: Vec::new(),
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
            halo: 0.0,
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
            far_side: Vec::new(),
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
        edge_margin: 0.0,
        constraints: Default::default(),
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

#[test]
fn edge_and_region_constraints_are_kept() {
    use pcb_placer::constraints::Edge;
    let mut problem = chain(12);
    // R5 (index 7) must touch the top edge, R8 (index 10) stay in the
    // bottom-right corner box.
    problem.constraints.edges.push((7, Edge::Top, 0.5));
    problem.constraints.regions.push((10, [60.0, 35.0, 80.0, 50.0]));
    problem.constraints.relation_weight = 4.0;
    let placement = place(&problem, &Config::new());
    assert!(placement.illegal.is_empty(), "{:?}", placement.illegal);
    assert_eq!(placement.constraints.len(), 2);
    for status in &placement.constraints {
        assert!(status.satisfied, "{status:?}");
    }
    let top = problem.components[7].center(placement.poses[7])[1]
        - problem.components[7].half_extent(placement.poses[7].angle)[1];
    assert!(top <= 0.5 + 1.0e-6, "top {top}");
}

#[test]
fn relations_pull_parts_together_against_the_netlist() {
    use pcb_placer::constraints::{Anchor, Edge, Relation};
    let mut problem = chain(12);
    // R0 and R11 sit at opposite ends of the chain; ask for R11 right next
    // to R0's second pin, and R6 directly below R1.
    problem.constraints.relations.push(Relation::Near {
        part: 13,
        anchor: Anchor::Point(2, [2.0, 0.0]),
        max: 2.0,
    });
    problem.constraints.relations.push(Relation::Beside {
        part: 8,
        anchor: 3,
        side: Edge::Bottom,
        max_gap: 3.0,
    });
    problem.constraints.relation_weight = 4.0;
    let placement = place(&problem, &Config::new());
    assert!(placement.illegal.is_empty(), "{:?}", placement.illegal);
    for status in &placement.constraints {
        assert!(status.satisfied, "{status:?}");
    }
}

#[test]
fn a_through_hole_part_leaves_the_far_side_free_but_for_its_holes() {
    use pcb_placer::legal::is_legal;
    let part = |side: Side, far_side: Vec<[f64; 4]>| Component {
        name: String::new(),
        body_center: [0.0, 0.0],
        body_size: [10.0, 10.0],
        round: false,
        halo: 0.0,
        pins: Vec::new(),
        side,
        fixed: false,
        angle_options: vec![0.0],
        far_side,
    };
    let mut problem = chain(1);
    problem.components = vec![
        // A through-hole part on the front with one 2 mm hole at its centre.
        part(Side::Front, vec![[-1.0, -1.0, 1.0, 1.0]]),
        // An SMD part on the back.
        Component { body_size: [3.0, 3.0], ..part(Side::Back, Vec::new()) },
        // Artwork: occupies nothing.
        Component { body_size: [20.0, 20.0], ..part(Side::Neither, Vec::new()) },
    ];
    problem.poses = vec![Pose { position: [40.0, 25.0], angle: 0.0 }; 3];
    problem.spacing = 0.2;
    // Under the body but away from the hole: legal.
    let beside = Pose { position: [44.0, 25.0], angle: 0.0 };
    assert!(is_legal(&problem, &problem.poses, 1, beside, 0..3));
    // On the hole: not legal.
    let on_hole = Pose { position: [40.5, 25.0], angle: 0.0 };
    assert!(!is_legal(&problem, &problem.poses, 1, on_hole, 0..3));
}

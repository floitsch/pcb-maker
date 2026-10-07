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
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
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
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: Vec::new(),
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
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
        min_spacing: 0.0,
        far_side_pads_only: false,
        pieces: Vec::new(),
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
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
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

#[test]
fn parts_sit_inside_a_hollow_part_but_off_its_pads() {
    use pcb_placer::legal::is_legal;
    let mut problem = chain(1);
    let part = |size: f64, fixed: bool| Component {
        name: String::new(),
        body_center: [0.0, 0.0],
        body_size: [size, size],
        round: false,
        halo: 0.0,
        pins: Vec::new(),
        side: Side::Front,
        fixed,
        angle_options: vec![0.0],
        far_side: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
    };
    // A shield outline over the whole board with one header pad at its
    // left end.
    let shield = Component { hollow: vec![[-9.0, -1.0, -7.0, 1.0]], ..part(20.0, true) };
    problem.components = vec![shield, part(2.0, false)];
    problem.poses = vec![
        Pose { position: [25.0, 25.0], angle: 0.0 },
        Pose { position: [25.0, 25.0], angle: 0.0 },
    ];
    let legal = |problem: &Problem, x: f64| {
        is_legal(problem, &problem.poses, 1, Pose { position: [x, 25.0], angle: 0.0 }, 0..2)
    };
    // Inside the outline is fine; on the pad is not.
    assert!(legal(&problem, 25.0));
    assert!(!legal(&problem, 17.0));
    // A part with holes may not sit inside it (KiCad's pth_inside_courtyard).
    problem.components[1].far_side = vec![[-0.5, -0.5, 0.5, 0.5]];
    assert!(!legal(&problem, 25.0));
    problem.components[1].far_side.clear();
    // Without the hollow boxes the body blocks.
    problem.components[0].hollow.clear();
    assert!(!legal(&problem, 25.0));
}

#[test]
fn tight_bodies_are_the_last_resort() {
    // Two parts with 4 mm courtyards around 2 mm bodies on a 7 x 4 mm board:
    // the courtyards do not fit side by side, the bodies do.
    let part = |net: usize| Component {
        name: String::new(),
        body_center: [0.0, 0.0],
        body_size: [4.0, 4.0],
        round: false,
        halo: 0.0,
        pins: vec![Pin { offset: [0.0, 0.0], net }],
        side: Side::Front,
        fixed: false,
        angle_options: vec![0.0],
        far_side: Vec::new(),
        hollow: Vec::new(),
        tight: Some([-1.0, -1.0, 1.0, 1.0]),
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
    };
    let mut problem = Problem {
        outline: vec![[0.0, 0.0], [7.0, 0.0], [7.0, 4.0], [0.0, 4.0]],
        components: vec![part(0), part(0)],
        net_weights: vec![1.0],
        poses: vec![Pose { position: [3.5, 2.0], angle: 0.0 }; 2],
        spacing: 0.2,
        grid: 0.1,
        edge_margin: 0.0,
        min_spacing: 0.2,
        far_side_pads_only: false,
        pieces: Vec::new(),
        constraints: Default::default(),
    };
    let placement = place(&problem, &Config::new());
    assert!(placement.unplaced.is_empty() && placement.illegal.is_empty(), "{:?}", placement.unplaced);
    assert!(placement.relaxation.tight);
    // Without them, one part finds no place.
    for component in &mut problem.components {
        component.tight = None;
    }
    let placement = place(&problem, &Config::new());
    assert!(!placement.unplaced.is_empty() && !placement.relaxation.tight);
}

#[test]
fn a_body_may_reach_over_a_cutout_but_its_pads_keep_away() {
    use pcb_placer::legal::is_legal;
    let mut problem = chain(1);
    let part = |size: f64, side: Side, fixed: bool| Component {
        name: String::new(),
        body_center: [0.0, 0.0],
        body_size: [size, size],
        round: false,
        halo: 0.0,
        pins: Vec::new(),
        side,
        fixed,
        angle_options: vec![0.0],
        far_side: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
    };
    // A 2 mm hole in the board, and a 10 mm connector with a pad at its
    // left end.
    let cutout = Component { copper_only: true, ..part(2.0, Side::Both, true) };
    let connector = Component { pads: vec![[-5.0, -1.0, -3.0, 1.0]], ..part(10.0, Side::Front, false) };
    problem.components = vec![cutout, connector];
    problem.poses = vec![
        Pose { position: [25.0, 25.0], angle: 0.0 },
        Pose { position: [25.0, 25.0], angle: 0.0 },
    ];
    problem.edge_margin = 0.5;
    let legal = |problem: &Problem, x: f64| {
        is_legal(problem, &problem.poses, 1, Pose { position: [x, 25.0], angle: 0.0 }, 0..2)
    };
    // Body over the hole, pad 3 mm away: fine.
    assert!(legal(&problem, 23.0));
    // Pad within the edge margin of the hole: not.
    assert!(!legal(&problem, 28.5));
    // A solid obstacle blocks the body.
    problem.components[0].copper_only = false;
    assert!(!legal(&problem, 23.0));
}

#[test]
fn apart_keeps_connected_parts_away_from_each_other() {
    use pcb_placer::constraints::Relation;
    let part = || Component {
        name: String::new(),
        body_center: [0.0, 0.0],
        body_size: [4.0, 4.0],
        round: false,
        halo: 0.0,
        pins: vec![Pin { offset: [0.0, 0.0], net: 0 }],
        side: Side::Front,
        fixed: false,
        angle_options: vec![0.0],
        far_side: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
    };
    let mut problem = Problem {
        outline: vec![[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]],
        components: vec![part(), part()],
        net_weights: vec![1.0],
        poses: vec![Pose { position: [30.0, 20.0], angle: 0.0 }; 2],
        spacing: 0.5,
        grid: 0.5,
        edge_margin: 0.5,
        min_spacing: 0.2,
        far_side_pads_only: false,
        pieces: Vec::new(),
        constraints: Default::default(),
    };
    problem.constraints.relations.push(Relation::Apart { part: 0, anchor: 1, min: 15.0 });
    problem.constraints.relation_weight = 50.0;
    let placement = place(&problem, &Config::new());
    assert!(placement.unplaced.is_empty());
    let [a, b] = [placement.poses[0].position, placement.poses[1].position];
    let gap = ((a[0] - b[0]).abs() - 4.0).max(0.0).hypot(((a[1] - b[1]).abs() - 4.0).max(0.0));
    assert!(gap >= 15.0 - 0.1, "gap {gap}");
    assert!(placement.constraints.iter().all(|status| status.satisfied));
}

#[test]
fn parts_of_a_kind_are_turned_alike() {
    use pcb_placer::legal::align_orientations;
    let part = |net: usize| Component {
        name: String::new(),
        body_center: [0.0, 0.0],
        body_size: [3.0, 1.5],
        round: false,
        halo: 0.0,
        pins: vec![Pin { offset: [-1.0, 0.0], net }, Pin { offset: [1.0, 0.0], net: net + 1 }],
        side: Side::Front,
        fixed: false,
        angle_options: vec![0.0, 90.0, 180.0, 270.0],
        far_side: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
    };
    let problem = Problem {
        outline: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
        components: vec![part(0), part(2), part(4)],
        net_weights: vec![1.0; 6],
        poses: vec![
            Pose { position: [10.0, 10.0], angle: 0.0 },
            Pose { position: [20.0, 10.0], angle: 0.0 },
            Pose { position: [30.0, 10.0], angle: 90.0 },
        ],
        spacing: 0.5,
        grid: 0.5,
        edge_margin: 0.5,
        min_spacing: 0.2,
        far_side_pads_only: false,
        pieces: Vec::new(),
        constraints: Default::default(),
    };
    let mut poses = problem.poses.clone();
    assert_eq!(align_orientations(&problem, &mut poses, 0.5), 1);
    assert!(poses.iter().all(|pose| pose.angle == 0.0));
}

#[test]
fn a_part_held_at_an_edge_keeps_to_its_own_piece() {
    use pcb_placer::constraints::Edge;
    // Two 20 x 20 mm boards side by side, joined by a zero-width bridge
    // (as the KiCad adapter joins pieces). A part held at the right edge
    // and wired to a part on the left piece goes to that piece's right
    // edge; measured against both pieces' box it had to go to x = 50.
    let part = Component {
        name: "J1".into(),
        body_center: [0.0, 0.0],
        body_size: [4.0, 4.0],
        round: false,
        halo: 0.0,
        pins: vec![Pin { offset: [0.0, 0.0], net: 0 }],
        side: Side::Front,
        fixed: false,
        angle_options: vec![0.0],
        far_side: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
    };
    let mut problem = Problem {
        outline: vec![
            [0.0, 0.0],
            [20.0, 0.0],
            [30.0, 0.0],
            [50.0, 0.0],
            [50.0, 20.0],
            [30.0, 20.0],
            [30.0, 0.0],
            [20.0, 0.0],
            [20.0, 20.0],
            [0.0, 20.0],
        ],
        components: vec![part.clone(), Component { name: "U1".into(), fixed: true, ..part }],
        net_weights: vec![1.0],
        poses: vec![Pose { position: [8.0, 10.0], angle: 0.0 }, Pose { position: [5.0, 10.0], angle: 0.0 }],
        spacing: 0.2,
        grid: 0.1,
        edge_margin: 0.0,
        min_spacing: 0.2,
        far_side_pads_only: false,
        pieces: vec![[0.0, 0.0, 20.0, 20.0], [30.0, 0.0, 50.0, 20.0]],
        constraints: Default::default(),
    };
    problem.constraints.edges.push((0, Edge::Right, 0.5));
    let placement = place(&problem, &Config::new());
    assert!(placement.unplaced.is_empty() && placement.illegal.is_empty());
    let right = placement.poses[0].position[0] + 2.0;
    assert!((19.4..=20.0).contains(&right), "right side at {right}");
}

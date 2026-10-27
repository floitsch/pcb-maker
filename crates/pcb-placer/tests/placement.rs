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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
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
            hole_rings: Vec::new(),
            copper_graphics: Vec::new(),
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
        shared_halos: false,
        escape: None,
        crossings: None,
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
fn global_placement_starts_parts_where_a_start_says() {
    // A panel's copies: R0-R5 confined to the left half, R6-R11 to the
    // right; the right ones start at their region's centre instead of the
    // common start (the fixed pins' centroid and the board's centre, at
    // x = 40: clamped against the region's near border, where they piled
    // up on SmartSpin2k). One step of global placement leaves them near
    // where they started.
    let mut problem = chain(12);
    for index in 2..8 {
        problem
            .constraints
            .regions
            .push((index, [0.0, 0.0, 40.0, 50.0]));
    }
    for index in 8..14 {
        problem
            .constraints
            .regions
            .push((index, [40.0, 0.0, 80.0, 50.0]));
    }
    let mut config = Config::new().global;
    config.max_iterations = 1;
    let centre = |problem: &Problem, poses: &[Pose], index: usize| {
        problem.components[index].center(poses[index])
    };
    let common = pcb_placer::global::global_place(&problem, &config);
    for index in 8..14 {
        problem.constraints.starts.push((index, [60.0, 25.0]));
    }
    let started = pcb_placer::global::global_place(&problem, &config);
    for index in 8..14 {
        let [x, y] = centre(&problem, &started.poses, index);
        assert!(
            (x - 60.0).abs() < 6.0 && (y - 25.0).abs() < 6.0,
            "R{} at {x}, {y}",
            index - 2
        );
        assert!(
            centre(&problem, &common.poses, index)[0] < 50.0,
            "R{} began at the common start",
            index - 2
        );
    }
    // The parts without a start begin at the common start, as before (the
    // step after it moves them differently: the chain pulls).
    for index in 2..8 {
        let [x, _] = centre(&problem, &started.poses, index);
        assert!(
            (x - centre(&problem, &common.poses, index)[0]).abs() < 1.0,
            "R{} at {x}",
            index - 2
        );
    }
}

#[test]
fn edge_and_region_constraints_are_kept() {
    use pcb_placer::constraints::Edge;
    let mut problem = chain(12);
    // R5 (index 7) must touch the top edge, R8 (index 10) stay in the
    // bottom-right corner box.
    problem.constraints.edges.push((7, Edge::Top, 0.5));
    problem
        .constraints
        .regions
        .push((10, [60.0, 35.0, 80.0, 50.0]));
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let mut problem = chain(1);
    problem.components = vec![
        // A through-hole part on the front with one 2 mm hole at its centre.
        part(Side::Front, vec![[-1.0, -1.0, 1.0, 1.0]]),
        // An SMD part on the back.
        Component {
            body_size: [3.0, 3.0],
            ..part(Side::Back, Vec::new())
        },
        // Artwork: occupies nothing.
        Component {
            body_size: [20.0, 20.0],
            ..part(Side::Neither, Vec::new())
        },
    ];
    problem.poses = vec![
        Pose {
            position: [40.0, 25.0],
            angle: 0.0
        };
        3
    ];
    problem.spacing = 0.2;
    // Under the body but away from the hole: legal.
    let beside = Pose {
        position: [44.0, 25.0],
        angle: 0.0,
    };
    assert!(is_legal(&problem, &problem.poses, 1, beside, 0..3));
    // On the hole: not legal.
    let on_hole = Pose {
        position: [40.5, 25.0],
        angle: 0.0,
    };
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    // A shield outline over the whole board with one header pad at its
    // left end.
    let shield = Component {
        hollow: vec![[-9.0, -1.0, -7.0, 1.0]],
        ..part(20.0, true)
    };
    problem.components = vec![shield, part(2.0, false)];
    problem.poses = vec![
        Pose {
            position: [25.0, 25.0],
            angle: 0.0,
        },
        Pose {
            position: [25.0, 25.0],
            angle: 0.0,
        },
    ];
    let legal = |problem: &Problem, x: f64| {
        is_legal(
            problem,
            &problem.poses,
            1,
            Pose {
                position: [x, 25.0],
                angle: 0.0,
            },
            0..2,
        )
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
        pins: vec![Pin {
            offset: [0.0, 0.0],
            net,
        }],
        side: Side::Front,
        fixed: false,
        angle_options: vec![0.0],
        far_side: Vec::new(),
        far_copper: Vec::new(),
        far_courtyard: Vec::new(),
        hollow: Vec::new(),
        tight: Some([-1.0, -1.0, 1.0, 1.0]),
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let mut problem = Problem {
        outline: vec![[0.0, 0.0], [7.0, 0.0], [7.0, 4.0], [0.0, 4.0]],
        components: vec![part(0), part(0)],
        net_weights: vec![1.0],
        poses: vec![
            Pose {
                position: [3.5, 2.0],
                angle: 0.0
            };
            2
        ],
        spacing: 0.2,
        grid: 0.1,
        edge_margin: 0.0,
        min_spacing: 0.2,
        far_side_pads_only: false,
        shared_halos: false,
        escape: None,
        crossings: None,
        pieces: Vec::new(),
        constraints: Default::default(),
    };
    let placement = place(&problem, &Config::new());
    assert!(
        placement.unplaced.is_empty() && placement.illegal.is_empty(),
        "{:?}",
        placement.unplaced
    );
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    // A 2 mm hole in the board, and a 10 mm connector with a pad at its
    // left end.
    let cutout = Component {
        copper_only: true,
        ..part(2.0, Side::Both, true)
    };
    let connector = Component {
        pads: vec![[-5.0, -1.0, -3.0, 1.0]],
        ..part(10.0, Side::Front, false)
    };
    problem.components = vec![cutout, connector];
    problem.poses = vec![
        Pose {
            position: [25.0, 25.0],
            angle: 0.0,
        },
        Pose {
            position: [25.0, 25.0],
            angle: 0.0,
        },
    ];
    problem.edge_margin = 0.5;
    let legal = |problem: &Problem, x: f64| {
        is_legal(
            problem,
            &problem.poses,
            1,
            Pose {
                position: [x, 25.0],
                angle: 0.0,
            },
            0..2,
        )
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
fn a_cutout_the_user_cut_keeps_bodies_off() {
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    // A triangular window (its box 4 x 4 mm) and a 2 mm part.
    let window = vec![[23.0, 23.0], [27.0, 23.0], [23.0, 27.0]];
    let cutout = Component {
        copper_only: true,
        cutout_outline: window,
        ..part(4.0, Side::Both, true)
    };
    problem.components = vec![cutout, part(2.0, Side::Front, false)];
    problem.poses = vec![
        Pose {
            position: [25.0, 25.0],
            angle: 0.0
        };
        2
    ];
    let legal = |problem: &Problem, at: [f64; 2]| {
        is_legal(
            problem,
            &problem.poses,
            1,
            Pose {
                position: at,
                angle: 0.0,
            },
            0..2,
        )
    };
    // A designer's slot: the body may reach over it.
    assert!(legal(&problem, [24.0, 24.0]));
    problem.constraints.solid_cutouts = vec![0];
    assert!(!legal(&problem, [24.0, 24.0]));
    // Beside the window's slanted side, inside its box: on the board.
    assert!(legal(&problem, [26.6, 26.6]));
    // Touching it: still on the board.
    assert!(legal(&problem, [22.0, 25.0]));
}

#[test]
fn a_part_held_at_the_top_of_an_l_keeps_to_its_step() {
    use pcb_placer::constraints::{Edge, outline_sides, side_distance};
    // An L: 20 mm wide and 40 tall on the left, its right block 40 x 20
    // mm, 20 mm lower. A part held at the top, wired to a part in the
    // right block, sits under the step (y = 20), not at the box's top
    // (y = 0, off the board there).
    let outline = vec![
        [0.0, 0.0],
        [20.0, 0.0],
        [20.0, 20.0],
        [60.0, 20.0],
        [60.0, 40.0],
        [0.0, 40.0],
    ];
    let sides = outline_sides(std::slice::from_ref(&outline));
    let count = |edge: Edge| sides.iter().filter(|(facing, _)| *facing == edge).count();
    assert_eq!(
        [
            count(Edge::Left),
            count(Edge::Top),
            count(Edge::Right),
            count(Edge::Bottom)
        ],
        [1, 2, 2, 1]
    );
    // Either way round the loop.
    let reversed: Vec<_> = outline.iter().rev().copied().collect();
    let again = outline_sides(&[reversed]);
    assert_eq!(
        again
            .iter()
            .filter(|(facing, _)| *facing == Edge::Top)
            .count(),
        2
    );
    // Under the step, 1 mm below it; in the left arm, 3 mm below its top;
    // beside the step's end, missed by how far the body misses it.
    assert!(
        (side_distance(&sides, Edge::Top, [40.0, 23.0], [2.0, 2.0]).unwrap() - 1.0).abs() < 1.0e-9
    );
    assert!(
        (side_distance(&sides, Edge::Top, [10.0, 5.0], [2.0, 2.0]).unwrap() - 3.0).abs() < 1.0e-9
    );
    assert!(
        (side_distance(&sides, Edge::Right, [40.0, 30.0], [2.0, 2.0]).unwrap() - 18.0).abs()
            < 1.0e-9
    );
    // A slanted side faces where its normal points most: a triangle.
    let triangle = outline_sides(&[vec![[0.0, 0.0], [10.0, 0.0], [0.0, 30.0]]]);
    assert_eq!(
        triangle.iter().map(|(edge, _)| *edge).collect::<Vec<_>>(),
        vec![Edge::Top, Edge::Right, Edge::Left]
    );

    let part = Component {
        name: "J1".into(),
        body_center: [0.0, 0.0],
        body_size: [4.0, 4.0],
        round: false,
        halo: 0.0,
        pins: vec![Pin {
            offset: [0.0, 0.0],
            net: 0,
        }],
        side: Side::Front,
        fixed: false,
        angle_options: vec![0.0],
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let mut problem = Problem {
        outline,
        components: vec![
            part.clone(),
            Component {
                name: "U1".into(),
                fixed: true,
                ..part
            },
        ],
        net_weights: vec![1.0],
        poses: vec![
            Pose {
                position: [30.0, 30.0],
                angle: 0.0,
            },
            Pose {
                position: [45.0, 32.0],
                angle: 0.0,
            },
        ],
        spacing: 0.2,
        grid: 0.1,
        edge_margin: 0.0,
        min_spacing: 0.2,
        far_side_pads_only: false,
        shared_halos: false,
        escape: None,
        crossings: None,
        pieces: Vec::new(),
        constraints: Default::default(),
    };
    problem.constraints.edges.push((0, Edge::Top, 0.5));
    // Against the box's top, the part has to go to the left arm (over the
    // right block, the box's top is off the board).
    let boxed = place(&problem, &Config::new());
    let top = boxed.poses[0].position[1] - 2.0;
    assert!(top < 0.6, "against the box: top at {top}");
    problem.constraints.sides = sides;
    let placement = place(&problem, &Config::new());
    assert!(placement.unplaced.is_empty() && placement.illegal.is_empty());
    let [x, y] = placement.poses[0].position;
    assert!(
        (20.0..=20.5).contains(&(y - 2.0)) && x - 2.0 >= 20.0,
        "J1 at {x}, {y}"
    );
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
        pins: vec![Pin {
            offset: [0.0, 0.0],
            net: 0,
        }],
        side: Side::Front,
        fixed: false,
        angle_options: vec![0.0],
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let mut problem = Problem {
        outline: vec![[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]],
        components: vec![part(), part()],
        net_weights: vec![1.0],
        poses: vec![
            Pose {
                position: [30.0, 20.0],
                angle: 0.0
            };
            2
        ],
        spacing: 0.5,
        grid: 0.5,
        edge_margin: 0.5,
        min_spacing: 0.2,
        far_side_pads_only: false,
        shared_halos: false,
        escape: None,
        crossings: None,
        pieces: Vec::new(),
        constraints: Default::default(),
    };
    problem.constraints.relations.push(Relation::Apart {
        part: 0,
        anchor: 1,
        min: 15.0,
    });
    problem.constraints.relation_weight = 50.0;
    let placement = place(&problem, &Config::new());
    assert!(placement.unplaced.is_empty());
    let [a, b] = [placement.poses[0].position, placement.poses[1].position];
    let gap = ((a[0] - b[0]).abs() - 4.0)
        .max(0.0)
        .hypot(((a[1] - b[1]).abs() - 4.0).max(0.0));
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
        pins: vec![
            Pin {
                offset: [-1.0, 0.0],
                net,
            },
            Pin {
                offset: [1.0, 0.0],
                net: net + 1,
            },
        ],
        side: Side::Front,
        fixed: false,
        angle_options: vec![0.0, 90.0, 180.0, 270.0],
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let problem = Problem {
        outline: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
        components: vec![part(0), part(2), part(4)],
        net_weights: vec![1.0; 6],
        poses: vec![
            Pose {
                position: [10.0, 10.0],
                angle: 0.0,
            },
            Pose {
                position: [20.0, 10.0],
                angle: 0.0,
            },
            Pose {
                position: [30.0, 10.0],
                angle: 90.0,
            },
        ],
        spacing: 0.5,
        grid: 0.5,
        edge_margin: 0.5,
        min_spacing: 0.2,
        far_side_pads_only: false,
        shared_halos: false,
        escape: None,
        crossings: None,
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
        pins: vec![Pin {
            offset: [0.0, 0.0],
            net: 0,
        }],
        side: Side::Front,
        fixed: false,
        angle_options: vec![0.0],
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
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
        components: vec![
            part.clone(),
            Component {
                name: "U1".into(),
                fixed: true,
                ..part
            },
        ],
        net_weights: vec![1.0],
        poses: vec![
            Pose {
                position: [8.0, 10.0],
                angle: 0.0,
            },
            Pose {
                position: [5.0, 10.0],
                angle: 0.0,
            },
        ],
        spacing: 0.2,
        grid: 0.1,
        edge_margin: 0.0,
        min_spacing: 0.2,
        far_side_pads_only: false,
        shared_halos: false,
        escape: None,
        crossings: None,
        pieces: vec![[0.0, 0.0, 20.0, 20.0], [30.0, 0.0, 50.0, 20.0]],
        constraints: Default::default(),
    };
    problem.constraints.edges.push((0, Edge::Right, 0.5));
    let placement = place(&problem, &Config::new());
    assert!(placement.unplaced.is_empty() && placement.illegal.is_empty());
    let right = placement.poses[0].position[0] + 2.0;
    assert!((19.4..=20.0).contains(&right), "right side at {right}");
}

#[test]
fn parts_turned_by_odd_angles_meet_as_turned_rectangles() {
    use pcb_placer::legal::is_legal;
    let bar = Component {
        name: String::new(),
        body_center: [0.0, 0.0],
        body_size: [10.0, 2.0],
        round: false,
        halo: 0.0,
        pins: Vec::new(),
        side: Side::Front,
        fixed: false,
        angle_options: vec![30.0],
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let mut problem = chain(1);
    problem.components = vec![bar.clone(), bar];
    problem.spacing = 0.2;
    // KiCad turns a local point by -angle: the bar's own axes on the board.
    let (sin, cos) = (-30.0f64).to_radians().sin_cos();
    let (ex, ey) = ([cos, sin], [-sin, cos]);
    let local = |u: f64, v: f64| [40.0 + u * ex[0] + v * ey[0], 25.0 + u * ex[1] + v * ey[1]];
    let at = |v: f64| Pose {
        position: local(0.0, v),
        angle: 30.0,
    };
    problem.poses = vec![at(0.0), at(3.0)];
    // Side by side 3 mm apart: the bars (2 mm wide) keep 1 mm, while the
    // boxes around them (10.7 x 6.7 mm) overlap.
    assert!(is_legal(
        &problem,
        &problem.poses,
        1,
        problem.poses[1],
        0..2
    ));
    // 2.1 mm apart: closer than the spacing.
    assert!(!is_legal(&problem, &problem.poses, 1, at(2.1), 0..2));
    // A board turned like the bars: a bar along its edge is on the board,
    // though the box around it is not.
    problem.outline = vec![
        local(-15.0, -15.0),
        local(15.0, -15.0),
        local(15.0, 15.0),
        local(-15.0, 15.0),
    ];
    problem.poses = vec![at(-10.0), at(13.5)];
    assert!(is_legal(&problem, &problem.poses, 1, at(13.5), 0..2));
    assert!(!is_legal(&problem, &problem.poses, 1, at(14.5), 0..2));
}

/// A front part and a back part for the far-side and halo tests: the
/// first with a 2 mm hole at its centre and copper only copper may not come
/// near 3-5 mm below it, the second a 3 x 3 mm SMD part whose pad fills it.
fn across_the_board() -> Problem {
    let part = |side: Side, size: [f64; 2]| Component {
        name: String::new(),
        body_center: [0.0, 0.0],
        body_size: size,
        round: false,
        halo: 0.0,
        pins: Vec::new(),
        side,
        fixed: false,
        angle_options: vec![0.0],
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let mut problem = chain(1);
    problem.components = vec![
        Component {
            far_side: vec![[-1.0, -1.0, 1.0, 1.0]],
            far_copper: vec![[-1.0, 3.0, 1.0, 5.0]],
            far_courtyard: Vec::new(),
            ..part(Side::Front, [10.0, 10.0])
        },
        Component {
            pads: vec![[-1.5, -1.5, 1.5, 1.5]],
            ..part(Side::Back, [3.0, 3.0])
        },
    ];
    problem.poses = vec![
        Pose {
            position: [40.0, 25.0],
            angle: 0.0
        };
        2
    ];
    problem.spacing = 0.5;
    problem.min_spacing = 0.2;
    problem
}

#[test]
fn the_annealer_can_keep_the_gaps_across_the_board_the_legality_test_asks_for() {
    use pcb_placer::anneal::{AnnealConfig, overlaps};
    use pcb_placer::legal::is_legal;
    let problem = across_the_board();
    let at = |x: f64, y: f64| {
        let mut poses = problem.poses.clone();
        poses[1] = Pose {
            position: [x, y],
            angle: 0.0,
        };
        poses
    };
    let old = AnnealConfig::default();
    let legal = AnnealConfig {
        legal_far_side: true,
        ..AnnealConfig::default()
    };
    // 0.3 mm from the hole (the spacing is 0.5): the old model sees
    // nothing closer than half the spacing.
    let near_hole = at(42.8, 25.0);
    assert!(!is_legal(&problem, &near_hole, 1, near_hole[1], 0..2));
    assert_eq!(overlaps(&problem, &near_hole, &old)[1], 0.0);
    assert!(overlaps(&problem, &near_hole, &legal)[1] > 0.0);
    // Its pad 0.1 mm from the far side's copper (the clearance is 0.2):
    // the old model does not see such copper at all.
    let near_copper = at(40.0, 31.6);
    assert!(!is_legal(&problem, &near_copper, 1, near_copper[1], 0..2));
    assert_eq!(overlaps(&problem, &near_copper, &old)[1], 0.0);
    assert!(overlaps(&problem, &near_copper, &legal)[1] > 0.0);
    // Clear of both: legal, and no overlap either way.
    let clear = at(40.0, 34.0);
    assert!(is_legal(&problem, &clear, 1, clear[1], 0..2));
    assert_eq!(overlaps(&problem, &clear, &legal)[1], 0.0);
}

#[test]
fn shared_halos_keep_the_larger_halo_between_neighbours() {
    use pcb_placer::legal::is_legal;
    let mut problem = across_the_board();
    for component in &mut problem.components {
        component.side = Side::Front;
        component.far_side.clear();
        component.far_copper.clear();
        component.body_size = [2.0, 1.0];
        component.pads.clear();
        component.halo = 0.65;
    }
    problem.components[1].halo = 0.3;
    // Bodies 1.2 mm apart: the halos' sum and the spacing want 1.45, the
    // larger halo and the spacing 1.15.
    let poses = vec![
        Pose {
            position: [40.0, 25.0],
            angle: 0.0,
        },
        Pose {
            position: [43.2, 25.0],
            angle: 0.0,
        },
    ];
    assert!(!is_legal(&problem, &poses, 1, poses[1], 0..2));
    problem.shared_halos = true;
    assert!(is_legal(&problem, &poses, 1, poses[1], 0..2));
    assert!(is_legal(&problem, &poses, 0, poses[0], 0..2));
    // 1.1 mm apart is too close either way.
    let closer = vec![
        poses[0],
        Pose {
            position: [43.1, 25.0],
            angle: 0.0,
        },
    ];
    assert!(!is_legal(&problem, &closer, 1, closer[1], 0..2));
    // A disc beside a rectangle's corner: the same answer from either side
    // (growing one body by the halo instead of the gap was not).
    problem.components[1].round = true;
    problem.components[1].body_size = [2.0, 2.0];
    for step in 0..40 {
        let offset = 1.5 + 0.02 * step as f64;
        let poses = vec![
            poses[0],
            Pose {
                position: [40.0 + 1.0 + offset * 0.7, 25.0 + 0.5 + offset * 0.7],
                angle: 0.0,
            },
        ];
        assert_eq!(
            is_legal(&problem, &poses, 0, poses[0], 0..2),
            is_legal(&problem, &poses, 1, poses[1], 0..2),
            "offset {offset}"
        );
    }
}

#[test]
fn a_placement_with_shared_halos_is_legal() {
    let mut problem = chain(12);
    for component in &mut problem.components {
        component.halo = if component.fixed { 0.0 } else { 0.65 };
    }
    problem.shared_halos = true;
    let placement = place(&problem, &Config::new());
    assert!(placement.unplaced.is_empty());
    assert!(placement.illegal.is_empty(), "{:?}", placement.illegal);
}

/// A chain with a busy end: parts R0-R5 also carry a bundle of nets
/// between them (several wires each), the rest of the chain only its
/// links.
fn busy_chain() -> Problem {
    let mut problem = chain(14);
    let base = problem.net_weights.len();
    for index in 0..6 {
        let component = &mut problem.components[2 + index];
        for wire in 0..6 {
            component.pins.push(Pin {
                offset: [0.0, -1.0 + 0.4 * wire as f64],
                net: base + wire,
            });
        }
    }
    problem.net_weights.extend(std::iter::repeat_n(1.0, 6));
    for component in &mut problem.components {
        component.halo = if component.fixed { 0.0 } else { 0.65 };
    }
    problem
}

#[test]
fn routing_room_gives_the_busy_parts_the_halos_and_stays_legal() {
    use pcb_placer::room::RoutingRoom;
    let problem = busy_chain();
    let mut config = Config::new();
    config.routing_room = Some(RoutingRoom {
        net_demand: vec![1.0; problem.net_weights.len()],
        ..Default::default()
    });
    let placement = place(&problem, &config);
    assert!(placement.unplaced.is_empty());
    assert!(placement.illegal.is_empty(), "{:?}", placement.illegal);
    let halos = placement
        .relaxation
        .halos
        .as_ref()
        .expect("routing room set the halos");
    // The bundle's parts have more halo than before, the quiet ones less.
    let busy: f64 = (2..8).map(|index| halos[index]).sum::<f64>() / 6.0;
    let quiet: f64 = (8..16).map(|index| halos[index]).sum::<f64>() / 8.0;
    assert!(busy > quiet, "busy {busy} quiet {quiet}: {halos:?}");
    assert!((8..16).all(|index| halos[index] < 0.65), "{halos:?}");
    assert!(busy > 0.65, "{halos:?}");
    // The fixed connectors keep theirs; nothing exceeds twice the old halo.
    assert_eq!((halos[0], halos[1]), (0.0, 0.0));
    assert!(
        halos.iter().all(|halo| *halo <= 2.0 * 0.65 + 1.0e-9),
        "{halos:?}"
    );
    // The relaxation applies the same halos to the unscaled problem, as
    // the layout's moves see it.
    let mut again = problem.clone();
    placement.relaxation.apply(&mut again);
    for (component, halo) in again.components.iter().zip(halos.iter()) {
        assert_eq!(component.halo, *halo);
    }
    let placed = Problem {
        poses: placement.poses.clone(),
        ..again
    };
    assert!(pcb_placer::legal::illegal_components(&placed, &placement.poses).is_empty());
}

#[test]
fn without_routing_room_the_placement_is_unchanged() {
    let problem = busy_chain();
    let first = place(&problem, &Config::new());
    assert!(first.relaxation.halos.is_none());
    // A plane on every net leaves no demand: the halos stay as they were
    // (the median is zero), and no second pass runs.
    let mut config = Config::new();
    config.routing_room = Some(pcb_placer::room::RoutingRoom {
        net_demand: vec![0.0; problem.net_weights.len()],
        ..Default::default()
    });
    let second = place(&problem, &config);
    assert!(second.illegal.is_empty());
    assert!(second.relaxation.halos.is_none());
    assert_eq!(second.poses, first.poses);
}

/// Two back-mounted hot-swap switches: on the back a 9 x 6 mm socket, on
/// the front their 14 mm square courtyard (woagboard's choc switches).
/// KiCad checks the front squares against each other although both parts
/// are on the back.
#[test]
fn courtyards_on_the_far_side_of_parts_on_one_side_keep_apart() {
    use pcb_placer::anneal::{AnnealConfig, overlaps};
    use pcb_placer::legal::is_legal;
    let square = [-7.0, -7.0, 7.0, 7.0];
    let switch = Component {
        name: String::new(),
        body_center: [0.0, -3.0],
        body_size: [9.0, 6.0],
        round: false,
        halo: 0.0,
        pins: Vec::new(),
        side: Side::Back,
        fixed: false,
        angle_options: vec![0.0],
        far_side: vec![[-1.0, -1.0, 1.0, 1.0], square],
        far_copper: Vec::new(),
        far_courtyard: vec![square],
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let mut problem = chain(1);
    problem.components = vec![switch.clone(), switch.clone()];
    problem.spacing = 0.5;
    let at = |x: f64| Pose {
        position: [x, 25.0],
        angle: 0.0,
    };
    problem.poses = vec![at(30.0), at(42.0)];
    // 12 mm apart the sockets (9 mm wide) keep 3 mm, but the front squares
    // overlap by 2 mm.
    assert!(!is_legal(&problem, &problem.poses, 1, at(42.0), 0..2));
    // 14.5 mm apart the squares keep the spacing.
    assert!(is_legal(&problem, &problem.poses, 1, at(44.5), 0..2));
    // The annealer sees the overlap too.
    assert!(overlaps(&problem, &problem.poses, &AnnealConfig::default())[1] > 0.0);
    let mut apart = problem.poses.clone();
    apart[1] = at(44.5);
    assert_eq!(overlaps(&problem, &apart, &AnnealConfig::default())[1], 0.0);
    // Without the far courtyard (a switch whose front holds only its holes)
    // 12 mm apart is fine.
    let mut plain = problem.clone();
    for component in &mut plain.components {
        component.far_courtyard.clear();
        component.far_side.truncate(1);
    }
    assert!(is_legal(&plain, &plain.poses, 1, at(42.0), 0..2));
    // The placer keeps them apart.
    let placement = pcb_placer::place(&problem, &pcb_placer::Config::new());
    assert!(placement.illegal.is_empty() && placement.unplaced.is_empty());
    let gap = (placement.poses[0].position[0] - placement.poses[1].position[0])
        .abs()
        .max((placement.poses[0].position[1] - placement.poses[1].position[1]).abs());
    assert!(gap >= 14.5 - 1.0e-6, "squares {gap} mm apart");
}

/// `count` key switches of 17 x 17 mm on a `width` x `height` board.
fn keys_on(width: f64, height: f64, count: usize) -> Problem {
    let mut problem = chain(count);
    problem.outline = vec![[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]];
    problem.poses[0].position = [2.0, height / 2.0];
    problem.poses[1].position = [width - 2.0, height / 2.0];
    for component in &mut problem.components[2..] {
        component.body_size = [17.0, 17.0];
    }
    for pose in &mut problem.poses[2..] {
        pose.position = [width / 2.0, height / 2.0];
    }
    problem
}

#[test]
fn a_board_without_room_says_no_at_once() {
    // Ten 17 mm keys on 80 x 40 mm: 2890 mm2 of 3200, but two strips of
    // 80 mm hold only four keys each. No level can place them: the placer
    // says so without annealing at every level.
    let problem = keys_on(80.0, 40.0, 10);
    let placement = place(&problem, &Config::new());
    assert!(placement.overfull);
    assert!(!placement.unplaced.is_empty());
    // Little work: one global placement and a legalisation near where it
    // put the parts.
    assert!(
        (placement.work as f64) < 5.0 * pcb_placer::REFERENCE_WORK_PER_SECOND,
        "work {}",
        placement.work
    );
    // Eight fit (two rows of four); the placer seats at least some.
    assert!(placement.unplaced.len() < 10, "{:?}", placement.unplaced);
    // Room enough (four rows of 6 on 110 x 75 mm): no fast no.
    let placement = place(&keys_on(110.0, 75.0, 10), &Config::new());
    assert!(!placement.overfull);
    assert!(placement.unplaced.is_empty(), "{:?}", placement.unplaced);
}

#[test]
fn the_work_budget_is_kept_inside_the_passes() {
    // A crowded board that is not hopeless by area or strips (40 parts of
    // 6 x 3 mm on 32 x 25 mm: 720 of 800 mm2), on a small budget: the
    // legalisation and the passes after it stop near the budget instead
    // of searching the whole board for every part without room.
    let mut problem = chain(40);
    problem.outline = vec![[0.0, 0.0], [32.0, 0.0], [32.0, 25.0], [0.0, 25.0]];
    problem.poses[0].position = [2.0, 12.5];
    problem.poses[1].position = [30.0, 12.5];
    for pose in &mut problem.poses[2..] {
        pose.position = [16.0, 12.5];
    }
    let free = place(&problem, &Config::new());
    // The global placement is not budgeted (and counts the same in both).
    let start = pcb_placer::work();
    pcb_placer::global::global_place(&problem, &Config::new().global);
    let global = pcb_placer::work() - start;
    let mut config = Config::new();
    let limit = (0.05 * pcb_placer::REFERENCE_WORK_PER_SECOND) as u64;
    config.work_limit = Some(limit);
    let placement = place(&problem, &config);
    assert!(!placement.overfull);
    eprintln!(
        "work without a limit {}, with {}, global {global}",
        free.work, placement.work
    );
    assert!(
        free.work - global > 10 * limit,
        "the test needs a board whose passes take long"
    );
    // What follows the global placement overruns by one hurried pass.
    assert!(
        placement.work - global < 5 * limit,
        "work {} (global {global}) against a limit of {limit}",
        placement.work
    );
}

#[test]
fn parts_without_room_go_first_until_none_takes_anothers_place() {
    use pcb_placer::constraints::Edge;
    use pcb_placer::legal::{is_legal, legalize, legalize_first, legalize_stuck_first};
    // MIDAS-MK1's bottom edge, simplified: two 7.7 x 35.4 mm terminal
    // blocks (J103, J105) held at the bottom, a 24.7 x 19.1 mm card slot
    // (J108) held at the left edge and annealed into the bottom-left
    // corner, and fixed parts (Q203, R207, J101 and the like) filling the
    // bottom right from x = 36. Left of them the bottom has room for both
    // blocks only when J108 moves up its edge.
    let part = |name: &str, size: [f64; 2], fixed: bool| Component {
        name: name.into(),
        body_center: [0.0, 0.0],
        body_size: size,
        round: false,
        halo: 0.0,
        pins: vec![Pin {
            offset: [0.0, 0.0],
            net: 0,
        }],
        side: Side::Front,
        fixed,
        angle_options: vec![0.0],
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let at = |x: f64, y: f64| Pose {
        position: [x, y],
        angle: 0.0,
    };
    let mut problem = Problem {
        outline: vec![[0.0, 0.0], [60.0, 0.0], [60.0, 100.0], [0.0, 100.0]],
        components: vec![
            part("Q203", [24.0, 40.0], true),
            part("J108", [24.7, 19.1], false),
            part("J103", [7.7, 35.4], false),
            part("J105", [7.7, 35.4], false),
        ],
        net_weights: vec![1.0],
        // Where the anneal left them: J108 in the corner, J103 and J105
        // wanting spots at and over the fixed block.
        poses: vec![
            at(48.0, 80.0),
            at(12.35, 90.45),
            at(35.0, 82.3),
            at(45.0, 82.3),
        ],
        spacing: 0.2,
        grid: 0.5,
        edge_margin: 0.0,
        min_spacing: 0.2,
        far_side_pads_only: false,
        shared_halos: false,
        escape: None,
        crossings: None,
        pieces: Vec::new(),
        constraints: Default::default(),
    };
    problem.constraints.edges.push((1, Edge::Left, 0.5));
    problem.constraints.edges.push((2, Edge::Bottom, 0.5));
    problem.constraints.edges.push((3, Edge::Bottom, 0.5));
    let annealed = problem.poses.clone();
    // Largest first: J108 keeps the corner, J103 the spot next to the
    // fixed block, and J105 finds none.
    let mut poses = annealed.clone();
    assert_eq!(legalize(&problem, &mut poses), vec![3]);
    // J105 first takes J103's spot, and J103 finds none.
    let mut poses = annealed.clone();
    assert_eq!(legalize_first(&problem, &mut poses, &[3]), vec![2]);
    // Both first: they stand side by side and J108 goes up its edge.
    let (poses, failed) = legalize_stuck_first(&problem, &annealed, &[3], 4, false);
    assert!(failed.is_empty(), "{failed:?}");
    for index in 0..problem.components.len() {
        assert!(
            is_legal(&problem, &poses, index, poses[index], 0..poses.len()),
            "{} at {:?}",
            problem.components[index].name,
            poses[index]
        );
    }
    assert!(poses[1].position[0] < 13.0 && poses[1].position[1] < 64.6 - 9.55);
    assert!(poses[2].position[1] > 82.0 && poses[3].position[1] > 82.0);
}

#[test]
fn pads_of_a_part_held_by_its_copper_keep_the_copper_to_edge_clearance_on_every_side() {
    use pcb_placer::constraints::Edge;
    // A module held at the left edge by its copper (the edge_copper
    // level), its top pad near the top edge: the project's copper-to-edge
    // clearance (0.3 mm) holds there too, not the copper clearance (0.2).
    let module = Component {
        name: "U3".into(),
        body_center: [0.0, 0.0],
        body_size: [4.0, 4.0],
        round: false,
        halo: 0.0,
        pins: vec![Pin {
            offset: [0.0, -1.5],
            net: 0,
        }],
        side: Side::Front,
        fixed: false,
        angle_options: vec![0.0],
        far_side: Vec::new(),
        far_copper: Vec::new(),
        far_courtyard: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: vec![[-0.5, -2.0, 0.5, -1.0]],
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let mut problem = Problem {
        outline: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]],
        components: vec![module],
        net_weights: vec![1.0],
        poses: vec![Pose {
            position: [2.5, 10.0],
            angle: 0.0,
        }],
        spacing: 0.0,
        grid: 0.0,
        edge_margin: 0.5,
        min_spacing: 0.2,
        far_side_pads_only: false,
        shared_halos: false,
        escape: None,
        crossings: None,
        pieces: Vec::new(),
        constraints: Default::default(),
    };
    problem.constraints.edges.push((0, Edge::Left, 1.0));
    problem.constraints.edge_copper = true;
    problem.constraints.copper_edge = 0.3;
    let legal = |y: f64| {
        let pose = Pose {
            position: [2.5, y],
            angle: 0.0,
        };
        pcb_placer::legal::is_legal(&problem, &[pose], 0, pose, std::iter::empty())
    };
    // The pad's top edge 0.25 mm from the board's: too close.
    assert!(!legal(2.25));
    assert!(legal(2.31));
}

#[test]
fn the_anneal_evaluates_its_guidance_within_its_share_and_is_unchanged_without_a_field() {
    use pcb_placer::anneal::{AnnealConfig, anneal};
    use pcb_placer::guidance::{Guidance, GuidanceField};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let problem = chain(8);
    let plain = {
        let mut poses = problem.poses.clone();
        anneal(&problem, &mut poses, &AnnealConfig::default());
        poses
    };
    // A guidance that never has a field: the plain anneal, pose for pose.
    let empty = Guidance {
        evaluate: Arc::new(|_: &[Pose]| None),
        weight: 1.0,
        work_share: 0.1,
        min_accepted: 10,
        bias: 0.5,
    };
    let mut poses = problem.poses.clone();
    anneal(
        &problem,
        &mut poses,
        &AnnealConfig {
            guidance: Some(empty),
            ..AnnealConfig::default()
        },
    );
    assert_eq!(poses, plain);
    // A field hot on the left half: evaluated more than once, the
    // evaluations' work within their share (the first and the trace's
    // aside).
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let parts = problem.components.len();
    let hot = Guidance {
        evaluate: Arc::new(move |_: &[Pose]| {
            counted.fetch_add(1, Ordering::Relaxed);
            Some(GuidanceField {
                origin: [0.0, 0.0],
                tile_mm: 10.0,
                tiles_x: 8,
                tiles_y: 5,
                overflow: (0..40)
                    .map(|tile| if tile % 8 < 4 { 2.0 } else { 0.0 })
                    .collect(),
                attribution: vec![1.0; parts],
                total_overflow: 40.0,
                work: 1000,
            })
        }),
        weight: 1.0,
        work_share: 0.1,
        min_accepted: 10,
        bias: 0.5,
    };
    let started = pcb_placer::work();
    let mut poses = problem.poses.clone();
    anneal(
        &problem,
        &mut poses,
        &AnnealConfig {
            guidance: Some(hot),
            ..AnnealConfig::default()
        },
    );
    let evaluations = calls.load(Ordering::Relaxed);
    let total = pcb_placer::work() - started;
    assert!(evaluations > 2, "{evaluations} evaluations");
    let guided = 1000 * (evaluations as u64 - 2);
    assert!(
        guided as f64 <= 0.1 * (total - guided - 2000) as f64 + 1000.0,
        "{guided} of {total}"
    );
}

#[test]
fn eviction_finds_a_held_part_s_narrow_window_in_a_tab() {
    use pcb_placer::constraints::Edge;
    // OM-FlexGrid-Flex's flex connector J1: pads 11.15 mm wide in a tab
    // 12.4 mm wide, held at the tab's top edge; at the edge-copper level
    // its pads keep the 0.5 mm copper-to-edge clearance from the tab's
    // sides too, a window of 0.25 mm (x 16.075 to 16.325) that the
    // eviction's 0.5 mm scan from the board's left end (16.0, 16.5)
    // missed.
    let connector = Component {
        name: "J1".into(),
        body_center: [0.0, 1.5],
        body_size: [11.15, 4.0],
        round: false,
        halo: 0.0,
        pins: vec![Pin {
            offset: [0.0, 0.0],
            net: 0,
        }],
        side: Side::Front,
        fixed: false,
        angle_options: vec![0.0],
        far_side: Vec::new(),
        far_copper: Vec::new(),
        far_courtyard: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: vec![[-5.575, -0.5, 5.575, 3.5]],
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let mut problem = Problem {
        outline: vec![
            [10.0, 0.0],
            [22.4, 0.0],
            [22.4, 10.0],
            [40.0, 10.0],
            [40.0, 30.0],
            [0.0, 30.0],
            [0.0, 10.0],
            [10.0, 10.0],
        ],
        components: vec![connector],
        net_weights: vec![1.0],
        poses: vec![Pose {
            position: [30.0, 20.0],
            angle: 0.0,
        }],
        spacing: 0.0,
        grid: 0.1,
        edge_margin: 0.5,
        min_spacing: 0.2,
        far_side_pads_only: false,
        shared_halos: false,
        escape: None,
        crossings: None,
        pieces: Vec::new(),
        constraints: Default::default(),
    };
    problem.constraints.edges.push((0, Edge::Top, 0.5));
    problem.constraints.edge_copper = true;
    problem.constraints.copper_edge = 0.5;
    let mut poses = problem.poses.clone();
    let left = pcb_placer::legal::evict_for(&problem, &mut poses, &[0]);
    assert!(left.is_empty(), "no spot found");
    let x = poses[0].position[0];
    assert!((16.07..=16.33).contains(&x), "at {:?}", poses[0]);
    assert!(pcb_placer::legal::is_legal(
        &problem,
        &poses,
        0,
        poses[0],
        std::iter::empty()
    ));
}

#[test]
fn a_part_near_another_follows_it_through_the_legaliser() {
    use pcb_placer::constraints::{Anchor, Relation};
    // R2 (index 4) lies on J1 where the anneal left it: the legaliser
    // moves it out to the right. R0 (index 2) is near R2's pin and lay
    // above it. With follow_anchors, R0 is legalised after R2 and moved
    // as R2 moved; without, it is legalised first and stays.
    let mut problem = chain(3);
    problem.poses[4].position = [4.0, 25.0];
    problem.poses[2].position = [4.0, 15.0];
    problem.poses[3].position = [40.0, 40.0];
    for component in &mut problem.components[2..] {
        component.angle_options = vec![0.0];
    }
    problem.constraints.relations.push(Relation::NearAny {
        part: 2,
        anchors: vec![Anchor::Point(4, [2.0, 0.0])],
        max: 1.0,
    });
    problem.constraints.relation_weight = 50.0;
    let legalised = |follow: bool| {
        let mut problem = problem.clone();
        problem.constraints.follow_anchors = follow;
        problem.constraints.link_pairs();
        let mut poses = problem.poses.clone();
        let failed = pcb_placer::legal::legalize(&problem, &mut poses);
        assert!(failed.is_empty(), "{failed:?}");
        poses
    };
    let off = legalised(false);
    let on = legalised(true);
    // R2 moved the same way both times.
    assert_eq!(off[4], on[4]);
    let shift = on[4].position[0] - 4.0;
    assert!(shift > 4.0, "R2 at {:?}", on[4]);
    assert!(
        (off[2].position[0] - 4.0).abs() < 1.0e-9,
        "R0 at {:?}",
        off[2]
    );
    assert!(
        (on[2].position[0] - (4.0 + shift)).abs() <= 0.5 + 1.0e-9,
        "R0 at {:?}, R2 at {:?}",
        on[2],
        on[4]
    );
}

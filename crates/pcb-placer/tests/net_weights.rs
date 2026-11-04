// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! One net, one weight everywhere: the global placement, the anneal, the
//! refinement and the relation repair all price a net's wire by
//! `Problem::net_weights` (the KiCad lowering's `net_weight` sets it once).
//! A part between two fixed connectors, on one net to each, goes to the
//! side of the heavier net in each of the four.

use pcb_placer::{
    Component, GlobalConfig, Pin, Pose, Problem, Side, anneal, constraints, global, legal,
};

fn component(name: &str, size: [f64; 2], pins: Vec<Pin>, fixed: bool) -> Component {
    Component {
        name: name.into(),
        body_center: [0.0, 0.0],
        body_size: size,
        round: false,
        halo: 0.0,
        pins,
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
        courtless: Vec::new(),
        slots: Vec::new(),
    }
}

/// J1 (net 0) at the left, J2 (net 1) at the right, R1 (nets 0 and 1)
/// in the middle; the nets weighted `weights`.
fn tug(weights: [f64; 2]) -> Problem {
    let pin = |offset: [f64; 2], net: usize| Pin { offset, net };
    Problem {
        outline: vec![[0.0, 0.0], [80.0, 0.0], [80.0, 20.0], [0.0, 20.0]],
        components: vec![
            component("J1", [4.0, 6.0], vec![pin([0.0, 0.0], 0)], true),
            component("J2", [4.0, 6.0], vec![pin([0.0, 0.0], 1)], true),
            component(
                "R1",
                [3.0, 1.5],
                vec![pin([-1.0, 0.0], 0), pin([1.0, 0.0], 1)],
                false,
            ),
        ],
        net_weights: weights.to_vec(),
        poses: vec![
            Pose {
                position: [4.0, 10.0],
                angle: 0.0,
            },
            Pose {
                position: [76.0, 10.0],
                angle: 0.0,
            },
            Pose {
                position: [40.0, 10.0],
                angle: 0.0,
            },
        ],
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

const LEFT: [f64; 2] = [1.0, 0.2];
const RIGHT: [f64; 2] = [0.2, 1.0];

/// R1's x after `stage` on the tug weighted `weights`.
fn after(weights: [f64; 2], stage: impl Fn(&Problem, &mut Vec<Pose>)) -> f64 {
    let problem = tug(weights);
    let mut poses = problem.poses.clone();
    stage(&problem, &mut poses);
    poses[2].position[0]
}

fn sides(stage: impl Fn(&Problem, &mut Vec<Pose>)) -> (f64, f64) {
    (after(LEFT, &stage), after(RIGHT, &stage))
}

#[test]
fn the_global_placement_pulls_towards_the_heavier_net() {
    let (left, right) = sides(|problem, poses| {
        *poses = global::global_place(problem, &GlobalConfig::default()).poses;
    });
    assert!(left < 40.0 && right > 40.0, "{left} {right}");
}

#[test]
fn the_anneal_pulls_towards_the_heavier_net() {
    let (left, right) = sides(|problem, poses| {
        anneal::anneal(problem, poses, &anneal::AnnealConfig::default());
    });
    assert!(left < 20.0 && right > 60.0, "{left} {right}");
}

#[test]
fn the_refinement_pulls_towards_the_heavier_net() {
    let (left, right) = sides(|problem, poses| {
        legal::refine(problem, poses, 8);
    });
    assert!(left < 40.0 && right > 40.0, "{left} {right}");
}

#[test]
fn the_relation_repair_takes_the_side_of_the_heavier_net() {
    // R1 starts far from a fixed mark in the middle it must be near: the
    // repair puts it beside the mark, on the side the wire prefers.
    let (left, right) = sides(|problem, poses| {
        let mut problem = problem.clone();
        problem
            .components
            .push(component("M1", [4.0, 2.0], Vec::new(), true));
        problem.poses.push(Pose {
            position: [40.0, 3.0],
            angle: 0.0,
        });
        poses.push(problem.poses[3]);
        poses[2].position[1] = 17.0;
        problem.constraints.relation_weight = 1.0;
        problem
            .constraints
            .relations
            .push(constraints::Relation::Near {
                part: 2,
                anchor: constraints::Anchor::Body(3),
                max: 3.0,
            });
        assert!(legal::repair_relations(&problem, poses) > 0);
    });
    assert!(left < 40.0 && right > 40.0, "{left} {right}");
}

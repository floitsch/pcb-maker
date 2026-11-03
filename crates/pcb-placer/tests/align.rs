// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Rows of repeated parts (`pcb_placer::align`).

use pcb_placer::align::{Align, Settings};
use pcb_placer::problem::{Component, Pin, Pose, Problem, Side};

fn part(net: usize, size: [f64; 2], fixed: bool) -> Component {
    Component {
        name: String::new(),
        body_center: [0.0, 0.0],
        body_size: size,
        round: false,
        halo: 0.0,
        pins: vec![
            Pin {
                offset: [-size[0] / 3.0, 0.0],
                net,
            },
            Pin {
                offset: [size[0] / 3.0, 0.0],
                net: net + 1,
            },
        ],
        side: Side::Front,
        fixed,
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
    }
}

fn problem(components: Vec<Component>, poses: Vec<Pose>) -> Problem {
    let nets = 2 * components.len() + 2;
    Problem {
        outline: vec![[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]],
        components,
        net_weights: vec![1.0; nets],
        poses,
        spacing: 0.5,
        grid: 0.05,
        edge_margin: 0.5,
        min_spacing: 0.2,
        far_side_pads_only: false,
        shared_halos: false,
        escape: None,
        crossings: None,
        pieces: Vec::new(),
        constraints: Default::default(),
    }
}

fn at(x: f64, y: f64, angle: f64) -> Pose {
    Pose {
        position: [x, y],
        angle,
    }
}

#[test]
fn a_part_costs_its_offset_from_the_nearest_of_its_kind_in_line() {
    let p = problem(
        vec![
            part(0, [2.0, 1.0], false),
            part(2, [2.0, 1.0], false),
            part(4, [2.0, 1.0], false),
            part(6, [3.0, 1.0], false),
        ],
        vec![
            at(10.0, 10.0, 0.0),
            at(14.0, 10.3, 0.0),
            at(40.0, 10.0, 0.0),
            at(10.0, 20.0, 0.0),
        ],
    );
    let align = Align::new(&p, Settings::default());
    // Three of a kind; the 3 x 1 part is alone in its kind.
    assert_eq!(align.kinds.len(), 1);
    assert_eq!(align.kind[3], None);
    let cost = |poses: &[Pose], index| align.part_cost(&p, poses, index);
    let mut poses = p.poses.clone();
    assert!((cost(&poses, 0) - 0.3).abs() < 1e-9);
    assert!((cost(&poses, 1) - 0.3).abs() < 1e-9);
    // Beyond reach (10 mm): nothing to line up with.
    assert_eq!(cost(&poses, 2), 1.0);
    assert_eq!(cost(&poses, 3), 0.0);
    // In line, and turned alike modulo half a turn (two pins).
    poses[1] = at(14.0, 10.0, 180.0);
    assert_eq!(cost(&poses, 0), 0.0);
    // Turned otherwise: in line but the cap more, at most the cap.
    poses[1] = at(14.0, 10.0, 90.0);
    assert_eq!(cost(&poses, 0), 1.0);
}

#[test]
fn the_parts_a_move_touches_carry_the_whole_change() {
    // Twelve parts of one kind and two fixed anchors; random moves and
    // turns: the energy's change over the affected parts is the change of
    // the board's energy.
    let mut components: Vec<Component> = (0..12).map(|i| part(2 * i, [2.0, 1.0], false)).collect();
    components.push(part(30, [2.0, 1.0], true));
    components.push(part(32, [2.0, 1.0], true));
    let poses: Vec<Pose> = (0..14)
        .map(|i| at(5.0 + 3.0 * (i % 5) as f64, 5.0 + 4.0 * (i / 5) as f64, 0.0))
        .collect();
    let p = problem(components, poses);
    let align = Align::new(
        &p,
        Settings {
            reach: 7.0,
            ..Settings::default()
        },
    );
    let mut poses = p.poses.clone();
    let mut seed = 12345u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for _ in 0..300 {
        let index = ((next() * 12.0) as usize).min(11);
        let from = p.components[index].center(poses[index]);
        let before_total = align.total(&p, &poses);
        let old = poses[index];
        let snap = |v: f64| (v * 4.0).round() / 4.0;
        poses[index] = if next() < 0.2 {
            at(
                old.position[0],
                old.position[1],
                [0.0, 90.0, 180.0, 270.0][(next() * 4.0) as usize % 4],
            )
        } else {
            at(
                snap((old.position[0] + (next() - 0.5) * 12.0).clamp(1.0, 59.0)),
                snap((old.position[1] + (next() - 0.5) * 12.0).clamp(1.0, 39.0)),
                old.angle,
            )
        };
        let mut touched = Vec::new();
        align.affected(&p, &poses, index, from, &mut touched);
        let after_local = align.sum(&p, &poses, &touched);
        let after_total = align.total(&p, &poses);
        let moved = poses[index];
        poses[index] = old;
        let before_local = align.sum(&p, &poses, &touched);
        assert!(
            ((after_total - before_total) - (after_local - before_local)).abs() < 1e-9,
            "total change {} local change {}",
            after_total - before_total,
            after_local - before_local
        );
        if next() < 0.7 {
            poses[index] = moved;
        }
    }
}

#[test]
fn the_finish_lines_a_part_up_where_it_costs_next_to_nothing() {
    use pcb_placer::legal::align_rows;
    // R1 and R3 are beyond each other's reach (8 mm); both are 0.4 mm out
    // of line with R2 between them, which pulls nothing.
    let p = problem(
        vec![
            part(0, [2.0, 1.0], false),
            part(2, [2.0, 1.0], false),
            part(4, [2.0, 1.0], false),
        ],
        vec![at(10.0, 10.0, 0.0), at(14.0, 10.4, 0.0), at(18.0, 10.0, 0.0)],
    );
    let align = Align::new(&p, Settings::default());
    let mut poses = p.poses.clone();
    assert_eq!(align.lined_up(&p, &poses), Some(0.0));
    assert!((align.total(&p, &poses) - 1.2).abs() < 1e-9);
    // R1 lines up with R2, then R3 does.
    assert_eq!(align_rows(&p, &mut poses, &align, 0.5), 2);
    assert!(poses.iter().all(|pose| (pose.position[1] - 10.4).abs() < 1e-9));
    assert_eq!(align.lined_up(&p, &poses), Some(1.0));
    assert_eq!(align.total(&p, &poses), 0.0);
}

#[test]
fn the_anneal_lines_up_parts_of_a_kind_with_the_term() {
    use pcb_placer::anneal::{AnnealConfig, anneal};
    // Eight parts of one kind each tied to a fixed anchor along a ragged
    // line: without the term they sit at their anchors, with it they
    // line up.
    let mut components = Vec::new();
    let mut poses = Vec::new();
    for i in 0..8 {
        components.push(part(2 * i, [2.0, 1.0], false));
        poses.push(at(
            8.0 + 5.0 * i as f64,
            20.0 + 0.6 * ((i * 7) % 3) as f64,
            0.0,
        ));
    }
    for i in 0..8 {
        let mut anchor = part(2 * i, [1.0, 1.0], true);
        anchor.pins.truncate(1);
        components.push(anchor);
        poses.push(at(
            8.0 + 5.0 * i as f64,
            14.0 + 0.6 * ((i * 7) % 3) as f64,
            0.0,
        ));
    }
    let p = problem(components, poses);
    let settings = Settings {
        weight: 4.0,
        reach: 7.0,
        ..Settings::default()
    };
    let align = std::sync::Arc::new(Align::new(&p, settings));
    let run = |with: bool| {
        let mut poses = p.poses.clone();
        let config = AnnealConfig {
            stages: 40,
            align: with.then(|| align.clone()),
            ..AnnealConfig::default()
        };
        anneal(&p, &mut poses, &config);
        align.lined_up(&p, &poses).unwrap()
    };
    let (off, on) = (run(false), run(true));
    assert!(on >= off + 0.25, "lined up off {off}, on {on}");
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The global placement's start stages taken alone.

use super::field::Field;
use super::start::{add_fillers, bodies, fixed_charge};
use super::*;
use crate::problem::{Component, Pin, Side};

fn part(side: Side, fixed: bool) -> Component {
    Component {
        name: String::new(),
        body_center: [0.0, 0.0],
        body_size: [2.0, 1.0],
        round: false,
        halo: 0.0,
        pins: vec![
            Pin {
                offset: [-0.6, 0.0],
                net: 0,
            },
            Pin {
                offset: [0.6, 0.0],
                net: 1,
            },
        ],
        side,
        fixed,
        angle_options: vec![0.0, 90.0],
        far_side: Vec::new(),
        far_copper: Vec::new(),
        far_open: Default::default(),
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

/// `components` at (10, 10) on `outline`.
fn board(outline: Vec<Point>, components: Vec<Component>) -> Problem {
    let poses = vec![
        Pose {
            position: [10.0, 10.0],
            angle: 0.0,
        };
        components.len()
    ];
    Problem {
        outline,
        components,
        net_weights: vec![1.0; 2],
        poses,
        spacing: 0.5,
        grid: 0.25,
        edge_margin: 0.5,
        min_spacing: 0.2,
        far_side_pads_only: false,
        escape: None,
        crossings: None,
        pieces: Vec::new(),
        constraints: Default::default(),
    }
}

fn rectangle() -> Vec<Point> {
    vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]
}

/// The immovable charge is the area outside the outline (an L whose cut
/// lies on bin borders, sampled exactly) and each fixed body on its side's
/// field; a board with parts on one side has one field.
#[test]
fn the_fixed_charge_is_what_lies_outside_and_the_fixed_bodies() {
    let outline = vec![
        [0.0, 0.0],
        [40.0, 0.0],
        [40.0, 15.0],
        [20.0, 15.0],
        [20.0, 30.0],
        [0.0, 30.0],
    ];
    let problem = board(
        outline,
        vec![part(Side::Front, true), part(Side::Front, false)],
    );
    let mut field = Field::new(64, problem.bounds());
    let charge = fixed_charge(&problem, &problem.poses, &mut field);
    assert_eq!(charge.side_count, 1);
    let outside: f64 = field.fixed.iter().sum();
    assert!((outside - 300.0).abs() < 1.0e-6, "{outside}");
    let fixed: f64 = charge.fixed[0].iter().sum();
    assert!((fixed - 302.0).abs() < 1.0e-6, "{fixed}");
    assert!((charge.free[0] - (1200.0 - 302.0)).abs() < 1.0e-6);

    // A back part makes a field per side: the fixed front body charges
    // the front's only.
    let problem = board(
        rectangle(),
        vec![part(Side::Front, true), part(Side::Back, false)],
    );
    let mut field = Field::new(64, problem.bounds());
    let charge = fixed_charge(&problem, &problem.poses, &mut field);
    assert_eq!(charge.side_count, 2);
    let sums: Vec<f64> = charge.fixed.iter().map(|side| side.iter().sum()).collect();
    assert!((sums[0] - 2.0).abs() < 1.0e-6, "{sums:?}");
    assert!(sums[1].abs() < 1.0e-9, "{sums:?}");
}

/// Fillers take `whitespace_fill` of a side's whitespace in squares of a
/// typical body's size (a bin at least); none at the default 0.
#[test]
fn fillers_take_their_share_of_the_whitespace() {
    let mut square = part(Side::Front, false);
    square.body_size = [2.0, 2.0];
    let problem = board(rectangle(), vec![square; 4]);
    let movable: Vec<usize> = (0..4).collect();
    let mut field = Field::new(64, problem.bounds());
    let charge = fixed_charge(&problem, &problem.poses, &mut field);
    let mut config = GlobalConfig::default();
    let mut none = bodies(&problem, &problem.poses, &movable, 1);
    add_fillers(&mut none, &config, &field, &charge);
    assert_eq!(none.len(), 4);

    config.whitespace_fill = 0.5;
    let mut filled = bodies(&problem, &problem.poses, &movable, 1);
    add_fillers(&mut filled, &config, &field, &charge);
    // Bodies of 4 mm², so fillers of 2 mm a side: half of the 1184 mm² of
    // whitespace is 148 of them.
    assert_eq!(filled.len(), 4 + 148);
    assert!(filled[4..].iter().all(|body| body.component.is_none()));
}

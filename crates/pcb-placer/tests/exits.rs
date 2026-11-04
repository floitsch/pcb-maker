// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The pad-exits repair (`pcb_placer::exits`): a part whose pad another
//! part closes at one orientation turns to one where it is open; where no
//! pad is closed nothing moves; and `place` with the switch places as
//! without it when nothing is closed.

use pcb_placer::exits::{ClosedPad, ExitSession, PadExits, repair};
use pcb_placer::{Component, Config, Pin, Pose, Problem, Side, place};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
        angle_options: if fixed {
            vec![0.0]
        } else {
            vec![0.0, 90.0, 180.0, 270.0]
        },
        far_side: Vec::new(),
        far_open: Default::default(),
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

/// U1 (a fine-pitch part, movable, at 0 degrees) between two fixed
/// connectors J1 and J2 it is wired to on both sides, so that turning it
/// costs no wire; J3, a fixed part close above U1.
fn board() -> Problem {
    let pin = |offset: [f64; 2], net: usize| Pin { offset, net };
    let components = vec![
        component(
            "U1",
            [3.0, 3.0],
            vec![pin([-1.0, 0.0], 0), pin([1.0, 0.0], 1)],
            false,
        ),
        component("J1", [2.0, 2.0], vec![pin([0.0, 0.0], 0)], true),
        component("J2", [2.0, 2.0], vec![pin([0.0, 0.0], 1)], true),
        component("J3", [2.0, 1.0], vec![pin([0.0, 0.0], 2)], true),
    ];
    let poses = vec![
        Pose {
            position: [20.0, 10.0],
            angle: 0.0,
        },
        Pose {
            position: [5.0, 10.0],
            angle: 0.0,
        },
        Pose {
            position: [35.0, 10.0],
            angle: 0.0,
        },
        Pose {
            position: [20.0, 13.0],
            angle: 0.0,
        },
    ];
    Problem {
        outline: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 20.0], [0.0, 20.0]],
        components,
        net_weights: vec![1.0; 3],
        poses,
        spacing: 0.5,
        grid: 0.5,
        edge_margin: 0.0,
        min_spacing: 0.0,
        far_side_pads_only: false,
        escape: None,
        crossings: None,
        pieces: Vec::new(),
        constraints: Default::default(),
    }
}

/// The router's answer as a stand-in: U1's pad 2 faces J3's pad and is
/// closed by it where U1 stands at `closed_at` degrees (anywhere within
/// the repair's reach: no small move opens it, a turn does; no other
/// part's pad closes anything). Counts the questions.
fn exits(closed_at: Option<f64>, asked: Arc<AtomicUsize>) -> PadExits {
    PadExits {
        factory: Arc::new(move || -> ExitSession {
            let asked = asked.clone();
            Box::new(move |poses: &[Pose], parts: &[usize]| {
                asked.fetch_add(1, Ordering::Relaxed);
                let u1 = poses[0];
                let closed = parts.contains(&0)
                    && closed_at.is_some_and(|angle| (u1.angle - angle).abs() < 1.0e-9);
                let pads = if closed {
                    vec![ClosedPad {
                        part: 0,
                        pad: "U1.2".into(),
                        net: "B".into(),
                        layer: 0,
                        blocked_by: vec!["J3.1".into(), "U1.3".into()],
                        blockers: vec![3],
                        own_only: false,
                    }]
                } else {
                    Vec::new()
                };
                Some((pads, 1000))
            })
        }),
        reach: 1.0,
        candidates: 16,
    }
}

#[test]
fn a_part_whose_pad_another_part_closes_turns_to_open_it() {
    let problem = board();
    let mut poses = problem.poses.clone();
    let asked = Arc::new(AtomicUsize::new(0));
    let outcome = repair(&problem, &mut poses, &[], &exits(Some(0.0), asked.clone())).unwrap();
    // Closed (the part is illegal by this rule) before, open after.
    assert_eq!(outcome.closed_before, 1);
    assert!(outcome.closed.is_empty(), "{:?}", outcome.closed);
    assert_eq!(outcome.moves.len(), 1);
    let moved = &outcome.moves[0];
    assert_eq!(moved.part, 0, "U1 moves (J3 is fixed)");
    assert_eq!(moved.opened, vec!["U1.2".to_string()]);
    assert_eq!(poses[0], moved.to);
    // The cheapest open candidate: a turn in place (a turn moved along
    // the row costs as much wire, and more distance).
    assert_eq!(poses[0].position, problem.poses[0].position);
    assert_ne!(poses[0].angle, 0.0);
    // The other parts stay.
    assert_eq!(poses[1..], problem.poses[1..]);
    assert!(asked.load(Ordering::Relaxed) >= 2);
}

#[test]
fn with_no_closed_pad_nothing_moves() {
    let problem = board();
    let mut poses = problem.poses.clone();
    let asked = Arc::new(AtomicUsize::new(0));
    let outcome = repair(&problem, &mut poses, &[], &exits(None, asked.clone())).unwrap();
    assert_eq!(outcome.closed_before, 0);
    assert!(outcome.closed.is_empty() && outcome.moves.is_empty());
    assert_eq!(outcome.tried, 0);
    assert_eq!(poses, problem.poses);
    assert_eq!(asked.load(Ordering::Relaxed), 1, "one question: the census");
}

#[test]
fn a_footprints_own_closed_pad_moves_nothing() {
    let problem = board();
    let mut poses = problem.poses.clone();
    let own = PadExits {
        factory: Arc::new(|| -> ExitSession {
            Box::new(|_: &[Pose], parts: &[usize]| {
                let pads = parts
                    .contains(&0)
                    .then(|| ClosedPad {
                        part: 0,
                        pad: "U1.2".into(),
                        net: "B".into(),
                        layer: 0,
                        blocked_by: vec!["U1.3".into()],
                        blockers: Vec::new(),
                        own_only: true,
                    })
                    .into_iter()
                    .collect();
                Some((pads, 10))
            })
        }),
        reach: 1.0,
        candidates: 16,
    };
    let outcome = repair(&problem, &mut poses, &[], &own).unwrap();
    assert_eq!(outcome.closed.len(), 1);
    assert!(outcome.moves.is_empty());
    assert_eq!(poses, problem.poses);
}

#[test]
fn place_with_the_switch_and_nothing_closed_places_as_without_it() {
    let problem = board();
    let mut config = Config::new();
    config.work_limit = Some(50_000_000);
    let plain = place(&problem, &config);
    assert!(plain.exits.is_none());
    config.pad_exits = Some(exits(None, Arc::new(AtomicUsize::new(0))));
    let checked = place(&problem, &config);
    assert_eq!(plain.poses, checked.poses);
    assert_eq!(
        plain.work, checked.work,
        "the repair's work is booked apart"
    );
    let outcome = checked.exits.expect("the switch's outcome");
    assert!(outcome.closed.is_empty() && outcome.moves.is_empty());
    assert!(outcome.work > 0);
}

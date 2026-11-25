// Copyright (C) 2026 Florian Loitsch.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Why a result leaves terminals open (open.rs): each reason in a board
//! that has it.

use super::*;
use crate::board::{Net, Obstacle, ObstacleKind, RuleClass, Terminal};
use crate::geometry::Shape;

/// One layer, 30 x 20 mm: per net its pads (round, 0.3 mm), labelled
/// `NAME.INDEX`, and keepouts (centre, half size) that stop every track.
fn board_of(nets: &[(&str, Vec<[f64; 2]>)], keepouts: &[([f64; 2], [f64; 2])]) -> Board {
    let class = RuleClass {
        trace_width: 0.25,
        clearance: 0.2,
        via_diameter: 0.6,
        via_drill: 0.3,
        layers: 0,
    };
    let mut obstacles = Vec::new();
    let mut board_nets = Vec::new();
    for (id, (name, pads)) in nets.iter().enumerate() {
        let mut terminals = Vec::new();
        for (index, at) in pads.iter().enumerate() {
            let label = format!("{name}.{index}");
            obstacles.push(Obstacle {
                shape: Shape::Circle {
                    center: *at,
                    radius: 0.3,
                },
                layers: 1,
                kind: ObstacleKind::Copper,
                net: Some(id as NetId),
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: label.clone(),
            });
            terminals.push(Terminal {
                anchor: *at,
                layers: 1,
                pad: obstacles.len() - 1,
                contact: None,
                label,
            });
        }
        board_nets.push(Net {
            name: (*name).into(),
            class: 0,
            terminals,
        });
    }
    for (center, half) in keepouts {
        obstacles.push(Obstacle {
            shape: Shape::rectangle(*center, *half, 0.0),
            layers: 1,
            kind: ObstacleKind::Keepout,
            net: None,
            clearance: 0.0,
            clearance_override: None,
            plated: false,
            blocks_tracks: true,
            blocks_vias: true,
            label: "wall".into(),
        });
    }
    Board {
        fixed_pad_route_clearance: Default::default(),
        layer_count: 1,
        outline: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
        edge_clearance: 0.1,
        hole_clearance: 0.25,
        hole_to_hole: 0.25,
        classes: vec![class],
        neck_width: 0.25,
        obstacles,
        nets: board_nets,
        planes: Vec::new(),
        solid_pads: Vec::new(),
        isolated_pads: Vec::new(),
        solid_pad_obstacles: Vec::new(),
        isolated_pad_obstacles: Vec::new(),
        pad_spokes: Default::default(),
        merged_pads: Vec::new(),
        no_via_pads: Vec::new(),
        fill_passes: Vec::new(),
        pin_row_gaps: Vec::new(),
        terminal_copper: Vec::new(),
        attachment: Default::default(),
        via_spans: Vec::new(),
    }
}

fn config() -> Config {
    Config {
        pitches: vec![0.1],
        ..Config::default()
    }
}

/// The result's open terminals as (label, reason name).
fn open_of(result: &RoutingResult) -> Vec<(String, String)> {
    result
        .diagnostics
        .open
        .iter()
        .map(|open| (open.label.clone(), open.reason.name()))
        .collect()
}

/// Net A from (0.6, 10) to (29.4, 10) and net B from (15, 0.6) to
/// (15, 19.4) on one layer: each one's track walls the board from edge to
/// edge (its pads leave no room for a track past them), so one crosses
/// the other or stays open.
fn crossing() -> Board {
    board_of(
        &[
            ("A", vec![[0.6, 10.0], [29.4, 10.0]]),
            ("B", vec![[15.0, 0.6], [15.0, 19.4]]),
        ],
        &[],
    )
}

#[test]
fn a_pad_walled_in_by_keepouts_has_no_path_on_the_board() {
    // G.1 sits in a closed ring of keepouts: the negotiation's
    // board-wide search, other copper passable, does not reach it.
    let board = board_of(
        &[(
            "G",
            vec![[3.0, 10.0], [8.0, 10.0], [27.0, 4.0], [27.0, 16.0]],
        )],
        &[
            ([8.0, 8.5], [1.7, 0.2]),
            ([8.0, 11.5], [1.7, 0.2]),
            ([6.5, 10.0], [0.2, 1.7]),
            ([9.5, 10.0], [0.2, 1.7]),
        ],
    );
    let result = Router::new(&board, &config()).run();
    assert_eq!(
        result.status[0],
        NetStatus::Partial {
            unconnected_terminals: 1
        }
    );
    assert_eq!(
        open_of(&result),
        vec![("G.1".to_string(), "no_path_static".to_string())]
    );
}

#[test]
fn a_net_with_one_pad_of_access_is_never_searched() {
    // B.1 lies inside a keepout: no lattice node, so B has one terminal
    // with access and is not routable. Its dead pad is `no_access`, its
    // other pad `net_not_routable`; A routes.
    let board = board_of(
        &[
            ("A", vec![[3.0, 4.0], [27.0, 4.0]]),
            ("B", vec![[3.0, 14.0], [20.0, 14.0]]),
        ],
        &[([20.0, 14.0], [1.5, 1.5])],
    );
    let result = Router::new(&board, &config()).run();
    assert_eq!(result.status[0], NetStatus::Routed);
    assert_eq!(result.status[1], NetStatus::Unreachable);
    assert_eq!(
        open_of(&result),
        vec![
            ("B.0".to_string(), "net_not_routable".to_string()),
            ("B.1".to_string(), "no_access".to_string()),
        ]
    );
}

#[test]
fn the_hard_repair_finds_no_path_past_the_other_nets_copper() {
    // A and B cannot both be routed: the negotiation ends with them in
    // conflict, the hard repair rips the more contested up and finds no
    // path around the other's copper on the whole board. Without the
    // trouble repair, which would trade one for the other.
    let config = Config {
        trouble_repair: false,
        ..config()
    };
    let result = Router::new(&crossing(), &config).run();
    let open = &result.diagnostics.open;
    assert!(!open.is_empty(), "{:?}", result.status);
    let net = open[0].net;
    assert!(open.iter().all(|terminal| terminal.net == net));
    assert!(
        open.iter()
            .all(|terminal| terminal.reason == OpenReason::NoPathHard("board")),
        "{open:?}"
    );
    // The repair's entry says it was in conflict with the other net,
    // and where.
    let entry = result
        .diagnostics
        .repair
        .iter()
        .find(|entry| entry.net == net)
        .expect("the open net was taken up by the repair");
    assert!(entry.conflicted);
    assert!(entry.skipped.is_empty());
    assert_eq!(entry.conflicted_with, vec![1 - net]);
    let (layer, at) = entry.conflict_at.expect("a conflict's place");
    assert_eq!(layer, 0);
    assert!(at[0] > 0.0 && at[0] < 30.0 && at[1] > 0.0 && at[1] < 20.0);
}

#[test]
fn a_repair_skipped_past_the_work_deadline_says_so() {
    // Routed against each other at no price, then finished past the
    // attempt's work deadline: the hard repair rips the conflicted net
    // up and does not search it again.
    let board = crossing();
    let mut router = Router::new(&board, &config());
    for net in 0..2 {
        router.route_net_seq(net, 0.0, false, 1.0);
        router.stamp(net);
    }
    assert!(!router.conflicts(0).is_empty());
    router.config.deadline_work = Some(0.0);
    let result = router.finish(&[0, 1]);
    let open = &result.diagnostics.open;
    assert!(!open.is_empty(), "{:?}", result.status);
    assert!(
        open.iter()
            .all(|terminal| terminal.reason == OpenReason::RepairSkipped("work_deadline")),
        "{open:?}"
    );
    let entry = result
        .diagnostics
        .repair
        .iter()
        .find(|entry| entry.net == open[0].net)
        .unwrap();
    assert_eq!(entry.skipped, "work_deadline");
    assert_eq!(entry.conflicted_with, vec![1 - open[0].net]);
}

#[test]
fn a_terminal_a_phase_opens_is_lost_there() {
    // A routed alone; a phase that rips it up leaves its terminals open,
    // and they are lost there, whatever was recorded before.
    let board = crossing();
    let mut router = Router::new(&board, &config());
    router.route_net_seq(0, 0.0, false, 1.0);
    router.stamp(0);
    assert!(router.nets[0].complete);
    router.open_reasons_start();
    router.open_reasons[0][1] = OpenReason::NoPathStatic;
    let before = router.open_flags();
    router.rip_up(0);
    router.note_lost(&before, "clean_up", false);
    assert_eq!(
        router.open_reasons[0],
        vec![OpenReason::Lost("clean_up"); 2]
    );
    // With `unknown_only` (the hard repair's own phase) a reason
    // recorded stays.
    router.open_reasons_start();
    router.open_reasons[0][1] = OpenReason::NoPathHard("board");
    router.route_net_seq(0, 0.0, false, 1.0);
    router.stamp(0);
    let before = router.open_flags();
    router.rip_up(0);
    router.note_lost(&before, "hard_repair", true);
    assert_eq!(
        router.open_reasons[0],
        vec![
            OpenReason::Lost("hard_repair"),
            OpenReason::NoPathHard("board")
        ]
    );
    // A terminal open before the phase is not lost there.
    let before = router.open_flags();
    router.note_lost(&before, "bends", false);
    assert_eq!(router.open_reasons[0][0], OpenReason::Lost("hard_repair"));
}

#[test]
fn the_copper_checks_terminals_apart_are_named() {
    // A routed net the copper check finds apart: its terminals off the
    // largest piece are open by the check, whatever the router's flags.
    let board = crossing();
    let mut router = Router::new(&board, &config());
    router.route_net_seq(0, 0.0, false, 1.0);
    router.stamp(0);
    let status = vec![
        NetStatus::Partial {
            unconnected_terminals: 1,
        },
        NetStatus::Routed,
    ];
    let open = router.open_terminals(&status, &[(0, vec![1])]);
    assert_eq!(
        open,
        vec![OpenTerminal {
            net: 0,
            terminal: 1,
            label: "A.1".into(),
            reason: OpenReason::CopperCheck,
        }]
    );
}

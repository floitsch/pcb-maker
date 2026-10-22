// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use super::*;

/// A walk stamped with each footprint's leading edge after the first
/// node lists the same cells in the same order as with whole footprints,
/// near the lattice's edges too.
#[test]
fn fresh_footprints_mark_as_whole_ones() {
    let (nx, ny) = (23i64, 17i64);
    let words = ((nx * ny) as usize).div_ceil(64);
    let disc_offsets = disc(0.33, 0.1);
    let stamps = Stamps {
        stub_to_trace: Vec::new(),
        stub_to_via: Vec::new(),
        trace_to_trace: disc_offsets.clone(),
        trace_to_via: disc(0.21, 0.1),
        via_to_trace: Vec::new(),
        via_to_via: Vec::new(),
        diagonal_to_trace: diagonal_extra(0.33, 0.1),
        diagonal_to_via: Vec::new(),
        diagonal_block: diagonal_block(0.33, 0.1),
        diagonal_block_via: [Vec::new(), Vec::new()],
        stub_diagonal_block: [Vec::new(), Vec::new()],
        fresh: Fresh::default(),
    };
    let fresh = Fresh::of(&stamps);
    let mut state = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..200 {
        let (mut x, mut y) = (0i64, 0i64);
        let mut cells = vec![0u32];
        for _ in 0..60 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let (dx, dy) = DIRECTIONS[(state >> 61) as usize];
            let (tx, ty) = (x + dx as i64, y + dy as i64);
            if tx >= 0 && ty >= 0 && tx < nx && ty < ny {
                (x, y) = (tx, ty);
                cells.push((y * nx + x) as u32);
            }
        }
        let (mut whole_marks, mut fresh_marks) = (vec![0u64; words], vec![0u64; words]);
        let (mut whole, mut ahead) = (Vec::new(), Vec::new());
        for (index, cell) in cells.iter().enumerate() {
            mark_footprint(
                &mut whole_marks,
                words,
                &mut whole,
                0,
                *cell,
                &disc_offsets,
                (1, 1),
                (nx, ny),
                fresh.reach,
            );
            let offsets = if index == 0 {
                &disc_offsets
            } else {
                let previous = cells[index - 1] as i64;
                let (dx, dy) = (
                    *cell as i64 % nx - previous % nx,
                    *cell as i64 / nx - previous / nx,
                );
                &fresh.trace_to_trace[((dx + 1) * 3 + dy + 1) as usize]
            };
            mark_footprint(
                &mut fresh_marks,
                words,
                &mut ahead,
                0,
                *cell,
                offsets,
                (1, 1),
                (nx, ny),
                fresh.reach,
            );
        }
        assert_eq!(whole, ahead);
        assert_eq!(whole_marks, fresh_marks);
    }
    // The bisectors of a diagonal run share no cell (`Router::stamp`
    // stamps each whole).
    for radius in [0.1, 0.25, 0.33, 0.6, 1.4] {
        let bisector = diagonal_extra(radius, 0.1);
        assert!(
            bisector
                .iter()
                .all(|(i, j)| !bisector.contains(&(i + 1, j + 1)))
        );
    }
}

#[test]
fn a_tiny_budget_has_the_guard_floor_before_its_wall_guard() {
    // Batch_1 in route mode: a budget of 0.2 s used to be cut by the
    // guard at 0.8 s, inside its first pour analysis.
    assert!(!budget_guard_fires(0.9, 0.2));
    assert!(!budget_guard_fires(BUDGET_GUARD_FLOOR, 0.0));
    assert!(budget_guard_fires(BUDGET_GUARD_FLOOR + 0.1, 0.0));
    // A budget above the floor keeps its four times.
    assert!(!budget_guard_fires(39.0, 10.0));
    assert!(budget_guard_fires(41.0, 10.0));
}

#[test]
fn key_heap_pops_in_order() {
    let mut heap = KeyHeap::default();
    let mut reference = std::collections::BinaryHeap::new();
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for round in 0..20_000 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        if round % 3 == 2 {
            assert_eq!(
                heap.pop(),
                reference.pop().map(|std::cmp::Reverse(key)| key)
            );
        } else {
            let key = state >> 40;
            heap.push(key);
            reference.push(std::cmp::Reverse(key));
        }
    }
    while let Some(std::cmp::Reverse(key)) = reference.pop() {
        assert_eq!(heap.pop(), Some(key));
    }
    assert_eq!(heap.pop(), None);
}

#[test]
fn key_heap_pops_as_binary_heap_across_clears() {
    // A search's pattern: keys mostly at or above the last popped one,
    // a few below it, repeats, and the heap cleared between searches.
    let mut heap = KeyHeap::default();
    let mut reference = std::collections::BinaryHeap::new();
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut last = 0u64;
    for round in 0..200_000u64 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        if round % 40_000 == 39_999 {
            heap.clear();
            reference.clear();
            last = 0;
            continue;
        }
        if (state >> 60).is_multiple_of(3) {
            let popped = heap.pop();
            assert_eq!(popped, reference.pop().map(|std::cmp::Reverse(key)| key));
            if let Some(key) = popped {
                last = key;
            }
        } else {
            let noise = (state >> 20) & 0xff_ffff;
            let key = match (state >> 8) % 16 {
                0 => last.saturating_sub(noise),
                1 => last,
                _ => last + (noise << ((state >> 4) % 24)),
            };
            heap.push(key);
            reference.push(std::cmp::Reverse(key));
        }
    }
    while let Some(std::cmp::Reverse(key)) = reference.pop() {
        assert_eq!(heap.pop(), Some(key));
    }
    assert_eq!(heap.pop(), None);
}
use crate::board::{Net, Obstacle, ObstacleKind, Plane, RuleClass, Terminal};
use crate::geometry::Shape;

fn copper(shape: Shape, net: NetId, label: &str) -> Obstacle {
    Obstacle {
        shape,
        layers: 1,
        kind: ObstacleKind::Copper,
        net: Some(net),
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks: true,
        blocks_vias: true,
        label: label.into(),
    }
}

/// A pour on one layer, cut by two parallel walls of another net
/// running diagonally to the lattice: between them only the nodes on
/// one diagonal keep the brush's clearance (0.45 mm from both walls
/// where 0.425 is needed; the next diagonals are 0.071 mm closer).
/// `notch` adds a small disc of the other net beside one diagonal step
/// that passes it closer than the clearance while both of the step's
/// nodes keep it.
fn diagonal_channel(notch: bool) -> (Router, [f64; 2]) {
    let class = RuleClass {
        trace_width: 0.25,
        clearance: 0.2,
        via_diameter: 0.6,
        via_drill: 0.3,
    };
    let mut obstacles = vec![
        copper(
            Shape::Circle {
                center: [0.5, 7.5],
                radius: 0.3,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [7.5, 0.5],
                radius: 0.3,
            },
            1,
            "Q.1",
        ),
    ];
    // Walls with radius 0.1 along y = x +- 0.6364 (0.45 mm from the
    // diagonal y = x on both sides, perpendicular).
    let offset = 0.45 * std::f64::consts::SQRT_2;
    for sign in [-1.0, 1.0] {
        obstacles.push(copper(
            Shape::Capsule {
                start: [-1.0, -1.0 + sign * offset],
                end: [9.0, 9.0 + sign * offset],
                radius: 0.1,
            },
            1,
            "wall",
        ));
    }
    // The step from (3.0, 3.0) to (3.1, 3.1) has its middle at (3.05,
    // 3.05); a disc of radius 0.075 at 0.397 mm from it across the
    // diagonal comes within 0.397 - 0.075 = 0.322 < 0.325 of the step
    // and 0.328 of either node.
    let middle = [3.05, 3.05];
    let across = [
        -std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    ];
    if notch {
        obstacles.push(copper(
            Shape::Circle {
                center: [middle[0] + across[0] * 0.397, middle[1] + across[1] * 0.397],
                radius: 0.075,
            },
            1,
            "notch",
        ));
    }
    let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal {
        anchor: at,
        layers: 1,
        pad,
        contact: None,
        label: label.into(),
    };
    let board = Board {
        layer_count: 1,
        outline: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]],
        edge_clearance: 0.1,
        hole_clearance: 0.25,
        hole_to_hole: 0.25,
        classes: vec![class],
        neck_width: 0.25,
        obstacles,
        nets: vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![terminal(0, [0.5, 7.5], "P.1")],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![terminal(1, [7.5, 0.5], "Q.1")],
            },
        ],
        planes: vec![Plane {
            net: 0,
            class: 0,
            layer: 0,
            polygon: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]],
            excluded: Vec::new(),
            above: Vec::new(),
            connect: true,
            thermal_reach: 0.0,
            thermal_gap: 0.0,
            exclusive: false,
            solid: false,
            hatch: None,
        }],
        solid_pads: Vec::new(),
        isolated_pads: Vec::new(),
        solid_pad_obstacles: Vec::new(),
        isolated_pad_obstacles: Vec::new(),
        pad_spokes: Default::default(),
        merged_pads: Vec::new(),
        no_via_pads: Vec::new(),
        fill_passes: Vec::new(),
    };
    let config = Config {
        pitches: vec![0.1],
        ..Config::default()
    };
    (Router::new(&board, &config), middle)
}

/// One layer, net G: pads at the left, at the right top and bottom,
/// and one near the left pad walled in by a keepout ring, which no
/// track reaches.
fn walled_in_pad() -> Router {
    let class = RuleClass {
        trace_width: 0.25,
        clearance: 0.2,
        via_diameter: 0.6,
        via_drill: 0.3,
    };
    let pads = [[3.0, 10.0], [8.0, 10.0], [27.0, 4.0], [27.0, 16.0]];
    let mut obstacles: Vec<Obstacle> = pads
        .iter()
        .enumerate()
        .map(|(index, at)| {
            copper(
                Shape::Circle {
                    center: *at,
                    radius: 0.3,
                },
                0,
                &format!("G.{index}"),
            )
        })
        .collect();
    let keepout = |center: [f64; 2], half: [f64; 2]| Obstacle {
        shape: Shape::rectangle(center, half, 0.0),
        layers: 1,
        kind: ObstacleKind::Keepout,
        net: None,
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks: true,
        blocks_vias: true,
        label: "ring".into(),
    };
    for (center, half) in [
        ([8.0, 8.5], [1.7, 0.2]),
        ([8.0, 11.5], [1.7, 0.2]),
        ([6.5, 10.0], [0.2, 1.7]),
        ([9.5, 10.0], [0.2, 1.7]),
    ] {
        obstacles.push(keepout(center, half));
    }
    let terminals = pads
        .iter()
        .enumerate()
        .map(|(index, at)| Terminal {
            anchor: *at,
            layers: 1,
            pad: index,
            contact: None,
            label: format!("G.{index}"),
        })
        .collect();
    let board = Board {
        layer_count: 1,
        outline: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
        edge_clearance: 0.1,
        hole_clearance: 0.25,
        hole_to_hole: 0.25,
        classes: vec![class],
        neck_width: 0.25,
        obstacles,
        nets: vec![Net {
            name: "G".into(),
            class: 0,
            terminals,
        }],
        planes: Vec::new(),
        solid_pads: Vec::new(),
        isolated_pads: Vec::new(),
        solid_pad_obstacles: Vec::new(),
        isolated_pad_obstacles: Vec::new(),
        pad_spokes: Default::default(),
        merged_pads: Vec::new(),
        no_via_pads: Vec::new(),
        fill_passes: Vec::new(),
    };
    let config = Config {
        pitches: vec![0.1],
        ..Config::default()
    };
    Router::new(&board, &config)
}

/// One net on one layer with two pads either side of a wall across the
/// whole board (jiran-ble-lite's halves, which only a frame no track
/// crosses joins).
fn halves(apart_trees: bool) -> Router {
    let class = RuleClass {
        trace_width: 0.25,
        clearance: 0.2,
        via_diameter: 0.6,
        via_drill: 0.3,
    };
    let pads = [[3.0, 10.0], [8.0, 4.0], [22.0, 10.0], [27.0, 16.0]];
    let mut obstacles: Vec<Obstacle> = pads
        .iter()
        .enumerate()
        .map(|(index, at)| {
            copper(
                Shape::Circle {
                    center: *at,
                    radius: 0.3,
                },
                0,
                &format!("G.{index}"),
            )
        })
        .collect();
    obstacles.push(Obstacle {
        shape: Shape::rectangle([15.0, 10.0], [0.5, 11.0], 0.0),
        layers: 1,
        kind: ObstacleKind::Keepout,
        net: None,
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks: true,
        blocks_vias: true,
        label: "wall".into(),
    });
    let terminals = pads
        .iter()
        .enumerate()
        .map(|(index, at)| Terminal {
            anchor: *at,
            layers: 1,
            pad: index,
            contact: None,
            label: format!("G.{index}"),
        })
        .collect();
    let board = Board {
        layer_count: 1,
        outline: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
        edge_clearance: 0.1,
        hole_clearance: 0.25,
        hole_to_hole: 0.25,
        classes: vec![class],
        neck_width: 0.25,
        obstacles,
        nets: vec![Net {
            name: "G".into(),
            class: 0,
            terminals,
        }],
        planes: Vec::new(),
        solid_pads: Vec::new(),
        isolated_pads: Vec::new(),
        solid_pad_obstacles: Vec::new(),
        isolated_pad_obstacles: Vec::new(),
        pad_spokes: Default::default(),
        merged_pads: Vec::new(),
        no_via_pads: Vec::new(),
        fill_passes: Vec::new(),
    };
    let config = Config {
        pitches: vec![0.1],
        apart_trees,
        ..Config::default()
    };
    Router::new(&board, &config)
}

#[test]
fn terminals_no_path_reaches_are_joined_among_themselves() {
    // The left pair is the net's tree; the right pair no path reaches from
    // it. Joined apart, the right pads get a branch of their own, KiCad's
    // one unconnected item between the two trees instead of two; the
    // net's count still calls them unconnected and the net incomplete.
    let right_branch = |router: &Router| {
        router.nets[0]
            .branches
            .iter()
            .any(|branch| branch.start_terminal >= 2 || branch.end_terminal >= 2)
    };
    let mut router = halves(true);
    router.route_net_seq(0, 1.0, false, 1.0);
    router.stamp(0);
    assert_eq!(router.nets[0].unreachable, vec![false, false, true, true]);
    assert_eq!(router.nets[0].connected, vec![true, true, false, false]);
    assert!(!router.nets[0].complete);
    assert!(right_branch(&router));
    // So does the hard repair after the negotiation.
    router.rip_up(0);
    router.resolve_remaining(&[0]);
    assert_eq!(router.nets[0].connected, vec![true, true, false, false]);
    assert!(right_branch(&router));
    // Without, the right pads stay apart from each other too.
    let mut router = halves(false);
    router.route_net_seq(0, 1.0, false, 1.0);
    router.stamp(0);
    assert_eq!(router.nets[0].connected, vec![true, true, false, false]);
    assert!(!right_branch(&router));
}

#[test]
fn a_walled_in_pad_does_not_stop_the_rest_of_its_net() {
    let mut router = walled_in_pad();
    // The first search connects what it reaches, and the board-wide
    // search for the walled-in pad fails.
    router.route_net_seq(0, 1.0, false, 1.0);
    router.stamp(0);
    assert_eq!(router.nets[0].connected, vec![true, false, true, true]);
    assert_eq!(router.nets[0].hopeless_searches, 1);
    assert_eq!(router.nets[0].unreachable, vec![false, true, false, false]);
    // After a second such failure the net keeps to its corridors; the
    // walled-in pad is no target any more.
    router.rip_up(0);
    router.nets[0].hopeless_searches = 2;
    router.route_net_seq(0, 1.0, false, 1.0);
    router.stamp(0);
    assert_eq!(router.nets[0].connected, vec![true, false, true, true]);
    assert!(!router.nets[0].complete);
    // The hard repair after negotiation keeps the incomplete net's tree
    // and only tries to join the rest to it.
    let nodes = |router: &Router| {
        router.nets[0]
            .branches
            .iter()
            .map(|branch| branch.nodes.clone())
            .collect::<Vec<_>>()
    };
    let before = nodes(&router);
    assert!(!before.is_empty());
    router.resolve_remaining(&[0]);
    assert_eq!(&nodes(&router)[..before.len()], &before[..]);
    assert_eq!(router.nets[0].connected, vec![true, false, true, true]);
}

/// One layer: net A along y = 10 from x = 3 over 15 to 27, nets B and
/// C straight across its right half at x = 19 and 23.
fn crossed_net() -> Router {
    let class = RuleClass {
        trace_width: 0.25,
        clearance: 0.2,
        via_diameter: 0.6,
        via_drill: 0.3,
    };
    let nets: [(&str, Vec<[f64; 2]>); 3] = [
        ("A", vec![[3.0, 10.0], [15.0, 10.0], [27.0, 10.0]]),
        ("B", vec![[19.0, 3.0], [19.0, 17.0]]),
        ("C", vec![[23.0, 3.0], [23.0, 17.0]]),
    ];
    let mut obstacles = Vec::new();
    let mut board_nets = Vec::new();
    for (id, (name, pads)) in nets.iter().enumerate() {
        let mut terminals = Vec::new();
        for (index, at) in pads.iter().enumerate() {
            let label = format!("{name}.{index}");
            obstacles.push(copper(
                Shape::Circle {
                    center: *at,
                    radius: 0.3,
                },
                id as NetId,
                &label,
            ));
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
    let board = Board {
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
    };
    let config = Config {
        pitches: vec![0.1],
        ..Config::default()
    };
    Router::new(&board, &config)
}

#[test]
fn the_hard_repair_keeps_the_branches_out_of_conflict() {
    let mut router = crossed_net();
    // Routed against each other at no price: B and C cross A.
    for net in 0..3 {
        router.route_net_seq(net, 0.0, false, 1.0);
        router.stamp(net);
    }
    assert!(router.nets.iter().all(|state| state.complete));
    let conflicts: Vec<usize> = (0..3).map(|net| router.conflicts(net).len()).collect();
    assert!(
        conflicts[0] > conflicts[1].max(conflicts[2]),
        "{conflicts:?}"
    );
    // The left branch (3 to 15) is out of conflict. A net stuck in
    // conflict for eight iterations is ripped up whole by the
    // negotiation; the repair keeps what is out of conflict.
    let left: Vec<Vec<Node>> = router.nets[0]
        .branches
        .iter()
        .filter(|branch| {
            branch
                .nodes
                .iter()
                .all(|node| router.grid.center_of(node.cell as usize)[0] < 16.0)
        })
        .map(|branch| branch.nodes.clone())
        .collect();
    assert!(!left.is_empty());
    router.nets[0].stuck = 8;
    router.resolve_remaining(&[0, 1, 2]);
    assert!(left.iter().all(|nodes| {
        router.nets[0]
            .branches
            .iter()
            .any(|branch| branch.nodes == *nodes)
    }));
    // B and C stay as they were: only A was in the way.
    assert!(router.nets[1].complete && router.nets[2].complete);
}

#[test]
fn a_budget_in_seconds_buys_expansions_at_the_calibration() {
    // A budget of seconds is `expansions_per_second()` expansions a
    // second, and the work and the `profile rate:` line count the same
    // expansions back: the calibration enters only through the rate.
    let mut router = crossed_net();
    router.probe(1.0e-3);
    let expansions = router.expansions();
    assert!(expansions > 0);
    assert_eq!(
        router.work_seconds(),
        expansions as f64 / expansions_per_second()
    );
    assert_eq!(router.profile.phase_expansions[0], expansions);
    assert!((router.profile.phase_search[0] - router.work_seconds()).abs() < 1.0e-12);
    let line = router.profile.rate_line();
    assert!(
        line.starts_with(&format!("profile rate: phases {expansions} expansions in ")),
        "{line}"
    );
    assert!(
        line.contains(&format!("rate {:.0}", expansions_per_second())),
        "{line}"
    );
    // A probe allowed fewer expansions than one iteration takes stops
    // after that iteration: its limit is the seconds at the constant.
    let mut short = crossed_net();
    short.probe(1.0 / expansions_per_second());
    assert_eq!(
        short.config.negotiation_expansions, 0,
        "the probe restores the limit"
    );
    assert!(short.expansions() <= expansions);
    assert!(short.iterations <= router.iterations);
}

#[test]
fn pour_analyses_count_as_budget_work() {
    // Each analysis counts the lattice nodes inside its pour, its
    // stamps and its fill probe's items, queries and polygon vertices,
    // in the phase budgets' work and not in the search's; the count is
    // the same on every run, and a copy of the router keeps it.
    let (mut router, _) = diagonal_channel(false);
    // A caller's deadline far ahead: the wall-clock guard of `spent`
    // (`GUARD` times these microsecond budgets) stays out of it, so
    // only the work decides however busy the machine is.
    router.config_mut().deadline =
        Some(std::time::Instant::now() + std::time::Duration::from_secs(3600));
    let fresh = router.clone();
    let clock = router.clock();
    let search = router.work_seconds();
    router.analyze_pours(0);
    router.analyze_pours_full(0, &|_| false);
    let inside = router.nets[0].plane[0]
        .iter()
        .filter(|inside| **inside)
        .count();
    assert!(inside > 0);
    assert_eq!(router.pour_inside(0), inside);
    let own =
        inside as u64 * POUR_NODE_NANOS + router.nets[0].stamped.len() as u64 * POUR_STAMP_NANOS;
    assert_eq!(router.pour_analysis_nanos(0), own);
    let probe = router.probe_nanos();
    let expected = (2 * own + probe) as f64 / 1.0e9;
    assert!((router.budget_work() - clock.0 - expected).abs() < 1.0e-12);
    assert_eq!(router.work_seconds(), search);
    let copy = router.clone();
    assert_eq!(copy.budget_work(), router.budget_work());
    fresh.analyze_pours(0);
    fresh.analyze_pours_full(0, &|_| false);
    assert_eq!(fresh.budget_work(), router.budget_work());
    // A budget smaller than that work is spent by it alone.
    assert!(router.spent(&clock, 0.5 * expected));
    assert!(!router.spent(&clock, 2.0 * expected));
    // A net without a pour counts nothing.
    assert_eq!(router.pour_analysis_nanos(1), 0);
}

#[test]
fn a_diagonal_neck_of_the_fill_holds_together() {
    let (router, _) = diagonal_channel(false);
    let grid = router.grid().clone();
    let (pours, _, _) = router.analyze_pours(0);
    let piece = |x: f64, y: f64| pours.label[0][grid.nearest_node([x, y]).unwrap()];
    // The lattice lies on the diagonal: its nodes are free, their side
    // neighbours are not.
    let on = grid.nearest_node([2.0, 2.0]).unwrap();
    assert!((grid.center_of(on)[0] - grid.center_of(on)[1]).abs() < 1.0e-9);
    assert!(pours.solid[0][on]);
    assert!(!pours.solid[0][grid.nearest_node([2.1, 2.0]).unwrap()]);
    assert!(!pours.solid[0][grid.nearest_node([2.0, 2.1]).unwrap()]);
    // The channel is one piece from end to end, and the fill on either
    // side of the walls is another each.
    assert_ne!(piece(2.0, 2.0), 0);
    assert_eq!(piece(2.0, 2.0), piece(6.0, 6.0));
    assert_ne!(piece(1.0, 6.0), piece(6.0, 1.0));
    assert_ne!(piece(1.0, 6.0), piece(2.0, 2.0));
    assert_eq!(pours.pieces, 3);
}

#[test]
fn a_diagonal_step_passing_other_copper_too_closely_does_not_join() {
    let (router, middle) = diagonal_channel(true);
    let grid = router.grid().clone();
    let (pours, _, _) = router.analyze_pours(0);
    let piece = |x: f64, y: f64| pours.label[0][grid.nearest_node([x, y]).unwrap()];
    // Both nodes of the step stay free; the step between them does not
    // keep the clearance, so the channel falls into two pieces there.
    let (before, after) = (
        [middle[0] - 0.05, middle[1] - 0.05],
        [middle[0] + 0.05, middle[1] + 0.05],
    );
    assert!(pours.solid[0][grid.nearest_node(before).unwrap()]);
    assert!(pours.solid[0][grid.nearest_node(after).unwrap()]);
    assert_ne!(piece(before[0], before[1]), piece(after[0], after[1]));
    assert_eq!(piece(2.0, 2.0), piece(before[0], before[1]));
    assert_eq!(piece(6.0, 6.0), piece(after[0], after[1]));
    assert_eq!(pours.pieces, 4);
}

fn rect(center: [f64; 2], half: [f64; 2]) -> Shape {
    Shape::Polygon {
        points: vec![
            [center[0] - half[0], center[1] - half[1]],
            [center[0] + half[0], center[1] - half[1]],
            [center[0] + half[0], center[1] + half[1]],
            [center[0] - half[0], center[1] + half[1]],
        ],
    }
}

fn plane(net: NetId, layer: usize, thermal: Option<(f64, f64)>) -> Plane {
    Plane {
        net,
        class: 0,
        layer,
        polygon: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        excluded: Vec::new(),
        above: Vec::new(),
        connect: true,
        thermal_reach: thermal.map_or(0.0, |(gap, bridge)| gap + bridge),
        thermal_gap: thermal.map_or(0.0, |(gap, _)| gap),
        exclusive: false,
        solid: false,
        hatch: None,
    }
}

fn board(
    layer_count: usize,
    obstacles: Vec<Obstacle>,
    nets: Vec<Net>,
    planes: Vec<Plane>,
    solid_pads: Vec<String>,
) -> Board {
    Board {
        layer_count,
        outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        edge_clearance: 0.1,
        hole_clearance: 0.25,
        hole_to_hole: 0.25,
        classes: vec![RuleClass {
            trace_width: 0.25,
            clearance: 0.2,
            via_diameter: 0.6,
            via_drill: 0.3,
        }],
        neck_width: 0.25,
        obstacles,
        nets,
        planes,
        solid_pads,
        isolated_pads: Vec::new(),
        solid_pad_obstacles: Vec::new(),
        isolated_pad_obstacles: Vec::new(),
        pad_spokes: Default::default(),
        merged_pads: Vec::new(),
        no_via_pads: Vec::new(),
        fill_passes: Vec::new(),
    }
}

#[test]
fn a_plane_stub_cut_off_from_the_main_piece_is_routed_again() {
    // Two front pads of P reach P's plane on the back through a stub
    // and a via each. While pads must reach the main piece, the stub
    // whose via no longer lands in it (the signals cut it off) does not
    // join its pad to the plane: the pad is routed again, to the main
    // piece; the other pad keeps its stub.
    let mut obstacles = vec![
        copper(
            Shape::Circle {
                center: [2.0, 5.0],
                radius: 0.3,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [8.0, 5.0],
                radius: 0.3,
            },
            0,
            "P.2",
        ),
    ];
    for obstacle in &mut obstacles {
        obstacle.layers = 0b01;
    }
    let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal {
        anchor: at,
        layers: 0b01,
        pad,
        contact: None,
        label: label.into(),
    };
    let board = board(
        2,
        obstacles,
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![
                terminal(0, [2.0, 5.0], "P.1"),
                terminal(1, [8.0, 5.0], "P.2"),
            ],
        }],
        vec![plane(0, 1, None)],
        Vec::new(),
    );
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let grid = router.grid().clone();
    let node = |layer: u8, at: [f64; 2]| Node {
        layer,
        cell: grid.nearest_node(at).unwrap() as u32,
    };
    let stub = |pad: u16, from: [f64; 2], to: [f64; 2]| {
        let steps = ((to[0] - from[0]) / 0.1).round() as usize;
        let mut nodes: Vec<Node> = (0..=steps)
            .map(|step| node(0, [from[0] + 0.1 * step as f64, from[1]]))
            .collect();
        nodes.push(node(1, to));
        Branch {
            nodes,
            start_terminal: pad,
            end_terminal: PLANE_TERMINAL,
        }
    };
    let cut_off = [3.0, 5.0];
    {
        let state = &mut router.nets[0];
        assert!(!state.plane[1].is_empty());
        assert_eq!(state.on_plane, vec![false, false]);
        state.branches = vec![
            stub(0, [2.0, 5.0], cut_off),
            stub(1, [8.0, 5.0], [9.0, 5.0]),
        ];
        // The main piece: the plane but around the first stub's via.
        let target: Vec<bool> = (0..grid.cells())
            .map(|cell| crate::geometry::distance(grid.center_of(cell), cut_off) > 0.5)
            .collect();
        state.plane_target = vec![Vec::new(), target];
    }
    router.stamp(0);
    router.route_net_seq(0, 0.0, true, 1.0);
    let state = &router.nets[0];
    assert!(state.complete);
    assert_eq!(
        state.branches.len(),
        3,
        "{:?}",
        state
            .branches
            .iter()
            .map(|branch| (branch.start_terminal, branch.end_terminal))
            .collect::<Vec<_>>()
    );
    // The new branch leaves pad 1's copper and ends on the main piece.
    let added = &state.branches[2];
    let end = *added.nodes.last().unwrap();
    assert_eq!(added.end_terminal, PLANE_TERMINAL);
    assert!(state.plane_target[end.layer as usize][end.cell as usize]);
    assert!(
        added
            .nodes
            .iter()
            .any(|node| state.branches[0].nodes.contains(node))
            || added.start_terminal == 0
    );
}

#[test]
fn a_spoke_passing_another_nets_copper_too_closely_does_not_count() {
    // P.1 sits in a row between two pads of Q that reach 0.5 mm past
    // it, 0.7 mm apart. Its spokes land in fill that P.2 (a solid pad)
    // holds, but on the way there a brush between the Q pads would come
    // closer than the zone's clearance (0.3 mm, more than the tracks'
    // 0.2) to them: it keeps 0.425 mm from each. KiCad keeps none.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [0.75, 0.15]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [8.5, 8.5],
                radius: 0.4,
            },
            0,
            "P.2",
        ),
        copper(rect([5.0, 4.5], [1.25, 0.15]), 1, "Q.1"),
        copper(rect([5.0, 5.5], [1.25, 0.15]), 1, "Q.2"),
    ];
    let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal {
        anchor: at,
        layers: 1,
        pad,
        contact: None,
        label: label.into(),
    };
    let mut pour = plane(0, 0, Some((0.5, 0.5)));
    pour.class = 1;
    let mut board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [5.0, 5.0], "P.1"),
                    terminal(1, [8.5, 8.5], "P.2"),
                ],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![
                    terminal(2, [5.0, 4.5], "Q.1"),
                    terminal(3, [5.0, 5.5], "Q.2"),
                ],
            },
        ],
        vec![pour],
        vec!["P.2".into()],
    );
    board.classes.push(RuleClass {
        trace_width: 0.25,
        clearance: 0.3,
        via_diameter: 0.6,
        via_drill: 0.3,
    });
    let joined = |spoke_clearance: bool| {
        let router = Router::new(
            &board,
            &Config {
                pitches: vec![0.1],
                spoke_clearance,
                ..Config::default()
            },
        );
        let (pours, mut parent, _, _) = router.analyze_pours_full(0, &|_| false);
        let base = pours.pieces + 1;
        find(&mut parent, base) == find(&mut parent, base + 1)
    };
    // Without the check the probes beyond the row count as spokes.
    assert!(joined(false));
    assert!(!joined(true));
}

#[test]
fn a_pad_whose_stub_is_its_only_copper_in_the_ring_rejoins_the_plane() {
    // P.1 (the last terminal), a through-hole pad cut off before, has a stub inside its own
    // thermal ring down to the second layer: with the stub the pour has
    // two more pieces (the stub's nodes on either layer) than without
    // it. The pad's spokes reach fill that P.3 (solid) holds, so it
    // rejoins the plane and loses the stub; the terminals of the
    // analysis without the stub are looked up by its own pieces
    // (ASH-ART Qfwfq panicked: index 143 of 143).
    let mut obstacles = vec![
        copper(
            Shape::Circle {
                center: [3.0, 5.0],
                radius: 0.3,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [7.0, 5.0],
                radius: 0.3,
            },
            0,
            "P.2",
        ),
        copper(
            Shape::Circle {
                center: [5.0, 8.0],
                radius: 0.3,
            },
            0,
            "P.3",
        ),
    ];
    for obstacle in &mut obstacles {
        obstacle.layers = 0b11;
    }
    let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal {
        anchor: at,
        layers: 0b11,
        pad,
        contact: None,
        label: label.into(),
    };
    let board = board(
        2,
        obstacles,
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![
                terminal(2, [5.0, 8.0], "P.3"),
                terminal(1, [7.0, 5.0], "P.2"),
                terminal(0, [3.0, 5.0], "P.1"),
            ],
        }],
        vec![plane(0, 0, Some((0.5, 0.5))), plane(0, 1, Some((0.5, 0.5)))],
        vec!["P.3".into()],
    );
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let grid = router.grid().clone();
    let node = |layer: u8, x: f64| Node {
        layer,
        cell: grid.nearest_node([x, 5.0]).unwrap() as u32,
    };
    {
        let state = &mut router.nets[0];
        assert!(state.on_plane[2]);
        state.on_plane[2] = false;
        state.branches = vec![Branch {
            nodes: vec![node(0, 3.0), node(0, 3.1), node(0, 3.2), node(1, 3.2)],
            start_terminal: 2,
            end_terminal: PLANE_TERMINAL,
        }];
    }
    router.stamp(0);
    let with_stub = router.analyze_pours(0).0.pieces;
    let without_stub = router.analyze_pours_skipping(0, &|_| true).0.pieces;
    assert_eq!(with_stub, without_stub + 2);
    router.refresh_pour(0);
    let state = &router.nets[0];
    assert!(state.on_plane[2]);
    assert!(state.branches.is_empty());
}

/// The spokes of `net`'s pads as the model sees them, with the
/// union-find and the terminal base of the analysis.
fn spoke_report(router: &Router, net: NetId) -> (Vec<SpokeRecord>, Vec<usize>, usize) {
    let mut report = Vec::new();
    let (pours, parent, _, _) = router.analyze_pours_reporting(net, &|_| false, Some(&mut report));
    (report, parent, pours.pieces + 1)
}

fn terminal(pad: usize, at: [f64; 2], label: &str) -> Terminal {
    Terminal {
        anchor: at,
        layers: 1,
        pad,
        contact: None,
        label: label.into(),
    }
}

#[test]
fn a_spoke_ends_at_the_gap_and_half_the_minimum_width() {
    // Right of P.1 a wall of Q leaves a channel of brush centres 0.15 mm
    // wide, from 0.425 to 0.575 mm off the pad: KiCad's spoke ends at
    // the gap and half the minimum width (0.43 mm) and lands there; the
    // old probe, at the gap and the spoke's width (0.7 mm), found no
    // fill. P.2 is solid: the fill has an item.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [0.5, 0.5]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [2.0, 8.0],
                radius: 0.4,
            },
            0,
            "P.2",
        ),
        copper(rect([8.15, 5.0], [1.75, 4.0]), 1, "Q.1"),
    ];
    let board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [5.0, 5.0], "P.1"),
                    terminal(1, [2.0, 8.0], "P.2"),
                ],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![terminal(2, [8.15, 5.0], "Q.1")],
            },
        ],
        vec![plane(0, 0, Some((0.3, 0.4)))],
        vec!["P.2".into()],
    );
    let outcome = |kicad_spokes: bool| {
        let router = Router::new(
            &board,
            &Config {
                pitches: vec![0.1],
                kicad_spokes,
                ..Config::default()
            },
        );
        let (report, _, _) = spoke_report(&router, 0);
        report
            .iter()
            .find(|record| record.terminal == 0)
            .unwrap()
            .outcomes[0]
    };
    assert!(matches!(outcome(true), SpokeOutcome::Good(piece) if piece != 0));
    assert_eq!(outcome(false), SpokeOutcome::NoFill);
}

#[test]
fn a_piece_that_two_pads_spoke_into_is_no_island() {
    // P.1 and P.2 (thermal) in a pour without any other copper of P:
    // one fill piece touches both, so KiCad's connectivity joins them
    // through it and its DRC counts their spokes. P.3, alone in a
    // piece walled off by Q, has spokes into a piece that only it
    // touches: an isolated island to the DRC.
    let obstacles = vec![
        copper(rect([2.0, 2.0], [0.4, 0.4]), 0, "P.1"),
        copper(rect([4.0, 2.0], [0.4, 0.4]), 0, "P.2"),
        copper(rect([8.0, 8.0], [0.4, 0.4]), 0, "P.3"),
        copper(rect([6.5, 8.0], [0.1, 3.0]), 1, "Q.1"),
        copper(rect([8.5, 5.5], [2.0, 0.1]), 1, "Q.2"),
    ];
    let board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [2.0, 2.0], "P.1"),
                    terminal(1, [4.0, 2.0], "P.2"),
                    terminal(2, [8.0, 8.0], "P.3"),
                ],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![
                    terminal(3, [6.5, 8.0], "Q.1"),
                    terminal(4, [8.5, 5.5], "Q.2"),
                ],
            },
        ],
        vec![plane(0, 0, Some((0.3, 0.4)))],
        Vec::new(),
    );
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (report, mut parent, base) = spoke_report(&router, 0);
    let outcomes = |terminal: usize| {
        report
            .iter()
            .find(|record| record.terminal == terminal)
            .unwrap()
            .outcomes
    };
    assert!(
        outcomes(0)
            .iter()
            .all(|outcome| matches!(outcome, SpokeOutcome::Good(_)))
    );
    assert!(
        outcomes(2)
            .iter()
            .all(|outcome| matches!(outcome, SpokeOutcome::Island(_)))
    );
    assert_eq!(find(&mut parent, base), find(&mut parent, base + 1));
    assert_ne!(find(&mut parent, base), find(&mut parent, base + 2));
    // The old model: a piece without a track, a via or a solid pad is
    // an island to every pad.
    let old = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            kicad_spokes: false,
            ..Config::default()
        },
    );
    let (report, _, _) = spoke_report(&old, 0);
    assert!(
        report
            .iter()
            .find(|record| record.terminal == 0)
            .unwrap()
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome, SpokeOutcome::Island(_)))
    );
}

/// P.1 at the middle of a pour, walled in by Q on three sides (+x, +y
/// and -y, right next to its spokes) so that only its -x spoke lands;
/// P.2, solid, gives the fill an item.
fn one_spoke_board() -> Board {
    let obstacles = vec![
        copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [2.0, 8.0],
                radius: 0.4,
            },
            0,
            "P.2",
        ),
        copper(rect([6.0, 5.0], [0.25, 0.25]), 1, "Q.1"),
        copper(rect([5.0, 6.0], [0.25, 0.25]), 1, "Q.2"),
        copper(rect([5.0, 4.0], [0.25, 0.25]), 1, "Q.3"),
    ];
    board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [5.0, 5.0], "P.1"),
                    terminal(1, [2.0, 8.0], "P.2"),
                ],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![
                    terminal(2, [6.0, 5.0], "Q.1"),
                    terminal(3, [5.0, 6.0], "Q.2"),
                    terminal(4, [5.0, 4.0], "Q.3"),
                ],
            },
        ],
        vec![plane(0, 0, Some((0.3, 0.4)))],
        vec!["P.2".into()],
    )
}

#[test]
fn a_track_from_the_pad_into_the_fill_counts_as_a_spoke() {
    // KiCad's DRC counts a track that starts within half a gap of the
    // pad and ends in fill as a spoke; a track that bends in the gap
    // does not count (its first segment ends outside the fill).
    let board = one_spoke_board();
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let grid = router.grid().clone();
    let node = |at: [f64; 2]| Node {
        layer: 0,
        cell: grid.nearest_node(at).unwrap() as u32,
    };
    let record = |router: &Router| {
        spoke_report(router, 0)
            .0
            .into_iter()
            .find(|record| record.terminal == 0)
            .unwrap()
    };
    let before = record(&router);
    assert_eq!(
        before
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, SpokeOutcome::Good(_)))
            .count(),
        1
    );
    assert_eq!(Router::spokes_short(&before), 1);
    // Diagonally out of the pad, past the corners of Q, into the fill.
    let straight: Vec<Node> = (0..=12)
        .map(|step| node([5.0 + 0.1 * step as f64, 5.0 + 0.1 * step as f64]))
        .collect();
    router.nets[0].branches = vec![Branch {
        nodes: straight,
        start_terminal: 0,
        end_terminal: PLANE_TERMINAL,
    }];
    let after = record(&router);
    assert_eq!(after.tracks, 1);
    assert_eq!(Router::spokes_short(&after), 0);
    // Bent inside the gap: the first run ends at (5.3, 5.3), in the
    // pad's relief.
    let mut bent: Vec<Node> = (0..=3)
        .map(|step| node([5.0 + 0.1 * step as f64, 5.0 + 0.1 * step as f64]))
        .collect();
    bent.extend((1..=9).map(|step| node([5.3 + 0.1 * step as f64, 5.3 + 0.1 * (step / 2) as f64])));
    router.nets[0].branches = vec![Branch {
        nodes: bent,
        start_terminal: 0,
        end_terminal: PLANE_TERMINAL,
    }];
    assert_eq!(record(&router).tracks, 0);
}

#[test]
fn each_zone_counts_the_spokes_in_its_own_part() {
    // P.1 at (5, 5) with four spokes into one fill, but two zones of P
    // meet just past its +y edge: a whole-board zone and, above it in
    // priority, a zone from y = 5.4 up. KiCad fills the lower zone
    // less the upper one's outline and checks the pad against each
    // zone apart: three spokes cross its ring in the lower zone, one
    // (+y, the ring at 5.45) in the upper zone, starved there
    // (Telemetry's USB shield pads against the edge zone of priority
    // 11). Equal priorities overlap: the lower zone then has all four,
    // the upper one still one. Manual spokes add a track that ends in
    // the upper zone's part.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [2.0, 8.0],
                radius: 0.4,
            },
            0,
            "P.2",
        ),
    ];
    let high: Vec<crate::geometry::Point> =
        vec![[0.0, 5.4], [10.0, 5.4], [10.0, 10.0], [0.0, 10.0]];
    let seamed = |above: bool| {
        let mut low = plane(0, 0, Some((0.3, 0.4)));
        if above {
            low.above = vec![high.clone()];
        }
        let mut upper = plane(0, 0, Some((0.3, 0.4)));
        upper.polygon = high.clone();
        board(
            1,
            obstacles.clone(),
            vec![Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [5.0, 5.0], "P.1"),
                    terminal(1, [2.0, 8.0], "P.2"),
                ],
            }],
            vec![low, upper],
            vec!["P.2".into()],
        )
    };
    let record = |router: &Router| {
        spoke_report(router, 0)
            .0
            .into_iter()
            .find(|record| record.terminal == 0)
            .unwrap()
    };
    for above in [true, false] {
        let board = seamed(above);
        let router = Router::new(
            &board,
            &Config {
                pitches: vec![0.1],
                ..Config::default()
            },
        );
        let before = record(&router);
        assert_eq!(
            before
                .outcomes
                .iter()
                .filter(|outcome| matches!(outcome, SpokeOutcome::Good(_)))
                .count(),
            4,
            "{before:?}"
        );
        assert_eq!(
            (
                before.seam_short,
                before.seam_zone,
                Router::spokes_short(&before)
            ),
            (1, Some(1), 1),
            "{before:?}"
        );
    }
    // Only the upper zone: the pad lies outside it but for its +y
    // spoke, which crosses the ring there.
    let mut router = Router::new(
        &seamed(true),
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    assert_eq!(router.add_manual_spokes(), 1);
    let after = record(&router);
    // The track (its stub to the pad's edge ends in the -x spoke, in
    // the lower zone; its diagonal run in the upper zone's fill).
    assert_eq!(
        (after.seam_short, Router::spokes_short(&after)),
        (0, 0),
        "{after:?}"
    );
    let track = router.nets[0].branches.last().unwrap();
    let end = router
        .grid()
        .center_of(track.nodes.last().unwrap().cell as usize);
    assert!(end[1] > 5.4, "{end:?}");
}

#[test]
fn manual_spokes_add_a_track_from_a_starved_pad_into_the_main_piece() {
    let board = one_spoke_board();
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    assert_eq!(router.add_manual_spokes(), 1);
    let state = &router.nets[0];
    assert_eq!(state.branches.len(), 1);
    let branch = &state.branches[0];
    assert_eq!(
        (branch.start_terminal, branch.end_terminal),
        (0, PLANE_TERMINAL)
    );
    assert!(router.branch_is_legal(0, branch));
    let record = spoke_report(&router, 0)
        .0
        .into_iter()
        .find(|record| record.terminal == 0)
        .unwrap();
    assert_eq!(Router::spokes_short(&record), 0);
    // Nothing more to add.
    assert_eq!(router.add_manual_spokes(), 0);
}

#[test]
fn manual_spokes_reach_pours_of_nets_routed_as_tracks() {
    // The tracks rung: P's pour does not connect pads (the net is routed
    // as tracks), but KiCad fills it and counts P.1's spokes all the
    // same; the router lends the net its pour masks for the analysis.
    let mut board = one_spoke_board();
    board.planes[0].connect = false;
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    assert!(router.nets[0].plane.is_empty());
    assert_eq!(router.add_manual_spokes(), 1);
    assert!(router.nets[0].plane.is_empty());
    let branch = &router.nets[0].branches[0];
    assert_eq!(
        (branch.start_terminal, branch.end_terminal),
        (0, PLANE_TERMINAL)
    );
    assert!(router.branch_is_legal(0, branch));
}

/// P.1, a through-hole pad (0.8 mm, drill 0.4 mm), in a box of Q walls
/// on layer 0: its spokes there land in a piece that nothing else of P
/// touches (KiCad's isolated island). P.2, solid, outside the box. With
/// `both_layers`, P's pour covers layer 1 too, where nothing cuts it.
fn island_board(both_layers: bool) -> Board {
    let mut obstacles = vec![
        copper(
            Shape::Circle {
                center: [5.0, 5.0],
                radius: 0.4,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [8.5, 8.5],
                radius: 0.4,
            },
            0,
            "P.2",
        ),
        copper(rect([3.5, 5.0], [0.1, 1.6]), 1, "Q.1"),
        copper(rect([6.5, 5.0], [0.1, 1.6]), 1, "Q.2"),
        copper(rect([5.0, 3.5], [1.6, 0.1]), 1, "Q.3"),
        copper(rect([5.0, 6.5], [1.6, 0.1]), 1, "Q.4"),
    ];
    obstacles[0].layers = 0b11;
    obstacles[1].layers = 0b11;
    obstacles.push(Obstacle {
        shape: Shape::Circle {
            center: [5.0, 5.0],
            radius: 0.2,
        },
        layers: 0b11,
        kind: ObstacleKind::Hole,
        net: Some(0),
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks: true,
        blocks_vias: true,
        label: "P.1 plated hole".into(),
    });
    let through = |pad: usize, at: [f64; 2], label: &str| Terminal {
        anchor: at,
        layers: 0b11,
        pad,
        contact: None,
        label: label.into(),
    };
    let mut planes = vec![plane(0, 0, Some((0.3, 0.4)))];
    if both_layers {
        planes.push(plane(0, 1, Some((0.3, 0.4))));
    }
    board(
        2,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![through(0, [5.0, 5.0], "P.1"), through(1, [8.5, 8.5], "P.2")],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![
                    terminal(2, [3.5, 5.0], "Q.1"),
                    terminal(3, [6.5, 5.0], "Q.2"),
                    terminal(4, [5.0, 3.5], "Q.3"),
                    terminal(5, [5.0, 6.5], "Q.4"),
                ],
            },
        ],
        planes,
        vec!["P.2".into()],
    )
}

/// P.1's track on layer 1: out of the pad along +x to (5.9, 5.0), over
/// the island's fill on layer 0 (KiCad's DRC counts it as a spoke of
/// layer 0: it starts in the pad and ends in that layer's fill), then
/// up and out of the box.
fn layer_one_track(router: &Router) -> Branch {
    let grid = router.grid();
    let mut nodes: Vec<Node> = (0..=9)
        .map(|step| Node {
            layer: 1,
            cell: grid.nearest_node([5.0 + 0.1 * step as f64, 5.0]).unwrap() as u32,
        })
        .collect();
    nodes.extend((1..=30).map(|step| Node {
        layer: 1,
        cell: grid.nearest_node([5.9, 5.0 + 0.1 * step as f64]).unwrap() as u32,
    }));
    Branch {
        nodes,
        start_terminal: 0,
        end_terminal: PLANE_TERMINAL,
    }
}

fn island_record(router: &Router) -> SpokeRecord {
    spoke_report(router, 0)
        .0
        .into_iter()
        .find(|record| record.terminal == 0 && record.layer == 0)
        .unwrap()
}

#[test]
fn a_track_leaving_a_through_hole_pad_between_its_spokes_does_not_join_their_island() {
    // P.1 leaves on layer 0 by a via at its centre: its track there is
    // no longer than the pad's hole. KiCad's fill reaches a plated pad
    // only up to its drill, so the track touches no spoke and the
    // island stays one (mackerel-30's C50.2 on In1).
    let board = island_board(false);
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let centre = router.grid().nearest_node([5.0, 5.0]).unwrap() as u32;
    router.nets[0].branches = vec![Branch {
        nodes: vec![
            Node {
                layer: 0,
                cell: centre,
            },
            Node {
                layer: 1,
                cell: centre,
            },
        ],
        start_terminal: 0,
        end_terminal: PLANE_TERMINAL,
    }];
    let outcomes = island_record(&router).outcomes;
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, SpokeOutcome::Island(_))),
        "{outcomes:?}"
    );
}

#[test]
fn a_pad_in_an_island_gets_a_track_into_it_with_island_stubs() {
    // Layer 0's island holds only P.1, and its track on layer 1 counts
    // as a spoke, so the island's spokes do not: a starved thermal. No
    // straight track reaches kept fill and no other layer has a pour to
    // stitch to: P.1 gets a track on layer 0 into its island, the
    // island holds two items and its spokes count.
    let board = island_board(false);
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            island_stubs: true,
            ..Config::default()
        },
    );
    router.nets[0].branches = vec![layer_one_track(&router)];
    router.stamp(0);
    let before = island_record(&router);
    assert_eq!(before.tracks, 1);
    assert!(Router::spokes_short(&before) > 0, "{before:?}");
    assert_eq!(router.add_manual_spokes(), 1);
    let added = router.nets[0].branches.last().unwrap().clone();
    assert_eq!(
        (added.start_terminal, added.end_terminal),
        (0, PLANE_TERMINAL)
    );
    assert!(added.nodes.iter().all(|node| node.layer == 0));
    assert!(router.branch_is_legal(0, &added));
    assert_eq!(Router::spokes_short(&island_record(&router)), 0);
    // The result reports it (the verdict's `repairs.island_stubs`).
    assert_eq!(router.snapshot().island_stubs, 1);
    // Without island stubs (the default) nothing helps.
    assert!(!Config::default().island_stubs);
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    router.nets[0].branches = vec![layer_one_track(&router)];
    router.stamp(0);
    assert_eq!(router.add_manual_spokes(), 0);
    assert_eq!(router.snapshot().island_stubs, 0);
}

#[test]
fn a_pad_in_an_island_gets_a_via_to_the_main_piece_on_another_layer() {
    // As above, with P's pour on layer 1 too: a via from the island to
    // layer 1's fill (the main piece, which P.2 holds) joins the island
    // to the plane, and its spokes count.
    let board = island_board(true);
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    router.nets[0].branches = vec![layer_one_track(&router)];
    router.stamp(0);
    assert!(Router::spokes_short(&island_record(&router)) > 0);
    assert!(router.add_manual_spokes() >= 1);
    let via = router.nets[0].branches[1].clone();
    assert_eq!(via.nodes.len(), 2);
    assert_eq!(via.nodes[0].cell, via.nodes[1].cell);
    assert_ne!(via.nodes[0].layer, via.nodes[1].layer);
    let at = router.grid().center_of(via.nodes[0].cell as usize);
    assert!(
        (3.6..6.4).contains(&at[0]) && (3.6..6.4).contains(&at[1]),
        "{at:?}"
    );
    assert_eq!(Router::spokes_short(&island_record(&router)), 0);
}

#[test]
fn spokes_of_two_pads_side_by_side_meet() {
    // P.1 and P.2 0.5 mm apart: no brush fits between them (it keeps
    // 0.425 mm from each), but each spoke's end lies in the other's
    // spoke, and KiCad keeps both: one outline holds the two pads.
    let obstacles = vec![
        copper(rect([4.0, 5.0], [0.4, 0.4]), 0, "P.1"),
        copper(rect([4.9 + 0.4, 5.0], [0.4, 0.4]), 0, "P.2"),
    ];
    let board = board(
        1,
        obstacles,
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![
                terminal(0, [4.0, 5.0], "P.1"),
                terminal(1, [5.3, 5.0], "P.2"),
            ],
        }],
        vec![plane(0, 0, Some((0.3, 0.4)))],
        Vec::new(),
    );
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (report, mut parent, base) = spoke_report(&router, 0);
    assert_eq!(
        report
            .iter()
            .find(|record| record.terminal == 0)
            .unwrap()
            .outcomes[0],
        SpokeOutcome::Pad(1)
    );
    assert_eq!(
        report
            .iter()
            .find(|record| record.terminal == 1)
            .unwrap()
            .outcomes[1],
        SpokeOutcome::Pad(0)
    );
    assert_eq!(find(&mut parent, base), find(&mut parent, base + 1));
}

#[test]
fn no_spoke_is_narrower_than_the_minimum_width() {
    // A pad 0.2 mm wide: its spokes would be 0.2 mm wide (no wider than
    // the pad), less than the zone's 0.25 mm minimum: KiCad draws none.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [0.5, 0.1]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [2.0, 8.0],
                radius: 0.4,
            },
            0,
            "P.2",
        ),
    ];
    let board = board(
        1,
        obstacles,
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![
                terminal(0, [5.0, 5.0], "P.1"),
                terminal(1, [2.0, 8.0], "P.2"),
            ],
        }],
        vec![plane(0, 0, Some((0.3, 0.4)))],
        vec!["P.2".into()],
    );
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (report, _, _) = spoke_report(&router, 0);
    assert_eq!(
        report
            .iter()
            .find(|record| record.terminal == 0)
            .unwrap()
            .outcomes,
        [SpokeOutcome::NoFill; 4]
    );
}

#[test]
fn the_fill_probe_finds_brush_centres_between_the_nodes() {
    // Two walls of Q 0.7 mm apart leave brush centres only for x in
    // [5.025, 5.075] (0.325 mm from each): no node of the 0.1 mm
    // lattice lies there, the probe finds the fill all the same.
    let obstacles = vec![
        copper(rect([3.7, 5.0], [1.0, 4.0]), 1, "Q.1"),
        copper(rect([6.4, 5.0], [1.0, 4.0]), 1, "Q.2"),
        copper(
            Shape::Circle {
                center: [5.0, 9.5],
                radius: 0.2,
            },
            0,
            "P.1",
        ),
    ];
    let board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![terminal(2, [5.0, 9.5], "P.1")],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [3.7, 5.0], "Q.1"),
                    terminal(1, [6.4, 5.0], "Q.2"),
                ],
            },
        ],
        vec![plane(0, 0, Some((0.3, 0.4)))],
        Vec::new(),
    );
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let grid = router.grid().clone();
    let state = &router.nets[0];
    assert!(!state.plane[0][grid.nearest_node([5.0, 5.0]).unwrap()]);
    assert!(!state.plane[0][grid.nearest_node([5.1, 5.0]).unwrap()]);
    let rings = vec![Vec::new()];
    let probe = fill::FillProbe::new(&router, 0, &rings);
    assert!(probe.fits(0, [5.05, 5.0], &[]));
    assert!(!probe.fits(0, [5.0, 5.0], &[]));
    assert!(probe.fill_at(0, [5.0, 5.0]));
    assert!(!probe.fill_at(0, [4.85, 5.0]));
}

#[test]
fn a_pads_own_clearance_lets_the_fill_closer() {
    // Q.1 sits 0.7 mm right of P.1's edge. With the zone's 0.2 mm
    // clearance no brush centre fits between P.1's relief (0.425 mm
    // out) and Q.1 (0.325 mm off): P.1's +x spoke finds no fill. Q.1's
    // own 0.05 mm override replaces the zone's clearance in KiCad's
    // fill (Hub's USB-C pins): the spoke lands, on the lattice too.
    let outcome = |clearance_override: Option<f64>| {
        let mut obstacles = vec![
            copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
            copper(
                Shape::Circle {
                    center: [2.0, 8.0],
                    radius: 0.4,
                },
                0,
                "P.2",
            ),
            copper(rect([6.15, 5.0], [0.15, 0.15]), 1, "Q.1"),
        ];
        obstacles[2].clearance_override = clearance_override;
        let board = board(
            1,
            obstacles,
            vec![
                Net {
                    name: "P".into(),
                    class: 0,
                    terminals: vec![
                        terminal(0, [5.0, 5.0], "P.1"),
                        terminal(1, [2.0, 8.0], "P.2"),
                    ],
                },
                Net {
                    name: "Q".into(),
                    class: 0,
                    terminals: vec![terminal(2, [6.15, 5.0], "Q.1")],
                },
            ],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            vec!["P.2".into()],
        );
        let router = Router::new(
            &board,
            &Config {
                pitches: vec![0.1],
                ..Config::default()
            },
        );
        let (report, _, _) = spoke_report(&router, 0);
        report
            .iter()
            .find(|record| record.terminal == 0)
            .unwrap()
            .outcomes[0]
    };
    assert!(!matches!(outcome(None), SpokeOutcome::Good(_)));
    assert!(matches!(outcome(Some(0.05)), SpokeOutcome::Good(piece) if piece != 0));
}

#[test]
fn spokes_meet_whatever_their_directions() {
    // Two round pads 1.35 mm apart (a USB-C receptacle's A1 and B12):
    // their diagonal spokes cross at right angles, each end inside the
    // other's spoke, in the other pad's relief and short of room beside
    // Q.1. KiCad keeps both.
    let obstacles = vec![
        copper(
            Shape::Circle {
                center: [5.0, 5.0],
                radius: 0.35,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [5.0, 3.65],
                radius: 0.35,
            },
            0,
            "P.2",
        ),
        copper(
            Shape::Circle {
                center: [6.2, 4.325],
                radius: 0.15,
            },
            1,
            "Q.1",
        ),
    ];
    let board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [5.0, 5.0], "P.1"),
                    terminal(1, [5.0, 3.65], "P.2"),
                ],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![terminal(2, [6.2, 4.325], "Q.1")],
            },
        ],
        vec![plane(0, 0, Some((0.5, 0.5)))],
        Vec::new(),
    );
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (report, mut parent, base) = spoke_report(&router, 0);
    // P.1's +x-y spoke (index 1) and P.2's +x+y (index 0).
    assert_eq!(
        report
            .iter()
            .find(|record| record.terminal == 0)
            .unwrap()
            .outcomes[1],
        SpokeOutcome::Pad(1)
    );
    assert_eq!(
        report
            .iter()
            .find(|record| record.terminal == 1)
            .unwrap()
            .outcomes[0],
        SpokeOutcome::Pad(0)
    );
    assert_eq!(find(&mut parent, base), find(&mut parent, base + 1));
}

#[test]
fn a_turned_pads_spokes_turn_with_it() {
    let close =
        |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1.0e-9 && (a[1] - b[1]).abs() < 1.0e-9;
    let d = std::f64::consts::FRAC_1_SQRT_2;
    // A 1.0 x 0.4 pad at 45 degrees: spokes along its own axes, its
    // local x turned as KiCad's RotatePoint turns it.
    let frame = crate::board::PadSpokes {
        center: [0.0, 0.0],
        half: [0.5, 0.2],
        orientation: 45.0,
        spoke_angle: 90.0,
        circular: false,
    };
    let spokes = frame.spokes(0.6);
    let along_x = spokes.iter().find(|spoke| close(spoke.0, [d, -d])).unwrap();
    assert!((along_x.1 - 0.5).abs() < 1.0e-9 && (along_x.2 - 1.1).abs() < 1.0e-9);
    let along_y = spokes.iter().find(|spoke| close(spoke.0, [d, d])).unwrap();
    assert!((along_y.1 - 0.2).abs() < 1.0e-9 && (along_y.2 - 0.8).abs() < 1.0e-9);
    // The same pad unturned with a 45 degree spoke angle: diagonal
    // spokes to its box grown by the reach.
    let frame = crate::board::PadSpokes {
        orientation: 0.0,
        spoke_angle: 45.0,
        ..frame
    };
    let spokes = frame.spokes(0.6);
    assert!(close(spokes[0].0, [d, d]));
    assert!((spokes[0].1 - 0.2 / d).abs() < 1.0e-9 && (spokes[0].2 - 0.8 / d).abs() < 1.0e-9);
}

#[test]
fn a_pad_merged_into_its_terminal_is_checked_on_its_own_layers() {
    // P.1 twice: a pad on the back (the terminal) and a through-hole
    // pad at its spot, merged into it. On the front the through-hole
    // pad is copper of its own with a thermal relief: it has spokes;
    // on the back it lies inside the larger pad.
    let mut obstacles = vec![
        copper(rect([5.0, 5.0], [0.8, 0.4]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [5.0, 5.0],
                radius: 0.4,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [2.0, 8.0],
                radius: 0.4,
            },
            0,
            "P.2",
        ),
    ];
    obstacles[0].layers = 0b10;
    obstacles[1].layers = 0b11;
    obstacles[2].layers = 0b11;
    let mut board = board(
        2,
        obstacles,
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![
                Terminal {
                    anchor: [5.0, 5.0],
                    layers: 0b10,
                    pad: 0,
                    contact: None,
                    label: "P.1".into(),
                },
                Terminal {
                    anchor: [2.0, 8.0],
                    layers: 0b11,
                    pad: 2,
                    contact: None,
                    label: "P.2".into(),
                },
            ],
        }],
        vec![plane(0, 0, Some((0.3, 0.4))), plane(0, 1, Some((0.3, 0.4)))],
        vec!["P.2".into()],
    );
    board.merged_pads.push(crate::board::MergedPad {
        net: 0,
        host: 0,
        pad: 1,
        anchor: [5.0, 5.0],
        layers: 0b11,
        padstack: false,
    });
    // Inside the larger pad too.
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (report, _, _) = spoke_report(&router, 0);
    let merged: Vec<usize> = report
        .iter()
        .filter(|record| record.pad == 1)
        .map(|record| record.layer)
        .collect();
    assert_eq!(merged, vec![0, 1]);
    assert!(
        report
            .iter()
            .find(|record| record.pad == 1)
            .unwrap()
            .outcomes
            .iter()
            .any(|outcome| matches!(outcome, SpokeOutcome::Good(_)))
    );
}

#[test]
fn thermal_vias_inside_an_exposed_pad_have_their_own_rings_in_a_hatched_zone() {
    // P.1, an exposed pad 2 mm square, with two thermal vias of its
    // number merged into it: one at its centre, one in its corner by
    // the walls of Q. In a hatched zone KiCad gives every thermal pad a
    // ring a spoke wide, inside the exposed pad too, and keeps a via's
    // spokes where they reach it (0xCB's U3.45): the centre via has
    // four; the corner via's ring and spokes outside the pad fall to
    // Q's clearance, its inward spoke alone stays, and the DRC finds
    // it starved. The outline holds both pads: no island.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [1.0, 1.0]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [5.0, 5.0],
                radius: 0.25,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [5.75, 5.75],
                radius: 0.25,
            },
            0,
            "P.1",
        ),
        copper(rect([6.4, 5.5], [0.2, 1.5]), 1, "Q.1"),
        copper(rect([5.5, 6.4], [1.5, 0.2]), 1, "Q.2"),
    ];
    let mut planes = vec![plane(0, 0, Some((0.15, 0.3)))];
    planes[0].hatch = Some(crate::board::Hatch {
        thickness: 0.25,
        gap: 1.0,
        orientation: 0.0,
        smoothing_level: 0,
        smoothing_value: 0.0,
        min_hole_area: 0.3,
        via_reliefs: true,
    });
    let mut board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![terminal(0, [5.0, 5.0], "P.1")],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![
                    terminal(3, [6.4, 5.5], "Q.1"),
                    terminal(4, [5.5, 6.4], "Q.2"),
                ],
            },
        ],
        planes,
        Vec::new(),
    );
    for (pad, anchor) in [(1, [5.0, 5.0]), (2, [5.75, 5.75])] {
        board.merged_pads.push(crate::board::MergedPad {
            net: 0,
            host: 0,
            pad,
            anchor,
            layers: 1,
            padstack: false,
        });
    }
    let good = |record: &SpokeRecord| {
        record
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, SpokeOutcome::Good(_) | SpokeOutcome::Pad(_)))
            .count()
            + record.crossings
    };
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (report, _, _) = spoke_report(&router, 0);
    let centre = report.iter().find(|record| record.pad == 1).unwrap();
    assert_eq!(good(centre), 4, "{:?}", centre.outcomes);
    let corner = report.iter().find(|record| record.pad == 2).unwrap();
    assert_eq!(good(corner), 1, "{:?}", corner.outcomes);
    assert!(
        corner
            .outcomes
            .iter()
            .all(|outcome| !matches!(outcome, SpokeOutcome::Island(_)))
    );
}

#[test]
fn a_fill_probe_counts_its_queries_and_polygon_vertices() {
    // The board of `thermal_vias_inside_an_exposed_pad_have_their_own_
    // rings_in_a_hatched_zone`: a hatched pour around P.1, a 2 mm
    // square (a polygon of 4 vertices) with two round thermal vias.
    // A query costs the same whatever its items, and a polygon's test
    // goes through its vertices (Batch_1's board cutouts of up to 134
    // vertices, in every query near them): a probe charges both beside
    // its items, and the analysis's work counts them.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [1.0, 1.0]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [5.0, 5.0],
                radius: 0.25,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [5.75, 5.75],
                radius: 0.25,
            },
            0,
            "P.1",
        ),
        copper(rect([6.4, 5.5], [0.2, 1.5]), 1, "Q.1"),
        copper(rect([5.5, 6.4], [1.5, 0.2]), 1, "Q.2"),
    ];
    let mut planes = vec![plane(0, 0, Some((0.15, 0.3)))];
    planes[0].hatch = Some(crate::board::Hatch {
        thickness: 0.25,
        gap: 1.0,
        orientation: 0.0,
        smoothing_level: 0,
        smoothing_value: 0.0,
        min_hole_area: 0.3,
        via_reliefs: true,
    });
    let mut board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![terminal(0, [5.0, 5.0], "P.1")],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![
                    terminal(3, [6.4, 5.5], "Q.1"),
                    terminal(4, [5.5, 6.4], "Q.2"),
                ],
            },
        ],
        planes,
        Vec::new(),
    );
    for (pad, anchor) in [(1, [5.0, 5.0]), (2, [5.75, 5.75])] {
        board.merged_pads.push(crate::board::MergedPad {
            net: 0,
            host: 0,
            pad,
            anchor,
            layers: 1,
            padstack: false,
        });
    }
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let rings = router.thermal_ring_pads(0);
    let probe = fill::FillProbe::new(&router, 0, &rings);
    // One query beside Q.1 (a polygon of 4 vertices within reach).
    probe.fits(0, [7.0, 5.5], &[]);
    probe.charge();
    let load =
        |counter: &std::sync::atomic::AtomicU64| counter.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(load(&router.scratch.probe_queries), 1);
    let vertices = load(&router.scratch.probe_vertices);
    assert!(vertices >= 4 && vertices % 4 == 0, "{vertices}");
    let items = load(&router.scratch.probe_ops);
    assert!(items > 0);
    assert_eq!(
        router.probe_nanos(),
        items * PROBE_ITEM_NANOS + PROBE_QUERY_NANOS + vertices * PROBE_VERTEX_NANOS
    );
    // A point in P.1's ring tests the pad's polygon too.
    let before = router.probe_nanos();
    probe.in_ring(0, [6.2, 5.0]);
    probe.charge();
    assert!(load(&router.scratch.probe_vertices) > vertices);
    assert!(router.probe_nanos() > before);
    // The analysis charges its probe's queries and vertices as work.
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    router.analyze_pours(0);
    assert!(load(&router.scratch.probe_queries) > 0);
    assert!(load(&router.scratch.probe_vertices) > 0);
    let own = router.pour_analysis_nanos(0);
    assert!(
        (router.pour_work_seconds() - (own + router.probe_nanos()) as f64 / 1.0e9).abs() < 1.0e-12
    );
    assert!(router.probe_nanos() > load(&router.scratch.probe_ops) * PROBE_ITEM_NANOS);
}

#[test]
fn the_tracks_rung_reads_a_hatched_zone_as_hatched() {
    // The board of `thermal_vias_inside_an_exposed_pad_have_their_own_
    // rings_in_a_hatched_zone` in the tracks rung: P's pour does not
    // connect (the net is routed as tracks), but KiCad fills it hatched
    // all the same, the vias' rings in it. With the masks lent as
    // `add_manual_spokes` lends them the model reads the hatch there
    // too: the corner via keeps its one spoke into its ring (starved),
    // as with the pour connected.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [1.0, 1.0]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [5.0, 5.0],
                radius: 0.25,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [5.75, 5.75],
                radius: 0.25,
            },
            0,
            "P.1",
        ),
        copper(rect([6.4, 5.5], [0.2, 1.5]), 1, "Q.1"),
        copper(rect([5.5, 6.4], [1.5, 0.2]), 1, "Q.2"),
    ];
    let mut planes = vec![plane(0, 0, Some((0.15, 0.3)))];
    planes[0].hatch = Some(crate::board::Hatch {
        thickness: 0.25,
        gap: 1.0,
        orientation: 0.0,
        smoothing_level: 0,
        smoothing_value: 0.0,
        min_hole_area: 0.3,
        via_reliefs: true,
    });
    planes[0].connect = false;
    let mut board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![terminal(0, [5.0, 5.0], "P.1")],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![
                    terminal(3, [6.4, 5.5], "Q.1"),
                    terminal(4, [5.5, 6.4], "Q.2"),
                ],
            },
        ],
        planes,
        Vec::new(),
    );
    for (pad, anchor) in [(1, [5.0, 5.0]), (2, [5.75, 5.75])] {
        board.merged_pads.push(crate::board::MergedPad {
            net: 0,
            host: 0,
            pad,
            anchor,
            layers: 1,
            padstack: false,
        });
    }
    let good = |record: &SpokeRecord| {
        record
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, SpokeOutcome::Good(_) | SpokeOutcome::Pad(_)))
            .count()
            + record.crossings
    };
    let lent = |config: Config| {
        let mut router = Router::new(&board, &config);
        assert!(router.nets[0].plane.is_empty());
        let (plane, plane_class) = router.pour_masks(0, false);
        router.nets[0].plane = plane;
        router.nets[0].plane_class = plane_class;
        router
    };
    let router = lent(Config {
        pitches: vec![0.1],
        ..Config::default()
    });
    let probe_rings = router.thermal_ring_pads(0);
    let probe = fill::FillProbe::new(&router, 0, &probe_rings);
    assert!(probe.hatch(0).is_some());
    let (report, _, _) = spoke_report(&router, 0);
    let centre = report.iter().find(|record| record.pad == 1).unwrap();
    assert_eq!(good(centre), 4, "{:?}", centre.outcomes);
    let corner = report.iter().find(|record| record.pad == 2).unwrap();
    assert_eq!(
        (good(corner), Router::spokes_short(corner)),
        (1, 1),
        "{:?}",
        corner.outcomes
    );
}

#[test]
fn a_track_ends_for_the_drc_at_a_junction() {
    // P.1's track runs straight out of the pad into the fill and on
    // into Q.4, where no fill is; another branch of P leaves it at
    // (6.2, 6.2). The router emits the run as two segments, cut at the
    // junction, and KiCad's DRC tests the first one's end, in fill: a
    // spoke (the run's end, in Q.4, would count nothing).
    let mut board = one_spoke_board();
    board
        .obstacles
        .push(copper(rect([7.5, 7.5], [0.3, 0.3]), 1, "Q.4"));
    let record = |config: Config| {
        let mut router = Router::new(&board, &config);
        let grid = router.grid().clone();
        let node = |at: [f64; 2]| Node {
            layer: 0,
            cell: grid.nearest_node(at).unwrap() as u32,
        };
        let straight: Vec<Node> = (0..=25)
            .map(|step| node([5.0 + 0.1 * step as f64, 5.0 + 0.1 * step as f64]))
            .collect();
        let side: Vec<Node> = (0..=10)
            .map(|step| node([6.2, 6.2 + 0.1 * step as f64]))
            .collect();
        router.nets[0].branches = vec![
            Branch {
                nodes: straight,
                start_terminal: 0,
                end_terminal: PLANE_TERMINAL,
            },
            Branch {
                nodes: side,
                start_terminal: NO_TERMINAL,
                end_terminal: PLANE_TERMINAL,
            },
        ];
        spoke_report(&router, 0)
            .0
            .into_iter()
            .find(|record| record.terminal == 0)
            .unwrap()
    };
    let cut = record(Config {
        pitches: vec![0.1],
        ..Config::default()
    });
    assert_eq!((cut.tracks, Router::spokes_short(&cut)), (1, 0));
}

#[test]
fn a_starved_thermal_via_inside_an_exposed_pad_gets_a_track_into_its_ring() {
    // The corner via of `thermal_vias_inside_an_exposed_pad_have_their_
    // own_rings_in_a_hatched_zone`, starved with one spoke: a track from
    // its centre into its ring over the exposed pad (which KiCad's DRC
    // counts as a spoke) makes two, its copper inside the exposed pad's.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [1.0, 1.0]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [5.0, 5.0],
                radius: 0.25,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [5.75, 5.75],
                radius: 0.25,
            },
            0,
            "P.1",
        ),
        copper(rect([6.4, 5.5], [0.2, 1.5]), 1, "Q.1"),
        copper(rect([5.5, 6.4], [1.5, 0.2]), 1, "Q.2"),
    ];
    let mut planes = vec![plane(0, 0, Some((0.15, 0.3)))];
    planes[0].hatch = Some(crate::board::Hatch {
        thickness: 0.25,
        gap: 1.0,
        orientation: 0.0,
        smoothing_level: 0,
        smoothing_value: 0.0,
        min_hole_area: 0.3,
        via_reliefs: true,
    });
    let mut board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![terminal(0, [5.0, 5.0], "P.1")],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![
                    terminal(3, [6.4, 5.5], "Q.1"),
                    terminal(4, [5.5, 6.4], "Q.2"),
                ],
            },
        ],
        planes,
        Vec::new(),
    );
    for (pad, anchor) in [(1, [5.0, 5.0]), (2, [5.75, 5.75])] {
        board.merged_pads.push(crate::board::MergedPad {
            net: 0,
            host: 0,
            pad,
            anchor,
            layers: 1,
            padstack: false,
        });
    }
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let corner = |router: &Router| {
        spoke_report(router, 0)
            .0
            .into_iter()
            .find(|record| record.pad == 2)
            .unwrap()
    };
    assert_eq!(Router::spokes_short(&corner(&router)), 1);
    let clock = router.clock();
    assert_eq!(router.add_inner_spokes(0, None, &clock, f64::INFINITY), 1);
    let after = corner(&router);
    assert_eq!((Router::spokes_short(&after), after.tracks), (0, 1));
    let (layer, start, end, width) = router.nets[0].inner_spokes[0];
    assert_eq!((layer, start), (0, [5.75, 5.75]));
    assert!(board.obstacles[0].shape.contains(end));
    assert!((width - 0.25).abs() < 1.0e-9);
    assert!(
        router
            .materialize(0)
            .0
            .segments
            .iter()
            .any(|segment| segment.start == start && segment.end == end)
    );
    // Nothing more to add.
    let clock = router.clock();
    assert_eq!(router.add_inner_spokes(0, None, &clock, f64::INFINITY), 0);
}

#[test]
fn the_exposed_pads_spoke_through_a_thermal_vias_ring_counts_twice() {
    // In a solid zone the exposed pad's relief takes its thermal vias'
    // spokes; the DRC counts what crosses a via's ring: the pad's own
    // spoke along +x runs from its centre right through via A's ring,
    // crossing it on both sides, two spokes (mackerel's mounting holes
    // on In1). Via B, off the spokes, crosses no fill: not connected,
    // no finding either.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [1.0, 1.0]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [5.6, 5.0],
                radius: 0.2,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [4.4, 4.4],
                radius: 0.2,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [8.0, 5.0],
                radius: 0.4,
            },
            0,
            "P.2",
        ),
    ];
    // P.2, solid, is the fill's second item.
    let mut board = board(
        1,
        obstacles,
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![
                terminal(0, [5.0, 5.0], "P.1"),
                terminal(3, [8.0, 5.0], "P.2"),
            ],
        }],
        vec![plane(0, 0, Some((0.3, 0.4)))],
        vec!["P.2".into()],
    );
    for (pad, anchor) in [(1, [5.6, 5.0]), (2, [4.4, 4.4])] {
        board.merged_pads.push(crate::board::MergedPad {
            net: 0,
            host: 0,
            pad,
            anchor,
            layers: 1,
            padstack: false,
        });
    }
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (report, _, _) = spoke_report(&router, 0);
    let on_spoke = report.iter().find(|record| record.pad == 1).unwrap();
    assert!(
        on_spoke
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome, SpokeOutcome::NoFill)),
        "{:?}",
        on_spoke.outcomes
    );
    assert_eq!(on_spoke.crossings, 2);
    let off_spoke = report.iter().find(|record| record.pad == 2).unwrap();
    assert!(
        off_spoke
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome, SpokeOutcome::NoFill)),
        "{:?}",
        off_spoke.outcomes
    );
    assert_eq!(off_spoke.crossings, 0);
}

#[test]
fn a_spoke_cut_off_at_the_pad_brings_no_item() {
    // P.1, 0.3 x 0.8 mm, with Q.1 and Q.2 beside the end of its -y
    // spoke, whose clearances cut the spoke where it leaves the pad: no
    // brush centre within half the minimum width of the pad keeps its
    // distance from them; past them the fill is free. KiCad's spoke still
    // crosses the ring there, but its piece does not hold P.1: with
    // only P.2 in it, the crossing is an island's. Without Q the spoke
    // joins P.1 to the piece: two items, a counted spoke.
    let outcome = |walls: bool| {
        let mut obstacles = vec![
            copper(rect([5.0, 5.0], [0.15, 0.4]), 0, "P.1"),
            copper(
                Shape::Circle {
                    center: [5.0, 1.5],
                    radius: 0.4,
                },
                0,
                "P.2",
            ),
            // The other spokes end in R's copper.
            copper(rect([5.8, 5.0], [0.15, 0.3]), 2, "R.1"),
            copper(rect([4.2, 5.0], [0.15, 0.3]), 2, "R.2"),
            copper(rect([5.0, 6.1], [0.3, 0.15]), 2, "R.3"),
        ];
        let mut terminals_q = Vec::new();
        if walls {
            obstacles.push(copper(
                Shape::Circle {
                    center: [4.75, 4.55],
                    radius: 0.05,
                },
                1,
                "Q.1",
            ));
            obstacles.push(copper(
                Shape::Circle {
                    center: [5.25, 4.55],
                    radius: 0.05,
                },
                1,
                "Q.2",
            ));
            terminals_q = vec![
                terminal(5, [4.75, 4.55], "Q.1"),
                terminal(6, [5.25, 4.55], "Q.2"),
            ];
        }
        let board = board(
            1,
            obstacles,
            vec![
                Net {
                    name: "P".into(),
                    class: 0,
                    terminals: vec![
                        terminal(0, [5.0, 5.0], "P.1"),
                        terminal(1, [5.0, 1.5], "P.2"),
                    ],
                },
                Net {
                    name: "Q".into(),
                    class: 0,
                    terminals: terminals_q,
                },
                Net {
                    name: "R".into(),
                    class: 0,
                    terminals: vec![
                        terminal(2, [5.8, 5.0], "R.1"),
                        terminal(3, [4.2, 5.0], "R.2"),
                        terminal(4, [5.0, 6.1], "R.3"),
                    ],
                },
            ],
            vec![plane(0, 0, Some((0.5, 0.4)))],
            vec!["P.2".into()],
        );
        let router = Router::new(
            &board,
            &Config {
                pitches: vec![0.1],
                ..Config::default()
            },
        );
        let (report, _, _) = spoke_report(&router, 0);
        report
            .iter()
            .find(|record| record.terminal == 0)
            .unwrap()
            .outcomes[3]
    };
    assert!(
        matches!(outcome(true), SpokeOutcome::Island(_)),
        "{:?}",
        outcome(true)
    );
    assert!(matches!(outcome(false), SpokeOutcome::Good(_)));
}

#[test]
fn spokes_of_other_pads_across_the_ring_count() {
    // P.1 and P.2 meet with diagonal spokes beside P.3, whose ring
    // half a gap out the two spokes cross: KiCad's DRC counts both
    // crossings as P.3's spokes.
    let obstacles = vec![
        copper(
            Shape::Circle {
                center: [5.0, 5.0],
                radius: 0.35,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [5.0, 6.35],
                radius: 0.35,
            },
            0,
            "P.2",
        ),
        copper(rect([6.0, 5.675], [0.2, 0.2]), 0, "P.3"),
    ];
    let board = board(
        1,
        obstacles,
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![
                terminal(0, [5.0, 5.0], "P.1"),
                terminal(1, [5.0, 6.35], "P.2"),
                terminal(2, [6.0, 5.675], "P.3"),
            ],
        }],
        vec![plane(0, 0, Some((0.5, 0.5)))],
        Vec::new(),
    );
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (report, _, _) = spoke_report(&router, 0);
    let crossings = report
        .iter()
        .find(|record| record.terminal == 2)
        .unwrap()
        .crossings;
    assert_eq!(crossings, 2);
}

#[test]
fn a_hatched_zones_holes_lie_on_its_grid() {
    // A hatched pour (web 0.5, holes 1.0 mm, at 0 degrees): the holes
    // sit on a grid of 1.5 mm from the origin, a hole (grown back by
    // half the minimum width) from 0.125 to 1.125 mm; a web's width
    // along the fill's edge stays filled.
    let mut planes = vec![plane(0, 0, Some((0.3, 0.4)))];
    planes[0].hatch = Some(crate::board::Hatch {
        thickness: 0.5,
        gap: 1.0,
        orientation: 0.0,
        smoothing_level: 0,
        smoothing_value: 0.0,
        min_hole_area: 0.3,
        via_reliefs: true,
    });
    let board = board(
        1,
        vec![copper(
            Shape::Circle {
                center: [8.0, 8.0],
                radius: 0.4,
            },
            0,
            "P.1",
        )],
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![terminal(0, [8.0, 8.0], "P.1")],
        }],
        planes,
        vec!["P.1".into()],
    );
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let rings = vec![Vec::new()];
    let probe = fill::FillProbe::new(&router, 0, &rings);
    assert!(probe.in_hatch_hole(0, [3.6, 3.6]));
    assert!(!probe.in_hatch_hole(0, [4.4, 3.6]));
    assert!(!probe.final_fill_at(0, [3.6, 3.6]));
    assert!(probe.final_fill_at(0, [4.4, 3.6]));
    // By the board's edge: the web along it.
    assert!(!probe.in_hatch_hole(0, [0.4, 3.6]));
    assert!((probe.spoke_half_width(0) - 0.25).abs() < 1.0e-9);
}

#[test]
fn a_track_to_a_pad_at_the_same_spot_counts_for_both() {
    // P.1 and P.2 at one spot (a USB-C receptacle's A1 and B12): the
    // DRC counts the track that leaves them, ending in fill, for both,
    // though the router's branch names one.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
        copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.2"),
        copper(
            Shape::Circle {
                center: [2.0, 8.0],
                radius: 0.4,
            },
            0,
            "P.3",
        ),
        copper(rect([6.0, 5.0], [0.25, 0.25]), 1, "Q.1"),
        copper(rect([5.0, 6.0], [0.25, 0.25]), 1, "Q.2"),
        copper(rect([5.0, 4.0], [0.25, 0.25]), 1, "Q.3"),
    ];
    let board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [5.0, 5.0], "P.1"),
                    terminal(1, [5.0, 5.0], "P.2"),
                    terminal(2, [2.0, 8.0], "P.3"),
                ],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![
                    terminal(3, [6.0, 5.0], "Q.1"),
                    terminal(4, [5.0, 6.0], "Q.2"),
                    terminal(5, [5.0, 4.0], "Q.3"),
                ],
            },
        ],
        vec![plane(0, 0, Some((0.3, 0.4)))],
        vec!["P.3".into()],
    );
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let grid = router.grid().clone();
    let node = |at: [f64; 2]| Node {
        layer: 0,
        cell: grid.nearest_node(at).unwrap() as u32,
    };
    let straight: Vec<Node> = (0..=12)
        .map(|step| node([5.0 + 0.1 * step as f64, 5.0 + 0.1 * step as f64]))
        .collect();
    router.nets[0].branches = vec![Branch {
        nodes: straight,
        start_terminal: 0,
        end_terminal: PLANE_TERMINAL,
    }];
    let (report, _, _) = spoke_report(&router, 0);
    assert_eq!(
        report
            .iter()
            .find(|record| record.terminal == 0)
            .unwrap()
            .tracks,
        1
    );
    assert_eq!(
        report
            .iter()
            .find(|record| record.terminal == 1)
            .unwrap()
            .tracks,
        1
    );
}

#[test]
fn a_spoke_ends_in_a_neck_exactly_the_minimum_width() {
    // P.1's +y spoke ends between its own relief and P.2's, 0.25 mm
    // apart in a zone of 0.25 mm minimum width (1.25 mm between the
    // pads, 0.5 mm gaps): brush centres fit on
    // one line only, which the probe's samples around the end miss and
    // its search along the spoke finds (Castor's J9.10).
    let obstacles = vec![
        copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
        copper(rect([5.0, 6.85], [0.3, 0.3]), 0, "P.2"),
        copper(
            Shape::Circle {
                center: [2.0, 8.0],
                radius: 0.4,
            },
            0,
            "P.3",
        ),
    ];
    let board = board(
        1,
        obstacles,
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![
                terminal(0, [5.0, 5.0], "P.1"),
                terminal(1, [5.0, 6.85], "P.2"),
                terminal(2, [2.0, 8.0], "P.3"),
            ],
        }],
        vec![plane(0, 0, Some((0.5, 0.4)))],
        vec!["P.3".into()],
    );
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let rings = router.thermal_ring_pads(0);
    let probe = fill::FillProbe::new(&router, 0, &rings);
    let end = [5.0, 5.0 + 0.3 + 0.5 + fill::KICAD_MAX_ERROR + 0.125];
    assert!(probe.fill_along(0, end, [0.0, 1.0]));
}

#[test]
fn a_track_ending_in_what_clearances_leave_of_a_spoke_counts() {
    // Q.1's clearance trims the left of P.1's +y spoke (0.4 mm wide)
    // to x >= 4.85; brush centres fit from x = 4.975. P.1's track ends
    // at x = 4.86, 0.01 mm inside what is left of the spoke (PixelWave's
    // C4.1): the DRC counts it as a spoke. The probe's old samples (the
    // spoke's middle line) found no brush there.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [0.28, 0.31]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [2.0, 8.0],
                radius: 0.4,
            },
            0,
            "P.2",
        ),
        copper(rect([4.4, 5.3], [0.25, 0.5]), 1, "Q.1"),
        copper(rect([5.85, 5.0], [0.15, 0.3]), 2, "R.1"),
        copper(rect([5.0, 4.15], [0.3, 0.15]), 2, "R.2"),
    ];
    let board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [5.0, 5.0], "P.1"),
                    terminal(1, [2.0, 8.0], "P.2"),
                ],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![terminal(2, [4.4, 5.3], "Q.1")],
            },
            Net {
                name: "R".into(),
                class: 0,
                terminals: vec![
                    terminal(3, [5.85, 5.0], "R.1"),
                    terminal(4, [5.0, 4.15], "R.2"),
                ],
            },
        ],
        vec![plane(0, 0, Some((0.3, 0.4)))],
        vec!["P.2".into()],
    );
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let grid = router.grid().clone();
    let start = Node {
        layer: 0,
        cell: grid.nearest_node([5.0, 5.0]).unwrap() as u32,
    };
    router.nets[0].branches = vec![Branch {
        nodes: vec![start],
        start_terminal: 0,
        end_terminal: NO_TERMINAL,
    }];
    router.nets[0]
        .adopted
        .insert(0, ([5.0, 5.0], [4.86, 5.46], 0.05));
    let (report, _, _) = spoke_report(&router, 0);
    let record = report.iter().find(|record| record.terminal == 0).unwrap();
    assert_eq!(
        record
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, SpokeOutcome::Good(_)))
            .count(),
        1,
        "{:?}",
        record.outcomes
    );
    assert_eq!(record.tracks, 1);
}

#[test]
fn a_pads_spokes_make_the_pieces_they_land_in_one_outline() {
    // Q's walls above and below P.1 split the pour into a left piece,
    // with P.2 in it, and a right piece with nothing. P.1 has no hole:
    // its spokes overlap at its centre, so the two pieces and the
    // spokes are one outline to KiCad, holding P.1 and P.2, and its
    // right spoke counts too.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [2.0, 5.0],
                radius: 0.4,
            },
            0,
            "P.2",
        ),
        copper(rect([5.0, 2.15], [0.1, 2.15]), 1, "Q.1"),
        copper(rect([5.0, 7.85], [0.1, 2.15]), 1, "Q.2"),
    ];
    let board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [5.0, 5.0], "P.1"),
                    terminal(1, [2.0, 5.0], "P.2"),
                ],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![
                    terminal(2, [5.0, 2.15], "Q.1"),
                    terminal(3, [5.0, 7.85], "Q.2"),
                ],
            },
        ],
        vec![plane(0, 0, Some((0.3, 0.4)))],
        vec!["P.2".into()],
    );
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (report, _, _) = spoke_report(&router, 0);
    let joined = report
        .iter()
        .find(|record| record.terminal == 0)
        .unwrap()
        .outcomes;
    assert!(
        matches!(joined[0], SpokeOutcome::Good(_)) && matches!(joined[1], SpokeOutcome::Good(_)),
        "{joined:?}"
    );
}

#[test]
fn a_solid_pads_merged_vias_keep_their_thermal_spokes() {
    // The lowering gives zone connections per label: "P.1" is solid
    // because its terminal (the larger pad on the back, an exposed pad)
    // is. The through-hole pad merged into it inherits the zone's thermal
    // relief, and KiCad's DRC counts its spokes (ray4477's U1.33 vias,
    // OpenAir's U5.17): it keeps its records and its ring, the terminal's
    // own pad has none.
    let mut obstacles = vec![
        copper(rect([5.0, 5.0], [0.8, 0.4]), 0, "P.1"),
        copper(
            Shape::Circle {
                center: [5.0, 5.0],
                radius: 0.4,
            },
            0,
            "P.1",
        ),
        copper(
            Shape::Circle {
                center: [2.0, 8.0],
                radius: 0.4,
            },
            0,
            "P.2",
        ),
    ];
    obstacles[0].layers = 0b10;
    obstacles[1].layers = 0b11;
    obstacles[2].layers = 0b11;
    let mut board = board(
        2,
        obstacles,
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![
                Terminal {
                    anchor: [5.0, 5.0],
                    layers: 0b10,
                    pad: 0,
                    contact: None,
                    label: "P.1".into(),
                },
                Terminal {
                    anchor: [2.0, 8.0],
                    layers: 0b11,
                    pad: 2,
                    contact: None,
                    label: "P.2".into(),
                },
            ],
        }],
        vec![plane(0, 0, Some((0.3, 0.4))), plane(0, 1, Some((0.3, 0.4)))],
        vec!["P.1".into(), "P.2".into()],
    );
    board.merged_pads.push(crate::board::MergedPad {
        net: 0,
        host: 0,
        pad: 1,
        anchor: [5.0, 5.0],
        layers: 0b11,
        padstack: false,
    });
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (report, _, _) = spoke_report(&router, 0);
    let layers_of = |pad: usize| -> Vec<usize> {
        report
            .iter()
            .filter(|record| record.pad == pad)
            .map(|record| record.layer)
            .collect()
    };
    assert_eq!(layers_of(1), vec![0, 1]);
    assert!(layers_of(0).is_empty());
    let rings = router.thermal_ring_pads(0);
    assert!(rings[0].iter().any(|(pad, _)| *pad == 1));
}

/// `one_spoke_board` with Q's zone of a higher priority above P.1: P's
/// pour leaves Q's outline out (`Plane::excluded`), and Q's polygon is
/// that of a plane of Q on the layer.
fn zone_above_board(keepout: bool) -> Board {
    let mut board = one_spoke_board();
    // Q.2 above P.1 goes: the +y spoke would land but for the zone.
    board.obstacles[3].shape = rect([9.0, 9.0], [0.25, 0.25]);
    board.nets[1].terminals[1].anchor = [9.0, 9.0];
    let region = vec![[3.0, 6.0], [7.0, 6.0], [7.0, 7.0], [3.0, 7.0]];
    board.planes[0].excluded.push(region.clone());
    if !keepout {
        let mut above = plane(1, 0, None);
        above.polygon = region;
        board.planes.push(above);
    }
    board
}

#[test]
fn another_nets_zone_of_a_higher_priority_keeps_its_clearance() {
    // Q's zone above P.1 fills up to its outline at y = 6, and P's fill
    // keeps the clearance (0.2) and half its minimum width from that
    // fill (USSTRocketry's /Vin under GND's zone of priority 3). P.1's
    // +y spoke ends at 5.725, 0.275 from the outline: no fill there. A
    // keepout's region is cut exactly: the spoke lands, as before. A
    // track of P ending 0.2 from the outline still ends in fill (its
    // own copper keeps Q's fill away: robotique's D3.1).
    let outcome = |keepout: bool| {
        let board = zone_above_board(keepout);
        let router = Router::new(
            &board,
            &Config {
                pitches: vec![0.1],
                ..Config::default()
            },
        );
        let record = spoke_report(&router, 0)
            .0
            .into_iter()
            .find(|record| record.terminal == 0)
            .unwrap();
        record.outcomes[2]
    };
    assert_eq!(outcome(false), SpokeOutcome::NoFill);
    assert!(matches!(outcome(true), SpokeOutcome::Good(_)));

    let board = zone_above_board(false);
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let grid = router.grid().clone();
    let node = |at: [f64; 2]| Node {
        layer: 0,
        cell: grid.nearest_node(at).unwrap() as u32,
    };
    router.nets[0].branches = vec![Branch {
        nodes: (0..=8)
            .map(|step| node([5.0, 5.0 + 0.1 * step as f64]))
            .collect(),
        start_terminal: 0,
        end_terminal: PLANE_TERMINAL,
    }];
    let record = spoke_report(&router, 0)
        .0
        .into_iter()
        .find(|record| record.terminal == 0)
        .unwrap();
    assert_eq!(record.tracks, 1);
}

/// `one_spoke_board` with a fourth pad of Q left of P.1: none of P.1's
/// spokes lands, and the diagonals past Q's corners reach the fill.
fn spokeless_board() -> Board {
    let mut board = one_spoke_board();
    board
        .obstacles
        .push(copper(rect([4.0, 5.0], [0.25, 0.25]), 1, "Q.4"));
    board.nets[1].terminals.push(terminal(5, [4.0, 5.0], "Q.4"));
    board
}

#[test]
fn a_pad_without_a_spoke_in_the_model_gets_a_track_into_the_fill() {
    // The model finds P.1 with no spoke at all: no thermal connection,
    // no starved thermal. KiCad found one on 22 such pads of 85 new
    // findings at scale, so the pad gets one straight track into kept
    // fill, which KiCad's DRC counts only while the pad is short.
    let board = spokeless_board();
    let config = |spokeless_tracks: bool| Config {
        pitches: vec![0.1],
        spokeless_tracks,
        ..Config::default()
    };
    let mut router = Router::new(&board, &config(true));
    let record = |router: &Router| {
        spoke_report(router, 0)
            .0
            .into_iter()
            .find(|record| record.terminal == 0)
            .unwrap()
    };
    let before = record(&router);
    assert!(Router::spokeless(&before));
    assert_eq!(Router::spokes_short(&before), 0);
    assert_eq!(router.add_manual_spokes(), 1);
    let branch = router.nets[0].branches[0].clone();
    assert_eq!(branch.start_terminal, 0);
    assert!(router.branch_is_legal(0, &branch));
    assert_eq!(record(&router).tracks, 1);
    // Once: the pad has its track.
    assert_eq!(router.add_manual_spokes(), 0);
    let mut router = Router::new(&board, &config(false));
    assert_eq!(router.add_manual_spokes(), 0);
}

#[test]
fn a_net_crossing_a_starved_pads_spoke_is_routed_around_it() {
    // P.1 has two spokes, -x and -y (Q.1 and Q.2 take +x and +y); R runs
    // straight across its -y spoke, which no longer lands: one spoke,
    // starved. R is routed again around the spoke's strip, and P.1 has
    // its two spokes back (74th's ESP32-H2 C1.2).
    let mut board = one_spoke_board();
    board.obstacles[4].shape = rect([9.0, 1.0], [0.25, 0.25]);
    board.nets[1].terminals[2].anchor = [9.0, 1.0];
    board
        .obstacles
        .push(copper(rect([2.5, 4.4], [0.2, 0.2]), 2, "R.1"));
    board
        .obstacles
        .push(copper(rect([7.5, 4.4], [0.2, 0.2]), 2, "R.2"));
    board.nets.push(Net {
        name: "R".into(),
        class: 0,
        terminals: vec![
            terminal(5, [2.5, 4.4], "R.1"),
            terminal(6, [7.5, 4.4], "R.2"),
        ],
    });
    let mut router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let grid = router.grid().clone();
    let node = |at: [f64; 2]| Node {
        layer: 0,
        cell: grid.nearest_node(at).unwrap() as u32,
    };
    let short = |router: &Router| {
        spoke_report(router, 0)
            .0
            .iter()
            .find(|record| record.terminal == 0)
            .map(Router::spokes_short)
            .unwrap()
    };
    assert_eq!(short(&router), 0);
    let state = &mut router.nets[2];
    state.branches = vec![Branch {
        nodes: (0..=50)
            .map(|step| node([2.5 + 0.1 * step as f64, 4.4]))
            .collect(),
        start_terminal: 0,
        end_terminal: 1,
    }];
    state.connected.fill(true);
    state.complete = true;
    router.stamp(2);
    assert_eq!(short(&router), 1);
    assert_eq!(router.free_lost_spokes(), 1);
    assert!(router.nets[2].complete);
    assert_eq!(short(&router), 0);
    let mut off = Config {
        pitches: vec![0.1],
        ..Config::default()
    };
    off.lost_spoke_repair = false;
    let mut router = Router::new(&board, &off);
    assert_eq!(router.free_lost_spokes(), 0);
}

#[test]
fn the_copper_check_joins_what_overlaps_on_a_shared_layer() {
    // P.1 and P.2 on the front, P.3 on the back. A front track joins P.1
    // and P.2; a back track from P.2 reaches P.3 only through the via at
    // P.2 (a front pad does not join a back track by itself).
    let mut back = copper(rect([8.0, 5.0], [0.3, 0.3]), 0, "P.3");
    back.layers = 2;
    let obstacles = vec![
        copper(rect([2.0, 5.0], [0.3, 0.3]), 0, "P.1"),
        copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.2"),
        back,
    ];
    let mut terminals = vec![
        terminal(0, [2.0, 5.0], "P.1"),
        terminal(1, [5.0, 5.0], "P.2"),
        terminal(2, [8.0, 5.0], "P.3"),
    ];
    terminals[2].layers = 2;
    let nets = vec![Net {
        name: "P".into(),
        class: 0,
        terminals,
    }];
    let board = board(2, obstacles, nets, Vec::new(), Vec::new());
    let segment = |layer: usize, start: [f64; 2], end: [f64; 2]| crate::board::Segment {
        layer,
        start,
        end,
        width: 0.25,
    };
    let mut route = crate::board::NetRoute {
        segments: vec![
            segment(0, [2.0, 5.0], [5.0, 5.0]),
            segment(1, [5.0, 5.0], [8.0, 5.0]),
        ],
        vias: Vec::new(),
    };
    let components = crate::verify::copper_components(&board, 0, &route);
    assert_eq!(components[0], components[1]);
    assert_ne!(components[1], components[2]);
    route.vias.push(crate::board::Via {
        at: [5.0, 5.0],
        diameter: 0.6,
        drill: 0.3,
    });
    let components = crate::verify::copper_components(&board, 0, &route);
    assert_eq!(components[0], components[2]);
    // A track short of the pad by more than nothing joins nothing.
    route.segments[0].end = [4.5, 5.0];
    let components = crate::verify::copper_components(&board, 0, &route);
    assert_ne!(components[0], components[1]);
}

#[test]
fn a_solid_pad_joins_only_the_fill_that_reaches_it() {
    // A bar of Q across the whole pour splits it 0.3 mm below P.1, a
    // solidly connected pad: its fill never reaches P.1 (Q's clearance and
    // the brush keep it 0.6 mm off), so P.1 and P.2 below are apart.
    // ORBIT's U2.14 read as joined to a piece 0.3 mm past its edge.
    let obstacles = vec![
        copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
        copper(rect([5.0, 8.0], [0.3, 0.3]), 0, "P.2"),
        copper(rect([5.0, 5.65], [5.5, 0.05]), 1, "Q.1"),
    ];
    let mut solid = plane(0, 0, Some((0.3, 0.4)));
    solid.solid = true;
    let board = board(
        1,
        obstacles,
        vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [5.0, 5.0], "P.1"),
                    terminal(1, [5.0, 8.0], "P.2"),
                ],
            },
            Net {
                name: "Q".into(),
                class: 0,
                terminals: vec![terminal(2, [5.0, 5.65], "Q.1")],
            },
        ],
        vec![solid],
        Vec::new(),
    );
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (pours, mut parent, _) = router.analyze_pours(0);
    let base = pours.pieces + 1;
    assert_ne!(find(&mut parent, base), find(&mut parent, base + 1));
}

#[test]
fn a_pad_without_zone_connection_is_kept_out_of_the_fill() {
    // P.1 is set to no zone connection: KiCad's fill keeps the zone's
    // clearance from it as from another net's pad, so no fill node lies
    // on it or within the clearance and half the minimum width of it
    // (1999AZZAR's J2.RN: the pour model filled over the pad and a
    // branch of no length "connected" it).
    let obstacles = vec![
        copper(rect([5.0, 5.0], [0.5, 0.5]), 0, "P.1"),
        copper(rect([2.0, 2.0], [0.5, 0.5]), 0, "P.2"),
    ];
    let mut board = board(
        1,
        obstacles,
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![
                terminal(0, [5.0, 5.0], "P.1"),
                terminal(1, [2.0, 2.0], "P.2"),
            ],
        }],
        vec![plane(0, 0, Some((0.3, 0.4)))],
        Vec::new(),
    );
    board.isolated_pads = vec!["P.1".into()];
    board.isolated_pad_obstacles = vec![0];
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (pours, _, _) = router.analyze_pours(0);
    let label = |point: [f64; 2]| pours.label[0][router.grid.nearest_node(point).unwrap()];
    // Clearance 0.2 and half of 0.25: 0.325 from the pad.
    assert_eq!(label([5.0, 5.0]), 0);
    assert_eq!(label([5.8, 5.0]), 0);
    assert_ne!(label([5.9, 5.0]), 0);
}

#[test]
fn a_hatched_zone_keeps_an_unconnected_pads_own_larger_clearance() {
    // P.1 (no zone connection) has a local clearance of 0.5 mm over the
    // zone's 0.2: KiCad 10.0.7's hatched fill keeps the larger
    // (buildCopperItemClearances), its solid fill the zone's alone
    // (knockoutThermalReliefs). Half the minimum width 0.125 beyond.
    let reach = |hatch: Option<crate::board::Hatch>| {
        let mut obstacles = vec![
            copper(rect([5.0, 5.0], [0.5, 0.5]), 0, "P.1"),
            copper(rect([2.0, 2.0], [0.5, 0.5]), 0, "P.2"),
        ];
        obstacles[0].clearance_override = Some(0.5);
        let mut board = board(
            1,
            obstacles,
            vec![Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [5.0, 5.0], "P.1"),
                    terminal(1, [2.0, 2.0], "P.2"),
                ],
            }],
            vec![Plane {
                hatch,
                ..plane(0, 0, Some((0.3, 0.4)))
            }],
            Vec::new(),
        );
        board.isolated_pads = vec!["P.1".into()];
        board.isolated_pad_obstacles = vec![0];
        let router = Router::new(
            &board,
            &Config {
                pitches: vec![0.1],
                ..Config::default()
            },
        );
        router.isolated_knockouts(0)[0].clone()
    };
    let close = |found: Vec<(usize, f64)>, expected: f64| {
        found.len() == 1 && found[0].0 == 0 && (found[0].1 - expected).abs() < 1.0e-9
    };
    assert!(close(reach(None), 0.325));
    let hatch = crate::board::Hatch {
        thickness: 0.3,
        gap: 0.5,
        orientation: 0.0,
        smoothing_level: 0,
        smoothing_value: 0.0,
        min_hole_area: 0.3,
        via_reliefs: false,
    };
    assert!(close(reach(Some(hatch)), 0.625));
}

#[test]
fn the_fill_keeps_its_brush_inside_the_zone_outline() {
    // KiCad's fill holds no brush centre closer than half the minimum
    // width (0.125 mm) to the zone's outline; the pour mask held nodes on
    // the outline itself (mothenjoyer69's GND_B: a column of them 0.1 mm
    // inside joined pieces KiCad keeps apart).
    let board = board(
        1,
        vec![
            copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
            copper(rect([2.0, 5.0], [0.3, 0.3]), 0, "P.2"),
        ],
        vec![Net {
            name: "P".into(),
            class: 0,
            terminals: vec![
                terminal(0, [5.0, 5.0], "P.1"),
                terminal(1, [2.0, 5.0], "P.2"),
            ],
        }],
        vec![Plane {
            polygon: vec![[1.0, 1.0], [9.0, 1.0], [9.0, 9.0], [1.0, 9.0]],
            ..plane(0, 0, Some((0.3, 0.4)))
        }],
        Vec::new(),
    );
    let router = Router::new(
        &board,
        &Config {
            pitches: vec![0.1],
            ..Config::default()
        },
    );
    let (mask, _) = router.pour_masks(0, false);
    let inside = |point: [f64; 2]| mask[0][router.grid.nearest_node(point).unwrap()];
    assert!(!inside([1.1, 5.0]));
    assert!(inside([1.2, 5.0]));
    assert!(!inside([5.0, 8.9]));
    assert!(inside([5.0, 8.8]));
}

#[test]
fn a_custom_pad_spoke_is_no_wider_than_its_anchor() {
    // KiCad caps a spoke's width by the pad's size, a custom pad's by its
    // anchor's: with a 0.1 mm anchor no spoke reaches the minimum width
    // and the pad stays apart from the pour (Cuprum77's paw pads); with a
    // 1 mm anchor its spokes join it to the pour and to P.2.
    let joined = |anchor_radius: f64| {
        let anchor = Shape::Circle {
            center: [3.0, 5.0],
            radius: anchor_radius,
        };
        let obstacles = vec![
            copper(
                Shape::Union {
                    parts: vec![anchor.clone(), rect([3.0, 5.0], [1.0, 1.0])],
                },
                0,
                "P.1",
            ),
            copper(rect([7.0, 5.0], [0.5, 0.5]), 0, "P.2"),
        ];
        let mut custom = terminal(0, [3.0, 5.0], "P.1");
        custom.contact = Some(anchor);
        let board = board(
            1,
            obstacles,
            vec![Net {
                name: "P".into(),
                class: 0,
                terminals: vec![custom, terminal(1, [7.0, 5.0], "P.2")],
            }],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            Vec::new(),
        );
        let router = Router::new(
            &board,
            &Config {
                pitches: vec![0.1],
                ..Config::default()
            },
        );
        let (_, mut parent, base) = spoke_report(&router, 0);
        find(&mut parent, base) == find(&mut parent, base + 1)
    };
    assert!(!joined(0.05));
    assert!(joined(0.5));
}

#[test]
fn a_via_of_the_net_has_a_thermal_relief_in_a_hatched_zone() {
    // issue6284's U102.6: in a hatched zone whose pads connect by
    // thermals KiCad 10 gives the net's vias a relief of the thermal gap
    // too (knockoutThermalReliefs), and a pad's spoke whose end lies in
    // it is dropped. Checked with pcbnew 10.0.7 on a board like this one
    // (a 1 mm pad, gap 0.5, spoke 0.5, a 0.6 mm via of the net 1.95 mm
    // right and 0.55 mm below the pad's centre): the fill crosses the
    // pad's ring on the +x side without the via, not with it, and again
    // with `connect_pads yes` (a solid via has no relief).
    let via = |label: &str| Obstacle {
        layers: 1,
        ..copper(
            Shape::Circle {
                center: [6.95, 5.55],
                radius: 0.3,
            },
            0,
            label,
        )
    };
    let spokes = |reliefs: bool| {
        let obstacles = vec![
            copper(rect([5.0, 5.0], [0.5, 0.5]), 0, "P.1"),
            copper(rect([2.0, 2.0], [0.5, 0.5]), 0, "P.2"),
            via("existing via"),
        ];
        let mut planes = vec![plane(0, 0, Some((0.5, 0.5)))];
        planes[0].hatch = Some(crate::board::Hatch {
            thickness: 1.0,
            gap: 1.5,
            orientation: 0.0,
            smoothing_level: 0,
            smoothing_value: 0.0,
            min_hole_area: 0.3,
            via_reliefs: reliefs,
        });
        let board = board(
            1,
            obstacles,
            vec![Net {
                name: "GND".into(),
                class: 0,
                terminals: vec![
                    terminal(0, [5.0, 5.0], "P.1"),
                    terminal(1, [2.0, 2.0], "P.2"),
                ],
            }],
            planes,
            Vec::new(),
        );
        let router = Router::new(
            &board,
            &Config {
                pitches: vec![0.1],
                ..Config::default()
            },
        );
        let (report, _, _) = spoke_report(&router, 0);
        // The lattice keeps the relief out of the pour's pieces but for
        // the via's spokes along the axes.
        let (pours, _, _) = router.analyze_pours(0);
        let label = |at: [f64; 2]| pours.label[0][router.grid.nearest_node(at).unwrap()];
        assert_eq!(label([7.4, 6.0]) == 0, reliefs);
        assert_ne!(label([6.95, 6.3]), 0);
        report
            .iter()
            .find(|record| record.pad == 0)
            .unwrap()
            .outcomes
    };
    // The +x spoke: into the fill without the via's relief, none with it.
    assert!(
        matches!(spokes(false)[0], SpokeOutcome::Good(_)),
        "{:?}",
        spokes(false)
    );
    assert!(
        matches!(spokes(true)[0], SpokeOutcome::NoFill),
        "{:?}",
        spokes(true)
    );
}

#[test]
fn a_spoke_end_inside_a_clearance_is_dropped() {
    // anthracite's R503.2: KiCad keeps a spoke only where its end itself
    // lies in the fill. P.1's +x spoke ends at (5.93, 5.0) (the gap 0.3,
    // half the minimum width and 0.005 past the pad); Q.1's edge 0.202
    // mm from it keeps the spoke, 0.198 mm drops it under a 0.2 mm
    // clearance (pcbnew 10.0.7 on this board: the fill crosses the pad's
    // ring at (5.65, 5.0) with the first, not with the second). A brush
    // centre half the minimum width beside the end fits either way within
    // the probe's tolerance, which the model took for a spoke.
    let outcome = |edge: f64| {
        let obstacles = vec![
            copper(rect([5.0, 5.0], [0.5, 0.5]), 0, "P.1"),
            copper(rect([2.0, 2.0], [0.5, 0.5]), 0, "P.2"),
            copper(rect([6.1, edge - 0.25], [0.4, 0.25]), 1, "Q.1"),
            copper(rect([8.5, 8.5], [0.4, 0.25]), 1, "Q.2"),
        ];
        let board = board(
            1,
            obstacles,
            vec![
                Net {
                    name: "GND".into(),
                    class: 0,
                    terminals: vec![
                        terminal(0, [5.0, 5.0], "P.1"),
                        terminal(1, [2.0, 2.0], "P.2"),
                    ],
                },
                Net {
                    name: "Q".into(),
                    class: 0,
                    terminals: vec![
                        terminal(2, [6.1, edge - 0.25], "Q.1"),
                        terminal(3, [8.5, 8.5], "Q.2"),
                    ],
                },
            ],
            vec![plane(0, 0, Some((0.3, 0.5)))],
            Vec::new(),
        );
        let router = Router::new(
            &board,
            &Config {
                pitches: vec![0.1],
                ..Config::default()
            },
        );
        let (report, _, _) = spoke_report(&router, 0);
        report
            .iter()
            .find(|record| record.pad == 0)
            .unwrap()
            .outcomes[0]
    };
    assert!(
        matches!(outcome(4.798), SpokeOutcome::Good(_)),
        "{:?}",
        outcome(4.798)
    );
    assert!(
        matches!(outcome(4.802), SpokeOutcome::NoFill),
        "{:?}",
        outcome(4.802)
    );
}

#[test]
fn a_build_is_priced_by_its_counts() {
    // The pricing: each count at its constant, in the reference's
    // nanoseconds, scaled to the process's rate as the pour model is.
    let counts = BuildCounts {
        map_nodes: 1_000_000,
        tests: 2_000_000,
        edges: 3_000_000,
        step_edges: 400_000,
    };
    let nanos = 1.0e6 * BUILD_MAP_NANOS
        + 2.0e6 * BUILD_TEST_NANOS
        + 3.0e6 * BUILD_EDGE_NANOS
        + 4.0e5 * BUILD_STEP_NANOS;
    assert!((counts.work_nanos() - nanos).abs() < 1.0e-6);
    assert!((counts.work_seconds() - nanos / 1.0e9 * reference_time_scale()).abs() < 1.0e-12);
    assert_eq!(BuildCounts::default().work_nanos(), 0.0);
}

#[test]
fn a_build_counts_its_maps_tests_and_pour_masks() {
    // The same board builds to the same counts and work every time (the
    // ladder charges it as work: no wall clock in it); every class's map
    // nodes count; a connected pour adds the nodes of its box and the
    // edges of the nodes it may fill; the result carries the work.
    let board = one_spoke_board();
    let config = Config {
        pitches: vec![0.1],
        share_static_maps: false,
        ..Config::default()
    };
    let router = Router::new(&board, &config);
    let counts = router.build_counts();
    assert_eq!(counts, Router::new(&board, &config).build_counts());
    let cells = router.grid().cells() as u64;
    assert_eq!(
        counts.map_nodes,
        cells * board.layer_count as u64 * board.classes.len() as u64
    );
    assert!(counts.tests >= cells && counts.edges > 0);
    assert!((router.build_work() - counts.work_seconds()).abs() < 1.0e-12);
    assert!(router.build_work() > 0.0);
    let mut unpoured = board.clone();
    for pour in &mut unpoured.planes {
        pour.connect = false;
    }
    let without = Router::new(&unpoured, &config).build_counts();
    assert_eq!(without.map_nodes, counts.map_nodes);
    assert!(without.tests < counts.tests && without.edges < counts.edges);
    // The routing's result carries the build and every pour analysis.
    let mut routed = Router::new(&board, &config);
    let result = routed.run_in_place();
    assert_eq!(result.build_work, routed.build_work());
    assert_eq!(result.pour_work, routed.pour_work());
    assert!(result.pour_work >= result.manual_spokes_work - 1.0e-12);
}

#[test]
fn routers_of_one_board_and_lattice_share_their_static_maps() {
    // A router whose static maps another router or a hold of the same
    // board, lattice and classes built shares them; what it counts is the
    // same either way (the canonical price: `build_work` as if it built
    // everything, the shared parts keyed in `shared_build`), and what the
    // process did is its own maps (`physical_build_work`). The board and
    // the pitch are this test's own (the maps are shared across the
    // process).
    let board = one_spoke_board();
    let config = Config {
        pitches: vec![0.0913],
        ..Config::default()
    };
    let alone = Router::new(
        &board,
        &Config {
            share_static_maps: false,
            ..config.clone()
        },
    );
    let held = hold_static_maps(&board, &config);
    assert!(held.built_work() > 0.0);
    assert_eq!(held.key(), static_maps_key(&board, &config));
    assert!(held.bytes() > 0);
    let shared = Router::new(&board, &config);
    assert!(shared.statics_shared && !alone.statics_shared);
    assert!(Arc::ptr_eq(
        &shared.statics,
        &Router::new(&board, &config).statics
    ));
    // The same counts and work as a router that built them all.
    assert_eq!(shared.build_counts(), alone.build_counts());
    assert!((shared.build_work() - alone.build_work()).abs() < 1.0e-12);
    let parts = shared.shared_build();
    assert_eq!(parts.key(), held.key());
    assert!(alone.shared_build().statics.is_none());
    // The hold built the shared parts; the router did its own maps.
    assert!((held.built_work() - parts.total()).abs() < 1.0e-9);
    assert!(
        (shared.physical_build_work() + parts.total() - alone.build_work()).abs() < 1.0e-9,
        "{} {} {}",
        shared.physical_build_work(),
        parts.total(),
        alone.build_work()
    );
    // A second hold finds them: no work.
    assert_eq!(hold_static_maps(&board, &config).built_work(), 0.0);
    // Held maps count in the process's held bytes.
    assert!(held_maps_bytes().0 > 0);
    // The maps are the same as built alone; so is the routing.
    assert_eq!(shared.statics.len(), alone.statics.len());
    for (a, b) in shared.statics.iter().zip(alone.statics.iter()) {
        assert_eq!(a.trace, b.trace);
        assert_eq!(a.via_blocked, b.via_blocked);
    }
    // Nothing keeps them once their holders are gone (until 2026-10-21
    // the last maps built stayed for the process's life).
    drop(held);
    drop(shared);
    assert!(hold_static_maps(&board, &config).built_work() > 0.0);
    let mut one = Router::new(&board, &config);
    let mut two = Router::new(
        &board,
        &Config {
            share_static_maps: false,
            ..config.clone()
        },
    );
    one.run_in_place();
    two.run_in_place();
    assert_eq!(one.fingerprint(), two.fingerprint());
}

#[test]
fn a_negotiation_bound_of_nothing_is_one_expansion() {
    // 0 in `negotiation_expansions` means no limit: a bound of no work
    // (the ladder's share with nothing left) is the least limit there is,
    // so the negotiation stops after its first iteration by it.
    assert_eq!(negotiation_expansions(0.0), 1);
    assert_eq!(negotiation_expansions(-1.0), 1);
    assert_eq!(
        negotiation_expansions(2.0),
        (2.0 * expansions_per_second()) as u64
    );
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Unit tests of the spoke model's stages that need no router. The
//! model as a whole is tested through `analyze_pours_full` in
//! `router/tests.rs`.

use super::drc::record_seam;
use super::pieces::{overlapping_spoke_joins, tally_spokes, track_meets_spoke};
use super::*;

/// A spoke of record 0 in slot `index` from `anchor` along `direction`.
fn spoke(index: usize, anchor: [f64; 2], direction: [f64; 2], width: f64) -> Spoke {
    Spoke {
        record: 0,
        index,
        pad: 0,
        layer: 0,
        anchor,
        direction,
        edge: 0.5,
        tip: 1.5,
        gap: 0.3,
        width,
        side: 0.0,
    }
}

fn record(terminal: usize, outcomes: [SpokeOutcome; 4]) -> SpokeRecord {
    SpokeRecord {
        terminal,
        pad: terminal,
        anchor: [0.0, 0.0],
        crossings: 0,
        layer: 0,
        round: false,
        outcomes,
        tracks: 0,
        seam_short: 0,
        seam_zone: None,
        drawn: true,
        required: 2,
    }
}

fn hole(label: &str, center: [f64; 2], radius: f64) -> crate::board::Obstacle {
    crate::board::Obstacle {
        shape: crate::geometry::Shape::Circle { center, radius },
        layers: 0b11,
        kind: crate::board::ObstacleKind::Hole,
        net: None,
        clearance: 0.0,
        clearance_override: None,
        plated: true,
        blocks_tracks: true,
        blocks_vias: true,
        label: label.to_string(),
    }
}

#[test]
fn a_spoke_holds_its_strip_up_to_its_tip() {
    let spoke = spoke(0, [1.0, 1.0], [1.0, 0.0], 0.4);
    assert_eq!(spoke.end(), [2.5, 1.0]);
    assert!(spoke.contains([2.0, 1.15]));
    assert!(spoke.contains([2.5000005, 1.0]));
    assert!(!spoke.contains([2.0, 1.25]));
    assert!(!spoke.contains([0.99, 1.0]));
    assert!(!spoke.contains([2.51, 1.0]));
}

#[test]
fn a_pad_counts_once_in_the_outline_of_merged_pieces() {
    let items = HashMap::from([(1, 1), (2, 0), (3, 2)]);
    let pads = HashMap::from([(1, vec![7]), (2, vec![7, 8])]);
    let mut pieces = PieceItems::new(items, pads);
    assert_eq!(pieces.count(1), 2);
    assert_eq!(pieces.count(2), 2);
    assert_eq!(pieces.count(3), 2);
    assert_eq!(pieces.count(9), 0);
    let mut parent: Vec<usize> = (0..4).collect();
    pieces.merge(&mut parent, 1, 2);
    // Items 1 + 0, pads 7 and 8 once each.
    assert_eq!(pieces.count(1), 3);
    assert_eq!(pieces.count(2), 3);
    assert_eq!(pieces.count(3), 2);
    assert_eq!(find(&mut parent, 1), find(&mut parent, 2));
    assert_ne!(find(&mut parent, 1), find(&mut parent, 3));
    pieces.merge(&mut parent, 3, 2);
    assert_eq!(pieces.count(1), 5);
}

#[test]
fn a_pads_plated_hole_is_the_one_nearest_it() {
    let mut pad = hole("J1.1 plated hole", [9.0, 9.0], 2.0);
    pad.kind = crate::board::ObstacleKind::Copper;
    let obstacles = vec![
        hole("J1.1 plated hole", [0.0, 0.0], 0.4),
        hole("J1.1 plated hole", [5.0, 0.0], 0.5),
        hole("J1.2", [2.0, 0.0], 0.3),
        pad,
    ];
    let holes = PlatedHoles::of(&obstacles);
    assert_eq!(holes.radius_at("J1.1", [0.1, 0.0]), 0.4);
    assert_eq!(holes.radius_at("J1.1", [4.9, 0.0]), 0.5);
    assert_eq!(holes.radius_at("J1.1", [9.0, 9.0]), 0.5);
    assert_eq!(holes.radius_at("J1.2", [2.0, 0.0]), 0.0);
}

#[test]
fn overlapping_spokes_join_their_pieces_outside_the_hole() {
    let outcomes = [
        SpokeOutcome::Good(3),
        SpokeOutcome::Good(4),
        SpokeOutcome::Good(5),
        SpokeOutcome::Good(3),
    ];
    let east = spoke(0, [0.0, 0.0], [1.0, 0.0], 0.5);
    let west = spoke(1, [0.0, 0.0], [-1.0, 0.0], 0.5);
    let north = spoke(2, [0.0, 0.0], [0.0, 1.0], 0.5);
    let south = spoke(3, [0.0, 0.0], [0.0, -1.0], 0.5);
    // At right angles they overlap up to half the width times the root
    // of two from the centre, facing up to half the width.
    assert_eq!(
        overlapping_spoke_joins(&[&east, &north], &outcomes, 0.0),
        vec![(3, 5)]
    );
    assert_eq!(
        overlapping_spoke_joins(&[&east, &north], &outcomes, 0.34),
        vec![(3, 5)]
    );
    assert!(overlapping_spoke_joins(&[&east, &north], &outcomes, 0.36).is_empty());
    assert_eq!(
        overlapping_spoke_joins(&[&east, &west], &outcomes, 0.2),
        vec![(3, 4)]
    );
    assert!(overlapping_spoke_joins(&[&east, &west], &outcomes, 0.3).is_empty());
    // Spokes into one piece join nothing.
    assert!(overlapping_spoke_joins(&[&east, &south], &outcomes, 0.0).is_empty());
    assert_eq!(
        overlapping_spoke_joins(&[&east, &west, &north], &outcomes, 0.0),
        vec![(3, 4), (3, 5), (4, 5)]
    );
}

#[test]
fn a_track_touches_a_spoke_outside_the_hole_or_past_the_ring() {
    let spoke = spoke(0, [0.0, 0.0], [1.0, 0.0], 0.4);
    let across = |x: f64, radius: f64| vec![([x, -1.0], [x, 1.0], radius)];
    assert!(track_meets_spoke(&spoke, true, 0.0, &across(1.0, 0.1)));
    assert!(!track_meets_spoke(&spoke, true, 0.0, &across(1.7, 0.1)));
    // Cut off from the pad, the spoke begins at the ring (0.65).
    assert!(track_meets_spoke(&spoke, true, 0.0, &across(0.55, 0.05)));
    assert!(!track_meets_spoke(&spoke, false, 0.0, &across(0.55, 0.05)));
    assert!(track_meets_spoke(&spoke, false, 0.0, &across(0.62, 0.05)));
    // A stub inside the hole touches nothing; one leaving it does.
    let stub = |to: f64| vec![([0.0, 0.0], [to, 0.0], 0.05)];
    assert!(!track_meets_spoke(&spoke, true, 0.3, &stub(0.2)));
    assert!(track_meets_spoke(&spoke, true, 0.3, &stub(0.5)));
}

#[test]
fn pass_two_counts_spokes_into_pieces_with_another_item() {
    use SpokeOutcome::*;
    let terminal_base = 10;
    let mut parent: Vec<usize> = (0..12).collect();
    let mut records = vec![
        record(0, [Good(5), Good(6), Pad(1), NoFill]),
        record(1, [Good(7), Good(0), Good(0), Blocked]),
    ];
    let pieces = PieceItems::new(
        HashMap::from([(5, 1)]),
        HashMap::from([(5, vec![10]), (6, vec![10])]),
    );
    let tallies = tally_spokes(
        &mut parent,
        terminal_base,
        &mut records,
        &[[false; 4], [false, true, false, false]],
        &[[true; 4], [false, true, true, true]],
        &[false, false],
        &pieces,
    );
    assert_eq!(tallies, vec![(2, vec![6]), (1, vec![])]);
    assert_eq!(records[0].outcomes, [Good(5), Island(6), Pad(1), NoFill]);
    // Cut off from the pad into a piece holding nothing: gone; a sliver
    // the pad's track touches counts, one it does not is an island.
    assert_eq!(records[1].outcomes, [NoFill, Good(0), Island(0), Blocked]);
    assert_eq!(find(&mut parent, 10), find(&mut parent, 5));
    assert_eq!(find(&mut parent, 10), find(&mut parent, 11));
    assert_ne!(find(&mut parent, 10), find(&mut parent, 6));
}

#[test]
fn inside_its_terminals_pad_a_spoke_counts_and_joins() {
    use SpokeOutcome::*;
    let mut parent: Vec<usize> = (0..12).collect();
    let mut records = vec![record(0, [Good(5), Good(6), Good(0), NoFill])];
    let pieces = PieceItems::new(HashMap::new(), HashMap::new());
    let tallies = tally_spokes(
        &mut parent,
        10,
        &mut records,
        &[[false; 4]],
        &[[true, false, true, true]],
        &[true],
        &pieces,
    );
    assert_eq!(tallies, vec![(3, vec![])]);
    assert_eq!(records[0].outcomes, [Good(5), Good(6), Good(0), NoFill]);
    assert_eq!(find(&mut parent, 10), find(&mut parent, 5));
    assert_ne!(find(&mut parent, 10), find(&mut parent, 6));
}

#[test]
fn the_seam_shortfall_is_the_worst_zones() {
    let mut short = record(0, [SpokeOutcome::NoFill; 4]);
    // Zone 3: two spokes. Zone 4: one island spoke and nothing else.
    // Zone 5: only a track, no fill across the ring: no connection.
    record_seam(
        &mut short,
        &[3, 4, 5],
        &[[2, 0, 0, 0], [0, 1, 0, 0], [0, 0, 0, 1]],
    );
    assert_eq!((short.seam_short, short.seam_zone), (1, Some(4)));
    // A track counts with the spokes, and then the islands do not.
    let mut tracked = record(0, [SpokeOutcome::NoFill; 4]);
    record_seam(&mut tracked, &[3, 4], &[[1, 1, 0, 1], [0, 0, 1, 0]]);
    assert_eq!((tracked.seam_short, tracked.seam_zone), (1, Some(4)));
    let mut whole = record(0, [SpokeOutcome::NoFill; 4]);
    record_seam(&mut whole, &[3], &[[1, 0, 1, 0]]);
    assert_eq!((whole.seam_short, whole.seam_zone), (0, None));
}

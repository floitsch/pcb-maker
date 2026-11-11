// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use super::guide::{BLOCK, Guide, coarse_distances};

/// The octile distance in blocks between two blocks.
fn octile(a: (usize, usize), b: (usize, usize)) -> f32 {
    let dx = a.0.abs_diff(b.0) as f32;
    let dy = a.1.abs_diff(b.1) as f32;
    let both = dx.min(dy);
    both * std::f32::consts::SQRT_2 + (dx - both) + (dy - both)
}

#[test]
fn coarse_distances_are_octile_to_the_nearest_target_block() {
    let (blocks_x, blocks_y) = (7, 5);
    // Targets in blocks (1, 1) and (6, 4), given as cells.
    let targets = [(BLOCK + 3, BLOCK + 2), (6 * BLOCK, 4 * BLOCK + 7)];
    let distance = coarse_distances(targets.iter().copied(), blocks_x, blocks_y);
    for y in 0..blocks_y {
        for x in 0..blocks_x {
            let expected = octile((x, y), (1, 1)).min(octile((x, y), (6, 4)));
            let got = distance[y * blocks_x + x];
            assert!(
                (got - expected).abs() < 1.0e-5,
                "block ({x}, {y}): {got} against {expected}"
            );
        }
    }
}

#[test]
fn coarse_distances_without_targets_are_infinite() {
    let distance = coarse_distances(std::iter::empty(), 3, 2);
    assert!(distance.iter().all(|d| d.is_infinite()));
}

fn costs() -> Vec<[f32; 8]> {
    let diagonal = std::f32::consts::SQRT_2;
    vec![
        [1.0, diagonal, 2.0, diagonal, 1.0, diagonal, 2.0, diagonal],
        [2.0, diagonal, 1.0, diagonal, 2.0, diagonal, 1.0, diagonal],
    ]
}

#[test]
fn the_heuristic_of_a_plane_target_is_zero() {
    let guide = Guide::new(None, true, std::iter::empty(), (40, 40), costs(), 10.0, 1.0);
    assert_eq!(guide.heuristic(0, 13, 27), 0.0);
}

#[test]
fn the_heuristic_to_points_takes_the_cheaper_of_layer_and_via() {
    let points = [(0, 0, 0.0f32)];
    let guide = Guide::new(
        Some(&points),
        false,
        std::iter::empty(),
        (40, 40),
        costs(),
        3.0,
        1.0,
    );
    // Ten steps along y on layer 0 cost 20 there; a via and layer 1's
    // cheap vertical steps cost 3 + 10.
    assert_eq!(guide.heuristic(0, 0, 10), 13.0);
    // On layer 1 the vertical steps are cheap already.
    assert_eq!(guide.heuristic(1, 0, 10), 10.0);
    // At the point itself, and inside its reach, nothing is left.
    assert_eq!(guide.heuristic(0, 0, 0), 0.0);
    let reaching = [(0, 0, 10.0f32)];
    let guide = Guide::new(
        Some(&reaching),
        false,
        std::iter::empty(),
        (40, 40),
        costs(),
        3.0,
        1.0,
    );
    assert_eq!(guide.heuristic(0, 0, 10), 0.0);
}

#[test]
fn the_coarse_heuristic_is_nothing_within_a_block_and_a_diagonal() {
    let target = [(4usize, 4usize)];
    let guide = Guide::new(
        None,
        false,
        target.iter().copied(),
        (64, 64),
        costs(),
        3.0,
        1.0,
    );
    // Blocks one diagonal away bound nothing (the target may lie at
    // their corner).
    assert_eq!(guide.heuristic(0, 4, 4), 0.0);
    assert_eq!(guide.heuristic(0, BLOCK as i32 + 1, BLOCK as i32 + 1), 0.0);
    // Four blocks along x: (4 - sqrt 2) blocks of the cheapest axis step.
    let far = guide.heuristic(0, 4 * BLOCK as i32, 4);
    let expected = (4.0 - std::f32::consts::SQRT_2) * BLOCK as f32;
    assert!((far - expected).abs() < 1.0e-4, "{far} against {expected}");
}

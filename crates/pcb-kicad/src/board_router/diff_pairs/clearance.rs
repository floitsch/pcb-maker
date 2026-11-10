// Copyright (C) 2026 Toit contributors.

//! The checks of the plan's copper against the board as lowered: how far
//! copper keeps from an obstacle, the obstacles near a point, the layers
//! clear of them, and whether straight stubs or an end's track keep their
//! clearance.

use super::*;
use pcb_router as core;

/// How far copper of `clearance` keeps from `obstacle`.
pub(super) fn keep_from(board: &core::Board, obstacle: &core::Obstacle, clearance: f64) -> f64 {
    match obstacle.kind {
        core::ObstacleKind::Copper => board.copper_clearance(
            &core::RuleClass {
                trace_width: 0.0,
                clearance,
                via_diameter: 0.0,
                via_drill: 0.0,
                layers: 0,
            },
            obstacle,
        ),
        core::ObstacleKind::Keepout => obstacle.clearance,
        core::ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
    }
}

/// The obstacles that block tracks within `reach` of `at`.
pub(super) fn obstacles_near(board: &core::Board, at: Point, reach: f64) -> Vec<usize> {
    scanned(board.obstacles.len());
    board
        .obstacles
        .iter()
        .enumerate()
        .filter(|(_, obstacle)| {
            let bounds = obstacle.shape.aabb();
            obstacle.blocks_tracks
                && at[0] >= bounds.minimum[0] - reach
                && at[0] <= bounds.maximum[0] + reach
                && at[1] >= bounds.minimum[1] - reach
                && at[1] <= bounds.maximum[1] + reach
        })
        .map(|(index, _)| index)
        .collect()
}

/// The layers where a disc of `radius` (plus each obstacle's clearance)
/// at `at` is clear of the obstacles `near` and of the board edge.
pub(super) fn clear_layers(
    board: &core::Board,
    near: &[usize],
    at: Point,
    radius: f64,
    clearance: f64,
    mask: core::LayerMask,
) -> core::LayerMask {
    // The outline's edges twice (inside, the edge's distance).
    tested(2 * board.outline.len());
    if !core::geometry::point_in_polygon(at, &board.outline)
        || core::geometry::polygon_edges(&board.outline)
            .map(|(a, b)| core::geometry::point_segment_distance(at, a, b))
            .fold(f64::INFINITY, f64::min)
            < radius + board.edge_clearance
    {
        return 0;
    }
    let mut free = mask;
    tested(near.len());
    for &index in near {
        let obstacle = &board.obstacles[index];
        if obstacle.layers & free == 0 {
            continue;
        }
        if obstacle.shape.distance_to_point(at) < radius + keep_from(board, obstacle, clearance) {
            free &= !obstacle.layers;
        }
    }
    free
}

/// Whether straight stubs of the pair's width from the pads (`pads`:
/// positive, negative) to their stubs' ends (`ends`) on `layer` keep their
/// clearance from the obstacles `near` (another net's copper, keepouts,
/// holes; the pair's other net its gap), from the board's edge and from
/// each other: then the link carries them (`LinkPlan::fixed`).
pub(super) fn stubs_clear(
    board: &core::Board,
    near: &[usize],
    pair: &Pair,
    pads: [Point; 2],
    ends: [Point; 2],
    layer: core::LayerMask,
) -> bool {
    let half = pair.width / 2.0;
    if core::geometry::segment_segment_distance(pads[0], ends[0], pads[1], ends[1])
        < pair.width + pair.gap.min(pair.clearance) - EPSILON
    {
        return false;
    }
    let nets = [pair.positive, pair.negative];
    tested(2 * (near.len() + board.outline.len()));
    (0..2).all(|polarity| {
        let (pad, end) = (pads[polarity], ends[polarity]);
        let (own, other) = (nets[polarity], nets[1 - polarity]);
        core::geometry::polygon_edges(&board.outline).all(|(a, b)| {
            core::geometry::segment_segment_distance(pad, end, a, b) >= half + board.edge_clearance
        }) && near.iter().all(|&index| {
            let obstacle = &board.obstacles[index];
            if obstacle.layers & layer == 0 || obstacle.net == Some(own) {
                return true;
            }
            // A pad inside a mask opening leaves it by its stub (the
            // router's `stub_is_clear`, `verify`).
            if obstacle.kind == core::ObstacleKind::Keepout
                && obstacle.label.starts_with("solder mask opening")
                && obstacle.shape.contains(pad)
            {
                return true;
            }
            let mut keep = keep_from(board, obstacle, pair.clearance);
            if obstacle.net == Some(other) {
                keep = keep.min(pair.gap);
            }
            obstacle.shape.distance_to_segment(pad, end) >= half + keep - EPSILON
        })
    })
}

/// Whether a track of the pair's width from `at` to `to` on `layer`
/// keeps its clearance from the obstacles `near`, the pair's own copper
/// the pair's gap (either net may end there: where the pads' order crosses
/// the run's, the stubs swap sides).
pub(super) fn end_clear(
    board: &core::Board,
    near: &[usize],
    pair: &Pair,
    (at, to): (Point, Point),
    layer: core::LayerMask,
) -> bool {
    tested(near.len());
    near.iter().all(|&index| {
        let obstacle = &board.obstacles[index];
        if obstacle.layers & layer == 0 {
            return true;
        }
        let mut keep = keep_from(board, obstacle, pair.clearance);
        if obstacle.net == Some(pair.positive) || obstacle.net == Some(pair.negative) {
            keep = keep.min(pair.gap);
        }
        obstacle.shape.distance_to_segment(at, to) >= pair.width / 2.0 + keep
    })
}

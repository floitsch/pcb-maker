// Copyright (C) 2026 Toit contributors.

//! A crossover in a launch's lead, at a crossed end of a link.

use super::*;
use pcb_router as core;

/// A crossover in a launch's lead (`crossover`): where the pads' order at
/// a link end is against the run's (a *crossed* end), one net (`straight`)
/// runs straight on from its stub's end while the other dives under it, a
/// via beside the run on each side of the straight track, and comes up on
/// its far side; the run starts past it, its centreline beside the
/// straight track. KiCad's gap check sees the straight track and the
/// diving net's tracks beside it a gap apart, as in the run.
#[derive(Clone, Debug)]
pub(super) struct Crossover {
    /// The run's start past the crossover (on its centreline).
    pub run_start: Point,
    /// Per polarity: the tracks (layer index, from, to) from the stub's end
    /// to the run's start, in order, and the vias.
    pub segments: [Vec<(usize, Point, Point)>; 2],
    pub vias: [Vec<Point>; 2],
}

/// A crossover at a run start `start` going `outward` on `layer` (one
/// bit), the positive pad on `side` of it (1 left): the stubs' ends at
/// `ends` (positive, negative, on the pads' sides), the run past it with
/// the nets on the other sides. Either net may run straight; the first
/// whose copper (the tracks, the vias on every layer, the diving net's
/// track on another of the pair's layers) keeps every clearance from the
/// obstacles `near` and the edge wins. `None`: no layer or no room.
pub(super) fn crossover(
    board: &core::Board,
    near: &[usize],
    pair: &Pair,
    (start, outward, side): (Point, Point, f64),
    (ends, e): ([Point; 2], f64),
    layer: core::LayerMask,
) -> Option<Crossover> {
    let index = layer.trailing_zeros() as usize;
    let mut others: Vec<usize> = (0..board.layer_count)
        .filter(|&other| other != index && pair.layers & (1 << other) != 0)
        .collect();
    others.sort_by_key(|&other| (other as i64 - index as i64).abs());
    if others.is_empty() {
        return None;
    }
    let v = left(outward);
    let at = |x: f64, y: f64| add(start, add(scale(outward, x), scale(v, y)));
    let (d, w) = (pair.offset(), pair.width);
    let radius = pair.via_diameter / 2.0;
    // A via of the diving net keeps the clearance from the straight track,
    // its hole the hole clearance.
    let apart = (radius + pair.clearance).max(pair.via_drill / 2.0 + pair.hole_clearance)
        + w / 2.0
        + ENDS_MARGIN;
    let round = |value: f64| (value.max(0.0) / 0.01).ceil() * 0.01;
    let (out_near, out_far) = (round(apart - 2.0 * e), round(apart - 2.0 * d));
    for straight in [0usize, 1] {
        let dive = 1 - straight;
        // The straight net's side (in `v`) at the pads.
        let sigma = if straight == 0 { side } else { -side };
        let jog = LAUNCH_STEP;
        let first = at(jog + out_near, -sigma * (e + out_near));
        let second = at(jog + out_near, sigma * (e + 2.0 * d + out_far));
        let back = at(jog + out_near + out_far, sigma * (e + 2.0 * d));
        let end_x = jog + out_near + out_far + LAUNCH_STEP;
        let run_start = at(end_x, sigma * (e + d));
        if core::geometry::distance(first, second) < pair.via_drill + board.hole_to_hole {
            continue;
        }
        let mut segments: [Vec<(usize, Point, Point)>; 2] = Default::default();
        segments[straight].push((index, ends[straight], at(end_x, sigma * e)));
        let corner = at(jog, -sigma * e);
        let diving = &mut segments[dive];
        diving.push((index, ends[dive], corner));
        if out_near > 0.0 {
            diving.push((index, corner, first));
        }
        let mut on_layer = None;
        for &other in &others {
            if end_clear(board, near, pair, (first, second), 1 << other) {
                on_layer = Some(other);
                break;
            }
        }
        let Some(other) = on_layer else {
            continue;
        };
        diving.push((other, first, second));
        if out_far > 0.0 {
            diving.push((index, second, back));
        }
        diving.push((index, back, at(end_x, sigma * (e + 2.0 * d))));
        let vias = [first, second];
        let tracks_clear = segments.iter().flatten().all(|&(layer, from, to)| {
            core::geometry::distance(from, to) < EPSILON
                || end_clear(board, near, pair, (from, to), 1 << layer)
        });
        // The vias' copper keeps the clearance, their holes the hole
        // clearance from other copper and hole to hole from other holes.
        let vias_clear = vias.iter().all(|&via| {
            clear_layers(board, near, via, radius, pair.clearance, board.all_layers())
                == board.all_layers()
                && near.iter().all(|&index| {
                    let obstacle = &board.obstacles[index];
                    let distance = obstacle.shape.distance_to_point(via);
                    match obstacle.kind {
                        core::ObstacleKind::Hole => {
                            distance >= pair.via_drill / 2.0 + board.hole_to_hole
                        }
                        core::ObstacleKind::Copper
                            if obstacle
                                .net
                                .is_none_or(|net| net != pair.positive && net != pair.negative) =>
                        {
                            distance >= pair.via_drill / 2.0 + board.hole_clearance
                        }
                        _ => true,
                    }
                })
        });
        if tracks_clear && vias_clear {
            let mut all_vias: [Vec<Point>; 2] = Default::default();
            all_vias[dive] = vias.to_vec();
            return Some(Crossover {
                run_start,
                segments,
                vias: all_vias,
            });
        }
    }
    None
}

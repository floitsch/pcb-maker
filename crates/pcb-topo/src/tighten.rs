// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Tightening routed copper: any legal routing (the lattice router's) read
//! as a topology and pulled tight, which gives TopoR's look (any-angle
//! tracks, arcs around obstacles, no staircases) with the completion of the
//! router that made it. Each net's tracks become chains between fixed
//! points (pad copper, vias, branch points, ends); each chain is a piece
//! whose embedding is the chain itself. A piece whose tight version the
//! exact verifier rejects keeps its original copper, until the board is
//! clean, so the result is never worse than the input.

use crate::realize::{self, Piece, PlacedVia};
use crate::taut::distance;
use pcb_router::{Board, NetRoute, ObstacleKind, Point, Segment, verify};
use std::collections::HashMap;

fn key(point: Point) -> (i64, i64) {
    ((point[0] * 1.0e5).round() as i64, (point[1] * 1.0e5).round() as i64)
}

/// A chain of one net's segments on one layer.
struct Chain {
    net: usize,
    layer: usize,
    width: f64,
    points: Vec<Point>,
    /// The original segments, kept if the tight version is rejected.
    segments: Vec<Segment>,
}

fn chains(board: &Board, routes: &[NetRoute]) -> Vec<Chain> {
    let mut chains = Vec::new();
    for (net, route) in routes.iter().enumerate() {
        // Nodes per layer and position; edges are segments.
        let mut nodes: HashMap<(usize, (i64, i64)), Vec<usize>> = HashMap::new();
        for (index, segment) in route.segments.iter().enumerate() {
            for end in [segment.start, segment.end] {
                nodes.entry((segment.layer, key(end))).or_default().push(index);
            }
        }
        let vias: Vec<(i64, i64)> = route.vias.iter().map(|via| key(via.at)).collect();
        let in_own_pad = |layer: usize, point: Point| {
            board.obstacles.iter().any(|obstacle| {
                obstacle.kind == ObstacleKind::Copper
                    && obstacle.net == Some(net as u32)
                    && obstacle.layers & (1 << layer) != 0
                    && obstacle.shape.contains(point)
            })
        };
        let fixed = |layer: usize, point: Point| {
            let node = (layer, key(point));
            nodes[&node].len() != 2 || vias.contains(&node.1) || in_own_pad(layer, point)
        };
        let mut used = vec![false; route.segments.len()];
        // Chains start at fixed nodes; what is left are closed loops,
        // which stay as they are.
        for (start_index, segment) in route.segments.iter().enumerate() {
            for start in [segment.start, segment.end] {
                if used[start_index] || !fixed(segment.layer, start) {
                    continue;
                }
                let layer = segment.layer;
                let mut points = vec![start];
                let mut segments = Vec::new();
                let mut current = start_index;
                let mut at = start;
                let mut width = segment.width;
                loop {
                    used[current] = true;
                    let piece = &route.segments[current];
                    segments.push(piece.clone());
                    width = width.min(piece.width);
                    let next = if key(piece.start) == key(at) { piece.end } else { piece.start };
                    points.push(next);
                    at = next;
                    if fixed(layer, next) {
                        break;
                    }
                    let Some(&following) = nodes[&(layer, key(next))].iter().find(|&&other| !used[other]) else {
                        break;
                    };
                    current = following;
                }
                chains.push(Chain { net, layer, width, points, segments });
            }
        }
        for (index, segment) in route.segments.iter().enumerate() {
            if !used[index] {
                chains.push(Chain {
                    net,
                    layer: segment.layer,
                    width: segment.width,
                    points: vec![segment.start, segment.end],
                    segments: vec![segment.clone()],
                });
            }
        }
    }
    chains
}

#[derive(Clone, Debug, Default)]
pub struct TightenReport {
    pub pieces: usize,
    pub tightened: usize,
    /// Pieces that kept their original copper.
    pub kept: usize,
    pub length_before: f64,
    pub length_after: f64,
}

/// `routes` pulled tight where the result stays legal.
pub fn tighten(board: &Board, routes: &[NetRoute]) -> (Vec<NetRoute>, TightenReport) {
    let chains = chains(board, routes);
    let mut report = TightenReport { pieces: chains.len(), ..TightenReport::default() };
    let pieces: Vec<Piece> = chains
        .iter()
        .enumerate()
        .map(|(index, chain)| Piece {
            wire: index,
            net: chain.net as u32,
            class: board.nets[chain.net].class,
            layer: chain.layer,
            width: chain.width,
            points: chain.points.clone(),
        })
        .collect();
    let vias: Vec<PlacedVia> = routes
        .iter()
        .enumerate()
        .flat_map(|(net, route)| {
            route.vias.iter().map(move |via| PlacedVia { wire: usize::MAX, net: net as u32, class: board.nets[net].class, at: via.at })
        })
        .collect();
    let realized = realize::realize(board, &pieces, &vias);
    // Start with every tight piece that was found; fall back to the
    // original copper for pieces the verifier objects to, until clean.
    let mut original: Vec<bool> = realized.paths.iter().map(Option::is_none).collect();
    let assemble = |original: &[bool]| -> (Vec<NetRoute>, Vec<Vec<usize>>) {
        let mut out: Vec<NetRoute> = routes.iter().map(|route| NetRoute { segments: Vec::new(), vias: route.vias.clone() }).collect();
        let mut owners: Vec<Vec<usize>> = vec![Vec::new(); routes.len()];
        for (index, chain) in chains.iter().enumerate() {
            if original[index] {
                for segment in &chain.segments {
                    out[chain.net].segments.push(segment.clone());
                    owners[chain.net].push(index);
                }
            } else {
                let points = realized.paths[index].as_ref().expect("tight path");
                for pair in points.windows(2) {
                    if distance(pair[0], pair[1]) < 1.0e-6 {
                        continue;
                    }
                    out[chain.net].segments.push(Segment { layer: chain.layer, start: pair[0], end: pair[1], width: chain.width });
                    owners[chain.net].push(index);
                }
            }
        }
        (out, owners)
    };
    let result = loop {
        let (out, owners) = assemble(&original);
        let violations = verify(board, &out);
        let mut changed = false;
        for violation in &violations {
            let mut blame = Vec::new();
            if let Some(segment) = violation.segment {
                blame.push(owners[violation.net as usize][segment]);
            }
            if let Some((net, segment)) = violation.other_segment {
                blame.push(owners[net as usize][segment]);
            }
            for index in blame {
                if !original[index] {
                    original[index] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            break out;
        }
    };
    report.kept = original.iter().filter(|&&kept| kept).count();
    report.tightened = report.pieces - report.kept;
    let length = |routes: &[NetRoute]| -> f64 {
        routes.iter().flat_map(|route| &route.segments).map(|segment| distance(segment.start, segment.end)).sum()
    };
    report.length_before = length(routes);
    report.length_after = length(&result);
    (result, report)
}

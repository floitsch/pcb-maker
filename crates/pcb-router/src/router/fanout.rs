// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The fan-out plan (`Router::fanout_plan`): every pad of a dense row (a
//! neighbour of another net closer than `FINE_PITCH`, or at most two
//! straight ways out) and its straight ways out (`ExitRun`: a lattice run
//! from the pad's edge along one of the four axes from the lattice node at
//! the pad's centre, `EXIT_DEPTH` beyond the pad, every node and step legal
//! for the net), with its row (part and outward side) and where its
//! ratsnest leads. The crossing census reads it (router/dive.rs,
//! `PCB_ROUTER_TROUBLE_WHY`).
//!
//! It was the first half of the fan-out step (`fanout`, off; docs/router.md,
//! "Tried and rejected"), which chose one way out per
//! pad jointly with its neighbours, diagonal ways and via sites included,
//! and reserved it against other nets by a cost before the negotiation.
//! Reserving the ways moved which pad of a crossing pair stays open and
//! cost passes elsewhere; the step was removed on 2026-11-04
//! (docs/handover.md, "Tried and not taken").

use super::*;

/// How far beyond its pad a way out reaches, in millimetres (at least a
/// lattice step).
const EXIT_DEPTH: f64 = 0.5;

/// Pads closer than this (centre to centre, another net, the same layer)
/// make a row fine-pitch.
const FINE_PITCH: f64 = 0.65;

/// A part with fewer pads has no row.
const MIN_PART_PADS: usize = 4;

/// One way out of a pad: a straight lattice run from the first node
/// outside the pad to `end`.
#[derive(Clone, Debug)]
pub(super) struct ExitRun {
    pub(super) start: crate::geometry::Point,
    pub(super) end: crate::geometry::Point,
}

/// A dense pad and its ways out.
#[derive(Clone, Debug)]
pub(super) struct DensePad {
    pub(super) net: NetId,
    pub(super) terminal: usize,
    pub(super) layer: usize,
    pub(super) half_width: f64,
    pub(super) clearance: f64,
    /// Its ways out along the four axes that reach `EXIT_DEPTH`.
    pub(super) runs: Vec<ExitRun>,
    /// Its part's outward axis direction (an index of `DIRECTIONS`) and
    /// the part's name, for the rows' order.
    pub(super) row: Option<(String, usize)>,
    /// Where the ratsnest leads (`None` for no other pad).
    pub(super) target: Option<crate::geometry::Point>,
}

/// The dense pads and their ways out (`Router::fanout_plan`), for the
/// crossing census (router/dive.rs) and the tests.
#[derive(Clone, Debug, Default)]
pub(super) struct FanoutPlan {
    pub(super) pads: Vec<DensePad>,
}

impl Router {
    /// The fan-out's plan on the current board: the dense pads and their
    /// straight ways out.
    pub(super) fn fanout_plan(&self) -> FanoutPlan {
        let mut plan = FanoutPlan::default();
        let pitch = self.grid.pitch;
        let depth = EXIT_DEPTH.max(pitch);
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        // Every single-layer terminal's anchor, by layer, for the pitch
        // and the parts' centres.
        struct Anchor {
            net: NetId,
            at: crate::geometry::Point,
            layer: usize,
        }
        let mut anchors: Vec<Anchor> = Vec::new();
        let mut parts: HashMap<&str, ([f64; 2], usize)> = HashMap::new();
        for (net, description) in self.board.nets.iter().enumerate() {
            for terminal in &description.terminals {
                let part = terminal
                    .label
                    .rsplit_once('.')
                    .map_or(terminal.label.as_str(), |(part, _)| part);
                let entry = parts.entry(part).or_insert(([0.0, 0.0], 0));
                entry.0[0] += terminal.anchor[0];
                entry.0[1] += terminal.anchor[1];
                entry.1 += 1;
                if terminal.layers.count_ones() == 1 {
                    anchors.push(Anchor {
                        net: net as NetId,
                        at: terminal.anchor,
                        layer: terminal.layers.trailing_zeros() as usize,
                    });
                }
            }
        }
        // A bucket grid of 1 mm for the nearest neighbour.
        let bucket =
            |point: crate::geometry::Point| (point[0].floor() as i64, point[1].floor() as i64);
        let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (index, anchor) in anchors.iter().enumerate() {
            buckets.entry(bucket(anchor.at)).or_default().push(index);
        }
        // A pad of a coarser row takes part with at most this many
        // straight ways out.
        let coarse_free: usize = 2;
        for (net, description) in self.board.nets.iter().enumerate() {
            let net = net as NetId;
            let state = &self.nets[net as usize];
            // A pour net's pads join their pour by spokes and stubs: no
            // way out of theirs is reserved, and they pay for none
            // (`Router::search`).
            if !state.routable || !state.plane.is_empty() {
                continue;
            }
            let class = description.class;
            for (index, terminal) in description.terminals.iter().enumerate() {
                if terminal.layers.count_ones() != 1 {
                    continue;
                }
                let layer = terminal.layers.trailing_zeros() as usize;
                let (bx, by) = bucket(terminal.anchor);
                let mut nearest = f64::INFINITY;
                for y in by - 1..=by + 1 {
                    for x in bx - 1..=bx + 1 {
                        for other in buckets.get(&(x, y)).into_iter().flatten() {
                            let other = &anchors[*other];
                            if other.net != net && other.layer == layer {
                                nearest = nearest
                                    .min(crate::geometry::distance(other.at, terminal.anchor));
                            }
                        }
                    }
                }
                let shape = &self.board.obstacles[terminal.pad].shape;
                let Some(centre) = self.grid.nearest_node(terminal.anchor) else {
                    continue;
                };
                if !shape.contains(self.grid.center_of(centre)) {
                    // The pad's centre line is off the lattice: its
                    // stubs are oblique, no straight run leaves it.
                    continue;
                }
                let (cx, cy) = self.grid.xy(centre);
                let part = terminal
                    .label
                    .rsplit_once('.')
                    .map_or(terminal.label.as_str(), |(part, _)| part);
                // Rows are a part's: a part of fewer pads (a 0201's two
                // pads closer than the fine pitch) has none.
                if parts
                    .get(part)
                    .is_none_or(|(_, count)| *count < MIN_PART_PADS)
                {
                    continue;
                }
                let part_centre = parts
                    .get(part)
                    .map(|(sum, count)| [sum[0] / *count as f64, sum[1] / *count as f64]);
                // The ratsnest's direction: the nearest pad of the net on
                // another part, else the nearest pad; none for a pour net.
                let target = if state.plane.is_empty() {
                    description
                        .terminals
                        .iter()
                        .enumerate()
                        .filter(|(other, _)| *other != index)
                        .map(|(_, other)| {
                            let other_part = other
                                .label
                                .rsplit_once('.')
                                .map_or(other.label.as_str(), |(part, _)| part);
                            (
                                other_part == part,
                                crate::geometry::distance(other.anchor, terminal.anchor),
                                other.anchor,
                            )
                        })
                        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)))
                        .map(|(_, _, at)| at)
                } else {
                    None
                };
                let mut runs = Vec::new();
                // The axis directions (the even ones of `DIRECTIONS`).
                for (direction, (dx, dy)) in crate::grid::DIRECTIONS.iter().enumerate().step_by(2) {
                    let (dx, dy) = (*dx as i64, *dy as i64);
                    let (mut x, mut y) = (cx as i64, cy as i64);
                    let mut start: Option<crate::geometry::Point> = None;
                    let mut end = None;
                    loop {
                        x += dx;
                        y += dy;
                        if x < 0 || y < 0 || x >= nx || y >= ny {
                            break;
                        }
                        let cell = (y * nx + x) as usize;
                        let point = self.grid.center_of(cell);
                        if start.is_none() && shape.contains(point) {
                            continue;
                        }
                        let node_class = self.node_class(net, cell as u32);
                        if !self.statics[node_class].trace_allowed(layer, cell, net) {
                            break;
                        }
                        // The step into this node (a diagonal one may
                        // pass a corner closer than its nodes do).
                        let previous = ((y - dy) * nx + x - dx) as usize;
                        let (blocked, owner) = self.statics[self.node_class(net, previous as u32)]
                            .edges_at(layer, previous);
                        if blocked & (1 << direction) != 0 && owner != crate::grid::owner16(net) {
                            break;
                        }
                        let first = *start.get_or_insert(point);
                        // Beyond the pad by `depth` (the first node outside
                        // lies within a pitch of its edge).
                        if crate::geometry::distance(first, point) + pitch >= depth - 1.0e-9 {
                            end = Some(point);
                            break;
                        }
                    }
                    let (Some(start), Some(end)) = (start, end) else {
                        continue;
                    };
                    runs.push(ExitRun { start, end });
                }
                // The straight ways along the axes: a pad with at most
                // two is in a row whatever its pitch.
                let free = runs.len();
                // A pad with no straight way out at all is listed (the
                // census names it) where the row is fine-pitch.
                if nearest >= FINE_PITCH && (free == 0 || free > coarse_free) {
                    continue;
                }
                let neck = self.node_class(net, centre as u32);
                plan.pads.push(DensePad {
                    net,
                    terminal: index,
                    layer,
                    half_width: self.width_on(neck, layer) / 2.0,
                    clearance: self.board.classes[class].clearance,
                    row: part_centre.map(|centre| {
                        // The axis direction closest to the way from the
                        // part's centre to the pad.
                        let away = [
                            terminal.anchor[0] - centre[0],
                            terminal.anchor[1] - centre[1],
                        ];
                        let outward = (0..8)
                            .step_by(2)
                            .max_by(|a, b| {
                                let dot = |direction: usize| {
                                    let (dx, dy) = crate::grid::DIRECTIONS[direction];
                                    away[0] * dx as f64 + away[1] * dy as f64
                                };
                                dot(*a).total_cmp(&dot(*b)).then(b.cmp(a))
                            })
                            .unwrap_or(0);
                        (part.to_string(), outward)
                    }),
                    target,
                    runs,
                });
            }
        }
        plan
    }
}

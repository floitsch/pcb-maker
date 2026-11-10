// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Thermal spokes as KiCad draws and counts them.
//!
//! `Router::kicad_spokes` runs the model in stages, each in its own file:
//! `draw` (the items before any spoke, the pads that get spokes and the
//! spokes KiCad draws), `land` (where each spoke lands), `pieces` (the
//! items the kept spokes add, the pieces KiCad's outlines join, pass
//! two's count) and `drc` (pass three: what KiCad's DRC adds per pad and
//! per zone).

use super::*;

mod draw;
mod drc;
mod land;
mod pieces;
#[cfg(test)]
mod tests;

use draw::Drawn;
use drc::{DrcInputs, KeptPieces};
use land::Landed;
use pieces::{PieceItems, PlatedHoles, Touches};

/// The net the spoke model counts, and what every stage reads of it.
struct SpokeNet<'a> {
    board: &'a Board,
    net: NetId,
    pours: &'a crate::pour::PourMap,
    state: &'a NetState,
    description: &'a crate::board::Net,
    /// The branches left out (`kicad_spokes`'s `skip`).
    skip: &'a dyn Fn(usize) -> bool,
    layers: usize,
    pitch: f64,
    /// KiCad's DRC's minimum spoke count (the board setup's).
    required: usize,
    /// Per layer, whether a zone of the net there is solid.
    solid: Vec<bool>,
}

impl<'a> SpokeNet<'a> {
    fn new(
        router: &'a Router,
        net: NetId,
        pours: &'a crate::pour::PourMap,
        skip: &'a dyn Fn(usize) -> bool,
    ) -> Self {
        let layers = router.board.layer_count;
        let solid: Vec<bool> = (0..layers)
            .map(|layer| {
                router
                    .board
                    .planes
                    .iter()
                    .any(|plane| plane.net == net && plane.layer == layer && plane.solid)
            })
            .collect();
        Self {
            board: &router.board,
            net,
            pours,
            state: &router.nets[net as usize],
            description: &router.board.nets[net as usize],
            skip,
            layers,
            pitch: router.grid.pitch,
            required: router.config.min_spokes,
            solid,
        }
    }

    /// Whether `terminal`'s pad connects to no zone (the lowering's
    /// per label).
    fn isolated(&self, terminal: usize) -> bool {
        self.board
            .isolated_pads
            .binary_search(&self.description.terminals[terminal].label)
            .is_ok()
    }

    /// Whether `terminal`'s pad connects to its zones solidly (the
    /// lowering's per label).
    fn solid_pad(&self, terminal: usize) -> bool {
        self.board
            .solid_pads
            .binary_search(&self.description.terminals[terminal].label)
            .is_ok()
    }

    /// The pour map's piece at node `cell` on `layer`; 0 for none.
    fn label_at(&self, layer: usize, cell: usize) -> u32 {
        self.pours.label[layer].get(cell).copied().unwrap_or(0)
    }
}

/// One thermal spoke KiCad draws (`Router::draw_spokes`).
#[derive(Clone, Debug)]
struct Spoke {
    /// Its record (an index into the records) and its slot in the
    /// record's outcomes.
    record: usize,
    index: usize,
    /// Its pad (an index into the board's obstacles) and layer.
    pad: usize,
    layer: usize,
    /// The point it leaves from and its direction (a unit vector).
    anchor: crate::geometry::Point,
    direction: crate::geometry::Point,
    /// The distances along it from the anchor to the pad's edge and to
    /// its tip.
    edge: f64,
    tip: f64,
    /// The zone's thermal gap.
    gap: f64,
    /// Its width, and how far from its axis a brush centre may lie in it.
    width: f64,
    side: f64,
}

impl Spoke {
    /// Whether `point` lies in the spoke's strip, from its anchor to its
    /// tip (a micron's tolerance).
    fn contains(&self, point: crate::geometry::Point) -> bool {
        let offset = [point[0] - self.anchor[0], point[1] - self.anchor[1]];
        let along = offset[0] * self.direction[0] + offset[1] * self.direction[1];
        let across = (offset[0] * self.direction[1] - offset[1] * self.direction[0]).abs();
        along >= -1.0e-6 && along <= self.tip + 1.0e-6 && across <= self.width / 2.0 + 1.0e-6
    }

    /// The spoke's tip, the end KiCad tests for lying in fill.
    fn end(&self) -> crate::geometry::Point {
        [
            self.anchor[0] + self.direction[0] * self.tip,
            self.anchor[1] + self.direction[1] * self.tip,
        ]
    }
}

/// Joins the sets of `a` and `b` in the union-find `parent`.
fn union(parent: &mut [usize], a: usize, b: usize) {
    let (a, b) = (find(parent, a), find(parent, b));
    parent[a] = b;
}

impl Router {
    /// The pads' thermal spokes as KiCad makes and counts them (the
    /// `kicad_spokes` model), joining each pad to the pieces its counted
    /// spokes reach. Returns the spoke islands (piece, terminal) of pads
    /// short of two spokes.
    ///
    /// KiCad (zone_filler.cpp, buildThermalSpokes and fillCopperZone)
    /// draws a spoke from the pad's centre to its box grown by the thermal
    /// gap and half the zone's minimum width, and keeps it when that end
    /// lies in the fill (the fill before spokes, opened by the minimum
    /// width); the spoke is as wide as the zone's spoke width but no wider
    /// than the pad, and none is drawn when that is narrower than the
    /// minimum width. Other nets' clearances then trim it, and the
    /// opening removes what is left narrower than the minimum width. The
    /// DRC (drc_test_provider_zone_connections.cpp) counts a spoke where
    /// the fill crosses the ring half a gap around the pad, ignores those
    /// into a fill piece connected to nothing but the pad, and, while a
    /// pad is short, counts a track that leaves the pad and ends in fill.
    /// A piece reached by two pads' spokes connects them (KiCad's
    /// connectivity joins every item a fill piece touches); a piece that
    /// only the one pad touches is the "isolated island" of its report.
    ///
    /// On the lattice a spoke lands where a brush centre (a free node)
    /// lies within half the minimum width and half a pitch of its end, and
    /// it is blocked where, along it from the middle of the gap to its end,
    /// no node across its width leaves the brush room.
    // The signature is the caller's (`analyze_pours_full`): `parent` stays
    // a `Vec` though the stages take a slice of it.
    #[allow(clippy::too_many_arguments, clippy::ptr_arg)]
    pub(super) fn kicad_spokes(
        &self,
        net: NetId,
        pours: &crate::pour::PourMap,
        parent: &mut Vec<usize>,
        terminal_base: usize,
        skip: &dyn Fn(usize) -> bool,
        rings: &[Vec<(usize, f64)>],
        branch_pieces: &[Vec<(u32, usize)>],
        report: Option<&mut Vec<SpokeRecord>>,
    ) -> Vec<(usize, usize)> {
        let model = SpokeNet::new(self, net, pours, skip);
        // Items per piece (KiCad's CN_ZONE_LAYER::HasSingleConnection):
        // each branch with a node in it, each pad joined solidly, and each
        // pad whose spoke lands in it.
        let mut items = self.piece_items(&model, branch_pieces);
        let probe = fill::FillProbe::new(self, net, rings);
        // Per layer, the net's zones KiCad fills there when there are
        // several.
        let zones_on = self.zones_by_layer(&model);
        // Pass one: the spokes' geometry, then where each lands (Good holds
        // the piece until the items are counted).
        let spoke_pads = self.spoke_pads(&model);
        let Drawn {
            mut records,
            spokes,
            inner: record_inner,
        } = self.draw_spokes(&model, &probe, spoke_pads);
        let Landed { met_pads, attached } = self.land_spokes(&model, &probe, &mut records, &spokes);
        // The pads' drill holes: KiCad's fill keeps out of a plated hole,
        // so a spoke reaches into a through-hole pad only up to its hole.
        let mut record_attached = vec![[true; 4]; records.len()];
        for (position, spoke) in spokes.iter().enumerate() {
            record_attached[spoke.record][spoke.index] = attached[position];
        }
        let holes = PlatedHoles::of(&self.board.obstacles);
        let Touches {
            touched: touched_spokes,
            pads: pad_items,
            joins: spoke_joins,
        } = self.touch_pieces(
            &model,
            &records,
            &spokes,
            &record_attached,
            &holes,
            &mut items,
        );
        let mut pieces = PieceItems::new(items, pad_items);
        self.join_necks(
            &model,
            &probe,
            parent,
            &records,
            &spokes,
            &mut pieces,
            spoke_joins,
        );
        // Pass two: spokes into pieces with another item count and join.
        let tallies = pieces::tally_spokes(
            parent,
            terminal_base,
            &mut records,
            &touched_spokes,
            &record_attached,
            &record_inner,
            &pieces,
        );
        // Pass three, with every join made: the DRC's track spokes.
        let roots = drc::kept_roots(&model, parent, terminal_base, &records, branch_pieces);
        let inputs = DrcInputs {
            spokes: &spokes,
            outcomes: records.iter().map(|record| record.outcomes).collect(),
            pads: records
                .iter()
                .map(|record| (record.terminal, record.pad, record.anchor))
                .collect(),
            inner: &record_inner,
            met_pads: &met_pads,
            zones_on: &zones_on,
            thermal_holes: self.thermal_holes(&model, rings),
            kept: KeptPieces::new(roots),
            terminal_base,
        };
        let spoke_islands = self.count_drc(&model, &probe, &inputs, parent, &mut records, tallies);
        if let Some(report) = report {
            report.extend(records);
        }
        probe.charge();
        spoke_islands
    }

    /// The piece of the pour map on `layer` whose labelled node lies
    /// nearest `point` within `radius`, and that node's distance.
    fn nearest_piece(
        &self,
        model: &SpokeNet,
        layer: usize,
        point: crate::geometry::Point,
        radius: f64,
    ) -> Option<(f64, u32)> {
        let mut best: Option<(f64, u32)> = None;
        if let Some((x0, y0, x1, y1)) = self.grid.node_range(
            crate::geometry::Aabb {
                minimum: point,
                maximum: point,
            }
            .inflated(radius),
        ) {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let piece = model.label_at(layer, self.grid.index(x, y));
                    let distance = crate::geometry::distance(point, self.grid.center(x, y));
                    if piece != 0
                        && distance <= radius
                        && best.is_none_or(|(best, _)| distance < best)
                    {
                        best = Some((distance, piece));
                    }
                }
            }
        }
        best
    }

    /// Per branch of `net` (none for those `skip` names), the pieces of
    /// `pours` its copper touches: those its nodes lie in and those with a
    /// brush centre within half its width
    /// (a via's radius) and half the minimum width of a node, as KiCad's
    /// connectivity joins a track to every fill outline its copper overlaps
    /// (Castor's U6.5: its exit track and via run along a piece through
    /// nodes the neighbours' clearances keep from the fill, and the piece
    /// held them as two items).
    pub(super) fn branch_pieces(
        &self,
        net: NetId,
        pours: &crate::pour::PourMap,
        skip: &dyn Fn(usize) -> bool,
    ) -> Vec<Vec<(u32, usize)>> {
        let state = &self.nets[net as usize];
        let pitch = self.grid.pitch;
        let class = self.board.classes[self.board.nets[net as usize].class];
        state
            .branches
            .iter()
            .enumerate()
            .map(|(index, branch)| {
                // Per piece, the straight runs (KiCad's track segments, each
                // an item of the fill outline it touches) that touch it.
                let mut seen: Vec<(u32, usize)> = Vec::new();
                if skip(index) {
                    return seen;
                }
                let mut run = 0usize;
                let mut last: Option<(i64, i64, u8)> = None;
                let add = |seen: &mut Vec<(u32, usize)>,
                           piece: u32,
                           run: usize,
                           marks: &mut Vec<(u32, usize)>| {
                    if marks.contains(&(piece, run)) {
                        return;
                    }
                    marks.push((piece, run));
                    match seen.iter_mut().find(|(seen, _)| *seen == piece) {
                        Some(entry) => entry.1 += 1,
                        None => seen.push((piece, 1)),
                    }
                };
                let mut marks: Vec<(u32, usize)> = Vec::new();
                for (position, node) in branch.nodes.iter().enumerate() {
                    if position > 0 {
                        let previous = branch.nodes[position - 1];
                        let (ax, ay) = self.grid.xy(previous.cell as usize);
                        let (bx, by) = self.grid.xy(node.cell as usize);
                        let step = (bx as i64 - ax as i64, by as i64 - ay as i64, node.layer);
                        if previous.layer != node.layer || last.is_some_and(|last| last != step) {
                            run += 1;
                        }
                        last = (previous.layer == node.layer).then_some(step);
                    }
                    let layer = node.layer as usize;
                    let labels = &pours.label[layer];
                    if labels.is_empty() {
                        continue;
                    }
                    let piece = labels[node.cell as usize];
                    if piece != 0 {
                        add(&mut seen, piece, run, &mut marks);
                        continue;
                    }
                    let via = [position.wrapping_sub(1), position + 1]
                        .iter()
                        .filter_map(|other| branch.nodes.get(*other))
                        .any(|other| other.cell == node.cell && other.layer != node.layer);
                    let copper = if via {
                        class.via_diameter / 2.0
                    } else if let Some((_, _, radius)) = state.adopted.get(&index) {
                        *radius
                    } else {
                        self.width_at(net, node.cell, node.layer as usize) / 2.0
                    };
                    let reach =
                        copper + self.board.classes[state.plane_class[layer]].trace_width / 2.0;
                    let cells = (reach / pitch).floor() as i64;
                    let (x, y) = self.grid.xy(node.cell as usize);
                    if self.grid.warp.is_some() {
                        // Lines at the pins: the nodes within the reach,
                        // measured (`disc_cells` leaves the radius out).
                        self.grid
                            .disc_cells(x, y, (reach * 1.0e6).round() as i64 + 1, |cell| {
                                let piece = labels[cell];
                                if piece != 0 {
                                    add(&mut seen, piece, run, &mut marks);
                                }
                            });
                        continue;
                    }
                    for dy in -cells..=cells {
                        for dx in -cells..=cells {
                            if ((dx * dx + dy * dy) as f64) * pitch * pitch > reach * reach {
                                continue;
                            }
                            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                            if nx < 0
                                || ny < 0
                                || nx >= self.grid.nx as i64
                                || ny >= self.grid.ny as i64
                            {
                                continue;
                            }
                            let piece = labels[self.grid.index(nx as usize, ny as usize)];
                            if piece != 0 {
                                add(&mut seen, piece, run, &mut marks);
                            }
                        }
                    }
                }
                seen
            })
            .collect()
    }

    /// A pad's four thermal spokes as KiCad builds them (`PadSpokes` where
    /// the lowering gave the pad's frame; else along the axes of its box,
    /// diagonally for a circle): whether they run diagonally, the point
    /// they leave from, per spoke (in the order of `SpokeRecord::outcomes`:
    /// +x, -x, +y, -y, or +x+y, +x-y, -x+y, -x-y; a turned pad's in the
    /// nearest of those) its direction, the distance from that point to
    /// the pad's edge along it and to the edge grown by `reach`; and the
    /// pad's smaller side (no spoke is wider). 0xCB's 1337 has its parts
    /// at 45 degrees, their spokes too.
    pub(super) fn spoke_geometry(
        &self,
        pad: usize,
        anchor: crate::geometry::Point,
        reach: f64,
    ) -> (
        bool,
        crate::geometry::Point,
        [(crate::geometry::Point, f64, f64); 4],
        f64,
    ) {
        self.spoke_geometry_in(pad, anchor, reach, true)
    }

    /// `spoke_geometry`, in the pad's own frame only with `frames` (without,
    /// along the axes of its box: the thermal guards).
    pub(super) fn spoke_geometry_in(
        &self,
        pad: usize,
        anchor: crate::geometry::Point,
        reach: f64,
        frames: bool,
    ) -> (
        bool,
        crate::geometry::Point,
        [(crate::geometry::Point, f64, f64); 4],
        f64,
    ) {
        let d = std::f64::consts::FRAC_1_SQRT_2;
        let diagonals = [[d, d], [d, -d], [-d, d], [-d, -d]];
        let axes = [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]];
        if let Some(frame) = self.spoke_frame(pad).filter(|_| frames) {
            let spokes = frame.spokes(reach);
            let first = spokes[0].0;
            let round = first[0].abs() > 0.3 && first[1].abs() > 0.3;
            let canonical = if round { diagonals } else { axes };
            let mut taken = [false; 4];
            let mut result = [([0.0, 0.0], 0.0, 0.0); 4];
            for (slot, direction) in canonical.iter().enumerate() {
                let best = (0..4)
                    .filter(|index| !taken[*index])
                    .max_by(|a, b| {
                        let dot = |index: usize| {
                            spokes[index].0[0] * direction[0] + spokes[index].0[1] * direction[1]
                        };
                        dot(*a).total_cmp(&dot(*b)).then(b.cmp(a))
                    })
                    .unwrap();
                taken[best] = true;
                result[slot] = spokes[best];
            }
            return (
                round,
                frame.center,
                result,
                2.0 * frame.half[0].min(frame.half[1]),
            );
        }
        let shape = &self.board.obstacles[pad].shape;
        let bounds = shape.aabb();
        let round = matches!(shape, crate::geometry::Shape::Circle { .. });
        let half = [
            (bounds.maximum[0] - bounds.minimum[0]) / 2.0,
            (bounds.maximum[1] - bounds.minimum[1]) / 2.0,
        ];
        let directions = if round { diagonals } else { axes };
        let mut result = [([0.0, 0.0], 0.0, 0.0); 4];
        for (slot, direction) in directions.into_iter().enumerate() {
            let edge = if round {
                half[0]
            } else {
                (half[0] * direction[0]).abs() + (half[1] * direction[1]).abs()
            };
            result[slot] = (direction, edge, edge + reach);
        }
        (round, anchor, result, 2.0 * half[0].min(half[1]))
    }

    /// The copper of `branch` leaving terminal `terminal` (one of its
    /// ends) on its first layer: the stub from the pad's anchor to the
    /// lattice (an escape's polyline) and the first straight run, each
    /// segment with half its width. Empty when the branch does not end at
    /// the terminal.
    pub(super) fn first_run(
        &self,
        net: NetId,
        terminal: usize,
        branch: &Branch,
    ) -> Vec<(crate::geometry::Point, crate::geometry::Point, f64)> {
        let state = &self.nets[net as usize];
        let anchor = self.board.nets[net as usize].terminals[terminal].anchor;
        let mut segments = Vec::new();
        for (end, forward) in [(branch.start_terminal, true), (branch.end_terminal, false)] {
            if end as usize != terminal || branch.nodes.is_empty() {
                continue;
            }
            let nodes: Vec<Node> = if forward {
                branch.nodes.clone()
            } else {
                branch.nodes.iter().rev().copied().collect()
            };
            let first = self.grid.center_of(nodes[0].cell as usize);
            let half = self.width_at(net, nodes[0].cell, nodes[0].layer as usize) / 2.0;
            match state.escapes.get(&nodes[0]) {
                Some((_, width, polyline)) => segments.extend(
                    polyline
                        .windows(2)
                        .map(|pair| (pair[0], pair[1], width / 2.0)),
                ),
                None if crate::geometry::distance(anchor, first) > 1.0e-7 => {
                    segments.push((anchor, first, half))
                }
                None => {}
            }
            let step = |a: Node, b: Node| {
                let (ax, ay) = self.grid.xy(a.cell as usize);
                let (bx, by) = self.grid.xy(b.cell as usize);
                (a.layer == b.layer).then_some((bx as i64 - ax as i64, by as i64 - ay as i64))
            };
            if nodes.len() >= 2
                && let Some(direction) = step(nodes[0], nodes[1])
            {
                let mut last = 1;
                while last + 1 < nodes.len()
                    && step(nodes[last], nodes[last + 1]) == Some(direction)
                {
                    last += 1;
                }
                segments.push((first, self.grid.center_of(nodes[last].cell as usize), half));
            }
        }
        segments
    }

    /// Whether lattice piece `piece` of the fill on `layer` (`labels`, the
    /// pour map's) is one outline of KiCad's fill with a piece `kept`
    /// accepts, joined by a neck the lattice misses: a passage whose brush
    /// centres lie between the nodes. KiCad's fill keeps a passage as wide
    /// as the zone's minimum width, whose centres may lie on one line a few
    /// microns wide (video's +5V on In2.Cu: a 0.26 mm strip between the
    /// zone's edge and a via's clearance, its centres 10 microns from the
    /// nearest node; the 50 mm² piece beyond it holds a via, and a track of
    /// U21.7 ending over it is a spoke to KiCad's DRC). From the pieces
    /// reached so far (a few at most), for each other piece with a node
    /// within `NECK_REACH` of one of theirs, the closest pairs of nodes are
    /// tried: a flood of brush centres (`FillProbe::fits`) on a grid of a
    /// twelfth of the pitch, 8-connected, in their box grown by one and a
    /// half pitches. `kept` gets the other piece and a point of it. No work
    /// is charged beyond the probe's own: it runs only for track ends over
    /// a piece holding nothing.
    pub(super) fn neck_joined(
        &self,
        probe: &fill::FillProbe,
        labels: &[u32],
        layer: usize,
        piece: u32,
        kept: &mut dyn FnMut(u32, crate::geometry::Point) -> bool,
    ) -> bool {
        const NECK_REACH_PITCHES: i64 = 10;
        const PAIRS_PER_PIECE: usize = 4;
        const MAX_PIECES: usize = 6;
        // A pair of nodes: their squared distance in pitches, this
        // piece's node, the other's.
        type Pair = (i64, (i64, i64), (i64, i64));
        let (nx, ny) = (self.grid.nx as i64, self.grid.ny as i64);
        let label = |x: i64, y: i64| {
            if x < 0 || y < 0 || x >= nx || y >= ny {
                0
            } else {
                labels[(y * nx + x) as usize]
            }
        };
        let mut reached = vec![piece];
        let mut queue = vec![piece];
        let mut tried: HashSet<u32> = HashSet::new();
        while let Some(current) = queue.pop() {
            // Its rim: nodes with a side neighbour outside it.
            let rim: Vec<(i64, i64)> = labels
                .iter()
                .enumerate()
                .filter(|(_, label)| **label == current)
                .map(|(index, _)| ((index as i64) % nx, (index as i64) / nx))
                .filter(|(x, y)| {
                    [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|(dx, dy)| label(x + dx, y + dy) != current)
                })
                .collect();
            let mut pairs: HashMap<u32, Vec<Pair>> = HashMap::new();
            for &(x, y) in &rim {
                for dy in -NECK_REACH_PITCHES..=NECK_REACH_PITCHES {
                    for dx in -NECK_REACH_PITCHES..=NECK_REACH_PITCHES {
                        let distance = dx * dx + dy * dy;
                        if distance > NECK_REACH_PITCHES * NECK_REACH_PITCHES {
                            continue;
                        }
                        let other = label(x + dx, y + dy);
                        if other == 0 || reached.contains(&other) || tried.contains(&other) {
                            continue;
                        }
                        let entry = pairs.entry(other).or_default();
                        entry.push((distance, (x, y), (x + dx, y + dy)));
                        if entry.len() > 4 * PAIRS_PER_PIECE {
                            entry.sort_unstable();
                            entry.truncate(PAIRS_PER_PIECE);
                        }
                    }
                }
            }
            let mut others: Vec<(u32, Vec<Pair>)> = pairs.into_iter().collect();
            others.sort_unstable_by_key(|(other, candidates)| {
                (candidates.iter().map(|pair| pair.0).min(), *other)
            });
            for (other, mut candidates) in others {
                candidates.sort_unstable();
                candidates.truncate(PAIRS_PER_PIECE);
                tried.insert(other);
                let passage = candidates
                    .iter()
                    .any(|(_, from, to)| self.brush_passage(probe, layer, *from, *to));
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                    eprintln!(
                        "neck layer {layer}: piece {current} to {other} ({:?}): {passage}",
                        candidates
                            .iter()
                            .map(|(_, from, to)| (
                                self.grid.center(from.0 as usize, from.1 as usize),
                                self.grid.center(to.0 as usize, to.1 as usize)
                            ))
                            .collect::<Vec<_>>()
                    );
                }
                if !passage {
                    continue;
                }
                let (x, y) = candidates[0].2;
                if kept(other, self.grid.center(x as usize, y as usize)) {
                    return true;
                }
                if reached.len() < MAX_PIECES {
                    reached.push(other);
                    queue.push(other);
                }
            }
        }
        false
    }

    /// Whether brush centres of the fill join lattice node `from` to node
    /// `to` (`neck_joined`): a flood of centres from `from` reaches one
    /// whose brush overlaps `to`'s, less than the minimum width from it.
    /// KiCad's pruning deflates the fill by half the minimum width and
    /// inflates it again, so two sets of centres closer than that are one
    /// outline (and a centre that close to the net's own track touches
    /// its copper: video's U21.8, a strip of fill beside its track).
    fn brush_passage(
        &self,
        probe: &fill::FillProbe,
        layer: usize,
        from: (i64, i64),
        to: (i64, i64),
    ) -> bool {
        const STEPS: i64 = 12;
        let pitch = self.grid.pitch;
        let step = pitch / STEPS as f64;
        let origin = self.grid.center(from.0 as usize, from.1 as usize);
        let target = self.grid.center(to.0 as usize, to.1 as usize);
        let overlap = 2.0 * probe.brush_half(layer) - 1.0e-6;
        let margin = STEPS + STEPS / 2;
        let (x0, x1) = (
            (to.0 - from.0).min(0) * STEPS - margin,
            (to.0 - from.0).max(0) * STEPS + margin,
        );
        let (y0, y1) = (
            (to.1 - from.1).min(0) * STEPS - margin,
            (to.1 - from.1).max(0) * STEPS + margin,
        );
        let width = (x1 - x0 + 1) as usize;
        let mut state = vec![false; width * (y1 - y0 + 1) as usize];
        let slot = |x: i64, y: i64| ((y - y0) as usize) * width + (x - x0) as usize;
        let mut stack = vec![(0i64, 0i64)];
        state[slot(0, 0)] = true;
        while let Some((x, y)) = stack.pop() {
            let point = [origin[0] + x as f64 * step, origin[1] + y as f64 * step];
            if crate::geometry::distance(point, target) < overlap {
                return true;
            }
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (px, py) = (x + dx, y + dy);
                    if (dx, dy) == (0, 0) || px < x0 || px > x1 || py < y0 || py > y1 {
                        continue;
                    }
                    let index = slot(px, py);
                    if state[index] {
                        continue;
                    }
                    state[index] = true;
                    let point = [origin[0] + px as f64 * step, origin[1] + py as f64 * step];
                    if probe.fits(layer, point, &[]) {
                        stack.push((px, py));
                    }
                }
            }
        }
        false
    }

    /// The frame of a pad's thermal spokes as KiCad builds them, where the
    /// lowering gave one: a custom pad's only for the measurement
    /// (`Config::measured_thermals`).
    pub(super) fn spoke_frame(&self, pad: usize) -> Option<&crate::board::PadSpokes> {
        self.board
            .pad_spokes
            .get(&pad)
            .filter(|frame| !frame.custom || self.config.measured_thermals)
    }

    /// Whether a pad merged into its terminal's larger pad (an exposed
    /// pad's thermal via, not a padstack's other shape) connects to the
    /// zone by a thermal relief. KiCad resolves the zone connection per pad
    /// (`DRC_ENGINE::EvalZoneConnection`: the pad's own, else its
    /// footprint's, else the zone's), and its DRC checks the spokes of a
    /// thermal pad only: a via set to solid (`(zone_connect 2)`) is joined
    /// by the fill wherever it touches, one set to none not at all, and
    /// neither is a starved thermal however few spokes its neighbours
    /// leave (glasgow's U1.57: the exposed pad on F.Cu thermal, its 25
    /// vias solid, no finding on In1.Cu where the model counted one spoke).
    /// The pad's own setting comes from the lowering's per-pad lists
    /// (`Board::solid_pad_obstacles`, `isolated_pad_obstacles`); a board
    /// without them (not lowered from KiCad) keeps the label's isolation.
    pub(super) fn merged_pad_thermal(&self, merged: &crate::board::MergedPad) -> bool {
        let board = &self.board;
        if board.solid_pad_obstacles.is_empty() && board.isolated_pad_obstacles.is_empty() {
            return board.nets[merged.net as usize]
                .terminals
                .get(merged.host)
                .is_some_and(|host| board.isolated_pads.binary_search(&host.label).is_err());
        }
        board
            .solid_pad_obstacles
            .binary_search(&merged.pad)
            .is_err()
            && board
                .isolated_pad_obstacles
                .binary_search(&merged.pad)
                .is_err()
    }

    /// Whether the spokes of a pad on a layer leave it a starved thermal to
    /// KiCad's DRC, and how many more counted spokes it needs: none counted
    /// and none into islands is no thermal connection (not starved); with
    /// tracks counted, the islands no longer stand in for spokes.
    pub(super) fn spokes_short(record: &SpokeRecord) -> usize {
        let good = record
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, SpokeOutcome::Good(_) | SpokeOutcome::Pad(_)))
            .count();
        let islands = record
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, SpokeOutcome::Island(_)))
            .count();
        if record.required == 0 || (good == 0 && islands == 0 && record.crossings == 0) {
            return 0;
        }
        let counted = good + record.crossings + record.tracks;
        if counted == 0 && islands >= record.required {
            return record.seam_short;
        }
        record
            .required
            .saturating_sub(counted)
            .max(record.seam_short)
    }

    /// Whether the model finds a pad on a layer with no spoke at all (none
    /// counted, none into an island, no other pad's across its ring) and
    /// no track of its own into the fill: KiCad's DRC sees no thermal
    /// connection there unless it finds a spoke the model does not
    /// (`Config::spokeless_tracks`).
    pub(super) fn spokeless(record: &SpokeRecord) -> bool {
        record.drawn
            && record.required > 0
            && record.crossings == 0
            && record.tracks == 0
            && record.seam_short == 0
            && record
                .outcomes
                .iter()
                .all(|outcome| matches!(outcome, SpokeOutcome::NoFill | SpokeOutcome::Blocked))
    }

    /// Whether `point` lies in the part of zone `zone` (an index into the
    /// board's planes) KiCad fills: its outline less those of the net's
    /// zones of a higher priority there.
    pub(super) fn in_zone_part(&self, zone: usize, point: crate::geometry::Point) -> bool {
        let plane = &self.board.planes[zone];
        crate::geometry::point_in_polygon(point, &plane.polygon)
            && !plane
                .above
                .iter()
                .any(|above| crate::geometry::point_in_polygon(point, above))
    }
}

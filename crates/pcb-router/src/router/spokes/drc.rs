// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Pass three, with every join made: what KiCad's DRC adds to a pad's own
//! spokes (other pads' spokes across its ring, its tracks ending in fill)
//! and the count per zone where several zones fill the layer.

use super::*;

/// What the zones' island removal keeps of the net's pieces
/// (`Router::kept_piece`): the roots of the clusters that hold something,
/// and the caches of the questions asked so far.
pub(super) struct KeptPieces {
    /// The roots (in the union-find) of the pieces kept for what they
    /// hold (`kept_roots`).
    roots: HashSet<usize>,
    /// Per layer and piece, its nodes, for the layers counted so far.
    piece_nodes: std::cell::RefCell<HashMap<(usize, u32), usize>>,
    counted_layers: std::cell::RefCell<Vec<usize>>,
    /// Per layer and piece holding nothing, whether a neck joins it to a
    /// kept piece (`neck_joined`).
    neck_kept: std::cell::RefCell<HashMap<(usize, u32), bool>>,
}

impl KeptPieces {
    /// The pieces kept for the clusters of `roots`, nothing asked yet.
    pub(super) fn new(roots: HashSet<usize>) -> Self {
        Self {
            roots,
            piece_nodes: Default::default(),
            counted_layers: Default::default(),
            neck_kept: Default::default(),
        }
    }
}

/// What pass three reads of the earlier passes (`Router::count_drc`).
pub(super) struct DrcInputs<'a> {
    pub(super) spokes: &'a [Spoke],
    /// Per record, its outcomes after pass two.
    pub(super) outcomes: Vec<[SpokeOutcome; 4]>,
    /// Per record, its terminal, pad and anchor.
    pub(super) pads: Vec<(usize, usize, crate::geometry::Point)>,
    /// Per record, whether its pad lies inside its terminal's pad.
    pub(super) inner: &'a [bool],
    /// Per record, the pads whose spokes met its own.
    pub(super) met_pads: &'a [Vec<usize>],
    /// Per layer, the net's zones when there are several (`zones_by_layer`).
    pub(super) zones_on: &'a [Vec<usize>],
    /// Per layer, the plated holes of the net's thermal pads there.
    pub(super) thermal_holes: Vec<Vec<usize>>,
    pub(super) kept: KeptPieces,
    pub(super) terminal_base: usize,
}

/// The roots of the clusters whose pieces KiCad keeps for what they hold
/// (pass three, with every join made): the terminals', the pieces pads
/// spoke into as islands, and those holding a track or via of the net.
pub(super) fn kept_roots(
    model: &SpokeNet,
    parent: &mut [usize],
    terminal_base: usize,
    records: &[SpokeRecord],
    branch_pieces: &[Vec<(u32, usize)>],
) -> HashSet<usize> {
    let SpokeNet { state, skip, .. } = *model;
    // KiCad removes a fill piece no pad's cluster reaches (an orphan)
    // before the DRC looks, so a track ending there counts for nothing
    // (Castor's RV10.1: its F.Cu track ends over an orphan of B.Cu).
    // KiCad's clusters join a piece to the pads whose spokes reach it
    // whether the DRC counts them or not (BAGEL's J102.1: its In1 track
    // ends over a piece of In2 that pads spoke into as islands).
    let mut terminal_roots: HashSet<usize> = (0..state.terminal_nodes.len())
        .map(|terminal| find(parent, terminal_base + terminal))
        .collect();
    for record in records {
        for outcome in &record.outcomes {
            if let SpokeOutcome::Island(piece) = outcome
                && *piece != 0
            {
                terminal_roots.insert(find(parent, *piece as usize));
            }
        }
    }
    // A piece holding a track or via of the net is kept too: KiCad's
    // island removal deletes only pieces with no connection at all
    // (anthracite's J107.4 on In2.Cu: its tracks end, on other layers,
    // over a piece of In2's fill that holds a via of the net and no
    // pad; KiCad counts them).
    let copper_pieces: Vec<u32> = branch_pieces
        .iter()
        .enumerate()
        .filter(|(index, _)| !skip(*index))
        .flat_map(|(_, pieces)| pieces.iter().map(|(piece, _)| *piece))
        .collect();
    for piece in copper_pieces {
        terminal_roots.insert(find(parent, piece as usize));
    }
    terminal_roots
}

/// KiCad's rule per zone of the layer (`zones`, with `zone_tallies` per
/// zone: the pad's spokes, its island spokes, other pads' crossings and
/// its tracks there): sets `record`'s seam shortfall and zone to the
/// zone it is most short in, where that is more than the record holds.
pub(super) fn record_seam(record: &mut SpokeRecord, zones: &[usize], zone_tallies: &[[usize; 4]]) {
    // KiCad's rule per zone: none of its fill across the ring
    // is no connection to it; tracks count with the spokes, and
    // its island spokes only when nothing else does.
    for (position, [spokes, islands, crossings, tracks]) in zone_tallies.iter().copied().enumerate()
    {
        if spokes + islands + crossings == 0 {
            continue;
        }
        let mut counted = spokes + crossings + tracks;
        if counted == 0 {
            counted = islands;
        }
        let short = record.required.saturating_sub(counted);
        if short > record.seam_short {
            record.seam_short = short;
            record.seam_zone = Some(zones[position]);
        }
    }
}

impl Router {
    /// Per layer, the plated holes (indexes into the board's obstacles)
    /// of the net's thermal pads there (those `rings` holds).
    pub(super) fn thermal_holes(
        &self,
        model: &SpokeNet,
        rings: &[Vec<(usize, f64)>],
    ) -> Vec<Vec<usize>> {
        let SpokeNet { net, layers, .. } = *model;
        (0..layers)
            .map(|layer| {
                self.board
                    .obstacles
                    .iter()
                    .enumerate()
                    .filter(|(_, obstacle)| {
                        obstacle.kind == crate::board::ObstacleKind::Hole
                            && obstacle.net == Some(net)
                            && obstacle.label.ends_with(" plated hole")
                    })
                    .filter(|(_, obstacle)| {
                        let bounds = obstacle.shape.aabb();
                        let centre = [
                            (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                            (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
                        ];
                        rings[layer]
                            .iter()
                            .any(|(pad, _)| self.board.obstacles[*pad].shape.contains(centre))
                    })
                    .map(|(index, _)| index)
                    .collect()
            })
            .collect()
    }

    /// Pass three: per record (with its pass-two tally, the spokes that
    /// count and its island pieces), adds the other pads' spokes across
    /// its ring and, where it is short, its tracks ending in fill, and
    /// the shortfall per zone. Returns the spoke islands (piece, terminal)
    /// of the pads still short.
    pub(super) fn count_drc(
        &self,
        model: &SpokeNet,
        probe: &fill::FillProbe,
        inputs: &DrcInputs,
        parent: &mut [usize],
        records: &mut [SpokeRecord],
        tallies: Vec<(usize, Vec<usize>)>,
    ) -> Vec<(usize, usize)> {
        let description = model.description;
        let mut spoke_islands: Vec<(usize, usize)> = Vec::new();
        for (record_index, (record, (good, islands))) in records.iter_mut().zip(tallies).enumerate()
        {
            // Where other pads' spokes cross this pad's ring (for the count
            // per zone below).
            let crossing_points =
                self.ring_crossings(model, probe, inputs, parent, record_index, record);
            let good = good + record.crossings;
            // Per zone of the layer: the pad's spokes whose
            // crossing of its ring half a gap out lies in the zone's part
            // (a spoke the seam splits lengthwise crosses in both), the
            // island spokes so, the other pads' spokes crossing there and,
            // below, the tracks ending in its fill.
            let zones = &inputs.zones_on[record.layer];
            let mut zone_tallies =
                self.zone_tallies(inputs.spokes, record_index, record, zones, &crossing_points);
            let zone_needs_tracks = zone_tallies.iter().any(|[spokes, islands, crossings, _]| {
                spokes + islands + crossings > 0 && spokes + crossings < record.required
            });
            let track_ends = if good < record.required || zone_needs_tracks {
                self.track_spokes(model, probe, inputs, parent, record)
            } else {
                Vec::new()
            };
            if !zones.is_empty() {
                for end in &track_ends {
                    for (position, zone) in zones.iter().enumerate() {
                        if self.in_zone_part(*zone, *end) {
                            zone_tallies[position][3] += 1;
                        }
                    }
                }
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                    eprintln!(
                        "zones {} layer {}: {:?} tallies (spokes, islands, crossings, tracks) {:?}",
                        description.terminals[record.terminal].label,
                        record.layer,
                        zones,
                        zone_tallies
                    );
                }
                record_seam(record, zones, &zone_tallies);
            }
            if good + record.tracks < record.required {
                for piece in islands {
                    if !spoke_islands.iter().any(|(island, _)| *island == piece) {
                        spoke_islands.push((piece, record.terminal));
                    }
                }
            }
        }
        spoke_islands
    }

    /// Counts in `record.crossings` the other pads' counted spokes that
    /// cross the record's ring half a gap out (joining in `parent` the
    /// pads whose strip runs over its copper), and returns where each
    /// crosses (twice for a strip right through the ring).
    fn ring_crossings(
        &self,
        model: &SpokeNet,
        probe: &fill::FillProbe,
        inputs: &DrcInputs,
        parent: &mut [usize],
        record_index: usize,
        record: &mut SpokeRecord,
    ) -> Vec<crate::geometry::Point> {
        let SpokeNet { net, state, .. } = *model;
        let terminal_base = inputs.terminal_base;
        let mut crossing_points: Vec<crate::geometry::Point> = Vec::new();
        // Other pads' spokes across this pad's ring half a gap out: the
        // DRC counts every crossing of the fill with the ring, and KiCad
        // adds spokes after cutting the reliefs (Castor's U6.11 between
        // U6.12's spoke and its own in a 0.65 mm row; Hub's J3.S1, which
        // a round neighbour's diagonal spoke crosses). Spokes meeting
        // this pad's own are counted with it already.
        let gap = self
            .board
            .planes
            .iter()
            .filter(|plane| plane.net == net && plane.layer == record.layer)
            .map(|plane| plane.thermal_gap)
            .fold(0.0f64, f64::max);
        let pad = &self.board.obstacles[record.pad];
        let minimum = self.board.classes[state.plane_class[record.layer]].trace_width;
        let met = &inputs.met_pads[record_index];
        let bounds = pad.shape.aabb();
        let radius = 0.5
            * (bounds.maximum[0] - bounds.minimum[0]).hypot(bounds.maximum[1] - bounds.minimum[1])
            + gap;
        let centre = [
            (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
            (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
        ];
        for other in inputs.spokes.iter().filter(|other| {
            other.layer == record.layer
                && other.pad != record.pad
                && crate::geometry::distance(other.anchor, centre)
                    <= other.tip + other.width + radius
        }) {
            let (other_terminal, _, other_anchor) = inputs.pads[other.record];
            if crate::geometry::distance(other_anchor, record.anchor) < 1.0e-6
                || met.contains(&other.pad)
            {
                continue;
            }
            let counted = match inputs.outcomes[other.record][other.index] {
                SpokeOutcome::Pad(_) => true,
                SpokeOutcome::Good(piece) => piece != 0,
                _ => false,
            };
            if !counted
                || pad.shape.distance_to_segment(other.anchor, other.end())
                    > gap / 2.0 + other.width / 2.0
            {
                continue;
            }
            // Where its strip comes within half a gap of the pad,
            // the brush keeps room in it.
            // Inside its terminal's pad, the strip over that pad
            // (spokes start at their pad's centre).
            let start = if inputs.inner[record_index] {
                0.0
            } else {
                other.edge
            };
            let crossing =
                ring_crossing(probe, record.layer, &pad.shape, other, start, gap, minimum);
            if let Some(point) = crossing {
                // Inside its terminal's pad a strip may run right
                // through the pad's ring (the pad's own spokes from
                // its centre across a thermal via): KiCad counts the
                // crossings on both sides, two spokes.
                let through = inputs.inner[record_index]
                    && pad.shape.distance_to_point(other.anchor) > gap / 2.0 + other.width / 2.0
                    && pad.shape.distance_to_point(other.end()) > gap / 2.0 + other.width / 2.0;
                record.crossings += if through { 2 } else { 1 };
                crossing_points.extend(std::iter::repeat_n(point, if through { 2 } else { 1 }));
                // A strip over the pad's copper joins the two pads.
                if pad.shape.distance_to_segment(other.anchor, other.end()) <= other.width / 2.0 {
                    union(
                        parent,
                        terminal_base + record.terminal,
                        terminal_base + other_terminal,
                    );
                }
            }
        }
        crossing_points
    }

    /// Per zone of `zones` (none when the layer has one zone): the
    /// record's counted spokes and island spokes whose crossing of its
    /// ring half a gap out lies in the zone's part, and the other pads'
    /// crossings (`crossing_points`) there; the tracks' column is 0.
    fn zone_tallies(
        &self,
        spokes: &[Spoke],
        record_index: usize,
        record: &SpokeRecord,
        zones: &[usize],
        crossing_points: &[crate::geometry::Point],
    ) -> Vec<[usize; 4]> {
        let mut zone_tallies: Vec<[usize; 4]> = vec![[0; 4]; zones.len()];
        if !zones.is_empty() {
            for spoke in spokes.iter().filter(|spoke| spoke.record == record_index) {
                let kind = match record.outcomes[spoke.index] {
                    SpokeOutcome::Good(_) | SpokeOutcome::Pad(_) => 0,
                    SpokeOutcome::Island(_) => 1,
                    _ => continue,
                };
                let along = spoke.edge + spoke.gap / 2.0;
                let half = (spoke.width / 2.0 - 1.0e-3).max(0.0);
                let samples: Vec<crate::geometry::Point> = (0..=8)
                    .map(|step| {
                        let offset = -half + half * step as f64 / 4.0;
                        [
                            spoke.anchor[0] + spoke.direction[0] * along
                                - spoke.direction[1] * offset,
                            spoke.anchor[1]
                                + spoke.direction[1] * along
                                + spoke.direction[0] * offset,
                        ]
                    })
                    .collect();
                for (position, zone) in zones.iter().enumerate() {
                    if samples.iter().any(|point| self.in_zone_part(*zone, *point)) {
                        zone_tallies[position][kind] += 1;
                    }
                }
            }
            for point in crossing_points {
                for (position, zone) in zones.iter().enumerate() {
                    if self.in_zone_part(*zone, *point) {
                        zone_tallies[position][2] += 1;
                    }
                }
            }
        }
        zone_tallies
    }

    /// Counts in `record.tracks` the pad's own tracks that KiCad's DRC
    /// counts as spokes, and returns the end of each in the fill.
    fn track_spokes(
        &self,
        model: &SpokeNet,
        probe: &fill::FillProbe,
        inputs: &DrcInputs,
        parent: &mut [usize],
        record: &mut SpokeRecord,
    ) -> Vec<crate::geometry::Point> {
        let SpokeNet {
            net,
            state,
            description,
            skip,
            ..
        } = *model;
        let mut track_ends: Vec<crate::geometry::Point> = Vec::new();
        // The pad's own tracks (KiCad's DRC: each track segment
        // connected to the pad that starts within half a gap of it
        // and ends in this layer's fill, on any layer; else, one
        // that ends there and starts in the fill). A routed branch
        // is its stub from the pad's anchor and its first straight
        // run; an adopted one its track.
        let terminal = &description.terminals[record.terminal];
        let pad = &self.board.obstacles[record.pad];
        let gap = self
            .board
            .planes
            .iter()
            .filter(|plane| plane.net == net && plane.layer == record.layer)
            .map(|plane| plane.thermal_gap)
            .fold(0.0f64, f64::max);
        let near_pad =
            |point: crate::geometry::Point| pad.shape.distance_to_point(point) <= gap / 2.0;
        let layer = record.layer;
        let in_fill = |parent: &mut [usize], point: crate::geometry::Point| {
            self.track_end_in_fill(model, probe, inputs, parent, layer, point)
        };
        // A pad merged into its terminal has the tracks whose copper
        // overlaps its own (KiCad's connectivity: 0xCB's thermal
        // vias, which the exposed pad's tracks cross), not all its
        // terminal's.
        let merged_pad = record.pad != terminal.pad;
        let half_width = self.width_on(description.class, record.layer) / 2.0;
        let connected = |a: crate::geometry::Point, b: crate::geometry::Point, radius: f64| {
            !merged_pad || pad.shape.distance_to_segment(a, b) <= radius + 1.0e-6
        };
        // The emitted segments end at junctions (`materialize`):
        // KiCad tests the end of the segment, not of the run
        // (0xCB's MX3.S1: its track's run passes a junction in
        // fill and ends in a hatch hole).
        let junctions = junction_nodes(&state.branches);
        for (index, branch) in state.branches.iter().enumerate() {
            if skip(index) || branch.nodes.is_empty() {
                continue;
            }
            let (segments, radius) =
                self.pad_track_segments(model, record, index, branch, &junctions, half_width);
            for (a, b) in segments {
                let counted = connected(a, b, radius)
                    && if near_pad(a) {
                        in_fill(parent, b)
                    } else {
                        near_pad(b) && in_fill(parent, a)
                    };
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() && !counted {
                    let line = probe.quietly(|| {
                        format!(
                            "track not a spoke {} layer {}: {a:?}..{b:?} near {} {} fill {} {} hole {} {} ring {} {} kept {} {}",
                            terminal.label,
                            record.layer,
                            near_pad(a),
                            near_pad(b),
                            probe.fill_at(record.layer, a),
                            probe.fill_at(record.layer, b),
                            probe.in_hatch_hole(record.layer, a),
                            probe.in_hatch_hole(record.layer, b),
                            probe.in_ring(record.layer, a),
                            probe.in_ring(record.layer, b),
                            self.kept_piece(model, probe, &inputs.kept, parent, record.layer, a),
                            self.kept_piece(model, probe, &inputs.kept, parent, record.layer, b)
                        )
                    });
                    eprintln!("{line}");
                }
                if counted {
                    record.tracks += 1;
                    track_ends.push(if near_pad(a) { b } else { a });
                    if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                        eprintln!(
                            "track spoke {} layer {}: branch {index} {a:?}..{b:?}",
                            terminal.label, record.layer
                        );
                    }
                }
            }
        }
        // The short tracks from pads inside their terminal's pad
        // (`inner_spokes`), tracks of the net as any.
        for (layer, a, b, width) in &state.inner_spokes {
            if *layer == record.layer
                && pad.shape.distance_to_segment(*a, *b) <= width / 2.0 + 1.0e-6
                && if near_pad(*a) {
                    in_fill(parent, *b)
                } else {
                    near_pad(*b) && in_fill(parent, *a)
                }
            {
                record.tracks += 1;
                track_ends.push(if near_pad(*a) { *b } else { *a });
            }
        }
        track_ends
    }

    /// The segments of branch `index` (`branch`) that connect to the
    /// record's pad, and their half width (`half_width` for a routed
    /// branch): an adopted branch's track, else per end at the pad its
    /// stub from the anchor and its first straight run up to a junction.
    fn pad_track_segments(
        &self,
        model: &SpokeNet,
        record: &SpokeRecord,
        index: usize,
        branch: &Branch,
        junctions: &HashSet<Node>,
        half_width: f64,
    ) -> (Vec<(crate::geometry::Point, crate::geometry::Point)>, f64) {
        let SpokeNet {
            state, description, ..
        } = *model;
        let terminal = &description.terminals[record.terminal];
        let pad = &self.board.obstacles[record.pad];
        // A branch ends at this pad, or at a pad of the net at its
        // very spot (a USB-C receptacle's A1 and B12: KiCad connects
        // a track to both).
        let at_pad = |end: u16| {
            end as usize == record.terminal
                || description
                    .terminals
                    .get(end as usize)
                    .is_some_and(|other| {
                        end != NO_TERMINAL
                            && end != PLANE_TERMINAL
                            && end != FREE_END
                            && crate::geometry::distance(other.anchor, record.anchor) < 1.0e-6
                    })
        };
        let mut segments: Vec<(crate::geometry::Point, crate::geometry::Point)> = Vec::new();
        let mut radius = half_width;
        if let Some((start, end, adopted)) = state.adopted.get(&index) {
            radius = *adopted;
            if (at_pad(branch.start_terminal) || at_pad(branch.end_terminal))
                && pad.layers & (1 << branch.nodes[0].layer) != 0
            {
                segments.push((*start, *end));
            }
        } else {
            for (end, forward) in [(branch.start_terminal, true), (branch.end_terminal, false)] {
                if !at_pad(end) {
                    continue;
                }
                let nodes: Vec<Node> = if forward {
                    branch.nodes.clone()
                } else {
                    branch.nodes.iter().rev().copied().collect()
                };
                // Connected to the pad: on one of its layers.
                if pad.layers & (1 << nodes[0].layer) == 0 {
                    continue;
                }
                let first = self.grid.center_of(nodes[0].cell as usize);
                match state.escapes.get(&nodes[0]) {
                    Some((_, _, polyline)) => {
                        segments.extend(polyline.windows(2).map(|pair| (pair[0], pair[1])))
                    }
                    None if crate::geometry::distance(terminal.anchor, first) > 1.0e-7 => {
                        segments.push((terminal.anchor, first))
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
                        && !junctions.contains(&nodes[last])
                    {
                        last += 1;
                    }
                    segments.push((first, self.grid.center_of(nodes[last].cell as usize)));
                }
            }
        }
        (segments, radius)
    }

    /// Whether a track ending at `point` on `layer` ends in fill as
    /// KiCad's DRC tests it: out of a thermal pad's plated hole, and in a
    /// kept spoke, a pad's ring or kept fill at the track-end tolerance.
    fn track_end_in_fill(
        &self,
        model: &SpokeNet,
        probe: &fill::FillProbe,
        inputs: &DrcInputs,
        parent: &mut [usize],
        layer: usize,
        point: crate::geometry::Point,
    ) -> bool {
        // KiCad's fill keeps out of a thermal pad's plated hole (an
        // exposed pad's thermal via: a track to its centre ends in
        // no fill, 0xCB's U3.45).
        // The test is at a point, which KiCad's fill covers only
        // where it keeps the clearance to a micron or so
        // (`TRACK_END_TOLERANCE`).
        !inputs.thermal_holes[layer]
            .iter()
            .any(|hole| self.board.obstacles[*hole].shape.contains(point))
            && probe.for_track_end(|| {
                let minimum = self.board.classes[model.state.plane_class[layer]].trace_width;
                // A kept spoke is fill too, as far as other nets'
                // clearances leave the brush room in it (they start at
                // their pads' centres: a track to the pad's edge ends on
                // its own spoke, Castor's C12.1; U11.10's spoke is cut
                // over the pad by its neighbours in the row).
                inputs.spokes.iter().any(|spoke| {
                    let outcome = inputs.outcomes[spoke.record][spoke.index];
                    self.in_kept_spoke(probe, spoke, outcome, layer, minimum, point)
                }) || probe.in_ring(layer, point)
                    || (probe.final_fill_at(layer, point)
                        && self.kept_piece(model, probe, &inputs.kept, parent, layer, point))
            })
    }

    /// Whether `point` on `layer` lies in what other nets' clearances
    /// leave of `spoke` (with outcome `outcome`; `minimum` the zone's
    /// minimum width): a brush centre in the spoke within half the
    /// minimum width of it.
    fn in_kept_spoke(
        &self,
        probe: &fill::FillProbe,
        spoke: &Spoke,
        outcome: SpokeOutcome,
        layer: usize,
        minimum: f64,
        point: crate::geometry::Point,
    ) -> bool {
        if spoke.layer != layer || matches!(outcome, SpokeOutcome::NoFill) || !spoke.contains(point)
        {
            return false;
        }
        let offset = [point[0] - spoke.anchor[0], point[1] - spoke.anchor[1]];
        let along = offset[0] * spoke.direction[0] + offset[1] * spoke.direction[1];
        // A spoke whose end lands is KiCad's (its filler
        // tests the end alone) and other nets' clearances
        // cut it only beyond the pad: where they block it
        // the part over the pad stays fill (video's
        // U15.59, its tracks to the pad's edge end there).
        let reach = if matches!(outcome, SpokeOutcome::Blocked) {
            if !self.config.measured_thermals || along > spoke.edge + 1.0e-6 {
                return false;
            }
            spoke.edge
        } else {
            spoke.tip
        };
        let across = offset[0] * spoke.direction[1] - offset[1] * spoke.direction[0];
        // A brush centre in the spoke (within `side` of its
        // axis) within half the minimum width of the point,
        // on a grid of an eighth of the minimum width: the
        // point may lie in what the clearances leave of the
        // spoke only near its far edge (PixelWave's C4.1:
        // its track ends 0.01 mm inside a spoke its
        // neighbour's clearance trims to 0.43 mm).
        let step = minimum / 8.0;
        let mut tried: Vec<(i64, i64)> = Vec::new();
        (-4..=4).any(|i: i64| {
            (-4..=4).any(|j: i64| {
                let along = (along + i as f64 * step).clamp(0.0, reach);
                let across = (across + j as f64 * step).clamp(-spoke.side, spoke.side);
                let key = (
                    (along / 1.0e-6).round() as i64,
                    (across / 1.0e-6).round() as i64,
                );
                if tried.contains(&key) {
                    return false;
                }
                tried.push(key);
                let centre = [
                    spoke.anchor[0] + spoke.direction[0] * along + spoke.direction[1] * across,
                    spoke.anchor[1] + spoke.direction[1] * along - spoke.direction[0] * across,
                ];
                // No relief cuts a spoke (KiCad adds them after
                // the reliefs: Hub's C35.1, its track ends in
                // its spoke where it meets C30.1's, inside
                // C30.1's relief).
                crate::geometry::distance(centre, point) <= minimum / 2.0 + 1.0e-9
                    && probe.blocker(layer, centre, None).is_none()
            })
        })
    }

    /// Whether the piece nearest `point` on `layer` survives KiCad's
    /// island removal: its cluster holds something (`KeptPieces::roots`),
    /// its zone keeps pieces holding nothing of its area, or (in the
    /// measurement) a neck the lattice cuts joins it to a kept piece.
    fn kept_piece(
        &self,
        model: &SpokeNet,
        probe: &fill::FillProbe,
        kept: &KeptPieces,
        parent: &mut [usize],
        layer: usize,
        point: crate::geometry::Point,
    ) -> bool {
        let SpokeNet {
            pours,
            state,
            pitch,
            ..
        } = *model;
        let radius = self.board.classes[state.plane_class[layer]].trace_width / 2.0 + pitch;
        let Some((_, piece)) = self.nearest_piece(model, layer, point, radius) else {
            return false;
        };
        let is_kept = |parent: &mut [usize], piece: u32, at: crate::geometry::Point| {
            kept.roots.contains(&find(parent, piece as usize))
                || self.kept_bare(model, kept, layer, at, piece)
        };
        if is_kept(parent, piece, point) {
            return true;
        }
        // A piece holding nothing may be one outline of KiCad's with
        // a kept piece the lattice cuts it from (`neck_joined`; the
        // measurement's).
        if !self.config.measured_thermals {
            return false;
        }
        if let Some(known) = kept.neck_kept.borrow().get(&(layer, piece)) {
            return *known;
        }
        let joined = self.neck_joined(
            probe,
            &pours.label[layer],
            layer,
            piece,
            &mut |other, at| is_kept(parent, other, at),
        );
        // A debug output's question (`FillProbe::quietly`) leaves the
        // cache as it was.
        if !probe.is_quiet() {
            kept.neck_kept.borrow_mut().insert((layer, piece), joined);
        }
        joined
    }

    /// Whether the zones' island removal keeps piece `piece` of `layer`
    /// (its zone the one at `point`) though it holds nothing
    /// (`Plane::island_min_area`): all of them (never), those of the area
    /// (by area), none (always).
    fn kept_bare(
        &self,
        model: &SpokeNet,
        kept: &KeptPieces,
        layer: usize,
        point: crate::geometry::Point,
        piece: u32,
    ) -> bool {
        let SpokeNet {
            net, pours, pitch, ..
        } = *model;
        let Some(plane) = self.board.planes.iter().find(|plane| {
            plane.net == net
                && plane.layer == layer
                && crate::geometry::point_in_polygon(point, &plane.polygon)
        }) else {
            return false;
        };
        match plane.island_min_area {
            None => true,
            Some(minimum) if minimum > 0.0 => {
                let mut nodes = kept.piece_nodes.borrow_mut();
                if !kept.counted_layers.borrow().contains(&layer) {
                    kept.counted_layers.borrow_mut().push(layer);
                    for label in &pours.label[layer] {
                        if *label != 0 {
                            *nodes.entry((layer, *label)).or_default() += 1;
                        }
                    }
                }
                nodes.get(&(layer, piece)).copied().unwrap_or(0) as f64 * pitch * pitch >= minimum
            }
            Some(_) => false,
        }
    }
}

/// Where `other`'s strip, from `start` along it to its tip, crosses the
/// ring half a gap (`gap` the zone's thermal gap) around a pad of shape
/// `pad` on `layer`: the first step (a quarter of the minimum width
/// `minimum` apart) where it comes within half a gap of the pad and the
/// brush keeps room in it; that point on the spoke's axis.
fn ring_crossing(
    probe: &fill::FillProbe,
    layer: usize,
    pad: &crate::geometry::Shape,
    other: &Spoke,
    start: f64,
    gap: f64,
    minimum: f64,
) -> Option<crate::geometry::Point> {
    let steps = (((other.tip - start) / (minimum / 4.0)).ceil() as usize).max(1);
    (0..=steps).find_map(|step| {
        let along = start + (other.tip - start) * step as f64 / steps as f64;
        ([
            -other.width / 2.0,
            -other.side,
            0.0,
            other.side,
            other.width / 2.0,
        ]
        .iter()
        .any(|offset| {
            let at = [
                other.anchor[0] + other.direction[0] * along - other.direction[1] * offset,
                other.anchor[1] + other.direction[1] * along + other.direction[0] * offset,
            ];
            pad.distance_to_point(at) < gap / 2.0
        }) && [-other.side, 0.0, other.side].iter().any(|offset| {
            let at = [
                other.anchor[0] + other.direction[0] * along - other.direction[1] * offset,
                other.anchor[1] + other.direction[1] * along + other.direction[0] * offset,
            ];
            pad.distance_to_point(at) < gap / 2.0 + minimum / 2.0
                && probe.blocker(layer, at, None).is_none()
        }))
        .then(|| {
            [
                other.anchor[0] + other.direction[0] * along,
                other.anchor[1] + other.direction[1] * along,
            ]
        })
    })
}

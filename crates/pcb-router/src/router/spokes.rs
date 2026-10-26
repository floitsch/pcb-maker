// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Thermal spokes as KiCad draws and counts them.

use super::*;

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
    #[allow(clippy::too_many_arguments)]
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
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        let description = &self.board.nets[net as usize];
        let pitch = self.grid.pitch;
        // KiCad's DRC's minimum spoke count (the board setup's).
        let required = self.config.min_spokes;
        let union = |parent: &mut Vec<usize>, a: usize, b: usize| {
            let (a, b) = (find(parent, a), find(parent, b));
            parent[a] = b;
        };
        let isolated = |terminal: usize| {
            self.board
                .isolated_pads
                .binary_search(&description.terminals[terminal].label)
                .is_ok()
        };
        let solid_pad = |terminal: usize| {
            self.board
                .solid_pads
                .binary_search(&description.terminals[terminal].label)
                .is_ok()
        };
        let solid: Vec<bool> = (0..layers)
            .map(|layer| {
                self.board
                    .planes
                    .iter()
                    .any(|plane| plane.net == net && plane.layer == layer && plane.solid)
            })
            .collect();
        let label_at =
            |layer: usize, cell: usize| pours.label[layer].get(cell).copied().unwrap_or(0);
        // Items per piece (KiCad's CN_ZONE_LAYER::HasSingleConnection):
        // each branch with a node in it, each pad joined solidly, and each
        // pad whose spoke lands in it.
        let mut items: HashMap<u32, usize> = HashMap::new();
        for (index, pieces) in branch_pieces.iter().enumerate() {
            if skip(index) {
                continue;
            }
            for (piece, runs) in pieces {
                *items.entry(*piece).or_default() += runs;
            }
        }
        let reach = (0.8 / pitch).ceil() as usize;
        for (terminal, nodes) in state.terminal_nodes.iter().enumerate() {
            if isolated(terminal) {
                continue;
            }
            let mut seen: Vec<u32> = Vec::new();
            for node in nodes
                .iter()
                .filter(|node| solid[node.layer as usize] || solid_pad(terminal))
            {
                if let Some(piece) =
                    pours.piece_near(&self.grid, node.layer as usize, node.cell as usize, reach)
                    && !seen.contains(&piece)
                {
                    seen.push(piece);
                }
            }
            for piece in seen {
                *items.entry(piece).or_default() += 1;
            }
        }
        let probe = fill::FillProbe::new(self, net, rings);
        // Per layer, the net's zones KiCad fills there when there are
        // several: its DRC checks a pad against each zone
        // apart (DRC_TEST_PROVIDER_ZONE_CONNECTIONS::testZoneLayer), with
        // the crossings of that zone's fill alone, and each zone's fill is
        // its own outline less those of the net's zones of a higher
        // priority (zone_filler.cpp, subtractHigherPriorityZones; the fill
        // up to the seam is the same, built over the neighbours' outlines
        // and then cut).
        let zones_on: Vec<Vec<usize>> = (0..layers)
            .map(|layer| {
                let zones: Vec<usize> = self
                    .board
                    .planes
                    .iter()
                    .enumerate()
                    .filter(|(_, plane)| plane.net == net && plane.layer == layer)
                    .map(|(index, _)| index)
                    .collect();
                if zones.len() >= 2 { zones } else { Vec::new() }
            })
            .collect();
        let in_zone = |zone: usize, point: crate::geometry::Point| self.in_zone_part(zone, point);
        // Pass one: the spokes' geometry, then where each lands (Good holds
        // the piece until the items are counted).
        struct Spoke {
            record: usize,
            index: usize,
            pad: usize,
            layer: usize,
            anchor: crate::geometry::Point,
            direction: crate::geometry::Point,
            edge: f64,
            tip: f64,
            gap: f64,
            width: f64,
            side: f64,
        }
        let mut records: Vec<SpokeRecord> = Vec::new();
        let mut spokes: Vec<Spoke> = Vec::new();
        // The terminals' pads on the layers they are reached on, and the
        // pads merged into them on their layers: where they are copper of
        // their own (U1.24's through-hole pad on F.Cu beside its B.Cu pad)
        // and inside their terminal's pad, where KiCad
        // builds and counts their spokes as any pad's (an exposed pad's
        // thermal vias: in a hatched zone each has its ring, which its
        // spokes reach; in a solid one the pad's relief takes them, and
        // the DRC counts what crosses their rings, the pad's own spokes).
        // Per pad, whether it lies inside its terminal's pad on the layer.
        let mut spoke_pads: Vec<SpokePad> = Vec::new();
        // A padstack pad is its terminal's pad on the layers its own shape
        // has, the stack's shape (`MergedPad::padstack`) on the others.
        let stacked = |terminal: usize, layer: usize| {
            self.board.merged_pads.iter().any(|merged| {
                merged.padstack
                    && merged.net == net
                    && merged.host == terminal
                    && merged.layers & (1 << layer) != 0
            })
        };
        for (terminal, nodes) in state.terminal_nodes.iter().enumerate() {
            let own = self.board.obstacles[description.terminals[terminal].pad].layers;
            let layers_of_nodes: std::collections::BTreeSet<usize> = nodes
                .iter()
                .map(|node| node.layer as usize)
                .filter(|layer| own & (1 << layer) != 0 || !stacked(terminal, *layer))
                .collect();
            spoke_pads.push((
                terminal,
                description.terminals[terminal].pad,
                description.terminals[terminal].anchor,
                layers_of_nodes,
                Default::default(),
            ));
        }
        for merged in self
            .board
            .merged_pads
            .iter()
            .filter(|merged| merged.net == net && merged.host < state.terminal_nodes.len())
        {
            let host = &self.board.obstacles[description.terminals[merged.host].pad];
            let inside = |layer: &usize| {
                host.layers & (1 << layer) != 0 && host.shape.contains(merged.anchor)
            };
            let layers: std::collections::BTreeSet<usize> = (0..layers)
                .filter(|layer| merged.layers & (1 << layer) != 0)
                .collect();
            let inner = layers.iter().copied().filter(inside).collect();
            spoke_pads.push((merged.host, merged.pad, merged.anchor, layers, inner));
        }
        // Per record, whether its pad lies inside its terminal's pad.
        let mut record_inner: Vec<bool> = Vec::new();
        for (terminal, pad_index, pad_anchor, layers_of_nodes, inner) in spoke_pads {
            // The lowering gives zone connections per pad label, and the
            // largest pad of a label is its terminal: a solid connection
            // there is that pad's (an exposed pad set to solid), while the
            // thermal vias merged into it inherit the zone's thermal
            // relief, which KiCad's DRC checks (ray4477's U1.33 and
            // OpenAir's U5.17 vias on the far layers: no model row).
            // A pad merged into its terminal that is not its terminal's
            // own copper (an exposed pad's thermal via) has its own zone
            // connection, which KiCad's DRC checks by pad
            // (`merged_pad_thermal`: glasgow's U1.57, whose vias are set
            // to solid inside a thermal exposed pad).
            let merged = self.board.merged_pads.iter().find(|merged| {
                merged.net == net
                    && !merged.padstack
                    && merged.pad == pad_index
                    && merged.host == terminal
            });
            let skipped = match merged {
                Some(merged) => !self.merged_pad_thermal(merged),
                None => {
                    isolated(terminal)
                        || (solid_pad(terminal)
                            && (pad_index == description.terminals[terminal].pad
                                || self
                                    .board
                                    .merged_pads
                                    .iter()
                                    .any(|merged| merged.padstack && merged.pad == pad_index)))
                }
            };
            if skipped {
                continue;
            }
            for layer in layers_of_nodes {
                // Solid where the zones the pad lies in are: a small solid
                // zone of the net elsewhere on the layer leaves the large
                // thermal one's pads thermal (Apex's GND on F.Cu, a solid
                // zone of priority 3 beside the board's: C1.2, C2.2 and
                // C6.2 had no records).
                let containing: Vec<bool> = self
                    .board
                    .planes
                    .iter()
                    .filter(|plane| {
                        plane.net == net
                            && plane.layer == layer
                            && crate::geometry::point_in_polygon(pad_anchor, &plane.polygon)
                    })
                    .map(|plane| plane.solid)
                    .collect();
                let solid_here = if containing.is_empty() {
                    solid[layer]
                } else {
                    containing.iter().all(|solid| *solid)
                };
                if pours.label[layer].is_empty() || solid_here {
                    continue;
                }
                let planes = || {
                    self.board
                        .planes
                        .iter()
                        .filter(|plane| plane.net == net && plane.layer == layer && !plane.solid)
                };
                let gap = planes()
                    .map(|plane| plane.thermal_gap)
                    .fold(0.0f64, f64::max);
                let bridge = planes()
                    .map(|plane| (plane.thermal_reach - plane.thermal_gap).max(0.0))
                    .fold(0.0f64, f64::max);
                let minimum = self.board.classes[state.plane_class[layer]].trace_width;
                // The end KiCad tests: half the minimum width past the
                // gap, half the web's width in a hatched zone.
                let (round, anchor, geometry, minor) = self.spoke_geometry(
                    pad_index,
                    pad_anchor,
                    gap + fill::KICAD_MAX_ERROR + probe.spoke_half_width(layer),
                );
                // KiCad caps a spoke's width by the pad's size, a custom
                // pad's by its anchor's (Cuprum77's paw pads: a 0.1 mm
                // anchor in large primitives, no spoke, the pad unconnected
                // where the model joined it to the pour).
                let minor = match &description.terminals[terminal].contact {
                    Some(contact) if pad_index == description.terminals[terminal].pad => {
                        let bounds = contact.aabb();
                        minor.min(
                            (bounds.maximum[0] - bounds.minimum[0])
                                .min(bounds.maximum[1] - bounds.minimum[1]),
                        )
                    }
                    _ => minor,
                };
                let width = bridge.min(minor);
                // No spoke narrower than the minimum width.
                let drawn = width + 1.0e-9 >= minimum;
                if drawn {
                    for (index, (direction, edge, tip)) in geometry.into_iter().enumerate() {
                        spokes.push(Spoke {
                            record: records.len(),
                            index,
                            pad: pad_index,
                            layer,
                            anchor,
                            direction,
                            edge,
                            tip,
                            gap,
                            width,
                            side: ((width - minimum) / 2.0).max(0.0),
                        });
                    }
                }
                records.push(SpokeRecord {
                    terminal,
                    pad: pad_index,
                    anchor: pad_anchor,
                    crossings: 0,
                    layer,
                    round,
                    outcomes: [SpokeOutcome::NoFill; 4],
                    tracks: 0,
                    seam_short: 0,
                    seam_zone: None,
                    drawn,
                    required,
                });
                record_inner.push(inner.contains(&layer));
            }
        }
        let inside = |spoke: &Spoke, point: crate::geometry::Point| {
            let offset = [point[0] - spoke.anchor[0], point[1] - spoke.anchor[1]];
            let along = offset[0] * spoke.direction[0] + offset[1] * spoke.direction[1];
            let across = (offset[0] * spoke.direction[1] - offset[1] * spoke.direction[0]).abs();
            along >= -1.0e-6 && along <= spoke.tip + 1.0e-6 && across <= spoke.width / 2.0 + 1.0e-6
        };
        let end_of = |spoke: &Spoke| {
            [
                spoke.anchor[0] + spoke.direction[0] * spoke.tip,
                spoke.anchor[1] + spoke.direction[1] * spoke.tip,
            ]
        };
        // Per record, the pads whose spokes met its own.
        let mut met_pads: Vec<Vec<usize>> = vec![Vec::new(); records.len()];
        // Per spoke, whether what is left of it joins the pad to the ring
        // half a gap out: neighbours' clearances may cut a spoke between the
        // pad and the ring (k30's U14.12 in a row of 0.8 mm pitch: its
        // spoke reaches a pocket of fill past the row, cut off at the pad's
        // end). The part beyond still crosses the ring, but the pad is no
        // item of the piece it lands in (KiCad removes the pocket, an
        // island with nothing in it).
        let mut attached = vec![true; spokes.len()];
        for (position, spoke) in spokes.iter().enumerate() {
            let layer = spoke.layer;
            let minimum = self.board.classes[state.plane_class[layer]].trace_width;
            let end = end_of(spoke);
            // The DRC counts the spoke where the fill crosses the ring half a
            // gap around the pad: a brush centre within half the minimum
            // width of it, joined to the spoke's end (centres all the way).
            let from = (spoke.edge + spoke.gap / 2.0 + minimum / 2.0).min(spoke.tip);
            // KiCad keeps the spoke when its end lies in the fill, or in
            // a spoke of another pad whose end lies in this one (two pads
            // side by side: mackerel-08's J10 pins on In1, Castor's J2
            // shield).
            if !probe.spoke_end_lands(layer, end, spoke.direction) {
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                    eprintln!(
                        "no fill at spoke end {} layer {layer} dir {:?} end {end:?}: {:?}",
                        description.terminals[records[spoke.record].terminal].label,
                        spoke.direction,
                        probe.blocker(layer, end, Some(&[]))
                    );
                }
                // Spokes of pads apart, whatever their directions (KiCad
                // asks only that each end lies in the other spoke: Hub's
                // J3.A1 and B12, 1.35 mm apart, meet with diagonal spokes at
                // right angles). A pad doubled at one spot (a USB-C
                // receptacle's A1 and B12) has one set of spokes in KiCad,
                // which dedups coincident pads: its spokes do not meet.
                // Pads of one terminal meet too where they are apart (an
                // exposed pad's thermal vias).
                let meets = spokes.iter().find(|other| {
                    other.layer == layer
                        && (records[other.record].terminal != records[spoke.record].terminal
                            || other.pad != spoke.pad)
                        && crate::geometry::distance(other.anchor, spoke.anchor) > 1.0e-6
                        && crate::geometry::distance(other.anchor, spoke.anchor)
                            <= other.tip + spoke.tip
                        && inside(other, end)
                        && inside(spoke, end_of(other))
                });
                if let Some(other) = meets
                    && (!self.config.spoke_clearance
                        || probe.spoke_clear(
                            layer,
                            spoke.anchor,
                            spoke.direction,
                            from,
                            spoke.tip,
                            spoke.side,
                        ))
                {
                    records[spoke.record].outcomes[spoke.index] =
                        SpokeOutcome::Pad(records[other.record].terminal as u32);
                    met_pads[spoke.record].push(other.pad);
                }
                continue;
            }
            // What is left of it after the clearances, from the middle of
            // the gap to its end, keeps the brush's width (k30's U2.7: a neighbour's clearance cuts it on
            // a fine-pitch row).
            // The brush must fit from the ring to where the fill's own
            // brush centres begin, the gap and half the minimum width past
            // the pad (they keep that from its relief), not to the end,
            // which KiCad tests 5 microns further for lying in fill
            // (Telemetry's D12.1: its spoke ends in a strip exactly the
            // minimum width between its relief and D12.2's clearance, whose
            // brush centres lie on one line).
            let clear_to = (spoke.tip - fill::KICAD_MAX_ERROR).max(from);
            if self.config.spoke_clearance
                && !probe.spoke_clear(
                    layer,
                    spoke.anchor,
                    spoke.direction,
                    from,
                    clear_to,
                    spoke.side,
                )
            {
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                    let steps = 8;
                    for step in 0..=steps {
                        let along = from + (spoke.tip - from) * step as f64 / steps as f64;
                        let at = [
                            spoke.anchor[0] + spoke.direction[0] * along,
                            spoke.anchor[1] + spoke.direction[1] * along,
                        ];
                        if let Some(why) = probe.blocker(layer, at, None) {
                            eprintln!(
                                "blocked spoke {} layer {layer} dir {:?} width {:.3} side {:.3} at {along:.3} ({:.3}..{:.3}): {why}",
                                description.terminals[records[spoke.record].terminal].label,
                                spoke.direction,
                                spoke.width,
                                spoke.side,
                                from,
                                spoke.tip
                            );
                            break;
                        }
                    }
                }
                records[spoke.record].outcomes[spoke.index] = SpokeOutcome::Blocked;
                continue;
            }
            // The piece it joins: the nearest of the pour map's (none for
            // a sliver the lattice misses: a piece of its own).
            let radius = minimum / 2.0 + pitch;
            let mut best: Option<(f64, u32)> = None;
            if let Some((x0, y0, x1, y1)) = self.grid.node_range(
                crate::geometry::Aabb {
                    minimum: end,
                    maximum: end,
                }
                .inflated(radius),
            ) {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let piece = label_at(layer, self.grid.index(x, y));
                        let distance = crate::geometry::distance(end, self.grid.center(x, y));
                        if piece != 0
                            && distance <= radius
                            && best.is_none_or(|(best, _)| distance < best)
                        {
                            best = Some((distance, piece));
                        }
                    }
                }
            }
            // None that near: the end may lie in the narrow tip of a wedge
            // of fill whose brush centres the lattice holds only further in
            // (bit-preserve's IC11.1: its spoke ends where a track's
            // clearance and the relief leave a wedge, the nearest labelled
            // node 0.3 mm away). The nearest labelled node within the
            // minimum width and four pitches that a straight run of fill
            // joins to the end.
            if best.is_none() {
                let far = minimum + 4.0 * pitch;
                let mut candidates: Vec<(f64, usize)> = Vec::new();
                if let Some((x0, y0, x1, y1)) = self.grid.node_range(
                    crate::geometry::Aabb {
                        minimum: end,
                        maximum: end,
                    }
                    .inflated(far),
                ) {
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let cell = self.grid.index(x, y);
                            let distance = crate::geometry::distance(end, self.grid.center(x, y));
                            if label_at(layer, cell) != 0 && distance <= far {
                                candidates.push((distance, cell));
                            }
                        }
                    }
                }
                candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                let step = (pitch.min(minimum) / 2.0).max(0.01);
                best = candidates.into_iter().take(8).find_map(|(distance, cell)| {
                    let centre = self.grid.center_of(cell);
                    let steps = ((distance / step).ceil() as usize).max(1);
                    (1..steps)
                        .all(|index| {
                            let t = index as f64 / steps as f64;
                            probe.test_area_at(
                                layer,
                                [
                                    end[0] + (centre[0] - end[0]) * t,
                                    end[1] + (centre[1] - end[1]) * t,
                                ],
                            )
                        })
                        .then(|| (distance, label_at(layer, cell)))
                });
            }
            // Piece 0 stands for a sliver the lattice misses: the spoke
            // counts only where a track of the pad touches it too.
            records[spoke.record].outcomes[spoke.index] =
                SpokeOutcome::Good(best.map_or(0, |(_, piece)| piece));
            // Only a kept spoke's join to the pad matters.
            if self.config.spoke_clearance {
                // A brush centre within half the minimum width of the pad
                // touches it.
                attached[position] = probe.spoke_clear(
                    layer,
                    spoke.anchor,
                    spoke.direction,
                    (spoke.edge + minimum / 2.0 - 1.0e-3).min(from),
                    from,
                    spoke.side,
                );
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                    let at = [
                        spoke.anchor[0] + spoke.direction[0] * spoke.edge,
                        spoke.anchor[1] + spoke.direction[1] * spoke.edge,
                    ];
                    eprintln!(
                        "spoke {} layer {layer} dir {:?} attached {} (edge {:.3} from {:.3} side {:.3}; at the edge: {:?})",
                        description.terminals[records[spoke.record].terminal].label,
                        spoke.direction,
                        attached[position],
                        spoke.edge,
                        from,
                        spoke.side,
                        probe.blocker(layer, at, None)
                    );
                }
            }
        }
        // The pads' drill holes: KiCad's fill keeps out of a plated hole,
        // so a spoke reaches into a through-hole pad only up to its hole.
        let mut record_attached = vec![[true; 4]; records.len()];
        for (position, spoke) in spokes.iter().enumerate() {
            record_attached[spoke.record][spoke.index] = attached[position];
        }
        // Per label, each plated hole's centre and radius (pads may share
        // a number: the hole is the one at the pad).
        let mut holes: HashMap<&str, Vec<(crate::geometry::Point, f64)>> = HashMap::new();
        for obstacle in self
            .board
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.kind == crate::board::ObstacleKind::Hole)
        {
            if let Some(label) = obstacle.label.strip_suffix(" plated hole") {
                let bounds = obstacle.shape.aabb();
                let centre = [
                    (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                    (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
                ];
                let radius = ((bounds.maximum[0] - bounds.minimum[0])
                    .min(bounds.maximum[1] - bounds.minimum[1])
                    / 2.0)
                    .max(0.0);
                holes.entry(label).or_default().push((centre, radius));
            }
        }
        let hole_at = |label: &str, at: crate::geometry::Point| {
            holes
                .get(label)
                .and_then(|holes| {
                    holes.iter().min_by(|a, b| {
                        crate::geometry::distance(a.0, at)
                            .total_cmp(&crate::geometry::distance(b.0, at))
                    })
                })
                .map_or(0.0, |(_, radius)| *radius)
        };
        // Per record, which of its spokes the pad's own tracks on the layer
        // touch, and so join to the piece the spoke lands in (a second
        // item: Castor's D11.3, KiCad counts its one spoke, its exit stub
        // in the piece). A track touches a spoke only where its first run
        // overlaps the spoke's strip outside the pad's hole (mackerel-30's
        // C50.2: a track leaving a round pad along an axis misses its
        // diagonal spokes, which reach the pad only up to its drill, and
        // KiCad finds the piece they land in an isolated island).
        let mut touched_spokes: Vec<[bool; 4]> = Vec::new();
        // Per piece, the pads joined to it by a spoke (one item each,
        // however many pieces of one outline they reach), and the pairs of
        // pieces a pad's overlapping spokes join.
        let mut pad_items: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut spoke_joins: Vec<(u32, u32)> = Vec::new();
        for (record_index, record) in records.iter().enumerate() {
            let (terminal, layer, outcomes) = (record.terminal, record.layer, record.outcomes);
            let kept: Vec<&Spoke> = spokes
                .iter()
                .filter(|spoke| {
                    spoke.record == record_index
                        && matches!(
                            outcomes[spoke.index],
                            SpokeOutcome::Good(_) | SpokeOutcome::Pad(_)
                        )
                })
                .collect();
            let hole = hole_at(
                description.terminals[terminal].label.as_str(),
                record.anchor,
            );
            let strip_of = |spoke: &Spoke| {
                let (d, w) = (spoke.direction, spoke.width / 2.0);
                let normal = [-d[1] * w, d[0] * w];
                // A spoke cut off from the pad begins where the ring is
                // (a track touching it over the pad touches nothing).
                let start = if record_attached[record_index][spoke.index] {
                    hole
                } else {
                    spoke.edge + spoke.gap / 2.0
                };
                let near = [
                    spoke.anchor[0] + d[0] * start.min(spoke.tip),
                    spoke.anchor[1] + d[1] * start.min(spoke.tip),
                ];
                let far = [
                    spoke.anchor[0] + d[0] * spoke.tip,
                    spoke.anchor[1] + d[1] * spoke.tip,
                ];
                crate::geometry::Shape::Polygon {
                    points: vec![
                        [near[0] + normal[0], near[1] + normal[1]],
                        [far[0] + normal[0], far[1] + normal[1]],
                        [far[0] - normal[0], far[1] - normal[1]],
                        [near[0] - normal[0], near[1] - normal[1]],
                    ],
                }
            };
            let mut touched = [false; 4];
            for (index, branch) in state.branches.iter().enumerate() {
                if skip(index) || branch.nodes.is_empty() {
                    continue;
                }
                let ends_here = (branch.start_terminal as usize == terminal
                    && branch
                        .nodes
                        .first()
                        .is_some_and(|node| node.layer as usize == layer))
                    || (branch.end_terminal as usize == terminal
                        && branch
                            .nodes
                            .last()
                            .is_some_and(|node| node.layer as usize == layer));
                if !ends_here {
                    continue;
                }
                let segments: Vec<(crate::geometry::Point, crate::geometry::Point, f64)> =
                    if let Some((start, end, radius)) = state.adopted.get(&index) {
                        vec![(*start, *end, *radius)]
                    } else {
                        self.first_run(net, terminal, branch)
                    };
                for spoke in &kept {
                    let strip = strip_of(spoke);
                    let meets = if record_attached[record_index][spoke.index] && hole > 0.0 {
                        // KiCad's spoke runs from the pad's centre and the
                        // plated hole is knocked out of it: what is left
                        // beside the hole reaches nearer the centre than
                        // the hole's radius along the spoke (k30's C16.1:
                        // a 0.8 mm drill, its exit stub straight down meets
                        // the diagonal spoke's corner beside the hole, and
                        // KiCad joins it to the piece). A point of the
                        // track's centreline outside the hole within its
                        // half width of the whole strip.
                        let whole = {
                            let (d, w) = (spoke.direction, spoke.width / 2.0);
                            let normal = [-d[1] * w, d[0] * w];
                            let far = [
                                spoke.anchor[0] + d[0] * spoke.tip,
                                spoke.anchor[1] + d[1] * spoke.tip,
                            ];
                            let near = spoke.anchor;
                            crate::geometry::Shape::Polygon {
                                points: vec![
                                    [near[0] + normal[0], near[1] + normal[1]],
                                    [far[0] + normal[0], far[1] + normal[1]],
                                    [far[0] - normal[0], far[1] - normal[1]],
                                    [near[0] - normal[0], near[1] - normal[1]],
                                ],
                            }
                        };
                        segments.iter().any(|(a, b, radius)| {
                            let length = crate::geometry::distance(*a, *b);
                            let steps = ((length / 0.005).ceil() as usize).max(1);
                            (0..=steps).any(|step| {
                                let t = step as f64 / steps as f64;
                                let point = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                                crate::geometry::distance(point, spoke.anchor) >= hole - 1.0e-9
                                    && whole.distance_to_point(point) <= *radius + 1.0e-9
                            })
                        })
                    } else {
                        segments
                            .iter()
                            .any(|(a, b, radius)| strip.distance_to_segment(*a, *b) <= *radius)
                    };
                    if meets {
                        touched[spoke.index] = true;
                    }
                }
            }
            // Per piece: whether a spoke joins the pad to it, and whether
            // the pad's own track touches one (each an item of it).
            let mut seen: Vec<(u32, bool, bool)> = Vec::new();
            for (index, outcome) in outcomes.iter().enumerate() {
                if let SpokeOutcome::Good(piece) = outcome
                    && *piece != 0
                {
                    let joined = record_attached[record_index][index];
                    match seen.iter_mut().find(|(seen, _, _)| seen == piece) {
                        Some(entry) => {
                            entry.1 |= joined;
                            entry.2 |= touched[index];
                        }
                        None => seen.push((*piece, joined, touched[index])),
                    }
                }
            }
            for (piece, joined, tracked) in seen {
                *items.entry(piece).or_default() += tracked as usize;
                if joined {
                    let pads = pad_items.entry(piece).or_default();
                    if !pads.contains(&record.pad) {
                        pads.push(record.pad);
                    }
                }
            }
            // Kept spokes of one pad are one polygon where they overlap:
            // all of them on a pad without a hole (they start at its
            // centre), on a plated pad where they still overlap outside its
            // drill. The pieces they land in are then one outline to
            // KiCad, the pad one item of it.
            let joined_spokes: Vec<&&Spoke> = kept
                .iter()
                .filter(|spoke| {
                    record_attached[record_index][spoke.index]
                        && matches!(outcomes[spoke.index], SpokeOutcome::Good(piece) if piece != 0)
                })
                .collect();
            if joined_spokes.len() > 1 {
                let hole = hole_at(
                    description.terminals[terminal].label.as_str(),
                    record.anchor,
                );
                let piece_of = |spoke: &Spoke| {
                    if let SpokeOutcome::Good(piece) = outcomes[spoke.index] {
                        piece
                    } else {
                        0
                    }
                };
                for (position, a) in joined_spokes.iter().enumerate() {
                    for b in &joined_spokes[position + 1..] {
                        let (pa, pb) = (piece_of(a), piece_of(b));
                        if pa == pb {
                            continue;
                        }
                        let facing = a.direction[0] * b.direction[0]
                            + a.direction[1] * b.direction[1]
                            < -0.5;
                        let reach = a.width.min(b.width) / 2.0
                            * if facing {
                                1.0
                            } else {
                                std::f64::consts::SQRT_2
                            };
                        if hole < reach - 1.0e-6 {
                            spoke_joins.push((pa, pb));
                        }
                    }
                }
            }
            touched_spokes.push(touched);
        }
        // Necks the lattice cuts: a piece a kept spoke lands in, short of
        // two items, joins a piece next to it where KiCad's fill runs
        // between their nodes (a neck narrower than the lattice resolves;
        // census of 7 boards: 23 island spokes in KiCad outlines our map
        // splits, wrong verdicts on 4 pads such as Castor's U8.12).
        let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
        let merge =
            |groups: &mut HashMap<u32, Vec<u32>>, parent: &mut Vec<usize>, a: u32, b: u32| {
                union(parent, a as usize, b as usize);
                let mut group = groups.get(&a).cloned().unwrap_or_else(|| vec![a]);
                group.extend(groups.get(&b).cloned().unwrap_or_else(|| vec![b]));
                group.sort_unstable();
                group.dedup();
                for member in &group {
                    groups.insert(*member, group.clone());
                }
            };
        let count_of = |groups: &HashMap<u32, Vec<u32>>, piece: u32| -> usize {
            let single = [piece];
            let members: &[u32] = groups.get(&piece).map_or(&single[..], |group| &group[..]);
            let mut pads: Vec<usize> = Vec::new();
            let mut count = 0;
            for member in members {
                count += items.get(member).copied().unwrap_or(0);
                for pad in pad_items.get(member).map_or(&[][..], |pads| &pads[..]) {
                    if !pads.contains(pad) {
                        pads.push(*pad);
                    }
                }
            }
            count + pads.len()
        };
        for (a, b) in std::mem::take(&mut spoke_joins) {
            merge(&mut groups, parent, a, b);
        }
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        let mut done: HashSet<(usize, u32)> = HashSet::new();
        for spoke in &spokes {
            let SpokeOutcome::Good(piece) = records[spoke.record].outcomes[spoke.index] else {
                continue;
            };
            let layer = spoke.layer;
            if piece == 0 || count_of(&groups, piece) >= 2 || !done.insert((layer, piece)) {
                continue;
            }
            // The piece's nodes on the layer, from the node nearest the
            // spoke's end (a bounded walk).
            let end = end_of(spoke);
            let Some((x0, y0, x1, y1)) = self.grid.node_range(
                crate::geometry::Aabb {
                    minimum: end,
                    maximum: end,
                }
                .inflated(2.0 * pitch + spoke.width),
            ) else {
                continue;
            };
            let mut queue: Vec<usize> = Vec::new();
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let cell = self.grid.index(x, y);
                    if label_at(layer, cell) == piece {
                        queue.push(cell);
                    }
                }
            }
            let mut seen: HashSet<usize> = queue.iter().copied().collect();
            let mut neighbours: Vec<u32> = Vec::new();
            while let Some(cell) = queue.pop() {
                if seen.len() > 4000 {
                    break;
                }
                let (x, y) = self.grid.xy(cell);
                for dy in -2i64..=2 {
                    for dx in -2i64..=2 {
                        let (ox, oy) = (x as i64 + dx, y as i64 + dy);
                        if (dx == 0 && dy == 0) || ox < 0 || oy < 0 || ox >= nx || oy >= ny {
                            continue;
                        }
                        let other = self.grid.index(ox as usize, oy as usize);
                        let label = label_at(layer, other);
                        if label == piece {
                            if dx.abs() <= 1 && dy.abs() <= 1 && seen.insert(other) {
                                queue.push(other);
                            }
                        } else if label != 0 && !neighbours.contains(&label) {
                            let (a, b) = (self.grid.center_of(cell), self.grid.center_of(other));
                            let steps = 8;
                            if (0..=steps).all(|step| {
                                let t = step as f64 / steps as f64;
                                probe.fill_at(
                                    layer,
                                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                                )
                            }) {
                                neighbours.push(label);
                            }
                        }
                    }
                }
            }
            for label in neighbours {
                merge(&mut groups, parent, piece, label);
            }
        }
        // Pass two: spokes into pieces with another item count and join.
        let mut tallies: Vec<(usize, Vec<usize>)> = Vec::new();
        for (((record, touched), joined), inner) in records
            .iter_mut()
            .zip(&touched_spokes)
            .zip(&record_attached)
            .zip(&record_inner)
        {
            let mut good = 0;
            let mut islands: Vec<usize> = Vec::new();
            for (index, outcome) in record.outcomes.iter_mut().enumerate() {
                let tracked = &touched[index];
                if *inner && matches!(*outcome, SpokeOutcome::Good(_)) {
                    // Inside its terminal's pad a spoke lands in fill that
                    // overlaps that pad (in a hatched zone, its own ring):
                    // the outline holds both pads, two items.
                    if let SpokeOutcome::Good(piece) = *outcome
                        && piece != 0
                        && joined[index]
                    {
                        union(parent, terminal_base + record.terminal, piece as usize);
                    }
                    good += 1;
                } else if let SpokeOutcome::Pad(other) = *outcome {
                    // One outline holds both pads: the spoke counts.
                    union(
                        parent,
                        terminal_base + record.terminal,
                        terminal_base + other as usize,
                    );
                    good += 1;
                } else if let SpokeOutcome::Good(0) = *outcome {
                    if *tracked {
                        good += 1;
                    } else {
                        *outcome = SpokeOutcome::Island(0);
                    }
                } else if let SpokeOutcome::Good(piece) = *outcome {
                    let count = count_of(&groups, piece);
                    if !joined[index] && !tracked {
                        // Cut off from the pad: the crossing counts where
                        // the piece holds two items, is an island's with
                        // one, and is gone with none.
                        if count >= 2 {
                            good += 1;
                        } else if count == 1 {
                            *outcome = SpokeOutcome::Island(piece);
                        } else {
                            *outcome = SpokeOutcome::NoFill;
                        }
                    } else if count >= 2 {
                        union(parent, terminal_base + record.terminal, piece as usize);
                        good += 1;
                    } else {
                        *outcome = SpokeOutcome::Island(piece);
                        islands.push(piece as usize);
                    }
                }
            }
            tallies.push((good, islands));
        }
        // Pass three, with every join made: the DRC's track spokes. KiCad
        // removes a fill piece no pad's cluster reaches (an orphan) before
        // the DRC looks, so a track ending there counts for nothing
        // (Castor's RV10.1: its F.Cu track ends over an orphan of B.Cu).
        // KiCad's clusters join a piece to the pads whose spokes reach it
        // whether the DRC counts them or not (BAGEL's J102.1: its In1 track
        // ends over a piece of In2 that pads spoke into as islands).
        let mut terminal_roots: HashSet<usize> = (0..state.terminal_nodes.len())
            .map(|terminal| find(parent, terminal_base + terminal))
            .collect();
        for record in &records {
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
        // What the zones' island removal keeps of a piece holding nothing
        // (`Plane::island_min_area`): all of them (never), those of the
        // area (by area), none (always).
        let piece_nodes: std::cell::RefCell<HashMap<(usize, u32), usize>> = Default::default();
        let counted_layers: std::cell::RefCell<Vec<usize>> = Default::default();
        let kept_bare = |layer: usize, point: crate::geometry::Point, piece: u32| {
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
                    let mut nodes = piece_nodes.borrow_mut();
                    if !counted_layers.borrow().contains(&layer) {
                        counted_layers.borrow_mut().push(layer);
                        for label in &pours.label[layer] {
                            if *label != 0 {
                                *nodes.entry((layer, *label)).or_default() += 1;
                            }
                        }
                    }
                    nodes.get(&(layer, piece)).copied().unwrap_or(0) as f64 * pitch * pitch
                        >= minimum
                }
                Some(_) => false,
            }
        };
        let neck_kept: std::cell::RefCell<HashMap<(usize, u32), bool>> = Default::default();
        let kept_piece = |parent: &mut Vec<usize>, layer: usize, point: crate::geometry::Point| {
            let radius = self.board.classes[state.plane_class[layer]].trace_width / 2.0 + pitch;
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
                        let piece = label_at(layer, self.grid.index(x, y));
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
            let Some((_, piece)) = best else {
                return false;
            };
            let kept = |parent: &mut Vec<usize>, piece: u32, at: crate::geometry::Point| {
                terminal_roots.contains(&find(parent, piece as usize))
                    || kept_bare(layer, at, piece)
            };
            if kept(parent, piece, point) {
                return true;
            }
            // A piece holding nothing may be one outline of KiCad's with
            // a kept piece the lattice cuts it from (`neck_joined`).
            if let Some(known) = neck_kept.borrow().get(&(layer, piece)) {
                return *known;
            }
            let joined = self.neck_joined(
                &probe,
                &pours.label[layer],
                layer,
                piece,
                &mut |other, at| kept(parent, other, at),
            );
            neck_kept.borrow_mut().insert((layer, piece), joined);
            joined
        };
        // Per layer, the plated holes of the net's thermal pads there.
        let thermal_holes: Vec<Vec<usize>> = (0..layers)
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
            .collect();
        let records_outcomes: Vec<[SpokeOutcome; 4]> =
            records.iter().map(|record| record.outcomes).collect();
        let records_pads: Vec<(usize, usize, crate::geometry::Point)> = records
            .iter()
            .map(|record| (record.terminal, record.pad, record.anchor))
            .collect();
        let mut spoke_islands: Vec<(usize, usize)> = Vec::new();
        for (record_index, (record, (good, islands))) in records.iter_mut().zip(tallies).enumerate()
        {
            // Where other pads' spokes cross this pad's ring (for the count
            // per zone below).
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
            let met = &met_pads[record_index];
            let bounds = pad.shape.aabb();
            let radius = 0.5
                * (bounds.maximum[0] - bounds.minimum[0])
                    .hypot(bounds.maximum[1] - bounds.minimum[1])
                + gap;
            let centre = [
                (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
            ];
            for other in spokes.iter().filter(|other| {
                other.layer == record.layer
                    && other.pad != record.pad
                    && crate::geometry::distance(other.anchor, centre)
                        <= other.tip + other.width + radius
            }) {
                let (other_terminal, _, other_anchor) = records_pads[other.record];
                if crate::geometry::distance(other_anchor, record.anchor) < 1.0e-6
                    || met.contains(&other.pad)
                {
                    continue;
                }
                let counted = match records_outcomes[other.record][other.index] {
                    SpokeOutcome::Pad(_) => true,
                    SpokeOutcome::Good(piece) => piece != 0,
                    _ => false,
                };
                if !counted
                    || pad.shape.distance_to_segment(other.anchor, end_of(other))
                        > gap / 2.0 + other.width / 2.0
                {
                    continue;
                }
                // Where its strip comes within half a gap of the pad,
                // the brush keeps room in it.
                // Inside its terminal's pad, the strip over that pad
                // (spokes start at their pad's centre).
                let start = if record_inner[record_index] {
                    0.0
                } else {
                    other.edge
                };
                let steps = (((other.tip - start) / (minimum / 4.0)).ceil() as usize).max(1);
                let crossing = (0..=steps).find_map(|step| {
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
                            other.anchor[0] + other.direction[0] * along
                                - other.direction[1] * offset,
                            other.anchor[1]
                                + other.direction[1] * along
                                + other.direction[0] * offset,
                        ];
                        pad.shape.distance_to_point(at) < gap / 2.0
                    }) && [-other.side, 0.0, other.side].iter().any(|offset| {
                        let at = [
                            other.anchor[0] + other.direction[0] * along
                                - other.direction[1] * offset,
                            other.anchor[1]
                                + other.direction[1] * along
                                + other.direction[0] * offset,
                        ];
                        pad.shape.distance_to_point(at) < gap / 2.0 + minimum / 2.0
                            && probe.blocker(record.layer, at, None).is_none()
                    }))
                    .then(|| {
                        [
                            other.anchor[0] + other.direction[0] * along,
                            other.anchor[1] + other.direction[1] * along,
                        ]
                    })
                });
                if let Some(point) = crossing {
                    // Inside its terminal's pad a strip may run right
                    // through the pad's ring (the pad's own spokes from
                    // its centre across a thermal via): KiCad counts the
                    // crossings on both sides, two spokes.
                    let through = record_inner[record_index]
                        && pad.shape.distance_to_point(other.anchor)
                            > gap / 2.0 + other.width / 2.0
                        && pad.shape.distance_to_point(end_of(other))
                            > gap / 2.0 + other.width / 2.0;
                    record.crossings += if through { 2 } else { 1 };
                    crossing_points.extend(std::iter::repeat_n(point, if through { 2 } else { 1 }));
                    // A strip over the pad's copper joins the two pads.
                    if pad.shape.distance_to_segment(other.anchor, end_of(other))
                        <= other.width / 2.0
                    {
                        union(
                            parent,
                            terminal_base + record.terminal,
                            terminal_base + other_terminal,
                        );
                    }
                }
            }
            let good = good + record.crossings;
            // Per zone of the layer: the pad's spokes whose
            // crossing of its ring half a gap out lies in the zone's part
            // (a spoke the seam splits lengthwise crosses in both), the
            // island spokes so, the other pads' spokes crossing there and,
            // below, the tracks ending in its fill.
            let zones = &zones_on[record.layer];
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
                        if samples.iter().any(|point| in_zone(*zone, *point)) {
                            zone_tallies[position][kind] += 1;
                        }
                    }
                }
                for point in &crossing_points {
                    for (position, zone) in zones.iter().enumerate() {
                        if in_zone(*zone, *point) {
                            zone_tallies[position][2] += 1;
                        }
                    }
                }
            }
            let zone_needs_tracks = zone_tallies.iter().any(|[spokes, islands, crossings, _]| {
                spokes + islands + crossings > 0 && spokes + crossings < record.required
            });
            let mut track_ends: Vec<crate::geometry::Point> = Vec::new();
            if good < record.required || zone_needs_tracks {
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
                let minimum = self.board.classes[state.plane_class[record.layer]].trace_width;
                let in_fill = |parent: &mut Vec<usize>, point: crate::geometry::Point| {
                    // A kept spoke is fill too, as far as other nets'
                    // clearances leave the brush room in it (they start at
                    // their pads' centres: a track to the pad's edge ends on
                    // its own spoke, Castor's C12.1; U11.10's spoke is cut
                    // over the pad by its neighbours in the row).
                    spokes.iter().any(|spoke| {
                        let outcome = records_outcomes[spoke.record][spoke.index];
                        if spoke.layer != record.layer
                            || matches!(outcome, SpokeOutcome::NoFill)
                            || !inside(spoke, point)
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
                            if along > spoke.edge + 1.0e-6 {
                                return false;
                            }
                            spoke.edge
                        } else {
                            spoke.tip
                        };
                        let across =
                            offset[0] * spoke.direction[1] - offset[1] * spoke.direction[0];
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
                                let across =
                                    (across + j as f64 * step).clamp(-spoke.side, spoke.side);
                                let key = (
                                    (along / 1.0e-6).round() as i64,
                                    (across / 1.0e-6).round() as i64,
                                );
                                if tried.contains(&key) {
                                    return false;
                                }
                                tried.push(key);
                                let centre = [
                                    spoke.anchor[0]
                                        + spoke.direction[0] * along
                                        + spoke.direction[1] * across,
                                    spoke.anchor[1] + spoke.direction[1] * along
                                        - spoke.direction[0] * across,
                                ];
                                // No relief cuts a spoke (KiCad adds them after
                                // the reliefs: Hub's C35.1, its track ends in
                                // its spoke where it meets C30.1's, inside
                                // C30.1's relief).
                                crate::geometry::distance(centre, point) <= minimum / 2.0 + 1.0e-9
                                    && probe.blocker(record.layer, centre, None).is_none()
                            })
                        })
                    }) || probe.in_ring(record.layer, point)
                        || (probe.final_fill_at(record.layer, point)
                            && kept_piece(parent, record.layer, point))
                };
                // KiCad's fill keeps out of a thermal pad's plated hole (an
                // exposed pad's thermal via: a track to its centre ends in
                // no fill, 0xCB's U3.45).
                // The test is at a point, which KiCad's fill covers only
                // where it keeps the clearance to a micron or so
                // (`TRACK_END_TOLERANCE`).
                let in_fill = |parent: &mut Vec<usize>, point: crate::geometry::Point| {
                    !thermal_holes[record.layer]
                        .iter()
                        .any(|hole| self.board.obstacles[*hole].shape.contains(point))
                        && probe.for_track_end(|| in_fill(parent, point))
                };
                // A pad merged into its terminal has the tracks whose copper
                // overlaps its own (KiCad's connectivity: 0xCB's thermal
                // vias, which the exposed pad's tracks cross), not all its
                // terminal's.
                let merged_pad = record.pad != terminal.pad;
                let half_width = self.width_on(description.class, record.layer) / 2.0;
                let connected =
                    |a: crate::geometry::Point, b: crate::geometry::Point, radius: f64| {
                        !merged_pad || pad.shape.distance_to_segment(a, b) <= radius + 1.0e-6
                    };
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
                                    && crate::geometry::distance(other.anchor, record.anchor)
                                        < 1.0e-6
                            })
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
                    let mut segments: Vec<(crate::geometry::Point, crate::geometry::Point)> =
                        Vec::new();
                    let mut radius = half_width;
                    if let Some((start, end, adopted)) = state.adopted.get(&index) {
                        radius = *adopted;
                        if (at_pad(branch.start_terminal) || at_pad(branch.end_terminal))
                            && pad.layers & (1 << branch.nodes[0].layer) != 0
                        {
                            segments.push((*start, *end));
                        }
                    } else {
                        for (end, forward) in
                            [(branch.start_terminal, true), (branch.end_terminal, false)]
                        {
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
                                Some((_, _, polyline)) => segments
                                    .extend(polyline.windows(2).map(|pair| (pair[0], pair[1]))),
                                None if crate::geometry::distance(terminal.anchor, first)
                                    > 1.0e-7 =>
                                {
                                    segments.push((terminal.anchor, first))
                                }
                                None => {}
                            }
                            let step = |a: Node, b: Node| {
                                let (ax, ay) = self.grid.xy(a.cell as usize);
                                let (bx, by) = self.grid.xy(b.cell as usize);
                                (a.layer == b.layer)
                                    .then_some((bx as i64 - ax as i64, by as i64 - ay as i64))
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
                                segments
                                    .push((first, self.grid.center_of(nodes[last].cell as usize)));
                            }
                        }
                    }
                    for (a, b) in segments {
                        let counted = connected(a, b, radius)
                            && if near_pad(a) {
                                in_fill(parent, b)
                            } else {
                                near_pad(b) && in_fill(parent, a)
                            };
                        if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() && !counted {
                            eprintln!(
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
                                kept_piece(parent, record.layer, a),
                                kept_piece(parent, record.layer, b)
                            );
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
            }
            if !zones.is_empty() {
                for end in &track_ends {
                    for (position, zone) in zones.iter().enumerate() {
                        if in_zone(*zone, *end) {
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
                // KiCad's rule per zone: none of its fill across the ring
                // is no connection to it; tracks count with the spokes, and
                // its island spokes only when nothing else does.
                for (position, [spokes, islands, crossings, tracks]) in
                    zone_tallies.iter().copied().enumerate()
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
            if good + record.tracks < record.required {
                for piece in islands {
                    if !spoke_islands.iter().any(|(island, _)| *island == piece) {
                        spoke_islands.push((piece, record.terminal));
                    }
                }
            }
        }
        if let Some(report) = report {
            report.extend(records);
        }
        probe.charge();
        spoke_islands
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
        if let Some(frame) = self.board.pad_spokes.get(&pad).filter(|_| frames) {
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
            let mut pairs: HashMap<u32, Vec<(i64, (i64, i64), (i64, i64))>> = HashMap::new();
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
            let mut others: Vec<(u32, Vec<(i64, (i64, i64), (i64, i64))>)> =
                pairs.into_iter().collect();
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

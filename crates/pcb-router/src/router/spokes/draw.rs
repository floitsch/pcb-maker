// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The spokes KiCad draws: the items the pieces hold before any spoke,
//! the pads that get spokes on which layers, and each pad's spokes.

use super::*;

/// The spoke records of the net's pads and the spokes KiCad draws for
/// them (`Router::draw_spokes`).
pub(super) struct Drawn {
    /// Per pad and layer, its record (every outcome `NoFill` so far).
    pub(super) records: Vec<SpokeRecord>,
    /// The spokes KiCad draws, each naming its record.
    pub(super) spokes: Vec<Spoke>,
    /// Per record, whether its pad lies inside its terminal's pad.
    pub(super) inner: Vec<bool>,
}

impl Router {
    /// Items per piece (KiCad's CN_ZONE_LAYER::HasSingleConnection)
    /// before any spoke: each branch with a node in it (`branch_pieces`,
    /// less those `skip` names) and each pad joined solidly. The pads
    /// whose spokes land in a piece are added later (`touch_pieces`).
    pub(super) fn piece_items(
        &self,
        model: &SpokeNet,
        branch_pieces: &[Vec<(u32, usize)>],
    ) -> HashMap<u32, usize> {
        let SpokeNet {
            pours,
            state,
            skip,
            pitch,
            ..
        } = *model;
        let solid = &model.solid;
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
        // Each window nearest first, left at its first labelled node
        // (`piece_near` looked at every node of it, for every node of every
        // pad: a quarter of nrf24_remote's pour analyses).
        let near = crate::pour::NearOrder::new(reach);
        for (terminal, nodes) in state.terminal_nodes.iter().enumerate() {
            if model.isolated(terminal) {
                continue;
            }
            let mut seen: Vec<u32> = Vec::new();
            for node in nodes
                .iter()
                .filter(|node| solid[node.layer as usize] || model.solid_pad(terminal))
            {
                if let Some(piece) = pours.piece_near_ordered(
                    &self.grid,
                    node.layer as usize,
                    node.cell as usize,
                    &near,
                ) && !seen.contains(&piece)
                {
                    seen.push(piece);
                }
            }
            for piece in seen {
                *items.entry(piece).or_default() += 1;
            }
        }
        items
    }

    /// Per layer, the net's zones KiCad fills there when there are
    /// several (indexes into the board's planes); none where there is
    /// one or none.
    pub(super) fn zones_by_layer(&self, model: &SpokeNet) -> Vec<Vec<usize>> {
        let SpokeNet { net, layers, .. } = *model;
        // Its DRC checks a pad against each zone
        // apart (DRC_TEST_PROVIDER_ZONE_CONNECTIONS::testZoneLayer), with
        // the crossings of that zone's fill alone, and each zone's fill is
        // its own outline less those of the net's zones of a higher
        // priority (zone_filler.cpp, subtractHigherPriorityZones; the fill
        // up to the seam is the same, built over the neighbours' outlines
        // and then cut).
        (0..layers)
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
            .collect()
    }

    /// The pads whose spokes the model draws (`SpokePad`): each
    /// terminal's pad on the layers its nodes are on, then the pads merged
    /// into the terminals on their own layers.
    pub(super) fn spoke_pads(&self, model: &SpokeNet) -> Vec<SpokePad> {
        let SpokeNet {
            net,
            state,
            description,
            layers,
            ..
        } = *model;
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
        spoke_pads
    }

    /// Whether pad `pad_index` of `terminal` has no thermal spokes to
    /// check: its zone connection is solid or none.
    fn pad_without_spokes(&self, model: &SpokeNet, terminal: usize, pad_index: usize) -> bool {
        let SpokeNet {
            net, description, ..
        } = *model;
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
        // to solid inside a thermal exposed pad), in the measurement.
        let merged = self.board.merged_pads.iter().find(|merged| {
            merged.net == net
                && !merged.padstack
                && merged.pad == pad_index
                && merged.host == terminal
        });
        match merged {
            Some(merged) if self.config.measured_thermals => !self.merged_pad_thermal(merged),
            _ => {
                model.isolated(terminal)
                    || (model.solid_pad(terminal)
                        && (pad_index == description.terminals[terminal].pad
                            || self
                                .board
                                .merged_pads
                                .iter()
                                .any(|merged| merged.padstack && merged.pad == pad_index)))
            }
        }
    }

    /// Pass one's geometry: per pad of `spoke_pads` and layer where the
    /// zones it lies in are thermal, its record, and the four spokes KiCad
    /// draws for it unless they would be narrower than the minimum width.
    pub(super) fn draw_spokes(
        &self,
        model: &SpokeNet,
        probe: &fill::FillProbe,
        spoke_pads: Vec<SpokePad>,
    ) -> Drawn {
        let SpokeNet {
            net,
            pours,
            state,
            description,
            required,
            ..
        } = *model;
        let solid = &model.solid;
        let mut records: Vec<SpokeRecord> = Vec::new();
        let mut spokes: Vec<Spoke> = Vec::new();
        // Per record, whether its pad lies inside its terminal's pad.
        let mut record_inner: Vec<bool> = Vec::new();
        for (terminal, pad_index, pad_anchor, layers_of_nodes, inner) in spoke_pads {
            if self.pad_without_spokes(model, terminal, pad_index) {
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
        Drawn {
            records,
            spokes,
            inner: record_inner,
        }
    }
}

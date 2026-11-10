// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Where each spoke KiCad draws lands: in a piece of the fill, in another
//! pad's spoke, blocked by other nets' clearances, or in no fill.

use super::*;

/// What landing the spokes found besides their outcomes
/// (`Router::land_spokes`).
pub(super) struct Landed {
    /// Per record, the pads whose spokes met its own.
    pub(super) met_pads: Vec<Vec<usize>>,
    /// Per spoke, whether what is left of it joins the pad to the ring
    /// half a gap out.
    pub(super) attached: Vec<bool>,
}

impl Router {
    /// Pass one's landing: sets each spoke's outcome in `records`
    /// (`Good` holding the piece it lands in until the items are counted,
    /// `Pad`, `Blocked` or left `NoFill`) and returns the pads met and
    /// whether each spoke stays joined to its pad.
    pub(super) fn land_spokes(
        &self,
        model: &SpokeNet,
        probe: &fill::FillProbe,
        records: &mut [SpokeRecord],
        spokes: &[Spoke],
    ) -> Landed {
        let SpokeNet {
            state, description, ..
        } = *model;
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
            let end = spoke.end();
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
                        probe.quietly(|| probe.blocker(layer, end, Some(&[])))
                    );
                }
                if let Some(other) = self.meeting_spoke(probe, records, spokes, spoke, from) {
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
            // In the measurement, a spoke the clearances cut short of that
            // still counts where what is left of it joins the fill beside
            // it (`spoke_joins_fill`; 0xCB's U3.23 on B.Cu).
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
                && !(self.config.measured_thermals
                    && probe.spoke_joins_fill(
                        layer,
                        spoke.anchor,
                        spoke.direction,
                        from,
                        clear_to,
                        spoke.side,
                    ))
            {
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                    let label = &description.terminals[records[spoke.record].terminal].label;
                    debug_blocked_spoke(probe, label, spoke, from);
                }
                records[spoke.record].outcomes[spoke.index] = SpokeOutcome::Blocked;
                continue;
            }
            // Piece 0 stands for a sliver the lattice misses: the spoke
            // counts only where a track of the pad touches it too.
            records[spoke.record].outcomes[spoke.index] =
                SpokeOutcome::Good(self.landing_piece(model, probe, layer, end, minimum));
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
                        probe.quietly(|| probe.blocker(layer, at, None))
                    );
                }
            }
        }
        Landed { met_pads, attached }
    }

    /// The spoke of another pad that `spoke`, whose end lands in no fill,
    /// meets (each end in the other's strip), where the brush keeps room
    /// along `spoke` from `from` to its tip; None where none does.
    fn meeting_spoke<'s>(
        &self,
        probe: &fill::FillProbe,
        records: &[SpokeRecord],
        spokes: &'s [Spoke],
        spoke: &Spoke,
        from: f64,
    ) -> Option<&'s Spoke> {
        let layer = spoke.layer;
        let end = spoke.end();
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
                && crate::geometry::distance(other.anchor, spoke.anchor) <= other.tip + spoke.tip
                && other.contains(end)
                && spoke.contains(other.end())
        });
        meets.filter(|_| {
            !self.config.spoke_clearance
                || probe.spoke_clear(
                    layer,
                    spoke.anchor,
                    spoke.direction,
                    from,
                    spoke.tip,
                    spoke.side,
                )
        })
    }

    /// The piece of the pour map on `layer` that a spoke ending at `end`
    /// joins (`minimum` the zone's minimum width): the nearest labelled
    /// node, else one a straight run of fill joins to the end; 0 for none.
    fn landing_piece(
        &self,
        model: &SpokeNet,
        probe: &fill::FillProbe,
        layer: usize,
        end: crate::geometry::Point,
        minimum: f64,
    ) -> u32 {
        let pitch = model.pitch;
        // The piece it joins: the nearest of the pour map's (none for
        // a sliver the lattice misses: a piece of its own).
        let radius = minimum / 2.0 + pitch;
        let mut best = self.nearest_piece(model, layer, end, radius);
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
                        if model.label_at(layer, cell) != 0 && distance <= far {
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
                    .then(|| (distance, model.label_at(layer, cell)))
            });
        }
        best.map_or(0, |(_, piece)| piece)
    }
}

/// Prints where along `spoke` (of pad `label`), from `from` to its tip,
/// other nets' copper blocks the brush (`PCB_ROUTER_SPOKE_DEBUG`).
fn debug_blocked_spoke(probe: &fill::FillProbe, label: &str, spoke: &Spoke, from: f64) {
    let layer = spoke.layer;
    let steps = 8;
    for step in 0..=steps {
        let along = from + (spoke.tip - from) * step as f64 / steps as f64;
        let at = [
            spoke.anchor[0] + spoke.direction[0] * along,
            spoke.anchor[1] + spoke.direction[1] * along,
        ];
        if let Some(why) = probe.quietly(|| probe.blocker(layer, at, None)) {
            eprintln!(
                "blocked spoke {} layer {layer} dir {:?} width {:.3} side {:.3} at {along:.3} ({:.3}..{:.3}): {why}",
                label, spoke.direction, spoke.width, spoke.side, from, spoke.tip
            );
            break;
        }
    }
}

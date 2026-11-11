// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The pour analysis: which pads a net's pours join to their main piece.
//!
//! `Router::analyze_pours_uncounted` runs in stages: the free fill and its
//! pieces (`free`), then the joins of the pads and branches to them and
//! the main piece (`joins`).

use super::*;

mod free;
mod joins;

use free::Fill;
use joins::{Joins, PourPads, main_piece};

impl Router {
    /// Which pour piece every terminal and branch of `net` belongs to.
    /// Returns the pour map, a union-find over `pieces + terminals +
    /// branches` elements (piece labels start at 1; element 0 is unused),
    /// and the root of the main piece (the one holding most terminals).
    pub(super) fn analyze_pours(&self, net: NetId) -> (crate::pour::PourMap, Vec<usize>, usize) {
        self.analyze_pours_skipping(net, &|_| false)
    }

    /// `analyze_pours` leaving out the branches `skip` names (by index):
    /// what the pour and the other branches join without them.
    pub(super) fn analyze_pours_skipping(
        &self,
        net: NetId,
        skip: &dyn Fn(usize) -> bool,
    ) -> (crate::pour::PourMap, Vec<usize>, usize) {
        let started = std::time::Instant::now();
        let result = self.analyze_pours_skipping_untimed(net, skip);
        self.profile.analysis_nanos.fetch_add(
            started.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.profile
            .analysis_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        result
    }

    pub(super) fn analyze_pours_skipping_untimed(
        &self,
        net: NetId,
        skip: &dyn Fn(usize) -> bool,
    ) -> (crate::pour::PourMap, Vec<usize>, usize) {
        let (pours, parent, main, _) = self.analyze_pours_full(net, skip);
        (pours, parent, main)
    }

    /// `analyze_pours_skipping` with the spoke islands too: the pieces
    /// without copper of the net in them that a pad's thermal spoke
    /// reaches, each with one such pad. KiCad removes the piece and does
    /// not count the spoke (Castor: RV8 with one spoke into the fill and
    /// one into a 15 mm² piece, "1 spokes connected to isolated island").
    pub(super) fn analyze_pours_full(
        &self,
        net: NetId,
        skip: &dyn Fn(usize) -> bool,
    ) -> (crate::pour::PourMap, Vec<usize>, usize, Vec<(usize, usize)>) {
        self.analyze_pours_reporting(net, skip, None)
    }

    /// `analyze_pours_full`, recording each pad's spokes in `report` when
    /// given (for `dump_pours`).
    ///
    /// Every analysis counts as work (`pour_analysis_nanos`).
    pub(super) fn analyze_pours_reporting(
        &self,
        net: NetId,
        skip: &dyn Fn(usize) -> bool,
        report: Option<&mut Vec<SpokeRecord>>,
    ) -> (crate::pour::PourMap, Vec<usize>, usize, Vec<(usize, usize)>) {
        let started = std::time::Instant::now();
        let ops_before = self
            .scratch
            .probe_ops
            .load(std::sync::atomic::Ordering::Relaxed);
        let probe_before = self.probe_nanos();
        let counts_before = (
            self.scratch
                .probe_queries
                .load(std::sync::atomic::Ordering::Relaxed),
            self.scratch
                .probe_vertices
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        let result = self.analyze_pours_uncounted(net, skip, report);
        let units = self.pour_analysis_nanos(net);
        self.scratch
            .pour_units
            .fetch_add(units, std::sync::atomic::Ordering::Relaxed);
        self.scratch.pour_nanos.fetch_add(
            started.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        if std::env::var_os("PCB_ROUTER_POUR_TRACE").is_some() {
            // For calibrating the work's constants: the counts beside the
            // wall time.
            let state = &self.nets[net as usize];
            eprintln!(
                "pour trace: inside {} stamped {} probe items {} work {units} ns, wall {} ns, layer nodes {} probe queries {} probe vertices {} probe work {} ns hatched {}",
                self.pour_inside(net),
                state.stamped.len(),
                self.scratch
                    .probe_ops
                    .load(std::sync::atomic::Ordering::Relaxed)
                    - ops_before,
                started.elapsed().as_nanos(),
                self.pour_layer_nodes(net),
                self.scratch
                    .probe_queries
                    .load(std::sync::atomic::Ordering::Relaxed)
                    - counts_before.0,
                self.scratch
                    .probe_vertices
                    .load(std::sync::atomic::Ordering::Relaxed)
                    - counts_before.1,
                self.probe_nanos() - probe_before,
                self.board
                    .planes
                    .iter()
                    .any(|plane| plane.net == net && plane.hatch.is_some()),
            );
        }
        result
    }

    /// The work of a pour analysis of `net` but its fill probe's, in
    /// nanoseconds of an idle machine: the lattice nodes inside its pour
    /// (the fill mask and its labelling) and its stamps.
    pub(super) fn pour_analysis_nanos(&self, net: NetId) -> u64 {
        self.pour_inside(net) as u64 * POUR_NODE_NANOS
            + self.nets[net as usize].stamped.len() as u64 * POUR_STAMP_NANOS
    }

    /// Lattice nodes of the layers `net` pours on (the fill mask and the
    /// labelling scan every node of such a layer).
    pub(super) fn pour_layer_nodes(&self, net: NetId) -> usize {
        self.nets[net as usize]
            .plane
            .iter()
            .filter(|mask| !mask.is_empty())
            .count()
            * self.grid.cells()
    }

    /// Lattice nodes inside the pour of `net`, over its layers.
    pub(super) fn pour_inside(&self, net: NetId) -> usize {
        self.nets[net as usize]
            .plane
            .iter()
            .map(|mask| mask.iter().filter(|inside| **inside).count())
            .sum()
    }

    /// `analyze_pours_full` without the work's count: the free fill and
    /// its pieces (`free`), the pads and branches joined to them and the
    /// main piece (`joins`).
    pub(super) fn analyze_pours_uncounted(
        &self,
        net: NetId,
        skip: &dyn Fn(usize) -> bool,
        report: Option<&mut Vec<SpokeRecord>>,
    ) -> (crate::pour::PourMap, Vec<usize>, usize, Vec<(usize, usize)>) {
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        let own = self.own_pour_stamps(state);
        let fill = self.free_fill(net, &own, skip);
        let pours = self.fill_pieces(net, &own, &fill);
        let Fill {
            reclaimed, rings, ..
        } = fill;
        let branch_pieces = self.branch_pieces(net, &pours, skip);
        let terminal_count = state.terminal_nodes.len();
        let mut joins = Joins::new(pours.pieces, terminal_count, state.branches.len());
        // A pad joins a pour piece the way KiCad joins it: through a
        // thermal spoke, straight out from the pad (along its axes, or
        // diagonally for a round pad), across the thermal gap into fill. A
        // radius around the pad counted fill that only lies diagonally
        // beside a fine-pitch pad row, which KiCad leaves unconnected.
        // A solid pour (KiCad's `connect_pads yes`) joins a pad wherever
        // its fill comes close.
        let pads = PourPads {
            net,
            board: &self.board,
            state,
            description: &self.board.nets[net as usize],
            solid: (0..layers)
                .map(|layer| {
                    self.board
                        .planes
                        .iter()
                        .any(|plane| plane.net == net && plane.layer == layer && plane.solid)
                })
                .collect(),
        };
        // Pieces with copper of the net in them: a track or via, or a pad
        // the fill touches. KiCad keeps a fill piece only with such an item
        // in it; a piece joined to the net by thermal spokes alone is an
        // island, removed with its spokes (k30-SBC: 8 pads with "spokes
        // connected to isolated island", each in a 1-35 mm² piece of its own
        // and connected by a track; a via put in the piece cures it). Such
        // a piece gets no connectivity from the pads it spokes to; the pads
        // get none from it.
        //
        // A solid connection joins the pad to the pieces whose fill reaches
        // its copper (KiCad cuts no relief: the fill runs onto the pad),
        // not to every piece near it: ORBIT's U2.14 (connect_pads yes,
        // clearance 0.5 mm) read as joined to a piece 0.3 mm past its edge,
        // beyond the clearance of the pads beside it, and its island of
        // U2's ground pads as joined to the rest; KiCad's fill keeps them
        // apart and reported them unconnected.
        let mut touched = self.join_solid_pads(&pads, &pours, &mut joins, &rings, reclaimed);
        self.join_branches(state, &mut joins, &branch_pieces, skip, &mut touched);
        let spoke_islands = if self.config.kicad_spokes {
            self.kicad_spokes(
                net,
                &pours,
                &mut joins.parent,
                joins.terminal_base,
                skip,
                &rings,
                &branch_pieces,
                report,
            )
        } else {
            self.straight_spokes(&pads, &pours, &own, &mut joins, &touched, report)
        };
        let largest = main_piece(
            &mut joins.parent,
            &pours.label,
            joins.terminal_base,
            terminal_count,
        );
        (pours, joins.parent, largest, spoke_islands)
    }

    /// Whether the fill of the piece holding free node `cell` on `layer`
    /// reaches a solidly connected pad (`shape`, its centre `anchor`;
    /// `distance` from the node to its copper): the brush at the node
    /// covers the pad's edge, or a straight run of fill (the probe's,
    /// off the lattice) leads from the node to it. Within two pitches
    /// beyond the brush the lattice cannot tell; farther, the fill does
    /// not reach.
    pub(super) fn solid_fill_reaches(
        &self,
        probe: &fill::FillProbe,
        layer: usize,
        shape: &crate::geometry::Shape,
        anchor: crate::geometry::Point,
        cell: usize,
        distance: f64,
    ) -> bool {
        let half = probe.brush_half(layer);
        if distance <= half + 1.0e-9 {
            return true;
        }
        let pitch = self.grid.pitch;
        if distance > half + 2.0 * pitch || shape.distance_to_point(anchor) > 0.0 {
            return false;
        }
        let at = self.grid.center_of(cell);
        let point = |t: f64| {
            [
                at[0] + (anchor[0] - at[0]) * t,
                at[1] + (anchor[1] - at[1]) * t,
            ]
        };
        // Where the run towards the pad's centre enters its copper.
        let (mut outside, mut inside) = (0.0f64, 1.0f64);
        for _ in 0..24 {
            let middle = (outside + inside) / 2.0;
            if shape.distance_to_point(point(middle)) > 0.0 {
                outside = middle;
            } else {
                inside = middle;
            }
        }
        let entry = point(inside);
        let length = crate::geometry::distance(at, entry);
        let steps = ((length / (pitch / 4.0)).ceil() as usize).max(1);
        (0..=steps).all(|step| {
            let t = step as f64 / steps as f64;
            probe.fill_at(
                layer,
                [
                    at[0] + (entry[0] - at[0]) * t,
                    at[1] + (entry[1] - at[1]) * t,
                ],
            )
        })
    }
}

#[cfg(test)]
mod tests;

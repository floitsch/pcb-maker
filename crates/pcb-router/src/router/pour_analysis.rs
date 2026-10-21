// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The pour analysis: which pads a net's pours join to their main piece.

use super::*;

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

    pub(super) fn analyze_pours_uncounted(
        &self,
        net: NetId,
        skip: &dyn Fn(usize) -> bool,
        mut report: Option<&mut Vec<SpokeRecord>>,
    ) -> (crate::pour::PourMap, Vec<usize>, usize, Vec<(usize, usize)>) {
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        let own = {
            let maps: Vec<usize> = (0..layers)
                .filter(|layer| !state.plane[*layer].is_empty())
                .flat_map(|layer| {
                    let class = state.plane_class[layer];
                    [
                        self.map_index(class, layer),
                        self.diagonal_map(class, layer, 0),
                        self.diagonal_map(class, layer, 1),
                    ]
                })
                .collect();
            OwnStamps::new(&state.stamped, &maps, self.grid.cells())
        };
        let mut free: Vec<Vec<bool>> = (0..layers)
            .map(|layer| {
                let mask = &state.plane[layer];
                if mask.is_empty() {
                    return Vec::new();
                }
                let map = self.map_index(state.plane_class[layer], layer);
                (0..mask.len())
                    .map(|cell| {
                        mask[cell]
                            && self.occupancy[map][cell] as usize
                                == own.contains(map, cell) as usize
                    })
                    .collect()
            })
            .collect();
        let rings = self.thermal_ring_pads(net);
        self.cut_thermal_rings(&rings, &mut free);
        self.cut_thermal_rings(&self.isolated_knockouts(net), &mut free);
        self.cut_via_reliefs(net, &mut free);
        // The net's own tracks are copper of the net, and the fill joins
        // them anywhere, inside a pad's thermal ring too (a plane stub ends
        // in the ring: Castor's U7.3 read as stranded with "1 branch, 0
        // pieces" and was rerouted and stitched for nothing).
        for (index, branch) in state.branches.iter().enumerate() {
            if skip(index) {
                continue;
            }
            for node in &branch.nodes {
                let layer = node.layer as usize;
                if free[layer].is_empty() {
                    continue;
                }
                let cell = node.cell as usize;
                let map = self.map_index(state.plane_class[layer], layer);
                if state.plane[layer][cell]
                    && self.occupancy[map][cell] as usize == own.contains(map, cell) as usize
                {
                    free[layer][cell] = true;
                }
            }
        }
        // Two free nodes diagonal to each other whose common neighbours are
        // both taken join when the brush can go straight from one to the
        // other: no other net's copper, fixed or routed, and no thermal ring
        // comes closer to the step than to the nodes. A neck of KiCad's
        // fill running diagonally to the lattice holds no two nodes side by
        // side; labelled 4-connected, k30-SBC's +5V plane on In1 fell into
        // 674 pieces where KiCad's fill has 178 outlines (with the diagonal
        // steps 183; GND on F.Cu 420, 298 and 306).
        let diagonal =
            |layer: usize, start: usize, orientation: usize| {
                let class = state.plane_class[layer];
                let direction = if orientation == 0 { 1 } else { 7 };
                if !self.statics[class].edge_allowed(layer, start, direction, net) {
                    return false;
                }
                let map = self.diagonal_map(class, layer, orientation);
                if self.occupancy[map][start] as usize != own.contains(map, start) as usize {
                    return false;
                }
                let Some(end) = self.grid.neighbor(start, direction) else {
                    return false;
                };
                let (a, b) = (self.grid.center_of(start), self.grid.center_of(end));
                let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                !rings[layer].iter().any(|(pad, reach)| {
                    let shape = &self.board.obstacles[*pad].shape;
                    shape.aabb().inflated(*reach + self.grid.pitch).intersects(
                        crate::geometry::Aabb {
                            minimum: middle,
                            maximum: middle,
                        },
                    ) && shape.distance_to_segment(a, b) < *reach
                })
            };
        let pours = crate::pour::PourMap::build(&self.grid, &free, &diagonal);
        let branch_pieces = self.branch_pieces(net, &pours, skip);
        let terminal_count = state.terminal_nodes.len();
        let terminal_base = pours.pieces + 1;
        let branch_base = terminal_base + terminal_count;
        let mut parent: Vec<usize> = (0..branch_base + state.branches.len()).collect();
        let reach = (0.8 / self.grid.pitch).ceil() as usize;
        let union = |parent: &mut Vec<usize>, a: usize, b: usize| {
            let (a, b) = (find(parent, a), find(parent, b));
            parent[a] = b;
        };
        // A pad joins a pour piece the way KiCad joins it: through a
        // thermal spoke, straight out from the pad (along its axes, or
        // diagonally for a round pad), across the thermal gap into fill. A
        // radius around the pad counted fill that only lies diagonally
        // beside a fine-pitch pad row, which KiCad leaves unconnected.
        // A solid pour (KiCad's `connect_pads yes`) joins a pad wherever
        // its fill comes close.
        let solid: Vec<bool> = (0..layers)
            .map(|layer| {
                self.board
                    .planes
                    .iter()
                    .any(|plane| plane.net == net && plane.layer == layer && plane.solid)
            })
            .collect();
        let description = &self.board.nets[net as usize];
        // Per pad, KiCad's own zone connection overrides the pour's.
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
        let mut touched: Vec<usize> = Vec::new();
        let probe = fill::FillProbe::new(self, net, &rings);
        for (terminal, nodes) in state.terminal_nodes.iter().enumerate() {
            if isolated(terminal) {
                continue;
            }
            let pad = &self.board.obstacles[description.terminals[terminal].pad];
            let anchor = description.terminals[terminal].anchor;
            // Per layer and piece, its node nearest the pad.
            let mut candidates: Vec<(usize, u32, usize, f64)> = Vec::new();
            for node in nodes
                .iter()
                .filter(|node| solid[node.layer as usize] || solid_pad(terminal))
            {
                let layer = node.layer as usize;
                for (piece, cell) in pours.pieces_near(&self.grid, layer, node.cell as usize, reach)
                {
                    let distance = pad.shape.distance_to_point(self.grid.center_of(cell));
                    match candidates
                        .iter_mut()
                        .find(|(other_layer, other, _, _)| *other_layer == layer && *other == piece)
                    {
                        Some(entry) if distance < entry.3 => {
                            *entry = (layer, piece, cell, distance)
                        }
                        Some(_) => {}
                        None => candidates.push((layer, piece, cell, distance)),
                    }
                }
            }
            for (layer, piece, cell, distance) in candidates {
                if self.solid_fill_reaches(&probe, layer, &pad.shape, anchor, cell, distance) {
                    union(&mut parent, terminal_base + terminal, piece as usize);
                    touched.push(piece as usize);
                }
            }
        }
        probe.charge();
        let keep = vec![true; state.branches.len()];
        let hosts = junction_hosts(&state.branches, &keep);
        for (index, branch) in state.branches.iter().enumerate() {
            if skip(index) {
                continue;
            }
            for (piece, _) in &branch_pieces[index] {
                union(&mut parent, branch_base + index, *piece as usize);
                touched.push(*piece as usize);
            }
            for (terminal, node) in [
                (branch.start_terminal, branch.nodes[0]),
                (branch.end_terminal, *branch.nodes.last().unwrap()),
            ] {
                if terminal == NO_TERMINAL {
                    if let Some(host) = hosts.get(&node)
                        && !skip(*host)
                    {
                        union(&mut parent, branch_base + index, branch_base + host);
                    }
                } else if terminal != PLANE_TERMINAL && terminal != FREE_END {
                    union(
                        &mut parent,
                        branch_base + index,
                        terminal_base + terminal as usize,
                    );
                }
            }
        }
        let spoke_islands = if self.config.kicad_spokes {
            self.kicad_spokes(
                net,
                &pours,
                &mut parent,
                terminal_base,
                skip,
                &rings,
                &branch_pieces,
                report,
            )
        } else {
            let with_items: HashSet<usize> = touched
                .iter()
                .map(|piece| find(&mut parent, *piece))
                .collect();
            let has_item =
                |parent: &mut Vec<usize>, piece: usize| with_items.contains(&find(parent, piece));
            let mut spoke_islands: Vec<(usize, usize)> = Vec::new();
            for (terminal, nodes) in state.terminal_nodes.iter().enumerate() {
                if isolated(terminal) || solid_pad(terminal) {
                    continue;
                }
                let pad = &self.board.obstacles[description.terminals[terminal].pad];
                let anchor = description.terminals[terminal].anchor;
                let bounds = pad.shape.aabb();
                let round = matches!(pad.shape, crate::geometry::Shape::Circle { .. });
                // In layer order: a hash set's order made the union order, and
                // so the roots, the main piece's tie-break and the stitching
                // vias' anchors, differ from run to run.
                let layers_of_nodes: std::collections::BTreeSet<usize> =
                    nodes.iter().map(|node| node.layer as usize).collect();
                for layer in layers_of_nodes {
                    if pours.label[layer].is_empty() || solid[layer] {
                        continue;
                    }
                    let gap = self
                        .board
                        .planes
                        .iter()
                        .filter(|plane| plane.net == net && plane.layer == layer)
                        .map(|plane| plane.thermal_reach)
                        .fold(0.0f64, f64::max)
                        .max(self.grid.pitch);
                    let half = [
                        (bounds.maximum[0] - bounds.minimum[0]) / 2.0,
                        (bounds.maximum[1] - bounds.minimum[1]) / 2.0,
                    ];
                    let directions: [[f64; 2]; 4] = if round {
                        let d = std::f64::consts::FRAC_1_SQRT_2;
                        [[d, d], [d, -d], [-d, d], [-d, -d]]
                    } else {
                        [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]]
                    };
                    // KiCad wants two spokes into kept fill on the layer; a
                    // spoke into an island matters only to a pad short of them.
                    let mut good = 0;
                    let mut islands: Vec<usize> = Vec::new();
                    let mut outcomes = [SpokeOutcome::NoFill; 4];
                    // The spoke's strip, as far as the brush's map tells: its
                    // middle and the lines half a spoke less half a brush to
                    // either side (a brush centre there keeps the clearance
                    // the spoke's edge needs).
                    let bridge = self
                        .board
                        .planes
                        .iter()
                        .filter(|plane| plane.net == net && plane.layer == layer)
                        .map(|plane| (plane.thermal_reach - plane.thermal_gap).max(0.0))
                        .fold(0.0f64, f64::max);
                    let side =
                        ((bridge - self.board.classes[state.plane_class[layer]].trace_width) / 2.0)
                            .max(0.0);
                    let plane_map = self.map_index(state.plane_class[layer], layer);
                    let clear = |cell: usize| {
                        state.plane[layer][cell]
                            && self.occupancy[plane_map][cell] as usize
                                == own.contains(plane_map, cell) as usize
                    };
                    for (direction_index, direction) in directions.into_iter().enumerate() {
                        // From the pad's edge along the spoke, the fill must
                        // begin within the gap and a little more.
                        let edge = if round {
                            half[0]
                        } else {
                            (half[0] * direction[0]).abs() + (half[1] * direction[1]).abs()
                        };
                        for extra in [gap, gap + self.grid.pitch] {
                            let distance = edge + extra;
                            let point = [
                                anchor[0] + direction[0] * distance,
                                anchor[1] + direction[1] * distance,
                            ];
                            let Some(cell) = self.grid.nearest_node(point) else {
                                continue;
                            };
                            let piece = pours.label[layer][cell];
                            // KiCad keeps a spoke only where it keeps the
                            // clearance all the way from the pad to the fill:
                            // on a fine-pitch row the neighbours' clearance
                            // cuts it (k30's U2.7: fill 0.7 mm right of the pad
                            // counted as a spoke past pad 6's corner, 0.2 mm
                            // from the spoke; KiCad left the pad unconnected).
                            if piece != 0 && self.config.spoke_clearance {
                                let steps =
                                    ((extra / (self.grid.pitch / 2.0)).ceil() as usize).max(1);
                                let blocked = (1..=steps).any(|step| {
                                    let along = edge + extra * step as f64 / steps as f64;
                                    [-side, 0.0, side].iter().any(|offset| {
                                        let at = [
                                            anchor[0] + direction[0] * along
                                                - direction[1] * offset,
                                            anchor[1]
                                                + direction[1] * along
                                                + direction[0] * offset,
                                        ];
                                        self.grid.nearest_node(at).is_none_or(|cell| !clear(cell))
                                    })
                                });
                                if blocked {
                                    outcomes[direction_index] = SpokeOutcome::Blocked;
                                    break;
                                }
                            }
                            if piece != 0 {
                                if has_item(&mut parent, piece as usize) {
                                    union(&mut parent, terminal_base + terminal, piece as usize);
                                    good += 1;
                                    outcomes[direction_index] = SpokeOutcome::Good(piece);
                                } else {
                                    islands.push(piece as usize);
                                    outcomes[direction_index] = SpokeOutcome::Island(piece);
                                }
                                break;
                            }
                        }
                    }
                    if let Some(report) = report.as_deref_mut() {
                        report.push(SpokeRecord {
                            terminal,
                            pad: description.terminals[terminal].pad,
                            anchor,
                            crossings: 0,
                            layer,
                            round,
                            outcomes,
                            tracks: 0,
                            seam_short: 0,
                            seam_zone: None,
                            drawn: true,
                        });
                    }
                    if good < 2 {
                        for piece in islands {
                            if !spoke_islands.iter().any(|(island, _)| *island == piece) {
                                spoke_islands.push((piece, terminal));
                            }
                        }
                    }
                }
            }
            spoke_islands
        };
        let mut votes: HashMap<usize, usize> = HashMap::new();
        for terminal in 0..terminal_count {
            *votes
                .entry(find(&mut parent, terminal_base + terminal))
                .or_default() += 1;
        }
        let most = votes.values().copied().max().unwrap_or(0);
        let mut leaders: Vec<usize> = votes
            .iter()
            .filter(|(_, count)| **count == most)
            .map(|(root, _)| *root)
            .collect();
        leaders.sort_unstable();
        // Equal votes: the component with the most fill.
        let largest = if leaders.len() > 1 {
            let mut area: HashMap<usize, usize> = HashMap::new();
            let mut root_of: HashMap<u32, usize> = HashMap::new();
            for labels in &pours.label {
                for label in labels.iter().filter(|label| **label != 0) {
                    let root = *root_of
                        .entry(*label)
                        .or_insert_with(|| find(&mut parent, *label as usize));
                    *area.entry(root).or_default() += 1;
                }
            }
            leaders
                .iter()
                .copied()
                .max_by_key(|root| (area.get(root).copied().unwrap_or(0), usize::MAX - *root))
                .unwrap_or(0)
        } else {
            leaders.first().copied().unwrap_or(0)
        };
        (pours, parent, largest, spoke_islands)
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

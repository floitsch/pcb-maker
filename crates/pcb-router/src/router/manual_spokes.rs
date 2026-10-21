// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Manual and inner spokes for starved thermal pads, and their rings.

use super::*;

impl Router {
    /// For each pad the spoke model finds a starved thermal (fewer than
    /// two spokes as KiCad counts them), straight tracks from the pad into
    /// the main piece of its pour, as many as it is short, each on free
    /// lattice nodes (the net's own rules, no other net's copper): KiCad's
    /// DRC counts a track that starts within half a gap of the pad and
    /// ends in fill as a spoke, and a track keeps the net class's
    /// clearance, often less than the zone's (Castor's C49.2 and U14.2,
    /// in a field of other nets' pads that the fill keeps 0.5 mm from).
    /// Returns the tracks added.
    pub fn add_manual_spokes(&mut self) -> usize {
        let mut added = 0;
        // Every net with a pour, those routed as tracks too (the tracks
        // rung): KiCad fills their zones all the same, with the masks
        // set for the analysis only.
        let nets: Vec<NetId> = (0..self.nets.len() as NetId)
            .filter(|net| {
                self.nets[*net as usize].routable
                    && self
                        .board
                        .planes
                        .iter()
                        .any(|plane| plane.net == *net && plane.thermal_gap > 0.0)
            })
            .collect();
        for net in nets {
            let lent = self.nets[net as usize].plane.is_empty();
            if lent {
                let (plane, plane_class) = self.pour_masks(net, false);
                let state = &mut self.nets[net as usize];
                state.plane = plane;
                state.plane_class = plane_class;
            }
            added += self.add_manual_spokes_of(net);
            added += self.add_inner_spokes(net);
            if lent {
                let class = self.board.nets[net as usize].class;
                let state = &mut self.nets[net as usize];
                state.plane = Vec::new();
                state.plane_class = vec![class; self.board.layer_count];
            }
        }
        added
    }

    /// For each pad merged into its terminal's larger pad (an exposed
    /// pad's thermal via) that the spoke model finds starved on a layer
    /// where it lies inside that pad: a straight track of the net's width
    /// from its centre to fill over the larger pad (in a hatched zone its
    /// own ring), its copper wholly inside the larger pad's, the shortest
    /// of sixteen directions. KiCad's DRC counts a track connected to the
    /// pad that starts in its ring and ends in fill as a spoke; inside the
    /// larger pad's copper it keeps every clearance that pad keeps. Kept
    /// only if the model then finds the pad no longer short (0xCB's U3.45
    /// corner vias, whose ring and outer spokes the QFN's pins take).
    pub(super) fn add_inner_spokes(&mut self, net: NetId) -> usize {
        let mut added = 0;
        let mut tried: HashSet<(usize, usize)> = HashSet::new();
        loop {
            let mut report = Vec::new();
            self.analyze_pours_reporting(net, &|_| false, Some(&mut report));
            let description = &self.board.nets[net as usize];
            // The pad of its number it lies in on the record's layer: its
            // terminal's, or another pad merged into that terminal (an
            // exposed pad's copper on the far side: ray4477's U1.33 vias in
            // the 2.5 mm pad on B.Cu, the 3.6 mm terminal on F.Cu).
            let container = |record: &SpokeRecord| -> Option<usize> {
                let terminal_pad = description.terminals[record.terminal].pad;
                if record.pad == terminal_pad {
                    return None;
                }
                let holds = |pad: usize| {
                    let obstacle = &self.board.obstacles[pad];
                    obstacle.layers & (1 << record.layer) != 0
                        && obstacle.shape.contains(record.anchor)
                };
                if holds(terminal_pad) {
                    return Some(terminal_pad);
                }
                self.board
                    .merged_pads
                    .iter()
                    .filter(|other| {
                        other.net == net
                            && other.host == record.terminal
                            && other.pad != record.pad
                            && holds(other.pad)
                    })
                    .map(|other| other.pad)
                    .max_by(|a, b| {
                        let area = |pad: usize| {
                            let bounds = self.board.obstacles[pad].shape.aabb();
                            (bounds.maximum[0] - bounds.minimum[0])
                                * (bounds.maximum[1] - bounds.minimum[1])
                        };
                        area(*a).total_cmp(&area(*b))
                    })
            };
            let Some((record, host)) = report.iter().find_map(|record| {
                (Self::spokes_short(record) > 0 && !tried.contains(&(record.pad, record.layer)))
                    .then(|| container(record).map(|host| (record, host)))
                    .flatten()
            }) else {
                break;
            };
            tried.insert((record.pad, record.layer));
            let (layer, anchor, pad, short) = (
                record.layer,
                record.anchor,
                record.pad,
                Self::spokes_short(record),
            );
            let host = &self.board.obstacles[host];
            let width = self.board.classes[description.class].trace_width;
            let gap = self
                .board
                .planes
                .iter()
                .filter(|plane| plane.net == net && plane.layer == layer)
                .map(|plane| plane.thermal_gap)
                .fold(0.0f64, f64::max);
            let bridge = self
                .board
                .planes
                .iter()
                .filter(|plane| plane.net == net && plane.layer == layer)
                .map(|plane| (plane.thermal_reach - plane.thermal_gap).max(0.0))
                .fold(0.0f64, f64::max);
            let bounds = self.board.obstacles[pad].shape.aabb();
            let radius = 0.5
                * (bounds.maximum[0] - bounds.minimum[0])
                    .max(bounds.maximum[1] - bounds.minimum[1]);
            // Copper wholly inside the larger pad: its ends and sides.
            let inside_host = |a: crate::geometry::Point, b: crate::geometry::Point| {
                let length = crate::geometry::distance(a, b).max(1.0e-9);
                let d = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
                let h = width / 2.0;
                let steps = ((length / (h / 2.0)).ceil() as usize).max(1);
                (0..=steps).all(|step| {
                    let t = length * step as f64 / steps as f64;
                    let at = [a[0] + d[0] * t, a[1] + d[1] * t];
                    [[-d[1] * h, d[0] * h], [d[1] * h, -d[0] * h], [0.0, 0.0]]
                        .iter()
                        .all(|offset| host.shape.contains([at[0] + offset[0], at[1] + offset[1]]))
                }) && host.shape.contains([b[0] + d[0] * h, b[1] + d[1] * h])
            };
            let candidate = {
                let rings = self.thermal_ring_pads(net);
                let probe = fill::FillProbe::new(self, net, &rings);
                let holes: Vec<&crate::board::Obstacle> = self
                    .board
                    .obstacles
                    .iter()
                    .filter(|obstacle| {
                        obstacle.kind == crate::board::ObstacleKind::Hole
                            && obstacle.net == Some(net)
                    })
                    .collect();
                let mut best: Option<(f64, crate::geometry::Point)> = None;
                // From the ring half a gap out to the far side of a
                // hatched zone's ring.
                let (near, far) = (
                    radius + gap / 2.0 + width / 2.0,
                    radius + gap + bridge.max(width),
                );
                for index in 0..16 {
                    let angle = std::f64::consts::TAU * index as f64 / 16.0;
                    let direction = [angle.cos(), angle.sin()];
                    let steps = 8;
                    for step in 0..=steps {
                        let length = near + (far - near) * step as f64 / steps as f64;
                        let end = [
                            anchor[0] + direction[0] * length,
                            anchor[1] + direction[1] * length,
                        ];
                        if best.is_some_and(|(best, _)| best <= length) {
                            break;
                        }
                        if holes.iter().any(|hole| hole.shape.contains(end))
                            || !inside_host(anchor, end)
                        {
                            continue;
                        }
                        if probe.for_track_end(|| {
                            probe.in_ring(layer, end) || probe.final_fill_at(layer, end)
                        }) {
                            best = Some((length, end));
                            break;
                        }
                    }
                }
                best.map(|(_, end)| end)
            };
            if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                eprintln!(
                    "inner spoke for pad {pad} layer {layer} at {anchor:?} short {short}: {candidate:?}"
                );
            }
            let Some(end) = candidate else {
                continue;
            };
            let state = &mut self.nets[net as usize];
            state.inner_spokes.push((layer, anchor, end, width));
            let mut report = Vec::new();
            self.analyze_pours_reporting(net, &|_| false, Some(&mut report));
            let after = report
                .iter()
                .find(|record| record.pad == pad && record.layer == layer)
                .map_or(short, Self::spokes_short);
            if after < short {
                if self.config.verbose {
                    eprintln!(
                        "inner spoke: {} pad at {anchor:?} on layer {layer} to {end:?}",
                        self.board.obstacles[pad].label
                    );
                }
                added += 1;
            } else {
                self.nets[net as usize].inner_spokes.pop();
            }
        }
        added
    }

    /// `add_manual_spokes` for one net whose pour masks are set.
    pub(super) fn add_manual_spokes_of(&mut self, net: NetId) -> usize {
        let cells = self.grid.cells();
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        let reach = (2.5 / self.grid.pitch).ceil() as i64;
        let mut added = 0;
        {
            // Per pad and layer: tracks tried; a failed try ends its tries.
            let mut tried: HashMap<(usize, usize), usize> = HashMap::new();
            loop {
                let mut report = Vec::new();
                let (pours, mut parent, main, _) =
                    self.analyze_pours_reporting(net, &|_| false, Some(&mut report));
                // A pad merged into its terminal has no nodes of its own to
                // start a track from.
                let own = |record: &SpokeRecord| {
                    record.pad == self.board.nets[net as usize].terminals[record.terminal].pad
                };
                let times = |record: &SpokeRecord| {
                    tried
                        .get(&(record.terminal, record.layer))
                        .copied()
                        .unwrap_or(0)
                };
                let Some(record) = report.iter().find(|record| {
                    own(record)
                        && ((Self::spokes_short(record) > 0 && times(record) < 2)
                            || (self.config.spokeless_tracks
                                && Self::spokeless(record)
                                && times(record) == 0))
                }) else {
                    break;
                };
                // A spokeless pad gets one track into kept fill, nothing
                // else (`Config::spokeless_tracks`).
                let spokeless = Self::spokes_short(record) == 0;
                *tried.entry((record.terminal, record.layer)).or_default() += 1;
                let layer = record.layer;
                // Short in one zone where zones meet: the
                // track must end in that zone's part, which the DRC checks
                // apart (Telemetry's USB shield pads, whose one spoke into
                // the edge zone of priority 11 left it short).
                let seam_zone = record.seam_zone.filter(|_| record.seam_short > 0);
                // The pieces only this pad's spokes reach (KiCad's isolated
                // islands).
                let islands: Vec<u32> = record
                    .outcomes
                    .iter()
                    .filter_map(|outcome| match outcome {
                        SpokeOutcome::Island(piece) if *piece != 0 => Some(*piece),
                        _ => None,
                    })
                    .collect();
                // The directions of its spokes into an island the lattice
                // misses (a sliver: piece 0), and how far out of the pad's
                // centre the fill begins along them (its edge and the gap).
                let slivers: Vec<(crate::geometry::Point, f64)> = {
                    let terminal = &self.board.nets[net as usize].terminals[record.terminal];
                    let gap = self
                        .board
                        .planes
                        .iter()
                        .filter(|plane| plane.net == net && plane.layer == layer)
                        .map(|plane| plane.thermal_gap)
                        .fold(0.0f64, f64::max);
                    let (_, _, geometry, _) =
                        self.spoke_geometry(terminal.pad, terminal.anchor, gap);
                    record
                        .outcomes
                        .iter()
                        .enumerate()
                        .filter(|(_, outcome)| **outcome == SpokeOutcome::Island(0))
                        .map(|(index, _)| (geometry[index].0, geometry[index].2))
                        .collect()
                };
                let class = self.board.nets[net as usize].class;
                let state = &self.nets[net as usize];
                let own = OwnStamps::new(
                    &state.stamped,
                    &[
                        self.map_index(class, layer),
                        self.diagonal_map(class, layer, 0),
                        self.diagonal_map(class, layer, 1),
                    ],
                    cells,
                );
                let free = |map: usize, cell: usize| {
                    self.occupancy[map][cell] as usize == own.contains(map, cell) as usize
                };
                let statics = &self.statics[class];
                let trace = self.map_index(class, layer);
                // The shortest straight track into a kept piece, and into
                // one of the pad's islands.
                // Every straight track into kept fill, the shortest first
                // when they are tried.
                let mut candidates: Vec<Vec<Node>> = Vec::new();
                let mut best_island: Option<Vec<Node>> = None;
                // The net's own copper: a track that meets it on the way is
                // cut there into segments, and KiCad's DRC tests the
                // segment from the pad, which then ends at the junction
                // (1999AZZAR's U2.19: a manual spoke along its exit track,
                // whose first segment ended in the relief).
                let own_nodes: HashSet<(u8, u32)> = state
                    .branches
                    .iter()
                    .flat_map(|branch| branch.nodes.iter())
                    .map(|node| (node.layer, node.cell))
                    .collect();
                // The track ends in fill, not on a node the net's own
                // copper made free (Castor's C10.2: a step onto its own
                // track, which KiCad's DRC does not count).
                let rings = self.thermal_ring_pads(net);
                let probe = fill::FillProbe::new(self, net, &rings);
                // Into a piece joined to a pad of the net (KiCad keeps it),
                // the main piece first.
                let terminal_base = pours.pieces + 1;
                let kept: HashSet<usize> = (0..state.terminal_nodes.len())
                    .map(|terminal| find(&mut parent, terminal_base + terminal))
                    .collect();
                let starts: Vec<Node> = state.terminal_nodes[record.terminal]
                    .iter()
                    .copied()
                    .filter(|node| node.layer as usize == layer)
                    .collect();
                for start in starts {
                    if !statics.trace_allowed(layer, start.cell as usize, net)
                        || !free(trace, start.cell as usize)
                    {
                        continue;
                    }
                    for (direction, (dx, dy)) in DIRECTIONS.iter().enumerate() {
                        let mut path = vec![start];
                        let (mut x, mut y) = ((start.cell as i64) % nx, (start.cell as i64) / nx);
                        for _ in 0..reach {
                            let previous = (y * nx + x) as usize;
                            let (next_x, next_y) = (x + *dx as i64, y + *dy as i64);
                            if next_x < 0 || next_y < 0 || next_x >= nx || next_y >= ny {
                                break;
                            }
                            let cell = (next_y * nx + next_x) as usize;
                            let diagonal_ok = if *dx != 0 && *dy != 0 {
                                let (from, orientation) = if *dx > 0 {
                                    (previous, (*dy < 0) as usize)
                                } else {
                                    (cell, (*dy > 0) as usize)
                                };
                                free(self.diagonal_map(class, layer, orientation), from)
                            } else {
                                true
                            };
                            if !statics.trace_allowed(layer, cell, net)
                                || !statics.edge_allowed(layer, previous, direction, net)
                                || !free(trace, cell)
                                || !diagonal_ok
                                || own_nodes.contains(&(layer as u8, cell as u32))
                            {
                                break;
                            }
                            path.push(Node {
                                layer: layer as u8,
                                cell: cell as u32,
                            });
                            let piece = pours.label[layer][cell];
                            if piece != 0
                                && kept.contains(&find(&mut parent, piece as usize))
                                && seam_zone.is_none_or(|zone| {
                                    self.in_zone_part(zone, self.grid.center_of(cell))
                                })
                                && probe.fill_at(layer, self.grid.center_of(cell))
                            {
                                candidates.push(path.clone());
                                break;
                            }
                            // Into an island: one of its pieces, or the
                            // fill along a spoke into a sliver.
                            let along_sliver = || {
                                let (from, to) = (
                                    self.grid.center_of(start.cell as usize),
                                    self.grid.center_of(cell),
                                );
                                let length = crate::geometry::distance(from, to);
                                length > 0.0
                                    && slivers.iter().any(|(direction, reach)| {
                                        ((to[0] - from[0]) * direction[0]
                                            + (to[1] - from[1]) * direction[1])
                                            / length
                                            > 0.99
                                            && length >= *reach
                                    })
                            };
                            if ((piece != 0 && islands.contains(&piece)) || along_sliver())
                                && probe.fill_at(layer, self.grid.center_of(cell))
                            {
                                if best_island
                                    .as_ref()
                                    .is_none_or(|best| path.len() < best.len())
                                {
                                    best_island = Some(path.clone());
                                }
                                break;
                            }
                            (x, y) = (next_x, next_y);
                        }
                    }
                }
                let label = self.board.nets[net as usize].terminals[record.terminal]
                    .label
                    .clone();
                // A track is kept when the model then counts it (KiCad's
                // DRC tests the segment as emitted, from the pad into the
                // fill), the shortest of the first four that it counts.
                candidates.sort_by_key(|path| path.len());
                let (pad, terminal) = (record.pad, record.terminal);
                let before_short = Self::spokes_short(record);
                let before_tracks = record.tracks;
                let mut counted = false;
                for nodes in candidates.into_iter().take(4) {
                    let length = nodes.len();
                    self.nets[net as usize].branches.push(Branch {
                        nodes,
                        start_terminal: terminal as u16,
                        end_terminal: PLANE_TERMINAL,
                    });
                    self.stamp(net);
                    let mut after = Vec::new();
                    self.analyze_pours_reporting(net, &|_| false, Some(&mut after));
                    counted = after
                        .iter()
                        .find(|record| record.pad == pad && record.layer == layer)
                        .is_some_and(|record| {
                            if spokeless {
                                record.tracks > before_tracks
                            } else {
                                Self::spokes_short(record) < before_short
                            }
                        });
                    if counted {
                        if self.config.verbose {
                            eprintln!("manual spoke: {label} on layer {layer}, {length} nodes");
                        }
                        break;
                    }
                    self.nets[net as usize].branches.pop();
                    self.stamp(net);
                }
                if counted {
                    added += 1;
                    continue;
                }
                if spokeless {
                    tried.insert((record.terminal, record.layer), 2);
                    continue;
                }
                // No track reaches kept fill: a via from an island to the
                // main piece on another layer.
                if !islands.is_empty() && self.board.layer_count >= 2 {
                    let anchor = self.board.nets[net as usize].terminals[record.terminal].anchor;
                    let mut anchors: HashMap<usize, crate::geometry::Point> = HashMap::new();
                    for piece in &islands {
                        anchors
                            .entry(find(&mut parent, *piece as usize))
                            .or_insert(anchor);
                    }
                    let chosen =
                        self.stitching_vias(net, &pours, &mut parent, main, &anchors, true);
                    if let Some((layer, other, cell)) = chosen.first().copied() {
                        if self.config.verbose {
                            eprintln!("island via: {label} on layer {layer} to layer {other}");
                        }
                        self.nets[net as usize].branches.push(Branch {
                            nodes: vec![
                                Node {
                                    layer: layer as u8,
                                    cell: cell as u32,
                                },
                                Node {
                                    layer: other as u8,
                                    cell: cell as u32,
                                },
                            ],
                            start_terminal: PLANE_TERMINAL,
                            end_terminal: PLANE_TERMINAL,
                        });
                        self.stamp(net);
                        added += 1;
                        continue;
                    }
                }
                // Else the pad's own track into its island: two items in it.
                if self.config.island_stubs
                    && let Some(nodes) = best_island
                {
                    if self.config.verbose {
                        eprintln!(
                            "island stub: {label} on layer {layer}, {} nodes",
                            nodes.len()
                        );
                    }
                    self.nets[net as usize].branches.push(Branch {
                        nodes,
                        start_terminal: record.terminal as u16,
                        end_terminal: PLANE_TERMINAL,
                    });
                    self.stamp(net);
                    added += 1;
                    self.island_stubs += 1;
                    continue;
                }
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                    eprintln!(
                        "manual spoke: none for {label} on layer {layer} ({} start nodes, {} islands)",
                        self.nets[net as usize].terminal_nodes[record.terminal]
                            .iter()
                            .filter(|node| node.layer as usize == layer)
                            .count(),
                        islands.len()
                    );
                }
                tried.insert((record.terminal, record.layer), 2);
            }
        }
        added
    }

    /// Per layer, the pads of `net` whose thermal relief keeps the pour's
    /// fill away, each with the distance the brush centre keeps from it
    /// (the gap and half the brush). A thermal relief keeps its gap around
    /// the pads it connects, all but the spokes: fill does not flow past
    /// such a pad, it joins the pad only through the spokes
    /// (`analyze_pours_full`). PolyKybd's GND ran across a connector's
    /// 0.3 mm pins in this model and fell into islands in KiCad's.
    pub(super) fn thermal_ring_pads(&self, net: NetId) -> Vec<Vec<(usize, f64)>> {
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        let description = &self.board.nets[net as usize];
        let mut result = vec![Vec::new(); layers];
        for (layer, pads) in result.iter_mut().enumerate() {
            if state.plane[layer].is_empty() {
                continue;
            }
            let Some(gap) = self
                .board
                .planes
                .iter()
                .filter(|plane| plane.net == net && plane.layer == layer && !plane.solid)
                .map(|plane| plane.thermal_gap)
                .reduce(f64::max)
                .filter(|gap| *gap > 0.0)
            else {
                continue;
            };
            let reach = gap + self.board.classes[state.plane_class[layer]].trace_width / 2.0;
            let thermal = |label: &str| {
                self.board
                    .isolated_pads
                    .binary_search(&label.to_string())
                    .is_err()
                    && self
                        .board
                        .solid_pads
                        .binary_search(&label.to_string())
                        .is_err()
            };
            for terminal in &description.terminals {
                if !thermal(&terminal.label) {
                    continue;
                }
                if self.board.obstacles[terminal.pad].layers & (1 << layer) == 0 {
                    continue;
                }
                pads.push((terminal.pad, reach));
            }
            // KiCad cuts a relief (and in a hatched zone adds a ring) around
            // every thermal pad, a pad merged into its terminal's larger pad
            // too (an exposed pad's thermal vias, on the far side or inside
            // the pad).
            for merged in self
                .board
                .merged_pads
                .iter()
                .filter(|merged| merged.net == net && merged.layers & (1 << layer) != 0)
            {
                // A solid connection of the label is the terminal's own
                // (the largest pad); the pads merged into it keep the
                // zone's thermal relief (`kicad_spokes`).
                if description.terminals.get(merged.host).is_some_and(|host| {
                    self.board.isolated_pads.binary_search(&host.label).is_err()
                }) {
                    pads.push((merged.pad, reach));
                }
            }
        }
        result
    }

    /// Clears the nodes of `free` (per layer, empty for none) that the
    /// brush may not take inside a thermal ring (`thermal_ring_pads`).
    /// Per layer, the pads of `net` set to no zone connection (by pad,
    /// `Board::isolated_pad_obstacles`) where the net pours, each with what
    /// KiCad's filler keeps from it: the zone's clearance and half the
    /// minimum width (the brush's centre stays that far from the pad, as
    /// from another net's). The fill does not touch such a pad; only a
    /// track from it into the fill joins it (1999AZZAR's J2.RN read as on
    /// the pour, a branch of no length "connected" it, KiCad found it
    /// unconnected).
    pub(super) fn isolated_knockouts(&self, net: NetId) -> Vec<Vec<(usize, f64)>> {
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        let mut result = vec![Vec::new(); layers];
        if self.board.isolated_pad_obstacles.is_empty() {
            return result;
        }
        for (layer, pads) in result.iter_mut().enumerate() {
            if state.plane[layer].is_empty() {
                continue;
            }
            let Some(clearance) = self
                .board
                .planes
                .iter()
                .filter(|plane| plane.net == net && plane.layer == layer)
                .map(|plane| self.board.classes[plane.class].clearance)
                .reduce(f64::max)
            else {
                continue;
            };
            let reach = clearance + self.board.classes[state.plane_class[layer]].trace_width / 2.0;
            for &pad in &self.board.isolated_pad_obstacles {
                let obstacle = &self.board.obstacles[pad];
                if obstacle.net == Some(net) && obstacle.layers & (1 << layer) != 0 {
                    pads.push((pad, reach));
                }
            }
        }
        result
    }

    /// Clears the nodes of `free` inside the thermal reliefs KiCad cuts
    /// around the net's vias in a hatched zone (`FillProbe::relieved_vias`:
    /// the via's radius, the gap and half the minimum width), but for its
    /// four spokes along the axes (the brush's centres within half a spoke
    /// less half the minimum width of them), which join the via to the
    /// fill. IotThermalCamera's Tgnd: a via of the net beside TU8 held the
    /// lattice's diagonal neck between two pieces that KiCad's relief
    /// keeps apart.
    pub(super) fn cut_via_reliefs(&self, net: NetId, free: &mut [Vec<bool>]) {
        let vias = fill::FillProbe::relieved_vias(self, net);
        for (layer, vias) in vias.iter().enumerate() {
            if free[layer].is_empty() || vias.is_empty() {
                continue;
            }
            let planes = || {
                self.board
                    .planes
                    .iter()
                    .filter(|plane| plane.net == net && plane.layer == layer)
            };
            let gap = planes()
                .map(|plane| plane.thermal_gap)
                .fold(0.0f64, f64::max);
            let bridge = planes()
                .map(|plane| (plane.thermal_reach - plane.thermal_gap).max(0.0))
                .fold(0.0f64, f64::max);
            let half =
                self.board.classes[self.nets[net as usize].plane_class[layer]].trace_width / 2.0;
            for (at, radius) in vias {
                let reach = radius + gap + half;
                let corridor = ((bridge.min(2.0 * radius)) / 2.0 - half).max(0.0) + 1.0e-6;
                let bounds = crate::geometry::Aabb {
                    minimum: *at,
                    maximum: *at,
                }
                .inflated(reach);
                let Some((x0, y0, x1, y1)) = self.grid.node_range(bounds) else {
                    continue;
                };
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let centre = self.grid.center(x, y);
                        let (dx, dy) = ((centre[0] - at[0]).abs(), (centre[1] - at[1]).abs());
                        if crate::geometry::distance(centre, *at) < reach
                            && dx > corridor
                            && dy > corridor
                        {
                            free[layer][self.grid.index(x, y)] = false;
                        }
                    }
                }
            }
        }
    }

    pub(super) fn cut_thermal_rings(&self, rings: &[Vec<(usize, f64)>], free: &mut [Vec<bool>]) {
        for (layer, pads) in rings.iter().enumerate() {
            if free[layer].is_empty() {
                continue;
            }
            for (pad, reach) in pads {
                let pad = &self.board.obstacles[*pad];
                let Some((x0, y0, x1, y1)) =
                    self.grid.node_range(pad.shape.aabb().inflated(*reach))
                else {
                    continue;
                };
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        if pad.shape.distance_to_point(self.grid.center(x, y)) < *reach {
                            free[layer][self.grid.index(x, y)] = false;
                        }
                    }
                }
            }
        }
    }
}

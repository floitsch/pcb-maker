// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Stitching vias and the pour nets' stitching after the negotiation.

use super::*;

impl Router {
    /// Stitching vias for pour net `net`: per island in `anchors` (a root
    /// of `parent`, with the point it should be near), the free via spot
    /// closest to that point that joins the island to another piece on
    /// another layer (with `main_only`, to the main piece only; else the
    /// main piece is worth a 5 mm detour), keeping the hole-to-hole
    /// distance from the net's vias and from each other. Returns (layer,
    /// other layer, cell) per via.
    pub(super) fn stitching_vias(
        &self,
        net: NetId,
        pours: &crate::pour::PourMap,
        parent: &mut [usize],
        main: usize,
        anchors: &HashMap<usize, crate::geometry::Point>,
        main_only: bool,
    ) -> Vec<(usize, usize, usize)> {
        let class_index = self.board.nets[net as usize].class;
        let class = self.board.classes[class_index];
        let layers = self.board.layer_count;
        let via_reach = 0;
        let via_map = self.map_index(class_index, layers);
        let own = OwnStamps::new(
            &self.nets[net as usize].stamped,
            &[via_map],
            self.grid.cells(),
        );
        // A via keeps the hole-to-hole distance from the net's vias
        // already there too: KiCad's rule holds within a net
        // (PolyKybd's stitches landed 0.1 mm from earlier ones).
        let spacing = class.via_drill + self.board.hole_to_hole;
        let mut crowded = vec![false; self.grid.cells()];
        for branch in &self.nets[net as usize].branches {
            for pair in branch.nodes.windows(2) {
                if pair[0].cell != pair[1].cell || pair[0].layer == pair[1].layer {
                    continue;
                }
                let center = self.grid.center_of(pair[0].cell as usize);
                let keep = spacing.max(class.via_diameter);
                let bounds = crate::geometry::Aabb {
                    minimum: center,
                    maximum: center,
                }
                .inflated(keep);
                if let Some((x0, y0, x1, y1)) = self.grid.node_range(bounds) {
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            if crate::geometry::distance(center, self.grid.center(x, y)) < keep {
                                crowded[self.grid.index(x, y)] = true;
                            }
                        }
                    }
                }
            }
        }
        let mut best: HashMap<usize, (f64, usize, usize, usize)> = HashMap::new();
        if layers >= 2 {
            let roots: Vec<Vec<usize>> = (0..layers)
                .map(|layer| {
                    pours.label[layer]
                        .iter()
                        .map(|piece| {
                            if *piece == 0 {
                                0
                            } else {
                                find(parent, *piece as usize)
                            }
                        })
                        .collect()
                })
                .collect();
            for layer in 0..layers {
                for (cell, island) in roots[layer].iter().enumerate() {
                    let Some(anchor) = anchors.get(island) else {
                        continue;
                    };
                    if crowded[cell]
                        || !self.statics[class_index].via_allowed(cell, net)
                        || self.occupancy[via_map][cell] as usize
                            != own.contains(via_map, cell) as usize
                        || !pours.solid_around(&self.grid, layer, cell, via_reach)
                    {
                        continue;
                    }
                    for other in (0..layers).filter(|other| *other != layer) {
                        let target = roots[other].get(cell).copied().unwrap_or(0);
                        if target == 0
                            || target == *island
                            || (main_only && target != main)
                            || !pours.solid_around(&self.grid, other, cell, via_reach)
                        {
                            continue;
                        }
                        let bonus = if target == main { 0.0 } else { 5.0 };
                        let distance =
                            bonus + crate::geometry::distance(*anchor, self.grid.center_of(cell));
                        let entry = best.entry(*island).or_insert((f64::INFINITY, 0, 0, 0));
                        if distance < entry.0 {
                            *entry = (distance, layer, other, cell);
                        }
                    }
                }
            }
        }
        // Two new vias keep the hole-to-hole distance between them.
        let mut chosen: Vec<(usize, usize, usize)> = Vec::new();
        let mut candidates: Vec<(f64, usize, usize, usize, usize)> = best
            .into_iter()
            .filter(|(_, entry)| entry.0.is_finite())
            .map(|(island, (distance, layer, other, cell))| (distance, layer, other, cell, island))
            .collect();
        // Ties broken by the cell, then the island (two islands of one pad
        // on two layers share an anchor and a best cell): a hash map's
        // order must not decide.
        candidates.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap()
                .then(a.3.cmp(&b.3))
                .then(a.4.cmp(&b.4))
        });
        for (_, layer, other, cell, _) in candidates {
            let at = self.grid.center_of(cell);
            if chosen.iter().any(|(_, _, placed)| {
                *placed == cell
                    || crate::geometry::distance(at, self.grid.center_of(*placed))
                        < spacing.max(class.via_diameter)
            }) {
                continue;
            }
            chosen.push((layer, other, cell));
        }
        chosen
    }

    /// Connects pour islands that hold terminals to the main piece with
    /// vias; terminals that cannot be reached that way are routed to the
    /// main piece like off-pour pads. Returns the number of stitching vias.
    /// Plane ends of `net`'s branches that lie in no pour piece any more
    /// (other nets' copper took the fill after the branch was routed) and
    /// hang past a junction with another branch of the net: the tail is
    /// trimmed back to the junction, which now ends the branch
    /// (USB_Cable_Tester: a /SHLD branch ran 0.1 mm past a junction into
    /// fill that signals rerouted after a part moved took; KiCad reports
    /// the stub as a dangling track). Returns the nodes trimmed.
    pub(super) fn trim_dangling_plane_ends(&mut self, net: NetId) -> usize {
        if self.nets[net as usize].plane.is_empty() {
            return 0;
        }
        let (pours, _, _) = self.analyze_pours(net);
        let state = &self.nets[net as usize];
        let mut uses: HashMap<Node, usize> = HashMap::new();
        for branch in &state.branches {
            for node in &branch.nodes {
                *uses.entry(*node).or_default() += 1;
            }
        }
        let label = |node: &Node| {
            pours.label[node.layer as usize]
                .get(node.cell as usize)
                .copied()
                .unwrap_or(0)
        };
        // (branch, from its end, nodes to drop)
        let mut cuts: Vec<(usize, bool, usize)> = Vec::new();
        for (index, branch) in state.branches.iter().enumerate().skip(state.fixed) {
            for at_end in [true, false] {
                let terminal = if at_end {
                    branch.end_terminal
                } else {
                    branch.start_terminal
                };
                if terminal != PLANE_TERMINAL || branch.nodes.len() < 3 {
                    continue;
                }
                let nodes: Vec<&Node> = if at_end {
                    branch.nodes.iter().rev().collect()
                } else {
                    branch.nodes.iter().collect()
                };
                if label(nodes[0]) != 0 {
                    continue;
                }
                let Some(stop) = nodes
                    .iter()
                    .position(|node| label(node) != 0 || uses.get(node).copied().unwrap_or(0) > 1)
                else {
                    continue;
                };
                // Only back to a junction, the branch keeping some length.
                if stop > 0
                    && uses.get(nodes[stop]).copied().unwrap_or(0) > 1
                    && label(nodes[stop]) == 0
                    && stop + 1 < nodes.len()
                    && nodes[stop].layer == nodes[stop - 1].layer
                {
                    cuts.push((index, at_end, stop));
                }
            }
        }
        if cuts.is_empty() {
            return 0;
        }
        self.unstamp(net);
        let state = &mut self.nets[net as usize];
        let mut trimmed = 0;
        for (index, at_end, drop) in cuts {
            let branch = &mut state.branches[index];
            if at_end {
                branch.nodes.truncate(branch.nodes.len() - drop);
                branch.end_terminal = NO_TERMINAL;
            } else {
                branch.nodes.drain(..drop);
                branch.start_terminal = NO_TERMINAL;
            }
            trimmed += drop;
        }
        self.stamp(net);
        trimmed
    }

    pub(super) fn stitch_pours(&mut self, net: NetId, order: &[NetId]) -> usize {
        let class = self.board.classes[self.board.nets[net as usize].class];
        let layers = self.board.layer_count;
        let terminal_count = self.nets[net as usize].terminal_nodes.len();
        let _ = class;
        let mut stitches = 0;
        let mut rerouted = false;
        let mut previous_islands = usize::MAX;
        let mut stalled = 0;
        for _ in 0..1500 {
            let (pours, mut parent, main, spoke_islands) = self.analyze_pours_full(net, &|_| false);
            let terminal_base = pours.pieces + 1;
            let islands: Vec<usize> = (0..terminal_count)
                .filter(|terminal| find(&mut parent, terminal_base + terminal) != main)
                .collect();
            if islands.is_empty() && spoke_islands.is_empty() {
                break;
            }
            // Vias that do not reduce the stranded count are not progress.
            let island_count = islands.len() + spoke_islands.len();
            if island_count >= previous_islands {
                stalled += 1;
                if stalled > 3 && rerouted {
                    break;
                }
            } else {
                stalled = 0;
            }
            previous_islands = island_count;
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                for terminal in islands.iter().take(6) {
                    let root = find(&mut parent, terminal_base + terminal);
                    let branches = (0..self.nets[net as usize].branches.len())
                        .filter(|index| {
                            find(&mut parent, terminal_base + terminal_count + index) == root
                        })
                        .count();
                    let pieces: Vec<usize> = (1..=pours.pieces)
                        .filter(|piece| find(&mut parent, *piece) == root)
                        .collect();
                    let sizes: Vec<usize> = pieces
                        .iter()
                        .take(4)
                        .map(|piece| {
                            pours
                                .label
                                .iter()
                                .map(|layer| layer.iter().filter(|l| **l == *piece as u32).count())
                                .sum()
                        })
                        .collect();
                    for (index, branch) in self.nets[net as usize].branches.iter().enumerate() {
                        if find(&mut parent, terminal_base + terminal_count + index) != root {
                            continue;
                        }
                        let ends = |terminal: u16| match terminal {
                            NO_TERMINAL => "junction".to_string(),
                            PLANE_TERMINAL => "plane".to_string(),
                            FREE_END => "free".to_string(),
                            other => format!("pad {other}"),
                        };
                        let first = branch.nodes[0];
                        let last = *branch.nodes.last().unwrap();
                        eprintln!(
                            "    branch {index}: {} -> {}, {} nodes, first {:?} label {}, last {:?} label {}, on_plane {:?}",
                            ends(branch.start_terminal),
                            ends(branch.end_terminal),
                            branch.nodes.len(),
                            self.grid.center_of(first.cell as usize),
                            pours.label[first.layer as usize]
                                .get(first.cell as usize)
                                .copied()
                                .unwrap_or(0),
                            self.grid.center_of(last.cell as usize),
                            pours.label[last.layer as usize]
                                .get(last.cell as usize)
                                .copied()
                                .unwrap_or(0),
                            self.nets[net as usize].on_plane.get(*terminal).copied()
                        );
                    }
                    eprintln!(
                        "  stranded {} at {:?} layers {:#b}: {branches} branches, {} pieces (sizes {sizes:?}), terminal nodes {}",
                        self.board.nets[net as usize].terminals[*terminal].label,
                        self.board.nets[net as usize].terminals[*terminal].anchor,
                        self.board.nets[net as usize].terminals[*terminal].layers,
                        pieces.len(),
                        self.nets[net as usize].terminal_nodes[*terminal].len()
                    );
                }
            }
            if self.config.verbose {
                let attached = islands
                    .iter()
                    .filter(|terminal| {
                        let root = find(&mut parent, terminal_base + **terminal);
                        (1..=pours.pieces).any(|piece| find(&mut parent, piece) == root)
                    })
                    .count();
                eprintln!(
                    "pour {}: {} pieces, {} stranded terminals ({} touch a piece), {} spoke islands",
                    self.board.nets[net as usize].name,
                    pours.pieces,
                    islands.len(),
                    attached,
                    spoke_islands.len()
                );
            }
            // Per stranded island, the via closest to its terminal that joins
            // it to another piece (the main piece is worth a detour), all
            // found in one pass over the board and added together: one via
            // per pass and island made a ground net with 300 stranded pads
            // cost 300 board-wide floods. Islands without room stay for
            // rerouting.
            let mut anchors: HashMap<usize, crate::geometry::Point> = HashMap::new();
            for terminal in &islands {
                let island = find(&mut parent, terminal_base + terminal);
                anchors
                    .entry(island)
                    .or_insert(self.board.nets[net as usize].terminals[*terminal].anchor);
            }
            // A spoke island gets a via too: with copper of the net in it
            // KiCad keeps it, and the spoke counts.
            for (piece, terminal) in &spoke_islands {
                let island = find(&mut parent, *piece);
                anchors
                    .entry(island)
                    .or_insert(self.board.nets[net as usize].terminals[*terminal].anchor);
            }
            let chosen = self.stitching_vias(net, &pours, &mut parent, main, &anchors, false);
            if self.config.verbose && !spoke_islands.is_empty() {
                let into_spoke_islands = chosen
                    .iter()
                    .filter(|(layer, _, cell)| {
                        let root = find(&mut parent, pours.label[*layer][*cell] as usize);
                        spoke_islands
                            .iter()
                            .any(|(piece, _)| find(&mut parent, *piece) == root)
                    })
                    .count();
                eprintln!(
                    "pour {}: {} vias into the {} spoke islands",
                    self.board.nets[net as usize].name,
                    into_spoke_islands,
                    spoke_islands.len()
                );
            }
            if !chosen.is_empty() {
                for (layer, other, cell) in &chosen {
                    self.nets[net as usize].branches.push(Branch {
                        nodes: vec![
                            Node {
                                layer: *layer as u8,
                                cell: *cell as u32,
                            },
                            Node {
                                layer: *other as u8,
                                cell: *cell as u32,
                            },
                        ],
                        start_terminal: PLANE_TERMINAL,
                        end_terminal: PLANE_TERMINAL,
                    });
                }
                self.stamp(net);
                stitches += chosen.len();
                continue;
            }
            // Spoke islands without room for a via stay as they are. Past
            // the caller's wall-clock deadline the stranded pads stay too:
            // their hard reroute and forced connection are a repair (PolyKybd
            // left: about 10 s a finish under load).
            if rerouted || islands.is_empty() || self.past_wall_deadline() {
                break;
            }
            // No via fits: route the stranded terminals to the main piece.
            rerouted = true;
            stalled = 0;
            let mut target: Vec<Vec<bool>> = Vec::new();
            for layer in 0..layers {
                target.push(
                    pours.label[layer]
                        .iter()
                        .map(|piece| *piece != 0 && find(&mut parent, *piece as usize) == main)
                        .collect(),
                );
            }
            for terminal in &islands {
                self.nets[net as usize].on_plane[*terminal] = false;
            }
            self.nets[net as usize].plane_target = target;
            self.unstamp(net);
            self.route_net_seq(net, 0.0, true, 1.0);
            self.stamp(net);
            let hard_complete = self.nets[net as usize].complete;
            let mut forced = None;
            if !hard_complete {
                forced = Some(self.force_connect(net, order));
            }
            if self.config.verbose {
                eprintln!(
                    "pour {}: reroute of {} stranded terminals: hard complete {hard_complete}, forced {forced:?}, {} branches",
                    self.board.nets[net as usize].name,
                    islands.len(),
                    self.nets[net as usize].branches.len()
                );
            }
            self.nets[net as usize].plane_target = Vec::new();
        }
        let (pours, mut parent, main) = self.analyze_pours(net);
        let terminal_base = pours.pieces + 1;
        let mut complete = true;
        for terminal in 0..terminal_count {
            let connected = find(&mut parent, terminal_base + terminal) == main;
            self.nets[net as usize].connected[terminal] = connected;
            complete &= connected;
        }
        self.nets[net as usize].complete = complete;
        stitches
    }
}

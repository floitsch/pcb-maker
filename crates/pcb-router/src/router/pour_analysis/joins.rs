// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The pour analysis's joins: which pieces the net's pads and branches
//! join (solid connections, branches, straight thermal spokes) and which
//! group is the main piece.

use super::*;

/// The analysis's union-find over the pieces (labels from 1; element 0
/// unused), then the terminals, then the branches.
pub(super) struct Joins {
    pub(super) parent: Vec<usize>,
    pub(super) terminal_base: usize,
    pub(super) branch_base: usize,
}

impl Joins {
    pub(super) fn new(pieces: usize, terminals: usize, branches: usize) -> Self {
        let terminal_base = pieces + 1;
        let branch_base = terminal_base + terminals;
        Self {
            parent: (0..branch_base + branches).collect(),
            terminal_base,
            branch_base,
        }
    }

    pub(super) fn union(&mut self, a: usize, b: usize) {
        union(&mut self.parent, a, b);
    }
}

/// Joins the groups of `a` and `b` (`b`'s root the root of both).
pub(super) fn union(parent: &mut [usize], a: usize, b: usize) {
    let (a, b) = (find(parent, a), find(parent, b));
    parent[a] = b;
}

/// The net whose pads the analysis joins, and what it reads of them.
pub(super) struct PourPads<'a> {
    pub(super) net: NetId,
    pub(super) board: &'a Board,
    pub(super) state: &'a NetState,
    pub(super) description: &'a crate::board::Net,
    /// Per layer, whether a pour of the net there is solid (KiCad's
    /// `connect_pads yes`).
    pub(super) solid: Vec<bool>,
}

impl PourPads<'_> {
    /// Whether KiCad's own zone connection of `terminal`'s pad is none.
    pub(super) fn isolated(&self, terminal: usize) -> bool {
        self.board
            .isolated_pads
            .binary_search(&self.description.terminals[terminal].label)
            .is_ok()
    }

    /// Whether KiCad's own zone connection of `terminal`'s pad is solid.
    pub(super) fn solid_pad(&self, terminal: usize) -> bool {
        self.board
            .solid_pads
            .binary_search(&self.description.terminals[terminal].label)
            .is_ok()
    }
}

impl Router {
    /// Joins each solidly connected pad (a solid pour's, or one KiCad
    /// connects solidly) to the pieces whose fill reaches its copper;
    /// returns those pieces.
    pub(super) fn join_solid_pads(
        &self,
        pads: &PourPads,
        pours: &crate::pour::PourMap,
        joins: &mut Joins,
        rings: &[Vec<(usize, f64)>],
        reclaimed: Vec<Vec<bool>>,
    ) -> Vec<usize> {
        let layers = self.board.layer_count;
        let (state, description) = (pads.state, pads.description);
        let reach = (0.8 / self.grid.steps().0).ceil() as usize;
        let near = crate::pour::NearOrder::new(reach);
        let mut touched: Vec<usize> = Vec::new();
        let probe = fill::FillProbe::new(self, pads.net, rings).with_reclaimed(reclaimed);
        for (terminal, nodes) in state.terminal_nodes.iter().enumerate() {
            if pads.isolated(terminal) {
                continue;
            }
            let pad = &self.board.obstacles[description.terminals[terminal].pad];
            let anchor = description.terminals[terminal].anchor;
            // Per layer and piece, its node nearest the pad.
            let mut candidates: Vec<(usize, u32, usize, f64)> = Vec::new();
            // Per layer, the pieces that can join the pad: those with a
            // node within the brush and two pitches of its copper, the
            // farthest `solid_fill_reaches` looks (a piece's candidate is
            // its node nearest the pad, and a candidate farther joins
            // nothing and asks the probe nothing). A pad's hundreds of
            // nodes at a fine pitch each looked at every node of their
            // window for every piece (nrf24_remote's analyses two fifths
            // of its route); the windows are now visited nearest first and
            // left once each of these pieces was met.
            let mut among: Vec<Option<Vec<u32>>> = vec![None; layers];
            for node in nodes
                .iter()
                .filter(|node| pads.solid[node.layer as usize] || pads.solid_pad(terminal))
            {
                let layer = node.layer as usize;
                let among = among[layer].get_or_insert_with(|| {
                    let reach = probe.brush_half(layer) + 3.0 * self.grid.pitch;
                    match self.grid.node_range(pad.shape.aabb().inflated(reach)) {
                        Some((x0, y0, x1, y1)) => pours.pieces_in_box(
                            &self.grid,
                            layer,
                            (x0 as i64, y0 as i64, x1 as i64, y1 as i64),
                        ),
                        None => Vec::new(),
                    }
                });
                if among.is_empty() {
                    continue;
                }
                for (piece, cell) in
                    pours.pieces_near_ordered(&self.grid, layer, node.cell as usize, &near, among)
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
                    joins.union(joins.terminal_base + terminal, piece as usize);
                    touched.push(piece as usize);
                }
            }
        }
        probe.charge();
        touched
    }

    /// Joins each branch but those `skip` names to the pieces it lies in
    /// (`branch_pieces`), to its terminals and to its junctions' hosts;
    /// adds those pieces to `touched`.
    pub(super) fn join_branches(
        &self,
        state: &NetState,
        joins: &mut Joins,
        branch_pieces: &[Vec<(u32, usize)>],
        skip: &dyn Fn(usize) -> bool,
        touched: &mut Vec<usize>,
    ) {
        let (terminal_base, branch_base) = (joins.terminal_base, joins.branch_base);
        let keep = vec![true; state.branches.len()];
        let hosts = junction_hosts(&state.branches, &keep);
        for (index, branch) in state.branches.iter().enumerate() {
            if skip(index) {
                continue;
            }
            for (piece, _) in &branch_pieces[index] {
                joins.union(branch_base + index, *piece as usize);
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
                        joins.union(branch_base + index, branch_base + host);
                    }
                } else if terminal != PLANE_TERMINAL && terminal != FREE_END {
                    joins.union(branch_base + index, terminal_base + terminal as usize);
                }
            }
        }
    }

    /// Without the KiCad spoke model (`Config::kicad_spokes` off): joins
    /// each pad with thermal relief to the kept pieces its straight
    /// spokes reach (`touched`: the pieces with copper of the net in
    /// them); returns the spoke islands, each with a pad short of two
    /// good spokes that spokes into it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn straight_spokes(
        &self,
        pads: &PourPads,
        pours: &crate::pour::PourMap,
        own: &OwnStamps,
        joins: &mut Joins,
        touched: &[usize],
        mut report: Option<&mut Vec<SpokeRecord>>,
    ) -> Vec<(usize, usize)> {
        let with_items: HashSet<usize> = touched
            .iter()
            .map(|piece| find(&mut joins.parent, *piece))
            .collect();
        let mut spoke_islands: Vec<(usize, usize)> = Vec::new();
        for (terminal, nodes) in pads.state.terminal_nodes.iter().enumerate() {
            if pads.isolated(terminal) || pads.solid_pad(terminal) {
                continue;
            }
            // In layer order: a hash set's order made the union order, and
            // so the roots, the main piece's tie-break and the stitching
            // vias' anchors, differ from run to run.
            let layers_of_nodes: std::collections::BTreeSet<usize> =
                nodes.iter().map(|node| node.layer as usize).collect();
            for layer in layers_of_nodes {
                if pours.label[layer].is_empty() || pads.solid[layer] {
                    continue;
                }
                let (good, islands) = self.pad_spokes(
                    pads,
                    pours,
                    own,
                    joins,
                    &with_items,
                    terminal,
                    layer,
                    report.as_deref_mut(),
                );
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
    }

    /// The straight spokes of `terminal`'s pad on `layer`: joins the pad
    /// to the kept pieces they reach (`with_items`: the roots of the
    /// pieces with copper of the net); returns how many did and the
    /// islands the others reach.
    #[allow(clippy::too_many_arguments)]
    fn pad_spokes(
        &self,
        pads: &PourPads,
        pours: &crate::pour::PourMap,
        own: &OwnStamps,
        joins: &mut Joins,
        with_items: &HashSet<usize>,
        terminal: usize,
        layer: usize,
        report: Option<&mut Vec<SpokeRecord>>,
    ) -> (usize, Vec<usize>) {
        let (net, state, description) = (pads.net, pads.state, pads.description);
        let terminal_base = joins.terminal_base;
        let has_item =
            |parent: &mut Vec<usize>, piece: usize| with_items.contains(&find(parent, piece));
        let pad = &self.board.obstacles[description.terminals[terminal].pad];
        let anchor = description.terminals[terminal].anchor;
        let bounds = pad.shape.aabb();
        let round = matches!(pad.shape, crate::geometry::Shape::Circle { .. });
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
            ((bridge - self.board.classes[state.plane_class[layer]].trace_width) / 2.0).max(0.0);
        let plane_map = self.map_index(state.plane_class[layer], layer);
        let clear = |cell: usize| {
            state.plane[layer][cell]
                && self.occupancy.get(plane_map, cell) as usize
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
                    let steps = ((extra / (self.grid.pitch / 2.0)).ceil() as usize).max(1);
                    let blocked = (1..=steps).any(|step| {
                        let along = edge + extra * step as f64 / steps as f64;
                        [-side, 0.0, side].iter().any(|offset| {
                            let at = [
                                anchor[0] + direction[0] * along - direction[1] * offset,
                                anchor[1] + direction[1] * along + direction[0] * offset,
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
                    if has_item(&mut joins.parent, piece as usize) {
                        joins.union(terminal_base + terminal, piece as usize);
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
        if let Some(report) = report {
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
                required: self.config.min_spokes,
            });
        }
        (good, islands)
    }
}

/// The root of the main piece: the group holding most terminals (the
/// terminals from `terminal_base` in `parent`), on equal votes the one
/// with the most fill (`labels`: the pieces' labels per layer and node),
/// then the least root.
pub(super) fn main_piece(
    parent: &mut [usize],
    labels: &[Vec<u32>],
    terminal_base: usize,
    terminal_count: usize,
) -> usize {
    let mut votes: HashMap<usize, usize> = HashMap::new();
    for terminal in 0..terminal_count {
        *votes
            .entry(find(parent, terminal_base + terminal))
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
    if leaders.len() > 1 {
        let mut area: HashMap<usize, usize> = HashMap::new();
        let mut root_of: HashMap<u32, usize> = HashMap::new();
        for labels in labels {
            for label in labels.iter().filter(|label| **label != 0) {
                let root = *root_of
                    .entry(*label)
                    .or_insert_with(|| find(parent, *label as usize));
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
    }
}

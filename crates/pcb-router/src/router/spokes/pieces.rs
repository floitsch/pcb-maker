// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! What the kept spokes make of the pour map's pieces: the items each
//! piece holds, the pieces KiCad's outlines join, and pass two's count of
//! each pad's spokes.

use super::*;

/// Per label, each plated hole's centre and radius (pads may share a
/// number: the hole is the one at the pad).
pub(super) struct PlatedHoles<'a> {
    holes: HashMap<&'a str, Vec<(crate::geometry::Point, f64)>>,
}

impl<'a> PlatedHoles<'a> {
    /// The plated holes among `obstacles` (labelled "<pad> plated hole").
    pub(super) fn of(obstacles: &'a [crate::board::Obstacle]) -> Self {
        let mut holes: HashMap<&str, Vec<(crate::geometry::Point, f64)>> = HashMap::new();
        for obstacle in obstacles
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
        Self { holes }
    }

    /// The radius of pad `label`'s plated hole nearest `at`; 0 for a pad
    /// without one.
    pub(super) fn radius_at(&self, label: &str, at: crate::geometry::Point) -> f64 {
        self.holes
            .get(label)
            .and_then(|holes| {
                holes.iter().min_by(|a, b| {
                    crate::geometry::distance(a.0, at)
                        .total_cmp(&crate::geometry::distance(b.0, at))
                })
            })
            .map_or(0.0, |(_, radius)| *radius)
    }
}

/// The pour map's pieces as KiCad's connectivity counts their items, and
/// the groups of pieces its outlines hold as one (`merge`).
pub(super) struct PieceItems {
    /// Per piece: each branch with a node in it, each pad joined solidly,
    /// and each pad track touching a spoke into it.
    pub(super) items: HashMap<u32, usize>,
    /// Per piece, the pads joined to it by a spoke (one item each, however
    /// many pieces of one outline they reach).
    pub(super) pads: HashMap<u32, Vec<usize>>,
    /// Per piece merged with others, its group (sorted, itself included).
    groups: HashMap<u32, Vec<u32>>,
}

impl PieceItems {
    /// The items `items` and pads `pads` per piece, no piece merged yet.
    pub(super) fn new(items: HashMap<u32, usize>, pads: HashMap<u32, Vec<usize>>) -> Self {
        Self {
            items,
            pads,
            groups: HashMap::new(),
        }
    }

    /// Makes pieces `a` and `b` one outline: joins them in `parent` and
    /// merges their groups.
    pub(super) fn merge(&mut self, parent: &mut [usize], a: u32, b: u32) {
        let groups = &mut self.groups;
        union(parent, a as usize, b as usize);
        let mut group = groups.get(&a).cloned().unwrap_or_else(|| vec![a]);
        group.extend(groups.get(&b).cloned().unwrap_or_else(|| vec![b]));
        group.sort_unstable();
        group.dedup();
        for member in &group {
            groups.insert(*member, group.clone());
        }
    }

    /// The items of `piece`'s outline: its group's items, and each pad a
    /// spoke joins to any of them once.
    pub(super) fn count(&self, piece: u32) -> usize {
        let single = [piece];
        let members: &[u32] = self
            .groups
            .get(&piece)
            .map_or(&single[..], |group| &group[..]);
        let mut pads: Vec<usize> = Vec::new();
        let mut count = 0;
        for member in members {
            count += self.items.get(member).copied().unwrap_or(0);
            for pad in self.pads.get(member).map_or(&[][..], |pads| &pads[..]) {
                if !pads.contains(pad) {
                    pads.push(*pad);
                }
            }
        }
        count + pads.len()
    }
}

/// What the kept spokes add to the pieces (`Router::touch_pieces`).
pub(super) struct Touches {
    /// Per record, which of its spokes the pad's own tracks touch.
    pub(super) touched: Vec<[bool; 4]>,
    /// Per piece, the pads joined to it by a spoke.
    pub(super) pads: HashMap<u32, Vec<usize>>,
    /// The pairs of pieces a pad's overlapping spokes join.
    pub(super) joins: Vec<(u32, u32)>,
}

impl Router {
    /// Per record, the pad's kept spokes as items of the pieces they land
    /// in: adds to `items` a piece's tracked spokes (a spoke the pad's own
    /// track touches), and returns the tracks' touches, the pads per piece
    /// and the pairs of pieces a pad's overlapping spokes join.
    /// `attached` is per record and spoke whether it stays joined to the
    /// pad.
    pub(super) fn touch_pieces(
        &self,
        model: &SpokeNet,
        records: &[SpokeRecord],
        spokes: &[Spoke],
        attached: &[[bool; 4]],
        holes: &PlatedHoles,
        items: &mut HashMap<u32, usize>,
    ) -> Touches {
        let description = model.description;
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
            let (terminal, outcomes) = (record.terminal, record.outcomes);
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
            let hole = holes.radius_at(
                description.terminals[terminal].label.as_str(),
                record.anchor,
            );
            let record_attached = &attached[record_index];
            let touched = self.spokes_touched(model, record, &kept, record_attached, hole);
            // Per piece: whether a spoke joins the pad to it, and whether
            // the pad's own track touches one (each an item of it).
            let mut seen: Vec<(u32, bool, bool)> = Vec::new();
            for (index, outcome) in outcomes.iter().enumerate() {
                if let SpokeOutcome::Good(piece) = outcome
                    && *piece != 0
                {
                    let joined = record_attached[index];
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
            let joined_spokes: Vec<&Spoke> = kept
                .iter()
                .copied()
                .filter(|spoke| {
                    record_attached[spoke.index]
                        && matches!(outcomes[spoke.index], SpokeOutcome::Good(piece) if piece != 0)
                })
                .collect();
            if joined_spokes.len() > 1 {
                let hole = holes.radius_at(
                    description.terminals[terminal].label.as_str(),
                    record.anchor,
                );
                spoke_joins.extend(overlapping_spoke_joins(&joined_spokes, &outcomes, hole));
            }
            touched_spokes.push(touched);
        }
        Touches {
            touched: touched_spokes,
            pads: pad_items,
            joins: spoke_joins,
        }
    }

    /// Which of `record`'s kept spokes (`kept`; `attached` per spoke
    /// whether it stays joined to the pad, `hole` the pad's plated hole's
    /// radius) the first runs of the pad's own branches on its layer touch.
    fn spokes_touched(
        &self,
        model: &SpokeNet,
        record: &SpokeRecord,
        kept: &[&Spoke],
        attached: &[bool; 4],
        hole: f64,
    ) -> [bool; 4] {
        let SpokeNet {
            net, state, skip, ..
        } = *model;
        let (terminal, layer) = (record.terminal, record.layer);
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
            for spoke in kept {
                if track_meets_spoke(spoke, attached[spoke.index], hole, &segments) {
                    touched[spoke.index] = true;
                }
            }
        }
        touched
    }

    /// Joins the pieces KiCad's outlines hold as one: the pairs a pad's
    /// overlapping spokes join (`joins`), then the necks the lattice cuts
    /// next to a piece a kept spoke lands in that is short of two items.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn join_necks(
        &self,
        model: &SpokeNet,
        probe: &fill::FillProbe,
        parent: &mut [usize],
        records: &[SpokeRecord],
        spokes: &[Spoke],
        pieces: &mut PieceItems,
        joins: Vec<(u32, u32)>,
    ) {
        // Necks the lattice cuts: a piece a kept spoke lands in, short of
        // two items, joins a piece next to it where KiCad's fill runs
        // between their nodes (a neck narrower than the lattice resolves;
        // census of 7 boards: 23 island spokes in KiCad outlines our map
        // splits, wrong verdicts on 4 pads such as Castor's U8.12).
        for (a, b) in joins {
            pieces.merge(parent, a, b);
        }
        let mut done: HashSet<(usize, u32)> = HashSet::new();
        for spoke in spokes {
            let SpokeOutcome::Good(piece) = records[spoke.record].outcomes[spoke.index] else {
                continue;
            };
            let layer = spoke.layer;
            if piece == 0 || pieces.count(piece) >= 2 || !done.insert((layer, piece)) {
                continue;
            }
            let Some(neighbours) = self.neck_neighbours(model, probe, spoke, piece) else {
                continue;
            };
            for label in neighbours {
                pieces.merge(parent, piece, label);
            }
        }
    }

    /// The pieces next to `piece` (where `spoke` lands) that KiCad's fill
    /// joins to it: from its nodes near the spoke's end (a bounded walk),
    /// each other piece within two nodes whose centres fill joins. None
    /// where the end has no nodes around it.
    fn neck_neighbours(
        &self,
        model: &SpokeNet,
        probe: &fill::FillProbe,
        spoke: &Spoke,
        piece: u32,
    ) -> Option<Vec<u32>> {
        let pitch = model.pitch;
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        let layer = spoke.layer;
        // The piece's nodes on the layer, from the node nearest the
        // spoke's end (a bounded walk).
        let end = spoke.end();
        let (x0, y0, x1, y1) = self.grid.node_range(
            crate::geometry::Aabb {
                minimum: end,
                maximum: end,
            }
            .inflated(2.0 * pitch + spoke.width),
        )?;
        let mut queue: Vec<usize> = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let cell = self.grid.index(x, y);
                if model.label_at(layer, cell) == piece {
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
                    let label = model.label_at(layer, other);
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
        Some(neighbours)
    }
}

/// Whether a track (`segments`, each with half its width) touches
/// `spoke`'s strip outside the pad's plated hole of radius `hole`: the
/// strip from the hole where the spoke stays `attached` to the pad, from
/// the ring half a gap out where it does not.
pub(super) fn track_meets_spoke(
    spoke: &Spoke,
    attached: bool,
    hole: f64,
    segments: &[(crate::geometry::Point, crate::geometry::Point, f64)],
) -> bool {
    let strip = {
        let (d, w) = (spoke.direction, spoke.width / 2.0);
        let normal = [-d[1] * w, d[0] * w];
        // A spoke cut off from the pad begins where the ring is
        // (a track touching it over the pad touches nothing).
        let start = if attached {
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
    if attached && hole > 0.0 {
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
    }
}

/// The pairs of pieces that a pad's kept spokes joined to it (`joined`,
/// each `Good` in `outcomes` with a piece) make one outline where they
/// overlap outside the pad's plated hole of radius `hole`.
pub(super) fn overlapping_spoke_joins(
    joined: &[&Spoke],
    outcomes: &[SpokeOutcome; 4],
    hole: f64,
) -> Vec<(u32, u32)> {
    let mut joins = Vec::new();
    let piece_of = |spoke: &Spoke| {
        if let SpokeOutcome::Good(piece) = outcomes[spoke.index] {
            piece
        } else {
            0
        }
    };
    for (position, a) in joined.iter().enumerate() {
        for b in &joined[position + 1..] {
            let (pa, pb) = (piece_of(a), piece_of(b));
            if pa == pb {
                continue;
            }
            let facing = a.direction[0] * b.direction[0] + a.direction[1] * b.direction[1] < -0.5;
            let reach = a.width.min(b.width) / 2.0
                * if facing {
                    1.0
                } else {
                    std::f64::consts::SQRT_2
                };
            if hole < reach - 1.0e-6 {
                joins.push((pa, pb));
            }
        }
    }
    joins
}

/// Pass two: per record, the spokes that count (into a piece with another
/// item, into another pad's spoke, or inside the terminal's pad) and the
/// pieces of its island spokes. Joins each pad (`terminal_base` plus its
/// terminal in `parent`) to what its counted spokes reach, and turns the
/// outcomes of spokes into pieces short of an item into `Island` (or
/// `NoFill` where a spoke cut off from its pad reaches a piece holding
/// nothing). `touched`, `attached` and `inner` are per record.
pub(super) fn tally_spokes(
    parent: &mut [usize],
    terminal_base: usize,
    records: &mut [SpokeRecord],
    touched: &[[bool; 4]],
    attached: &[[bool; 4]],
    inner: &[bool],
    pieces: &PieceItems,
) -> Vec<(usize, Vec<usize>)> {
    let mut tallies: Vec<(usize, Vec<usize>)> = Vec::new();
    for (((record, touched), joined), inner) in
        records.iter_mut().zip(touched).zip(attached).zip(inner)
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
                let count = pieces.count(piece);
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
    tallies
}

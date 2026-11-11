// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! `Router::stamp`: a net's new stamp list, marked footprint by footprint
//! for each querying class (escape stubs, branch nodes, vias, diagonal
//! steps' bisectors), then counted on the occupancy against its old list.

use super::*;

/// Where `stamp` marks the net's new stamps: a bit per map and cell
/// (`Router::stamp_mark`, taken for the marking) and the list they go to,
/// in marking order.
struct Marker<'a> {
    marks: &'a mut [u64],
    words: usize,
    stamped: &'a mut Stamped,
    size: (i64, i64),
    grid: &'a Grid,
}

impl Marker<'_> {
    /// `mark_footprint` of `offsets` around `cell` on `map` (`step`: the
    /// step a segment footprint runs along; `shape`: the exact shape asked
    /// where the lattice is not uniform).
    #[inline(always)]
    fn mark(
        &mut self,
        map: usize,
        cell: u32,
        offsets: &[(i32, i32)],
        step: (i64, i64),
        reach: i64,
        shape: Option<Shape>,
    ) {
        mark_footprint(
            self.marks,
            self.words,
            self.stamped,
            map,
            cell,
            offsets,
            step,
            self.size,
            reach,
            shape.map(|shape| (self.grid, shape)),
        );
    }
}

/// The maps of one querying class that a pass of `stamp` marks.
struct QueryMaps {
    querying: usize,
    layers: usize,
    /// The class's first trace map (its via map follows its layers').
    base: usize,
    /// The first diagonal map of every class.
    diagonal_base: usize,
    /// The first per-layer via map of every class (`SpanModel`), if any.
    span_base: Option<usize>,
}

impl QueryMaps {
    /// The class's via map.
    fn via(&self) -> usize {
        self.base + self.layers
    }

    /// The class's via map of `layer` (with via spans).
    fn layer_via(&self, layer: usize) -> Option<usize> {
        self.span_base
            .map(|first| first + self.querying * self.layers + layer)
    }

    /// The class's diagonal map of `layer` and `orientation`.
    fn diagonal(&self, layer: usize, orientation: usize) -> usize {
        self.diagonal_base + (self.querying * self.layers + layer) * 2 + orientation
    }
}

impl Router {
    pub(in crate::router) fn stamp(&mut self, net: NetId) {
        self.copper_changes += 1;
        // A full restamp: the new list is built first (marked, counts left
        // alone), then only the stamps that went or came change their
        // counts. The counts end as with the old list unstamped and the new
        // one stamped, without touching the stamps a net keeps (most of
        // them, when only a branch was rerouted).
        let old = std::mem::take(&mut self.nets[net as usize].stamped);
        let mut stamped =
            Stamped::with_capacity(old.len(), self.occupancy.len(), self.grid.cells());
        let branches = std::mem::take(&mut self.nets[net as usize].branches);
        let escapes = std::mem::take(&mut self.nets[net as usize].escapes);
        let words = self.grid.cells().div_ceil(64);
        let mut marks = std::mem::take(&mut self.stamp_mark);
        self.mark_stamps(
            &mut Marker {
                marks: &mut marks,
                words,
                stamped: &mut stamped,
                size: (self.grid.nx as i64, self.grid.ny as i64),
                grid: &self.grid,
            },
            net,
            &branches,
            &escapes,
        );
        self.stamp_mark = marks;
        self.settle_counts(&old, &stamped, words);
        let state = &mut self.nets[net as usize];
        state.branches = branches;
        state.escapes = escapes;
        stamped.trim();
        state.stamped = stamped;
    }

    /// Marks the footprints of `net`'s branches and escape stubs on every
    /// querying class's maps.
    fn mark_stamps(
        &self,
        marker: &mut Marker,
        net: NetId,
        branches: &[Branch],
        escapes: &HashMap<Node, Escape>,
    ) {
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        // Each node's class (narrower in a neck zone), all branches in one
        // list.
        let mut node_classes: Vec<usize> =
            Vec::with_capacity(branches.iter().map(|branch| branch.nodes.len()).sum());
        for branch in branches {
            node_classes.extend(
                branch
                    .nodes
                    .iter()
                    .map(|node| self.node_class(net, node.cell)),
            );
        }
        // With via spans, every via-map stamp of an item on one layer goes
        // to that layer's via map too (`SpanModel`).
        let span_base = self.spans.as_ref().map(|spans| spans.base);
        for querying in 0..self.board.classes.len() {
            let maps = QueryMaps {
                querying,
                layers,
                base: querying * (layers + 1),
                diagonal_base: self.board.classes.len() * (layers + 1),
                span_base,
            };
            let mut first = 0;
            for branch in branches {
                let classes = &node_classes[first..first + branch.nodes.len()];
                first += branch.nodes.len();
                for end in [branch.nodes[0], *branch.nodes.last().unwrap()] {
                    if let Some((cells, width, polyline)) = escapes.get(&end) {
                        self.mark_stub(marker, &maps, class, end, cells, *width, polyline);
                    }
                }
                for index in 0..branch.nodes.len() {
                    self.mark_node(marker, &maps, net, class, branch, classes, index);
                }
            }
        }
    }

    /// The stamp variant of `layer` (with per-layer class rules).
    fn variant_of(&self, layer: usize) -> usize {
        self.class_rules
            .as_ref()
            .map_or(0, |split| split.resolved.layer_variant[layer])
    }

    /// The stamps of `class`'s copper on `layer` against `querying`.
    fn layer_stamps(&self, class: usize, layer: usize, querying: usize) -> &Stamps {
        match &self.class_rules {
            None => &self.stamps[class][querying],
            Some(split) => {
                &self.stamps[class]
                    [split.resolved.layer_variant[layer] * self.board.classes.len() + querying]
            }
        }
    }

    /// Marks the escape stub at branch end `end` (its sampled `cells`,
    /// its `width` and `polyline`).
    #[allow(clippy::too_many_arguments)]
    fn mark_stub(
        &self,
        marker: &mut Marker,
        maps: &QueryMaps,
        class: usize,
        end: Node,
        cells: &[u32],
        width: f64,
        polyline: &[crate::geometry::Point],
    ) {
        // A stub at the neck width stamps the neck's footprint.
        let stub_class = match self.neck_of[class] {
            Some(neck)
                if width
                    < self
                        .class_rules
                        .as_ref()
                        .map_or(self.board.classes[class].trace_width, |split| {
                            split.width_on(&self.board, class, end.layer as usize)
                        })
                        - 1.0e-9 =>
            {
                neck
            }
            _ => class,
        };
        let stub_stamps = self.layer_stamps(stub_class, end.layer as usize, maps.querying);
        if !self.config.exact_escape_stubs {
            mark_snapped_stub(marker, maps, stub_stamps, end, cells);
        } else if Self::stub_on_lattice(polyline) {
            mark_lattice_stub(marker, maps, stub_stamps, end, cells);
        } else {
            // Off the lattice's lines: the nodes, steps and vias the
            // stub's geometry keeps away, measured (`stub_footprint`).
            let footprint = escapes::stub_footprint(
                &self.grid,
                polyline,
                end.cell,
                stub_stamps.trace_radius,
                stub_stamps.via_radius,
            );
            let unbounded = i64::MAX;
            let layer = end.layer as usize;
            for (map, offsets) in [
                (maps.base + layer, &footprint.trace),
                (maps.via(), &footprint.via),
                (maps.layer_via(layer).unwrap_or(maps.via()), &footprint.via),
                (maps.diagonal(layer, 0), &footprint.diagonal[0]),
                (maps.diagonal(layer, 1), &footprint.diagonal[1]),
            ] {
                marker.mark(map, end.cell, offsets, (1, 1), unbounded, None);
            }
        }
    }

    /// Marks node `index` of `branch` (`classes`: its nodes' classes):
    /// its trace footprints, the via it ends (once per via), the bisector
    /// of the diagonal step that reached it.
    #[allow(clippy::too_many_arguments)]
    fn mark_node(
        &self,
        marker: &mut Marker,
        maps: &QueryMaps,
        net: NetId,
        class: usize,
        branch: &Branch,
        classes: &[usize],
        index: usize,
    ) {
        let (nx, layers, base) = (self.grid.nx as i64, maps.layers, maps.base);
        let node = branch.nodes[index];
        // A node in a neck zone stamps its narrower footprint.
        let node_class = classes[index];
        let variant = self.variant_of(node.layer as usize);
        let own = &self.stamps[node_class][variant * self.board.classes.len() + maps.querying];
        let reach = own.fresh.reach;
        let x = node.cell as i64 % nx;
        let y = node.cell as i64 / nx;
        // The step from the node before, when that node stamped
        // the same footprints: only their leading edge is new.
        let mut step = None;
        let mut diagonal = None;
        let mut is_via = false;
        let mut via_before = false;
        if index > 0 {
            let previous = branch.nodes[index - 1];
            let (dx, dy) = (x - previous.cell as i64 % nx, y - previous.cell as i64 / nx);
            is_via = previous.cell == node.cell && previous.layer != node.layer;
            // (A step from a layer of another stamp variant
            // stamps whole footprints.)
            if classes[index - 1] == node_class
                && dx.abs() <= 1
                && dy.abs() <= 1
                && self.variant_of(previous.layer as usize) == variant
            {
                step = Some(((dx + 1) * 3 + dy + 1) as usize);
            }
            if !is_via && dx != 0 && dy != 0 {
                diagonal = Some((dx, dy));
            }
            via_before = index > 1
                && previous.cell == node.cell
                && branch.nodes[index - 2].cell == previous.cell
                && branch.nodes[index - 2].layer != previous.layer;
        }
        let same_layer = index > 0 && branch.nodes[index - 1].layer == node.layer;
        let map = base + node.layer as usize;
        let offsets = match step {
            Some(step) if same_layer => &own.fresh.trace_to_trace[step],
            _ => &own.trace_to_trace,
        };
        marker.mark(
            map,
            node.cell,
            offsets,
            (1, 1),
            reach,
            Some(Shape::Disc(own.radii.trace_to_trace)),
        );
        let map = base + layers;
        let offsets = match step {
            Some(step) => &own.fresh.trace_to_via[step],
            None => &own.trace_to_via,
        };
        marker.mark(
            map,
            node.cell,
            offsets,
            (1, 1),
            reach,
            Some(Shape::Disc(own.radii.trace_to_via)),
        );
        if let Some(map) = maps.layer_via(node.layer as usize) {
            // The layer's own map had the node before stamp
            // these offsets only on the same layer.
            let offsets = match step {
                Some(step) if same_layer => &own.fresh.trace_to_via[step],
                _ => &own.trace_to_via,
            };
            marker.mark(
                map,
                node.cell,
                offsets,
                (1, 1),
                reach,
                Some(Shape::Disc(own.radii.trace_to_via)),
            );
        }
        for orientation in 0..2 {
            let map = maps.diagonal(node.layer as usize, orientation);
            let offsets = match step {
                Some(step) if same_layer => &own.fresh.diagonal_block[orientation][step],
                _ => &own.diagonal_block[orientation],
            };
            marker.mark(
                map,
                node.cell,
                offsets,
                (1, 1),
                reach,
                Some(Shape::Starts(orientation, own.radii.diagonal_block)),
            );
        }
        // A via through several layers at one cell: the via
        // before stamped the same footprints at the same cell.
        if is_via && !via_before {
            self.mark_via(marker, maps, net, class, branch, index);
        }
        // A diagonal step covers the nodes on its bisector too
        // (the bisectors of a diagonal run are parallel lines
        // that share no cell: no leading edge to take).
        if let Some(step) = diagonal {
            let previous = branch.nodes[index - 1];
            let map = base + node.layer as usize;
            marker.mark(
                map,
                previous.cell,
                &own.diagonal_to_trace,
                step,
                reach,
                Some(Shape::Segment(own.radii.diagonal_to_trace)),
            );
            for map in [Some(base + layers), maps.layer_via(node.layer as usize)]
                .into_iter()
                .flatten()
            {
                marker.mark(
                    map,
                    previous.cell,
                    &own.diagonal_to_via,
                    step,
                    reach,
                    Some(Shape::Segment(own.radii.diagonal_to_via)),
                );
            }
        }
    }

    /// Marks the via that node `index` of `branch` ends, on each layer it
    /// pierces.
    fn mark_via(
        &self,
        marker: &mut Marker,
        maps: &QueryMaps,
        net: NetId,
        class: usize,
        branch: &Branch,
        index: usize,
    ) {
        let (layers, base, querying) = (maps.layers, maps.base, maps.querying);
        let cell = branch.nodes[index].cell;
        // The via's layers: all of them, or with spans the
        // shortest span holding the layers its chain at
        // this cell visits.
        let pierced = match &self.spans {
            Some(spans) if spans.takes_spans(net) => {
                spans.chain_mask(net, chain_layers(&branch.nodes, index))
            }
            _ => crate::board::LayerMask::MAX,
        };
        for layer in 0..layers {
            if pierced & (1 << layer) == 0 {
                continue;
            }
            let stamps = self.layer_stamps(class, layer, querying);
            let map = base + layer;
            marker.mark(
                map,
                cell,
                &stamps.via_to_trace,
                (1, 1),
                stamps.fresh.reach,
                Some(Shape::Disc(stamps.radii.via_to_trace)),
            );
        }
        // Via to via is the largest of every layer's.
        let stamps = &self.stamps[class][querying];
        let reach = stamps.fresh.reach;
        let map = base + layers;
        marker.mark(
            map,
            cell,
            &stamps.via_to_via,
            (1, 1),
            reach,
            Some(Shape::Disc(stamps.radii.via_to_via)),
        );
        // With spans, on each layer it pierces: two vias
        // meet only where both are (KiCad keeps neither
        // clearance nor hole-to-hole between vias of
        // disjoint spans).
        for layer in 0..layers {
            if pierced & (1 << layer) == 0 {
                continue;
            }
            let Some(map) = maps.layer_via(layer) else {
                break;
            };
            marker.mark(
                map,
                cell,
                &stamps.via_to_via,
                (1, 1),
                reach,
                Some(Shape::Disc(stamps.radii.via_to_via)),
            );
        }
        for layer in 0..layers {
            if pierced & (1 << layer) == 0 {
                continue;
            }
            let stamps = self.layer_stamps(class, layer, querying);
            for orientation in 0..2 {
                let map = maps.diagonal(layer, orientation);
                marker.mark(
                    map,
                    cell,
                    &stamps.diagonal_block_via[orientation],
                    (1, 1),
                    stamps.fresh.reach,
                    Some(Shape::Starts(orientation, stamps.radii.diagonal_block_via)),
                );
            }
        }
    }

    /// Counts the new list `stamped` on the occupancy against the `old`
    /// one, clearing the marks `mark_stamps` left.
    fn settle_counts(&mut self, old: &Stamped, stamped: &Stamped, words: usize) {
        // An old stamp still marked is a new one too: its count stays, and
        // its mark goes so that the new list passes it over; the others
        // go (the count down, the tile's claim with the last of a cell).
        // The count a stamp AHEAD entries on is asked for early: the maps
        // are larger than the core's cache and a list visits dozens of them
        // (each list decoded once for both: `iter_ahead`).
        let nxu = self.grid.nx;
        const AHEAD: usize = 32;
        for ((map, cell), ahead) in old.iter_ahead::<AHEAD>() {
            if let Some((map, cell)) = ahead {
                self.occupancy.prefetch(map as usize, cell as usize);
            }
            let (map, cell) = (map as usize, cell as usize);
            let bit = 1u64 << (cell % 64);
            if self.stamp_mark[map * words + cell / 64] & bit != 0 {
                self.stamp_mark[map * words + cell / 64] &= !bit;
                continue;
            }
            if self.occupancy.decrement(map, cell) {
                self.tile_claimed[map][(cell / nxu / TILE) * self.tiles_x + (cell % nxu) / TILE] -=
                    1;
            }
        }
        // A new stamp still marked was not stamped before: it counts.
        for ((map, cell), ahead) in stamped.iter_ahead::<AHEAD>() {
            if let Some((map, cell)) = ahead {
                self.occupancy.prefetch(map as usize, cell as usize);
            }
            let (map, cell) = (map as usize, cell as usize);
            let bit = 1u64 << (cell % 64);
            if self.stamp_mark[map * words + cell / 64] & bit == 0 {
                continue;
            }
            self.stamp_mark[map * words + cell / 64] &= !bit;
            if self.occupancy.increment(map, cell) {
                self.tile_claimed[map][(cell / nxu / TILE) * self.tiles_x + (cell % nxu) / TILE] +=
                    1;
            }
        }
    }
}

/// An escape stub off exact geometry: the lattice's discs grown by the
/// snapping error, at every sampled node.
fn mark_snapped_stub(
    marker: &mut Marker,
    maps: &QueryMaps,
    stub_stamps: &Stamps,
    end: Node,
    cells: &[u32],
) {
    let reach = stub_stamps.fresh.reach;
    let layer = end.layer as usize;
    for cell in cells {
        for (map, offsets, radius) in [
            (
                Some(maps.base + layer),
                &stub_stamps.stub_to_trace,
                stub_stamps.radii.stub_to_trace,
            ),
            (
                Some(maps.via()),
                &stub_stamps.stub_to_via,
                stub_stamps.radii.stub_to_via,
            ),
            (
                maps.layer_via(layer),
                &stub_stamps.stub_to_via,
                stub_stamps.radii.stub_to_via,
            ),
        ] {
            let Some(map) = map else {
                continue;
            };
            marker.mark(
                map,
                *cell,
                offsets,
                (1, 1),
                reach,
                Some(Shape::Disc(radius)),
            );
        }
        for orientation in 0..2 {
            marker.mark(
                maps.diagonal(layer, orientation),
                *cell,
                &stub_stamps.stub_diagonal_block[orientation],
                (1, 1),
                reach,
                Some(Shape::Starts(
                    orientation,
                    stub_stamps.radii.stub_diagonal_block,
                )),
            );
        }
    }
}

/// An exact escape stub along a lattice line: lattice copper, its nodes
/// stamp a branch node's footprints.
fn mark_lattice_stub(
    marker: &mut Marker,
    maps: &QueryMaps,
    stub_stamps: &Stamps,
    end: Node,
    cells: &[u32],
) {
    let reach = stub_stamps.fresh.reach;
    let layer = end.layer as usize;
    for cell in cells {
        marker.mark(
            maps.base + layer,
            *cell,
            &stub_stamps.trace_to_trace,
            (1, 1),
            reach,
            Some(Shape::Disc(stub_stamps.radii.trace_to_trace)),
        );
        for map in [Some(maps.via()), maps.layer_via(layer)]
            .into_iter()
            .flatten()
        {
            marker.mark(
                map,
                *cell,
                &stub_stamps.trace_to_via,
                (1, 1),
                reach,
                Some(Shape::Disc(stub_stamps.radii.trace_to_via)),
            );
        }
        for orientation in 0..2 {
            marker.mark(
                maps.diagonal(layer, orientation),
                *cell,
                &stub_stamps.diagonal_block[orientation],
                (1, 1),
                reach,
                Some(Shape::Starts(orientation, stub_stamps.radii.diagonal_block)),
            );
        }
    }
}

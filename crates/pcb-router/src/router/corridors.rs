// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Corridor stubs: the way out of a pad's pocket where the lattice does
//! not pass it (docs/router.md, "Corridors of pocketed pads").
//!
//! A pad without a legal node of its own leaves by escape stubs to the
//! legal nodes nearby (`escape_nodes`). Where those nodes lie in a pocket
//! of its own part's pads, the way out may be a corridor a track passes
//! only near its centre line: OpenFC's U10 (an X2SON-6 with custom pads)
//! holds its +3.3V pad 5 between its corner pads, and the gaps between
//! them leave the class's 0.09 mm track with 0.09 mm clearance 15 and 30
//! um of play. Whether a node row falls there is the lattice's phase
//! against the part's position, so the pad connected on some placements
//! and stayed open on others. When the lattice closes a pad's pocket, a
//! corridor stub finds the way out off the lattice: a path through the
//! legal region at 5 um from the pad's anchor to a legal node beyond the
//! pocket, straightened into a polyline every segment of which keeps the
//! exact clearances, and kept as the pad's escape to that node.

use super::*;

/// `PCB_ROUTER_CORRIDOR_ESCAPES` when set: overrides
/// `Config::corridor_escapes` (`0` or empty off, anything else on).
pub fn corridor_escapes_override() -> Option<bool> {
    let value = std::env::var("PCB_ROUTER_CORRIDOR_ESCAPES").ok()?;
    Some(!(value.is_empty() || value == "0"))
}

/// How far beyond its pad's box a pad's pocket must reach to be a way out,
/// mm (as `pad_exits`'s `ESCAPE`).
const POCKET_ESCAPE: f64 = 1.0;

/// The most lattice nodes a pocket's flood takes before it counts the
/// pocket as open.
const POCKET_NODES: usize = 20_000;

/// The path's raster, mm.
const STEP: f64 = 0.005;

/// The least slack the path keeps from every obstacle, tried in turn: the
/// widest first, so that the path runs near a corridor's centre line.
const MARGINS: [f64; 4] = [0.01, 0.005, 0.002, 0.0005];

/// Something a path's point keeps away from: an obstacle's shape and the
/// distance a track's centre must keep from it, or the outline.
enum Keep<'a> {
    Shape(&'a crate::geometry::Shape, f64),
    Edge(crate::geometry::Point, crate::geometry::Point, f64),
}

impl Router {
    /// The pocket of `nodes` (a terminal's legal nodes on `layer`) on the
    /// lattice: the legal nodes and steps of `net` from them. Open when
    /// it reaches a node beyond `out_box`, a node where the net's via may
    /// stand (on a board of more than one layer) or a node in another pad
    /// of the net; else the closed pocket's nodes.
    #[allow(clippy::too_many_arguments)]
    fn pocket(
        &self,
        net: NetId,
        layer: usize,
        class: usize,
        own_pad: usize,
        nodes: &[usize],
        out_box: crate::geometry::Aabb,
        counts: &mut [u64; 2],
    ) -> Option<HashSet<usize>> {
        let statics = &self.statics[class];
        let others: Vec<&crate::geometry::Shape> = self.board.nets[net as usize]
            .terminals
            .iter()
            .filter(|other| other.layers & (1 << layer) != 0 && other.pad != own_pad)
            .map(|other| &self.board.obstacles[other.pad].shape)
            .filter(|shape| shape.aabb().intersects(out_box))
            .collect();
        let mut reached: HashSet<usize> = nodes.iter().copied().collect();
        let mut stack: Vec<usize> = nodes.to_vec();
        while let Some(cell) = stack.pop() {
            counts[0] += 1;
            let point = self.grid.center_of(cell);
            if point[0] < out_box.minimum[0]
                || point[0] > out_box.maximum[0]
                || point[1] < out_box.minimum[1]
                || point[1] > out_box.maximum[1]
                || (self.board.layer_count > 1 && statics.via_allowed(cell, net))
                || reached.len() > POCKET_NODES
            {
                return None;
            }
            let (blocked, owner) = statics.edges_at(layer, cell);
            for direction in 0..8 {
                let Some(next) = self.grid.neighbor(cell, direction) else {
                    continue;
                };
                if reached.contains(&next)
                    || (blocked & (1 << direction) != 0 && owner != crate::grid::owner16(net))
                    || !statics.trace_allowed(layer, next, net)
                {
                    continue;
                }
                let at = self.grid.center_of(next);
                if others.iter().any(|shape| shape.contains(at)) {
                    return None;
                }
                reached.insert(next);
                stack.push(next);
            }
        }
        Some(reached)
    }

    /// A corridor stub for a single-layer terminal of `net` whose escape
    /// nodes (`nodes`, from `escape_nodes`) lie in a pocket the lattice
    /// closes (`pocket`): a path at `STEP` through the region where the
    /// net's track (its width on the layer, else its neck) keeps its
    /// clearances, from the terminal's anchor to a legal node outside the
    /// pocket whose own pocket is open, straightened (`straighten_path`).
    /// Added to `nodes` and `escapes` as an escape of that node.
    pub(super) fn corridor_escape(
        &self,
        net: NetId,
        terminal: &crate::board::Terminal,
        nodes: &mut Vec<Node>,
        escapes: &mut HashMap<Node, Escape>,
        counts: &mut [u64; 2],
    ) {
        // The census (docs/router.md, "Corridors of pocketed pads"): one
        // line on stderr per escape pad, its pocket and its stub.
        let census = std::env::var_os("PCB_ROUTER_CORRIDOR_CENSUS").is_some();
        if !(self.config.corridor_escapes || census) || terminal.layers.count_ones() != 1 {
            return;
        }
        let net_name = &self.board.nets[net as usize].name;
        let layer = terminal.layers.trailing_zeros() as usize;
        let class = self.board.nets[net as usize].class;
        let pad_box = self.board.obstacles[terminal.pad].shape.aabb();
        let out_box = pad_box.inflated(POCKET_ESCAPE);
        let starts: Vec<usize> = nodes
            .iter()
            .filter(|node| node.layer as usize == layer)
            .map(|node| node.cell as usize)
            .collect();
        // The pocket at the widest that leaves: the neck's where the class
        // has one.
        let flood_class = self.neck_of[class].unwrap_or(class);
        let Some(pocket) = self.pocket(
            net,
            layer,
            flood_class,
            terminal.pad,
            &starts,
            out_box,
            counts,
        ) else {
            if census {
                eprintln!(
                    "corridor census: pitch {} origin {:?} pad {} net {net_name} layer {layer} open",
                    self.grid.pitch, self.grid.origin, terminal.label
                );
            }
            return;
        };
        let track = self.width_on(class, layer);
        let neck = self.neck_width_on(class, layer);
        let mut widths = vec![(track, class)];
        if let Some(neck_class) = self.neck_of[class].filter(|_| neck < track) {
            widths.push((neck, neck_class));
        }
        let debug = std::env::var_os("PCB_ROUTER_DEBUG").is_some();
        for (width, width_class) in widths {
            let keeps = self.corridor_keeps(net, layer, class, terminal.anchor, out_box, width);
            let found = self.corridor_path(
                net,
                layer,
                width_class,
                terminal,
                out_box,
                &keeps,
                &pocket,
                escapes,
                counts,
            );
            let Some((cell, polyline, margin)) = found else {
                continue;
            };
            if !polyline.windows(2).all(|pair| {
                self.stub_is_clear(net, layer, pair[0], pair[1], width)
                    && self.stub_keeps_edge(pair[0], pair[1], width)
            }) {
                continue;
            }
            if census {
                eprintln!(
                    "corridor census: pitch {} origin {:?} pad {} net {net_name} layer {layer} closed {} stub {} points width {width} margin {margin}",
                    self.grid.pitch,
                    self.grid.origin,
                    terminal.label,
                    pocket.len(),
                    polyline.len()
                );
            }
            if !self.config.corridor_escapes {
                return;
            }
            if debug {
                eprintln!(
                    "corridor escape for {} at {:?}: {} points to {:?}, width {width}",
                    terminal.label,
                    terminal.anchor,
                    polyline.len(),
                    self.grid.center_of(cell)
                );
            }
            let node = Node {
                layer: layer as u8,
                cell: cell as u32,
            };
            let cells = self.sampled_stub_cells(&polyline);
            escapes.insert(node, (cells, width, polyline));
            nodes.push(node);
            return;
        }
        if census {
            eprintln!(
                "corridor census: pitch {} origin {:?} pad {} net {net_name} layer {layer} closed {} none",
                self.grid.pitch,
                self.grid.origin,
                terminal.label,
                pocket.len()
            );
        }
        if debug {
            eprintln!(
                "corridor escape for {} at {:?}: its pocket of {} nodes is closed, no way out",
                terminal.label,
                terminal.anchor,
                pocket.len()
            );
        }
    }

    /// What a corridor path of `width` keeps away from in `window`: other
    /// nets' copper, keepouts and holes on `layer` with the clearances
    /// `stub_is_clear` asks (no fit tolerance), and the outline's edges.
    fn corridor_keeps(
        &self,
        net: NetId,
        layer: usize,
        class: usize,
        anchor: crate::geometry::Point,
        window: crate::geometry::Aabb,
        width: f64,
    ) -> Vec<Keep<'_>> {
        let half = width / 2.0;
        let mut keeps = Vec::new();
        for obstacle in &self.board.obstacles {
            if obstacle.net == Some(net)
                || obstacle.layers & (1 << layer) == 0
                || !obstacle.blocks_tracks
            {
                continue;
            }
            if obstacle.kind == crate::board::ObstacleKind::Keepout
                && obstacle.label.starts_with("solder mask opening")
                && obstacle.shape.contains(anchor)
            {
                continue;
            }
            let required = half
                + match obstacle.kind {
                    crate::board::ObstacleKind::Copper => {
                        self.obstacle_clearance(class, crate::rules::TRACK, obstacle, layer)
                    }
                    crate::board::ObstacleKind::Keepout => obstacle.clearance,
                    crate::board::ObstacleKind::Hole => {
                        self.board.hole_clearance.max(obstacle.clearance)
                    }
                };
            if obstacle.shape.aabb().intersects(window.inflated(required)) {
                keeps.push(Keep::Shape(&obstacle.shape, required));
            }
        }
        let required = half + self.board.edge_clearance + SAFETY;
        let grown = window.inflated(required);
        for (a, b) in crate::geometry::polygon_edges(&self.board.outline) {
            let edge = crate::geometry::Aabb {
                minimum: [a[0].min(b[0]), a[1].min(b[1])],
                maximum: [a[0].max(b[0]), a[1].max(b[1])],
            };
            if edge.intersects(grown) {
                keeps.push(Keep::Edge(a, b, required));
            }
        }
        keeps
    }

    /// The path of a corridor stub (`corridor_escape`): breadth first on
    /// the `STEP` raster of `window` through the points whose slack
    /// against `keeps` is at least a margin (`MARGINS`, the widest that
    /// finds one), from `anchor` to the first raster point at a legal node
    /// of `class` outside `pocket` (and not another pad's escape) whose
    /// own pocket is open; straightened. The node, the polyline and the
    /// margin it kept.
    #[allow(clippy::too_many_arguments)]
    fn corridor_path(
        &self,
        net: NetId,
        layer: usize,
        class: usize,
        terminal: &crate::board::Terminal,
        window: crate::geometry::Aabb,
        keeps: &[Keep<'_>],
        pocket: &HashSet<usize>,
        escapes: &HashMap<Node, Escape>,
        counts: &mut [u64; 2],
    ) -> Option<(usize, Vec<crate::geometry::Point>, f64)> {
        let anchor = terminal.anchor;
        let nx = ((window.maximum[0] - window.minimum[0]) / STEP).ceil() as usize + 1;
        let ny = ((window.maximum[1] - window.minimum[1]) / STEP).ceil() as usize + 1;
        let at = |index: usize| {
            [
                window.minimum[0] + (index % nx) as f64 * STEP,
                window.minimum[1] + (index / nx) as f64 * STEP,
            ]
        };
        let mut slack = vec![f32::NAN; nx * ny];
        let mut slack_at = |index: usize, counts: &mut [u64; 2]| -> f64 {
            if slack[index].is_nan() {
                let point = at(index);
                let mut least = f64::INFINITY;
                for keep in keeps {
                    counts[1] += 1;
                    let value = match keep {
                        Keep::Shape(shape, required) => shape.distance_to_point(point) - required,
                        Keep::Edge(a, b, required) => {
                            crate::geometry::point_segment_distance(point, *a, *b) - required
                        }
                    };
                    least = least.min(value);
                }
                if !crate::geometry::point_in_polygon(point, &self.board.outline) {
                    least = -1.0;
                }
                slack[index] = least as f32;
            }
            slack[index] as f64
        };
        let start = {
            let x = ((anchor[0] - window.minimum[0]) / STEP).round() as usize;
            let y = ((anchor[1] - window.minimum[1]) / STEP).round() as usize;
            y.min(ny - 1) * nx + x.min(nx - 1)
        };
        let statics = &self.statics[class];
        let mut closed: HashSet<usize> = pocket.clone();
        let mut open: HashSet<usize> = HashSet::new();
        for margin in MARGINS {
            let mut parent = vec![u32::MAX; nx * ny];
            parent[start] = start as u32;
            let mut queue = std::collections::VecDeque::from([start]);
            while let Some(index) = queue.pop_front() {
                let point = at(index);
                // A legal node whose centre lies in this raster cell.
                if let Some(cell) = self.grid.nearest_node(point) {
                    let center = self.grid.center_of(cell);
                    let node = Node {
                        layer: layer as u8,
                        cell: cell as u32,
                    };
                    if (center[0] - point[0]).abs() <= STEP / 2.0
                        && (center[1] - point[1]).abs() <= STEP / 2.0
                        && !closed.contains(&cell)
                        && !escapes.contains_key(&node)
                        && statics.trace_allowed(layer, cell, net)
                    {
                        let leaves = open.contains(&cell)
                            || match self.pocket(
                                net,
                                layer,
                                class,
                                terminal.pad,
                                &[cell],
                                window,
                                counts,
                            ) {
                                None => true,
                                Some(other) => {
                                    closed.extend(other);
                                    false
                                }
                            };
                        if leaves {
                            open.insert(cell);
                            let mut path = vec![center];
                            let mut at_index = index;
                            while at_index != start {
                                path.push(at(at_index));
                                at_index = parent[at_index] as usize;
                            }
                            path.push(anchor);
                            path.reverse();
                            return Some((cell, straighten_path(&path, keeps), margin));
                        }
                    }
                }
                let (x, y) = ((index % nx) as i64, (index / nx) as i64);
                for (dx, dy) in crate::grid::DIRECTIONS {
                    let (x, y) = (x + dx as i64, y + dy as i64);
                    if x < 0 || y < 0 || x >= nx as i64 || y >= ny as i64 {
                        continue;
                    }
                    let next = y as usize * nx + x as usize;
                    if parent[next] != u32::MAX || slack_at(next, counts) < margin {
                        continue;
                    }
                    parent[next] = index as u32;
                    queue.push_back(next);
                }
            }
        }
        None
    }
}

/// `path` with the fewest corners: from each corner on, the farthest
/// later point a straight segment reaches keeping every `keeps` distance.
fn straighten_path(
    path: &[crate::geometry::Point],
    keeps: &[Keep<'_>],
) -> Vec<crate::geometry::Point> {
    let clear = |a: crate::geometry::Point, b: crate::geometry::Point| {
        keeps.iter().all(|keep| match keep {
            Keep::Shape(shape, required) => shape.distance_to_segment(a, b) >= *required,
            Keep::Edge(p, q, required) => {
                crate::geometry::segment_segment_distance(a, b, *p, *q) >= *required
            }
        })
    };
    let mut points = vec![path[0]];
    let mut from = 0;
    while from + 1 < path.len() {
        let mut to = from + 1;
        while to + 1 < path.len() && clear(path[from], path[to + 1]) {
            to += 1;
        }
        points.push(path[to]);
        from = to;
    }
    points
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Pads without a way out, on the statics alone (`ExitProbe`): a pad from
//! whose legal nodes (the router's terminal nodes: lattice nodes well
//! inside its copper where its net's track may lie) no lattice path of
//! legal nodes and steps reaches `ESCAPE` beyond the pad, another pad of
//! its net, or a node where its net's via may stand, lies in a pocket that
//! fixed copper, keepouts and the board edge close at its class's width.
//! No route can connect it: no move of the router opens it, only moving or
//! turning a part does (docs/router.md, "Crossings decided in the repair":
//! OpenAirScope's CM1624 and the laptop keyboard's nRF aQFN). The placer
//! asks it of a placement (pcb-kicad's `pad_exits`, docs/placer.md "Pads
//! without a way out (2026-11-04)").
//!
//! The fan-out's ways out (`fanout_plan`: straight runs from the node at
//! the pad's centre) say less: a pad the router enters off its centre
//! line, or leaves with a bend, has a way where every straight run from
//! its centre is closed (DSP-ADAU1452's LFCSP: 60 pads without a straight
//! way on the 2026-11-04 placement, 55 of them connected in the night's
//! result). The probe asks the router's own question instead, on the same
//! maps.
//!
//! The probe builds the static maps only on windows of the board's lattice
//! around the pads asked about (the lattice `Grid::choose` takes for the
//! whole board, cut to a window per 4 mm tile of pads): the router's maps
//! there, at a small part of their cost.

use super::*;

/// How far beyond its pad's box a pad's pocket must reach to be a way
/// out, mm.
const ESCAPE: f64 = 1.0;

/// Pads are grouped by tiles of this side (mm) of their anchors, a window
/// of maps each.
const TILE_MM: f64 = 4.0;

/// What closes a closed pad's pocket: the item nearest to a node at its
/// rim.
#[derive(Clone, Debug, PartialEq)]
pub struct ExitBlocker {
    /// The item's label (`REF.PAD` for a pad), or `the board edge`.
    pub label: String,
    /// The obstacle, `None` for the edge.
    pub obstacle: Option<usize>,
    /// The pad's own part's copper (its exposed pad, its neighbours).
    pub own: bool,
    /// The rim nodes it closes.
    pub nodes: usize,
}

/// A pad with no way out (`ExitProbe::closed_pads`).
#[derive(Clone, Debug, PartialEq)]
pub struct ClosedPad {
    /// The terminal's label (`REF.PAD`) and its net and layer.
    pub label: String,
    pub net: String,
    pub layer: usize,
    /// The nearest pad of another net on the layer, centre to centre.
    pub pitch: f64,
    /// The legal nodes its pocket holds.
    pub pocket: usize,
    /// What closes the pocket, the most rim nodes first.
    pub blockers: Vec<ExitBlocker>,
}

impl ClosedPad {
    /// Only the pad's own part closes it: a footprint that walls its pad
    /// in (no placement opens it).
    pub fn own_only(&self) -> bool {
        self.blockers.iter().all(|blocker| blocker.own)
    }

    /// The distinct items that close it, other parts' first (then by the
    /// rim nodes they close).
    pub fn blocked_by(&self) -> Vec<String> {
        let mut blockers: Vec<&ExitBlocker> = self.blockers.iter().collect();
        blockers.sort_by(|a, b| {
            a.own
                .cmp(&b.own)
                .then(b.nodes.cmp(&a.nodes))
                .then(a.label.cmp(&b.label))
        });
        let mut names: Vec<String> = Vec::new();
        for blocker in blockers {
            if !names.contains(&blocker.label) {
                names.push(blocker.label.clone());
            }
        }
        names
    }
}

/// The part a label names: the text before its first `.` or space
/// (`U14.10`, `U14 copper ...`).
pub fn label_part(label: &str) -> &str {
    label.split_once(['.', ' ']).map_or(label, |(part, _)| part)
}

/// The probe: the board as the router sees it (its classes split by the
/// rules, with their neck variants) and the board's lattice. Moving a
/// part is moving its obstacles and terminals in `board_mut` (the same
/// indices as the board it was built from).
pub struct ExitProbe {
    board: Board,
    neck_of: Vec<Option<usize>>,
    class_rules: Option<Arc<crate::rules::ClassRules>>,
    grid: Grid,
    neck_reach: f64,
    /// Per net: a connected pour carries it (its pads join by spokes).
    poured: Vec<bool>,
    /// Per part (`label_part`), its terminals (net, terminal).
    terminals: HashMap<String, Vec<(NetId, usize)>>,
    /// What the probe did since `take_work_seconds`, counted.
    counts: ProbeCounts,
}

/// What a probe did, counted (`ExitProbe::take_work_seconds` prices it).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProbeCounts {
    /// Probes built (each: the board with its neck classes, the lattice,
    /// the parts' terminals) and the obstacles and terminals they hold.
    pub probes: u64,
    pub probe_items: u64,
    /// Windows of maps, the maps built and the obstacles each build went
    /// through, their nodes and the maps' own counts (`StaticMaps::tests`,
    /// `vertex_tests`, `tight_tests`).
    pub windows: u64,
    pub builds: u64,
    pub build_obstacles: u64,
    pub map_nodes: u64,
    pub tests: u64,
    pub edges: u64,
    pub steps: u64,
    /// Nodes the pockets took, and obstacles measured at their rims.
    pub pocket_nodes: u64,
    pub measures: u64,
    /// Pads tested, and of them those without a legal node of their own
    /// (the router leaves them by its escape stubs, which the probe does
    /// not judge: never reported closed).
    pub pads: u64,
    pub escape_pads: u64,
}

/// Reference nanoseconds per item of a probe built, per obstacle a map
/// build goes through (its passes over the board's obstacles), per node
/// of a pocket and per obstacle measured at a rim; the maps' own counts at
/// the router's build prices (`BUILD_*_NANOS`).
const PROBE_ITEM_NANOS: f64 = 400.0;
const BUILD_OBSTACLE_NANOS: f64 = 60.0;
const POCKET_NANOS: f64 = 40.0;
const MEASURE_NANOS: f64 = 60.0;

/// The CPU time of the probes against their counts at the prices above:
/// 1.5 to 3.8 on the census of sixteen boards' placements (2026-11-04,
/// one thread on the shared machine), the median 2.5.
const CALIBRATION: f64 = 2.5;

/// The work of `counts`, reference nanoseconds.
pub fn work_nanos(counts: &ProbeCounts) -> f64 {
    CALIBRATION
        * (counts.probe_items as f64 * PROBE_ITEM_NANOS
            + counts.build_obstacles as f64 * BUILD_OBSTACLE_NANOS
            + counts.map_nodes as f64 * BUILD_MAP_NANOS
            + counts.tests as f64 * BUILD_TEST_NANOS
            + counts.edges as f64 * BUILD_EDGE_NANOS
            + counts.steps as f64 * BUILD_STEP_NANOS
            + counts.pocket_nodes as f64 * POCKET_NANOS
            + counts.measures as f64 * MEASURE_NANOS)
}

impl ExitProbe {
    pub fn new(board: &Board, config: &Config) -> Self {
        let (board, neck_of, class_rules) = Router::with_neck_classes(board, config);
        let grid = crate::pin_lines::lattice(&board, &config.pitches, config.pin_lines);
        let mut poured = vec![false; board.nets.len()];
        for plane in board.planes.iter().filter(|plane| plane.connect) {
            if let Some(entry) = poured.get_mut(plane.net as usize) {
                *entry = true;
            }
        }
        let mut terminals: HashMap<String, Vec<(NetId, usize)>> = HashMap::new();
        for (net, description) in board.nets.iter().enumerate() {
            for (index, terminal) in description.terminals.iter().enumerate() {
                terminals
                    .entry(label_part(&terminal.label).to_string())
                    .or_default()
                    .push((net as NetId, index));
            }
        }
        let items = (board.obstacles.len()
            + board
                .nets
                .iter()
                .map(|net| net.terminals.len())
                .sum::<usize>()) as u64;
        Self {
            neck_reach: config.neck_reach,
            board,
            neck_of,
            class_rules,
            grid,
            poured,
            terminals,
            counts: ProbeCounts {
                probes: 1,
                probe_items: items,
                ..Default::default()
            },
        }
    }

    /// The parts with terminals, by name, sorted.
    pub fn parts(&self) -> Vec<String> {
        let mut parts: Vec<String> = self.terminals.keys().cloned().collect();
        parts.sort();
        parts
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    /// The board, to move parts' obstacles and terminals in.
    pub fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }

    /// The work done since the last call, in seconds of the reference
    /// machine.
    pub fn take_work_seconds(&mut self) -> f64 {
        let counts = std::mem::take(&mut self.counts);
        work_nanos(&counts) / 1.0e9 * reference_time_scale()
    }

    /// The counts since the last `take_work_seconds` (for calibrating
    /// `work_nanos`).
    pub fn counts(&self) -> ProbeCounts {
        self.counts
    }

    fn width_on(&self, class: usize, layer: usize) -> f64 {
        match &self.class_rules {
            Some(split) => split.width_on(&self.board, class, layer),
            None => self.board.classes[class].trace_width,
        }
    }

    /// The pads of the part `part` (a reference) without a way out (see
    /// the module): single-layer pads of nets with another pad that no
    /// connected pour carries, with legal nodes of their own (a pad
    /// without one takes the router's escape stubs, which this does not
    /// judge).
    pub fn closed_pads(&mut self, part: &str) -> Vec<ClosedPad> {
        let Some(terminals) = self.terminals.get(part).cloned() else {
            return Vec::new();
        };
        let tested: Vec<(NetId, usize)> = terminals
            .into_iter()
            .filter(|&(net, index)| {
                let description = &self.board.nets[net as usize];
                description.terminals.len() >= 2
                    && !self.poured[net as usize]
                    && description.terminals[index].layers.count_ones() == 1
            })
            .collect();
        let mut tiles: std::collections::BTreeMap<(i64, i64), Vec<(NetId, usize)>> =
            Default::default();
        for &(net, index) in &tested {
            let anchor = self.board.nets[net as usize].terminals[index].anchor;
            let tile = (
                (anchor[0] / TILE_MM).floor() as i64,
                (anchor[1] / TILE_MM).floor() as i64,
            );
            tiles.entry(tile).or_default().push((net, index));
        }
        let mut closed = Vec::new();
        for pads in tiles.into_values() {
            closed.extend(self.closed_in_window(&pads));
        }
        closed
    }

    fn closed_in_window(&mut self, tested: &[(NetId, usize)]) -> Vec<ClosedPad> {
        let board = &self.board;
        let mut bounds: Option<crate::geometry::Aabb> = None;
        for &(net, index) in tested {
            let terminal = &board.nets[net as usize].terminals[index];
            let pad = board.obstacles[terminal.pad].shape.aabb();
            bounds = Some(match bounds {
                None => pad,
                Some(bounds) => crate::geometry::Aabb {
                    minimum: [
                        bounds.minimum[0].min(pad.minimum[0]),
                        bounds.minimum[1].min(pad.minimum[1]),
                    ],
                    maximum: [
                        bounds.maximum[0].max(pad.maximum[0]),
                        bounds.maximum[1].max(pad.maximum[1]),
                    ],
                },
            });
        }
        let Some(bounds) = bounds else {
            return Vec::new();
        };
        let window = bounds.inflated(ESCAPE + 3.0 * self.grid.pitch);
        let Some((x0, y0, x1, y1)) = self.grid.node_range(window) else {
            return Vec::new();
        };
        let grid = self.grid.window(x0, y0, x1, y1);
        let mut counts = ProbeCounts {
            windows: 1,
            ..Default::default()
        };
        // The static maps of the classes the tested nets route at here.
        let mut statics: HashMap<usize, StaticMaps> = HashMap::new();
        for &(net, _) in tested {
            let class = board.nets[net as usize].class;
            for class in std::iter::once(class).chain(self.neck_of[class]) {
                statics.entry(class).or_insert_with(|| {
                    let maps = StaticMaps::build_with_rules(
                        board,
                        &grid,
                        class,
                        true,
                        self.class_rules.as_deref(),
                    );
                    counts.builds += 1;
                    counts.build_obstacles += board.obstacles.len() as u64;
                    counts.map_nodes += (grid.cells() * board.layer_count) as u64;
                    counts.tests += maps.tests;
                    counts.edges += maps.vertex_tests;
                    counts.steps += maps.tight_tests;
                    maps
                });
            }
        }
        // The obstacles near the window, for naming what closes a pocket.
        let near: Vec<usize> = board
            .obstacles
            .iter()
            .enumerate()
            .filter(|(_, obstacle)| {
                let other = obstacle.shape.aabb();
                other.minimum[0] <= window.maximum[0]
                    && other.maximum[0] >= window.minimum[0]
                    && other.minimum[1] <= window.maximum[1]
                    && other.maximum[1] >= window.minimum[1]
            })
            .map(|(index, _)| index)
            .collect();
        let mut closed = Vec::new();
        let mut reached = vec![false; grid.cells()];
        for &(net, index) in tested {
            let description = &board.nets[net as usize];
            let terminal = &description.terminals[index];
            let layer = terminal.layers.trailing_zeros() as usize;
            let pad = &board.obstacles[terminal.pad];
            let contact = terminal.contact.as_ref().unwrap_or(&pad.shape);
            let class = description.class;
            // The neck zones (`Router::prepare_net`): within `neck_reach`
            // of the net's pads narrower than its track and clearance.
            let rules = board.classes[class];
            let zones: Vec<crate::geometry::Point> = match self.neck_of[class] {
                Some(_) if self.neck_reach > 0.0 => description
                    .terminals
                    .iter()
                    .filter(|other| {
                        let pad = &board.obstacles[other.pad];
                        let bounds = pad.shape.aabb();
                        let narrow = (bounds.maximum[0] - bounds.minimum[0])
                            .min(bounds.maximum[1] - bounds.minimum[1]);
                        narrow < rules.trace_width + rules.clearance
                            && !pad.label.starts_with("pair launch")
                    })
                    .map(|other| other.anchor)
                    .collect(),
                _ => Vec::new(),
            };
            let reach = self.neck_reach;
            let class_of = |cell: usize| -> usize {
                let point = grid.center_of(cell);
                match self.neck_of[class] {
                    Some(neck)
                        if zones.iter().any(|anchor| {
                            (point[0] - anchor[0]).abs() <= reach + 1.0e-9
                                && (point[1] - anchor[1]).abs() <= reach + 1.0e-9
                        }) =>
                    {
                        neck
                    }
                    _ => class,
                }
            };
            // The net's other pads on the layer in the window: reaching
            // one joins them.
            let others: Vec<&crate::geometry::Shape> = description
                .terminals
                .iter()
                .enumerate()
                .filter(|(other, record)| *other != index && record.layers & (1 << layer) != 0)
                .map(|(_, record)| &board.obstacles[record.pad].shape)
                .collect();
            // The router's terminal nodes: legal nodes well inside the pad.
            let mut queue: Vec<usize> = Vec::new();
            if let Some((px0, py0, px1, py1)) = grid.node_range(contact.aabb()) {
                for y in py0..=py1 {
                    for x in px0..=px1 {
                        let cell = grid.index(x, y);
                        if statics[&class].trace_allowed(layer, cell, net)
                            && well_inside(contact, grid.center(x, y))
                        {
                            queue.push(cell);
                        }
                    }
                }
            }
            counts.pads += 1;
            if queue.is_empty() {
                counts.escape_pads += 1;
                if std::env::var_os("PCB_PAD_EXITS_WHY").is_some() {
                    eprintln!(
                        "pad exits: {} has no legal node of its own (the router's escape stubs, not judged)",
                        terminal.label
                    );
                }
                continue;
            }
            let pad_box = pad.shape.aabb();
            let out_box = pad_box.inflated(ESCAPE);
            reached.fill(false);
            for &cell in &queue {
                reached[cell] = true;
            }
            let mut pocket: Vec<usize> = Vec::new();
            // Rim: a closed neighbour node (or the step into it) and the
            // point between.
            let mut rim: Vec<crate::geometry::Point> = Vec::new();
            let mut open = false;
            let mut opened_by = "";
            while let Some(cell) = queue.pop() {
                pocket.push(cell);
                let point = grid.center_of(cell);
                let node_class = class_of(cell);
                if !pad.shape.contains(point) {
                    let outside = point[0] < out_box.minimum[0]
                        || point[0] > out_box.maximum[0]
                        || point[1] < out_box.minimum[1]
                        || point[1] > out_box.maximum[1];
                    if outside
                        || (board.layer_count > 1 && statics[&node_class].via_allowed(cell, net))
                        || others.iter().any(|shape| shape.contains(point))
                    {
                        open = true;
                        opened_by = if outside {
                            "beyond the pad"
                        } else if others.iter().any(|shape| shape.contains(point)) {
                            "another pad of its net"
                        } else {
                            "a via site"
                        };
                        break;
                    }
                } else if board.layer_count > 1 && statics[&node_class].via_allowed(cell, net) {
                    // A via in the pad itself (where its net's via may
                    // stand there).
                    open = true;
                    opened_by = "a via in the pad";
                    break;
                }
                let (blocked, owner) = statics[&node_class].edges_at(layer, cell);
                for direction in 0..8 {
                    let Some(next) = grid.neighbor(cell, direction) else {
                        // The window's edge lies beyond `ESCAPE`.
                        continue;
                    };
                    if reached[next] {
                        continue;
                    }
                    let step_closed =
                        blocked & (1 << direction) != 0 && owner != crate::grid::owner16(net);
                    if step_closed || !statics[&class_of(next)].trace_allowed(layer, next, net) {
                        let at = grid.center_of(next);
                        rim.push(if step_closed {
                            [(at[0] + point[0]) / 2.0, (at[1] + point[1]) / 2.0]
                        } else {
                            at
                        });
                        continue;
                    }
                    reached[next] = true;
                    queue.push(next);
                }
            }
            counts.pocket_nodes += pocket.len() as u64;
            if open {
                if std::env::var_os("PCB_PAD_EXITS_WHY").is_some() {
                    eprintln!(
                        "pad exits: {} open after {} nodes ({opened_by}), at ({:.3}, {:.3})",
                        terminal.label,
                        pocket.len(),
                        grid.center_of(*pocket.last().unwrap())[0],
                        grid.center_of(*pocket.last().unwrap())[1]
                    );
                }
                continue;
            }
            // What closes the pocket: per rim point the item with the least
            // slack (an obstacle of another net on the layer, or the edge).
            let own_part = label_part(&terminal.label);
            let half = self.width_on(class, layer) / 2.0;
            let class_rules = board.classes[class];
            rim.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
            rim.dedup();
            let mut closing: std::collections::BTreeMap<Option<usize>, usize> = Default::default();
            let why = std::env::var_os("PCB_PAD_EXITS_WHY").is_some();
            for point in &rim {
                let mut best: Option<(f64, Option<usize>)> = None;
                for &obstacle_index in &near {
                    let obstacle = &board.obstacles[obstacle_index];
                    if obstacle.net == Some(net)
                        || obstacle.layers & (1 << layer) == 0
                        || !obstacle.blocks_tracks
                    {
                        continue;
                    }
                    counts.measures += 1;
                    let clearance = match obstacle.kind {
                        crate::board::ObstacleKind::Copper => {
                            board.copper_clearance(&class_rules, obstacle)
                        }
                        crate::board::ObstacleKind::Keepout => obstacle.clearance,
                        crate::board::ObstacleKind::Hole => {
                            board.hole_clearance.max(obstacle.clearance)
                        }
                    };
                    let slack = obstacle.shape.distance_to_point(*point) - half - clearance;
                    if best.is_none_or(|(least, _)| slack < least) {
                        best = Some((slack, Some(obstacle_index)));
                    }
                }
                let edge = polygon_edges_distance(&board.outline, *point);
                let edge_slack = if crate::geometry::point_in_polygon(*point, &board.outline) {
                    edge - half - board.edge_clearance
                } else {
                    -edge
                };
                if best.is_none_or(|(least, _)| edge_slack < least) {
                    best = Some((edge_slack, None));
                }
                let item = best.and_then(|(_, item)| item);
                if why {
                    eprintln!(
                        "pad exits: {} rim ({:.3}, {:.3}) closed by {} (slack {:.3})",
                        terminal.label,
                        point[0],
                        point[1],
                        item.map_or("the board edge", |index| board.obstacles[index]
                            .label
                            .as_str()),
                        best.map_or(0.0, |(slack, _)| slack)
                    );
                }
                *closing.entry(item).or_default() += 1;
            }
            let blockers = closing
                .into_iter()
                .map(|(obstacle, nodes)| {
                    let label = obstacle.map_or_else(
                        || "the board edge".to_string(),
                        |index| board.obstacles[index].label.clone(),
                    );
                    let own = obstacle.is_some() && label_part(&label) == own_part;
                    ExitBlocker {
                        label,
                        obstacle,
                        own,
                        nodes,
                    }
                })
                .collect();
            closed.push(ClosedPad {
                label: terminal.label.clone(),
                net: description.name.clone(),
                layer,
                pitch: description_pitch(board, net, terminal, layer),
                pocket: pocket.len(),
                blockers,
            });
        }
        let total = &mut self.counts;
        total.windows += counts.windows;
        total.builds += counts.builds;
        total.build_obstacles += counts.build_obstacles;
        total.map_nodes += counts.map_nodes;
        total.tests += counts.tests;
        total.edges += counts.edges;
        total.steps += counts.steps;
        total.pocket_nodes += counts.pocket_nodes;
        total.measures += counts.measures;
        total.pads += counts.pads;
        total.escape_pads += counts.escape_pads;
        closed
    }
}

/// The nearest pad of another net on `layer`, centre to centre.
fn description_pitch(board: &Board, net: NetId, terminal: &crate::Terminal, layer: usize) -> f64 {
    board
        .nets
        .iter()
        .enumerate()
        .filter(|(other, _)| *other as NetId != net)
        .flat_map(|(_, other)| other.terminals.iter())
        .filter(|other| other.layers & (1 << layer) != 0)
        .map(|other| crate::geometry::distance(other.anchor, terminal.anchor))
        .fold(f64::INFINITY, f64::min)
}

/// The distance from `point` to the nearest edge of `polygon`.
fn polygon_edges_distance(
    polygon: &[crate::geometry::Point],
    point: crate::geometry::Point,
) -> f64 {
    (0..polygon.len())
        .map(|index| {
            crate::geometry::point_segment_distance(
                point,
                polygon[index],
                polygon[(index + 1) % polygon.len()],
            )
        })
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Obstacle, ObstacleKind, RuleClass};
    use crate::geometry::Shape;
    use crate::{Net, Terminal};

    fn obstacle(shape: Shape, net: Option<NetId>, label: &str) -> Obstacle {
        Obstacle {
            shape,
            layers: 1,
            kind: ObstacleKind::Copper,
            net,
            clearance: 0.0,
            clearance_override: None,
            plated: false,
            blocks_tracks: true,
            blocks_vias: true,
            label: label.into(),
        }
    }

    fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
        Shape::Polygon {
            points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        }
    }

    /// U1: three 0.3 x 1.0 mm pads at 0.5 mm pitch (nets A, B, C) along x
    /// around (5, 5), its exposed pad (net G) below the row; each net's
    /// other pad far off. The middle pad's ways: along the row its
    /// neighbours, down the exposed pad, the diagonals the neighbours'
    /// corners: up alone is free. With `wall`, J9 (a fixed pad of net X)
    /// sits 0.3 mm above its tip.
    fn board(wall: bool) -> Board {
        let class = RuleClass {
            trace_width: 0.2,
            clearance: 0.2,
            via_diameter: 0.6,
            via_drill: 0.3,
            layers: 0,
        };
        let mut obstacles = Vec::new();
        let mut nets = Vec::new();
        let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal {
            anchor: at,
            layers: 1,
            pad,
            contact: None,
            label: label.into(),
        };
        for (index, name) in ["A", "B", "C"].iter().enumerate() {
            let x = 4.5 + 0.5 * index as f64;
            let net = nets.len() as NetId;
            obstacles.push(obstacle(
                rectangle(x - 0.15, 4.5, x + 0.15, 5.5),
                Some(net),
                &format!("U1.{}", index + 1),
            ));
            obstacles.push(obstacle(
                rectangle(1.0 + 3.0 * index as f64, 0.5, 1.4 + 3.0 * index as f64, 0.9),
                Some(net),
                &format!("R{index}.1"),
            ));
            nets.push(Net {
                name: name.to_string(),
                class: 0,
                terminals: vec![
                    terminal(obstacles.len() - 2, [x, 5.0], &format!("U1.{}", index + 1)),
                    terminal(
                        obstacles.len() - 1,
                        [1.2 + 3.0 * index as f64, 0.7],
                        &format!("R{index}.1"),
                    ),
                ],
            });
        }
        let ground = nets.len() as NetId;
        obstacles.push(obstacle(
            rectangle(4.2, 3.6, 5.8, 4.3),
            Some(ground),
            "U1.4",
        ));
        obstacles.push(obstacle(
            rectangle(8.5, 8.5, 9.0, 9.0),
            Some(ground),
            "R9.1",
        ));
        nets.push(Net {
            name: "G".into(),
            class: 0,
            terminals: vec![
                terminal(obstacles.len() - 2, [5.0, 3.95], "U1.4"),
                terminal(obstacles.len() - 1, [8.75, 8.75], "R9.1"),
            ],
        });
        if wall {
            let x = nets.len() as NetId;
            obstacles.push(obstacle(rectangle(4.6, 5.8, 5.4, 6.3), Some(x), "J9.1"));
            obstacles.push(obstacle(rectangle(1.0, 8.5, 1.5, 9.0), Some(x), "J9.2"));
            nets.push(Net {
                name: "X".into(),
                class: 0,
                terminals: vec![
                    terminal(obstacles.len() - 2, [5.0, 6.05], "J9.1"),
                    terminal(obstacles.len() - 1, [1.25, 8.75], "J9.2"),
                ],
            });
        }
        Board {
            layer_count: 1,
            outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            edge_clearance: 0.1,
            hole_clearance: 0.25,
            hole_to_hole: 0.25,
            classes: vec![class],
            neck_width: 0.2,
            obstacles,
            nets,
            planes: Vec::new(),
            solid_pads: Vec::new(),
            isolated_pads: Vec::new(),
            solid_pad_obstacles: Vec::new(),
            isolated_pad_obstacles: Vec::new(),
            pad_spokes: Default::default(),
            merged_pads: Vec::new(),
            no_via_pads: Vec::new(),
            fill_passes: Vec::new(),
            pin_row_gaps: Vec::new(),
            terminal_copper: Vec::new(),
            attachment: Default::default(),
            via_spans: Vec::new(),
        }
    }

    fn config() -> Config {
        Config {
            pitches: vec![0.1],
            ..Config::default()
        }
    }

    /// Turns U1's copper a quarter about the middle pad's centre (5, 5).
    fn turn_part(board: &mut Board, part: &str) {
        let turn = |point: crate::geometry::Point| [5.0 - (point[1] - 5.0), 5.0 + (point[0] - 5.0)];
        for obstacle in &mut board.obstacles {
            if label_part(&obstacle.label) != part {
                continue;
            }
            if let Shape::Polygon { points } = &mut obstacle.shape {
                for point in points.iter_mut() {
                    *point = turn(*point);
                }
            }
        }
        for net in &mut board.nets {
            for terminal in &mut net.terminals {
                if label_part(&terminal.label) == part {
                    terminal.anchor = turn(terminal.anchor);
                }
            }
        }
    }

    #[test]
    fn a_pad_walled_by_another_parts_pad_has_no_way_out_until_its_part_turns() {
        // Without J9 the middle pad leaves upwards: nothing is closed.
        let mut free = ExitProbe::new(&board(false), &config());
        assert!(free.closed_pads("U1").is_empty());
        assert!(free.take_work_seconds() > 0.0);

        let mut probe = ExitProbe::new(&board(true), &config());
        let closed = probe.closed_pads("U1");
        assert_eq!(closed.len(), 1, "{closed:?}");
        let pad = &closed[0];
        assert_eq!((pad.label.as_str(), pad.net.as_str()), ("U1.2", "B"));
        // Its pocket: up J9's pad; along the row its own neighbours, down
        // its exposed pad.
        let labels: Vec<(&str, bool)> = pad
            .blockers
            .iter()
            .map(|blocker| (blocker.label.as_str(), blocker.own))
            .collect();
        assert!(labels.contains(&("J9.1", false)), "{labels:?}");
        assert!(labels.contains(&("U1.4", true)), "{labels:?}");
        assert!(labels.contains(&("U1.1", true)) && labels.contains(&("U1.3", true)));
        assert!(
            !pad.own_only(),
            "another part closes it: the placement's doing"
        );
        assert_eq!(pad.blocked_by()[0], "J9.1", "other parts first");
        // J9 alone has ways out (nothing of U1 closes them).
        assert!(probe.closed_pads("J9").is_empty());

        // A quarter turn of U1: the row runs along y, the exposed pad lies
        // to the right, and the middle pad leaves to the left; J9 above
        // closes nothing.
        turn_part(probe.board_mut(), "U1");
        assert!(probe.closed_pads("U1").is_empty());
    }

    #[test]
    fn a_pad_its_own_part_walls_in_is_the_footprints() {
        // J9 is U1's own (a part of the footprint): only U1 closes its
        // middle pad.
        let mut walled = board(true);
        for obstacle in &mut walled.obstacles {
            if obstacle.label == "J9.1" {
                obstacle.label = "U1.5".into();
            }
        }
        for net in &mut walled.nets {
            for terminal in &mut net.terminals {
                if terminal.label == "J9.1" {
                    terminal.label = "U1.5".into();
                }
            }
        }
        let mut probe = ExitProbe::new(&walled, &config());
        let closed = probe.closed_pads("U1");
        assert_eq!(closed.len(), 1);
        assert!(closed[0].own_only());
    }

    #[test]
    fn a_pad_that_may_take_a_via_has_a_way_out() {
        // A via small enough to sit in the 0.3 mm pad clear of its
        // neighbours.
        let mut two_layers = board(true);
        two_layers.layer_count = 2;
        two_layers.classes[0].via_diameter = 0.25;
        two_layers.classes[0].via_drill = 0.1;
        let mut probe = ExitProbe::new(&two_layers, &config());
        // No pad is listed as taking no via: a via in the pad leaves it.
        assert!(probe.closed_pads("U1").is_empty());
        two_layers.no_via_pads = (0..two_layers.obstacles.len()).collect();
        let mut probe = ExitProbe::new(&two_layers, &config());
        assert_eq!(probe.closed_pads("U1").len(), 1);
    }
}

// Copyright (C) 2026 Toit contributors.

//! The placement race's pre-ranking: each placement of one board is
//! lowered as the router would see it and scored by its RUDY routing
//! demand above the tiles' free capacity (`rudy_overflow`), or by the
//! coarse global route (an experiment). A score takes milliseconds; the
//! race then probes only the best-ranked placements.
//!
//! RUDY is computed on the router's tiles (`TILE` x `TILE` lattice nodes)
//! from the board alone: no router is built. Copper layers go to four
//! slots, the same for every board: front, back, the first inner layer,
//! the other inner layers (averaged). Until 2026-11-10 these maps were
//! the input of a learned congestion predictor (the `pcb-congestion`
//! crate), which ranked placements no better than RUDY and was removed
//! (docs/handover.md, "Tried and not taken").

use super::*;
use crate::board_router::lower;
use crate::canonical_order::footprint_items;
use crate::footprint_bodies::write_footprint_pose;
use pcb_placer as placer;
use pcb_router as core;
use pcb_router::geometry::{Point, Shape};
use pcb_router::router::congestion::TileFrame;
use pcb_router::{Board, ObstacleKind};

pub(crate) struct Ranking {
    /// Per placement, lower is better.
    pub scores: Vec<f64>,
    /// Placement indices, best first (ties to the earlier one).
    pub order: Vec<usize>,
    pub seconds: f64,
    pub lower_seconds: f64,
    pub score_seconds: f64,
}

/// The pre-ranking scores the layout knows (`congestion_score`).
pub(crate) const SCORES: [&str; 3] = ["rudy", "global", "none"];

/// One placement's score: `rudy` (RUDY demand above capacity) or
/// `global` (the global route's demand above half the boundaries'
/// capacity, docs/global-route.md; an experiment).
pub(crate) fn score_board(
    board: &core::Board,
    core_config: &core::Config,
    score: &str,
) -> Result<f64, String> {
    match score {
        "rudy" => {
            let frame = core::router::congestion::tile_frame(board, core_config);
            Ok(rudy_overflow(board, &frame, 0.25) as f64)
        }
        "global" => Ok(
            pcb_global::route(board, &pcb_global::GlobalConfig::default())
                .score
                .soft_overflow[0],
        ),
        _ => Err(format!(
            "congestion_score {score:?} is not one of {}",
            SCORES.join(", ")
        )),
    }
}

/// `pcb` is the board as the layout routes it (`without_copper_texts`).
pub(crate) fn rank_placements(
    pcb: &Expr,
    placements: &[(Vec<placer::Pose>, placer::Relaxation)],
    router_config: &KiCadBoardRouterConfig,
    core_config: &core::Config,
    connect: bool,
    score: &str,
) -> Result<Ranking, String> {
    let started = std::time::Instant::now();
    let (mut lower_seconds, mut score_seconds) = (0.0, 0.0);
    let mut scores = Vec::with_capacity(placements.len());
    for (poses, _) in placements {
        let lowered_at = std::time::Instant::now();
        let mut candidate = pcb.clone();
        {
            let mut footprints = footprint_items(&mut candidate)?;
            for (footprint, pose) in footprints.iter_mut().zip(poses) {
                write_footprint_pose(footprint, *pose)?;
            }
        }
        let board = lower(&candidate, router_config, connect)?.board;
        lower_seconds += lowered_at.elapsed().as_secs_f64();
        let scored_at = std::time::Instant::now();
        scores.push(score_board(&board, core_config, score)?);
        score_seconds += scored_at.elapsed().as_secs_f64();
    }
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|a, b| scores[*a].total_cmp(&scores[*b]).then(a.cmp(b)));
    Ok(Ranking {
        scores,
        order,
        seconds: started.elapsed().as_secs_f64(),
        lower_seconds,
        score_seconds,
    })
}

/// Layer slots: front, back, first inner, other inner layers.
const SLOTS: usize = 4;

/// The slot of copper layer `layer` of `layers` (0 front, last back).
fn slot_of(layer: usize, layers: usize) -> usize {
    if layer == 0 {
        0
    } else if layer + 1 == layers {
        1
    } else if layer == 1 {
        2
    } else {
        3
    }
}

/// How many copper layers share each slot.
fn slot_sizes(layers: usize) -> [usize; SLOTS] {
    let mut sizes = [0; SLOTS];
    for layer in 0..layers {
        sizes[slot_of(layer, layers)] += 1;
    }
    sizes
}

/// The maps RUDY's overflow reads, per tile (`[tile_y][tile_x]`).
struct RudyMaps {
    tiles: usize,
    /// The share of the tile's nodes inside the outline.
    inside: Vec<f32>,
    /// Per slot, the share of the tile's nodes under an obstacle (outside
    /// the outline included), averaged over the slot's layers.
    obstacle: [Vec<f32>; SLOTS],
    /// Per slot, the share under pad copper (copper with a net).
    pad_copper: [Vec<f32>; SLOTS],
    /// Per slot, 1 where the board has a layer in it.
    present: [f32; SLOTS],
    /// Routing demand: each net's spanning tree edges' wire area (length
    /// times its track pitch) spread over the edges' boxes; pour nets left
    /// out.
    demand: Vec<f32>,
}

/// A bit per lattice node.
struct NodeMask {
    nx: usize,
    ny: usize,
    bits: Vec<u64>,
}

impl NodeMask {
    fn new(nx: usize, ny: usize) -> Self {
        Self {
            nx,
            ny,
            bits: vec![0; (nx * ny).div_ceil(64)],
        }
    }

    fn set_span(&mut self, y: usize, x0: usize, x1: usize) {
        for x in x0..=x1 {
            let index = y * self.nx + x;
            self.bits[index / 64] |= 1 << (index % 64);
        }
    }

    /// Set nodes per tile over the tile's node count.
    fn tile_shares(&self, frame: &TileFrame, out: &mut [f32]) {
        let tile = frame.tile;
        let mut counts = vec![0u32; frame.tiles_x * frame.tiles_y];
        for (word_index, word) in self.bits.iter().enumerate() {
            if *word == 0 {
                continue;
            }
            let mut bits = *word;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let index = word_index * 64 + bit;
                if index >= self.nx * self.ny {
                    break;
                }
                let (x, y) = (index % self.nx, index / self.nx);
                counts[(y / tile) * frame.tiles_x + x / tile] += 1;
            }
        }
        for (index, count) in counts.iter().enumerate() {
            let (tx, ty) = (index % frame.tiles_x, index / frame.tiles_x);
            let width = (self.nx - tx * tile).min(tile);
            let height = (self.ny - ty * tile).min(tile);
            out[index] = *count as f32 / (width * height) as f32;
        }
    }
}

/// Node rows and spans whose centres lie inside `shape`.
fn fill_shape(shape: &Shape, frame: &TileFrame, mask: &mut NodeMask) {
    let pitch = frame.pitch;
    let row_range = |low: f64, high: f64| -> Option<(usize, usize)> {
        let y0 = ((low - frame.origin[1]) / pitch).ceil().max(0.0);
        let y1 = ((high - frame.origin[1]) / pitch)
            .floor()
            .min(frame.ny as f64 - 1.0);
        (y0 <= y1).then_some((y0 as usize, y1 as usize))
    };
    let span = |low: f64, high: f64| -> Option<(usize, usize)> {
        let x0 = ((low - frame.origin[0]) / pitch).ceil().max(0.0);
        let x1 = ((high - frame.origin[0]) / pitch)
            .floor()
            .min(frame.nx as f64 - 1.0);
        (x0 <= x1).then_some((x0 as usize, x1 as usize))
    };
    match shape {
        Shape::Circle { center, radius } => {
            let Some((y0, y1)) = row_range(center[1] - radius, center[1] + radius) else {
                return;
            };
            for y in y0..=y1 {
                let dy = frame.origin[1] + y as f64 * pitch - center[1];
                let half = (radius * radius - dy * dy).max(0.0).sqrt();
                if let Some((x0, x1)) = span(center[0] - half, center[0] + half) {
                    mask.set_span(y, x0, x1);
                }
            }
        }
        Shape::Capsule { .. } => {
            let bounds = shape.aabb();
            let Some((y0, y1)) = row_range(bounds.minimum[1], bounds.maximum[1]) else {
                return;
            };
            let Some((x0, x1)) = span(bounds.minimum[0], bounds.maximum[0]) else {
                return;
            };
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let point = [
                        frame.origin[0] + x as f64 * pitch,
                        frame.origin[1] + y as f64 * pitch,
                    ];
                    if shape.contains(point) {
                        mask.set_span(y, x, x);
                    }
                }
            }
        }
        Shape::Polygon { points } => fill_polygon(points, frame, mask),
        Shape::Union { parts } => {
            for part in parts {
                fill_shape(part, frame, mask);
            }
        }
    }
}

/// Even-odd scanline fill at node centres.
fn fill_polygon(points: &[Point], frame: &TileFrame, mask: &mut NodeMask) {
    if points.len() < 3 {
        return;
    }
    let pitch = frame.pitch;
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for point in points {
        low = low.min(point[1]);
        high = high.max(point[1]);
    }
    let y0 = ((low - frame.origin[1]) / pitch).ceil().max(0.0);
    let y1 = ((high - frame.origin[1]) / pitch)
        .floor()
        .min(frame.ny as f64 - 1.0);
    if y0 > y1 {
        return;
    }
    let mut crossings = Vec::new();
    for y in y0 as usize..=y1 as usize {
        let yc = frame.origin[1] + y as f64 * pitch;
        crossings.clear();
        for index in 0..points.len() {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            if (a[1] <= yc) != (b[1] <= yc) {
                crossings.push(a[0] + (yc - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
            }
        }
        crossings.sort_by(f64::total_cmp);
        for pair in crossings.as_chunks::<2>().0 {
            let x0 = ((pair[0] - frame.origin[0]) / pitch).ceil().max(0.0);
            let x1 = ((pair[1] - frame.origin[0]) / pitch)
                .floor()
                .min(frame.nx as f64 - 1.0);
            if x0 <= x1 {
                mask.set_span(y, x0 as usize, x1 as usize);
            }
        }
    }
}

/// Adds `density` (a share of the area) uniformly over the rectangle
/// `[low, high]` (millimetres), weighted by each tile's overlap.
fn add_box(frame: &TileFrame, out: &mut [f32], low: Point, high: Point, density: f64) {
    let tile_mm = frame.tile_mm();
    // Tile t spans nodes [t * tile, (t + 1) * tile): from half a pitch
    // before its first node centre.
    let start = [
        frame.origin[0] - frame.pitch / 2.0,
        frame.origin[1] - frame.pitch / 2.0,
    ];
    let overlaps = |axis: usize, count: usize| -> Vec<(usize, f64)> {
        let a = (low[axis] - start[axis]) / tile_mm;
        let b = (high[axis] - start[axis]) / tile_mm;
        let first = a.floor().max(0.0) as usize;
        let last = (b.ceil() as isize - 1).clamp(0, count as isize - 1) as usize;
        (first..=last.max(first))
            .filter(|t| *t < count)
            .map(|t| (t, ((b.min(t as f64 + 1.0) - a.max(t as f64)).max(0.0))))
            .filter(|(_, share)| *share > 0.0)
            .collect()
    };
    let xs = overlaps(0, frame.tiles_x);
    let ys = overlaps(1, frame.tiles_y);
    for (ty, share_y) in &ys {
        for (tx, share_x) in &xs {
            out[ty * frame.tiles_x + tx] += (density * share_x * share_y) as f32;
        }
    }
}

/// Rectilinear minimum spanning tree edges (Prim, O(n^2)).
fn mst_edges(points: &[Point]) -> Vec<(usize, usize)> {
    let n = points.len();
    if n < 2 {
        return Vec::new();
    }
    let mut in_tree = vec![false; n];
    let mut best = vec![(f64::INFINITY, 0usize); n];
    in_tree[0] = true;
    for index in 1..n {
        best[index] = (
            (points[index][0] - points[0][0]).abs() + (points[index][1] - points[0][1]).abs(),
            0,
        );
    }
    let mut edges = Vec::with_capacity(n - 1);
    for _ in 1..n {
        let next = (0..n)
            .filter(|index| !in_tree[*index])
            .min_by(|a, b| best[*a].0.total_cmp(&best[*b].0))
            .unwrap();
        in_tree[next] = true;
        edges.push((best[next].1, next));
        for index in 0..n {
            if !in_tree[index] {
                let distance = (points[index][0] - points[next][0]).abs()
                    + (points[index][1] - points[next][1]).abs();
                if distance < best[index].0 {
                    best[index] = (distance, next);
                }
            }
        }
    }
    edges
}

/// The board's RUDY maps on `frame`.
fn rasterize(board: &Board, frame: &TileFrame) -> RudyMaps {
    let layers = board.layer_count;
    let tiles = frame.tiles_x * frame.tiles_y;
    let sizes = slot_sizes(layers);
    // Per layer: pad copper (copper with a net) and other obstacles.
    let mut pad_masks: Vec<NodeMask> = (0..layers)
        .map(|_| NodeMask::new(frame.nx, frame.ny))
        .collect();
    let mut obstacle_masks: Vec<NodeMask> = (0..layers)
        .map(|_| NodeMask::new(frame.nx, frame.ny))
        .collect();
    for obstacle in &board.obstacles {
        let is_pad = obstacle.kind == ObstacleKind::Copper && obstacle.net.is_some();
        if !is_pad && !obstacle.blocks_tracks {
            continue;
        }
        for layer in 0..layers {
            if obstacle.layers & (1 << layer) == 0 {
                continue;
            }
            let mask = if is_pad {
                &mut pad_masks[layer]
            } else {
                &mut obstacle_masks[layer]
            };
            fill_shape(&obstacle.shape, frame, mask);
        }
    }
    let mut inside = NodeMask::new(frame.nx, frame.ny);
    fill_polygon(&board.outline, frame, &mut inside);
    let mut share = vec![0.0f32; tiles];
    inside.tile_shares(frame, &mut share);
    let inside_share = share.clone();
    // Outside the outline is an obstacle on every layer.
    for mask in &mut obstacle_masks {
        for (word, inside) in mask.bits.iter_mut().zip(&inside.bits) {
            *word |= !inside;
        }
    }
    let mut obstacle: [Vec<f32>; SLOTS] = std::array::from_fn(|_| vec![0.0f32; tiles]);
    let mut pad_copper: [Vec<f32>; SLOTS] = std::array::from_fn(|_| vec![0.0f32; tiles]);
    for layer in 0..layers {
        let slot = slot_of(layer, layers);
        let weight = 1.0 / sizes[slot] as f32;
        for (out, masks) in [
            (&mut pad_copper[slot], &pad_masks),
            (&mut obstacle[slot], &obstacle_masks),
        ] {
            masks[layer].tile_shares(frame, &mut share);
            for tile in 0..tiles {
                out[tile] += weight * share[tile];
            }
        }
    }
    let present = sizes.map(|size| if size > 0 { 1.0 } else { 0.0 });
    let poured: Vec<bool> = (0..board.nets.len())
        .map(|net| {
            board
                .planes
                .iter()
                .any(|plane| plane.net as usize == net && plane.connect)
        })
        .collect();
    let tile_mm = frame.tile_mm();
    let mut demand = vec![0.0f32; tiles];
    for (net, record) in board.nets.iter().enumerate() {
        if record.terminals.len() < 2 || poured[net] {
            continue;
        }
        let rules = board.classes[record.class];
        let track = rules.trace_width + rules.clearance;
        let points: Vec<Point> = record
            .terminals
            .iter()
            .map(|terminal| terminal.anchor)
            .collect();
        // Boxes at least a tile wide: a straight run's demand lies in the
        // tile row it runs along.
        let grow = |low: Point, high: Point| -> (Point, Point) {
            let mut low = low;
            let mut high = high;
            for axis in 0..2 {
                let missing = tile_mm - (high[axis] - low[axis]);
                if missing > 0.0 {
                    low[axis] -= missing / 2.0;
                    high[axis] += missing / 2.0;
                }
            }
            (low, high)
        };
        for (a, b) in mst_edges(&points) {
            let edge_low = [
                points[a][0].min(points[b][0]),
                points[a][1].min(points[b][1]),
            ];
            let edge_high = [
                points[a][0].max(points[b][0]),
                points[a][1].max(points[b][1]),
            ];
            let (edge_low, edge_high) = grow(edge_low, edge_high);
            let edge_length =
                (points[a][0] - points[b][0]).abs() + (points[a][1] - points[b][1]).abs();
            let edge_area = (edge_high[0] - edge_low[0]) * (edge_high[1] - edge_low[1]);
            add_box(
                frame,
                &mut demand,
                edge_low,
                edge_high,
                edge_length * track / edge_area,
            );
        }
    }
    RudyMaps {
        tiles,
        inside: inside_share,
        obstacle,
        pad_copper,
        present,
        demand,
    }
}

impl RudyMaps {
    /// Routing demand above `alpha` times the tile's free capacity (the
    /// free share of every layer present: not obstacle, not pad copper),
    /// summed over the board's tiles.
    fn overflow(&self, alpha: f32) -> f32 {
        let mut total = 0.0;
        for tile in 0..self.tiles {
            if self.inside[tile] <= 0.0 {
                continue;
            }
            let mut capacity = 0.0;
            for slot in 0..SLOTS {
                let free = 1.0 - self.obstacle[slot][tile] - self.pad_copper[slot][tile];
                capacity += self.present[slot] * free.clamp(0.0, 1.0);
            }
            total += (self.demand[tile] - alpha * capacity).max(0.0);
        }
        total
    }
}

/// The race's ranker: RUDY routing demand (over the nets' spanning tree
/// edges) above `alpha` times the tiles' free capacity, summed over the
/// board's tiles on `frame`. Lower is better. `alpha` 0.25 ranked
/// held-out placements best (fitted 0.2-0.3).
pub fn rudy_overflow(board: &Board, frame: &TileFrame, alpha: f32) -> f32 {
    rasterize(board, frame).overflow(alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_router::{Net, Obstacle, RuleClass, Terminal};

    fn frame(nx: usize, ny: usize) -> TileFrame {
        TileFrame {
            origin: [0.0, 0.0],
            pitch: 0.1,
            nx,
            ny,
            tile: 16,
            tiles_x: nx.div_ceil(16),
            tiles_y: ny.div_ceil(16),
        }
    }

    fn pad(center: Point, half: f64, layers: u32, net: u32) -> Obstacle {
        Obstacle {
            shape: Shape::rectangle(center, [half, half], 0.0),
            layers,
            kind: ObstacleKind::Copper,
            net: Some(net),
            clearance: 0.0,
            clearance_override: None,
            plated: false,
            blocks_tracks: true,
            blocks_vias: true,
            label: String::new(),
        }
    }

    /// A 3.2 x 3.2 mm two-layer board (2 x 2 tiles of 1.6 mm) with one
    /// two-pin net from the first tile to the last.
    fn small_board() -> Board {
        let outline = vec![[-0.05, -0.05], [3.15, -0.05], [3.15, 3.15], [-0.05, 3.15]];
        Board {
            layer_count: 2,
            outline,
            edge_clearance: 0.0,
            hole_clearance: 0.0,
            hole_to_hole: 0.0,
            classes: vec![RuleClass {
                trace_width: 0.2,
                clearance: 0.2,
                via_diameter: 0.6,
                via_drill: 0.3,
                layers: 0,
            }],
            neck_width: 0.2,
            // A 0.5 mm square SMD pad on the front (5 x 5 nodes at 0.1 mm)
            // and a through pad.
            obstacles: vec![pad([0.4, 0.4], 0.25, 1, 0), pad([2.4, 2.4], 0.25, 3, 0)],
            nets: vec![Net {
                name: "A".into(),
                class: 0,
                terminals: vec![
                    Terminal {
                        anchor: [0.4, 0.4],
                        layers: 1,
                        pad: 0,
                        contact: None,
                        label: "U1.1".into(),
                    },
                    Terminal {
                        anchor: [2.4, 2.4],
                        layers: 3,
                        pad: 1,
                        contact: None,
                        label: "U2.1".into(),
                    },
                ],
            }],
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

    #[test]
    fn small_board_maps() {
        let board = small_board();
        let maps = rasterize(&board, &frame(32, 32));
        // A 0.5 mm pad covers 5 x 5 node centres of the 256 of a tile.
        let pad = &maps.pad_copper[0];
        assert!((pad[0] - 25.0 / 256.0).abs() < 1.0e-6, "{pad:?}");
        assert!((pad[3] - 25.0 / 256.0).abs() < 1.0e-6, "{pad:?}");
        assert_eq!(maps.pad_copper[1][0], 0.0);
        // The outline holds every node.
        assert_eq!(maps.inside, vec![1.0; 4]);
        assert_eq!(maps.obstacle[0], vec![0.0; 4]);
        assert_eq!(maps.present, [1.0, 1.0, 0.0, 0.0]);
        // The spanning tree's one edge: length 4 mm times a 0.4 mm track
        // pitch over the 2 x 2 mm box, spread over the tiles it overlaps
        // (box from 0.4 to 2.4 mm; tiles from -0.05 to 1.55 and 1.55 to
        // 3.15).
        let density = 4.0 * 0.4 / 4.0;
        let near = (1.55 - 0.4) / 1.6;
        let far = (2.4 - 1.55) / 1.6;
        let expected =
            [near * near, far * near, near * far, far * far].map(|share| (density * share) as f32);
        for (value, want) in maps.demand.iter().zip(expected) {
            assert!(
                (value - want).abs() < 1.0e-5,
                "{:?} against {expected:?}",
                maps.demand
            );
        }
    }

    #[test]
    fn rudy_overflow_counts_demand_above_capacity() {
        let board = small_board();
        let frame = frame(32, 32);
        // Two layers of free tiles (less the pads): no overflow at alpha
        // 0.25 for one short net, all of the demand at alpha 0.
        assert_eq!(rudy_overflow(&board, &frame, 0.25), 0.0);
        let demand: f32 = rasterize(&board, &frame).demand.iter().sum();
        assert!((rudy_overflow(&board, &frame, 0.0) - demand).abs() < 1.0e-5);
    }

    /// The race's RUDY ranking on a known 2 x 2-tile case: a two-pin net
    /// with a 2 mm track pitch from (0.4, 0.4) to (2.4, 2.4) demands
    /// 4 mm x 2 mm over its 2 x 2 mm box (density 2) spread over the
    /// four tiles by their overlap; each tile's capacity is its two free
    /// layers less the pad copper. The same net with its pads 0.4 mm
    /// apart in one tile demands less and ranks first.
    #[test]
    fn rudy_ranking_on_a_two_by_two_tile_board() {
        let mut board = small_board();
        board.classes[0] = RuleClass {
            trace_width: 1.0,
            clearance: 1.0,
            via_diameter: 0.6,
            via_drill: 0.3,
            layers: 0,
        };
        let near = (1.55 - 0.4) / 1.6;
        let far = (2.4 - 1.55) / 1.6;
        let pad_share: f64 = 25.0 / 256.0;
        // Tiles 0 and 3 hold a pad on the front (tile 3's through pad on
        // both layers).
        let capacity: [f64; 4] = [2.0 - pad_share, 2.0, 2.0, 2.0 - 2.0 * pad_share];
        let demand = [near * near, far * near, near * far, far * far].map(|share| 2.0 * share);
        let expected: f64 = demand
            .iter()
            .zip(capacity)
            .map(|(d, c)| (d - 0.25 * c).max(0.0))
            .sum();
        let spread = rudy_overflow(&board, &frame(32, 32), 0.25);
        assert!(
            (spread as f64 - expected).abs() < 1.0e-4,
            "{spread} against {expected}"
        );
        // The second pad next to the first: a 0.8 mm net in one tile.
        let mut close = board.clone();
        close.obstacles[1] = pad([0.8, 0.8], 0.25, 3, 0);
        close.nets[0].terminals[1].anchor = [0.8, 0.8];
        let packed = rudy_overflow(&close, &frame(32, 32), 0.25);
        assert!(packed < spread, "{packed} against {spread}");
    }

    #[test]
    fn outside_the_outline_is_an_obstacle() {
        let mut board = small_board();
        // Only the left half of the board.
        board.outline = vec![[-0.05, -0.05], [1.55, -0.05], [1.55, 3.15], [-0.05, 3.15]];
        let maps = rasterize(&board, &frame(32, 32));
        assert_eq!(maps.inside, vec![1.0, 0.0, 1.0, 0.0]);
        assert_eq!(maps.obstacle[1], vec![0.0, 1.0, 0.0, 1.0]);
    }
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The congestion model's input: a lowered board rasterized on the
//! router's tiles (`TILE` x `TILE` lattice nodes). Every channel is
//! computed from the board alone in milliseconds: no router is built.
//!
//! Copper layers go to four slots, the same for every board: front, back,
//! the first inner layer, the other inner layers (averaged). A two-layer
//! board leaves the inner slots empty (`layer_present` tells).

use pcb_router::geometry::{Point, Shape};
use pcb_router::router::congestion::TileFrame;
use pcb_router::{Board, ObstacleKind};

/// Layer slots of the model.
pub const SLOTS: usize = 4;

/// Per-slot channels, in order (each `SLOTS` planes, slot-major within the
/// channel).
pub const LAYER_CHANNELS: [&str; 5] = ["pins", "pad_copper", "obstacle", "pour", "layer_present"];

/// Board-wide channels, in order, after the per-slot ones.
pub const BOARD_CHANNELS: [&str; 12] = [
    "inside",
    "rudy",
    "rudy_mst",
    "pin_rudy",
    "rudy_pour",
    "nets_here",
    "through_pins",
    "layer_count",
    "track_pitch",
    "via_pitch",
    "tile_mm",
    "pins_scaled",
];

pub fn channel_names() -> Vec<String> {
    let mut names = Vec::new();
    for channel in LAYER_CHANNELS {
        for slot in ["F", "B", "In1", "InN"] {
            names.push(format!("{channel}.{slot}"));
        }
    }
    names.extend(BOARD_CHANNELS.iter().map(|name| name.to_string()));
    names
}

pub fn channel_count() -> usize {
    LAYER_CHANNELS.len() * SLOTS + BOARD_CHANNELS.len()
}

/// The slot of copper layer `layer` of `layers` (0 front, last back).
pub fn slot_of(layer: usize, layers: usize) -> usize {
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
pub fn slot_sizes(layers: usize) -> [usize; SLOTS] {
    let mut sizes = [0; SLOTS];
    for layer in 0..layers {
        sizes[slot_of(layer, layers)] += 1;
    }
    sizes
}

/// Channel-major maps `[channel][tile_y][tile_x]`.
#[derive(Clone, Debug)]
pub struct FeatureMaps {
    pub tiles_x: usize,
    pub tiles_y: usize,
    pub data: Vec<f32>,
}

impl FeatureMaps {
    pub fn plane(&self, channel: usize) -> &[f32] {
        let size = self.tiles_x * self.tiles_y;
        &self.data[channel * size..(channel + 1) * size]
    }

    pub fn channel(&self, name: &str) -> Option<&[f32]> {
        channel_names().iter().position(|known| known == name).map(|index| self.plane(index))
    }
}

/// A bit per lattice node.
struct NodeMask {
    nx: usize,
    ny: usize,
    bits: Vec<u64>,
}

impl NodeMask {
    fn new(nx: usize, ny: usize) -> Self {
        Self { nx, ny, bits: vec![0; (nx * ny).div_ceil(64)] }
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
        let y1 = ((high - frame.origin[1]) / pitch).floor().min(frame.ny as f64 - 1.0);
        (y0 <= y1).then_some((y0 as usize, y1 as usize))
    };
    let span = |low: f64, high: f64| -> Option<(usize, usize)> {
        let x0 = ((low - frame.origin[0]) / pitch).ceil().max(0.0);
        let x1 = ((high - frame.origin[0]) / pitch).floor().min(frame.nx as f64 - 1.0);
        (x0 <= x1).then_some((x0 as usize, x1 as usize))
    };
    match shape {
        Shape::Circle { center, radius } => {
            let Some((y0, y1)) = row_range(center[1] - radius, center[1] + radius) else { return };
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
            let Some((y0, y1)) = row_range(bounds.minimum[1], bounds.maximum[1]) else { return };
            let Some((x0, x1)) = span(bounds.minimum[0], bounds.maximum[0]) else { return };
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let point = [frame.origin[0] + x as f64 * pitch, frame.origin[1] + y as f64 * pitch];
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
    let y1 = ((high - frame.origin[1]) / pitch).floor().min(frame.ny as f64 - 1.0);
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
        for pair in crossings.chunks_exact(2) {
            let x0 = ((pair[0] - frame.origin[0]) / pitch).ceil().max(0.0);
            let x1 = ((pair[1] - frame.origin[0]) / pitch).floor().min(frame.nx as f64 - 1.0);
            if x0 <= x1 {
                mask.set_span(y, x0 as usize, x1 as usize);
            }
        }
    }
}

/// Adds `density` (a share of the area) uniformly over the rectangle
/// `[x0, x1] x [y0, y1]` (millimetres), weighted by each tile's overlap.
fn add_box(frame: &TileFrame, out: &mut [f32], low: Point, high: Point, density: f64) {
    let tile_mm = frame.tile_mm();
    // Tile t spans nodes [t * tile, (t + 1) * tile): from half a pitch
    // before its first node centre.
    let start = [frame.origin[0] - frame.pitch / 2.0, frame.origin[1] - frame.pitch / 2.0];
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

fn tile_of(frame: &TileFrame, point: Point) -> Option<usize> {
    let x = ((point[0] - frame.origin[0]) / frame.pitch).round();
    let y = ((point[1] - frame.origin[1]) / frame.pitch).round();
    (x >= 0.0 && y >= 0.0 && (x as usize) < frame.nx && (y as usize) < frame.ny)
        .then(|| (y as usize / frame.tile) * frame.tiles_x + x as usize / frame.tile)
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
        best[index] = ((points[index][0] - points[0][0]).abs() + (points[index][1] - points[0][1]).abs(), 0);
    }
    let mut edges = Vec::with_capacity(n - 1);
    for _ in 1..n {
        let next = (0..n).filter(|index| !in_tree[*index]).min_by(|a, b| best[*a].0.total_cmp(&best[*b].0)).unwrap();
        in_tree[next] = true;
        edges.push((best[next].1, next));
        for index in 0..n {
            if !in_tree[index] {
                let distance = (points[index][0] - points[next][0]).abs() + (points[index][1] - points[next][1]).abs();
                if distance < best[index].0 {
                    best[index] = (distance, next);
                }
            }
        }
    }
    edges
}

/// The board's channels on `frame` (see `channel_names`).
pub fn rasterize(board: &Board, frame: &TileFrame) -> FeatureMaps {
    let layers = board.layer_count;
    let tiles = frame.tiles_x * frame.tiles_y;
    let channels = channel_count();
    let mut data = vec![0.0f32; channels * tiles];
    let sizes = slot_sizes(layers);
    let plane_index = |channel: usize, slot: usize| (channel * SLOTS + slot) * tiles;
    let board_index = |name: &str| {
        (LAYER_CHANNELS.len() * SLOTS + BOARD_CHANNELS.iter().position(|known| *known == name).unwrap()) * tiles
    };
    // Per layer: pad copper (copper with a net), other obstacles, pours.
    let mut pad_masks: Vec<NodeMask> = (0..layers).map(|_| NodeMask::new(frame.nx, frame.ny)).collect();
    let mut obstacle_masks: Vec<NodeMask> = (0..layers).map(|_| NodeMask::new(frame.nx, frame.ny)).collect();
    let mut pour_masks: Vec<NodeMask> = (0..layers).map(|_| NodeMask::new(frame.nx, frame.ny)).collect();
    for obstacle in &board.obstacles {
        let is_pad = obstacle.kind == ObstacleKind::Copper && obstacle.net.is_some();
        if !is_pad && !obstacle.blocks_tracks {
            continue;
        }
        for layer in 0..layers {
            if obstacle.layers & (1 << layer) == 0 {
                continue;
            }
            let mask = if is_pad { &mut pad_masks[layer] } else { &mut obstacle_masks[layer] };
            fill_shape(&obstacle.shape, frame, mask);
        }
    }
    for plane in &board.planes {
        if plane.layer < layers {
            fill_polygon(&plane.polygon, frame, &mut pour_masks[plane.layer]);
        }
    }
    let mut inside = NodeMask::new(frame.nx, frame.ny);
    fill_polygon(&board.outline, frame, &mut inside);
    let mut share = vec![0.0f32; tiles];
    inside.tile_shares(frame, &mut share);
    let inside_share = share.clone();
    data[board_index("inside")..board_index("inside") + tiles].copy_from_slice(&share);
    // Outside the outline is an obstacle on every layer.
    for mask in &mut obstacle_masks {
        for (word, inside) in mask.bits.iter_mut().zip(&inside.bits) {
            *word |= !inside;
        }
    }
    for layer in 0..layers {
        let slot = slot_of(layer, layers);
        let weight = 1.0 / sizes[slot] as f32;
        for (channel, masks) in [(1, &pad_masks), (2, &obstacle_masks), (3, &pour_masks)] {
            masks[layer].tile_shares(frame, &mut share);
            let base = plane_index(channel, slot);
            for tile in 0..tiles {
                data[base + tile] += weight * share[tile];
            }
        }
    }
    for slot in 0..SLOTS {
        if sizes[slot] > 0 {
            let base = plane_index(4, slot);
            data[base..base + tiles].iter_mut().for_each(|value| *value = 1.0);
        }
    }
    // Pins per layer slot, through pins, distinct nets per tile.
    let all = board.all_layers();
    let mut last_net = vec![u32::MAX; tiles];
    let poured: Vec<bool> = (0..board.nets.len())
        .map(|net| board.planes.iter().any(|plane| plane.net as usize == net && plane.connect))
        .collect();
    for (net, record) in board.nets.iter().enumerate() {
        for terminal in &record.terminals {
            let Some(tile) = tile_of(frame, terminal.anchor) else { continue };
            for layer in 0..layers {
                if terminal.layers & (1 << layer) != 0 {
                    let slot = slot_of(layer, layers);
                    data[plane_index(0, slot) + tile] += 1.0 / sizes[slot] as f32;
                }
            }
            if terminal.layers & all == all && layers > 1 {
                data[board_index("through_pins") + tile] += 1.0;
            }
            data[board_index("pins_scaled") + tile] += 1.0;
            if last_net[tile] != net as u32 {
                last_net[tile] = net as u32;
                data[board_index("nets_here") + tile] += 1.0;
            }
        }
    }
    // Routing demand: each net's wire area (length times its track pitch)
    // spread over its bounding box (RUDY), over its spanning tree's edge
    // boxes, and at its pins (pin RUDY). Pour nets apart.
    let tile_mm = frame.tile_mm();
    let mut rudy = vec![0.0f32; tiles];
    let mut rudy_mst = vec![0.0f32; tiles];
    let mut pin_rudy = vec![0.0f32; tiles];
    let mut rudy_pour = vec![0.0f32; tiles];
    for (net, record) in board.nets.iter().enumerate() {
        if record.terminals.len() < 2 {
            continue;
        }
        let rules = board.classes[record.class];
        let track = rules.trace_width + rules.clearance;
        let points: Vec<Point> = record.terminals.iter().map(|terminal| terminal.anchor).collect();
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        for point in &points {
            for axis in 0..2 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
        let edges = mst_edges(&points);
        let length: f64 = edges
            .iter()
            .map(|(a, b)| (points[*a][0] - points[*b][0]).abs() + (points[*a][1] - points[*b][1]).abs())
            .sum();
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
        let (box_low, box_high) = grow(low, high);
        let area = (box_high[0] - box_low[0]) * (box_high[1] - box_low[1]);
        let density = length * track / area;
        if poured[net] {
            add_box(frame, &mut rudy_pour, box_low, box_high, density);
            continue;
        }
        add_box(frame, &mut rudy, box_low, box_high, density);
        for (a, b) in edges {
            let edge_low = [points[a][0].min(points[b][0]), points[a][1].min(points[b][1])];
            let edge_high = [points[a][0].max(points[b][0]), points[a][1].max(points[b][1])];
            let (edge_low, edge_high) = grow(edge_low, edge_high);
            let edge_length = (points[a][0] - points[b][0]).abs() + (points[a][1] - points[b][1]).abs();
            let edge_area = (edge_high[0] - edge_low[0]) * (edge_high[1] - edge_low[1]);
            add_box(frame, &mut rudy_mst, edge_low, edge_high, edge_length * track / edge_area);
        }
        let width = box_high[0] - box_low[0];
        let height = box_high[1] - box_low[1];
        let per_pin = (width + height) / (width * height) * track * tile_mm;
        for point in &points {
            if let Some(tile) = tile_of(frame, *point) {
                pin_rudy[tile] += per_pin as f32;
            }
        }
    }
    for (name, values) in [("rudy", &rudy), ("rudy_mst", &rudy_mst), ("pin_rudy", &pin_rudy), ("rudy_pour", &rudy_pour)] {
        let base = board_index(name);
        data[base..base + tiles].copy_from_slice(values);
    }
    // Board constants, on the board only.
    let mut counts = vec![0usize; board.classes.len()];
    for net in &board.nets {
        counts[net.class] += 1;
    }
    let class = (0..counts.len()).max_by_key(|class| (counts[*class], usize::MAX - class)).unwrap_or(0);
    let rules = board.classes.get(class).copied().unwrap_or(pcb_router::RuleClass {
        trace_width: 0.25,
        clearance: 0.2,
        via_diameter: 0.6,
        via_drill: 0.3,
    });
    for (name, value) in [
        ("layer_count", layers as f64 / SLOTS as f64),
        ("track_pitch", (rules.trace_width + rules.clearance) / tile_mm),
        ("via_pitch", (rules.via_diameter + rules.clearance) / tile_mm),
        ("tile_mm", tile_mm / 1.6),
    ] {
        let base = board_index(name);
        for tile in 0..tiles {
            data[base + tile] = if inside_share[tile] > 0.0 { value as f32 } else { 0.0 };
        }
    }
    FeatureMaps { tiles_x: frame.tiles_x, tiles_y: frame.tiles_y, data }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_router::{Net, Obstacle, RuleClass, Terminal};

    fn frame(nx: usize, ny: usize) -> TileFrame {
        TileFrame { origin: [0.0, 0.0], pitch: 0.1, nx, ny, tile: 16, tiles_x: nx.div_ceil(16), tiles_y: ny.div_ceil(16) }
    }

    fn pad(center: Point, half: f64, layers: u32, net: u32) -> Obstacle {
        Obstacle {
            shape: Shape::rectangle(center, [half, half], 0.0),
            layers,
            kind: ObstacleKind::Copper,
            net: Some(net),
            clearance: 0.0,
            clearance_override: None,
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
            classes: vec![RuleClass { trace_width: 0.2, clearance: 0.2, via_diameter: 0.6, via_drill: 0.3 }],
            neck_width: 0.2,
            // A 0.5 mm square SMD pad on the front (5 x 5 nodes at 0.1 mm)
            // and a through pad.
            obstacles: vec![pad([0.4, 0.4], 0.25, 1, 0), pad([2.4, 2.4], 0.25, 3, 0)],
            nets: vec![Net {
                name: "A".into(),
                class: 0,
                terminals: vec![
                    Terminal { anchor: [0.4, 0.4], layers: 1, pad: 0, contact: None, label: "U1.1".into() },
                    Terminal { anchor: [2.4, 2.4], layers: 3, pad: 1, contact: None, label: "U2.1".into() },
                ],
            }],
            planes: Vec::new(),
            solid_pads: Vec::new(),
            isolated_pads: Vec::new(),
        }
    }

    #[test]
    fn small_board_channels() {
        let board = small_board();
        let frame = frame(32, 32);
        let maps = rasterize(&board, &frame);
        assert_eq!(maps.data.len(), channel_count() * 4);
        let get = |name: &str| maps.channel(name).unwrap().to_vec();
        // Pins: one SMD pin on the front in tile 0, the through pin on both
        // layers in tile 3.
        assert_eq!(get("pins.F"), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(get("pins.B"), vec![0.0, 0.0, 0.0, 1.0]);
        assert_eq!(get("pins.In1"), vec![0.0; 4]);
        assert_eq!(get("through_pins"), vec![0.0, 0.0, 0.0, 1.0]);
        // A 0.5 mm pad covers 5 x 5 node centres of the 256 of a tile.
        let pad = get("pad_copper.F");
        assert!((pad[0] - 25.0 / 256.0).abs() < 1.0e-6, "{pad:?}");
        assert!((pad[3] - 25.0 / 256.0).abs() < 1.0e-6, "{pad:?}");
        assert_eq!(get("pad_copper.B")[0], 0.0);
        // The outline holds every node.
        assert_eq!(get("inside"), vec![1.0; 4]);
        assert_eq!(get("obstacle.F"), vec![0.0; 4]);
        assert_eq!(get("layer_present.F"), vec![1.0; 4]);
        assert_eq!(get("layer_present.In1"), vec![0.0; 4]);
        // RUDY: length 4 mm times a 0.4 mm track pitch over the 2 x 2 mm
        // box, spread over the tiles it overlaps (box from 0.4 to 2.4 mm;
        // tiles from -0.05 to 1.55 and 1.55 to 3.15).
        let rudy = get("rudy");
        let density = 4.0 * 0.4 / 4.0;
        let near = (1.55 - 0.4) / 1.6;
        let far = (2.4 - 1.55) / 1.6;
        let expected = [near * near, far * near, near * far, far * far].map(|share| (density * share) as f32);
        for (value, want) in rudy.iter().zip(expected) {
            assert!((value - want).abs() < 1.0e-5, "{rudy:?} against {expected:?}");
        }
        // Two pins on one net, each tile counts the net once.
        assert_eq!(get("nets_here"), vec![1.0, 0.0, 0.0, 1.0]);
        assert!((get("track_pitch")[0] - 0.25).abs() < 1.0e-6);
        assert!((get("layer_count")[0] - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn outside_the_outline_is_an_obstacle() {
        let mut board = small_board();
        // Only the left half of the board.
        board.outline = vec![[-0.05, -0.05], [1.55, -0.05], [1.55, 3.15], [-0.05, 3.15]];
        let maps = rasterize(&board, &frame(32, 32));
        assert_eq!(maps.channel("inside").unwrap(), &[1.0, 0.0, 1.0, 0.0]);
        assert_eq!(maps.channel("obstacle.B").unwrap(), &[0.0, 1.0, 0.0, 1.0]);
    }
}

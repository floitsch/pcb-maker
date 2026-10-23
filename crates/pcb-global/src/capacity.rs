// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The tile grid and what each tile boundary holds: per layer, how many
//! tracks of the reference class cross it; per tile, how many vias stand
//! in it; per layer and tile, whose pour covers it.
//!
//! A boundary's capacity is measured on the boundary line itself: samples
//! every `sample_mm` along it, each free where a track centreline may run
//! (inside the outline by the edge clearance plus half a track, and away
//! from every obstacle on the layer by its clearance plus half a track).
//! A run of free samples between two obstacles holds `floor(length /
//! pitch) + 1` tracks (exact for a gap between two pads); a run along the
//! whole boundary counts its length over the pitch, and a run from one
//! end of the boundary to an obstacle half a track more (a gap the tile
//! corner splits gets its `+ 1` shared between the two boundaries); the
//! sum is rounded to whole tracks. A pad of a net the boundary cuts adds
//! one track: its own net's escape crosses there through the pad (a pin
//! row lying on a boundary would otherwise close it to its own pins).

use crate::GlobalConfig;
use pcb_router::geometry::{Aabb, Point};
use pcb_router::{Board, Obstacle, ObstacleKind, RuleClass};

/// The tile grid over the board's outline.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// The lower corner of tile (0, 0), in millimetres.
    pub origin: Point,
    pub tile_mm: f64,
    pub tiles_x: usize,
    pub tiles_y: usize,
    pub layers: usize,
}

impl Frame {
    /// Tiles of `tile_mm` over the box of the outline and the terminals.
    pub fn of_board(board: &Board, tile_mm: f64) -> Self {
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        let points = board.outline.iter().copied().chain(
            board
                .nets
                .iter()
                .flat_map(|net| net.terminals.iter().map(|terminal| terminal.anchor)),
        );
        for point in points {
            for axis in 0..2 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
        if !low[0].is_finite() {
            low = [0.0, 0.0];
            high = [tile_mm, tile_mm];
        }
        let count = |axis: usize| (((high[axis] - low[axis]) / tile_mm).ceil() as usize).max(1);
        Self {
            origin: low,
            tile_mm,
            tiles_x: count(0),
            tiles_y: count(1),
            layers: board.layer_count,
        }
    }

    pub fn tiles(&self) -> usize {
        self.tiles_x * self.tiles_y
    }

    /// The tile holding `point` (clamped to the grid).
    pub fn tile_of(&self, point: Point) -> usize {
        let index = |axis: usize, count: usize| {
            (((point[axis] - self.origin[axis]) / self.tile_mm)
                .floor()
                .max(0.0) as usize)
                .min(count - 1)
        };
        index(1, self.tiles_y) * self.tiles_x + index(0, self.tiles_x)
    }

    /// The lower corner of `tile`.
    pub fn corner(&self, tile: usize) -> Point {
        [
            self.origin[0] + (tile % self.tiles_x) as f64 * self.tile_mm,
            self.origin[1] + (tile / self.tiles_x) as f64 * self.tile_mm,
        ]
    }
}

/// What the tiles hold. Edge arrays are indexed `layer * tiles + tile`,
/// where the boundary belongs to the tile left of it (`edges[0]`) or above
/// it (`edges[1]`).
#[derive(Clone, Debug)]
pub struct Capacity {
    /// Tracks of the reference class across the boundary to the right
    /// (`[0]`) and to the tile below (`[1]`).
    pub edges: [Vec<f32>; 2],
    /// Vias of the reference class that fit in the tile.
    pub vias: Vec<f32>,
    /// Per layer and tile: the net (plus one) whose connected pour covers
    /// at least `pour_share` of the tile, 0 for none.
    pub pour_owner: Vec<u32>,
    /// Per layer and tile: the largest share of the tile any pour covers.
    pub pour_share: Vec<f32>,
    /// Per tile: whether any part of it lies inside the outline.
    pub inside: Vec<bool>,
    /// The class capacities count in (the one most nets use).
    pub reference: RuleClass,
    /// Distance tests made, each weighted by its shape's size: the
    /// build's deterministic work.
    pub work: u64,
}

/// What one distance test against `shape` costs, in units of a circle's.
fn shape_cost(shape: &pcb_router::geometry::Shape) -> u64 {
    use pcb_router::geometry::Shape;
    match shape {
        Shape::Circle { .. } | Shape::Capsule { .. } => 1,
        Shape::Polygon { points } => 1 + points.len() as u64 / 4,
        Shape::Union { parts } => parts.iter().map(shape_cost).sum::<u64>().max(1),
    }
}

/// The class most nets with two or more terminals use.
pub fn reference_class(board: &Board) -> RuleClass {
    let mut counts = vec![0usize; board.classes.len()];
    for net in &board.nets {
        if net.terminals.len() > 1 {
            counts[net.class] += 1;
        }
    }
    let class = (0..counts.len())
        .max_by_key(|class| (counts[*class], usize::MAX - class))
        .unwrap_or(0);
    board.classes.get(class).copied().unwrap_or(RuleClass {
        trace_width: 0.25,
        clearance: 0.2,
        via_diameter: 0.6,
        via_drill: 0.3,
    })
}

/// Sorted positions along the other axis where the line `axis = coord`
/// crosses the polygon's edges (even-odd: inside between pairs).
fn crossings(points: &[Point], axis: usize, coord: f64) -> Vec<f64> {
    let other = 1 - axis;
    let mut found = Vec::new();
    if points.len() < 3 {
        return found;
    }
    for index in 0..points.len() {
        let (a, b) = (points[index], points[(index + 1) % points.len()]);
        if (a[axis] <= coord) != (b[axis] <= coord) {
            let t = (coord - a[axis]) / (b[axis] - a[axis]);
            found.push(a[other] + t * (b[other] - a[other]));
        }
    }
    found.sort_by(f64::total_cmp);
    found
}

/// Whether `value` lies inside the even-odd intervals of `crossings`,
/// at least `margin` from their ends.
fn inside_by(crossings: &[f64], value: f64, margin: f64) -> bool {
    crossings
        .as_chunks::<2>()
        .0
        .iter()
        .any(|pair| value >= pair[0] + margin && value <= pair[1] - margin)
}

/// How near a track centreline of `reference` may come to `obstacle`.
fn track_reach(board: &Board, reference: &RuleClass, obstacle: &Obstacle) -> f64 {
    let clearance = match obstacle.kind {
        ObstacleKind::Copper => board.copper_clearance(reference, obstacle),
        ObstacleKind::Keepout => obstacle.clearance,
        ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
    };
    clearance + reference.trace_width / 2.0
}

/// How near a via centre of `reference` may come to `obstacle`.
fn via_reach(board: &Board, reference: &RuleClass, obstacle: &Obstacle) -> f64 {
    match obstacle.kind {
        ObstacleKind::Copper => {
            board.copper_clearance(reference, obstacle) + reference.via_diameter / 2.0
        }
        ObstacleKind::Keepout => obstacle.clearance + reference.via_diameter / 2.0,
        ObstacleKind::Hole => (board.hole_clearance + reference.via_diameter / 2.0)
            .max(board.hole_to_hole + reference.via_drill / 2.0),
    }
}

/// Obstacles that keep tracks out (pads of every net count: the capacity
/// is what is left for passing nets).
fn blocks_tracks(obstacle: &Obstacle) -> bool {
    obstacle.blocks_tracks || (obstacle.kind == ObstacleKind::Copper && obstacle.net.is_some())
}

fn blocks_vias(obstacle: &Obstacle) -> bool {
    obstacle.blocks_vias || obstacle.kind == ObstacleKind::Copper
}

impl Capacity {
    #[allow(
        clippy::needless_range_loop,
        reason = "directions, layers and samples index parallel arrays alike"
    )]
    pub fn build(board: &Board, frame: &Frame, config: &GlobalConfig) -> Self {
        let reference = reference_class(board);
        let layers = frame.layers;
        let tiles = frame.tiles();
        let t = frame.tile_mm;
        let pitch = reference.trace_width + reference.clearance;
        let edge_margin = board.edge_clearance + reference.trace_width / 2.0;
        // Obstacles that matter, with their reaches, bucketed by the tiles
        // their grown boxes touch.
        struct Entry {
            index: usize,
            track: f64,
            via: f64,
            bounds: Aabb,
            cost: u64,
        }
        let mut work = 0u64;
        let mut entries = Vec::new();
        for (index, obstacle) in board.obstacles.iter().enumerate() {
            let (tracks, vias) = (blocks_tracks(obstacle), blocks_vias(obstacle));
            if !tracks && !vias {
                continue;
            }
            let track = if tracks {
                track_reach(board, &reference, obstacle)
            } else {
                f64::NEG_INFINITY
            };
            let via = if vias {
                via_reach(board, &reference, obstacle)
            } else {
                f64::NEG_INFINITY
            };
            let bounds = obstacle.shape.aabb().inflated(track.max(via));
            entries.push(Entry {
                index,
                track,
                via,
                bounds,
                cost: shape_cost(&obstacle.shape),
            });
        }
        let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); tiles];
        let span = |low: f64, high: f64, axis: usize, count: usize| -> Option<(usize, usize)> {
            let a = ((low - frame.origin[axis]) / t).floor();
            let b = ((high - frame.origin[axis]) / t).floor();
            if b < 0.0 || a >= count as f64 {
                return None;
            }
            Some((a.max(0.0) as usize, (b as usize).min(count - 1)))
        };
        for (entry_index, entry) in entries.iter().enumerate() {
            let Some((x0, x1)) = span(
                entry.bounds.minimum[0],
                entry.bounds.maximum[0],
                0,
                frame.tiles_x,
            ) else {
                continue;
            };
            let Some((y0, y1)) = span(
                entry.bounds.minimum[1],
                entry.bounds.maximum[1],
                1,
                frame.tiles_y,
            ) else {
                continue;
            };
            for ty in y0..=y1 {
                for tx in x0..=x1 {
                    buckets[ty * frame.tiles_x + tx].push(entry_index as u32);
                }
            }
        }
        // Boundaries: per direction, per line, the outline's crossings
        // once; per segment the samples.
        let samples = ((t / config.sample_mm).round() as usize).max(1);
        let step = t / samples as f64;
        let mut edges = [vec![0.0f32; layers * tiles], vec![0.0f32; layers * tiles]];
        let mut stamp = vec![u32::MAX; entries.len()];
        let mut blocked = vec![0u32; samples];
        let all = board.all_layers();
        let mut generation = 0u32;
        // The lines a boundary is measured on: the boundary itself, or
        // also the lines a quarter tile into either tile (the mean of the
        // three: obstacles inside the tiles count too).
        let offsets: &[f64] = if config.boundary_lines >= 3 {
            &[-0.25, 0.0, 0.25]
        } else {
            &[0.0]
        };
        let mut sums = vec![0.0f64; layers];
        for direction in 0..2 {
            // direction 0: vertical lines x = const between tx and tx + 1;
            // direction 1: horizontal lines y = const between ty and ty + 1.
            let (fixed, along) = if direction == 0 { (0, 1) } else { (1, 0) };
            let lines = if direction == 0 {
                frame.tiles_x
            } else {
                frame.tiles_y
            };
            let segments = if direction == 0 {
                frame.tiles_y
            } else {
                frame.tiles_x
            };
            for line in 0..lines.saturating_sub(1) {
                let coords: Vec<f64> = offsets
                    .iter()
                    .map(|offset| frame.origin[fixed] + (line as f64 + 1.0 + offset) * t)
                    .collect();
                let outlines: Vec<Vec<f64>> = coords
                    .iter()
                    .map(|coord| crossings(&board.outline, fixed, *coord))
                    .collect();
                for segment in 0..segments {
                    let (tile, next) = if direction == 0 {
                        let tile = segment * frame.tiles_x + line;
                        (tile, tile + 1)
                    } else {
                        let tile = line * frame.tiles_x + segment;
                        (tile, tile + frame.tiles_x)
                    };
                    let start = frame.origin[along] + segment as f64 * t;
                    sums.iter_mut().for_each(|sum| *sum = 0.0);
                    for (coord, outline) in coords.iter().copied().zip(&outlines) {
                        let position = |k: usize| -> Point {
                            let mut point = [0.0; 2];
                            point[fixed] = coord;
                            point[along] = start + (k as f64 + 0.5) * step;
                            point
                        };
                        for (k, slot) in blocked.iter_mut().enumerate() {
                            let value = start + (k as f64 + 0.5) * step;
                            *slot = if inside_by(outline, value, edge_margin) {
                                0
                            } else {
                                all
                            };
                        }
                        if blocked.iter().all(|mask| *mask == all) {
                            continue;
                        }
                        generation = generation.wrapping_add(1);
                        // Pads of a net the line cuts, per layer: each
                        // passes its own net's track (its escape) across.
                        let mut own_passages = vec![0u32; layers];
                        let mut segment_begin = position(0);
                        segment_begin[along] = start;
                        let mut segment_end = segment_begin;
                        segment_end[along] = start + t;
                        for bucket in [tile, next] {
                            for &entry_index in &buckets[bucket] {
                                let entry_index = entry_index as usize;
                                if stamp[entry_index] == generation {
                                    continue;
                                }
                                stamp[entry_index] = generation;
                                let entry = &entries[entry_index];
                                if !entry.track.is_finite() {
                                    continue;
                                }
                                let obstacle = &board.obstacles[entry.index];
                                let grown = obstacle.shape.aabb().inflated(entry.track);
                                if coord < grown.minimum[fixed] || coord > grown.maximum[fixed] {
                                    continue;
                                }
                                let first = ((grown.minimum[along] - start) / step - 0.5)
                                    .ceil()
                                    .max(0.0);
                                let last = ((grown.maximum[along] - start) / step - 0.5)
                                    .floor()
                                    .min(samples as f64 - 1.0);
                                if first > last {
                                    continue;
                                }
                                let layers_of = obstacle.layers & all;
                                if obstacle.kind == ObstacleKind::Copper
                                    && obstacle.net.is_some()
                                    && obstacle
                                        .shape
                                        .distance_to_segment(segment_begin, segment_end)
                                        <= 0.0
                                {
                                    for (layer, count) in own_passages.iter_mut().enumerate() {
                                        if layers_of & (1 << layer) != 0 {
                                            *count += 1;
                                        }
                                    }
                                }
                                for (k, slot) in blocked
                                    .iter_mut()
                                    .enumerate()
                                    .take(last as usize + 1)
                                    .skip(first as usize)
                                {
                                    if *slot & layers_of == layers_of {
                                        continue;
                                    }
                                    work += entry.cost;
                                    if obstacle.shape.distance_to_point(position(k)) < entry.track {
                                        *slot |= layers_of;
                                    }
                                }
                            }
                        }
                        for (layer, sum) in sums.iter_mut().enumerate() {
                            let bit = 1u32 << layer;
                            let mut total = 0.0f64;
                            let mut run = 0usize;
                            let mut run_start = 0usize;
                            for k in 0..=samples {
                                let free = k < samples && blocked[k] & bit == 0;
                                if free {
                                    if run == 0 {
                                        run_start = k;
                                    }
                                    run += 1;
                                } else if run > 0 {
                                    let open = (run_start == 0) as u8 + (k == samples) as u8;
                                    total += match open {
                                        0 => ((run - 1) as f64 * step / pitch).floor() + 1.0,
                                        1 => run as f64 * step / pitch + 0.5,
                                        _ => run as f64 * step / pitch,
                                    };
                                    run = 0;
                                }
                            }
                            // Tracks are whole.
                            *sum += total.round() + own_passages[layer] as f64;
                        }
                    }
                    for (layer, sum) in sums.iter().enumerate() {
                        edges[direction][layer * tiles + tile] =
                            (sum / offsets.len() as f64) as f32;
                    }
                }
            }
        }
        // Vias: sites on a grid of `via_sample_mm` inside each tile, free
        // on every layer.
        let via_samples = ((t / config.via_sample_mm).round() as usize).max(1);
        let via_step = t / via_samples as f64;
        let via_pitch = reference.via_diameter + reference.clearance;
        let site_area = via_pitch * via_pitch;
        let mut vias = vec![0.0f32; tiles];
        let mut inside = vec![false; tiles];
        for ty in 0..frame.tiles_y {
            let rows: Vec<Vec<f64>> = (0..via_samples)
                .map(|row| {
                    let y = frame.origin[1] + ty as f64 * t + (row as f64 + 0.5) * via_step;
                    crossings(&board.outline, 1, y)
                })
                .collect();
            for tx in 0..frame.tiles_x {
                let tile = ty * frame.tiles_x + tx;
                let corner = frame.corner(tile);
                let mut free = 0usize;
                let mut any_inside = false;
                for (row, outline) in rows.iter().enumerate() {
                    for column in 0..via_samples {
                        let point = [
                            corner[0] + (column as f64 + 0.5) * via_step,
                            corner[1] + (row as f64 + 0.5) * via_step,
                        ];
                        if !inside_by(outline, point[0], 0.0) {
                            continue;
                        }
                        any_inside = true;
                        let hit = buckets[tile].iter().any(|&entry_index| {
                            let entry = &entries[entry_index as usize];
                            entry.via.is_finite()
                                && point[0] >= entry.bounds.minimum[0]
                                && point[0] <= entry.bounds.maximum[0]
                                && point[1] >= entry.bounds.minimum[1]
                                && point[1] <= entry.bounds.maximum[1]
                                && {
                                    work += entry.cost;
                                    board.obstacles[entry.index].shape.distance_to_point(point)
                                        < entry.via
                                }
                        });
                        if !hit {
                            free += 1;
                        }
                    }
                }
                inside[tile] = any_inside;
                let share = free as f64 / (via_samples * via_samples) as f64;
                vias[tile] = (share * t * t / site_area) as f32;
            }
        }
        // Pours: 4 x 4 samples a tile, the higher-priority zones cut out.
        let mut pour_owner = vec![0u32; layers * tiles];
        let mut pour_share = vec![0.0f32; layers * tiles];
        const SUB: usize = 4;
        for plane in &board.planes {
            if plane.layer >= layers {
                continue;
            }
            let mut counts = vec![0u32; tiles];
            for row in 0..frame.tiles_y * SUB {
                let y = frame.origin[1] + (row as f64 + 0.5) * t / SUB as f64;
                let polygon = crossings(&plane.polygon, 1, y);
                if polygon.is_empty() {
                    continue;
                }
                let excluded: Vec<Vec<f64>> = plane
                    .excluded
                    .iter()
                    .map(|points| crossings(points, 1, y))
                    .collect();
                for column in 0..frame.tiles_x * SUB {
                    let x = frame.origin[0] + (column as f64 + 0.5) * t / SUB as f64;
                    if inside_by(&polygon, x, 0.0)
                        && !excluded.iter().any(|cut| inside_by(cut, x, 0.0))
                    {
                        counts[(row / SUB) * frame.tiles_x + column / SUB] += 1;
                    }
                }
            }
            for (tile, count) in counts.iter().enumerate() {
                let share = *count as f32 / (SUB * SUB) as f32;
                let index = plane.layer * tiles + tile;
                if share > pour_share[index] {
                    pour_share[index] = share;
                    if plane.connect && share as f64 >= config.pour_share {
                        pour_owner[index] = plane.net + 1;
                    }
                }
            }
        }
        let share = config.capacity_share as f32;
        if share != 1.0 {
            for value in edges.iter_mut().flatten().chain(vias.iter_mut()) {
                *value *= share;
            }
        }
        Self {
            edges,
            vias,
            pour_owner,
            pour_share,
            inside,
            reference,
            work,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pcb_router::geometry::Shape;
    use pcb_router::{Net, Terminal};

    pub(crate) fn empty_board(width: f64, height: f64, layers: usize) -> Board {
        Board {
            layer_count: layers,
            outline: vec![[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]],
            edge_clearance: 0.0,
            hole_clearance: 0.25,
            hole_to_hole: 0.25,
            classes: vec![RuleClass {
                trace_width: 0.2,
                clearance: 0.2,
                via_diameter: 0.6,
                via_drill: 0.3,
            }],
            neck_width: 0.2,
            obstacles: Vec::new(),
            nets: vec![Net {
                name: "A".into(),
                class: 0,
                terminals: vec![
                    Terminal {
                        anchor: [0.5, 0.5],
                        layers: 1,
                        pad: 0,
                        contact: None,
                        label: "U1.1".into(),
                    },
                    Terminal {
                        anchor: [width - 0.5, 0.5],
                        layers: 1,
                        pad: 0,
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
            no_via_pads: Default::default(),
            fill_passes: Vec::new(),
        }
    }

    pub(crate) fn wall(center: Point, half: Point, layers: u32) -> Obstacle {
        Obstacle {
            shape: Shape::rectangle(center, half, 0.0),
            layers,
            kind: ObstacleKind::Copper,
            net: None,
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: String::new(),
        }
    }

    #[test]
    fn a_free_boundary_holds_its_length_over_the_pitch() {
        let board = empty_board(4.0, 2.0, 2);
        let config = GlobalConfig {
            tile_mm: 2.0,
            ..GlobalConfig::default()
        };
        let frame = Frame::of_board(&board, config.tile_mm);
        assert_eq!((frame.tiles_x, frame.tiles_y), (2, 1));
        let capacity = Capacity::build(&board, &frame, &config);
        // 2 mm over a 0.4 mm pitch, on both layers (whole tracks).
        assert!(
            (capacity.edges[0][0] - 5.0).abs() < 1.0e-4,
            "{:?}",
            capacity.edges
        );
        assert!((capacity.edges[0][2] - 5.0).abs() < 1.0e-4);
        // No boundary below the single row.
        assert_eq!(capacity.edges[1][0], 0.0);
        // A free 2 x 2 mm tile: 4 / 0.8^2 vias.
        assert!(
            (capacity.vias[0] - 6.25).abs() < 1.0e-3,
            "{:?}",
            capacity.vias
        );
    }

    #[test]
    fn a_gap_between_two_pads_holds_whole_tracks() {
        let mut board = empty_board(4.0, 2.0, 1);
        // Two walls across the boundary x = 2 from y = 0 to 0.6 and from
        // 1.4 to 2: a 0.8 mm gap. Centrelines keep 0.3 mm from each: a
        // 0.2 mm free run, one track.
        board.obstacles.push(wall([2.0, 0.3], [0.5, 0.3], 1));
        board.obstacles.push(wall([2.0, 1.7], [0.5, 0.3], 1));
        let config = GlobalConfig {
            tile_mm: 2.0,
            ..GlobalConfig::default()
        };
        let frame = Frame::of_board(&board, config.tile_mm);
        let capacity = Capacity::build(&board, &frame, &config);
        assert!(
            (capacity.edges[0][0] - 1.0).abs() < 1.0e-4,
            "{:?}",
            capacity.edges
        );
        // A 1.6 mm gap: a 1.0 mm free run, three tracks at a 0.4 mm pitch.
        board.obstacles[0] = wall([2.0, 0.1], [0.5, 0.1], 1);
        board.obstacles[1] = wall([2.0, 1.9], [0.5, 0.1], 1);
        let capacity = Capacity::build(&board, &frame, &config);
        assert!(
            (capacity.edges[0][0] - 3.0).abs() < 1.0e-4,
            "{:?}",
            capacity.edges
        );
    }

    #[test]
    fn a_pad_on_a_boundary_passes_its_own_net() {
        // The boundary x = 2 closed by copper of no net, but for a pad of
        // net 0 that it cuts: no track passes, the pad's own escape does.
        let mut board = empty_board(4.0, 2.0, 1);
        board.obstacles.push(wall([2.0, 1.0], [0.5, 1.0], 1));
        let mut pad = wall([2.0, 1.0], [0.3, 0.2], 1);
        pad.net = Some(0);
        board.obstacles.push(pad);
        let config = GlobalConfig {
            tile_mm: 2.0,
            ..GlobalConfig::default()
        };
        let frame = Frame::of_board(&board, config.tile_mm);
        let capacity = Capacity::build(&board, &frame, &config);
        assert_eq!(capacity.edges[0][0], 1.0, "{:?}", capacity.edges);
        // Without the pad the boundary is closed.
        board.obstacles.pop();
        let capacity = Capacity::build(&board, &frame, &config);
        assert_eq!(capacity.edges[0][0], 0.0, "{:?}", capacity.edges);
    }

    #[test]
    fn the_outline_bounds_the_boundaries() {
        // An L: the upper right tile is outside.
        let mut board = empty_board(4.0, 4.0, 1);
        board.outline = vec![
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 2.0],
            [2.0, 2.0],
            [2.0, 4.0],
            [0.0, 4.0],
        ];
        let config = GlobalConfig {
            tile_mm: 2.0,
            ..GlobalConfig::default()
        };
        let frame = Frame::of_board(&board, config.tile_mm);
        let capacity = Capacity::build(&board, &frame, &config);
        // Tile 0 (lower left) to tile 1 (lower right): open.
        assert!(capacity.edges[0][0] > 4.0);
        // Tile 2 (upper left) to tile 3 (outside): closed.
        assert_eq!(capacity.edges[0][2], 0.0);
        // Tile 1 to tile 3 (below it in index order): closed.
        assert_eq!(capacity.edges[1][1], 0.0);
        assert!(!capacity.inside[3]);
        assert_eq!(capacity.vias[3], 0.0);
    }
}

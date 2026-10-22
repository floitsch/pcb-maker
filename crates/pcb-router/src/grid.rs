// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The routing lattice and the static (never changing during a run) maps
//! derived from pads, keepouts, holes and the outline.

use crate::board::{Board, ClassId, NetId, ObstacleKind};
use crate::geometry::{Aabb, Point, Shape, polygon_edges};

/// Positions are written to the board in nanometres; keep that much slack
/// so rounding can never turn a tangent into a violation.
pub const SAFETY: f64 = 2.0e-6;

pub const FREE: u32 = 0;
pub const BLOCKED: u32 = u32::MAX;

pub fn owner(net: NetId) -> u32 {
    net + 1
}

/// `FREE`, `BLOCKED` and `owner` in 16 bits, for the per-node maps that
/// only name a net in rare places (the edge and via owners): half the
/// bytes of a lattice-sized map. Boards have far fewer nets.
pub const BLOCKED16: u16 = u16::MAX;
pub const FREE16: u16 = 0;

/// A 16-bit map value (`FREE16`, `BLOCKED16`, `owner16`) as the 32-bit
/// one (`FREE`, `BLOCKED`, `owner`).
#[inline(always)]
pub fn widen16(value: u16) -> u32 {
    if value == BLOCKED16 {
        BLOCKED
    } else {
        value as u32
    }
}

/// The most nets the router's maps name: the 16-bit maps (`owner16`, the
/// search's `Hot` guard and cover, the via cover's low half) hold
/// `owner(net)` below `BLOCKED16`.
pub const MAX_NETS: usize = BLOCKED16 as usize - 2;

/// The most copper layers the router's maps hold (`free_layers`: a bit a
/// layer in 32 bits).
pub const MAX_LAYERS: usize = 32;

/// Whether the router's maps can hold `board`: at most `MAX_NETS` nets and
/// `MAX_LAYERS` copper layers. An error names the limit and the board's
/// count; `Router::new` asserts it (`Router::try_new` returns it), and
/// pcb-kicad's lowering returns it as an input error.
pub fn check_capacity(board: &Board) -> Result<(), String> {
    if board.nets.len() > MAX_NETS {
        return Err(format!(
            "the board has {} nets: the router names nets in 16 bits, at most {MAX_NETS}",
            board.nets.len()
        ));
    }
    if board.layer_count > MAX_LAYERS {
        return Err(format!(
            "the board has {} copper layers: the router holds at most {MAX_LAYERS} (a bit a layer in 32 bits)",
            board.layer_count
        ));
    }
    Ok(())
}

/// The most nodes a layer of the lattice may have: the maps and the
/// search's records index a layer's nodes in 32 bits.
pub const MAX_LATTICE_NODES: usize = u32::MAX as usize;

/// A floor on the router's lattice maps, in bytes per node and layer:
/// `LATTICE_BYTES_A_NODE` and `LATTICE_BYTES_A_CLASS` per rule class.
/// Under every one of 1 644 lattices the logs of 2026-10 measured (the
/// router's `memory: N MB in maps` line, its classes counted after the
/// neck classes it adds) by a factor of 1.25 or more, so that what the
/// floor refuses would not have fitted (`LatticeSize::maps_bytes_floor`).
pub const LATTICE_BYTES_A_NODE: f64 = 32.0;
pub const LATTICE_BYTES_A_CLASS: f64 = 6.0;

/// The lattice a board needs at a pitch, from its outline's extent alone,
/// before anything is chosen or built: what bounds the maps before they
/// are allocated (`Grid::choose` puts the origin up to a pitch before the
/// outline, so its lattice has these nodes or one more each way).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatticeSize {
    /// The outline's extent, millimetres (infinite or NaN when the
    /// outline's coordinates are, or when it has none).
    pub extent: [f64; 2],
    pub pitch: f64,
    /// Nodes along each axis, as floating point (an absurd board's count
    /// does not fit an integer).
    pub nodes: [f64; 2],
}

impl LatticeSize {
    pub fn of(board: &Board, pitch: f64) -> Self {
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for point in &board.outline {
            for axis in 0..2 {
                // Not `f64::min`: a NaN must not drop out of the extent.
                if point[axis].is_nan() || point[axis] < minimum[axis] {
                    minimum[axis] = point[axis];
                }
                if point[axis].is_nan() || point[axis] > maximum[axis] {
                    maximum[axis] = point[axis];
                }
            }
        }
        let extent = [maximum[0] - minimum[0], maximum[1] - minimum[1]];
        Self {
            extent,
            pitch,
            nodes: extent.map(|extent| (extent / pitch).floor() + 1.0),
        }
    }

    /// Nodes on one layer.
    pub fn cells(&self) -> f64 {
        self.nodes[0] * self.nodes[1]
    }

    /// A floor on the router's lattice maps, in bytes, with `layers`
    /// copper layers and `classes` rule classes (`LATTICE_BYTES_A_NODE`).
    pub fn maps_bytes_floor(&self, layers: usize, classes: usize) -> f64 {
        self.cells()
            * layers as f64
            * (LATTICE_BYTES_A_NODE + LATTICE_BYTES_A_CLASS * classes as f64)
    }

    /// Whether the maps can index the lattice at all: a finite outline
    /// and at most `MAX_LATTICE_NODES` nodes a layer.
    pub fn check(&self) -> Result<(), String> {
        if !(self
            .extent
            .iter()
            .all(|extent| extent.is_finite() && *extent >= 0.0)
            && self.pitch.is_finite()
            && self.pitch > 0.0)
        {
            return Err(format!(
                "the board's outline spans {:?} mm at a pitch of {} mm: the lattice needs a finite outline and pitch",
                self.extent, self.pitch
            ));
        }
        if self.cells().is_nan() || self.cells() > MAX_LATTICE_NODES as f64 {
            return Err(format!(
                "the board's outline spans {:.1} x {:.1} mm: at a pitch of {} mm its lattice has {:.0} x {:.0} nodes a layer, more than the router's maps index ({MAX_LATTICE_NODES})",
                self.extent[0], self.extent[1], self.pitch, self.nodes[0], self.nodes[1]
            ));
        }
        Ok(())
    }
}

pub fn owner16(net: NetId) -> u16 {
    debug_assert!(net + 1 < BLOCKED16 as u32);
    (net + 1) as u16
}

/// Unit steps for the eight planar directions, counter-clockwise from +x.
pub const DIRECTIONS: [(i32, i32); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

#[derive(Clone, Debug)]
pub struct Grid {
    pub origin: Point,
    pub pitch: f64,
    pub nx: usize,
    pub ny: usize,
}

/// Channels between neighbouring pads that a track of the narrowest class
/// fits through with less than a pitch of play: (axis the pads are spaced
/// along, channel centre on that axis, slack each side). Only channels next
/// to a pad fenced in on all four sides count (the inside of a BGA), where
/// the channel is the only way in; between the pads of a connector row it is
/// a shortcut that must not move the lattice off the pad centres.
fn tight_channels(board: &Board) -> Vec<(usize, f64, f64)> {
    let width = board
        .classes
        .iter()
        .map(|class| class.trace_width)
        .fold(f64::INFINITY, f64::min)
        .min(if board.neck_width > 0.0 {
            board.neck_width
        } else {
            f64::INFINITY
        });
    let clearance = board
        .classes
        .iter()
        .map(|class| class.clearance)
        .fold(f64::INFINITY, f64::min);
    if !width.is_finite() || !clearance.is_finite() {
        return Vec::new();
    }
    let pads: Vec<(Point, Aabb, f64)> = board
        .nets
        .iter()
        .flat_map(|net| net.terminals.iter())
        .map(|terminal| {
            let pad = &board.obstacles[terminal.pad];
            (
                terminal.anchor,
                pad.shape.aabb(),
                pad.clearance.max(clearance),
            )
        })
        .collect();
    // Row and column neighbours within 2 mm, through a coarse hash.
    let cell = 2.0;
    let mut buckets: std::collections::HashMap<(i64, i64), Vec<usize>> =
        std::collections::HashMap::new();
    for (index, (anchor, _, _)) in pads.iter().enumerate() {
        buckets
            .entry((
                (anchor[0] / cell).floor() as i64,
                (anchor[1] / cell).floor() as i64,
            ))
            .or_default()
            .push(index);
    }
    // (neighbour, axis of separation, sign)
    let neighbours: Vec<Vec<(usize, usize, bool)>> = (0..pads.len())
        .map(|index| {
            let anchor = pads[index].0;
            let (bx, by) = (
                (anchor[0] / cell).floor() as i64,
                (anchor[1] / cell).floor() as i64,
            );
            let mut found = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let Some(others) = buckets.get(&(bx + dx, by + dy)) else {
                        continue;
                    };
                    for &other in others {
                        if other == index {
                            continue;
                        }
                        let other_anchor = pads[other].0;
                        let separation = [other_anchor[0] - anchor[0], other_anchor[1] - anchor[1]];
                        let axis = if separation[0].abs() >= separation[1].abs() {
                            0
                        } else {
                            1
                        };
                        if separation[1 - axis].abs() <= 0.05 && separation[axis].abs() <= cell {
                            found.push((other, axis, separation[axis] > 0.0));
                        }
                    }
                }
            }
            found
        })
        .collect();
    let enclosed: Vec<bool> = neighbours
        .iter()
        .map(|found| {
            let mut sides = [false; 4];
            for (_, axis, positive) in found {
                sides[axis * 2 + *positive as usize] = true;
            }
            sides.iter().all(|side| *side)
        })
        .collect();
    let mut channels = Vec::new();
    for (index, (_, aabb, pad_clearance)) in pads.iter().enumerate() {
        for &(other, axis, positive) in &neighbours[index] {
            if other <= index || !(enclosed[index] || enclosed[other]) {
                continue;
            }
            let (_, other_aabb, other_clearance) = &pads[other];
            let (near, far) = if positive {
                (aabb.maximum[axis], other_aabb.minimum[axis])
            } else {
                (other_aabb.maximum[axis], aabb.minimum[axis])
            };
            let free = far - near - pad_clearance - other_clearance;
            // A channel with no play at all is not a channel (rounding
            // decides whether it passes DRC); it must not steer the lattice.
            if free < width + 0.005 {
                continue;
            }
            channels.push((axis, (near + far) / 2.0, (free - width) / 2.0));
        }
    }
    channels
}

impl Grid {
    pub fn cells(&self) -> usize {
        self.nx * self.ny
    }

    /// Whether the maps can hold this lattice: at most
    /// `MAX_LATTICE_NODES` nodes a layer (checked before any map is
    /// allocated; `cells` would overflow past it).
    pub fn check_size(&self) -> Result<(), String> {
        match self.nx.checked_mul(self.ny) {
            Some(cells) if cells <= MAX_LATTICE_NODES && self.pitch > 0.0 => Ok(()),
            _ => Err(format!(
                "a lattice of {} x {} nodes at a pitch of {} mm: more than the router's maps index ({MAX_LATTICE_NODES} nodes a layer)",
                self.nx, self.ny, self.pitch
            )),
        }
    }

    pub fn index(&self, x: usize, y: usize) -> usize {
        y * self.nx + x
    }

    pub fn xy(&self, index: usize) -> (usize, usize) {
        (index % self.nx, index / self.nx)
    }

    pub fn center(&self, x: usize, y: usize) -> Point {
        [
            self.origin[0] + x as f64 * self.pitch,
            self.origin[1] + y as f64 * self.pitch,
        ]
    }

    pub fn center_of(&self, index: usize) -> Point {
        let (x, y) = self.xy(index);
        self.center(x, y)
    }

    pub fn neighbor(&self, index: usize, direction: usize) -> Option<usize> {
        let (x, y) = self.xy(index);
        let (dx, dy) = DIRECTIONS[direction];
        let nx = x as i64 + dx as i64;
        let ny = y as i64 + dy as i64;
        (nx >= 0 && ny >= 0 && (nx as usize) < self.nx && (ny as usize) < self.ny)
            .then(|| self.index(nx as usize, ny as usize))
    }

    /// Inclusive node range whose centres can lie inside `bounds`.
    pub fn node_range(&self, bounds: Aabb) -> Option<(usize, usize, usize, usize)> {
        let low = |value: f64, origin: f64| ((value - origin) / self.pitch).ceil();
        let high = |value: f64, origin: f64| ((value - origin) / self.pitch).floor();
        let x0 = low(bounds.minimum[0], self.origin[0]).max(0.0);
        let y0 = low(bounds.minimum[1], self.origin[1]).max(0.0);
        let x1 = high(bounds.maximum[0], self.origin[0]).min(self.nx as f64 - 1.0);
        let y1 = high(bounds.maximum[1], self.origin[1]).min(self.ny as f64 - 1.0);
        (x0 <= x1 && y0 <= y1).then_some((x0 as usize, y0 as usize, x1 as usize, y1 as usize))
    }

    pub fn nearest_node(&self, point: Point) -> Option<usize> {
        let x = ((point[0] - self.origin[0]) / self.pitch).round();
        let y = ((point[1] - self.origin[1]) / self.pitch).round();
        (x >= 0.0 && y >= 0.0 && (x as usize) < self.nx && (y as usize) < self.ny)
            .then(|| self.index(x as usize, y as usize))
    }

    /// Picks the candidate pitch and lattice phase that puts the most pad
    /// anchors exactly on nodes. Through-hole boards live on an imperial
    /// lattice, and a one-track channel between two pads is only usable when
    /// a node row falls in its centre.
    pub fn choose(board: &Board, candidates: &[f64]) -> Self {
        // Pad centres vote for the lattice phase. Only narrow pads (a
        // fine-pitch part's, where only an aligned node row can leave the
        // pad) vote when there are any: a wide pad has legal nodes inside
        // it whatever the phase.
        // A pad narrow across an axis needs a node line aligned on that
        // axis (tracks leave it along the other one).
        let mut anchors: Vec<(Point, [f64; 2])> = board
            .nets
            .iter()
            .flat_map(|net| {
                net.terminals.iter().map(|terminal| {
                    let bounds = board.obstacles[terminal.pad].shape.aabb();
                    let extent = [
                        bounds.maximum[0] - bounds.minimum[0],
                        bounds.maximum[1] - bounds.minimum[1],
                    ];
                    (
                        terminal.anchor,
                        extent.map(|width| if width < 0.35 { 1.0 } else { 0.0 }),
                    )
                })
            })
            .collect();
        for axis in 0..2 {
            if anchors.iter().all(|(_, weight)| weight[axis] == 0.0) {
                anchors
                    .iter_mut()
                    .for_each(|(_, weight)| weight[axis] = 1.0);
            }
        }
        let mut bounds = Aabb {
            minimum: [f64::INFINITY; 2],
            maximum: [f64::NEG_INFINITY; 2],
        };
        for point in &board.outline {
            for (axis, value) in point.iter().enumerate() {
                bounds.minimum[axis] = bounds.minimum[axis].min(*value);
                bounds.maximum[axis] = bounds.maximum[axis].max(*value);
            }
        }
        let channels = tight_channels(board);
        if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
            eprintln!(
                "lattice: {} tight channels, {} pad anchors",
                channels.len(),
                anchors.len()
            );
            for (axis, centre, slack) in channels.iter().take(6) {
                eprintln!("  channel axis {axis} at {centre:.3} slack {slack:.3}");
            }
        }
        let mut best: Option<(f64, f64, [f64; 2])> = None;
        for &pitch in candidates {
            let mut phase = [0.0; 2];
            let mut score = 0.0;
            for axis in 0..2 {
                let anchor_weight: f64 = anchors.iter().map(|(_, weight)| weight[axis]).sum();
                let mut histogram = std::collections::BTreeMap::<i64, f64>::new();
                for (anchor, weight) in &anchors {
                    let residue = anchor[axis].rem_euclid(pitch);
                    *histogram
                        .entry((residue * 1.0e4).round() as i64)
                        .or_default() += weight[axis];
                }
                // Residues wrap around: 0 and `pitch` are the same phase.
                let wrap = (pitch * 1.0e4).round() as i64;
                let mut merged = std::collections::BTreeMap::<i64, f64>::new();
                for (residue, count) in histogram {
                    *merged.entry(residue % wrap).or_default() += count;
                }
                // A tight channel needs a node row within its slack of its
                // centre; missing one walls off a pad, so all channels
                // together weigh twice all pad centres. Candidate phases
                // come from both.
                let tight: Vec<(f64, f64)> = channels
                    .iter()
                    .filter(|(channel_axis, _, slack)| {
                        *channel_axis == axis && *slack < pitch / 2.0
                    })
                    .map(|(_, centre, slack)| (centre.rem_euclid(pitch), *slack))
                    .collect();
                let mut phases: Vec<(i64, f64)> = merged.into_iter().collect();
                for (centre, _) in &tight {
                    phases.push(((centre * 1.0e4).round() as i64 % wrap, 0.0));
                }
                let mut axis_best: Option<(f64, i64)> = None;
                for (residue, count) in phases {
                    let phase = residue as f64 / 1.0e4;
                    let aligned = tight
                        .iter()
                        .filter(|(centre, slack)| {
                            let offset = (centre - phase).rem_euclid(pitch);
                            offset.min(pitch - offset) <= *slack + 1.0e-6
                        })
                        .count();
                    let aligned_anchors = if count == 0.0 {
                        anchors
                            .iter()
                            .filter(|(anchor, _)| {
                                let offset = (anchor[axis] - phase).rem_euclid(pitch);
                                offset.min(pitch - offset) <= 1.0e-4
                            })
                            .map(|(_, weight)| weight[axis])
                            .sum()
                    } else {
                        count
                    };
                    let value = 2.0 * aligned as f64 / tight.len().max(1) as f64
                        + aligned_anchors / anchor_weight.max(1.0e-9);
                    if axis_best.is_none_or(|(best, best_residue)| {
                        value > best + 1e-9 || (value > best - 1e-9 && residue < best_residue)
                    }) {
                        axis_best = Some((value, residue));
                    }
                }
                let (value, residue) = axis_best.unwrap_or((0.0, 0));
                phase[axis] = residue as f64 / 1.0e4;
                score += value;
            }
            if best.is_none_or(|(best_score, _, _)| score > best_score + 1e-9) {
                best = Some((score, pitch, phase));
            }
        }
        let (_, pitch, phase) = best.unwrap_or((0.0, candidates[0], [0.0; 2]));
        let mut origin = [0.0; 2];
        for axis in 0..2 {
            let steps = ((bounds.minimum[axis] - phase[axis]) / pitch).floor();
            // On whole nanometres, as the board file holds positions: node
            // distances then come out exact (two tracks at exactly their
            // clearance stay legal).
            origin[axis] = ((phase[axis] + steps * pitch) * 1.0e6).round() / 1.0e6;
        }
        // Saturating: an outline the router cannot hold (an infinite
        // coordinate) gives a lattice `check_size` refuses, not an
        // overflow here.
        let nx = (((bounds.maximum[0] - origin[0]) / pitch).ceil() as usize).saturating_add(1);
        let ny = (((bounds.maximum[1] - origin[1]) / pitch).ceil() as usize).saturating_add(1);
        Self {
            origin,
            pitch,
            nx,
            ny,
        }
    }
}

/// Where a trace centreline or a via of one rule class may be, considering
/// only immovable objects.
///
/// A class no search runs with (a pour's brush, read only by the pour
/// model) has no `via_owner` and `free_layers`, which only a search reads.
#[derive(Clone, Debug)]
pub struct StaticMaps {
    /// Per layer and node: `FREE16`, `BLOCKED16`, or the only net allowed
    /// there (`owner16`); `trace_at` reads it as `FREE`, `BLOCKED` or an
    /// `owner`. 16 bits since 2026-10-22 (the boards' nets fit,
    /// `MAX_NETS`): the largest static map, two bytes a node, layer and
    /// class less.
    pub trace: Vec<Vec<u16>>,
    /// Per layer and node: directions whose unit step passes too close to an
    /// obstacle although both end nodes are legal.
    pub edge_block: Vec<Vec<u8>>,
    /// The only net exempt from `edge_block` (its own pad, `owner16`), or
    /// `BLOCKED16`.
    pub edge_owner: Vec<Vec<u16>>,
    /// Per node: a via of this class is forbidden (a bit a node; a byte
    /// until 2026-10-22).
    pub via_blocked: NodeBits,
    /// Where `via_blocked` holds only for other nets: inside a net's own
    /// surface pad, deep enough for the whole via (a thermal via in a
    /// ground pad, as designers place them), or within only the hole
    /// clearance of its own plated hole. `owner16`, or 0.
    pub via_owner: Vec<u16>,
    /// Per node: the layers (bits) where `trace` is `FREE`, so that a via
    /// step reads one word instead of one per layer.
    /// In as few bytes a node as the board's layers need (one up to
    /// eight layers; four bytes until 2026-10-22).
    pub free_layers: LayerBits,
    /// `trace` without the solder mask openings, where it differs: what a
    /// pour's fill may cover.
    fill_trace: Option<FillTrace>,
    /// What building the maps took, counted: nodes tested against an
    /// obstacle and the shape vertices those tests went through.
    pub tests: u64,
    pub vertex_tests: u64,
    /// The step-against-shape tests of nodes near an obstacle's edge.
    pub tight_tests: u64,
}

/// A bit per node (`StaticMaps::via_blocked`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeBits {
    words: Vec<u64>,
    len: usize,
}

impl NodeBits {
    pub fn from_bools(values: &[bool]) -> Self {
        let mut words = vec![0u64; values.len().div_ceil(64)];
        for (index, value) in values.iter().enumerate() {
            if *value {
                words[index / 64] |= 1 << (index % 64);
            }
        }
        Self {
            words,
            len: values.len(),
        }
    }

    #[inline(always)]
    pub fn get(&self, index: usize) -> bool {
        self.words[index / 64] & (1 << (index % 64)) != 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = bool> + '_ {
        (0..self.len).map(|index| self.get(index))
    }

    pub fn bytes(&self) -> usize {
        self.words.capacity() * 8
    }
}

/// Per node a set of layers (bits), in the bytes `layers` need
/// (`StaticMaps::free_layers`).
#[derive(Clone, Debug)]
pub struct LayerBits {
    bytes: Vec<u8>,
    width: usize,
}

impl LayerBits {
    pub fn new(values: &[u32], layers: usize) -> Self {
        let width = match layers {
            0..=8 => 1,
            9..=16 => 2,
            _ => 4,
        };
        let mut bytes = Vec::with_capacity(values.len() * width);
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes()[..width]);
        }
        Self { bytes, width }
    }

    #[inline(always)]
    pub fn get(&self, index: usize) -> u32 {
        match self.width {
            1 => self.bytes[index] as u32,
            2 => u16::from_le_bytes([self.bytes[2 * index], self.bytes[2 * index + 1]]) as u32,
            _ => u32::from_le_bytes(self.bytes[4 * index..4 * index + 4].try_into().unwrap()),
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes.capacity()
    }
}

/// The cells where `trace` differs from what a pour's fill may cover (the
/// solder mask openings, a small part of the board): a bit per layer and
/// node marks them, and their values before the openings are kept apart.
#[derive(Clone, Debug)]
struct FillTrace {
    differs: Vec<Vec<u64>>,
    values: std::collections::HashMap<(u32, u32), u32>,
}

impl FillTrace {
    fn new(layers: usize, cells: usize) -> Self {
        Self {
            differs: vec![vec![0; cells.div_ceil(64)]; layers],
            values: std::collections::HashMap::new(),
        }
    }

    /// Records the value of a cell before an opening first changes it.
    fn remember(&mut self, layer: usize, index: usize, value: u32) {
        let word = &mut self.differs[layer][index / 64];
        if *word & (1 << (index % 64)) == 0 {
            *word |= 1 << (index % 64);
            self.values.insert((layer as u32, index as u32), value);
        }
    }

    /// Claims a remembered value for `net` too (copper the fill keeps
    /// away from as well as the tracks).
    fn claim(&mut self, layer: usize, index: usize, net: Option<NetId>) {
        if self.differs[layer][index / 64] & (1 << (index % 64)) != 0
            && let Some(value) = self.values.get_mut(&(layer as u32, index as u32))
        {
            claim(value, net);
        }
    }

    fn get(&self, layer: usize, index: usize) -> Option<u32> {
        (self.differs[layer][index / 64] & (1 << (index % 64)) != 0)
            .then(|| self.values[&(layer as u32, index as u32)])
    }

    fn bytes(&self) -> usize {
        self.differs
            .iter()
            .map(|layer| layer.capacity() * 8)
            .sum::<usize>()
            + self.values.capacity() * 16
    }
}

fn claim(cell: &mut u32, net: Option<NetId>) {
    let value = net.map_or(BLOCKED, owner);
    if *cell == FREE {
        *cell = value;
    } else if *cell != value {
        *cell = BLOCKED;
    }
}

fn claim16(cell: &mut u16, net: Option<NetId>) {
    let value = net.map_or(BLOCKED16, owner16);
    if *cell == 0 {
        *cell = value;
    } else if *cell != value {
        *cell = BLOCKED16;
    }
}

impl StaticMaps {
    /// `searched`: whether a search runs with this class (see the type).
    pub fn build(board: &Board, grid: &Grid, class: ClassId, searched: bool) -> Self {
        Self::build_with_rules(board, grid, class, searched, None)
    }

    /// `build` with the rules beyond the net classes (`crate::rules`):
    /// a pad, an existing track or via keeps what the rules ask of this
    /// class's tracks (per layer) and vias, and a via keeps the same-net
    /// hole-to-hole spacing from its own net's plated holes.
    pub fn build_with_rules(
        board: &Board,
        grid: &Grid,
        class: ClassId,
        searched: bool,
        net_rules: Option<&crate::rules::ClassRules>,
    ) -> Self {
        // Input-reachable only through a caller that skips the check
        // (`check_capacity`; the router's build checks it first).
        assert!(
            board.nets.len() <= MAX_NETS,
            "{} nets: the static maps name nets in 16 bits (check_capacity)",
            board.nets.len()
        );
        let rules = board.classes[class];
        let cells = grid.cells();
        let half_width = rules.trace_width / 2.0;
        let via_radius = rules.via_diameter / 2.0;
        let step = grid.pitch * std::f64::consts::SQRT_2;

        // What the build does, counted (`tests`, `vertex_tests`): the
        // outline's crossings per row, its edges' neighbourhoods, every
        // node tested against an obstacle and the edges each test measures.
        let outline_edges = polygon_edges(&board.outline).count() as u64;
        let (mut tests, mut vertex_tests) = (cells as u64, grid.ny as u64 * outline_edges);
        let mut tight_tests = 0u64;
        let mut inside = vec![false; cells];
        for y in 0..grid.ny {
            let line = grid.origin[1] + y as f64 * grid.pitch;
            let mut crossings: Vec<f64> = polygon_edges(&board.outline)
                .filter(|(a, b)| (a[1] > line) != (b[1] > line))
                .map(|(a, b)| a[0] + (line - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
                .collect();
            crossings.sort_by(f64::total_cmp);
            for span in crossings.as_chunks::<2>().0 {
                let x0 = ((span[0] - grid.origin[0]) / grid.pitch).ceil().max(0.0) as usize;
                let x1 = ((span[1] - grid.origin[0]) / grid.pitch).floor();
                if x1 < 0.0 {
                    continue;
                }
                for x in x0..=(x1 as usize).min(grid.nx - 1) {
                    inside[grid.index(x, y)] = true;
                }
            }
        }

        let mut base_trace: Vec<u16> = inside
            .iter()
            .map(|inside| if *inside { FREE16 } else { BLOCKED16 })
            .collect();
        let mut via_blocked: Vec<bool> = inside.iter().map(|inside| !inside).collect();
        let mut via_owner: Vec<u16> = if searched { vec![0; cells] } else { Vec::new() };
        let trace_edge = half_width + board.edge_clearance + SAFETY;
        let via_edge = via_radius + board.edge_clearance + SAFETY;
        // A unit step between two legal nodes can cut an outline corner by
        // at most the sagitta of that chord.
        let edge_margin = step * step / (8.0 * trace_edge.max(grid.pitch));
        for (a, b) in polygon_edges(&board.outline) {
            let reach = trace_edge.max(via_edge) + edge_margin;
            let bounds = Aabb {
                minimum: [a[0].min(b[0]), a[1].min(b[1])],
                maximum: [a[0].max(b[0]), a[1].max(b[1])],
            }
            .inflated(reach);
            let Some((x0, y0, x1, y1)) = grid.node_range(bounds) else {
                continue;
            };
            let nodes = ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;
            tests += nodes;
            vertex_tests += nodes;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let distance = crate::geometry::point_segment_distance(grid.center(x, y), a, b);
                    let index = grid.index(x, y);
                    if distance < trace_edge + edge_margin {
                        base_trace[index] = BLOCKED16;
                    }
                    if distance < via_edge {
                        via_blocked[index] = true;
                    }
                }
            }
        }

        // A pad may straddle the outline (card-edge fingers, connector
        // shells) but a track centreline on it still keeps the edge
        // clearance: pad cells inside the edge zone stay blocked.
        let edge_zone: Vec<bool> = base_trace.iter().map(|value| *value == BLOCKED16).collect();
        let mut trace = vec![base_trace; board.layer_count];
        let mut edge_block = vec![vec![0u8; cells]; board.layer_count];
        let mut edge_owner = vec![vec![0u16; cells]; board.layer_count];

        // Solder mask openings keep tracks out but not a pour's fill: they
        // come last, and the trace map before them is the pours' (MokyaLora:
        // a motor's shield pad under a mask opening looked cut off from its
        // ground pour, which KiCad fills there).
        let mask_opening = |obstacle: &&crate::board::Obstacle| {
            obstacle.kind == ObstacleKind::Keepout
                && obstacle.label.starts_with("solder mask opening")
        };
        // So do the rule areas that keep tracks out but let pours in
        // (`Board::fill_passes`: a plane layer's `no tracks` area), which
        // hid the plane's fill from the pour model before 2026-10-21.
        let passes = |index: usize, obstacle: &&crate::board::Obstacle| {
            mask_opening(obstacle) || board.fill_passes.binary_search(&index).is_ok()
        };
        // A pad's own clearance override replaces the zone's clearance in
        // KiCad's fill also where it is smaller than this class's (which
        // the tracks keep): such pads come after the others, and the fill
        // keeps only the override from them (`Board::fill_clearance`).
        let overridden = |obstacle: &&crate::board::Obstacle| {
            obstacle.kind == ObstacleKind::Copper
                && !mask_opening(obstacle)
                && board.fill_clearance(&rules, obstacle)
                    < board.copper_clearance(&rules, obstacle) - 1.0e-9
        };
        let ordered: Vec<(usize, &crate::board::Obstacle)> = board
            .obstacles
            .iter()
            .enumerate()
            .filter(|(index, obstacle)| !passes(*index, obstacle) && !overridden(obstacle))
            .chain(
                board
                    .obstacles
                    .iter()
                    .enumerate()
                    .filter(|(_, obstacle)| overridden(obstacle)),
            )
            .chain(
                board
                    .obstacles
                    .iter()
                    .enumerate()
                    .filter(|(index, obstacle)| passes(*index, obstacle)),
            )
            .collect();
        let first_apart = ordered
            .iter()
            .position(|(index, obstacle)| passes(*index, obstacle) || overridden(obstacle));
        let mut fill_trace: Option<FillTrace> = None;
        for (position, (obstacle_index, obstacle)) in ordered.into_iter().enumerate() {
            if Some(position) == first_apart {
                fill_trace = Some(FillTrace::new(board.layer_count, cells));
            }
            // From the first pad apart on: where the fill keeps away too
            // (within the fill's reach of an overridden pad; any other
            // copper), a remembered value is claimed as well.
            let opening = mask_opening(&obstacle);
            let fill_reach = (obstacle.kind == ObstacleKind::Copper && !opening)
                .then(|| half_width + board.fill_clearance(&rules, obstacle) + SAFETY);
            let (trace_reach, via_reach) = match obstacle.kind {
                ObstacleKind::Copper => {
                    let clearance = board.copper_clearance(&rules, obstacle);
                    // Copper keeps the hole clearance from a via's drill too.
                    (
                        half_width + clearance,
                        (via_radius + clearance).max(rules.via_drill / 2.0 + board.hole_clearance),
                    )
                }
                ObstacleKind::Keepout => (
                    half_width + obstacle.clearance,
                    via_radius + obstacle.clearance,
                ),
                ObstacleKind::Hole => {
                    let clearance = board.hole_clearance.max(obstacle.clearance);
                    (
                        half_width + clearance,
                        (via_radius + clearance).max(rules.via_drill / 2.0 + board.hole_to_hole),
                    )
                }
            };
            let trace_reach = trace_reach + SAFETY;
            let via_reach = via_reach + SAFETY;
            // The rules beyond the classes: per layer for tracks, for vias
            // the largest over the obstacle's layers.
            let mut ruled_trace: Option<Vec<f64>> = None;
            let mut via_reach = via_reach;
            if let Some(split) = net_rules
                && obstacle.kind == ObstacleKind::Copper
            {
                let per_layer: Vec<f64> = (0..board.layer_count)
                    .map(|layer| {
                        (half_width
                            + split.obstacle_extra(class, crate::rules::TRACK, obstacle, layer)
                            + SAFETY)
                            .max(trace_reach)
                    })
                    .collect();
                if per_layer.iter().any(|reach| *reach > trace_reach) {
                    ruled_trace = Some(per_layer);
                }
                via_reach = via_reach.max(
                    via_radius
                        + split.obstacle_extra_via(class, obstacle, board.layer_count)
                        + SAFETY,
                );
            }
            let trace_reach = ruled_trace.as_ref().map_or(trace_reach, |per_layer| {
                per_layer.iter().copied().fold(trace_reach, f64::max)
            });
            // A via of the hole's own net keeps the same-net hole-to-hole
            // spacing from its drill (KiCad's hole_to_hole holds within a
            // net: nuclone's VSS via 0.464 mm from J3's VSS pad hole).
            let same_net_hole = net_rules
                .map_or(0.0, |split| split.resolved.same_net_hole_to_hole)
                .max(0.0);
            let own_hole_reach = rules.via_drill / 2.0 + same_net_hole + SAFETY;
            let margin = step * step / (8.0 * trace_reach);
            let bounds = obstacle.shape.aabb();

            // A polygon of many corners (a board cutout, a rule area) is
            // measured once per node from its edges' neighbourhoods
            // (`PolygonField`), not edge by edge for every node of its box
            // (ULK's 454-corner cutout: 15 s a class, 60-190 s a lattice).
            let field = match &obstacle.shape {
                Shape::Polygon { points } if points.len() >= FIELD_CORNERS => {
                    let via_range = obstacle
                        .blocks_vias
                        .then(|| grid.node_range(bounds.inflated(via_reach)))
                        .flatten();
                    let trace_range = obstacle
                        .blocks_tracks
                        .then(|| grid.node_range(bounds.inflated(trace_reach + margin)))
                        .flatten();
                    [via_range, trace_range]
                        .into_iter()
                        .flatten()
                        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
                        .map(|range| {
                            let field = PolygonField::new(
                                grid,
                                points,
                                range,
                                via_reach.max(trace_reach + margin),
                            );
                            tests += field.distances.len() as u64;
                            vertex_tests += field.measured;
                            field
                        })
                }
                _ => None,
            };
            let corners = if field.is_some() {
                0
            } else {
                shape_corners(&obstacle.shape)
            };
            let mut tight = 0u64;
            let distance_at = |x: usize, y: usize, center: Point| match &field {
                Some(field) => field.distance(x, y),
                None => obstacle.shape.distance_to_point(center),
            };
            if obstacle.blocks_vias
                && let Some((x0, y0, x1, y1)) = grid.node_range(bounds.inflated(via_reach))
            {
                // A pad no via may overlap (`Board::no_via_pads`: an SMD
                // pad) holds no via of its own net either, but that net's
                // vias may touch it from outside: beside it they need no
                // clearance from their own pad.
                let no_via = board.no_via_pads.binary_search(&obstacle_index).is_ok();
                let beside = obstacle.net.filter(|_| no_via);
                // Another surface pad of a net may hold that net's own vias
                // where the whole via lies inside its copper.
                let own_vias = obstacle.net.filter(|_| {
                    !no_via
                        && obstacle.kind == ObstacleKind::Copper
                        && obstacle.layers.count_ones() == 1
                });
                // A routed pad's plated hole keeps other nets' vias at the
                // hole clearance from the drill (KiCad measures a via's
                // copper to the hole: jetson's 0.5 via 0.244 from J2's
                // 0.55 drill in 0.75 copper, 0.25 required). Its own net
                // is exempt, as in KiCad (tracks end on the drill), and
                // keeps the pad copper's rule.
                let own_hole = obstacle.net.filter(|_| obstacle.kind == ObstacleKind::Hole);
                // A region where a pour stays whole (`keep whole <net>
                // <layer>`) keeps other nets' vias out; its net's pass.
                let keep_whole = obstacle.net.filter(|_| {
                    obstacle.kind == ObstacleKind::Keepout
                        && obstacle.label.starts_with("keep whole ")
                });
                let nodes = ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;
                tests += nodes;
                vertex_tests += nodes * corners;
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let center = grid.center(x, y);
                        let distance = distance_at(x, y, center);
                        if distance < via_reach {
                            let index = grid.index(x, y);
                            if searched {
                                via_owner[index] = match (own_vias, own_hole) {
                                    // A node an earlier obstacle left to the
                                    // net (its pin's `beside` arm) stays the
                                    // net's: the map does not depend on
                                    // whether a QFN's pins come before its
                                    // exposed pad.
                                    (Some(net), _)
                                        if (!via_blocked[index]
                                            || via_owner[index] == owner16(net))
                                            && obstacle
                                                .shape
                                                .contains_disc(center, via_radius + SAFETY) =>
                                    {
                                        owner16(net)
                                    }
                                    (_, Some(net))
                                        if (!via_blocked[index]
                                            || via_owner[index] == owner16(net))
                                            && !(same_net_hole > 0.0
                                                && distance < own_hole_reach) =>
                                    {
                                        owner16(net)
                                    }
                                    _ => match (beside, keep_whole) {
                                        (Some(net), _)
                                            if (!via_blocked[index]
                                                || via_owner[index] == owner16(net))
                                                && distance >= via_radius + SAFETY =>
                                        {
                                            owner16(net)
                                        }
                                        (None, Some(net))
                                            if !via_blocked[index]
                                                || via_owner[index] == owner16(net) =>
                                        {
                                            owner16(net)
                                        }
                                        _ => 0,
                                    },
                                };
                            }
                            via_blocked[index] = true;
                        }
                    }
                }
            }
            if !obstacle.blocks_tracks {
                continue;
            }
            let Some((x0, y0, x1, y1)) = grid.node_range(bounds.inflated(trace_reach + margin))
            else {
                continue;
            };
            for layer in 0..board.layer_count {
                if obstacle.layers & (1 << layer) == 0 {
                    continue;
                }
                let (trace_reach, margin) = match &ruled_trace {
                    None => (trace_reach, margin),
                    Some(per_layer) => (per_layer[layer], step * step / (8.0 * per_layer[layer])),
                };
                let nodes = ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;
                tests += nodes;
                vertex_tests += nodes * corners;
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let index = grid.index(x, y);
                        let center = grid.center(x, y);
                        let distance = distance_at(x, y, center);
                        if distance < trace_reach {
                            if !edge_zone[index] {
                                if let Some(fill) = &mut fill_trace {
                                    match fill_reach {
                                        Some(reach) if distance < reach => {
                                            fill.claim(layer, index, obstacle.net)
                                        }
                                        _ => fill.remember(
                                            layer,
                                            index,
                                            widen16(trace[layer][index]),
                                        ),
                                    }
                                }
                                claim16(&mut trace[layer][index], obstacle.net);
                            }
                        } else if distance < trace_reach + margin {
                            tight += 1;
                            mark_tight_edges(
                                grid,
                                &obstacle.shape,
                                obstacle.net,
                                trace_reach,
                                index,
                                &mut edge_block[layer],
                                &mut edge_owner[layer],
                            );
                        }
                    }
                }
            }
            // A tight node measures its eight steps against the shape.
            tight_tests += tight * 8 * shape_corners(&obstacle.shape);
        }
        // Unchanged cells need no record (an opening that claimed nothing).
        if let Some(fill) = &mut fill_trace {
            let values = std::mem::take(&mut fill.values);
            for ((layer, index), value) in values {
                if widen16(trace[layer as usize][index as usize]) == value {
                    fill.differs[layer as usize][index as usize / 64] &= !(1 << (index % 64));
                } else {
                    fill.values.insert((layer, index), value);
                }
            }
            fill.values.shrink_to_fit();
        }
        assert!(
            board.layer_count <= MAX_LAYERS,
            "{} copper layers: free_layers holds 32 (check_capacity)",
            board.layer_count
        );
        let mut free_layers = vec![0u32; if searched { cells } else { 0 }];
        let via_blocked = NodeBits::from_bools(&via_blocked);
        if searched {
            for (layer, map) in trace.iter().enumerate() {
                for (cell, value) in map.iter().enumerate() {
                    if *value == FREE16 {
                        free_layers[cell] |= 1 << layer;
                    }
                }
            }
        }
        Self {
            trace,
            edge_block,
            edge_owner,
            via_blocked,
            via_owner,
            free_layers: LayerBits::new(&free_layers, board.layer_count),
            fill_trace,
            tests,
            vertex_tests,
            tight_tests,
        }
    }

    /// Whether `net` may put a via on this node.
    pub fn via_allowed(&self, index: usize, net: NetId) -> bool {
        !self.via_blocked.get(index) || self.via_owner[index] == owner16(net)
    }

    /// Whether `net` may have a trace centreline on this node.
    pub fn trace_allowed(&self, layer: usize, index: usize, net: NetId) -> bool {
        let value = self.trace[layer][index];
        value == FREE16 || value == owner16(net)
    }

    /// `trace` at a node as `FREE`, `BLOCKED` or an `owner`.
    #[inline(always)]
    pub fn trace_at(&self, layer: usize, index: usize) -> u32 {
        widen16(self.trace[layer][index])
    }

    /// Whether a pour of `net` fills this node (mask openings do not stop
    /// a fill).
    pub fn fill_allowed(&self, layer: usize, index: usize, net: NetId) -> bool {
        let value = self
            .fill_trace
            .as_ref()
            .and_then(|fill| fill.get(layer, index))
            .unwrap_or_else(|| self.trace_at(layer, index));
        value == FREE || value == owner(net)
    }

    /// Whether `net` may take the unit step leaving `index` in `direction`.
    pub fn edge_allowed(&self, layer: usize, index: usize, direction: usize, net: NetId) -> bool {
        self.edge_block[layer][index] & (1 << direction) == 0
            || self.edge_owner[layer][index] == owner16(net)
    }

    /// Bytes held by the per-node maps.
    pub fn bytes(&self) -> usize {
        fn of<T>(maps: &[Vec<T>]) -> usize {
            maps.iter()
                .map(|map| map.capacity() * std::mem::size_of::<T>())
                .sum()
        }
        of(&self.trace)
            + of(&self.edge_block)
            + of(&self.edge_owner)
            + self.via_blocked.bytes()
            + self.via_owner.capacity() * 2
            + self.free_layers.bytes()
            + self.fill_trace.as_ref().map_or(0, FillTrace::bytes)
    }
}

/// Corners from which a polygon obstacle's distances come from a
/// `PolygonField`.
const FIELD_CORNERS: usize = 16;

/// The edges a distance test of `shape` goes through (1 for a circle or a
/// capsule): what one node's test against it costs, counted.
pub(crate) fn shape_corners(shape: &Shape) -> u64 {
    match shape {
        Shape::Polygon { points } => points.len() as u64,
        Shape::Union { parts } => parts.iter().map(shape_corners).sum(),
        _ => 1,
    }
}

/// The distances from the nodes of an inclusive node range to a filled
/// polygon, equal to `Shape::distance_to_point` bit for bit wherever that is
/// below `cap` (the same edge distances, the same crossing test as
/// `point_in_polygon`), and at least `cap` elsewhere. Each edge updates only
/// the nodes within `cap` of its box, and the inside comes from each row's
/// sorted crossings: the cost follows the edges' neighbourhoods and the
/// rows, not the box's nodes times the corners.
struct PolygonField {
    x0: usize,
    y0: usize,
    width: usize,
    distances: Vec<f64>,
    /// The edge distances and crossings it computed (the build's count).
    measured: u64,
}

impl PolygonField {
    fn new(grid: &Grid, points: &[Point], range: (usize, usize, usize, usize), cap: f64) -> Self {
        let (x0, y0, x1, y1) = range;
        let width = x1 - x0 + 1;
        let mut distances = vec![f64::INFINITY; width * (y1 - y0 + 1)];
        let mut measured = ((y1 - y0 + 1) * points.len()) as u64;
        for (a, b) in polygon_edges(points) {
            // One node more than the cap: a node just inside it by rounding
            // is measured too.
            let near = Aabb {
                minimum: [a[0].min(b[0]), a[1].min(b[1])],
                maximum: [a[0].max(b[0]), a[1].max(b[1])],
            }
            .inflated(cap + grid.pitch);
            let Some((ex0, ey0, ex1, ey1)) = grid.node_range(near) else {
                continue;
            };
            let (ex0, ey0, ex1, ey1) = (ex0.max(x0), ey0.max(y0), ex1.min(x1), ey1.min(y1));
            measured += ((ex1 + 1).saturating_sub(ex0) * (ey1 + 1).saturating_sub(ey0)) as u64;
            for y in ey0..=ey1 {
                for x in ex0..=ex1 {
                    let distance = crate::geometry::point_segment_distance(grid.center(x, y), a, b);
                    let slot = &mut distances[(y - y0) * width + x - x0];
                    if distance < *slot {
                        *slot = distance;
                    }
                }
            }
        }
        let mut crossings = Vec::new();
        for y in y0..=y1 {
            let line = grid.center(x0, y)[1];
            crossings.clear();
            crossings.extend(
                polygon_edges(points)
                    .filter(|(a, b)| (a[1] > line) != (b[1] > line))
                    .map(|(a, b)| a[0] + (line - a[1]) / (b[1] - a[1]) * (b[0] - a[0])),
            );
            crossings.sort_by(f64::total_cmp);
            for x in x0..=x1 {
                let point = grid.center(x, y)[0];
                // `point_in_polygon`: inside where an odd number of
                // crossings lie beyond the point.
                // Not `crossing <= point`: a NaN crossing counts as not
                // beyond, as in `point_in_polygon`.
                #[allow(clippy::neg_cmp_op_on_partial_ord)]
                let beyond =
                    crossings.len() - crossings.partition_point(|crossing| !(point < *crossing));
                if beyond % 2 == 1 {
                    distances[(y - y0) * width + x - x0] = 0.0;
                }
            }
        }
        Self {
            x0,
            y0,
            width,
            distances,
            measured,
        }
    }

    fn distance(&self, x: usize, y: usize) -> f64 {
        self.distances[(y - self.y0) * self.width + x - self.x0]
    }
}

fn mark_tight_edges(
    grid: &Grid,
    shape: &Shape,
    net: Option<NetId>,
    reach: f64,
    index: usize,
    edge_block: &mut [u8],
    edge_owner: &mut [u16],
) {
    let center = grid.center_of(index);
    for direction in 0..8 {
        let Some(neighbor) = grid.neighbor(index, direction) else {
            continue;
        };
        if shape.distance_to_segment(center, grid.center_of(neighbor)) < reach {
            edge_block[index] |= 1 << direction;
            claim16(&mut edge_owner[index], net);
            edge_block[neighbor] |= 1 << ((direction + 4) % 8);
            claim16(&mut edge_owner[neighbor], net);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A board of `nets` empty nets on `layers` layers.
    fn board_of(nets: usize, layers: usize) -> Board {
        Board {
            layer_count: layers,
            outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            edge_clearance: 0.2,
            hole_clearance: 0.2,
            hole_to_hole: 0.2,
            classes: Vec::new(),
            neck_width: 0.1,
            obstacles: Vec::new(),
            nets: (0..nets)
                .map(|net| crate::board::Net {
                    name: format!("n{net}"),
                    class: 0,
                    terminals: Vec::new(),
                })
                .collect(),
            planes: Vec::new(),
            solid_pads: Vec::new(),
            isolated_pads: Vec::new(),
            solid_pad_obstacles: Vec::new(),
            isolated_pad_obstacles: Vec::new(),
            pad_spokes: Default::default(),
            merged_pads: Vec::new(),
            no_via_pads: Vec::new(),
            fill_passes: Vec::new(),
        }
    }

    #[test]
    fn a_lattice_is_sized_and_checked_before_any_map_is_allocated() {
        // 10 x 10 mm at 0.1 mm: 101 x 101 nodes, a floor of 2 layers x
        // (32 + 6) bytes for one class.
        let mut board = board_of(2, 2);
        let size = LatticeSize::of(&board, 0.1);
        assert_eq!(size.nodes, [101.0, 101.0]);
        assert_eq!(size.maps_bytes_floor(2, 1), 101.0 * 101.0 * 2.0 * 38.0);
        assert!(size.check().is_ok());
        let grid = Grid::choose(&board, &[0.1]);
        assert!(grid.check_size().is_ok());
        assert!(grid.nx >= 101 && grid.nx <= 102);
        // An infinite or NaN coordinate (until 2026-10-21 `usize::MAX + 1`):
        // refused, the choice saturated, nothing allocated.
        for bad in [f64::INFINITY, f64::NAN] {
            board.outline[1][0] = bad;
            assert!(LatticeSize::of(&board, 0.1).check().is_err(), "{bad}");
            let grid = Grid::choose(&board, &[0.1]);
            if bad.is_infinite() {
                assert!(grid.check_size().is_err());
                let routed = crate::router::Router::try_new(&board, &crate::Config::default());
                assert!(routed.is_err());
            }
        }
        // A 10 m board at 0.1 mm: 10^10 nodes a layer, past 32 bits.
        board.outline = vec![[0.0, 0.0], [10_000.0, 0.0], [10_000.0, 10_000.0]];
        let error = LatticeSize::of(&board, 0.1).check().unwrap_err();
        assert!(
            error.contains("more than the router's maps index"),
            "{error}"
        );
        assert!(Grid::choose(&board, &[0.1]).check_size().is_err());
    }

    #[test]
    fn a_board_beyond_the_maps_capacity_is_an_error_naming_the_limit_and_the_count() {
        assert_eq!(check_capacity(&board_of(MAX_NETS, MAX_LAYERS)), Ok(()));
        // 16 bits: up to 65 533 nets (until 2026-10-21 the router asserted
        // fewer than 32 767).
        assert!(check_capacity(&board_of(40_000, 4)).is_ok());
        let nets = check_capacity(&board_of(MAX_NETS + 1, 4)).unwrap_err();
        assert!(
            nets.contains("65534 nets") && nets.contains("at most 65533"),
            "{nets}"
        );
        let layers = check_capacity(&board_of(2, 33)).unwrap_err();
        assert!(
            layers.contains("33 copper layers") && layers.contains("at most 32"),
            "{layers}"
        );
        let routed = crate::router::Router::try_new(&board_of(2, 33), &crate::Config::default());
        assert_eq!(routed.err(), Some(layers));
    }

    /// A concave polygon of many corners (a ring of `count` points whose
    /// radius alternates, like a cutout with notches) around `center`.
    fn star(center: Point, count: usize, outer: f64, inner: f64) -> Vec<Point> {
        (0..count)
            .map(|index| {
                let angle = index as f64 / count as f64 * std::f64::consts::TAU;
                let radius = if index % 2 == 0 { outer } else { inner };
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]
            })
            .collect()
    }

    #[test]
    fn polygon_field_matches_distance_to_point_below_its_cap() {
        let grid = Grid {
            origin: [0.0375, -0.0125],
            pitch: 0.1,
            nx: 140,
            ny: 120,
        };
        for (points, cap) in [
            (star([7.0, 6.0], 64, 5.0, 3.5), 0.45),
            (star([6.93, 5.97], 200, 4.0, 3.9), 0.3),
            // A cutout whose corners sit on lattice rows and columns.
            (
                vec![
                    [2.0375, 2.0875],
                    [11.0375, 2.0875],
                    [11.0375, 9.0875],
                    [6.0375, 4.0875],
                    [2.0375, 9.0875],
                ],
                0.25,
            ),
        ] {
            let shape = Shape::Polygon {
                points: points.clone(),
            };
            let range = grid
                .node_range(shape.aabb().inflated(cap))
                .expect("on the lattice");
            let field = PolygonField::new(&grid, &points, range, cap);
            let (x0, y0, x1, y1) = range;
            let mut inside = 0;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let exact = shape.distance_to_point(grid.center(x, y));
                    let measured = field.distance(x, y);
                    if exact < cap {
                        assert_eq!(measured.to_bits(), exact.to_bits(), "node {x}, {y}");
                    } else {
                        assert!(measured >= cap, "node {x}, {y}: {measured} for {exact}");
                    }
                    inside += usize::from(exact == 0.0);
                }
            }
            assert!(inside > 100, "the polygon covers nodes");
        }
    }
}

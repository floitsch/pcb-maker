// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Electrostatic global placement after ePlace/RePlAce.
//!
//! Component area is charge. The potential solves Poisson's equation on the
//! board (Neumann boundary, spectral solution), so spreading pressure acts at
//! a distance and points towards free space instead of only between touching
//! neighbours. Wirelength is the smooth weighted-average model on true pin
//! positions. Nesterov's method minimizes `wirelength + lambda * density`
//! while lambda ramps up until bodies no longer overlap. Fixed parts and
//! everything outside the outline are immovable charge. On a PCB every part
//! is large compared to a density bin, so bodies are mutually exclusive at
//! density 1; routing room comes from per-part halos, and filler charges
//! decide how much of the remaining whitespace stays between the parts.

use crate::constraints;
use crate::problem::{Point, Pose, Problem, point_in_polygon, rotate};
use std::collections::BTreeMap;

/// The netlist start's share of the common start's noise
/// (`initial::quadratic_start`).
const NETLIST_NOISE: f64 = 0.1;

/// What the global placement does with the netlist's stacks
/// (`spread_stacks`; docs/placer.md, "The netlist's stacks are lost in the
/// descent, not at the start (2026-11-08)" and "Banks on a wide bus glued
/// through the descent (2026-11-08)").
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stacks {
    /// Left on their spot (the start's noise separates them).
    Off,
    /// Spread into banks that move as one body in the descent: the parts
    /// with `least_nets` nets or more whose nets have `share` in common
    /// (`stacks::shared`) and `least_common` nets at least
    /// (`stacks::common`), in groups of `least_members` at least. A bank at
    /// the start only is undone by the descent where no wire holds its
    /// parts, and glued banks of capacitors on a rail and ground are taken
    /// apart by the anneal (stage 1 of 2026-11-08: -1.2 %, not taken);
    /// glued pairs only redraw the board (SNSP-CPU-01's two SRAMs end as
    /// neighbours either way).
    Glue {
        share: f64,
        least_nets: usize,
        least_common: usize,
        least_members: usize,
    },
}

impl Stacks {
    /// Banks on a wide parallel bus (the default): three parts or more with
    /// three nets or more that share nine tenths of their nets and 24 at
    /// least (a memory's address and data lines; video's eight SIMM
    /// sockets, 50 shared nets, and mackerel-08's seven SRAMs, 30).
    pub const BANKS: Stacks = Stacks::Glue {
        share: 0.9,
        least_nets: 3,
        least_common: 24,
        least_members: 3,
    };
}

/// Spreads the netlist's stacks (`crate::stacks`) at the solved `centers`
/// of the `movable` bodies into banks at their pitch around their
/// centroids: the parts that `rule` (`Stacks::Glue`) takes as alike and
/// whose bodies (halos included) overlap there. A bank's cell is the
/// largest body's extent with its halo on each axis; its shape is the one
/// that adds the least wire to the stack's nets (`stacks::bank_columns`),
/// the bank moved onto the board where it fits. `held` bodies stay.
/// Returns the stacks (body indices).
#[allow(clippy::too_many_arguments)]
fn spread_stacks(
    problem: &Problem,
    rule: Stacks,
    movable: &[usize],
    bodies: &[Body],
    offsets: &[Vec<(Point, usize)>],
    fixed_pins: &[(Point, usize)],
    held: &[bool],
    centers: &mut [Point],
) -> Vec<Vec<usize>> {
    let Stacks::Glue {
        share: least_share,
        least_nets,
        least_common,
        least_members,
    } = rule
    else {
        return Vec::new();
    };
    let count = movable.len();
    let nets: Vec<Vec<usize>> = offsets
        .iter()
        .map(|pins| pins.iter().map(|(_, net)| *net).collect())
        .collect();
    let mut keys = crate::stacks::net_keys(&nets, least_nets);
    for (body, key) in keys.iter_mut().enumerate() {
        if held[body] {
            *key = None;
        }
    }
    let half: Vec<Point> = bodies.iter().map(|body| body.half).collect();
    let field: Vec<usize> = bodies.iter().map(|body| body.field).collect();
    let stacks = crate::stacks::find_stacks(
        centers,
        &half,
        &field,
        &keys,
        least_share,
        least_common,
        least_members,
    );
    if stacks.is_empty() {
        return Vec::new();
    }
    let bounds = problem.bounds();
    let margin = problem.edge_margin;
    let low = [bounds[0] + margin, bounds[1] + margin];
    let high = [bounds[2] - margin, bounds[3] - margin];
    let room = [(high[0] - low[0]).max(0.0), (high[1] - low[1]).max(0.0)];
    // Each net's pins: the movable body owning it (or none) and where it
    // is at the solve.
    let mut pins_of: Vec<Vec<(Option<usize>, Point)>> = vec![Vec::new(); problem.net_weights.len()];
    for (at, net) in fixed_pins {
        pins_of[*net].push((None, *at));
    }
    for (body, pins) in offsets.iter().enumerate() {
        for (offset, net) in pins {
            pins_of[*net].push((
                Some(body),
                [centers[body][0] + offset[0], centers[body][1] + offset[1]],
            ));
        }
    }
    for members in &stacks {
        // The nets every member has, and the extent of each without the
        // stack.
        let common: Vec<usize> = keys[members[0]]
            .iter()
            .flatten()
            .copied()
            .filter(|net| {
                members.iter().all(|body| {
                    keys[*body]
                        .as_ref()
                        .is_some_and(|key| key.binary_search(net).is_ok())
                })
            })
            .collect();
        let spans: Vec<(f64, Point)> = common
            .iter()
            .map(|net| {
                let mut extent = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for (owner, at) in &pins_of[*net] {
                    if owner.is_some_and(|owner| members.binary_search(&owner).is_ok()) {
                        continue;
                    }
                    extent = [
                        extent[0].min(at[0]),
                        extent[1].min(at[1]),
                        extent[2].max(at[0]),
                        extent[3].max(at[1]),
                    ];
                }
                let span = if extent[0] <= extent[2] {
                    [extent[2] - extent[0], extent[3] - extent[1]]
                } else {
                    [0.0, 0.0]
                };
                (problem.net_weights[*net].max(0.0), span)
            })
            .collect();
        let cell = members.iter().fold([0.0f64; 2], |cell, body| {
            [
                cell[0].max(2.0 * half[*body][0]),
                cell[1].max(2.0 * half[*body][1]),
            ]
        });
        let columns = crate::stacks::bank_columns(members.len(), cell, &spans, room);
        let rows = members.len().div_ceil(columns);
        let mut centroid = [0.0; 2];
        for body in members {
            centroid[0] += centers[*body][0] / members.len() as f64;
            centroid[1] += centers[*body][1] / members.len() as f64;
        }
        // On the board where the bank fits.
        let extent = [columns as f64 * cell[0], rows as f64 * cell[1]];
        for axis in 0..2 {
            if extent[axis] <= room[axis] {
                centroid[axis] = centroid[axis].clamp(
                    low[axis] + extent[axis] / 2.0,
                    high[axis] - extent[axis] / 2.0,
                );
            }
        }
        // The members fill the cells row by row in the order of their
        // solved centres (by y, then x), the index breaking ties.
        let mut order = members.clone();
        order.sort_by(|a, b| {
            centers[*a][1]
                .total_cmp(&centers[*b][1])
                .then(centers[*a][0].total_cmp(&centers[*b][0]))
                .then(a.cmp(b))
        });
        let cells = crate::stacks::bank_centers(members.len(), cell, columns, centroid);
        for (body, at) in order.iter().zip(cells) {
            centers[*body] = at;
        }
    }
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        let said: Vec<String> = stacks
            .iter()
            .map(|members| {
                let names: Vec<&str> = members
                    .iter()
                    .take(4)
                    .map(|body| problem.components[movable[*body]].name.as_str())
                    .collect();
                format!(
                    "{} [{}{}]",
                    members.len(),
                    names.join(" "),
                    if members.len() > 4 { " ..." } else { "" }
                )
            })
            .collect();
        eprintln!(
            "placer: netlist stacks banked ({count} movable): {}",
            said.join("; ")
        );
    }
    stacks
}

/// A pin on a net: its position and, for a movable body's pin, the body and
/// the pin's index on it (`None` for a fixed pin).
type NetPin = (Point, Option<(usize, usize)>);

#[derive(Clone, Debug)]
pub struct GlobalConfig {
    /// Bins per side of the density grid the charge is spread on.
    pub bins: usize,
    /// Fraction of the whitespace (free area not needed by bodies and their
    /// halos) that is occupied by filler charge. 1 lets wirelength pull the
    /// parts into a compact cluster; 0 spreads them over the whole board.
    pub whitespace_fill: f64,
    /// The density overflow at which the global placement stops (after
    /// its first 20 iterations).
    pub stop_overflow: f64,
    /// Iterations of the global placement at most.
    pub max_iterations: usize,
    /// The global placement stops where the overflow has not fallen below
    /// `stall_factor` times its least so far for this many iterations (0,
    /// the default: never): the density has settled where it can. Video's
    /// memory modules held the overflow at 0.07 to 0.12 from iteration 200
    /// to the cap of 1500 while the density weight grew by 10^15, which
    /// left the wires no say (docs/placer.md, "A race over skeletons").
    /// On under the race over skeletons (`skeleton::place_skeleton`).
    pub stall_iterations: usize,
    /// The factor on the least overflow so far that a new overflow must
    /// fall below to count as progress (`stall_iterations`).
    pub stall_factor: f64,
    /// Rotations never turn a body to an angle at which it is longer than
    /// the board on either axis while another angle fits (off by default;
    /// on under the race over skeletons, with `stall_iterations`).
    pub fit_turns: bool,
    /// Growth of the density weight per iteration.
    pub lambda_growth: f64,
    /// Iterations between discrete rotation updates.
    pub rotation_interval: usize,
    /// Nets with more pins are ignored when choosing rotations.
    pub rotation_net_limit: usize,
    /// Every this many iterations the poses are kept as a frame (for
    /// animations); 0 keeps none but the last.
    pub frame_interval: usize,
    /// Seed of the start's noise: the parts start near the fixed parts'
    /// pin centroid (or the board's centre), the fillers anywhere.
    pub seed: u64,
    /// A diagnostic warm start (`crate::Start`): per component, whether
    /// it starts at its pose's centre without the start's noise. `None`:
    /// every part from the start point.
    pub warm: Option<std::sync::Arc<Vec<bool>>>,
    /// What the global placement does with the netlist's stacks
    /// (`Stacks`; by default `Stacks::BANKS`).
    pub stacks: Stacks,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            bins: 64,
            whitespace_fill: 0.0,
            stop_overflow: 0.04,
            max_iterations: 1500,
            stall_iterations: 0,
            stall_factor: 0.98,
            fit_turns: false,
            lambda_growth: 1.03,
            rotation_interval: 15,
            rotation_net_limit: 24,
            frame_interval: 5,
            seed: 1,
            warm: None,
            stacks: Stacks::BANKS,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub iteration: usize,
    pub overflow: f64,
    pub wirelength: f64,
    pub poses: Vec<Pose>,
}

struct Random(u64);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A movable charge: a real component or a filler.
struct Body {
    /// Index into the problem's components; `None` for fillers.
    component: Option<usize>,
    half: Point,
    /// Half extent of the body alone (without halo), and its angle, for
    /// constraints.
    bare: Point,
    angle: f64,
    pins: f64,
    /// Which side's density field the body charges (0: front, 1: back on
    /// a two-sided board).
    field: usize,
    /// Scales the half extent for density: a hollow part charges only the
    /// area its blocking boxes take.
    density: f64,
    /// Escape demand (`Problem::escape`): the charge grows on each side
    /// (-x, +x, -y, +y) by this much, mm.
    extra: [f64; 4],
}

impl Body {
    fn charge_half(&self) -> Point {
        [self.half[0] * self.density, self.half[1] * self.density]
    }

    /// The charge's centre and half extent with the escape demand's room.
    fn charge_box(&self, center: Point) -> (Point, Point) {
        let half = self.charge_half();
        let extra = self.extra;
        (
            [
                center[0] + (extra[1] - extra[0]) / 2.0,
                center[1] + (extra[3] - extra[2]) / 2.0,
            ],
            [
                half[0] + (extra[0] + extra[1]) / 2.0,
                half[1] + (extra[2] + extra[3]) / 2.0,
            ],
        )
    }
}

struct Field {
    bins: usize,
    origin: Point,
    bin: Point,
    cosine: Vec<f64>,
    /// `cosine` transposed (index = x * n + u).
    cosine_t: Vec<f64>,
    /// The sine basis times its frequency, along x and along y.
    sine_x: Vec<f64>,
    sine_y: Vec<f64>,
    frequency_x: Vec<f64>,
    frequency_y: Vec<f64>,
    fixed: Vec<f64>,
    field_x: Vec<f64>,
    field_y: Vec<f64>,
}

impl Field {
    fn new(bins: usize, bounds: [f64; 4]) -> Self {
        let mut cosine = vec![0.0; bins * bins];
        let mut sine = vec![0.0; bins * bins];
        for u in 0..bins {
            for x in 0..bins {
                let phase = std::f64::consts::PI * u as f64 * (x as f64 + 0.5) / bins as f64;
                cosine[u * bins + x] = phase.cos();
                sine[u * bins + x] = phase.sin();
            }
        }
        let size = [bounds[2] - bounds[0], bounds[3] - bounds[1]];
        let frequency_x: Vec<f64> = (0..bins)
            .map(|u| std::f64::consts::PI * u as f64 / size[0])
            .collect();
        let frequency_y: Vec<f64> = (0..bins)
            .map(|v| std::f64::consts::PI * v as f64 / size[1])
            .collect();
        let mut cosine_t = vec![0.0; bins * bins];
        let mut sine_x = vec![0.0; bins * bins];
        let mut sine_y = vec![0.0; bins * bins];
        for u in 0..bins {
            for x in 0..bins {
                cosine_t[x * bins + u] = cosine[u * bins + x];
                sine_x[u * bins + x] = frequency_x[u] * sine[u * bins + x];
                sine_y[u * bins + x] = frequency_y[u] * sine[u * bins + x];
            }
        }
        Self {
            bins,
            origin: [bounds[0], bounds[1]],
            bin: [size[0] / bins as f64, size[1] / bins as f64],
            frequency_x,
            frequency_y,
            cosine,
            cosine_t,
            sine_x,
            sine_y,
            fixed: vec![0.0; bins * bins],
            field_x: vec![0.0; bins * bins],
            field_y: vec![0.0; bins * bins],
        }
    }

    /// Calls `visit(bin index, overlap area)` for a rectangle.
    fn overlap(&self, center: Point, half: Point, mut visit: impl FnMut(usize, f64)) {
        let low = [center[0] - half[0], center[1] - half[1]];
        let high = [center[0] + half[0], center[1] + half[1]];
        let range = |axis: usize| {
            let first = ((low[axis] - self.origin[axis]) / self.bin[axis])
                .floor()
                .max(0.0);
            let last = ((high[axis] - self.origin[axis]) / self.bin[axis])
                .floor()
                .min(self.bins as f64 - 1.0);
            // A body entirely off the field touches no bin.
            (first <= last).then_some((first as usize, last as usize))
        };
        let (Some((x0, x1)), Some((y0, y1))) = (range(0), range(1)) else {
            return;
        };
        for y in y0..=y1 {
            let bottom = self.origin[1] + y as f64 * self.bin[1];
            let height = (high[1].min(bottom + self.bin[1]) - low[1].max(bottom)).max(0.0);
            for x in x0..=x1 {
                let left = self.origin[0] + x as f64 * self.bin[0];
                let width = (high[0].min(left + self.bin[0]) - low[0].max(left)).max(0.0);
                if width > 0.0 && height > 0.0 {
                    visit(y * self.bins + x, width * height);
                }
            }
        }
    }

    /// Bodies smaller than a bin are smeared over ~1.4 bins with their charge
    /// preserved, which keeps the field smooth (ePlace's local smoothing).
    fn smoothed(&self, half: Point) -> (Point, f64) {
        let minimum = [
            self.bin[0] * std::f64::consts::FRAC_1_SQRT_2,
            self.bin[1] * std::f64::consts::FRAC_1_SQRT_2,
        ];
        let smooth = [half[0].max(minimum[0]), half[1].max(minimum[1])];
        (smooth, (half[0] * half[1]) / (smooth[0] * smooth[1]))
    }

    /// Solves the Poisson equation for `density` (area per bin) and stores
    /// the electric field.
    fn solve(&mut self, density: &[f64]) {
        // Every pass adds scaled rows to rows (no sums across a row), so
        // it runs on contiguous memory and vectorizes.
        let n = self.bins;
        let bin_area = self.bin[0] * self.bin[1];
        let mean = density.iter().sum::<f64>() / (n * n) as f64;
        let mut temporary = vec![0.0; n * n];
        let mut coefficients = vec![0.0; n * n];
        // Forward cosine transform, rows then columns. Index = v * n + u.
        for y in 0..n {
            let row = &mut temporary[y * n..(y + 1) * n];
            for x in 0..n {
                let value = (density[y * n + x] - mean) / bin_area;
                for (out, basis) in row.iter_mut().zip(&self.cosine_t[x * n..(x + 1) * n]) {
                    *out += value * basis;
                }
            }
        }
        for v in 0..n {
            let row = &mut coefficients[v * n..(v + 1) * n];
            for y in 0..n {
                let basis = self.cosine[v * n + y];
                for (out, value) in row.iter_mut().zip(&temporary[y * n..(y + 1) * n]) {
                    *out += value * basis;
                }
            }
            let scale = |index: usize| if index == 0 { 1.0 } else { 2.0 } / n as f64;
            for (u, out) in row.iter_mut().enumerate() {
                let frequency = self.frequency_x[u].powi(2) + self.frequency_y[v].powi(2);
                *out = if u == 0 && v == 0 {
                    0.0
                } else {
                    *out * scale(u) * scale(v) / frequency
                };
            }
        }
        // Field = -grad(potential): sine along the differentiated axis.
        for along_x in [true, false] {
            temporary.iter_mut().for_each(|value| *value = 0.0);
            let first = if along_x { &self.sine_x } else { &self.cosine };
            for v in 0..n {
                let row = &mut temporary[v * n..(v + 1) * n];
                for u in 0..n {
                    let coefficient = coefficients[v * n + u];
                    for (out, basis) in row.iter_mut().zip(&first[u * n..(u + 1) * n]) {
                        *out += coefficient * basis;
                    }
                }
            }
            let second = if along_x { &self.cosine } else { &self.sine_y };
            let output = if along_x {
                &mut self.field_x
            } else {
                &mut self.field_y
            };
            output.iter_mut().for_each(|value| *value = 0.0);
            for y in 0..n {
                let row = &mut output[y * n..(y + 1) * n];
                for v in 0..n {
                    let basis = second[v * n + y];
                    for (out, value) in row.iter_mut().zip(&temporary[v * n..(v + 1) * n]) {
                        *out += value * basis;
                    }
                }
            }
        }
    }
}

/// The descent's state at an iteration (`GlobalResult::history`): what
/// decides whether the wires arrange the parts before the density spreads
/// them (docs/placer.md, "The global placement's start (2026-11-07)").
#[derive(Clone, Copy, Debug, Default)]
pub struct GlobalStep {
    pub iteration: usize,
    pub overflow: f64,
    /// The density weight.
    pub lambda: f64,
    /// The weighted-average wirelength's smoothing length, mm.
    pub gamma: f64,
    /// The wirelength gradient's sum of absolute components.
    pub wire: f64,
    /// The density gradient's, times `lambda`.
    pub density: f64,
    /// The weighted half-perimeter wirelength at the poses.
    pub wirelength: f64,
}

/// Iterations between two `GlobalStep`s of the history (the last is
/// always kept).
pub const HISTORY_INTERVAL: usize = 10;

pub struct GlobalResult {
    pub poses: Vec<Pose>,
    pub frames: Vec<Frame>,
    pub iterations: usize,
    pub overflow: f64,
    /// The poses the descent started from (the start's noise, or the
    /// netlist's start).
    pub start: Vec<Pose>,
    /// The descent's state every `HISTORY_INTERVAL` iterations and at its
    /// last.
    pub history: Vec<GlobalStep>,
    /// The netlist's stacks glued into banks (`GlobalConfig::stacks`), by
    /// component index.
    pub stacks: Vec<Vec<usize>>,
}

pub fn global_place(problem: &Problem, config: &GlobalConfig) -> GlobalResult {
    let bounds = problem.bounds();
    let mut field = Field::new(config.bins, bounds);
    let bin_area = field.bin[0] * field.bin[1];
    let n = config.bins;

    // Immovable charge: outside the outline and fixed bodies.
    const SAMPLES: usize = 4;
    for y in 0..n {
        for x in 0..n {
            let mut outside = 0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let point = [
                        field.origin[0]
                            + (x as f64 + (sx as f64 + 0.5) / SAMPLES as f64) * field.bin[0],
                        field.origin[1]
                            + (y as f64 + (sy as f64 + 0.5) / SAMPLES as f64) * field.bin[1],
                    ];
                    outside += !point_in_polygon(point, &problem.outline) as usize;
                }
            }
            field.fixed[y * n + x] = bin_area * outside as f64 / (SAMPLES * SAMPLES) as f64;
        }
    }
    let mut poses = problem.poses.clone();
    // A board with parts on both sides gets one density field per side:
    // a front part and a back part may share the same spot.
    use crate::problem::Side;
    let two_sided = problem
        .components
        .iter()
        .any(|component| component.side == Side::Front)
        && problem
            .components
            .iter()
            .any(|component| component.side == Side::Back);
    let side_count = if two_sided { 2 } else { 1 };
    let fields_of = |side: Side| -> Vec<usize> {
        match side {
            Side::Front => vec![0],
            Side::Back => vec![side_count - 1],
            Side::Both => (0..side_count).collect(),
            Side::Neither => Vec::new(),
        }
    };
    let mut side_fixed = vec![field.fixed.clone(); side_count];
    for (index, component) in problem.components.iter().enumerate() {
        if component.fixed {
            let pose = poses[index];
            let half = component.half_extent(pose.angle);
            for side in fields_of(component.side) {
                if component.hollow.is_empty() {
                    field.overlap(
                        component.center(pose),
                        [half[0] + component.halo, half[1] + component.halo],
                        |bin, area| side_fixed[side][bin] += area,
                    );
                } else {
                    for (center, half) in component.hollow_boxes(pose) {
                        field.overlap(center, half, |bin, area| side_fixed[side][bin] += area);
                    }
                }
            }
            // A through-hole part's holes are in the way on the other side.
            if two_sided && component.side != Side::Both {
                let other = 1 - fields_of(component.side).first().copied().unwrap_or(0);
                for (center, half) in component.far_boxes(pose) {
                    field.overlap(center, half, |bin, area| side_fixed[other][bin] += area);
                }
            }
        }
    }
    for fixed in &mut side_fixed {
        for value in fixed.iter_mut() {
            *value = value.min(bin_area);
        }
    }
    let side_free: Vec<f64> = side_fixed
        .iter()
        .map(|fixed| fixed.iter().map(|fixed| bin_area - fixed).sum())
        .collect();

    let movable: Vec<usize> = (0..problem.components.len())
        .filter(|index| !problem.components[*index].fixed)
        .collect();
    if movable.is_empty() {
        return GlobalResult {
            start: poses.clone(),
            poses,
            frames: Vec::new(),
            iterations: 0,
            overflow: 0.0,
            history: Vec::new(),
            stacks: Vec::new(),
        };
    }
    let mut bodies: Vec<Body> = movable
        .iter()
        .map(|index| {
            let component = &problem.components[*index];
            let half = component.half_extent(poses[*index].angle);
            let halo = component.halo;
            Body {
                component: Some(*index),
                half: [half[0] + halo, half[1] + halo],
                bare: half,
                angle: poses[*index].angle,
                pins: component.pins.len() as f64,
                field: fields_of(component.side).first().copied().unwrap_or(0),
                density: if component.hollow.is_empty() {
                    1.0
                } else {
                    let whole = 4.0 * (half[0] + halo) * (half[1] + halo);
                    (component.blocking_area() / whole.max(1.0e-9))
                        .sqrt()
                        .min(1.0)
                },
                extra: [0.0; 4],
            }
        })
        .collect();
    let movable_area: f64 = bodies
        .iter()
        .map(|body| 4.0 * body.charge_half()[0] * body.charge_half()[1])
        .sum();
    let mut areas: Vec<f64> = bodies
        .iter()
        .map(|body| 4.0 * body.charge_half()[0] * body.charge_half()[1])
        .collect();
    areas.sort_by(f64::total_cmp);
    let trimmed =
        &areas[areas.len() / 10..(areas.len() * 9).div_ceil(10).max(areas.len() / 10 + 1)];
    let filler_side = (trimmed.iter().sum::<f64>() / trimmed.len() as f64)
        .sqrt()
        .max(field.bin[0].min(field.bin[1]));
    // Fillers take a share of each side's whitespace.
    for (side, free) in side_free.iter().enumerate().take(side_count) {
        let side_movable: f64 = bodies
            .iter()
            .filter(|body| body.field == side)
            .map(|body| 4.0 * body.charge_half()[0] * body.charge_half()[1])
            .sum();
        let filler_area = config.whitespace_fill.clamp(0.0, 1.0) * (free - side_movable).max(0.0);
        let filler_count = (filler_area / (filler_side * filler_side)).floor() as usize;
        for _ in 0..filler_count {
            bodies.push(Body {
                component: None,
                half: [filler_side / 2.0; 2],
                bare: [filler_side / 2.0; 2],
                angle: 0.0,
                pins: 0.0,
                field: side,
                density: 1.0,
                extra: [0.0; 4],
            });
        }
    }

    // Start from the pin centroid of fixed parts (or the board centre) with
    // a little noise; fillers start spread out.
    let mut random = Random(config.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut anchor = [0.0; 2];
    let mut anchors = 0.0;
    for (index, component) in problem.components.iter().enumerate() {
        if component.fixed {
            for pin in &component.pins {
                let at = component.pin_position(pin, poses[index]);
                anchor[0] += at[0];
                anchor[1] += at[1];
                anchors += 1.0;
            }
        }
    }
    let board_center = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
    let start = if anchors > 0.0 {
        [
            0.5 * (anchor[0] / anchors) + 0.5 * board_center[0],
            0.5 * (anchor[1] / anchors) + 0.5 * board_center[1],
        ]
    } else {
        board_center
    };
    let size = [bounds[2] - bounds[0], bounds[3] - bounds[1]];
    let starts: BTreeMap<usize, Point> = problem.constraints.starts.iter().copied().collect();
    let mut major: Vec<Point> = bodies
        .iter()
        .map(|body| {
            if let Some(index) = body.component {
                if config.warm.as_ref().is_some_and(|warm| warm[index]) {
                    // The noise is drawn all the same: the other parts
                    // start where they would.
                    random.next();
                    random.next();
                    return problem.components[index].center(poses[index]);
                }
                let start = starts.get(&index).copied().unwrap_or(start);
                [
                    start[0] + (random.next() - 0.5) * 0.05 * size[0],
                    start[1] + (random.next() - 0.5) * 0.05 * size[1],
                ]
            } else {
                [
                    bounds[0] + random.next() * size[0],
                    bounds[1] + random.next() * size[1],
                ]
            }
        })
        .collect();
    let clamp = |centers: &mut Vec<Point>, bodies: &[Body]| {
        for (center, body) in centers.iter_mut().zip(bodies) {
            for axis in 0..2 {
                let low = bounds[axis] + body.half[axis];
                let high = bounds[axis + 2] - body.half[axis];
                center[axis] = if low <= high {
                    center[axis].clamp(low, high)
                } else {
                    (bounds[axis] + bounds[axis + 2]) / 2.0
                };
            }
            if let Some(index) = body.component {
                constraints::clamp_center(problem, index, center, body.bare, body.angle);
            }
        }
    };
    clamp(&mut major, &bodies);

    // Per movable component: pin offsets from the body centre at its angle.
    let pin_offsets = |poses: &[Pose]| -> Vec<Vec<(Point, usize)>> {
        movable
            .iter()
            .map(|index| {
                let component = &problem.components[*index];
                component
                    .pins
                    .iter()
                    .map(|pin| {
                        let local = [
                            pin.offset[0] - component.body_center[0],
                            pin.offset[1] - component.body_center[1],
                        ];
                        (rotate(local, -poses[*index].angle), pin.net)
                    })
                    .collect()
            })
            .collect()
    };
    let mut offsets = pin_offsets(&poses);
    let fixed_pins: Vec<(Point, usize)> = problem
        .components
        .iter()
        .enumerate()
        .filter(|(_, component)| component.fixed)
        .flat_map(|(index, component)| {
            let pose = poses[index];
            component
                .pins
                .iter()
                .map(move |pin| (component.pin_position(pin, pose), pin.net))
        })
        .collect();
    let nets = problem.net_weights.len();
    // The banks that move as one body in the descent (body indices), and
    // the same by component index.
    let mut glued: Vec<Vec<usize>> = Vec::new();
    let mut stacked: Vec<Vec<usize>> = Vec::new();
    {
        // The parts start where the netlist's quadratic wirelength puts
        // them (`initial::quadratic_start`), with a tenth of the noise
        // drawn above: it breaks ties (parts on the same nets), not the
        // arrangement. From one point with the whole noise the arrangement
        // was the noise's (docs/placer.md, "The global placement's start
        // (2026-11-07)").
        let count = movable.len();
        let warm = |index: usize| config.warm.as_ref().is_some_and(|warm| warm[index]);
        // A part with a start of its own (a panel copy's, at its region's
        // centre) begins there as before: solved, the copies' parts were
        // projected against their regions' borders and piled up there
        // (SmartSpin2k, stage 1: 33 to 51 first-route open terminals
        // became 65 to 90).
        let held: Vec<bool> = movable
            .iter()
            .map(|index| warm(*index) || starts.contains_key(index))
            .collect();
        let anchor: Vec<Point> = movable
            .iter()
            .map(|index| starts.get(index).copied().unwrap_or(start))
            .collect();
        let noise: Vec<Point> = (0..count)
            .map(|body| {
                if held[body] {
                    [0.0; 2]
                } else {
                    [
                        major[body][0] - anchor[body][0],
                        major[body][1] - anchor[body][1],
                    ]
                }
            })
            .collect();
        let mut centers: Vec<Point> = (0..count)
            .map(|body| {
                if held[body] {
                    major[body]
                } else {
                    anchor[body]
                }
            })
            .collect();
        let solve_work = crate::initial::quadratic_start(
            problem,
            &movable,
            &offsets,
            &fixed_pins,
            &anchor,
            &held,
            &mut centers,
            |centers: &mut [Point]| {
                let mut moved = centers.to_vec();
                clamp(&mut moved, &bodies);
                centers.copy_from_slice(&moved);
            },
        );
        crate::add_work(solve_work);
        if config.stacks != Stacks::Off {
            let stacks = spread_stacks(
                problem,
                config.stacks,
                &movable,
                &bodies[..count],
                &offsets,
                &fixed_pins,
                &held,
                &mut centers,
            );
            stacked = stacks
                .iter()
                .map(|members| members.iter().map(|body| movable[*body]).collect())
                .collect();
            glued = stacks;
        }
        for body in 0..count {
            major[body] = [
                centers[body][0] + NETLIST_NOISE * noise[body][0],
                centers[body][1] + NETLIST_NOISE * noise[body][1],
            ];
        }
        clamp(&mut major, &bodies);
    }
    let start_poses: Vec<Pose> = {
        let mut at = poses.clone();
        for (body, index) in movable.iter().enumerate() {
            at[*index].position =
                problem.components[*index].position_for_center(major[body], at[*index].angle);
        }
        at
    };
    let mut history = Vec::new();

    let mut reference = major.clone();
    let mut momentum = 1.0f64;
    let mut previous: Option<(Vec<Point>, Vec<Point>)> = None;
    let mut lambda = 0.0;
    let mut overflow = 1.0;
    // The least overflow so far (by `stall_factor`) and its iteration.
    let mut least = (f64::INFINITY, 0usize);
    let mut frames = Vec::new();
    let mut iterations = 0;
    let mut density = vec![0.0; n * n];
    // Escape demand: each net's pins, for the parts' exits.
    let escape = problem.escape.as_deref();
    let escape_members = escape.map(|_| crate::escape::members(problem));

    let mut last_step = GlobalStep::default();
    for iteration in 0..config.max_iterations {
        iterations = iteration + 1;
        // Work: the field solve per side dominates, then bodies and pins.
        crate::add_work((side_fixed.len() * n * n * n + bodies.len() + fixed_pins.len()) as u64);
        // Density of all charges at the reference solution, side by side.
        let mut density_gradient = vec![[0.0; 2]; bodies.len()];
        let mut overflow_area = 0.0;
        for (side, fixed) in side_fixed.iter().enumerate() {
            density.copy_from_slice(fixed);
            let mut real = vec![0.0; n * n];
            for (body, center) in bodies
                .iter()
                .zip(&reference)
                .filter(|(body, _)| body.field == side)
            {
                let (half, scale) = field.smoothed(body.charge_half());
                if escape.is_none() {
                    field.overlap(*center, half, |bin, area| {
                        density[bin] += area * scale;
                        if body.component.is_some() {
                            real[bin] += area * scale;
                        }
                    });
                } else {
                    // The room in front of dense sides is charge, not
                    // body: it pushes, the overflow counts the bodies.
                    let (at, grown) = body.charge_box(*center);
                    let (grown, grown_scale) = field.smoothed(grown);
                    field.overlap(at, grown, |bin, area| density[bin] += area * grown_scale);
                    if body.component.is_some() {
                        field.overlap(*center, half, |bin, area| real[bin] += area * scale);
                    }
                }
            }
            overflow_area += real
                .iter()
                .zip(fixed)
                .map(|(real, fixed)| (real - (bin_area - fixed)).max(0.0))
                .sum::<f64>();
            field.solve(&density);
            for (index, (body, center)) in bodies.iter().zip(&reference).enumerate() {
                if body.field != side {
                    continue;
                }
                let (at, half) = if escape.is_none() {
                    (*center, body.charge_half())
                } else {
                    body.charge_box(*center)
                };
                let (half, scale) = field.smoothed(half);
                let mut force = [0.0; 2];
                field.overlap(at, half, |bin, area| {
                    force[0] += area * scale * field.field_x[bin];
                    force[1] += area * scale * field.field_y[bin];
                });
                density_gradient[index] = [-force[0], -force[1]];
            }
        }
        overflow = overflow_area / movable_area;

        // Weighted-average wirelength gradient.
        let gamma = 8.0
            * field.bin[0].max(field.bin[1])
            * 10f64.powf((overflow.min(1.0) - 0.1) * 20.0 / 9.0 - 1.0);
        let mut wire_gradient = vec![[0.0; 2]; bodies.len()];
        let mut net_pins: Vec<Vec<NetPin>> = vec![Vec::new(); nets];
        for (at, net) in &fixed_pins {
            net_pins[*net].push((*at, None));
        }
        for (body, pins) in offsets.iter().enumerate() {
            for (pin, (offset, net)) in pins.iter().enumerate() {
                let at = [
                    reference[body][0] + offset[0],
                    reference[body][1] + offset[1],
                ];
                net_pins[*net].push((at, Some((body, pin))));
            }
        }
        for (net, pins) in net_pins.iter().enumerate() {
            if pins.len() < 2 {
                continue;
            }
            let weight = problem.net_weights[net];
            for axis in 0..2 {
                let maximum = pins
                    .iter()
                    .map(|(at, _)| at[axis])
                    .fold(f64::NEG_INFINITY, f64::max);
                let minimum = pins
                    .iter()
                    .map(|(at, _)| at[axis])
                    .fold(f64::INFINITY, f64::min);
                let (mut sum_high, mut weighted_high, mut sum_low, mut weighted_low) =
                    (0.0, 0.0, 0.0, 0.0);
                for (at, _) in pins {
                    let high = ((at[axis] - maximum) / gamma).exp();
                    let low = ((minimum - at[axis]) / gamma).exp();
                    sum_high += high;
                    weighted_high += at[axis] * high;
                    sum_low += low;
                    weighted_low += at[axis] * low;
                }
                for (at, owner) in pins {
                    let Some((body, _)) = owner else {
                        continue;
                    };
                    let high = ((at[axis] - maximum) / gamma).exp();
                    let low = ((minimum - at[axis]) / gamma).exp();
                    let upper = high
                        * ((1.0 + at[axis] / gamma) * sum_high - weighted_high / gamma)
                        / (sum_high * sum_high);
                    let lower = low * ((1.0 - at[axis] / gamma) * sum_low + weighted_low / gamma)
                        / (sum_low * sum_low);
                    wire_gradient[*body][axis] += weight * (upper - lower);
                }
            }
        }

        if !problem.constraints.relations.is_empty() {
            let mut at = poses.clone();
            for (body, index) in movable.iter().enumerate() {
                at[*index].position = problem.components[*index]
                    .position_for_center(reference[body], at[*index].angle);
            }
            let mut body_of = vec![usize::MAX; problem.components.len()];
            for (body, index) in movable.iter().enumerate() {
                body_of[*index] = body;
            }
            constraints::add_relation_gradient(
                problem,
                &at,
                &body_of,
                &mut wire_gradient,
                problem.constraints.relation_weight,
            );
        }

        let wire: f64 = wire_gradient.iter().map(|g| g[0].abs() + g[1].abs()).sum();
        let charge: f64 = density_gradient
            .iter()
            .map(|g| g[0].abs() + g[1].abs())
            .sum();
        if iteration == 0 {
            lambda = if charge > 0.0 { wire / charge } else { 1.0 };
            if lambda == 0.0 {
                lambda = 1.0e-3;
            }
        }
        let step_state = GlobalStep {
            iteration,
            overflow,
            lambda,
            gamma,
            wire,
            density: lambda * charge,
            wirelength: 0.0,
        };

        let precondition = |body: &Body| {
            let charge = 4.0 * body.charge_half()[0] * body.charge_half()[1];
            (body.pins + lambda * charge).max(1.0)
        };
        let mut gradient: Vec<Point> = bodies
            .iter()
            .enumerate()
            .map(|(index, body)| {
                let precondition = precondition(body);
                [
                    (wire_gradient[index][0] + lambda * density_gradient[index][0]) / precondition,
                    (wire_gradient[index][1] + lambda * density_gradient[index][1]) / precondition,
                ]
            })
            .collect();
        // A glued bank moves as one body: its members' summed forces over
        // their summed preconditioners.
        for members in &glued {
            let mut force = [0.0; 2];
            let mut weight = 0.0;
            for body in members {
                for axis in 0..2 {
                    force[axis] +=
                        wire_gradient[*body][axis] + lambda * density_gradient[*body][axis];
                }
                weight += precondition(&bodies[*body]);
            }
            for body in members {
                gradient[*body] = [force[0] / weight, force[1] / weight];
            }
        }

        // Barzilai-Borwein estimate of the inverse Lipschitz constant.
        let norm = |a: &[Point], b: &[Point]| -> f64 {
            a.iter()
                .zip(b)
                .map(|(a, b)| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2))
                .sum::<f64>()
                .sqrt()
        };
        let largest = gradient
            .iter()
            .map(|g| g[0].abs().max(g[1].abs()))
            .fold(0.0, f64::max)
            .max(1.0e-12);
        let limit = 1.5 * field.bin[0].max(field.bin[1]) / largest;
        let step = match &previous {
            Some((old_reference, old_gradient)) => {
                let change = norm(&gradient, old_gradient);
                if change > 0.0 {
                    (norm(&reference, old_reference) / change).min(limit)
                } else {
                    limit
                }
            }
            None => 0.1 * limit,
        };

        let mut next_major: Vec<Point> = reference
            .iter()
            .zip(&gradient)
            .map(|(at, g)| [at[0] - step * g[0], at[1] - step * g[1]])
            .collect();
        clamp(&mut next_major, &bodies);
        let next_momentum = (1.0 + (4.0 * momentum * momentum + 1.0).sqrt()) / 2.0;
        let blend = (momentum - 1.0) / next_momentum;
        let mut next_reference: Vec<Point> = next_major
            .iter()
            .zip(&major)
            .map(|(new, old)| {
                [
                    new[0] + blend * (new[0] - old[0]),
                    new[1] + blend * (new[1] - old[1]),
                ]
            })
            .collect();
        clamp(&mut next_reference, &bodies);
        previous = Some((reference, gradient));
        major = next_major;
        reference = next_reference;
        momentum = next_momentum;
        lambda *= config.lambda_growth;

        for (body, index) in movable.iter().enumerate() {
            let component = &problem.components[*index];
            poses[*index].position =
                component.position_for_center(major[body], poses[*index].angle);
        }

        // Discrete rotations once the parts have started to separate (a
        // glued bank's members keep their angles).
        let glued_angles: Vec<(usize, f64)> = glued
            .iter()
            .flatten()
            .map(|body| (movable[*body], poses[movable[*body]].angle))
            .collect();
        if overflow < 0.75
            && config.rotation_interval > 0
            && iteration % config.rotation_interval == config.rotation_interval - 1
            && choose_rotations(
                problem,
                &movable,
                &major,
                &mut poses,
                config.rotation_net_limit,
                escape.zip(escape_members.as_deref()),
                config.fit_turns,
            )
        {
            for (index, angle) in &glued_angles {
                poses[*index].angle = *angle;
            }
            offsets = pin_offsets(&poses);
            for (body, index) in movable.iter().enumerate() {
                let component = &problem.components[*index];
                let half = component.half_extent(poses[*index].angle);
                let halo = component.halo;
                bodies[body].half = [half[0] + halo, half[1] + halo];
                bodies[body].bare = half;
                bodies[body].angle = poses[*index].angle;
                poses[*index].position = problem.components[*index]
                    .position_for_center(major[body], poses[*index].angle);
            }
            clamp(&mut major, &bodies);
            reference = major.clone();
            momentum = 1.0;
            previous = None;
        }
        // Escape demand: each part's room per side, from where its pins'
        // nets go now.
        if let (Some(escape), Some(members)) = (escape, escape_members.as_deref())
            && config.rotation_interval > 0
            && iteration % config.rotation_interval == config.rotation_interval - 1
        {
            let mut exits = Vec::new();
            for (body, index) in movable.iter().enumerate() {
                crate::escape::exits(problem, escape, members, &poses, *index, &mut exits);
                bodies[body].extra = crate::escape::inflation(escape, &exits);
            }
        }

        if iteration % HISTORY_INTERVAL == 0 {
            history.push(GlobalStep {
                wirelength: problem.wirelength(&poses),
                ..step_state
            });
        }
        last_step = step_state;
        if config.frame_interval > 0 && iteration % config.frame_interval == 0 {
            frames.push(Frame {
                iteration,
                overflow,
                wirelength: problem.wirelength(&poses),
                poses: poses.clone(),
            });
        }
        if overflow <= config.stop_overflow && iteration > 20 {
            break;
        }
        if overflow < least.0 * config.stall_factor {
            least = (overflow, iteration);
        }
        if config.stall_iterations > 0 && iteration >= least.1 + config.stall_iterations {
            if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
                eprintln!(
                    "placer: global placement stalled: overflow {overflow:.3}, its least {:.3} at iteration {} of {iterations}",
                    least.0, least.1
                );
            }
            break;
        }
    }
    frames.push(Frame {
        iteration: iterations,
        overflow,
        wirelength: problem.wirelength(&poses),
        poses: poses.clone(),
    });
    if history
        .last()
        .is_none_or(|step: &GlobalStep| step.iteration != last_step.iteration)
    {
        history.push(GlobalStep {
            wirelength: problem.wirelength(&poses),
            ..last_step
        });
    }
    GlobalResult {
        poses,
        frames,
        iterations,
        overflow,
        start: start_poses,
        history,
        stacks: stacked,
    }
}

/// Greedily picks, per movable component, the allowed angle minimizing the
/// half-perimeter of its nets, rotating about the body centre.
pub fn choose_rotations(
    problem: &Problem,
    movable: &[usize],
    centers: &[Point],
    poses: &mut [Pose],
    net_limit: usize,
    escape: Option<(&crate::escape::Escape, &[crate::escape::NetPins])>,
    fit_turns: bool,
) -> bool {
    let nets = problem.net_weights.len();
    // Escape demand: the other parts' bodies where they are now.
    let bodies: Vec<(Point, Point)> = if escape.is_some() {
        (0..problem.components.len())
            .map(|index| crate::escape::body_box(problem, index, poses[index]))
            .collect()
    } else {
        Vec::new()
    };
    let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nets];
    for (index, component) in problem.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            members[description.net].push((index, pin));
        }
    }
    let mut changed = false;
    let bounds = problem.bounds();
    let size = [bounds[2] - bounds[0], bounds[3] - bounds[1]];
    for (body, index) in movable.iter().enumerate() {
        let component = &problem.components[*index];
        if component.angle_options.len() < 2 || component.pins.is_empty() {
            continue;
        }
        let mut incident: Vec<usize> = component.pins.iter().map(|pin| pin.net).collect();
        incident.sort_unstable();
        incident.dedup();
        incident.retain(|net| members[*net].len() >= 2 && members[*net].len() <= net_limit);
        if incident.is_empty() {
            continue;
        }
        let cost = |poses: &[Pose]| -> f64 {
            incident
                .iter()
                .map(|net| {
                    let mut bounds = [
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ];
                    for (other, pin) in &members[*net] {
                        let at = problem.components[*other]
                            .pin_position(&problem.components[*other].pins[*pin], poses[*other]);
                        bounds[0] = bounds[0].min(at[0]);
                        bounds[1] = bounds[1].min(at[1]);
                        bounds[2] = bounds[2].max(at[0]);
                        bounds[3] = bounds[3].max(at[1]);
                    }
                    problem.net_weights[*net] * ((bounds[2] - bounds[0]) + (bounds[3] - bounds[1]))
                })
                .sum()
        };
        // Never an angle at which the body is longer than the board on
        // either axis while another angle fits: no density settles there
        // and no legal place exists (video's 116 x 8 mm memory modules
        // turned upright on its 107 mm high board).
        let fits = |angle: f64| {
            let half = component.half_extent(angle);
            2.0 * half[0] <= size[0] + 1.0e-9 && 2.0 * half[1] <= size[1] + 1.0e-9
        };
        let some_fit = component.angle_options.iter().any(|angle| fits(*angle));
        let allowed = |angle: f64| !fit_turns || !some_fit || fits(angle);
        let original = poses[*index];
        let mut best = (
            if allowed(original.angle) {
                cost(poses)
            } else {
                f64::INFINITY
            },
            original,
        );
        if let Some((escape, members)) = escape {
            // The quarter turn that points the dense sides at free space
            // wins when its wirelength is close to the shortest.
            let mut options = Vec::new();
            if allowed(original.angle) {
                options.push((best.0, original));
            }
            for angle in &component.angle_options {
                if (angle - original.angle).abs() < 1.0e-9 || !allowed(*angle) {
                    continue;
                }
                poses[*index] = Pose {
                    position: component.position_for_center(centers[body], *angle),
                    angle: *angle,
                };
                options.push((cost(poses), poses[*index]));
            }
            let shortest = options
                .iter()
                .map(|(cost, _)| *cost)
                .fold(f64::INFINITY, f64::min);
            let mut chosen: Option<(f64, f64, Pose)> = None;
            for (wire, pose) in options {
                if wire > shortest * (1.0 + escape.turn_tolerance) + 1.0e-9 {
                    continue;
                }
                poses[*index] = pose;
                let blocked = crate::orient::escape_blocking(
                    problem, escape, members, poses, *index, &bodies,
                );
                if chosen.is_none_or(|(least, shortest_wire, _)| {
                    blocked < least - 1.0e-9
                        || (blocked < least + 1.0e-9 && wire < shortest_wire - 1.0e-9)
                }) {
                    chosen = Some((blocked, wire, pose));
                }
            }
            if let Some((_, wire, pose)) = chosen {
                best = (wire, pose);
            }
            poses[*index] = best.1;
            changed |= best.1 != original;
            continue;
        }
        for angle in &component.angle_options {
            if (angle - original.angle).abs() < 1.0e-9 || !allowed(*angle) {
                continue;
            }
            poses[*index] = Pose {
                position: component.position_for_center(centers[body], *angle),
                angle: *angle,
            };
            let candidate = cost(poses);
            if candidate < best.0 - 1.0e-9 {
                best = (candidate, poses[*index]);
            }
        }
        poses[*index] = best.1;
        changed |= best.1 != original;
    }
    changed
}

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

#[derive(Clone, Debug)]
pub struct GlobalConfig {
    pub bins: usize,
    /// Fraction of the whitespace (free area not needed by bodies and their
    /// halos) that is occupied by filler charge. 1 lets wirelength pull the
    /// parts into a compact cluster; 0 spreads them over the whole board.
    pub whitespace_fill: f64,
    pub stop_overflow: f64,
    pub max_iterations: usize,
    /// Growth of the density weight per iteration.
    pub lambda_growth: f64,
    /// Iterations between discrete rotation updates.
    pub rotation_interval: usize,
    /// Nets with more pins are ignored when choosing rotations.
    pub rotation_net_limit: usize,
    pub frame_interval: usize,
    pub seed: u64,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            bins: 64,
            whitespace_fill: 0.6,
            stop_overflow: 0.04,
            max_iterations: 1500,
            lambda_growth: 1.03,
            rotation_interval: 15,
            rotation_net_limit: 24,
            frame_interval: 5,
            seed: 1,
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
}

impl Body {
    fn charge_half(&self) -> Point {
        [self.half[0] * self.density, self.half[1] * self.density]
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
        let frequency_x: Vec<f64> = (0..bins).map(|u| std::f64::consts::PI * u as f64 / size[0]).collect();
        let frequency_y: Vec<f64> = (0..bins).map(|v| std::f64::consts::PI * v as f64 / size[1]).collect();
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
            let first = ((low[axis] - self.origin[axis]) / self.bin[axis]).floor().max(0.0);
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
                *out = if u == 0 && v == 0 { 0.0 } else { *out * scale(u) * scale(v) / frequency };
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
            let output = if along_x { &mut self.field_x } else { &mut self.field_y };
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

pub struct GlobalResult {
    pub poses: Vec<Pose>,
    pub frames: Vec<Frame>,
    pub iterations: usize,
    pub overflow: f64,
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
                        field.origin[0] + (x as f64 + (sx as f64 + 0.5) / SAMPLES as f64) * field.bin[0],
                        field.origin[1] + (y as f64 + (sy as f64 + 0.5) / SAMPLES as f64) * field.bin[1],
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
    let two_sided = problem.components.iter().any(|component| component.side == Side::Front)
        && problem.components.iter().any(|component| component.side == Side::Back);
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
            poses,
            frames: Vec::new(),
            iterations: 0,
            overflow: 0.0,
        };
    }
    let mut bodies: Vec<Body> = movable
        .iter()
        .map(|index| {
            let component = &problem.components[*index];
            let half = component.half_extent(poses[*index].angle);
            Body {
                component: Some(*index),
                half: [half[0] + component.halo, half[1] + component.halo],
                bare: half,
                angle: poses[*index].angle,
                pins: component.pins.len() as f64,
                field: fields_of(component.side).first().copied().unwrap_or(0),
                density: if component.hollow.is_empty() {
                    1.0
                } else {
                    let whole = 4.0 * (half[0] + component.halo) * (half[1] + component.halo);
                    (component.blocking_area() / whole.max(1.0e-9)).sqrt().min(1.0)
                },
            }
        })
        .collect();
    let movable_area: f64 = bodies.iter().map(|body| 4.0 * body.charge_half()[0] * body.charge_half()[1]).sum();
    let mut areas: Vec<f64> = bodies.iter().map(|body| 4.0 * body.charge_half()[0] * body.charge_half()[1]).collect();
    areas.sort_by(f64::total_cmp);
    let trimmed = &areas[areas.len() / 10..(areas.len() * 9).div_ceil(10).max(areas.len() / 10 + 1)];
    let filler_side = (trimmed.iter().sum::<f64>() / trimmed.len() as f64)
        .sqrt()
        .max(field.bin[0].min(field.bin[1]));
    // Fillers take a share of each side's whitespace.
    for side in 0..side_count {
        let side_movable: f64 = bodies
            .iter()
            .filter(|body| body.field == side)
            .map(|body| 4.0 * body.charge_half()[0] * body.charge_half()[1])
            .sum();
        let filler_area = config.whitespace_fill.clamp(0.0, 1.0) * (side_free[side] - side_movable).max(0.0);
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
    let mut major: Vec<Point> = bodies
        .iter()
        .map(|body| {
            if body.component.is_some() {
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

    let mut reference = major.clone();
    let mut momentum = 1.0f64;
    let mut previous: Option<(Vec<Point>, Vec<Point>)> = None;
    let mut lambda = 0.0;
    let mut overflow = 1.0;
    let mut frames = Vec::new();
    let mut iterations = 0;
    let mut density = vec![0.0; n * n];

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
            for (body, center) in bodies.iter().zip(&reference).filter(|(body, _)| body.field == side) {
                let (half, scale) = field.smoothed(body.charge_half());
                field.overlap(*center, half, |bin, area| {
                    density[bin] += area * scale;
                    if body.component.is_some() {
                        real[bin] += area * scale;
                    }
                });
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
                let (half, scale) = field.smoothed(body.charge_half());
                let mut force = [0.0; 2];
                field.overlap(*center, half, |bin, area| {
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
        let mut net_pins: Vec<Vec<(Point, Option<(usize, usize)>)>> = vec![Vec::new(); nets];
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
                let maximum = pins.iter().map(|(at, _)| at[axis]).fold(f64::NEG_INFINITY, f64::max);
                let minimum = pins.iter().map(|(at, _)| at[axis]).fold(f64::INFINITY, f64::min);
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
                    let upper = high * ((1.0 + at[axis] / gamma) * sum_high - weighted_high / gamma)
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
                at[*index].position =
                    problem.components[*index].position_for_center(reference[body], at[*index].angle);
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

        if iteration == 0 {
            let wire: f64 = wire_gradient.iter().map(|g| g[0].abs() + g[1].abs()).sum();
            let charge: f64 = density_gradient.iter().map(|g| g[0].abs() + g[1].abs()).sum();
            lambda = if charge > 0.0 { wire / charge } else { 1.0 };
            if lambda == 0.0 {
                lambda = 1.0e-3;
            }
        }

        let gradient: Vec<Point> = bodies
            .iter()
            .enumerate()
            .map(|(index, body)| {
                let charge = 4.0 * body.charge_half()[0] * body.charge_half()[1];
                let precondition = (body.pins + lambda * charge).max(1.0);
                [
                    (wire_gradient[index][0] + lambda * density_gradient[index][0]) / precondition,
                    (wire_gradient[index][1] + lambda * density_gradient[index][1]) / precondition,
                ]
            })
            .collect();

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
            poses[*index].position = component.position_for_center(major[body], poses[*index].angle);
        }

        // Discrete rotations once the parts have started to separate.
        if overflow < 0.75
            && config.rotation_interval > 0
            && iteration % config.rotation_interval == config.rotation_interval - 1
            && choose_rotations(problem, &movable, &major, &mut poses, config.rotation_net_limit)
        {
            offsets = pin_offsets(&poses);
            for (body, index) in movable.iter().enumerate() {
                let component = &problem.components[*index];
                let half = component.half_extent(poses[*index].angle);
                bodies[body].half = [half[0] + component.halo, half[1] + component.halo];
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
    }
    frames.push(Frame {
        iteration: iterations,
        overflow,
        wirelength: problem.wirelength(&poses),
        poses: poses.clone(),
    });
    GlobalResult {
        poses,
        frames,
        iterations,
        overflow,
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
) -> bool {
    let nets = problem.net_weights.len();
    let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nets];
    for (index, component) in problem.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            members[description.net].push((index, pin));
        }
    }
    let mut changed = false;
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
        let original = poses[*index];
        let mut best = (cost(poses), original);
        for angle in &component.angle_options {
            if (angle - original.angle).abs() < 1.0e-9 {
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

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The global placement's charges and their field: the start's random
//! numbers, the movable bodies and fillers (`Body`), and the density grid
//! with its spectral Poisson solve (`Field`).

use crate::problem::Point;

pub(super) struct Random(pub(super) u64);

impl Random {
    #[inline]
    pub(super) fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A movable charge: a real component or a filler.
pub(super) struct Body {
    /// Index into the problem's components; `None` for fillers.
    pub(super) component: Option<usize>,
    pub(super) half: Point,
    /// Half extent of the body alone (without halo), and its angle, for
    /// constraints.
    pub(super) bare: Point,
    pub(super) angle: f64,
    pub(super) pins: f64,
    /// Which side's density field the body charges (0: front, 1: back on
    /// a two-sided board).
    pub(super) field: usize,
    /// Scales the half extent for density: a hollow part charges only the
    /// area its blocking boxes take.
    pub(super) density: f64,
    /// Escape demand (`Problem::escape`): the charge grows on each side
    /// (-x, +x, -y, +y) by this much, mm.
    pub(super) extra: [f64; 4],
}

impl Body {
    #[inline]
    pub(super) fn charge_half(&self) -> Point {
        [self.half[0] * self.density, self.half[1] * self.density]
    }

    /// The charge's centre and half extent with the escape demand's room.
    #[inline]
    pub(super) fn charge_box(&self, center: Point) -> (Point, Point) {
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

pub(super) struct Field {
    pub(super) bins: usize,
    pub(super) origin: Point,
    pub(super) bin: Point,
    pub(super) cosine: Vec<f64>,
    /// `cosine` transposed (index = x * n + u).
    pub(super) cosine_t: Vec<f64>,
    /// The sine basis times its frequency, along x and along y.
    pub(super) sine_x: Vec<f64>,
    pub(super) sine_y: Vec<f64>,
    pub(super) frequency_x: Vec<f64>,
    pub(super) frequency_y: Vec<f64>,
    pub(super) fixed: Vec<f64>,
    pub(super) field_x: Vec<f64>,
    pub(super) field_y: Vec<f64>,
}

impl Field {
    pub(super) fn new(bins: usize, bounds: [f64; 4]) -> Self {
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
    pub(super) fn overlap(&self, center: Point, half: Point, mut visit: impl FnMut(usize, f64)) {
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
    #[inline]
    pub(super) fn smoothed(&self, half: Point) -> (Point, f64) {
        let minimum = [
            self.bin[0] * std::f64::consts::FRAC_1_SQRT_2,
            self.bin[1] * std::f64::consts::FRAC_1_SQRT_2,
        ];
        let smooth = [half[0].max(minimum[0]), half[1].max(minimum[1])];
        (smooth, (half[0] * half[1]) / (smooth[0] * smooth[1]))
    }

    /// Solves the Poisson equation for `density` (area per bin) and stores
    /// the electric field.
    pub(super) fn solve(&mut self, density: &[f64]) {
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

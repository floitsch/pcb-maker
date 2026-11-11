// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The global placement's Nesterov descent on `wirelength + lambda *
//! density`. One iteration, in its order: the density gradient and the
//! overflow (`spread_charge`), the weighted-average wirelength gradient
//! with the relations' (`wire_gradient`), the density weight's start, the
//! preconditioned gradient with the glued banks (`preconditioned`), the
//! step (`nesterov_step`), the poses, the discrete rotations (`turn`), the
//! escape demand (`escape_demand`), the history and frames (`record`) and
//! the stop tests (`stops`).

use super::field::{Body, Field};
use super::{Board, Frame, GlobalResult, GlobalStep, HISTORY_INTERVAL, choose_rotations};
use crate::constraints;
use crate::problem::{Point, Pose};

/// A pin on a net: its position and, for a movable body's pin, the body and
/// the pin's index on it (`None` for a fixed pin).
type NetPin = (Point, Option<(usize, usize)>);

/// What the descent updates: the bodies (turned by the rotations, grown by
/// the escape demand), the solution and its momentum, the density weight,
/// and what it reports.
pub(super) struct Descent<'a> {
    board: &'a Board<'a>,
    field: Field,
    bodies: Vec<Body>,
    /// Per movable body: its pins' offsets from the body centre and nets.
    offsets: Vec<Vec<(Point, usize)>>,
    /// The poses at `major`.
    poses: Vec<Pose>,
    /// The solution and the reference solution (Nesterov's look-ahead)
    /// the gradient is taken at, per body.
    major: Vec<Point>,
    reference: Vec<Point>,
    momentum: f64,
    /// The reference and the gradient of the iteration before (the step's
    /// Barzilai-Borwein estimate).
    previous: Option<(Vec<Point>, Vec<Point>)>,
    /// The density weight.
    lambda: f64,
    overflow: f64,
    /// The least overflow so far (by `stall_factor`) and its iteration.
    least: (f64, usize),
    /// A side's charge per bin (scratch).
    density: Vec<f64>,
    frames: Vec<Frame>,
    history: Vec<GlobalStep>,
    iterations: usize,
    last_step: GlobalStep,
}

impl<'a> Descent<'a> {
    /// The descent from the start's solution `major` (clamped).
    pub(super) fn new(
        board: &'a Board<'a>,
        field: Field,
        bodies: Vec<Body>,
        offsets: Vec<Vec<(Point, usize)>>,
        poses: Vec<Pose>,
        major: Vec<Point>,
    ) -> Self {
        let n = board.config.bins;
        Descent {
            board,
            field,
            bodies,
            offsets,
            poses,
            reference: major.clone(),
            major,
            momentum: 1.0f64,
            previous: None,
            lambda: 0.0,
            overflow: 1.0,
            least: (f64::INFINITY, 0usize),
            density: vec![0.0; n * n],
            frames: Vec::new(),
            history: Vec::new(),
            iterations: 0,
            last_step: GlobalStep::default(),
        }
    }

    /// Iterates until the overflow is low enough, the descent stalls or
    /// `max_iterations` ran.
    pub(super) fn run(&mut self) {
        for iteration in 0..self.board.config.max_iterations {
            if self.iterate(iteration) {
                break;
            }
        }
    }

    /// One iteration; whether the descent stops after it.
    fn iterate(&mut self, iteration: usize) -> bool {
        let board = self.board;
        let n = board.config.bins;
        self.iterations = iteration + 1;
        // Work: the field solve per side dominates, then bodies and pins.
        crate::add_work(
            (board.side_fixed.len() * n * n * n + self.bodies.len() + board.fixed_pins.len())
                as u64,
        );
        let density_gradient = self.spread_charge();
        let (gamma, wire_gradient) = self.wire_gradient();

        let wire: f64 = wire_gradient.iter().map(|g| g[0].abs() + g[1].abs()).sum();
        let charge: f64 = density_gradient
            .iter()
            .map(|g| g[0].abs() + g[1].abs())
            .sum();
        if iteration == 0 {
            self.lambda = if charge > 0.0 { wire / charge } else { 1.0 };
            if self.lambda == 0.0 {
                self.lambda = 1.0e-3;
            }
        }
        let step_state = GlobalStep {
            iteration,
            overflow: self.overflow,
            lambda: self.lambda,
            gamma,
            wire,
            density: self.lambda * charge,
            wirelength: 0.0,
        };

        let gradient = self.preconditioned(&wire_gradient, &density_gradient);
        self.nesterov_step(gradient);

        for (body, index) in board.movable.iter().enumerate() {
            let component = &board.problem.components[*index];
            self.poses[*index].position =
                component.position_for_center(self.major[body], self.poses[*index].angle);
        }

        self.turn(iteration);
        self.escape_demand(iteration);
        self.record(iteration, step_state);
        self.stops(iteration)
    }

    /// The density gradient of every body at the reference solution, side
    /// by side (each side's charge spread on the grid, its field solved,
    /// the field over each body summed), and the overflow: the bodies'
    /// area beyond the free area of each bin over the movable area. The
    /// per-body loops stay whole: they are the descent's inner loops.
    fn spread_charge(&mut self) -> Vec<[f64; 2]> {
        let board = self.board;
        let n = board.config.bins;
        let bin_area = board.bin_area;
        let escape = board.escape;
        let field = &mut self.field;
        let density = &mut self.density;
        let bodies = &self.bodies;
        let reference = &self.reference;
        // Density of all charges at the reference solution, side by side.
        let mut density_gradient = vec![[0.0; 2]; bodies.len()];
        let mut overflow_area = 0.0;
        for (side, fixed) in board.side_fixed.iter().enumerate() {
            density.copy_from_slice(fixed);
            let mut real = vec![0.0; n * n];
            for (body, center) in bodies
                .iter()
                .zip(reference)
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
            field.solve(density);
            for (index, (body, center)) in bodies.iter().zip(reference).enumerate() {
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
        self.overflow = overflow_area / board.movable_area;
        density_gradient
    }

    /// The weighted-average wirelength's gradient per body at the reference
    /// solution, its smoothing length by the overflow (returned first), and
    /// the relations' gradient added. The per-net loops stay whole.
    fn wire_gradient(&self) -> (f64, Vec<Point>) {
        let board = self.board;
        let problem = board.problem;
        let reference = &self.reference;
        let overflow = self.overflow;
        let nets = problem.net_weights.len();
        // Weighted-average wirelength gradient.
        let gamma = 8.0
            * self.field.bin[0].max(self.field.bin[1])
            * 10f64.powf((overflow.min(1.0) - 0.1) * 20.0 / 9.0 - 1.0);
        let mut wire_gradient = vec![[0.0; 2]; self.bodies.len()];
        let mut net_pins: Vec<Vec<NetPin>> = vec![Vec::new(); nets];
        for (at, net) in &board.fixed_pins {
            net_pins[*net].push((*at, None));
        }
        for (body, pins) in self.offsets.iter().enumerate() {
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
            let mut at = self.poses.clone();
            for (body, index) in board.movable.iter().enumerate() {
                at[*index].position = problem.components[*index]
                    .position_for_center(reference[body], at[*index].angle);
            }
            let mut body_of = vec![usize::MAX; problem.components.len()];
            for (body, index) in board.movable.iter().enumerate() {
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
        (gamma, wire_gradient)
    }

    /// The gradient of `wirelength + lambda * density` per body over its
    /// preconditioner (its pins and its charge times lambda); a glued bank
    /// moves as one body.
    fn preconditioned(&self, wire_gradient: &[Point], density_gradient: &[[f64; 2]]) -> Vec<Point> {
        let lambda = self.lambda;
        let precondition = |body: &Body| {
            let charge = 4.0 * body.charge_half()[0] * body.charge_half()[1];
            (body.pins + lambda * charge).max(1.0)
        };
        let mut gradient: Vec<Point> = self
            .bodies
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
        for members in &self.board.glued {
            let mut force = [0.0; 2];
            let mut weight = 0.0;
            for body in members {
                for axis in 0..2 {
                    force[axis] +=
                        wire_gradient[*body][axis] + lambda * density_gradient[*body][axis];
                }
                weight += precondition(&self.bodies[*body]);
            }
            for body in members {
                gradient[*body] = [force[0] / weight, force[1] / weight];
            }
        }
        gradient
    }

    /// Nesterov's step along `gradient` from the reference solution: its
    /// length by the Barzilai-Borwein estimate of the inverse Lipschitz
    /// constant (at most a bin and a half for the largest component), the
    /// new solution and look-ahead clamped onto the board; then the
    /// density weight grows.
    fn nesterov_step(&mut self, gradient: Vec<Point>) {
        let board = self.board;
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
        let limit = 1.5 * self.field.bin[0].max(self.field.bin[1]) / largest;
        let step = match &self.previous {
            Some((old_reference, old_gradient)) => {
                let change = norm(&gradient, old_gradient);
                if change > 0.0 {
                    (norm(&self.reference, old_reference) / change).min(limit)
                } else {
                    limit
                }
            }
            None => 0.1 * limit,
        };

        let mut next_major: Vec<Point> = self
            .reference
            .iter()
            .zip(&gradient)
            .map(|(at, g)| [at[0] - step * g[0], at[1] - step * g[1]])
            .collect();
        board.clamp(&mut next_major, &self.bodies);
        let momentum = self.momentum;
        let next_momentum = (1.0 + (4.0 * momentum * momentum + 1.0).sqrt()) / 2.0;
        let blend = (momentum - 1.0) / next_momentum;
        let mut next_reference: Vec<Point> = next_major
            .iter()
            .zip(&self.major)
            .map(|(new, old)| {
                [
                    new[0] + blend * (new[0] - old[0]),
                    new[1] + blend * (new[1] - old[1]),
                ]
            })
            .collect();
        board.clamp(&mut next_reference, &self.bodies);
        let reference = std::mem::replace(&mut self.reference, next_reference);
        self.previous = Some((reference, gradient));
        self.major = next_major;
        self.momentum = next_momentum;
        self.lambda *= board.config.lambda_growth;
    }

    /// Discrete rotations every `rotation_interval` iterations once the
    /// parts have started to separate (`choose_rotations`; a glued bank's
    /// members keep their angles). A turn restarts the momentum.
    fn turn(&mut self, iteration: usize) {
        let board = self.board;
        let (problem, config, movable) = (board.problem, board.config, &board.movable);
        // Discrete rotations once the parts have started to separate (a
        // glued bank's members keep their angles).
        let glued_angles: Vec<(usize, f64)> = board
            .glued
            .iter()
            .flatten()
            .map(|body| (movable[*body], self.poses[movable[*body]].angle))
            .collect();
        if self.overflow < 0.75
            && config.rotation_interval > 0
            && iteration % config.rotation_interval == config.rotation_interval - 1
            && choose_rotations(
                problem,
                movable,
                &self.major,
                &mut self.poses,
                config.rotation_net_limit,
                board.escape.zip(board.escape_members.as_deref()),
                config.fit_turns,
            )
        {
            for (index, angle) in &glued_angles {
                self.poses[*index].angle = *angle;
            }
            self.offsets = board.pin_offsets(&self.poses);
            for (body, index) in movable.iter().enumerate() {
                let component = &problem.components[*index];
                let half = component.half_extent(self.poses[*index].angle);
                let halo = component.halo;
                self.bodies[body].half = [half[0] + halo, half[1] + halo];
                self.bodies[body].bare = half;
                self.bodies[body].angle = self.poses[*index].angle;
                self.poses[*index].position = problem.components[*index]
                    .position_for_center(self.major[body], self.poses[*index].angle);
            }
            board.clamp(&mut self.major, &self.bodies);
            self.reference = self.major.clone();
            self.momentum = 1.0;
            self.previous = None;
        }
    }

    /// Escape demand, every `rotation_interval` iterations: each part's
    /// room per side, from where its pins' nets go now.
    fn escape_demand(&mut self, iteration: usize) {
        let board = self.board;
        let config = board.config;
        // Escape demand: each part's room per side, from where its pins'
        // nets go now.
        if let (Some(escape), Some(members)) = (board.escape, board.escape_members.as_deref())
            && config.rotation_interval > 0
            && iteration % config.rotation_interval == config.rotation_interval - 1
        {
            let mut exits = Vec::new();
            for (body, index) in board.movable.iter().enumerate() {
                crate::escape::exits(
                    board.problem,
                    escape,
                    members,
                    &self.poses,
                    *index,
                    &mut exits,
                );
                self.bodies[body].extra = crate::escape::inflation(escape, &exits);
            }
        }
    }

    /// The history every `HISTORY_INTERVAL` iterations and a frame every
    /// `frame_interval`.
    fn record(&mut self, iteration: usize, step_state: GlobalStep) {
        let board = self.board;
        let problem = board.problem;
        if iteration.is_multiple_of(HISTORY_INTERVAL) {
            self.history.push(GlobalStep {
                wirelength: problem.wirelength(&self.poses),
                ..step_state
            });
        }
        self.last_step = step_state;
        if board.config.frame_interval > 0 && iteration.is_multiple_of(board.config.frame_interval)
        {
            self.frames.push(Frame {
                iteration,
                overflow: self.overflow,
                wirelength: problem.wirelength(&self.poses),
                poses: self.poses.clone(),
            });
        }
    }

    /// Whether the descent stops: the overflow at `stop_overflow` (after
    /// the first 20 iterations), or no progress for `stall_iterations`.
    fn stops(&mut self, iteration: usize) -> bool {
        let config = self.board.config;
        let overflow = self.overflow;
        if overflow <= config.stop_overflow && iteration > 20 {
            return true;
        }
        if overflow < self.least.0 * config.stall_factor {
            self.least = (overflow, iteration);
        }
        if config.stall_iterations > 0 && iteration >= self.least.1 + config.stall_iterations {
            if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
                eprintln!(
                    "placer: global placement stalled: overflow {overflow:.3}, its least {:.3} at iteration {} of {}",
                    self.least.0, self.least.1, self.iterations
                );
            }
            return true;
        }
        false
    }

    /// The result: the last frame and, unless kept already, the last
    /// iteration's state in the history.
    pub(super) fn result(mut self, start: Vec<Pose>, stacks: Vec<Vec<usize>>) -> GlobalResult {
        let problem = self.board.problem;
        self.frames.push(Frame {
            iteration: self.iterations,
            overflow: self.overflow,
            wirelength: problem.wirelength(&self.poses),
            poses: self.poses.clone(),
        });
        let last_step = self.last_step;
        if self
            .history
            .last()
            .is_none_or(|step: &GlobalStep| step.iteration != last_step.iteration)
        {
            self.history.push(GlobalStep {
                wirelength: problem.wirelength(&self.poses),
                ..last_step
            });
        }
        GlobalResult {
            poses: self.poses,
            frames: self.frames,
            iterations: self.iterations,
            overflow: self.overflow,
            start,
            history: self.history,
            stacks,
        }
    }
}

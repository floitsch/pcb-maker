// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Simulated-annealing legalization.
//!
//! Global placement leaves small overlaps. Resolving them greedily part by
//! part exiles whatever comes last, so instead all parts negotiate: moves,
//! quarter turns and swaps are judged on wirelength plus an overlap penalty
//! that grows until no overlap is worth keeping. It starts cold, because the
//! global placement is already good and must not be scrambled.

use crate::problem::{Point, Pose, Problem, point_in_polygon};

#[derive(Clone, Debug)]
pub struct AnnealConfig {
    pub stages: usize,
    pub moves_per_part: usize,
    /// Initial move radius as a fraction of the board's larger side.
    pub initial_radius: f64,
    pub seed: u64,
}

impl Default for AnnealConfig {
    fn default() -> Self {
        Self {
            stages: 160,
            moves_per_part: 40,
            initial_radius: 0.06,
            seed: 7,
        }
    }
}

struct Random(u64);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, bound: usize) -> usize {
        ((self.next() * bound as f64) as usize).min(bound - 1)
    }
}

#[derive(Clone, Copy)]
struct Rect {
    center: Point,
    half: Point,
}

struct State<'a> {
    problem: &'a Problem,
    poses: Vec<Pose>,
    rects: Vec<Rect>,
    members: Vec<Vec<(usize, usize)>>,
    incident: Vec<Vec<usize>>,
    bounds: [f64; 4],
    rectangular: bool,
}

impl State<'_> {
    /// The body inflated by its halo and half the spacing: two such
    /// rectangles overlap exactly when the parts are too close.
    fn rect(&self, index: usize, pose: Pose) -> Rect {
        let component = &self.problem.components[index];
        let half = component.half_extent(pose.angle);
        let margin = component.halo + self.problem.spacing / 2.0;
        Rect {
            center: component.center(pose),
            half: [half[0] + margin, half[1] + margin],
        }
    }

    fn overlap(&self, index: usize, rect: Rect) -> f64 {
        let side = self.problem.components[index].side;
        let mut total = 0.0;
        for (other, body) in self.rects.iter().enumerate() {
            if other == index || !side.collides(self.problem.components[other].side) {
                continue;
            }
            let width = rect.half[0] + body.half[0] - (rect.center[0] - body.center[0]).abs();
            let height = rect.half[1] + body.half[1] - (rect.center[1] - body.center[1]).abs();
            if width > 0.0 && height > 0.0 {
                total += width * height;
            }
        }
        // Leaving the board is as bad as overlapping something.
        let component = &self.problem.components[index];
        let margin = component.halo + self.problem.spacing / 2.0 - self.problem.edge_margin;
        let bare = [rect.half[0] - margin, rect.half[1] - margin];
        let outside_x = (self.bounds[0] - (rect.center[0] - bare[0])).max(0.0)
            + ((rect.center[0] + bare[0]) - self.bounds[2]).max(0.0);
        let outside_y = (self.bounds[1] - (rect.center[1] - bare[1])).max(0.0)
            + ((rect.center[1] + bare[1]) - self.bounds[3]).max(0.0);
        total += 2.0 * (outside_x * 2.0 * bare[1] + outside_y * 2.0 * bare[0]);
        if !self.rectangular {
            for corner in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
                let point = [
                    rect.center[0] + corner[0] * bare[0],
                    rect.center[1] + corner[1] * bare[1],
                ];
                if !point_in_polygon(point, &self.problem.outline) {
                    total += 4.0 * bare[0] * bare[1];
                }
            }
        }
        total
    }

    fn wirelength(&self, nets: &[usize]) -> f64 {
        nets.iter()
            .map(|net| {
                let pins = &self.members[*net];
                if pins.len() < 2 {
                    return 0.0;
                }
                let mut bounds = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for (component, pin) in pins {
                    let description = &self.problem.components[*component];
                    let at = description.pin_position(&description.pins[*pin], self.poses[*component]);
                    bounds[0] = bounds[0].min(at[0]);
                    bounds[1] = bounds[1].min(at[1]);
                    bounds[2] = bounds[2].max(at[0]);
                    bounds[3] = bounds[3].max(at[1]);
                }
                self.problem.net_weights[*net] * ((bounds[2] - bounds[0]) + (bounds[3] - bounds[1]))
            })
            .sum()
    }
}

fn snap(value: f64, grid: f64) -> f64 {
    if grid > 0.0 {
        (value / grid).round() * grid
    } else {
        value
    }
}

/// Improves `poses` in place and returns the remaining overlap area.
pub fn anneal(problem: &Problem, poses: &mut Vec<Pose>, config: &AnnealConfig) -> f64 {
    let movable: Vec<usize> = (0..problem.components.len())
        .filter(|index| !problem.components[*index].fixed)
        .collect();
    if movable.is_empty() {
        return 0.0;
    }
    let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); problem.net_weights.len()];
    for (index, component) in problem.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            members[description.net].push((index, pin));
        }
    }
    let incident = problem
        .components
        .iter()
        .map(|component| {
            let mut nets: Vec<usize> = component.pins.iter().map(|pin| pin.net).collect();
            nets.sort_unstable();
            nets.dedup();
            nets
        })
        .collect();
    let bounds = problem.bounds();
    let area: f64 = {
        let outline = &problem.outline;
        (0..outline.len())
            .map(|index| {
                let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            .abs()
            / 2.0
    };
    let rectangular =
        (area - (bounds[2] - bounds[0]) * (bounds[3] - bounds[1])).abs() < 1.0e-6 * area.max(1.0);
    for index in &movable {
        poses[*index].position = [
            snap(poses[*index].position[0], problem.grid),
            snap(poses[*index].position[1], problem.grid),
        ];
    }
    let mut state = State {
        problem,
        poses: poses.clone(),
        rects: Vec::new(),
        members,
        incident,
        bounds,
        rectangular,
    };
    state.rects = (0..problem.components.len())
        .map(|index| state.rect(index, state.poses[index]))
        .collect();

    let mut random = Random(config.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let board = (bounds[2] - bounds[0]).max(bounds[3] - bounds[1]);
    let grid = if problem.grid > 0.0 { problem.grid } else { 0.05 };
    let mut radius = (config.initial_radius * board).max(2.0 * grid);
    // Millimetres of wirelength one square millimetre of overlap is worth.
    let mut penalty = 0.5;
    let mut temperature = 0.3;
    let penalty_growth = (400.0f64 / penalty).powf(1.0 / config.stages as f64);
    let radius_decay = ((1.5 * grid) / radius).powf(1.0 / config.stages as f64);

    for _ in 0..config.stages {
        for _ in 0..config.moves_per_part * movable.len() {
            let index = movable[random.below(movable.len())];
            let component = &problem.components[index];
            let kind = random.next();
            if kind < 0.12 && movable.len() > 1 {
                // Swap two parts, keeping each at the other's body centre.
                let other = movable[random.below(movable.len())];
                if other == index {
                    continue;
                }
                let second = &problem.components[other];
                let (first_pose, second_pose) = (state.poses[index], state.poses[other]);
                let first_new = Pose {
                    position: component
                        .position_for_center(second.center(second_pose), first_pose.angle),
                    angle: first_pose.angle,
                };
                let second_new = Pose {
                    position: second
                        .position_for_center(component.center(first_pose), second_pose.angle),
                    angle: second_pose.angle,
                };
                let snapped = |pose: Pose| Pose {
                    position: [snap(pose.position[0], problem.grid), snap(pose.position[1], problem.grid)],
                    angle: pose.angle,
                };
                let (first_new, second_new) = (snapped(first_new), snapped(second_new));
                let mut nets = state.incident[index].clone();
                nets.extend(&state.incident[other]);
                nets.sort_unstable();
                nets.dedup();
                let before = state.wirelength(&nets)
                    + penalty
                        * (state.overlap(index, state.rects[index])
                            + state.overlap(other, state.rects[other]));
                let saved = (state.rects[index], state.rects[other]);
                state.poses[index] = first_new;
                state.poses[other] = second_new;
                state.rects[index] = state.rect(index, first_new);
                state.rects[other] = state.rect(other, second_new);
                let after = state.wirelength(&nets)
                    + penalty
                        * (state.overlap(index, state.rects[index])
                            + state.overlap(other, state.rects[other]));
                let delta = after - before;
                if delta > 0.0 && random.next() >= (-delta / temperature).exp() {
                    state.poses[index] = first_pose;
                    state.poses[other] = second_pose;
                    state.rects[index] = saved.0;
                    state.rects[other] = saved.1;
                }
                continue;
            }
            let old = state.poses[index];
            let new = if kind < 0.27 && component.angle_options.len() > 1 {
                let angle = component.angle_options[random.below(component.angle_options.len())];
                if (angle - old.angle).abs() < 1.0e-9 {
                    continue;
                }
                let position = component.position_for_center(component.center(old), angle);
                Pose {
                    position: [snap(position[0], problem.grid), snap(position[1], problem.grid)],
                    angle,
                }
            } else {
                let step = |random: &mut Random| {
                    let cells = (radius / grid).max(1.0);
                    ((random.next() * 2.0 - 1.0) * cells).round() * grid
                };
                let position = [
                    old.position[0] + step(&mut random),
                    old.position[1] + step(&mut random),
                ];
                if position == old.position {
                    continue;
                }
                Pose {
                    position,
                    angle: old.angle,
                }
            };
            let before = state.wirelength(&state.incident[index])
                + penalty * state.overlap(index, state.rects[index]);
            let saved = state.rects[index];
            state.poses[index] = new;
            state.rects[index] = state.rect(index, new);
            let after = state.wirelength(&state.incident[index])
                + penalty * state.overlap(index, state.rects[index]);
            let delta = after - before;
            if delta > 0.0 && random.next() >= (-delta / temperature).exp() {
                state.poses[index] = old;
                state.rects[index] = saved;
            }
        }
        penalty *= penalty_growth;
        radius *= radius_decay;
        temperature *= 0.975;
    }
    *poses = state.poses.clone();
    movable
        .iter()
        .map(|index| state.overlap(*index, state.rects[*index]))
        .sum::<f64>()
        / 2.0
}

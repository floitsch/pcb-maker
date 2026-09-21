// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! PCB component placement: electrostatic global placement, legalization,
//! and wirelength-driven detailed placement.

pub mod anneal;
pub mod global;
pub mod legal;
pub mod problem;
pub mod view;

pub use global::{Frame, GlobalConfig};
pub use problem::{Component, Pin, Point, Pose, Problem, Side};

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub global: GlobalConfig,
    pub anneal: anneal::AnnealConfig,
    pub refine_passes: usize,
    /// Halos shrink until bodies plus margins fit into this share of the
    /// free board area.
    pub maximum_utilization: f64,
}

impl Config {
    pub fn new() -> Self {
        Self {
            global: GlobalConfig::default(),
            anneal: anneal::AnnealConfig::default(),
            refine_passes: 8,
            maximum_utilization: 0.6,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Placement {
    pub poses: Vec<Pose>,
    pub frames: Vec<Frame>,
    pub global_iterations: usize,
    pub global_overflow: f64,
    pub wirelength_initial: f64,
    pub wirelength_global: f64,
    pub wirelength_legal: f64,
    pub wirelength_final: f64,
    /// Components for which no legal position was found.
    pub unplaced: Vec<usize>,
    /// Components violating spacing or the outline in the final result.
    pub illegal: Vec<usize>,
}

/// Area of the outline polygon.
fn outline_area(problem: &Problem) -> f64 {
    let outline = &problem.outline;
    (0..outline.len())
        .map(|index| {
            let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

/// Shrinks the routing halos until bodies, halos and spacing together need
/// no more than `limit` of the free board area. Halos are a wish; a crowded
/// board cannot afford them in full.
fn fit_halos(problem: &Problem, limit: f64) -> Problem {
    let demand = |problem: &Problem, scale: f64, fixed: bool| -> f64 {
        problem
            .components
            .iter()
            .filter(|component| component.fixed == fixed)
            .map(|component| {
                let margin = if fixed {
                    0.0
                } else {
                    2.0 * scale * component.halo + problem.spacing
                };
                (component.body_size[0] + margin) * (component.body_size[1] + margin)
            })
            .sum()
    };
    let free = (outline_area(problem) - demand(problem, 0.0, true)).max(1.0e-9);
    let mut scale = 1.0;
    while scale > 0.0 && demand(problem, scale, false) / free > limit {
        scale -= 0.125;
    }
    let mut fitted = problem.clone();
    for component in &mut fitted.components {
        component.halo *= scale.max(0.0);
    }
    fitted
}

pub fn place(problem: &Problem, config: &Config) -> Placement {
    let fitted = fit_halos(problem, config.maximum_utilization);
    let problem = &fitted;
    let wirelength_initial = problem.wirelength(&problem.poses);
    let global = global::global_place(problem, &config.global);
    let global_poses = global.poses;
    let wirelength_global = problem.wirelength(&global_poses);
    let mut frames = global.frames;

    // Crowded boards cannot afford the full comfort margins: give up halos,
    // then the spacing, before giving up on a part.
    let mut best: Option<(Vec<usize>, Vec<Pose>, Problem)> = None;
    for (halo_scale, spacing_scale) in [(1.0, 1.0), (0.5, 1.0), (0.0, 1.0), (0.0, 0.0)] {
        let mut relaxed = problem.clone();
        relaxed.spacing *= spacing_scale;
        for component in &mut relaxed.components {
            component.halo *= halo_scale;
        }
        let mut poses = global_poses.clone();
        anneal::anneal(&relaxed, &mut poses, &config.anneal);
        let failed = legal::legalize(&relaxed, &mut poses);
        let better = best
            .as_ref()
            .is_none_or(|(unplaced, _, _)| failed.len() < unplaced.len());
        let done = failed.is_empty();
        if better {
            best = Some((failed, poses, relaxed));
        }
        if done {
            break;
        }
    }
    let (unplaced, mut poses, relaxed) = best.expect("at least one relaxation level ran");
    let wirelength_legal = problem.wirelength(&poses);
    frames.push(Frame {
        iteration: global.iterations + 1,
        overflow: global.overflow,
        wirelength: wirelength_legal,
        poses: poses.clone(),
    });
    let problem = &relaxed;
    legal::refine(problem, &mut poses, config.refine_passes);
    let wirelength_final = problem.wirelength(&poses);
    frames.push(Frame {
        iteration: global.iterations + 2,
        overflow: global.overflow,
        wirelength: wirelength_final,
        poses: poses.clone(),
    });
    let illegal = legal::illegal_components(problem, &poses);
    Placement {
        poses,
        frames,
        global_iterations: global.iterations,
        global_overflow: global.overflow,
        wirelength_initial,
        wirelength_global,
        wirelength_legal,
        wirelength_final,
        unplaced,
        illegal,
    }
}

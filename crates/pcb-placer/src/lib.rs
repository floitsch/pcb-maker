// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! PCB component placement: electrostatic global placement, legalization,
//! and wirelength-driven detailed placement.

pub mod global;
pub mod legal;
pub mod problem;
pub mod view;

pub use global::{Frame, GlobalConfig};
pub use problem::{Component, Pin, Point, Pose, Problem, Side};

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub global: GlobalConfig,
    pub refine_passes: usize,
}

impl Config {
    pub fn new() -> Self {
        Self {
            global: GlobalConfig::default(),
            refine_passes: 8,
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

pub fn place(problem: &Problem, config: &Config) -> Placement {
    let wirelength_initial = problem.wirelength(&problem.poses);
    let global = global::global_place(problem, &config.global);
    let mut poses = global.poses;
    let global_poses = poses.clone();
    let mut spacing = problem.spacing;
    let wirelength_global = problem.wirelength(&poses);
    let mut frames = global.frames;
    let mut unplaced = legal::legalize(problem, &mut poses);
    if !unplaced.is_empty() && problem.spacing > 0.0 {
        // A crowded board: give up the comfort gap before giving up.
        let mut tight = problem.clone();
        tight.spacing = 0.0;
        let mut retry = global_poses.clone();
        let failed = legal::legalize(&tight, &mut retry);
        if failed.len() < unplaced.len() {
            poses = retry;
            unplaced = failed;
            spacing = 0.0;
        }
    }
    let wirelength_legal = problem.wirelength(&poses);
    frames.push(Frame {
        iteration: global.iterations + 1,
        overflow: global.overflow,
        wirelength: wirelength_legal,
        poses: poses.clone(),
    });
    let mut problem = problem.clone();
    problem.spacing = spacing;
    let problem = &problem;
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

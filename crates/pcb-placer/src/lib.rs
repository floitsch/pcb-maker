// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! PCB component placement: electrostatic global placement, legalization,
//! and wirelength-driven detailed placement.

pub mod anneal;
pub mod constraints;
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
    /// Every placement constraint with whether the result keeps it.
    pub constraints: Vec<constraints::Status>,
    /// The relaxation the placement needed: work on it (moves after
    /// routing) must keep to the same rules.
    pub relaxation: Relaxation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Relaxation {
    /// Spacing between bodies and the placement grid.
    pub spacing: f64,
    pub grid: f64,
    /// Factor on every part's halo.
    pub halo_scale: f64,
    /// Bodies come closer to the edge as far as their copper allows.
    pub edge_inset: bool,
    /// Bodies shrank to their tight boxes (courtyards overlap).
    pub tight: bool,
    /// Parts held at an edge are on the board when their copper is.
    pub edge_copper: bool,
    /// Movable bodies keep only the rules' copper-to-edge clearance from
    /// the edge (not the placement's larger margin).
    pub edge_rule: f64,
}

impl Relaxation {
    /// Applies the relaxation to `problem`, whose halos are unscaled.
    pub fn apply(&self, problem: &mut Problem) {
        problem.spacing = self.spacing;
        problem.grid = self.grid;
        for component in &mut problem.components {
            component.halo *= self.halo_scale;
            if !self.edge_inset {
                component.edge_inset = 0.0;
            }
            if self.tight {
                component.use_tight_body();
            }
        }
        problem.constraints.edge_copper = self.edge_copper;
        problem.edge_margin = problem.edge_margin.min(self.edge_rule);
    }
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
fn fit_halos(problem: &Problem, limit: f64) -> (Problem, f64) {
    let demand = |problem: &Problem, scale: f64, fixed: bool| -> f64 {
        problem
            .components
            .iter()
            .filter(|component| component.fixed == fixed && component.side != Side::Neither)
            .map(|component| {
                let margin = if fixed {
                    0.0
                } else {
                    2.0 * scale * component.halo + problem.spacing
                };
                if component.hollow.is_empty() {
                    (component.body_size[0] + margin) * (component.body_size[1] + margin)
                } else {
                    component.blocking_area()
                }
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
    (fitted, scale.max(0.0))
}

pub fn place(problem: &Problem, config: &Config) -> Placement {
    let (mut fitted, fit_scale) = fit_halos(problem, config.maximum_utilization);
    fitted.constraints.link_pairs();
    let problem = &fitted;
    let wirelength_initial = problem.wirelength(&problem.poses);
    let debug = std::env::var_os("PCB_PLACER_DEBUG").is_some();
    let started = std::time::Instant::now();
    let global = global::global_place(problem, &config.global);
    if debug {
        eprintln!("placer: global {:.1}s", started.elapsed().as_secs_f64());
    }
    let global_poses = global.poses;
    let wirelength_global = problem.wirelength(&global_poses);
    let mut frames = global.frames;

    // Crowded boards cannot afford the full comfort margins: give up halos,
    // then the spacing, before giving up on a part.
    let mut best: Option<(Vec<usize>, Vec<Pose>, Problem)> = None;
    // The last level also snaps to a finer grid: on small, crowded boards
    // the regular grid leaves no legal spot where a finer one does.
    let fine = problem.grid.min(0.1);
    // Then bodies may come closer to the edge, as far as their copper
    // allows (the full margin is room for routing too). Last, where the
    // board's rules let courtyards overlap: bodies without the courtyard's
    // margin.
    let inset = problem.components.iter().any(|component| component.edge_inset > 0.0);
    let tight = problem.components.iter().any(|component| component.tight.is_some());
    let mut levels = vec![
        (1.0, 1.0, problem.grid, false, false),
        (0.5, 1.0, problem.grid, false, false),
        (0.0, 1.0, problem.grid, false, false),
        (0.0, 0.0, problem.grid, false, false),
        (0.0, 0.0, fine, false, false),
    ];
    if inset {
        levels.push((0.0, 0.0, fine, true, false));
    }
    if tight {
        levels.push((0.0, 0.0, fine, true, true));
    }
    // Then bodies keep only the rules' copper-to-edge clearance from the
    // edge (a narrow board whose parts' copper barely fits). Last, parts
    // held at an edge only need their copper on the board (a card edge in
    // its tab).
    let rule_margin = (problem.constraints.copper_edge + 0.05).min(problem.edge_margin);
    let mut levels: Vec<(f64, f64, f64, bool, bool, bool, f64)> = levels
        .into_iter()
        .map(|(halo, spacing, grid, inset, tight)| (halo, spacing, grid, inset, tight, false, problem.edge_margin))
        .collect();
    if rule_margin < problem.edge_margin {
        levels.push((0.0, 0.0, fine, true, tight, false, rule_margin));
    }
    if !problem.constraints.edges.is_empty() {
        levels.push((0.0, 0.0, fine, true, tight, true, rule_margin));
    }
    let mut relaxation = None;
    for (halo_scale, spacing_scale, grid, inset, tight, edge_copper, edge_rule) in levels {
        // Past the caller's deadline the best placement so far stands.
        if best.is_some() && config.anneal.deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            break;
        }
        if grid == fine && fine == problem.grid && spacing_scale == 0.0 && !inset && best.is_some() {
            // Same as the previous level.
            continue;
        }
        let mut relaxed = problem.clone();
        relaxed.grid = grid;
        relaxed.spacing = (relaxed.spacing * spacing_scale).max(relaxed.min_spacing);
        relaxed.constraints.edge_copper = edge_copper;
        relaxed.edge_margin = relaxed.edge_margin.min(edge_rule);
        for component in &mut relaxed.components {
            component.halo *= halo_scale;
            if !inset {
                component.edge_inset = 0.0;
            }
            if tight {
                component.use_tight_body();
            }
        }
        let level_started = std::time::Instant::now();
        // Every level is looser than the one before: what was legal there
        // stays legal. First only the parts that failed look for room.
        if let Some((known_failed, known_poses, _)) = best.as_ref()
            && !known_failed.is_empty()
        {
            let mut kept = known_poses.clone();
            let keep: Vec<usize> = (0..relaxed.components.len()).filter(|index| !known_failed.contains(index)).collect();
            let failed = legal::legalize_keeping(&relaxed, &mut kept, known_failed, &keep);
            if debug {
                eprintln!(
                    "placer: level halo {halo_scale} spacing {spacing_scale} grid {grid} inset {inset} tight {tight}: kept placement, {:.1}s, {} failed",
                    level_started.elapsed().as_secs_f64(),
                    failed.len()
                );
            }
            if failed.len() < known_failed.len() {
                relaxation = Some(Relaxation {
                    spacing: relaxed.spacing,
                    grid,
                    halo_scale: fit_scale * halo_scale,
                    edge_inset: inset,
                    tight,
                    edge_copper,
                    edge_rule,
                });
                let done = failed.is_empty();
                best = Some((failed, kept, relaxed.clone()));
                if done {
                    break;
                }
            }
        }
        let mut poses = global_poses.clone();
        let level_started = std::time::Instant::now();
        anneal::anneal(&relaxed, &mut poses, &config.anneal);
        let annealed = poses.clone();
        let anneal_seconds = level_started.elapsed().as_secs_f64();
        let mut failed = legal::legalize(&relaxed, &mut poses);
        if debug {
            eprintln!(
                "placer: level halo {halo_scale} spacing {spacing_scale} grid {grid} inset {inset} tight {tight}: anneal {anneal_seconds:.1}s, legalize {:.1}s, {} failed",
                level_started.elapsed().as_secs_f64() - anneal_seconds,
                failed.len()
            );
        }
        // The parts that found no room go first in a second pass.
        if !failed.is_empty() {
            let mut retry = annealed;
            let again = legal::legalize_first(&relaxed, &mut retry, &failed);
            if again.len() < failed.len() {
                poses = retry;
                failed = again;
            }
        }
        let better = best
            .as_ref()
            .is_none_or(|(unplaced, _, _)| failed.len() < unplaced.len());
        let done = failed.is_empty();
        if better {
            relaxation = Some(Relaxation {
                spacing: relaxed.spacing,
                grid,
                halo_scale: fit_scale * halo_scale,
                edge_inset: inset,
                tight,
                edge_copper,
                edge_rule,
            });
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
    legal::center_edge_copper(problem, &mut poses);
    // Moving one part can break a relation repaired before; repeat while
    // it helps.
    for _ in 0..5 {
        if legal::repair_relations(problem, &mut poses) == 0 {
            break;
        }
        legal::refine(problem, &mut poses, config.refine_passes);
    }
    // Parts of a kind turned alike where it costs next to nothing.
    legal::align_orientations(problem, &mut poses, 0.5);
    let wirelength_final = problem.wirelength(&poses);
    frames.push(Frame {
        iteration: global.iterations + 2,
        overflow: global.overflow,
        wirelength: wirelength_final,
        poses: poses.clone(),
    });
    problem.sync_followers(&mut poses);
    let illegal = legal::illegal_components(problem, &poses);
    let constraints = constraints::report(problem, &poses);
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
        constraints,
        relaxation: relaxation.expect("at least one relaxation level ran"),
    }
}

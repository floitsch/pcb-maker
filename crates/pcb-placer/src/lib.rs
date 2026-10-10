// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! PCB component placement: electrostatic global placement, legalization,
//! and wirelength-driven detailed placement.

pub mod anneal;
mod buckets;
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
    /// Where courtyards may overlap (the tight levels), let a part's body
    /// reach over other parts' pads on its side, keeping only copper apart
    /// (katia's connectors partly over the switches' hot-swap socket pads,
    /// as its designer placed them). Off by default: a socket is a body.
    pub overlap_far_side_pads: bool,
    /// Placement work (see `work`) after which no further level or anneal
    /// stage starts; the best placement so far stands.
    pub work_limit: Option<u64>,
}

impl Config {
    pub fn new() -> Self {
        Self {
            global: GlobalConfig::default(),
            anneal: anneal::AnnealConfig::default(),
            refine_passes: 8,
            maximum_utilization: 0.6,
            overlap_far_side_pads: false,
            work_limit: None,
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
    /// The placement work done (see `work`).
    pub work: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Relaxation {
    /// Spacing between bodies and the placement grid.
    pub spacing: f64,
    pub grid: f64,
    /// Factor on the halos of parts with many pins (`MANY_PINS`).
    pub halo_scale: f64,
    /// Factor on the other parts' halos: they shrink first, as the many-pin
    /// parts' escapes need the room most.
    pub small_halo_scale: f64,
    /// Bodies come closer to the edge as far as their copper allows.
    pub edge_inset: bool,
    /// Bodies shrank to their tight boxes (courtyards overlap).
    pub tight: bool,
    /// Parts held at an edge are on the board when their copper is.
    pub edge_copper: bool,
    /// Movable bodies keep only the rules' copper-to-edge clearance from
    /// the edge (not the placement's larger margin).
    pub edge_rule: f64,
    /// Courtyards may touch where their copper keeps the clearance
    /// (`Constraints::courtyard_spacing`).
    pub courtyard_spacing: bool,
}

impl Relaxation {
    /// Applies the relaxation to `problem`, whose halos are unscaled.
    pub fn apply(&self, problem: &mut Problem) {
        problem.spacing = self.spacing;
        problem.grid = self.grid;
        for component in &mut problem.components {
            component.halo *= if component.pins.len() >= MANY_PINS { self.halo_scale } else { self.small_halo_scale };
            if !self.edge_inset {
                component.edge_inset = 0.0;
            }
            if self.tight {
                component.use_tight_body();
            }
        }
        problem.constraints.edge_copper = self.edge_copper;
        problem.constraints.courtyard_spacing = self.courtyard_spacing;
        problem.edge_margin = problem.edge_margin.min(self.edge_rule);
    }
}

/// Parts with at least this many pins keep their halos one level longer.
const MANY_PINS: usize = 10;

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

/// Whether the parts on a side need more area than the board has, bodies
/// and spacing alone (halos are a wish; cutouts take no room): no packing
/// can place them all.
fn overfull(problem: &Problem) -> bool {
    let mut used = [0.0f64; 2];
    for component in &problem.components {
        if component.copper_only {
            continue;
        }
        let area = if !component.hollow.is_empty() {
            component.blocking_area()
        } else if component.fixed {
            component.body_size[0] * component.body_size[1]
        } else {
            (component.body_size[0] + problem.spacing) * (component.body_size[1] + problem.spacing)
        };
        match component.side {
            Side::Front => used[0] += area,
            Side::Back => used[1] += area,
            Side::Both => {
                used[0] += area;
                used[1] += area;
            }
            Side::Neither => {}
        }
    }
    let area = outline_area(problem);
    used.iter().any(|used| *used > area)
}

/// The board area per side (front, back) that fixed parts leave free, on
/// a coarse raster: overlapping fixed bodies (a copper graphic and its
/// mask twin) count once; cutouts take no room from bodies; a hollow
/// part blocks with its boxes only.
fn free_areas(problem: &Problem) -> [f64; 2] {
    let bounds = problem.bounds();
    let (width, height) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
    let cell = (width.max(height) / 200.0).max(1.0e-6);
    let (nx, ny) = ((width / cell).ceil().max(1.0) as usize, (height / cell).ceil().max(1.0) as usize);
    let mut covered = [vec![false; nx * ny], vec![false; nx * ny]];
    for (component, pose) in problem.components.iter().zip(&problem.poses) {
        if !component.fixed || component.copper_only || component.side == Side::Neither {
            continue;
        }
        let boxes = if component.hollow.is_empty() {
            vec![(component.center(*pose), component.half_extent(pose.angle))]
        } else {
            component.hollow_boxes(*pose)
        };
        let sides: &[usize] = match component.side {
            Side::Front => &[0],
            Side::Back => &[1],
            _ => &[0, 1],
        };
        for (center, half) in boxes {
            let x0 = (((center[0] - half[0] - bounds[0]) / cell).floor().max(0.0) as usize).min(nx - 1);
            let x1 = (((center[0] + half[0] - bounds[0]) / cell).ceil().max(0.0) as usize).min(nx);
            let y0 = (((center[1] - half[1] - bounds[1]) / cell).floor().max(0.0) as usize).min(ny - 1);
            let y1 = (((center[1] + half[1] - bounds[1]) / cell).ceil().max(0.0) as usize).min(ny);
            for y in y0..y1 {
                for x in x0..x1 {
                    for side in sides {
                        covered[*side][y * nx + x] = true;
                    }
                }
            }
        }
    }
    let mut free = [0.0; 2];
    for y in 0..ny {
        for x in 0..nx {
            let point = [bounds[0] + (x as f64 + 0.5) * cell, bounds[1] + (y as f64 + 0.5) * cell];
            if !crate::problem::point_in_polygon(point, &problem.outline) {
                continue;
            }
            for side in 0..2 {
                if !covered[side][y * nx + x] {
                    free[side] += cell * cell;
                }
            }
        }
    }
    free
}

/// Shrinks the routing halos until each side's movable bodies, halos and
/// spacing need no more than `limit` of the area fixed parts leave free
/// there: the small parts' halos first, then those of parts with many pins
/// (their escapes need the room most). Halos are a wish; a crowded board
/// cannot afford them in full. Returns the scales of the small and of the
/// many-pin parts' halos. (Counting both sides against one side's area,
/// and cutouts as taken, left Sisu, link and katia without any halo.)
fn fit_halos(problem: &Problem, limit: f64) -> (Problem, [f64; 2]) {
    let scale_of = |component: &crate::problem::Component, scales: [f64; 2]| {
        if component.pins.len() >= MANY_PINS { scales[1] } else { scales[0] }
    };
    let free = free_areas(problem).map(|area| area.max(1.0e-9));
    let utilization = |scales: [f64; 2]| -> f64 {
        let mut used = [0.0f64; 2];
        for component in &problem.components {
            if component.fixed || component.copper_only || component.side == Side::Neither {
                continue;
            }
            let margin = 2.0 * scale_of(component, scales) * component.halo + problem.spacing;
            let area = if component.hollow.is_empty() {
                (component.body_size[0] + margin) * (component.body_size[1] + margin)
            } else {
                component.blocking_area()
            };
            match component.side {
                Side::Front => used[0] += area,
                Side::Back => used[1] += area,
                _ => {
                    used[0] += area;
                    used[1] += area;
                }
            }
        }
        (used[0] / free[0]).max(used[1] / free[1])
    };
    let mut scales = [1.0f64, 1.0];
    for which in 0..2 {
        while scales[which] > 0.0 && utilization(scales) > limit {
            scales[which] -= 0.125;
        }
        scales[which] = scales[which].max(0.0);
    }
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!(
            "placer: halos fit at {scales:?}: free {:.0} / {:.0} mm2; bodies and spacing {:.2}, full halos {:.2}, many-pin halos only {:.2}",
            free[0],
            free[1],
            utilization([0.0, 0.0]),
            utilization([1.0, 1.0]),
            utilization([0.0, 1.0]),
        );
    }
    let mut fitted = problem.clone();
    for component in &mut fitted.components {
        component.halo *= scale_of(component, scales);
    }
    (fitted, scales)
}

thread_local! {
    /// Placement work done on this thread: legality checks and anneal
    /// moves. Budgets count it rather than seconds, so that a busy or a
    /// slower machine places the same way.
    static WORK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Placement work one thread does per second on an idle machine: budgets
/// in seconds become work limits at this rate.
pub const WORK_PER_SECOND: f64 = 150.0e6;

/// The placement work done on this thread so far.
pub fn work() -> u64 {
    WORK.with(|work| work.get())
}

pub(crate) fn add_work(amount: u64) {
    WORK.with(|work| work.set(work.get() + amount));
}

pub fn place(problem: &Problem, config: &Config) -> Placement {
    let (mut fitted, fit_scale) = fit_halos(problem, config.maximum_utilization);
    fitted.constraints.link_pairs();
    let problem = &fitted;
    let wirelength_initial = problem.wirelength(&problem.poses);
    let debug = std::env::var_os("PCB_PLACER_DEBUG").is_some();
    let started = std::time::Instant::now();
    let work_started = work();
    let stop_at_work = config.work_limit.map(|limit| work_started + limit);
    let mut anneal_config = config.anneal.clone();
    anneal_config.stop_at_work = stop_at_work.or(anneal_config.stop_at_work);
    let out_of_budget = || {
        config.anneal.deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline)
            || anneal_config.stop_at_work.is_some_and(|stop| work() >= stop)
    };
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
    // Halos go in steps: halved, then only the many-pin parts' (whose
    // escapes need the room most), then all. The last number is the factor
    // on the other parts' halos.
    let mut levels = vec![
        (1.0, 1.0, problem.grid, false, false, 1.0),
        (0.5, 1.0, problem.grid, false, false, 1.0),
        (0.5, 1.0, problem.grid, false, false, 0.0),
        (0.0, 1.0, problem.grid, false, false, 0.0),
        (0.0, 0.0, problem.grid, false, false, 0.0),
        (0.0, 0.0, fine, false, false, 0.0),
    ];
    if inset {
        levels.push((0.0, 0.0, fine, true, false, 0.0));
    }
    if tight {
        levels.push((0.0, 0.0, fine, true, true, 0.0));
    }
    // Then bodies keep only the rules' copper-to-edge clearance from the
    // edge (a narrow board whose parts' copper barely fits). Last, parts
    // held at an edge only need their copper on the board (a card edge in
    // its tab).
    let rule_margin = (problem.constraints.copper_edge + 0.05).min(problem.edge_margin);
    let mut levels: Vec<(f64, f64, f64, bool, bool, bool, f64, f64)> = levels
        .into_iter()
        .map(|(halo, spacing, grid, inset, tight, small)| (halo, spacing, grid, inset, tight, false, problem.edge_margin, small))
        .collect();
    if rule_margin < problem.edge_margin {
        levels.push((0.0, 0.0, fine, true, tight, false, rule_margin, 0.0));
    }
    if !problem.constraints.edges.is_empty() {
        levels.push((0.0, 0.0, fine, true, tight, true, rule_margin, 0.0));
    }
    // Last, courtyards may touch where the copper inside them keeps the
    // clearance: how designers pack a crowded side.
    let courtyard_level = levels.len();
    {
        let last = *levels.last().expect("levels");
        levels.push(last);
    }
    let mut relaxation = None;
    // The grid and bodies the last anneal ran with.
    let mut annealed_shape: Option<(f64, bool)> = None;
    let level_count = levels.len();
    for (level, (halo_scale, spacing_scale, grid, inset, tight, edge_copper, edge_rule, small_factor)) in
        levels.into_iter().enumerate()
    {
        // Past the caller's deadline the best placement so far stands.
        if best.is_some() && out_of_budget() {
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
        relaxed.constraints.courtyard_spacing = level == courtyard_level;
        relaxed.far_side_pads_only = tight && config.overlap_far_side_pads;
        relaxed.edge_margin = relaxed.edge_margin.min(edge_rule);
        for component in &mut relaxed.components {
            component.halo *= halo_scale;
            if component.pins.len() < MANY_PINS {
                component.halo *= small_factor;
            }
            if !inset {
                component.edge_inset = 0.0;
            }
            if tight {
                component.use_tight_body();
            }
        }
        // Bodies that need more room than a side has cannot all be placed
        // at this level: on to a looser one (OpenESC's boards, packed with
        // overlapping courtyards, fit only with tight bodies; every level
        // before them annealed and legalized in vain, 1-4 minutes each).
        if level + 1 < level_count && overfull(&relaxed) {
            if debug {
                eprintln!(
                    "placer: level halo {halo_scale} (small {small_factor}) spacing {spacing_scale} grid {grid} inset {inset} tight {tight}: bodies need more room than a side has"
                );
            }
            continue;
        }
        let level_started = std::time::Instant::now();
        // Every level is looser than the one before: what was legal there
        // stays legal. First only the parts that failed look for room.
        if let Some((known_failed, known_poses, _)) = best.as_ref()
            && !known_failed.is_empty()
        {
            let mut kept = known_poses.clone();
            let keep: Vec<usize> = (0..relaxed.components.len()).filter(|index| !known_failed.contains(index)).collect();
            let mut failed = legal::legalize_keeping(&relaxed, &mut kept, known_failed, &keep);
            if !failed.is_empty() && failed.len() <= (problem.components.len() / 50).max(3) {
                let mut evicted = kept.clone();
                let again = legal::evict_for(&relaxed, &mut evicted, &failed);
                if again.len() < failed.len() {
                    if debug {
                        eprintln!("placer: eviction seated {} of {} parts", failed.len() - again.len(), failed.len());
                    }
                    kept = evicted;
                    failed = again;
                }
            }
            if debug {
                eprintln!(
                    "placer: level halo {halo_scale} (small {small_factor}) spacing {spacing_scale} grid {grid} inset {inset} tight {tight}: kept placement, {:.1}s, {} failed",
                    level_started.elapsed().as_secs_f64(),
                    failed.len()
                );
            }
            if failed.len() < known_failed.len() {
                relaxation = Some(Relaxation {
                    spacing: relaxed.spacing,
                    grid,
                    halo_scale: fit_scale[1] * halo_scale,
                    small_halo_scale: fit_scale[0] * halo_scale * small_factor,
                    edge_inset: inset,
                    tight,
                    edge_copper,
                    edge_rule,
                    courtyard_spacing: level == courtyard_level,
                });
                let done = failed.is_empty();
                best = Some((failed, kept, relaxed.clone()));
                // On a finer grid or with tighter bodies, a fresh anneal
                // may still beat the kept placement, whose last parts sit
                // wherever room was left (OpenESC 30x30: 1504 mm legal
                // against 881 for another seed's anneal).
                if done && (annealed_shape == Some((grid, tight)) || out_of_budget()) {
                    break;
                }
            }
        }
        // A few parts still without room: the looser levels are what they
        // need, not another full anneal at each one (katia: four parts
        // failed at every level, 40-160 s each, and the deadline came
        // before the levels that might seat them).
        // A level with a finer grid or tighter bodies is a new problem,
        // though: its anneal seats what the coarser levels could not
        // (OpenESC's mini: 2-4 parts without room at the regular grid, all
        // seated by the anneal on the fine grid).
        if let Some((known_failed, _, _)) = best.as_ref()
            && !known_failed.is_empty()
            && known_failed.len() <= (problem.components.len() / 50).max(3)
            && annealed_shape == Some((grid, tight))
        {
            continue;
        }
        annealed_shape = Some((grid, tight));
        let mut poses = global_poses.clone();
        let level_started = std::time::Instant::now();
        anneal::anneal(&relaxed, &mut poses, &anneal_config);
        let annealed = poses.clone();
        let anneal_seconds = level_started.elapsed().as_secs_f64();
        let mut failed = legal::legalize(&relaxed, &mut poses);
        if debug {
            eprintln!(
                "placer: level halo {halo_scale} (small {small_factor}) spacing {spacing_scale} grid {grid} inset {inset} tight {tight}: wirelength global {:.0}, annealed {:.0}, legalized {:.0}; anneal {anneal_seconds:.1}s, legalize {:.1}s, {} failed, work {} ({:.0}/s overall)",
                problem.wirelength(&global_poses),
                problem.wirelength(&annealed),
                problem.wirelength(&poses),
                level_started.elapsed().as_secs_f64() - anneal_seconds,
                failed.len(),
                work() - work_started,
                (work() - work_started) as f64 / started.elapsed().as_secs_f64().max(1.0e-3)
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
        // A few still without room: take a spot from movable parts and find
        // those new ones.
        if !failed.is_empty() && failed.len() <= (problem.components.len() / 50).max(3) {
            let mut evicted = poses.clone();
            let again = legal::evict_for(&relaxed, &mut evicted, &failed);
            if again.len() < failed.len() {
                if debug {
                    eprintln!("placer: eviction seated {} of {} parts", failed.len() - again.len(), failed.len());
                }
                poses = evicted;
                failed = again;
            }
        }
        let better = best.as_ref().is_none_or(|(unplaced, kept, _)| {
            failed.len() < unplaced.len()
                || (failed.len() == unplaced.len() && problem.wirelength(&poses) < problem.wirelength(kept))
        });
        let done = failed.is_empty();
        if better {
            relaxation = Some(Relaxation {
                spacing: relaxed.spacing,
                grid,
                halo_scale: fit_scale[1] * halo_scale,
                small_halo_scale: fit_scale[0] * halo_scale * small_factor,
                edge_inset: inset,
                tight,
                edge_copper,
                edge_rule,
                courtyard_spacing: level == courtyard_level,
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
    let finishing = std::time::Instant::now();
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
    if debug {
        eprintln!("placer: refinement and relation repair {:.1}s", finishing.elapsed().as_secs_f64());
    }
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
        work: work() - work_started,
    }
}

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

//!
//! The stages, each in its file under `global/`: the charges and their
//! field (field.rs), the start (start.rs: the immovable charge, the bodies
//! and fillers, the noise and the netlist's start) and the descent
//! (descent.rs).

mod descent;
mod field;
mod start;

use crate::constraints;
use crate::problem::{Point, Pose, Problem, rotate};
use std::collections::BTreeMap;

use descent::Descent;
use field::{Body, Field};

/// What the global placement does with the netlist's stacks
/// (`spread_stacks`; docs/placer.md, "The global placement's start" and
/// "Banks on a wide bus glued through the descent").
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

/// The descent's state at an iteration (`GlobalResult::history`): what
/// decides whether the wires arrange the parts before the density spreads
/// them (docs/placer.md, "The global placement's start").
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

/// What the global placement's stages read: the board, the settings, the
/// movable parts and the immovable charge.
struct Board<'a> {
    problem: &'a Problem,
    config: &'a GlobalConfig,
    bounds: [f64; 4],
    /// A density bin's area.
    bin_area: f64,
    /// Per density field, the immovable area in each bin
    /// (`start::FixedCharge`).
    side_fixed: Vec<Vec<f64>>,
    /// The movable components: body `b` is component `movable[b]`, the
    /// fillers follow them.
    movable: Vec<usize>,
    /// The movable bodies' charge.
    movable_area: f64,
    /// The fixed parts' pins: position and net.
    fixed_pins: Vec<(Point, usize)>,
    /// The banks that move as one body in the descent (body indices; the
    /// netlist start's `spread_stacks`).
    glued: Vec<Vec<usize>>,
    /// Escape demand (`Problem::escape`) and each net's pins, for the
    /// parts' exits.
    escape: Option<&'a crate::escape::Escape>,
    escape_members: Option<Vec<crate::escape::NetPins>>,
}

impl Board<'_> {
    /// Clamps the bodies' `centers` onto the board (the board's centre on
    /// an axis the body is longer than) and a component's into its
    /// constraints (`constraints::clamp_center`).
    fn clamp(&self, centers: &mut [Point], bodies: &[Body]) {
        let bounds = self.bounds;
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
                constraints::clamp_center(self.problem, index, center, body.bare, body.angle);
            }
        }
    }

    /// Per movable component: pin offsets from the body centre at its angle.
    fn pin_offsets(&self, poses: &[Pose]) -> Vec<Vec<(Point, usize)>> {
        let problem = self.problem;
        self.movable
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
    }
}

/// The global placement of `problem`'s movable parts: the start (the
/// immovable charge, the bodies and fillers, the noise and the netlist's
/// start), then the descent until the bodies no longer overlap.
pub fn global_place(problem: &Problem, config: &GlobalConfig) -> GlobalResult {
    let bounds = problem.bounds();
    let mut field = Field::new(config.bins, bounds);
    let bin_area = field.bin[0] * field.bin[1];
    let poses = problem.poses.clone();
    let charge = start::fixed_charge(problem, &poses, &mut field);

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
    let mut bodies = start::bodies(problem, &poses, &movable, charge.side_count);
    let movable_area: f64 = bodies
        .iter()
        .map(|body| 4.0 * body.charge_half()[0] * body.charge_half()[1])
        .sum();
    start::add_fillers(&mut bodies, config, &field, &charge);
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
    // Escape demand: each net's pins, for the parts' exits.
    let escape = problem.escape.as_deref();
    let mut board = Board {
        problem,
        config,
        bounds,
        bin_area,
        side_fixed: charge.fixed,
        movable,
        movable_area,
        fixed_pins,
        glued: Vec::new(),
        escape,
        escape_members: escape.map(|_| crate::escape::members(problem)),
    };

    let starts: BTreeMap<usize, Point> = problem.constraints.starts.iter().copied().collect();
    let (start, mut major) = start::noise_start(&board, &poses, &bodies, &starts);
    board.clamp(&mut major, &bodies);
    let offsets = board.pin_offsets(&poses);
    let (glued, stacked) =
        start::netlist_start(&board, &bodies, &offsets, &starts, start, &mut major);
    board.glued = glued;
    let start_poses: Vec<Pose> = {
        let mut at = poses.clone();
        for (body, index) in board.movable.iter().enumerate() {
            at[*index].position =
                problem.components[*index].position_for_center(major[body], at[*index].angle);
        }
        at
    };

    let mut descent = Descent::new(&board, field, bodies, offsets, poses, major);
    descent.run();
    descent.result(start_poses, stacked)
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

#[cfg(test)]
mod tests;

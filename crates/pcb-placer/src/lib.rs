// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! PCB component placement: electrostatic global placement, legalization,
//! and wirelength-driven detailed placement.

pub mod anneal;
mod buckets;
pub mod constraints;
pub mod crossings;
pub mod escape;
pub mod exits;
pub mod facing;
pub mod global;
pub mod groups;
pub mod guidance;
pub mod initial;
pub mod legal;
pub mod orient;
pub mod problem;
pub mod skeleton;
pub mod stacks;
pub mod view;

pub use global::{Frame, GlobalConfig};
pub use problem::{Component, FarOpen, Pin, Point, Pose, Problem, Side};

#[derive(Clone, Debug, Default)]
pub struct Config {
    /// The global (electrostatic) placement's settings.
    pub global: GlobalConfig,
    /// The annealer's settings.
    pub anneal: anneal::AnnealConfig,
    /// Passes of the wirelength-reducing local search on the legal
    /// placement (`legal::refine`), also after each relation repair.
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
    /// The orientation search starts from a canonical pose (`orient`):
    /// angle options in a fixed order, every movable part that may turn at
    /// the first of them, so that the placement does not depend on the
    /// angles the board held (0 degrees from an import, the designer's in
    /// a stack). Off: the parts start at the angles they hold, options in
    /// the order given (the placer before 2026-10-15).
    pub orientation_search: bool,
    /// Bus-group moves (`groups`, docs/placer.md "Bus-group moves
    /// (2026-10-26)"): banks of parts on one bus and parts with their
    /// passives move, turn, trade places and reorder as units in the
    /// anneal and the refinement. Off (`None`) by default.
    pub bus_groups: Option<groups::Settings>,
    /// KiCad's legality at the last level (where courtyards may touch):
    /// courtyards touch where the pads
    /// keep the copper clearance pad to pad, an unheld part's pads keep
    /// the edge margin, the board's silkscreen marks the project does not
    /// rate an error over a part are no obstacle, and a lead through the
    /// board keeps the copper clearance from the far side's pads
    /// (docs/placer.md, "The legality model against KiCad", the molecules).
    pub kicad_last_level: bool,
    /// Pads without a way out (`exits`, docs/placer.md "Pads without a
    /// way out (2026-11-04)"): after the placement, the router's test of
    /// every pad's way out on the statics, and moves or turns
    /// of the parts whose pads another part or the edge closes. Off
    /// (`None`) by default.
    pub pad_exits: Option<exits::PadExits>,
    /// The first relaxation level the placement tries (`relaxation_levels`;
    /// the last if there are fewer): the race over skeletons places the
    /// passives from the skeleton's level on, so that the held skeleton is
    /// legal at theirs (`skeleton`). 0 by default: every level.
    pub first_level: usize,
    /// A diagnostic start (`PCB_PLACER_START`, docs/placer.md "Where the
    /// arrangement is lost (2026-11-04)"): the stages from `Start::stage`
    /// on run from given poses (the designer's). `None` by default: the
    /// placer's own start.
    pub start: Option<std::sync::Arc<Start>>,
    /// A diagnostic (`PCB_PLACER_SKIP_ANNEAL`, docs/placer.md "Where the
    /// arrangement is lost (2026-11-04)"): no anneal at any level, each
    /// level's legaliser seats the global placement's poses (or the
    /// start's). Off by default.
    pub skip_anneal: bool,
}

/// Where a diagnostic start (`Config::start`) enters the pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartStage {
    /// The global placement starts from the poses: the parts of
    /// `Start::parts` at their centres without the start's noise and at
    /// their angles, the others from the placer's own start.
    Global,
    /// No global placement: each level's anneal starts from the poses.
    Anneal,
    /// No global placement and no anneal: each level's legaliser seats
    /// the poses (its nearest legal spots), the refinement follows.
    Legalize,
}

/// A diagnostic start (`Config::start`).
#[derive(Clone, Debug)]
pub struct Start {
    /// Per component, the pose to start from (a fixed part keeps its own).
    pub poses: Vec<Pose>,
    /// Per component, whether it starts from `poses` (`StartStage::Global`
    /// only; the later stages start every movable part there).
    pub parts: Vec<bool>,
    pub stage: StartStage,
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
            orientation_search: true,
            bus_groups: None,
            kicad_last_level: false,
            pad_exits: None,
            first_level: 0,
            start: None,
            skip_anneal: false,
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
    /// The poses after each stage (the global placement's start, global,
    /// annealed, legal, final; under the race over skeletons the kept
    /// skeleton's before them, `skeleton_` and the stage), for the
    /// measurement of where wirelength goes (`PCB_PLACER_TRACE`).
    pub stage_poses: Vec<(&'static str, Vec<Pose>)>,
    /// The global placements' descents (`global::GlobalResult::history`):
    /// `global`, and the kept skeleton's `skeleton_global`.
    pub global_histories: Vec<(&'static str, Vec<global::GlobalStep>)>,
    /// Even the tightest bodies need more area than a side has: no
    /// packing places every part, and the placer said so at once.
    pub overfull: bool,
    /// What the pad-exits repair found and did (`Config::pad_exits`;
    /// `None`: off, or its session could not answer).
    pub exits: Option<exits::ExitOutcome>,
    /// The relaxation level the placement ended at (its index among
    /// `relaxation_levels`).
    pub level: usize,
    /// The race over skeletons' account of this placement (`skeleton`),
    /// where it ran.
    pub skeleton: Option<skeleton::Report>,
    /// The relaxation levels that ran, each with the parts it left
    /// without room and the placement's work when it ended
    /// (`PCB_PLACER_TRACE`: what walks a placement to its level, and what
    /// each level cost).
    pub walk: Vec<(usize, Vec<usize>, u64)>,
}

#[derive(Clone, Debug, PartialEq)]
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
    /// KiCad's legality at this level (`Constraints::pad_clearance`).
    pub pad_clearance: bool,
    /// Per component, the halo the placement gave it, where it is not the
    /// pin-count halo times the scales (the skeleton race's merged halos,
    /// `skeleton`).
    pub halos: Option<std::sync::Arc<Vec<f64>>>,
    /// The parts that lie on the board's marks, having found no room off
    /// them (`Constraints::on_marks`).
    pub on_marks: Vec<usize>,
}

/// Seats the parts of `failed` (no room at this level) where only the
/// board's marks (`Constraints::yielding`: its own texts and silkscreen
/// graphics) are in the way: they may lie on them (`on_marks`), the other
/// parts keep their places. A connector at the edge under its own pin
/// names (OpenFC, MIDAS-MK1.1) or beside a business card's artwork
/// (USB_Keypad) takes its spot there at the level it fails, rather than
/// every part giving up its routing room first.
fn seat_on_marks(
    relaxed: &mut Problem,
    poses: &mut Vec<Pose>,
    failed: &mut Vec<usize>,
    pass_work: u64,
    budget: Option<u64>,
    debug: bool,
) {
    if failed.is_empty() || relaxed.constraints.yielding.is_empty() {
        return;
    }
    let mut trial = relaxed.clone();
    let mut on_marks = trial.constraints.on_marks.clone();
    on_marks.extend(failed.iter().copied());
    on_marks.sort_unstable();
    on_marks.dedup();
    trial.constraints.on_marks = on_marks;
    let keep: Vec<usize> = (0..trial.components.len())
        .filter(|index| !failed.contains(index))
        .collect();
    let mut seated = poses.clone();
    let seat_mark = mark();
    let mut again = legal::legalize_keeping(&trial, &mut seated, failed, &keep);
    let kept_left = again.len();
    // Still without room (the others took the spots by the marks, where
    // MIDAS-MK1.1's two 36 mm headers go at the bottom edge): every part
    // again, those first, the others around them. Not where both an
    // eighth of the movable parts or more found no room (the level is too
    // tight for them, which the next level eases) and the passes over
    // every part (`pass_work` each, the level's full legalization) would
    // take more than an eighth of the placement's `budget`: on a crowded
    // panel they took the work of the looser levels that place it
    // (OpenRX's panel: 55 of 216 parts without room at full halos, a pass
    // 35 s, 100 s of its 600 s of work at each of the first two levels).
    // Elsewhere the passes stay (MIDAS-MK1.1: 1 to 10 of 160 parts failed,
    // a pass 10 s of 200; reCamera: more of fewer parts, cheap passes).
    let movable = trial
        .components
        .iter()
        .filter(|component| !component.fixed)
        .count();
    let many = failed.len() * 8 >= movable;
    let costly = budget.is_some_and(|budget| STUCK_FIRST_PASSES as u64 * pass_work > budget / 8);
    if !again.is_empty() && !(many && costly) {
        let (retry, left) =
            legal::legalize_stuck_first(&trial, poses, failed, STUCK_FIRST_PASSES, debug);
        if left.len() < again.len() {
            seated = retry;
            again = left;
        }
    }
    phase_line(debug, "legalize on marks", seat_mark);
    if debug {
        eprintln!(
            "placer: on marks: {} failed, {kept_left} left with the others kept, {} in the end",
            failed.len(),
            again.len()
        );
    }
    if again.len() < failed.len() {
        if debug {
            eprintln!(
                "placer: {} of {} parts seated on the board's marks",
                failed.len() - again.len(),
                failed.len()
            );
        }
        trial
            .constraints
            .on_marks
            .retain(|part| !again.contains(part));
        *relaxed = trial;
        *poses = seated;
        *failed = again;
    }
}

impl Relaxation {
    /// Applies the relaxation to `problem`, whose halos are unscaled.
    pub fn apply(&self, problem: &mut Problem) {
        problem.spacing = self.spacing;
        problem.grid = self.grid;
        for (index, component) in problem.components.iter_mut().enumerate() {
            component.halo = match self.halos.as_ref().and_then(|halos| halos.get(index)) {
                Some(halo) => *halo,
                None => {
                    component.halo
                        * if component.pins.len() >= MANY_PINS {
                            self.halo_scale
                        } else {
                            self.small_halo_scale
                        }
                }
            };
            if !self.edge_inset {
                component.edge_inset = 0.0;
            }
            if self.tight {
                component.use_tight_body();
            }
        }
        problem.constraints.edge_copper = self.edge_copper;
        problem.constraints.courtyard_spacing = self.courtyard_spacing || self.pad_clearance;
        problem.constraints.pad_clearance = self.pad_clearance;
        if self.pad_clearance {
            problem.pass_marks();
        }
        problem.edge_margin = problem.edge_margin.min(self.edge_rule);
        problem.constraints.on_marks = self.on_marks.clone();
    }
}

/// Parts with at least this many pins keep their halos one level longer.
const MANY_PINS: usize = 10;

/// Passes of the legaliser with the parts that found no room first, each
/// adding the parts the pass before left without room
/// (`legal::legalize_stuck_first`), at a loosest level (no halo, spacing
/// or grid left to give) that keeps the placement of the level before,
/// when a few parts (as many as an eviction takes on) found no room among
/// the kept ones.
const STUCK_FIRST_PASSES: usize = 4;

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

/// Whether the level loop goes on to the tight bodies at once: more
/// parts without room than a level's passes seat (a fiftieth of the
/// parts, three at least), and less work left than twice the last level's
/// for each level before the tight one (a level is a kept placement and a
/// fresh anneal: on OpenESC's panel 1.5 to 2 times the one before).
fn skip_to_tight(
    failed: usize,
    parts: usize,
    work_left: u64,
    level_work: u64,
    levels_before: usize,
) -> bool {
    failed > (parts / 50).max(3)
        && level_work > 0
        && (work_left as u128) < 2 * level_work as u128 * levels_before as u128
}

/// Whether the parts on a side need more area than the board has, bodies
/// and spacing alone (halos are a wish; cutouts take no room): no packing
/// can place them all.
fn overfull(problem: &Problem) -> bool {
    side_overfull(problem, true)
}

/// `overfull` counting the fixed parts too (`with_fixed`) or the movable
/// ones alone: those may not overlap each other whatever the fixed ones
/// do (a designer's fixed parts may overlap where the rules allow it).
fn side_overfull(problem: &Problem, with_fixed: bool) -> bool {
    let mut used = [0.0f64; 2];
    for component in &problem.components {
        if component.copper_only || (component.fixed && !with_fixed) {
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

/// The least extents of a part's body along x and along y over the angles
/// it may take (a part that may turn by a quarter has its smaller side
/// both ways; a row turned only by halves keeps its length along its axis).
pub fn least_extents(component: &Component) -> [f64; 2] {
    let size = component.body_size;
    let mut least = [f64::INFINITY, f64::INFINITY];
    for angle in &component.angle_options {
        let turns = angle.rem_euclid(360.0) / 90.0;
        let extent = if (turns - turns.round()).abs() > 1.0e-6 {
            // Any angle: no more than the smaller side either way.
            [size[0].min(size[1]); 2]
        } else if turns.round() as i64 % 2 == 0 {
            size
        } else {
            [size[1], size[0]]
        };
        least = [least[0].min(extent[0]), least[1].min(extent[1])];
    }
    if least[0].is_finite() { least } else { size }
}

/// Whether bodies with the least extents `parts` (`least_extents`) can lie
/// side by side in a `size` box, by strips: the bodies at least `t` tall
/// meet any vertical line at most `floor(height / t)` times (bodies do
/// not overlap), so their widths sum to at most that times the box's
/// width; the same across. A necessary condition only: `false` means no
/// arrangement exists.
pub fn strips_hold(parts: &[[f64; 2]], size: [f64; 2]) -> bool {
    for axis in 0..2 {
        // Lines across `axis`: the parts' extents across them, then along.
        let mut sorted: Vec<[f64; 2]> = parts
            .iter()
            .map(|extent| [extent[1 - axis], extent[axis]])
            .filter(|extent| extent[0] > 0.0)
            .collect();
        sorted.sort_by(|a, b| b[0].total_cmp(&a[0]));
        let mut sum = 0.0;
        for [across, along] in sorted {
            sum += along;
            if sum > (size[1 - axis] / across).floor() * size[axis] + 1.0e-6 {
                return false;
            }
        }
    }
    true
}

/// Whether a side's movable parts cannot lie in the outline's box in any
/// arrangement, by strips (`strips_hold`). Exact, and stronger than the
/// area for large parts: 34 key switches with 18 x 17 mm courtyards on
/// 80 x 40 mm (96 % by area) need 34 x 17 mm of length where two strips
/// of 80 mm are all there is. Parts held at an edge or reaching over one,
/// hollow parts and cutouts are left out; a body may reach its
/// `edge_inset` beyond the outline.
fn strips_overfull(problem: &Problem) -> bool {
    if problem.outline.len() < 3 {
        return false;
    }
    let bounds = problem.bounds();
    let reach = problem
        .components
        .iter()
        .map(|component| component.edge_inset)
        .fold(0.0f64, f64::max)
        .max(0.0);
    let size = [
        bounds[2] - bounds[0] + 2.0 * reach,
        bounds[3] - bounds[1] + 2.0 * reach,
    ];
    let constraints = &problem.constraints;
    [Side::Front, Side::Back].into_iter().any(|side| {
        let parts: Vec<[f64; 2]> = problem
            .components
            .iter()
            .enumerate()
            .filter(|(index, component)| {
                !component.fixed
                    && !component.copper_only
                    && component.hollow.is_empty()
                    && component.side.collides(side)
                    && component.side != Side::Neither
                    && !constraints.edges.iter().any(|(part, _, _)| part == index)
                    && constraints.overhang(*index).is_none()
            })
            .map(|(_, component)| least_extents(component))
            .collect();
        !strips_hold(&parts, size)
    })
}

/// The board area per side (front, back) that fixed parts leave free, on
/// a coarse raster: overlapping fixed bodies (a copper graphic and its
/// mask twin) count once; cutouts take no room from bodies; a hollow
/// part blocks with its boxes only.
fn free_areas(problem: &Problem) -> [f64; 2] {
    let bounds = problem.bounds();
    let (width, height) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
    let cell = (width.max(height) / 200.0).max(1.0e-6);
    let (nx, ny) = (
        (width / cell).ceil().max(1.0) as usize,
        (height / cell).ceil().max(1.0) as usize,
    );
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
            let x0 =
                (((center[0] - half[0] - bounds[0]) / cell).floor().max(0.0) as usize).min(nx - 1);
            let x1 = (((center[0] + half[0] - bounds[0]) / cell).ceil().max(0.0) as usize).min(nx);
            let y0 =
                (((center[1] - half[1] - bounds[1]) / cell).floor().max(0.0) as usize).min(ny - 1);
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
            let point = [
                bounds[0] + (x as f64 + 0.5) * cell,
                bounds[1] + (y as f64 + 0.5) * cell,
            ];
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
        if component.pins.len() >= MANY_PINS {
            scales[1]
        } else {
            scales[0]
        }
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

thread_local! {
    /// The tests' choice of check mode on this thread (`with_check_mode`),
    /// or None: `PCB_PLACER_CHECK_OVERLAP`'s.
    static CHECK_MODE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Check mode (`PCB_PLACER_CHECK_OVERLAP`): every cache and shortcut is
/// checked against its direct computation at every use. A check books no
/// work, draws no random number and leaves every cache as the unchecked
/// path leaves it, so that a placement in check mode is the placement
/// without it (`check_mode_places_as_without_it`).
pub(crate) fn check_mode() -> bool {
    CHECK_MODE
        .with(|mode| mode.get())
        .unwrap_or_else(|| std::env::var_os("PCB_PLACER_CHECK_OVERLAP").is_some())
}

/// Whether the caches are checked: in check mode and in debug builds.
pub(crate) fn checking() -> bool {
    cfg!(debug_assertions) || check_mode()
}

/// Runs `body` with check mode on or off on this thread (the tests: the
/// environment is the process's).
#[cfg(test)]
pub(crate) fn with_check_mode<T>(on: bool, body: impl FnOnce() -> T) -> T {
    let before = CHECK_MODE.with(|mode| mode.replace(Some(on)));
    let result = body();
    CHECK_MODE.with(|mode| mode.set(before));
    result
}

/// Placement work one thread does per second on the reference (the 5950X
/// idle, docs/placer.md, "Placement work against the CPU"). The rate a
/// process turns its budgets in seconds into work limits at is
/// `work_per_second`: this one unless the process set its own (pcb-kicad's
/// `work_rate`: the start-up calibration scales it to the machine,
/// `PCB_WORK_RATE` pins it).
pub const REFERENCE_WORK_PER_SECOND: f64 = 150.0e6;

/// The rate `set_work_per_second` set (its bits; 0 when unset).
static WORK_RATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Placement work a second stands for in this process: budgets in seconds
/// become work limits at this rate. The reference's
/// (`REFERENCE_WORK_PER_SECOND`) unless the process set its own, once,
/// before any work.
pub fn work_per_second() -> f64 {
    match WORK_RATE.load(std::sync::atomic::Ordering::Relaxed) {
        0 => REFERENCE_WORK_PER_SECOND,
        bits => f64::from_bits(bits),
    }
}

/// Sets the rate of `work_per_second` for the rest of the process. Panics
/// on a rate that is not a positive finite number.
pub fn set_work_per_second(rate: f64) {
    assert!(
        rate.is_finite() && rate > 0.0,
        "a work rate must be positive: {rate}"
    );
    WORK_RATE.store(rate.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// The work of one pin's position (a turn of its offset) in a wirelength
/// sum, in units of one part-against-part test.
pub(crate) const PIN_WORK: u64 = 3;

/// The placement work done on this thread so far.
pub fn work() -> u64 {
    WORK.with(|work| work.get())
}

thread_local! {
    /// The work at which the running `place` on this thread is out of its
    /// budget (`Config::work_limit`): the legalisation, eviction, refinement
    /// and repair passes check it inside, not only between levels.
    static STOP: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Whether the running placement has spent its work budget, counting
/// `pending` work not yet added.
pub(crate) fn out_of_work(pending: u64) -> bool {
    STOP.with(|stop| stop.get().is_some_and(|stop| work() + pending >= stop))
}

/// Sets the budget's stop for the passes for as long as it lives, and
/// restores the one before when dropped.
struct StopGuard(Option<u64>);

impl StopGuard {
    fn set(stop: Option<u64>) -> Self {
        StopGuard(STOP.with(|cell| cell.replace(stop)))
    }
}

impl Drop for StopGuard {
    fn drop(&mut self) {
        STOP.with(|cell| cell.set(self.0));
    }
}

pub(crate) fn add_work(amount: u64) {
    WORK.with(|work| work.set(work.get() + amount));
}

/// Takes back work a debug check of a cache booked (its direct
/// computation is not the placement's work).
pub(crate) fn add_work_back(amount: u64) {
    WORK.with(|work| work.set(work.get() - amount));
}

/// The CPU time this thread has run (Linux: `/proc/thread-self/schedstat`),
/// in seconds; `None` where that is not available.
fn thread_cpu_seconds() -> Option<f64> {
    let text = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    let nanoseconds: f64 = text.split_whitespace().next()?.parse().ok()?;
    Some(nanoseconds * 1.0e-9)
}

/// Debugging aid (`PCB_PLACER_DEBUG`): a phase's wall time and the thread's
/// CPU time against the work it counted, from a mark taken at its start.
/// The ratio is work over CPU time (the wall clock on a busy machine runs
/// on while the thread waits); near 1 when the work is calibrated.
fn phase_line(debug: bool, name: &str, mark: Mark) {
    if debug {
        let seconds = mark.0.elapsed().as_secs_f64();
        let cpu = match (mark.2, thread_cpu_seconds()) {
            (Some(start), Some(now)) => now - start,
            _ => seconds,
        };
        let work = work() - mark.1;
        let counted = work as f64 / work_per_second();
        eprintln!(
            "placer phase {name}: {seconds:.2} s, cpu {cpu:.2} s, work {counted:.2} s ({:.2})",
            counted / cpu.max(1.0e-6)
        );
        if crossings::profile::enabled() {
            let tally = crossings::profile::since(mark.3, crossings::profile::tally());
            // Wall per unit in the count against the rest of the phase.
            let inside = tally.nanos as f64 * 1.0e-9;
            let rest_rate = (work - tally.booked) as f64 / (seconds - inside).max(1.0e-9);
            let inside_rate = tally.booked as f64 / inside.max(1.0e-9);
            eprintln!(
                "placer crossings {name}: wall {seconds:.4} s, work {work}, inside {inside:.4} s, booked {}, calls {}, pins {}, prim {}, lines {}, cells {}, met {}, tested {}, grid_cells {}, scanned {}, tree {:.4} s, count {:.4} s, insert {:.4} s, remove {:.4} s; rest {rest_rate:.0}/s, inside {inside_rate:.0}/s, factor {:.3}",
                tally.booked,
                tally.calls,
                tally.pins,
                tally.prim,
                tally.lines,
                tally.cells,
                tally.met,
                tally.tested,
                tally.grid_cells,
                tally.scanned,
                tally.tree_nanos as f64 * 1.0e-9,
                tally.count_nanos as f64 * 1.0e-9,
                tally.insert_nanos as f64 * 1.0e-9,
                tally.remove_nanos as f64 * 1.0e-9,
                rest_rate / inside_rate.max(1.0e-9),
            );
        }
    }
}

type Mark = (
    std::time::Instant,
    u64,
    Option<f64>,
    crossings::profile::Tally,
);

fn mark() -> Mark {
    (
        std::time::Instant::now(),
        work(),
        thread_cpu_seconds(),
        crossings::profile::tally(),
    )
}

/// Refinement, edge-copper centring, relation repair and alignment (of
/// orientations): what
/// follows legalization.
fn finish(
    problem: &Problem,
    poses: &mut [Pose],
    passes: usize,
    groups: Option<&groups::BusGroups>,
    debug: bool,
) {
    let refine_mark = mark();
    // Escape demand: one state through the refinements, its measured
    // correction taken now and every so many kept moves.
    let mut escape = problem.escape.as_deref().map(|escape| {
        let mut state = escape::EscapeState::new(problem, escape, poses, |_| true);
        state.measure_from_now(poses);
        state
    });
    // Ratsnest crossings: one state through the refinements.
    let mut crossings = problem
        .crossings
        .as_deref()
        .filter(|crossings| crossings.refine)
        .map(|crossings| crossings::CrossingState::new(problem, crossings, poses));
    let crossings_before = crossings.as_ref().map(|state| state.total());
    legal::refine_with(
        problem,
        poses,
        passes,
        escape.as_mut(),
        crossings.as_mut(),
        groups,
    );
    phase_line(debug, "refine", refine_mark);
    let repair_mark = mark();
    legal::center_edge_copper(problem, poses);
    // Moving one part can break a relation repaired before; repeat while
    // it helps.
    for _ in 0..5 {
        if legal::repair_relations(problem, poses) == 0 {
            break;
        }
        legal::refine_with(
            problem,
            poses,
            passes,
            escape.as_mut(),
            crossings.as_mut(),
            groups,
        );
    }
    // Parts of a kind turned alike where it costs next to nothing.
    legal::align_orientations(problem, poses, 0.5);
    // A pair's series parts facing the pair's two ends.
    if !problem.constraints.facing.is_empty() {
        let faces = facing::face(problem, poses);
        if debug {
            eprintln!(
                "placer: pair rows: {} turned to face their pairs' ends, {} without room to",
                faces.turned, faces.held
            );
        }
    }
    phase_line(debug, "repair and align", repair_mark);
    if let Some(state) = &escape {
        eprintln!(
            "placer: escape demand: {} measurements, {} kept moves",
            state.measurements,
            state.kept_moves()
        );
    }
    if debug && let (Some(state), Some(before)) = (crossings.as_mut(), crossings_before) {
        // The line's count is not the placement's work: booked, it made a
        // run with PCB_PLACER_DEBUG place otherwise than one without (its
        // seeds' work, so the race's batches and the seeds kept).
        let counted = work();
        state.resync(poses);
        add_work_back(work() - counted);
        eprintln!(
            "placer: ratsnest crossings {before} -> {} in the refinement",
            state.total()
        );
    }
}

/// A relaxation level of `place`: halo scale, spacing scale, grid, edge
/// inset, tight bodies, edge copper, edge margin, and the factor on the
/// halos of parts with fewer than `MANY_PINS` pins.
type Level = (f64, f64, f64, bool, bool, bool, f64, f64);

/// The relaxation levels a placement walks down, loosest comfort first,
/// and the index of the last, where courtyards may touch
/// (`Constraints::courtyard_spacing`).
fn relaxation_levels(problem: &Problem) -> (Vec<Level>, usize) {
    // The last level also snaps to a finer grid: on small, crowded boards
    // the regular grid leaves no legal spot where a finer one does.
    let fine = problem.grid.min(0.1);
    // Then bodies may come closer to the edge, as far as their copper
    // allows (the full margin is room for routing too). Last, where the
    // board's rules let courtyards overlap: bodies without the courtyard's
    // margin.
    let inset = problem
        .components
        .iter()
        .any(|component| component.edge_inset > 0.0);
    let tight = problem
        .components
        .iter()
        .any(|component| component.tight.is_some());
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
    let mut levels: Vec<Level> = levels
        .into_iter()
        .map(|(halo, spacing, grid, inset, tight, small)| {
            (
                halo,
                spacing,
                grid,
                inset,
                tight,
                false,
                problem.edge_margin,
                small,
            )
        })
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
    (levels, courtyard_level)
}

/// Whether level `level` judges by KiCad's legality
/// (`Constraints::pad_clearance`): the last with `kicad_last_level`.
fn kicad_level(config: &Config, level: usize, courtyard_level: usize) -> bool {
    config.kicad_last_level && level == courtyard_level
}

/// The problem at level `level` of `relaxation_levels`: its halos,
/// spacing, grid and bodies.
fn relax_level(
    problem: &Problem,
    config: &Config,
    level: usize,
    courtyard_level: usize,
    (halo_scale, spacing_scale, grid, inset, tight, edge_copper, edge_rule, small_factor): Level,
) -> Problem {
    let mut relaxed = problem.clone();
    relaxed.grid = grid;
    relaxed.spacing = (relaxed.spacing * spacing_scale).max(relaxed.min_spacing);
    relaxed.constraints.edge_copper = edge_copper;
    relaxed.constraints.courtyard_spacing = level == courtyard_level;
    // KiCad's legality where the spacing is the rules' least, or at the
    // last level.
    if kicad_level(config, level, courtyard_level) {
        relaxed.constraints.courtyard_spacing = true;
        relaxed.constraints.pad_clearance = true;
        relaxed.pass_marks();
    }
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
    relaxed
}

/// One relaxation level as `level_problems` gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelShape {
    /// Factors on the halos of the parts with many pins and the others.
    pub halo_scale: f64,
    pub small_halo_scale: f64,
    /// Spacing between bodies, mm, and the placement grid.
    pub spacing: f64,
    pub grid: f64,
    pub edge_inset: bool,
    pub tight: bool,
    pub edge_copper: bool,
    pub edge_rule: f64,
    pub courtyard_spacing: bool,
    pub pad_clearance: bool,
}

/// The problem the global placement sees (`place`'s, before its levels):
/// the halos fitted to the board, the linked pairs found; for diagnostics
/// that judge given poses by the global placement's model.
pub fn global_problem(problem: &Problem, config: &Config) -> Problem {
    let (mut fitted, _) = fit_halos(problem, config.maximum_utilization);
    fitted.constraints.link_pairs();
    fitted
}

/// The problem at every relaxation level a placement may walk down, the
/// loosest first, with the board's halos fitted as `place` fits them: for
/// diagnostics that judge given poses (a designer's) level by level.
pub fn level_problems(problem: &Problem, config: &Config) -> Vec<(LevelShape, Problem)> {
    let (mut fitted, _) = fit_halos(problem, config.maximum_utilization);
    fitted.constraints.link_pairs();
    let (levels, courtyard_level) = relaxation_levels(&fitted);
    levels
        .into_iter()
        .enumerate()
        .map(|(index, level)| {
            let relaxed = relax_level(&fitted, config, index, courtyard_level, level);
            let shape = LevelShape {
                halo_scale: level.0,
                small_halo_scale: level.7,
                spacing: relaxed.spacing,
                grid: relaxed.grid,
                edge_inset: level.3,
                tight: level.4,
                edge_copper: level.5,
                edge_rule: level.6,
                courtyard_spacing: index == courtyard_level,
                pad_clearance: relaxed.constraints.pad_clearance,
            };
            (shape, relaxed)
        })
        .collect()
}

/// The first relaxation level at which the parts' bodies and spacing fit
/// the board by area (`overfull`), from `config.first_level` on, as `place`
/// walks the levels (the last level where none does, and the first where
/// no level can: `hopeless`). The race over skeletons places its skeleton
/// from here: at a level where the whole board is overfull, a skeleton's
/// gaps cannot hold the passives (the OpenESC 4in1-mini fits only with
/// tight bodies, and its skeleton, placed at level 1, left its passives no
/// room on 13 of 15 seeds).
pub fn first_fitting_level(problem: &Problem, config: &Config) -> usize {
    let (fitted, _) = fit_halos(problem, config.maximum_utilization);
    let (levels, courtyard_level) = relaxation_levels(&fitted);
    let count = levels.len();
    let first = config.first_level.min(count - 1);
    let relax = |level: usize| relax_level(&fitted, config, level, courtyard_level, levels[level]);
    if side_overfull(&relax(count - 1), false) || strips_overfull(&relax(count - 1)) {
        return first;
    }
    (first..count)
        .find(|level| *level + 1 == count || !overfull(&relax(*level)))
        .unwrap_or(count - 1)
}

pub fn place(problem: &Problem, config: &Config) -> Placement {
    let (mut fitted, fit_scale) = fit_halos(problem, config.maximum_utilization);
    fitted.constraints.link_pairs();
    let wirelength_initial = fitted.wirelength(&fitted.poses);
    if config.orientation_search {
        orient::canonical_start(&mut fitted);
    }
    // A diagnostic start: its parts at its poses, angles included (a
    // panel's copy or a sub-board is a problem of its own: none there).
    let start = config
        .start
        .as_deref()
        .filter(|start| start.poses.len() == fitted.components.len());
    if let Some(start) = start {
        for index in 0..fitted.components.len() {
            let all = start.stage != StartStage::Global;
            if !fitted.components[index].fixed && (all || start.parts[index]) {
                fitted.poses[index] = start.poses[index];
            }
        }
    }
    // The stages a diagnostic start skips.
    let skip_global = start.is_some_and(|start| start.stage != StartStage::Global);
    let skip_anneal =
        config.skip_anneal || start.is_some_and(|start| start.stage == StartStage::Legalize);
    let problem = &fitted;
    let debug = std::env::var_os("PCB_PLACER_DEBUG").is_some();
    let started = std::time::Instant::now();
    let work_started = work();
    let stop_at_work = config.work_limit.map(|limit| work_started + limit);
    let mut anneal_config = config.anneal.clone();
    anneal_config.stop_at_work = stop_at_work.or(anneal_config.stop_at_work);
    // Bus groups: found once, on the fitted problem (the levels change
    // halos and spacing, not nets).
    let bus_groups = config.bus_groups.as_ref().map(|settings| {
        let groups = std::sync::Arc::new(groups::BusGroups::detect(problem, settings));
        if debug {
            let banks: Vec<String> = groups
                .groups
                .iter()
                .filter(|group| group.kind == groups::Kind::Bank)
                .map(|group| {
                    group
                        .chain
                        .iter()
                        .map(|part| problem.components[*part].name.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect();
            eprintln!(
                "placer: bus groups: {} banks [{}], {} clusters",
                banks.len(),
                banks.join("; "),
                groups.groups.len() - banks.len()
            );
        }
        groups
    });
    anneal_config.bus_groups = bus_groups.clone();
    let out_of_budget = || {
        config
            .anneal
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
            || anneal_config
                .stop_at_work
                .is_some_and(|stop| work() >= stop)
    };
    // The passes check the budget inside too (a part without room
    // searched the whole board at every level: urchin on 80 x 40 mm took
    // 11, 10 and 8 s of a 10 s budget in its first level's passes).
    let _stop = StopGuard::set(anneal_config.stop_at_work);
    let global_mark = mark();
    let global = match start {
        Some(start) if start.stage == StartStage::Global => {
            let mut warm = config.global.clone();
            warm.warm = Some(std::sync::Arc::new(start.parts.clone()));
            global::global_place(problem, &warm)
        }
        _ if skip_global => global::GlobalResult {
            poses: problem.poses.clone(),
            frames: Vec::new(),
            iterations: 0,
            overflow: 0.0,
            start: problem.poses.clone(),
            history: Vec::new(),
            stacks: Vec::new(),
        },
        _ => global::global_place(problem, &config.global),
    };
    phase_line(debug, "global", global_mark);
    if debug {
        eprintln!("placer: global {:.1}s", started.elapsed().as_secs_f64());
    }
    let mut global_poses = global.poses;
    let global_start = global.start;
    let global_history = global.history;
    // Bus groups: the banks' order and arrangement (`groups::arrange`).
    if let Some(groups) = bus_groups
        .as_deref()
        .filter(|groups| groups.settings.arrange && !skip_global)
    {
        let arrange_mark = mark();
        let arranged = groups::arrange(problem, groups, &mut global_poses);
        phase_line(debug, "arrange", arrange_mark);
        if debug {
            eprintln!(
                "placer: arrange: {} banks, {} arranged, cost {:.0} -> {:.0}",
                arranged.banks, arranged.taken, arranged.cost_before, arranged.cost_after
            );
        }
    }
    let wirelength_global = problem.wirelength(&global_poses);
    let mut frames = global.frames;

    // Crowded boards cannot afford the full comfort margins: give up halos,
    // then the spacing, before giving up on a part.
    let mut best: Option<(Vec<usize>, Vec<Pose>, Problem)> = None;
    // The last level also snaps to a finer grid (`relaxation_levels`).
    let fine = problem.grid.min(0.1);
    let (levels, courtyard_level) = relaxation_levels(problem);
    let mut relaxation = None;
    // The level `relaxation` is of.
    let mut best_level = 0;
    // The work of the last level's full legalization: what a pass over
    // every part costs (`seat_on_marks`).
    let mut pass_work = 0;
    // The annealed poses the best placement was legalized from.
    let mut best_annealed: Option<Vec<Pose>> = None;
    // The grid and bodies the last anneal ran with.
    let mut annealed_shape: Option<(f64, bool)> = None;
    let level_count = levels.len();
    // The problem at a level: its halos, spacing, grid and bodies.
    let relax =
        |level: usize, shape: Level| relax_level(problem, config, level, courtyard_level, shape);
    // No packing fits the parts even at the loosest level (the last): the
    // movable bodies need more area than a side has, or more than strips
    // of the board can hold (`strips_overfull`). Then no level can place
    // them all, and the placer says so at once instead of annealing and
    // legalizing at every level in vain. Exact: a board that can be placed
    // never takes this way.
    let hopeless = {
        let last = relax(level_count - 1, *levels.last().expect("levels"));
        side_overfull(&last, false) || strips_overfull(&last)
    };
    let mut overfull_sides = false;
    // Where the levels go on to when work runs short (`skip_to_tight`):
    // the first level with tight bodies, or without them the first with
    // no spacing on the fine grid; and the work the last level that ran
    // took.
    let first_tight = levels.iter().position(|level| level.4).or_else(|| {
        levels
            .iter()
            .position(|level| level.1 == 0.0 && level.2 == fine && level.2 < problem.grid)
    });
    let mut last_level_work: Option<u64> = None;
    let mut skipping_to_tight = false;
    let mut walk: Vec<(usize, Vec<usize>, u64)> = Vec::new();
    for (
        level,
        (halo_scale, spacing_scale, grid, inset, tight, edge_copper, edge_rule, small_factor),
    ) in levels.into_iter().enumerate()
    {
        // From `first_level` on (the race over skeletons: the passives
        // from the held skeleton's level).
        if level < config.first_level.min(level_count - 1) {
            continue;
        }
        // Past the caller's deadline the best placement so far stands.
        if best.is_some() && out_of_budget() {
            break;
        }
        // Many parts still without room and too little work left to try
        // every level before the tight bodies: on to them (OpenESC's
        // panel: 27 of 398 parts without room at the spacing level, each
        // looser grid and edge level 250-330 s of the 600, and the tight
        // level, which places it, never came). Where the rules keep
        // courtyards apart, on to the fine grid without spacing (the
        // laptop's power board: 35 parts without room after the halo
        // levels, placed on the fine grid with 4000 s, refused with 600).
        if !skipping_to_tight
            && let (Some(first_tight), Some(level_work), Some(stop), Some((failed, _, _))) =
                (first_tight, last_level_work, stop_at_work, best.as_ref())
            && level < first_tight
            && skip_to_tight(
                failed.len(),
                problem.components.len(),
                stop.saturating_sub(work()),
                level_work,
                first_tight - level,
            )
        {
            skipping_to_tight = true;
            if debug {
                eprintln!(
                    "placer: {} parts without room, {:.0} s of work left, a level {:.0} s: on to level {first_tight}",
                    failed.len(),
                    stop.saturating_sub(work()) as f64 / work_per_second(),
                    level_work as f64 / work_per_second()
                );
            }
        }
        if skipping_to_tight && first_tight.is_some_and(|first| level < first) {
            continue;
        }
        let level_mark = work();
        if grid == fine && fine == problem.grid && spacing_scale == 0.0 && !inset && best.is_some()
        {
            // Same as the previous level.
            continue;
        }
        let relaxed = relax(
            level,
            (
                halo_scale,
                spacing_scale,
                grid,
                inset,
                tight,
                edge_copper,
                edge_rule,
                small_factor,
            ),
        );
        // Bodies that need more room than a side has cannot all be placed
        // at this level: on to a looser one (OpenESC's boards, packed with
        // overlapping courtyards, fit only with tight bodies; every level
        // before them annealed and legalized in vain, 1-4 minutes each).
        if level + 1 < level_count && (hopeless || overfull(&relaxed)) {
            if debug {
                eprintln!(
                    "placer: level halo {halo_scale} (small {small_factor}) spacing {spacing_scale} grid {grid} inset {inset} tight {tight}: bodies need more room than a side has"
                );
            }
            continue;
        }
        // Even the last level's bodies need more area than a side has
        // (every level before was overfull too, so none ran): no packing
        // places them all. A fast no: the parts are seated near where the
        // global placement put them as far as there is room, without an
        // anneal, and the passes after it stop at once (urchin's 80 parts
        // on 80 x 40 mm took more than ten minutes before).
        if hopeless {
            if debug {
                eprintln!(
                    "placer: the last level is overfull too: no packing fits, the parts are seated where there is room"
                );
            }
            overfull_sides = true;
            STOP.with(|stop| stop.set(Some(work())));
            // On the regular grid: the hurried search reaches 15 mm there.
            let mut relaxed = relaxed;
            relaxed.grid = problem.grid;
            let mut poses = global_poses.clone();
            let failed = legal::legalize(&relaxed, &mut poses);
            walk.push((level, failed.clone(), work() - work_started));
            best_level = level;
            relaxation = Some(Relaxation {
                spacing: relaxed.spacing,
                grid: relaxed.grid,
                halo_scale: fit_scale[1] * halo_scale,
                small_halo_scale: fit_scale[0] * halo_scale * small_factor,
                edge_inset: inset,
                tight,
                edge_copper,
                edge_rule,
                courtyard_spacing: level == courtyard_level,
                pad_clearance: kicad_level(config, level, courtyard_level),
                halos: None,
                on_marks: relaxed.constraints.on_marks.clone(),
            });
            best = Some((failed, poses, relaxed));
            break;
        }
        let level_started = std::time::Instant::now();
        // Every level is looser than the one before: what was legal there
        // stays legal. First only the parts that failed look for room.
        let mut relaxed = relaxed;
        if let Some((known_failed, known_poses, known_relaxed)) = best.as_ref()
            && !known_failed.is_empty()
        {
            // The parts the level before seated on marks lie there still.
            relaxed.constraints.on_marks = known_relaxed.constraints.on_marks.clone();
            let mut kept = known_poses.clone();
            let keep: Vec<usize> = (0..relaxed.components.len())
                .filter(|index| !known_failed.contains(index))
                .collect();
            let keeping_mark = mark();
            let mut failed = legal::legalize_keeping(&relaxed, &mut kept, known_failed, &keep);
            phase_line(debug, "legalize keeping", keeping_mark);
            if !failed.is_empty() && failed.len() <= (problem.components.len() / 50).max(3) {
                let mut evicted = kept.clone();
                let evict_mark = mark();
                let again = legal::evict_for(&relaxed, &mut evicted, &failed);
                phase_line(debug, "evict", evict_mark);
                if again.len() < failed.len() {
                    if debug {
                        eprintln!(
                            "placer: eviction seated {} of {} parts",
                            failed.len() - again.len(),
                            failed.len()
                        );
                    }
                    kept = evicted;
                    failed = again;
                }
            }
            // Still without room, with no halo, spacing or grid left to
            // give (only the edge and courtyard levels remain): the
            // level's own pass with those parts first, from the anneal
            // that gave the kept placement (the kept parts may have to
            // move: MIDAS-MK1's J105 needs J103's spot, J103 J108's, and
            // J108 a spot it has only once bodies may reach past the edge).
            if !failed.is_empty()
                && halo_scale == 0.0
                && spacing_scale == 0.0
                && grid == fine
                && failed.len() <= (problem.components.len() / 50).max(3)
                && !out_of_work(0)
                && let Some(annealed) = best_annealed.as_ref()
            {
                let retry_mark = mark();
                let (retry, again) = legal::legalize_stuck_first(
                    &relaxed,
                    annealed,
                    &failed,
                    STUCK_FIRST_PASSES,
                    debug,
                );
                phase_line(debug, "legalize first", retry_mark);
                if again.len() < failed.len() {
                    kept = retry;
                    failed = again;
                }
            }
            seat_on_marks(
                &mut relaxed,
                &mut kept,
                &mut failed,
                pass_work,
                config.work_limit,
                debug,
            );
            if debug {
                eprintln!(
                    "placer: level halo {halo_scale} (small {small_factor}) spacing {spacing_scale} grid {grid} inset {inset} tight {tight}: kept placement, {:.1}s, {} failed",
                    level_started.elapsed().as_secs_f64(),
                    failed.len()
                );
            }
            walk.push((level, failed.clone(), work() - work_started));
            if failed.len() < known_failed.len() {
                best_level = level;
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
                    pad_clearance: kicad_level(config, level, courtyard_level),
                    halos: None,
                    on_marks: relaxed.constraints.on_marks.clone(),
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
        // A fresh placement: no part lies on a mark yet.
        relaxed.constraints.on_marks.clear();
        let mut poses = global_poses.clone();
        let level_started = std::time::Instant::now();
        let anneal_mark = mark();
        if !skip_anneal {
            anneal::anneal(&relaxed, &mut poses, &anneal_config);
            // A pair's series parts turned to face the pair's ends before
            // the legaliser seats them (`facing`; the refinement's turn
            // finds no room where neighbours sit tight).
            facing::turn_to_face(&mut relaxed, &mut poses);
        }
        phase_line(debug, "anneal", anneal_mark);
        let annealed = poses.clone();
        let annealed_for_trace = annealed.clone();
        let anneal_seconds = level_started.elapsed().as_secs_f64();
        let legalize_mark = mark();
        let mut failed = legal::legalize(&relaxed, &mut poses);
        pass_work = work() - legalize_mark.1;
        phase_line(debug, "legalize", legalize_mark);
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
        // The parts that found no room go first in a second pass (not past
        // the work budget).
        if !failed.is_empty() && !out_of_work(0) {
            let mut retry = annealed;
            let retry_mark = mark();
            let again = legal::legalize_first(&relaxed, &mut retry, &failed);
            phase_line(debug, "legalize first", retry_mark);
            if again.len() < failed.len() {
                poses = retry;
                failed = again;
            }
        }
        // A few still without room: take a spot from movable parts and find
        // those new ones.
        if !failed.is_empty() && failed.len() <= (problem.components.len() / 50).max(3) {
            let mut evicted = poses.clone();
            let evict_mark = mark();
            let again = legal::evict_for(&relaxed, &mut evicted, &failed);
            phase_line(debug, "evict", evict_mark);
            if again.len() < failed.len() {
                if debug {
                    eprintln!(
                        "placer: eviction seated {} of {} parts",
                        failed.len() - again.len(),
                        failed.len()
                    );
                }
                poses = evicted;
                failed = again;
            }
        }
        seat_on_marks(
            &mut relaxed,
            &mut poses,
            &mut failed,
            pass_work,
            config.work_limit,
            debug,
        );
        walk.push((level, failed.clone(), work() - work_started));
        let better = best.as_ref().is_none_or(|(unplaced, kept, _)| {
            failed.len() < unplaced.len()
                || (failed.len() == unplaced.len()
                    && problem.wirelength(&poses) < problem.wirelength(kept))
        });
        let done = failed.is_empty();
        if better {
            best_level = level;
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
                pad_clearance: kicad_level(config, level, courtyard_level),
                halos: None,
                on_marks: relaxed.constraints.on_marks.clone(),
            });
            best = Some((failed, poses, relaxed));
            best_annealed = Some(annealed_for_trace);
        }
        last_level_work = Some(work() - level_mark);
        if done {
            break;
        }
    }
    // Parts still without room when the budget ended the levels (the
    // fast no of an overfull board, a deadline): those the marks alone
    // keep out lie on them (hurried past the budget, as every pass is).
    if let Some((failed, poses, relaxed)) = best.as_mut()
        && !failed.is_empty()
    {
        let before = failed.len();
        seat_on_marks(relaxed, poses, failed, pass_work, config.work_limit, debug);
        if failed.len() < before
            && let Some(relaxation) = relaxation.as_mut()
        {
            relaxation.on_marks = relaxed.constraints.on_marks.clone();
        }
    }
    let (unplaced, mut poses, relaxed) = best.expect("at least one relaxation level ran");
    let wirelength_legal = problem.wirelength(&poses);
    let mut stage_poses = vec![("start", global_start), ("global", global_poses.clone())];
    if let Some(annealed) = best_annealed {
        stage_poses.push(("annealed", annealed));
    }
    stage_poses.push(("legal", poses.clone()));
    frames.push(Frame {
        iteration: global.iterations + 1,
        overflow: global.overflow,
        wirelength: wirelength_legal,
        poses: poses.clone(),
    });
    let finishing = std::time::Instant::now();
    finish(
        &relaxed,
        &mut poses,
        config.refine_passes,
        bus_groups.as_deref(),
        debug,
    );
    if debug {
        eprintln!(
            "placer: refinement and relation repair {:.1}s",
            finishing.elapsed().as_secs_f64()
        );
    }
    // Pads without a way out: the router's test of the placement, and
    // the moves that open the pads other parts close.
    let exits = config.pad_exits.as_ref().and_then(|exits| {
        let exits_mark = mark();
        let before = work();
        let mut outcome = exits::repair(&relaxed, &mut poses, &unplaced, exits);
        // Its work is the outcome's, not the seed's: the caller charges it
        // to the clock beside the seeds' (`ExitOutcome::work`), so that
        // the seeds a race keeps do not depend on the switch.
        let spent = work() - before;
        add_work_back(spent);
        if let Some(outcome) = outcome.as_mut() {
            outcome.work = spent;
        }
        if debug && let Some(outcome) = &outcome {
            eprintln!(
                "placer: pad exits: {} closed pads, {} moves ({} candidates), {} still closed",
                outcome.closed_before,
                outcome.moves.len(),
                outcome.tried,
                outcome.closed.len()
            );
        }
        phase_line(debug, "pad exits", exits_mark);
        outcome
    });
    let problem = &relaxed;
    let wirelength_final = problem.wirelength(&poses);
    frames.push(Frame {
        iteration: global.iterations + 2,
        overflow: global.overflow,
        wirelength: wirelength_final,
        poses: poses.clone(),
    });
    problem.sync_followers(&mut poses);
    stage_poses.push(("final", poses.clone()));
    let illegal = legal::illegal_components(problem, &poses);
    if debug {
        for &index in &illegal {
            let mut met: Vec<String> = Vec::new();
            if !legal::is_legal(problem, &poses, index, poses[index], std::iter::empty()) {
                met.push("outline or hard constraint".into());
            }
            for other in 0..problem.components.len() {
                if other != index
                    && let Some(kind) =
                        legal::conflict_kind(problem, &poses, index, poses[index], other)
                {
                    met.push(format!("{} {kind}", problem.components[other].name));
                }
            }
            eprintln!(
                "placer: illegal {} at {:?}: {}",
                problem.components[index].name,
                poses[index],
                met.join(", ")
            );
        }
    }
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
        stage_poses,
        global_histories: vec![("global", global_history)],
        overfull: overfull_sides,
        exits,
        level: best_level,
        skeleton: None,
        walk,
    }
}

#[cfg(test)]
mod level_tests {
    use super::skip_to_tight;

    #[test]
    fn the_levels_go_on_to_tight_bodies_only_when_many_parts_lack_room_and_work_runs_short() {
        // OpenESC's panel after its spacing level: 27 of 482 parts, 436 s
        // left, a level 164 s, two levels before the tight one.
        assert!(skip_to_tight(27, 482, 436, 164, 2));
        // Enough work for every level before it.
        assert!(!skip_to_tight(27, 482, 700, 164, 2));
        // Few parts without room: the next levels' passes seat them.
        assert!(!skip_to_tight(3, 100, 10, 164, 2));
        assert!(!skip_to_tight(9, 482, 10, 164, 2));
        // No level ran yet.
        assert!(!skip_to_tight(27, 482, 10, 0, 2));
    }
}

#[cfg(test)]
mod check_mode_tests {
    use super::*;

    /// `legal`'s random boards (parts of every kind the legality test
    /// knows, cutouts with outlines, marks, linked pairs) on a board of
    /// sixteen edges, their pins on ten nets, with the crossings and the
    /// escape terms on: every cache that check mode checks is in use.
    fn board(round: usize) -> Problem {
        let mut state = 0x5DEE_CE66_D1CE_4E5Bu64 ^ round as u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let (mut problem, _) = crate::legal::tests::random_board(round, &mut next);
        let nets = 10;
        for component in &mut problem.components {
            for pin in &mut component.pins {
                pin.net = (next() * nets as f64) as usize;
            }
        }
        problem.net_weights = vec![1.0; nets];
        // A ragged outline (the board's `Outline` by bands from 12 edges).
        problem.outline = (0..16)
            .map(|index| {
                let angle = index as f64 / 16.0 * std::f64::consts::TAU;
                let radius = if index % 2 == 0 { 24.0 } else { 21.0 };
                [15.0 + radius * angle.cos(), 15.0 + radius * angle.sin()]
            })
            .collect();
        problem.crossings = Some(std::sync::Arc::new(crossings::Crossings {
            counted: vec![true; nets],
            weight: 2.0,
            legalise: true,
            refine: true,
            anneal: true,
        }));
        problem.escape = Some(std::sync::Arc::new(escape::Escape {
            pitch: 0.5,
            layers: 2.0,
            counted: vec![true; nets],
            weight: 1.0,
            inflate: 1.0,
            turn_tolerance: 0.05,
            measured: None,
            measure_every: 0,
        }));
        problem
    }

    /// Places `board(round)` with check mode off and on (on this thread
    /// alone: the tests of each round run side by side): the same poses,
    /// bit for bit, and the same work booked. A verification mode must not
    /// change what it verifies: a check that booked its direct
    /// computation's work moved the anneal's stages and the placement's
    /// stops (eternal-keypad, 2026-11-04).
    fn places_as_without_it(round: usize) {
        let problem = board(round);
        let mut config = Config::new();
        // Through the anneal, the legaliser and the refinement (the
        // escape term's prepared pads among its candidates).
        config.work_limit = Some(400_000_000);
        config.anneal.seed = round as u64 + 1;
        let plain = with_check_mode(false, || place(&problem, &config));
        let checked = with_check_mode(true, || place(&problem, &config));
        let bits = |placement: &Placement| {
            placement
                .poses
                .iter()
                .map(|pose| {
                    (
                        pose.position[0].to_bits(),
                        pose.position[1].to_bits(),
                        pose.angle.to_bits(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(plain.work, checked.work, "round {round}: work booked");
        let moved = (0..problem.components.len())
            .filter(|part| bits(&plain)[*part] != bits(&checked)[*part])
            .count();
        assert_eq!(moved, 0, "round {round}: parts placed otherwise");
        assert_eq!(
            (plain.unplaced, plain.illegal),
            (checked.unplaced, checked.illegal),
            "round {round}"
        );
    }

    #[test]
    fn check_mode_places_as_without_it() {
        places_as_without_it(0);
    }

    #[test]
    fn check_mode_places_as_without_it_with_courtyard_spacing() {
        places_as_without_it(1);
    }

    #[test]
    fn check_mode_places_as_without_it_with_pad_mask_gaps() {
        places_as_without_it(2);
    }

    #[test]
    fn check_mode_places_as_without_it_with_far_side_pads_only() {
        places_as_without_it(3);
    }
}

#[cfg(test)]
mod start_tests {
    use super::*;
    use crate::problem::{Component, Pin, Side};

    fn part(net: usize, fixed: bool) -> Component {
        Component {
            name: String::new(),
            body_center: [0.0, 0.0],
            body_size: [2.0, 1.0],
            round: false,
            halo: 0.0,
            pins: vec![
                Pin {
                    offset: [-0.6, 0.0],
                    net,
                },
                Pin {
                    offset: [0.6, 0.0],
                    net: net + 1,
                },
            ],
            side: Side::Front,
            fixed,
            angle_options: vec![0.0, 90.0, 180.0, 270.0],
            far_side: Vec::new(),
            far_copper: Vec::new(),
            far_open: Default::default(),
            far_courtyard: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: Vec::new(),
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
            hole_rings: Vec::new(),
            copper_graphics: Vec::new(),
            courtless: Vec::new(),
            slots: Vec::new(),
        }
    }

    /// Eight movable parts in a chain of nets (part i on nets i and i + 1)
    /// and two fixed ones, on 40 x 30 mm, all stacked at the board's
    /// centre as an import leaves them; and a legal spread arrangement
    /// on the grid (the designer's, for the tests).
    fn board() -> (Problem, Vec<Pose>) {
        let components: Vec<Component> = (0..10).map(|i| part(i, i >= 8)).collect();
        let stacked: Vec<Pose> = (0..10)
            .map(|i| Pose {
                position: if i >= 8 {
                    [5.0 + 30.0 * (i - 8) as f64, 15.0]
                } else {
                    [20.0, 15.0]
                },
                angle: 0.0,
            })
            .collect();
        let spread: Vec<Pose> = (0..10)
            .map(|i| {
                if i >= 8 {
                    stacked[i]
                } else {
                    Pose {
                        position: [8.0 + 4.0 * (i % 4) as f64, 9.0 + 6.0 * (i / 4) as f64],
                        angle: if i % 2 == 0 { 0.0 } else { 90.0 },
                    }
                }
            })
            .collect();
        let problem = Problem {
            outline: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
            components,
            net_weights: vec![1.0; 11],
            poses: stacked,
            spacing: 0.5,
            grid: 0.25,
            edge_margin: 0.5,
            min_spacing: 0.2,
            far_side_pads_only: false,
            escape: None,
            crossings: None,
            pieces: Vec::new(),
            constraints: Default::default(),
        };
        (problem, spread)
    }

    fn stage<'a>(placement: &'a Placement, name: &str) -> &'a [Pose] {
        &placement
            .stage_poses
            .iter()
            .find(|(stage, _)| *stage == name)
            .unwrap_or_else(|| panic!("no stage {name}"))
            .1
    }

    fn started(stage: StartStage, poses: &[Pose], parts: Vec<bool>) -> Config {
        let mut config = Config::new();
        config.start = Some(std::sync::Arc::new(Start {
            poses: poses.to_vec(),
            parts,
            stage,
        }));
        config
    }

    /// Without the global placement the stages after it start from the
    /// start's poses, angles included; with neither the global placement
    /// nor the anneal, a legal start is a fixed point of the legaliser.
    #[test]
    fn a_start_past_the_global_placement_begins_at_its_poses() {
        let (problem, spread) = board();
        for stage_name in [StartStage::Anneal, StartStage::Legalize] {
            let config = started(stage_name, &spread, vec![true; 10]);
            let placement = place(&problem, &config);
            assert_eq!(stage(&placement, "global"), &spread[..], "{stage_name:?}");
            assert!(placement.illegal.is_empty() && placement.unplaced.is_empty());
        }
        let placement = place(
            &problem,
            &started(StartStage::Legalize, &spread, vec![true; 10]),
        );
        // No anneal: the legaliser's input is the start, and keeps it.
        assert_eq!(stage(&placement, "annealed"), &spread[..]);
        assert_eq!(stage(&placement, "legal"), &spread[..]);
    }

    /// Without the anneal the legaliser seats the global placement's
    /// poses: the annealed stage is the global one.
    #[test]
    fn without_the_anneal_the_legaliser_seats_the_global_placement() {
        let (problem, _) = board();
        let mut config = Config::new();
        config.skip_anneal = true;
        let placement = place(&problem, &config);
        assert_eq!(stage(&placement, "annealed"), stage(&placement, "global"));
        assert!(placement.illegal.is_empty() && placement.unplaced.is_empty());
        let annealed = place(&problem, &Config::new());
        assert_ne!(stage(&annealed, "annealed"), stage(&annealed, "global"));
    }

    /// The global placement's warm start: the parts it names begin at
    /// their centres, the others where they begin without it (the start's
    /// noise is drawn for every part all the same).
    #[test]
    fn the_global_placements_warm_parts_begin_at_their_centres() {
        let (mut problem, spread) = board();
        let parts: Vec<bool> = (0..10).map(|i| i % 3 == 0).collect();
        let cold = global::global_place(
            &problem,
            &GlobalConfig {
                max_iterations: 0,
                ..GlobalConfig::default()
            },
        );
        for (index, pose) in spread.iter().enumerate() {
            if parts[index] {
                problem.poses[index] = *pose;
            }
        }
        let warm = global::global_place(
            &problem,
            &GlobalConfig {
                max_iterations: 0,
                warm: Some(std::sync::Arc::new(parts.clone())),
                ..GlobalConfig::default()
            },
        );
        for index in 0..8 {
            let component = &problem.components[index];
            let center = component.center(warm.poses[index]);
            if parts[index] {
                let expected = component.center(spread[index]);
                assert!(
                    (center[0] - expected[0]).abs() < 1.0e-9
                        && (center[1] - expected[1]).abs() < 1.0e-9,
                    "part {index}: {center:?} against {expected:?}"
                );
                assert_eq!(warm.poses[index].angle, spread[index].angle);
            } else {
                assert_eq!(warm.poses[index], cold.poses[index], "part {index}");
            }
        }
    }

    /// The objective's wire terms are the problem's wirelength, and its
    /// overlap counts only parts that meet.
    #[test]
    fn the_objective_splits_the_wirelength_and_sees_overlap() {
        let (problem, spread) = board();
        let apart = anneal::objective(&problem, &spread);
        let sum: f64 = apart.wirelength.iter().sum();
        assert!((sum - problem.wirelength(&spread)).abs() < 1.0e-9);
        assert_eq!(apart.half_perimeter, apart.wirelength);
        assert_eq!(apart.wirelength[1] + apart.wirelength[2], 0.0);
        assert_eq!((apart.overlap, apart.overlapping), (0.0, 0));
        assert_eq!(apart.crossings_all, None);
        let stacked = anneal::objective(&problem, &problem.poses);
        assert!(stacked.overlap > 1.0 && stacked.overlapping == 8);
    }
}

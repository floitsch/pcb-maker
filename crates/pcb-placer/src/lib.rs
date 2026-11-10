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
mod place;
pub mod problem;
pub mod skeleton;
pub mod stacks;
pub mod view;

pub use global::{Frame, GlobalConfig};
use place::{relax_level, relaxation_levels};
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
    /// Bus-group moves (`groups`, docs/placer.md "Bus-group moves"): banks of
    /// parts on one bus and parts with their passives move, turn, trade
    /// places and reorder as units in the anneal and the refinement. Off
    /// (`None`) by default.
    pub bus_groups: Option<groups::Settings>,
    /// KiCad's legality at the last level (where courtyards may touch):
    /// courtyards touch where the pads
    /// keep the copper clearance pad to pad, an unheld part's pads keep
    /// the edge margin, the board's silkscreen marks the project does not
    /// rate an error over a part are no obstacle, and a lead through the
    /// board keeps the copper clearance from the far side's pads
    /// (docs/placer.md, "The legality model against KiCad", the molecules).
    pub kicad_last_level: bool,
    /// Pads without a way out (`exits`, docs/placer.md "Pads without a way
    /// out"): after the placement, the router's test of every pad's way out
    /// on the statics, and moves or turns of the parts whose pads another
    /// part or the edge closes. Off (`None`) by default.
    pub pad_exits: Option<exits::PadExits>,
    /// The first relaxation level the placement tries (`relaxation_levels`;
    /// the last if there are fewer): the race over skeletons places the
    /// passives from the skeleton's level on, so that the held skeleton is
    /// legal at theirs (`skeleton`). 0 by default: every level.
    pub first_level: usize,
    /// A diagnostic start (`PCB_PLACER_START`, docs/placer.md "Where the
    /// arrangement is lost"): the stages from `Start::stage` on run from
    /// given poses (the designer's). `None` by default: the placer's own
    /// start.
    pub start: Option<std::sync::Arc<Start>>,
    /// A diagnostic (`PCB_PLACER_SKIP_ANNEAL`, docs/placer.md "Where the
    /// arrangement is lost"): no anneal at any level, each level's legaliser
    /// seats the global placement's poses (or the start's). Off by default.
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

/// Places `problem`'s movable parts: the halos fitted to the board, the
/// global placement, the walk down the relaxation levels until every part
/// has room, then the refinement of the best level's placement (the stages
/// in `place/`).
pub fn place(problem: &Problem, config: &Config) -> Placement {
    let fitted = place::prepare(problem, config);
    let wirelength_initial = fitted.wirelength_initial;
    let placing = place::Placing::new(&fitted, config);
    // The passes check the budget inside too (a part without room
    // searched the whole board at every level: urchin on 80 x 40 mm took
    // 11, 10 and 8 s of a 10 s budget in its first level's passes).
    let _stop = StopGuard::set(placing.anneal_config.stop_at_work);
    let global = placing.global_stage();
    let wirelength_global = fitted.problem.wirelength(&global.poses);
    let walk = placing.walk_levels(&global.poses);
    placing.conclude(walk, global, wirelength_initial, wirelength_global)
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

    /// `place`'s first stage: a global start's named parts and a later
    /// start's every movable part begin at the start's poses (a fixed part
    /// keeps its own), and the stages the start skips are skipped; a start
    /// for another problem is none.
    #[test]
    fn the_preparation_takes_a_starts_poses_and_skips_its_stages() {
        let (problem, spread) = board();
        let mut moved = spread.clone();
        moved[8].position = [1.0, 1.0];
        let parts: Vec<bool> = (0..10).map(|i| i % 2 == 0).collect();
        let expect = [
            (StartStage::Global, false, false),
            (StartStage::Anneal, true, false),
            (StartStage::Legalize, true, true),
        ];
        for (stage_name, skip_global, skip_anneal) in expect {
            let config = started(stage_name, &moved, parts.clone());
            let fitted = place::prepare(&problem, &config);
            let placing = place::Placing::new(&fitted, &config);
            assert_eq!(
                (placing.skip_global, placing.skip_anneal),
                (skip_global, skip_anneal),
                "{stage_name:?}"
            );
            for index in 0..8 {
                if parts[index] || stage_name != StartStage::Global {
                    assert_eq!(fitted.problem.poses[index], spread[index], "{index}");
                } else {
                    assert_ne!(fitted.problem.poses[index], spread[index], "{index}");
                }
            }
            assert_eq!(fitted.problem.poses[8], problem.poses[8]);
        }
        let config = started(StartStage::Legalize, &moved[..9], vec![true; 9]);
        let fitted = place::prepare(&problem, &config);
        let placing = place::Placing::new(&fitted, &config);
        assert!(fitted.start.is_none() && !placing.skip_global && !placing.skip_anneal);
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

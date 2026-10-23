// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! PCB component placement: electrostatic global placement, legalization,
//! and wirelength-driven detailed placement.

pub mod anneal;
mod buckets;
pub mod constraints;
pub mod escape;
pub mod global;
pub mod guidance;
pub mod legal;
pub mod orient;
pub mod problem;
pub mod room;
pub mod view;

pub use global::{Frame, GlobalConfig};
pub use problem::{Component, Pin, Point, Pose, Problem, Side};

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
    /// Routing room from congestion (`room`): after placing with the
    /// pin-count halos, each part's halo is rescaled by the routing demand
    /// around it, within the same halo area, and the placement is annealed
    /// again from there. Off (`None`) by default.
    pub routing_room: Option<room::RoutingRoom>,
    /// The orientation search starts from a canonical pose (`orient`):
    /// angle options in a fixed order, every movable part that may turn at
    /// the first of them, so that the placement does not depend on the
    /// angles the board held (0 degrees from an import, the designer's in
    /// a stack). Off: the parts start at the angles they hold, options in
    /// the order given (the placer before 2026-10-15).
    pub orientation_search: bool,
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
            routing_room: None,
            orientation_search: true,
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
    /// The poses after each stage (global, annealed, legal, final), for
    /// the measurement of where wirelength goes (`PCB_PLACER_TRACE`).
    pub stage_poses: Vec<(&'static str, Vec<Pose>)>,
    /// Routing room was asked for and could not seat every part with its
    /// halos: the placement has the pin-count halos. Such a placement
    /// ranks after those with routing room (its shorter wires are the
    /// halos it did not get).
    pub routing_room_failed: bool,
    /// Even the tightest bodies need more area than a side has: no
    /// packing places every part, and the placer said so at once.
    pub overfull: bool,
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
    /// Per component, the halo the placement gave it, where it is not the
    /// pin-count halo times the scales (`Config::routing_room`).
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
        problem.constraints.courtyard_spacing = self.courtyard_spacing;
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
            // Shared halos: each gap holds one halo, half of it per part.
            let sides = if problem.shared_halos { 1.0 } else { 2.0 };
            let margin = sides * scale_of(component, scales) * component.halo + problem.spacing;
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
        let counted = (work() - mark.1) as f64 / work_per_second();
        eprintln!(
            "placer phase {name}: {seconds:.2} s, cpu {cpu:.2} s, work {counted:.2} s ({:.2})",
            counted / cpu.max(1.0e-6)
        );
    }
}

type Mark = (std::time::Instant, u64, Option<f64>);

fn mark() -> Mark {
    (std::time::Instant::now(), work(), thread_cpu_seconds())
}

/// Refinement, edge-copper centring, relation repair and alignment: what
/// follows legalization.
fn finish(problem: &Problem, poses: &mut [Pose], passes: usize, debug: bool) {
    let refine_mark = mark();
    // Escape demand: one state through the refinements, its measured
    // correction taken now and every so many kept moves.
    let mut escape = problem.escape.as_deref().map(|escape| {
        let mut state = escape::EscapeState::new(problem, escape, poses, |_| true);
        state.measure_from_now(poses);
        state
    });
    legal::refine_with(problem, poses, passes, escape.as_mut());
    phase_line(debug, "refine", refine_mark);
    let repair_mark = mark();
    legal::center_edge_copper(problem, poses);
    // Moving one part can break a relation repaired before; repeat while
    // it helps.
    for _ in 0..5 {
        if legal::repair_relations(problem, poses) == 0 {
            break;
        }
        legal::refine_with(problem, poses, passes, escape.as_mut());
    }
    // Parts of a kind turned alike where it costs next to nothing.
    legal::align_orientations(problem, poses, 0.5);
    phase_line(debug, "repair and align", repair_mark);
    if let Some(state) = &escape {
        eprintln!(
            "placer: escape demand: {} measurements, {} kept moves",
            state.measurements,
            state.kept_moves()
        );
    }
}

/// Debugging aid: the area the movable parts' halos take.
fn halos_area(problem: &Problem) -> f64 {
    problem
        .components
        .iter()
        .filter(|component| !component.fixed && component.hollow.is_empty())
        .map(|component| {
            let (width, height) = (component.body_size[0], component.body_size[1]);
            (width + 2.0 * component.halo) * (height + 2.0 * component.halo) - width * height
        })
        .sum()
}

/// A relaxation level of `place`: halo scale, spacing scale, grid, edge
/// inset, tight bodies, edge copper, edge margin, and the factor on the
/// halos of parts with fewer than `MANY_PINS` pins.
type Level = (f64, f64, f64, bool, bool, bool, f64, f64);

pub fn place(problem: &Problem, config: &Config) -> Placement {
    // The pin-count halos as asked for, before the board's fit.
    let requested: Vec<f64> = problem
        .components
        .iter()
        .map(|component| component.halo)
        .collect();
    let (mut fitted, fit_scale) = fit_halos(problem, config.maximum_utilization);
    fitted.constraints.link_pairs();
    let wirelength_initial = fitted.wirelength(&fitted.poses);
    if config.orientation_search {
        orient::canonical_start(&mut fitted);
    }
    let problem = &fitted;
    let debug = std::env::var_os("PCB_PLACER_DEBUG").is_some();
    let started = std::time::Instant::now();
    let work_started = work();
    let stop_at_work = config.work_limit.map(|limit| work_started + limit);
    let mut anneal_config = config.anneal.clone();
    anneal_config.stop_at_work = stop_at_work.or(anneal_config.stop_at_work);
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
    let global = global::global_place(problem, &config.global);
    phase_line(debug, "global", global_mark);
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
    let mut relaxation = None;
    // The work of the last level's full legalization: what a pass over
    // every part costs (`seat_on_marks`).
    let mut pass_work = 0;
    // The annealed poses the best placement was legalized from.
    let mut best_annealed: Option<Vec<Pose>> = None;
    // The grid and bodies the last anneal ran with.
    let mut annealed_shape: Option<(f64, bool)> = None;
    let level_count = levels.len();
    // The problem at a level: its halos, spacing, grid and bodies.
    let relax = |level: usize,
                 (
        halo_scale,
        spacing_scale,
        grid,
        inset,
        tight,
        edge_copper,
        edge_rule,
        small_factor,
    ): Level| {
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
        relaxed
    };
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
    for (
        level,
        (halo_scale, spacing_scale, grid, inset, tight, edge_copper, edge_rule, small_factor),
    ) in levels.into_iter().enumerate()
    {
        // Past the caller's deadline the best placement so far stands.
        if best.is_some() && out_of_budget() {
            break;
        }
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
        anneal::anneal(&relaxed, &mut poses, &anneal_config);
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
        let better = best.as_ref().is_none_or(|(unplaced, kept, _)| {
            failed.len() < unplaced.len()
                || (failed.len() == unplaced.len()
                    && problem.wirelength(&poses) < problem.wirelength(kept))
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
                halos: None,
                on_marks: relaxed.constraints.on_marks.clone(),
            });
            best = Some((failed, poses, relaxed));
            best_annealed = Some(annealed_for_trace);
        }
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
    let (unplaced, mut poses, mut relaxed) = best.expect("at least one relaxation level ran");
    let wirelength_legal = problem.wirelength(&poses);
    let mut stage_poses = vec![("global", global_poses.clone())];
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
    finish(&relaxed, &mut poses, config.refine_passes, debug);
    if debug {
        eprintln!(
            "placer: refinement and relation repair {:.1}s",
            finishing.elapsed().as_secs_f64()
        );
    }
    // Routing room: the halos follow the routing demand around each part
    // at this placement, and the parts anneal again from here.
    let room_halos = match &config.routing_room {
        Some(room)
            if unplaced.is_empty()
                && relaxed
                    .components
                    .iter()
                    .any(|component| !component.fixed && component.halo > 0.0)
                && !out_of_budget() =>
        {
            let halos = room::halos(&relaxed, &poses, room, &requested);
            let unchanged = relaxed
                .components
                .iter()
                .zip(&halos)
                .all(|(component, halo)| component.halo == *halo);
            (!unchanged).then_some(halos)
        }
        _ => None,
    };
    let mut routing_room_failed = false;
    if let Some(mut halos) = room_halos {
        let room_mark = mark();
        let mut roomy = relaxed.clone();
        for (component, halo) in roomy.components.iter_mut().zip(&halos) {
            component.halo = *halo;
        }
        let mut again = poses.clone();
        // From a legal placement: small moves only.
        let mut reheated = anneal_config.clone();
        reheated.initial_radius = reheated.initial_radius.min(0.02);
        anneal::anneal(&roomy, &mut again, &reheated);
        let annealed = again.clone();
        let mut failed = legal::legalize(&roomy, &mut again);
        if !failed.is_empty() {
            let mut retry = annealed;
            let still = legal::legalize_first(&roomy, &mut retry, &failed);
            if still.len() < failed.len() {
                again = retry;
                failed = still;
            }
        }
        // The few parts still without room fall back to their halo at the
        // level, then to none (they are where the demand gathered them);
        // the others stay where they are.
        let unseated = failed.len();
        for fallback in [true, false] {
            if failed.is_empty() || failed.len() > (relaxed.components.len() / 50).max(3) {
                break;
            }
            for index in &failed {
                let halo = if fallback {
                    relaxed.components[*index].halo.min(halos[*index])
                } else {
                    0.0
                };
                halos[*index] = halo;
                roomy.components[*index].halo = halo;
            }
            let keep: Vec<usize> = (0..roomy.components.len())
                .filter(|index| !failed.contains(index))
                .collect();
            let mut retry = again.clone();
            let still = legal::legalize_keeping(&roomy, &mut retry, &failed, &keep);
            if still.len() < failed.len() {
                again = retry;
                failed = still;
            }
        }
        if debug {
            eprintln!(
                "placer: routing room: {unseated} parts without room after legalizing; halo area {:.0} to {:.0} mm2, {} parts without halo (before {}); {} failed",
                halos_area(&relaxed),
                halos_area(&roomy),
                roomy
                    .components
                    .iter()
                    .filter(|component| !component.fixed && component.halo == 0.0)
                    .count(),
                relaxed
                    .components
                    .iter()
                    .filter(|component| !component.fixed && component.halo == 0.0)
                    .count(),
                failed.len()
            );
        }
        if failed.is_empty() {
            finish(&roomy, &mut again, config.refine_passes, debug);
            relaxed.sync_followers(&mut poses);
            stage_poses.push(("pin_halos", poses.clone()));
            poses = again;
            relaxed = roomy;
            if let Some(relaxation) = relaxation.as_mut() {
                relaxation.halos = Some(std::sync::Arc::new(halos));
            }
        } else {
            routing_room_failed = true;
        }
        phase_line(debug, "routing room", room_mark);
    }
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
        routing_room_failed,
        overfull: overfull_sides,
    }
}

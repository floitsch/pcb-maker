// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The relaxation levels and their walk. Crowded boards cannot afford the
//! full comfort margins: the walk gives up halos, then the spacing, then
//! the grid, the edge and the courtyards' margins, level by level
//! (`relaxation_levels`), before giving up on a part. At each level it
//! runs (`Placing::walk_levels`): whether it runs at all (`Walk::admit`,
//! the overfull tests), the hurried seat of a hopeless board
//! (`Walk::hurried`), the kept placement of the level before with its
//! parts without room looking again (`Walk::keep_level`), and a fresh
//! anneal and legalisation (`Walk::fresh_level`); in the end the parts
//! still without room on the board's marks (`Walk::seat_leftovers`).

use super::Placing;
use crate::{
    Config, MANY_PINS, Pose, Problem, Relaxation, STOP, anneal, facing, legal, mark, out_of_work,
    overfull, phase_line, side_overfull, skip_to_tight, strips_overfull, work, work_per_second,
};

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

/// Passes of the legaliser with the parts that found no room first, each
/// adding the parts the pass before left without room
/// (`legal::legalize_stuck_first`), at a loosest level (no halo, spacing
/// or grid left to give) that keeps the placement of the level before,
/// when a few parts (as many as an eviction takes on) found no room among
/// the kept ones.
const STUCK_FIRST_PASSES: usize = 4;

/// A relaxation level of `place`: halo scale, spacing scale, grid, edge
/// inset, tight bodies, edge copper, edge margin, and the factor on the
/// halos of parts with fewer than `MANY_PINS` pins.
pub(crate) type Level = (f64, f64, f64, bool, bool, bool, f64, f64);

/// The relaxation levels a placement walks down, loosest comfort first,
/// and the index of the last, where courtyards may touch
/// (`Constraints::courtyard_spacing`).
pub(crate) fn relaxation_levels(problem: &Problem) -> (Vec<Level>, usize) {
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
pub(crate) fn kicad_level(config: &Config, level: usize, courtyard_level: usize) -> bool {
    config.kicad_last_level && level == courtyard_level
}

/// The problem at level `level` of `relaxation_levels`: its halos,
/// spacing, grid and bodies.
pub(crate) fn relax_level(
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

/// What the walk reads at every level: the levels, the global placement's
/// poses and what the walk's tests decide by.
struct Ladder<'a> {
    placing: &'a Placing<'a>,
    global_poses: &'a [Pose],
    courtyard_level: usize,
    level_count: usize,
    /// The finer grid of the last levels (`relaxation_levels`).
    fine: f64,
    /// Where the levels go on to when work runs short (`skip_to_tight`):
    /// the first level with tight bodies, or without them the first with
    /// no spacing on the fine grid.
    first_tight: Option<usize>,
}

impl Ladder<'_> {
    /// The problem at a level: its halos, spacing, grid and bodies.
    fn relax(&self, level: usize, shape: Level) -> Problem {
        let placing = self.placing;
        relax_level(
            placing.problem,
            placing.config,
            level,
            self.courtyard_level,
            shape,
        )
    }

    /// The relaxation a placement at `level` (its problem `relaxed`)
    /// needed, the halo scales with the halos' fit.
    fn relaxation(&self, level: usize, shape: Level, relaxed: &Problem) -> Relaxation {
        let (halo_scale, _, _, inset, tight, edge_copper, edge_rule, small_factor) = shape;
        let placing = self.placing;
        let fit_scale = placing.fit_scale;
        Relaxation {
            spacing: relaxed.spacing,
            grid: relaxed.grid,
            halo_scale: fit_scale[1] * halo_scale,
            small_halo_scale: fit_scale[0] * halo_scale * small_factor,
            edge_inset: inset,
            tight,
            edge_copper,
            edge_rule,
            courtyard_spacing: level == self.courtyard_level,
            pad_clearance: kicad_level(placing.config, level, self.courtyard_level),
            halos: None,
            on_marks: relaxed.constraints.on_marks.clone(),
        }
    }
}

/// Whether the walk runs a level, skips it or ends.
enum Next {
    Run,
    Skip,
    Stop,
}

/// What the level walk updates: the best placement so far, what the next
/// levels decide by, and what the placement reports of the walk.
pub(crate) struct Walk {
    /// The best placement so far: the parts it left without room, its
    /// poses and its level's problem.
    pub(super) best: Option<(Vec<usize>, Vec<Pose>, Problem)>,
    /// The relaxation `best` needed, and its level.
    pub(super) relaxation: Option<Relaxation>,
    pub(super) best_level: usize,
    /// The annealed poses the best placement was legalized from.
    pub(super) best_annealed: Option<Vec<Pose>>,
    /// The levels that ran (`Placement::walk`).
    pub(super) walk: Vec<(usize, Vec<usize>, u64)>,
    /// Even the last level's bodies need more area than a side has
    /// (`Placement::overfull`).
    pub(super) overfull_sides: bool,
    /// The work of the last level's full legalization: what a pass over
    /// every part costs (`seat_on_marks`).
    pass_work: u64,
    /// The grid and bodies the last anneal ran with.
    annealed_shape: Option<(f64, bool)>,
    /// The work the last level that ran took (`skip_to_tight`).
    last_level_work: Option<u64>,
    skipping_to_tight: bool,
}

impl Placing<'_> {
    /// Walks the relaxation levels from `global_poses` until a level seats
    /// every part, the budget ends or the levels do.
    pub(crate) fn walk_levels(&self, global_poses: &[Pose]) -> Walk {
        let problem = self.problem;
        let debug = self.debug;
        // The last level also snaps to a finer grid (`relaxation_levels`).
        let fine = problem.grid.min(0.1);
        let (levels, courtyard_level) = relaxation_levels(problem);
        let level_count = levels.len();
        // No packing fits the parts even at the loosest level (the last): the
        // movable bodies need more area than a side has, or more than strips
        // of the board can hold (`strips_overfull`). Then no level can place
        // them all, and the placer says so at once instead of annealing and
        // legalizing at every level in vain. Exact: a board that can be placed
        // never takes this way.
        let hopeless = {
            let last = relax_level(
                problem,
                self.config,
                level_count - 1,
                courtyard_level,
                *levels.last().expect("levels"),
            );
            side_overfull(&last, false) || strips_overfull(&last)
        };
        // Where the levels go on to when work runs short (`skip_to_tight`):
        // the first level with tight bodies, or without them the first with
        // no spacing on the fine grid; and the work the last level that ran
        // took.
        let first_tight = levels.iter().position(|level| level.4).or_else(|| {
            levels
                .iter()
                .position(|level| level.1 == 0.0 && level.2 == fine && level.2 < problem.grid)
        });
        let ladder = Ladder {
            placing: self,
            global_poses,
            courtyard_level,
            level_count,
            fine,
            first_tight,
        };
        let mut walk = Walk {
            best: None,
            relaxation: None,
            best_level: 0,
            best_annealed: None,
            walk: Vec::new(),
            overfull_sides: false,
            pass_work: 0,
            annealed_shape: None,
            last_level_work: None,
            skipping_to_tight: false,
        };
        for (level, shape) in levels.into_iter().enumerate() {
            let (halo_scale, spacing_scale, grid, inset, tight, _, _, small_factor) = shape;
            match walk.admit(&ladder, level) {
                Next::Run => {}
                Next::Skip => continue,
                Next::Stop => break,
            }
            let level_mark = work();
            if grid == fine
                && fine == problem.grid
                && spacing_scale == 0.0
                && !inset
                && walk.best.is_some()
            {
                // Same as the previous level.
                continue;
            }
            let relaxed = ladder.relax(level, shape);
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
            if hopeless {
                walk.hurried(&ladder, level, shape, relaxed);
                break;
            }
            let mut relaxed = relaxed;
            if walk.keep_level(&ladder, level, shape, &mut relaxed) {
                break;
            }
            if walk.few_left(&ladder, shape) {
                continue;
            }
            if walk.fresh_level(&ladder, level, shape, relaxed, level_mark) {
                break;
            }
        }
        walk.seat_leftovers(self);
        walk
    }
}

impl Walk {
    /// Whether the walk runs `level`: from `first_level` on, not past the
    /// budget once a placement stands, and not before the tight bodies
    /// once the walk goes on to them.
    fn admit(&mut self, ladder: &Ladder, level: usize) -> Next {
        let placing = ladder.placing;
        let problem = placing.problem;
        // From `first_level` on (the race over skeletons: the passives
        // from the held skeleton's level).
        if level < placing.config.first_level.min(ladder.level_count - 1) {
            return Next::Skip;
        }
        // Past the caller's deadline the best placement so far stands.
        if self.best.is_some() && placing.out_of_budget() {
            return Next::Stop;
        }
        // Many parts still without room and too little work left to try
        // every level before the tight bodies: on to them (OpenESC's
        // panel: 27 of 398 parts without room at the spacing level, each
        // looser grid and edge level 250-330 s of the 600, and the tight
        // level, which places it, never came). Where the rules keep
        // courtyards apart, on to the fine grid without spacing (the
        // laptop's power board: 35 parts without room after the halo
        // levels, placed on the fine grid with 4000 s, refused with 600).
        if !self.skipping_to_tight
            && let (Some(first_tight), Some(level_work), Some(stop), Some((failed, _, _))) = (
                ladder.first_tight,
                self.last_level_work,
                placing.stop_at_work,
                self.best.as_ref(),
            )
            && level < first_tight
            && skip_to_tight(
                failed.len(),
                problem.components.len(),
                stop.saturating_sub(work()),
                level_work,
                first_tight - level,
            )
        {
            self.skipping_to_tight = true;
            if placing.debug {
                eprintln!(
                    "placer: {} parts without room, {:.0} s of work left, a level {:.0} s: on to level {first_tight}",
                    failed.len(),
                    stop.saturating_sub(work()) as f64 / work_per_second(),
                    level_work as f64 / work_per_second()
                );
            }
        }
        if self.skipping_to_tight && ladder.first_tight.is_some_and(|first| level < first) {
            return Next::Skip;
        }
        Next::Run
    }

    /// Even the last level's bodies need more area than a side has (every
    /// level before was overfull too, so none ran): no packing places them
    /// all. A fast no: the parts are seated near where the global placement
    /// put them as far as there is room, without an anneal, and the passes
    /// after it stop at once (urchin's 80 parts on 80 x 40 mm took more
    /// than ten minutes before).
    fn hurried(&mut self, ladder: &Ladder, level: usize, shape: Level, relaxed: Problem) {
        let placing = ladder.placing;
        if placing.debug {
            eprintln!(
                "placer: the last level is overfull too: no packing fits, the parts are seated where there is room"
            );
        }
        self.overfull_sides = true;
        STOP.with(|stop| stop.set(Some(work())));
        // On the regular grid: the hurried search reaches 15 mm there.
        let mut relaxed = relaxed;
        relaxed.grid = placing.problem.grid;
        let mut poses = ladder.global_poses.to_vec();
        let failed = legal::legalize(&relaxed, &mut poses);
        self.walk
            .push((level, failed.clone(), work() - placing.work_started));
        self.best_level = level;
        self.relaxation = Some(ladder.relaxation(level, shape, &relaxed));
        self.best = Some((failed, poses, relaxed));
    }

    /// Every level is looser than the one before: what was legal there
    /// stays legal. First only the parts that failed look for room, around
    /// the kept placement (then by eviction, then with no halo, spacing or
    /// grid left to give, the level's own pass with them first from the
    /// kept placement's anneal, then on the board's marks). Whether the
    /// walk ends: every part seated, and no fresh anneal to try.
    fn keep_level(
        &mut self,
        ladder: &Ladder,
        level: usize,
        shape: Level,
        relaxed: &mut Problem,
    ) -> bool {
        let (halo_scale, spacing_scale, grid, inset, tight, _, _, small_factor) = shape;
        let placing = ladder.placing;
        let problem = placing.problem;
        let debug = placing.debug;
        let level_started = std::time::Instant::now();
        // Every level is looser than the one before: what was legal there
        // stays legal. First only the parts that failed look for room.
        if let Some((known_failed, known_poses, known_relaxed)) = self.best.as_ref()
            && !known_failed.is_empty()
        {
            // The parts the level before seated on marks lie there still.
            relaxed.constraints.on_marks = known_relaxed.constraints.on_marks.clone();
            let mut kept = known_poses.clone();
            let keep: Vec<usize> = (0..relaxed.components.len())
                .filter(|index| !known_failed.contains(index))
                .collect();
            let keeping_mark = mark();
            let mut failed = legal::legalize_keeping(relaxed, &mut kept, known_failed, &keep);
            phase_line(debug, "legalize keeping", keeping_mark);
            if !failed.is_empty() && failed.len() <= (problem.components.len() / 50).max(3) {
                let mut evicted = kept.clone();
                let evict_mark = mark();
                let again = legal::evict_for(relaxed, &mut evicted, &failed);
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
                && grid == ladder.fine
                && failed.len() <= (problem.components.len() / 50).max(3)
                && !out_of_work(0)
                && let Some(annealed) = self.best_annealed.as_ref()
            {
                let retry_mark = mark();
                let (retry, again) = legal::legalize_stuck_first(
                    relaxed,
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
                relaxed,
                &mut kept,
                &mut failed,
                self.pass_work,
                placing.config.work_limit,
                debug,
            );
            if debug {
                eprintln!(
                    "placer: level halo {halo_scale} (small {small_factor}) spacing {spacing_scale} grid {grid} inset {inset} tight {tight}: kept placement, {:.1}s, {} failed",
                    level_started.elapsed().as_secs_f64(),
                    failed.len()
                );
            }
            self.walk
                .push((level, failed.clone(), work() - placing.work_started));
            if failed.len() < known_failed.len() {
                self.best_level = level;
                self.relaxation = Some(ladder.relaxation(level, shape, relaxed));
                let done = failed.is_empty();
                self.best = Some((failed, kept, relaxed.clone()));
                // On a finer grid or with tighter bodies, a fresh anneal
                // may still beat the kept placement, whose last parts sit
                // wherever room was left (OpenESC 30x30: 1504 mm legal
                // against 881 for another seed's anneal).
                if done && (self.annealed_shape == Some((grid, tight)) || placing.out_of_budget()) {
                    return true;
                }
            }
        }
        false
    }

    /// Whether a few parts are still without room after a level of the
    /// same grid and bodies: the looser levels are what they need, not
    /// another full anneal.
    fn few_left(&self, ladder: &Ladder, shape: Level) -> bool {
        let (_, _, grid, _, tight, _, _, _) = shape;
        let problem = ladder.placing.problem;
        // A few parts still without room: the looser levels are what they
        // need, not another full anneal at each one (katia: four parts
        // failed at every level, 40-160 s each, and the deadline came
        // before the levels that might seat them).
        // A level with a finer grid or tighter bodies is a new problem,
        // though: its anneal seats what the coarser levels could not
        // (OpenESC's mini: 2-4 parts without room at the regular grid, all
        // seated by the anneal on the fine grid).
        if let Some((known_failed, _, _)) = self.best.as_ref()
            && !known_failed.is_empty()
            && known_failed.len() <= (problem.components.len() / 50).max(3)
            && self.annealed_shape == Some((grid, tight))
        {
            return true;
        }
        false
    }

    /// A fresh placement at `level`: the anneal from the global placement's
    /// poses (with the pair rows turned to face their pairs), the
    /// legalisation, a second pass with the parts without room first, an
    /// eviction for a few, the board's marks; kept where it beats the best
    /// so far. Whether the walk ends: every part seated.
    fn fresh_level(
        &mut self,
        ladder: &Ladder,
        level: usize,
        shape: Level,
        mut relaxed: Problem,
        level_mark: u64,
    ) -> bool {
        let (halo_scale, spacing_scale, grid, inset, tight, _, _, small_factor) = shape;
        let placing = ladder.placing;
        let problem = placing.problem;
        let debug = placing.debug;
        self.annealed_shape = Some((grid, tight));
        // A fresh placement: no part lies on a mark yet.
        relaxed.constraints.on_marks.clear();
        let mut poses = ladder.global_poses.to_vec();
        let level_started = std::time::Instant::now();
        let anneal_mark = mark();
        if !placing.skip_anneal {
            anneal::anneal(&relaxed, &mut poses, &placing.anneal_config);
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
        self.pass_work = work() - legalize_mark.1;
        phase_line(debug, "legalize", legalize_mark);
        if debug {
            let work_started = placing.work_started;
            eprintln!(
                "placer: level halo {halo_scale} (small {small_factor}) spacing {spacing_scale} grid {grid} inset {inset} tight {tight}: wirelength global {:.0}, annealed {:.0}, legalized {:.0}; anneal {anneal_seconds:.1}s, legalize {:.1}s, {} failed, work {} ({:.0}/s overall)",
                problem.wirelength(ladder.global_poses),
                problem.wirelength(&annealed),
                problem.wirelength(&poses),
                level_started.elapsed().as_secs_f64() - anneal_seconds,
                failed.len(),
                work() - work_started,
                (work() - work_started) as f64
                    / placing.started.elapsed().as_secs_f64().max(1.0e-3)
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
            self.pass_work,
            placing.config.work_limit,
            debug,
        );
        self.walk
            .push((level, failed.clone(), work() - placing.work_started));
        let better = self.best.as_ref().is_none_or(|(unplaced, kept, _)| {
            failed.len() < unplaced.len()
                || (failed.len() == unplaced.len()
                    && problem.wirelength(&poses) < problem.wirelength(kept))
        });
        let done = failed.is_empty();
        if better {
            self.best_level = level;
            self.relaxation = Some(ladder.relaxation(level, shape, &relaxed));
            self.best = Some((failed, poses, relaxed));
            self.best_annealed = Some(annealed_for_trace);
        }
        self.last_level_work = Some(work() - level_mark);
        done
    }

    /// Parts still without room when the budget ended the levels (the
    /// fast no of an overfull board, a deadline): those the marks alone
    /// keep out lie on them (hurried past the budget, as every pass is).
    fn seat_leftovers(&mut self, placing: &Placing) {
        if let Some((failed, poses, relaxed)) = self.best.as_mut()
            && !failed.is_empty()
        {
            let before = failed.len();
            seat_on_marks(
                relaxed,
                poses,
                failed,
                self.pass_work,
                placing.config.work_limit,
                placing.debug,
            );
            if failed.len() < before
                && let Some(relaxation) = self.relaxation.as_mut()
            {
                relaxation.on_marks = relaxed.constraints.on_marks.clone();
            }
        }
    }
}

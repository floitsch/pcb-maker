// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The end of `place`: the best level's placement refined, its relations
//! repaired and its orientations aligned (`finish`), its pads' ways out
//! opened (`Placing::pad_exits`), and the result with its checks
//! (`Placing::conclude`).

use super::Placing;
use super::levels::Walk;
use crate::{
    Frame, Placement, Pose, Problem, add_work_back, constraints, crossings, escape, exits, facing,
    global, groups, legal, mark, phase_line, work,
};

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

impl Placing<'_> {
    /// The placement from the walk's best level: the stage poses and
    /// frames, the refinement (`finish`), the pad exits and the checks of
    /// the result.
    pub(crate) fn conclude(
        &self,
        walk: Walk,
        global: global::GlobalResult,
        wirelength_initial: f64,
        wirelength_global: f64,
    ) -> Placement {
        let (problem, config, debug) = (self.problem, self.config, self.debug);
        let Walk {
            best,
            relaxation,
            best_level,
            best_annealed,
            walk,
            overfull_sides,
            ..
        } = walk;
        let mut frames = global.frames;
        let (unplaced, mut poses, relaxed) = best.expect("at least one relaxation level ran");
        let wirelength_legal = problem.wirelength(&poses);
        let mut stage_poses = vec![("start", global.start), ("global", global.poses.clone())];
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
            self.bus_groups.as_deref(),
            debug,
        );
        if debug {
            eprintln!(
                "placer: refinement and relation repair {:.1}s",
                finishing.elapsed().as_secs_f64()
            );
        }
        let exits = self.pad_exits(&relaxed, &mut poses, &unplaced);
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
            say_illegal(problem, &poses, &illegal);
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
            work: work() - self.work_started,
            stage_poses,
            global_histories: vec![("global", global.history)],
            overfull: overfull_sides,
            exits,
            level: best_level,
            skeleton: None,
            walk,
        }
    }

    /// Pads without a way out (`Config::pad_exits`): the router's test of
    /// the placement, and the moves that open the pads other parts close.
    fn pad_exits(
        &self,
        relaxed: &Problem,
        poses: &mut [Pose],
        unplaced: &[usize],
    ) -> Option<exits::ExitOutcome> {
        let debug = self.debug;
        // Pads without a way out: the router's test of the placement, and
        // the moves that open the pads other parts close.
        self.config.pad_exits.as_ref().and_then(|exits| {
            let exits_mark = mark();
            let before = work();
            let mut outcome = exits::repair(relaxed, poses, unplaced, exits);
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
        })
    }
}

/// The debug lines on the parts the result leaves illegal: what each
/// meets (`PCB_PLACER_DEBUG`).
fn say_illegal(problem: &Problem, poses: &[Pose], illegal: &[usize]) {
    for &index in illegal {
        let mut met: Vec<String> = Vec::new();
        if !legal::is_legal(problem, poses, index, poses[index], std::iter::empty()) {
            met.push("outline or hard constraint".into());
        }
        for other in 0..problem.components.len() {
            if other != index
                && let Some(kind) = legal::conflict_kind(problem, poses, index, poses[index], other)
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

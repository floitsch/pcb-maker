// Copyright (C) 2026 Toit contributors.

//! After the first route: whether the moves run at all, the move descent
//! (an experiment) and the footprint moves where nets fought, each a
//! reroute in place, kept when the board improves.

use super::candidates::Kept;
use super::placement::Copper;
use super::setup::{LayoutContext, WorkLedger};
use super::*;

/// The router the layout routes with, its result and the result's score
/// (open terminals, then vias and copper; `score`).
pub(super) struct Routing {
    pub router: core::router::Router,
    pub result: core::RoutingResult,
    pub best: (usize, f64),
}

/// What the first route decided for the steps after it.
pub(super) struct FirstRouteJudgement {
    /// Too many connections open for moves, the resume and the polish:
    /// the final ladder (or the narrow-signal attempt) gets the time.
    pub many_open: bool,
    /// Whether the narrow-signal step applies (`narrow_step_applies`).
    pub narrow_applies: bool,
}

/// What the move trials hand on: every trial, and whether the board kept
/// is one a trial polished.
pub(super) struct MovesOutcome {
    pub moves: Vec<KiCadBoardLayoutMove>,
    pub polished: bool,
}

/// The state of the move trials between two trials.
struct MoveTrials {
    moves: Vec<KiCadBoardLayoutMove>,
    since_improvement: usize,
    /// While connections are open, trials are judged on the open ones,
    /// which the polish (an exact cleanup of every net, up to minutes on a
    /// big board) does not change: it waits until the end. The first
    /// route was not polished either.
    polished: bool,
    /// Trials since a move last closed an open connection: when moves stop
    /// closing them, the final ladder is the better use of the time.
    since_fewer_open: usize,
    /// The parts tried lately.
    recently: Vec<usize>,
    /// With `global_guidance`'s `moves`: the parts' attribution in the
    /// global route of the kept board, recomputed when a move is kept.
    guided_attribution: Option<Vec<f64>>,
}

/// The parts a move may take: not fixed, with pins.
pub(super) fn movable_parts(problem: &crate::board_placer::LoweredPlacement) -> Vec<usize> {
    (0..problem.problem.components.len())
        .filter(|index| {
            !problem.problem.components[*index].fixed
                && !problem.problem.components[*index].pins.is_empty()
        })
        .collect()
}

/// Whether the board is too far from complete after its first route for
/// the moves (`open` connections open on the router's `board`), logged.
pub(super) fn judge_first_route(
    ctx: &LayoutContext,
    board: &core::Board,
    copper: &Copper,
    open: usize,
) -> FirstRouteJudgement {
    // Moves fine-tune a placement that nearly routes; with many
    // connections open the final ladder (pours as tracks or planes, finer
    // pitches) is the better use of the time, and it never ran when the
    // moves took it (Sisu: 108 unconnected in the layout, 20 when route
    // mode routed the same placement).
    // Where the ladder has no other way of connecting the pours to try
    // (no pours, or two layers), the moves are the only lever left (the
    // breadboard fence on two layers: 41 open, no move was ever tried).
    // A bounded round instead of none (at most four parts in a tenth of
    // the budget, tried 2026-10-21) lost time for nothing: DSP-ADAU1452 at
    // a third of its budget, 100 open after the first route, its three
    // trials rerouted 32 nets each and left 224 to 285 open (a reroute in
    // that congestion does not settle), none kept, 89 s of work more for
    // the same board.
    let terminals: usize = board.nets.iter().map(|net| net.terminals.len()).sum();
    let ladder_has_rungs = copper.connect && copper.layers.names.len() >= 4;
    let many_open = ladder_has_rungs && open > ((terminals as f64 * 0.05) as usize).max(10);
    // Where the narrow-signal step applies, a board still far from
    // complete after its first route skips the moves, the resume and the
    // polish too, and the narrow-signal attempt gets the time (below): the
    // step is what nearly completes such boards (jetson: 48 to 6 open in
    // route mode), and the moves and the resume took 388 s of jetson's
    // work for 31 to 25 open and left 218 s, too little for any ladder.
    let narrow_applies = crate::board_router::narrow_step_applies(&ctx.router_config);
    let far_open =
        narrow_applies && ladder_has_rungs && !many_open && far_from_complete(open, terminals);
    if many_open {
        eprintln!(
            "layout: {} open after the first route: no moves, the final ladder gets the time",
            open
        );
    } else if far_open {
        eprintln!(
            "layout: {} open after the first route (more than {}), the narrow-signal step applies: no moves, the narrow-signal attempt gets the time",
            open,
            far_open_limit(terminals)
        );
    }
    FirstRouteJudgement {
        many_open: many_open || far_open,
        narrow_applies,
    }
}

/// The move descent (an experiment, docs/architecture.md, "Off by default, still in the code"):
/// cheap trials, so it runs also where the moves below do not; where it
/// keeps moves, the router's routes as they stand are the result. Hands
/// on its report where it ran.
pub(super) fn descend(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    kept: &mut Kept,
    routing: &mut Routing,
    copper: &Copper,
) -> Result<Option<crate::move_descent::KiCadMoveDescentReport>, String> {
    let config = ctx.config;
    let Some(descent) = crate::move_descent::descent_config(config.move_descent.as_ref()) else {
        return Ok(None);
    };
    if !(routing.best.0 > 0 && !routing.router.past_deadline() && ctx.stop.allows(LayoutStep::Move))
    {
        return Ok(None);
    }
    let moves_budget = config
        .move_seconds
        .min((0.65 * config.total_seconds - ledger.work).max(0.0));
    let budget = descent.trial_share * moves_budget;
    ctx.arm(&mut routing.router, ledger.clock());
    let pool_excess = ledger.pool_excess;
    let report = crate::move_descent::descend(
        crate::move_descent::DescentState {
            pcb: &mut kept.pcb,
            board: &mut kept.board,
            problem: &mut kept.problem,
            router: &mut routing.router,
            work: &mut ledger.work,
        },
        &descent,
        budget,
        &ctx.router_config,
        copper.connect,
        config.placer.keep_board_marks,
        &|router, work| ctx.arm(router, work + pool_excess),
        &|| ctx.stop.allows(LayoutStep::Move),
    )?;
    if report.kept > 0 {
        routing.result = routing.router.finish_as_is();
        routing.best = score(&routing.result, &kept.board);
        eprintln!(
            "layout: the move descent kept {} moves: {} open (the router's count {})",
            report.kept, routing.best.0, report.open_after
        );
    }
    Ok(Some(report))
}

/// The footprint moves where nets fought (`moves` trials, none with
/// `many_open`): the most congested movable part not tried lately, up to
/// three legal poses near it each rerouted in place, the first that
/// improves the board kept. Ends at the patience, the moves' budget
/// (counted from `move_work_started`), the router's deadline or the wall
/// clock. Hands on the trials and whether the board kept is polished.
#[allow(
    clippy::too_many_arguments,
    reason = "the layout's state is threaded through by reference"
)]
pub(super) fn try_moves(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    kept: &mut Kept,
    routing: &mut Routing,
    copper: &Copper,
    movable: &[usize],
    many_open: bool,
    move_work_started: f64,
) -> Result<MovesOutcome, String> {
    let config = ctx.config;
    let mut trials = MoveTrials {
        moves: Vec::new(),
        since_improvement: 0,
        polished: false,
        since_fewer_open: 0,
        recently: Vec::new(),
        guided_attribution: None,
    };
    // `global_guidance` with `moves`: the parts by their attribution in
    // the global route of the kept board, recomputed when a move is kept
    // (docs/global-route.md, "Phase 2").
    let guided_moves = kept
        .placer_config
        .global_guidance
        .as_ref()
        .filter(|guidance| guidance.moves)
        .map(|guidance| guidance.threshold);
    let guidance_index = crate::board_placer::reference_index(&kept.problem.references);
    for _ in 0..if many_open { 0 } else { config.moves } {
        if !ctx.stop.allows(LayoutStep::Move) {
            break;
        }
        ctx.arm(&mut routing.router, ledger.clock());
        if routing.best.0 == 0 && trials.since_improvement >= config.patience {
            break;
        }
        let budget = if routing.best.0 == 0 {
            config.move_seconds.min(config.polish_seconds)
        } else {
            config.move_seconds
        }
        .min((0.65 * config.total_seconds - move_work_started).max(0.0));
        if trials.since_improvement >= 2 * config.patience
            || (routing.best.0 > 0 && trials.since_fewer_open >= config.patience / 2)
            || ledger.work - move_work_started > budget
            || routing.router.past_deadline()
        {
            break;
        }
        if let Some(threshold) = guided_moves
            && trials.guided_attribution.is_none()
        {
            let route = pcb_global::route(&kept.board, &pcb_global::GlobalConfig::default());
            trials.guided_attribution = Some(
                crate::board_placer::attribution(
                    &kept.board,
                    &route,
                    &guidance_index,
                    kept.problem.problem.components.len(),
                    threshold,
                )
                .0,
            );
        }
        let Some(index) = next_part(
            &kept.problem,
            movable,
            &mut trials,
            &routing.result,
            routing.best.0,
        ) else {
            break;
        };
        let candidates = candidate_poses(&kept.problem, index, &config.steps_mm, &routing.result);
        if candidates.is_empty() {
            trials.since_improvement += 1;
            continue;
        }

        let mut improved = false;
        for (_, pose) in candidates {
            // A trial is a reroute and a finish (60-200 s on a big board
            // under load): none starts past the deadline.
            if !ctx.stop.allows(LayoutStep::Move) {
                break;
            }
            if try_pose(ctx, ledger, kept, routing, copper, &mut trials, index, pose)? {
                improved = true;
                break;
            }
        }
        if improved {
            trials.since_improvement = 0;
        } else {
            trials.since_improvement += 1;
        }
    }
    Ok(MovesOutcome {
        moves: trials.moves,
        polished: trials.polished,
    })
}

/// The part the next trial moves: the most congested movable footprint
/// that was not tried lately (with the guidance: the most
/// overflow-attributed one first), noted as tried. `None` where no part is
/// left, or the board is complete and no part sits in congestion.
fn next_part(
    problem: &crate::board_placer::LoweredPlacement,
    movable: &[usize],
    trials: &mut MoveTrials,
    result: &core::RoutingResult,
    open: usize,
) -> Option<usize> {
    let mut ranked: Vec<(f64, f64, usize)> = movable
        .iter()
        .filter(|index| !trials.recently.contains(index))
        .map(|index| {
            let component = &problem.problem.components[*index];
            let pose = problem.problem.poses[*index];
            (
                trials
                    .guided_attribution
                    .as_ref()
                    .map_or(0.0, |attribution| attribution[*index]),
                body_congestion(
                    result,
                    component.center(pose),
                    component.half_extent(pose.angle),
                    2.0,
                ),
                *index,
            )
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(b.1.total_cmp(&a.1))
            .then(a.2.cmp(&b.2))
    });
    let &(attributed, congestion, index) = ranked.first()?;
    let congestion = congestion.max(attributed);
    if congestion <= 0.0 && open == 0 {
        return None;
    }
    trials.recently.push(index);
    if trials.recently.len() > movable.len() / 3 + 1 {
        trials.recently.remove(0);
    }
    Some(index)
}

/// Candidate poses of part `index`: steps along the axes (`steps_mm`) and
/// its angles, legal against the placement and breaking no near or
/// relative constraint further, ranked by the congestion they land in
/// (and the step's length); the best three.
fn candidate_poses(
    problem: &crate::board_placer::LoweredPlacement,
    index: usize,
    steps_mm: &[f64],
    result: &core::RoutingResult,
) -> Vec<(f64, placer::Pose)> {
    let component = &problem.problem.components[index];
    let current = problem.problem.poses[index];
    let mut candidates: Vec<(f64, placer::Pose)> = Vec::new();
    for angle in component.angle_options.iter().copied() {
        for step in steps_mm.iter().copied() {
            for (dx, dy) in [(step, 0.0), (-step, 0.0), (0.0, step), (0.0, -step)] {
                let center = component.center(current);
                let wanted = [center[0] + dx, center[1] + dy];
                let position = component.position_for_center(wanted, angle);
                let pose = placer::Pose {
                    position: placer::constraints::snap_position(
                        &problem.problem,
                        index,
                        position,
                        angle,
                    ),
                    angle,
                };
                if pose == current {
                    continue;
                }
                if !placer::legal::is_legal(
                    &problem.problem,
                    &problem.problem.poses,
                    index,
                    pose,
                    0..problem.problem.components.len(),
                ) {
                    continue;
                }
                // A nudge must not break a near or relative constraint
                // further.
                if !problem.problem.constraints.relations.is_empty() {
                    let before = placer::constraints::relation_penalty(
                        &problem.problem,
                        &problem.problem.poses,
                        Some(index),
                    );
                    let mut trial = problem.problem.poses.clone();
                    trial[index] = pose;
                    if placer::constraints::relation_penalty(&problem.problem, &trial, Some(index))
                        > before + 1.0e-9
                    {
                        continue;
                    }
                }
                let landing = body_congestion(
                    result,
                    component.center(pose),
                    component.half_extent(pose.angle),
                    2.0,
                );
                candidates.push((landing + 0.01 * (dx.abs() + dy.abs()), pose));
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    candidates.truncate(3);
    candidates
}

/// One trial: part `index` (and the parts its row carries) moved to
/// `pose`, the board lowered again and the router updated and rerouted in
/// place, its work booked and the trial recorded. Kept when the board
/// improves (the kept state becomes the trial's); else everything is
/// restored. Hands on whether it was kept (false also where the move is
/// not one to make: then nothing is recorded).
#[allow(
    clippy::too_many_arguments,
    reason = "the layout's state is threaded through by reference"
)]
fn try_pose(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    kept: &mut Kept,
    routing: &mut Routing,
    copper: &Copper,
    trials: &mut MoveTrials,
    index: usize,
    pose: placer::Pose,
) -> Result<bool, String> {
    let problem = &mut kept.problem;
    let router = &mut routing.router;
    let trial_started = std::time::Instant::now();
    let saved_router = router.saved_copy();
    let saved_pcb = kept.pcb.clone();
    let saved_pose = problem.problem.poses[index];
    let saved_poses = problem.problem.poses.clone();
    // The part moves, and the parts its row carries with it.
    let mut poses = saved_poses.clone();
    poses[index] = pose;
    problem.problem.sync_followers(&mut poses);
    {
        let mut footprints = footprint_items(&mut kept.pcb)?;
        for (part, (old, new)) in saved_poses.iter().zip(&poses).enumerate() {
            if part == index || old != new {
                write_footprint_pose(footprints[part], *new)?;
            }
        }
    }
    problem.problem.poses = poses;
    let (trial_board, plan_work) = lower_for_route(
        &without_copper_texts(&kept.pcb, ctx.config.placer.keep_board_marks),
        &ctx.router_config,
        copper.connect,
    )?;
    ledger.work += plan_work;
    ctx.arm(router, ledger.clock());
    let expansions_before = router.expansions();
    // (An error ends the layout: nothing to restore.) A move that
    // takes a part to another sub-board splits the nets otherwise
    // (`sub_boards`), and one that changes the pairs' plan (a pair
    // point or a link gained or lost: the links are nets of the
    // route) changes the nets the router holds: not a move to make.
    let rerouted = match router.update(&trial_board) {
        Ok(rerouted) => rerouted,
        Err(error)
            if error == crate::sub_boards::OTHER_NETS
                || (plan_work > 0.0 && error == core::router::UPDATE_NEEDS_SAME_NETS) =>
        {
            if error == core::router::UPDATE_NEEDS_SAME_NETS {
                eprintln!(
                    "layout: a move of {} changes the differential pairs' plan: not tried",
                    problem.references[index]
                );
            }
            router.restore_saved(saved_router);
            kept.pcb = saved_pcb;
            problem.problem.poses = saved_poses.clone();
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let polish = routing.best.0 == 0;
    let trial = router.reroute(polish);
    ledger.work += router.expansions().saturating_sub(expansions_before) as f64
        / core::router::expansions_per_second();
    let trial_score = score(&trial, &trial_board);
    let improved = trial_score < routing.best;
    if improved && trial_score.0 < routing.best.0 {
        trials.since_fewer_open = 0;
    } else {
        trials.since_fewer_open += 1;
    }
    trials.moves.push(KiCadBoardLayoutMove {
        reference: problem.references[index].clone(),
        from: [
            saved_pose.position[0],
            saved_pose.position[1],
            saved_pose.angle,
        ],
        to: [pose.position[0], pose.position[1], pose.angle],
        rerouted_nets: rerouted,
        seconds: trial_started.elapsed().as_secs_f64(),
        kept: improved,
        unconnected_terminals: trial_score.0,
        vias: trial.routes.iter().map(|route| route.vias.len()).sum(),
        length_mm: trial
            .routes
            .iter()
            .flat_map(|route| route.segments.iter())
            .map(|segment| distance_squared(segment.start, segment.end).sqrt())
            .sum(),
    });
    if improved {
        routing.best = trial_score;
        routing.result = trial;
        kept.board = trial_board;
        trials.guided_attribution = None;
        trials.polished = polish;
        return Ok(true);
    }
    router.restore_saved(saved_router);
    kept.pcb = saved_pcb;
    problem.problem.poses = saved_poses.clone();
    Ok(false)
}

// Copyright (C) 2026 Florian Loitsch.

//! A placement retry with the pads of an integral outline as obstacles.

use super::*;

pub(super) fn recover(
    context: &PlaceContext,
    lowered: &mut LoweredPlacement,
    picked: &mut Picked,
    settings: &core::Config,
    layout_work_available: Option<f64>,
) -> Option<KiCadPerimeterContactRecovery> {
    let [(owner, contacts)] = lowered.perimeter_contact_boxes.as_slice() else {
        return None;
    };
    let owner = *owner;
    let part = &lowered.problem.components[owner];
    if picked
        .skeleton_race
        .as_ref()
        .is_some_and(|race| race.budget_ok)
        || !part.fixed
        || part.holes_inside
        || lowered
            .problem
            .constraints
            .followers
            .iter()
            .any(|f| f.leader == owner || f.part == owner)
        || !picked
            .placement
            .unplaced
            .iter()
            .chain(&picked.placement.illegal)
            .any(|&index| {
                let stranded = &lowered.problem.components[index];
                !stranded.fixed && stranded.side.collides(part.side)
            })
    {
        return None;
    }
    let modeled_seconds = |work| work as f64 / core::work_per_second();
    let used = picked.work_seconds.max(modeled_seconds(picked.work.pool));
    let remaining = layout_work_available
        .or(context.config.work_seconds)
        .map_or(35.0, |limit| limit - used);
    let budget = remaining.min(35.0) * 0.8;
    let expired = || {
        context
            .config
            .deadline
            .is_some_and(|deadline| deadline <= std::time::Instant::now())
    };
    if !remaining.is_finite() || budget < 8.0 || expired() {
        return None;
    }
    let misplaced = |placement: &core::Placement| {
        placement
            .unplaced
            .iter()
            .chain(&placement.illegal)
            .collect::<BTreeSet<_>>()
            .len()
    };
    let before = misplaced(&picked.placement);
    let reference = lowered.references[owner].clone();
    eprintln!(
        "placer: retrying {reference} using the perimeter's physical contacts ({budget:.1} modeled seconds for search)"
    );
    let mut problem = lowered.problem.clone();
    problem.components[owner].hollow = contacts.clone();
    problem.components[owner].holes_inside = true;
    let mut settings = settings.clone();
    settings.work_limit = Some((budget * core::work_per_second()) as u64);
    let offset = picked.seed_kept.wrapping_sub(settings.global.seed);
    settings.global.seed = picked.seed_kept;
    settings.anneal.seed = settings.anneal.seed.wrapping_add(offset);
    settings.start = Some(std::sync::Arc::new(core::Start {
        poses: picked.placement.poses.clone(),
        parts: vec![true; problem.components.len()],
        stage: core::StartStage::Legalize,
    }));
    let trial = core::place(&problem, &settings);
    let work = trial
        .work
        .saturating_add(trial.exits.as_ref().map_or(0, |exit| exit.work));
    let work_seconds = modeled_seconds(work);
    let after = misplaced(&trial);
    let preserved = !expired()
        && work_seconds <= remaining
        && after == 0
        && constraint_report(&problem, &trial.poses, &lowered.references)
            .into_iter()
            .chain(pair_row_statuses(
                &lowered.pair_rows,
                &problem,
                &trial.poses,
                &lowered.references,
            ))
            .all(|status| status.requirement_satisfied())
        && problem
            .components
            .iter()
            .enumerate()
            .all(|(index, component)| {
                !component.fixed || problem.poses[index] == trial.poses[index]
            });
    if preserved {
        lowered.problem = problem;
        picked.placement = trial;
        picked.alternatives.clear();
        picked.alternative_seed_metrics.clear();
        picked.alternative_exits.clear();
        picked.pad_objective_composition = None;
        picked.seeds_placed += 1;
    }
    picked.work_seconds += work_seconds;
    picked.work.pool += work;
    picked.work.seeds += work;
    picked.work.run += 1;
    Some(KiCadPerimeterContactRecovery {
        reference,
        status: if preserved { "accepted" } else { "rejected" },
        misplaced_before: before,
        misplaced_after: after,
        work_seconds,
    })
}

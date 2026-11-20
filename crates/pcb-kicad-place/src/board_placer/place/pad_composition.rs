// Copyright (C) 2026 Florian Loitsch.

//! Bounded search for a compact placement with the achieved pad proximity.

use super::*;
use pcb_placer::constraints::{Anchor, Relation, Status, keeps_pad_near_requirements};

const DISTANCE_SLACK_MM: f64 = 0.001;
const MAX_PARENTS: usize = 6;
const MAX_DONORS: usize = 4;
const MAX_OBJECTIVES: usize = 12;

#[derive(Default)]
pub(super) struct Search {
    pub best: Option<(core::Placement, KiCadPadObjectiveComposition)>,
    pub work: u64,
    pub drafts: u64,
}

#[derive(Clone, Copy)]
struct Objective {
    source: Anchor,
    anchor: Anchor,
    distance: f64,
}

/// Try combinations from the kept seeds and two compact drafts. Draft
/// limits steer the solver only. All candidates are evaluated again under
/// the original brief and must keep each achieved pad distance within a
/// micron while improving the modeled wirelength.
pub(super) fn explore(
    context: &PlaceContext,
    lowered: &LoweredPlacement,
    config: &core::Config,
    seeds: &[(u64, core::Placement)],
    used: u64,
) -> Search {
    let mut search = Search::default();
    if seeds.len() < 2 || config.pad_exits.is_some() || config.start.is_some() {
        return search;
    }
    let problem = &lowered.problem;
    let incumbent = &seeds[0].1;
    if !legal(incumbent) {
        return search;
    }
    let objectives: Vec<_> = problem
        .constraints
        .relations
        .iter()
        .filter_map(|relation| match relation {
            Relation::PadNear {
                source,
                anchor,
                max: None,
                ..
            } => Some((*source, *anchor)),
            _ => None,
        })
        .zip(objective_distances(&incumbent.constraints))
        .map(|((source, anchor), distance)| Objective {
            source,
            anchor,
            distance,
        })
        .collect();
    if objectives.is_empty() || objectives.len() > MAX_OBJECTIVES {
        return search;
    }
    let remaining = config
        .work_limit
        .map_or(u64::MAX, |limit| limit.saturating_sub(used));
    let per_draft = remaining.min((8.0 * core::work_per_second()) as u64) / 2;
    let deadline = context.config.deadline;
    let on_time = || deadline.is_none_or(|deadline| std::time::Instant::now() < deadline);
    let minimum_work = (0.2 * core::work_per_second()) as u64;
    if remaining < minimum_work || !on_time() {
        return search;
    }

    let mut parents: Vec<_> = seeds[1..]
        .iter()
        .filter(|(_, parent)| legal(parent))
        .collect();
    parents.sort_by(|a, b| a.1.wirelength_final.total_cmp(&b.1.wirelength_final));
    let check_start = core::work();
    for (seed, parent) in parents
        .into_iter()
        .take(MAX_PARENTS)
        .take_while(|_| on_time() && core::work().saturating_sub(check_start) < remaining)
    {
        consider(
            &mut search,
            lowered,
            incumbent,
            &objectives,
            seeds,
            *seed,
            "objective_seed",
            parent,
        );
    }
    search.work += core::work().saturating_sub(check_start);

    // One tight draft and one compact draft with a shared ceiling. A
    // quarter grid step above the longest achieved objective lets local
    // packing differ while limiting the search's proximity penalty.
    let shared = objectives
        .iter()
        .map(|pair| pair.distance)
        .fold(0.0, f64::max)
        + (incumbent.relaxation.grid / 4.0).max(DISTANCE_SLACK_MM);
    for (kind, shared_limit) in [("tight_draft", None), ("compact_draft", Some(shared))] {
        let budget = per_draft.min(remaining.saturating_sub(search.work));
        if budget < minimum_work || !on_time() {
            break;
        }
        let mut problem = problem.clone();
        let mut limits = objectives.iter();
        for relation in &mut problem.constraints.relations {
            if let Relation::PadNear { max, .. } = relation
                && max.is_none()
                && let Some(pair) = limits.next()
            {
                *max = Some(shared_limit.unwrap_or(pair.distance + DISTANCE_SLACK_MM));
            }
        }
        let mut config = config.clone();
        config.work_limit = Some(budget);
        config.refine_pad_near |= shared_limit.is_none();
        let seed = config.global.seed;
        let start = core::work();
        let parent = core::place(&problem, &config);
        consider(
            &mut search,
            lowered,
            incumbent,
            &objectives,
            seeds,
            seed,
            kind,
            &parent,
        );
        search.work += core::work().saturating_sub(start);
        search.drafts += 1;
    }
    search
}

fn legal(placement: &core::Placement) -> bool {
    placement.unplaced.is_empty() && placement.illegal.is_empty()
}

fn objective_distances(statuses: &[Status]) -> impl Iterator<Item = f64> + '_ {
    statuses
        .iter()
        .filter(|status| status.kind == "pad_near_objective")
        .filter_map(|status| status.distance)
}

fn ordinary_requirements_kept(before: &[Status], after: &[Status]) -> bool {
    before
        .iter()
        .zip(after)
        .all(|(old, new)| old.pad_near.is_some() || new.violation <= old.violation + 1.0e-9)
}

#[allow(
    clippy::too_many_arguments,
    reason = "compare a compact parent against the same incumbent and donor pool"
)]
fn consider(
    search: &mut Search,
    lowered: &LoweredPlacement,
    incumbent: &core::Placement,
    objectives: &[Objective],
    donors: &[(u64, core::Placement)],
    parent_seed: u64,
    parent_search: &'static str,
    parent: &core::Placement,
) {
    if !legal(parent)
        || (parent.relaxation.courtyard_spacing && !incumbent.relaxation.courtyard_spacing)
        || parent.relaxation.on_marks.len() > incumbent.relaxation.on_marks.len()
    {
        return;
    }
    let problem = &lowered.problem;
    let mut relaxed = problem.clone();
    let mut relaxation = parent.relaxation.clone();
    // Use the planning-halo allowances already reached by these seeds.
    // Physical bodies, spacing, outline, and keepouts stay at the parent's level.
    relaxation.halo_scale = relaxation.halo_scale.min(incumbent.relaxation.halo_scale);
    relaxation.small_halo_scale = relaxation
        .small_halo_scale
        .min(incumbent.relaxation.small_halo_scale);
    relaxation.apply(&mut relaxed);
    let mut poses = parent.poses.clone();
    let mut statuses = core::constraints::report(problem, &poses);
    let mut transfers = Vec::new();
    for (index, pair) in objectives.iter().enumerate() {
        let before: Vec<_> = objective_distances(&statuses).collect();
        let source = pair.source.part();
        let anchor = pair.anchor.part();
        if !statuses
            .iter()
            .filter(|status| status.kind == "pad_near_objective")
            .nth(index)
            .and_then(|status| status.pad_near.as_ref())
            .is_some_and(|near| near.refinable)
        {
            continue;
        }
        for (donor_seed, donor) in donors
            .iter()
            .filter(|(_, donor)| legal(donor))
            .take(MAX_DONORS)
        {
            let pose = transfer_pose(donor.poses[source], donor.poses[anchor], poses[anchor]);
            if pose == poses[source]
                || !relaxed.components[source]
                    .angle_options
                    .iter()
                    .any(|allowed| {
                        let difference = (pose.angle - allowed).rem_euclid(360.0);
                        difference.min(360.0 - difference) <= 1.0e-6
                    })
                || !core::legal::is_legal(&relaxed, &poses, source, pose, 0..poses.len())
            {
                continue;
            }
            let mut trial = poses.clone();
            trial[source] = pose;
            let measured = core::constraints::report(problem, &trial);
            let after: Vec<_> = objective_distances(&measured).collect();
            if after[index] < before[index] - 1.0e-9
                && after
                    .iter()
                    .zip(&before)
                    .all(|(new, old)| *new <= old + 1.0e-9)
                && ordinary_requirements_kept(&statuses, &measured)
                && keeps_pad_near_requirements(problem, &poses, &trial)
            {
                poses = trial;
                statuses = measured;
                transfers.push(KiCadPadObjectiveTransfer {
                    part: lowered.references[pair.source.reported_part()].clone(),
                    anchor: lowered.references[pair.anchor.reported_part()].clone(),
                    donor_seed: *donor_seed,
                });
                break;
            }
        }
    }
    let wirelength = problem.wirelength(&poses);
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!(
            "placer: pad objective {parent_search} seed {parent_seed}: wirelength {wirelength:.3}, distances {:?}, {} transfers, target {:?}",
            objective_distances(&statuses).collect::<Vec<_>>(),
            transfers.len(),
            objectives
                .iter()
                .map(|pair| pair.distance)
                .collect::<Vec<_>>(),
        );
    }
    if wirelength >= incumbent.wirelength_final - 1.0e-6
        || search
            .best
            .as_ref()
            .is_some_and(|(best, _)| wirelength >= best.wirelength_final - 1.0e-6)
        || objective_distances(&statuses)
            .zip(objectives)
            .any(|(distance, pair)| distance > pair.distance + DISTANCE_SLACK_MM)
        || !ordinary_requirements_kept(&incumbent.constraints, &statuses)
        || !keeps_pad_near_requirements(problem, &incumbent.poses, &poses)
        || !core::legal::illegal_components(&relaxed, &poses).is_empty()
    {
        return;
    }
    let mut candidate = parent.clone();
    candidate.relaxation = relaxation;
    candidate.wirelength_final = wirelength;
    candidate.constraints = statuses;
    candidate.poses = poses;
    candidate.frames.push(core::Frame {
        iteration: candidate
            .frames
            .last()
            .map_or(0, |frame| frame.iteration + 1),
        overflow: 0.0,
        wirelength,
        poses: candidate.poses.clone(),
    });
    candidate
        .stage_poses
        .push(("pad_objective_composition", candidate.poses.clone()));
    search.best = Some((
        candidate,
        KiCadPadObjectiveComposition {
            parent_seed,
            parent_search,
            transfers,
        },
    ));
}

fn transfer_pose(
    source: core::Pose,
    donor_anchor: core::Pose,
    parent_anchor: core::Pose,
) -> core::Pose {
    let turn = parent_anchor.angle - donor_anchor.angle;
    let local = [
        source.position[0] - donor_anchor.position[0],
        source.position[1] - donor_anchor.position[1],
    ];
    let rotated = core::problem::rotate(local, -turn);
    core::Pose {
        position: [
            parent_anchor.position[0] + rotated[0],
            parent_anchor.position[1] + rotated[1],
        ],
        angle: (source.angle + turn).rem_euclid(360.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_keeps_source_pose_in_the_anchors_rotated_frame() {
        let pose = |position, angle| core::Pose { position, angle };
        let source = pose([13.0, 11.0], 180.0);
        let donor_anchor = pose([10.0, 10.0], 0.0);
        let parent_anchor = pose([20.0, 30.0], 90.0);
        let transferred = transfer_pose(source, donor_anchor, parent_anchor);
        assert_eq!(transferred, pose([21.0, 27.0], 270.0));
        assert_eq!(
            transfer_pose(transferred, parent_anchor, donor_anchor),
            source
        );
    }
}

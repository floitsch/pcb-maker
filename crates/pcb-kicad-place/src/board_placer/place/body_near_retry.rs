// Copyright (C) 2026 Florian Loitsch.

//! Search one authored body-to-pin limit while retaining the other poses.

use super::*;
use pcb_placer::constraints::{self, Anchor, Relation};
use serde::Serialize;
use serde_json::Value;

const GRID_MM: f64 = 0.1;
const MAX_DISTANCE_MM: f64 = 3.0;
const MAX_BODY_MM: f64 = 6.0;

#[derive(Serialize)]
pub struct BodyNearReseat {
    pub part: String,
    pub pin_of: String,
    pub before_at: [f64; 3],
    pub at: [f64; 3],
    pub before_mm: f64,
    pub distance_mm: f64,
    pub max_mm: f64,
    pub local_pitch_mm: f64,
    pub relaxed_halos: [String; 2],
}

/// A trial pose for a whole-board placement at ordinary body spacing.
/// The caller supplies an allowance and decides whether to run the trial.
pub fn propose_body_near_reseat(
    project: &Path,
    board_id: &str,
    config: &KiCadBoardPlacerConfig,
    incumbent: &Path,
    report: &Value,
    work_limit: u64,
) -> (Result<BodyNearReseat, &'static str>, u64) {
    let started = core::work();
    let mut considered_work = 0;
    let mut probe = || -> Option<BodyNearReseat> {
        if ["unplaced", "illegal"]
            .into_iter()
            .any(|key| !report[key].as_array().is_some_and(Vec::is_empty))
            || ["modules", "sub_boards", "panel_copies", "halos"]
                .into_iter()
                .any(|key| !report[key].is_null())
            || [
                "tight_bodies",
                "courtyard_spacing",
                "edge_inset",
                "edge_copper",
                "pad_clearance",
            ]
            .into_iter()
            .any(|key| report[key] == true)
            || !report["parts_on_marks"].is_null()
                && !report["parts_on_marks"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
            || !config.allow_back.is_empty()
            || !config.tuning.halo_overrides_mm.is_empty()
        {
            return None;
        }
        let incumbent = parse(&fs::read_to_string(incumbent).ok()?).ok()?;
        let lowered = prepare_board(project, board_id, config, Some(&incumbent))
            .ok()?
            .lowered;
        let mut problem = lowered.problem;
        let poses = problem.poses.clone();
        let mut missed = lowered.body_near_pin_of.iter().filter(|(index, _)| {
            matches!(problem.constraints.relations[**index], Relation::Near { part, anchor, max }
                if !constraints::near_limit_kept(
                    constraints::body_near_distance(&problem, &poses, part, anchor), max))
        });
        let (index, pin) = missed.next()?;
        if missed.next().is_some() {
            return None;
        }
        let relation = problem.constraints.relations[*index].clone();
        let Relation::Near {
            part,
            anchor: Anchor::Point(target, offset),
            max,
        } = relation
        else {
            return None;
        };
        let before = poses[part];
        let source = &problem.components[part];
        let peer = &problem.components[target];
        if source.fixed
            || source.has_holes()
            || source.side != peer.side
            || source.round
            || source.body_size.into_iter().any(|size| size > MAX_BODY_MM)
            || source.angle_options.iter().any(|a| a % 90.0 != 0.0)
            || !(0.0 < max && max <= MAX_DISTANCE_MM)
            || problem
                .constraints
                .followers
                .iter()
                .any(|f| [part, target].contains(&f.part) || [part, target].contains(&f.leader))
        {
            return None;
        }
        let required = &lowered.references[part];
        if lowered
            .references
            .iter()
            .filter(|name| *name == required)
            .count()
            != 1
        {
            return None;
        }
        let anchor = Anchor::Point(target, offset);
        let before_mm = constraints::body_near_distance(&problem, &poses, part, anchor);
        let grid = GRID_MM.min(report["grid_mm"].as_f64()?);
        if grid < 0.05 {
            return None;
        }
        core::Relaxation {
            spacing: report["spacing_mm"].as_f64()?,
            grid,
            halo_scale: report["halo_scale"].as_f64()?,
            small_halo_scale: report["small_halo_scale"].as_f64()?,
            edge_inset: false,
            tight: false,
            edge_copper: false,
            edge_rule: report["edge_rule_mm"].as_f64()?,
            courtyard_spacing: false,
            pad_clearance: false,
            halos: None,
            on_marks: Vec::new(),
        }
        .apply(&mut problem);
        problem.components[part].halo = 0.0;
        problem.components[target].halo = 0.0;
        let point = constraints::relation_target(&problem, &poses, &relation);
        let mut best = None::<(f64, f64, core::Pose)>;
        let mut trial = poses.clone();
        let authored = problem
            .constraints
            .relations
            .len()
            .saturating_sub(problem.constraints.automatic);
        let additional = problem.constraints.relations[..authored]
            .iter()
            .enumerate()
            .filter(|(i, r)| *i != *index && r.involves(part))
            .map(|(_, r)| (r, constraints::relation_violation(&problem, &poses, r)))
            .collect::<Vec<_>>();
        let component = &problem.components[part];
        for &angle in &component.angle_options {
            let center = component.position_for_center(point, angle);
            let half = component.half_extent(angle);
            let window = |axis: usize| {
                let reach = max + half[axis];
                let lo = ((center[axis] - reach) / grid).ceil() as i64;
                let hi = ((center[axis] + reach) / grid).floor() as i64;
                lo..=hi
            };
            for x in window(0) {
                for y in window(1) {
                    if core::work().saturating_sub(started) + considered_work >= work_limit {
                        return None;
                    }
                    considered_work += 8;
                    let pose = core::Pose {
                        position: [x as f64 * grid, y as f64 * grid],
                        angle,
                    };
                    let motion = (pose.position[0] - before.position[0])
                        .hypot(pose.position[1] - before.position[1]);
                    if best.is_some_and(|(dist, _, _)| motion >= dist) {
                        continue;
                    }
                    trial[part] = pose;
                    let distance = constraints::body_near_distance(&problem, &trial, part, anchor);
                    if distance > max - 1.0e-6
                        || !core::legal::is_legal(&problem, &poses, part, pose, 0..poses.len())
                    {
                        continue;
                    }
                    considered_work += constraints::relation_work(&problem, part);
                    if additional.iter().all(|(r, before)| {
                        constraints::relation_violation(&problem, &trial, r) <= before + 1.0e-6
                    }) {
                        best = Some((motion, distance, pose));
                    }
                }
            }
        }
        let (_, distance_mm, pose) = best?;
        let at = |pose: core::Pose| [pose.position[0], pose.position[1], pose.angle];
        Some(BodyNearReseat {
            part: required.clone(),
            pin_of: pin.clone(),
            before_at: at(before),
            at: at(pose),
            before_mm,
            distance_mm,
            max_mm: max,
            local_pitch_mm: grid,
            relaxed_halos: [required.clone(), lowered.references[target].clone()],
        })
    };
    let proposal = probe();
    let work = core::work().saturating_sub(started) + considered_work;
    let result = if work >= work_limit {
        Err("insufficient_body_near_probe_budget")
    } else {
        proposal.ok_or(if considered_work == 0 {
            "unsupported_body_near_probe_geometry"
        } else {
            "no_legal_fixed_neighbour_body_near_pose"
        })
    };
    (result, work)
}

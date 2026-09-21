// Copyright (C) 2026 Toit contributors.

//! Bounded coordinate descent over rigid quarter-turns at fixed centers.

use crate::{PlacementPose, validate_serialized_placement_poses};
use layout_trace_model::model::{PinRef, Problem, Rotation, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct OrientationRefinementConfig {
    pub maximum_sweeps: usize,
    pub maximum_trials: usize,
    /// Absolute minimum reduction in the weighted squared-distance objective.
    pub minimum_improvement: f64,
}
impl Default for OrientationRefinementConfig {
    fn default() -> Self {
        Self {
            maximum_sweeps: 4,
            maximum_trials: 512,
            minimum_improvement: 1.0e-9,
        }
    }
}
impl OrientationRefinementConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.maximum_sweeps == 0 || self.maximum_trials == 0 {
            return Err("orientation refinement budgets must be positive".into());
        }
        if !self.minimum_improvement.is_finite() || self.minimum_improvement < 0.0 {
            return Err("orientation minimum_improvement must be finite and non-negative".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OrientationNetCost {
    pub connection: String,
    pub representation: &'static str,
    /// Mean of squared minimum pad-center distances over terminal pairs.
    pub mean_squared_distance_mm2: f64,
    pub weighted_cost: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrientationTrialStatus {
    NoImprovement,
    Rejected,
    FeasibleNotSelected,
    Accepted,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OrientationTrial {
    pub sweep: usize,
    pub component: String,
    pub from_degrees: f64,
    pub to_degrees: f64,
    pub cost_before: f64,
    pub cost_after: f64,
    pub status: OrientationTrialStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrientationTermination {
    NoImprovingLegalQuarterTurn,
    TrialBudget,
    SweepBudget,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OrientationRefinementEvidence {
    pub objective: &'static str,
    pub cost_before: f64,
    pub cost_after: f64,
    pub sweeps: usize,
    pub accepted_changes: usize,
    pub legality_checks: usize,
    pub termination: OrientationTermination,
    pub trials: Vec<OrientationTrial>,
    pub net_costs_before: Vec<OrientationNetCost>,
    pub net_costs_after: Vec<OrientationNetCost>,
    pub centers_preserved: bool,
    pub routability_claimed: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OrientationRefinementResult {
    pub poses: Vec<PlacementPose>,
    pub evidence: OrientationRefinementEvidence,
}

fn net_costs(
    problem: &Problem,
    poses: &[PlacementPose],
) -> Result<Vec<OrientationNetCost>, String> {
    let poses = poses
        .iter()
        .map(|p| (p.component.as_str(), p))
        .collect::<BTreeMap<_, _>>();
    let components = problem
        .components
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect::<BTreeMap<_, _>>();
    let points = |terminal: &PinRef, primary: &str, allowed: &[String]| {
        let c = components[terminal.component.as_str()];
        let p = poses[terminal.component.as_str()];
        let pin = c
            .pins
            .iter()
            .find(|p| p.id == terminal.pin)
            .expect("schema-checked pin");
        let mut offsets = pin
            .pads
            .iter()
            .filter(|pad| pad.layer == primary || allowed.contains(&pad.layer))
            .map(|pad| pin.pad_local_center(pad))
            .collect::<Vec<_>>();
        if offsets.is_empty() {
            offsets.push(pin.offset);
        }
        let (sin, cos) = p.rotation_degrees.to_radians().sin_cos();
        offsets
            .into_iter()
            .map(|v| {
                Vec2::new(
                    p.position.x + cos * v.x - sin * v.y,
                    p.position.y + sin * v.x + cos * v.y,
                )
            })
            .collect::<Vec<_>>()
    };
    let pair_cost = |a: &[Vec2], b: &[Vec2]| {
        a.iter()
            .flat_map(|a| {
                b.iter()
                    .map(move |b| (a.x - b.x).powi(2) + (a.y - b.y).powi(2))
            })
            .fold(f64::INFINITY, f64::min)
    };
    let mut result = Vec::new();
    for net in &problem.nets {
        let distance = pair_cost(
            &points(&net.from, &net.layer, &net.allowed_layers),
            &points(&net.to, &net.layer, &net.allowed_layers),
        );
        result.push(OrientationNetCost {
            connection: net.id.clone(),
            representation: "legacy_branch",
            mean_squared_distance_mm2: distance,
            weighted_cost: distance * net.width * net.tension_weight,
        });
    }
    for net in &problem.electrical_nets {
        let mut terminals = net.terminals.iter().collect::<Vec<_>>();
        terminals.sort_by(|a, b| (&a.component, &a.pin).cmp(&(&b.component, &b.pin)));
        let locations = terminals
            .iter()
            .map(|t| points(t, &net.layer, &net.allowed_layers))
            .collect::<Vec<_>>();
        let mut sum = 0.0;
        let mut pairs = 0;
        for (i, first) in locations.iter().enumerate() {
            for second in &locations[i + 1..] {
                sum += pair_cost(first, second);
                pairs += 1;
            }
        }
        let distance = if pairs == 0 { 0.0 } else { sum / pairs as f64 };
        result.push(OrientationNetCost {
            connection: net.id.clone(),
            representation: "electrical_net",
            mean_squared_distance_mm2: distance,
            weighted_cost: distance * net.width * net.tension_weight,
        });
    }
    result
        .sort_by(|a, b| (a.representation, &a.connection).cmp(&(b.representation, &b.connection)));
    if result
        .iter()
        .any(|v| !v.mean_squared_distance_mm2.is_finite() || !v.weighted_cost.is_finite())
    {
        return Err("orientation attraction cost is not finite".into());
    }
    Ok(result)
}
fn total(costs: &[OrientationNetCost]) -> Result<f64, String> {
    let cost: f64 = costs.iter().map(|c| c.weighted_cost).sum();
    if !cost.is_finite() {
        return Err("orientation total attraction cost is not finite".into());
    }
    Ok(cost)
}

/// Refine a legal placement without projecting any trial. Candidates are
/// independent quarter-turns from each component's current orientation. The
/// best legal improvement for that component is committed before the next
/// component is visited. All subsequent scores use the updated placement.
/// Multiple pad centers are endpoint alternatives, not separately movable pads.
/// Their minimum separation ignores routing obstacles and layer-change costs.
pub fn refine_placement_orientations(
    problem: &Problem,
    input: &[PlacementPose],
    config: &OrientationRefinementConfig,
) -> Result<OrientationRefinementResult, String> {
    config.check()?;
    validate_serialized_placement_poses(problem, input)?;
    let mut poses = input.to_vec();
    poses.sort_by(|a, b| a.component.cmp(&b.component));
    crate::placement::validate_placement_exclusions(problem, &poses)?;
    let components = problem
        .components
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect::<BTreeMap<_, _>>();
    let before = net_costs(problem, &poses)?;
    let initial = total(&before)?;
    let mut current = initial;
    let mut trials = Vec::new();
    let mut accepted = 0;
    let mut legality_checks = 0;
    let mut sweeps = 0;
    let mut termination = OrientationTermination::SweepBudget;
    'sweeps: for sweep in 0..config.maximum_sweeps {
        sweeps += 1;
        let accepted_before = accepted;
        for index in 0..poses.len() {
            if components[poses[index].component.as_str()]
                .constraints
                .rotation
                == Rotation::Fixed
            {
                continue;
            }
            let original = poses[index].rotation_degrees;
            let mut best: Option<(usize, f64, f64)> = None;
            let mut budget_hit = false;
            for turn in [90.0, 180.0, 270.0] {
                if trials.len() == config.maximum_trials {
                    budget_hit = true;
                    break;
                }
                let rotation = (original + turn).rem_euclid(360.0);
                poses[index].rotation_degrees = rotation;
                let score = total(&net_costs(problem, &poses)?)?;
                let mut status = OrientationTrialStatus::NoImprovement;
                let mut error = None;
                // A relative numerical floor prevents tiny round-off changes
                // from cycling even when the requested absolute floor is zero.
                let epsilon = config.minimum_improvement.max(current.abs() * 1.0e-12);
                if current - score > epsilon {
                    legality_checks += 1;
                    match validate_serialized_placement_poses(problem, &poses).and_then(|()| {
                        crate::placement::validate_placement_exclusions(problem, &poses)
                    }) {
                        Ok(()) => {
                            status = OrientationTrialStatus::FeasibleNotSelected;
                            if best.is_none_or(|(_, cost, _)| score < cost) {
                                best = Some((trials.len(), score, rotation));
                            }
                        }
                        Err(reason) => {
                            status = OrientationTrialStatus::Rejected;
                            error = Some(reason);
                        }
                    }
                }
                trials.push(OrientationTrial {
                    sweep,
                    component: poses[index].component.clone(),
                    from_degrees: original,
                    to_degrees: rotation,
                    cost_before: current,
                    cost_after: score,
                    status,
                    error,
                });
            }
            poses[index].rotation_degrees = original;
            if let Some((trial, score, rotation)) = best {
                poses[index].rotation_degrees = rotation;
                current = score;
                accepted += 1;
                trials[trial].status = OrientationTrialStatus::Accepted;
            }
            if budget_hit {
                termination = OrientationTermination::TrialBudget;
                break 'sweeps;
            }
        }
        if accepted_before == accepted {
            termination = OrientationTermination::NoImprovingLegalQuarterTurn;
            break;
        }
    }
    validate_serialized_placement_poses(problem, &poses)?;
    crate::placement::validate_placement_exclusions(problem, &poses)?;
    let after = net_costs(problem, &poses)?;
    Ok(OrientationRefinementResult {
        poses,
        evidence: OrientationRefinementEvidence {
            objective: "sum of width * tension_weight * mean pairwise squared minimum pad-center distance; no copper routing claim",
            cost_before: initial,
            cost_after: total(&after)?,
            sweeps,
            accepted_changes: accepted,
            legality_checks,
            termination,
            trials,
            net_costs_before: before,
            net_costs_after: after,
            centers_preserved: true,
            routability_claimed: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InitialPlacementConfig, run_initial_placement, run_initial_placement_archive};
    fn fixture() -> Problem {
        serde_json::from_str(include_str!(
            "../../../benchmarks/small/orientation-region-control.json"
        ))
        .unwrap()
    }
    fn poses(problem: &Problem) -> Vec<PlacementPose> {
        problem
            .components
            .iter()
            .map(|c| PlacementPose {
                component: c.id.clone(),
                position: c.position,
                rotation_degrees: c.rotation_degrees,
            })
            .collect()
    }
    fn part(result: &OrientationRefinementResult) -> &PlacementPose {
        result.poses.iter().find(|p| p.component == "PART").unwrap()
    }

    #[test]
    fn planted_rotation_is_recovered_without_moving_centers_and_invalid_turns_are_retained() {
        let p = fixture();
        let input = poses(&p);
        let r = refine_placement_orientations(&p, &input, &Default::default()).unwrap();
        assert_eq!(part(&r).rotation_degrees, 0.0);
        assert!((r.evidence.cost_before - 45.125).abs() < 1.0e-9);
        assert!((r.evidence.cost_after - 21.125).abs() < 1.0e-9);
        assert_eq!(r.evidence.accepted_changes, 1);
        assert_eq!(
            r.evidence.termination,
            OrientationTermination::NoImprovingLegalQuarterTurn
        );
        assert_eq!(
            r.evidence
                .trials
                .iter()
                .filter(|t| t.status == OrientationTrialStatus::Rejected)
                .count(),
            2
        );
        for pose in &r.poses {
            assert_eq!(
                pose.position,
                input
                    .iter()
                    .find(|p| p.component == pose.component)
                    .unwrap()
                    .position
            );
        }
        assert!(
            r.evidence
                .trials
                .iter()
                .filter(|t| t.status == OrientationTrialStatus::Rejected)
                .all(|t| t.error.as_ref().unwrap().contains("bounds"))
        );
    }

    #[test]
    fn zero_attraction_preserves_pose_and_net_distance_evidence() {
        let mut p = fixture();
        for n in &mut p.nets {
            n.tension_weight = 0.0;
        }
        let r = refine_placement_orientations(&p, &poses(&p), &Default::default()).unwrap();
        assert_eq!(part(&r).rotation_degrees, 180.0);
        assert_eq!(r.evidence.accepted_changes, 0);
        assert_eq!(r.evidence.cost_after, 0.0);
        assert!(
            r.evidence
                .net_costs_after
                .iter()
                .all(|n| n.mean_squared_distance_mm2 > 0.0)
        );
    }

    #[test]
    fn budgets_return_a_legal_partial_result_without_claiming_convergence() {
        let p = fixture();
        for (trials, expected_angle) in [(1, 180.0), (2, 0.0)] {
            let config = OrientationRefinementConfig {
                maximum_trials: trials,
                ..Default::default()
            };
            let r = refine_placement_orientations(&p, &poses(&p), &config).unwrap();
            assert_eq!(r.evidence.trials.len(), trials);
            assert_eq!(part(&r).rotation_degrees, expected_angle);
            assert_eq!(r.evidence.termination, OrientationTermination::TrialBudget);
            validate_serialized_placement_poses(&p, &r.poses).unwrap();
        }
        let config = OrientationRefinementConfig {
            maximum_sweeps: 1,
            ..Default::default()
        };
        let r = refine_placement_orientations(&p, &poses(&p), &config).unwrap();
        assert_eq!(r.evidence.termination, OrientationTermination::SweepBudget);
    }

    #[test]
    fn explicit_nets_use_physical_pad_alternatives_instead_of_pin_origins() {
        let mut p = fixture();
        p.electrical_nets=serde_json::from_value(serde_json::json!([
            {"id":"LEFT","width":0.25,"terminals":[{"component":"L","pin":"1"},{"component":"PART","pin":"1"}]},
            {"id":"RIGHT","width":0.25,"terminals":[{"component":"PART","pin":"2"},{"component":"R","pin":"1"}]}
        ])).unwrap();
        p.nets.clear();
        let c = p.components.iter_mut().find(|c| c.id == "PART").unwrap();
        for pin in &mut c.pins {
            for pad in &mut pin.pads {
                pad.local_center = Some(pin.offset);
            }
            pin.offset = Vec2::new(99.0, 99.0);
        }
        let mut alternative = c.pins[0].pads[0].clone();
        alternative.id = "alternative".into();
        alternative.local_center = Some(Vec2::new(1.0, 0.0));
        c.pins[0].pads.push(alternative);
        let r = refine_placement_orientations(&p, &poses(&p), &Default::default()).unwrap();
        assert_eq!(part(&r).rotation_degrees, 0.0);
        assert!((r.evidence.cost_before - 34.8125).abs() < 1.0e-9);
        assert!((r.evidence.cost_after - 21.125).abs() < 1.0e-9);
    }

    #[test]
    fn multi_terminal_scoring_and_refinement_are_permutation_stable() {
        let mut p = fixture();
        p.components
            .iter_mut()
            .find(|c| c.id == "R")
            .unwrap()
            .position
            .x = 16.0;
        p.nets.clear();
        p.electrical_nets=serde_json::from_value(serde_json::json!([{"id":"THREE","width":0.25,"terminals":[
            {"component":"L","pin":"1"},{"component":"R","pin":"1"},{"component":"PART","pin":"1"}
        ]}])).unwrap();
        let r = refine_placement_orientations(&p, &poses(&p), &Default::default()).unwrap();
        assert_eq!(part(&r).rotation_degrees, 0.0);
        assert!((r.evidence.cost_before - 306.5 / 3.0 * 0.25).abs() < 1.0e-9);
        assert!((r.evidence.cost_after - 294.5 / 3.0 * 0.25).abs() < 1.0e-9);
        p.components.reverse();
        p.electrical_nets[0].terminals.reverse();
        assert_eq!(
            r,
            refine_placement_orientations(&p, &poses(&p), &Default::default()).unwrap()
        );
    }

    #[test]
    fn refinement_is_opt_in_and_runs_before_archive_admission() {
        let p = fixture();
        let mut config = InitialPlacementConfig::default();
        let base = run_initial_placement(&p, &config).unwrap();
        assert!(base.evidence.orientation_refinement.is_none());
        assert!(
            !serde_json::to_value(&config)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("orientation_refinement")
        );
        config.orientation_refinement = Some(Default::default());
        let result = run_initial_placement(&p, &config).unwrap();
        assert_eq!(result.evidence.rotated_components, 1);
        assert_eq!(result.evidence.translated_components, 0);
        assert!(result.evidence.orientation_refinement.is_some());
        let archive = run_initial_placement_archive(&p, &config, 1).unwrap();
        assert_eq!(
            archive.attempts[0].placement.as_ref().unwrap().poses,
            result.poses
        );
    }

    #[test]
    fn invalid_input_and_zero_budgets_fail_closed() {
        let p = fixture();
        let mut input = poses(&p);
        input[0] = input[1].clone();
        assert!(refine_placement_orientations(&p, &input, &Default::default()).is_err());
        let config = OrientationRefinementConfig {
            maximum_trials: 0,
            ..Default::default()
        };
        assert!(refine_placement_orientations(&p, &poses(&p), &config).is_err());
    }
}

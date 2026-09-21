// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

use layout_trace_model::{Problem, Vec2};
use pcb_routing::{DutGridRoutingConfig, DutGridRoutingResult};
use pcb_validate::SolvedComponent;
use serde::{Deserialize, Serialize};

use crate::{PressureRepairConfig, PressureRepairResult, repair_routing_with_pressure};

fn default_feedback_archive_capacity() -> usize {
    8
}

fn default_feedback_parent_count() -> usize {
    2
}

fn default_feedback_run_bound() -> usize {
    8
}

/// Controls repeated, multi-parent tracer-to-placer evolution.
///
/// `maximum_feedback_runs` is the exact global expensive-work bound. Each run
/// is additionally bounded by `PressureRepairConfig`; aggregate A* expansions
/// are evidence because the current router cannot interrupt a complete
/// multi-branch evaluation at an external expansion boundary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PressureFeedbackArchiveConfig {
    pub generations: usize,
    #[serde(default = "default_feedback_archive_capacity")]
    pub archive_capacity: usize,
    #[serde(default = "default_feedback_parent_count")]
    pub parents_per_generation: usize,
    #[serde(default = "default_feedback_run_bound")]
    pub maximum_feedback_runs: usize,
}

impl PressureFeedbackArchiveConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.generations == 0
            || self.archive_capacity == 0
            || self.parents_per_generation == 0
            || self.maximum_feedback_runs == 0
        {
            return Err("pressure feedback archive work bounds must be positive".into());
        }
        Ok(())
    }
}

impl Default for PressureFeedbackArchiveConfig {
    fn default() -> Self {
        Self {
            generations: 2,
            archive_capacity: default_feedback_archive_capacity(),
            parents_per_generation: default_feedback_parent_count(),
            maximum_feedback_runs: default_feedback_run_bound(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PressureFeedbackArchiveSeed {
    pub id: String,
    pub components: Vec<SolvedComponent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureFeedbackArchiveEntryDisposition {
    Retained,
    Evicted,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct PressureFeedbackArchiveRank {
    pub failed_branches: usize,
    pub exact_violations: usize,
    pub via_count: usize,
    pub route_length_micrometres: u64,
    pub bend_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct PressureFeedbackArchiveEntry {
    pub sequence: usize,
    pub root_seed: usize,
    pub root_seed_id: String,
    pub generation: usize,
    pub parent_sequence: Option<usize>,
    pub source_run: usize,
    pub disposition: PressureFeedbackArchiveEntryDisposition,
    pub rank: PressureFeedbackArchiveRank,
    pub routing: DutGridRoutingResult,
}

#[derive(Clone, Debug, Serialize)]
pub struct PressureFeedbackArchiveRun {
    pub ordinal: usize,
    pub generation: usize,
    pub root_seed: usize,
    pub root_seed_id: String,
    pub parent_sequence: Option<usize>,
    pub parent_pose_fingerprint: String,
    pub elapsed_micros: u64,
    pub result: Option<PressureRepairResult>,
    pub error: Option<String>,
    pub produced_sequence: Option<usize>,
    pub produced_duplicate_of: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PressureFeedbackArchiveEvidence {
    pub strategy: String,
    pub config: PressureFeedbackArchiveConfig,
    pub seed_count: usize,
    pub completed_generations: usize,
    pub attempted_feedback_runs: usize,
    pub failed_feedback_runs: usize,
    pub duplicate_candidates: usize,
    pub total_feedback_expansions: u64,
    pub termination: String,
    pub ranking_rule: String,
    pub retained_sequences: Vec<usize>,
    pub selected_sequence: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PressureFeedbackArchiveResult {
    pub evidence: PressureFeedbackArchiveEvidence,
    pub entries: Vec<PressureFeedbackArchiveEntry>,
    pub runs: Vec<PressureFeedbackArchiveRun>,
}

#[derive(Clone)]
struct PendingParent {
    root_seed: usize,
    root_seed_id: String,
    sequence: Option<usize>,
    components: Vec<SolvedComponent>,
}

/// Repeatedly expands the best unexpanded routed placements and retains a
/// bounded canonical archive. Every call to the feedback policy reroutes from
/// the supplied poses; neither provisional copper nor solver state is shared
/// between parents or generations.
pub fn evolve_routing_feedback_archive(
    problem: &Problem,
    seeds: &[PressureFeedbackArchiveSeed],
    routing_config: &DutGridRoutingConfig,
    repair_config: &PressureRepairConfig,
    archive_config: &PressureFeedbackArchiveConfig,
) -> Result<PressureFeedbackArchiveResult, String> {
    problem.check_schema()?;
    routing_config.check()?;
    repair_config.check()?;
    archive_config.check()?;
    if seeds.is_empty() {
        return Err("pressure feedback archive requires at least one seed".into());
    }
    if seeds.len() > archive_config.parents_per_generation {
        return Err(format!(
            "pressure feedback archive has {} seeds above parents_per_generation {}",
            seeds.len(),
            archive_config.parents_per_generation
        ));
    }
    if seeds.len() > archive_config.archive_capacity {
        return Err(format!(
            "pressure feedback archive has {} seeds above archive_capacity {}",
            seeds.len(),
            archive_config.archive_capacity
        ));
    }
    if seeds.len() > archive_config.maximum_feedback_runs {
        return Err(format!(
            "pressure feedback archive needs at least {} feedback runs to evaluate every seed",
            seeds.len()
        ));
    }
    let mut seed_ids = BTreeSet::new();
    for seed in seeds {
        if seed.id.is_empty() || !seed_ids.insert(seed.id.as_str()) {
            return Err("pressure feedback archive seed ids must be unique and non-empty".into());
        }
    }

    let mut entries = Vec::<PressureFeedbackArchiveEntry>::new();
    let mut runs = Vec::new();
    let mut pose_sequences = BTreeMap::<String, usize>::new();
    let mut expanded_poses = BTreeSet::new();
    let mut next_sequence = 0;
    let mut total_feedback_expansions = 0_u64;
    let mut failed_feedback_runs = 0;
    let mut duplicate_candidates = 0;
    let mut completed_generations = 0;
    let mut termination = "generation_limit".to_string();
    let mut parents = seeds
        .iter()
        .enumerate()
        .map(|(root_seed, seed)| PendingParent {
            root_seed,
            root_seed_id: seed.id.clone(),
            sequence: None,
            components: seed.components.clone(),
        })
        .collect::<Vec<_>>();

    for generation in 1..=archive_config.generations {
        if parents.is_empty() {
            termination = "no_unexpanded_parent".into();
            break;
        }
        let mut attempted_in_generation = 0;
        for parent in parents {
            if runs.len() >= archive_config.maximum_feedback_runs {
                termination = "feedback_run_limit".into();
                break;
            }
            let parent_fingerprint = component_pose_fingerprint(&parent.components);
            if !expanded_poses.insert(parent_fingerprint.clone()) {
                continue;
            }
            attempted_in_generation += 1;
            let run_ordinal = runs.len();
            let started = Instant::now();
            let evaluated = repair_routing_with_pressure(
                problem,
                &parent.components,
                routing_config,
                repair_config,
            );
            let elapsed_micros = started.elapsed().as_micros() as u64;
            match evaluated {
                Ok(result) => {
                    total_feedback_expansions =
                        total_feedback_expansions.saturating_add(result.evidence.total_expansions);
                    let initial_routing = result.attempts[0]
                        .routing
                        .as_ref()
                        .expect("successful pressure result retains initial routing")
                        .clone();
                    let parent_sequence = match parent.sequence {
                        Some(sequence) => sequence,
                        None => {
                            admit_feedback_archive_entry(
                                &mut entries,
                                &mut pose_sequences,
                                &mut next_sequence,
                                archive_config.archive_capacity,
                                parent.root_seed,
                                &parent.root_seed_id,
                                0,
                                None,
                                run_ordinal,
                                initial_routing,
                            )
                            .0
                        }
                    };
                    let selected_routing = result.attempts[result.evidence.selected_attempt]
                        .routing
                        .as_ref()
                        .expect("selected pressure attempt retains routing")
                        .clone();
                    let selected_fingerprint =
                        component_pose_fingerprint(&selected_routing.candidate.components);
                    let (produced_sequence, duplicate_of) =
                        if selected_fingerprint == parent_fingerprint {
                            (None, Some(parent_sequence))
                        } else {
                            let (sequence, duplicate) = admit_feedback_archive_entry(
                                &mut entries,
                                &mut pose_sequences,
                                &mut next_sequence,
                                archive_config.archive_capacity,
                                parent.root_seed,
                                &parent.root_seed_id,
                                generation,
                                Some(parent_sequence),
                                run_ordinal,
                                selected_routing,
                            );
                            (Some(sequence), duplicate)
                        };
                    if duplicate_of.is_some() {
                        duplicate_candidates += 1;
                    }
                    runs.push(PressureFeedbackArchiveRun {
                        ordinal: run_ordinal,
                        generation,
                        root_seed: parent.root_seed,
                        root_seed_id: parent.root_seed_id,
                        parent_sequence: Some(parent_sequence),
                        parent_pose_fingerprint: parent_fingerprint,
                        elapsed_micros,
                        result: Some(result),
                        error: None,
                        produced_sequence,
                        produced_duplicate_of: duplicate_of,
                    });
                }
                Err(error) => {
                    failed_feedback_runs += 1;
                    runs.push(PressureFeedbackArchiveRun {
                        ordinal: run_ordinal,
                        generation,
                        root_seed: parent.root_seed,
                        root_seed_id: parent.root_seed_id,
                        parent_sequence: parent.sequence,
                        parent_pose_fingerprint: parent_fingerprint,
                        elapsed_micros,
                        result: None,
                        error: Some(error),
                        produced_sequence: None,
                        produced_duplicate_of: None,
                    });
                }
            }
        }
        if attempted_in_generation > 0 {
            completed_generations = generation;
        }
        if runs.len() >= archive_config.maximum_feedback_runs {
            termination = "feedback_run_limit".into();
            break;
        }
        parents = ranked_retained_indices(&entries)
            .into_iter()
            .filter(|index| {
                !expanded_poses.contains(&component_pose_fingerprint(
                    &entries[*index].routing.candidate.components,
                ))
            })
            .take(archive_config.parents_per_generation)
            .map(|index| {
                let entry = &entries[index];
                PendingParent {
                    root_seed: entry.root_seed,
                    root_seed_id: entry.root_seed_id.clone(),
                    sequence: Some(entry.sequence),
                    components: entry.routing.candidate.components.clone(),
                }
            })
            .collect();
    }

    let retained_indices = ranked_retained_indices(&entries);
    let retained_sequences = retained_indices
        .iter()
        .map(|index| entries[*index].sequence)
        .collect::<Vec<_>>();
    let selected_sequence = retained_sequences.first().copied();
    Ok(PressureFeedbackArchiveResult {
        evidence: PressureFeedbackArchiveEvidence {
            strategy: "multi-parent-pressure-feedback-archive-v1".into(),
            config: archive_config.clone(),
            seed_count: seeds.len(),
            completed_generations,
            attempted_feedback_runs: runs.len(),
            failed_feedback_runs,
            duplicate_candidates,
            total_feedback_expansions,
            termination,
            ranking_rule: "failed branches, exact violations, via count, routed length, bends, stable sequence; native validation intentionally deferred".into(),
            retained_sequences,
            selected_sequence,
        },
        entries,
        runs,
    })
}

#[allow(clippy::too_many_arguments)]
fn admit_feedback_archive_entry(
    entries: &mut Vec<PressureFeedbackArchiveEntry>,
    pose_sequences: &mut BTreeMap<String, usize>,
    next_sequence: &mut usize,
    capacity: usize,
    root_seed: usize,
    root_seed_id: &str,
    generation: usize,
    parent_sequence: Option<usize>,
    source_run: usize,
    routing: DutGridRoutingResult,
) -> (usize, Option<usize>) {
    let fingerprint = component_pose_fingerprint(&routing.candidate.components);
    if let Some(sequence) = pose_sequences.get(&fingerprint).copied() {
        return (sequence, Some(sequence));
    }
    let sequence = *next_sequence;
    *next_sequence += 1;
    pose_sequences.insert(fingerprint, sequence);
    entries.push(PressureFeedbackArchiveEntry {
        sequence,
        root_seed,
        root_seed_id: root_seed_id.into(),
        generation,
        parent_sequence,
        source_run,
        disposition: PressureFeedbackArchiveEntryDisposition::Evicted,
        rank: feedback_archive_rank(&routing),
        routing,
    });
    let mut ranked = (0..entries.len()).collect::<Vec<_>>();
    ranked.sort_by_key(|index| (entries[*index].rank.clone(), entries[*index].sequence));
    let retained = ranked.into_iter().take(capacity).collect::<BTreeSet<_>>();
    for (index, entry) in entries.iter_mut().enumerate() {
        entry.disposition = if retained.contains(&index) {
            PressureFeedbackArchiveEntryDisposition::Retained
        } else {
            PressureFeedbackArchiveEntryDisposition::Evicted
        };
    }
    (sequence, None)
}

fn ranked_retained_indices(entries: &[PressureFeedbackArchiveEntry]) -> Vec<usize> {
    let mut ranked = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.disposition == PressureFeedbackArchiveEntryDisposition::Retained)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    ranked.sort_by_key(|index| (entries[*index].rank.clone(), entries[*index].sequence));
    ranked
}

fn feedback_archive_rank(routing: &DutGridRoutingResult) -> PressureFeedbackArchiveRank {
    let mut via_count = 0;
    let mut route_length_mm = 0.0;
    let mut bend_count = 0;
    for trace in &routing.candidate.traces {
        via_count += trace.vias.len();
        for points in trace.points.windows(2) {
            route_length_mm += (points[1].x - points[0].x).hypot(points[1].y - points[0].y);
        }
        for points in trace.points.windows(3) {
            let first = Vec2::new(points[1].x - points[0].x, points[1].y - points[0].y);
            let second = Vec2::new(points[2].x - points[1].x, points[2].y - points[1].y);
            if (first.x * second.y - first.y * second.x).abs() > 1.0e-9
                || first.x * second.x + first.y * second.y < 0.0
            {
                bend_count += 1;
            }
        }
    }
    PressureFeedbackArchiveRank {
        failed_branches: routing.evidence.failed_branches,
        exact_violations: routing.validation.violations.len(),
        via_count,
        route_length_micrometres: (route_length_mm * 1_000.0).round() as u64,
        bend_count,
    }
}

fn component_pose_fingerprint(components: &[SolvedComponent]) -> String {
    let mut poses = components
        .iter()
        .map(|component| {
            format!(
                "{}:{:016x}:{:016x}:{:016x}",
                component.id,
                component.position.x.to_bits(),
                component.position.y.to_bits(),
                component.rotation_degrees.to_bits()
            )
        })
        .collect::<Vec<_>>();
    poses.sort();
    poses.join("|")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall_problem() -> Problem {
        serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json"
        ))
        .unwrap()
    }

    fn declared_components(problem: &Problem) -> Vec<SolvedComponent> {
        problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect()
    }

    fn routing_config() -> DutGridRoutingConfig {
        DutGridRoutingConfig {
            retry_grid_mm: vec![0.25, 0.1],
            max_expansions_per_search: 5_000_000,
            ..DutGridRoutingConfig::default()
        }
    }

    #[test]
    fn archive_retains_seed_and_feedback_child_with_lineage() {
        let problem = wall_problem();
        let result = evolve_routing_feedback_archive(
            &problem,
            &[PressureFeedbackArchiveSeed {
                id: "wall".into(),
                components: declared_components(&problem),
            }],
            &routing_config(),
            &PressureRepairConfig::default(),
            &PressureFeedbackArchiveConfig {
                generations: 1,
                archive_capacity: 2,
                parents_per_generation: 1,
                maximum_feedback_runs: 1,
            },
        )
        .unwrap();

        assert_eq!(result.evidence.attempted_feedback_runs, 1);
        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.entries[0].generation, 0);
        assert_eq!(result.entries[1].generation, 1);
        assert_eq!(result.entries[1].parent_sequence, Some(0));
        assert_eq!(result.evidence.selected_sequence, Some(1));
        assert!(result.entries[1].routing.complete());
    }

    #[test]
    fn archive_bounds_cover_every_initial_parent() {
        let problem = wall_problem();
        let seeds = [PressureFeedbackArchiveSeed {
            id: "wall".into(),
            components: declared_components(&problem),
        }];
        let mut config = PressureFeedbackArchiveConfig::default();
        config.parents_per_generation = 0;
        assert!(
            evolve_routing_feedback_archive(
                &problem,
                &seeds,
                &routing_config(),
                &PressureRepairConfig::default(),
                &config,
            )
            .is_err()
        );
        config.parents_per_generation = 1;
        config.maximum_feedback_runs = 0;
        assert!(
            evolve_routing_feedback_archive(
                &problem,
                &seeds,
                &routing_config(),
                &PressureRepairConfig::default(),
                &config,
            )
            .is_err()
        );
    }
}

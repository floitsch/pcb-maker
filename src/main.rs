// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

mod freerouting_backend;

use pcb_core::{
    Board, Bounds, Component, ComponentKind, Connection, Frame, Layer, Mobility, Rules, Terminal,
    TerminalRef, Vec2,
};
use pcb_engine::{
    Backend, CompilePolicy, CpuReferenceBackend, NoField, RectangleRepresentation, SolverConfig,
    TraceSamplingPolicy, TwoTerminalRepresentation, compile_particle_world,
};
use std::{
    collections::{BTreeSet, VecDeque},
    env, fs,
    path::Path,
    process::{Command as ProcessCommand, ExitCode},
    time::Instant,
};

fn select_base_placement_fallback(
    feedback_final_rung: usize,
    fallback_final_rung: Option<usize>,
) -> bool {
    fallback_final_rung.is_some_and(|fallback| fallback >= feedback_final_rung)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "help".into());
    match command.as_str() {
        "check" => {
            let input = arguments
                .next()
                .ok_or_else(|| "usage: pcb-maker check <problem.json>".to_string())?;
            if arguments.next().is_some() {
                return Err("usage: pcb-maker check <problem.json>".into());
            }
            let problem = load_layout_trace_problem(Path::new(&input))?;
            println!(
                "valid layout-trace problem: {} components, {} legacy branches, {} electrical nets, {} layers",
                problem.components.len(),
                problem.nets.len(),
                problem.electrical_nets.len(),
                problem.board.layers.len()
            );
            Ok(())
        }
        "validate" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker validate <problem.json> <candidate.json>".to_string()
            })?;
            let candidate_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker validate <problem.json> <candidate.json>".to_string()
            })?;
            if arguments.next().is_some() {
                return Err("usage: pcb-maker validate <problem.json> <candidate.json>".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let source = fs::read_to_string(&candidate_path)
                .map_err(|error| format!("failed to read {candidate_path}: {error}"))?;
            let candidate = parse_candidate_or_result(&source)
                .map_err(|error| format!("failed to parse {candidate_path}: {error}"))?;
            let assessment = pcb_validate::validate_candidate(&problem, &candidate)
                .map_err(|error| format!("invalid {candidate_path}: {error}"))?;
            println!(
                "{}: {} physical finding(s), {} electrical finding(s)",
                if assessment.complete {
                    "valid"
                } else {
                    "invalid"
                },
                assessment.geometry.violations.len(),
                assessment.electrical.findings.len()
            );
            if assessment.complete {
                Ok(())
            } else {
                Err(format!(
                    "candidate failed exact validation with {} finding(s)",
                    assessment.violations.len()
                ))
            }
        }
        "insert-connection" => {
            let usage = "usage: pcb-maker insert-connection <source.problem.json> <target.problem.json> <parent.candidate-or-result.json> <output.json> [config.json]";
            let source_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let target_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let parent_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let config_path = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let source = load_layout_trace_problem(Path::new(&source_path))?;
            let target = load_layout_trace_problem(Path::new(&target_path))?;
            let parent_source = fs::read_to_string(&parent_path)
                .map_err(|error| format!("failed to read {parent_path}: {error}"))?;
            let parent = parse_candidate_or_result(&parent_source)
                .map_err(|error| format!("failed to parse {parent_path}: {error}"))?;
            let config = load_connection_insertion_config(config_path.as_deref())?;
            let result = pcb_coordinator::insert_connection(&source, &target, &parent, &config)
                .map_err(|error| error.to_string())?;
            result
                .check(&source, &target, &parent, &config)
                .map_err(|error| error.to_string())?;
            write_serialized_result(&result, Some(&output))?;
            println!(
                "wrote {output}: {:?}, branch={}, local_trials={}, attempts={}",
                result.disposition,
                result.evidence.extension.branch,
                result.evidence.attempts[0].local_trials.len(),
                result.evidence.attempts.len(),
            );
            if result.complete() {
                Ok(())
            } else {
                Err("connection insertion exhausted local and global repair; exact parent was returned".into())
            }
        }
        "trace-placement" => {
            let usage = "usage: pcb-maker trace-placement <problem.json> <policy-name|config.json> <trace.json>";
            let problem_path = arguments.next().ok_or(usage)?;
            let policy_name = arguments.next().ok_or(usage)?;
            let output = arguments.next().ok_or(usage)?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let config = load_initial_placement_config(&policy_name)?;
            let frames = std::cell::RefCell::new(Vec::new());
            let result = pcb_placement::run_initial_placement_traced(&problem, &config, &|frame| {
                frames.borrow_mut().push(frame)
            });
            let report = serde_json::json!({
                "schema_version": 1,
                "scope": "Harmonic seed and legalization sweeps only; diagnostic contacts do not establish native validity or routability.",
                "config": config, "frames": frames.into_inner(),
                "result": result.as_ref().ok(), "error": result.as_ref().err(),
            });
            write_serialized_result(&report, Some(&output))?;
            result.map(|_| ())
        }
        "place" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker place <problem.json> <policy-name|config.json> [result.json]"
                    .to_string()
            })?;
            let policy_name = arguments.next().ok_or_else(|| {
                "usage: pcb-maker place <problem.json> <policy-name|config.json> [result.json]"
                    .to_string()
            })?;
            let output = arguments.next();
            if arguments.next().is_some() {
                return Err(
                    "usage: pcb-maker place <problem.json> <policy-name|config.json> [result.json]"
                        .into(),
                );
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let config = load_initial_placement_config(&policy_name)?;
            let result = pcb_placement::run_initial_placement(&problem, &config)?;
            let serialized =
                serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?;
            if let Some(output) = output {
                if let Some(parent) = Path::new(&output).parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::write(&output, format!("{serialized}\n")).map_err(|error| error.to_string())?;
                println!(
                    "wrote {output}: policy={}, moved={}, distance={:.3}->{:.3} mm, common-work={} constraint + {} body-pair checks",
                    result.evidence.policy,
                    result.evidence.moved_components,
                    result.evidence.connectivity_distance_before_mm,
                    result.evidence.connectivity_distance_after_mm,
                    result.evidence.hard_constraint_checks,
                    result.evidence.body_pair_checks
                );
            } else {
                println!("{serialized}");
            }
            Ok(())
        }
        "route-grid" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker route-grid <problem.json> [result.json] [routing-config.json]"
                    .to_string()
            })?;
            let output = arguments.next();
            let routing_config = arguments.next();
            if arguments.next().is_some() {
                return Err(
                    "usage: pcb-maker route-grid <problem.json> [result.json] [routing-config.json]"
                        .into(),
                );
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let config = load_grid_routing_config(routing_config.as_deref())?;
            let result = pcb_routing::route_problem_with_dut_grid(&problem, &config)?;
            write_routing_result(&result, output.as_deref())?;
            report_routing_result(&result, output.as_deref());
            if result.complete() {
                Ok(())
            } else {
                Err(format!(
                    "routing is incomplete: {} of {} branches failed and exact validation found {} violation(s)",
                    result.evidence.failed_branches,
                    result.evidence.branch_count,
                    result.validation.violations.len()
                ))
            }
        }
        "route-grid-negotiated" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker route-grid-negotiated <problem.json> [result.json] [negotiated-config.json]"
                    .to_string()
            })?;
            let output = arguments.next();
            let routing_config = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker route-grid-negotiated <problem.json> [result.json] [negotiated-config.json]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let config = load_negotiated_routing_config(routing_config.as_deref())?;
            let result = pcb_routing::route_problem_with_negotiated_dut_grid(&problem, &config)?;
            write_negotiated_routing_result(&result, output.as_deref())?;
            report_negotiated_routing_result(&result, output.as_deref());
            if result.complete() {
                Ok(())
            } else {
                Err(format!(
                    "negotiated routing is incomplete: selected pass {} routed {}/{} with {} congested cell(s) and {} exact violation(s)",
                    result.evidence.selected_pass,
                    result.evidence.routing.routed_branches,
                    result.evidence.routing.branch_count,
                    result.evidence.passes[result.evidence.selected_pass].congested_cells,
                    result.validation.violations.len()
                ))
            }
        }
        "explore-conflict-actions" => {
            let usage = "usage: pcb-maker explore-conflict-actions <problem.json> [result.json] [config.json]";
            let problem_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next();
            let config_path = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let source = pcb_coordinator::candidate_from_declared_seed_routes(&problem)?;
            let config = load_conflict_action_config(config_path.as_deref())?;
            let result =
                pcb_coordinator::explore_one_axis_aligned_conflict(&problem, &source, &config)?;
            write_serialized_result(&result, output.as_deref())?;
            if let Some(output) = output.as_deref() {
                let selection = result.evidence.selected_attempt.map_or_else(
                    || "none".to_string(),
                    |index| {
                        format!(
                            "{} ({:.3} mm)",
                            result.attempts[index].action.id,
                            result.attempts[index].score.estimated_total_cost_mm
                        )
                    },
                );
                println!(
                    "wrote {output}: actions={}, exact={}, selected={selection}",
                    result.evidence.generated_actions, result.evidence.exact_complete_actions
                );
            }
            if result.evidence.complete {
                Ok(())
            } else {
                Err("no conflict action produced an exact-complete candidate".into())
            }
        }
        "search-conflict-actions" => {
            let usage = "usage: pcb-maker search-conflict-actions <problem.json> [result.json] [config.json]";
            let problem_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next();
            let config_path = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let source = pcb_coordinator::candidate_from_declared_seed_routes(&problem)?;
            let config = load_best_first_conflict_action_config(config_path.as_deref())?;
            let result = pcb_coordinator::search_axis_aligned_conflicts_best_first(
                &problem, &source, &config,
            )?;
            write_serialized_result(&result, output.as_deref())?;
            if let Some(output) = output.as_deref() {
                let selected = result.evidence.selected_attempt.map_or_else(
                    || "none".to_string(),
                    |index| {
                        format!(
                            "{} at depth {} ({:.3} mm)",
                            result.attempts[index].state_id,
                            result.attempts[index].depth,
                            result.attempts[index].score.path_cost_mm
                        )
                    },
                );
                println!(
                    "wrote {output}: expanded={}, transitions={}, unique={}, duplicates={}, exact={}, selected={selected}",
                    result.evidence.expanded_states,
                    result.evidence.generated_transitions,
                    result.evidence.unique_states,
                    result.evidence.duplicate_states,
                    result.evidence.exact_complete_states,
                );
            }
            if result.evidence.complete {
                Ok(())
            } else {
                Err("bounded conflict-action search found no exact-complete candidate".into())
            }
        }
        "continue-board" => {
            let usage = "usage: pcb-maker continue-board <problem.json> [result.json] [config.json] [progress-prefix]";
            let problem_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next();
            let config_path = arguments.next();
            let progress_prefix = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let config = load_board_continuation_config(config_path.as_deref())?;
            let result = pcb_coordinator::continue_board(&problem, &config)?;
            write_serialized_result(&result, output.as_deref())?;
            if let Some(prefix) = progress_prefix.as_deref() {
                for stage in &result.attempts {
                    render_progress_layer_pair(
                        &format!(
                            "cont {:02} x{:.2} {} u={} m={} p={} r={} rr={}",
                            stage.depth,
                            stage.scale,
                            if stage.accepted {
                                "ok"
                            } else {
                                "rejected"
                            },
                            stage.reused_previous_candidate,
                            stage.continuous_motion.as_ref().map_or("none", |motion| {
                                match motion.status {
                                    pcb_routing::ContinuousCandidateRepairStatus::ExactComplete => {
                                        "exact"
                                    }
                                    pcb_routing::ContinuousCandidateRepairStatus::RolledBack => {
                                        "rollback"
                                    }
                                    pcb_routing::ContinuousCandidateRepairStatus::NoRepairableFindings => {
                                        "no_findings"
                                    }
                                    pcb_routing::ContinuousCandidateRepairStatus::Unsupported => {
                                        "unsupported"
                                    }
                                }
                            }),
                            stage
                                .continuous_post_process
                                .as_ref()
                                .map_or("none", |post| post.status.as_str()),
                            stage.local_repair_rounds,
                            if stage.rerouted_branches.is_empty() {
                                "none".to_string()
                            } else {
                                stage.rerouted_branches.join(",")
                            },
                        ),
                        &stage.problem,
                        &stage.routing.candidate,
                        &format!("{prefix}-stage-{:02}", stage.depth),
                    )?;
                }
            }
            if let Some(output) = output.as_deref() {
                println!(
                    "wrote {output}: accepted={}/{}, last-scale={}, target={}",
                    result.evidence.accepted_stages,
                    result.evidence.attempted_stages,
                    result
                        .evidence
                        .last_valid_scale
                        .map_or_else(|| "none".to_string(), |scale| format!("{scale:.3}")),
                    if result.complete() {
                        "exact"
                    } else {
                        "incomplete"
                    }
                );
            }
            if result.complete() {
                Ok(())
            } else {
                Err(format!(
                    "board continuation stopped before the target at stage {:?}",
                    result.evidence.stopped_at_stage
                ))
            }
        }
        "place-route-grid" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker place-route-grid <problem.json> <placement-policy|config.json> [result.json] [routing-config.json]".to_string()
            })?;
            let placement_config = arguments.next().ok_or_else(|| {
                "usage: pcb-maker place-route-grid <problem.json> <placement-policy|config.json> [result.json] [routing-config.json]".to_string()
            })?;
            let output = arguments.next();
            let routing_config = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker place-route-grid <problem.json> <placement-policy|config.json> [result.json] [routing-config.json]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement = pcb_placement::run_initial_placement(
                &problem,
                &load_initial_placement_config(&placement_config)?,
            )?;
            let components = solved_components_from_placement(&problem, &placement)?;
            let config = load_grid_routing_config(routing_config.as_deref())?;
            let result =
                pcb_routing::route_problem_with_dut_grid_at_poses(&problem, &components, &config)?;
            write_routing_result(&result, output.as_deref())?;
            report_routing_result(&result, output.as_deref());
            if result.complete() {
                Ok(())
            } else {
                Err(format!(
                    "placement routed incompletely: {} of {} branches failed and exact validation found {} violation(s)",
                    result.evidence.failed_branches,
                    result.evidence.branch_count,
                    result.validation.violations.len()
                ))
            }
        }
        "place-route-grid-width-continuation" => {
            let usage = "usage: pcb-maker place-route-grid-width-continuation <problem.json> <placement-policy|config.json> <width-continuation-config.json> <output-directory>";
            let problem_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let placement_config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let continuation_config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let output_directory = Path::new(&output_directory);
            if output_directory.exists() {
                return Err(format!(
                    "refusing to overwrite width-continuation experiment {}",
                    output_directory.display()
                ));
            }
            fs::create_dir_all(output_directory).map_err(|error| error.to_string())?;
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement_config = load_initial_placement_config(&placement_config_path)?;
            let placement = pcb_placement::run_initial_placement(&problem, &placement_config)?;
            let components = solved_components_from_placement(&problem, &placement)?;
            let continuation_config = load_width_continuation_config(&continuation_config_path)?;
            let started = Instant::now();
            let result = pcb_routing::route_with_width_continuation_at_poses(
                &problem,
                &components,
                &continuation_config,
            )?;
            let elapsed_micros = started.elapsed().as_micros() as u64;
            write_pretty_json_file(&output_directory.join("problem.json"), &problem)?;
            write_pretty_json_file(
                &output_directory.join("placement-config.json"),
                &placement_config,
            )?;
            write_pretty_json_file(&output_directory.join("placement.json"), &placement)?;
            write_pretty_json_file(
                &output_directory.join("width-continuation-config.json"),
                &continuation_config,
            )?;
            write_pretty_json_file(&output_directory.join("result.json"), &result)?;
            write_pretty_json_file(
                &output_directory.join("timing.json"),
                &serde_json::json!({
                    "advisory_only": true,
                    "elapsed_micros": elapsed_micros
                }),
            )?;
            render_progress_layer_pair(
                &format!(
                    "thin seed at {:.0}% width",
                    continuation_config.initial_width_scale * 100.0
                ),
                &problem,
                &result.seed.candidate,
                &output_directory
                    .join("stage000-thin-seed")
                    .display()
                    .to_string(),
            )?;
            for (stage_index, stage) in result.stages.iter().enumerate() {
                let prefix = output_directory.join(format!(
                    "stage{:03}-width-{:03}",
                    stage_index + 1,
                    (stage.width_scale * 100.0).round() as usize
                ));
                render_progress_layer_pair(
                    &format!("width continuation {:.0}%", stage.width_scale * 100.0),
                    &problem,
                    &stage.candidate,
                    &prefix.display().to_string(),
                )?;
                if let Some(proposal) = &stage.proposed_candidate {
                    render_progress_layer_pair(
                        &format!("stalled width proposal {:.0}%", stage.width_scale * 100.0),
                        &problem,
                        proposal,
                        &format!("{}-proposal", prefix.display()),
                    )?;
                }
            }
            println!(
                "wrote {}: status={:?}, seed={}/{}, reached-width={:.0}%, full-width-findings={}, elapsed={:.3}s",
                output_directory.display(),
                result.status,
                result.seed.evidence.routed_branches,
                result.seed.evidence.branch_count,
                result.reached_width_scale * 100.0,
                result.full_width_validation.violations.len(),
                elapsed_micros as f64 / 1_000_000.0
            );
            if result.complete() {
                Ok(())
            } else {
                Err(format!(
                    "width continuation stopped with status {:?} at {:.0}% width; diagnostic proposals and the last exact candidate remain retained",
                    result.status,
                    result.reached_width_scale * 100.0
                ))
            }
        }
        "place-route-grid-negotiated" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker place-route-grid-negotiated <problem.json> <placement-policy|config.json> [result.json] [negotiated-config.json]".to_string()
            })?;
            let placement_config = arguments.next().ok_or_else(|| {
                "usage: pcb-maker place-route-grid-negotiated <problem.json> <placement-policy|config.json> [result.json] [negotiated-config.json]".to_string()
            })?;
            let output = arguments.next();
            let routing_config = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker place-route-grid-negotiated <problem.json> <placement-policy|config.json> [result.json] [negotiated-config.json]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement = pcb_placement::run_initial_placement(
                &problem,
                &load_initial_placement_config(&placement_config)?,
            )?;
            let components = solved_components_from_placement(&problem, &placement)?;
            let config = load_negotiated_routing_config(routing_config.as_deref())?;
            let result = pcb_routing::route_problem_with_negotiated_dut_grid_at_poses(
                &problem,
                &components,
                &config,
            )?;
            write_negotiated_routing_result(&result, output.as_deref())?;
            report_negotiated_routing_result(&result, output.as_deref());
            if result.complete() {
                Ok(())
            } else {
                Err(format!(
                    "negotiated placement routing remained incomplete after {} pass(es)",
                    result.evidence.completed_passes
                ))
            }
        }
        "analyze-corridors" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker analyze-corridors <problem.json> <placement-policy|config.json> [result.json] [corridor-config.json]".to_string()
            })?;
            let placement_config = arguments.next().ok_or_else(|| {
                "usage: pcb-maker analyze-corridors <problem.json> <placement-policy|config.json> [result.json] [corridor-config.json]".to_string()
            })?;
            let output = arguments.next();
            let corridor_config = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker analyze-corridors <problem.json> <placement-policy|config.json> [result.json] [corridor-config.json]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement = pcb_placement::run_initial_placement(
                &problem,
                &load_initial_placement_config(&placement_config)?,
            )?;
            let components = solved_components_from_placement(&problem, &placement)?;
            let config = load_corridor_analysis_config(corridor_config.as_deref())?;
            let result =
                pcb_routing::analyze_problem_corridors_at_poses(&problem, &components, &config)?;
            write_corridor_analysis_result(&result, output.as_deref())?;
            if let Some(output) = output.as_deref() {
                println!(
                    "wrote {output}: corridor-layers={}, embedded={}/{}, cells={}, gates={}",
                    result.corridors.len(),
                    result.evidence.successful_branches,
                    result.evidence.branch_count,
                    result
                        .evidence
                        .layer_work
                        .iter()
                        .map(|layer| layer.cells)
                        .sum::<usize>(),
                    result
                        .evidence
                        .layer_work
                        .iter()
                        .map(|layer| layer.gates)
                        .sum::<usize>()
                );
            }
            if result.complete() {
                Ok(())
            } else {
                Err(format!(
                    "corridor analysis has {} failed branch embedding(s)",
                    result.evidence.failed_branches
                ))
            }
        }
        "analyze-route-families" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker analyze-route-families <problem.json> <placement-policy|config.json> [result.json] [family-config.json]".to_string()
            })?;
            let placement_config = arguments.next().ok_or_else(|| {
                "usage: pcb-maker analyze-route-families <problem.json> <placement-policy|config.json> [result.json] [family-config.json]".to_string()
            })?;
            let output = arguments.next();
            let family_config = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker analyze-route-families <problem.json> <placement-policy|config.json> [result.json] [family-config.json]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement = pcb_placement::run_initial_placement(
                &problem,
                &load_initial_placement_config(&placement_config)?,
            )?;
            let components = solved_components_from_placement(&problem, &placement)?;
            let config = load_route_family_analysis_config(family_config.as_deref())?;
            let result = pcb_routing::analyze_problem_route_families_at_poses(
                &problem,
                &components,
                &config,
            )?;
            write_route_family_analysis_result(&result, output.as_deref())?;
            if let Some(output) = output.as_deref() {
                println!(
                    "wrote {output}: status={:?}, variables={}, family-searches={}, assignment-searches={}, visibility-states={}",
                    result.evidence.status,
                    result
                        .portfolio
                        .as_ref()
                        .map_or(0, |portfolio| portfolio.variables.len()),
                    result.production_work.family_searches,
                    result.production_work.assignment_searches,
                    result.production_work.visibility_states
                );
            }
            if result.complete() {
                Ok(())
            } else {
                Err(format!("route-family analysis: {}", result.evidence.detail))
            }
        }
        "materialize-route-family-portfolio" => {
            let usage = "usage: pcb-maker materialize-route-family-portfolio <problem.json> <portfolio.json> <placement-policy|config.json> [result.json]";
            let problem_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let portfolio_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let placement_config = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement = pcb_placement::run_initial_placement(
                &problem,
                &load_initial_placement_config(&placement_config)?,
            )?;
            let components = solved_components_from_placement(&problem, &placement)?;
            let source = fs::read_to_string(&portfolio_path)
                .map_err(|error| format!("failed to read {portfolio_path}: {error}"))?;
            let portfolio = pcb_routing::parse_route_family_portfolio(&source)
                .map_err(|error| format!("failed to parse {portfolio_path}: {error}"))?;
            let result = pcb_routing::materialize_route_family_portfolio_at_poses(
                &problem,
                &components,
                &portfolio,
                Default::default(),
            )?;
            write_serialized_result(&result, output.as_deref())?;
            if let Some(output) = output.as_deref() {
                println!(
                    "wrote {output}: schema={}, variables={}, exact={}",
                    result.portfolio.schema_version,
                    result.portfolio.variables.len(),
                    if result.complete() {
                        "pass"
                    } else {
                        "not-materialized"
                    }
                );
            }
            if result.complete() {
                Ok(())
            } else {
                Err("route-family portfolio did not select and materialize an exact-complete candidate".into())
            }
        }
        "repair-route-families" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker repair-route-families <problem.json> <candidate-or-result.json> [result.json] [family-config.json]".to_string()
            })?;
            let candidate_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker repair-route-families <problem.json> <candidate-or-result.json> [result.json] [family-config.json]".to_string()
            })?;
            let output = arguments.next();
            let family_config = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker repair-route-families <problem.json> <candidate-or-result.json> [result.json] [family-config.json]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let source = fs::read_to_string(&candidate_path)
                .map_err(|error| format!("failed to read {candidate_path}: {error}"))?;
            let candidate = parse_candidate_or_result(&source)
                .map_err(|error| format!("failed to parse {candidate_path}: {error}"))?;
            let config = load_route_family_analysis_config(family_config.as_deref())?;
            let result =
                pcb_routing::repair_candidate_route_families(&problem, &candidate, &config)?;
            write_route_family_repair_result(&result, output.as_deref())?;
            if let Some(output) = output.as_deref() {
                println!(
                    "wrote {output}: status={:?}, selected={}, conflicts={}->{}, findings={}->{}, family-searches={}, assignment-searches={}",
                    result.evidence.status,
                    result.evidence.selected_branches.len(),
                    result.evidence.original_selected_conflicts,
                    result
                        .evidence
                        .proposed_selected_conflicts
                        .map_or("-".into(), |value| value.to_string()),
                    result.evidence.original_findings,
                    result
                        .evidence
                        .proposed_findings
                        .map_or("-".into(), |value| value.to_string()),
                    result.production_work.family_searches,
                    result.production_work.assignment_searches
                );
            }
            if result.accepted() {
                Ok(())
            } else {
                Err(format!("route-family repair: {}", result.evidence.detail))
            }
        }
        "repair-route-grid" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker repair-route-grid <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [repair-config.json]".to_string()
            })?;
            let placement_config = arguments.next().ok_or_else(|| {
                "usage: pcb-maker repair-route-grid <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [repair-config.json]".to_string()
            })?;
            let output = arguments.next();
            let routing_config = arguments.next();
            let repair_config = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker repair-route-grid <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [repair-config.json]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement = pcb_placement::run_initial_placement(
                &problem,
                &load_initial_placement_config(&placement_config)?,
            )?;
            let components = solved_components_from_placement(&problem, &placement)?;
            let routing_config = load_grid_routing_config(routing_config.as_deref())?;
            let repair_config = load_blocker_repair_config(repair_config.as_deref())?;
            let result = pcb_coordinator::repair_routing_blockers(
                &problem,
                &components,
                &routing_config,
                &repair_config,
            )?;
            write_serialized_result(&result, output.as_deref())?;
            if let Some(output) = output.as_deref() {
                println!(
                    "wrote {output}: selected attempt {}/{}, moved={}, routed={}/{}, exact={}",
                    result.evidence.selected_attempt,
                    result.attempts.len() - 1,
                    result.attempts[result.evidence.selected_attempt]
                        .blocker
                        .as_deref()
                        .unwrap_or("none"),
                    result.evidence.routing.routed_branches,
                    result.evidence.routing.branch_count,
                    if result.validation.complete {
                        "pass"
                    } else {
                        "fail"
                    }
                );
            }
            if result.evidence.complete {
                Ok(())
            } else {
                Err(format!(
                    "blocker repair remained incomplete after {} repair attempt(s)",
                    result.evidence.attempted_repairs
                ))
            }
        }
        "optimize-route-junctions" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker optimize-route-junctions <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [junction-config.json]".to_string()
            })?;
            let placement_config = arguments.next().ok_or_else(|| {
                "usage: pcb-maker optimize-route-junctions <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [junction-config.json]".to_string()
            })?;
            let output = arguments.next();
            let routing_config = arguments.next();
            let junction_config = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker optimize-route-junctions <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [junction-config.json]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement = pcb_placement::run_initial_placement(
                &problem,
                &load_initial_placement_config(&placement_config)?,
            )?;
            let components = solved_components_from_placement(&problem, &placement)?;
            let routing_config = load_grid_routing_config(routing_config.as_deref())?;
            let junction_config = load_route_junction_config(junction_config.as_deref())?;
            let result = pcb_coordinator::optimize_route_junctions(
                &problem,
                &components,
                &routing_config,
                &junction_config,
            )?;
            write_serialized_result(&result, output.as_deref())?;
            if let Some(output) = output.as_deref() {
                println!(
                    "wrote {output}: selected attempt {}/{}, accepted={}, traces={}, length={:.6}->{:.6} mm, exact={}",
                    result.evidence.selected_attempt,
                    result.attempts.len() - 1,
                    result.evidence.accepted,
                    result.candidate.traces.len(),
                    result.evidence.baseline_trace_length_mm,
                    result.evidence.selected_trace_length_mm,
                    if result.validation.complete {
                        "pass"
                    } else {
                        "fail"
                    }
                );
            }
            if result.evidence.complete {
                Ok(())
            } else {
                Err("route junction optimization did not retain a complete candidate".into())
            }
        }
        "repair-route-pressure" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker repair-route-pressure <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [pressure-config.json]".to_string()
            })?;
            let placement_config = arguments.next().ok_or_else(|| {
                "usage: pcb-maker repair-route-pressure <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [pressure-config.json]".to_string()
            })?;
            let output = arguments.next();
            let routing_config = arguments.next();
            let pressure_config = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker repair-route-pressure <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [pressure-config.json]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement = pcb_placement::run_initial_placement(
                &problem,
                &load_initial_placement_config(&placement_config)?,
            )?;
            let components = solved_components_from_placement(&problem, &placement)?;
            let routing_config = load_grid_routing_config(routing_config.as_deref())?;
            let pressure_config = load_pressure_repair_config(pressure_config.as_deref())?;
            let result = pcb_coordinator::repair_routing_with_pressure(
                &problem,
                &components,
                &routing_config,
                &pressure_config,
            )?;
            write_serialized_result(&result, output.as_deref())?;
            if let Some(output) = output.as_deref() {
                println!(
                    "wrote {output}: selected attempt {}/{}, iterations={}, routed={}/{}, total-expansions={}, exact={}",
                    result.evidence.selected_attempt,
                    result.attempts.len() - 1,
                    result.evidence.completed_iterations,
                    result.evidence.routing.routed_branches,
                    result.evidence.routing.branch_count,
                    result.evidence.total_expansions,
                    if result.validation.complete {
                        "pass"
                    } else {
                        "fail"
                    }
                );
            }
            if result.evidence.complete {
                Ok(())
            } else {
                Err(format!(
                    "pressure repair remained incomplete after {} repair attempt(s)",
                    result.evidence.attempted_repairs
                ))
            }
        }
        "repair-route-order" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker repair-route-order <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [repair-config.json]".to_string()
            })?;
            let placement_config = arguments.next().ok_or_else(|| {
                "usage: pcb-maker repair-route-order <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [repair-config.json]".to_string()
            })?;
            let output = arguments.next();
            let routing_config = arguments.next();
            let repair_config = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker repair-route-order <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [repair-config.json]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement = pcb_placement::run_initial_placement(
                &problem,
                &load_initial_placement_config(&placement_config)?,
            )?;
            let components = solved_components_from_placement(&problem, &placement)?;
            let routing_config = load_grid_routing_config(routing_config.as_deref())?;
            let repair_config = load_failure_directed_order_config(repair_config.as_deref())?;
            let result = pcb_coordinator::repair_route_order(
                &problem,
                &components,
                &routing_config,
                &repair_config,
            )?;
            write_order_repair_result(&result, output.as_deref())?;
            if let Some(output) = output.as_deref() {
                println!(
                    "wrote {output}: selected attempt {}/{}, routed={}/{}, exact={}",
                    result.evidence.selected_attempt,
                    result.attempts.len() - 1,
                    result.evidence.routing.routed_branches,
                    result.evidence.routing.branch_count,
                    if result.validation.complete {
                        "pass"
                    } else {
                        "fail"
                    }
                );
            }
            if result.evidence.complete {
                Ok(())
            } else {
                Err(format!(
                    "failure-directed ordering remained incomplete after {} reorder attempt(s)",
                    result.evidence.attempted_reorders
                ))
            }
        }
        "repair-route-ripup" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker repair-route-ripup <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [ripup-config.json]".to_string()
            })?;
            let placement_config = arguments.next().ok_or_else(|| {
                "usage: pcb-maker repair-route-ripup <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [ripup-config.json]".to_string()
            })?;
            let output = arguments.next();
            let routing_config = arguments.next();
            let repair_config = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker repair-route-ripup <problem.json> <placement-policy|config.json> [result.json] [routing-config.json] [ripup-config.json]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement = pcb_placement::run_initial_placement(
                &problem,
                &load_initial_placement_config(&placement_config)?,
            )?;
            let components = solved_components_from_placement(&problem, &placement)?;
            let routing_config = load_grid_routing_config(routing_config.as_deref())?;
            let repair_config = load_selective_ripup_config(repair_config.as_deref())?;
            let result = pcb_coordinator::repair_with_selective_ripup(
                &problem,
                &components,
                &routing_config,
                &repair_config,
            )?;
            write_selective_ripup_result(&result, output.as_deref())?;
            if let Some(output) = output.as_deref() {
                println!(
                    "wrote {output}: selected attempt {}/{}, routed={}/{}, selected-expansions={}, total-expansions={}, exact={}",
                    result.evidence.selected_attempt,
                    result.attempts.len() - 1,
                    result.evidence.routing.routed_branches,
                    result.evidence.routing.branch_count,
                    result.evidence.routing.expansions,
                    result.evidence.total_expansions,
                    if result.validation.complete {
                        "pass"
                    } else {
                        "fail"
                    }
                );
            }
            if result.evidence.complete {
                Ok(())
            } else {
                Err(format!(
                    "selective rip-up remained incomplete after {} repair attempt(s)",
                    result.evidence.attempted_repairs
                ))
            }
        }
        "view-route" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker view-route <problem.json> <candidate-or-result.json> [output.html]"
                    .to_string()
            })?;
            let result_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker view-route <problem.json> <candidate-or-result.json> [output.html]"
                    .to_string()
            })?;
            let output = arguments.next().unwrap_or_else(|| "run/route.html".into());
            if arguments.next().is_some() {
                return Err("usage: pcb-maker view-route <problem.json> <candidate-or-result.json> [output.html]".into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let source = fs::read_to_string(&result_path)
                .map_err(|error| format!("failed to read {result_path}: {error}"))?;
            let document: serde_json::Value = serde_json::from_str(&source)
                .map_err(|error| format!("failed to parse {result_path}: {error}"))?;
            let candidate = parse_candidate_or_result(&source)
                .map_err(|error| format!("failed to parse {result_path}: {error}"))?;
            let validation = pcb_validate::validate_candidate(&problem, &candidate)?;
            let mut evidence = document
                .get("evidence")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            if let Some(attempts) = document.get("attempts") {
                let mut viewer_attempts = attempts.as_array().cloned().unwrap_or_default();
                let already_has_seed = viewer_attempts
                    .first()
                    .and_then(|attempt| attempt.get("state_id"))
                    .and_then(serde_json::Value::as_str)
                    == Some("seed");
                if !already_has_seed
                    && let Some(source_candidate) = document.get("source_candidate")
                {
                    viewer_attempts.insert(
                        0,
                        serde_json::json!({
                            "id": "seed",
                            "state_id": "seed",
                            "depth": 0,
                            "candidate": source_candidate,
                            "validation": evidence.get("source_validation").cloned()
                                .unwrap_or_else(|| serde_json::json!({"complete": false, "violations": []}))
                        }),
                    );
                    if let Some(selected) = evidence["selected_attempt"].as_u64() {
                        evidence["selected_attempt"] = serde_json::json!(selected + 1);
                    }
                }
                evidence["attempts"] = serde_json::Value::Array(viewer_attempts);
            }
            if let Some(transitions) = document.get("transitions") {
                evidence["transitions"] = transitions.clone();
            }
            let html = pcb_viewer::render_candidate_html(
                "PCB route candidate",
                &problem,
                &candidate,
                &validation,
                &evidence,
            )
            .map_err(|error| error.to_string())?;
            let path = Path::new(&output);
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(path, html).map_err(|error| error.to_string())?;
            println!(
                "wrote {output}: {} traces, {} exact finding(s)",
                candidate.traces.len(),
                validation.violations.len()
            );
            Ok(())
        }
        "render-route-layers" => {
            let usage = "usage: pcb-maker render-route-layers <problem.json> <candidate-or-result.json> <output-prefix>";
            let problem_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let result_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_prefix = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let source = fs::read_to_string(&result_path)
                .map_err(|error| format!("failed to read {result_path}: {error}"))?;
            let candidate = parse_candidate_or_result(&source)
                .map_err(|error| format!("failed to parse {result_path}: {error}"))?;
            render_progress_layer_pair(
                &Path::new(&problem_path).display().to_string(),
                &problem,
                &candidate,
                &output_prefix,
            )?;
            Ok(())
        }
        "view-continuous-repair" => {
            let result_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker view-continuous-repair <route-family-result.json> [output.html]"
                    .to_string()
            })?;
            let output = arguments
                .next()
                .unwrap_or_else(|| "run/continuous-repair.html".into());
            if arguments.next().is_some() {
                return Err("usage: pcb-maker view-continuous-repair <route-family-result.json> [output.html]".into());
            }
            let source = fs::read_to_string(&result_path)
                .map_err(|error| format!("failed to read {result_path}: {error}"))?;
            let document: serde_json::Value = serde_json::from_str(&source)
                .map_err(|error| format!("failed to parse {result_path}: {error}"))?;
            let frames_value = document
                .pointer("/continuous_repair/frames")
                .or_else(|| document.pointer("/continuous_repair_attempt/frames"))
                .or_else(|| document.pointer("/frames"))
                .ok_or_else(|| {
                    format!("{result_path} does not contain continuous family-repair frames")
                })?;
            let frames: Vec<Frame> = serde_json::from_value(frames_value.clone())
                .map_err(|error| format!("invalid continuous repair frames: {error}"))?;
            if frames.is_empty() {
                return Err(format!(
                    "{result_path} contains no continuous family-repair frames"
                ));
            }
            let html = pcb_viewer::render_html("continuous family-obligation repair", &frames)
                .map_err(|error| error.to_string())?;
            let path = Path::new(&output);
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(path, html).map_err(|error| error.to_string())?;
            println!("wrote {output} with {} correction frames", frames.len());
            Ok(())
        }
        "relax-continuous" => {
            let problem_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker relax-continuous <problem.json> [result.json] [viewer.html] [subdivide|preserve-seed]"
                    .to_string()
            })?;
            let result_path = arguments
                .next()
                .unwrap_or_else(|| "run/continuous-relax.json".into());
            let viewer_path = arguments
                .next()
                .unwrap_or_else(|| "run/continuous-relax.html".into());
            let trace_sampling = match arguments.next().as_deref() {
                None | Some("subdivide") => TraceSamplingPolicy::SubdivideToMaximumSpacing,
                Some("preserve-seed") => TraceSamplingPolicy::PreserveSeedVertices,
                Some(other) => {
                    return Err(format!(
                        "unknown trace sampling policy {other:?}; expected subdivide or preserve-seed"
                    ));
                }
            };
            if arguments.next().is_some() {
                return Err(
                    "usage: pcb-maker relax-continuous <problem.json> [result.json] [viewer.html] [subdivide|preserve-seed]"
                        .into(),
                );
            }
            let problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let mut adapter = pcb_routing::compile_problem_for_continuous_engine(
                &problem,
                CompilePolicy {
                    two_terminal_representation:
                        TwoTerminalRepresentation::AnalyticRigidBodyAttachments,
                    rectangle_representation: RectangleRepresentation::AnalyticRigidBodyAttachments,
                    trace_sampling,
                    maximum_trace_spacing: 2.0,
                    ..CompilePolicy::default()
                },
            )?;
            let initial_solution = adapter.solution()?;
            let mut backend = CpuReferenceBackend::with_field(NoField);
            let config = SolverConfig {
                field_strength: 0.0,
                projection_iterations: 8,
                ..SolverConfig::default()
            };
            let initial_config = SolverConfig {
                projection_iterations: 0,
                ..config
            };
            let mut frames = vec![backend.step(&mut adapter.compiled.world, &initial_config, 0)];
            for step in 1..=20 {
                frames.push(backend.step(&mut adapter.compiled.world, &config, step));
            }
            let final_solution = adapter.solution()?;
            let candidate = adapter.materialize_candidate()?;
            let validation = pcb_validate::validate_candidate(&problem, &candidate)?;
            let result = serde_json::json!({
                "strategy": "continuous-analytic-body-attachments-cpu-v1",
                "input": problem_path,
                "configuration": {
                    "steps": 20,
                    "projection_iterations": config.projection_iterations,
                    "maximum_trace_spacing_mm": 2.0,
                    "maximum_trace_particles_per_connection": 1_000_000,
                    "trace_sampling": format!("{trace_sampling:?}"),
                    "field": "disabled"
                },
                "initial_solution": initial_solution,
                "final_solution": final_solution,
                "candidate": candidate,
                "validation": validation,
                "frames": frames
            });
            for path in [&result_path, &viewer_path] {
                if let Some(parent) = Path::new(path)
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
            }
            fs::write(
                &result_path,
                serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            fs::write(
                &viewer_path,
                pcb_viewer::render_html("continuous coupled relaxation", &frames)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            println!(
                "wrote {result_path} and {viewer_path}: {} exact finding(s)",
                validation.violations.len()
            );
            Ok(())
        }
        "resistor-field" => {
            let output = arguments
                .next()
                .unwrap_or_else(|| "run/resistor-field.html".into());
            if arguments.next().is_some() {
                return Err("usage: pcb-maker resistor-field [output.html]".into());
            }
            let frames = resistor_field_frames()?;
            let html =
                pcb_viewer::render_html("particleized resistor + Eulerian density field", &frames)
                    .map_err(|error| error.to_string())?;
            if let Some(parent) = std::path::Path::new(&output).parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(&output, html).map_err(|error| error.to_string())?;
            println!(
                "wrote {output} with {} frames using {}",
                frames.len(),
                CpuReferenceBackend::new().name()
            );
            Ok(())
        }
        "resistor-ablation" => {
            let output_prefix = arguments
                .next()
                .unwrap_or_else(|| "run/resistor-ablation".into());
            if arguments.next().is_some() {
                return Err("usage: pcb-maker resistor-ablation [output-prefix]".into());
            }
            let endpoint_frames = resistor_field_frames_with_representation(
                TwoTerminalRepresentation::ExperimentalEndpointParticles,
            )?;
            let rigid_frames = resistor_field_frames_with_representation(
                TwoTerminalRepresentation::AnalyticRigidBodyAttachments,
            )?;
            let endpoint_rotation_frames = resistor_rotation_control_frames(
                TwoTerminalRepresentation::ExperimentalEndpointParticles,
            )?;
            let rigid_rotation_frames = resistor_rotation_control_frames(
                TwoTerminalRepresentation::AnalyticRigidBodyAttachments,
            )?;
            let endpoint_path = format!("{output_prefix}-endpoint.html");
            let rigid_path = format!("{output_prefix}-rigid.html");
            let endpoint_rotation_path = format!("{output_prefix}-rotation-endpoint.html");
            let rigid_rotation_path = format!("{output_prefix}-rotation-rigid.html");
            let evidence_path = format!("{output_prefix}.json");
            if let Some(parent) = Path::new(&output_prefix)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(
                &endpoint_path,
                pcb_viewer::render_html("resistor ablation: endpoint particles", &endpoint_frames)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            fs::write(
                &rigid_path,
                pcb_viewer::render_html("resistor ablation: analytic rigid body", &rigid_frames)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            fs::write(
                &endpoint_rotation_path,
                pcb_viewer::render_html(
                    "rotation control: endpoint particles",
                    &endpoint_rotation_frames,
                )
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            fs::write(
                &rigid_rotation_path,
                pcb_viewer::render_html(
                    "rotation control: analytic rigid body",
                    &rigid_rotation_frames,
                )
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            let evidence = serde_json::json!({
                "field_load": {
                    "fixture": "resistor_field_fixture",
                    "endpoint_particles": resistor_representation_evidence(&endpoint_frames)?,
                    "analytic_rigid_body": resistor_representation_evidence(&rigid_frames)?
                },
                "asymmetric_terminal_perturbation": {
                    "fixture": "resistor_rotation_fixture",
                    "endpoint_particles": resistor_representation_evidence(&endpoint_rotation_frames)?,
                    "analytic_rigid_body": resistor_representation_evidence(&rigid_rotation_frames)?
                },
                "limitations": [
                    "CPU reference only; no GPU timing or occupancy measurement",
                    "this standalone representation ablation does not run candidate materialization or exact validation",
                    "one fixture is diagnostic evidence, not a representation verdict"
                ]
            });
            fs::write(
                &evidence_path,
                serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            println!("wrote matched field-load and rotation-control viewers plus {evidence_path}");
            Ok(())
        }
        "generate-semantic-kicad-ladder" => {
            let usage = "usage: pcb-maker generate-semantic-kicad-ladder <problem.json> <placement-policy|config.json> <template-config.json> <output-directory>";
            let problem_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let placement_policy = arguments.next().ok_or_else(|| usage.to_string())?;
            let template_config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let (_, _, placement, _, report, timing) = generate_semantic_kicad_ladder_from_inputs(
                Path::new(&problem_path),
                &placement_policy,
                Path::new(&template_config_path),
                None,
                Path::new(&output_directory),
            )?;
            write_pretty_json_file(
                &Path::new(&output_directory).join("generation-timing.json"),
                &timing,
            )?;
            println!(
                "wrote {}: {} semantic component(s), {} schematic component(s), {} electrical connection(s); placement policy={}, moved={}, distance={:.3}->{:.3} mm",
                output_directory,
                report.semantic_components,
                report.schematic_components,
                report.electrical_connections,
                placement.evidence.policy,
                placement.evidence.moved_components,
                placement.evidence.connectivity_distance_before_mm,
                placement.evidence.connectivity_distance_after_mm,
            );
            Ok(())
        }
        "solve-semantic-kicad-prefix" => {
            let usage = "usage: pcb-maker solve-semantic-kicad-prefix <problem.json> <placement-policy|config.json> <template-config.json> <target-rung> <output-directory> [insertion-config.json]";
            let problem_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let placement_policy = arguments.next().ok_or_else(|| usage.to_string())?;
            let template_config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let target_rung = arguments
                .next()
                .ok_or_else(|| usage.to_string())?
                .parse::<usize>()
                .map_err(|error| format!("target-rung must be a non-negative integer: {error}"))?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let insertion_config_path = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let output_directory = Path::new(&output_directory);
            if output_directory.exists() {
                return Err(format!(
                    "refusing to overwrite cold semantic KiCad prefix {}",
                    output_directory.display()
                ));
            }
            let cold_started = Instant::now();
            let insertion =
                load_kicad_connection_insertion_config(insertion_config_path.as_deref())?;
            let generated_directory = output_directory.join("generated-ladder");
            let generation_started = Instant::now();
            let (
                active_problem,
                placement_config,
                placement,
                active_template_config,
                template,
                generation_timing,
            ) = generate_semantic_kicad_ladder_from_inputs(
                Path::new(&problem_path),
                &placement_policy,
                Path::new(&template_config_path),
                Some(target_rung),
                &generated_directory,
            )?;
            let generation_elapsed_micros = generation_started.elapsed().as_micros() as u64;
            if target_rung > template.electrical_connections {
                return Err(format!(
                    "target rung {target_rung} exceeds the generated ladder's {} connections",
                    template.electrical_connections
                ));
            }

            fs::copy(&problem_path, output_directory.join("source-problem.json"))
                .map_err(|error| format!("failed to snapshot {problem_path}: {error}"))?;
            fs::copy(
                &template_config_path,
                output_directory.join("source-template-config.json"),
            )
            .map_err(|error| format!("failed to snapshot {template_config_path}: {error}"))?;
            write_pretty_json_file(
                &output_directory.join("active-prefix-problem.json"),
                &active_problem,
            )?;
            write_pretty_json_file(
                &output_directory.join("placement-config.json"),
                &placement_config,
            )?;
            write_pretty_json_file(
                &output_directory.join("template-config.json"),
                &active_template_config,
            )?;
            write_pretty_json_file(&output_directory.join("insertion-config.json"), &insertion)?;

            let declaration = pcb_kicad::load_declaration(&template.declaration)?;
            let materialization_started = Instant::now();
            let initial =
                pcb_kicad::materialize_rung(&declaration, 0, &output_directory.join("initial"))?;
            let initial_materialization_elapsed_micros =
                materialization_started.elapsed().as_micros() as u64;
            let initial_verification = if target_rung == 0 {
                Some(pcb_kicad::verify_materialized_rung(
                    &initial.output_directory,
                    &declaration.board_id,
                )?)
            } else {
                None
            };
            let progression_started = Instant::now();
            let progression = if target_rung == 0 {
                None
            } else {
                Some(pcb_kicad::progress_kicad_connections(
                    &declaration,
                    &initial.output_directory,
                    &output_directory.join("solve"),
                    &pcb_kicad::KiCadConnectionProgressionConfig {
                        maximum_connections: target_rung,
                        reuse_committed_parent_verification: true,
                        insertion: insertion.clone(),
                    },
                )?)
            };
            let progression_elapsed_micros = progression_started.elapsed().as_micros() as u64;
            let final_rung = progression
                .as_ref()
                .map_or(initial.rung, |result| result.final_rung);
            let final_directory = progression.as_ref().map_or_else(
                || initial.output_directory.clone(),
                |result| result.final_directory.clone(),
            );
            let completed = final_rung == target_rung
                && initial_verification
                    .as_ref()
                    .is_none_or(|verification| verification.complete);
            let manifest = serde_json::json!({
                "schema_version": 1,
                "contract": "prune semantic connectivity to the selected prefix before placement; regenerate placement; materialize zero-copper rung 0; solve every enabled connection inside this run; never consume a previously solved smaller prefix",
                "target_rung": target_rung,
                "completed": completed,
                "source_snapshots": {
                    "source_problem": "source-problem.json",
                    "active_prefix_problem": "active-prefix-problem.json",
                    "placement_config": "placement-config.json",
                    "source_template_config": "source-template-config.json",
                    "template_config": "template-config.json",
                    "insertion_config": "insertion-config.json"
                },
                "placement": placement,
                "template": template,
                "initial": initial,
                "initial_verification": initial_verification,
                "progression": progression,
                "timing": {
                    "advisory_only": true,
                    "generation_detail": generation_timing,
                    "semantic_generation_elapsed_micros": generation_elapsed_micros,
                    "initial_materialization_elapsed_micros": initial_materialization_elapsed_micros,
                    "progression_elapsed_micros": progression_elapsed_micros,
                    "total_to_manifest_elapsed_micros": cold_started.elapsed().as_micros() as u64
                },
                "final_rung": final_rung,
                "final_directory": final_directory
            });
            write_pretty_json_file(&output_directory.join("cold-prefix.json"), &manifest)?;
            println!(
                "wrote {}: cold semantic prefix 0 -> {}, target={}, placement-policy={}, moved={}, complete={}, selected={}",
                output_directory.display(),
                final_rung,
                target_rung,
                placement.evidence.policy,
                placement.evidence.moved_components,
                completed,
                final_directory.display(),
            );
            if completed {
                Ok(())
            } else {
                Err(format!(
                    "cold semantic KiCad prefix stopped at rung {final_rung} before target {target_rung}; the exact last valid board remains selected"
                ))
            }
        }
        "solve-semantic-kicad-order-portfolio" => {
            let usage = "usage: pcb-maker solve-semantic-kicad-order-portfolio <problem.json> <placement-policy|config.json> <template-config.json> <target-rung> <order-search-config.json> <output-directory> [insertion-config.json]";
            let problem_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let placement_policy = arguments.next().ok_or_else(|| usage.to_string())?;
            let template_config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let target_rung = arguments
                .next()
                .ok_or_else(|| usage.to_string())?
                .parse::<usize>()
                .map_err(|error| format!("target-rung must be a non-negative integer: {error}"))?;
            let order_search_config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let insertion_config_path = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let insertion =
                load_kicad_connection_insertion_config(insertion_config_path.as_deref())?;
            let (selected_rung, selected_directory, completed) =
                solve_semantic_kicad_order_portfolio(
                    Path::new(&problem_path),
                    &placement_policy,
                    Path::new(&template_config_path),
                    target_rung,
                    Path::new(&order_search_config_path),
                    Path::new(&output_directory),
                    &insertion,
                )?;
            println!(
                "wrote {output_directory}: selected rung={selected_rung}/{target_rung}, complete={completed}, board={}",
                selected_directory.display()
            );
            if completed {
                Ok(())
            } else {
                Err(format!(
                    "cold connection-order portfolio stopped at rung {selected_rung} before target {target_rung}; every attempted cold lineage remains retained"
                ))
            }
        }
        "solve-semantic-kicad-placement-portfolio" => {
            let usage = "usage: pcb-maker solve-semantic-kicad-placement-portfolio <problem.json> <placement-portfolio.json> <template-config.json> <target-rung> <output-directory> [insertion-config.json]";
            let problem_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let portfolio_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let template_config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let target_rung = arguments
                .next()
                .ok_or_else(|| usage.to_string())?
                .parse::<usize>()
                .map_err(|error| format!("target-rung must be a non-negative integer: {error}"))?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let insertion_config_path = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let insertion =
                load_kicad_connection_insertion_config(insertion_config_path.as_deref())?;
            let (selected_id, final_rung, selected_directory, completed) =
                solve_semantic_kicad_placement_portfolio(
                    Path::new(&problem_path),
                    Path::new(&portfolio_path),
                    Path::new(&template_config_path),
                    target_rung,
                    Path::new(&output_directory),
                    &insertion,
                )?;
            println!(
                "wrote {output_directory}: selected placement={selected_id}, rung={final_rung}/{target_rung}, complete={completed}, board={}",
                selected_directory.display()
            );
            if completed {
                Ok(())
            } else {
                Err(format!(
                    "cold placement portfolio stopped at rung {final_rung} before target {target_rung}; the exact farthest board remains selected"
                ))
            }
        }
        "solve-semantic-kicad-prefix-feedback" => {
            let usage = "usage: pcb-maker solve-semantic-kicad-prefix-feedback <problem.json> <placement-policy|config.json> <template-config.json> <target-rung> <output-directory> <dut-routing-config.json> <pressure-config.json> [insertion-config.json]";
            let problem_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let placement_policy = arguments.next().ok_or_else(|| usage.to_string())?;
            let template_config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let target_rung = arguments
                .next()
                .ok_or_else(|| usage.to_string())?
                .parse::<usize>()
                .map_err(|error| format!("target-rung must be a non-negative integer: {error}"))?;
            let output_directory_argument = arguments.next().ok_or_else(|| usage.to_string())?;
            let routing_config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let pressure_config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let insertion_config_path = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let output_directory = Path::new(&output_directory_argument);
            if output_directory.exists() {
                return Err(format!(
                    "refusing to overwrite cold semantic KiCad feedback prefix {}",
                    output_directory.display()
                ));
            }
            let cold_started = Instant::now();
            let mut active_problem = load_layout_trace_problem(Path::new(&problem_path))?;
            let placement_config = load_initial_placement_config(&placement_policy)?;
            let template_config_source = fs::read_to_string(&template_config_path)
                .map_err(|error| format!("failed to read {template_config_path}: {error}"))?;
            let mut active_template_config: pcb_kicad::SemanticKiCadTemplateConfig =
                serde_json::from_str(&template_config_source)
                    .map_err(|error| format!("failed to parse {template_config_path}: {error}"))?;
            retain_semantic_connection_prefix(
                &mut active_problem,
                &mut active_template_config,
                target_rung,
            )?;
            let routing_config = load_grid_routing_config(Some(&routing_config_path))?;
            let pressure_config = load_pressure_repair_config(Some(&pressure_config_path))?;
            let insertion =
                load_kicad_connection_insertion_config(insertion_config_path.as_deref())?;

            let base_placement =
                pcb_placement::run_initial_placement(&active_problem, &placement_config)?;
            let base_components =
                solved_components_from_placement(&active_problem, &base_placement)?;
            let feedback_started = Instant::now();
            let feedback = pcb_coordinator::repair_routing_with_pressure(
                &active_problem,
                &base_components,
                &routing_config,
                &pressure_config,
            )?;
            let feedback_elapsed_micros = feedback_started.elapsed().as_micros() as u64;
            let selected_placement_poses = feedback
                .candidate
                .components
                .iter()
                .map(|component| pcb_placement::PlacementPose {
                    component: component.id.clone(),
                    position: component.position,
                    rotation_degrees: component.rotation_degrees,
                })
                .collect::<Vec<_>>();
            let selected_placement_demand = pcb_placement::placement_demand_evidence(
                &active_problem,
                &selected_placement_poses,
                [32, 32],
            )?;
            let selected_poses = selected_placement_poses
                .iter()
                .map(|pose| pcb_kicad::SemanticKiCadPose {
                    component: pose.component.clone(),
                    position: pose.position,
                    rotation_degrees: pose.rotation_degrees,
                })
                .collect::<Vec<_>>();

            let generated_directory = output_directory.join("generated-ladder");
            let generation_started = Instant::now();
            let template = pcb_kicad::write_semantic_kicad_ladder_template(
                &active_problem,
                &selected_poses,
                &active_template_config,
                &generated_directory,
            )?;
            let generation_elapsed_micros = generation_started.elapsed().as_micros() as u64;
            if target_rung > template.electrical_connections {
                return Err(format!(
                    "target rung {target_rung} exceeds the generated ladder's {} connections",
                    template.electrical_connections
                ));
            }

            fs::copy(&problem_path, output_directory.join("source-problem.json"))
                .map_err(|error| format!("failed to snapshot {problem_path}: {error}"))?;
            fs::copy(
                &template_config_path,
                output_directory.join("source-template-config.json"),
            )
            .map_err(|error| format!("failed to snapshot {template_config_path}: {error}"))?;
            write_pretty_json_file(
                &output_directory.join("active-prefix-problem.json"),
                &active_problem,
            )?;
            write_pretty_json_file(
                &output_directory.join("placement-config.json"),
                &placement_config,
            )?;
            write_pretty_json_file(
                &output_directory.join("base-placement.json"),
                &base_placement,
            )?;
            write_pretty_json_file(
                &output_directory.join("dut-routing-config.json"),
                &routing_config,
            )?;
            write_pretty_json_file(
                &output_directory.join("pressure-config.json"),
                &pressure_config,
            )?;
            write_pretty_json_file(&output_directory.join("placement-feedback.json"), &feedback)?;
            write_pretty_json_file(
                &output_directory.join("selected-feedback-poses.json"),
                &selected_placement_poses,
            )?;
            write_pretty_json_file(
                &output_directory.join("selected-placement-demand.json"),
                &selected_placement_demand,
            )?;
            write_pretty_json_file(
                &output_directory.join("template-config.json"),
                &active_template_config,
            )?;
            write_pretty_json_file(&output_directory.join("insertion-config.json"), &insertion)?;

            let declaration = pcb_kicad::load_declaration(&template.declaration)?;
            let materialization_started = Instant::now();
            let initial =
                pcb_kicad::materialize_rung(&declaration, 0, &output_directory.join("initial"))?;
            let initial_materialization_elapsed_micros =
                materialization_started.elapsed().as_micros() as u64;
            let progression_started = Instant::now();
            let progression = if target_rung == 0 {
                None
            } else {
                Some(pcb_kicad::progress_kicad_connections(
                    &declaration,
                    &initial.output_directory,
                    &output_directory.join("solve"),
                    &pcb_kicad::KiCadConnectionProgressionConfig {
                        maximum_connections: target_rung,
                        reuse_committed_parent_verification: true,
                        insertion: insertion.clone(),
                    },
                )?)
            };
            let progression_elapsed_micros = progression_started.elapsed().as_micros() as u64;
            let final_rung = progression
                .as_ref()
                .map_or(initial.rung, |result| result.final_rung);
            let final_directory = progression.as_ref().map_or_else(
                || initial.output_directory.clone(),
                |result| result.final_directory.clone(),
            );
            let completed = final_rung == target_rung;
            let base_components_by_id = base_components
                .iter()
                .map(|component| (component.id.as_str(), component))
                .collect::<std::collections::HashMap<_, _>>();
            let feedback_moved_components = feedback
                .candidate
                .components
                .iter()
                .filter(|selected| {
                    base_components_by_id
                        .get(selected.id.as_str())
                        .is_some_and(|base| {
                            (selected.position.x - base.position.x).abs() > 1.0e-9
                                || (selected.position.y - base.position.y).abs() > 1.0e-9
                                || (selected.rotation_degrees - base.rotation_degrees).abs()
                                    > 1.0e-9
                        })
                })
                .count();

            // Semantic feedback is a proposal, not a commit. If its selected
            // placement cannot survive the complete cold native progression,
            // independently regenerate the unmodified base placement from
            // zero copper inside this run. Keep both trials and select the
            // fallback transactionally; never continue from the feedback
            // trial's last valid partial board.
            let fallback_started = Instant::now();
            let fallback = if completed {
                None
            } else {
                let fallback_root = output_directory.join("base-placement-fallback");
                let fallback_poses = base_placement
                    .poses
                    .iter()
                    .map(|pose| pcb_kicad::SemanticKiCadPose {
                        component: pose.component.clone(),
                        position: pose.position,
                        rotation_degrees: pose.rotation_degrees,
                    })
                    .collect::<Vec<_>>();
                let generation_started = Instant::now();
                let fallback_template = pcb_kicad::write_semantic_kicad_ladder_template(
                    &active_problem,
                    &fallback_poses,
                    &active_template_config,
                    &fallback_root.join("generated-ladder"),
                )?;
                let generation_elapsed_micros = generation_started.elapsed().as_micros() as u64;
                let fallback_declaration =
                    pcb_kicad::load_declaration(&fallback_template.declaration)?;
                let materialization_started = Instant::now();
                let fallback_initial = pcb_kicad::materialize_rung(
                    &fallback_declaration,
                    0,
                    &fallback_root.join("initial"),
                )?;
                let materialization_elapsed_micros =
                    materialization_started.elapsed().as_micros() as u64;
                let progression_started = Instant::now();
                let fallback_progression = if target_rung == 0 {
                    None
                } else {
                    Some(pcb_kicad::progress_kicad_connections(
                        &fallback_declaration,
                        &fallback_initial.output_directory,
                        &fallback_root.join("solve"),
                        &pcb_kicad::KiCadConnectionProgressionConfig {
                            maximum_connections: target_rung,
                            reuse_committed_parent_verification: true,
                            insertion: insertion.clone(),
                        },
                    )?)
                };
                let progression_elapsed_micros = progression_started.elapsed().as_micros() as u64;
                let fallback_final_rung = fallback_progression
                    .as_ref()
                    .map_or(fallback_initial.rung, |result| result.final_rung);
                let fallback_final_directory = fallback_progression.as_ref().map_or_else(
                    || fallback_initial.output_directory.clone(),
                    |result| result.final_directory.clone(),
                );
                Some((
                    fallback_template,
                    fallback_initial,
                    fallback_progression,
                    fallback_final_rung,
                    fallback_final_directory,
                    generation_elapsed_micros,
                    materialization_elapsed_micros,
                    progression_elapsed_micros,
                ))
            };
            let fallback_elapsed_micros = fallback_started.elapsed().as_micros() as u64;
            let fallback_selected = select_base_placement_fallback(
                final_rung,
                fallback
                    .as_ref()
                    .map(|(_, _, _, fallback_rung, _, _, _, _)| *fallback_rung),
            );
            let selected_variant = if fallback_selected {
                "base_placement_fallback"
            } else {
                "feedback_placement"
            };
            let selected_template = fallback
                .as_ref()
                .filter(|_| fallback_selected)
                .map_or(&template, |(template, _, _, _, _, _, _, _)| template);
            let selected_initial = fallback
                .as_ref()
                .filter(|_| fallback_selected)
                .map_or(&initial, |(_, initial, _, _, _, _, _, _)| initial);
            let selected_progression = fallback.as_ref().filter(|_| fallback_selected).map_or(
                progression.as_ref(),
                |(_, _, progression, _, _, _, _, _)| progression.as_ref(),
            );
            let selected_final_rung = fallback
                .as_ref()
                .filter(|_| fallback_selected)
                .map_or(final_rung, |(_, _, _, rung, _, _, _, _)| *rung);
            let selected_final_directory = fallback
                .as_ref()
                .filter(|_| fallback_selected)
                .map_or(&final_directory, |(_, _, _, _, directory, _, _, _)| {
                    directory
                });
            let selected_completed = selected_final_rung == target_rung;
            let fallback_manifest = fallback.as_ref().map(
                |(
                    template,
                    initial,
                    progression,
                    final_rung,
                    final_directory,
                    generation_elapsed_micros,
                    materialization_elapsed_micros,
                    progression_elapsed_micros,
                )| {
                    serde_json::json!({
                        "attempted": true,
                        "completed": *final_rung == target_rung,
                        "selected": fallback_selected,
                        "selection_reason": if fallback_selected {
                            "feedback placement failed before the target; the independently regenerated base placement reached at least as far and is the exact transactional fallback"
                        } else {
                            "feedback placement remained the farther native progression"
                        },
                        "template": template,
                        "initial": initial,
                        "progression": progression,
                        "final_rung": final_rung,
                        "final_directory": final_directory,
                        "timing": {
                            "semantic_generation_elapsed_micros": generation_elapsed_micros,
                            "initial_materialization_elapsed_micros": materialization_elapsed_micros,
                            "progression_elapsed_micros": progression_elapsed_micros
                        }
                    })
                },
            );
            let manifest = serde_json::json!({
                "schema_version": 2,
                "contract": "prune semantic connectivity before base placement; run bounded tracer-to-placer pressure feedback; discard every semantic route; treat the feedback placement as provisional until a zero-copper native progression reaches the target; on failure independently regenerate the base placement and all routes inside this run; select the exact transactional fallback rather than a partial feedback board; never consume a previously solved smaller prefix",
                "target_rung": target_rung,
                "completed": selected_completed,
                "selected_variant": selected_variant,
                "source_snapshots": {
                    "source_problem": "source-problem.json",
                    "active_prefix_problem": "active-prefix-problem.json",
                    "placement_config": "placement-config.json",
                    "base_placement": "base-placement.json",
                    "dut_routing_config": "dut-routing-config.json",
                    "pressure_config": "pressure-config.json",
                    "placement_feedback": "placement-feedback.json",
                    "selected_feedback_poses": "selected-feedback-poses.json",
                    "selected_placement_demand": "selected-placement-demand.json",
                    "source_template_config": "source-template-config.json",
                    "template_config": "template-config.json",
                    "insertion_config": "insertion-config.json"
                },
                "base_placement": base_placement,
                "feedback": {
                    "strategy": feedback.evidence.strategy,
                    "semantic_complete": feedback.evidence.complete,
                    "selected_attempt": feedback.evidence.selected_attempt,
                    "attempted_repairs": feedback.evidence.attempted_repairs,
                    "routed_branches": feedback.evidence.routing.routed_branches,
                    "failed_branches": feedback.evidence.routing.failed_branches,
                    "moved_components": feedback_moved_components,
                    "selected_placement_demand": selected_placement_demand,
                    "evidence": "placement-feedback.json"
                },
                "feedback_native_trial": {
                    "completed": completed,
                    "selected": !fallback_selected,
                    "template": template,
                    "initial": initial,
                    "progression": progression,
                    "final_rung": final_rung,
                    "final_directory": final_directory
                },
                "base_placement_fallback": fallback_manifest,
                "template": selected_template,
                "initial": selected_initial,
                "progression": selected_progression,
                "timing": {
                    "advisory_only": true,
                    "feedback_elapsed_micros": feedback_elapsed_micros,
                    "semantic_generation_elapsed_micros": generation_elapsed_micros,
                    "initial_materialization_elapsed_micros": initial_materialization_elapsed_micros,
                    "progression_elapsed_micros": progression_elapsed_micros,
                    "fallback_elapsed_micros": fallback_elapsed_micros,
                    "total_to_manifest_elapsed_micros": cold_started.elapsed().as_micros() as u64
                },
                "final_rung": selected_final_rung,
                "final_directory": selected_final_directory
            });
            write_pretty_json_file(
                &output_directory.join("cold-feedback-prefix.json"),
                &manifest,
            )?;
            println!(
                "wrote {}: tracer feedback {}/{} branches, moved={}, feedback-native-rung={}, selected-variant={}, cold KiCad prefix 0 -> {}, target={}, complete={}, selected={}",
                output_directory.display(),
                feedback.evidence.routing.routed_branches,
                feedback.evidence.routing.branch_count,
                feedback_moved_components,
                final_rung,
                selected_variant,
                selected_final_rung,
                target_rung,
                selected_completed,
                selected_final_directory.display(),
            );
            if selected_completed {
                Ok(())
            } else {
                Err(format!(
                    "cold semantic KiCad feedback prefix stopped at rung {selected_final_rung} before target {target_rung}; the farthest exact native trial remains selected"
                ))
            }
        }
        "materialize-kicad-rung" => {
            let declaration_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker materialize-kicad-rung <declaration.json> <rung> <output-directory>"
                    .to_string()
            })?;
            let rung = arguments
                .next()
                .ok_or_else(|| {
                    "usage: pcb-maker materialize-kicad-rung <declaration.json> <rung> <output-directory>"
                        .to_string()
                })?
                .parse::<usize>()
                .map_err(|error| format!("rung must be a non-negative integer: {error}"))?;
            let output = arguments.next().ok_or_else(|| {
                "usage: pcb-maker materialize-kicad-rung <declaration.json> <rung> <output-directory>"
                    .to_string()
            })?;
            if arguments.next().is_some() {
                return Err("usage: pcb-maker materialize-kicad-rung <declaration.json> <rung> <output-directory>".into());
            }
            let declaration = pcb_kicad::load_declaration(Path::new(&declaration_path))?;
            let report = pcb_kicad::materialize_rung(&declaration, rung, Path::new(&output))?;
            println!(
                "wrote {}: {} selected connection(s), {} schematic wire(s), {} no-connect marker(s), {} segment(s), {} arc(s), {} via(s), {} zone(s)",
                report.output_directory.display(),
                report.selected_connections.len(),
                report.schematic_wires,
                report.schematic_no_connects,
                report.pcb_segments,
                report.pcb_arcs,
                report.pcb_vias,
                report.pcb_zones
            );
            Ok(())
        }
        "insert-kicad-connection" => {
            let usage = "usage: pcb-maker insert-kicad-connection <declaration.json> <parent-rung-directory> <transaction-directory> [config.json]";
            let declaration_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let parent_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let config = match arguments.next() {
                Some(path) => {
                    let source = fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?;
                    serde_json::from_str(&source)
                        .map_err(|error| format!("failed to parse {path}: {error}"))?
                }
                None => pcb_kicad::KiCadConnectionInsertionConfig::default(),
            };
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let declaration = pcb_kicad::load_declaration(Path::new(&declaration_path))?;
            let result = pcb_kicad::insert_kicad_connection(
                &declaration,
                Path::new(&parent_directory),
                Path::new(&output_directory),
                &config,
            )?;
            println!(
                "wrote {output_directory}: {:?}, {} -> {}, connection={}, attempts={}, selected={}",
                result.disposition,
                result.evidence.source_rung,
                result.evidence.target_rung,
                result.evidence.inserted_connection,
                result.evidence.attempts.len(),
                result.selected_directory.display(),
            );
            if result.disposition == pcb_kicad::KiCadConnectionInsertionDisposition::Committed {
                Ok(())
            } else {
                Err("KiCad connection insertion exhausted its bounded portfolio; exact parent rollback was selected".into())
            }
        }
        "progress-kicad-connections" => {
            let usage = "usage: pcb-maker progress-kicad-connections <declaration.json> <initial-parent-directory> <progression-directory> [config.json]";
            let declaration_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let parent_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let config = match arguments.next() {
                Some(path) => {
                    let source = fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?;
                    serde_json::from_str(&source)
                        .map_err(|error| format!("failed to parse {path}: {error}"))?
                }
                None => pcb_kicad::KiCadConnectionProgressionConfig::default(),
            };
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let declaration = pcb_kicad::load_declaration(Path::new(&declaration_path))?;
            let result = pcb_kicad::progress_kicad_connections(
                &declaration,
                Path::new(&parent_directory),
                Path::new(&output_directory),
                &config,
            )?;
            println!(
                "wrote {output_directory}: {:?}, rung {} -> {}, committed={}, transactions={}, route-expansions={}, local-portfolio={}/{}, local-candidates={}, local-native-gates={}, selected={}",
                result.termination,
                result.initial_rung,
                result.final_rung,
                result.committed_connections,
                result.attempted_transactions,
                result.total_route_expansions,
                result.total_local_portfolio_entries_evaluated,
                result.total_local_portfolio_entries_available,
                result.total_local_candidates_generated,
                result.total_local_native_gates,
                result.final_directory.display(),
            );
            if matches!(
                result.termination,
                pcb_kicad::KiCadConnectionProgressionTermination::BoundReached
                    | pcb_kicad::KiCadConnectionProgressionTermination::FinalRungReached
            ) {
                Ok(())
            } else {
                Err(format!(
                    "KiCad connection progression stopped at rung {} with {:?}; the exact last valid board remains selected",
                    result.final_rung, result.termination
                ))
            }
        }
        "verify-kicad-rung" => {
            let directory = arguments.next().ok_or_else(|| {
                "usage: pcb-maker verify-kicad-rung <directory> <board-id>".to_string()
            })?;
            let board_id = arguments.next().ok_or_else(|| {
                "usage: pcb-maker verify-kicad-rung <directory> <board-id>".to_string()
            })?;
            if arguments.next().is_some() {
                return Err("usage: pcb-maker verify-kicad-rung <directory> <board-id>".into());
            }
            let report = pcb_kicad::verify_materialized_rung(Path::new(&directory), &board_id)?;
            println!(
                "{}: ERC={}, DRC={}, parity={}, selected-net-unconnected={}, intentional-no-connect-groups={}, library-metadata-warnings={}",
                if report.complete {
                    "complete"
                } else {
                    "incomplete"
                },
                report.erc_violations,
                report.drc_design_violations,
                report.schematic_parity_issues,
                report.selected_net_unconnected_items,
                report.intentional_no_connect_groups,
                report.library_metadata_warnings
            );
            if report.complete {
                Ok(())
            } else {
                Err("KiCad rung did not pass the completion policy".into())
            }
        }
        "repair-kicad-silkscreen" => {
            let usage = "usage: pcb-maker repair-kicad-silkscreen <source-directory> <board-id> <output-directory>";
            let source = arguments.next().ok_or(usage)?;
            let board_id = arguments.next().ok_or(usage)?;
            let output = arguments.next().ok_or(usage)?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report = pcb_kicad::repair_kicad_silkscreen(
                Path::new(&source),
                &board_id,
                Path::new(&output),
            )?;
            println!(
                "wrote {output}: silkscreen-complete={}, remaining-annotation-findings={}",
                report["silkscreen_complete"], report["remaining_annotation_findings"]
            );
            if report["silkscreen_complete"] == true {
                Ok(())
            } else {
                Err(
                    "silkscreen repair remains incomplete; retained native report and rendering"
                        .into(),
                )
            }
        }
        "normalize-kicad-footprint-links" => {
            let usage = "usage: pcb-maker normalize-kicad-footprint-links <directory> <board-id> [report.json]";
            let directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let report_path = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report =
                pcb_kicad::normalize_kicad_footprint_links(Path::new(&directory), &board_id)?;
            let serialized =
                serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
            if let Some(report_path) = report_path {
                fs::write(&report_path, format!("{serialized}\n"))
                    .map_err(|error| format!("failed to write {report_path}: {error}"))?;
                println!(
                    "wrote {report_path}: normalized {} footprint link(s)",
                    report.updated_references.len()
                );
            } else {
                println!("{serialized}");
            }
            Ok(())
        }
        "strip-kicad-copper" => {
            let usage = "usage: pcb-maker strip-kicad-copper <source.kicad_pcb> <output.kicad_pcb> [report.json]";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let report_path = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report = pcb_kicad::write_kicad_board_without_copper(
                Path::new(&source),
                Path::new(&output),
            )?;
            let serialized =
                serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
            if let Some(report_path) = report_path {
                fs::write(&report_path, format!("{serialized}\n"))
                    .map_err(|error| format!("failed to write {report_path}: {error}"))?;
                println!(
                    "wrote {output} and {report_path}: removed {} tracks/arcs, {} vias, {} copper zones, and {} copper graphics",
                    report.removed_segments + report.removed_arcs,
                    report.removed_vias,
                    report.removed_copper_zones,
                    report.removed_copper_graphics,
                );
            } else {
                println!("{serialized}");
            }
            Ok(())
        }
        "inspect-kicad-board" => {
            let usage = "usage: pcb-maker inspect-kicad-board <board.kicad_pcb>";
            let board = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let statistics = pcb_kicad::inspect_kicad_board(Path::new(&board))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&statistics).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        "inspect-kicad-routing-cut" => {
            let usage = "usage: pcb-maker inspect-kicad-routing-cut <board.kicad_pcb> <connection> <report.json> [config.json]";
            let board = arguments.next().ok_or(usage)?;
            let connection = arguments.next().ok_or(usage)?;
            let output = arguments.next().ok_or(usage)?;
            let config = if let Some(path) = arguments.next() {
                serde_json::from_str(&fs::read_to_string(&path).map_err(|error| error.to_string())?)
                    .map_err(|error| error.to_string())?
            } else { pcb_kicad::KiCadRoutingCutConfig::default() };
            if arguments.next().is_some() { return Err(usage.into()); }
            let report = pcb_kicad::inspect_kicad_routing_cut(Path::new(&board), &connection, &config)?;
            write_serialized_result(&report, Some(&output))
        }
        "inspect-kicad-terminal-access" => {
            let usage = "usage: pcb-maker inspect-kicad-terminal-access <board.kicad_pcb> <connection> <report.json> [config.json]";
            let board = arguments.next().ok_or(usage)?;
            let connection = arguments.next().ok_or(usage)?;
            let output = arguments.next().ok_or(usage)?;
            let config = if let Some(path) = arguments.next() {
                serde_json::from_str(&fs::read_to_string(&path).map_err(|error| error.to_string())?)
                    .map_err(|error| error.to_string())?
            } else { pcb_kicad::KiCadGridRouteConfig::default() };
            if arguments.next().is_some() { return Err(usage.into()); }
            let report = pcb_kicad::inspect_kicad_terminal_access(Path::new(&board), &connection, &config)?;
            write_serialized_result(&report, Some(&output))
        }
        "relax-kicad-joint-routes" => {
            let usage = "usage: pcb-maker relax-kicad-joint-routes <source-directory> <board-id> <target-candidate.json> <config.json> <output-directory>";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let target = arguments.next().ok_or_else(|| usage.to_string())?;
            let config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() { return Err(usage.into()); }
            let config: pcb_kicad::KiCadJointRelaxationConfig = serde_json::from_str(
                &fs::read_to_string(&config_path).map_err(|error| error.to_string())?
            ).map_err(|error| error.to_string())?;
            let result = pcb_kicad::relax_kicad_joint_routes(Path::new(&source), &board_id, Path::new(&target), &config, Path::new(&output))?;
            println!("joint status={:?}; selected={}", result.status, result.selected_directory.display());
            if result.status != pcb_kicad::KiCadJointRelaxationStatus::Admitted {
                return Err(format!("joint insertion {:?}: {}", result.status, result.reason.as_deref().unwrap_or("not admitted")));
            }
            Ok(())
        }
        "inspect-kicad-route-obstructions" => {
            let usage = "usage: pcb-maker inspect-kicad-route-obstructions <board.kicad_pcb> <candidate.json> <routing-config.json> <output.json>";
            let board = arguments.next().ok_or(usage)?;
            let candidate = arguments.next().ok_or(usage)?;
            let config_path = arguments.next().ok_or(usage)?;
            let output = arguments.next().ok_or(usage)?;
            if arguments.next().is_some() { return Err(usage.into()); }
            let config: pcb_kicad::KiCadGridRouteConfig = serde_json::from_str(
                &fs::read_to_string(&config_path).map_err(|e| e.to_string())?
            ).map_err(|e| e.to_string())?;
            let report = pcb_kicad::inspect_kicad_route_obstructions(
                Path::new(&board), Path::new(&candidate), &config
            )?;
            write_serialized_result(&report, Some(&output))
        }
        "inspect-kicad-connections" => {
            let usage = "usage: pcb-maker inspect-kicad-connections <board.kicad_pcb>";
            let board = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let connections = pcb_kicad::inspect_kicad_routable_connections(Path::new(&board))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&connections).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        "layout-kicad-board" => {
            let usage = "usage: pcb-maker layout-kicad-board <source-directory> <board-id> <output-directory> [router-config.json|auto] [layout-config.json]";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let router = board_router_config(arguments.next().as_deref(), &source, &board_id)?;
            let config: pcb_kicad::KiCadBoardLayoutConfig = match arguments.next() {
                Some(path) => serde_json::from_str(
                    &std::fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?,
                )
                .map_err(|error| format!("failed to parse {path}: {error}"))?,
                None => Default::default(),
            };
            let result = pcb_kicad::layout_kicad_board(
                Path::new(&source),
                &board_id,
                Path::new(&output),
                &config,
                &router,
            )?;
            for round in &result.rounds {
                println!(
                    "round {}: wirelength {:.0} mm, routed {} ({} unconnected), {} vias, {:.0} mm, place {:.1}s route {:.1}s, congested {:?}",
                    round.round,
                    round.wirelength_mm,
                    round.routed_connections,
                    round.unconnected_terminals,
                    round.vias,
                    round.length_mm,
                    round.placement_seconds,
                    round.routing_seconds,
                    round
                        .most_congested
                        .iter()
                        .take(3)
                        .map(|(reference, _)| reference.as_str())
                        .collect::<Vec<_>>(),
                );
            }
            println!(
                "selected round {}: routed {}/{} connections, {} vias, {:.1} mm, native complete={}",
                result.selected_round,
                result.routed.routed_connections,
                result.routed.routable_connections,
                result.routed.vias,
                result.routed.length_mm,
                result.routed.native.as_ref().map(|native| native.complete).unwrap_or(false),
            );
            if result.routed.unconnected_terminals > 0 || !result.routed.internal_violations.is_empty() {
                return Err("layout is not electrically complete".into());
            }
            Ok(())
        }
        "place-kicad-board" => {
            let usage = "usage: pcb-maker place-kicad-board <source-directory> <board-id> <output-directory> [config.json]";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let config: pcb_kicad::KiCadBoardPlacerConfig = match arguments.next() {
                Some(path) => serde_json::from_str(
                    &std::fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?,
                )
                .map_err(|error| format!("failed to parse {path}: {error}"))?,
                None => Default::default(),
            };
            let result = pcb_kicad::place_kicad_board(
                Path::new(&source),
                &board_id,
                Path::new(&output),
                &config,
            )?;
            println!(
                "placed {} of {} components ({} nets): wirelength source {:.0} mm, global {:.0}, legal {:.0}, final {:.0}; {} global iterations, overflow {:.3}; unplaced {:?}, illegal {:?}; {:.2}s",
                result.movable,
                result.components,
                result.nets,
                result.wirelength_source_mm,
                result.wirelength_global_mm,
                result.wirelength_legal_mm,
                result.wirelength_final_mm,
                result.global_iterations,
                result.global_overflow,
                result.unplaced,
                result.illegal,
                result.seconds,
            );
            if !result.unplaced.is_empty() || !result.illegal.is_empty() {
                return Err("placement is not legal".into());
            }
            Ok(())
        }
        "route-kicad-board" => {
            let usage = "usage: pcb-maker route-kicad-board <source-directory> <board-id> <output-directory> [config.json|auto]";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let config = board_router_config(arguments.next().as_deref(), &source, &board_id)?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let result = pcb_kicad::route_kicad_board(
                Path::new(&source),
                &board_id,
                Path::new(&output),
                &config,
            )?;
            println!(
                "routed {}/{} connections ({} unconnected terminals), {} vias, {:.1} mm, {} iterations; lowering {:.2}s routing {:.2}s internal check {:.2}s ({} violations) native {:.2}s complete={}",
                result.routed_connections,
                result.routable_connections,
                result.unconnected_terminals,
                result.vias,
                result.length_mm,
                result.iterations,
                result.lowering_seconds,
                result.routing_seconds,
                result.internal_verification_seconds,
                result.internal_violations.len(),
                result.native_verification_seconds,
                result.native.as_ref().map(|native| native.complete).unwrap_or(false),
            );
            if !result.internal_violations.is_empty() {
                return Err(format!(
                    "{} internal clearance violation(s); see board-router.json",
                    result.internal_violations.len()
                ));
            }
            if result.native.as_ref().is_some_and(|native| !native.complete) {
                return Err("native KiCad verification is not complete".into());
            }
            Ok(())
        }
        "route-kicad-board-freerouting" => {
            let usage = "usage: pcb-maker route-kicad-board-freerouting <source-directory> <board-id> <output-directory> <config.json>";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let config = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            freerouting_backend::run(
                Path::new(&source),
                &board_id,
                Path::new(&output),
                Path::new(&config),
            )
        }
        "route-kicad-board-adaptive" => {
            let usage = "usage: pcb-maker route-kicad-board-adaptive <source-directory> <board-id> <output-directory> <config.json>";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let config: pcb_kicad::KiCadAdaptiveRoutingConfig =
                serde_json::from_str(&fs::read_to_string(&config_path).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("failed to parse {config_path}: {e}"))?;
            let result = pcb_kicad::route_kicad_board_adaptively(
                Path::new(&source),
                &board_id,
                Path::new(&output),
                &config,
            )?;
            println!(
                "wrote {output}: complete={}, routing_complete={}, annotations={}, passes={}, selected={:?}, vias={}, track_mm={:.3}",
                result.complete,
                result.routing_complete,
                result.outstanding_annotation_findings,
                result.passes.len(),
                result.selected_pass,
                result.result_statistics.vias,
                result
                    .result_statistics
                    .physical_copper
                    .physical_centerline_length_mm
            );
            if result.complete {
                Ok(())
            } else if result.routing_complete {
                Err("routing connects every net, but outstanding native findings prevent complete-layout admission".into())
            } else {
                Err(
                    "adaptive routing exhausted its bounded proposals without a complete board"
                        .into(),
                )
            }
        }
        "resume-kicad-board-sequential" => {
            let usage = "usage: pcb-maker resume-kicad-board-sequential <checkpoint-directory> <output-directory> <resume-config.json>";
            let checkpoint = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let config: pcb_kicad::KiCadSequentialResumeConfig =
                serde_json::from_str(&fs::read_to_string(&config_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let result = pcb_kicad::resume_kicad_board_sequentially(
                Path::new(&checkpoint),
                Path::new(&output),
                &config,
            )?;
            println!(
                "wrote {output}: resumed={}, routed={}/{}, termination={:?}",
                result
                    .resume
                    .as_ref()
                    .map_or(0, |r| r.initial_completed_connections),
                result.completed_connections,
                result.connection_order.len(),
                result.termination
            );
            if result.termination == pcb_kicad::KiCadSequentialRouterTermination::Complete {
                Ok(())
            } else {
                Err("resumed routing stopped before all connections were completed".into())
            }
        }
        "route-kicad-board-sequential" => {
            let usage = "usage: pcb-maker route-kicad-board-sequential <source-directory> <board-id> <output-directory> <config.json>";
            let source_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let config_source = fs::read_to_string(&config_path)
                .map_err(|error| format!("failed to read {config_path}: {error}"))?;
            let config: pcb_kicad::KiCadSequentialRouterConfig =
                serde_json::from_str(&config_source)
                    .map_err(|error| format!("failed to parse {config_path}: {error}"))?;
            let result = pcb_kicad::route_kicad_board_sequentially(
                Path::new(&source_directory),
                &board_id,
                Path::new(&output_directory),
                &config,
            )?;
            println!(
                "wrote {output_directory}: termination={:?}, routed={}/{}, expansions={}",
                result.termination,
                result.completed_connections,
                result.connection_order.len(),
                result.total_expansions
            );
            if result.termination == pcb_kicad::KiCadSequentialRouterTermination::Complete {
                Ok(())
            } else {
                Err(format!(
                    "sequential KiCad routing stopped after {}/{} connections with {:?}",
                    result.completed_connections,
                    result.connection_order.len(),
                    result.termination
                ))
            }
        }
        "search-kicad-board-sequential-orders" => {
            let usage = "usage: pcb-maker search-kicad-board-sequential-orders <source-directory> <board-id> <output-directory> <config.json>";
            let source_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let config_source = fs::read_to_string(&config_path)
                .map_err(|error| format!("failed to read {config_path}: {error}"))?;
            let config: pcb_kicad::KiCadSequentialOrderSearchConfig =
                serde_json::from_str(&config_source)
                    .map_err(|error| format!("failed to parse {config_path}: {error}"))?;
            let result = pcb_kicad::search_kicad_sequential_orders(
                Path::new(&source_directory),
                &board_id,
                Path::new(&output_directory),
                &config,
            )?;
            let selected = &result.trials[result.selected_trial];
            println!(
                "wrote {output_directory}: trials={}, selected={}, routed={}/{}, remaining={}, complete={}",
                result.trials.len(),
                result.selected_trial,
                selected.completed_connections,
                selected.connection_order.len(),
                selected.remaining_unconnected_items,
                result.complete
            );
            if result.complete {
                Ok(())
            } else {
                Err(format!(
                    "sequential KiCad order search stopped after {} trial(s); best trial routed {}/{} connections",
                    result.trials.len(),
                    selected.completed_connections,
                    selected.connection_order.len()
                ))
            }
        }
        "bridge-kicad-open-connections" => {
            let usage = "usage: pcb-maker bridge-kicad-open-connections <source-directory> <board-id> <config.json> <output-directory>";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board = arguments.next().ok_or_else(|| usage.to_string())?;
            let request = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() { return Err(usage.into()); }
            let config = serde_json::from_str(&fs::read_to_string(&request).map_err(|e| e.to_string())?)
                .map_err(|e| format!("invalid island bridge request: {e}"))?;
            let result = pcb_kicad::bridge_kicad_open_connections(Path::new(&source), &board, &config, Path::new(&output))?;
            println!("{}: {} bridges admitted, {} opens; selected {}", result.status, result.admitted_bridges,
                result.selected_native.selected_net_unconnected_items, result.selected_directory.display());
            if result.status == "complete" && result.source_unchanged && result.selected_native.complete {
                Ok(())
            } else {
                Err(result.reason.unwrap_or_else(|| "native island bridge operation remains incomplete".into()))
            }
        }
        "propose-kicad-pad-pair-route" => {
            let usage = "usage: pcb-maker propose-kicad-pad-pair-route <board.kicad_pcb> <request.json> <output-directory>";
            let board = arguments.next().ok_or_else(|| usage.to_string())?;
            let request = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() { return Err(usage.into()); }
            let config = serde_json::from_str(&fs::read_to_string(&request).map_err(|e| e.to_string())?)
                .map_err(|e| format!("invalid pad-pair request: {e}"))?;
            let result = pcb_kicad::propose_kicad_pad_pair_route(Path::new(&board), &config, Path::new(&output))?;
            if result.route_found && result.source_unchanged {
                println!("wrote {output}: provisional bridge found; native admission required");
                Ok(())
            } else {
                Err(result.error.unwrap_or_else(|| "pad-pair proposal failed or source changed".into()))
            }
        }
        "route-kicad-connection" => {
            let pcb_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker route-kicad-connection <board.kicad_pcb> <connection> <candidate.json> [config.json]"
                    .to_string()
            })?;
            let connection = arguments.next().ok_or_else(|| {
                "usage: pcb-maker route-kicad-connection <board.kicad_pcb> <connection> <candidate.json> [config.json]"
                    .to_string()
            })?;
            let output = arguments.next().ok_or_else(|| {
                "usage: pcb-maker route-kicad-connection <board.kicad_pcb> <connection> <candidate.json> [config.json]"
                    .to_string()
            })?;
            let config = match arguments.next() {
                Some(path) => {
                    let source = fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?;
                    serde_json::from_str(&source)
                        .map_err(|error| format!("failed to parse {path}: {error}"))?
                }
                None => pcb_kicad::KiCadGridRouteConfig::default(),
            };
            if arguments.next().is_some() {
                return Err("usage: pcb-maker route-kicad-connection <board.kicad_pcb> <connection> <candidate.json> [config.json]".into());
            }
            let candidate = pcb_kicad::route_materialized_connection(
                Path::new(&pcb_path),
                &connection,
                &config,
            )?;
            if let Some(parent) = Path::new(&output)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(
                &output,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&candidate).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {output}: {error}"))?;
            println!(
                "wrote {output}: {} segment(s), {} via(s), cost={}, expansions={}",
                candidate.supplemental_segments.len(),
                candidate.supplemental_vias.len(),
                candidate.cost,
                candidate.expansions
            );
            Ok(())
        }
        "diagnose-kicad-yielding-connections" => {
            let usage = "usage: pcb-maker diagnose-kicad-yielding-connections <board.kicad_pcb> <target-connection> <evidence.json> [config.json]";
            let pcb_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let connection = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let config = match arguments.next() {
                Some(path) => {
                    let source = fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?;
                    serde_json::from_str(&source)
                        .map_err(|error| format!("failed to parse {path}: {error}"))?
                }
                None => pcb_kicad::KiCadYieldingConnectionDiagnosisConfig::default(),
            };
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let diagnosis = pcb_kicad::diagnose_kicad_yielding_connections(
                Path::new(&pcb_path),
                &connection,
                &config,
            )?;
            if let Some(parent) = Path::new(&output)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(
                &output,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&diagnosis).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {output}: {error}"))?;
            println!(
                "wrote {output}: base-route={}, foreign-connections={}, trials={}, successful-counterfactuals={}, truncated={}",
                diagnosis.base_route_found,
                diagnosis.foreign_routed_connections.len(),
                diagnosis.trials.len(),
                diagnosis.successful_counterfactuals,
                diagnosis.truncated,
            );
            Ok(())
        }
        "repair-kicad-single-connection-ripup" => {
            let usage = "usage: pcb-maker repair-kicad-single-connection-ripup <unrouted-target-directory> <board-id> <target-connection> <output-directory> [config.json]";
            let source_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let connection = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let config = match arguments.next() {
                Some(path) => {
                    let source = fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?;
                    serde_json::from_str(&source)
                        .map_err(|error| format!("failed to parse {path}: {error}"))?
                }
                None => pcb_kicad::KiCadSingleConnectionRipupConfig::default(),
            };
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let result = pcb_kicad::repair_kicad_with_single_connection_ripup(
                Path::new(&source_directory),
                &board_id,
                &connection,
                Path::new(&output_directory),
                &config,
            )?;
            println!(
                "wrote {output_directory}: diagnosis-successes={}, repair-attempts={}, selected={}, native-complete={}",
                result.diagnosis.successful_counterfactuals,
                result.attempts.len(),
                result
                    .selected_directory
                    .as_ref()
                    .map_or_else(|| "none".into(), |path| path.display().to_string()),
                result
                    .selected_attempt
                    .and_then(|i| result.attempts[i].final_verification.as_ref())
                    .is_some_and(|v| v.complete),
            );
            if result.selected_attempt.is_some() {
                Ok(())
            } else {
                Err(
                    "no single-connection rip-up action met the requested native admission gate"
                        .into(),
                )
            }
        }
        "import-kicad-route-candidate" => {
            let usage = "usage: pcb-maker import-kicad-route-candidate <board.kicad_pcb> <connection> <candidate.json>";
            let pcb_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let connection = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let candidate =
                pcb_kicad::import_route_candidate_from_pcb(Path::new(&pcb_path), &connection)?;
            if let Some(parent) = Path::new(&output)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(
                &output,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&candidate).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {output}: {error}"))?;
            println!(
                "wrote {output}: imported {} segment(s), {} via(s)",
                candidate.supplemental_segments.len(),
                candidate.supplemental_vias.len()
            );
            Ok(())
        }
        "apply-kicad-route-candidate" => {
            let pcb_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker apply-kicad-route-candidate <board.kicad_pcb> <candidate.json> <output.kicad_pcb> [yielding-connections.json]"
                    .to_string()
            })?;
            let candidate_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker apply-kicad-route-candidate <board.kicad_pcb> <candidate.json> <output.kicad_pcb> [yielding-connections.json]"
                    .to_string()
            })?;
            let output = arguments.next().ok_or_else(|| {
                "usage: pcb-maker apply-kicad-route-candidate <board.kicad_pcb> <candidate.json> <output.kicad_pcb> [yielding-connections.json]"
                    .to_string()
            })?;
            let yielding_path = arguments.next();
            if arguments.next().is_some() {
                return Err("usage: pcb-maker apply-kicad-route-candidate <board.kicad_pcb> <candidate.json> <output.kicad_pcb> [yielding-connections.json]".into());
            }
            let yielding: Vec<String> = if let Some(path) = yielding_path {
                serde_json::from_str(&fs::read_to_string(path).map_err(|error| error.to_string())?)
                    .map_err(|error| error.to_string())?
            } else {
                Vec::new()
            };
            pcb_kicad::apply_route_candidate_with_yielding_connections(
                Path::new(&pcb_path),
                Path::new(&candidate_path),
                &yielding,
                Path::new(&output),
            )?;
            println!("wrote {output}");
            Ok(())
        }
        "relax-kicad-reference-fields" => {
            let usage = "usage: pcb-maker relax-kicad-reference-fields <board.kicad_pcb> <candidate.json> <drc.json> <iteration> <output-candidate.json> <evidence.json>";
            let pcb_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let candidate_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let drc_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let iteration = arguments
                .next()
                .ok_or_else(|| usage.to_string())?
                .parse::<usize>()
                .map_err(|error| format!("invalid reference relaxation iteration: {error}"))?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let evidence_path = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let (candidate, evidence) = pcb_kicad::relax_reference_fields_from_drc(
                Path::new(&pcb_path),
                Path::new(&candidate_path),
                Path::new(&drc_path),
                iteration,
            )?;
            for path in [&output, &evidence_path] {
                if let Some(parent) = Path::new(path)
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
            }
            fs::write(
                &output,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&candidate).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {output}: {error}"))?;
            fs::write(
                &evidence_path,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {evidence_path}: {error}"))?;
            println!(
                "wrote {output} and {evidence_path}: moved {} reference field(s) using iteration {}",
                evidence.selected_reference_fields, evidence.iteration
            );
            Ok(())
        }
        "search-kicad-reference-fields" => {
            let usage = "usage: pcb-maker search-kicad-reference-fields <source-rung-directory> <board-id> <candidate.json> <output-directory>";
            let source_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let candidate_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let output_directory = Path::new(&output_directory);
            let (selected_candidate, evidence) = pcb_kicad::search_reference_field_actions(
                Path::new(&source_directory),
                &board_id,
                Path::new(&candidate_path),
                output_directory,
            )?;
            let evidence_path = output_directory.join("portfolio.json");
            fs::write(
                &evidence_path,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {}: {error}", evidence_path.display()))?;
            if let Some(candidate) = selected_candidate {
                let selected_path = output_directory.join("selected-candidate.json");
                fs::write(
                    &selected_path,
                    format!(
                        "{}\n",
                        serde_json::to_string_pretty(&candidate)
                            .map_err(|error| error.to_string())?
                    ),
                )
                .map_err(|error| format!("failed to write {}: {error}", selected_path.display()))?;
                let selected = if evidence.selected_source {
                    "source candidate".to_string()
                } else {
                    format!("phase {:?}", evidence.selected_phase)
                };
                println!(
                    "wrote {}, {} retained action(s), {} native-complete action(s), selected {}",
                    output_directory.display(),
                    evidence.attempted_actions,
                    evidence.exact_complete_actions,
                    selected
                );
                Ok(())
            } else {
                Err(format!(
                    "no native-complete reference action; retained {} attempt(s) in {}",
                    evidence.attempted_actions,
                    output_directory.display()
                ))
            }
        }
        "discover-kicad-vias" => {
            let usage = "usage: pcb-maker discover-kicad-vias <board.kicad_pcb> <output-directory> <config.json>";
            let board = arguments.next().ok_or(usage)?;
            let output = arguments.next().ok_or(usage)?;
            let config = arguments.next().ok_or(usage)?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let config =
                serde_json::from_str(&fs::read_to_string(config).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let report = pcb_kicad::discover_kicad_via_opportunities(
                Path::new(&board),
                Path::new(&output),
                &config,
            )?;
            println!(
                "wrote {output}: {} potential via-removal actions",
                report["generated_actions"]
            );
            Ok(())
        }
        "refine-kicad-vias" => {
            let usage = "usage: pcb-maker refine-kicad-vias <source-directory> <board-id> <candidate.json> <output-directory> <config.json>";
            let source = arguments.next().ok_or(usage)?;
            let board_id = arguments.next().ok_or(usage)?;
            let candidate = arguments.next().ok_or(usage)?;
            let output = arguments.next().ok_or(usage)?;
            let config = arguments.next().ok_or(usage)?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let config =
                serde_json::from_str(&fs::read_to_string(config).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let report = pcb_kicad::refine_kicad_vias(
                Path::new(&source),
                &board_id,
                Path::new(&candidate),
                Path::new(&output),
                &config,
            )?;
            println!(
                "wrote {}: vias {} -> {}, stored track {} -> {} mm",
                output,
                report["source_statistics"]["vias"],
                report["result_statistics"]["vias"],
                report["source_statistics"]["stored_segment_length_mm"],
                report["result_statistics"]["stored_segment_length_mm"]
            );
            Ok(())
        }
        "search-kicad-via-actions" => {
            let usage = "usage: pcb-maker search-kicad-via-actions <source-rung-directory> <board-id> <candidate.json> <output-directory> <config.json>";
            let source_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let candidate_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let config_source = fs::read_to_string(&config_path)
                .map_err(|error| format!("failed to read {config_path}: {error}"))?;
            let config = serde_json::from_str(&config_source)
                .map_err(|error| format!("failed to parse {config_path}: {error}"))?;
            let output_directory = Path::new(&output_directory);
            let (selected, evidence) = pcb_kicad::search_via_topology_actions(
                Path::new(&source_directory),
                &board_id,
                Path::new(&candidate_path),
                output_directory,
                &config,
            )?;
            let portfolio_path = output_directory.join("portfolio.json");
            fs::write(
                &portfolio_path,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {}: {error}", portfolio_path.display()))?;
            if let Some(candidate) = selected {
                let selected_path = output_directory.join("selected-candidate.json");
                fs::write(
                    &selected_path,
                    format!(
                        "{}\n",
                        serde_json::to_string_pretty(&candidate)
                            .map_err(|error| error.to_string())?
                    ),
                )
                .map_err(|error| format!("failed to write {}: {error}", selected_path.display()))?;
                println!(
                    "wrote {}, {} generated action(s), {} native-complete, selected {}",
                    output_directory.display(),
                    evidence.generated_actions,
                    evidence.exact_complete_actions,
                    evidence.selected_attempt.map_or_else(
                        || "source candidate".into(),
                        |attempt| format!("attempt {attempt}")
                    )
                );
                Ok(())
            } else {
                Err(format!(
                    "no native-complete via action; retained {} attempt(s) in {}",
                    evidence.attempted_actions,
                    output_directory.display()
                ))
            }
        }
        "replace-kicad-connection-with-zones" => {
            let usage = "usage: pcb-maker replace-kicad-connection-with-zones <source-rung-directory> <board-id> <output-directory> <config.json>";
            let source_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let config_source = fs::read_to_string(&config_path)
                .map_err(|error| format!("failed to read {config_path}: {error}"))?;
            let config = serde_json::from_str(&config_source)
                .map_err(|error| format!("failed to parse {config_path}: {error}"))?;
            let output_directory = Path::new(&output_directory);
            let evidence = pcb_kicad::replace_kicad_connection_with_zones(
                Path::new(&source_directory),
                &board_id,
                output_directory,
                &config,
            )?;
            let evidence_path = output_directory.join("evidence.json");
            fs::write(
                &evidence_path,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {}: {error}", evidence_path.display()))?;
            println!(
                "wrote {}: removed {} segment(s)/{} via(s), added {} zone(s), native-complete={}",
                output_directory.display(),
                evidence.removed_segments,
                evidence.removed_vias,
                evidence.added_zones,
                evidence.complete
            );
            Ok(())
        }
        "render-kicad-layers" => {
            let usage = "usage: pcb-maker render-kicad-layers <board.kicad_pcb> <output-prefix> [config.json]";
            let pcb_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_prefix = arguments.next().ok_or_else(|| usage.to_string())?;
            let config = match arguments.next() {
                Some(path) => {
                    let source = fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?;
                    serde_json::from_str(&source)
                        .map_err(|error| format!("failed to parse {path}: {error}"))?
                }
                None => pcb_kicad::KiCadLayerRenderConfig::default(),
            };
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report = pcb_kicad::render_board_layers(
                Path::new(&pcb_path),
                Path::new(&output_prefix),
                &config,
            )?;
            println!(
                "wrote {} and {}",
                report.front.display(),
                report.back.display()
            );
            Ok(())
        }
        "measure-kicad-route-candidate" => {
            let candidate_path = arguments.next().ok_or_else(|| {
                "usage: pcb-maker measure-kicad-route-candidate <candidate.json>".to_string()
            })?;
            if arguments.next().is_some() {
                return Err(
                    "usage: pcb-maker measure-kicad-route-candidate <candidate.json>".into(),
                );
            }
            let candidate = pcb_kicad::read_route_candidate(Path::new(&candidate_path))?;
            let quality = pcb_kicad::route_candidate_quality(&candidate)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&quality).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        "shorten-kicad-route-candidate" => {
            let usage = "usage: pcb-maker shorten-kicad-route-candidate <board.kicad_pcb> <candidate.json> <output-candidate.json> <evidence.json> [config.json]";
            let pcb_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let candidate_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let evidence_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let shortening_config = match arguments.next() {
                Some(path) => {
                    let source = fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?;
                    serde_json::from_str(&source)
                        .map_err(|error| format!("failed to parse {path}: {error}"))?
                }
                None => pcb_kicad::KiCadRouteShorteningConfig::default(),
            };
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let (candidate, evidence) = pcb_kicad::shorten_route_candidate_with_config(
                Path::new(&pcb_path),
                Path::new(&candidate_path),
                &shortening_config,
            )?;
            for path in [&output, &evidence_path] {
                if let Some(parent) = Path::new(path)
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
            }
            fs::write(
                &output,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&candidate).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {output}: {error}"))?;
            fs::write(
                &evidence_path,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {evidence_path}: {error}"))?;
            println!(
                "wrote {output} and {evidence_path}: {:.3} -> {:.3} mm, {} -> {} segments, {} via(s)",
                evidence.before.length_mm,
                evidence.after.length_mm,
                evidence.before.segments,
                evidence.after.segments,
                evidence.after.vias
            );
            Ok(())
        }
        "relax-kicad-route-candidate" => {
            let usage = "usage: pcb-maker relax-kicad-route-candidate <board.kicad_pcb> <candidate.json> <committed-candidate.json> <proposal-candidate.json> <evidence.json> <config.json>";
            let pcb_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let candidate_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let proposal_output = arguments.next().ok_or_else(|| usage.to_string())?;
            let evidence_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let config_path = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let source = fs::read_to_string(&config_path)
                .map_err(|error| format!("failed to read {config_path}: {error}"))?;
            let config = serde_json::from_str(&source)
                .map_err(|error| format!("failed to parse {config_path}: {error}"))?;
            let (candidate, proposal, evidence) = pcb_kicad::relax_route_candidate(
                Path::new(&pcb_path),
                Path::new(&candidate_path),
                &config,
            )?;
            for path in [&output, &proposal_output, &evidence_path] {
                if let Some(parent) = Path::new(path)
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
            }
            fs::write(
                &output,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&candidate).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {output}: {error}"))?;
            let proposal_written = proposal.is_some();
            if let Some(proposal) = proposal {
                fs::write(
                    &proposal_output,
                    format!(
                        "{}\n",
                        serde_json::to_string_pretty(&proposal)
                            .map_err(|error| error.to_string())?
                    ),
                )
                .map_err(|error| format!("failed to write {proposal_output}: {error}"))?;
            } else if Path::new(&proposal_output).exists() {
                fs::remove_file(&proposal_output).map_err(|error| {
                    format!("failed to remove stale {proposal_output}: {error}")
                })?;
            }
            fs::write(
                &evidence_path,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write {evidence_path}: {error}"))?;
            if proposal_written {
                println!(
                    "wrote {output}, {proposal_output}, and {evidence_path}: status={:?}, {:.3} -> {:.3} mm, max-motion={:.6} mm",
                    evidence.status,
                    evidence.before.length_mm,
                    evidence.after.length_mm,
                    evidence.maximum_point_motion_mm
                );
            } else {
                println!(
                    "wrote {output} and {evidence_path}: status={:?}, no supported proposal",
                    evidence.status
                );
            }
            Ok(())
        }
        "benchmark-route-freerouting" => {
            let usage =
                "usage: pcb-maker benchmark-route-freerouting <benchmark.json> <output-directory>";
            let config = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report =
                pcb_benchmark::run_freerouting_benchmark(Path::new(&config), Path::new(&output))?;
            let result = report.result.as_ref();
            println!(
                "wrote {}: status={:?}, passes={}, source-copper=0, result-segments={}, result-vias={}, native-unconnected={}, native-drc={}",
                report.output_directory.display(),
                report.status,
                report
                    .routing
                    .as_ref()
                    .map_or(0, |routing| routing.autorouter_passes),
                result.map_or(0, |result| result.statistics.segments),
                result.map_or(0, |result| result.statistics.vias),
                result.map_or(0, |result| result
                    .verification
                    .selected_net_unconnected_items),
                result.map_or(0, |result| result.verification.drc_design_violations),
            );
            if report.complete() {
                Ok(())
            } else {
                Err(report.failure.unwrap_or_else(|| {
                    "competitive route benchmark did not pass native KiCad admission".into()
                }))
            }
        }
        "benchmark-route-compare" => {
            let usage =
                "usage: pcb-maker benchmark-route-compare <comparison.json> <output-directory>";
            let config = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report =
                pcb_benchmark::run_route_comparison(Path::new(&config), Path::new(&output))?;
            println!(
                "wrote {}: status={:?}, Freerouting={}, pcb-maker={}, leader={}, same-placement={}",
                report.output_directory.display(),
                report.status,
                report.freerouting_solved,
                report.pcb_maker_solved,
                report.completion_leader,
                report.fixed_placement_matches,
            );
            if report.status == pcb_benchmark::CompetitiveComparisonStatus::Finished {
                Ok(())
            } else {
                Err(report
                    .failure
                    .unwrap_or_else(|| "competitive comparison failed".into()))
            }
        }
        "benchmark-route-corpus" => {
            let usage = "usage: pcb-maker benchmark-route-corpus <corpus.json> <output-directory>";
            let config = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report = pcb_benchmark::run_route_corpus(Path::new(&config), Path::new(&output))?;
            println!(
                "wrote {}: status={:?}, cases={}, finished={}, failed={}, both-solved={}",
                report.output_directory.display(),
                report.status,
                report.totals.cases,
                report.totals.finished,
                report.totals.failed,
                report.totals.both_solved,
            );
            if report.status == pcb_benchmark::CompetitiveCorpusStatus::Finished {
                Ok(())
            } else {
                Err(format!(
                    "competitive corpus has {} failed case(s)",
                    report.totals.failed
                ))
            }
        }
        "help" | "--help" | "-h" => {
            println!(
                "usage:\n  pcb-maker check <problem.json>\n  pcb-maker validate <problem.json> <candidate-or-result.json>\n  pcb-maker insert-connection <source.problem.json> <target.problem.json> <parent.candidate-or-result.json> <output.json> [config.json]\n  pcb-maker place <problem.json> <policy-name|config.json> [result.json]\n  pcb-maker trace-placement <problem.json> <policy-name|config.json> <trace.json>\n  pcb-maker route-grid <problem.json> [result.json] [routing-config.json]\n  pcb-maker route-grid-negotiated <problem.json> [result.json] [negotiated-config.json]\n  pcb-maker explore-conflict-actions <problem.json> [result.json] [config.json]\n  pcb-maker search-conflict-actions <problem.json> [result.json] [config.json]\n  pcb-maker continue-board <problem.json> [result.json] [config.json] [progress-prefix]\n  pcb-maker place-route-grid <problem.json> <policy-name|config.json> [result.json] [routing-config.json]\n  pcb-maker place-route-grid-negotiated <problem.json> <policy-name|config.json> [result.json] [negotiated-config.json]\n  pcb-maker analyze-corridors <problem.json> <policy-name|config.json> [result.json] [corridor-config.json]\n  pcb-maker analyze-route-families <problem.json> <policy-name|config.json> [result.json] [family-config.json]\n  pcb-maker repair-route-families <problem.json> <candidate-or-result.json> [result.json] [family-config.json]\n  pcb-maker repair-route-grid <problem.json> <policy-name|config.json> [result.json] [routing-config.json] [repair-config.json]\n  pcb-maker optimize-route-junctions <problem.json> <policy-name|config.json> [result.json] [routing-config.json] [junction-config.json]\n  pcb-maker repair-route-pressure <problem.json> <policy-name|config.json> [result.json] [routing-config.json] [pressure-config.json]\n  pcb-maker repair-route-order <problem.json> <policy-name|config.json> [result.json] [routing-config.json] [repair-config.json]\n  pcb-maker repair-route-ripup <problem.json> <policy-name|config.json> [result.json] [routing-config.json] [ripup-config.json]\n  pcb-maker view-route <problem.json> <candidate-or-result.json> [output.html]\n  pcb-maker render-route-layers <problem.json> <candidate-or-result.json> <output-prefix>\n  pcb-maker view-continuous-repair <route-family-result.json> [output.html]\n  pcb-maker relax-continuous <problem.json> [result.json] [viewer.html] [subdivide|preserve-seed]\n  pcb-maker generate-semantic-kicad-ladder <problem.json> <placement-policy|config.json> <template-config.json> <output-directory>\n  pcb-maker solve-semantic-kicad-prefix <problem.json> <placement-policy|config.json> <template-config.json> <target-rung> <output-directory> [insertion-config.json]\n  pcb-maker materialize-kicad-rung <declaration.json> <rung> <output-directory>\n  pcb-maker insert-kicad-connection <declaration.json> <parent-rung-directory> <transaction-directory> [config.json]\n  pcb-maker progress-kicad-connections <declaration.json> <initial-parent-directory> <progression-directory> [config.json]\n  pcb-maker verify-kicad-rung <directory> <board-id>\n  pcb-maker repair-kicad-silkscreen <source-directory> <board-id> <output-directory>\n  pcb-maker normalize-kicad-footprint-links <directory> <board-id> [report.json]\n  pcb-maker strip-kicad-copper <source.kicad_pcb> <output.kicad_pcb> [report.json]\n  pcb-maker inspect-kicad-board <board.kicad_pcb>\n  pcb-maker inspect-kicad-connections <board.kicad_pcb>\n  pcb-maker route-kicad-connection <board.kicad_pcb> <connection> <candidate.json> [config.json]\n  pcb-maker import-kicad-route-candidate <board.kicad_pcb> <connection> <candidate.json>\n  pcb-maker apply-kicad-route-candidate <board.kicad_pcb> <candidate.json> <output.kicad_pcb>\n  pcb-maker relax-kicad-reference-fields <board.kicad_pcb> <candidate.json> <drc.json> <iteration> <output-candidate.json> <evidence.json>\n  pcb-maker search-kicad-reference-fields <source-rung-directory> <board-id> <candidate.json> <output-directory>\n  pcb-maker render-kicad-layers <board.kicad_pcb> <output-prefix> [config.json]\n  pcb-maker measure-kicad-route-candidate <candidate.json>\n  pcb-maker shorten-kicad-route-candidate <board.kicad_pcb> <candidate.json> <output-candidate.json> <evidence.json> [config.json]\n  pcb-maker resistor-field [output.html]\n  pcb-maker resistor-ablation [output-prefix]"
            );
            println!(
                "  pcb-maker relax-kicad-route-candidate <board.kicad_pcb> <candidate.json> <committed-candidate.json> <proposal-candidate.json> <evidence.json> <config.json>"
            );
            println!(
                "  pcb-maker solve-semantic-kicad-placement-portfolio <problem.json> <placement-portfolio.json> <template-config.json> <target-rung> <output-directory> [insertion-config.json]"
            );
            println!(
                "  pcb-maker solve-semantic-kicad-order-portfolio <problem.json> <placement-policy|config.json> <template-config.json> <target-rung> <order-search-config.json> <output-directory> [insertion-config.json]"
            );
            println!(
                "  pcb-maker place-route-grid-width-continuation <problem.json> <placement-policy|config.json> <width-continuation-config.json> <output-directory>"
            );
            println!(
                "  pcb-maker solve-semantic-kicad-prefix-feedback <problem.json> <placement-policy|config.json> <template-config.json> <target-rung> <output-directory> <dut-routing-config.json> <pressure-config.json> [insertion-config.json]"
            );
            println!(
                "  pcb-maker diagnose-kicad-yielding-connections <board.kicad_pcb> <target-connection> <evidence.json> [config.json]"
            );
            println!(
                "  pcb-maker repair-kicad-single-connection-ripup <unrouted-target-directory> <board-id> <target-connection> <output-directory> [config.json]"
            );
            println!(
                "  pcb-maker search-kicad-via-actions <source-rung-directory> <board-id> <candidate.json> <output-directory> <config.json>"
            );
            println!(
                "  pcb-maker refine-kicad-vias <source-directory> <board-id> <candidate.json> <output-directory> <config.json>"
            );
            println!(
                "  pcb-maker discover-kicad-vias <board.kicad_pcb> <output-directory> <config.json>"
            );
            println!(
                "  pcb-maker replace-kicad-connection-with-zones <source-rung-directory> <board-id> <output-directory> <config.json>"
            );
            println!(
                "  pcb-maker materialize-route-family-portfolio <problem.json> <portfolio.json> <policy-name|config.json> [result.json]"
            );
            println!("  pcb-maker benchmark-route-freerouting <benchmark.json> <output-directory>");
            println!("  pcb-maker benchmark-route-compare <comparison.json> <output-directory>");
            println!("  pcb-maker benchmark-route-corpus <corpus.json> <output-directory>");
            println!("  pcb-maker inspect-kicad-board <board.kicad_pcb>");
            println!("  pcb-maker inspect-kicad-terminal-access <board.kicad_pcb> <connection> <report.json> [config.json]");
            println!("  pcb-maker inspect-kicad-routing-cut <board.kicad_pcb> <connection> <report.json> [config.json]");
            println!(
                "  pcb-maker route-kicad-board-sequential <source-directory> <board-id> <output-directory> <config.json>"
            );
            println!(
                "  pcb-maker resume-kicad-board-sequential <checkpoint-directory> <output-directory> <resume-config.json>"
            );
            println!(
                "  pcb-maker bridge-kicad-open-connections <source-directory> <board-id> <config.json> <output-directory>\n  pcb-maker relax-kicad-joint-routes <source-directory> <board-id> <target-candidate.json> <config.json> <output-directory>\n  pcb-maker inspect-kicad-route-obstructions <board.kicad_pcb> <candidate.json> <routing-config.json> <output.json>\n  pcb-maker propose-kicad-pad-pair-route <board.kicad_pcb> <request.json> <output-directory>"
            );
            println!(
                "  pcb-maker route-kicad-board-freerouting <source-directory> <board-id> <output-directory> <config.json>"
            );
            println!(
                "  pcb-maker route-kicad-board-adaptive <source-directory> <board-id> <output-directory> <config.json>"
            );
            println!(
                "  pcb-maker search-kicad-board-sequential-orders <source-directory> <board-id> <output-directory> <config.json>"
            );
            Ok(())
        }
        other => Err(format!("unknown command {other:?}")),
    }
}

fn parse_candidate_or_result(source: &str) -> Result<pcb_validate::CandidateArtifact, String> {
    let document: serde_json::Value =
        serde_json::from_str(source).map_err(|error| error.to_string())?;
    let candidate = document.get("candidate").unwrap_or(&document).clone();
    serde_json::from_value(candidate).map_err(|error| error.to_string())
}

fn load_grid_routing_config(
    path: Option<&str>,
) -> Result<pcb_routing::DutGridRoutingConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_routing::DutGridRoutingConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read routing config {path}: {error}"))?;
    let config: pcb_routing::DutGridRoutingConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse routing config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn load_width_continuation_config(
    path: &str,
) -> Result<pcb_routing::WidthContinuationConfig, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read width-continuation config {path}: {error}"))?;
    let config: pcb_routing::WidthContinuationConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse width-continuation config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn load_connection_insertion_config(
    path: Option<&str>,
) -> Result<pcb_coordinator::ConnectionInsertionConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_coordinator::ConnectionInsertionConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read connection insertion config {path}: {error}"))?;
    let config: pcb_coordinator::ConnectionInsertionConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse connection insertion config {path}: {error}"))?;
    config.check().map_err(|error| error.to_string())?;
    Ok(config)
}

fn load_negotiated_routing_config(
    path: Option<&str>,
) -> Result<pcb_routing::NegotiatedDutGridRoutingConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_routing::NegotiatedDutGridRoutingConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read negotiated routing config {path}: {error}"))?;
    let config: pcb_routing::NegotiatedDutGridRoutingConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse negotiated routing config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn load_conflict_action_config(
    path: Option<&str>,
) -> Result<pcb_coordinator::ConflictActionSearchConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_coordinator::ConflictActionSearchConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read conflict-action config {path}: {error}"))?;
    let config: pcb_coordinator::ConflictActionSearchConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse conflict-action config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn load_best_first_conflict_action_config(
    path: Option<&str>,
) -> Result<pcb_coordinator::BestFirstConflictActionSearchConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_coordinator::BestFirstConflictActionSearchConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read best-first conflict config {path}: {error}"))?;
    let config: pcb_coordinator::BestFirstConflictActionSearchConfig =
        serde_json::from_str(&source).map_err(|error| {
            format!("failed to parse best-first conflict config {path}: {error}")
        })?;
    config.check()?;
    Ok(config)
}

fn load_board_continuation_config(
    path: Option<&str>,
) -> Result<pcb_coordinator::BoardContinuationConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_coordinator::BoardContinuationConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read board continuation config {path}: {error}"))?;
    let config: pcb_coordinator::BoardContinuationConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse board continuation config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn load_corridor_analysis_config(
    path: Option<&str>,
) -> Result<pcb_routing::CorridorAnalysisConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_routing::CorridorAnalysisConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read corridor config {path}: {error}"))?;
    let config: pcb_routing::CorridorAnalysisConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse corridor config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn load_route_family_analysis_config(
    path: Option<&str>,
) -> Result<pcb_routing::RouteFamilyAnalysisConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_routing::RouteFamilyAnalysisConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read route-family config {path}: {error}"))?;
    let config: pcb_routing::RouteFamilyAnalysisConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse route-family config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn load_blocker_repair_config(
    path: Option<&str>,
) -> Result<pcb_coordinator::BlockerRepairConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_coordinator::BlockerRepairConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read blocker repair config {path}: {error}"))?;
    let config: pcb_coordinator::BlockerRepairConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse blocker repair config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn load_pressure_repair_config(
    path: Option<&str>,
) -> Result<pcb_coordinator::PressureRepairConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_coordinator::PressureRepairConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read pressure repair config {path}: {error}"))?;
    let config: pcb_coordinator::PressureRepairConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse pressure repair config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn load_route_junction_config(
    path: Option<&str>,
) -> Result<pcb_coordinator::RouteJunctionConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_coordinator::RouteJunctionConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read route junction config {path}: {error}"))?;
    let config: pcb_coordinator::RouteJunctionConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse route junction config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn load_failure_directed_order_config(
    path: Option<&str>,
) -> Result<pcb_coordinator::FailureDirectedOrderConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_coordinator::FailureDirectedOrderConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read route-order repair config {path}: {error}"))?;
    let config: pcb_coordinator::FailureDirectedOrderConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse route-order repair config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn load_selective_ripup_config(
    path: Option<&str>,
) -> Result<pcb_coordinator::SelectiveRipupConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_coordinator::SelectiveRipupConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read selective rip-up config {path}: {error}"))?;
    let config: pcb_coordinator::SelectiveRipupConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse selective rip-up config {path}: {error}"))?;
    config.check()?;
    Ok(config)
}

fn write_serialized_result<T: serde::Serialize>(
    result: &T,
    output: Option<&str>,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(result).map_err(|error| error.to_string())?;
    if let Some(output) = output {
        let path = Path::new(output);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, format!("{serialized}\n")).map_err(|error| error.to_string())
    } else {
        println!("{serialized}");
        Ok(())
    }
}

fn render_progress_layer_pair(
    title: &str,
    problem: &layout_trace_model::Problem,
    candidate: &pcb_validate::CandidateArtifact,
    output_prefix: &str,
) -> Result<(), String> {
    let front_layer = problem.board.layers.first().map(|layer| layer.id.as_str());
    let back_layer = problem.board.layers.get(1).map(|layer| layer.id.as_str());
    for (side, layer) in [("front", front_layer), ("back", back_layer)] {
        let svg_output = format!("{output_prefix}-{side}.svg");
        let path = Path::new(&svg_output);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let svg = pcb_viewer::render_candidate_layer_svg(
            &format!("{title} — {side}"),
            problem,
            candidate,
            layer,
        );
        fs::write(path, svg).map_err(|error| error.to_string())?;
        let png_output = format!("{output_prefix}-{side}.png");
        let png_status = ProcessCommand::new("rsvg-convert")
            .arg(&svg_output)
            .arg("-o")
            .arg(&png_output)
            .status();
        match png_status {
            Ok(status) if status.success() => println!(
                "wrote {svg_output} and {png_output}: {}",
                layer.map_or("no declared copper", |layer| layer)
            ),
            Ok(status) => println!(
                "wrote {svg_output}; PNG conversion exited with {status}: {}",
                layer.map_or("no declared copper", |layer| layer)
            ),
            Err(error) => println!(
                "wrote {svg_output}; PNG conversion unavailable ({error}): {}",
                layer.map_or("no declared copper", |layer| layer)
            ),
        }
    }
    Ok(())
}

fn write_order_repair_result(
    result: &pcb_coordinator::FailureDirectedOrderResult,
    output: Option<&str>,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(result).map_err(|error| error.to_string())?;
    if let Some(output) = output {
        let path = Path::new(output);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, format!("{serialized}\n")).map_err(|error| error.to_string())
    } else {
        println!("{serialized}");
        Ok(())
    }
}

fn write_selective_ripup_result(
    result: &pcb_coordinator::SelectiveRipupResult,
    output: Option<&str>,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(result).map_err(|error| error.to_string())?;
    if let Some(output) = output {
        let path = Path::new(output);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, format!("{serialized}\n")).map_err(|error| error.to_string())
    } else {
        println!("{serialized}");
        Ok(())
    }
}

fn solved_components_from_placement(
    problem: &layout_trace_model::Problem,
    placement: &pcb_placement::InitialPlacementResult,
) -> Result<Vec<pcb_validate::SolvedComponent>, String> {
    placement
        .poses
        .iter()
        .map(|pose| {
            let component = problem
                .components
                .iter()
                .find(|component| component.id == pose.component)
                .ok_or_else(|| {
                    format!("placement returned unknown component {}", pose.component)
                })?;
            Ok(pcb_validate::SolvedComponent {
                id: pose.component.clone(),
                position: pose.position,
                size: component.size,
                rotation_degrees: pose.rotation_degrees,
            })
        })
        .collect()
}

fn write_routing_result(
    result: &pcb_routing::DutGridRoutingResult,
    output: Option<&str>,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(result).map_err(|error| error.to_string())?;
    if let Some(output) = output {
        let path = Path::new(output);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, format!("{serialized}\n")).map_err(|error| error.to_string())
    } else {
        println!("{serialized}");
        Ok(())
    }
}

fn write_negotiated_routing_result(
    result: &pcb_routing::NegotiatedDutGridRoutingResult,
    output: Option<&str>,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(result).map_err(|error| error.to_string())?;
    if let Some(output) = output {
        let path = Path::new(output);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, format!("{serialized}\n")).map_err(|error| error.to_string())
    } else {
        println!("{serialized}");
        Ok(())
    }
}

fn write_corridor_analysis_result(
    result: &pcb_routing::CorridorAnalysisResult,
    output: Option<&str>,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(result).map_err(|error| error.to_string())?;
    if let Some(output) = output {
        let path = Path::new(output);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, format!("{serialized}\n")).map_err(|error| error.to_string())
    } else {
        println!("{serialized}");
        Ok(())
    }
}

fn write_route_family_analysis_result(
    result: &pcb_routing::RouteFamilyAnalysisResult,
    output: Option<&str>,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(result).map_err(|error| error.to_string())?;
    if let Some(output) = output {
        let path = Path::new(output);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, format!("{serialized}\n")).map_err(|error| error.to_string())
    } else {
        println!("{serialized}");
        Ok(())
    }
}

fn write_route_family_repair_result(
    result: &pcb_routing::RouteFamilyRepairResult,
    output: Option<&str>,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(result).map_err(|error| error.to_string())?;
    if let Some(output) = output {
        let path = Path::new(output);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, format!("{serialized}\n")).map_err(|error| error.to_string())
    } else {
        println!("{serialized}");
        Ok(())
    }
}

fn report_routing_result(result: &pcb_routing::DutGridRoutingResult, output: Option<&str>) {
    if let Some(output) = output {
        println!(
            "wrote {output}: routed={}/{}, searches={}, expansions={}, exact={}",
            result.evidence.routed_branches,
            result.evidence.branch_count,
            result.evidence.searches,
            result.evidence.expansions,
            if result.validation.complete {
                "pass"
            } else {
                "fail"
            }
        );
    }
}

fn report_negotiated_routing_result(
    result: &pcb_routing::NegotiatedDutGridRoutingResult,
    output: Option<&str>,
) {
    if let Some(output) = output {
        let pass = &result.evidence.passes[result.evidence.selected_pass];
        println!(
            "wrote {output}: selected-pass={}/{}, routed={}/{}, congested-cells={}, selected-expansions={}, total-expansions={}, exact={}",
            result.evidence.selected_pass,
            result.evidence.completed_passes - 1,
            result.evidence.routing.routed_branches,
            result.evidence.routing.branch_count,
            pass.congested_cells,
            result.evidence.routing.expansions,
            result.evidence.total_expansions,
            if result.validation.complete {
                "pass"
            } else {
                "fail"
            }
        );
    }
}

fn initial_placement_config(name: &str) -> Result<pcb_placement::InitialPlacementConfig, String> {
    use pcb_placement::{InitialPlacementConfig, InitialPlacementPolicy, UnderanchoredSeedPolicy};

    let policy = match name {
        "declared" => InitialPlacementPolicy::Declared,
        "grid" => InitialPlacementPolicy::GridPacking { gap_mm: 1.0 },
        "random" => InitialPlacementPolicy::Random { attempts: 16 },
        "barycentric" => InitialPlacementPolicy::ConnectivityBarycentric {
            iterations: 12,
            attraction: 0.35,
        },
        "harmonic" => InitialPlacementPolicy::HarmonicPorts {
            fixed_obstacle_seed_projection: false,
            largest_first_seed_insertion: false,
            skip_collision_intervals: false,
            seed_extra_pad_gap_mm: 0.0,
            coupled_legalization: None,
            iterations: 128,
            legalization_sweeps: 64,
            maximum_pair_checks: 1_000_000,
            connected_pair_spacing_floor: true,
            underanchored_seed: UnderanchoredSeedPolicy::Declared,
        },
        _ => {
            return Err(format!(
                "unknown placement policy {name:?}; expected declared, grid, random, barycentric, or harmonic"
            ));
        }
    };
    Ok(InitialPlacementConfig {
        orientation_refinement: None,
        policy,
        seed: 0,
        projection_sweeps: 256,
    })
}

fn load_initial_placement_config(
    name_or_path: &str,
) -> Result<pcb_placement::InitialPlacementConfig, String> {
    if let Ok(config) = initial_placement_config(name_or_path) {
        return Ok(config);
    }
    let path = Path::new(name_or_path);
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "unknown placement policy and failed to read config {}: {error}",
            path.display()
        )
    })?;
    let config: pcb_placement::InitialPlacementConfig =
        serde_json::from_str(&source).map_err(|error| {
            format!(
                "failed to parse placement config {}: {error}",
                path.display()
            )
        })?;
    config.check()?;
    Ok(config)
}

fn load_layout_trace_problem(path: &Path) -> Result<layout_trace_model::Problem, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let problem: layout_trace_model::Problem = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    problem
        .check_schema()
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    Ok(problem)
}

#[derive(Clone, Debug, serde::Serialize)]
struct SemanticKiCadGenerationTiming {
    advisory_only: bool,
    problem_loading_elapsed_micros: u64,
    config_and_prefix_elapsed_micros: u64,
    placement_elapsed_micros: u64,
    template_generation_elapsed_micros: u64,
    total_elapsed_micros: u64,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct SemanticKiCadPlacementPortfolioEntry {
    id: String,
    placement: pcb_placement::InitialPlacementConfig,
    /// Number of cheap, unique placement proposals from this policy that may
    /// enter independent zero-copper native trials. One preserves the
    /// historical first-feasible behavior.
    #[serde(default = "default_placement_proposal_finalists")]
    proposal_finalists: usize,
    /// Optional semantic tracer-to-placer evolution applied independently to
    /// every retained initial proposal before native routing. The unmodified
    /// proposal remains a control by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    routing_feedback: Option<SemanticKiCadPlacementRoutingFeedbackConfig>,
}

fn default_placement_proposal_finalists() -> usize {
    1
}

fn default_retain_feedback_parent_trial() -> bool {
    true
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct SemanticKiCadPlacementRoutingFeedbackConfig {
    routing: pcb_routing::DutGridRoutingConfig,
    pressure: pcb_coordinator::PressureRepairConfig,
    /// Preserve the exact unmodified placement as an independently rerouted
    /// native control and transactional fallback.
    #[serde(default = "default_retain_feedback_parent_trial")]
    retain_parent_trial: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct SemanticKiCadPlacementPortfolioConfig {
    /// Explicit hard bound: adding an entry must never silently multiply a
    /// multi-minute native experiment.
    maximum_entries: usize,
    /// Optional bound after proposal archives expand an entry into several
    /// expensive native trials. Old configs default this to maximum_entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    maximum_trials: Option<usize>,
    via_penalty_mm: f64,
    entries: Vec<SemanticKiCadPlacementPortfolioEntry>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct SemanticKiCadOrderTrial {
    ordinal: usize,
    parent_trial: Option<usize>,
    order_fingerprint: String,
    connection_order: Vec<String>,
    proposal: Option<pcb_kicad::KiCadConnectionOrderProposalEvidence>,
    directory: std::path::PathBuf,
    placement: Option<pcb_placement::InitialPlacementResult>,
    template: Option<pcb_kicad::SemanticKiCadTemplateReport>,
    initial: Option<pcb_kicad::MaterializationReport>,
    progression: Option<pcb_kicad::KiCadConnectionProgressionResult>,
    final_rung: usize,
    final_directory: std::path::PathBuf,
    completed: bool,
    board_route_quality: Option<SemanticKiCadBoardRouteQuality>,
    error: Option<String>,
    selected: bool,
    elapsed_micros: u64,
}

#[derive(Clone, Debug)]
struct PendingSemanticKiCadOrderTrial {
    parent_trial: Option<usize>,
    connection_order: Vec<String>,
    proposal: Option<pcb_kicad::KiCadConnectionOrderProposalEvidence>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct SemanticKiCadBoardRouteQuality {
    connections: usize,
    length_mm: f64,
    segments: usize,
    stored_track_length_mm: f64,
    stored_segments: usize,
    overlapping_track_length_mm: f64,
    vias: usize,
    close_via_pairs_within_connections: usize,
    clustered_vias_within_connections: usize,
    score_mm: f64,
    interpretation: String,
}

#[derive(Clone, Debug, serde::Serialize)]
struct SemanticKiCadPlacementPortfolioTrialTiming {
    advisory_only: bool,
    placement_elapsed_micros: u64,
    routing_feedback_elapsed_micros: u64,
    template_generation_elapsed_micros: u64,
    initial_materialization_elapsed_micros: u64,
    progression_elapsed_micros: u64,
    total_elapsed_micros: u64,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum SemanticKiCadPlacementTrialOrigin {
    InitialProposal,
    RoutingFeedback,
}

#[derive(Clone, Debug, serde::Serialize)]
struct SemanticKiCadPlacementRoutingFeedbackEvidence {
    evidence: String,
    strategy: String,
    semantic_complete: bool,
    selected_attempt: usize,
    attempted_repairs: usize,
    total_expansions: u64,
    routed_branches: usize,
    failed_branches: usize,
    moved_components: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct SemanticKiCadPlacementPortfolioTrial {
    ordinal: usize,
    entry_ordinal: usize,
    proposal_ordinal: usize,
    proposal_archive_rank: usize,
    origin: SemanticKiCadPlacementTrialOrigin,
    id: String,
    directory: std::path::PathBuf,
    placement_config: pcb_placement::InitialPlacementConfig,
    /// The independently generated initial proposal that is this trial's
    /// parent, retained even when evaluated_poses contain a feedback child.
    placement: Option<pcb_placement::InitialPlacementResult>,
    evaluated_poses: Option<Vec<pcb_placement::PlacementPose>>,
    evaluated_demand: Option<pcb_placement::PlacementDemandEvidence>,
    routing_feedback: Option<SemanticKiCadPlacementRoutingFeedbackEvidence>,
    template: Option<pcb_kicad::SemanticKiCadTemplateReport>,
    initial: Option<pcb_kicad::MaterializationReport>,
    progression: Option<pcb_kicad::KiCadConnectionProgressionResult>,
    final_rung: usize,
    final_directory: std::path::PathBuf,
    completed: bool,
    board_route_quality: Option<SemanticKiCadBoardRouteQuality>,
    error: Option<String>,
    selected: bool,
    timing: SemanticKiCadPlacementPortfolioTrialTiming,
}

struct SemanticKiCadPlacementTrialInput {
    entry_ordinal: usize,
    proposal_ordinal: usize,
    proposal_archive_rank: usize,
    origin: SemanticKiCadPlacementTrialOrigin,
    id: String,
    placement_config: pcb_placement::InitialPlacementConfig,
    placement: Result<pcb_placement::InitialPlacementResult, String>,
    evaluated_poses: Option<Vec<pcb_placement::PlacementPose>>,
    routing_feedback: Option<pcb_coordinator::PressureRepairResult>,
    placement_elapsed_micros: u64,
    routing_feedback_elapsed_micros: u64,
}

fn validate_semantic_kicad_placement_portfolio_config(
    config: &SemanticKiCadPlacementPortfolioConfig,
) -> Result<(), String> {
    if config.maximum_entries == 0 || config.entries.is_empty() {
        return Err("placement portfolio requires a positive bound and at least one entry".into());
    }
    if config.entries.len() > config.maximum_entries {
        return Err(format!(
            "placement portfolio declares {} entries above maximum_entries {}",
            config.entries.len(),
            config.maximum_entries
        ));
    }
    let maximum_trials = config.maximum_trials.unwrap_or(config.maximum_entries);
    if maximum_trials == 0 {
        return Err("placement portfolio maximum_trials must be positive".into());
    }
    let declared_trials = config.entries.iter().try_fold(0_usize, |total, entry| {
        if entry.proposal_finalists == 0 {
            return Err(format!(
                "placement portfolio entry {:?} proposal_finalists must be positive",
                entry.id
            ));
        }
        if let Some(feedback) = &entry.routing_feedback {
            feedback.routing.check()?;
            feedback.pressure.check()?;
        }
        let trials_per_proposal = if entry
            .routing_feedback
            .as_ref()
            .is_some_and(|feedback| feedback.retain_parent_trial)
        {
            2
        } else {
            1
        };
        total
            .checked_add(
                entry
                    .proposal_finalists
                    .checked_mul(trials_per_proposal)
                    .ok_or_else(|| "placement portfolio trial count overflow".to_string())?,
            )
            .ok_or_else(|| "placement portfolio trial count overflow".to_string())
    })?;
    if declared_trials > maximum_trials {
        return Err(format!(
            "placement portfolio declares {declared_trials} finalist trials above maximum_trials {maximum_trials}"
        ));
    }
    if !config.via_penalty_mm.is_finite() || config.via_penalty_mm < 0.0 {
        return Err("placement portfolio via_penalty_mm must be finite and non-negative".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for entry in &config.entries {
        if entry.id.is_empty()
            || !entry.id.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
            || !ids.insert(entry.id.as_str())
        {
            return Err(
                "placement portfolio entry ids must be unique, non-empty filesystem labels".into(),
            );
        }
        entry.placement.check()?;
    }
    Ok(())
}

fn same_semantic_placement_poses(
    left: &[pcb_placement::PlacementPose],
    right: &[pcb_placement::PlacementPose],
) -> bool {
    let mut left = left
        .iter()
        .map(|pose| {
            (
                pose.component.as_str(),
                pose.position.x.to_bits(),
                pose.position.y.to_bits(),
                pose.rotation_degrees.to_bits(),
            )
        })
        .collect::<Vec<_>>();
    let mut right = right
        .iter()
        .map(|pose| {
            (
                pose.component.as_str(),
                pose.position.x.to_bits(),
                pose.position.y.to_bits(),
                pose.rotation_degrees.to_bits(),
            )
        })
        .collect::<Vec<_>>();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

fn routing_feedback_moved_components(
    initial: &pcb_placement::InitialPlacementResult,
    feedback: &pcb_coordinator::PressureRepairResult,
) -> Vec<String> {
    let initial = initial
        .poses
        .iter()
        .map(|pose| (pose.component.as_str(), pose))
        .collect::<std::collections::HashMap<_, _>>();
    let mut moved = feedback
        .candidate
        .components
        .iter()
        .filter(|component| {
            initial.get(component.id.as_str()).is_none_or(|pose| {
                component.position.x.to_bits() != pose.position.x.to_bits()
                    || component.position.y.to_bits() != pose.position.y.to_bits()
                    || component.rotation_degrees.to_bits() != pose.rotation_degrees.to_bits()
            })
        })
        .map(|component| component.id.clone())
        .collect::<Vec<_>>();
    moved.sort();
    moved
}

#[allow(clippy::too_many_arguments)]
fn append_semantic_kicad_placement_trial_inputs(
    trial_inputs: &mut Vec<SemanticKiCadPlacementTrialInput>,
    problem: &layout_trace_model::Problem,
    entry: &SemanticKiCadPlacementPortfolioEntry,
    entry_ordinal: usize,
    proposal_ordinal: usize,
    proposal_archive_rank: usize,
    base_id: String,
    placement: Result<pcb_placement::InitialPlacementResult, String>,
    placement_elapsed_micros: u64,
) -> Result<(), String> {
    let Some(feedback_config) = &entry.routing_feedback else {
        trial_inputs.push(SemanticKiCadPlacementTrialInput {
            entry_ordinal,
            proposal_ordinal,
            proposal_archive_rank,
            origin: SemanticKiCadPlacementTrialOrigin::InitialProposal,
            id: base_id,
            placement_config: entry.placement.clone(),
            placement,
            evaluated_poses: None,
            routing_feedback: None,
            placement_elapsed_micros,
            routing_feedback_elapsed_micros: 0,
        });
        return Ok(());
    };

    let placement = placement?;
    let initial_components = solved_components_from_placement(problem, &placement)?;
    let feedback_started = Instant::now();
    let feedback = pcb_coordinator::repair_routing_with_pressure(
        problem,
        &initial_components,
        &feedback_config.routing,
        &feedback_config.pressure,
    )?;
    let routing_feedback_elapsed_micros = feedback_started.elapsed().as_micros() as u64;
    let feedback_poses = feedback
        .candidate
        .components
        .iter()
        .map(|component| pcb_placement::PlacementPose {
            component: component.id.clone(),
            position: component.position,
            rotation_degrees: component.rotation_degrees,
        })
        .collect::<Vec<_>>();
    let changed = !same_semantic_placement_poses(&placement.poses, &feedback_poses);

    if feedback_config.retain_parent_trial {
        trial_inputs.push(SemanticKiCadPlacementTrialInput {
            entry_ordinal,
            proposal_ordinal,
            proposal_archive_rank,
            origin: SemanticKiCadPlacementTrialOrigin::InitialProposal,
            id: format!("{base_id}-parent"),
            placement_config: entry.placement.clone(),
            placement: Ok(placement.clone()),
            evaluated_poses: None,
            // A no-op feedback search is still important negative evidence,
            // but must not create an identical expensive native trial.
            routing_feedback: (!changed).then(|| feedback.clone()),
            placement_elapsed_micros,
            routing_feedback_elapsed_micros: if changed {
                0
            } else {
                routing_feedback_elapsed_micros
            },
        });
    }
    if changed || !feedback_config.retain_parent_trial {
        trial_inputs.push(SemanticKiCadPlacementTrialInput {
            entry_ordinal,
            proposal_ordinal,
            proposal_archive_rank,
            origin: SemanticKiCadPlacementTrialOrigin::RoutingFeedback,
            id: format!("{base_id}-feedback"),
            placement_config: entry.placement.clone(),
            placement: Ok(placement),
            evaluated_poses: Some(feedback_poses),
            routing_feedback: Some(feedback),
            placement_elapsed_micros,
            routing_feedback_elapsed_micros,
        });
    }
    Ok(())
}

fn measure_semantic_kicad_board_route_quality(
    declaration: &pcb_kicad::LadderDeclaration,
    rung: usize,
    directory: &Path,
    via_penalty_mm: f64,
) -> Result<SemanticKiCadBoardRouteQuality, String> {
    let board = directory.join(format!("{}.kicad_pcb", declaration.board_id));
    let mut length_mm = 0.0;
    let mut segments = 0;
    let mut stored_track_length_mm = 0.0;
    let mut stored_segments = 0;
    let mut overlapping_track_length_mm = 0.0;
    let mut vias = 0;
    let mut close_via_pairs_within_connections = 0;
    let mut clustered_vias_within_connections = 0;
    for connection in declaration.connections.iter().take(rung) {
        let candidate = pcb_kicad::import_route_candidate_from_pcb(&board, connection)?;
        let quality = pcb_kicad::route_candidate_quality(&candidate)?;
        length_mm += quality.length_mm;
        segments += quality.segments;
        stored_track_length_mm += quality.stored_track_length_mm;
        stored_segments += quality.stored_segments;
        overlapping_track_length_mm += quality.overlapping_track_length_mm;
        vias += quality.vias;
        close_via_pairs_within_connections += quality.close_via_pairs;
        clustered_vias_within_connections += quality.clustered_vias;
    }
    Ok(SemanticKiCadBoardRouteQuality {
        connections: rung,
        length_mm,
        segments,
        stored_track_length_mm,
        stored_segments,
        overlapping_track_length_mm,
        vias,
        close_via_pairs_within_connections,
        clustered_vias_within_connections,
        score_mm: length_mm + vias as f64 * via_penalty_mm,
        interpretation: "sum of independently imported per-connection physical copper; close/clustered via counts do not include cross-net pairs".into(),
    })
}

fn ranked_semantic_kicad_placement_trial_indices(
    trials: &[SemanticKiCadPlacementPortfolioTrial],
) -> Vec<usize> {
    let mut ranked = (0..trials.len()).collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        let left = &trials[*left];
        let right = &trials[*right];
        right
            .completed
            .cmp(&left.completed)
            .then_with(|| right.final_rung.cmp(&left.final_rung))
            .then_with(|| {
                left.board_route_quality
                    .as_ref()
                    .map_or(f64::INFINITY, |quality| quality.score_mm)
                    .total_cmp(
                        &right
                            .board_route_quality
                            .as_ref()
                            .map_or(f64::INFINITY, |quality| quality.score_mm),
                    )
            })
            .then_with(|| {
                left.evaluated_demand
                    .as_ref()
                    .map_or(usize::MAX, |demand| demand.cross_net_proper_crossings)
                    .cmp(
                        &right
                            .evaluated_demand
                            .as_ref()
                            .map_or(usize::MAX, |demand| demand.cross_net_proper_crossings),
                    )
            })
            .then_with(|| {
                left.evaluated_demand
                    .as_ref()
                    .map_or(f64::INFINITY, |demand| demand.squared_cell_demand)
                    .total_cmp(
                        &right
                            .evaluated_demand
                            .as_ref()
                            .map_or(f64::INFINITY, |demand| demand.squared_cell_demand),
                    )
            })
            .then_with(|| left.ordinal.cmp(&right.ordinal))
    });
    ranked
}

fn solve_semantic_kicad_placement_portfolio(
    problem_path: &Path,
    portfolio_config_path: &Path,
    template_config_path: &Path,
    target_rung: usize,
    output_directory: &Path,
    insertion: &pcb_kicad::KiCadConnectionInsertionConfig,
) -> Result<(String, usize, std::path::PathBuf, bool), String> {
    if output_directory.exists() {
        return Err(format!(
            "refusing to overwrite cold semantic KiCad placement portfolio {}",
            output_directory.display()
        ));
    }
    let portfolio_started = Instant::now();
    let portfolio_source = fs::read_to_string(portfolio_config_path).map_err(|error| {
        format!(
            "failed to read placement portfolio {}: {error}",
            portfolio_config_path.display()
        )
    })?;
    let portfolio: SemanticKiCadPlacementPortfolioConfig = serde_json::from_str(&portfolio_source)
        .map_err(|error| {
            format!(
                "failed to parse placement portfolio {}: {error}",
                portfolio_config_path.display()
            )
        })?;
    validate_semantic_kicad_placement_portfolio_config(&portfolio)?;
    let mut active_problem = load_layout_trace_problem(problem_path)?;
    let template_source = fs::read_to_string(template_config_path).map_err(|error| {
        format!(
            "failed to read semantic KiCad template config {}: {error}",
            template_config_path.display()
        )
    })?;
    let mut active_template_config: pcb_kicad::SemanticKiCadTemplateConfig =
        serde_json::from_str(&template_source).map_err(|error| {
            format!(
                "failed to parse semantic KiCad template config {}: {error}",
                template_config_path.display()
            )
        })?;
    retain_semantic_connection_prefix(
        &mut active_problem,
        &mut active_template_config,
        target_rung,
    )?;

    fs::create_dir_all(output_directory).map_err(|error| {
        format!(
            "failed to create placement portfolio {}: {error}",
            output_directory.display()
        )
    })?;
    fs::copy(problem_path, output_directory.join("source-problem.json")).map_err(|error| {
        format!(
            "failed to snapshot source problem {}: {error}",
            problem_path.display()
        )
    })?;
    fs::copy(
        template_config_path,
        output_directory.join("source-template-config.json"),
    )
    .map_err(|error| {
        format!(
            "failed to snapshot template config {}: {error}",
            template_config_path.display()
        )
    })?;
    fs::copy(
        portfolio_config_path,
        output_directory.join("source-placement-portfolio.json"),
    )
    .map_err(|error| {
        format!(
            "failed to snapshot placement portfolio {}: {error}",
            portfolio_config_path.display()
        )
    })?;
    write_pretty_json_file(
        &output_directory.join("active-prefix-problem.json"),
        &active_problem,
    )?;
    write_pretty_json_file(
        &output_directory.join("template-config.json"),
        &active_template_config,
    )?;
    write_pretty_json_file(
        &output_directory.join("placement-portfolio.json"),
        &portfolio,
    )?;
    write_pretty_json_file(&output_directory.join("insertion-config.json"), insertion)?;

    let mut trial_inputs = Vec::new();
    for (entry_ordinal, entry) in portfolio.entries.iter().enumerate() {
        let started = Instant::now();
        if entry.proposal_finalists == 1 {
            let placement = pcb_placement::run_initial_placement(&active_problem, &entry.placement);
            append_semantic_kicad_placement_trial_inputs(
                &mut trial_inputs,
                &active_problem,
                entry,
                entry_ordinal,
                0,
                0,
                entry.id.clone(),
                placement,
                started.elapsed().as_micros() as u64,
            )?;
            continue;
        }

        let archive = pcb_placement::run_initial_placement_archive(
            &active_problem,
            &entry.placement,
            entry.proposal_finalists,
        )?;
        let placement_elapsed_micros = started.elapsed().as_micros() as u64;
        let archive_directory =
            output_directory.join(format!("entry-{entry_ordinal:03}-{}", entry.id));
        fs::create_dir_all(&archive_directory).map_err(|error| {
            format!(
                "failed to create placement archive {}: {error}",
                archive_directory.display()
            )
        })?;
        write_pretty_json_file(&archive_directory.join("placement-archive.json"), &archive)?;
        for (proposal_archive_rank, proposal_ordinal) in archive
            .retained_attempt_ordinals
            .iter()
            .copied()
            .enumerate()
        {
            let placement = archive
                .attempts
                .iter()
                .find(|attempt| attempt.ordinal == proposal_ordinal)
                .and_then(|attempt| attempt.placement.clone())
                .ok_or_else(|| {
                    format!(
                        "placement archive {:?} retained missing proposal {proposal_ordinal}",
                        entry.id
                    )
                })?;
            append_semantic_kicad_placement_trial_inputs(
                &mut trial_inputs,
                &active_problem,
                entry,
                entry_ordinal,
                proposal_ordinal,
                proposal_archive_rank,
                format!("{}-proposal-{proposal_ordinal:03}", entry.id),
                Ok(placement),
                placement_elapsed_micros,
            )?;
        }
        if archive.retained_attempt_ordinals.is_empty() {
            trial_inputs.push(SemanticKiCadPlacementTrialInput {
                entry_ordinal,
                proposal_ordinal: 0,
                proposal_archive_rank: 0,
                origin: SemanticKiCadPlacementTrialOrigin::InitialProposal,
                id: format!("{}-no-feasible-proposal", entry.id),
                placement_config: entry.placement.clone(),
                placement: Err(format!(
                    "placement archive {:?} retained no feasible proposal; inspect {}",
                    entry.id,
                    archive_directory.join("placement-archive.json").display()
                )),
                evaluated_poses: None,
                routing_feedback: None,
                placement_elapsed_micros,
                routing_feedback_elapsed_micros: 0,
            });
        }
    }

    let mut trials = Vec::with_capacity(trial_inputs.len());
    for (ordinal, input) in trial_inputs.into_iter().enumerate() {
        let trial_started = Instant::now();
        let trial_directory = output_directory.join(format!("trial-{ordinal:03}-{}", input.id));
        fs::create_dir_all(&trial_directory).map_err(|error| {
            format!(
                "failed to create placement trial {}: {error}",
                trial_directory.display()
            )
        })?;
        write_pretty_json_file(
            &trial_directory.join("placement-config.json"),
            &input.placement_config,
        )?;
        let routing_feedback = input.routing_feedback.as_ref().map(|feedback| {
            SemanticKiCadPlacementRoutingFeedbackEvidence {
                evidence: "routing-feedback.json".into(),
                strategy: feedback.evidence.strategy.clone(),
                semantic_complete: feedback.evidence.complete,
                selected_attempt: feedback.evidence.selected_attempt,
                attempted_repairs: feedback.evidence.attempted_repairs,
                total_expansions: feedback.evidence.total_expansions,
                routed_branches: feedback.evidence.routing.routed_branches,
                failed_branches: feedback.evidence.routing.failed_branches,
                moved_components: input.placement.as_ref().map_or_else(
                    |_| Vec::new(),
                    |placement| routing_feedback_moved_components(placement, feedback),
                ),
            }
        });
        if let Some(feedback) = &input.routing_feedback {
            write_pretty_json_file(&trial_directory.join("routing-feedback.json"), feedback)?;
        }
        let placement_elapsed_micros = input.placement_elapsed_micros;
        let routing_feedback_elapsed_micros = input.routing_feedback_elapsed_micros;
        let mut template_generation_elapsed_micros = 0;
        let mut initial_materialization_elapsed_micros = 0;
        let mut progression_elapsed_micros = 0;
        let evaluated = (|| {
            let placement = input.placement.clone()?;
            write_pretty_json_file(&trial_directory.join("placement.json"), &placement)?;
            let evaluated_poses = input
                .evaluated_poses
                .clone()
                .unwrap_or_else(|| placement.poses.clone());
            let evaluated_demand = pcb_placement::placement_demand_evidence(
                &active_problem,
                &evaluated_poses,
                [32, 32],
            )?;
            write_pretty_json_file(
                &trial_directory.join("evaluated-poses.json"),
                &evaluated_poses,
            )?;
            write_pretty_json_file(
                &trial_directory.join("evaluated-placement-demand.json"),
                &evaluated_demand,
            )?;
            let poses = evaluated_poses
                .iter()
                .map(|pose| pcb_kicad::SemanticKiCadPose {
                    component: pose.component.clone(),
                    position: pose.position,
                    rotation_degrees: pose.rotation_degrees,
                })
                .collect::<Vec<_>>();
            let started = Instant::now();
            let template = pcb_kicad::write_semantic_kicad_ladder_template(
                &active_problem,
                &poses,
                &active_template_config,
                &trial_directory.join("generated-ladder"),
            )?;
            template_generation_elapsed_micros = started.elapsed().as_micros() as u64;
            if template.electrical_connections != target_rung {
                return Err(format!(
                    "placement trial generated {} connections for requested rung {target_rung}",
                    template.electrical_connections
                ));
            }
            let declaration = pcb_kicad::load_declaration(&template.declaration)?;
            let started = Instant::now();
            let initial =
                pcb_kicad::materialize_rung(&declaration, 0, &trial_directory.join("initial"))?;
            initial_materialization_elapsed_micros = started.elapsed().as_micros() as u64;
            let started = Instant::now();
            let progression = if target_rung == 0 {
                None
            } else {
                Some(pcb_kicad::progress_kicad_connections(
                    &declaration,
                    &initial.output_directory,
                    &trial_directory.join("solve"),
                    &pcb_kicad::KiCadConnectionProgressionConfig {
                        maximum_connections: target_rung,
                        reuse_committed_parent_verification: true,
                        insertion: insertion.clone(),
                    },
                )?)
            };
            progression_elapsed_micros = started.elapsed().as_micros() as u64;
            let final_rung = progression
                .as_ref()
                .map_or(initial.rung, |result| result.final_rung);
            let final_directory = progression.as_ref().map_or_else(
                || initial.output_directory.clone(),
                |result| result.final_directory.clone(),
            );
            let completed = final_rung == target_rung;
            let quality = completed
                .then(|| {
                    measure_semantic_kicad_board_route_quality(
                        &declaration,
                        final_rung,
                        &final_directory,
                        portfolio.via_penalty_mm,
                    )
                })
                .transpose()?;
            Ok::<_, String>((
                placement,
                evaluated_poses,
                evaluated_demand,
                template,
                initial,
                progression,
                final_rung,
                final_directory,
                completed,
                quality,
            ))
        })();
        let timing = SemanticKiCadPlacementPortfolioTrialTiming {
            advisory_only: true,
            placement_elapsed_micros,
            routing_feedback_elapsed_micros,
            template_generation_elapsed_micros,
            initial_materialization_elapsed_micros,
            progression_elapsed_micros,
            total_elapsed_micros: trial_started.elapsed().as_micros() as u64,
        };
        let trial = match evaluated {
            Ok((
                placement,
                evaluated_poses,
                evaluated_demand,
                template,
                initial,
                progression,
                final_rung,
                final_directory,
                completed,
                board_route_quality,
            )) => SemanticKiCadPlacementPortfolioTrial {
                ordinal,
                entry_ordinal: input.entry_ordinal,
                proposal_ordinal: input.proposal_ordinal,
                proposal_archive_rank: input.proposal_archive_rank,
                origin: input.origin,
                id: input.id.clone(),
                directory: trial_directory,
                placement_config: input.placement_config.clone(),
                placement: Some(placement),
                evaluated_poses: Some(evaluated_poses),
                evaluated_demand: Some(evaluated_demand),
                routing_feedback,
                template: Some(template),
                initial: Some(initial),
                progression,
                final_rung,
                final_directory,
                completed,
                board_route_quality,
                error: None,
                selected: false,
                timing,
            },
            Err(error) => SemanticKiCadPlacementPortfolioTrial {
                ordinal,
                entry_ordinal: input.entry_ordinal,
                proposal_ordinal: input.proposal_ordinal,
                proposal_archive_rank: input.proposal_archive_rank,
                origin: input.origin,
                id: input.id.clone(),
                directory: trial_directory.clone(),
                placement_config: input.placement_config.clone(),
                placement: None,
                evaluated_poses: None,
                evaluated_demand: None,
                routing_feedback,
                template: None,
                initial: None,
                progression: None,
                final_rung: 0,
                final_directory: trial_directory,
                completed: false,
                board_route_quality: None,
                error: Some(error),
                selected: false,
                timing,
            },
        };
        trials.push(trial);
    }

    let selected = *ranked_semantic_kicad_placement_trial_indices(&trials)
        .first()
        .ok_or_else(|| "placement portfolio retained no trials".to_string())?;
    trials[selected].selected = true;
    for trial in &trials {
        write_pretty_json_file(&trial.directory.join("trial.json"), trial)?;
    }
    let selected_id = trials[selected].id.clone();
    let selected_rung = trials[selected].final_rung;
    let selected_directory = trials[selected].final_directory.clone();
    let completed = trials[selected].completed;
    let manifest = serde_json::json!({
        "schema_version": 3,
        "contract": "prune connectivity once; generate every placement independently; optionally retain a bounded cheap placement-proposal archive; optionally run bounded routing-failure feedback independently from each retained proposal; retain unmodified parents as configured controls; discard all semantic copper; materialize zero copper and reroute every enabled connection for every native trial; never share poses, copper, or routing decisions between trials; select native completion before route score",
        "target_rung": target_rung,
        "completed": completed,
        "selected_trial": selected,
        "selected_id": selected_id,
        "selected_final_rung": selected_rung,
        "selected_directory": selected_directory,
        "selection_rule": format!(
            "native completion, farther rung, total per-connection physical copper + {:.6} mm per via, ratsnest crossing count, squared demand, ordinal",
            portfolio.via_penalty_mm
        ),
        "source_snapshots": {
            "source_problem": "source-problem.json",
            "active_prefix_problem": "active-prefix-problem.json",
            "source_template_config": "source-template-config.json",
            "template_config": "template-config.json",
            "source_placement_portfolio": "source-placement-portfolio.json",
            "placement_portfolio": "placement-portfolio.json",
            "insertion_config": "insertion-config.json",
            "optional_placement_archives": "entry-*/placement-archive.json",
            "optional_routing_feedback": "trial-*/routing-feedback.json"
        },
        "trials": &trials,
        "timing": {
            "advisory_only": true,
            "total_to_manifest_elapsed_micros": portfolio_started.elapsed().as_micros() as u64
        }
    });
    write_pretty_json_file(
        &output_directory.join("cold-placement-portfolio.json"),
        &manifest,
    )?;
    Ok((
        trials[selected].id.clone(),
        selected_rung,
        trials[selected].final_directory.clone(),
        completed,
    ))
}

#[allow(clippy::too_many_arguments)]
fn run_semantic_kicad_order_trial(
    ordinal: usize,
    pending: PendingSemanticKiCadOrderTrial,
    problem_path: &Path,
    placement_policy: &str,
    source_template_config: &pcb_kicad::SemanticKiCadTemplateConfig,
    target_rung: usize,
    trial_directory: &Path,
    insertion: &pcb_kicad::KiCadConnectionInsertionConfig,
    via_penalty_mm: f64,
) -> SemanticKiCadOrderTrial {
    let started = Instant::now();
    let fingerprint = pcb_kicad::connection_order_fingerprint(&pending.connection_order);
    let mut trial = SemanticKiCadOrderTrial {
        ordinal,
        parent_trial: pending.parent_trial,
        order_fingerprint: fingerprint,
        connection_order: pending.connection_order.clone(),
        proposal: pending.proposal,
        directory: trial_directory.to_path_buf(),
        placement: None,
        template: None,
        initial: None,
        progression: None,
        final_rung: 0,
        final_directory: trial_directory.to_path_buf(),
        completed: false,
        board_route_quality: None,
        error: None,
        selected: false,
        elapsed_micros: 0,
    };
    let evaluated = (|| {
        fs::create_dir_all(trial_directory).map_err(|error| {
            format!(
                "failed to create connection-order trial {}: {error}",
                trial_directory.display()
            )
        })?;
        let mut proposed_template = source_template_config.clone();
        proposed_template.connection_order = pending.connection_order;
        let proposed_template_path = trial_directory.join("proposed-template-config.json");
        write_pretty_json_file(&proposed_template_path, &proposed_template)?;
        let (
            active_problem,
            placement_config,
            placement,
            active_template_config,
            template,
            generation_timing,
        ) = generate_semantic_kicad_ladder_from_inputs(
            problem_path,
            placement_policy,
            &proposed_template_path,
            Some(target_rung),
            &trial_directory.join("generated-ladder"),
        )?;
        write_pretty_json_file(
            &trial_directory.join("active-prefix-problem.json"),
            &active_problem,
        )?;
        write_pretty_json_file(
            &trial_directory.join("placement-config.json"),
            &placement_config,
        )?;
        write_pretty_json_file(&trial_directory.join("placement.json"), &placement)?;
        write_pretty_json_file(
            &trial_directory.join("active-template-config.json"),
            &active_template_config,
        )?;
        write_pretty_json_file(
            &trial_directory.join("generation-timing.json"),
            &generation_timing,
        )?;
        let declaration = pcb_kicad::load_declaration(&template.declaration)?;
        let initial =
            pcb_kicad::materialize_rung(&declaration, 0, &trial_directory.join("initial"))?;
        let progression = if target_rung == 0 {
            None
        } else {
            Some(pcb_kicad::progress_kicad_connections(
                &declaration,
                &initial.output_directory,
                &trial_directory.join("solve"),
                &pcb_kicad::KiCadConnectionProgressionConfig {
                    maximum_connections: target_rung,
                    reuse_committed_parent_verification: true,
                    insertion: insertion.clone(),
                },
            )?)
        };
        let final_rung = progression
            .as_ref()
            .map_or(initial.rung, |result| result.final_rung);
        let final_directory = progression.as_ref().map_or_else(
            || initial.output_directory.clone(),
            |result| result.final_directory.clone(),
        );
        let completed = final_rung == target_rung;
        let quality = completed
            .then(|| {
                measure_semantic_kicad_board_route_quality(
                    &declaration,
                    final_rung,
                    &final_directory,
                    via_penalty_mm,
                )
            })
            .transpose()?;
        Ok::<_, String>((
            placement,
            template,
            initial,
            progression,
            final_rung,
            final_directory,
            completed,
            quality,
        ))
    })();
    match evaluated {
        Ok((
            placement,
            template,
            initial,
            progression,
            final_rung,
            final_directory,
            completed,
            quality,
        )) => {
            trial.placement = Some(placement);
            trial.template = Some(template);
            trial.initial = Some(initial);
            trial.progression = progression;
            trial.final_rung = final_rung;
            trial.final_directory = final_directory;
            trial.completed = completed;
            trial.board_route_quality = quality;
        }
        Err(error) => trial.error = Some(error),
    }
    trial.elapsed_micros = started.elapsed().as_micros() as u64;
    trial
}

fn ranked_semantic_kicad_order_trial_indices(trials: &[SemanticKiCadOrderTrial]) -> Vec<usize> {
    let mut ranked = (0..trials.len()).collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        let left = &trials[*left];
        let right = &trials[*right];
        right
            .completed
            .cmp(&left.completed)
            .then_with(|| right.final_rung.cmp(&left.final_rung))
            .then_with(|| {
                left.board_route_quality
                    .as_ref()
                    .map_or(f64::INFINITY, |quality| quality.score_mm)
                    .total_cmp(
                        &right
                            .board_route_quality
                            .as_ref()
                            .map_or(f64::INFINITY, |quality| quality.score_mm),
                    )
            })
            .then_with(|| left.ordinal.cmp(&right.ordinal))
    });
    ranked
}

fn solve_semantic_kicad_order_portfolio(
    problem_path: &Path,
    placement_policy: &str,
    template_config_path: &Path,
    target_rung: usize,
    order_search_config_path: &Path,
    output_directory: &Path,
    insertion: &pcb_kicad::KiCadConnectionInsertionConfig,
) -> Result<(usize, std::path::PathBuf, bool), String> {
    if output_directory.exists() {
        return Err(format!(
            "refusing to overwrite cold connection-order portfolio {}",
            output_directory.display()
        ));
    }
    let started = Instant::now();
    let template_source = fs::read_to_string(template_config_path).map_err(|error| {
        format!(
            "failed to read semantic KiCad template config {}: {error}",
            template_config_path.display()
        )
    })?;
    let source_template_config: pcb_kicad::SemanticKiCadTemplateConfig =
        serde_json::from_str(&template_source).map_err(|error| {
            format!(
                "failed to parse semantic KiCad template config {}: {error}",
                template_config_path.display()
            )
        })?;
    if source_template_config.connection_order.is_empty()
        || target_rung > source_template_config.connection_order.len()
    {
        return Err(format!(
            "connection-order search target rung {target_rung} requires an explicit order with at least {target_rung} entries"
        ));
    }
    let config_source = fs::read_to_string(order_search_config_path).map_err(|error| {
        format!(
            "failed to read connection-order search config {}: {error}",
            order_search_config_path.display()
        )
    })?;
    let config: pcb_kicad::KiCadConnectionOrderSearchConfig = serde_json::from_str(&config_source)
        .map_err(|error| {
            format!(
                "failed to parse connection-order search config {}: {error}",
                order_search_config_path.display()
            )
        })?;
    config.check()?;

    fs::create_dir_all(output_directory).map_err(|error| {
        format!(
            "failed to create connection-order portfolio {}: {error}",
            output_directory.display()
        )
    })?;
    fs::copy(problem_path, output_directory.join("source-problem.json"))
        .map_err(|error| format!("failed to snapshot {}: {error}", problem_path.display()))?;
    fs::copy(
        template_config_path,
        output_directory.join("source-template-config.json"),
    )
    .map_err(|error| {
        format!(
            "failed to snapshot {}: {error}",
            template_config_path.display()
        )
    })?;
    fs::copy(
        order_search_config_path,
        output_directory.join("source-order-search-config.json"),
    )
    .map_err(|error| {
        format!(
            "failed to snapshot {}: {error}",
            order_search_config_path.display()
        )
    })?;
    write_pretty_json_file(&output_directory.join("order-search-config.json"), &config)?;
    write_pretty_json_file(&output_directory.join("insertion-config.json"), insertion)?;

    let base_order = source_template_config.connection_order.clone();
    let mut queued_fingerprints =
        BTreeSet::from([pcb_kicad::connection_order_fingerprint(&base_order)]);
    let mut queue = VecDeque::from([PendingSemanticKiCadOrderTrial {
        parent_trial: None,
        connection_order: base_order,
        proposal: None,
    }]);
    let mut trials = Vec::new();
    while trials.len() < config.maximum_trials {
        let Some(pending) = queue.pop_front() else {
            break;
        };
        let ordinal = trials.len();
        let trial_directory = output_directory.join(format!("trial-{ordinal:03}"));
        let trial = run_semantic_kicad_order_trial(
            ordinal,
            pending,
            problem_path,
            placement_policy,
            &source_template_config,
            target_rung,
            &trial_directory,
            insertion,
            config.via_penalty_mm,
        );
        let children = trial
            .progression
            .as_ref()
            .map_or_else(Vec::new, |progression| {
                pcb_kicad::diagnosed_connection_order_proposals(
                    &trial.connection_order,
                    progression,
                    &config,
                )
                .into_iter()
                .map(|proposal| PendingSemanticKiCadOrderTrial {
                    parent_trial: None,
                    connection_order: proposal.connection_order,
                    proposal: Some(proposal.evidence),
                })
                .collect()
            });
        let completed = trial.completed;
        trials.push(trial);
        write_pretty_json_file(&trial_directory.join("trial.json"), &trials[ordinal])?;
        if completed && config.stop_after_first_complete {
            break;
        }
        let mut retained_children = children
            .into_iter()
            .filter_map(|mut child| {
                let fingerprint = pcb_kicad::connection_order_fingerprint(&child.connection_order);
                queued_fingerprints.insert(fingerprint).then(|| {
                    child.parent_trial = Some(ordinal);
                    child
                })
            })
            .collect::<Vec<_>>();
        match config.frontier_policy {
            pcb_kicad::KiCadConnectionOrderFrontierPolicy::BreadthFirst => {
                queue.extend(retained_children)
            }
            pcb_kicad::KiCadConnectionOrderFrontierPolicy::DepthFirst => {
                while let Some(child) = retained_children.pop() {
                    queue.push_front(child);
                }
            }
        }
    }
    let selected = *ranked_semantic_kicad_order_trial_indices(&trials)
        .first()
        .ok_or_else(|| "connection-order portfolio evaluated no trials".to_string())?;
    trials[selected].selected = true;
    for trial in &trials {
        write_pretty_json_file(&trial.directory.join("trial.json"), trial)?;
    }
    let selected_rung = trials[selected].final_rung;
    let selected_directory = trials[selected].final_directory.clone();
    let completed = trials[selected].completed;
    let manifest = serde_json::json!({
        "schema_version": 1,
        "contract": "begin with the declared order; derive bounded order alternatives only from retained yielding diagnoses; regenerate placement and every route from zero copper for every order; retain all failed lineages; select native completion and farther rung before route quality",
        "target_rung": target_rung,
        "completed": completed,
        "selected_trial": selected,
        "selected_final_rung": selected_rung,
        "selected_directory": selected_directory,
        "evaluated_trials": trials.len(),
        "queued_but_unevaluated_trials": queue.len(),
        "selection_rule": format!(
            "native completion, farther rung, physical copper + {:.6} mm per via, stable trial ordinal",
            config.via_penalty_mm
        ),
        "source_snapshots": {
            "source_problem": "source-problem.json",
            "source_template_config": "source-template-config.json",
            "source_order_search_config": "source-order-search-config.json",
            "order_search_config": "order-search-config.json",
            "insertion_config": "insertion-config.json"
        },
        "trials": &trials,
        "elapsed_micros": started.elapsed().as_micros() as u64
    });
    write_pretty_json_file(
        &output_directory.join("cold-order-portfolio.json"),
        &manifest,
    )?;
    Ok((
        selected_rung,
        trials[selected].final_directory.clone(),
        completed,
    ))
}

fn generate_semantic_kicad_ladder_from_inputs(
    problem_path: &Path,
    placement_policy: &str,
    template_config_path: &Path,
    connection_prefix: Option<usize>,
    output_directory: &Path,
) -> Result<
    (
        layout_trace_model::Problem,
        pcb_placement::InitialPlacementConfig,
        pcb_placement::InitialPlacementResult,
        pcb_kicad::SemanticKiCadTemplateConfig,
        pcb_kicad::SemanticKiCadTemplateReport,
        SemanticKiCadGenerationTiming,
    ),
    String,
> {
    let total_started = Instant::now();
    let problem_started = Instant::now();
    let mut problem = load_layout_trace_problem(problem_path)?;
    let problem_loading_elapsed_micros = problem_started.elapsed().as_micros() as u64;
    let config_started = Instant::now();
    let placement_config = load_initial_placement_config(placement_policy)?;
    let config_source = fs::read_to_string(template_config_path).map_err(|error| {
        format!(
            "failed to read semantic KiCad template config {}: {error}",
            template_config_path.display()
        )
    })?;
    let template_config: pcb_kicad::SemanticKiCadTemplateConfig =
        serde_json::from_str(&config_source).map_err(|error| {
            format!(
                "failed to parse semantic KiCad template config {}: {error}",
                template_config_path.display()
            )
        })?;
    let mut template_config = template_config;
    if let Some(prefix) = connection_prefix {
        retain_semantic_connection_prefix(&mut problem, &mut template_config, prefix)?;
    }
    let config_and_prefix_elapsed_micros = config_started.elapsed().as_micros() as u64;
    let placement_started = Instant::now();
    let placement = pcb_placement::run_initial_placement(&problem, &placement_config)?;
    let placement_elapsed_micros = placement_started.elapsed().as_micros() as u64;
    let poses = placement
        .poses
        .iter()
        .map(|pose| pcb_kicad::SemanticKiCadPose {
            component: pose.component.clone(),
            position: pose.position,
            rotation_degrees: pose.rotation_degrees,
        })
        .collect::<Vec<_>>();
    let template_started = Instant::now();
    let report = pcb_kicad::write_semantic_kicad_ladder_template(
        &problem,
        &poses,
        &template_config,
        output_directory,
    )?;
    let template_generation_elapsed_micros = template_started.elapsed().as_micros() as u64;
    let timing = SemanticKiCadGenerationTiming {
        advisory_only: true,
        problem_loading_elapsed_micros,
        config_and_prefix_elapsed_micros,
        placement_elapsed_micros,
        template_generation_elapsed_micros,
        total_elapsed_micros: total_started.elapsed().as_micros() as u64,
    };
    Ok((
        problem,
        placement_config,
        placement,
        template_config,
        report,
        timing,
    ))
}

fn retain_semantic_connection_prefix(
    problem: &mut layout_trace_model::Problem,
    template_config: &mut pcb_kicad::SemanticKiCadTemplateConfig,
    prefix: usize,
) -> Result<(), String> {
    if template_config.connection_order.is_empty() {
        return Err(
            "cold semantic prefixes require an explicit template connection_order so placement and routing see the same selected nets"
                .into(),
        );
    }
    if prefix > template_config.connection_order.len() {
        return Err(format!(
            "target rung {prefix} exceeds the template connection_order's {} connections",
            template_config.connection_order.len()
        ));
    }
    template_config.connection_order.truncate(prefix);
    let selected = template_config
        .connection_order
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    problem
        .nets
        .retain(|net| selected.contains(net.electrical_net.as_deref().unwrap_or(net.id.as_str())));
    problem
        .electrical_nets
        .retain(|net| selected.contains(net.id.as_str()));
    problem.check_schema()?;
    Ok(())
}

fn load_kicad_connection_insertion_config(
    path: Option<&str>,
) -> Result<pcb_kicad::KiCadConnectionInsertionConfig, String> {
    let Some(path) = path else {
        return Ok(pcb_kicad::KiCadConnectionInsertionConfig::default());
    };
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read KiCad insertion config {path}: {error}"))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse KiCad insertion config {path}: {error}"))
}

fn write_pretty_json_file(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, format!("{serialized}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn resistor_field_frames() -> Result<Vec<Frame>, String> {
    resistor_field_frames_with_representation(
        TwoTerminalRepresentation::ExperimentalEndpointParticles,
    )
}

fn resistor_field_frames_with_representation(
    representation: TwoTerminalRepresentation,
) -> Result<Vec<Frame>, String> {
    let board = resistor_field_fixture();
    let mut compiled = compile_particle_world(
        &board,
        CompilePolicy {
            two_terminal_representation: representation,
            trace_sampling: TraceSamplingPolicy::PreserveSeedVertices,
            maximum_trace_spacing: 4.5,
            ..CompilePolicy::default()
        },
    )?;
    let mut backend = CpuReferenceBackend::new();
    let config = SolverConfig {
        field_strength: 10.0,
        target_density: 0.08,
        maximum_field_step: 0.12,
        projection_iterations: 1,
        ..SolverConfig::default()
    };
    let initial_config = SolverConfig {
        field_strength: 0.0,
        projection_iterations: 0,
        ..config
    };
    let mut frames = vec![backend.step(&mut compiled.world, &initial_config, 0)];
    for step in 1..=80 {
        let frame = backend.step(&mut compiled.world, &config, step);
        if step % 2 == 0 || step == 1 || step == 80 {
            frames.push(frame);
        }
    }
    Ok(frames)
}

fn resistor_representation_evidence(frames: &[Frame]) -> Result<serde_json::Value, String> {
    let initial = frames
        .first()
        .ok_or_else(|| "resistor ablation produced no initial frame".to_string())?;
    let final_frame = frames
        .last()
        .ok_or_else(|| "resistor ablation produced no final frame".to_string())?;
    let (initial_center, initial_angle) = resistor_pose(initial)?;
    let (final_center, final_angle) = resistor_pose(final_frame)?;
    let initial_particles = initial
        .particles
        .iter()
        .map(|particle| (particle.id, particle.position))
        .collect::<std::collections::HashMap<_, _>>();
    let maximum_particle_motion = frames
        .iter()
        .flat_map(|frame| &frame.particles)
        .filter_map(|particle| {
            initial_particles
                .get(&particle.id)
                .map(|start| particle.position.distance(*start))
        })
        .fold(0.0_f32, f32::max);
    Ok(serde_json::json!({
        "frames": frames.len(),
        "particle_count": final_frame.particles.len(),
        "body_count": final_frame.bodies.len(),
        "total_equality_constraint_count": final_frame.constraints.iter().filter(|constraint| constraint.family == "equality_distance").count(),
        "attachment_count": final_frame.attachments.len(),
        "projections_per_step": final_frame.metrics.constraint_projections,
        "scalar_constraint_rows_per_step": final_frame.metrics.constraint_scalar_rows,
        "initial_center": initial_center,
        "final_center": final_center,
        "center_displacement_mm": final_center.distance(initial_center),
        "initial_angle_radians": initial_angle,
        "final_angle_radians": final_angle,
        "rotation_radians": final_angle - initial_angle,
        "maximum_particle_motion_mm": maximum_particle_motion,
        "initial_maximum_residual_mm": initial.metrics.max_constraint_residual,
        "final_maximum_residual_mm": final_frame.metrics.max_constraint_residual
    }))
}

fn resistor_rotation_control_frames(
    representation: TwoTerminalRepresentation,
) -> Result<Vec<Frame>, String> {
    let board = Board {
        bounds: Bounds::new(Vec2::ZERO, Vec2::new(20.0, 20.0)),
        layers: vec![Layer { id: "top".into() }],
        rules: Rules { clearance: 0.2 },
        components: vec![Component {
            id: "R1".into(),
            position: Vec2::new(10.0, 10.0),
            rotation_radians: 0.0,
            kind: ComponentKind::TwoTerminalBody {
                length: 4.0,
                radius: 0.8,
            },
            mobility: Mobility::FREE,
            terminals: vec![
                Terminal {
                    id: "1".into(),
                    local_position: Vec2::new(-2.0, 0.0),
                },
                Terminal {
                    id: "2".into(),
                    local_position: Vec2::new(2.0, 0.0),
                },
            ],
        }],
        connections: vec![],
    };
    let mut compiled = compile_particle_world(
        &board,
        CompilePolicy {
            two_terminal_representation: representation,
            ..CompilePolicy::default()
        },
    )?;
    let pulled_terminal = compiled.terminal_particles[&("R1".into(), "2".into())] as usize;
    let mut backend = CpuReferenceBackend::with_field(NoField);
    let initial_config = SolverConfig {
        projection_iterations: 0,
        ..SolverConfig::default()
    };
    let mut frames = vec![backend.step(&mut compiled.world, &initial_config, 0)];
    compiled
        .world
        .particles
        .set_position(pulled_terminal, Vec2::new(12.0, 3.0));
    let config = SolverConfig {
        projection_iterations: 1,
        ..SolverConfig::default()
    };
    for step in 1..=30 {
        frames.push(backend.step(&mut compiled.world, &config, step));
    }
    Ok(frames)
}

fn resistor_pose(frame: &Frame) -> Result<(Vec2, f32), String> {
    if let Some(body) = frame.bodies.iter().find(|body| body.label == "R1") {
        return Ok((body.position, body.angle_radians));
    }
    let mut terminals = frame
        .particles
        .iter()
        .filter(|particle| particle.label.starts_with("R1:"))
        .collect::<Vec<_>>();
    terminals.sort_by(|left, right| left.label.cmp(&right.label));
    if terminals.len() != 2 {
        return Err(format!(
            "expected two R1 terminal particles, found {}",
            terminals.len()
        ));
    }
    let first = terminals[0].position;
    let second = terminals[1].position;
    Ok((
        (first + second) * 0.5,
        (second.y - first.y).atan2(second.x - first.x),
    ))
}

fn resistor_field_fixture() -> Board {
    let terminal = |id: &str, x: f32| Terminal {
        id: id.into(),
        local_position: Vec2::new(x, 0.0),
    };
    let terminal_ref = |component: &str, terminal: &str| TerminalRef {
        component: component.into(),
        terminal: terminal.into(),
    };
    Board {
        bounds: Bounds::new(Vec2::ZERO, Vec2::new(40.0, 24.0)),
        layers: vec![Layer { id: "top".into() }],
        rules: Rules { clearance: 0.4 },
        components: vec![
            Component {
                id: "LEFT".into(),
                position: Vec2::new(3.0, 12.0),
                rotation_radians: 0.0,
                kind: ComponentKind::Anchor,
                mobility: Mobility::FIXED,
                terminals: vec![terminal("1", 0.0)],
            },
            Component {
                id: "R1".into(),
                position: Vec2::new(20.0, 10.8),
                rotation_radians: 0.0,
                kind: ComponentKind::TwoTerminalBody {
                    length: 4.0,
                    radius: 1.0,
                },
                mobility: Mobility::FREE,
                terminals: vec![terminal("1", -2.0), terminal("2", 2.0)],
            },
            Component {
                id: "RIGHT".into(),
                position: Vec2::new(37.0, 12.0),
                rotation_radians: 0.0,
                kind: ComponentKind::Anchor,
                mobility: Mobility::FIXED,
                terminals: vec![terminal("1", 0.0)],
            },
            Component {
                id: "TOP_BLOCK".into(),
                position: Vec2::new(20.0, 6.0),
                rotation_radians: 0.0,
                kind: ComponentKind::Rect {
                    size: Vec2::new(10.0, 4.0),
                    routing_keepout: true,
                },
                mobility: Mobility::FIXED,
                terminals: vec![],
            },
            Component {
                id: "BOTTOM_BLOCK".into(),
                position: Vec2::new(20.0, 18.0),
                rotation_radians: 0.0,
                kind: ComponentKind::Rect {
                    size: Vec2::new(10.0, 4.0),
                    routing_keepout: true,
                },
                mobility: Mobility::FIXED,
                terminals: vec![],
            },
        ],
        connections: vec![
            Connection {
                id: "LEFT_TO_R1".into(),
                terminals: vec![terminal_ref("LEFT", "1"), terminal_ref("R1", "1")],
                layer: 0,
                width: 0.8,
                seed_route: vec![
                    Vec2::new(3.0, 12.0),
                    Vec2::new(8.0, 9.0),
                    Vec2::new(13.0, 10.0),
                    Vec2::new(18.0, 10.8),
                ],
            },
            Connection {
                id: "R1_TO_RIGHT".into(),
                terminals: vec![terminal_ref("R1", "2"), terminal_ref("RIGHT", "1")],
                layer: 0,
                width: 0.8,
                seed_route: vec![
                    Vec2::new(22.0, 10.8),
                    Vec2::new(27.0, 14.5),
                    Vec2::new(32.0, 15.0),
                    Vec2::new(37.0, 12.0),
                ],
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn diagnostic_contains_visible_motion_and_an_unresolved_constraint() {
        let frames = resistor_field_frames().unwrap();
        let initial: HashMap<_, _> = frames[0]
            .particles
            .iter()
            .map(|particle| (particle.id, particle.position))
            .collect();
        let maximum_motion = frames
            .iter()
            .flat_map(|frame| &frame.particles)
            .map(|particle| particle.position.distance(initial[&particle.id]))
            .fold(0.0, f32::max);

        assert_eq!(frames[0].metrics.max_displacement, 0.0);
        assert!(maximum_motion > 2.0);
        assert!(frames.last().unwrap().metrics.max_constraint_residual > 1.0);
    }

    #[test]
    fn resistor_ablation_exposes_distinct_work_and_pose_evidence() {
        let endpoint = resistor_field_frames_with_representation(
            TwoTerminalRepresentation::ExperimentalEndpointParticles,
        )
        .unwrap();
        let rigid = resistor_field_frames_with_representation(
            TwoTerminalRepresentation::AnalyticRigidBodyAttachments,
        )
        .unwrap();
        let endpoint_evidence = resistor_representation_evidence(&endpoint).unwrap();
        let rigid_evidence = resistor_representation_evidence(&rigid).unwrap();

        assert_eq!(endpoint_evidence["body_count"], 0);
        assert_eq!(rigid_evidence["body_count"], 1);
        assert_eq!(rigid_evidence["attachment_count"], 2);
        assert_eq!(
            endpoint_evidence["total_equality_constraint_count"]
                .as_u64()
                .unwrap(),
            rigid_evidence["total_equality_constraint_count"]
                .as_u64()
                .unwrap()
                + 1
        );
        assert_ne!(
            endpoint_evidence["projections_per_step"],
            rigid_evidence["projections_per_step"]
        );
        assert!(rigid_evidence["rotation_radians"].as_f64().unwrap().abs() > 0.001);

        let endpoint_rotation = resistor_rotation_control_frames(
            TwoTerminalRepresentation::ExperimentalEndpointParticles,
        )
        .unwrap();
        let rigid_rotation = resistor_rotation_control_frames(
            TwoTerminalRepresentation::AnalyticRigidBodyAttachments,
        )
        .unwrap();
        assert!(
            resistor_representation_evidence(&endpoint_rotation).unwrap()["rotation_radians"]
                .as_f64()
                .unwrap()
                .abs()
                > 0.5
        );
        assert!(
            resistor_representation_evidence(&rigid_rotation).unwrap()["rotation_radians"]
                .as_f64()
                .unwrap()
                .abs()
                > 0.2
        );
    }

    #[test]
    fn placement_policy_names_are_explicit() {
        for name in ["declared", "grid", "random", "barycentric", "harmonic"] {
            initial_placement_config(name).unwrap();
        }
        assert!(initial_placement_config("best").is_err());
        assert!(
            load_initial_placement_config("experiments/configs/harmonic-standard.json").is_ok()
        );
    }

    #[test]
    fn cold_prefix_prunes_connectivity_before_placement_without_removing_components() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut problem = load_layout_trace_problem(
            &repository.join("benchmarks/imported/layout-trace/dual-esp32-benchmark.json"),
        )
        .unwrap();
        let original_components = problem.components.len();
        let config_source = fs::read_to_string(
            repository.join("benchmarks/dual-esp32-ladder/template-config.json"),
        )
        .unwrap();
        let mut config: pcb_kicad::SemanticKiCadTemplateConfig =
            serde_json::from_str(&config_source).unwrap();

        retain_semantic_connection_prefix(&mut problem, &mut config, 2).unwrap();

        assert_eq!(problem.components.len(), original_components);
        assert_eq!(config.connection_order, ["SIG01_W_R", "SIG01_AFTER_LINK"]);
        assert_eq!(problem.nets.len(), 3);
        assert!(problem.nets.iter().all(|net| {
            matches!(
                net.electrical_net.as_deref().unwrap_or(&net.id),
                "SIG01_W_R" | "SIG01_AFTER_LINK"
            )
        }));
        assert!(problem.electrical_nets.is_empty());
    }

    #[test]
    fn failed_feedback_prefers_an_equally_complete_base_fallback() {
        assert!(select_base_placement_fallback(11, Some(15)));
        assert!(select_base_placement_fallback(11, Some(11)));
        assert!(!select_base_placement_fallback(11, Some(10)));
        assert!(!select_base_placement_fallback(11, None));
    }

    #[test]
    fn placement_portfolio_is_explicitly_bounded_and_has_unique_labels() {
        let entry = |id: &str| SemanticKiCadPlacementPortfolioEntry {
            id: id.into(),
            placement: pcb_placement::InitialPlacementConfig::default(),
            proposal_finalists: 1,
            routing_feedback: None,
        };
        let valid = SemanticKiCadPlacementPortfolioConfig {
            maximum_entries: 2,
            maximum_trials: None,
            via_penalty_mm: 2.0,
            entries: vec![entry("declared"), entry("harmonic")],
        };
        validate_semantic_kicad_placement_portfolio_config(&valid).unwrap();

        let mut too_many = valid.clone();
        too_many.maximum_entries = 1;
        assert!(validate_semantic_kicad_placement_portfolio_config(&too_many).is_err());
        let mut duplicate = valid.clone();
        duplicate.entries[1].id = "declared".into();
        assert!(validate_semantic_kicad_placement_portfolio_config(&duplicate).is_err());
        let mut unsafe_label = valid;
        unsafe_label.entries[1].id = "../harmonic".into();
        assert!(validate_semantic_kicad_placement_portfolio_config(&unsafe_label).is_err());

        let archived = SemanticKiCadPlacementPortfolioConfig {
            maximum_entries: 1,
            maximum_trials: Some(2),
            via_penalty_mm: 2.0,
            entries: vec![SemanticKiCadPlacementPortfolioEntry {
                id: "random-archive".into(),
                placement: pcb_placement::InitialPlacementConfig::default(),
                proposal_finalists: 2,
                routing_feedback: None,
            }],
        };
        validate_semantic_kicad_placement_portfolio_config(&archived).unwrap();
        let mut insufficient_trial_bound = archived.clone();
        insufficient_trial_bound.maximum_trials = Some(1);
        assert!(
            validate_semantic_kicad_placement_portfolio_config(&insufficient_trial_bound).is_err()
        );
        let mut zero_finalists = archived;
        zero_finalists.entries[0].proposal_finalists = 0;
        assert!(validate_semantic_kicad_placement_portfolio_config(&zero_finalists).is_err());

        let feedback = SemanticKiCadPlacementPortfolioConfig {
            maximum_entries: 1,
            maximum_trials: Some(4),
            via_penalty_mm: 2.0,
            entries: vec![SemanticKiCadPlacementPortfolioEntry {
                id: "feedback-archive".into(),
                placement: pcb_placement::InitialPlacementConfig::default(),
                proposal_finalists: 2,
                routing_feedback: Some(SemanticKiCadPlacementRoutingFeedbackConfig {
                    routing: pcb_routing::DutGridRoutingConfig::default(),
                    pressure: pcb_coordinator::PressureRepairConfig::default(),
                    retain_parent_trial: true,
                }),
            }],
        };
        validate_semantic_kicad_placement_portfolio_config(&feedback).unwrap();
        let mut insufficient_feedback_bound = feedback;
        insufficient_feedback_bound.maximum_trials = Some(3);
        assert!(
            validate_semantic_kicad_placement_portfolio_config(&insufficient_feedback_bound)
                .is_err()
        );
    }

    #[test]
    fn old_placement_portfolio_json_defaults_to_one_finalist_per_entry() {
        let config: SemanticKiCadPlacementPortfolioConfig =
            serde_json::from_value(serde_json::json!({
                "maximum_entries": 1,
                "via_penalty_mm": 2.0,
                "entries": [{
                    "id": "declared",
                    "placement": {"policy": {"kind": "declared"}, "seed": 0, "projection_sweeps": 8}
                }]
            }))
            .unwrap();
        assert_eq!(config.maximum_trials, None);
        assert_eq!(config.entries[0].proposal_finalists, 1);
        assert!(config.entries[0].routing_feedback.is_none());
        validate_semantic_kicad_placement_portfolio_config(&config).unwrap();
    }

    #[test]
    fn placement_portfolio_composes_a_feedback_child_with_its_parent() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
        let problem =
            load_layout_trace_problem(&repository.join(
                "benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json",
            ))
            .unwrap();
        let placement_config = pcb_placement::InitialPlacementConfig::default();
        let placement = pcb_placement::run_initial_placement(&problem, &placement_config).unwrap();
        let entry = SemanticKiCadPlacementPortfolioEntry {
            id: "wall".into(),
            placement: placement_config,
            proposal_finalists: 1,
            routing_feedback: Some(SemanticKiCadPlacementRoutingFeedbackConfig {
                routing: pcb_routing::DutGridRoutingConfig {
                    retry_grid_mm: vec![0.25, 0.1],
                    max_expansions_per_search: 5_000_000,
                    ..pcb_routing::DutGridRoutingConfig::default()
                },
                pressure: pcb_coordinator::PressureRepairConfig::default(),
                retain_parent_trial: true,
            }),
        };
        let mut inputs = Vec::new();
        append_semantic_kicad_placement_trial_inputs(
            &mut inputs,
            &problem,
            &entry,
            0,
            0,
            0,
            "wall".into(),
            Ok(placement),
            0,
        )
        .unwrap();

        assert_eq!(inputs.len(), 2);
        assert!(matches!(
            inputs[0].origin,
            SemanticKiCadPlacementTrialOrigin::InitialProposal
        ));
        assert!(matches!(
            inputs[1].origin,
            SemanticKiCadPlacementTrialOrigin::RoutingFeedback
        ));
        assert!(inputs[0].routing_feedback.is_none());
        assert!(
            inputs[1]
                .routing_feedback
                .as_ref()
                .is_some_and(|feedback| feedback.evidence.complete)
        );
        assert!(!same_semantic_placement_poses(
            &inputs[1].placement.as_ref().unwrap().poses,
            inputs[1].evaluated_poses.as_ref().unwrap(),
        ));
    }

    #[test]
    fn placement_portfolio_retains_noop_feedback_without_duplicate_native_trial() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut problem =
            load_layout_trace_problem(&repository.join(
                "benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json",
            ))
            .unwrap();
        problem.nets.clear();
        problem.electrical_nets.clear();
        let placement_config = pcb_placement::InitialPlacementConfig::default();
        let placement = pcb_placement::run_initial_placement(&problem, &placement_config).unwrap();
        let entry = SemanticKiCadPlacementPortfolioEntry {
            id: "no-connections".into(),
            placement: placement_config,
            proposal_finalists: 1,
            routing_feedback: Some(SemanticKiCadPlacementRoutingFeedbackConfig {
                routing: pcb_routing::DutGridRoutingConfig::default(),
                pressure: pcb_coordinator::PressureRepairConfig::default(),
                retain_parent_trial: true,
            }),
        };
        let mut inputs = Vec::new();
        append_semantic_kicad_placement_trial_inputs(
            &mut inputs,
            &problem,
            &entry,
            0,
            0,
            0,
            "no-connections".into(),
            Ok(placement),
            0,
        )
        .unwrap();

        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].evaluated_poses.is_none());
        assert!(
            inputs[0]
                .routing_feedback
                .as_ref()
                .is_some_and(|feedback| feedback.evidence.complete)
        );
    }

    #[test]
    fn placement_portfolio_ranks_native_completion_then_rung_then_route_score() {
        let trial = |ordinal: usize, completed: bool, rung: usize, score: Option<f64>| {
            SemanticKiCadPlacementPortfolioTrial {
                ordinal,
                entry_ordinal: ordinal,
                proposal_ordinal: 0,
                proposal_archive_rank: 0,
                origin: SemanticKiCadPlacementTrialOrigin::InitialProposal,
                id: format!("trial-{ordinal}"),
                directory: std::path::PathBuf::from(format!("trial-{ordinal}")),
                placement_config: pcb_placement::InitialPlacementConfig::default(),
                placement: None,
                evaluated_poses: None,
                evaluated_demand: None,
                routing_feedback: None,
                template: None,
                initial: None,
                progression: None,
                final_rung: rung,
                final_directory: std::path::PathBuf::from(format!("trial-{ordinal}/final")),
                completed,
                board_route_quality: score.map(|score_mm| SemanticKiCadBoardRouteQuality {
                    connections: rung,
                    length_mm: score_mm,
                    segments: 0,
                    stored_track_length_mm: score_mm,
                    stored_segments: 0,
                    overlapping_track_length_mm: 0.0,
                    vias: 0,
                    close_via_pairs_within_connections: 0,
                    clustered_vias_within_connections: 0,
                    score_mm,
                    interpretation: String::new(),
                }),
                error: None,
                selected: false,
                timing: SemanticKiCadPlacementPortfolioTrialTiming {
                    advisory_only: true,
                    placement_elapsed_micros: 0,
                    routing_feedback_elapsed_micros: 0,
                    template_generation_elapsed_micros: 0,
                    initial_materialization_elapsed_micros: 0,
                    progression_elapsed_micros: 0,
                    total_elapsed_micros: 0,
                },
            }
        };
        let trials = vec![
            trial(0, false, 8, None),
            trial(1, true, 7, Some(20.0)),
            trial(2, true, 7, Some(18.0)),
            trial(3, false, 9, None),
        ];
        assert_eq!(
            ranked_semantic_kicad_placement_trial_indices(&trials),
            vec![2, 1, 3, 0]
        );
    }

    #[test]
    fn order_portfolio_ranks_completion_and_reach_before_quality() {
        let trial = |ordinal: usize, completed: bool, rung: usize, score: Option<f64>| {
            SemanticKiCadOrderTrial {
                ordinal,
                parent_trial: None,
                order_fingerprint: format!("order-{ordinal}"),
                connection_order: vec![format!("N{ordinal}")],
                proposal: None,
                directory: std::path::PathBuf::from(format!("trial-{ordinal}")),
                placement: None,
                template: None,
                initial: None,
                progression: None,
                final_rung: rung,
                final_directory: std::path::PathBuf::from(format!("trial-{ordinal}/final")),
                completed,
                board_route_quality: score.map(|score_mm| SemanticKiCadBoardRouteQuality {
                    connections: rung,
                    length_mm: score_mm,
                    segments: 0,
                    stored_track_length_mm: score_mm,
                    stored_segments: 0,
                    overlapping_track_length_mm: 0.0,
                    vias: 0,
                    close_via_pairs_within_connections: 0,
                    clustered_vias_within_connections: 0,
                    score_mm,
                    interpretation: String::new(),
                }),
                error: None,
                selected: false,
                elapsed_micros: 0,
            }
        };
        let trials = vec![
            trial(0, false, 14, None),
            trial(1, true, 18, Some(500.0)),
            trial(2, true, 18, Some(490.0)),
            trial(3, false, 17, None),
        ];
        assert_eq!(
            ranked_semantic_kicad_order_trial_indices(&trials),
            vec![2, 1, 3, 0]
        );
    }
}

/// A router configuration from a file, or resolved from the KiCad project
/// when the path is absent or `auto`.
fn board_router_config(
    path: Option<&str>,
    source: &str,
    board_id: &str,
) -> Result<pcb_kicad::KiCadBoardRouterConfig, String> {
    match path {
        None | Some("auto") => pcb_kicad::resolve_project_rules(
            &Path::new(source).join(format!("{board_id}.kicad_pro")),
            &Path::new(source).join(format!("{board_id}.kicad_pcb")),
        ),
        Some(path) => pcb_kicad::KiCadBoardRouterConfig::from_json(
            serde_json::from_str(
                &std::fs::read_to_string(path)
                    .map_err(|error| format!("failed to read {path}: {error}"))?,
            )
            .map_err(|error| format!("failed to parse {path}: {error}"))?,
        ),
    }
}

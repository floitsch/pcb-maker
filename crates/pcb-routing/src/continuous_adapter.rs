// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{
    Problem, Vec2,
    geometry::{add, rotate_degrees},
    model::{Movement, PinRef, Rotation},
    topology::{RouteClass, TerminalSector},
};
use pcb_core::{
    Board as EngineBoard, Component as EngineComponent, ComponentKind, Connection,
    ContinuousSolution, Layer, Mobility, Rules as EngineRules, Terminal, TerminalRef,
    Vec2 as EngineVec2,
};
use pcb_engine::{CompilePolicy, CompiledWorld, compile_particle_world};
use pcb_validate::{
    CANDIDATE_SCHEMA_VERSION, CandidateArtifact, SolvedComponent, SolvedRouteGraph,
    SolvedRouteNode, SolvedRouteNodeKind, SolvedTrace,
};

/// Semantic adapter around a compiled continuous world. The engine stays
/// unaware of layout-trace schemas and exact-validator artifact types.
#[derive(Clone, Debug)]
pub struct CompiledContinuousProblem {
    pub compiled: CompiledWorld,
    source: Problem,
    branches: Vec<ContinuousBranch>,
}

#[derive(Clone, Debug)]
struct ContinuousBranch {
    id: String,
    electrical_net: String,
    width: f64,
    tension_weight: f64,
    layer: String,
    from: PinRef,
    to: PinRef,
    seed_route: Vec<Vec2>,
}

/// Lower the currently supported layout-trace subset into the experimental
/// continuous engine. Unsupported semantics fail closed rather than being
/// approximated silently.
pub fn compile_problem_for_continuous_engine(
    problem: &Problem,
    policy: CompilePolicy,
) -> Result<CompiledContinuousProblem, String> {
    problem.check_schema()?;
    if !problem.placement_constraints.is_empty() {
        return Err(
            "continuous adapter does not yet compile relational placement constraints".into(),
        );
    }
    let inverse_mass = checked_f32(problem.solver.component_mobility, "component mobility")?;
    let rotation_mobility = checked_f32(
        problem.solver.rotation_mobility,
        "component rotation mobility",
    )?;
    let layers = problem
        .board
        .layers
        .iter()
        .map(|layer| Layer {
            id: layer.id.clone(),
        })
        .collect::<Vec<_>>();
    let layer_indices = layers
        .iter()
        .enumerate()
        .map(|(index, layer)| (layer.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let components = problem
        .components
        .iter()
        .map(|component| {
            let (translate_x, translate_y) = match component.constraints.movement {
                Movement::Fixed => (false, false),
                Movement::Horizontal => (true, false),
                Movement::Vertical => (false, true),
                Movement::Free => (true, true),
            };
            let rotate = component.constraints.rotation == Rotation::Free;
            let mobility = Mobility {
                inverse_mass: if translate_x || translate_y {
                    inverse_mass
                } else {
                    0.0
                },
                rotation_mobility: if rotate { rotation_mobility } else { 0.0 },
                translate_x,
                translate_y,
                rotate,
            };
            let terminals = component
                .pins
                .iter()
                .map(|pin| {
                    Ok(Terminal {
                        id: pin.id.clone(),
                        local_position: engine_vec(pin.offset, "pin offset")?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let kind = match component.pins.len() {
                0 => ComponentKind::Rect {
                    size: engine_vec(component.size, "component size")?,
                    routing_keepout: component.body_is_routing_keepout,
                },
                1 if component.constraints.movement == Movement::Fixed
                    && component.constraints.rotation == Rotation::Fixed =>
                {
                    ComponentKind::Anchor
                }
                2 if policy.two_terminal_representation
                    != pcb_engine::TwoTerminalRepresentation::Unspecified
                    && supported_two_terminal_kind(component).is_some() =>
                {
                    supported_two_terminal_kind(component)
                        .expect("checked supported two-terminal geometry")
                }
                _ => ComponentKind::Rect {
                    size: engine_vec(component.size, "component size")?,
                    routing_keepout: component.body_is_routing_keepout,
                },
            };
            Ok(EngineComponent {
                id: component.id.clone(),
                position: engine_vec(component.position, "component position")?,
                rotation_radians: checked_f32(
                    component.rotation_degrees.to_radians(),
                    "component rotation",
                )?,
                kind,
                mobility,
                terminals,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let branches = continuous_branches(problem)?;
    let connections = branches
        .iter()
        .map(|branch| {
            let layer = *layer_indices.get(branch.layer.as_str()).ok_or_else(|| {
                format!("branch {} uses unknown layer {}", branch.id, branch.layer)
            })?;
            let seed_route = if branch.seed_route.len() >= 2 {
                branch
                    .seed_route
                    .iter()
                    .map(|point| engine_vec(*point, "route seed"))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                vec![
                    engine_vec(
                        declared_pin_position(problem, &branch.from)?,
                        "route endpoint",
                    )?,
                    engine_vec(
                        declared_pin_position(problem, &branch.to)?,
                        "route endpoint",
                    )?,
                ]
            };
            Ok(Connection {
                id: branch.id.clone(),
                terminals: vec![terminal_ref(&branch.from), terminal_ref(&branch.to)],
                layer,
                width: checked_f32(branch.width, "trace width")?,
                seed_route,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let board = EngineBoard {
        bounds: pcb_core::Bounds::new(
            engine_vec(problem.board.bounds.min, "board minimum")?,
            engine_vec(problem.board.bounds.max, "board maximum")?,
        ),
        layers,
        rules: EngineRules {
            clearance: checked_f32(problem.rules.clearance, "clearance")?,
        },
        components,
        connections,
    };
    let compiled = compile_particle_world(&board, policy)?;
    Ok(CompiledContinuousProblem {
        compiled,
        source: problem.clone(),
        branches,
    })
}

impl CompiledContinuousProblem {
    pub fn solution(&self) -> Result<ContinuousSolution, String> {
        self.compiled.solution()
    }

    /// Convert representation-independent engine state into the common
    /// candidate artifact. This does not claim legality; callers must pass the
    /// result through `pcb_validate::validate_candidate`.
    pub fn materialize_candidate(&self) -> Result<CandidateArtifact, String> {
        let problem = &self.source;
        problem.check_schema()?;
        let solution = self.solution()?;
        let solution_poses = unique_solution_poses(&solution)?;
        let components = problem
            .components
            .iter()
            .map(|declared| {
                let solved = solution_poses.get(declared.id.as_str()).ok_or_else(|| {
                    format!("continuous solution omitted component {}", declared.id)
                })?;
                let mut position = Vec2::new(solved.position.x as f64, solved.position.y as f64);
                match declared.constraints.movement {
                    Movement::Fixed => position = declared.position,
                    Movement::Horizontal => position.y = declared.position.y,
                    Movement::Vertical => position.x = declared.position.x,
                    Movement::Free => {}
                }
                let rotation_degrees = if declared.constraints.rotation == Rotation::Fixed {
                    declared.rotation_degrees
                } else {
                    (solved.rotation_radians as f64)
                        .to_degrees()
                        .rem_euclid(360.0)
                };
                Ok(SolvedComponent {
                    id: declared.id.clone(),
                    position,
                    size: declared.size,
                    rotation_degrees,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if solution_poses.len() != components.len() {
            return Err("continuous solution contains unknown or duplicate components".into());
        }
        let poses = components
            .iter()
            .map(|component| (component.id.as_str(), component))
            .collect::<BTreeMap<_, _>>();
        let connections = solution
            .connections
            .iter()
            .map(|connection| (connection.id.as_str(), connection))
            .collect::<BTreeMap<_, _>>();
        if connections.len() != solution.connections.len() {
            return Err("continuous solution contains duplicate connection IDs".into());
        }
        let mut traces = Vec::with_capacity(self.branches.len());
        for branch in &self.branches {
            let connection = connections
                .get(branch.id.as_str())
                .ok_or_else(|| format!("continuous solution omitted connection {}", branch.id))?;
            if connection.points.len() < 2 {
                return Err(format!(
                    "connection {} has fewer than two points",
                    branch.id
                ));
            }
            let mut points = connection
                .points
                .iter()
                .map(|point| Vec2::new(point.x as f64, point.y as f64))
                .collect::<Vec<_>>();
            points[0] = solved_pin_position(problem, &poses, &branch.from)?;
            let last = points.len() - 1;
            points[last] = solved_pin_position(problem, &poses, &branch.to)?;
            if points
                .iter()
                .any(|point| !point.x.is_finite() || !point.y.is_finite())
            {
                return Err(format!("connection {} has a non-finite point", branch.id));
            }
            let from_node = terminal_node_id(&branch.electrical_net, &branch.from);
            let to_node = terminal_node_id(&branch.electrical_net, &branch.to);
            traces.push(SolvedTrace {
                branch: branch.id.clone(),
                electrical_net: branch.electrical_net.clone(),
                net: branch.id.clone(),
                from_node,
                to_node,
                width: branch.width,
                tension_weight: branch.tension_weight,
                layer: branch.layer.clone(),
                segment_layers: vec![branch.layer.clone(); points.len() - 1],
                vias: Vec::new(),
                points,
                route_class: RouteClass::direct(
                    branch.layer.clone(),
                    TerminalSector {
                        component: branch.from.component.clone(),
                        sector: branch.from.pin.clone(),
                    },
                    TerminalSector {
                        component: branch.to.component.clone(),
                        sector: branch.to.pin.clone(),
                    },
                ),
                route_basis_fingerprint: None,
            });
        }
        if connections.len() != traces.len() {
            return Err("continuous solution contains an unknown connection".into());
        }
        traces.sort_by(|left, right| left.branch.cmp(&right.branch));
        let route_graphs = materialize_route_graphs(&traces, &self.branches, problem, &poses)?;
        Ok(CandidateArtifact {
            schema_version: CANDIDATE_SCHEMA_VERSION,
            components,
            traces,
            route_graphs,
        })
    }
}

fn continuous_branches(problem: &Problem) -> Result<Vec<ContinuousBranch>, String> {
    let mut branches = Vec::new();
    let mut branch_ids = BTreeSet::new();
    for net in &problem.nets {
        if !branch_ids.insert(net.id.clone()) {
            return Err(format!("duplicate continuous branch ID {}", net.id));
        }
        branches.push(ContinuousBranch {
            id: net.id.clone(),
            electrical_net: net.electrical_net.clone().unwrap_or_else(|| net.id.clone()),
            width: net.width,
            tension_weight: net.tension_weight,
            layer: net.layer.clone(),
            from: net.from.clone(),
            to: net.to.clone(),
            seed_route: net.seed_route.clone(),
        });
    }
    for net in &problem.electrical_nets {
        let mut terminals = net.terminals.clone();
        terminals.sort_by(|left, right| {
            (left.component.as_str(), left.pin.as_str())
                .cmp(&(right.component.as_str(), right.pin.as_str()))
        });
        let positions = terminals
            .iter()
            .map(|terminal| declared_pin_position(problem, terminal))
            .collect::<Result<Vec<_>, _>>()?;
        let mut connected = vec![false; terminals.len()];
        connected[0] = true;
        for branch_index in 0..terminals.len() - 1 {
            let mut best: Option<(f64, usize, usize)> = None;
            for from in 0..terminals.len() {
                if !connected[from] {
                    continue;
                }
                for to in 0..terminals.len() {
                    if connected[to] {
                        continue;
                    }
                    let dx = positions[from].x - positions[to].x;
                    let dy = positions[from].y - positions[to].y;
                    let candidate = (dx * dx + dy * dy, from, to);
                    if best.as_ref().is_none_or(|current| {
                        candidate.0.total_cmp(&current.0).is_lt()
                            || (candidate.0 == current.0
                                && (candidate.1, candidate.2) < (current.1, current.2))
                    }) {
                        best = Some(candidate);
                    }
                }
            }
            let (_, from, to) = best.ok_or_else(|| {
                format!("electrical net {} has a disconnected terminal set", net.id)
            })?;
            connected[to] = true;
            let id = format!("{}#branch{branch_index}", net.id);
            if !branch_ids.insert(id.clone()) {
                return Err(format!("duplicate generated continuous branch ID {id}"));
            }
            branches.push(ContinuousBranch {
                id,
                electrical_net: net.id.clone(),
                width: net.width,
                tension_weight: net.tension_weight,
                layer: net.layer.clone(),
                from: terminals[from].clone(),
                to: terminals[to].clone(),
                seed_route: Vec::new(),
            });
        }
    }
    Ok(branches)
}

fn supported_two_terminal_kind(
    component: &layout_trace_model::model::Component,
) -> Option<ComponentKind> {
    let first = component.pins[0].offset;
    let second = component.pins[1].offset;
    let midpoint = Vec2::new((first.x + second.x) * 0.5, (first.y + second.y) * 0.5);
    let length = (second.x - first.x).abs();
    let tolerance = 1.0e-6;
    if midpoint.x.abs() > tolerance
        || midpoint.y.abs() > tolerance
        || first.y.abs() > tolerance
        || second.y.abs() > tolerance
        || length <= tolerance
        || (component.size.x - length).abs() > tolerance
    {
        return None;
    }
    Some(ComponentKind::TwoTerminalBody {
        length: length as f32,
        radius: (component.size.y * 0.5) as f32,
    })
}

fn unique_solution_poses(
    solution: &ContinuousSolution,
) -> Result<BTreeMap<&str, &pcb_core::ContinuousComponentPose>, String> {
    let mut result = BTreeMap::new();
    for pose in &solution.components {
        if result.insert(pose.id.as_str(), pose).is_some() {
            return Err(format!("duplicate continuous component pose {}", pose.id));
        }
    }
    Ok(result)
}

fn materialize_route_graphs(
    traces: &[SolvedTrace],
    branches: &[ContinuousBranch],
    problem: &Problem,
    poses: &BTreeMap<&str, &SolvedComponent>,
) -> Result<Vec<SolvedRouteGraph>, String> {
    let mut terminals = BTreeMap::<String, BTreeMap<(String, String), SolvedRouteNode>>::new();
    for branch in branches {
        for terminal in [&branch.from, &branch.to] {
            let node_id = terminal_node_id(&branch.electrical_net, terminal);
            terminals
                .entry(branch.electrical_net.clone())
                .or_default()
                .entry((terminal.component.clone(), terminal.pin.clone()))
                .or_insert(SolvedRouteNode {
                    id: node_id,
                    electrical_net: branch.electrical_net.clone(),
                    position: solved_pin_position(problem, poses, terminal)?,
                    incident_branches: Vec::new(),
                    kind: SolvedRouteNodeKind::Terminal {
                        component: terminal.component.clone(),
                        pin: terminal.pin.clone(),
                    },
                });
        }
    }
    for trace in traces {
        let nodes = terminals
            .get_mut(&trace.electrical_net)
            .ok_or_else(|| format!("missing graph for {}", trace.electrical_net))?;
        for endpoint in [&trace.from_node, &trace.to_node] {
            nodes
                .values_mut()
                .find(|node| node.id == *endpoint)
                .ok_or_else(|| format!("missing route endpoint {endpoint}"))?
                .incident_branches
                .push(trace.branch.clone());
        }
    }
    let trace_nets = traces
        .iter()
        .map(|trace| trace.electrical_net.clone())
        .collect::<BTreeSet<_>>();
    let mut graphs = terminals
        .into_iter()
        .map(|(electrical_net, nodes)| {
            let mut nodes = nodes.into_values().collect::<Vec<_>>();
            for node in &mut nodes {
                node.incident_branches.sort();
                node.incident_branches.dedup();
            }
            nodes.sort_by(|left, right| left.id.cmp(&right.id));
            let mut graph_branches = traces
                .iter()
                .filter(|trace| trace.electrical_net == electrical_net)
                .map(|trace| trace.branch.clone())
                .collect::<Vec<_>>();
            graph_branches.sort();
            SolvedRouteGraph {
                electrical_net,
                nodes,
                branches: graph_branches,
            }
        })
        .collect::<Vec<_>>();
    graphs.sort_by(|left, right| left.electrical_net.cmp(&right.electrical_net));
    if graphs.len() != trace_nets.len() {
        return Err("route graph materialization lost an electrical net".into());
    }
    Ok(graphs)
}

fn terminal_ref(reference: &PinRef) -> TerminalRef {
    TerminalRef {
        component: reference.component.clone(),
        terminal: reference.pin.clone(),
    }
}

fn terminal_node_id(net: &str, terminal: &PinRef) -> String {
    format!("terminal:{net}/{}.{}", terminal.component, terminal.pin)
}

fn declared_pin_position(problem: &Problem, terminal: &PinRef) -> Result<Vec2, String> {
    let component = problem
        .components
        .iter()
        .find(|component| component.id == terminal.component)
        .ok_or_else(|| format!("unknown component {}", terminal.component))?;
    let pin = component
        .pins
        .iter()
        .find(|pin| pin.id == terminal.pin)
        .ok_or_else(|| format!("unknown pin {}.{}", terminal.component, terminal.pin))?;
    Ok(add(
        component.position,
        rotate_degrees(pin.offset, component.rotation_degrees),
    ))
}

fn solved_pin_position(
    problem: &Problem,
    poses: &BTreeMap<&str, &SolvedComponent>,
    terminal: &PinRef,
) -> Result<Vec2, String> {
    let component = problem
        .components
        .iter()
        .find(|component| component.id == terminal.component)
        .ok_or_else(|| format!("unknown component {}", terminal.component))?;
    let pin = component
        .pins
        .iter()
        .find(|pin| pin.id == terminal.pin)
        .ok_or_else(|| format!("unknown pin {}.{}", terminal.component, terminal.pin))?;
    let pose = poses
        .get(terminal.component.as_str())
        .ok_or_else(|| format!("missing solved component {}", terminal.component))?;
    Ok(add(
        pose.position,
        rotate_degrees(pin.offset, pose.rotation_degrees),
    ))
}

fn engine_vec(value: Vec2, label: &str) -> Result<EngineVec2, String> {
    Ok(EngineVec2::new(
        checked_f32(value.x, label)?,
        checked_f32(value.y, label)?,
    ))
}

fn checked_f32(value: f64, label: &str) -> Result<f32, String> {
    let converted = value as f32;
    if !value.is_finite() || !converted.is_finite() {
        Err(format!("{label} cannot be represented by the f32 engine"))
    } else {
        Ok(converted)
    }
}

#[cfg(test)]
mod tests {
    use pcb_engine::{
        Backend, CpuReferenceBackend, NoField, SolverConfig, TraceSamplingPolicy,
        TwoTerminalRepresentation,
    };
    use pcb_validate::validate_candidate;

    use super::*;

    fn fixture() -> Problem {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/continuous-resistor-coupling.json"
        ))
        .unwrap();
        problem.check_schema().unwrap();
        problem
    }

    fn analytic_policy() -> CompilePolicy {
        policy(TwoTerminalRepresentation::AnalyticRigidBodyAttachments)
    }

    fn policy(representation: TwoTerminalRepresentation) -> CompilePolicy {
        CompilePolicy {
            two_terminal_representation: representation,
            rectangle_representation:
                pcb_engine::RectangleRepresentation::AnalyticRigidBodyAttachments,
            trace_sampling: TraceSamplingPolicy::PreserveSeedVertices,
            maximum_trace_spacing: 2.0,
            ..CompilePolicy::default()
        }
    }

    #[test]
    fn continuous_solution_materializes_and_exact_validates_after_coupled_motion() {
        let problem = fixture();
        for representation in [
            TwoTerminalRepresentation::ExperimentalEndpointParticles,
            TwoTerminalRepresentation::AnalyticRigidBodyAttachments,
        ] {
            let mut adapter =
                compile_problem_for_continuous_engine(&problem, policy(representation)).unwrap();
            let initial_x = adapter
                .solution()
                .unwrap()
                .components
                .iter()
                .find(|component| component.id == "R")
                .unwrap()
                .position
                .x;
            let mut backend = CpuReferenceBackend::with_field(NoField);
            let config = SolverConfig {
                field_strength: 0.0,
                projection_iterations: 8,
                ..SolverConfig::default()
            };
            for step in 0..20 {
                backend.step(&mut adapter.compiled.world, &config, step);
            }
            let solution = adapter.solution().unwrap();
            let final_x = solution
                .components
                .iter()
                .find(|component| component.id == "R")
                .unwrap()
                .position
                .x;
            assert!((final_x - initial_x).abs() > 0.05);
            let candidate = adapter.materialize_candidate().unwrap();
            let validation = validate_candidate(&problem, &candidate).unwrap();
            assert!(validation.complete, "{:?}", validation.violations);
            assert_eq!(candidate.traces.len(), 2);
            assert_eq!(candidate.route_graphs.len(), 2);
        }
    }

    #[test]
    fn materialization_does_not_hide_an_exact_invalid_engine_pose() {
        let problem = fixture();
        let mut multipin_policy = analytic_policy();
        multipin_policy.trace_sampling = TraceSamplingPolicy::SubdivideToMaximumSpacing;
        let mut adapter = compile_problem_for_continuous_engine(&problem, multipin_policy).unwrap();
        let body = adapter.compiled.body_handles["R"] as usize;
        adapter
            .compiled
            .world
            .bodies
            .set_position(body, pcb_core::Vec2::new(0.0, 10.0));
        let candidate = adapter.materialize_candidate().unwrap();
        let validation = validate_candidate(&problem, &candidate).unwrap();
        assert!(!validation.complete);
        assert!(
            validation
                .violations
                .iter()
                .any(|violation| violation.code == "component_outside_board")
        );
    }

    #[test]
    fn multi_terminal_graph_and_general_body_pins_exact_validate() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/continuous-multipin-tree.json"
        ))
        .unwrap();
        let mut adapter =
            compile_problem_for_continuous_engine(&problem, analytic_policy()).unwrap();
        assert_eq!(adapter.compiled.world.attachments.len(), 3);
        assert_eq!(adapter.compiled.world.bodies.len(), 1);
        let mut backend = CpuReferenceBackend::with_field(NoField);
        let config = SolverConfig {
            field_strength: 0.0,
            projection_iterations: 8,
            ..SolverConfig::default()
        };
        for step in 0..20 {
            backend.step(&mut adapter.compiled.world, &config, step);
        }
        let candidate = adapter.materialize_candidate().unwrap();
        assert_eq!(candidate.traces.len(), 4);
        assert_eq!(candidate.route_graphs.len(), 1);
        assert_eq!(candidate.route_graphs[0].nodes.len(), 5);
        let validation = validate_candidate(&problem, &candidate).unwrap();
        assert!(validation.complete, "{:?}", validation.violations);
    }
}

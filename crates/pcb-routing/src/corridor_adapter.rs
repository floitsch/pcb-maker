// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{
    Problem, Vec2,
    geometry::{Obb, add, rotate_degrees, scale, sub},
    model::{CopperShape, Pad, Pin, PinRef},
};
use layout_trace_routing::corridor::{
    CorridorGraph, CorridorObstacle, EmbeddingPurpose, RouteCorridorEmbedding,
    RouteEmbeddingRequest, build_corridor_graph, embed_route, embed_route_centerline_seed,
};
use pcb_validate::{SolvedComponent, validate_geometry};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CorridorEmbeddingMode {
    #[default]
    FullWidth,
    CenterlineSeed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorridorAnalysisConfig {
    pub circle_segments: usize,
    pub placement_revision: u64,
    pub first_corridor_revision: u64,
    #[serde(default)]
    pub embedding_mode: CorridorEmbeddingMode,
}

impl CorridorAnalysisConfig {
    pub fn check(&self) -> Result<(), String> {
        if !(8..=128).contains(&self.circle_segments) {
            return Err("corridor circle_segments must be between 8 and 128".into());
        }
        Ok(())
    }
}

impl Default for CorridorAnalysisConfig {
    fn default() -> Self {
        Self {
            circle_segments: 16,
            placement_revision: 1,
            first_corridor_revision: 1,
            embedding_mode: CorridorEmbeddingMode::FullWidth,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CorridorLayerWork {
    pub layer: String,
    pub obstacles: usize,
    pub cells: usize,
    pub gates: usize,
    pub semantic_regions: usize,
    pub semantic_passages: usize,
    pub topology_cuts: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct CorridorBranchEvidence {
    pub branch: String,
    pub electrical_net: String,
    pub selected_layer: String,
    pub successful: bool,
    pub embedding: RouteCorridorEmbedding,
}

#[derive(Clone, Debug, Serialize)]
pub struct CorridorAnalysisEvidence {
    pub strategy: String,
    pub config: CorridorAnalysisConfig,
    pub layer_work: Vec<CorridorLayerWork>,
    pub branch_count: usize,
    pub successful_branches: usize,
    pub failed_branches: usize,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CorridorAnalysisResult {
    pub components: Vec<SolvedComponent>,
    pub corridors: Vec<CorridorGraph>,
    pub branches: Vec<CorridorBranchEvidence>,
    pub evidence: CorridorAnalysisEvidence,
}

impl CorridorAnalysisResult {
    pub fn complete(&self) -> bool {
        self.evidence.failed_branches == 0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CorridorBranch {
    pub(crate) id: String,
    pub(crate) electrical_net: String,
    pub(crate) width: f64,
    pub(crate) tension_weight: f64,
    pub(crate) layer: String,
    pub(crate) allowed_layers: Vec<String>,
    pub(crate) from: PinRef,
    pub(crate) to: PinRef,
    pub(crate) reference: Vec<Vec2>,
}

/// Build current-placement corridor epochs and independently embed every
/// declared branch. This is topology/capacity evidence, not a joint routed
/// candidate: inter-route allocation and layer/via selection remain explicit
/// downstream stages.
pub fn analyze_problem_corridors(
    problem: &Problem,
    config: &CorridorAnalysisConfig,
) -> Result<CorridorAnalysisResult, String> {
    let components = problem
        .components
        .iter()
        .map(|component| SolvedComponent {
            id: component.id.clone(),
            position: component.position,
            size: component.size,
            rotation_degrees: component.rotation_degrees,
        })
        .collect::<Vec<_>>();
    analyze_problem_corridors_at_poses(problem, &components, config)
}

pub fn analyze_problem_corridors_at_poses(
    problem: &Problem,
    components: &[SolvedComponent],
    config: &CorridorAnalysisConfig,
) -> Result<CorridorAnalysisResult, String> {
    problem.check_schema()?;
    config.check()?;
    let placement = validate_geometry(problem, components, &[], &[]);
    if !placement.complete {
        return Err(format!(
            "corridor placement failed exact geometry with {} finding(s)",
            placement.violations.len()
        ));
    }
    let poses = component_pose_map(components)?;
    let mut corridors = Vec::new();
    let mut layer_work = Vec::new();
    for (index, layer) in problem.board.layers.iter().enumerate() {
        let revision = config
            .first_corridor_revision
            .checked_add(index as u64)
            .ok_or_else(|| "corridor revision overflow".to_string())?;
        let obstacles = routing_obstacles(problem, &poses, &layer.id, config.circle_segments)?;
        let graph = build_corridor_graph(
            &layer.id,
            problem.board.bounds,
            &obstacles,
            config.placement_revision,
            revision,
        )
        .map_err(|error| error.to_string())?;
        layer_work.push(CorridorLayerWork {
            layer: layer.id.clone(),
            obstacles: graph.obstacles.len(),
            cells: graph.cells.len(),
            gates: graph.gates.len(),
            semantic_regions: graph.semantic.regions.len(),
            semantic_passages: graph.semantic.passages.len(),
            topology_cuts: graph.cut_basis.cuts.len(),
        });
        corridors.push(graph);
    }

    let graph_by_layer = corridors
        .iter()
        .map(|graph| (graph.layer.as_str(), graph))
        .collect::<BTreeMap<_, _>>();
    let mut branches = Vec::new();
    for branch in compile_corridor_branches(problem, &poses)? {
        let graph = graph_by_layer
            .get(branch.layer.as_str())
            .ok_or_else(|| format!("branch {} uses unknown layer {}", branch.id, branch.layer))?;
        let (from, from_obstacle) =
            terminal_attachment(problem, &poses, &branch.from, &branch.layer)?;
        let (to, to_obstacle) = terminal_attachment(problem, &poses, &branch.to, &branch.layer)?;
        let mut reference = if branch.reference.len() >= 2 {
            branch.reference.clone()
        } else {
            vec![from, to]
        };
        reference[0] = from;
        let last = reference.len() - 1;
        reference[last] = to;
        let request = RouteEmbeddingRequest {
            net: &branch.id,
            from_component: &from_obstacle,
            to_component: &to_obstacle,
            physical_width: branch.width,
            clearance: problem.rules.clearance,
            from,
            from_toward: reference[1],
            to,
            to_toward: reference[last - 1],
            reference: &reference,
            target_homotopy: None,
            persistent_basis_matches: false,
        };
        let embedding = match config.embedding_mode {
            CorridorEmbeddingMode::FullWidth => embed_route(graph, request),
            CorridorEmbeddingMode::CenterlineSeed => embed_route_centerline_seed(graph, request),
        };
        let successful = match config.embedding_mode {
            CorridorEmbeddingMode::FullWidth => {
                embedding.purpose == EmbeddingPurpose::FullWidthCertificate
                    && embedding.failure.is_none()
                    && embedding.clearance.certified
                    && embedding.epoch_homotopy_verified
            }
            CorridorEmbeddingMode::CenterlineSeed => embedding.usable_centerline_seed(),
        };
        branches.push(CorridorBranchEvidence {
            branch: branch.id,
            electrical_net: branch.electrical_net,
            selected_layer: branch.layer,
            successful,
            embedding,
        });
    }
    branches.sort_by(|left, right| left.branch.cmp(&right.branch));
    let successful_branches = branches.iter().filter(|branch| branch.successful).count();
    let branch_count = branches.len();
    Ok(CorridorAnalysisResult {
        components: components.to_vec(),
        corridors,
        branches,
        evidence: CorridorAnalysisEvidence {
            strategy: "layout-trace-corridor-embedding-v1".into(),
            config: config.clone(),
            layer_work,
            branch_count,
            successful_branches,
            failed_branches: branch_count - successful_branches,
            limitations: vec![
                "embeddings are independent; joint passage allocation and copper-pair assignment are not applied by this adapter".into(),
                "each branch uses its preferred layer; bounded layer/via portfolio selection remains a separate stage".into(),
                "multi-terminal nets use the same deterministic terminal-anchored Prim tree as the DUT adapter".into(),
            ],
        },
    })
}

pub(crate) fn component_pose_map(
    components: &[SolvedComponent],
) -> Result<BTreeMap<&str, &SolvedComponent>, String> {
    let mut poses = BTreeMap::new();
    for component in components {
        if poses.insert(component.id.as_str(), component).is_some() {
            return Err(format!("duplicate solved component {}", component.id));
        }
    }
    Ok(poses)
}

fn routing_obstacles(
    problem: &Problem,
    poses: &BTreeMap<&str, &SolvedComponent>,
    layer: &str,
    circle_segments: usize,
) -> Result<Vec<CorridorObstacle>, String> {
    let mut obstacles = Vec::new();
    for component in &problem.components {
        let pose = poses
            .get(component.id.as_str())
            .ok_or_else(|| format!("missing solved component {}", component.id))?;
        if component.body_is_routing_keepout {
            obstacles.push(CorridorObstacle {
                id: component.id.clone(),
                polygon: Obb {
                    center: pose.position,
                    half_size: scale(pose.size, 0.5),
                    rotation_degrees: pose.rotation_degrees,
                }
                .corners()
                .to_vec(),
            });
        }
        for pin in &component.pins {
            for pad in pin.pads.iter().filter(|pad| pad.layer == layer) {
                let center = add(
                    pose.position,
                    rotate_degrees(pin.pad_local_center(pad), pose.rotation_degrees),
                );
                obstacles.push(CorridorObstacle {
                    id: pad_obstacle_id(&component.id, pin, pad),
                    polygon: copper_shape_polygon(
                        center,
                        pose.rotation_degrees,
                        &pad.shape,
                        circle_segments,
                    ),
                });
            }
        }
        for (index, keepout) in component
            .routing_keepouts
            .iter()
            .enumerate()
            .filter(|(_, keepout)| keepout.layer == layer)
        {
            let center = add(
                pose.position,
                rotate_degrees(keepout.offset, pose.rotation_degrees),
            );
            obstacles.push(CorridorObstacle {
                id: format!("keepout:{}:{index}", component.id),
                polygon: copper_shape_polygon(
                    center,
                    pose.rotation_degrees,
                    &keepout.shape,
                    circle_segments,
                ),
            });
        }
    }
    obstacles.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(obstacles)
}

fn copper_shape_polygon(
    center: Vec2,
    component_rotation: f64,
    shape: &CopperShape,
    circle_segments: usize,
) -> Vec<Vec2> {
    match shape {
        CopperShape::Circle { diameter } => (0..circle_segments)
            .map(|index| {
                let angle = std::f64::consts::TAU * index as f64 / circle_segments as f64;
                add(
                    center,
                    Vec2::new(angle.cos() * diameter * 0.5, angle.sin() * diameter * 0.5),
                )
            })
            .collect(),
        CopperShape::Rect {
            size,
            rotation_degrees,
        } => Obb {
            center,
            half_size: scale(*size, 0.5),
            rotation_degrees: component_rotation + rotation_degrees,
        }
        .corners()
        .to_vec(),
    }
}

fn pad_obstacle_id(component: &str, pin: &Pin, pad: &Pad) -> String {
    if pin
        .pads
        .iter()
        .filter(|candidate| candidate.layer == pad.layer)
        .count()
        == 1
    {
        format!("pad:{component}:{}", pin.id)
    } else {
        format!("pad:{component}:{}:{}", pin.id, pad.id)
    }
}

pub(crate) fn terminal_attachment(
    problem: &Problem,
    poses: &BTreeMap<&str, &SolvedComponent>,
    terminal: &PinRef,
    layer: &str,
) -> Result<(Vec2, String), String> {
    let component = problem
        .components
        .iter()
        .find(|component| component.id == terminal.component)
        .ok_or_else(|| format!("unknown terminal component {}", terminal.component))?;
    let pose = poses
        .get(component.id.as_str())
        .ok_or_else(|| format!("missing solved component {}", component.id))?;
    let pin = component
        .pins
        .iter()
        .find(|pin| pin.id == terminal.pin)
        .ok_or_else(|| format!("unknown terminal {}.{}", terminal.component, terminal.pin))?;
    let mut pads = pin
        .pads
        .iter()
        .filter(|pad| pad.layer == layer)
        .collect::<Vec<_>>();
    pads.sort_by(|left, right| left.id.cmp(&right.id));
    let (local, obstacle) = if let Some(pad) = pads.first() {
        (
            pin.pad_local_center(pad),
            pad_obstacle_id(&component.id, pin, pad),
        )
    } else if component.body_is_routing_keepout {
        (pin.offset, component.id.clone())
    } else {
        (pin.offset, format!("terminal:{}:{}", component.id, pin.id))
    };
    Ok((
        add(pose.position, rotate_degrees(local, pose.rotation_degrees)),
        obstacle,
    ))
}

pub(crate) fn compile_corridor_branches(
    problem: &Problem,
    poses: &BTreeMap<&str, &SolvedComponent>,
) -> Result<Vec<CorridorBranch>, String> {
    let mut branches = problem
        .nets
        .iter()
        .map(|net| CorridorBranch {
            id: net.id.clone(),
            electrical_net: net.electrical_net.clone().unwrap_or_else(|| net.id.clone()),
            width: net.width,
            tension_weight: net.tension_weight,
            layer: net.layer.clone(),
            allowed_layers: effective_allowed_layers(&net.layer, &net.allowed_layers),
            from: net.from.clone(),
            to: net.to.clone(),
            reference: net.seed_route.clone(),
        })
        .collect::<Vec<_>>();
    let mut ids = branches
        .iter()
        .map(|branch| branch.id.clone())
        .collect::<BTreeSet<_>>();
    for net in &problem.electrical_nets {
        let mut terminals = net.terminals.clone();
        terminals.sort_by(|left, right| {
            (left.component.as_str(), left.pin.as_str())
                .cmp(&(right.component.as_str(), right.pin.as_str()))
        });
        terminals
            .dedup_by(|left, right| left.component == right.component && left.pin == right.pin);
        let positions = terminals
            .iter()
            .map(|terminal| {
                terminal_attachment(problem, poses, terminal, &net.layer).map(|item| item.0)
            })
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
                    let delta = sub(positions[from], positions[to]);
                    let candidate = (delta.x * delta.x + delta.y * delta.y, from, to);
                    if best.as_ref().is_none_or(|current| {
                        candidate.0.total_cmp(&current.0).is_lt()
                            || (candidate.0 == current.0
                                && (candidate.1, candidate.2) < (current.1, current.2))
                    }) {
                        best = Some(candidate);
                    }
                }
            }
            let (_, from, to) = best.expect("multi-terminal tree must connect");
            connected[to] = true;
            let id = format!("{}#branch{}", net.id, branch_index);
            if !ids.insert(id.clone()) {
                return Err(format!("duplicate generated corridor branch ID {id}"));
            }
            branches.push(CorridorBranch {
                id,
                electrical_net: net.id.clone(),
                width: net.width,
                tension_weight: net.tension_weight,
                layer: net.layer.clone(),
                allowed_layers: effective_allowed_layers(&net.layer, &net.allowed_layers),
                from: terminals[from].clone(),
                to: terminals[to].clone(),
                reference: vec![positions[from], positions[to]],
            });
        }
    }
    branches.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(branches)
}

pub(crate) fn effective_allowed_layers(primary: &str, declared: &[String]) -> Vec<String> {
    let mut layers = if declared.is_empty() {
        vec![primary.to_owned()]
    } else {
        declared.to_vec()
    };
    layers.retain(|layer| layer != primary);
    layers.insert(0, primary.to_owned());
    let mut seen = BTreeSet::new();
    layers
        .into_iter()
        .filter(|layer| seen.insert(layer.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forced_crossing_builds_both_corridor_epochs_and_embeds_branches() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ))
        .unwrap();
        let result =
            analyze_problem_corridors(&problem, &CorridorAnalysisConfig::default()).unwrap();
        assert_eq!(result.corridors.len(), 2);
        assert_eq!(result.branches.len(), 2);
        assert!(result.complete(), "{:?}", result.branches);
        assert!(
            result
                .evidence
                .layer_work
                .iter()
                .all(|layer| layer.cells > 0 && layer.gates > 0)
        );
    }

    #[test]
    fn circle_tessellation_is_explicitly_bounded() {
        let config = CorridorAnalysisConfig {
            circle_segments: 7,
            ..CorridorAnalysisConfig::default()
        };
        assert!(config.check().is_err());
    }
}

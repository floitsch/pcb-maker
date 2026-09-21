// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{
    Problem, Vec2,
    geometry::{add, length, rotate_degrees, scale as scale_vec, sub},
    model::{Component as DeclaredComponent, CopperShape, Movement, Pad, Pin, Rotation},
};
use pcb_core::{Bounds, Frame, Mobility, Vec2 as EngineVec2};
use pcb_engine::{
    Backend, CopperAttachmentInput, CopperBodyInput, CopperPointRef, CopperPolylineInput,
    CopperPolylineMobility, CopperRepairRequest, CopperSegmentBodyPair, CopperSeparationPair,
    CopperSharedPointInput, CpuReferenceBackend, NoField, SolverConfig, compile_copper_repair,
};
use pcb_validate::{
    CandidateArtifact, ExactValidationAssessment, SolvedComponent, SolvedRouteNodeKind,
    validate_candidate,
};
use serde::{Deserialize, Serialize};

fn default_steps() -> usize {
    4
}

fn default_projection_iterations() -> usize {
    32
}

fn default_maximum_segment_pairs() -> usize {
    100_000
}

fn default_numerical_clearance_margin_nm() -> u32 {
    1_000
}

fn default_maximum_trace_tension_step_mm() -> f64 {
    0.2
}

fn default_trust_region_backtracking_steps() -> usize {
    6
}

/// Candidate-space trust region applied after the continuous engine proposes
/// motion. The engine remains a representation-independent geometry producer;
/// this boundary decides how much of that motion may enter an exact semantic
/// candidate.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuousCandidateTrustRegionConfig {
    pub maximum_trace_point_motion_mm: f64,
    pub maximum_body_translation_mm: f64,
    pub maximum_body_rotation_degrees: f64,
    #[serde(default = "default_trust_region_backtracking_steps")]
    pub backtracking_steps: usize,
}

impl ContinuousCandidateTrustRegionConfig {
    fn check(self) -> Result<(), String> {
        for (name, value) in [
            (
                "maximum_trace_point_motion_mm",
                self.maximum_trace_point_motion_mm,
            ),
            (
                "maximum_body_translation_mm",
                self.maximum_body_translation_mm,
            ),
            (
                "maximum_body_rotation_degrees",
                self.maximum_body_rotation_degrees,
            ),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!(
                    "continuous candidate repair trust_region.{name} must be finite and positive"
                ));
            }
        }
        if self.backtracking_steps == 0 || self.backtracking_steps > 32 {
            return Err(
                "continuous candidate repair trust_region.backtracking_steps must be between 1 and 32"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuousCandidateRepairConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_steps")]
    pub steps: usize,
    #[serde(default = "default_projection_iterations")]
    pub projection_iterations: usize,
    #[serde(default = "default_maximum_segment_pairs")]
    pub maximum_segment_pairs: usize,
    #[serde(default = "default_numerical_clearance_margin_nm")]
    pub numerical_clearance_margin_nm: u32,
    #[serde(default = "default_true")]
    pub move_trace_interiors: bool,
    #[serde(default = "default_true")]
    pub move_components: bool,
    /// Allow a pure trace/trace pressure set to recruit the endpoint bodies of
    /// the selected traces. Disabled by default so trace-only and explicit
    /// body-pressure experiments remain distinct.
    #[serde(default)]
    pub move_connected_components_for_trace_conflicts: bool,
    #[serde(default)]
    pub trace_tension_strength: f64,
    #[serde(default = "default_maximum_trace_tension_step_mm")]
    pub maximum_trace_tension_step_mm: f64,
    /// Optional semantic trust region. `None` preserves the original engine
    /// proposal exactly and therefore remains an explicit control.
    #[serde(default)]
    pub trust_region: Option<ContinuousCandidateTrustRegionConfig>,
}

const fn default_true() -> bool {
    true
}

impl Default for ContinuousCandidateRepairConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            steps: default_steps(),
            projection_iterations: default_projection_iterations(),
            maximum_segment_pairs: default_maximum_segment_pairs(),
            numerical_clearance_margin_nm: default_numerical_clearance_margin_nm(),
            move_trace_interiors: true,
            move_components: true,
            move_connected_components_for_trace_conflicts: false,
            trace_tension_strength: 0.0,
            maximum_trace_tension_step_mm: default_maximum_trace_tension_step_mm(),
            trust_region: None,
        }
    }
}

impl ContinuousCandidateRepairConfig {
    pub fn check(self) -> Result<(), String> {
        if self.steps == 0 {
            return Err("continuous candidate repair steps must be positive".into());
        }
        if self.projection_iterations == 0 {
            return Err(
                "continuous candidate repair projection_iterations must be positive".into(),
            );
        }
        if self.maximum_segment_pairs == 0 {
            return Err(
                "continuous candidate repair maximum_segment_pairs must be positive".into(),
            );
        }
        if self.numerical_clearance_margin_nm > 1_000_000 {
            return Err(
                "continuous candidate repair numerical_clearance_margin_nm must not exceed 1000000"
                    .into(),
            );
        }
        if !self.trace_tension_strength.is_finite()
            || !(0.0..=100.0).contains(&self.trace_tension_strength)
        {
            return Err(
                "continuous candidate repair trace_tension_strength must be finite and between 0 and 100"
                    .into(),
            );
        }
        if !self.maximum_trace_tension_step_mm.is_finite()
            || !(1.0e-6..=10.0).contains(&self.maximum_trace_tension_step_mm)
        {
            return Err(
                "continuous candidate repair maximum_trace_tension_step_mm must be finite and between 0.000001 and 10"
                    .into(),
            );
        }
        if let Some(trust_region) = self.trust_region {
            trust_region.check()?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuousCandidateRepairStatus {
    ExactComplete,
    RolledBack,
    NoRepairableFindings,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ContinuousCandidateBodyMotion {
    pub component: String,
    pub from: Vec2,
    pub to: Vec2,
    pub distance_mm: f64,
    pub rotation_before_degrees: f64,
    pub rotation_after_degrees: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ContinuousCandidateTraceMotion {
    pub branch: String,
    /// `None` means that the proposed polyline changed point count, so there
    /// is no one-to-one displacement measurement. The current compiler keeps
    /// point counts stable, but keeping the evidence JSON finite makes this
    /// boundary safe for future motion processors.
    pub maximum_point_motion_mm: Option<f64>,
    pub moved_points: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ContinuousCandidateTrustRegionTrial {
    pub scale: f64,
    pub complete: bool,
    pub findings: usize,
    pub total_shortfall_mm: f64,
    pub selected: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContinuousCandidateRepairResult {
    pub status: ContinuousCandidateRepairStatus,
    /// Transactional candidate: a failed or unsupported proposal retains the
    /// exact input candidate for a later discrete fallback.
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_candidate: Option<CandidateArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_validation: Option<ExactValidationAssessment>,
    pub selected_branches: Vec<String>,
    pub selected_bodies: Vec<String>,
    /// Exact obstacle identities such as `keepout:U1:0`. These are separate
    /// from component bodies because an explicit shape has no independent
    /// placement degree of freedom.
    pub selected_obstacles: Vec<String>,
    pub unsupported_findings: Vec<String>,
    pub segment_pairs: usize,
    pub segment_body_pairs: usize,
    pub body_motion: Vec<ContinuousCandidateBodyMotion>,
    pub trace_motion: Vec<ContinuousCandidateTraceMotion>,
    /// Empty for the legacy unbounded control. Frames describe the raw engine
    /// proposal; these trials describe exact candidate-space backtracking.
    pub trust_region_trials: Vec<ContinuousCandidateTrustRegionTrial>,
    pub frames: Vec<Frame>,
    pub trace_tension_edge_integrations: u64,
    pub detail: String,
}

impl ContinuousCandidateRepairResult {
    pub fn accepted(&self) -> bool {
        self.status == ContinuousCandidateRepairStatus::ExactComplete
    }
}

#[derive(Clone, Debug)]
struct EndpointBinding {
    branch: String,
    point_index: usize,
    polyline: String,
    polyline_point_index: usize,
    node: String,
    component: String,
    pin: String,
    local_position: Vec2,
}

#[derive(Clone, Debug)]
struct RepairLayerRun<'a> {
    id: String,
    trace: &'a pcb_validate::SolvedTrace,
    start_point: usize,
    end_point: usize,
    layer: String,
}

fn repair_layer_runs(trace: &pcb_validate::SolvedTrace) -> Vec<RepairLayerRun<'_>> {
    let mut runs = Vec::new();
    let mut start_segment = 0;
    while start_segment < trace.segment_layers.len() {
        let layer = trace.segment_layers[start_segment].clone();
        let mut end_segment = start_segment;
        while end_segment + 1 < trace.segment_layers.len()
            && trace.segment_layers[end_segment + 1] == layer
        {
            end_segment += 1;
        }
        runs.push(RepairLayerRun {
            id: format!("{}:layer-run:{}", trace.branch, runs.len()),
            trace,
            start_point: start_segment,
            end_point: end_segment + 1,
            layer,
        });
        start_segment = end_segment + 1;
    }
    runs
}

fn exact_total_shortfall_mm(validation: &ExactValidationAssessment) -> f64 {
    validation
        .violations
        .iter()
        .map(|finding| (finding.required_distance - finding.actual_distance).max(0.0))
        .sum()
}

fn exact_validation_score(validation: &ExactValidationAssessment) -> (usize, u64) {
    (
        validation.violations.len(),
        exact_total_shortfall_mm(validation).to_bits(),
    )
}

fn shortest_rotation_delta_degrees(from: f64, to: f64) -> f64 {
    (to - from + 180.0).rem_euclid(360.0) - 180.0
}

fn clamp_motion(delta: Vec2, maximum: f64) -> Vec2 {
    let distance = length(delta);
    if distance > maximum {
        scale_vec(delta, maximum / distance)
    } else {
        delta
    }
}

fn candidate_at_motion_scale(
    before: &CandidateArtifact,
    raw: &CandidateArtifact,
    scale: f64,
    endpoints: &[EndpointBinding],
    selected_branches: &BTreeSet<String>,
    selected_bodies: &BTreeSet<String>,
    config: ContinuousCandidateTrustRegionConfig,
) -> Result<CandidateArtifact, String> {
    if !scale.is_finite() || !(0.0..=1.0).contains(&scale) {
        return Err("trust-region motion scale must be finite and between zero and one".into());
    }
    let raw_components = raw
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let raw_traces = raw
        .traces
        .iter()
        .map(|trace| (trace.branch.as_str(), trace))
        .collect::<BTreeMap<_, _>>();
    let mut proposal = before.clone();
    for component in proposal
        .components
        .iter_mut()
        .filter(|component| selected_bodies.contains(&component.id))
    {
        let after = raw_components.get(component.id.as_str()).ok_or_else(|| {
            format!(
                "trust-region interpolation lost selected component {}",
                component.id
            )
        })?;
        component.position = add(
            component.position,
            clamp_motion(
                sub(after.position, component.position),
                config.maximum_body_translation_mm * scale,
            ),
        );
        let rotation_delta =
            shortest_rotation_delta_degrees(component.rotation_degrees, after.rotation_degrees)
                .clamp(
                    -config.maximum_body_rotation_degrees * scale,
                    config.maximum_body_rotation_degrees * scale,
                );
        component.rotation_degrees =
            (component.rotation_degrees + rotation_delta).rem_euclid(360.0);
    }
    for trace in proposal
        .traces
        .iter_mut()
        .filter(|trace| selected_branches.contains(&trace.branch))
    {
        let after = raw_traces.get(trace.branch.as_str()).ok_or_else(|| {
            format!(
                "trust-region interpolation lost selected branch {}",
                trace.branch
            )
        })?;
        if trace.points.len() != after.points.len() {
            return Err(format!(
                "trust-region interpolation changed point count for selected branch {}",
                trace.branch
            ));
        }
        let fixed_via_points = trace
            .vias
            .iter()
            .map(|via| via.point_index)
            .collect::<BTreeSet<_>>();
        for (index, (point, raw_point)) in trace.points.iter_mut().zip(&after.points).enumerate() {
            if fixed_via_points.contains(&index) {
                continue;
            }
            *point = add(
                *point,
                clamp_motion(
                    sub(*raw_point, *point),
                    config.maximum_trace_point_motion_mm * scale,
                ),
            );
        }
        trace.route_basis_fingerprint = None;
    }

    // Linear interpolation of a rotated body's endpoint is not a rigid local
    // attachment. Recompute every selected terminal from its interpolated body
    // pose before the exact gate.
    let proposal_components = proposal
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let mut graph_positions = BTreeMap::<String, Vec2>::new();
    for binding in endpoints {
        let component = proposal_components
            .get(binding.component.as_str())
            .ok_or_else(|| {
                format!(
                    "trust-region interpolation lost endpoint body {}",
                    binding.component
                )
            })?;
        let position = add(
            component.position,
            rotate_degrees(binding.local_position, component.rotation_degrees),
        );
        let trace = proposal
            .traces
            .iter_mut()
            .find(|trace| trace.branch == binding.branch)
            .ok_or_else(|| {
                format!(
                    "trust-region interpolation lost endpoint branch {}",
                    binding.branch
                )
            })?;
        trace.points[binding.point_index] = position;
        if graph_positions
            .insert(binding.node.clone(), position)
            .is_some_and(|prior| length(sub(prior, position)) > 1.0e-8)
        {
            return Err(format!(
                "trust-region interpolation produced inconsistent shared node {}",
                binding.node
            ));
        }
    }
    for graph in &mut proposal.route_graphs {
        for node in &mut graph.nodes {
            if let Some(position) = graph_positions.get(&node.id) {
                node.position = *position;
            }
        }
    }
    Ok(proposal)
}

fn select_trust_region_proposal(
    problem: &Problem,
    before: &CandidateArtifact,
    raw: &CandidateArtifact,
    endpoints: &[EndpointBinding],
    selected_branches: &BTreeSet<String>,
    selected_bodies: &BTreeSet<String>,
    config: ContinuousCandidateTrustRegionConfig,
) -> Result<
    (
        CandidateArtifact,
        ExactValidationAssessment,
        Vec<ContinuousCandidateTrustRegionTrial>,
    ),
    String,
> {
    let mut candidates = Vec::with_capacity(config.backtracking_steps);
    for step in 0..config.backtracking_steps {
        let scale = 1.0 / 2.0_f64.powi(step as i32);
        let candidate = candidate_at_motion_scale(
            before,
            raw,
            scale,
            endpoints,
            selected_branches,
            selected_bodies,
            config,
        )?;
        let validation = validate_candidate(problem, &candidate)?;
        candidates.push((scale, candidate, validation));
        if candidates.last().is_some_and(|entry| entry.2.complete) {
            break;
        }
    }
    let selected = candidates
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, _, validation))| exact_validation_score(validation))
        .map(|(index, _)| index)
        .ok_or_else(|| "trust-region backtracking produced no candidates".to_string())?;
    let trials = candidates
        .iter()
        .enumerate()
        .map(
            |(index, (scale, _, validation))| ContinuousCandidateTrustRegionTrial {
                scale: *scale,
                complete: validation.complete,
                findings: validation.violations.len(),
                total_shortfall_mm: exact_total_shortfall_mm(validation),
                selected: index == selected,
            },
        )
        .collect::<Vec<_>>();
    let (_, candidate, validation) = candidates.swap_remove(selected);
    Ok((candidate, validation, trials))
}

/// Compile exact trace/trace and trace/body findings into the generic copper
/// engine. The result is committed only when the complete candidate passes the
/// independent exact geometry and connectivity gates.
pub fn repair_candidate_continuously(
    problem: &Problem,
    candidate: &CandidateArtifact,
    config: ContinuousCandidateRepairConfig,
) -> Result<ContinuousCandidateRepairResult, String> {
    problem.check_schema()?;
    config.check()?;
    let baseline_validation = validate_candidate(problem, candidate)?;
    if baseline_validation.complete {
        return Ok(outcome_without_proposal(
            ContinuousCandidateRepairStatus::NoRepairableFindings,
            candidate,
            baseline_validation,
            "candidate already passes exact validation",
        ));
    }

    let trace_ids = candidate
        .traces
        .iter()
        .map(|trace| trace.branch.clone())
        .collect::<BTreeSet<_>>();
    let mut selected_branches = BTreeSet::<String>::new();
    let mut selected_bodies = BTreeSet::<String>::new();
    let mut selected_obstacles = BTreeSet::<String>::new();
    let declared_keepouts = problem
        .components
        .iter()
        .flat_map(|component| {
            component
                .routing_keepouts
                .iter()
                .enumerate()
                .map(move |(index, keepout)| {
                    (
                        format!("keepout:{}:{index}", component.id),
                        (component, index, keepout),
                    )
                })
        })
        .collect::<BTreeMap<_, _>>();
    let declared_pads = problem
        .components
        .iter()
        .flat_map(|component| {
            component.pins.iter().flat_map(move |pin| {
                pin.pads.iter().map(move |pad| {
                    (
                        format!("pad:{}:{}:{}", component.id, pin.id, pad.id),
                        (component, pin, pad),
                    )
                })
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut supported_findings = 0_usize;
    let mut unsupported = Vec::<String>::new();
    for finding in &baseline_validation.geometry.violations {
        match finding.code.as_str() {
            "trace_trace_clearance" => {
                let branches = finding
                    .objects
                    .iter()
                    .filter(|object| trace_ids.contains(*object))
                    .cloned()
                    .collect::<Vec<_>>();
                if branches.len() == 2 {
                    selected_branches.extend(branches);
                    supported_findings += 1;
                } else {
                    unsupported.push(finding.code.clone());
                }
            }
            "trace_obstacle_clearance" => {
                let branch = finding
                    .objects
                    .iter()
                    .find(|object| trace_ids.contains(*object))
                    .cloned();
                let body = finding
                    .objects
                    .iter()
                    .find_map(|object| object.strip_prefix("body:").map(str::to_owned));
                let keepout = finding
                    .objects
                    .iter()
                    .find(|object| declared_keepouts.contains_key(*object))
                    .cloned();
                let pad = finding
                    .objects
                    .iter()
                    .find(|object| declared_pads.contains_key(*object))
                    .cloned();
                if let (Some(branch), Some(body)) = (branch.clone(), body) {
                    selected_branches.insert(branch);
                    selected_bodies.insert(body);
                    supported_findings += 1;
                } else if let (Some(branch), Some(pad)) = (branch.clone(), pad) {
                    selected_branches.insert(branch);
                    selected_obstacles.insert(pad);
                    supported_findings += 1;
                } else if let (Some(branch), Some(keepout)) = (branch, keepout) {
                    selected_branches.insert(branch);
                    selected_obstacles.insert(keepout);
                    supported_findings += 1;
                } else {
                    unsupported.push(format!("{}:{:?}", finding.code, finding.objects));
                }
            }
            "via_trace_clearance" => {
                let branches = finding
                    .objects
                    .iter()
                    .filter(|object| trace_ids.contains(*object))
                    .cloned()
                    .collect::<Vec<_>>();
                if branches.len() == 2 {
                    selected_branches.extend(branches);
                    supported_findings += 1;
                } else {
                    unsupported.push(format!("{}:{:?}", finding.code, finding.objects));
                }
            }
            code if finding
                .objects
                .iter()
                .any(|object| trace_ids.contains(object)) =>
            {
                unsupported.push(code.to_owned());
            }
            _ => {}
        }
    }
    if config.move_components
        && config.move_connected_components_for_trace_conflicts
        && selected_obstacles.is_empty()
    {
        for graph in &candidate.route_graphs {
            for node in &graph.nodes {
                if let SolvedRouteNodeKind::Terminal { component, .. } = &node.kind
                    && node
                        .incident_branches
                        .iter()
                        .any(|branch| selected_branches.contains(branch))
                {
                    selected_bodies.insert(component.clone());
                }
            }
        }
    }
    // Moving a component without every incident trace would tear electrical
    // endpoints away from its pads. Close the selected set over all branches
    // incident to a pressure-selected body before compiling any mobility.
    if config.move_components {
        for graph in &candidate.route_graphs {
            for node in &graph.nodes {
                if let SolvedRouteNodeKind::Terminal { component, .. } = &node.kind
                    && selected_bodies.contains(component)
                {
                    selected_branches.extend(node.incident_branches.iter().cloned());
                }
            }
        }
    }
    if supported_findings == 0 {
        let status = if unsupported.is_empty() {
            ContinuousCandidateRepairStatus::NoRepairableFindings
        } else {
            ContinuousCandidateRepairStatus::Unsupported
        };
        let mut outcome = outcome_without_proposal(
            status,
            candidate,
            baseline_validation,
            if unsupported.is_empty() {
                "exact findings contain no supported trace/trace or trace/body correction"
                    .to_string()
            } else {
                format!("unsupported exact findings: {}", unsupported.join(", "))
            },
        );
        outcome.unsupported_findings = unsupported;
        return Ok(outcome);
    }

    for id in &selected_obstacles {
        if let Some((component, _, keepout)) = declared_keepouts.get(id) {
            if !matches!(keepout.shape, CopperShape::Rect { .. }) {
                return Ok(unsupported_outcome(
                    candidate,
                    baseline_validation,
                    &selected_branches,
                    &selected_bodies,
                    &selected_obstacles,
                    format!(
                        "selected obstacle {id} is not rectangular; continuous segment/shape correction currently supports explicit rectangles"
                    ),
                ));
            }
            if component.constraints.movement != Movement::Fixed
                || component.constraints.rotation != Rotation::Fixed
            {
                return Ok(unsupported_outcome(
                    candidate,
                    baseline_validation,
                    &selected_branches,
                    &selected_bodies,
                    &selected_obstacles,
                    format!(
                        "selected obstacle {id} belongs to movable component {}; rigid compound-shape motion is not yet compiled",
                        component.id
                    ),
                ));
            }
            continue;
        }
        let (component, _, _) = declared_pads
            .get(id)
            .ok_or_else(|| format!("continuous repair selected unknown obstacle {id}"))?;
        if config.move_components && selected_bodies.contains(&component.id) {
            return Ok(unsupported_outcome(
                candidate,
                baseline_validation,
                &selected_branches,
                &selected_bodies,
                &selected_obstacles,
                format!(
                    "selected pad {id} belongs to moving component {}; rigid compound-shape motion is not yet compiled",
                    component.id
                ),
            ));
        }
    }

    let traces = candidate
        .traces
        .iter()
        .map(|trace| (trace.branch.as_str(), trace))
        .collect::<BTreeMap<_, _>>();
    let components = candidate
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let declarations = problem
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let nodes = candidate
        .route_graphs
        .iter()
        .flat_map(|graph| &graph.nodes)
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();

    for branch in &selected_branches {
        let trace = traces
            .get(branch.as_str())
            .ok_or_else(|| format!("continuous repair selected unknown branch {branch}"))?;
        if trace.segment_layers.is_empty() {
            return Ok(unsupported_outcome(
                candidate,
                baseline_validation,
                &selected_branches,
                &selected_bodies,
                &selected_obstacles,
                format!("selected branch {branch} has no segment layers"),
            ));
        }
    }

    let selected_layers = selected_branches
        .iter()
        .flat_map(|branch| traces[branch.as_str()].segment_layers.iter().cloned())
        .collect::<BTreeSet<_>>();
    let input_traces = candidate
        .traces
        .iter()
        .filter(|trace| {
            !trace.segment_layers.is_empty()
                && trace
                    .segment_layers
                    .iter()
                    .any(|layer| selected_layers.contains(layer))
        })
        .collect::<Vec<_>>();
    let input_trace_ids = input_traces
        .iter()
        .map(|trace| trace.branch.as_str())
        .collect::<BTreeSet<_>>();
    if selected_branches
        .iter()
        .any(|branch| !input_trace_ids.contains(branch.as_str()))
    {
        return Err("selected continuous branch was omitted from its layer input".into());
    }

    let mut body_inputs = BTreeMap::<String, CopperBodyInput>::new();
    let mut explicit_obstacle_layers = BTreeMap::<String, String>::new();
    let mut explicit_pad_owners = BTreeMap::<String, (String, String)>::new();
    let mut circular_pad_inputs = Vec::<(CopperPolylineInput, String, String, String)>::new();
    for declared in problem
        .components
        .iter()
        .filter(|component| component.body_is_routing_keepout)
    {
        insert_body_input(
            &mut body_inputs,
            body_input(
                problem,
                &components,
                declared.id.as_str(),
                config.move_components && selected_bodies.contains(&declared.id),
            )?,
        )?;
    }
    for (id, (declared, index, keepout)) in &declared_keepouts {
        if !selected_layers.contains(&keepout.layer)
            || declared.constraints.movement != Movement::Fixed
            || declared.constraints.rotation != Rotation::Fixed
            || !matches!(keepout.shape, CopperShape::Rect { .. })
        {
            continue;
        }
        insert_body_input(
            &mut body_inputs,
            fixed_rect_keepout_input(&components, declared, *index)?,
        )?;
        explicit_obstacle_layers.insert(id.clone(), keepout.layer.clone());
    }
    for (id, (declared, pin, pad)) in &declared_pads {
        if !selected_layers.contains(&pad.layer)
            || (config.move_components && selected_bodies.contains(&declared.id))
        {
            continue;
        }
        match &pad.shape {
            CopperShape::Rect { .. } => insert_body_input(
                &mut body_inputs,
                fixed_rect_pad_input(&components, declared, pin, pad)?,
            )?,
            CopperShape::Circle { .. } => circular_pad_inputs.push((
                fixed_circle_pad_input(&components, declared, pin, pad)?,
                pad.layer.clone(),
                declared.id.clone(),
                pin.id.clone(),
            )),
        }
        explicit_obstacle_layers.insert(id.clone(), pad.layer.clone());
        explicit_pad_owners.insert(id.clone(), (declared.id.clone(), pin.id.clone()));
    }
    if let Some(missing) = selected_obstacles
        .iter()
        .find(|id| !explicit_obstacle_layers.contains_key(*id))
    {
        return Err(format!(
            "selected supported explicit obstacle {missing} was omitted from continuous input"
        ));
    }
    for body in &selected_bodies {
        insert_body_input(
            &mut body_inputs,
            body_input(problem, &components, body, config.move_components)?,
        )?;
    }
    let input_runs = input_traces
        .iter()
        .flat_map(|trace| repair_layer_runs(trace))
        .filter(|run| selected_layers.contains(&run.layer))
        .collect::<Vec<_>>();
    let mut endpoints = Vec::<EndpointBinding>::new();
    for branch in &selected_branches {
        let trace = traces[branch.as_str()];
        let endpoint_nodes = [
            (0, trace.from_node.as_str()),
            (trace.points.len() - 1, trace.to_node.as_str()),
        ];
        for (point_index, node_id) in endpoint_nodes {
            let node = nodes
                .get(node_id)
                .ok_or_else(|| format!("selected branch {branch} lacks endpoint node {node_id}"))?;
            let SolvedRouteNodeKind::Terminal { component, pin } = &node.kind else {
                return Ok(unsupported_outcome(
                    candidate,
                    baseline_validation,
                    &selected_branches,
                    &selected_bodies,
                    &selected_obstacles,
                    format!(
                        "selected branch {branch} has a junction endpoint; continuous endpoint attachment is not yet defined"
                    ),
                ));
            };
            let pose = components.get(component.as_str()).ok_or_else(|| {
                format!("selected branch {branch} references missing pose {component}")
            })?;
            insert_body_input(
                &mut body_inputs,
                body_input(
                    problem,
                    &components,
                    component,
                    config.move_components && selected_bodies.contains(component),
                )?,
            )?;
            let local_position = rotate_degrees(
                sub(trace.points[point_index], pose.position),
                -pose.rotation_degrees,
            );
            let run = input_runs
                .iter()
                .find(|run| {
                    run.trace.branch == *branch
                        && run.start_point <= point_index
                        && point_index <= run.end_point
                })
                .ok_or_else(|| {
                    format!("selected branch {branch} endpoint is outside its layer runs")
                })?;
            endpoints.push(EndpointBinding {
                branch: branch.clone(),
                point_index,
                polyline: run.id.clone(),
                polyline_point_index: point_index - run.start_point,
                node: node_id.to_owned(),
                component: component.clone(),
                pin: pin.clone(),
                local_position,
            });
        }
    }

    let mut polylines = input_runs
        .iter()
        .map(|run| {
            Ok(CopperPolylineInput {
                id: run.id.clone(),
                points: run.trace.points[run.start_point..=run.end_point]
                    .iter()
                    .map(|point| engine_vec(*point, "trace point"))
                    .collect::<Result<Vec<_>, String>>()?,
                width: checked_f32(run.trace.width, "trace width")?,
                tension_weight: checked_f32(run.trace.tension_weight, "trace tension weight")?,
                mobility: if selected_branches.contains(&run.trace.branch) {
                    if config.move_trace_interiors {
                        CopperPolylineMobility::All
                    } else {
                        CopperPolylineMobility::Fixed
                    }
                } else {
                    CopperPolylineMobility::Fixed
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    polylines.extend(
        circular_pad_inputs
            .iter()
            .map(|(polyline, _, _, _)| polyline.clone()),
    );
    let mut shared_points = Vec::<CopperSharedPointInput>::new();
    let mut via_anchors = Vec::<(String, String, String, BTreeSet<String>)>::new();
    for trace in &input_traces {
        for (via_index, via) in trace.vias.iter().enumerate() {
            let id = format!("{}:fixed-via:{via_index}", trace.branch);
            let position = engine_vec(via.position, "fixed via position")?;
            polylines.push(CopperPolylineInput {
                id: id.clone(),
                points: vec![position, position],
                width: checked_f32(via.diameter, "fixed via diameter")?,
                tension_weight: 0.0,
                mobility: CopperPolylineMobility::Fixed,
            });
            let mut members = input_runs
                .iter()
                .filter(|run| {
                    run.trace.branch == trace.branch
                        && run.start_point <= via.point_index
                        && via.point_index <= run.end_point
                })
                .map(|run| CopperPointRef {
                    polyline: run.id.clone(),
                    point_index: via.point_index - run.start_point,
                })
                .collect::<Vec<_>>();
            members.push(CopperPointRef {
                polyline: id.clone(),
                point_index: 0,
            });
            let endpoint_via = via.point_index == 0 || via.point_index + 1 == trace.points.len();
            let required_members = if endpoint_via { 2 } else { 3 };
            if members.len() < required_members {
                return Ok(unsupported_outcome(
                    candidate,
                    baseline_validation,
                    &selected_branches,
                    &selected_bodies,
                    &selected_obstacles,
                    format!(
                        "via {}:{} does not join its required {} layer run(s)",
                        trace.branch,
                        via_index,
                        required_members - 1
                    ),
                ));
            }
            shared_points.push(CopperSharedPointInput {
                id: format!("fixed-via-junction:{}:{via_index}", trace.branch),
                members,
            });
            via_anchors.push((
                id,
                trace.branch.clone(),
                trace.electrical_net.clone(),
                [via.from_layer.clone(), via.to_layer.clone()]
                    .into_iter()
                    .collect(),
            ));
        }
    }
    let attachments = endpoints
        .iter()
        .map(|binding| {
            Ok(CopperAttachmentInput {
                polyline: binding.polyline.clone(),
                point_index: binding.polyline_point_index,
                body: binding.component.clone(),
                local_position: engine_vec(binding.local_position, "attachment local position")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let numerical_margin = config.numerical_clearance_margin_nm as f64 / 1_000_000.0;
    let clearance = checked_f32(problem.rules.clearance + numerical_margin, "clearance")?;
    let mut separations = Vec::<CopperSeparationPair>::new();
    for first in 0..input_runs.len() {
        for second in first + 1..input_runs.len() {
            let left = &input_runs[first];
            let right = &input_runs[second];
            if left.trace.electrical_net == right.trace.electrical_net
                || left.layer != right.layer
                || (!selected_branches.contains(&left.trace.branch)
                    && !selected_branches.contains(&right.trace.branch))
            {
                continue;
            }
            separations.push(CopperSeparationPair {
                first: left.id.clone(),
                second: right.id.clone(),
                clearance,
            });
        }
    }
    for (via, owner_branch, electrical_net, layers) in &via_anchors {
        for run in &input_runs {
            if run.trace.electrical_net == *electrical_net
                || !layers.contains(&run.layer)
                || (!selected_branches.contains(&run.trace.branch)
                    && !selected_branches.contains(owner_branch))
            {
                continue;
            }
            separations.push(CopperSeparationPair {
                first: via.clone(),
                second: run.id.clone(),
                clearance,
            });
        }
    }
    let endpoint_owners = endpoints.iter().fold(
        BTreeMap::<&str, BTreeSet<&str>>::new(),
        |mut owners, binding| {
            owners
                .entry(binding.branch.as_str())
                .or_default()
                .insert(binding.component.as_str());
            owners
        },
    );
    let endpoint_pad_owners = endpoints.iter().fold(
        BTreeMap::<&str, BTreeSet<(&str, &str)>>::new(),
        |mut owners, binding| {
            owners
                .entry(binding.branch.as_str())
                .or_default()
                .insert((binding.component.as_str(), binding.pin.as_str()));
            owners
        },
    );
    for (pad, layer, component, pin) in &circular_pad_inputs {
        for run in &input_runs {
            if layer != &run.layer
                || endpoint_pad_owners
                    .get(run.trace.branch.as_str())
                    .is_some_and(|owners| owners.contains(&(component.as_str(), pin.as_str())))
                || (!selected_branches.contains(&run.trace.branch)
                    && !selected_obstacles.contains(&pad.id))
            {
                continue;
            }
            separations.push(CopperSeparationPair {
                first: pad.id.clone(),
                second: run.id.clone(),
                clearance,
            });
        }
    }
    let mut body_separations = Vec::<CopperSegmentBodyPair>::new();
    for run in &input_runs {
        for body in body_inputs.keys() {
            if let Some(layer) = explicit_obstacle_layers.get(body) {
                if layer != &run.layer {
                    continue;
                }
                if explicit_pad_owners.get(body).is_some_and(|owner| {
                    endpoint_pad_owners
                        .get(run.trace.branch.as_str())
                        .is_some_and(|owners| {
                            owners.contains(&(owner.0.as_str(), owner.1.as_str()))
                        })
                }) {
                    continue;
                }
                if selected_branches.contains(&run.trace.branch)
                    || selected_obstacles.contains(body)
                {
                    body_separations.push(CopperSegmentBodyPair {
                        polyline: run.id.clone(),
                        body: body.clone(),
                        clearance,
                    });
                }
                continue;
            }
            let body_is_keepout = declarations
                .get(body.as_str())
                .is_some_and(|declared| declared.body_is_routing_keepout);
            let body_is_selected_pad_proxy = selected_bodies.contains(body)
                && declarations.get(body.as_str()).is_some_and(|declared| {
                    declared
                        .pins
                        .iter()
                        .flat_map(|pin| &pin.pads)
                        .any(|pad| pad.layer == run.layer)
                });
            if (!body_is_keepout && !body_is_selected_pad_proxy)
                || endpoint_owners
                    .get(run.trace.branch.as_str())
                    .is_some_and(|owners| owners.contains(body.as_str()))
            {
                continue;
            }
            if selected_branches.contains(&run.trace.branch) || selected_bodies.contains(body) {
                body_separations.push(CopperSegmentBodyPair {
                    polyline: run.id.clone(),
                    body: body.clone(),
                    clearance,
                });
            }
        }
    }

    let request = CopperRepairRequest {
        bounds: Bounds::new(
            engine_vec(problem.board.bounds.min, "board minimum")?,
            engine_vec(problem.board.bounds.max, "board maximum")?,
        ),
        bodies: body_inputs.into_values().collect(),
        polylines,
        shared_points,
        attachments,
        separations,
        body_separations,
        body_body_separations: Vec::new(),
        maximum_segment_pairs: config.maximum_segment_pairs,
    };
    let mut compiled = match compile_copper_repair(&request) {
        Ok(compiled) => compiled,
        Err(error) => {
            return Ok(unsupported_outcome(
                candidate,
                baseline_validation,
                &selected_branches,
                &selected_bodies,
                &selected_obstacles,
                format!("continuous candidate repair compilation failed: {error}"),
            ));
        }
    };
    let segment_pairs = compiled.segment_pairs;
    let segment_body_pairs = compiled.segment_body_pairs;
    let mut backend = CpuReferenceBackend::with_field(NoField);
    let solver = SolverConfig {
        field_strength: 0.0,
        projection_iterations: config.projection_iterations,
        trace_tension_strength: checked_f32(
            config.trace_tension_strength,
            "trace tension strength",
        )?,
        maximum_trace_tension_step: checked_f32(
            config.maximum_trace_tension_step_mm,
            "maximum trace tension step",
        )?,
        ..SolverConfig::default()
    };
    let mut frames = Vec::with_capacity(config.steps + 1);
    frames.push(backend.step(
        &mut compiled.world,
        &SolverConfig {
            projection_iterations: 0,
            trace_tension_strength: 0.0,
            ..solver
        },
        0,
    ));
    for step in 1..=config.steps {
        frames.push(backend.step(&mut compiled.world, &solver, step as u64));
    }

    let mut proposed = candidate.clone();
    let engine_bodies = compiled
        .body_poses()
        .into_iter()
        .map(|body| (body.id.clone(), body))
        .collect::<BTreeMap<_, _>>();
    for component in &mut proposed.components {
        if !config.move_components || !selected_bodies.contains(&component.id) {
            continue;
        }
        let Some(body) = engine_bodies.get(&component.id) else {
            continue;
        };
        let declared = declarations.get(component.id.as_str()).ok_or_else(|| {
            format!(
                "continuous repair candidate has unknown component {}",
                component.id
            )
        })?;
        let engine_position = Vec2::new(body.position.x as f64, body.position.y as f64);
        component.position = match declared.constraints.movement {
            Movement::Fixed => component.position,
            Movement::Horizontal => Vec2::new(engine_position.x, component.position.y),
            Movement::Vertical => Vec2::new(component.position.x, engine_position.y),
            Movement::Free => engine_position,
        };
        if declared.constraints.rotation == Rotation::Free {
            component.rotation_degrees = (body.angle_radians as f64).to_degrees().rem_euclid(360.0);
        }
    }
    let engine_polylines = compiled.polylines().into_iter().collect::<BTreeMap<_, _>>();
    for run in &input_runs {
        if !selected_branches.contains(&run.trace.branch) {
            continue;
        }
        let points = engine_polylines
            .get(&run.id)
            .ok_or_else(|| format!("continuous repair lost layer run {}", run.id))?;
        let trace = proposed
            .traces
            .iter_mut()
            .find(|trace| trace.branch == run.trace.branch)
            .ok_or_else(|| format!("continuous repair lost branch {}", run.trace.branch))?;
        for (local_index, point) in points.iter().enumerate() {
            trace.points[run.start_point + local_index] = Vec2::new(point.x as f64, point.y as f64);
        }
        for via in &trace.vias {
            trace.points[via.point_index] = via.position;
        }
        trace.route_basis_fingerprint = None;
    }
    let proposed_components = proposed
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let mut graph_positions = BTreeMap::<String, Vec2>::new();
    for binding in &endpoints {
        let component = proposed_components
            .get(binding.component.as_str())
            .ok_or_else(|| format!("continuous repair lost component {}", binding.component))?;
        let position = add(
            component.position,
            rotate_degrees(binding.local_position, component.rotation_degrees),
        );
        let trace = proposed
            .traces
            .iter_mut()
            .find(|trace| trace.branch == binding.branch)
            .ok_or_else(|| format!("continuous repair lost branch {}", binding.branch))?;
        trace.points[binding.point_index] = position;
        if graph_positions
            .insert(binding.node.clone(), position)
            .is_some_and(|prior| length(sub(prior, position)) > 1.0e-8)
        {
            return Err(format!(
                "continuous repair produced inconsistent shared node {}",
                binding.node
            ));
        }
    }
    for graph in &mut proposed.route_graphs {
        for node in &mut graph.nodes {
            if let Some(position) = graph_positions.get(&node.id) {
                node.position = *position;
            }
        }
    }

    let (proposed, proposed_validation, trust_region_trials) =
        if let Some(trust_region) = config.trust_region {
            select_trust_region_proposal(
                problem,
                candidate,
                &proposed,
                &endpoints,
                &selected_branches,
                &selected_bodies,
                trust_region,
            )?
        } else {
            let validation = validate_candidate(problem, &proposed)?;
            (proposed, validation, Vec::new())
        };
    let body_motion = candidate
        .components
        .iter()
        .filter_map(|before| {
            let after = proposed
                .components
                .iter()
                .find(|component| component.id == before.id)?;
            let distance_mm = length(sub(after.position, before.position));
            let rotation_changed =
                (after.rotation_degrees - before.rotation_degrees).abs() > 1.0e-9;
            (distance_mm > 1.0e-9 || rotation_changed).then_some(ContinuousCandidateBodyMotion {
                component: before.id.clone(),
                from: before.position,
                to: after.position,
                distance_mm,
                rotation_before_degrees: before.rotation_degrees,
                rotation_after_degrees: after.rotation_degrees,
            })
        })
        .collect::<Vec<_>>();
    let trace_motion = candidate
        .traces
        .iter()
        .filter_map(|before| {
            let after = proposed
                .traces
                .iter()
                .find(|trace| trace.branch == before.branch)?;
            if before.points.len() != after.points.len() {
                return Some(ContinuousCandidateTraceMotion {
                    branch: before.branch.clone(),
                    maximum_point_motion_mm: None,
                    moved_points: after.points.len(),
                });
            }
            let motions = before
                .points
                .iter()
                .zip(&after.points)
                .map(|(left, right)| length(sub(*right, *left)))
                .collect::<Vec<_>>();
            let maximum_point_motion_mm = motions.iter().copied().fold(0.0_f64, f64::max);
            let moved_points = motions.iter().filter(|motion| **motion > 1.0e-9).count();
            (moved_points > 0).then_some(ContinuousCandidateTraceMotion {
                branch: before.branch.clone(),
                maximum_point_motion_mm: Some(maximum_point_motion_mm),
                moved_points,
            })
        })
        .collect::<Vec<_>>();
    let accepted = proposed_validation.complete;
    Ok(ContinuousCandidateRepairResult {
        status: if accepted {
            ContinuousCandidateRepairStatus::ExactComplete
        } else {
            ContinuousCandidateRepairStatus::RolledBack
        },
        candidate: if accepted {
            proposed.clone()
        } else {
            candidate.clone()
        },
        validation: if accepted {
            proposed_validation.clone()
        } else {
            baseline_validation
        },
        proposed_candidate: Some(proposed),
        proposed_validation: Some(proposed_validation.clone()),
        selected_branches: selected_branches.into_iter().collect(),
        selected_bodies: selected_bodies.into_iter().collect(),
        selected_obstacles: selected_obstacles.into_iter().collect(),
        unsupported_findings: unsupported,
        segment_pairs,
        segment_body_pairs,
        body_motion,
        trace_motion,
        trust_region_trials,
        trace_tension_edge_integrations: frames
            .iter()
            .map(|frame| frame.metrics.trace_tension_edges)
            .sum(),
        frames,
        detail: if accepted {
            format!(
                "continuous candidate correction passed exact validation after {supported_findings} supported finding(s)"
            )
        } else {
            format!(
                "continuous candidate correction remained incomplete with {} exact finding(s); rolled back for discrete fallback",
                proposed_validation.violations.len()
            )
        },
    })
}

fn outcome_without_proposal(
    status: ContinuousCandidateRepairStatus,
    candidate: &CandidateArtifact,
    validation: ExactValidationAssessment,
    detail: impl Into<String>,
) -> ContinuousCandidateRepairResult {
    ContinuousCandidateRepairResult {
        status,
        candidate: candidate.clone(),
        validation,
        proposed_candidate: None,
        proposed_validation: None,
        selected_branches: Vec::new(),
        selected_bodies: Vec::new(),
        selected_obstacles: Vec::new(),
        unsupported_findings: Vec::new(),
        segment_pairs: 0,
        segment_body_pairs: 0,
        body_motion: Vec::new(),
        trace_motion: Vec::new(),
        trust_region_trials: Vec::new(),
        trace_tension_edge_integrations: 0,
        frames: Vec::new(),
        detail: detail.into(),
    }
}

fn unsupported_outcome(
    candidate: &CandidateArtifact,
    validation: ExactValidationAssessment,
    branches: &BTreeSet<String>,
    bodies: &BTreeSet<String>,
    obstacles: &BTreeSet<String>,
    detail: impl Into<String>,
) -> ContinuousCandidateRepairResult {
    let mut result = outcome_without_proposal(
        ContinuousCandidateRepairStatus::Unsupported,
        candidate,
        validation,
        detail,
    );
    result.selected_branches = branches.iter().cloned().collect();
    result.selected_bodies = bodies.iter().cloned().collect();
    result.selected_obstacles = obstacles.iter().cloned().collect();
    result
}

fn insert_body_input(
    bodies: &mut BTreeMap<String, CopperBodyInput>,
    body: CopperBodyInput,
) -> Result<(), String> {
    if let Some(current) = bodies.get(&body.id) {
        if current.position != body.position
            || current.angle_radians != body.angle_radians
            || current.size != body.size
        {
            return Err(format!("conflicting continuous body input {}", body.id));
        }
        return Ok(());
    }
    bodies.insert(body.id.clone(), body);
    Ok(())
}

fn fixed_rect_keepout_input(
    components: &BTreeMap<&str, &SolvedComponent>,
    declared: &DeclaredComponent,
    index: usize,
) -> Result<CopperBodyInput, String> {
    let keepout = declared.routing_keepouts.get(index).ok_or_else(|| {
        format!(
            "continuous keepout references missing shape {}:{index}",
            declared.id
        )
    })?;
    let CopperShape::Rect {
        size,
        rotation_degrees,
    } = &keepout.shape
    else {
        return Err(format!(
            "continuous keepout keepout:{}:{index} is not rectangular",
            declared.id
        ));
    };
    let solved = components
        .get(declared.id.as_str())
        .ok_or_else(|| format!("continuous keepout lacks solved component {}", declared.id))?;
    let position = add(
        solved.position,
        rotate_degrees(keepout.offset, solved.rotation_degrees),
    );
    Ok(CopperBodyInput {
        id: format!("keepout:{}:{index}", declared.id),
        position: engine_vec(position, "explicit keepout position")?,
        angle_radians: checked_f32(
            (solved.rotation_degrees + rotation_degrees).to_radians(),
            "explicit keepout rotation",
        )?,
        size: engine_vec(*size, "explicit keepout size")?,
        mobility: Mobility::FIXED,
    })
}

fn fixed_rect_pad_input(
    components: &BTreeMap<&str, &SolvedComponent>,
    declared: &DeclaredComponent,
    pin: &Pin,
    pad: &Pad,
) -> Result<CopperBodyInput, String> {
    let CopperShape::Rect {
        size,
        rotation_degrees,
    } = &pad.shape
    else {
        return Err(format!(
            "continuous pad pad:{}:{}:{} is not rectangular",
            declared.id, pin.id, pad.id
        ));
    };
    let solved = components
        .get(declared.id.as_str())
        .ok_or_else(|| format!("continuous pad lacks solved component {}", declared.id))?;
    let position = add(
        solved.position,
        rotate_degrees(pin.pad_local_center(pad), solved.rotation_degrees),
    );
    Ok(CopperBodyInput {
        id: format!("pad:{}:{}:{}", declared.id, pin.id, pad.id),
        position: engine_vec(position, "pad position")?,
        angle_radians: checked_f32(
            (solved.rotation_degrees + rotation_degrees).to_radians(),
            "pad rotation",
        )?,
        size: engine_vec(*size, "pad size")?,
        mobility: Mobility::FIXED,
    })
}

fn fixed_circle_pad_input(
    components: &BTreeMap<&str, &SolvedComponent>,
    declared: &DeclaredComponent,
    pin: &Pin,
    pad: &Pad,
) -> Result<CopperPolylineInput, String> {
    let CopperShape::Circle { diameter } = &pad.shape else {
        return Err(format!(
            "continuous pad pad:{}:{}:{} is not circular",
            declared.id, pin.id, pad.id
        ));
    };
    let solved = components
        .get(declared.id.as_str())
        .ok_or_else(|| format!("continuous pad lacks solved component {}", declared.id))?;
    let position = engine_vec(
        add(
            solved.position,
            rotate_degrees(pin.pad_local_center(pad), solved.rotation_degrees),
        ),
        "circular pad position",
    )?;
    Ok(CopperPolylineInput {
        id: format!("pad:{}:{}:{}", declared.id, pin.id, pad.id),
        points: vec![position, position],
        width: checked_f32(*diameter, "circular pad diameter")?,
        tension_weight: 0.0,
        mobility: CopperPolylineMobility::Fixed,
    })
}

fn body_input(
    problem: &Problem,
    components: &BTreeMap<&str, &SolvedComponent>,
    id: &str,
    move_components: bool,
) -> Result<CopperBodyInput, String> {
    let declared = problem
        .components
        .iter()
        .find(|component| component.id == id)
        .ok_or_else(|| format!("continuous body references unknown component {id}"))?;
    let solved = components
        .get(id)
        .ok_or_else(|| format!("continuous body lacks solved component {id}"))?;
    let (translate_x, translate_y) = if move_components {
        match declared.constraints.movement {
            Movement::Fixed => (false, false),
            Movement::Horizontal => (true, false),
            Movement::Vertical => (false, true),
            Movement::Free => (true, true),
        }
    } else {
        (false, false)
    };
    let rotate = move_components && declared.constraints.rotation == Rotation::Free;
    Ok(CopperBodyInput {
        id: id.to_owned(),
        position: engine_vec(solved.position, "component position")?,
        angle_radians: checked_f32(solved.rotation_degrees.to_radians(), "component rotation")?,
        size: engine_vec(solved.size, "component size")?,
        mobility: Mobility {
            inverse_mass: if translate_x || translate_y {
                checked_f32(problem.solver.component_mobility, "component mobility")?
            } else {
                0.0
            },
            rotation_mobility: if rotate {
                checked_f32(problem.solver.rotation_mobility, "rotation mobility")?
            } else {
                0.0
            },
            translate_x,
            translate_y,
            rotate,
        },
    })
}

fn engine_vec(point: Vec2, label: &str) -> Result<EngineVec2, String> {
    Ok(EngineVec2::new(
        checked_f32(point.x, label)?,
        checked_f32(point.y, label)?,
    ))
}

fn checked_f32(value: f64, label: &str) -> Result<f32, String> {
    if !value.is_finite() || value < f32::MIN as f64 || value > f32::MAX as f64 {
        return Err(format!("{label} is outside the finite f32 range"));
    }
    Ok(value as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DutGridRoutingConfig, route_problem_with_dut_grid};

    fn fixture() -> Problem {
        serde_json::from_str(include_str!(
            "../../../benchmarks/small/board-continuation-force-motion.json"
        ))
        .unwrap()
    }

    #[test]
    fn exact_trace_body_finding_pushes_a_vertical_component() {
        let problem = fixture();
        let mut clear_problem = problem.clone();
        clear_problem
            .components
            .iter_mut()
            .find(|component| component.id == "YIELDING_BLOCKER")
            .unwrap()
            .position
            .y = 7.0;
        let mut candidate = route_problem_with_dut_grid(
            &clear_problem,
            &DutGridRoutingConfig {
                allow_vias: false,
                grid_mm: 0.25,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap()
        .candidate;
        candidate
            .components
            .iter_mut()
            .find(|component| component.id == "YIELDING_BLOCKER")
            .unwrap()
            .position
            .y = 6.6;
        let before = validate_candidate(&problem, &candidate).unwrap();
        assert!(!before.complete);
        assert!(
            before
                .geometry
                .violations
                .iter()
                .any(|finding| finding.code == "trace_obstacle_clearance")
        );

        let result = repair_candidate_continuously(
            &problem,
            &candidate,
            ContinuousCandidateRepairConfig {
                enabled: true,
                ..ContinuousCandidateRepairConfig::default()
            },
        )
        .unwrap();

        assert!(result.accepted(), "{}", result.detail);
        assert!(result.validation.complete);
        assert_eq!(result.selected_branches, ["SIGNAL"]);
        assert_eq!(result.selected_bodies, ["YIELDING_BLOCKER"]);
        assert_eq!(result.body_motion.len(), 1);
        assert!(
            result.body_motion[0].distance_mm > 0.09,
            "{:?}",
            result.body_motion
        );
        assert!(
            result
                .trace_motion
                .iter()
                .any(|motion| motion.branch == "SIGNAL")
        );
        assert!(result.segment_body_pairs > 0);
        assert_eq!(result.frames.len(), 5);
    }

    #[test]
    fn semantic_trust_region_caps_a_raw_engine_proposal() {
        let problem = fixture();
        let mut clear_problem = problem.clone();
        clear_problem
            .components
            .iter_mut()
            .find(|component| component.id == "YIELDING_BLOCKER")
            .unwrap()
            .position
            .y = 7.0;
        let mut candidate = route_problem_with_dut_grid(
            &clear_problem,
            &DutGridRoutingConfig {
                allow_vias: false,
                grid_mm: 0.25,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap()
        .candidate;
        candidate
            .components
            .iter_mut()
            .find(|component| component.id == "YIELDING_BLOCKER")
            .unwrap()
            .position
            .y = 6.6;

        let result = repair_candidate_continuously(
            &problem,
            &candidate,
            ContinuousCandidateRepairConfig {
                enabled: true,
                trust_region: Some(ContinuousCandidateTrustRegionConfig {
                    maximum_trace_point_motion_mm: 0.01,
                    maximum_body_translation_mm: 0.01,
                    maximum_body_rotation_degrees: 0.1,
                    backtracking_steps: 4,
                }),
                ..ContinuousCandidateRepairConfig::default()
            },
        )
        .unwrap();

        assert!(!result.trust_region_trials.is_empty());
        assert_eq!(result.trust_region_trials[0].scale, 1.0);
        assert_eq!(
            result
                .trust_region_trials
                .iter()
                .filter(|trial| trial.selected)
                .count(),
            1
        );
        assert!(
            result
                .body_motion
                .iter()
                .all(|motion| motion.distance_mm <= 0.010001),
            "{:?}",
            result.body_motion
        );
        assert!(
            result.trace_motion.iter().all(|motion| motion
                .maximum_point_motion_mm
                .is_some_and(|distance| distance <= 0.010001)),
            "{:?}",
            result.trace_motion
        );
        assert_eq!(result.candidate, candidate);
    }

    #[test]
    fn trust_region_config_is_explicitly_bounded() {
        let mut config = ContinuousCandidateRepairConfig {
            enabled: true,
            trust_region: Some(ContinuousCandidateTrustRegionConfig {
                maximum_trace_point_motion_mm: 0.1,
                maximum_body_translation_mm: 0.1,
                maximum_body_rotation_degrees: 1.0,
                backtracking_steps: 6,
            }),
            ..ContinuousCandidateRepairConfig::default()
        };
        assert!(config.check().is_ok());
        config.trust_region.as_mut().unwrap().backtracking_steps = 0;
        assert!(config.check().is_err());
        config.trust_region.as_mut().unwrap().backtracking_steps = 33;
        assert!(config.check().is_err());
        config
            .trust_region
            .as_mut()
            .unwrap()
            .maximum_trace_point_motion_mm = f64::NAN;
        assert!(config.check().is_err());
    }

    #[test]
    fn circular_keepout_is_reported_as_unsupported_without_mutating_the_candidate() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/board-continuation-explicit-keepout-fallback.json"
        ))
        .unwrap();
        let mut problem_without_keepout = problem.clone();
        problem_without_keepout
            .components
            .iter_mut()
            .find(|component| component.id == "WALL")
            .unwrap()
            .routing_keepouts
            .clear();
        let candidate = route_problem_with_dut_grid(
            &problem_without_keepout,
            &DutGridRoutingConfig {
                allow_vias: false,
                grid_mm: 0.25,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap()
        .candidate;
        assert!(!validate_candidate(&problem, &candidate).unwrap().complete);

        let result = repair_candidate_continuously(
            &problem,
            &candidate,
            ContinuousCandidateRepairConfig {
                enabled: true,
                ..ContinuousCandidateRepairConfig::default()
            },
        )
        .unwrap();

        assert_eq!(result.status, ContinuousCandidateRepairStatus::Unsupported);
        assert_eq!(result.candidate, candidate);
        assert!(result.proposed_candidate.is_none());
        assert_eq!(result.selected_obstacles, ["keepout:WALL:0"]);
        assert!(result.detail.contains("not rectangular"));
        assert!(result.frames.is_empty());
    }
}

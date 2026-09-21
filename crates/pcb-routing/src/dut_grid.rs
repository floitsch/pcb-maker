// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{
    Problem, Vec2,
    geometry::{
        EPSILON, Obb, add, cross, length, point_obb_signed_distance, point_segment_distance,
        rect_contains_with_margin, rotate_degrees, scale, segment_distance, segment_obb_distance,
        segment_proximity, sub,
    },
    model::{Component, CopperShape, PinRef},
    topology::{RouteClass, TerminalSector},
};
use pcb_grid_router::{DutAStar, GridPosition, GridRouteOutcome, GridRouteRequest, GridRouter};
use pcb_validate::{
    CANDIDATE_SCHEMA_VERSION, CandidateArtifact, SolvedComponent, SolvedRouteGraph,
    SolvedRouteNode, SolvedRouteNodeKind, SolvedTrace, SolvedVia, validate_candidate,
    validate_geometry,
};

use crate::{
    BranchRoutingEvidence, BranchRoutingStatus, DutGridRoutingConfig, DutGridRoutingEvidence,
    DutGridRoutingResult, GridRoutingAttemptEvidence, MultiTerminalRoutingPolicy,
    NegotiatedDutGridRoutingConfig, NegotiatedDutGridRoutingEvidence,
    NegotiatedDutGridRoutingResult, NegotiatedPassEvidence, RasterSafety, RouteOrder,
    RoutingBlockerEvidence, RoutingBlockerKind, TerminalJunctionAttemptEvidence,
    TreeAttachmentAttemptEvidence, TreeAttachmentEvidence, TreeAttachmentPortfolioObjective,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TerminalKey {
    component: String,
    pin: String,
}

impl From<&PinRef> for TerminalKey {
    fn from(value: &PinRef) -> Self {
        Self {
            component: value.component.clone(),
            pin: value.pin.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct TerminalAttachment {
    position: Vec2,
    layers: Vec<String>,
}

#[derive(Clone, Debug)]
struct BranchRequest {
    id: String,
    electrical_net: String,
    width: f64,
    tension_weight: f64,
    preferred_layer: String,
    allowed_layers: Vec<String>,
    from: TerminalKey,
    to: TerminalKey,
    input_order: usize,
    tree_growth_index: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
enum ExactShape {
    Circle { center: Vec2, radius: f64 },
    Rect(Obb),
}

impl ExactShape {
    fn point_distance(self, point: Vec2) -> f64 {
        match self {
            Self::Circle { center, radius } => (length(sub(point, center)) - radius).max(0.0),
            Self::Rect(obb) => point_obb_signed_distance(point, obb).0.max(0.0),
        }
    }

    fn segment_distance(self, first: Vec2, second: Vec2) -> f64 {
        match self {
            Self::Circle { center, radius } => {
                (point_segment_distance(center, first, second) - radius).max(0.0)
            }
            Self::Rect(obb) => segment_obb_distance(first, second, obb),
        }
    }

    fn bounds(self, margin: f64) -> (Vec2, Vec2) {
        match self {
            Self::Circle { center, radius } => {
                let extent = radius + margin;
                (
                    Vec2::new(center.x - extent, center.y - extent),
                    Vec2::new(center.x + extent, center.y + extent),
                )
            }
            Self::Rect(obb) => {
                let radians = obb.rotation_degrees.to_radians();
                let extent = Vec2::new(
                    radians.cos().abs() * obb.half_size.x
                        + radians.sin().abs() * obb.half_size.y
                        + margin,
                    radians.sin().abs() * obb.half_size.x
                        + radians.cos().abs() * obb.half_size.y
                        + margin,
                );
                (sub(obb.center, extent), add(obb.center, extent))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObstacleKind {
    Body,
    Keepout,
    Pad,
}

#[derive(Clone, Debug)]
struct RasterObstacle {
    owner: String,
    pin: Option<String>,
    layer: String,
    kind: ObstacleKind,
    shape: ExactShape,
}

#[derive(Clone, Debug)]
struct GridMap {
    origin: Vec2,
    grid_mm: f64,
    width: usize,
    height: usize,
    layer_ids: Vec<String>,
    layer_indexes: BTreeMap<String, usize>,
}

impl GridMap {
    fn new(problem: &Problem, grid_mm: f64) -> Result<Self, String> {
        let width = ((problem.board.bounds.width() / grid_mm).floor() as usize)
            .checked_add(1)
            .ok_or_else(|| "routing grid width overflow".to_string())?;
        let height = ((problem.board.bounds.height() / grid_mm).floor() as usize)
            .checked_add(1)
            .ok_or_else(|| "routing grid height overflow".to_string())?;
        let layer_ids = problem
            .board
            .layers
            .iter()
            .map(|layer| layer.id.clone())
            .collect::<Vec<_>>();
        let layer_indexes = layer_ids
            .iter()
            .enumerate()
            .map(|(index, layer)| (layer.clone(), index))
            .collect();
        Ok(Self {
            origin: problem.board.bounds.min,
            grid_mm,
            width,
            height,
            layer_ids,
            layer_indexes,
        })
    }

    fn point(&self, position: GridPosition) -> Vec2 {
        Vec2::new(
            self.origin.x + position.x as f64 * self.grid_mm,
            self.origin.y + position.y as f64 * self.grid_mm,
        )
    }

    fn snap(&self, point: Vec2, layer: &str) -> Result<GridPosition, String> {
        let layer = *self
            .layer_indexes
            .get(layer)
            .ok_or_else(|| format!("unknown routing layer {layer}"))?;
        let x = ((point.x - self.origin.x) / self.grid_mm)
            .round()
            .clamp(0.0, (self.width - 1) as f64) as usize;
        let y = ((point.y - self.origin.y) / self.grid_mm)
            .round()
            .clamp(0.0, (self.height - 1) as f64) as usize;
        Ok(GridPosition { x, y, layer })
    }

    fn state_count(&self) -> Result<usize, String> {
        self.width
            .checked_mul(self.height)
            .and_then(|count| count.checked_mul(self.layer_ids.len()))
            .ok_or_else(|| "routing grid state count overflow".into())
    }

    fn state_index(&self, position: GridPosition) -> usize {
        position.layer * self.width * self.height + position.y * self.width + position.x
    }

    fn plane_size(&self) -> usize {
        self.width * self.height
    }

    fn cell_bounds(&self, minimum: Vec2, maximum: Vec2) -> Option<(usize, usize, usize, usize)> {
        let grid_maximum = self.point(GridPosition {
            x: self.width - 1,
            y: self.height - 1,
            layer: 0,
        });
        if maximum.x < self.origin.x
            || maximum.y < self.origin.y
            || minimum.x > grid_maximum.x
            || minimum.y > grid_maximum.y
        {
            return None;
        }
        let first_x = (((minimum.x - self.origin.x) / self.grid_mm).ceil() as isize)
            .clamp(0, self.width as isize - 1) as usize;
        let last_x = (((maximum.x - self.origin.x) / self.grid_mm).floor() as isize)
            .clamp(0, self.width as isize - 1) as usize;
        let first_y = (((minimum.y - self.origin.y) / self.grid_mm).ceil() as isize)
            .clamp(0, self.height as isize - 1) as usize;
        let last_y = (((maximum.y - self.origin.y) / self.grid_mm).floor() as isize)
            .clamp(0, self.height as isize - 1) as usize;
        (first_x <= last_x && first_y <= last_y).then_some((first_x, last_x, first_y, last_y))
    }
}

struct GraphBuilder {
    terminals: BTreeMap<TerminalKey, TerminalAttachment>,
    junctions: BTreeMap<String, SolvedRouteNode>,
}

impl GraphBuilder {
    fn new(terminals: BTreeMap<TerminalKey, TerminalAttachment>) -> Self {
        Self {
            terminals,
            junctions: BTreeMap::new(),
        }
    }

    fn add_junction(&mut self, junction: SolvedRouteNode) -> Result<(), String> {
        if self
            .junctions
            .insert(junction.id.clone(), junction)
            .is_some()
        {
            return Err("duplicate generated route junction".into());
        }
        Ok(())
    }

    fn finish(
        self,
        electrical_net: String,
        traces: &[SolvedTrace],
    ) -> Result<SolvedRouteGraph, String> {
        let mut nodes = self
            .terminals
            .into_iter()
            .map(|(terminal, attachment)| SolvedRouteNode {
                id: terminal_node_id(&electrical_net, &terminal),
                electrical_net: electrical_net.clone(),
                position: attachment.position,
                incident_branches: Vec::new(),
                kind: SolvedRouteNodeKind::Terminal {
                    component: terminal.component,
                    pin: terminal.pin,
                },
            })
            .map(|node| (node.id.clone(), node))
            .collect::<BTreeMap<_, _>>();
        for (id, junction) in self.junctions {
            if nodes.insert(id.clone(), junction).is_some() {
                return Err(format!("route junction {id} collides with another node"));
            }
        }
        let mut branches = BTreeSet::new();
        for trace in traces
            .iter()
            .filter(|trace| trace.electrical_net == electrical_net)
        {
            if !branches.insert(trace.branch.clone()) {
                return Err(format!("duplicate trace branch {}", trace.branch));
            }
            for endpoint in [&trace.from_node, &trace.to_node] {
                nodes
                    .get_mut(endpoint)
                    .ok_or_else(|| {
                        format!(
                            "trace {} references missing endpoint node {endpoint}",
                            trace.branch
                        )
                    })?
                    .incident_branches
                    .push(trace.branch.clone());
            }
            for via in &trace.vias {
                let node = SolvedRouteNode {
                    id: format!("{}::via::{}", trace.branch, via.point_index),
                    electrical_net: trace.electrical_net.clone(),
                    position: via.position,
                    incident_branches: vec![trace.branch.clone()],
                    kind: SolvedRouteNodeKind::Via {
                        point_index: via.point_index,
                        from_layer: via.from_layer.clone(),
                        to_layer: via.to_layer.clone(),
                        diameter: via.diameter,
                        drill: via.drill,
                    },
                };
                if nodes.insert(node.id.clone(), node).is_some() {
                    return Err(format!("duplicate route node for branch {}", trace.branch));
                }
            }
        }
        let mut nodes = nodes.into_values().collect::<Vec<_>>();
        for node in &mut nodes {
            node.incident_branches.sort();
            node.incident_branches.dedup();
        }
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(SolvedRouteGraph {
            electrical_net,
            nodes,
            branches: branches.into_iter().collect(),
        })
    }
}

struct BranchSearchResult {
    trace: Option<SolvedTrace>,
    evidence: BranchRoutingEvidence,
}

struct TreeGrowthCommit {
    searched: TreeAttachmentSelection,
    committed: TreeAttachmentSelection,
    target: TerminalKey,
    trimmed_prefix_points: usize,
    new_segment_split: bool,
}

struct TreeGrowthTrial {
    searched: TreeAttachmentSelection,
    target: TerminalKey,
    routing_branch: BranchRequest,
    result: BranchSearchResult,
    trimmed: Option<TrimmedTreePrefix>,
}

#[derive(Clone, Debug)]
struct TreeAttachmentSelection {
    trace_index: usize,
    source_branch: String,
    point_index: usize,
    existing_segment_index: Option<usize>,
    node: String,
    position: Vec2,
    layer: String,
    interior: bool,
    source_terminal: Option<TerminalKey>,
}

#[derive(Clone, Debug)]
struct TrimmedTreePrefix {
    selection: TreeAttachmentSelection,
    trimmed_prefix_points: usize,
    new_segment_split: bool,
}

#[derive(Clone, Debug)]
struct PathTreeContact {
    progress: f64,
    new_segment_index: usize,
    new_segment_parameter: f64,
    selection: TreeAttachmentSelection,
}

struct FrontierClassificationContext<'a> {
    problem: &'a Problem,
    config: &'a DutGridRoutingConfig,
    grid: &'a GridMap,
    obstacles: &'a [RasterObstacle],
    terminals: &'a BTreeSet<TerminalKey>,
    branch: &'a BranchRequest,
    existing: &'a [SolvedTrace],
}

struct PathValidationContext<'a> {
    problem: &'a Problem,
    grid: &'a GridMap,
    obstacles: &'a [RasterObstacle],
    terminals: &'a BTreeSet<TerminalKey>,
    branch: &'a BranchRequest,
    from: &'a TerminalAttachment,
    to: &'a TerminalAttachment,
    existing: &'a [SolvedTrace],
    allow_existing_copper_overlap: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct BlockerHitAccumulator {
    count: usize,
    sum_x: f64,
    sum_y: f64,
}

impl BlockerHitAccumulator {
    fn record(&mut self, point: Vec2) {
        self.count += 1;
        self.sum_x += point.x;
        self.sum_y += point.y;
    }

    fn centroid(self) -> Vec2 {
        let divisor = self.count as f64;
        Vec2::new(self.sum_x / divisor, self.sum_y / divisor)
    }
}

#[derive(Clone, Copy)]
enum CopperReservation<'a> {
    Hard,
    Negotiated {
        congestion_cost: u32,
        history_costs: &'a [u32],
    },
}

#[derive(Clone, Copy)]
struct FixedRouting<'a> {
    baseline: &'a DutGridRoutingResult,
    reroute_branches: &'a BTreeSet<String>,
}

impl CopperReservation<'_> {
    fn negotiated(self) -> bool {
        matches!(self, Self::Negotiated { .. })
    }
}

/// Route at the problem's declared component poses.
pub fn route_problem_with_dut_grid(
    problem: &Problem,
    config: &DutGridRoutingConfig,
) -> Result<DutGridRoutingResult, String> {
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
    route_problem_with_dut_grid_at_poses(problem, &components, config)
}

/// Route at an explicitly supplied placement. The placement is validated
/// before it can influence rasterization.
pub fn route_problem_with_dut_grid_at_poses(
    problem: &Problem,
    components: &[SolvedComponent],
    config: &DutGridRoutingConfig,
) -> Result<DutGridRoutingResult, String> {
    route_problem_with_dut_grid_pass(problem, components, config, CopperReservation::Hard, None)
}

/// Keep every legal baseline trace outside `reroute_branches` fixed, then
/// reroute only the selected failure group. This is the transactional primitive
/// used by selective rip-up coordinators.
pub fn reroute_problem_branches_with_dut_grid_at_poses(
    problem: &Problem,
    components: &[SolvedComponent],
    baseline: &DutGridRoutingResult,
    reroute_branches: &BTreeSet<String>,
    config: &DutGridRoutingConfig,
) -> Result<DutGridRoutingResult, String> {
    if reroute_branches.is_empty() {
        return Err("selective reroute requires at least one branch".into());
    }
    if baseline.candidate.components != components {
        return Err("selective reroute placement differs from its baseline candidate".into());
    }
    let retained_traces = baseline
        .candidate
        .traces
        .iter()
        .filter(|trace| !reroute_branches.contains(&trace.branch))
        .cloned()
        .collect::<Vec<_>>();
    let retained_geometry = validate_geometry(
        problem,
        components,
        &retained_traces,
        &baseline.candidate.route_graphs,
    );
    if !retained_geometry.complete {
        let first = retained_geometry.violations.first();
        return Err(format!(
            "selective reroute retained copper has invalid fixed geometry with {} finding(s): {}",
            retained_geometry.violations.len(),
            first.map_or("unknown geometry violation", |item| item.message.as_str())
        ));
    }
    route_problem_with_dut_grid_pass(
        problem,
        components,
        config,
        CopperReservation::Hard,
        Some(FixedRouting {
            baseline,
            reroute_branches,
        }),
    )
}

/// Selectively reroute an arbitrary semantic candidate. This is the narrow
/// adapter needed by coordinators which reached their current candidate by a
/// processor other than the grid router (for example, width continuation).
/// The candidate is independently validated and converted into auditable
/// retained-branch evidence before the ordinary selective-reroute path runs.
pub fn reroute_candidate_branches_with_dut_grid_at_poses(
    problem: &Problem,
    candidate: &CandidateArtifact,
    reroute_branches: &BTreeSet<String>,
    config: &DutGridRoutingConfig,
) -> Result<DutGridRoutingResult, String> {
    let validation = validate_candidate(problem, candidate)?;
    let mut branches = BTreeMap::<String, BranchRoutingEvidence>::new();
    for trace in &candidate.traces {
        branches
            .entry(trace.branch.clone())
            .or_insert_with(|| BranchRoutingEvidence {
                branch: trace.branch.clone(),
                electrical_net: trace.electrical_net.clone(),
                width: trace.width,
                status: BranchRoutingStatus::Found,
                terminal_layer_searches: 0,
                expansions: 0,
                exact_edge_retries: 0,
                selected_cost: None,
                selected_grid_mm: None,
                grid_attempts: Vec::new(),
                point_count: trace.points.len(),
                via_count: trace.vias.len(),
                blockers: Vec::new(),
                tree_attachment: None,
                tree_attachment_attempts: Vec::new(),
                tree_attachment_searches_truncated: false,
                terminal_junction_attempts: Vec::new(),
                terminal_junction_searches_truncated: false,
            });
    }
    let branches = branches.into_values().collect::<Vec<_>>();
    let baseline = DutGridRoutingResult {
        candidate: candidate.clone(),
        validation,
        evidence: DutGridRoutingEvidence {
            strategy: "semantic-candidate-selective-reroute-baseline/v1".into(),
            config: config.clone(),
            branch_count: branches.len(),
            routed_branches: branches.len(),
            failed_branches: 0,
            searches: 0,
            expansions: 0,
            branches,
        },
    };
    reroute_problem_branches_with_dut_grid_at_poses(
        problem,
        &candidate.components,
        &baseline,
        reroute_branches,
        config,
    )
}

/// Add selected branches to an independently certified parent candidate while
/// treating every parent pose and trace as an immutable reservation.
///
/// This is deliberately narrower than selective rip-up: `added_branches` must
/// not already occur in the parent. Each call starts from the supplied parent,
/// so a portfolio can inspect several routing policies without one failed
/// trial mutating the next trial's baseline.
pub fn add_problem_branches_with_dut_grid_at_poses(
    problem: &Problem,
    parent: &CandidateArtifact,
    added_branches: &BTreeSet<String>,
    config: &DutGridRoutingConfig,
) -> Result<DutGridRoutingResult, String> {
    if added_branches.is_empty() {
        return Err("branch addition requires at least one added branch".into());
    }
    if let Some(branch) = parent
        .traces
        .iter()
        .find(|trace| added_branches.contains(&trace.branch))
    {
        return Err(format!(
            "branch addition parent already contains selected branch {}",
            branch.branch
        ));
    }
    let validation = validate_candidate(problem, parent)?;
    let branches = parent
        .traces
        .iter()
        .map(|trace| BranchRoutingEvidence {
            branch: trace.branch.clone(),
            electrical_net: trace.electrical_net.clone(),
            width: trace.width,
            status: BranchRoutingStatus::Found,
            terminal_layer_searches: 0,
            expansions: 0,
            exact_edge_retries: 0,
            selected_cost: None,
            selected_grid_mm: None,
            grid_attempts: Vec::new(),
            point_count: trace.points.len(),
            via_count: trace.vias.len(),
            blockers: Vec::new(),
            tree_attachment: None,
            tree_attachment_attempts: Vec::new(),
            tree_attachment_searches_truncated: false,
            terminal_junction_attempts: Vec::new(),
            terminal_junction_searches_truncated: false,
        })
        .collect::<Vec<_>>();
    let baseline = DutGridRoutingResult {
        candidate: parent.clone(),
        validation,
        evidence: DutGridRoutingEvidence {
            strategy: "immutable-parent-branch-addition-baseline/v1".into(),
            config: config.clone(),
            branch_count: branches.len(),
            routed_branches: branches.len(),
            failed_branches: 0,
            searches: 0,
            expansions: 0,
            branches,
        },
    };
    reroute_problem_branches_with_dut_grid_at_poses(
        problem,
        &parent.components,
        &baseline,
        added_branches,
        config,
    )
}

/// Route with bounded negotiated congestion at the problem's declared poses.
/// Copper from earlier branches in a pass is costly rather than forbidden;
/// only a congestion-free candidate which passes exact validation is complete.
pub fn route_problem_with_negotiated_dut_grid(
    problem: &Problem,
    config: &NegotiatedDutGridRoutingConfig,
) -> Result<NegotiatedDutGridRoutingResult, String> {
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
    route_problem_with_negotiated_dut_grid_at_poses(problem, &components, config)
}

/// Route with bounded negotiated congestion at an explicit placement.
pub fn route_problem_with_negotiated_dut_grid_at_poses(
    problem: &Problem,
    components: &[SolvedComponent],
    config: &NegotiatedDutGridRoutingConfig,
) -> Result<NegotiatedDutGridRoutingResult, String> {
    problem.check_schema()?;
    config.check()?;
    let grid = GridMap::new(problem, config.routing.grid_mm)?;
    let mut history_costs = vec![0_u32; grid.state_count()?];
    let mut attempts = Vec::new();
    let mut passes = Vec::new();
    let mut selected = 0;
    let mut selected_rank = None;
    let mut total_expansions = 0_u64;

    for pass in 0..config.maximum_passes {
        let attempt = route_problem_with_dut_grid_pass(
            problem,
            components,
            &config.routing,
            CopperReservation::Negotiated {
                congestion_cost: config.congestion_cost,
                history_costs: &history_costs,
            },
            None,
        )?;
        let overused = congested_grid_states(problem, &grid, &attempt.candidate)?;
        let exact_violations = attempt.validation.geometry.violations.len();
        total_expansions = total_expansions.saturating_add(attempt.evidence.expansions);
        let rank = (
            attempt.evidence.failed_branches,
            exact_violations,
            overused.len(),
            attempt.validation.electrical.findings.len(),
            attempt.evidence.expansions,
        );
        if selected_rank.is_none_or(|current| rank < current) {
            selected = attempts.len();
            selected_rank = Some(rank);
        }
        let pass_complete = attempt.complete() && overused.is_empty();
        passes.push(NegotiatedPassEvidence {
            pass,
            routed_branches: attempt.evidence.routed_branches,
            failed_branches: attempt.evidence.failed_branches,
            congested_cells: overused.len(),
            exact_violations,
            expansions: attempt.evidence.expansions,
        });
        attempts.push(attempt);
        if pass_complete {
            break;
        }
        for state in overused {
            history_costs[state] = history_costs[state].saturating_add(config.history_cost);
        }
    }

    let selected_routing = attempts[selected].clone();
    Ok(NegotiatedDutGridRoutingResult {
        candidate: selected_routing.candidate,
        validation: selected_routing.validation,
        evidence: NegotiatedDutGridRoutingEvidence {
            strategy: "testing-esp32-duts-negotiated-congestion-v1".into(),
            config: config.clone(),
            selected_pass: selected,
            completed_passes: attempts.len(),
            total_expansions,
            passes,
            routing: selected_routing.evidence,
        },
        attempts,
    })
}

fn route_problem_with_dut_grid_pass(
    problem: &Problem,
    components: &[SolvedComponent],
    config: &DutGridRoutingConfig,
    reservation: CopperReservation<'_>,
    fixed: Option<FixedRouting<'_>>,
) -> Result<DutGridRoutingResult, String> {
    problem.check_schema()?;
    config.check()?;
    if config.multi_terminal_routing == MultiTerminalRoutingPolicy::SharedCopperTree
        && !problem.electrical_nets.is_empty()
        && (fixed.is_some() || reservation.negotiated())
    {
        return Err(
            "generated shared-copper-tree branches are not yet defined for selective or negotiated rerouting"
                .into(),
        );
    }
    let placement = validate_geometry(problem, components, &[], &[]);
    if !placement.complete {
        let first = placement.violations.first();
        return Err(format!(
            "routing placement failed exact geometry with {} finding(s): {}{}",
            placement.violations.len(),
            first.map_or("unknown placement violation", |item| item.message.as_str()),
            first.map_or_else(String::new, |item| format!(
                " [{}: required {:.12}, actual {:.12}, objects={:?}]",
                item.code, item.required_distance, item.actual_distance, item.objects
            ))
        ));
    }

    let component_poses = component_pose_map(components)?;
    let mut branches = compile_branches(problem, &component_poses, config)?;
    order_branches(
        &mut branches,
        config.route_order,
        &config.priority_branches,
        &component_poses,
        problem,
    )?;
    preserve_shared_tree_growth_order(&mut branches);
    let attachments = compile_attachments(problem, &component_poses, &branches)?;
    let obstacles = raster_obstacles(problem, &component_poses)?;

    let mut graph_builders = BTreeMap::<String, GraphBuilder>::new();
    for ((net, terminal), attachment) in &attachments {
        graph_builders
            .entry(net.clone())
            .or_insert_with(|| GraphBuilder::new(BTreeMap::new()))
            .terminals
            .insert(terminal.clone(), attachment.clone());
    }
    let branches_by_id = branches
        .iter()
        .map(|branch| (branch.id.as_str(), branch))
        .collect::<BTreeMap<_, _>>();
    if let Some(fixed) = fixed {
        for branch in fixed.reroute_branches {
            if !branches_by_id.contains_key(branch.as_str()) {
                return Err(format!("selective reroute names unknown branch {branch}"));
            }
        }
        for graph in &fixed.baseline.candidate.route_graphs {
            let Some(builder) = graph_builders.get_mut(&graph.electrical_net) else {
                continue;
            };
            for node in &graph.nodes {
                if matches!(node.kind, SolvedRouteNodeKind::Junction { .. })
                    && node
                        .incident_branches
                        .iter()
                        .all(|branch| !fixed.reroute_branches.contains(branch))
                {
                    builder.add_junction(node.clone())?;
                }
            }
        }
    }

    let net_terminals = attachments.keys().fold(
        BTreeMap::<String, BTreeSet<TerminalKey>>::new(),
        |mut map, (net, terminal)| {
            map.entry(net.clone()).or_default().insert(terminal.clone());
            map
        },
    );
    let mut traces = fixed
        .map(|fixed| {
            fixed
                .baseline
                .candidate
                .traces
                .iter()
                .filter(|trace| !fixed.reroute_branches.contains(&trace.branch))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for trace in &traces {
        branches_by_id.get(trace.branch.as_str()).ok_or_else(|| {
            format!(
                "selective reroute baseline contains unknown branch {}",
                trace.branch
            )
        })?;
    }
    let mut branch_evidence = Vec::new();
    let mut failed_shared_nets = BTreeSet::new();
    for branch in &branches {
        if let Some(fixed) = fixed
            && !fixed.reroute_branches.contains(&branch.id)
        {
            let retained = fixed
                .baseline
                .evidence
                .branches
                .iter()
                .find(|evidence| evidence.branch == branch.id)
                .ok_or_else(|| {
                    format!(
                        "selective reroute baseline lacks evidence for branch {}",
                        branch.id
                    )
                })?;
            branch_evidence.push(retained_branch_evidence(retained));
            continue;
        }
        if branch.tree_growth_index.is_some() && failed_shared_nets.contains(&branch.electrical_net)
        {
            branch_evidence.push(tree_unavailable_evidence(branch));
            continue;
        }
        let tree_growth = if branch.tree_growth_index.is_some_and(|index| index > 0) {
            let ranked = ranked_tree_attachments(
                branch,
                &traces,
                &attachments,
                &net_terminals[&branch.electrical_net],
                config,
            );
            if ranked.is_empty() {
                failed_shared_nets.insert(branch.electrical_net.clone());
                branch_evidence.push(tree_unavailable_evidence(branch));
                continue;
            }
            Some(ranked)
        } else {
            None
        };
        let (mut result, routing_branch, tree_commit) = if let Some(ranked) = tree_growth {
            let (result, routing_branch, commit) = route_tree_attachment_trials(
                problem,
                config,
                &obstacles,
                &net_terminals,
                branch,
                &attachments,
                &traces,
                reservation,
                ranked,
            )?;
            (result, routing_branch, Some(commit))
        } else {
            let routing_branch = branch.clone();
            let from = &attachments[&(branch.electrical_net.clone(), branch.from.clone())];
            let to = &attachments[&(branch.electrical_net.clone(), branch.to.clone())];
            let result = route_branch(
                problem,
                config,
                &obstacles,
                &net_terminals,
                &routing_branch,
                from,
                to,
                &traces,
                reservation,
            )?;
            (result, routing_branch, None)
        };
        if let Some(mut trace) = result.trace.take() {
            if let Some(tree_commit) = tree_commit {
                let target = tree_commit.target;
                let searched_selection = tree_commit.searched;
                let mut selection = tree_commit.committed;
                let mut trimmed_prefix_points = tree_commit.trimmed_prefix_points;
                let mut new_segment_split = tree_commit.new_segment_split;
                let mut selected_search = searched_selection.clone();
                let mut selected_evidence = result.evidence.clone();
                let mut terminal_preference_selected = false;
                if let Some(slack) = config.terminal_junction_slack_mm {
                    let ordinary_distance = estimated_tree_terminal_distance(
                        &searched_selection,
                        &target,
                        &branch.electrical_net,
                        &attachments,
                        config,
                    );
                    let slack_cells = (slack / config.grid_mm).floor() as u64;
                    let to = &attachments[&(branch.electrical_net.clone(), target.clone())];
                    let eligible =
                        connected_terminal_tree_selections(&routing_branch, &traces, &attachments)
                            .into_iter()
                            .filter(|source| {
                                estimated_tree_terminal_distance(
                                    source,
                                    &target,
                                    &branch.electrical_net,
                                    &attachments,
                                    config,
                                ) <= ordinary_distance + slack_cells
                            })
                            .collect::<Vec<_>>();
                    result.evidence.terminal_junction_searches_truncated =
                        eligible.len() > config.maximum_terminal_junction_searches;
                    let mut selected_attempt = None;
                    for source in eligible
                        .into_iter()
                        .take(config.maximum_terminal_junction_searches)
                    {
                        let source_terminal = source
                            .source_terminal
                            .as_ref()
                            .expect("terminal tree selection")
                            .clone();
                        let source_attachment = TerminalAttachment {
                            position: source.position,
                            layers: vec![source.layer.clone()],
                        };
                        let mut alternative_branch = routing_branch.clone();
                        alternative_branch.from = source_terminal.clone();
                        alternative_branch.to = target.clone();
                        let mut alternative = route_branch(
                            problem,
                            config,
                            &obstacles,
                            &net_terminals,
                            &alternative_branch,
                            &source_attachment,
                            to,
                            &traces,
                            reservation,
                        )?;
                        let mut retained_length_mm = None;
                        let mut via_count = None;
                        let mut bend_count = None;
                        let mut candidate = None;
                        if let Some(mut alternative_trace) = alternative.trace.take() {
                            let alternative_trimmed = trim_tree_prefix_to_last_contact(
                                &mut alternative_trace,
                                &alternative_branch,
                                &traces,
                                &attachments,
                                &source,
                            )?;
                            retained_length_mm = Some(solved_trace_length(&alternative_trace));
                            via_count = Some(alternative_trace.vias.len());
                            bend_count = Some(solved_trace_bend_count(&alternative_trace));
                            candidate = Some((alternative_trace, alternative_trimmed));
                        }
                        let choose = candidate.as_ref().is_some_and(|(candidate, _)| {
                            terminal_branch_is_preferred(candidate, &trace, slack)
                        });
                        let attempt_index = result.evidence.terminal_junction_attempts.len();
                        result.evidence.terminal_junction_attempts.push(
                            TerminalJunctionAttemptEvidence {
                                source_terminal: format!(
                                    "{}.{}",
                                    source_terminal.component, source_terminal.pin
                                ),
                                status: alternative.evidence.status,
                                searches: alternative.evidence.terminal_layer_searches,
                                expansions: alternative.evidence.expansions,
                                retained_length_mm,
                                via_count,
                                bend_count,
                                selected: false,
                            },
                        );
                        accumulate_branch_search_work(&mut result.evidence, &alternative.evidence);
                        if choose {
                            let (alternative_trace, alternative_trimmed) =
                                candidate.expect("preferred terminal candidate");
                            trace = alternative_trace;
                            selection = alternative_trimmed.selection;
                            trimmed_prefix_points = alternative_trimmed.trimmed_prefix_points;
                            new_segment_split = alternative_trimmed.new_segment_split;
                            selected_search = source;
                            selected_evidence = alternative.evidence;
                            terminal_preference_selected = true;
                            selected_attempt = Some(attempt_index);
                        }
                    }
                    if let Some(selected_attempt) = selected_attempt {
                        result.evidence.terminal_junction_attempts[selected_attempt].selected =
                            true;
                    }
                }
                use_selected_branch_evidence(&mut result.evidence, &selected_evidence);
                result.evidence.tree_attachment = Some(tree_attachment_evidence(
                    &selected_search,
                    &selection,
                    &target,
                    trimmed_prefix_points,
                    new_segment_split,
                    terminal_preference_selected,
                ));
                result.evidence.point_count = trace.points.len();
                result.evidence.via_count = trace.vias.len();
                commit_tree_growth(
                    &mut traces,
                    graph_builders
                        .get_mut(&branch.electrical_net)
                        .expect("compiled electrical net"),
                    &mut trace,
                    &routing_branch,
                    &target,
                    &selection,
                )?;
            } else {
                traces.push(trace);
            }
        } else if branch.tree_growth_index.is_some() {
            failed_shared_nets.insert(branch.electrical_net.clone());
        }
        branch_evidence.push(result.evidence);
    }
    traces.sort_by(|left, right| left.branch.cmp(&right.branch));
    branch_evidence.sort_by(|left, right| left.branch.cmp(&right.branch));
    let route_graphs = graph_builders
        .into_iter()
        .map(|(net, builder)| builder.finish(net, &traces))
        .collect::<Result<Vec<_>, _>>()?;
    let candidate = CandidateArtifact {
        schema_version: CANDIDATE_SCHEMA_VERSION,
        components: components.to_vec(),
        traces,
        route_graphs,
    };
    let validation = validate_candidate(problem, &candidate)?;
    let routed_branches = branch_evidence
        .iter()
        .filter(|branch| branch.status == BranchRoutingStatus::Found)
        .count();
    let searches = branch_evidence
        .iter()
        .map(|branch| branch.terminal_layer_searches)
        .sum();
    let expansions = branch_evidence.iter().map(|branch| branch.expansions).sum();
    let evidence = DutGridRoutingEvidence {
        strategy: if config.terminal_junction_slack_mm.is_some()
            && config.maximum_tree_attachment_searches > 1
        {
            "testing-esp32-duts-astar-shared-portfolio-terminal-junction-v1"
        } else if config.terminal_junction_slack_mm.is_some() {
            "testing-esp32-duts-astar-shared-terminal-junction-v1"
        } else if config.maximum_tree_attachment_searches > 1 {
            "testing-esp32-duts-astar-shared-attachment-portfolio-v1"
        } else if config.multi_terminal_routing == MultiTerminalRoutingPolicy::SharedCopperTree {
            "testing-esp32-duts-astar-shared-copper-tree-v2"
        } else if fixed.is_some() {
            "testing-esp32-duts-astar-selective-reroute-v1"
        } else if reservation.negotiated() {
            "testing-esp32-duts-astar-negotiated-pass-v1"
        } else {
            "testing-esp32-duts-astar-semantic-v1"
        }
        .into(),
        config: config.clone(),
        branch_count: branches.len(),
        routed_branches,
        failed_branches: branches.len() - routed_branches,
        searches,
        expansions,
        branches: branch_evidence,
    };
    Ok(DutGridRoutingResult {
        candidate,
        validation,
        evidence,
    })
}

fn retained_branch_evidence(evidence: &BranchRoutingEvidence) -> BranchRoutingEvidence {
    BranchRoutingEvidence {
        branch: evidence.branch.clone(),
        electrical_net: evidence.electrical_net.clone(),
        width: evidence.width,
        status: evidence.status,
        terminal_layer_searches: 0,
        expansions: 0,
        exact_edge_retries: 0,
        selected_cost: evidence.selected_cost,
        selected_grid_mm: evidence.selected_grid_mm,
        grid_attempts: Vec::new(),
        point_count: evidence.point_count,
        via_count: evidence.via_count,
        blockers: evidence.blockers.clone(),
        tree_attachment: evidence.tree_attachment.clone(),
        tree_attachment_attempts: evidence.tree_attachment_attempts.clone(),
        tree_attachment_searches_truncated: evidence.tree_attachment_searches_truncated,
        terminal_junction_attempts: evidence.terminal_junction_attempts.clone(),
        terminal_junction_searches_truncated: evidence.terminal_junction_searches_truncated,
    }
}

fn tree_unavailable_evidence(branch: &BranchRequest) -> BranchRoutingEvidence {
    BranchRoutingEvidence {
        branch: branch.id.clone(),
        electrical_net: branch.electrical_net.clone(),
        width: branch.width,
        status: BranchRoutingStatus::TreeUnavailable,
        terminal_layer_searches: 0,
        expansions: 0,
        exact_edge_retries: 0,
        selected_cost: None,
        selected_grid_mm: None,
        grid_attempts: Vec::new(),
        point_count: 0,
        via_count: 0,
        blockers: Vec::new(),
        tree_attachment: None,
        tree_attachment_attempts: Vec::new(),
        tree_attachment_searches_truncated: false,
        terminal_junction_attempts: Vec::new(),
        terminal_junction_searches_truncated: false,
    }
}

fn component_pose_map(
    components: &[SolvedComponent],
) -> Result<BTreeMap<&str, &SolvedComponent>, String> {
    let mut result = BTreeMap::new();
    for component in components {
        if result.insert(component.id.as_str(), component).is_some() {
            return Err(format!("duplicate solved component {}", component.id));
        }
    }
    Ok(result)
}

fn compile_branches(
    problem: &Problem,
    poses: &BTreeMap<&str, &SolvedComponent>,
    config: &DutGridRoutingConfig,
) -> Result<Vec<BranchRequest>, String> {
    let mut branches = Vec::new();
    let mut branch_ids = BTreeSet::new();
    for (input_order, net) in problem.nets.iter().enumerate() {
        if !branch_ids.insert(net.id.clone()) {
            return Err(format!("duplicate route branch ID {}", net.id));
        }
        branches.push(BranchRequest {
            id: net.id.clone(),
            electrical_net: net.electrical_net.clone().unwrap_or_else(|| net.id.clone()),
            width: net.width,
            tension_weight: net.tension_weight,
            preferred_layer: net.layer.clone(),
            allowed_layers: allowed_layers(&net.layer, &net.allowed_layers),
            from: TerminalKey::from(&net.from),
            to: TerminalKey::from(&net.to),
            input_order,
            tree_growth_index: None,
        });
    }
    let mut input_order = branches.len();
    for net in &problem.electrical_nets {
        let mut terminals = net
            .terminals
            .iter()
            .map(TerminalKey::from)
            .collect::<Vec<_>>();
        terminals.sort();
        terminals.dedup();
        let positions = terminals
            .iter()
            .map(|terminal| logical_pin_position(problem, poses, terminal))
            .collect::<Result<Vec<_>, _>>()?;
        let edges = if config.multi_terminal_routing == MultiTerminalRoutingPolicy::SharedCopperTree
        {
            let root = positions[0];
            let mut pending = (1..terminals.len()).collect::<Vec<_>>();
            pending.sort_by_key(|&index| {
                let dx = ((positions[index].x - root.x).abs() / config.grid_mm).round() as u64;
                let dy = ((positions[index].y - root.y).abs() / config.grid_mm).round() as u64;
                (dx + dy, index)
            });
            pending.into_iter().map(|to| (0, to)).collect::<Vec<_>>()
        } else {
            let mut result = Vec::new();
            let mut connected = vec![false; terminals.len()];
            connected[0] = true;
            for _ in 0..terminals.len() - 1 {
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
                let (_, from, to) = best.expect("multi-terminal tree has a disconnected terminal");
                connected[to] = true;
                result.push((from, to));
            }
            result
        };
        for (branch_index, (from, to)) in edges.into_iter().enumerate() {
            let id = format!("{}#branch{}", net.id, branch_index);
            if !branch_ids.insert(id.clone()) {
                return Err(format!("duplicate generated route branch ID {id}"));
            }
            branches.push(BranchRequest {
                id,
                electrical_net: net.id.clone(),
                width: net.width,
                tension_weight: net.tension_weight,
                preferred_layer: net.layer.clone(),
                allowed_layers: allowed_layers(&net.layer, &net.allowed_layers),
                from: terminals[from].clone(),
                to: terminals[to].clone(),
                input_order,
                tree_growth_index: (config.multi_terminal_routing
                    == MultiTerminalRoutingPolicy::SharedCopperTree)
                    .then_some(branch_index),
            });
            input_order += 1;
        }
    }
    if branches.iter().any(|branch| branch.from == branch.to) {
        return Err("route branch connects a terminal to itself".into());
    }
    Ok(branches)
}

fn allowed_layers(primary: &str, declared: &[String]) -> Vec<String> {
    let mut layers = if declared.is_empty() {
        vec![primary.to_owned()]
    } else {
        declared.to_vec()
    };
    let mut seen = BTreeSet::new();
    layers.retain(|layer| seen.insert(layer.clone()));
    layers
}

fn order_branches(
    branches: &mut [BranchRequest],
    order: RouteOrder,
    priority_branches: &[String],
    poses: &BTreeMap<&str, &SolvedComponent>,
    problem: &Problem,
) -> Result<(), String> {
    let mut lengths = BTreeMap::new();
    for branch in branches.iter() {
        let from = logical_pin_position(problem, poses, &branch.from)?;
        let to = logical_pin_position(problem, poses, &branch.to)?;
        lengths.insert(branch.id.clone(), length(sub(from, to)));
    }
    branches.sort_by(|left, right| match order {
        RouteOrder::Input => left
            .input_order
            .cmp(&right.input_order)
            .then_with(|| left.id.cmp(&right.id)),
        RouteOrder::WidthDescending => right
            .width
            .total_cmp(&left.width)
            .then_with(|| left.id.cmp(&right.id)),
        RouteOrder::LongestFirst => lengths[&right.id]
            .total_cmp(&lengths[&left.id])
            .then_with(|| left.id.cmp(&right.id)),
    });
    let known = branches
        .iter()
        .map(|branch| branch.id.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = priority_branches
        .iter()
        .find(|branch| !known.contains(branch.as_str()))
    {
        return Err(format!("priority_branches names unknown branch {unknown}"));
    }
    let priorities = priority_branches
        .iter()
        .enumerate()
        .map(|(rank, branch)| (branch.as_str(), rank))
        .collect::<BTreeMap<_, _>>();
    branches.sort_by_key(|branch| {
        priorities
            .get(branch.id.as_str())
            .map_or((true, usize::MAX), |rank| (false, *rank))
    });
    Ok(())
}

fn preserve_shared_tree_growth_order(branches: &mut [BranchRequest]) {
    let nets = branches
        .iter()
        .filter(|branch| branch.tree_growth_index.is_some())
        .map(|branch| branch.electrical_net.clone())
        .collect::<BTreeSet<_>>();
    for net in nets {
        let positions = branches
            .iter()
            .enumerate()
            .filter(|(_, branch)| {
                branch.electrical_net == net && branch.tree_growth_index.is_some()
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let mut ordered = positions
            .iter()
            .map(|&index| branches[index].clone())
            .collect::<Vec<_>>();
        ordered.sort_by_key(|branch| branch.tree_growth_index);
        for (position, branch) in positions.into_iter().zip(ordered) {
            branches[position] = branch;
        }
    }
}

fn logical_pin_position(
    problem: &Problem,
    poses: &BTreeMap<&str, &SolvedComponent>,
    terminal: &TerminalKey,
) -> Result<Vec2, String> {
    let component = declared_component(problem, &terminal.component)?;
    let pin = component
        .pins
        .iter()
        .find(|pin| pin.id == terminal.pin)
        .ok_or_else(|| format!("unknown terminal {}.{}", terminal.component, terminal.pin))?;
    let pose = poses
        .get(terminal.component.as_str())
        .ok_or_else(|| format!("missing solved component {}", terminal.component))?;
    Ok(add(
        pose.position,
        rotate_degrees(pin.offset, pose.rotation_degrees),
    ))
}

fn compile_attachments(
    problem: &Problem,
    poses: &BTreeMap<&str, &SolvedComponent>,
    branches: &[BranchRequest],
) -> Result<BTreeMap<(String, TerminalKey), TerminalAttachment>, String> {
    let mut preferences = BTreeMap::<(String, TerminalKey), (String, String)>::new();
    for branch in branches {
        for terminal in [&branch.from, &branch.to] {
            let key = (branch.electrical_net.clone(), terminal.clone());
            let preference = (branch.id.clone(), branch.preferred_layer.clone());
            if preferences
                .get(&key)
                .is_none_or(|current| preference.0 < current.0)
            {
                preferences.insert(key, preference);
            }
        }
    }
    preferences
        .into_iter()
        .map(|((net, terminal), (_, preferred_layer))| {
            terminal_attachment(problem, poses, &terminal, &preferred_layer)
                .map(|attachment| ((net, terminal), attachment))
        })
        .collect()
}

fn terminal_attachment(
    problem: &Problem,
    poses: &BTreeMap<&str, &SolvedComponent>,
    terminal: &TerminalKey,
    preferred_layer: &str,
) -> Result<TerminalAttachment, String> {
    let component = declared_component(problem, &terminal.component)?;
    let pin = component
        .pins
        .iter()
        .find(|pin| pin.id == terminal.pin)
        .ok_or_else(|| format!("unknown terminal {}.{}", terminal.component, terminal.pin))?;
    let pose = poses
        .get(terminal.component.as_str())
        .ok_or_else(|| format!("missing solved component {}", terminal.component))?;
    if pin.pads.is_empty() {
        return Ok(TerminalAttachment {
            position: add(
                pose.position,
                rotate_degrees(pin.offset, pose.rotation_degrees),
            ),
            layers: problem
                .board
                .layers
                .iter()
                .map(|layer| layer.id.clone())
                .collect(),
        });
    }
    let mut pads = pin.pads.iter().collect::<Vec<_>>();
    pads.sort_by(|left, right| {
        (left.layer != preferred_layer)
            .cmp(&(right.layer != preferred_layer))
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.layer.cmp(&right.layer))
    });
    let selected = pads[0];
    let selected_center = pin.pad_local_center(selected);
    let mut layers = pin
        .pads
        .iter()
        .filter(|pad| length(sub(pin.pad_local_center(pad), selected_center)) <= EPSILON)
        .map(|pad| pad.layer.clone())
        .collect::<Vec<_>>();
    layers.sort();
    layers.dedup();
    Ok(TerminalAttachment {
        position: add(
            pose.position,
            rotate_degrees(selected_center, pose.rotation_degrees),
        ),
        layers,
    })
}

fn declared_component<'a>(problem: &'a Problem, id: &str) -> Result<&'a Component, String> {
    problem
        .components
        .iter()
        .find(|component| component.id == id)
        .ok_or_else(|| format!("unknown component {id}"))
}

fn shape_at(shape: &CopperShape, center: Vec2, rotation: f64) -> ExactShape {
    match shape {
        CopperShape::Circle { diameter } => ExactShape::Circle {
            center,
            radius: diameter * 0.5,
        },
        CopperShape::Rect {
            size,
            rotation_degrees,
        } => ExactShape::Rect(Obb {
            center,
            half_size: Vec2::new(size.x * 0.5, size.y * 0.5),
            rotation_degrees: rotation + rotation_degrees,
        }),
    }
}

fn raster_obstacles(
    problem: &Problem,
    poses: &BTreeMap<&str, &SolvedComponent>,
) -> Result<Vec<RasterObstacle>, String> {
    let mut result = Vec::new();
    for component in &problem.components {
        let pose = poses
            .get(component.id.as_str())
            .ok_or_else(|| format!("missing solved component {}", component.id))?;
        if component.body_is_routing_keepout {
            for layer in &problem.board.layers {
                result.push(RasterObstacle {
                    owner: component.id.clone(),
                    pin: None,
                    layer: layer.id.clone(),
                    kind: ObstacleKind::Body,
                    shape: ExactShape::Rect(Obb {
                        center: pose.position,
                        half_size: Vec2::new(pose.size.x * 0.5, pose.size.y * 0.5),
                        rotation_degrees: pose.rotation_degrees,
                    }),
                });
            }
        }
        for keepout in &component.routing_keepouts {
            let center = add(
                pose.position,
                rotate_degrees(keepout.offset, pose.rotation_degrees),
            );
            result.push(RasterObstacle {
                owner: component.id.clone(),
                pin: None,
                layer: keepout.layer.clone(),
                kind: ObstacleKind::Keepout,
                shape: shape_at(&keepout.shape, center, pose.rotation_degrees),
            });
        }
        for pin in &component.pins {
            for pad in &pin.pads {
                let center = add(
                    pose.position,
                    rotate_degrees(pin.pad_local_center(pad), pose.rotation_degrees),
                );
                result.push(RasterObstacle {
                    owner: component.id.clone(),
                    pin: Some(pin.id.clone()),
                    layer: pad.layer.clone(),
                    kind: ObstacleKind::Pad,
                    shape: shape_at(&pad.shape, center, pose.rotation_degrees),
                });
            }
        }
    }
    Ok(result)
}

fn ranked_tree_attachments(
    branch: &BranchRequest,
    traces: &[SolvedTrace],
    attachments: &BTreeMap<(String, TerminalKey), TerminalAttachment>,
    terminals: &BTreeSet<TerminalKey>,
    config: &DutGridRoutingConfig,
) -> Vec<(TreeAttachmentSelection, TerminalKey)> {
    let endpoint_nodes = traces
        .iter()
        .filter(|trace| trace.electrical_net == branch.electrical_net)
        .flat_map(|trace| [&trace.from_node, &trace.to_node])
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let pending = terminals
        .iter()
        .filter(|terminal| {
            !endpoint_nodes.contains(terminal_node_id(&branch.electrical_net, terminal).as_str())
        })
        .collect::<Vec<_>>();
    let mut candidates = Vec::<(
        (u64, String, usize, String, TerminalKey),
        TreeAttachmentSelection,
        TerminalKey,
    )>::new();
    for (trace_index, trace) in traces.iter().enumerate() {
        for point_index in 0..trace.points.len() {
            let Some(selection) =
                tree_vertex_selection(branch, traces, attachments, trace_index, point_index)
            else {
                continue;
            };
            for &target in &pending {
                let target_attachment =
                    &attachments[&(branch.electrical_net.clone(), target.clone())];
                let delta = sub(selection.position, target_attachment.position);
                let dx = (delta.x.abs() / config.grid_mm).round() as u64;
                let dy = (delta.y.abs() / config.grid_mm).round() as u64;
                let layer_cost = if target_attachment.layers.contains(&selection.layer) {
                    0
                } else {
                    u64::from(config.via_cost / config.straight_cost)
                };
                let key = (
                    dx + dy + layer_cost,
                    selection.source_branch.clone(),
                    selection.point_index,
                    selection.layer.clone(),
                    target.clone(),
                );
                candidates.push((key, selection.clone(), target.clone()));
            }
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    let mut seen = BTreeSet::new();
    let mut ranked = Vec::<(TreeAttachmentSelection, TerminalKey)>::new();
    for (_, selection, target) in candidates {
        let identity = (
            target.clone(),
            selection.node.clone(),
            selection.position.x.to_bits(),
            selection.position.y.to_bits(),
            selection.layer.clone(),
        );
        if !seen.insert(identity) {
            continue;
        }
        let too_close = ranked.iter().any(|(retained, retained_target)| {
            retained_target == &target
                && retained.layer == selection.layer
                && length(sub(retained.position, selection.position)) + EPSILON
                    < config.tree_attachment_minimum_spacing_mm
        });
        if !too_close {
            ranked.push((selection, target));
        }
    }
    ranked
}

fn tree_vertex_selection(
    branch: &BranchRequest,
    traces: &[SolvedTrace],
    attachments: &BTreeMap<(String, TerminalKey), TerminalAttachment>,
    trace_index: usize,
    point_index: usize,
) -> Option<TreeAttachmentSelection> {
    let trace = traces.get(trace_index)?;
    if trace.electrical_net != branch.electrical_net || trace.points.len() < 2 {
        return None;
    }
    let position = *trace.points.get(point_index)?;
    let last = trace.points.len() - 1;
    let (node, layer, interior) = if point_index == 0 {
        (
            trace.from_node.clone(),
            trace.segment_layers.first()?.clone(),
            false,
        )
    } else if point_index == last {
        (
            trace.to_node.clone(),
            trace.segment_layers.last()?.clone(),
            false,
        )
    } else {
        if trace.vias.iter().any(|via| via.point_index == point_index)
            || trace.segment_layers[point_index - 1] != trace.segment_layers[point_index]
        {
            return None;
        }
        (
            format!("junction:{}:growth:{}", branch.electrical_net, branch.id),
            trace.segment_layers[point_index].clone(),
            true,
        )
    };
    let source_terminal = (!interior)
        .then(|| {
            attachments
                .keys()
                .filter(|(net, _)| net == &branch.electrical_net)
                .map(|(_, terminal)| terminal)
                .find(|terminal| terminal_node_id(&branch.electrical_net, terminal) == node)
                .cloned()
        })
        .flatten();
    Some(TreeAttachmentSelection {
        trace_index,
        source_branch: trace.branch.clone(),
        point_index,
        existing_segment_index: None,
        node,
        position,
        layer,
        interior,
        source_terminal,
    })
}

fn tree_segment_selection(
    branch: &BranchRequest,
    traces: &[SolvedTrace],
    trace_index: usize,
    segment_index: usize,
    position: Vec2,
) -> Option<TreeAttachmentSelection> {
    let trace = traces.get(trace_index)?;
    if trace.electrical_net != branch.electrical_net
        || segment_index >= trace.segment_layers.len()
        || segment_index + 1 >= trace.points.len()
    {
        return None;
    }
    Some(TreeAttachmentSelection {
        trace_index,
        source_branch: trace.branch.clone(),
        point_index: segment_index + 1,
        existing_segment_index: Some(segment_index),
        node: format!("junction:{}:growth:{}", branch.electrical_net, branch.id),
        position,
        layer: trace.segment_layers[segment_index].clone(),
        interior: true,
        source_terminal: None,
    })
}

fn prefer_path_tree_contact(candidate: &PathTreeContact, current: &PathTreeContact) -> bool {
    if candidate.progress > current.progress + EPSILON {
        return true;
    }
    if (candidate.progress - current.progress).abs() > EPSILON {
        return false;
    }
    let candidate_key = (
        candidate.selection.existing_segment_index.is_some(),
        candidate.selection.interior,
        candidate.selection.source_branch.as_str(),
        candidate.selection.point_index,
        candidate.selection.node.as_str(),
    );
    let current_key = (
        current.selection.existing_segment_index.is_some(),
        current.selection.interior,
        current.selection.source_branch.as_str(),
        current.selection.point_index,
        current.selection.node.as_str(),
    );
    candidate_key < current_key
}

fn trim_tree_prefix_to_last_contact(
    trace: &mut SolvedTrace,
    branch: &BranchRequest,
    existing: &[SolvedTrace],
    attachments: &BTreeMap<(String, TerminalKey), TerminalAttachment>,
    searched: &TreeAttachmentSelection,
) -> Result<TrimmedTreePrefix, String> {
    let mut last_contact = None::<PathTreeContact>;
    for trace_point_index in 1..trace.points.len().saturating_sub(1) {
        if trace
            .vias
            .iter()
            .any(|via| via.point_index == trace_point_index)
        {
            continue;
        }
        let Some(outgoing_layer) = trace.segment_layers.get(trace_point_index) else {
            continue;
        };
        let mut contact = None::<((bool, String, usize, String), TreeAttachmentSelection)>;
        for (existing_index, existing_trace) in existing.iter().enumerate() {
            for existing_point_index in 0..existing_trace.points.len() {
                let Some(selection) = tree_vertex_selection(
                    branch,
                    existing,
                    attachments,
                    existing_index,
                    existing_point_index,
                ) else {
                    continue;
                };
                if selection.layer != *outgoing_layer
                    || length(sub(selection.position, trace.points[trace_point_index])) > EPSILON
                {
                    continue;
                }
                let key = (
                    selection.interior,
                    selection.source_branch.clone(),
                    selection.point_index,
                    selection.node.clone(),
                );
                if contact.as_ref().is_none_or(|(current, _)| key < *current) {
                    contact = Some((key, selection));
                }
            }
        }
        if let Some((_, selection)) = contact {
            let candidate = PathTreeContact {
                progress: trace_point_index as f64,
                new_segment_index: trace_point_index.saturating_sub(1),
                new_segment_parameter: 1.0,
                selection,
            };
            if last_contact
                .as_ref()
                .is_none_or(|current| prefer_path_tree_contact(&candidate, current))
            {
                last_contact = Some(candidate);
            }
        }
    }

    for (new_segment_index, new_segment) in trace.points.windows(2).enumerate() {
        let Some(new_layer) = trace.segment_layers.get(new_segment_index) else {
            continue;
        };
        let new_direction = sub(new_segment[1], new_segment[0]);
        for (existing_index, existing_trace) in existing.iter().enumerate() {
            if existing_trace.electrical_net != branch.electrical_net {
                continue;
            }
            for (existing_segment_index, existing_segment) in
                existing_trace.points.windows(2).enumerate()
            {
                if existing_trace.segment_layers.get(existing_segment_index) != Some(new_layer) {
                    continue;
                }
                let existing_direction = sub(existing_segment[1], existing_segment[0]);
                if cross(new_direction, existing_direction).abs() <= EPSILON {
                    // Collinear overlap is handled by the existing vertex-prefix
                    // rule. Its continuum of possible contacts has no unique
                    // graph split point.
                    continue;
                }
                let proximity = segment_proximity(
                    new_segment[0],
                    new_segment[1],
                    existing_segment[0],
                    existing_segment[1],
                );
                if proximity.distance > EPSILON {
                    continue;
                }
                let progress = new_segment_index as f64 + proximity.parameter_a;
                if progress <= EPSILON || progress >= (trace.points.len() - 1) as f64 - EPSILON {
                    continue;
                }
                let new_point_index = if proximity.parameter_a <= EPSILON {
                    Some(new_segment_index)
                } else if proximity.parameter_a >= 1.0 - EPSILON {
                    Some(new_segment_index + 1)
                } else {
                    None
                };
                if new_point_index.is_some_and(|point_index| {
                    trace.vias.iter().any(|via| via.point_index == point_index)
                }) {
                    continue;
                }
                let selection = if proximity.parameter_b <= EPSILON {
                    tree_vertex_selection(
                        branch,
                        existing,
                        attachments,
                        existing_index,
                        existing_segment_index,
                    )
                } else if proximity.parameter_b >= 1.0 - EPSILON {
                    tree_vertex_selection(
                        branch,
                        existing,
                        attachments,
                        existing_index,
                        existing_segment_index + 1,
                    )
                } else {
                    tree_segment_selection(
                        branch,
                        existing,
                        existing_index,
                        existing_segment_index,
                        scale(add(proximity.point_a, proximity.point_b), 0.5),
                    )
                };
                let Some(selection) = selection.filter(|selection| selection.layer == *new_layer)
                else {
                    continue;
                };
                let candidate = PathTreeContact {
                    progress,
                    new_segment_index,
                    new_segment_parameter: proximity.parameter_a,
                    selection,
                };
                if last_contact
                    .as_ref()
                    .is_none_or(|current| prefer_path_tree_contact(&candidate, current))
                {
                    last_contact = Some(candidate);
                }
            }
        }
    }

    let Some(contact) = last_contact else {
        return Ok(TrimmedTreePrefix {
            selection: searched.clone(),
            trimmed_prefix_points: 0,
            new_segment_split: false,
        });
    };
    let mut new_segment_split = false;
    let prefix_points = if contact.new_segment_parameter > EPSILON
        && contact.new_segment_parameter < 1.0 - EPSILON
    {
        let insertion_index = contact.new_segment_index + 1;
        trace
            .points
            .insert(insertion_index, contact.selection.position);
        let layer = trace.segment_layers[contact.new_segment_index].clone();
        trace.segment_layers.insert(insertion_index, layer);
        for via in &mut trace.vias {
            if via.point_index >= insertion_index {
                via.point_index += 1;
            }
        }
        new_segment_split = true;
        insertion_index
    } else if contact.new_segment_parameter <= EPSILON {
        contact.new_segment_index
    } else {
        contact.new_segment_index + 1
    };
    trace.points = trace.points[prefix_points..].to_vec();
    trace.segment_layers = trace.segment_layers[prefix_points..].to_vec();
    trace.vias = trace
        .vias
        .iter()
        .filter(|via| via.point_index > prefix_points)
        .cloned()
        .map(|mut via| {
            via.point_index -= prefix_points;
            via
        })
        .collect();
    trace.layer = trace
        .segment_layers
        .first()
        .cloned()
        .ok_or_else(|| "tree-prefix trimming removed the whole routed branch".to_string())?;
    Ok(TrimmedTreePrefix {
        selection: contact.selection,
        trimmed_prefix_points: prefix_points,
        new_segment_split,
    })
}

fn connected_terminal_tree_selections(
    branch: &BranchRequest,
    traces: &[SolvedTrace],
    attachments: &BTreeMap<(String, TerminalKey), TerminalAttachment>,
) -> Vec<TreeAttachmentSelection> {
    let mut terminals = BTreeMap::<TerminalKey, TreeAttachmentSelection>::new();
    for (trace_index, trace) in traces.iter().enumerate() {
        if trace.points.len() < 2 {
            continue;
        }
        for point_index in [0, trace.points.len() - 1] {
            let Some(selection) =
                tree_vertex_selection(branch, traces, attachments, trace_index, point_index)
            else {
                continue;
            };
            let Some(terminal) = selection.source_terminal.clone() else {
                continue;
            };
            let candidate_key = (
                selection.source_branch.as_str(),
                selection.point_index,
                selection.layer.as_str(),
            );
            if terminals.get(&terminal).is_none_or(|current| {
                candidate_key
                    < (
                        current.source_branch.as_str(),
                        current.point_index,
                        current.layer.as_str(),
                    )
            }) {
                terminals.insert(terminal, selection);
            }
        }
    }
    terminals.into_values().collect()
}

fn estimated_tree_terminal_distance(
    selection: &TreeAttachmentSelection,
    target: &TerminalKey,
    electrical_net: &str,
    attachments: &BTreeMap<(String, TerminalKey), TerminalAttachment>,
    config: &DutGridRoutingConfig,
) -> u64 {
    let target = &attachments[&(electrical_net.to_owned(), target.clone())];
    let delta = sub(selection.position, target.position);
    let dx = (delta.x.abs() / config.grid_mm).round() as u64;
    let dy = (delta.y.abs() / config.grid_mm).round() as u64;
    let layer_cost = if target.layers.contains(&selection.layer) {
        0
    } else {
        u64::from(config.via_cost / config.straight_cost)
    };
    dx + dy + layer_cost
}

fn solved_trace_length(trace: &SolvedTrace) -> f64 {
    trace
        .points
        .windows(2)
        .map(|segment| length(sub(segment[1], segment[0])))
        .sum()
}

fn solved_trace_bend_count(trace: &SolvedTrace) -> usize {
    let mut bends = 0;
    let mut previous = None::<(Vec2, &str)>;
    for (index, segment) in trace.points.windows(2).enumerate() {
        let direction = sub(segment[1], segment[0]);
        let layer = trace.segment_layers[index].as_str();
        if let Some((previous_direction, previous_layer)) = previous
            && previous_layer == layer
        {
            let cross = previous_direction.x * direction.y - previous_direction.y * direction.x;
            let dot = previous_direction.x * direction.x + previous_direction.y * direction.y;
            if cross.abs() > EPSILON || dot <= 0.0 {
                bends += 1;
            }
        }
        previous = Some((direction, layer));
    }
    bends
}

fn tree_growth_trial_is_preferred(
    candidate: &TreeGrowthTrial,
    current: &TreeGrowthTrial,
    objective: TreeAttachmentPortfolioObjective,
) -> bool {
    let candidate_trace = candidate
        .result
        .trace
        .as_ref()
        .expect("successful tree-growth trial");
    let current_trace = current
        .result
        .trace
        .as_ref()
        .expect("successful tree-growth trial");
    let candidate_length = solved_trace_length(candidate_trace);
    let current_length = solved_trace_length(current_trace);
    let candidate_bends = solved_trace_bend_count(candidate_trace);
    let current_bends = solved_trace_bend_count(current_trace);
    let candidate_vias = candidate_trace.vias.len();
    let current_vias = current_trace.vias.len();
    let compare_length = || {
        if candidate_length + EPSILON < current_length {
            Some(true)
        } else if current_length + EPSILON < candidate_length {
            Some(false)
        } else {
            None
        }
    };
    let preferred = match objective {
        TreeAttachmentPortfolioObjective::RetainedLength => compare_length()
            .or_else(|| (candidate_vias != current_vias).then_some(candidate_vias < current_vias))
            .or_else(|| {
                (candidate_bends != current_bends).then_some(candidate_bends < current_bends)
            }),
        TreeAttachmentPortfolioObjective::ViaCountThenLength => (candidate_vias != current_vias)
            .then_some(candidate_vias < current_vias)
            .or_else(compare_length)
            .or_else(|| {
                (candidate_bends != current_bends).then_some(candidate_bends < current_bends)
            }),
        TreeAttachmentPortfolioObjective::RouterCost => {
            let candidate_cost = candidate.result.evidence.selected_cost.unwrap_or(u32::MAX);
            let current_cost = current.result.evidence.selected_cost.unwrap_or(u32::MAX);
            (candidate_cost != current_cost)
                .then_some(candidate_cost < current_cost)
                .or_else(compare_length)
                .or_else(|| {
                    (candidate_vias != current_vias).then_some(candidate_vias < current_vias)
                })
                .or_else(|| {
                    (candidate_bends != current_bends).then_some(candidate_bends < current_bends)
                })
        }
    };
    if let Some(preferred) = preferred {
        return preferred;
    }
    (
        &candidate.target,
        candidate.searched.source_branch.as_str(),
        candidate.searched.point_index,
        candidate.searched.layer.as_str(),
    ) < (
        &current.target,
        current.searched.source_branch.as_str(),
        current.searched.point_index,
        current.searched.layer.as_str(),
    )
}

#[allow(clippy::too_many_arguments)]
fn route_tree_attachment_trials(
    problem: &Problem,
    config: &DutGridRoutingConfig,
    obstacles: &[RasterObstacle],
    net_terminals: &BTreeMap<String, BTreeSet<TerminalKey>>,
    branch: &BranchRequest,
    attachments: &BTreeMap<(String, TerminalKey), TerminalAttachment>,
    existing: &[SolvedTrace],
    reservation: CopperReservation<'_>,
    ranked: Vec<(TreeAttachmentSelection, TerminalKey)>,
) -> Result<(BranchSearchResult, BranchRequest, TreeGrowthCommit), String> {
    let searches_truncated = ranked.len() > config.maximum_tree_attachment_searches;
    let mut trials = Vec::new();
    for (searched, target) in ranked
        .into_iter()
        .take(config.maximum_tree_attachment_searches)
    {
        let from = TerminalAttachment {
            position: searched.position,
            layers: vec![searched.layer.clone()],
        };
        let to = &attachments[&(branch.electrical_net.clone(), target.clone())];
        let mut routing_branch = branch.clone();
        routing_branch.from = searched
            .source_terminal
            .clone()
            .unwrap_or_else(|| target.clone());
        routing_branch.to = target.clone();
        let mut result = route_branch(
            problem,
            config,
            obstacles,
            net_terminals,
            &routing_branch,
            &from,
            to,
            existing,
            reservation,
        )?;
        let trimmed = if let Some(trace) = &mut result.trace {
            Some(trim_tree_prefix_to_last_contact(
                trace,
                &routing_branch,
                existing,
                attachments,
                &searched,
            )?)
        } else {
            None
        };
        trials.push(TreeGrowthTrial {
            searched,
            target,
            routing_branch,
            result,
            trimmed,
        });
    }
    if trials.is_empty() {
        return Err("shared-tree growth has no attachment trial".into());
    }
    let selected_index = trials
        .iter()
        .enumerate()
        .filter(|(_, trial)| trial.result.trace.is_some())
        .fold(None, |best: Option<usize>, (index, trial)| {
            if best.is_none_or(|current| {
                tree_growth_trial_is_preferred(
                    trial,
                    &trials[current],
                    config.tree_attachment_portfolio_objective,
                )
            }) {
                Some(index)
            } else {
                best
            }
        })
        .unwrap_or(0);
    let attempts = trials
        .iter()
        .enumerate()
        .map(|(index, trial)| TreeAttachmentAttemptEvidence {
            target_terminal: format!("{}.{}", trial.target.component, trial.target.pin),
            source_branch: trial.searched.source_branch.clone(),
            point_index: trial.searched.point_index,
            position: trial.searched.position,
            layer: trial.searched.layer.clone(),
            status: trial.result.evidence.status,
            searches: trial.result.evidence.terminal_layer_searches,
            expansions: trial.result.evidence.expansions,
            committed_position: trial
                .trimmed
                .as_ref()
                .map(|trimmed| trimmed.selection.position),
            trimmed_prefix_points: trial
                .trimmed
                .as_ref()
                .map_or(0, |trimmed| trimmed.trimmed_prefix_points),
            retained_length_mm: trial.result.trace.as_ref().map(solved_trace_length),
            via_count: trial.result.trace.as_ref().map(|trace| trace.vias.len()),
            bend_count: trial.result.trace.as_ref().map(solved_trace_bend_count),
            selected: index == selected_index,
        })
        .collect::<Vec<_>>();
    let mut aggregate_evidence = trials[0].result.evidence.clone();
    for trial in trials.iter().skip(1) {
        accumulate_branch_search_work(&mut aggregate_evidence, &trial.result.evidence);
    }
    use_selected_branch_evidence(
        &mut aggregate_evidence,
        &trials[selected_index].result.evidence,
    );
    let selected = trials.remove(selected_index);
    let TreeGrowthTrial {
        searched,
        target,
        routing_branch,
        mut result,
        trimmed,
    } = selected;
    aggregate_evidence.tree_attachment_attempts = attempts;
    aggregate_evidence.tree_attachment_searches_truncated = searches_truncated;
    result.evidence = aggregate_evidence;
    let (committed, trimmed_prefix_points, new_segment_split) = trimmed.map_or_else(
        || (searched.clone(), 0, false),
        |trimmed| {
            (
                trimmed.selection,
                trimmed.trimmed_prefix_points,
                trimmed.new_segment_split,
            )
        },
    );
    Ok((
        result,
        routing_branch,
        TreeGrowthCommit {
            searched,
            committed,
            target,
            trimmed_prefix_points,
            new_segment_split,
        },
    ))
}

fn terminal_branch_is_preferred(
    candidate: &SolvedTrace,
    current: &SolvedTrace,
    slack: f64,
) -> bool {
    if candidate.vias.len() != current.vias.len() {
        return candidate.vias.len() < current.vias.len();
    }
    let candidate_length = solved_trace_length(candidate);
    let current_length = solved_trace_length(current);
    if candidate_length > current_length + slack + EPSILON {
        return false;
    }
    if candidate_length + EPSILON < current_length {
        return true;
    }
    solved_trace_bend_count(candidate) <= solved_trace_bend_count(current)
}

fn accumulate_branch_search_work(
    total: &mut BranchRoutingEvidence,
    attempt: &BranchRoutingEvidence,
) {
    total.terminal_layer_searches += attempt.terminal_layer_searches;
    total.expansions += attempt.expansions;
    total.exact_edge_retries += attempt.exact_edge_retries;
    total.grid_attempts.extend(attempt.grid_attempts.clone());
}

fn use_selected_branch_evidence(
    total: &mut BranchRoutingEvidence,
    selected: &BranchRoutingEvidence,
) {
    total.status = selected.status;
    total.selected_cost = selected.selected_cost;
    total.selected_grid_mm = selected.selected_grid_mm;
    total.point_count = selected.point_count;
    total.via_count = selected.via_count;
    total.blockers = selected.blockers.clone();
}

fn tree_attachment_evidence(
    searched: &TreeAttachmentSelection,
    committed: &TreeAttachmentSelection,
    target: &TerminalKey,
    trimmed_prefix_points: usize,
    new_segment_split: bool,
    terminal_preference_selected: bool,
) -> TreeAttachmentEvidence {
    TreeAttachmentEvidence {
        target_terminal: format!("{}.{}", target.component, target.pin),
        searched_source_branch: searched.source_branch.clone(),
        searched_point_index: searched.point_index,
        searched_position: searched.position,
        searched_layer: searched.layer.clone(),
        source_branch: committed.source_branch.clone(),
        point_index: committed.point_index,
        node: committed.node.clone(),
        position: committed.position,
        layer: committed.layer.clone(),
        interior: committed.interior,
        existing_segment_split: committed.existing_segment_index.is_some(),
        new_segment_split,
        trimmed_prefix_points,
        terminal_preference_selected,
    }
}

fn junction_sector(junction: &str) -> TerminalSector {
    TerminalSector {
        component: "@junction".into(),
        sector: junction.into(),
    }
}

fn commit_tree_growth(
    traces: &mut Vec<SolvedTrace>,
    graph: &mut GraphBuilder,
    trace: &mut SolvedTrace,
    branch: &BranchRequest,
    target: &TerminalKey,
    selection: &TreeAttachmentSelection,
) -> Result<(), String> {
    let source_branch = traces
        .get(selection.trace_index)
        .ok_or_else(|| "tree attachment references a missing trace".to_string())?
        .branch
        .clone();
    if selection.interior {
        let junction = SolvedRouteNode {
            id: selection.node.clone(),
            electrical_net: branch.electrical_net.clone(),
            position: selection.position,
            incident_branches: Vec::new(),
            kind: SolvedRouteNodeKind::Junction {
                layer: selection.layer.clone(),
                movable: true,
            },
        };
        graph.add_junction(junction)?;
        split_tree_trace(traces, selection, branch)?;
    }
    trace.from_node = selection.node.clone();
    trace.to_node = terminal_node_id(&branch.electrical_net, target);
    trace.route_class.from = selection.source_terminal.as_ref().map_or_else(
        || junction_sector(&selection.node),
        |terminal| TerminalSector {
            component: terminal.component.clone(),
            sector: terminal.pin.clone(),
        },
    );
    trace.route_class.to = TerminalSector {
        component: target.component.clone(),
        sector: target.pin.clone(),
    };
    trace.route_basis_fingerprint = None;
    debug_assert_eq!(source_branch, traces[selection.trace_index].branch);
    traces.push(trace.clone());
    Ok(())
}

fn split_tree_trace(
    traces: &mut Vec<SolvedTrace>,
    selection: &TreeAttachmentSelection,
    growing_branch: &BranchRequest,
) -> Result<(), String> {
    if let Some(segment_index) = selection.existing_segment_index {
        let trace = traces
            .get_mut(selection.trace_index)
            .ok_or_else(|| "tree segment split references a missing trace".to_string())?;
        if segment_index >= trace.segment_layers.len()
            || segment_index + 1 >= trace.points.len()
            || trace.segment_layers[segment_index] != selection.layer
        {
            return Err("tree segment split references invalid geometry".into());
        }
        let insertion_index = segment_index + 1;
        if insertion_index != selection.point_index {
            return Err("tree segment split point index is inconsistent".into());
        }
        trace.points.insert(insertion_index, selection.position);
        let layer = trace.segment_layers[segment_index].clone();
        trace.segment_layers.insert(insertion_index, layer);
        for via in &mut trace.vias {
            if via.point_index >= insertion_index {
                via.point_index += 1;
            }
        }
    }
    let original = traces
        .get(selection.trace_index)
        .ok_or_else(|| "tree split references a missing trace".to_string())?
        .clone();
    let point_index = selection.point_index;
    if point_index == 0 || point_index + 1 >= original.points.len() {
        return Err("tree split point is not interior".into());
    }
    if original
        .vias
        .iter()
        .any(|via| via.point_index == point_index)
    {
        return Err("tree split point coincides with a via".into());
    }
    let tail_id = format!("{}::split::{}", original.branch, growing_branch.id);
    if traces.iter().any(|trace| trace.branch == tail_id) {
        return Err(format!("tree split branch ID {tail_id} already exists"));
    }
    let mut tail = original.clone();
    tail.branch = tail_id.clone();
    tail.net = tail_id;
    tail.points = original.points[point_index..].to_vec();
    tail.segment_layers = original.segment_layers[point_index..].to_vec();
    tail.vias = original
        .vias
        .iter()
        .filter(|via| via.point_index > point_index)
        .cloned()
        .map(|mut via| {
            via.point_index -= point_index;
            via
        })
        .collect();
    tail.from_node = selection.node.clone();
    tail.route_class.from = junction_sector(&selection.node);
    tail.route_basis_fingerprint = None;
    tail.layer = tail.segment_layers[0].clone();

    let head = &mut traces[selection.trace_index];
    head.points.truncate(point_index + 1);
    head.segment_layers.truncate(point_index);
    head.vias.retain(|via| via.point_index < point_index);
    head.to_node = selection.node.clone();
    head.route_class.to = junction_sector(&selection.node);
    head.route_basis_fingerprint = None;
    traces.push(tail);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn route_branch(
    problem: &Problem,
    config: &DutGridRoutingConfig,
    obstacles: &[RasterObstacle],
    net_terminals: &BTreeMap<String, BTreeSet<TerminalKey>>,
    branch: &BranchRequest,
    from: &TerminalAttachment,
    to: &TerminalAttachment,
    existing: &[SolvedTrace],
    reservation: CopperReservation<'_>,
) -> Result<BranchSearchResult, String> {
    let mut total_searches = 0;
    let mut total_expansions = 0_u64;
    let mut blocker_hits = BTreeMap::<(RoutingBlockerKind, String), BlockerHitAccumulator>::new();
    let mut router = DutAStar;
    let mut grid_attempts = Vec::new();
    let mut start_layers = from.layers.clone();
    let mut finish_layers = to.layers.clone();
    start_layers.sort();
    finish_layers.sort();
    let grid_sizes = std::iter::once(config.grid_mm)
        .chain(config.retry_grid_mm.iter().copied())
        .collect::<Vec<_>>();
    for grid_mm in grid_sizes {
        let grid = GridMap::new(problem, grid_mm)?;
        let classification = FrontierClassificationContext {
            problem,
            config,
            grid: &grid,
            obstacles,
            terminals: &net_terminals[&branch.electrical_net],
            branch,
            existing,
        };
        let path_validation = PathValidationContext {
            problem,
            grid: &grid,
            obstacles,
            terminals: &net_terminals[&branch.electrical_net],
            branch,
            from,
            to,
            existing,
            allow_existing_copper_overlap: reservation.negotiated(),
        };
        let mut best: Option<(u32, usize, Vec<GridPosition>)> = None;
        let mut searches = 0;
        let mut expansions = 0_u64;
        let mut exact_edge_retries = 0;
        let mut exhausted = false;
        let mut exact_geometry_rejected = false;
        for start_layer in &start_layers {
            for finish_layer in &finish_layers {
                let start = grid.snap(from.position, start_layer)?;
                let finish = grid.snap(to.position, finish_layer)?;
                let mut request = raster_request(
                    problem,
                    config,
                    &grid,
                    obstacles,
                    net_terminals,
                    branch,
                    start,
                    finish,
                    existing,
                    reservation,
                )?;
                loop {
                    searches += 1;
                    match router.route(&request)? {
                        GridRouteOutcome::Found {
                            path,
                            cost,
                            expansions: work,
                        } => {
                            expansions += u64::from(work);
                            if let Some((first, second)) =
                                first_invalid_path_transition(&path_validation, &path)
                            {
                                exact_geometry_rejected = true;
                                if exact_edge_retries >= config.maximum_exact_edge_retries {
                                    break;
                                }
                                request.block_planar_transition(first, second)?;
                                exact_edge_retries += 1;
                                continue;
                            }
                            let via_count = path
                                .windows(2)
                                .filter(|pair| pair[0].layer != pair[1].layer)
                                .count();
                            if best.as_ref().is_none_or(|current| {
                                (cost, via_count, path.len())
                                    < (current.0, current.1, current.2.len())
                            }) {
                                best = Some((cost, via_count, path));
                            }
                            break;
                        }
                        GridRouteOutcome::NoPath {
                            expansions: work,
                            blocked_frontier,
                        } => {
                            expansions += u64::from(work);
                            classify_frontier_blockers(
                                &classification,
                                &blocked_frontier,
                                &mut blocker_hits,
                            );
                            break;
                        }
                        GridRouteOutcome::BudgetExhausted {
                            expansions: work,
                            blocked_frontier,
                        } => {
                            expansions += u64::from(work);
                            exhausted = true;
                            classify_frontier_blockers(
                                &classification,
                                &blocked_frontier,
                                &mut blocker_hits,
                            );
                            break;
                        }
                    }
                }
            }
        }
        total_searches += searches;
        total_expansions += expansions;
        let status = if best.is_some() {
            BranchRoutingStatus::Found
        } else if exhausted {
            BranchRoutingStatus::BudgetExhausted
        } else if exact_geometry_rejected {
            BranchRoutingStatus::ExactGeometryRejected
        } else {
            BranchRoutingStatus::NoPath
        };
        grid_attempts.push(GridRoutingAttemptEvidence {
            grid_mm,
            status,
            terminal_layer_searches: searches,
            expansions,
            exact_edge_retries,
        });
        if let Some((cost, _, path)) = best {
            let trace = solved_trace(problem, &grid, branch, from, to, &path)?;
            let evidence = BranchRoutingEvidence {
                branch: branch.id.clone(),
                electrical_net: branch.electrical_net.clone(),
                width: branch.width,
                status: BranchRoutingStatus::Found,
                terminal_layer_searches: total_searches,
                expansions: total_expansions,
                exact_edge_retries: grid_attempts
                    .iter()
                    .map(|attempt| attempt.exact_edge_retries)
                    .sum(),
                selected_cost: Some(cost),
                selected_grid_mm: Some(grid_mm),
                grid_attempts,
                point_count: trace.points.len(),
                via_count: trace.vias.len(),
                blockers: blocker_evidence(blocker_hits),
                tree_attachment: None,
                tree_attachment_attempts: Vec::new(),
                tree_attachment_searches_truncated: false,
                terminal_junction_attempts: Vec::new(),
                terminal_junction_searches_truncated: false,
            };
            return Ok(BranchSearchResult {
                trace: Some(trace),
                evidence,
            });
        }
        if exhausted {
            break;
        }
    }
    let status = grid_attempts
        .last()
        .map_or(BranchRoutingStatus::NoPath, |attempt| attempt.status);
    Ok(BranchSearchResult {
        trace: None,
        evidence: BranchRoutingEvidence {
            branch: branch.id.clone(),
            electrical_net: branch.electrical_net.clone(),
            width: branch.width,
            status,
            terminal_layer_searches: total_searches,
            expansions: total_expansions,
            exact_edge_retries: grid_attempts
                .iter()
                .map(|attempt| attempt.exact_edge_retries)
                .sum(),
            selected_cost: None,
            selected_grid_mm: None,
            grid_attempts,
            point_count: 0,
            via_count: 0,
            blockers: blocker_evidence(blocker_hits),
            tree_attachment: None,
            tree_attachment_attempts: Vec::new(),
            tree_attachment_searches_truncated: false,
            terminal_junction_attempts: Vec::new(),
            terminal_junction_searches_truncated: false,
        },
    })
}

fn blocker_evidence(
    hits: BTreeMap<(RoutingBlockerKind, String), BlockerHitAccumulator>,
) -> Vec<RoutingBlockerEvidence> {
    hits.into_iter()
        .map(|((kind, object), hits)| RoutingBlockerEvidence {
            kind,
            object,
            frontier_hits: hits.count,
            frontier_centroid: hits.centroid(),
        })
        .collect()
}

fn first_invalid_path_transition(
    context: &PathValidationContext<'_>,
    path: &[GridPosition],
) -> Option<(GridPosition, GridPosition)> {
    let planar = path
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0].x != pair[1].x || pair[0].y != pair[1].y)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let first_planar = planar.first().copied()?;
    let last_planar = planar.last().copied()?;
    let margin = context.branch.width * 0.5 + context.problem.rules.clearance;
    let attached_owners = BTreeSet::from([
        context.branch.from.component.as_str(),
        context.branch.to.component.as_str(),
    ]);
    for index in planar {
        let pair = [path[index], path[index + 1]];
        if pair[0].layer != pair[1].layer {
            return Some((pair[0], pair[1]));
        }
        let first = if index == first_planar {
            context.from.position
        } else {
            context.grid.point(pair[0])
        };
        let second = if index == last_planar {
            context.to.position
        } else {
            context.grid.point(pair[1])
        };
        if !rect_contains_with_margin(context.problem.board.bounds, first, margin)
            || !rect_contains_with_margin(context.problem.board.bounds, second, margin)
        {
            return Some((pair[0], pair[1]));
        }
        let layer = &context.grid.layer_ids[pair[0].layer];
        if context.obstacles.iter().any(|obstacle| {
            obstacle.layer == *layer
                && !trace_obstacle_exempt(
                    obstacle,
                    &context.branch.electrical_net,
                    context.terminals,
                    &attached_owners,
                )
                && obstacle.shape.segment_distance(first, second) < margin - EPSILON
        }) {
            return Some((pair[0], pair[1]));
        }
        if context.allow_existing_copper_overlap {
            continue;
        }
        for trace in context.existing {
            if trace.electrical_net == context.branch.electrical_net {
                continue;
            }
            let segment_collision = trace
                .points
                .windows(2)
                .enumerate()
                .filter(|(segment_index, _)| trace.segment_layers[*segment_index] == *layer)
                .any(|(_, segment)| {
                    segment_distance(first, second, segment[0], segment[1])
                        < context.branch.width * 0.5
                            + trace.width * 0.5
                            + context.problem.rules.clearance
                            - EPSILON
                });
            let via_collision = trace.vias.iter().any(|via| {
                via_layers(via).contains(&layer.as_str())
                    && point_segment_distance(via.position, first, second)
                        < context.branch.width * 0.5
                            + via.diameter * 0.5
                            + context.problem.rules.clearance
                            - EPSILON
            });
            if segment_collision || via_collision {
                return Some((pair[0], pair[1]));
            }
        }
    }
    None
}

fn classify_frontier_blockers(
    context: &FrontierClassificationContext<'_>,
    frontier: &[GridPosition],
    hits: &mut BTreeMap<(RoutingBlockerKind, String), BlockerHitAccumulator>,
) {
    let guard = raster_guard(context.config, context.grid.grid_mm);
    let margin = context.branch.width * 0.5 + context.problem.rules.clearance;
    let attached_owners = BTreeSet::from([
        context.branch.from.component.as_str(),
        context.branch.to.component.as_str(),
    ]);
    for &position in frontier {
        let point = context.grid.point(position);
        let layer = &context.grid.layer_ids[position.layer];
        if !rect_contains_with_margin(context.problem.board.bounds, point, margin + guard) {
            record_blocker(hits, RoutingBlockerKind::BoardEdge, "board", point);
        }
        for obstacle in context.obstacles {
            if obstacle.layer != *layer
                || trace_obstacle_exempt(
                    obstacle,
                    &context.branch.electrical_net,
                    context.terminals,
                    &attached_owners,
                )
                || obstacle.shape.point_distance(point) >= margin + guard - EPSILON
            {
                continue;
            }
            let kind = match obstacle.kind {
                ObstacleKind::Body => RoutingBlockerKind::ComponentBody,
                ObstacleKind::Keepout => RoutingBlockerKind::Keepout,
                ObstacleKind::Pad => RoutingBlockerKind::Pad,
            };
            record_blocker(hits, kind, &obstacle.owner, point);
        }
        for trace in context.existing {
            if trace.electrical_net == context.branch.electrical_net {
                continue;
            }
            let segment_hit = trace
                .points
                .windows(2)
                .enumerate()
                .filter(|(segment_index, _)| trace.segment_layers[*segment_index] == *layer)
                .any(|(_, segment)| {
                    point_segment_distance(point, segment[0], segment[1])
                        < context.branch.width * 0.5
                            + trace.width * 0.5
                            + context.problem.rules.clearance
                            + guard
                            - EPSILON
                });
            let via_hit = trace.vias.iter().any(|via| {
                via_layers(via).contains(&layer.as_str())
                    && length(sub(point, via.position))
                        < context.branch.width * 0.5
                            + via.diameter * 0.5
                            + context.problem.rules.clearance
                            + guard
                            - EPSILON
            });
            if segment_hit || via_hit {
                record_blocker(hits, RoutingBlockerKind::RoutedTrace, &trace.branch, point);
            }
        }
    }
}

fn record_blocker(
    hits: &mut BTreeMap<(RoutingBlockerKind, String), BlockerHitAccumulator>,
    kind: RoutingBlockerKind,
    object: &str,
    point: Vec2,
) {
    hits.entry((kind, object.to_owned()))
        .or_default()
        .record(point);
}

fn raster_guard(config: &DutGridRoutingConfig, grid_mm: f64) -> f64 {
    match config.raster_safety {
        RasterSafety::ExactCenterline => 0.0,
        RasterSafety::ConservativeCells => grid_mm * 2.0_f64.sqrt() * 0.5,
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_request(
    problem: &Problem,
    config: &DutGridRoutingConfig,
    grid: &GridMap,
    obstacles: &[RasterObstacle],
    net_terminals: &BTreeMap<String, BTreeSet<TerminalKey>>,
    branch: &BranchRequest,
    start: GridPosition,
    finish: GridPosition,
    existing: &[SolvedTrace],
    reservation: CopperReservation<'_>,
) -> Result<GridRouteRequest, String> {
    let state_count = grid.state_count()?;
    let allowed = branch
        .allowed_layers
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let terminal_set = &net_terminals[&branch.electrical_net];
    let attached_owners =
        BTreeSet::from([branch.from.component.as_str(), branch.to.component.as_str()]);
    let guard = raster_guard(config, grid.grid_mm);
    let trace_margin = branch.width * 0.5 + problem.rules.clearance;
    let mut blocked = vec![false; state_count];
    let transition_count = state_count
        .checked_mul(8)
        .ok_or_else(|| "routing grid transition count overflow".to_string())?;
    let mut trace_enter_costs = match reservation {
        CopperReservation::Hard => vec![0; state_count],
        CopperReservation::Negotiated { history_costs, .. } => {
            if history_costs.len() != state_count {
                return Err("negotiated history volume does not match routing grid".into());
            }
            history_costs.to_vec()
        }
    };
    let mut via_enter_costs = vec![u32::MAX; state_count];
    let plane_size = grid.plane_size();

    // Establish board/layer legality once per cell. Physical objects are then
    // scattered only over their bounded grid footprint instead of every cell
    // scanning every object.
    for layer_index in 0..grid.layer_ids.len() {
        let layer = &grid.layer_ids[layer_index];
        let plane = &mut blocked[layer_index * plane_size..(layer_index + 1) * plane_size];
        if !allowed.contains(layer) {
            plane.fill(true);
            continue;
        }
        for y in 0..grid.height {
            for x in 0..grid.width {
                let point = grid.point(GridPosition {
                    x,
                    y,
                    layer: layer_index,
                });
                plane[y * grid.width + x] =
                    !rect_contains_with_margin(problem.board.bounds, point, trace_margin + guard);
            }
        }
    }

    for obstacle in obstacles {
        let Some(&layer_index) = grid.layer_indexes.get(&obstacle.layer) else {
            continue;
        };
        if !allowed.contains(&obstacle.layer)
            || trace_obstacle_exempt(
                obstacle,
                &branch.electrical_net,
                terminal_set,
                &attached_owners,
            )
        {
            continue;
        }
        mark_shape(
            &mut blocked[layer_index * plane_size..(layer_index + 1) * plane_size],
            grid,
            obstacle.shape,
            trace_margin + guard,
            true,
        );
    }
    for trace in existing {
        if trace.electrical_net == branch.electrical_net {
            continue;
        }
        let margin = branch.width * 0.5 + trace.width * 0.5 + problem.rules.clearance + guard;
        for (segment_index, segment) in trace.points.windows(2).enumerate() {
            let layer = &trace.segment_layers[segment_index];
            let Some(&layer_index) = grid.layer_indexes.get(layer) else {
                continue;
            };
            if allowed.contains(layer) {
                match reservation {
                    CopperReservation::Hard => mark_segment(
                        &mut blocked[layer_index * plane_size..(layer_index + 1) * plane_size],
                        grid,
                        segment[0],
                        segment[1],
                        margin,
                        true,
                    ),
                    CopperReservation::Negotiated {
                        congestion_cost, ..
                    } => add_segment_cost(
                        &mut trace_enter_costs
                            [layer_index * plane_size..(layer_index + 1) * plane_size],
                        grid,
                        segment[0],
                        segment[1],
                        margin,
                        congestion_cost,
                    ),
                }
            }
        }
        for via in &trace.vias {
            let margin = branch.width * 0.5 + via.diameter * 0.5 + problem.rules.clearance + guard;
            for layer in via_layers(via) {
                let Some(&layer_index) = grid.layer_indexes.get(layer) else {
                    continue;
                };
                if allowed.contains(layer) {
                    match reservation {
                        CopperReservation::Hard => mark_circle(
                            &mut blocked[layer_index * plane_size..(layer_index + 1) * plane_size],
                            grid,
                            via.position,
                            margin,
                            true,
                        ),
                        CopperReservation::Negotiated {
                            congestion_cost, ..
                        } => add_circle_cost(
                            &mut trace_enter_costs
                                [layer_index * plane_size..(layer_index + 1) * plane_size],
                            grid,
                            via.position,
                            margin,
                            congestion_cost,
                        ),
                    }
                }
            }
        }
    }

    if config.allow_vias && allowed.len() > 1 {
        let radius = problem.rules.via_diameter * 0.5;
        let mut via_legal = vec![true; plane_size];
        let mut via_soft_costs = vec![0_u32; plane_size];
        for y in 0..grid.height {
            for x in 0..grid.width {
                let point = grid.point(GridPosition { x, y, layer: 0 });
                via_legal[y * grid.width + x] = rect_contains_with_margin(
                    problem.board.bounds,
                    point,
                    radius + problem.rules.clearance,
                );
            }
        }
        for obstacle in obstacles {
            if !allowed.contains(&obstacle.layer) {
                continue;
            }
            let same_net_pad = obstacle.kind == ObstacleKind::Pad
                && obstacle.pin.as_ref().is_some_and(|pin| {
                    terminal_set.contains(&TerminalKey {
                        component: obstacle.owner.clone(),
                        pin: pin.clone(),
                    })
                });
            if problem.rules.allow_via_in_pad && same_net_pad {
                continue;
            }
            mark_shape(
                &mut via_legal,
                grid,
                obstacle.shape,
                radius + problem.rules.clearance,
                false,
            );
        }
        for trace in existing {
            if trace.electrical_net == branch.electrical_net {
                continue;
            }
            for (segment_index, segment) in trace.points.windows(2).enumerate() {
                if !allowed.contains(&trace.segment_layers[segment_index]) {
                    continue;
                }
                match reservation {
                    CopperReservation::Hard => mark_segment(
                        &mut via_legal,
                        grid,
                        segment[0],
                        segment[1],
                        radius + trace.width * 0.5 + problem.rules.clearance,
                        false,
                    ),
                    CopperReservation::Negotiated {
                        congestion_cost, ..
                    } => add_segment_cost(
                        &mut via_soft_costs,
                        grid,
                        segment[0],
                        segment[1],
                        radius + trace.width * 0.5 + problem.rules.clearance,
                        congestion_cost,
                    ),
                }
            }
            for via in &trace.vias {
                if via_layers(via).iter().any(|layer| allowed.contains(*layer)) {
                    match reservation {
                        CopperReservation::Hard => mark_circle(
                            &mut via_legal,
                            grid,
                            via.position,
                            radius + via.diameter * 0.5 + problem.rules.clearance,
                            false,
                        ),
                        CopperReservation::Negotiated {
                            congestion_cost, ..
                        } => add_circle_cost(
                            &mut via_soft_costs,
                            grid,
                            via.position,
                            radius + via.diameter * 0.5 + problem.rules.clearance,
                            congestion_cost,
                        ),
                    }
                }
            }
        }
        for layer_index in 0..grid.layer_ids.len() {
            if !allowed.contains(&grid.layer_ids[layer_index]) {
                continue;
            }
            for (cell, legal) in via_legal.iter().copied().enumerate() {
                if legal {
                    let history = match reservation {
                        CopperReservation::Hard => 0,
                        CopperReservation::Negotiated { history_costs, .. } => allowed
                            .iter()
                            .filter_map(|layer| grid.layer_indexes.get(layer))
                            .fold(0_u32, |cost, layer| {
                                cost.saturating_add(history_costs[layer * plane_size + cell])
                            }),
                    };
                    via_enter_costs[layer_index * plane_size + cell] =
                        history.saturating_add(via_soft_costs[cell]);
                }
            }
        }
    }
    blocked[grid.state_index(start)] = false;
    blocked[grid.state_index(finish)] = false;
    Ok(GridRouteRequest {
        width: grid.width,
        height: grid.height,
        layers: grid.layer_ids.len(),
        start,
        finish,
        max_expansions: config.max_expansions_per_search,
        alternate_starts: Vec::new(),
        alternate_finishes: Vec::new(),
        straight_cost: config.straight_cost,
        diagonal_cost: config.diagonal_cost,
        bend_cost: config.bend_cost,
        via_cost: config.via_cost,
        allow_vias: config.allow_vias && allowed.len() > 1,
        blocked,
        blocked_planar_transitions: vec![false; transition_count],
        trace_enter_costs,
        via_enter_costs,
    })
}

fn mark_shape(plane: &mut [bool], grid: &GridMap, shape: ExactShape, margin: f64, value: bool) {
    let (minimum, maximum) = shape.bounds(margin);
    let Some((first_x, last_x, first_y, last_y)) = grid.cell_bounds(minimum, maximum) else {
        return;
    };
    for y in first_y..=last_y {
        for x in first_x..=last_x {
            let point = grid.point(GridPosition { x, y, layer: 0 });
            if shape.point_distance(point) < margin - EPSILON {
                plane[y * grid.width + x] = value;
            }
        }
    }
}

fn mark_segment(
    plane: &mut [bool],
    grid: &GridMap,
    first: Vec2,
    second: Vec2,
    margin: f64,
    value: bool,
) {
    let minimum = Vec2::new(
        first.x.min(second.x) - margin,
        first.y.min(second.y) - margin,
    );
    let maximum = Vec2::new(
        first.x.max(second.x) + margin,
        first.y.max(second.y) + margin,
    );
    let Some((first_x, last_x, first_y, last_y)) = grid.cell_bounds(minimum, maximum) else {
        return;
    };
    for y in first_y..=last_y {
        for x in first_x..=last_x {
            let point = grid.point(GridPosition { x, y, layer: 0 });
            if point_segment_distance(point, first, second) < margin - EPSILON {
                plane[y * grid.width + x] = value;
            }
        }
    }
}

fn mark_circle(plane: &mut [bool], grid: &GridMap, center: Vec2, radius: f64, value: bool) {
    let minimum = Vec2::new(center.x - radius, center.y - radius);
    let maximum = Vec2::new(center.x + radius, center.y + radius);
    let Some((first_x, last_x, first_y, last_y)) = grid.cell_bounds(minimum, maximum) else {
        return;
    };
    for y in first_y..=last_y {
        for x in first_x..=last_x {
            let point = grid.point(GridPosition { x, y, layer: 0 });
            if length(sub(point, center)) < radius - EPSILON {
                plane[y * grid.width + x] = value;
            }
        }
    }
}

fn add_segment_cost(
    plane: &mut [u32],
    grid: &GridMap,
    first: Vec2,
    second: Vec2,
    margin: f64,
    cost: u32,
) {
    let minimum = Vec2::new(
        first.x.min(second.x) - margin,
        first.y.min(second.y) - margin,
    );
    let maximum = Vec2::new(
        first.x.max(second.x) + margin,
        first.y.max(second.y) + margin,
    );
    let Some((first_x, last_x, first_y, last_y)) = grid.cell_bounds(minimum, maximum) else {
        return;
    };
    for y in first_y..=last_y {
        for x in first_x..=last_x {
            let point = grid.point(GridPosition { x, y, layer: 0 });
            if point_segment_distance(point, first, second) < margin - EPSILON {
                let entry = &mut plane[y * grid.width + x];
                *entry = entry.saturating_add(cost);
            }
        }
    }
}

fn add_circle_cost(plane: &mut [u32], grid: &GridMap, center: Vec2, radius: f64, cost: u32) {
    let minimum = Vec2::new(center.x - radius, center.y - radius);
    let maximum = Vec2::new(center.x + radius, center.y + radius);
    let Some((first_x, last_x, first_y, last_y)) = grid.cell_bounds(minimum, maximum) else {
        return;
    };
    for y in first_y..=last_y {
        for x in first_x..=last_x {
            let point = grid.point(GridPosition { x, y, layer: 0 });
            if length(sub(point, center)) < radius - EPSILON {
                let entry = &mut plane[y * grid.width + x];
                *entry = entry.saturating_add(cost);
            }
        }
    }
}

/// Map exact copper-clearance conflicts back to the grid states which created
/// them. This avoids both missing diagonal edge conflicts and charging legal
/// neighboring cells merely because their finite cell areas touch.
fn congested_grid_states(
    problem: &Problem,
    grid: &GridMap,
    candidate: &CandidateArtifact,
) -> Result<BTreeSet<usize>, String> {
    let mut overused = BTreeSet::new();

    for first_index in 0..candidate.traces.len() {
        let first = &candidate.traces[first_index];
        if first.segment_layers.len() != first.points.len().saturating_sub(1) {
            return Err(format!(
                "trace {} segment layer count does not match its points",
                first.branch
            ));
        }
        for second in &candidate.traces[first_index + 1..] {
            if second.segment_layers.len() != second.points.len().saturating_sub(1) {
                return Err(format!(
                    "trace {} segment layer count does not match its points",
                    second.branch
                ));
            }
            if first.electrical_net == second.electrical_net {
                continue;
            }
            let required = (first.width + second.width) * 0.5 + problem.rules.clearance;
            for (first_segment_index, first_segment) in first.points.windows(2).enumerate() {
                let layer = &first.segment_layers[first_segment_index];
                for (second_segment_index, second_segment) in second.points.windows(2).enumerate() {
                    if layer != &second.segment_layers[second_segment_index]
                        || segment_distance(
                            first_segment[0],
                            first_segment[1],
                            second_segment[0],
                            second_segment[1],
                        ) + EPSILON
                            >= required
                    {
                        continue;
                    }
                    mark_segment_endpoint_states(&mut overused, grid, layer, first_segment)?;
                    mark_segment_endpoint_states(&mut overused, grid, layer, second_segment)?;
                }
            }
        }
    }

    for trace in &candidate.traces {
        for via in &trace.vias {
            for other in &candidate.traces {
                if trace.electrical_net == other.electrical_net {
                    continue;
                }
                let required = via.diameter * 0.5 + other.width * 0.5 + problem.rules.clearance;
                for (segment_index, segment) in other.points.windows(2).enumerate() {
                    let layer = &other.segment_layers[segment_index];
                    if !via_layers(via).contains(&layer.as_str())
                        || point_segment_distance(via.position, segment[0], segment[1]) + EPSILON
                            >= required
                    {
                        continue;
                    }
                    mark_grid_state(&mut overused, grid, layer, via.position)?;
                    mark_segment_endpoint_states(&mut overused, grid, layer, segment)?;
                }
            }
        }
    }

    let vias = candidate
        .traces
        .iter()
        .flat_map(|trace| trace.vias.iter().map(move |via| (trace, via)))
        .collect::<Vec<_>>();
    for first_index in 0..vias.len() {
        let (first_trace, first_via) = vias[first_index];
        for (second_trace, second_via) in &vias[first_index + 1..] {
            if first_trace.electrical_net == second_trace.electrical_net {
                continue;
            }
            let required =
                (first_via.diameter + second_via.diameter) * 0.5 + problem.rules.clearance;
            if length(sub(first_via.position, second_via.position)) + EPSILON >= required {
                continue;
            }
            for layer in via_layers(first_via) {
                if via_layers(second_via).contains(&layer) {
                    mark_grid_state(&mut overused, grid, layer, first_via.position)?;
                    mark_grid_state(&mut overused, grid, layer, second_via.position)?;
                }
            }
        }
    }
    Ok(overused)
}

fn mark_segment_endpoint_states(
    states: &mut BTreeSet<usize>,
    grid: &GridMap,
    layer: &str,
    segment: &[Vec2],
) -> Result<(), String> {
    for point in segment {
        mark_grid_state(states, grid, layer, *point)?;
    }
    Ok(())
}

fn mark_grid_state(
    states: &mut BTreeSet<usize>,
    grid: &GridMap,
    layer: &str,
    point: Vec2,
) -> Result<(), String> {
    states.insert(grid.state_index(grid.snap(point, layer)?));
    Ok(())
}

fn trace_obstacle_exempt(
    obstacle: &RasterObstacle,
    electrical_net: &str,
    terminals: &BTreeSet<TerminalKey>,
    attached_owners: &BTreeSet<&str>,
) -> bool {
    if obstacle.kind == ObstacleKind::Body && attached_owners.contains(obstacle.owner.as_str()) {
        return true;
    }
    obstacle.kind == ObstacleKind::Pad
        && obstacle.pin.as_ref().is_some_and(|pin| {
            terminals.contains(&TerminalKey {
                component: obstacle.owner.clone(),
                pin: pin.clone(),
            })
        })
        && !electrical_net.is_empty()
}

fn via_layers(via: &SolvedVia) -> [&str; 2] {
    [&via.from_layer, &via.to_layer]
}

fn solved_trace(
    problem: &Problem,
    grid: &GridMap,
    branch: &BranchRequest,
    from: &TerminalAttachment,
    to: &TerminalAttachment,
    path: &[GridPosition],
) -> Result<SolvedTrace, String> {
    if path.is_empty() {
        return Err("grid router returned an empty path".into());
    }
    let mut points = vec![grid.point(path[0])];
    let mut segment_layers = Vec::new();
    let mut vias = Vec::new();
    let mut current_layer = path[0].layer;
    for pair in path.windows(2) {
        if pair[0].x == pair[1].x && pair[0].y == pair[1].y && pair[0].layer != pair[1].layer {
            vias.push(SolvedVia {
                position: *points.last().unwrap(),
                from_layer: grid.layer_ids[current_layer].clone(),
                to_layer: grid.layer_ids[pair[1].layer].clone(),
                diameter: problem.rules.via_diameter,
                drill: problem.rules.via_drill,
                point_index: points.len() - 1,
            });
            current_layer = pair[1].layer;
        } else {
            if pair[0].layer != pair[1].layer {
                return Err("grid router changed coordinate and layer in one step".into());
            }
            points.push(grid.point(pair[1]));
            segment_layers.push(grid.layer_ids[current_layer].clone());
        }
    }
    if points.len() == 1 {
        points.push(to.position);
        segment_layers.push(grid.layer_ids[current_layer].clone());
    }
    points[0] = from.position;
    let last = points.len() - 1;
    points[last] = to.position;
    for via in &mut vias {
        via.position = points[via.point_index];
    }
    let route_layer = segment_layers
        .first()
        .cloned()
        .unwrap_or_else(|| branch.preferred_layer.clone());
    Ok(SolvedTrace {
        branch: branch.id.clone(),
        electrical_net: branch.electrical_net.clone(),
        net: branch.id.clone(),
        from_node: terminal_node_id(&branch.electrical_net, &branch.from),
        to_node: terminal_node_id(&branch.electrical_net, &branch.to),
        width: branch.width,
        tension_weight: branch.tension_weight,
        layer: route_layer.clone(),
        segment_layers,
        vias,
        points,
        route_class: RouteClass::direct(
            route_layer,
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
    })
}

fn terminal_node_id(net: &str, terminal: &TerminalKey) -> String {
    format!("terminal:{net}/{}.{}", terminal.component, terminal.pin)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(source: &str) -> Problem {
        let problem: Problem = serde_json::from_str(source).unwrap();
        problem.check_schema().unwrap();
        problem
    }

    #[test]
    fn routes_a_single_branch_into_an_exact_candidate() {
        let problem = load(
            r#"{
                "schema_version":1,
                "board":{"bounds":{"min":{"x":0.0,"y":0.0},"max":{"x":20.0,"y":12.0}},"layers":[{"id":"top"}]},
                "rules":{"clearance":0.2},
                "components":[
                    {"id":"A","position":{"x":2.0,"y":6.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.8}}]}]},
                    {"id":"B","position":{"x":18.0,"y":6.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.8}}]}]}
                ],
                "nets":[{"id":"N","width":0.3,"from":{"component":"A","pin":"1"},"to":{"component":"B","pin":"1"}}]
            }"#,
        );
        let result =
            route_problem_with_dut_grid(&problem, &DutGridRoutingConfig::default()).unwrap();
        assert!(result.complete(), "{:?}", result.validation.violations);
        assert_eq!(result.candidate.traces.len(), 1);
        assert_eq!(result.evidence.routed_branches, 1);
    }

    #[test]
    fn exact_edge_retry_routes_around_a_subgrid_pad_corner() {
        let problem = load(
            r#"{
                "schema_version":1,
                "board":{"bounds":{"min":{"x":0.0,"y":0.0},"max":{"x":6.0,"y":6.0}},"layers":[{"id":"top"}]},
                "rules":{"clearance":0.1},
                "components":[
                    {"id":"A","position":{"x":1.0,"y":1.0},"size":{"x":0.2,"y":0.2},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.2}}]}]},
                    {"id":"B","position":{"x":5.0,"y":5.0},"size":{"x":0.2,"y":0.2},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.2}}]}]},
                    {"id":"X","position":{"x":1.5,"y":1.5},"size":{"x":0.2,"y":0.2},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.2}}]}]}
                ],
                "nets":[{"id":"N","width":0.2,"from":{"component":"A","pin":"1"},"to":{"component":"B","pin":"1"}}]
            }"#,
        );
        let result = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                grid_mm: 1.0,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();
        assert!(result.complete(), "{:?}", result.validation.violations);
        assert!(result.evidence.branches[0].exact_edge_retries > 0);
    }

    #[test]
    fn routes_the_imported_multi_terminal_tree() {
        let problem = load(include_str!(
            "../../../benchmarks/imported/layout-trace/multi-terminal-tree.json"
        ));
        let result =
            route_problem_with_dut_grid(&problem, &DutGridRoutingConfig::default()).unwrap();
        assert!(result.complete(), "{:?}", result.validation.violations);
        assert_eq!(result.candidate.traces.len(), 2);
        assert_eq!(
            result.candidate.route_graphs[0]
                .nodes
                .iter()
                .filter(|node| matches!(node.kind, SolvedRouteNodeKind::Terminal { .. }))
                .count(),
            3
        );
    }

    #[test]
    fn shared_copper_tree_grows_from_an_explicit_interior_junction() {
        let problem = load(include_str!(
            "../../../benchmarks/imported/layout-trace/multi-terminal-tree.json"
        ));
        let baseline =
            route_problem_with_dut_grid(&problem, &DutGridRoutingConfig::default()).unwrap();
        let shared = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                multi_terminal_routing: MultiTerminalRoutingPolicy::SharedCopperTree,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();

        assert!(shared.complete(), "{:?}", shared.validation.violations);
        assert_eq!(shared.evidence.branch_count, 2);
        assert_eq!(shared.candidate.traces.len(), 3);
        let attachment = shared.evidence.branches[1]
            .tree_attachment
            .as_ref()
            .expect("second terminal grew from existing copper");
        assert!(attachment.interior);
        let junction = shared.candidate.route_graphs[0]
            .nodes
            .iter()
            .find(|node| node.id == attachment.node)
            .expect("tree attachment has a durable junction node");
        assert_eq!(junction.incident_branches.len(), 3);
        let length = |result: &DutGridRoutingResult| {
            result
                .candidate
                .traces
                .iter()
                .flat_map(|trace| trace.points.windows(2))
                .map(|segment| length(sub(segment[1], segment[0])))
                .sum::<f64>()
        };
        assert!(length(&shared) < length(&baseline));
        // This first policy improves copper length but currently spends more
        // search work (1,184 versus 874 expansions on this fixture). Keep both
        // metrics visible rather than turning either one into the sole verdict.
        assert!(shared.evidence.expansions > 0);
    }

    #[test]
    fn shared_copper_tree_continues_after_an_interior_split() {
        let problem = load(
            r#"{
                "schema_version":1,
                "board":{"bounds":{"min":{"x":0.0,"y":0.0},"max":{"x":40.0,"y":30.0}},"layers":[{"id":"top"}]},
                "rules":{"clearance":0.2},
                "components":[
                    {"id":"A","position":{"x":5.0,"y":5.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.8}}]}]},
                    {"id":"B","position":{"x":5.0,"y":25.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.8}}]}]},
                    {"id":"C","position":{"x":25.0,"y":15.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.8}}]}]},
                    {"id":"D","position":{"x":35.0,"y":15.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.8}}]}]}
                ],
                "electrical_nets":[{"id":"BUS","width":0.3,"terminals":[{"component":"A","pin":"1"},{"component":"B","pin":"1"},{"component":"C","pin":"1"},{"component":"D","pin":"1"}]}]
            }"#,
        );
        let result = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                multi_terminal_routing: MultiTerminalRoutingPolicy::SharedCopperTree,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();

        assert!(result.complete(), "{:?}", result.validation.violations);
        assert_eq!(result.evidence.branch_count, 3);
        assert_eq!(result.evidence.routed_branches, 3);
        assert_eq!(result.candidate.traces.len(), 4);
        assert_eq!(
            result
                .evidence
                .branches
                .iter()
                .filter(|branch| branch.tree_attachment.is_some())
                .count(),
            2
        );
        let graph = &result.candidate.route_graphs[0];
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| matches!(node.kind, SolvedRouteNodeKind::Terminal { .. }))
                .count(),
            4
        );
        assert!(graph.nodes.iter().any(|node| {
            matches!(node.kind, SolvedRouteNodeKind::Junction { .. })
                && node.incident_branches.len() == 3
        }));
    }

    #[test]
    fn shared_tree_trims_copper_duplicated_before_the_last_tree_contact() {
        let problem = load(include_str!(
            "../../../benchmarks/small/shared-tree-prefix-trim.json"
        ));
        let result = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                multi_terminal_routing: MultiTerminalRoutingPolicy::SharedCopperTree,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();

        assert!(result.complete(), "{:?}", result.validation.violations);
        let growth = result
            .evidence
            .branches
            .iter()
            .find(|branch| branch.branch == "BUS#branch1")
            .unwrap();
        let attachment = growth.tree_attachment.as_ref().unwrap();
        assert_eq!(attachment.trimmed_prefix_points, 15);
        assert_eq!(attachment.searched_point_index, 20);
        assert_eq!(attachment.searched_position, Vec2::new(5.0, 15.0));
        assert_eq!(attachment.position, Vec2::new(5.0, 7.5));
        assert_eq!(growth.point_count, 47);
        let grown = result
            .candidate
            .traces
            .iter()
            .find(|trace| trace.branch == "BUS#branch1")
            .unwrap();
        assert_eq!(grown.points[0], attachment.position);
        assert!(
            result
                .candidate
                .traces
                .iter()
                .filter(|trace| trace.branch != grown.branch)
                .all(|trace| !trace.points.iter().any(|point| *point == grown.points[1]))
        );
        let junction = result.candidate.route_graphs[0]
            .nodes
            .iter()
            .find(|node| node.id == attachment.node)
            .unwrap();
        assert_eq!(junction.incident_branches.len(), 3);
    }

    #[test]
    fn shared_tree_materializes_a_half_grid_segment_crossing() {
        let problem = load(
            r#"{
                "schema_version":1,
                "board":{"bounds":{"min":{"x":0.0,"y":0.0},"max":{"x":4.0,"y":3.0}},"layers":[{"id":"top"}]},
                "rules":{"clearance":0.1},
                "components":[
                    {"id":"A","position":{"x":1.0,"y":1.0},"size":{"x":0.2,"y":0.2},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.2}}]}]},
                    {"id":"B","position":{"x":2.0,"y":2.0},"size":{"x":0.2,"y":0.2},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.2}}]}]},
                    {"id":"C","position":{"x":3.0,"y":1.0},"size":{"x":0.2,"y":0.2},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.2}}]}]}
                ],
                "electrical_nets":[{"id":"BUS","width":0.1,"terminals":[{"component":"A","pin":"1"},{"component":"B","pin":"1"},{"component":"C","pin":"1"}]}]
            }"#,
        );
        let terminal = |component: &str| TerminalKey {
            component: component.into(),
            pin: "1".into(),
        };
        let attachments = [
            (terminal("A"), Vec2::new(1.0, 1.0)),
            (terminal("B"), Vec2::new(2.0, 2.0)),
            (terminal("C"), Vec2::new(3.0, 1.0)),
        ]
        .into_iter()
        .map(|(terminal, position)| {
            (
                ("BUS".to_string(), terminal),
                TerminalAttachment {
                    position,
                    layers: vec!["top".into()],
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
        let route_class = |from: &str, to: &str| {
            RouteClass::direct(
                "top",
                TerminalSector {
                    component: from.into(),
                    sector: "1".into(),
                },
                TerminalSector {
                    component: to.into(),
                    sector: "1".into(),
                },
            )
        };
        let mut traces = vec![SolvedTrace {
            branch: "BUS#branch0".into(),
            electrical_net: "BUS".into(),
            net: "BUS#branch0".into(),
            from_node: terminal_node_id("BUS", &terminal("A")),
            to_node: terminal_node_id("BUS", &terminal("B")),
            width: 0.1,
            tension_weight: 1.0,
            layer: "top".into(),
            segment_layers: vec!["top".into()],
            vias: Vec::new(),
            points: vec![Vec2::new(1.0, 1.0), Vec2::new(2.0, 2.0)],
            route_class: route_class("A", "B"),
            route_basis_fingerprint: None,
        }];
        let branch = BranchRequest {
            id: "BUS#branch1".into(),
            electrical_net: "BUS".into(),
            width: 0.1,
            tension_weight: 1.0,
            preferred_layer: "top".into(),
            allowed_layers: vec!["top".into()],
            from: terminal("A"),
            to: terminal("C"),
            input_order: 1,
            tree_growth_index: Some(1),
        };
        let searched = tree_vertex_selection(&branch, &traces, &attachments, 0, 0).unwrap();
        let mut growing = SolvedTrace {
            branch: branch.id.clone(),
            electrical_net: "BUS".into(),
            net: branch.id.clone(),
            from_node: terminal_node_id("BUS", &terminal("A")),
            to_node: terminal_node_id("BUS", &terminal("C")),
            width: 0.1,
            tension_weight: 1.0,
            layer: "top".into(),
            segment_layers: vec!["top".into(), "top".into(), "top".into()],
            vias: Vec::new(),
            points: vec![
                Vec2::new(1.0, 1.0),
                Vec2::new(1.0, 2.0),
                Vec2::new(2.0, 1.0),
                Vec2::new(3.0, 1.0),
            ],
            route_class: route_class("A", "C"),
            route_basis_fingerprint: None,
        };

        let trimmed = trim_tree_prefix_to_last_contact(
            &mut growing,
            &branch,
            &traces,
            &attachments,
            &searched,
        )
        .unwrap();
        assert_eq!(trimmed.selection.position, Vec2::new(1.5, 1.5));
        assert_eq!(trimmed.selection.existing_segment_index, Some(0));
        assert_eq!(trimmed.trimmed_prefix_points, 2);
        assert!(trimmed.new_segment_split);
        assert_eq!(growing.points[0], Vec2::new(1.5, 1.5));

        let graph_terminals = attachments
            .iter()
            .map(|((_, terminal), attachment)| (terminal.clone(), attachment.clone()))
            .collect();
        let mut graph = GraphBuilder::new(graph_terminals);
        commit_tree_growth(
            &mut traces,
            &mut graph,
            &mut growing,
            &branch,
            &terminal("C"),
            &trimmed.selection,
        )
        .unwrap();
        let route_graph = graph.finish("BUS".into(), &traces).unwrap();
        let junction = route_graph
            .nodes
            .iter()
            .find(|node| node.id == trimmed.selection.node)
            .unwrap();
        assert_eq!(junction.incident_branches.len(), 3);
        assert_eq!(traces.len(), 3);
        assert!(
            traces
                .iter()
                .all(|trace| trace.points[0] == junction.position
                    || *trace.points.last().unwrap() == junction.position)
        );

        let candidate = CandidateArtifact {
            schema_version: CANDIDATE_SCHEMA_VERSION,
            components: problem
                .components
                .iter()
                .map(|component| SolvedComponent {
                    id: component.id.clone(),
                    position: component.position,
                    size: component.size,
                    rotation_degrees: component.rotation_degrees,
                })
                .collect(),
            traces,
            route_graphs: vec![route_graph],
        };
        let validation = validate_candidate(&problem, &candidate).unwrap();
        assert!(validation.complete, "{:?}", validation.violations);
    }

    #[test]
    fn shared_tree_selects_the_next_terminal_from_current_tree_geometry() {
        let problem = load(include_str!(
            "../../../benchmarks/small/shared-tree-dynamic-terminal.json"
        ));
        let result = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                multi_terminal_routing: MultiTerminalRoutingPolicy::SharedCopperTree,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();

        assert!(result.complete(), "{:?}", result.validation.violations);
        let targets = result
            .evidence
            .branches
            .iter()
            .filter_map(|branch| {
                branch
                    .tree_attachment
                    .as_ref()
                    .map(|attachment| attachment.target_terminal.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(targets, ["D.1", "C.1"]);
        let first_growth = result.evidence.branches[1]
            .tree_attachment
            .as_ref()
            .unwrap();
        assert_eq!(first_growth.position, Vec2::new(34.0, 5.0));
        assert!(first_growth.interior);
        assert_eq!(result.evidence.routed_branches, 3);
        assert!(result.validation.electrical.complete);

        let longest = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                route_order: RouteOrder::LongestFirst,
                multi_terminal_routing: MultiTerminalRoutingPolicy::SharedCopperTree,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();
        assert_eq!(longest.candidate, result.candidate);
    }

    #[test]
    fn shared_tree_does_not_route_dependent_growth_after_a_failed_trunk() {
        let problem = load(
            r#"{
                "schema_version":1,
                "board":{"bounds":{"min":{"x":0.0,"y":0.0},"max":{"x":20.0,"y":20.0}},"layers":[{"id":"top"}]},
                "rules":{"clearance":0.2},
                "components":[
                    {"id":"A","position":{"x":1.0,"y":5.0},"size":{"x":0.5,"y":0.5},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.5}}]}]},
                    {"id":"B","position":{"x":9.0,"y":5.0},"size":{"x":0.5,"y":0.5},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.5}}]}]},
                    {"id":"C","position":{"x":1.0,"y":15.0},"size":{"x":0.5,"y":0.5},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.5}}]}]},
                    {"id":"D","position":{"x":1.0,"y":18.0},"size":{"x":0.5,"y":0.5},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.5}}]}]},
                    {"id":"WALL","position":{"x":5.0,"y":10.0},"size":{"x":1.0,"y":20.0},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":true,"pins":[]}
                ],
                "electrical_nets":[{"id":"BUS","width":0.3,"terminals":[{"component":"A","pin":"1"},{"component":"B","pin":"1"},{"component":"C","pin":"1"},{"component":"D","pin":"1"}]}]
            }"#,
        );
        let result = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                multi_terminal_routing: MultiTerminalRoutingPolicy::SharedCopperTree,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();

        assert!(!result.complete());
        assert_eq!(result.evidence.routed_branches, 0);
        assert_eq!(
            result.evidence.branches[0].status,
            BranchRoutingStatus::NoPath
        );
        assert_eq!(
            result.evidence.branches[1].status,
            BranchRoutingStatus::TreeUnavailable
        );
        assert_eq!(
            result.evidence.branches[2].status,
            BranchRoutingStatus::TreeUnavailable
        );
        assert_eq!(result.evidence.branches[1].expansions, 0);
        assert_eq!(result.evidence.branches[2].expansions, 0);
    }

    #[test]
    fn terminal_junction_preference_trades_one_search_for_a_simpler_shorter_tree() {
        let problem = load(include_str!(
            "../../../benchmarks/small/shared-tree-terminal-junction.json"
        ));
        let baseline = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                multi_terminal_routing: MultiTerminalRoutingPolicy::SharedCopperTree,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();
        let preferred = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                multi_terminal_routing: MultiTerminalRoutingPolicy::SharedCopperTree,
                terminal_junction_slack_mm: Some(1.0),
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();

        assert!(baseline.complete());
        assert!(preferred.complete());
        assert_eq!(baseline.candidate.traces.len(), 3);
        assert_eq!(preferred.candidate.traces.len(), 2);
        assert_eq!(
            baseline.candidate.route_graphs[0]
                .nodes
                .iter()
                .filter(|node| matches!(node.kind, SolvedRouteNodeKind::Junction { .. }))
                .count(),
            1
        );
        assert!(
            !preferred.candidate.route_graphs[0]
                .nodes
                .iter()
                .any(|node| matches!(node.kind, SolvedRouteNodeKind::Junction { .. }))
        );
        let attachment = preferred.evidence.branches[1]
            .tree_attachment
            .as_ref()
            .unwrap();
        assert!(attachment.terminal_preference_selected);
        assert_eq!(attachment.node, "terminal:TREE/B.1");
        assert_eq!(
            preferred.evidence.branches[1]
                .terminal_junction_attempts
                .len(),
            1
        );
        assert!(preferred.evidence.branches[1].terminal_junction_attempts[0].selected);
        let total_length = |result: &DutGridRoutingResult| {
            result
                .candidate
                .traces
                .iter()
                .map(solved_trace_length)
                .sum::<f64>()
        };
        assert!(total_length(&preferred) < total_length(&baseline));
        assert!(preferred.evidence.expansions > baseline.evidence.expansions);
    }

    #[test]
    fn terminal_junction_preference_requires_shared_tree_routing() {
        let config = DutGridRoutingConfig {
            terminal_junction_slack_mm: Some(0.0),
            ..DutGridRoutingConfig::default()
        };
        assert!(config.check().is_err());
    }

    #[test]
    fn terminal_junction_alternative_searches_obey_the_explicit_bound() {
        let problem = load(include_str!(
            "../../../benchmarks/small/shared-tree-dynamic-terminal.json"
        ));
        let result = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                multi_terminal_routing: MultiTerminalRoutingPolicy::SharedCopperTree,
                terminal_junction_slack_mm: Some(100.0),
                maximum_terminal_junction_searches: 1,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();

        assert!(result.complete(), "{:?}", result.validation.violations);
        assert!(
            result
                .evidence
                .branches
                .iter()
                .any(|branch| branch.terminal_junction_searches_truncated)
        );
        assert!(
            result
                .evidence
                .branches
                .iter()
                .all(|branch| { branch.terminal_junction_attempts.len() <= 1 })
        );
    }

    #[test]
    fn small_board_ladder_is_exact_and_bounded() {
        struct Rung {
            name: &'static str,
            source: &'static str,
            branches: usize,
            maximum_expansions: u64,
            minimum_vias: usize,
        }

        let rungs = [
            Rung {
                name: "realistic pad-bank escape",
                source: include_str!(
                    "../../../benchmarks/imported/layout-trace/esp32-slices/esp-pad-multi-contact-escape.json"
                ),
                branches: 1,
                maximum_expansions: 200,
                minimum_vias: 0,
            },
            Rung {
                name: "explicit three-terminal tree",
                source: include_str!(
                    "../../../benchmarks/imported/layout-trace/multi-terminal-tree.json"
                ),
                branches: 2,
                maximum_expansions: 2_000,
                minimum_vias: 0,
            },
            Rung {
                name: "resistor turn escape",
                source: include_str!(
                    "../../../benchmarks/imported/layout-trace/esp32-slices/resistor-turn-escape.json"
                ),
                branches: 2,
                maximum_expansions: 1_000,
                minimum_vias: 0,
            },
            Rung {
                name: "ESP32 pad fanout clearance",
                source: include_str!(
                    "../../../benchmarks/imported/layout-trace/esp32-slices/esp-pad-fanout-clearance.json"
                ),
                branches: 2,
                maximum_expansions: 10_000,
                minimum_vias: 0,
            },
            Rung {
                name: "forced two-layer crossing",
                source: include_str!(
                    "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
                ),
                branches: 2,
                maximum_expansions: 30_000,
                minimum_vias: 1,
            },
            Rung {
                name: "power and capacitor interaction",
                source: include_str!(
                    "../../../benchmarks/imported/layout-trace/esp32-slices/power-west-capacitor-interaction.json"
                ),
                branches: 4,
                maximum_expansions: 5_000,
                minimum_vias: 0,
            },
            Rung {
                name: "six-branch west-side geometry",
                source: include_str!(
                    "../../../benchmarks/imported/layout-trace/esp32-slices/west-six-final-geometry.json"
                ),
                branches: 6,
                maximum_expansions: 30_000,
                minimum_vias: 0,
            },
            Rung {
                name: "eight-branch south header breakout",
                source: include_str!(
                    "../../../benchmarks/imported/layout-trace/esp32-slices/south-header-breakout.json"
                ),
                branches: 8,
                maximum_expansions: 40_000,
                minimum_vias: 0,
            },
            Rung {
                name: "ten-branch resistor link bundle",
                source: include_str!(
                    "../../../benchmarks/imported/layout-trace/esp32-slices/resistor-link-bundle.json"
                ),
                branches: 10,
                maximum_expansions: 40_000,
                minimum_vias: 0,
            },
        ];

        for rung in rungs {
            let result =
                route_problem_with_dut_grid(&load(rung.source), &DutGridRoutingConfig::default())
                    .unwrap();
            assert!(
                result.complete(),
                "{} failed: {:?}",
                rung.name,
                result.validation.violations
            );
            assert_eq!(
                result.evidence.routed_branches, rung.branches,
                "{}",
                rung.name
            );
            assert!(
                result.evidence.expansions <= rung.maximum_expansions,
                "{} used {} expansions, budget is {}",
                rung.name,
                result.evidence.expansions,
                rung.maximum_expansions
            );
            assert!(
                result
                    .candidate
                    .traces
                    .iter()
                    .map(|trace| trace.vias.len())
                    .sum::<usize>()
                    >= rung.minimum_vias,
                "{} did not exercise its required layer transition",
                rung.name
            );
        }
    }

    #[test]
    fn routes_the_two_diagonal_fixture_without_crossing() {
        let problem = load(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ));
        let result =
            route_problem_with_dut_grid(&problem, &DutGridRoutingConfig::default()).unwrap();
        assert!(result.complete(), "{:?}", result.validation.violations);
        assert_eq!(result.evidence.routed_branches, 2);
    }

    #[test]
    fn negotiated_congestion_routes_the_two_diagonal_fixture_exactly() {
        let problem = load(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ));
        let result = route_problem_with_negotiated_dut_grid(
            &problem,
            &NegotiatedDutGridRoutingConfig::default(),
        )
        .unwrap();
        assert!(result.complete(), "{:?}", result.validation.violations);
        assert_eq!(result.evidence.routing.routed_branches, 2);
        assert_eq!(
            result.evidence.passes[result.evidence.selected_pass].congested_cells,
            0
        );
    }

    #[test]
    fn negotiated_history_reprices_an_exact_crossing() {
        let problem = load(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ));
        let result = route_problem_with_negotiated_dut_grid(
            &problem,
            &NegotiatedDutGridRoutingConfig {
                routing: DutGridRoutingConfig {
                    via_cost: 100,
                    ..DutGridRoutingConfig::default()
                },
                maximum_passes: 16,
                congestion_cost: 1,
                history_cost: 100_000,
            },
        )
        .unwrap();
        assert!(result.evidence.completed_passes > 1);
        assert!(result.evidence.passes[0].exact_violations > 0);
        assert!(
            result.complete(),
            "passes={:?}, violations={:?}",
            result.evidence.passes,
            result.validation.violations
        );
    }

    #[test]
    fn bounded_tree_attachment_portfolio_selects_a_shorter_interior_branch() {
        let problem = load(include_str!(
            "../../../benchmarks/small/shared-tree-attachment-portfolio.json"
        ));
        let baseline = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                multi_terminal_routing: MultiTerminalRoutingPolicy::SharedCopperTree,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();
        let portfolio = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                multi_terminal_routing: MultiTerminalRoutingPolicy::SharedCopperTree,
                maximum_tree_attachment_searches: 2,
                tree_attachment_minimum_spacing_mm: 2.0,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();

        assert!(baseline.complete(), "{:?}", baseline.validation.violations);
        assert!(
            portfolio.complete(),
            "{:?}",
            portfolio.validation.violations
        );
        let total_length = |result: &DutGridRoutingResult| {
            result
                .candidate
                .traces
                .iter()
                .map(solved_trace_length)
                .sum::<f64>()
        };
        assert!((total_length(&baseline) - 19.242_640_687_119_284).abs() < EPSILON);
        assert!((total_length(&portfolio) - 17.828_427_124_746_19).abs() < EPSILON);
        assert!(total_length(&portfolio) + EPSILON < total_length(&baseline));
        assert_eq!(baseline.evidence.expansions, 368);
        assert_eq!(portfolio.evidence.expansions, 500);
        let growth = &portfolio.evidence.branches[1];
        assert_eq!(growth.tree_attachment_attempts.len(), 2);
        assert!(growth.tree_attachment_searches_truncated);
        assert!(!growth.tree_attachment_attempts[0].selected);
        assert!(growth.tree_attachment_attempts[1].selected);
        assert_eq!(
            growth.tree_attachment_attempts[1].position,
            Vec2::new(5.0, 8.0)
        );
        let attachment = growth.tree_attachment.as_ref().unwrap();
        assert!(attachment.interior);
        assert_eq!(attachment.position, Vec2::new(5.0, 8.0));
    }

    #[test]
    fn tree_attachment_portfolio_requires_shared_tree_routing_and_a_positive_bound() {
        let disabled = DutGridRoutingConfig {
            maximum_tree_attachment_searches: 0,
            ..DutGridRoutingConfig::default()
        };
        assert!(disabled.check().is_err());
        let wrong_policy = DutGridRoutingConfig {
            maximum_tree_attachment_searches: 2,
            ..DutGridRoutingConfig::default()
        };
        assert!(wrong_policy.check().is_err());
    }

    #[test]
    fn negotiated_overlap_is_provisional_not_complete() {
        let problem = load(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ));
        let result = route_problem_with_negotiated_dut_grid(
            &problem,
            &NegotiatedDutGridRoutingConfig {
                maximum_passes: 1,
                congestion_cost: 1,
                ..NegotiatedDutGridRoutingConfig::default()
            },
        )
        .unwrap();
        assert_eq!(result.evidence.routing.routed_branches, 2);
        assert!(!result.validation.complete);
        assert!(!result.complete());
        assert!(result.evidence.passes[0].congested_cells > 0);
    }

    #[test]
    fn selective_reroute_keeps_unselected_copper_fixed() {
        let problem = load(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ));
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
        let baseline = route_problem_with_dut_grid_at_poses(
            &problem,
            &components,
            &DutGridRoutingConfig::default(),
        )
        .unwrap();
        let reroute = BTreeSet::from(["DIAGONAL_SE".to_string()]);
        let result = reroute_problem_branches_with_dut_grid_at_poses(
            &problem,
            &components,
            &baseline,
            &reroute,
            &DutGridRoutingConfig {
                priority_branches: vec!["DIAGONAL_SE".into()],
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();
        assert!(result.complete(), "{:?}", result.validation.violations);
        assert_eq!(result.candidate.traces.len(), 2);
        assert_eq!(
            result
                .evidence
                .branches
                .iter()
                .find(|branch| branch.branch == "DIAGONAL_NE")
                .unwrap()
                .expansions,
            0
        );
    }

    #[test]
    fn selective_reroute_may_replace_an_invalid_selected_branch() {
        let problem = load(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ));
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
        let mut baseline = route_problem_with_dut_grid_at_poses(
            &problem,
            &components,
            &DutGridRoutingConfig::default(),
        )
        .unwrap();
        let damaged = baseline
            .candidate
            .traces
            .iter_mut()
            .find(|trace| trace.branch == "DIAGONAL_SE")
            .unwrap();
        damaged.points[1] = Vec2::new(
            problem.board.bounds.min.x - 10.0,
            problem.board.bounds.min.y - 10.0,
        );
        baseline.validation = validate_candidate(&problem, &baseline.candidate).unwrap();
        assert!(!baseline.validation.geometry.complete);

        let reroute = BTreeSet::from(["DIAGONAL_SE".to_string()]);
        let result = reroute_problem_branches_with_dut_grid_at_poses(
            &problem,
            &components,
            &baseline,
            &reroute,
            &DutGridRoutingConfig {
                priority_branches: vec!["DIAGONAL_SE".into()],
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();

        assert!(result.complete(), "{:?}", result.validation.violations);
        assert_eq!(
            result
                .evidence
                .branches
                .iter()
                .find(|branch| branch.branch == "DIAGONAL_NE")
                .unwrap()
                .expansions,
            0
        );
    }

    #[test]
    fn selective_reroute_still_rejects_invalid_retained_copper() {
        let problem = load(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ));
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
        let mut baseline = route_problem_with_dut_grid_at_poses(
            &problem,
            &components,
            &DutGridRoutingConfig::default(),
        )
        .unwrap();
        let retained = baseline
            .candidate
            .traces
            .iter_mut()
            .find(|trace| trace.branch == "DIAGONAL_NE")
            .unwrap();
        retained.points[1] = Vec2::new(
            problem.board.bounds.min.x - 10.0,
            problem.board.bounds.min.y - 10.0,
        );
        let reroute = BTreeSet::from(["DIAGONAL_SE".to_string()]);

        let error = reroute_problem_branches_with_dut_grid_at_poses(
            &problem,
            &components,
            &baseline,
            &reroute,
            &DutGridRoutingConfig::default(),
        )
        .unwrap_err();
        assert!(error.contains("retained copper has invalid fixed geometry"));
    }

    #[test]
    fn preserves_budget_exhaustion_as_evidence() {
        let problem = load(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ));
        let result = route_problem_with_dut_grid(
            &problem,
            &DutGridRoutingConfig {
                max_expansions_per_search: 1,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();
        assert!(!result.complete());
        assert!(
            result
                .evidence
                .branches
                .iter()
                .any(|branch| branch.status == BranchRoutingStatus::BudgetExhausted)
        );
        assert!(!result.validation.complete);
    }

    #[test]
    fn refines_a_grid_alias_without_hiding_the_coarse_failure() {
        let problem = load(include_str!(
            "../../../benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json"
        ));
        let mut components = problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        components
            .iter_mut()
            .find(|component| component.id == "WALL")
            .unwrap()
            .position
            .y = 15.6;
        let result = route_problem_with_dut_grid_at_poses(
            &problem,
            &components,
            &DutGridRoutingConfig {
                retry_grid_mm: vec![0.25, 0.1],
                max_expansions_per_search: 5_000_000,
                ..DutGridRoutingConfig::default()
            },
        )
        .unwrap();
        assert!(result.complete(), "{:?}", result.validation.violations);
        let branch = &result.evidence.branches[0];
        assert_eq!(branch.selected_grid_mm, Some(0.1));
        assert_eq!(
            branch
                .grid_attempts
                .iter()
                .map(|attempt| attempt.status)
                .collect::<Vec<_>>(),
            vec![
                BranchRoutingStatus::NoPath,
                BranchRoutingStatus::NoPath,
                BranchRoutingStatus::Found
            ]
        );
        assert!(
            branch
                .blockers
                .iter()
                .any(|blocker| blocker.object == "WALL")
        );
    }
}

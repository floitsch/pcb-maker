//! Exhaustive physical validation over the durable candidate representation.
//!
//! This intentionally favors a small, obviously independent correctness oracle
//! over the old engine's cached runtime broad phase. Faster validators may be
//! compared against it, but may not replace it as the final gate until parity is
//! demonstrated.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{
    Problem, Rect, Vec2,
    geometry::{
        EPSILON, Obb, add, length, obbs_overlap, point_obb_signed_distance, point_segment_distance,
        rect_contains_with_margin, rotate_degrees, segment_distance, sub,
    },
    model::{
        BoardEdge, CopperShape, Movement, PlacementAnchor, PlacementConstraint, Rotation,
        angular_distance_degrees,
    },
};
use serde::Serialize;

use crate::{
    SolvedComponent, SolvedRouteGraph, SolvedRouteNodeKind, SolvedTrace, SolvedVia, Violation,
};

const POSE_TOLERANCE: f64 = 1.0e-8;
// The placement projector certifies relational constraints to 1e-6 mm. Keep
// this boundary identical so a projector-approved pose cannot fail solely on
// sub-nanometre residual noise. Copper/clearance comparisons retain their own
// stricter geometry epsilon.
const PLACEMENT_CONSTRAINT_TOLERANCE: f64 = 1.0e-6;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GeometryValidationWork {
    pub component_pairs: usize,
    pub trace_segment_obstacle_pairs: usize,
    pub trace_segment_pairs: usize,
    pub via_obstacle_pairs: usize,
    pub via_trace_segment_pairs: usize,
    pub via_pairs: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GeometryValidationAssessment {
    pub complete: bool,
    pub exhaustive: bool,
    pub work: GeometryValidationWork,
    pub violations: Vec<Violation>,
}

#[derive(Clone, Copy, Debug)]
enum ExactShape {
    Circle { center: Vec2, radius: f64 },
    Rect(Obb),
}

impl ExactShape {
    fn segment_distance(self, from: Vec2, to: Vec2) -> f64 {
        match self {
            Self::Circle { center, radius } => {
                (point_segment_distance(center, from, to) - radius).max(0.0)
            }
            Self::Rect(obb) => layout_trace_model::geometry::segment_obb_distance(from, to, obb),
        }
    }

    fn point_distance(self, point: Vec2) -> f64 {
        match self {
            Self::Circle { center, radius } => (length(sub(point, center)) - radius).max(0.0),
            Self::Rect(obb) => point_obb_signed_distance(point, obb).0.max(0.0),
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
struct Obstacle {
    id: String,
    owner: String,
    pin: Option<String>,
    layer: String,
    kind: ObstacleKind,
    shape: ExactShape,
}

fn violation(
    violations: &mut Vec<Violation>,
    code: &str,
    mut objects: Vec<String>,
    required_distance: f64,
    actual_distance: f64,
    message: &str,
) {
    objects.sort();
    objects.dedup();
    violations.push(Violation {
        code: code.into(),
        objects,
        required_distance,
        actual_distance,
        message: message.into(),
    });
}

fn obb(component: &SolvedComponent) -> Obb {
    Obb {
        center: component.position,
        half_size: Vec2::new(component.size.x * 0.5, component.size.y * 0.5),
        rotation_degrees: component.rotation_degrees,
    }
}

fn shape_at(shape: &CopperShape, center: Vec2, component_rotation: f64) -> ExactShape {
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
            rotation_degrees: component_rotation + rotation_degrees,
        }),
    }
}

fn component_lookup<'a>(
    components: &'a [SolvedComponent],
    violations: &mut Vec<Violation>,
) -> BTreeMap<&'a str, &'a SolvedComponent> {
    let mut result = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for component in components {
        if result.insert(component.id.as_str(), component).is_some() {
            duplicates.insert(component.id.clone());
        }
    }
    for id in duplicates {
        result.remove(id.as_str());
        violation(
            violations,
            "duplicate_solved_component",
            vec![id],
            0.0,
            0.0,
            "solved component identity is duplicated",
        );
    }
    result
}

fn validate_component_poses(
    problem: &Problem,
    components: &[SolvedComponent],
    solved: &BTreeMap<&str, &SolvedComponent>,
    violations: &mut Vec<Violation>,
    work: &mut GeometryValidationWork,
) {
    for declared in &problem.components {
        let Some(candidate) = solved.get(declared.id.as_str()).copied() else {
            violation(
                violations,
                "missing_solved_component",
                vec![declared.id.clone()],
                0.0,
                0.0,
                "declared component is absent from the candidate",
            );
            continue;
        };
        if !candidate.position.x.is_finite()
            || !candidate.position.y.is_finite()
            || !candidate.size.x.is_finite()
            || !candidate.size.y.is_finite()
            || !candidate.rotation_degrees.is_finite()
            || candidate.size.x <= 0.0
            || candidate.size.y <= 0.0
        {
            violation(
                violations,
                "invalid_component_pose",
                vec![declared.id.clone()],
                0.0,
                0.0,
                "component pose and size must be finite and size must be positive",
            );
            continue;
        }
        if (candidate.size.x - declared.size.x).abs() > POSE_TOLERANCE
            || (candidate.size.y - declared.size.y).abs() > POSE_TOLERANCE
        {
            violation(
                violations,
                "component_size_mismatch",
                vec![declared.id.clone()],
                0.0,
                length(sub(candidate.size, declared.size)),
                "candidate changed the semantic component body size",
            );
        }
        match declared.constraints.movement {
            Movement::Fixed
                if length(sub(candidate.position, declared.position)) > POSE_TOLERANCE =>
            {
                violation(
                    violations,
                    "fixed_component_moved",
                    vec![declared.id.clone()],
                    0.0,
                    length(sub(candidate.position, declared.position)),
                    "fixed component moved from its declared position",
                );
            }
            Movement::Horizontal
                if (candidate.position.y - declared.position.y).abs() > POSE_TOLERANCE =>
            {
                violation(
                    violations,
                    "horizontal_component_moved_vertically",
                    vec![declared.id.clone()],
                    0.0,
                    (candidate.position.y - declared.position.y).abs(),
                    "horizontal-only component changed its y coordinate",
                );
            }
            Movement::Vertical
                if (candidate.position.x - declared.position.x).abs() > POSE_TOLERANCE =>
            {
                violation(
                    violations,
                    "vertical_component_moved_horizontally",
                    vec![declared.id.clone()],
                    0.0,
                    (candidate.position.x - declared.position.x).abs(),
                    "vertical-only component changed its x coordinate",
                );
            }
            _ => {}
        }
        if declared.constraints.rotation == Rotation::Fixed
            && angular_distance_degrees(candidate.rotation_degrees, declared.rotation_degrees)
                > POSE_TOLERANCE
        {
            violation(
                violations,
                "fixed_component_rotated",
                vec![declared.id.clone()],
                0.0,
                angular_distance_degrees(candidate.rotation_degrees, declared.rotation_degrees),
                "fixed-rotation component changed orientation",
            );
        }
        let candidate_obb = obb(candidate);
        if candidate_obb
            .corners()
            .iter()
            .any(|corner| !rect_contains_with_margin(problem.board.bounds, *corner, 0.0))
        {
            violation(
                violations,
                "component_outside_board",
                vec![declared.id.clone()],
                0.0,
                0.0,
                "oriented component body extends outside the board",
            );
        }
        if let Some(region) = declared.constraints.region
            && candidate_obb
                .corners()
                .iter()
                .any(|corner| !rect_contains_with_margin(region, *corner, 0.0))
        {
            violation(
                violations,
                "component_outside_region",
                vec![declared.id.clone()],
                0.0,
                0.0,
                "oriented component body extends outside its movement region",
            );
        }
    }
    let declared = problem
        .components
        .iter()
        .map(|component| component.id.as_str())
        .collect::<BTreeSet<_>>();
    for component in components {
        if !declared.contains(component.id.as_str()) {
            violation(
                violations,
                "extra_solved_component",
                vec![component.id.clone()],
                0.0,
                0.0,
                "candidate contains a component absent from the problem",
            );
        }
    }
    for first in 0..components.len() {
        for second in first + 1..components.len() {
            work.component_pairs += 1;
            if obbs_overlap(obb(&components[first]), obb(&components[second])) {
                violation(
                    violations,
                    "component_overlap",
                    vec![components[first].id.clone(), components[second].id.clone()],
                    0.0,
                    0.0,
                    "oriented component bodies overlap",
                );
            }
        }
    }
}

fn anchor_position(
    problem: &Problem,
    solved: &BTreeMap<&str, &SolvedComponent>,
    anchor: &PlacementAnchor,
) -> Option<Vec2> {
    let declared = problem
        .components
        .iter()
        .find(|component| component.id == anchor.component)?;
    let candidate = solved.get(anchor.component.as_str()).copied()?;
    let offset = anchor
        .pin
        .as_ref()
        .and_then(|pin| declared.pins.iter().find(|item| item.id == *pin))
        .map_or(Vec2::ZERO, |pin| pin.offset);
    Some(add(
        candidate.position,
        rotate_degrees(offset, candidate.rotation_degrees),
    ))
}

fn edge_gap(edge: BoardEdge, bounds: Rect, component: Obb) -> f64 {
    let corners = component.corners();
    match edge {
        BoardEdge::North => bounds.max.y - corners.iter().map(|p| p.y).fold(f64::MIN, f64::max),
        BoardEdge::East => bounds.max.x - corners.iter().map(|p| p.x).fold(f64::MIN, f64::max),
        BoardEdge::South => corners.iter().map(|p| p.y).fold(f64::MAX, f64::min) - bounds.min.y,
        BoardEdge::West => corners.iter().map(|p| p.x).fold(f64::MAX, f64::min) - bounds.min.x,
    }
}

fn validate_placement_constraints(
    problem: &Problem,
    solved: &BTreeMap<&str, &SolvedComponent>,
    violations: &mut Vec<Violation>,
) {
    for constraint in &problem.placement_constraints {
        match constraint {
            PlacementConstraint::MaximumDistance {
                first,
                second,
                maximum,
            } => {
                let Some((first_position, second_position)) =
                    anchor_position(problem, solved, first)
                        .zip(anchor_position(problem, solved, second))
                else {
                    continue;
                };
                let actual = length(sub(first_position, second_position));
                if actual > maximum + PLACEMENT_CONSTRAINT_TOLERANCE {
                    violation(
                        violations,
                        "maximum_placement_distance",
                        vec![first.component.clone(), second.component.clone()],
                        *maximum,
                        actual,
                        "placement anchors exceed their declared maximum distance",
                    );
                }
            }
            PlacementConstraint::FaceBoardEdge {
                component,
                edge,
                local_facing_direction_degrees,
                angular_tolerance_degrees,
                maximum_distance,
            } => {
                let Some(candidate) = solved.get(component.as_str()).copied() else {
                    continue;
                };
                let actual_angle = candidate.rotation_degrees + local_facing_direction_degrees;
                let angular_error =
                    angular_distance_degrees(actual_angle, edge.outward_angle_degrees());
                if angular_error > angular_tolerance_degrees + PLACEMENT_CONSTRAINT_TOLERANCE {
                    violation(
                        violations,
                        "component_edge_facing",
                        vec![component.clone()],
                        *angular_tolerance_degrees,
                        angular_error,
                        "component does not face its declared board edge",
                    );
                }
                if let Some(maximum) = maximum_distance {
                    let actual = edge_gap(*edge, problem.board.bounds, obb(candidate));
                    if actual > maximum + PLACEMENT_CONSTRAINT_TOLERANCE {
                        violation(
                            violations,
                            "component_edge_distance",
                            vec![component.clone()],
                            *maximum,
                            actual,
                            "component body is too far from its declared board edge",
                        );
                    }
                }
            }
        }
    }
}

fn obstacles(problem: &Problem, solved: &BTreeMap<&str, &SolvedComponent>) -> Vec<Obstacle> {
    let mut result = Vec::new();
    for declared in &problem.components {
        let Some(candidate) = solved.get(declared.id.as_str()).copied() else {
            continue;
        };
        if declared.body_is_routing_keepout {
            for layer in &problem.board.layers {
                result.push(Obstacle {
                    id: format!("body:{}", declared.id),
                    owner: declared.id.clone(),
                    pin: None,
                    layer: layer.id.clone(),
                    kind: ObstacleKind::Body,
                    shape: ExactShape::Rect(obb(candidate)),
                });
            }
        }
        for (index, keepout) in declared.routing_keepouts.iter().enumerate() {
            let center = add(
                candidate.position,
                rotate_degrees(keepout.offset, candidate.rotation_degrees),
            );
            result.push(Obstacle {
                id: format!("keepout:{}:{index}", declared.id),
                owner: declared.id.clone(),
                pin: None,
                layer: keepout.layer.clone(),
                kind: ObstacleKind::Keepout,
                shape: shape_at(&keepout.shape, center, candidate.rotation_degrees),
            });
        }
        for pin in &declared.pins {
            for pad in &pin.pads {
                let center = add(
                    candidate.position,
                    rotate_degrees(pin.pad_local_center(pad), candidate.rotation_degrees),
                );
                result.push(Obstacle {
                    id: format!("pad:{}:{}:{}", declared.id, pin.id, pad.id),
                    owner: declared.id.clone(),
                    pin: Some(pin.id.clone()),
                    layer: pad.layer.clone(),
                    kind: ObstacleKind::Pad,
                    shape: shape_at(&pad.shape, center, candidate.rotation_degrees),
                });
            }
        }
    }
    result
}

fn net_terminals(problem: &Problem) -> BTreeMap<String, BTreeSet<(String, String)>> {
    let mut result = BTreeMap::<String, BTreeSet<(String, String)>>::new();
    for net in &problem.nets {
        let id = net.electrical_net.as_ref().unwrap_or(&net.id).clone();
        result
            .entry(id.clone())
            .or_default()
            .insert((net.from.component.clone(), net.from.pin.clone()));
        result
            .entry(id)
            .or_default()
            .insert((net.to.component.clone(), net.to.pin.clone()));
    }
    for net in &problem.electrical_nets {
        result.entry(net.id.clone()).or_default().extend(
            net.terminals
                .iter()
                .map(|terminal| (terminal.component.clone(), terminal.pin.clone())),
        );
    }
    result
}

fn endpoint_owners(trace: &SolvedTrace, graphs: &[SolvedRouteGraph]) -> BTreeSet<String> {
    graphs
        .iter()
        .filter(|graph| graph.electrical_net == trace.electrical_net)
        .flat_map(|graph| &graph.nodes)
        .filter(|node| node.id == trace.from_node || node.id == trace.to_node)
        .filter_map(|node| match &node.kind {
            SolvedRouteNodeKind::Terminal { component, .. } => Some(component.clone()),
            _ => None,
        })
        .collect()
}

fn obstacle_is_exempt(
    obstacle: &Obstacle,
    trace: &SolvedTrace,
    attached: &BTreeSet<String>,
    terminals: &BTreeMap<String, BTreeSet<(String, String)>>,
) -> bool {
    if obstacle.kind == ObstacleKind::Body && attached.contains(&obstacle.owner) {
        return true;
    }
    obstacle.kind == ObstacleKind::Pad
        && obstacle.pin.as_ref().is_some_and(|pin| {
            terminals
                .get(&trace.electrical_net)
                .is_some_and(|items| items.contains(&(obstacle.owner.clone(), pin.clone())))
        })
}

fn via_layers(via: &SolvedVia) -> [&str; 2] {
    [&via.from_layer, &via.to_layer]
}

/// Run an exhaustive physical correctness gate. Approximate energies and
/// spatial indices are deliberately not consulted.
pub fn validate_geometry(
    problem: &Problem,
    components: &[SolvedComponent],
    traces: &[SolvedTrace],
    route_graphs: &[SolvedRouteGraph],
) -> GeometryValidationAssessment {
    let mut violations = Vec::new();
    let mut work = GeometryValidationWork::default();
    let solved = component_lookup(components, &mut violations);
    validate_component_poses(problem, components, &solved, &mut violations, &mut work);
    validate_placement_constraints(problem, &solved, &mut violations);
    let obstacles = obstacles(problem, &solved);
    let terminals = net_terminals(problem);
    let board_layers = problem
        .board
        .layers
        .iter()
        .map(|layer| layer.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut valid_traces = vec![false; traces.len()];

    for (trace_index, trace) in traces.iter().enumerate() {
        if trace.width <= 0.0
            || !trace.width.is_finite()
            || trace.points.len() < 2
            || trace
                .points
                .iter()
                .any(|point| !point.x.is_finite() || !point.y.is_finite())
        {
            violation(
                &mut violations,
                "invalid_trace_geometry",
                vec![trace.branch.clone()],
                0.0,
                0.0,
                "trace needs finite positive width and at least two points",
            );
            continue;
        }
        if trace.segment_layers.len() + 1 != trace.points.len()
            || trace
                .segment_layers
                .iter()
                .any(|layer| !board_layers.contains(layer.as_str()))
        {
            violation(
                &mut violations,
                "invalid_trace_layers",
                vec![trace.branch.clone()],
                0.0,
                0.0,
                "trace segment layers do not match its geometry or board",
            );
            continue;
        }
        valid_traces[trace_index] = true;
        let margin = trace.width * 0.5 + problem.rules.clearance;
        if trace
            .points
            .iter()
            .any(|point| !rect_contains_with_margin(problem.board.bounds, *point, margin))
        {
            violation(
                &mut violations,
                "trace_outside_board",
                vec![trace.branch.clone()],
                margin,
                0.0,
                "trace centerline is too close to the board boundary",
            );
        }
        let attached = endpoint_owners(trace, route_graphs);
        for (segment_index, segment) in trace.points.windows(2).enumerate() {
            let layer = &trace.segment_layers[segment_index];
            for obstacle in obstacles.iter().filter(|obstacle| obstacle.layer == *layer) {
                if obstacle_is_exempt(obstacle, trace, &attached, &terminals) {
                    continue;
                }
                work.trace_segment_obstacle_pairs += 1;
                let actual = obstacle.shape.segment_distance(segment[0], segment[1]);
                if actual + EPSILON < margin {
                    violation(
                        &mut violations,
                        "trace_obstacle_clearance",
                        vec![trace.branch.clone(), obstacle.id.clone()],
                        margin,
                        actual,
                        "trace does not maintain clearance from component copper or keepout",
                    );
                }
            }
        }
    }

    for first in 0..traces.len() {
        for second in first + 1..traces.len() {
            if !valid_traces[first]
                || !valid_traces[second]
                || traces[first].electrical_net == traces[second].electrical_net
            {
                continue;
            }
            let required =
                (traces[first].width + traces[second].width) * 0.5 + problem.rules.clearance;
            for (first_index, first_segment) in traces[first].points.windows(2).enumerate() {
                for (second_index, second_segment) in traces[second].points.windows(2).enumerate() {
                    if traces[first].segment_layers.get(first_index)
                        != traces[second].segment_layers.get(second_index)
                    {
                        continue;
                    }
                    work.trace_segment_pairs += 1;
                    let actual = segment_distance(
                        first_segment[0],
                        first_segment[1],
                        second_segment[0],
                        second_segment[1],
                    );
                    if actual + EPSILON < required {
                        violation(
                            &mut violations,
                            "trace_trace_clearance",
                            vec![traces[first].branch.clone(), traces[second].branch.clone()],
                            required,
                            actual,
                            "different electrical nets do not maintain copper clearance",
                        );
                    }
                }
            }
        }
    }

    for (trace_index, trace) in traces.iter().enumerate() {
        if !valid_traces[trace_index] {
            continue;
        }
        for via in &trace.vias {
            if !via.position.x.is_finite()
                || !via.position.y.is_finite()
                || !via.diameter.is_finite()
                || !via.drill.is_finite()
                || !board_layers.contains(via.from_layer.as_str())
                || !board_layers.contains(via.to_layer.as_str())
            {
                violation(
                    &mut violations,
                    "invalid_via_geometry",
                    vec![trace.branch.clone()],
                    0.0,
                    0.0,
                    "via position, dimensions, and layer references must be valid",
                );
                continue;
            }
            let required_board = via.diameter * 0.5 + problem.rules.clearance;
            if via.diameter + EPSILON < problem.rules.via_diameter
                || via.drill + EPSILON < problem.rules.via_drill
                || via.drill > via.diameter + EPSILON
            {
                violation(
                    &mut violations,
                    "invalid_via_dimensions",
                    vec![trace.branch.clone()],
                    problem.rules.via_diameter,
                    via.diameter,
                    "via dimensions are smaller than the declared fabrication rule",
                );
            }
            if !rect_contains_with_margin(problem.board.bounds, via.position, required_board) {
                violation(
                    &mut violations,
                    "via_outside_board",
                    vec![trace.branch.clone()],
                    required_board,
                    0.0,
                    "via annulus is too close to the board boundary",
                );
            }
            for obstacle in obstacles
                .iter()
                .filter(|obstacle| via_layers(via).contains(&obstacle.layer.as_str()))
            {
                if problem.rules.allow_via_in_pad
                    && obstacle.kind == ObstacleKind::Pad
                    && obstacle_is_exempt(
                        obstacle,
                        trace,
                        &endpoint_owners(trace, route_graphs),
                        &terminals,
                    )
                {
                    continue;
                }
                work.via_obstacle_pairs += 1;
                let actual = obstacle.shape.point_distance(via.position);
                let required = via.diameter * 0.5 + problem.rules.clearance;
                if actual + EPSILON < required {
                    violation(
                        &mut violations,
                        "via_obstacle_clearance",
                        vec![trace.branch.clone(), obstacle.id.clone()],
                        required,
                        actual,
                        "via does not maintain clearance from component copper or keepout",
                    );
                }
            }
            for (other_index, other) in traces.iter().enumerate() {
                if !valid_traces[other_index]
                    || other_index == trace_index
                    || other.electrical_net == trace.electrical_net
                {
                    continue;
                }
                for (segment_index, segment) in other.points.windows(2).enumerate() {
                    if !via_layers(via).contains(&other.segment_layers[segment_index].as_str()) {
                        continue;
                    }
                    work.via_trace_segment_pairs += 1;
                    let actual = point_segment_distance(via.position, segment[0], segment[1]);
                    let required = via.diameter * 0.5 + other.width * 0.5 + problem.rules.clearance;
                    if actual + EPSILON < required {
                        violation(
                            &mut violations,
                            "via_trace_clearance",
                            vec![trace.branch.clone(), other.branch.clone()],
                            required,
                            actual,
                            "via does not maintain clearance from another electrical net",
                        );
                    }
                }
            }
        }
    }

    let vias = traces
        .iter()
        .enumerate()
        .filter(|(trace_index, _)| valid_traces[*trace_index])
        .flat_map(|(_, trace)| trace.vias.iter().map(move |via| (trace, via)))
        .filter(|(_, via)| {
            via.position.x.is_finite()
                && via.position.y.is_finite()
                && via.diameter.is_finite()
                && via.drill.is_finite()
                && board_layers.contains(via.from_layer.as_str())
                && board_layers.contains(via.to_layer.as_str())
        })
        .collect::<Vec<_>>();
    for first in 0..vias.len() {
        for second in first + 1..vias.len() {
            let (first_trace, first_via) = vias[first];
            let (second_trace, second_via) = vias[second];
            if first_trace.electrical_net == second_trace.electrical_net
                || !via_layers(first_via)
                    .iter()
                    .any(|layer| via_layers(second_via).contains(layer))
            {
                continue;
            }
            work.via_pairs += 1;
            let required =
                (first_via.diameter + second_via.diameter) * 0.5 + problem.rules.clearance;
            let actual = length(sub(first_via.position, second_via.position));
            if actual + EPSILON < required {
                violation(
                    &mut violations,
                    "via_via_clearance",
                    vec![first_trace.branch.clone(), second_trace.branch.clone()],
                    required,
                    actual,
                    "vias on different electrical nets do not maintain clearance",
                );
            }
        }
    }

    violations.sort_by(|left, right| {
        (
            left.code.as_str(),
            left.objects.as_slice(),
            left.message.as_str(),
        )
            .cmp(&(
                right.code.as_str(),
                right.objects.as_slice(),
                right.message.as_str(),
            ))
    });
    violations.dedup_by(|left, right| {
        left.code == right.code
            && left.objects == right.objects
            && left.required_distance.to_bits() == right.required_distance.to_bits()
            && left.actual_distance.to_bits() == right.actual_distance.to_bits()
    });
    GeometryValidationAssessment {
        complete: violations.is_empty(),
        exhaustive: true,
        work,
        violations,
    }
}

#[cfg(test)]
mod tests {
    use layout_trace_model::topology::{RouteClass, TerminalSector};

    use super::*;

    fn problem() -> Problem {
        serde_json::from_str(
            r#"{
                "schema_version":1,
                "board":{"bounds":{"min":{"x":0.0,"y":0.0},"max":{"x":20.0,"y":20.0}},"layers":[{"id":"top"}]},
                "rules":{"clearance":0.2,"via_diameter":0.8,"via_drill":0.4},
                "components":[
                    {"id":"A","position":{"x":2.0,"y":10.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"fixed","rotation":"free"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"id":"p","layer":"top","shape":{"kind":"circle","diameter":0.8}}]}]},
                    {"id":"B","position":{"x":18.0,"y":10.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"id":"p","layer":"top","shape":{"kind":"circle","diameter":0.8}}]}]}
                ],
                "nets":[
                    {"id":"N1","width":0.4,"layer":"top","from":{"component":"A","pin":"1"},"to":{"component":"B","pin":"1"}},
                    {"id":"N2","width":0.4,"layer":"top","from":{"component":"A","pin":"1"},"to":{"component":"B","pin":"1"}}
                ]
            }"#,
        )
        .unwrap()
    }

    fn component(id: &str, x: f64) -> SolvedComponent {
        SolvedComponent {
            id: id.into(),
            position: Vec2::new(x, 10.0),
            size: Vec2::new(1.0, 1.0),
            rotation_degrees: 0.0,
        }
    }

    fn trace(id: &str, net: &str, from: Vec2, to: Vec2) -> SolvedTrace {
        SolvedTrace {
            branch: id.into(),
            electrical_net: net.into(),
            net: id.into(),
            from_node: format!("{id}-from"),
            to_node: format!("{id}-to"),
            width: 0.4,
            tension_weight: 1.0,
            layer: "top".into(),
            segment_layers: vec!["top".into()],
            vias: Vec::new(),
            points: vec![from, to],
            route_class: RouteClass::direct(
                "top",
                TerminalSector {
                    component: "A".into(),
                    sector: "1".into(),
                },
                TerminalSector {
                    component: "B".into(),
                    sector: "1".into(),
                },
            ),
            route_basis_fingerprint: None,
        }
    }

    #[test]
    fn detects_an_exact_crossing_between_different_nets() {
        let problem = problem();
        let traces = vec![
            trace("N1", "N1", Vec2::new(2.0, 8.0), Vec2::new(18.0, 12.0)),
            trace("N2", "N2", Vec2::new(2.0, 12.0), Vec2::new(18.0, 8.0)),
        ];
        let result = validate_geometry(
            &problem,
            &[component("A", 2.0), component("B", 18.0)],
            &traces,
            &[],
        );
        assert!(
            result
                .violations
                .iter()
                .any(|item| item.code == "trace_trace_clearance")
        );
        assert_eq!(result.work.trace_segment_pairs, 1);
    }

    #[test]
    fn free_rotation_is_preserved_while_fixed_rotation_is_enforced() {
        let problem = problem();
        let mut first = component("A", 2.0);
        first.rotation_degrees = 83.0;
        let mut second = component("B", 18.0);
        second.rotation_degrees = 12.0;
        let result = validate_geometry(&problem, &[first, second], &[], &[]);
        assert!(
            !result
                .violations
                .iter()
                .any(|item| item.objects == ["A"] && item.code == "fixed_component_rotated")
        );
        assert!(
            result
                .violations
                .iter()
                .any(|item| item.objects == ["B"] && item.code == "fixed_component_rotated")
        );
    }

    #[test]
    fn candidate_cannot_make_a_component_smaller_to_pass() {
        let problem = problem();
        let mut first = component("A", 2.0);
        first.size.x = 0.1;
        let result = validate_geometry(&problem, &[first, component("B", 18.0)], &[], &[]);
        assert!(
            result
                .violations
                .iter()
                .any(|item| item.code == "component_size_mismatch")
        );
    }

    #[test]
    fn undersized_via_is_rejected() {
        let problem = problem();
        let mut route = trace("N1", "N1", Vec2::new(2.0, 10.0), Vec2::new(18.0, 10.0));
        route.vias.push(SolvedVia {
            position: Vec2::new(10.0, 10.0),
            from_layer: "top".into(),
            to_layer: "top".into(),
            diameter: 0.2,
            drill: 0.1,
            point_index: 0,
        });
        let result = validate_geometry(
            &problem,
            &[component("A", 2.0), component("B", 18.0)],
            &[route],
            &[],
        );
        assert!(
            result
                .violations
                .iter()
                .any(|item| item.code == "invalid_via_dimensions")
        );
    }

    #[test]
    fn non_finite_candidate_values_fail_closed() {
        let problem = problem();
        let mut first = component("A", 2.0);
        first.position.x = f64::NAN;
        let route = trace("N1", "N1", Vec2::new(2.0, 10.0), Vec2::new(f64::NAN, 10.0));
        let result = validate_geometry(&problem, &[first, component("B", 18.0)], &[route], &[]);
        assert!(
            result
                .violations
                .iter()
                .any(|item| item.code == "invalid_component_pose")
        );
        assert!(
            result
                .violations
                .iter()
                .any(|item| item.code == "invalid_trace_geometry")
        );
    }

    #[test]
    fn relational_tolerance_matches_the_placement_projector_contract() {
        let problem: Problem = serde_json::from_str(
            r#"{
                "schema_version":1,
                "board":{"bounds":{"min":{"x":0.0,"y":0.0},"max":{"x":20.0,"y":20.0}},"layers":[{"id":"top"}]},
                "rules":{"clearance":0.2},
                "components":[
                    {"id":"A","position":{"x":2.0,"y":10.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"free","rotation":"fixed"},"pins":[]},
                    {"id":"B","position":{"x":7.5,"y":10.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"free","rotation":"fixed"},"pins":[]}
                ],
                "placement_constraints":[{"kind":"maximum_distance","first":{"component":"A"},"second":{"component":"B"},"maximum":5.5}]
            }"#,
        )
        .unwrap();
        let within_projector_tolerance = validate_geometry(
            &problem,
            &[component("A", 2.0), component("B", 7.500_000_5)],
            &[],
            &[],
        );
        assert!(within_projector_tolerance.complete);

        let outside_projector_tolerance = validate_geometry(
            &problem,
            &[component("A", 2.0), component("B", 7.500_002)],
            &[],
            &[],
        );
        assert!(
            outside_projector_tolerance
                .violations
                .iter()
                .any(|violation| violation.code == "maximum_placement_distance")
        );
    }
}

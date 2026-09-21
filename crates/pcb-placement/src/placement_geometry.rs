// Copyright (C) 2026 Toit contributors.

//! Mechanical bodies and copper pads have separate exclusion margins. The
//! legacy path is untouched when neither component declares explicit geometry.
use super::*;
use layout_trace_model::model::Component;

struct Primitive {
    center: Vec2,
    shape: CopperShape,
    rotation: f64,
    margin: f64,
    copper: bool,
    /// Rotated offsets from center for a convex mechanical body part.
    polygon: Option<Vec<Vec2>>,
    /// Empty retains legacy all-layer exclusions.
    layers: Vec<String>,
}

impl Primitive {
    fn projection(&self, axis: Vec2) -> (f64, f64) {
        let center = dot(self.center, axis);
        if let Some(vertices) = &self.polygon {
            let low = vertices
                .iter()
                .map(|v| dot(*v, axis))
                .fold(f64::INFINITY, f64::min);
            let high = vertices
                .iter()
                .map(|v| dot(*v, axis))
                .fold(f64::NEG_INFINITY, f64::max);
            (center + low, center + high)
        } else {
            let extent = self.support(axis);
            (center - extent, center + extent)
        }
    }

    fn support(&self, axis: Vec2) -> f64 {
        if let Some(vertices) = &self.polygon {
            return vertices
                .iter()
                .map(|v| dot(*v, axis).abs())
                .fold(0.0, f64::max);
        }
        match self.shape {
            CopperShape::Circle { diameter } => diameter / 2.0,
            CopperShape::Rect { size, .. } => {
                let axes = oriented_axes(self.rotation);
                (size.x * dot(axis, axes[0]).abs() + size.y * dot(axis, axes[1]).abs()) / 2.0
            }
        }
    }
}

fn primitives(component: &Component, pose: &PlacementPose, clearance: f64) -> Vec<Primitive> {
    let Some(geometry) = &component.placement_geometry else {
        let (_, size) = placement_envelope(component);
        return vec![Primitive {
            center: placement_envelope_center(component, pose),
            shape: CopperShape::Rect {
                size,
                rotation_degrees: 0.0,
            },
            rotation: pose.rotation_degrees,
            margin: clearance,
            copper: false,
            polygon: None,
            layers: Vec::new(),
        }];
    };
    let mut result = vec![Primitive {
        center: pose.position,
        shape: geometry.body.clone(),
        rotation: pose.rotation_degrees,
        margin: geometry.clearance,
        copper: false,
        polygon: None,
        layers: geometry.body_layers.clone(),
    }];
    if !geometry.body_parts.is_empty() {
        result = geometry
            .body_parts
            .iter()
            .map(|part| Primitive {
                center: pose.position,
                shape: geometry.body.clone(),
                rotation: pose.rotation_degrees,
                margin: geometry.clearance,
                copper: false,
                polygon: Some(
                    part.vertices
                        .iter()
                        .map(|v| rotated(*v, pose.rotation_degrees))
                        .collect(),
                ),
                layers: geometry.body_layers.clone(),
            })
            .collect();
    }
    for pin in &component.pins {
        for pad in &pin.pads {
            let rotation = match pad.shape {
                CopperShape::Circle { .. } => 0.0,
                CopperShape::Rect {
                    rotation_degrees, ..
                } => rotation_degrees,
            };
            result.push(Primitive {
                center: add(
                    pose.position,
                    rotated(pin.pad_local_center(pad), pose.rotation_degrees),
                ),
                shape: pad.shape.clone(),
                rotation: pose.rotation_degrees + rotation,
                margin: clearance,
                copper: true,
                polygon: None,
                layers: if geometry.body_layers.is_empty() {
                    Vec::new()
                } else {
                    vec![pad.layer.clone()]
                },
            });
        }
    }
    result
}

fn primitive_overlap(a: &Primitive, b: &Primitive, stable: Vec2) -> Option<(Vec2, f64)> {
    if !a.layers.is_empty()
        && !b.layers.is_empty()
        && !a.layers.iter().any(|layer| b.layers.contains(layer))
    {
        return None;
    }
    let delta = sub(a.center, b.center);
    let mut axes = Vec::new();
    for p in [a, b] {
        if let Some(vertices) = &p.polygon {
            for (first, second) in vertices
                .iter()
                .zip(vertices.iter().cycle().skip(1))
                .take(vertices.len())
            {
                let edge = sub(*second, *first);
                let length = vector_length(edge);
                axes.push(Vec2::new(-edge.y / length, edge.x / length));
            }
        } else if matches!(p.shape, CopperShape::Rect { .. }) {
            axes.extend(oriented_axes(p.rotation));
        }
    }
    if a.polygon.is_some() || b.polygon.is_some() {
        for (polygon, circle) in [(a, b), (b, a)] {
            if let Some(vertices) = &polygon.polygon
                && matches!(circle.shape, CopperShape::Circle { .. })
            {
                for vertex in vertices {
                    let direction = sub(add(polygon.center, *vertex), circle.center);
                    let length = vector_length(direction);
                    if length > FEASIBILITY_TOLERANCE_MM {
                        axes.push(scale(direction, 1.0 / length));
                    }
                }
            }
        }
    } else {
        match (&a.shape, &b.shape) {
            (CopperShape::Circle { .. }, CopperShape::Circle { .. }) => {
                let length = vector_length(delta);
                axes.push(if length > FEASIBILITY_TOLERANCE_MM {
                    scale(delta, 1.0 / length)
                } else {
                    stable
                });
            }
            (CopperShape::Circle { .. }, CopperShape::Rect { .. })
            | (CopperShape::Rect { .. }, CopperShape::Circle { .. }) => {
                let (circle, rect) = if matches!(a.shape, CopperShape::Circle { .. }) {
                    (a, b)
                } else {
                    (b, a)
                };
                let CopperShape::Rect { size, .. } = rect.shape else {
                    unreachable!()
                };
                let relative = rotated(sub(circle.center, rect.center), -rect.rotation);
                let closest = Vec2::new(
                    relative.x.clamp(-size.x / 2.0, size.x / 2.0),
                    relative.y.clamp(-size.y / 2.0, size.y / 2.0),
                );
                let direction = rotated(sub(relative, closest), rect.rotation);
                let length = vector_length(direction);
                if length > FEASIBILITY_TOLERANCE_MM {
                    axes.push(scale(direction, 1.0 / length));
                }
            }
            _ => {}
        }
    }
    // Body/pad pairs need mechanical exclusion. Only copper/copper pairs
    // inherit copper clearance; a legacy envelope retains its old margin.
    let margin = if a.copper && b.copper || !a.copper && !b.copper {
        a.margin.max(b.margin)
    } else if a.copper {
        b.margin
    } else {
        a.margin
    };
    let mut best = (Vec2::ZERO, f64::INFINITY);
    for mut axis in axes {
        if a.polygon.is_some() || b.polygon.is_some() {
            let (a_low, a_high) = a.projection(axis);
            let (b_low, b_high) = b.projection(axis);
            let positive = b_high - a_low + margin;
            let negative = a_high - b_low + margin;
            if positive <= FEASIBILITY_TOLERANCE_MM || negative <= FEASIBILITY_TOLERANCE_MM {
                return None;
            }
            let depth = if negative < positive {
                axis = scale(axis, -1.0);
                negative
            } else {
                positive
            };
            if depth < best.1 {
                best = (axis, depth);
            }
            continue;
        }
        if dot(delta, axis) < -FEASIBILITY_TOLERANCE_MM
            || (dot(delta, axis).abs() <= FEASIBILITY_TOLERANCE_MM && dot(stable, axis) < 0.0)
        {
            axis = scale(axis, -1.0);
        }
        let depth = a.support(axis) + b.support(axis) + margin - dot(delta, axis).abs();
        if depth <= FEASIBILITY_TOLERANCE_MM {
            return None;
        }
        if depth < best.1 {
            best = (axis, depth);
        }
    }
    Some(best)
}

fn primitives_with_extra_pad_gap(
    component: &Component,
    pose: &PlacementPose,
    clearance: f64,
    extra_pad_gap: f64,
) -> Vec<Primitive> {
    let mut result = primitives(component, pose, clearance);
    if extra_pad_gap > 0.0 {
        for primitive in &mut result {
            if primitive.copper {
                primitive.margin += extra_pad_gap;
            }
        }
    }
    result
}

/// Cached local geometry for conservative separation certificates. An axis is
/// usable only when the exact SAT predicate tests it for every primitive of
/// at least one complete footprint. Circles have no fixed certifying axes.
/// This intentionally falls through when no common axis exists; world AABBs
/// plus a radial margin are not safe for the existing relaxed SAT margins.
pub(super) struct PreparedProjectionGeometry {
    primitives: Vec<Primitive>,
    common_axes: Vec<Vec2>,
    maximum_margin: f64,
}

fn primitive_fixed_axes(primitive: &Primitive) -> Vec<Vec2> {
    if let Some(vertices) = &primitive.polygon {
        vertices
            .iter()
            .zip(vertices.iter().cycle().skip(1))
            .take(vertices.len())
            .map(|(first, second)| {
                let edge = sub(*second, *first);
                let length = vector_length(edge);
                Vec2::new(-edge.y / length, edge.x / length)
            })
            .collect()
    } else if matches!(primitive.shape, CopperShape::Rect { .. }) {
        oriented_axes(primitive.rotation).to_vec()
    } else {
        Vec::new()
    }
}

fn same_axis(first: Vec2, second: Vec2) -> bool {
    // No angular tolerance: only axes used verbatim by the predicate certify
    // its rejection. Floating signed zero compares equal in Vec2.
    first == second || first == scale(second, -1.0)
}

pub(super) fn prepare_projection_geometry(
    component: &Component,
    rotation_degrees: f64,
    clearance: f64,
) -> PreparedProjectionGeometry {
    prepare_projection_geometry_with_pad_gap(component, rotation_degrees, clearance, 0.0)
}

pub(super) fn prepare_projection_geometry_with_pad_gap(
    component: &Component,
    rotation_degrees: f64,
    clearance: f64,
    extra_pad_gap: f64,
) -> PreparedProjectionGeometry {
    let local = PlacementPose {
        component: component.id.clone(),
        position: Vec2::ZERO,
        rotation_degrees,
    };
    let primitives = primitives_with_extra_pad_gap(component, &local, clearance, extra_pad_gap);
    let mut common_axes = primitive_fixed_axes(&primitives[0]);
    for primitive in &primitives[1..] {
        let axes = primitive_fixed_axes(primitive);
        common_axes.retain(|axis| axes.iter().any(|other| same_axis(*axis, *other)));
    }
    let mut unique = Vec::new();
    for axis in common_axes {
        if !unique.iter().any(|other| same_axis(axis, *other)) {
            unique.push(axis);
        }
    }
    let maximum_margin = primitives.iter().map(|p| p.margin).fold(0.0, f64::max);
    PreparedProjectionGeometry {
        primitives,
        common_axes: unique,
        maximum_margin,
    }
}

pub(super) struct PairProjectionBounds {
    rows: Vec<(Vec2, (f64, f64), (f64, f64))>,
    margin: f64,
}

pub(super) fn prepare_pair_projection_bounds(
    first: &PreparedProjectionGeometry,
    second: &PreparedProjectionGeometry,
) -> PairProjectionBounds {
    let mut axes = first.common_axes.clone();
    for axis in &second.common_axes {
        if !axes.iter().any(|other| same_axis(*axis, *other)) {
            axes.push(*axis);
        }
    }
    let projection = |geometry: &PreparedProjectionGeometry, axis| {
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for primitive in &geometry.primitives {
            let (a, b) = primitive.projection(axis);
            // f64::min/max silently ignore NaN. Do not turn a partially invalid
            // primitive union into a finite separating certificate.
            if !a.is_finite() || !b.is_finite() {
                return (f64::NAN, f64::NAN);
            }
            low = low.min(a);
            high = high.max(b);
        }
        (low, high)
    };
    let rows = axes
        .into_iter()
        .map(|axis| (axis, projection(first, axis), projection(second, axis)))
        .collect();
    PairProjectionBounds {
        rows,
        margin: first.maximum_margin.max(second.maximum_margin),
    }
}

impl PairProjectionBounds {
    pub(super) fn certainly_disjoint(&self, first: Vec2, second: Vec2) -> bool {
        self.rows
            .iter()
            .any(|&(axis, (a_low, a_high), (b_low, b_high))| {
                let a = dot(first, axis);
                let b = dot(second, axis);
                if ![
                    axis.x,
                    axis.y,
                    a,
                    b,
                    a_low,
                    a_high,
                    b_low,
                    b_high,
                    self.margin,
                ]
                .iter()
                .all(|v| v.is_finite())
                {
                    return false;
                }
                // Local cached projections associate floating operations differently
                // from rebuilding world primitives. Only expand the allowed overlap;
                // uncertain/borderline separations go to the unchanged exact test.
                let roundoff = 32.0
                    * f64::EPSILON
                    * (1.0
                        + first.x.abs()
                        + first.y.abs()
                        + second.x.abs()
                        + second.y.abs()
                        + a_low.abs()
                        + a_high.abs()
                        + b_low.abs()
                        + b_high.abs()
                        + self.margin.abs());
                let margin = self.margin + FEASIBILITY_TOLERANCE_MM + roundoff;
                a + a_low - (b + b_high) > margin || b + b_low - (a + a_high) > margin
            })
    }
}

/// A strict subset of centers rejected by a fixed-axis primitive SAT test.
/// Circles are deliberately absent: their extra axes can change with position.
pub(super) struct PreparedCollisionIntervals {
    pairs: Vec<Vec<(Vec2, f64, f64)>>,
    coordinate_scale: f64,
}

pub(super) fn prepare_collision_intervals(
    a: &PreparedProjectionGeometry,
    b: &PreparedProjectionGeometry,
    b_position: Vec2,
) -> PreparedCollisionIntervals {
    let mut result = PreparedCollisionIntervals {
        pairs: Vec::new(),
        coordinate_scale: b_position.x.abs() + b_position.y.abs(),
    };
    for first in &a.primitives {
        for second in &b.primitives {
            if matches!(first.shape, CopperShape::Circle { .. })
                || matches!(second.shape, CopperShape::Circle { .. })
                || (!first.layers.is_empty()
                    && !second.layers.is_empty()
                    && !first
                        .layers
                        .iter()
                        .any(|layer| second.layers.contains(layer)))
            {
                continue;
            }
            let margin = if first.copper == second.copper {
                first.margin.max(second.margin)
            } else if first.copper {
                second.margin
            } else {
                first.margin
            };
            let mut rows = Vec::new();
            let axes = primitive_fixed_axes(first)
                .into_iter()
                .chain(primitive_fixed_axes(second));
            let mut raw_scale = 0.0_f64;
            let finite_vertices = [first, second].iter().all(|p| {
                raw_scale = raw_scale.max(p.center.x.abs() + p.center.y.abs());
                p.center.x.is_finite()
                    && p.center.y.is_finite()
                    && p.polygon.as_ref().is_none_or(|vertices| {
                        vertices.iter().all(|v| {
                            raw_scale = raw_scale.max(v.x.abs() + v.y.abs());
                            v.x.is_finite() && v.y.is_finite()
                        })
                    })
            });
            if !finite_vertices || !raw_scale.is_finite() {
                continue;
            }
            result.coordinate_scale = result.coordinate_scale.max(raw_scale);
            for axis in axes {
                let (al, ah) = first.projection(axis);
                let (bl, bh) = second.projection(axis);
                let offset = dot(b_position, axis);
                // Moving-footprint projection t must lie strictly inside these
                // limits on EVERY tested axis for this primitive pair to collide.
                let low = bl + offset - ah - margin;
                let high = bh + offset - al + margin;
                if ![axis.x, axis.y, al, ah, bl, bh, offset, margin, low, high]
                    .iter()
                    .all(|v| v.is_finite())
                    || axis == Vec2::ZERO
                {
                    rows.clear();
                    break;
                }
                result.coordinate_scale = result
                    .coordinate_scale
                    .max(al.abs() + ah.abs() + bl.abs() + bh.abs() + margin.abs());
                rows.push((axis, low, high));
            }
            if !rows.is_empty() {
                result.pairs.push(rows);
            }
        }
    }
    result
}

impl PreparedCollisionIntervals {
    pub(super) fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Open y intervals wholly inside one primitive-pair collision at fixed x.
    /// The returned intervals may overlap; their union is still certainly illegal.
    pub(super) fn row_intervals(&self, x: f64, low_y: f64, high_y: f64) -> Vec<(f64, f64)> {
        let scale = 1.0 + self.coordinate_scale + x.abs() + low_y.abs() + high_y.abs();
        let guard = FEASIBILITY_TOLERANCE_MM + 128.0 * f64::EPSILON * scale;
        if ![x, low_y, high_y, scale, guard]
            .iter()
            .all(|v| v.is_finite())
        {
            return Vec::new();
        }
        let mut intervals = Vec::new();
        for rows in &self.pairs {
            let mut low = f64::NEG_INFINITY;
            let mut high = f64::INFINITY;
            let mut certain = true;
            for &(axis, minimum, maximum) in rows {
                let minimum = minimum + guard;
                let maximum = maximum - guard;
                if minimum >= maximum {
                    certain = false;
                    break;
                }
                let offset = axis.x * x;
                if axis.y == 0.0 {
                    if !(minimum < offset && offset < maximum) {
                        certain = false;
                        break;
                    }
                    continue;
                }
                let a = (minimum - offset) / axis.y;
                let b = (maximum - offset) / axis.y;
                if !a.is_finite() || !b.is_finite() {
                    certain = false;
                    break;
                }
                // Division/intersection rounds inward, never across a contact.
                let y_guard =
                    FEASIBILITY_TOLERANCE_MM + 128.0 * f64::EPSILON * (scale + a.abs() + b.abs());
                low = low.max(a.min(b) + y_guard);
                high = high.min(a.max(b) - y_guard);
                if low >= high {
                    certain = false;
                    break;
                }
            }
            if certain && low < high && high > low_y && low < high_y {
                intervals.push((low, high));
            }
        }
        intervals
    }
}

pub(super) fn overlap_correction(
    a: &Component,
    ap: &PlacementPose,
    b: &Component,
    bp: &PlacementPose,
    clearance: f64,
) -> Option<(Vec2, f64)> {
    overlap_correction_with_pad_gap(a, ap, b, bp, clearance, 0.0)
}

pub(super) fn overlap_correction_with_pad_gap(
    a: &Component,
    ap: &PlacementPose,
    b: &Component,
    bp: &PlacementPose,
    clearance: f64,
    extra_pad_gap: f64,
) -> Option<(Vec2, f64)> {
    let first = primitives_with_extra_pad_gap(a, ap, clearance, extra_pad_gap);
    let second = primitives_with_extra_pad_gap(b, bp, clearance, extra_pad_gap);
    let stable = stable_pair_direction(&a.id, &b.id);
    let mut deepest: Option<(Vec2, f64)> = None;
    for a in &first {
        for b in &second {
            if let Some(correction) = primitive_overlap(a, b, stable)
                && deepest.is_none_or(|previous| correction.1 > previous.1)
            {
                deepest = Some(correction);
            }
        }
    }
    deepest
}

/// Sufficient separation of all interacting body/pad primitives on one axis.
/// Unlike the minimum-overlap normal, this may move a pair around a blockage.
/// Nonconvex shapes can fit more closely; ordinary geometry still checks every
/// resulting proposal, and failure of this conservative branch proves nothing
/// about the other possible placements.
pub(super) fn separation_along_axis(
    a: &Component,
    ap: &PlacementPose,
    b: &Component,
    bp: &PlacementPose,
    clearance: f64,
    axis: Vec2,
) -> Option<f64> {
    let first = primitives(a, ap, clearance);
    let second = primitives(b, bp, clearance);
    let mut required: Option<f64> = None;
    for a in &first {
        for b in &second {
            if !a.layers.is_empty()
                && !b.layers.is_empty()
                && !a.layers.iter().any(|layer| b.layers.contains(layer))
            {
                continue;
            }
            let margin = if a.copper && b.copper || !a.copper && !b.copper {
                a.margin.max(b.margin)
            } else if a.copper {
                b.margin
            } else {
                a.margin
            };
            let depth = b.projection(axis).1 - a.projection(axis).0 + margin;
            required = Some(required.map_or(depth, |previous| previous.max(depth)));
        }
    }
    required
}

pub(super) fn position_limits(
    problem: &Problem,
    component: &Component,
    pose: &PlacementPose,
    legacy_allowed: Rect,
) -> (Vec2, Vec2) {
    let Some(geometry) = &component.placement_geometry else {
        let extent = rotated_extent(component.size, pose.rotation_degrees);
        return (
            add(legacy_allowed.min, extent),
            sub(legacy_allowed.max, extent),
        );
    };
    let mut minimum = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut maximum = Vec2::new(f64::INFINITY, f64::INFINITY);
    for primitive in primitives(component, pose, 0.0) {
        let overhang = if primitive.copper {
            -geometry.pad_edge_clearance
        } else {
            geometry.board_overhang
        };
        let expansion = Vec2::new(overhang, overhang);
        let mut allowed = Rect {
            min: sub(problem.board.bounds.min, expansion),
            max: add(problem.board.bounds.max, expansion),
        };
        if let Some(region) = component.constraints.region {
            allowed.min.x = allowed.min.x.max(region.min.x);
            allowed.min.y = allowed.min.y.max(region.min.y);
            allowed.max.x = allowed.max.x.min(region.max.x);
            allowed.max.y = allowed.max.y.min(region.max.y);
        }
        let (low, high) = if primitive.polygon.is_some() {
            let (x_low, x_high) = primitive.projection(Vec2::new(1.0, 0.0));
            let (y_low, y_high) = primitive.projection(Vec2::new(0.0, 1.0));
            (
                Vec2::new(
                    allowed.min.x - (x_low - pose.position.x),
                    allowed.min.y - (y_low - pose.position.y),
                ),
                Vec2::new(
                    allowed.max.x - (x_high - pose.position.x),
                    allowed.max.y - (y_high - pose.position.y),
                ),
            )
        } else {
            let offset = sub(primitive.center, pose.position);
            let extent = Vec2::new(
                primitive.support(Vec2::new(1.0, 0.0)),
                primitive.support(Vec2::new(0.0, 1.0)),
            );
            (
                sub(add(allowed.min, extent), offset),
                sub(sub(allowed.max, extent), offset),
            )
        };
        minimum.x = minimum.x.max(low.x);
        minimum.y = minimum.y.max(low.y);
        maximum.x = maximum.x.min(high.x);
        maximum.y = maximum.y.min(high.y);
    }
    (minimum, maximum)
}

pub(super) fn body_extent(component: &Component, rotation: f64) -> Vec2 {
    if let Some(geometry) = &component.placement_geometry
        && let CopperShape::Circle { diameter } = geometry.body
    {
        return Vec2::new(diameter / 2.0, diameter / 2.0);
    }
    rotated_extent(component.size, rotation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Problem {
        serde_json::from_value(json!({
            "schema_version":1,
            "board":{"bounds":{"min":{"x":0,"y":0},"max":{"x":20,"y":20}},"layers":[{"id":"top"}]},
            "rules":{"clearance":0.4,"via_diameter":0.8,"via_drill":0.4},
            "components":[
                {"id":"socket","position":{"x":10,"y":10},"size":{"x":4,"y":4},
                 "placement_geometry":{"body":{"kind":"circle","diameter":4},"clearance":0},
                 "pins":[],"body_is_routing_keepout":false},
                {"id":"part","position":{"x":12,"y":12},"size":{"x":1,"y":1},
                 "placement_geometry":{"body":{"kind":"rect","size":{"x":1,"y":1}},"clearance":0},
                 "pins":[],"body_is_routing_keepout":false}
            ]
        }))
        .unwrap()
    }

    fn validate(problem: &Problem) -> Result<(), String> {
        let poses = problem
            .components
            .iter()
            .map(|c| PlacementPose {
                component: c.id.clone(),
                position: c.position,
                rotation_degrees: c.rotation_degrees,
            })
            .collect::<Vec<_>>();
        validate_serialized_placement_poses(problem, &poses)
    }

    fn concave_fixture() -> Problem {
        let mut problem = fixture();
        problem.components[0].size = Vec2::new(6.0, 6.0);
        problem.components[0].placement_geometry = Some(
            serde_json::from_value(json!({
                "body":{"kind":"rect","size":{"x":6,"y":6}},"clearance":0,
                "body_parts":[
                    {"vertices":[{"x":-3,"y":-3},{"x":3,"y":-3},{"x":3,"y":-1}]},
                    {"vertices":[{"x":-3,"y":-3},{"x":3,"y":-1},{"x":-3,"y":-1}]},
                    {"vertices":[{"x":-3,"y":-1},{"x":-1,"y":-1},{"x":-1,"y":3}]},
                    {"vertices":[{"x":-3,"y":-1},{"x":-1,"y":3},{"x":-3,"y":3}]}
                ]
            }))
            .unwrap(),
        );
        problem.components[1].position = Vec2::new(11.0, 11.0);
        problem
    }

    #[test]
    fn collision_intervals_only_skip_actual_rotated_polygon_and_pad_contacts() {
        let mut cases = vec![fixture(), concave_fixture()];
        let mut padded = concave_fixture();
        for (index, c) in padded.components.iter_mut().enumerate() {
            c.placement_geometry.as_mut().unwrap().clearance = 0.35;
            c.placement_geometry.as_mut().unwrap().body_layers =
                vec![if index == 0 { "top" } else { "bottom" }.into()];
            c.pins = serde_json::from_value(json!([{"id":"1","offset":{"x":7,"y":-2},
                "pads":[{"layer":"top","shape":{"kind":"rect","size":{"x":2,"y":1},"rotation_degrees":45}}]}])).unwrap();
        }
        cases.push(padded);
        let mut legacy = fixture();
        for c in &mut legacy.components {
            c.placement_geometry = None;
        }
        cases.push(legacy);
        let mut skipped = 0;
        let mut unskipped_contacts = 0;
        for problem in cases {
            for (aa, ba) in [(0.0, 0.0), (45.0, 45.0), (17.0, 83.0), (90.0, 0.0)] {
                let mut ap = PlacementPose {
                    component: problem.components[0].id.clone(),
                    position: Vec2::ZERO,
                    rotation_degrees: aa,
                };
                let bp = PlacementPose {
                    component: problem.components[1].id.clone(),
                    position: Vec2::new(10.0, 10.0),
                    rotation_degrees: ba,
                };
                let a = prepare_projection_geometry(
                    &problem.components[0],
                    aa,
                    problem.rules.clearance,
                );
                let b = prepare_projection_geometry(
                    &problem.components[1],
                    ba,
                    problem.rules.clearance,
                );
                let certificate = prepare_collision_intervals(&a, &b, bp.position);
                for ix in -40..=40 {
                    ap.position.x = 10.0 + ix as f64 * 0.25;
                    let intervals = certificate.row_intervals(ap.position.x, 0.0, 20.0);
                    for iy in 0..=80 {
                        ap.position.y = iy as f64 * 0.25;
                        let inside = intervals
                            .iter()
                            .any(|&(lo, hi)| lo < ap.position.y && ap.position.y < hi);
                        let exact = overlap_correction(
                            &problem.components[0],
                            &ap,
                            &problem.components[1],
                            &bp,
                            problem.rules.clearance,
                        )
                        .is_some();
                        assert!(
                            !inside || exact,
                            "false collision at {:?}, rotations {aa}/{ba}",
                            ap.position
                        );
                        skipped += usize::from(inside);
                        unskipped_contacts += usize::from(!inside && exact);
                    }
                }
            }
        }
        assert!(skipped > 1000);
        assert!(
            unskipped_contacts > 0,
            "guarded boundaries and unsupported pairs retain point checks"
        );
    }

    #[test]
    fn collision_intervals_preserve_tolerance_layers_and_circle_fallback() {
        let mut problem = fixture();
        let a = prepare_projection_geometry(&problem.components[0], 0.0, 0.0);
        let b = prepare_projection_geometry(&problem.components[1], 0.0, 0.0);
        assert!(prepare_collision_intervals(&a, &b, Vec2::ZERO).is_empty());
        for c in &mut problem.components {
            c.size = Vec2::new(2.0, 2.0);
            c.placement_geometry.as_mut().unwrap().body = CopperShape::Rect {
                size: c.size,
                rotation_degrees: 0.0,
            };
        }
        let build = |p: &Problem| {
            prepare_collision_intervals(
                &prepare_projection_geometry(&p.components[0], 0.0, 0.0),
                &prepare_projection_geometry(&p.components[1], 0.0, 0.0),
                Vec2::ZERO,
            )
        };
        let intervals = build(&problem).row_intervals(0.0, -3.0, 3.0);
        let covered = |y: f64| intervals.iter().any(|&(lo, hi)| lo < y && y < hi);
        assert!(covered(0.0));
        assert!(!covered(2.0 - FEASIBILITY_TOLERANCE_MM));
        assert!(covered(2.0 - 4.0 * FEASIBILITY_TOLERANCE_MM));
        problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body_layers = vec!["top".into()];
        problem.components[1]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body_layers = vec!["bottom".into()];
        assert!(build(&problem).is_empty());
        assert!(
            build(&problem)
                .row_intervals(f64::NAN, -3.0, 3.0)
                .is_empty()
        );
        // Shrinkage may consume a thin or numerically uncertain interval.
        // Reversed bounds must not be swapped into an apparent collision.
        let thin = PreparedCollisionIntervals {
            pairs: vec![vec![(Vec2::new(0.0, 1.0), -1e-8, 1e-8)]],
            coordinate_scale: 0.0,
        };
        assert!(thin.row_intervals(0.0, -1.0, 1.0).is_empty());
        let uncertain = PreparedCollisionIntervals {
            pairs: vec![vec![(Vec2::new(0.0, 1.0), -1.0, 1.0)]],
            coordinate_scale: 1e18,
        };
        assert!(uncertain.row_intervals(0.0, -1.0, 1.0).is_empty());
    }

    #[test]
    fn cached_certificate_preserves_rotated_relaxed_sat_margin() {
        let mut problem = fixture();
        for component in &mut problem.components {
            component.size = Vec2::new(2.0, 2.0);
            component.placement_geometry.as_mut().unwrap().body = CopperShape::Rect {
                size: Vec2::new(2.0, 2.0),
                rotation_degrees: 0.0,
            };
            component.placement_geometry.as_mut().unwrap().clearance = 0.2;
        }
        let first = PlacementPose {
            component: problem.components[0].id.clone(),
            position: Vec2::new(10.0, 10.0),
            rotation_degrees: 45.0,
        };
        let mut second = PlacementPose {
            component: problem.components[1].id.clone(),
            position: Vec2::new(10.0 + 2.0 * 2.0_f64.sqrt() + 0.22, 10.0),
            rotation_degrees: 45.0,
        };
        let a = prepare_projection_geometry(&problem.components[0], 45.0, 0.2);
        let b = prepare_projection_geometry(&problem.components[1], 45.0, 0.2);
        let bounds = prepare_pair_projection_bounds(&a, &b);
        // Raw world AABB gap is 0.22 > margin 0.2, but each tested local
        // separating gap is smaller. Rejecting on the world X gap is unsound.
        assert!(
            overlap_correction(
                &problem.components[0],
                &first,
                &problem.components[1],
                &second,
                0.2
            )
            .is_some()
        );
        assert!(!bounds.certainly_disjoint(first.position, second.position));
        second.position.x += 1.0;
        assert!(bounds.certainly_disjoint(first.position, second.position));
        assert!(
            overlap_correction(
                &problem.components[0],
                &first,
                &problem.components[1],
                &second,
                0.2
            )
            .is_none()
        );
    }

    #[test]
    fn cached_certificates_never_prune_native_parts_pads_layers_or_legacy_contacts() {
        let mut cases = vec![fixture(), concave_fixture()];
        let mut padded = concave_fixture();
        padded.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .clearance = 0.35;
        for (index, component) in padded.components.iter_mut().enumerate() {
            component.placement_geometry.as_mut().unwrap().body_layers =
                vec![if index == 0 { "top" } else { "bottom" }.into()];
            component.pins=serde_json::from_value(json!([
                {"id":"1","offset":{"x":7,"y":-2},"pads":[
                    {"layer":"top","shape":{"kind":"rect","size":{"x":2,"y":1},"rotation_degrees":45}},
                    {"layer":"bottom","shape":{"kind":"circle","diameter":0.8}}
                ]}
            ])).unwrap();
        }
        cases.push(padded);
        let mut legacy = fixture();
        for component in &mut legacy.components {
            component.placement_geometry = None;
        }
        cases.push(legacy);
        let mut pruned = 0;
        let mut contacts = 0;
        for problem in cases {
            for (first_angle, second_angle) in [(0.0, 0.0), (45.0, 45.0), (17.0, 83.0), (90.0, 0.0)]
            {
                let first = PlacementPose {
                    component: problem.components[0].id.clone(),
                    position: Vec2::new(10.0, 10.0),
                    rotation_degrees: first_angle,
                };
                let mut second = PlacementPose {
                    component: problem.components[1].id.clone(),
                    position: Vec2::ZERO,
                    rotation_degrees: second_angle,
                };
                let a = prepare_projection_geometry(
                    &problem.components[0],
                    first_angle,
                    problem.rules.clearance,
                );
                let b = prepare_projection_geometry(
                    &problem.components[1],
                    second_angle,
                    problem.rules.clearance,
                );
                let bounds = prepare_pair_projection_bounds(&a, &b);
                for x in -12..=12 {
                    for y in -12..=12 {
                        second.position = Vec2::new(10.0 + x as f64 * 0.75, 10.0 + y as f64 * 0.75);
                        let overlap = body_overlap_correction(
                            &problem.components[0],
                            &first,
                            &problem.components[1],
                            &second,
                            problem.rules.clearance,
                        )
                        .is_some();
                        contacts += usize::from(overlap);
                        if bounds.certainly_disjoint(first.position, second.position) {
                            pruned += 1;
                            assert!(
                                !overlap,
                                "false certificate at {first_angle}/{second_angle}, offset {x}/{y}"
                            );
                        }
                    }
                }
            }
        }
        assert!(pruned > 1000);
        assert!(contacts > 100);
    }

    #[test]
    fn no_common_axis_and_nonfinite_data_fall_through_to_exact_geometry() {
        let mut problem = fixture();
        for component in &mut problem.components {
            component.placement_geometry.as_mut().unwrap().body =
                CopperShape::Circle { diameter: 2.0 };
        }
        let a = prepare_projection_geometry(&problem.components[0], 0.0, 0.2);
        let b = prepare_projection_geometry(&problem.components[1], 0.0, 0.2);
        let bounds = prepare_pair_projection_bounds(&a, &b);
        assert!(bounds.rows.is_empty());
        assert!(!bounds.certainly_disjoint(Vec2::ZERO, Vec2::new(100.0, 100.0)));
        problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body = CopperShape::Circle { diameter: f64::NAN };
        problem.components[1]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body = CopperShape::Rect {
            size: Vec2::new(2.0, 2.0),
            rotation_degrees: 0.0,
        };
        let bad = prepare_projection_geometry(&problem.components[0], 0.0, 0.2);
        let rect = prepare_projection_geometry(&problem.components[1], 0.0, 0.2);
        let uncertain = prepare_pair_projection_bounds(&bad, &rect);
        assert!(!uncertain.certainly_disjoint(Vec2::ZERO, Vec2::new(100.0, 100.0)));
        let invalid = PairProjectionBounds {
            rows: vec![(Vec2::new(1.0, 0.0), (f64::NEG_INFINITY, 0.0), (0.0, 1.0))],
            margin: 0.2,
        };
        assert!(!invalid.certainly_disjoint(Vec2::ZERO, Vec2::new(100.0, 100.0)));
    }

    #[test]
    fn concave_body_keeps_notch_space_and_rejects_actual_material_overlap() {
        let mut problem = concave_fixture();
        validate(&problem).unwrap();
        let parts = problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body_parts
            .clone();
        problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body_parts
            .clear();
        assert!(validate(&problem).is_err()); // The envelope would fill the notch.
        problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body_parts = parts;
        problem.components[1].position = Vec2::new(8.0, 11.0);
        assert!(validate(&problem).is_err());
        problem.components[0].rotation_degrees = 90.0;
        problem.components[1].position = Vec2::new(9.0, 11.0);
        validate(&problem).unwrap();
        problem.components[1].position = Vec2::new(9.0, 8.0);
        assert!(validate(&problem).is_err());
    }

    #[test]
    fn convex_parts_handle_round_bodies_and_asymmetric_board_extents() {
        let mut problem = concave_fixture();
        problem.components[1]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body = CopperShape::Circle { diameter: 1.0 };
        problem.components[1].position = Vec2::new(9.5, 11.0);
        validate(&problem).unwrap(); // Tangent to the notch's left boundary.
        problem.components[1].position.x = 9.4;
        assert!(validate(&problem).is_err());
        problem.components.pop();
        problem.components[0].position.x = 2.5;
        assert!(validate(&problem).is_err());
        problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .board_overhang = 0.5;
        validate(&problem).unwrap();
        problem.components[0].constraints.region = Some(problem.board.bounds);
        assert!(validate(&problem).is_err());
    }

    #[test]
    fn invalid_convex_parts_cannot_replace_the_declared_body_bounds() {
        let mut problem = concave_fixture();
        problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body_parts[0]
            .vertices = vec![
            Vec2::new(-3.0, -3.0),
            Vec2::new(3.0, -3.0),
            Vec2::new(0.0, -2.0),
            Vec2::new(3.0, -1.0),
        ];
        assert!(problem.check_schema().is_err());
        problem = concave_fixture();
        for part in &mut problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body_parts
        {
            for v in &mut part.vertices {
                v.x += 0.1;
            }
        }
        assert!(problem.check_schema().is_err());
    }

    #[test]
    fn explicit_body_sides_allow_opposite_assembly_but_preserve_pad_contacts() {
        let mut problem = fixture();
        problem
            .board
            .layers
            .push(serde_json::from_value(json!({"id":"bottom"})).unwrap());
        problem.components[1].position = problem.components[0].position;
        for (i, c) in problem.components.iter_mut().enumerate() {
            let layer = if i == 0 { "top" } else { "bottom" };
            c.placement_geometry.as_mut().unwrap().body_layers = vec![layer.into()];
            c.pins=serde_json::from_value(json!([
                {"id":"1","offset":{"x":5,"y":5},"pads":[{"layer":layer,"shape":{"kind":"circle","diameter":0.2}}]}
            ])).unwrap();
        }
        validate(&problem).unwrap();
        problem.components[1].pins[0].pads[0].layer = "top".into();
        assert!(validate(&problem).is_err());
        problem.components[1].pins[0].pads[0].layer = "bottom".into();
        let mut through_pad = problem.components[0].pins[0].pads[0].clone();
        through_pad.layer = "bottom".into();
        problem.components[0].pins[0].pads.push(through_pad);
        assert!(validate(&problem).is_err());
        problem.components[0].pins[0].pads.pop();
        problem.components[1]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body_layers = vec!["top".into()];
        assert!(validate(&problem).is_err());
        problem.components[1]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body_layers = vec!["unknown".into()];
        assert!(problem.check_schema().is_err());
    }

    #[test]
    fn circular_corner_space_is_usable_but_real_overlap_is_rejected() {
        let mut problem = fixture();
        validate(&problem).unwrap();
        problem.components[0].placement_geometry = None;
        assert!(validate(&problem).is_err());
        problem = fixture();
        problem.components[1].position = Vec2::new(11.9, 11.9);
        assert!(validate(&problem).is_err());
        // Rotated rectangles need the circle-to-corner axis as well as face axes.
        problem = fixture();
        problem.components[1].rotation_degrees = 45.0;
        validate(&problem).unwrap();
        problem.components[1].position = Vec2::new(11.7, 11.7);
        assert!(validate(&problem).is_err());
    }

    #[test]
    fn body_overhang_does_not_allow_outside_pads_or_escape_from_regions() {
        let mut problem = fixture();
        problem.components.remove(1);
        let c = &mut problem.components[0];
        c.position = Vec2::new(0.5, 5.0);
        c.rotation_degrees = 45.0;
        c.placement_geometry.as_mut().unwrap().board_overhang = 1.5;
        c.pins=serde_json::from_value(json!([
            {"id":"1","offset":{"x":0,"y":0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.2}}]}
        ])).unwrap();
        validate(&problem).unwrap();
        problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .pad_edge_clearance = 0.41;
        assert!(validate(&problem).is_err());
        problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .pad_edge_clearance = 0.4;
        validate(&problem).unwrap();
        problem.components[0].pins[0].offset = Vec2::new(-1.0, 0.0);
        assert!(validate(&problem).is_err());
        problem.components[0].pins[0].offset = Vec2::ZERO;
        problem.components[0].constraints.region = Some(problem.board.bounds);
        assert!(validate(&problem).is_err());
    }

    #[test]
    fn separate_body_margins_keep_copper_clearance_and_external_pad_collisions() {
        let mut problem = fixture();
        for (i, c) in problem.components.iter_mut().enumerate() {
            c.size = Vec2::new(0.2, 0.2);
            c.position = Vec2::new(5.0 + i as f64, 5.0);
            c.placement_geometry.as_mut().unwrap().body = CopperShape::Circle { diameter: 0.2 };
            c.pins = serde_json::from_value(json!([
                {"id":"1","offset":{"x":if i==0 {0.3} else {-0.3},"y":0},
                 "pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.2}}]}
            ]))
            .unwrap();
        }
        // Bodies have 0.8 mm separation, but the pad gap is only 0.2 mm.
        assert!(validate(&problem).is_err());
        problem.components[1].position.x = 6.21;
        validate(&problem).unwrap();
        problem.components[0].pins[0].offset.x = 1.21;
        assert!(validate(&problem).is_err());
    }

    #[test]
    fn explicit_geometry_rejects_mismatched_bounds_and_invalid_policy() {
        let mut problem = fixture();
        problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .clearance = -0.1;
        assert!(problem.check_schema().is_err());
        problem = fixture();
        problem.components[0].size.x = 3.0;
        assert!(problem.check_schema().is_err());
        problem = fixture();
        problem.components[0]
            .placement_geometry
            .as_mut()
            .unwrap()
            .board_overhang = f64::NAN;
        assert!(problem.check_schema().is_err());
    }

    fn pad_gap_fixture() -> Problem {
        serde_json::from_value(json!({
            "schema_version":1,
            "board":{"bounds":{"min":{"x":-10,"y":-10},"max":{"x":10,"y":10}},"layers":[{"id":"top"},{"id":"bottom"}]},
            "rules":{"clearance":0.2},
            "components":[
                {"id":"A","position":{"x":0,"y":0},"size":{"x":0.2,"y":0.2},
                 "placement_geometry":{"body":{"kind":"rect","size":{"x":0.2,"y":0.2}},"clearance":0,"body_layers":["top"]},
                 "pins":[{"id":"1","offset":{"x":0,"y":0},"pads":[{"layer":"top","shape":{"kind":"rect","size":{"x":1,"y":1}}}]}]},
                {"id":"B","position":{"x":1.4,"y":0},"size":{"x":0.2,"y":0.2},
                 "placement_geometry":{"body":{"kind":"rect","size":{"x":0.2,"y":0.2}},"clearance":0,"body_layers":["top"]},
                 "pins":[{"id":"1","offset":{"x":0,"y":0},"pads":[{"layer":"top","shape":{"kind":"rect","size":{"x":1,"y":1}}}]}]}
            ],"nets":[]
        })).unwrap()
    }

    #[test]
    fn seed_pad_gap_changes_only_copper_pair_margin() {
        let mut problem = pad_gap_fixture();
        let before = serde_json::to_value(&problem).unwrap();
        let mut poses = declared_poses(&problem);
        let contact = |problem: &Problem, poses: &[PlacementPose], extra| {
            overlap_correction_with_pad_gap(
                &problem.components[0],
                &poses[0],
                &problem.components[1],
                &poses[1],
                problem.rules.clearance,
                extra,
            )
        };
        assert_eq!(
            contact(&problem, &poses, 0.0),
            overlap_correction(
                &problem.components[0],
                &poses[0],
                &problem.components[1],
                &poses[1],
                0.2
            )
        );
        assert!(contact(&problem, &poses, 0.0).is_none());
        assert!(contact(&problem, &poses, 0.6).is_some());
        poses[1].position.x = 1.8;
        assert!(contact(&problem, &poses, 0.6).is_none());
        assert_eq!(serde_json::to_value(&problem).unwrap(), before);
        // Removing B's copper leaves only body/body and mixed body/pad pairs.
        // Those never inherit even a very large copper-only reservation.
        problem.components[1].pins.clear();
        poses[1].position.x = 0.7;
        assert!(contact(&problem, &poses, 10.0).is_none());
    }

    #[test]
    fn seed_pad_gap_preserves_opposite_side_overlap_and_through_hole_interaction() {
        let mut problem = pad_gap_fixture();
        problem.components[1]
            .placement_geometry
            .as_mut()
            .unwrap()
            .body_layers = vec!["bottom".into()];
        problem.components[1].pins[0].pads[0].layer = "bottom".into();
        let mut poses = declared_poses(&problem);
        poses[1].position = poses[0].position;
        assert!(
            overlap_correction_with_pad_gap(
                &problem.components[0],
                &poses[0],
                &problem.components[1],
                &poses[1],
                0.2,
                0.6
            )
            .is_none()
        );
        let mut top_pad = problem.components[1].pins[0].pads[0].clone();
        top_pad.layer = "top".into();
        problem.components[1].pins[0].pads.push(top_pad);
        poses[1].position.x = 1.4;
        assert!(
            overlap_correction(
                &problem.components[0],
                &poses[0],
                &problem.components[1],
                &poses[1],
                0.2
            )
            .is_none()
        );
        assert!(
            overlap_correction_with_pad_gap(
                &problem.components[0],
                &poses[0],
                &problem.components[1],
                &poses[1],
                0.2,
                0.6
            )
            .is_some()
        );
    }

    #[test]
    fn reserved_gap_interval_and_disjoint_certificates_match_exact_rotated_pads() {
        let mut problem = pad_gap_fixture();
        problem.components[0].pins[0].offset = Vec2::new(0.3, -0.2);
        let mut certified = 0;
        let mut disjoint = 0;
        for rotation in [0.0, 17.0, 45.0] {
            let mut poses = declared_poses(&problem);
            poses[0].rotation_degrees = rotation;
            poses[1].position = Vec2::new(0.7, 0.4);
            let a = prepare_projection_geometry_with_pad_gap(
                &problem.components[0],
                rotation,
                0.2,
                0.6,
            );
            let b = prepare_projection_geometry_with_pad_gap(&problem.components[1], 0.0, 0.2, 0.6);
            let bounds = prepare_pair_projection_bounds(&a, &b);
            let intervals = prepare_collision_intervals(&a, &b, poses[1].position);
            for ix in -20..=20 {
                poses[0].position.x = ix as f64 * 0.2;
                let ranges = intervals.row_intervals(poses[0].position.x, -4.0, 4.0);
                for iy in -20..=20 {
                    poses[0].position.y = iy as f64 * 0.2;
                    let overlap = overlap_correction_with_pad_gap(
                        &problem.components[0],
                        &poses[0],
                        &problem.components[1],
                        &poses[1],
                        0.2,
                        0.6,
                    )
                    .is_some();
                    if bounds.certainly_disjoint(poses[0].position, poses[1].position) {
                        disjoint += 1;
                        assert!(!overlap);
                    }
                    if ranges
                        .iter()
                        .any(|&(lo, hi)| poses[0].position.y > lo && poses[0].position.y < hi)
                    {
                        certified += 1;
                        assert!(overlap);
                    }
                }
            }
        }
        assert!(certified > 100 && disjoint > 1000);
    }
}

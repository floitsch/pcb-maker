use crate::{Backend, EulerianDensityAlgorithm, FieldAlgorithm, MobilityMask, World};
use pcb_core::{
    AttachmentView, BodyView, ConstraintView, Frame, FrameMetrics, ParticleView, Vec2, VectorView,
};

#[derive(Clone, Copy, Debug)]
pub struct SolverConfig {
    pub timestep: f32,
    pub projection_iterations: usize,
    pub field_width: usize,
    pub field_height: usize,
    pub field_smoothing_steps: usize,
    pub target_density: f32,
    pub field_strength: f32,
    pub maximum_field_step: f32,
    pub trace_tension_strength: f32,
    pub maximum_trace_tension_step: f32,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            timestep: 0.05,
            projection_iterations: 8,
            field_width: 48,
            field_height: 32,
            field_smoothing_steps: 4,
            target_density: 0.15,
            field_strength: 0.8,
            maximum_field_step: 0.2,
            trace_tension_strength: 0.0,
            maximum_trace_tension_step: 0.2,
        }
    }
}

pub struct CpuReferenceBackend {
    field: Box<dyn FieldAlgorithm>,
}

impl CpuReferenceBackend {
    pub fn new() -> Self {
        Self::with_field(EulerianDensityAlgorithm::default())
    }

    pub fn with_field(field: impl FieldAlgorithm + 'static) -> Self {
        Self {
            field: Box::new(field),
        }
    }

    pub fn field_name(&self) -> &'static str {
        self.field.name()
    }
}

impl Default for CpuReferenceBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for CpuReferenceBackend {
    fn name(&self) -> &'static str {
        "cpu-reference"
    }

    fn step(&mut self, world: &mut World, config: &SolverConfig, step: u64) -> Frame {
        self.field
            .configure(world.bounds, config.field_width, config.field_height);
        self.field.clear();
        for index in 0..world.particles.len() {
            self.field.scatter(
                world.particles.position(index),
                world.particles.field_weight[index],
            );
        }
        self.field
            .solve(config.target_density, config.field_smoothing_steps);

        world.particles.snapshot_previous();
        world.bodies.snapshot_previous();
        let mut vectors = Vec::new();
        let mut tension_deltas = vec![Vec2::ZERO; world.particles.len()];
        let apply_tension = config.trace_tension_strength.is_finite()
            && config.trace_tension_strength > 0.0
            && config.maximum_trace_tension_step.is_finite()
            && config.maximum_trace_tension_step > 0.0;
        if apply_tension {
            for edge in 0..world.trace_tension.len() {
                let first = world.trace_tension.first[edge] as usize;
                let second = world.trace_tension.second[edge] as usize;
                let difference = world.particles.position(second) - world.particles.position(first);
                let distance = difference.length();
                if distance <= 1.0e-8 {
                    continue;
                }
                let delta = difference / distance
                    * (config.trace_tension_strength
                        * world.trace_tension.weight[edge]
                        * config.timestep);
                tension_deltas[first] += delta;
                tension_deltas[second] -= delta;
            }
            for (index, raw) in tension_deltas.into_iter().enumerate() {
                if world.particles.inverse_mass[index] == 0.0 {
                    continue;
                }
                let origin = world.particles.position(index);
                let delta = clamp_length(
                    world.particles.mobility[index]
                        .project(raw * world.particles.inverse_mass[index]),
                    config.maximum_trace_tension_step,
                );
                world.particles.set_position(index, origin + delta);
                if delta.length_squared() > 0.0 {
                    vectors.push(VectorView {
                        particle: world.particles.stable_id[index],
                        origin,
                        vector: delta,
                        source: "trace_tension".into(),
                    });
                }
            }
        }
        for index in 0..world.particles.len() {
            if world.particles.inverse_mass[index] == 0.0 {
                continue;
            }
            let origin = world.particles.position(index);
            let raw = -self.field.gradient(origin) * config.field_strength * config.timestep;
            let delta = clamp_length(
                world.particles.mobility[index].project(raw),
                config.maximum_field_step,
            );
            world.particles.set_position(index, origin + delta);
            vectors.push(VectorView {
                particle: world.particles.stable_id[index],
                origin,
                vector: delta,
                source: "density_field".into(),
            });
        }

        let mut projections = 0_u64;
        let mut scalar_rows = 0_u64;
        for _ in 0..config.projection_iterations {
            for index in 0..world.segment_clearance.len() {
                project_segment_clearance(world, index);
                projections += 1;
                scalar_rows += 1;
            }
            for index in 0..world.segment_body_clearance.len() {
                project_segment_body_clearance(world, index);
                projections += 1;
                scalar_rows += 1;
            }
            for index in 0..world.body_body_clearance.len() {
                project_body_body_clearance(world, index);
                projections += 1;
                scalar_rows += 1;
            }
            for index in 0..world.maximum_distance.len() {
                project_distance(
                    world,
                    world.maximum_distance.first[index] as usize,
                    world.maximum_distance.second[index] as usize,
                    world.maximum_distance.maximum[index],
                    true,
                );
                projections += 1;
                scalar_rows += 1;
            }
            // Shape constraints are the final positional authority in a
            // projection sweep. A trace sampling constraint may deform its
            // own chain, but it must not leave a component stretched.
            for index in 0..world.equality_distance.len() {
                project_distance(
                    world,
                    world.equality_distance.first[index] as usize,
                    world.equality_distance.second[index] as usize,
                    world.equality_distance.distance[index],
                    false,
                );
                projections += 1;
                scalar_rows += 1;
            }
            for index in 0..world.attachments.len() {
                project_attachment(world, index);
                projections += 1;
                scalar_rows += 2;
            }
            for index in 0..world.bodies.len() {
                clamp_body_to_bounds(world, index);
            }
            for index in 0..world.particles.len() {
                let clamped = world.bounds.clamp(world.particles.position(index));
                world.particles.set_position(index, clamped);
            }
        }

        frame(
            world,
            self.field.as_ref(),
            vectors,
            step,
            projections,
            scalar_rows,
            if apply_tension {
                world.trace_tension.len() as u64
            } else {
                0
            },
        )
    }
}

fn project_segment_clearance(world: &mut World, constraint: usize) {
    let indices = [
        world.segment_clearance.first_start[constraint] as usize,
        world.segment_clearance.first_end[constraint] as usize,
        world.segment_clearance.second_start[constraint] as usize,
        world.segment_clearance.second_end[constraint] as usize,
    ];
    let positions = indices.map(|index| world.particles.position(index));
    let (first_parameter, second_parameter, first_point, second_point) =
        closest_segment_points(positions[0], positions[1], positions[2], positions[3]);
    let delta = second_point - first_point;
    let distance = delta.length();
    let minimum = world.segment_clearance.minimum[constraint];
    if distance >= minimum {
        return;
    }
    // Copper-clearance obligations never authorize a centerline crossing, but
    // retain a deterministic fallback so a nearly coincident numeric witness
    // cannot inject NaNs into the solver.
    let normal = if distance > 1.0e-8 {
        delta / distance
    } else {
        let first_direction = positions[1] - positions[0];
        let second_direction = positions[3] - positions[2];
        let direction = if first_direction.length_squared() >= second_direction.length_squared() {
            first_direction
        } else {
            second_direction
        };
        if direction.length_squared() > 1.0e-8 {
            perpendicular(direction) / direction.length()
        } else {
            Vec2::new(0.0, 1.0)
        }
    };
    let coefficients = [
        -(1.0 - first_parameter),
        -first_parameter,
        1.0 - second_parameter,
        second_parameter,
    ];
    let gradients = coefficients.map(|coefficient| normal * coefficient);
    let projected = [0, 1, 2, 3]
        .map(|position| world.particles.mobility[indices[position]].project(gradients[position]));
    let denominator = [0, 1, 2, 3]
        .into_iter()
        .map(|position| {
            world.particles.inverse_mass[indices[position]] * projected[position].length_squared()
        })
        .sum::<f32>();
    if denominator <= 1.0e-8 {
        return;
    }
    let lambda = (distance - minimum) / denominator;
    for position in 0..4 {
        let index = indices[position];
        let correction = projected[position] * (-world.particles.inverse_mass[index] * lambda);
        world
            .particles
            .set_position(index, positions[position] + correction);
    }
}

#[derive(Clone, Copy, Debug)]
struct SegmentBodyProximity {
    segment_parameter: f32,
    point_on_body: Vec2,
    normal: Vec2,
    distance: f32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BodyBodyProximity {
    pub point_on_first: Vec2,
    pub point_on_second: Vec2,
    /// Unit vector from the second body toward the first body.
    pub normal: Vec2,
    /// Signed surface distance. Overlap is negative penetration depth.
    pub distance: f32,
}

fn project_body_body_clearance(world: &mut World, constraint: usize) {
    let first = world.body_body_clearance.first[constraint] as usize;
    let second = world.body_body_clearance.second[constraint] as usize;
    let proximity = body_body_proximity(world, first, second);
    let minimum = world.body_body_clearance.minimum[constraint];
    if proximity.distance >= minimum {
        return;
    }

    let first_position = world.bodies.position(first);
    let second_position = world.bodies.position(second);
    let first_gradient = world.bodies.mobility[first].project(proximity.normal);
    let second_gradient = world.bodies.mobility[second].project(-proximity.normal);
    let first_radius = proximity.point_on_first - first_position;
    let second_radius = proximity.point_on_second - second_position;
    let first_angular_gradient = perpendicular(first_radius).dot(proximity.normal);
    let second_angular_gradient = -perpendicular(second_radius).dot(proximity.normal);
    let denominator = world.bodies.inverse_mass[first] * first_gradient.length_squared()
        + world.bodies.inverse_mass[second] * second_gradient.length_squared()
        + if world.bodies.rotate[first] {
            world.bodies.inverse_inertia[first] * first_angular_gradient * first_angular_gradient
        } else {
            0.0
        }
        + if world.bodies.rotate[second] {
            world.bodies.inverse_inertia[second] * second_angular_gradient * second_angular_gradient
        } else {
            0.0
        };
    if denominator <= 1.0e-8 {
        return;
    }
    let lambda = (proximity.distance - minimum) / denominator;
    world.bodies.set_position(
        first,
        first_position + first_gradient * (-world.bodies.inverse_mass[first] * lambda),
    );
    world.bodies.set_position(
        second,
        second_position + second_gradient * (-world.bodies.inverse_mass[second] * lambda),
    );
    if world.bodies.rotate[first] {
        world.bodies.angle_radians[first] +=
            -world.bodies.inverse_inertia[first] * lambda * first_angular_gradient;
    }
    if world.bodies.rotate[second] {
        world.bodies.angle_radians[second] +=
            -world.bodies.inverse_inertia[second] * lambda * second_angular_gradient;
    }
}

pub(crate) fn body_body_proximity(world: &World, first: usize, second: usize) -> BodyBodyProximity {
    let positions = [world.bodies.position(first), world.bodies.position(second)];
    let angles = [
        world.bodies.angle_radians[first],
        world.bodies.angle_radians[second],
    ];
    let halves = [
        world.bodies.half_size(first),
        world.bodies.half_size(second),
    ];
    let local_corners = |half: Vec2| {
        [
            Vec2::new(-half.x, -half.y),
            Vec2::new(half.x, -half.y),
            Vec2::new(half.x, half.y),
            Vec2::new(-half.x, half.y),
        ]
    };
    let corners = [0, 1].map(|body| {
        local_corners(halves[body]).map(|corner| positions[body] + rotate(corner, angles[body]))
    });

    let axes = [
        rotate(Vec2::new(1.0, 0.0), angles[0]),
        rotate(Vec2::new(0.0, 1.0), angles[0]),
        rotate(Vec2::new(1.0, 0.0), angles[1]),
        rotate(Vec2::new(0.0, 1.0), angles[1]),
    ];
    let mut overlapping = true;
    let mut minimum_translation = f32::INFINITY;
    let mut overlap_normal = Vec2::new(1.0, 0.0);
    for axis in axes {
        let projection = |body: usize| {
            corners[body].iter().map(|corner| corner.dot(axis)).fold(
                (f32::INFINITY, f32::NEG_INFINITY),
                |(minimum, maximum), value| (minimum.min(value), maximum.max(value)),
            )
        };
        let (first_minimum, first_maximum) = projection(0);
        let (second_minimum, second_maximum) = projection(1);
        if first_maximum < second_minimum || second_maximum < first_minimum {
            overlapping = false;
            continue;
        }
        let move_first_positive = second_maximum - first_minimum;
        let move_first_negative = first_maximum - second_minimum;
        let (translation, normal) = if move_first_positive < move_first_negative {
            (move_first_positive, axis)
        } else if move_first_negative < move_first_positive {
            (move_first_negative, -axis)
        } else if (positions[0] - positions[1]).dot(axis) >= 0.0 {
            (move_first_positive, axis)
        } else {
            (move_first_negative, -axis)
        };
        if translation < minimum_translation {
            minimum_translation = translation;
            overlap_normal = normal;
        }
    }

    if overlapping {
        let support = |body: usize, direction: Vec2| {
            *corners[body]
                .iter()
                .min_by(|left, right| left.dot(direction).total_cmp(&right.dot(direction)))
                .expect("oriented body has four corners")
        };
        return BodyBodyProximity {
            point_on_first: support(0, overlap_normal),
            point_on_second: support(1, -overlap_normal),
            normal: overlap_normal,
            distance: -minimum_translation,
        };
    }

    let mut best = None::<(f32, Vec2, Vec2)>;
    for first_edge in 0..4 {
        for second_edge in 0..4 {
            let (_, _, point_on_first, point_on_second) = closest_segment_points(
                corners[0][first_edge],
                corners[0][(first_edge + 1) % 4],
                corners[1][second_edge],
                corners[1][(second_edge + 1) % 4],
            );
            let distance_squared = point_on_first.distance(point_on_second).powi(2);
            if best
                .as_ref()
                .is_none_or(|(best_distance, _, _)| distance_squared < *best_distance)
            {
                best = Some((distance_squared, point_on_first, point_on_second));
            }
        }
    }
    let (distance_squared, point_on_first, point_on_second) =
        best.expect("two oriented bodies have edge pairs");
    let distance = distance_squared.sqrt();
    BodyBodyProximity {
        point_on_first,
        point_on_second,
        normal: (point_on_first - point_on_second)
            .normalized_or((positions[0] - positions[1]).normalized_or(Vec2::new(1.0, 0.0))),
        distance,
    }
}

fn project_segment_body_clearance(world: &mut World, constraint: usize) {
    let start = world.segment_body_clearance.start[constraint] as usize;
    let end = world.segment_body_clearance.end[constraint] as usize;
    let body = world.segment_body_clearance.body[constraint] as usize;
    let start_position = world.particles.position(start);
    let end_position = world.particles.position(end);
    let body_position = world.bodies.position(body);
    let proximity = segment_body_proximity(world, start_position, end_position, body);
    let minimum = world.segment_body_clearance.minimum[constraint];
    if proximity.distance >= minimum {
        return;
    }

    let particle_gradients = [
        proximity.normal * (1.0 - proximity.segment_parameter),
        proximity.normal * proximity.segment_parameter,
    ];
    let particle_indices = [start, end];
    let projected_particles = [0, 1].map(|position| {
        world.particles.mobility[particle_indices[position]].project(particle_gradients[position])
    });
    let body_gradient = world.bodies.mobility[body].project(-proximity.normal);
    let body_radius = proximity.point_on_body - body_position;
    let angular_gradient = -perpendicular(body_radius).dot(proximity.normal);
    let denominator = [0, 1]
        .into_iter()
        .map(|position| {
            world.particles.inverse_mass[particle_indices[position]]
                * projected_particles[position].length_squared()
        })
        .sum::<f32>()
        + world.bodies.inverse_mass[body] * body_gradient.length_squared()
        + if world.bodies.rotate[body] {
            world.bodies.inverse_inertia[body] * angular_gradient * angular_gradient
        } else {
            0.0
        };
    if denominator <= 1.0e-8 {
        return;
    }
    let lambda = (proximity.distance - minimum) / denominator;
    for position in 0..2 {
        let particle = particle_indices[position];
        let correction =
            projected_particles[position] * (-world.particles.inverse_mass[particle] * lambda);
        let original = if position == 0 {
            start_position
        } else {
            end_position
        };
        world
            .particles
            .set_position(particle, original + correction);
    }
    world.bodies.set_position(
        body,
        body_position + body_gradient * (-world.bodies.inverse_mass[body] * lambda),
    );
    if world.bodies.rotate[body] {
        world.bodies.angle_radians[body] +=
            -world.bodies.inverse_inertia[body] * lambda * angular_gradient;
    }
}

fn segment_body_proximity(
    world: &World,
    segment_start: Vec2,
    segment_end: Vec2,
    body: usize,
) -> SegmentBodyProximity {
    let body_position = world.bodies.position(body);
    let angle = world.bodies.angle_radians[body];
    let half = world.bodies.half_size(body);
    let local_corners = [
        Vec2::new(-half.x, -half.y),
        Vec2::new(half.x, -half.y),
        Vec2::new(half.x, half.y),
        Vec2::new(-half.x, half.y),
    ];
    let corners = local_corners.map(|corner| body_position + rotate(corner, angle));
    let mut best = None::<(f32, Vec2, Vec2)>;
    for edge in 0..4 {
        let (_, _, point_on_segment, point_on_body) = closest_segment_points(
            segment_start,
            segment_end,
            corners[edge],
            corners[(edge + 1) % 4],
        );
        let distance_squared = point_on_segment.distance(point_on_body).powi(2);
        if best
            .as_ref()
            .is_none_or(|(best_distance, _, _)| distance_squared < *best_distance)
        {
            best = Some((distance_squared, point_on_segment, point_on_body));
        }
        if distance_squared <= 1.0e-16 {
            best = Some((distance_squared, point_on_segment, point_on_body));
            break;
        }
    }

    let segment_direction = segment_end - segment_start;
    let segment_length_squared = segment_direction.length_squared();
    let parameter_for = |point: Vec2| {
        if segment_length_squared <= 1.0e-12 {
            0.0
        } else {
            ((point - segment_start).dot(segment_direction) / segment_length_squared)
                .clamp(0.0, 1.0)
        }
    };
    let to_local = |point: Vec2| rotate(point - body_position, -angle);
    let start_local = to_local(segment_start);
    let end_local = to_local(segment_end);
    let inside = |point: Vec2| point.x.abs() <= half.x && point.y.abs() <= half.y;
    let intersects_or_inside = best
        .as_ref()
        .is_some_and(|(distance_squared, _, _)| *distance_squared <= 1.0e-16)
        || inside(start_local)
        || inside(end_local);

    if intersects_or_inside {
        let midpoint = (segment_start + segment_end) * 0.5;
        let midpoint_local = to_local(midpoint);
        let x_face = half.x - midpoint_local.x.abs().min(half.x);
        let y_face = half.y - midpoint_local.y.abs().min(half.y);
        let local_normal = if x_face < y_face {
            Vec2::new(if midpoint_local.x < 0.0 { -1.0 } else { 1.0 }, 0.0)
        } else if y_face < x_face || midpoint_local.y.abs() > 1.0e-8 {
            Vec2::new(0.0, if midpoint_local.y < 0.0 { -1.0 } else { 1.0 })
        } else {
            let lateral = perpendicular(segment_direction).normalized_or(Vec2::new(0.0, 1.0));
            return SegmentBodyProximity {
                segment_parameter: 0.5,
                point_on_body: midpoint,
                normal: lateral,
                distance: 0.0,
            };
        };
        let normal = rotate(local_normal, angle);
        let face_local = if local_normal.x != 0.0 {
            Vec2::new(
                local_normal.x * half.x,
                midpoint_local.y.clamp(-half.y, half.y),
            )
        } else {
            Vec2::new(
                midpoint_local.x.clamp(-half.x, half.x),
                local_normal.y * half.y,
            )
        };
        return SegmentBodyProximity {
            segment_parameter: 0.5,
            point_on_body: body_position + rotate(face_local, angle),
            normal,
            distance: 0.0,
        };
    }

    let (distance_squared, point_on_segment, point_on_body) =
        best.expect("oriented body always has four edges");
    let distance = distance_squared.sqrt();
    SegmentBodyProximity {
        segment_parameter: parameter_for(point_on_segment),
        point_on_body,
        normal: (point_on_segment - point_on_body).normalized_or(Vec2::new(0.0, 1.0)),
        distance,
    }
}

fn closest_segment_points(
    first_start: Vec2,
    first_end: Vec2,
    second_start: Vec2,
    second_end: Vec2,
) -> (f32, f32, Vec2, Vec2) {
    let first = first_end - first_start;
    let second = second_end - second_start;
    let offset = first_start - second_start;
    let first_length = first.dot(first);
    let second_length = second.dot(second);
    let second_offset = second.dot(offset);
    let epsilon = 1.0e-12;
    let (mut first_parameter, mut second_parameter) =
        if first_length <= epsilon && second_length <= epsilon {
            (0.0, 0.0)
        } else if first_length <= epsilon {
            (0.0, (second_offset / second_length).clamp(0.0, 1.0))
        } else {
            let first_offset = first.dot(offset);
            if second_length <= epsilon {
                ((-first_offset / first_length).clamp(0.0, 1.0), 0.0)
            } else {
                let product = first.dot(second);
                let denominator = first_length * second_length - product * product;
                let first_parameter = if denominator.abs() > epsilon {
                    ((product * second_offset - first_offset * second_length) / denominator)
                        .clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let candidate = (product * first_parameter + second_offset) / second_length;
                if candidate < 0.0 {
                    ((-first_offset / first_length).clamp(0.0, 1.0), 0.0)
                } else if candidate > 1.0 {
                    (
                        ((product - first_offset) / first_length).clamp(0.0, 1.0),
                        1.0,
                    )
                } else {
                    (first_parameter, candidate)
                }
            }
        };
    first_parameter = first_parameter.clamp(0.0, 1.0);
    second_parameter = second_parameter.clamp(0.0, 1.0);
    // In f32, almost-parallel long segments can lose the small denominator
    // above to cancellation. Endpoint-to-segment candidates make the result
    // robust in that regime while the interior candidate still detects true
    // crossings.
    let project = |point: Vec2, start: Vec2, direction: Vec2, length: f32| {
        if length <= epsilon {
            0.0
        } else {
            ((point - start).dot(direction) / length).clamp(0.0, 1.0)
        }
    };
    let candidates = [
        (first_parameter, second_parameter),
        (
            0.0,
            project(first_start, second_start, second, second_length),
        ),
        (1.0, project(first_end, second_start, second, second_length)),
        (project(second_start, first_start, first, first_length), 0.0),
        (project(second_end, first_start, first, first_length), 1.0),
    ];
    let &(first_parameter, second_parameter) = candidates
        .iter()
        .min_by(|(left_first, left_second), (right_first, right_second)| {
            let left_delta =
                (first_start + first * *left_first) - (second_start + second * *left_second);
            let right_delta =
                (first_start + first * *right_first) - (second_start + second * *right_second);
            left_delta
                .length_squared()
                .total_cmp(&right_delta.length_squared())
                .then_with(|| left_first.total_cmp(right_first))
                .then_with(|| left_second.total_cmp(right_second))
        })
        .expect("fixed closest-segment candidate set is non-empty");
    (
        first_parameter,
        second_parameter,
        first_start + first * first_parameter,
        second_start + second * second_parameter,
    )
}

fn project_attachment(world: &mut World, attachment: usize) {
    project_attachment_axis(world, attachment, Vec2::new(1.0, 0.0));
    project_attachment_axis(world, attachment, Vec2::new(0.0, 1.0));
}

fn project_attachment_axis(world: &mut World, attachment: usize, axis: Vec2) {
    let particle = world.attachments.particle[attachment] as usize;
    let body = world.attachments.body[attachment] as usize;
    let particle_position = world.particles.position(particle);
    let body_position = world.bodies.position(body);
    let local = world.attachments.local(attachment);
    let rotated = rotate(local, world.bodies.angle_radians[body]);
    let residual = (particle_position - body_position - rotated).dot(axis);
    if residual.abs() <= 1.0e-8 {
        return;
    }
    let particle_gradient = world.particles.mobility[particle].project(axis);
    let body_gradient = world.bodies.mobility[body].project(axis);
    let angular_gradient = -perpendicular(rotated).dot(axis);
    let particle_weight =
        world.particles.inverse_mass[particle] * particle_gradient.length_squared();
    let body_weight = world.bodies.inverse_mass[body] * body_gradient.length_squared();
    let angular_weight = if world.bodies.rotate[body] {
        world.bodies.inverse_inertia[body] * angular_gradient * angular_gradient
    } else {
        0.0
    };
    let denominator = particle_weight + body_weight + angular_weight;
    if denominator <= 1.0e-8 {
        return;
    }
    let lambda = residual / denominator;
    world.particles.set_position(
        particle,
        particle_position - particle_gradient * (world.particles.inverse_mass[particle] * lambda),
    );
    world.bodies.set_position(
        body,
        body_position + body_gradient * (world.bodies.inverse_mass[body] * lambda),
    );
    if world.bodies.rotate[body] {
        world.bodies.angle_radians[body] -=
            world.bodies.inverse_inertia[body] * angular_gradient * lambda;
    }
}

fn clamp_body_to_bounds(world: &mut World, body: usize) {
    let angle = world.bodies.angle_radians[body];
    let half = world.bodies.half_size(body);
    let cosine = angle.cos().abs();
    let sine = angle.sin().abs();
    let extent = Vec2::new(
        cosine * half.x + sine * half.y,
        sine * half.x + cosine * half.y,
    );
    let position = world.bodies.position(body);
    let minimum_x = world.bounds.min.x + extent.x;
    let maximum_x = world.bounds.max.x - extent.x;
    let minimum_y = world.bounds.min.y + extent.y;
    let maximum_y = world.bounds.max.y - extent.y;
    // Oversized experimental geometry must not panic inside `f32::clamp`.
    // Leave an impossible axis unchanged so semantic validation can report it.
    let clamped = Vec2::new(
        if minimum_x <= maximum_x {
            position.x.clamp(minimum_x, maximum_x)
        } else {
            position.x
        },
        if minimum_y <= maximum_y {
            position.y.clamp(minimum_y, maximum_y)
        } else {
            position.y
        },
    );
    let correction = world.bodies.mobility[body].project(clamped - position);
    world.bodies.set_position(body, position + correction);
}

fn project_distance(world: &mut World, first: usize, second: usize, target: f32, upper_only: bool) {
    let first_position = world.particles.position(first);
    let second_position = world.particles.position(second);
    let delta = second_position - first_position;
    let distance = delta.length();
    if distance <= 1.0e-8 || (upper_only && distance <= target) {
        return;
    }
    let normal = delta / distance;
    let first_gradient = world.particles.mobility[first].project(normal);
    let second_gradient = world.particles.mobility[second].project(normal);
    let first_weight = world.particles.inverse_mass[first] * first_gradient.length_squared();
    let second_weight = world.particles.inverse_mass[second] * second_gradient.length_squared();
    let denominator = first_weight + second_weight;
    if denominator <= 1.0e-8 {
        return;
    }
    let lambda = (distance - target) / denominator;
    let first_correction = first_gradient * (world.particles.inverse_mass[first] * lambda);
    let second_correction = second_gradient * (world.particles.inverse_mass[second] * lambda);
    world
        .particles
        .set_position(first, first_position + first_correction);
    world
        .particles
        .set_position(second, second_position - second_correction);
}

fn frame(
    world: &World,
    field: &dyn FieldAlgorithm,
    vectors: Vec<VectorView>,
    step: u64,
    projections: u64,
    scalar_rows: u64,
    tension_edges: u64,
) -> Frame {
    let particles = (0..world.particles.len())
        .map(|index| ParticleView {
            id: world.particles.stable_id[index],
            label: world.particles.label[index].clone(),
            position: world.particles.position(index),
            role: world.particles.role[index],
            field_weight: world.particles.field_weight[index],
            inverse_mass: world.particles.inverse_mass[index],
        })
        .collect();
    let bodies = (0..world.bodies.len())
        .map(|index| BodyView {
            id: world.bodies.stable_id[index],
            label: world.bodies.label[index].clone(),
            position: world.bodies.position(index),
            angle_radians: world.bodies.angle_radians[index],
            half_size: world.bodies.half_size(index),
            inverse_mass: world.bodies.inverse_mass[index],
            inverse_inertia: world.bodies.inverse_inertia[index],
        })
        .collect();
    let mut constraints = Vec::new();
    let mut max_constraint_residual = 0.0_f32;
    for index in 0..world.equality_distance.len() {
        let residual = (world
            .particles
            .position(world.equality_distance.first[index] as usize)
            .distance(
                world
                    .particles
                    .position(world.equality_distance.second[index] as usize),
            )
            - world.equality_distance.distance[index])
            .abs();
        max_constraint_residual = max_constraint_residual.max(residual);
        constraints.push(ConstraintView {
            label: world.equality_distance.label[index].clone(),
            family: "equality_distance".into(),
            first: world.particles.stable_id[world.equality_distance.first[index] as usize],
            second: world.particles.stable_id[world.equality_distance.second[index] as usize],
            residual,
        });
    }
    for index in 0..world.maximum_distance.len() {
        let residual = (world
            .particles
            .position(world.maximum_distance.first[index] as usize)
            .distance(
                world
                    .particles
                    .position(world.maximum_distance.second[index] as usize),
            )
            - world.maximum_distance.maximum[index])
            .max(0.0);
        max_constraint_residual = max_constraint_residual.max(residual);
        constraints.push(ConstraintView {
            label: world.maximum_distance.label[index].clone(),
            family: "maximum_distance".into(),
            first: world.particles.stable_id[world.maximum_distance.first[index] as usize],
            second: world.particles.stable_id[world.maximum_distance.second[index] as usize],
            residual,
        });
    }
    for index in 0..world.segment_clearance.len() {
        let first_start = world.segment_clearance.first_start[index] as usize;
        let first_end = world.segment_clearance.first_end[index] as usize;
        let second_start = world.segment_clearance.second_start[index] as usize;
        let second_end = world.segment_clearance.second_end[index] as usize;
        let (_, _, first_point, second_point) = closest_segment_points(
            world.particles.position(first_start),
            world.particles.position(first_end),
            world.particles.position(second_start),
            world.particles.position(second_end),
        );
        let residual =
            (world.segment_clearance.minimum[index] - first_point.distance(second_point)).max(0.0);
        max_constraint_residual = max_constraint_residual.max(residual);
        constraints.push(ConstraintView {
            label: world.segment_clearance.label[index].clone(),
            family: "segment_clearance".into(),
            first: world.particles.stable_id[first_start],
            second: world.particles.stable_id[second_start],
            residual,
        });
    }
    for index in 0..world.segment_body_clearance.len() {
        let start = world.segment_body_clearance.start[index] as usize;
        let end = world.segment_body_clearance.end[index] as usize;
        let body = world.segment_body_clearance.body[index] as usize;
        let proximity = segment_body_proximity(
            world,
            world.particles.position(start),
            world.particles.position(end),
            body,
        );
        let residual = (world.segment_body_clearance.minimum[index] - proximity.distance).max(0.0);
        max_constraint_residual = max_constraint_residual.max(residual);
        constraints.push(ConstraintView {
            label: world.segment_body_clearance.label[index].clone(),
            family: "segment_body_clearance".into(),
            first: world.particles.stable_id[start],
            second: world.bodies.stable_id[body],
            residual,
        });
    }
    for index in 0..world.body_body_clearance.len() {
        let first = world.body_body_clearance.first[index] as usize;
        let second = world.body_body_clearance.second[index] as usize;
        let proximity = body_body_proximity(world, first, second);
        let residual = (world.body_body_clearance.minimum[index] - proximity.distance).max(0.0);
        max_constraint_residual = max_constraint_residual.max(residual);
        constraints.push(ConstraintView {
            label: world.body_body_clearance.label[index].clone(),
            family: "body_body_clearance".into(),
            first: world.bodies.stable_id[first],
            second: world.bodies.stable_id[second],
            residual,
        });
    }
    let attachments = (0..world.attachments.len())
        .map(|index| {
            let particle = world.attachments.particle[index] as usize;
            let body = world.attachments.body[index] as usize;
            let target = world.bodies.position(body)
                + rotate(
                    world.attachments.local(index),
                    world.bodies.angle_radians[body],
                );
            let residual = world.particles.position(particle).distance(target);
            max_constraint_residual = max_constraint_residual.max(residual);
            AttachmentView {
                label: world.attachments.label[index].clone(),
                particle: world.particles.stable_id[particle],
                body: world.bodies.stable_id[body],
                target,
                residual,
            }
        })
        .collect();
    let max_displacement = (0..world.particles.len())
        .map(|index| {
            world
                .particles
                .position(index)
                .distance(world.particles.previous(index))
        })
        .fold(0.0, f32::max);
    let max_body_displacement = (0..world.bodies.len())
        .map(|index| {
            world
                .bodies
                .position(index)
                .distance(world.bodies.previous(index))
        })
        .fold(0.0, f32::max);
    let max_body_rotation_radians = (0..world.bodies.len())
        .map(|index| {
            (world.bodies.angle_radians[index] - world.bodies.previous_angle_radians[index]).abs()
        })
        .fold(0.0, f32::max);
    Frame {
        step,
        bounds: world.bounds,
        particles,
        bodies,
        constraints,
        attachments,
        vectors,
        field: field.view(),
        metrics: FrameMetrics {
            max_constraint_residual,
            max_field_pressure: field.max_pressure(),
            max_displacement,
            max_body_displacement,
            max_body_rotation_radians,
            constraint_projections: projections,
            constraint_scalar_rows: scalar_rows,
            trace_tension_edges: tension_edges,
            field_cells: field.cell_count() as u64,
        },
    }
}

fn rotate(value: Vec2, angle: f32) -> Vec2 {
    let cosine = angle.cos();
    let sine = angle.sin();
    Vec2::new(
        value.x * cosine - value.y * sine,
        value.x * sine + value.y * cosine,
    )
}

fn perpendicular(value: Vec2) -> Vec2 {
    Vec2::new(-value.y, value.x)
}

fn clamp_length(value: Vec2, maximum: f32) -> Vec2 {
    let length = value.length();
    if length > maximum && length > 0.0 {
        value * (maximum / length)
    } else {
        value
    }
}

#[allow(dead_code)]
fn effective_mass(inverse_mass: f32, mobility: MobilityMask, normal: Vec2) -> f32 {
    inverse_mass * mobility.project(normal).length_squared()
}

use crate::{BodyInit, MobilityMask, ParticleInit, Segment, World};
use pcb_core::{
    Board, ComponentKind, ContinuousComponentPose, ContinuousConnection, ContinuousSolution,
    ParticleRole, Vec2,
};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RectangleRepresentation {
    CornerDistanceParticles,
    AnalyticRigidBodyAttachments,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TwoTerminalRepresentation {
    Unspecified,
    ExperimentalEndpointParticles,
    AnalyticRigidBodyAttachments,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceSamplingPolicy {
    PreserveSeedVertices,
    SubdivideToMaximumSpacing,
}

#[derive(Clone, Copy, Debug)]
pub struct CompilePolicy {
    pub rectangle_representation: RectangleRepresentation,
    pub two_terminal_representation: TwoTerminalRepresentation,
    pub trace_sampling: TraceSamplingPolicy,
    pub maximum_trace_spacing: f32,
    pub maximum_trace_particles_per_connection: usize,
}

impl Default for CompilePolicy {
    fn default() -> Self {
        Self {
            rectangle_representation: RectangleRepresentation::CornerDistanceParticles,
            two_terminal_representation: TwoTerminalRepresentation::Unspecified,
            trace_sampling: TraceSamplingPolicy::SubdivideToMaximumSpacing,
            maximum_trace_spacing: 2.0,
            maximum_trace_particles_per_connection: 1_000_000,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CompiledWorld {
    pub world: World,
    pub terminal_particles: HashMap<(String, String), u32>,
    pub body_handles: HashMap<String, u32>,
    component_pose_bindings: Vec<ComponentPoseBinding>,
    connection_bindings: Vec<ConnectionBinding>,
}

#[derive(Clone, Debug)]
enum ComponentPoseBinding {
    Declared {
        id: String,
        position: Vec2,
        rotation_radians: f32,
    },
    ParticleShape {
        id: String,
        particles: Vec<u32>,
        local_positions: Vec<Vec2>,
    },
    Body {
        id: String,
        body: u32,
    },
}

#[derive(Clone, Debug)]
struct ConnectionBinding {
    id: String,
    layer: usize,
    width: f32,
    particles: Vec<u32>,
}

impl CompiledWorld {
    pub fn solution(&self) -> Result<ContinuousSolution, String> {
        let components = self
            .component_pose_bindings
            .iter()
            .map(|binding| match binding {
                ComponentPoseBinding::Declared {
                    id,
                    position,
                    rotation_radians,
                } => Ok(ContinuousComponentPose {
                    id: id.clone(),
                    position: *position,
                    rotation_radians: *rotation_radians,
                }),
                ComponentPoseBinding::ParticleShape {
                    id,
                    particles,
                    local_positions,
                } => particle_shape_pose(&self.world, id, particles, local_positions),
                ComponentPoseBinding::Body { id, body } => {
                    let body = *body as usize;
                    Ok(ContinuousComponentPose {
                        id: id.clone(),
                        position: self.world.bodies.position(body),
                        rotation_radians: self.world.bodies.angle_radians[body],
                    })
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let connections = self
            .connection_bindings
            .iter()
            .map(|binding| ContinuousConnection {
                id: binding.id.clone(),
                layer: binding.layer,
                width: binding.width,
                points: binding
                    .particles
                    .iter()
                    .map(|particle| self.world.particles.position(*particle as usize))
                    .collect(),
            })
            .collect();
        Ok(ContinuousSolution {
            components,
            connections,
        })
    }
}

pub trait RepresentationCompiler {
    fn name(&self) -> &'static str;
    fn compile(&self, board: &Board) -> Result<CompiledWorld, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ParticleRepresentationCompiler {
    pub policy: CompilePolicy,
}

impl RepresentationCompiler for ParticleRepresentationCompiler {
    fn name(&self) -> &'static str {
        "particle-distance-v0"
    }

    fn compile(&self, board: &Board) -> Result<CompiledWorld, String> {
        compile_particle_world(board, self.policy)
    }
}

pub fn compile_particle_world(
    board: &Board,
    policy: CompilePolicy,
) -> Result<CompiledWorld, String> {
    board.validate()?;
    if policy.maximum_trace_spacing <= 0.0 {
        return Err("maximum trace spacing must be positive".into());
    }

    let mut world = World::new(board.bounds);
    let mut terminal_particles = HashMap::new();
    let mut body_handles = HashMap::new();
    let mut component_pose_bindings = Vec::new();
    let mut connection_bindings = Vec::new();
    let mut next_stable_id = 0_u32;

    for component in &board.components {
        let mask = mobility_mask(
            component.mobility.translate_x,
            component.mobility.translate_y,
        );
        match component.kind {
            ComponentKind::Anchor => {
                component_pose_bindings.push(ComponentPoseBinding::Declared {
                    id: component.id.clone(),
                    position: component.position,
                    rotation_radians: component.rotation_radians,
                });
                let field_weight = reciprocal_count(component.terminals.len());
                for terminal in &component.terminals {
                    let position = component.position
                        + rotate(terminal.local_position, component.rotation_radians);
                    let handle = world.particles.push(ParticleInit {
                        position,
                        field_weight,
                        inverse_mass: component.mobility.inverse_mass,
                        mobility: mask,
                        role: ParticleRole::Terminal,
                        stable_id: next_stable_id,
                        label: format!("{}:{}", component.id, terminal.id),
                    });
                    next_stable_id += 1;
                    terminal_particles.insert((component.id.clone(), terminal.id.clone()), handle);
                }
            }
            ComponentKind::TwoTerminalBody { length, radius } => {
                if component.terminals.len() != 2 {
                    return Err(format!(
                        "{} is a two-terminal body but declares {} terminals",
                        component.id,
                        component.terminals.len()
                    ));
                }
                match policy.two_terminal_representation {
                    TwoTerminalRepresentation::Unspecified => {
                        return Err(format!(
                            "{} needs an explicit two-terminal representation; available models have different rotation and work semantics",
                            component.id
                        ));
                    }
                    TwoTerminalRepresentation::ExperimentalEndpointParticles => {
                        let mut handles = [0_u32; 2];
                        for (index, terminal) in component.terminals.iter().enumerate() {
                            let position = component.position
                                + rotate(terminal.local_position, component.rotation_radians);
                            let handle = world.particles.push(ParticleInit {
                                position,
                                field_weight: 0.5,
                                inverse_mass: component.mobility.inverse_mass * 2.0,
                                mobility: mask,
                                role: ParticleRole::Terminal,
                                stable_id: next_stable_id,
                                label: format!("{}:{}", component.id, terminal.id),
                            });
                            next_stable_id += 1;
                            handles[index] = handle;
                            terminal_particles
                                .insert((component.id.clone(), terminal.id.clone()), handle);
                        }
                        world.equality_distance.push(
                            handles[0],
                            handles[1],
                            length,
                            format!("component:{}:length", component.id),
                        );
                        component_pose_bindings.push(ComponentPoseBinding::ParticleShape {
                            id: component.id.clone(),
                            particles: handles.to_vec(),
                            local_positions: component
                                .terminals
                                .iter()
                                .map(|terminal| terminal.local_position)
                                .collect(),
                        });
                    }
                    TwoTerminalRepresentation::AnalyticRigidBodyAttachments => {
                        let inverse_inertia = if !component.mobility.rotate {
                            0.0
                        } else {
                            component.mobility.rotation_mobility * 12.0
                                / (length * length + 4.0 * radius * radius)
                        };
                        let body = world.bodies.push(BodyInit {
                            position: component.position,
                            angle_radians: component.rotation_radians,
                            half_size: Vec2::new(length * 0.5, radius),
                            inverse_mass: component.mobility.inverse_mass,
                            inverse_inertia,
                            mobility: mask,
                            rotate: component.mobility.rotate,
                            stable_id: next_stable_id,
                            label: component.id.clone(),
                        });
                        next_stable_id += 1;
                        body_handles.insert(component.id.clone(), body);
                        component_pose_bindings.push(ComponentPoseBinding::Body {
                            id: component.id.clone(),
                            body,
                        });
                        for terminal in &component.terminals {
                            let position = component.position
                                + rotate(terminal.local_position, component.rotation_radians);
                            let particle = world.particles.push(ParticleInit {
                                position,
                                field_weight: 0.5,
                                inverse_mass: component.mobility.inverse_mass * 2.0,
                                mobility: mask,
                                role: ParticleRole::Terminal,
                                stable_id: next_stable_id,
                                label: format!("{}:{}", component.id, terminal.id),
                            });
                            next_stable_id += 1;
                            terminal_particles
                                .insert((component.id.clone(), terminal.id.clone()), particle);
                            world.attachments.push(
                                particle,
                                body,
                                terminal.local_position,
                                format!("component:{}:attachment:{}", component.id, terminal.id),
                            );
                        }
                    }
                }
            }
            ComponentKind::Rect { size, .. } => match policy.rectangle_representation {
                RectangleRepresentation::CornerDistanceParticles => {
                    let half = size * 0.5;
                    let offsets = [
                        Vec2::new(-half.x, -half.y),
                        Vec2::new(half.x, -half.y),
                        Vec2::new(half.x, half.y),
                        Vec2::new(-half.x, half.y),
                    ];
                    let mut handles = [0_u32; 4];
                    for (index, offset) in offsets.iter().enumerate() {
                        handles[index] = world.particles.push(ParticleInit {
                            position: component.position
                                + rotate(*offset, component.rotation_radians),
                            field_weight: 0.25,
                            inverse_mass: component.mobility.inverse_mass * 4.0,
                            mobility: mask,
                            role: ParticleRole::Component,
                            stable_id: next_stable_id,
                            label: format!("{}:corner:{index}", component.id),
                        });
                        next_stable_id += 1;
                    }
                    component_pose_bindings.push(ComponentPoseBinding::ParticleShape {
                        id: component.id.clone(),
                        particles: handles.to_vec(),
                        local_positions: offsets.to_vec(),
                    });
                    for first in 0..4 {
                        for second in (first + 1)..4 {
                            world.equality_distance.push(
                                handles[first],
                                handles[second],
                                offsets[first].distance(offsets[second]),
                                format!("component:{}:shape:{first}:{second}", component.id),
                            );
                        }
                    }
                    for terminal in &component.terminals {
                        let (corner, distance) = offsets
                            .iter()
                            .enumerate()
                            .map(|(index, offset)| {
                                (index, offset.distance(terminal.local_position))
                            })
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .expect("a rectangle has four corners");
                        if distance > 1.0e-4 {
                            return Err(format!(
                                "{}:{} is not on a rectangle corner; affine particle attachments are not implemented yet",
                                component.id, terminal.id
                            ));
                        }
                        terminal_particles
                            .insert((component.id.clone(), terminal.id.clone()), handles[corner]);
                    }
                }
                RectangleRepresentation::AnalyticRigidBodyAttachments => {
                    let inverse_inertia = if !component.mobility.rotate {
                        0.0
                    } else {
                        component.mobility.rotation_mobility * 12.0
                            / (size.x * size.x + size.y * size.y)
                    };
                    let body = world.bodies.push(BodyInit {
                        position: component.position,
                        angle_radians: component.rotation_radians,
                        half_size: size * 0.5,
                        inverse_mass: component.mobility.inverse_mass,
                        inverse_inertia,
                        mobility: mask,
                        rotate: component.mobility.rotate,
                        stable_id: next_stable_id,
                        label: component.id.clone(),
                    });
                    next_stable_id += 1;
                    body_handles.insert(component.id.clone(), body);
                    component_pose_bindings.push(ComponentPoseBinding::Body {
                        id: component.id.clone(),
                        body,
                    });
                    let terminal_count = component.terminals.len();
                    let field_weight = reciprocal_count(terminal_count);
                    let terminal_inverse_mass =
                        component.mobility.inverse_mass * terminal_count.max(1) as f32;
                    for terminal in &component.terminals {
                        let position = component.position
                            + rotate(terminal.local_position, component.rotation_radians);
                        let particle = world.particles.push(ParticleInit {
                            position,
                            field_weight,
                            inverse_mass: terminal_inverse_mass,
                            mobility: mask,
                            role: ParticleRole::Terminal,
                            stable_id: next_stable_id,
                            label: format!("{}:{}", component.id, terminal.id),
                        });
                        next_stable_id += 1;
                        terminal_particles
                            .insert((component.id.clone(), terminal.id.clone()), particle);
                        world.attachments.push(
                            particle,
                            body,
                            terminal.local_position,
                            format!("component:{}:attachment:{}", component.id, terminal.id),
                        );
                    }
                }
            },
        }
    }

    for connection in &board.connections {
        if connection.terminals.len() != 2 {
            return Err(format!(
                "{} is multi-terminal; it needs a coordinator-owned route graph before particle compilation",
                connection.id
            ));
        }
        let first = terminal_handle(&terminal_particles, &connection.terminals[0])?;
        let last = terminal_handle(&terminal_particles, &connection.terminals[1])?;
        let seed = sampled_trace_points(
            connection,
            world.particles.position(first as usize),
            world.particles.position(last as usize),
            policy.trace_sampling,
            policy.maximum_trace_spacing,
            policy.maximum_trace_particles_per_connection,
        )?;
        let mut chain = vec![first];
        let trace_field_weight = reciprocal_count(seed.len());
        for (index, position) in seed.into_iter().enumerate() {
            let handle = world.particles.push(ParticleInit {
                position,
                field_weight: trace_field_weight,
                inverse_mass: 1.0,
                mobility: MobilityMask::XY,
                role: ParticleRole::Trace,
                stable_id: next_stable_id,
                label: format!("{}:trace:{index}", connection.id),
            });
            next_stable_id += 1;
            chain.push(handle);
        }
        chain.push(last);
        for pair in chain.windows(2) {
            world.trace_tension.push(
                pair[0],
                pair[1],
                1.0,
                format!("trace:{}:tension", connection.id),
            );
            world.maximum_distance.push(
                pair[0],
                pair[1],
                policy.maximum_trace_spacing,
                format!("trace:{}:sampling", connection.id),
            );
            world.segments.push(Segment {
                first: pair[0],
                second: pair[1],
                layer: connection.layer,
                width: connection.width,
                connection: connection.id.clone(),
            });
        }
        connection_bindings.push(ConnectionBinding {
            id: connection.id.clone(),
            layer: connection.layer,
            width: connection.width,
            particles: chain,
        });
    }

    Ok(CompiledWorld {
        world,
        terminal_particles,
        body_handles,
        component_pose_bindings,
        connection_bindings,
    })
}

fn particle_shape_pose(
    world: &World,
    id: &str,
    particles: &[u32],
    local_positions: &[Vec2],
) -> Result<ContinuousComponentPose, String> {
    if particles.len() != local_positions.len() || particles.len() < 2 {
        return Err(format!("{id} has an invalid particle pose binding"));
    }
    let local_axis = local_positions[1] - local_positions[0];
    let world_axis = world.particles.position(particles[1] as usize)
        - world.particles.position(particles[0] as usize);
    if local_axis.length_squared() <= 1.0e-12 || world_axis.length_squared() <= 1.0e-12 {
        return Err(format!("{id} has a degenerate particle pose binding"));
    }
    let rotation_radians = world_axis.y.atan2(world_axis.x) - local_axis.y.atan2(local_axis.x);
    let position = particles
        .iter()
        .zip(local_positions)
        .map(|(particle, local)| {
            world.particles.position(*particle as usize) - rotate(*local, rotation_radians)
        })
        .fold(Vec2::ZERO, |sum, value| sum + value)
        / particles.len() as f32;
    if !position.x.is_finite() || !position.y.is_finite() || !rotation_radians.is_finite() {
        return Err(format!("{id} produced a non-finite component pose"));
    }
    Ok(ContinuousComponentPose {
        id: id.into(),
        position,
        rotation_radians,
    })
}

fn rotate(value: Vec2, angle: f32) -> Vec2 {
    let cosine = angle.cos();
    let sine = angle.sin();
    Vec2::new(
        value.x * cosine - value.y * sine,
        value.x * sine + value.y * cosine,
    )
}

fn terminal_handle(
    handles: &HashMap<(String, String), u32>,
    terminal: &pcb_core::TerminalRef,
) -> Result<u32, String> {
    handles
        .get(&(terminal.component.clone(), terminal.terminal.clone()))
        .copied()
        .ok_or_else(|| {
            format!(
                "unknown terminal {}:{}",
                terminal.component, terminal.terminal
            )
        })
}

fn sampled_trace_points(
    connection: &pcb_core::Connection,
    first: Vec2,
    last: Vec2,
    policy: TraceSamplingPolicy,
    maximum_spacing: f32,
    maximum_particles: usize,
) -> Result<Vec<Vec2>, String> {
    let interior = if connection.seed_route.len() >= 2 {
        &connection.seed_route[1..connection.seed_route.len() - 1]
    } else {
        &[]
    };
    if policy == TraceSamplingPolicy::PreserveSeedVertices {
        if interior.len() > maximum_particles {
            return Err(format!(
                "trace {} needs {} seed particles, exceeding the configured limit {maximum_particles}",
                connection.id,
                interior.len()
            ));
        }
        return Ok(interior.to_vec());
    }
    let mut waypoints = Vec::with_capacity(interior.len() + 2);
    waypoints.push(first);
    waypoints.extend_from_slice(interior);
    waypoints.push(last);
    let mut sampled = Vec::new();
    for (segment_index, pair) in waypoints.windows(2).enumerate() {
        let delta = pair[1] - pair[0];
        let subdivisions = (delta.length() / maximum_spacing).ceil().max(1.0) as usize;
        for subdivision in 1..=subdivisions {
            let is_final_endpoint =
                segment_index == waypoints.len() - 2 && subdivision == subdivisions;
            if !is_final_endpoint {
                if sampled.len() == maximum_particles {
                    return Err(format!(
                        "trace {} subdivision exceeds the configured particle limit {maximum_particles}",
                        connection.id
                    ));
                }
                sampled.push(pair[0] + delta * (subdivision as f32 / subdivisions as f32));
            }
        }
    }
    Ok(sampled)
}

fn mobility_mask(x: bool, y: bool) -> MobilityMask {
    match (x, y) {
        (false, false) => MobilityMask::FIXED,
        (true, false) => MobilityMask::X,
        (false, true) => MobilityMask::Y,
        (true, true) => MobilityMask::XY,
    }
}

fn reciprocal_count(count: usize) -> f32 {
    if count == 0 { 0.0 } else { 1.0 / count as f32 }
}

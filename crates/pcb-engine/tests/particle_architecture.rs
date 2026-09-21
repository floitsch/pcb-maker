use pcb_core::{
    Board, Bounds, Component, ComponentKind, Connection, Layer, Mobility, ParticleRole, Rules,
    Terminal, TerminalRef, Vec2,
};
use pcb_engine::{
    Backend, CompilePolicy, CpuReferenceBackend, DensityField, GpuDispatchPlan, GpuKernel,
    MobilityMask, NoField, ParticleInit, ParticleRepresentationCompiler, RectangleRepresentation,
    RepresentationCompiler, SolverConfig, TwoTerminalRepresentation, World, compile_particle_world,
};

fn experimental_particle_policy() -> CompilePolicy {
    CompilePolicy {
        two_terminal_representation: TwoTerminalRepresentation::ExperimentalEndpointParticles,
        ..CompilePolicy::default()
    }
}

fn analytic_rigid_body_policy() -> CompilePolicy {
    CompilePolicy {
        two_terminal_representation: TwoTerminalRepresentation::AnalyticRigidBodyAttachments,
        ..CompilePolicy::default()
    }
}

fn resistor_board() -> Board {
    Board {
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
    }
}

#[test]
fn two_terminal_passive_is_particles_plus_a_generic_constraint() {
    let compiler = ParticleRepresentationCompiler {
        policy: experimental_particle_policy(),
    };
    assert_eq!(compiler.name(), "particle-distance-v0");
    let compiled = compiler.compile(&resistor_board()).unwrap();
    assert_eq!(compiled.world.particles.len(), 2);
    assert_eq!(compiled.world.equality_distance.len(), 1);
    assert!(
        compiled
            .world
            .particles
            .role
            .iter()
            .all(|role| *role == ParticleRole::Terminal)
    );
    assert_eq!(compiled.world.equality_distance.distance, vec![4.0]);
    assert_eq!(compiled.world.particles.field_weight, vec![0.5, 0.5]);
}

#[test]
fn analytic_two_terminal_body_has_explicit_pose_and_local_attachments() {
    let compiled = compile_particle_world(&resistor_board(), analytic_rigid_body_policy()).unwrap();
    assert_eq!(compiled.world.particles.len(), 2);
    assert_eq!(compiled.world.bodies.len(), 1);
    assert_eq!(compiled.world.attachments.len(), 2);
    assert!(compiled.world.equality_distance.is_empty());
    assert!(compiled.world.bodies.rotate[0]);
    assert_eq!(compiled.body_handles["R1"], 0);
    assert_eq!(compiled.world.attachments.local(0), Vec2::new(-2.0, 0.0));
    assert_eq!(compiled.world.attachments.local(1), Vec2::new(2.0, 0.0));
}

#[test]
fn both_two_terminal_representations_emit_the_same_initial_semantic_pose() {
    let mut board = resistor_board();
    board.components[0].rotation_radians = 0.7;
    for policy in [experimental_particle_policy(), analytic_rigid_body_policy()] {
        let compiled = compile_particle_world(&board, policy).unwrap();
        let solution = compiled.solution().unwrap();
        assert_eq!(solution.components.len(), 1);
        assert_eq!(solution.components[0].id, "R1");
        assert!(
            solution.components[0]
                .position
                .distance(Vec2::new(10.0, 10.0))
                < 1.0e-5
        );
        assert!((solution.components[0].rotation_radians - 0.7).abs() < 1.0e-5);
    }
}

#[test]
fn asymmetric_terminal_pull_rotates_analytic_body_and_preserves_its_length() {
    let mut compiled =
        compile_particle_world(&resistor_board(), analytic_rigid_body_policy()).unwrap();
    compiled
        .world
        .particles
        .set_position(1, Vec2::new(12.0, 3.0));
    let mut backend = CpuReferenceBackend::with_field(NoField);
    let frame = backend.step(
        &mut compiled.world,
        &SolverConfig {
            projection_iterations: 80,
            ..SolverConfig::default()
        },
        0,
    );

    let angle = compiled.world.bodies.angle_radians[0];
    let terminal_distance = compiled
        .world
        .particles
        .position(0)
        .distance(compiled.world.particles.position(1));
    assert!(angle.abs() > 0.2, "expected rotation, got {angle}");
    assert!((terminal_distance - 4.0).abs() < 1.0e-3);
    assert!(frame.metrics.max_constraint_residual < 1.0e-3);
    assert_eq!(frame.bodies.len(), 1);
    assert_eq!(frame.attachments.len(), 2);
    assert_eq!(frame.metrics.constraint_projections, 160);
    assert_eq!(frame.metrics.constraint_scalar_rows, 320);
}

#[test]
fn rotation_mobility_is_an_independent_control() {
    let mut board = resistor_board();
    board.components[0].mobility = Mobility {
        inverse_mass: 1.0,
        rotation_mobility: 1.0,
        translate_x: true,
        translate_y: true,
        rotate: false,
    };
    let mut compiled = compile_particle_world(&board, analytic_rigid_body_policy()).unwrap();
    compiled
        .world
        .particles
        .set_position(1, Vec2::new(12.0, 3.0));
    let mut backend = CpuReferenceBackend::with_field(NoField);
    backend.step(
        &mut compiled.world,
        &SolverConfig {
            projection_iterations: 80,
            ..SolverConfig::default()
        },
        0,
    );
    assert_eq!(compiled.world.bodies.angle_radians[0], 0.0);
}

#[test]
fn rectangular_body_is_uniform_particles_and_distance_constraints() {
    let board = Board {
        bounds: Bounds::new(Vec2::ZERO, Vec2::new(20.0, 20.0)),
        layers: vec![Layer { id: "top".into() }],
        rules: Rules { clearance: 0.2 },
        components: vec![Component {
            id: "U1".into(),
            position: Vec2::new(10.0, 10.0),
            rotation_radians: 0.0,
            kind: ComponentKind::Rect {
                size: Vec2::new(4.0, 2.0),
                routing_keepout: true,
            },
            mobility: Mobility::FREE,
            terminals: vec![],
        }],
        connections: vec![],
    };

    let compiled = compile_particle_world(&board, CompilePolicy::default()).unwrap();
    assert_eq!(compiled.world.particles.len(), 4);
    assert_eq!(compiled.world.equality_distance.len(), 6);
    assert_eq!(
        compiled.world.particles.field_weight.iter().sum::<f32>(),
        1.0
    );
    assert!(
        compiled
            .world
            .particles
            .role
            .iter()
            .all(|role| *role == ParticleRole::Component)
    );
}

#[test]
fn analytic_rect_body_supports_arbitrary_body_local_terminals() {
    let board = Board {
        bounds: Bounds::new(Vec2::ZERO, Vec2::new(20.0, 20.0)),
        layers: vec![Layer { id: "top".into() }],
        rules: Rules { clearance: 0.2 },
        components: vec![Component {
            id: "U1".into(),
            position: Vec2::new(10.0, 10.0),
            rotation_radians: 0.4,
            kind: ComponentKind::Rect {
                size: Vec2::new(5.0, 3.0),
                routing_keepout: true,
            },
            mobility: Mobility::FREE,
            terminals: vec![
                Terminal {
                    id: "1".into(),
                    local_position: Vec2::new(-1.7, 0.8),
                },
                Terminal {
                    id: "2".into(),
                    local_position: Vec2::new(2.1, -0.4),
                },
                Terminal {
                    id: "3".into(),
                    local_position: Vec2::new(0.3, 1.2),
                },
            ],
        }],
        connections: vec![],
    };
    let compiled = compile_particle_world(
        &board,
        CompilePolicy {
            rectangle_representation: RectangleRepresentation::AnalyticRigidBodyAttachments,
            ..CompilePolicy::default()
        },
    )
    .unwrap();
    assert_eq!(compiled.world.bodies.len(), 1);
    assert_eq!(compiled.world.particles.len(), 3);
    assert_eq!(compiled.world.attachments.len(), 3);
    assert_eq!(compiled.world.particles.field_weight, vec![1.0 / 3.0; 3]);
    let solution = compiled.solution().unwrap();
    assert!(
        solution.components[0]
            .position
            .distance(Vec2::new(10.0, 10.0))
            < 1.0e-5
    );
    assert!((solution.components[0].rotation_radians - 0.4).abs() < 1.0e-6);
}

#[test]
fn generic_distance_projection_restores_passive_length() {
    let mut compiled =
        compile_particle_world(&resistor_board(), experimental_particle_policy()).unwrap();
    compiled
        .world
        .particles
        .set_position(1, Vec2::new(17.0, 11.0));
    let mut backend = CpuReferenceBackend::new();
    let config = SolverConfig {
        field_strength: 0.0,
        projection_iterations: 12,
        ..SolverConfig::default()
    };
    let frame = backend.step(&mut compiled.world, &config, 0);
    let distance = compiled
        .world
        .particles
        .position(0)
        .distance(compiled.world.particles.position(1));
    assert!((distance - 4.0).abs() < 1.0e-4);
    assert!(frame.metrics.max_constraint_residual < 1.0e-4);
}

#[test]
fn density_scatter_conserves_deposited_mass() {
    let mut field = DensityField::new(Bounds::new(Vec2::ZERO, Vec2::new(10.0, 10.0)), 16, 16);
    field.scatter(Vec2::new(4.25, 7.75), 3.5);
    assert!((field.density.iter().sum::<f32>() - 3.5).abs() < 1.0e-5);
}

#[test]
fn gpu_plan_exposes_field_and_constraint_families_as_separate_dispatches() {
    let plan = GpuDispatchPlan::default();
    assert!(plan.kernels.contains(&GpuKernel::ScatterField));
    assert!(plan.kernels.contains(&GpuKernel::SmoothPressure));
    assert!(plan.kernels.contains(&GpuKernel::AccumulateTraceTension));
    assert!(plan.kernels.contains(&GpuKernel::ProjectEqualityDistance));
    assert!(plan.kernels.contains(&GpuKernel::ProjectMaximumDistance));
    assert!(plan.kernels.contains(&GpuKernel::ProjectSegmentClearance));
    assert!(
        plan.kernels
            .contains(&GpuKernel::ProjectSegmentBodyClearance)
    );
    assert!(plan.kernels.contains(&GpuKernel::ProjectBodyBodyClearance));
    assert!(plan.kernels.contains(&GpuKernel::ProjectAttachments));
}

#[test]
fn field_algorithm_is_replaceable_without_changing_the_solver() {
    let mut compiled =
        compile_particle_world(&resistor_board(), experimental_particle_policy()).unwrap();
    let mut backend = CpuReferenceBackend::with_field(NoField);
    assert_eq!(backend.field_name(), "disabled");
    let frame = backend.step(&mut compiled.world, &SolverConfig::default(), 0);
    assert!(frame.field.is_none());
    assert_eq!(frame.metrics.max_field_pressure, 0.0);
}

#[test]
fn two_terminal_representation_must_be_selected_explicitly() {
    let error = compile_particle_world(&resistor_board(), CompilePolicy::default()).unwrap_err();
    assert!(error.contains("explicit two-terminal representation"));
    assert!(error.contains("different rotation and work semantics"));
}

#[test]
fn trace_sampling_constraint_does_not_create_artificial_tension() {
    let mut world = World::new(Bounds::new(Vec2::ZERO, Vec2::new(20.0, 20.0)));
    let first = world.particles.push(ParticleInit {
        position: Vec2::new(5.0, 5.0),
        field_weight: 1.0,
        inverse_mass: 1.0,
        mobility: MobilityMask::XY,
        role: ParticleRole::Trace,
        stable_id: 0,
        label: "trace:0".into(),
    });
    let second = world.particles.push(ParticleInit {
        position: Vec2::new(6.0, 5.0),
        field_weight: 1.0,
        inverse_mass: 1.0,
        mobility: MobilityMask::XY,
        role: ParticleRole::Trace,
        stable_id: 1,
        label: "trace:1".into(),
    });
    world
        .maximum_distance
        .push(first, second, 3.0, "trace:sampling".into());

    let mut backend = CpuReferenceBackend::with_field(NoField);
    backend.step(&mut world, &SolverConfig::default(), 0);

    assert_eq!(world.particles.position(0), Vec2::new(5.0, 5.0));
    assert_eq!(world.particles.position(1), Vec2::new(6.0, 5.0));
}

#[test]
fn semantic_trace_sampling_subdivides_long_direct_segments() {
    let anchor = |id: &str, x: f32| Component {
        id: id.into(),
        position: Vec2::new(x, 10.0),
        rotation_radians: 0.0,
        kind: ComponentKind::Anchor,
        mobility: Mobility::FIXED,
        terminals: vec![Terminal {
            id: "p".into(),
            local_position: Vec2::ZERO,
        }],
    };
    let board = Board {
        bounds: Bounds::new(Vec2::ZERO, Vec2::new(20.0, 20.0)),
        layers: vec![Layer { id: "top".into() }],
        rules: Rules { clearance: 0.2 },
        components: vec![anchor("A", 2.0), anchor("B", 12.0)],
        connections: vec![Connection {
            id: "N".into(),
            terminals: vec![
                TerminalRef {
                    component: "A".into(),
                    terminal: "p".into(),
                },
                TerminalRef {
                    component: "B".into(),
                    terminal: "p".into(),
                },
            ],
            layer: 0,
            width: 0.3,
            seed_route: Vec::new(),
        }],
    };
    let mut compiled = compile_particle_world(&board, CompilePolicy::default()).unwrap();
    assert_eq!(compiled.world.particles.len(), 6);
    assert_eq!(compiled.world.maximum_distance.len(), 5);
    let solution = compiled.solution().unwrap();
    assert_eq!(solution.connections[0].points.len(), 6);
    assert!(
        solution.connections[0]
            .points
            .windows(2)
            .all(|pair| pair[0].distance(pair[1]) <= 2.0 + 1.0e-6)
    );
    let mut backend = CpuReferenceBackend::with_field(NoField);
    let frame = backend.step(&mut compiled.world, &SolverConfig::default(), 0);
    assert!(frame.metrics.max_constraint_residual < 1.0e-6);
}

#[test]
fn semantic_trace_sampling_fails_before_exceeding_its_particle_bound() {
    let anchor = |id: &str, x: f32| Component {
        id: id.into(),
        position: Vec2::new(x, 10.0),
        rotation_radians: 0.0,
        kind: ComponentKind::Anchor,
        mobility: Mobility::FIXED,
        terminals: vec![Terminal {
            id: "p".into(),
            local_position: Vec2::ZERO,
        }],
    };
    let board = Board {
        bounds: Bounds::new(Vec2::ZERO, Vec2::new(20.0, 20.0)),
        layers: vec![Layer { id: "top".into() }],
        rules: Rules { clearance: 0.2 },
        components: vec![anchor("A", 2.0), anchor("B", 12.0)],
        connections: vec![Connection {
            id: "N".into(),
            terminals: vec![
                TerminalRef {
                    component: "A".into(),
                    terminal: "p".into(),
                },
                TerminalRef {
                    component: "B".into(),
                    terminal: "p".into(),
                },
            ],
            layer: 0,
            width: 0.3,
            seed_route: Vec::new(),
        }],
    };
    let error = compile_particle_world(
        &board,
        CompilePolicy {
            maximum_trace_particles_per_connection: 3,
            ..CompilePolicy::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("particle limit 3"));
}

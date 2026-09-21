// Copyright (C) 2026 Toit contributors.

use pcb_core::{Bounds, Mobility, Vec2, VectorView};
use pcb_engine::*;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn line(id: &str, points: &[(f32, f32)], mobility: CopperPolylineMobility) -> CopperPolylineInput {
    CopperPolylineInput {
        id: id.into(),
        points: points.iter().map(|&(x, y)| Vec2::new(x, y)).collect(),
        width: 0.4,
        tension_weight: 1.0,
        mobility,
    }
}

fn base() -> CopperRepairRequest {
    CopperRepairRequest {
        bounds: Bounds::new(Vec2::ZERO, Vec2::new(30.0, 20.0)),
        bodies: vec![],
        polylines: vec![],
        shared_points: vec![],
        attachments: vec![],
        separations: vec![],
        body_separations: vec![],
        body_body_separations: vec![],
        maximum_segment_pairs: 10000,
    }
}

fn body(id: &str, p: (f32, f32), size: (f32, f32), mobility: Mobility) -> CopperBodyInput {
    CopperBodyInput {
        id: id.into(),
        position: Vec2::new(p.0, p.1),
        angle_radians: 0.0,
        size: Vec2::new(size.0, size.1),
        mobility,
    }
}

fn run(
    out: &Path,
    id: &str,
    note: &str,
    request: CopperRepairRequest,
    tension: f32,
    density: f32,
    steps: u64,
    sweeps: usize,
) {
    let mut compiled = compile_copper_repair(&request).unwrap();
    // Explicit ablation: copper compilation intentionally supplies zero field
    // weights. Unit sample weights demonstrate activation, not area-correct occupancy.
    let unit_field_weights = id.ends_with("weighted");
    if unit_field_weights {
        compiled.world.particles.field_weight.fill(1.0);
    }
    let mut backend = CpuReferenceBackend::new();
    let config = SolverConfig {
        trace_tension_strength: tension,
        field_strength: density,
        projection_iterations: sweeps,
        field_width: 40,
        field_height: 26,
        maximum_trace_tension_step: 0.10,
        ..SolverConfig::default()
    };
    let specs: Vec<Value> = request
        .polylines
        .iter()
        .map(|p| {
            json!({
                "id": p.id, "width":p.width,"mobility":format!("{:?}",p.mobility),
                "initial_points":p.points,
            })
        })
        .collect();
    let body_specs: Vec<Value> = request
        .bodies
        .iter()
        .map(|b| {
            json!({
                "id":b.id,"size":b.size,"mobility":b.mobility,"initial_position":b.position,
                "initial_angle":b.angle_radians,
            })
        })
        .collect();
    let mut frames = vec![];
    for step in 0..=steps {
        let before: Vec<_> = (0..compiled.world.particles.len())
            .map(|i| compiled.world.particles.position(i))
            .collect();
        let initial = SolverConfig {
            trace_tension_strength: 0.0,
            field_strength: 0.0,
            projection_iterations: 0,
            ..config
        };
        let mut frame = backend.step(
            &mut compiled.world,
            if step == 0 { &initial } else { &config },
            step,
        );
        // The solver already records the proposed field/tension displacements.
        // Recover the net positional correction; this is not a physical force.
        for (i, old) in before.iter().enumerate() {
            let stable_id = compiled.world.particles.stable_id[i];
            let proposed = frame
                .vectors
                .iter()
                .filter(|v| v.particle == stable_id)
                .fold(Vec2::ZERO, |sum, v| sum + v.vector);
            let correction = compiled.world.particles.position(i) - *old - proposed;
            if correction.length() > 1e-7 {
                frame.vectors.push(VectorView {
                    particle: stable_id,
                    origin: *old + proposed,
                    vector: correction,
                    source: "constraint_correction".into(),
                });
            }
        }
        let routes: Vec<Value> = compiled
            .polylines()
            .into_iter()
            .map(|(id, points)| json!({"id":id,"points":points}))
            .collect();
        let length: f32 = compiled
            .polylines()
            .iter()
            .flat_map(|(_, p)| p.windows(2))
            .map(|p| p[0].distance(p[1]))
            .sum();
        frames.push(json!({"frame":frame,"routes":routes,"length_mm":length}));
    }
    let document = json!({"id":id,"note":note,"scope":"Actual pcb-engine CPU reference backend; reduced single-layer copper fixtures, not KiCad verification",
        "settings":{"tension":tension,"density":density,"unit_sample_field_weights":unit_field_weights,"steps":steps,"projection_sweeps":sweeps,"timestep":config.timestep},
        "routes":specs,"bodies":body_specs,
        "body_clearances":request.body_separations.iter().map(|p|json!({"route":p.polyline,"body":p.body,"clearance":p.clearance})).collect::<Vec<_>>(),
        "trace_clearances":request.separations.iter().map(|p|json!({"first":p.first,"second":p.second,"clearance":p.clearance})).collect::<Vec<_>>(),
        "attachments":request.attachments.iter().map(|a|json!({"route":a.polyline,"point":a.point_index,"body":a.body,"local_position":a.local_position})).collect::<Vec<_>>(),
        "shared_points":request.shared_points.iter().map(|g|json!({"id":g.id,"members":g.members.iter().map(|m|json!({"route":m.polyline,"point":m.point_index})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "frames":frames});
    fs::write(
        out.join(format!("{id}.json")),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    println!(
        "{id}: {:.4} -> {:.4} mm; residual {}",
        document["frames"][0]["length_mm"].as_f64().unwrap(),
        document["frames"][steps as usize]["length_mm"]
            .as_f64()
            .unwrap(),
        document["frames"][steps as usize]["frame"]["metrics"]["max_constraint_residual"]
    );
}

fn main() {
    let directory = std::env::args().nth(1).expect("output directory");
    let settled_only = std::env::args().nth(2).as_deref() == Some("--settled-only");
    let out = Path::new(&directory);
    fs::create_dir_all(out).unwrap();
    let mut slack = base();
    slack.polylines.push(line(
        "slack",
        &[(2., 10.), (8., 4.), (15., 3.), (22., 5.), (28., 10.)],
        CopperPolylineMobility::Interior,
    ));
    if settled_only {
        run(
            out,
            "slack-settled",
            "Longer-budget continuation of the slack-tension fixture, approaching its known 26 mm straight-line lower bound.",
            slack,
            3.,
            0.,
            1200,
            8,
        );
    } else {
        run(
            out,
            "slack-no-tension",
            "Default zero tension leaves a legal slack route unchanged.",
            slack.clone(),
            0.,
            0.,
            240,
            8,
        );
        run(
            out,
            "slack-tension",
            "Explicit trace-length gradient pulls the same route taut.",
            slack,
            3.,
            0.,
            240,
            8,
        );
        for (id, y) in [("obstacle-upper", 2.), ("obstacle-lower", 17.)] {
            let mut r = base();
            r.bodies
                .push(body("fixed obstacle", (15., 9.), (6., 8.), Mobility::FIXED));
            r.polylines.push(line(
                "signal",
                &[(2., 8.), (10., y), (15., y), (20., y), (28., 8.)],
                CopperPolylineMobility::Interior,
            ));
            r.body_separations.push(CopperSegmentBodyPair {
                polyline: "signal".into(),
                body: "fixed obstacle".into(),
                clearance: 0.3,
            });
            run(
                out,
                id,
                "Same endpoints and obstacle; seed chooses a different side. Continuous tightening is not a topology search.",
                r,
                3.,
                0.,
                480,
                16,
            );
        }
        for (id, density) in [
            ("trace-contact", 0.),
            ("trace-contact-density", 6.),
            ("trace-contact-density-weighted", 6.),
        ] {
            let mut r = base();
            r.polylines.push(line(
                "fixed",
                &[(6., 10.), (24., 10.)],
                CopperPolylineMobility::Fixed,
            ));
            r.polylines.push(line(
                "moving",
                &[(2., 8.), (8., 9.7), (15., 9.7), (22., 9.7), (28., 8.)],
                CopperPolylineMobility::Interior,
            ));
            r.separations.push(CopperSeparationPair {
                first: "fixed".into(),
                second: "moving".into(),
                clearance: 0.3,
            });
            run(
                out,
                id,
                "Initially overlapping trace envelopes. Copper compilation has zero field weights: enabling field strength alone has no effect. The weighted ablation uses unit sample weights, not physical area occupancy.",
                r,
                3.,
                density,
                240,
                8,
            );
        }
    }
    let mut coupled = base();
    coupled
        .bodies
        .push(body("free component", (17., 14.), (4., 2.), Mobility::FREE));
    coupled.polylines.push(line(
        "left",
        &[(2., 6.), (7., 10.), (15., 14.)],
        CopperPolylineMobility::All,
    ));
    coupled.polylines.push(line(
        "right",
        &[(19., 14.), (24., 12.), (28., 6.)],
        CopperPolylineMobility::All,
    ));
    coupled.attachments = vec![
        CopperAttachmentInput {
            polyline: "left".into(),
            point_index: 2,
            body: "free component".into(),
            local_position: Vec2::new(-2., 0.),
        },
        CopperAttachmentInput {
            polyline: "right".into(),
            point_index: 0,
            body: "free component".into(),
            local_position: Vec2::new(2., 0.),
        },
    ];
    // Each external endpoint is joined to a fixed one-point-length anchor trace.
    // Shared point compilation pins it exactly while attached component endpoints remain mobile.
    for (id, source, index, x) in [
        ("anchor-left", "left", 0, 2.),
        ("anchor-right", "right", 2, 28.),
    ] {
        coupled.polylines.push(line(
            id,
            &[(x, 6.), (x, 5.5)],
            CopperPolylineMobility::Fixed,
        ));
        coupled.shared_points.push(CopperSharedPointInput {
            id: id.into(),
            members: vec![
                CopperPointRef {
                    polyline: source.into(),
                    point_index: index,
                },
                CopperPointRef {
                    polyline: id.into(),
                    point_index: 0,
                },
            ],
        });
    }
    run(
        out,
        if settled_only {
            "coupled-settled"
        } else {
            "coupled-component"
        },
        "Trace tension moves and rotates a full rigid body through its local terminal attachments. External anchors stay fixed.",
        coupled,
        3.,
        0.,
        if settled_only { 1800 } else { 360 },
        16,
    );
}

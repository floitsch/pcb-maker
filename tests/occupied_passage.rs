// Copyright (C) 2026 Toit contributors.

use layout_trace_model::{Problem, model::Movement};
use pcb_coordinator::{
    PressureDirectionPolicy, PressureRepairConfig, repair_routing_with_pressure,
};
use pcb_routing::DutGridRoutingConfig;
use pcb_validate::SolvedComponent;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn retained_copper_drives_a_passage_move_and_fixed_wall_stays_unsolved() {
    let mut problem: Problem = serde_json::from_str(include_str!(
        "../benchmarks/small/passage-pressure-occupied.json"
    ))
    .unwrap();
    let routing: DutGridRoutingConfig = serde_json::from_value(serde_json::json!({
        "grid_mm":0.1,"retry_grid_mm":[],"max_expansions_per_search":5000000,
        "maximum_exact_edge_retries":64,"straight_cost":1000,"diagonal_cost":1414,
        "bend_cost":100,"via_cost":8000,"allow_vias":false,"route_order":"width_descending",
        "priority_branches":[],"raster_safety":"exact_centerline"
    }))
    .unwrap();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out = std::env::temp_dir().join(format!(
        "pcb-maker-occupied-passage-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&out).unwrap();
    eprintln!("Passage regression renderings: {}", out.display());
    for (name, policy, fixed, complete) in [
        (
            "single-trace-control",
            PressureDirectionPolicy::PassageCapacity,
            false,
            false,
        ),
        (
            "occupied-passage",
            PressureDirectionPolicy::PassageCapacityWithCopper,
            false,
            true,
        ),
        (
            "fixed-wall-control",
            PressureDirectionPolicy::PassageCapacityWithCopper,
            true,
            false,
        ),
    ] {
        if fixed {
            problem.components.last_mut().unwrap().constraints.movement = Movement::Fixed;
        }
        let initial: Vec<_> = problem
            .components
            .iter()
            .map(|c| SolvedComponent {
                id: c.id.clone(),
                position: c.position,
                size: c.size,
                rotation_degrees: c.rotation_degrees,
            })
            .collect();
        let config = PressureRepairConfig {
            direction_policy: policy,
            ..Default::default()
        };
        let result = repair_routing_with_pressure(&problem, &initial, &routing, &config).unwrap();
        fs::write(
            out.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        fs::write(
            out.join(format!("{name}.svg")),
            pcb_viewer::render_candidate_layer_svg(name, &problem, &result.candidate, Some("top")),
        )
        .unwrap();
        assert_eq!(result.validation.complete, complete);
        assert_eq!(result.evidence.attempted_repairs, usize::from(complete));
        if complete {
            assert_eq!(result.candidate.traces.len(), 3);
            assert!((result.candidate.components.last().unwrap().position.y - 6.0).abs() > 0.29);
            let p = &result.evidence.iterations[0].pressures[0].passage_proposals[0];
            assert!((p.required_gap_mm - 1.4).abs() < 1e-8);
            assert!((p.physical_deficit_mm - 0.25).abs() < 1e-8);
            assert_eq!(p.copper_demand.as_ref().unwrap().occupants.len(), 1);
        } else {
            assert_eq!(result.candidate.components, initial);
        }
    }
}

// Copyright (C) 2026 Toit contributors.

use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn plated_terminal_routes_directly_and_cleanup_repairs_legacy_endpoint_vias() {
    assert!(
        Command::new("kicad-cli")
            .arg("version")
            .output()
            .unwrap()
            .status
            .success()
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("benchmarks/competitive/mechanisms");
    let mut problem: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(fixtures.join("problems/mandatory-layer-transition.json")).unwrap(),
    )
    .unwrap();
    let mut bottom_pad = problem["components"][0]["pins"][0]["pads"][0].clone();
    bottom_pad["layer"] = "bottom".into();
    problem["components"][0]["pins"][0]["pads"]
        .as_array_mut()
        .unwrap()
        .push(bottom_pad);
    let problem: layout_trace_model::Problem = serde_json::from_value(problem).unwrap();
    let config: pcb_kicad::SemanticKiCadTemplateConfig = serde_json::from_str(
        &fs::read_to_string(fixtures.join("mandatory-layer-transition-template.json")).unwrap(),
    )
    .unwrap();
    let poses: Vec<_> = problem
        .components
        .iter()
        .map(|c| pcb_kicad::SemanticKiCadPose {
            component: c.id.clone(),
            position: c.position,
            rotation_degrees: 0.0,
        })
        .collect();
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-pad-layer-contact-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    let template = pcb_kicad::write_semantic_kicad_ladder_template(
        &problem,
        &poses,
        &config,
        &directory.join("template"),
    )
    .unwrap();
    let declaration = pcb_kicad::load_declaration(&template.declaration).unwrap();
    let source = directory.join("source");
    let materialized = pcb_kicad::materialize_rung(&declaration, 1, &source).unwrap();
    let source = materialized.output_directory;
    let board = source.join(format!("{}.kicad_pcb", config.board_id));
    let route = pcb_kicad::route_materialized_connection(
        &board,
        "TOP_TO_BOTTOM",
        &pcb_kicad::KiCadGridRouteConfig {
            trace_width_mm: 0.4,
            clearance_mm: 0.3,
            via_size_mm: 0.8,
            via_drill_mm: 0.4,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(route.supplemental_vias.is_empty());
    assert!(
        route
            .branches
            .iter()
            .flat_map(|b| &b.path)
            .all(|p| p.layer == "B.Cu")
    );
    let candidate_path = directory.join("candidate.json");
    fs::write(&candidate_path, serde_json::to_vec_pretty(&route).unwrap()).unwrap();
    pcb_kicad::apply_route_candidate(&board, &candidate_path, &board).unwrap();
    assert!(
        pcb_kicad::verify_materialized_rung(&source, &config.board_id)
            .unwrap()
            .complete
    );
    // Keep an explicit valid legacy route to retain the cleanup regression
    // even though initial routing no longer generates its unnecessary via.
    let mut route = route;
    let start = route.branches[0].start_terminal;
    let finish = route.branches[0].finish_terminal;
    let via_at = [(start[0] + finish[0]) / 2.0, (start[1] + finish[1]) / 2.0];
    route.router = "seeded-legacy-plated-terminal-regression".into();
    route.branches[0].path = [
        (start, "F.Cu"),
        (via_at, "F.Cu"),
        (via_at, "B.Cu"),
        (finish, "B.Cu"),
    ]
    .into_iter()
    .map(|(at, layer)| pcb_kicad::KiCadRoutePoint {
        at,
        layer: layer.into(),
    })
    .collect();
    route.supplemental_segments = [(start, via_at, "F.Cu"), (via_at, finish, "B.Cu")]
        .into_iter()
        .map(|(start, end, layer)| pcb_kicad::SupplementalSegment {
            connection: route.connection.clone(),
            start,
            end,
            width: route.config.trace_width_mm,
            layer: layer.into(),
        })
        .collect();
    route.supplemental_vias = vec![pcb_kicad::SupplementalVia {
        connection: route.connection.clone(),
        at: via_at,
        size: route.config.via_size_mm,
        drill: route.config.via_drill_mm,
        layers: ["F.Cu".into(), "B.Cu".into()],
    }];
    let materialized =
        pcb_kicad::materialize_rung(&declaration, 1, &directory.join("cleanup-source")).unwrap();
    let source = materialized.output_directory;
    let board = source.join(format!("{}.kicad_pcb", config.board_id));
    let candidate_path = directory.join("legacy-candidate.json");
    fs::write(&candidate_path, serde_json::to_vec_pretty(&route).unwrap()).unwrap();
    pcb_kicad::apply_route_candidate(&board, &candidate_path, &board).unwrap();
    assert!(
        pcb_kicad::verify_materialized_rung(&source, &config.board_id)
            .unwrap()
            .complete
    );
    let before = fs::read(&board).unwrap();
    let via = &route.supplemental_vias[0];
    let search_config = pcb_kicad::KiCadViaActionSearchConfig {
        target_at: via.at,
        target_layers: via.layers.clone(),
        removal_layers: vec!["B.Cu".into()],
        relocation_radius_mm: 0.0,
        relocation_step_mm: 0.0,
        maximum_relocation_candidates: 0,
        feasible_frontier: None,
        local_reroute: None,
        maximum_actions: 1,
        maximum_affected_branches: 1,
        via_penalty_mm: 2.0,
        minimum_score_improvement_mm: 0.001,
    };
    let search = directory.join("search");
    let (selected, evidence) = pcb_kicad::search_via_topology_actions(
        &source,
        &config.board_id,
        &candidate_path,
        &search,
        &search_config,
    )
    .unwrap();
    fs::write(
        directory.join("portfolio.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    assert_eq!(evidence.selected_attempt, Some(0));
    assert_eq!(evidence.exact_complete_actions, 1);
    let selected = selected.unwrap();
    assert!(selected.supplemental_vias.is_empty());
    assert!(
        selected
            .branches
            .iter()
            .flat_map(|b| &b.path)
            .all(|p| p.layer == "B.Cu")
    );
    assert_eq!(fs::read(&board).unwrap(), before);
    let attempt = search.join(evidence.attempts[0].artifact_directory.as_ref().unwrap());
    assert!(attempt.join("preview.svg").is_file());
    assert!(evidence.attempts[0].verification.as_ref().unwrap().complete);

    // A plated hub must also connect separately routed front/back branches.
    // Exercise both root-based routing and attachment to an existing tree.
    let mut three = serde_json::to_value(&problem).unwrap();
    three["components"][0]["position"]["x"] = 15.0.into();
    let mut top = three["components"][1].clone();
    top["id"] = "TOP_BRANCH".into();
    top["position"]["x"] = 5.0.into();
    top["pins"][0]["pads"][0]["layer"] = "top".into();
    three["components"].as_array_mut().unwrap().push(top);
    three["nets"] = serde_json::json!([]);
    three["electrical_nets"] = serde_json::json!([{
        "id": "TOP_TO_BOTTOM", "width": 0.4, "layer": "top", "allowed_layers": ["top", "bottom"],
        "terminals": [{"component": "TOP_ONLY", "pin": "1"},
            {"component": "TOP_BRANCH", "pin": "1"}, {"component": "BOTTOM_ONLY", "pin": "1"}]
    }]);
    let three: layout_trace_model::Problem = serde_json::from_value(three).unwrap();
    let poses: Vec<_> = three
        .components
        .iter()
        .map(|c| pcb_kicad::SemanticKiCadPose {
            component: c.id.clone(),
            position: c.position,
            rotation_degrees: 0.0,
        })
        .collect();
    let template = pcb_kicad::write_semantic_kicad_ladder_template(
        &three,
        &poses,
        &config,
        &directory.join("three-terminal-template"),
    )
    .unwrap();
    let declaration = pcb_kicad::load_declaration(&template.declaration).unwrap();
    for policy in [
        pcb_kicad::KiCadMultiTerminalRoutingPolicy::RootedStar,
        pcb_kicad::KiCadMultiTerminalRoutingPolicy::SharedCopperTree,
    ] {
        let parent = directory.join(format!("three-terminal-{policy:?}"));
        let materialized = pcb_kicad::materialize_rung(&declaration, 1, &parent).unwrap();
        let source = materialized.output_directory;
        let board = source.join(format!("{}.kicad_pcb", config.board_id));
        let mut routing = route.config.clone();
        routing.multi_terminal_routing = policy;
        routing.reachability_preflight = true;
        let candidate =
            pcb_kicad::route_materialized_connection(&board, "TOP_TO_BOTTOM", &routing).unwrap();
        assert_eq!(candidate.branches.len(), 2);
        assert!(
            candidate.supplemental_vias.is_empty(),
            "{policy:?}: {candidate:#?}"
        );
        assert!(
            candidate
                .supplemental_segments
                .iter()
                .any(|s| s.layer == "F.Cu")
        );
        assert!(
            candidate
                .supplemental_segments
                .iter()
                .any(|s| s.layer == "B.Cu")
        );
        let path = source.join("candidate.json");
        fs::write(&path, serde_json::to_vec_pretty(&candidate).unwrap()).unwrap();
        pcb_kicad::apply_route_candidate(&board, &path, &board).unwrap();
        assert!(
            pcb_kicad::verify_materialized_rung(&source, &config.board_id)
                .unwrap()
                .complete
        );
        assert!(source.join("preview.svg").is_file());
    }
    if std::env::var_os("PCB_MAKER_KEEP_TEST_ARTIFACTS").is_some() {
        println!(
            "retained pad-layer-contact artifacts: {}",
            directory.display()
        );
    } else {
        fs::remove_dir_all(directory).unwrap();
    }
}

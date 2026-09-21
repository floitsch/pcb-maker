// Copyright (C) 2026 Toit contributors.

use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn autonomous_native_routing_finds_the_only_allowed_via_window() {
    native_via_window(pcb_kicad::KiCadGridHeuristic::Geometric);
}

#[test]
fn obstacle_guided_native_routing_finds_the_only_allowed_via_window() {
    native_via_window(pcb_kicad::KiCadGridHeuristic::ObstacleDistances);
}

fn native_via_window(heuristic: pcb_kicad::KiCadGridHeuristic) {
    if !Command::new("kicad-cli")
        .arg("version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        eprintln!("skipping native via-keepout test: kicad-cli is unavailable");
        return;
    }
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mechanisms = repository.join("benchmarks/competitive/mechanisms");
    let problem: layout_trace_model::Problem = serde_json::from_str(
        &fs::read_to_string(mechanisms.join("problems/mandatory-layer-transition.json")).unwrap(),
    )
    .unwrap();
    let config: pcb_kicad::SemanticKiCadTemplateConfig = serde_json::from_str(
        &fs::read_to_string(mechanisms.join("mandatory-layer-transition-template.json")).unwrap(),
    )
    .unwrap();
    let poses: Vec<_> = problem
        .components
        .iter()
        .map(|component| pcb_kicad::SemanticKiCadPose {
            component: component.id.clone(),
            position: component.position,
            rotation_degrees: 0.0,
        })
        .collect();
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-native-via-keepout-{}-{}",
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
    let materialized =
        pcb_kicad::materialize_rung(&declaration, 1, &directory.join("source")).unwrap();
    let board = materialized
        .output_directory
        .join(format!("{}.kicad_pcb", config.board_id));
    let mut source = fs::read_to_string(&board).unwrap();
    let end = source.rfind(')').unwrap();
    // Native board is [9,9]..[41,27]. Both copper layers prohibit vias
    // everywhere except [24,17]..[26,19]; tracks remain permitted.
    let areas = [[9.0,9.0,24.0,27.0], [26.0,9.0,41.0,27.0],
        [24.0,9.0,26.0,17.0], [24.0,19.0,26.0,27.0]].iter().enumerate().map(|(index, [x1,y1,x2,y2])| format!(r#"
        (zone (net 0) (net_name "") (layers "F.Cu" "B.Cu")
          (uuid "aaaaaaaa-bbbb-cccc-dddd-{index:012}") (name "VIA_ONLY_{index}")
          (hatch edge 0.5) (connect_pads (clearance 0)) (min_thickness 0.25)
          (keepout (tracks allowed) (vias not_allowed) (pads allowed) (copperpour allowed) (footprints allowed))
          (fill (thermal_gap 0.3) (thermal_bridge_width 0.3))
          (polygon (pts (xy {x1} {y1}) (xy {x2} {y1}) (xy {x2} {y2}) (xy {x1} {y2}))))
        "#)).collect::<String>();
    source.insert_str(end, &areas);
    fs::write(&board, &source).unwrap();
    let config = pcb_kicad::KiCadSequentialRouterConfig {
        routing_portfolio: vec![pcb_kicad::KiCadGridRouteConfig {
            trace_width_mm: 0.4,
            clearance_mm: 0.3,
            via_size_mm: 0.8,
            via_drill_mm: 0.4,
            reachability_preflight: true,
            heuristic,
            ..pcb_kicad::KiCadGridRouteConfig::default()
        }],
        ..pcb_kicad::KiCadSequentialRouterConfig::default()
    };
    let result = pcb_kicad::route_kicad_board_sequentially(
        &materialized.output_directory,
        &declaration.board_id,
        &directory.join("routing"),
        &config,
    )
    .unwrap();
    assert_eq!(
        result.termination,
        pcb_kicad::KiCadSequentialRouterTermination::Complete,
        "{result:#?}"
    );
    assert_eq!(result.completed_connections, 1);
    assert_eq!(result.steps[0].attempts.len(), 1);
    let attempt = &result.steps[0].attempts[0];
    assert!(attempt.native_admission.as_ref().unwrap().complete);
    for checked in [
        &attempt.native_admission.as_ref().unwrap().directory,
        &result.result_directory,
    ] {
        let preview = fs::read_to_string(checked.join("preview.svg")).unwrap();
        assert!(preview.contains("<svg") && preview.contains("</svg>"));
        let metadata: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(checked.join("preview.json")).unwrap())
                .unwrap();
        assert!(metadata["render_error"].is_null());
        assert_eq!(metadata["verification"]["complete"], true);
    }
    let candidate = pcb_kicad::read_route_candidate(attempt.candidate.as_ref().unwrap()).unwrap();
    if heuristic == pcb_kicad::KiCadGridHeuristic::ObstacleDistances {
        assert!(candidate.heuristic_expansions.is_some_and(|work| work > 0));
        assert_eq!(candidate.reachability_expansions, 0);
    } else {
        assert!(candidate.heuristic_expansions.is_none());
    }
    assert_eq!(candidate.supplemental_vias.len(), 1);
    let via = &candidate.supplemental_vias[0];
    assert!((24.0..=26.0).contains(&via.at[0]) && (17.0..=19.0).contains(&via.at[1]));
    assert_eq!(fs::read_to_string(&board).unwrap(), source);
    if std::env::var_os("PCB_MAKER_KEEP_TEST_ARTIFACTS").is_some() {
        println!(
            "retained native via-keepout artifacts: {}",
            directory.display()
        );
    } else {
        fs::remove_dir_all(directory).unwrap();
    }
}

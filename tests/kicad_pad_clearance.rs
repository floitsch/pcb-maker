// Copyright (C) 2026 Toit contributors.

use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn autonomous_native_routing_respects_local_pad_clearance() {
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
        "pcb-maker-native-pad-clearance-{}-{}",
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
    // A mechanical pad near the direct route needs more space than the
    // routing portfolio's 0.3 mm default.
    let areas = r#"(footprint "Generated:ClearanceHole" (layer "F.Cu")
      (at 25 19.8) (uuid "aaaaaaaa-bbbb-cccc-dddd-000000000001")
      (attr board_only exclude_from_pos_files exclude_from_bom)
      (property "Reference" "H1" (at 0 2.5 0) (layer "F.Fab")
        (effects (font (size 0.8 0.8) (thickness 0.12))))
      (pad "" np_thru_hole circle (at 0 0) (size 1.2 1.2) (drill 1.2)
        (layers "*.Cu" "*.Mask") (clearance 1.85)
        (uuid "aaaaaaaa-bbbb-cccc-dddd-000000000002")))"#;
    source.insert_str(end, areas);
    fs::write(&board, &source).unwrap();
    fs::write(
        materialized
            .output_directory
            .join("Generated.pretty/ClearanceHole.kicad_mod"),
        areas.replace("Generated:ClearanceHole", "ClearanceHole"),
    )
    .unwrap();
    let config = pcb_kicad::KiCadSequentialRouterConfig {
        routing_portfolio: vec![pcb_kicad::KiCadGridRouteConfig {
            trace_width_mm: 0.4,
            clearance_mm: 0.3,
            via_size_mm: 0.8,
            via_drill_mm: 0.4,
            reachability_preflight: true,
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
    assert_eq!(candidate.supplemental_vias.len(), 1);
    assert_eq!(fs::read_to_string(&board).unwrap(), source);
    if std::env::var_os("PCB_MAKER_KEEP_TEST_ARTIFACTS").is_some() {
        println!(
            "retained native pad-clearance artifacts: {}",
            directory.display()
        );
    } else {
        fs::remove_dir_all(directory).unwrap();
    }
}

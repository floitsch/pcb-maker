// Copyright (C) 2026 Toit contributors.

use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn coarse_grid_uses_legal_gap_without_weakening_blocked_profile() {
    if !Command::new("kicad-cli")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipping native grid-alignment test: kicad-cli is unavailable");
        return;
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures = repo.join("benchmarks/esp32-pad-gaps");
    let problem: layout_trace_model::Problem =
        serde_json::from_str(&fs::read_to_string(fixtures.join("gap.problem.json")).unwrap())
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
        "pcb-maker-grid-alignment-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    for profile in ["open", "blocked"] {
        let config: pcb_kicad::SemanticKiCadTemplateConfig = serde_json::from_str(
            &fs::read_to_string(fixtures.join(format!("gap-{profile}/template.json"))).unwrap(),
        )
        .unwrap();
        let template = pcb_kicad::write_semantic_kicad_ladder_template(
            &problem,
            &poses,
            &config,
            &directory.join(profile).join("template"),
        )
        .unwrap();
        let declaration = pcb_kicad::load_declaration(&template.declaration).unwrap();
        let mut candidates = vec![];
        for (name, alignment) in [
            ("control", pcb_kicad::KiCadGridAlignment::BoardOrigin),
            (
                "aligned",
                pcb_kicad::KiCadGridAlignment::NearestNarrowPadGap,
            ),
        ] {
            let materialized =
                pcb_kicad::materialize_rung(&declaration, 1, &directory.join(profile).join(name))
                    .unwrap();
            let board = materialized
                .output_directory
                .join(format!("{}.kicad_pcb", config.board_id));
            let routing = pcb_kicad::KiCadGridRouteConfig {
                resolution_mm: 0.05,
                trace_width_mm: if profile == "open" { 0.15 } else { 0.25 },
                clearance_mm: if profile == "open" { 0.1 } else { 0.2 },
                via_size_mm: 0.7,
                via_drill_mm: 0.3,
                via_cost_mm: Some(2.0),
                reachability_preflight: true,
                grid_alignment: alignment,
                ..Default::default()
            };
            let candidate =
                pcb_kicad::route_materialized_connection(&board, "CROSS_ROW", &routing).unwrap();
            let path = materialized.output_directory.join("candidate.json");
            fs::write(&path, serde_json::to_string_pretty(&candidate).unwrap()).unwrap();
            pcb_kicad::apply_route_candidate(&board, &path, &board).unwrap();
            let statistics = pcb_kicad::inspect_kicad_board(&board).unwrap();
            let quality = pcb_kicad::route_candidate_quality(&candidate).unwrap();
            assert!(
                (quality.length_mm - statistics.physical_copper.physical_centerline_length_mm)
                    .abs()
                    < 1e-6,
                "candidate/native length mismatch for {profile}/{name}"
            );
            let native = pcb_kicad::verify_materialized_rung(
                &materialized.output_directory,
                &config.board_id,
            )
            .unwrap();
            assert!(native.complete, "{profile}/{name}: {native:#?}");
            assert!(materialized.output_directory.join("preview.svg").is_file());
            let preview: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(materialized.output_directory.join("preview.json")).unwrap(),
            )
            .unwrap();
            assert!(preview["render_error"].is_null());
            candidates.push(candidate);
        }
        let control = &candidates[0];
        let aligned = &candidates[1];
        if profile == "open" {
            let evidence = aligned.grid_alignment_evidence.as_ref().unwrap();
            assert_eq!(evidence.axis, 1);
            assert!((evidence.offset_mm - 0.025).abs() < 1e-9);
            let gap_x = config.board_origin_mm[0] + 6.0;
            let gap_y = config.board_origin_mm[1] + 4.0;
            assert!(
                aligned
                    .branches
                    .iter()
                    .any(|branch| branch.path.windows(2).any(|p| {
                        let dx = p[1].at[0] - p[0].at[0];
                        if dx.abs() < 1e-9 || p[0].layer != "F.Cu" || p[1].layer != "F.Cu" {
                            return false;
                        }
                        let t = (gap_x - p[0].at[0]) / dx;
                        (0.0..=1.0).contains(&t)
                            && (p[0].at[1] + t * (p[1].at[1] - p[0].at[1]) - gap_y).abs() < 0.01
                    }))
            );
            let before = pcb_kicad::route_candidate_quality(control).unwrap();
            let after = pcb_kicad::route_candidate_quality(aligned).unwrap();
            assert_eq!(after.vias, 0);
            assert!(after.length_mm + 0.8 < before.length_mm);
            assert!(aligned.expansions < control.expansions);
            assert!(
                aligned.grid_size.iter().product::<usize>()
                    <= control.grid_size.iter().product::<usize>()
            );
        } else {
            assert!(aligned.grid_alignment_evidence.is_none());
            assert_eq!(
                serde_json::to_value(&aligned.branches).unwrap(),
                serde_json::to_value(&control.branches).unwrap()
            );
            assert_eq!(aligned.grid_origin, control.grid_origin);
        }
    }
    if std::env::var_os("PCB_MAKER_KEEP_TEST_ARTIFACTS").is_some() {
        println!(
            "retained native grid-alignment artifacts: {}",
            directory.display()
        );
    } else {
        fs::remove_dir_all(directory).unwrap();
    }
}

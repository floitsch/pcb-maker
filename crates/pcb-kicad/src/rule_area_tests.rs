// Copyright (C) 2026 Toit contributors.

use super::*;

fn board(area: &str) -> String {
    format!(
        r#"(kicad_pcb
        (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
        (footprint "A" (at 2 10)
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/TARGET")))
        (footprint "B" (at 18 10)
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "B.Cu") (net "/TARGET")))
        {area})"#
    )
}

fn area(bounds: [f64; 4], tracks: bool, vias: bool) -> String {
    let [x1, y1, x2, y2] = bounds;
    format!(
        r#"(zone (layers "F.Cu" "B.Cu") (name "TEST_KEEPOUT")
        (keepout (tracks {}) (vias {}))
        (polygon (pts (xy {x1} {y1}) (xy {x2} {y1}) (xy {x2} {y2}) (xy {x1} {y2}))))"#,
        if tracks { "not_allowed" } else { "allowed" },
        if vias { "not_allowed" } else { "allowed" }
    )
}

#[test]
fn keepout_permissions_apply_independently_to_trace_and_via_checks() {
    for tracks in [false, true] {
        for vias in [false, true] {
            let pcb = parse(&board(&area([8.0, 8.0, 12.0, 12.0], tracks, vias))).unwrap();
            let model = KiCadRoutingModel::from_pcb(&pcb, "TARGET").unwrap();
            let grid = KiCadObstacleGrid::new(&model, 4.0, 0.0);
            for layer in 0..2 {
                assert_eq!(
                    grid.any_contains(&model, layer, [10.0, 10.0], 0.5, CopperQueryKind::Trace),
                    tracks
                );
                assert_eq!(
                    grid.any_contains(&model, layer, [10.0, 10.0], 0.5, CopperQueryKind::Via),
                    vias
                );
                assert_eq!(
                    grid.any_intersects_segment(&model, layer, [6.0, 10.0], [14.0, 10.0], 0.5),
                    tracks
                );
                assert_eq!(
                    route_segment_is_clear(
                        [6.0, 10.0],
                        [14.0, 10.0],
                        layer,
                        &model,
                        &KiCadGridRouteConfig::default()
                    ),
                    !tracks
                );
            }
        }
    }
}

#[test]
fn mandatory_layer_change_finds_a_via_window_and_rejects_a_fully_closed_window() {
    let directory = env::temp_dir().join(format!(
        "pcb-maker-via-window-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("board.kicad_pcb");
    let config = KiCadGridRouteConfig {
        reachability_preflight: true,
        ..KiCadGridRouteConfig::default()
    };
    fs::write(&path, board("")).unwrap();
    let unconstrained = route_materialized_connection(&path, "TARGET", &config).unwrap();
    assert_eq!(unconstrained.supplemental_vias.len(), 1);
    let exhausted = route_materialized_connection(
        &path,
        "TARGET",
        &KiCadGridRouteConfig {
            max_expansions: 1,
            ..config.clone()
        },
    )
    .unwrap_err();
    assert!(
        exhausted.contains("exhausted its search budget"),
        "{exhausted}"
    );
    assert_eq!(route_failure_expansions(&exhausted), Some(2));
    // An interior window keeps the old finish-side via outside the legal area.
    let contains_via = |via: &SupplementalVia| {
        (8.0..=10.0).contains(&via.at[0]) && (9.0..=11.0).contains(&via.at[1])
    };
    assert!(!unconstrained.supplemental_vias.iter().all(contains_via));
    let keepouts = [
        [0.0, 0.0, 8.0, 20.0],
        [10.0, 0.0, 20.0, 20.0],
        [8.0, 0.0, 10.0, 9.0],
        [8.0, 11.0, 10.0, 20.0],
    ]
    .map(|bounds| area(bounds, false, true))
    .join("\n");
    fs::write(&path, board(&keepouts)).unwrap();
    let model = KiCadRoutingModel::from_pcb(&parse(&board(&keepouts)).unwrap(), "TARGET").unwrap();
    assert!(!candidate_via_blockers(&unconstrained, &model).is_empty());
    let routed = route_materialized_connection(&path, "TARGET", &config).unwrap();
    assert_eq!(routed.supplemental_vias.len(), 1);
    assert!(routed.supplemental_vias.iter().all(contains_via));
    assert!(candidate_via_blockers(&routed, &model).is_empty());
    let selected: Vec<_> = (0..routed.branches.len()).collect();
    assert!(selected_geometry_blockers(&routed, &selected, &model).is_empty());
    fs::write(&path, board(&area([0.0, 0.0, 20.0, 20.0], false, true))).unwrap();
    let error = route_materialized_connection(&path, "TARGET", &config).unwrap_err();
    assert!(error.contains("reachability"), "{error}");
    assert!(
        error.contains("found no path on the routing grid"),
        "{error}"
    );
    assert!(!error.contains("exhausted"), "{error}");
    fs::remove_dir_all(directory).unwrap();
}

// Copyright (C) 2026 Toit contributors.

use super::*;

fn candidate(paths: Vec<Vec<([f64; 2], &str)>>) -> KiCadRouteCandidate {
    let mut segments = vec![];
    let branches = paths
        .into_iter()
        .map(|path| {
            for p in path.windows(2) {
                if p[0].1 == p[1].1 && p[0].0 != p[1].0 {
                    segments.push(SupplementalSegment {
                        connection: "SIGNAL".into(),
                        start: p[0].0,
                        end: p[1].0,
                        layer: p[0].1.into(),
                        width: 0.25,
                    });
                }
            }
            KiCadRouteBranch {
                start_terminal: path.first().unwrap().0,
                finish_terminal: path.last().unwrap().0,
                cost: 0,
                expansions: 0,
                path: path
                    .into_iter()
                    .map(|(at, layer)| KiCadRoutePoint {
                        at,
                        layer: layer.into(),
                    })
                    .collect(),
            }
        })
        .collect();
    KiCadRouteCandidate {
        schema_version: 2,
        connection: "SIGNAL".into(),
        router: "union-test".into(),
        config: KiCadGridRouteConfig::default(),
        terminals: vec![],
        grid_origin: [0.0, 0.0],
        grid_alignment_evidence: None,
        heuristic_expansions: None,
        grid_size: [1, 1],
        cost: 0,
        reachability_expansions: 0,
        expansions: 0,
        branches,
        tree_attachment_attempts: vec![],
        supplemental_segments: segments,
        supplemental_vias: vec![],
        footprint_placements: vec![],
        reference_placements: vec![],
    }
}

#[test]
fn adjacent_backtracks_have_physical_length_and_do_not_reverse_quality_ranking() {
    let retained = candidate(vec![vec![
        ([12.0, 14.0], "F.Cu"),
        ([20.025, 14.0], "F.Cu"),
        ([20.0, 14.0], "F.Cu"),
    ]]);
    let quality = route_candidate_quality(&retained).unwrap();
    assert!((quality.length_mm - 8.025).abs() < 1e-12);
    assert!((quality.stored_track_length_mm - 8.05).abs() < 1e-12);
    assert!((quality.overlapping_track_length_mm - 0.025).abs() < 1e-12);
    let original = serde_json::to_value(&retained).unwrap();
    route_candidate_quality(&retained).unwrap();
    assert_eq!(serde_json::to_value(&retained).unwrap(), original);

    let retraced = candidate(vec![vec![
        ([0.0, 0.0], "F.Cu"),
        ([8.025, 0.0], "F.Cu"),
        ([8.0, 0.0], "F.Cu"),
    ]]);
    let detour = candidate(vec![vec![
        ([0.0, 0.0], "F.Cu"),
        ([4.0, 0.35], "F.Cu"),
        ([8.0, 0.0], "F.Cu"),
    ]]);
    let a = route_candidate_quality(&retraced).unwrap();
    let b = route_candidate_quality(&detour).unwrap();
    assert!(a.stored_track_length_mm > b.stored_track_length_mm);
    assert!(a.length_mm < b.length_mm);
    assert!((a.length_mm - 8.025).abs() < 1e-12);
}

#[test]
fn all_short_collinear_walks_match_interval_union_regardless_of_branch_partition() {
    // Independent oracle: a continuous walk along one line covers exactly
    // [minimum, maximum]. Exercise repeated vertices, backtracks and loops.
    for code in 0..1024_usize {
        let coordinates: Vec<_> = (0..5).map(|i| ((code >> (2 * i)) & 3) as f64).collect();
        let lo = coordinates.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = coordinates
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        if lo == hi {
            continue;
        }
        for direction in [[1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [3.0, 2.0]] {
            for layer in ["F.Cu", "B.Cu"] {
                let path: Vec<_> = coordinates
                    .iter()
                    .map(|&x| ([-5.25 + x * direction[0], 7.5 + x * direction[1]], layer))
                    .collect();
                let walk = candidate(vec![path.clone()]);
                let independent = candidate(
                    path.windows(2)
                        .filter(|p| p[0] != p[1])
                        .map(|p| p.to_vec())
                        .collect(),
                );
                let expected =
                    (hi - lo) * (direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
                let actual = route_candidate_quality(&walk).unwrap();
                let split = route_candidate_quality(&independent).unwrap();
                assert!(
                    (actual.length_mm - expected).abs() < 1e-9,
                    "walk {code}, {direction:?}, {layer}"
                );
                assert!((actual.length_mm - split.length_mm).abs() < 1e-9);
                assert!(actual.overlapping_track_length_mm >= -1e-9);
            }
        }
    }
}

#[test]
fn ordinary_bends_keep_their_graph_and_opposite_layer_copper_stays_distinct() {
    let mut bend = candidate(vec![vec![
        ([0.0, 0.0], "F.Cu"),
        ([2.0, 0.0], "F.Cu"),
        ([2.0, 2.0], "F.Cu"),
    ]]);
    let result = normalize_selected_route_graph(&mut bend.branches, &[0]);
    assert_eq!(result.contact_points, 0);
    assert_eq!(result.inserted_points, 0);
    assert_eq!(route_candidate_quality(&bend).unwrap().length_mm, 4.0);

    let layered = candidate(vec![vec![
        ([0.0, 0.0], "F.Cu"),
        ([2.0, 0.0], "F.Cu"),
        ([2.0, 0.0], "B.Cu"),
        ([0.0, 0.0], "B.Cu"),
    ]]);
    let quality = route_candidate_quality(&layered).unwrap();
    assert_eq!(quality.length_mm, 4.0);
    assert_eq!(quality.vias, 1);
}

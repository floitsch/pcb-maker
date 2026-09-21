// Copyright (C) 2026 Toit contributors.

use super::*;

pub(super) fn plated_anchor(model: &KiCadRoutingModel, at: [f64; 2]) -> bool {
    model.terminal_pads.iter().any(|pad| {
        pad.plated_through_hole
            && pad.layers == [true, true]
            && distance_squared(pad.at, at) <= 1.0e-12
    })
}

pub(super) fn plated_copper_at(model: &KiCadRoutingModel, at: [f64; 2]) -> bool {
    model.terminal_pads.iter().any(|pad| {
        pad.plated_through_hole && pad.layers == [true, true] && pad.geometry.contains(at, 0.0)
    })
}

/// A plated pad can terminate a route on either copper layer without an
/// additional drilled via. Only remove endpoint transitions proven to be at
/// one plated pad spanning both layers; coincident separate SMD pads do not
/// establish a layer connection.
fn trim_endpoint_vias(path: &mut Vec<KiCadRoutePoint>, model: &KiCadRoutingModel) -> usize {
    let redundant = |a: &KiCadRoutePoint, b: &KiCadRoutePoint| {
        a.at == b.at
            && a.layer != b.layer
            && matches!(
                (copper_layer_index(&a.layer), copper_layer_index(&b.layer)),
                (Some(0), Some(1)) | (Some(1), Some(0))
            )
            && plated_anchor(model, a.at)
    };
    let mut removed = 0;
    while path.len() > 2 && redundant(&path[0], &path[1]) {
        path.remove(0);
        removed += 1;
    }
    while path.len() > 2 && redundant(&path[path.len() - 2], &path[path.len() - 1]) {
        path.pop();
        removed += 1;
    }
    removed
}

pub(super) fn normalize_endpoint_contacts(
    candidate: &mut KiCadRouteCandidate,
    model: &KiCadRoutingModel,
) -> Result<(), String> {
    let removed: usize = candidate
        .branches
        .iter_mut()
        .map(|branch| trim_endpoint_vias(&mut branch.path, model))
        .sum();
    if removed > 0 {
        let (segments, vias) = materialize_candidate_branches(
            &candidate.connection,
            &candidate.branches,
            &candidate.config,
        )?;
        candidate.supplemental_segments = segments;
        candidate.supplemental_vias = vias;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(pads: &str) -> KiCadRoutingModel {
        let pcb = parse(&format!(
            r#"(kicad_pcb
          (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts"))
          (footprint "J1" (at 2 2) {pads})
          (footprint "J2" (at 8 8)
            (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1)
              (layers "*.Cu" "*.Mask") (net "/SIGNAL"))))"#
        ))
        .unwrap();
        KiCadRoutingModel::from_pcb(&pcb, "SIGNAL").unwrap()
    }

    fn path() -> Vec<KiCadRoutePoint> {
        [
            ([2.0, 2.0], "F.Cu"),
            ([2.0, 2.0], "B.Cu"),
            ([5.0, 5.0], "B.Cu"),
            ([5.0, 5.0], "F.Cu"),
            ([8.0, 8.0], "F.Cu"),
            ([8.0, 8.0], "B.Cu"),
        ]
        .into_iter()
        .map(|(at, layer)| KiCadRoutePoint {
            at,
            layer: layer.into(),
        })
        .collect()
    }

    #[test]
    fn plated_endpoints_connect_layers_without_deleting_interior_vias() {
        let model = model(
            r#"(pad "1" thru_hole circle (at 0 0) (size 2 2)
          (drill 1) (layers "*.Cu" "*.Mask") (net "/SIGNAL"))"#,
        );
        let mut points = path();
        assert_eq!(trim_endpoint_vias(&mut points, &model), 2);
        assert_eq!(points.len(), 4);
        assert_eq!(points[0].layer, "B.Cu");
        assert_eq!(points[3].layer, "F.Cu");
        assert_eq!(points[1].at, points[2].at);
        assert_ne!(points[1].layer, points[2].layer);
        assert_eq!(trim_endpoint_vias(&mut points, &model), 0);
    }

    #[test]
    fn layer_union_and_unplated_holes_do_not_prove_a_pad_layer_connection() {
        for pads in [
            r#"(pad "1" smd circle (at 0 0) (size 2 2) (layers "F.Cu") (net "/SIGNAL"))
               (pad "2" smd circle (at 0 0) (size 2 2) (layers "B.Cu") (net "/SIGNAL"))"#,
            r#"(pad "1" np_thru_hole circle (at 0 0) (size 2 2) (drill 1)
               (layers "*.Cu" "*.Mask") (net "/SIGNAL"))"#,
            r#"(pad "1" thru_hole circle (at 0 0) (size 2 2)
               (layers "*.Cu" "*.Mask") (net "/SIGNAL"))"#,
            r#"(pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1)
               (layers "F.Cu") (net "/SIGNAL"))"#,
        ] {
            let mut points = path();
            assert_eq!(trim_endpoint_vias(&mut points, &model(pads)), 1);
            assert_eq!(points[0].at, points[1].at);
            assert_ne!(points[0].layer, points[1].layer);
        }
    }
}

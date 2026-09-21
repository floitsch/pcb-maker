// Copyright (C) 2026 Toit contributors.

use super::*;

/// One mandatory copper contact. Equal anchors do not connect copper layers.
/// A drilled plated pad is the only reason both layers can be alternatives
/// within one contact rather than two distinct routing goals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ElectricalTerminal {
    pub at: [f64; 2],
    pub layers: [bool; 2],
}

pub(super) fn plated_pad(pad: &Expr) -> bool {
    pad.children().get(2).and_then(Expr::atom) == Some("thru_hole")
        && pad.child("drill").is_some_and(|drill| {
            drill
                .children()
                .iter()
                .skip(1)
                .filter_map(Expr::atom)
                .filter_map(|value| value.parse::<f64>().ok())
                .any(|diameter| diameter.is_finite() && diameter > 0.0)
        })
}

pub(super) fn contact_layers(layers: [bool; 2], plated: bool) -> Vec<[bool; 2]> {
    if layers == [true, true] && !plated {
        vec![[true, false], [false, true]]
    } else {
        vec![layers]
    }
}

impl KiCadRoutingModel {
    pub(super) fn electrical_terminals(&self) -> Vec<ElectricalTerminal> {
        let mut result = Vec::new();
        for &at in &self.terminals {
            let layers = terminal_copper_layers_at(self, at).unwrap_or([false; 2]);
            result.extend(
                contact_layers(layers, pad_layer_contacts::plated_anchor(self, at))
                    .into_iter()
                    .map(|layers| ElectricalTerminal { at, layers }),
            );
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(back_pad: &str, copper: &str, remote: bool) -> String {
        format!(
            r#"(kicad_pcb
          (gr_rect (start 0 0) (end 10 6) (layer "Edge.Cuts"))
          (footprint "A" (at 2 2)
            (property "Reference" "A")
            (pad "1" smd circle (at 0 0) (size 1.2 1.2)
              (layers "F.Cu") (net "/SIGNAL")))
          (footprint "B" (at 2 2)
            (property "Reference" "B") {back_pad})
          {} {copper})"#,
            if remote {
                r#"(footprint "C" (at 8 2)
                  (property "Reference" "C")
                  (pad "1" smd circle (at 0 0) (size 1.2 1.2)
                    (layers "F.Cu") (net "/SIGNAL")))"#
            } else {
                ""
            },
        )
    }

    const BACK: &str = r#"(pad "1" smd circle (at 0 0) (size 1.2 1.2)
        (layers "B.Cu") (net "/SIGNAL"))"#;
    const PLATED: &str = r#"(pad "1" thru_hole circle (at 0 0) (size 1.2 1.2)
        (drill 0.4) (layers "*.Cu") (net "/SIGNAL"))"#;
    const FRONT_TRACE: &str = r#"(segment (start 2 2) (end 8 2) (width 0.2)
        (layer "F.Cu") (net "/SIGNAL"))"#;
    const VIA: &str = r#"(via (at 2 2) (size 0.6) (drill 0.3)
        (layers "F.Cu" "B.Cu") (net "/SIGNAL"))"#;

    #[test]
    fn coincident_smd_layers_are_mandatory_goals_while_plated_layers_are_alternatives() {
        for (back, expected) in [
            (BACK, vec![[true, false], [false, true], [true, false]]),
            (PLATED, vec![[true, true], [true, false]]),
        ] {
            let model =
                KiCadRoutingModel::from_pcb(&parse(&fixture(back, "", true)).unwrap(), "SIGNAL")
                    .unwrap();
            assert_eq!(
                model
                    .electrical_terminals()
                    .iter()
                    .map(|t| t.layers)
                    .collect::<Vec<_>>(),
                expected
            );
            let front = &model.electrical_terminals()[0];
            let options = terminal_access::copper_options(
                &model,
                front,
                [0.0; 2],
                [40, 24],
                &KiCadGridRouteConfig {
                    resolution_mm: 0.25,
                    ..Default::default()
                },
                |_| false,
            )
            .unwrap();
            assert!(options.iter().any(|p| p.layer == 0));
            assert_eq!(options.iter().any(|p| p.layer == 1), back == PLATED);
        }
    }

    #[test]
    fn coincident_smd_contacts_route_a_real_via_for_two_and_three_terminal_nets() {
        let directory = env::temp_dir().join(format!(
            "pcb-maker-electrical-terminals-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        for remote in [false, true] {
            fs::write(&board, fixture(BACK, "", remote)).unwrap();
            let inventory = inspect_kicad_routable_connections(&board).unwrap();
            assert_eq!(inventory.len(), 1);
            assert_eq!(
                inventory[0].distinct_pad_centers,
                if remote { 2 } else { 1 }
            );
            assert_eq!(
                inventory[0].electrical_terminal_count(),
                if remote { 3 } else { 2 }
            );
            let recorded = serde_json::to_value(&inventory[0]).unwrap();
            assert_eq!(
                recorded["electrical_terminal_count"],
                if remote { 3 } else { 2 }
            );
            for edge in [0.0, 0.15] {
                for policy in [
                    KiCadTerminalContactPolicy::PadAnchor,
                    KiCadTerminalContactPolicy::FirstPadContact,
                ] {
                    let candidate = route_materialized_connection(
                        &board,
                        "SIGNAL",
                        &KiCadGridRouteConfig {
                            resolution_mm: 0.25,
                            trace_width_mm: 0.2,
                            clearance_mm: 0.1,
                            edge_clearance_mm: edge,
                            via_size_mm: 0.6,
                            via_drill_mm: 0.3,
                            multi_terminal_routing:
                                KiCadMultiTerminalRoutingPolicy::SharedCopperTree,
                            tree_attachment_search: KiCadTreeAttachmentSearch::MultiSource,
                            tree_attachment_objective: KiCadTreeAttachmentObjective::RouterCost,
                            terminal_contact_policy: policy,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    assert_eq!(candidate.terminals.len(), if remote { 3 } else { 2 });
                    assert_eq!(candidate.branches.len(), if remote { 2 } else { 1 });
                    assert_eq!(candidate.supplemental_vias.len(), 1);
                    if !remote && edge == 0.15 {
                        assert!(candidate.supplemental_segments.is_empty());
                    }
                    let candidate_path = directory.join("candidate.json");
                    write_typed_json(&candidate_path, &candidate).unwrap();
                    let read = read_route_candidate(&candidate_path).unwrap();
                    let (segments, vias) = materialize_candidate_branches(
                        &read.connection,
                        &read.branches,
                        &read.config,
                    )
                    .unwrap();
                    assert_eq!(segments.len(), read.supplemental_segments.len());
                    assert_eq!(vias.len(), 1);
                    let output = directory.join("routed.kicad_pcb");
                    apply_route_candidate(&board, &candidate_path, &output).unwrap();
                    let imported = import_route_candidate_from_pcb(&output, "SIGNAL").unwrap();
                    assert_eq!(imported.terminals.len(), candidate.terminals.len());
                    assert_eq!(imported.supplemental_vias.len(), 1);
                    assert!(candidate.branches.iter().any(|branch| {
                        branch
                            .path
                            .windows(2)
                            .any(|pair| pair[0].layer != pair[1].layer)
                    }));
                }
            }
        }
        fs::write(&board, fixture(PLATED, "", false)).unwrap();
        assert!(
            inspect_kicad_routable_connections(&board)
                .unwrap()
                .is_empty()
        );
        let legacy: KiCadConnectionSummary =
            serde_json::from_str(r#"{"connection":"SIGNAL","distinct_pad_centers":2}"#).unwrap();
        assert_eq!(legacy.electrical_terminal_count(), 2);
        assert!(
            serde_json::to_value(legacy)
                .unwrap()
                .get("electrical_terminal_count")
                .is_none()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn copper_import_requires_the_real_via_between_coincident_smd_layers() {
        let without = parse(&fixture(BACK, FRONT_TRACE, true)).unwrap();
        assert!(import_route_candidate_from_expr(&without, "SIGNAL").is_err());
        let with = parse(&fixture(BACK, &format!("{FRONT_TRACE} {VIA}"), true)).unwrap();
        let imported = import_route_candidate_from_expr(&with, "SIGNAL").unwrap();
        assert_eq!(imported.terminals.len(), 3);
        assert_eq!(imported.supplemental_vias.len(), 1);
        let plated = parse(&fixture(PLATED, FRONT_TRACE, true)).unwrap();
        let imported = import_route_candidate_from_expr(&plated, "SIGNAL").unwrap();
        assert_eq!(imported.terminals.len(), 2);
        assert!(imported.supplemental_vias.is_empty());
    }
}

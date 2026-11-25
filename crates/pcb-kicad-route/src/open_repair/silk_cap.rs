// Copyright (C) 2026 Florian Loitsch.

//! A narrow proof for the KiCad 10.0.8 limit on immutable silkscreen findings.

use super::*;
use pcb_kicad_file::board_stats::is_rule_area;

const KIND: &str = "silk_overlap";

/// Without custom rules, KiCad 10.0.8's implicit silk clearance rule
/// matches only F/B silkscreen target layers. Its separate mask check can
/// see the added vias and remains fully checked by the caller.
/// https://github.com/KiCad/kicad-source-mirror/blob/10.0.8/pcbnew/drc/drc_engine.cpp#L190-L194
/// https://github.com/KiCad/kicad-source-mirror/blob/10.0.8/pcbnew/drc/drc_test_provider_silk_clearance.cpp#L237-L253
#[derive(PartialEq, Eq)]
pub(super) struct ImplicitSilkRules(Option<Vec<u8>>);

impl ImplicitSilkRules {
    pub(super) fn read(board: &Path) -> Option<Self> {
        match fs::symlink_metadata(board.with_extension("kicad_dru")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return None,
        }
        match fs::read(board.with_extension("kicad_pro")) {
            Ok(bytes) => Some(Self(Some(bytes))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(Self(None)),
            Err(_) => None,
        }
    }
}

pub(super) struct SilkCap {
    findings: Findings,
}

impl SilkCap {
    pub(super) fn prove(pcb: &Expr, report: &Value) -> Option<Self> {
        let findings = capped_silk(report)?;
        let mut forms = vec![pcb];
        let mut ids = BTreeMap::new();
        let mut silk = BTreeMap::new();
        while let Some(item) = forms.pop() {
            // The stackup describes layer materials. Its layer entries
            // are not graphical objects on those layers.
            if item.head() == Some("stackup") {
                continue;
            }
            forms.extend(item.children());
            let id = item_id(item);
            if let Some(id) = &id {
                *ids.entry(id.clone()).or_insert(0) += 1;
            }
            if item.head() == Some("zone") && !is_rule_area(item) && !copper_layers(item) {
                return None;
            }
            let mut layers = layer_names(item).filter(|layer| layer.ends_with(".SilkS"));
            let Some(layer) = layers.next() else { continue };
            // Only static artwork is covered. Text that depends on a
            // computed KiCad value could change without a PCB edit.
            if !matches!(layer, "F.SilkS" | "B.SilkS")
                || layers.next().is_some()
                || !matches!(
                    item.head(),
                    Some(
                        "property"
                            | "fp_text"
                            | "gr_text"
                            | "fp_text_box"
                            | "gr_text_box"
                            | "fp_line"
                            | "fp_arc"
                            | "fp_circle"
                            | "fp_rect"
                            | "fp_poly"
                            | "fp_curve"
                            | "gr_line"
                            | "gr_arc"
                            | "gr_circle"
                            | "gr_rect"
                            | "gr_poly"
                            | "gr_curve"
                    )
                )
                || contains_text_variable(item)
            {
                return None;
            }
            if let Some(id) = id {
                silk.insert(id, layer);
            }
        }
        for finding in report["violations"]
            .as_array()?
            .iter()
            .filter(|finding| finding["type"] == KIND)
        {
            let [first, second] = finding["items"].as_array()?.as_slice() else {
                return None;
            };
            let layer = |item: &Value| {
                let id = item["uuid"].as_str()?.to_ascii_lowercase();
                if ids.get(&id) != Some(&1) {
                    return None;
                }
                silk.get(&id).copied()
            };
            let same_face = layer(first).is_some_and(|face| layer(second) == Some(face));
            if !same_face || first["uuid"] == second["uuid"] {
                return None;
            }
        }
        Some(Self { findings })
    }

    pub(super) fn preserved(&self, report: &Value, additions: &[Expr]) -> bool {
        // Only ordinary top-level copper may be appended. The originating
        // PCB bytes, including all artwork, are otherwise untouched.
        additions
            .iter()
            .all(|item| route_item(item) && copper_layers(item))
            && capped_silk(report).is_some_and(|findings| findings == self.findings)
    }
}

fn contains_text_variable(item: &Expr) -> bool {
    match item {
        Expr::Atom(text) => text.contains("${") || text.contains("@{"),
        Expr::List(children) => children.iter().any(contains_text_variable),
    }
}

fn layer_names(item: &Expr) -> impl Iterator<Item = &str> {
    item.children()
        .iter()
        .filter(|form| matches!(form.head(), Some("layer" | "layers")))
        .flat_map(|form| form.children().iter().skip(1).filter_map(Expr::atom))
}

fn copper_layers(item: &Expr) -> bool {
    let mut layers = layer_names(item).peekable();
    layers.peek().is_some() && layers.all(|layer| layer.ends_with(".Cu"))
}

fn capped_silk(report: &Value) -> Option<Findings> {
    if report["kicad_version"] != "10.0.8"
        || report[CAPPED_KEY] == true
        || report["unconnected_items"].as_array()?.len() >= KICAD_UNCONNECTED_CAP
        || !finding_types_at_cap(report, "schematic_parity").is_empty()
        || copper_finding_types_at_cap(report) != [KIND]
    {
        return None;
    }
    let silk = report["violations"]
        .as_array()?
        .iter()
        .filter(|finding| finding["type"] == KIND)
        .cloned()
        .collect::<Vec<_>>();
    if silk
        .iter()
        .any(|finding| finding["severity"] != "warning" || finding["excluded"] == true)
    {
        return None;
    }
    finding_counts(&serde_json::json!({ "violations": silk }), "violations").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOARD: &str = r#"(kicad_pcb
        (setup (stackup (layer "F.SilkS" (type "Top Silk Screen"))
                        (layer "B.SilkS" (type "Bottom Silk Screen"))))
        (gr_line (layer "F.SilkS") (uuid "first"))
        (footprint "test" (fp_line (layer "F.SilkS") (uuid "second")))
        (zone (layers "*.Cu")))"#;

    fn report() -> Value {
        let finding = json!({
            "type": KIND, "severity": "warning",
            "items": [{"uuid":"first"}, {"uuid":"second"}],
        });
        json!({"kicad_version": "10.0.8", "violations": vec![finding; 199],
            "unconnected_items": [null], "schematic_parity": []})
    }

    #[test]
    fn invariant_cap_still_requires_exact_reported_silk_and_full_other_findings() {
        let baseline = report();
        let proof = SilkCap::prove(&parse(BOARD).unwrap(), &baseline).unwrap();
        let segment = parse(r#"(segment (layer "F.Cu"))"#).unwrap();
        let mut candidate = baseline.clone();
        candidate["unconnected_items"] = json!([]);
        assert!(proof.preserved(&candidate, std::slice::from_ref(&segment)));
        assert!(preserved_findings(&baseline, &candidate).unwrap());
        assert!(!preserved(&baseline, &candidate).unwrap());
        assert!(!proof.preserved(&candidate, &[parse("(gr_line (layer F.SilkS))").unwrap()]));

        candidate["violations"][0]["description"] = json!("changed");
        assert!(!proof.preserved(&candidate, std::slice::from_ref(&segment)));
        candidate["violations"][0] = baseline["violations"][0].clone();
        candidate["violations"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"silk_over_copper"}));
        assert!(proof.preserved(&candidate, std::slice::from_ref(&segment)));
        assert!(!preserved_findings(&baseline, &candidate).unwrap());
        candidate["violations"]
            .as_array_mut()
            .unwrap()
            .extend(vec![json!({"type":"silk_over_copper"}); 198]);
        assert!(!proof.preserved(&candidate, std::slice::from_ref(&segment)));
        candidate = baseline.clone();
        candidate["schematic_parity"] = json!(vec![json!({"type":"net_conflict"}); 199]);
        assert!(!proof.preserved(&candidate, std::slice::from_ref(&segment)));
        candidate = baseline;
        candidate["unconnected_items"] = json!(vec![Value::Null; KICAD_UNCONNECTED_CAP]);
        assert!(!proof.preserved(&candidate, &[segment]));
    }

    #[test]
    fn refuses_mixed_layers_movable_artwork_and_unproved_reports() {
        let baseline = report();
        for modified in [
            BOARD.replace("fp_line (layer \"F.SilkS\")", "fp_line (layer B.SilkS)"),
            BOARD.replace("fp_line (layer \"F.SilkS\")", "fp_line (layer F.Cu)"),
            BOARD.replace("(layers \"*.Cu\")", "(layers \"*.Cu\" \"F.SilkS\")"),
            BOARD.replace("(zone", "(gr_text \"${LENGTH}\" (layer B.SilkS)) (zone"),
            BOARD.replace("(zone", "(via (uuid first)) (zone"),
        ] {
            assert!(SilkCap::prove(&parse(&modified).unwrap(), &baseline).is_none());
        }
        let pcb = parse(BOARD).unwrap();
        for (key, value) in [("severity", json!("error")), ("excluded", json!(true))] {
            let mut modified = baseline.clone();
            modified["violations"][0][key] = value;
            assert!(SilkCap::prove(&pcb, &modified).is_none());
        }
        let mut modified = baseline;
        modified["kicad_version"] = json!("10.0.9");
        assert!(SilkCap::prove(&pcb, &modified).is_none());
    }
}

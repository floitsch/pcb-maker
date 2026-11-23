// Copyright (C) 2026 Florian Loitsch.

use super::*;

/// Native footprint-filter findings sharing the same PCB library ID and
/// symbol filter text. The filter text is preserved in KiCad's spelling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KiCadFootprintFilterMismatch {
    pub footprint_id: String,
    pub symbol_filters: String,
    pub references: Vec<String>,
    pub findings: usize,
}

pub(in crate::verdict) fn matches_on_board(
    report: &Value,
    board: &crate::Expr,
) -> Vec<KiCadFootprintFilterMismatch> {
    let findings: Vec<_> = report["schematic_parity"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|finding| {
            finding["type"].as_str() == Some("footprint_filters_mismatch")
                && matches!(finding["severity"].as_str(), Some("error" | "warning"))
        })
        .collect();
    if findings.is_empty() {
        return Vec::new();
    }

    let mut footprints = BTreeMap::new();
    for footprint in board
        .children()
        .iter()
        .filter(|item| matches!(item.head(), Some("footprint" | "module")))
    {
        if let Some(uuid) = crate::kicad_nets::footprint_uuid(footprint) {
            footprints
                .entry(uuid)
                .and_modify(|item| *item = None)
                .or_insert(Some(footprint));
        }
    }
    let fields = |finding: &Value| {
        let [item] = finding["items"].as_array()?.as_slice() else {
            return None;
        };
        let footprint = footprints.get(item["uuid"].as_str()?)?.as_ref()?;
        let reference = crate::footprint_reference(footprint)?;
        let item_reference = item["description"].as_str()?.strip_prefix("Footprint ")?;
        if reference != item_reference || reference.is_empty() {
            return None;
        }
        let footprint_id = footprint.children().get(1)?.atom()?;
        let (reported_id, filters) = finding["description"]
            .as_str()?
            .trim()
            .trim_end_matches('.')
            .rsplit_once(" doesn't match symbol's footprint filters (")?;
        let filters = filters.strip_suffix(')')?;
        (reported_id == footprint_id && !footprint_id.is_empty() && !filters.trim().is_empty())
            .then_some((footprint_id.to_string(), filters.to_string(), reference))
    };
    let mut groups: BTreeMap<_, (BTreeSet<_>, usize)> = BTreeMap::new();
    for (footprint_id, filters, reference) in findings.into_iter().filter_map(fields) {
        let (references, count) = groups.entry((footprint_id, filters)).or_default();
        references.insert(reference);
        *count += 1;
    }
    groups
        .into_iter()
        .map(|((footprint_id, symbol_filters), (references, findings))| {
            KiCadFootprintFilterMismatch {
                footprint_id,
                symbol_filters,
                references: references.into_iter().collect(),
                findings,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn groups_native_matches_by_footprint_and_filter_with_exact_board_identity() {
        let board = crate::parse(
            r#"(kicad_pcb
            (footprint "Custom:Open Jumper" (uuid "one") (property "Reference" "JP1"))
            (footprint "Custom:Open Jumper" (uuid "two") (property "Reference" "JP2"))
            (footprint "Other:SOIC" (uuid "three") (property "Reference" "U1")))"#,
        )
        .unwrap();
        let finding = |reference, uuid, id, filters| {
            json!({
                "type": "footprint_filters_mismatch",
                "severity": "error",
                "description": format!("{id} doesn't match symbol's footprint filters ({filters})"),
                "items": [{"uuid": uuid, "description": format!("Footprint {reference}")}],
            })
        };
        let report = json!({"schematic_parity": [
            finding("JP2", "two", "Custom:Open Jumper", "Jumper* SolderJumper*Open*"),
            finding("JP1", "one", "Custom:Open Jumper", "Jumper* SolderJumper*Open*"),
            finding("U1", "three", "Stale:Part", "QFN*"),
            finding("U2", "three", "Other:SOIC", "QFN*"),
            finding("U1", "gone", "Other:SOIC", "QFN*"),
            {"type": "footprint_symbol_mismatch", "severity": "error"},
        ]});
        assert_eq!(
            matches_on_board(&report, &board),
            vec![KiCadFootprintFilterMismatch {
                footprint_id: "Custom:Open Jumper".into(),
                symbol_filters: "Jumper* SolderJumper*Open*".into(),
                references: vec!["JP1".into(), "JP2".into()],
                findings: 2,
            }]
        );
        assert!(matches_on_board(&json!({"schematic_parity": []}), &board).is_empty());
    }
}

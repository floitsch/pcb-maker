// Copyright (C) 2026 Florian Loitsch.

use super::*;
use crate::local_pour_repair::check_rule_area;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadLocalPourRetry {
    pub pad: String,
    pub pad_uuid: String,
    pub at_mm: [f64; 2],
    pub layer: String,
    pub rule_area_uuid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_area_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_area_footprint: Option<String>,
    pub tracks_allowed: bool,
    pub zone_fill_allowed: bool,
    pub next_action: &'static str,
    pub retry_argv: Vec<String>,
}

pub(super) fn diagnose(
    result: &LoweredBoard,
    opens: &BTreeMap<String, Vec<KiCadNativePair>>,
    selected: &[String],
    files: &KiCadRunFiles,
) -> BTreeMap<String, Vec<KiCadLocalPourRetry>> {
    let pads = open_pad_findings::native_open_pads(&result.pads, opens, selected);
    let mut suggestions: BTreeMap<String, Vec<KiCadLocalPourRetry>> = BTreeMap::new();
    let Ok(layers) = LayerTable::from_pcb(&result.pcb) else {
        return suggestions;
    };
    if pads.is_empty() {
        return suggestions;
    }
    let items = open_pad_findings::board_items(&result.pcb);
    for (uuid, (net, pad)) in &pads {
        let Some(Some(item)) = items.get(uuid) else {
            continue;
        };
        if item.node.head() != Some("pad")
            || item.node.children().get(2).and_then(Expr::atom) != Some("smd")
            || suggestions
                .get(*net)
                .is_some_and(|entries| entries.len() >= 3)
            || pads
                .values()
                .filter(|(_, other)| other.reference == pad.reference && other.number == pad.number)
                .count()
                != 1
            || result
                .pcb
                .children()
                .iter()
                .filter(|owner| {
                    matches!(owner.head(), Some("footprint" | "module"))
                        && footprint_reference(owner).as_ref() == Some(&pad.reference)
                })
                .count()
                != 1
        {
            continue;
        }
        let Ok(mask) = layers.pad_mask(item.node) else {
            continue;
        };
        if mask.count_ones() != 1 {
            continue;
        }
        let Ok(rule) = check_rule_area(&result.pcb, &layers, mask, pad.at) else {
            continue;
        };
        let Some(rule_area_uuid) = footprint_uuid(rule.zone) else {
            continue;
        };
        let selector = format!("{}:{}", pad.reference, pad.number);
        let retry_argv = vec![
            "pcb-maker".into(),
            "repair-kicad-opens".into(),
            files.directory.display().to_string(),
            files.board_id.clone(),
            "--local-pour".into(),
            "<fresh-output-dir>".into(),
            "--pad".into(),
            selector.clone(),
        ];
        suggestions.entry((*net).clone()).or_default().push(KiCadLocalPourRetry {
            pad: selector, pad_uuid: uuid.clone(), at_mm: pad.at,
            layer: layers.names[mask.trailing_zeros() as usize].clone(), rule_area_uuid,
            rule_area_name: form_atom(rule.zone, "name", 1).map(str::to_owned),
            rule_area_footprint: rule.owner.and_then(footprint_reference),
            tracks_allowed: false, zone_fill_allowed: true,
            next_action: "If the design permits adding a local copper zone on this layer, try the listed command with a fresh output directory. KiCad will check the proposed connection in the stored and refilled board.",
            retry_argv,
        });
    }
    suggestions
}

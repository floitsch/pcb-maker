// Copyright (C) 2026 Florian Loitsch.

//! Retry guidance for a route blocked by unchanged input pad geometry.

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRetryGuidance {
    pub kind: &'static str,
    pub same_placement_and_rules_can_resolve: bool,
    pub blocking_findings: BTreeMap<String, usize>,
    pub footprints: Vec<String>,
    pub examples: Vec<KiCadFixedPadFinding>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadFixedPadFinding {
    #[serde(rename = "type")]
    pub kind: String,
    pub description: String,
    pub items: Vec<KiCadFixedPadItem>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadFixedPadItem {
    pub footprint: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_mm: Option<[f64; 2]>,
}

pub(super) fn assess(
    facts: &KiCadRunFacts,
    status: KiCadRunStatus,
    counts: &BTreeMap<String, usize>,
    findings: &[&DrcFinding],
) -> Option<KiCadRetryGuidance> {
    let native = facts.native.as_ref()?;
    if facts.command != "route-kicad-board"
        || !facts.routes
        || status != KiCadRunStatus::Incomplete
        || facts.boards_mixed().is_some()
        || !facts.unplaced.is_empty()
        || !facts.constraints_missed.is_empty()
        || facts.pin_swapped
        || native.unconnected != 0
        || facts.routed_connections < facts.routable_connections
        || facts.unconnected_terminals != 0
        || facts.internal_violations != 0
        || !parity::problems(native).is_empty()
        || native
            .errors
            .values()
            .any(|count| *count >= KICAD_FINDING_CAP)
    {
        return None;
    }
    let mut covered = BTreeMap::new();
    for finding in findings {
        if !ELECTRICAL_KINDS.contains(&finding.kind.as_str())
            || !finding
                .item_descriptions
                .iter()
                .all(|item| is_pad_item(item))
        {
            return None;
        }
        *covered.entry(finding.kind.clone()).or_default() += 1;
    }
    if &covered != counts {
        return None;
    }
    let mut footprints = kept_parts_of(findings, facts.input.as_ref())?;
    footprints.sort_by(|a, b| crate::canonical_order::natural_cmp(a, b));
    let examples = findings
        .iter()
        .take(FINDING_ACTIONS)
        .map(|finding| KiCadFixedPadFinding {
            kind: finding.kind.clone(),
            description: finding.description.clone(),
            items: finding
                .items
                .iter()
                .zip(&finding.item_descriptions)
                .filter_map(|(item, description)| {
                    Some(KiCadFixedPadItem {
                        footprint: pad_footprint(description)?.into(),
                        description: description.clone(),
                        anchor_mm: item.located(),
                    })
                })
                .collect(),
        })
        .collect();
    Some(KiCadRetryGuidance {
        kind: "fixed_input_pads",
        same_placement_and_rules_can_resolve: false,
        blocking_findings: covered,
        footprints,
        examples,
    })
}

impl KiCadRetryGuidance {
    pub(super) fn action(&self) -> String {
        let limit = self.footprints.len().min(8);
        let names = self.footprints[..limit].join(", ");
        let more = self.footprints.len() - limit;
        let parts = if more == 0 {
            names
        } else {
            format!("{names} and {more} more")
        };
        format!(
            "Correct the input pad or hole geometry at {parts} before routing again. Where a finding spans two footprints, move one to meet the rule. Every blocking DRC finding ({}) is between fixed input pads. Another route with the same placement and rules cannot resolve these findings. See retry_guidance for the items KiCad reported.",
            counts(&self.blocking_findings),
        )
    }
}

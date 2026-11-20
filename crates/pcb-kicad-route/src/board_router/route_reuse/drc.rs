// Copyright (C) 2026 Florian Loitsch.

//! Exact native DRC delta for route-reuse admission and diagnostics.

use super::{BTreeMap, Path, Serialize, read_typed_json};
use serde_json::Value;

const EXAMPLE_LIMIT: usize = 6;
const ITEM_LIMIT: usize = 3;
const TEXT_LIMIT: usize = 256;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRouteReuseFinding {
    #[serde(rename = "type")]
    pub kind: String,
    pub severity: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub excluded: bool,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    pub items: Vec<KiCadRouteReuseFindingItem>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRouteReuseFindingItem {
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    /// The position KiCad reports for this item, such as a track's start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_mm: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

#[derive(Default)]
pub(super) struct Delta {
    pub counts: BTreeMap<String, usize>,
    pub examples: Vec<KiCadRouteReuseFinding>,
}

fn findings<'a>(report: &'a Value, field: &str) -> Result<&'a [Value], String> {
    report[field]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("KiCad's reuse check did not report {field}"))
}

pub(super) fn parity_counts(report: &Value) -> Result<BTreeMap<String, usize>, String> {
    let mut counts = BTreeMap::new();
    for item in findings(report, "schematic_parity")? {
        let kind = item["type"].as_str().unwrap_or("unknown");
        *counts.entry(kind.to_string()).or_default() += 1;
    }
    Ok(counts)
}

fn finding_key(finding: &Value) -> String {
    // Match the full native finding, including item identity, position,
    // severity and exclusion, with item order ignored.
    let mut copy = finding.clone();
    if let Some(items) = copy["items"].as_array_mut() {
        items.sort_by_cached_key(Value::to_string);
    }
    copy.to_string()
}

/// Returns the count of each new KiCad finding of any severity. Missing
/// parity evidence, an incomplete route, or an incomplete DRC rejects reuse.
pub fn check_route_reuse(
    baseline_drc: &Path,
    trial_drc: &Path,
) -> Result<BTreeMap<String, usize>, String> {
    compare_drc(baseline_drc, trial_drc).map(|delta| delta.counts)
}

pub(super) fn compare_drc(baseline_drc: &Path, trial_drc: &Path) -> Result<Delta, String> {
    compare_reports(
        &read_typed_json(baseline_drc)?,
        &read_typed_json(trial_drc)?,
    )
}

fn compare_reports(baseline: &Value, trial: &Value) -> Result<Delta, String> {
    for report in [baseline, trial] {
        let severities = findings(report, "included_severities")?;
        if ["error", "warning", "exclusion"]
            .iter()
            .any(|severity| !severities.iter().any(|known| known == *severity))
        {
            return Err("KiCad's reuse check did not cover all DRC severities".into());
        }
    }
    if !findings(trial, "unconnected_items")?.is_empty() {
        return Err("KiCad found open connections in the seeded trial".into());
    }
    if !findings(baseline, "schematic_parity")?.is_empty()
        || !findings(trial, "schematic_parity")?.is_empty()
    {
        return Err(
            "KiCad found schematic parity issues in the current input or seeded trial".into(),
        );
    }
    let mut counts = BTreeMap::<String, usize>::new();
    for item in findings(baseline, "violations")? {
        *counts.entry(finding_key(item)).or_default() += 1;
    }
    let metric = trial["coordinate_units"] == "mm";
    let mut delta = Delta::default();
    let mut extra_examples = Vec::new();
    let mut seen = BTreeMap::<&str, usize>::new();
    for item in findings(trial, "violations")? {
        let kind = item["type"]
            .as_str()
            .ok_or("KiCad DRC finding has no type")?;
        let occurrences = seen.entry(kind).or_default();
        *occurrences += 1;
        if *occurrences >= 199 {
            return Err(format!(
                "KiCad's seeded trial reached the DRC finding cap for {kind}"
            ));
        }
        if let Some(count) = counts
            .get_mut(&finding_key(item))
            .filter(|count| **count != 0)
        {
            *count -= 1;
            continue;
        }
        *delta.counts.entry(kind.to_string()).or_default() += 1;
        if delta.examples.len() == EXAMPLE_LIMIT {
            continue;
        }
        // Give each reported type a representative before using the spare
        // slots for additional findings of the same type.
        let samples = if delta.examples.iter().any(|example| example.kind == kind) {
            &mut extra_examples
        } else {
            &mut delta.examples
        };
        if samples.len() < EXAMPLE_LIMIT
            && let Some(example) = finding_example(kind, item, metric)
        {
            samples.push(example);
        }
    }
    delta.examples.extend(
        extra_examples
            .into_iter()
            .take(EXAMPLE_LIMIT - delta.examples.len()),
    );
    Ok(delta)
}

fn short_text(text: &str) -> String {
    let mut chars = text.chars();
    let mut shown: String = chars.by_ref().take(TEXT_LIMIT).collect();
    if chars.next().is_some() {
        shown.push('…');
    }
    shown
}

fn finding_example(kind: &str, finding: &Value, metric: bool) -> Option<KiCadRouteReuseFinding> {
    let description = finding["description"].as_str()?;
    let named_rule = finding["rule"].as_str().or_else(|| {
        // KiCad puts simple named rules at the end of the description.
        // The full description also preserves other native rule formats.
        description.split_once("(rule '")?.1.strip_suffix("')")
    });
    Some(KiCadRouteReuseFinding {
        kind: kind.into(),
        severity: short_text(finding["severity"].as_str()?),
        excluded: finding["excluded"].as_bool().unwrap_or(false),
        description: short_text(description),
        rule: named_rule.filter(|rule| !rule.is_empty()).map(short_text),
        items: finding["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item_example(item, metric))
            .take(ITEM_LIMIT)
            .collect(),
    })
}

fn item_example(item: &Value, metric: bool) -> Option<KiCadRouteReuseFindingItem> {
    let description = item["description"].as_str()?;
    let anchor_mm = if metric {
        item["pos"]["x"]
            .as_f64()
            .zip(item["pos"]["y"].as_f64())
            .map(|(x, y)| [x, y])
            .filter(|at| !(description.starts_with("Zone ") && *at == [0.0, 0.0]))
    } else {
        None
    };
    Some(KiCadRouteReuseFindingItem {
        description: short_text(description),
        net: reported_net(description).map(short_text),
        anchor_mm,
        uuid: item["uuid"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(short_text),
    })
}

fn reported_net(description: &str) -> Option<&str> {
    let net = if let Some(pad) = description.strip_prefix("Pad ") {
        pad.split_once(" [")?.1.rsplit_once("] of ")?.0
    } else {
        let copper = [
            "Track [",
            "Arc [",
            "Via [",
            "Through via [",
            "Blind via [",
            "Buried via [",
            "Micro via [",
            "Zone [",
        ]
        .into_iter()
        .find_map(|kind| description.strip_prefix(kind))?;
        copper.rsplit_once("] on ")?.0
    };
    (!net.is_empty() && net != "<no net>").then_some(net)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(violations: Vec<Value>) -> Value {
        json!({
            "coordinate_units": "mm",
            "included_severities": ["error", "warning", "exclusion"],
            "unconnected_items": [],
            "schematic_parity": [],
            "violations": violations,
        })
    }

    #[test]
    fn the_preflight_counts_native_parity_types() {
        let mut baseline = report(vec![]);
        assert!(parity_counts(&baseline).unwrap().is_empty());
        baseline["schematic_parity"] = json!([
            {"type": "missing_footprint"},
            {"type": "net_conflict"},
            {"type": "net_conflict"},
        ]);
        assert_eq!(
            parity_counts(&baseline).unwrap(),
            BTreeMap::from([("missing_footprint".into(), 1), ("net_conflict".into(), 2)])
        );
        let trial = report(vec![]);
        assert!(compare_reports(&baseline, &trial).is_err());
        assert!(parity_counts(&json!({})).is_err());
    }

    #[test]
    fn examples_share_exact_multiset_subtraction_with_counts() {
        let mut warning = json!({
            "type": "items_not_allowed",
            "severity": "warning",
            "description": "Items not allowed (rule 'BACK track width restriction')",
            "items": [
                {"description": "Track [BACK] on F.Cu, length 0.5958 mm",
                 "uuid": "track", "pos": {"x": 92.655, "y": 109.025}},
                {"description": "Rule Area [comment] on F.Cu",
                 "uuid": "area", "pos": {"x": 47.635, "y": 20.32}},
            ]
        });
        let baseline = report(vec![warning.clone()]);
        warning["items"].as_array_mut().unwrap().reverse();
        assert!(
            compare_reports(&baseline, &report(vec![warning.clone()]))
                .unwrap()
                .examples
                .is_empty()
        );
        let delta = compare_reports(&baseline, &report(vec![warning.clone(), warning])).unwrap();
        assert_eq!(delta.counts["items_not_allowed"], 1);
        assert_eq!(delta.examples.len(), 1);
        assert_eq!(delta.examples[0].severity, "warning");
        assert_eq!(
            delta.examples[0].rule.as_deref(),
            Some("BACK track width restriction")
        );
        assert_eq!(
            delta.examples[0].items[1].anchor_mm,
            Some([92.655, 109.025])
        );
        assert_eq!(delta.examples[0].items[1].net.as_deref(), Some("BACK"));
        assert_eq!(delta.examples[0].items[0].net, None);
    }

    #[test]
    fn evidence_is_bounded_and_prefers_distinct_finding_types() {
        let item = json!({"description": "Pad 3 [N[2]] of U1 on F.Cu", "pos": {"x": 1, "y": 2}});
        let warning = json!({
            "type": "track_width", "severity": "warning", "description": "a".repeat(300),
            "items": vec![item; 5],
        });
        let error = json!({
            "type": "clearance", "severity": "error", "description": "Copper clearance",
            "items": [{"description": "Zone [<no net>] on F.Cu", "pos": {"x": 0, "y": 0}}],
        });
        let mut violations = vec![warning; 12];
        violations.push(error);
        let delta = compare_reports(&report(vec![]), &report(violations)).unwrap();
        assert_eq!(delta.counts["track_width"], 12);
        assert_eq!(delta.counts["clearance"], 1);
        assert_eq!(delta.examples.len(), EXAMPLE_LIMIT);
        assert_eq!(delta.examples[0].items.len(), ITEM_LIMIT);
        assert_eq!(delta.examples[0].items[0].net.as_deref(), Some("N[2]"));
        assert_eq!(
            delta.examples[0].description.chars().count(),
            TEXT_LIMIT + 1
        );
        assert_eq!(delta.examples[1].kind, "clearance");
        assert_eq!(delta.examples[1].items[0].anchor_mm, None);
    }
}

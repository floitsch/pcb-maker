// Copyright (C) 2026 Florian Loitsch.

//! Keep native netclass-clearance warnings visible in routing verdicts.

use super::{drc_item, footprint_clearance};
use crate::Expr;
use crate::kicad_nets::{footprint_uuid, item_net, net_codes};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum Scope {
    TrackOrVia,
    PadPair,
    Other,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadClassClearanceWarnings {
    pub class: String,
    pub required_mm: f64,
    scope: Scope,
    pub count: usize,
    pub examples: Vec<KiCadClearanceExample>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadClearanceExample {
    pub measured_mm: f64,
    pub items: [KiCadClearanceItem; 2],
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadClearanceItem {
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_mm: Option<[f64; 2]>,
}

fn rule(text: &str) -> Option<(&str, &str, f64, f64)> {
    let clause = super::finding_rule_clause(text)?;
    let (rule, actual) = clause
        .strip_prefix("netclass '")?
        .rsplit_once("; actual ")?;
    let (class, required) = rule.rsplit_once("' clearance ")?;
    if class.is_empty() {
        return None;
    }
    let actual_mm = footprint_clearance::millimetres(actual)?;
    let required_mm = footprint_clearance::millimetres(required)?;
    // Rounded native output can show the same value for a reported pair.
    (actual_mm <= required_mm).then_some((class, required, actual_mm, required_mm))
}

pub(super) fn collect(board: &Expr, report: &Value) -> Vec<KiCadClassClearanceWarnings> {
    let candidates = report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|finding| {
            finding["type"] == "clearance"
                && finding["severity"] == "warning"
                && finding["excluded"] != true
        })
        .filter_map(|finding| {
            let [a, b] = finding["items"].as_array()?.as_slice() else {
                return None;
            };
            let detail = rule(finding["description"].as_str()?)?;
            Some((finding, [a, b], detail))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Vec::new();
    }
    let codes = net_codes(board);
    let mut objects = BTreeMap::new();
    let mut add = |item: &Expr, kind| {
        if let Some(uuid) = footprint_uuid(item) {
            let info = Some((kind, item_net(item, &codes).map(str::to_owned)));
            objects
                .entry(uuid.to_ascii_lowercase())
                .and_modify(|entry| *entry = None)
                .or_insert(info);
        }
    };
    for item in board.children() {
        match item.head() {
            Some("segment" | "arc" | "via") => add(item, Scope::TrackOrVia),
            Some("footprint" | "module") => {
                for pad in item
                    .children()
                    .iter()
                    .filter(|child| child.head() == Some("pad"))
                {
                    add(pad, Scope::PadPair);
                }
            }
            _ => {}
        }
    }
    let mut groups = BTreeMap::<_, KiCadClassClearanceWarnings>::new();
    for (finding, items, (class, required, measured_mm, required_mm)) in candidates {
        let found = items.map(|item| drc_item(item, finding));
        let names = found.each_ref().map(|item| {
            item.uuid
                .as_ref()
                .and_then(|uuid| objects.get(&uuid.to_ascii_lowercase())?.as_ref())
        });
        let scope = if names
            .iter()
            .flatten()
            .any(|(kind, _)| *kind == Scope::TrackOrVia)
        {
            Scope::TrackOrVia
        } else if names
            .iter()
            .all(|entry| entry.is_some_and(|(kind, _)| *kind == Scope::PadPair))
        {
            Scope::PadPair
        } else {
            Scope::Other
        };
        let group =
            groups
                .entry((scope, class, required))
                .or_insert_with(|| KiCadClassClearanceWarnings {
                    class: class.into(),
                    required_mm,
                    scope,
                    count: 0,
                    examples: Vec::new(),
                });
        let example = KiCadClearanceExample {
            measured_mm,
            items: std::array::from_fn(|i| {
                let item = &found[i];
                let description = items[i]["description"].as_str().unwrap_or_default();
                KiCadClearanceItem {
                    description: description.into(),
                    uuid: item.uuid.clone(),
                    net: names[i]
                        .and_then(|(_, net)| net.clone())
                        .or_else(|| super::item_net(description)),
                    at_mm: (report["coordinate_units"] == "mm")
                        .then_some(item.at)
                        .flatten(),
                }
            }),
        };
        group.count += 1;
        group.examples.push(example);
        group
            .examples
            .sort_by(|a, b| a.measured_mm.total_cmp(&b.measured_mm));
        group.examples.truncate(3);
    }
    groups.into_values().collect()
}

pub(super) fn actions(warnings: &[KiCadClassClearanceWarnings]) -> Vec<String> {
    [
        (Scope::TrackOrVia, "involving tracks, arcs or vias", "Inspect the named copper and its pad exits for a route that meets the configured clearance."),
        (Scope::PadPair, "between pads", "Review the named pad geometry and actual part poses against the intended footprint and placement."),
        (Scope::Other, "with other or unresolved item geometry", "Review the native item descriptions and locations against the configured clearance."),
    ]
    .into_iter()
    .filter_map(|(scope, geometry, guidance)| {
        let groups: Vec<_> = warnings.iter().filter(|group| group.scope == scope).collect();
        let worst = groups.iter().min_by(|a, b| {
            let shortfall = |group: &KiCadClassClearanceWarnings| group.examples[0].measured_mm - group.required_mm;
            shortfall(a).total_cmp(&shortfall(b))
        })?;
        let count: usize = groups.iter().map(|group| group.count).sum();
        Some(format!(
            "KiCad reported {count} netclass clearance warnings {geometry}. The largest reported shortfall is for {} at {:.4} mm against {:.4} mm. {guidance} See class_clearance_warnings and drc.json.",
            worst.class, worst.examples[0].measured_mm, worst.required_mm,
        ))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;
    use serde_json::json;

    #[test]
    fn groups_only_active_native_warnings_with_exact_copper_witnesses() {
        let board = parse(
            r#"(kicad_pcb (segment (net "DATA+") (uuid track))
               (footprint "J" (pad "1" (net "GND") (uuid p1))
                              (pad "2" (net "DATA-") (uuid p2))))"#,
        )
        .unwrap();
        let item = |uuid, description| {
            json!({
                "uuid":uuid, "description": description, "pos": {"x":3.0, "y":4.0}
            })
        };
        let track = item("track", "Track [DATA+] on B.Cu");
        let pad1 = item("p1", "Pad 1 [GND] of J1 on B.Cu");
        let pad2 = item("p2", "Pad 2 [DATA-] of J1 on B.Cu");
        let warning = |items, severity, excluded| {
            json!({
                "type": "clearance", "description":
                "Clearance violation (netclass 'USB(D)' clearance 400 µm; actual 102.6 µm) (B.Cu)",
                "severity": severity, "excluded": excluded, "items": items
            })
        };
        let report = json!({"coordinate_units":"mm","violations":[
            warning([pad1.clone(),track.clone()], "warning", false),
            warning([pad2.clone(),pad1.clone()], "warning", false),
            warning([track.clone(),pad2.clone()], "warning", true),
            warning([pad2,track],"error",false),
        ]});
        let groups = collect(&board, &report);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].scope, Scope::TrackOrVia);
        assert_eq!(groups[1].scope, Scope::PadPair);
        assert!(groups.iter().all(|group| group.count == 1));
        let routed = &groups[0];
        assert_eq!(routed.class, "USB(D)");
        assert_eq!(routed.required_mm, 0.4);
        assert!((routed.examples[0].measured_mm - 0.1026).abs() < 1e-10);
        assert_eq!(routed.examples[0].items[1].uuid.as_deref(), Some("track"));
        assert_eq!(routed.examples[0].items[1].net.as_deref(), Some("DATA+"));
        assert_eq!(routed.examples[0].items[1].at_mm, Some([3.0, 4.0]));
        assert_eq!(actions(&groups).len(), 2);
    }

    #[test]
    fn ignores_non_class_rules_and_reports_native_rounded_equality() {
        let no_class = "Clearance violation (rule 'launch' clearance 0.4000 mm; actual 0.2000 mm)";
        assert!(rule(no_class).is_none());
        let class = "Clearance violation (netclass 'USB' clearance 0.4000 mm; actual 0.4000 mm)";
        assert_eq!(rule(class), Some(("USB", "0.4000 mm", 0.4, 0.4)));
    }
}

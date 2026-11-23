// Copyright (C) 2026 Florian Loitsch.

use super::*;

const MAX_SAMPLES: usize = 8;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadOpenPadFindings {
    pub matched_findings: usize,
    pub samples: Vec<KiCadOpenPadFinding>,
    pub next_actions: Vec<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadOpenPadFinding {
    pub reference: String,
    pub pad: String,
    pub pad_uuid: String,
    pub pad_at_mm: [f64; 2],
    pub kind: String,
    pub severity: String,
    pub description: String,
    pub counterpart_relationship: &'static str,
    pub counterpart: KiCadOpenPadCounterpart,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadOpenPadCounterpart {
    pub uuid: String,
    pub description: String,
    /// The position KiCad reports for this item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_at_mm: Option<[f64; 2]>,
    pub board_item: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
}

pub(super) struct BoardItem<'a> {
    pub(super) node: &'a Expr,
    pub(super) footprint: Option<&'a Expr>,
}

pub(super) fn board_items(pcb: &Expr) -> BTreeMap<String, Option<BoardItem<'_>>> {
    let mut items = BTreeMap::new();
    for node in pcb.children() {
        let footprint = matches!(node.head(), Some("footprint" | "module")).then_some(node);
        let children = footprint.into_iter().flat_map(Expr::children);
        for item in std::iter::once(node).chain(children) {
            if let Some(uuid) = footprint_uuid(item) {
                items
                    .entry(uuid.to_ascii_lowercase())
                    .and_modify(|entry| *entry = None)
                    .or_insert_with(|| {
                        Some(BoardItem {
                            node: item,
                            footprint,
                        })
                    });
            }
        }
    }
    items
}

pub(super) fn native_open_pads<'a>(
    pads: &'a [BoardPad],
    opens: &BTreeMap<String, Vec<KiCadNativePair>>,
    selected: &'a [String],
) -> BTreeMap<String, (&'a String, &'a BoardPad)> {
    if !selected.iter().any(|net| opens.contains_key(net)) {
        return BTreeMap::new();
    }
    let mut by_uuid = BTreeMap::new();
    for pad in pads {
        if let Some(uuid) = &pad.uuid {
            by_uuid
                .entry(uuid.to_ascii_lowercase())
                .and_modify(|entry| *entry = None)
                .or_insert(Some(pad));
        }
    }
    let mut open_pads = BTreeMap::new();
    for net in selected {
        for pair in opens
            .get(net)
            .into_iter()
            .flatten()
            .filter(|pair| pair.retained)
        {
            for item in &pair.items {
                let Some(uuid) = item.uuid.as_ref().map(|id| id.to_ascii_lowercase()) else {
                    continue;
                };
                let Some(Some(pad)) = by_uuid.get(&uuid) else {
                    continue;
                };
                if pad.net.as_ref() == Some(net)
                    && !pad.reference.is_empty()
                    && !pad.number.is_empty()
                    && distance_squared(item.at, pad.at) < 1.0e-6
                {
                    open_pads.insert(uuid, (net, *pad));
                }
            }
        }
    }
    open_pads
}

pub(super) fn diagnose(
    pcb: &Expr,
    pads: &[BoardPad],
    drc: &serde_json::Value,
    opens: &BTreeMap<String, Vec<KiCadNativePair>>,
    selected: &[String],
) -> BTreeMap<String, KiCadOpenPadFindings> {
    let open_pads = native_open_pads(pads, opens, selected);
    if open_pads.is_empty() {
        return BTreeMap::new();
    }
    let items = board_items(pcb);
    let mut matched: BTreeMap<String, Vec<KiCadOpenPadFinding>> = BTreeMap::new();
    for row in drc["violations"].as_array().into_iter().flatten() {
        let Some(severity @ ("error" | "warning")) = row["severity"].as_str() else {
            continue;
        };
        if row["excluded"].as_bool() == Some(true)
            || !matches!(row["type"].as_str(), Some("shorting_items" | "clearance"))
        {
            continue;
        }
        let finding = crate::verdict::drc_finding(row);
        let ([a, b], [label_a, label_b]) = (
            finding.items.as_slice(),
            finding.item_descriptions.as_slice(),
        ) else {
            continue;
        };
        for (endpoint, other, label) in [(a, b, label_b), (b, a, label_a)] {
            let Some(uuid) = endpoint.uuid.as_ref().map(|id| id.to_ascii_lowercase()) else {
                continue;
            };
            let Some((net, pad)) = open_pads.get(&uuid) else {
                continue;
            };
            let Some(Some(found)) = items.get(&uuid) else {
                continue;
            };
            if found.node.head() != Some("pad") {
                continue;
            }
            if !endpoint
                .located()
                .is_some_and(|at| distance_squared(at, pad.at) < 1.0e-6)
            {
                continue;
            }
            let Some(other_uuid) = other
                .uuid
                .as_ref()
                .map(|id| id.to_ascii_lowercase())
                .filter(|id| id != &uuid)
            else {
                continue;
            };
            let Some(Some(counterpart)) = items.get(&other_uuid) else {
                continue;
            };
            let counterpart_relationship = match counterpart.footprint {
                Some(owner)
                    if found
                        .footprint
                        .is_some_and(|footprint| std::ptr::eq(owner, footprint)) =>
                {
                    "same_footprint"
                }
                Some(_) => "different_footprint",
                None => "board_copper",
            };
            matched
                .entry((*net).clone())
                .or_default()
                .push(KiCadOpenPadFinding {
                    reference: pad.reference.clone(),
                    pad: pad.number.clone(),
                    pad_uuid: uuid,
                    pad_at_mm: pad.at,
                    kind: finding.kind.clone(),
                    severity: severity.into(),
                    description: finding.description.clone(),
                    counterpart_relationship,
                    counterpart: KiCadOpenPadCounterpart {
                        uuid: other_uuid,
                        description: label.clone(),
                        reported_at_mm: other.located(),
                        board_item: counterpart.node.head().unwrap_or_default().into(),
                        footprint: counterpart.footprint.and_then(footprint_reference),
                        layer: form_atom(counterpart.node, "layer", 1).map(str::to_string),
                    },
                });
        }
    }
    matched.into_iter().map(|(net, mut samples)| {
        samples.sort_by(|a, b| {
            let priority = |item: &KiCadOpenPadFinding| (item.kind != "shorting_items", item.severity != "error");
            priority(a).cmp(&priority(b)).then(a.pad_uuid.cmp(&b.pad_uuid))
                .then(a.counterpart.uuid.cmp(&b.counterpart.uuid)).then(a.description.cmp(&b.description))
        });
        let matched_findings = samples.len();
        let next_actions = [
            ("same_footprint", "Review the named pad's net assignment and the local copper in that footprint's source land pattern."),
            ("different_footprint", "Inspect the current relative footprint poses and the source geometry of the named copper in KiCad."),
            ("board_copper", "Inspect the named pad and board copper in KiCad and resolve the reported finding before retrying this connection."),
        ].into_iter().filter_map(|(relationship, action)| samples.iter()
            .any(|sample| sample.counterpart_relationship == relationship).then_some(action)).collect();
        samples.truncate(MAX_SAMPLES);
        (net, KiCadOpenPadFindings { matched_findings, samples, next_actions })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const BOARD: &str = r#"(kicad_pcb
        (footprint "LED" (property "Reference" "D1") (at 10 10)
          (pad "1" smd rect (at 0 0) (net "data") (uuid "open"))
          (pad "2" smd rect (at 1 0) (net "data") (uuid "other-pad")))
        (footprint "Antenna" (property "Reference" "AE1") (at 11 10)
          (fp_poly (layer "F.Cu") (uuid "polygon"))))"#;

    fn report() -> Value {
        let pad = |id, x| json!({"description": "Pad on D1 [data]", "pos": {"x": x, "y": 10}, "uuid": id});
        let copper = json!({"description": "Polygon [<no net>] of AE1 on F.Cu", "pos": {"x": 11, "y": 10}, "uuid": "polygon"});
        json!({
            "unconnected_items": [{"severity": "error", "items": [pad("open", 10), pad("remote", 15)]}],
            "violations": [
                {"type": "shorting_items", "description": "Items shorting two nets", "severity": "error", "items": [copper, pad("open", 10)]},
                {"type": "clearance", "description": "Unrelated pad clearance", "severity": "error", "items": [copper, pad("other-pad", 11)]},
            ]
        })
    }

    fn inspect(board: &str, drc: &Value) -> BTreeMap<String, KiCadOpenPadFindings> {
        let pcb = parse(board).unwrap();
        let names = NetNames::of(&pcb);
        let pads = board_pads(&pcb, &names).unwrap();
        let (opens, _) = native_pairs_by_net(Some(drc), &names);
        diagnose(&pcb, &pads, drc, &opens, &["data".into()])
    }

    #[test]
    fn open_pad_finding_matches_physical_uuid_and_names_footprint_copper() {
        let result = inspect(BOARD, &report());
        let findings = &result["data"];
        assert_eq!(findings.matched_findings, 1);
        let evidence = &findings.samples[0];
        assert_eq!(evidence.pad, "1");
        assert_eq!(evidence.pad_uuid, "open");
        assert_eq!(evidence.kind, "shorting_items");
        assert_eq!(evidence.counterpart_relationship, "different_footprint");
        assert_eq!(evidence.counterpart.footprint.as_deref(), Some("AE1"));
        assert_eq!(evidence.counterpart.board_item, "fp_poly");
        assert_eq!(evidence.counterpart.layer.as_deref(), Some("F.Cu"));
        assert_eq!(evidence.counterpart.reported_at_mm, Some([11.0, 10.0]));
    }

    #[test]
    fn open_pad_finding_advice_for_copper_on_the_same_footprint_uses_source_geometry() {
        let board = r#"(kicad_pcb
            (footprint "LED" (property "Reference" "D1") (at 10 10)
              (pad "1" smd rect (at 0 0) (net "data") (uuid "open"))
              (fp_poly (layer "F.Cu") (uuid "polygon"))))"#;
        let result = inspect(board, &report());
        let found = &result["data"];
        assert_eq!(found.samples[0].counterpart_relationship, "same_footprint");
        assert_eq!(
            found.samples[0].counterpart.footprint.as_deref(),
            Some("D1")
        );
        assert_eq!(found.next_actions.len(), 1);
        assert!(found.next_actions[0].contains("source land pattern"));
    }

    #[test]
    fn open_pad_finding_omits_stale_excluded_or_unresolved_evidence() {
        let mut stale = report();
        stale["violations"][0]["items"][1]["pos"]["x"] = json!(10.01);
        assert!(inspect(BOARD, &stale).is_empty());
        let mut excluded = report();
        excluded["unconnected_items"][0]["excluded"] = json!(true);
        assert!(inspect(BOARD, &excluded).is_empty());
        let mut missing = report();
        missing["violations"][0]["items"][0]["uuid"] = json!("absent");
        assert!(inspect(BOARD, &missing).is_empty());
    }
}

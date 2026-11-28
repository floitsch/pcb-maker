// Copyright (C) 2026 Florian Loitsch.

//! Findings whose named items are arbitrary representatives.
//!
//! KiCad's `connection_width` check measures the merged copper of one net
//! on one layer. Its finding carries the rule, the limit, the measured
//! width and the layer in the description; the two items it names are the
//! first two of an unordered set of that net's items near the neck, and
//! which two come first changes with the path of the checked file (KiCad
//! 10.0.8: the same board bytes under directories of another name length
//! name other tracks of the same joint, with every other field equal).
//! Two checks of the same copper therefore agree on such a finding without
//! agreeing on its items.
//!
//! `net_witness_neutral` rewrites a report for comparison: a finding of a
//! rule in `ONE_NET_RULES` whose items all resolve, by UUID, to copper of
//! one net of the board keeps its type, severity, exclusion and
//! description and names the net once per item instead of the items. A
//! finding with an unknown item, an item without a net or items of two
//! nets stays as KiCad wrote it. What the neutral form no longer tells
//! apart: two necks of the same net on the same layer with the same
//! measured width (KiCad prints 0.1 um) at different places.

use pcb_kicad_file::kicad_nets::{footprint_uuid, item_net, net_codes};
use pcb_kicad_file::kicad_sexpr::Expr;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The rule types whose finding is about one net's merged copper and
/// names arbitrary items of it.
const ONE_NET_RULES: [&str; 1] = ["connection_width"];

const FINDINGS: [&str; 3] = ["violations", "unconnected_items", "schematic_parity"];

/// The net of every copper item with a unique identity: tracks, vias,
/// zones and the footprints' pads. `None` when two items share a UUID.
fn copper_nets(board: &Expr) -> Option<BTreeMap<String, String>> {
    let codes = net_codes(board);
    let mut nets = BTreeMap::new();
    let mut add = |item: &Expr| -> Option<()> {
        let (Some(uuid), Some(net)) = (footprint_uuid(item), item_net(item, &codes)) else {
            return Some(());
        };
        if uuid.is_empty() || net.is_empty() {
            return Some(());
        }
        nets.insert(uuid.to_ascii_lowercase(), net.to_owned())
            .is_none()
            .then_some(())
    };
    for item in board.children() {
        match item.head() {
            Some("segment" | "arc" | "via" | "zone") => add(item)?,
            Some("footprint") => {
                for pad in item.children() {
                    if pad.head() == Some("pad") {
                        add(pad)?;
                    }
                }
            }
            _ => {}
        }
    }
    Some(nets)
}

/// The report with the items of one-net findings replaced by their net
/// (see the module's text). Everything else is copied.
pub fn net_witness_neutral(board: &Expr, report: &Value) -> Value {
    let mut neutral = report.clone();
    let Some(nets) = copper_nets(board) else {
        return neutral;
    };
    for field in FINDINGS {
        let Some(findings) = neutral[field].as_array_mut() else {
            continue;
        };
        for finding in findings {
            if !finding["type"]
                .as_str()
                .is_some_and(|kind| ONE_NET_RULES.contains(&kind))
            {
                continue;
            }
            let Some(items) = finding["items"]
                .as_array()
                .filter(|items| !items.is_empty())
            else {
                continue;
            };
            let named: Option<Vec<&String>> = items
                .iter()
                .map(|item| nets.get(&item["uuid"].as_str()?.to_ascii_lowercase()))
                .collect();
            let Some(named) = named else {
                continue;
            };
            if named.iter().any(|net| *net != named[0]) {
                continue;
            }
            finding["items"] = json!(
                named
                    .iter()
                    .map(|net| json!({ "net": net }))
                    .collect::<Vec<_>>()
            );
        }
    }
    neutral
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_kicad_file::kicad_sexpr::parse;

    fn board() -> Expr {
        parse(
            r#"(kicad_pcb
            (segment (start 1 1) (end 2 1) (width 0.09) (layer "F.Cu") (net "+3V3") (uuid "a"))
            (segment (start 2 1) (end 2 2) (width 0.09) (layer "F.Cu") (net "+3V3") (uuid "b"))
            (via (at 2 2) (size 0.35) (drill 0.2) (layers "F.Cu" "B.Cu") (net "+3V3") (uuid "c"))
            (segment (start 5 1) (end 6 1) (width 0.09) (layer "F.Cu") (net "GND") (uuid "g"))
            (zone (net "+3V3") (layer "F.Cu") (uuid "z"))
            (footprint "R" (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "+3V3") (uuid "p"))
                           (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (uuid "free"))))"#,
        )
        .unwrap()
    }

    fn finding(kind: &str, width: &str, items: [&str; 2]) -> Value {
        json!({
            "type": kind, "severity": "warning",
            "description": format!("Copper connection too narrow (board setup constraints minimum connection width 0.0900 mm; actual {width} mm) (F.Cu)"),
            "items": items.map(|uuid| json!({"uuid": uuid, "description": uuid, "pos": {"x": 1.0, "y": 1.0}})),
        })
    }

    fn report(findings: Vec<Value>) -> Value {
        json!({"violations": findings, "unconnected_items": [], "schematic_parity": []})
    }

    fn neutral(findings: Vec<Value>) -> Value {
        net_witness_neutral(&board(), &report(findings))
    }

    #[test]
    fn other_items_of_the_same_net_are_the_same_width_finding() {
        let first = neutral(vec![finding("connection_width", "0.0463", ["a", "b"])]);
        assert_ne!(
            first,
            report(vec![finding("connection_width", "0.0463", ["a", "b"])])
        );
        for other in [["b", "c"], ["c", "A"], ["z", "p"]] {
            assert_eq!(
                first,
                neutral(vec![finding("connection_width", "0.0463", other)])
            );
        }
    }

    #[test]
    fn the_measurement_the_net_the_rule_and_the_count_still_tell_findings_apart() {
        let first = neutral(vec![finding("connection_width", "0.0463", ["a", "b"])]);
        // Another measured width, another net.
        assert_ne!(
            first,
            neutral(vec![finding("connection_width", "0.0696", ["a", "b"])])
        );
        assert_ne!(
            first,
            neutral(vec![finding("connection_width", "0.0463", ["g", "g"])])
        );
        // Another severity.
        let mut error = finding("connection_width", "0.0463", ["b", "c"]);
        error["severity"] = json!("error");
        assert_ne!(first, neutral(vec![error]));
        // One finding is not two.
        assert_ne!(
            first,
            neutral(vec![
                finding("connection_width", "0.0463", ["a", "b"]),
                finding("connection_width", "0.0463", ["b", "c"])
            ])
        );
    }

    #[test]
    fn findings_outside_the_property_stay_as_kicad_wrote_them() {
        // Another rule, items of two nets, an unknown item, an item without a net.
        for raw in [
            finding("clearance", "0.0463", ["a", "b"]),
            finding("connection_width", "0.0463", ["a", "g"]),
            finding("connection_width", "0.0463", ["a", "unknown"]),
            finding("connection_width", "0.0463", ["a", "free"]),
        ] {
            assert_eq!(neutral(vec![raw.clone()]), report(vec![raw]));
        }
        // Two items with one identity: nothing is rewritten.
        let twice = parse(
            r#"(kicad_pcb
            (segment (start 1 1) (end 2 1) (width 0.09) (layer "F.Cu") (net "N") (uuid "a"))
            (segment (start 2 1) (end 2 2) (width 0.09) (layer "F.Cu") (net "N") (uuid "a")))"#,
        )
        .unwrap();
        let raw = report(vec![finding("connection_width", "0.0463", ["a", "a"])]);
        assert_eq!(net_witness_neutral(&twice, &raw), raw);
    }
}

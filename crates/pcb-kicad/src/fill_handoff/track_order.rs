// Copyright (C) 2026 Florian Loitsch.

//! Reproduce native width and pair diagnostics through track ordering.

use super::{FindingCounts, counts, keys, same_metadata};
use pcb_kicad_file::kicad_nets::{footprint_uuid, item_net, net_codes};
use pcb_kicad_file::kicad_sexpr::{Expr, form_atom, form_xy};
use pcb_kicad_route::board_router::LayerTable;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn track(item: &Expr) -> bool {
    matches!(item.head(), Some("segment" | "arc" | "via"))
}

fn index(pcb: &Expr) -> Option<BTreeMap<String, &Expr>> {
    let mut tracks = BTreeMap::new();
    for item in pcb.children().iter().filter(|item| track(item)) {
        let id = footprint_uuid(item)?.to_ascii_lowercase();
        if id.is_empty() || tracks.insert(id, item).is_some() {
            return None;
        }
    }
    Some(tracks)
}

/// Adopt the writer's ordering of complete original track and via forms.
/// Native serialization supplies only the order, through a bijection of
/// UUIDs and item kinds. Other forms stay in their original slots.
pub(super) fn reorder(pcb: &mut Expr, native: &Expr) -> Option<usize> {
    index(native)?;
    let mut originals = index(pcb)?;
    let ordered = native
        .children()
        .iter()
        .filter(|item| track(item))
        .map(|item| {
            let id = footprint_uuid(item)?.to_ascii_lowercase();
            let original = originals.remove(&id)?;
            (original.head() == item.head()).then(|| original.clone())
        })
        .collect::<Option<Vec<_>>>()?;
    if !originals.is_empty() {
        return None;
    }
    let Expr::List(items) = pcb else {
        return None;
    };
    let mut changed = 0;
    for (slot, original) in items.iter_mut().filter(|item| track(item)).zip(ordered) {
        if *slot != original {
            *slot = original;
            changed += 1;
        }
    }
    (changed > 0).then_some(changed)
}

struct Copper<'a> {
    tracks: BTreeMap<String, &'a Expr>,
    nets: BTreeMap<String, String>,
    layers: LayerTable,
}

impl Copper<'_> {
    fn local_contact(&self, track: &Expr, item: &Value, layer: &str) -> Option<Value> {
        let requested = self.layers.index(layer)?;
        if track.head() == Some("via") {
            let top = self.layers.index(form_atom(track, "layers", 1)?)?;
            let bottom = self.layers.index(form_atom(track, "layers", 2)?)?;
            if !(top.min(bottom)..=top.max(bottom)).contains(&requested) {
                return None;
            }
        } else if form_atom(track, "layer", 1)? != layer {
            return None;
        }
        let nm = |value: f64| (value * 1.0e6).round() as i64;
        let point = [item["pos"]["x"].as_f64()?, item["pos"]["y"].as_f64()?].map(nm);
        let shape_points = ["at", "start", "mid", "end"]
            .into_iter()
            .filter_map(|name| form_xy(track, name).ok());
        if !shape_points
            .into_iter()
            .any(|position| position.map(nm) == point)
        {
            return None;
        }
        Some(json!({"net": item_net(track, &self.nets)?, "contact_nm": point}))
    }

    fn measurement(&self, kind: &str, mut finding: Value) -> Option<String> {
        let description = finding["description"].as_str()?;
        let [a, b] = finding["items"].as_array()?.as_slice() else {
            return None;
        };
        let route = |item: &Value| {
            self.tracks
                .get(&item["uuid"].as_str()?.to_ascii_lowercase())
                .copied()
        };
        let mut witnesses = match kind {
            "diff_pair_uncoupled_length_too_long" => {
                let pair = [route(a)?, route(b)?];
                if pair.iter().any(|item| item.head() == Some("via")) {
                    return None;
                }
                let nets = [
                    item_net(pair[0], &self.nets)?,
                    item_net(pair[1], &self.nets)?,
                ];
                if nets[0] == nets[1] || nets.contains(&"") {
                    return None;
                }
                nets.map(|net| json!({"net": net}))
            }
            "connection_width" => {
                let (_, layer) = description.strip_suffix(')')?.rsplit_once(" (")?;
                let [a_track, b_track] = [route(a), route(b)];
                // Keep the other physical anchor as KiCad reports it.
                match (a_track, b_track) {
                    (Some(track), None) => [self.local_contact(track, a, layer)?, b.clone()],
                    (None, Some(track)) => [self.local_contact(track, b, layer)?, a.clone()],
                    _ => return None,
                }
            }
            _ => return None,
        };
        witnesses.sort_by_cached_key(Value::to_string);
        finding["items"] = json!(witnesses);
        Some(finding.to_string())
    }

    fn unmatched(&self, ours: &FindingCounts, other: &FindingCounts) -> Option<FindingCounts> {
        let mut measurements = BTreeMap::new();
        for ((kind, raw), count) in ours {
            let excess = count.saturating_sub(
                other
                    .get(&(kind.clone(), raw.clone()))
                    .copied()
                    .unwrap_or_default(),
            );
            if excess == 0 {
                continue;
            }
            let measured = self.measurement(kind, serde_json::from_str(raw).ok()?)?;
            *measurements.entry((kind.clone(), measured)).or_default() += excess;
        }
        Some(measurements)
    }
}

/// A raw comparison already identified a mismatch. Restrict ordering
/// replay to the two measured-rule witnesses seen to depend on order.
/// The original and ordered measurements keep the complete rule text,
/// value and severity. All other findings remain subject to exact checks.
pub(super) fn same_measurements(pcb: &Expr, before: &Value, after: &Value) -> bool {
    if same_metadata(before, before, after).is_err()
        || [before, after].into_iter().any(|report| {
            !counts(report).possibly_capped_types.is_empty()
                || report[crate::outline_pieces::CAPPED_KEY] == true
        })
        || keys(before, "schematic_parity") != keys(after, "schematic_parity")
        || before["unconnected_items"].as_array().map(Vec::len)
            != after["unconnected_items"].as_array().map(Vec::len)
    {
        return false;
    }
    let [first, second] = [keys(before, "violations"), keys(after, "violations")];
    if first == second {
        return true;
    }
    let (Some(tracks), Ok(layers)) = (index(pcb), LayerTable::from_pcb(pcb)) else {
        return false;
    };
    let copper = Copper {
        tracks,
        nets: net_codes(pcb),
        layers,
    };
    copper
        .unmatched(&first, &second)
        .is_some_and(|expected| copper.unmatched(&second, &first).as_ref() == Some(&expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_kicad_file::kicad_sexpr::parse;

    #[test]
    fn only_complete_track_forms_can_change_order() {
        let original = parse(
            r#"(kicad_pcb
          (segment (uuid "a") (width 0.1))
          (footprint "R1" (at 1 2))
          (via (uuid "b") (size 0.5)))"#,
        )
        .unwrap();
        let native = parse(
            r#"(kicad_pcb
          (via (uuid "b") (size 0.8))
          (segment (uuid "a") (width 0.2)))"#,
        )
        .unwrap();
        let mut ordered = original.clone();
        assert_eq!(reorder(&mut ordered, &native), Some(2));
        assert_eq!(
            ordered.children(),
            &[
                original.children()[0].clone(),
                original.children()[3].clone(),
                original.children()[2].clone(),
                original.children()[1].clone()
            ]
        );
        for ambiguous in [
            r#"(kicad_pcb (via (uuid "b")) (via (uuid "b")))"#,
            r#"(kicad_pcb (via (uuid "b")) (via (uuid "a")))"#,
            r#"(kicad_pcb (via (uuid "b")))"#,
        ] {
            let mut untouched = original.clone();
            assert_eq!(reorder(&mut untouched, &parse(ambiguous).unwrap()), None);
            assert_eq!(untouched, original);
        }
    }

    fn report(kind: &str, description: &str, a: Value, b: Value) -> Value {
        json!({"violations":[{"type":kind,"severity":"error","description":description,"items":[a,b]}],"schematic_parity":[],"unconnected_items":[]})
    }

    #[test]
    fn a_pair_measurement_keeps_both_nets_and_its_exact_rule_and_value() {
        let board = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
          (net 1 "P") (net 2 "N") (net 3 "OTHER")
          (segment (uuid "p1") (net 1) (layer "F.Cu"))
          (segment (uuid "p2") (net 1) (layer "B.Cu"))
          (segment (uuid "n") (net 2) (layer "F.Cu"))
          (segment (uuid "other") (net 3) (layer "F.Cu")))"#,
        )
        .unwrap();
        let issue = |a, b, length| {
            report(
                "diff_pair_uncoupled_length_too_long",
                length,
                json!({"uuid":a}),
                json!({"uuid":b}),
            )
        };
        let original = issue("p1", "n", "USB limit 2 mm; actual 4.2500 mm");
        assert!(same_measurements(
            &board,
            &original,
            &issue("p2", "n", "USB limit 2 mm; actual 4.2500 mm")
        ));
        assert!(!same_measurements(
            &board,
            &original,
            &issue("other", "n", "USB limit 2 mm; actual 4.2500 mm")
        ));
        assert!(!same_measurements(
            &board,
            &original,
            &issue("p2", "n", "USB limit 2 mm; actual 4.7500 mm")
        ));
    }

    #[test]
    fn local_width_preserves_the_layer_contact_and_other_anchor() {
        let board = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (4 "In1.Cu" power) (31 "B.Cu" signal))
          (net 1 "CC2")
          (segment (uuid "track") (net 1) (layer "B.Cu") (start 1 2) (end 2 2))
          (via (uuid "via") (net 1) (layers "F.Cu" "B.Cu") (at 1 2))
          (via blind (uuid "blind") (net 1) (layers "F.Cu" "In1.Cu") (at 1 2))
          (via (uuid "moved") (net 1) (layers "F.Cu" "B.Cu") (at 1 3)))"#,
        )
        .unwrap();
        let issue = |uuid, pad, y| {
            report(
                "connection_width",
                "min 0.1000 mm; actual 0.0165 mm (B.Cu)",
                json!({"uuid":uuid,"pos":{"x":1,"y":y}}),
                json!({"uuid":pad}),
            )
        };
        let original = issue("track", "pad", 2);
        assert!(same_measurements(
            &board,
            &original,
            &issue("via", "pad", 2)
        ));
        assert!(!same_measurements(
            &board,
            &original,
            &issue("blind", "pad", 2)
        ));
        assert!(!same_measurements(
            &board,
            &original,
            &issue("moved", "pad", 3)
        ));
        assert!(!same_measurements(
            &board,
            &original,
            &issue("via", "other_pad", 2)
        ));
    }
}

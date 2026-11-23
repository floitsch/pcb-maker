// Copyright (C) 2026 Florian Loitsch.

//! Resolve a transient DRC teardrop endpoint to kept copper it touches.

use super::*;
use pcb_router::geometry::{distance, point_in_polygon};

pub(super) struct Witness<'a> {
    pub at: [f64; 2],
    pub layer: &'a str,
}

pub(super) fn witness<'a>(item: &'a Value, net: &str) -> Option<Witness<'a>> {
    let layer = item["description"]
        .as_str()?
        .strip_prefix("Teardrop [")?
        .strip_prefix(unescape(net).as_ref())?
        .strip_prefix("] on ")?;
    let at = [item["pos"]["x"].as_f64()?, item["pos"]["y"].as_f64()?];
    (layer.ends_with(".Cu") && at.iter().all(|value| value.is_finite()))
        .then_some(Witness { at, layer })
}

#[derive(Serialize)]
pub struct TeardropContact {
    pub witness_mm: [f64; 2],
    pub layer: String,
    pub kept_item: String,
    pub contact_mm: [f64; 2],
}

/// Native refill can recreate a teardrop with another UUID. The DRC position
/// is the outline's first vertex. Require a unique matching layer, net and
/// vertex, and target the centerline of an existing track inside its fill.
pub(super) fn contact(
    source: &Expr,
    filled: &Expr,
    codes: &BTreeMap<String, String>,
    net: &str,
    witness: Witness<'_>,
) -> Option<TeardropContact> {
    let filled_codes = net_codes(filled);
    let mut matches = filled.children().iter().filter(|zone| {
        zone.head() == Some("zone")
            && zone
                .child("attr")
                .and_then(|attr| attr.child("teardrop"))
                .is_some()
            && item_net(zone, &filled_codes).is_some_and(|name| unescape(name) == unescape(net))
            && form_atom(zone, "layer", 1) == Some(witness.layer)
            && zone
                .child("polygon")
                .and_then(|polygon| polygon.child("pts"))
                .and_then(|pts| form_xy(pts, "xy").ok())
                == Some(witness.at)
    });
    let zone = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let mut fills = zone.children().iter().filter(|fill| {
        fill.head() == Some("filled_polygon") && form_atom(fill, "layer", 1) == Some(witness.layer)
    });
    let fill = fills.next()?;
    if fills.next().is_some() || fill.child("island").is_some() {
        return None;
    }
    let polygon = fill
        .child("pts")?
        .children()
        .iter()
        .skip(1)
        .map(|point| {
            Some([
                point.children().get(1)?.atom()?.parse::<f64>().ok()?,
                point.children().get(2)?.atom()?.parse::<f64>().ok()?,
            ])
        })
        .collect::<Option<Vec<_>>>()?;
    if polygon.len() < 3 || polygon.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }
    let (kept_item, contact_mm) = source
        .children()
        .iter()
        .filter(|item| {
            item.head() == Some("segment")
                && form_atom(item, "layer", 1) == Some(witness.layer)
                && item_net(item, codes) == Some(net)
        })
        .filter_map(|item| {
            let start = form_xy(item, "start").ok()?;
            let end = form_xy(item, "end").ok()?;
            let mid = std::array::from_fn(|i| (start[i] + end[i]) / 2.0);
            let at = [start, end, mid]
                .into_iter()
                .filter(|&at| point_in_polygon(at, &polygon))
                .min_by(|&a, &b| distance(a, witness.at).total_cmp(&distance(b, witness.at)))?;
            Some((item_id(item)?, at))
        })
        .min_by(|(a_id, a), (b_id, b)| {
            distance(*a, witness.at)
                .total_cmp(&distance(*b, witness.at))
                .then(a_id.cmp(b_id))
        })?;
    Some(TeardropContact {
        witness_mm: witness.at,
        layer: witness.layer.to_owned(),
        kept_item,
        contact_mm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_transient_native_endpoint_requires_same_net_and_exact_filled_contact() {
        let source = parse(
            r#"(kicad_pcb
                (footprint "C" (pad "1" smd rect (net "GND") (uuid "pad")))
                (segment (start 1 1) (end 2 1) (layer "F.Cu") (net "GND") (uuid "track"))
                (segment (start 1 1) (end 2 1) (layer "B.Cu") (net "GND") (uuid "other-layer"))
                (segment (start 1 1) (end 2 1) (layer "F.Cu") (net "VCC") (uuid "other-net")))"#,
        )
        .unwrap();
        let zone = r#"(zone (net "GND") (layer "F.Cu") (uuid "regenerated")
                (attr (teardrop (type padvia)))
                (polygon (pts (xy 0.9 0.8) (xy 1.4 0.8) (xy 1.4 1.2) (xy 0.9 1.2)))
                (filled_polygon (layer "F.Cu")
                    (pts (xy 0.9 0.8) (xy 1.4 0.8) (xy 1.4 1.2) (xy 0.9 1.2))))"#;
        let fill = parse(&format!("(kicad_pcb {zone})")).unwrap();
        let report = json!({"unconnected_items":[{"items":[
            {"uuid":"transient", "description":"Teardrop [GND] on F.Cu", "pos":{"x":0.9,"y":0.8}},
            {"uuid":"pad", "description":"Pad 1 [GND]", "pos":{"x":5.0,"y":1.0}}
        ]}]});
        let codes = net_codes(&source);
        assert!(native_opens(&source, &report, &codes).is_empty());
        assert_eq!(
            native_open_witnesses(&source, &report, &codes, true).len(),
            1
        );
        let item = &report["unconnected_items"][0]["items"][0];
        let from = witness(item, "GND").unwrap();
        let resolved = contact(&source, &fill, &codes, "GND", from).unwrap();
        assert_eq!(resolved.kept_item, "track");
        assert_eq!(resolved.contact_mm, [1.0, 1.0]);

        let ambiguous = parse(&format!("(kicad_pcb {zone} {zone})")).unwrap();
        let outline_only = parse(&format!(
            "(kicad_pcb {})",
            zone.replace("filled_polygon", "polygon")
        ))
        .unwrap();
        for invalid in [&ambiguous, &outline_only] {
            assert!(
                contact(
                    &source,
                    invalid,
                    &codes,
                    "GND",
                    witness(item, "GND").unwrap()
                )
                .is_none()
            );
        }
        let mut other_net = report.clone();
        other_net["unconnected_items"][0]["items"][0]["description"] =
            json!("Teardrop [VCC] on F.Cu");
        assert!(native_open_witnesses(&source, &other_net, &codes, true).is_empty());
    }
}

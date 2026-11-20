// Copyright (C) 2026 Florian Loitsch.

//! Measured copper-zone geometry from the PCB's stored fill polygons.

use super::*;
use crate::kicad_nets::{item_net, net_codes};

#[derive(Clone, Debug, Serialize)]
pub struct KiCadStoredZoneFills {
    /// What the measurement reads, without refilling the board.
    pub provenance: &'static str,
    pub layers: Vec<KiCadStoredZoneLayer>,
    /// Stored fill polygons whose net, layer, or contour could not be read.
    pub unattributed_polygons: usize,
    /// Copper-zone definitions with an ambiguous net or unresolved layer set.
    pub unattributed_zones: usize,
    /// Legacy fill forms that store raster segments instead of polygons.
    pub legacy_fill_segment_forms: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KiCadStoredZoneLayer {
    /// The exact KiCad net name. A netless copper zone has `null` here.
    pub net: Option<String>,
    pub layer: String,
    pub zone_count: usize,
    pub stored_polygon_count: usize,
    /// Sum of attributed polygon areas. Overlapping contours are not unioned.
    pub filled_area_mm2: f64,
}

/// Top-level and footprint-owned copper pours, without rule areas or teardrops.
pub fn copper_zone_definitions(pcb: &Expr) -> impl Iterator<Item = &Expr> {
    let footprints = pcb
        .children()
        .iter()
        .filter(|item| matches!(item.head(), Some("footprint" | "module")));
    pcb.children()
        .iter()
        .chain(footprints.flat_map(Expr::children))
        .filter(|item| {
            item.head() == Some("zone")
                && !is_rule_area(item)
                && !item
                    .child("attr")
                    .is_some_and(|attrs| attrs.child("teardrop").is_some())
                && zone_layer_names(item).any(|name| name.ends_with(".Cu"))
        })
}

/// The layer names written by a zone, before expanding KiCad's copper aliases.
fn zone_layer_names(zone: &Expr) -> impl Iterator<Item = &str> {
    zone.child("layers")
        .or_else(|| zone.child("layer"))
        .into_iter()
        .flat_map(|layers| layers.children().iter().skip(1).filter_map(Expr::atom))
}

fn zone_net<'a>(
    zone: &'a Expr,
    codes: &'a BTreeMap<String, String>,
    conflicted: &BTreeSet<&str>,
) -> Result<Option<&'a str>, ()> {
    let fallback = form_atom(zone, "net_name", 1);
    if zone.child("net").is_none() {
        return Ok(fallback.filter(|net| !net.is_empty()));
    }
    if let Some([_, Expr::Atom(raw)]) = zone.child("net").map(Expr::children)
        && conflicted.contains(raw.as_str())
    {
        return Err(());
    }
    if let Some(net) = item_net(zone, codes) {
        if fallback.is_some_and(|declared| declared != net) {
            return Err(());
        }
        return Ok(Some(net).filter(|net| !net.is_empty()));
    }
    if let Some([_, Expr::Atom(code)]) = zone.child("net").map(Expr::children) {
        if code == "0" {
            return fallback.is_none_or(str::is_empty).then_some(None).ok_or(());
        }
        if code.bytes().all(|c| c.is_ascii_digit()) {
            return fallback.filter(|net| !net.is_empty()).map(Some).ok_or(());
        }
    }
    Err(())
}

pub fn stored_zone_fills(pcb: &Expr) -> KiCadStoredZoneFills {
    let active = board_copper_layers(pcb);
    let codes = net_codes(pcb);
    let conflicted = pcb
        .children()
        .iter()
        .filter_map(|item| {
            let [head, code, name] = item.children() else {
                return None;
            };
            let (code, name) = (code.atom()?, name.atom()?);
            (head.atom() == Some("net") && codes.get(code).is_some_and(|old| old != name))
                .then_some(code)
        })
        .collect::<BTreeSet<_>>();
    let mut rows = BTreeMap::<(Option<&str>, &str), KiCadStoredZoneLayer>::new();
    let mut unattributed_polygons = 0;
    let mut unattributed_zones = 0;
    let mut legacy_fill_segment_forms = 0;
    for zone in copper_zone_definitions(pcb) {
        let mut layers = BTreeSet::new();
        let mut unknown_layer = false;
        for layer in zone_layer_names(zone) {
            match layer {
                "*.Cu" => layers.extend(active.iter().map(String::as_str)),
                "F&B.Cu" => layers.extend(
                    ["F.Cu", "B.Cu"]
                        .into_iter()
                        .filter(|name| active.iter().any(|layer| layer == name)),
                ),
                name if active.iter().any(|layer| layer == name) => {
                    layers.insert(name);
                }
                name if name.ends_with(".Cu") => unknown_layer = true,
                _ => (),
            }
        }
        let polygons = zone
            .children()
            .iter()
            .filter(|item| item.head() == Some("filled_polygon"));
        legacy_fill_segment_forms += zone
            .children()
            .iter()
            .filter(|item| item.head() == Some("fill_segments"))
            .count();
        let Ok(net) = zone_net(zone, &codes, &conflicted) else {
            unattributed_zones += 1;
            unattributed_polygons += polygons.count();
            continue;
        };
        if unknown_layer || layers.is_empty() {
            unattributed_zones += 1;
        }
        for layer in &layers {
            rows.entry((net, layer))
                .or_insert_with(|| KiCadStoredZoneLayer {
                    net: net.map(str::to_owned),
                    layer: (*layer).to_owned(),
                    ..KiCadStoredZoneLayer::default()
                })
                .zone_count += 1;
        }
        for polygon in polygons {
            let layer = match polygon.child("layer") {
                Some(form) => form.children().get(1).and_then(Expr::atom),
                None if layers.len() == 1 && !unknown_layer => layers.first().copied(),
                None => None,
            };
            if let Some(row) = layer
                .and_then(|layer| layers.contains(layer).then_some(layer))
                .and_then(|layer| rows.get_mut(&(net, layer)))
                && let Ok(area) = filled_polygon_area(polygon)
                && area.is_finite()
            {
                row.stored_polygon_count += 1;
                row.filled_area_mm2 += area;
            } else {
                unattributed_polygons += 1;
            }
        }
    }
    KiCadStoredZoneFills {
        provenance: "pcb_file_filled_polygons",
        layers: rows.into_values().collect(),
        unattributed_polygons,
        unattributed_zones,
        legacy_fill_segment_forms,
    }
}

pub fn inspect_kicad_stored_zone_fills(path: &Path) -> Result<KiCadStoredZoneFills, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let pcb = parse(&text)?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!("{} is not a KiCad PCB", path.display()));
    }
    Ok(stored_zone_fills(&pcb))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = r#"(layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (2 "B.Cu" signal))
        (net 1 "GND") (net 2 "/GND")"#;
    const POINTS: &str = "(pts (xy 0 0) (xy 2 0) (xy 2 2) (xy 0 2))";

    fn board(items: &str) -> Expr {
        parse(&format!("(kicad_pcb {HEADER} {items})")).unwrap()
    }

    #[test]
    fn stored_areas_resolve_legacy_net_codes_and_exact_names_on_alias_layers() {
        let pcb = board(&format!(
            r#"
            (zone (net 1) (net_name "GND") (layers "F&B.Cu")
              (filled_polygon (layer "B.Cu") {POINTS}))
            (footprint "test" (zone (net "/GND") (layer "In1.Cu") (filled_polygon {POINTS})))
            (zone (net 0) (layer "In1.Cu") (fill_segments {POINTS}))
            (zone (net "GND") (layer "F.Cu") (keepout (tracks not_allowed)))
            (zone (net "GND") (layer "F.Cu") (attr (teardrop via_pad)) (filled_polygon {POINTS}))"#
        ));
        let report = stored_zone_fills(&pcb);
        assert_eq!(report.provenance, "pcb_file_filled_polygons");
        assert_eq!(
            report
                .layers
                .iter()
                .map(|row| {
                    (
                        row.net.as_deref(),
                        row.layer.as_str(),
                        row.zone_count,
                        row.stored_polygon_count,
                        row.filled_area_mm2,
                    )
                })
                .collect::<Vec<_>>(),
            [
                (None, "In1.Cu", 1, 0, 0.),
                (Some("/GND"), "In1.Cu", 1, 1, 4.),
                (Some("GND"), "B.Cu", 1, 1, 4.),
                (Some("GND"), "F.Cu", 1, 0, 0.),
            ]
        );
        assert_eq!(
            (
                report.unattributed_zones,
                report.unattributed_polygons,
                report.legacy_fill_segment_forms
            ),
            (0, 0, 1)
        );
    }

    #[test]
    fn empty_unfilled_and_unattributed_zone_data_remain_distinct() {
        assert!(stored_zone_fills(&board("")).layers.is_empty());
        let unfilled = stored_zone_fills(&board(r#"(zone (net "GND") (layers "*.Cu"))"#));
        assert_eq!(unfilled.layers.len(), 3);
        assert!(
            unfilled
                .layers
                .iter()
                .all(|layer| layer.zone_count == 1 && layer.stored_polygon_count == 0)
        );
        let pcb = board(&format!(
            r#"
            (zone (net 88) (layer "F.Cu") (filled_polygon {POINTS}))
            (zone (net 1) (net_name "/GND") (layer "F.Cu") (filled_polygon {POINTS}))
            (zone (net 1) (layers "F&B.Cu") (filled_polygon {POINTS}))
            (zone (net 9) (net_name "legacy rail") (layer "B.Cu") (filled_polygon {POINTS}))"#
        ));
        let report = stored_zone_fills(&pcb);
        assert_eq!(
            (report.unattributed_zones, report.unattributed_polygons),
            (2, 3)
        );
        assert_eq!(
            report
                .layers
                .iter()
                .map(|row| row.filled_area_mm2)
                .sum::<f64>(),
            4.0
        );
        assert!(
            report
                .layers
                .iter()
                .any(|row| row.net.as_deref() == Some("legacy rail") && row.filled_area_mm2 == 4.0)
        );
    }
}

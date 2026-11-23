// Copyright (C) 2026 Florian Loitsch.

//! Compare the copper pours of a written board against an explicit reference.

use pcb_kicad_file::board_stats::{copper_zone_definitions, filled_polygon_area, track_arc_length};
use pcb_kicad_file::kicad_nets::{footprint_reference, item_net, net_codes};
use pcb_kicad_file::kicad_sexpr::{Expr, form_atom, form_f64, form_xy, parse};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadDesignerPours {
    pub reference_board: PathBuf,
    pub missing_pours: Vec<KiCadMissingPour>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadMissingPour {
    pub net: String,
    pub terminals: Vec<KiCadPourTerminal>,
    /// Sum of stored filled polygon areas, measured on each copper layer.
    pub reference_filled_mm2: f64,
    pub reference_layer_mm2: BTreeMap<String, f64>,
    pub reference_track_mm: f64,
    pub reference_vias: usize,
    pub candidate_track_mm: f64,
    pub candidate_vias: usize,
    pub candidate_tracks: Vec<KiCadPourTrack>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct KiCadPourTerminal {
    pub reference: String,
    pub pad: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadPourTrack {
    pub layer: String,
    pub width_mm: f64,
    /// Sum of stored straight and circular track centrelines at this width.
    pub length_mm: f64,
}

type NetCodes = BTreeMap<String, String>;

/// Duplicate physical pads with the same number and net are one electrical
/// terminal. Ambiguous reference or pad identities exclude the affected net.
pub(crate) fn terminals(
    pcb: &Expr,
    codes: &NetCodes,
) -> BTreeMap<String, BTreeSet<KiCadPourTerminal>> {
    let mut references = BTreeSet::new();
    let mut ambiguous_references = BTreeSet::new();
    let mut pins = BTreeMap::<KiCadPourTerminal, BTreeSet<String>>::new();
    for footprint in pcb.children().iter().filter(|item| is_footprint(item)) {
        let Some(reference) = footprint_reference(footprint).filter(|name| !name.is_empty()) else {
            continue;
        };
        if !references.insert(reference.clone()) {
            ambiguous_references.insert(reference.clone());
        }
        for pad in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            if pad.children().get(2).and_then(Expr::atom) == Some("np_thru_hole") {
                continue;
            }
            let Some(number) = pad
                .children()
                .get(1)
                .and_then(Expr::atom)
                .filter(|n| !n.is_empty())
            else {
                continue;
            };
            let id = KiCadPourTerminal {
                reference: reference.clone(),
                pad: number.into(),
            };
            let net = item_net(pad, codes).unwrap_or_default();
            pins.entry(id).or_default().insert(net.into());
        }
    }
    let mut nets = BTreeMap::<String, BTreeSet<KiCadPourTerminal>>::new();
    let mut ambiguous_nets = BTreeSet::new();
    for (pad, assignments) in pins {
        if assignments.len() != 1 || ambiguous_references.contains(&pad.reference) {
            ambiguous_nets.extend(assignments);
        } else if let Some(net) = assignments.into_iter().next().filter(|net| !net.is_empty()) {
            nets.entry(net).or_default().insert(pad);
        }
    }
    nets.retain(|net, group| group.len() >= 2 && !ambiguous_nets.contains(net));
    nets
}

fn is_footprint(item: &Expr) -> bool {
    matches!(item.head(), Some("footprint" | "module"))
}

fn copper_zones<'a>(
    pcb: &'a Expr,
    codes: &'a NetCodes,
) -> impl Iterator<Item = (&'a str, &'a Expr)> {
    copper_zone_definitions(pcb).filter_map(|item| {
        let net = item_net(item, codes).filter(|net| !net.is_empty())?;
        Some((net, item))
    })
}

fn reference_fills(
    pcb: &Expr,
    codes: &NetCodes,
    eligible: &BTreeMap<String, BTreeSet<KiCadPourTerminal>>,
) -> BTreeMap<String, BTreeMap<String, f64>> {
    let mut filled = BTreeMap::<String, BTreeMap<String, f64>>::new();
    let mut unreadable = BTreeSet::new();
    for (name, zone) in copper_zones(pcb, codes).filter(|(name, _)| eligible.contains_key(*name)) {
        for polygon in zone
            .children()
            .iter()
            .filter(|item| item.head() == Some("filled_polygon"))
        {
            let layer = form_atom(polygon, "layer", 1).or_else(|| form_atom(zone, "layer", 1));
            match (layer, filled_polygon_area(polygon)) {
                (Some(layer), Ok(area)) if layer.ends_with(".Cu") && area.is_finite() => {
                    if area > 0.0 {
                        *filled
                            .entry(name.into())
                            .or_default()
                            .entry(layer.into())
                            .or_default() += area;
                    }
                }
                _ => {
                    unreadable.insert(name);
                }
            }
        }
    }
    filled.retain(|name, _| !unreadable.contains(name.as_str()));
    filled
}

#[derive(Default)]
pub(crate) struct RoutedCopper {
    pub tracks: BTreeMap<(String, i64), f64>,
    vias: usize,
}

impl RoutedCopper {
    fn length(&self) -> f64 {
        self.tracks.values().sum()
    }
}

pub(crate) fn routed_copper(
    pcb: &Expr,
    codes: &NetCodes,
    includes: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, RoutedCopper>, String> {
    let mut inventory = BTreeMap::<String, RoutedCopper>::new();
    for item in pcb.children() {
        let kind = item.head();
        if !matches!(kind, Some("via" | "arc" | "segment")) {
            continue;
        }
        let Some(net) = item_net(item, codes).filter(|name| includes(name)) else {
            continue;
        };
        let routed = inventory.entry(net.into()).or_default();
        if kind == Some("via") {
            routed.vias += 1;
            continue;
        }
        let width = form_f64(item, "width", 1)?;
        let layer =
            form_atom(item, "layer", 1).ok_or_else(|| format!("track on {net} has no layer"))?;
        let length = if kind == Some("arc") {
            track_arc_length(item)?
        } else {
            let [x, y] = form_xy(item, "start")?;
            let [end_x, end_y] = form_xy(item, "end")?;
            (end_x - x).hypot(end_y - y)
        };
        if length > 0.0 {
            *routed
                .tracks
                .entry((layer.into(), (width * 1_000_000.0).round() as i64))
                .or_default() += length;
        }
    }
    Ok(inventory)
}

fn missing_pours(reference: &Expr, candidate: &Expr) -> Result<Vec<KiCadMissingPour>, String> {
    if [reference, candidate]
        .iter()
        .any(|pcb| pcb.head() != Some("kicad_pcb"))
    {
        return Err("pour comparison requires two KiCad PCBs".into());
    }
    let reference_codes = net_codes(reference);
    let candidate_codes = net_codes(candidate);
    let mut matching = terminals(reference, &reference_codes);
    let candidate_terminals = terminals(candidate, &candidate_codes);
    let candidate_zones: BTreeSet<_> = copper_zones(candidate, &candidate_codes)
        .map(|(name, _)| name)
        .collect();
    // A zone on the candidate may be unfilled on disk and still pour on
    // refill. Suppress those nets instead of confusing the fill state with
    // an omitted zone or a deliberate move of a plane to another layer.
    matching.retain(|name, group| {
        candidate_terminals.get(name) == Some(group) && !candidate_zones.contains(name.as_str())
    });
    let filled = reference_fills(reference, &reference_codes, &matching);
    let mut reference_routed =
        routed_copper(reference, &reference_codes, |net| filled.contains_key(net))?;
    let mut candidate_routed =
        routed_copper(candidate, &candidate_codes, |net| filled.contains_key(net))?;
    let mut result = Vec::new();
    for (net, layer_area) in filled {
        let before = reference_routed.remove(&net).unwrap_or_default();
        let after = candidate_routed.remove(&net).unwrap_or_default();
        result.push(KiCadMissingPour {
            terminals: matching
                .remove(&net)
                .expect("a measured net has matching terminals")
                .into_iter()
                .collect(),
            net,
            reference_filled_mm2: layer_area.values().sum(),
            reference_layer_mm2: layer_area,
            reference_track_mm: before.length(),
            reference_vias: before.vias,
            candidate_track_mm: after.length(),
            candidate_vias: after.vias,
            candidate_tracks: after
                .tracks
                .into_iter()
                .map(|((layer, width), length_mm)| KiCadPourTrack {
                    layer,
                    width_mm: width as f64 / 1_000_000.0,
                    length_mm,
                })
                .collect(),
        });
    }
    result.sort_by(|a, b| {
        b.reference_filled_mm2
            .total_cmp(&a.reference_filled_mm2)
            .then(a.net.cmp(&b.net))
    });
    Ok(result)
}

pub fn compare_kicad_designer_pours(
    reference: &Path,
    candidate: &Path,
) -> Result<KiCadDesignerPours, String> {
    let read = |path: &Path| {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        parse(&text).map_err(|error| format!("parse {}: {error}", path.display()))
    };
    Ok(KiCadDesignerPours {
        reference_board: reference.to_path_buf(),
        missing_pours: missing_pours(&read(reference)?, &read(candidate)?)?,
    })
}

pub(crate) fn action(report: &KiCadDesignerPours) -> Option<String> {
    let first = report.missing_pours.first()?;
    let layers = first
        .reference_layer_mm2
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let widths = first
        .candidate_tracks
        .iter()
        .map(|item| item.width_mm.to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");
    let tracks = if widths.is_empty() {
        "no routed tracks".into()
    } else {
        format!(
            "{:.2} mm of routed tracks with widths {widths} mm",
            first.candidate_track_mm
        )
    };
    Some(format!(
        "Review designer_pours for nets with filled copper on the reference and no zone on this board. {} has {} matching terminals, {:.2} mm² of reference fill on {layers}, and {tracks} here. Where a zone is required, use add_pours with region.pads and a chosen layer to place it around terminals in the current layout.",
        first.net,
        first.terminals.len(),
        first.reference_filled_mm2,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TERMINALS: &str = r#"
      (footprint test (property "Reference" "J1")
        (pad "2" smd rect (net 1 "rail")))
      (footprint test (property "Reference" "U1")
        (pad "1" smd rect (net "rail")))"#;

    const FILLED: &str = r#"
      (filled_polygon (layer "F.Cu") (pts (xy 0 0) (xy 10 0) (xy 10 8) (xy 0 8)))
      (filled_polygon (layer "B.Cu") (pts (xy 0 0) (xy 2 0) (xy 2 1) (xy 0 1)))"#;

    fn board(items: &str, terminals: &str) -> Expr {
        parse(&format!(
            r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
              (net 1 "rail")
              {terminals} {items})"#
        ))
        .unwrap()
    }

    #[test]
    fn a_missing_pour_has_per_layer_evidence_and_exact_terminals() {
        let reference = board(
            &format!(
                r#"(zone (net 1) (layers "*.Cu") {FILLED})
            (segment (start 0 0) (end 1 0) (width 0.3) (layer "F.Cu") (net 1))"#
            ),
            TERMINALS,
        );
        let candidate = board(
            &format!(
                r#"
            (zone (net "rail") (layers "*.Cu") (attr (teardrop via_pad)) {FILLED})
            (zone (net "rail") (layer "F.Cu") (keepout (copperpour not_allowed)))
            (segment (start 0 0) (end 2 0) (width 0.1712) (layer "F.Cu") (net "rail"))
            (arc (start 1 0) (mid 0 1) (end -1 0) (width 0.6) (layer "B.Cu") (net 1))
            (via (net "rail"))"#
            ),
            TERMINALS,
        );
        let result = missing_pours(&reference, &candidate).unwrap();
        assert_eq!(result.len(), 1);
        let result = &result[0];
        assert_eq!(result.net, "rail");
        assert_eq!(
            result
                .terminals
                .iter()
                .map(|pad| (pad.reference.as_str(), pad.pad.as_str()))
                .collect::<Vec<_>>(),
            [("J1", "2"), ("U1", "1")]
        );
        assert_eq!(result.reference_filled_mm2, 82.0);
        assert_eq!(
            result.reference_layer_mm2,
            BTreeMap::from([("B.Cu".into(), 2.), ("F.Cu".into(), 80.)])
        );
        assert_eq!(result.reference_track_mm, 1.0);
        assert_eq!(result.reference_vias, 0);
        assert_eq!(result.candidate_vias, 1);
        assert!((result.candidate_track_mm - (2.0 + std::f64::consts::PI)).abs() < 1.0e-12);
        assert_eq!(
            result.candidate_tracks[1],
            KiCadPourTrack {
                layer: "F.Cu".into(),
                width_mm: 0.1712,
                length_mm: 2.
            }
        );
    }

    #[test]
    fn unfilled_zones_moved_layers_changed_pads_and_ambiguous_references_do_not_show_loss() {
        let reference = board(
            &format!(r#"(zone (net 1) (layers "*.Cu") {FILLED})"#),
            TERMINALS,
        );
        for candidate in [
            reference.clone(),
            board(r#"(zone (net 1) (layer "B.Cu"))"#, TERMINALS),
            board("", &TERMINALS.replace(r#"(pad "1""#, r#"(pad "3""#)),
            board("", &TERMINALS.replace(r#""U1""#, r#""J1""#)),
            board(
                "",
                &format!(
                    r#"{TERMINALS} (footprint test (property "Reference" "J1") (pad "4" smd rect (net "other")))"#
                ),
            ),
        ] {
            assert!(missing_pours(&reference, &candidate).unwrap().is_empty());
        }
        let only_definition = board(r#"(zone (net 1) (layer "F.Cu"))"#, TERMINALS);
        assert!(
            missing_pours(&only_definition, &board("", TERMINALS))
                .unwrap()
                .is_empty()
        );
    }
}

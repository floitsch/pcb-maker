// Copyright (C) 2026 Florian Loitsch.

//! Pad-relative copper-zone geometry and its update after placement.

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPourRegion {
    /// Exact `REF:PAD` names, all on this pour's net.
    pub pads: Vec<String>,
    /// How far the zone extends beyond the box of the named pad centres.
    pub margin_mm: f64,
}

pub(super) fn region_uuid(net: &str, layer: &str, region: &KiCadPourRegion) -> String {
    let mut hash = pour_hash(net, layer).rotate_left(11) ^ 0x752f_6102_ff29_c833;
    // The same set of anchors describes one region, independent of order.
    for pad in region.pads.iter().collect::<BTreeSet<_>>() {
        for byte in (pad.len() as u64)
            .to_le_bytes()
            .into_iter()
            .chain(pad.bytes())
        {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
    }
    zone_uuid(hash)
}

pub(crate) const PAD_REGION_PREFIX: &str = "pcb-maker pad region ";

pub(crate) fn region_name(net: &str, layer: &str) -> String {
    format!("{PAD_REGION_PREFIX}{net} {layer}")
}

/// Reposition our pad-relative zones when their anchor footprints moved.
pub fn refresh_pour_regions(pcb: &mut Expr, requests: &[KiCadPourRequest]) -> Result<(), String> {
    if requests.iter().all(|request| request.region.is_none()) {
        return Ok(());
    }
    let names = NetNames::of(pcb);
    let find = |zone: &Expr| {
        let net = node_net(zone)?;
        let layer = form_atom(zone, "layer", 1)?;
        let id = form_atom(zone, "uuid", 1)?;
        let normalized = names.normalize(net);
        if form_atom(zone, "name", 1) != Some(region_name(normalized, layer).as_str()) {
            return None;
        }
        requests.iter().find_map(|request| {
            let region = request.region.as_ref()?;
            (names.normalize(&request.net) == normalized
                && request.layers.iter().any(|target| target == layer)
                && id == region_uuid(net, layer, region))
            .then(|| (region, normalized.to_string()))
        })
    };
    let mut updates = Vec::new();
    let mut pads = None;
    for (index, zone) in pcb
        .children()
        .iter()
        .enumerate()
        .filter(|(_, item)| item.head() == Some("zone"))
    {
        let Some((region, net)) = find(zone) else {
            continue;
        };
        if pads.is_none() {
            pads = Some(region_pads(pcb, &LayerTable::from_pcb(pcb)?)?);
        }
        let points = pad_region_polygon(region, &net, pads.as_deref().unwrap_or_default())?;
        let polygon = parse(&format!("(polygon (pts {points}))"))?;
        // KiCad writes coordinates on its nanometre lattice. Keep the
        // valid stored fill when a save changed only decimal spelling.
        let position = |polygon: &Expr| {
            outline::pts_points(polygon).ok().map(|points| {
                points
                    .into_iter()
                    .map(|point| point.map(|mm| (mm * 1_000_000.0).round() as i64))
                    .collect::<Vec<_>>()
            })
        };
        let same_outline = zone
            .child("polygon")
            .and_then(position)
            .is_some_and(|old| position(&polygon) == Some(old));
        if !same_outline {
            updates.push((index, polygon));
        }
    }
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    for (index, polygon) in updates {
        let Expr::List(fields) = &mut items[index] else {
            unreachable!("zone is a list")
        };
        fields.retain(|field| !matches!(field.head(), Some("filled_polygon" | "fill_segments")));
        if let Some(old) = fields
            .iter_mut()
            .find(|field| field.head() == Some("polygon"))
        {
            *old = polygon;
        } else {
            fields.push(polygon);
        }
    }
    Ok(())
}

pub(super) fn pad_region_polygon(
    region: &KiCadPourRegion,
    net: &str,
    pads: &[RegionPad],
) -> Result<String, String> {
    if !region.margin_mm.is_finite() || region.margin_mm < 0.0 || region.pads.is_empty() {
        return Err(format!(
            "region for the {net} pour needs pad names and a finite nonnegative margin_mm"
        ));
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for name in &region.pads {
        let mut matches = pads.iter().filter(|pad| pad.label == *name);
        let first = matches
            .next()
            .ok_or_else(|| format!("region for the {net} pour: no copper pad {name:?}"))?;
        for pad in std::iter::once(first).chain(matches) {
            if !pad.unique_reference || pad.net.as_deref() != Some(net) {
                return Err(format!(
                    "region for the {net} pour: {name:?} must name pads on that net in a unique footprint"
                ));
            }
            for axis in 0..2 {
                min[axis] = min[axis].min(pad.center[axis] - region.margin_mm);
                max[axis] = max[axis].max(pad.center[axis] + region.margin_mm);
            }
        }
    }
    let limit = f64::from(i32::MAX) / 1_000_000.0;
    if min
        .iter()
        .chain(&max)
        .any(|value| !(-limit..=limit).contains(value))
        || (0..2).any(|axis| min[axis] >= max[axis])
    {
        return Err(format!(
            "region for the {net} pour has empty or out-of-range bounds"
        ));
    }
    let [x0, y0] = min;
    let [x1, y1] = max;
    Ok(format!(
        "(xy {x0} {y0}) (xy {x1} {y0}) (xy {x1} {y1}) (xy {x0} {y1})"
    ))
}

#[cfg(test)]
mod pad_region_tests {
    use super::*;

    const BOARD: &str = r#"(kicad_pcb
      (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
      (gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts"))
      (footprint "a" (at 10 10 90) (property "Reference" "U1")
        (pad "1" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net "/VCC")))
      (footprint "b" (at 18 12) (property "Reference" "J1")
        (pad "2" thru_hole circle (at 0 0) (size 1 1) (layers "*.Cu") (net "/VCC"))
        (pad "2" thru_hole circle (at 2 0) (size 1 1) (layers "*.Cu") (net "/VCC"))
        (pad "3" thru_hole circle (at 0 3) (size 1 1) (layers "*.Cu") (net "/GND"))))"#;

    fn request(pads: &[&str]) -> KiCadPourRequest {
        serde_json::from_value(serde_json::json!({
            "net": "/VCC", "layers": ["In1.Cu"],
            "region": {"pads": pads, "margin_mm": 2.0}
        }))
        .unwrap()
    }

    #[test]
    fn a_bounded_zone_uses_the_current_pad_centres_after_rotation() {
        let mut pcb = parse(BOARD).unwrap();
        let keep = request(&["J1:2", "U1:1"]);
        add_pour_zones(&mut pcb, std::slice::from_ref(&keep)).unwrap();
        let zone = pcb.child("zone").unwrap();
        assert_eq!(form_atom(zone, "layer", 1), Some("In1.Cu"));
        assert_eq!(
            zone.child("polygon"),
            Some(&parse("(polygon (pts (xy 8 7) (xy 22 7) (xy 22 14) (xy 8 14)))").unwrap())
        );
        let saved = pcb.clone();
        add_pour_zones(&mut pcb, &[keep]).unwrap();
        assert_eq!(pcb, saved);
    }

    #[test]
    fn a_pour_region_rejects_missing_or_foreign_pad_centres() {
        let original = parse(BOARD).unwrap();
        for pads in [&["J1:2", "U1:8"][..], &["J1:2", "J1:3"][..], &[][..]] {
            let mut pcb = original.clone();
            assert!(add_pour_zones(&mut pcb, &[request(pads)]).is_err());
            assert_eq!(pcb, original);
        }
        let mut plane = request(&["U1:1", "J1:2"]);
        plane.plane = true;
        assert!(add_pour_zones(&mut original.clone(), &[plane]).is_err());
        let mut duplicated = parse(&BOARD.replace("\"U1\"", "\"J1\"")).unwrap();
        assert!(add_pour_zones(&mut duplicated, &[request(&["J1:2"])]).is_err());
    }

    #[test]
    fn a_moved_pad_refreshes_only_its_generated_zone_and_invalidates_the_fill() {
        let mut pcb = parse(BOARD).unwrap();
        let request = request(&["J1:2", "U1:1"]);
        add_pour_zones(&mut pcb, std::slice::from_ref(&request)).unwrap();
        let with_fill = encode(&pcb).replace(
            "(polygon",
            "(filled_polygon (layer \"In1.Cu\") (pts (xy 8 7) (xy 22 7) (xy 22 14))) (polygon",
        );
        let mut respelled = parse(&with_fill.replace("(xy 8 7)", "(xy 8.000000 7)")).unwrap();
        let same = respelled.clone();
        refresh_pour_regions(&mut respelled, std::slice::from_ref(&request)).unwrap();
        assert_eq!(respelled, same);
        let mut moved = parse(&with_fill.replace("(at 10 10 90)", "(at 12 10 90)")).unwrap();
        refresh_pour_regions(&mut moved, &[request]).unwrap();
        let before = pcb.child("zone").unwrap();
        let after = moved.child("zone").unwrap();
        assert_eq!(before.child("uuid"), after.child("uuid"));
        assert_eq!(before.child("fill"), after.child("fill"));
        assert!(after.child("filled_polygon").is_none());
        assert_eq!(
            after.child("polygon"),
            Some(&parse("(polygon (pts (xy 10 7) (xy 22 7) (xy 22 14) (xy 10 14)))").unwrap())
        );
    }

    #[test]
    fn disjoint_regions_coexist_on_the_same_net_and_layer_with_an_authored_zone() {
        let source = format!(
            r#"{}
            (footprint "c" (at 26 26) (property "Reference" "U2")
              (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/VCC")))
            (zone (net "/VCC") (layer "In1.Cu") (uuid "designer-zone")
              (polygon (pts (xy 1 25) (xy 4 25) (xy 4 28) (xy 1 28)))))"#,
            BOARD.strip_suffix(')').unwrap()
        );
        let mut pcb = parse(&source).unwrap();
        let first = request(&["J1:2", "U1:1"]);
        let second = request(&["U2:1"]);
        add_pour_zones(&mut pcb, &[first.clone(), second.clone()]).unwrap();
        let zones = |pcb: &Expr| {
            pcb.children()
                .iter()
                .filter(|item| item.head() == Some("zone"))
                .cloned()
                .collect::<Vec<_>>()
        };
        let before = zones(&pcb);
        assert_eq!(before.len(), 3);
        let ids = before
            .iter()
            .filter_map(|zone| form_atom(zone, "uuid", 1))
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), 3);

        let reordered = request(&["U1:1", "J1:2"]);
        let mut changed = parse(&encode(&pcb).replace("(at 26 26)", "(at 26 27)")).unwrap();
        add_pour_zones(&mut changed, &[second, reordered.clone()]).unwrap();
        let after = zones(&changed);
        assert_eq!(&after[..2], &before[..2]);
        assert_eq!(after[2].child("uuid"), before[2].child("uuid"));
        assert_eq!(
            after[2].child("polygon"),
            Some(&parse("(polygon (pts (xy 24 25) (xy 28 25) (xy 28 29) (xy 24 29)))").unwrap())
        );

        let original = changed.clone();
        assert!(
            add_pour_zones(&mut changed, &[first, reordered])
                .unwrap_err()
                .contains("same anchor pads")
        );
        assert_eq!(changed, original);
    }
}

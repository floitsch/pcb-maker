// Copyright (C) 2026 Florian Loitsch.

//! Identify a physical pad in a native open whose printed witnesses are zones.

use super::*;
use crate::board_router::LayerTable;
use pcb_kicad_file::geometry::rotate_vector;

#[derive(Clone, Debug, Serialize)]
pub struct ZoneOpenPad {
    pub net: String,
    pub zone_uuids: [String; 2],
    pub reference: String,
    pub number: String,
    pub repeated_pin_number: bool,
    pub uuid: String,
    pub at_mm: [f64; 2],
    pub copper_layers: Vec<String>,
    pub other_component_copper_items: usize,
    pub basis: &'static str,
}

/// Use the refilled native pad/track/via partition to resolve the zone
/// witness. A zone's reported position is not a contact point.
pub fn locate_zone_open_pads(
    path: &Path,
    pcb: &Expr,
    native: &Value,
    selected: &[String],
    expired: impl Fn() -> bool,
) -> Result<Vec<ZoneOpenPad>, String> {
    let codes = net_codes(pcb);
    let names = NetNames::of(pcb);
    let witnesses = zone_witnesses(pcb, native, &codes)
        .into_iter()
        .filter(|(net, _)| {
            selected
                .iter()
                .any(|selected| selected == names.normalize(net))
        })
        .collect::<Vec<_>>();
    if witnesses.is_empty() || !Pieces::of_board(pcb).is_ok_and(|pieces| pieces.is_none()) {
        return Ok(Vec::new());
    }
    let Some(version) = native["kicad_version"].as_str() else {
        return Ok(Vec::new());
    };
    let source = fs::read(path).map_err(|error| error.to_string())?;
    let groups = native_report::connectivity_partition(path, version, true, expired)?;
    if fs::read(path).map_err(|error| error.to_string())? != source {
        return Err("The board changed during native zone-open localization.".into());
    }
    let layers = LayerTable::from_pcb(pcb)?;
    Ok(witnesses
        .into_iter()
        .filter_map(|(net, zones)| resolve(pcb, &layers, &codes, &groups, net, zones))
        .collect())
}

fn zone_witnesses<'a>(
    pcb: &'a Expr,
    native: &'a Value,
    codes: &'a BTreeMap<String, String>,
) -> Vec<(&'a str, [String; 2])> {
    if native["coordinate_units"] != "mm"
        || native[CAPPED_KEY] == true
        || native["unconnected_items"]
            .as_array()
            .is_none_or(|items| items.len() >= KICAD_UNCONNECTED_CAP)
    {
        return Vec::new();
    }
    let zones: BTreeSet<_> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone") && item.child("keepout").is_none())
        .filter_map(item_id)
        .collect();
    let mut by_net = BTreeMap::<_, Vec<_>>::new();
    for (net, items) in native_opens(pcb, native, codes) {
        // Include any reported second open on the net when checking its
        // uniqueness, even if the report marks that other open excluded.
        by_net.entry(net).or_default().push(items);
    }
    by_net
        .into_iter()
        .filter_map(|(net, pairs)| {
            let [items] = pairs.as_slice() else {
                return None;
            };
            let [first, second] = *items else { return None };
            let zone = |item: &Value| {
                let id = item["uuid"].as_str()?.to_ascii_lowercase();
                (item["description"].as_str()?.starts_with("Zone ") && zones.contains(&id))
                    .then_some(id)
            };
            let ids = [zone(first)?, zone(second)?];
            let active = native["unconnected_items"]
                .as_array()?
                .iter()
                .any(|finding| {
                    finding["items"]
                        .as_array()
                        .is_some_and(|reported| reported.as_slice() == *items)
                        && matches!(finding["severity"].as_str(), Some("warning" | "error"))
                        && finding["excluded"] != true
                });
            active.then_some((net, ids))
        })
        .collect()
}

fn resolve(
    pcb: &Expr,
    layers: &LayerTable,
    codes: &BTreeMap<String, String>,
    groups: &[(String, Vec<String>)],
    net: &str,
    zone_uuids: [String; 2],
) -> Option<ZoneOpenPad> {
    let display = unescape(net);
    let members: Vec<_> = groups
        .iter()
        .filter(|(name, _)| name == display.as_ref())
        .map(|(_, items)| items.as_slice())
        .collect();
    let [first, second] = members.as_slice() else {
        return None;
    };
    let (uuid, other) = match (first, second) {
        ([id], main) | (main, [id]) if main.len() > 1 => (id, main),
        _ => return None,
    };
    let mut pads = pcb
        .children()
        .iter()
        .filter(|part| matches!(part.head(), Some("footprint" | "module")))
        .flat_map(|part| {
            part.children()
                .iter()
                .filter(|item| item.head() == Some("pad"))
                .map(move |pad| (part, pad))
        });
    let mut landed = pads
        .clone()
        .filter(|(_, pad)| item_id(pad).as_ref() == Some(uuid));
    let (part, pad) = landed.next()?;
    if landed.next().is_some()
        || item_net(pad, codes) != Some(net)
        || !pads.any(|(_, candidate)| {
            item_net(candidate, codes) == Some(net)
                && item_id(candidate).is_some_and(|id| other.binary_search(&id).is_ok())
        })
    {
        return None;
    }
    let [x, y, angle] = form_at(part).ok()?;
    let offset = rotate_vector(form_xy(pad, "at").ok()?, -angle);
    let mask = layers.pad_mask(pad).ok()?;
    let copper_layers = layers
        .names
        .iter()
        .enumerate()
        .filter(|(layer, _)| mask & (1 << layer) != 0)
        .map(|(_, name)| name.clone())
        .collect::<Vec<_>>();
    if copper_layers.is_empty() {
        return None;
    }
    let number = pad.children().get(1)?.atom()?;
    let repeated_pin_number = part
        .children()
        .iter()
        .filter(|item| {
            item.head() == Some("pad")
                && item.children().get(1).and_then(Expr::atom) == Some(number)
        })
        .nth(1)
        .is_some();
    Some(ZoneOpenPad {
        net: display.into_owned(),
        zone_uuids,
        reference: footprint_reference(part)?,
        number: number.into(),
        repeated_pin_number,
        uuid: uuid.clone(),
        at_mm: [x + offset[0], y + offset[1]],
        copper_layers,
        other_component_copper_items: other.len(),
        basis: "KiCad's refilled native copper partition has two components and one contains only this physical pad",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOARD: &str = r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
        (footprint "P" (layer "F.Cu") (at 5 10 90) (property "Reference" "U1")
            (pad "12" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net "GND") (uuid "pad-a"))
            (pad "12" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "GND") (uuid "pad-b")))
        (zone (layer "F.Cu") (net "GND") (uuid "front"))
        (zone (layer "B.Cu") (net "GND") (uuid "back")))"#;

    fn native_zone_open() -> Value {
        json!({"coordinate_units":"mm", "unconnected_items":[{"severity":"error", "items":[
            {"uuid":"front","description":"Zone [GND] on F.Cu","pos":{"x":0,"y":0}},
            {"uuid":"back","description":"Zone [GND] on B.Cu","pos":{"x":0,"y":0}}]}]})
    }

    #[test]
    fn a_zone_pair_resolves_one_physical_pad_without_using_zone_positions() {
        let pcb = parse(BOARD).unwrap();
        let codes = net_codes(&pcb);
        let layers = LayerTable::from_pcb(&pcb).unwrap();
        let report = native_zone_open();
        let witnesses = zone_witnesses(&pcb, &report, &codes);
        let [(net, zones)] = witnesses.as_slice() else {
            panic!("expected a zone pair")
        };
        let native = |main: &[&str]| {
            vec![
                ("GND".to_string(), vec!["pad-a".to_string()]),
                (
                    "GND".to_string(),
                    main.iter().map(|item| (*item).to_owned()).collect(),
                ),
            ]
        };
        let mut groups = native(&["pad-b", "via-1"]);
        let on_pad = resolve(&pcb, &layers, &codes, &groups, net, zones.clone()).unwrap();
        assert_eq!(on_pad.uuid, "pad-a");
        assert_eq!(on_pad.reference, "U1");
        assert_eq!(on_pad.number, "12");
        assert!(on_pad.repeated_pin_number);
        assert_eq!(on_pad.at_mm, [5.0, 9.0]);
        assert_eq!(on_pad.copper_layers, ["F.Cu"]);
        assert!(
            resolve(
                &pcb,
                &layers,
                &codes,
                &native(&["pad-b"]),
                net,
                zones.clone()
            )
            .is_none()
        );
        assert!(
            resolve(
                &pcb,
                &layers,
                &codes,
                &native(&["via-0", "via-1"]),
                net,
                zones.clone()
            )
            .is_none()
        );
        groups.push(("GND".into(), vec!["pad-c".into()]));
        assert!(resolve(&pcb, &layers, &codes, &groups, net, zones.clone()).is_none());
    }

    #[test]
    fn requires_an_active_unambiguous_native_pair_of_same_net_zones() {
        let pcb = parse(BOARD).unwrap();
        let codes = net_codes(&pcb);
        let mut report = native_zone_open();
        report["unconnected_items"][0]["excluded"] = json!(true);
        assert!(zone_witnesses(&pcb, &report, &codes).is_empty());
        let mut report = native_zone_open();
        report["unconnected_items"][0]["items"][1]["uuid"] = json!("unrelated-zone");
        assert!(zone_witnesses(&pcb, &report, &codes).is_empty());
        let mut report = native_zone_open();
        let repeat = report["unconnected_items"][0].clone();
        report["unconnected_items"]
            .as_array_mut()
            .unwrap()
            .push(repeat);
        assert!(zone_witnesses(&pcb, &report, &codes).is_empty());
        let mixed = parse(&BOARD.replace(
            "(net \"GND\") (uuid \"back\")",
            "(net \"+5V\") (uuid \"back\")",
        ))
        .unwrap();
        assert!(zone_witnesses(&mixed, &native_zone_open(), &net_codes(&mixed)).is_empty());
    }
}

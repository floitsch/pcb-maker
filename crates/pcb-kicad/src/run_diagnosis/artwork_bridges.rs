// Copyright (C) 2026 Florian Loitsch.

use super::*;
use serde::Deserialize;
use std::collections::BTreeSet;

const MAX_TARGETS: usize = 8;
const MAX_WITNESSES: usize = 64;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadArtworkBridges {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<&'static str>,
    pub kicad_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_sha256: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<KiCadArtworkBridge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<&'static str>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadArtworkBridge {
    pub footprint: String,
    pub footprint_id: String,
    pub layer: String,
    pub from: KiCadArtworkPad,
    pub to: KiCadArtworkPad,
    pub native_finding: KiCadArtworkWitness,
    /// Consecutive native zero-clearance contacts from `from` to `to`.
    pub artwork_path: Vec<KiCadArtworkItem>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadArtworkPad {
    pub uuid: String,
    pub pin: String,
    pub net: String,
    pub at_mm: [f64; 2],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadArtworkItem {
    pub uuid: String,
    pub shape: String,
    pub at_mm: [f64; 2],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadArtworkWitness {
    pub kind: String,
    pub pad_uuid: String,
    pub graphic_uuid: String,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct ProbeWitness {
    footprint: String,
    layer: String,
    open_pad_uuid: String,
    finding_pad_uuid: String,
    graphic_uuid: String,
    kind: String,
}

#[derive(Deserialize)]
struct NativeProof {
    paths: Vec<KiCadArtworkBridge>,
    skipped: Vec<String>,
}

fn is_copper_artwork(kind: &str) -> bool {
    matches!(
        kind,
        "fp_line" | "fp_rect" | "fp_circle" | "fp_arc" | "fp_poly"
    )
}

pub(super) fn diagnose(
    result: &LoweredBoard,
    opens: &BTreeMap<String, Vec<KiCadNativePair>>,
    selected: &[String],
    board: &Path,
    drc: &serde_json::Value,
) -> Option<KiCadArtworkBridges> {
    let version = drc["kicad_version"].as_str()?;
    let open = open_pad_findings::native_open_pads(&result.pads, opens, selected);
    if open.is_empty() {
        return None;
    }
    let items = open_pad_findings::board_items(&result.pcb);
    let pads: BTreeMap<_, _> = result
        .pads
        .iter()
        .filter_map(|pad| Some((pad.uuid.as_ref()?.to_ascii_lowercase(), pad)))
        .collect();
    let (witnesses, mut skipped) = native_witnesses(drc, &open, &items, &pads);
    if witnesses.is_empty() {
        return None;
    }
    let request = serde_json::to_value(&witnesses).ok()?;
    let native = crate::native_report::footprint_artwork_bridges(board, version, &request)
        .and_then(|(value, hash)| {
            serde_json::from_value::<NativeProof>(value)
                .map(|proof| (proof, hash))
                .map_err(|error| error.to_string())
        });
    let mut paths = Vec::new();
    let mut board_sha256 = None;
    match native {
        Ok((proof, hash)) => {
            board_sha256 = Some(hash);
            skipped.extend(proof.skipped);
            for path in proof.paths {
                let matches_pad = |pad: &KiCadArtworkPad| {
                    items.get(&pad.uuid).is_some_and(Option::is_some)
                        && pads.get(&pad.uuid).is_some_and(|parsed| {
                            parsed.reference == path.footprint
                                && parsed.number == pad.pin
                                && parsed.net.as_deref() == Some(&pad.net)
                                && distance_squared(parsed.at, pad.at_mm) < 1.0e-6
                        })
                };
                let matches_graphic = |graphic: &KiCadArtworkItem| {
                    items
                        .get(&graphic.uuid)
                        .and_then(Option::as_ref)
                        .is_some_and(|item| {
                            item.node.head().is_some_and(is_copper_artwork)
                                && form_atom(item.node, "layer", 1) == Some(&path.layer)
                                && item.footprint.and_then(footprint_reference).as_deref()
                                    == Some(&path.footprint)
                        })
                };
                if path.from.net != path.to.net
                    && matches_pad(&path.from)
                    && matches_pad(&path.to)
                    && !path.artwork_path.is_empty()
                    && path.artwork_path.iter().all(matches_graphic)
                    && matches_witness(&path, &witnesses)
                {
                    paths.push(path);
                } else {
                    let reason = format!(
                        "{} on {} has no stable PCB identity for every native path item",
                        path.footprint, path.layer
                    );
                    if !skipped.contains(&reason) {
                        skipped.push(reason);
                    }
                }
            }
        }
        Err(reason) => skipped.push(reason),
    }
    if paths.is_empty() && skipped.is_empty() {
        return None;
    }
    let next_action = (!paths.is_empty()).then_some(
        "This copper path is inside one footprint and remains when board traces change. Review the contacts in the source footprint and check the intended electrical behavior against the schematic before changing the design.",
    );
    Some(KiCadArtworkBridges {
        status: if paths.is_empty() {
            "unavailable"
        } else if skipped.is_empty() {
            "verified"
        } else {
            "partially_verified"
        },
        evidence: (!paths.is_empty()).then_some("kicad_native_zero_clearance_shape_contacts"),
        kicad_version: version.into(),
        board_sha256,
        paths,
        skipped,
        next_action,
    })
}

fn native_witnesses(
    drc: &serde_json::Value,
    open: &BTreeMap<String, (&String, &BoardPad)>,
    items: &BTreeMap<String, Option<open_pad_findings::BoardItem<'_>>>,
    pads: &BTreeMap<String, &BoardPad>,
) -> (BTreeSet<ProbeWitness>, Vec<String>) {
    let mut open_by_reference: BTreeMap<&str, Vec<(&str, &BoardPad)>> = BTreeMap::new();
    for (uuid, (_, pad)) in open {
        open_by_reference
            .entry(&pad.reference)
            .or_default()
            .push((uuid, pad));
    }
    let mut targets = BTreeSet::new();
    let mut witnesses = BTreeSet::new();
    let mut skipped = Vec::new();
    for row in drc["violations"].as_array().into_iter().flatten() {
        if row["excluded"] == true || !matches!(row["severity"].as_str(), Some("warning" | "error"))
        {
            continue;
        }
        let Some(kind @ ("shorting_items" | "clearance")) = row["type"].as_str() else {
            continue;
        };
        let finding = crate::verdict::drc_finding(row);
        let [a, b] = finding.items.as_slice() else {
            continue;
        };
        for (endpoint, graphic) in [(a, b), (b, a)] {
            let (Some(pad_uuid), Some(graphic_uuid)) = (&endpoint.uuid, &graphic.uuid) else {
                continue;
            };
            let (pad_uuid, graphic_uuid) = (
                pad_uuid.to_ascii_lowercase(),
                graphic_uuid.to_ascii_lowercase(),
            );
            let (Some(Some(pad_item)), Some(Some(art)), Some(pad)) = (
                items.get(&pad_uuid),
                items.get(&graphic_uuid),
                pads.get(&pad_uuid),
            ) else {
                continue;
            };
            if !art.node.head().is_some_and(is_copper_artwork)
                || !pad_item
                    .footprint
                    .zip(art.footprint)
                    .is_some_and(|(pad, graphic)| std::ptr::eq(pad, graphic))
                || !endpoint
                    .located()
                    .is_some_and(|at| distance_squared(at, pad.at) < 1.0e-6)
                || pad.net.is_none()
            {
                continue;
            }
            let Some(layer) =
                form_atom(art.node, "layer", 1).filter(|layer| layer.ends_with(".Cu"))
            else {
                continue;
            };
            let Some(open_here) = open_by_reference.get(pad.reference.as_str()) else {
                continue;
            };
            for (open_uuid, open_pad) in open_here {
                let direct = *open_uuid == pad_uuid;
                if !direct && (kind != "shorting_items" || pad.net == open_pad.net) {
                    continue;
                }
                let target = (pad.reference.as_str(), layer);
                if !targets.contains(&target) && targets.len() == MAX_TARGETS {
                    if skipped.is_empty() {
                        skipped.push("additional footprint layers exceed the native artwork probe's target limit".into());
                    }
                    continue;
                }
                let witness = ProbeWitness {
                    footprint: pad.reference.clone(),
                    layer: layer.to_owned(),
                    open_pad_uuid: (*open_uuid).into(),
                    finding_pad_uuid: pad_uuid.clone(),
                    graphic_uuid: graphic_uuid.clone(),
                    kind: kind.into(),
                };
                if !witnesses.contains(&witness) && witnesses.len() == MAX_WITNESSES {
                    skipped.push(
                        "additional native findings exceed the artwork probe's witness limit"
                            .into(),
                    );
                    return (witnesses, skipped);
                }
                targets.insert(target);
                witnesses.insert(witness);
            }
        }
    }
    (witnesses, skipped)
}

fn matches_witness(path: &KiCadArtworkBridge, witnesses: &BTreeSet<ProbeWitness>) -> bool {
    let finding = &path.native_finding;
    let contact = if path.from.uuid == finding.pad_uuid {
        path.artwork_path.first()
    } else if path.to.uuid == finding.pad_uuid && finding.kind == "shorting_items" {
        path.artwork_path.last()
    } else {
        None
    };
    contact.is_some_and(|item| item.uuid == finding.graphic_uuid)
        && witnesses.iter().any(|witness| {
            witness.footprint == path.footprint
                && witness.layer == path.layer
                && witness.open_pad_uuid == path.from.uuid
                && witness.finding_pad_uuid == finding.pad_uuid
                && witness.graphic_uuid == finding.graphic_uuid
                && witness.kind == finding.kind
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_sibling_witness_needs_an_active_short_and_a_selected_open_on_its_owner() {
        let pcb = parse(
            r#"(kicad_pcb
            (footprint "Antenna" (layer "B.Cu") (at 0 0) (property "Reference" "AE1")
                (pad "1" smd rect (at 1 0) (net "F") (uuid "feed"))
                (pad "2" thru_hole circle (at 2 0) (net "GND") (uuid "ground"))
                (fp_poly (layer "B.Cu") (uuid "art"))))"#,
        )
        .unwrap();
        let names = NetNames::of(&pcb);
        let pads = board_pads(&pcb, &names).unwrap();
        let item =
            |id, x, description| json!({"uuid":id, "description":description, "pos":{"x":x,"y":0}});
        let mut drc = json!({
            "unconnected_items": [{"severity":"error","items":[item("feed",1,"Pad 1 [F] of AE1"),item("remote",5,"Pad 1 [F] of R1")]}],
            "violations": [{"type":"shorting_items","severity":"error","items":[item("ground",2,"PTH pad 2 [GND] of AE1"),item("art",2,"Polygon [<no net>] of AE1")]}],
        });
        let (opens, _) = native_pairs_by_net(Some(&drc), &names);
        let selected = ["F".to_owned()];
        let open = open_pad_findings::native_open_pads(&pads, &opens, &selected);
        let items = open_pad_findings::board_items(&pcb);
        let pads = pads
            .iter()
            .map(|pad| (pad.uuid.clone().unwrap(), pad))
            .collect();
        let witnesses = |drc: &serde_json::Value| native_witnesses(drc, &open, &items, &pads).0;
        let found = witnesses(&drc);
        let witness = found.first().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            (
                &*witness.open_pad_uuid,
                &*witness.finding_pad_uuid,
                &*witness.graphic_uuid
            ),
            ("feed", "ground", "art")
        );
        drc["violations"][0]["type"] = json!("clearance");
        assert!(witnesses(&drc).is_empty());
        drc["violations"][0]["type"] = json!("shorting_items");
        drc["violations"][0]["excluded"] = json!(true);
        assert!(witnesses(&drc).is_empty());
        assert!(
            native_witnesses(&drc, &BTreeMap::new(), &items, &pads)
                .0
                .is_empty()
        );
    }
}

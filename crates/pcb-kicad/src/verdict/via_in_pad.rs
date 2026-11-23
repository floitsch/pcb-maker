// Copyright (C) 2026 Florian Loitsch.

use super::*;
use crate::kicad_sexpr::{Expr, encode, form_at, unescape};

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaInPadRetry {
    pub footprint: String,
    pub pad_numbers: Vec<String>,
    pub references: Vec<String>,
    pub open_pads: Vec<KiCadEnclosedOpenPad>,
    pub router_config_patch: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadEnclosedOpenPad {
    pub reference: String,
    pub pad: String,
    pub uuid: String,
    pub net: String,
}

impl KiCadViaInPadRetry {
    pub(super) fn action(&self) -> String {
        let references = self.references.join(", ");
        let subjects = if self.references.len() == 1 {
            format!("{references} ({})", self.footprint)
        } else {
            format!("{} ({references})", self.footprint)
        };
        format!(
            "KiCad reports opens at enclosed pad{} {} on {subjects}. If the fabrication process supports filled and capped vias in pads, add the exact part entries in via_in_pad_retries to the router config's via_in_pad list. The through vias also need clearance from copper on the opposite face.",
            if self.pad_numbers.len() == 1 { "" } else { "s" },
            self.pad_numbers.join(", "),
        )
    }
}

/// Compare local land patterns. The written pad angle is board-relative.
/// Identity and assigned nets are checked separately against native opens.
fn land_pattern(footprint: &Expr) -> Option<Vec<String>> {
    let [_, _, angle] = form_at(footprint).ok()?;
    let mut pads = Vec::new();
    for pad in footprint
        .children()
        .iter()
        .filter(|item| item.head() == Some("pad"))
    {
        let [x, y, pad_angle] = form_at(pad).ok()?;
        let relative = (pad_angle - angle).rem_euclid(360.0);
        let mut fields: Vec<_> = pad
            .children()
            .iter()
            .filter(|field| {
                !matches!(
                    field.head(),
                    Some("at" | "uuid" | "tstamp" | "net" | "pinfunction" | "pintype")
                )
            })
            .cloned()
            .collect();
        fields.push(crate::parse(&format!("(at {x} {y} {relative})")).ok()?);
        pads.push(encode(&Expr::List(fields)));
    }
    pads.sort();
    Some(pads)
}

pub(super) fn grouped_retries(
    pcb: &Expr,
    report: &Value,
    planned: Option<&crate::KiCadViaInPadReport>,
) -> Vec<KiCadViaInPadRetry> {
    let Some(planned) = planned.filter(|plan| !plan.closed.is_empty()) else {
        return Vec::new();
    };
    let Some(opens) = report["unconnected_items"]
        .as_array()
        .filter(|opens| opens.len() < 499 && report[crate::outline_pieces::CAPPED_KEY] != true)
    else {
        return Vec::new();
    };
    let names = crate::note_net_names(pcb);
    let native: BTreeMap<_, _> = opens
        .iter()
        .filter(|finding| finding["severity"] == "error" && finding["excluded"] != true)
        .flat_map(|finding| finding["items"].as_array().into_iter().flatten())
        .filter_map(|item| {
            let id = item["uuid"].as_str()?.to_ascii_lowercase();
            let description = item["description"].as_str()?;
            let (reference, pad) = report_pad(description)?;
            let net = item_net(description)?;
            Some((id, (reference, pad, net)))
        })
        .collect();
    let footprints = footprints_by_reference(Some(pcb));
    let codes = crate::net_codes(pcb);
    let mut groups = BTreeMap::<_, Vec<KiCadEnclosedOpenPad>>::new();
    for closed in &planned.closed {
        if closed.pads.is_empty() {
            continue;
        }
        let Some([footprint]) = footprints.get(&closed.part).map(Vec::as_slice) else {
            continue;
        };
        if footprint.children().get(1).and_then(Expr::atom) != Some(closed.footprint.as_str()) {
            continue;
        }
        let mut open_pads = BTreeMap::new();
        for pad in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            let (Some(number), Some(net), Some(uuid)) = (
                pad.children().get(1).and_then(Expr::atom),
                crate::kicad_nets::item_net(pad, &codes),
                crate::footprint_uuid(pad),
            ) else {
                continue;
            };
            let label = format!("{number} ({})", names.normalize(net));
            if !closed.pads.contains(&label) {
                continue;
            }
            let validated =
                native
                    .get(&uuid.to_ascii_lowercase())
                    .filter(|(reference, pad, named_net)| {
                        reference == &closed.part
                            && pad == number
                            && unescape(net) == named_net.as_str()
                    });
            let value = validated.map(|(_, _, named_net)| KiCadEnclosedOpenPad {
                reference: closed.part.clone(),
                pad: number.into(),
                uuid,
                net: named_net.clone(),
            });
            open_pads
                .entry(label)
                .and_modify(|entry| *entry = None)
                .or_insert(value);
        }
        let Some(open_pads) = closed
            .pads
            .iter()
            .map(|label| open_pads.remove(label).flatten())
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let Some(pattern) = land_pattern(footprint) else {
            continue;
        };
        let mut pad_numbers: Vec<_> = open_pads.iter().map(|pad| pad.pad.clone()).collect();
        pad_numbers.sort_by(|a, b| crate::canonical_order::natural_cmp(a, b));
        groups
            .entry((closed.footprint.clone(), pad_numbers, pattern))
            .or_default()
            .extend(open_pads);
    }
    groups
        .into_iter()
        .map(|((footprint, pad_numbers, _), mut open_pads)| {
            open_pads.sort_by(|a, b| {
                crate::canonical_order::natural_cmp(&a.reference, &b.reference)
                    .then(crate::canonical_order::natural_cmp(&a.pad, &b.pad))
            });
            let mut references: Vec<_> =
                open_pads.iter().map(|pad| pad.reference.clone()).collect();
            references.dedup();
            let entries: Vec<_> = references
                .iter()
                .map(|part| serde_json::json!({"part": part}))
                .collect();
            KiCadViaInPadRetry {
                footprint,
                pad_numbers,
                references,
                open_pads,
                router_config_patch: serde_json::json!({"via_in_pad": entries}),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn retry_group_requires_same_land_pattern_and_exact_native_pad_endpoints() {
        let foot = |reference, angle, size, uuid| {
            format!(
                r#"(footprint "lib:Switch" (at 10 20 {angle}) (property "Reference" "{reference}")
                (pad "1" smd circle (at 0 0 {angle}) (net "/COL") (size {size} {size}) (layers "F.Cu") (uuid {uuid})))"#
            )
        };
        let pcb = crate::parse(&format!(
            "(kicad_pcb {} {} {} {} {} {})",
            foot("SW2", 0, 4, "sw2"),
            foot("SW3", 180, 4, "sw3"),
            foot("SW4", 0, 3, "sw4"),
            foot("SW5", 0, 4, "sw5"),
            foot("SW6", 0, 4, "sw6a"),
            foot("SW6", 0, 4, "sw6b"),
        ))
        .unwrap();
        let closed = ["SW2", "SW3", "SW4", "SW5", "SW6"]
            .into_iter()
            .map(|part| crate::KiCadClosedPads {
                part: part.into(),
                footprint: "lib:Switch".into(),
                pads: vec!["1 (COL)".into()],
                nets: vec!["COL".into()],
            })
            .collect();
        let planned = crate::KiCadViaInPadReport {
            closed,
            ..Default::default()
        };
        let opens: Vec<_> = [
            ("SW2", "sw2"),
            ("SW3", "sw3"),
            ("SW4", "sw4"),
            ("SW5", "a-different-land"),
            ("SW6", "sw6a"),
        ]
        .into_iter()
        .map(|(reference, uuid)| {
            json!({"severity": "error", "items": [{
                "uuid": uuid, "description": format!("Pad 1 [/COL] of {reference} on F.Cu"),
            }]})
        })
        .collect();
        let retries = grouped_retries(&pcb, &json!({"unconnected_items": opens}), Some(&planned));
        assert_eq!(retries.len(), 2);
        let repeated = retries
            .iter()
            .find(|retry| retry.references.len() == 2)
            .unwrap();
        assert_eq!(repeated.references, ["SW2", "SW3"]);
        assert_eq!(
            repeated.router_config_patch,
            json!({"via_in_pad": [{"part": "SW2"}, {"part": "SW3"}]})
        );
        let lone = retries
            .iter()
            .find(|retry| retry.references.len() == 1)
            .unwrap();
        assert_eq!(lone.references, ["SW4"]);
    }
}

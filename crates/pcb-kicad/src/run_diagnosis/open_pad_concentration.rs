// Copyright (C) 2026 Florian Loitsch.

use super::*;

/// A concentration of native open nets at singleton pads on the saved board.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadOpenPadConcentration {
    pub native_open_nets: usize,
    pub concentrated_nets: usize,
    pub footprints: Vec<KiCadOpenPadFootprint>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadOpenPadFootprint {
    pub reference: String,
    pub distinct_nets: usize,
    pub pads: Vec<KiCadOpenPadTerminal>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct KiCadOpenPadTerminal {
    pub pad: String,
    pub net: String,
    pub pad_uuid: String,
}

const MIN_NETS_AT_FOOTPRINT: usize = 4;
const MAX_FOOTPRINTS: usize = 3;

fn distinct_nets(pads: &[KiCadOpenPadTerminal]) -> BTreeSet<&str> {
    pads.iter().map(|pin| pin.net.as_str()).collect()
}

/// Native unconnected items identify physical pads by UUID. Confirm those
/// pads are alone in their saved copper component with the same connectivity
/// as `inspect-routing-run`. Ordinary pours are not modelled by that graph.
pub(crate) fn measure_open_pad_concentration(
    pcb: Expr,
    config: &KiCadBoardRouterConfig,
    drc: &serde_json::Value,
) -> Option<KiCadOpenPadConcentration> {
    let item_count = drc["unconnected_items"].as_array()?.len();
    if !(MIN_NETS_AT_FOOTPRINT..KICAD_REPORT_CAP).contains(&item_count) {
        return None;
    }
    let names = NetNames::of(&pcb);
    let (native, unattributed) = native_pairs_by_net(Some(drc), &names);
    if unattributed > 0 || native.len() < MIN_NETS_AT_FOOTPRINT {
        return None;
    }
    let pads = board_pads(&pcb, &names).ok()?;
    let mut refs = BTreeMap::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        if let Some(reference) = footprint_reference(footprint) {
            *refs.entry(unquoted(&reference)).or_insert(0usize) += 1;
        }
    }
    let mut pads_by_uuid = BTreeMap::new();
    for pad in &pads {
        if let Some(uuid) = pad.uuid.as_deref()
            && pads_by_uuid.insert(uuid, pad).is_some()
        {
            return None;
        }
    }
    let mut candidates: BTreeMap<String, Vec<KiCadOpenPadTerminal>> = BTreeMap::new();
    for (net, pairs) in &native {
        for item in pairs.iter().flat_map(|pair| &pair.items) {
            let Some(uuid) = item.uuid.as_deref() else {
                continue;
            };
            let Some(pad) = pads_by_uuid.get(uuid).filter(|pad| {
                pad.net.as_ref() == Some(net)
                    && !pad.number.is_empty()
                    && refs.get(&pad.reference) == Some(&1)
            }) else {
                continue;
            };
            candidates
                .entry(pad.reference.clone())
                .or_default()
                .push(KiCadOpenPadTerminal {
                    pad: pad.number.clone(),
                    net: net.clone(),
                    pad_uuid: uuid.into(),
                });
        }
    }
    candidates.retain(|_, pads| {
        pads.sort_unstable();
        pads.dedup();
        distinct_nets(pads).len() >= MIN_NETS_AT_FOOTPRINT
    });
    let nets: BTreeSet<&str> = candidates
        .values()
        .flat_map(|pads| distinct_nets(pads))
        .collect();
    if nets.len() < native.len().div_ceil(2) {
        return None;
    }
    let result = LoweredBoard::new(pcb, config, names, pads).ok()?;
    let mut isolated = BTreeSet::new();
    for name in nets {
        let Some(net) = result.net_id(name) else {
            continue;
        };
        if !result.pour_layers(net).is_empty() || result.teardrops.unmodelled.contains(&net) {
            continue;
        }
        let components = result.components(net);
        if components.len() < 2 {
            continue;
        }
        isolated.extend(components.into_iter().filter_map(|component| {
            let [pad] = component.pads.as_slice() else {
                return None;
            };
            if component.other.is_empty() && pad.layers.len() == 1 {
                pad.uuid.clone()
            } else {
                None
            }
        }));
    }
    let mut footprints = Vec::new();
    for (reference, mut pads) in candidates {
        pads.retain(|pad| isolated.contains(&pad.pad_uuid));
        let distinct_nets = distinct_nets(&pads).len();
        if distinct_nets >= MIN_NETS_AT_FOOTPRINT {
            footprints.push(KiCadOpenPadFootprint {
                reference,
                distinct_nets,
                pads,
            });
        }
    }
    footprints.sort_by(|a, b| {
        b.distinct_nets
            .cmp(&a.distinct_nets)
            .then(a.reference.cmp(&b.reference))
    });
    footprints.truncate(MAX_FOOTPRINTS);
    let concentrated_nets = footprints
        .iter()
        .flat_map(|part| distinct_nets(&part.pads))
        .collect::<BTreeSet<_>>()
        .len();
    (concentrated_nets >= native.len().div_ceil(2)).then_some(KiCadOpenPadConcentration {
        native_open_nets: native.len(),
        concentrated_nets,
        footprints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"(kicad_pcb (version 20241229)
        (general (thickness 1.6)) (paper "A4")
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
        (gr_rect (start 0 0) (end 20 20) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
        (footprint "fixture" (layer "F.Cu") (at 6 6) (property "Reference" "U1") %PINS%)
        %OTHER%)"#;

    #[test]
    fn recognizes_isolated_native_pads_and_excludes_common_connected_or_power_pads() {
        let mut pins = String::new();
        let mut remotes = String::new();
        let mut local_tracks = String::new();
        let mut opens = Vec::new();
        for pin in 1..=4 {
            let net = format!("NET{pin}");
            let y = 6 + pin;
            let uuid = |prefix| format!("{prefix}0010011-0000-4000-8000-00000000000{pin}");
            let pad = |prefix, number, pos| {
                format!(
                    r#"(pad "{number}" smd rect (at 0 {pos}) (size 0.5 0.5) (layers "F.Cu") (net "{net}") (uuid "{}"))"#,
                    uuid(prefix)
                )
            };
            pins.push_str(&pad('a', pin, pin));
            let part = |ref_prefix, x, pad_prefix| {
                format!(
                    r#"(footprint "fixture" (layer "F.Cu") (at {x} {y}) (property "Reference" "{ref_prefix}{pin}") {})"#,
                    pad(pad_prefix, 1, 0)
                )
            };
            remotes.push_str(&part('R', 16, 'b'));
            local_tracks.push_str(&part('C', 9, 'c'));
            local_tracks.push_str(&format!(
                r#"(segment (start 6 {y}) (end 9 {y}) (width 0.2) (layer "F.Cu") (net "{net}") (uuid "{}"))"#,
                uuid('d')
            ));
            opens.push(serde_json::json!({"items": [
                {"description": format!("Pad {pin} [{net}] of U1 on F.Cu"), "uuid": uuid('a')},
                {"description": format!("Pad 1 [{net}] of R{pin} on F.Cu"), "uuid": uuid('b')},
            ]}));
        }
        let board = BOARD.replace("%PINS%", &pins).replace("%OTHER%", &remotes);
        let with_tracks = format!("{}{local_tracks})", board.strip_suffix(')').unwrap());
        let drc = serde_json::json!({"unconnected_items": opens});
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..Default::default()
        };
        let measure = |board: &str, report: &serde_json::Value| {
            measure_open_pad_concentration(parse(board).unwrap(), &config, report)
        };
        let found = measure(&board, &drc).unwrap();
        assert_eq!(found.concentrated_nets, 4);
        assert_eq!(found.native_open_nets, 4);
        assert_eq!(found.footprints.len(), 1);
        assert_eq!(found.footprints[0].reference, "U1");
        assert_eq!(found.footprints[0].pads.len(), 4);
        assert!(measure(&with_tracks, &drc).is_none());
        let repeated = format!(
            r#"{} (footprint "fixture" (layer "F.Cu") (at 5 5) (property "Reference" "U1")))"#,
            board.strip_suffix(')').unwrap()
        );
        assert!(measure(&repeated, &drc).is_none());
        let mut missing_id = drc.clone();
        missing_id["unconnected_items"][0]["items"][0]
            .as_object_mut()
            .unwrap()
            .remove("uuid");
        assert!(measure(&board, &missing_id).is_none());
        let power = |s: &str| {
            (1..=4).fold(s.to_owned(), |s, pin| {
                s.replace(&format!("NET{pin}"), "GND")
            })
        };
        assert!(
            measure(
                &power(&board),
                &serde_json::from_str(&power(&drc.to_string())).unwrap()
            )
            .is_none()
        );
        assert!(measure(&board, &serde_json::json!({"unconnected_items": []})).is_none());
    }
}

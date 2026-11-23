// Copyright (C) 2026 Florian Loitsch.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Evidence for missing PCB pads from the schematic KiCad just checked.

use super::{
    KiCadMissingSchematicPad, KiCadSchematicPinConnection, KiCadSchematicPinEvidence, named_net,
};
use crate::DrcTally;
use pcb_kicad_file::kicad_sexpr::{Expr, form_atom};
use std::{collections::BTreeMap, path::Path};

pub(in crate::verdict) fn note_schematic_pins(tally: &mut DrcTally, schematic: &Path) {
    if !tally
        .missing_schematic_pads
        .iter()
        .any(|pad| pad.reference.is_some())
        || !schematic.is_file()
    {
        return;
    }
    if let Ok(netlist) = crate::native_report::export_schematic_netlist(schematic) {
        annotate(&mut tally.missing_schematic_pads, &netlist);
    }
}

#[derive(Clone, Copy)]
struct Terminal<'a> {
    net: &'a str,
    terminals: usize,
    pin_type: Option<&'a str>,
}

impl Terminal<'_> {
    fn evidence(self) -> Option<KiCadSchematicPinEvidence> {
        let connection = if self.terminals > 1 || named_net(self.net) {
            KiCadSchematicPinConnection::Connected
        } else if self
            .net
            .trim_start_matches('/')
            .starts_with("unconnected-(")
            && self.net.ends_with(')')
        {
            match self.pin_type {
                Some("no_connect") => KiCadSchematicPinConnection::NoConnectPinType,
                Some(pin_type) if pin_type.split('+').skip(1).any(|kind| kind == "no_connect") => {
                    KiCadSchematicPinConnection::NoConnectMarker
                }
                _ => KiCadSchematicPinConnection::Isolated,
            }
        } else {
            return None;
        };
        Some(KiCadSchematicPinEvidence {
            connection,
            net: self.net.into(),
            terminals: self.terminals,
            pin_type: self.pin_type.map(str::to_owned),
        })
    }
}

fn annotate(missing: &mut [KiCadMissingSchematicPad], netlist: &Expr) {
    if netlist.head() != Some("export") {
        return;
    }
    let (Some(components), Some(nets)) = (netlist.child("components"), netlist.child("nets"))
    else {
        return;
    };
    let mut references = BTreeMap::new();
    for component in components
        .children()
        .iter()
        .filter(|row| row.head() == Some("comp"))
    {
        if let Some(reference) = form_atom(component, "ref", 1) {
            *references.entry(reference).or_insert(0) += 1;
        }
    }
    let mut membership = BTreeMap::new();
    for net in nets
        .children()
        .iter()
        .filter(|row| row.head() == Some("net"))
    {
        let Some(name) = form_atom(net, "name", 1).filter(|name| !name.is_empty()) else {
            return;
        };
        let nodes = || {
            net.children()
                .iter()
                .filter(|node| node.head() == Some("node"))
        };
        let terminals = nodes().count();
        for node in nodes() {
            let (Some(reference), Some(pin)) =
                (form_atom(node, "ref", 1), form_atom(node, "pin", 1))
            else {
                return;
            };
            let terminal = Terminal {
                net: name,
                terminals,
                pin_type: form_atom(node, "pintype", 1),
            };
            membership
                .entry((reference, pin))
                .and_modify(|target| *target = None)
                .or_insert(Some(terminal));
        }
    }
    for pad in missing {
        let Some(reference) = pad
            .reference
            .as_deref()
            .filter(|r| references.get(r) == Some(&1))
        else {
            continue;
        };
        let Some(Some(terminal)) = membership.get(&(reference, pad.pin.as_str())) else {
            continue;
        };
        // The native finding sometimes includes the net. Require it to
        // agree with the export when it names one.
        if pad.net.as_deref().is_some_and(|name| {
            name.trim_start_matches('/') != terminal.net.trim_start_matches('/')
        }) {
            continue;
        }
        pad.schematic_netlist = terminal.evidence();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tally_drc;
    use serde_json::json;

    #[test]
    fn fresh_netlist_distinguishes_marked_isolated_and_connected_pins() {
        use KiCadSchematicPinConnection::*;
        let findings = json!({
            "schematic_parity": (1..=6)
                .map(|pin| json!({
                    "type": "net_conflict",
                    "description": format!("No pad found for pin {pin} in symbol U1."),
                }))
                .collect::<Vec<_>>(),
        });
        let mut tally = tally_drc(&findings);
        let netlist = crate::parse(
            r#"(export
                (components (comp (ref U1)) (comp (ref R1)))
                (nets
                    (net (name "unconnected-(U1-Pad1)")
                        (node (ref U1) (pin 1) (pintype "input+no_connect")))
                    (net (name "unconnected-(U1-Pad2)")
                        (node (ref U1) (pin 2) (pintype no_connect)))
                    (net (name "unconnected-(U1-NC-Pad3)")
                        (node (ref U1) (pin 3) (pintype input)))
                    (net (name "unconnected-(ordinary)")
                        (node (ref U1) (pin 4) (pintype "input+no_connect"))
                        (node (ref R1) (pin 1) (pintype passive)))
                    (net (name "LED_ON")
                        (node (ref U1) (pin 5) (pintype output)))))"#,
        )
        .unwrap();

        let before = super::super::problems(&tally);
        annotate(&mut tally.missing_schematic_pads, &netlist);
        assert_eq!(
            tally
                .missing_schematic_pads
                .iter()
                .map(|pad| pad
                    .schematic_netlist
                    .as_ref()
                    .map(|evidence| evidence.connection))
                .collect::<Vec<_>>(),
            vec![
                Some(NoConnectMarker),
                Some(NoConnectPinType),
                Some(Isolated),
                Some(Connected),
                Some(Connected),
                None,
            ]
        );
        let actions = super::super::actions(&tally, "drc.json");
        assert!(actions[0].contains("U1:4 (unconnected-(ordinary)), U1:5 (LED_ON)"));
        assert!(actions[1].contains("U1:1 with no-connect markers"));
        assert!(actions[2].contains("U1:2 with the no-connect symbol pin type"));
        assert!(actions[3].contains("U1:3 on single-pin unconnected nets"));
        assert!(actions[4].contains("U1:6"));
        assert_eq!(super::super::problems(&tally), before);
        assert_eq!(tally.schematic_parity, 6);
    }

    #[test]
    fn ambiguous_or_disagreeing_pins_keep_the_native_only_guidance() {
        let findings = json!({
            "schematic_parity": [
                {"type": "net_conflict", "description": "No pad found for pin 1 in symbol U1."},
                {"type": "net_conflict", "description": "No pad found for pin 2 in symbol U2."},
                {"type": "net_conflict", "description": "No pad found for pin 3 (SDA) in symbol U3."},
            ]
        });
        let mut tally = tally_drc(&findings);
        let netlist = crate::parse(
            r#"(export
                (components (comp (ref U1)) (comp (ref U1)) (comp (ref U2)) (comp (ref U3)))
                (nets
                    (net (name "unconnected-(U1-Pad1)")
                        (node (ref U1) (pin 1) (pintype "input+no_connect")))
                    (net (name "unconnected-(U2-Pad2)")
                        (node (ref U2) (pin 2) (pintype "input+no_connect")))
                    (net (name "SPI")
                        (node (ref U2) (pin 2)))
                    (net (name "unconnected-(U3-Pad3)")
                        (node (ref U3) (pin 3) (pintype "input+no_connect")))))"#,
        )
        .unwrap();
        annotate(&mut tally.missing_schematic_pads, &netlist);
        assert!(
            tally
                .missing_schematic_pads
                .iter()
                .all(|pad| pad.schematic_netlist.is_none())
        );
    }
}

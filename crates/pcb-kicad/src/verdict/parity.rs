// Copyright (C) 2026 Florian Loitsch.

//! Evidence from KiCad's schematic-to-PCB parity check.

use super::*;

mod footprint_filters;
mod netlist;
pub use footprint_filters::KiCadFootprintFilterMismatch;
pub(super) use footprint_filters::matches_on_board as footprint_filter_matches;
pub(super) use netlist::note_schematic_pins;

/// A schematic component for which KiCad found no PCB footprint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KiCadMissingSchematicFootprint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub description: String,
}

impl KiCadMissingSchematicFootprint {
    fn from_finding(finding: &Value) -> Self {
        let description = finding["description"].as_str().unwrap_or_default();
        // Native findings for absent footprints have no PCB items. KiCad puts
        // the reference and symbol value in the description.
        let (reference, value) = description
            .strip_prefix("Missing footprint ")
            .and_then(|label| label.split_once(" ("))
            .and_then(|(reference, value)| {
                let value = value.strip_suffix(')')?;
                (!reference.is_empty() && !reference.contains(char::is_whitespace))
                    .then_some((Some(reference.into()), Some(value.into())))
            })
            .unwrap_or_default();
        Self {
            reference,
            value,
            description: description.into(),
        }
    }
}

/// A symbol pin for which KiCad found no pad on the PCB footprint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KiCadMissingSchematicPad {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    pub pin: String,
    /// The net as KiCad's current parity diagnostic names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    /// Pin membership from a fresh KiCad export of the checked schematic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schematic_netlist: Option<KiCadSchematicPinEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KiCadSchematicPinEvidence {
    pub connection: KiCadSchematicPinConnection,
    pub net: String,
    pub terminals: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_type: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadSchematicPinConnection {
    NoConnectMarker,
    NoConnectPinType,
    Isolated,
    Connected,
}

impl KiCadMissingSchematicPad {
    pub(super) fn from_finding(finding: &Value) -> Option<Self> {
        if finding["type"].as_str() != Some("net_conflict") {
            return None;
        }
        let description = finding["description"]
            .as_str()?
            .trim_end_matches('.')
            .strip_prefix("No pad found for pin ")?;
        let (terminal, described_reference) =
            if let Some(terminal) = description.strip_suffix(" in schematic") {
                (terminal, None)
            } else {
                let (terminal, reference) = description.split_once(" in symbol ")?;
                (terminal, Some(reference))
            };
        let (pin, net) = if let Some((pin, net)) = terminal.split_once(" (") {
            (pin, Some(net.strip_suffix(')')?.trim()))
        } else {
            (terminal, None)
        };
        let pin = pin.trim();
        if pin.is_empty() {
            return None;
        }
        let item_reference = match finding["items"].as_array().map(Vec::as_slice) {
            Some([item]) => item["description"]
                .as_str()
                .and_then(|label| label.strip_prefix("Footprint ")),
            _ => None,
        };
        Some(Self {
            reference: item_reference
                .or(described_reference)
                .filter(|name| !name.is_empty())
                .map(str::to_string),
            pin: pin.into(),
            net: net.filter(|name| !name.is_empty()).map(str::to_string),
            schematic_netlist: None,
        })
    }

    pub(super) fn live_net(&self) -> Option<&str> {
        self.net.as_deref().filter(|name| named_net(name))
    }

    fn action_net(&self) -> Option<&str> {
        self.schematic_netlist
            .as_ref()
            .filter(|evidence| evidence.connection == KiCadSchematicPinConnection::Connected)
            .map(|evidence| evidence.net.as_str())
            .or_else(|| self.live_net())
    }

    pub(super) fn label(&self) -> String {
        match &self.reference {
            Some(reference) => format!("{reference}:{}", self.pin),
            None => format!("schematic pin {}", self.pin),
        }
    }
}

/// A pad number whose copper is assigned to a different net from its symbol.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct KiCadSchematicNetMismatch {
    pub reference: String,
    pub pin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pcb_net: Option<String>,
    pub schematic_net: String,
    /// The number of native PCB land findings at this logical pad number.
    pub physical_pads: usize,
}

impl KiCadSchematicNetMismatch {
    fn from_finding(finding: &Value) -> Option<Self> {
        if finding["type"].as_str() != Some("net_conflict") {
            return None;
        }
        let description = finding["description"]
            .as_str()?
            .trim_end_matches('.')
            .strip_suffix(')')?;
        let (pcb_net, schematic_net) = if let Some(mismatch) = description.strip_prefix("Pad net (")
        {
            let (actual, expected) =
                mismatch.split_once(") doesn't match net given by schematic (")?;
            (Some(actual), expected)
        } else {
            (
                None,
                description.strip_prefix("Pad missing net given by schematic (")?,
            )
        };
        if pcb_net == Some(schematic_net)
            || !(pcb_net.is_some_and(named_net) || named_net(schematic_net))
        {
            return None;
        }
        let [item] = finding["items"].as_array()?.as_slice() else {
            return None;
        };
        let (reference, pin) = report_pad(item["description"].as_str()?)?;
        Some(Self {
            reference,
            pin,
            pcb_net: pcb_net.map(str::to_string),
            schematic_net: schematic_net.into(),
            physical_pads: 0,
        })
    }
}

fn named_net(name: &str) -> bool {
    let name = name.trim().trim_start_matches('/');
    !matches!(name, "" | "<no net>" | "<none>") && !name.starts_with("unconnected-(")
}

pub(super) fn tally(report: &Value, tally: &mut DrcTally) {
    let Some(findings) = report["schematic_parity"].as_array() else {
        return;
    };
    tally.schematic_parity = findings.len();
    let mut wrong_nets = BTreeMap::new();
    for finding in findings {
        let kind = finding["type"].as_str().unwrap_or("unknown");
        *tally
            .schematic_parity_types
            .entry(kind.to_string())
            .or_default() += 1;
        if kind == "missing_footprint" {
            tally
                .missing_schematic_footprints
                .push(KiCadMissingSchematicFootprint::from_finding(finding));
        } else if let Some(pin) = KiCadMissingSchematicPad::from_finding(finding) {
            tally.missing_schematic_pads.push(pin);
        } else if let Some(mismatch) = KiCadSchematicNetMismatch::from_finding(finding) {
            *wrong_nets.entry(mismatch).or_insert(0) += 1;
        }
    }
    tally.schematic_net_mismatches = wrong_nets
        .into_iter()
        .map(|(mut item, pads)| {
            item.physical_pads = pads;
            item
        })
        .collect();
}

pub(super) fn missing_live_pads(
    tally: &DrcTally,
) -> impl Iterator<Item = (&KiCadMissingSchematicPad, &str)> {
    tally
        .missing_schematic_pads
        .iter()
        .filter_map(|pad| pad.live_net().map(|net| (pad, net)))
}

pub(super) fn problems(native: &DrcTally) -> Vec<String> {
    let absent = native.missing_schematic_footprints.len();
    let missing = missing_live_pads(native).count();
    let wrong = native.schematic_net_mismatches.len();
    let mut problems = Vec::new();
    if absent == 1 {
        problems.push("1 schematic component is missing its PCB footprint".into());
    } else if absent > 1 {
        problems.push(format!(
            "{absent} schematic components are missing their PCB footprints"
        ));
    }
    if missing == 1 {
        problems.push("1 schematic pin on a named net has no PCB pad".into());
    } else if missing > 1 {
        problems.push(format!(
            "{missing} schematic pins on named nets have no PCB pad"
        ));
    }
    if wrong == 1 {
        problems.push("1 PCB pad number has a different net from the schematic".into());
    } else if wrong > 1 {
        problems.push(format!(
            "{wrong} PCB pad numbers have different nets from the schematic"
        ));
    }
    problems
}

pub(super) fn actions(native: &DrcTally, drc: &str) -> Vec<String> {
    let live = native
        .missing_schematic_pads
        .iter()
        .filter_map(|pad| pad.action_net().map(|net| (pad, net)))
        .map(|(pad, net)| format!("{} ({net})", pad.label()))
        .collect::<Vec<_>>();
    let mut actions = Vec::new();
    if !native.missing_schematic_footprints.is_empty() {
        let labels = native
            .missing_schematic_footprints
            .iter()
            .map(|footprint| {
                footprint
                    .reference
                    .as_deref()
                    .unwrap_or("unknown reference")
            })
            .map(str::to_string)
            .collect::<Vec<_>>();
        actions.push(format!(
            "restore the missing PCB footprint for {} by updating the PCB from the current schematic. Check the symbol's footprint assignment and intended placement. For intentionally schematic-only components, set Exclude from board on the symbol. Rerun parity and routing. Native details are in {drc}",
            brief(&labels, 6),
        ));
    }
    if !live.is_empty() {
        actions.push(format!(
            "fix the PCB pad mapping for {}. KiCad reports these schematic net terminals have no matching PCB pad. Select a matching footprint or correct the pin numbering, then update the PCB from the current schematic and rerun parity",
            brief(&live, 6),
        ));
    }
    if !native.schematic_net_mismatches.is_empty() {
        let labels = native
            .schematic_net_mismatches
            .iter()
            .map(|item| {
                let land = if item.physical_pads == 1 {
                    "pad"
                } else {
                    "pads"
                };
                format!(
                    "{}:{} (PCB {}, schematic {}, {} physical {land})",
                    item.reference,
                    item.pin,
                    item.pcb_net.as_deref().unwrap_or("unassigned"),
                    item.schematic_net,
                    item.physical_pads
                )
            })
            .collect::<Vec<_>>();
        actions.push(format!(
            "update the PCB from the current schematic for {}: verify the intended pin mapping and reroute those nets, then rerun parity",
            brief(&labels, 6),
        ));
    }
    let non_live = native
        .missing_schematic_pads
        .iter()
        .filter(|pad| pad.action_net().is_none());
    for (connection, description) in [
        (
            KiCadSchematicPinConnection::NoConnectMarker,
            "with no-connect markers",
        ),
        (
            KiCadSchematicPinConnection::NoConnectPinType,
            "with the no-connect symbol pin type",
        ),
        (
            KiCadSchematicPinConnection::Isolated,
            "on single-pin unconnected nets",
        ),
    ] {
        let labels = non_live
            .clone()
            .filter(|pad| {
                pad.schematic_netlist
                    .as_ref()
                    .is_some_and(|evidence| evidence.connection == connection)
            })
            .map(KiCadMissingSchematicPad::label)
            .collect::<Vec<_>>();
        if !labels.is_empty() {
            actions.push(format!(
                "KiCad's current schematic netlist lists {} {description}. Parity still reports missing PCB pads. Verify that the chosen package exposes the intended pins and correct any symbol or footprint mismatch, then rerun parity",
                brief(&labels, 6),
            ));
        }
    }
    let other = non_live
        .filter(|pad| pad.schematic_netlist.is_none())
        .map(KiCadMissingSchematicPad::label)
        .collect::<Vec<_>>();
    if !other.is_empty() {
        actions.push(format!(
            "review the footprint and schematic pin mapping for {}: KiCad found no matching PCB pad. Check whether the current schematic leaves these pins unwired or the selected package needs a PCB pad",
            brief(&other, 6),
        ));
    }
    if !native.footprint_filter_mismatches.is_empty() {
        let labels = native
            .footprint_filter_mismatches
            .iter()
            .map(|group| {
                format!(
                    "{} (PCB {}, symbol filters {})",
                    brief(&group.references, 4),
                    group.footprint_id,
                    group.symbol_filters
                )
            })
            .collect::<Vec<_>>();
        actions.push(format!(
            "KiCad reports footprint filter mismatches. {}. Compare the pin numbers, pad mapping and physical package with the intended part. Correct the assigned footprint in the source schematic when the package differs. When the footprint is verified compatible, correct the intended symbol's footprint filter in the source. Update the PCB from that schematic and rerun parity. footprint_filter_mismatches lists all affected references.",
            brief(&labels, 3)
        ));
    }
    let described = native.missing_schematic_footprints.len()
        + native.missing_schematic_pads.len()
        + native
            .schematic_net_mismatches
            .iter()
            .map(|item| item.physical_pads)
            .sum::<usize>()
        + native
            .footprint_filter_mismatches
            .iter()
            .map(|group| group.findings)
            .sum::<usize>();
    if native.schematic_parity > described {
        actions.push(format!(
            "review the schematic parity findings ({}) in {drc} against the current schematic and PCB",
            counts(&native.schematic_parity_types),
        ));
    }
    actions
}

pub(super) fn brief(labels: &[String], limit: usize) -> String {
    let shown = labels[..labels.len().min(limit)].join(", ");
    if labels.len() > limit {
        format!("{shown}, and {} more", labels.len() - limit)
    } else {
        shown
    }
}

// Copyright (C) 2026 Toit contributors.

//! A starter constraints.json for a board (`describe-kicad-board
//! --constraints-template`): the plug-in connectors as edge candidates, each
//! decoupling capacitor near the closest supply pin of its IC, and comments
//! that say what the agent decides.

use super::*;
use crate::quality::{capacitance, ground, rail_name};

/// A footprint of the board as the template needs it.
struct Part {
    reference: String,
    name: String,
    at: [f64; 3],
    /// Pad name, net (normalized), centre on the board, supply pin
    /// (`supply_pin`).
    pads: Vec<(String, Option<String>, [f64; 2], bool)>,
    value: String,
    body: Option<[f64; 4]>,
}

fn parts_of(pcb: &Expr) -> Result<Vec<Part>, String> {
    let names = NetNames::of(pcb);
    let mut parts = Vec::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let Some(reference) = footprint_reference(footprint) else {
            continue;
        };
        let at = form_at(footprint)?;
        let mut pads = Vec::new();
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            let Ok(position) = form_xy(pad, "at") else {
                continue;
            };
            let offset = crate::geometry::rotate_vector(position, -at[2]);
            let name = pad
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or("")
                .to_string();
            let net = node_net(pad)
                .filter(|net| !net.is_empty())
                .map(|net| names.normalize(net).to_string());
            let supply = net.as_deref().is_some_and(|net| {
                supply_pin(
                    form_atom(pad, "pinfunction", 1),
                    form_atom(pad, "pintype", 1),
                    &name,
                    net,
                )
            });
            pads.push((name, net, [at[0] + offset[0], at[1] + offset[1]], supply));
        }
        let value = footprint
            .children()
            .iter()
            .find(|item| {
                item.head() == Some("property")
                    && item.children().get(1).and_then(Expr::atom) == Some("Value")
            })
            .and_then(|item| item.children().get(2).and_then(Expr::atom))
            .unwrap_or("")
            .to_string();
        parts.push(Part {
            reference,
            name: footprint
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or("")
                .to_string(),
            at,
            pads,
            value,
            body: crate::placement_constraints::part_box(footprint)?,
        });
    }
    Ok(parts)
}

/// A pin function without KiCad's appended pad number and markup
/// (`EN_3` -> `EN`, `~{SHDN}` -> `SHDN`), upper case.
fn pin_function(function: &str, pad: &str) -> String {
    let function = function
        .strip_suffix(&format!("_{pad}"))
        .unwrap_or(function);
    function
        .replace("~{", "")
        .replace(['{', '}', '~'], "")
        .trim()
        .to_ascii_uppercase()
}

/// Whether a pin function names an enable, shutdown or other control
/// input (`EN`, `~{SHDN}`, `CE`, `ON/OFF`, `SLEEP`), which a rail may drive
/// but no capacitor decouples.
fn control_function(function: &str) -> bool {
    const CONTROLS: [&str; 18] = [
        "EN", "ENA", "ENABLE", "SHDN", "SD", "CE", "ON", "ON/OFF", "RUN", "SLEEP", "RESET", "RST",
        "PG", "PGOOD", "MODE", "SYNC", "CS", "SEL",
    ];
    let control = |name: &str| {
        CONTROLS.iter().any(|control| {
            name.strip_prefix(control)
                .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
        })
    };
    // `nCE`, `nSHDN`: active low.
    control(function) || function.strip_prefix('N').is_some_and(control)
}

/// Whether a pin function names a supply (`VDD`, `VCC`, `AVDD`, `VIN`,
/// `VBAT`, `VM`, `3V3`, ...).
fn supply_function(function: &str) -> bool {
    let name = function.trim_start_matches('+');
    [
        "VDD", "VCC", "AVDD", "DVDD", "PVDD", "IOVDD", "AVCC", "DVCC", "VIN", "PVIN", "VBAT",
        "BAT", "VBUS", "VSYS", "VREG", "VOUT", "VDDIO", "VDDA", "VCCIO", "VPP",
    ]
    .iter()
    .any(|supply| name.starts_with(supply))
        || ["VM", "VS", "V+", "VCCA", "VCCB"].iter().any(|supply| {
            name.strip_prefix(supply).is_some_and(|rest| {
                rest.is_empty() || rest.chars().all(|c| c.is_ascii_digit() || c == '_')
            })
        })
        || name.split_once('V').is_some_and(|(volts, rest)| {
            !volts.is_empty()
                && volts.chars().all(|c| c.is_ascii_digit())
                && rest.chars().all(|c| c.is_ascii_digit())
        })
}

/// Whether an IC's pad on a (non-ground) net is a supply pin a decoupling
/// capacitor belongs at: by the pin's function where the board names it
/// (never an enable or other control input tied to the rail: the
/// schematic trial's C7, 22 uF, went to the LDO's EN pin), else by its
/// electrical type (power in or out; a logic input or output tied to a
/// rail is none), else by the net's name (`rail_name`).
fn supply_pin(function: Option<&str>, pintype: Option<&str>, pad: &str, net: &str) -> bool {
    if let Some(function) = function.map(|function| pin_function(function, pad))
        && !function.is_empty()
    {
        if control_function(&function) {
            return false;
        }
        if supply_function(&function) {
            return true;
        }
    }
    let pintype = pintype.unwrap_or("");
    if pintype.starts_with("power_in") || pintype.starts_with("power_out") {
        return true;
    }
    if [
        "input",
        "output",
        "bidirectional",
        "tri_state",
        "open_collector",
        "open_emitter",
    ]
    .iter()
    .any(|logic| pintype.starts_with(logic))
    {
        return false;
    }
    rail_name(net)
}

/// The reference's letters (`U` of `U12`).
fn prefix(reference: &str) -> &str {
    reference.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_')
}

/// Where the project's schematics (every `.kicad_sch` beside the board)
/// draw each symbol: per reference, the sheet instance it is on (file and
/// instance path) and its position there.
fn schematic_positions(directory: &Path) -> BTreeMap<String, (String, [f64; 2])> {
    let mut positions = BTreeMap::new();
    let Ok(entries) = fs::read_dir(directory) else {
        return positions;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "kicad_sch")
        })
        .collect();
    files.sort();
    for file in files {
        let Some(schematic) = fs::read_to_string(&file)
            .ok()
            .and_then(|text| parse(&text).ok())
        else {
            continue;
        };
        let sheet = file
            .file_name()
            .map_or(String::new(), |name| name.to_string_lossy().to_string());
        for symbol in schematic
            .children()
            .iter()
            .filter(|item| item.head() == Some("symbol"))
        {
            let Ok(at) = form_xy(symbol, "at") else {
                continue;
            };
            let instances = symbol
                .child("instances")
                .map(Expr::children)
                .unwrap_or_default();
            for path in instances
                .iter()
                .filter(|project| project.head() == Some("project"))
                .flat_map(|project| project.children())
                .filter(|path| path.head() == Some("path"))
            {
                if let (Some(instance), Some(reference)) = (
                    path.children().get(1).and_then(Expr::atom),
                    form_atom(path, "reference", 1),
                ) {
                    positions
                        .entry(reference.to_string())
                        .or_insert((format!("{sheet}{instance}"), at));
                }
            }
        }
    }
    positions
}

/// Each capacitor's supply pin by the schematic (`positions`): first the IC
/// of its rail drawn nearest to it on its sheet instance; then an IC of the
/// rail that got none takes an extra of an IC with more capacitors than
/// supply pins on that rail (the extra drawn farthest from its IC, to the
/// IC nearest to it), until none is left without or none has extras; at
/// each IC its capacitors go to its pins in turn. Capacitors the schematic
/// does not place stay out (the caller assigns them in turn). The schematic
/// trial: C7 and C8 (22 uF on VDD) both drawn nearest the second motor
/// driver U6; U5 takes C8, as the designer's board has them.
fn schematic_assignment(
    capacitors: &[(String, String)],
    supplies: &BTreeMap<String, Vec<(String, [f64; 2])>>,
    positions: &BTreeMap<String, (String, [f64; 2])>,
) -> BTreeMap<String, String> {
    let ic_of = |pin: &str| pin.split_once(':').map_or(pin, |(ic, _)| ic).to_string();
    let mut assigned = BTreeMap::new();
    let rails: BTreeSet<&String> = capacitors.iter().map(|(_, net)| net).collect();
    for rail in rails {
        let pins = &supplies[rail];
        // The rail's ICs with their supply pins, in board order.
        let mut ics: Vec<(String, Vec<String>)> = Vec::new();
        for (pin, _) in pins {
            let ic = ic_of(pin);
            match ics.iter_mut().find(|(known, _)| *known == ic) {
                Some((_, list)) => list.push(pin.clone()),
                None => ics.push((ic, vec![pin.clone()])),
            }
        }
        let distance = |capacitor: &str, ic: &str| {
            let (sheet, at) = positions.get(capacitor)?;
            let (other, there) = positions.get(ic)?;
            (sheet == other).then(|| crate::geometry::distance_squared(*at, *there).sqrt())
        };
        // Pass 1: the nearest IC.
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (capacitor, _) in capacitors.iter().filter(|(_, net)| net == rail) {
            if let Some((ic, _)) = ics
                .iter()
                .filter_map(|(ic, _)| distance(capacitor, ic).map(|d| (ic, d)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
            {
                groups
                    .entry(ic.clone())
                    .or_default()
                    .push(capacitor.clone());
            }
        }
        // Pass 2: an IC drawn without a capacitor takes another's extra.
        loop {
            let pin_count = |ic: &str| {
                ics.iter()
                    .find(|(known, _)| known == ic)
                    .map_or(0, |(_, pins)| pins.len())
            };
            let extra = groups
                .iter()
                .filter(|(ic, group)| group.len() > pin_count(ic))
                .flat_map(|(ic, group)| {
                    group.iter().filter_map(move |capacitor| {
                        distance(capacitor, ic).map(|d| (ic, capacitor, d))
                    })
                })
                .max_by(|a, b| a.2.total_cmp(&b.2))
                .map(|(ic, capacitor, _)| (ic.clone(), capacitor.clone()));
            let Some((from, capacitor)) = extra else {
                break;
            };
            let Some(to) = ics
                .iter()
                .filter(|(ic, _)| groups.get(ic).is_none_or(Vec::is_empty))
                .filter_map(|(ic, _)| distance(&capacitor, ic).map(|d| (ic, d)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(ic, _)| ic.clone())
            else {
                break;
            };
            groups
                .get_mut(&from)
                .expect("the extra's IC")
                .retain(|known| *known != capacitor);
            groups.entry(to).or_default().push(capacitor);
        }
        // At each IC, its pins in turn, the capacitors in board order.
        for (ic, ic_pins) in &ics {
            let Some(group) = groups.get(ic) else {
                continue;
            };
            let mut ordered: Vec<&String> = group.iter().collect();
            ordered.sort_by_key(|capacitor| {
                capacitors.iter().position(|(known, _)| known == *capacitor)
            });
            for (turn, capacitor) in ordered.into_iter().enumerate() {
                assigned.insert(capacitor.clone(), ic_pins[turn % ic_pins.len()].clone());
            }
        }
    }
    assigned
}

/// Whether the board's parts have a placement: most footprints at
/// positions of their own (a board fresh from a netlist, or stacked for
/// a layout, has them at a few points).
fn placed(parts: &[Part]) -> bool {
    let positions: BTreeSet<(i64, i64)> = parts
        .iter()
        .map(|part| {
            (
                (part.at[0] * 10.0).round() as i64,
                (part.at[1] * 10.0).round() as i64,
            )
        })
        .collect();
    parts.len() >= 2 && positions.len() * 2 > parts.len()
}

/// The sides of the outline box `bounds` a body lies within `reach` of,
/// the nearest first.
fn near_edges(bounds: [f64; 4], body: [f64; 4], reach: f64) -> Vec<&'static str> {
    let mut edges: Vec<(f64, &'static str)> = [
        (body[0] - bounds[0], "left"),
        (body[1] - bounds[1], "top"),
        (bounds[2] - body[2], "right"),
        (bounds[3] - body[3], "bottom"),
    ]
    .into_iter()
    .filter(|(gap, _)| *gap <= reach)
    .collect();
    edges.sort_by(|a, b| a.0.total_cmp(&b.0));
    edges.into_iter().map(|(_, edge)| edge).collect()
}

/// The edge a direction on the board points to.
fn edge_of(direction: [f64; 2]) -> &'static str {
    if direction[0].abs() >= direction[1].abs() {
        if direction[0] < 0.0 { "left" } else { "right" }
    } else if direction[1] < 0.0 {
        "top"
    } else {
        "bottom"
    }
}

/// The starter constraints for `directory/<board_id>.kicad_pcb`
/// (`describe-kicad-board --constraints-template`): docs/agents.md, "Starter
/// constraints for a board".
pub fn constraints_template(directory: &Path, board_id: &str) -> Result<serde_json::Value, String> {
    let path = directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let pcb = parse(&text)?;
    note_net_names(&pcb);
    let parts = parts_of(&pcb)?;
    let footprints: Vec<&Expr> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint") && footprint_reference(item).is_some())
        .collect();
    let has_placement = placed(&parts);
    let bounds = outline::board_loops(&pcb).ok().map(|loops| {
        loops.outline.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            },
        )
    });
    let mut comment: Vec<String> = vec![
        "A starter from describe-kicad-board --constraints-template: check each entry, delete what does not hold, add what the board needs (docs/constraints.md), then delete these comments.".into(),
        if has_placement {
            "The board has a placement: each connector's edge is the side of the outline it lies at (or faces), each capacitor's pin the supply pin it sits closest to.".into()
        } else {
            "The board has no placement yet (the parts share a few positions): the connectors' edges are \"any\" (the side nearest to where the placer puts them) and each capacitor goes to a supply pin of the IC the schematic draws it nearest to (an IC of its rail drawn with none gets one of another's extras), else to the rail's supply pins in turn.".into()
        },
        "move_all: every part is placed anew, wherever it starts (remove it to keep the parts that look deliberately placed).".into(),
    ];
    // Plug-in connectors: edge candidates.
    let mut edges = Vec::new();
    let mut other_connectors = Vec::new();
    let mut unclear = Vec::new();
    for (part, footprint) in parts.iter().zip(&footprints) {
        let wired = part.pads.iter().any(|pad| pad.1.is_some());
        if !wired {
            continue;
        }
        if !crate::netlist_import::edge_connector(&part.name) {
            if matches!(prefix(&part.reference), "J" | "P" | "CN" | "CON" | "USB") {
                other_connectors.push(part.reference.clone());
            }
            continue;
        }
        let mouth = crate::board_placer::connector_mouth(footprint)?;
        let facing = mouth.map(|mouth| edge_of(crate::geometry::rotate_vector(mouth, -part.at[2])));
        let near = match (has_placement, bounds, part.body) {
            (true, Some(bounds), Some(body)) => near_edges(bounds, body, 3.0),
            _ => Vec::new(),
        };
        // At a corner, the side its courtyard reaches farthest beyond its
        // pads towards (an overhanging receptacle's shell), else the
        // nearest.
        let sides = crate::board_placer::connector_sides(footprint)?;
        let reach = |edge: &str| {
            sides.map_or(0.0, |sides| {
                sides
                    .iter()
                    .filter(|(_, direction)| {
                        edge_of(crate::geometry::rotate_vector(*direction, -part.at[2])) == edge
                    })
                    .map(|(reach, _)| *reach)
                    .fold(f64::NEG_INFINITY, f64::max)
            })
        };
        let at_edge = facing
            .filter(|edge| near.contains(edge))
            .or_else(|| {
                near.iter()
                    .copied()
                    .max_by(|a, b| reach(a).total_cmp(&reach(b)))
                    .filter(|_| near.len() > 1 && sides.is_some())
            })
            .or_else(|| near.first().copied());
        let edge = if has_placement {
            at_edge.or(facing).unwrap_or("any")
        } else {
            "any"
        };
        let mut entry = serde_json::json!({"part": part.reference, "edge": edge, "flush": true});
        if mouth.is_some() {
            entry["opening_outwards"] = true.into();
        } else {
            unclear.push(format!(
                "{} (at {} degrees here)",
                part.reference, part.at[2]
            ));
        }
        edges.push(entry);
    }
    if !edges.is_empty() {
        comment.push("edge: the plug-in connectors (USB, jacks, coax, terminal blocks, by footprint name). opening_outwards turns the mouth out of the board; flush puts the body against the edge.".into());
    }
    if !unclear.is_empty() {
        comment.push(format!(
            "Which way these open is not clear from their footprints (no side of the courtyard reaches clearly beyond the pads): give each a rotation entry instead of trusting the edge alone: {}.",
            unclear.join(", ")
        ));
    }
    if !other_connectors.is_empty() {
        comment.push(format!(
            "Other connectors (headers, sockets: no edge entry): {}; add an edge entry for those that belong at an edge.",
            other_connectors.join(", ")
        ));
    }
    // Decoupling capacitors: the supply pins of the ICs by rail.
    let mut supplies: BTreeMap<String, Vec<(String, [f64; 2])>> = BTreeMap::new();
    for part in parts
        .iter()
        .filter(|part| matches!(prefix(&part.reference), "U" | "IC") && part.pads.len() >= 3)
    {
        let mut seen = BTreeSet::new();
        for (pad, net, center, supply) in &part.pads {
            let Some(net) = net else {
                continue;
            };
            if ground(net) || !*supply || !seen.insert(pad.clone()) {
                continue;
            }
            supplies
                .entry(net.clone())
                .or_default()
                .push((format!("{}:{pad}", part.reference), *center));
        }
    }
    // The decoupling capacitors: (part, its supply pad's net, centre).
    let mut capacitors: Vec<(&Part, String, [f64; 2])> = Vec::new();
    for part in parts
        .iter()
        .filter(|part| prefix(&part.reference) == "C" && part.pads.len() == 2)
    {
        let (Some(a), Some(b)) = (&part.pads[0].1, &part.pads[1].1) else {
            continue;
        };
        let supply = match (ground(a), ground(b)) {
            (false, true) => &part.pads[0],
            (true, false) => &part.pads[1],
            _ => continue,
        };
        let net = supply.1.clone().expect("a supply pad has a net");
        if supplies.contains_key(&net) {
            capacitors.push((part, net, supply.2));
        }
    }
    // Without a placement, the schematic says which IC a capacitor is
    // drawn at.
    let assigned = if has_placement {
        BTreeMap::new()
    } else {
        schematic_assignment(
            &capacitors
                .iter()
                .map(|(part, net, _)| (part.reference.clone(), net.clone()))
                .collect::<Vec<_>>(),
            &supplies,
            &schematic_positions(directory),
        )
    };
    let mut near = Vec::new();
    let mut taken: BTreeMap<String, usize> = BTreeMap::new();
    for (part, net, center) in &capacitors {
        let pins = &supplies[net];
        let pin = if has_placement {
            pins.iter()
                .min_by(|x, y| {
                    crate::geometry::distance_squared(x.1, *center)
                        .total_cmp(&crate::geometry::distance_squared(y.1, *center))
                })
                .map(|(pin, _)| pin.clone())
        } else if let Some(pin) = assigned.get(&part.reference) {
            Some(pin.clone())
        } else {
            // In turn over the rail's pins: one capacitor a pin first.
            let turn = taken.entry(net.clone()).or_default();
            let pin = pins[*turn % pins.len()].0.clone();
            *turn += 1;
            Some(pin)
        };
        let Some(pin) = pin else {
            continue;
        };
        let bulk = capacitance(&part.value).is_some_and(|farads| farads >= 4.7e-6);
        near.push(serde_json::json!({
            "part": part.reference,
            "pin_of": pin,
            "max_mm": if bulk { 3.0 } else { 1.5 },
        }));
    }
    if !near.is_empty() {
        comment.push("near: each decoupling capacitor (two pins, a supply rail and ground, the rail on an IC's supply pin) at that pin, 1.5 mm body to pin (3 mm for 4.7 uF and more). Without these entries the placer pulls each to within 2.5 mm of its IC's first pin on the rail (1 mm of the nearest supply pin with the placer's decoupling_near), softer; written down, place-kicad-board checks them and the verdict reports a miss. Several capacitors at one pin, or more than fit around a fine-pitch IC: give them other pins of the rail or loosen max_mm.".into());
    }
    comment.push("To fill in: parts that must not move (fixed), keepouts (antennas, mounting hardware), regions, the outline if the board has none (docs/constraints.md).".into());
    let mut template = serde_json::json!({
        "version": 1,
        "comment": comment,
        "move_all": true,
    });
    if !edges.is_empty() {
        template["edge"] = edges.into();
    }
    if !near.is_empty() {
        template["near"] = near.into();
    }
    Ok(template)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(dir: &Path, placed: bool) {
        // A USB receptacle whose courtyard reaches beyond its pads on its
        // -y side (the mouth), U1 with two +3V3 pins, C1 and C2 on +3V3.
        let (j, u, c1, c2) = if placed {
            ("(at 2 10 90)", "(at 20 10)", "(at 23 10)", "(at 17 10)")
        } else {
            ("(at 20 10)", "(at 20 10)", "(at 20 10)", "(at 20 10)")
        };
        let text = format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
            (footprint "Connector_USB:USB_C_Receptacle" {j} (property "Reference" "J1")
                (fp_rect (start -4 -5) (end 4 2) (layer "F.CrtYd"))
                (pad "A1" smd rect (at -2 1) (size 0.5 1) (layers "F.Cu") (net 2 "GND"))
                (pad "A4" smd rect (at 2 1) (size 0.5 1) (layers "F.Cu") (net 1 "+3V3")))
            (footprint "Header" (at 30 15) (property "Reference" "J2")
                (pad "1" thru_hole circle (at 0 0) (size 1.7 1.7) (layers "*.Cu") (net 1 "+3V3")))
            (footprint "QFN" {u} (property "Reference" "U1")
                (fp_rect (start -2 -2) (end 2 2) (layer "F.CrtYd"))
                (pad "1" smd rect (at 1.5 0) (size 0.3 0.8) (layers "F.Cu") (net 1 "+3V3") (pintype "power_in"))
                (pad "2" smd rect (at -1.5 0) (size 0.3 0.8) (layers "F.Cu") (net 1 "+3V3") (pintype "power_in"))
                (pad "3" smd rect (at 0 1.5) (size 0.8 0.3) (layers "F.Cu") (net 2 "GND")))
            (footprint "C_0402" {c1} (property "Reference" "C1") (property "Value" "100nF")
                (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "+3V3"))
                (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 2 "GND")))
            (footprint "C_0805" {c2} (property "Reference" "C2") (property "Value" "10uF")
                (pad "1" smd rect (at 0.9 0) (size 0.9 1.2) (layers "F.Cu") (net 1 "+3V3"))
                (pad "2" smd rect (at -0.9 0) (size 0.9 1.2) (layers "F.Cu") (net 2 "GND"))))"#
        );
        fs::write(dir.join("t.kicad_pcb"), text).unwrap();
    }

    #[test]
    fn a_decoupling_capacitor_goes_to_a_supply_pin_of_the_ic_the_schematic_draws_it_at() {
        // The schematic trial, stacked: VDD feeds an LDO (U3: VIN, and EN
        // tied to VDD) and two motor drivers (U5, U6: VM); C4 is drawn at
        // U3, C7 and C8 (22 uF) both nearest U6.
        let dir = std::env::temp_dir().join(format!("ux3-template-sch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let pad = |name: &str, net: &str, function: &str, kind: &str| {
            format!(
                r#"(pad "{name}" smd rect (at 0 0) (size 0.3 0.3) (layers "F.Cu") (net 1 "{net}") (pinfunction "{function}_{name}") (pintype "{kind}"))"#
            )
        };
        let ic = |reference: &str, pads: &[String]| {
            format!(
                r#"(footprint "IC" (at 0 0) (property "Reference" "{reference}") {})"#,
                pads.join(" ")
            )
        };
        let capacitor = |reference: &str, value: &str| {
            format!(
                r#"(footprint "C_0805" (at 0 0) (property "Reference" "{reference}") (property "Value" "{value}")
                    (pad "1" smd rect (at -0.9 0) (size 0.9 1.2) (layers "F.Cu") (net 1 "VDD"))
                    (pad "2" smd rect (at 0.9 0) (size 0.9 1.2) (layers "F.Cu") (net 2 "GND")))"#
            )
        };
        let driver = |reference: &str| {
            ic(
                reference,
                &[
                    pad("1", "VDD", "VM", "power_in"),
                    pad("4", "GND", "GND", "power_in"),
                    pad("7", "VDD", "~{SLEEP}", "input"),
                ],
            )
        };
        let board = [
            capacitor("C4", "1uF"),
            capacitor("C7", "22uF"),
            capacitor("C8", "22uF"),
            ic(
                "U3",
                &[
                    pad("1", "VDD", "VIN", "power_in"),
                    pad("2", "GND", "GND", "power_in"),
                    pad("3", "VDD", "EN", "input"),
                ],
            ),
            driver("U5"),
            driver("U6"),
        ];
        fs::write(
            dir.join("t.kicad_pcb"),
            format!(
                r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) {})"#,
                board.join(" ")
            ),
        )
        .unwrap();
        let symbol = |reference: &str, x: f64, y: f64| {
            format!(
                r#"(symbol (lib_id "x") (at {x} {y} 0) (instances (project "t" (path "/root" (reference "{reference}") (unit 1)))))"#
            )
        };
        let schematic = [
            symbol("U3", 320.0, 426.7),
            symbol("C4", 306.1, 433.1),
            symbol("U5", 73.7, 250.2),
            symbol("U6", 74.9, 292.1),
            symbol("C7", 104.1, 332.7),
            symbol("C8", 115.6, 332.7),
        ];
        fs::write(
            dir.join("t.kicad_sch"),
            format!("(kicad_sch (version 20231120) {})", schematic.join(" ")),
        )
        .unwrap();
        let template = constraints_template(&dir, "t").unwrap();
        let pins: Vec<(&str, &str)> = template["near"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry["part"].as_str().unwrap(),
                    entry["pin_of"].as_str().unwrap(),
                )
            })
            .collect();
        // Never U3:3 (EN) or a driver's SLEEP input; C7 at U6, C8 at U5.
        assert_eq!(pins, vec![("C4", "U3:1"), ("C7", "U6:1"), ("C8", "U5:1")]);
        // Without the schematic: the supply pins in turn, still no EN.
        fs::remove_file(dir.join("t.kicad_sch")).unwrap();
        let template = constraints_template(&dir, "t").unwrap();
        let pins: Vec<&str> = template["near"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["pin_of"].as_str().unwrap())
            .collect();
        assert_eq!(pins, vec!["U3:1", "U5:1", "U6:1"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn supply_pins_are_told_by_function_type_and_net() {
        assert!(supply_pin(Some("VIN_1"), Some("power_in"), "1", "VDD"));
        assert!(!supply_pin(Some("EN_3"), Some("input"), "3", "VDD"));
        assert!(!supply_pin(Some("~{SHDN}"), None, "4", "+3V3"));
        assert!(!supply_pin(Some("nCE"), Some("passive"), "4", "+3V3"));
        assert!(supply_pin(Some("3V3_3"), Some("passive"), "3", "+3.3V"));
        assert!(supply_pin(Some("VM"), None, "1", "MOTOR"));
        assert!(!supply_pin(
            Some("SDO/SA0_1"),
            Some("bidirectional"),
            "1",
            "+3.3V"
        ));
        assert!(!supply_pin(Some("VSS"), Some("passive"), "8", "SIG"));
        // Nothing known: the net's name.
        assert!(supply_pin(None, None, "1", "+3V3"));
        assert!(!supply_pin(None, None, "1", "SIG"));
    }

    #[test]
    fn the_template_names_the_connectors_and_the_decoupling_pairs() {
        let dir = std::env::temp_dir().join(format!("ux2-template-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        board(&dir, true);
        let template = constraints_template(&dir, "t").unwrap();
        // J1 at the left edge, opening outwards; J2 a header, only named.
        assert_eq!(
            template["edge"],
            serde_json::json!([{"part": "J1", "edge": "left", "flush": true, "opening_outwards": true}])
        );
        // Each capacitor at the pin nearest to it; the bulk one looser.
        assert_eq!(
            template["near"],
            serde_json::json!([
                {"part": "C1", "pin_of": "U1:1", "max_mm": 1.5},
                {"part": "C2", "pin_of": "U1:2", "max_mm": 3.0}
            ])
        );
        let comments = template["comment"].to_string();
        assert!(comments.contains("J2"), "{comments}");
        // It reads as constraints.
        let path = dir.join("constraints.json");
        fs::write(&path, template.to_string()).unwrap();
        crate::read_placement_constraints(&path).unwrap();
        // Stacked: any edge, the pins in turn.
        board(&dir, false);
        let template = constraints_template(&dir, "t").unwrap();
        assert_eq!(template["edge"][0]["edge"], "any");
        assert_eq!(template["near"][0]["pin_of"], "U1:1");
        assert_eq!(template["near"][1]["pin_of"], "U1:2");
        fs::remove_dir_all(&dir).unwrap();
    }
}

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
    /// Pad name, net (normalized), centre on the board, power pin.
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
            pads.push((
                pad.children()
                    .get(1)
                    .and_then(Expr::atom)
                    .unwrap_or("")
                    .to_string(),
                node_net(pad)
                    .filter(|net| !net.is_empty())
                    .map(|net| names.normalize(net).to_string()),
                [at[0] + offset[0], at[1] + offset[1]],
                form_atom(pad, "pintype", 1) == Some("power_in"),
            ));
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

/// The reference's letters (`U` of `U12`).
fn prefix(reference: &str) -> &str {
    reference.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_')
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
            "The board has no placement yet (the parts share a few positions): the connectors' edges are \"any\" (the side nearest to where the placer puts them) and each capacitor goes to a supply pin of its rail in turn.".into()
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
        for (pad, net, center, power) in &part.pads {
            let Some(net) = net else {
                continue;
            };
            if ground(net) || !(*power || rail_name(net)) || !seen.insert(pad.clone()) {
                continue;
            }
            supplies
                .entry(net.clone())
                .or_default()
                .push((format!("{}:{pad}", part.reference), *center));
        }
    }
    let mut near = Vec::new();
    let mut taken: BTreeMap<String, usize> = BTreeMap::new();
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
        let net = supply.1.as_ref().expect("a supply pad has a net");
        let Some(pins) = supplies.get(net) else {
            continue;
        };
        let pin = if has_placement {
            pins.iter()
                .min_by(|x, y| {
                    crate::geometry::distance_squared(x.1, supply.2)
                        .total_cmp(&crate::geometry::distance_squared(y.1, supply.2))
                })
                .map(|(pin, _)| pin.clone())
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

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

/// The distance from a box (`[x0, y0, x1, y1]`) to a point, 0 inside.
fn box_to_point(body: [f64; 4], point: [f64; 2]) -> f64 {
    let dx = (body[0] - point[0]).max(point[0] - body[2]).max(0.0);
    let dy = (body[1] - point[1]).max(point[1] - body[3]).max(0.0);
    dx.hypot(dy)
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

/// Whether a footprint is an RF module with an antenna of its own (KiCad's
/// `RF_Module:` library, or named for an ESP32 or nRF module).
fn rf_module(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("rf_module:")
        || [
            "esp32", "esp8266", "esp-0", "esp-12", "wroom", "wrover", "nrf52", "nrf53", "nrf91",
            "mdbt", "nina-b", "bl65",
        ]
        .iter()
        .any(|word| name.contains(word))
}

/// Where an RF module's antenna is: the direction on the board from its
/// pads' centre to its footprint's keepout zones (`true`), else to its
/// texts that say "Antenna" or "Keepout" on a drawing layer (`false`:
/// trial 6's laptop power, whose ESP32-S2-MINI-1 marks its antenna and
/// keepout area in text only).
fn antenna_direction(footprint: &Expr, part: &Part) -> Result<Option<([f64; 2], bool)>, String> {
    if !rf_module(&part.name) || part.pads.is_empty() {
        return Ok(None);
    }
    let count = part.pads.len() as f64;
    let pads = part.pads.iter().fold([0.0, 0.0], |sum, pad| {
        [sum[0] + pad.2[0] / count, sum[1] + pad.2[1] / count]
    });
    let mut zone: Option<[f64; 4]> = None;
    for points in footprint
        .children()
        .iter()
        .filter(|child| child.head() == Some("zone") && child.child("keepout").is_some())
        .filter_map(|zone| {
            zone.child("polygon")
                .and_then(|polygon| polygon.child("pts"))
        })
    {
        for point in points
            .children()
            .iter()
            .filter(|child| child.head() == Some("xy"))
        {
            // Zones in a footprint are stored in board coordinates.
            let [x, y] = form_xy_of(point)?;
            let bounds = zone.get_or_insert([x, y, x, y]);
            *bounds = [
                bounds[0].min(x),
                bounds[1].min(y),
                bounds[2].max(x),
                bounds[3].max(y),
            ];
        }
    }
    let (target, from_zone) = match zone {
        Some(bounds) => (
            [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0],
            true,
        ),
        None => {
            let marks: Vec<[f64; 2]> = footprint
                .children()
                .iter()
                .filter(|child| {
                    child.head() == Some("fp_text")
                        && child.children().get(1).and_then(Expr::atom) == Some("user")
                        && child
                            .children()
                            .get(2)
                            .and_then(Expr::atom)
                            .is_some_and(|text| {
                                let text = text.to_ascii_lowercase();
                                text.contains("antenna")
                                    || text.contains("keepout")
                                    || text.contains("keep-out")
                                    || text.contains("keep out")
                            })
                        && form_atom(child, "layer", 1).is_some_and(|layer| !layer.ends_with(".Cu"))
                })
                .filter_map(|text| form_xy(text, "at").ok())
                .map(|local| {
                    let offset = crate::geometry::rotate_vector(local, -part.at[2]);
                    [part.at[0] + offset[0], part.at[1] + offset[1]]
                })
                .collect();
            if marks.is_empty() {
                return Ok(None);
            }
            let count = marks.len() as f64;
            (
                marks.iter().fold([0.0, 0.0], |sum, mark| {
                    [sum[0] + mark[0] / count, sum[1] + mark[1] / count]
                }),
                false,
            )
        }
    };
    let direction = [target[0] - pads[0], target[1] - pads[1]];
    Ok((direction[0].hypot(direction[1]) >= 0.5).then_some((direction, from_zone)))
}

/// The point of an `(xy x y)` form.
fn form_xy_of(point: &Expr) -> Result<[f64; 2], String> {
    let value = |index: usize| {
        point
            .children()
            .get(index)
            .and_then(Expr::atom)
            .and_then(|atom| atom.parse::<f64>().ok())
            .ok_or_else(|| "a keepout point without its coordinates".to_string())
    };
    Ok([value(1)?, value(2)?])
}

/// The rotations that turn an end of a part (its direction on the board,
/// the part at `angle`) to each edge: `top 0, right 270, ...`.
fn turns_to_edges(direction: [f64; 2], angle: f64) -> String {
    let local = crate::geometry::rotate_vector(direction, angle);
    ["top", "right", "bottom", "left"]
        .iter()
        .filter_map(|edge| {
            [0.0, 90.0, 180.0, 270.0]
                .into_iter()
                .find(|turn| edge_of(crate::geometry::rotate_vector(local, -turn)) == *edge)
                .map(|turn| format!("{edge} {turn}"))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The template's comment on the project's `length` and `skew` rules (as
/// the router reads them, `project_rules::length_rules`): per rule its
/// bounds, its nets and the parts their pins lie on, and what the layout's
/// placer holds of it (fresh-agent trial 6: neither the description nor
/// the template named the firestarter's DVP rule).
fn length_rule_comments(
    directory: &Path,
    board_id: &str,
    pcb: &Path,
    parts: &[Part],
) -> Vec<String> {
    let Ok(rules) = crate::project_rules::resolve_project_rules(
        &directory.join(format!("{board_id}.kicad_pro")),
        pcb,
    ) else {
        return Vec::new();
    };
    rules
        .length_rules
        .iter()
        .map(|rule| {
            let nets: BTreeSet<&str> = rule.nets.iter().map(|member| member.net.as_str()).collect();
            let on: BTreeSet<&str> = parts
                .iter()
                .filter(|part| {
                    part.pads
                        .iter()
                        .any(|(_, net, ..)| net.as_deref().is_some_and(|net| nets.contains(net)))
                })
                .map(|part| part.reference.as_str())
                .collect();
            let mut named: Vec<&str> = nets.iter().copied().take(6).collect();
            let more = nets.len().saturating_sub(6);
            let more = if more > 0 { format!(", {more} more") } else { String::new() };
            let bound = |value: Option<f64>| value.map_or("-".to_string(), |v| format!("{v:.2}"));
            let held = if rule.constraint == "length" {
                "the layout's placer holds two parts' pins of such a net within 0.8 of the maximum (length_maxima) and, with {\"placer\": {\"length_minima\": true}} in layout.json, at least half the minimum apart; board-placer.json's length_rules says where they ended; an apart or near entry between the parts holds them more"
            } else {
                "a skew: the router matches the group's nets by meanders; the placer holds nothing of it"
            };
            named.sort();
            format!(
                "{} rule '{}' (min {}, max {} mm) on {} nets ({}{more}) between {}: {held}.",
                rule.constraint,
                rule.rule,
                bound(rule.min),
                bound(rule.max),
                nets.len(),
                named.join(", "),
                on.iter().copied().collect::<Vec<_>>().join(", ")
            )
        })
        .collect()
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
    // RF modules: the antenna at an edge, the side the footprint's keepout
    // (else its "Antenna" text) tells.
    for (part, footprint) in parts.iter().zip(&footprints) {
        let Some((direction, from_zone)) = antenna_direction(footprint, part)? else {
            continue;
        };
        let facing = edge_of(direction);
        let turns = turns_to_edges(direction, part.at[2]);
        if from_zone {
            let edge = if has_placement { facing } else { "any" };
            let overhang = if has_placement {
                serde_json::json!({"edge": facing})
            } else {
                serde_json::json!({})
            };
            edges.push(serde_json::json!({
                "part": part.reference, "edge": edge, "flush": true, "overhang": overhang,
            }));
            comment.push(format!(
                "edge: {} ({}) is an RF module whose footprint draws its antenna keepout (at {} degrees here it faces the {facing} edge): overhang puts the keepout beyond the edge and the module flush inside it, as the module makers' layout guides ask. To keep the keepout on the board instead, drop overhang and give a rotation that turns the antenna end to the edge: {turns}.",
                part.reference, part.name, part.at[2]
            ));
        } else {
            comment.push(format!(
                "{} ({}) is an RF module whose footprint marks its antenna in text only (no keepout zone, so no overhang): the antenna end faces the {facing} edge at {} degrees here. To put it at an edge, add an edge entry (\"flush\": true) and the rotation that turns that end to it: {turns}.",
                part.reference, part.name, part.at[2]
            ));
        }
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
    let mut widened = Vec::new();
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
        let default = if bulk { 3.0 } else { 1.5 };
        // On a placed board, at least the designer's own distance (the
        // capacitor's courtyard box to the pin; the placer measures from
        // its body, which holds that box), rounded up to 0.5 mm:
        // framework_mobo_lefthalf's designer has C1 7.96 mm from U1:22,
        // and 1.5 mm missed on the designer's own placement.
        let designer = if has_placement {
            part.body
                .zip(pins.iter().find(|(name, _)| *name == pin))
                .map(|(body, (_, at))| box_to_point(body, *at))
        } else {
            None
        };
        let max_mm = match designer {
            // (within the placer's tolerance of the default: kept)
            Some(distance) if distance > default + pcb_placer::constraints::RELATION_TOLERANCE => {
                let max_mm = (distance / 0.5).ceil() * 0.5;
                widened.push(format!(
                    "{} at {pin} {distance:.2} mm (max_mm {max_mm})",
                    part.reference
                ));
                max_mm
            }
            _ => default,
        };
        near.push(serde_json::json!({
            "part": part.reference,
            "pin_of": pin,
            "max_mm": max_mm,
        }));
    }
    if !near.is_empty() {
        comment.push("near: each decoupling capacitor (two pins, a supply rail and ground, the rail on an IC's supply pin) at that pin, 1.5 mm body to pin (3 mm for 4.7 uF and more). Without these entries the placer pulls each to within 2.5 mm of its IC's first pin on the rail (1 mm of the nearest supply pin with the placer's decoupling_near), softer; written down, place-kicad-board checks them and the verdict reports a miss. Several capacitors at one pin, or more than fit around a fine-pitch IC: give them other pins of the rail or loosen max_mm.".into());
    }
    if !widened.is_empty() {
        comment.push(format!(
            "near: these lie farther from their pin on this board than 1.5 mm (3 mm), so their max_mm is that distance rounded up to 0.5 mm, which the board's own placement keeps; tighten them where the design wants the capacitor closer: {}.",
            widened.join(", ")
        ));
    }
    let held: BTreeSet<String> = near
        .iter()
        .filter_map(|entry| entry["part"].as_str().map(str::to_string))
        .collect();
    let (rows, row_comments) = banks(&parts, &footprints, &held)?;
    comment.extend(row_comments);
    comment.extend(length_rule_comments(directory, board_id, &path, &parts));
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
    if !rows.is_empty() {
        template["row"] = rows.into();
    }
    Ok(template)
}

/// The fewest parts that make a bank (`banks`).
const BANK_PARTS: usize = 6;

/// The gap the template keeps between a bank's pads (mm): a common copper
/// clearance; the row's own check holds the project's.
const BANK_GAP: f64 = 0.2;

/// Banks of parts: `BANK_PARTS` or more two-pad parts of one footprint on
/// the same two nets that no `near` entry holds (`held`): a bulk capacitor
/// bank. The netlist's start puts parts on the same nets at one spot and
/// the placement spreads them where they fit (trial 8: the 4in1-mini's 22
/// 1206 capacitors on +BATT and GND came back around the board's edge; a
/// row as the designer's block left 69 unconnected against 89, one layout
/// each). Each becomes a `row` with columns at its pads' pitch (their span
/// and `BANK_GAP`), the row across the parts' short side, about as wide as
/// it is tall; with a comment.
fn banks(
    parts: &[Part],
    footprints: &[&Expr],
    held: &BTreeSet<String>,
) -> Result<(Vec<serde_json::Value>, Vec<String>), String> {
    let mut groups: BTreeMap<(String, String, String), Vec<usize>> = BTreeMap::new();
    for (index, part) in parts.iter().enumerate() {
        if held.contains(&part.reference) || part.pads.len() != 2 {
            continue;
        }
        let (Some(a), Some(b)) = (&part.pads[0].1, &part.pads[1].1) else {
            continue;
        };
        if a == b {
            continue;
        }
        let (first, second) = if a < b { (a, b) } else { (b, a) };
        groups
            .entry((part.name.clone(), first.clone(), second.clone()))
            .or_default()
            .push(index);
    }
    let mut rows = Vec::new();
    let mut comments = Vec::new();
    for ((name, first, second), members) in groups {
        let mut references: Vec<String> = members
            .iter()
            .map(|index| parts[*index].reference.clone())
            .collect();
        references.sort_by(|a, b| crate::canonical_order::natural_cmp(a, b));
        references.dedup();
        if members.len() < BANK_PARTS || references.len() < 2 {
            continue;
        }
        // The pads' span on the board, as the first part (the row's
        // leader) is turned.
        let leader = members
            .iter()
            .copied()
            .find(|index| parts[*index].reference == references[0])
            .unwrap_or(members[0]);
        let pads = crate::placement_constraints::pad_boxes(footprints[leader])?;
        let Some(span) = pads.iter().map(|(bounds, ..)| *bounds).reduce(|a, b| {
            [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
        }) else {
            continue;
        };
        let size = [span[2] - span[0], span[3] - span[1]];
        let up = |value: f64| ((value + BANK_GAP) / 0.05 - 1.0e-9).ceil() * 0.05;
        let (axis, pitch, row_pitch) = if size[0] <= size[1] {
            ("x", up(size[0]), up(size[1]))
        } else {
            ("y", up(size[1]), up(size[0]))
        };
        let count = members.len();
        let columns = ((count as f64 * row_pitch / pitch).sqrt().round() as usize).clamp(1, count);
        let round = |value: f64| (value * 100.0).round() / 100.0;
        rows.push(serde_json::json!({
            "parts": references,
            "pitch_mm": round(pitch),
            "row_pitch_mm": round(row_pitch),
            "columns": columns,
            "axis": axis,
        }));
        comments.push(format!(
            "row: {count} parts of {} on {first} and {second} ({}, ...) look like a bank (bulk capacitors). The placement starts every part where its nets pull it, which for parts on the same two nets is one spot, and spreads them where they fit; this row keeps them one block of {columns} columns, {} mm apart along the board's {axis} and {} mm between the lines (their pads' span and {BANK_GAP} mm, as turned now), as designers lay a bank (trial 8's 4in1-mini: 69 unconnected against 89, one layout each). Give the row's first part a place (place with x and y) where the bank belongs, or delete the entry to let the placer spread them.",
            name.rsplit(':').next().unwrap_or(&name),
            references.iter().take(3).cloned().collect::<Vec<_>>().join(", "),
            round(pitch),
            round(row_pitch),
        ));
    }
    Ok((rows, comments))
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

    /// Six 1206 capacitors on +BATT and GND that no near entry holds, a
    /// bank: one row with columns at their pads' pitch, across their short
    /// side (trial 8's 4in1-mini). Five are no bank.
    #[test]
    fn a_bank_of_parts_on_two_nets_becomes_a_row() {
        let template = |count: usize| {
            let dir = std::env::temp_dir().join(format!("ux8-template-bank-{count}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            // Turned 90 degrees: the pads (1.15 x 1.8, 2.95 mm apart) lie
            // along the board's y, 1.8 mm across x.
            let capacitor = |n: usize| {
                format!(
                    r#"(footprint "Capacitor_SMD:C_1206_3216Metric" (at 50 50 90) (property "Reference" "CL{n}") (property "Value" "4.7uF")
                        (pad "1" smd roundrect (at -1.475 0 90) (size 1.15 1.8) (layers "F.Cu") (net 1 "+BATT"))
                        (pad "2" smd roundrect (at 1.475 0 90) (size 1.15 1.8) (layers "F.Cu") (net 2 "GND")))"#
                )
            };
            let parts: Vec<String> = (30..30 + count).map(capacitor).collect();
            fs::write(
                dir.join("t.kicad_pcb"),
                format!(
                    r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
                    (gr_rect (start 0 0) (end 100 100) (layer "Edge.Cuts")) {})"#,
                    parts.join(" ")
                ),
            )
            .unwrap();
            let template = constraints_template(&dir, "t").unwrap();
            fs::remove_dir_all(&dir).unwrap();
            template
        };
        let six = template(6);
        let row = &six["row"][0];
        assert_eq!(row["parts"].as_array().unwrap().len(), 6);
        assert_eq!(row["parts"][0], "CL30");
        assert_eq!(row["axis"], "x");
        assert_eq!(row["pitch_mm"], 2.0);
        assert_eq!(row["row_pitch_mm"], 4.3);
        assert_eq!(row["columns"], 4);
        assert!(
            six["comment"]
                .as_array()
                .unwrap()
                .iter()
                .any(|line| line.as_str().unwrap().starts_with("row: 6 parts of C_1206_3216Metric on +BATT and GND")),
            "{six}"
        );
        assert!(template(5).get("row").is_none());
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
    fn a_placed_boards_near_entries_keep_at_least_the_designers_distance() {
        // framework_mobo_lefthalf: the designer has C1 7.96 mm from U1:22,
        // and the template's 1.5 mm missed on the designer's placement.
        let dir = std::env::temp_dir().join(format!("ux5-template-far-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        board(&dir, true);
        let path = dir.join("t.kicad_pcb");
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace(r#""C_0402" (at 23 10)"#, r#""C_0402" (at 26 10)"#);
        fs::write(&path, text).unwrap();
        let template = constraints_template(&dir, "t").unwrap();
        // C1's pads reach 25.25, U1:1 lies at 21.5: 3.75 mm, max_mm 4; C2
        // (10 uF) 0.85 mm from U1:2 keeps its 3 mm.
        assert_eq!(
            template["near"],
            serde_json::json!([
                {"part": "C1", "pin_of": "U1:1", "max_mm": 4.0},
                {"part": "C2", "pin_of": "U1:2", "max_mm": 3.0}
            ])
        );
        let comments = template["comment"].to_string();
        assert!(
            comments.contains("C1 at U1:1 3.75 mm (max_mm 4)"),
            "{comments}"
        );
        // Stacked (no placement): today's defaults.
        board(&dir, false);
        let template = constraints_template(&dir, "t").unwrap();
        assert!(
            template["near"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["max_mm"] == 1.5 || entry["max_mm"] == 3.0),
            "{}",
            template["near"]
        );
        assert!(!template["comment"].to_string().contains("rounded up"));
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
        assert!(!comments.contains("length rule"), "{comments}");
        // A length rule of the project: named with its nets and parts.
        fs::write(
            dir.join("t.kicad_dru"),
            "(version 1)\n(rule \"bus\" (constraint length (min 20mm) (max 22mm)) (condition \"A.NetName == '+3V3'\"))\n",
        )
        .unwrap();
        let ruled = constraints_template(&dir, "t").unwrap();
        let comments = ruled["comment"].to_string();
        assert!(
            comments.contains("length rule 'bus' (min 20.00, max 22.00 mm) on 1 nets (+3V3) between C1, C2, J1, J2, U1")
                && comments.contains("length_minima\\\": true}} in layout.json, at least half the minimum apart"),
            "{comments}"
        );
        fs::remove_file(dir.join("t.kicad_dru")).unwrap();
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

    /// An ESP32 module at (30, 20), turned 90 degrees, with its antenna
    /// keepout (`zone`) or only an "Antenna" text at the footprint's -y
    /// end, which faces the left edge at 90 degrees.
    fn rf_board(dir: &Path, zone: bool, placed: bool) {
        let (u, r) = if placed {
            ("(at 30 20 90)", "(at 50 30)")
        } else {
            ("(at 30 20 90)", "(at 30 20)")
        };
        let marks = if zone {
            // Board coordinates: x 18 to 23 (left of the module).
            r#"(zone (net 0) (net_name "") (layer "F.Cu") (hatch edge 0.5) (connect_pads (clearance 0)) (min_thickness 0.25)
                (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints not_allowed))
                (polygon (pts (xy 18 12) (xy 23 12) (xy 23 28) (xy 18 28))))"#
        } else {
            r#"(fp_text user "Antenna" (at 0 -7.5 0) (layer "Cmts.User") (effects (font (size 1 1) (thickness 0.15))))"#
        };
        let text = format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 60 40) (layer "Edge.Cuts"))
            (footprint "RF_Module:ESP32-S2-MINI-1" {u} (property "Reference" "U1")
                (fp_rect (start -7.7 -9.75) (end 7.7 10.25) (layer "F.CrtYd"))
                (pad "1" smd rect (at -7 0) (size 0.8 0.8) (layers "F.Cu") (net 1 "+3V3"))
                (pad "2" smd rect (at 7 0) (size 0.8 0.8) (layers "F.Cu") (net 2 "GND"))
                (pad "3" smd rect (at 0 8) (size 0.8 0.8) (layers "F.Cu") (net 3 "TX"))
                {marks})
            (footprint "R_0402" {r} (property "Reference" "R1")
                (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 3 "TX"))
                (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 2 "GND"))))"#
        );
        fs::write(dir.join("t.kicad_pcb"), text).unwrap();
    }

    #[test]
    fn an_rf_modules_antenna_keepout_puts_it_at_an_edge_with_overhang() {
        let dir = std::env::temp_dir().join(format!("ux12-template-rf-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // A keepout on a placed board: the edge it faces, overhanging it.
        rf_board(&dir, true, true);
        let template = constraints_template(&dir, "t").unwrap();
        assert_eq!(
            template["edge"],
            serde_json::json!([{"part": "U1", "edge": "left", "flush": true, "overhang": {"edge": "left"}}])
        );
        let comments = template["comment"].to_string();
        assert!(
            comments.contains("faces the left edge): overhang puts the keepout beyond the edge"),
            "{comments}"
        );
        assert!(
            comments.contains("top 0, right 270, bottom 180, left 90"),
            "{comments}"
        );
        let path = dir.join("constraints.json");
        fs::write(&path, template.to_string()).unwrap();
        crate::read_placement_constraints(&path).unwrap();
        // Stacked: any edge; the overhang takes the edge chosen.
        rf_board(&dir, true, false);
        let template = constraints_template(&dir, "t").unwrap();
        assert_eq!(
            template["edge"],
            serde_json::json!([{"part": "U1", "edge": "any", "flush": true, "overhang": {}}])
        );
        // An "Antenna" text without a keepout: a comment, no entry.
        rf_board(&dir, false, true);
        let template = constraints_template(&dir, "t").unwrap();
        assert!(template.get("edge").is_none(), "{template}");
        let comments = template["comment"].to_string();
        assert!(
            comments.contains("U1 (RF_Module:ESP32-S2-MINI-1) is an RF module whose footprint marks its antenna in text only (no keepout zone, so no overhang): the antenna end faces the left edge at 90 degrees here"),
            "{comments}"
        );
        assert!(
            comments.contains("top 0, right 270, bottom 180, left 90"),
            "{comments}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}

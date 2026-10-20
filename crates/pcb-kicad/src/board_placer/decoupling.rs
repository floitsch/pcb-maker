// Copyright (C) 2026 Toit contributors.

//! Decoupling capacitors, crystals, switch-node inductors and protectors
//! pulled to the pins they serve.

use super::*;
use pcb_placer as core;

/// Decoupling capacitors pulled to IC supply pins: every capacitor between
/// a rail and ground goes near a supply pin of an IC on that rail, whichever
/// suits; up to 4.7 µF within `decoupling_mm` (2.5 mm) of the pin, bulk capacitors within
/// 5 mm. Crystals go within 3 mm of the IC pins they drive. Capacitors that a user
/// constraint names, fixed ones and ones a row carries keep to those.
pub(super) fn decoupling_relations(
    pcb: &Expr,
    problem: &core::Problem,
    near_mm: f64,
) -> Result<Vec<core::constraints::Relation>, String> {
    use crate::quality::{capacitance, ground, rail_name};
    use core::constraints::{Anchor, Relation};
    struct Supply {
        part: usize,
        offset: [f64; 2],
        net: String,
    }
    let footprints: Vec<&Expr> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .collect();
    // A user's "near that part" leaves room for "at its supply pin";
    // anything else a user said about a capacitor stands.
    let named: BTreeSet<usize> = problem
        .constraints
        .relations
        .iter()
        .filter_map(|relation| match relation {
            Relation::Near {
                anchor: Anchor::Body(_),
                ..
            } => None,
            Relation::Near {
                anchor: Anchor::Area(_, _, _, half),
                ..
            } if *half != [0.0, 0.0] => None,
            other => other.parts()[0],
        })
        .chain(problem.constraints.edges.iter().map(|(part, _, _)| *part))
        .chain(problem.constraints.regions.iter().map(|(part, _)| *part))
        .chain(
            problem
                .constraints
                .followers
                .iter()
                .map(|follower| follower.part),
        )
        .collect();
    let prefix = |reference: &str| {
        reference
            .trim_end_matches(|c: char| c.is_ascii_digit() || c == '_')
            .to_string()
    };
    let mut supplies: Vec<Supply> = Vec::new();
    // Capacitors between ground and another net: decoupling when that net
    // is a rail (by name, or an IC's supply pin is on it).
    let mut capacitors: Vec<(usize, String, bool)> = Vec::new();
    // Crystals and the nets on their signal pins; IC pads by net.
    let mut crystals: Vec<(usize, Vec<String>)> = Vec::new();
    let mut ic_pads: BTreeMap<String, Vec<(usize, [f64; 2])>> = BTreeMap::new();
    // IC pads whose pin function names a switch node (`SW`, `LX`), by net.
    let mut switch_pads: BTreeMap<String, Vec<(usize, [f64; 2])>> = BTreeMap::new();
    // Inductors and their switch-node nets.
    let mut inductors: Vec<(usize, Vec<String>)> = Vec::new();
    // ESD protection and the nets it guards; connector pads by net.
    let mut protectors: Vec<(usize, Vec<String>)> = Vec::new();
    let mut connector_pads: BTreeMap<String, Vec<(usize, [f64; 2])>> = BTreeMap::new();
    for (index, footprint) in footprints.iter().enumerate() {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let pads: Vec<(Option<String>, [f64; 2], bool, bool)> = footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
            .map(|pad| {
                let at = form_at(pad).unwrap_or([0.0; 3]);
                // The pin function without KiCad's appended pin number.
                let function = form_atom(pad, "pinfunction", 1).unwrap_or("");
                let function = function
                    .rsplit_once('_')
                    .filter(|(_, number)| number.chars().all(|c| c.is_ascii_digit()))
                    .map_or(function, |(name, _)| name);
                let function = function.to_ascii_uppercase();
                let switch = matches!(
                    function.as_str(),
                    "SW" | "LX" | "SWITCH" | "PH" | "SWN" | "SWP" | "BST" | "VSW"
                ) || function.starts_with("SW")
                    && function[2..].chars().all(|c| c.is_ascii_digit())
                    || function.starts_with("LX")
                        && function[2..].chars().all(|c| c.is_ascii_digit());
                (
                    node_net(pad)
                        .filter(|raw| placer_net(raw))
                        .map(|raw| normalize_net(raw).to_string()),
                    [at[0], at[1]],
                    form_atom(pad, "pintype", 1) == Some("power_in"),
                    switch,
                )
            })
            .collect();
        let value = footprint
            .children()
            .iter()
            .find(|item| {
                item.head() == Some("property")
                    && item.children().get(1).and_then(Expr::atom) == Some("Value")
            })
            .and_then(|item| item.children().get(2).and_then(Expr::atom))
            .unwrap_or("")
            .to_ascii_uppercase();
        let protector = crate::quality::esd_protector(&value)
            && !problem.components[index].fixed
            && !named.contains(&index);
        if matches!(
            prefix(&reference).as_str(),
            "J" | "P" | "CN" | "CON" | "USB"
        ) {
            for (net, offset, _, _) in &pads {
                if let Some(net) = net.as_ref().filter(|net| !ground(net)) {
                    connector_pads
                        .entry(net.clone())
                        .or_default()
                        .push((index, *offset));
                }
            }
        }
        if protector {
            let nets: Vec<String> = pads
                .iter()
                .filter_map(|(net, _, _, _)| net.clone())
                .filter(|net| !ground(net))
                .collect();
            protectors.push((index, nets));
            continue;
        }
        match prefix(&reference).as_str() {
            "U" | "IC" if pads.len() >= 3 => {
                let mut seen = BTreeSet::new();
                for (net, offset, power, switch) in &pads {
                    let Some(net) = net else {
                        continue;
                    };
                    ic_pads
                        .entry(net.clone())
                        .or_default()
                        .push((index, *offset));
                    if *switch {
                        switch_pads
                            .entry(net.clone())
                            .or_default()
                            .push((index, *offset));
                    }
                    // One anchor per rail and IC: its first supply pin.
                    if !ground(net) && (*power || rail_name(net)) && seen.insert(net.clone()) {
                        supplies.push(Supply {
                            part: index,
                            offset: *offset,
                            net: net.clone(),
                        });
                    }
                }
            }
            _ if {
                let name = footprint
                    .children()
                    .get(1)
                    .and_then(Expr::atom)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                prefix(&reference) == "Y" || name.contains("crystal") || name.contains("resonator")
            } =>
            {
                if named.contains(&index) || problem.components[index].fixed {
                    continue;
                }
                let nets: Vec<String> = pads
                    .iter()
                    .filter_map(|(net, _, _, _)| net.clone())
                    .filter(|net| !ground(net) && !rail_name(net))
                    .collect();
                crystals.push((index, nets));
            }
            "L" if pads.len() == 2 => {
                if named.contains(&index) || problem.components[index].fixed {
                    continue;
                }
                // The switch node: the net that is neither ground nor a rail.
                let nets: Vec<String> = pads
                    .iter()
                    .filter_map(|(net, _, _, _)| net.clone())
                    .filter(|net| !ground(net) && !rail_name(net))
                    .collect();
                if nets.len() == 1 {
                    inductors.push((index, nets));
                }
            }
            "C" if pads.len() == 2 => {
                if named.contains(&index) || problem.components[index].fixed {
                    continue;
                }
                let (Some(a), Some(b)) = (&pads[0].0, &pads[1].0) else {
                    continue;
                };
                let rail = match (ground(a), ground(b)) {
                    (false, true) => a.clone(),
                    (true, false) => b.clone(),
                    _ => continue,
                };
                let value = footprint
                    .children()
                    .iter()
                    .find(|item| {
                        item.head() == Some("property")
                            && item.children().get(1).and_then(Expr::atom) == Some("Value")
                    })
                    .and_then(|item| item.children().get(2).and_then(Expr::atom))
                    .unwrap_or("");
                let bulk = capacitance(value).is_some_and(|farads| farads >= 4.7e-6);
                capacitors.push((index, rail, bulk));
            }
            _ => {}
        }
    }
    // Each capacitor near whichever supply pin of its rail the placer finds
    // best: the netlist does not say which IC a capacitor serves.
    let mut relations = Vec::new();
    for (index, rail, bulk) in capacitors {
        let anchors: Vec<Anchor> = supplies
            .iter()
            .filter(|supply| supply.net == rail)
            .map(|supply| Anchor::Point(supply.part, supply.offset))
            .collect();
        if anchors.is_empty() {
            continue;
        }
        relations.push(Relation::NearAny {
            part: index,
            anchors,
            max: if bulk { 5.0 } else { near_mm },
        });
    }
    // A switcher's inductor at the IC's switch pin: a small hot loop. Only
    // where a pin function says the pin switches (a supply filter's
    // inductor also sits between an IC pin and a rail, and pulling it in
    // costs routability: Brushless_ESC).
    for (index, nets) in inductors {
        let anchors: Vec<Anchor> = nets
            .iter()
            .flat_map(|net| switch_pads.get(net).into_iter().flatten())
            .map(|(ic, offset)| Anchor::Point(*ic, *offset))
            .collect();
        if !anchors.is_empty() {
            relations.push(Relation::NearAny {
                part: index,
                anchors,
                max: 3.0,
            });
        }
    }
    // ESD protection at the connector pins it guards: a surge should meet
    // it before anything else.
    for (index, nets) in protectors {
        let anchors: Vec<Anchor> = nets
            .iter()
            .flat_map(|net| connector_pads.get(net).into_iter().flatten())
            .map(|(connector, offset)| Anchor::Point(*connector, *offset))
            .collect();
        if !anchors.is_empty() {
            relations.push(Relation::NearAny {
                part: index,
                anchors,
                max: 3.0,
            });
        }
    }
    // A crystal close to the IC pins it drives (short, quiet oscillator
    // traces).
    for (index, nets) in crystals {
        for net in nets {
            for (ic, offset) in ic_pads.get(&net).into_iter().flatten() {
                relations.push(Relation::Near {
                    part: index,
                    anchor: Anchor::Point(*ic, *offset),
                    max: 3.0,
                });
            }
        }
    }
    Ok(relations)
}

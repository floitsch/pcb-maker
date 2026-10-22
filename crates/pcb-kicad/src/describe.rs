// Copyright (C) 2026 Toit contributors.

//! A board described for whoever writes constraints for it (often an
//! agent): parts with their sizes and pads, nets with their pads and rules,
//! the outline and the copper layers.

use super::*;
use crate::board_placer::local_body;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardDescription {
    pub board_id: String,
    /// Bounding box of the outline, or `None` for a board without one.
    pub outline: Option<KiCadDescribedOutline>,
    pub copper_layers: Vec<String>,
    pub footprints: Vec<KiCadDescribedFootprint>,
    pub nets: Vec<KiCadDescribedNet>,
    /// Parts with several connected general-purpose pins: by the pad's
    /// function (`GPIO12`, `IO5`, `PA3`, `P0.13`), or, for a part whose
    /// pads name no function on the board, by the pin's electrical type
    /// (bidirectional or passive pins of one symbol unit: the board's
    /// `pintype`, else the schematic's symbol). If
    /// the firmware can use any of them for any of these signals,
    /// `swappable` lets the layout choose.
    pub swap_candidates: Vec<KiCadSwapCandidate>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadSwapCandidate {
    pub part: String,
    pub value: String,
    /// Globs for the layout config's `swappable` `pins`: function globs
    /// (`GPIO*`) where the pads name general-purpose functions, else the
    /// pad numbers.
    pub pins: Vec<String>,
    /// The connected pads they match, as `PAD (function)` (or `PAD (pin
    /// name, type)` for a candidate by electrical type).
    pub connected: Vec<String>,
    /// How the pins were found: `function` (the pads' general-purpose
    /// function names) or `electrical type` (bidirectional or passive pins
    /// of one symbol unit; its number in `unit`, 0 for the pins common to
    /// every unit).
    pub by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<u32>,
}

/// A symbol pin as the schematic has it: unit (0: common to every unit),
/// electrical type and name.
#[derive(Clone, Debug, PartialEq)]
struct SymbolPin {
    unit: u32,
    kind: String,
    name: String,
}

/// The pins of the schematic's placed symbols by (reference, pin number):
/// `<board_id>.kicad_sch` in `directory` and its sheets (`Sheetfile`),
/// each sheet's symbols from its own `lib_symbols`. Empty without a
/// schematic (a board on its own); an unreadable sheet is skipped.
fn schematic_symbol_pins(
    directory: &Path,
    board_id: &str,
) -> BTreeMap<(String, String), SymbolPin> {
    let mut result = BTreeMap::new();
    let mut pending = vec![directory.join(format!("{board_id}.kicad_sch"))];
    let mut seen = BTreeSet::new();
    while let Some(path) = pending.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        let Some(sheet) = fs::read_to_string(&path)
            .ok()
            .and_then(|text| parse(&text).ok())
        else {
            continue;
        };
        let mut library: BTreeMap<String, Vec<(String, SymbolPin)>> = BTreeMap::new();
        if let Some(symbols) = sheet.child("lib_symbols") {
            for symbol in symbols
                .children()
                .iter()
                .filter(|item| item.head() == Some("symbol"))
            {
                let Some(id) = symbol.children().get(1).and_then(Expr::atom) else {
                    continue;
                };
                let pins = library.entry(id.to_string()).or_default();
                let mut visit = |node: &Expr, unit: u32| {
                    for pin in node
                        .children()
                        .iter()
                        .filter(|item| item.head() == Some("pin"))
                    {
                        let Some(number) = form_atom(pin, "number", 1) else {
                            continue;
                        };
                        pins.push((
                            number.to_string(),
                            SymbolPin {
                                unit,
                                kind: pin
                                    .children()
                                    .get(1)
                                    .and_then(Expr::atom)
                                    .unwrap_or("")
                                    .to_string(),
                                name: form_atom(pin, "name", 1).unwrap_or("").to_string(),
                            },
                        ));
                    }
                };
                visit(symbol, 0);
                for unit_symbol in symbol
                    .children()
                    .iter()
                    .filter(|item| item.head() == Some("symbol"))
                {
                    // Sub-symbols are named <name>_<unit>_<style>.
                    let unit = unit_symbol
                        .children()
                        .get(1)
                        .and_then(Expr::atom)
                        .and_then(|name| name.rsplit('_').nth(1))
                        .and_then(|unit| unit.parse().ok())
                        .unwrap_or(0);
                    visit(unit_symbol, unit);
                }
            }
        }
        for item in sheet.children() {
            match item.head() {
                Some("symbol") => {
                    let Some(pins) = form_atom(item, "lib_id", 1).and_then(|id| library.get(id))
                    else {
                        continue;
                    };
                    let mut references: Vec<String> = item
                        .children()
                        .iter()
                        .filter(|child| {
                            child.head() == Some("property")
                                && child.children().get(1).and_then(Expr::atom) == Some("Reference")
                        })
                        .filter_map(|child| child.children().get(2).and_then(Expr::atom))
                        .map(str::to_string)
                        .collect();
                    // A sheet used more than once names each copy's
                    // reference under `instances`.
                    if let Some(instances) = item.child("instances") {
                        let mut stack: Vec<&Expr> = vec![instances];
                        while let Some(node) = stack.pop() {
                            if let Some(reference) = form_atom(node, "reference", 1)
                                && !references.iter().any(|known| known == reference)
                            {
                                references.push(reference.to_string());
                            }
                            stack.extend(
                                node.children()
                                    .iter()
                                    .filter(|child| child.head().is_some()),
                            );
                        }
                    }
                    for reference in references {
                        for (number, pin) in pins {
                            result
                                .entry((reference.clone(), number.clone()))
                                .or_insert_with(|| pin.clone());
                        }
                    }
                }
                Some("sheet") => {
                    if let Some(file) = item
                        .children()
                        .iter()
                        .find(|child| {
                            child.head() == Some("property")
                                && matches!(
                                    child.children().get(1).and_then(Expr::atom),
                                    Some("Sheetfile" | "Sheet file")
                                )
                        })
                        .and_then(|child| child.children().get(2).and_then(Expr::atom))
                    {
                        pending.push(path.parent().unwrap_or(directory).join(file));
                    }
                }
                _ => {}
            }
        }
    }
    result
}

/// Reference prefixes whose passive pins are not interchangeable: diodes,
/// transistors, switches, crystals, fuses, inductors, transformers,
/// relays, test points and the like (a resistor network's are).
fn fixed_passive(prefix: &str) -> bool {
    matches!(
        prefix,
        "D" | "LED"
            | "Q"
            | "SW"
            | "S"
            | "K"
            | "F"
            | "FB"
            | "L"
            | "Y"
            | "X"
            | "T"
            | "BT"
            | "TP"
            | "H"
            | "MH"
            | "JP"
    )
}

/// The `swappable` glob for a general-purpose pin function: `GPIO*`,
/// `IO*`, `PA*` (port A), `P0.*`.
fn general_purpose(function: &str) -> Option<String> {
    let digits = |text: &str| !text.is_empty() && text.chars().all(|c| c.is_ascii_digit());
    // KiCad 10 appends the pin number (`GPIO12_5`).
    let function = function
        .rsplit_once('_')
        .filter(|(_, number)| digits(number))
        .map_or(function, |(name, _)| name);
    if function.strip_prefix("GPIO").is_some_and(digits) {
        return Some("GPIO*".into());
    }
    if function.strip_prefix("IO").is_some_and(digits) {
        return Some("IO*".into());
    }
    let mut characters = function.chars();
    match (characters.next(), characters.next()) {
        (Some('P'), Some(port @ 'A'..='K')) if digits(characters.as_str()) => {
            Some(format!("P{port}*"))
        }
        (Some('P'), Some(port @ '0'..='9')) => {
            let rest = characters.as_str();
            rest.strip_prefix('.')
                .is_some_and(digits)
                .then(|| format!("P{port}.*"))
        }
        _ => None,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadDescribedOutline {
    pub minimum: [f64; 2],
    pub maximum: [f64; 2],
    pub width: f64,
    pub height: f64,
    /// The outline itself (arcs as segments), for tests against its shape
    /// rather than its box.
    pub points: Vec<[f64; 2]>,
    /// True when the board draws nothing on Edge.Cuts itself and this
    /// outline comes from footprints' Edge.Cuts (a slide switch's slots on
    /// a board fresh from a netlist): not the board's shape. Give the board
    /// one with `outline` in constraints.json.
    pub from_footprints: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadDescribedFootprint {
    pub reference: String,
    /// The footprint's `uuid` (`tstamp` in older files): it tells apart
    /// footprints sharing a reference (a panel's copies), and a constraint
    /// entry for such a reference names its footprint by it.
    pub uuid: Option<String>,
    pub value: String,
    pub footprint: String,
    /// Origin and orientation, as KiCad stores them.
    pub at: [f64; 3],
    /// The side the part occupies: where its SMD pads are (else its
    /// footprint's layer).
    pub side: String,
    /// The footprint's own layer, `F.Cu` or `B.Cu`: what `back` and
    /// `front` in constraints.json set. It differs from `side` for a
    /// footprint whose pads are on the other side (urchin's key sockets:
    /// front footprints with their pads on B.Cu).
    pub layer: String,
    /// Courtyard (or pad) box in the footprint's own frame, before rotation.
    pub size_mm: [f64; 2],
    /// The body's box on the board: [min x, min y, max x, max y].
    pub body: [f64; 4],
    /// The body itself on the board: its corners, turned with the part, or
    /// for a round body (a mounting hole) its centre and radius.
    pub body_corners: Vec<[f64; 2]>,
    pub body_circle: Option<([f64; 2], f64)>,
    pub through_hole: bool,
    pub locked: bool,
    /// Pad name and net (`None` for an unconnected pad).
    pub pads: Vec<(String, Option<String>)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadDescribedNet {
    pub name: String,
    /// `REF:PAD` for every pad on the net.
    pub pads: Vec<String>,
    pub track_width_mm: Option<f64>,
    pub clearance_mm: Option<f64>,
}

fn property(footprint: &Expr, name: &str) -> String {
    footprint
        .children()
        .iter()
        .find_map(|item| {
            if item.head() != Some("property") || item.children().get(1)?.atom()? != name {
                return None;
            }
            Some(item.children().get(2)?.atom()?.to_string())
        })
        .unwrap_or_default()
}

pub fn describe_kicad_board(
    directory: &Path,
    board_id: &str,
) -> Result<KiCadBoardDescription, String> {
    let board_path = directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&board_path)
        .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let pcb = parse(&text)?;
    let names = NetNames::of(&pcb);
    let board_edge = pcb.children().iter().any(|item| {
        item.head().is_some_and(|head| head.starts_with("gr_"))
            && item
                .child("layer")
                .and_then(|layer| layer.children().get(1))
                .and_then(Expr::atom)
                == Some("Edge.Cuts")
    });
    let outline = outline::board_loops(&pcb).ok().map(|loops| {
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for point in &loops.outline {
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(point[axis]);
                maximum[axis] = maximum[axis].max(point[axis]);
            }
        }
        KiCadDescribedOutline {
            minimum,
            maximum,
            width: maximum[0] - minimum[0],
            height: maximum[1] - minimum[1],
            points: loops.outline.clone(),
            from_footprints: !board_edge,
        }
    });
    let layers = board_router::LayerTable::from_pcb(&pcb)?;
    let rules = project_rules::resolve_project_rules(
        &directory.join(format!("{board_id}.kicad_pro")),
        &board_path,
    )
    .ok();
    let mut footprints = Vec::new();
    let mut nets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // General-purpose pads per part: (part, value, pad, function, glob, net).
    let mut general: Vec<(String, String, String, String, String, String)> = Vec::new();
    // Pads by electrical type: (part, value, pad, pin name, unit, type, net).
    let mut typed: Vec<(String, String, String, String, u32, String, String)> = Vec::new();
    // Parts with a pad function on the board: their names decide.
    let mut functioned: BTreeSet<String> = BTreeSet::new();
    let symbol_pins = schematic_symbol_pins(directory, board_id);
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let (center, size, round_body) = local_body(footprint)?;
        let at = form_at(footprint)?;
        let (sin, cos) = (-at[2]).to_radians().sin_cos();
        let offset = [
            center[0] * cos - center[1] * sin,
            center[0] * sin + center[1] * cos,
        ];
        let middle = [at[0] + offset[0], at[1] + offset[1]];
        let half = [
            (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0,
            (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0,
        ];
        let round = |value: f64| (value * 1000.0).round() / 1000.0;
        let body = [
            round(middle[0] - half[0]),
            round(middle[1] - half[1]),
            round(middle[0] + half[0]),
            round(middle[1] + half[1]),
        ];
        let mut pads = Vec::new();
        let mut through_hole = false;
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            let name = pad
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or("")
                .to_string();
            through_hole |= matches!(
                pad.children().get(2).and_then(Expr::atom),
                Some("thru_hole" | "np_thru_hole")
            );
            let net = node_net(pad)
                .filter(|raw| names.routable(raw))
                .map(|raw| names.normalize(raw).to_string());
            if let Some(net) = &net {
                nets.entry(net.clone())
                    .or_default()
                    .push(format!("{reference}:{name}"));
                let function = form_atom(pad, "pinfunction", 1).unwrap_or("");
                if !function.is_empty() {
                    functioned.insert(reference.clone());
                }
                if let Some(glob) = general_purpose(function) {
                    general.push((
                        reference.clone(),
                        property(footprint, "Value"),
                        name.clone(),
                        function.to_string(),
                        glob,
                        net.clone(),
                    ));
                }
                let symbol = symbol_pins.get(&(reference.clone(), name.clone()));
                let kind = form_atom(pad, "pintype", 1)
                    .map(|kind| kind.split('+').next().unwrap_or(kind))
                    .or(symbol.map(|pin| pin.kind.as_str()))
                    .unwrap_or("");
                if matches!(kind, "bidirectional" | "passive") {
                    typed.push((
                        reference.clone(),
                        property(footprint, "Value"),
                        name.clone(),
                        if function.is_empty() {
                            symbol.map_or(String::new(), |pin| pin.name.clone())
                        } else {
                            function.to_string()
                        },
                        symbol.map_or(0, |pin| pin.unit),
                        kind.to_string(),
                        net.clone(),
                    ));
                }
            }
            pads.push((name, net));
        }
        footprints.push(KiCadDescribedFootprint {
            value: property(footprint, "Value"),
            footprint: footprint
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or("")
                .to_string(),
            at,
            side: if crate::board_placer::on_back(footprint) {
                "back"
            } else {
                "front"
            }
            .into(),
            layer: if form_atom(footprint, "layer", 1) == Some("B.Cu") {
                "B.Cu"
            } else {
                "F.Cu"
            }
            .into(),
            size_mm: [round(size[0]), round(size[1])],
            body_corners: [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
                .iter()
                .map(|[sx, sy]| {
                    let local = [sx * size[0] / 2.0, sy * size[1] / 2.0];
                    [
                        round(middle[0] + local[0] * cos - local[1] * sin),
                        round(middle[1] + local[0] * sin + local[1] * cos),
                    ]
                })
                .collect(),
            body_circle: round_body
                .then(|| ([round(middle[0]), round(middle[1])], round(size[0] / 2.0))),
            body,
            through_hole,
            locked: footprint.child("locked").is_some()
                || footprint
                    .children()
                    .iter()
                    .any(|child| child.atom() == Some("locked")),
            pads,
            uuid: footprint_uuid(footprint),
            reference,
        });
    }
    let nets: Vec<KiCadDescribedNet> = nets
        .into_iter()
        .map(|(name, mut pads)| {
            pads.sort();
            let rule = rules.as_ref().and_then(|rules| {
                rules
                    .connection_rules
                    .get(&name)
                    .or(rules.default_rules.as_ref())
            });
            KiCadDescribedNet {
                track_width_mm: rule.map(|rule| rule.trace_width_mm),
                clearance_mm: rule.map(|rule| rule.clearance_mm),
                name,
                pads,
            }
        })
        .collect();
    // Candidates: at least three general-purpose pads on nets that reach
    // another part. Not connectors: their pin order is the interface.
    let mut swap_candidates: Vec<KiCadSwapCandidate> = Vec::new();
    for (part, value, pad, function, glob, net) in general {
        let prefix = part.trim_end_matches(|c: char| c.is_ascii_digit());
        if matches!(prefix, "J" | "P" | "CN" | "CON" | "X" | "USB")
            || nets
                .iter()
                .find(|described| described.name == net)
                .is_none_or(|described| described.pads.len() < 2)
        {
            continue;
        }
        let index = match swap_candidates
            .iter()
            .position(|candidate| candidate.part == part)
        {
            Some(index) => index,
            None => {
                swap_candidates.push(KiCadSwapCandidate {
                    part,
                    value,
                    pins: Vec::new(),
                    connected: Vec::new(),
                    by: "function".into(),
                    unit: None,
                });
                swap_candidates.len() - 1
            }
        };
        let candidate = &mut swap_candidates[index];
        if !candidate.pins.contains(&glob) {
            candidate.pins.push(glob);
        }
        candidate.connected.push(format!("{pad} ({function})"));
    }
    swap_candidates.retain(|candidate| candidate.connected.len() >= 3);
    // Parts whose pads name no function on the board (a generated board,
    // the fence's: the names are in the schematic only): bidirectional or
    // passive pins
    // of one symbol unit on signal nets (not ground or a rail) that reach
    // another part, at least three on three nets; their pad numbers are
    // the globs. Not connectors (their pin order is the interface) and
    // not parts whose passive pins differ (diodes, transistors, switches,
    // crystals), unless their pins are bidirectional.
    let mut by_unit: BTreeMap<(String, u32), KiCadSwapCandidate> = BTreeMap::new();
    let mut unit_nets: BTreeMap<(String, u32), BTreeSet<String>> = BTreeMap::new();
    for (part, value, pad, pin_name, unit, kind, net) in typed {
        if functioned.contains(&part) {
            continue;
        }
        let prefix = part.trim_end_matches(|c: char| c.is_ascii_digit());
        if matches!(prefix, "J" | "P" | "CN" | "CON" | "X" | "USB")
            || (kind == "passive" && fixed_passive(prefix))
            || crate::quality::ground(&net)
            || crate::quality::rail_name(&net)
            || nets
                .iter()
                .find(|described| described.name == net)
                .is_none_or(|described| {
                    !described.pads.iter().any(|other| {
                        other
                            .split_once(':')
                            .is_some_and(|(owner, _)| owner != part)
                    })
                })
        {
            continue;
        }
        let key = (part.clone(), unit);
        unit_nets.entry(key.clone()).or_default().insert(net);
        let candidate = by_unit.entry(key).or_insert_with(|| KiCadSwapCandidate {
            part,
            value,
            pins: Vec::new(),
            connected: Vec::new(),
            by: "electrical type".into(),
            unit: Some(unit),
        });
        if !candidate.pins.contains(&pad) {
            candidate.pins.push(pad.clone());
        }
        candidate.connected.push(if pin_name.is_empty() {
            format!("{pad} ({kind})")
        } else {
            format!("{pad} ({pin_name}, {kind})")
        });
    }
    let mut typed_candidates: Vec<KiCadSwapCandidate> = by_unit
        .into_iter()
        .filter(|(key, candidate)| {
            candidate.connected.len() >= 3 && unit_nets.get(key).is_some_and(|nets| nets.len() >= 3)
        })
        .map(|(_, candidate)| candidate)
        .collect();
    typed_candidates.sort_by(|a, b| {
        crate::canonical_order::natural_cmp(&a.part, &b.part).then(a.unit.cmp(&b.unit))
    });
    swap_candidates.extend(typed_candidates);
    Ok(KiCadBoardDescription {
        board_id: board_id.into(),
        outline,
        copper_layers: layers.names.clone(),
        footprints,
        nets,
        swap_candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hierarchical_and_a_bare_net_of_one_name_stay_two_nets() {
        // gametank's expansion-port board: pads on "/A15" and on "A15",
        // two nets, whatever board this thread lowered before.
        crate::note_net_names(&parse("(kicad_pcb)").unwrap());
        let board = r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
            (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
            (footprint "R_0805" (layer "F.Cu") (at 10 10) (property "Reference" "R1") (property "Value" "10k")
              (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net "A15"))
              (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net "/A15")))
            (footprint "R_0805" (layer "F.Cu") (at 30 10) (property "Reference" "R2") (property "Value" "10k")
              (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net "A15"))
              (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net "/A15"))))"#;
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-describe-nets-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("b.kicad_pcb"), board).unwrap();
        let description = describe_kicad_board(&directory, "b");
        fs::remove_dir_all(&directory).ok();
        let mut nets: Vec<(String, Vec<String>)> = description
            .unwrap()
            .nets
            .into_iter()
            .map(|net| (net.name, net.pads))
            .collect();
        nets.sort();
        assert_eq!(
            nets,
            [
                (
                    "/A15".to_string(),
                    vec!["R1:2".to_string(), "R2:2".to_string()]
                ),
                (
                    "A15".to_string(),
                    vec!["R1:1".to_string(), "R2:1".to_string()]
                )
            ]
        );
    }

    #[test]
    fn general_purpose_pins_by_function() {
        for (function, glob) in [
            ("GPIO12", "GPIO*"),
            ("GPIO12_5", "GPIO*"),
            ("IO5", "IO*"),
            ("PA3", "PA*"),
            ("PB12_40", "PB*"),
            ("P0.13", "P0.*"),
        ] {
            assert_eq!(
                general_purpose(function).as_deref(),
                Some(glob),
                "{function}"
            );
        }
        for function in [
            "GND", "VDD", "PAD", "IO", "PA", "P0", "EN", "GPIO", "IO12/ADC", "PWR",
        ] {
            assert_eq!(general_purpose(function), None, "{function}");
        }
    }
    #[test]
    fn swap_candidates_by_electrical_type_where_pads_name_no_function() {
        // U1's pads carry no pinfunction (a generated board); the
        // schematic names its pins PA15, PA0/A0, ... as bidirectional and
        // its supply as power_in. RN1 is a resistor network (passive), D1
        // a dual diode (passive, not interchangeable), J1 a connector.
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-describe-typed-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let pad = |number: &str, x: f64, net: &str| {
            format!(
                r#"(pad "{number}" smd rect (at {x} 0) (size 0.3 1) (layers "F.Cu") (net 1 "{net}"))"#
            )
        };
        let part = |reference: &str, x: f64, pads: &[(&str, &str)]| {
            let pads: Vec<String> = pads
                .iter()
                .enumerate()
                .map(|(index, (number, net))| pad(number, index as f64, net))
                .collect();
            format!(
                r#"(footprint "f" (layer "F.Cu") (at {x} 10) (property "Reference" "{reference}") (property "Value" "v") {})"#,
                pads.join(" ")
            )
        };
        let board = format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts")) {} {} {} {})"#,
            part(
                "U1",
                10.0,
                &[
                    ("1", "ROW1"),
                    ("2", "ROW2"),
                    ("10", "ROW3"),
                    ("47", "GND"),
                    ("48", "+3V3")
                ]
            ),
            part(
                "RN1",
                25.0,
                &[
                    ("1", "ROW1"),
                    ("2", "ROW2"),
                    ("3", "ROW3"),
                    ("8", "X1"),
                    ("7", "X2"),
                    ("6", "X3")
                ]
            ),
            part("D1", 35.0, &[("1", "ROW1"), ("2", "ROW2"), ("3", "ROW3")]),
            part(
                "J1",
                45.0,
                &[
                    ("1", "X1"),
                    ("2", "X2"),
                    ("3", "X3"),
                    ("4", "GND"),
                    ("5", "+3V3")
                ]
            ),
        );
        let symbol = |id: &str, pins: &[(&str, &str, &str)]| {
            let pins: Vec<String> = pins
                .iter()
                .map(|(kind, name, number)| {
                    format!(r#"(pin {kind} line (at 0 0 0) (length 2.54) (name "{name}") (number "{number}"))"#)
                })
                .collect();
            format!(
                r#"(symbol "L:{id}" (symbol "{id}_0_1") (symbol "{id}_1_1" {}))"#,
                pins.join(" ")
            )
        };
        let placed = |id: &str, reference: &str| {
            format!(
                r#"(symbol (lib_id "L:{id}") (at 0 0 0) (unit 1) (property "Reference" "{reference}"))"#
            )
        };
        let schematic = format!(
            "(kicad_sch (lib_symbols {} {} {}) {} {} {})",
            symbol(
                "MCU",
                &[
                    ("bidirectional", "PA15", "1"),
                    ("bidirectional", "PA16", "2"),
                    ("bidirectional", "PA0/A0", "10"),
                    ("power_in", "GND", "47"),
                    ("power_in", "VDD", "48"),
                ]
            ),
            symbol(
                "RN",
                &[
                    ("passive", "R1.1", "1"),
                    ("passive", "R2.1", "2"),
                    ("passive", "R3.1", "3"),
                    ("passive", "R3.2", "6"),
                    ("passive", "R2.2", "7"),
                    ("passive", "R1.2", "8"),
                ]
            ),
            symbol(
                "D",
                &[
                    ("passive", "A", "1"),
                    ("passive", "K", "2"),
                    ("passive", "AK", "3")
                ]
            ),
            placed("MCU", "U1"),
            placed("RN", "RN1"),
            placed("D", "D1"),
        );
        fs::write(directory.join("b.kicad_pcb"), board).unwrap();
        fs::write(directory.join("b.kicad_sch"), schematic).unwrap();
        let description = describe_kicad_board(&directory, "b").unwrap();
        let summary: Vec<(String, Vec<String>, String, Option<u32>)> = description
            .swap_candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.part.clone(),
                    candidate.pins.clone(),
                    candidate.by.clone(),
                    candidate.unit,
                )
            })
            .collect();
        let pins = |list: &[&str]| list.iter().map(|pin| pin.to_string()).collect::<Vec<_>>();
        assert_eq!(
            summary,
            [
                (
                    "RN1".to_string(),
                    pins(&["1", "2", "3", "8", "7", "6"]),
                    "electrical type".to_string(),
                    Some(1)
                ),
                (
                    "U1".to_string(),
                    pins(&["1", "2", "10"]),
                    "electrical type".to_string(),
                    Some(1)
                ),
            ]
        );
        assert_eq!(
            description.swap_candidates[1].connected[0],
            "1 (PA15, bidirectional)"
        );
        let _ = fs::remove_dir_all(&directory);
    }
}

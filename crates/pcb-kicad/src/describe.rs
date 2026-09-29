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
    /// Parts with several connected general-purpose pins (`GPIO12`, `IO5`,
    /// `PA3`, `P0.13`): if the firmware can use any of them for any of
    /// these signals, `swappable` lets the layout choose.
    pub swap_candidates: Vec<KiCadSwapCandidate>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadSwapCandidate {
    pub part: String,
    pub value: String,
    /// Globs for the layout config's `swappable` `pins`.
    pub pins: Vec<String>,
    /// The connected pads they match, as `PAD (function)`.
    pub connected: Vec<String>,
}

/// The `swappable` glob for a general-purpose pin function: `GPIO*`,
/// `IO*`, `PA*` (port A), `P0.*`.
fn general_purpose(function: &str) -> Option<String> {
    let digits = |text: &str| !text.is_empty() && text.chars().all(|c| c.is_ascii_digit());
    // KiCad 10 appends the pin number (`GPIO12_5`).
    let function = function.rsplit_once('_').filter(|(_, number)| digits(number)).map_or(function, |(name, _)| name);
    if function.strip_prefix("GPIO").is_some_and(digits) {
        return Some("GPIO*".into());
    }
    if function.strip_prefix("IO").is_some_and(digits) {
        return Some("IO*".into());
    }
    let mut characters = function.chars();
    match (characters.next(), characters.next()) {
        (Some('P'), Some(port @ 'A'..='K')) if digits(characters.as_str()) => Some(format!("P{port}*")),
        (Some('P'), Some(port @ '0'..='9')) => {
            let rest = characters.as_str();
            rest.strip_prefix('.').is_some_and(digits).then(|| format!("P{port}.*"))
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
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadDescribedFootprint {
    pub reference: String,
    pub value: String,
    pub footprint: String,
    /// Origin and orientation, as KiCad stores them.
    pub at: [f64; 3],
    pub side: String,
    /// Courtyard (or pad) box in the footprint's own frame, before rotation.
    pub size_mm: [f64; 2],
    /// The body's box on the board: [min x, min y, max x, max y].
    pub body: [f64; 4],
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

pub fn describe_kicad_board(directory: &Path, board_id: &str) -> Result<KiCadBoardDescription, String> {
    let board_path = directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&board_path).map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let pcb = parse(&text)?;
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
        }
    });
    let layers = board_router::LayerTable::from_pcb(&pcb)?;
    let rules = project_rules::resolve_project_rules(&directory.join(format!("{board_id}.kicad_pro")), &board_path).ok();
    let mut footprints = Vec::new();
    let mut nets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // General-purpose pads per part: (part, value, pad, function, glob, net).
    let mut general: Vec<(String, String, String, String, String, String)> = Vec::new();
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let (center, size, _) = local_body(footprint)?;
        let at = form_at(footprint)?;
        let (sin, cos) = (-at[2]).to_radians().sin_cos();
        let offset = [center[0] * cos - center[1] * sin, center[0] * sin + center[1] * cos];
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
        for pad in footprint.children().iter().filter(|child| child.head() == Some("pad")) {
            let name = pad.children().get(1).and_then(Expr::atom).unwrap_or("").to_string();
            through_hole |= matches!(pad.children().get(2).and_then(Expr::atom), Some("thru_hole" | "np_thru_hole"));
            let net = node_net(pad)
                .filter(|raw| board_router::routable_net(raw))
                .map(|raw| normalize_net(raw).to_string());
            if let Some(net) = &net {
                nets.entry(net.clone()).or_default().push(format!("{reference}:{name}"));
                let function = form_atom(pad, "pinfunction", 1).unwrap_or("");
                if let Some(glob) = general_purpose(function) {
                    general.push((reference.clone(), property(footprint, "Value"), name.clone(), function.to_string(), glob, net.clone()));
                }
            }
            pads.push((name, net));
        }
        footprints.push(KiCadDescribedFootprint {
            value: property(footprint, "Value"),
            footprint: footprint.children().get(1).and_then(Expr::atom).unwrap_or("").to_string(),
            at,
            side: if crate::board_placer::on_back(footprint) { "back" } else { "front" }.into(),
            size_mm: [round(size[0]), round(size[1])],
            body,
            through_hole,
            locked: footprint.child("locked").is_some()
                || footprint.children().iter().any(|child| child.atom() == Some("locked")),
            pads,
            reference,
        });
    }
    let nets: Vec<KiCadDescribedNet> = nets
        .into_iter()
        .map(|(name, mut pads)| {
            pads.sort();
            let rule = rules
                .as_ref()
                .and_then(|rules| rules.connection_rules.get(&name).or(rules.default_rules.as_ref()));
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
            || nets.iter().find(|described| described.name == net).is_none_or(|described| described.pads.len() < 2)
        {
            continue;
        }
        let index = match swap_candidates.iter().position(|candidate| candidate.part == part) {
            Some(index) => index,
            None => {
                swap_candidates.push(KiCadSwapCandidate { part, value, pins: Vec::new(), connected: Vec::new() });
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
    fn general_purpose_pins_by_function() {
        for (function, glob) in [("GPIO12", "GPIO*"), ("GPIO12_5", "GPIO*"), ("IO5", "IO*"), ("PA3", "PA*"), ("PB12_40", "PB*"), ("P0.13", "P0.*")] {
            assert_eq!(general_purpose(function).as_deref(), Some(glob), "{function}");
        }
        for function in ["GND", "VDD", "PAD", "IO", "PA", "P0", "EN", "GPIO", "IO12/ADC", "PWR"] {
            assert_eq!(general_purpose(function), None, "{function}");
        }
    }
}

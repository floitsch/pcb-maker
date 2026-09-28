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
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let (_, size, _) = local_body(footprint)?;
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
            }
            pads.push((name, net));
        }
        footprints.push(KiCadDescribedFootprint {
            value: property(footprint, "Value"),
            footprint: footprint.children().get(1).and_then(Expr::atom).unwrap_or("").to_string(),
            at: form_at(footprint)?,
            side: if form_atom(footprint, "layer", 1) == Some("B.Cu") { "back" } else { "front" }.into(),
            size_mm: [(size[0] * 1000.0).round() / 1000.0, (size[1] * 1000.0).round() / 1000.0],
            through_hole,
            locked: footprint.child("locked").is_some()
                || footprint.children().iter().any(|child| child.atom() == Some("locked")),
            pads,
            reference,
        });
    }
    let nets = nets
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
    Ok(KiCadBoardDescription {
        board_id: board_id.into(),
        outline,
        copper_layers: layers.names.clone(),
        footprints,
        nets,
    })
}

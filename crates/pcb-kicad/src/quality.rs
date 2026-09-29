// Copyright (C) 2026 Toit contributors.

//! Objective layout-quality measures of a KiCad board (see
//! docs/quality.md): what the parts and nets are for is inferred from the
//! board file (net names, pad pin types, references, footprint names), then
//! each measure is computed from the geometry. The same report is made for
//! a designer's board and for ours, so the two can be compared.

use super::*;
use crate::board_placer::{connector_mouth, local_body};

#[derive(Clone, Debug, Serialize)]
pub struct KiCadQualityReport {
    pub board_id: String,
    pub economy: KiCadEconomy,
    pub decoupling: KiCadDecoupling,
    pub crystals: Vec<KiCadCrystal>,
    pub separation: KiCadSeparation,
    pub assembly: KiCadAssembly,
    pub connectors: KiCadConnectors,
    pub manufacturing: KiCadManufacturing,
    /// Ground pours and how much other copper cuts them (return paths).
    pub planes: Vec<KiCadPlane>,
    /// Estimated price of 10 boards at common fabs (docs/cost.md).
    pub cost: crate::cost::KiCadCost,
}

/// What the board costs to make, in the terms fabs price by.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadEconomy {
    pub width_mm: f64,
    pub height_mm: f64,
    pub copper_layers: usize,
    pub parts: usize,
    /// Distinct value + footprint pairs (feeder slots at an assembler).
    pub unique_parts: usize,
    pub through_hole_parts: usize,
    /// Sides carrying SMD parts (1: one assembly pass).
    pub smd_sides: usize,
    pub vias: usize,
    pub smallest_via_drill_mm: Option<f64>,
    pub narrowest_track_mm: Option<f64>,
    pub track_length_mm: f64,
}

/// Distance from every IC supply pin to the nearest decoupling capacitor
/// pad on the same rail (pad centre to pad centre).
#[derive(Clone, Debug, Serialize)]
pub struct KiCadDecoupling {
    pub supply_pins: usize,
    pub capacitors: usize,
    pub median_mm: Option<f64>,
    /// Share of supply pins with a capacitor within 3 mm and within 5 mm.
    pub within_3mm: Option<f64>,
    pub within_5mm: Option<f64>,
    /// The farthest pins: `REF:PAD` and distance (none: no capacitor on
    /// that rail).
    pub worst: Vec<(String, Option<f64>)>,
    /// The other way round, for boards with fewer capacitors than pins: for
    /// every decoupling capacitor, the distance to the nearest supply pin on
    /// its rail. Median, and share within 3 mm.
    pub capacitor_median_mm: Option<f64>,
    pub capacitors_within_3mm: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadCrystal {
    pub reference: String,
    /// Longest pad-to-pad distance from the crystal to the IC pins it
    /// drives.
    pub longest_mm: Option<f64>,
}

/// Distances from noisy parts (inductors) to sensitive ones (crystals,
/// antenna modules).
#[derive(Clone, Debug, Serialize)]
pub struct KiCadSeparation {
    pub inductors: usize,
    pub sensitive: usize,
    /// Smallest body-to-body gap between an inductor and a sensitive part.
    pub closest_mm: Option<f64>,
    /// Pairs closer than 15 mm.
    pub too_close: Vec<(String, String, f64)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadAssembly {
    /// Polarised parts (diodes, LEDs, ICs, transistors, polarised caps).
    pub polarised: usize,
    /// Share of them at the most common orientation of their kind and side.
    pub orientation_consistency: Option<f64>,
    /// Parts (other than connectors and mounting holes) whose body is closer
    /// than 2.5 mm to the board edge: assemblers then need rails.
    pub near_edge: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadConnectors {
    /// Connectors with a clear mouth (the side a plug goes in).
    pub with_mouth: usize,
    /// Of those, the ones at an edge (body within 2 mm) facing out.
    pub facing_out: usize,
    pub not_facing_out: Vec<String>,
}

/// A ground pour on one layer: signal tracks on that layer cut it, and
/// return currents detour around the cuts.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadPlane {
    pub layer: String,
    pub net: String,
    /// Length of other nets' tracks on the layer.
    pub cut_by_mm: f64,
    /// The longest straight cut: the slot a return current must go around.
    pub longest_cut_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadManufacturing {
    /// Vias whose hole lies in a small SMD pad (solder wicks away).
    pub vias_in_pads: usize,
    /// Two-pad parts of 0603 or smaller whose pads take tracks more than
    /// twice as wide on one side (tombstoning).
    pub tombstone_risk: Vec<String>,
    /// Counts by type in the directory's `drc.json`, if there is one.
    pub drc: BTreeMap<String, usize>,
}

struct Pad {
    name: String,
    net: Option<String>,
    center: [f64; 2],
    half: [f64; 2],
    smd: bool,
    power_pin: bool,
}

struct Part {
    reference: String,
    value: String,
    footprint: String,
    angle: f64,
    back: bool,
    through_hole: bool,
    body: [f64; 4],
    pads: Vec<Pad>,
    mouth: Option<[f64; 2]>,
}

impl Part {
    fn prefix(&self) -> &str {
        self.reference.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_')
    }
}

pub(crate) fn ground(net: &str) -> bool {
    let name = net.trim_start_matches('/').to_ascii_uppercase();
    name.contains("GND") || name == "VSS" || name == "0V" || name.starts_with("VSS")
}

pub(crate) fn rail_name(net: &str) -> bool {
    let name = net.trim_start_matches('/').to_ascii_uppercase();
    // A voltage first: 3V3, 5V, 3.3V, 12V_IN, 3V3_DUT.
    let rest = name.trim_start_matches('+');
    let number = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
    let voltage = number.len() < rest.len() && number.starts_with('V');
    name.starts_with('+')
        || voltage
        || ["VCC", "VDD", "VBUS", "VIN", "VBAT", "VSYS", "VIO", "VREF", "VDDA", "AVCC", "AVDD", "DVDD", "V+"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

/// A capacitor's value in farads ("100n", "0.1uF", "4u7", "10 µF").
pub(crate) fn capacitance(value: &str) -> Option<f64> {
    let value = value.trim().replace('µ', "u").replace("μ", "u");
    let value = value.trim_end_matches(['F', 'f']).trim();
    let unit = value.find(|c: char| c.is_ascii_alphabetic())?;
    let scale = match value[unit..].chars().next()? {
        'p' => 1e-12,
        'n' => 1e-9,
        'u' => 1e-6,
        'm' => 1e-3,
        _ => return None,
    };
    let (whole, fraction) = (&value[..unit], &value[unit + 1..]);
    let fraction: String = fraction.chars().take_while(|c| c.is_ascii_digit()).collect();
    let number: f64 = if fraction.is_empty() { whole.trim().parse().ok()? } else { format!("{whole}.{fraction}").parse().ok()? };
    Some(number * scale)
}

fn box_gap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let dx = (a[0] - b[2]).max(b[0] - a[2]).max(0.0);
    let dy = (a[1] - b[3]).max(b[1] - a[3]).max(0.0);
    dx.hypot(dy)
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[values.len() / 2])
}

fn share(count: usize, total: usize) -> Option<f64> {
    (total > 0).then(|| (count as f64 / total as f64 * 1000.0).round() / 1000.0)
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn parts_of(pcb: &Expr) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        let at = form_at(footprint)?;
        let (center, size, _) = local_body(footprint)?;
        let (sin, cos) = (-at[2]).to_radians().sin_cos();
        let middle = [at[0] + center[0] * cos - center[1] * sin, at[1] + center[0] * sin + center[1] * cos];
        let half = [
            (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0,
            (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0,
        ];
        let mut pads = Vec::new();
        let mut through_hole = false;
        for pad in footprint.children().iter().filter(|child| child.head() == Some("pad")) {
            let kind = pad.children().get(2).and_then(Expr::atom).unwrap_or("");
            through_hole |= matches!(kind, "thru_hole" | "np_thru_hole");
            let pad_at = form_at(pad)?;
            let offset = rotate_vector([pad_at[0], pad_at[1]], -at[2]);
            let size = form_xy(pad, "size").unwrap_or([0.0, 0.0]);
            let (sin, cos) = (-pad_at[2]).to_radians().sin_cos();
            pads.push(Pad {
                name: pad.children().get(1).and_then(Expr::atom).unwrap_or("").to_string(),
                net: node_net(pad).filter(|raw| board_router::routable_net(raw)).map(|raw| normalize_net(raw).to_string()),
                center: [at[0] + offset[0], at[1] + offset[1]],
                half: [
                    (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0,
                    (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0,
                ],
                smd: kind == "smd",
                power_pin: matches!(form_atom(pad, "pintype", 1), Some("power_in")),
            });
        }
        let property = |name: &str| {
            footprint
                .children()
                .iter()
                .find(|item| item.head() == Some("property") && item.children().get(1).and_then(Expr::atom) == Some(name))
                .and_then(|item| item.children().get(2).and_then(Expr::atom))
                .unwrap_or("")
                .to_string()
        };
        parts.push(Part {
            reference: footprint_reference(footprint).unwrap_or_default(),
            value: property("Value"),
            footprint: footprint.children().get(1).and_then(Expr::atom).unwrap_or("").to_string(),
            angle: at[2],
            back: crate::board_placer::on_back(footprint),
            through_hole,
            body: [middle[0] - half[0], middle[1] - half[1], middle[0] + half[0], middle[1] + half[1]],
            pads,
            mouth: connector_mouth(footprint)?.map(|local| rotate_vector(local, -at[2])),
        });
    }
    Ok(parts)
}

struct Track {
    start: [f64; 2],
    end: [f64; 2],
    width: f64,
    layer: String,
    net: Option<String>,
}

pub fn score_kicad_board(directory: &Path, board_id: &str) -> Result<KiCadQualityReport, String> {
    let board_path = directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&board_path).map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let pcb = parse(&text)?;
    let parts = parts_of(&pcb)?;
    let layers = board_router::LayerTable::from_pcb(&pcb)?;
    let outline = outline::board_loops(&pcb).ok().map(|loops| {
        loops.outline.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
            [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
        })
    });

    let mut tracks = Vec::new();
    let mut vias = Vec::new();
    for item in pcb.children() {
        match item.head() {
            Some("segment" | "arc") => {
                tracks.push(Track {
                    start: form_xy(item, "start")?,
                    end: form_xy(item, "end")?,
                    width: form_f64(item, "width", 1).unwrap_or(0.0),
                    layer: form_atom(item, "layer", 1).unwrap_or("").to_string(),
                    net: node_net(item).map(|raw| normalize_net(raw).to_string()),
                });
            }
            Some("via") => {
                let at = form_at(item)?;
                vias.push(([at[0], at[1]], form_f64(item, "drill", 1).unwrap_or(0.0)));
            }
            _ => {}
        }
    }

    // Economy.
    let mut unique = BTreeSet::new();
    let mut smd_sides = BTreeSet::new();
    for part in &parts {
        if part.pads.is_empty() {
            continue;
        }
        unique.insert((part.value.clone(), part.footprint.clone()));
        if part.pads.iter().any(|pad| pad.smd) {
            smd_sides.insert(part.back);
        }
    }
    let economy = KiCadEconomy {
        width_mm: outline.map_or(0.0, |b| round(b[2] - b[0])),
        height_mm: outline.map_or(0.0, |b| round(b[3] - b[1])),
        copper_layers: layers.len(),
        parts: parts.iter().filter(|part| !part.pads.is_empty()).count(),
        unique_parts: unique.len(),
        through_hole_parts: parts.iter().filter(|part| part.through_hole && part.pads.iter().any(|pad| pad.net.is_some())).count(),
        smd_sides: smd_sides.len(),
        vias: vias.len(),
        smallest_via_drill_mm: vias.iter().map(|(_, drill)| *drill).filter(|drill| *drill > 0.0).reduce(f64::min),
        narrowest_track_mm: tracks.iter().map(|track| track.width).filter(|width| *width > 0.0).reduce(f64::min),
        track_length_mm: round(tracks.iter().map(|track| distance(track.start, track.end)).sum()),
    };

    // Decoupling: supply pins of ICs against small capacitors between a
    // rail and ground.
    let rail = |net: &str| !ground(net) && rail_name(net);
    let mut rails = BTreeSet::new();
    for part in &parts {
        for pad in &part.pads {
            if let Some(net) = &pad.net
                && !ground(net)
                && (pad.power_pin || rail(net))
            {
                rails.insert(net.clone());
            }
        }
    }
    let ic = |part: &Part| matches!(part.prefix(), "U" | "IC") && part.pads.len() >= 3;
    let mut capacitor_pads: BTreeMap<String, Vec<[f64; 2]>> = BTreeMap::new();
    let mut supply_pads: BTreeMap<String, Vec<[f64; 2]>> = BTreeMap::new();
    for part in parts.iter().filter(|part| ic(part)) {
        for pad in &part.pads {
            if let Some(net) = &pad.net
                && rails.contains(net)
                && (pad.power_pin || rail(net))
            {
                supply_pads.entry(net.clone()).or_default().push(pad.center);
            }
        }
    }
    let mut capacitors = 0;
    for part in parts.iter().filter(|part| part.prefix() == "C" && part.pads.len() == 2) {
        let nets: Vec<Option<&String>> = part.pads.iter().map(|pad| pad.net.as_ref()).collect();
        let small = capacitance(&part.value).is_none_or(|farads| farads <= 1.0e-6 + 1e-12);
        let (Some(a), Some(b)) = (nets[0], nets[1]) else {
            continue;
        };
        let supply = match (rails.contains(a) && ground(b), rails.contains(b) && ground(a)) {
            (true, _) => 0,
            (_, true) => 1,
            _ => continue,
        };
        if small {
            capacitors += 1;
            capacitor_pads
                .entry(part.pads[supply].net.clone().unwrap())
                .or_default()
                .push(part.pads[supply].center);
        }
    }
    let mut pin_distances = Vec::new();
    for part in parts.iter().filter(|part| ic(part)) {
        for pad in &part.pads {
            let Some(net) = &pad.net else {
                continue;
            };
            if !rails.contains(net) || !(pad.power_pin || rail(net)) {
                continue;
            }
            let nearest = capacitor_pads
                .get(net)
                .and_then(|pads| pads.iter().map(|center| distance(*center, pad.center)).reduce(f64::min));
            pin_distances.push((format!("{}:{}", part.reference, pad.name), nearest));
        }
    }
    let mut capacitor_distances: Vec<f64> = capacitor_pads
        .iter()
        .flat_map(|(net, pads)| {
            let pins = supply_pads.get(net);
            pads.iter().filter_map(move |pad| {
                pins?.iter().map(|pin| distance(*pin, *pad)).reduce(f64::min)
            })
        })
        .collect();
    let capacitors_within_3mm = share(
        capacitor_distances.iter().filter(|d| **d <= 3.0).count(),
        capacitor_distances.len(),
    );
    let capacitor_median_mm = median(&mut capacitor_distances).map(round);
    let mut found: Vec<f64> = pin_distances.iter().filter_map(|(_, d)| *d).collect();
    let supply_pins = pin_distances.len();
    let within = |limit: f64| share(pin_distances.iter().filter(|(_, d)| d.is_some_and(|d| d <= limit)).count(), supply_pins);
    let (within_3mm, within_5mm) = (within(3.0), within(5.0));
    pin_distances.sort_by(|a, b| b.1.unwrap_or(f64::INFINITY).total_cmp(&a.1.unwrap_or(f64::INFINITY)));
    let decoupling = KiCadDecoupling {
        supply_pins,
        capacitors,
        median_mm: median(&mut found).map(round),
        within_3mm,
        within_5mm,
        worst: pin_distances.iter().take(5).map(|(pin, d)| (pin.clone(), d.map(round))).collect(),
        capacitor_median_mm,
        capacitors_within_3mm,
    };

    // Crystals: pads to the IC pins on the same nets.
    let crystal = |part: &Part| {
        let name = part.footprint.to_ascii_lowercase();
        part.prefix() == "Y" || name.contains("crystal") || name.contains("oscillator") || name.contains("resonator")
    };
    let mut crystals = Vec::new();
    for part in parts.iter().filter(|part| crystal(part)) {
        let mut longest: Option<f64> = None;
        for pad in &part.pads {
            let Some(net) = pad.net.as_ref().filter(|net| !ground(net) && !rails.contains(*net)) else {
                continue;
            };
            for other in parts.iter().filter(|other| ic(other)) {
                for target in other.pads.iter().filter(|target| target.net.as_ref() == Some(net)) {
                    let d = distance(pad.center, target.center);
                    longest = Some(longest.map_or(d, |l| l.max(d)));
                }
            }
        }
        crystals.push(KiCadCrystal { reference: part.reference.clone(), longest_mm: longest.map(round) });
    }

    // Separation: inductors against crystals and antenna modules.
    let antenna = |part: &Part| {
        let name = part.footprint.to_ascii_lowercase();
        name.contains("esp32") || name.contains("esp8266") || name.contains("wroom") || name.contains("antenna")
            || name.contains("rf_module") || name.contains("nrf24") || name.contains("rfm")
    };
    let inductors: Vec<&Part> = parts.iter().filter(|part| part.prefix() == "L" && part.pads.len() == 2).collect();
    let sensitive: Vec<&Part> = parts.iter().filter(|part| crystal(part) || antenna(part)).collect();
    let mut too_close = Vec::new();
    let mut closest: Option<f64> = None;
    for inductor in &inductors {
        for victim in &sensitive {
            let gap = box_gap(inductor.body, victim.body);
            closest = Some(closest.map_or(gap, |c| c.min(gap)));
            if gap < 15.0 {
                too_close.push((inductor.reference.clone(), victim.reference.clone(), round(gap)));
            }
        }
    }
    let separation = KiCadSeparation {
        inductors: inductors.len(),
        sensitive: sensitive.len(),
        closest_mm: closest.map(round),
        too_close,
    };

    // Assembly.
    let polarised = |part: &Part| {
        matches!(part.prefix(), "D" | "LED" | "U" | "IC" | "Q")
            || (part.prefix() == "C" && part.footprint.to_ascii_lowercase().contains("cp_"))
    };
    let mut by_kind: BTreeMap<(String, bool), BTreeMap<i64, usize>> = BTreeMap::new();
    for part in parts.iter().filter(|part| polarised(part)) {
        let angle = (part.angle.rem_euclid(360.0)).round() as i64 % 360;
        *by_kind.entry((part.prefix().to_string(), part.back)).or_default().entry(angle).or_default() += 1;
    }
    let polarised_count: usize = by_kind.values().flat_map(|angles| angles.values()).sum();
    let modal: usize = by_kind.values().map(|angles| angles.values().copied().max().unwrap_or(0)).sum();
    let connector = |part: &Part| {
        matches!(part.prefix(), "J" | "P" | "CN" | "CON" | "USB" | "X") || part.mouth.is_some()
    };
    let mounting = |part: &Part| part.prefix() == "H" || part.footprint.to_ascii_lowercase().contains("mountinghole");
    let near_edge = match outline {
        Some(board) => parts
            .iter()
            .filter(|part| !part.pads.is_empty() && !connector(part) && !mounting(part))
            .filter(|part| {
                let b = part.body;
                (b[0] - board[0]).min(b[1] - board[1]).min(board[2] - b[2]).min(board[3] - b[3]) < 2.5
            })
            .map(|part| part.reference.clone())
            .collect(),
        None => Vec::new(),
    };
    let assembly = KiCadAssembly {
        polarised: polarised_count,
        orientation_consistency: share(modal, polarised_count),
        near_edge,
    };

    // Connectors: mouth at the nearest edge, pointing out.
    let mut with_mouth = 0;
    let mut facing_out = 0;
    let mut not_facing_out = Vec::new();
    if let Some(board) = outline {
        for part in parts.iter().filter(|part| part.mouth.is_some() && connector(part)) {
            with_mouth += 1;
            let b = part.body;
            let edges = [
                (b[0] - board[0], [-1.0, 0.0]),
                (b[1] - board[1], [0.0, -1.0]),
                (board[2] - b[2], [1.0, 0.0]),
                (board[3] - b[3], [0.0, 1.0]),
            ];
            let (gap, normal) = edges.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
            let mouth = part.mouth.unwrap();
            if gap <= 2.0 && mouth[0] * normal[0] + mouth[1] * normal[1] > 0.7 {
                facing_out += 1;
            } else {
                not_facing_out.push(part.reference.clone());
            }
        }
    }
    let connectors = KiCadConnectors { with_mouth, facing_out, not_facing_out };

    // Manufacturing.
    let mut vias_in_pads = 0;
    for (at, drill) in &vias {
        let inside = parts.iter().flat_map(|part| &part.pads).any(|pad| {
            pad.smd
                && pad.half[0].max(pad.half[1]) < 1.0
                && (at[0] - pad.center[0]).abs() < pad.half[0] + drill / 2.0
                && (at[1] - pad.center[1]).abs() < pad.half[1] + drill / 2.0
        });
        vias_in_pads += inside as usize;
    }
    let mut tombstone_risk = Vec::new();
    for part in parts.iter().filter(|part| part.pads.len() == 2 && part.pads.iter().all(|pad| pad.smd)) {
        let small = ["0201", "0402", "0603", "1005", "1608"].iter().any(|size| part.footprint.contains(size));
        if !small {
            continue;
        }
        let width_into = |pad: &Pad| -> f64 {
            tracks
                .iter()
                .filter(|track| {
                    [track.start, track.end].iter().any(|end| {
                        (end[0] - pad.center[0]).abs() <= pad.half[0] && (end[1] - pad.center[1]).abs() <= pad.half[1]
                    })
                })
                .map(|track| track.width)
                .sum()
        };
        let (a, b) = (width_into(&part.pads[0]), width_into(&part.pads[1]));
        if a > 0.0 && b > 0.0 && a.max(b) > 2.0 * a.min(b) {
            tombstone_risk.push(part.reference.clone());
        }
    }
    let mut drc = BTreeMap::new();
    if let Ok(text) = fs::read_to_string(directory.join("drc.json"))
        && let Ok(report) = serde_json::from_str::<serde_json::Value>(&text)
    {
        for violation in report["violations"].as_array().into_iter().flatten() {
            if let Some(kind) = violation["type"].as_str() {
                *drc.entry(kind.to_string()).or_default() += 1;
            }
        }
        let unconnected = report["unconnected_items"].as_array().map_or(0, Vec::len);
        if unconnected > 0 {
            drc.insert("unconnected_items".into(), unconnected);
        }
    }
    let manufacturing = KiCadManufacturing { vias_in_pads, tombstone_risk, drc };
    // Ground pours by layer, and the other nets' tracks on those layers.
    let mut planes = Vec::new();
    let mut seen = BTreeSet::new();
    for zone in pcb.children().iter().filter(|item| item.head() == Some("zone")) {
        if zone.child("keepout").is_some() {
            continue;
        }
        let Some(net) = form_atom(zone, "net_name", 1).or_else(|| node_net(zone)).map(|raw| normalize_net(raw).to_string()) else {
            continue;
        };
        if !ground(&net) {
            continue;
        }
        let zone_layers: Vec<String> = zone
            .child("layers")
            .map(|layers| layers.children().iter().skip(1).filter_map(Expr::atom).map(str::to_owned).collect())
            .or_else(|| form_atom(zone, "layer", 1).map(|layer| vec![layer.to_string()]))
            .unwrap_or_default();
        for layer in zone_layers {
            if !seen.insert((layer.clone(), net.clone())) {
                continue;
            }
            let cuts: Vec<f64> = tracks
                .iter()
                .filter(|track| track.layer == layer && track.net.as_ref().is_some_and(|other| *other != net))
                .map(|track| distance(track.start, track.end))
                .collect();
            planes.push(KiCadPlane {
                layer,
                net: net.clone(),
                cut_by_mm: round(cuts.iter().sum()),
                longest_cut_mm: round(cuts.iter().copied().fold(0.0, f64::max)),
            });
        }
    }
    let joints = |through: bool| {
        parts
            .iter()
            .flat_map(|part| &part.pads)
            .filter(|pad| pad.net.is_some() && pad.smd != through)
            .count()
    };
    let cost = crate::cost::estimate(
        &crate::cost::CostInputs {
            width_mm: economy.width_mm,
            height_mm: economy.height_mm,
            layers: economy.copper_layers,
            smallest_drill_mm: economy.smallest_via_drill_mm,
            narrowest_track_mm: economy.narrowest_track_mm,
            vias_in_pads: manufacturing.vias_in_pads,
            smd_sides: economy.smd_sides,
            unique_parts: economy.unique_parts,
            smd_joints: joints(false),
            tht_joints: joints(true),
        },
        10,
    );

    Ok(KiCadQualityReport {
        board_id: board_id.into(),
        economy,
        decoupling,
        crystals,
        separation,
        assembly,
        connectors,
        manufacturing,
        planes,
        cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_and_roles_are_read_like_a_designer_writes_them() {
        for (value, farads) in [("100n", 1e-7), ("0.1uF", 1e-7), ("4u7", 4.7e-6), ("10 µF", 1e-5), ("22pF", 2.2e-11)] {
            let got = capacitance(value).unwrap();
            assert!((got - farads).abs() < farads * 1e-9, "{value}: {got}");
        }
        assert!(capacitance("DNP").is_none());
        for net in ["GND", "/AGND", "GNDA", "VSS"] {
            assert!(ground(net), "{net}");
        }
        for net in ["+3V3", "3V3", "/VCC", "VDD_IO", "VBUS", "5V", "+12V", "3V3_DUT", "3.3V"] {
            assert!(rail_name(net), "{net}");
        }
        for net in ["/SDA", "Net-(U1-Pad3)", "LED1", "V_SENSE"] {
            assert!(!rail_name(net), "{net}");
        }
    }
}

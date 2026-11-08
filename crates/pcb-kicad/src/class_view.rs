// Copyright (C) 2026 Toit contributors.

//! The net classes as an agent reads them: per class its width,
//! clearance, nets, the custom rules' widths per layer and the layers
//! they keep it off (`describe-kicad-board`'s `classes`), the classes and
//! rules a fab's impedance control is for (`fab-manifest.json`'s
//! `controlled_impedance`), and the layers a routed board's classes went
//! on (the verdict's `class_layers`).
//!
//! KiCad ties no class to a layer unless a custom rule does: a class
//! whose width is the designer's impedance on one layer (Sisu_SSE-9's
//! RF_ANT_INT, 0.1102 mm, a stripline on In1.Cu) passes every check on
//! an outer layer at that width, where it is another impedance (trial 7:
//! MODEM_ANT1 came back 55 mm on B.Cu).

use super::*;
use crate::project_rules::KiCadProjectRulesReport;

/// The class a net is in as KiCad names it: its highest-priority class
/// (`KiCadProjectRulesReport::net_classes`), "Default" for a net in none.
pub(crate) fn net_class<'a>(report: &'a KiCadProjectRulesReport, net: &str) -> &'a str {
    report
        .net_classes
        .get(net)
        .and_then(|classes| classes.first())
        .map_or("Default", String::as_str)
}

/// One class of the project (`describe-kicad-board`'s `classes`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadClassRow {
    pub class: String,
    /// The class's track width and clearance as the project states them
    /// (the nets' `track_width_mm` is raised to the board's minimums).
    pub track_width_mm: Option<f64>,
    pub clearance_mm: Option<f64>,
    /// The board's nets in the class.
    pub nets: usize,
    /// Per copper layer where a custom `track_width` rule sets the width
    /// of the class's nets on some layers only: that width.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub layer_widths_mm: BTreeMap<String, f64>,
    /// The layers a custom `disallow track` rule keeps its nets off.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub disallowed_layers: Vec<String>,
}

impl KiCadClassRow {
    /// Whether the class looks like a controlled impedance: widths per
    /// layer from the rules, or a class other than Default narrower than
    /// Default (a wider one is likely a current to carry).
    pub(crate) fn impedance(&self, default_width: Option<f64>) -> bool {
        !self.layer_widths_mm.is_empty()
            || (self.class != "Default"
                && matches!(
                    (self.track_width_mm, default_width),
                    (Some(width), Some(default)) if width < default - 1.0e-6
                ))
    }
}

/// The project's classes with their nets among `nets` (as the router
/// names them): Default first, then by name; a class without nets is
/// listed too (the project defines it).
pub(crate) fn class_rows(report: &KiCadProjectRulesReport, nets: &[String]) -> Vec<KiCadClassRow> {
    let mut names: BTreeSet<&str> = report.class_values.keys().map(String::as_str).collect();
    names.insert("Default");
    let mut members: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for net in nets {
        let class = net_class(report, net);
        names.insert(class);
        members.entry(class).or_default().push(net);
    }
    let mut rows: Vec<KiCadClassRow> = names
        .into_iter()
        .map(|class| {
            let nets = members.get(class).cloned().unwrap_or_default();
            let mut layer_widths_mm: BTreeMap<String, f64> = BTreeMap::new();
            let mut disallowed: BTreeSet<String> = BTreeSet::new();
            for net in &nets {
                for (layer, width) in report.layer_track_widths.get(*net).into_iter().flatten() {
                    layer_widths_mm.entry(layer.clone()).or_insert(*width);
                }
                disallowed.extend(
                    report
                        .track_disallowed_layers
                        .get(*net)
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
            }
            let values = report.class_values.get(class).copied().unwrap_or_default();
            KiCadClassRow {
                class: class.to_string(),
                track_width_mm: values[0],
                clearance_mm: values[1],
                nets: nets.len(),
                layer_widths_mm,
                disallowed_layers: copper_order(disallowed),
            }
        })
        .collect();
    rows.sort_by_key(|row| row.class != "Default");
    rows
}

/// Copper layer names in the stack's order (F.Cu, In1.Cu, ..., B.Cu).
fn copper_order(layers: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut layers: Vec<String> = layers.into_iter().collect();
    layers.sort_by_key(|name| layer_rank(name));
    layers
}

fn layer_rank(name: &str) -> (u8, u32) {
    match name {
        "F.Cu" => (0, 0),
        "B.Cu" => (2, 0),
        other => (
            1,
            other
                .trim_start_matches("In")
                .trim_end_matches(".Cu")
                .parse()
                .unwrap_or(u32::MAX),
        ),
    }
}

fn outer(layer: &str) -> bool {
    matches!(layer, "F.Cu" | "B.Cu")
}

/// One class's copper on the written board (the verdict's
/// `class_layers`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadClassLayers {
    pub class: String,
    /// The class's own track width (`None`: the project states none).
    pub track_width_mm: Option<f64>,
    /// The class's nets with tracks on the board.
    pub nets: usize,
    /// Per copper layer with tracks of the class, in the stack's order.
    pub layers: Vec<KiCadClassLayer>,
    /// The layers the designer's board (the reference, else the input
    /// where it has tracks) runs the class on, where it has any of it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub designer_layers: Vec<String>,
    /// The custom rules' widths per layer for the class (on every layer
    /// they name, with its tracks or not).
    #[serde(skip)]
    pub rule_widths: BTreeMap<String, f64>,
    /// Whether the class looks like an impedance without layer rules
    /// (`KiCadClassRow::impedance`), and Default's width.
    #[serde(skip)]
    pub impedance: Option<f64>,
    /// Where the class went on a layer its width is not for: a layer the
    /// rules give it no width on while they give one on others, or both
    /// outer and inner layers for a class narrower than Default without
    /// layer rules (an impedance's width differs between a microstrip and
    /// a stripline), or, for such a class, a layer the designer's board
    /// does not run it on. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadClassLayer {
    pub layer: String,
    /// Its tracks' length there, mm, and the nets with tracks there.
    pub length_mm: f64,
    pub nets: usize,
    /// The width a custom rule sets for the class on this layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_width_mm: Option<f64>,
}

/// The tracks of `pcb` per net (as the router names it) and copper layer:
/// their length, mm (an arc by its two chords through its midpoint).
pub(crate) fn track_lengths(pcb: &Expr) -> BTreeMap<String, BTreeMap<String, f64>> {
    let names = NetNames::of(pcb);
    let point = |item: &Expr, key: &str| -> Option<[f64; 2]> {
        let form = item.child(key)?;
        Some([
            form.children().get(1)?.atom()?.parse().ok()?,
            form.children().get(2)?.atom()?.parse().ok()?,
        ])
    };
    let mut lengths: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for item in pcb.children() {
        let kind = item.head();
        if !matches!(kind, Some("segment" | "arc")) {
            continue;
        }
        let (Some(net), Some(layer)) = (node_net(item), form_atom(item, "layer", 1)) else {
            continue;
        };
        if !layer.ends_with(".Cu") || !names.routable(net) {
            continue;
        }
        let (Some(start), Some(end)) = (point(item, "start"), point(item, "end")) else {
            continue;
        };
        let length = match point(item, "mid").filter(|_| kind == Some("arc")) {
            Some(mid) => {
                pcb_router::geometry::distance(start, mid)
                    + pcb_router::geometry::distance(mid, end)
            }
            None => pcb_router::geometry::distance(start, end),
        };
        *lengths
            .entry(names.normalize(net).to_string())
            .or_default()
            .entry(layer.to_string())
            .or_default() += length;
    }
    lengths
}

/// Per class with tracks on `pcb` (a routed board), the layers they went
/// on, with a warning where a class went on a layer its width is not for
/// (`KiCadClassLayers::warning`).
pub(crate) fn class_layers(report: &KiCadProjectRulesReport, pcb: &Expr) -> Vec<KiCadClassLayers> {
    let lengths = track_lengths(pcb);
    let nets: Vec<String> = lengths.keys().cloned().collect();
    let rows = class_rows(report, &nets);
    let default_width = report
        .class_values
        .get("Default")
        .and_then(|values| values[0]);
    let mut result = Vec::new();
    for row in rows.iter().filter(|row| row.nets > 0) {
        let mut per_layer: BTreeMap<&str, (f64, usize)> = BTreeMap::new();
        for (net, layers) in &lengths {
            if net_class(report, net) != row.class {
                continue;
            }
            for (layer, length) in layers {
                let entry = per_layer.entry(layer.as_str()).or_default();
                entry.0 += length;
                entry.1 += 1;
            }
        }
        let mut layers: Vec<KiCadClassLayer> = per_layer
            .into_iter()
            .map(|(layer, (length, nets))| KiCadClassLayer {
                layer: layer.to_string(),
                length_mm: (length * 1000.0).round() / 1000.0,
                nets,
                rule_width_mm: row.layer_widths_mm.get(layer).copied(),
            })
            .collect();
        layers.sort_by_key(|layer| layer_rank(&layer.layer));
        let mut entry = KiCadClassLayers {
            class: row.class.clone(),
            track_width_mm: row.track_width_mm,
            nets: row.nets,
            layers,
            designer_layers: Vec::new(),
            rule_widths: row.layer_widths_mm.clone(),
            impedance: (row.layer_widths_mm.is_empty() && row.impedance(default_width))
                .then(|| default_width.unwrap_or_default()),
            warning: None,
        };
        entry.warning = warning(&entry);
        result.push(entry);
    }
    result
}

/// `KiCadClassLayers::warning` of `class`: for a class with widths per
/// layer from the rules, the layers it went on that they give no width on
/// (and the designer's board, where known, does not run it on); for a
/// class narrower than Default without layer rules, the layers the
/// designer's board does not run it on, else (no designer's copper) both
/// outer and inner layers.
fn warning(class: &KiCadClassLayers) -> Option<String> {
    let listed = |layers: &[&KiCadClassLayer]| {
        layers
            .iter()
            .map(|layer| format!("{} {:.1} mm", layer.layer, layer.length_mm))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let designer = if class.designer_layers.is_empty() {
        String::new()
    } else {
        format!(
            "; the designer's board runs it on {}",
            class.designer_layers.join(", ")
        )
    };
    let off: Vec<&KiCadClassLayer> = class
        .layers
        .iter()
        .filter(|layer| surely_off(class).contains(&layer.layer.as_str()))
        .collect();
    if !class.rule_widths.is_empty() {
        return (!off.is_empty()).then(|| {
            format!(
                "{} has widths per layer from the custom rules ({}) but went on layers they give it no width on ({}{designer}): a width for one impedance differs by layer",
                class.class,
                class
                    .rule_widths
                    .iter()
                    .map(|(layer, width)| format!("{layer} {width} mm"))
                    .collect::<Vec<_>>()
                    .join(", "),
                listed(&off),
            )
        });
    }
    let default = class.impedance?;
    let what = format!(
        "{} ({} mm, narrower than Default's {} mm: likely a controlled impedance)",
        class.class,
        class.track_width_mm.unwrap_or_default(),
        default
    );
    if !class.designer_layers.is_empty() {
        return (!off.is_empty()).then(|| {
            format!(
                "{what} went on layers the designer's board does not run it on ({}{designer}): an impedance's width differs by layer, and no rule says which layer this width is for",
                listed(&off),
            )
        });
    }
    let (outside, inside): (Vec<&KiCadClassLayer>, Vec<&KiCadClassLayer>) =
        class.layers.iter().partition(|layer| outer(&layer.layer));
    (!outside.is_empty() && !inside.is_empty()).then(|| {
        format!(
            "{what} went on outer layers ({}) and inner ones ({}): an impedance's width differs between a microstrip outside and a stripline inside, and no rule says which layer this width is for",
            listed(&outside),
            listed(&inside),
        )
    })
}

/// The layers `class` went on that are surely not its: those its rules
/// give no width on where they give one on others, else those the
/// designer's board does not run it on.
fn surely_off(class: &KiCadClassLayers) -> Vec<&str> {
    let designer = |layer: &KiCadClassLayer| {
        !class.designer_layers.is_empty() && class.designer_layers.contains(&layer.layer)
    };
    class
        .layers
        .iter()
        .filter(|layer| {
            if !class.rule_widths.is_empty() {
                layer.rule_width_mm.is_none() && !designer(layer)
            } else {
                !class.designer_layers.is_empty() && !designer(layer)
            }
        })
        .map(|layer| layer.layer.as_str())
        .collect()
}

/// Adds to `facts.class_layers` (`route_facts`) the layers the designer's
/// board runs each class on: the first of `boards` (the reference's, then
/// the input's) with tracks of the classes. A class narrower than Default
/// without layer rules (likely a controlled impedance) that went on a
/// layer the designer's board does not run it on gets a warning (trial 7:
/// Sisu's MODEM_ANT1, 0.1102 mm, the designer's stripline on In1.Cu, came
/// back 55 mm on B.Cu alone, which the outer-and-inner rule cannot see).
pub fn note_designer_class_layers(
    facts: &mut crate::KiCadRunFacts,
    result: &crate::KiCadBoardRouterResult,
    boards: &[PathBuf],
) {
    if facts.class_layers.is_empty() {
        return;
    }
    if let Some(designer) = boards.iter().find_map(|path| {
        let text = fs::read_to_string(path).ok()?;
        let lengths = track_lengths(&parse(&text).ok()?);
        (!lengths.is_empty()).then_some(lengths)
    }) {
        add_designer_layers(&result.rules.project, &mut facts.class_layers, &designer);
    }
}

/// A net the board written runs much narrower than the designer's board
/// does (the verdict's `designer_widths`): KiCad's DRC checks no width
/// above the minimum, so a supply drawn wide by hand comes back at its
/// class's width with nothing said (trial 8: the rp2040 motor
/// controller's VMOT, 2 mm on the designer's board, 0.2 mm in route mode;
/// its project's class VMOT, 2 mm, names no net).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadDesignerWidth {
    pub net: String,
    /// The net's class on the board written ("Default" for a net in none).
    pub class: String,
    /// The width of the designer's tracks on the net at half their
    /// length (the length-weighted median), and that length.
    pub designer_width_mm: f64,
    pub designer_length_mm: f64,
    /// The same of the board written.
    pub routed_width_mm: f64,
    pub routed_length_mm: f64,
    /// A class of the project that holds no net and is named as the net
    /// (VMOT for VMOT): the project defines the width but assigns it to
    /// nothing (KiCad assigns classes by the .kicad_pro's
    /// `netclass_patterns` or the schematic's directives), with its width.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unassigned_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unassigned_class_width_mm: Option<f64>,
}

/// Per net of `pcb`, its tracks' length per width (micrometres).
pub(crate) fn track_widths(pcb: &Expr) -> BTreeMap<String, BTreeMap<i64, f64>> {
    let names = NetNames::of(pcb);
    let point = |item: &Expr, key: &str| -> Option<[f64; 2]> {
        let form = item.child(key)?;
        Some([
            form.children().get(1)?.atom()?.parse().ok()?,
            form.children().get(2)?.atom()?.parse().ok()?,
        ])
    };
    let mut widths: BTreeMap<String, BTreeMap<i64, f64>> = BTreeMap::new();
    for item in pcb.children() {
        let kind = item.head();
        if !matches!(kind, Some("segment" | "arc")) {
            continue;
        }
        let (Some(net), Some(layer)) = (node_net(item), form_atom(item, "layer", 1)) else {
            continue;
        };
        if !layer.ends_with(".Cu") || !names.routable(net) {
            continue;
        }
        let Some(width) = form_atom(item, "width", 1).and_then(|width| width.parse::<f64>().ok())
        else {
            continue;
        };
        let (Some(start), Some(end)) = (point(item, "start"), point(item, "end")) else {
            continue;
        };
        let length = match point(item, "mid").filter(|_| kind == Some("arc")) {
            Some(mid) => {
                pcb_router::geometry::distance(start, mid)
                    + pcb_router::geometry::distance(mid, end)
            }
            None => pcb_router::geometry::distance(start, end),
        };
        *widths
            .entry(names.normalize(net).to_string())
            .or_default()
            .entry((width * 1000.0).round() as i64)
            .or_default() += length;
    }
    widths
}

/// The width at half the length of `widths` (micrometres to length), in
/// millimetres, and the length.
fn median_width(widths: &BTreeMap<i64, f64>) -> (f64, f64) {
    let total: f64 = widths.values().sum();
    let mut seen = 0.0;
    for (width, length) in widths {
        seen += length;
        if seen >= total / 2.0 {
            return (*width as f64 / 1000.0, total);
        }
    }
    (0.0, total)
}

/// The nets `routed` runs at under half the width the designer's board
/// (`designer`) gives them, over 3 mm of the designer's tracks at least,
/// the widest first (`KiCadDesignerWidth`).
pub(crate) fn designer_widths(
    report: &KiCadProjectRulesReport,
    designer: &BTreeMap<String, BTreeMap<i64, f64>>,
    routed: &BTreeMap<String, BTreeMap<i64, f64>>,
) -> Vec<KiCadDesignerWidth> {
    let mut members: BTreeMap<&str, usize> = BTreeMap::new();
    for net in report.net_classes.keys() {
        *members.entry(net_class(report, net)).or_default() += 1;
    }
    let mut narrower: Vec<KiCadDesignerWidth> = Vec::new();
    for (net, theirs) in designer {
        let Some(ours) = routed.get(net) else {
            continue;
        };
        let (designer_width, designer_length) = median_width(theirs);
        let (routed_width, routed_length) = median_width(ours);
        if designer_length < 3.0
            || routed_length <= 0.0
            || designer_width < 2.0 * routed_width - 1.0e-6
            || designer_width < routed_width + 0.1
        {
            continue;
        }
        let bare = net.trim_start_matches('/');
        let unassigned = report
            .class_values
            .iter()
            .find(|(class, _)| {
                class.as_str() != "Default"
                    && class.eq_ignore_ascii_case(bare)
                    && !members.contains_key(class.as_str())
            })
            .map(|(class, values)| (class.clone(), values[0]));
        narrower.push(KiCadDesignerWidth {
            net: net.clone(),
            class: net_class(report, net).to_string(),
            designer_width_mm: designer_width,
            designer_length_mm: (designer_length * 10.0).round() / 10.0,
            routed_width_mm: routed_width,
            routed_length_mm: (routed_length * 10.0).round() / 10.0,
            unassigned_class_width_mm: unassigned.as_ref().and_then(|(_, width)| *width),
            unassigned_class: unassigned.map(|(class, _)| class),
        });
    }
    narrower.sort_by(|a, b| {
        b.designer_width_mm
            .total_cmp(&a.designer_width_mm)
            .then(b.designer_length_mm.total_cmp(&a.designer_length_mm))
    });
    narrower
}

/// Adds to `facts.designer_widths` the nets the board written
/// (`written`) runs at under half the designer's width: the first of
/// `boards` (the reference's, then the input's) with tracks.
pub fn note_designer_widths(
    facts: &mut crate::KiCadRunFacts,
    result: &crate::KiCadBoardRouterResult,
    boards: &[PathBuf],
    written: &Path,
) {
    let Some(routed) = fs::read_to_string(written)
        .ok()
        .and_then(|text| parse(&text).ok())
        .map(|pcb| track_widths(&pcb))
    else {
        return;
    };
    if let Some(designer) = boards.iter().find_map(|path| {
        let text = fs::read_to_string(path).ok()?;
        let widths = track_widths(&parse(&text).ok()?);
        (!widths.is_empty()).then_some(widths)
    }) {
        facts.designer_widths = designer_widths(&result.rules.project, &designer, &routed);
    }
}

/// The verdict's next action for the nets run narrower than on the
/// designer's board (`KiCadDesignerWidth`).
pub(crate) fn designer_widths_action(widths: &[KiCadDesignerWidth]) -> Option<String> {
    if widths.is_empty() {
        return None;
    }
    let shown = 6;
    let mm = |value: f64| format!("{}", (value * 1000.0).round() / 1000.0);
    let mut listed: Vec<String> = widths
        .iter()
        .take(shown)
        .map(|width| {
            format!(
                "{} {} mm on the designer's board -> {} mm here (class {})",
                width.net,
                mm(width.designer_width_mm),
                mm(width.routed_width_mm),
                width.class
            )
        })
        .collect();
    if widths.len() > shown {
        listed.push(format!("{} more", widths.len() - shown));
    }
    let example: Vec<String> = widths
        .iter()
        .take(2)
        .map(|width| {
            format!(
                "{{\"name\": \"{}\", \"nets\": [\"{}\"], \"track_width_mm\": {}}}",
                width
                    .unassigned_class
                    .clone()
                    .unwrap_or_else(|| format!("{} width", width.net.trim_start_matches('/'))),
                width.net,
                mm(width
                    .unassigned_class_width_mm
                    .unwrap_or(width.designer_width_mm))
            )
        })
        .collect();
    let unassigned: Vec<String> = widths
        .iter()
        .filter_map(|width| {
            Some(format!(
                "{} ({} mm)",
                width.unassigned_class.as_ref()?,
                mm(width.unassigned_class_width_mm?)
            ))
        })
        .collect();
    let classes = if unassigned.is_empty() {
        String::new()
    } else {
        format!(
            "; the project defines the classes {} but assigns them no net (KiCad assigns classes by the .kicad_pro's netclass_patterns or the schematic's net class directives): assigning them there does the same for KiCad too",
            unassigned.join(", ")
        )
    };
    Some(format!(
        "nets run at under half the width the designer's board gives them, which KiCad's DRC does not check (a width above the minimum is no finding: likely a current to carry; verdict.designer_widths): {}. Give them their width with a class in the router config (\"net_classes\": [{}]; wider tracks take room, so compare the open connections){classes}",
        listed.join(", "),
        example.join(", ")
    ))
}

/// `note_designer_class_layers` with the designer's tracks
/// (`track_lengths` of its board).
fn add_designer_layers(
    report: &KiCadProjectRulesReport,
    classes: &mut [KiCadClassLayers],
    designer: &BTreeMap<String, BTreeMap<String, f64>>,
) {
    for class in classes {
        let mut layers: BTreeSet<String> = BTreeSet::new();
        for (net, on) in designer {
            if net_class(report, net) == class.class {
                layers.extend(on.keys().cloned());
            }
        }
        class.designer_layers = copper_order(layers);
        class.warning = warning(class);
    }
}

/// The verdict's next action for the classes that went on layers their
/// width is not for (`KiCadClassLayers::warning`).
pub(crate) fn class_layers_action(classes: &[KiCadClassLayers]) -> Option<String> {
    let warned: Vec<&KiCadClassLayers> = classes
        .iter()
        .filter(|class| class.warning.is_some())
        .collect();
    // The example: a class whose rules name its layers (the layers
    // without a width are surely not its), else the first.
    let first = warned
        .iter()
        .find(|class| !surely_off(class).is_empty())
        .or(warned.first())?;
    // A layer the class's rules give no width on, or the designer's board
    // does not run it on, is surely not its; for another class only the
    // designer knows which layers its width is for.
    let layer = surely_off(first).first().copied().unwrap_or("<layer>");
    Some(format!(
        "classes went on layers their width is not for, which KiCad's DRC does not check (verdict.class_layers): {}. Keep such a class on the layers its width is for with a custom rule in the board's .kicad_dru, e.g. (rule \"{} layers\" (condition \"A.hasNetclass('{}') && A.Layer == '{}'\") (constraint disallow track)) for each layer it must not use (a new file starts with (version 1)), or give it a track_width rule per layer (a condition on A.Layer) for each layer it may use",
        warned
            .iter()
            .filter_map(|class| class.warning.as_deref())
            .collect::<Vec<_>>()
            .join("; "),
        first.class,
        first.class,
        layer,
    ))
}

/// What a fab's impedance control is for (`fab-manifest.json`'s
/// `controlled_impedance`): the classes that look like an impedance
/// (`KiCadClassRow::impedance`) and the custom rules' track geometry on
/// some layers only.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadControlledImpedance {
    pub classes: Vec<KiCadClassRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<crate::project_rules::KiCadLayerGeometryRule>,
    pub action: String,
}

/// `KiCadControlledImpedance` of a project's rules over the board's nets
/// (as the router names them); `None` where nothing asks for it.
pub(crate) fn controlled_impedance(
    report: &KiCadProjectRulesReport,
    nets: &[String],
) -> Option<KiCadControlledImpedance> {
    let default_width = report
        .class_values
        .get("Default")
        .and_then(|values| values[0]);
    let classes: Vec<KiCadClassRow> = class_rows(report, nets)
        .into_iter()
        .filter(|row| row.nets > 0 && row.impedance(default_width))
        .collect();
    if classes.is_empty() && report.layer_geometry_rules.is_empty() {
        return None;
    }
    let names: Vec<&str> = classes.iter().map(|row| row.class.as_str()).collect();
    Some(KiCadControlledImpedance {
        action: format!(
            "order impedance control on this stackup (fab-manifest.json's stackup, the job file's) for {}: the widths are the designer's for its dielectrics, and the fab adjusts them to its own",
            if names.is_empty() {
                "the custom rules' track geometry per layer".to_string()
            } else {
                format!("the classes {}", names.join(", "))
            }
        ),
        classes,
        rules: report.layer_geometry_rules.clone(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A four-layer project: Default 0.2 mm, USB 0.1 mm with a 0.1 mm
    /// width rule on In1.Cu and a `disallow track` on In2.Cu, RF 0.12 mm
    /// (narrower than Default, no layer rule), PWR 0.5 mm; USB_DP routed on
    /// In1.Cu and F.Cu, RF_OUT on F.Cu and In2.Cu, VBAT on F.Cu and B.Cu,
    /// SDA on F.Cu. Returns the directory and the project's path.
    pub(crate) fn impedance_project(tag: &str) -> (PathBuf, PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-class-view-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("board.kicad_pcb"),
            r#"(kicad_pcb (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
  (setup (stackup
    (layer "F.Mask" (type "Top Solder Mask") (thickness 0.01))
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.0918) (material "3313") (epsilon_r 4.1) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.0152))
    (layer "dielectric 2" (type "core") (thickness 1.2) (material "FR4") (epsilon_r 4.5))
    (layer "In2.Cu" (type "copper") (thickness 0.0152))
    (layer "dielectric 3" (type "prepreg") (thickness 0.0918) (material "3313") (epsilon_r 4.1))
    (layer "B.Cu" (type "copper") (thickness 0.035))
    (copper_finish "ENIG") (dielectric_constraints no)))
  (footprint "a" (at 0 0)
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "USB_DP"))
    (pad "2" smd rect (at 0 5) (size 1 1) (layers "F.Cu") (net "USB_DP"))
    (pad "3" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "RF_OUT"))
    (pad "4" smd rect (at 2 5) (size 1 1) (layers "F.Cu") (net "RF_OUT"))
    (pad "5" smd rect (at 4 0) (size 1 1) (layers "F.Cu") (net "VBAT"))
    (pad "6" smd rect (at 4 5) (size 1 1) (layers "F.Cu") (net "VBAT"))
    (pad "7" smd rect (at 6 0) (size 1 1) (layers "F.Cu") (net "SDA"))
    (pad "8" smd rect (at 6 5) (size 1 1) (layers "F.Cu") (net "SDA")))
  (segment (start 0 0) (end 0 1) (width 0.1) (layer "F.Cu") (net "USB_DP"))
  (segment (start 0 1) (end 0 5) (width 0.1) (layer "In1.Cu") (net "USB_DP"))
  (segment (start 2 0) (end 2 2) (width 0.12) (layer "F.Cu") (net "RF_OUT"))
  (arc (start 2 2) (mid 2.5 3) (end 2 4) (width 0.12) (layer "In2.Cu") (net "RF_OUT"))
  (segment (start 4 0) (end 4 3) (width 0.5) (layer "F.Cu") (net "VBAT"))
  (segment (start 4 3) (end 4 5) (width 0.5) (layer "B.Cu") (net "VBAT"))
  (segment (start 6 0) (end 6 5) (width 0.2) (layer "F.Cu") (net "SDA")))"#,
        )
        .unwrap();
        let project = directory.join("board.kicad_pro");
        fs::write(
            &project,
            r#"{"board": {"design_settings": {"rules": {"min_clearance": 0.1, "min_track_width": 0.1}}},
                "net_settings": {"classes": [
                    {"name": "Default", "clearance": 0.2, "track_width": 0.2, "via_diameter": 0.6, "via_drill": 0.3},
                    {"name": "USB", "clearance": 0.15, "track_width": 0.1},
                    {"name": "RF", "clearance": 0.2, "track_width": 0.12},
                    {"name": "PWR", "clearance": 0.2, "track_width": 0.5}],
                  "netclass_patterns": [{"netclass": "USB", "pattern": "USB_*"}, {"netclass": "RF", "pattern": "RF_*"}, {"netclass": "PWR", "pattern": "VBAT"}]}}"#,
        )
        .unwrap();
        fs::write(
            directory.join("board.kicad_dru"),
            r#"(version 1)
(rule "USB 90 ohm on L2"
  (condition "A.hasNetclass('USB') && A.Layer == 'In1.Cu'")
  (constraint track_width (min 0.1mm) (opt 0.1mm) (max 0.1mm))
  (constraint diff_pair_gap (min 0.11mm) (opt 0.12mm) (max 0.13mm)))
(rule "Keep USB off L3"
  (condition "A.hasNetclass('USB') && A.Layer == 'In2.Cu'")
  (constraint disallow track))"#,
        )
        .unwrap();
        (directory, project)
    }

    fn report_of(directory: &Path, project: &Path) -> KiCadProjectRulesReport {
        crate::resolve_project_rules(project, &directory.join("board.kicad_pcb"))
            .unwrap()
            .project_rules
            .unwrap()
    }

    #[test]
    fn the_rules_report_holds_the_widths_per_layer_and_the_closed_layers() {
        let (directory, project) = impedance_project("report");
        let report = report_of(&directory, &project);
        assert_eq!(
            report.layer_track_widths["USB_DP"],
            BTreeMap::from([("In1.Cu".to_string(), 0.1)])
        );
        assert_eq!(report.track_disallowed_layers["USB_DP"], ["In2.Cu"]);
        let rules: Vec<(&str, &[String])> = report
            .layer_geometry_rules
            .iter()
            .map(|rule| (rule.constraint.as_str(), rule.layers.as_slice()))
            .collect();
        assert_eq!(
            rules,
            [
                ("track_width", &["In1.Cu".to_string()][..]),
                ("diff_pair_gap", &["In1.Cu".to_string()][..])
            ]
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    /// A supply the designer drew 2 mm wide comes back at its class's
    /// 0.2 mm: named, with the project's class of its name that holds no
    /// net (trial 8: the rp2040 motor controller's VMOT).
    #[test]
    fn nets_run_narrower_than_the_designers_are_named() {
        let (directory, project) = impedance_project("widths");
        fs::write(
            &project,
            r#"{"board": {"design_settings": {"rules": {"min_clearance": 0.1, "min_track_width": 0.1}}},
                "net_settings": {"classes": [
                    {"name": "Default", "clearance": 0.2, "track_width": 0.2},
                    {"name": "VBAT", "clearance": 0.2, "track_width": 2.0},
                    {"name": "USB", "clearance": 0.15, "track_width": 0.1}],
                  "netclass_patterns": [{"netclass": "USB", "pattern": "USB_*"}]}}"#,
        )
        .unwrap();
        let report = report_of(&directory, &project);
        let board = |vbat: f64, sda: f64| {
            parse(&format!(
                r#"(kicad_pcb
  (segment (start 0 0) (end 10 0) (width {vbat}) (layer "F.Cu") (net "VBAT"))
  (segment (start 10 0) (end 11 0) (width 0.2) (layer "F.Cu") (net "VBAT"))
  (segment (start 0 5) (end 10 5) (width {sda}) (layer "F.Cu") (net "SDA")))"#
            ))
            .unwrap()
        };
        let designer = track_widths(&board(2.0, 0.25));
        assert_eq!(designer["VBAT"][&2000], 10.0);
        let widths = designer_widths(&report, &designer, &track_widths(&board(0.2, 0.2)));
        assert_eq!(widths.len(), 1, "{widths:?}");
        assert_eq!(widths[0].net, "VBAT");
        assert_eq!(widths[0].class, "Default");
        assert_eq!((widths[0].designer_width_mm, widths[0].routed_width_mm), (2.0, 0.2));
        assert_eq!(widths[0].unassigned_class.as_deref(), Some("VBAT"));
        let action = designer_widths_action(&widths).unwrap();
        assert!(
            action.contains("VBAT 2 mm on the designer's board -> 0.2 mm here (class Default)")
                && action.contains(r#""net_classes": [{"name": "VBAT", "nets": ["VBAT"], "track_width_mm": 2}]"#)
                && action.contains("the project defines the classes VBAT (2 mm) but assigns them no net"),
            "{action}"
        );
        // Kept at the designer's width: nothing to say.
        assert!(designer_widths(&report, &designer, &designer).is_empty());
        assert!(designer_widths_action(&[]).is_none());
        fs::remove_dir_all(&directory).unwrap();
    }

    /// A minimum width on the outer layers alone (the 4in1-mini's "2 oz
    /// outer copper, track width", 0.16 mm) is the process's floor there,
    /// not a width a class is for: no widths per layer, and no warning
    /// for Default on the inner layers (trial 8: the verdict told the
    /// agent to keep every net off In1.Cu to In4.Cu).
    #[test]
    fn a_minimum_width_on_some_layers_is_no_width_per_layer() {
        let (directory, project) = impedance_project("floor");
        fs::write(
            directory.join("board.kicad_dru"),
            r#"(version 1)
(rule "2 oz outer copper, track width"
  (constraint track_width (min 0.16mm))
  (layer outer))"#,
        )
        .unwrap();
        let report = report_of(&directory, &project);
        assert!(report.layer_track_widths.is_empty(), "{:?}", report.layer_track_widths);
        let text = fs::read_to_string(directory.join("board.kicad_pcb")).unwrap();
        let classes = class_layers(&report, &parse(&text).unwrap());
        let default = classes.iter().find(|class| class.class == "Default").unwrap();
        assert_eq!(default.warning, None);
        // Nor an impedance to order on the release.
        assert!(report.layer_geometry_rules.is_empty());
        assert!(controlled_impedance(&report, &["SDA".to_string()]).is_none());
        // A maximum on those layers is a width aimed at: kept.
        fs::write(
            directory.join("board.kicad_dru"),
            r#"(version 1)
(rule "RF: 50 ohm width on the outer layers"
  (condition "A.hasNetclass('RF')")
  (constraint track_width (min 0.1mm) (max 0.11mm))
  (layer outer))"#,
        )
        .unwrap();
        let report = report_of(&directory, &project);
        assert_eq!(
            report.layer_track_widths["RF_OUT"],
            BTreeMap::from([("B.Cu".to_string(), 0.1), ("F.Cu".to_string(), 0.1)])
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_class_on_a_layer_its_width_is_not_for_is_warned() {
        let (directory, project) = impedance_project("layers");
        let report = report_of(&directory, &project);
        let text = fs::read_to_string(directory.join("board.kicad_pcb")).unwrap();
        let classes = class_layers(&report, &parse(&text).unwrap());
        let by_name = |name: &str| classes.iter().find(|class| class.class == name).unwrap();
        // USB: its rule's width on In1.Cu, none on F.Cu.
        let usb = by_name("USB");
        let layers: Vec<(&str, f64, Option<f64>)> = usb
            .layers
            .iter()
            .map(|layer| (layer.layer.as_str(), layer.length_mm, layer.rule_width_mm))
            .collect();
        assert_eq!(layers, [("F.Cu", 1.0, None), ("In1.Cu", 4.0, Some(0.1))]);
        assert!(
            usb.warning
                .as_deref()
                .unwrap()
                .contains("went on layers they give it no width on (F.Cu 1.0 mm)")
        );
        // RF: narrower than Default, outside and inside (the arc by its
        // chords).
        let rf = by_name("RF");
        assert_eq!(rf.layers[1].layer, "In2.Cu");
        assert!((rf.layers[1].length_mm - 2.0 * 1.25f64.sqrt()).abs() < 1.0e-3);
        assert!(
            rf.warning
                .as_deref()
                .unwrap()
                .starts_with("RF (0.12 mm, narrower than Default's 0.2 mm: likely a controlled impedance) went on outer layers (F.Cu 2.0 mm) and inner ones (In2.Cu 2.2 mm)")
        );
        // A power class on both sides and Default: nothing to say.
        assert_eq!(by_name("PWR").warning, None);
        assert_eq!(by_name("Default").warning, None);
        let action = class_layers_action(&classes).unwrap();
        assert!(action.contains("(rule \"USB layers\" (condition \"A.hasNetclass('USB') && A.Layer == 'F.Cu'\") (constraint disallow track))"));
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_class_on_a_layer_the_designers_board_does_not_run_it_on_is_warned() {
        let (directory, project) = impedance_project("designer");
        let report = report_of(&directory, &project);
        let text = fs::read_to_string(directory.join("board.kicad_pcb")).unwrap();
        // The route: RF on B.Cu alone (trial 7's MODEM_ANT1), which the
        // outer-and-inner rule does not see.
        let routed = text
            .replace(
                r#"(segment (start 2 0) (end 2 2) (width 0.12) (layer "F.Cu") (net "RF_OUT"))"#,
                r#"(segment (start 2 0) (end 2 2) (width 0.12) (layer "B.Cu") (net "RF_OUT"))"#,
            )
            .replace(
                r#"(arc (start 2 2) (mid 2.5 3) (end 2 4) (width 0.12) (layer "In2.Cu") (net "RF_OUT"))"#,
                "",
            );
        let mut classes = class_layers(&report, &parse(&routed).unwrap());
        let rf = |classes: &[KiCadClassLayers]| {
            classes
                .iter()
                .find(|class| class.class == "RF")
                .unwrap()
                .clone()
        };
        assert_eq!(rf(&classes).warning, None);
        // The designer's board runs it on F.Cu and In2.Cu.
        add_designer_layers(
            &report,
            &mut classes,
            &track_lengths(&parse(&text).unwrap()),
        );
        let rf = rf(&classes);
        assert_eq!(rf.designer_layers, ["F.Cu", "In2.Cu"]);
        assert_eq!(
            rf.warning.as_deref(),
            Some(
                "RF (0.12 mm, narrower than Default's 0.2 mm: likely a controlled impedance) went on layers the designer's board does not run it on (B.Cu 2.0 mm; the designer's board runs it on F.Cu, In2.Cu): an impedance's width differs by layer, and no rule says which layer this width is for"
            )
        );
        // USB on F.Cu, where its rules give no width, as the designer's
        // board runs it: nothing either.
        let usb = classes.iter().find(|class| class.class == "USB").unwrap();
        assert_eq!(usb.warning, None);
        // The power class went where the designer's did: nothing.
        let power = classes.iter().find(|class| class.class == "PWR").unwrap();
        assert_eq!((power.designer_layers.len(), &power.warning), (2, &None));
        let action = class_layers_action(&classes).unwrap();
        assert!(
            action.contains(
                "(rule \"RF layers\" (condition \"A.hasNetclass('RF') && A.Layer == 'B.Cu'\")"
            ),
            "{action}"
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn the_impedance_classes_are_those_with_widths_per_layer_or_narrower_than_default() {
        let (directory, project) = impedance_project("impedance");
        let report = report_of(&directory, &project);
        let nets: Vec<String> = ["RF_OUT", "SDA", "USB_DP", "VBAT"]
            .map(String::from)
            .to_vec();
        let impedance = controlled_impedance(&report, &nets).unwrap();
        let classes: Vec<&str> = impedance
            .classes
            .iter()
            .map(|row| row.class.as_str())
            .collect();
        assert_eq!(classes, ["RF", "USB"]);
        assert_eq!(impedance.rules.len(), 2);
        assert!(
            impedance
                .action
                .starts_with("order impedance control on this stackup")
        );
        // Without such classes or rules, nothing.
        let plain = KiCadProjectRulesReport::default();
        assert_eq!(controlled_impedance(&plain, &nets), None);
        fs::remove_dir_all(&directory).unwrap();
    }
}

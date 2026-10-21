// Copyright (C) 2026 Toit contributors.

//! Routing rules resolved from the KiCad project file, so a board can be
//! routed without any hand-written or helper-generated configuration.

use super::*;

mod dru;

pub use dru::KiCadCustomRuleReport;

/// KiCad's built-in defaults for a project without net settings.
const DEFAULT_CLEARANCE: f64 = 0.2;
const DEFAULT_TRACK: f64 = 0.2;
const DEFAULT_VIA: f64 = 0.6;
const DEFAULT_DRILL: f64 = 0.3;

/// KiCad's board setup minimums where the project does not state them
/// (board_design_settings.cpp, 10.0.7; KiCad's DRC of a project without
/// the key reports e.g. "board setup constraints min width 0.2000 mm").
const KICAD_MINIMUMS: [(&str, f64); 9] = [
    ("min_clearance", 0.0),
    ("min_connection", 0.0),
    ("min_track_width", 0.2),
    ("min_via_annular_width", 0.1),
    ("min_via_diameter", 0.5),
    ("min_through_hole_diameter", 0.3),
    ("min_hole_to_hole", 0.25),
    ("min_hole_clearance", 0.25),
    ("min_copper_edge_clearance", 0.5),
];

/// What the project's rules gave the router beyond its net classes, for
/// `board-router.json`'s `rules`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KiCadProjectRulesReport {
    /// Board setup minimums the project does not state, taken at KiCad's
    /// defaults (`min_track_width 0.2 mm`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub kicad_defaults: Vec<String>,
    /// The custom rules file read (`<project>.kicad_dru`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_rules_file: Option<String>,
    /// Why the custom rules file could not be read, when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_rules_error: Option<String>,
    /// Every constraint of every custom rule and what the router made of
    /// it (`dru::KiCadCustomRuleReport::status`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom_rules: Vec<KiCadCustomRuleReport>,
    /// The project's routing rules the router does not keep, one line
    /// each; KiCad still checks them, so their findings are left to the
    /// designer (the verdict's next_actions name them).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmodelled: Vec<String>,
}

pub(crate) fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(position) = star {
            p = position + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// A net class pattern of the project as KiCad matches it
/// (`EDA_COMBINED_MATCHER` for net classes): an anchored regular
/// expression or an anchored wildcard, either one (`.*(_TAP|V_POE).*`
/// is a regular expression, `/GPIO*` a wildcard, `+3V3` only the latter).
struct NetClassPattern {
    text: String,
    regex: Option<regex_lite::Regex>,
}

impl NetClassPattern {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            regex: regex_lite::Regex::new(&format!("^(?:{text})$")).ok(),
        }
    }

    fn matches(&self, net: &str) -> bool {
        wildcard_match(&self.text, net)
            || self.regex.as_ref().is_some_and(|regex| regex.is_match(net))
    }
}

/// The most common value, in micrometres, of a list of dimensions.
fn most_common(values: &[f64]) -> Option<f64> {
    let mut counts = BTreeMap::<i64, usize>::new();
    for value in values {
        *counts.entry((value * 1000.0).round() as i64).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(value, count)| (*count, -*value))
        .map(|(value, _)| value as f64 / 1000.0)
}

/// A net class of the project; KiCad 8 and later leave out what the
/// class takes from `Default`.
struct ClassSpec {
    name: String,
    /// KiCad's priority (lower first); the class's place in the list
    /// where the project has none.
    priority: i64,
    track: Option<f64>,
    clearance: Option<f64>,
    via: Option<f64>,
    drill: Option<f64>,
}

/// The copper layer names of a board's `(layers ...)`, in the router's
/// order (F.Cu, In1.Cu, In2.Cu, ..., B.Cu: the lowering's `LayerTable`),
/// so that a layer's index is the router's.
fn copper_layer_names(pcb: &Expr) -> Vec<String> {
    let mut names: Vec<String> = pcb
        .child("layers")
        .map(|layers| {
            layers
                .children()
                .iter()
                .filter_map(|layer| layer.children().get(1).and_then(Expr::atom))
                .filter(|name| name.ends_with(".Cu"))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    names.sort_by_key(|name| match name.as_str() {
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
    });
    names
}

/// Resolves per-connection rules for every net on `pcb` from the project
/// file next to it. Classes come from `net_settings`; a net takes every
/// class assigned to it (explicitly, by a KiCad 6/7 class's net list, by
/// every pattern it matches) and, as KiCad's composite classes, each
/// value from the first of them by priority that has it, else from
/// `Default`. Every value is floored at the board's design-rule minimum
/// (KiCad's default where the project states none). A project without
/// net settings takes its default geometry from the board's existing
/// copper. The project's custom rules (`<project>.kicad_dru`) raise the
/// clearances, widths, vias and the board's edge, hole and hole-to-hole
/// clearances where they apply (`dru`); `project_rules` reports them.
pub fn resolve_project_rules(
    project_path: &Path,
    pcb_path: &Path,
) -> Result<KiCadBoardRouterConfig, String> {
    let project: serde_json::Value = match fs::read_to_string(project_path) {
        Ok(source) => serde_json::from_str(&source)
            .map_err(|error| format!("failed to parse {}: {error}", project_path.display()))?,
        Err(_) => serde_json::Value::Null,
    };
    let pcb_source = fs::read_to_string(pcb_path)
        .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
    let pcb = parse(&pcb_source)?;
    let mut report = KiCadProjectRulesReport::default();

    let rules = &project["board"]["design_settings"]["rules"];
    let kicad_minimum = |key: &str| {
        KICAD_MINIMUMS
            .iter()
            .find(|(name, _)| *name == key)
            .map_or(0.0, |(_, value)| *value)
    };
    let minimum = |key: &str| rules[key].as_f64().unwrap_or_else(|| kicad_minimum(key));
    for (key, value) in KICAD_MINIMUMS {
        if value > 0.0 && rules[key].as_f64().is_none() {
            report.kicad_defaults.push(format!("{key} {value} mm"));
        }
    }
    let severity = project["board"]["design_settings"]["rule_severities"]["connection_width"]
        .as_str()
        .map(str::to_owned);
    let floor =
        connection_width_floor(minimum("min_connection"), severity.as_deref(), floor_mode());

    let mut widths = Vec::new();
    let mut via_sizes = Vec::new();
    let mut via_drills = Vec::new();
    for item in pcb.children() {
        match item.head() {
            Some("segment") => widths.extend(form_f64(item, "width", 1).ok()),
            Some("via") => {
                via_sizes.extend(form_f64(item, "size", 1).ok());
                via_drills.extend(form_f64(item, "drill", 1).ok());
            }
            _ => {}
        }
    }

    let mut specs = Vec::<ClassSpec>::new();
    for (index, class) in project["net_settings"]["classes"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(name) = class["name"].as_str() else {
            continue;
        };
        let value = |key: &str| class[key].as_f64().filter(|v| *v > 0.0);
        specs.push(ClassSpec {
            name: name.to_string(),
            priority: class["priority"].as_i64().unwrap_or(index as i64),
            track: value("track_width"),
            clearance: value("clearance"),
            via: value("via_diameter"),
            drill: value("via_drill"),
        });
    }
    let default = match specs.iter().find(|spec| spec.name == "Default") {
        Some(spec) => KiCadConnectionRoutingRules {
            trace_width_mm: spec.track.unwrap_or(DEFAULT_TRACK),
            clearance_mm: spec.clearance.unwrap_or(DEFAULT_CLEARANCE),
            via_size_mm: spec.via.unwrap_or(DEFAULT_VIA),
            via_drill_mm: spec.drill.unwrap_or(DEFAULT_DRILL),
        },
        None => KiCadConnectionRoutingRules {
            trace_width_mm: most_common(&widths).unwrap_or(DEFAULT_TRACK),
            clearance_mm: DEFAULT_CLEARANCE,
            via_size_mm: most_common(&via_sizes).unwrap_or(DEFAULT_VIA),
            via_drill_mm: most_common(&via_drills).unwrap_or(DEFAULT_DRILL),
        },
    };
    let floored = |mut class: KiCadConnectionRoutingRules| {
        class.trace_width_mm = class
            .trace_width_mm
            .max(minimum("min_track_width"))
            .max(floor.unwrap_or(0.0));
        class.clearance_mm = class.clearance_mm.max(minimum("min_clearance"));
        class.via_size_mm = class.via_size_mm.max(minimum("min_via_diameter"));
        class.via_drill_mm = class.via_drill_mm.max(minimum("min_through_hole_diameter"));
        let annulus = minimum("min_via_annular_width").max(0.05);
        class.via_size_mm = class.via_size_mm.max(class.via_drill_mm + 2.0 * annulus);
        class
    };
    // The values of a net in `classes` (names of `specs`, any order).
    let resolve = |classes: &[&ClassSpec]| {
        let mut ordered: Vec<&ClassSpec> = classes.to_vec();
        ordered.sort_by_key(|spec| spec.priority);
        let first =
            |field: fn(&ClassSpec) -> Option<f64>| ordered.iter().find_map(|spec| field(spec));
        floored(KiCadConnectionRoutingRules {
            trace_width_mm: first(|spec| spec.track).unwrap_or(default.trace_width_mm),
            clearance_mm: first(|spec| spec.clearance).unwrap_or(default.clearance_mm),
            via_size_mm: first(|spec| spec.via).unwrap_or(default.via_size_mm),
            via_drill_mm: first(|spec| spec.drill).unwrap_or(default.via_drill_mm),
        })
    };
    let default_rules = floored(default.clone());

    let assignments = &project["net_settings"]["netclass_assignments"];
    let patterns: Vec<(NetClassPattern, String)> = project["net_settings"]["netclass_patterns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                NetClassPattern::new(entry["pattern"].as_str()?),
                entry["netclass"].as_str()?.to_string(),
            ))
        })
        .collect();

    // KiCad 6 and 7 list a class's nets in the class itself.
    let listed: BTreeMap<String, String> = project["net_settings"]["classes"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|class| {
            let name = class["name"].as_str().unwrap_or("Default").to_string();
            class["nets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|net| net.as_str().map(|net| (net.to_string(), name.clone())))
                .collect::<Vec<_>>()
        })
        .collect();
    let mut nets = BTreeSet::<String>::new();
    let mut pad_kinds = BTreeSet::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            pad_kinds.insert(match pad.children().get(2).and_then(Expr::atom) {
                Some("thru_hole") => dru::Kind::PlatedPad,
                Some("np_thru_hole") => dru::Kind::NpthPad,
                _ => dru::Kind::SmdPad,
            });
            if let Some(net) = node_net(pad) {
                nets.insert(net.to_string());
            }
        }
    }
    let mut connection_rules = BTreeMap::new();
    let mut net_facts = Vec::new();
    for raw in nets {
        let mut names: Vec<String> = Vec::new();
        let mut add = |name: &str| {
            if !names.iter().any(|known| known == name) {
                names.push(name.to_string());
            }
        };
        match &assignments[raw.as_str()] {
            serde_json::Value::String(name) => add(name),
            serde_json::Value::Array(entries) => {
                for name in entries.iter().filter_map(serde_json::Value::as_str) {
                    add(name);
                }
            }
            _ => {}
        }
        if let Some(name) = listed.get(&raw).or_else(|| listed.get(normalize_net(&raw))) {
            add(name);
        }
        for (pattern, name) in &patterns {
            if pattern.matches(&raw) {
                add(name);
            }
        }
        let mut classes: Vec<&ClassSpec> = names
            .iter()
            .filter_map(|name| specs.iter().find(|spec| &spec.name == name))
            .collect();
        classes.sort_by_key(|spec| spec.priority);
        let rules = if classes.is_empty() {
            default_rules.clone()
        } else {
            resolve(&classes)
        };
        net_facts.push(dru::NetFacts {
            name: raw.clone(),
            classes: if classes.is_empty() {
                vec!["Default".to_string()]
            } else {
                classes.iter().map(|spec| spec.name.clone()).collect()
            },
        });
        connection_rules.insert(normalize_net(&raw).to_string(), rules);
    }
    let mut default_rules = default_rules;
    // The narrowest width of the project's classes, used or not.
    let narrowest = specs
        .iter()
        .map(|spec| resolve(&[spec]).trace_width_mm)
        .chain(std::iter::once(default_rules.trace_width_mm))
        .fold(f64::INFINITY, f64::min);
    // The smallest width of the project's width menu, if it allows any
    // narrower than a class.
    let narrow_signal = project["board"]["design_settings"]["track_widths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_f64())
        .filter(|width| *width > 0.0 && *width >= minimum("min_track_width"))
        .fold(f64::INFINITY, f64::min);
    let mut edge_clearance = minimum("min_copper_edge_clearance");
    let mut hole_clearance = minimum("min_hole_clearance");
    let mut hole_to_hole = minimum("min_hole_to_hole");
    let mut neck = if minimum("min_track_width") > 0.0 {
        minimum("min_track_width").max(0.1)
    } else {
        // (`narrowest` is a fold of `f64::min` from infinity: never NaN.)
        narrowest.clamp(0.1, 0.127)
    };
    let mut narrow_signal = narrow_signal.is_finite().then_some(narrow_signal);
    let severities: BTreeMap<String, String> =
        project["board"]["design_settings"]["rule_severities"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
            .collect();
    // The router's rules beyond the classes (`pcb_router::NetRules`).
    let mut net_rules = pcb_router::NetRules::default();
    // KiCad's hole_to_hole holds between the holes of one net too (a via
    // beside a pad hole of its own net: nuclone's VSS); the router keeps
    // it where the project rates the findings errors.
    if dru::finding_severity("hole_to_hole", &severities) == "error" {
        net_rules.same_net_hole_to_hole = hole_to_hole;
    }

    // The project's custom rules.
    let dru_path = project_path.with_extension("kicad_dru");
    if let Ok(text) = fs::read_to_string(&dru_path) {
        report.custom_rules_file = dru_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        match dru::parse_rules(&text) {
            Err(error) => {
                report.unmodelled.push(format!(
                    "the custom rules file does not parse ({error}): none of its rules is kept"
                ));
                report.custom_rules_error = Some(error);
            }
            Ok(rules) => {
                let current = net_facts
                    .iter()
                    .map(|net| connection_rules[normalize_net(&net.name)].clone())
                    .collect();
                let board = dru::BoardFacts {
                    nets: net_facts,
                    copper_layers: copper_layer_names(&pcb),
                    pad_kinds,
                    current,
                    board_clearances: [edge_clearance, hole_clearance, hole_to_hole],
                    neck_width: neck,
                };
                let lowered = dru::lower(&rules, &board, &severities);
                for (index, net) in board.nets.iter().enumerate() {
                    if let Some(rules) = connection_rules.get_mut(normalize_net(&net.name)) {
                        apply_custom(rules, &lowered, Some(index));
                    }
                }
                apply_custom(&mut default_rules, &lowered, None);
                edge_clearance = edge_clearance.max(lowered.edge_clearance);
                hole_clearance = hole_clearance.max(lowered.hole_clearance);
                hole_to_hole = hole_to_hole.max(lowered.hole_to_hole);
                neck = neck.max(lowered.width_min_all);
                narrow_signal = narrow_signal.map(|width| width.max(lowered.width_min_all));
                // A net's own minimum floors its neck-downs and its narrow
                // width (the classes are raised by `apply_custom`).
                for (index, net) in board.nets.iter().enumerate() {
                    let floor = lowered.width_min[index].max(lowered.width_min_all);
                    if floor > neck + 1.0e-9 {
                        net_rules
                            .width_floor
                            .insert(normalize_net(&net.name).to_string(), floor);
                    }
                }
                let names = |nets: &Option<Vec<usize>>| {
                    nets.as_ref().map(|nets| {
                        nets.iter()
                            .map(|&index| normalize_net(&board.nets[index].name).to_string())
                            .collect::<Vec<_>>()
                    })
                };
                for pair in &lowered.pair_clearances {
                    net_rules.clearances.push(pcb_router::ClearanceRule {
                        first: names(&pair.first),
                        second: names(&pair.second),
                        kinds: pair.kinds,
                        layers: pair.layers,
                        clearance: pair.clearance,
                    });
                }
                report.custom_rules = lowered.report;
                report.unmodelled.extend(lowered.unmodelled);
            }
        }
    }

    let mut config = KiCadBoardRouterConfig {
        narrow_signal_mm: narrow_signal,
        relaxed_clearance_mm:
            match project["board"]["design_settings"]["rule_severities"]["clearance"].as_str() {
                Some("warning" | "ignore") if minimum("min_clearance") > 0.0 => {
                    Some(minimum("min_clearance"))
                }
                _ => None,
            },
        // Designers neck down to the board minimum where a pad demands it
        // (KiCad's 0.2 mm where the project states none); a project whose
        // minimum is 0 gets a fab's usual 0.127 mm (KiCad's DRC checks
        // widths against the stated minimum only).
        neck_width_mm: Some(neck),
        connection_rules,
        default_rules: Some(default_rules),
        // KiCad's built-in board setup applies when the project is silent.
        edge_clearance_mm: edge_clearance,
        hole_clearance_mm: hole_clearance,
        hole_to_hole_clearance_mm: hole_to_hole,
        solder_mask_to_copper_mm: rules["solder_mask_to_copper_clearance"].as_f64(),
        solder_mask_bridge_severity:
            project["board"]["design_settings"]["rule_severities"]["solder_mask_bridge"]
                .as_str()
                .map(str::to_owned),
        min_connection_mm: floor,
        connection_width_severity: (minimum("min_connection") > 0.0)
            .then(|| severity.unwrap_or_else(|| "warning".into())),
        project_rules: Some(report),
        net_rules: (!net_rules.is_empty()).then(|| std::sync::Arc::new(net_rules)),
        ..KiCadBoardRouterConfig::default()
    };
    // The narrow-signal and neck widths, too (the classes are floored
    // above).
    config.apply_width_floor();
    Ok(config)
}

/// Raises a net's rules (`index` into the lowering's nets; `None`: the
/// rules of nets without pads) to what the custom rules ask.
fn apply_custom(
    rules: &mut KiCadConnectionRoutingRules,
    lowered: &dru::Lowered,
    index: Option<usize>,
) {
    let per_net = |values: &Vec<f64>, fallback: f64| index.map_or(fallback, |index| values[index]);
    rules.clearance_mm = rules
        .clearance_mm
        .max(lowered.clearance_all)
        .max(per_net(&lowered.clearance, 0.0));
    if let Some(opt) = index.and_then(|index| lowered.width_opt[index]) {
        rules.trace_width_mm = opt;
    }
    let width_min = lowered.width_min_all.max(per_net(&lowered.width_min, 0.0));
    rules.trace_width_mm = rules
        .trace_width_mm
        .min(per_net(&lowered.width_max, f64::INFINITY))
        .max(width_min);
    let drill_min = lowered
        .via_drill_min_all
        .max(per_net(&lowered.via_drill_min, 0.0));
    rules.via_drill_mm = rules
        .via_drill_mm
        .min(per_net(&lowered.via_drill_max, f64::INFINITY))
        .max(drill_min);
    let annulus = lowered
        .annulus_min_all
        .max(per_net(&lowered.annulus_min, 0.0));
    let diameter_min = lowered
        .via_diameter_min_all
        .max(per_net(&lowered.via_diameter_min, 0.0))
        .max(rules.via_drill_mm + 2.0 * annulus);
    rules.via_size_mm = rules
        .via_size_mm
        .min(per_net(&lowered.via_diameter_max, f64::INFINITY))
        .max(diameter_min);
}

/// Which connection_width severities make `min_connection` a floor on the
/// router's widths (`PCB_MIN_CONNECTION_FLOOR`: `off`, `warning` for error
/// and warning severity; anything else or unset: error severity only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloorMode {
    Off,
    Errors,
    ErrorsAndWarnings,
}

fn floor_mode() -> FloorMode {
    match std::env::var("PCB_MIN_CONNECTION_FLOOR").as_deref() {
        Ok("off") => FloorMode::Off,
        Ok("warning") => FloorMode::ErrorsAndWarnings,
        _ => FloorMode::Errors,
    }
}

/// The track width floor from the board setup's minimum connection width
/// and the project's severity of KiCad's connection_width findings. KiCad
/// checks only with a minimum above 0; a project without the severity
/// gets KiCad's default, a warning. By default only the error severity
/// floors the widths (`FloorMode::Errors`): a warning does not fail the
/// pass rule, and on OpenESC (0.16 over classes of 0.09, warning) the floor
/// cost routing in route mode (4in1-mini 52 to 89 unconnected, 4in1 104
/// to 121), so there the designer's choice of a warning is taken at its
/// word; `PCB_MIN_CONNECTION_FLOOR=warning` floors warnings too. `ignore`
/// never does.
pub(crate) fn connection_width_floor(
    min_connection: f64,
    severity: Option<&str>,
    mode: FloorMode,
) -> Option<f64> {
    if min_connection <= 0.0 {
        return None;
    }
    let applies = match (mode, severity.unwrap_or("warning")) {
        (FloorMode::Off, _) | (_, "ignore") => false,
        (FloorMode::Errors, severity) => severity == "error",
        (FloorMode::ErrorsAndWarnings, severity) => severity == "error" || severity == "warning",
    };
    applies.then_some(min_connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_follow_kicad_pattern_semantics() {
        assert!(wildcard_match("GND", "GND"));
        assert!(wildcard_match("/GPIO*", "/GPIO18\\USB_D-"));
        assert!(wildcard_match("Net-(U?-Pad1)", "Net-(U3-Pad1)"));
        assert!(!wildcard_match("GND", "GNDA"));
        assert!(wildcard_match("*", ""));
    }

    #[test]
    fn clearance_as_warning_offers_the_board_minimum() {
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-relaxed-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let pcb = directory.join("board.kicad_pcb");
        fs::write(
            &pcb,
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
              (footprint "a" (at 0 0) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "A"))))"#,
        )
        .unwrap();
        let project = directory.join("board.kicad_pro");
        let write = |severity: &str| {
            let text = format!(
                r#"{{"board": {{"design_settings": {{"rules": {{"min_clearance": 0.1}}, "rule_severities": {{"clearance": "{severity}"}}}}}},
                   "net_settings": {{"classes": [{{"name": "Default", "clearance": 0.2, "track_width": 0.2, "via_diameter": 0.6, "via_drill": 0.3}}]}}}}"#
            );
            fs::write(&project, text).unwrap();
        };
        write("warning");
        assert_eq!(
            resolve_project_rules(&project, &pcb)
                .unwrap()
                .relaxed_clearance_mm,
            Some(0.1)
        );
        write("error");
        assert_eq!(
            resolve_project_rules(&project, &pcb)
                .unwrap()
                .relaxed_clearance_mm,
            None
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn the_connection_width_severity_decides_the_floor() {
        use FloorMode::*;
        assert_eq!(
            connection_width_floor(0.16, Some("warning"), ErrorsAndWarnings),
            Some(0.16)
        );
        assert_eq!(
            connection_width_floor(0.127, Some("error"), ErrorsAndWarnings),
            Some(0.127)
        );
        // KiCad's default severity is a warning.
        assert_eq!(
            connection_width_floor(0.1, None, ErrorsAndWarnings),
            Some(0.1)
        );
        assert_eq!(
            connection_width_floor(0.1, Some("ignore"), ErrorsAndWarnings),
            None
        );
        // No minimum: KiCad does not check.
        assert_eq!(
            connection_width_floor(0.0, Some("error"), ErrorsAndWarnings),
            None
        );
        assert_eq!(connection_width_floor(0.16, Some("warning"), Errors), None);
        assert_eq!(
            connection_width_floor(0.127, Some("error"), Errors),
            Some(0.127)
        );
        assert_eq!(connection_width_floor(0.127, Some("error"), Off), None);
    }

    #[test]
    fn the_minimum_connection_floors_the_project_widths() {
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-min-connection-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let pcb = directory.join("board.kicad_pcb");
        fs::write(
            &pcb,
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
              (footprint "a" (at 0 0) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "A"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "PWR"))))"#,
        )
        .unwrap();
        let project = directory.join("board.kicad_pro");
        // OpenESC's setup (classes and menu at 0.09, the minimum connection
        // 0.16; a power class above it), at several severities.
        let write = |severity: &str| {
            let text = format!(
                r#"{{"board": {{"design_settings": {{"rules": {{"min_track_width": 0.09, "min_connection": 0.16}},
                       "rule_severities": {{"connection_width": "{severity}"}},
                       "track_widths": [0.0, 0.09, 0.11, 0.2, 0.3]}}}},
                   "net_settings": {{"classes": [{{"name": "Default", "clearance": 0.09, "track_width": 0.09, "via_diameter": 0.4, "via_drill": 0.2}},
                                                {{"name": "Power", "clearance": 0.09, "track_width": 0.3, "via_diameter": 0.4, "via_drill": 0.2}}],
                                    "netclass_assignments": {{"PWR": "Power"}}}}}}"#
            );
            fs::write(&project, text).unwrap();
        };
        // At warning severity the default sets no floor.
        if std::env::var_os("PCB_MIN_CONNECTION_FLOOR").is_none() {
            write("warning");
            let config = resolve_project_rules(&project, &pcb).unwrap();
            assert_eq!(config.min_connection_mm, None);
            assert_eq!(config.connection_width_severity.as_deref(), Some("warning"));
            assert_eq!(config.connection_rules["A"].trace_width_mm, 0.09);
        }
        write("error");
        let config = resolve_project_rules(&project, &pcb).unwrap();
        assert_eq!(config.min_connection_mm, Some(0.16));
        assert_eq!(config.connection_width_severity.as_deref(), Some("error"));
        assert_eq!(config.connection_rules["A"].trace_width_mm, 0.16);
        assert_eq!(config.connection_rules["PWR"].trace_width_mm, 0.3);
        // The menu's smallest width, raised to the floor.
        assert_eq!(config.narrow_signal_mm, Some(0.16));
        assert_eq!(config.neck_width_mm, Some(0.16));
        write("ignore");
        let config = resolve_project_rules(&project, &pcb).unwrap();
        assert_eq!(config.min_connection_mm, None);
        assert_eq!(config.connection_width_severity.as_deref(), Some("ignore"));
        assert_eq!(config.connection_rules["A"].trace_width_mm, 0.09);
        assert_eq!(config.narrow_signal_mm, Some(0.09));
        assert_eq!(config.neck_width_mm, Some(0.1));
        fs::remove_dir_all(&directory).unwrap();
    }

    fn project_directory(tag: &str, project: &str, dru: Option<&str>) -> (PathBuf, PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-rules-{tag}-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let pcb = directory.join("board.kicad_pcb");
        fs::write(
            &pcb,
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (footprint "a" (at 0 0)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/POE_RAW"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "V_POE_GOOD"))
                (pad "3" thru_hole circle (at 4 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu") (net "SDA"))
                (pad "4" smd rect (at 6 0) (size 1 1) (layers "F.Cu") (net "USB_DP"))))"#,
        )
        .unwrap();
        let path = directory.join("board.kicad_pro");
        fs::write(&path, project).unwrap();
        if let Some(dru) = dru {
            fs::write(directory.join("board.kicad_dru"), dru).unwrap();
        }
        (directory, path)
    }

    #[test]
    fn a_project_without_minimums_gets_kicads_defaults() {
        // As the intake writes an inferred project: classes only.
        let (directory, project) = project_directory(
            "defaults",
            r#"{"net_settings": {"classes": [{"name": "Default", "clearance": 0.2, "track_width": 0.5, "via_diameter": 0.4, "via_drill": 0.2}]}}"#,
            None,
        );
        let config = resolve_project_rules(&project, &directory.join("board.kicad_pcb")).unwrap();
        // KiCad checks 0.2 mm tracks without a min_track_width: the router
        // necked down to 0.127 before.
        assert_eq!(config.neck_width_mm, Some(0.2));
        let rules = &config.connection_rules["SDA"];
        // min_via_diameter 0.5, min_through_hole_diameter 0.3.
        assert_eq!(rules.via_drill_mm, 0.3);
        assert_eq!(rules.via_size_mm, 0.5);
        assert_eq!(config.edge_clearance_mm, 0.5);
        let report = config.project_rules.unwrap();
        assert!(
            report
                .kicad_defaults
                .contains(&"min_track_width 0.2 mm".to_string())
        );
        assert!(report.custom_rules_file.is_none());
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn net_class_patterns_are_regular_expressions_and_classes_combine() {
        // dacha's compute board: `.*(_TAP|V_POE|POE_RAW).*` is a regular
        // expression; the router read it as a wildcard and gave /POE_RAW
        // the Default class (402 HIGH_V clearance findings). A class
        // without a width takes Default's (KiCad 8); a net in two classes
        // takes each value from the first by priority that has it.
        let (directory, project) = project_directory(
            "patterns",
            r#"{"board": {"design_settings": {"rules": {"min_track_width": 0.1}}},
                "net_settings": {
                  "classes": [
                    {"name": "Default", "clearance": 0.15, "track_width": 0.2, "via_diameter": 0.5, "via_drill": 0.25, "priority": 2147483647},
                    {"name": "HIGH_V", "clearance": 0.3, "track_width": 0.4, "priority": 1},
                    {"name": "HIGH_V_SIGNAL", "clearance": 0.3, "priority": 0},
                    {"name": "USB", "track_width": 0.3, "priority": 5}],
                  "netclass_patterns": [
                    {"netclass": "HIGH_V", "pattern": ".*(_TAP|V_POE|POE_RAW).*"},
                    {"netclass": "HIGH_V_SIGNAL", "pattern": "V_POE_GOOD"},
                    {"netclass": "USB", "pattern": "USB_D?"},
                    {"netclass": "HIGH_V", "pattern": "+3V3"}]}}"#,
            None,
        );
        let config = resolve_project_rules(&project, &directory.join("board.kicad_pcb")).unwrap();
        let rules = |net: &str| config.connection_rules[net].clone();
        assert_eq!(rules("POE_RAW").clearance_mm, 0.3);
        assert_eq!(rules("POE_RAW").trace_width_mm, 0.4);
        // V_POE_GOOD: HIGH_V_SIGNAL (priority 0) has no width, HIGH_V (1)
        // has 0.4.
        assert_eq!(rules("V_POE_GOOD").clearance_mm, 0.3);
        assert_eq!(rules("V_POE_GOOD").trace_width_mm, 0.4);
        // USB: a wildcard; clearance and vias from Default.
        assert_eq!(rules("USB_DP").trace_width_mm, 0.3);
        assert_eq!(rules("USB_DP").clearance_mm, 0.15);
        assert_eq!(rules("USB_DP").via_size_mm, 0.5);
        assert_eq!(rules("SDA").clearance_mm, 0.15);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn custom_rules_raise_the_router_rules_and_are_reported() {
        let (directory, project) = project_directory(
            "custom",
            r#"{"board": {"design_settings": {"rules": {"min_track_width": 0.15, "min_copper_edge_clearance": 0.0, "min_clearance": 0.1}}},
                "net_settings": {
                  "classes": [
                    {"name": "Default", "clearance": 0.15, "track_width": 0.2, "via_diameter": 0.6, "via_drill": 0.3},
                    {"name": "HV", "clearance": 0.2, "track_width": 0.5}],
                  "netclass_patterns": [{"netclass": "HV", "pattern": "*POE*"}]}}"#,
            Some(
                r#"(version 1)
(rule "HV isolation" (condition "A.NetClass == 'HV' && A.Net != B.Net") (constraint clearance (min 1.0mm)))
(rule "Track width, outer" (layer outer) (condition "A.Type == 'track'") (constraint track_width (min 0.127mm)))
(rule "Wide USB" (condition "A.NetName == 'USB_DP'") (constraint track_width (min 0.25mm)))
(rule "Edge" (condition "A.Type == 'track'") (constraint edge_clearance (min 0.3mm)))
(rule "PTH to track" (condition "A.isPlated() && A.Type != 'Via' && B.Type == 'track'") (constraint hole_clearance (min 0.33mm)))
(rule "Creepage" (condition "A.NetClass == 'HV'") (constraint creepage (min 4mm)))
(rule "Keepout" (condition "A.insideArea('ANT')") (constraint clearance (min 5mm)))
(rule "Silk" (constraint silk_clearance (min 0.15mm)))"#,
            ),
        );
        let config = resolve_project_rules(&project, &directory.join("board.kicad_pcb")).unwrap();
        let rules = |net: &str| config.connection_rules[net].clone();
        assert_eq!(rules("POE_RAW").clearance_mm, 1.0);
        assert_eq!(rules("V_POE_GOOD").clearance_mm, 1.0);
        assert_eq!(rules("SDA").clearance_mm, 0.15);
        assert_eq!(config.default_rules.as_ref().unwrap().clearance_mm, 0.15);
        assert_eq!(rules("USB_DP").trace_width_mm, 0.25);
        assert_eq!(rules("SDA").trace_width_mm, 0.2);
        // The looser outer-layer width leaves the board setup's 0.15.
        assert_eq!(config.neck_width_mm, Some(0.15));
        assert_eq!(config.edge_clearance_mm, 0.3);
        assert_eq!(config.hole_clearance_mm, 0.33);
        let report = config.project_rules.unwrap();
        assert_eq!(report.custom_rules_file.as_deref(), Some("board.kicad_dru"));
        let status: Vec<(&str, &str)> = report
            .custom_rules
            .iter()
            .map(|rule| (rule.rule.as_str(), rule.status.as_str()))
            .collect();
        assert_eq!(
            status,
            [
                ("HV isolation", "applied"),
                ("Track width, outer", "no_effect"),
                ("Wide USB", "applied"),
                ("Edge", "approximated"),
                ("PTH to track", "approximated"),
                ("Creepage", "unmodelled"),
                ("Keepout", "unmodelled"),
                ("Silk", "not_routing"),
            ]
        );
        assert_eq!(report.unmodelled.len(), 2);
        assert!(report.unmodelled[1].contains("A.insidearea()"));
        // USB_DP's minimum floors its neck-downs (the others keep the
        // board's 0.15); hole to hole is a warning by KiCad's default.
        let net_rules = config.net_rules.unwrap();
        assert_eq!(net_rules.width_floor, [("USB_DP".to_string(), 0.25)].into());
        assert!(net_rules.clearances.is_empty());
        assert_eq!(net_rules.same_net_hole_to_hole, 0.0);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn pair_item_and_layer_clearances_become_router_rules() {
        let (directory, project) = project_directory(
            "pairs",
            r#"{"board": {"design_settings": {"rules": {"min_track_width": 0.15, "min_hole_to_hole": 0.5},
                                              "rule_severities": {"hole_near_hole": "error"}}},
                "net_settings": {
                  "classes": [
                    {"name": "Default", "clearance": 0.127, "track_width": 0.2, "via_diameter": 0.6, "via_drill": 0.3},
                    {"name": "PRI", "clearance": 0.2, "track_width": 0.5}],
                  "netclass_patterns": [{"netclass": "PRI", "pattern": "*POE*"}]}}"#,
            Some(
                r#"(version 1)
(rule "Isolation" (condition "A.NetClass == 'PRI' && B.NetClass == 'Default'") (constraint clearance (min 2.5mm)))
(rule "Pad to Track" (condition "A.Type == 'Pad' && B.Type == 'Track'") (constraint clearance (min 0.2mm)))
(rule "Outer" (layer outer) (condition "A.Type == 'Via' && B.Type == 'Via'") (constraint clearance (min 0.3mm)))"#,
            ),
        );
        let config = resolve_project_rules(&project, &directory.join("board.kicad_pcb")).unwrap();
        // No net's own clearance rises: SDA keeps 0.127 against SDA's
        // kind, the primary nets their 0.2.
        assert_eq!(config.connection_rules["SDA"].clearance_mm, 0.127);
        assert_eq!(config.connection_rules["POE_RAW"].clearance_mm, 0.2);
        let report = config.project_rules.as_ref().unwrap();
        let status: Vec<&str> = report
            .custom_rules
            .iter()
            .map(|rule| rule.status.as_str())
            .collect();
        assert_eq!(status, ["applied", "applied", "applied"]);
        assert!(report.unmodelled.is_empty());
        let net_rules = config.net_rules.unwrap();
        // KiCad 8's hole_near_hole as an error: within a net too.
        assert_eq!(net_rules.same_net_hole_to_hole, 0.5);
        let isolation: Vec<&pcb_router::ClearanceRule> = net_rules
            .clearances
            .iter()
            .filter(|rule| rule.clearance == 2.5)
            .collect();
        assert_eq!(isolation.len(), 2);
        let primary = Some(vec!["POE_RAW".to_string(), "V_POE_GOOD".to_string()]);
        let secondary = Some(vec!["SDA".to_string(), "USB_DP".to_string()]);
        assert!(
            isolation
                .iter()
                .any(|rule| rule.first == primary && rule.second == secondary)
        );
        // The later rules take tracks against pads and vias against vias
        // from it (KiCad applies the last matching rule).
        use pcb_router::rules::{ALL_KINDS, PAD, TRACK, VIA, kind_bit};
        assert!(
            isolation
                .iter()
                .all(|rule| rule.kinds == ALL_KINDS & !kind_bit(TRACK, PAD) & !kind_bit(VIA, VIA))
        );
        let pad = net_rules
            .clearances
            .iter()
            .find(|rule| rule.clearance == 0.2)
            .unwrap();
        assert_eq!((&pad.first, &pad.second), (&None, &None));
        assert_eq!(
            pad.kinds,
            pcb_router::rules::kind_bit(pcb_router::rules::TRACK, pcb_router::rules::PAD)
        );
        let vias = net_rules
            .clearances
            .iter()
            .find(|rule| rule.clearance == 0.3)
            .unwrap();
        assert_eq!(
            vias.kinds,
            pcb_router::rules::kind_bit(pcb_router::rules::VIA, pcb_router::rules::VIA)
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    /// The lowering over the intake's boards with custom rules
    /// (`PCB_RULES_CENSUS=<root>/benchmarks/expansion/data/manifest.json
    /// cargo test --release -p pcb-kicad census_of_custom_rules --
    /// --ignored --nocapture`): per constraint, how many rules and boards
    /// end in each status.
    #[test]
    #[ignore]
    fn census_of_custom_rules() {
        let Some(manifest) = std::env::var_os("PCB_RULES_CENSUS") else {
            return;
        };
        let manifest = Path::new(&manifest);
        let root = manifest.ancestors().nth(4).unwrap();
        let boards: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(manifest).unwrap()).unwrap();
        let mut counts = BTreeMap::<(String, String), (usize, BTreeSet<String>)>::new();
        let mut failed = 0;
        let mut with_rules = 0;
        for board in boards["boards"].as_array().unwrap() {
            let Some(rules) = board["custom_rules_path"].as_str() else {
                continue;
            };
            with_rules += 1;
            let pcb = root.join(board["board_path"].as_str().unwrap());
            let project = root.join(rules).with_extension("kicad_pro");
            let name = board["name"].as_str().unwrap().to_string();
            match resolve_project_rules(&project, &pcb) {
                Ok(config) => {
                    let report = config.project_rules.unwrap();
                    if let Some(error) = &report.custom_rules_error {
                        println!("{name}: {error}");
                    }
                    for rule in report.custom_rules {
                        let entry = counts.entry((rule.constraint, rule.status)).or_default();
                        entry.0 += 1;
                        entry.1.insert(name.clone());
                    }
                }
                Err(error) => {
                    failed += 1;
                    println!("{name}: {error}");
                }
            }
        }
        println!("{with_rules} boards with a .kicad_dru, {failed} not resolved");
        println!("| constraint | status | rules | boards |");
        println!("| --- | --- | ---: | ---: |");
        for ((constraint, status), (rules, boards)) in counts {
            println!("| {constraint} | {status} | {rules} | {} |", boards.len());
        }
    }
}

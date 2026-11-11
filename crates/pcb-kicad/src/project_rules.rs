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
    /// The board's minimum track width and clearance, with where each
    /// comes from (the project's board setup or KiCad's default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimums: Option<KiCadProjectMinimums>,
    /// Widths and clearances a router file asked for below the project's
    /// (`allow_below_project_minimum`): the run used them; KiCad's DRC
    /// holds the board to the project's.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<KiCadRuleOverride>,
    /// The largest clearance a custom rule asks between the pads of every
    /// two nets on some layer (the placer keeps parts at least that far
    /// apart, and at least the classes' largest: `largest_clearance`); a
    /// rule on some layers only reaches the router as a pair rule
    /// (`net_rules.clearances`), which no class shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad_clearance_mm: Option<f64>,
    /// The footprints whose small SMD pads the board's own vias (or the
    /// `--reference` board's) lie in: the designer's sign that the fab
    /// fills and caps them, which lets the router put a via in those
    /// footprints' enclosed pads (`via_in_pad`; docs/router.md, "Vias in
    /// BGA pads").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub via_in_pad_designer: Vec<crate::board_router::KiCadViaInPadEntry>,
    /// The spans of the board's own blind, buried and micro vias (or the
    /// `--reference` board's): the designer's sign that the fab builds
    /// them, which allows those spans to the router (`blind_vias`;
    /// docs/router.md, "Blind, buried and micro vias").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub via_spans_designer: Vec<crate::board_router::KiCadViaSpanEntry>,
    /// A KiCad 6 or 7 project's via switches (`allow_blind_buried_vias`,
    /// `allow_microvias`) as `via_spans` keywords.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub via_spans_project: Vec<String>,
    /// The board's micro via minimums (board setup).
    #[serde(skip)]
    pub micro_via_minimums: crate::board_router::KiCadMicroViaMinimums,
    /// The default class's micro via (`microvia_diameter`,
    /// `microvia_drill`), where the project states it.
    #[serde(skip)]
    pub micro_via_mm: Option<[f64; 2]>,
    /// The project's predefined via sizes (diameter, drill), for a via in
    /// a pad.
    #[serde(skip)]
    pub via_sizes: Vec<[f64; 2]>,
    /// The board's via minimums: diameter, drill, annular width (board
    /// setup, custom rules on every via).
    #[serde(skip)]
    pub via_minimums: [f64; 3],
    /// Per net (as the router names it): its classes, the highest
    /// priority first ("Default" for a net in none).
    #[serde(skip)]
    pub net_classes: BTreeMap<String, Vec<String>>,
    /// Per class of the project: its track width and clearance as the
    /// project states them (before the board's minimums).
    #[serde(skip)]
    pub class_values: BTreeMap<String, [Option<f64>; 2]>,
    /// Per net (as the router names it) that a custom `track_width` rule
    /// with an opt or a maximum bounds on some layers only: per such layer
    /// the width the rule sets (its opt, else its min, else its max): the
    /// designer's widths per layer, as for one impedance on outer and inner
    /// layers (a minimum alone, a process floor such as 2 oz outer copper's,
    /// is none).
    #[serde(skip)]
    pub layer_track_widths: BTreeMap<String, BTreeMap<String, f64>>,
    /// Per net a custom `disallow track` rule keeps off copper layers:
    /// those layers.
    #[serde(skip)]
    pub track_disallowed_layers: BTreeMap<String, Vec<String>>,
    /// The custom rules' track geometry on some copper layers only (a
    /// `track_width`, `diff_pair_gap` or `diff_pair_width` constraint
    /// whose layer clause or condition names a strict subset of the
    /// board's copper layers): a designer's controlled impedance.
    #[serde(skip)]
    pub layer_geometry_rules: Vec<KiCadLayerGeometryRule>,
}

/// A custom rule's track geometry on some layers only
/// (`KiCadProjectRulesReport::layer_geometry_rules`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadLayerGeometryRule {
    pub rule: String,
    pub constraint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opt_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_mm: Option<f64>,
    /// The copper layers its layer clause and condition name.
    pub layers: Vec<String>,
    /// Its condition as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

/// The class whose track width a net takes: of its classes (`classes`,
/// the highest priority first, `KiCadProjectRulesReport::net_classes`) the
/// first whose own width the project states (`class_values`), else
/// "Default" (a net in no class, or in classes that set only clearances).
pub(crate) fn width_class<'a>(
    classes: Option<&'a Vec<String>>,
    class_values: &BTreeMap<String, [Option<f64>; 2]>,
) -> &'a str {
    classes
        .and_then(|names| {
            names.iter().find(|class| {
                class_values
                    .get(*class)
                    .is_some_and(|values| values[0].is_some())
            })
        })
        .map_or("Default", String::as_str)
}

/// The board's minimum track width and clearance (`min_track_width`,
/// `min_clearance` of the board setup), each with its source.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KiCadProjectMinimums {
    pub track_width_mm: f64,
    pub track_width_source: String,
    pub clearance_mm: f64,
    pub clearance_source: String,
}

/// A width or clearance a router file asks for below what the project
/// holds the board to (`preflight_overrides`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KiCadRuleOverride {
    /// The file's key: `net_classes[<name>]`, `default_rules`,
    /// `connection_rules[<net>]`, `neck_width_mm`, `narrow_signal_mm`,
    /// `relaxed_clearance_mm`.
    pub source: String,
    /// `track_width` or `clearance`.
    pub quantity: String,
    pub requested_mm: f64,
    /// What the project holds it to, and where that comes from.
    pub project_minimum_mm: f64,
    pub project_source: String,
    /// The nets (or the class's patterns) it applies to; empty for all.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<String>,
}

impl KiCadRuleOverride {
    /// One line for an error or a warning.
    pub fn line(&self) -> String {
        format!(
            "{} asks for {} {} mm below the project's {} mm ({}){}",
            self.source,
            self.quantity.replace('_', " "),
            self.requested_mm,
            self.project_minimum_mm,
            self.project_source,
            if self.nets.is_empty() {
                String::new()
            } else {
                format!(" on {}", self.nets.join(", "))
            }
        )
    }
}

/// The widths and clearances a router file (`named`: its keys; `config`:
/// the file over the project) asks for below the project's (`project`:
/// its resolution): a net class's or `default_rules`' or a net's
/// (`connection_rules`) track width below the board's `min_track_width`,
/// its clearance below the board's `min_clearance` (and `default_rules`'
/// and `connection_rules`' below the project's own class clearance,
/// which KiCad's DRC holds the board to), the neck and narrow-signal
/// widths below `min_track_width`, `relaxed_clearance_mm` below
/// `min_clearance`.
pub fn preflight_overrides(
    named: &BTreeSet<String>,
    config: &KiCadBoardRouterConfig,
    project: &KiCadBoardRouterConfig,
) -> Vec<KiCadRuleOverride> {
    const EPSILON: f64 = 1.0e-9;
    let Some(minimums) = project
        .project_rules
        .as_ref()
        .and_then(|report| report.minimums.clone())
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut check = |source: String,
                     quantity: &str,
                     requested: Option<f64>,
                     minimum: f64,
                     minimum_source: &str,
                     nets: Vec<String>| {
        if let Some(requested) = requested
            && requested < minimum - EPSILON
        {
            found.push(KiCadRuleOverride {
                source,
                quantity: quantity.into(),
                requested_mm: requested,
                project_minimum_mm: minimum,
                project_source: minimum_source.into(),
                nets,
            });
        }
    };
    let width = (
        minimums.track_width_mm,
        minimums.track_width_source.as_str(),
    );
    let clearance = (minimums.clearance_mm, minimums.clearance_source.as_str());
    for class in &config.net_classes {
        let source = format!("net_classes[{}]", class.name);
        check(
            source.clone(),
            "track_width",
            class.track_width_mm,
            width.0,
            width.1,
            class.nets.clone(),
        );
        check(
            source,
            "clearance",
            class.clearance_mm,
            clearance.0,
            clearance.1,
            class.nets.clone(),
        );
    }
    if named.contains("default_rules")
        && let Some(rules) = &config.default_rules
    {
        check(
            "default_rules".into(),
            "track_width",
            Some(rules.trace_width_mm),
            width.0,
            width.1,
            Vec::new(),
        );
        let board = project
            .default_rules
            .as_ref()
            .map_or(0.0, |rules| rules.clearance_mm);
        let (minimum, source) = if board > clearance.0 {
            (
                board,
                "the project's Default class clearance (net_settings.classes)",
            )
        } else {
            clearance
        };
        check(
            "default_rules".into(),
            "clearance",
            Some(rules.clearance_mm),
            minimum,
            source,
            Vec::new(),
        );
    }
    if named.contains("connection_rules") {
        for (net, rules) in &config.connection_rules {
            let source = format!("connection_rules[{net}]");
            check(
                source.clone(),
                "track_width",
                Some(rules.trace_width_mm),
                width.0,
                width.1,
                vec![net.clone()],
            );
            let board = project
                .connection_rules
                .get(net)
                .or(project.default_rules.as_ref())
                .map_or(0.0, |rules| rules.clearance_mm);
            let (minimum, source_of) = if board > clearance.0 {
                (
                    board,
                    "the net's class clearance in the project (net_settings.classes)",
                )
            } else {
                clearance
            };
            check(
                source,
                "clearance",
                Some(rules.clearance_mm),
                minimum,
                source_of,
                vec![net.clone()],
            );
        }
    }
    for (key, value) in [
        ("neck_width_mm", config.neck_width_mm),
        ("narrow_signal_mm", config.narrow_signal_mm),
    ] {
        if named.contains(key) {
            check(
                key.into(),
                "track_width",
                value,
                width.0,
                width.1,
                Vec::new(),
            );
        }
    }
    if named.contains("relaxed_clearance_mm") {
        check(
            "relaxed_clearance_mm".into(),
            "clearance",
            config.relaxed_clearance_mm,
            clearance.0,
            clearance.1,
            Vec::new(),
        );
    }
    found
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
            // A pattern that is no expression by itself is none anchored
            // either: `x)|(?:.*` closes the anchoring group and matched
            // every net (KiCad reports the expression as invalid).
            regex: regex_lite::Regex::new(text)
                .ok()
                .and_then(|_| regex_lite::Regex::new(&format!("^(?:{text})$")).ok()),
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
    /// The class's `diff_pair_width` and `diff_pair_gap`.
    dp_width: Option<f64>,
    dp_gap: Option<f64>,
}

/// The board's zones and rule areas with a name or uuid, the footprints'
/// too, with their copper layers (`dru::NamedArea`): what KiCad's area
/// tests in rule conditions search (`searchAreas`).
fn named_areas(pcb: &Expr) -> Vec<dru::NamedArea> {
    let copper = copper_layer_names(pcb);
    let zones = pcb.children().iter().flat_map(|item| match item.head() {
        Some("zone") => vec![item],
        Some("footprint") => item
            .children()
            .iter()
            .filter(|child| child.head() == Some("zone"))
            .collect(),
        _ => Vec::new(),
    });
    zones
        .map(|zone| {
            let names: Vec<&str> = match form_atom(zone, "layer", 1) {
                Some(layer) => vec![layer],
                None => zone
                    .child("layers")
                    .map(Expr::children)
                    .unwrap_or_default()
                    .iter()
                    .skip(1)
                    .filter_map(Expr::atom)
                    .collect(),
            };
            let layers = copper
                .iter()
                .filter(|layer| {
                    names.iter().any(|name| match *name {
                        "*.Cu" => true,
                        "F&B.Cu" => matches!(layer.as_str(), "F.Cu" | "B.Cu"),
                        "*.In.Cu" | "In*.Cu" => layer.starts_with("In"),
                        name => name == layer.as_str(),
                    })
                })
                .cloned()
                .collect();
            dru::NamedArea {
                name: form_atom(zone, "name", 1).unwrap_or_default().to_string(),
                uuid: form_atom(zone, "uuid", 1).unwrap_or_default().to_string(),
                layers,
            }
        })
        .filter(|area| !area.name.is_empty() || !area.uuid.is_empty())
        .collect()
}

/// The copper layer names of a board's `(layers ...)`, in the router's
/// order (F.Cu, In1.Cu, In2.Cu, ..., B.Cu: the lowering's `LayerTable`),
/// so that a layer's index is the router's.
pub(crate) fn copper_layer_names(pcb: &Expr) -> Vec<String> {
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
///
/// The stages: the board setup (`report_board_setup`), the net classes
/// (`Classes`) and the nets' (`assign_nets`), the board-wide values
/// (`board_wide`), the custom rules (`read_custom_rules`,
/// `apply_lowered`), the rules on one track (`diff_pair_rules`,
/// `length_rules`, `track_disallowed`), then the config.
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
    // The connections are keyed as the lowering will look them up: a board
    // with a bare net beside a hierarchical one of the same name keeps the
    // slash (meshsat's "/VIN_CELL" and "VIN_CELL": keyed "VIN_CELL" while
    // the lowering asked for "/VIN_CELL", the net routed at Default's 0.14
    // instead of its class CELL's 0.15).
    let net_names = crate::note_net_names(&pcb);
    let mut report = KiCadProjectRulesReport::default();

    let rules = &project["board"]["design_settings"]["rules"];
    let setup = DesignRules(rules);
    report_board_setup(&project, &pcb, setup, &mut report);
    let severity = project["board"]["design_settings"]["rule_severities"]["connection_width"]
        .as_str()
        .map(str::to_owned);
    let floor = connection_width_floor(
        setup.minimum("min_connection"),
        severity.as_deref(),
        floor_mode(),
    );

    let classes = Classes::of_project(&project, &pcb, setup, floor, &mut report);
    let default_rules = classes.floored(classes.default.clone());
    let (nets, pad_kinds) = pads_of(&pcb);
    let NetAssignment {
        connection_rules,
        pair_classes,
        net_facts,
        keys,
    } = assign_nets(
        nets,
        &classes,
        &Assignments::of_project(&project),
        &default_rules,
        &net_names,
        &mut report,
    );
    let wide = board_wide(&project, &classes, &default_rules);
    let severities = rule_severities(&project);
    // The router's rules beyond the classes (`pcb_router::NetRules`).
    let mut net_rules = pcb_router::NetRules::default();
    // KiCad's hole_to_hole holds between the holes of one net too (a via
    // beside a pad hole of its own net: nuclone's VSS); the router keeps
    // it where the project rates the findings errors.
    if dru::finding_severity("hole_to_hole", &severities) == "error" {
        net_rules.same_net_hole_to_hole = wide.hole_to_hole;
    }
    let mut resolved = Resolved {
        connection_rules,
        default_rules,
        wide,
        net_rules,
        // The nets whose width a custom rule's opt gives on every layer:
        // the narrow-signal step keeps them (`ruled_widths`).
        ruled_widths: BTreeSet::new(),
        report,
    };

    // The project's custom rules.
    let dru_path = project_path.with_extension("kicad_dru");
    let pair_facts = net_facts.clone();
    let mut pair_custom_rules: Vec<dru::Rule> = Vec::new();
    if let Some(rules) = read_custom_rules(&dru_path, &mut resolved.report) {
        let board = board_facts(net_facts, &keys, &pcb, pad_kinds, &resolved);
        let lowered = dru::lower(&rules, &board, &severities);
        let board_minimum = setup.minimum("min_track_width").max(floor.unwrap_or(0.0));
        apply_lowered(&mut resolved, lowered, &board, &keys, board_minimum);
        pair_custom_rules = rules;
    }
    let Resolved {
        connection_rules,
        default_rules,
        wide,
        net_rules,
        ruled_widths,
        mut report,
    } = resolved;

    let diff_pair_rules = diff_pair_rules(
        &pair_facts,
        &keys,
        &pair_classes,
        &pair_custom_rules,
        &copper_layer_names(&pcb),
        |key: &str| connection_rules.get(key).cloned(),
        setup.minimum("min_track_width").max(floor.unwrap_or(0.0)),
    );
    let length_rules = length_rules(
        &pair_facts,
        &keys,
        &pair_custom_rules,
        &copper_layer_names(&pcb),
        &severities,
    );
    let track_disallowed = track_disallowed(
        &pair_facts,
        &keys,
        &pair_custom_rules,
        &copper_layer_names(&pcb),
        &severities,
        &mut report,
    );
    let copper = copper_layer_names(&pcb);
    report.layer_geometry_rules = layer_geometry_rules(&pair_custom_rules, &copper);
    report.track_disallowed_layers = layer_names(&track_disallowed, &copper);
    let mut config = KiCadBoardRouterConfig {
        narrow_signal_mm: wide.narrow_signal,
        // Designers neck down to the board minimum where a pad demands it
        // (KiCad's 0.2 mm where the project states none); a project whose
        // minimum is 0 gets a fab's usual 0.127 mm (KiCad's DRC checks
        // widths against the stated minimum only).
        neck_width_mm: Some(wide.neck),
        connection_rules,
        default_rules: Some(default_rules),
        // KiCad's built-in board setup applies when the project is silent.
        edge_clearance_mm: wide.edge_clearance,
        hole_clearance_mm: wide.hole_clearance,
        hole_to_hole_clearance_mm: wide.hole_to_hole,
        min_connection_mm: floor,
        connection_width_severity: (setup.minimum("min_connection") > 0.0)
            .then(|| severity.unwrap_or_else(|| "warning".into())),
        project_rules: Some(report),
        net_rules: (!net_rules.is_empty()).then(|| std::sync::Arc::new(net_rules)),
        diff_pair_rules,
        length_rules,
        track_disallowed,
        ruled_widths,
        ..setup_checks(&project, setup)
    };
    // The narrow-signal and neck widths, too (the classes are floored
    // above).
    config.apply_width_floor();
    Ok(config)
}

/// The router config's values that come straight from the board setup's
/// checks (the rest at their defaults): the clearance a project relaxes
/// to where its clearance findings are no errors, the solder mask's
/// clearance and bridge severity, the starved-thermal spoke count.
fn setup_checks(project: &serde_json::Value, setup: DesignRules) -> KiCadBoardRouterConfig {
    let rules = setup.0;
    let severities = &project["board"]["design_settings"]["rule_severities"];
    KiCadBoardRouterConfig {
        relaxed_clearance_mm: match severities["clearance"].as_str() {
            Some("warning" | "ignore") if setup.minimum("min_clearance") > 0.0 => {
                Some(setup.minimum("min_clearance"))
            }
            _ => None,
        },
        solder_mask_to_copper_mm: rules["solder_mask_to_copper_clearance"].as_f64(),
        solder_mask_bridge_severity: severities["solder_mask_bridge"].as_str().map(str::to_owned),
        // KiCad's starved-thermal check: the board setup's minimum spoke
        // count, none where the project ignores the finding.
        min_resolved_spokes: Some(
            if severities["starved_thermal"].as_str() == Some("ignore") {
                0
            } else {
                rules["min_resolved_spokes"].as_u64().unwrap_or(2) as usize
            },
        ),
        ..KiCadBoardRouterConfig::default()
    }
}

/// Per net, the names of the copper layers of its mask (`copper`: the
/// router's order).
fn layer_names(
    masks: &BTreeMap<String, pcb_router::LayerMask>,
    copper: &[String],
) -> BTreeMap<String, Vec<String>> {
    masks
        .iter()
        .map(|(net, mask)| {
            let layers = copper
                .iter()
                .enumerate()
                .filter(|(layer, _)| mask & (1 << layer) != 0)
                .map(|(_, name)| name.clone())
                .collect();
            (net.clone(), layers)
        })
        .collect()
}

/// The board setup's design rules (`board.design_settings.rules` of the
/// project).
#[derive(Clone, Copy)]
struct DesignRules<'a>(&'a serde_json::Value);

impl DesignRules<'_> {
    /// A minimum as the project states it, else KiCad's default
    /// (`KICAD_MINIMUMS`), else 0.
    fn minimum(&self, key: &str) -> f64 {
        self.0[key].as_f64().unwrap_or_else(|| {
            KICAD_MINIMUMS
                .iter()
                .find(|(name, _)| *name == key)
                .map_or(0.0, |(_, value)| *value)
        })
    }

    /// Where a minimum comes from, for the report.
    fn source_of(&self, key: &str) -> String {
        if self.0[key].as_f64().is_some() {
            format!("board.design_settings.rules.{key} in the .kicad_pro")
        } else {
            format!("KiCad's default {key} (the project does not state it)")
        }
    }
}

/// In nanometres, as KiCad holds them (a project's 0.19999999999999998
/// is 0.2).
fn nanometres(value: f64) -> f64 {
    (value * 1.0e6).round() / 1.0e6
}

/// Reports the board setup: the minimums taken at KiCad's defaults, the
/// board's minimum track width and clearance with their sources, and
/// what a via may be (the project's via sizes and minimums, the
/// designer's own vias in pads and of spans, the micro vias).
fn report_board_setup(
    project: &serde_json::Value,
    pcb: &Expr,
    setup: DesignRules,
    report: &mut KiCadProjectRulesReport,
) {
    let rules = setup.0;
    for (key, value) in KICAD_MINIMUMS {
        if value > 0.0 && rules[key].as_f64().is_none() {
            report.kicad_defaults.push(format!("{key} {value} mm"));
        }
    }
    report.minimums = Some(KiCadProjectMinimums {
        track_width_mm: nanometres(setup.minimum("min_track_width")),
        track_width_source: setup.source_of("min_track_width"),
        clearance_mm: nanometres(setup.minimum("min_clearance")),
        clearance_source: setup.source_of("min_clearance"),
    });
    // A via in a pad's sizes: the project's predefined vias, within the
    // board's minimums (raised below by custom rules on every via).
    report.via_sizes = project["board"]["design_settings"]["via_dimensions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|size| Some([size["diameter"].as_f64()?, size["drill"].as_f64()?]))
        .filter(|[diameter, drill]| *drill > 0.0 && diameter > drill)
        .map(|[diameter, drill]| [nanometres(diameter), nanometres(drill)])
        .collect();
    report.via_minimums = [
        nanometres(setup.minimum("min_via_diameter")),
        nanometres(setup.minimum("min_through_hole_diameter")),
        nanometres(setup.minimum("min_via_annular_width")),
    ];
    report.via_in_pad_designer = crate::board_router::designer_entries(pcb, "the board");
    report.via_spans_designer = crate::board_router::designer_spans(pcb, "the board");
    report.via_spans_project = crate::board_router::project_spans(rules);
    report.micro_via_minimums = crate::board_router::KiCadMicroViaMinimums {
        diameter_mm: rules["min_microvia_diameter"].as_f64().unwrap_or(0.2),
        drill_mm: rules["min_microvia_drill"].as_f64().unwrap_or(0.1),
        annular_mm: setup.minimum("min_via_annular_width"),
    };
    report.micro_via_mm = project["net_settings"]["classes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|class| class["name"].as_str() == Some("Default"))
        .and_then(|class| {
            Some([
                class["microvia_diameter"].as_f64()?,
                class["microvia_drill"].as_f64()?,
            ])
        })
        .filter(|[diameter, drill]| *drill > 0.0 && diameter > drill)
        .map(|[diameter, drill]| [nanometres(diameter), nanometres(drill)]);
}

/// The project's net classes (`net_settings.classes`) and how a net's
/// values resolve from them, floored at the board's minimums.
struct Classes<'a> {
    specs: Vec<ClassSpec>,
    /// Default's values before the board's minimums (`default_class`).
    default: KiCadConnectionRoutingRules,
    setup: DesignRules<'a>,
    /// The track width floor of the board's minimum connection width
    /// (`connection_width_floor`).
    floor: Option<f64>,
}

impl<'a> Classes<'a> {
    /// Reads the project's classes, reporting each one's track width and
    /// clearance as the project states them (`class_values`).
    fn of_project(
        project: &serde_json::Value,
        pcb: &Expr,
        setup: DesignRules<'a>,
        floor: Option<f64>,
        report: &mut KiCadProjectRulesReport,
    ) -> Self {
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
            report
                .class_values
                .insert(name.to_string(), [value("track_width"), value("clearance")]);
            specs.push(ClassSpec {
                name: name.to_string(),
                priority: class["priority"].as_i64().unwrap_or(index as i64),
                track: value("track_width"),
                clearance: value("clearance"),
                via: value("via_diameter"),
                drill: value("via_drill"),
                dp_width: value("diff_pair_width"),
                dp_gap: value("diff_pair_gap"),
            });
        }
        let default = default_class(&specs, pcb);
        Self {
            specs,
            default,
            setup,
            floor,
        }
    }

    /// The class named `name`, if the project has it.
    fn spec(&self, name: &str) -> Option<&ClassSpec> {
        self.specs.iter().find(|spec| spec.name == name)
    }

    /// A class's values floored at the board's minimums (and the via's
    /// diameter at its drill and annular ring).
    fn floored(&self, mut class: KiCadConnectionRoutingRules) -> KiCadConnectionRoutingRules {
        let minimum = |key: &str| self.setup.minimum(key);
        class.trace_width_mm = class
            .trace_width_mm
            .max(minimum("min_track_width"))
            .max(self.floor.unwrap_or(0.0));
        class.clearance_mm = class.clearance_mm.max(minimum("min_clearance"));
        class.via_size_mm = class.via_size_mm.max(minimum("min_via_diameter"));
        class.via_drill_mm = class.via_drill_mm.max(minimum("min_through_hole_diameter"));
        let annulus = minimum("min_via_annular_width").max(0.05);
        class.via_size_mm = class.via_size_mm.max(class.via_drill_mm + 2.0 * annulus);
        class
    }

    /// The values of a net in `classes` (names of `specs`, any order).
    fn resolve(&self, classes: &[&ClassSpec]) -> KiCadConnectionRoutingRules {
        let mut ordered: Vec<&ClassSpec> = classes.to_vec();
        ordered.sort_by_key(|spec| spec.priority);
        let first =
            |field: fn(&ClassSpec) -> Option<f64>| ordered.iter().find_map(|spec| field(spec));
        let default = &self.default;
        self.floored(KiCadConnectionRoutingRules {
            trace_width_mm: first(|spec| spec.track).unwrap_or(default.trace_width_mm),
            clearance_mm: first(|spec| spec.clearance).unwrap_or(default.clearance_mm),
            via_size_mm: first(|spec| spec.via).unwrap_or(default.via_size_mm),
            via_drill_mm: first(|spec| spec.drill).unwrap_or(default.via_drill_mm),
        })
    }
}

/// Default's values: the project's Default class (KiCad's built-in
/// values where it leaves one out), else, for a project without net
/// settings, the board's most common track width and via.
fn default_class(specs: &[ClassSpec], pcb: &Expr) -> KiCadConnectionRoutingRules {
    match specs.iter().find(|spec| spec.name == "Default") {
        Some(spec) => KiCadConnectionRoutingRules {
            trace_width_mm: spec.track.unwrap_or(DEFAULT_TRACK),
            clearance_mm: spec.clearance.unwrap_or(DEFAULT_CLEARANCE),
            via_size_mm: spec.via.unwrap_or(DEFAULT_VIA),
            via_drill_mm: spec.drill.unwrap_or(DEFAULT_DRILL),
        },
        None => {
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
            KiCadConnectionRoutingRules {
                trace_width_mm: most_common(&widths).unwrap_or(DEFAULT_TRACK),
                clearance_mm: DEFAULT_CLEARANCE,
                via_size_mm: most_common(&via_sizes).unwrap_or(DEFAULT_VIA),
                via_drill_mm: most_common(&via_drills).unwrap_or(DEFAULT_DRILL),
            }
        }
    }
}

/// The nets of the board's pads (as the board writes them) and the kinds
/// of pad it has.
fn pads_of(pcb: &Expr) -> (BTreeSet<String>, BTreeSet<dru::Kind>) {
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
    (nets, pad_kinds)
}

/// How the project assigns nets to classes besides their patterns' own:
/// by name (`netclass_assignments`), by pattern (`netclass_patterns`),
/// and by a KiCad 6 or 7 class's net list.
struct Assignments<'a> {
    assignments: &'a serde_json::Value,
    patterns: Vec<(NetClassPattern, String)>,
    /// Per net, the class whose list names it.
    listed: BTreeMap<String, String>,
}

impl<'a> Assignments<'a> {
    fn of_project(project: &'a serde_json::Value) -> Self {
        let patterns = project["net_settings"]["netclass_patterns"]
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
        let listed = project["net_settings"]["classes"]
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
        Self {
            assignments: &project["net_settings"]["netclass_assignments"],
            patterns,
            listed,
        }
    }

    /// The names of the classes a net (`text` and `key`, `key_and_text`) is
    /// assigned to, each once, in the order found: by name, by its class's
    /// list (by its text, else its key's), by pattern.
    fn class_names(&self, text: &str, key: &str) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut add = |name: &str| {
            if !names.iter().any(|known| known == name) {
                names.push(name.to_string());
            }
        };
        match &self.assignments[text] {
            serde_json::Value::String(name) => add(name),
            serde_json::Value::Array(entries) => {
                for name in entries.iter().filter_map(serde_json::Value::as_str) {
                    add(name);
                }
            }
            _ => {}
        }
        if let Some(name) = self
            .listed
            .get(text)
            .or_else(|| self.listed.get(unescape(key).as_ref()))
        {
            add(name);
        }
        for (pattern, name) in &self.patterns {
            if pattern.matches(text) {
                add(name);
            }
        }
        names
    }
}

/// The nets' classes and rules (`assign_nets`).
struct NetAssignment {
    /// Per net (as the router names it), its rules before the custom
    /// rules.
    connection_rules: BTreeMap<String, KiCadConnectionRoutingRules>,
    /// Per net of `net_facts`: its classes' diff pair width and gap (the
    /// first by priority that has one, else Default's) and the class.
    pair_classes: Vec<PairClass>,
    /// Per net with pads, in the board's order of names: its name and
    /// classes as the custom rules' conditions see them.
    net_facts: Vec<dru::NetFacts>,
    /// Per net of `net_facts`: its key (`key_and_text`).
    keys: Vec<String>,
}

/// A net's two names, made once from its name in the board (`raw`, the
/// atom's text): its key, as the lowering and every map of the router's
/// config name it (the board's spelling with its escapes, `A\"B`, less
/// the hierarchical slash, `NetNames::normalize`), and its text, as KiCad
/// shows it and the project's JSON and custom rules name it (`A"B`,
/// `unescape`). Until 2026-11-10 the custom rules' step looked the
/// rules up by the text: a net with a backslash or a quote panicked
/// (pinguz97_Zynq-SoM's `ZYNQ_PS_MIO8\VM1`) or missed its custom rules.
fn key_and_text(raw: &str, net_names: &NetNames) -> (String, String) {
    (
        net_names.normalize(raw).to_string(),
        unescape(raw).into_owned(),
    )
}

/// Gives every net with pads its classes, highest priority first
/// ("Default" for a net in none, reported in `net_classes`), and its
/// rules: its classes' (`Classes::resolve`), else `default_rules`.
fn assign_nets(
    nets: BTreeSet<String>,
    classes: &Classes,
    assignments: &Assignments,
    default_rules: &KiCadConnectionRoutingRules,
    net_names: &NetNames,
    report: &mut KiCadProjectRulesReport,
) -> NetAssignment {
    let mut connection_rules = BTreeMap::new();
    let mut pair_classes: Vec<PairClass> = Vec::new();
    let default_spec = classes.spec("Default");
    let mut net_facts = Vec::new();
    let mut keys = Vec::new();
    for raw in nets {
        let (key, text) = key_and_text(&raw, net_names);
        let names = assignments.class_names(&text, &key);
        let mut net_classes: Vec<&ClassSpec> =
            names.iter().filter_map(|name| classes.spec(name)).collect();
        net_classes.sort_by_key(|spec| spec.priority);
        let rules = if net_classes.is_empty() {
            default_rules.clone()
        } else {
            classes.resolve(&net_classes)
        };
        net_facts.push(dru::NetFacts {
            name: text,
            classes: if net_classes.is_empty() {
                vec!["Default".to_string()]
            } else {
                net_classes.iter().map(|spec| spec.name.clone()).collect()
            },
            diff_pair: None,
        });
        report.net_classes.insert(
            key.clone(),
            net_facts
                .last()
                .map(|net: &dru::NetFacts| net.classes.clone())
                .unwrap_or_default(),
        );
        let first = |field: fn(&ClassSpec) -> Option<f64>| {
            net_classes
                .iter()
                .find_map(|spec| field(spec))
                .or_else(|| default_spec.and_then(field))
        };
        pair_classes.push(PairClass {
            width: first(|spec| spec.dp_width),
            gap: first(|spec| spec.dp_gap),
            class: net_classes
                .first()
                .map_or_else(|| "Default".to_string(), |spec| spec.name.clone()),
        });
        connection_rules.insert(key.clone(), rules);
        keys.push(key);
    }
    NetAssignment {
        connection_rules,
        pair_classes,
        net_facts,
        keys,
    }
}

/// The board-wide widths and clearances, which the custom rules may raise
/// (`apply_lowered`).
struct BoardWide {
    edge_clearance: f64,
    hole_clearance: f64,
    hole_to_hole: f64,
    /// The neck-down width.
    neck: f64,
    /// The narrow-signal width: the smallest of the project's width menu,
    /// where it has one.
    narrow_signal: Option<f64>,
}

/// The board setup's edge, hole and hole-to-hole clearances, the neck-down
/// width (the board's minimum, else the narrowest class's within 0.1 to
/// 0.127 mm) and the narrow-signal width.
fn board_wide(
    project: &serde_json::Value,
    classes: &Classes,
    default_rules: &KiCadConnectionRoutingRules,
) -> BoardWide {
    let setup = classes.setup;
    // The narrowest width of the project's classes, used or not.
    let narrowest = classes
        .specs
        .iter()
        .map(|spec| classes.resolve(&[spec]).trace_width_mm)
        .chain(std::iter::once(default_rules.trace_width_mm))
        .fold(f64::INFINITY, f64::min);
    // The smallest width of the project's width menu, if it allows any
    // narrower than a class.
    let narrow_signal = project["board"]["design_settings"]["track_widths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_f64())
        .filter(|width| *width > 0.0 && *width >= setup.minimum("min_track_width"))
        .fold(f64::INFINITY, f64::min);
    let neck = if setup.minimum("min_track_width") > 0.0 {
        setup.minimum("min_track_width").max(0.1)
    } else {
        // (`narrowest` is a fold of `f64::min` from infinity: never NaN.)
        narrowest.clamp(0.1, 0.127)
    };
    BoardWide {
        edge_clearance: setup.minimum("min_copper_edge_clearance"),
        hole_clearance: setup.minimum("min_hole_clearance"),
        hole_to_hole: setup.minimum("min_hole_to_hole"),
        neck,
        narrow_signal: narrow_signal.is_finite().then_some(narrow_signal),
    }
}

/// The project's severities of KiCad's findings (`rule_severities`).
fn rule_severities(project: &serde_json::Value) -> BTreeMap<String, String> {
    project["board"]["design_settings"]["rule_severities"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
        .collect()
}

/// What the resolution has gathered for the router config by the custom
/// rules, which raise it (`apply_lowered`).
struct Resolved {
    connection_rules: BTreeMap<String, KiCadConnectionRoutingRules>,
    default_rules: KiCadConnectionRoutingRules,
    wide: BoardWide,
    net_rules: pcb_router::NetRules,
    /// The nets whose width a custom rule's opt gives on every layer.
    ruled_widths: BTreeSet<String>,
    report: KiCadProjectRulesReport,
}

/// The rules of the project's custom rules file, where it has one that
/// parses; the file is reported, and why it does not parse.
fn read_custom_rules(
    dru_path: &Path,
    report: &mut KiCadProjectRulesReport,
) -> Option<Vec<dru::Rule>> {
    let text = fs::read_to_string(dru_path).ok()?;
    report.custom_rules_file = dru_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    match dru::parse_rules(&text) {
        Err(error) => {
            report.unmodelled.push(format!(
                "the custom rules file does not parse ({error}): none of its rules is kept"
            ));
            report.custom_rules_error = Some(error);
            None
        }
        Ok(rules) => Some(rules),
    }
}

/// What the custom rules' lowering needs of the board: its nets with
/// pads (`net_facts`) and their rules so far (by their `keys`), its
/// copper layers and kinds of pad, the board-wide clearances and neck
/// width so far, its areas.
fn board_facts(
    net_facts: Vec<dru::NetFacts>,
    keys: &[String],
    pcb: &Expr,
    pad_kinds: BTreeSet<dru::Kind>,
    resolved: &Resolved,
) -> dru::BoardFacts {
    // Every net of `keys` has its rules (`assign_nets`); one without would
    // have Default's.
    let current = keys
        .iter()
        .map(|key| {
            resolved
                .connection_rules
                .get(key)
                .unwrap_or(&resolved.default_rules)
                .clone()
        })
        .collect();
    dru::BoardFacts {
        nets: net_facts,
        copper_layers: copper_layer_names(pcb),
        pad_kinds,
        current,
        board_clearances: [
            resolved.wide.edge_clearance,
            resolved.wide.hole_clearance,
            resolved.wide.hole_to_hole,
        ],
        neck_width: resolved.wide.neck,
        connection_floor_warnings: floor_mode() == FloorMode::ErrorsAndWarnings,
        areas: named_areas(pcb),
    }
}

/// Folds the lowered custom rules into the resolution: the nets' and the
/// default rules (`apply_custom`), the via minimums, the board-wide
/// clearances and widths, the router's per-net width floors, widths per
/// layer and pair clearances, and the report, each net by its key (`keys`,
/// one per net of `board`). `board_minimum`: the board's minimum track
/// width (with the connection width floor).
fn apply_lowered(
    resolved: &mut Resolved,
    lowered: dru::Lowered,
    board: &dru::BoardFacts,
    keys: &[String],
    board_minimum: f64,
) {
    let key = |index: usize| keys[index].clone();
    for (index, net) in keys.iter().enumerate() {
        if let Some(rules) = resolved.connection_rules.get_mut(net) {
            apply_custom(rules, &lowered, Some(index));
        }
    }
    apply_custom(&mut resolved.default_rules, &lowered, None);
    let report = &mut resolved.report;
    report.via_minimums = [
        report.via_minimums[0].max(lowered.via_diameter_min_all),
        report.via_minimums[1].max(lowered.via_drill_min_all),
        report.via_minimums[2].max(lowered.annulus_min_all),
    ];
    for index in 0..board.nets.len() {
        if lowered.width_opt[index].is_some() {
            resolved.ruled_widths.insert(key(index));
        }
    }
    let wide = &mut resolved.wide;
    wide.edge_clearance = wide.edge_clearance.max(lowered.edge_clearance);
    wide.hole_clearance = wide.hole_clearance.max(lowered.hole_clearance);
    wide.hole_to_hole = wide.hole_to_hole.max(lowered.hole_to_hole);
    wide.neck = wide.neck.max(lowered.width_min_all);
    wide.narrow_signal = wide
        .narrow_signal
        .map(|width| width.max(lowered.width_min_all));
    // A net's own minimum floors its neck-downs and its narrow
    // width (the classes are raised by `apply_custom`).
    for index in 0..board.nets.len() {
        let floor = lowered.width_min[index].max(lowered.width_min_all);
        if floor > wide.neck + 1.0e-9 {
            resolved.net_rules.width_floor.insert(key(index), floor);
        }
    }
    // Track widths per layer (rules on some layers only): per
    // net with one, its bounds on each layer, every-layer
    // bounds and the board's minimum widths folded in.
    for (&index, widths) in &lowered.rule_layer_widths {
        report.layer_track_widths.insert(
            key(index),
            widths
                .iter()
                .map(|(&layer, &width)| (board.copper_layers[layer].clone(), width))
                .collect(),
        );
    }
    if !lowered.layer_width_min.is_empty() {
        for index in 0..board.nets.len() {
            if let Some(widths) = layer_widths(&lowered, board, index, board_minimum) {
                resolved.net_rules.layer_widths.insert(key(index), widths);
            }
        }
    }
    let names = |nets: &Option<Vec<usize>>| {
        nets.as_ref()
            .map(|nets| nets.iter().map(|&index| key(index)).collect::<Vec<_>>())
    };
    for pair in &lowered.pair_clearances {
        resolved
            .net_rules
            .clearances
            .push(pcb_router::ClearanceRule {
                first: names(&pair.first),
                second: names(&pair.second),
                kinds: pair.kinds,
                layers: pair.layers,
                clearance: pair.clearance,
            });
    }
    report.pad_clearance_mm =
        (lowered.pad_clearance_all > 0.0).then_some(lowered.pad_clearance_all);
    report.custom_rules = lowered.report;
    report.unmodelled.extend(lowered.unmodelled);
}

/// A net's track width bounds on each copper layer (`index` into the
/// lowering's nets), where a custom rule on some layers only bounds it:
/// the per-layer bounds with the every-layer ones and `board_minimum`
/// folded in. `None` for a net no such rule bounds.
fn layer_widths(
    lowered: &dru::Lowered,
    board: &dru::BoardFacts,
    index: usize,
    board_minimum: f64,
) -> Option<Vec<pcb_router::LayerWidth>> {
    let ruled = (0..board.copper_layers.len()).any(|layer| {
        lowered.layer_width_min[layer][index] > 0.0
            || lowered.layer_width_max[layer][index].is_finite()
            || lowered.layer_width_opt[layer][index].is_some()
    });
    if !ruled {
        return None;
    }
    Some(
        (0..board.copper_layers.len())
            .map(|layer| pcb_router::LayerWidth {
                min: lowered.layer_width_min[layer][index]
                    .max(lowered.width_min[index])
                    .max(lowered.width_min_all)
                    .max(board_minimum),
                max: lowered.layer_width_max[layer][index].min(lowered.width_max[index]),
                opt: lowered.layer_width_opt[layer][index],
            })
            .collect(),
    )
}

/// The custom rules' track geometry on some copper layers only
/// (`KiCadProjectRulesReport::layer_geometry_rules`): a `track_width`,
/// `diff_pair_gap` or `diff_pair_width` constraint whose layer clause or
/// condition names a strict subset of the board's copper layers.
fn layer_geometry_rules(rules: &[dru::Rule], copper: &[String]) -> Vec<KiCadLayerGeometryRule> {
    let mut found = Vec::new();
    for rule in rules {
        let layers = dru::rule_copper_layers(rule, copper);
        if layers.is_empty() || layers.len() == copper.len() {
            continue;
        }
        // A geometry the rule aims at (an opt or a maximum); a minimum
        // alone is the process's floor on those layers (the 4in1-mini's
        // 2 oz outer copper), no impedance to order (trial 8: its release
        // asked for impedance control on Default).
        for constraint in rule.constraints.iter().filter(|constraint| {
            matches!(
                constraint.kind.as_str(),
                "track_width" | "diff_pair_gap" | "diff_pair_width"
            ) && (constraint.opt.is_some() || constraint.max.is_some())
        }) {
            found.push(KiCadLayerGeometryRule {
                rule: rule.name.clone(),
                constraint: constraint.kind.clone(),
                min_mm: constraint.min,
                opt_mm: constraint.opt,
                max_mm: constraint.max,
                layers: layers.clone(),
                condition: rule.condition.clone(),
            });
        }
    }
    found
}

/// A router config file over the project's rules. The project's rules are
/// the base and the file's keys replace them key by key: a file of
/// `{"seeds": 4}` routes by exactly the rules of `auto`, and one naming
/// `default_rules` keeps the project's edge, hole and mask clearances
/// unless it names them too. The class geometry is one key here:
/// `connection_rules` and `default_rules` together replace the project's
/// classes (the project gives every net with pads an entry, so a file's
/// `default_rules` would otherwise apply to no net). The custom rules'
/// per-net floors and pair clearances (`net_rules`) and the rules report
/// always come from the project. `project` is the project's resolution
/// (`resolve_project_rules`); when it failed, a file naming the class
/// geometry routes by its own keys and any other file fails with the
/// project's error.
pub fn router_config_over_project(
    file: serde_json::Value,
    project: Result<KiCadBoardRouterConfig, String>,
) -> Result<KiCadBoardRouterConfig, String> {
    let named = named_rule_keys(&file);
    let mut config = KiCadBoardRouterConfig::from_json(file)?;
    let names_classes = named.contains("connection_rules") || named.contains("default_rules");
    let project = match project {
        Ok(project) => project,
        Err(error) if !names_classes => return Err(error),
        Err(_) => {
            config.apply_width_floor();
            return Ok(config);
        }
    };
    // Widths and clearances below the project's: refused before any
    // work unless the file allows them, reported either way.
    let overrides = preflight_overrides(&named, &config, &project);
    if !overrides.is_empty() && !config.allow_below_project_minimum {
        return Err(crate::exit::input_error(format!(
            "the router config asks for less than the project's rules: {}; KiCad's DRC holds the board to the project's (change the project's board setup or net classes), or set \"allow_below_project_minimum\": true to route with them anyway (the verdict then warns)",
            overrides
                .iter()
                .map(KiCadRuleOverride::line)
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    let take = |key: &str| !named.contains(key);
    if !names_classes {
        config.connection_rules = project.connection_rules;
        config.default_rules = project.default_rules;
    }
    if take("edge_clearance_mm") {
        config.edge_clearance_mm = project.edge_clearance_mm;
    }
    if take("hole_clearance_mm") {
        config.hole_clearance_mm = project.hole_clearance_mm;
    }
    if take("hole_to_hole_clearance_mm") {
        config.hole_to_hole_clearance_mm = project.hole_to_hole_clearance_mm;
    }
    if take("neck_width_mm") {
        config.neck_width_mm = project.neck_width_mm;
    }
    if take("narrow_signal_mm") {
        config.narrow_signal_mm = project.narrow_signal_mm;
    }
    if take("relaxed_clearance_mm") {
        config.relaxed_clearance_mm = project.relaxed_clearance_mm;
    }
    if take("solder_mask_to_copper_mm") {
        config.solder_mask_to_copper_mm = project.solder_mask_to_copper_mm;
    }
    if take("solder_mask_bridge_severity") {
        config.solder_mask_bridge_severity = project.solder_mask_bridge_severity;
    }
    if take("min_resolved_spokes") {
        config.min_resolved_spokes = project.min_resolved_spokes;
    }
    if take("min_connection_mm") {
        config.min_connection_mm = project.min_connection_mm;
    }
    if take("connection_width_severity") {
        config.connection_width_severity = project.connection_width_severity;
    }
    config.project_rules = project.project_rules;
    if let Some(report) = config.project_rules.as_mut() {
        report.overrides = overrides;
    }
    config.net_rules = project.net_rules;
    config.diff_pair_rules = project.diff_pair_rules;
    config.length_rules = project.length_rules;
    config.track_disallowed = project.track_disallowed;
    config.ruled_widths = project.ruled_widths;
    // The width floor holds for a file's own classes too.
    config.apply_width_floor();
    Ok(config)
}

/// The keys a router config file names (`router_config_over_project`):
/// its top-level keys, or for an adaptive-routing config the rules its
/// first portfolio entry gives (`KiCadBoardRouterConfig::from_json`).
fn named_rule_keys(file: &serde_json::Value) -> BTreeSet<String> {
    if let Some(entry) = file
        .get("sequential")
        .and_then(|sequential| sequential.get("routing_portfolio"))
        .and_then(|portfolio| portfolio.get(0))
    {
        let mut named: std::collections::BTreeSet<String> = [
            "connection_rules",
            "edge_clearance_mm",
            "hole_to_hole_clearance_mm",
        ]
        .into_iter()
        .filter(|key| entry.get(key).is_some())
        .map(str::to_owned)
        .collect();
        if [
            "trace_width_mm",
            "clearance_mm",
            "via_size_mm",
            "via_drill_mm",
        ]
        .iter()
        .all(|key| entry.get(key).and_then(serde_json::Value::as_f64).is_some())
        {
            named.insert("default_rules".into());
        }
        return named;
    }
    file.as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
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

/// Which connection_width severities make `min_connection` (and a custom
/// `connection_width` rule) a floor on the router's widths
/// (`PCB_MIN_CONNECTION_FLOOR`: `off`, `errors` for error severity only;
/// anything else or unset: error and warning severity, since 2026-10-24).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloorMode {
    Off,
    Errors,
    ErrorsAndWarnings,
}

fn floor_mode() -> FloorMode {
    match std::env::var("PCB_MIN_CONNECTION_FLOOR").as_deref() {
        Ok("off") => FloorMode::Off,
        Ok("errors") => FloorMode::Errors,
        _ => FloorMode::ErrorsAndWarnings,
    }
}

/// The track width floor from the board setup's minimum connection width
/// and the project's severity of KiCad's connection_width findings. KiCad
/// checks only with a minimum above 0; a project without the severity
/// gets KiCad's default, a warning. By default error and warning severity
/// floor the widths (`FloorMode::ErrorsAndWarnings`, since 2026-10-24):
/// once the widths were per layer, OpenESC (0.16 over classes and inner
/// layers of 0.09, warning; the designer's tracks are never narrower than
/// 0.16) routed its inner layers at 0.09 and KiCad reported 225 warnings;
/// with the floor none, no error more, and the unconnected items moved
/// both ways (4in1-mini 39 to 53, 4in1 30 to 25 at 30 s of work; until
/// 2026-10-15's measurement the floor cost more, 52 to 89 and 104 to 121,
/// and only errors floored). `PCB_MIN_CONNECTION_FLOOR=errors` floors
/// errors only, `off` never; `ignore` never floors.
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

/// A net's classes' differential pair settings (`resolve_project_rules`).
struct PairClass {
    width: Option<f64>,
    gap: Option<f64>,
    class: String,
}

/// KiCad's differential pair of a net by its name (`DRC_ENGINE::
/// MatchDpSuffix`): past trailing digits and underscores, a `+` or `P`
/// (positive) or `-` or `N` (negative); returns the complementary net's
/// name, the base name (up to the suffix) and whether the net is the
/// positive one. `D+` pairs with `D-`, `USB_DP` with `USB_DN`, `TX_P_1`
/// with `TX_N_1`; `USB_DM` is no pair's.
pub(crate) fn diff_pair_of(name: &str) -> Option<(String, String, bool)> {
    let characters: Vec<char> = name.chars().collect();
    for (from_end, character) in characters.iter().rev().enumerate() {
        let (complement, positive) = match character {
            '0'..='9' | '_' => continue,
            '+' => ('-', true),
            '-' => ('+', false),
            'P' => ('N', true),
            'N' => ('P', false),
            _ => return None,
        };
        let at = characters.len() - 1 - from_end;
        let base: String = characters[..at].iter().collect();
        let tail: String = characters[at + 1..].iter().collect();
        return Some((format!("{base}{complement}{tail}"), base, positive));
    }
    None
}

/// What the project gives the nets of differential pairs (both nets
/// present on the board): per copper layer the width (the custom rules'
/// track_width for the pair's track there, else the class's
/// diff_pair_width, else the net's width) and the gap (the rules'
/// diff_pair_gap, else the class's diff_pair_gap, else its clearance), and
/// the rules' diff_pair_uncoupled maximum. A rule's opt is taken, else the
/// class value held inside its min and max. The gap is never below what
/// KiCad's clearance check lets the two nets come (the class's diff pair
/// gap where narrower than its clearance, else the clearance).
fn diff_pair_rules(
    facts: &[dru::NetFacts],
    keys: &[String],
    classes: &[PairClass],
    rules: &[dru::Rule],
    copper: &[String],
    connection: impl Fn(&str) -> Option<KiCadConnectionRoutingRules>,
    min_width: f64,
) -> BTreeMap<String, crate::board_router::KiCadDiffPairRule> {
    let names = keys_by_name(facts, keys);
    let pick = |picked: &Option<dru::Picked>, base: f64| match picked {
        Some(picked) => picked.opt.unwrap_or_else(|| {
            base.max(picked.min.unwrap_or(0.0))
                .min(picked.max.unwrap_or(f64::INFINITY))
        }),
        None => base,
    };
    let mut out = BTreeMap::new();
    for (index, (fact, key)) in facts.iter().zip(keys).enumerate() {
        let Some((partner, base, _)) = diff_pair_of(&fact.name) else {
            continue;
        };
        let Some(&partner) = names.get(partner.as_str()) else {
            continue;
        };
        let Some(current) = connection(key) else {
            continue;
        };
        let class = &classes[index];
        let mut paired = fact.clone();
        paired.diff_pair = Some(Some(base));
        let mut rule = crate::board_router::KiCadDiffPairRule {
            partner: partner.to_string(),
            ..Default::default()
        };
        // KiCad's P to N clearance: the class's diff pair gap where it is
        // narrower than the clearance (its implicit rule), else the
        // clearance.
        let pair_clearance = match class.gap {
            Some(gap) if gap < current.clearance_mm => gap,
            _ => current.clearance_mm,
        };
        for layer in copper {
            let width = dru::track_constraint(rules, &paired, layer, copper, "track_width");
            let gap = dru::track_constraint(rules, &paired, layer, copper, "diff_pair_gap");
            let uncoupled =
                dru::track_constraint(rules, &paired, layer, copper, "diff_pair_uncoupled");
            // KiCad's `skew` (the pair's nets' lengths apart) and `length`
            // (each net's): the skew pass matches them (docs/router.md,
            // "Skew and length").
            let skew = dru::track_constraint(rules, &paired, layer, copper, "skew");
            let length = dru::track_constraint(rules, &paired, layer, copper, "length");
            if let Some(max) = skew.as_ref().and_then(|picked| picked.max) {
                rule.max_skew = Some(rule.max_skew.map_or(max, |known: f64| known.min(max)));
            }
            if let Some(picked) = &length
                && rule.length.is_none()
            {
                rule.length = Some((
                    picked.min.unwrap_or(0.0),
                    picked.opt,
                    picked.max.unwrap_or(f64::INFINITY),
                ));
            }
            let w = pick(&width, class.width.unwrap_or(current.trace_width_mm)).max(min_width);
            let g = pick(&gap, class.gap.unwrap_or(current.clearance_mm)).max(pair_clearance);
            rule.layers.push((w, g));
            rule.disallowed
                .push(dru::track_disallowed(rules, &paired, layer, copper));
            rule.width_ruled.push(width.is_some());
            rule.width_bounds
                .push(width.as_ref().map_or((0.0, f64::INFINITY), |picked| {
                    (
                        picked.min.unwrap_or(0.0),
                        picked.max.unwrap_or(f64::INFINITY),
                    )
                }));
            rule.sources.push(match (&width, &gap) {
                (_, Some(picked)) | (Some(picked), None) => format!("rule '{}'", picked.rule),
                (None, None) if class.gap.is_some() => format!("net class '{}'", class.class),
                _ => "net class clearance".into(),
            });
            for picked in [&width, &gap, &uncoupled, &skew, &length]
                .into_iter()
                .flatten()
            {
                if !rule.rules.contains(&picked.rule) {
                    rule.rules.push(picked.rule.clone());
                }
            }
            if rule.asked.is_none() {
                rule.asked = [("diff_pair_gap", &gap), ("diff_pair_uncoupled", &uncoupled)]
                    .into_iter()
                    .find_map(|(kind, picked)| {
                        picked
                            .as_ref()
                            .map(|picked| format!("rule '{}' constrains {kind}", picked.rule))
                    });
            }
            if let Some(max) = uncoupled.as_ref().and_then(|picked| picked.max) {
                rule.max_uncoupled =
                    Some(rule.max_uncoupled.map_or(max, |known: f64| known.min(max)));
            }
        }
        out.insert(key.clone(), rule);
    }
    out
}

/// Per net of `facts` by its name (as KiCad shows it), its key (`keys`,
/// one per net): KiCad pairs nets by their names, the router by keys.
fn keys_by_name<'a>(facts: &'a [dru::NetFacts], keys: &'a [String]) -> BTreeMap<&'a str, &'a str> {
    facts
        .iter()
        .zip(keys)
        .map(|(fact, key)| (fact.name.as_str(), key.as_str()))
        .collect()
}

/// Per net with one (by its key, `keys`), the copper layers (bits in the
/// router's order, `copper`) where a custom rule surely disallows its
/// tracks (`(constraint disallow track)`, `dru::track_disallowed`; KiCad's
/// `inDiffPair()` known, as for the pairs' own layers): every rung of the
/// ladder keeps the net's tracks off them (`RuleClass::layers`). A rule
/// that closes a layer for some net is reported `applied` (before, every
/// `disallow track` was `unmodelled`: only the exclusive planes and the
/// pairs' classes kept nets off such layers).
fn track_disallowed(
    facts: &[dru::NetFacts],
    keys: &[String],
    rules: &[dru::Rule],
    copper: &[String],
    severities: &BTreeMap<String, String>,
    report: &mut KiCadProjectRulesReport,
) -> BTreeMap<String, pcb_router::LayerMask> {
    let mut out = BTreeMap::new();
    let track_rules: Vec<(usize, &dru::Rule)> = rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| {
            rule.constraints.iter().any(|constraint| {
                constraint.kind == "disallow" && constraint.words.iter().any(|word| word == "track")
            })
        })
        // KiCad's items_not_allowed: only a rule whose findings are errors
        // (its own severity, else the project's) closes a layer, as the
        // other rules' lowering skips those below (OpenAIO-Whoop's "In1
        // and In6: GND planes, no tracks" is a warning).
        .filter(|(_, rule)| {
            rule.severity
                .clone()
                .unwrap_or_else(|| dru::finding_severity("disallow", severities))
                == "error"
        })
        .collect();
    if track_rules.is_empty() {
        return out;
    }
    let names: BTreeSet<&str> = facts.iter().map(|net| net.name.as_str()).collect();
    // Per rule (index): the nets it closes some layer for.
    let mut closing: BTreeMap<usize, usize> = BTreeMap::new();
    for (fact, key) in facts.iter().zip(keys) {
        let mut known = fact.clone();
        known.diff_pair = Some(
            diff_pair_of(&fact.name)
                .filter(|(partner, ..)| names.contains(partner.as_str()))
                .map(|(_, base, _)| base),
        );
        let mut mask: pcb_router::LayerMask = 0;
        let mut by: BTreeSet<usize> = BTreeSet::new();
        for (layer, name) in copper.iter().enumerate().take(32) {
            for &(index, rule) in &track_rules {
                if dru::track_disallowed(std::slice::from_ref(rule), &known, name, copper) {
                    mask |= 1 << layer;
                    by.insert(index);
                }
            }
        }
        for index in by {
            *closing.entry(index).or_default() += 1;
        }
        if mask != 0 {
            out.insert(key.clone(), mask);
        }
    }
    for (index, nets) in closing {
        let rule = &rules[index];
        let prefix = format!("disallow ('{}'", rule.name);
        for entry in &mut report.custom_rules {
            if entry.rule == rule.name
                && entry.constraint == "disallow"
                && entry.status == "unmodelled"
            {
                entry.status = "applied".into();
                entry.detail = format!(
                    "tracks of {nets} net{} kept off the layers it disallows (the class's layers, every rung)",
                    if nets == 1 { "" } else { "s" }
                );
            }
        }
        report.unmodelled.retain(|line| !line.starts_with(&prefix));
    }
    out
}

/// The custom rules' `length` and `skew` constraints with the nets they
/// hold for (docs/router.md, "Length and skew rules"): per net and copper
/// layer the rule KiCad applies to a track of the net there (the last
/// whose condition surely holds, `inDiffPair()` known), grouped by the
/// rule as KiCad's matched-length check groups them; a through via is
/// the rule's where it holds on F.Cu (KiCad evaluates a via's rules on
/// its first layer). A rule (or the project) that rates the findings
/// below an error is passed over, as is a constraint without a bound in
/// lengths (KiCad 10's time-domain `ps`).
fn length_rules(
    facts: &[dru::NetFacts],
    keys: &[String],
    rules: &[dru::Rule],
    copper: &[String],
    severities: &BTreeMap<String, String>,
) -> Vec<crate::board_router::KiCadLengthRule> {
    use crate::board_router::{KiCadLengthMember, KiCadLengthRule};
    let names: BTreeSet<&str> = facts.iter().map(|net| net.name.as_str()).collect();
    let mut groups: BTreeMap<(usize, &str), KiCadLengthRule> = BTreeMap::new();
    for kind in ["length", "skew"] {
        if dru::finding_severity(kind, severities) != "error" {
            continue;
        }
        for (fact, key) in facts.iter().zip(keys) {
            let mut known = fact.clone();
            known.diff_pair = Some(
                diff_pair_of(&fact.name)
                    .filter(|(partner, ..)| names.contains(partner.as_str()))
                    .map(|(_, base, _)| base),
            );
            for layer in copper {
                let Some((index, picked)) = dru::track_rule(rules, &known, layer, copper, kind)
                else {
                    continue;
                };
                let rule = &rules[index];
                if rule.severity.as_deref().is_some_and(|own| own != "error")
                    || (picked.min.is_none() && picked.max.is_none())
                {
                    continue;
                }
                let group = groups
                    .entry((index, kind))
                    .or_insert_with(|| KiCadLengthRule {
                        rule: picked.rule.clone(),
                        constraint: kind.to_string(),
                        min: picked.min,
                        opt: picked.opt,
                        max: picked.max,
                        within_diff_pairs: rule
                            .constraints
                            .iter()
                            .rev()
                            .find(|constraint| constraint.kind == kind)
                            .is_some_and(|constraint| {
                                constraint
                                    .words
                                    .iter()
                                    .any(|word| word == "within_diff_pairs")
                            }),
                        nets: Vec::new(),
                    });
                let net = key.clone();
                if group.nets.last().is_none_or(|member| member.net != net) {
                    group.nets.push(KiCadLengthMember {
                        net,
                        layers: Vec::new(),
                        vias: false,
                    });
                }
                let member = group.nets.last_mut().expect("a member");
                member.layers.push(layer.clone());
                member.vias |= layer == "F.Cu";
            }
        }
    }
    groups.into_values().collect()
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
        // At warning severity the default floors too (since 2026-10-24).
        if std::env::var_os("PCB_MIN_CONNECTION_FLOOR").is_none() {
            write("warning");
            let config = resolve_project_rules(&project, &pcb).unwrap();
            assert_eq!(config.min_connection_mm, Some(0.16));
            assert_eq!(config.connection_width_severity.as_deref(), Some("warning"));
            assert_eq!(config.connection_rules["A"].trace_width_mm, 0.16);
            assert_eq!(config.neck_width_mm, Some(0.16));
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
    fn differential_pairs_are_named_as_kicad_names_them() {
        let pair = diff_pair_of;
        assert_eq!(pair("D+"), Some(("D-".into(), "D".into(), true)));
        assert_eq!(
            pair("/USB_D-"),
            Some(("/USB_D+".into(), "/USB_D".into(), false))
        );
        assert_eq!(
            pair("USB_DP"),
            Some(("USB_DN".into(), "USB_D".into(), true))
        );
        assert_eq!(pair("TX_P_1"), Some(("TX_N_1".into(), "TX_".into(), true)));
        assert_eq!(pair("CLKN"), Some(("CLKP".into(), "CLK".into(), false)));
        // KiCad pairs no `DM`, and a supply is no pair's.
        assert_eq!(pair("USB_DM"), None);
        assert_eq!(pair("+3V3"), None);
        assert_eq!(pair("GND"), None);
    }

    #[test]
    fn length_and_skew_rules_group_their_nets_by_rule_and_layer() {
        let pcb = r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (footprint "a" (at 0 0)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/D0"))
                (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net "/D1"))
                (pad "3" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "/CLK"))
                (pad "4" smd rect (at 3 0) (size 1 1) (layers "F.Cu") (net "/TIME"))
                (pad "5" smd rect (at 4 0) (size 1 1) (layers "F.Cu") (net "GND"))))"#;
        let project = r#"{"net_settings": {"classes": [
              {"name": "Default", "clearance": 0.15, "track_width": 0.25, "via_diameter": 0.6, "via_drill": 0.3},
              {"name": "BUS", "clearance": 0.15, "track_width": 0.2}],
              "netclass_patterns": [{"pattern": "/D*", "netclass": "BUS"}, {"pattern": "/CLK", "netclass": "BUS"}]}}"#;
        let dru = r#"(version 1)
            (rule "bus length" (condition "A.NetClass == 'BUS'")
              (constraint length (min 25mm) (max 27mm)))
            (rule "bus skew inner" (condition "A.NetClass == 'BUS' && A.Layer == 'In1.Cu'")
              (constraint skew (max 0.5mm)))
            (rule "time" (condition "A.NetName == '/TIME'")
              (constraint length (min 450ps) (max 550ps)))"#;
        let (directory, path) = project_directory("lengths", project, Some(dru));
        let board = directory.join("board.kicad_pcb");
        fs::write(&board, pcb).unwrap();
        let config = resolve_project_rules(&path, &board).unwrap();
        let rules = &config.length_rules;
        assert_eq!(rules.len(), 2, "{rules:?}");
        let length = rules
            .iter()
            .find(|rule| rule.constraint == "length")
            .unwrap();
        assert_eq!(length.rule, "bus length");
        assert_eq!((length.min, length.max), (Some(25.0), Some(27.0)));
        assert_eq!(
            length
                .nets
                .iter()
                .map(|member| member.net.as_str())
                .collect::<Vec<_>>(),
            ["CLK", "D0", "D1"]
        );
        assert!(
            length
                .nets
                .iter()
                .all(|member| member.vias && member.layers.len() == 3)
        );
        // The skew rule holds on In1.Cu only: its nets' tracks there, no
        // via (KiCad evaluates a via's rules on F.Cu).
        let skew = rules.iter().find(|rule| rule.constraint == "skew").unwrap();
        assert_eq!(skew.max, Some(0.5));
        assert!(
            skew.nets
                .iter()
                .all(|member| member.layers == ["In1.Cu"] && !member.vias)
        );
        // A time-domain bound is no length: still unmodelled. The others
        // are reported kept once length matching is on.
        let report = config.project_rules.as_ref().unwrap();
        assert!(report.unmodelled.iter().any(|line| line.contains("'time'")));
        assert_eq!(report.unmodelled.len(), 3, "{:?}", report.unmodelled);
        let kept = crate::board_router::with_lengths(&config);
        let report = kept.project_rules.as_ref().unwrap();
        let status = |rule: &str| {
            report
                .custom_rules
                .iter()
                .find(|entry| entry.rule == rule)
                .map(|entry| entry.status.clone())
        };
        assert_eq!(status("bus length").as_deref(), Some("approximated"));
        assert_eq!(status("bus skew inner").as_deref(), Some("applied"));
        assert_eq!(status("time").as_deref(), Some("unmodelled"));
        assert_eq!(report.unmodelled.len(), 1, "{:?}", report.unmodelled);
        let off = crate::board_router::with_lengths(&KiCadBoardRouterConfig {
            length_matching: Some(false),
            ..config.clone()
        });
        assert_eq!(off.project_rules.as_ref().unwrap().unmodelled.len(), 3);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_disallow_track_rule_closes_its_layers_for_the_nets_it_holds_for() {
        // ducktop2's "solid ground layers" and "controlled-impedance signal
        // layers": no track but GND's on In1.Cu, and the DIFF class's off
        // In1.Cu and In2.Cu (bits 1 and 2).
        let pcb = r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "In2.Cu" signal) (3 "B.Cu" signal) (25 "Edge.Cuts" user))
              (footprint "a" (at 0 0)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND"))
                (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net "/SIG"))
                (pad "3" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "/TX_P"))
                (pad "4" smd rect (at 3 0) (size 1 1) (layers "F.Cu") (net "/TX_N"))
                (pad "5" smd rect (at 4 0) (size 1 1) (layers "F.Cu") (net "/AREA"))))"#;
        let project = r#"{"net_settings": {"classes": [
              {"name": "Default", "clearance": 0.15, "track_width": 0.25, "via_diameter": 0.6, "via_drill": 0.3},
              {"name": "DIFF", "clearance": 0.15, "track_width": 0.2}],
              "netclass_patterns": [{"pattern": "/TX_*", "netclass": "DIFF"}]}}"#;
        let dru = r#"(version 1)
            (rule "solid ground layers" (condition "A.Layer == 'In1.Cu' && A.NetName != 'GND'")
              (constraint disallow track))
            (rule "impedance layers" (condition "A.hasNetclass('DIFF') && A.Layer != 'F.Cu' && A.Layer != 'B.Cu'")
              (constraint disallow track))
            (rule "inside an area" (condition "A.insideArea('X') && A.NetName == '/AREA'")
              (constraint disallow track))
            (rule "a warning" (severity warning) (condition "A.Layer == 'In2.Cu' && A.NetName == '/SIG'")
              (constraint disallow track))"#;
        let (directory, path) = project_directory("disallow", project, Some(dru));
        let board = directory.join("board.kicad_pcb");
        fs::write(&board, pcb).unwrap();
        let config = resolve_project_rules(&path, &board).unwrap();
        let closed = &config.track_disallowed;
        assert_eq!(closed.get("GND"), None);
        // A rule below an error closes nothing (In2.Cu stays open to /SIG).
        assert_eq!(closed.get("SIG"), Some(&0b10));
        assert_eq!(closed.get("TX_P"), Some(&0b110));
        assert_eq!(closed.get("TX_N"), Some(&0b110));
        // A condition the lowering cannot decide closes nothing more: /AREA
        // keeps only the ground rule's In1.Cu.
        assert_eq!(closed.get("AREA"), Some(&0b10));
        let report = config.project_rules.as_ref().unwrap();
        let status = |rule: &str| {
            report
                .custom_rules
                .iter()
                .find(|entry| entry.rule == rule)
                .map(|entry| entry.status.clone())
        };
        assert_eq!(status("solid ground layers").as_deref(), Some("applied"));
        assert_eq!(status("impedance layers").as_deref(), Some("applied"));
        assert_eq!(status("inside an area").as_deref(), Some("unmodelled"));
        assert!(
            report
                .unmodelled
                .iter()
                .all(|line| !line.contains("ground layers") && !line.contains("impedance")),
            "{:?}",
            report.unmodelled
        );
        // Read again under a router config file.
        let over = router_config_over_project(serde_json::json!({"seeds": 2}), Ok(config.clone()))
            .unwrap();
        assert_eq!(&over.track_disallowed, closed);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_pairs_width_and_gap_come_from_the_rules_else_its_class() {
        let pcb = r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (footprint "a" (at 0 0)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/TX_P"))
                (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net "/TX_N"))
                (pad "3" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "D+"))
                (pad "4" smd rect (at 3 0) (size 1 1) (layers "F.Cu") (net "D-"))
                (pad "5" smd rect (at 4 0) (size 1 1) (layers "F.Cu") (net "RESET_N"))))"#;
        let project = r#"{"board": {"design_settings": {"rules": {"min_track_width": 0.1}}},
              "net_settings": {"classes": [
              {"name": "Default", "clearance": 0.15, "track_width": 0.25, "via_diameter": 0.6, "via_drill": 0.3, "diff_pair_width": 0.2, "diff_pair_gap": 0.18},
              {"name": "HS", "clearance": 0.15, "track_width": 0.2, "diff_pair_width": 0.16, "diff_pair_gap": 0.1}],
              "netclass_patterns": [{"pattern": "/TX_*", "netclass": "HS"}]}}"#;
        let dru = r#"(version 1)
            (rule "pairs outer" (layer outer) (condition "A.inDiffPair('/TX')")
              (constraint track_width (min 0.17mm) (max 0.17mm))
              (constraint diff_pair_gap (min 0.12mm) (opt 0.13mm) (max 0.14mm))
              (constraint diff_pair_uncoupled (max 5mm)))
            (rule "ground layer" (layer "In1.Cu") (condition "A.NetName != 'GND'")
              (constraint disallow track))"#;
        let (directory, path) = project_directory("diff-pairs", project, Some(dru));
        let board = directory.join("board.kicad_pcb");
        fs::write(&board, pcb).unwrap();
        let config = resolve_project_rules(&path, &board).unwrap();
        let rules = &config.diff_pair_rules;
        // Both nets of a pair, keyed as the lowering names them (the
        // sheet's slash dropped where no other net takes the name);
        // RESET_N has no RESET_P.
        assert_eq!(
            rules.keys().map(String::as_str).collect::<Vec<_>>(),
            ["D+", "D-", "TX_N", "TX_P"]
        );
        let tx = &rules["TX_P"];
        assert_eq!(tx.partner, "TX_N");
        // The rule on the outer layers (its min and max hold the class's
        // 0.16 at 0.17; its opt gap), the class on In1.Cu (its gap below
        // the clearance: KiCad's implicit pair rule lets the nets come
        // that close).
        assert_eq!(tx.layers, vec![(0.17, 0.13), (0.16, 0.1), (0.17, 0.13)]);
        assert_eq!(tx.max_uncoupled, Some(5.0));
        // No track but GND's on In1.Cu: the pairs keep off it.
        assert_eq!(tx.disallowed, vec![false, true, false]);
        // The rule's width bounds outside, none on In1.Cu.
        assert_eq!(tx.width_bounds[0], (0.17, 0.17));
        assert_eq!(tx.width_bounds[1], (0.0, f64::INFINITY));
        assert_eq!(tx.rules, vec!["pairs outer".to_string()]);
        assert_eq!(tx.sources[1], "net class 'HS'");
        // Default's pair settings for D+ and D-; the rule's pattern does
        // not take them.
        assert_eq!(rules["D-"].layers, vec![(0.2, 0.18); 3]);
        // Without the switch, the rule stays unmodelled in the report.
        let report = config.project_rules.as_ref().unwrap();
        assert!(
            report
                .custom_rules
                .iter()
                .any(|rule| rule.constraint == "diff_pair_gap" && rule.status == "unmodelled")
        );
        let paired = crate::board_router::with_pairs(&config);
        let report = paired.project_rules.as_ref().unwrap();
        assert!(
            report
                .custom_rules
                .iter()
                .filter(|rule| rule.rule == "pairs outer")
                .all(|rule| rule.status == "applied")
        );
        assert!(
            !report
                .unmodelled
                .iter()
                .any(|line| line.contains("pairs outer"))
        );
        // The pair's nets route at the pair's width.
        assert_eq!(paired.connection_rules["TX_P"].trace_width_mm, 0.17);
        // The rule's diff_pair_gap asks for the pair coupled; the class's
        // pair settings do not (D+, D-).
        assert_eq!(
            tx.asked.as_deref(),
            Some("rule 'pairs outer' constrains diff_pair_gap")
        );
        assert_eq!(rules["D-"].asked, None);
        // Route mode's default by need (the environment unset), and the
        // explicit setting over it.
        if std::env::var_os("PCB_ROUTER_DIFF_PAIRS").is_none() {
            assert!(!config.diff_pairs_on());
            let mut route = config.clone();
            route.diff_pairs_by_need = true;
            let choice = route.diff_pairs_choice();
            assert!(choice.on);
            assert_eq!(choice.default, "rules");
            assert!(choice.reason.contains("pairs outer"), "{}", choice.reason);
            route.diff_pairs = Some(false);
            assert_eq!(route.diff_pairs_choice().default, "explicit");
            assert!(!route.diff_pairs_on());
            // Class settings alone leave it off.
            route.diff_pairs = None;
            route.diff_pair_rules.retain(|net, _| net.starts_with('D'));
            let choice = route.diff_pairs_choice();
            assert!(!choice.on);
            assert_eq!(choice.default, "off");
        }
        fs::remove_dir_all(&directory).unwrap();
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
    fn escaped_net_names_meet_the_projects_and_a_pattern_cannot_escape_its_anchors() {
        // A net `A"B` is `"A\"B"` in the board and `A"B` in the project's
        // JSON and the custom rules: until 2026-10-21 it fell to Default
        // without a word. `x)|(?:.*` closed the anchoring group and put
        // every net in its class.
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-rules-escapes-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source = r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (footprint "a" (at 0 0)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "A\"B"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "C\\D"))
                (pad "3" smd rect (at 4 0) (size 1 1) (layers "F.Cu") (net "GND"))))"#;
        let pcb = directory.join("board.kicad_pcb");
        fs::write(&pcb, source).unwrap();
        let project = directory.join("board.kicad_pro");
        fs::write(
            &project,
            r#"{"net_settings": {
                  "classes": [
                    {"name": "Default", "clearance": 0.14, "track_width": 0.25, "via_diameter": 0.7, "via_drill": 0.3},
                    {"name": "QUOTE", "clearance": 0.2, "track_width": 0.5, "priority": 1},
                    {"name": "SLASH", "clearance": 0.2, "track_width": 0.6, "priority": 2},
                    {"name": "ALL", "clearance": 0.3, "track_width": 0.9, "priority": 0}],
                  "netclass_assignments": {"A\"B": "QUOTE"},
                  "netclass_patterns": [
                    {"netclass": "SLASH", "pattern": "C\\\\D"},
                    {"netclass": "ALL", "pattern": "x)|(?:.*"}]}}"#,
        )
        .unwrap();
        crate::note_net_names(&parse(source).unwrap());
        let config = resolve_project_rules(&project, &pcb).unwrap();
        let width = |net: &str| config.connection_rules[normalize_net(net)].trace_width_mm;
        assert_eq!(width(r#"A\"B"#), 0.5);
        assert_eq!(width(r"C\\D"), 0.6);
        assert_eq!(width("GND"), 0.25);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn custom_rules_reach_nets_with_a_quote_or_a_backslash() {
        // The board spells the nets `A\"B`, `/C\\D` and `E\"_P`/`E\"_N`,
        // the custom rules name their text (`A"B`, `/C\D`): until
        // 2026-11-10 the rules' step looked the router's rules up by the
        // text and panicked (pinguz97_Zynq-SoM's `ZYNQ_PS_MIO8\\VM1`).
        let directory = std::env::temp_dir().join(format!(
            "pcb-maker-rules-dru-escapes-{}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let source = r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (footprint "a" (at 0 0)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "A\"B"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "/C\\D"))
                (pad "3" smd rect (at 4 0) (size 1 1) (layers "F.Cu") (net "E\"_P"))
                (pad "4" smd rect (at 6 0) (size 1 1) (layers "F.Cu") (net "E\"_N"))
                (pad "5" smd rect (at 8 0) (size 1 1) (layers "F.Cu") (net "GND"))))"#;
        let pcb = directory.join("board.kicad_pcb");
        fs::write(&pcb, source).unwrap();
        let project = directory.join("board.kicad_pro");
        fs::write(
            &project,
            r#"{"net_settings": {"classes": [
                  {"name": "Default", "clearance": 0.15, "track_width": 0.2, "via_diameter": 0.6, "via_drill": 0.3}]}}"#,
        )
        .unwrap();
        fs::write(
            directory.join("board.kicad_dru"),
            r#"(version 1)
(rule "quote" (condition "A.NetName == 'A\"B'") (constraint track_width (min 0.4mm)) (constraint clearance (min 0.3mm)))
(rule "backslash" (condition "A.NetName == '/C\\\\D'") (constraint track_width (min 0.5mm)))
(rule "pair" (condition "A.NetName == 'E\"_P' || A.NetName == 'E\"_N'") (constraint diff_pair_gap (opt 0.35mm)))"#,
        )
        .unwrap();
        let config = resolve_project_rules(&project, &pcb).unwrap();
        let rules = |net: &str| config.connection_rules[net].clone();
        assert_eq!(
            (
                rules(r#"A\"B"#).trace_width_mm,
                rules(r#"A\"B"#).clearance_mm
            ),
            (0.4, 0.3)
        );
        assert_eq!(rules(r"C\\D").trace_width_mm, 0.5);
        assert_eq!(rules("GND").trace_width_mm, 0.2);
        let floors = &config.net_rules.as_ref().unwrap().width_floor;
        assert_eq!(
            *floors,
            [(r#"A\"B"#.to_string(), 0.4), (r"C\\D".to_string(), 0.5)].into()
        );
        let pair = &config.diff_pair_rules[r#"E\"_P"#];
        assert_eq!(pair.partner, r#"E\"_N"#);
        assert_eq!(pair.layers[0].1, 0.35);
        let report = config.project_rules.unwrap();
        let status: Vec<(&str, &str)> = report
            .custom_rules
            .iter()
            .map(|rule| (rule.rule.as_str(), rule.status.as_str()))
            .collect();
        // A pair's gap is the pair's rule above (`diff_pair_rules`), not
        // the lowering's.
        assert_eq!(
            status,
            [
                ("quote", "applied"),
                ("quote", "applied"),
                ("backslash", "applied"),
                ("pair", "unmodelled")
            ]
        );
        assert_eq!(
            report.net_classes.keys().collect::<Vec<_>>(),
            [r#"A\"B"#, r"C\\D", r#"E\"_N"#, r#"E\"_P"#, "GND"]
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_hierarchical_net_beside_a_bare_one_keeps_its_class() {
        // meshsat's pcb-d: pads on "/VIN_CELL", a bare "VIN_CELL" in the
        // net table; the project puts "/VIN_CELL" in CELL.
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-rules-bare-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source = r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (net 0 "") (net 1 "/VIN_CELL") (net 2 "VIN_CELL")
              (footprint "a" (at 0 0)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "/VIN_CELL"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net 1 "/VIN_CELL"))))"#;
        let pcb = directory.join("board.kicad_pcb");
        fs::write(&pcb, source).unwrap();
        let project = directory.join("board.kicad_pro");
        fs::write(
            &project,
            r#"{"net_settings": {
                  "classes": [
                    {"name": "Default", "clearance": 0.14, "track_width": 0.25, "via_diameter": 0.7, "via_drill": 0.3},
                    {"name": "CELL", "clearance": 0.15, "track_width": 0.8, "via_diameter": 1.0, "via_drill": 0.5, "priority": 2}],
                  "netclass_patterns": [{"netclass": "CELL", "pattern": "/VIN_CELL"}]}}"#,
        )
        .unwrap();
        // Another board's names on this thread before (the bare set is
        // per thread).
        crate::note_net_names(&parse("(kicad_pcb)").unwrap());
        let config = resolve_project_rules(&project, &pcb).unwrap();
        // The key the lowering asks for.
        crate::note_net_names(&parse(source).unwrap());
        let rules = &config.connection_rules[normalize_net("/VIN_CELL")];
        assert_eq!((rules.clearance_mm, rules.trace_width_mm), (0.15, 0.8));
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
                // No area named ANT on the board: KiCad's area test is
                // false (dru::resolve_areas).
                ("Keepout", "no_effect"),
                ("Silk", "not_routing"),
            ]
        );
        assert_eq!(report.unmodelled.len(), 1);
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
        // None of them holds between the pads of every two nets: the
        // placer keeps the classes' largest.
        assert_eq!(report.pad_clearance_mm, None);
        assert_eq!(crate::board_placer::largest_clearance(&config), 0.2);
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
        use pcb_router::rules::{ALL_KINDS, PAD, PLATED, TRACK, VIA, kind_bit};
        let pads = kind_bit(TRACK, PAD) | kind_bit(TRACK, PLATED);
        for rule in &isolation {
            assert_eq!(rule.kinds, ALL_KINDS & !pads & !kind_bit(VIA, VIA));
        }
        let pad = net_rules
            .clearances
            .iter()
            .find(|rule| rule.clearance == 0.2)
            .unwrap();
        assert_eq!((&pad.first, &pad.second), (&None, &None));
        assert_eq!(pad.kinds, pads);
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

    #[test]
    fn a_clearance_on_the_outer_layers_spaces_the_parts() {
        // OpenESC's 4in1 boards (harvested with their .kicad_dru since
        // 2026-10-24): the board's 0.09 mm, and 0.16 mm on the outer
        // layers by a custom rule. The router keeps it as a pair rule; the
        // placer read only the classes and packed pads 0.09 mm apart
        // (4in1-mini: 170 clearance errors between pads of two parts).
        let project = r#"{"board": {"design_settings": {"rules": {"min_clearance": 0.09, "min_track_width": 0.09}}},
                "net_settings": {"classes": [{"name": "Default", "clearance": 0.09, "track_width": 0.09, "via_diameter": 0.35, "via_drill": 0.2}]}}"#;
        let (directory, path) = project_directory(
            "outer",
            project,
            Some(
                r#"(version 1)
(rule "silkscreen over pad" (constraint silk_clearance (min 0.15mm)) (condition "A.Type == 'Pad'"))
(rule "2 oz outer copper, clearance" (constraint clearance (min 0.16mm)) (layer outer))
(rule "2 oz outer copper, track width" (constraint track_width (min 0.16mm)) (layer outer))"#,
            ),
        );
        // Four layers: the rule leaves the inner ones at the board's.
        let four_layers = |directory: &Path| {
            let pcb = directory.join("board.kicad_pcb");
            let text = fs::read_to_string(&pcb).unwrap().replacen(
                r#"(0 "F.Cu" signal) (2 "B.Cu" signal)"#,
                r#"(0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal)"#,
                1,
            );
            assert!(text.contains("In1.Cu"));
            fs::write(&pcb, text).unwrap();
        };
        four_layers(&directory);
        let config = resolve_project_rules(&path, &directory.join("board.kicad_pcb")).unwrap();
        assert_eq!(config.connection_rules["SDA"].clearance_mm, 0.09);
        let outer = config
            .net_rules
            .as_ref()
            .unwrap()
            .clearances
            .iter()
            .find(|rule| rule.clearance == 0.16)
            .unwrap();
        assert_eq!((&outer.first, &outer.second), (&None, &None));
        // The width per layer: the class keeps 0.09, the outer layers'
        // tracks 0.16 (until 2026-10-24 0.16 on every layer).
        assert_eq!(config.connection_rules["SDA"].trace_width_mm, 0.09);
        let widths = &config.net_rules.as_ref().unwrap().layer_widths["SDA"];
        assert_eq!(
            widths.iter().map(|width| width.min).collect::<Vec<_>>(),
            [0.16, 0.09, 0.09, 0.16]
        );
        let report = config.project_rules.as_ref().unwrap();
        let width_rule = report
            .custom_rules
            .iter()
            .find(|rule| rule.constraint == "track_width")
            .unwrap();
        assert_eq!(width_rule.status, "applied");
        assert_eq!(report.pad_clearance_mm, Some(0.16));
        assert_eq!(crate::board_placer::largest_clearance(&config), 0.16);
        fs::remove_dir_all(&directory).unwrap();
        // Without the file: the classes' 0.09, no floor.
        let (directory, path) = project_directory("outer-none", project, None);
        four_layers(&directory);
        let config = resolve_project_rules(&path, &directory.join("board.kicad_pcb")).unwrap();
        assert_eq!(
            config.project_rules.as_ref().unwrap().pad_clearance_mm,
            None
        );
        assert_eq!(crate::board_placer::largest_clearance(&config), 0.09);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_connection_width_floors_the_widths_per_layer() {
        // OpenESC's layer rules with a custom connection width on the inner
        // layers (an error), and then the board setup's minimum connection
        // at warning severity, which floors every layer by default.
        let with = |min_connection: &str| {
            format!(
                r#"{{"board": {{"design_settings": {{"rules": {{"min_clearance": 0.09, "min_track_width": 0.09{min_connection}}},
                    "rule_severities": {{"connection_width": "error"}}}}}},
                "net_settings": {{"classes": [{{"name": "Default", "clearance": 0.09, "track_width": 0.09, "via_diameter": 0.35, "via_drill": 0.2}}]}}}}"#
            )
        };
        let dru = r#"(version 1)
(rule "2 oz outer copper, track width" (constraint track_width (min 0.16mm)) (layer outer))
(rule "inner connections" (constraint connection_width (min 0.12mm)) (layer inner))"#;
        let four_layers = |directory: &Path| {
            let pcb = directory.join("board.kicad_pcb");
            let text = fs::read_to_string(&pcb).unwrap().replacen(
                r#"(0 "F.Cu" signal) (2 "B.Cu" signal)"#,
                r#"(0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal)"#,
                1,
            );
            fs::write(&pcb, text).unwrap();
        };
        let (directory, path) = project_directory("connection-layers", &with(""), Some(dru));
        four_layers(&directory);
        let config = resolve_project_rules(&path, &directory.join("board.kicad_pcb")).unwrap();
        let mins = |config: &KiCadBoardRouterConfig| {
            config.net_rules.as_ref().unwrap().layer_widths["SDA"]
                .iter()
                .map(|width| width.min)
                .collect::<Vec<_>>()
        };
        assert_eq!(mins(&config), [0.16, 0.12, 0.12, 0.16]);
        assert_eq!(config.connection_rules["SDA"].trace_width_mm, 0.09);
        let report = config.project_rules.as_ref().unwrap();
        let rule = report
            .custom_rules
            .iter()
            .find(|rule| rule.constraint == "connection_width")
            .unwrap();
        assert_eq!(rule.status, "applied", "{}", rule.detail);
        assert!(report.unmodelled.is_empty(), "{:?}", report.unmodelled);
        fs::remove_dir_all(&directory).unwrap();
        // The board setup's 0.16 at warning severity: every layer (the
        // default mode floors warnings).
        if std::env::var_os("PCB_MIN_CONNECTION_FLOOR").is_none() {
            let project = with(", \"min_connection\": 0.16").replace("\"error\"", "\"warning\"");
            let (directory, path) = project_directory("connection-board", &project, Some(dru));
            four_layers(&directory);
            let config = resolve_project_rules(&path, &directory.join("board.kicad_pcb")).unwrap();
            assert_eq!(config.min_connection_mm, Some(0.16));
            assert_eq!(config.connection_rules["SDA"].trace_width_mm, 0.16);
            assert_eq!(config.neck_width_mm, Some(0.16));
            if let Some(widths) = config
                .net_rules
                .as_ref()
                .and_then(|rules| rules.layer_widths.get("SDA"))
            {
                assert!(widths.iter().all(|width| width.min >= 0.16), "{widths:?}");
            }
            fs::remove_dir_all(&directory).unwrap();
        }
    }

    /// A project with mask, edge and hole rules, a minimum connection at
    /// error severity and a custom rule, and its board with an outline.
    fn rich_project(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let (directory, project) = project_directory(
            tag,
            r#"{"board": {"design_settings": {"rules": {"min_copper_edge_clearance": 0.3, "min_hole_clearance": 0.3, "min_hole_to_hole": 0.4,
                   "solder_mask_to_copper_clearance": 0.05, "min_connection": 0.15},
                   "rule_severities": {"solder_mask_bridge": "warning", "connection_width": "error"}}},
               "net_settings": {"classes": [{"name": "Default", "clearance": 0.2, "track_width": 0.2, "via_diameter": 0.6, "via_drill": 0.3}]}}"#,
            Some(
                r#"(version 1)
(rule "Wide USB" (condition "A.NetName == 'USB_DP'") (constraint track_width (min 0.25mm)))"#,
            ),
        );
        let pcb = directory.join("board.kicad_pcb");
        fs::write(
            &pcb,
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (setup (pad_to_mask_clearance 0.05))
              (gr_rect (start -5 -5) (end 15 5) (layer "Edge.Cuts"))
              (footprint "a" (at 0 0)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu" "F.Mask") (net "/POE_RAW"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu" "F.Mask") (net "V_POE_GOOD"))
                (pad "3" thru_hole circle (at 4 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu" "*.Mask") (net "SDA"))
                (pad "4" smd rect (at 6 0) (size 1 1) (layers "F.Cu" "F.Mask") (net "USB_DP")))
              (footprint "b" (at 0 3)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu" "F.Mask") (net "/POE_RAW"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu" "F.Mask") (net "V_POE_GOOD"))
                (pad "3" thru_hole circle (at 4 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu" "*.Mask") (net "SDA"))
                (pad "4" smd rect (at 6 0) (size 1 1) (layers "F.Cu" "F.Mask") (net "USB_DP"))))"#,
        )
        .unwrap();
        (directory, project, pcb)
    }

    /// The remapper trial's narrow-four control: 0.15 mm classes
    /// under the board's 0.2 mm minimum are refused before any work,
    /// naming the value, the minimum and its source, unless the file
    /// allows them; then they are kept and reported.
    #[test]
    fn classes_below_the_projects_minimum_are_refused_unless_allowed() {
        let (directory, project) = project_directory(
            "below-minimum",
            r#"{"board": {"design_settings": {"rules": {"min_track_width": 0.2, "min_clearance": 0.2}}},
               "net_settings": {"classes": [{"name": "Default", "clearance": 0.2, "track_width": 0.2, "via_diameter": 0.6, "via_drill": 0.3}]}}"#,
            None,
        );
        let pcb = directory.join("board.kicad_pcb");
        fs::write(
            &pcb,
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start -5 -5) (end 15 5) (layer "Edge.Cuts"))
              (footprint "a" (at 0 0)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu" "F.Mask") (net "QSPI_SCLK"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu" "F.Mask") (net "CTS1RTS")))
              (footprint "b" (at 0 3)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu" "F.Mask") (net "QSPI_SCLK"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu" "F.Mask") (net "CTS1RTS"))))"#,
        )
        .unwrap();
        let narrow = serde_json::json!({"net_classes": [{"name": "Narrow", "nets": ["QSPI_SCLK", "CTS1RTS"],
            "track_width_mm": 0.15, "clearance_mm": 0.15}]});
        let error =
            router_config_over_project(narrow.clone(), resolve_project_rules(&project, &pcb))
                .unwrap_err();
        assert!(crate::exit::is_input_error(&error), "{error}");
        for part in [
            "net_classes[Narrow] asks for track width 0.15 mm below the project's 0.2 mm (board.design_settings.rules.min_track_width",
            "net_classes[Narrow] asks for clearance 0.15 mm below the project's 0.2 mm (board.design_settings.rules.min_clearance",
            "on QSPI_SCLK, CTS1RTS",
            "allow_below_project_minimum",
        ] {
            assert!(error.contains(part), "{part}: {error}");
        }
        let mut allowed = narrow;
        allowed["allow_below_project_minimum"] = true.into();
        let config =
            router_config_over_project(allowed, resolve_project_rules(&project, &pcb)).unwrap();
        let overrides = &config.project_rules.as_ref().unwrap().overrides;
        assert_eq!(overrides.len(), 2);
        assert_eq!(overrides[0].quantity, "track_width");
        assert_eq!(overrides[0].requested_mm, 0.15);
        assert_eq!(overrides[0].project_minimum_mm, 0.2);
        assert_eq!(overrides[0].nets, ["QSPI_SCLK", "CTS1RTS"]);
        // The run's classes keep the requested width.
        assert_eq!(config.net_classes[0].track_width_mm, Some(0.15));
        // A file default_rules below the board's Default class clearance
        // (KiCad's DRC holds the board to it) and a narrow neck too.
        let error = router_config_over_project(
            serde_json::json!({"default_rules": {"trace_width_mm": 0.2, "clearance_mm": 0.1, "via_size_mm": 0.6, "via_drill_mm": 0.3},
                "neck_width_mm": 0.1}),
            resolve_project_rules(&project, &pcb),
        )
        .unwrap_err();
        assert!(
            error.contains("default_rules asks for clearance 0.1 mm"),
            "{error}"
        );
        assert!(
            error.contains("neck_width_mm asks for track width 0.1 mm"),
            "{error}"
        );
        assert!(
            !error.contains("default_rules asks for track width"),
            "{error}"
        );
        // At the minimum: accepted, nothing reported.
        let config = router_config_over_project(
            serde_json::json!({"net_classes": [{"name": "Even", "nets": ["CTS1RTS"], "track_width_mm": 0.2}]}),
            resolve_project_rules(&project, &pcb),
        )
        .unwrap();
        assert!(config.project_rules.unwrap().overrides.is_empty());
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_file_that_only_tunes_the_router_lowers_the_rules_of_auto() {
        let (directory, project, pcb) = rich_project("tunes");
        let auto = resolve_project_rules(&project, &pcb).unwrap();
        let file = router_config_over_project(
            serde_json::json!({"seeds": 4}),
            resolve_project_rules(&project, &pcb),
        )
        .unwrap();
        assert_eq!(file.seeds, Some(4));
        assert_eq!(file.solder_mask_to_copper_mm, Some(0.05));
        assert_eq!(file.solder_mask_bridge_severity.as_deref(), Some("warning"));
        assert_eq!(file.min_connection_mm, Some(0.15));
        assert!(file.net_rules.is_some() && file.project_rules.is_some());
        let board = parse(&fs::read_to_string(&pcb).unwrap()).unwrap();
        // The lowered board (its spokes' map in order) and every rule the
        // configuration carries.
        let lowered = |config: &KiCadBoardRouterConfig| {
            let mut lowered = crate::board_router::lower(&board, config, true)
                .unwrap()
                .board;
            let spokes: BTreeMap<_, _> = std::mem::take(&mut lowered.pad_spokes)
                .into_iter()
                .collect();
            format!(
                "{lowered:?} {spokes:?} {:?} {:?}",
                (
                    &config.connection_rules,
                    &config.default_rules,
                    config.edge_clearance_mm,
                    config.hole_clearance_mm,
                    config.hole_to_hole_clearance_mm,
                    config.neck_width_mm,
                    config.narrow_signal_mm,
                    config.relaxed_clearance_mm,
                ),
                (
                    config.solder_mask_to_copper_mm,
                    &config.solder_mask_bridge_severity,
                    config.min_connection_mm,
                    &config.connection_width_severity,
                    &config.net_rules,
                    &config.project_rules,
                )
            )
        };
        assert_eq!(lowered(&file), lowered(&auto));
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_file_with_default_rules_keeps_the_projects_clearances() {
        let (directory, project, pcb) = rich_project("default-rules");
        let rules = serde_json::json!({"trace_width_mm": 0.3, "clearance_mm": 0.25, "via_size_mm": 0.8, "via_drill_mm": 0.4});
        let config = router_config_over_project(
            serde_json::json!({"default_rules": rules}),
            resolve_project_rules(&project, &pcb),
        )
        .unwrap();
        // The file's classes replace the project's.
        assert!(config.connection_rules.is_empty());
        assert_eq!(config.default_rules.as_ref().unwrap().clearance_mm, 0.25);
        // The project's edge, hole and mask rules stay.
        assert_eq!(config.edge_clearance_mm, 0.3);
        assert_eq!(config.hole_clearance_mm, 0.3);
        assert_eq!(config.hole_to_hole_clearance_mm, 0.4);
        assert_eq!(config.solder_mask_to_copper_mm, Some(0.05));
        assert_eq!(
            config.solder_mask_bridge_severity.as_deref(),
            Some("warning")
        );
        assert!(config.net_rules.is_some());
        // Unless the file names them.
        let config = router_config_over_project(
            serde_json::json!({"default_rules": rules, "edge_clearance_mm": 0.1, "hole_clearance_mm": 0.2,
                "solder_mask_bridge_severity": "error"}),
            resolve_project_rules(&project, &pcb),
        )
        .unwrap();
        assert_eq!(config.edge_clearance_mm, 0.1);
        assert_eq!(config.hole_clearance_mm, 0.2);
        assert_eq!(config.hole_to_hole_clearance_mm, 0.4);
        assert_eq!(config.solder_mask_bridge_severity.as_deref(), Some("error"));
        // A project that does not resolve: a file with classes routes by
        // its own keys, one without fails.
        assert!(
            router_config_over_project(
                serde_json::json!({"default_rules": rules}),
                Err("no project".into())
            )
            .is_ok()
        );
        assert!(
            router_config_over_project(serde_json::json!({"seeds": 2}), Err("no project".into()))
                .is_err()
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn the_board_setup_states_a_minimum_or_takes_kicads() {
        let rules = serde_json::json!({"min_track_width": 0.15});
        let setup = DesignRules(&rules);
        assert_eq!(setup.minimum("min_track_width"), 0.15);
        assert_eq!(setup.minimum("min_via_diameter"), 0.5);
        assert_eq!(setup.minimum("min_microvia_drill"), 0.0);
        assert!(setup.source_of("min_track_width").contains(".kicad_pro"));
        assert!(
            setup
                .source_of("min_clearance")
                .starts_with("KiCad's default")
        );
        assert_eq!(nanometres(0.19999999999999998), 0.2);
    }

    #[test]
    fn a_net_takes_its_classes_by_name_list_and_pattern_each_once() {
        let project = serde_json::json!({"net_settings": {
            "classes": [{"name": "Power", "nets": ["VBUS"]}],
            "netclass_assignments": {"VBUS": ["USB", "Power"], "SDA": "I2C"},
            "netclass_patterns": [
                {"pattern": "V*", "netclass": "Power"},
                {"pattern": "VB.*", "netclass": "Wide"}
            ]
        }});
        let assignments = Assignments::of_project(&project);
        assert_eq!(
            assignments.class_names("VBUS", "VBUS"),
            ["USB", "Power", "Wide"]
        );
        assert_eq!(assignments.class_names("SDA", "SDA"), ["I2C"]);
        assert!(assignments.class_names("GND", "GND").is_empty());
    }

    #[test]
    fn a_project_without_a_default_class_takes_the_boards_common_copper() {
        let pcb = parse(
            r#"(kicad_pcb
              (segment (start 0 0) (end 1 0) (width 0.3) (layer "F.Cu") (net 1))
              (segment (start 1 0) (end 2 0) (width 0.3) (layer "F.Cu") (net 1))
              (segment (start 2 0) (end 3 0) (width 0.15) (layer "F.Cu") (net 1))
              (via (at 3 0) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1)))"#,
        )
        .unwrap();
        let rules = default_class(&[], &pcb);
        assert_eq!(rules.trace_width_mm, 0.3);
        assert_eq!(rules.via_size_mm, 0.8);
        assert_eq!(rules.via_drill_mm, 0.4);
        assert_eq!(rules.clearance_mm, DEFAULT_CLEARANCE);
    }

    #[test]
    fn layer_geometry_is_a_rule_aiming_at_a_width_on_some_layers() {
        let rules = dru::parse_rules(
            r#"(version 1)
(rule outer (layer outer) (constraint track_width (min 0.1) (opt 0.2)))
(rule floor (layer outer) (constraint track_width (min 0.16)))
(rule every (constraint track_width (opt 0.2)))
(rule inner (condition "A.Layer == 'In1.Cu'") (constraint diff_pair_gap (max 0.2)))"#,
        )
        .unwrap();
        let copper: Vec<String> = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
            .map(String::from)
            .to_vec();
        let found = layer_geometry_rules(&rules, &copper);
        let summary: Vec<(&str, &str, Vec<&str>)> = found
            .iter()
            .map(|rule| {
                (
                    rule.rule.as_str(),
                    rule.constraint.as_str(),
                    rule.layers.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("outer", "track_width", vec!["F.Cu", "B.Cu"]),
                ("inner", "diff_pair_gap", vec!["In1.Cu"]),
            ]
        );
    }
}

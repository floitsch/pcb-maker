// Copyright (C) 2026 Toit contributors.

//! The lowering of the custom rules to the router's rules: what it needs
//! of the board, what it gives the router, and the recognition of each
//! constraint (`lower_constraint`) before its kind's lowering
//! (`clearance`, `sizes`).

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use super::condition::{
    Ex, Item, Kind, NetFacts, Side, T3, leaf_on, net_leaves, opaque_atoms, pair_truth,
    parse_condition, side_name, text_matches,
};
use super::{Constraint, KiCadCustomRuleReport, Rule, clearance, sizes};

/// What the lowering needs of the board.
#[derive(Clone)]
pub(crate) struct BoardFacts {
    /// The nets with pads.
    pub nets: Vec<NetFacts>,
    /// In the router's order (F.Cu, the inner layers, B.Cu): a layer's
    /// index is its bit in `PairClearance::layers`.
    pub copper_layers: Vec<String>,
    /// The kinds of pad the board has.
    pub pad_kinds: BTreeSet<Kind>,
    /// Each net's rules before the custom rules (in `nets`' order), and
    /// the board's edge, hole and hole-to-hole clearances: a rule that
    /// asks no more than these changes nothing (`no_effect`).
    pub current: Vec<crate::KiCadConnectionRoutingRules>,
    pub board_clearances: [f64; 3],
    /// The neck-down width before the custom rules.
    pub neck_width: f64,
    /// Whether connection_width findings at warning severity floor the
    /// track widths too (`project_rules::FloorMode`): a custom
    /// `connection_width` rule is lowered where its findings are errors,
    /// and where they are warnings only with this.
    pub connection_floor_warnings: bool,
    /// The board's named zones and rule areas (the footprints' too), which
    /// `intersectsArea()`, `insideArea()` and `enclosedByArea()` name.
    pub areas: Vec<NamedArea>,
}

/// A zone or rule area a condition can name (`BoardFacts::areas`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NamedArea {
    pub name: String,
    pub uuid: String,
    /// Its copper layers, by name.
    pub layers: Vec<String>,
}

/// The atom an area test leaves for the geometry (`resolve_areas`):
/// unknown to the lowering, and the only unknown a clearance rule may
/// keep wherever it can hold.
const AREA_ATOM: &str = "area ";

/// KiCad's area tests on an item (`intersectsArea('X')`, `insideArea`,
/// `enclosedByArea`; pcbexpr_functions.cpp) hold only on the area's own
/// layers: an item on another layer never meets it (MokyaLora's
/// TFT_ZONE on In3.Cu: kicad-cli finds tracks 0.13 mm apart inside the
/// outline on F.Cu, In1.Cu and B.Cu under its 0.25 mm rule, and flags
/// only In3.Cu's). An area test becomes "the item is on one of the
/// areas' layers, and (unknown) where it lies": false off those layers,
/// false where no area has the name, unknown elsewhere.
fn resolve_areas(ex: Ex, areas: &[NamedArea]) -> Ex {
    let again = |ex: Box<Ex>| Box::new(resolve_areas(*ex, areas));
    match ex {
        Ex::Or(a, b) => Ex::Or(again(a), again(b)),
        Ex::And(a, b) => Ex::And(again(a), again(b)),
        Ex::Not(a) => Ex::Not(again(a)),
        Ex::Compare(operator, a, b) => Ex::Compare(operator, again(a), again(b)),
        Ex::Call(side, name, arguments)
            if matches!(
                name.as_str(),
                "intersectsarea" | "insidearea" | "enclosedbyarea"
            ) && matches!(arguments.as_slice(), [Ex::Text(_)])
                && side != Side::Both =>
        {
            let Some(Ex::Text(pattern)) = arguments.first() else {
                unreachable!()
            };
            let mut layers: Vec<&str> = areas
                .iter()
                .filter(|area| {
                    area.uuid == *pattern
                        || crate::project_rules::wildcard_match(pattern, &area.name)
                })
                .flat_map(|area| area.layers.iter().map(String::as_str))
                .collect();
            layers.sort_unstable();
            layers.dedup();
            let on_layers = layers
                .into_iter()
                .map(|layer| Ex::Call(side, "existsonlayer".into(), vec![Ex::Text(layer.into())]))
                .reduce(|a, b| Ex::Or(Box::new(a), Box::new(b)));
            match on_layers {
                None => Ex::Number(0.0),
                Some(on_layers) => Ex::And(
                    Box::new(on_layers),
                    Box::new(Ex::Opaque(format!(
                        "{AREA_ATOM}{}.{name}('{pattern}')",
                        side_name(side)
                    ))),
                ),
            }
        }
        other => other,
    }
}

/// Whether every atom the lowering cannot evaluate is an area's geometry
/// (`resolve_areas`).
pub(super) fn unknown_only_by_area(ex: &Ex) -> bool {
    let mut atoms = BTreeSet::new();
    opaque_atoms(ex, &mut atoms);
    !atoms.is_empty() && atoms.iter().all(|atom| atom.starts_with(AREA_ATOM))
}

/// A clearance between the items of two sets of nets (router
/// `ClearanceRule`): an item of kind A (`ROUTED_KINDS`) of a net of `first`
/// against one of kind B (`OTHER_KINDS`) of a net of `second`, on
/// `layers`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PairClearance {
    /// Indices into `BoardFacts::nets`; `None`: every item, of no net too.
    pub first: Option<Vec<usize>>,
    pub second: Option<Vec<usize>>,
    /// Bit `4 * a + b` for kinds `ROUTED_KINDS[a]` and B's (track, via,
    /// SMD pad, plated pad)
    /// (pcb_router::rules::kind_bit).
    pub kinds: u16,
    /// Bit per index of `BoardFacts::copper_layers`; all of them for a
    /// rule on every layer.
    pub layers: u32,
    pub clearance: f64,
}

/// The kinds the router lays (A of a clearance) and the kinds they meet
/// (B), in the router's order (pcb_router::rules::TRACK, VIA, PAD,
/// PLATED).
pub(super) const ROUTED_KINDS: [Kind; 2] = [Kind::Track, Kind::Via];

/// The rules a .kicad_dru gives the router.
#[derive(Clone, Debug, Default)]
pub struct Lowered {
    /// Clearance floors per net (the index of `BoardFacts::nets`).
    pub clearance: Vec<f64>,
    /// A clearance floor for every net (and nets without pads).
    pub clearance_all: f64,
    /// The largest clearance a rule asks between the pads of every two
    /// nets on some layer (the placer's copper clearance floor: the
    /// router lays no pads, and a rule on some layers only is a pair rule
    /// of the router that no per-net clearance shows).
    pub pad_clearance_all: f64,
    pub width_min: Vec<f64>,
    pub width_min_all: f64,
    pub width_max: Vec<f64>,
    pub width_opt: Vec<Option<f64>>,
    /// Track width bounds of rules that hold on some layers only (a
    /// `(layer ...)` clause or a layer condition), per index of
    /// `BoardFacts::copper_layers` and per net; empty when no rule is
    /// one: the per-net values above hold on every layer.
    pub layer_width_min: Vec<Vec<f64>>,
    pub layer_width_max: Vec<Vec<f64>>,
    pub layer_width_opt: Vec<Vec<Option<f64>>>,
    /// Per net and layer, the width of the last `track_width` rule on
    /// some layers only that holds there and aims at a width (an opt or a
    /// maximum; a minimum alone is the process's floor on those layers):
    /// its opt, else its min, else its max, whether or not it changes what
    /// the router does: the designer's widths per layer, for the reports.
    pub rule_layer_widths: BTreeMap<usize, BTreeMap<usize, f64>>,
    pub via_diameter_min: Vec<f64>,
    pub via_diameter_min_all: f64,
    pub via_diameter_max: Vec<f64>,
    pub via_drill_min: Vec<f64>,
    pub via_drill_min_all: f64,
    pub via_drill_max: Vec<f64>,
    pub annulus_min: Vec<f64>,
    pub annulus_min_all: f64,
    pub edge_clearance: f64,
    pub hole_clearance: f64,
    pub hole_to_hole: f64,
    /// Clearances the per-net values cannot keep exactly: between two
    /// sets of nets, from some item types only (pads to tracks), on some
    /// layers only (router `ClearanceRule`s).
    pub pair_clearances: Vec<PairClearance>,
    pub report: Vec<KiCadCustomRuleReport>,
    /// Set by a clearance kept wherever an area may hold (`lower`'s
    /// report says so).
    pub(super) by_area: bool,
    /// Routing rules the router does not keep, one line each, for the
    /// verdict.
    pub unmodelled: Vec<String>,
}

/// The project's severity of the findings a constraint makes, with
/// KiCad's defaults (board_design_settings.cpp: hole_to_hole is a
/// warning, the others errors).
pub(crate) fn finding_severity(kind: &str, severities: &BTreeMap<String, String>) -> String {
    let (key, default) = match kind {
        "clearance" => ("clearance", "error"),
        "track_width" => ("track_width", "error"),
        "connection_width" => ("connection_width", "warning"),
        "edge_clearance" => ("copper_edge_clearance", "error"),
        "hole_clearance" => ("hole_clearance", "error"),
        "hole_to_hole" => ("hole_to_hole", "warning"),
        "via_diameter" => ("via_diameter", "error"),
        "hole_size" => ("drill_out_of_range", "error"),
        "annular_width" => ("annular_width", "error"),
        "length" => ("length_out_of_range", "error"),
        "skew" => ("skew_out_of_range", "error"),
        "disallow" => ("items_not_allowed", "error"),
        _ => return "error".into(),
    };
    severities
        .get(key)
        // KiCad 8's name for hole_to_hole, which KiCad 10 still reads.
        .or_else(|| {
            (key == "hole_to_hole")
                .then(|| severities.get("hole_near_hole"))
                .flatten()
        })
        .cloned()
        .unwrap_or_else(|| default.into())
}

/// Constraints that do not concern the copper the router lays.
const NOT_ROUTING: [&str; 9] = [
    "silk_clearance",
    "text_height",
    "text_thickness",
    "courtyard_clearance",
    "solder_paste_abs_margin",
    "solder_paste_rel_margin",
    "solder_mask_expansion",
    "track_angle",
    "track_segment_length",
];

pub(super) fn mm(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text} mm")
}

impl Lowered {
    pub(super) fn new(nets: usize) -> Self {
        Self {
            clearance: vec![0.0; nets],
            width_min: vec![0.0; nets],
            width_max: vec![f64::INFINITY; nets],
            width_opt: vec![None; nets],
            via_diameter_min: vec![0.0; nets],
            via_diameter_max: vec![f64::INFINITY; nets],
            via_drill_min: vec![0.0; nets],
            via_drill_max: vec![f64::INFINITY; nets],
            annulus_min: vec![0.0; nets],
            ..Self::default()
        }
    }

    pub(super) fn note(
        &mut self,
        rule: &Rule,
        constraint: &Constraint,
        status: &str,
        detail: String,
    ) {
        if status == "unmodelled" {
            self.unmodelled.push(format!(
                "{} ('{}'{}): {detail}",
                constraint.kind,
                rule.name,
                constraint
                    .min
                    .map(|value| format!(", min {}", mm(value)))
                    .or_else(|| constraint.max.map(|value| format!(", max {}", mm(value))))
                    .unwrap_or_default()
            ));
        }
        self.report.push(KiCadCustomRuleReport {
            rule: rule.name.clone(),
            constraint: constraint.kind.clone(),
            status: status.into(),
            detail,
        });
    }
}

/// The groups of nets a condition cannot tell apart, with their members.
fn net_groups(ex: &Ex, nets: &[NetFacts]) -> Vec<Vec<usize>> {
    let mut leaves = Vec::new();
    let mut compares_classes = false;
    net_leaves(ex, &mut leaves, &mut compares_classes);
    let mut groups: BTreeMap<(Vec<u8>, String), Vec<usize>> = BTreeMap::new();
    for (index, net) in nets.iter().enumerate() {
        let signature = leaves
            .iter()
            .map(|leaf| match leaf_on(leaf, net) {
                T3::True => 1,
                T3::False => 0,
                T3::Unknown => 2,
            })
            .collect();
        let class = if compares_classes {
            net.effective()
        } else {
            String::new()
        };
        groups.entry((signature, class)).or_default().push(index);
    }
    groups.into_values().collect()
}

/// The layers of the board a rule's `(layer ...)` names (`outer`,
/// `inner`, a name, a pattern); all of them without one.
pub(super) fn rule_layers<'a>(rule: &Rule, copper: &'a [String]) -> Vec<&'a str> {
    copper
        .iter()
        .map(String::as_str)
        .filter(|layer| match rule.layer.as_deref() {
            None => true,
            Some("outer") => matches!(*layer, "F.Cu" | "B.Cu"),
            Some("inner") => !matches!(*layer, "F.Cu" | "B.Cu"),
            Some(pattern) => text_matches(pattern, layer),
        })
        .collect()
}

/// The copper layers a rule names: those of its layer clause, narrowed
/// to the layers its condition names in quotes where it names any
/// (`A.Layer == 'In1.Cu' || A.Layer == 'In4.Cu'`). A reading of the text,
/// for reports: the lowering evaluates the condition per layer.
pub(crate) fn rule_copper_layers(rule: &Rule, copper: &[String]) -> Vec<String> {
    let clause = rule_layers(rule, copper);
    let condition = rule.condition.as_deref().unwrap_or("");
    let named: Vec<&str> = clause
        .iter()
        .copied()
        .filter(|layer| {
            condition.contains(&format!("'{layer}'")) || condition.contains(&format!("\"{layer}\""))
        })
        .collect();
    if named.is_empty() { clause } else { named }
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Lowers the rules of a .kicad_dru for a board.
pub fn lower(rules: &[Rule], board: &BoardFacts, severities: &BTreeMap<String, String>) -> Lowered {
    let mut lowered = Lowered::new(board.nets.len());
    let no_net = NetFacts {
        name: String::new(),
        classes: vec!["Default".into()],
        diff_pair: None,
    };
    let conditions: Vec<Option<Ex>> = rules
        .iter()
        .map(|rule| match rule.condition.as_deref() {
            None => Some(Ex::Number(1.0)),
            Some(text) => parse_condition(text).ok(),
        })
        .collect();
    for (index, rule) in rules.iter().enumerate() {
        for constraint in &rule.constraints {
            // KiCad applies the last matching rule of a constraint: a later
            // rule with a minimum of the same constraint takes the pairs it
            // surely matches (JLCPCB's template: "Pad to Track" 0.2 mm and,
            // after it, "Min. Spacing" 0.127 on everything; AMLing2's drone
            // carrier, where KiCad finds no track 0.13 mm from a pad).
            let later: Vec<Later> = if SHADOWED.contains(&constraint.kind.as_str()) {
                rules[index + 1..]
                    .iter()
                    .zip(&conditions[index + 1..])
                    .filter(|(other, _)| {
                        other
                            .constraints
                            .iter()
                            .any(|own| own.kind == constraint.kind && own.min.is_some())
                    })
                    .filter_map(|(other, condition)| {
                        Some((
                            condition.as_ref()?,
                            rule_layers(other, &board.copper_layers),
                        ))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            lower_constraint(
                &mut lowered,
                rule,
                constraint,
                board,
                severities,
                &no_net,
                &later,
            );
            if std::mem::take(&mut lowered.by_area)
                && let Some(note) = lowered.report.last_mut()
                && matches!(note.status.as_str(), "applied" | "approximated")
            {
                note.status = "approximated".into();
                note.detail.push_str(
                    " (where a rule area may hold: on the area's layers wherever the area lies)",
                );
            }
        }
    }
    lowered
}

/// Constraints a later rule takes from an earlier one where it surely
/// matches (`lower`).
const SHADOWED: [&str; 4] = [
    "clearance",
    "hole_clearance",
    "edge_clearance",
    "hole_to_hole",
];

/// A later rule's condition and layers (`lower`).
type Later<'a> = (&'a Ex, Vec<&'a str>);

/// The routed kinds (A) and the kinds they meet (B) a constraint is
/// checked between, `None` for B when KiCad checks the item alone.
fn kinds_of(kind: &str, pads: &BTreeSet<Kind>) -> (Vec<Kind>, Option<Vec<Kind>>) {
    let holes: Vec<Kind> = [Kind::Via, Kind::PlatedPad, Kind::NpthPad]
        .into_iter()
        .filter(|kind| *kind == Kind::Via || pads.contains(kind))
        .collect();
    match kind {
        "clearance" => {
            let mut others = vec![Kind::Track, Kind::Via];
            others.extend(pads.iter().copied().filter(|kind| *kind != Kind::NpthPad));
            (vec![Kind::Track, Kind::Via], Some(others))
        }
        // KiCad checks a connection width per net and layer on the union
        // of its copper; the router lays tracks, so a minimum is a floor
        // on its tracks' width.
        "track_width" | "connection_width" => (vec![Kind::Track], None),
        "via_diameter" | "hole_size" | "annular_width" => (vec![Kind::Via], None),
        "edge_clearance" => (vec![Kind::Track, Kind::Via], Some(vec![Kind::Edge])),
        "hole_clearance" => (vec![Kind::Track, Kind::Via], Some(holes)),
        "hole_to_hole" => (vec![Kind::Via], Some(holes)),
        _ => (Vec::new(), None),
    }
}

/// Lowers one constraint of a rule: `recognise` decides whether the
/// router keeps it at all, then its kind's lowering does (`clearance`,
/// `sizes`). Every path notes the constraint once.
fn lower_constraint(
    lowered: &mut Lowered,
    rule: &Rule,
    constraint: &Constraint,
    board: &BoardFacts,
    severities: &BTreeMap<String, String>,
    no_net: &NetFacts,
    later: &[Later],
) {
    let Some((layers, condition)) = recognise(lowered, rule, constraint, board, severities) else {
        return;
    };
    let kind = constraint.kind.as_str();
    let matcher = Matcher::new(board, no_net, later, kind, layers, condition);
    match kind {
        "clearance" | "hole_clearance" | "edge_clearance" | "hole_to_hole" => {
            let Some(value) = constraint.min else {
                lowered.note(rule, constraint, "no_effect", "no minimum".into());
                return;
            };
            if kind == "clearance" {
                clearance::lower_net_clearance(lowered, rule, constraint, &matcher, value);
            } else {
                clearance::lower_board_clearance(lowered, rule, constraint, &matcher, value);
            }
        }
        _ => sizes::lower_sizes(lowered, rule, constraint, &matcher),
    }
}

/// Recognises whether the router keeps a constraint at all, and notes it
/// where not: one on what the router does not lay, a severity that makes
/// its findings no errors, a layer clause no layer of the board meets, a
/// condition that does not parse, a `disallow`, a kind the router does
/// not model. Otherwise the board's layers of the rule and its condition
/// (with its areas resolved).
pub(super) fn recognise<'a>(
    lowered: &mut Lowered,
    rule: &Rule,
    constraint: &Constraint,
    board: &'a BoardFacts,
    severities: &BTreeMap<String, String>,
) -> Option<(Vec<&'a str>, Ex)> {
    let kind = constraint.kind.as_str();
    if NOT_ROUTING.contains(&kind) {
        lowered.note(
            rule,
            constraint,
            "not_routing",
            "not a rule on routed copper".into(),
        );
        return None;
    }
    if let Some(severity) = rule
        .severity
        .as_deref()
        .filter(|s| *s != "error")
        .filter(|s| {
            !(kind == "connection_width" && *s == "warning" && board.connection_floor_warnings)
        })
    {
        lowered.note(
            rule,
            constraint,
            "skipped",
            format!("the rule's severity is {severity}: its findings are no errors"),
        );
        return None;
    }
    let modelled = matches!(
        kind,
        "clearance"
            | "track_width"
            | "connection_width"
            | "via_diameter"
            | "hole_size"
            | "annular_width"
            | "edge_clearance"
            | "hole_clearance"
            | "hole_to_hole"
    );
    let layers = rule_layers(rule, &board.copper_layers);
    if layers.is_empty() {
        lowered.note(
            rule,
            constraint,
            "skipped",
            format!(
                "layer {} is not on this board",
                rule.layer.as_deref().unwrap_or("?")
            ),
        );
        return None;
    }
    let condition = match rule.condition.as_deref().map(parse_condition).transpose() {
        Ok(condition) => resolve_areas(condition.unwrap_or(Ex::Number(1.0)), &board.areas),
        Err(error) => {
            lowered.note(
                rule,
                constraint,
                if modelled {
                    "unmodelled"
                } else {
                    "not_routing"
                },
                format!("the condition does not parse: {error}"),
            );
            return None;
        }
    };
    if kind == "disallow" {
        note_disallow(lowered, rule, constraint);
        return None;
    }
    if !modelled {
        note_unmodelled(lowered, rule, constraint);
        return None;
    }
    // (A connection width rule's own severity is KiCad's for its
    // findings; above, only a warning passes, where warnings floor.)
    let severity = match (kind, rule.severity.as_deref()) {
        ("connection_width", Some(own)) => own.to_string(),
        _ => finding_severity(kind, severities),
    };
    // A connection width at warning severity is a floor where the mode
    // floors warnings (`project_rules::FloorMode`).
    let floors_warning =
        kind == "connection_width" && severity == "warning" && board.connection_floor_warnings;
    if severity != "error" && !floors_warning {
        lowered.note(
            rule,
            constraint,
            "skipped",
            format!("the project rates these findings {severity}"),
        );
        return None;
    }
    Some((layers, condition))
}

/// Notes a `disallow` constraint: blind, buried and micro vias the router
/// does not lay unless asked, footprints and graphics it does not place,
/// anything else a rule area it does not keep (`project_rules`'
/// `track_disallowed` keeps a `disallow track` per net).
fn note_disallow(lowered: &mut Lowered, rule: &Rule, constraint: &Constraint) {
    let words: Vec<&str> = constraint.words.iter().map(String::as_str).collect();
    if words
        .iter()
        .all(|word| matches!(*word, "buried_via" | "micro_via" | "blind_via"))
    {
        lowered.note(
            rule,
            constraint,
            "no_effect",
            format!(
                "{}: the router lays through vias only unless the router setting \"blind_vias\" is on (its spans then come from via_spans or the designer's own vias, and this rule is not kept)",
                words.join(", ")
            ),
        );
    } else if words
        .iter()
        .all(|word| matches!(*word, "footprint" | "text" | "graphic"))
    {
        lowered.note(
            rule,
            constraint,
            "not_routing",
            "placement or graphics".into(),
        );
    } else {
        lowered.note(
            rule,
            constraint,
            "unmodelled",
            format!(
                "disallow {} where {} (rule areas from rules are not lowered)",
                words.join(", "),
                rule.condition.as_deref().unwrap_or("anywhere")
            ),
        );
    }
}

/// Notes a constraint of a kind the router does not model, with why.
fn note_unmodelled(lowered: &mut Lowered, rule: &Rule, constraint: &Constraint) {
    let kind = constraint.kind.as_str();
    let detail = match kind {
        "creepage" => "creepage (surface distance) is not modelled".to_string(),
        "length" | "skew" | "max_uncoupled" | "via_count" => {
            "no length tuning or via counts in the router".into()
        }
        "diff_pair_gap" | "diff_pair_uncoupled" => {
            "differential pairs route coupled only with the router setting `diff_pairs`, off here (a layout's routes keep it off unless the router config sets \"diff_pairs\": true; route mode turns it on where such a rule holds)".into()
        }
        "physical_clearance" | "physical_hole_clearance" => {
            "physical clearances (also within a net) are not modelled".into()
        }
        "thermal_relief_gap" | "thermal_spoke_width" | "zone_connection" | "min_resolved_spokes" => {
            "zone connection rules are KiCad's fill's".into()
        }
        "assertion" => "assertions are not evaluated".into(),
        "bridged_mask" => "mask bridging rules are not modelled".into(),
        other => format!("{other} is not modelled"),
    };
    // Pour rules and assertions do not stop a route.
    let status = if matches!(
        kind,
        "thermal_relief_gap" | "thermal_spoke_width" | "zone_connection" | "min_resolved_spokes"
    ) || kind == "assertion"
    {
        "not_routing"
    } else {
        "unmodelled"
    };
    lowered.note(rule, constraint, status, detail);
}

/// What a rule's condition matches on this board (`lower_constraint`):
/// the condition, the layers it holds on, the kinds of item the
/// constraint is checked between (`kinds_of`), the groups of nets it
/// cannot tell apart, and the later rules, which take what they surely
/// match.
pub(super) struct Matcher<'a> {
    pub board: &'a BoardFacts,
    /// The constraint's kind (`clearance`, `track_width`, ...).
    pub kind: &'a str,
    /// The board's layers of the rule (`rule_layers`).
    pub layers: Vec<&'a str>,
    pub condition: Ex,
    /// The groups of nets neither the condition nor a later rule's can
    /// tell apart (`net_groups`), as indices into `BoardFacts::nets`.
    pub groups: Vec<Vec<usize>>,
    /// Whether a later rule surely took some pair (`truth`); remembered
    /// for the report.
    pub shadowed_any: Cell<bool>,
    /// The net of an item of no net (an unplated hole).
    no_net: &'a NetFacts,
    later: &'a [Later<'a>],
    /// The routed kinds (A) and the kinds they meet (B; `None` for a
    /// constraint on an item alone), `kinds_of`.
    a_kinds: Vec<Kind>,
    b_kinds: Option<Vec<Kind>>,
}

impl<'a> Matcher<'a> {
    pub fn new(
        board: &'a BoardFacts,
        no_net: &'a NetFacts,
        later: &'a [Later<'a>],
        kind: &'a str,
        layers: Vec<&'a str>,
        condition: Ex,
    ) -> Self {
        let (a_kinds, b_kinds) = kinds_of(kind, &board.pad_kinds);
        // The groups tell apart what the later rules' conditions do too (a
        // later rule may take some of the nets of one group of this rule).
        let grouping = later.iter().fold(condition.clone(), |all, (other, _)| {
            Ex::And(Box::new(all), Box::new((*other).clone()))
        });
        let groups = net_groups(&grouping, &board.nets);
        Self {
            board,
            kind,
            layers,
            condition,
            groups,
            shadowed_any: Cell::new(false),
            no_net,
            later,
            a_kinds,
            b_kinds,
        }
    }

    /// The net that stands for a group: the condition tells its nets
    /// apart from no other.
    pub fn representative(&self, group: &[usize]) -> &'a NetFacts {
        &self.board.nets[group[0]]
    }

    /// The condition's truth for a pair of items (B `None`: A alone),
    /// false where a later rule surely takes the pair (`lower`).
    pub fn truth(&self, a: Item, b: Option<Item>, same: bool) -> T3 {
        let truth = pair_truth(&self.condition, a, b, same);
        if truth != T3::False
            && self.later.iter().any(|(condition, layers)| {
                layers.contains(&a.layer) && pair_truth(condition, a, b, same) == T3::True
            })
        {
            self.shadowed_any.set(true);
            T3::False
        } else {
            truth
        }
    }

    /// Whether the rule matches an item of `a_net` against one of `b_net`
    /// (or the item alone, for a constraint on one item) of the kinds the
    /// constraint is checked between, on some layer of the rule.
    pub fn truth_for(&self, a_net: Option<&NetFacts>, b_net: Option<&NetFacts>, same: bool) -> T3 {
        let mut result = T3::False;
        for &layer in &self.layers {
            for a_kind in &self.a_kinds {
                let a = Item {
                    kind: *a_kind,
                    net: a_net,
                    layer,
                };
                match &self.b_kinds {
                    None => result = result.or(self.truth(a, None, false)),
                    Some(b_kinds) => {
                        for b_kind in b_kinds {
                            let b_net = match b_kind {
                                Kind::NpthPad => Some(self.no_net),
                                Kind::Edge => None,
                                _ => b_net,
                            };
                            let b = Item {
                                kind: *b_kind,
                                net: b_net,
                                layer: if *b_kind == Kind::Edge {
                                    "Edge.Cuts"
                                } else {
                                    layer
                                },
                            };
                            result = result.or(self.truth(a, Some(b), same));
                        }
                    }
                }
                if result == T3::True {
                    return result;
                }
            }
        }
        result
    }

    /// What the condition depends on that the lowering cannot tell, for a
    /// note (`opaque_atoms`).
    pub fn opaque(&self) -> String {
        let mut atoms = BTreeSet::new();
        opaque_atoms(&self.condition, &mut atoms);
        atoms.into_iter().collect::<Vec<_>>().join(", ")
    }
}

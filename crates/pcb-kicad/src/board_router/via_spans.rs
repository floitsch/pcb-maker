// Copyright (C) 2026 Toit contributors.

//! Blind, buried and micro vias (docs/router.md, "Blind, buried and micro
//! vias (2026-11-05)"): which via spans a board allows, and the size of a
//! micro via.
//!
//! KiCad 8 and later allow every via kind on every board (the board setup's
//! switches `allow_blind_buried_vias` and `allow_microvias` are gone; KiCad
//! 6 and 7 projects still carry them); whether the fab builds them is the
//! designer's choice. The router takes the spans a board allows from what
//! says so: the router settings' `via_spans`, the designer's own vias of
//! those kinds (the board's, or the `--reference` board's), and a KiCad 6 or
//! 7 project's switches. With the switch `blind_vias` off (or nothing
//! allowing a span) every via is a through via, as before.

use super::*;
use pcb_router as core;

/// `PCB_ROUTER_BLIND_VIAS` when set: overrides `blind_vias` (`0` or empty
/// off, anything else on).
pub fn blind_vias_override() -> Option<bool> {
    let value = std::env::var("PCB_ROUTER_BLIND_VIAS").ok()?;
    Some(!(value.is_empty() || value == "0"))
}

/// Whether the router takes the spans the board allows (`blind_vias`;
/// unset: off, docs/router.md for the measure).
pub(crate) fn blind_vias_on(config: &KiCadBoardRouterConfig) -> bool {
    blind_vias_override().unwrap_or(config.blind_vias.unwrap_or(false))
}

/// One via span the designer's board uses (`KiCadProjectRulesReport::
/// via_spans_designer`): its layers, KiCad's kind, how many of the board's
/// vias take it and their most common diameter and drill.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KiCadViaSpanEntry {
    pub layers: [String; 2],
    /// `blind`, `buried` or `micro`.
    pub kind: String,
    pub vias: usize,
    pub via_mm: [f64; 2],
    /// Where it comes from: "the board", "the reference board".
    pub source: String,
}

impl KiCadViaSpanEntry {
    /// As a `via_spans` entry (`F.Cu-In1.Cu`, `F.Cu-In1.Cu:micro`).
    pub fn spelled(&self) -> String {
        let span = format!("{}-{}", self.layers[0], self.layers[1]);
        if self.kind == "micro" {
            format!("{span}:micro")
        } else {
            span
        }
    }
}

/// The designer's vias of a span short of the stack on `pcb`, one entry
/// per span and kind, the most used first.
pub(crate) fn designer_spans(pcb: &Expr, source: &str) -> Vec<KiCadViaSpanEntry> {
    let Ok(layers) = LayerTable::from_pcb(pcb) else {
        return Vec::new();
    };
    let count = layers.len();
    if count < 3 {
        return Vec::new();
    }
    let mut found: BTreeMap<(usize, usize, bool), Vec<[f64; 2]>> = BTreeMap::new();
    for via in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("via"))
    {
        let kind = via.children().get(1).and_then(Expr::atom);
        let micro = kind == Some("micro");
        if !matches!(kind, Some("blind" | "buried" | "micro")) {
            continue;
        }
        let ends: Vec<usize> = via
            .child("layers")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .skip(1)
            .filter_map(Expr::atom)
            .filter_map(|name| layers.index(name.trim_matches('"')))
            .collect();
        let [a, b] = ends[..] else {
            continue;
        };
        let (top, bottom) = (a.min(b), a.max(b));
        let span = core::ViaSpan {
            top,
            bottom,
            micro,
            size: None,
        };
        // A "blind" via of the whole stack is a through via; a micro via
        // deeper than one dielectric is taken as blind or buried.
        let span = if micro && !span.is_valid(count) {
            core::ViaSpan {
                micro: false,
                ..span
            }
        } else {
            span
        };
        if !span.is_valid(count) {
            continue;
        }
        let (Ok(size), Ok(drill)) = (form_f64(via, "size", 1), form_f64(via, "drill", 1)) else {
            continue;
        };
        found
            .entry((span.top, span.bottom, span.micro))
            .or_default()
            .push([size, drill]);
    }
    let mut entries: Vec<KiCadViaSpanEntry> = found
        .into_iter()
        .map(|((top, bottom, micro), sizes)| {
            let span = core::ViaSpan {
                top,
                bottom,
                micro,
                size: None,
            };
            KiCadViaSpanEntry {
                layers: [layers.names[top].clone(), layers.names[bottom].clone()],
                kind: kind_name(span, count).to_string(),
                vias: sizes.len(),
                via_mm: most_common_size(&sizes),
                source: source.to_string(),
            }
        })
        .collect();
    entries.sort_by(|a, b| b.vias.cmp(&a.vias).then_with(|| a.layers.cmp(&b.layers)));
    entries
}

fn kind_name(span: core::ViaSpan, layers: usize) -> &'static str {
    match span.kind(layers) {
        core::ViaKind::Micro => "micro",
        core::ViaKind::Blind => "blind",
        core::ViaKind::Buried => "buried",
        core::ViaKind::Through => "through",
    }
}

fn most_common_size(sizes: &[[f64; 2]]) -> [f64; 2] {
    let mut counts: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    for [size, drill] in sizes {
        *counts
            .entry((
                (size * 1.0e6).round() as i64,
                (drill * 1.0e6).round() as i64,
            ))
            .or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map_or([0.0; 2], |((size, drill), _)| {
            [size as f64 / 1.0e6, drill as f64 / 1.0e6]
        })
}

/// The spans a KiCad 6 or 7 project's switches allow, as `via_spans`
/// keywords: `blind` and `buried` for `allow_blind_buried_vias`, `micro`
/// for `allow_microvias`.
pub(crate) fn project_spans(rules: &serde_json::Value) -> Vec<String> {
    let mut spans = Vec::new();
    if rules["allow_microvias"].as_bool() == Some(true) {
        spans.push("micro".to_string());
    }
    if rules["allow_blind_buried_vias"].as_bool() == Some(true) {
        spans.extend(["blind".to_string(), "buried".to_string()]);
    }
    spans
}

/// The micro via's size: the designer's of the span, else the project's
/// (the default class's `microvia_diameter` and `microvia_drill`), raised
/// to the board's micro via minimums and annular width; None where that
/// is no smaller than the class's via (`class` diameter and drill), which
/// the router's maps hold every via to.
pub(crate) fn micro_size(
    wanted: [f64; 2],
    minimums: KiCadMicroViaMinimums,
    class: [f64; 2],
) -> Option<[f64; 2]> {
    // In nanometres, as the board file holds them.
    let nanometres = |value: f64| (value * 1.0e6).round() / 1.0e6;
    let drill = nanometres(wanted[1].max(minimums.drill_mm));
    let diameter = nanometres(
        wanted[0]
            .max(minimums.diameter_mm)
            .max(drill + 2.0 * minimums.annular_mm),
    );
    (drill > 0.0 && diameter <= class[0] + 1.0e-9 && drill <= class[1] + 1.0e-9)
        .then_some([diameter, drill])
}

/// The board's micro via minimums (board setup `min_microvia_diameter`,
/// `min_microvia_drill`; the annular width applies to every via).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KiCadMicroViaMinimums {
    pub diameter_mm: f64,
    pub drill_mm: f64,
    pub annular_mm: f64,
}

impl Default for KiCadMicroViaMinimums {
    /// KiCad's defaults (board_design_settings.cpp, 10.0.7).
    fn default() -> Self {
        Self {
            diameter_mm: 0.2,
            drill_mm: 0.1,
            annular_mm: 0.1,
        }
    }
}

/// The spans `spelled` names on `layers` (`via_spans` entries): an explicit
/// `F.Cu-In2.Cu` (blind or buried by its layers), `F.Cu-In1.Cu:micro`, or
/// a keyword: `blind` (every span from an outer layer to an inner one),
/// `buried` (every span between inner layers), `micro` (one dielectric
/// down from each outer layer). An entry naming no span of the stack is an
/// error.
pub(crate) fn spans_named(
    spelled: &str,
    layers: &LayerTable,
) -> Result<Vec<core::ViaSpan>, String> {
    let count = layers.len();
    let span = |top: usize, bottom: usize, micro: bool| core::ViaSpan {
        top,
        bottom,
        micro,
        size: None,
    };
    let spans: Vec<core::ViaSpan> = match spelled {
        "micro" => vec![span(0, 1, true), span(count - 2, count - 1, true)],
        "blind" => (1..count - 1)
            .flat_map(|inner| [span(0, inner, false), span(inner, count - 1, false)])
            .collect(),
        "buried" => (1..count - 1)
            .flat_map(|top| (top + 1..count - 1).map(move |bottom| span(top, bottom, false)))
            .collect(),
        _ => {
            let (names, micro) = match spelled.strip_suffix(":micro") {
                Some(names) => (names, true),
                None => (spelled, false),
            };
            let index = |name: &str| {
                layers
                    .index(name.trim())
                    .ok_or_else(|| format!("via_spans: no copper layer {name:?} in {spelled:?}"))
            };
            let (first, last) = names
                .split_once('-')
                .ok_or_else(|| format!("via_spans: {spelled:?} is not <layer>-<layer>"))?;
            let (a, b) = (index(first)?, index(last)?);
            vec![span(a.min(b), a.max(b), micro)]
        }
    };
    let spans: Vec<core::ViaSpan> = spans
        .into_iter()
        .filter(|span| span.is_valid(count))
        .collect();
    if spans.is_empty() && !matches!(spelled, "micro" | "blind" | "buried") {
        return Err(format!(
            "via_spans: {spelled:?} is no span short of the stack of {count} copper layers \
             (a micro via is one dielectric deep)"
        ));
    }
    Ok(spans)
}

/// The spans the router may use on the board of `layers`, with what allows
/// each (`via_spans` of the router settings, else the designer's and the
/// project's): empty with `blind_vias` off, on two layers, or where nothing
/// allows a span. A micro via takes its own size (`micro_size`) where that
/// is smaller than the class's.
pub(crate) fn allowed_spans(
    config: &KiCadBoardRouterConfig,
    layers: &LayerTable,
    class_via: [f64; 2],
) -> Result<(Vec<core::ViaSpan>, Vec<String>), String> {
    if !blind_vias_on(config) || layers.len() < 3 {
        return Ok((Vec::new(), Vec::new()));
    }
    let rules = config.project_rules.as_ref();
    let mut named: Vec<(String, Option<[f64; 2]>, String)> = Vec::new();
    match &config.via_spans {
        Some(listed) => {
            for spelled in listed {
                named.push((spelled.clone(), None, "via_spans".to_string()));
            }
        }
        None => {
            for entry in rules
                .into_iter()
                .flat_map(|rules| &rules.via_spans_designer)
            {
                named.push((entry.spelled(), Some(entry.via_mm), entry.source.clone()));
            }
            for keyword in rules.into_iter().flat_map(|rules| &rules.via_spans_project) {
                named.push((keyword.clone(), None, "the project's switches".to_string()));
            }
        }
    }
    let minimums = rules.map_or_else(KiCadMicroViaMinimums::default, |rules| {
        rules.micro_via_minimums
    });
    let project_micro = rules
        .and_then(|rules| rules.micro_via_mm)
        .unwrap_or([0.3, 0.1]);
    let mut spans: Vec<core::ViaSpan> = Vec::new();
    let mut report = Vec::new();
    for (spelled, designer_size, source) in named {
        for mut span in spans_named(&spelled, layers)? {
            if spans.iter().any(|known| known.mask() == span.mask()) {
                continue;
            }
            if span.micro {
                span.size = micro_size(designer_size.unwrap_or(project_micro), minimums, class_via);
            }
            report.push(format!(
                "{}-{} {}{} ({source})",
                layers.names[span.top],
                layers.names[span.bottom],
                kind_name(span, layers.len()),
                span.size
                    .map(|[diameter, drill]| format!(" {diameter}/{drill} mm"))
                    .unwrap_or_default()
            ));
            spans.push(span);
        }
    }
    Ok((spans, report))
}

/// Adds the `--reference` board's vias of a span to the designer's spans
/// (`KiCadProjectRulesReport::via_spans_designer`), as the reference's vias
/// in pads (`admit_reference_vias_in_pads`).
pub fn admit_reference_via_spans(
    config: &mut KiCadBoardRouterConfig,
    reference: &Path,
    board_id: &str,
) {
    let path = reference.join(format!("{board_id}.kicad_pcb"));
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(pcb) = parse(&text) else {
        return;
    };
    let entries = designer_spans(&pcb, "the reference board");
    if entries.is_empty() {
        return;
    }
    let report = config.project_rules.get_or_insert_with(Default::default);
    for entry in entries {
        if !report
            .via_spans_designer
            .iter()
            .any(|known| known.layers == entry.layers && known.kind == entry.kind)
        {
            report.via_spans_designer.push(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_rules::KiCadProjectRulesReport;

    fn table(count: usize) -> LayerTable {
        let mut names = vec!["F.Cu".to_string()];
        names.extend((1..count - 1).map(|inner| format!("In{inner}.Cu")));
        names.push("B.Cu".to_string());
        LayerTable { names }
    }

    fn spelled(spans: &[core::ViaSpan]) -> Vec<(usize, usize, bool)> {
        spans
            .iter()
            .map(|span| (span.top, span.bottom, span.micro))
            .collect()
    }

    #[test]
    fn the_keywords_name_the_spans_of_the_stack() {
        let six = table(6);
        assert_eq!(
            spelled(&spans_named("micro", &six).unwrap()),
            [(0, 1, true), (4, 5, true)]
        );
        assert_eq!(
            spelled(&spans_named("blind", &six).unwrap()),
            [
                (0, 1, false),
                (1, 5, false),
                (0, 2, false),
                (2, 5, false),
                (0, 3, false),
                (3, 5, false),
                (0, 4, false),
                (4, 5, false)
            ]
        );
        assert_eq!(
            spelled(&spans_named("buried", &six).unwrap()),
            [
                (1, 2, false),
                (1, 3, false),
                (1, 4, false),
                (2, 3, false),
                (2, 4, false),
                (3, 4, false)
            ]
        );
        assert_eq!(
            spelled(&spans_named("In1.Cu-F.Cu:micro", &six).unwrap()),
            [(0, 1, true)]
        );
        assert_eq!(
            spelled(&spans_named("In1.Cu-In4.Cu", &six).unwrap()),
            [(1, 4, false)]
        );
        // KiCad's rules: no micro via two dielectrics deep, no span of the
        // whole stack, only the board's layers.
        assert!(spans_named("F.Cu-In2.Cu:micro", &six).is_err());
        assert!(spans_named("F.Cu-B.Cu", &six).is_err());
        assert!(spans_named("F.Cu-In7.Cu", &six).is_err());
        assert!(spans_named("F.Cu", &six).is_err());
        // Four layers have no buried span but In1-In2.
        assert_eq!(
            spelled(&spans_named("buried", &table(4)).unwrap()),
            [(1, 2, false)]
        );
    }

    #[test]
    fn a_micro_via_keeps_the_boards_minimums_and_stays_within_the_class_via() {
        let minimums = KiCadMicroViaMinimums::default();
        // 0.3 / 0.1 at a 0.1 annular width: as asked.
        assert_eq!(
            micro_size([0.3, 0.1], minimums, [0.6, 0.3]),
            Some([0.3, 0.1])
        );
        // A ring thinner than the annular width grows.
        assert_eq!(
            micro_size([0.25, 0.1], minimums, [0.6, 0.3]),
            Some([0.3, 0.1])
        );
        // A drill under the minimum grows.
        assert_eq!(
            micro_size([0.3, 0.05], minimums, [0.6, 0.3]),
            Some([0.3, 0.1])
        );
        // Larger than the class's via: the class's (None).
        assert_eq!(micro_size([0.3, 0.1], minimums, [0.25, 0.1]), None);
    }

    #[test]
    fn the_designers_vias_of_a_span_are_counted_by_span_and_kind() {
        let pcb = parse(
            r#"(kicad_pcb (version 20260206)
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal))
  (via micro (at 1 1) (size 0.3) (drill 0.1) (layers "F.Cu" "In1.Cu") (net "A"))
  (via micro (at 2 1) (size 0.3) (drill 0.1) (layers "F.Cu" "In1.Cu") (net "A"))
  (via micro (at 3 1) (size 0.35) (drill 0.15) (layers "F.Cu" "In1.Cu") (net "A"))
  (via buried (at 4 1) (size 0.5) (drill 0.25) (layers "In1.Cu" "In2.Cu") (net "A"))
  (via blind (at 5 1) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "A"))
  (via micro (at 6 1) (size 0.3) (drill 0.1) (layers "F.Cu" "In2.Cu") (net "A"))
  (via (at 7 1) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "A")))"#,
        )
        .unwrap();
        let entries = designer_spans(&pcb, "the board");
        let found: Vec<(String, &str, usize, [f64; 2])> = entries
            .iter()
            .map(|entry| {
                (
                    entry.spelled(),
                    entry.kind.as_str(),
                    entry.vias,
                    entry.via_mm,
                )
            })
            .collect();
        // The whole-stack "blind" via is a through via; the micro via two
        // dielectrics deep is a blind one.
        assert_eq!(
            found,
            [
                ("F.Cu-In1.Cu:micro".to_string(), "micro", 3, [0.3, 0.1]),
                ("F.Cu-In2.Cu".to_string(), "blind", 1, [0.3, 0.1]),
                ("In1.Cu-In2.Cu".to_string(), "buried", 1, [0.5, 0.25]),
            ]
        );
    }

    #[test]
    fn the_spans_follow_the_switch_the_settings_and_the_designer() {
        let layers = table(4);
        let mut config = KiCadBoardRouterConfig {
            project_rules: Some(KiCadProjectRulesReport {
                via_spans_designer: vec![KiCadViaSpanEntry {
                    layers: ["F.Cu".into(), "In1.Cu".into()],
                    kind: "micro".into(),
                    vias: 4,
                    via_mm: [0.3, 0.1],
                    source: "the board".into(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let class = [0.6, 0.3];
        if blind_vias_override().is_none() {
            // Off by default: through vias only.
            assert!(allowed_spans(&config, &layers, class).unwrap().0.is_empty());
        }
        config.blind_vias = Some(true);
        if blind_vias_override() != Some(false) {
            let (spans, report) = allowed_spans(&config, &layers, class).unwrap();
            assert_eq!(spelled(&spans), [(0, 1, true)]);
            assert_eq!(spans[0].size, Some([0.3, 0.1]));
            assert_eq!(report, ["F.Cu-In1.Cu micro 0.3/0.1 mm (the board)"]);
            // The settings' list replaces the designer's.
            config.via_spans = Some(vec!["buried".into(), "F.Cu-In2.Cu".into()]);
            let (spans, _) = allowed_spans(&config, &layers, class).unwrap();
            assert_eq!(spelled(&spans), [(1, 2, false), (0, 2, false)]);
            // Two layers: nothing to span.
            assert!(
                allowed_spans(&config, &table(2), class)
                    .unwrap()
                    .0
                    .is_empty()
            );
            config.via_spans = Some(vec!["F.Cu-In9.Cu".into()]);
            assert!(allowed_spans(&config, &layers, class).is_err());
        }
    }

    #[test]
    fn a_via_of_a_span_is_written_with_its_kind_and_layers() {
        let names: Vec<String> = table(4).names;
        let span = |top: usize, bottom: usize, micro: bool| core::ViaSpan {
            top,
            bottom,
            micro,
            size: None,
        };
        let written = |span: Option<core::ViaSpan>| {
            let (layers, kind) = via_layers_and_kind(span, 4, &names);
            let text = crate::kicad_sexpr::encode(&crate::copper_emit::supplemental_via_expr(
                &crate::copper_emit::SupplementalVia {
                    connection: "A".into(),
                    at: [1.0, 2.0],
                    size: 0.3,
                    drill: 0.1,
                    layers,
                    kind,
                },
            ));
            // One line, single spaces.
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        let through = written(None);
        assert!(through.starts_with("(via (at 1 2)"), "{through}");
        assert!(through.contains(r#"(layers "F.Cu" "B.Cu")"#));
        let micro = written(Some(span(0, 1, true)));
        assert!(micro.starts_with("(via micro (at 1 2)"), "{micro}");
        assert!(micro.contains(r#"(layers "F.Cu" "In1.Cu")"#));
        let blind = written(Some(span(1, 3, false)));
        assert!(blind.starts_with("(via blind "), "{blind}");
        assert!(blind.contains(r#"(layers "In1.Cu" "B.Cu")"#));
        let buried = written(Some(span(1, 2, false)));
        assert!(buried.starts_with("(via buried "), "{buried}");
        assert!(buried.contains(r#"(layers "In1.Cu" "In2.Cu")"#));
        // The kind is part of the via's identity (its uuid).
        let uuid = |text: &str| text[text.find("(uuid").unwrap()..].to_string();
        assert_ne!(uuid(&through), uuid(&micro));
    }

    #[test]
    fn a_kicad_7_projects_switches_allow_the_spans_they_name() {
        let rules = serde_json::json!({"allow_blind_buried_vias": true, "allow_microvias": false});
        assert_eq!(project_spans(&rules), ["blind", "buried"]);
        let rules = serde_json::json!({"allow_microvias": true});
        assert_eq!(project_spans(&rules), ["micro"]);
        assert!(project_spans(&serde_json::json!({})).is_empty());
    }
}

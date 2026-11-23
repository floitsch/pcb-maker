// Copyright (C) 2026 Florian Loitsch.

use super::KiCadProjectRulesReport;
use crate::designer_pours::{
    KiCadPourTerminal, KiCadPourTrack, RoutedCopper, routed_copper, terminals,
};
use crate::kicad_nets::net_codes;
use crate::project_rules::width_class;
use crate::{DrcTally, Expr, NetNames};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadClassTrackWidths {
    pub net: String,
    pub class: String,
    /// The preferred width in the saved project's net class.
    pub class_width_mm: f64,
    pub tracks: Vec<KiCadPourTrack>,
    /// Length measured below the class's preferred width.
    pub below_class_width_mm: f64,
    /// Active native width errors naming this net, when DRC was run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_width_errors: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<KiCadReferenceTrackWidths>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadReferenceTrackWidths {
    pub board: PathBuf,
    pub terminals: Vec<KiCadPourTerminal>,
    pub tracks: Vec<KiCadPourTrack>,
}

fn width_lengths(copper: RoutedCopper) -> Vec<KiCadPourTrack> {
    copper
        .tracks
        .into_iter()
        .map(|((layer, nanometres), length_mm)| KiCadPourTrack {
            layer,
            width_mm: nanometres as f64 / 1_000_000.0,
            length_mm,
        })
        .collect()
}

pub(super) fn matching_terminals(
    candidate: &Expr,
    reference: &Expr,
) -> BTreeMap<String, BTreeSet<KiCadPourTerminal>> {
    let mut groups = terminals(candidate, &net_codes(candidate));
    let reference = terminals(reference, &net_codes(reference));
    groups.retain(|name, group| reference.get(name) == Some(group));
    groups
}

pub(crate) fn class_track_widths(
    rules: &KiCadProjectRulesReport,
    board: &Expr,
    native: Option<&DrcTally>,
) -> Vec<KiCadClassTrackWidths> {
    let Some(default_width) = rules.class_values.get("Default").and_then(|width| width[0]) else {
        return Vec::new();
    };
    let names = NetNames::of(board);
    let wide_class = |net: &str| {
        let class = width_class(
            rules.net_classes.get(names.normalize(net)),
            &rules.class_values,
        );
        let width = rules.class_values.get(class)?[0]?;
        (width > default_width + 1.0e-6).then_some((class, width))
    };
    let Ok(copper) = routed_copper(board, &net_codes(board), |net| wide_class(net).is_some())
    else {
        return Vec::new();
    };
    copper
        .into_iter()
        .filter_map(|(net, copper)| {
            let (class, class_width_mm) = wide_class(&net)?;
            let tracks = width_lengths(copper);
            if tracks.is_empty() {
                return None;
            }
            let below_class_width_mm = tracks
                .iter()
                .filter(|track| track.width_mm + 1.0e-6 < class_width_mm)
                .fold(0.0, |length, track| length + track.length_mm);
            let native_width_errors = native.map(|drc| {
                drc.findings
                    .iter()
                    .filter(|finding| {
                        matches!(finding.kind.as_str(), "track_width" | "connection_width")
                            && finding
                                .nets
                                .iter()
                                .any(|name| names.normalize(name) == names.normalize(&net))
                    })
                    .count()
            });
            Some(KiCadClassTrackWidths {
                net,
                class: class.into(),
                class_width_mm,
                tracks,
                below_class_width_mm,
                native_width_errors,
                reference: None,
            })
        })
        .collect()
}

pub(super) fn add_reference_track_widths(
    widths: &mut [KiCadClassTrackWidths],
    path: &Path,
    reference: &Expr,
    matching: &BTreeMap<String, BTreeSet<KiCadPourTerminal>>,
) {
    if widths.is_empty() {
        return;
    }
    let nets: BTreeSet<_> = widths.iter().map(|width| width.net.as_str()).collect();
    let Ok(mut copper) = routed_copper(reference, &net_codes(reference), |name| {
        nets.contains(name) && matching.contains_key(name)
    }) else {
        return;
    };
    for width in widths {
        let Some(reference_copper) = copper.remove(&width.net) else {
            continue;
        };
        let tracks = width_lengths(reference_copper);
        if !tracks.is_empty() {
            width.reference = Some(KiCadReferenceTrackWidths {
                board: path.into(),
                terminals: matching[&width.net].iter().cloned().collect(),
                tracks,
            });
        }
    }
}

pub(crate) fn wide_track_actions(widths: &[KiCadClassTrackWidths]) -> Vec<String> {
    widths.iter().filter(|width| width.below_class_width_mm > 0.001).take(4).map(|width| {
        let native = width.native_width_errors.map_or(String::new(), |errors| {
                format!(" KiCad reported {errors} active width {} naming this net.",
                    if errors == 1 { "error" } else { "errors" })
        });
            let reference = if width.reference.is_some() {
                " and the reference route for the same terminals"
            } else {
                ""
            };
        format!(
                "{} uses {:.3} mm of track below its class {} preferred width of {} mm. See class_track_widths for the measured widths by layer{reference}.{native}",
            width.net, width.below_class_width_mm, width.class, width.class_width_mm,
        )
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, tally_drc};
    use serde_json::json;

    fn fixture() -> Expr {
        parse(
            r#"(kicad_pcb
            (footprint "J" (property "Reference" "J6") (pad "1" smd rect (net "/bat_+")))
            (footprint "Q" (property "Reference" "Q2") (pad "3" smd rect (net "/bat_+")))
            (segment (start 1 0) (end 1.5 0) (width 0.127) (layer "B.Cu") (net "/bat_+"))
            (segment (start 1.5 0) (end 11.5 0) (width 1) (layer "F.Cu") (net "/bat_+")))"#,
        )
        .unwrap()
    }

    #[test]
    fn wide_classes_report_both_track_widths_and_separate_native_errors() {
        let rules = KiCadProjectRulesReport {
            net_classes: [(
                "bat_+".into(),
                vec!["ClearanceOnly".into(), "BatteryFeed".into()],
            )]
            .into(),
            class_values: [
                ("BatteryFeed".into(), [Some(1.0), None]),
                ("ClearanceOnly".into(), [None, Some(0.35)]),
                ("Default".into(), [Some(0.2), None]),
            ]
            .into(),
            ..Default::default()
        };
        let board = fixture();
        let report = tally_drc(&json!({"violations":[
            {"type":"track_width", "severity":"error", "items":[{"description":"Track [/bat_+] on B.Cu"}]},
            {"type":"track_width", "severity":"error", "items":[{"description":"Track [different] on B.Cu"}]},
            {"type":"track_width", "severity":"warning", "items":[{"description":"Track [/bat_+] on B.Cu"}]}
        ]}));
        let mut widths = class_track_widths(&rules, &board, Some(&report));
        assert_eq!(widths.len(), 1);
        assert_eq!(widths[0].class, "BatteryFeed");
        assert_eq!(widths[0].tracks.len(), 2);
        assert_eq!(widths[0].below_class_width_mm, 0.5);
        assert_eq!(widths[0].native_width_errors, Some(1));
        assert_eq!(
            class_track_widths(&rules, &board, None)[0].native_width_errors,
            None
        );
        let wrong_pad = parse(&crate::encode(&board).replace("(pad \"3\"", "(pad \"4\"")).unwrap();
        for reference in [&wrong_pad, &board] {
            add_reference_track_widths(
                &mut widths,
                Path::new("source.kicad_pcb"),
                reference,
                &matching_terminals(&board, reference),
            );
            assert_eq!(widths[0].reference.is_some(), reference == &board);
        }
        assert!(wide_track_actions(&widths)[0].contains("KiCad reported 1 active width error"));
    }
}

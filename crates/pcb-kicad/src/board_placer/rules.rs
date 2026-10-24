// Copyright (C) 2026 Toit contributors.

//! The project's rules the placer reads: clearances, courtyard overlap
//! and hole rules, the mask margin.

use super::*;

/// Whether the project's DRC lets courtyards overlap (its severity is not
/// `error`; KiCad's default is).
/// Whether the project's DRC makes a hole inside a courtyard an error
/// (KiCad's default ignores it).
pub(crate) fn holes_in_courtyards_error(project: &Path) -> bool {
    fs::read_to_string(project)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|project| {
            let severities = &project["board"]["design_settings"]["rule_severities"];
            ["pth_inside_courtyard", "npth_inside_courtyard"]
                .iter()
                .any(|rule| severities[rule].as_str() == Some("error"))
        })
}

pub(crate) fn courtyards_may_overlap(project: &Path) -> bool {
    fs::read_to_string(project)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|project| {
            project["board"]["design_settings"]["rule_severities"]["courtyards_overlap"]
                .as_str()
                .map(|severity| severity != "error")
        })
        .unwrap_or(false)
}

/// The largest copper clearance of a router configuration's rules: its
/// classes', and a custom rule's between the pads of every two nets on
/// some layer (`KiCadProjectRulesReport::pad_clearance_mm`: OpenESC's 0.16
/// mm on the outer layers, a pair rule of the router that no class shows;
/// without it the placer packed pads 0.09 mm apart and KiCad found 170
/// clearance errors between them).
pub(crate) fn largest_clearance(config: &KiCadBoardRouterConfig) -> f64 {
    config
        .connection_rules
        .values()
        .chain(config.default_rules.iter())
        .map(|rules| rules.clearance_mm)
        .chain(
            config
                .project_rules
                .as_ref()
                .and_then(|report| report.pad_clearance_mm),
        )
        .fold(0.0, f64::max)
}

/// How far a footprint's mask opening keeps other parts beyond the bodies'
/// spacing (at least the copper clearance): KiCad's solder_mask_bridge
/// flags another footprint's pad on the mask layer within the minimum web
/// plus the pad's expansion (the board's `pad_to_mask_clearance`; a
/// graphic opening has none), and covered copper within the project's
/// `solder_mask_to_copper_clearance`. Zero where the copper clearance is
/// the larger (most boards).
pub(super) fn mask_opening_growth(pcb: &Expr, config: &KiCadBoardPlacerConfig) -> f64 {
    let setup = |key: &str| {
        pcb.child("setup")
            .and_then(|setup| form_f64(setup, key, 1).ok())
            .unwrap_or(0.0)
            .max(0.0)
    };
    let margin = (setup("solder_mask_min_width") + setup("pad_to_mask_clearance"))
        .max(config.solder_mask_to_copper_mm.unwrap_or(0.0).max(0.0));
    (margin - config.copper_clearance_mm.unwrap_or(0.2)).max(0.0)
}

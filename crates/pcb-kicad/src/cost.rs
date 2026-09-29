// Copyright (C) 2026 Toit contributors.

//! What a board costs at the common fabs, from the price snapshot of
//! 2026-09-29 (docs/cost.md): board price, the surcharges the layout
//! triggers, JLCPCB assembly, and the thresholds worth staying under.
//! Estimates for comparing layouts, not quotes.

use super::*;

/// The layout facts fabs price by.
#[derive(Clone, Debug, Default)]
pub(crate) struct CostInputs {
    pub width_mm: f64,
    pub height_mm: f64,
    pub layers: usize,
    pub smallest_drill_mm: Option<f64>,
    pub narrowest_track_mm: Option<f64>,
    pub vias_in_pads: usize,
    pub smd_sides: usize,
    pub unique_parts: usize,
    pub smd_joints: usize,
    pub tht_joints: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadCost {
    pub quantity: usize,
    pub fabs: Vec<KiCadFabQuote>,
    /// Thresholds this board crosses and what staying under them saves.
    pub hints: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadFabQuote {
    pub fab: String,
    pub currency: String,
    pub board: f64,
    /// Surcharges the layout triggers, by cause.
    pub surcharges: Vec<(String, f64)>,
    /// Assembly without parts, where the snapshot allows an estimate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assembly: Option<f64>,
    pub total: f64,
}

fn cents(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// Board price at JLCPCB (USD): flat while both sides are at most 102 mm.
fn jlcpcb_board(width: f64, height: f64, layers: usize, quantity: usize) -> f64 {
    let total_m2 = width * height * quantity as f64 / 1.0e6;
    if width.max(height) <= 102.0 && quantity <= 10 {
        match (layers <= 2, quantity <= 5) {
            (true, true) => 4.0,
            (true, false) => 5.0,
            (false, _) => 8.0,
        }
    } else if layers <= 2 {
        4.0 + 100.0 * total_m2
    } else {
        // Fitted to the 103 x 103 mm, 5-piece jump ($31.60).
        31.6 * (total_m2 / 0.053).max(1.0).powf(0.8)
    }
}

/// Board price at PCBWay (USD): flat while both sides are at most 100 mm.
fn pcbway_board(width: f64, height: f64, layers: usize, quantity: usize) -> f64 {
    let total_m2 = width * height * quantity as f64 / 1.0e6;
    if width.max(height) <= 100.0 && quantity <= 10 {
        if layers <= 2 { 5.0 } else if quantity <= 5 { 26.0 } else { 51.0 }
    } else if layers <= 2 {
        37.0 + 99.0 * total_m2
    } else {
        77.0 + 150.0 * total_m2
    }
}

pub(crate) fn estimate(inputs: &CostInputs, quantity: usize) -> KiCadCost {
    let (w, h) = (inputs.width_mm, inputs.height_mm);
    let area_cm2 = w * h / 100.0;
    let q = quantity as f64;
    let layers = inputs.layers.max(1);
    let drill = inputs.smallest_drill_mm.unwrap_or(0.3);
    let track = inputs.narrowest_track_mm.unwrap_or(0.2);
    let mut fabs = Vec::new();
    let quote = |fab: &str, currency: &str, board: f64, surcharges: Vec<(String, f64)>, assembly: Option<f64>| {
        let total = board + surcharges.iter().map(|(_, cost)| cost).sum::<f64>() + assembly.unwrap_or(0.0);
        KiCadFabQuote {
            fab: fab.into(),
            currency: currency.into(),
            board: cents(board),
            surcharges: surcharges.into_iter().map(|(cause, cost)| (cause, cents(cost))).collect(),
            assembly: assembly.map(cents),
            total: cents(total),
        }
    };

    // JLCPCB, with assembly: Economic for one side, Standard for two.
    let mut surcharges = Vec::new();
    if drill < 0.2 - 1e-9 {
        surcharges.push(("via drill under 0.2 mm".into(), 34.0));
    } else if drill < 0.3 - 1e-9 {
        surcharges.push(("via drill under 0.3 mm".into(), 17.0));
    }
    let tht = if inputs.tht_joints > 0 { 3.58 + 0.0164 * inputs.tht_joints as f64 * q } else { 0.0 };
    let smd = 0.0016 * inputs.smd_joints as f64 * q;
    let assembly = if inputs.smd_sides == 0 && inputs.tht_joints == 0 {
        None
    } else if inputs.smd_sides <= 1 {
        Some(9.71 + smd + tht)
    } else {
        Some(33.77 * inputs.smd_sides as f64 + 1.53 * inputs.unique_parts as f64 + smd + tht + 14.93)
    };
    fabs.push(quote("JLCPCB", "USD", jlcpcb_board(w, h, layers, quantity), surcharges, assembly));

    let mut surcharges = Vec::new();
    if drill < 0.2 - 1e-9 {
        surcharges.push(("via drill under 0.2 mm".into(), 200.0));
    }
    if track < 0.1 - 1e-9 {
        surcharges.push(("tracks under 4 mil".into(), 140.0));
    }
    fabs.push(quote("PCBWay", "USD", pcbway_board(w, h, layers, quantity), surcharges, None));

    // Aisler: HASL rate for two layers within its rules, else ENIG.
    let rate = if layers > 2 {
        0.13
    } else if track >= 0.2 - 1e-9 && drill >= 0.3 - 1e-9 {
        0.067
    } else {
        0.097
    };
    let boards = 3.0 * (q / 3.0).ceil();
    fabs.push(quote("Aisler", "EUR", 14.0 + rate * area_cm2 * boards, Vec::new(), None));

    let base = (58.0 + 0.13 * area_cm2 * q) * if layers > 2 { 1.47 } else { 1.0 };
    let mut surcharges = Vec::new();
    if track < 0.125 - 1e-9 {
        surcharges.push(("tracks under 125 µm".into(), base * 0.4));
    }
    fabs.push(quote("Eurocircuits", "EUR", base, surcharges, None));

    fabs.push(quote(
        "Multi-CB (Saving)",
        "EUR",
        (12.0 + 0.04 * area_cm2 * q) * if layers > 2 { 2.0 } else { 1.0 },
        Vec::new(),
        None,
    ));
    fabs.push(quote(
        "OSH Park",
        "USD",
        0.775 * area_cm2 * (q / 3.0).ceil() * if layers > 2 { 2.0 } else { 1.0 },
        Vec::new(),
        None,
    ));

    // Thresholds worth staying under.
    let mut hints = Vec::new();
    if w.max(h) > 100.0 {
        let inside = pcbway_board(w.min(100.0), h.min(100.0), layers, quantity);
        hints.push(format!(
            "a side is over 100 mm: at PCBWay ${:.2} instead of ${:.2}, at JLCPCB ${:.2} instead of ${:.2} (both sides at most 102 mm)",
            pcbway_board(w, h, layers, quantity),
            inside,
            jlcpcb_board(w, h, layers, quantity),
            jlcpcb_board(w.min(102.0), h.min(102.0), layers, quantity),
        ));
    }
    if drill < 0.3 - 1e-9 {
        hints.push(format!("the smallest via drill is {drill} mm: 0.3 mm or more avoids $17-34 at JLCPCB"));
    }
    if inputs.vias_in_pads > 0 {
        hints.push(format!(
            "{} vias in pads: left open they wick solder; filled and capped they cost $17-50 at JLCPCB, about $100 at PCBWay, €67 at Eurocircuits",
            inputs.vias_in_pads
        ));
    }
    if inputs.smd_sides > 1 {
        hints.push("SMD parts on both sides: one side allows JLCPCB Economic assembly, about $60 less".into());
    }
    KiCadCost { quantity, fabs, hints }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_follow_the_snapshot_and_its_cliffs() {
        let small = CostInputs { width_mm: 50.0, height_mm: 50.0, layers: 2, ..Default::default() };
        let cost = estimate(&small, 10);
        let fab = |name: &str| cost.fabs.iter().find(|fab| fab.fab == name).unwrap().clone();
        assert_eq!(fab("JLCPCB").board, 5.0);
        assert_eq!(fab("PCBWay").board, 5.0);
        // Aisler: €14 + 0.067/cm² · 25 cm² · 12 boards.
        assert_eq!(fab("Aisler").board, cents(14.0 + 0.067 * 25.0 * 12.0));
        assert!(cost.hints.is_empty());
        // Just over 100 mm: PCBWay leaves its flat price.
        let wide = CostInputs { width_mm: 101.0, height_mm: 100.0, layers: 2, ..Default::default() };
        let cost = estimate(&wide, 10);
        assert!(cost.fabs.iter().find(|fab| fab.fab == "PCBWay").unwrap().board > 30.0);
        assert_eq!(cost.fabs.iter().find(|fab| fab.fab == "JLCPCB").unwrap().board, 5.0);
        assert_eq!(cost.hints.len(), 1);
        // Small drills and two assembly sides cost extra; vias in pads are
        // a hint (filling them is optional).
        let busy = CostInputs {
            smallest_drill_mm: Some(0.25),
            vias_in_pads: 2,
            smd_sides: 2,
            unique_parts: 10,
            smd_joints: 100,
            ..small
        };
        let cost = estimate(&busy, 10);
        let jlc = cost.fabs.iter().find(|fab| fab.fab == "JLCPCB").unwrap();
        assert_eq!(jlc.surcharges.len(), 1);
        assert!(jlc.assembly.unwrap() > 60.0);
        assert_eq!(cost.hints.len(), 3);
    }
}

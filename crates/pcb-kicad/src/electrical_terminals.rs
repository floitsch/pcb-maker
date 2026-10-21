// Copyright (C) 2026 Toit contributors.

use super::*;

pub(super) fn plated_pad(pad: &Expr) -> bool {
    pad.children().get(2).and_then(Expr::atom) == Some("thru_hole")
        && pad.child("drill").is_some_and(|drill| {
            drill
                .children()
                .iter()
                .skip(1)
                .filter_map(Expr::atom)
                .filter_map(|value| value.parse::<f64>().ok())
                .any(|diameter| diameter.is_finite() && diameter > 0.0)
        })
}

pub(super) fn contact_layers(layers: [bool; 2], plated: bool) -> Vec<[bool; 2]> {
    if layers == [true, true] && !plated {
        vec![[true, false], [false, true]]
    } else {
        vec![layers]
    }
}

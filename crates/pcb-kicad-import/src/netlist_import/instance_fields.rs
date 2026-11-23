// Copyright (C) 2026 Florian Loitsch.

//! Own-instance sourcing fields proved by the schematic. A shared land
//! pattern's library file has no one instance's catalog assignments.

use super::*;

pub(super) type VerifiedFields = BTreeMap<String, BTreeMap<String, String>>;

pub(super) fn sourcing_field(name: &str) -> bool {
    matches!(name, "Datasheet" | "Description" | "LCSC" | "MFG" | "MFGPN")
}

/// Remove only hidden fabrication properties whose exact contents are
/// restored on the board by `set_symbol_fields`. Unknown properties,
/// repeated names, visible text and other layers still define the model.
pub(super) fn project_sourcing_fields(model: &mut Expr, verified: &BTreeMap<String, String>) {
    let Expr::List(items) = model else { return };
    let property = |item: &Expr| {
        (item.head() == Some("property"))
            .then(|| item.children().get(1).and_then(Expr::atom))
            .flatten()
            .map(str::to_owned)
    };
    let mut counts = BTreeMap::<String, usize>::new();
    for name in items.iter().filter_map(property) {
        *counts.entry(name).or_default() += 1;
    }
    items.retain(|item| {
        let Some(name) = property(item) else {
            return true;
        };
        let value = item.children().get(2).and_then(Expr::atom);
        !(counts.get(&name) == Some(&1)
            && verified
                .get(&name)
                .map(String::as_str)
                .is_some_and(|expected| value == Some(expected))
            && form_atom(item, "hide", 1) == Some("yes")
            && matches!(form_atom(item, "layer", 1), Some("F.Fab" | "B.Fab")))
    });
}

/// A new shared library model gets standard Reference and Value defaults.
/// Its board instances keep their own forms, including original caches.
pub(super) fn clear_example_text(model: &mut Expr, footprint_name: &str) {
    let Expr::List(items) = model else { return };
    for item in items {
        let Some(value) = (match (item.head(), item.children().get(1).and_then(Expr::atom)) {
            (Some("property"), Some("Reference")) | (Some("fp_text"), Some("reference")) => {
                Some("REF**")
            }
            (Some("property"), Some("Value")) | (Some("fp_text"), Some("value")) => {
                Some(footprint_name)
            }
            _ => None,
        }) else {
            continue;
        };
        let Expr::List(parts) = item else { continue };
        if let Some(text) = parts.get_mut(2) {
            *text = Expr::Atom(requote(value));
            parts.retain(|part| part.head() != Some("render_cache"));
        }
    }
}

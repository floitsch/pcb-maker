// Copyright (C) 2026 Toit contributors.

//! What the lowering leaves out without an error, for the report.

use super::*;

/// What the lowering leaves out of the board without an error, one line
/// each, for the report's `warnings`: copper on a layer the board's stack
/// lacks (a typo, `In3.Cu` on a two-layer board: KiCad warns when it
/// loads such a board, the router routes through it) and text boxes of
/// fewer than three corners (no box to keep away from).
pub(crate) fn input_warnings(pcb: &Expr) -> Vec<String> {
    let Ok(layers) = LayerTable::from_pcb(pcb) else {
        return Vec::new();
    };
    // (kind, layer) -> (count, the first's position).
    type Seen = (usize, Option<[f64; 2]>);
    let mut unknown: BTreeMap<(&str, &str), Seen> = BTreeMap::new();
    let mut boxes = Vec::new();
    for item in pcb.children() {
        let kind = match item.head() {
            Some(kind @ ("segment" | "arc" | "via" | "zone")) => kind,
            Some("footprint") => {
                let reference = footprint_reference(item).unwrap_or_default();
                for text in item
                    .children()
                    .iter()
                    .filter(|child| child.head() == Some("fp_text_box"))
                {
                    if !geometry::text_hidden(text)
                        && matches!(geometry::text_box_shapes(text), Ok(shapes) if shapes.is_empty())
                    {
                        boxes.push(format!(
                            "{reference}'s text box has fewer than three corners: it is left out"
                        ));
                    }
                }
                continue;
            }
            Some("gr_text_box") => {
                if !geometry::text_hidden(item)
                    && matches!(geometry::text_box_shapes(item), Ok(shapes) if shapes.is_empty())
                {
                    boxes.push(
                        "a board text box has fewer than three corners: it is left out".into(),
                    );
                }
                continue;
            }
            _ => continue,
        };
        for name in item_layer_names(item) {
            if name.ends_with(".Cu")
                && !matches!(name, "*.Cu" | "F&B.Cu")
                && layers.index(name).is_none()
            {
                let at = form_xy(item, "start").or_else(|_| form_xy(item, "at")).ok();
                let entry = unknown.entry((kind, name)).or_insert((0, at));
                entry.0 += 1;
            }
        }
    }
    let stack = layers.names.join(", ");
    let mut warnings: Vec<String> = unknown
        .into_iter()
        .map(|((kind, layer), (count, at))| {
            format!(
                "{count} {kind}{} on {layer}{}, which the board's copper layers ({stack}) do not include: the router ignores {} there",
                if count == 1 { "" } else { "s" },
                at.map_or(String::new(), |at| format!(" (the first at {:.3}, {:.3})", at[0], at[1])),
                if count == 1 { "it" } else { "them" }
            )
        })
        .collect();
    warnings.extend(boxes);
    warnings
}

// Copyright (C) 2026 Florian Loitsch.

//! A board outline carried by its own perimeter contacts.

use super::*;

/// A held, schematic-linked footprint can describe the entire PCB and
/// its edge contacts. Without a separate body drawn or modeled, its
/// contact rows do not imply an IC-sized body filling the PCB interior.
pub(super) fn perimeter_contacts(
    context: &LowerContext,
    footprint: &Expr,
    pads: &[[f64; 4]],
) -> bool {
    let edge = |item: &Expr| form_atom(item, "layer", 1) == Some("Edge.Cuts");
    if !footprint_locked(footprint)
        || footprint.child("path").is_none()
        || footprint.child("model").is_some()
        || footprint_reference(footprint)
            .is_some_and(|reference| context.config.free.contains(&reference))
        || context.pcb.children().iter().any(edge)
        || footprint.children().iter().any(|item| {
            matches!(
                item.head(),
                Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly" | "fp_curve")
            ) && matches!(form_atom(item, "layer", 1), Some("F.Fab" | "B.Fab"))
        })
    {
        return false;
    }
    let mut items = vec![Expr::Atom("footprint".into())];
    items.extend(
        footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("at") || edge(item))
            .cloned(),
    );
    let own_board = Expr::List(vec![Expr::Atom("kicad_pcb".into()), Expr::List(items)]);
    let Ok(own) = outline::board_loops_strict(&own_board) else {
        return false;
    };
    let [outer] = own.piece_loops.as_slice() else {
        return false;
    };
    if !outline::board_loops_strict(context.pcb).is_ok_and(|all| all.piece_loops == own.piece_loops)
    {
        return false;
    }
    let Ok([x, y, angle]) = form_at(footprint) else {
        return false;
    };
    let contact_count = pads
        .iter()
        .filter(|pad| {
            let offset = rotate_vector([(pad[0] + pad[2]) / 2.0, (pad[1] + pad[3]) / 2.0], -angle);
            let center = [x + offset[0], y + offset[1]];
            let radius_squared = ((pad[2] - pad[0]).powi(2) + (pad[3] - pad[1]).powi(2)) / 4.0;
            outer
                .iter()
                .zip(outer.iter().cycle().skip(1))
                .any(|(a, b)| point_segment_distance_squared(center, *a, *b) <= radius_squared)
        })
        .count();
    contact_count > pads.len() / 2
}

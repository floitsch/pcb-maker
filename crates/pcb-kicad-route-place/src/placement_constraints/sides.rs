// Copyright (C) 2026 Toit contributors.

//! `back` and `front`: the side a part goes on.

use super::*;

/// Flips the footprints `back` and `front` name onto that side.
pub(crate) fn apply_sides(
    pcb: &mut Expr,
    constraints: &KiCadPlacementConstraints,
) -> Result<(), String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut used = vec![false; constraints.back.len() + constraints.front.len()];
    for item in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = footprint_reference(item).unwrap_or_default();
        let mut side = None;
        for (index, (pattern, wanted)) in constraints
            .back
            .iter()
            .map(|pattern| (pattern, "B.Cu"))
            .chain(constraints.front.iter().map(|pattern| (pattern, "F.Cu")))
            .enumerate()
        {
            if glob_matches(pattern, &reference) {
                used[index] = true;
                if side.is_some_and(|side| side != wanted) {
                    return Err(format!(
                        "{reference} is named for both the front and the back"
                    ));
                }
                side = Some(wanted);
            }
        }
        if let Some(side) = side
            && form_atom(item, "layer", 1) != Some(side)
        {
            flip::flip_footprint(item)?;
        }
    }
    if let Some(index) = used.iter().position(|used| !used) {
        let pattern = constraints
            .back
            .iter()
            .chain(&constraints.front)
            .nth(index)
            .unwrap();
        return Err(format!("side constraint {pattern:?} matches no part"));
    }
    Ok(())
}

/// Reports the sides `apply_sides` flipped the parts to as kept
/// constraints, row members included (they lie on their row's side).
pub(super) fn report_sides(
    problem: &mut core::Problem,
    parts: &Parts,
    constraints: &KiCadPlacementConstraints,
) {
    // The side to keep is where the flipped footprint's pads put the part,
    // not the constraint's name for it: urchin's key sockets are front
    // footprints with their pads on B.Cu, so on B.Cu (as `back` asks) the
    // part lies on the front, and every `back` entry for them was reported
    // missed on a board that kept them all.
    for (kind, patterns) in [("back", &constraints.back), ("front", &constraints.front)] {
        for index in (0..parts.footprints).filter(|index| {
            patterns
                .iter()
                .any(|pattern| glob_matches(pattern, &parts.references[*index]))
        }) {
            let side = problem.components[index].side;
            problem.constraints.kept.push(core::constraints::Kept {
                kind,
                part: index,
                side: Some(side),
                angles: Vec::new(),
                position: None,
            });
        }
    }
}

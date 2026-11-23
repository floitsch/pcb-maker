// Copyright (C) 2026 Florian Loitsch.

//! Identify local shapes that are necessary for the mechanical perimeter.

use super::mechanical::{board_parts, edge};
use super::mechanical_parts::edge_cuts;
use super::*;
use pcb_kicad_file::outline::board_loops_strict;

/// Closed slots inside an intact outline do not determine its perimeter.
/// Compare the actual outer pieces with and without the candidate's edges.
pub(super) fn perimeter_owners(board: &Expr, eligible: impl Fn(&Expr) -> bool) -> Vec<&Expr> {
    let owners: Vec<_> = board_parts(board)
        .filter(|part| eligible(part) && part.children().iter().any(edge_cuts))
        .collect();
    if owners.is_empty() {
        return Vec::new();
    }
    let bare = outline_only(
        "kicad_pcb",
        board.children().iter().filter(|item| edge(item)),
    );
    let forms: Vec<_> = owners
        .iter()
        .map(|part| {
            outline_only(
                "footprint",
                part.children()
                    .iter()
                    .filter(|item| item.head() == Some("at") || edge_cuts(item)),
            )
        })
        .collect();
    let geometry = |omit: Option<usize>| {
        let mut children = bare.children().to_vec();
        children.extend(
            forms
                .iter()
                .enumerate()
                .filter(|(index, _)| omit != Some(*index))
                .map(|(_, form)| form.clone()),
        );
        board_loops_strict(&Expr::List(children))
    };
    let Ok(full) = geometry(None) else {
        // An incomplete source will fail closure on the generated board.
        // In particular, an invalid movable slot cannot supply the frame.
        return Vec::new();
    };
    if board_loops_strict(&bare).is_ok_and(|outer| outer.piece_loops == full.piece_loops) {
        return Vec::new();
    }
    owners
        .into_iter()
        .enumerate()
        .filter(|(index, _)| {
            !geometry(Some(*index)).is_ok_and(|remainder| remainder.piece_loops == full.piece_loops)
        })
        .map(|(_, part)| part)
        .collect()
}

pub(super) fn same_owner(a: &Expr, b: &Expr) -> bool {
    footprint_reference(a) == footprint_reference(b)
        && form_atom(a, "path", 1) == form_atom(b, "path", 1)
}

/// When the existing PCB moves a perimeter onto board graphics, keep
/// that edit once instead of also inheriting the library's local outline.
pub(super) fn detached_outlines(
    existing: &Expr,
    donor_outline: &[&Expr],
    imported: &[(usize, &Expr)],
) -> Vec<usize> {
    donor_outline
        .iter()
        .filter_map(|owner| {
            let mut edits = board_parts(existing).filter(|part| same_owner(part, owner));
            let edited = edits.next()?;
            if edits.next().is_some() || edited.children().iter().any(edge_cuts) {
                return None;
            }
            let mut matches = imported.iter().filter(|(_, part)| same_owner(part, edited));
            let (index, _) = matches.next()?;
            matches.next().is_none().then_some(*index)
        })
        .collect()
}

fn outline_only<'a>(head: &str, forms: impl Iterator<Item = &'a Expr>) -> Expr {
    Expr::List(
        std::iter::once(Expr::Atom(head.into()))
            .chain(forms.cloned())
            .collect(),
    )
}

/// Numbered pad sites check that a rebuilt footprint uses the source's
/// local origin and face before attaching a perimeter to it. The target's
/// pad definitions and nets remain schematic-derived.
pub(super) fn same_pad_sites(source: &Expr, target: &Expr) -> bool {
    fn sites(part: &Expr) -> Option<BTreeMap<(&str, i64, i64), usize>> {
        let mut sites = BTreeMap::new();
        for pad in part
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            let number = pad.children().get(1)?.atom()?;
            if number.is_empty() {
                continue;
            }
            let at = if pad.child("at").is_some() {
                form_at(pad).ok()?
            } else {
                [0.0; 3]
            };
            if !at[0].is_finite() || !at[1].is_finite() {
                return None;
            }
            *sites
                .entry((
                    number,
                    (at[0] * 1e6).round() as i64,
                    (at[1] * 1e6).round() as i64,
                ))
                .or_default() += 1;
        }
        Some(sites)
    }
    matches!((sites(source), sites(target)), (Some(a), Some(b)) if !a.is_empty() && a == b)
}

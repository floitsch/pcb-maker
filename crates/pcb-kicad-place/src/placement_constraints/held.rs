// Copyright (C) 2026 Toit contributors.

//! The parts the placer holds where the input has them (`HeldParts`),
//! their pads and courtyards, and the input check's warnings for held
//! parts on top of each other.

use super::*;
use crate::footprint_bodies::{PadBox, pad_boxes};

/// The parts the placer leaves where the input has them, one flag per
/// footprint in the board's order (`held_parts`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeldParts(pub Vec<bool>);

impl HeldParts {
    /// Whether the board's footprint `index` (among its footprints) is held.
    pub fn holds(&self, index: usize) -> bool {
        self.0.get(index).copied().unwrap_or(false)
    }
}

/// The one notion of a held part (2026-10-21): the footprints of `pcb`
/// that the placer leaves where they are, as the lowering decides it with
/// `config` (its constraints resolved, the outline they give applied to
/// `pcb` already). Held are
/// - the placer's `fixed` and `fixed_patterns`, constraints.json's `fixed`
///   and parts it places with `x` and `y` (or adds there);
/// - by default, unless `free` names them: locked footprints, footprints
///   without a net, and parts at the edge (a body off the outline, a
///   connector within `edge_keep_mm` of it, a part in a designer's rule
///   area that forbids footprints);
/// - but not the default ones a constraint names (an edge, a region, a
///   side, a place without coordinates) nor, under `move_all`, any part
///   but locked ones, artwork and parts nobody can name; a row's members
///   are held when its leader is.
///
/// The keepout constraints (which rule areas allow footprints), the free
/// dimension (which parts must stay inside the outline) and the placer
/// itself use it.
pub fn held_parts(pcb: &Expr, config: &KiCadBoardPlacerConfig) -> Result<HeldParts, String> {
    let mut config = config.clone();
    let mut board = pcb.clone();
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &mut config.constraints {
        expand_additions(constraints, &board)?;
        // A part on `"edge": "any"` moves whichever edge it settles to.
        for entry in &mut constraints.edge {
            if entry.edge == "any" {
                entry.edge = "bottom".into();
            }
        }
        apply_sides(&mut board, constraints)?;
    }
    let footprints = board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .count();
    let lowered = crate::board_placer::lower_placement(&board, &config)?;
    let mut held: Vec<bool> = lowered.problem.components[..footprints]
        .iter()
        .map(|component| component.fixed)
        .collect();
    // A row's members are fixed in the problem only because their leader
    // carries them: they are held when it is.
    for follower in &lowered.problem.constraints.followers {
        if follower.part < footprints {
            held[follower.part] = held.get(follower.leader).copied().unwrap_or(false);
        }
    }
    Ok(HeldParts(held))
}

/// `held_parts` for a board as given, with the outline its constraints
/// give (a rectangle of `width` and `height`, an outline sized from the
/// parts) applied first.
pub fn held_parts_as_given(
    pcb: &Expr,
    config: &KiCadBoardPlacerConfig,
) -> Result<HeldParts, String> {
    let Some(KiCadConstraintsSource::Inline(constraints)) = &config.constraints else {
        return held_parts(pcb, config);
    };
    let Some(outline) = &constraints.outline else {
        return held_parts(pcb, config);
    };
    let mut board = pcb.clone();
    let mut part_area = 0.0;
    for footprint in board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let (_, size, _) = crate::footprint_bodies::local_body(footprint)?;
        part_area += (size[0] + config.spacing_mm) * (size[1] + config.spacing_mm);
    }
    apply_outline(&mut board, outline, part_area)?;
    held_parts(&board, config)
}

/// Whether two pads' copper layer lists share a layer (`*.Cu` and
/// `F&B.Cu` reach both outer sides, which is what the parts' pads use).
fn pad_layers_meet(a: &[String], b: &[String]) -> bool {
    let wide = |name: &str| name == "*.Cu" || name == "F&B.Cu";
    a.iter()
        .any(|x| b.iter().any(|y| x == y || wide(x) || wide(y)))
}

/// A point of a shape's copper (for a containment test).
fn point_of(shape: &pcb_router::geometry::Shape) -> Option<[f64; 2]> {
    use pcb_router::geometry::Shape;
    match shape {
        Shape::Circle { center, .. } => Some(*center),
        Shape::Capsule { start, .. } => Some(*start),
        Shape::Polygon { points } => points.first().copied(),
        Shape::Union { parts } => parts.iter().find_map(point_of),
    }
}

/// Whether two copper shapes touch or overlap: the distance between them
/// is zero (an edge of one crosses or touches the other, or one lies
/// inside the other).
fn shapes_meet(a: &pcb_router::geometry::Shape, b: &pcb_router::geometry::Shape) -> bool {
    use pcb_router::geometry::Shape;
    match b {
        Shape::Circle { center, radius } => a.distance_to_point(*center) <= *radius,
        Shape::Capsule { start, end, radius } => a.distance_to_segment(*start, *end) <= *radius,
        Shape::Polygon { points } => {
            points.iter().enumerate().any(|(index, start)| {
                let end = points[(index + 1) % points.len()];
                a.distance_to_segment(*start, end) <= 0.0
            }) || point_of(a).is_some_and(|point| b.contains(point))
        }
        Shape::Union { parts } => parts.iter().any(|part| shapes_meet(a, part)),
    }
}

/// The area two boxes share.
fn box_overlap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let w = a[2].min(b[2]) - a[0].max(b[0]);
    let h = a[3].min(b[3]) - a[1].max(b[1]);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

/// Pairs of parts the placer keeps where they are (`HeldParts`), each with
/// a pad on a net, that lie on top of each other: their courtyards on one
/// side share half the smaller one or more, or pads of the two meet on a
/// copper layer (by 0.02 mm or more). A stack the input fixed places and
/// routes without a word otherwise, and the board comes back with opens
/// (fence issue 5: a fixed_patterns glob that fixed six resistors stacked
/// at their initial position). Each pair once, in reference order, with
/// what overlaps (`courtyards`, `pads on F.Cu`).
pub(crate) fn held_parts_overlapping(
    pcb: &Expr,
    held: &HeldParts,
) -> Result<Vec<(String, String, String)>, String> {
    struct Part {
        reference: String,
        courtyard: Option<([f64; 4], usize)>,
        pads: Vec<PadBox>,
    }
    let mut parts = Vec::new();
    for (index, footprint) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .enumerate()
    {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let wired = footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
            .any(|pad| pad.child("net").is_some());
        if !wired || !held.holds(index) {
            continue;
        }
        parts.push(Part {
            courtyard: courtyard_box(footprint)?,
            pads: pad_boxes(footprint)?,
            reference,
        });
    }
    parts.sort_by(|a, b| crate::canonical_order::natural_cmp(&a.reference, &b.reference));
    let mut pairs = Vec::new();
    for (index, first) in parts.iter().enumerate() {
        for second in &parts[index + 1..] {
            let mut what = Vec::new();
            if let (Some((a, side_a)), Some((b, side_b))) = (first.courtyard, second.courtyard)
                && side_a == side_b
            {
                let smaller = ((a[2] - a[0]) * (a[3] - a[1])).min((b[2] - b[0]) * (b[3] - b[1]));
                if smaller > 0.0 && box_overlap(a, b) >= 0.5 * smaller {
                    what.push("courtyards".to_string());
                }
            }
            let mut layers: Vec<String> = Vec::new();
            for (a, layers_a, shape_a, net_a) in &first.pads {
                for (b, layers_b, shape_b, net_b) in &second.pads {
                    // Pads of one net on each other are joined, not in
                    // conflict: framework_mobo_lefthalf's R31 (a 0R) has
                    // its pad 1 soldered onto J1's B5, both Net-(J1-CC2).
                    if net_a.is_some() && net_a == net_b {
                        continue;
                    }
                    let shrunk =
                        |r: &[f64; 4]| [r[0] + 0.01, r[1] + 0.01, r[2] - 0.01, r[3] - 0.01];
                    // The boxes first, then the copper itself: an oval or
                    // round pad's box reaches past its copper
                    // (framework_mobo_lefthalf's R31 and R32 under J1's
                    // oval pads, 0.087 mm apart in KiCad, boxes 0.2 mm
                    // into each other, were warned of as pads that
                    // "cannot both be connected").
                    if box_overlap(shrunk(a), shrunk(b)) > 0.0
                        && pad_layers_meet(layers_a, layers_b)
                        && match (shape_a, shape_b) {
                            (Some(shape_a), Some(shape_b)) => shapes_meet(shape_a, shape_b),
                            _ => true,
                        }
                    {
                        let layer = layers_a
                            .iter()
                            .find(|x| layers_b.contains(x))
                            .or(layers_a.first())
                            .cloned()
                            .unwrap_or_default();
                        if !layers.contains(&layer) {
                            layers.push(layer);
                        }
                    }
                }
            }
            if !layers.is_empty() {
                what.push(format!("pads on {}", layers.join(", ")));
            }
            if !what.is_empty() {
                pairs.push((
                    first.reference.clone(),
                    second.reference.clone(),
                    what.join(" and "),
                ));
            }
        }
    }
    Ok(pairs)
}

/// The input check's warnings for fixed parts on top of each other
/// (`held_parts_overlapping`) on the project's board, before the run.
pub fn overlap_warnings(
    source_directory: &Path,
    board_id: &str,
    config: &KiCadBoardPlacerConfig,
) -> Result<Vec<String>, String> {
    let resolved = resolve_constraints(config, source_directory)?;
    let path = source_directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let pcb = parse(&text)?;
    // The parts the layout holds (`held_parts`); a board the lowering
    // refuses is the placement's to report, not the input check's.
    let held = held_parts_as_given(&pcb, &resolved).unwrap_or_default();
    let held = match &resolved.constraints {
        Some(KiCadConstraintsSource::Inline(constraints)) => {
            without_parts_placed_elsewhere(&pcb, held, constraints)
        }
        _ => held,
    };
    Ok(held_parts_overlapping(&pcb, &held)?
        .into_iter()
        .map(|(first, second, what)| overlap_warning(&first, &second, &what))
        .collect())
}

/// `held` without the parts a `place` entry puts at `x`, `y`: they are
/// held where the entry puts them, not where the input has them, so their
/// input position says nothing about an overlap (a fresh netlist import
/// stacks every part at one point: trial 4's six thumb keys placed at the
/// designer's positions gave 15 warnings "fixed parts SW19 and SW20
/// overlap" ahead of every other next action). The lowering checks the
/// held parts again at the positions the constraints give them.
pub(crate) fn without_parts_placed_elsewhere(
    pcb: &Expr,
    mut held: HeldParts,
    constraints: &KiCadPlacementConstraints,
) -> HeldParts {
    let placed: BTreeSet<&str> = constraints
        .place
        .iter()
        .filter(|entry| entry.x.is_some() && entry.y.is_some())
        .map(|entry| entry.part.as_str())
        .collect();
    if placed.is_empty() {
        return held;
    }
    let references: Vec<String> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .map(|footprint| footprint_reference(footprint).unwrap_or_default())
        .collect();
    // A row whose first part is placed carries its other parts there too
    // (trial 6: eight jacks in a row led by a placed J4 gave 21 warnings
    // "fixed parts J5 and J6 overlap" for the import's stack).
    let mut carried: BTreeSet<&str> = BTreeSet::new();
    for row in &constraints.row {
        let mut members: Vec<&str> = Vec::new();
        for pattern in &row.parts {
            let mut matched: Vec<&str> = references
                .iter()
                .filter(|reference| glob_matches(pattern, reference))
                .map(String::as_str)
                .collect();
            matched.sort_by_key(|reference| natural(reference));
            for reference in matched {
                if !members.contains(&reference) {
                    members.push(reference);
                }
            }
        }
        if members.first().is_some_and(|first| placed.contains(first)) {
            carried.extend(members);
        }
    }
    for (index, reference) in references.iter().enumerate() {
        if (placed.contains(reference.as_str()) || carried.contains(reference.as_str()))
            && let Some(slot) = held.0.get_mut(index)
        {
            *slot = false;
        }
    }
    held
}

/// The warning for two held parts on top of each other.
/// Only pads on each other cost a connection: parts in a courtyard of
/// another on purpose (jetson-nano-baseboard's 36 capacitors and
/// resistors under its SO-DIMM socket J15, trial 5) were said to have pads
/// that "cannot both be connected".
pub fn overlap_warning(first: &str, second: &str, what: &str) -> String {
    if what.contains("pads") {
        format!(
            "fixed parts {first} and {second} overlap ({what}): the placer keeps both where the input has them and their pads cannot both be connected; give one its own place (place, or free it from fixed and fixed_patterns)"
        )
    } else {
        format!(
            "fixed parts {first} and {second} overlap ({what}): the placer keeps both where the input has them; their pads are apart, so it costs no connection (a part under a socket or module on purpose); where it is a mistake give one its own place (place, or free it from fixed and fixed_patterns)"
        )
    }
}

/// What the input check warns of before a placement runs: keepouts over
/// held parts (`keepout_warnings`), then held parts on top of each other
/// (`overlap_warnings`). The commands print them and the verdict's
/// next_actions repeat them.
pub fn placement_input_warnings(
    source_directory: &Path,
    board_id: &str,
    config: &KiCadBoardPlacerConfig,
) -> Result<Vec<String>, String> {
    let mut warnings = keepout_warnings(source_directory, board_id, config)?;
    warnings.extend(overlap_warnings(source_directory, board_id, config)?);
    Ok(warnings)
}

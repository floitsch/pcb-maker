// Copyright (C) 2026 Toit contributors.

//! `keepout`: boxes no part may enter, written as KiCad rule areas, and
//! the warnings for held parts inside them.

use super::*;

/// A box no part may enter; written to the board as a KiCad rule area, so
/// the router and KiCad's DRC keep it too.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadKeepoutConstraint {
    pub x: [f64; 2],
    pub y: [f64; 2],
    /// As for regions: `board` (default) or `absolute`.
    #[serde(default)]
    pub origin: Option<String>,
    /// `front`, `back` or `both` (default).
    #[serde(default)]
    pub side: Option<String>,
    /// No tracks, vias or pours either: bare board (under an antenna, for
    /// a label or a mechanical part).
    #[serde(default)]
    pub copper: bool,
}

/// Names of the rule areas `apply_keepouts` writes. Parts inside one are
/// not there on purpose (unlike parts inside a designer's rule area).
pub(crate) const KEEPOUT_NAME: &str = "constraint keepout";

/// A keepout's box on the board ([x0, y0, x1, y1]) for an outline whose
/// top-left corner is `corner`.
fn keepout_box(
    keepout: &KiCadKeepoutConstraint,
    number: usize,
    corner: [f64; 2],
) -> Result<[f64; 4], String> {
    let offset = match keepout.origin.as_deref() {
        None | Some("board") => corner,
        Some("absolute") => [0.0, 0.0],
        Some(other) => {
            return Err(format!(
                "unknown keepout origin {other:?} (board or absolute)"
            ));
        }
    };
    let (x0, x1) = (
        offset[0] + keepout.x[0].min(keepout.x[1]),
        offset[0] + keepout.x[0].max(keepout.x[1]),
    );
    let (y0, y1) = (
        offset[1] + keepout.y[0].min(keepout.y[1]),
        offset[1] + keepout.y[0].max(keepout.y[1]),
    );
    if !(x1 > x0 && y1 > y0) {
        return Err(format!("keepout {} is empty", number + 1));
    }
    Ok([x0, y0, x1, y1])
}

/// Per keepout, the held parts (`HeldParts`) whose courtyard meets its
/// box on its side: the input put them there on purpose.
pub(crate) fn held_parts_in_keepouts(
    pcb: &Expr,
    keepouts: &[KiCadKeepoutConstraint],
    held: &HeldParts,
) -> Result<Vec<Vec<String>>, String> {
    if keepouts.is_empty() {
        return Ok(Vec::new());
    }
    let corner = outline_corner(pcb)?;
    let boxes: Vec<[f64; 4]> = keepouts
        .iter()
        .enumerate()
        .map(|(number, keepout)| keepout_box(keepout, number, corner))
        .collect::<Result<_, _>>()?;
    let mut inside = vec![Vec::new(); keepouts.len()];
    for (index, footprint) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .enumerate()
    {
        if !held.holds(index) {
            continue;
        }
        let reference = footprint_reference(footprint).unwrap_or_default();
        let Some((courtyard, side)) = courtyard_box(footprint)? else {
            continue;
        };
        for (number, keepout) in keepouts.iter().enumerate() {
            let on_side = match keepout.side.as_deref() {
                Some("front") => side == 0,
                Some("back") => side == 1,
                _ => true,
            };
            let area = boxes[number];
            if on_side
                && courtyard[0] < area[2]
                && area[0] < courtyard[2]
                && courtyard[1] < area[3]
                && area[1] < courtyard[3]
            {
                inside[number].push(reference.clone());
            }
        }
    }
    // In reference order, whatever the board file's order.
    for parts in &mut inside {
        parts.sort_by(|a, b| crate::canonical_order::natural_cmp(a, b));
    }
    Ok(inside)
}

/// The warning for keepout `number` (from 0) whose box meets the held
/// parts `parts`: its rule area allows footprints.
pub(crate) fn held_parts_warning(
    keepout: &KiCadKeepoutConstraint,
    number: usize,
    parts: &[String],
) -> String {
    format!(
        "keepout {} (x {}..{}, y {}..{}) meets the courtyards of parts the placer keeps where they are ({}): its rule area is written with footprints allowed, so KiCad does not report them; the placer still keeps movable parts out{}; shrink the box off them to have KiCad check it too",
        number + 1,
        keepout.x[0],
        keepout.x[1],
        keepout.y[0],
        keepout.y[1],
        parts.join(", "),
        if keepout.copper {
            ", and tracks, vias and pours stay out of it (pads inside it are reached from outside the box only)"
        } else {
            ""
        }
    )
}

/// The warnings `held_parts_in_keepouts` gives for a placement's
/// constraints on the project's board, before it runs. With an outline
/// constraint (the import writes `"outline": {}`) the boxes are measured
/// from the outline's corner: the outline is applied first, as the
/// placement applies it (`place_kicad_board`: the parts the constraints
/// add, then the outline sized by the parts' bodies), so the warnings are
/// the placement's own.
pub fn keepout_warnings(
    source_directory: &Path,
    board_id: &str,
    config: &KiCadBoardPlacerConfig,
) -> Result<Vec<String>, String> {
    let mut resolved = resolve_constraints(config, source_directory)?;
    if !matches!(&resolved.constraints, Some(KiCadConstraintsSource::Inline(constraints)) if !constraints.keepout.is_empty())
    {
        return Ok(Vec::new());
    }
    let path = source_directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut pcb = crate::canonical_order::parse_board(&text)?;
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &mut resolved.constraints {
        expand_additions(constraints, &pcb)?;
    }
    let Some(KiCadConstraintsSource::Inline(constraints)) = &resolved.constraints else {
        return Ok(Vec::new());
    };
    if let Some(outline) = &constraints.outline {
        let mut part_area = 0.0;
        for footprint in pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
        {
            let (_, size, _) = crate::board_placer::local_body(footprint)?;
            part_area += (size[0] + resolved.spacing_mm) * (size[1] + resolved.spacing_mm);
        }
        apply_outline(&mut pcb, outline, part_area)?;
    }
    let held = held_parts_in_keepouts(
        &pcb,
        &constraints.keepout,
        &held_parts(&pcb, &resolved).unwrap_or_default(),
    )?;
    Ok(held
        .iter()
        .enumerate()
        .filter(|(_, parts)| !parts.is_empty())
        .map(|(number, parts)| held_parts_warning(&constraints.keepout[number], number, parts))
        .collect())
}

pub(crate) fn outline_corner(pcb: &Expr) -> Result<[f64; 2], String> {
    Ok(outline::board_loops(pcb)?
        .outline
        .iter()
        .fold([f64::INFINITY, f64::INFINITY], |corner, point| {
            [corner[0].min(point[0]), corner[1].min(point[1])]
        }))
}

/// Replaces the board's constraint keepouts by the given ones, and returns
/// a warning per keepout that meets a part the placer keeps where the
/// input has it (`held_parts_in_keepouts`). Such a keepout's rule area
/// allows footprints: KiCad would report the part the input put there on
/// purpose, and the placer keeps movable parts out of it by its name
/// (`KEEPOUT_NAME`) whatever it allows.
pub(crate) fn apply_keepouts(
    pcb: &mut Expr,
    keepouts: &[KiCadKeepoutConstraint],
    held: &HeldParts,
) -> Result<Vec<String>, String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| {
        !(item.head() == Some("zone")
            && form_atom(item, "name", 1).is_some_and(|name| name.starts_with(KEEPOUT_NAME)))
    });
    if keepouts.is_empty() {
        return Ok(Vec::new());
    }
    let corner = outline_corner(pcb)?;
    let inside = held_parts_in_keepouts(pcb, keepouts, held)?;
    let mut warnings = Vec::new();
    let mut zones = Vec::new();
    for (number, keepout) in keepouts.iter().enumerate() {
        let [x0, y0, x1, y1] = keepout_box(keepout, number, corner)?;
        let layers = match (keepout.side.as_deref(), keepout.copper) {
            (Some("front"), _) => "\"F.Cu\"",
            (Some("back"), _) => "\"B.Cu\"",
            (None | Some("both"), false) => "\"F.Cu\" \"B.Cu\"",
            (None | Some("both"), true) => "\"*.Cu\"",
            (Some(other), _) => {
                return Err(format!(
                    "unknown keepout side {other:?} (front, back or both)"
                ));
            }
        };
        let copper = if keepout.copper {
            "not_allowed"
        } else {
            "allowed"
        };
        let footprints = if inside[number].is_empty() {
            "not_allowed"
        } else {
            warnings.push(held_parts_warning(keepout, number, &inside[number]));
            "allowed"
        };
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in format!("{KEEPOUT_NAME} {number} {x0} {y0} {x1} {y1}").bytes() {
            hash = (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3);
        }
        zones.push(parse(&format!(
            "(zone (layers {layers}) (uuid \"{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}\") (name \"{KEEPOUT_NAME} {}\") \
             (hatch edge 0.5) (connect_pads (clearance 0)) (min_thickness 0.25) \
             (keepout (tracks {copper}) (vias {copper}) (pads allowed) (copperpour {copper}) (footprints {footprints})) \
             (fill (thermal_gap 0.5) (thermal_bridge_width 0.5)) \
             (polygon (pts (xy {x0} {y0}) (xy {x1} {y0}) (xy {x1} {y1}) (xy {x0} {y1}))))",
            hash >> 32,
            (hash >> 16) & 0xffff,
            hash & 0xfff,
            (hash >> 12) & 0xfff,
            hash & 0xffff_ffff_ffff,
            number + 1,
        ))?);
    }
    let Expr::List(items) = pcb else {
        unreachable!();
    };
    items.extend(zones);
    Ok(warnings)
}

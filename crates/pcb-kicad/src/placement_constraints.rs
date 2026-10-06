// Copyright (C) 2026 Toit contributors.

//! `constraints.json`: placement intent from the user (often an agent).
//!
//! ```json
//! {"version": 1,
//!  "fixed": ["J1", "H*"],
//!  "rotation": [{"part": "U1", "angle": 90}],
//!  "edge": [{"part": "J4", "edge": "left", "flush": true}],
//!  "region": [{"parts": ["U5", "C1?"], "x": [0, 20], "y": [0, 15], "origin": "board"}],
//!  "keepout": [{"x": [30, 40], "y": [0, 10], "side": "front", "copper": true}],
//!  "back": ["BT1", "C2?"],
//!  "hollow": ["SHIELD1"],
//!  "near": [{"part": "C1", "pin_of": "U1:48", "max_mm": 3},
//!           {"part": "U2", "part_of": "J4", "max_mm": 10}],
//!  "relative": [{"part": "J4", "below": "U3", "max_gap_mm": 3}],
//!  "group": [{"parts": ["U3", "L1", "C5?"], "max_mm": 4}],
//!  "row": [{"parts": ["D1", "D2", "D3", "D4"], "pitch_mm": 5, "axis": "x"}],
//!  "apart": [{"parts": ["U5"], "from": ["U1", "Q*"], "min_mm": 10}],
//!  "device_front": "left",
//!  "place": [{"part": "SW1", "at": "front"}, {"part": "J1", "x": 10, "y": 5, "angle": 90}]}
//! ```
//!
//! Parts named by a constraint may move even where a default rule would
//! fix them, unless `fixed` names them. Edge and region constraints are hard
//! (legality); near and relative constraints are soft (a penalty); every
//! constraint is reported back with whether the placement keeps it.

use super::*;
use pcb_placer as core;
use crate::board_placer::glob_matches;
use pcb_placer::constraints::{Anchor, Edge, Relation};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPlacementConstraints {
    pub version: u32,
    /// A rectangular board outline replacing the board's own (or giving a
    /// board without one its shape).
    #[serde(default)]
    pub outline: Option<KiCadOutlineConstraint>,
    /// References or glob patterns (`*`, `?`) that keep their pose.
    #[serde(default)]
    pub fixed: Vec<String>,
    /// Every footprint moves unless `fixed` names it or it is locked: for
    /// boards fresh from a netlist, where the default rules (parts on the
    /// edge stay) would read intent into a stack of parts. Mounting holes
    /// no constraint names go to the corners.
    #[serde(default)]
    pub move_all: bool,
    #[serde(default)]
    pub rotation: Vec<KiCadRotationConstraint>,
    #[serde(default)]
    pub edge: Vec<KiCadEdgeConstraint>,
    #[serde(default)]
    pub region: Vec<KiCadRegionConstraint>,
    #[serde(default)]
    pub keepout: Vec<KiCadKeepoutConstraint>,
    /// References or glob patterns of parts that go on the bottom side
    /// (flipped as KiCad flips them), and of parts that go on the top.
    #[serde(default)]
    pub back: Vec<String>,
    #[serde(default)]
    pub front: Vec<String>,
    /// References or glob patterns of parts whose pads alone block other
    /// parts: other parts may sit inside their courtyard (a shield's
    /// outline around its headers, a module mounted above parts).
    #[serde(default)]
    pub hollow: Vec<String>,
    #[serde(default)]
    pub near: Vec<KiCadNearConstraint>,
    #[serde(default)]
    pub relative: Vec<KiCadRelativeConstraint>,
    #[serde(default)]
    pub group: Vec<KiCadGroupConstraint>,
    #[serde(default)]
    pub row: Vec<KiCadRowConstraint>,
    #[serde(default)]
    pub apart: Vec<KiCadApartConstraint>,
    /// The board edge the device's front is at (`left`, `right`, `top`,
    /// `bottom`): `place` then understands `front` and `rear`.
    #[serde(default)]
    pub device_front: Option<String>,
    #[serde(default)]
    pub place: Vec<KiCadPlaceConstraint>,
}

/// Parts kept away from others: every body of `parts` at least `min_mm`
/// from every body of `from` (a temperature sensor away from the regulator,
/// an audio input away from the switcher).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadApartConstraint {
    /// References or glob patterns.
    pub parts: Vec<String>,
    pub from: Vec<String>,
    pub min_mm: f64,
}

/// Where one part goes, in words (`at`) or exactly (`x`, `y`, `angle`).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPlaceConstraint {
    pub part: String,
    /// `left`, `right`, `top`, `bottom` (on that edge), `top-left`,
    /// `top-right`, `bottom-left`, `bottom-right` (in that corner),
    /// `center` (the middle third both ways), `left-half`, `right-half`,
    /// `top-half`, `bottom-half`, or `front`/`rear` (the edge
    /// `device_front` names, and the opposite one).
    #[serde(default)]
    pub at: Option<String>,
    /// The footprint's origin, exactly: the part is put there and kept.
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    /// KiCad orientation in degrees (default: the part's current one).
    #[serde(default)]
    pub angle: Option<f64>,
    /// For `x`/`y`, as for regions: `board` (default) or `absolute`.
    #[serde(default)]
    pub origin: Option<String>,
}

/// Parts in a line at a fixed pitch, turned alike (LED bars, key rows, test
/// points). The row is placed as one part, led by its first part: other
/// constraints name the first part only.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRowConstraint {
    /// References or glob patterns, in row order (a glob's parts in natural
    /// order: D2 before D10).
    pub parts: Vec<String>,
    /// Distance between neighbouring parts' origins.
    pub pitch_mm: f64,
    /// `x` (default): along the board's x axis; `y`: along its y axis. The
    /// row may run either way; a rotation constraint on its first part
    /// fixes that.
    #[serde(default)]
    pub axis: Option<String>,
    /// Parts per line: with it, the parts fill a grid line by line (a
    /// keyboard's switches), the lines `row_pitch_mm` apart (default
    /// `pitch_mm`) across the axis.
    #[serde(default)]
    pub columns: Option<usize>,
    #[serde(default)]
    pub row_pitch_mm: Option<f64>,
}

/// Parts kept together: each one's body within `max_mm` of the central
/// part's body (a regulator with its inductor and capacitors).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadGroupConstraint {
    /// References or glob patterns.
    pub parts: Vec<String>,
    /// The part the others gather around (default: the largest).
    #[serde(default)]
    pub around: Option<String>,
    pub max_mm: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadOutlineConstraint {
    /// Size in millimetres. Leave both out and the board is sized from its
    /// parts: `area_factor` times their total body area, at `aspect`.
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    /// Board area per unit of part area when sizing automatically.
    /// Default: 2 on two layers, 1.5 on more, the median of 616 open-source
    /// boards (1.9 and 1.4); `layout-kicad-board` grows it while the parts
    /// do not fit or the first route leaves connections open.
    #[serde(default)]
    pub area_factor: Option<f64>,
    /// Width over height when sizing automatically (default 1.5).
    #[serde(default)]
    pub aspect: Option<f64>,
    /// When sizing automatically, `layout-kicad-board` goes on shrinking the
    /// board by a fifth while a quick route still completes: the smallest
    /// board that routes.
    #[serde(default)]
    pub shrink: bool,
    /// Top-left corner in board coordinates. Default: the old outline's
    /// top-left corner, or, without one, centred on the footprints.
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRotationConstraint {
    pub part: String,
    /// KiCad orientation in degrees.
    #[serde(default)]
    pub angle: Option<f64>,
    /// Allowed orientations, instead of a single `angle`.
    #[serde(default)]
    pub angles: Vec<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadEdgeConstraint {
    pub part: String,
    /// `left`, `right`, `top` or `bottom` of the board outline's bounding
    /// box, or `any`: the edge nearest to where the part settles without
    /// this constraint.
    pub edge: String,
    /// The body touches the edge (keeping the edge margin).
    #[serde(default)]
    pub flush: bool,
    /// Largest distance from the edge margin line to the body (default 1 mm,
    /// 0 when `flush`).
    #[serde(default)]
    pub max_mm: Option<f64>,
    /// Not supported yet: reported as a warning.
    #[serde(default)]
    pub overhang: Option<serde_json::Value>,
    /// Not supported yet: reported as a warning.
    #[serde(default)]
    pub opening_outwards: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRegionConstraint {
    /// References or glob patterns.
    pub parts: Vec<String>,
    pub x: [f64; 2],
    pub y: [f64; 2],
    /// `board` (default): relative to the top-left corner of the outline's
    /// bounding box; `absolute`: KiCad board coordinates.
    #[serde(default)]
    pub origin: Option<String>,
}

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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadNearConstraint {
    pub part: String,
    /// Another part: the gap between the two bodies is at most `max_mm`.
    #[serde(default)]
    pub part_of: Option<String>,
    /// A pad, `REF:PAD`: the gap between the body and the pad centre.
    #[serde(default)]
    pub pin_of: Option<String>,
    pub max_mm: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRelativeConstraint {
    pub part: String,
    #[serde(default)]
    pub below: Option<String>,
    #[serde(default)]
    pub above: Option<String>,
    #[serde(default)]
    pub left_of: Option<String>,
    #[serde(default)]
    pub right_of: Option<String>,
    /// Largest gap between the two bodies (default 5 mm).
    #[serde(default)]
    pub max_gap_mm: Option<f64>,
}

/// Where the constraints come from in a placer config: a file (relative to
/// the source directory) or the constraints themselves.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum KiCadConstraintsSource {
    File(PathBuf),
    Inline(KiCadPlacementConstraints),
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadConstraintStatus {
    pub kind: String,
    pub part: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    pub satisfied: bool,
    pub violation_mm: f64,
}

pub fn read_placement_constraints(path: &Path) -> Result<KiCadPlacementConstraints, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let constraints: KiCadPlacementConstraints =
        serde_json::from_str(&text).map_err(|error| format!("invalid constraints {}: {error}", path.display()))?;
    if constraints.version != 1 {
        return Err(format!("{}: unsupported constraints version {}", path.display(), constraints.version));
    }
    Ok(constraints)
}

/// Replaces a file reference by the file's contents.
pub(super) fn resolve_constraints(
    config: &KiCadBoardPlacerConfig,
    source_directory: &Path,
) -> Result<KiCadBoardPlacerConfig, String> {
    let mut resolved = config.clone();
    if let Some(KiCadConstraintsSource::File(path)) = &config.constraints {
        resolved.constraints = Some(KiCadConstraintsSource::Inline(read_placement_constraints(
            &source_directory.join(path),
        )?));
    }
    Ok(resolved)
}

/// Board area per unit of part area for an outline sized from the parts:
/// about what designers use (median 1.9 on two layers, 1.4 on more).
pub(super) fn default_area_factor(pcb: &Expr) -> f64 {
    let copper = pcb
        .child("layers")
        .map(|layers| {
            layers
                .children()
                .iter()
                .filter(|layer| layer.children().get(1).and_then(Expr::atom).is_some_and(|name| name.ends_with(".Cu")))
                .count()
        })
        .unwrap_or(2);
    if copper > 2 { 1.5 } else { 2.0 }
}

/// Replaces the board's Edge.Cuts graphics by the constraint's rectangle.
/// `part_area` is the parts' total body area, for automatic sizing.
/// Returns the size used.
pub(super) fn apply_outline(
    pcb: &mut Expr,
    outline: &KiCadOutlineConstraint,
    part_area: f64,
) -> Result<[f64; 2], String> {
    let (width, height) = match (outline.width, outline.height) {
        (Some(width), Some(height)) => (width, height),
        (None, None) => {
            let area = part_area * outline.area_factor.unwrap_or_else(|| default_area_factor(pcb));
            let aspect = outline.aspect.unwrap_or(1.5).max(0.1);
            let mut width = (area * aspect).sqrt();
            // Both sides within 100 mm keeps the cheap fabs' flat price
            // (docs/cost.md): give up the aspect before that.
            if width > 100.0 && area <= 100.0 * 100.0 {
                width = 100.0;
            }
            // Whole half millimetres, rounded up.
            ((width * 2.0).ceil() / 2.0, (area / width * 2.0).ceil() / 2.0)
        }
        _ => return Err("outline needs both width and height, or neither (automatic size)".into()),
    };
    if !(width > 0.0 && height > 0.0) {
        return Err("outline width and height must be positive".into());
    }
    let old = outline::board_loops(pcb).ok().map(|loops| {
        loops.outline.iter().fold([f64::INFINITY, f64::INFINITY], |corner, point| {
            [corner[0].min(point[0]), corner[1].min(point[1])]
        })
    });
    let corner = match (outline.x, outline.y, old) {
        (Some(x), Some(y), _) => [x, y],
        (x, y, Some(old)) if old[0].is_finite() => [x.unwrap_or(old[0]), y.unwrap_or(old[1])],
        (x, y, _) => {
            // Centred on the footprints.
            let origins: Vec<[f64; 3]> = pcb
                .children()
                .iter()
                .filter(|item| item.head() == Some("footprint"))
                .map(form_at)
                .collect::<Result<_, _>>()?;
            let count = origins.len().max(1) as f64;
            let center = origins
                .iter()
                .fold([0.0, 0.0], |sum, at| [sum[0] + at[0] / count, sum[1] + at[1] / count]);
            [
                x.unwrap_or(center[0] - width / 2.0),
                y.unwrap_or(center[1] - height / 2.0),
            ]
        }
    };
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| {
        !(matches!(
            item.head(),
            Some("gr_line" | "gr_rect" | "gr_arc" | "gr_circle" | "gr_poly" | "gr_curve")
        ) && form_atom(item, "layer", 1) == Some("Edge.Cuts"))
    });
    let rectangle = parse(&format!(
        "(gr_rect (start {} {}) (end {} {}) (stroke (width 0.05) (type solid)) (fill no) (layer \"Edge.Cuts\"))",
        corner[0],
        corner[1],
        corner[0] + width,
        corner[1] + height
    ))?;
    items.push(rectangle);
    Ok([width, height])
}

/// Names of the rule areas `apply_keepouts` writes. Parts inside one are
/// not there on purpose (unlike parts inside a designer's rule area).
pub(super) const KEEPOUT_NAME: &str = "constraint keepout";

/// Replaces the board's constraint keepouts by the given ones.
pub(super) fn apply_keepouts(pcb: &mut Expr, keepouts: &[KiCadKeepoutConstraint]) -> Result<(), String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| {
        !(item.head() == Some("zone") && form_atom(item, "name", 1).is_some_and(|name| name.starts_with(KEEPOUT_NAME)))
    });
    if keepouts.is_empty() {
        return Ok(());
    }
    let corner = outline::board_loops(pcb)?
        .outline
        .iter()
        .fold([f64::INFINITY, f64::INFINITY], |corner, point| {
            [corner[0].min(point[0]), corner[1].min(point[1])]
        });
    let mut zones = Vec::new();
    for (number, keepout) in keepouts.iter().enumerate() {
        let offset = match keepout.origin.as_deref() {
            None | Some("board") => corner,
            Some("absolute") => [0.0, 0.0],
            Some(other) => return Err(format!("unknown keepout origin {other:?} (board or absolute)")),
        };
        let (x0, x1) = (offset[0] + keepout.x[0].min(keepout.x[1]), offset[0] + keepout.x[0].max(keepout.x[1]));
        let (y0, y1) = (offset[1] + keepout.y[0].min(keepout.y[1]), offset[1] + keepout.y[0].max(keepout.y[1]));
        if !(x1 > x0 && y1 > y0) {
            return Err(format!("keepout {} is empty", number + 1));
        }
        let layers = match (keepout.side.as_deref(), keepout.copper) {
            (Some("front"), _) => "\"F.Cu\"",
            (Some("back"), _) => "\"B.Cu\"",
            (None | Some("both"), false) => "\"F.Cu\" \"B.Cu\"",
            (None | Some("both"), true) => "\"*.Cu\"",
            (Some(other), _) => return Err(format!("unknown keepout side {other:?} (front, back or both)")),
        };
        let copper = if keepout.copper { "not_allowed" } else { "allowed" };
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in format!("{KEEPOUT_NAME} {number} {x0} {y0} {x1} {y1}").bytes() {
            hash = (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3);
        }
        zones.push(parse(&format!(
            "(zone (layers {layers}) (uuid \"{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}\") (name \"{KEEPOUT_NAME} {}\") \
             (hatch edge 0.5) (connect_pads (clearance 0)) (min_thickness 0.25) \
             (keepout (tracks {copper}) (vias {copper}) (pads allowed) (copperpour {copper}) (footprints not_allowed)) \
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
    Ok(())
}

/// Flips the footprints `back` and `front` name onto that side.
pub(super) fn apply_sides(pcb: &mut Expr, constraints: &KiCadPlacementConstraints) -> Result<(), String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut used = vec![false; constraints.back.len() + constraints.front.len()];
    for item in items.iter_mut().filter(|item| item.head() == Some("footprint")) {
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
                    return Err(format!("{reference} is named for both the front and the back"));
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
        let pattern = constraints.back.iter().chain(&constraints.front).nth(index).unwrap();
        return Err(format!("side constraint {pattern:?} matches no part"));
    }
    Ok(())
}

fn edge(name: &str) -> Result<Edge, String> {
    match name {
        "left" => Ok(Edge::Left),
        "right" => Ok(Edge::Right),
        "top" => Ok(Edge::Top),
        "bottom" => Ok(Edge::Bottom),
        other => Err(format!("unknown edge {other:?} (left, right, top, bottom or any)")),
    }
}

/// Natural order of references: D2 before D10.
fn natural(reference: &str) -> (String, u64, String) {
    let digits = reference.find(|c: char| c.is_ascii_digit()).unwrap_or(reference.len());
    let end = reference[digits..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(reference.len(), |end| digits + end);
    (
        reference[..digits].to_string(),
        reference[digits..end].parse().unwrap_or(0),
        reference[end..].to_string(),
    )
}

/// Turns every row into one part: the first part carries the others' bodies,
/// pins and pads at their places in the row; the others become followers
/// that occupy nothing themselves. Returns each follower's leader.
fn apply_rows(
    problem: &mut core::Problem,
    references: &[String],
    footprints: usize,
    pad_boxes: &[Vec<[f64; 4]>],
    rows: &[KiCadRowConstraint],
) -> Result<BTreeMap<usize, usize>, String> {
    let mut leaders = BTreeMap::new();
    for row in rows {
        let mut members: Vec<usize> = Vec::new();
        for pattern in &row.parts {
            let mut parts: Vec<usize> = (0..footprints).filter(|index| glob_matches(pattern, &references[*index])).collect();
            if parts.is_empty() {
                return Err(format!("row part {pattern:?} matches no part"));
            }
            parts.sort_by_key(|index| natural(&references[*index]));
            for part in parts {
                if !members.contains(&part) {
                    members.push(part);
                }
            }
        }
        if members.len() < 2 {
            return Err(format!("row {:?} has fewer than two parts", row.parts));
        }
        if !(row.pitch_mm > 0.0) {
            return Err(format!("row {:?}: pitch_mm must be positive", row.parts));
        }
        let axis = match row.axis.as_deref() {
            None | Some("x") => [1.0, 0.0],
            Some("y") => [0.0, 1.0],
            Some(other) => return Err(format!("unknown row axis {other:?} (x or y)")),
        };
        // Lines of a grid follow each other across the axis.
        let across = [axis[1], axis[0]];
        let columns = row.columns.unwrap_or(members.len());
        let line_pitch = row.row_pitch_mm.unwrap_or(row.pitch_mm);
        if columns == 0 || !(line_pitch > 0.0) {
            return Err(format!("row {:?}: columns and row_pitch_mm must be positive", row.parts));
        }
        let leader = members[0];
        let side = problem.components[leader].side;
        let angle = problem.poses[leader].angle;
        let mut body = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut carried = problem.components[leader].clone();
        carried.pins.clear();
        carried.far_side.clear();
        carried.pads.clear();
        for (place, member) in members.iter().enumerate() {
            if leaders.contains_key(member) || leaders.values().any(|leader| leader == member) {
                return Err(format!("{} is in two rows", references[*member]));
            }
            let component = &problem.components[*member];
            if component.side != side {
                return Err(format!("row {:?}: all parts must be on one side", row.parts));
            }
            // The member's place in the leader's own frame.
            let along = (place % columns) as f64 * row.pitch_mm;
            let line = (place / columns) as f64 * line_pitch;
            let offset = core::problem::rotate(
                [axis[0] * along + across[0] * line, axis[1] * along + across[1] * line],
                angle,
            );
            let shift = |boxes: &[[f64; 4]]| -> Vec<[f64; 4]> {
                boxes
                    .iter()
                    .map(|b| [b[0] + offset[0], b[1] + offset[1], b[2] + offset[0], b[3] + offset[1]])
                    .collect()
            };
            let center = [component.body_center[0] + offset[0], component.body_center[1] + offset[1]];
            body = [
                body[0].min(center[0] - component.body_size[0] / 2.0),
                body[1].min(center[1] - component.body_size[1] / 2.0),
                body[2].max(center[0] + component.body_size[0] / 2.0),
                body[3].max(center[1] + component.body_size[1] / 2.0),
            ];
            carried.pins.extend(component.pins.iter().map(|pin| core::Pin {
                offset: [pin.offset[0] + offset[0], pin.offset[1] + offset[1]],
                net: pin.net,
            }));
            carried.far_side.extend(shift(&component.far_side));
            carried.pads.extend(shift(&pad_boxes[*member]));
            carried.halo = carried.halo.max(component.halo);
            carried.edge_inset = carried.edge_inset.min(component.edge_inset);
            if *member != leader {
                leaders.insert(*member, leader);
                problem.constraints.followers.push(core::constraints::Follower {
                    part: *member,
                    leader,
                    offset,
                    angle: 0.0,
                });
            }
        }
        carried.body_center = [(body[0] + body[2]) / 2.0, (body[1] + body[3]) / 2.0];
        carried.body_size = [body[2] - body[0], body[3] - body[1]];
        carried.round = false;
        carried.tight = None;
        carried.courtyards.clear();
        carried.holes_inside = false;
        // Turning the row by a half turn keeps it along its axis.
        carried.angle_options.retain(|option| {
            let turn = (option - angle).rem_euclid(360.0);
            turn.abs() < 1.0e-6 || (turn - 180.0).abs() < 1.0e-6
        });
        if carried.angle_options.is_empty() {
            carried.angle_options = vec![angle];
        }
        problem.components[leader] = carried;
        for member in &members[1..] {
            let component = &mut problem.components[*member];
            component.side = core::Side::Neither;
            component.fixed = true;
            component.pins.clear();
            component.far_side.clear();
            component.pads.clear();
            component.halo = 0.0;
            component.body_size = [0.01, 0.01];
        }
    }
    let mut poses = problem.poses.clone();
    problem.sync_followers(&mut poses);
    problem.poses = poses;
    Ok(leaders)
}

/// Applies `constraints` to a lowered problem. `pads[i]` maps pad names of
/// footprint `i` to their local offsets. Returns warnings for parts of the
/// constraints that are accepted but not acted on.
pub(super) fn apply_constraints(
    problem: &mut core::Problem,
    references: &[String],
    pads: &[BTreeMap<String, [f64; 2]>],
    pad_boxes: &[Vec<[f64; 4]>],
    keepouts: &[Option<[f64; 4]>],
    mouths: &[Option<[f64; 2]>],
    locked: &[bool],
    constraints: &KiCadPlacementConstraints,
    weight: f64,
) -> Result<Vec<String>, String> {
    let footprints = pads.len();
    let leaders = apply_rows(problem, references, footprints, pad_boxes, &constraints.row)?;
    let find = |reference: &str| -> Result<usize, String> {
        let index = references[..footprints]
            .iter()
            .position(|candidate| candidate == reference)
            .ok_or_else(|| format!("constraint names unknown part {reference:?}"))?;
        match leaders.get(&index) {
            Some(leader) => Err(format!(
                "{reference} is carried by its row: constrain the row's first part, {}",
                references[*leader]
            )),
            None => Ok(index),
        }
    };
    // Globs skip the parts rows carry (their first part stands for them).
    let matching = |pattern: &str| -> Vec<usize> {
        (0..footprints)
            .filter(|index| !leaders.contains_key(index) && glob_matches(pattern, &references[*index]))
            .collect()
    };
    let mut warnings = Vec::new();
    let mut constrained = BTreeSet::new();
    let bounds = problem.bounds();

    for rotation in &constraints.rotation {
        let index = find(&rotation.part)?;
        let mut angles = rotation.angles.clone();
        angles.extend(rotation.angle);
        if angles.is_empty() {
            return Err(format!("rotation constraint for {} names no angle", rotation.part));
        }
        let angles: Vec<f64> = angles.iter().map(|angle| angle.rem_euclid(360.0)).collect();
        let current = problem.poses[index].angle;
        // The part turns about its body centre into the first allowed angle.
        if !angles.iter().any(|angle| (angle - current).abs() < 1.0e-6) {
            let component = &problem.components[index];
            let center = component.center(problem.poses[index]);
            problem.poses[index] = core::Pose {
                position: component.position_for_center(center, angles[0]),
                angle: angles[0],
            };
        }
        problem.components[index].angle_options = angles;
        constrained.insert(index);
    }
    // Parts put somewhere in words become edge and region constraints;
    // parts put at a pose stay there.
    let mut pinned = Vec::new();
    let opposite = |name: &str| match name {
        "left" => Ok("right"),
        "right" => Ok("left"),
        "top" => Ok("bottom"),
        "bottom" => Ok("top"),
        other => Err(format!("unknown device_front {other:?} (left, right, top or bottom)")),
    };
    for entry in &constraints.place {
        let index = find(&entry.part)?;
        match (&entry.at, entry.x, entry.y) {
            (Some(at), None, None) => {
                let word = match at.as_str() {
                    "front" | "rear" => {
                        let front = constraints
                            .device_front
                            .as_deref()
                            .ok_or_else(|| format!("place {}: {at:?} needs device_front", entry.part))?;
                        opposite(front)?;
                        if at == "front" { front.to_string() } else { opposite(front)?.to_string() }
                    }
                    other => other.to_string(),
                };
                let (width, height) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
                let region = |x0: f64, y0: f64, x1: f64, y1: f64| {
                    [bounds[0] + x0 * width, bounds[1] + y0 * height, bounds[0] + x1 * width, bounds[1] + y1 * height]
                };
                let (edges, area): (Vec<Edge>, Option<[f64; 4]>) = match word.as_str() {
                    "left" | "right" | "top" | "bottom" => (vec![edge(&word)?], None),
                    "top-left" => (vec![Edge::Top, Edge::Left], None),
                    "top-right" => (vec![Edge::Top, Edge::Right], None),
                    "bottom-left" => (vec![Edge::Bottom, Edge::Left], None),
                    "bottom-right" => (vec![Edge::Bottom, Edge::Right], None),
                    "center" => (Vec::new(), Some(region(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0))),
                    "left-half" => (Vec::new(), Some(region(0.0, 0.0, 0.5, 1.0))),
                    "right-half" => (Vec::new(), Some(region(0.5, 0.0, 1.0, 1.0))),
                    "top-half" => (Vec::new(), Some(region(0.0, 0.0, 1.0, 0.5))),
                    "bottom-half" => (Vec::new(), Some(region(0.0, 0.5, 1.0, 1.0))),
                    other => {
                        return Err(format!(
                            "place {}: unknown position {other:?} (an edge, a corner like top-left, center, a half like left-half, front or rear)",
                            entry.part
                        ))
                    }
                };
                for side in edges {
                    problem.constraints.edges.push((index, side, 1.0 + 1.0e-3));
                }
                if let Some(area) = area {
                    problem.constraints.regions.push((index, area));
                }
                constrained.insert(index);
            }
            (None, Some(x), Some(y)) => {
                let offset = match entry.origin.as_deref() {
                    None | Some("board") => [bounds[0], bounds[1]],
                    Some("absolute") => [0.0, 0.0],
                    Some(other) => return Err(format!("unknown place origin {other:?} (board or absolute)")),
                };
                let angle = entry.angle.unwrap_or(problem.poses[index].angle).rem_euclid(360.0);
                problem.poses[index] = core::Pose {
                    position: [offset[0] + x, offset[1] + y],
                    angle,
                };
                pinned.push(index);
            }
            _ => {
                return Err(format!(
                    "place {}: give either `at` or both `x` and `y`",
                    entry.part
                ))
            }
        }
    }
    for entry in &constraints.edge {
        let index = find(&entry.part)?;
        let reach = entry.max_mm.unwrap_or(if entry.flush { 0.0 } else { 1.0 });
        problem.constraints.edges.push((index, edge(&entry.edge)?, reach.max(0.0) + 1.0e-3));
        if let Some(overhang) = &entry.overhang {
            let side = match overhang.get("edge").and_then(|edge| edge.as_str()) {
                Some(name) => edge(name)?,
                None => edge(&entry.edge)?,
            };
            let local = keepouts[index].ok_or_else(|| {
                format!(
                    "edge constraint for {}: overhang needs a keepout zone in the footprint (the part that sticks out)",
                    entry.part
                )
            })?;
            // Only orientations that point the keepout at that edge.
            let component = &problem.components[index];
            let pointing = |angle: f64| {
                let box_center = component.offset([(local[0] + local[2]) / 2.0, (local[1] + local[3]) / 2.0], angle);
                let body_center = component.offset(component.body_center, angle);
                let direction = [box_center[0] - body_center[0], box_center[1] - body_center[1]];
                match side {
                    Edge::Left => direction[0] < -direction[1].abs(),
                    Edge::Right => direction[0] > direction[1].abs(),
                    Edge::Top => direction[1] < -direction[0].abs(),
                    Edge::Bottom => direction[1] > direction[0].abs(),
                }
            };
            let angles: Vec<f64> = component.angle_options.iter().copied().filter(|angle| pointing(*angle)).collect();
            if angles.is_empty() {
                return Err(format!(
                    "edge constraint for {}: no allowed rotation points its keepout at the {} edge",
                    entry.part, side.name()
                ));
            }
            if !angles.iter().any(|angle| (angle - problem.poses[index].angle).abs() < 1.0e-6) {
                let center = component.center(problem.poses[index]);
                problem.poses[index] = core::Pose {
                    position: component.position_for_center(center, angles[0]),
                    angle: angles[0],
                };
            }
            problem.components[index].angle_options = angles;
            problem.constraints.overhangs.push((index, side, local));
        }
        if entry.opening_outwards == Some(true) {
            let side = edge(&entry.edge)?;
            let mouth = mouths[index].ok_or_else(|| {
                format!(
                    "edge constraint for {}: cannot tell which way it opens (no side of its courtyard reaches clearly beyond its pads); give a rotation instead",
                    entry.part
                )
            })?;
            let component = &problem.components[index];
            let facing = |angle: f64| {
                let direction = component.offset(mouth, angle);
                match side {
                    Edge::Left => direction[0] < -0.5,
                    Edge::Right => direction[0] > 0.5,
                    Edge::Top => direction[1] < -0.5,
                    Edge::Bottom => direction[1] > 0.5,
                }
            };
            let angles: Vec<f64> = component.angle_options.iter().copied().filter(|angle| facing(*angle)).collect();
            if angles.is_empty() {
                return Err(format!(
                    "edge constraint for {}: no allowed rotation opens it towards the {} edge",
                    entry.part,
                    side.name()
                ));
            }
            if !angles.iter().any(|angle| (angle - problem.poses[index].angle).abs() < 1.0e-6) {
                let center = component.center(problem.poses[index]);
                problem.poses[index] = core::Pose {
                    position: component.position_for_center(center, angles[0]),
                    angle: angles[0],
                };
            }
            problem.components[index].angle_options = angles;
        }
        constrained.insert(index);
    }
    for entry in &constraints.region {
        let offset = match entry.origin.as_deref() {
            None | Some("board") => [bounds[0], bounds[1]],
            Some("absolute") => [0.0, 0.0],
            Some(other) => return Err(format!("unknown region origin {other:?} (board or absolute)")),
        };
        let region = [
            offset[0] + entry.x[0].min(entry.x[1]),
            offset[1] + entry.y[0].min(entry.y[1]),
            offset[0] + entry.x[0].max(entry.x[1]),
            offset[1] + entry.y[0].max(entry.y[1]),
        ];
        let mut any = false;
        for pattern in &entry.parts {
            for index in matching(pattern) {
                problem.constraints.regions.push((index, region));
                constrained.insert(index);
                any = true;
            }
        }
        if !any {
            return Err(format!("region constraint {:?} matches no part", entry.parts));
        }
    }
    for entry in &constraints.near {
        let part = find(&entry.part)?;
        let anchor = match (&entry.part_of, &entry.pin_of) {
            (Some(other), None) => Anchor::Body(find(other)?),
            (None, Some(pin)) => {
                let (reference, pad) = pin
                    .split_once(':')
                    .ok_or_else(|| format!("pin_of {pin:?} is not REF:PAD"))?;
                let index = find(reference)?;
                let offset = pads[index]
                    .get(pad)
                    .ok_or_else(|| format!("pin_of {pin:?}: {reference} has no pad {pad:?}"))?;
                Anchor::Point(index, *offset)
            }
            _ => return Err(format!("near constraint for {} needs exactly one of part_of and pin_of", entry.part)),
        };
        problem.constraints.relations.push(Relation::Near {
            part,
            anchor,
            max: entry.max_mm.max(0.0),
        });
        constrained.insert(part);
    }
    for entry in &constraints.group {
        let mut members = Vec::new();
        for pattern in &entry.parts {
            let parts = matching(pattern);
            if parts.is_empty() {
                return Err(format!("group constraint {pattern:?} matches no part"));
            }
            for part in parts {
                if !members.contains(&part) {
                    members.push(part);
                }
            }
        }
        let center = match &entry.around {
            Some(reference) => find(reference)?,
            None => *members
                .iter()
                .max_by(|a, b| {
                    let area = |index: usize| problem.components[index].body_size[0] * problem.components[index].body_size[1];
                    area(**a).total_cmp(&area(**b))
                })
                .expect("a group has members"),
        };
        for part in members.into_iter().filter(|part| *part != center) {
            problem.constraints.relations.push(Relation::Near {
                part,
                anchor: Anchor::Body(center),
                max: entry.max_mm.max(0.0),
            });
            constrained.insert(part);
        }
        constrained.insert(center);
    }
    for entry in &constraints.apart {
        let expand = |patterns: &[String]| -> Result<Vec<usize>, String> {
            let mut parts = Vec::new();
            for pattern in patterns {
                let matched = matching(pattern);
                if matched.is_empty() {
                    return Err(format!("apart constraint {pattern:?} matches no part"));
                }
                for part in matched {
                    if !parts.contains(&part) {
                        parts.push(part);
                    }
                }
            }
            Ok(parts)
        };
        let (parts, others) = (expand(&entry.parts)?, expand(&entry.from)?);
        if !(entry.min_mm > 0.0) {
            return Err(format!("apart constraint {:?}: min_mm must be positive", entry.parts));
        }
        for part in &parts {
            for other in others.iter().filter(|other| *other != part) {
                problem.constraints.relations.push(Relation::Apart {
                    part: *part,
                    anchor: *other,
                    min: entry.min_mm,
                });
            }
            constrained.insert(*part);
        }
    }
    for entry in &constraints.relative {
        let part = find(&entry.part)?;
        let sides = [
            (&entry.below, Edge::Bottom),
            (&entry.above, Edge::Top),
            (&entry.left_of, Edge::Left),
            (&entry.right_of, Edge::Right),
        ];
        let named: Vec<_> = sides.iter().filter(|(other, _)| other.is_some()).collect();
        if named.len() != 1 {
            return Err(format!(
                "relative constraint for {} needs exactly one of below, above, left_of, right_of",
                entry.part
            ));
        }
        let (other, side) = named[0];
        problem.constraints.relations.push(Relation::Beside {
            part,
            anchor: find(other.as_deref().expect("named"))?,
            side: *side,
            max_gap: entry.max_gap_mm.unwrap_or(5.0).max(0.0),
        });
        constrained.insert(part);
    }
    problem.constraints.relation_weight = weight;

    // Constrained parts move (the user said where they go), fixed ones stay.
    if constraints.move_all {
        // Parts without nets move too: from a netlist they sit in the stack
        // with the rest. Mounting holes nobody placed go to the corners, as
        // designers put them, then to the middle of the edges.
        let named: BTreeSet<usize> = constraints.fixed.iter().flat_map(|pattern| matching(pattern)).collect();
        let mut spots = [
            vec![Edge::Top, Edge::Left],
            vec![Edge::Top, Edge::Right],
            vec![Edge::Bottom, Edge::Left],
            vec![Edge::Bottom, Edge::Right],
            vec![Edge::Top],
            vec![Edge::Bottom],
            vec![Edge::Left],
            vec![Edge::Right],
        ]
        .into_iter();
        for index in 0..footprints {
            // Artwork (a logo) occupies nothing and stays.
            let component = &problem.components[index];
            if locked[index] || leaders.contains_key(&index) || component.side == core::Side::Neither {
                continue;
            }
            // A part without nets and without a reference (a keyboard's
            // case holes, drawn by hand) is something no constraint can
            // name, so nobody can say where it goes: it stays. A netlist
            // gives every part a reference.
            if references[index].is_empty() && component.pins.is_empty() {
                continue;
            }
            let mounting_hole = component.pins.is_empty() && component.has_holes();
            if mounting_hole && !constrained.contains(&index) && !named.contains(&index) && !pinned.contains(&index) {
                for side in spots.next().unwrap_or_default() {
                    problem.constraints.edges.push((index, side, 1.0 + 1.0e-3));
                }
            }
            problem.components[index].fixed = false;
        }
    }
    for pattern in &constraints.hollow {
        let parts = matching(pattern);
        if parts.is_empty() {
            return Err(format!("hollow constraint {pattern:?} matches no part"));
        }
        for index in parts {
            if pad_boxes[index].is_empty() {
                return Err(format!("hollow part {} has no pads", references[index]));
            }
            problem.components[index].hollow = pad_boxes[index].clone();
        }
    }
    // A part sent to a side is placed there.
    for pattern in constraints.back.iter().chain(&constraints.front) {
        constrained.extend(matching(pattern));
    }
    for index in constrained {
        problem.components[index].fixed = false;
    }
    // With an outline from the constraints, parts entirely off the board
    // are unplaced, not deliberately hanging over the edge.
    if constraints.outline.is_some() {
        for index in 0..footprints {
            let component = &problem.components[index];
            let pose = problem.poses[index];
            let (center, half) = (component.center(pose), component.half_extent(pose.angle));
            let off = center[0] + half[0] <= bounds[0]
                || center[0] - half[0] >= bounds[2]
                || center[1] + half[1] <= bounds[1]
                || center[1] - half[1] >= bounds[3];
            if off && !component.pins.is_empty() {
                problem.components[index].fixed = false;
            }
        }
    }
    for index in pinned {
        problem.components[index].fixed = true;
    }
    for pattern in &constraints.fixed {
        let matched = matching(pattern);
        if matched.is_empty() {
            return Err(format!("fixed pattern {pattern:?} matches no part"));
        }
        for index in matched {
            problem.components[index].fixed = true;
        }
    }
    Ok(warnings)
}

pub(super) fn constraint_report(
    problem: &core::Problem,
    poses: &[core::Pose],
    references: &[String],
) -> Vec<KiCadConstraintStatus> {
    core::constraints::report(problem, poses)
        .into_iter()
        .map(|status| KiCadConstraintStatus {
            kind: status.kind.into(),
            part: references[status.part].clone(),
            other: status.other.filter(|other| *other != status.part).map(|other| references[other].clone()),
            satisfied: status.satisfied,
            violation_mm: (status.violation * 1000.0).round() / 1000.0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem() -> (core::Problem, Vec<String>, Vec<BTreeMap<String, [f64; 2]>>) {
        let part = |fixed: bool| core::Component {
            name: String::new(),
            body_center: [0.0, 0.0],
            body_size: [2.0, 1.0],
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side: core::Side::Front,
            fixed,
            angle_options: vec![0.0, 90.0, 180.0, 270.0],
            far_side: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: Vec::new(),
            copper_only: false,
        };
        let problem = core::Problem {
            outline: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
            components: vec![part(true), part(false), part(false)],
            net_weights: Vec::new(),
            poses: vec![core::Pose { position: [10.0, 10.0], angle: 0.0 }; 3],
            spacing: 0.2,
            grid: 0.1,
            edge_margin: 0.5,
            min_spacing: 0.0,
            far_side_pads_only: false,
            constraints: Default::default(),
        };
        let references = vec!["J1".to_string(), "C1".to_string(), "C2".to_string()];
        let pads = vec![
            BTreeMap::from([("1".to_string(), [0.5, 0.0])]),
            BTreeMap::new(),
            BTreeMap::new(),
        ];
        (problem, references, pads)
    }

    fn apply(json: &str) -> Result<(core::Problem, Vec<String>), String> {
        let (mut problem, references, pads) = problem();
        let constraints: KiCadPlacementConstraints = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let warnings =
            apply_constraints(
                &mut problem,
                &references,
                &pads,
                &[vec![[0.0, -0.5, 1.0, 0.5]], Vec::new(), Vec::new()],
                &[None, None, None],
                &[None, None, None],
                &[false, false, false],
                &constraints,
                10.0,
            )?;
        Ok((problem, warnings))
    }

    #[test]
    fn move_all_sends_mounting_holes_nobody_placed_to_the_corners() {
        let run = |json: &str| {
            let (mut problem, references, pads) = problem();
            // J1 is a mounting hole: a hole, no nets.
            problem.components[0].far_side = vec![[-1.0, -1.0, 1.0, 1.0]];
            let constraints: KiCadPlacementConstraints = serde_json::from_str(json).unwrap();
            apply_constraints(
                &mut problem,
                &references,
                &pads,
                &[Vec::new(), Vec::new(), Vec::new()],
                &[None, None, None],
                &[None, None, None],
                &[false, false, false],
                &constraints,
                10.0,
            )
            .unwrap();
            problem
        };
        let problem = run(r#"{"version": 1, "move_all": true}"#);
        assert!(problem.components.iter().all(|component| !component.fixed));
        assert_eq!(problem.constraints.edges, vec![(0, Edge::Top, 1.0 + 1.0e-3), (0, Edge::Left, 1.0 + 1.0e-3)]);
        // Named anywhere, it goes where it is told.
        let problem = run(r#"{"version": 1, "move_all": true, "fixed": ["J1"]}"#);
        assert!(problem.components[0].fixed && problem.constraints.edges.is_empty());
        let problem = run(r#"{"version": 1, "move_all": true, "place": [{"part": "J1", "at": "bottom"}]}"#);
        assert_eq!(problem.constraints.edges, vec![(0, Edge::Bottom, 1.0 + 1.0e-3)]);
    }

    #[test]
    fn constraints_resolve_parts_pins_and_globs() {
        let (problem, warnings) = apply(
            r#"{"version": 1,
                "edge": [{"part": "J1", "edge": "left", "flush": true}],
                "region": [{"parts": ["C*"], "x": [0, 10], "y": [0, 5]}],
                "near": [{"part": "C1", "pin_of": "J1:1", "max_mm": 2}],
                "relative": [{"part": "C2", "below": "C1"}],
                "rotation": [{"part": "C2", "angles": [90, 270]}]}"#,
        )
        .unwrap();
        assert!(warnings.is_empty());
        // J1 was fixed by default; an edge constraint frees it.
        assert!(!problem.components[0].fixed);
        assert_eq!(problem.constraints.edges.len(), 1);
        assert_eq!(problem.constraints.regions.len(), 2);
        assert_eq!(
            problem.constraints.relations[0],
            Relation::Near { part: 1, anchor: Anchor::Point(0, [0.5, 0.0]), max: 2.0 }
        );
        assert!(matches!(
            problem.constraints.relations[1],
            Relation::Beside { part: 2, anchor: 1, side: Edge::Bottom, .. }
        ));
        assert_eq!(problem.components[2].angle_options, vec![90.0, 270.0]);
        assert_eq!(problem.poses[2].angle, 90.0);
    }

    #[test]
    fn an_outline_without_size_is_sized_from_the_parts() {
        let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)) (gr_line (start 0 0) (end 1 0) (layer \"Edge.Cuts\")))").unwrap();
        let outline: KiCadOutlineConstraint = serde_json::from_str(r#"{"aspect": 2.0}"#).unwrap();
        // 200 mm2 of parts at the default factor 2 (two layers): 400 mm2,
        // 28.5 x 14.5 (whole half millimetres, rounded up).
        let size = apply_outline(&mut pcb, &outline, 200.0).unwrap();
        assert_eq!(size, [28.5, 14.5]);
        let edges: Vec<_> = pcb
            .children()
            .iter()
            .filter(|item| form_atom(item, "layer", 1) == Some("Edge.Cuts"))
            .collect();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].head(), Some("gr_rect"));
        let only_width: KiCadOutlineConstraint = serde_json::from_str(r#"{"width": 10}"#).unwrap();
        assert!(apply_outline(&mut pcb, &only_width, 1.0).is_err());
        // 4500 mm2 of parts, 9000 mm2 of board: 116 x 78 at 1.5, but both
        // sides stay within 100 mm (the flat price at JLCPCB and PCBWay).
        let automatic: KiCadOutlineConstraint = serde_json::from_str("{}").unwrap();
        assert_eq!(apply_outline(&mut pcb, &automatic, 4500.0).unwrap(), [100.0, 90.0]);
        // On four layers, 1.5 times the parts.
        let mut four = parse("(kicad_pcb (layers (0 \"F.Cu\" signal) (4 \"In1.Cu\" signal) (6 \"In2.Cu\" signal) (2 \"B.Cu\" signal)))").unwrap();
        assert_eq!(apply_outline(&mut four, &outline, 300.0).unwrap(), [30.0, 15.0]);
    }

    #[test]
    fn fixed_wins_over_a_constraint_and_mistakes_are_errors() {
        let (problem, _) = apply(r#"{"version": 1, "fixed": ["C1"], "near": [{"part": "C1", "part_of": "C2", "max_mm": 1}]}"#).unwrap();
        assert!(problem.components[1].fixed);
        let (problem, _) = apply(r#"{"version": 1, "group": [{"parts": ["C*", "J1"], "around": "J1", "max_mm": 2}]}"#).unwrap();
        assert_eq!(problem.constraints.relations.len(), 2);
        assert!(problem.constraints.relations.iter().all(|relation| matches!(
            relation,
            Relation::Near { anchor: Anchor::Body(0), max, .. } if *max == 2.0
        )));
        let (problem, _) = apply(r#"{"version": 1, "hollow": ["J1"]}"#).unwrap();
        assert_eq!(problem.components[0].hollow, vec![[0.0, -0.5, 1.0, 0.5]]);
        for bad in [
            r#"{"version": 1, "near": [{"part": "R9", "part_of": "C2", "max_mm": 1}]}"#,
            r#"{"version": 1, "near": [{"part": "C1", "pin_of": "J1:7", "max_mm": 1}]}"#,
            r#"{"version": 1, "near": [{"part": "C1", "max_mm": 1}]}"#,
            r#"{"version": 1, "edge": [{"part": "C1", "edge": "north"}]}"#,
            r#"{"version": 1, "relative": [{"part": "C1", "below": "C2", "above": "J1"}]}"#,
            r#"{"version": 1, "fixed": ["Q*"]}"#,
            r#"{"version": 1, "edge": [{"part": "C1", "edge": "top", "overhang": {"edge": "top"}}]}"#,
            r#"{"version": 1, "colour": "green"}"#,
            r#"{"version": 1, "hollow": ["U*"]}"#,
            r#"{"version": 1, "group": [{"parts": ["C1", "U9"], "max_mm": 2}]}"#,
            r#"{"version": 1, "hollow": ["C1"]}"#,
        ] {
            assert!(apply(bad).is_err(), "{bad} was accepted");
        }
    }

    #[test]
    fn apart_keeps_every_named_part_from_every_other() {
        let (problem, _) = apply(r#"{"version": 1, "apart": [{"parts": ["C1"], "from": ["C*", "J1"], "min_mm": 5}]}"#).unwrap();
        assert_eq!(
            problem.constraints.relations,
            vec![Relation::Apart { part: 1, anchor: 2, min: 5.0 }, Relation::Apart { part: 1, anchor: 0, min: 5.0 }]
        );
        for bad in [
            r#"{"version": 1, "apart": [{"parts": ["C1"], "from": ["U*"], "min_mm": 5}]}"#,
            r#"{"version": 1, "apart": [{"parts": ["C1"], "from": ["C2"], "min_mm": 0}]}"#,
        ] {
            assert!(apply(bad).is_err(), "{bad} was accepted");
        }
    }

    #[test]
    fn place_takes_words_and_exact_poses() {
        let (problem, _) = apply(
            r#"{"version": 1, "device_front": "left",
                "place": [{"part": "C1", "at": "front"}, {"part": "C2", "at": "bottom-right"},
                          {"part": "J1", "x": 5, "y": 6, "angle": 90}]}"#,
        )
        .unwrap();
        let edges: Vec<(usize, Edge)> = problem.constraints.edges.iter().map(|(part, edge, _)| (*part, *edge)).collect();
        assert_eq!(edges, vec![(1, Edge::Left), (2, Edge::Bottom), (2, Edge::Right)]);
        assert!(!problem.components[1].fixed && !problem.components[2].fixed);
        assert_eq!(problem.poses[0], core::Pose { position: [5.0, 6.0], angle: 90.0 });
        assert!(problem.components[0].fixed);
        let (problem, _) = apply(r#"{"version": 1, "place": [{"part": "C1", "at": "center"}, {"part": "C2", "at": "top-half"}]}"#).unwrap();
        let expected = [(1, [10.0, 20.0 / 3.0, 20.0, 40.0 / 3.0]), (2, [0.0, 0.0, 30.0, 10.0])];
        assert_eq!(problem.constraints.regions.len(), 2);
        for ((part, region), (want_part, want)) in problem.constraints.regions.iter().zip(expected) {
            assert_eq!(*part, want_part);
            assert!(region.iter().zip(want).all(|(a, b)| (a - b).abs() < 1.0e-9), "{region:?}");
        }
        for bad in [
            r#"{"version": 1, "place": [{"part": "C1", "at": "front"}]}"#,
            r#"{"version": 1, "place": [{"part": "C1", "at": "somewhere"}]}"#,
            r#"{"version": 1, "place": [{"part": "C1", "at": "left", "x": 1, "y": 2}]}"#,
            r#"{"version": 1, "place": [{"part": "C1", "x": 1}]}"#,
            r#"{"version": 1, "device_front": "north", "place": [{"part": "C1", "at": "front"}]}"#,
        ] {
            assert!(apply(bad).is_err(), "{bad} was accepted");
        }
    }

    #[test]
    fn a_row_is_placed_as_its_first_part() {
        let (problem, _) = apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}]}"#).unwrap();
        // C1 carries C2 three millimetres to its right: one 5 x 1 mm body.
        assert_eq!(problem.components[1].body_size, [5.0, 1.0]);
        assert_eq!(problem.components[1].body_center, [1.5, 0.0]);
        assert!(problem.components[2].fixed && problem.components[2].side == core::Side::Neither);
        assert_eq!(
            problem.constraints.followers,
            vec![core::constraints::Follower { part: 2, leader: 1, offset: [3.0, 0.0], angle: 0.0 }]
        );
        assert_eq!(problem.poses[2].position, [13.0, 10.0]);
        // Only half turns keep the row along its axis.
        assert_eq!(problem.components[1].angle_options, vec![0.0, 180.0]);
        // Two lines of one: C2 below C1.
        let (problem, _) =
            apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3, "columns": 1, "row_pitch_mm": 4}]}"#).unwrap();
        assert_eq!(problem.constraints.followers[0].offset, [0.0, 4.0]);
        assert_eq!(problem.components[1].body_size, [2.0, 5.0]);
        for bad in [
            r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}], "near": [{"part": "C2", "part_of": "J1", "max_mm": 1}]}"#,
            r#"{"version": 1, "row": [{"parts": ["C1"], "pitch_mm": 3}]}"#,
            r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 0}]}"#,
            r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3, "axis": "z"}]}"#,
            r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}, {"parts": ["C2", "J1"], "pitch_mm": 3}]}"#,
        ] {
            assert!(apply(bad).is_err(), "{bad} was accepted");
        }
    }

    #[test]
    fn side_constraints_flip_parts_onto_their_side() {
        let footprint = |reference: &str, layer: &str| {
            format!(
                "(footprint \"R\" (layer \"{layer}\") (at 5 5) (property \"Reference\" \"{reference}\" (at 0 -1 0) (layer \"F.SilkS\")) \
                 (pad \"1\" smd rect (at 1 1) (size 1 1) (layers \"{layer}\")))"
            )
        };
        let board = format!("(kicad_pcb {} {} {})", footprint("R1", "F.Cu"), footprint("R2", "F.Cu"), footprint("C1", "B.Cu"));
        let sides = |json: &str| -> Result<Vec<String>, String> {
            let mut pcb = parse(&board).unwrap();
            apply_sides(&mut pcb, &serde_json::from_str(json).unwrap())?;
            Ok(pcb
                .children()
                .iter()
                .filter(|item| item.head() == Some("footprint"))
                .map(|item| form_atom(item, "layer", 1).unwrap().to_string())
                .collect())
        };
        assert_eq!(sides(r#"{"version": 1, "back": ["R*"], "front": ["C1"]}"#).unwrap(), ["B.Cu", "B.Cu", "F.Cu"]);
        // A part already on its side stays as it is.
        assert_eq!(sides(r#"{"version": 1, "back": ["C1", "R2"]}"#).unwrap(), ["F.Cu", "B.Cu", "B.Cu"]);
        assert!(sides(r#"{"version": 1, "back": ["R*"], "front": ["R1"]}"#).is_err());
        assert!(sides(r#"{"version": 1, "back": ["U1"]}"#).is_err());
    }

    #[test]
    fn keepouts_become_rule_areas_and_replace_earlier_ones() {
        let mut pcb = parse(
            "(kicad_pcb (gr_rect (start 10 20) (end 60 60) (stroke (width 0.05) (type solid)) (fill no) (layer \"Edge.Cuts\")))",
        )
        .unwrap();
        let keepouts = |json: &str| -> Vec<KiCadKeepoutConstraint> {
            serde_json::from_str::<KiCadPlacementConstraints>(json).unwrap().keepout
        };
        let zones = |pcb: &Expr| -> Vec<Expr> {
            pcb.children().iter().filter(|item| item.head() == Some("zone")).cloned().collect()
        };
        apply_keepouts(
            &mut pcb,
            &keepouts(r#"{"version": 1, "keepout": [{"x": [0, 5], "y": [2, 4], "copper": true}, {"x": [1, 2], "y": [1, 2], "side": "back"}]}"#),
        )
        .unwrap();
        let written = zones(&pcb);
        assert_eq!(written.len(), 2);
        // Relative to the outline's corner; copper keepouts block tracks on every layer.
        let text = format!("{}", encode(&written[0]));
        assert!(text.contains("(xy 10 22)") && text.contains("(xy 15 24)"), "{text}");
        assert!(text.contains("\"*.Cu\"") && text.contains("(tracks not_allowed)"), "{text}");
        let text = format!("{}", encode(&written[1]));
        assert!(text.contains("\"B.Cu\"") && text.contains("(tracks allowed)") && text.contains("(footprints not_allowed)"));
        // Applying again replaces them; nothing removes them all.
        apply_keepouts(&mut pcb, &keepouts(r#"{"version": 1, "keepout": [{"x": [0, 5], "y": [0, 5]}]}"#)).unwrap();
        assert_eq!(zones(&pcb).len(), 1);
        apply_keepouts(&mut pcb, &[]).unwrap();
        assert!(zones(&pcb).is_empty());
        for bad in [
            r#"{"version": 1, "keepout": [{"x": [0, 0], "y": [0, 5]}]}"#,
            r#"{"version": 1, "keepout": [{"x": [0, 1], "y": [0, 5], "side": "top"}]}"#,
            r#"{"version": 1, "keepout": [{"x": [0, 1], "y": [0, 5], "origin": "center"}]}"#,
        ] {
            assert!(apply_keepouts(&mut pcb, &keepouts(bad)).is_err(), "{bad} was accepted");
        }
    }
}

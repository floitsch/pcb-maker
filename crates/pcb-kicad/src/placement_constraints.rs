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
use crate::board_placer::glob_matches;
use pcb_placer as core;
use pcb_placer::constraints::{Anchor, Edge, Relation};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPlacementConstraints {
    pub version: u32,
    /// Notes for the reader, not read by the placer: a text or a list of
    /// texts (the import's `constraints.starter.json` says there what it
    /// inferred).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<serde_json::Value>,
    /// A board outline replacing the board's own (or giving a board
    /// without one its shape): a rectangle, a polygon, round corners,
    /// cutouts, several pieces.
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
    /// Parts the schematic does not have, added to the board by
    /// `import-kicad-netlist` (mounting holes plated on a net, a
    /// footprint for another switch): the layout keeps those with `x` and
    /// `y` there, as `place` does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add: Vec<KiCadAddConstraint>,
}

/// A part the board has and the schematic does not: a library footprint
/// (`footprint`) or the footprint of that reference on the board given
/// with `import-kicad-netlist --footprints-from` (`from_board`). Written to
/// the board `board_only`, so that KiCad's schematic parity check leaves it
/// out.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadAddConstraint {
    /// The new part's reference (no part of the schematic may have it).
    pub part: String,
    /// `Library:Name`, from the project's libraries as the schematic's
    /// footprints are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<String>,
    /// The footprint `part` (or `board_reference`) of the
    /// `--footprints-from` board, the one at `x`, `y` (`origin`
    /// `absolute`) where that reference repeats there; without `x` and `y`
    /// it stays where that board has it, on its side, locked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_board: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_reference: Option<String>,
    /// Pad number to net name. Without it a `from_board` part keeps the
    /// board's nets and a library footprint has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nets: Option<BTreeMap<String, String>>,
    /// Where it goes, as `place` says it (`origin` `board`, the default, or
    /// `absolute`); without `x` and `y` the placer places it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// `front` (default for a library footprint) or `back`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
}

/// The layout's side of `add`: every added part must be on the board (the
/// import puts it there), and those given `x` and `y` are kept there, as
/// `place` keeps a part. Empties `add`, so that a second call changes
/// nothing.
pub(super) fn expand_additions(
    constraints: &mut KiCadPlacementConstraints,
    pcb: &Expr,
) -> Result<(), String> {
    let additions = std::mem::take(&mut constraints.add);
    for addition in additions {
        let on_board = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .any(|footprint| {
                footprint_reference(footprint).as_deref() == Some(addition.part.as_str())
            });
        if !on_board {
            return Err(format!(
                "add {}: the board has no such part; import-kicad-netlist adds the parts of add: import again into the same directory",
                addition.part
            ));
        }
        if let (Some(x), Some(y)) = (addition.x, addition.y) {
            constraints.place.push(KiCadPlaceConstraint {
                part: addition.part,
                at: None,
                x: Some(x),
                y: Some(y),
                angle: addition.angle,
                origin: addition.origin,
                uuid: None,
            });
        }
    }
    Ok(())
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
    /// Which of several footprints sharing `part`'s reference (a panel's
    /// copies) this entry is about: the footprint's `uuid` (`tstamp` in
    /// older files). Without it, the n-th entry of this kind for a
    /// repeated reference is about its n-th footprint in board order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
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
    /// top-left corner, or, without one, centred on the footprints. For a
    /// polygon, the top-left corner of its points' bounding box; for
    /// pieces, of all pieces' (a piece's own `x`/`y` are its corner's
    /// offset within the set, default 0).
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    /// A polygon instead of a rectangle: its corners in millimetres,
    /// closed implicitly, moved so that their bounding box's top-left
    /// corner lies at `x`, `y`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polygon: Option<Vec<[f64; 2]>>,
    /// Every corner of the rectangle or polygon rounded with this radius
    /// (less where a side is too short for it), as lines and arcs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    /// Holes in the board (Edge.Cuts inner contours): copper keeps the
    /// edge clearance from them and no part's body enters them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cutouts: Vec<KiCadCutout>,
    /// Several pieces instead of one shape (a board with a breakaway part,
    /// boards side by side): each a rectangle or polygon with its own
    /// `x`/`y` offset, radius and cutouts. They may not overlap.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pieces: Vec<KiCadOutlineConstraint>,
}

/// A hole cut into an outline: a box (`x`, `y` ranges) or a polygon,
/// relative to its outline's (or piece's) top-left corner unless `origin`
/// is `absolute`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadCutout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polygon: Option<Vec<[f64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    /// `board` (default): relative to the outline's top-left corner;
    /// `absolute`: KiCad board coordinates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl KiCadOutlineConstraint {
    /// Sized from the parts: no size, no polygon, no pieces.
    pub fn automatic(&self) -> bool {
        self.width.is_none()
            && self.height.is_none()
            && self.polygon.is_none()
            && self.pieces.is_empty()
    }

    /// A polygon, or pieces one of which is: edge constraints then hold a
    /// part at the nearest side of the polygon facing their edge.
    pub fn polygonal(&self) -> bool {
        self.polygon.is_some() || self.pieces.iter().any(|piece| piece.polygon.is_some())
    }
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
    /// Which of several footprints sharing `part`'s reference (a panel's
    /// copies) this entry is about: the footprint's `uuid` (`tstamp` in
    /// older files). Without it, the n-th entry of this kind for a
    /// repeated reference is about its n-th footprint in board order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadEdgeConstraint {
    pub part: String,
    /// `left`, `right`, `top` or `bottom` of the board outline's bounding
    /// box (on a polygon outline: the nearest side of the polygon facing
    /// that way), or `any`: the edge nearest to where the part settles
    /// without this constraint.
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
    /// Which of several footprints sharing `part`'s reference (a panel's
    /// copies) this entry is about: the footprint's `uuid` (`tstamp` in
    /// older files). Without it, the n-th entry of this kind for a
    /// repeated reference is about its n-th footprint in board order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRegionConstraint {
    /// References or glob patterns.
    pub parts: Vec<String>,
    /// The box, in millimetres; or `at` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<[f64; 2]>,
    /// A part of the outline's box in words, whatever its size: `left-half`,
    /// `right-half`, `top-half`, `bottom-half` or `center` (the middle third
    /// both ways), as `place` says them (a keyboard half's keys on an
    /// outline still to be sized).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
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
#[allow(
    clippy::large_enum_variant,
    reason = "one per placer config; a box would only complicate the many matches on Inline"
)]
pub enum KiCadConstraintsSource {
    File(PathBuf),
    Inline(KiCadPlacementConstraints),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadConstraintStatus {
    pub kind: String,
    pub part: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    pub satisfied: bool,
    pub violation_mm: f64,
}

pub fn read_placement_constraints(path: &Path) -> Result<KiCadPlacementConstraints, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let constraints: KiCadPlacementConstraints = serde_json::from_str(&text)
        .map_err(|error| format!("invalid constraints {}: {error}", path.display()))?;
    if constraints.version != 1 {
        return Err(format!(
            "{}: unsupported constraints version {}",
            path.display(),
            constraints.version
        ));
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
                .filter(|layer| {
                    layer
                        .children()
                        .get(1)
                        .and_then(Expr::atom)
                        .is_some_and(|name| name.ends_with(".Cu"))
                })
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
    if outline.polygon.is_some()
        || !outline.pieces.is_empty()
        || outline.corner_radius.is_some_and(|radius| radius != 0.0)
        || !outline.cutouts.is_empty()
    {
        return apply_shaped_outline(pcb, outline, part_area);
    }
    let (width, height) = match (outline.width, outline.height) {
        (Some(width), Some(height)) => (width, height),
        (None, None) => {
            let area = part_area
                * outline
                    .area_factor
                    .unwrap_or_else(|| default_area_factor(pcb));
            let aspect = outline.aspect.unwrap_or(1.5).max(0.1);
            let mut width = (area * aspect).sqrt();
            // Both sides within 100 mm keeps the cheap fabs' flat price
            // (docs/cost.md): give up the aspect before that.
            if width > 100.0 && area <= 100.0 * 100.0 {
                width = 100.0;
            }
            // Whole half millimetres, rounded up.
            (
                (width * 2.0).ceil() / 2.0,
                (area / width * 2.0).ceil() / 2.0,
            )
        }
        _ => return Err("outline needs both width and height, or neither (automatic size)".into()),
    };
    if !(width > 0.0 && height > 0.0) {
        return Err("outline width and height must be positive".into());
    }
    let old = outline::board_loops(pcb).ok().map(|loops| {
        loops
            .outline
            .iter()
            .fold([f64::INFINITY, f64::INFINITY], |corner, point| {
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
            let center = origins.iter().fold([0.0, 0.0], |sum, at| {
                [sum[0] + at[0] / count, sum[1] + at[1] / count]
            });
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

/// The automatic size of an outline sized from the parts: `area_factor`
/// times their area at `aspect`, both sides within 100 mm when the area
/// allows, in whole half millimetres.
fn automatic_size(pcb: &Expr, outline: &KiCadOutlineConstraint, part_area: f64) -> (f64, f64) {
    let area = part_area
        * outline
            .area_factor
            .unwrap_or_else(|| default_area_factor(pcb));
    let aspect = outline.aspect.unwrap_or(1.5).max(0.1);
    let mut width = (area * aspect).sqrt();
    if width > 100.0 && area <= 100.0 * 100.0 {
        width = 100.0;
    }
    (
        (width * 2.0).ceil() / 2.0,
        (area / width * 2.0).ceil() / 2.0,
    )
}

/// A closed contour as written: its corners and the radius rounding
/// each of them.
#[derive(Clone, Debug, PartialEq)]
struct Contour {
    corners: Vec<[f64; 2]>,
    radius: f64,
}

/// One corner of a contour, rounded: the arc's start, middle and end (all
/// three the corner itself when it stays sharp).
fn fillet(
    previous: [f64; 2],
    corner: [f64; 2],
    next: [f64; 2],
    radius: f64,
) -> Option<[[f64; 2]; 3]> {
    let (la, lb) = (
        distance_squared(previous, corner).sqrt(),
        distance_squared(next, corner).sqrt(),
    );
    if radius <= 0.0 || la < 1.0e-9 || lb < 1.0e-9 {
        return None;
    }
    let u = [
        (previous[0] - corner[0]) / la,
        (previous[1] - corner[1]) / la,
    ];
    let v = [(next[0] - corner[0]) / lb, (next[1] - corner[1]) / lb];
    let angle = (u[0] * v[0] + u[1] * v[1]).clamp(-1.0, 1.0).acos();
    // A straight run or a spike stays as it is.
    if angle > std::f64::consts::PI - 1.0e-6 || angle < 1.0e-6 {
        return None;
    }
    let half = angle / 2.0;
    // Each side gives at most half its length to each of its corners.
    let tangent = (radius / half.tan()).min(la / 2.0).min(lb / 2.0);
    let radius = tangent * half.tan();
    if radius < 1.0e-6 {
        return None;
    }
    let start = [corner[0] + u[0] * tangent, corner[1] + u[1] * tangent];
    let end = [corner[0] + v[0] * tangent, corner[1] + v[1] * tangent];
    let w = [u[0] + v[0], u[1] + v[1]];
    let length = w[0].hypot(w[1]);
    let w = [w[0] / length, w[1] / length];
    let center_distance = radius / half.sin();
    let center = [
        corner[0] + w[0] * center_distance,
        corner[1] + w[1] * center_distance,
    ];
    let mid = [center[0] - w[0] * radius, center[1] - w[1] * radius];
    Some([start, mid, end])
}

/// A piece of a contour: `Ok` a line from point to point, `Err` an arc
/// (start, mid, end).
type ContourPiece = Result<[[f64; 2]; 2], [[f64; 2]; 3]>;

/// The contour's pieces in order: `Ok` lines from point to point, `Err`
/// arcs (start, mid, end).
fn contour_pieces(contour: &Contour) -> Vec<ContourPiece> {
    let corners = &contour.corners;
    let count = corners.len();
    let rounded: Vec<Option<[[f64; 2]; 3]>> = (0..count)
        .map(|index| {
            fillet(
                corners[(index + count - 1) % count],
                corners[index],
                corners[(index + 1) % count],
                contour.radius,
            )
        })
        .collect();
    let mut pieces = Vec::new();
    for index in 0..count {
        let end_here = rounded[index].map_or(corners[index], |arc| arc[2]);
        if let Some(arc) = rounded[index] {
            pieces.push(Err(arc));
        }
        let next = (index + 1) % count;
        let start_next = rounded[next].map_or(corners[next], |arc| arc[0]);
        if distance_squared(end_here, start_next).sqrt() > 1.0e-6 {
            pieces.push(Ok([end_here, start_next]));
        }
    }
    pieces
}

/// The contour as a polygon, arcs flattened as the outline reader does.
fn contour_points(contour: &Contour) -> Vec<[f64; 2]> {
    let mut points: Vec<[f64; 2]> = Vec::new();
    for piece in contour_pieces(contour) {
        let run = match piece {
            Ok(line) => line.to_vec(),
            Err(arc) => outline::arc_points(arc[0], arc[1], arc[2]),
        };
        for point in run {
            if points
                .last()
                .is_none_or(|last| distance_squared(*last, point) > 1.0e-18)
            {
                points.push(point);
            }
        }
    }
    if points.len() > 1 && distance_squared(points[0], *points.last().unwrap()) < 1.0e-18 {
        points.pop();
    }
    points
}

/// A coordinate as written: rounded to a nanometre, KiCad's unit.
fn coordinate(value: f64) -> String {
    let rounded = (value * 1.0e6).round() / 1.0e6;
    format!("{}", if rounded == 0.0 { 0.0 } else { rounded })
}

/// The Edge.Cuts graphics of a contour: lines and arcs.
fn contour_items(contour: &Contour) -> Result<Vec<Expr>, String> {
    let xy = |point: [f64; 2]| format!("{} {}", coordinate(point[0]), coordinate(point[1]));
    contour_pieces(contour)
        .into_iter()
        .map(|piece| {
            parse(&match piece {
                Ok([start, end]) => format!(
                    "(gr_line (start {}) (end {}) (stroke (width 0.05) (type solid)) (layer \"Edge.Cuts\"))",
                    xy(start),
                    xy(end)
                ),
                Err([start, mid, end]) => format!(
                    "(gr_arc (start {}) (mid {}) (end {}) (stroke (width 0.05) (type solid)) (layer \"Edge.Cuts\"))",
                    xy(start),
                    xy(mid),
                    xy(end)
                ),
            })
        })
        .collect()
}

fn signed_area(points: &[[f64; 2]]) -> f64 {
    (0..points.len())
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0
}

/// Whether segments ab and cd cross or touch.
fn segments_meet(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let cross = |o: [f64; 2], p: [f64; 2], q: [f64; 2]| {
        (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0])
    };
    let on = |o: [f64; 2], p: [f64; 2], q: [f64; 2]| {
        q[0] >= o[0].min(p[0]) - 1.0e-9
            && q[0] <= o[0].max(p[0]) + 1.0e-9
            && q[1] >= o[1].min(p[1]) - 1.0e-9
            && q[1] <= o[1].max(p[1]) + 1.0e-9
    };
    let (d1, d2, d3, d4) = (
        cross(c, d, a),
        cross(c, d, b),
        cross(a, b, c),
        cross(a, b, d),
    );
    let eps = 1.0e-12;
    if ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps))
        && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
    {
        return true;
    }
    (d1.abs() <= eps && on(c, d, a))
        || (d2.abs() <= eps && on(c, d, b))
        || (d3.abs() <= eps && on(a, b, c))
        || (d4.abs() <= eps && on(a, b, d))
}

/// A polygon's corners checked: finite, three at least (a closing point
/// repeating the first is dropped), an area, and no side crossing another.
fn checked_polygon(points: &[[f64; 2]], what: &str) -> Result<Vec<[f64; 2]>, String> {
    let mut points = points.to_vec();
    if points.len() > 1 && distance_squared(points[0], *points.last().unwrap()) < 1.0e-18 {
        points.pop();
    }
    if points
        .iter()
        .any(|point| !point[0].is_finite() || !point[1].is_finite())
    {
        return Err(format!("{what}: coordinates must be numbers"));
    }
    points.dedup_by(|a, b| distance_squared(*a, *b) < 1.0e-18);
    if points.len() < 3 {
        return Err(format!("{what} needs three corners at least"));
    }
    if signed_area(&points).abs() < 1.0e-9 {
        return Err(format!("{what} has no area"));
    }
    let count = points.len();
    for i in 0..count {
        for j in i + 1..count {
            // Neighbouring sides share a corner.
            if j == i + 1 || (i == 0 && j == count - 1) {
                continue;
            }
            if segments_meet(
                points[i],
                points[(i + 1) % count],
                points[j],
                points[(j + 1) % count],
            ) {
                return Err(format!(
                    "{what} crosses itself: the side from {:?} to {:?} meets the side from {:?} to {:?}",
                    points[i],
                    points[(i + 1) % count],
                    points[j],
                    points[(j + 1) % count]
                ));
            }
        }
    }
    Ok(points)
}

fn radius_of(radius: Option<f64>, what: &str) -> Result<f64, String> {
    let radius = radius.unwrap_or(0.0);
    if !(radius >= 0.0 && radius.is_finite()) {
        return Err(format!("{what}: corner_radius must be zero or more"));
    }
    Ok(radius)
}

fn bounding_box(points: &[[f64; 2]]) -> [f64; 4] {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    )
}

/// Whether two closed polygons share any point (a side meets a side, or
/// one lies inside the other).
fn polygons_meet(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    let (ba, bb) = (bounding_box(a), bounding_box(b));
    if ba[2] < bb[0] || bb[2] < ba[0] || ba[3] < bb[1] || bb[3] < ba[1] {
        return false;
    }
    for i in 0..a.len() {
        for j in 0..b.len() {
            if segments_meet(a[i], a[(i + 1) % a.len()], b[j], b[(j + 1) % b.len()]) {
                return true;
            }
        }
    }
    point_in_polygon(a[0], b) || point_in_polygon(b[0], a)
}

/// One piece of an outline as written: its contour (in the set's own
/// frame until placed) and its cutouts' (board coordinates where
/// `origin` is absolute, else relative to the piece's corner).
struct ShapedPiece {
    contour: Contour,
    /// The corners' bounding box in the set's frame.
    bounds: [f64; 4],
    cutouts: Vec<(Contour, bool)>,
}

fn shaped_piece(
    piece: &KiCadOutlineConstraint,
    what: &str,
    nested: bool,
    pcb: &Expr,
    part_area: f64,
) -> Result<ShapedPiece, String> {
    if nested && !piece.pieces.is_empty() {
        return Err(format!("{what}: a piece cannot have pieces"));
    }
    if (piece.shrink || piece.area_factor.is_some() || piece.aspect.is_some())
        && (nested || !piece.automatic())
    {
        return Err(format!(
            "{what}: area_factor, aspect and shrink are for an outline sized from the parts (no width, height or polygon)"
        ));
    }
    let radius = radius_of(piece.corner_radius, what)?;
    let corners = match (&piece.polygon, piece.width, piece.height) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            return Err(format!(
                "{what}: give a polygon or width and height, not both"
            ));
        }
        (Some(polygon), None, None) => checked_polygon(polygon, &format!("{what} polygon"))?,
        (None, Some(width), Some(height)) => {
            if !(width > 0.0 && height > 0.0) {
                return Err(format!("{what}: width and height must be positive"));
            }
            vec![[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]]
        }
        (None, None, None) if !nested => {
            if !piece.cutouts.is_empty() {
                return Err(format!(
                    "{what}: cutouts need a sized outline (width and height, or a polygon)"
                ));
            }
            let (width, height) = automatic_size(pcb, piece, part_area);
            vec![[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]]
        }
        (None, None, None) => return Err(format!("{what} needs width and height or a polygon")),
        _ => return Err("outline needs both width and height, or neither (automatic size)".into()),
    };
    // The corners' box moves to the piece's own offset.
    let own = bounding_box(&corners);
    let offset = if nested {
        [piece.x.unwrap_or(0.0), piece.y.unwrap_or(0.0)]
    } else {
        [0.0, 0.0]
    };
    let corners: Vec<[f64; 2]> = corners
        .iter()
        .map(|p| [p[0] - own[0] + offset[0], p[1] - own[1] + offset[1]])
        .collect();
    let mut cutouts = Vec::new();
    for (number, cutout) in piece.cutouts.iter().enumerate() {
        let name = format!("{what} cutout {}", number + 1);
        let radius = radius_of(cutout.corner_radius, &name)?;
        let absolute = match cutout.origin.as_deref() {
            None | Some("board") => false,
            Some("absolute") => true,
            Some(other) => {
                return Err(format!(
                    "unknown cutout origin {other:?} (board or absolute)"
                ));
            }
        };
        let corners = match (&cutout.polygon, cutout.x, cutout.y) {
            (Some(polygon), None, None) => checked_polygon(polygon, &name)?,
            (None, Some(x), Some(y)) => {
                let (x0, x1, y0, y1) = (
                    x[0].min(x[1]),
                    x[0].max(x[1]),
                    y[0].min(y[1]),
                    y[0].max(y[1]),
                );
                if !(x1 > x0 && y1 > y0) {
                    return Err(format!("{name} is empty"));
                }
                vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
            }
            _ => return Err(format!("{name}: give x and y ranges or a polygon")),
        };
        cutouts.push((Contour { corners, radius }, absolute));
    }
    Ok(ShapedPiece {
        bounds: bounding_box(&corners),
        contour: Contour { corners, radius },
        cutouts,
    })
}

/// Writes an outline that is not a plain rectangle (a polygon, rounded
/// corners, cutouts, several pieces) as Edge.Cuts lines and arcs,
/// replacing the board's own. Returns the size of its bounding box.
fn apply_shaped_outline(
    pcb: &mut Expr,
    outline: &KiCadOutlineConstraint,
    part_area: f64,
) -> Result<[f64; 2], String> {
    let mut pieces = Vec::new();
    if outline.pieces.is_empty() {
        pieces.push(shaped_piece(outline, "outline", false, pcb, part_area)?);
    } else {
        if outline.polygon.is_some()
            || outline.width.is_some()
            || outline.height.is_some()
            || outline.corner_radius.is_some()
            || !outline.cutouts.is_empty()
            || outline.shrink
            || outline.area_factor.is_some()
            || outline.aspect.is_some()
        {
            return Err("outline: with pieces, each piece carries its own shape (width and height or polygon, corner_radius, cutouts); the outline only x and y".into());
        }
        for (number, piece) in outline.pieces.iter().enumerate() {
            pieces.push(shaped_piece(
                piece,
                &format!("outline piece {}", number + 1),
                true,
                pcb,
                part_area,
            )?);
        }
    }
    let set = pieces.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, piece| {
            [
                b[0].min(piece.bounds[0]),
                b[1].min(piece.bounds[1]),
                b[2].max(piece.bounds[2]),
                b[3].max(piece.bounds[3]),
            ]
        },
    );
    let (width, height) = (set[2] - set[0], set[3] - set[1]);
    let old = outline::board_loops(pcb).ok().map(|loops| {
        loops
            .outline
            .iter()
            .fold([f64::INFINITY, f64::INFINITY], |corner, point| {
                [corner[0].min(point[0]), corner[1].min(point[1])]
            })
    });
    let corner = match (outline.x, outline.y, old) {
        (Some(x), Some(y), _) => [x, y],
        (x, y, Some(old)) if old[0].is_finite() => [x.unwrap_or(old[0]), y.unwrap_or(old[1])],
        (x, y, _) => {
            let origins: Vec<[f64; 3]> = pcb
                .children()
                .iter()
                .filter(|item| item.head() == Some("footprint"))
                .map(form_at)
                .collect::<Result<_, _>>()?;
            let count = origins.len().max(1) as f64;
            let center = origins.iter().fold([0.0, 0.0], |sum, at| {
                [sum[0] + at[0] / count, sum[1] + at[1] / count]
            });
            [
                x.unwrap_or(center[0] - width / 2.0),
                y.unwrap_or(center[1] - height / 2.0),
            ]
        }
    };
    let shift = [corner[0] - set[0], corner[1] - set[1]];
    let moved = |points: &[[f64; 2]], by: [f64; 2]| -> Vec<[f64; 2]> {
        points
            .iter()
            .map(|p| [p[0] + by[0], p[1] + by[1]])
            .collect()
    };
    let mut contours: Vec<Contour> = Vec::new();
    let mut flat_pieces: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut flat_cutouts: Vec<(Vec<[f64; 2]>, String)> = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        let name = if pieces.len() > 1 {
            format!("outline piece {}", index + 1)
        } else {
            "outline".to_string()
        };
        let contour = Contour {
            corners: moved(&piece.contour.corners, shift),
            radius: piece.contour.radius,
        };
        let flat = contour_points(&contour);
        for (other, earlier) in flat_pieces.iter().enumerate() {
            if polygons_meet(&flat, earlier) {
                return Err(format!(
                    "{name} meets outline piece {}: pieces may not overlap or touch",
                    other + 1
                ));
            }
        }
        contours.push(contour);
        // The piece's corner: where its cutouts' coordinates start.
        let piece_corner = [piece.bounds[0] + shift[0], piece.bounds[1] + shift[1]];
        for (number, (cutout, absolute)) in piece.cutouts.iter().enumerate() {
            let cutout_name = format!("{name} cutout {}", number + 1);
            let by = if *absolute { [0.0, 0.0] } else { piece_corner };
            let cutout = Contour {
                corners: moved(&cutout.corners, by),
                radius: cutout.radius,
            };
            let hole = contour_points(&cutout);
            let crosses = (0..hole.len()).any(|i| {
                (0..flat.len()).any(|j| {
                    segments_meet(
                        hole[i],
                        hole[(i + 1) % hole.len()],
                        flat[j],
                        flat[(j + 1) % flat.len()],
                    )
                })
            });
            if crosses || !point_in_polygon(hole[0], &flat) {
                return Err(format!(
                    "{cutout_name} does not lie inside its outline (coordinates are relative to the outline's top-left corner unless origin is absolute)"
                ));
            }
            for (earlier, earlier_name) in &flat_cutouts {
                if polygons_meet(&hole, earlier) {
                    return Err(format!(
                        "{cutout_name} meets {earlier_name}: cutouts may not overlap or touch"
                    ));
                }
            }
            flat_cutouts.push((hole, cutout_name));
            contours.push(cutout);
        }
        flat_pieces.push(flat);
    }
    let mut graphics = Vec::new();
    for contour in &contours {
        graphics.extend(contour_items(contour)?);
    }
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| {
        !(matches!(
            item.head(),
            Some("gr_line" | "gr_rect" | "gr_arc" | "gr_circle" | "gr_poly" | "gr_curve")
        ) && form_atom(item, "layer", 1) == Some("Edge.Cuts"))
    });
    items.extend(graphics);
    Ok([width, height])
}

/// Names of the rule areas `apply_keepouts` writes. Parts inside one are
/// not there on purpose (unlike parts inside a designer's rule area).
pub(super) const KEEPOUT_NAME: &str = "constraint keepout";

/// The parts the placer leaves where the input has them, one flag per
/// footprint in the board's order (`held_parts`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HeldParts(pub Vec<bool>);

impl HeldParts {
    /// Whether the board's footprint `index` (among its footprints) is held.
    pub(crate) fn holds(&self, index: usize) -> bool {
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
pub(crate) fn held_parts(pcb: &Expr, config: &KiCadBoardPlacerConfig) -> Result<HeldParts, String> {
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
pub(crate) fn held_parts_as_given(
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
        let (_, size, _) = crate::board_placer::local_body(footprint)?;
        part_area += (size[0] + config.spacing_mm) * (size[1] + config.spacing_mm);
    }
    apply_outline(&mut board, outline, part_area)?;
    held_parts(&board, config)
}

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

/// The box of a footprint's courtyard on the board, and its side (0 front,
/// 1 back); `None` without one.
fn courtyard_box(footprint: &Expr) -> Result<Option<([f64; 4], usize)>, String> {
    let at = form_at(footprint)?;
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut side = None;
    for child in footprint.children() {
        let Some(layer) = form_atom(child, "layer", 1).filter(|layer| layer.ends_with(".CrtYd"))
        else {
            continue;
        };
        let mut points = match child.head() {
            Some("fp_rect") => {
                let (start, end) = (form_xy(child, "start")?, form_xy(child, "end")?);
                vec![start, end, [start[0], end[1]], [end[0], start[1]]]
            }
            Some("fp_circle") => {
                let center = form_xy(child, "center")?;
                let end = form_xy(child, "end")?;
                let radius = distance_squared(center, end).sqrt();
                outline::circle_points(center, radius)
            }
            Some("fp_line" | "fp_arc") => outline::outline_points(child)?,
            Some("fp_poly") => outline::pts_points(child)?,
            _ => continue,
        };
        side = Some(usize::from(layer.starts_with("B.")));
        for point in points.drain(..) {
            let offset = rotate_vector(point, -at[2]);
            let board = [at[0] + offset[0], at[1] + offset[1]];
            bounds = [
                bounds[0].min(board[0]),
                bounds[1].min(board[1]),
                bounds[2].max(board[0]),
                bounds[3].max(board[1]),
            ];
        }
    }
    Ok(side.map(|side| (bounds, side)))
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

/// A pad's box on the board and its copper layers' names.
type PadBox = ([f64; 4], Vec<String>);

/// Pads of a footprint as boxes on the board: [x0, y0, x1, y1] and the
/// copper layers' names (`*.Cu` and `F&B.Cu` stand for both outer sides).
fn pad_boxes(footprint: &Expr) -> Result<Vec<PadBox>, String> {
    let at = form_at(footprint)?;
    let mut boxes = Vec::new();
    for pad in footprint
        .children()
        .iter()
        .filter(|child| child.head() == Some("pad"))
    {
        let (Ok(position), Ok(size)) = (form_xy(pad, "at"), form_xy(pad, "size")) else {
            continue;
        };
        // KiCad stores a pad's angle as the absolute one.
        let angle = pad
            .child("at")
            .and_then(|form| form.children().get(3))
            .and_then(Expr::atom)
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0)
            .to_radians();
        let offset = rotate_vector(position, -at[2]);
        let center = [at[0] + offset[0], at[1] + offset[1]];
        let (sin, cos) = angle.sin_cos();
        let half = [
            (size[0] * cos).abs() / 2.0 + (size[1] * sin).abs() / 2.0,
            (size[0] * sin).abs() / 2.0 + (size[1] * cos).abs() / 2.0,
        ];
        let layers: Vec<String> = pad
            .child("layers")
            .map(|layers| {
                layers
                    .children()
                    .iter()
                    .skip(1)
                    .filter_map(Expr::atom)
                    .filter(|name| name.ends_with(".Cu"))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if layers.is_empty() {
            continue;
        }
        boxes.push((
            [
                center[0] - half[0],
                center[1] - half[1],
                center[0] + half[0],
                center[1] + half[1],
            ],
            layers,
        ));
    }
    Ok(boxes)
}

/// A part's box on the board: its courtyard's, else its pads' (`None`
/// without either).
pub(crate) fn part_box(footprint: &Expr) -> Result<Option<[f64; 4]>, String> {
    if let Some((courtyard, _)) = courtyard_box(footprint)? {
        return Ok(Some(courtyard));
    }
    Ok(pad_boxes(footprint)?
        .into_iter()
        .map(|(bounds, _)| bounds)
        .reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        }))
}

/// Whether two pads' copper layer lists share a layer (`*.Cu` and
/// `F&B.Cu` reach both outer sides, which is what the parts' pads use).
fn pad_layers_meet(a: &[String], b: &[String]) -> bool {
    let wide = |name: &str| name == "*.Cu" || name == "F&B.Cu";
    a.iter()
        .any(|x| b.iter().any(|y| x == y || wide(x) || wide(y)))
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
            for (a, layers_a) in &first.pads {
                for (b, layers_b) in &second.pads {
                    let shrunk =
                        |r: &[f64; 4]| [r[0] + 0.01, r[1] + 0.01, r[2] - 0.01, r[3] - 0.01];
                    if box_overlap(shrunk(a), shrunk(b)) > 0.0
                        && pad_layers_meet(layers_a, layers_b)
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
    Ok(held_parts_overlapping(&pcb, &held)?
        .into_iter()
        .map(|(first, second, what)| overlap_warning(&first, &second, &what))
        .collect())
}

/// The warning for two held parts on top of each other.
pub(crate) fn overlap_warning(first: &str, second: &str, what: &str) -> String {
    format!(
        "fixed parts {first} and {second} overlap ({what}): the placer keeps both where the input has them and their pads cannot both be connected; give one its own place (place, or free it from fixed and fixed_patterns)"
    )
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
pub(super) fn apply_keepouts(
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

/// Flips the footprints `back` and `front` name onto that side.
pub(super) fn apply_sides(
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

fn edge(name: &str) -> Result<Edge, String> {
    match name {
        "left" => Ok(Edge::Left),
        "right" => Ok(Edge::Right),
        "top" => Ok(Edge::Top),
        "bottom" => Ok(Edge::Bottom),
        other => Err(format!(
            "unknown edge {other:?} (left, right, top, bottom or any)"
        )),
    }
}

/// Natural order of references: D2 before D10.
pub(super) fn natural(reference: &str) -> (String, u64, String) {
    let digits = reference
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(reference.len());
    let end = reference[digits..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(reference.len(), |end| digits + end);
    (
        reference[..digits].to_string(),
        reference[digits..end].parse().unwrap_or(0),
        reference[end..].to_string(),
    )
}

/// What `apply_rows` returns: each follower's leader, each row member's
/// place, and the rows' first parts.
type AppliedRows = (
    BTreeMap<usize, usize>,
    BTreeMap<usize, RowPlace>,
    Vec<usize>,
);

/// Turns every row into one part: the first part carries the others' bodies,
/// pins and pads at their places in the row; the others become followers
/// that occupy nothing themselves. Returns each follower's leader.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "!(x > 0.0) rejects NaN as well as zero and negative values"
)]
fn apply_rows(
    problem: &mut core::Problem,
    references: &[String],
    footprints: usize,
    pad_boxes: &[Vec<[f64; 4]>],
    rows: &[KiCadRowConstraint],
) -> Result<AppliedRows, String> {
    let mut leaders = BTreeMap::new();
    let mut places = BTreeMap::new();
    // The first parts of the rows.
    let mut user_rows = Vec::new();
    for row in rows {
        let mut members: Vec<usize> = Vec::new();
        for pattern in &row.parts {
            let mut parts: Vec<usize> = (0..footprints)
                .filter(|index| glob_matches(pattern, &references[*index]))
                .collect();
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
            return Err(format!(
                "row {:?}: columns and row_pitch_mm must be positive",
                row.parts
            ));
        }
        let leader = members[0];
        user_rows.push(leader);
        let side = problem.components[leader].side;
        // The row's frame: the leader's angle, or that angle turned by a
        // half turn when the members lie exactly
        // that way. A board placed before (layout mode lowers its own
        // placement again) holds a row the placer turned: built from the
        // leader's angle alone, it ran the other way, reported a region
        // the row kept as missed by the row's length and moved the members.
        // A stack fresh from a netlist matches every frame alike and keeps
        // the leader's.
        let place_of = |place: usize| {
            let along = (place % columns) as f64 * row.pitch_mm;
            let line = (place / columns) as f64 * line_pitch;
            [
                axis[0] * along + across[0] * line,
                axis[1] * along + across[1] * line,
            ]
        };
        let leader_pose = problem.poses[leader];
        let mismatch = |turn: f64| -> f64 {
            members
                .iter()
                .enumerate()
                .map(|(place, member)| {
                    let offset = core::problem::rotate(place_of(place), turn);
                    let at = problem.poses[*member].position;
                    (leader_pose.position[0] + offset[0] - at[0]).powi(2)
                        + (leader_pose.position[1] + offset[1] - at[1]).powi(2)
                })
                .sum()
        };
        // Only a row that lies exactly so (members within 0.01 mm of their
        // places) turns the frame: parts anywhere else keep the leader's.
        let exact = 1.0e-4 * members.len() as f64;
        let turn = if mismatch(0.0) > exact && mismatch(180.0) <= exact {
            180.0
        } else {
            0.0
        };
        let angle = leader_pose.angle + turn;
        let mut body = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut carried = problem.components[leader].clone();
        // Where a member blocks with shaped boxes (`hollow`), the row
        // blocks with every member's: its own boxes, or its whole body.
        let shaped = members
            .iter()
            .any(|member| !problem.components[*member].hollow.is_empty());
        carried.hollow.clear();
        carried.tight_hollow.clear();
        carried.pins.clear();
        carried.far_side.clear();
        carried.far_copper.clear();
        carried.far_courtyard.clear();
        carried.pads.clear();
        for (place, member) in members.iter().enumerate() {
            if leaders.contains_key(member) || leaders.values().any(|leader| leader == member) {
                return Err(format!("{} is in two rows", references[*member]));
            }
            let component = &problem.components[*member];
            if component.side != side {
                return Err(format!(
                    "row {:?}: all parts must be on one side",
                    row.parts
                ));
            }
            // The member's place in the leader's own frame.
            let offset = core::problem::rotate(place_of(place), angle);
            let shift = |boxes: &[[f64; 4]]| -> Vec<[f64; 4]> {
                boxes
                    .iter()
                    .map(|b| {
                        [
                            b[0] + offset[0],
                            b[1] + offset[1],
                            b[2] + offset[0],
                            b[3] + offset[1],
                        ]
                    })
                    .collect()
            };
            let center = [
                component.body_center[0] + offset[0],
                component.body_center[1] + offset[1],
            ];
            body = [
                body[0].min(center[0] - component.body_size[0] / 2.0),
                body[1].min(center[1] - component.body_size[1] / 2.0),
                body[2].max(center[0] + component.body_size[0] / 2.0),
                body[3].max(center[1] + component.body_size[1] / 2.0),
            ];
            carried
                .pins
                .extend(component.pins.iter().map(|pin| core::Pin {
                    offset: [pin.offset[0] + offset[0], pin.offset[1] + offset[1]],
                    net: pin.net,
                }));
            carried.far_side.extend(shift(&component.far_side));
            carried.far_copper.extend(shift(&component.far_copper));
            carried
                .far_courtyard
                .extend(shift(&component.far_courtyard));
            carried.pads.extend(shift(&pad_boxes[*member]));
            if shaped {
                if component.hollow.is_empty() {
                    let half = [component.body_size[0] / 2.0, component.body_size[1] / 2.0];
                    carried.hollow.push([
                        center[0] - half[0],
                        center[1] - half[1],
                        center[0] + half[0],
                        center[1] + half[1],
                    ]);
                } else {
                    carried.hollow.extend(shift(&component.hollow));
                }
            }
            carried.halo = carried.halo.max(component.halo);
            carried.edge_inset = carried.edge_inset.min(component.edge_inset);
            if *member != leader {
                leaders.insert(*member, leader);
                places.insert(
                    *member,
                    RowPlace {
                        offset,
                        center,
                        half: [component.body_size[0] / 2.0, component.body_size[1] / 2.0],
                    },
                );
                problem
                    .constraints
                    .followers
                    .push(core::constraints::Follower {
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
            component.far_copper.clear();
            component.far_courtyard.clear();
            component.pads.clear();
            component.halo = 0.0;
            component.body_size = [0.01, 0.01];
        }
    }
    let mut poses = problem.poses.clone();
    problem.sync_followers(&mut poses);
    problem.poses = poses;
    Ok((leaders, places, user_rows))
}

/// Where a row member sits in its leader's frame: its origin's offset, its
/// body's centre and half size.
struct RowPlace {
    offset: [f64; 2],
    center: [f64; 2],
    half: [f64; 2],
}

/// A region smaller than the part in every orientation it may take is an
/// input error, said at once: the placer would try every relaxation level
/// in vain first (a minute on an 80-part keyboard) and then report the
/// part as unplaced. The smallest body counts (the tight one, without the
/// courtyard's margin, where the part has one).
fn region_holds(
    component: &core::Component,
    reference: &str,
    region: [f64; 4],
) -> Result<(), String> {
    let mut size = component.body_size;
    if let Some(tight) = component.tight {
        size = [
            size[0].min(tight[2] - tight[0]),
            size[1].min(tight[3] - tight[1]),
        ];
    }
    let quarter = |angle: f64| {
        let turns = angle.rem_euclid(360.0) / 90.0;
        ((turns - turns.round()).abs() < 1.0e-6).then(|| turns.round() as i64 % 2)
    };
    let mut turns = Vec::new();
    for angle in &component.angle_options {
        match quarter(*angle) {
            Some(turn) => turns.push(turn),
            // A part that may turn at any angle: no quick answer.
            None => return Ok(()),
        }
    }
    if turns.is_empty() {
        turns.push(0);
    }
    let (width, height) = (region[2] - region[0], region[3] - region[1]);
    let fits = turns.iter().any(|turn| {
        let (w, h) = if *turn == 0 {
            (size[0], size[1])
        } else {
            (size[1], size[0])
        };
        w <= width + 1.0e-6 && h <= height + 1.0e-6
    });
    if fits {
        return Ok(());
    }
    Err(format!(
        "region for {reference} cannot hold it: the box is {width:.2} x {height:.2} mm, the part {:.2} x {:.2} mm (turned as it may be): give it a larger box",
        size[0], size[1]
    ))
}

/// Applies `constraints` to a lowered problem. `pads[i]` maps pad names of
/// footprint `i` to their local offsets. Returns warnings for parts of the
/// constraints that are accepted but not acted on.
#[allow(
    clippy::too_many_arguments,
    reason = "the lowered placement's per-footprint tables, each its own slice"
)]
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "!(x > 0.0) rejects NaN as well as zero and negative values"
)]
pub(super) fn apply_constraints(
    problem: &mut core::Problem,
    references: &[String],
    pads: &[BTreeMap<String, [f64; 2]>],
    pad_boxes: &[Vec<[f64; 4]>],
    keepouts: &[Option<[f64; 4]>],
    mouths: &[Option<[f64; 2]>],
    locked: &[bool],
    uuids: &[Option<String>],
    constraints: &KiCadPlacementConstraints,
    weight: f64,
) -> Result<Vec<String>, String> {
    let footprints = pads.len();
    let (leaders, places, user_rows) =
        apply_rows(problem, references, footprints, pad_boxes, &constraints.row)?;
    // Constraints no placement can keep (a region smaller than its part, a
    // row longer than the board): said at the end, all of them, with the
    // room the board would need.
    let mut cannot = Vec::new();
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
    // A reference several parts share (a panel's copies): an entry with a
    // `uuid` is about the footprint with that uuid (from_pcbench writes
    // one for every entry of a repeated reference). Without one, they are
    // named in turn: the n-th such entry of one kind for it is about its
    // n-th part in board order (hand-written constraints). All of them on
    // its first part asked one connector to touch the left, top and bottom
    // edges at once (the molecule panels' J3 to J6); in turn, the entries
    // of a task that writes them only for the copies at an edge bound
    // other copies (the molecules' fixed J1, J3 and J4 missed 17 edges by
    // whole tile pitches).
    let instance = |reference: &str,
                    uuid: Option<&str>,
                    used: &mut BTreeMap<String, usize>|
     -> Result<usize, String> {
        let parts: Vec<usize> = (0..footprints)
            .filter(|index| references[*index] == reference)
            .collect();
        let index = if let Some(uuid) = uuid {
            if parts.is_empty() {
                return find(reference);
            }
            *parts
                .iter()
                .find(|index| uuids.get(**index).and_then(Option::as_deref).is_some_and(|own| own.eq_ignore_ascii_case(uuid)))
                .ok_or_else(|| format!("constraint names {reference} with uuid {uuid:?}, which no footprint {reference} has"))?
        } else {
            if parts.len() < 2 {
                return find(reference);
            }
            let count = used.entry(reference.to_string()).or_default();
            let index = parts[*count % parts.len()];
            *count += 1;
            index
        };
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
            .filter(|index| {
                !leaders.contains_key(index) && glob_matches(pattern, &references[*index])
            })
            .collect()
    };
    let warnings = Vec::new();
    let mut constrained = BTreeSet::new();
    let bounds = problem.bounds();

    let mut used = BTreeMap::new();
    for rotation in &constraints.rotation {
        let index = instance(&rotation.part, rotation.uuid.as_deref(), &mut used)?;
        let mut angles = rotation.angles.clone();
        angles.extend(rotation.angle);
        if angles.is_empty() {
            return Err(format!(
                "rotation constraint for {} names no angle",
                rotation.part
            ));
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
        problem.components[index].angle_options = angles.clone();
        problem.constraints.kept.push(core::constraints::Kept {
            kind: "rotation",
            part: index,
            side: None,
            angles,
        });
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
        other => Err(format!(
            "unknown device_front {other:?} (left, right, top or bottom)"
        )),
    };
    let mut used = BTreeMap::new();
    for entry in &constraints.place {
        let index = instance(&entry.part, entry.uuid.as_deref(), &mut used)?;
        match (&entry.at, entry.x, entry.y) {
            (Some(at), None, None) => {
                let word = match at.as_str() {
                    "front" | "rear" => {
                        let front = constraints.device_front.as_deref().ok_or_else(|| {
                            format!("place {}: {at:?} needs device_front", entry.part)
                        })?;
                        opposite(front)?;
                        if at == "front" {
                            front.to_string()
                        } else {
                            opposite(front)?.to_string()
                        }
                    }
                    other => other.to_string(),
                };
                let (width, height) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
                let region = |x0: f64, y0: f64, x1: f64, y1: f64| {
                    [
                        bounds[0] + x0 * width,
                        bounds[1] + y0 * height,
                        bounds[0] + x1 * width,
                        bounds[1] + y1 * height,
                    ]
                };
                let (edges, area): (Vec<Edge>, Option<[f64; 4]>) = match word.as_str() {
                    "left" | "right" | "top" | "bottom" => (vec![edge(&word)?], None),
                    "top-left" => (vec![Edge::Top, Edge::Left], None),
                    "top-right" => (vec![Edge::Top, Edge::Right], None),
                    "bottom-left" => (vec![Edge::Bottom, Edge::Left], None),
                    "bottom-right" => (vec![Edge::Bottom, Edge::Right], None),
                    "center" => (
                        Vec::new(),
                        Some(region(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0)),
                    ),
                    "left-half" => (Vec::new(), Some(region(0.0, 0.0, 0.5, 1.0))),
                    "right-half" => (Vec::new(), Some(region(0.5, 0.0, 1.0, 1.0))),
                    "top-half" => (Vec::new(), Some(region(0.0, 0.0, 1.0, 0.5))),
                    "bottom-half" => (Vec::new(), Some(region(0.0, 0.5, 1.0, 1.0))),
                    other => {
                        return Err(format!(
                            "place {}: unknown position {other:?} (an edge, a corner like top-left, center, a half like left-half, front or rear)",
                            entry.part
                        ));
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
                    Some(other) => {
                        return Err(format!(
                            "unknown place origin {other:?} (board or absolute)"
                        ));
                    }
                };
                let angle = entry
                    .angle
                    .unwrap_or(problem.poses[index].angle)
                    .rem_euclid(360.0);
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
                ));
            }
        }
    }
    let mut used = BTreeMap::new();
    for entry in &constraints.edge {
        let index = instance(&entry.part, entry.uuid.as_deref(), &mut used)?;
        let reach = entry.max_mm.unwrap_or(if entry.flush { 0.0 } else { 1.0 });
        problem
            .constraints
            .edges
            .push((index, edge(&entry.edge)?, reach.max(0.0) + 1.0e-3));
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
                let box_center = component.offset(
                    [(local[0] + local[2]) / 2.0, (local[1] + local[3]) / 2.0],
                    angle,
                );
                let body_center = component.offset(component.body_center, angle);
                let direction = [
                    box_center[0] - body_center[0],
                    box_center[1] - body_center[1],
                ];
                match side {
                    Edge::Left => direction[0] < -direction[1].abs(),
                    Edge::Right => direction[0] > direction[1].abs(),
                    Edge::Top => direction[1] < -direction[0].abs(),
                    Edge::Bottom => direction[1] > direction[0].abs(),
                }
            };
            let angles: Vec<f64> = component
                .angle_options
                .iter()
                .copied()
                .filter(|angle| pointing(*angle))
                .collect();
            if angles.is_empty() {
                return Err(format!(
                    "edge constraint for {}: no allowed rotation points its keepout at the {} edge",
                    entry.part,
                    side.name()
                ));
            }
            if !angles
                .iter()
                .any(|angle| (angle - problem.poses[index].angle).abs() < 1.0e-6)
            {
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
            let angles: Vec<f64> = component
                .angle_options
                .iter()
                .copied()
                .filter(|angle| facing(*angle))
                .collect();
            if angles.is_empty() {
                return Err(format!(
                    "edge constraint for {}: no allowed rotation opens it towards the {} edge",
                    entry.part,
                    side.name()
                ));
            }
            if !angles
                .iter()
                .any(|angle| (angle - problem.poses[index].angle).abs() < 1.0e-6)
            {
                let center = component.center(problem.poses[index]);
                problem.poses[index] = core::Pose {
                    position: component.position_for_center(center, angles[0]),
                    angle: angles[0],
                };
            }
            problem.components[index].angle_options = angles.clone();
            problem.constraints.kept.push(core::constraints::Kept {
                kind: "opening_outwards",
                part: index,
                side: None,
                angles,
            });
        }
        constrained.insert(index);
    }
    for entry in &constraints.region {
        let offset = match entry.origin.as_deref() {
            None | Some("board") => [bounds[0], bounds[1]],
            Some("absolute") => [0.0, 0.0],
            Some(other) => {
                return Err(format!(
                    "unknown region origin {other:?} (board or absolute)"
                ));
            }
        };
        let region = match (&entry.at, entry.x, entry.y) {
            (None, Some(x), Some(y)) => [
                offset[0] + x[0].min(x[1]),
                offset[1] + y[0].min(y[1]),
                offset[0] + x[0].max(x[1]),
                offset[1] + y[0].max(y[1]),
            ],
            (Some(at), None, None) => {
                let [x0, y0, x1, y1] = match at.as_str() {
                    "left-half" => [0.0, 0.0, 0.5, 1.0],
                    "right-half" => [0.5, 0.0, 1.0, 1.0],
                    "top-half" => [0.0, 0.0, 1.0, 0.5],
                    "bottom-half" => [0.0, 0.5, 1.0, 1.0],
                    "center" => [1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0],
                    other => {
                        return Err(format!(
                            "region {:?}: unknown at {other:?} (left-half, right-half, top-half, bottom-half or center)",
                            entry.parts
                        ));
                    }
                };
                let (width, height) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
                [
                    bounds[0] + x0 * width,
                    bounds[1] + y0 * height,
                    bounds[0] + x1 * width,
                    bounds[1] + y1 * height,
                ]
            }
            _ => {
                return Err(format!(
                    "region {:?}: give both x and y, or at",
                    entry.parts
                ));
            }
        };
        let mut any = false;
        for pattern in &entry.parts {
            for index in matching(pattern) {
                if let Err(error) =
                    region_holds(&problem.components[index], &references[index], region)
                {
                    cannot.push(error);
                }
                problem.constraints.regions.push((index, region));
                constrained.insert(index);
                any = true;
            }
        }
        if !any {
            return Err(format!(
                "region constraint {:?} matches no part",
                entry.parts
            ));
        }
    }
    for entry in &constraints.near {
        let part = find(&entry.part)?;
        // The anchor may be a part a row carries (a keyboard's diode at its
        // switch): its place in the row, on the row's first part.
        let any = |reference: &str| -> Result<usize, String> {
            references[..footprints]
                .iter()
                .position(|candidate| candidate == reference)
                .ok_or_else(|| format!("constraint names unknown part {reference:?}"))
        };
        let anchor = match (&entry.part_of, &entry.pin_of) {
            (Some(other), None) => {
                let index = any(other)?;
                match (leaders.get(&index), places.get(&index)) {
                    (Some(leader), Some(place)) => {
                        Anchor::Area(*leader, index, place.center, place.half)
                    }
                    _ => Anchor::Body(index),
                }
            }
            (None, Some(pin)) => {
                let (reference, pad) = pin
                    .split_once(':')
                    .ok_or_else(|| format!("pin_of {pin:?} is not REF:PAD"))?;
                let index = any(reference)?;
                let offset = pads[index]
                    .get(pad)
                    .ok_or_else(|| format!("pin_of {pin:?}: {reference} has no pad {pad:?}"))?;
                match (leaders.get(&index), places.get(&index)) {
                    (Some(leader), Some(place)) => Anchor::Area(
                        *leader,
                        index,
                        [offset[0] + place.offset[0], offset[1] + place.offset[1]],
                        [0.0, 0.0],
                    ),
                    _ => Anchor::Point(index, *offset),
                }
            }
            _ => {
                return Err(format!(
                    "near constraint for {} needs exactly one of part_of and pin_of",
                    entry.part
                ));
            }
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
                    let area = |index: usize| {
                        problem.components[index].body_size[0]
                            * problem.components[index].body_size[1]
                    };
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
            return Err(format!(
                "apart constraint {:?}: min_mm must be positive",
                entry.parts
            ));
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
        let named: BTreeSet<usize> = constraints
            .fixed
            .iter()
            .flat_map(|pattern| matching(pattern))
            .collect();
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
            if locked[index]
                || leaders.contains_key(&index)
                || component.side == core::Side::Neither
            {
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
            if mounting_hole
                && !constrained.contains(&index)
                && !named.contains(&index)
                && !pinned.contains(&index)
            {
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
    if let Some(outline) = &constraints.outline {
        // On a polygon, a part whose centre lies off it (in an L's notch)
        // too.
        let polygonal = outline.polygonal() && problem.outline.len() >= 3;
        for index in 0..footprints {
            let component = &problem.components[index];
            let pose = problem.poses[index];
            let (center, half) = (component.center(pose), component.half_extent(pose.angle));
            let off = center[0] + half[0] <= bounds[0]
                || center[0] - half[0] >= bounds[2]
                || center[1] + half[1] <= bounds[1]
                || center[1] - half[1] >= bounds[3]
                || (polygonal && !point_in_polygon(center, &problem.outline));
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
    // A row or an edge's parts the outline cannot hold. Not for an
    // outline sized from the parts: the layout grows it while parts do
    // not fit.
    let automatic = constraints
        .outline
        .as_ref()
        .is_some_and(KiCadOutlineConstraint::automatic);
    let outlined = if automatic {
        Vec::new()
    } else {
        crate::placement_room::outline_holds(problem, references, &user_rows)
    };
    if !cannot.is_empty() || !outlined.is_empty() {
        crate::placement_room::note_room(crate::placement_room::room(
            problem,
            references,
            !outlined.is_empty(),
        ));
        cannot.extend(outlined);
        return Err(cannot.join("; "));
    }
    // The sides `apply_sides` flipped the parts to, row members included
    // (they lie on their row's side), reported as kept constraints. The
    // side to keep is where the flipped footprint's pads put the part, not
    // the constraint's name for it: urchin's key sockets are front
    // footprints with their pads on B.Cu, so on B.Cu (as `back` asks) the
    // part lies on the front, and every `back` entry for them was reported
    // missed on a board that kept them all.
    for (kind, patterns) in [("back", &constraints.back), ("front", &constraints.front)] {
        for index in (0..footprints).filter(|index| {
            patterns
                .iter()
                .any(|pattern| glob_matches(pattern, &references[*index]))
        }) {
            let side = problem.components[index].side;
            problem.constraints.kept.push(core::constraints::Kept {
                kind,
                part: index,
                side: Some(side),
                angles: Vec::new(),
            });
        }
    }
    Ok(warnings)
}

pub(super) fn constraint_report(
    problem: &core::Problem,
    poses: &[core::Pose],
    references: &[String],
) -> Vec<KiCadConstraintStatus> {
    // The constraints the lowering keeps by construction (sides, turns)
    // after the placed ones: listed as kept, like every other.
    core::constraints::report(problem, poses)
        .into_iter()
        .chain(core::constraints::report_kept(problem, poses))
        .map(|status| KiCadConstraintStatus {
            kind: status.kind.into(),
            part: references[status.part].clone(),
            other: status
                .other
                .filter(|other| *other != status.part)
                .map(|other| references[other].clone()),
            satisfied: status.satisfied,
            violation_mm: (status.violation * 1000.0).round() / 1000.0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    type PadNames = Vec<BTreeMap<String, [f64; 2]>>;

    fn problem() -> (core::Problem, Vec<String>, PadNames) {
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
            far_copper: Vec::new(),
            far_courtyard: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: Vec::new(),
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
            hole_rings: Vec::new(),
        };
        let problem = core::Problem {
            outline: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
            components: vec![part(true), part(false), part(false)],
            net_weights: Vec::new(),
            poses: vec![
                core::Pose {
                    position: [10.0, 10.0],
                    angle: 0.0
                };
                3
            ],
            spacing: 0.2,
            grid: 0.1,
            edge_margin: 0.5,
            min_spacing: 0.0,
            far_side_pads_only: false,
            shared_halos: false,
            escape: None,
            crossings: None,
            pieces: Vec::new(),
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
        let constraints: KiCadPlacementConstraints =
            serde_json::from_str(json).map_err(|e| e.to_string())?;
        let warnings = apply_constraints(
            &mut problem,
            &references,
            &pads,
            &[vec![[0.0, -0.5, 1.0, 0.5]], Vec::new(), Vec::new()],
            &[None, None, None],
            &[None, None, None],
            &[false, false, false],
            &[None, None, None],
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
                &[None, None, None],
                &constraints,
                10.0,
            )
            .unwrap();
            problem
        };
        let problem = run(r#"{"version": 1, "move_all": true}"#);
        assert!(problem.components.iter().all(|component| !component.fixed));
        assert_eq!(
            problem.constraints.edges,
            vec![(0, Edge::Top, 1.0 + 1.0e-3), (0, Edge::Left, 1.0 + 1.0e-3)]
        );
        // Named anywhere, it goes where it is told.
        let problem = run(r#"{"version": 1, "move_all": true, "fixed": ["J1"]}"#);
        assert!(problem.components[0].fixed && problem.constraints.edges.is_empty());
        let problem =
            run(r#"{"version": 1, "move_all": true, "place": [{"part": "J1", "at": "bottom"}]}"#);
        assert_eq!(
            problem.constraints.edges,
            vec![(0, Edge::Bottom, 1.0 + 1.0e-3)]
        );
    }

    #[test]
    fn a_repeated_reference_names_its_parts_in_turn() {
        // A panel's copies repeat references: C1 twice. The first edge
        // entry for C1 is about the first C1, the second about the second
        // (all on the first asked one part to touch two opposite edges).
        let (mut problem, mut references, pads) = problem();
        references[2] = "C1".to_string();
        let constraints: KiCadPlacementConstraints = serde_json::from_str(
            r#"{"version": 1,
                "edge": [{"part": "C1", "edge": "left"}, {"part": "C1", "edge": "right"}],
                "rotation": [{"part": "C1", "angle": 90}, {"part": "C1", "angle": 0}]}"#,
        )
        .unwrap();
        apply_constraints(
            &mut problem,
            &references,
            &pads,
            &[Vec::new(), Vec::new(), Vec::new()],
            &[None, None, None],
            &[None, None, None],
            &[false, false, false],
            &[None, None, None],
            &constraints,
            10.0,
        )
        .unwrap();
        let edges: Vec<(usize, Edge)> = problem
            .constraints
            .edges
            .iter()
            .map(|(part, edge, _)| (*part, *edge))
            .collect();
        assert_eq!(edges, vec![(1, Edge::Left), (2, Edge::Right)]);
        assert_eq!(problem.components[1].angle_options, vec![90.0]);
        assert_eq!(problem.components[2].angle_options, vec![0.0]);
    }

    #[test]
    fn a_repeated_reference_entry_with_a_uuid_names_that_footprint() {
        // The molecule panels' tasks list edge entries only for the copies
        // at an edge, not in board order, and some copies have none: in
        // turn, the first entry bound the first copy. By uuid each entry
        // binds its own footprint, whatever the order; entries without one
        // still take the parts in turn (hand-written constraints), and a
        // uuid no such footprint has is an error.
        let (mut problem, mut references, pads) = problem();
        references[2] = "C1".to_string();
        let uuids = [
            Some("aaaa".to_string()),
            Some("b1".to_string()),
            Some("C2".to_string()),
        ];
        let constraints: KiCadPlacementConstraints = serde_json::from_str(
            r#"{"version": 1,
                "edge": [{"part": "C1", "edge": "top", "uuid": "c2"}, {"part": "C1", "edge": "left"},
                         {"part": "C1", "edge": "bottom", "uuid": "c2"}],
                "rotation": [{"part": "C1", "angle": 90, "uuid": "c2"}],
                "place": [{"part": "C1", "at": "right", "uuid": "b1"}]}"#,
        )
        .unwrap();
        apply_constraints(
            &mut problem,
            &references,
            &pads,
            &[Vec::new(), Vec::new(), Vec::new()],
            &[None, None, None],
            &[None, None, None],
            &[false, false, false],
            &uuids,
            &constraints,
            10.0,
        )
        .unwrap();
        let edges: Vec<(usize, Edge)> = problem
            .constraints
            .edges
            .iter()
            .map(|(part, edge, _)| (*part, *edge))
            .collect();
        assert_eq!(
            edges,
            vec![
                (1, Edge::Right),
                (2, Edge::Top),
                (1, Edge::Left),
                (2, Edge::Bottom)
            ]
        );
        assert_eq!(problem.components[2].angle_options, vec![90.0]);
        assert_ne!(problem.components[1].angle_options, vec![90.0]);
        // Written back as read: no uuid where the entry had none.
        let written = serde_json::to_string(&constraints.edge[1]).unwrap();
        assert!(!written.contains("uuid"), "{written}");

        let (mut fresh, mut references, pads) = super::tests::problem();
        references[2] = "C1".to_string();
        let unknown: KiCadPlacementConstraints = serde_json::from_str(
            r#"{"version": 1, "edge": [{"part": "C1", "edge": "top", "uuid": "aaaa"}]}"#,
        )
        .unwrap();
        let error = apply_constraints(
            &mut fresh,
            &references,
            &pads,
            &[Vec::new(), Vec::new(), Vec::new()],
            &[None, None, None],
            &[None, None, None],
            &[false, false, false],
            &uuids,
            &unknown,
            10.0,
        )
        .unwrap_err();
        assert!(error.contains("uuid \"aaaa\""), "{error}");
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
            Relation::Near {
                part: 1,
                anchor: Anchor::Point(0, [0.5, 0.0]),
                max: 2.0
            }
        );
        assert!(matches!(
            problem.constraints.relations[1],
            Relation::Beside {
                part: 2,
                anchor: 1,
                side: Edge::Bottom,
                ..
            }
        ));
        assert_eq!(problem.components[2].angle_options, vec![90.0, 270.0]);
        assert_eq!(problem.poses[2].angle, 90.0);
    }

    #[test]
    fn a_region_in_words_is_that_part_of_the_outline_and_a_comment_is_read_past() {
        // The board is 30 x 20 mm: the left half is x 0-15, the centre the
        // middle third both ways.
        let (problem, _) = apply(
            r#"{"version": 1, "comment": ["inferred", "decide the pitch"],
                "region": [{"parts": ["C1"], "at": "left-half"}, {"parts": ["C2"], "at": "center"}]}"#,
        )
        .unwrap();
        assert_eq!(problem.constraints.regions[0], (1, [0.0, 0.0, 15.0, 20.0]));
        let (_, centre) = problem.constraints.regions[1];
        assert!(
            (centre[0] - 10.0).abs() < 1e-9 && (centre[3] - 40.0 / 3.0).abs() < 1e-9,
            "{centre:?}"
        );
        for wrong in [
            r#"{"version": 1, "region": [{"parts": ["C1"], "at": "left"}]}"#,
            r#"{"version": 1, "region": [{"parts": ["C1"], "at": "left-half", "x": [0, 1], "y": [0, 1]}]}"#,
            r#"{"version": 1, "region": [{"parts": ["C1"], "x": [0, 1]}]}"#,
        ] {
            assert!(apply(wrong).is_err(), "{wrong}");
        }
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
        assert_eq!(
            apply_outline(&mut pcb, &automatic, 4500.0).unwrap(),
            [100.0, 90.0]
        );
        // On four layers, 1.5 times the parts.
        let mut four = parse("(kicad_pcb (layers (0 \"F.Cu\" signal) (4 \"In1.Cu\" signal) (6 \"In2.Cu\" signal) (2 \"B.Cu\" signal)))").unwrap();
        assert_eq!(
            apply_outline(&mut four, &outline, 300.0).unwrap(),
            [30.0, 15.0]
        );
    }

    fn edge_cuts(pcb: &Expr) -> Vec<&Expr> {
        pcb.children()
            .iter()
            .filter(|item| form_atom(item, "layer", 1) == Some("Edge.Cuts"))
            .collect()
    }

    #[test]
    fn outline_shapes_parse_and_mistakes_are_errors() {
        let parsed: KiCadPlacementConstraints = serde_json::from_str(
            r#"{"version": 1, "outline": {"polygon": [[0, 0], [40, 0], [40, 10], [20, 10], [20, 30], [0, 30]], "corner_radius": 1,
                "cutouts": [{"x": [2, 6], "y": [2, 6]}, {"polygon": [[5, 15], [10, 15], [5, 20]], "origin": "absolute"}]}}"#,
        )
        .unwrap();
        let outline = parsed.outline.unwrap();
        assert_eq!(outline.polygon.as_ref().unwrap().len(), 6);
        assert_eq!(outline.cutouts.len(), 2);
        assert!(outline.polygonal() && !outline.automatic());
        let pieces: KiCadOutlineConstraint =
            serde_json::from_str(r#"{"pieces": [{"width": 20, "height": 10}, {"polygon": [[0, 0], [10, 0], [0, 10]], "x": 25}]}"#).unwrap();
        assert!(pieces.polygonal() && !pieces.automatic());
        let rounded: KiCadOutlineConstraint =
            serde_json::from_str(r#"{"width": 20, "height": 10, "corner_radius": 2}"#).unwrap();
        assert!(!rounded.polygonal());
        assert!(
            serde_json::from_str::<KiCadOutlineConstraint>(r#"{"polygon": [[0, 0]], "radius": 1}"#)
                .is_err(),
            "unknown key"
        );
        assert!(
            serde_json::from_str::<KiCadCutout>(r#"{"x": [0, 1], "y": [0, 1], "diameter": 1}"#)
                .is_err(),
            "unknown key"
        );

        let error = |json: &str| {
            let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)))").unwrap();
            let outline: KiCadOutlineConstraint = serde_json::from_str(json).unwrap();
            apply_outline(&mut pcb, &outline, 100.0).unwrap_err()
        };
        assert!(error(r#"{"polygon": [[0, 0], [10, 0]]}"#).contains("three corners"));
        assert!(error(r#"{"polygon": [[0, 0], [10, 0], [20, 0]]}"#).contains("no area"));
        assert!(
            error(r#"{"polygon": [[0, 0], [10, 0], [0, 10], [12, 14]]}"#)
                .contains("crosses itself")
        );
        assert!(
            error(r#"{"polygon": [[0, 0], [10, 0], [0, 10]], "width": 10}"#).contains("not both")
        );
        assert!(
            error(r#"{"polygon": [[0, 0], [10, 0], [0, 10]], "aspect": 2}"#)
                .contains("sized from the parts")
        );
        assert!(
            error(r#"{"width": 10, "height": 10, "corner_radius": -1}"#).contains("corner_radius")
        );
        assert!(
            error(r#"{"width": 10, "height": 10, "cutouts": [{"x": [8, 12], "y": [1, 2]}]}"#)
                .contains("does not lie inside")
        );
        assert!(error(r#"{"width": 10, "height": 10, "cutouts": [{"x": [1, 4], "y": [1, 4]}, {"x": [3, 6], "y": [3, 6]}]}"#).contains("cutouts may not overlap"));
        assert!(
            error(r#"{"width": 10, "height": 10, "cutouts": [{"x": [1, 4]}]}"#)
                .contains("x and y ranges or a polygon")
        );
        assert!(
            error(r#"{"cutouts": [{"x": [1, 4], "y": [1, 4]}]}"#).contains("need a sized outline")
        );
        assert!(
            error(
                r#"{"pieces": [{"width": 10, "height": 10}, {"width": 10, "height": 10, "x": 5}]}"#
            )
            .contains("pieces may not overlap")
        );
        assert!(
            error(r#"{"pieces": [{"width": 10, "height": 10}], "width": 5}"#)
                .contains("each piece carries its own shape")
        );
        assert!(error(r#"{"pieces": [{"x": 5}]}"#).contains("needs width and height or a polygon"));
    }

    #[test]
    fn outline_shapes_are_written_as_edge_cuts_lines_and_arcs() {
        use crate::outline::board_loops;
        let area = |points: &[[f64; 2]]| signed_area(points).abs();
        // An L, its corner at (5, 5): six lines, read back as one loop.
        let mut pcb = parse("(kicad_pcb (gr_rect (start 0 0) (end 1 1) (layer \"Edge.Cuts\")) (footprint \"x\" (at 10 10)))").unwrap();
        let outline: KiCadOutlineConstraint =
            serde_json::from_str(r#"{"polygon": [[0, 0], [40, 0], [40, 10], [20, 10], [20, 30], [0, 30]], "x": 5, "y": 5}"#).unwrap();
        assert_eq!(
            apply_outline(&mut pcb, &outline, 0.0).unwrap(),
            [40.0, 30.0]
        );
        let items = edge_cuts(&pcb);
        assert_eq!(items.len(), 6);
        assert!(items.iter().all(|item| item.head() == Some("gr_line")));
        assert_eq!(form_xy(items[0], "start").unwrap(), [5.0, 5.0]);
        assert_eq!(form_xy(items[0], "end").unwrap(), [45.0, 5.0]);
        let loops = board_loops(&pcb).unwrap();
        assert_eq!(loops.piece_loops.len(), 1);
        assert!(loops.cutouts.is_empty());
        assert!((area(&loops.outline) - (40.0 * 10.0 + 20.0 * 20.0)).abs() < 1.0e-9);

        // A rectangle with round corners, a box cut out and a triangle cut
        // out (absolute): four arcs, four lines, a box and three lines.
        let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)))").unwrap();
        let outline: KiCadOutlineConstraint = serde_json::from_str(
            r#"{"width": 30, "height": 20, "x": 0, "y": 0, "corner_radius": 3,
                "cutouts": [{"x": [10, 20], "y": [5, 10]}, {"polygon": [[2, 12], [6, 12], [2, 16]], "origin": "absolute"}]}"#,
        )
        .unwrap();
        assert_eq!(
            apply_outline(&mut pcb, &outline, 0.0).unwrap(),
            [30.0, 20.0]
        );
        let items = edge_cuts(&pcb);
        let count = |head: &str| {
            items
                .iter()
                .filter(|item| item.head() == Some(head))
                .count()
        };
        assert_eq!((count("gr_arc"), count("gr_line")), (4, 4 + 4 + 3));
        // The top-left arc: from (0, 3) through the corner's diagonal to (3, 0).
        let arc = items
            .iter()
            .find(|item| item.head() == Some("gr_arc"))
            .unwrap();
        assert_eq!(form_xy(arc, "start").unwrap(), [0.0, 3.0]);
        assert_eq!(form_xy(arc, "end").unwrap(), [3.0, 0.0]);
        let mid = form_xy(arc, "mid").unwrap();
        let expected = 3.0 - 3.0 / 2f64.sqrt();
        assert!(
            (mid[0] - expected).abs() < 1.0e-6 && (mid[1] - expected).abs() < 1.0e-6,
            "{mid:?}"
        );
        let loops = board_loops(&pcb).unwrap();
        // (The arcs are read back as chords within 5 um.)
        let rounded = 30.0 * 20.0 - (4.0 - std::f64::consts::PI) * 9.0;
        assert!(
            (area(&loops.outline) - rounded).abs() < 0.1,
            "{}",
            area(&loops.outline)
        );
        assert_eq!(loops.cutouts.len(), 2);
        assert_eq!(loops.board_cutouts, vec![true, true]);
        let mut holes: Vec<f64> = loops.cutouts.iter().map(|cutout| area(cutout)).collect();
        holes.sort_by(f64::total_cmp);
        assert!(
            (holes[0] - 8.0).abs() < 1.0e-9 && (holes[1] - 50.0).abs() < 1.0e-9,
            "{holes:?}"
        );

        // Two pieces, a rectangle and a triangle 5 mm to its right.
        let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)))").unwrap();
        let outline: KiCadOutlineConstraint = serde_json::from_str(
            r#"{"x": 100, "y": 50, "pieces": [{"width": 20, "height": 10}, {"polygon": [[0, 0], [10, 0], [0, 10]], "x": 25, "y": 0}]}"#,
        )
        .unwrap();
        assert_eq!(
            apply_outline(&mut pcb, &outline, 0.0).unwrap(),
            [35.0, 10.0]
        );
        let loops = board_loops(&pcb).unwrap();
        assert_eq!(
            loops.pieces,
            vec![[100.0, 50.0, 120.0, 60.0], [125.0, 50.0, 135.0, 60.0]]
        );
        assert!((area(&loops.outline) - 250.0).abs() < 1.0e-9);

        // A rectangle without round corners or cutouts stays one gr_rect.
        let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)))").unwrap();
        let plain: KiCadOutlineConstraint =
            serde_json::from_str(r#"{"width": 30, "height": 20, "corner_radius": 0}"#).unwrap();
        apply_outline(&mut pcb, &plain, 0.0).unwrap();
        assert_eq!(
            edge_cuts(&pcb)
                .iter()
                .map(|item| item.head())
                .collect::<Vec<_>>(),
            vec![Some("gr_rect")]
        );
        // A radius larger than a side allows rounds that corner less.
        let mut pcb = parse("(kicad_pcb (footprint \"x\" (at 10 10)))").unwrap();
        let slot: KiCadOutlineConstraint =
            serde_json::from_str(r#"{"width": 30, "height": 4, "corner_radius": 5}"#).unwrap();
        apply_outline(&mut pcb, &slot, 0.0).unwrap();
        let loops = board_loops(&pcb).unwrap();
        let stadium = 26.0 * 4.0 + std::f64::consts::PI * 4.0;
        assert!(
            (area(&loops.outline) - stadium).abs() < 0.1,
            "{}",
            area(&loops.outline)
        );
    }

    #[test]
    fn fixed_wins_over_a_constraint_and_mistakes_are_errors() {
        let (problem, _) = apply(r#"{"version": 1, "fixed": ["C1"], "near": [{"part": "C1", "part_of": "C2", "max_mm": 1}]}"#).unwrap();
        assert!(problem.components[1].fixed);
        let (problem, _) = apply(
            r#"{"version": 1, "group": [{"parts": ["C*", "J1"], "around": "J1", "max_mm": 2}]}"#,
        )
        .unwrap();
        assert_eq!(problem.constraints.relations.len(), 2);
        assert!(
            problem
                .constraints
                .relations
                .iter()
                .all(|relation| matches!(
                    relation,
                    Relation::Near { anchor: Anchor::Body(0), max, .. } if *max == 2.0
                ))
        );
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
        let (problem, _) = apply(
            r#"{"version": 1, "apart": [{"parts": ["C1"], "from": ["C*", "J1"], "min_mm": 5}]}"#,
        )
        .unwrap();
        assert_eq!(
            problem.constraints.relations,
            vec![
                Relation::Apart {
                    part: 1,
                    anchor: 2,
                    min: 5.0
                },
                Relation::Apart {
                    part: 1,
                    anchor: 0,
                    min: 5.0
                }
            ]
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
        let edges: Vec<(usize, Edge)> = problem
            .constraints
            .edges
            .iter()
            .map(|(part, edge, _)| (*part, *edge))
            .collect();
        assert_eq!(
            edges,
            vec![(1, Edge::Left), (2, Edge::Bottom), (2, Edge::Right)]
        );
        assert!(!problem.components[1].fixed && !problem.components[2].fixed);
        assert_eq!(
            problem.poses[0],
            core::Pose {
                position: [5.0, 6.0],
                angle: 90.0
            }
        );
        assert!(problem.components[0].fixed);
        let (problem, _) = apply(r#"{"version": 1, "place": [{"part": "C1", "at": "center"}, {"part": "C2", "at": "top-half"}]}"#).unwrap();
        let expected = [
            (1, [10.0, 20.0 / 3.0, 20.0, 40.0 / 3.0]),
            (2, [0.0, 0.0, 30.0, 10.0]),
        ];
        assert_eq!(problem.constraints.regions.len(), 2);
        for ((part, region), (want_part, want)) in problem.constraints.regions.iter().zip(expected)
        {
            assert_eq!(*part, want_part);
            assert!(
                region.iter().zip(want).all(|(a, b)| (a - b).abs() < 1.0e-9),
                "{region:?}"
            );
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
        let (problem, _) =
            apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}]}"#).unwrap();
        // C1 carries C2 three millimetres to its right: one 5 x 1 mm body.
        assert_eq!(problem.components[1].body_size, [5.0, 1.0]);
        assert_eq!(problem.components[1].body_center, [1.5, 0.0]);
        assert!(problem.components[2].fixed && problem.components[2].side == core::Side::Neither);
        assert_eq!(
            problem.constraints.followers,
            vec![core::constraints::Follower {
                part: 2,
                leader: 1,
                offset: [3.0, 0.0],
                angle: 0.0
            }]
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
    fn constraints_kept_by_construction_are_reported() {
        // A side, the turns a part may take, a connector opening outwards:
        // the placer cannot break them; the report lists them as kept.
        let (mut problem, references, pads) = problem();
        let constraints: KiCadPlacementConstraints = serde_json::from_str(
            r#"{"version": 1, "front": ["C1"], "rotation": [{"part": "C2", "angles": [90, 270]}],
                "edge": [{"part": "C1", "edge": "left", "opening_outwards": true}]}"#,
        )
        .unwrap();
        apply_constraints(
            &mut problem,
            &references,
            &pads,
            &[Vec::new(), Vec::new(), Vec::new()],
            &[None, None, None],
            // C1 opens towards -x in its own frame.
            &[None, Some([-1.0, 0.0]), None],
            &[false, false, false],
            &[None, None, None],
            &constraints,
            10.0,
        )
        .unwrap();
        let kept = |poses: &[core::Pose]| -> Vec<(String, String, bool)> {
            constraint_report(&problem, poses, &references)
                .into_iter()
                .filter(|status| status.kind != "edge")
                .map(|status| (status.kind, status.part, status.satisfied))
                .collect()
        };
        let row = |kind: &str, part: &str, satisfied: bool| {
            (kind.to_string(), part.to_string(), satisfied)
        };
        assert_eq!(
            kept(&problem.poses),
            vec![
                row("rotation", "C2", true),
                row("opening_outwards", "C1", true),
                row("front", "C1", true)
            ]
        );
        // Turned another way, they would be missed.
        let mut poses = problem.poses.clone();
        poses[1].angle = 180.0;
        poses[2].angle = 0.0;
        assert_eq!(
            kept(&poses),
            vec![
                row("rotation", "C2", false),
                row("opening_outwards", "C1", false),
                row("front", "C1", true)
            ]
        );

        // A footprint `back` put on B.Cu whose pads are on the front (a key
        // socket under its switch) lies on the front: kept all the same.
        let (mut socket, references, pads) = self::problem();
        let constraints: KiCadPlacementConstraints =
            serde_json::from_str(r#"{"version": 1, "back": ["C1"]}"#).unwrap();
        assert_eq!(socket.components[1].side, core::Side::Front);
        apply_constraints(
            &mut socket,
            &references,
            &pads,
            &[Vec::new(), Vec::new(), Vec::new()],
            &[None, None, None],
            &[None, None, None],
            &[false, false, false],
            &[None, None, None],
            &constraints,
            10.0,
        )
        .unwrap();
        let statuses: Vec<(String, bool)> = constraint_report(&socket, &socket.poses, &references)
            .into_iter()
            .map(|status| (status.kind, status.satisfied))
            .collect();
        assert_eq!(statuses, vec![("back".to_string(), true)]);
    }

    #[test]
    fn a_row_the_outline_cannot_hold_is_an_error_at_once() {
        // Two 2 x 1 mm parts 25 mm apart: 27 mm on a 30 mm board; 40 apart
        // do not fit, turned by a half or not (a row turns no further).
        assert!(apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 25}]}"#).is_ok());
        let error =
            apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 40}]}"#).unwrap_err();
        assert!(
            error.contains(
                "the row of C1 (2 parts) cannot fit the outline: the row is 42.00 x 1.00 mm"
            ),
            "{error}"
        );
        // The room it would need is noted for the verdict (other tests may
        // note theirs in between: only that there is one).
        assert!(crate::placement_room::noted_room().is_some());
        // With a region too small as well, both are said.
        let error = apply(
            r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 40}], "region": [{"parts": ["J1"], "x": [0, 1], "y": [0, 0.5]}]}"#,
        )
        .unwrap_err();
        assert!(
            error.contains("region for J1 cannot hold it") && error.contains("the row of C1"),
            "{error}"
        );
    }

    #[test]
    fn a_turned_row_keeps_its_members() {
        // A board placed before: the placer turned the row (C1 at 180
        // degrees, C2 three millimetres to its left). Lowered again, the
        // row runs the way it lies; it used to run to the right and pull
        // C2 there.
        let (mut problem, references, pads) = problem();
        problem.poses[1] = core::Pose {
            position: [10.0, 10.0],
            angle: 180.0,
        };
        problem.poses[2] = core::Pose {
            position: [7.0, 10.0],
            angle: 180.0,
        };
        let constraints: KiCadPlacementConstraints =
            serde_json::from_str(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}]}"#)
                .unwrap();
        apply_constraints(
            &mut problem,
            &references,
            &pads,
            &[Vec::new(), Vec::new(), Vec::new()],
            &[None, None, None],
            &[None, None, None],
            &[false, false, false],
            &[None, None, None],
            &constraints,
            10.0,
        )
        .unwrap();
        assert_eq!(problem.poses[2].position, [7.0, 10.0]);
        let center = problem.components[1].center(problem.poses[1]);
        assert!(
            (center[0] - 8.5).abs() < 1.0e-9 && (center[1] - 10.0).abs() < 1.0e-9,
            "{center:?}"
        );
    }

    #[test]
    fn near_may_name_a_row_member_as_its_anchor() {
        // D1 near C2, which the row C1, C2 carries: the anchor is C2's
        // place in the row (its body, or its pad), on the row's first part.
        let (mut problem, mut references, mut pads) = problem();
        problem.components.push(problem.components[1].clone());
        problem.poses.push(core::Pose {
            position: [10.0, 10.0],
            angle: 0.0,
        });
        references.push("D1".into());
        pads[2].insert("1".into(), [0.5, 0.0]);
        pads.push(BTreeMap::new());
        let constraints: KiCadPlacementConstraints = serde_json::from_str(
            r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}],
                "near": [{"part": "D1", "part_of": "C2", "max_mm": 1}, {"part": "D1", "pin_of": "C2:1", "max_mm": 1}]}"#,
        )
        .unwrap();
        apply_constraints(
            &mut problem,
            &references,
            &pads,
            &[Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            &[None, None, None, None],
            &[None, None, None, None],
            &[false, false, false, false],
            &[None, None, None, None],
            &constraints,
            10.0,
        )
        .unwrap();
        assert_eq!(
            problem.constraints.relations,
            vec![
                Relation::Near {
                    part: 3,
                    anchor: Anchor::Area(1, 2, [3.0, 0.0], [1.0, 0.5]),
                    max: 1.0
                },
                Relation::Near {
                    part: 3,
                    anchor: Anchor::Area(1, 2, [3.5, 0.0], [0.0, 0.0]),
                    max: 1.0
                },
            ]
        );
        assert!(apply(r#"{"version": 1, "row": [{"parts": ["C*"], "pitch_mm": 3}], "near": [{"part": "C2", "part_of": "J1", "max_mm": 1}]}"#).is_err());
    }

    #[test]
    fn side_constraints_flip_parts_onto_their_side() {
        let footprint = |reference: &str, layer: &str| {
            format!(
                "(footprint \"R\" (layer \"{layer}\") (at 5 5) (property \"Reference\" \"{reference}\" (at 0 -1 0) (layer \"F.SilkS\")) \
                 (pad \"1\" smd rect (at 1 1) (size 1 1) (layers \"{layer}\")))"
            )
        };
        let board = format!(
            "(kicad_pcb {} {} {})",
            footprint("R1", "F.Cu"),
            footprint("R2", "F.Cu"),
            footprint("C1", "B.Cu")
        );
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
        assert_eq!(
            sides(r#"{"version": 1, "back": ["R*"], "front": ["C1"]}"#).unwrap(),
            ["B.Cu", "B.Cu", "F.Cu"]
        );
        // A part already on its side stays as it is.
        assert_eq!(
            sides(r#"{"version": 1, "back": ["C1", "R2"]}"#).unwrap(),
            ["F.Cu", "B.Cu", "B.Cu"]
        );
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
            serde_json::from_str::<KiCadPlacementConstraints>(json)
                .unwrap()
                .keepout
        };
        let zones = |pcb: &Expr| -> Vec<Expr> {
            pcb.children()
                .iter()
                .filter(|item| item.head() == Some("zone"))
                .cloned()
                .collect()
        };
        let held = &HeldParts::default();
        apply_keepouts(
            &mut pcb,
            &keepouts(r#"{"version": 1, "keepout": [{"x": [0, 5], "y": [2, 4], "copper": true}, {"x": [1, 2], "y": [1, 2], "side": "back"}]}"#),
            held,
        )
        .unwrap();
        let written = zones(&pcb);
        assert_eq!(written.len(), 2);
        // Relative to the outline's corner; copper keepouts block tracks on every layer.
        let text = encode(&written[0]).to_string();
        assert!(
            text.contains("(xy 10 22)") && text.contains("(xy 15 24)"),
            "{text}"
        );
        assert!(
            text.contains("\"*.Cu\"") && text.contains("(tracks not_allowed)"),
            "{text}"
        );
        let text = encode(&written[1]).to_string();
        assert!(
            text.contains("\"B.Cu\"")
                && text.contains("(tracks allowed)")
                && text.contains("(footprints not_allowed)")
        );
        // Applying again replaces them; nothing removes them all.
        apply_keepouts(
            &mut pcb,
            &keepouts(r#"{"version": 1, "keepout": [{"x": [0, 5], "y": [0, 5]}]}"#),
            held,
        )
        .unwrap();
        assert_eq!(zones(&pcb).len(), 1);
        apply_keepouts(&mut pcb, &[], held).unwrap();
        assert!(zones(&pcb).is_empty());
        for bad in [
            r#"{"version": 1, "keepout": [{"x": [0, 0], "y": [0, 5]}]}"#,
            r#"{"version": 1, "keepout": [{"x": [0, 1], "y": [0, 5], "side": "top"}]}"#,
            r#"{"version": 1, "keepout": [{"x": [0, 1], "y": [0, 5], "origin": "center"}]}"#,
        ] {
            assert!(
                apply_keepouts(&mut pcb, &keepouts(bad), held).is_err(),
                "{bad} was accepted"
            );
        }
    }

    #[test]
    fn one_notion_of_a_held_part_covers_every_case() {
        // A 50 x 40 mm board; each part is held or not for one reason.
        let part = |reference: &str, at: [f64; 2], extra: &str, net: bool| {
            format!(
                r#"(footprint "p" (layer "F.Cu") (at {} {}) {extra} (property "Reference" "{reference}")
              (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
              (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu"){}))"#,
                at[0],
                at[1],
                if net { " (net 1 \"A\")" } else { "" }
            )
        };
        let header = r#"(footprint "hdr" (layer "F.Cu") (at 30 21.5) (property "Reference" "J1")
              (fp_rect (start -6 -1.2) (end 6 1.2) (layer "F.CrtYd"))
              (pad "1" thru_hole circle (at -3.81 0) (size 1.7 1.7) (drill 1) (layers "*.Cu") (net 1 "A"))
              (pad "2" thru_hole circle (at -1.27 0) (size 1.7 1.7) (drill 1) (layers "*.Cu") (net 1 "A"))
              (pad "3" thru_hole circle (at 1.27 0) (size 1.7 1.7) (drill 1) (layers "*.Cu") (net 1 "A"))
              (pad "4" thru_hole circle (at 3.81 0) (size 1.7 1.7) (drill 1) (layers "*.Cu") (net 1 "A")))"#;
        let parts = [
            part("R1", [30.0, 40.0], "", true),             // plain: moves
            part("F1", [20.0, 40.0], "", true),             // placer fixed
            part("P1", [25.0, 40.0], "", true),             // fixed_patterns
            part("C1", [35.0, 40.0], "", true),             // constraints fixed
            part("X1", [40.0, 40.0], "", true),             // place with x and y
            part("X2", [45.0, 40.0], "", true),             // place by a word
            part("L1", [30.0, 30.0], "(locked yes)", true), // locked
            part("L2", [35.0, 30.0], "(locked yes)", true), // locked, on an edge constraint
            part("N1", [40.0, 30.0], "", false),            // no net
            part("N2", [45.0, 30.0], "", false),            // no net, free
            part("E1", [11.0, 40.0], "", true),             // body over the outline
            header.to_string(),                             // a connector at the edge
        ];
        let text = format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 10 20) (end 60 60) (layer "Edge.Cuts")) {})"#,
            parts.join("\n")
        );
        let pcb = parse(&text).unwrap();
        let constraints: KiCadPlacementConstraints = serde_json::from_value(serde_json::json!({
            "version": 1,
            "fixed": ["C1"],
            "place": [{"part": "X1", "x": 30, "y": 20}, {"part": "X2", "at": "center"}],
            "edge": [{"part": "L2", "edge": "any"}]
        }))
        .unwrap();
        let config = KiCadBoardPlacerConfig {
            fixed: vec!["F1".into()],
            fixed_patterns: vec!["P*".into()],
            free: vec!["N2".into()],
            constraints: Some(KiCadConstraintsSource::Inline(constraints.clone())),
            ..Default::default()
        };
        let held_names = |held: &HeldParts| -> Vec<String> {
            pcb.children()
                .iter()
                .filter(|item| item.head() == Some("footprint"))
                .enumerate()
                .filter(|(index, _)| held.holds(*index))
                .map(|(_, footprint)| footprint_reference(footprint).unwrap_or_default())
                .collect()
        };
        assert_eq!(
            held_names(&held_parts(&pcb, &config).unwrap()),
            ["F1", "P1", "C1", "X1", "L1", "N1", "E1", "J1"]
        );
        // A row's members go with its leader: N1 (no net) behind the
        // movable R1 moves with it.
        let mut in_row = constraints.clone();
        in_row.row =
            serde_json::from_value(serde_json::json!([{"parts": ["R1", "N1"], "pitch_mm": 3}]))
                .unwrap();
        let row_config = KiCadBoardPlacerConfig {
            constraints: Some(KiCadConstraintsSource::Inline(in_row)),
            ..config.clone()
        };
        assert_eq!(
            held_names(&held_parts(&pcb, &row_config).unwrap()),
            ["F1", "P1", "C1", "X1", "L1", "E1", "J1"]
        );
        // move_all: every part is placed but locked ones and those
        // constraints.json fixes or places (docs/constraints.md); the
        // placer's own `fixed` list and the defaults go too.
        let mut all = constraints;
        all.move_all = true;
        let config = KiCadBoardPlacerConfig {
            constraints: Some(KiCadConstraintsSource::Inline(all)),
            ..config
        };
        assert_eq!(
            held_names(&held_parts(&pcb, &config).unwrap()),
            ["C1", "X1", "L1"]
        );
    }

    #[test]
    fn a_keepout_over_a_fixed_part_allows_footprints_and_warns() {
        // J2 (fixed) reaches 1 mm into the first box; R1 (movable) lies in
        // the second; H1 (no net) is in the third, on the back only.
        let board = r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 10 20) (end 60 60) (layer "Edge.Cuts"))
            (footprint "conn" (layer "F.Cu") (at 20 25 90) (property "Reference" "J2")
              (fp_rect (start -1 -3) (end 1 3) (layer "F.CrtYd"))
              (pad "1" thru_hole circle (at 0 0) (size 1.7 1.7) (drill 1) (layers "*.Cu") (net 1 "A")))
            (footprint "r" (layer "F.Cu") (at 40 40) (property "Reference" "R1")
              (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
              (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "A")))
            (footprint "hole" (layer "B.Cu") (at 30 50) (property "Reference" "H1")
              (fp_circle (center 0 0) (end 2 0) (layer "B.CrtYd"))
              (pad "" np_thru_hole circle (at 0 0) (size 3 3) (drill 3) (layers "*.Cu"))))"#;
        let keepouts: Vec<KiCadKeepoutConstraint> =
            serde_json::from_str::<KiCadPlacementConstraints>(
                r#"{"version": 1, "keepout": [
                {"x": [12, 20], "y": [0, 4]},
                {"x": [25, 35], "y": [15, 25], "copper": true},
                {"x": [15, 25], "y": [25, 35], "side": "back"}]}"#,
            )
            .unwrap()
            .keepout;
        let mut pcb = parse(board).unwrap();
        let config = KiCadBoardPlacerConfig {
            fixed: vec!["J2".into()],
            ..Default::default()
        };
        let held = &held_parts(&pcb, &config).unwrap();
        // J2 at (20, 25) turned a quarter: its courtyard is x 17..23, y
        // 24..26 on the board; the first box (from the corner (10, 20)) x
        // 22..30, y 20..24 only touches it. H1 (no net: held) lies in the
        // third, R1 (movable) in the second.
        let inside = held_parts_in_keepouts(&pcb, &keepouts, held).unwrap();
        assert_eq!(
            inside,
            [Vec::<String>::new(), Vec::new(), vec!["H1".to_string()]]
        );
        let warnings = apply_keepouts(&mut pcb, &keepouts, held).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].starts_with("keepout 3 ") && warnings[0].contains("(H1)"),
            "{}",
            warnings[0]
        );
        let zones: Vec<String> = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("zone"))
            .map(|zone| encode(zone).to_string())
            .collect();
        assert!(zones[0].contains("(footprints not_allowed)"));
        assert!(
            zones[1].contains("(footprints not_allowed)")
                && zones[1].contains("(tracks not_allowed)")
        );
        assert!(zones[2].contains("(footprints allowed)"), "{}", zones[2]);
        // A box reaching into the fixed J2's courtyard: footprints allowed;
        // copper stays out.
        let over: Vec<KiCadKeepoutConstraint> = serde_json::from_str::<KiCadPlacementConstraints>(
            r#"{"version": 1, "keepout": [{"x": [12, 20], "y": [0, 4.5], "copper": true}]}"#,
        )
        .unwrap()
        .keepout;
        let warnings = apply_keepouts(&mut pcb, &over, held).unwrap();
        assert!(
            warnings[0].contains("(J2)") && warnings[0].contains("tracks, vias and pours stay out")
        );
        let zone = pcb
            .children()
            .iter()
            .find(|item| item.head() == Some("zone"))
            .map(|zone| encode(zone).to_string())
            .unwrap();
        assert!(zone.contains("(footprints allowed)") && zone.contains("(tracks not_allowed)"));
        // Not fixed: the movable part is the placer's to move.
        let movable = KiCadBoardPlacerConfig {
            free: vec!["H1".into()],
            ..Default::default()
        };
        let movable = held_parts(&pcb, &movable).unwrap();
        assert!(
            apply_keepouts(&mut pcb, &over, &movable)
                .unwrap()
                .is_empty()
        );
        // Fixed by constraints.json: held as by the placer's own list (the
        // fence's issue 15 for that input).
        let constraints: KiCadPlacementConstraints =
            serde_json::from_str(r#"{"version": 1, "fixed": ["J2"]}"#).unwrap();
        let by_constraints = KiCadBoardPlacerConfig {
            free: vec!["H1".into()],
            constraints: Some(KiCadConstraintsSource::Inline(constraints)),
            ..Default::default()
        };
        let by_constraints = held_parts(&pcb, &by_constraints).unwrap();
        let warnings = apply_keepouts(&mut pcb, &over, &by_constraints).unwrap();
        assert!(warnings[0].contains("(J2)"), "{warnings:?}");
    }

    #[test]
    fn the_input_check_warns_of_a_held_part_in_a_keepout_with_an_outline_too() {
        // H1 (no net: held) on the back at (30, 50), the outline's corner
        // at (10, 20): the box x 15..25, y 25..35 from the corner holds it.
        // The import writes an outline; the check applies it first, as
        // the placement does.
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-keepout-warnings-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("b.kicad_pcb"),
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 10 20) (end 60 60) (layer "Edge.Cuts"))
            (footprint "hole" (layer "B.Cu") (at 30 50) (property "Reference" "H1")
              (fp_circle (center 0 0) (end 2 0) (layer "B.CrtYd"))
              (pad "" np_thru_hole circle (at 0 0) (size 3 3) (drill 3) (layers "*.Cu"))))"#,
        )
        .unwrap();
        for outline in ["", r#""outline": {"width": 50, "height": 40},"#] {
            let constraints: KiCadPlacementConstraints = serde_json::from_str(&format!(
                r#"{{"version": 1, {outline} "keepout": [{{"x": [15, 25], "y": [25, 35]}}]}}"#
            ))
            .unwrap();
            let config = KiCadBoardPlacerConfig {
                constraints: Some(KiCadConstraintsSource::Inline(constraints)),
                ..KiCadBoardPlacerConfig::default()
            };
            let warnings = keepout_warnings(&directory, "b", &config).unwrap();
            assert_eq!(warnings.len(), 1, "{outline}: {warnings:?}");
            assert!(warnings[0].contains("(H1)"), "{}", warnings[0]);
        }
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn fixed_parts_at_one_spot_are_warned_of() {
        // C1 and C2 fixed at one spot; C3 fixed beside them (courtyards
        // touch); R1 movable on top of C1; U1 on the back under C1 (its
        // courtyard on the other side, its pads on B.Cu).
        let part = |reference: &str, x: f64, y: f64, layer: &str| {
            format!(
                r#"(footprint "c" (layer "{layer}.Cu") (at {x} {y}) (property "Reference" "{reference}")
                  (fp_rect (start -1 -0.5) (end 1 0.5) (layer "{layer}.CrtYd"))
                  (pad "1" smd rect (at -0.5 0) (size 0.6 0.6) (layers "{layer}.Cu") (net 1 "A"))
                  (pad "2" smd rect (at 0.5 0) (size 0.6 0.6) (layers "{layer}.Cu") (net 2 "B")))"#
            )
        };
        let board = format!(
            "(kicad_pcb (layers (0 \"F.Cu\" signal) (31 \"B.Cu\" signal)) (gr_rect (start 0 0) (end 30 20) (layer \"Edge.Cuts\")) {} {} {} {} {})",
            part("C1", 10.0, 10.0, "F"),
            part("C2", 10.0, 10.0, "F"),
            part("C3", 12.0, 10.0, "F"),
            part("R1", 10.0, 10.0, "F"),
            part("U1", 10.0, 10.0, "B"),
        );
        let pcb = parse(&board).unwrap();
        let fixed = [
            "C1".to_string(),
            "C2".to_string(),
            "C3".to_string(),
            "U1".to_string(),
        ];
        let config = KiCadBoardPlacerConfig {
            fixed: fixed.to_vec(),
            ..Default::default()
        };
        let held = held_parts(&pcb, &config).unwrap();
        let pairs = held_parts_overlapping(&pcb, &held).unwrap();
        assert_eq!(
            pairs,
            [(
                "C1".to_string(),
                "C2".to_string(),
                "courtyards and pads on F.Cu".to_string()
            )]
        );
        let warning = overlap_warning(&pairs[0].0, &pairs[0].1, &pairs[0].2);
        assert!(
            warning.starts_with("fixed parts C1 and C2 overlap"),
            "{warning}"
        );
        // Not fixed, C2 is the placer's to move: nothing to warn of.
        let fixed = ["C1".to_string(), "C3".to_string(), "U1".to_string()];
        let config = KiCadBoardPlacerConfig {
            fixed: fixed.to_vec(),
            ..Default::default()
        };
        let held = held_parts(&pcb, &config).unwrap();
        assert!(held_parts_overlapping(&pcb, &held).unwrap().is_empty());
    }
}

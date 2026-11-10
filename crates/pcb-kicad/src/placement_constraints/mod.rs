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

// One file per constraint kind (or group of kinds), each with its entries'
// types and the stage of `apply_constraints` that applies them; the outline,
// keepouts and sides are applied to the board itself before the lowering.
mod board_outline;
mod edge;
mod fixed;
mod held;
mod hollow;
mod keepout;
mod lookup;
mod place;
mod region;
mod relations;
mod rotation;
mod rows;
mod sides;

pub use board_outline::KiCadOutlineConstraint;
pub(crate) use board_outline::{apply_outline, default_area_factor};
pub use edge::KiCadEdgeConstraint;
use edge::{apply_edges, edge};
use fixed::{
    apply_move_all, fix_pinned_and_fixed, free_constrained, free_parts_off_the_outline,
    refuse_what_cannot_be_kept,
};
use held::courtyard_box;
#[cfg(test)]
pub(crate) use held::overlap_warning;
pub(crate) use held::{HeldParts, held_parts, held_parts_as_given, pad_boxes, part_box};
pub use held::{overlap_warnings, placement_input_warnings};
use hollow::apply_hollow;
pub(crate) use keepout::{KEEPOUT_NAME, apply_keepouts, outline_corner};
pub use keepout::{KiCadKeepoutConstraint, keepout_warnings};
use lookup::Parts;
use place::apply_places;
pub(crate) use place::expand_additions;
pub use place::{KiCadAddConstraint, KiCadPlaceConstraint};
pub use region::KiCadRegionConstraint;
use region::apply_regions;
pub use relations::{
    KiCadApartConstraint, KiCadGroupConstraint, KiCadNearConstraint, KiCadRelativeConstraint,
};
use relations::{apply_apart, apply_groups, apply_near, apply_relative};
pub use rotation::KiCadRotationConstraint;
use rotation::apply_rotations;
pub use rows::KiCadRowConstraint;
pub(crate) use rows::natural;
use rows::{RowPlace, apply_rows, rows_off_the_board};
pub(crate) use sides::apply_sides;
use sides::report_sides;

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
    /// What the entry is beyond kept or missed (a pair row's pitch and how
    /// it faces the pair's ends).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
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

/// Applies `constraints` to a lowered problem. `pads[i]` maps pad names of
/// footprint `i` to their local offsets. Returns warnings for parts of the
/// constraints that are accepted but not acted on.
///
/// The stages run in this order, and each refuses its own mistakes as it
/// meets them: rows (each made one part), rotation, place, edge, region,
/// near, group, apart, relative; then which parts move (`move_all`,
/// `hollow`, the parts the constraints name, `fixed`); last, all the
/// constraints no placement can keep at once, and the sides as kept.
#[allow(
    clippy::too_many_arguments,
    reason = "the lowered placement's per-footprint tables, each its own slice"
)]
pub(crate) fn apply_constraints(
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
    let parts = Parts {
        references,
        footprints,
        pads,
        pad_boxes,
        keepouts,
        mouths,
        locked,
        uuids,
        leaders,
        places,
    };
    // Constraints no placement can keep (a region smaller than its part, a
    // row longer than the board): said at the end, all of them, with the
    // room the board would need.
    let mut cannot = Vec::new();
    let warnings = Vec::new();
    let mut constrained = BTreeSet::new();
    let bounds = problem.bounds();

    apply_rotations(problem, &parts, &constraints.rotation, &mut constrained)?;
    let pinned = apply_places(problem, &parts, constraints, bounds, &mut constrained)?;
    apply_edges(problem, &parts, &constraints.edge, &mut constrained)?;
    apply_regions(
        problem,
        &parts,
        &constraints.region,
        bounds,
        &mut constrained,
        &mut cannot,
    )?;
    apply_near(problem, &parts, &constraints.near, &mut constrained)?;
    apply_groups(problem, &parts, &constraints.group, &mut constrained)?;
    apply_apart(problem, &parts, &constraints.apart, &mut constrained)?;
    apply_relative(problem, &parts, &constraints.relative, &mut constrained)?;
    problem.constraints.relation_weight = weight;

    // Constrained parts move (the user said where they go), fixed ones stay.
    if constraints.move_all {
        apply_move_all(problem, &parts, &constraints.fixed, &constrained, &pinned);
    }
    apply_hollow(problem, &parts, &constraints.hollow)?;
    free_constrained(problem, &parts, constraints, constrained);
    if let Some(outline) = &constraints.outline {
        free_parts_off_the_outline(problem, footprints, outline, bounds);
    }
    cannot.extend(rows_off_the_board(problem, references, &user_rows, &pinned));
    fix_pinned_and_fixed(problem, &parts, &pinned, &constraints.fixed)?;
    refuse_what_cannot_be_kept(problem, references, constraints, &user_rows, cannot)?;
    // The sides `apply_sides` flipped the parts to, reported as kept
    // constraints.
    report_sides(problem, &parts, constraints);
    Ok(warnings)
}

pub(crate) fn constraint_report(
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
            detail: None,
        })
        .collect()
}

#[cfg(test)]
mod tests;

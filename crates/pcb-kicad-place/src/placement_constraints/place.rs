// Copyright (C) 2026 Toit contributors.

//! `place` (where a part goes, in words or exactly), `device_front`, and
//! `add` (parts the schematic does not have, kept where `place` would
//! keep them).

use super::*;

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
pub fn expand_additions(
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

/// The edge opposite a `device_front` name.
fn opposite(name: &str) -> Result<&'static str, String> {
    match name {
        "left" => Ok("right"),
        "right" => Ok("left"),
        "top" => Ok("bottom"),
        "bottom" => Ok("top"),
        other => Err(format!(
            "unknown device_front {other:?} (left, right, top or bottom)"
        )),
    }
}

/// Applies `place`: parts put somewhere in words become edge and region
/// constraints; parts put at a pose stay there. Returns the parts put at
/// a pose, in entry order.
pub(super) fn apply_places(
    problem: &mut core::Problem,
    parts: &Parts,
    constraints: &KiCadPlacementConstraints,
    bounds: [f64; 4],
    constrained: &mut BTreeSet<usize>,
) -> Result<Vec<usize>, String> {
    // Parts put somewhere in words become edge and region constraints;
    // parts put at a pose stay there.
    let mut pinned = Vec::new();
    let mut used = BTreeMap::new();
    let mut exact: BTreeMap<usize, (usize, bool)> = BTreeMap::new();
    for entry in &constraints.place {
        let index = parts.instance(&entry.part, entry.uuid.as_deref(), &mut used)?;
        match (&entry.at, entry.x, entry.y) {
            (Some(at), None, None) => {
                place_in_words(
                    problem,
                    index,
                    entry,
                    at,
                    constraints.device_front.as_deref(),
                    bounds,
                )?;
                constrained.insert(index);
            }
            (None, Some(x), Some(y)) => {
                let pose = exact_pose(entry, [x, y], bounds, problem.poses[index].angle)?;
                let explicit = entry.angle.is_some();
                if let Some((kept_index, previous_explicit)) = exact.get_mut(&index) {
                    let previous = problem.poses[index];
                    let turn = (previous.angle - pose.angle).abs();
                    if distance_squared(previous.position, pose.position) > 1.0e-12
                        || (*previous_explicit && explicit && turn.min(360.0 - turn) > 1.0e-6)
                    {
                        let instance = parts
                            .uuids
                            .get(index)
                            .and_then(Option::as_ref)
                            .map_or_else(String::new, |uuid| format!(" ({uuid})"));
                        return Err(format!(
                            "conflicting place entries for {}{instance}: ({:.6}, {:.6}) mm at {:.6} degrees and ({:.6}, {:.6}) mm at {:.6} degrees. Give this footprint one exact pose. For repeated references, identify each footprint by uuid.",
                            entry.part,
                            previous.position[0],
                            previous.position[1],
                            previous.angle,
                            pose.position[0],
                            pose.position[1],
                            pose.angle,
                        ));
                    }
                    // A second entry can supply an angle the first omitted.
                    *previous_explicit |= explicit;
                    problem.constraints.kept[*kept_index].angles[0] = pose.angle;
                } else {
                    exact.insert(index, (problem.constraints.kept.len(), explicit));
                    problem.constraints.kept.push(core::constraints::Kept {
                        kind: "place",
                        part: index,
                        side: None,
                        angles: vec![pose.angle],
                        position: Some(pose.position),
                    });
                    pinned.push(index);
                }
                problem.poses[index] = pose;
            }
            _ => {
                return Err(format!(
                    "place {}: give either `at` or both `x` and `y`",
                    entry.part
                ));
            }
        }
    }
    Ok(pinned)
}

/// A part put somewhere in words (`at`): an edge, a corner (two edges), or
/// a part of the outline's box as a region.
fn place_in_words(
    problem: &mut core::Problem,
    index: usize,
    entry: &KiCadPlaceConstraint,
    at: &str,
    device_front: Option<&str>,
    bounds: [f64; 4],
) -> Result<(), String> {
    let word = match at {
        "front" | "rear" => {
            let front = device_front
                .ok_or_else(|| format!("place {}: {at:?} needs device_front", entry.part))?;
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
    Ok(())
}

/// Coordinates and angle of an exact placement in KiCad's frame.
fn exact_pose(
    entry: &KiCadPlaceConstraint,
    [x, y]: [f64; 2],
    bounds: [f64; 4],
    current_angle: f64,
) -> Result<core::Pose, String> {
    let offset = match entry.origin.as_deref() {
        None | Some("board") => [bounds[0], bounds[1]],
        Some("absolute") => [0.0, 0.0],
        Some(other) => {
            return Err(format!(
                "unknown place origin {other:?} (board or absolute)"
            ));
        }
    };
    let angle = entry.angle.unwrap_or(current_angle).rem_euclid(360.0);
    Ok(core::Pose {
        position: [offset[0] + x, offset[1] + y],
        angle,
    })
}

// Copyright (C) 2026 Toit contributors.

//! `constraints.json`: placement intent from the user (often an agent).
//!
//! ```json
//! {"version": 1,
//!  "fixed": ["J1", "H*"],
//!  "rotation": [{"part": "U1", "angle": 90}],
//!  "edge": [{"part": "J4", "edge": "left", "flush": true}],
//!  "region": [{"parts": ["U5", "C1?"], "x": [0, 20], "y": [0, 15], "origin": "board"}],
//!  "near": [{"part": "C1", "pin_of": "U1:48", "max_mm": 3},
//!           {"part": "U2", "part_of": "J4", "max_mm": 10}],
//!  "relative": [{"part": "J4", "below": "U3", "max_gap_mm": 3}]}
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
    #[serde(default)]
    pub rotation: Vec<KiCadRotationConstraint>,
    #[serde(default)]
    pub edge: Vec<KiCadEdgeConstraint>,
    #[serde(default)]
    pub region: Vec<KiCadRegionConstraint>,
    #[serde(default)]
    pub near: Vec<KiCadNearConstraint>,
    #[serde(default)]
    pub relative: Vec<KiCadRelativeConstraint>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadOutlineConstraint {
    pub width: f64,
    pub height: f64,
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
    /// `left`, `right`, `top` or `bottom` of the board outline's bounding box.
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

/// Replaces the board's Edge.Cuts graphics by the constraint's rectangle.
pub(super) fn apply_outline(pcb: &mut Expr, outline: &KiCadOutlineConstraint) -> Result<(), String> {
    if !(outline.width > 0.0 && outline.height > 0.0) {
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
                x.unwrap_or(center[0] - outline.width / 2.0),
                y.unwrap_or(center[1] - outline.height / 2.0),
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
        corner[0] + outline.width,
        corner[1] + outline.height
    ))?;
    items.push(rectangle);
    Ok(())
}

fn edge(name: &str) -> Result<Edge, String> {
    match name {
        "left" => Ok(Edge::Left),
        "right" => Ok(Edge::Right),
        "top" => Ok(Edge::Top),
        "bottom" => Ok(Edge::Bottom),
        other => Err(format!("unknown edge {other:?} (left, right, top or bottom)")),
    }
}

/// Applies `constraints` to a lowered problem. `pads[i]` maps pad names of
/// footprint `i` to their local offsets. Returns warnings for parts of the
/// constraints that are accepted but not acted on.
pub(super) fn apply_constraints(
    problem: &mut core::Problem,
    references: &[String],
    pads: &[BTreeMap<String, [f64; 2]>],
    keepouts: &[Option<[f64; 4]>],
    mouths: &[Option<[f64; 2]>],
    constraints: &KiCadPlacementConstraints,
    weight: f64,
) -> Result<Vec<String>, String> {
    let footprints = pads.len();
    let find = |reference: &str| -> Result<usize, String> {
        references[..footprints]
            .iter()
            .position(|candidate| candidate == reference)
            .ok_or_else(|| format!("constraint names unknown part {reference:?}"))
    };
    let matching = |pattern: &str| -> Vec<usize> {
        (0..footprints)
            .filter(|index| glob_matches(pattern, &references[*index]))
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
        };
        let problem = core::Problem {
            outline: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
            components: vec![part(true), part(false), part(false)],
            net_weights: Vec::new(),
            poses: vec![core::Pose { position: [10.0, 10.0], angle: 0.0 }; 3],
            spacing: 0.2,
            grid: 0.1,
            edge_margin: 0.5,
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
            apply_constraints(&mut problem, &references, &pads, &[None, None, None], &[None, None, None], &constraints, 10.0)?;
        Ok((problem, warnings))
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
    fn fixed_wins_over_a_constraint_and_mistakes_are_errors() {
        let (problem, _) = apply(r#"{"version": 1, "fixed": ["C1"], "near": [{"part": "C1", "part_of": "C2", "max_mm": 1}]}"#).unwrap();
        assert!(problem.components[1].fixed);
        for bad in [
            r#"{"version": 1, "near": [{"part": "R9", "part_of": "C2", "max_mm": 1}]}"#,
            r#"{"version": 1, "near": [{"part": "C1", "pin_of": "J1:7", "max_mm": 1}]}"#,
            r#"{"version": 1, "near": [{"part": "C1", "max_mm": 1}]}"#,
            r#"{"version": 1, "edge": [{"part": "C1", "edge": "north"}]}"#,
            r#"{"version": 1, "relative": [{"part": "C1", "below": "C2", "above": "J1"}]}"#,
            r#"{"version": 1, "fixed": ["Q*"]}"#,
            r#"{"version": 1, "edge": [{"part": "C1", "edge": "top", "overhang": {"edge": "top"}}]}"#,
            r#"{"version": 1, "colour": "green"}"#,
        ] {
            assert!(apply(bad).is_err(), "{bad} was accepted");
        }
    }
}

// Copyright (C) 2026 Toit contributors.

//! `region`: parts inside a box of the board.

use super::*;

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

/// Applies `region`: every part an entry's globs match stays inside its
/// box. A box too small for its part is said with the others no placement
/// can keep (`cannot`).
pub(super) fn apply_regions(
    problem: &mut core::Problem,
    parts: &Parts,
    entries: &[KiCadRegionConstraint],
    bounds: [f64; 4],
    constrained: &mut BTreeSet<usize>,
    cannot: &mut Vec<String>,
) -> Result<(), String> {
    for entry in entries {
        let region = region_box(entry, bounds)?;
        let mut any = false;
        for pattern in &entry.parts {
            for index in parts.matching(pattern) {
                if let Err(error) =
                    region_holds(&problem.components[index], &parts.references[index], region)
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
    Ok(())
}

/// A region's box on the board: its `x` and `y` ranges from its origin,
/// or the part of the outline's box (`bounds`) its `at` names.
fn region_box(entry: &KiCadRegionConstraint, bounds: [f64; 4]) -> Result<[f64; 4], String> {
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
    Ok(region)
}

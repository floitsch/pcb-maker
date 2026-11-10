// Copyright (C) 2026 Toit contributors.

//! `edge`: a part at an edge of the board, with `overhang` and
//! `opening_outwards`.

use super::*;

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

/// The edge a constraint names.
pub(super) fn edge(name: &str) -> Result<Edge, String> {
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

/// Applies `edge`: each named part within its reach of an edge, its
/// keepout pointed off the board where it overhangs, turned to open
/// outwards where asked.
pub(super) fn apply_edges(
    problem: &mut core::Problem,
    parts: &Parts,
    entries: &[KiCadEdgeConstraint],
    constrained: &mut BTreeSet<usize>,
) -> Result<(), String> {
    let mut used = BTreeMap::new();
    for entry in entries {
        let index = parts.instance(&entry.part, entry.uuid.as_deref(), &mut used)?;
        let reach = entry.max_mm.unwrap_or(if entry.flush { 0.0 } else { 1.0 });
        problem
            .constraints
            .edges
            .push((index, edge(&entry.edge)?, reach.max(0.0) + 1.0e-3));
        if let Some(overhang) = &entry.overhang {
            apply_overhang(problem, parts, index, entry, overhang)?;
        }
        if entry.opening_outwards == Some(true) {
            apply_opening_outwards(problem, parts, index, entry)?;
        }
        constrained.insert(index);
    }
    Ok(())
}

/// An edge entry's `overhang`: the part may take only the orientations
/// that point its footprint's keepout at the edge (its own `edge`, or the
/// entry's), and that keepout may hang over it.
fn apply_overhang(
    problem: &mut core::Problem,
    parts: &Parts,
    index: usize,
    entry: &KiCadEdgeConstraint,
    overhang: &serde_json::Value,
) -> Result<(), String> {
    let side = match overhang.get("edge").and_then(|edge| edge.as_str()) {
        Some(name) => edge(name)?,
        None => edge(&entry.edge)?,
    };
    let local = parts.keepouts[index].ok_or_else(|| {
        // Trial 6's laptop power: KiCad's ESP32-S2-MINI-1 footprint
        // draws its antenna without a keepout; the cookbook's
        // overhang was refused with no way on.
        format!(
            "edge constraint for {}: overhang needs a keepout zone in the footprint (the part that sticks out), and this footprint has none: drop the overhang and keep the antenna end at the edge with the edge entry alone (\"flush\": true) and a rotation that turns that end to it (\"rotation\": [{{\"part\": \"{}\", \"angle\": ...}}]), or draw the keepout in the footprint",
            entry.part, entry.part
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
    Ok(())
}

/// An edge entry's `opening_outwards`: the part may take only the
/// orientations that open it towards its edge, kept by construction.
fn apply_opening_outwards(
    problem: &mut core::Problem,
    parts: &Parts,
    index: usize,
    entry: &KiCadEdgeConstraint,
) -> Result<(), String> {
    let side = edge(&entry.edge)?;
    let mouth = parts.mouths[index].ok_or_else(|| {
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
        position: None,
    });
    Ok(())
}

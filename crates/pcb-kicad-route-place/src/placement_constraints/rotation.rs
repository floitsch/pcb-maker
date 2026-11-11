// Copyright (C) 2026 Toit contributors.

//! `rotation`: the angles a part may take.

use super::*;

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

/// Applies `rotation`: each named part may take only the angles given, and
/// turns about its body centre into the first unless it has one already.
pub(super) fn apply_rotations(
    problem: &mut core::Problem,
    parts: &Parts,
    rotations: &[KiCadRotationConstraint],
    constrained: &mut BTreeSet<usize>,
) -> Result<(), String> {
    let mut used = BTreeMap::new();
    for rotation in rotations {
        let index = parts.instance(&rotation.part, rotation.uuid.as_deref(), &mut used)?;
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
            position: None,
        });
        constrained.insert(index);
    }
    Ok(())
}

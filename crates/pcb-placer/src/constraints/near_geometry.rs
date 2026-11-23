// Copyright (C) 2026 Florian Loitsch.

//! A diagnostic bound at the reported target pose, ignoring all other parts.

use super::*;

#[derive(Clone, Debug)]
pub struct NearGeometry {
    pub minimum: f64,
    pub distance: f64,
    pub limit: f64,
    pub faces_without_room: Vec<&'static str>,
}

const SLACK: f64 = 1.0e-6;

/// A necessary distance for a same-side rectangular source and target.
/// Consider every allowed source angle and every separating face of the
/// target, enclosing the true outline in its box. Omit routing halos,
/// other components, pad clearance, grid snapping and regions. A part
/// may fit in this calculation and still have no legal place there.
pub fn near_geometry_at(
    problem: &Problem,
    poses: &[Pose],
    relation: &Relation,
    body_spacing: f64,
) -> Option<NearGeometry> {
    let (source, source_pad, anchor, limit) = match relation {
        Relation::Near {
            part,
            anchor: anchor @ Anchor::Point(_, _),
            max,
        } => (*part, None, *anchor, *max),
        Relation::PadNear {
            source: Anchor::Point(part, local),
            anchor: anchor @ Anchor::Point(_, _),
            max: Some(max),
            ..
        } => (*part, Some(*local), *anchor, *max),
        _ => return None,
    };
    let target = anchor.part();
    let constraints = &problem.constraints;
    let (own, other) = (&problem.components[source], &problem.components[target]);
    let ordinary_rectangle = |part: &crate::Component| {
        !part.round
            && !part.copper_only
            && part.hollow.is_empty()
            && part.courtyards.is_empty()
            && part.courtless.is_empty()
            && part
                .body_size
                .iter()
                .all(|size| size.is_finite() && *size > 0.0)
    };
    let cardinal = |angle: f64| (angle / 90.0 - (angle / 90.0).round()).abs() < 1.0e-9;
    if source == target
        || own.fixed
        || !own.side.collides(other.side)
        || !ordinary_rectangle(own)
        || !ordinary_rectangle(other)
        || !cardinal(poses[source].angle)
        || !cardinal(poses[target].angle)
        || own.angle_options.is_empty()
        || !own.angle_options.iter().copied().all(cardinal)
        || problem.pieces.len() > 1
        || !constraints.frames.keepouts.is_empty()
        || constraints.pad_clearance
        || constraints.courtyard_spacing
        || constraints.overhang(source).is_some()
        || constraints.edges.iter().any(|(part, _, _)| *part == source)
        || constraints.followers.iter().any(|member| {
            [source, target].contains(&member.part) || [source, target].contains(&member.leader)
        })
        || constraints.lies_on(source, target)
        || !body_spacing.is_finite()
        || body_spacing < 0.0
    {
        return None;
    }
    let (center, half) = body(problem, target, poses[target]);
    let pin = anchor_point(problem, poses, anchor).0;
    let (target_min, target_max): ([f64; 2], [f64; 2]) = (
        std::array::from_fn(|axis| center[axis] - half[axis]),
        std::array::from_fn(|axis| center[axis] + half[axis]),
    );
    if (0..2).any(|axis| pin[axis] < target_min[axis] || pin[axis] > target_max[axis]) {
        return None;
    }
    let distance = match source_pad {
        Some(local) => pad_near_distance(problem, poses, Anchor::Point(source, local), anchor),
        None => box_gap(body(problem, source, poses[source]), (pin, [0.0; 2])),
    };
    if near_limit_kept(distance, limit) {
        return None;
    }
    let outline = problem.bounds();
    let mut floor = f64::INFINITY;
    let mut available = [false; 4];
    let names = ["left", "right", "top", "bottom"];
    for &angle in &own.angle_options {
        let half = own.half_extent(angle);
        let pad_from_center = source_pad.map(|local| {
            let offset = own.offset(local, angle);
            let body_offset = own.offset(own.body_center, angle);
            [offset[0] - body_offset[0], offset[1] - body_offset[1]]
        });
        for (face, can_fit) in available.iter_mut().enumerate() {
            let axis = face / 2;
            let mut low = std::array::from_fn::<_, 2, _>(|i| outline[i] + half[i] - SLACK);
            let mut high = std::array::from_fn::<_, 2, _>(|i| outline[i + 2] - half[i] + SLACK);
            if face % 2 == 0 {
                high[axis] = high[axis].min(target_min[axis] - body_spacing - half[axis] + SLACK);
            } else {
                low[axis] = low[axis].max(target_max[axis] + body_spacing + half[axis] - SLACK);
            }
            if (0..2).any(|i| low[i] > high[i]) {
                continue;
            }
            *can_fit = true;
            let delta = |i: usize| {
                let (lo, hi) = if let Some(offset) = pad_from_center {
                    (low[i] + offset[i], high[i] + offset[i])
                } else {
                    (low[i] - half[i], high[i] + half[i])
                };
                (lo - pin[i]).max(pin[i] - hi).max(0.0)
            };
            floor = floor.min(delta(0).hypot(delta(1)));
        }
    }
    (floor.is_finite() && !near_limit_kept(floor, limit) && floor <= distance + SLACK).then(|| {
        NearGeometry {
            minimum: floor,
            distance,
            limit,
            faces_without_room: names
                .into_iter()
                .zip(available)
                .filter_map(|(name, fits)| (!fits).then_some(name))
                .collect(),
        }
    })
}

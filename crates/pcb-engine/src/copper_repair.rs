// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use pcb_core::{Bounds, Mobility, ParticleRole, Vec2};

use crate::{BodyInit, MobilityMask, ParticleInit, World};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopperPolylineMobility {
    Fixed,
    Interior,
    All,
}

#[derive(Clone, Debug)]
pub struct CopperPolylineInput {
    pub id: String,
    pub points: Vec<Vec2>,
    pub width: f32,
    pub tension_weight: f32,
    pub mobility: CopperPolylineMobility,
}

#[derive(Clone, Debug)]
pub struct CopperSeparationPair {
    pub first: String,
    pub second: String,
    pub clearance: f32,
}

#[derive(Clone, Debug)]
pub struct CopperSegmentBodyPair {
    pub polyline: String,
    pub body: String,
    pub clearance: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum CopperBodySeparationMinimum {
    Clearance(f32),
    PreserveInitial,
}

#[derive(Clone, Debug)]
pub struct CopperBodySeparationPair {
    pub first: String,
    pub second: String,
    pub minimum: CopperBodySeparationMinimum,
}

#[derive(Clone, Debug)]
pub struct CopperPointRef {
    pub polyline: String,
    pub point_index: usize,
}

#[derive(Clone, Debug)]
pub struct CopperSharedPointInput {
    pub id: String,
    pub members: Vec<CopperPointRef>,
}

#[derive(Clone, Debug)]
pub struct CopperBodyInput {
    pub id: String,
    pub position: Vec2,
    pub angle_radians: f32,
    pub size: Vec2,
    pub mobility: Mobility,
}

#[derive(Clone, Debug)]
pub struct CopperAttachmentInput {
    pub polyline: String,
    pub point_index: usize,
    pub body: String,
    pub local_position: Vec2,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CopperBodyPose {
    pub id: String,
    pub position: Vec2,
    pub angle_radians: f32,
}

#[derive(Clone, Debug)]
pub struct CopperRepairRequest {
    pub bounds: Bounds,
    pub bodies: Vec<CopperBodyInput>,
    pub polylines: Vec<CopperPolylineInput>,
    pub shared_points: Vec<CopperSharedPointInput>,
    pub attachments: Vec<CopperAttachmentInput>,
    pub separations: Vec<CopperSeparationPair>,
    pub body_separations: Vec<CopperSegmentBodyPair>,
    pub body_body_separations: Vec<CopperBodySeparationPair>,
    pub maximum_segment_pairs: usize,
}

#[derive(Clone, Debug)]
pub struct CompiledCopperRepair {
    pub world: World,
    polylines: Vec<(String, Vec<u32>)>,
    bodies: Vec<(String, u32)>,
    pub segment_pairs: usize,
    pub segment_body_pairs: usize,
    pub body_body_pairs: usize,
    pub shared_point_groups: usize,
    pub shared_point_members: usize,
    pub mixed_mobility_shared_point_groups: usize,
    pub tension_edges: usize,
    pub deduplicated_tension_edges: usize,
    pub deduplicated_segment_pairs: usize,
    pub deduplicated_segment_body_pairs: usize,
    pub deduplicated_body_body_pairs: usize,
}

fn canonical_segment(segment: [u32; 2]) -> (u32, u32) {
    if segment[0] <= segment[1] {
        (segment[0], segment[1])
    } else {
        (segment[1], segment[0])
    }
}

impl CompiledCopperRepair {
    pub fn polylines(&self) -> Vec<(String, Vec<Vec2>)> {
        self.polylines
            .iter()
            .map(|(id, particles)| {
                (
                    id.clone(),
                    particles
                        .iter()
                        .map(|particle| self.world.particles.position(*particle as usize))
                        .collect(),
                )
            })
            .collect()
    }

    pub fn body_poses(&self) -> Vec<CopperBodyPose> {
        self.bodies
            .iter()
            .map(|(id, body)| CopperBodyPose {
                id: id.clone(),
                position: self.world.bodies.position(*body as usize),
                angle_radians: self.world.bodies.angle_radians[*body as usize],
            })
            .collect()
    }
}

/// Compile a small, explicit copper-correction problem. This contract is
/// independent of route-family schemas: callers name polylines and pairs,
/// while the engine sees only flat particle and segment-constraint buffers.
pub fn compile_copper_repair(
    request: &CopperRepairRequest,
) -> Result<CompiledCopperRepair, String> {
    if request.maximum_segment_pairs == 0 {
        return Err("copper repair maximum_segment_pairs must be positive".into());
    }
    let mut world = World::new(request.bounds);
    let mut next_stable_id = 0_u32;
    let mut body_lookup = BTreeMap::<&str, u32>::new();
    let mut bodies = Vec::new();
    for body in &request.bodies {
        if body.id.is_empty() || body_lookup.contains_key(body.id.as_str()) {
            return Err(format!(
                "invalid or duplicate copper repair body {}",
                body.id
            ));
        }
        if !body.position.x.is_finite()
            || !body.position.y.is_finite()
            || !body.angle_radians.is_finite()
            || !body.size.x.is_finite()
            || !body.size.y.is_finite()
            || body.size.x <= 0.0
            || body.size.y <= 0.0
            || body.mobility.inverse_mass < 0.0
            || body.mobility.rotation_mobility < 0.0
        {
            return Err(format!("copper repair body {} is invalid", body.id));
        }
        let mobility =
            MobilityMask::from_axes(body.mobility.translate_x, body.mobility.translate_y);
        let inverse_inertia = if body.mobility.rotate {
            body.mobility.rotation_mobility * 12.0
                / (body.size.x * body.size.x + body.size.y * body.size.y)
        } else {
            0.0
        };
        let handle = world.bodies.push(BodyInit {
            position: body.position,
            angle_radians: body.angle_radians,
            half_size: body.size * 0.5,
            inverse_mass: body.mobility.inverse_mass,
            inverse_inertia,
            mobility,
            rotate: body.mobility.rotate,
            stable_id: next_stable_id,
            label: body.id.clone(),
        });
        next_stable_id += 1;
        body_lookup.insert(body.id.as_str(), handle);
        bodies.push((body.id.clone(), handle));
    }
    let mut polyline_indices = BTreeMap::<&str, usize>::new();
    for (index, polyline) in request.polylines.iter().enumerate() {
        if polyline.id.is_empty()
            || polyline_indices
                .insert(polyline.id.as_str(), index)
                .is_some()
        {
            return Err(format!(
                "invalid or duplicate copper repair polyline {}",
                polyline.id
            ));
        }
    }
    let point_is_movable =
        |polyline: &CopperPolylineInput, point_index: usize| match polyline.mobility {
            CopperPolylineMobility::Fixed => false,
            CopperPolylineMobility::Interior => {
                point_index != 0 && point_index + 1 != polyline.points.len()
            }
            CopperPolylineMobility::All => true,
        };
    let mut shared_ids = BTreeSet::new();
    let mut shared_group_by_member = BTreeMap::<(usize, usize), usize>::new();
    let mut shared_group_has_movable = vec![false; request.shared_points.len()];
    let mut shared_group_has_fixed = vec![false; request.shared_points.len()];
    let mut shared_point_members = 0;
    for (group_index, group) in request.shared_points.iter().enumerate() {
        if group.id.is_empty() || !shared_ids.insert(group.id.as_str()) {
            return Err(format!(
                "invalid or duplicate copper shared-point group {}",
                group.id
            ));
        }
        if group.members.len() < 2 {
            return Err(format!(
                "copper shared-point group {} has fewer than two members",
                group.id
            ));
        }
        let mut expected = None::<Vec2>;
        for member in &group.members {
            let &polyline_index =
                polyline_indices
                    .get(member.polyline.as_str())
                    .ok_or_else(|| {
                        format!(
                            "unknown shared-point polyline {} in group {}",
                            member.polyline, group.id
                        )
                    })?;
            let polyline = &request.polylines[polyline_index];
            let &point = polyline.points.get(member.point_index).ok_or_else(|| {
                format!(
                    "shared point {} is outside polyline {}",
                    member.point_index, member.polyline
                )
            })?;
            let movable = point_is_movable(polyline, member.point_index);
            if let Some(expected_point) = expected {
                if point.distance(expected_point) > 1.0e-6 {
                    return Err(format!(
                        "copper shared-point group {} has inconsistent positions",
                        group.id
                    ));
                }
            } else {
                expected = Some(point);
            }
            shared_group_has_movable[group_index] |= movable;
            shared_group_has_fixed[group_index] |= !movable;
            if shared_group_by_member
                .insert((polyline_index, member.point_index), group_index)
                .is_some()
            {
                return Err(format!(
                    "copper point {}:{} belongs to multiple shared-point groups",
                    member.polyline, member.point_index
                ));
            }
            shared_point_members += 1;
        }
    }
    let mut lookup = BTreeMap::<&str, (usize, Vec<u32>)>::new();
    let mut polylines = Vec::new();
    let mut shared_group_handles = vec![None; request.shared_points.len()];
    let mut tension_edge_properties = BTreeMap::<(u32, u32), (u32, u32)>::new();
    let mut deduplicated_tension_edges = 0;
    for (polyline_index, polyline) in request.polylines.iter().enumerate() {
        if polyline.id.is_empty() {
            return Err("copper repair polyline ID must not be empty".into());
        }
        if polyline.points.len() < 2 {
            return Err(format!(
                "copper repair polyline {} has fewer than two points",
                polyline.id
            ));
        }
        if !polyline.width.is_finite() || polyline.width <= 0.0 {
            return Err(format!(
                "copper repair polyline {} has invalid width",
                polyline.id
            ));
        }
        if !polyline.tension_weight.is_finite() || polyline.tension_weight < 0.0 {
            return Err(format!(
                "copper repair polyline {} has invalid tension weight",
                polyline.id
            ));
        }
        let mut particles = Vec::with_capacity(polyline.points.len());
        for (point_index, &point) in polyline.points.iter().enumerate() {
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(format!(
                    "copper repair polyline {} has a non-finite point",
                    polyline.id
                ));
            }
            let movable = point_is_movable(polyline, point_index);
            let shared_group = shared_group_by_member.get(&(polyline_index, point_index));
            let particle = if let Some(&group_index) = shared_group {
                let movable = !shared_group_has_fixed[group_index];
                if let Some(handle) = shared_group_handles[group_index] {
                    handle
                } else {
                    let handle = world.particles.push(ParticleInit {
                        position: point,
                        field_weight: 0.0,
                        inverse_mass: if movable { 1.0 } else { 0.0 },
                        mobility: if movable {
                            MobilityMask::XY
                        } else {
                            MobilityMask::FIXED
                        },
                        role: ParticleRole::Trace,
                        stable_id: next_stable_id,
                        label: format!("shared:{}", request.shared_points[group_index].id),
                    });
                    next_stable_id += 1;
                    shared_group_handles[group_index] = Some(handle);
                    handle
                }
            } else {
                let handle = world.particles.push(ParticleInit {
                    position: point,
                    field_weight: 0.0,
                    inverse_mass: if movable { 1.0 } else { 0.0 },
                    mobility: if movable {
                        MobilityMask::XY
                    } else {
                        MobilityMask::FIXED
                    },
                    role: ParticleRole::Trace,
                    stable_id: next_stable_id,
                    label: format!("{}:point:{point_index}", polyline.id),
                });
                next_stable_id += 1;
                handle
            };
            particles.push(particle);
        }
        lookup.insert(polyline.id.as_str(), (polyline_index, particles.clone()));
        if polyline.tension_weight > 0.0 {
            for pair in particles.windows(2) {
                let edge = canonical_segment([pair[0], pair[1]]);
                let properties = (polyline.tension_weight.to_bits(), polyline.width.to_bits());
                if let Some(existing) = tension_edge_properties.get(&edge) {
                    if *existing != properties {
                        return Err(format!(
                            "shared copper edge in {} has inconsistent width or tension",
                            polyline.id
                        ));
                    }
                    deduplicated_tension_edges += 1;
                } else {
                    tension_edge_properties.insert(edge, properties);
                    world.trace_tension.push(
                        pair[0],
                        pair[1],
                        polyline.tension_weight,
                        format!("copper:{}:tension", polyline.id),
                    );
                }
            }
        }
        polylines.push((polyline.id.clone(), particles));
    }

    let mut attached_points = BTreeSet::new();
    for attachment in &request.attachments {
        let (_, particles) = lookup
            .get(attachment.polyline.as_str())
            .ok_or_else(|| format!("unknown attached polyline {}", attachment.polyline))?;
        let &particle = particles.get(attachment.point_index).ok_or_else(|| {
            format!(
                "attachment point {} is outside polyline {}",
                attachment.point_index, attachment.polyline
            )
        })?;
        let &body = body_lookup
            .get(attachment.body.as_str())
            .ok_or_else(|| format!("unknown attachment body {}", attachment.body))?;
        if !attachment.local_position.x.is_finite() || !attachment.local_position.y.is_finite() {
            return Err("copper repair attachment has a non-finite local position".into());
        }
        if !attached_points.insert(particle) {
            return Err(format!(
                "duplicate physical attachment at {}:{}",
                attachment.polyline, attachment.point_index
            ));
        }
        world.attachments.push(
            particle,
            body,
            attachment.local_position,
            format!(
                "copper-endpoint:{}:{}->{}",
                attachment.polyline, attachment.point_index, attachment.body
            ),
        );
    }

    let mut unique_pairs = BTreeSet::new();
    let mut compiled_segment_keys = BTreeSet::new();
    let mut segment_pairs = 0_usize;
    let mut segment_body_pairs = 0_usize;
    let mut deduplicated_segment_pairs = 0_usize;
    let mut deduplicated_segment_body_pairs = 0_usize;
    for separation in &request.separations {
        if separation.first == separation.second {
            return Err(format!(
                "copper repair separation repeats polyline {}",
                separation.first
            ));
        }
        if !separation.clearance.is_finite() || separation.clearance < 0.0 {
            return Err("copper repair separation has invalid clearance".into());
        }
        let (first_index, first_particles) = lookup
            .get(separation.first.as_str())
            .ok_or_else(|| format!("unknown copper repair polyline {}", separation.first))?;
        let (second_index, second_particles) = lookup
            .get(separation.second.as_str())
            .ok_or_else(|| format!("unknown copper repair polyline {}", separation.second))?;
        let (first_index, second_index) = (*first_index, *second_index);
        let key = if first_index < second_index {
            (first_index, second_index)
        } else {
            (second_index, first_index)
        };
        if !unique_pairs.insert(key) {
            return Err(format!(
                "duplicate copper repair separation {} / {}",
                separation.first, separation.second
            ));
        }
        let required =
            (request.polylines[first_index].width + request.polylines[second_index].width) * 0.5
                + separation.clearance;
        for (first_segment, first) in first_particles.windows(2).enumerate() {
            for (second_segment, second) in second_particles.windows(2).enumerate() {
                let first_key = canonical_segment([first[0], first[1]]);
                let second_key = canonical_segment([second[0], second[1]]);
                if first_key == second_key {
                    return Err(format!(
                        "copper separation {} / {} compares one shared physical segment to itself",
                        separation.first, separation.second
                    ));
                }
                let (first_key, second_key) = if first_key <= second_key {
                    (first_key, second_key)
                } else {
                    (second_key, first_key)
                };
                if !compiled_segment_keys.insert((first_key, second_key, required.to_bits())) {
                    deduplicated_segment_pairs += 1;
                    continue;
                }
                segment_pairs = segment_pairs
                    .checked_add(1)
                    .ok_or_else(|| "copper repair segment-pair count overflowed".to_string())?;
                if segment_pairs > request.maximum_segment_pairs {
                    return Err(format!(
                        "copper repair exceeds maximum_segment_pairs {}",
                        request.maximum_segment_pairs
                    ));
                }
                world.segment_clearance.push(
                    [first[0], first[1]],
                    [second[0], second[1]],
                    required,
                    format!(
                        "copper:{}:{first_segment}/{}:{second_segment}",
                        separation.first, separation.second
                    ),
                );
            }
        }
    }
    let mut unique_body_pairs = BTreeSet::new();
    let mut compiled_segment_body_keys = BTreeSet::new();
    for separation in &request.body_separations {
        if !separation.clearance.is_finite() || separation.clearance < 0.0 {
            return Err("copper repair body separation has invalid clearance".into());
        }
        let (polyline_index, particles) = lookup
            .get(separation.polyline.as_str())
            .ok_or_else(|| format!("unknown copper repair polyline {}", separation.polyline))?;
        let polyline_index = *polyline_index;
        let &body = body_lookup
            .get(separation.body.as_str())
            .ok_or_else(|| format!("unknown copper repair body {}", separation.body))?;
        if !unique_body_pairs.insert((polyline_index, body)) {
            return Err(format!(
                "duplicate copper repair body separation {} / {}",
                separation.polyline, separation.body
            ));
        }
        let required = request.polylines[polyline_index].width * 0.5 + separation.clearance;
        for (segment, points) in particles.windows(2).enumerate() {
            let segment_key = canonical_segment([points[0], points[1]]);
            if !compiled_segment_body_keys.insert((segment_key, body, required.to_bits())) {
                deduplicated_segment_body_pairs += 1;
                continue;
            }
            segment_body_pairs = segment_body_pairs
                .checked_add(1)
                .ok_or_else(|| "copper repair segment-body pair count overflowed".to_string())?;
            let total_pairs = segment_pairs
                .checked_add(segment_body_pairs)
                .ok_or_else(|| "copper repair total pair count overflowed".to_string())?;
            if total_pairs > request.maximum_segment_pairs {
                return Err(format!(
                    "copper repair exceeds maximum_segment_pairs {}",
                    request.maximum_segment_pairs
                ));
            }
            world.segment_body_clearance.push(
                [points[0], points[1]],
                body,
                required,
                format!(
                    "copper-body:{}:{segment}/{}",
                    separation.polyline, separation.body
                ),
            );
        }
    }
    let mut compiled_body_body_pairs = BTreeMap::<(u32, u32), usize>::new();
    let mut deduplicated_body_body_pairs = 0_usize;
    let mut body_body_pairs = 0_usize;
    for separation in &request.body_body_separations {
        let &first = body_lookup
            .get(separation.first.as_str())
            .ok_or_else(|| format!("unknown copper repair body {}", separation.first))?;
        let &second = body_lookup
            .get(separation.second.as_str())
            .ok_or_else(|| format!("unknown copper repair body {}", separation.second))?;
        if first == second {
            return Err(format!(
                "copper body separation repeats body {}",
                separation.first
            ));
        }
        let key = if first < second {
            (first, second)
        } else {
            (second, first)
        };
        let minimum = match separation.minimum {
            CopperBodySeparationMinimum::Clearance(clearance) => {
                if !clearance.is_finite() || clearance < 0.0 {
                    return Err("copper repair body/body separation has invalid clearance".into());
                }
                clearance
            }
            CopperBodySeparationMinimum::PreserveInitial => {
                crate::solver::body_body_proximity(&world, first as usize, second as usize).distance
            }
        };
        if let Some(&constraint) = compiled_body_body_pairs.get(&key) {
            world.body_body_clearance.minimum[constraint] =
                world.body_body_clearance.minimum[constraint].max(minimum);
            deduplicated_body_body_pairs += 1;
            continue;
        }
        body_body_pairs = body_body_pairs
            .checked_add(1)
            .ok_or_else(|| "copper repair body/body pair count overflowed".to_string())?;
        let total_pairs = segment_pairs
            .checked_add(segment_body_pairs)
            .and_then(|pairs| pairs.checked_add(body_body_pairs))
            .ok_or_else(|| "copper repair total pair count overflowed".to_string())?;
        if total_pairs > request.maximum_segment_pairs {
            return Err(format!(
                "copper repair exceeds maximum_segment_pairs {}",
                request.maximum_segment_pairs
            ));
        }
        world.body_body_clearance.push(
            first,
            second,
            minimum,
            format!("body-body:{}/{}", separation.first, separation.second),
        );
        compiled_body_body_pairs.insert(key, world.body_body_clearance.len() - 1);
    }
    let tension_edges = world.trace_tension.len();
    Ok(CompiledCopperRepair {
        world,
        polylines,
        bodies,
        segment_pairs,
        segment_body_pairs,
        body_body_pairs,
        shared_point_groups: request.shared_points.len(),
        shared_point_members,
        mixed_mobility_shared_point_groups: shared_group_has_movable
            .iter()
            .zip(&shared_group_has_fixed)
            .filter(|(movable, fixed)| **movable && **fixed)
            .count(),
        tension_edges,
        deduplicated_tension_edges,
        deduplicated_segment_pairs,
        deduplicated_segment_body_pairs,
        deduplicated_body_body_pairs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, CpuReferenceBackend, NoField, SolverConfig};

    #[test]
    fn zero_tension_fixed_obstacles_do_not_add_integration_edges() {
        let compiled = compile_copper_repair(&CopperRepairRequest {
            bounds: Bounds::new(Vec2::ZERO, Vec2::new(10.0, 10.0)),
            bodies: Vec::new(),
            polylines: vec![
                CopperPolylineInput {
                    id: "movable".into(),
                    points: vec![
                        Vec2::new(1.0, 1.0),
                        Vec2::new(5.0, 2.0),
                        Vec2::new(9.0, 1.0),
                    ],
                    width: 0.2,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::Interior,
                },
                CopperPolylineInput {
                    id: "fixed-obstacle".into(),
                    points: vec![
                        Vec2::new(1.0, 8.0),
                        Vec2::new(3.0, 8.0),
                        Vec2::new(5.0, 8.0),
                        Vec2::new(9.0, 8.0),
                    ],
                    width: 0.2,
                    tension_weight: 0.0,
                    mobility: CopperPolylineMobility::Fixed,
                },
            ],
            shared_points: Vec::new(),
            attachments: Vec::new(),
            separations: Vec::new(),
            body_separations: Vec::new(),
            body_body_separations: Vec::new(),
            maximum_segment_pairs: 1,
        })
        .unwrap();
        assert_eq!(compiled.world.trace_tension.len(), 2);
    }

    #[test]
    fn shared_route_points_reuse_particles_edges_and_clearance_rows() {
        let mut compiled = compile_copper_repair(&CopperRepairRequest {
            bounds: Bounds::new(Vec2::ZERO, Vec2::new(8.0, 8.0)),
            bodies: Vec::new(),
            polylines: vec![
                CopperPolylineInput {
                    id: "a".into(),
                    points: vec![
                        Vec2::new(0.5, 1.0),
                        Vec2::new(2.0, 2.0),
                        Vec2::new(4.0, 2.0),
                        Vec2::new(7.0, 1.0),
                    ],
                    width: 0.2,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::Interior,
                },
                CopperPolylineInput {
                    id: "b".into(),
                    points: vec![
                        Vec2::new(0.5, 1.0),
                        Vec2::new(2.0, 2.0),
                        Vec2::new(4.0, 2.0),
                        Vec2::new(7.0, 4.0),
                    ],
                    width: 0.2,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::Interior,
                },
                CopperPolylineInput {
                    id: "obstacle".into(),
                    points: vec![Vec2::new(0.5, 7.0), Vec2::new(7.0, 7.0)],
                    width: 0.2,
                    tension_weight: 0.0,
                    mobility: CopperPolylineMobility::Fixed,
                },
            ],
            shared_points: (0..3)
                .map(|point_index| CopperSharedPointInput {
                    id: format!("trunk-{point_index}"),
                    members: vec![
                        CopperPointRef {
                            polyline: "a".into(),
                            point_index,
                        },
                        CopperPointRef {
                            polyline: "b".into(),
                            point_index,
                        },
                    ],
                })
                .collect(),
            attachments: Vec::new(),
            separations: vec![
                CopperSeparationPair {
                    first: "a".into(),
                    second: "obstacle".into(),
                    clearance: 0.2,
                },
                CopperSeparationPair {
                    first: "b".into(),
                    second: "obstacle".into(),
                    clearance: 0.2,
                },
            ],
            body_separations: Vec::new(),
            body_body_separations: Vec::new(),
            maximum_segment_pairs: 16,
        })
        .unwrap();
        assert_eq!(compiled.world.particles.len(), 7);
        assert_eq!(compiled.shared_point_groups, 3);
        assert_eq!(compiled.shared_point_members, 6);
        assert_eq!(compiled.tension_edges, 4);
        assert_eq!(compiled.deduplicated_tension_edges, 2);
        assert_eq!(compiled.segment_pairs, 4);
        assert_eq!(compiled.deduplicated_segment_pairs, 2);

        let mut backend = CpuReferenceBackend::with_field(NoField);
        backend.step(
            &mut compiled.world,
            &SolverConfig {
                field_strength: 0.0,
                projection_iterations: 1,
                ..SolverConfig::default()
            },
            0,
        );
        let paths = compiled.polylines().into_iter().collect::<BTreeMap<_, _>>();
        assert_eq!(&paths["a"][..3], &paths["b"][..3]);
    }

    #[test]
    fn a_fixed_terminal_anchors_a_shared_interior_route_contact() {
        let mut compiled = compile_copper_repair(&CopperRepairRequest {
            bounds: Bounds::new(Vec2::ZERO, Vec2::new(8.0, 8.0)),
            bodies: Vec::new(),
            polylines: vec![
                CopperPolylineInput {
                    id: "terminal-branch".into(),
                    points: vec![Vec2::new(2.0, 2.0), Vec2::new(6.0, 2.0)],
                    width: 0.2,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::Interior,
                },
                CopperPolylineInput {
                    id: "through-branch".into(),
                    points: vec![
                        Vec2::new(1.0, 6.0),
                        Vec2::new(2.0, 2.0),
                        Vec2::new(7.0, 6.0),
                    ],
                    width: 0.2,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::Interior,
                },
            ],
            shared_points: vec![CopperSharedPointInput {
                id: "terminal-t-contact".into(),
                members: vec![
                    CopperPointRef {
                        polyline: "terminal-branch".into(),
                        point_index: 0,
                    },
                    CopperPointRef {
                        polyline: "through-branch".into(),
                        point_index: 1,
                    },
                ],
            }],
            attachments: Vec::new(),
            separations: Vec::new(),
            body_separations: Vec::new(),
            body_body_separations: Vec::new(),
            maximum_segment_pairs: 1,
        })
        .unwrap();
        assert_eq!(compiled.mixed_mobility_shared_point_groups, 1);

        let mut backend = CpuReferenceBackend::with_field(NoField);
        backend.step(
            &mut compiled.world,
            &SolverConfig {
                field_strength: 0.0,
                trace_tension_strength: 4.0,
                maximum_trace_tension_step: 0.2,
                projection_iterations: 1,
                ..SolverConfig::default()
            },
            0,
        );
        let paths = compiled.polylines().into_iter().collect::<BTreeMap<_, _>>();
        assert_eq!(paths["terminal-branch"][0], Vec2::new(2.0, 2.0));
        assert_eq!(paths["through-branch"][1], Vec2::new(2.0, 2.0));
    }

    #[test]
    fn interior_points_separate_while_endpoints_remain_fixed() {
        let first = vec![
            Vec2::new(1.0, 1.0),
            Vec2::new(5.0, 3.0),
            Vec2::new(9.0, 1.0),
        ];
        let second = vec![
            Vec2::new(1.0, 5.0),
            Vec2::new(5.0, 3.4),
            Vec2::new(9.0, 5.0),
        ];
        let mut compiled = compile_copper_repair(&CopperRepairRequest {
            bounds: Bounds::new(Vec2::new(0.0, 0.0), Vec2::new(10.0, 6.0)),
            bodies: Vec::new(),
            polylines: vec![
                CopperPolylineInput {
                    id: "left".into(),
                    points: first.clone(),
                    width: 0.2,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::Interior,
                },
                CopperPolylineInput {
                    id: "right".into(),
                    points: second.clone(),
                    width: 0.2,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::Interior,
                },
            ],
            shared_points: Vec::new(),
            attachments: Vec::new(),
            separations: vec![CopperSeparationPair {
                first: "left".into(),
                second: "right".into(),
                clearance: 0.5,
            }],
            body_separations: Vec::new(),
            body_body_separations: Vec::new(),
            maximum_segment_pairs: 16,
        })
        .unwrap();
        let mut backend = CpuReferenceBackend::with_field(NoField);
        let frame = backend.step(
            &mut compiled.world,
            &SolverConfig {
                field_strength: 0.0,
                projection_iterations: 16,
                ..SolverConfig::default()
            },
            0,
        );
        let result = compiled.polylines();
        assert_eq!(result[0].1[0], first[0]);
        assert_eq!(result[0].1[2], first[2]);
        assert_eq!(result[1].1[0], second[0]);
        assert_eq!(result[1].1[2], second[2]);
        assert!(result[0].1[1] != first[1] || result[1].1[1] != second[1]);
        assert!(frame.metrics.max_constraint_residual < 1.0e-4);
    }

    #[test]
    fn segment_pair_budget_fails_before_world_expansion() {
        let request = CopperRepairRequest {
            bounds: Bounds::new(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0)),
            bodies: Vec::new(),
            polylines: vec![
                CopperPolylineInput {
                    id: "a".into(),
                    points: vec![Vec2::new(0.0, 0.0); 4],
                    width: 0.2,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::Interior,
                },
                CopperPolylineInput {
                    id: "b".into(),
                    points: vec![Vec2::new(1.0, 1.0); 4],
                    width: 0.2,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::Interior,
                },
            ],
            shared_points: Vec::new(),
            attachments: Vec::new(),
            separations: vec![CopperSeparationPair {
                first: "a".into(),
                second: "b".into(),
                clearance: 0.2,
            }],
            body_separations: Vec::new(),
            body_body_separations: Vec::new(),
            maximum_segment_pairs: 8,
        };
        assert!(
            compile_copper_repair(&request)
                .unwrap_err()
                .contains("maximum_segment_pairs 8")
        );
    }

    #[test]
    fn body_body_clearance_pushes_a_movable_oriented_body() {
        let mut compiled = compile_copper_repair(&CopperRepairRequest {
            bounds: Bounds::new(Vec2::ZERO, Vec2::new(12.0, 8.0)),
            bodies: vec![
                CopperBodyInput {
                    id: "fixed".into(),
                    position: Vec2::new(4.0, 4.0),
                    angle_radians: 0.0,
                    size: Vec2::new(2.0, 2.0),
                    mobility: Mobility::FIXED,
                },
                CopperBodyInput {
                    id: "movable".into(),
                    position: Vec2::new(6.25, 4.0),
                    angle_radians: 0.25,
                    size: Vec2::new(2.0, 1.0),
                    mobility: Mobility {
                        inverse_mass: 1.0,
                        rotation_mobility: 0.0,
                        translate_x: true,
                        translate_y: false,
                        rotate: false,
                    },
                },
            ],
            polylines: Vec::new(),
            shared_points: Vec::new(),
            attachments: Vec::new(),
            separations: Vec::new(),
            body_separations: Vec::new(),
            body_body_separations: vec![CopperBodySeparationPair {
                first: "fixed".into(),
                second: "movable".into(),
                minimum: CopperBodySeparationMinimum::Clearance(0.5),
            }],
            maximum_segment_pairs: 1,
        })
        .unwrap();
        let before = compiled.body_poses()[1].position.x;
        let mut backend = CpuReferenceBackend::with_field(NoField);
        let frame = backend.step(
            &mut compiled.world,
            &SolverConfig {
                field_strength: 0.0,
                projection_iterations: 16,
                ..SolverConfig::default()
            },
            0,
        );
        assert!(compiled.body_poses()[1].position.x > before);
        assert!(frame.metrics.max_constraint_residual < 1.0e-4);
        assert!(frame.constraints.iter().any(|constraint| {
            constraint.family == "body_body_clearance" && constraint.residual < 1.0e-4
        }));
    }

    #[test]
    fn preserve_initial_body_distance_restores_a_consumed_gap() {
        let mut compiled = compile_copper_repair(&CopperRepairRequest {
            bounds: Bounds::new(Vec2::ZERO, Vec2::new(12.0, 8.0)),
            bodies: vec![
                CopperBodyInput {
                    id: "fixed".into(),
                    position: Vec2::new(4.0, 4.0),
                    angle_radians: 0.0,
                    size: Vec2::new(2.0, 2.0),
                    mobility: Mobility::FIXED,
                },
                CopperBodyInput {
                    id: "movable".into(),
                    position: Vec2::new(6.5, 4.0),
                    angle_radians: 0.0,
                    size: Vec2::new(2.0, 2.0),
                    mobility: Mobility {
                        inverse_mass: 1.0,
                        rotation_mobility: 0.0,
                        translate_x: true,
                        translate_y: true,
                        rotate: false,
                    },
                },
            ],
            polylines: Vec::new(),
            shared_points: Vec::new(),
            attachments: Vec::new(),
            separations: Vec::new(),
            body_separations: Vec::new(),
            body_body_separations: vec![CopperBodySeparationPair {
                first: "fixed".into(),
                second: "movable".into(),
                minimum: CopperBodySeparationMinimum::PreserveInitial,
            }],
            maximum_segment_pairs: 1,
        })
        .unwrap();
        let movable = compiled.bodies[1].1 as usize;
        compiled
            .world
            .bodies
            .set_position(movable, Vec2::new(6.1, 4.0));
        let mut backend = CpuReferenceBackend::with_field(NoField);
        let frame = backend.step(
            &mut compiled.world,
            &SolverConfig {
                field_strength: 0.0,
                projection_iterations: 4,
                ..SolverConfig::default()
            },
            0,
        );
        assert!((compiled.body_poses()[1].position.x - 6.5).abs() < 1.0e-4);
        assert!(frame.metrics.max_constraint_residual < 1.0e-4);
    }

    #[test]
    fn body_body_contact_can_rotate_a_body_and_deduplicates_pairs() {
        let initial_angle = 0.35;
        let request = CopperRepairRequest {
            bounds: Bounds::new(Vec2::ZERO, Vec2::new(12.0, 8.0)),
            bodies: vec![
                CopperBodyInput {
                    id: "fixed".into(),
                    position: Vec2::new(6.0, 5.0),
                    angle_radians: 0.0,
                    size: Vec2::new(4.0, 0.5),
                    mobility: Mobility::FIXED,
                },
                CopperBodyInput {
                    id: "rotor".into(),
                    position: Vec2::new(6.0, 3.65),
                    angle_radians: initial_angle,
                    size: Vec2::new(4.0, 0.5),
                    mobility: Mobility {
                        inverse_mass: 0.0,
                        rotation_mobility: 1.0,
                        translate_x: false,
                        translate_y: false,
                        rotate: true,
                    },
                },
            ],
            polylines: Vec::new(),
            shared_points: Vec::new(),
            attachments: Vec::new(),
            separations: Vec::new(),
            body_separations: Vec::new(),
            body_body_separations: vec![
                CopperBodySeparationPair {
                    first: "fixed".into(),
                    second: "rotor".into(),
                    minimum: CopperBodySeparationMinimum::Clearance(0.25),
                },
                CopperBodySeparationPair {
                    first: "rotor".into(),
                    second: "fixed".into(),
                    minimum: CopperBodySeparationMinimum::Clearance(0.5),
                },
            ],
            maximum_segment_pairs: 1,
        };
        let mut compiled = compile_copper_repair(&request).unwrap();
        assert_eq!(compiled.body_body_pairs, 1);
        assert_eq!(compiled.deduplicated_body_body_pairs, 1);
        assert_eq!(compiled.world.body_body_clearance.minimum, vec![0.5]);
        let mut backend = CpuReferenceBackend::with_field(NoField);
        let before = backend.step(
            &mut compiled.world,
            &SolverConfig {
                field_strength: 0.0,
                projection_iterations: 0,
                ..SolverConfig::default()
            },
            0,
        );
        let after = backend.step(
            &mut compiled.world,
            &SolverConfig {
                field_strength: 0.0,
                projection_iterations: 1,
                ..SolverConfig::default()
            },
            1,
        );
        assert!(before.metrics.max_constraint_residual > 0.0);
        assert_ne!(compiled.body_poses()[1].angle_radians, initial_angle);
        assert!(after.metrics.max_constraint_residual < before.metrics.max_constraint_residual);
    }

    #[test]
    fn attached_trace_endpoints_transfer_clearance_force_to_bodies() {
        let mut compiled = compile_copper_repair(&CopperRepairRequest {
            bounds: Bounds::new(Vec2::new(0.0, 0.0), Vec2::new(12.0, 8.0)),
            bodies: vec![
                CopperBodyInput {
                    id: "A".into(),
                    position: Vec2::new(2.0, 3.7),
                    angle_radians: 0.0,
                    size: Vec2::new(0.2, 0.2),
                    mobility: Mobility {
                        inverse_mass: 1.0,
                        rotation_mobility: 0.0,
                        translate_x: false,
                        translate_y: true,
                        rotate: false,
                    },
                },
                CopperBodyInput {
                    id: "B".into(),
                    position: Vec2::new(10.0, 3.7),
                    angle_radians: 0.0,
                    size: Vec2::new(0.2, 0.2),
                    mobility: Mobility {
                        inverse_mass: 1.0,
                        rotation_mobility: 0.0,
                        translate_x: false,
                        translate_y: true,
                        rotate: false,
                    },
                },
            ],
            polylines: vec![
                CopperPolylineInput {
                    id: "lower".into(),
                    points: vec![Vec2::new(2.0, 3.0), Vec2::new(10.0, 3.0)],
                    width: 0.5,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::Fixed,
                },
                CopperPolylineInput {
                    id: "upper".into(),
                    points: vec![Vec2::new(2.0, 3.7), Vec2::new(10.0, 3.7)],
                    width: 0.5,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::All,
                },
            ],
            shared_points: Vec::new(),
            attachments: vec![
                CopperAttachmentInput {
                    polyline: "upper".into(),
                    point_index: 0,
                    body: "A".into(),
                    local_position: Vec2::new(0.0, 0.0),
                },
                CopperAttachmentInput {
                    polyline: "upper".into(),
                    point_index: 1,
                    body: "B".into(),
                    local_position: Vec2::new(0.0, 0.0),
                },
            ],
            separations: vec![CopperSeparationPair {
                first: "lower".into(),
                second: "upper".into(),
                clearance: 0.25,
            }],
            body_separations: Vec::new(),
            body_body_separations: Vec::new(),
            maximum_segment_pairs: 4,
        })
        .unwrap();
        let mut backend = CpuReferenceBackend::with_field(NoField);
        let config = SolverConfig {
            field_strength: 0.0,
            projection_iterations: 32,
            ..SolverConfig::default()
        };
        let mut frame = backend.step(&mut compiled.world, &config, 0);
        for step in 1..4 {
            frame = backend.step(&mut compiled.world, &config, step);
        }
        let poses = compiled.body_poses();
        assert!(poses.iter().all(|pose| pose.position.y > 3.749));
        assert!(frame.metrics.max_constraint_residual < 1.0e-4);
        assert!(
            frame
                .attachments
                .iter()
                .all(|attachment| attachment.residual < 1.0e-4)
        );
    }

    #[test]
    fn oriented_body_contact_redirects_a_trace_pair_correction() {
        let mut compiled = compile_copper_repair(&CopperRepairRequest {
            bounds: Bounds::new(Vec2::new(0.0, 0.0), Vec2::new(12.0, 8.0)),
            bodies: vec![CopperBodyInput {
                id: "blocker".into(),
                position: Vec2::new(6.0, 2.45),
                angle_radians: 0.0,
                size: Vec2::new(4.0, 0.1),
                mobility: Mobility::FIXED,
            }],
            polylines: vec![
                CopperPolylineInput {
                    id: "lower".into(),
                    points: vec![Vec2::new(2.0, 3.0), Vec2::new(10.0, 3.0)],
                    width: 0.5,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::All,
                },
                CopperPolylineInput {
                    id: "upper".into(),
                    points: vec![Vec2::new(2.0, 3.7), Vec2::new(10.0, 3.7)],
                    width: 0.5,
                    tension_weight: 1.0,
                    mobility: CopperPolylineMobility::All,
                },
            ],
            shared_points: Vec::new(),
            attachments: Vec::new(),
            separations: vec![CopperSeparationPair {
                first: "lower".into(),
                second: "upper".into(),
                clearance: 0.25,
            }],
            body_separations: vec![CopperSegmentBodyPair {
                polyline: "lower".into(),
                body: "blocker".into(),
                clearance: 0.251,
            }],
            body_body_separations: Vec::new(),
            maximum_segment_pairs: 4,
        })
        .unwrap();
        assert_eq!(compiled.segment_pairs, 1);
        assert_eq!(compiled.segment_body_pairs, 1);
        let mut backend = CpuReferenceBackend::with_field(NoField);
        let config = SolverConfig {
            field_strength: 0.0,
            projection_iterations: 32,
            ..SolverConfig::default()
        };
        let mut frame = backend.step(&mut compiled.world, &config, 0);
        for step in 1..4 {
            frame = backend.step(&mut compiled.world, &config, step);
        }
        let polylines = compiled.polylines().into_iter().collect::<BTreeMap<_, _>>();
        assert!(polylines["lower"].iter().all(|point| point.y > 3.0008));
        assert!(polylines["upper"].iter().all(|point| point.y > 3.7507));
        assert!(frame.metrics.max_constraint_residual < 1.0e-4);
        assert!(frame.constraints.iter().any(|constraint| {
            constraint.family == "segment_body_clearance" && constraint.residual < 1.0e-4
        }));
    }

    #[test]
    fn fixed_trace_pushes_a_movable_body_through_the_same_contact_row() {
        let mut compiled = compile_copper_repair(&CopperRepairRequest {
            bounds: Bounds::new(Vec2::new(0.0, 0.0), Vec2::new(12.0, 8.0)),
            bodies: vec![CopperBodyInput {
                id: "movable".into(),
                position: Vec2::new(6.0, 2.7),
                angle_radians: 0.0,
                size: Vec2::new(4.0, 0.4),
                mobility: Mobility {
                    inverse_mass: 1.0,
                    rotation_mobility: 0.0,
                    translate_x: false,
                    translate_y: true,
                    rotate: false,
                },
            }],
            polylines: vec![CopperPolylineInput {
                id: "fixed-trace".into(),
                points: vec![Vec2::new(2.0, 3.0), Vec2::new(10.0, 3.0)],
                width: 0.5,
                tension_weight: 1.0,
                mobility: CopperPolylineMobility::Fixed,
            }],
            shared_points: Vec::new(),
            attachments: Vec::new(),
            separations: Vec::new(),
            body_separations: vec![CopperSegmentBodyPair {
                polyline: "fixed-trace".into(),
                body: "movable".into(),
                clearance: 0.25,
            }],
            body_body_separations: Vec::new(),
            maximum_segment_pairs: 2,
        })
        .unwrap();
        let mut backend = CpuReferenceBackend::with_field(NoField);
        let frame = backend.step(
            &mut compiled.world,
            &SolverConfig {
                field_strength: 0.0,
                projection_iterations: 16,
                ..SolverConfig::default()
            },
            0,
        );
        let pose = &compiled.body_poses()[0];
        assert!(pose.position.y < 2.301);
        assert!(frame.metrics.max_constraint_residual < 1.0e-4);
    }

    #[test]
    fn segment_body_contact_exposes_angular_response() {
        let initial_angle = 0.2;
        let mut compiled = compile_copper_repair(&CopperRepairRequest {
            bounds: Bounds::new(Vec2::new(0.0, 0.0), Vec2::new(12.0, 8.0)),
            bodies: vec![CopperBodyInput {
                id: "rotor".into(),
                position: Vec2::new(6.0, 2.3),
                angle_radians: initial_angle,
                size: Vec2::new(4.0, 0.4),
                mobility: Mobility {
                    inverse_mass: 0.0,
                    rotation_mobility: 1.0,
                    translate_x: false,
                    translate_y: false,
                    rotate: true,
                },
            }],
            polylines: vec![CopperPolylineInput {
                id: "fixed-trace".into(),
                points: vec![Vec2::new(2.0, 3.0), Vec2::new(10.0, 3.0)],
                width: 0.5,
                tension_weight: 1.0,
                mobility: CopperPolylineMobility::Fixed,
            }],
            shared_points: Vec::new(),
            attachments: Vec::new(),
            separations: Vec::new(),
            body_separations: vec![CopperSegmentBodyPair {
                polyline: "fixed-trace".into(),
                body: "rotor".into(),
                clearance: 0.25,
            }],
            body_body_separations: Vec::new(),
            maximum_segment_pairs: 2,
        })
        .unwrap();
        let mut backend = CpuReferenceBackend::with_field(NoField);
        let before = backend.step(
            &mut compiled.world,
            &SolverConfig {
                field_strength: 0.0,
                projection_iterations: 0,
                ..SolverConfig::default()
            },
            0,
        );
        let after = backend.step(
            &mut compiled.world,
            &SolverConfig {
                field_strength: 0.0,
                projection_iterations: 1,
                ..SolverConfig::default()
            },
            1,
        );
        assert!(before.metrics.max_constraint_residual > 0.1);
        assert_ne!(compiled.body_poses()[0].angle_radians, initial_angle);
        assert!(after.metrics.max_constraint_residual < before.metrics.max_constraint_residual);
    }

    #[test]
    fn projected_trace_tension_shortens_a_bent_polyline() {
        let request = CopperRepairRequest {
            bounds: Bounds::new(Vec2::ZERO, Vec2::new(12.0, 8.0)),
            bodies: Vec::new(),
            polylines: vec![CopperPolylineInput {
                id: "bent".into(),
                points: vec![
                    Vec2::new(1.0, 4.0),
                    Vec2::new(6.0, 1.0),
                    Vec2::new(11.0, 4.0),
                ],
                width: 0.4,
                tension_weight: 1.0,
                mobility: CopperPolylineMobility::Interior,
            }],
            shared_points: Vec::new(),
            attachments: Vec::new(),
            separations: Vec::new(),
            body_separations: Vec::new(),
            body_body_separations: Vec::new(),
            maximum_segment_pairs: 100,
        };
        let mut compiled = compile_copper_repair(&request).unwrap();
        let before = compiled.polylines()[0]
            .1
            .windows(2)
            .map(|edge| edge[0].distance(edge[1]))
            .sum::<f32>();
        let mut backend = CpuReferenceBackend::with_field(NoField);
        let mut last = None;
        for step in 1..=8 {
            last = Some(backend.step(
                &mut compiled.world,
                &SolverConfig {
                    field_strength: 0.0,
                    trace_tension_strength: 2.0,
                    maximum_trace_tension_step: 0.2,
                    ..SolverConfig::default()
                },
                step,
            ));
        }
        let after = compiled.polylines()[0]
            .1
            .windows(2)
            .map(|edge| edge[0].distance(edge[1]))
            .sum::<f32>();
        let frame = last.unwrap();

        assert!(after < before - 0.5, "before={before}, after={after}");
        assert_eq!(frame.metrics.trace_tension_edges, 2);
        assert!(
            frame
                .vectors
                .iter()
                .any(|vector| vector.source == "trace_tension")
        );
        assert_eq!(compiled.polylines()[0].1[0], Vec2::new(1.0, 4.0));
        assert_eq!(compiled.polylines()[0].1[2], Vec2::new(11.0, 4.0));
    }
}

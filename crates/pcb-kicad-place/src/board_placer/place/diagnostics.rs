// Copyright (C) 2026 Florian Loitsch.

//! Local evidence for parts the placement left without a legal pose.

use super::*;

const MAX_CONFLICTS: usize = 8;

pub(super) fn diagnosis(
    problem: &core::Problem,
    poses: &[core::Pose],
    index: usize,
    references: &[String],
    uuids: &[Option<String>],
    repeated: &BTreeSet<String>,
    pair: Option<&core::legal::RegionPairConflict>,
) -> KiCadPlacementLocalDiagnostic {
    let pose = poses[index];
    let component = &problem.components[index];
    let hits: Vec<_> = (0..problem.components.len())
        .filter(|&other| other != index)
        .filter_map(|other| {
            core::legal::conflict_kind(problem, poses, index, pose, other).map(|kind| (other, kind))
        })
        .collect();
    let label_of = |other: usize| {
        part_label(
            &references[other],
            uuids.get(other).and_then(Option::as_deref),
            repeated,
        )
    };
    let conflicts = hits
        .iter()
        .take(MAX_CONFLICTS)
        .map(|&(other, kind)| {
            let other_pose = poses[other];
            let component = &problem.components[other];
            KiCadPlacementLocalConflict {
                kind,
                other: label_of(other),
                uuid: uuids.get(other).cloned().flatten(),
                fixed: component.fixed,
                side: side(component.side),
                at: [
                    other_pose.position[0],
                    other_pose.position[1],
                    other_pose.angle,
                ],
                body_bounds_mm: body_bounds(component, other_pose),
            }
        })
        .collect();
    let (mut region, mut next_action) =
        region_diagnostic(problem, poses, index, &hits, &label_of, pair);
    if let Some(region) = &mut region {
        let fitting: Vec<_> = region
            .by_rotation
            .iter()
            .filter(|row| row.center_bounds_mm.is_some())
            .collect();
        let cover = super::fixed_region_cover::blockers(problem, poses, index, &fitting, uuids);
        if cover.len() > 1 {
            region.fixed_region_cover = cover
                .into_iter()
                .map(|other| {
                    let pose = poses[other];
                    KiCadPlacementFixedBlocker {
                        part: label_of(other),
                        uuid: uuids[other].clone().expect("fixed footprint has a UUID"),
                        at: [pose.position[0], pose.position[1], pose.angle],
                    }
                })
                .collect();
            let label = label_of(index);
            let parts = region
                .fixed_region_cover
                .iter()
                .map(|blocker| blocker.part.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let face = side(component.side);
            let [left, top, right, bottom] = region.bounds_mm;
            next_action = Some(format!(
                "Fixed footprints {parts} cover every allowed centre and rotation for {label} on {face} in its region (x {left:.2}..{right:.2}, y {top:.2}..{bottom:.2} mm) under the placement body model. Before retrying, review the named holds or whether the mounting and assembly permit the other face within the same region."
            ));
        }
    }
    let violation = core::constraints::hard_violation(problem, index, pose);
    KiCadPlacementLocalDiagnostic {
        side: side(component.side),
        body_bounds_mm: body_bounds(component, pose),
        conflicts,
        more_conflicts: hits.len().saturating_sub(MAX_CONFLICTS),
        outside_outline_mm: (!core::legal::body_inside_outline(problem, index, pose))
            .then(|| problem.bounds()),
        hard_constraint_violation_mm: (violation > 1.0e-6).then_some(violation),
        region,
        next_action,
    }
}

pub(super) fn part_label(
    reference: &str,
    uuid: Option<&str>,
    repeated: &BTreeSet<String>,
) -> String {
    match uuid {
        Some(uuid) if repeated.contains(reference) => format!("{reference} (uuid {uuid})"),
        _ => reference.to_string(),
    }
}

fn side(side: core::Side) -> &'static str {
    match side {
        core::Side::Front => "front",
        core::Side::Back => "back",
        core::Side::Both => "both",
        core::Side::Neither => "neither",
    }
}

fn body_bounds(component: &core::Component, pose: core::Pose) -> [f64; 4] {
    let [x, y] = component.center(pose);
    let [hx, hy] = component.half_extent(pose.angle);
    [x - hx, y - hy, x + hx, y + hy]
}

fn region_diagnostic(
    problem: &core::Problem,
    poses: &[core::Pose],
    index: usize,
    hits: &[(usize, &str)],
    label_of: &impl Fn(usize) -> String,
    pair: Option<&core::legal::RegionPairConflict>,
) -> (Option<KiCadPlacementRegionDiagnostic>, Option<String>) {
    let Some(bounds) = problem
        .constraints
        .regions
        .iter()
        .filter(|(part, _)| *part == index)
        .map(|(_, b)| *b)
        .reduce(|a, b| {
            [
                a[0].max(b[0]),
                a[1].max(b[1]),
                a[2].min(b[2]),
                a[3].min(b[3]),
            ]
        })
    else {
        return (None, None);
    };
    let automatic = problem.constraints.automatic_regions.contains(&index);
    let label = label_of(index);
    let component = &problem.components[index];
    let by_rotation: Vec<_> = component
        .angle_options
        .iter()
        .map(|&angle| {
            let [hx, hy] = component.half_extent(angle);
            let [x1, y1, x2, y2] = bounds;
            let window = [x1 + hx, y1 + hy, x2 - hx, y2 - hy];
            let fits = window[0] <= window[2] + 2.0e-5 && window[1] <= window[3] + 2.0e-5;
            KiCadPlacementRegionRotation {
                angle_deg: angle,
                body_size_mm: [2.0 * hx, 2.0 * hy],
                center_bounds_mm: fits.then_some(window),
            }
        })
        .collect();
    let fitting: Vec<_> = by_rotation
        .iter()
        .filter(|row| row.center_bounds_mm.is_some())
        .collect();
    let blocker = hits.iter().map(|(other, _)| *other).find(|&other| {
        !fitting.is_empty()
            && fitting
                .iter()
                .all(|window| blocks_window(problem, index, other, poses[other], window))
    });
    let extent = format!(
        "{:.2} x {:.2} mm",
        (bounds[2] - bounds[0]).max(0.0),
        (bounds[3] - bounds[1]).max(0.0)
    );
    let packing_conflict = pair.filter(|pair| pair.region_part == index).map(|pair| {
        KiCadPlacementRegionPackingConflict {
            other: label_of(pair.other_part),
            available_mm: pair.available_mm,
            other_minimum_mm: pair.other_minimum_mm,
        }
    });
    let next_action = if let Some(packing) = &packing_conflict {
        let [left, top, right, bottom] = packing.available_mm;
        let [width, height] = packing.other_minimum_mm;
        let other = &packing.other;
        let change = if automatic {
            "Review their assigned area and whether the mechanical design permits either footprint on the other side".to_string()
        } else {
            format!(
                "Change an assigned side if mounting and access permit it, or revise {label}'s region if the enclosure allows"
            )
        };
        Some(format!(
            "{other} needs at least {width:.2} mm along x and {height:.2} mm along y at the allowed rotations. With {label} in its {extent} region, the board bounds leave at most {left:.2} mm left, {right:.2} mm right, {top:.2} mm above and {bottom:.2} mm below. The placement body model cannot fit both at their assigned sides. {change}, then rerun the placement check."
        ))
    } else if !by_rotation.is_empty() && fitting.is_empty() && !automatic {
        Some(format!(
            "{label}'s body exceeds its {extent} region at every allowed rotation. Enlarge its region or revise its allowed rotations, then rerun the placement check."
        ))
    } else {
        blocker.map(|other| {
            let other_label = label_of(other);
            let change = if problem.components[other].fixed {
                if automatic {
                    format!("Review the automatic assignment of {label} and the fixed position of {other_label}")
                } else {
                    format!("Revise {label}'s region or the constraint that fixes {other_label}")
                }
            } else {
                format!("Review a position for {other_label} that clears {label}'s region")
            };
            format!("{other_label}'s body at its retained pose blocks every allowed centre and rotation for {label} in its {extent} region. {change}, then rerun the placement check.")
        })
    };
    (
        Some(KiCadPlacementRegionDiagnostic {
            bounds_mm: bounds,
            automatic,
            origin: if automatic {
                KiCadPlacementRegionOrigin::BoardPiece
            } else {
                KiCadPlacementRegionOrigin::Authored
            },
            by_rotation,
            blocked_by: blocker.map(label_of),
            fixed_region_cover: Vec::new(),
            packing_conflict,
        }),
        next_action,
    )
}

/// A proof for ordinary, axis-aligned bodies only. The farthest allowed
/// centre along each axis must still overlap the stationary rectangle.
/// Include the legaliser's region tolerance and ignore additional halos.
fn blocks_window(
    problem: &core::Problem,
    part: usize,
    other: usize,
    other_pose: core::Pose,
    window: &KiCadPlacementRegionRotation,
) -> bool {
    let mine = &problem.components[part];
    let theirs = &problem.components[other];
    let quarter_turn = |angle: f64| angle % 90.0 == 0.0;
    if mine.round
        || theirs.round
        || mine.copper_only
        || theirs.copper_only
        || !quarter_turn(window.angle_deg)
        || !quarter_turn(other_pose.angle)
        || !mine.side.collides(theirs.side)
        || mine.hollow_for(theirs)
        || theirs.hollow_for(mine)
        || problem.constraints.frames.pass(part, other).is_some()
        || problem.constraints.lies_on(part, other)
        || problem.spacing < 0.0
    {
        return false;
    }
    let Some(bounds) = window.center_bounds_mm else {
        return false;
    };
    let center = theirs.center(other_pose);
    let half = theirs.half_extent(other_pose.angle);
    (0..2).all(|axis| {
        let farthest = (center[axis] - bounds[axis])
            .abs()
            .max((center[axis] - bounds[axis + 2]).abs());
        farthest + 2.0e-5 < half[axis] + window.body_size_mm[axis] / 2.0
    })
}

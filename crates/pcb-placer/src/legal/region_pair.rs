// Copyright (C) 2026 Florian Loitsch.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! An optimistic pair fit bound for a rectangular body held in a region.

use std::collections::BTreeMap;

use crate::{Problem, Side, problem::Component};

/// A pair of movable bodies that cannot fit together on their current side.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionPairConflict {
    pub region_part: usize,
    pub other_part: usize,
    /// The greatest optimistic width or height available to the second
    /// part on the left, top, right and bottom of the region-held part.
    pub available_mm: [f64; 4],
    /// Lower bounds on the second part's width and height, independently.
    pub other_minimum_mm: [f64; 2],
}

const TOLERANCE_MM: f64 = 1.0e-5;
const MAX_PAIR_CHECKS: usize = 200_000;

/// Per-axis size lower bounds across the allowed quarter turns and both
/// relaxation shapes. Taking the smallest size independently for each
/// axis also allows some rectangles the placer would never use.
fn minimum_size(part: &Component) -> Option<[f64; 2]> {
    if part.fixed
        || part.copper_only
        || part.round
        || !matches!(part.side, Side::Front | Side::Back)
        || !part.halo.is_finite()
        || part.halo < 0.0
        || part.angle_options.is_empty()
    {
        return None;
    }
    let mut size = part.body_size;
    if let Some([left, top, right, bottom]) = part.tight {
        size = [size[0].min(right - left), size[1].min(bottom - top)];
    }
    if !size
        .iter()
        .all(|extent| extent.is_finite() && *extent > 0.0)
    {
        return None;
    }
    let mut minimum = [f64::INFINITY; 2];
    for &angle in &part.angle_options {
        if !angle.is_finite() || angle.rem_euclid(90.0) != 0.0 {
            return None;
        }
        let swapped = angle.rem_euclid(180.0) != 0.0;
        let [x, y] = if swapped { [size[1], size[0]] } else { size };
        minimum = [minimum[0].min(x), minimum[1].min(y)];
    }
    Some(minimum)
}

/// Both the normal body and the tight body must occupy a solid box for
/// this pair. The legality test sometimes uses a hollow part's solid box
/// against a part with holes.
fn blocks_as_box(part: &Component, other: &Component) -> bool {
    !part.hollow_for(other)
        && !(part.tight.is_some()
            && !part.tight_hollow.is_empty()
            && (!other.has_holes() || part.holes_inside || part.courtyards.len() > 1))
}

/// A sufficient condition for infeasibility, independent of poses and
/// search budgets. Two axis-aligned solid rectangles must be separated
/// along either x or y. Give the first body its whole region and the
/// second the board's whole bounding box, with zero spacing, zero edge
/// margin and no cutouts. If all four separation strips still come up
/// short, the pair cannot fit. A missing certificate draws no conclusion.
pub fn region_pair_conflict(problem: &Problem) -> Option<RegionPairConflict> {
    if problem.constraints.regions.is_empty()
        || !problem.spacing.is_finite()
        || problem.spacing < 0.0
        || !problem.min_spacing.is_finite()
        || problem.min_spacing < 0.0
    {
        return None;
    }
    let board = problem.bounds();
    if !board.iter().all(|value| value.is_finite()) || board[0] >= board[2] || board[1] >= board[3]
    {
        return None;
    }

    let mut sizes: Vec<_> = problem.components.iter().map(minimum_size).collect();
    // With regions present, an ordinary quarter-turn body must stay
    // inside the outline, apart from the boundary tolerance. An overhang
    // or held edge can instead allow part of the body outside it.
    for &(part, _, _) in &problem.constraints.edges {
        *sizes.get_mut(part)? = None;
    }
    for &(part, _, _) in &problem.constraints.overhangs {
        *sizes.get_mut(part)? = None;
    }
    // `lies_on` can be enabled at a later relaxation for a part that
    // could not find room off a yielding mark.
    for &part in &problem.constraints.yielding {
        *sizes.get_mut(part)? = None;
    }

    let mut regions = BTreeMap::new();
    for &(part, bounds) in &problem.constraints.regions {
        if !bounds.iter().all(|value| value.is_finite()) {
            continue;
        }
        let held = regions.entry(part).or_insert(board);
        for axis in 0..2 {
            held[axis] = held[axis].max(bounds[axis]);
            held[axis + 2] = held[axis + 2].min(bounds[axis + 2]);
        }
    }
    let mut checks = 0;
    for (region_part, region) in regions {
        let Some([width, height]) = sizes.get(region_part).copied().flatten() else {
            continue;
        };
        if region[2] - region[0] + TOLERANCE_MM < width
            || region[3] - region[1] + TOLERANCE_MM < height
        {
            // The individual outline/region check can explain this case.
            continue;
        }
        let available_mm = [
            region[2] - width - board[0],
            region[3] - height - board[1],
            board[2] - region[0] - width,
            board[3] - region[1] - height,
        ];
        let held = &problem.components[region_part];
        for (other_part, minimum) in sizes.iter().enumerate() {
            if checks == MAX_PAIR_CHECKS {
                return None;
            }
            checks += 1;
            let Some([other_width, other_height]) = *minimum else {
                continue;
            };
            let other = &problem.components[other_part];
            if region_part != other_part
                && held.side == other.side
                && available_mm[0].max(available_mm[2]) + TOLERANCE_MM < other_width
                && available_mm[1].max(available_mm[3]) + TOLERANCE_MM < other_height
                && problem
                    .constraints
                    .frames
                    .pass(region_part, other_part)
                    .is_none()
                && blocks_as_box(held, other)
                && blocks_as_box(other, held)
            {
                return Some(RegionPairConflict {
                    region_part,
                    other_part,
                    available_mm,
                    other_minimum_mm: [other_width, other_height],
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::tests::pair;
    use super::*;
    use crate::constraints::{Edge, FramePass};

    fn restricted_pair() -> Problem {
        let (mut problem, _) = pair(0.0, 0.0);
        // The bodies and brief of OpenFC's same-side MCU and card socket.
        problem.outline = vec![
            [134.12, 68.188546],
            [172.062302, 68.188546],
            [172.062302, 106.130848],
            [134.12, 106.130848],
        ];
        problem.components[0].body_size = [10.60, 10.60];
        problem.components[1].body_size = [16.70, 15.79];
        problem.components[1].angle_options = vec![0.0, 90.0, 180.0, 270.0];
        problem
            .constraints
            .regions
            .push((0, [147.0, 80.5, 159.5, 94.0]));
        problem
    }

    #[test]
    fn a_region_and_a_board_can_leave_no_strip_wide_enough_for_a_second_body() {
        let mut problem = restricted_pair();
        problem.constraints.pad_clearance = true;
        let conflict = region_pair_conflict(&problem).unwrap();
        assert_eq!((conflict.region_part, conflict.other_part), (0, 1));
        assert_eq!(conflict.other_minimum_mm, [15.79, 15.79]);
        for (actual, expected) in conflict
            .available_mm
            .into_iter()
            .zip([14.780, 15.211454, 14.462302, 15.030848])
        {
            assert!((actual - expected).abs() < 1.0e-8);
        }
    }

    #[test]
    fn a_non_orthogonal_or_tighter_body_prevents_the_rectangle_proof() {
        let mut problem = restricted_pair();
        problem.components[1].angle_options.push(45.0);
        assert!(region_pair_conflict(&problem).is_none());

        let mut problem = restricted_pair();
        problem.components[1].tight = Some([-8.35, -7.0, 8.35, 7.0]);
        assert!(region_pair_conflict(&problem).is_none());

        let mut problem = restricted_pair();
        problem.components[1].tight = Some([-8.35, -7.895, 8.35, 7.895]);
        problem.components[1].tight_hollow = vec![[-1.0, -1.0, 1.0, 1.0]];
        assert!(region_pair_conflict(&problem).is_none());
    }

    #[test]
    fn shapes_and_side_exceptions_never_use_the_rectangle_proof() {
        let mut problem = restricted_pair();
        problem.components[1].hollow = vec![[-1.0, -1.0, 1.0, 1.0]];
        assert!(region_pair_conflict(&problem).is_none());

        let mut problem = restricted_pair();
        problem.components[1].round = true;
        assert!(region_pair_conflict(&problem).is_none());

        let mut problem = restricted_pair();
        problem.components[1].side = Side::Back;
        assert!(region_pair_conflict(&problem).is_none());

        let mut problem = restricted_pair();
        problem.components[1].fixed = true;
        assert!(region_pair_conflict(&problem).is_none());

        let mut problem = restricted_pair();
        problem.constraints.edges.push((1, Edge::Bottom, 1.0));
        assert!(region_pair_conflict(&problem).is_none());

        let mut problem = restricted_pair();
        problem
            .constraints
            .overhangs
            .push((1, Edge::Top, [-1.0, -1.0, 1.0, 1.0]));
        assert!(region_pair_conflict(&problem).is_none());
    }

    #[test]
    fn a_strip_that_could_fit_in_the_outline_bounds_is_not_refuted_by_a_notch() {
        let mut problem = restricted_pair();
        problem.outline.splice(
            2..4,
            [
                [172.062302, 107.0],
                [153.0, 107.0],
                [153.0, 102.0],
                [148.0, 102.0],
                [148.0, 107.0],
                [134.12, 107.0],
            ],
        );
        assert!(region_pair_conflict(&problem).is_none());
    }

    #[test]
    fn the_strip_bound_uses_board_coordinates_and_the_body_not_the_footprint_origin() {
        let mut problem = restricted_pair();
        let original = region_pair_conflict(&problem).unwrap();
        let delta = [-100.0, 50.0];
        for point in &mut problem.outline {
            point[0] += delta[0];
            point[1] += delta[1];
        }
        let region = &mut problem.constraints.regions[0].1;
        for axis in 0..2 {
            region[axis] += delta[axis];
            region[axis + 2] += delta[axis];
        }
        problem.components[0].body_center = [2.0, 3.0];
        problem.components[1].body_center = [-1.0, 4.0];
        let translated = region_pair_conflict(&problem).unwrap();
        assert_eq!(original.other_minimum_mm, translated.other_minimum_mm);
        for (a, b) in original
            .available_mm
            .into_iter()
            .zip(translated.available_mm)
        {
            assert!((a - b).abs() < 1.0e-8);
        }
    }

    #[test]
    fn yielding_parts_and_frame_passes_can_allow_the_bodies_to_overlap() {
        let mut problem = restricted_pair();
        problem.constraints.yielding.push(1);
        // Include the possibility of a later relaxation permitting overlap.
        assert!(!problem.constraints.lies_on(0, 1));
        assert!(region_pair_conflict(&problem).is_none());
        problem.constraints.on_marks.push(0);
        assert!(problem.constraints.lies_on(0, 1));
        assert!(region_pair_conflict(&problem).is_none());

        let mut problem = restricted_pair();
        problem.constraints.frames.keepouts.push(1);
        problem.constraints.frames.passes.push((
            0,
            FramePass {
                connector: true,
                ..FramePass::default()
            },
        ));
        assert!(problem.constraints.frames.pass(0, 1).is_some());
        assert!(region_pair_conflict(&problem).is_none());
    }
}

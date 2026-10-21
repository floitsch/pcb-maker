// Copyright (C) 2026 Toit contributors.

//! Point and segment queries against lowered pad and outline geometry, for
//! the tests only: production hands this geometry to the router, which does
//! its own queries. The tests use them as an oracle for the shapes.

use super::*;

impl ObstacleGeometry {
    pub(super) fn contains(&self, point: [f64; 2], inflate: f64) -> bool {
        match self {
            Self::Circle { center, radius } => {
                distance_squared(point, *center) < (radius + inflate).powi(2)
            }
            Self::Segment { start, end, radius } => {
                point_segment_distance_squared(point, *start, *end) < (radius + inflate).powi(2)
            }
            Self::Rectangle {
                center,
                half_size,
                angle_degrees,
            } => {
                let local = rotate_vector(
                    [point[0] - center[0], point[1] - center[1]],
                    -*angle_degrees,
                );
                point_intersects_rounded_rectangle(local, *half_size, inflate)
            }
            Self::Polygon { points } => point_intersects_inflated_polygon(point, points, inflate),
            Self::Union { parts } => parts.iter().any(|part| part.contains(point, inflate)),
        }
    }

    pub(super) fn intersects_segment(&self, start: [f64; 2], end: [f64; 2], inflate: f64) -> bool {
        match self {
            Self::Circle { center, radius } => {
                point_segment_distance_squared(*center, start, end) < (radius + inflate).powi(2)
            }
            Self::Segment {
                start: obstacle_start,
                end: obstacle_end,
                radius,
            } => {
                segment_segment_distance_squared(start, end, *obstacle_start, *obstacle_end)
                    < (radius + inflate).powi(2)
            }
            Self::Rectangle {
                center,
                half_size,
                angle_degrees,
            } => {
                let local_start = rotate_vector(
                    [start[0] - center[0], start[1] - center[1]],
                    -*angle_degrees,
                );
                let local_end =
                    rotate_vector([end[0] - center[0], end[1] - center[1]], -*angle_degrees);
                segment_intersects_rounded_rectangle(local_start, local_end, *half_size, inflate)
            }
            Self::Polygon { points } => {
                segment_intersects_inflated_polygon(start, end, points, inflate)
            }
            Self::Union { parts } => parts
                .iter()
                .any(|part| part.intersects_segment(start, end, inflate)),
        }
    }
}

impl BoardOutline {
    pub(super) fn contains_point_with_clearance(&self, point: [f64; 2], clearance: f64) -> bool {
        point_in_polygon(point, &self.points)
            && self.edges().all(|(start, end)| {
                point_segment_distance_squared(point, start, end) + 1.0e-18 >= clearance * clearance
            })
    }

    pub(super) fn contains_segment_with_clearance(
        &self,
        start: [f64; 2],
        end: [f64; 2],
        clearance: f64,
    ) -> bool {
        self.contains_point_with_clearance(start, clearance)
            && self.contains_point_with_clearance(end, clearance)
            && self.segment_clears_boundary(start, end, clearance)
    }

    pub(super) fn segment_clears_boundary(
        &self,
        start: [f64; 2],
        end: [f64; 2],
        clearance: f64,
    ) -> bool {
        self.edges().all(|(edge_start, edge_end)| {
            segment_segment_distance_squared(start, end, edge_start, edge_end) + 1.0e-18
                >= clearance * clearance
        })
    }

    pub(super) fn edges(&self) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
        self.points
            .iter()
            .copied()
            .zip(self.points.iter().copied().cycle().skip(1))
            .take(self.points.len())
    }
}

fn point_intersects_inflated_polygon(point: [f64; 2], polygon: &[[f64; 2]], inflate: f64) -> bool {
    if point_in_polygon(point, polygon) {
        return true;
    }
    polygon
        .windows(2)
        .any(|edge| point_segment_distance_squared(point, edge[0], edge[1]) < inflate.powi(2))
        || point_segment_distance_squared(point, polygon[polygon.len() - 1], polygon[0])
            < inflate.powi(2)
}

fn segment_intersects_inflated_polygon(
    start: [f64; 2],
    end: [f64; 2],
    polygon: &[[f64; 2]],
    inflate: f64,
) -> bool {
    point_in_polygon(start, polygon)
        || point_in_polygon(end, polygon)
        || polygon.windows(2).any(|edge| {
            segment_segment_distance_squared(start, end, edge[0], edge[1]) < inflate.powi(2)
        })
        || segment_segment_distance_squared(start, end, polygon[polygon.len() - 1], polygon[0])
            < inflate.powi(2)
}

fn point_intersects_rounded_rectangle(point: [f64; 2], half_size: [f64; 2], inflate: f64) -> bool {
    let outside = [
        (point[0].abs() - half_size[0]).max(0.0),
        (point[1].abs() - half_size[1]).max(0.0),
    ];
    (outside[0] == 0.0 && outside[1] == 0.0)
        || outside[0] * outside[0] + outside[1] * outside[1] < inflate * inflate
}

fn segment_intersects_rounded_rectangle(
    start: [f64; 2],
    end: [f64; 2],
    half_size: [f64; 2],
    inflate: f64,
) -> bool {
    if segment_intersects_axis_aligned_rectangle(start, end, half_size) {
        return true;
    }
    let corners = [
        [-half_size[0], -half_size[1]],
        [half_size[0], -half_size[1]],
        [half_size[0], half_size[1]],
        [-half_size[0], half_size[1]],
    ];
    (0..4).any(|edge| {
        segment_segment_distance_squared(start, end, corners[edge], corners[(edge + 1) % 4])
            < inflate * inflate
    })
}

fn segment_segment_distance_squared(
    first_start: [f64; 2],
    first_end: [f64; 2],
    second_start: [f64; 2],
    second_end: [f64; 2],
) -> f64 {
    if line_segments_intersect(first_start, first_end, second_start, second_end) {
        return 0.0;
    }
    point_segment_distance_squared(first_start, second_start, second_end)
        .min(point_segment_distance_squared(
            first_end,
            second_start,
            second_end,
        ))
        .min(point_segment_distance_squared(
            second_start,
            first_start,
            first_end,
        ))
        .min(point_segment_distance_squared(
            second_end,
            first_start,
            first_end,
        ))
}

fn line_segments_intersect(
    first_start: [f64; 2],
    first_end: [f64; 2],
    second_start: [f64; 2],
    second_end: [f64; 2],
) -> bool {
    fn cross(origin: [f64; 2], left: [f64; 2], right: [f64; 2]) -> f64 {
        (left[0] - origin[0]) * (right[1] - origin[1])
            - (left[1] - origin[1]) * (right[0] - origin[0])
    }
    fn contains(start: [f64; 2], end: [f64; 2], point: [f64; 2]) -> bool {
        point[0] >= start[0].min(end[0]) - 1e-9
            && point[0] <= start[0].max(end[0]) + 1e-9
            && point[1] >= start[1].min(end[1]) - 1e-9
            && point[1] <= start[1].max(end[1]) + 1e-9
    }

    let first_side_a = cross(first_start, first_end, second_start);
    let first_side_b = cross(first_start, first_end, second_end);
    let second_side_a = cross(second_start, second_end, first_start);
    let second_side_b = cross(second_start, second_end, first_end);
    if ((first_side_a > 1e-9 && first_side_b < -1e-9)
        || (first_side_a < -1e-9 && first_side_b > 1e-9))
        && ((second_side_a > 1e-9 && second_side_b < -1e-9)
            || (second_side_a < -1e-9 && second_side_b > 1e-9))
    {
        return true;
    }
    (first_side_a.abs() <= 1e-9 && contains(first_start, first_end, second_start))
        || (first_side_b.abs() <= 1e-9 && contains(first_start, first_end, second_end))
        || (second_side_a.abs() <= 1e-9 && contains(second_start, second_end, first_start))
        || (second_side_b.abs() <= 1e-9 && contains(second_start, second_end, first_end))
}

fn segment_intersects_axis_aligned_rectangle(
    start: [f64; 2],
    end: [f64; 2],
    half_size: [f64; 2],
) -> bool {
    let delta = [end[0] - start[0], end[1] - start[1]];
    let mut minimum = 0.0_f64;
    let mut maximum = 1.0_f64;
    for axis in 0..2 {
        if delta[axis].abs() <= f64::EPSILON {
            if start[axis].abs() <= half_size[axis] {
                continue;
            }
            return false;
        }
        let mut enter = (-half_size[axis] - start[axis]) / delta[axis];
        let mut leave = (half_size[axis] - start[axis]) / delta[axis];
        if enter > leave {
            std::mem::swap(&mut enter, &mut leave);
        }
        minimum = minimum.max(enter);
        maximum = maximum.min(leave);
        if minimum > maximum {
            return false;
        }
    }
    true
}

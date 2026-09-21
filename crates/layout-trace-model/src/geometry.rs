use std::f64::consts::PI;

use crate::model::{Rect, Vec2};

pub const EPSILON: f64 = 1.0e-9;

pub fn add(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x + b.x, a.y + b.y)
}

pub fn sub(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x - b.x, a.y - b.y)
}

pub fn scale(a: Vec2, factor: f64) -> Vec2 {
    Vec2::new(a.x * factor, a.y * factor)
}

pub fn dot(a: Vec2, b: Vec2) -> f64 {
    a.x * b.x + a.y * b.y
}

pub fn cross(a: Vec2, b: Vec2) -> f64 {
    a.x * b.y - a.y * b.x
}

pub fn length_squared(a: Vec2) -> f64 {
    dot(a, a)
}

pub fn length(a: Vec2) -> f64 {
    length_squared(a).sqrt()
}

pub fn normalized_or(a: Vec2, fallback: Vec2) -> Vec2 {
    let magnitude = length(a);
    if magnitude <= EPSILON {
        fallback
    } else {
        scale(a, 1.0 / magnitude)
    }
}

pub fn clamp_length(a: Vec2, maximum: f64) -> Vec2 {
    let magnitude = length(a);
    if magnitude > maximum {
        scale(a, maximum / magnitude)
    } else {
        a
    }
}

pub fn rotate_degrees(point: Vec2, degrees: f64) -> Vec2 {
    let angle = degrees * PI / 180.0;
    let (sin, cos) = angle.sin_cos();
    Vec2::new(cos * point.x - sin * point.y, sin * point.x + cos * point.y)
}

#[derive(Clone, Copy, Debug)]
pub struct Obb {
    pub center: Vec2,
    pub half_size: Vec2,
    pub rotation_degrees: f64,
}

impl Obb {
    pub fn corners(self) -> [Vec2; 4] {
        [
            Vec2::new(-self.half_size.x, -self.half_size.y),
            Vec2::new(self.half_size.x, -self.half_size.y),
            Vec2::new(self.half_size.x, self.half_size.y),
            Vec2::new(-self.half_size.x, self.half_size.y),
        ]
        .map(|corner| add(self.center, rotate_degrees(corner, self.rotation_degrees)))
    }

    pub fn axis_aligned_extent(self) -> Vec2 {
        let angle = self.rotation_degrees * PI / 180.0;
        let (sin, cos) = angle.sin_cos();
        Vec2::new(
            cos.abs() * self.half_size.x + sin.abs() * self.half_size.y,
            sin.abs() * self.half_size.x + cos.abs() * self.half_size.y,
        )
    }
}

pub fn point_obb_signed_distance(point: Vec2, obb: Obb) -> (f64, Vec2) {
    let local = rotate_degrees(sub(point, obb.center), -obb.rotation_degrees);
    let clamped = Vec2::new(
        local.x.clamp(-obb.half_size.x, obb.half_size.x),
        local.y.clamp(-obb.half_size.y, obb.half_size.y),
    );
    let delta = sub(local, clamped);
    let outside_distance = length(delta);

    let (signed_distance, local_normal) = if outside_distance > EPSILON {
        (outside_distance, scale(delta, 1.0 / outside_distance))
    } else {
        let distance_x = obb.half_size.x - local.x.abs();
        let distance_y = obb.half_size.y - local.y.abs();
        if distance_x < distance_y {
            (
                -distance_x,
                Vec2::new(if local.x < 0.0 { -1.0 } else { 1.0 }, 0.0),
            )
        } else {
            (
                -distance_y,
                Vec2::new(0.0, if local.y < 0.0 { -1.0 } else { 1.0 }),
            )
        }
    };

    (
        signed_distance,
        rotate_degrees(local_normal, obb.rotation_degrees),
    )
}

pub fn point_segment_distance(point: Vec2, start: Vec2, end: Vec2) -> f64 {
    let segment = sub(end, start);
    let denominator = length_squared(segment);
    if denominator <= EPSILON {
        return length(sub(point, start));
    }
    let t = (dot(sub(point, start), segment) / denominator).clamp(0.0, 1.0);
    length(sub(point, add(start, scale(segment, t))))
}

fn orientation(a: Vec2, b: Vec2, c: Vec2) -> f64 {
    cross(sub(b, a), sub(c, a))
}

fn on_segment(a: Vec2, point: Vec2, b: Vec2) -> bool {
    point.x >= a.x.min(b.x) - EPSILON
        && point.x <= a.x.max(b.x) + EPSILON
        && point.y >= a.y.min(b.y) - EPSILON
        && point.y <= a.y.max(b.y) + EPSILON
}

pub fn segments_intersect(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> bool {
    let o1 = orientation(a0, a1, b0);
    let o2 = orientation(a0, a1, b1);
    let o3 = orientation(b0, b1, a0);
    let o4 = orientation(b0, b1, a1);
    if ((o1 > EPSILON && o2 < -EPSILON) || (o1 < -EPSILON && o2 > EPSILON))
        && ((o3 > EPSILON && o4 < -EPSILON) || (o3 < -EPSILON && o4 > EPSILON))
    {
        return true;
    }
    (o1.abs() <= EPSILON && on_segment(a0, b0, a1))
        || (o2.abs() <= EPSILON && on_segment(a0, b1, a1))
        || (o3.abs() <= EPSILON && on_segment(b0, a0, b1))
        || (o4.abs() <= EPSILON && on_segment(b0, a1, b1))
}

pub fn segment_distance(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> f64 {
    if segments_intersect(a0, a1, b0, b1) {
        0.0
    } else {
        point_segment_distance(a0, b0, b1)
            .min(point_segment_distance(a1, b0, b1))
            .min(point_segment_distance(b0, a0, a1))
            .min(point_segment_distance(b1, a0, a1))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SegmentProximity {
    pub point_a: Vec2,
    pub point_b: Vec2,
    pub parameter_a: f64,
    pub parameter_b: f64,
    pub distance: f64,
}

/// Closest points on two finite segments, including stable parameters for
/// distributing a contact impulse to their endpoints.
pub fn segment_proximity(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> SegmentProximity {
    let first = sub(a1, a0);
    let second = sub(b1, b0);
    let offset = sub(a0, b0);
    let first_length = dot(first, first);
    let second_length = dot(second, second);
    let mut parameter_a;
    let mut parameter_b;

    if first_length <= EPSILON && second_length <= EPSILON {
        parameter_a = 0.0;
        parameter_b = 0.0;
    } else if first_length <= EPSILON {
        parameter_a = 0.0;
        parameter_b = (dot(second, offset) / second_length).clamp(0.0, 1.0);
    } else {
        let first_offset = dot(first, offset);
        if second_length <= EPSILON {
            parameter_b = 0.0;
            parameter_a = (-first_offset / first_length).clamp(0.0, 1.0);
        } else {
            let second_offset = dot(second, offset);
            let coupling = dot(first, second);
            let denominator = first_length * second_length - coupling * coupling;
            parameter_a = if denominator.abs() > EPSILON {
                ((coupling * second_offset - first_offset * second_length) / denominator)
                    .clamp(0.0, 1.0)
            } else {
                0.0
            };
            parameter_b = (coupling * parameter_a + second_offset) / second_length;
            if parameter_b < 0.0 {
                parameter_b = 0.0;
                parameter_a = (-first_offset / first_length).clamp(0.0, 1.0);
            } else if parameter_b > 1.0 {
                parameter_b = 1.0;
                parameter_a = ((coupling - first_offset) / first_length).clamp(0.0, 1.0);
            }
        }
    }

    let point_a = add(a0, scale(first, parameter_a));
    let point_b = add(b0, scale(second, parameter_b));
    SegmentProximity {
        point_a,
        point_b,
        parameter_a,
        parameter_b,
        distance: length(sub(point_a, point_b)),
    }
}

pub fn segment_obb_distance(start: Vec2, end: Vec2, obb: Obb) -> f64 {
    if point_obb_signed_distance(start, obb).0 <= 0.0
        || point_obb_signed_distance(end, obb).0 <= 0.0
    {
        return 0.0;
    }
    let corners = obb.corners();
    (0..4)
        .map(|index| segment_distance(start, end, corners[index], corners[(index + 1) % 4]))
        .fold(f64::INFINITY, f64::min)
}

fn project(points: &[Vec2; 4], axis: Vec2) -> (f64, f64) {
    points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |range, point| {
            let value = dot(*point, axis);
            (range.0.min(value), range.1.max(value))
        })
}

pub fn obbs_overlap(a: Obb, b: Obb) -> bool {
    let a_corners = a.corners();
    let b_corners = b.corners();
    let axes = [
        normalized_or(sub(a_corners[1], a_corners[0]), Vec2::new(1.0, 0.0)),
        normalized_or(sub(a_corners[3], a_corners[0]), Vec2::new(0.0, 1.0)),
        normalized_or(sub(b_corners[1], b_corners[0]), Vec2::new(1.0, 0.0)),
        normalized_or(sub(b_corners[3], b_corners[0]), Vec2::new(0.0, 1.0)),
    ];
    axes.into_iter().all(|axis| {
        let a_range = project(&a_corners, axis);
        let b_range = project(&b_corners, axis);
        a_range.1 >= b_range.0 - EPSILON && b_range.1 >= a_range.0 - EPSILON
    })
}

pub fn obb_distance(a: Obb, b: Obb) -> f64 {
    if obbs_overlap(a, b) {
        return 0.0;
    }
    let a_corners = a.corners();
    let b_corners = b.corners();
    let mut distance = f64::INFINITY;
    for a_index in 0..4 {
        for b_index in 0..4 {
            distance = distance.min(segment_distance(
                a_corners[a_index],
                a_corners[(a_index + 1) % 4],
                b_corners[b_index],
                b_corners[(b_index + 1) % 4],
            ));
        }
    }
    distance
}

pub fn rect_contains_with_margin(rect: Rect, point: Vec2, margin: f64) -> bool {
    point.x >= rect.min.x + margin - EPSILON
        && point.x <= rect.max.x - margin + EPSILON
        && point.y >= rect.min.y + margin - EPSILON
        && point.y <= rect.max.y - margin + EPSILON
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    #[test]
    fn crossing_segments_have_zero_distance() {
        assert_abs_diff_eq!(
            segment_distance(
                Vec2::new(0.0, 0.0),
                Vec2::new(2.0, 2.0),
                Vec2::new(0.0, 2.0),
                Vec2::new(2.0, 0.0),
            ),
            0.0
        );
    }

    #[test]
    fn separated_segments_report_distance() {
        assert_abs_diff_eq!(
            segment_distance(
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 2.0),
                Vec2::new(1.0, 2.0),
            ),
            2.0
        );
    }

    #[test]
    fn segment_proximity_reports_crossing_parameters() {
        let proximity = segment_proximity(
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(0.0, 2.0),
            Vec2::new(2.0, 0.0),
        );
        assert_abs_diff_eq!(proximity.distance, 0.0, epsilon = 1.0e-9);
        assert_abs_diff_eq!(proximity.parameter_a, 0.5, epsilon = 1.0e-9);
        assert_abs_diff_eq!(proximity.parameter_b, 0.5, epsilon = 1.0e-9);
        assert_abs_diff_eq!(proximity.point_a.x, 1.0, epsilon = 1.0e-9);
        assert_abs_diff_eq!(proximity.point_a.y, 1.0, epsilon = 1.0e-9);
    }

    #[test]
    fn signed_distance_is_negative_inside_box() {
        let (distance, normal) = point_obb_signed_distance(
            Vec2::new(0.9, 0.0),
            Obb {
                center: Vec2::ZERO,
                half_size: Vec2::new(1.0, 2.0),
                rotation_degrees: 0.0,
            },
        );
        assert_abs_diff_eq!(distance, -0.1);
        assert_eq!(normal, Vec2::new(1.0, 0.0));
    }
}

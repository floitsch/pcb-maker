// Copyright (C) 2026 Toit contributors.

//! Plane geometry shared by the board readers.

pub(crate) fn point_in_polygon(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let first = polygon[current];
        let second = polygon[previous];
        if point_segment_distance_squared(point, first, second) <= 1.0e-18 {
            return true;
        }
        if (first[1] > point[1]) != (second[1] > point[1])
            && point[0]
                < (second[0] - first[0]) * (point[1] - first[1]) / (second[1] - first[1]) + first[0]
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

/// The sine and cosine of an angle in degrees, exact for quarter turns
/// (`pcb_router::geometry::degrees_sin_cos`): one angle written 180 or
/// -180, 270 or -90 by two saves lowers alike.
pub(crate) fn quarter_sin_cos(degrees: f64) -> (f64, f64) {
    pcb_router::geometry::degrees_sin_cos(degrees)
}

pub(crate) fn rotate_vector(vector: [f64; 2], degrees: f64) -> [f64; 2] {
    let (sine, cosine) = quarter_sin_cos(degrees);
    [
        vector[0] * cosine - vector[1] * sine,
        vector[0] * sine + vector[1] * cosine,
    ]
}

pub(crate) fn distance_squared(left: [f64; 2], right: [f64; 2]) -> f64 {
    (left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2)
}

pub(crate) fn point_segment_distance_squared(
    point: [f64; 2],
    start: [f64; 2],
    end: [f64; 2],
) -> f64 {
    let segment = [end[0] - start[0], end[1] - start[1]];
    let length_squared = segment[0].powi(2) + segment[1].powi(2);
    if length_squared <= f64::EPSILON {
        return distance_squared(point, start);
    }
    let projection = (((point[0] - start[0]) * segment[0] + (point[1] - start[1]) * segment[1])
        / length_squared)
        .clamp(0.0, 1.0);
    distance_squared(
        point,
        [
            start[0] + projection * segment[0],
            start[1] + projection * segment[1],
        ],
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Point(pub(crate) i64, pub(crate) i64);

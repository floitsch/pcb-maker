// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Exact 2D distance queries for the copper shapes a board contains.

pub type Point = [f64; 2];

/// A filled copper or keepout shape. Distances are zero inside the shape.
#[derive(Clone, Debug)]
pub enum Shape {
    Circle { center: Point, radius: f64 },
    /// A segment swept by a disc: tracks, oval pads, rounded edges.
    Capsule { start: Point, end: Point, radius: f64 },
    /// A simple (possibly concave) filled polygon.
    Polygon { points: Vec<Point> },
    Union { parts: Vec<Shape> },
}

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub minimum: Point,
    pub maximum: Point,
}

impl Aabb {
    pub fn inflated(self, amount: f64) -> Self {
        Self {
            minimum: [self.minimum[0] - amount, self.minimum[1] - amount],
            maximum: [self.maximum[0] + amount, self.maximum[1] + amount],
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            minimum: [
                self.minimum[0].min(other.minimum[0]),
                self.minimum[1].min(other.minimum[1]),
            ],
            maximum: [
                self.maximum[0].max(other.maximum[0]),
                self.maximum[1].max(other.maximum[1]),
            ],
        }
    }

    pub fn intersects(self, other: Self) -> bool {
        self.minimum[0] <= other.maximum[0]
            && self.maximum[0] >= other.minimum[0]
            && self.minimum[1] <= other.maximum[1]
            && self.maximum[1] >= other.minimum[1]
    }
}

impl Shape {
    pub fn rectangle(center: Point, half_size: Point, angle_degrees: f64) -> Self {
        let (sin, cos) = angle_degrees.to_radians().sin_cos();
        let corner = |x: f64, y: f64| {
            [
                center[0] + x * cos - y * sin,
                center[1] + x * sin + y * cos,
            ]
        };
        Self::Polygon {
            points: vec![
                corner(-half_size[0], -half_size[1]),
                corner(half_size[0], -half_size[1]),
                corner(half_size[0], half_size[1]),
                corner(-half_size[0], half_size[1]),
            ],
        }
    }

    pub fn aabb(&self) -> Aabb {
        match self {
            Self::Circle { center, radius } => Aabb {
                minimum: [center[0] - radius, center[1] - radius],
                maximum: [center[0] + radius, center[1] + radius],
            },
            Self::Capsule { start, end, radius } => Aabb {
                minimum: [start[0].min(end[0]) - radius, start[1].min(end[1]) - radius],
                maximum: [start[0].max(end[0]) + radius, start[1].max(end[1]) + radius],
            },
            Self::Polygon { points } => {
                let mut bounds = Aabb {
                    minimum: [f64::INFINITY; 2],
                    maximum: [f64::NEG_INFINITY; 2],
                };
                for point in points {
                    for axis in 0..2 {
                        bounds.minimum[axis] = bounds.minimum[axis].min(point[axis]);
                        bounds.maximum[axis] = bounds.maximum[axis].max(point[axis]);
                    }
                }
                bounds
            }
            Self::Union { parts } => parts
                .iter()
                .map(Shape::aabb)
                .reduce(Aabb::union)
                .unwrap_or(Aabb {
                    minimum: [0.0; 2],
                    maximum: [0.0; 2],
                }),
        }
    }

    /// Distance from `point` to the filled shape; zero inside.
    pub fn distance_to_point(&self, point: Point) -> f64 {
        match self {
            Self::Circle { center, radius } => (distance(point, *center) - radius).max(0.0),
            Self::Capsule { start, end, radius } => {
                (point_segment_distance(point, *start, *end) - radius).max(0.0)
            }
            Self::Polygon { points } => {
                if point_in_polygon(point, points) {
                    0.0
                } else {
                    polygon_edges(points)
                        .map(|(a, b)| point_segment_distance(point, a, b))
                        .fold(f64::INFINITY, f64::min)
                }
            }
            Self::Union { parts } => parts
                .iter()
                .map(|part| part.distance_to_point(point))
                .fold(f64::INFINITY, f64::min),
        }
    }

    /// Distance from the segment `a`–`b` to the filled shape; zero on overlap.
    pub fn distance_to_segment(&self, a: Point, b: Point) -> f64 {
        match self {
            Self::Circle { center, radius } => {
                (point_segment_distance(*center, a, b) - radius).max(0.0)
            }
            Self::Capsule { start, end, radius } => {
                (segment_segment_distance(a, b, *start, *end) - radius).max(0.0)
            }
            Self::Polygon { points } => {
                if point_in_polygon(a, points) || point_in_polygon(b, points) {
                    0.0
                } else {
                    polygon_edges(points)
                        .map(|(c, d)| segment_segment_distance(a, b, c, d))
                        .fold(f64::INFINITY, f64::min)
                }
            }
            Self::Union { parts } => parts
                .iter()
                .map(|part| part.distance_to_segment(a, b))
                .fold(f64::INFINITY, f64::min),
        }
    }

    pub fn contains(&self, point: Point) -> bool {
        self.distance_to_point(point) <= 0.0
    }
}

pub fn polygon_edges(points: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(points.len())
}

pub fn distance(a: Point, b: Point) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

pub fn point_segment_distance(point: Point, a: Point, b: Point) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length_squared = ab[0] * ab[0] + ab[1] * ab[1];
    if length_squared <= 0.0 {
        return distance(point, a);
    }
    let t = (((point[0] - a[0]) * ab[0] + (point[1] - a[1]) * ab[1]) / length_squared)
        .clamp(0.0, 1.0);
    distance(point, [a[0] + t * ab[0], a[1] + t * ab[1]])
}

fn orientation(a: Point, b: Point, c: Point) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

pub fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    let d1 = orientation(c, d, a);
    let d2 = orientation(c, d, b);
    let d3 = orientation(a, b, c);
    let d4 = orientation(a, b, d);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

pub fn segment_segment_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    if segments_intersect(a, b, c, d) {
        return 0.0;
    }
    point_segment_distance(a, c, d)
        .min(point_segment_distance(b, c, d))
        .min(point_segment_distance(c, a, b))
        .min(point_segment_distance(d, a, b))
}

pub fn point_in_polygon(point: Point, points: &[Point]) -> bool {
    let mut inside = false;
    for (a, b) in polygon_edges(points) {
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let x = a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if point[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_are_zero_inside_and_exact_outside() {
        let circle = Shape::Circle {
            center: [0.0, 0.0],
            radius: 1.0,
        };
        assert_eq!(circle.distance_to_point([0.5, 0.0]), 0.0);
        assert!((circle.distance_to_point([3.0, 0.0]) - 2.0).abs() < 1e-12);
        assert!((circle.distance_to_segment([-5.0, 2.0], [5.0, 2.0]) - 1.0).abs() < 1e-12);

        let rectangle = Shape::rectangle([0.0, 0.0], [2.0, 1.0], 0.0);
        assert_eq!(rectangle.distance_to_point([1.9, 0.9]), 0.0);
        assert!((rectangle.distance_to_point([3.0, 0.0]) - 1.0).abs() < 1e-12);
        assert_eq!(rectangle.distance_to_segment([-5.0, 0.0], [5.0, 0.0]), 0.0);

        let rotated = Shape::rectangle([0.0, 0.0], [2.0, 1.0], 90.0);
        assert!((rotated.distance_to_point([0.0, 3.0]) - 1.0).abs() < 1e-9);

        let capsule = Shape::Capsule {
            start: [0.0, 0.0],
            end: [4.0, 0.0],
            radius: 0.5,
        };
        assert!((capsule.distance_to_segment([0.0, 2.0], [4.0, 2.0]) - 1.5).abs() < 1e-12);
    }
}

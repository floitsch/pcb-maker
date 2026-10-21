// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Exact 2D distance queries for the copper shapes a board contains.

pub type Point = [f64; 2];

/// A filled copper or keepout shape. Distances are zero inside the shape.
#[derive(Clone, Debug)]
pub enum Shape {
    Circle {
        center: Point,
        radius: f64,
    },
    /// A segment swept by a disc: tracks, oval pads, rounded edges.
    Capsule {
        start: Point,
        end: Point,
        radius: f64,
    },
    /// A simple (possibly concave) filled polygon.
    Polygon {
        points: Vec<Point>,
    },
    Union {
        parts: Vec<Shape>,
    },
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

/// The sine and cosine of an angle in degrees, exact for quarter turns: a
/// board file says 180 or -180, 270 or -90 for one angle, and the rounding
/// of `to_radians().sin_cos()` differs between them (the sign of 1e-16).
pub fn degrees_sin_cos(degrees: f64) -> (f64, f64) {
    let quarter = (degrees / 90.0).round();
    if (degrees - quarter * 90.0).abs() < 1.0e-9 {
        match (quarter as i64).rem_euclid(4) {
            0 => (0.0, 1.0),
            1 => (1.0, 0.0),
            2 => (0.0, -1.0),
            _ => (-1.0, 0.0),
        }
    } else {
        degrees.rem_euclid(360.0).to_radians().sin_cos()
    }
}

impl Shape {
    pub fn rectangle(center: Point, half_size: Point, angle_degrees: f64) -> Self {
        let (sin, cos) = degrees_sin_cos(angle_degrees);
        let corner =
            |x: f64, y: f64| [center[0] + x * cos - y * sin, center[1] + x * sin + y * cos];
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
                    for (axis, value) in point.iter().enumerate() {
                        bounds.minimum[axis] = bounds.minimum[axis].min(*value);
                        bounds.maximum[axis] = bounds.maximum[axis].max(*value);
                    }
                }
                bounds
            }
            Self::Union { parts } => {
                parts
                    .iter()
                    .map(Shape::aabb)
                    .reduce(Aabb::union)
                    .unwrap_or(Aabb {
                        minimum: [0.0; 2],
                        maximum: [0.0; 2],
                    })
            }
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

    /// Whether the whole disc of `radius` around `center` lies inside the
    /// shape (for a union: inside one of its parts, which is enough for
    /// the pads it is asked about).
    pub fn contains_disc(&self, center: Point, radius: f64) -> bool {
        match self {
            Self::Circle {
                center: c,
                radius: r,
            } => distance(center, *c) + radius <= *r,
            Self::Capsule {
                start,
                end,
                radius: r,
            } => point_segment_distance(center, *start, *end) + radius <= *r,
            Self::Polygon { points } => {
                point_in_polygon(center, points)
                    && polygon_edges(points)
                        .all(|(a, b)| point_segment_distance(center, a, b) >= radius)
            }
            Self::Union { parts } => parts.iter().any(|part| part.contains_disc(center, radius)),
        }
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
    let t =
        (((point[0] - a[0]) * ab[0] + (point[1] - a[1]) * ab[1]) / length_squared).clamp(0.0, 1.0);
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

/// A polygon of many vertices with its edges bucketed on a square grid
/// over its box, for local queries: `distance_below`, `distance` and
/// `contains` answer exactly what `Shape::distance_to_point` and
/// `point_in_polygon` answer (the same edges' tests, of fewer edges),
/// so a query near one corner of a large polygon (a panel's board
/// cutouts, a copper logo of 1589 corners) measures the edges near it,
/// not all of them. Each query returns how many cells and edges it
/// looked at (`tests`), the work it did.
#[derive(Clone, Debug)]
pub struct PolygonIndex {
    points: Vec<Point>,
    origin: Point,
    cell: f64,
    nx: usize,
    ny: usize,
    /// Per cell (row-major), the edges whose box touches it:
    /// `cell_edges[cell_start[c]..cell_start[c + 1]]`.
    cell_start: Vec<u32>,
    cell_edges: Vec<u32>,
    /// Per row (`row_height` high, from the box's bottom), the edges whose
    /// vertical extent touches it: the crossings of a horizontal ray.
    /// Finer than the cells: a long slot's sides lie in a row each.
    row_start: Vec<u32>,
    row_edges: Vec<u32>,
    row_height: f64,
    row_count: usize,
    /// The highest corner's height.
    top: f64,
}

/// Polygons of more corners than this are indexed (`PolygonIndex`).
pub const INDEXED_POLYGON_VERTICES: usize = 48;

/// Buckets of `count` lists from `(bucket, item)` pairs, as start offsets
/// and items.
fn buckets(count: usize, pairs: &[(usize, u32)]) -> (Vec<u32>, Vec<u32>) {
    let mut start = vec![0u32; count + 1];
    for (bucket, _) in pairs {
        start[bucket + 1] += 1;
    }
    for index in 0..count {
        start[index + 1] += start[index];
    }
    let mut next = start.clone();
    let mut items = vec![0u32; pairs.len()];
    for (bucket, item) in pairs {
        items[next[*bucket] as usize] = *item;
        next[*bucket] += 1;
    }
    (start, items)
}

impl PolygonIndex {
    /// The index of `points` (a polygon of at least one edge).
    pub fn new(points: &[Point]) -> Self {
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for point in points {
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(point[axis]);
                maximum[axis] = maximum[axis].max(point[axis]);
            }
        }
        let size = [
            (maximum[0] - minimum[0]).max(1.0e-6),
            (maximum[1] - minimum[1]).max(1.0e-6),
        ];
        // Cells of 0.25 mm (a clearance's size), at most 16384 of them.
        let cell = (size[0] * size[1] / 16384.0).sqrt().max(0.25);
        let nx = (size[0] / cell).floor() as usize + 1;
        let ny = (size[1] / cell).floor() as usize + 1;
        let rows = (4 * points.len()).clamp(1, 4096);
        let row_height = size[1] / rows as f64;
        let mut index = Self {
            points: points.to_vec(),
            origin: minimum,
            cell,
            nx,
            ny,
            cell_start: Vec::new(),
            cell_edges: Vec::new(),
            row_start: Vec::new(),
            row_edges: Vec::new(),
            row_height,
            row_count: rows,
            top: maximum[1],
        };
        let mut in_cells: Vec<(usize, u32)> = Vec::new();
        let mut in_rows: Vec<(usize, u32)> = Vec::new();
        for (edge, (a, b)) in polygon_edges(points).enumerate() {
            let (x0, y0) = index.clamped(a[0].min(b[0]), a[1].min(b[1]));
            let (x1, y1) = index.clamped(a[0].max(b[0]), a[1].max(b[1]));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    in_cells.push((y * nx + x, edge as u32));
                }
            }
            for row in index.row(a[1].min(b[1]))..=index.row(a[1].max(b[1])) {
                in_rows.push((row, edge as u32));
            }
        }
        (index.cell_start, index.cell_edges) = buckets(nx * ny, &in_cells);
        (index.row_start, index.row_edges) = buckets(rows, &in_rows);
        index
    }

    /// The edges put in cells and rows (the build's work).
    pub fn entries(&self) -> u64 {
        (self.cell_edges.len() + self.row_edges.len()) as u64
    }

    /// The cell of a point, clamped to the grid.
    fn clamped(&self, x: f64, y: f64) -> (usize, usize) {
        let column = ((x - self.origin[0]) / self.cell).floor();
        let row = ((y - self.origin[1]) / self.cell).floor();
        (
            column.clamp(0.0, (self.nx - 1) as f64) as usize,
            row.clamp(0.0, (self.ny - 1) as f64) as usize,
        )
    }

    /// The row of a height, clamped to the rows.
    fn row(&self, y: f64) -> usize {
        ((y - self.origin[1]) / self.row_height)
            .floor()
            .clamp(0.0, (self.row_count - 1) as f64) as usize
    }

    fn edge(&self, edge: u32) -> (Point, Point) {
        let edge = edge as usize;
        (
            self.points[edge],
            self.points[(edge + 1) % self.points.len()],
        )
    }

    /// `point_in_polygon(point, points)`, from the edges of the point's
    /// row; and the tests it did.
    pub fn contains(&self, point: Point) -> (bool, u64) {
        // Below or above every edge: no crossing.
        if !(point[1] >= self.origin[1] && point[1] <= self.top) {
            return (false, 1);
        }
        let row = self.row(point[1]);
        let edges = &self.row_edges[self.row_start[row] as usize..self.row_start[row + 1] as usize];
        let mut inside = false;
        for edge in edges {
            let (a, b) = self.edge(*edge);
            if (a[1] > point[1]) != (b[1] > point[1]) {
                let x = a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if point[0] < x {
                    inside = !inside;
                }
            }
        }
        (inside, 1 + edges.len() as u64)
    }

    /// The edges whose box meets the square of half side `reach` around
    /// `point` (each once), and the cells looked at.
    fn near(&self, point: Point, reach: f64, edges: &mut Vec<u32>) -> u64 {
        edges.clear();
        let reach = reach + 1.0e-6;
        if point[0] + reach < self.origin[0]
            || point[1] + reach < self.origin[1]
            || point[0] - reach > self.origin[0] + self.nx as f64 * self.cell
            || point[1] - reach > self.origin[1] + self.ny as f64 * self.cell
        {
            return 1;
        }
        let (x0, y0) = self.clamped(point[0] - reach, point[1] - reach);
        let (x1, y1) = self.clamped(point[0] + reach, point[1] + reach);
        for y in y0..=y1 {
            let row = y * self.nx;
            edges.extend_from_slice(
                &self.cell_edges
                    [self.cell_start[row + x0] as usize..self.cell_start[row + x1 + 1] as usize],
            );
        }
        edges.sort_unstable();
        edges.dedup();
        ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64
    }

    /// Whether `Shape::Polygon`'s `distance_to_point(point)` is below
    /// `threshold`, and the tests it did.
    pub fn distance_below(&self, point: Point, threshold: f64) -> (bool, u64) {
        let (inside, mut tests) = self.contains(point);
        if inside {
            return (0.0 < threshold, tests);
        }
        if threshold.is_nan() || threshold <= 0.0 {
            return (false, tests);
        }
        let mut edges = Vec::new();
        tests += self.near(point, threshold, &mut edges);
        tests += edges.len() as u64;
        let below = edges.iter().any(|edge| {
            let (a, b) = self.edge(*edge);
            point_segment_distance(point, a, b) < threshold
        });
        (below, tests)
    }

    /// `Shape::Polygon`'s `distance_to_point(point)` where it is below
    /// `cap`, else some distance of at least `cap`; and the tests it did:
    /// the nearest of the edges near the point (an edge outside the
    /// square of half side `cap` lies at least `cap` away).
    pub fn distance_within(&self, point: Point, cap: f64) -> (f64, u64) {
        let (inside, mut tests) = self.contains(point);
        if inside {
            return (0.0, tests);
        }
        if cap.is_nan() || cap <= 0.0 {
            return (cap.max(0.0), tests);
        }
        let mut edges = Vec::new();
        tests += self.near(point, cap, &mut edges);
        tests += edges.len() as u64;
        let least = edges
            .iter()
            .map(|edge| {
                let (a, b) = self.edge(*edge);
                point_segment_distance(point, a, b)
            })
            .fold(f64::INFINITY, f64::min);
        (if least < cap { least } else { cap }, tests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_angle_spelled_two_ways_has_one_sine_and_cosine() {
        for (a, b) in [
            (180.0, -180.0),
            (270.0, -90.0),
            (90.0, -270.0),
            (0.0, 360.0),
            (0.0, -0.0),
            (30.0, -330.0),
            (-45.0, 315.0),
        ] {
            assert_eq!(degrees_sin_cos(a), degrees_sin_cos(b), "{a} and {b}");
        }
        assert_eq!(degrees_sin_cos(90.0), (1.0, 0.0));
        assert_eq!(degrees_sin_cos(-180.0), (0.0, -1.0));
        assert_eq!(degrees_sin_cos(-90.0), (-1.0, 0.0));
        // Rectangles at 180 and -180 are the same polygon.
        let corners = |angle: f64| format!("{:?}", Shape::rectangle([1.0, 2.0], [3.0, 0.5], angle));
        assert_eq!(corners(180.0), corners(-180.0));
    }

    /// A star-shaped polygon of `corners` corners with a wavy rim, and
    /// a long slot of rounded ends (a panel's board cutout), against the
    /// plain tests: `PolygonIndex` answers what they answer.
    #[test]
    fn an_indexed_polygon_answers_as_the_plain_tests() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut noise = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let star: Vec<Point> = (0..300)
            .map(|index| {
                let angle = std::f64::consts::TAU * index as f64 / 300.0;
                let radius = 10.0 + 3.0 * (angle * 7.0).sin() + 0.5 * noise();
                [5.0 + radius * angle.cos(), -2.0 + radius * angle.sin()]
            })
            .collect();
        let mut slot: Vec<Point> = Vec::new();
        for index in 0..=40 {
            let angle = std::f64::consts::PI * (0.5 + index as f64 / 40.0);
            slot.push([angle.cos(), angle.sin()]);
        }
        for index in 0..=40 {
            let angle = std::f64::consts::PI * (1.5 + index as f64 / 40.0);
            slot.push([60.0 + angle.cos(), angle.sin()]);
        }
        for points in [star, slot] {
            let index = PolygonIndex::new(&points);
            let shape = Shape::Polygon {
                points: points.clone(),
            };
            let mut tests = 0;
            for _ in 0..20000 {
                let point = [-20.0 + 100.0 * noise(), -20.0 + 40.0 * noise()];
                let (inside, _) = index.contains(point);
                assert_eq!(inside, point_in_polygon(point, &points), "{point:?}");
                let exact = shape.distance_to_point(point);
                for threshold in [0.0, 0.05, 0.3, 1.0, 4.0] {
                    let (below, used) = index.distance_below(point, threshold);
                    tests += used;
                    assert_eq!(below, exact < threshold, "{point:?} {threshold}");
                    let (within, _) = index.distance_within(point, threshold);
                    if exact < threshold {
                        assert_eq!(within.to_bits(), exact.to_bits(), "{point:?} {threshold}");
                    } else {
                        assert!(within >= threshold, "{point:?} {threshold}");
                    }
                }
            }
            // Far fewer than the plain tests' corners a question.
            assert!(tests < 20000 * 5 * points.len() as u64 / 4, "{tests}");
        }
    }

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

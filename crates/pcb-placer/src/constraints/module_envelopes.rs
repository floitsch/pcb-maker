// Copyright (C) 2026 Florian Loitsch.

//! Pairwise geometry for bodies explicitly mounted on the same face.

use crate::problem::{Point, Pose, rotate};

#[derive(Clone, Debug)]
pub struct ModuleEnvelope {
    pub part: usize,
    /// The mounting face, independent of any through-hole pins.
    pub back: bool,
    /// Convex outline vertices in the part's local frame.
    pub vertices: Vec<Point>,
}

#[derive(Clone, Debug)]
pub struct ModuleEnvelopePair {
    pub modules: [ModuleEnvelope; 2],
    pub layer: String,
    pub min_gap_mm: f64,
}

const TOLERANCE_MM: f64 = 1.0e-6;

impl ModuleEnvelopePair {
    pub fn contains(&self, part: usize, other: usize) -> bool {
        let [a, b] = self.modules.each_ref().map(|module| module.part);
        [part, other] == [a, b] || [other, part] == [a, b]
    }

    pub fn partner(&self, part: usize) -> Option<usize> {
        let [a, b] = self.modules.each_ref().map(|module| module.part);
        if part == a {
            Some(b)
        } else {
            (part == b).then_some(a)
        }
    }

    fn outlines(&self, pose: impl Fn(usize) -> Pose) -> Option<[Vec<Point>; 2]> {
        let [first, second] = self.modules.each_ref();
        if first.back != second.back {
            return None;
        }
        let at = |module: &ModuleEnvelope| {
            let pose = pose(module.part);
            module
                .vertices
                .iter()
                .map(|&vertex| {
                    let [x, y] = rotate(vertex, -pose.angle);
                    [pose.position[0] + x, pose.position[1] + y]
                })
                .collect::<Vec<_>>()
        };
        Some([at(first), at(second)])
    }

    /// The translation needed to separate overlapping contours and the
    /// Euclidean gap between contours on the same face.
    pub fn measurement(&self, pose: impl Fn(usize) -> Pose) -> (f64, Option<f64>) {
        let Some([a, b]) = self.outlines(pose) else {
            return (0.0, None);
        };
        let depth = separating_depth(&a, &b);
        let overlap = if depth > TOLERANCE_MM { depth } else { 0.0 };
        let gap = if depth >= 0.0 { 0.0 } else { distance(&a, &b) };
        (overlap, Some(gap))
    }

    pub fn violates(&self, pose: impl Fn(usize) -> Pose) -> bool {
        self.outlines(pose).is_some_and(|[a, b]| {
            let depth = separating_depth(&a, &b);
            depth > TOLERANCE_MM
                || (self.min_gap_mm > TOLERANCE_MM
                    && (depth >= 0.0 || distance(&a, &b) + TOLERANCE_MM < self.min_gap_mm))
        })
    }

    pub(crate) fn work(&self) -> u64 {
        let edges = self
            .modules
            .iter()
            .map(|module| module.vertices.len())
            .sum::<usize>();
        (edges * edges) as u64 * if self.min_gap_mm > TOLERANCE_MM { 2 } else { 1 }
    }
}

fn separating_depth(a: &[Point], b: &[Point]) -> f64 {
    let mut closest = f64::INFINITY;
    for vertices in [a, b] {
        for (&[x, y], &[u, v]) in edges(vertices) {
            let length = (v - y).hypot(u - x);
            if length < 1.0e-9 {
                continue;
            }
            let normal = [(v - y) / length, (x - u) / length];
            let project = |polygon: &[Point]| {
                polygon
                    .iter()
                    .map(|[px, py]| px * normal[0] + py * normal[1])
                    .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], p| {
                        [lo.min(p), hi.max(p)]
                    })
            };
            let [lo_a, hi_a] = project(a);
            let [lo_b, hi_b] = project(b);
            let depth = (hi_a - lo_b).min(hi_b - lo_a);
            if depth < 0.0 {
                return depth;
            }
            closest = closest.min(depth);
        }
    }
    closest
}

fn edges(vertices: &[Point]) -> impl Iterator<Item = (&Point, &Point)> {
    vertices.iter().zip(vertices.iter().cycle().skip(1))
}

fn distance(a: &[Point], b: &[Point]) -> f64 {
    // For disjoint segments a closest pair includes at least one
    // endpoint. Check vertices in both directions for corners and edges.
    [(a, b), (b, a)]
        .into_iter()
        .flat_map(|(points, outline)| {
            points.iter().flat_map(move |&point| {
                edges(outline)
                    .map(move |(&start, &end)| super::point_segment_distance(point, start, end))
            })
        })
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_rotation_keeps_the_footprint_origin_and_kicad_angle() {
        let pair = ModuleEnvelopePair {
            modules: [
                ModuleEnvelope {
                    part: 0,
                    back: true,
                    vertices: vec![[1.0, 0.0], [5.0, 0.0], [5.0, 1.0], [1.0, 1.0]],
                },
                ModuleEnvelope {
                    part: 1,
                    back: true,
                    vertices: vec![[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]],
                },
            ],
            layer: "Eco2.User".into(),
            min_gap_mm: 0.0,
        };
        let at = |angle| {
            [
                Pose {
                    position: [10.0, 10.0],
                    angle,
                },
                Pose {
                    position: [10.5, 7.0],
                    angle: 0.0,
                },
            ]
        };
        assert_eq!(pair.measurement(|part| at(90.0)[part]), (1.0, Some(0.0)));
        assert_eq!(pair.measurement(|part| at(-90.0)[part]).0, 0.0);
    }

    #[test]
    fn module_gap_is_euclidean_even_at_corners() {
        let module = |part| ModuleEnvelope {
            part,
            back: false,
            vertices: vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
        };
        let mut pair = ModuleEnvelopePair {
            modules: [module(0), module(1)],
            layer: "Eco2.User".into(),
            min_gap_mm: 5.0,
        };
        let pose = |part| Pose {
            position: [[0.0, 0.0], [5.0, 6.0]][part],
            angle: 0.0,
        };
        assert_eq!(pair.measurement(pose), (0.0, Some(5.0)));
        assert!(!pair.violates(pose));
        pair.min_gap_mm = 5.1;
        assert!(pair.violates(pose));
        pair.modules[1].back = true;
        assert_eq!(pair.measurement(pose), (0.0, None));
        assert!(!pair.violates(pose));
    }
}

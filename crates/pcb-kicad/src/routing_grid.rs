// Copyright (C) 2026 Toit contributors.

/// Per-request containment cache. Copper obstacles and via rules are absent.
pub(super) struct OutlinePointCache<'a> {
    outline: &'a crate::BoardOutline,
    width: usize,
    origin: [f64; 2],
    resolution: f64,
    clearance: f64,
    inside: Vec<bool>,
}

impl<'a> OutlinePointCache<'a> {
    pub(super) fn new(
        outline: &'a crate::BoardOutline,
        width: usize,
        height: usize,
        origin: [f64; 2],
        resolution: f64,
        clearance: f64,
    ) -> Self {
        let mut result = Self {
            outline,
            width,
            origin,
            resolution,
            clearance,
            inside: Vec::with_capacity(width * height),
        };
        for y in 0..height {
            for x in 0..width {
                result
                    .inside
                    .push(outline.contains_point_with_clearance(result.at([x, y]), clearance));
            }
        }
        result
    }

    pub(super) fn at(&self, point: [usize; 2]) -> [f64; 2] {
        [
            self.origin[0] + point[0] as f64 * self.resolution,
            self.origin[1] + point[1] as f64 * self.resolution,
        ]
    }

    pub(super) fn contains_segment(&self, start: [usize; 2], end: [usize; 2]) -> bool {
        self.inside[start[1] * self.width + start[0]]
            && self.inside[end[1] * self.width + end[0]]
            && self
                .outline
                .segment_clears_boundary(self.at(start), self.at(end), self.clearance)
    }
}

pub(super) fn edge_mask_at(
    mask: &[bool],
    width: usize,
    start: [usize; 2],
    end: [usize; 2],
) -> bool {
    let direction = match (
        end[0] as isize - start[0] as isize,
        end[1] as isize - start[1] as isize,
    ) {
        (1, 0) => 0,
        (-1, 0) => 1,
        (0, 1) => 2,
        (0, -1) => 3,
        (1, 1) => 4,
        (1, -1) => 5,
        (-1, 1) => 6,
        (-1, -1) => 7,
        _ => panic!("non-adjacent planar edge"),
    };
    mask[(start[1] * width + start[0]) * 8 + direction]
}

#[cfg(test)]
mod outline_cache_tests {
    use super::*;

    #[test]
    fn cached_masks_match_original_for_concave_rotated_and_boundary_cases() {
        let outlines = [
            vec![[0., 0.], [4., 0.], [4., 4.], [0., 4.]],
            vec![[0., 0.], [4., 0.], [4., 4.], [2., 2.], [0., 4.]],
            vec![[0., 2.], [2., 0.], [4., 2.], [2., 4.]],
            vec![[0., 0.], [2., 0.], [4., 0.], [4., 4.], [0., 4.]],
        ];
        for points in outlines {
            let outline = crate::BoardOutline {
                points,
                bounds: [0., 0., 4., 4.],
            };
            for (width, height, origin, resolution) in [
                (23, 23, [-0.2, -0.2], 0.2),
                (19, 19, [0.1, -0.15], 0.25),
                (7, 7, [1.95, 1.95], 0.1),
                (1, 23, [0., 0.], 0.2),
                (23, 1, [0., 0.], 0.2),
            ] {
                for clearance in [0.0, 0.1, 0.2, 0.20000000000000004] {
                    let cache = OutlinePointCache::new(
                        &outline, width, height, origin, resolution, clearance,
                    );
                    let original =
                        planar_transition_mask(width, height, 2, |_layer, start, end| {
                            !outline.contains_segment_with_clearance(
                                cache.at(start),
                                cache.at(end),
                                clearance,
                            )
                        });
                    let endpoints =
                        planar_transition_mask(width, height, 2, |_layer, start, end| {
                            !cache.contains_segment(start, end)
                        });
                    let one_layer =
                        planar_transition_mask(width, height, 1, |_layer, start, end| {
                            !cache.contains_segment(start, end)
                        });
                    let layers = planar_transition_mask(width, height, 2, |_layer, start, end| {
                        edge_mask_at(&one_layer, width, start, end)
                    });
                    assert_eq!(endpoints, original);
                    assert_eq!(layers, original);
                }
            }
        }
    }

    #[test]
    fn cached_inside_endpoints_do_not_skip_concave_boundary_crossing() {
        let outline = crate::BoardOutline {
            bounds: [0., 0., 4., 4.],
            points: vec![[0., 0.], [4., 0.], [4., 4.], [2., 2.], [0., 4.]],
        };
        let cache = OutlinePointCache::new(&outline, 2, 1, [0.5, 3.], 3., 0.1);
        assert!(cache.inside.iter().all(|inside| *inside));
        assert!(!cache.contains_segment([0, 0], [1, 0]));
    }
}

/// Rasterize undirected geometric edges once, retaining the grid router's
/// directed transition layout. This is only for symmetric collision checks;
/// directional traversal penalties do not belong in this mask.
pub(super) fn planar_transition_mask(
    width: usize,
    height: usize,
    layers: usize,
    mut blocked: impl FnMut(usize, [usize; 2], [usize; 2]) -> bool,
) -> Vec<bool> {
    let plane = width * height;
    let mut result = vec![false; plane * layers * 8];
    // Forward/reverse indices match pcb-grid-router's eight planar directions.
    for layer in 0..layers {
        for y in 0..height {
            for x in 0..width {
                let state = layer * plane + y * width + x;
                for (direction, reverse, dx, dy) in
                    [(0, 1, 1, 0), (2, 3, 0, 1), (4, 7, 1, 1), (5, 6, 1, -1)]
                {
                    let Some(nx) = x.checked_add_signed(dx) else {
                        continue;
                    };
                    let Some(ny) = y.checked_add_signed(dy) else {
                        continue;
                    };
                    if nx >= width || ny >= height {
                        continue;
                    }
                    let collision = blocked(layer, [x, y], [nx, ny]);
                    result[state * 8 + direction] = collision;
                    let next = layer * plane + ny * width + nx;
                    result[next * 8 + reverse] = collision;
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KiCadObstacleGrid, KiCadRoutingModel, parse};

    #[test]
    fn symmetric_mask_matches_independent_directed_geometry_checks() {
        let pcb = parse(
            r#"(kicad_pcb
          (gr_rect (start 0 0) (end 10 8) (layer "Edge.Cuts"))
          (footprint "A" (at 1 1)
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/TARGET")))
          (footprint "B" (at 9 7)
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "B.Cu") (net "/TARGET")))
          (footprint "C" (at 3 3 30) (clearance 0.4)
            (pad "1" smd rect (at 0 0) (size 1.5 0.9) (layers "F.Cu") (net "/OTHER")))
          (footprint "D" (at 6 3 45)
            (pad "1" smd oval (at 0 0) (size 2 0.8) (layers "B.Cu") (net "/OTHER")))
          (footprint "H" (at 5 5)
            (pad "" np_thru_hole circle (at 0 0) (size 1 1) (drill 1) (layers "*.Cu" "*.Mask"))))"#,
        )
        .unwrap();
        let model = KiCadRoutingModel::from_pcb(&pcb, "TARGET").unwrap();
        for (width, height, resolution, clearance) in [
            (51, 41, 0.2, 0.1),
            (41, 33, 0.25, 0.2),
            (1, 41, 0.2, 0.1),
            (51, 1, 0.2, 0.1),
        ] {
            let grid = KiCadObstacleGrid::new(&model, 1.0, clearance);
            let collides = |layer, start: [usize; 2], end: [usize; 2]| {
                let at = |p: [usize; 2]| [p[0] as f64 * resolution, p[1] as f64 * resolution];
                let (start, end) = (at(start), at(end));
                !model
                    .outline
                    .contains_segment_with_clearance(start, end, 0.125)
                    || grid.any_intersects_segment(&model, layer, start, end, 0.075 + clearance)
            };
            let mut optimized_queries = 0;
            let actual = planar_transition_mask(width, height, 2, |layer, start, end| {
                optimized_queries += 1;
                collides(layer, start, end)
            });
            let cache = OutlinePointCache::new(
                &model.outline,
                width,
                height,
                [0.0, 0.0],
                resolution,
                0.125,
            );
            let outline_mask = planar_transition_mask(width, height, 1, |_layer, start, end| {
                !cache.contains_segment(start, end)
            });
            let cached = planar_transition_mask(width, height, 2, |layer, start, end| {
                edge_mask_at(&outline_mask, width, start, end)
                    || grid.any_intersects_segment(
                        &model,
                        layer,
                        cache.at(start),
                        cache.at(end),
                        0.075 + clearance,
                    )
            });
            assert_eq!(cached, actual);
            let mut expected = vec![false; width * height * 2 * 8];
            let mut directed_queries = 0;
            for layer in 0..2 {
                for y in 0..height {
                    for x in 0..width {
                        for (direction, (dx, dy)) in [
                            (1, 0),
                            (-1, 0),
                            (0, 1),
                            (0, -1),
                            (1, 1),
                            (1, -1),
                            (-1, 1),
                            (-1, -1),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let Some(nx) = x.checked_add_signed(dx) else {
                                continue;
                            };
                            let Some(ny) = y.checked_add_signed(dy) else {
                                continue;
                            };
                            if nx >= width || ny >= height {
                                continue;
                            }
                            directed_queries += 1;
                            expected[((layer * height + y) * width + x) * 8 + direction] =
                                collides(layer, [x, y], [nx, ny]);
                        }
                    }
                }
            }
            assert_eq!(
                actual, expected,
                "{width} x {height}, clearance {clearance}"
            );
            assert_eq!(optimized_queries * 2, directed_queries);
        }
    }
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Exact-boundary classification for a pair of fixed copper witnesses.
//!
//! This module deliberately performs an exhaustive segment-pair comparison.
//! It is intended as a small correctness primitive; callers that need spatial
//! indexing can put one in front of this classifier without changing its proof
//! semantics.

use std::cmp::Ordering;
use std::error::Error;
use std::fmt;

use crate::geometry::{EPSILON, segment_distance, segments_intersect};
use crate::model::Vec2;

/// One fixed, same-layer route witness and its physical copper requirements.
#[derive(Clone, Copy, Debug)]
pub struct FixedCopperRoute<'a> {
    pub polyline: &'a [Vec2],
    pub width: f64,
    /// Required empty space outside this route's copper boundary.
    pub clearance: f64,
}

/// Stable evidence for the closest decisive segment pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CopperPairEvidence {
    pub first_segment_index: usize,
    pub second_segment_index: usize,
    pub minimum_centerline_distance: f64,
    pub required_centerline_distance: f64,
    pub segment_pairs_analyzed: usize,
}

/// Mutually exclusive physical relationship between two fixed witnesses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FixedCopperPairClassification {
    Clear(CopperPairEvidence),
    CenterlineIntersection(CopperPairEvidence),
    /// The centerlines do not intersect, but their positive separation is less
    /// than the widths and per-route clearances require.
    ClearanceShortfall(CopperPairEvidence),
}

impl FixedCopperPairClassification {
    pub fn evidence(self) -> CopperPairEvidence {
        match self {
            Self::Clear(evidence)
            | Self::CenterlineIntersection(evidence)
            | Self::ClearanceShortfall(evidence) => evidence,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopperPairRouteSide {
    First,
    Second,
}

/// Fail-closed error while validating one reusable fixed route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedCopperRoutePreparationError {
    EmptyPolyline,
    NonFinitePoint { point_index: usize },
    NonFiniteWidth,
    NonPositiveWidth,
    NonFiniteClearance,
    NegativeClearance,
}

impl FixedCopperRoutePreparationError {
    fn at_side(self, route: CopperPairRouteSide) -> FixedCopperPairError {
        match self {
            Self::EmptyPolyline => FixedCopperPairError::EmptyPolyline { route },
            Self::NonFinitePoint { point_index } => {
                FixedCopperPairError::NonFinitePoint { route, point_index }
            }
            Self::NonFiniteWidth => FixedCopperPairError::NonFiniteWidth { route },
            Self::NonPositiveWidth => FixedCopperPairError::NonPositiveWidth { route },
            Self::NonFiniteClearance => FixedCopperPairError::NonFiniteClearance { route },
            Self::NegativeClearance => FixedCopperPairError::NegativeClearance { route },
        }
    }
}

impl fmt::Display for FixedCopperRoutePreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPolyline => write!(formatter, "polyline is empty"),
            Self::NonFinitePoint { point_index } => {
                write!(formatter, "polyline point {point_index} is non-finite")
            }
            Self::NonFiniteWidth => write!(formatter, "copper width is non-finite"),
            Self::NonPositiveWidth => write!(formatter, "copper width is not positive"),
            Self::NonFiniteClearance => write!(formatter, "clearance is non-finite"),
            Self::NegativeClearance => write!(formatter, "clearance is negative"),
        }
    }
}

impl Error for FixedCopperRoutePreparationError {}

/// Fail-closed input or arithmetic error from copper-pair classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedCopperPairError {
    EmptyPolyline {
        route: CopperPairRouteSide,
    },
    NonFinitePoint {
        route: CopperPairRouteSide,
        point_index: usize,
    },
    NonFiniteWidth {
        route: CopperPairRouteSide,
    },
    NonPositiveWidth {
        route: CopperPairRouteSide,
    },
    NonFiniteClearance {
        route: CopperPairRouteSide,
    },
    NegativeClearance {
        route: CopperPairRouteSide,
    },
    RequiredDistanceOverflow,
    NonFiniteGeometry,
}

impl fmt::Display for FixedCopperPairError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPolyline { route } => write!(formatter, "{route:?} polyline is empty"),
            Self::NonFinitePoint { route, point_index } => write!(
                formatter,
                "{route:?} polyline point {point_index} is non-finite"
            ),
            Self::NonFiniteWidth { route } => {
                write!(formatter, "{route:?} copper width is non-finite")
            }
            Self::NonPositiveWidth { route } => {
                write!(formatter, "{route:?} copper width is not positive")
            }
            Self::NonFiniteClearance { route } => {
                write!(formatter, "{route:?} clearance is non-finite")
            }
            Self::NegativeClearance { route } => {
                write!(formatter, "{route:?} clearance is negative")
            }
            Self::RequiredDistanceOverflow => {
                write!(formatter, "required centerline distance overflowed")
            }
            Self::NonFiniteGeometry => {
                write!(formatter, "segment distance computation was non-finite")
            }
        }
    }
}

impl Error for FixedCopperPairError {}

#[derive(Clone, Copy, Debug)]
struct Segment {
    index: usize,
    start: Vec2,
    end: Vec2,
}

/// A validated fixed witness with an immutable, reusable segment table.
///
/// Preparation takes `O(points)` time and space and performs the only segment
/// allocation. Pair classification borrows two prepared segment tables, takes
/// `O(first_segments * second_segments)` time and `O(1)` auxiliary space, and
/// performs no per-classification segment allocation.
#[derive(Debug)]
pub struct PreparedFixedCopperRoute<'a> {
    route: FixedCopperRoute<'a>,
    segments: Box<[Segment]>,
}

impl<'a> PreparedFixedCopperRoute<'a> {
    /// The exact borrowed source slice and immutable physical requirements.
    pub fn route(&self) -> FixedCopperRoute<'a> {
        self.route
    }

    pub fn polyline(&self) -> &'a [Vec2] {
        self.route.polyline
    }

    pub fn width(&self) -> f64 {
        self.route.width
    }

    pub fn clearance(&self) -> f64 {
        self.route.clearance
    }

    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    first: Segment,
    second: Segment,
    distance: f64,
}

/// Classify two fixed same-layer copper witnesses.
///
/// The required centerline distance is
/// `first.width / 2 + second.width / 2 + max(first.clearance, second.clearance)`.
/// Clearance denotes the copper-to-copper gap, rather than a halo contributed
/// independently by both routes. Exact threshold equality is clear. A
/// single-point polyline is treated as one zero-length segment; explicit
/// zero-length segments are retained as well.
pub fn classify_fixed_copper_pair(
    first: FixedCopperRoute<'_>,
    second: FixedCopperRoute<'_>,
) -> Result<FixedCopperPairClassification, FixedCopperPairError> {
    let first = prepare_fixed_copper_route(first)
        .map_err(|error| error.at_side(CopperPairRouteSide::First))?;
    let second = prepare_fixed_copper_route(second)
        .map_err(|error| error.at_side(CopperPairRouteSide::Second))?;
    classify_prepared_copper_pair(&first, &second)
}

/// Validate and segmentize one fixed witness for repeated pair classification.
pub fn prepare_fixed_copper_route<'a>(
    route: FixedCopperRoute<'a>,
) -> Result<PreparedFixedCopperRoute<'a>, FixedCopperRoutePreparationError> {
    validate_route(route)?;
    Ok(PreparedFixedCopperRoute {
        route,
        segments: segments(route.polyline).into_boxed_slice(),
    })
}

/// Classify two already prepared same-layer copper witnesses.
///
/// Results and threshold semantics are identical to
/// [`classify_fixed_copper_pair`]. Preparation has already rejected invalid
/// route inputs; this call can still fail closed if combining physical values
/// overflows or finite coordinates produce non-finite geometry arithmetic.
pub fn classify_prepared_copper_pair(
    first: &PreparedFixedCopperRoute<'_>,
    second: &PreparedFixedCopperRoute<'_>,
) -> Result<FixedCopperPairClassification, FixedCopperPairError> {
    let first_route = first.route;
    let second_route = second.route;

    let required_distance = first_route.width * 0.5
        + second_route.width * 0.5
        + first_route.clearance.max(second_route.clearance);
    if !required_distance.is_finite() {
        return Err(FixedCopperPairError::RequiredDistanceOverflow);
    }

    let segment_pairs_analyzed = first
        .segments
        .len()
        .checked_mul(second.segments.len())
        .ok_or(FixedCopperPairError::RequiredDistanceOverflow)?;
    let mut closest = None;
    let mut intersection = None;

    for first_segment in first.segments.iter() {
        for second_segment in second.segments.iter() {
            let intersects = segments_intersect(
                first_segment.start,
                first_segment.end,
                second_segment.start,
                second_segment.end,
            );
            let distance = if intersects {
                0.0
            } else {
                segment_distance(
                    first_segment.start,
                    first_segment.end,
                    second_segment.start,
                    second_segment.end,
                )
            };
            if !distance.is_finite() {
                return Err(FixedCopperPairError::NonFiniteGeometry);
            }
            let candidate = Candidate {
                first: *first_segment,
                second: *second_segment,
                distance,
            };
            retain_better(&mut closest, candidate);
            if intersects {
                retain_better(&mut intersection, candidate);
            }
        }
    }

    if let Some(candidate) = intersection {
        return Ok(FixedCopperPairClassification::CenterlineIntersection(
            evidence(candidate, required_distance, segment_pairs_analyzed),
        ));
    }

    let closest = closest.expect("validated polylines always yield at least one segment pair");
    let evidence = evidence(closest, required_distance, segment_pairs_analyzed);
    if closest.distance + EPSILON < required_distance {
        Ok(FixedCopperPairClassification::ClearanceShortfall(evidence))
    } else {
        Ok(FixedCopperPairClassification::Clear(evidence))
    }
}

fn validate_route(route: FixedCopperRoute<'_>) -> Result<(), FixedCopperRoutePreparationError> {
    if route.polyline.is_empty() {
        return Err(FixedCopperRoutePreparationError::EmptyPolyline);
    }
    for (point_index, point) in route.polyline.iter().enumerate() {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(FixedCopperRoutePreparationError::NonFinitePoint { point_index });
        }
    }
    if !route.width.is_finite() {
        return Err(FixedCopperRoutePreparationError::NonFiniteWidth);
    }
    if route.width <= 0.0 {
        return Err(FixedCopperRoutePreparationError::NonPositiveWidth);
    }
    if !route.clearance.is_finite() {
        return Err(FixedCopperRoutePreparationError::NonFiniteClearance);
    }
    if route.clearance < 0.0 {
        return Err(FixedCopperRoutePreparationError::NegativeClearance);
    }
    Ok(())
}

fn segments(polyline: &[Vec2]) -> Vec<Segment> {
    if polyline.len() == 1 {
        return vec![Segment {
            index: 0,
            start: polyline[0],
            end: polyline[0],
        }];
    }
    polyline
        .windows(2)
        .enumerate()
        .map(|(index, points)| Segment {
            index,
            start: points[0],
            end: points[1],
        })
        .collect()
}

fn evidence(
    candidate: Candidate,
    required_centerline_distance: f64,
    segment_pairs_analyzed: usize,
) -> CopperPairEvidence {
    CopperPairEvidence {
        first_segment_index: candidate.first.index,
        second_segment_index: candidate.second.index,
        minimum_centerline_distance: candidate.distance,
        required_centerline_distance,
        segment_pairs_analyzed,
    }
}

fn retain_better(best: &mut Option<Candidate>, candidate: Candidate) {
    if best.as_ref().map_or(true, |current| {
        compare_candidates(candidate, *current).is_lt()
    }) {
        *best = Some(candidate);
    }
}

fn compare_candidates(left: Candidate, right: Candidate) -> Ordering {
    left.distance
        .total_cmp(&right.distance)
        .then_with(|| compare_pair_key(left, right))
}

/// Geometric pair ordering makes tie resolution independent of route order and
/// segment-pair traversal order. Indices only disambiguate duplicate geometry.
fn compare_pair_key(left: Candidate, right: Candidate) -> Ordering {
    let (left_low, left_high) = ordered_segment_pair(left.first, left.second);
    let (right_low, right_high) = ordered_segment_pair(right.first, right.second);
    compare_segment_geometry(left_low, right_low)
        .then_with(|| compare_segment_geometry(left_high, right_high))
        .then_with(|| {
            let left_indices = ordered_indices(left.first.index, left.second.index);
            let right_indices = ordered_indices(right.first.index, right.second.index);
            left_indices.cmp(&right_indices)
        })
}

fn ordered_segment_pair(first: Segment, second: Segment) -> (Segment, Segment) {
    if compare_segment_geometry(first, second).is_gt() {
        (second, first)
    } else {
        (first, second)
    }
}

fn compare_segment_geometry(left: Segment, right: Segment) -> Ordering {
    let (left_start, left_end) = ordered_points(left.start, left.end);
    let (right_start, right_end) = ordered_points(right.start, right.end);
    compare_point(left_start, right_start).then_with(|| compare_point(left_end, right_end))
}

fn ordered_points(first: Vec2, second: Vec2) -> (Vec2, Vec2) {
    if compare_point(first, second).is_gt() {
        (second, first)
    } else {
        (first, second)
    }
}

fn compare_point(left: Vec2, right: Vec2) -> Ordering {
    left.x
        .total_cmp(&right.x)
        .then_with(|| left.y.total_cmp(&right.y))
}

fn ordered_indices(first: usize, second: usize) -> (usize, usize) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(points: &[Vec2]) -> FixedCopperRoute<'_> {
        FixedCopperRoute {
            polyline: points,
            width: 0.2,
            clearance: 0.1,
        }
    }

    fn point(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn crossing_is_centerline_intersection() {
        let first = [point(-1.0, 0.0), point(1.0, 0.0)];
        let second = [point(0.0, -1.0), point(0.0, 1.0)];
        let result = classify_fixed_copper_pair(route(&first), route(&second)).unwrap();
        let FixedCopperPairClassification::CenterlineIntersection(evidence) = result else {
            panic!("expected crossing, got {result:?}");
        };
        assert_eq!(evidence.first_segment_index, 0);
        assert_eq!(evidence.second_segment_index, 0);
        assert_eq!(evidence.minimum_centerline_distance, 0.0);
        assert!((evidence.required_centerline_distance - 0.3).abs() < EPSILON);
    }

    #[test]
    fn close_parallel_routes_have_positive_clearance_shortfall() {
        let first = [point(0.0, 0.0), point(2.0, 0.0)];
        let second = [point(0.0, 0.2), point(2.0, 0.2)];
        let result = classify_fixed_copper_pair(route(&first), route(&second)).unwrap();
        let FixedCopperPairClassification::ClearanceShortfall(evidence) = result else {
            panic!("expected shortfall, got {result:?}");
        };
        assert!((evidence.minimum_centerline_distance - 0.2).abs() < EPSILON);
        assert!(evidence.minimum_centerline_distance > 0.0);
    }

    #[test]
    fn exactly_at_threshold_is_clear() {
        let first = [point(0.0, 0.0), point(2.0, 0.0)];
        let second = [point(0.0, 0.3), point(2.0, 0.3)];
        let result = classify_fixed_copper_pair(route(&first), route(&second)).unwrap();
        assert!(matches!(result, FixedCopperPairClassification::Clear(_)));
    }

    #[test]
    fn well_separated_routes_are_clear() {
        let first = [point(0.0, 0.0), point(2.0, 0.0)];
        let second = [point(0.0, 1.0), point(2.0, 1.0)];
        let result = classify_fixed_copper_pair(route(&first), route(&second)).unwrap();
        let FixedCopperPairClassification::Clear(evidence) = result else {
            panic!("expected clear, got {result:?}");
        };
        assert_eq!(evidence.minimum_centerline_distance, 1.0);
    }

    #[test]
    fn unequal_clearances_use_one_conservative_gap() {
        let first_points = [point(0.0, 0.0), point(2.0, 0.0)];
        let second_points = [point(0.0, 0.5), point(2.0, 0.5)];
        let first = FixedCopperRoute {
            polyline: &first_points,
            width: 0.2,
            clearance: 0.1,
        };
        let second = FixedCopperRoute {
            polyline: &second_points,
            width: 0.4,
            clearance: 0.2,
        };
        let forward = classify_fixed_copper_pair(first, second).unwrap();
        let reverse = classify_fixed_copper_pair(second, first).unwrap();
        assert!(matches!(forward, FixedCopperPairClassification::Clear(_)));
        assert!(matches!(reverse, FixedCopperPairClassification::Clear(_)));
        assert!((forward.evidence().required_centerline_distance - 0.5).abs() < EPSILON);
        assert_eq!(
            forward.evidence().required_centerline_distance,
            reverse.evidence().required_centerline_distance
        );
    }

    #[test]
    fn endpoint_touch_is_centerline_intersection() {
        let first = [point(0.0, 0.0), point(1.0, 0.0)];
        let second = [point(1.0, 0.0), point(2.0, 1.0)];
        let result = classify_fixed_copper_pair(route(&first), route(&second)).unwrap();
        assert!(matches!(
            result,
            FixedCopperPairClassification::CenterlineIntersection(_)
        ));
    }

    #[test]
    fn zero_length_segment_is_classified() {
        let first = [point(0.0, 0.0), point(0.0, 0.0)];
        let second = [point(-1.0, 0.0), point(1.0, 0.0)];
        let result = classify_fixed_copper_pair(route(&first), route(&second)).unwrap();
        assert!(matches!(
            result,
            FixedCopperPairClassification::CenterlineIntersection(_)
        ));

        let single_point = [point(0.0, 0.2)];
        let result = classify_fixed_copper_pair(route(&single_point), route(&second)).unwrap();
        assert!(matches!(
            result,
            FixedCopperPairClassification::ClearanceShortfall(_)
        ));
    }

    #[test]
    fn swapping_routes_swaps_evidence_indices() {
        let first = [point(-2.0, 4.0), point(-1.0, 4.0), point(-1.0, 0.0)];
        let second = [point(0.0, 3.0), point(0.0, 1.0)];
        let forward = classify_fixed_copper_pair(route(&first), route(&second)).unwrap();
        let reverse = classify_fixed_copper_pair(route(&second), route(&first)).unwrap();
        assert_eq!(
            std::mem::discriminant(&forward),
            std::mem::discriminant(&reverse)
        );
        let forward = forward.evidence();
        let reverse = reverse.evidence();
        assert_eq!(forward.first_segment_index, reverse.second_segment_index);
        assert_eq!(forward.second_segment_index, reverse.first_segment_index);
        assert_eq!(
            forward.minimum_centerline_distance,
            reverse.minimum_centerline_distance
        );
        assert_eq!(
            forward.required_centerline_distance,
            reverse.required_centerline_distance
        );
    }

    #[test]
    fn witness_reversal_selects_same_geometric_pair() {
        let first = [point(-2.0, 4.0), point(-1.0, 4.0), point(-1.0, 0.0)];
        let second = [point(0.0, 3.0), point(0.0, 1.0)];
        let original = classify_fixed_copper_pair(route(&first), route(&second))
            .unwrap()
            .evidence();
        let mut reversed_first = first;
        let mut reversed_second = second;
        reversed_first.reverse();
        reversed_second.reverse();
        let reversed = classify_fixed_copper_pair(route(&reversed_first), route(&reversed_second))
            .unwrap()
            .evidence();
        assert_eq!(
            reversed.first_segment_index,
            first.len() - 2 - original.first_segment_index
        );
        assert_eq!(
            reversed.second_segment_index,
            second.len() - 2 - original.second_segment_index
        );
        assert_eq!(
            reversed.minimum_centerline_distance,
            original.minimum_centerline_distance
        );
    }

    #[test]
    fn invalid_physical_inputs_are_rejected() {
        let points = [point(0.0, 0.0)];
        let valid = route(&points);
        assert_eq!(
            classify_fixed_copper_pair(
                FixedCopperRoute {
                    width: -0.1,
                    ..valid
                },
                valid
            ),
            Err(FixedCopperPairError::NonPositiveWidth {
                route: CopperPairRouteSide::First
            })
        );
        assert_eq!(
            classify_fixed_copper_pair(
                valid,
                FixedCopperRoute {
                    clearance: -0.1,
                    ..valid
                }
            ),
            Err(FixedCopperPairError::NegativeClearance {
                route: CopperPairRouteSide::Second
            })
        );
        assert_eq!(
            classify_fixed_copper_pair(
                FixedCopperRoute {
                    width: f64::INFINITY,
                    ..valid
                },
                valid
            ),
            Err(FixedCopperPairError::NonFiniteWidth {
                route: CopperPairRouteSide::First
            })
        );
        assert_eq!(
            classify_fixed_copper_pair(
                valid,
                FixedCopperRoute {
                    clearance: f64::NAN,
                    ..valid
                }
            ),
            Err(FixedCopperPairError::NonFiniteClearance {
                route: CopperPairRouteSide::Second
            })
        );
        assert_eq!(
            classify_fixed_copper_pair(
                FixedCopperRoute {
                    width: 0.0,
                    ..valid
                },
                valid
            ),
            Err(FixedCopperPairError::NonPositiveWidth {
                route: CopperPairRouteSide::First
            })
        );
    }

    #[test]
    fn non_finite_point_and_empty_polyline_are_rejected() {
        let finite = [point(0.0, 0.0)];
        let non_finite = [point(f64::NAN, 0.0)];
        assert_eq!(
            classify_fixed_copper_pair(route(&non_finite), route(&finite)),
            Err(FixedCopperPairError::NonFinitePoint {
                route: CopperPairRouteSide::First,
                point_index: 0
            })
        );
        assert_eq!(
            classify_fixed_copper_pair(route(&[]), route(&finite)),
            Err(FixedCopperPairError::EmptyPolyline {
                route: CopperPairRouteSide::First
            })
        );
    }

    #[test]
    fn prepared_and_convenience_paths_are_exactly_equivalent() {
        let cases = [
            (
                vec![point(-1.0, 0.0), point(1.0, 0.0)],
                vec![point(0.0, -1.0), point(0.0, 1.0)],
            ),
            (
                vec![point(0.0, 0.0), point(2.0, 0.0)],
                vec![point(0.0, 0.2), point(2.0, 0.2)],
            ),
            (
                vec![point(0.0, 0.0), point(1.0, 0.0), point(2.0, 0.0)],
                vec![point(0.0, 1.0), point(2.0, 1.0)],
            ),
        ];
        for (first_points, second_points) in cases {
            let first_route = route(&first_points);
            let second_route = route(&second_points);
            let convenience = classify_fixed_copper_pair(first_route, second_route).unwrap();
            let first = prepare_fixed_copper_route(first_route).unwrap();
            let second = prepare_fixed_copper_route(second_route).unwrap();
            assert_eq!(
                classify_prepared_copper_pair(&first, &second).unwrap(),
                convenience
            );
            assert_eq!(first.route().polyline, first_route.polyline);
            assert_eq!(first.width(), first_route.width);
            assert_eq!(first.clearance(), first_route.clearance);
            assert!(std::ptr::eq(first.polyline(), first_route.polyline));
        }
    }

    #[test]
    fn prepared_segments_are_reused_across_many_classifications() {
        let first_points = [point(-2.0, 0.0), point(0.0, 0.0), point(2.0, 0.0)];
        let near_points = [point(-2.0, 0.2), point(2.0, 0.2)];
        let far_points = [point(-2.0, 1.0), point(2.0, 1.0)];
        let first = prepare_fixed_copper_route(route(&first_points)).unwrap();
        let near = prepare_fixed_copper_route(route(&near_points)).unwrap();
        let far = prepare_fixed_copper_route(route(&far_points)).unwrap();
        let first_storage = first.segments.as_ptr();
        let near_storage = near.segments.as_ptr();
        let expected_near = classify_prepared_copper_pair(&first, &near).unwrap();
        let expected_far = classify_prepared_copper_pair(&first, &far).unwrap();

        for _ in 0..32 {
            assert_eq!(
                classify_prepared_copper_pair(&first, &near).unwrap(),
                expected_near
            );
            assert_eq!(
                classify_prepared_copper_pair(&first, &far).unwrap(),
                expected_far
            );
            assert_eq!(first.segments.as_ptr(), first_storage);
            assert_eq!(near.segments.as_ptr(), near_storage);
        }
        assert_eq!(first.segment_count(), 2);
        assert_eq!(near.segment_count(), 1);
    }

    #[test]
    fn invalid_route_is_rejected_during_preparation() {
        let finite = [point(0.0, 0.0)];
        let non_finite = [point(0.0, f64::INFINITY)];
        assert_eq!(
            prepare_fixed_copper_route(route(&[])).unwrap_err(),
            FixedCopperRoutePreparationError::EmptyPolyline
        );
        assert_eq!(
            prepare_fixed_copper_route(route(&non_finite)).unwrap_err(),
            FixedCopperRoutePreparationError::NonFinitePoint { point_index: 0 }
        );
        assert_eq!(
            prepare_fixed_copper_route(FixedCopperRoute {
                polyline: &finite,
                width: -0.1,
                clearance: 0.1,
            })
            .unwrap_err(),
            FixedCopperRoutePreparationError::NonPositiveWidth
        );
        assert_eq!(
            prepare_fixed_copper_route(FixedCopperRoute {
                polyline: &finite,
                width: 0.2,
                clearance: f64::NAN,
            })
            .unwrap_err(),
            FixedCopperRoutePreparationError::NonFiniteClearance
        );
    }
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Placement constraints: what the user (often an agent) wants besides short
//! wires.
//!
//! Hard constraints restrict where a single part may be: along a board edge
//! or inside a region. They are part of legality, so every stage (global
//! placement's clamp, annealing, legalization, refinement, router-driven
//! moves) keeps them.
//!
//! Relations tie a part to another part or to a pin: near it, or on one side
//! of it. They are soft: a penalty in millimetres of violation, weighted
//! against wirelength, and a force in global placement. The report says for
//! each constraint whether it holds and by how much it is missed.

use crate::problem::{Point, Pose, Problem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    /// Smaller y (KiCad's y axis points down).
    Top,
    Bottom,
}

impl Edge {
    pub fn name(self) -> &'static str {
        match self {
            Edge::Left => "left",
            Edge::Right => "right",
            Edge::Top => "top",
            Edge::Bottom => "bottom",
        }
    }
}

/// What a relation measures against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    /// The body of a component.
    Body(usize),
    /// A point in a component's own frame (a pad).
    Point(usize, Point),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Relation {
    /// The gap between `part`'s body and the anchor is at most `max`.
    Near { part: usize, anchor: Anchor, max: f64 },
    /// `part` lies on `side` of `anchor`'s body (`Bottom`: below it), does
    /// not overlap it, overlaps its extent along the other axis, and the gap
    /// is at most `max_gap`.
    Beside {
        part: usize,
        anchor: usize,
        side: Edge,
        max_gap: f64,
    },
}

impl Relation {
    pub fn parts(&self) -> [Option<usize>; 2] {
        match self {
            Relation::Near { part, anchor, .. } => [
                Some(*part),
                Some(match anchor {
                    Anchor::Body(index) | Anchor::Point(index, _) => *index,
                }),
            ],
            Relation::Beside { part, anchor, .. } => [Some(*part), Some(*anchor)],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Constraints {
    /// The body lies within the distance of this board edge. On a
    /// constrained side the body may touch the edge itself (no edge margin:
    /// a courtyard may reach the board edge; copper clearance is the
    /// router's business).
    pub edges: Vec<(usize, Edge, f64)>,
    /// The body lies inside this box: [min x, min y, max x, max y].
    pub regions: Vec<(usize, [f64; 4])>,
    /// The part reaches beyond this board edge with the given box of its
    /// own frame ([min x, min y, max x, max y]; an antenna, a connector's
    /// mouth): that box lies outside the board, flush with the edge, and the
    /// rest of the body inside.
    pub overhangs: Vec<(usize, Edge, [f64; 4])>,
    pub relations: Vec<Relation>,
    /// Millimetres of wirelength one millimetre of relation violation costs.
    pub relation_weight: f64,
    /// Parts carried by others (a row): placed as part of their leader's
    /// body, their poses follow the leader's.
    pub followers: Vec<Follower>,
}

/// A part whose pose is its leader's pose composed with `offset` (in the
/// leader's own frame) and `angle` (added to the leader's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Follower {
    pub part: usize,
    pub leader: usize,
    pub offset: Point,
    pub angle: f64,
}

impl Constraints {
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
            && self.regions.is_empty()
            && self.overhangs.is_empty()
            && self.relations.is_empty()
    }

    pub fn overhang(&self, index: usize) -> Option<(Edge, [f64; 4])> {
        self.overhangs
            .iter()
            .find(|(part, _, _)| *part == index)
            .map(|(_, edge, local)| (*edge, *local))
    }
}

/// How far an overhanging box may stay short of the edge (mm).
const OVERHANG_SLACK: f64 = 0.5;

/// Edge margin per side (left, top, right, bottom) for a part: none on
/// sides an edge or overhang constraint puts it against.
pub fn side_margins(problem: &Problem, index: usize) -> [f64; 4] {
    let mut margins = [problem.components[index].edge_margin(problem.edge_margin); 4];
    let constraints = &problem.constraints;
    let sides = constraints
        .edges
        .iter()
        .filter(|(part, _, _)| *part == index)
        .map(|(_, edge, _)| *edge)
        .chain(constraints.overhang(index).map(|(edge, _)| edge));
    for edge in sides {
        margins[match edge {
            Edge::Left => 0,
            Edge::Top => 1,
            Edge::Right => 2,
            Edge::Bottom => 3,
        }] = 0.0;
    }
    margins
}

/// The overhang box in board coordinates: [min x, min y, max x, max y].
pub fn overhang_box(problem: &Problem, index: usize, pose: Pose, local: [f64; 4]) -> [f64; 4] {
    let component = &problem.components[index];
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for corner in [
        [local[0], local[1]],
        [local[2], local[1]],
        [local[2], local[3]],
        [local[0], local[3]],
    ] {
        let at = component.offset(corner, pose.angle);
        let at = [pose.position[0] + at[0], pose.position[1] + at[1]];
        bounds[0] = bounds[0].min(at[0]);
        bounds[1] = bounds[1].min(at[1]);
        bounds[2] = bounds[2].max(at[0]);
        bounds[3] = bounds[3].max(at[1]);
    }
    bounds
}

/// The part of an overhanging body that must lie on the board: its body box
/// cut at the edge line, as [min x, min y, max x, max y]. `None` for parts
/// without overhang.
pub fn inner_box(problem: &Problem, index: usize, pose: Pose) -> Option<[f64; 4]> {
    let (edge, _) = problem.constraints.overhang(index)?;
    let (center, half) = body(problem, index, pose);
    let mut inner = [
        center[0] - half[0],
        center[1] - half[1],
        center[0] + half[0],
        center[1] + half[1],
    ];
    let bounds = problem.bounds();
    match edge {
        Edge::Left => inner[0] = inner[0].max(bounds[0]),
        Edge::Top => inner[1] = inner[1].max(bounds[1]),
        Edge::Right => inner[2] = inner[2].min(bounds[2]),
        Edge::Bottom => inner[3] = inner[3].min(bounds[3]),
    }
    Some(inner)
}

/// How far the overhang box misses its place just beyond the edge.
fn overhang_violation(problem: &Problem, index: usize, pose: Pose) -> f64 {
    let Some((edge, local)) = problem.constraints.overhang(index) else {
        return 0.0;
    };
    let outside = overhang_box(problem, index, pose, local);
    let bounds = problem.bounds();
    // Distance of the box's inner side from the edge line: positive when
    // the box reaches onto the board.
    let reach = match edge {
        Edge::Left => outside[2] - bounds[0],
        Edge::Top => outside[3] - bounds[1],
        Edge::Right => bounds[2] - outside[0],
        Edge::Bottom => bounds[3] - outside[1],
    };
    reach.max(0.0) + (-reach - OVERHANG_SLACK).max(0.0)
}

/// Body box of a component at a pose: centre and half extents.
fn body(problem: &Problem, index: usize, pose: Pose) -> (Point, Point) {
    let component = &problem.components[index];
    (component.center(pose), component.half_extent(pose.angle))
}

/// Gap between two boxes (0 when they touch or overlap).
fn box_gap(a: (Point, Point), b: (Point, Point)) -> f64 {
    let dx = ((a.0[0] - b.0[0]).abs() - a.1[0] - b.1[0]).max(0.0);
    let dy = ((a.0[1] - b.0[1]).abs() - a.1[1] - b.1[1]).max(0.0);
    dx.hypot(dy)
}

fn anchor_point(problem: &Problem, poses: &[Pose], anchor: Anchor) -> (Point, Point) {
    match anchor {
        Anchor::Body(index) => body(problem, index, poses[index]),
        Anchor::Point(index, local) => {
            let component = &problem.components[index];
            let offset = component.offset(local, poses[index].angle);
            (
                [
                    poses[index].position[0] + offset[0],
                    poses[index].position[1] + offset[1],
                ],
                [0.0, 0.0],
            )
        }
    }
}

/// The band or box a body must stay in, from the edge and region
/// constraints on `index`, as limits for its body box: [min x, min y, max x,
/// max y] of the body's extent.
fn limits(problem: &Problem, index: usize) -> Option<[f64; 4]> {
    let constraints = &problem.constraints;
    let mut limits = [
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::INFINITY,
    ];
    let mut any = false;
    let bounds = problem.bounds();
    for (part, edge, _) in &constraints.edges {
        if *part != index {
            continue;
        }
        any = true;
        // The body stays on the board; how far inside is `clamp_center`'s
        // business.
        match edge {
            Edge::Left => limits[0] = limits[0].max(bounds[0]),
            Edge::Top => limits[1] = limits[1].max(bounds[1]),
            Edge::Right => limits[2] = limits[2].min(bounds[2]),
            Edge::Bottom => limits[3] = limits[3].min(bounds[3]),
        }
    }
    for (part, region) in &constraints.regions {
        if *part != index {
            continue;
        }
        any = true;
        for axis in 0..2 {
            limits[axis] = limits[axis].max(region[axis]);
            limits[axis + 2] = limits[axis + 2].min(region[axis + 2]);
        }
    }
    any.then_some(limits)
}

/// Whether the hard constraints on `index` hold at `pose`.
pub fn hard_ok(problem: &Problem, index: usize, pose: Pose) -> bool {
    hard_violation(problem, index, pose) <= 1.0e-6
}

/// How far (mm) the hard constraints on `index` are missed at `pose`.
pub fn hard_violation(problem: &Problem, index: usize, pose: Pose) -> f64 {
    let constraints = &problem.constraints;
    if constraints.edges.is_empty() && constraints.regions.is_empty() && constraints.overhangs.is_empty() {
        return 0.0;
    }
    let (center, half) = body(problem, index, pose);
    let low = [center[0] - half[0], center[1] - half[1]];
    let high = [center[0] + half[0], center[1] + half[1]];
    let bounds = problem.bounds();
    let mut violation: f64 = overhang_violation(problem, index, pose);
    for (part, edge, reach) in &constraints.edges {
        if *part != index {
            continue;
        }
        let distance = match edge {
            Edge::Left => low[0] - bounds[0],
            Edge::Top => low[1] - bounds[1],
            Edge::Right => bounds[2] - high[0],
            Edge::Bottom => bounds[3] - high[1],
        };
        // Beyond the edge is the outline check's business.
        violation = violation.max(distance - reach);
    }
    for (part, region) in &constraints.regions {
        if *part != index {
            continue;
        }
        for axis in 0..2 {
            violation = violation
                .max(region[axis] - low[axis])
                .max(high[axis] - region[axis + 2]);
        }
    }
    violation.max(0.0)
}

/// Clamps a body centre so that the body satisfies the hard constraints
/// (used by global placement). `half` is the body's half extent (without
/// halo), `angle` the part's orientation.
pub fn clamp_center(problem: &Problem, index: usize, center: &mut Point, half: Point, angle: f64) {
    if let Some((edge, local)) = problem.constraints.overhang(index) {
        // The overhang box sits just beyond the edge line.
        let component = &problem.components[index];
        let position = component.position_for_center(*center, angle);
        let outside = overhang_box(problem, index, Pose { position, angle }, local);
        let bounds = problem.bounds();
        match edge {
            Edge::Left => center[0] += bounds[0] - outside[2],
            Edge::Top => center[1] += bounds[1] - outside[3],
            Edge::Right => center[0] += bounds[2] - outside[0],
            Edge::Bottom => center[1] += bounds[3] - outside[1],
        }
    }
    let Some(limits) = limits(problem, index) else {
        return;
    };
    for axis in 0..2 {
        let low = limits[axis] + half[axis];
        let high = limits[axis + 2] - half[axis];
        if low <= high {
            center[axis] = center[axis].clamp(low, high);
        } else if low.is_finite() && high.is_finite() {
            center[axis] = (low + high) / 2.0;
        } else if low.is_finite() {
            center[axis] = low;
        } else if high.is_finite() {
            center[axis] = high;
        }
    }
    // An edge with a reach: keep the near side within reach of the edge.
    let bounds = problem.bounds();
    for (part, edge, reach) in &problem.constraints.edges {
        if *part != index {
            continue;
        }
        match edge {
            Edge::Left => center[0] = center[0].min(bounds[0] + reach + half[0]),
            Edge::Top => center[1] = center[1].min(bounds[1] + reach + half[1]),
            Edge::Right => center[0] = center[0].max(bounds[2] - reach - half[0]),
            Edge::Bottom => center[1] = center[1].max(bounds[3] - reach - half[1]),
        }
    }
}

/// Relations missed by less than this (grid snapping) count as kept.
pub const RELATION_TOLERANCE: f64 = 0.05;

/// How far (mm) a relation is missed.
pub fn relation_violation(problem: &Problem, poses: &[Pose], relation: &Relation) -> f64 {
    let violation = raw_relation_violation(problem, poses, relation);
    if violation <= RELATION_TOLERANCE { 0.0 } else { violation }
}

fn raw_relation_violation(problem: &Problem, poses: &[Pose], relation: &Relation) -> f64 {
    match relation {
        Relation::Near { part, anchor, max } => {
            let gap = box_gap(
                body(problem, *part, poses[*part]),
                anchor_point(problem, poses, *anchor),
            );
            (gap - max).max(0.0)
        }
        Relation::Beside {
            part,
            anchor,
            side,
            max_gap,
        } => {
            let (a, ah) = body(problem, *part, poses[*part]);
            let (b, bh) = body(problem, *anchor, poses[*anchor]);
            // `gap` along the relation's axis, positive when the part is on
            // the wanted side; `across`: how far the centre is outside the
            // anchor's extent along the other axis.
            let (gap, across) = match side {
                Edge::Bottom => ((a[1] - ah[1]) - (b[1] + bh[1]), (a[0] - b[0]).abs() - bh[0]),
                Edge::Top => ((b[1] - bh[1]) - (a[1] + ah[1]), (a[0] - b[0]).abs() - bh[0]),
                Edge::Right => ((a[0] - ah[0]) - (b[0] + bh[0]), (a[1] - b[1]).abs() - bh[1]),
                Edge::Left => ((b[0] - bh[0]) - (a[0] + ah[0]), (a[1] - b[1]).abs() - bh[1]),
            };
            (-gap).max(0.0) + (gap - max_gap).max(0.0) + across.max(0.0)
        }
    }
}

/// Weighted violation of the relations that involve `index` (all of them
/// for `None`).
pub fn relation_penalty(problem: &Problem, poses: &[Pose], index: Option<usize>) -> f64 {
    let constraints = &problem.constraints;
    if constraints.relations.is_empty() {
        return 0.0;
    }
    constraints
        .relations
        .iter()
        .filter(|relation| index.is_none_or(|index| relation.parts().contains(&Some(index))))
        .map(|relation| relation_violation(problem, poses, relation))
        .sum::<f64>()
        * constraints.relation_weight
}

/// Adds the relation forces (gradient of the penalty with respect to body
/// centres) for global placement. `body_of` maps component index to body
/// index (`usize::MAX` for fixed components).
pub fn add_relation_gradient(
    problem: &Problem,
    poses: &[Pose],
    body_of: &[usize],
    gradient: &mut [Point],
    weight: f64,
) {
    let mut push = |component: usize, force: Point| {
        let body = body_of[component];
        if body != usize::MAX {
            gradient[body][0] += weight * force[0];
            gradient[body][1] += weight * force[1];
        }
    };
    for relation in &problem.constraints.relations {
        if relation_violation(problem, poses, relation) <= 0.0 {
            continue;
        }
        match relation {
            Relation::Near { part, anchor, .. } => {
                let (a, _) = body(problem, *part, poses[*part]);
                let (b, _) = anchor_point(problem, poses, *anchor);
                let delta = [a[0] - b[0], a[1] - b[1]];
                let length = delta[0].hypot(delta[1]).max(1.0e-9);
                let unit = [delta[0] / length, delta[1] / length];
                push(*part, unit);
                let other = match anchor {
                    Anchor::Body(index) | Anchor::Point(index, _) => *index,
                };
                push(other, [-unit[0], -unit[1]]);
            }
            Relation::Beside {
                part,
                anchor,
                side,
                max_gap,
            } => {
                let (a, ah) = body(problem, *part, poses[*part]);
                let (b, bh) = body(problem, *anchor, poses[*anchor]);
                // Axis along the relation and the sign that moves the part
                // towards its side.
                let (axis, sign) = match side {
                    Edge::Bottom => (1, 1.0),
                    Edge::Top => (1, -1.0),
                    Edge::Right => (0, 1.0),
                    Edge::Left => (0, -1.0),
                };
                let gap = sign * (a[axis] - b[axis]) - ah[axis] - bh[axis];
                let mut force = [0.0; 2];
                if gap < 0.0 {
                    // Wrong side or overlapping: the part moves out.
                    force[axis] = -sign;
                } else if gap > *max_gap {
                    force[axis] = sign;
                }
                let other = 1 - axis;
                if (a[other] - b[other]).abs() > bh[other] {
                    force[other] = (a[other] - b[other]).signum();
                }
                push(*part, force);
                push(*anchor, [-force[0], -force[1]]);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Status {
    pub kind: &'static str,
    pub part: usize,
    pub other: Option<usize>,
    pub satisfied: bool,
    /// Millimetres by which the constraint is missed.
    pub violation: f64,
}

/// Every constraint with whether it holds at `poses`.
pub fn report(problem: &Problem, poses: &[Pose]) -> Vec<Status> {
    let constraints = &problem.constraints;
    let mut statuses = Vec::new();
    for (part, _, _) in &constraints.edges {
        let violation = hard_violation(problem, *part, poses[*part]);
        statuses.push(Status {
            kind: "edge",
            part: *part,
            other: None,
            satisfied: violation <= 1.0e-6,
            violation,
        });
    }
    for (part, _, _) in &constraints.overhangs {
        let violation = hard_violation(problem, *part, poses[*part]);
        statuses.push(Status {
            kind: "overhang",
            part: *part,
            other: None,
            satisfied: violation <= 1.0e-6,
            violation,
        });
    }
    for (part, _) in &constraints.regions {
        let violation = hard_violation(problem, *part, poses[*part]);
        statuses.push(Status {
            kind: "region",
            part: *part,
            other: None,
            satisfied: violation <= 1.0e-6,
            violation,
        });
    }
    for relation in &constraints.relations {
        let violation = relation_violation(problem, poses, relation);
        let [part, other] = relation.parts();
        statuses.push(Status {
            kind: match relation {
                Relation::Near { .. } => "near",
                Relation::Beside { .. } => "relative",
            },
            part: part.unwrap_or(0),
            other,
            satisfied: violation <= 1.0e-6,
            violation,
        });
    }
    statuses
}

/// Snaps a component origin to the placement grid. Parts against a
/// constrained edge get a grid phase that puts the constrained side exactly
/// on the edge (flush), and keep grid steps along it.
pub fn snap_position(problem: &Problem, index: usize, position: Point, angle: f64) -> Point {
    let grid = problem.grid;
    if grid <= 0.0 {
        return position;
    }
    let phase = grid_phase(problem, index, angle);
    let snap = |value: f64, phase: f64| ((value - phase) / grid).round() * grid + phase;
    [snap(position[0], phase[0]), snap(position[1], phase[1])]
}

fn grid_phase(problem: &Problem, index: usize, angle: f64) -> Point {
    let mut phase = [0.0, 0.0];
    let constraints = &problem.constraints;
    if constraints.edges.is_empty() && constraints.overhangs.is_empty() {
        return phase;
    }
    let grid = problem.grid;
    let component = &problem.components[index];
    let half = component.half_extent(angle);
    let bounds = problem.bounds();
    for (part, edge, _) in &constraints.edges {
        if *part != index {
            continue;
        }
        let (axis, coordinate) = match edge {
            Edge::Left => (0, bounds[0] + half[0]),
            Edge::Right => (0, bounds[2] - half[0]),
            Edge::Top => (1, bounds[1] + half[1]),
            Edge::Bottom => (1, bounds[3] - half[1]),
        };
        let mut center = [0.0, 0.0];
        center[axis] = coordinate;
        phase[axis] = component.position_for_center(center, angle)[axis].rem_euclid(grid);
    }
    if let Some((edge, local)) = constraints.overhang(index) {
        let outside = overhang_box(problem, index, Pose { position: [0.0, 0.0], angle }, local);
        let (axis, origin) = match edge {
            Edge::Left => (0, bounds[0] - outside[2]),
            Edge::Top => (1, bounds[1] - outside[3]),
            Edge::Right => (0, bounds[2] - outside[0]),
            Edge::Bottom => (1, bounds[3] - outside[1]),
        };
        phase[axis] = origin.rem_euclid(grid);
    }
    phase
}

/// Where a relation wants its part's body centre: the anchor point (near),
/// or just off the anchor's body on the relation's side (relative).
pub fn relation_target(problem: &Problem, poses: &[Pose], relation: &Relation) -> Point {
    match relation {
        Relation::Near { anchor, .. } => anchor_point(problem, poses, *anchor).0,
        Relation::Beside {
            part,
            anchor,
            side,
            max_gap,
        } => {
            let (_, ah) = body(problem, *part, poses[*part]);
            let (b, bh) = body(problem, *anchor, poses[*anchor]);
            let gap = max_gap / 2.0;
            match side {
                Edge::Bottom => [b[0], b[1] + bh[1] + gap + ah[1]],
                Edge::Top => [b[0], b[1] - bh[1] - gap - ah[1]],
                Edge::Right => [b[0] + bh[0] + gap + ah[0], b[1]],
                Edge::Left => [b[0] - bh[0] - gap - ah[0], b[1]],
            }
        }
    }
}

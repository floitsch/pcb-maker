// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Exact legality, legalization of a global placement, and
//! wirelength-driven detailed placement.

use crate::constraints;
use crate::problem::{Point, Pose, Problem, point_in_polygon};

#[derive(Clone, Copy, Debug)]
struct Rect {
    center: Point,
    /// Half extent of the axis-aligned box around the body.
    half: Point,
    round: bool,
    /// A body turned by other than a quarter turn: its direction (cosine
    /// and sine of the board-space angle of its own x axis) and its own half
    /// sizes. Overlap and outline tests use the turned rectangle, not the
    /// box around it (ErgoSNM's thumb keys at -30 degrees, whose boxes are
    /// half as large again and leave the slanted edge).
    turn: Option<(Point, Point)>,
}

impl Rect {
    /// The rectangle grown by `margin` on every side.
    fn grown(self, margin: f64) -> Rect {
        Rect {
            half: [self.half[0] + margin, self.half[1] + margin],
            turn: self
                .turn
                .map(|(direction, own)| (direction, [own[0] + margin, own[1] + margin])),
            ..self
        }
    }

    /// The corners, in order around the rectangle.
    fn corners(&self) -> [Point; 4] {
        match self.turn {
            None => [
                [self.center[0] - self.half[0], self.center[1] - self.half[1]],
                [self.center[0] + self.half[0], self.center[1] - self.half[1]],
                [self.center[0] + self.half[0], self.center[1] + self.half[1]],
                [self.center[0] - self.half[0], self.center[1] + self.half[1]],
            ],
            Some(([cos, sin], own)) => {
                [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].map(|[sx, sy]| {
                    let (x, y) = (sx * own[0], sy * own[1]);
                    [
                        self.center[0] + cos * x - sin * y,
                        self.center[1] + sin * x + cos * y,
                    ]
                })
            }
        }
    }

    /// Own axes and half sizes (the board's for an unturned rectangle).
    fn frame(&self) -> ([Point; 2], Point) {
        match self.turn {
            None => ([[1.0, 0.0], [0.0, 1.0]], self.half),
            Some(([cos, sin], own)) => ([[cos, sin], [-sin, cos]], own),
        }
    }
}

/// The direction of a body at `angle` when it is not a quarter turn.
fn turn_of(angle: f64, half: Point) -> Option<(Point, Point)> {
    direction_of(angle).map(|direction| (direction, half))
}

/// The direction `turn_of` gives a body at `angle`: None at a quarter turn.
fn direction_of(angle: f64) -> Option<Point> {
    // The quarter turns parts sit at, without `rem_euclid` (a call into
    // libm's fmod): the same answer as below.
    if angle == 0.0
        || angle == 90.0
        || angle == 180.0
        || angle == 270.0
        || angle == -90.0
        || angle == -180.0
        || angle == -270.0
    {
        return None;
    }
    let quarter = angle.rem_euclid(90.0);
    if quarter < 1.0e-6 || 90.0 - quarter < 1.0e-6 {
        return None;
    }
    // KiCad turns a local point by -angle (y down).
    let (sin, cos) = crate::problem::sin_cos_degrees(-angle);
    Some([cos, sin])
}

/// Boxes of a part's own frame ([min x, min y, max x, max y]) on the
/// board at `pose`, grown by `margin`: turned with the part when it is not
/// at a quarter turn (a 45 degree 0402's pads are not the square around
/// them).
fn turned_rects(boxes: &[[f64; 4]], pose: Pose, margin: f64) -> Vec<Rect> {
    turned(boxes, pose, margin).collect()
}

/// `turned_rects` one by one, without the list.
fn turned(boxes: &[[f64; 4]], pose: Pose, margin: f64) -> impl Iterator<Item = Rect> + Clone + '_ {
    let (sin, cos) = crate::problem::sin_cos_degrees(-pose.angle);
    // The pose's turns once for all its boxes (`offset`, `turn_of`).
    let turn = crate::problem::rotation(-pose.angle);
    let direction = direction_of(pose.angle);
    boxes.iter().map(move |local| {
        let center = crate::problem::rotate_by(
            [(local[0] + local[2]) / 2.0, (local[1] + local[3]) / 2.0],
            turn,
        );
        let own = [
            (local[2] - local[0]) / 2.0 + margin,
            (local[3] - local[1]) / 2.0 + margin,
        ];
        Rect {
            center: [pose.position[0] + center[0], pose.position[1] + center[1]],
            half: [
                cos.abs() * own[0] + sin.abs() * own[1],
                sin.abs() * own[0] + cos.abs() * own[1],
            ],
            round: false,
            turn: direction.map(|direction| (direction, own)),
        }
    })
}

thread_local! {
    /// Lists of rectangles the legality tests hold while they compare
    /// every box of one part with every box of another (`with_rects`).
    static RECT_LISTS: std::cell::RefCell<Vec<Vec<Rect>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Calls `test` with the rectangles of `rects` in a list kept for the
/// thread (no allocation a pair of parts, as `turned_rects` made one).
fn with_rects<T>(rects: impl Iterator<Item = Rect>, test: impl FnOnce(&[Rect]) -> T) -> T {
    let mut list = RECT_LISTS
        .with(|lists| lists.borrow_mut().pop())
        .unwrap_or_default();
    list.clear();
    list.extend(rects);
    let answer = test(&list);
    RECT_LISTS.with(|lists| lists.borrow_mut().push(list));
    answer
}

fn rect(problem: &Problem, index: usize, pose: Pose) -> Rect {
    let component = &problem.components[index];
    let half = component.half_extent(pose.angle);
    let own = [
        component.body_size[0] / 2.0 + component.halo,
        component.body_size[1] / 2.0 + component.halo,
    ];
    Rect {
        center: component.center(pose),
        half: [half[0] + component.halo, half[1] + component.halo],
        round: component.round,
        turn: if component.round {
            None
        } else {
            turn_of(pose.angle, own)
        },
    }
}

/// Whether two rectangles, at least one of them turned, come closer than
/// `spacing` (separating axes; the spacing grows both, which is a little
/// conservative at the corners).
fn turned_overlaps(a: Rect, b: Rect, spacing: f64) -> bool {
    let delta = [b.center[0] - a.center[0], b.center[1] - a.center[1]];
    let (axes_a, half_a) = a.frame();
    let (axes_b, half_b) = b.frame();
    let dot = |u: Point, v: Point| u[0] * v[0] + u[1] * v[1];
    for axis in axes_a.iter().chain(axes_b.iter()) {
        let reach_a =
            half_a[0] * dot(axes_a[0], *axis).abs() + half_a[1] * dot(axes_a[1], *axis).abs();
        let reach_b =
            half_b[0] * dot(axes_b[0], *axis).abs() + half_b[1] * dot(axes_b[1], *axis).abs();
        if dot(delta, *axis).abs() >= reach_a + reach_b + spacing - 1.0e-9 {
            return false;
        }
    }
    true
}

fn overlaps(a: Rect, b: Rect, spacing: f64) -> bool {
    if a.turn.is_some() || b.turn.is_some() {
        match (a.round, b.round) {
            (false, false) => return turned_overlaps(a, b, spacing),
            (true, false) | (false, true) => {
                // The disc's centre in the turned rectangle's frame.
                let (disc, body) = if a.round { (a, b) } else { (b, a) };
                let (axes, own) = body.frame();
                let delta = [
                    disc.center[0] - body.center[0],
                    disc.center[1] - body.center[1],
                ];
                let local = [
                    (delta[0] * axes[0][0] + delta[1] * axes[0][1]).abs(),
                    (delta[0] * axes[1][0] + delta[1] * axes[1][1]).abs(),
                ];
                let outside = [(local[0] - own[0]).max(0.0), (local[1] - own[1]).max(0.0)];
                return outside[0].hypot(outside[1]) < disc.half[0] + spacing - 1.0e-9;
            }
            (true, true) => {}
        }
    }
    let delta = [
        (a.center[0] - b.center[0]).abs(),
        (a.center[1] - b.center[1]).abs(),
    ];
    match (a.round, b.round) {
        (false, false) => {
            delta[0] < a.half[0] + b.half[0] + spacing - 1.0e-9
                && delta[1] < a.half[1] + b.half[1] + spacing - 1.0e-9
        }
        (true, true) => delta[0].hypot(delta[1]) < a.half[0] + b.half[0] + spacing - 1.0e-9,
        _ => {
            let (disc, body) = if a.round { (a, b) } else { (b, a) };
            let outside = [
                (delta[0] - body.half[0]).max(0.0),
                (delta[1] - body.half[1]).max(0.0),
            ];
            outside[0].hypot(outside[1]) < disc.half[0] + spacing - 1.0e-9
        }
    }
}

/// The gap two bodies on one side keep: the spacing, or at the
/// courtyard-spacing level what their copper needs.
fn body_spacing(problem: &Problem, a: usize, b: usize) -> f64 {
    if problem.constraints.pad_clearance {
        // Courtyards may touch; the pads keep their clearance
        // (`pads_too_close`).
        0.0
    } else if problem.constraints.courtyard_spacing {
        let components = &problem.components;
        components[a].copper_share(problem.min_spacing)
            + components[b].copper_share(problem.min_spacing)
    } else {
        problem.spacing
    }
}

/// Twice the signed area of the triangle `p`, `q`, `r` (positive where `r`
/// lies left of the line from `p` to `q`).
fn orient(p: Point, q: Point, r: Point) -> f64 {
    (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let (d1, d2, d3, d4) = (
        orient(c, d, a),
        orient(c, d, b),
        orient(a, b, c),
        orient(a, b, d),
    );
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

/// Whether `body` lies inside the board's outline: its corners inside
/// (from the outline's edges by bands, `board`, where there is one: the
/// edges a ray from a corner crosses lie in its band), no outline edge
/// crossing a side. An edge with all four corners on one side of its line
/// crosses no side (`segments_cross`'s second condition, the same
/// orientations): its sides' orientations are not taken (the fit test of
/// every legal spot walked the outline sixteen orientations an edge: a
/// fifth of the 4in1's legalisation, 2026-10-28). The same answers.
fn inside_outline(problem: &Problem, body: Rect, board: Option<&Outline>) -> bool {
    let corners = body.corners();
    let outline = &problem.outline;
    let inside = |corner: &Point| match board {
        Some(board) => {
            let inside = board.inside(*corner, outline);
            if board.check {
                assert_eq!(
                    inside,
                    point_in_polygon(*corner, outline),
                    "{corner:?} inside the outline by bands against the pass over every edge"
                );
            }
            inside
        }
        None => point_in_polygon(*corner, outline),
    };
    if !corners.iter().all(inside) {
        return false;
    }
    for index in 0..outline.len() {
        let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
        let left = corners.map(|corner| orient(a, b, corner) > 0.0);
        if left == [left[0]; 4] {
            continue;
        }
        for side in 0..4 {
            if segments_cross(a, b, corners[side], corners[(side + 1) % 4]) {
                return false;
            }
        }
    }
    true
}

/// The body alone, grown by `margin`, for tests against the board edge.
fn bare(problem: &Problem, index: usize, pose: Pose, margin: f64) -> Rect {
    let component = &problem.components[index];
    let half = component.half_extent(pose.angle);
    let own = [
        component.body_size[0] / 2.0 + margin,
        component.body_size[1] / 2.0 + margin,
    ];
    Rect {
        center: component.center(pose),
        half: [half[0] + margin, half[1] + margin],
        round: component.round,
        turn: if component.round {
            None
        } else {
            turn_of(pose.angle, own)
        },
    }
}

/// Whether the part keeps the edge margin inside the outline; an
/// overhanging part only with the share of its body that belongs on the
/// board.
fn on_board(problem: &Problem, index: usize, pose: Pose, board: Option<&Outline>) -> bool {
    let component = &problem.components[index];
    if problem.constraints.edge_copper && !component.pads.is_empty() {
        let mut touching = [false; 4];
        for (part, edge, _) in &problem.constraints.edges {
            if *part == index {
                touching[match edge {
                    constraints::Edge::Left => 0,
                    constraints::Edge::Top => 1,
                    constraints::Edge::Right => 2,
                    constraints::Edge::Bottom => 3,
                }] = true;
            }
        }
        if touching.iter().any(|touch| *touch) {
            // Copper on the constrained side may come as close to the edge as
            // the project's copper-to-edge clearance; elsewhere it keeps the
            // copper clearance too (the edge margin is room for routing,
            // which a tab does not need), and never less than that
            // copper-to-edge clearance: every side is a board edge, the side
            // an overhang reaches past too (the fence's ESP32 module, its top
            // pads 0.2 mm from the edge against the project's 0.3 mm; the
            // router cannot move a pad).
            let margins = touching.map(|touch| {
                // At KiCad's legality every side keeps the copper-to-edge
                // clearance, as KiCad checks it.
                if touch || problem.constraints.pad_clearance {
                    problem.constraints.copper_edge - 1.0e-6
                } else {
                    problem
                        .min_spacing
                        .min(problem.edge_margin)
                        .max(problem.constraints.copper_edge - 1.0e-6)
                }
            });
            return component.pad_boxes(pose).iter().all(|(center, half)| {
                let grown = [
                    center[0] - half[0] - margins[0],
                    center[1] - half[1] - margins[1],
                    center[0] + half[0] + margins[2],
                    center[1] + half[1] + margins[3],
                ];
                inside_outline(
                    problem,
                    Rect {
                        center: [(grown[0] + grown[2]) / 2.0, (grown[1] + grown[3]) / 2.0],
                        half: [(grown[2] - grown[0]) / 2.0, (grown[3] - grown[1]) / 2.0],
                        round: false,
                        turn: None,
                    },
                    board,
                )
            });
        }
    }
    let held = problem
        .constraints
        .edges
        .iter()
        .any(|(part, _, _)| *part == index)
        || problem.constraints.overhang(index).is_some();
    // At KiCad's legality an unheld part's pads keep the copper-to-edge
    // clearance, its body only the outline.
    if problem.constraints.pad_clearance && !held && !component.pads.is_empty() {
        return pads_keep_edge(problem, index, pose, board);
    }
    if problem.constraints.is_empty() || (!held && turn_of(pose.angle, [0.0, 0.0]).is_some()) {
        let margin = problem.components[index].edge_margin(problem.edge_margin);
        return inside_outline(problem, bare(problem, index, pose, margin), board);
    }
    let inner = constraints::inner_box(problem, index, pose).unwrap_or_else(|| {
        let (center, half) = (
            problem.components[index].center(pose),
            problem.components[index].half_extent(pose.angle),
        );
        [
            center[0] - half[0],
            center[1] - half[1],
            center[0] + half[0],
            center[1] + half[1],
        ]
    });
    // A side against a constrained edge may touch it: shrink by a hair so
    // the outline test does not sit on the boundary.
    let margins = constraints::side_margins(problem, index, pose.angle)
        .map(|margin| if margin > 0.0 { margin } else { -1.0e-6 });
    let grown = [
        inner[0] - margins[0],
        inner[1] - margins[1],
        inner[2] + margins[2],
        inner[3] + margins[3],
    ];
    if grown[0] > grown[2] || grown[1] > grown[3] {
        return false;
    }
    inside_outline(
        problem,
        Rect {
            center: [(grown[0] + grown[2]) / 2.0, (grown[1] + grown[3]) / 2.0],
            half: [(grown[2] - grown[0]) / 2.0, (grown[3] - grown[1]) / 2.0],
            round: false,
            turn: None,
        },
        board,
    )
}

/// At KiCad's legality, for a part whose copper lies inside its body at
/// the levels that let bodies near the edge (`edge_inset`): the body inside
/// the outline, each pad the project's copper-to-edge clearance from it,
/// as KiCad checks copper against the edge (not the body by its copper's
/// least inset, nor the level's margin for routing).
fn pads_keep_edge(problem: &Problem, index: usize, pose: Pose, board: Option<&Outline>) -> bool {
    let component = &problem.components[index];
    if !inside_outline(problem, bare(problem, index, pose, 0.0), board) {
        return false;
    }
    // Pads near no side of the body cannot be nearer the edge than the
    // body's own margin allows. KiCad checks the copper against the
    // project's copper-to-edge clearance (the level's edge margin where
    // that is less).
    let body = bare(problem, index, pose, 0.0);
    let reach = problem.edge_margin.min(problem.constraints.copper_edge);
    if inside_outline(problem, body.grown(reach), board) {
        return true;
    }
    turned_rects(&component.pads, pose, reach)
        .into_iter()
        .chain(turned_rects(&component.copper_graphics, pose, reach))
        .all(|pad| inside_outline(problem, pad, board))
}

/// Whether the body itself (without any margin) lies inside the outline.
pub fn body_inside_outline(problem: &Problem, index: usize, pose: Pose) -> bool {
    inside_outline(problem, bare(problem, index, pose, 0.0), None)
}

/// Whether the body grown by `reach` on every side touches the outline.
pub fn body_near_outline(problem: &Problem, index: usize, pose: Pose, reach: f64) -> bool {
    !inside_outline(problem, bare(problem, index, pose, reach), None)
}

/// Whether component `index` at `pose` is inside the board and clear of all
/// components in `others` (indices with their poses taken from `poses`).
pub fn is_legal(
    problem: &Problem,
    poses: &[Pose],
    index: usize,
    pose: Pose,
    others: impl Iterator<Item = usize>,
) -> bool {
    // Work: the outline test walks the outline, then each part is a box test.
    let mut work = 1;
    let legal = is_legal_counting(problem, poses, index, pose, others, &mut work);
    crate::add_work(work);
    legal
}

fn is_legal_counting(
    problem: &Problem,
    poses: &[Pose],
    index: usize,
    pose: Pose,
    others: impl Iterator<Item = usize>,
    work: &mut u64,
) -> bool {
    clear_of(problem, poses, index, pose, others, work)
        && fits_board(problem, index, pose, work, None)
}

/// Whether a movable part at `pose` is on the board and keeps its hard
/// constraints (a fixed one always is).
fn fits_board(
    problem: &Problem,
    index: usize,
    pose: Pose,
    work: &mut u64,
    board: Option<&Outline>,
) -> bool {
    if !problem.components[index].fixed {
        // The outline walk; with constraints, the edge and region lists
        // and the part's pads (its copper's offsets from the edges).
        *work += problem.outline.len() as u64;
        if !problem.constraints.is_empty() {
            let constraints = &problem.constraints;
            // The edge and region lists are scanned for the part's own
            // entries, a few comparisons each (a panel's copies put a
            // region on each of its 344 movable parts).
            *work += ((constraints.edges.len() + constraints.regions.len()).div_ceil(2)
                + 2 * problem.components[index].pads.len()) as u64;
            *work += 2 * held_work(problem, index);
        }
        if !on_board(problem, index, pose, board) || !constraints::hard_ok(problem, index, pose) {
            return false;
        }
    }
    true
}

/// The extra work of a part an edge or an overhang holds: its pads are
/// turned for their offsets from the edges, and the board's box is taken
/// from the outline (glasgow's eviction of a held connector did five
/// times the work it counted).
fn held_work(problem: &Problem, index: usize) -> u64 {
    let constraints = &problem.constraints;
    if constraints.edges.iter().any(|(part, _, _)| *part == index)
        || constraints.overhang(index).is_some()
    {
        (4 * problem.components[index].pads.len() + problem.outline.len() / 4) as u64
    } else {
        0
    }
}

/// Whether part `index` at `pose` keeps clear of the parts in `others`.
fn clear_of(
    problem: &Problem,
    poses: &[Pose],
    index: usize,
    pose: Pose,
    others: impl Iterator<Item = usize>,
    work: &mut u64,
) -> bool {
    let body = rect(problem, index, pose);
    let linked_work = linked_work(problem);
    // The parts first: on a crowded board they turn down most spots, and
    // the outline test walks every outline edge.
    for other in others {
        if !clear_of_one(
            problem,
            poses,
            index,
            pose,
            &body,
            linked_work,
            other,
            work,
            None,
        ) {
            return false;
        }
    }
    true
}

/// Whether part `index` at `pose` (its box `body`) keeps clear of part
/// `other`, counting the work of the test: one step of `clear_of`.
#[allow(clippy::too_many_arguments)]
fn clear_of_one(
    problem: &Problem,
    poses: &[Pose],
    index: usize,
    pose: Pose,
    body: &Rect,
    linked_work: u64,
    other: usize,
    work: &mut u64,
    facts: Option<&Facts>,
) -> bool {
    let body = *body;
    let side = problem.components[index].side;
    if other == index {
        return true;
    }
    *work += 1;
    // Two fixed parts are the designer's responsibility.
    if problem.components[index].fixed && problem.components[other].fixed {
        return true;
    }
    // A part without room elsewhere may lie on the board's marks.
    if match facts {
        Some(facts) => facts.lies_on(problem, index, other),
        None => problem.constraints.lies_on(index, other),
    } {
        return true;
    }
    let other_side = problem.components[other].side;
    if side.collides(other_side) {
        let (mine, theirs) = (&problem.components[index], &problem.components[other]);
        if mine.copper_only || theirs.copper_only {
            if copper_meets_cutout(problem, index, pose, other, poses[other], work, facts)
                || body_meets_solid_cutout(problem, index, pose, other, poses[other], work, facts)
            {
                return false;
            }
            return true;
        }
        let hollow = mine.hollow_for(theirs) || theirs.hollow_for(mine);
        if !hollow {
            // The other's box (two turns) and the linked-pair lookup.
            *work += RECT_WORK + linked_work;
        }
        if if hollow {
            hollow_overlap(problem, index, pose, other, poses[other], work)
        } else {
            if match facts {
                Some(facts) => facts.bare(problem, index, other),
                None => problem.constraints.bare(index, other),
            } {
                // Parts tied together meet without their halos.
                let bare = |part: usize, pose: Pose| {
                    let rect = rect(problem, part, pose);
                    let halo = problem.components[part].halo;
                    rect.grown(-halo)
                };
                overlaps(
                    bare(index, pose),
                    bare(other, poses[other]),
                    body_spacing(problem, index, other),
                )
            } else if problem.shared_halos {
                // The larger halo between them, as spacing between the
                // bodies (growing one body instead is not symmetric where
                // a disc meets a rectangle's corner).
                let (mine, theirs) = (
                    problem.components[index].halo,
                    problem.components[other].halo,
                );
                let gap = body_spacing(problem, index, other) + mine.max(theirs);
                overlaps(
                    body.grown(-mine),
                    rect(problem, other, poses[other]).grown(-theirs),
                    gap,
                )
            } else {
                overlaps(
                    body,
                    rect(problem, other, poses[other]),
                    body_spacing(problem, index, other),
                )
            }
        } {
            return false;
        }
        if !hollow && far_courtyard_overlap(problem, index, pose, other, poses[other], work) {
            return false;
        }
        if problem.constraints.pad_clearance
            && pads_too_close(problem, index, pose, other, poses[other], work)
        {
            return false;
        }
        if pads_in_hole_rings(problem, index, pose, other, poses[other], work) {
            return false;
        }
    } else if side.opposite(other_side)
        && far_side_overlap(problem, index, pose, other, poses[other], work)
    {
        return false;
    }
    true
}

/// The legality tests' prefilter by boxes (the legaliser's spot search,
/// the refinement's candidates, the eviction): per part the box on the
/// board around everything `clear_of_one` compares of it (its body with
/// its halo, pads, holes and their rings, courtyards, hollow boxes, the
/// far side's), and the widest gap a test asks for. Two parts at quarter
/// turns whose boxes lie farther apart than that on one axis are apart on
/// that axis in every comparison of the test, which takes its far path:
/// clear, and the work of the two parts alone, not of their poses (as
/// `Sweep`'s far parts). That work is measured once per pair with the
/// part far off (`far_work`) and booked without the test. A part turned
/// off a quarter turn (separating axes on its own axes) and a cutout are
/// always tested. The same answers and the same work as `clear_of`
/// (protoesp's legaliser tested 4.8 parts a spot, 86 % of them apart by
/// their boxes, 2026-10-28); checked against the direct test in debug
/// builds and with `PCB_PLACER_CHECK_OVERLAP`.
pub(crate) struct Apart {
    /// Per part: the box (own frame, [min x, min y, max x, max y]) around
    /// its pads, holes, rings, courtyards and hollow boxes; None where it
    /// has none.
    local: Vec<Option<[f64; 4]>>,
    /// Every gap a pair test asks for (the spacing, the copper clearance,
    /// the mask gap, two copper shares), and a margin for the rounding.
    gap: f64,
    /// Per part: the pose (bits) its board box was taken at, and the box
    /// (None: turned off a quarter turn, or a cutout).
    boxes: Vec<Option<KeptBox>>,
    /// The part whose far work `far` holds, and per other part its far
    /// work (`u64::MAX`: not measured yet).
    far_part: usize,
    far: Vec<u64>,
    /// The cutouts' outlines for the polygon tests, the parts' flags.
    facts: Facts,
    /// `linked_work(problem)` (a logarithm and a rounding, libm's calls,
    /// for every spot: 4 % of protoesp's legalisation, 2026-10-28).
    linked_work: u64,
    /// The board's outline by bands for the fit tests (an outline of
    /// more than a few edges).
    board: Option<Outline>,
    /// Every skipped test checked against the direct one.
    check: bool,
}

/// A part's pose (its bits) and its board box there (`Apart`).
type KeptBox = ([u64; 3], Option<[f64; 4]>);

impl Apart {
    pub(crate) fn new(problem: &Problem) -> Self {
        let count = problem.components.len();
        let local = problem
            .components
            .iter()
            .map(|component| {
                component
                    .pads
                    .iter()
                    .chain(&component.far_side)
                    .chain(&component.far_copper)
                    .chain(&component.far_courtyard)
                    .chain(&component.hollow)
                    .chain(&component.courtyards)
                    .chain(&component.hole_rings)
                    .fold(None, |union: Option<[f64; 4]>, b| {
                        Some(match union {
                            None => *b,
                            Some(u) => [
                                u[0].min(b[0]),
                                u[1].min(b[1]),
                                u[2].max(b[2]),
                                u[3].max(b[3]),
                            ],
                        })
                    })
            })
            .collect();
        let share = problem
            .components
            .iter()
            .map(|component| component.copper_share(problem.min_spacing))
            .fold(0.0, f64::max);
        let gap = problem
            .spacing
            .max(problem.min_spacing)
            .max(problem.constraints.pad_mask_gap)
            .max(2.0 * share)
            + 1.0e-6;
        Apart {
            local,
            gap,
            boxes: vec![None; count],
            far_part: usize::MAX,
            far: vec![u64::MAX; count],
            facts: Facts::new(problem),
            linked_work: linked_work(problem),
            board: (problem.outline.len() >= BOARD_INDEX_EDGES)
                .then(|| Outline::new(&problem.outline)),
            check: cfg!(debug_assertions) || std::env::var_os("PCB_PLACER_CHECK_OVERLAP").is_some(),
        }
    }

    /// The board box of part `part` at `pose`, its body's box `body`
    /// (`rect`); None where the part is turned off a quarter turn or is a
    /// cutout.
    fn board_box(
        &self,
        problem: &Problem,
        part: usize,
        pose: Pose,
        body: &Rect,
    ) -> Option<[f64; 4]> {
        if problem.components[part].copper_only || direction_of(pose.angle).is_some() {
            return None;
        }
        // A disc's radius is its first half extent (`overlaps`).
        let half = if body.round {
            let radius = body.half[0].max(body.half[1]);
            [radius, radius]
        } else {
            body.half
        };
        let mut extent = [
            body.center[0] - half[0],
            body.center[1] - half[1],
            body.center[0] + half[0],
            body.center[1] + half[1],
        ];
        if let Some(local) = &self.local[part]
            && let Some(rect) = turned(std::slice::from_ref(local), pose, 0.0).next()
        {
            extent = [
                extent[0].min(rect.center[0] - rect.half[0]),
                extent[1].min(rect.center[1] - rect.half[1]),
                extent[2].max(rect.center[0] + rect.half[0]),
                extent[3].max(rect.center[1] + rect.half[1]),
            ];
        }
        Some(extent)
    }

    /// `board_box` of part `other` at its pose in `poses`, kept per pose.
    fn other_box(&mut self, problem: &Problem, poses: &[Pose], other: usize) -> Option<[f64; 4]> {
        let pose = poses[other];
        let key = [
            pose.position[0].to_bits(),
            pose.position[1].to_bits(),
            pose.angle.to_bits(),
        ];
        if let Some((kept, extent)) = self.boxes[other]
            && kept == key
        {
            return extent;
        }
        let body = rect(problem, other, pose);
        let extent = self.board_box(problem, other, pose, &body);
        self.boxes[other] = Some((key, extent));
        extent
    }

    /// The work `clear_of_one` books for `index` against `other` on its
    /// far path: measured with `index` far off, kept per pair.
    fn far_work(&mut self, problem: &Problem, poses: &[Pose], index: usize, other: usize) -> u64 {
        if self.far_part != index {
            self.far_part = index;
            self.far.iter_mut().for_each(|work| *work = u64::MAX);
        }
        if self.far[other] == u64::MAX {
            let away = Pose {
                position: [
                    poses[index].position[0] + 1.0e7,
                    poses[index].position[1] + 1.0e7,
                ],
                angle: poses[index].angle,
            };
            let body = rect(problem, index, away);
            let mut work = 0;
            let clear = clear_of_one(
                problem,
                poses,
                index,
                away,
                &body,
                self.linked_work,
                other,
                &mut work,
                None,
            );
            debug_assert!(clear, "part {index} far off meets part {other}");
            self.far[other] = work;
        }
        self.far[other]
    }

    /// `clear_of(problem, poses, index, pose, others, work)`: the same
    /// answer and the same work, the parts apart by their boxes not tested.
    pub(crate) fn clear_of(
        &mut self,
        problem: &Problem,
        poses: &[Pose],
        index: usize,
        pose: Pose,
        others: impl Iterator<Item = usize>,
        work: &mut u64,
    ) -> bool {
        let body = rect(problem, index, pose);
        let linked_work = self.linked_work;
        let mine = self.board_box(problem, index, pose, &body);
        for other in others {
            if let Some(mine) = mine
                && other != index
                && let Some(theirs) = self.other_box(problem, poses, other)
                && boxes_apart(mine, theirs, self.gap)
            {
                let far = self.far_work(problem, poses, index, other);
                if self.check {
                    let mut direct = 0;
                    let clear = clear_of_one(
                        problem,
                        poses,
                        index,
                        pose,
                        &body,
                        linked_work,
                        other,
                        &mut direct,
                        Some(&self.facts),
                    );
                    assert_eq!(
                        (clear, direct),
                        (true, far),
                        "part {index} at {pose:?} against part {other}: (clear, work) apart by boxes against the direct test"
                    );
                }
                *work += far;
                continue;
            }
            if !clear_of_one(
                problem,
                poses,
                index,
                pose,
                &body,
                linked_work,
                other,
                work,
                Some(&self.facts),
            ) {
                return false;
            }
        }
        true
    }

    /// `is_legal(problem, poses, index, pose, others)`, booking its work.
    pub(crate) fn is_legal(
        &mut self,
        problem: &Problem,
        poses: &[Pose],
        index: usize,
        pose: Pose,
        others: impl Iterator<Item = usize>,
    ) -> bool {
        let mut work = 1;
        let legal = self.clear_of(problem, poses, index, pose, others, &mut work)
            && fits_board(problem, index, pose, &mut work, self.board.as_ref());
        crate::add_work(work);
        legal
    }
}

/// Whether two boxes ([min x, min y, max x, max y]) lie more than `gap`
/// apart on one axis.
fn boxes_apart(a: [f64; 4], b: [f64; 4], gap: f64) -> bool {
    b[0] - a[2] > gap || a[0] - b[2] > gap || b[1] - a[3] > gap || a[1] - b[3] > gap
}

/// How far the parts reach, for `Sweep`: per part the distance from its
/// body's centre to the farthest corner of anything it tests legality
/// with (its body and halo, pads, holes, courtyards, rings, copper),
/// whatever its pose (a turn keeps distances), and the widest gap a test
/// asks for.
pub(crate) struct Reach {
    radius: Vec<f64>,
    gap: f64,
}

impl Reach {
    pub(crate) fn new(problem: &Problem) -> Self {
        let radius = problem
            .components
            .iter()
            .map(|component| {
                let center = component.body_center;
                let corner = |box_: &[f64; 4]| {
                    [
                        [box_[0], box_[1]],
                        [box_[2], box_[1]],
                        [box_[2], box_[3]],
                        [box_[0], box_[3]],
                    ]
                    .iter()
                    .map(|point| (point[0] - center[0]).hypot(point[1] - center[1]))
                    .fold(0.0, f64::max)
                };
                let body = (component.body_size[0] / 2.0 + component.halo)
                    .hypot(component.body_size[1] / 2.0 + component.halo);
                component
                    .pads
                    .iter()
                    .chain(&component.far_side)
                    .chain(&component.far_copper)
                    .chain(&component.far_courtyard)
                    .chain(&component.hollow)
                    .chain(&component.courtyards)
                    .chain(&component.tight_hollow)
                    .chain(&component.hole_rings)
                    .chain(&component.copper_graphics)
                    .chain(component.tight.as_ref())
                    .map(corner)
                    .fold(body, f64::max)
                    * (1.0 + 1.0e-9)
                    + 1.0e-6
            })
            .collect();
        let gap = problem.spacing
            + problem.min_spacing
            + problem.constraints.pad_mask_gap
            + problem.edge_margin
            + 1.0;
        Reach { radius, gap }
    }
}

/// `is_legal(problem, poses, index, pose, 0..count)` for many spots of one
/// part within an area (the relation repair's ring search), the same
/// answer and the same work booked, without testing every part of the
/// board at every spot (the repair's legality tests were 10 to 36 % of a
/// placement's CPU on MegaDrive and SmartSpin2k's panel, 2026-10-28).
/// Only the parts that may come near are tested (`near`, in index order);
/// a part farther away takes the test's far path, which keeps it clear
/// and books work that depends on the two parts alone, not on their poses
/// (`far`: what `clear_of_one` books for it with the part moved far off),
/// so its work is added without the test. Cutouts (`copper_only`) are
/// always tested.
pub(crate) struct Sweep {
    index: usize,
    /// The parts that may come near a spot in the area, ascending.
    near: Vec<usize>,
    /// `prefix[k]`: the far work of the parts below `k` (0 for `index`).
    prefix: Vec<u64>,
    /// The direct test instead (the part is a cutout).
    direct: bool,
    /// Every answer and its work checked against the direct test (debug
    /// builds and `PCB_PLACER_CHECK_OVERLAP`).
    check: bool,
}

/// How much farther than the parts' reach and the gap a part must lie to
/// count as far: a test on turned boxes separates them along their own
/// axes, which may be 45 degrees off the axis they lie apart on (a factor
/// of the square root of two).
const SWEEP_FACTOR: f64 = 1.5;

impl Sweep {
    /// The sweep for part `index` at spots whose body centres lie in
    /// `area` (min x, min y, max x, max y), the others at `poses`.
    pub(crate) fn new(
        problem: &Problem,
        poses: &[Pose],
        reach: &Reach,
        index: usize,
        area: [f64; 4],
    ) -> Self {
        let count = problem.components.len();
        let check =
            cfg!(debug_assertions) || std::env::var_os("PCB_PLACER_CHECK_OVERLAP").is_some();
        if problem.components[index].copper_only {
            return Sweep {
                index,
                near: Vec::new(),
                prefix: Vec::new(),
                direct: true,
                check,
            };
        }
        let mine = SWEEP_FACTOR * (reach.radius[index] + reach.gap);
        let near = (0..count)
            .filter(|&other| {
                if other == index {
                    return false;
                }
                if problem.components[other].copper_only {
                    return true;
                }
                let center = problem.components[other].center(poses[other]);
                let theirs = SWEEP_FACTOR * (reach.radius[other] + reach.gap);
                let apart = [
                    (area[0] - center[0]).max(center[0] - area[2]),
                    (area[1] - center[1]).max(center[1] - area[3]),
                ];
                apart[0].max(apart[1]) < mine + theirs
            })
            .collect();
        // The far path's work: the part moved far off, where every test
        // keeps it clear.
        let away = Pose {
            position: [
                poses[index].position[0] + 1.0e7,
                poses[index].position[1] + 1.0e7,
            ],
            angle: poses[index].angle,
        };
        let body = rect(problem, index, away);
        let linked_work = linked_work(problem);
        let mut prefix = Vec::with_capacity(count + 1);
        let mut total = 0u64;
        prefix.push(0);
        for other in 0..count {
            let mut work = 0;
            let clear = clear_of_one(
                problem,
                poses,
                index,
                away,
                &body,
                linked_work,
                other,
                &mut work,
                None,
            );
            debug_assert!(clear, "part {index} far off meets part {other}");
            total += work;
            prefix.push(total);
        }
        Sweep {
            index,
            near,
            prefix,
            direct: false,
            check,
        }
    }

    /// `is_legal(problem, poses, self.index, pose, 0..count)`, booking
    /// its work. `pose`'s body centre must lie in the sweep's area.
    ///
    /// The near parts apart from the spot by their boxes (`Apart`) take
    /// their far work untested too (the relation repair's near parts were
    /// apart by their boxes at 91 to 95 % of its tests, 2026-10-28).
    pub(crate) fn is_legal(
        &self,
        problem: &Problem,
        poses: &[Pose],
        apart: &mut Apart,
        pose: Pose,
    ) -> bool {
        let count = problem.components.len();
        if self.direct {
            return is_legal(problem, poses, self.index, pose, 0..count);
        }
        let index = self.index;
        let body = rect(problem, index, pose);
        let linked_work = apart.linked_work;
        let mine = apart.board_box(problem, index, pose, &body);
        // `is_legal`'s own unit, then the parts in index order: the far
        // ones by their far work, the near ones tested.
        let mut work = 1;
        let mut next = 0;
        let mut clear = true;
        for &other in &self.near {
            work += self.prefix[other] - self.prefix[next];
            next = other + 1;
            if let Some(mine) = mine
                && let Some(theirs) = apart.other_box(problem, poses, other)
                && boxes_apart(mine, theirs, apart.gap)
            {
                work += self.prefix[other + 1] - self.prefix[other];
                continue;
            }
            if !clear_of_one(
                problem,
                poses,
                index,
                pose,
                &body,
                linked_work,
                other,
                &mut work,
                Some(&apart.facts),
            ) {
                clear = false;
                break;
            }
        }
        let legal = clear && {
            work += self.prefix[count] - self.prefix[next];
            fits_board(problem, index, pose, &mut work, apart.board.as_ref())
        };
        if self.check {
            let mut direct_work = 1;
            let direct = is_legal_counting(problem, poses, index, pose, 0..count, &mut direct_work);
            assert_eq!(
                (legal, work),
                (direct, direct_work),
                "part {index} at {pose:?}: swept (legal, work) against the direct test"
            );
        }
        crate::add_work(work);
        legal
    }
}

/// Debugging aid: why part `index` at `pose` may not sit beside `other`
/// (`None` when it may), by the test that turns it down.
pub fn conflict_kind(
    problem: &Problem,
    poses: &[Pose],
    index: usize,
    pose: Pose,
    other: usize,
) -> Option<&'static str> {
    let mut work = 0;
    if clear_of(
        problem,
        poses,
        index,
        pose,
        std::iter::once(other),
        &mut work,
    ) {
        return None;
    }
    let (mine, theirs) = (&problem.components[index], &problem.components[other]);
    Some(if mine.side.collides(theirs.side) {
        if mine.copper_only || theirs.copper_only {
            "pads in a cutout or pad keepout"
        } else if pads_in_hole_rings(problem, index, pose, other, poses[other], &mut work) {
            "pads in a hole's clearance"
        } else if mine.hollow_for(theirs) || theirs.hollow_for(mine) {
            "shaped or hollow boxes"
        } else if overlaps(
            rect(problem, index, pose).grown(-mine.halo),
            rect(problem, other, poses[other]).grown(-theirs.halo),
            -1.0e-3,
        ) {
            "bodies overlap"
        } else if far_courtyard_overlap(problem, index, pose, other, poses[other], &mut work) {
            "courtyards on the other side"
        } else if problem.constraints.pad_clearance
            && pads_too_close(problem, index, pose, other, poses[other], &mut work)
        {
            "pads closer than the clearance"
        } else {
            "bodies closer than the spacing"
        }
    } else {
        "far side"
    })
}

/// The work of setting up one spot to test (snapping, the body's box, the
/// bucket query), in the units of one part-against-part test.
const SPOT_WORK: usize = 24;

/// The work of a part's box at a pose (its centre and extent each turn
/// it), beyond the test itself.
const RECT_WORK: u64 = 4;

/// The work of looking a pair up among the linked pairs (a search of
/// a set of that size; nothing where there are none), as the annealer
/// counts it.
fn linked_work(problem: &Problem) -> u64 {
    let linked = problem.constraints.linked.len();
    if linked == 0 {
        0
    } else {
        ((linked as f64 + 1.0).log2() / 2.0).round() as u64
    }
}

/// The board's wirelength, counting its work (every pin).
fn counted_wirelength(problem: &Problem, poses: &[Pose]) -> f64 {
    crate::add_work(
        crate::PIN_WORK
            * problem
                .components
                .iter()
                .map(|component| component.pins.len())
                .sum::<usize>() as u64,
    );
    problem.wirelength(poses)
}

/// The relation penalty of one part, counting its work (every relation
/// is looked at).
fn counted_penalty(problem: &Problem, poses: &[Pose], index: usize) -> f64 {
    if problem.constraints.relations.is_empty() {
        return 0.0;
    }
    crate::add_work(
        4 * problem.constraints.relations.len() as u64 + constraints::relation_work(problem, index),
    );
    constraints::relation_penalty(problem, poses, Some(index))
}

/// `counted_penalty` from the relations each part is in (`PartRelations`):
/// the same value and the same work.
fn counted_penalty_of(
    problem: &Problem,
    relations: &constraints::PartRelations,
    poses: &[Pose],
    index: usize,
) -> f64 {
    if problem.constraints.relations.is_empty() {
        return 0.0;
    }
    crate::add_work(4 * problem.constraints.relations.len() as u64 + relations.work(index));
    relations.penalty(problem, poses, index)
}

/// The steps `(dx, dy)` on ring `ring` (`max(|dx|, |dy|) == ring`) within
/// `window` (low x, low y, high x, high y), in the order of a scan of the
/// window's square, rows (`dy`) ascending and each row's `dx` ascending,
/// that skips the steps off the ring: the same steps in the same order
/// without visiting the square's inside (a part without a spot scanned
/// every step of every ring of the board, `(2 r + 1)^2` a ring: most of
/// video's legalisation, 2026-10-28).
fn ring_steps(ring: i64, window: [i64; 4]) -> impl Iterator<Item = (i64, i64)> {
    let [low_x, low_y, high_x, high_y] = window;
    let (from_x, to_x) = ((-ring).max(low_x), ring.min(high_x));
    ((-ring).max(low_y)..=ring.min(high_y)).flat_map(move |dy| {
        let (row, ends): (std::ops::RangeInclusive<i64>, [Option<i64>; 2]) = if dy.abs() == ring {
            (from_x..=to_x, [None, None])
        } else {
            // A row inside the ring: its two ends, where the window holds
            // them (one where the ring is the centre).
            let left = (from_x..=to_x).contains(&-ring).then_some(-ring);
            let right = (ring != 0 && (from_x..=to_x).contains(&ring)).then_some(ring);
            ((to_x + 1)..=to_x, [left, right])
        };
        row.chain(ends.into_iter().flatten())
            .map(move |dx| (dx, dy))
    })
}

/// Placed parts by area: a spot is tested only against parts near it.
struct Buckets {
    grid: crate::buckets::Grid,
    /// How far beyond its rectangle a part meets another (spacing, the
    /// edge margin around cutouts, and some slack).
    reach: f64,
}

impl Buckets {
    fn new(problem: &Problem) -> Self {
        Buckets {
            grid: crate::buckets::Grid::new(problem.bounds(), 2.5, problem.components.len())
                .with_answers(),
            reach: problem.spacing
                + problem.min_spacing
                + problem.edge_margin
                + 1.0
                + problem.far_reach()
                + problem.far_courtyard_reach(),
        }
    }

    fn insert(&mut self, problem: &Problem, index: usize, pose: Pose) {
        let body = rect(problem, index, pose);
        self.grid.insert(index, body.center, body.half);
    }

    fn remove(&mut self, problem: &Problem, index: usize, pose: Pose) {
        let body = rect(problem, index, pose);
        self.grid.remove(index, body.center, body.half);
    }

    /// The parts that may meet part `index` at `pose`. Counts its work:
    /// the cells looked at and the parts found, besides the spot's own
    /// set-up (on SmartSpin2k's panel the legalization's spot searches did
    /// twenty times the work they counted).
    fn near(&mut self, problem: &Problem, index: usize, pose: Pose, out: &mut Vec<usize>) {
        let body = rect(problem, index, pose);
        let cells = self.grid.query(
            body.center,
            [body.half[0] + self.reach, body.half[1] + self.reach],
            out,
        );
        crate::add_work((SPOT_WORK + cells + out.len()) as u64);
    }
}

/// Whether a part's copper comes closer than the edge margin to a cutout
/// (either way round); its body may reach over the hole.
fn copper_meets_cutout(
    problem: &Problem,
    a: usize,
    pose_a: Pose,
    b: usize,
    pose_b: Pose,
    work: &mut u64,
    facts: Option<&Facts>,
) -> bool {
    let (hole, pose_hole, part, pose_part) = if problem.components[a].copper_only {
        (a, pose_a, b, pose_b)
    } else {
        (b, pose_b, a, pose_a)
    };
    if problem.components[part].copper_only {
        return false;
    }
    let margin = problem.edge_margin;
    let outline = &problem.components[hole].cutout_outline;
    let component = &problem.components[part];
    let pads = turned(&component.pads, pose_part, margin);
    *work += list_work(component.pads.len());
    // Its copper graphics too: KiCad checks them against a cutout's edge
    // as it checks pads (osprey's AE1 antenna).
    let graphics = turned(&component.copper_graphics, pose_part, margin);
    if !component.copper_graphics.is_empty() {
        *work += list_work(component.copper_graphics.len());
    }
    let pads = pads.chain(graphics);
    let pad_count = component.pads.len() + component.copper_graphics.len();
    if outline.len() >= 3 {
        // The hole's own outline: its box covers much more board where
        // the hole runs diagonally. A pad outside the outline's box cannot
        // meet it (a tabbed panel's slots run around whole copies, with
        // thousands of points).
        let indexed = facts.and_then(|facts| facts.outlines[hole].as_ref());
        let extent = indexed.map_or_else(|| outline_extent(outline), |indexed| indexed.extent);
        *work += edge_work(outline.len());
        for pad in pads {
            *work += 1;
            if !box_apart(&pad, extent, POLYGON_SLACK)
                && rect_meets_outline(&pad, outline, indexed, work)
            {
                return true;
            }
        }
        return false;
    }
    let hole = rect(problem, hole, pose_hole);
    *work += pad_count as u64;
    pads.into_iter().any(|pad| overlaps(pad, hole, 0.0))
}

/// Whether a part's body (without its halo) meets a cutout the user cut
/// into the outline (`Constraints::solid_cutouts`): a window in the board
/// holds no part, while a designer's slot may hold a connector's pegs.
fn body_meets_solid_cutout(
    problem: &Problem,
    a: usize,
    pose_a: Pose,
    b: usize,
    pose_b: Pose,
    work: &mut u64,
    facts: Option<&Facts>,
) -> bool {
    let solid = &problem.constraints.solid_cutouts;
    if solid.is_empty() {
        return false;
    }
    let (hole, part, pose_part) = if problem.components[a].copper_only {
        (a, b, pose_b)
    } else {
        (b, a, pose_a)
    };
    if problem.components[part].copper_only || !solid.contains(&hole) {
        return false;
    }
    let body = bare(problem, part, pose_part, 0.0);
    let outline = &problem.components[hole].cutout_outline;
    if outline.len() < 3 {
        return false;
    }
    let indexed = facts.and_then(|facts| facts.outlines[hole].as_ref());
    let extent = indexed.map_or_else(|| outline_extent(outline), |indexed| indexed.extent);
    // A body touching the hole's edge is on the board.
    let body = body.grown(-1.0e-6);
    !box_apart(&body, extent, 0.0) && rect_meets_outline(&body, outline, indexed, work)
}

/// The work of building a list of boxes (an allocation, and a turn of each
/// box) in units of one part-against-part test.
fn list_work(boxes: usize) -> u64 {
    8 + 3 * boxes as u64
}

/// The work of one pass over `edges` polygon edges: a few comparisons
/// each, a quarter of a part-against-part test.
fn edge_work(edges: usize) -> u64 {
    (edges as u64).div_ceil(4)
}

/// How far (mm) a polygon's edges must stay from a box for the box's
/// corners to lie on one side of it alike, float rounding included.
const POLYGON_SLACK: f64 = 1.0e-6;

/// Whether the box around `rect` and `extent` ([min x, min y, max x, max
/// y]) are more than `slack` apart.
fn box_apart(rect: &Rect, extent: [f64; 4], slack: f64) -> bool {
    rect.center[0] + rect.half[0] + slack < extent[0]
        || rect.center[0] - rect.half[0] - slack > extent[2]
        || rect.center[1] + rect.half[1] + slack < extent[1]
        || rect.center[1] - rect.half[1] - slack > extent[3]
}

/// The box around an outline's points ([min x, min y, max x, max y]).
fn outline_extent(outline: &[[f64; 2]]) -> [f64; 4] {
    let mut extent = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for point in outline {
        extent = [
            extent[0].min(point[0]),
            extent[1].min(point[1]),
            extent[2].max(point[0]),
            extent[3].max(point[1]),
        ];
    }
    extent
}

/// The box around the edge from `a` to `b`.
fn edge_extent(a: [f64; 2], b: [f64; 2]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[0].max(b[0]),
        a[1].max(b[1]),
    ]
}

/// Whether the edge from `a` to `b` crosses the ray from `point` towards
/// +x (the even-odd rule's step).
fn crosses_ray(a: [f64; 2], b: [f64; 2], point: [f64; 2]) -> bool {
    (a[1] > point[1]) != (b[1] > point[1])
        && point[0] < a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
}

/// A cutout's outline for the first pass of the polygon test
/// (`rect_meets_outline`): its box, and its edges by bands of y, each edge
/// in every band its y range touches. A box's edges near it lie in the
/// bands of its y range, the edges a ray from a point crosses in the band
/// of the point's y: the pass over every edge of a tabbed panel's slots
/// (thousands of points around whole copies), for every pad against them,
/// was two thirds of the 0xCB panel's legalisation (2026-10-28).
pub(crate) struct Outline {
    extent: [f64; 4],
    low: f64,
    /// Bands per millimetre of y.
    scale: f64,
    /// Per band, the edges (edge `i` from point `i` to the next).
    bands: Vec<Vec<u32>>,
    /// Every first pass checked against the pass over every edge (debug
    /// builds and `PCB_PLACER_CHECK_OVERLAP`).
    check: bool,
}

/// Outlines with fewer edges keep the plain pass.
const OUTLINE_INDEX_EDGES: usize = 32;

/// Board outlines with fewer edges keep the plain corner tests.
const BOARD_INDEX_EDGES: usize = 12;

impl Outline {
    fn new(polygon: &[[f64; 2]]) -> Self {
        let extent = outline_extent(polygon);
        let count = (polygon.len() / 4).clamp(1, 1 << 14);
        let height = extent[3] - extent[1];
        let scale = if height > 0.0 {
            count as f64 / height
        } else {
            0.0
        };
        let mut outline = Outline {
            extent,
            low: extent[1],
            scale,
            bands: vec![Vec::new(); count],
            check: cfg!(debug_assertions) || std::env::var_os("PCB_PLACER_CHECK_OVERLAP").is_some(),
        };
        for index in 0..polygon.len() {
            let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
            let (low, high) = (outline.band(a[1].min(b[1])), outline.band(a[1].max(b[1])));
            for band in low..=high {
                outline.bands[band].push(index as u32);
            }
        }
        outline
    }

    /// The band of `y` (monotonic in `y`; NaN and below the outline 0).
    fn band(&self, y: f64) -> usize {
        let at = (y - self.low) * self.scale;
        crate::buckets::cell_index(at).min(self.bands.len() - 1)
    }

    fn edge(polygon: &[[f64; 2]], index: u32) -> ([f64; 2], [f64; 2]) {
        let index = index as usize;
        (polygon[index], polygon[(index + 1) % polygon.len()])
    }

    /// Whether an edge comes near the box around `rect`
    /// (`rect_meets_polygon`'s `near`).
    fn near(&self, rect: &Rect, polygon: &[[f64; 2]]) -> bool {
        let low = self.band(rect.center[1] - rect.half[1] - POLYGON_SLACK);
        let high = self.band(rect.center[1] + rect.half[1] + POLYGON_SLACK);
        self.bands[low..=high].iter().any(|band| {
            band.iter().any(|&edge| {
                let (a, b) = Self::edge(polygon, edge);
                !box_apart(rect, edge_extent(a, b), POLYGON_SLACK)
            })
        })
    }

    /// Whether a polygon corner passes `in_box` (true only inside the box
    /// around `rect`): the corners in the bands of the box's y range (each
    /// edge's first point; a band holds every edge whose y range touches
    /// it), a hair wider for the rounding.
    fn any_corner(
        &self,
        rect: &Rect,
        polygon: &[[f64; 2]],
        in_box: impl Fn(&[f64; 2]) -> bool,
    ) -> bool {
        let low = self.band(rect.center[1] - rect.half[1] - POLYGON_SLACK);
        let high = self.band(rect.center[1] + rect.half[1] + POLYGON_SLACK);
        self.bands[low..=high]
            .iter()
            .any(|band| band.iter().any(|&edge| in_box(&polygon[edge as usize])))
    }

    /// Whether `point` lies inside by the even-odd rule
    /// (`rect_meets_polygon`'s `inside` where no edge is near).
    fn inside(&self, point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
        self.bands[self.band(point[1])]
            .iter()
            .filter(|&&edge| {
                let (a, b) = Self::edge(polygon, edge);
                crosses_ray(a, b, point)
            })
            .count()
            % 2
            == 1
    }
}

/// What the legality test looks up per part, kept for a search: the
/// `Outline` of a cutout's outline with many edges, and whether a part
/// lies on the board's marks, yields to them or is an obstacle without
/// routing room (`Constraints::lies_on`, `halo_free`: binary searches of
/// their lists, two to four a pair test, 7 to 10 % of protoesp's and the
/// 4in1's legalisation, 2026-10-28).
pub(crate) struct Facts {
    outlines: Vec<Option<Outline>>,
    /// Per part: `ON_MARKS`, `YIELDING`, `HALO_FREE`; None where a list
    /// is not ascending (its binary search is then the answer).
    flags: Option<Vec<u8>>,
    /// Whether any part lies on the marks.
    marks: bool,
}

const ON_MARKS: u8 = 1;
const YIELDING: u8 = 2;
const HALO_FREE: u8 = 4;

impl Facts {
    /// `problem.constraints.lies_on(a, b)`.
    fn lies_on(&self, problem: &Problem, a: usize, b: usize) -> bool {
        let Some(flags) = &self.flags else {
            return problem.constraints.lies_on(a, b);
        };
        self.marks
            && ((flags[a] & ON_MARKS != 0 && flags[b] & YIELDING != 0)
                || (flags[b] & ON_MARKS != 0 && flags[a] & YIELDING != 0))
    }

    /// `problem.constraints.bare(a, b)`.
    fn bare(&self, problem: &Problem, a: usize, b: usize) -> bool {
        let Some(flags) = &self.flags else {
            return problem.constraints.bare(a, b);
        };
        problem.constraints.linked(a, b) || flags[a] & HALO_FREE != 0 || flags[b] & HALO_FREE != 0
    }

    pub(crate) fn new(problem: &Problem) -> Self {
        let constraints = &problem.constraints;
        let lists = [
            (&constraints.on_marks, ON_MARKS),
            (&constraints.yielding, YIELDING),
            (&constraints.halo_free, HALO_FREE),
        ];
        let ascending = lists
            .iter()
            .all(|(list, _)| list.windows(2).all(|pair| pair[0] <= pair[1]));
        let flags = ascending.then(|| {
            let mut flags = vec![0u8; problem.components.len()];
            for (list, flag) in lists {
                for &part in list {
                    if let Some(flags) = flags.get_mut(part) {
                        *flags |= flag;
                    }
                }
            }
            flags
        });
        Facts {
            flags,
            marks: !constraints.on_marks.is_empty(),
            outlines: problem
                .components
                .iter()
                .map(|component| {
                    (component.copper_only && component.cutout_outline.len() >= OUTLINE_INDEX_EDGES)
                        .then(|| Outline::new(&component.cutout_outline))
                })
                .collect(),
        }
    }
}

/// Whether a box (turned or not) meets a polygon (overlap or touch), its
/// first pass from the outline's edges by bands
/// (`indexed`, the polygon's `Outline`) where there is one: the same
/// answer and the same work.
fn rect_meets_outline(
    rect: &Rect,
    polygon: &[[f64; 2]],
    indexed: Option<&Outline>,
    work: &mut u64,
) -> bool {
    if indexed.is_some_and(|indexed| indexed.check) {
        let (mut banded_work, mut plain_work) = (0, 0);
        let banded = rect_meets_outline_once(rect, polygon, indexed, &mut banded_work);
        let plain = rect_meets_outline_once(rect, polygon, None, &mut plain_work);
        assert_eq!(
            (banded, banded_work),
            (plain, plain_work),
            "{rect:?}: (meets, work) by bands against the passes over every edge"
        );
        *work += banded_work;
        return banded;
    }
    rect_meets_outline_once(rect, polygon, indexed, work)
}

/// `rect_meets_outline` without its check.
fn rect_meets_outline_once(
    rect: &Rect,
    polygon: &[[f64; 2]],
    indexed: Option<&Outline>,
    work: &mut u64,
) -> bool {
    // One pass first: where no edge comes near the box around the rect,
    // no polygon corner lies in it, no edge crosses it, and its corners
    // lie on one side of the outline alike, so the first corner's side is
    // the answer (the full test below makes nine passes).
    *work += edge_work(polygon.len());
    let corners = rect.corners();
    let first = corners[0];
    let plain = || {
        let mut inside = false;
        let mut near = false;
        for index in 0..polygon.len() {
            let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
            if !box_apart(rect, edge_extent(a, b), POLYGON_SLACK) {
                near = true;
                break;
            }
            if crosses_ray(a, b, first) {
                inside = !inside;
            }
        }
        (near, inside)
    };
    let (near, inside) = match indexed {
        Some(indexed) => {
            let near = indexed.near(rect, polygon);
            (near, !near && indexed.inside(first, polygon))
        }
        None => plain(),
    };
    if !near {
        return inside;
    }
    *work += 9 * edge_work(polygon.len());
    // The full test (an outline by bands: the corners' sides from their
    // bands, the polygon's corners from the bands of the box's y range,
    // an edge's sides' orientations only where the box's corners lie on
    // both sides of it; the same answer).
    let in_polygon = |corner: &[f64; 2]| match indexed {
        Some(indexed) => indexed.inside(*corner, polygon),
        None => crate::problem::point_in_polygon(*corner, polygon),
    };
    if corners.iter().any(in_polygon) {
        return true;
    }
    // A polygon corner inside the box.
    let (axes, own) = rect.frame();
    let in_box = |p: &[f64; 2]| {
        let delta = [p[0] - rect.center[0], p[1] - rect.center[1]];
        (delta[0] * axes[0][0] + delta[1] * axes[0][1]).abs() <= own[0]
            && (delta[0] * axes[1][0] + delta[1] * axes[1][1]).abs() <= own[1]
    };
    if match indexed {
        Some(indexed) => indexed.any_corner(rect, polygon, in_box),
        None => polygon.iter().any(in_box),
    } {
        return true;
    }
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let segments_cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]| {
        let (d1, d2) = (cross(c, d, a), cross(c, d, b));
        let (d3, d4) = (cross(a, b, c), cross(a, b, d));
        (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
    };
    (0..polygon.len()).any(|index| {
        let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
        // Every corner on one side of the edge's line: no side crosses it
        // (`segments_cross`'s second condition, the same orientations).
        let left = corners.map(|corner| cross(a, b, corner) > 0.0);
        left != [left[0]; 4]
            && (0..4).any(|edge| segments_cross(a, b, corners[edge], corners[(edge + 1) % 4]))
    })
}

/// At KiCad's legality (`Constraints::pad_clearance`): whether two parts
/// on one side whose bodies keep clear of each other bring pads closer
/// than the copper clearance (boxes grown on both axes: a little
/// conservative at diagonal corners). Parts whose bodies are farther
/// apart than that cannot.
fn pads_too_close(
    problem: &Problem,
    a: usize,
    pose_a: Pose,
    b: usize,
    pose_b: Pose,
    work: &mut u64,
) -> bool {
    let clearance = problem.min_spacing.max(problem.constraints.pad_mask_gap);
    let (first, second) = (&problem.components[a], &problem.components[b]);
    if first.pads.is_empty() || second.pads.is_empty() {
        return false;
    }
    let bare =
        |part: usize, pose: Pose| rect(problem, part, pose).grown(-problem.components[part].halo);
    if !overlaps(bare(a, pose_a), bare(b, pose_b), clearance) {
        return false;
    }
    let reach = bare(b, pose_b);
    // No list unless a pad comes near the other part.
    if !turned(&first.pads, pose_a, 0.0).any(|pad| overlaps(pad, reach, clearance)) {
        return false;
    }
    let mine: Vec<(usize, Rect)> = turned(&first.pads, pose_a, 0.0)
        .enumerate()
        .filter(|(_, pad)| overlaps(*pad, reach, clearance))
        .collect();
    let theirs = turned_rects(&second.pads, pose_b, 0.0);
    *work += list_work(first.pads.len())
        + list_work(second.pads.len())
        + (mine.len() * theirs.len()) as u64;
    let close: Vec<(usize, usize)> = mine
        .iter()
        .flat_map(|(index, pad)| {
            theirs
                .iter()
                .enumerate()
                .filter(|(_, other)| overlaps(*pad, **other, clearance))
                .map(move |(other, _)| (*index, other))
        })
        .collect();
    if close.is_empty() {
        return false;
    }
    // KiCad keeps the clearance between copper of different nets: two
    // pads of one net may come closer (mating connectors of neighbouring
    // cells, the molecules' J3 and J4, 0.03 mm apart).
    let (nets_a, nets_b) = (pad_nets(first), pad_nets(second));
    *work += (first.pads.len() * first.pins.len() + second.pads.len() * second.pins.len()) as u64;
    close.iter().any(|(a, b)| match (nets_a[*a], nets_b[*b]) {
        (Some(x), Some(y)) => x != y,
        _ => true,
    })
}

/// Per pad of a part (`Component::pads`), the net of the pin inside it
/// (None: no pin, a pad of no net).
fn pad_nets(component: &crate::problem::Component) -> Vec<Option<usize>> {
    component
        .pads
        .iter()
        .map(|pad| {
            component
                .pins
                .iter()
                .find(|pin| {
                    pin.offset[0] >= pad[0] - 1.0e-9
                        && pin.offset[0] <= pad[2] + 1.0e-9
                        && pin.offset[1] >= pad[1] - 1.0e-9
                        && pin.offset[1] <= pad[3] + 1.0e-9
                })
                .map(|pin| pin.net)
        })
        .collect()
}

/// Whether one part's pads come into the other's non-plated holes'
/// clearance rings (`Component::hole_rings`) closer than the copper
/// clearance: the hole clearance, whatever the bodies' spacing at the
/// level (a mouse bite's rings reach past its courtyard).
fn pads_in_hole_rings(
    problem: &Problem,
    a: usize,
    pose_a: Pose,
    b: usize,
    pose_b: Pose,
    work: &mut u64,
) -> bool {
    let clearance = problem.min_spacing;
    let mut meets = |ringed: usize, ring_pose: Pose, padded: usize, pad_pose: Pose| {
        let (rings, pads) = (
            &problem.components[ringed].hole_rings,
            &problem.components[padded].pads,
        );
        if rings.is_empty() || pads.is_empty() {
            return false;
        }
        let reach = rect(problem, padded, pad_pose);
        *work += list_work(rings.len());
        // No list unless a ring comes near the other part.
        if !turned(rings, ring_pose, 0.0).any(|ring| overlaps(ring, reach, clearance)) {
            return false;
        }
        let near: Vec<Rect> = turned(rings, ring_pose, 0.0)
            .filter(|ring| overlaps(*ring, reach, clearance))
            .collect();
        let pads = turned_rects(pads, pad_pose, 0.0);
        *work += list_work(pads.len()) + (near.len() * pads.len()) as u64;
        near.iter()
            .any(|ring| pads.iter().any(|pad| overlaps(*ring, *pad, clearance)))
    };
    meets(a, pose_a, b, pose_b) || meets(b, pose_b, a, pose_a)
}

/// Whether two parts on the same side meet where one of them is hollow:
/// only its blocking boxes count.
fn hollow_overlap(
    problem: &Problem,
    a: usize,
    pose_a: Pose,
    b: usize,
    pose_b: Pose,
    work: &mut u64,
) -> bool {
    // What a part blocks the other with, and how many boxes: its
    // rectangle (against an obstacle without routing room, without the
    // halo), or its hollow boxes and, against a part with holes, its
    // courtyards.
    let blocking = |index: usize, pose: Pose, other: usize| {
        let component = &problem.components[index];
        let empty: &[[f64; 4]] = &[];
        if !component.hollow_for(&problem.components[other]) {
            let halo = if problem.constraints.halo_free(other) {
                component.halo
            } else {
                0.0
            };
            let whole = rect(problem, index, pose).grown(-halo);
            return (
                1,
                Some(whole)
                    .into_iter()
                    .chain(turned(empty, pose, 0.0).chain(turned(empty, pose, 0.0))),
            );
        }
        let against = &problem.components[other];
        let courtyards: &[[f64; 4]] = if against.has_holes() && !component.holes_inside {
            &component.courtyards
        } else {
            empty
        };
        (
            component.hollow.len() + courtyards.len(),
            None.into_iter()
                .chain(turned(&component.hollow, pose, 0.0).chain(turned(courtyards, pose, 0.0))),
        )
    };
    let (their_count, theirs) = blocking(b, pose_b, a);
    let (my_count, mine) = blocking(a, pose_a, b);
    // The lists, and every pair at worst.
    *work += list_work(my_count) + list_work(their_count) + (my_count * their_count) as u64;
    // At KiCad's legality the boxes may touch, the pads keeping their
    // clearance (`pads_too_close`).
    let spacing = if problem.constraints.pad_clearance {
        0.0
    } else {
        problem.spacing
    };
    with_rects(theirs, |theirs| {
        mine.into_iter()
            .any(|mine| theirs.iter().any(|other| overlaps(mine, *other, spacing)))
    })
}

/// Whether a part's holes and pads on the far side meet the other part's
/// body there (either way round), for parts on opposite sides.
fn far_side_overlap(
    problem: &Problem,
    a: usize,
    pose_a: Pose,
    b: usize,
    pose_b: Pose,
    work: &mut u64,
) -> bool {
    let meets = |holes: usize, pose_holes: Pose, body: usize, pose_body: Pose, work: &mut u64| {
        let component = &problem.components[holes];
        if component.far_side.is_empty() && component.far_copper.is_empty() {
            return false;
        }
        let other = &problem.components[body];
        let far = turned(&component.far_side, pose_holes, 0.0);
        let copper = turned(&component.far_copper, pose_holes, 0.0);
        let (far_count, copper_count) = (component.far_side.len(), component.far_copper.len());
        *work += list_work(far_count) + list_work(copper_count);
        // The other part's copper: its pads, or all of it where it has
        // none (artwork, a copper graphic, a rule area).
        let empty: &[[f64; 4]] = &[];
        let pads = || {
            if other.pads.is_empty() {
                (
                    1,
                    Some(rect(problem, body, pose_body))
                        .into_iter()
                        .chain(turned(empty, pose_body, 0.0)),
                )
            } else {
                (
                    other.pads.len(),
                    None.into_iter().chain(turned(&other.pads, pose_body, 0.0)),
                )
            }
        };
        if problem.far_side_pads_only
            || (problem.constraints.pad_clearance && problem.constraints.leads_keep_copper)
        {
            // Copper against copper only (at KiCad's legality the leads are
            // copper to the other side).
            let (pad_count, pads) = pads();
            *work += list_work(pad_count);
            *work += ((far_count + copper_count) * pad_count) as u64;
            return with_rects(pads, |pads| {
                far.chain(copper).any(|far| {
                    pads.iter()
                        .any(|pad| overlaps(far, *pad, problem.min_spacing))
                })
            });
        }
        let target = rect(problem, body, pose_body);
        *work += far_count as u64;
        if far
            .into_iter()
            .any(|far| overlaps(far, target, problem.spacing))
        {
            return true;
        }
        // Copper that only copper may not come near.
        copper_count > 0 && {
            let (pad_count, pads) = pads();
            *work += list_work(pad_count);
            *work += (copper_count * pad_count) as u64;
            with_rects(pads, |pads| {
                copper.into_iter().any(|far| {
                    pads.iter()
                        .any(|pad| overlaps(far, *pad, problem.min_spacing))
                })
            })
        }
    };
    meets(a, pose_a, b, pose_b, work) || meets(b, pose_b, a, pose_a, work)
}

/// Whether two parts on the same side meet on the other side: one's
/// courtyard there (`far_courtyard`) against the other's courtyard, holes
/// and pads there, the gap their bodies keep. KiCad checks courtyards on
/// both sides: woagboard's back-mounted choc switches have their 14 mm
/// squares on the front, larger than their sockets on the back, and the
/// bodies alone let the squares overlap.
fn far_courtyard_overlap(
    problem: &Problem,
    a: usize,
    pose_a: Pose,
    b: usize,
    pose_b: Pose,
    work: &mut u64,
) -> bool {
    let meets = |yard: usize, pose_yard: Pose, other: usize, pose_other: Pose, work: &mut u64| {
        let (component, against) = (&problem.components[yard], &problem.components[other]);
        if component.far_courtyard.is_empty() || against.far_side.is_empty() {
            return false;
        }
        *work += (component.far_courtyard.len() * against.far_side.len()) as u64;
        let gap = body_spacing(problem, yard, other);
        let mut yards = turned(&component.far_courtyard, pose_yard, 0.0);
        with_rects(turned(&against.far_side, pose_other, 0.0), |theirs| {
            yards.any(|yard| theirs.iter().any(|their| overlaps(yard, *their, gap)))
        })
    };
    meets(a, pose_a, b, pose_b, work) || meets(b, pose_b, a, pose_a, work)
}

pub fn illegal_components(problem: &Problem, poses: &[Pose]) -> Vec<usize> {
    (0..problem.components.len())
        .filter(|index| {
            !is_legal(
                problem,
                poses,
                *index,
                poses[*index],
                0..problem.components.len(),
            )
        })
        .collect()
}

/// Where the body centre of `index` turned to `angle` may lie by its
/// regions ([min x, min y, max x, max y], as `constraints::hard_violation`
/// tests them, a hair wider), or None for a part without one. A position
/// whose centre lies outside is illegal: the searches skip it untested (a
/// panel's copies put a region on each movable part; a part without room
/// searched the whole board at every pass, SmartSpin2k's nine copies nine
/// times its own).
fn region_window(problem: &Problem, index: usize, angle: f64) -> Option<[f64; 4]> {
    let mut window: Option<[f64; 4]> = None;
    for (part, region) in &problem.constraints.regions {
        if *part != index {
            continue;
        }
        let half = problem.components[index].half_extent(angle);
        let slack = 1.0e-5;
        let this = [
            region[0] + half[0] - slack,
            region[1] + half[1] - slack,
            region[2] - half[0] + slack,
            region[3] - half[1] + slack,
        ];
        window = Some(match window {
            None => this,
            Some(window) => [
                window[0].max(this[0]),
                window[1].max(this[1]),
                window[2].min(this[2]),
                window[3].min(this[3]),
            ],
        });
    }
    window
}

/// Whether `center` lies in `window` (`region_window`; None: anywhere).
fn in_window(window: Option<[f64; 4]>, center: Point) -> bool {
    window.is_none_or(|window| {
        center[0] >= window[0]
            && center[1] >= window[1]
            && center[0] <= window[2]
            && center[1] <= window[3]
    })
}

/// The lattice steps (dx, dy) from a search's origin (whose body centre is
/// `center`) that `window` admits, one step wider on each side for the
/// rounding: [min dx, min dy, max dx, max dy]. None: no window.
fn window_steps(window: Option<[f64; 4]>, center: Point, step: f64) -> Option<[i64; 4]> {
    let window = window?;
    Some([
        ((window[0] - center[0]) / step).floor() as i64 - 1,
        ((window[1] - center[1]) / step).floor() as i64 - 1,
        ((window[2] - center[0]) / step).ceil() as i64 + 1,
        ((window[3] - center[1]) / step).ceil() as i64 + 1,
    ])
}

/// The rings (Chebyshev distances from the origin) holding a step of
/// `steps`: [nearest, farthest]; an empty window none (nearest > farthest).
fn window_rings(steps: [i64; 4]) -> (i64, i64) {
    if steps[0] > steps[2] || steps[1] > steps[3] {
        return (1, 0);
    }
    let nearest = |low: i64, high: i64| {
        if low <= 0 && 0 <= high {
            0
        } else {
            low.abs().min(high.abs())
        }
    };
    let farthest = |low: i64, high: i64| low.abs().max(high.abs());
    (
        nearest(steps[0], steps[2]).max(nearest(steps[1], steps[3])),
        farthest(steps[0], steps[2]).max(farthest(steps[1], steps[3])),
    )
}

/// How much farther than the closest legal spot the legaliser looks for
/// one with fewer ratsnest crossings, mm.
const CROSSING_REACH: f64 = 2.0;

/// Moves every movable component to the nearest legal, grid-snapped position,
/// largest bodies first. Returns the components that found no position.
pub fn legalize(problem: &Problem, poses: &mut [Pose]) -> Vec<usize> {
    legalize_first(problem, poses, &[])
}

/// `legalize`, placing the parts of `first` before all others (the ones a
/// previous pass could not place: the big parts, placed first, can leave
/// them no room).
pub fn legalize_first(problem: &Problem, poses: &mut [Pose], first: &[usize]) -> Vec<usize> {
    legalize_keeping(problem, poses, first, &[])
}

/// `legalize_first`, with the parts of `keep` left at their poses (legal
/// already: a looser level of the same placement) and placed before all
/// others.
pub fn legalize_keeping(
    problem: &Problem,
    poses: &mut [Pose],
    first: &[usize],
    keep: &[usize],
) -> Vec<usize> {
    let count = problem.components.len();
    let mut placed: Vec<usize> = (0..count)
        .filter(|index| problem.components[*index].fixed || keep.contains(index))
        .collect();
    let mut order: Vec<usize> = (0..count)
        .filter(|index| !problem.components[*index].fixed && !keep.contains(index))
        .collect();
    order.sort_by(|a, b| {
        let area = |index: &usize| {
            let size = problem.components[*index].body_size;
            size[0] * size[1]
        };
        let rank = |index: &usize| {
            first
                .iter()
                .position(|part| part == index)
                .unwrap_or(usize::MAX)
        };
        rank(a)
            .cmp(&rank(b))
            .then(area(b).total_cmp(&area(a)))
            .then(a.cmp(b))
    });
    if problem.constraints.follow_anchors && !problem.constraints.relations.is_empty() {
        order = follow_order(problem, poses, &order, first);
    }
    let bounds = problem.bounds();
    let step = if problem.grid > 0.0 {
        problem.grid
    } else {
        0.25
    };
    let mut rings = (((bounds[2] - bounds[0]).max(bounds[3] - bounds[1])) / step).ceil() as i64 + 1;
    // A fine grid is a last resort after the coarse one searched the whole
    // board: it only looks near where the part wants to be (a part with no
    // spot scanned millions of positions otherwise).
    if step < 0.3 {
        rings = rings.min((15.0 / step).ceil() as i64);
    }
    // Past the placement's work budget, as many rings as 15 mm of a 0.5 mm
    // grid (3 mm of the fine grid).
    let hurried_rings = (15.0 / step.max(0.5)).ceil() as i64;
    let mut buckets = Buckets::new(problem);
    for &index in &placed {
        buckets.insert(problem, index, poses[index]);
    }
    let mut apart = Apart::new(problem);
    // Escape demand: a spot whose channels overflow is that much farther.
    let mut escape = problem.escape.as_deref().map(|escape| {
        let mut present = vec![false; count];
        for &index in &placed {
            present[index] = true;
        }
        crate::escape::EscapeState::new(problem, escape, poses, |index| present[index])
    });
    // Ratsnest crossings: a spot is farther by the crossings it adds to
    // those of the part's nets where it wants to be (the parts not placed
    // yet where they want to be too).
    let mut crossings = problem
        .crossings
        .as_deref()
        .filter(|crossings| crossings.legalise)
        .map(|crossings| crate::crossings::CrossingState::new(problem, crossings, poses));
    let mut near = Vec::new();
    let mut failed = Vec::new();
    // Near relations follow their anchors (`Constraints::follow_anchors`):
    // the poses wanted, and which parts are legalised so far.
    let start: Vec<Pose> = if problem.constraints.follow_anchors {
        poses.to_vec()
    } else {
        Vec::new()
    };
    let mut done = vec![false; count];
    for &index in &placed {
        done[index] = true;
    }
    for index in order {
        let component = &problem.components[index];
        let annealed = poses[index];
        if !start.is_empty()
            && let Some(shift) =
                constraints::anchor_shift(problem, &start, poses, index, |part| done[part])
        {
            poses[index].position[0] += shift[0];
            poses[index].position[1] += shift[1];
        }
        let wanted = poses[index];
        let wanted_center = component.center(wanted);
        let mut best: Option<(f64, Pose)> = None;
        // The closest legal spot's distance without the crossings: the
        // crossings choose among the spots up to `CROSSING_REACH` farther,
        // not the whole board (a spot's crossings would otherwise widen
        // the search by their weight).
        let mut plain: Option<f64> = None;
        let wanted_crossings = crossings
            .as_mut()
            .map_or(0, |state| state.involving_or_kept(poses, &[index]));
        // A part near another (`follow_anchors`) pays for the near
        // relations it misses at a spot: the closest spot that keeps them
        // beats a closer one that does not.
        let own_relations: Vec<&constraints::Relation> = if start.is_empty() {
            Vec::new()
        } else {
            problem
                .constraints
                .relations
                .iter()
                .filter(|relation| {
                    matches!(
                        relation,
                        constraints::Relation::Near { part, .. }
                            | constraints::Relation::NearAny { part, .. } if *part == index
                    )
                })
                .collect()
        };
        for angle in &component.angle_options {
            // Rotating about the body centre keeps the part where global
            // placement wanted it; prefer the global angle on ties.
            let penalty = if (angle - wanted.angle).abs() < 1.0e-9 {
                0.0
            } else {
                step * step
            };
            let ideal = component.position_for_center(wanted_center, *angle);
            let origin = constraints::snap_position(problem, index, ideal, *angle);
            let window = window_steps(
                region_window(problem, index, *angle),
                component.center(Pose {
                    position: origin,
                    angle: *angle,
                }),
                step,
            );
            let (nearest, farthest) = window.map_or((0, rings), window_rings);
            'search: for ring in 0..=rings.min(farthest) {
                if ring < nearest {
                    continue;
                }
                // Once a legal spot exists, only slightly farther rings can
                // still hold a closer Euclidean match.
                if let Some((distance, _)) = best
                    && ((ring - 2).max(0) as f64 * step).powi(2) > distance
                {
                    break 'search;
                }
                if let Some(plain) = plain
                    && (ring - 2).max(0) as f64 * step > plain.sqrt() + CROSSING_REACH
                {
                    break 'search;
                }
                // Past the placement's work budget a part looks only near
                // where it wants to be: a part with no spot searched the
                // whole board (urchin on 80 x 40 mm: 40 parts, 45 s a pass
                // on the fine grid).
                if best.is_none() && ring > hurried_rings && crate::out_of_work(0) {
                    break 'search;
                }
                // Only the steps the part's regions admit (the same order).
                let [low_x, low_y, high_x, high_y] = window.unwrap_or([-ring, -ring, ring, ring]);
                for (dx, dy) in ring_steps(ring, [low_x, low_y, high_x, high_y]) {
                    let pose = Pose {
                        position: [origin[0] + dx as f64 * step, origin[1] + dy as f64 * step],
                        angle: *angle,
                    };
                    let distance = (pose.position[0] - ideal[0]).powi(2)
                        + (pose.position[1] - ideal[1]).powi(2)
                        + penalty;
                    if best.is_some_and(|(best, _)| best <= distance) {
                        continue;
                    }
                    // Legal first, then the escape overflow: on a
                    // crowded board most spots are illegal, and the
                    // overflow (each pin's way out, the channels with
                    // the parts around) cost several times the test
                    // there (katia: a pass of 134 s against 21 s
                    // without escape demand, the deadline before the
                    // level that places it). The spot chosen is the
                    // same: the least distance of the legal spots.
                    buckets.near(problem, index, pose, &mut near);
                    if !apart.is_legal(problem, poses, index, pose, near.iter().copied()) {
                        continue;
                    }
                    let distance = if own_relations.is_empty() {
                        distance
                    } else {
                        poses[index] = pose;
                        let missed: f64 = own_relations
                            .iter()
                            .map(|relation| {
                                constraints::relation_violation(problem, poses, relation)
                            })
                            .sum();
                        poses[index] = wanted;
                        crate::add_work(constraints::relation_work(problem, index));
                        let total = (distance.sqrt() + FOLLOW_WEIGHT * missed).powi(2);
                        if best.is_some_and(|(best, _)| best <= total) {
                            continue;
                        }
                        total
                    };
                    let distance = match escape.as_mut() {
                        None => distance,
                        Some(state) => {
                            let wanted = poses[index];
                            poses[index] = pose;
                            let overflow = state.cost(poses, index);
                            poses[index] = wanted;
                            let total = (distance.sqrt() + overflow).powi(2);
                            if best.is_some_and(|(best, _)| best <= total) {
                                continue;
                            }
                            total
                        }
                    };
                    let distance = match crossings.as_mut() {
                        None => distance,
                        Some(state) => {
                            if plain.is_none_or(|plain| distance < plain) {
                                plain = Some(distance);
                            }
                            poses[index] = pose;
                            let added = state
                                .involving(poses, &[index])
                                .saturating_sub(wanted_crossings);
                            poses[index] = wanted;
                            if added == 0 {
                                distance
                            } else {
                                let total =
                                    (distance.sqrt() + state.weight() * added as f64).powi(2);
                                if best.is_some_and(|(best, _)| best <= total) {
                                    continue;
                                }
                                total
                            }
                        }
                    };
                    best = Some((distance, pose));
                }
            }
        }
        match best {
            Some((_, pose)) => poses[index] = pose,
            None => {
                poses[index] = annealed;
                failed.push(index);
            }
        }
        done[index] = true;
        placed.push(index);
        buckets.insert(problem, index, poses[index]);
        if let Some(state) = escape.as_mut() {
            state.insert(index, poses[index]);
            state.refresh(poses, index);
        }
        if let Some(state) = crossings.as_mut()
            && poses[index] != annealed
        {
            state.kept(poses, &[index]);
        }
    }
    failed
}

/// Millimetres of displacement a millimetre of a missed near relation
/// costs a follower's spot in the legaliser (`Constraints::follow_anchors`).
const FOLLOW_WEIGHT: f64 = 4.0;

/// The legaliser's order with near relations followed
/// (`Constraints::follow_anchors`): a part near another
/// (`constraints::anchor_leader`) goes right after it, its followers by
/// `order`'s order (larger first), so that an IC's decoupling capacitors
/// take the spots at its pins before the parts that merely connect to it;
/// one near a part placed already (fixed, kept) goes after the parts of
/// `first`. Parts of `first` keep their place.
fn follow_order(problem: &Problem, poses: &[Pose], order: &[usize], first: &[usize]) -> Vec<usize> {
    let count = problem.components.len();
    let mut pending = vec![false; count];
    for &index in order {
        pending[index] = true;
    }
    let mut followers: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut early = Vec::new();
    let mut follows = vec![false; count];
    for &index in order {
        if first.contains(&index) {
            continue;
        }
        let Some(leader) = constraints::anchor_leader(problem, poses, index) else {
            continue;
        };
        follows[index] = true;
        if pending[leader] {
            followers[leader].push(index);
        } else {
            early.push(index);
        }
    }
    let mut result = Vec::with_capacity(order.len());
    let mut emitted = vec![false; count];
    let emit = |index: usize, result: &mut Vec<usize>, emitted: &mut Vec<bool>| {
        // The part, then its followers and theirs, depth first.
        let mut stack = vec![index];
        while let Some(part) = stack.pop() {
            if emitted[part] {
                continue;
            }
            emitted[part] = true;
            result.push(part);
            stack.extend(followers[part].iter().rev().copied());
        }
    };
    let ranked = order.iter().take_while(|index| first.contains(index));
    for &index in ranked {
        emit(index, &mut result, &mut emitted);
    }
    for &index in &early {
        emit(index, &mut result, &mut emitted);
    }
    for &index in order {
        if !follows[index] {
            emit(index, &mut result, &mut emitted);
        }
    }
    // Followers of a cycle of followers.
    for &index in order {
        emit(index, &mut result, &mut emitted);
    }
    result
}

/// Legalizes `annealed` with the parts of `failed` first; a part placed
/// first can take the spot of another that then finds none, and that one
/// joins the parts going first in the next pass, up to `passes` passes
/// while new parts fail and the work lasts. Returns the poses and the
/// parts without room of the pass that left the fewest (the first on
/// ties).
pub fn legalize_stuck_first(
    problem: &Problem,
    annealed: &[Pose],
    failed: &[usize],
    passes: usize,
    debug: bool,
) -> (Vec<Pose>, Vec<usize>) {
    let mut first = failed.to_vec();
    let mut best: Option<(Vec<Pose>, Vec<usize>)> = None;
    for _ in 0..passes.max(1) {
        let mut retry = annealed.to_vec();
        let again = legalize_first(problem, &mut retry, &first);
        if debug {
            eprintln!("placer: legalize first {first:?}: no room for {again:?}");
        }
        let newly: Vec<usize> = again
            .iter()
            .copied()
            .filter(|part| !first.contains(part))
            .collect();
        let done = again.is_empty() || newly.is_empty();
        if best
            .as_ref()
            .is_none_or(|(_, least)| again.len() < least.len())
        {
            best = Some((retry, again));
        }
        if done || crate::out_of_work(0) {
            break;
        }
        first.extend(newly);
    }
    best.expect("one pass at least")
}

/// The scans of `evict_for` for a spot of `index`: the coarse `step` on
/// both axes and, for a part held at one edge (or at two parallel ones)
/// on a finer grid, then the grid's step along that edge with the part
/// pulled flush against it (`clamp`). A held part has a window across
/// the edge (between the edge margin and its reach) and, at the
/// edge-copper levels where its copper keeps the copper-to-edge clearance
/// on every side, one along it, both narrower than the coarse step can hit
/// (OM-FlexGrid-Flex's 19-pin flex connector J1 in its 12.4 mm tab: 0.25
/// mm wide, between the coarse scan's x positions; the molecules'
/// DisplayPort connectors, 0.45 mm across the edge).
fn scan_steps(problem: &Problem, index: usize, step: f64) -> Vec<(f64, f64, bool)> {
    let mut steps = vec![(step, step, false)];
    let fine = problem.grid;
    if fine <= 0.0 || fine >= step {
        return steps;
    }
    let mut along = [false; 2];
    for (part, edge, _) in &problem.constraints.edges {
        if *part == index {
            match edge {
                constraints::Edge::Top | constraints::Edge::Bottom => along[0] = true,
                constraints::Edge::Left | constraints::Edge::Right => along[1] = true,
            }
        }
    }
    match along {
        [true, false] => steps.push((fine, step, true)),
        [false, true] => steps.push((step, fine, true)),
        _ => {}
    }
    steps
}

/// One scan of `evict_for` over the board at `steps` for a spot of
/// `index`: the spot whose displaced movable parts are smallest in area
/// (`best`, kept when no spot displaces less).
#[allow(clippy::too_many_arguments)]
fn evict_scan(
    problem: &Problem,
    poses: &[Pose],
    index: usize,
    bounds: [f64; 4],
    steps: [f64; 2],
    clamp: bool,
    buckets: &mut Buckets,
    apart: &mut Apart,
    near: &mut Vec<usize>,
    work: &mut u64,
    best: &mut Option<(f64, Pose, Vec<usize>)>,
) {
    let component = &problem.components[index];
    let mut y = bounds[1];
    while y <= bounds[3] {
        let mut x = bounds[0];
        while x <= bounds[2] {
            for angle in &component.angle_options {
                let mut at = [x, y];
                if clamp {
                    let center = component.center(Pose {
                        position: at,
                        angle: *angle,
                    });
                    let flush = constraints::flush_center(problem, index, center, *angle);
                    at = component.position_for_center(flush, *angle);
                }
                let pose = Pose {
                    position: constraints::snap_position(problem, index, at, *angle),
                    angle: *angle,
                };
                if !in_window(
                    region_window(problem, index, *angle),
                    component.center(pose),
                ) {
                    continue;
                }
                *work += SPOT_WORK as u64 + held_work(problem, index);
                if !fits_board(problem, index, pose, work, apart.board.as_ref()) {
                    continue;
                }
                buckets.near(problem, index, pose, near);
                let fixed = near
                    .iter()
                    .copied()
                    .filter(|other| problem.components[*other].fixed);
                if !apart.clear_of(problem, poses, index, pose, fixed, work) {
                    continue;
                }
                let mut movable: Vec<usize> = near
                    .iter()
                    .copied()
                    .filter(|other| !problem.components[*other].fixed)
                    .collect();
                movable.sort_unstable();
                let mut area = 0.0;
                let mut displaced = Vec::new();
                for other in movable {
                    if !apart.clear_of(problem, poses, index, pose, std::iter::once(other), work) {
                        let size = problem.components[other].body_size;
                        area += size[0] * size[1];
                        displaced.push(other);
                        if best.as_ref().is_some_and(|(best, _, _)| area >= *best) {
                            break;
                        }
                    }
                }
                if best.as_ref().is_none_or(|(best, _, _)| area < *best) {
                    *best = Some((area, pose, displaced));
                }
            }
            x += steps[0];
        }
        y += steps[1];
    }
}

/// Places each part of `failed` where it displaces the least area of
/// movable parts (never a fixed one), then finds the displaced parts new
/// spots; keeps that only if all of them find one. A part whose spot is
/// taken by others finds no room by searching around them (katia's back
/// side: the designer's spots for its controllers existed, held by parts
/// placed first). Returns the parts still without room.
pub fn evict_for(problem: &Problem, poses: &mut [Pose], failed: &[usize]) -> Vec<usize> {
    let count = problem.components.len();
    let bounds = problem.bounds();
    let step = if problem.grid > 0.0 {
        problem.grid.max(0.5)
    } else {
        0.5
    };
    let mut still = Vec::new();
    let mut placed: Vec<usize> = (0..count).filter(|index| !failed.contains(index)).collect();
    // Every spot is tested against the parts near it only (SmartSpin2k's
    // panel, 890 parts: one eviction tested every part at every spot and
    // took 20 minutes).
    let mut buckets = Buckets::new(problem);
    for &other in &placed {
        buckets.insert(problem, other, poses[other]);
    }
    let mut apart = Apart::new(problem);
    let mut near = Vec::new();
    let mut work = 0;
    for &index in failed {
        // Past the work budget no more parts are tried.
        if crate::out_of_work(work) {
            still.push(index);
            continue;
        }
        // The spot whose displaced movable parts are smallest in area.
        let mut best: Option<(f64, Pose, Vec<usize>)> = None;
        for (x_step, y_step, clamp) in scan_steps(problem, index, step) {
            // The finer scan along a held edge only where the coarse one
            // found no spot.
            if best.is_some() {
                break;
            }
            evict_scan(
                problem,
                poses,
                index,
                bounds,
                [x_step, y_step],
                clamp,
                &mut buckets,
                &mut apart,
                &mut near,
                &mut work,
                &mut best,
            );
        }
        let Some((_, pose, displaced)) = best else {
            still.push(index);
            continue;
        };
        let saved: Vec<Pose> = poses.to_vec();
        poses[index] = pose;
        for &moved in &displaced {
            buckets.remove(problem, moved, saved[moved]);
        }
        buckets.insert(problem, index, pose);
        let mut seated = Vec::new();
        let mut all = true;
        for &moved in &displaced {
            let component = &problem.components[moved];
            let mut spot: Option<(f64, Pose)> = None;
            let wanted = poses[moved].position;
            let mut y = bounds[1];
            while y <= bounds[3] {
                let mut x = bounds[0];
                while x <= bounds[2] {
                    for angle in &component.angle_options {
                        let candidate = Pose {
                            position: constraints::snap_position(problem, moved, [x, y], *angle),
                            angle: *angle,
                        };
                        if !in_window(
                            region_window(problem, moved, *angle),
                            component.center(candidate),
                        ) {
                            continue;
                        }
                        work += SPOT_WORK as u64 + held_work(problem, moved);
                        let distance = (candidate.position[0] - wanted[0]).powi(2)
                            + (candidate.position[1] - wanted[1]).powi(2);
                        if spot.is_some_and(|(best, _)| best <= distance) {
                            continue;
                        }
                        if !fits_board(problem, moved, candidate, &mut work, apart.board.as_ref()) {
                            continue;
                        }
                        buckets.near(problem, moved, candidate, &mut near);
                        if apart.clear_of(
                            problem,
                            poses,
                            moved,
                            candidate,
                            near.iter().copied(),
                            &mut work,
                        ) {
                            spot = Some((distance, candidate));
                        }
                    }
                    x += step;
                }
                y += step;
            }
            match spot {
                Some((_, candidate)) => {
                    poses[moved] = candidate;
                    buckets.insert(problem, moved, candidate);
                    seated.push(moved);
                }
                None => {
                    all = false;
                    break;
                }
            }
        }
        if all {
            placed.push(index);
        } else {
            for &moved in &seated {
                buckets.remove(problem, moved, poses[moved]);
            }
            buckets.remove(problem, index, pose);
            for &moved in &displaced {
                buckets.insert(problem, moved, saved[moved]);
            }
            poses.copy_from_slice(&saved);
            still.push(index);
        }
    }
    crate::add_work(work);
    still
}

/// For every part that misses a near or relative constraint, searches legal
/// positions around what the relation wants and takes the one with the
/// least relation penalty plus wirelength, if that beats where it is.
/// Returns the number of parts moved.
pub fn repair_relations(problem: &Problem, poses: &mut [Pose]) -> usize {
    let constraints = &problem.constraints;
    if constraints.relations.is_empty() {
        return 0;
    }
    let step = if problem.grid > 0.0 {
        problem.grid
    } else {
        0.25
    };
    let mut moved = 0;
    let sweep_reach = Reach::new(problem);
    let mut apart = Apart::new(problem);
    for relation in &constraints.relations {
        if crate::out_of_work(0) {
            break;
        }
        let Some(index) = relation.parts()[0] else {
            continue;
        };
        if problem.components[index].fixed
            || constraints::relation_violation(problem, poses, relation) <= 0.0
        {
            continue;
        }
        let component = &problem.components[index];
        // What the user asked for comes first: the relation's violation,
        // then the other relations and the wire (a chain's end held near
        // its start stayed 0.24 mm short when wire and penalty were summed).
        let score = |poses: &[Pose]| {
            (
                constraints::relation_violation(problem, poses, relation),
                counted_penalty(problem, poses, index) + counted_wirelength(problem, poses),
            )
        };
        let better = |a: (f64, f64), b: (f64, f64)| {
            a.0 < b.0 - 1.0e-9 || (a.0 <= b.0 + 1.0e-9 && a.1 < b.1 - 1.0e-9)
        };
        let original = poses[index];
        let mut best = (score(poses), original);
        let target = constraints::relation_target(problem, poses, relation);
        let reach = component.body_size[0].max(component.body_size[1]) + 6.0;
        let rings = (reach / step).ceil() as i64;
        // The spots' body centres, for the sweep's legality tests.
        let mut area = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for angle in &component.angle_options {
            let ideal = component.position_for_center(target, *angle);
            let origin = constraints::snap_position(problem, index, ideal, *angle);
            let center = component.center(Pose {
                position: origin,
                angle: *angle,
            });
            let span = rings as f64 * step + 1.0e-6;
            area = [
                area[0].min(center[0] - span),
                area[1].min(center[1] - span),
                area[2].max(center[0] + span),
                area[3].max(center[1] + span),
            ];
        }
        let sweep = Sweep::new(problem, poses, &sweep_reach, index, area);
        for angle in &component.angle_options {
            let ideal = component.position_for_center(target, *angle);
            let origin = constraints::snap_position(problem, index, ideal, *angle);
            for ring in 0..=rings {
                for (dx, dy) in ring_steps(ring, [-ring, -ring, ring, ring]) {
                    let pose = Pose {
                        position: [origin[0] + dx as f64 * step, origin[1] + dy as f64 * step],
                        angle: *angle,
                    };
                    if !sweep.is_legal(problem, poses, &mut apart, pose) {
                        continue;
                    }
                    poses[index] = pose;
                    let candidate = score(poses);
                    poses[index] = original;
                    if better(candidate, best.0) {
                        best = (candidate, pose);
                    }
                }
            }
        }
        if best.1 != original {
            poses[index] = best.1;
            moved += 1;
        }
    }
    moved
}

/// Turns parts of the same kind (same body and pin count, same side) to
/// their kind's most common angle where that stays legal, costs at most
/// `tolerance` mm of wire per part and misses no soft constraint more:
/// consistent orientation eases assembly and inspection. Returns the number
/// of parts turned.
pub fn align_orientations(problem: &Problem, poses: &mut [Pose], tolerance: f64) -> usize {
    let count = problem.components.len();
    let key = |index: usize| {
        let component = &problem.components[index];
        (
            (component.body_size[0] * 100.0).round() as i64,
            (component.body_size[1] * 100.0).round() as i64,
            component.pins.len(),
            component.side as u8,
        )
    };
    let mut kinds: std::collections::BTreeMap<(i64, i64, usize, u8), Vec<usize>> =
        Default::default();
    for index in 0..count {
        let component = &problem.components[index];
        if !component.fixed && component.pins.len() >= 2 && component.angle_options.len() > 1 {
            kinds.entry(key(index)).or_default().push(index);
        }
    }
    let angle_key = |angle: f64| (angle.rem_euclid(360.0).round() as i64) % 360;
    let mut turned = 0;
    for members in kinds.values().filter(|members| members.len() >= 2) {
        if crate::out_of_work(0) {
            break;
        }
        let mut counts: std::collections::BTreeMap<i64, usize> = Default::default();
        for index in members {
            *counts.entry(angle_key(poses[*index].angle)).or_default() += 1;
        }
        let (&modal, _) = counts
            .iter()
            .max_by_key(|(angle, count)| (**count, -**angle))
            .unwrap();
        for &index in members {
            if angle_key(poses[index].angle) == modal {
                continue;
            }
            let component = &problem.components[index];
            let Some(&angle) = component
                .angle_options
                .iter()
                .find(|option| angle_key(**option) == modal)
            else {
                continue;
            };
            let center = component.center(poses[index]);
            let position = constraints::snap_position(
                problem,
                index,
                component.position_for_center(center, angle),
                angle,
            );
            let pose = Pose { position, angle };
            if !is_legal(problem, poses, index, pose, 0..count) {
                continue;
            }
            let before = (
                counted_wirelength(problem, poses),
                counted_penalty(problem, poses, index),
            );
            let original = poses[index];
            poses[index] = pose;
            let after = (
                counted_wirelength(problem, poses),
                counted_penalty(problem, poses, index),
            );
            if after.0 <= before.0 + tolerance && after.1 <= before.1 + 1.0e-9 {
                turned += 1;
            } else {
                poses[index] = original;
            }
        }
    }
    turned
}

/// Local search on a legal placement: every move keeps it legal and strictly
/// reduces the weighted half-perimeter wirelength.
pub fn refine(problem: &Problem, poses: &mut [Pose], passes: usize) -> usize {
    let mut escape = problem
        .escape
        .as_deref()
        .map(|escape| crate::escape::EscapeState::new(problem, escape, poses, |_| true));
    let mut crossings = problem
        .crossings
        .as_deref()
        .filter(|crossings| crossings.refine)
        .map(|crossings| crate::crossings::CrossingState::new(problem, crossings, poses));
    refine_with(
        problem,
        poses,
        passes,
        escape.as_mut(),
        crossings.as_mut(),
        None,
    )
}

/// `refine`, with escape demand (`Problem::escape`) kept in `escape`
/// across calls (its measured correction): a move pays the channel
/// overflow it causes.
pub fn refine_with(
    problem: &Problem,
    poses: &mut [Pose],
    passes: usize,
    mut escape: Option<&mut crate::escape::EscapeState>,
    mut crossings: Option<&mut crate::crossings::CrossingState>,
    groups: Option<&crate::groups::BusGroups>,
) -> usize {
    if let Some(state) = escape.as_mut() {
        state.resync(poses);
    }
    if let Some(state) = crossings.as_mut() {
        state.resync(poses);
    }
    // The crossings of the nets on `parts`' pins at `poses`, in
    // millimetres of wirelength.
    let crossing_cost =
        |state: &mut crate::crossings::CrossingState, poses: &[Pose], parts: &[usize]| {
            state.weight() * state.involving(poses, parts) as f64
        };
    // The same at the poses kept, from the kept lines where they are
    // (`CrossingState::involving_or_kept`).
    let kept_crossing_cost =
        |state: &mut crate::crossings::CrossingState, poses: &[Pose], parts: &[usize]| {
            state.weight() * state.involving_or_kept(poses, parts) as f64
        };
    let count = problem.components.len();
    let nets = problem.net_weights.len();
    let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nets];
    for (index, component) in problem.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            members[description.net].push((index, pin));
        }
    }
    let incident: Vec<Vec<usize>> = problem
        .components
        .iter()
        .map(|component| {
            let mut nets: Vec<usize> = component.pins.iter().map(|pin| pin.net).collect();
            nets.sort_unstable();
            nets.dedup();
            nets
        })
        .collect();
    let cost = |poses: &[Pose], nets: &[usize]| -> f64 {
        crate::add_work(
            crate::PIN_WORK * nets.iter().map(|net| members[*net].len()).sum::<usize>() as u64,
        );
        nets.iter()
            .map(|net| net_cost(problem, &members, *net, poses))
            .sum()
    };

    let mut buckets = Buckets::new(problem);
    for (index, pose) in poses.iter().enumerate().take(count) {
        buckets.insert(problem, index, *pose);
    }
    let relations = constraints::PartRelations::new(problem);
    let mut apart = Apart::new(problem);
    let mut near = Vec::new();
    let step = if problem.grid > 0.0 {
        problem.grid
    } else {
        0.25
    };
    // Spots around a part's target are searched this far: legalization on
    // a crowded board leaves parts far from their nets, and the straight
    // line back is often taken (Sisu's feedback divider 46 mm from its
    // regulator).
    let rings = ((3.0 / step).ceil() as i64).max(1);
    let mut improvements = 0;
    for _ in 0..passes {
        let mut improved = false;
        for index in 0..count {
            // Past the work budget the placement stands as it is (legal).
            if crate::out_of_work(0) {
                break;
            }
            let component = &problem.components[index];
            if component.fixed || component.pins.is_empty() {
                continue;
            }
            // Where the other pins of this component's nets pull it.
            let mut target = [0.0; 2];
            let mut weight = 0.0;
            for net in &incident[index] {
                for (other, pin) in &members[*net] {
                    if *other == index {
                        continue;
                    }
                    let at = problem.components[*other]
                        .pin_position(&problem.components[*other].pins[*pin], poses[*other]);
                    let pull = problem.net_weights[*net] / (members[*net].len() - 1) as f64;
                    target[0] += pull * at[0];
                    target[1] += pull * at[1];
                    weight += pull;
                }
            }
            if weight == 0.0 {
                continue;
            }
            target = [target[0] / weight, target[1] / weight];
            let original = poses[index];
            // Only this part moves among its candidates: its nets' other
            // pins are folded once (`MovingNets`), `cost` of its nets.
            let moving = MovingNets::new(problem, &members, &incident[index], index, poses);
            // Its exits' targets, likewise (`EscapeState::prepare`).
            if let Some(state) = escape.as_mut() {
                state.prepare(poses, index);
            }
            let score = |poses: &[Pose]| {
                moving.cost(problem, &members, poses)
                    + counted_penalty_of(problem, &relations, poses, index)
            };
            let mut best = (score(poses), original);
            if let Some(state) = escape.as_mut() {
                best.0 += state.cost(poses, index);
            }
            if let Some(state) = crossings.as_mut() {
                best.0 += kept_crossing_cost(state, poses, &[index]);
            }
            let center = component.center(original);
            // Besides the pull of its nets, a part that misses a relation
            // is drawn towards what it relates to.
            let mut targets = vec![target];
            for relation in &problem.constraints.relations {
                if relation.parts()[0] != Some(index)
                    || constraints::relation_violation(problem, poses, relation) <= 0.0
                {
                    continue;
                }
                targets.push(constraints::relation_target(problem, poses, relation));
            }
            for (angle, target) in component
                .angle_options
                .iter()
                .flat_map(|angle| targets.iter().map(move |target| (angle, *target)))
            {
                for fraction in [1.0, 0.75, 0.5, 0.25, 0.125, 0.0] {
                    let wanted = [
                        center[0] + fraction * (target[0] - center[0]),
                        center[1] + fraction * (target[1] - center[1]),
                    ];
                    let position = component.position_for_center(wanted, *angle);
                    let pose = Pose {
                        position: constraints::snap_position(problem, index, position, *angle),
                        angle: *angle,
                    };
                    if pose == original {
                        continue;
                    }
                    poses[index] = pose;
                    let mut candidate = score(poses);
                    if candidate < best.0 - 1.0e-9
                        && let Some(state) = escape.as_mut()
                    {
                        candidate += state.cost(poses, index);
                    }
                    if candidate < best.0 - 1.0e-9 {
                        buckets.near(problem, index, pose, &mut near);
                        if apart.is_legal(problem, poses, index, pose, near.iter().copied()) {
                            if let Some(state) = crossings.as_mut() {
                                candidate += crossing_cost(state, poses, &[index]);
                            }
                            if candidate < best.0 - 1.0e-9 {
                                best = (candidate, pose);
                            }
                        }
                    }
                }
                // Then the nearest free spots around the target itself.
                let ideal = component.position_for_center(target, *angle);
                let origin = constraints::snap_position(problem, index, ideal, *angle);
                for ring in 0..=rings {
                    for (dx, dy) in ring_steps(ring, [-ring, -ring, ring, ring]) {
                        let pose = Pose {
                            position: [origin[0] + dx as f64 * step, origin[1] + dy as f64 * step],
                            angle: *angle,
                        };
                        if pose == original {
                            continue;
                        }
                        poses[index] = pose;
                        let mut candidate = score(poses);
                        if candidate < best.0 - 1.0e-9
                            && let Some(state) = escape.as_mut()
                        {
                            candidate += state.cost(poses, index);
                        }
                        if candidate < best.0 - 1.0e-9 {
                            buckets.near(problem, index, pose, &mut near);
                            if apart.is_legal(problem, poses, index, pose, near.iter().copied()) {
                                if let Some(state) = crossings.as_mut() {
                                    candidate += crossing_cost(state, poses, &[index]);
                                }
                                if candidate < best.0 - 1.0e-9 {
                                    best = (candidate, pose);
                                }
                            }
                        }
                    }
                }
            }
            if let Some(state) = escape.as_mut() {
                state.unprepare();
            }
            poses[index] = best.1;
            if best.1 != original {
                buckets.remove(problem, index, original);
                buckets.insert(problem, index, best.1);
                improved = true;
                improvements += 1;
                if let Some(state) = escape.as_mut() {
                    state.kept(poses, &[index]);
                }
                if let Some(state) = crossings.as_mut() {
                    state.kept(poses, &[index]);
                }
            }
        }

        // Exchange equally sized parts.
        for a in 0..count {
            for b in a + 1..count {
                let (first, second) = (&problem.components[a], &problem.components[b]);
                if first.fixed
                    || second.fixed
                    || first.side != second.side
                    || first.round != second.round
                    || !first.angle_options.contains(&poses[b].angle)
                    || !second.angle_options.contains(&poses[a].angle)
                    || (first.body_size[0] - second.body_size[0]).abs() > 1.0e-6
                    || (first.body_size[1] - second.body_size[1]).abs() > 1.0e-6
                    || (first.body_center[0] - second.body_center[0]).abs() > 1.0e-6
                    || (first.body_center[1] - second.body_center[1]).abs() > 1.0e-6
                {
                    continue;
                }
                let mut nets = incident[a].clone();
                nets.extend(&incident[b]);
                nets.sort_unstable();
                nets.dedup();
                let pair = |poses: &[Pose]| {
                    cost(poses, &nets)
                        + counted_penalty_of(problem, &relations, poses, a)
                        + counted_penalty_of(problem, &relations, poses, b)
                };
                let mut before = pair(poses);
                if let Some(state) = escape.as_mut() {
                    before += state.cost_without(poses, a, b) + state.cost_without(poses, b, a);
                }
                if let Some(state) = crossings.as_mut() {
                    before += kept_crossing_cost(state, poses, &[a, b]);
                }
                poses.swap(a, b);
                let mut after = pair(poses);
                if after < before - 1.0e-9
                    && let Some(state) = escape.as_mut()
                {
                    after += state.cost_without(poses, a, b) + state.cost_without(poses, b, a);
                }
                if after < before - 1.0e-9
                    && let Some(state) = crossings.as_mut()
                {
                    after += crossing_cost(state, poses, &[a, b]);
                }
                if after < before - 1.0e-9
                    && is_legal(problem, poses, a, poses[a], 0..count)
                    && is_legal(problem, poses, b, poses[b], 0..count)
                {
                    buckets.remove(problem, a, poses[b]);
                    buckets.remove(problem, b, poses[a]);
                    buckets.insert(problem, a, poses[a]);
                    buckets.insert(problem, b, poses[b]);
                    improved = true;
                    improvements += 1;
                    if let Some(state) = escape.as_mut() {
                        state.kept(poses, &[a, b]);
                    }
                    if let Some(state) = crossings.as_mut() {
                        state.kept(poses, &[a, b]);
                    }
                } else {
                    poses.swap(a, b);
                }
            }
        }
        // Bus groups as units (`crate::groups`).
        if let Some(groups) = groups.filter(|groups| groups.settings.refine) {
            let kept = refine_groups(
                problem,
                poses,
                groups,
                &mut buckets,
                &cost,
                escape.as_deref_mut(),
                crossings.as_deref_mut(),
            );
            if kept > 0 {
                improved = true;
                improvements += kept;
            }
        }
        if !improved {
            break;
        }
    }
    improvements
}

/// A net's weighted half perimeter at `poses` (0 below two pins): the
/// refinement's wirelength term of one net.
fn net_cost(problem: &Problem, members: &[Vec<(usize, usize)>], net: usize, poses: &[Pose]) -> f64 {
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (other, pin) in &members[net] {
        let at = problem.components[*other]
            .pin_position(&problem.components[*other].pins[*pin], poses[*other]);
        bounds[0] = bounds[0].min(at[0]);
        bounds[1] = bounds[1].min(at[1]);
        bounds[2] = bounds[2].max(at[0]);
        bounds[3] = bounds[3].max(at[1]);
    }
    if members[net].len() < 2 {
        0.0
    } else {
        problem.net_weights[net] * ((bounds[2] - bounds[0]) + (bounds[3] - bounds[1]))
    }
}

/// The refinement's wirelength of one part's nets while only that part
/// moves (its candidate spots and turns): per net, the box of the pins on
/// other parts folded once, so that a candidate turns only the part's own
/// pins (a candidate turned every pin of its nets, a power net's hundreds
/// among them: the refinement's cost was 3 to 5 % of a placement's
/// instructions, 2026-10-28). The same value bit for bit as `net_cost`
/// of each net summed in order: a minimum or maximum of numbers that are
/// neither NaN nor zero is the same whatever the order they are folded in
/// (equal ones have the same bits), and a net with a coordinate of either
/// zero (whose sign the fold's order could pick) is folded directly.
struct MovingNets {
    part: usize,
    nets: Vec<MovingNet>,
    /// The work `cost` books: as many pins as the nets have.
    work: u64,
}

struct MovingNet {
    net: usize,
    /// The other pins' box (min x, min y, max x, max y).
    bounds: [f64; 4],
    /// The part's own pins on the net.
    own: Vec<usize>,
    /// Folded directly (a zero among the other pins' coordinates).
    direct: bool,
}

impl MovingNets {
    fn new(
        problem: &Problem,
        members: &[Vec<(usize, usize)>],
        nets: &[usize],
        part: usize,
        poses: &[Pose],
    ) -> Self {
        let nets = nets
            .iter()
            .map(|&net| {
                let mut bounds = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                let mut own = Vec::new();
                let mut direct = false;
                for &(other, pin) in &members[net] {
                    if other == part {
                        own.push(pin);
                        continue;
                    }
                    let at = problem.components[other]
                        .pin_position(&problem.components[other].pins[pin], poses[other]);
                    direct |= at[0] == 0.0 || at[1] == 0.0;
                    bounds[0] = bounds[0].min(at[0]);
                    bounds[1] = bounds[1].min(at[1]);
                    bounds[2] = bounds[2].max(at[0]);
                    bounds[3] = bounds[3].max(at[1]);
                }
                MovingNet {
                    net,
                    bounds,
                    own,
                    direct,
                }
            })
            .collect::<Vec<_>>();
        let work =
            crate::PIN_WORK * nets.iter().map(|net| members[net.net].len()).sum::<usize>() as u64;
        MovingNets { part, nets, work }
    }

    /// The refinement's `cost` of the nets at `poses` (the other parts
    /// where `new` found them), counting the same work.
    fn cost(&self, problem: &Problem, members: &[Vec<(usize, usize)>], poses: &[Pose]) -> f64 {
        crate::add_work(self.work);
        let component = &problem.components[self.part];
        let pose = poses[self.part];
        self.nets
            .iter()
            .map(|moving| {
                let net = moving.net;
                if members[net].len() < 2 {
                    return 0.0;
                }
                if moving.direct {
                    return net_cost(problem, members, net, poses);
                }
                let mut bounds = moving.bounds;
                for &pin in &moving.own {
                    let at = component.pin_position(&component.pins[pin], pose);
                    if at[0] == 0.0 || at[1] == 0.0 {
                        return net_cost(problem, members, net, poses);
                    }
                    bounds[0] = bounds[0].min(at[0]);
                    bounds[1] = bounds[1].min(at[1]);
                    bounds[2] = bounds[2].max(at[0]);
                    bounds[3] = bounds[3].max(at[1]);
                }
                problem.net_weights[net] * ((bounds[2] - bounds[0]) + (bounds[3] - bounds[1]))
            })
            .sum()
    }
}

/// The refinement's bus-group pass (`crate::groups`): each group with more
/// than one part (its parts at the poses of now) tries to shift towards
/// the pull of its nets (the whole way, half, a quarter), to turn about
/// its centre, to trade places with each group like it and, a bank, to
/// reorder its members along its axis; each candidate placed as a unit
/// (`place_unit`) and judged by the wirelength of the moved parts' nets,
/// their relations and their nets' crossings. The best that lowers that
/// cost is kept. Escape demand does not judge these moves (the parts'
/// own moves after it do); it is kept up to date. Returns the moves kept.
fn refine_groups(
    problem: &Problem,
    poses: &mut [Pose],
    groups: &crate::groups::BusGroups,
    buckets: &mut Buckets,
    cost: &dyn Fn(&[Pose], &[usize]) -> f64,
    mut escape: Option<&mut crate::escape::EscapeState>,
    mut crossings: Option<&mut crate::crossings::CrossingState>,
) -> usize {
    use crate::groups::{Kind, Reorder, chain_center, pull, reordered, rotated, translated};
    let step = if problem.grid > 0.0 {
        problem.grid
    } else {
        0.25
    };
    let rings = ((3.0 / step).ceil() as i64).clamp(1, 12);
    let incident: Vec<Vec<usize>> = problem
        .components
        .iter()
        .map(|component| {
            let mut nets: Vec<usize> = component.pins.iter().map(|pin| pin.net).collect();
            nets.sort_unstable();
            nets.dedup();
            nets
        })
        .collect();
    let margin = |part: usize| problem.components[part].halo + problem.spacing / 2.0;
    let mut kept = 0;
    let mut parts = groups.attach(problem, poses);
    for index in 0..groups.groups.len() {
        if crate::out_of_work(0) {
            break;
        }
        let group = &groups.groups[index];
        let unit = &parts[index];
        if group.kind == Kind::Cluster && unit.len() < 2 {
            continue;
        }
        let mut candidates: Vec<Vec<(usize, Pose)>> = Vec::new();
        if let Some(towards) = pull(problem, poses, unit) {
            for fraction in [1.0, 0.5, 0.25] {
                let shift = [towards[0] * fraction, towards[1] * fraction];
                if shift[0].abs().max(shift[1].abs()) >= step / 2.0 {
                    candidates.push(translated(problem, poses, unit, shift));
                }
            }
        }
        let center = chain_center(problem, poses, &group.chain);
        for turn in [90.0, 180.0, 270.0] {
            if let Some(moved) = rotated(problem, poses, unit, center, turn) {
                candidates.push(moved);
            }
        }
        // Swaps with the nearest few peers.
        let mut peers: Vec<(f64, usize)> = group
            .peers
            .iter()
            .map(|other| {
                let at = chain_center(problem, poses, &groups.groups[*other].chain);
                (
                    (at[0] - center[0]).powi(2) + (at[1] - center[1]).powi(2),
                    *other,
                )
            })
            .collect();
        peers.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        peers.truncate(REFINE_PEERS);
        crate::add_work(4 * group.peers.len() as u64);
        for &(_, other) in &peers {
            let theirs = &parts[other];
            if unit.iter().any(|part| theirs.binary_search(part).is_ok()) {
                continue;
            }
            let at = chain_center(problem, poses, &groups.groups[other].chain);
            let mut moved =
                translated(problem, poses, unit, [at[0] - center[0], at[1] - center[1]]);
            moved.extend(translated(
                problem,
                poses,
                theirs,
                [center[0] - at[0], center[1] - at[1]],
            ));
            candidates.push(moved);
        }
        if group.kind == Kind::Bank {
            for how in [Reorder::InPlace, Reorder::Along(0), Reorder::Along(1)] {
                if let Some(moved) = reordered(problem, poses, group, unit, how, &margin) {
                    candidates.push(moved);
                }
            }
        }
        let mut best: Option<(f64, Vec<(usize, Pose)>)> = None;
        for wanted in candidates {
            let mut list: Vec<usize> = wanted.iter().map(|(part, _)| *part).collect();
            list.sort_unstable();
            let mut nets: Vec<usize> = list
                .iter()
                .flat_map(|part| incident[*part].iter().copied())
                .collect();
            nets.sort_unstable();
            nets.dedup();
            let score = |poses: &[Pose]| {
                cost(poses, &nets) + crate::groups::held_cost(problem, poses, &list)
            };
            let mut current = score(poses);
            if let Some(state) = crossings.as_mut() {
                current += state.weight() * state.involving(poses, &list) as f64;
            }
            let Some(placed) = place_unit_in(problem, poses, buckets, &wanted, rings) else {
                continue;
            };
            let old: Vec<(usize, Pose)> = placed
                .iter()
                .map(|(part, _)| (*part, poses[*part]))
                .collect();
            for (part, pose) in &placed {
                poses[*part] = *pose;
            }
            let mut candidate = score(poses);
            let floor = best.as_ref().map_or(1.0e-9, |(gain, _)| *gain);
            if current - candidate > floor
                && let Some(state) = crossings.as_mut()
            {
                candidate += state.weight() * state.involving(poses, &list) as f64;
            }
            let gain = current - candidate;
            if gain > floor {
                best = Some((gain, placed));
            }
            for (part, pose) in old {
                poses[part] = pose;
            }
        }
        if let Some((_, placed)) = best {
            let list: Vec<usize> = placed.iter().map(|(part, _)| *part).collect();
            for (part, pose) in &placed {
                buckets.remove(problem, *part, poses[*part]);
                poses[*part] = *pose;
                buckets.insert(problem, *part, *pose);
            }
            if let Some(state) = escape.as_mut() {
                state.kept(poses, &list);
            }
            if let Some(state) = crossings.as_mut() {
                state.kept(poses, &list);
            }
            kept += 1;
            parts = groups.attach(problem, poses);
        }
    }
    kept
}

/// The refinement's group pass tries swaps with this many of a group's
/// peers, the nearest.
const REFINE_PEERS: usize = 4;

/// Places a group of parts as a unit (the bus-group moves,
/// `crate::groups`): `wanted` gives each part's pose; the largest part
/// goes to the nearest legal spot around its own (the whole unit shifted
/// with it, in rings of the grid out to `rings`), then each other part
/// at its pose shifted alike, or where that is taken at the nearest legal
/// spot within a millimetre of it. A unit whose largest part finds a legal
/// spot and whose others do not is tried at the next few spots. Returns
/// the poses, or `None` when the unit does not fit there. Legal against
/// every part outside the unit (at `poses`) and among the unit's parts.
pub fn place_unit(
    problem: &Problem,
    poses: &[Pose],
    wanted: &[(usize, Pose)],
    rings: i64,
) -> Option<Vec<(usize, Pose)>> {
    let mut buckets = Buckets::new(problem);
    for (index, pose) in poses.iter().enumerate() {
        buckets.insert(problem, index, *pose);
    }
    place_unit_in(problem, poses, &mut buckets, wanted, rings)
}

/// The anchor spots `place_unit` tries at most.
const UNIT_TRIES: usize = 6;

/// `place_unit` with the parts at `poses` in `buckets`.
fn place_unit_in(
    problem: &Problem,
    poses: &[Pose],
    buckets: &mut Buckets,
    wanted: &[(usize, Pose)],
    rings: i64,
) -> Option<Vec<(usize, Pose)>> {
    if wanted.is_empty() {
        return Some(Vec::new());
    }
    let mut unit: Vec<usize> = wanted.iter().map(|(part, _)| *part).collect();
    unit.sort_unstable();
    let in_unit = |part: usize| unit.binary_search(&part).is_ok();
    let mut order: Vec<(usize, Pose)> = wanted.to_vec();
    order.sort_by(|(a, _), (b, _)| {
        let area = |index: usize| {
            let size = problem.components[index].body_size;
            size[0] * size[1]
        };
        area(*b).total_cmp(&area(*a)).then(a.cmp(b))
    });
    let step = if problem.grid > 0.0 {
        problem.grid
    } else {
        0.25
    };
    let member_rings = ((1.0 / step).ceil() as i64).max(1);
    let mut scratch = poses.to_vec();
    crate::add_work(poses.len() as u64 / 4);
    let mut near = Vec::new();
    let ring_offsets = |ring: i64| {
        let mut offsets: Vec<(i64, i64)> = Vec::new();
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs().max(dy.abs()) == ring {
                    offsets.push((dx, dy));
                }
            }
        }
        offsets.sort_by_key(|(dx, dy)| (dx * dx + dy * dy, *dy, *dx));
        offsets
    };
    let shifted = |part: usize, pose: Pose, shift: Point| Pose {
        position: constraints::snap_position(
            problem,
            part,
            [pose.position[0] + shift[0], pose.position[1] + shift[1]],
            pose.angle,
        ),
        angle: pose.angle,
    };
    // Whether `part` at `pose` is legal against the parts outside the unit
    // and the unit's parts placed so far.
    let mut legal = |scratch: &[Pose], placed: &[(usize, Pose)], part: usize, pose: Pose| {
        buckets.near(problem, part, pose, &mut near);
        near.retain(|other| !in_unit(*other));
        is_legal(
            problem,
            scratch,
            part,
            pose,
            near.iter()
                .copied()
                .chain(placed.iter().map(|(other, _)| *other)),
        )
    };
    let (anchor, anchor_pose) = order[0];
    let mut tries = 0;
    for ring in 0..=rings {
        for (dx, dy) in ring_offsets(ring) {
            let shift = [dx as f64 * step, dy as f64 * step];
            let pose = shifted(anchor, anchor_pose, shift);
            if !legal(&scratch, &[], anchor, pose) {
                continue;
            }
            tries += 1;
            let mut placed = vec![(anchor, pose)];
            scratch[anchor] = pose;
            let mut fits = true;
            for &(part, wanted_pose) in &order[1..] {
                let target = shifted(part, wanted_pose, shift);
                let mut found = None;
                'search: for member_ring in 0..=member_rings {
                    for (mx, my) in ring_offsets(member_ring) {
                        let pose = shifted(part, target, [mx as f64 * step, my as f64 * step]);
                        if legal(&scratch, &placed, part, pose) {
                            found = Some(pose);
                            break 'search;
                        }
                    }
                }
                match found {
                    Some(pose) => {
                        scratch[part] = pose;
                        placed.push((part, pose));
                    }
                    None => {
                        fits = false;
                        break;
                    }
                }
            }
            if fits {
                return Some(placed);
            }
            for (part, _) in &placed {
                scratch[*part] = poses[*part];
            }
            if tries >= UNIT_TRIES {
                return None;
            }
        }
    }
    None
}

/// Parts placed by their copper at an edge (a card edge in its tab) have
/// little play along that edge: each with less than 2 mm moves to the
/// middle of the stretch where it stays legal, away from the tab's sides.
pub fn center_edge_copper(problem: &Problem, poses: &mut [Pose]) {
    if !problem.constraints.edge_copper {
        return;
    }
    let step = if problem.grid > 0.0 {
        problem.grid
    } else {
        0.05
    };
    let everyone = 0..problem.components.len();
    for (part, edge, _) in problem.constraints.edges.clone() {
        if problem.components[part].fixed || problem.components[part].pads.is_empty() {
            continue;
        }
        let pose = poses[part];
        if !is_legal(problem, poses, part, pose, everyone.clone()) {
            continue;
        }
        // Along the edge: x for the top and bottom, y for the sides.
        let axis = match edge {
            constraints::Edge::Top | constraints::Edge::Bottom => 0,
            constraints::Edge::Left | constraints::Edge::Right => 1,
        };
        let reach = |direction: f64| {
            let mut last = 0.0;
            for count in 1..=200 {
                let mut moved = pose;
                moved.position[axis] += direction * step * count as f64;
                if !is_legal(problem, poses, part, moved, everyone.clone()) {
                    break;
                }
                last = direction * step * count as f64;
            }
            last
        };
        let (low, high) = (reach(-1.0), reach(1.0));
        if high - low > 2.0 {
            continue;
        }
        let middle = ((low + high) / 2.0 / step).round() * step;
        let mut centered = pose;
        centered.position[axis] += middle;
        if middle != 0.0 && is_legal(problem, poses, part, centered, everyone.clone()) {
            poses[part] = centered;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::{Component, Side};

    /// A 2 x 1 mm body (courtyard and pads) with two 0.6 x 0.6 mm pads at
    /// its ends, `inset` inside its courtyard on the long sides.
    fn resistor(inset: f64) -> Component {
        Component {
            name: "R".into(),
            body_center: [0.0, 0.0],
            body_size: [2.0, 1.0],
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side: Side::Front,
            fixed: false,
            angle_options: vec![0.0],
            far_side: Vec::new(),
            far_copper: Vec::new(),
            far_courtyard: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: vec![
                [-1.0 + inset, -0.3, -0.4 + inset, 0.3],
                [0.4 - inset, -0.3, 1.0 - inset, 0.3],
            ],
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
            hole_rings: Vec::new(),
            copper_graphics: Vec::new(),
        }
    }

    fn pair(inset: f64, gap: f64) -> (Problem, Vec<Pose>) {
        let poses = vec![
            Pose {
                position: [10.0, 10.0],
                angle: 0.0,
            },
            Pose {
                position: [12.0 + gap, 10.0],
                angle: 0.0,
            },
        ];
        let problem = Problem {
            outline: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 40.0], [0.0, 40.0]],
            components: vec![resistor(inset), resistor(inset)],
            net_weights: Vec::new(),
            poses: poses.clone(),
            spacing: 0.2,
            grid: 0.1,
            edge_margin: 0.5,
            min_spacing: 0.2,
            constraints: Default::default(),
            far_side_pads_only: false,
            shared_halos: false,
            pieces: Vec::new(),
            escape: None,
            crossings: None,
        };
        (problem, poses)
    }

    #[test]
    fn a_sweep_answers_and_books_as_the_direct_test() {
        let mut state = 0xD1B5_4A32_D192_ED03u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for round in 0..6 {
            let parts = 40;
            let (mut problem, _) = pair(0.0, 0.0);
            problem.outline = vec![[0.0, 0.0], [30.0, 0.0], [30.0, 30.0], [0.0, 30.0]];
            problem.constraints.pad_clearance = round % 2 == 1;
            problem.components = (0..parts)
                .map(|number| {
                    let mut part = resistor(0.0);
                    part.halo = if next() < 0.5 { 0.2 } else { 0.0 };
                    part.angle_options = vec![0.0, 90.0, 45.0];
                    part.side = match number % 4 {
                        0 => Side::Back,
                        1 => Side::Both,
                        _ => Side::Front,
                    };
                    part.fixed = next() < 0.2;
                    if next() < 0.3 {
                        part.far_side = vec![[-0.5, -0.5, 0.5, 0.5]];
                    }
                    if next() < 0.2 {
                        part.far_copper = vec![[-3.0, -0.2, -2.0, 0.2]];
                    }
                    if next() < 0.2 {
                        part.hollow = vec![[-1.0, -0.5, -0.6, 0.5], [0.6, -0.5, 1.0, 0.5]];
                        part.courtyards = vec![[-1.0, -0.5, 1.0, 0.5]];
                    }
                    if next() < 0.2 {
                        part.hole_rings = vec![[1.5, 1.5, 2.5, 2.5]];
                    }
                    if next() < 0.2 {
                        part.far_courtyard = vec![[-1.5, -1.0, 1.5, 1.0]];
                    }
                    part
                })
                .collect();
            let poses: Vec<Pose> = (0..parts)
                .map(|_| Pose {
                    position: [next() * 30.0, next() * 30.0],
                    angle: [0.0, 90.0, 45.0][(next() * 3.0) as usize],
                })
                .collect();
            problem.poses = poses.clone();
            let reach = Reach::new(&problem);
            let mut apart = Apart::new(&problem);
            apart.check = false;
            for index in 0..parts {
                let corner = [next() * 26.0, next() * 26.0];
                let area = [corner[0], corner[1], corner[0] + 4.0, corner[1] + 4.0];
                let mut sweep = Sweep::new(&problem, &poses, &reach, index, area);
                sweep.check = false;
                for _ in 0..40 {
                    let angle = [0.0, 90.0, 45.0][(next() * 3.0) as usize];
                    let center = [
                        area[0] + next() * (area[2] - area[0]),
                        area[1] + next() * (area[3] - area[1]),
                    ];
                    let component = &problem.components[index];
                    let pose = Pose {
                        position: component.position_for_center(center, angle),
                        angle,
                    };
                    let before = crate::work();
                    let swept = sweep.is_legal(&problem, &poses, &mut apart, pose);
                    let swept_work = crate::work() - before;
                    let before = crate::work();
                    let direct = is_legal(&problem, &poses, index, pose, 0..parts);
                    let direct_work = crate::work() - before;
                    assert_eq!((swept, swept_work), (direct, direct_work), "part {index}");
                }
            }
        }
    }

    /// Random parts of every kind the legality test knows, at random
    /// poses on a small board (many near, many far), for the box tests.
    fn random_board(round: usize, next: &mut impl FnMut() -> f64) -> (Problem, Vec<Pose>) {
        let parts = 40;
        let (mut problem, _) = pair(0.0, 0.0);
        problem.outline = vec![[0.0, 0.0], [30.0, 0.0], [30.0, 30.0], [0.0, 30.0]];
        problem.constraints.pad_clearance = round % 2 == 1;
        problem.constraints.courtyard_spacing = round % 3 == 1;
        problem.shared_halos = round % 4 == 2;
        problem.far_side_pads_only = round % 5 == 3;
        problem.constraints.pad_mask_gap = if round % 3 == 2 { 0.3 } else { 0.0 };
        problem.edge_margin = [0.5, 0.2, 1.0][round % 3];
        problem.components = (0..parts)
            .map(|number| {
                let mut part = resistor(0.0);
                part.halo = if next() < 0.5 { 0.2 } else { 0.0 };
                part.angle_options = vec![0.0, 90.0, 180.0, 270.0, 45.0];
                part.side = match number % 4 {
                    0 => Side::Back,
                    1 => Side::Both,
                    _ => Side::Front,
                };
                part.fixed = next() < 0.2;
                if next() < 0.15 {
                    part.round = true;
                    part.body_size = [1.4, 1.4];
                }
                if next() < 0.3 {
                    part.far_side = vec![[-0.5, -0.5, 0.5, 0.5]];
                }
                if next() < 0.2 {
                    part.far_copper = vec![[-3.0, -0.2, -2.0, 0.2]];
                }
                if next() < 0.2 {
                    part.hollow = vec![[-1.0, -0.5, -0.6, 0.5], [0.6, -0.5, 1.0, 0.5]];
                    part.courtyards = vec![[-1.0, -0.5, 1.0, 0.5]];
                }
                if next() < 0.2 {
                    part.hole_rings = vec![[1.5, 1.5, 2.5, 2.5]];
                }
                if next() < 0.2 {
                    part.far_courtyard = vec![[-1.5, -1.0, 1.5, 1.0]];
                }
                if next() < 0.1 {
                    part.pads.push([2.0, -0.3, 3.5, 0.3]);
                }
                if next() < 0.08 {
                    part.copper_only = true;
                    part.fixed = true;
                    if next() < 0.7 {
                        // A slot of many points (its outline on the board).
                        let center = [5.0 + next() * 20.0, 5.0 + next() * 20.0];
                        let points = 32 + (next() * 60.0) as usize;
                        part.cutout_outline = (0..points)
                            .map(|index| {
                                let angle = index as f64 / points as f64 * std::f64::consts::TAU;
                                let radius = 2.0 + 3.0 * next();
                                [
                                    center[0] + radius * angle.cos(),
                                    center[1] + 0.6 * radius * angle.sin(),
                                ]
                            })
                            .collect();
                    }
                }
                part
            })
            .collect();
        if round % 3 != 1 {
            // Marks some parts may lie on, obstacles without routing room.
            let yielding: Vec<usize> = (0..parts).filter(|_| next() < 0.15).collect();
            problem.constraints.halo_free =
                yielding.iter().copied().filter(|_| next() < 0.5).collect();
            problem.constraints.yielding = yielding;
            problem.constraints.on_marks = (0..parts).filter(|_| next() < 0.2).collect();
        }
        if round % 4 == 1 {
            // Linked pairs (they meet without their halos; the lookup's work).
            for _ in 0..12 {
                let (a, b) = (
                    (next() * parts as f64) as usize,
                    (next() * parts as f64) as usize,
                );
                if a != b {
                    problem.constraints.linked.insert((a.min(b), a.max(b)));
                }
            }
        }
        if round.is_multiple_of(2) {
            problem.constraints.solid_cutouts = (0..parts)
                .filter(|index| problem.components[*index].copper_only)
                .collect();
        }
        let poses: Vec<Pose> = (0..parts)
            .map(|_| Pose {
                position: [next() * 30.0, next() * 30.0],
                angle: [0.0, 90.0, 180.0, 270.0, 45.0][(next() * 4.4) as usize],
            })
            .collect();
        problem.poses = poses.clone();
        (problem, poses)
    }

    #[test]
    fn parts_apart_by_their_boxes_answer_and_book_as_the_direct_test() {
        let mut state = 0x7A3C_91E5_0D4B_62F1u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut skipped = 0;
        for round in 0..12 {
            let (problem, mut poses) = random_board(round, &mut next);
            let parts = problem.components.len();
            let mut apart = Apart::new(&problem);
            apart.check = false;
            for _ in 0..parts * 30 {
                // Now and then another part moves (its box kept by pose).
                if next() < 0.1 {
                    let moved = (next() * parts as f64) as usize;
                    poses[moved].position = [next() * 30.0, next() * 30.0];
                }
                let index = (next() * parts as f64) as usize;
                let angle = [0.0, 90.0, 180.0, 270.0, 45.0][(next() * 4.4) as usize];
                let pose = Pose {
                    position: [next() * 30.0, next() * 30.0],
                    angle,
                };
                let others: Vec<usize> = (0..parts).filter(|_| next() < 0.6).collect();
                let before = crate::work();
                let filtered =
                    apart.is_legal(&problem, &poses, index, pose, others.iter().copied());
                let filtered_work = crate::work() - before;
                let before = crate::work();
                let direct = is_legal(&problem, &poses, index, pose, others.iter().copied());
                let direct_work = crate::work() - before;
                assert_eq!(
                    (filtered, filtered_work),
                    (direct, direct_work),
                    "round {round}, part {index} at {pose:?}"
                );
                // How many tests the boxes spared (that the test sees some).
                let body = rect(&problem, index, pose);
                if let Some(mine) = apart.board_box(&problem, index, pose, &body) {
                    skipped += others
                        .iter()
                        .filter(|&&other| {
                            other != index
                                && apart
                                    .other_box(&problem, &poses, other)
                                    .is_some_and(|theirs| boxes_apart(mine, theirs, apart.gap))
                        })
                        .count();
                }
            }
        }
        assert!(skipped > 10_000, "{skipped} tests spared");
    }

    /// `rect_meets_polygon` before the outlines' bands (2026-10-28).
    fn rect_meets_polygon_before(rect: &Rect, polygon: &[[f64; 2]], work: &mut u64) -> bool {
        *work += edge_work(polygon.len());
        let corners = rect.corners();
        let first = corners[0];
        let mut inside = false;
        let mut near = false;
        for index in 0..polygon.len() {
            let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
            let extent = [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ];
            if !box_apart(rect, extent, POLYGON_SLACK) {
                near = true;
                break;
            }
            if (a[1] > first[1]) != (b[1] > first[1]) {
                let x = a[0] + (first[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if first[0] < x {
                    inside = !inside;
                }
            }
        }
        if !near {
            return inside;
        }
        *work += 9 * edge_work(polygon.len());
        if corners
            .iter()
            .any(|corner| crate::problem::point_in_polygon(*corner, polygon))
        {
            return true;
        }
        let (axes, own) = rect.frame();
        if polygon.iter().any(|p| {
            let delta = [p[0] - rect.center[0], p[1] - rect.center[1]];
            (delta[0] * axes[0][0] + delta[1] * axes[0][1]).abs() <= own[0]
                && (delta[0] * axes[1][0] + delta[1] * axes[1][1]).abs() <= own[1]
        }) {
            return true;
        }
        let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
        };
        let segments_cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]| {
            let (d1, d2) = (cross(c, d, a), cross(c, d, b));
            let (d3, d4) = (cross(a, b, c), cross(a, b, d));
            (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
        };
        (0..polygon.len()).any(|index| {
            let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
            (0..4).any(|edge| segments_cross(a, b, corners[edge], corners[(edge + 1) % 4]))
        })
    }

    #[test]
    fn an_outline_by_bands_answers_and_books_as_the_pass_over_every_edge() {
        let mut state = 0x51F1_5E7A_C0DE_1234u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut nears = 0;
        let mut near_meets = 0;
        let mut insides = 0;
        for round in 0..30 {
            let points = [40, 64, 333, 1200][round % 4];
            // A ragged ring around (20, 20), some stretches flat and on a
            // lattice (edges along the bands' borders, repeated y values).
            let polygon: Vec<[f64; 2]> = (0..points)
                .map(|index| {
                    let angle = index as f64 / points as f64 * std::f64::consts::TAU;
                    let radius = 8.0 + 6.0 * next();
                    let point = [20.0 + radius * angle.cos(), 20.0 + radius * angle.sin()];
                    if round % 3 == 0 {
                        [
                            (point[0] * 4.0).round() / 4.0,
                            (point[1] * 2.0).round() / 2.0,
                        ]
                    } else {
                        point
                    }
                })
                .collect();
            let outline = Outline::new(&polygon);
            for _ in 0..2000 {
                let angle = [0.0, 90.0, 30.0, 45.0][(next() * 4.0) as usize];
                let pose = Pose {
                    position: [next() * 40.0, next() * 40.0],
                    angle,
                };
                let local = [[-0.3 - next(), -0.2 - next(), 0.3 + next(), 0.2 + next()]];
                let rect = turned(&local, pose, 0.0).next().unwrap();
                let mut indexed_work = 0;
                let indexed =
                    rect_meets_outline(&rect, &polygon, Some(&outline), &mut indexed_work);
                let mut plain_work = 0;
                let plain = rect_meets_outline(&rect, &polygon, None, &mut plain_work);
                let mut before_work = 0;
                let before = rect_meets_polygon_before(&rect, &polygon, &mut before_work);
                assert_eq!(
                    (indexed, indexed_work),
                    (before, before_work),
                    "round {round}, {rect:?}"
                );
                assert_eq!(
                    (plain, plain_work),
                    (before, before_work),
                    "round {round}, {rect:?}"
                );
                if before_work > edge_work(polygon.len()) {
                    nears += 1;
                    if before {
                        near_meets += 1;
                    }
                } else if before {
                    insides += 1;
                }
            }
        }
        assert!(
            nears > 1000 && near_meets > 500 && nears - near_meets > 100 && insides > 1000,
            "{nears} near ({near_meets} meeting), {insides} inside"
        );
    }

    #[test]
    fn a_sweep_with_boxes_answers_and_books_as_the_direct_test() {
        let mut state = 0x3C6E_F372_FE94_F82Bu64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for round in 0..12 {
            let (problem, poses) = random_board(round, &mut next);
            let parts = problem.components.len();
            let reach = Reach::new(&problem);
            let mut apart = Apart::new(&problem);
            apart.check = false;
            for index in 0..parts {
                if problem.components[index].copper_only {
                    continue;
                }
                let corner = [next() * 26.0, next() * 26.0];
                let area = [corner[0], corner[1], corner[0] + 4.0, corner[1] + 4.0];
                let mut sweep = Sweep::new(&problem, &poses, &reach, index, area);
                sweep.check = false;
                for _ in 0..40 {
                    let angle = [0.0, 90.0, 180.0, 45.0][(next() * 4.0) as usize];
                    let center = [
                        area[0] + next() * (area[2] - area[0]),
                        area[1] + next() * (area[3] - area[1]),
                    ];
                    let component = &problem.components[index];
                    let pose = Pose {
                        position: component.position_for_center(center, angle),
                        angle,
                    };
                    let before = crate::work();
                    let swept = sweep.is_legal(&problem, &poses, &mut apart, pose);
                    let swept_work = crate::work() - before;
                    let before = crate::work();
                    let direct = is_legal(&problem, &poses, index, pose, 0..parts);
                    let direct_work = crate::work() - before;
                    assert_eq!(
                        (swept, swept_work),
                        (direct, direct_work),
                        "round {round}, part {index}"
                    );
                }
            }
        }
    }

    /// `inside_outline` before the outline's bands and the orientations'
    /// shortcut.
    fn inside_outline_before(problem: &Problem, body: Rect) -> bool {
        let corners = body.corners();
        if !corners
            .iter()
            .all(|corner| point_in_polygon(*corner, &problem.outline))
        {
            return false;
        }
        let outline = &problem.outline;
        for index in 0..outline.len() {
            let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
            for side in 0..4 {
                if segments_cross(a, b, corners[side], corners[(side + 1) % 4]) {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn the_outline_test_by_bands_answers_as_the_pass_over_every_edge() {
        let mut state = 0x0BAD_5EED_1234_ABCDu64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let (mut problem, _) = pair(0.0, 0.0);
        let (mut insides, mut outsides) = (0, 0);
        for round in 0..24 {
            let points = [12, 13, 40, 150][round % 4];
            // A rounded rectangle, a ragged ring, or one on a lattice
            // (edges along the bands' borders, corners on its edges).
            problem.outline = (0..points)
                .map(|index| {
                    let angle = index as f64 / points as f64 * std::f64::consts::TAU;
                    let radius = match round % 3 {
                        0 => 10.0 / angle.cos().abs().max(angle.sin().abs()).max(0.8),
                        _ => 7.0 + 5.0 * next(),
                    };
                    let point = [20.0 + radius * angle.cos(), 20.0 + radius * angle.sin()];
                    if round % 3 == 2 {
                        [
                            (point[0] * 2.0).round() / 2.0,
                            (point[1] * 2.0).round() / 2.0,
                        ]
                    } else {
                        point
                    }
                })
                .collect();
            let board = Outline::new(&problem.outline);
            for _ in 0..3000 {
                let pose = Pose {
                    position: [(next() * 80.0).round() / 2.0, (next() * 80.0).round() / 2.0],
                    angle: [0.0, 90.0, 30.0, 45.0][(next() * 4.0) as usize],
                };
                let local = [[-0.5 - 2.0 * next(), -0.5, 0.5, 0.5 + 2.0 * next()]];
                let body = turned(&local, pose, 0.0).next().unwrap();
                let before = inside_outline_before(&problem, body);
                assert_eq!(
                    inside_outline(&problem, body, Some(&board)),
                    before,
                    "{body:?}"
                );
                assert_eq!(inside_outline(&problem, body, None), before, "{body:?}");
                if before {
                    insides += 1;
                } else {
                    outsides += 1;
                }
            }
        }
        assert!(
            insides > 5_000 && outsides > 5_000,
            "{insides} in, {outsides} out"
        );
    }

    #[test]
    fn the_facts_flags_answer_as_the_constraints_lists() {
        let mut state = 0x1234_5678_9ABC_DEF1u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for round in 0..8 {
            let (mut problem, _) = random_board(round, &mut next);
            let parts = problem.components.len();
            if round == 5 {
                // A list out of order: the facts take the lists' answers.
                problem.constraints.yielding = vec![7, 3, 30, 12];
                problem.constraints.on_marks = vec![3, 12];
            }
            let facts = Facts::new(&problem);
            assert_eq!(facts.flags.is_none(), round == 5);
            for a in 0..parts {
                for b in 0..parts {
                    assert_eq!(
                        facts.lies_on(&problem, a, b),
                        problem.constraints.lies_on(a, b)
                    );
                    assert_eq!(facts.bare(&problem, a, b), problem.constraints.bare(a, b));
                }
            }
        }
    }

    #[test]
    fn ring_steps_are_the_squares_scan_on_the_ring() {
        for ring in 0..9i64 {
            for low_x in -10..=3 {
                for low_y in [-10, -ring, -2, 0, 4] {
                    for high_x in [-10, -3, 0, 1, ring, 10] {
                        for high_y in [-1, 0, 2, ring, 10] {
                            let window = [low_x, low_y, high_x, high_y];
                            let mut scan = Vec::new();
                            for dy in (-ring).max(low_y)..=ring.min(high_y) {
                                for dx in (-ring).max(low_x)..=ring.min(high_x) {
                                    if dx.abs().max(dy.abs()) == ring {
                                        scan.push((dx, dy));
                                    }
                                }
                            }
                            let steps: Vec<(i64, i64)> = ring_steps(ring, window).collect();
                            assert_eq!(steps, scan, "ring {ring} window {window:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn part_relations_give_the_penalty_of_the_scan() {
        use crate::constraints::{Anchor, Edge, PartRelations, Relation};
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let parts = 30;
        let (mut problem, _) = pair(0.0, 0.0);
        problem.components = (0..parts).map(|_| resistor(0.0)).collect();
        let pick = |next: &mut dyn FnMut() -> f64| (next() * parts as f64) as usize;
        for _ in 0..40 {
            let part = pick(&mut next);
            let kind = next();
            let relation = if kind < 0.3 {
                Relation::Near {
                    part,
                    anchor: Anchor::Point(pick(&mut next), [0.5, 0.0]),
                    max: next() * 3.0,
                }
            } else if kind < 0.6 {
                Relation::NearAny {
                    part,
                    anchors: (0..1 + (next() * 23.0) as usize)
                        .map(|_| Anchor::Body(pick(&mut next)))
                        .collect(),
                    max: next() * 3.0,
                }
            } else if kind < 0.8 {
                Relation::Beside {
                    part,
                    anchor: pick(&mut next),
                    side: Edge::Left,
                    max_gap: next() * 2.0,
                }
            } else {
                Relation::Apart {
                    part,
                    anchor: pick(&mut next),
                    min: next() * 5.0,
                }
            };
            problem.constraints.relations.push(relation);
        }
        problem.constraints.relation_weight = 1.7;
        let relations = PartRelations::new(&problem);
        for _ in 0..20 {
            let poses: Vec<Pose> = (0..parts)
                .map(|_| Pose {
                    position: [next() * 30.0, next() * 30.0],
                    angle: [0.0, 90.0, 180.0, 270.0][(next() * 4.0) as usize],
                })
                .collect();
            for index in 0..parts {
                let scan = constraints::relation_penalty(&problem, &poses, Some(index));
                let listed = relations.penalty(&problem, &poses, index);
                assert_eq!(scan.to_bits(), listed.to_bits(), "{scan} {listed}");
                assert_eq!(
                    relations.work(index),
                    constraints::relation_work(&problem, index)
                );
            }
        }
    }

    #[test]
    fn moving_nets_cost_the_direct_sum_bit_for_bit() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for round in 0..20 {
            let parts = 8 + round;
            let nets = 3 + round / 2;
            let mut components = Vec::new();
            let mut poses = Vec::new();
            for _ in 0..parts {
                let mut component = resistor(0.0);
                component.angle_options = vec![0.0, 90.0, 180.0, 270.0];
                for _ in 0..1 + (next() * 6.0) as usize {
                    // Some pins at whole millimetres, some on an axis of
                    // the frame: coordinates of either zero occur.
                    let coordinate = |value: f64| {
                        if value < 0.2 {
                            0.0
                        } else {
                            (value * 8.0).round() / 4.0 - 1.0
                        }
                    };
                    component.pins.push(crate::problem::Pin {
                        offset: [coordinate(next()), coordinate(next())],
                        net: (next() * nets as f64) as usize,
                    });
                }
                components.push(component);
                poses.push(Pose {
                    position: [
                        if next() < 0.1 { 0.0 } else { next() * 40.0 },
                        if next() < 0.1 { -0.0 } else { next() * 40.0 },
                    ],
                    angle: [0.0, 90.0, 180.0, 270.0][(next() * 4.0) as usize],
                });
            }
            let (mut problem, _) = pair(0.0, 0.0);
            problem.components = components;
            problem.poses = poses.clone();
            problem.net_weights = (0..nets).map(|_| 0.5 + next()).collect();
            let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nets];
            for (index, component) in problem.components.iter().enumerate() {
                for (pin, description) in component.pins.iter().enumerate() {
                    members[description.net].push((index, pin));
                }
            }
            for part in 0..parts {
                let mut incident: Vec<usize> = problem.components[part]
                    .pins
                    .iter()
                    .map(|pin| pin.net)
                    .collect();
                incident.sort_unstable();
                incident.dedup();
                let moving = MovingNets::new(&problem, &members, &incident, part, &poses);
                let mut candidate = poses.clone();
                for _ in 0..50 {
                    candidate[part] = Pose {
                        position: [
                            if next() < 0.1 {
                                0.0
                            } else {
                                next() * 50.0 - 5.0
                            },
                            next() * 50.0 - 5.0,
                        ],
                        angle: [0.0, 90.0, 180.0, 270.0][(next() * 4.0) as usize],
                    };
                    let before = crate::work();
                    let fast = moving.cost(&problem, &members, &candidate);
                    let booked = crate::work() - before;
                    let direct: f64 = incident
                        .iter()
                        .map(|net| net_cost(&problem, &members, *net, &candidate))
                        .sum();
                    assert_eq!(fast.to_bits(), direct.to_bits(), "{fast} {direct}");
                    let pins: usize = incident.iter().map(|net| members[*net].len()).sum();
                    assert_eq!(booked, crate::PIN_WORK * pins as u64);
                }
            }
        }
    }

    #[test]
    fn kicad_legality_lets_courtyards_touch_where_the_pads_keep_their_clearance() {
        // Pads 0.25 mm inside the courtyards' facing ends: touching
        // courtyards leave the pads 0.5 mm apart, which KiCad accepts.
        let (mut problem, poses) = pair(0.25, 0.0);
        assert!(!is_legal(&problem, &poses, 1, poses[1], 0..2));
        problem.constraints.pad_clearance = true;
        problem.constraints.courtyard_spacing = true;
        assert!(is_legal(&problem, &poses, 1, poses[1], 0..2));
        // Overlapping courtyards stay illegal.
        let (mut problem, poses) = pair(0.25, -0.05);
        problem.constraints.pad_clearance = true;
        assert!(!is_legal(&problem, &poses, 1, poses[1], 0..2));
    }

    #[test]
    fn kicad_legality_keeps_the_pads_the_clearance_apart() {
        // Pads at the courtyards' ends: 0.1 mm between the courtyards is
        // 0.1 mm between the pads, less than the 0.2 mm clearance.
        let (mut problem, poses) = pair(0.0, 0.1);
        problem.constraints.pad_clearance = true;
        problem.constraints.courtyard_spacing = true;
        assert!(!is_legal(&problem, &poses, 1, poses[1], 0..2));
        let (mut problem, poses) = pair(0.0, 0.2);
        problem.constraints.pad_clearance = true;
        problem.constraints.courtyard_spacing = true;
        assert!(is_legal(&problem, &poses, 1, poses[1], 0..2));
    }

    #[test]
    fn pads_of_one_net_may_come_closer_than_the_clearance() {
        // Pads at the courtyards' ends, 0.1 mm apart: of two nets too
        // close, of one net legal (mating connectors of two cells).
        let (mut problem, poses) = pair(0.0, 0.1);
        problem.constraints.pad_clearance = true;
        problem.constraints.courtyard_spacing = true;
        let pin = |x: f64, net: usize| crate::problem::Pin {
            offset: [x, 0.0],
            net,
        };
        problem.components[0].pins = vec![pin(-0.7, 0), pin(0.7, 1)];
        problem.components[1].pins = vec![pin(-0.7, 2), pin(0.7, 3)];
        assert!(!is_legal(&problem, &poses, 1, poses[1], 0..2));
        problem.components[1].pins = vec![pin(-0.7, 1), pin(0.7, 3)];
        assert!(is_legal(&problem, &poses, 1, poses[1], 0..2));
    }

    #[test]
    fn at_kicads_legality_a_lead_keeps_the_copper_clearance_from_the_far_sides_pads() {
        // A lead through the board (1 mm square) under the far part's
        // courtyard, 0.3 mm from its pads: off the body elsewhere, legal at
        // KiCad's legality where holes in courtyards are no error.
        let (mut problem, mut poses) = pair(0.4, 0.0);
        problem.components[1].side = Side::Back;
        problem.components[0].far_side = vec![[1.7, -0.5, 2.7, 0.5]];
        poses[1].position = [13.6, 10.0];
        problem.poses = poses.clone();
        // The lead (x 11.7 to 12.7) lies under the back part's body (12.6 to
        // 14.6), 0.3 mm from its first pad (13.0 to 13.6).
        assert!(!is_legal(&problem, &poses, 1, poses[1], 0..2));
        problem.constraints.pad_clearance = true;
        assert!(!is_legal(&problem, &poses, 1, poses[1], 0..2));
        problem.constraints.leads_keep_copper = true;
        assert!(is_legal(&problem, &poses, 1, poses[1], 0..2));
        // Within the copper clearance of a pad: not legal.
        poses[1].position = [13.4, 10.0];
        assert!(!is_legal(&problem, &poses, 1, poses[1], 0..2));
    }

    #[test]
    fn the_direction_is_the_one_before_the_shortcut() {
        let before = |angle: f64| {
            let quarter = angle.rem_euclid(90.0);
            if quarter < 1.0e-6 || 90.0 - quarter < 1.0e-6 {
                return None;
            }
            let (sin, cos) = (-angle).to_radians().sin_cos();
            Some([cos.to_bits(), sin.to_bits()])
        };
        for step in -1440..=1440 {
            let angle = step as f64 * 0.25;
            for angle in [angle, -angle, angle + 1.0e-7] {
                let now = direction_of(angle).map(|[cos, sin]| [cos.to_bits(), sin.to_bits()]);
                assert_eq!(now, before(angle), "at {angle}");
            }
        }
    }
}

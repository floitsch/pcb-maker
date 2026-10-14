// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Simulated-annealing legalization.
//!
//! Global placement leaves small overlaps. Resolving them greedily part by
//! part exiles whatever comes last, so instead all parts negotiate: moves,
//! quarter turns and swaps are judged on wirelength plus an overlap penalty
//! that grows until no overlap is worth keeping. It starts cold, because the
//! global placement is already good and must not be scrambled.

use crate::constraints;
use crate::problem::{Point, Pose, Problem, point_in_polygon};

#[derive(Clone, Debug)]
pub struct AnnealConfig {
    pub stages: usize,
    pub moves_per_part: usize,
    /// Initial move radius as a fraction of the board's larger side.
    pub initial_radius: f64,
    pub seed: u64,
    /// No further stage starts after this wall-clock time.
    pub deadline: Option<std::time::Instant>,
    /// No further stage starts once this thread's `crate::work()` reaches
    /// this count.
    pub stop_at_work: Option<u64>,
    /// Parts across the board meet at the legality test's gaps
    /// (`State::far_overlap`): holes the full spacing from bodies, far-side
    /// copper the copper clearance from pads. Without it holes meet bodies
    /// at half the spacing and far-side copper not at all, and the
    /// legalizer moves what the annealer could not see (Castor: 33 of 69
    /// conflicted parts). Off: flat on the quick tier, worse on video
    /// and OpenESC 4in1 (2026-10-14).
    pub legal_far_side: bool,
}

impl Default for AnnealConfig {
    fn default() -> Self {
        Self {
            stages: 160,
            moves_per_part: 40,
            initial_radius: 0.06,
            seed: 7,
            deadline: None,
            stop_at_work: None,
            legal_far_side: false,
        }
    }
}

struct Random(u64);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, bound: usize) -> usize {
        ((self.next() * bound as f64) as usize).min(bound - 1)
    }
}

#[derive(Clone, Copy)]
struct Rect {
    center: Point,
    half: Point,
}

struct State<'a> {
    problem: &'a Problem,
    poses: Vec<Pose>,
    rects: Vec<Rect>,
    members: Vec<Vec<(usize, usize)>>,
    incident: Vec<Vec<usize>>,
    bounds: [f64; 4],
    rectangular: bool,
    /// Work beyond the per-move base: parts and cells visited, box lists
    /// built and tested.
    work: std::cell::Cell<u64>,
    /// The rectangles by area: the overlap sum visits the parts near one.
    grid: std::cell::RefCell<crate::buckets::Grid>,
    near: std::cell::RefCell<Vec<usize>>,
    /// How far beyond its rectangle a part's overlap reaches (a cutout's
    /// edge margin around pads).
    reach: f64,
    /// Work per part visited (the linked-pair lookup).
    visit_work: u64,
    /// `PCB_PLACER_CHECK_OVERLAP`: each bucketed overlap sum is checked
    /// against the sum over all parts.
    check_overlap: bool,
    /// `AnnealConfig::legal_far_side`.
    legal_far_side: bool,
}

impl State<'_> {
    /// Moves part `index`'s rectangle, in the grid too.
    fn set_rect(&mut self, index: usize, rect: Rect) {
        let old = self.rects[index];
        let grid = self.grid.get_mut();
        grid.remove(index, old.center, old.half);
        grid.insert(index, rect.center, rect.half);
        self.rects[index] = rect;
    }

    /// Half the spacing, or the body's own share of it at the
    /// courtyard-spacing level.
    fn share(&self, component: &crate::problem::Component) -> f64 {
        if self.problem.constraints.courtyard_spacing {
            component.copper_share(self.problem.min_spacing)
        } else {
            self.problem.spacing / 2.0
        }
    }

    /// The body inflated by its halo and half the spacing: two such
    /// rectangles overlap exactly when the parts are too close.
    fn rect(&self, index: usize, pose: Pose) -> Rect {
        let component = &self.problem.components[index];
        let half = component.half_extent(pose.angle);
        let margin = component.halo + self.share(component);
        Rect {
            center: component.center(pose),
            half: [half[0] + margin, half[1] + margin],
        }
    }

    fn overlap(&self, index: usize, rect: Rect) -> f64 {
        let fast = self.overlap_with(index, rect, false);
        if self.check_overlap {
            let full = self.overlap_with(index, rect, true);
            assert!((fast - full).abs() <= 1.0e-9 * full.abs().max(1.0), "part {index}: bucketed {fast}, full {full}");
        }
        fast
    }

    fn overlap_with(&self, index: usize, rect: Rect, everything: bool) -> f64 {
        let side = self.problem.components[index].side;
        let mut total = 0.0;
        // A box list costs its allocation and transforms.
        let built = |boxes: Vec<(Point, Point)>| {
            self.work.set(self.work.get() + 8 + 2 * boxes.len() as u64);
            boxes
        };
        let area = |a: (Point, Point), b: (Point, Point)| {
            self.work.set(self.work.get() + 1);
            let width = a.1[0] + b.1[0] - (a.0[0] - b.0[0]).abs();
            let height = a.1[1] + b.1[1] - (a.0[1] - b.0[1]).abs();
            if width > 0.0 && height > 0.0 { width * height } else { 0.0 }
        };
        let component = &self.problem.components[index];
        let pose = self.poses[index];
        let own_far = if component.far_side.is_empty() { Vec::new() } else { built(component.far_boxes(pose)) };
        let mut near = self.near.borrow_mut();
        let cells = self.grid.borrow_mut().query(rect.center, [rect.half[0] + self.reach, rect.half[1] + self.reach], &mut near);
        if everything {
            near.clear();
            near.extend(0..self.rects.len());
        }
        self.work.set(self.work.get() + cells as u64 + near.len() as u64 * self.visit_work);
        for &other in near.iter() {
            if other == index {
                continue;
            }
            let body = &self.rects[other];
            let other_component = &self.problem.components[other];
            if side.collides(other_component.side) {
                if component.copper_only || other_component.copper_only {
                    // A cutout keeps only copper away.
                    let margin = self.problem.edge_margin;
                    let (hole, part, part_pose) = if component.copper_only {
                        ((rect.center, rect.half), other_component, self.poses[other])
                    } else {
                        ((body.center, body.half), component, pose)
                    };
                    if !part.copper_only {
                        for (center, half) in built(part.pad_boxes(part_pose)) {
                            total += area((center, [half[0] + margin, half[1] + margin]), hole);
                        }
                    }
                } else if !component.hollow_for(other_component) && !other_component.hollow_for(component) {
                    if self.problem.constraints.linked(index, other) {
                        // Parts tied together meet without their halos.
                        let (mine, theirs) = (component.halo, other_component.halo);
                        total += area(
                            (rect.center, [rect.half[0] - mine, rect.half[1] - mine]),
                            (body.center, [body.half[0] - theirs, body.half[1] - theirs]),
                        );
                    } else if self.problem.shared_halos {
                        // The larger halo between them.
                        let (mine, theirs) = (component.halo, other_component.halo);
                        let grow = mine.max(theirs) - mine;
                        total += area(
                            (rect.center, [rect.half[0] + grow, rect.half[1] + grow]),
                            (body.center, [body.half[0] - theirs, body.half[1] - theirs]),
                        );
                    } else {
                        total += area((rect.center, rect.half), (body.center, body.half));
                    }
                    // Courtyards on the other side meet there too
                    // (`legal::far_courtyard_overlap`).
                    if !component.far_courtyard.is_empty() || !other_component.far_courtyard.is_empty() {
                        total += self.far_courtyards(index, pose, other, self.poses[other], &built, &area)
                            + self.far_courtyards(other, self.poses[other], index, pose, &built, &area);
                    }
                } else {
                    // A hollow part blocks with its boxes (grown by half the
                    // spacing, as the rectangles are) only.
                    let blocking = |part: &crate::problem::Component,
                                    against: &crate::problem::Component,
                                    pose: Pose,
                                    whole: &Rect|
                     -> Vec<(Point, Point)> {
                        if !part.hollow_for(against) {
                            return vec![(whole.center, whole.half)];
                        }
                        let margin = self.problem.spacing / 2.0;
                        built(part.blocking_boxes(against, pose))
                            .into_iter()
                            .map(|(center, half)| (center, [half[0] + margin, half[1] + margin]))
                            .collect()
                    };
                    let theirs = blocking(other_component, component, self.poses[other], body);
                    for mine in blocking(component, other_component, pose, &rect) {
                        for other in &theirs {
                            total += area(mine, *other);
                        }
                    }
                }
            } else if side.opposite(other_component.side) {
                // Across the board only holes and pads meet bodies, at the
                // gaps the legality test asks for (`legal::far_side_overlap`).
                if self.legal_far_side {
                    total += self.far_overlap(index, pose, &rect, other, self.poses[other], body, &built, &area)
                        + self.far_overlap(other, self.poses[other], body, index, pose, &rect, &built, &area);
                } else {
                    for far in &own_far {
                        total += area(*far, (body.center, body.half));
                    }
                    if !other_component.far_side.is_empty() {
                        for far in built(other_component.far_boxes(self.poses[other])) {
                            total += area(far, (rect.center, rect.half));
                        }
                    }
                }
            }
        }
        // Leaving the board is as bad as overlapping something (for an
        // overhanging part, only with what belongs on the board).
        let component = &self.problem.components[index];
        let margin = component.halo + self.share(component) - component.edge_margin(self.problem.edge_margin);
        let mut bare = [rect.half[0] - margin, rect.half[1] - margin];
        let mut rect = rect;
        if !self.problem.constraints.is_empty() {
            // Constrained sides may touch the edge; an overhanging part
            // counts only its share on the board.
            let pose = self.poses[index];
            let inner = constraints::inner_box(self.problem, index, pose).unwrap_or_else(|| {
                let (center, half) = (component.center(pose), component.half_extent(pose.angle));
                [center[0] - half[0], center[1] - half[1], center[0] + half[0], center[1] + half[1]]
            });
            let margins = constraints::side_margins(self.problem, index, pose.angle);
            let grown = [
                inner[0] - margins[0],
                inner[1] - margins[1],
                inner[2] + margins[2],
                inner[3] + margins[3],
            ];
            rect.center = [(grown[0] + grown[2]) / 2.0, (grown[1] + grown[3]) / 2.0];
            bare = [
                ((grown[2] - grown[0]) / 2.0 - 1.0e-6).max(0.0),
                ((grown[3] - grown[1]) / 2.0 - 1.0e-6).max(0.0),
            ];
        }
        let outside_x = (self.bounds[0] - (rect.center[0] - bare[0])).max(0.0)
            + ((rect.center[0] + bare[0]) - self.bounds[2]).max(0.0);
        let outside_y = (self.bounds[1] - (rect.center[1] - bare[1])).max(0.0)
            + ((rect.center[1] + bare[1]) - self.bounds[3]).max(0.0);
        total += 2.0 * (outside_x * 2.0 * bare[1] + outside_y * 2.0 * bare[0]);
        if !self.rectangular {
            for corner in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
                let point = [
                    rect.center[0] + corner[0] * bare[0],
                    rect.center[1] + corner[1] * bare[1],
                ];
                if !point_in_polygon(point, &self.problem.outline) {
                    total += 4.0 * bare[0] * bare[1];
                }
            }
        }
        total
    }

    /// The overlap of part `holes`' far-side boxes with part `body` across
    /// the board, grown to the gaps `legal::far_side_overlap` asks for: its
    /// holes and pads the spacing from the body (with its halo), its
    /// copper that only copper may not come near the copper clearance from
    /// the body's pads.
    #[allow(clippy::too_many_arguments)]
    fn far_overlap(
        &self,
        holes: usize,
        holes_pose: Pose,
        _holes_rect: &Rect,
        body: usize,
        body_pose: Pose,
        body_rect: &Rect,
        built: &dyn Fn(Vec<(Point, Point)>) -> Vec<(Point, Point)>,
        area: &dyn Fn((Point, Point), (Point, Point)) -> f64,
    ) -> f64 {
        let component = &self.problem.components[holes];
        if component.far_side.is_empty() && component.far_copper.is_empty() {
            return 0.0;
        }
        let other = &self.problem.components[body];
        let grow = |boxes: Vec<(Point, Point)>, margin: f64| -> Vec<(Point, Point)> {
            boxes.into_iter().map(|(center, half)| (center, [half[0] + margin, half[1] + margin])).collect()
        };
        // The body's copper: its pads, or all of it where it has none.
        let share = self.share(other);
        let pads = || {
            if other.pads.is_empty() {
                vec![(body_rect.center, [body_rect.half[0] - share, body_rect.half[1] - share])]
            } else {
                built(other.pad_boxes(body_pose))
            }
        };
        let mut total = 0.0;
        let min_spacing = self.problem.min_spacing;
        if self.problem.far_side_pads_only {
            let pads = pads();
            let mut boxes = built(component.far_boxes(holes_pose));
            boxes.extend(built(component.boxes_of(&component.far_copper, holes_pose)));
            for far in grow(boxes, min_spacing) {
                for pad in &pads {
                    total += area(far, *pad);
                }
            }
            return total;
        }
        if !component.far_side.is_empty() {
            let margin = (self.problem.spacing - share).max(0.0);
            for far in grow(built(component.far_boxes(holes_pose)), margin) {
                total += area(far, (body_rect.center, body_rect.half));
            }
        }
        if !component.far_copper.is_empty() {
            let pads = pads();
            for far in grow(built(component.boxes_of(&component.far_copper, holes_pose)), min_spacing) {
                for pad in &pads {
                    total += area(far, *pad);
                }
            }
        }
        total
    }

    /// The overlap of part `yard`'s courtyard on the other side with part
    /// `other`'s courtyard, holes and pads there, for two parts on the
    /// same side, each grown by its share of the spacing.
    fn far_courtyards(
        &self,
        yard: usize,
        yard_pose: Pose,
        other: usize,
        other_pose: Pose,
        built: &dyn Fn(Vec<(Point, Point)>) -> Vec<(Point, Point)>,
        area: &dyn Fn((Point, Point), (Point, Point)) -> f64,
    ) -> f64 {
        let (component, against) = (&self.problem.components[yard], &self.problem.components[other]);
        if component.far_courtyard.is_empty() || against.far_side.is_empty() {
            return 0.0;
        }
        let (mine, theirs) = (self.share(component), self.share(against));
        let yards = built(component.boxes_of(&component.far_courtyard, yard_pose));
        let blocking = built(against.far_boxes(other_pose));
        let mut total = 0.0;
        for (center, half) in &yards {
            for (their_center, their_half) in &blocking {
                total += area(
                    (*center, [half[0] + mine, half[1] + mine]),
                    (*their_center, [their_half[0] + theirs, their_half[1] + theirs]),
                );
            }
        }
        total
    }

    /// Relations involving `index`, plus its hard constraints priced so
    /// steeply that no wirelength gain pays for missing them.
    fn constraint_cost(&self, index: usize) -> f64 {
        if self.problem.constraints.is_empty() {
            return 0.0;
        }
        constraints::relation_penalty(self.problem, &self.poses, Some(index))
            + 1000.0 * constraints::hard_violation(self.problem, index, self.poses[index])
    }

    fn wirelength(&self, nets: &[usize]) -> f64 {
        nets.iter()
            .map(|net| {
                let pins = &self.members[*net];
                if pins.len() < 2 {
                    return 0.0;
                }
                let mut bounds = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for (component, pin) in pins {
                    let description = &self.problem.components[*component];
                    let at = description.pin_position(&description.pins[*pin], self.poses[*component]);
                    bounds[0] = bounds[0].min(at[0]);
                    bounds[1] = bounds[1].min(at[1]);
                    bounds[2] = bounds[2].max(at[0]);
                    bounds[3] = bounds[3].max(at[1]);
                }
                self.problem.net_weights[*net] * ((bounds[2] - bounds[0]) + (bounds[3] - bounds[1]))
            })
            .sum()
    }
}


fn build_state<'a>(problem: &'a Problem, poses: &[Pose], legal_far_side: bool) -> State<'a> {
    let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); problem.net_weights.len()];
    for (index, component) in problem.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            members[description.net].push((index, pin));
        }
    }
    let incident = problem
        .components
        .iter()
        .map(|component| {
            let mut nets: Vec<usize> = component.pins.iter().map(|pin| pin.net).collect();
            nets.sort_unstable();
            nets.dedup();
            nets
        })
        .collect();
    let bounds = problem.bounds();
    let area: f64 = {
        let outline = &problem.outline;
        (0..outline.len())
            .map(|index| {
                let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            .abs()
            / 2.0
    };
    let rectangular =
        (area - (bounds[2] - bounds[0]) * (bounds[3] - bounds[1])).abs() < 1.0e-6 * area.max(1.0);
    let mut state = State {
        problem,
        poses: poses.to_vec(),
        rects: Vec::new(),
        members,
        incident,
        bounds,
        rectangular,
        work: std::cell::Cell::new(0),
        grid: std::cell::RefCell::new(crate::buckets::Grid::new(bounds, 2.5, problem.components.len())),
        near: std::cell::RefCell::new(Vec::new()),
        // The legality test's gaps across the board reach farther.
        reach: problem.edge_margin
            + 0.5
            + problem.far_reach()
            + problem.far_courtyard_reach()
            + if legal_far_side { problem.spacing + problem.min_spacing } else { 0.0 },
        visit_work: (1.0 + (problem.constraints.linked.len() as f64 + 1.0).log2() / 2.0).round() as u64,
        check_overlap: std::env::var_os("PCB_PLACER_CHECK_OVERLAP").is_some(),
        legal_far_side,
    };
    state.rects = (0..problem.components.len())
        .map(|index| state.rect(index, state.poses[index]))
        .collect();
    for (index, rect) in state.rects.iter().enumerate() {
        state.grid.get_mut().insert(index, rect.center, rect.half);
    }

    state
}

/// Debugging aid: the overlap the annealer sees for every part at `poses`.
pub fn overlaps(problem: &Problem, poses: &[Pose], config: &AnnealConfig) -> Vec<f64> {
    let state = build_state(problem, poses, config.legal_far_side);
    (0..problem.components.len()).map(|index| state.overlap(index, state.rects[index])).collect()
}

/// Improves `poses` in place and returns the remaining overlap area.
pub fn anneal(problem: &Problem, poses: &mut Vec<Pose>, config: &AnnealConfig) -> f64 {
    let movable: Vec<usize> = (0..problem.components.len())
        .filter(|index| !problem.components[*index].fixed)
        .collect();
    if movable.is_empty() {
        return 0.0;
    }
    for index in &movable {
        poses[*index].position =
            constraints::snap_position(problem, *index, poses[*index].position, poses[*index].angle);
    }
    let mut state = build_state(problem, poses, config.legal_far_side);
    // Work per move (before and after), besides the parts the overlap
    // test visits (counted there): when the outline is not a rectangle,
    // four corners against it (cheap per edge next to a part's visit);
    // the wirelength visits the part's pins' nets; the constraint cost
    // scans the relations. Fitted so that boards as different as katia,
    // link and OpenESC's do about the same work per second.
    // The hard constraints' test scans every edge and region entry for
    // the part's own (a panel's copies put a region on every part).
    let held = &problem.constraints;
    // Finding a part's relations scans every relation, and every anchor of
    // a decoupling capacitor's (glasgow's 82 relations, 23 anchors each).
    let scan: usize = held
        .relations
        .iter()
        .map(|relation| match relation {
            constraints::Relation::NearAny { anchors, .. } => 1 + anchors.len() / 16,
            _ => 1,
        })
        .sum();
    let fixed = scan
        + if state.rectangular { 0 } else { problem.outline.len() }
        + if held.is_empty() { 0 } else { (held.edges.len() + held.regions.len()) / 2 };
    // The relations a part is in are evaluated before and after its move.
    let move_work: Vec<u64> = (0..problem.components.len())
        .map(|index| {
            let pins: usize = state.incident[index].iter().map(|net| state.members[*net].len()).sum();
            2 * (fixed as u64 + crate::PIN_WORK * pins as u64) + 2 * constraints::relation_work(problem, index)
        })
        .collect();
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        let average = move_work.iter().sum::<u64>() as f64 / move_work.len().max(1) as f64;
        eprintln!(
            "anneal: {} parts ({} movable), outline {} ({}), relations {}, linked {}, far-side parts {}, hollow parts {}, move work {average:.0}",
            problem.components.len(),
            movable.len(),
            problem.outline.len(),
            if state.rectangular { "rectangular" } else { "shaped" },
            problem.constraints.relations.len(),
            problem.constraints.linked.len(),
            problem.components.iter().filter(|component| !component.far_side.is_empty()).count(),
            problem.components.iter().filter(|component| !component.hollow.is_empty()).count(),
        );
    }
    let mut random = Random(config.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let bounds = state.bounds;
    let board = (bounds[2] - bounds[0]).max(bounds[3] - bounds[1]);
    let grid = if problem.grid > 0.0 { problem.grid } else { 0.05 };
    let mut radius = (config.initial_radius * board).max(2.0 * grid);
    // Millimetres of wirelength one square millimetre of overlap is worth.
    let mut penalty = 0.5;
    let mut temperature = 0.3;
    let penalty_growth = (400.0f64 / penalty).powf(1.0 / config.stages as f64);
    let radius_decay = ((1.5 * grid) / radius).powf(1.0 / config.stages as f64);

    for _ in 0..config.stages {
        if config.deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline)
            || config.stop_at_work.is_some_and(|stop| crate::work() >= stop)
        {
            break;
        }
        let mut work = 0;
        for _ in 0..config.moves_per_part * movable.len() {
            let index = movable[random.below(movable.len())];
            work += move_work[index];
            let component = &problem.components[index];
            let kind = random.next();
            if kind < 0.12 && movable.len() > 1 {
                // Swap two parts, keeping each at the other's body centre.
                let other = movable[random.below(movable.len())];
                if other == index {
                    continue;
                }
                work += move_work[other];
                let second = &problem.components[other];
                let (first_pose, second_pose) = (state.poses[index], state.poses[other]);
                let first_new = Pose {
                    position: component
                        .position_for_center(second.center(second_pose), first_pose.angle),
                    angle: first_pose.angle,
                };
                let second_new = Pose {
                    position: second
                        .position_for_center(component.center(first_pose), second_pose.angle),
                    angle: second_pose.angle,
                };
                let snapped = |part: usize, pose: Pose| Pose {
                    position: constraints::snap_position(problem, part, pose.position, pose.angle),
                    angle: pose.angle,
                };
                let (first_new, second_new) = (snapped(index, first_new), snapped(other, second_new));
                let mut nets = state.incident[index].clone();
                nets.extend(&state.incident[other]);
                nets.sort_unstable();
                nets.dedup();
                let before = state.wirelength(&nets)
                    + state.constraint_cost(index)
                    + state.constraint_cost(other)
                    + penalty
                        * (state.overlap(index, state.rects[index])
                            + state.overlap(other, state.rects[other]));
                let saved = (state.rects[index], state.rects[other]);
                state.poses[index] = first_new;
                state.poses[other] = second_new;
                let (first_rect, second_rect) = (state.rect(index, first_new), state.rect(other, second_new));
                state.set_rect(index, first_rect);
                state.set_rect(other, second_rect);
                let after = state.wirelength(&nets)
                    + state.constraint_cost(index)
                    + state.constraint_cost(other)
                    + penalty
                        * (state.overlap(index, state.rects[index])
                            + state.overlap(other, state.rects[other]));
                let delta = after - before;
                if delta > 0.0 && random.next() >= (-delta / temperature).exp() {
                    state.poses[index] = first_pose;
                    state.poses[other] = second_pose;
                    state.set_rect(index, saved.0);
                    state.set_rect(other, saved.1);
                }
                continue;
            }
            let old = state.poses[index];
            let new = if kind < 0.27 && component.angle_options.len() > 1 {
                let angle = component.angle_options[random.below(component.angle_options.len())];
                if (angle - old.angle).abs() < 1.0e-9 {
                    continue;
                }
                let position = component.position_for_center(component.center(old), angle);
                Pose {
                    position: constraints::snap_position(problem, index, position, angle),
                    angle,
                }
            } else {
                let step = |random: &mut Random| {
                    let cells = (radius / grid).max(1.0);
                    ((random.next() * 2.0 - 1.0) * cells).round() * grid
                };
                let position = [
                    old.position[0] + step(&mut random),
                    old.position[1] + step(&mut random),
                ];
                if position == old.position {
                    continue;
                }
                Pose {
                    position,
                    angle: old.angle,
                }
            };
            let before = state.wirelength(&state.incident[index])
                + state.constraint_cost(index)
                + penalty * state.overlap(index, state.rects[index]);
            let saved = state.rects[index];
            state.poses[index] = new;
            let new_rect = state.rect(index, new);
            state.set_rect(index, new_rect);
            let after = state.wirelength(&state.incident[index])
                + state.constraint_cost(index)
                + penalty * state.overlap(index, state.rects[index]);
            let delta = after - before;
            if delta > 0.0 && random.next() >= (-delta / temperature).exp() {
                state.poses[index] = old;
                state.set_rect(index, saved);
            }
        }
        crate::add_work(work + state.work.replace(0));
        penalty *= penalty_growth;
        radius *= radius_decay;
        temperature *= 0.975;
    }
    *poses = state.poses.clone();
    movable
        .iter()
        .map(|index| state.overlap(*index, state.rects[*index]))
        .sum::<f64>()
        / 2.0
}

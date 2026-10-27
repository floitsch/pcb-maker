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

/// Boxes as (center, half size), as `Component::far_boxes` and its kin
/// return them.
type Boxes = Vec<(Point, Point)>;

#[derive(Clone, Debug)]
pub struct AnnealConfig {
    /// Stages of the annealing schedule: the overlap penalty grows and the
    /// move radius shrinks from stage to stage.
    pub stages: usize,
    /// Moves tried per movable part in each stage.
    pub moves_per_part: usize,
    /// Initial move radius as a fraction of the board's larger side.
    pub initial_radius: f64,
    /// Seed of the annealer's moves.
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
    /// The global route inside the anneal (`crate::guidance`, an
    /// experiment): off (`None`) by default.
    pub guidance: Option<crate::guidance::Guidance>,
    /// Bus-group moves (`crate::groups`, `Config::bus_groups`): after each
    /// stage, groups of parts move, turn, trade places and reorder as
    /// units, judged with the crossings of their nets. Off (`None`) by
    /// default.
    pub bus_groups: Option<std::sync::Arc<crate::groups::BusGroups>>,
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
            guidance: None,
            bus_groups: None,
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
    /// The last global route's overflow field (`AnnealConfig::guidance`).
    field: Option<crate::guidance::Prepared>,
    /// Per net, its term of `wirelength` at the poses and the field of
    /// now (`refresh_lengths`, `keep_lengths`): a move's wirelength before
    /// it is the sum of these, without turning every pin of its nets again
    /// (the before half of the anneal's wirelength, 3 to 7 % of a
    /// placement's instructions, 2026-10-28).
    lengths: Vec<f64>,
    /// Whether every cached sum is checked against the direct one (debug
    /// builds and `PCB_PLACER_CHECK_OVERLAP`).
    check_lengths: bool,
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
            assert!(
                (fast - full).abs() <= 1.0e-9 * full.abs().max(1.0),
                "part {index}: bucketed {fast}, full {full}"
            );
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
            if width > 0.0 && height > 0.0 {
                width * height
            } else {
                0.0
            }
        };
        let component = &self.problem.components[index];
        let pose = self.poses[index];
        let own_far = if component.far_side.is_empty() {
            Vec::new()
        } else {
            built(component.far_boxes(pose))
        };
        let mut near = self.near.borrow_mut();
        let cells = self.grid.borrow_mut().query(
            rect.center,
            [rect.half[0] + self.reach, rect.half[1] + self.reach],
            &mut near,
        );
        if everything {
            near.clear();
            near.extend(0..self.rects.len());
        }
        self.work
            .set(self.work.get() + cells as u64 + near.len() as u64 * self.visit_work);
        for &other in near.iter() {
            if other == index {
                continue;
            }
            let body = &self.rects[other];
            let other_component = &self.problem.components[other];
            if self.problem.constraints.lies_on(index, other) {
                continue;
            }
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
                        // Its copper graphics keep the margin as its pads do.
                        if !part.copper_graphics.is_empty() {
                            for (center, half) in
                                built(part.boxes_of(&part.copper_graphics, part_pose))
                            {
                                total += area((center, [half[0] + margin, half[1] + margin]), hole);
                            }
                        }
                        // A cutout the user cut keeps bodies off too (by
                        // its box here; legality tests its outline).
                        let solid = &self.problem.constraints.solid_cutouts;
                        if !solid.is_empty()
                            && solid.contains(&if component.copper_only { index } else { other })
                        {
                            total += area(
                                (part.center(part_pose), part.half_extent(part_pose.angle)),
                                hole,
                            );
                        }
                    }
                } else if !component.hollow_for(other_component)
                    && !other_component.hollow_for(component)
                {
                    if self.problem.constraints.bare(index, other) {
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
                    if !component.far_courtyard.is_empty()
                        || !other_component.far_courtyard.is_empty()
                    {
                        total += self.far_courtyards(
                            index,
                            pose,
                            other,
                            self.poses[other],
                            &built,
                            &area,
                        ) + self.far_courtyards(
                            other,
                            self.poses[other],
                            index,
                            pose,
                            &built,
                            &area,
                        );
                    }
                } else {
                    // A hollow part blocks with its boxes (grown by half the
                    // spacing, as the rectangles are) only.
                    // Against an obstacle without routing room, without
                    // the halo.
                    let bare = self.problem.constraints.halo_free(index)
                        || self.problem.constraints.halo_free(other);
                    let blocking = |part: &crate::problem::Component,
                                    against: &crate::problem::Component,
                                    pose: Pose,
                                    whole: &Rect|
                     -> Vec<(Point, Point)> {
                        if !part.hollow_for(against) {
                            let halo = if bare { part.halo } else { 0.0 };
                            return vec![(
                                whole.center,
                                [whole.half[0] - halo, whole.half[1] - halo],
                            )];
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
                    total += self.far_overlap(
                        index,
                        pose,
                        &rect,
                        other,
                        self.poses[other],
                        body,
                        &built,
                        &area,
                    ) + self.far_overlap(
                        other,
                        self.poses[other],
                        body,
                        index,
                        pose,
                        &rect,
                        &built,
                        &area,
                    );
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
        let margin = component.halo + self.share(component)
            - component.edge_margin(self.problem.edge_margin);
        let mut bare = [rect.half[0] - margin, rect.half[1] - margin];
        let mut rect = rect;
        if !self.problem.constraints.is_empty() {
            // Constrained sides may touch the edge; an overhanging part
            // counts only its share on the board.
            let pose = self.poses[index];
            let inner = constraints::inner_box(self.problem, index, pose).unwrap_or_else(|| {
                let (center, half) = (component.center(pose), component.half_extent(pose.angle));
                [
                    center[0] - half[0],
                    center[1] - half[1],
                    center[0] + half[0],
                    center[1] + half[1],
                ]
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
        built: &dyn Fn(Boxes) -> Boxes,
        area: &dyn Fn((Point, Point), (Point, Point)) -> f64,
    ) -> f64 {
        let component = &self.problem.components[holes];
        if component.far_side.is_empty() && component.far_copper.is_empty() {
            return 0.0;
        }
        let other = &self.problem.components[body];
        let grow = |boxes: Vec<(Point, Point)>, margin: f64| -> Vec<(Point, Point)> {
            boxes
                .into_iter()
                .map(|(center, half)| (center, [half[0] + margin, half[1] + margin]))
                .collect()
        };
        // The body's copper: its pads, or all of it where it has none.
        let share = self.share(other);
        let pads = || {
            if other.pads.is_empty() {
                vec![(
                    body_rect.center,
                    [body_rect.half[0] - share, body_rect.half[1] - share],
                )]
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
            for far in grow(
                built(component.boxes_of(&component.far_copper, holes_pose)),
                min_spacing,
            ) {
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
        built: &dyn Fn(Boxes) -> Boxes,
        area: &dyn Fn((Point, Point), (Point, Point)) -> f64,
    ) -> f64 {
        let (component, against) = (
            &self.problem.components[yard],
            &self.problem.components[other],
        );
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
                    (
                        *their_center,
                        [their_half[0] + theirs, their_half[1] + theirs],
                    ),
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

    /// A net's term of the wirelength at the poses of now: its weighted
    /// half perimeter, guided by the field.
    fn net_length(&self, net: usize) -> f64 {
        let pins = &self.members[net];
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
        let length = (bounds[2] - bounds[0]) + (bounds[3] - bounds[1]);
        let guided = match &self.field {
            Some(field) => 1.0 + field.weight * field.mean(bounds),
            None => 1.0,
        };
        self.problem.net_weights[net] * length * guided
    }

    fn wirelength(&self, nets: &[usize]) -> f64 {
        nets.iter().map(|net| self.net_length(*net)).sum()
    }

    /// `wirelength` from the cached terms (`lengths`): the same sum of the
    /// same values in the same order, so the same bits, as long as every
    /// change of the poses or the field since went through
    /// `refresh_lengths` or `keep_lengths`.
    fn cached_wirelength(&self, nets: &[usize]) -> f64 {
        let cached: f64 = nets.iter().map(|net| self.lengths[*net]).sum();
        if self.check_lengths {
            let direct = self.wirelength(nets);
            assert_eq!(
                cached.to_bits(),
                direct.to_bits(),
                "cached wirelength {cached}, direct {direct}"
            );
        }
        cached
    }

    /// `wirelength` at the poses of now, its terms left in `terms` for
    /// `keep_lengths` (the same sum: `wirelength`'s terms in its order).
    fn wirelength_terms(&self, nets: &[usize], terms: &mut Vec<f64>) -> f64 {
        terms.clear();
        terms.extend(nets.iter().map(|net| self.net_length(*net)));
        terms.iter().copied().sum()
    }

    /// Takes the terms `wirelength_terms` gave for `nets` as the cache's
    /// (after a move it measured was kept).
    fn keep_lengths(&mut self, nets: &[usize], terms: &[f64]) {
        for (net, term) in nets.iter().zip(terms) {
            self.lengths[*net] = *term;
        }
    }

    /// Every net's term again (the start, a new field, moves made outside
    /// the single moves and swaps).
    fn refresh_lengths(&mut self) {
        self.lengths = (0..self.members.len())
            .map(|net| self.net_length(net))
            .collect();
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
        grid: std::cell::RefCell::new(crate::buckets::Grid::new(
            bounds,
            2.5,
            problem.components.len(),
        )),
        near: std::cell::RefCell::new(Vec::new()),
        // The legality test's gaps across the board reach farther.
        reach: problem.edge_margin
            + 0.5
            + problem.far_reach()
            + problem.far_courtyard_reach()
            + if legal_far_side {
                problem.spacing + problem.min_spacing
            } else {
                0.0
            },
        visit_work: (1.0 + (problem.constraints.linked.len() as f64 + 1.0).log2() / 2.0).round()
            as u64,
        check_overlap: std::env::var_os("PCB_PLACER_CHECK_OVERLAP").is_some(),
        legal_far_side,
        field: None,
        lengths: Vec::new(),
        check_lengths: cfg!(debug_assertions)
            || std::env::var_os("PCB_PLACER_CHECK_OVERLAP").is_some(),
    };
    state.rects = (0..problem.components.len())
        .map(|index| state.rect(index, state.poses[index]))
        .collect();
    for (index, rect) in state.rects.iter().enumerate() {
        state.grid.get_mut().insert(index, rect.center, rect.half);
    }
    state.refresh_lengths();
    state
}

/// Debugging aid: the overlap the annealer sees for every part at `poses`.
pub fn overlaps(problem: &Problem, poses: &[Pose], config: &AnnealConfig) -> Vec<f64> {
    let state = build_state(problem, poses, config.legal_far_side);
    (0..problem.components.len())
        .map(|index| state.overlap(index, state.rects[index]))
        .collect()
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
        poses[*index].position = constraints::snap_position(
            problem,
            *index,
            poses[*index].position,
            poses[*index].angle,
        );
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
    let fixed =
        scan + if state.rectangular {
            0
        } else {
            problem.outline.len()
        } + if held.is_empty() {
            0
        } else {
            (held.edges.len() + held.regions.len()) / 2
        };
    // The relations a part is in are evaluated before and after its move.
    let move_work: Vec<u64> = (0..problem.components.len())
        .map(|index| {
            let pins: usize = state.incident[index]
                .iter()
                .map(|net| state.members[*net].len())
                .sum();
            2 * (fixed as u64 + crate::PIN_WORK * pins as u64)
                + 2 * constraints::relation_work(problem, index)
        })
        .collect();
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        let average = move_work.iter().sum::<u64>() as f64 / move_work.len().max(1) as f64;
        eprintln!(
            "anneal: {} parts ({} movable), outline {} ({}), relations {}, linked {}, far-side parts {}, hollow parts {}, move work {average:.0}",
            problem.components.len(),
            movable.len(),
            problem.outline.len(),
            if state.rectangular {
                "rectangular"
            } else {
                "shaped"
            },
            problem.constraints.relations.len(),
            problem.constraints.linked.len(),
            problem
                .components
                .iter()
                .filter(|component| !component.far_side.is_empty())
                .count(),
            problem
                .components
                .iter()
                .filter(|component| !component.hollow.is_empty())
                .count(),
        );
    }
    // Ratsnest crossings in the anneal (`Crossings::anneal`): a move pays
    // for the crossings of its parts' nets, and half the swaps trade a
    // part with one like it.
    // Only the nets of up to `ANNEAL_NET_PINS` pins count here.
    let fact = problem
        .crossings
        .as_deref()
        .filter(|crossings| crossings.anneal)
        .map(|crossings| {
            let mut fact = crossings.clone();
            for (net, counted) in fact.counted.iter_mut().enumerate() {
                if state
                    .members
                    .get(net)
                    .is_some_and(|pins| pins.len() > ANNEAL_NET_PINS)
                {
                    *counted = false;
                }
            }
            fact
        });
    let mut crossings = fact
        .as_ref()
        .map(|fact| crate::crossings::CrossingState::new(problem, fact, &state.poses));
    let peers = if crossings.is_some() {
        peers(problem, &movable)
    } else {
        Vec::new()
    };
    let mut tally = Tally {
        crossings_before: crossings.as_ref().map(|state| state.total()),
        ..Default::default()
    };
    let work_before = crate::work();
    let mut random = Random(config.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let bounds = state.bounds;
    let board = (bounds[2] - bounds[0]).max(bounds[3] - bounds[1]);
    let grid = if problem.grid > 0.0 {
        problem.grid
    } else {
        0.05
    };
    let mut radius = (config.initial_radius * board).max(2.0 * grid);
    // Millimetres of wirelength one square millimetre of overlap is worth.
    let mut penalty = 0.5;
    let mut temperature = 0.3;
    let penalty_growth = (400.0f64 / penalty).powf(1.0 / config.stages as f64);
    let radius_decay = ((1.5 * grid) / radius).powf(1.0 / config.stages as f64);

    // The global route's guidance: evaluated now and again whenever its
    // work stays within its share of the anneal's.
    let mut guide = config
        .guidance
        .as_ref()
        .map(|guidance| Guide::new(guidance, &mut state, &movable));
    let mut anneal_work = 0u64;
    // Bus-group moves: their own crossings state (the nets of up to the
    // groups' bus size), brought to the poses before each batch.
    let groups = config
        .bus_groups
        .as_deref()
        .filter(|groups| groups.settings.anneal && !groups.groups.is_empty());
    let group_fact = groups.and_then(|groups| {
        problem.crossings.as_deref().map(|crossings| {
            let mut fact = crossings.clone();
            for (net, counted) in fact.counted.iter_mut().enumerate() {
                if state
                    .members
                    .get(net)
                    .is_some_and(|pins| pins.len() > groups.settings.bus_net_pins)
                {
                    *counted = false;
                }
            }
            fact
        })
    });
    let mut group_crossings = group_fact
        .as_ref()
        .map(|fact| crate::crossings::CrossingState::new(problem, fact, &state.poses));
    let rigid: Vec<bool> = match groups.filter(|groups| groups.settings.rigid_banks) {
        Some(groups) => {
            let mut rigid = vec![false; problem.components.len()];
            for group in &groups.groups {
                if group.kind == crate::groups::Kind::Bank {
                    for &part in &group.chain {
                        rigid[part] = true;
                    }
                }
            }
            rigid
        }
        None => Vec::new(),
    };
    let mut group_tally = GroupTally {
        crossings_before: group_crossings.as_ref().map(|state| state.total()),
        ..Default::default()
    };

    // A move's nets' wirelength terms after it (`wirelength_terms`).
    let mut terms = Vec::new();
    for stage in 0..config.stages {
        if config
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
            || config
                .stop_at_work
                .is_some_and(|stop| crate::work() >= stop)
        {
            break;
        }
        let mut work = 0;
        for _ in 0..config.moves_per_part * movable.len() {
            tally.moves += 1;
            if let Some(guide) = guide.as_mut()
                && guide.due(anneal_work + work + state.work.get())
            {
                guide.evaluate(&mut state, &movable);
            }
            let index = match guide.as_ref().and_then(|guide| guide.cumulative.as_ref()) {
                Some(cumulative) if random.next() < guide.as_ref().unwrap().bias => {
                    movable[crate::guidance::pick(cumulative, random.next())]
                }
                _ => movable[random.below(movable.len())],
            };
            // A bank's members move with their bank (`Settings::rigid_banks`).
            if rigid.get(index) == Some(&true) {
                continue;
            }
            work += move_work[index];
            let component = &problem.components[index];
            let kind = random.next();
            if kind < 0.12 && movable.len() > 1 {
                // Swap two parts, keeping each at the other's body centre.
                let other = match peers.get(index) {
                    Some(like) if !like.is_empty() && random.next() < PEER_SWAPS => {
                        like[random.below(like.len())]
                    }
                    _ => movable[random.below(movable.len())],
                };
                if other == index || rigid.get(other) == Some(&true) {
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
                let (first_new, second_new) =
                    (snapped(index, first_new), snapped(other, second_new));
                let mut nets = state.incident[index].clone();
                nets.extend(&state.incident[other]);
                nets.sort_unstable();
                nets.dedup();
                let before = state.cached_wirelength(&nets)
                    + state.constraint_cost(index)
                    + state.constraint_cost(other)
                    + penalty
                        * (state.overlap(index, state.rects[index])
                            + state.overlap(other, state.rects[other]));
                let saved = (state.rects[index], state.rects[other]);
                state.poses[index] = first_new;
                state.poses[other] = second_new;
                let (first_rect, second_rect) =
                    (state.rect(index, first_new), state.rect(other, second_new));
                state.set_rect(index, first_rect);
                state.set_rect(other, second_rect);
                let after = state.wirelength_terms(&nets, &mut terms)
                    + state.constraint_cost(index)
                    + state.constraint_cost(other)
                    + penalty
                        * (state.overlap(index, state.rects[index])
                            + state.overlap(other, state.rects[other]));
                let delta = after - before;
                let rejected = match crossings.as_mut() {
                    Some(crossings) => !judge_with_crossings(
                        crossings,
                        &state.poses,
                        &[index, other],
                        delta,
                        temperature,
                        &mut random,
                        &mut tally,
                    ),
                    None => delta > 0.0 && random.next() >= (-delta / temperature).exp(),
                };
                if rejected {
                    state.poses[index] = first_pose;
                    state.poses[other] = second_pose;
                    state.set_rect(index, saved.0);
                    state.set_rect(other, saved.1);
                } else {
                    state.keep_lengths(&nets, &terms);
                    if let Some(guide) = guide.as_mut() {
                        guide.accepted += 1;
                    }
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
            let before = state.cached_wirelength(&state.incident[index])
                + state.constraint_cost(index)
                + penalty * state.overlap(index, state.rects[index]);
            let saved = state.rects[index];
            state.poses[index] = new;
            let new_rect = state.rect(index, new);
            state.set_rect(index, new_rect);
            let after = state.wirelength_terms(&state.incident[index], &mut terms)
                + state.constraint_cost(index)
                + penalty * state.overlap(index, state.rects[index]);
            let delta = after - before;
            let rejected = match crossings.as_mut() {
                Some(crossings) => !judge_with_crossings(
                    crossings,
                    &state.poses,
                    &[index],
                    delta,
                    temperature,
                    &mut random,
                    &mut tally,
                ),
                None => delta > 0.0 && random.next() >= (-delta / temperature).exp(),
            };
            if rejected {
                state.poses[index] = old;
                state.set_rect(index, saved);
            } else {
                // The nets of a part are its own list: no copy of it.
                let incident = std::mem::take(&mut state.incident[index]);
                state.keep_lengths(&incident, &terms);
                state.incident[index] = incident;
                if let Some(guide) = guide.as_mut() {
                    guide.accepted += 1;
                }
            }
        }
        if let Some(groups) = groups {
            work += group_batch(
                &mut state,
                groups,
                &move_work,
                GroupStep {
                    radius,
                    temperature,
                    penalty,
                },
                group_crossings.as_mut(),
                crossings.as_mut(),
                &mut random,
                &mut group_tally,
            );
            // Its moves keep their own sums: the cache again.
            state.refresh_lengths();
        }
        let stage_work = work + state.work.replace(0);
        anneal_work += stage_work;
        crate::add_work(stage_work);
        penalty *= penalty_growth;
        radius *= radius_decay;
        temperature *= 0.975;
        // The term ends once its work reaches its share of the anneal's
        // own (`TERM_SHARE`).
        if let Some(state) = crossings.as_ref() {
            let done = stage + 1;
            let projected = anneal_work as f64 / done as f64 * config.stages as f64;
            if done < config.stages && tally.work as f64 >= TERM_SHARE * projected {
                tally.term_stages = Some(done);
                tally.crossings_after = Some(state.total());
                crossings = None;
            }
        }
    }
    if let Some(guide) = guide.as_mut() {
        guide.finish(&mut state, anneal_work);
    }
    if groups.is_some() && std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!(
            "anneal: bus groups: {} moves tried, taken: {} shifts, {} turns, {} swaps, {} reorders; crossings of the groups' nets {:?} -> {:?} (after the last batch)",
            group_tally.tried,
            group_tally.taken[0],
            group_tally.taken[1],
            group_tally.taken[2],
            group_tally.taken[3],
            group_tally.crossings_before,
            group_crossings.as_ref().map(|state| state.total()),
        );
    }
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        let work = crate::work() - work_before;
        let per_move = |amount: u64| amount as f64 / tally.moves.max(1) as f64;
        eprintln!(
            "anneal: {} moves, work {:.2} s ({:.0} a move), crossings' share {:.0} a move; judged with crossings {}, cut by the bound {}, taken {}; the term ended after {:?} stages; crossings {:?} -> {:?}",
            tally.moves,
            work as f64 / crate::work_per_second(),
            per_move(work),
            per_move(tally.work),
            tally.judged,
            tally.cut,
            tally.taken,
            tally.term_stages,
            tally.crossings_before,
            crossings
                .as_ref()
                .map(|state| state.total())
                .or(tally.crossings_after),
        );
    }
    *poses = state.poses.clone();
    movable
        .iter()
        .map(|index| state.overlap(*index, state.rects[*index]))
        .sum::<f64>()
        / 2.0
}

/// The anneal's crossings term counts the nets of up to this many pins.
/// A move of a part on a larger net builds that net's whole tree again,
/// and its long lines meet many: on split72_left (VDD 165 pins, VSUP
/// 110, the matrix rows 36 to 41) the term took 46000 units a move
/// against the anneal's own 3600 and ate the placement's budget before
/// two parts had a place. The orders the term is for are buses of few
/// pins; power nets and matrix rows spread over the board whatever the
/// order (the legaliser and the refinement count every net).
pub const ANNEAL_NET_PINS: usize = 16;

/// The share of the anneal's swaps (with the crossings term) whose
/// partner is one of the part's `peers`.
const PEER_SWAPS: f64 = 0.5;

/// Counts of the anneal for its debug line.
#[derive(Default)]
struct Tally {
    moves: u64,
    /// Moves judged with the crossings term, those the bound rejected
    /// without the new trees, and the term's work.
    judged: u64,
    cut: u64,
    taken: u64,
    work: u64,
    crossings_before: Option<u64>,
    /// Where the term ended early: after this many stages, at this count.
    term_stages: Option<usize>,
    crossings_after: Option<u64>,
}

/// The most work the anneal's crossings term may take, as a share of
/// the anneal's own work over all its stages (projected after each
/// stage); once it has, the anneal goes on without it. Where lines are
/// dense the term costs several times the anneal (video: 12700 units a
/// move against 2800), and an anneal that pays for crossings to its end
/// leaves its overlaps to the legaliser, which undid the gain there
/// (19728 crossings off, 21361 with the term to the end, 14280 with it
/// for the first 13 of 160 stages at this share; MegaDrive keeps it for
/// 119 to 132 stages, Glasgow for all).
const TERM_SHARE: f64 = 0.5;

/// Per part, the movable parts the anneal's swaps (with the crossings
/// term) trade it with half the time: on the same side, and either of the
/// same footprint (pin count and body), or both with many pins
/// (`crossings::REORDER_PINS`) and the one with fewer at least
/// `crossings::REORDER_PIN_SHARE` of the other's (MegaDrive's QFP-160 and
/// QFP-128, whose order along the bus to the connectors the crossings
/// see). A random partner of a QFP is nearly always a passive, a trade
/// the overlap rejects at once.
fn peers(problem: &Problem, movable: &[usize]) -> Vec<Vec<usize>> {
    let mut peers = vec![Vec::new(); problem.components.len()];
    for (position, &a) in movable.iter().enumerate() {
        let first = &problem.components[a];
        for &b in &movable[position + 1..] {
            let second = &problem.components[b];
            if first.side != second.side || first.pins.is_empty() || second.pins.is_empty() {
                continue;
            }
            let same = first.pins.len() == second.pins.len()
                && first.round == second.round
                && (first.body_size[0] - second.body_size[0]).abs() <= 1.0e-6
                && (first.body_size[1] - second.body_size[1]).abs() <= 1.0e-6;
            let (fewer, more) = (
                first.pins.len().min(second.pins.len()),
                first.pins.len().max(second.pins.len()),
            );
            let large = fewer >= crate::crossings::REORDER_PINS
                && fewer as f64 >= crate::crossings::REORDER_PIN_SHARE * more as f64;
            if same || large {
                peers[a].push(b);
                peers[b].push(a);
            }
        }
    }
    peers
}

/// Whether the anneal takes a move whose other terms change the cost by
/// `delta`, with the crossings of the moved parts' nets (`state`, the
/// parts at their new poses in `poses`) at their weight: the rule of the
/// plain anneal (taken when the cost falls, else with probability
/// exp(-change / temperature), one random number drawn exactly when the
/// change is positive). The crossings as kept bound what the move can
/// save; when even saving all of them leaves the move rejected by the
/// number drawn, the new trees are not built.
fn judge_with_crossings(
    state: &mut crate::crossings::CrossingState,
    poses: &[Pose],
    moved: &[usize],
    delta: f64,
    temperature: f64,
    random: &mut Random,
    tally: &mut Tally,
) -> bool {
    let work = crate::work();
    tally.judged += 1;
    let weight = state.weight();
    let before = state.before(moved);
    let lowest = delta - weight * before as f64;
    let mut drawn = None;
    let taken = 'judged: {
        if lowest > 0.0 {
            let draw = random.next();
            if draw >= (-lowest / temperature).exp() {
                state.settle(poses, moved, false, 0);
                tally.cut += 1;
                break 'judged false;
            }
            drawn = Some(draw);
        }
        let after = state.after(poses);
        let change = after as i64 - before as i64;
        let total = delta + weight * change as f64;
        let taken =
            total <= 0.0 || drawn.unwrap_or_else(|| random.next()) < (-total / temperature).exp();
        state.settle(poses, moved, taken, change);
        taken
    };
    // The count's work in the anneal's units (`CROSSING_WORK`).
    let counted = crate::work() - work;
    let extra = counted * (CROSSING_WORK.0 - CROSSING_WORK.1) / CROSSING_WORK.1;
    crate::add_work(extra);
    tally.work += counted + extra;
    tally.taken += u64::from(taken);
    taken
}

/// The crossings' work in the anneal counts this many times (numerator,
/// denominator) what the count itself books: the count books each
/// operation at the CPU it takes in the legaliser and the refinement
/// (crossings.rs, `MET_WORK`), and against the anneal's own work its
/// queries (the kept lines counted again, the new lines among themselves
/// directly, most never entering the grid) take 1.6 times the CPU per unit
/// booked (MegaDrive 1.60, Glasgow 1.57, 2026-10-25; 13/5 of the count's
/// earlier booking of one unit per cell and line met), and a budget in
/// work should see what the term costs.
const CROSSING_WORK: (u64, u64) = (8, 5);

/// Counts of the bus-group moves for the debug line: tried, and taken
/// by kind (shift, turn, swap, reorder).
#[derive(Default)]
struct GroupTally {
    tried: u64,
    taken: [u64; 4],
    crossings_before: Option<u64>,
    /// The crossings' own counts (kept apart from the anneal term's,
    /// whose share of the work they would change).
    crossings: Tally,
}

/// A rigid bank's share of a stage's group moves: this many per member
/// (each with `Settings::anneal_moves`), where its members' single moves
/// would be `moves_per_part` each.
const BANK_SHARE: usize = 3;

/// A group move's work per part beyond its single move's: its new pose
/// (turned, snapped) and the lists the move builds.
const GROUP_PART_WORK: u64 = 16;

/// The stage's move radius, temperature and overlap penalty.
#[derive(Clone, Copy)]
struct GroupStep {
    radius: f64,
    temperature: f64,
    penalty: f64,
}

/// One stage's bus-group moves (`AnnealConfig::bus_groups`): groups
/// (their parts at the poses of now, `BusGroups::attach`) chosen at
/// random shift (half the time towards the pull of their nets, else by
/// a step of the stage's radius), turn about their centre, trade places
/// with a group like them, or (a bank) reorder along their axis; each
/// judged as a single move is, by the wirelength of the parts' nets,
/// their constraints and their overlap at the stage's penalty, and with
/// `group_crossings` the crossings of their nets (`judge_with_crossings`)
/// at the stage's temperature. Returns the work besides what the overlap
/// sums and the crossings count book.
#[allow(clippy::too_many_arguments)]
fn group_batch(
    state: &mut State,
    groups: &crate::groups::BusGroups,
    move_work: &[u64],
    step: GroupStep,
    mut group_crossings: Option<&mut crate::crossings::CrossingState>,
    mut crossings: Option<&mut crate::crossings::CrossingState>,
    random: &mut Random,
    group_tally: &mut GroupTally,
) -> u64 {
    use crate::groups::{Kind, Reorder, chain_center, pull, reordered, rotated, translated};
    let problem = state.problem;
    let parts = groups.attach(problem, &state.poses);
    let active: Vec<usize> = (0..groups.groups.len())
        .filter(|group| groups.groups[*group].kind == Kind::Bank || parts[*group].len() > 1)
        .collect();
    if active.is_empty() {
        return 0;
    }
    if let Some(group_crossings) = group_crossings.as_mut() {
        group_crossings.resync(&state.poses);
    }
    // Each active group's share of the attempts: one, a rigid bank's as
    // many as its members get single moves in an eighth of a stage.
    let shares: Vec<f64> = active
        .iter()
        .map(|group| {
            let group = &groups.groups[*group];
            if group.kind == Kind::Bank && groups.settings.rigid_banks {
                (BANK_SHARE * group.chain.len()) as f64
            } else {
                1.0
            }
        })
        .collect();
    let total: f64 = shares.iter().sum();
    let attempts = (groups.settings.anneal_moves * total).round() as usize;
    let grid = if problem.grid > 0.0 {
        problem.grid
    } else {
        0.05
    };
    let mut work = 0;
    for _ in 0..attempts {
        let index = {
            let mut draw = random.next() * total;
            let mut chosen = active[active.len() - 1];
            for (position, share) in shares.iter().enumerate() {
                if draw < *share {
                    chosen = active[position];
                    break;
                }
                draw -= share;
            }
            chosen
        };
        let group = &groups.groups[index];
        let kind = random.next();
        let (which, moved) = if kind < 0.2 && group.kind == Kind::Bank {
            let how = match random.below(3) {
                0 => Reorder::InPlace,
                1 => Reorder::Along(0),
                _ => Reorder::Along(1),
            };
            let margin = |part: usize| {
                let component = &problem.components[part];
                component.halo + state.share(component)
            };
            (
                3,
                reordered(problem, &state.poses, group, &parts[index], how, &margin),
            )
        } else if kind < 0.45 && !group.peers.is_empty() {
            let other = group.peers[random.below(group.peers.len())];
            if parts[index]
                .iter()
                .any(|part| parts[other].binary_search(part).is_ok())
            {
                (2, None)
            } else {
                let a = chain_center(problem, &state.poses, &group.chain);
                let b = chain_center(problem, &state.poses, &groups.groups[other].chain);
                let mut moved = translated(
                    problem,
                    &state.poses,
                    &parts[index],
                    [b[0] - a[0], b[1] - a[1]],
                );
                moved.extend(translated(
                    problem,
                    &state.poses,
                    &parts[other],
                    [a[0] - b[0], a[1] - b[1]],
                ));
                (2, Some(moved))
            }
        } else if kind < 0.6 {
            let center = chain_center(problem, &state.poses, &group.chain);
            let turn = [90.0, 180.0, 270.0][random.below(3)];
            (
                1,
                rotated(problem, &state.poses, &parts[index], center, turn),
            )
        } else {
            let shift = if random.next() < 0.5 {
                pull(problem, &state.poses, &parts[index]).map(|towards| {
                    let fraction = random.next();
                    [towards[0] * fraction, towards[1] * fraction]
                })
            } else {
                let cells = (step.radius / grid).max(1.0);
                let mut draw = || ((random.next() * 2.0 - 1.0) * cells).round() * grid;
                Some([draw(), draw()])
            };
            (
                0,
                shift
                    .filter(|shift| shift[0] != 0.0 || shift[1] != 0.0)
                    .map(|shift| translated(problem, &state.poses, &parts[index], shift)),
            )
        };
        let Some(moved) = moved else {
            continue;
        };
        if moved.iter().all(|(part, pose)| state.poses[*part] == *pose) {
            continue;
        }
        group_tally.tried += 1;
        let list: Vec<usize> = moved.iter().map(|(part, _)| *part).collect();
        let mut nets: Vec<usize> = list
            .iter()
            .flat_map(|part| state.incident[*part].iter().copied())
            .collect();
        nets.sort_unstable();
        nets.dedup();
        // Each part's move as a single move books it, and the poses built.
        for part in &list {
            work += move_work[*part] + GROUP_PART_WORK;
        }
        let mut sorted = list.clone();
        sorted.sort_unstable();
        let cost = |state: &State| {
            state.wirelength(&nets)
                + crate::groups::held_cost(problem, &state.poses, &sorted)
                + step.penalty
                    * list
                        .iter()
                        .map(|part| state.overlap(*part, state.rects[*part]))
                        .sum::<f64>()
        };
        let before = cost(state);
        let saved: Vec<(usize, Pose, Rect)> = list
            .iter()
            .map(|part| (*part, state.poses[*part], state.rects[*part]))
            .collect();
        for (part, pose) in &moved {
            state.poses[*part] = *pose;
            let rect = state.rect(*part, *pose);
            state.set_rect(*part, rect);
        }
        let delta = cost(state) - before;
        let taken = match group_crossings.as_mut() {
            Some(group_crossings) => judge_with_crossings(
                group_crossings,
                &state.poses,
                &list,
                delta,
                step.temperature,
                random,
                &mut group_tally.crossings,
            ),
            None => delta <= 0.0 || random.next() < (-delta / step.temperature).exp(),
        };
        if taken {
            group_tally.taken[which] += 1;
            if let Some(crossings) = crossings.as_mut() {
                crossings.kept(&state.poses, &list);
            }
        } else {
            for (part, pose, rect) in saved {
                state.poses[part] = pose;
                state.set_rect(part, rect);
            }
        }
    }
    work
}

/// The anneal's side of `AnnealConfig::guidance`.
struct Guide<'a> {
    guidance: &'a crate::guidance::Guidance,
    bias: f64,
    /// Over the movable parts, for draws by attribution.
    cumulative: Option<Vec<f64>>,
    accepted: usize,
    evaluations: usize,
    work: u64,
    last_work: u64,
    first_overflow: f64,
    last_overflow: f64,
}

impl<'a> Guide<'a> {
    fn new(guidance: &'a crate::guidance::Guidance, state: &mut State, movable: &[usize]) -> Self {
        let mut guide = Self {
            guidance,
            bias: guidance.bias,
            cumulative: None,
            accepted: 0,
            evaluations: 0,
            work: 0,
            last_work: 0,
            first_overflow: f64::NAN,
            last_overflow: f64::NAN,
        };
        guide.evaluate(state, movable);
        guide
    }

    /// Whether another evaluation is due, `anneal_work` done so far.
    fn due(&self, anneal_work: u64) -> bool {
        self.accepted >= self.guidance.min_accepted.max(1)
            && (self.work + self.last_work) as f64 <= self.guidance.work_share * anneal_work as f64
    }

    fn evaluate(&mut self, state: &mut State, movable: &[usize]) {
        self.accepted = 0;
        let Some(field) = (self.guidance.evaluate)(&state.poses) else {
            // Nothing to go by: the plain anneal from here.
            state.field = None;
            state.refresh_lengths();
            self.cumulative = None;
            self.last_work = u64::MAX / 4;
            return;
        };
        crate::add_work(field.work);
        self.work += field.work;
        self.last_work = field.work;
        self.evaluations += 1;
        if self.first_overflow.is_nan() {
            self.first_overflow = field.total_overflow;
        }
        self.last_overflow = field.total_overflow;
        state.field = crate::guidance::Prepared::new(&field, self.guidance.weight);
        state.refresh_lengths();
        self.cumulative = crate::guidance::cumulative(&field, movable);
    }

    /// One evaluation of the result, for the trace.
    fn finish(&mut self, state: &mut State, anneal_work: u64) {
        let Some(field) = (self.guidance.evaluate)(&state.poses) else {
            return;
        };
        crate::add_work(field.work);
        self.work += field.work;
        // The excess at the guidance's threshold, as the evaluator counts it.
        eprintln!(
            "guidance: {} evaluations, total overflow {:.0} first, {:.0} last, {:.0} annealed; work {:.1} s of the anneal's {:.1} s",
            self.evaluations,
            self.first_overflow,
            self.last_overflow,
            field.total_overflow,
            self.work as f64 / crate::work_per_second(),
            anneal_work as f64 / crate::work_per_second(),
        );
        state.field = None;
    }
}

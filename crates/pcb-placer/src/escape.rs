// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Escape demand (`Problem::escape`, an experiment, docs/placer.md "Escape
//! demand (2026-10-23)"): where a part's tracks leave its body, and
//! whether the channel between two parts can carry them.
//!
//! A pad's net leaves the body through the side on the cheapest way to the
//! net's nearest pad on another part (under the body costs twice), so a
//! part's demand on a side is the number of its nets leaving there,
//! recomputed from the parts' current positions. Two parts on one board
//! side whose bodies face each other across a gap form a channel: the
//! tracks leaving the facing stretches of both (except those going to the
//! other part, which cross the gap) must run along it, and it holds the
//! gap over the track pitch times the routable layers. Demand over that
//! capacity is the channel's overflow, attributed to both parts. A side
//! facing the board edge or a part without demand towards it costs
//! nothing.
//!
//! The global placement inflates each part's charge on a side by that
//! side's demand (`inflation`), the rotation choice prefers the angle that
//! points the dense sides at free space when the wirelength is close
//! (`orient::escape_blocking`), and the legaliser and the refinement add
//! the overflow a move causes to its cost (`EscapeState`). The refinement
//! corrects the estimate by a measured version: the global route's
//! overflow attribution per part (`Escape::measured`), evaluated again
//! every `measure_every` kept moves.

use crate::buckets::Grid;
use crate::problem::{Point, Pose, Problem};
use std::sync::Arc;

/// One measurement of the placement's global route: per component the
/// overflow attributed to it, and the placement work it counts.
pub type MeasuredSession = Box<dyn FnMut(&[Pose]) -> Option<(Vec<f64>, u64)> + Send>;
/// Opens a measuring session (each keeps its own incremental route).
pub type MeasuredFactory = dyn Fn() -> MeasuredSession + Send + Sync;

/// The lowering's escape fact and the experiment's settings.
#[derive(Clone)]
pub struct Escape {
    /// Track pitch (width plus clearance), mm.
    pub pitch: f64,
    /// Layers a channel's tracks may use.
    pub layers: f64,
    /// Per net: its escapes count (not carried by a pour, two pins or
    /// more).
    pub counted: Vec<bool>,
    /// Millimetres of wirelength one track of channel overflow costs in
    /// the legaliser and the refinement.
    pub weight: f64,
    /// Factor on the global placement's charge inflation (a side's demand
    /// times the pitch over the layers).
    pub inflate: f64,
    /// The rotation choice: angles within this share of the shortest
    /// wirelength are ranked by how free their dense sides face.
    pub turn_tolerance: f64,
    /// The measured version (the global route's attribution), if any.
    pub measured: Option<Arc<MeasuredFactory>>,
    /// Kept refinement moves between two measurements.
    pub measure_every: usize,
}

impl std::fmt::Debug for Escape {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Escape")
            .field("pitch", &self.pitch)
            .field("layers", &self.layers)
            .field("weight", &self.weight)
            .field("inflate", &self.inflate)
            .field("turn_tolerance", &self.turn_tolerance)
            .field("measured", &self.measured.is_some())
            .field("measure_every", &self.measure_every)
            .finish()
    }
}

impl Escape {
    /// Track room per track of demand, mm.
    pub fn track_room(&self) -> f64 {
        self.pitch / self.layers.max(1.0)
    }
}

/// Channels wider than this are no channels (other parts lie between).
pub const CHANNEL_REACH: f64 = 5.0;

/// Sides of an axis-aligned body: towards -x, +x, -y, +y.
pub const SIDES: usize = 4;

/// Where one of a part's nets leaves its body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Exit {
    pub side: usize,
    /// The exit's coordinate along its side (y for the x sides, x for the
    /// y sides).
    pub along: f64,
    /// The part the net's nearest other pad belongs to.
    pub target: usize,
}

/// A net's pins as (component, pin).
pub type NetPins = Vec<(usize, usize)>;

/// Per net, its pins.
pub fn members(problem: &Problem) -> Vec<NetPins> {
    let mut members = vec![Vec::new(); problem.net_weights.len()];
    for (index, component) in problem.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            members[description.net].push((index, pin));
        }
    }
    members
}

/// The body's axis-aligned box at `pose`: centre and half extent.
pub fn body_box(problem: &Problem, index: usize, pose: Pose) -> (Point, Point) {
    let component = &problem.components[index];
    (component.center(pose), component.half_extent(pose.angle))
}

/// The exits of part `index` at `poses[index]`, the other parts where
/// `poses` has them: one per net and side.
pub fn exits(
    problem: &Problem,
    escape: &Escape,
    members: &[Vec<(usize, usize)>],
    poses: &[Pose],
    index: usize,
    out: &mut Vec<Exit>,
) {
    out.clear();
    let component = &problem.components[index];
    let pose = poses[index];
    let (center, half) = body_box(problem, index, pose);
    let low = [center[0] - half[0], center[1] - half[1]];
    let high = [center[0] + half[0], center[1] + half[1]];
    let mut nets: Vec<(usize, usize)> = Vec::new();
    let mut work = 0u64;
    for pin in &component.pins {
        if !escape.counted.get(pin.net).copied().unwrap_or(false) {
            continue;
        }
        let at = component.pin_position(pin, pose);
        let mut nearest: Option<(f64, Point, usize)> = None;
        work += members[pin.net].len() as u64;
        for (other, other_pin) in &members[pin.net] {
            if *other == index {
                continue;
            }
            let part = &problem.components[*other];
            let there = part.pin_position(&part.pins[*other_pin], poses[*other]);
            let distance = (there[0] - at[0]).powi(2) + (there[1] - at[1]).powi(2);
            if nearest.is_none_or(|(best, _, _)| distance < best) {
                nearest = Some((distance, there, *other));
            }
        }
        let Some((_, target, owner)) = nearest else {
            continue;
        };
        let mut best = (f64::INFINITY, 0usize, 0.0f64);
        for side in 0..SIDES {
            let (exit, inside) = match side {
                0 => ([low[0], at[1].clamp(low[1], high[1])], at[0] - low[0]),
                1 => ([high[0], at[1].clamp(low[1], high[1])], high[0] - at[0]),
                2 => ([at[0].clamp(low[0], high[0]), low[1]], at[1] - low[1]),
                _ => ([at[0].clamp(low[0], high[0]), high[1]], high[1] - at[1]),
            };
            let cost = 2.0 * inside.max(0.0)
                + ((target[0] - exit[0]).powi(2) + (target[1] - exit[1]).powi(2)).sqrt();
            if cost < best.0 - 1.0e-9 {
                let along = if side < 2 { exit[1] } else { exit[0] };
                best = (cost, side, along);
            }
        }
        if nets.contains(&(pin.net, best.1)) {
            continue;
        }
        nets.push((pin.net, best.1));
        out.push(Exit {
            side: best.1,
            along: best.2,
            target: owner,
        });
    }
    crate::add_work(crate::PIN_WORK * (work + component.pins.len() as u64));
}

/// The demand per side (-x, +x, -y, +y) of a part's exits.
pub fn demand(exits: &[Exit]) -> [usize; SIDES] {
    let mut demand = [0; SIDES];
    for exit in exits {
        demand[exit.side] += 1;
    }
    demand
}

/// The channel between parts `a` and `b` (their poses and exits): its
/// tracks over capacity, 0 where the bodies do not face each other across
/// a gap, either side has no demand towards the other, or the gap is
/// wider than `CHANNEL_REACH`.
#[allow(clippy::too_many_arguments)]
pub fn channel(
    problem: &Problem,
    escape: &Escape,
    a: usize,
    pose_a: Pose,
    exits_a: &[Exit],
    b: usize,
    pose_b: Pose,
    exits_b: &[Exit],
) -> f64 {
    let (first, second) = (&problem.components[a], &problem.components[b]);
    if !first.side.collides(second.side) || exits_a.is_empty() || exits_b.is_empty() {
        return 0.0;
    }
    let (center_a, half_a) = body_box(problem, a, pose_a);
    let (center_b, half_b) = body_box(problem, b, pose_b);
    let gap = |axis: usize| (center_b[axis] - center_a[axis]).abs() - half_a[axis] - half_b[axis];
    let (gap_x, gap_y) = (gap(0), gap(1));
    // The axis the gap is across, and the other one the channel runs
    // along.
    let (across, along) = if gap_x >= 0.0 && gap_y < 0.0 {
        (0, 1)
    } else if gap_y >= 0.0 && gap_x < 0.0 {
        (1, 0)
    } else {
        return 0.0;
    };
    let width = if across == 0 { gap_x } else { gap_y };
    if width > CHANNEL_REACH {
        return 0.0;
    }
    let stretch = [
        (center_a[along] - half_a[along]).max(center_b[along] - half_b[along]) - escape.pitch / 2.0,
        (center_a[along] + half_a[along]).min(center_b[along] + half_b[along]) + escape.pitch / 2.0,
    ];
    let b_beyond = center_b[across] > center_a[across];
    let side_a = 2 * across + b_beyond as usize;
    let side_b = 2 * across + (!b_beyond) as usize;
    let count = |exits: &[Exit], side: usize, other: usize| {
        exits
            .iter()
            .filter(|exit| {
                exit.side == side
                    && exit.target != other
                    && exit.along >= stretch[0]
                    && exit.along <= stretch[1]
            })
            .count()
    };
    let demand_a = count(exits_a, side_a, b);
    let demand_b = count(exits_b, side_b, a);
    crate::add_work((exits_a.len() + exits_b.len()) as u64);
    if demand_a == 0 || demand_b == 0 {
        return 0.0;
    }
    let capacity = width.max(0.0) / escape.track_room();
    (demand_a as f64 + demand_b as f64 - capacity).max(0.0)
}

/// The global placement's inflation of a part's charge per side (-x, +x,
/// -y, +y), mm: its demand there times the track room.
pub fn inflation(escape: &Escape, exits: &[Exit]) -> [f64; SIDES] {
    demand(exits).map(|count| escape.inflate * count as f64 * escape.track_room())
}

/// The escape term of the legaliser and the refinement: the parts' exits
/// kept up to date as they move, the placed parts by area, and the
/// measured correction per part.
pub struct EscapeState<'a> {
    problem: &'a Problem,
    escape: &'a Escape,
    members: Vec<Vec<(usize, usize)>>,
    /// Per part, its exits where it was last refreshed.
    exits: Vec<Vec<Exit>>,
    grid: Grid,
    /// Where each part is in the grid (`None`: not in it).
    in_grid: Vec<Option<Pose>>,
    near: Vec<usize>,
    scratch: Vec<Exit>,
    /// Per part, the measured correction of its term (1 until measured).
    scale: Vec<f64>,
    session: Option<MeasuredSession>,
    kept: usize,
    /// Measurements taken.
    pub measurements: usize,
}

impl<'a> EscapeState<'a> {
    /// The state of `poses`, with the parts `present` says in the grid
    /// (the legaliser's placed parts; all in the refinement).
    pub fn new(
        problem: &'a Problem,
        escape: &'a Escape,
        poses: &[Pose],
        present: impl Fn(usize) -> bool,
    ) -> Self {
        let count = problem.components.len();
        let members = members(problem);
        let mut exits_of = vec![Vec::new(); count];
        for (index, exits_out) in exits_of.iter_mut().enumerate() {
            exits(problem, escape, &members, poses, index, exits_out);
        }
        let mut state = EscapeState {
            problem,
            escape,
            members,
            exits: exits_of,
            grid: Grid::new(problem.bounds(), 2.5, count),
            in_grid: vec![None; count],
            near: Vec::new(),
            scratch: Vec::new(),
            scale: vec![1.0; count],
            session: None,
            kept: 0,
            measurements: 0,
        };
        for (index, pose) in poses.iter().enumerate().take(count) {
            if present(index) {
                state.insert(index, *pose);
            }
        }
        state
    }

    /// Opens the measured version (the refinement): one measurement now,
    /// another every `measure_every` kept moves.
    pub fn measure_from_now(&mut self, poses: &[Pose]) {
        if let Some(factory) = &self.escape.measured {
            self.session = Some(factory());
            self.measure(poses);
        }
    }

    fn measure(&mut self, poses: &[Pose]) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let Some((attribution, work)) = session(poses) else {
            return;
        };
        crate::add_work(work);
        self.measurements += 1;
        let positive: Vec<f64> = attribution.iter().copied().filter(|a| *a > 0.0).collect();
        if positive.is_empty() || attribution.len() != self.scale.len() {
            return;
        }
        let mean = positive.iter().sum::<f64>() / positive.len() as f64;
        for (scale, measured) in self.scale.iter_mut().zip(&attribution) {
            *scale = 2.0 * measured / (measured + mean);
        }
    }

    /// Kept moves seen (`kept`).
    pub fn kept_moves(&self) -> usize {
        self.kept
    }

    pub fn insert(&mut self, index: usize, pose: Pose) {
        if let Some(old) = self.in_grid[index].take() {
            let (center, half) = body_box(self.problem, index, old);
            self.grid.remove(index, center, half);
        }
        let (center, half) = body_box(self.problem, index, pose);
        self.grid.insert(index, center, half);
        self.in_grid[index] = Some(pose);
    }

    /// The grid and every part's exits at `poses` (parts moved by others
    /// since: the relation repair between two refinements).
    pub fn resync(&mut self, poses: &[Pose]) {
        for index in 0..self.in_grid.len() {
            if let Some(pose) = self.in_grid[index]
                && pose != poses[index]
            {
                self.insert(index, poses[index]);
            }
            self.refresh(poses, index);
        }
    }

    /// The part's exits again, at `poses`.
    pub fn refresh(&mut self, poses: &[Pose], index: usize) {
        let mut out = std::mem::take(&mut self.exits[index]);
        exits(
            self.problem,
            self.escape,
            &self.members,
            poses,
            index,
            &mut out,
        );
        self.exits[index] = out;
    }

    /// The overflow of the channels part `index` forms at `poses[index]`
    /// with the parts in the grid (their exits as last refreshed, its own
    /// computed at `poses`), each scaled by the two parts' measured
    /// corrections, times the weight: millimetres of wirelength.
    pub fn cost(&mut self, poses: &[Pose], index: usize) -> f64 {
        self.cost_without(poses, index, index)
    }

    /// `cost` leaving out the channel with part `without` (a swap's
    /// partner, whose place in the grid is the other's).
    pub fn cost_without(&mut self, poses: &[Pose], index: usize, without: usize) -> f64 {
        let pose = poses[index];
        let mut own = std::mem::take(&mut self.scratch);
        exits(
            self.problem,
            self.escape,
            &self.members,
            poses,
            index,
            &mut own,
        );
        let mut total = 0.0;
        if !own.is_empty() {
            let (center, half) = body_box(self.problem, index, pose);
            let reach = CHANNEL_REACH + 0.1;
            let cells = self
                .grid
                .query(center, [half[0] + reach, half[1] + reach], &mut self.near);
            crate::add_work((cells + self.near.len()) as u64);
            for &other in &self.near {
                if other == index || other == without {
                    continue;
                }
                let Some(other_pose) = self.in_grid[other] else {
                    continue;
                };
                let overflow = channel(
                    self.problem,
                    self.escape,
                    index,
                    pose,
                    &own,
                    other,
                    other_pose,
                    &self.exits[other],
                );
                if overflow > 0.0 {
                    total += overflow * (self.scale[index] + self.scale[other]) / 2.0;
                }
            }
        }
        self.scratch = own;
        self.escape.weight * total
    }

    /// After a kept move of `moved`: the grid and the exits of the moved
    /// parts and of the parts sharing a small net with them; every
    /// `measure_every` kept moves the measured correction again.
    pub fn kept(&mut self, poses: &[Pose], moved: &[usize]) {
        let mut touched: Vec<usize> = moved.to_vec();
        for &index in moved {
            if self.in_grid[index].is_some() {
                self.insert(index, poses[index]);
            }
            for pin in &self.problem.components[index].pins {
                let members = &self.members[pin.net];
                if members.len() <= 24 && self.escape.counted.get(pin.net) == Some(&true) {
                    touched.extend(members.iter().map(|(part, _)| *part));
                }
            }
        }
        touched.sort_unstable();
        touched.dedup();
        for index in touched {
            self.refresh(poses, index);
        }
        self.kept += 1;
        if self.session.is_some()
            && self.escape.measure_every > 0
            && self.kept.is_multiple_of(self.escape.measure_every)
        {
            self.measure(poses);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::{Component, Pin, Side};

    fn part(name: &str, size: Point, pins: Vec<Pin>) -> Component {
        Component {
            name: name.into(),
            body_center: [0.0, 0.0],
            body_size: size,
            round: false,
            halo: 0.0,
            pins,
            side: Side::Front,
            fixed: false,
            angle_options: vec![0.0, 90.0, 180.0, 270.0],
            far_side: Vec::new(),
            far_copper: Vec::new(),
            far_courtyard: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: Vec::new(),
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
            hole_rings: Vec::new(),
        }
    }

    fn pose(x: f64, y: f64) -> Pose {
        Pose {
            position: [x, y],
            angle: 0.0,
        }
    }

    fn problem(components: Vec<Component>, nets: usize, poses: Vec<Pose>) -> Problem {
        Problem {
            outline: vec![[-50.0, -50.0], [50.0, -50.0], [50.0, 50.0], [-50.0, 50.0]],
            components,
            net_weights: vec![1.0; nets],
            poses,
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
        }
    }

    fn escape(nets: usize) -> Escape {
        Escape {
            pitch: 0.5,
            layers: 2.0,
            counted: vec![true; nets],
            weight: 1.0,
            inflate: 1.0,
            turn_tolerance: 0.05,
            measured: None,
            measure_every: 0,
        }
    }

    /// Two chips 10 mm square, side by side with `gap` between their
    /// facing sides; each has `count` pads on that side whose nets go to
    /// far parts beyond the chips, above them (y = 30): the tracks must
    /// leave through the facing sides... unless the cheaper way is up.
    /// Here the far parts sit straight beyond the channel's far end below
    /// the facing sides, so the nets leave through the facing sides.
    fn facing_chips(gap: f64, count: usize) -> (Problem, Escape) {
        let mut components = Vec::new();
        let mut poses = Vec::new();
        let mut nets = 0;
        let mut left = Vec::new();
        let mut right = Vec::new();
        for pad in 0..count {
            let y = -4.0 + 8.0 * pad as f64 / count.max(2) as f64;
            left.push(Pin {
                offset: [4.9, y],
                net: nets,
            });
            nets += 1;
            right.push(Pin {
                offset: [-4.9, y],
                net: nets,
            });
            nets += 1;
        }
        components.push(part("U1", [10.0, 10.0], left));
        poses.push(pose(0.0, 0.0));
        components.push(part("U2", [10.0, 10.0], right));
        poses.push(pose(10.0 + gap, 0.0));
        // The other ends: small parts in the channel's line, beyond the
        // chips (x between them, y far below).
        for net in 0..nets {
            components.push(part(
                &format!("R{net}"),
                [1.0, 0.5],
                vec![Pin {
                    offset: [0.0, 0.0],
                    net,
                }],
            ));
            poses.push(pose(5.0 + gap / 2.0, -40.0 + net as f64));
        }
        (problem(components, nets, poses), escape(nets))
    }

    #[test]
    fn a_pad_leaves_through_the_side_towards_its_net() {
        let (problem, escape) = facing_chips(2.0, 4);
        let members = members(&problem);
        let mut out = Vec::new();
        exits(&problem, &escape, &members, &problem.poses, 0, &mut out);
        // The pads sit 0.1 mm inside U1's +x side, the nets' other ends
        // in the gap's line far below: they leave through +x (cheaper
        // than 4.9 mm under the body to the bottom).
        assert_eq!(out.len(), 4);
        assert!(out.iter().all(|exit| exit.side == 1), "{out:?}");
        assert_eq!(demand(&out), [0, 4, 0, 0]);
        exits(&problem, &escape, &members, &problem.poses, 1, &mut out);
        assert_eq!(demand(&out), [4, 0, 0, 0]);
    }

    #[test]
    fn a_pad_far_from_its_target_side_turns_to_the_cheaper_side() {
        // One pad in the middle of U1's -x half, its net's other end far
        // to the +x side: under the body to +x costs twice the inside
        // way, out through -x and around costs the detour.
        let components = vec![
            part(
                "U1",
                [10.0, 2.0],
                vec![Pin {
                    offset: [-4.0, 0.0],
                    net: 0,
                }],
            ),
            part(
                "R1",
                [1.0, 1.0],
                vec![Pin {
                    offset: [0.0, 0.0],
                    net: 0,
                }],
            ),
        ];
        let problem = problem(components, 1, vec![pose(0.0, 0.0), pose(-20.0, 0.0)]);
        let escape = escape(1);
        let members = members(&problem);
        let mut out = Vec::new();
        exits(&problem, &escape, &members, &problem.poses, 0, &mut out);
        assert_eq!(out[0].side, 0);
        assert_eq!(out[0].target, 1);
        assert!((out[0].along - 0.0).abs() < 1.0e-9);
    }

    #[test]
    fn a_poured_net_needs_no_escape() {
        let (problem, mut escape) = facing_chips(2.0, 4);
        escape.counted[0] = false;
        let members = members(&problem);
        let mut out = Vec::new();
        exits(&problem, &escape, &members, &problem.poses, 0, &mut out);
        assert_eq!(demand(&out), [0, 3, 0, 0]);
    }

    #[test]
    fn a_channel_overflows_when_demand_exceeds_the_gap_over_the_pitch() {
        // 4 + 4 tracks; a 1 mm gap at a 0.5 mm pitch on two layers holds
        // 4: 4 over. At 2 mm it holds 8: none over. At 0.5 mm, 2: 6 over.
        for (gap, over) in [(1.0, 4.0), (2.0, 0.0), (0.5, 6.0), (3.0, 0.0)] {
            let (problem, escape) = facing_chips(gap, 4);
            let members = members(&problem);
            let (mut a, mut b) = (Vec::new(), Vec::new());
            exits(&problem, &escape, &members, &problem.poses, 0, &mut a);
            exits(&problem, &escape, &members, &problem.poses, 1, &mut b);
            let overflow = channel(
                &problem,
                &escape,
                0,
                problem.poses[0],
                &a,
                1,
                problem.poses[1],
                &b,
            );
            assert!(
                (overflow - over).abs() < 1.0e-9,
                "gap {gap}: {overflow} != {over}"
            );
            // Symmetric.
            let reverse = channel(
                &problem,
                &escape,
                1,
                problem.poses[1],
                &b,
                0,
                problem.poses[0],
                &a,
            );
            assert!((overflow - reverse).abs() < 1.0e-9);
        }
    }

    #[test]
    fn a_dense_side_facing_a_part_without_demand_or_across_sides_costs_nothing() {
        let (problem, escape) = facing_chips(0.5, 4);
        let members = members(&problem);
        let mut a = Vec::new();
        exits(&problem, &escape, &members, &problem.poses, 0, &mut a);
        // U2 without exits towards U1.
        let none: Vec<Exit> = Vec::new();
        assert_eq!(
            channel(
                &problem,
                &escape,
                0,
                problem.poses[0],
                &a,
                1,
                problem.poses[1],
                &none
            ),
            0.0
        );
        // Exits of U2 towards the other side only.
        let away = vec![Exit {
            side: 1,
            along: 0.0,
            target: 5,
        }];
        assert_eq!(
            channel(
                &problem,
                &escape,
                0,
                problem.poses[0],
                &a,
                1,
                problem.poses[1],
                &away
            ),
            0.0
        );
        // Nets between the two parts cross the gap: no channel demand.
        let between = vec![Exit {
            side: 0,
            along: 0.0,
            target: 0,
        }];
        assert_eq!(
            channel(
                &problem,
                &escape,
                0,
                problem.poses[0],
                &a,
                1,
                problem.poses[1],
                &between
            ),
            0.0
        );
        // A part on the other side of the board.
        let mut other_side = problem.clone();
        other_side.components[1].side = Side::Back;
        let mut b = Vec::new();
        exits(&problem, &escape, &members, &problem.poses, 1, &mut b);
        assert_eq!(
            channel(
                &other_side,
                &escape,
                0,
                problem.poses[0],
                &a,
                1,
                problem.poses[1],
                &b
            ),
            0.0
        );
    }

    #[test]
    fn only_the_exits_on_the_facing_stretch_count() {
        // U2 shifted up by 6 mm: only U1's exits above y = 1 - pitch/2
        // face it (pads at -4, -2, 0, 2: one), and U2's below its own
        // y = -5 + 6 = 1 (pads at 2, 4, 6, 8 in board terms: none face).
        let (mut problem, escape) = facing_chips(0.5, 4);
        problem.poses[1].position[1] = 6.0;
        let members = members(&problem);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        exits(&problem, &escape, &members, &problem.poses, 0, &mut a);
        exits(&problem, &escape, &members, &problem.poses, 1, &mut b);
        // U2's pads at board y 2, 4, 6, 8 on its -x side; the stretch is
        // y in [1 - 0.25, 5 + 0.25]: U2 has 2 there (2, 4), U1 one (2).
        // 3 tracks, capacity 0.5 / 0.25 = 2: 1 over.
        let overflow = channel(
            &problem,
            &escape,
            0,
            problem.poses[0],
            &a,
            1,
            problem.poses[1],
            &b,
        );
        assert!((overflow - 1.0).abs() < 1.0e-9, "{overflow}");
    }

    #[test]
    fn the_state_charges_a_move_into_a_full_channel() {
        let (problem, escape) = facing_chips(3.0, 4);
        let mut poses = problem.poses.clone();
        let mut state = EscapeState::new(&problem, &escape, &poses, |_| true);
        assert_eq!(state.cost(&poses, 1), 0.0);
        poses[1].position[0] = 10.5;
        // 8 tracks in 0.5 mm: 6 over.
        assert!((state.cost(&poses, 1) - 6.0).abs() < 1.0e-9);
        let inflation = inflation(&escape, &state.exits[0]);
        assert_eq!(inflation, [0.0, 1.0, 0.0, 0.0]);
    }
}

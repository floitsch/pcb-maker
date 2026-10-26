// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Bus groups (`Config::bus_groups`, docs/placer.md "Bus-group moves
//! (2026-10-26)"): the units a designer places as one, moved as one by the
//! anneal and the refinement.
//!
//! - A **bank**: parts of one footprint that share a bus (at least
//!   `min_shared` small nets between every two that are linked): a
//!   memory bank, the copies of a driver on one bus.
//! - A **cluster**: a part that is not a passive with the passives that
//!   share a net with it and lie within `reach` of its body (its
//!   decoupling, its filters, a connector's ESD parts).
//!
//! A bank moves with its members' passives too. Which passives belong to
//! which part is decided at the poses of the moment (`attach`): the
//! passives within reach, each to the nearest part it shares a net with.
//!
//! The moves (`translated`, `rotated`, `reordered`) give the new poses of
//! a group's parts: members keep their offsets (a turn turns the whole
//! group about its centre), and a reorder puts a bank's members along
//! their axis in the order of the pins they connect to outside the bank
//! (for a chain of parts on a bus, the order at the source part).

use crate::problem::{Point, Pose, Problem, Side, rotate};

/// The detection's settings and where the moves act.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Two parts of one footprint form a bank when they share at least
    /// this many bus nets.
    pub min_shared: usize,
    /// Nets of up to this many pins are bus nets (power nets and matrix
    /// rows are larger).
    pub bus_net_pins: usize,
    /// Passives: at most this many pins and a body of at most
    /// `passive_area` square millimetres.
    pub passive_pins: usize,
    pub passive_area: f64,
    /// A passive goes with a part when its body lies within this gap of
    /// the part's body (mm).
    pub reach: f64,
    /// After the global placement, each bank takes the order and the
    /// arrangement (in place, side by side along x or y) that lowers its
    /// nets' wirelength plus crossings, overlap left to the anneal
    /// (`arrange`).
    pub arrange: bool,
    /// Group moves in the anneal and in the refinement.
    pub anneal: bool,
    pub refine: bool,
    /// In the anneal a bank's members move only with their bank: their
    /// own moves (and swaps) are not drawn.
    pub rigid_banks: bool,
    /// Group moves the anneal tries per group with more than one part, in
    /// each stage.
    pub anneal_moves: f64,
    /// Banks larger than this are not banks (their parts are clusters).
    pub max_bank: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            min_shared: 4,
            bus_net_pins: 24,
            passive_pins: 4,
            passive_area: 16.0,
            reach: 3.0,
            arrange: true,
            anneal: true,
            refine: true,
            rigid_banks: true,
            anneal_moves: 2.0,
            max_bank: 16,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bank,
    Cluster,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub kind: Kind,
    /// The parts on the bus: a bank's members, a cluster's part alone.
    pub chain: Vec<usize>,
    /// Per chain part, the bus nets it has that not every chain part has:
    /// the nets whose far pins decide the order along the bank's axis.
    pub order_nets: Vec<Vec<usize>>,
    /// Per chain part, the pins (part, pin) of those nets on parts outside
    /// the chain.
    pub order_pins: Vec<Vec<(usize, usize)>>,
    /// Groups of the same kind on the same side whose chain bodies' area
    /// is within `SWAP_AREA` of this one's: the partners of a swap.
    pub peers: Vec<usize>,
}

/// Swaps trade groups whose chains' body area differs by at most this
/// factor.
pub const SWAP_AREA: f64 = 2.5;

#[derive(Clone, Debug)]
pub struct BusGroups {
    pub settings: Settings,
    pub groups: Vec<Group>,
    /// Per part: whether it is a passive (it goes with a part, never
    /// leads a group).
    pub passive: Vec<bool>,
    /// Per passive: the parts leading a cluster that a relation keeps it
    /// near (`bound`), or else that share a net with it.
    pub anchors_of: Vec<Vec<usize>>,
    /// Per passive: whether a relation binds it to its anchors (a
    /// decoupling capacitor): it goes with the nearest of them at any
    /// distance; another passive only within `Settings::reach`.
    pub bound: Vec<bool>,
    /// Per part: the cluster it leads.
    pub cluster_of: Vec<Option<usize>>,
}

impl BusGroups {
    /// The groups of `problem`'s movable parts.
    pub fn detect(problem: &Problem, settings: &Settings) -> BusGroups {
        let count = problem.components.len();
        let nets = problem.net_weights.len();
        let movable = |index: usize| {
            let component = &problem.components[index];
            !component.fixed && !component.pins.is_empty() && component.side != Side::Neither
        };
        let passive: Vec<bool> = (0..count)
            .map(|index| {
                let component = &problem.components[index];
                movable(index)
                    && component.pins.len() <= settings.passive_pins
                    && component.body_size[0] * component.body_size[1] <= settings.passive_area
            })
            .collect();
        let leads = |index: usize| {
            movable(index) && !passive[index] && problem.components[index].pins.len() >= 2
        };
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); nets];
        for (index, component) in problem.components.iter().enumerate() {
            for pin in &component.pins {
                members[pin.net].push(index);
            }
        }
        let counted = |net: usize| {
            problem
                .crossings
                .as_deref()
                .is_none_or(|crossings| crossings.counted.get(net).copied().unwrap_or(false))
        };
        let bus_net =
            |net: usize| (2..=settings.bus_net_pins).contains(&members[net].len()) && counted(net);
        // Per part its bus nets, each once.
        let bus_nets: Vec<Vec<usize>> = problem
            .components
            .iter()
            .map(|component| {
                let mut own: Vec<usize> = component
                    .pins
                    .iter()
                    .map(|pin| pin.net)
                    .filter(|net| bus_net(*net))
                    .collect();
                own.sort_unstable();
                own.dedup();
                own
            })
            .collect();
        // Bank links: parts of one footprint sharing `min_shared` bus nets.
        let same = |a: usize, b: usize| {
            let (first, second) = (&problem.components[a], &problem.components[b]);
            first.pins.len() == second.pins.len()
                && first.side == second.side
                && first.round == second.round
                && (first.body_size[0] - second.body_size[0]).abs() <= 1.0e-6
                && (first.body_size[1] - second.body_size[1]).abs() <= 1.0e-6
        };
        let leaders: Vec<usize> = (0..count)
            .filter(|index| {
                leads(*index) && problem.components[*index].pins.len() > settings.passive_pins
            })
            .collect();
        let mut parent: Vec<usize> = (0..count).collect();
        fn root(parent: &mut [usize], mut index: usize) -> usize {
            while parent[index] != index {
                parent[index] = parent[parent[index]];
                index = parent[index];
            }
            index
        }
        for (position, &a) in leaders.iter().enumerate() {
            for &b in &leaders[position + 1..] {
                if !same(a, b) {
                    continue;
                }
                let shared = sorted_common(&bus_nets[a], &bus_nets[b]);
                if shared >= settings.min_shared {
                    let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                    if ra != rb {
                        parent[ra.max(rb)] = ra.min(rb);
                    }
                }
            }
        }
        let mut banks: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
        for &part in &leaders {
            let top = root(&mut parent, part);
            banks.entry(top).or_default().push(part);
        }
        let mut groups = Vec::new();
        for (_, chain) in banks {
            if chain.len() < 2 || chain.len() > settings.max_bank {
                continue;
            }
            let order_nets = chain
                .iter()
                .map(|part| {
                    bus_nets[*part]
                        .iter()
                        .copied()
                        .filter(|net| {
                            !chain
                                .iter()
                                .all(|other| bus_nets[*other].binary_search(net).is_ok())
                        })
                        .collect()
                })
                .collect::<Vec<Vec<usize>>>();
            let order_pins = order_nets
                .iter()
                .map(|nets: &Vec<usize>| {
                    let mut pins = Vec::new();
                    for (index, component) in problem.components.iter().enumerate() {
                        if chain.contains(&index) {
                            continue;
                        }
                        for (pin, description) in component.pins.iter().enumerate() {
                            if nets.binary_search(&description.net).is_ok() {
                                pins.push((index, pin));
                            }
                        }
                    }
                    pins
                })
                .collect();
            groups.push(Group {
                kind: Kind::Bank,
                chain,
                order_nets,
                order_pins,
                peers: Vec::new(),
            });
        }
        // Every part that may lead leads a cluster.
        let mut cluster_of = vec![None; count];
        for index in 0..count {
            if leads(index) {
                cluster_of[index] = Some(groups.len());
                groups.push(Group {
                    kind: Kind::Cluster,
                    chain: vec![index],
                    order_nets: vec![Vec::new()],
                    order_pins: vec![Vec::new()],
                    peers: Vec::new(),
                });
            }
        }
        // The passives' candidate parts: the parts a relation keeps them
        // near (a decoupling capacitor's ICs), else those sharing a net
        // with them.
        let mut related: Vec<Vec<usize>> = vec![Vec::new(); count];
        for relation in &problem.constraints.relations {
            let (part, anchors): (usize, Vec<usize>) = match relation {
                crate::constraints::Relation::Near { part, anchor, .. } => {
                    (*part, vec![anchor.part()])
                }
                crate::constraints::Relation::NearAny { part, anchors, .. } => {
                    (*part, anchors.iter().map(|anchor| anchor.part()).collect())
                }
                _ => continue,
            };
            if part < count && passive[part] {
                related[part].extend(
                    anchors
                        .into_iter()
                        .filter(|anchor| *anchor != part && cluster_of[*anchor].is_some()),
                );
            }
        }
        let mut anchors_of = vec![Vec::new(); count];
        let mut bound = vec![false; count];
        for index in 0..count {
            if !passive[index] {
                continue;
            }
            if !related[index].is_empty() {
                let mut anchors = std::mem::take(&mut related[index]);
                anchors.sort_unstable();
                anchors.dedup();
                anchors_of[index] = anchors;
                bound[index] = true;
                continue;
            }
            let mut anchors: Vec<usize> = problem.components[index]
                .pins
                .iter()
                .flat_map(|pin| members[pin.net].iter().copied())
                .filter(|other| *other != index && cluster_of[*other].is_some())
                .collect();
            anchors.sort_unstable();
            anchors.dedup();
            anchors_of[index] = anchors;
        }
        // Swap partners.
        let area = |group: &Group| -> f64 {
            group
                .chain
                .iter()
                .map(|part| {
                    let size = problem.components[*part].body_size;
                    size[0] * size[1]
                })
                .sum()
        };
        let areas: Vec<f64> = groups.iter().map(area).collect();
        for a in 0..groups.len() {
            for b in a + 1..groups.len() {
                let (first, second) = (&groups[a], &groups[b]);
                if first.kind != second.kind
                    || problem.components[first.chain[0]].side
                        != problem.components[second.chain[0]].side
                {
                    continue;
                }
                let (low, high) = (areas[a].min(areas[b]), areas[a].max(areas[b]));
                if low > 0.0 && high <= SWAP_AREA * low {
                    groups[a].peers.push(b);
                    groups[b].peers.push(a);
                }
            }
        }
        BusGroups {
            settings: settings.clone(),
            groups,
            passive,
            anchors_of,
            bound,
            cluster_of,
        }
    }

    /// Per group, the parts it moves at `poses`: its chain, the passives a
    /// relation binds to a chain part (each with the nearest of its
    /// anchors), and the other passives within reach of a chain part (each
    /// with the nearest part it shares a net with), sorted.
    pub fn attach(&self, problem: &Problem, poses: &[Pose]) -> Vec<Vec<usize>> {
        let count = problem.components.len();
        let mut attached: Vec<Vec<usize>> = vec![Vec::new(); count];
        let mut work = 0u64;
        for index in 0..count {
            if !self.passive[index] || self.anchors_of[index].is_empty() {
                continue;
            }
            let (center, half) = body_box(problem, index, poses[index]);
            let mut best: Option<(f64, usize)> = None;
            for &anchor in &self.anchors_of[index] {
                work += 1;
                let (other_center, other_half) = body_box(problem, anchor, poses[anchor]);
                let gap_x = (center[0] - other_center[0]).abs() - half[0] - other_half[0];
                let gap_y = (center[1] - other_center[1]).abs() - half[1] - other_half[1];
                let gap = gap_x.max(gap_y).max(0.0);
                if (self.bound[index] || gap <= self.settings.reach)
                    && best.is_none_or(|(least, _)| gap < least)
                {
                    best = Some((gap, anchor));
                }
            }
            if let Some((_, anchor)) = best {
                attached[anchor].push(index);
            }
        }
        crate::add_work(work * 4);
        self.groups
            .iter()
            .map(|group| {
                let mut parts = group.chain.clone();
                for part in &group.chain {
                    parts.extend(attached[*part].iter().copied());
                }
                parts.sort_unstable();
                parts.dedup();
                parts
            })
            .collect()
    }

    /// For the report: per group with more than one part at `poses`, its
    /// kind, its chain and how many parts it moves.
    pub fn summary(&self, problem: &Problem, poses: &[Pose]) -> Vec<(Kind, Vec<usize>, usize)> {
        let parts = self.attach(problem, poses);
        self.groups
            .iter()
            .zip(parts)
            .filter(|(_, parts)| parts.len() > 1)
            .map(|(group, parts)| (group.kind, group.chain.clone(), parts.len()))
            .collect()
    }
}

/// How many entries two sorted lists share.
fn sorted_common(a: &[usize], b: &[usize]) -> usize {
    let (mut i, mut j, mut shared) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    shared
}

/// A part's body as (centre, half extent) at `pose`.
pub fn body_box(problem: &Problem, index: usize, pose: Pose) -> (Point, Point) {
    let component = &problem.components[index];
    (component.center(pose), component.half_extent(pose.angle))
}

/// The box around the bodies of `parts` at `poses`: [min x, min y, max x,
/// max y].
pub fn extent(problem: &Problem, poses: &[Pose], parts: &[usize]) -> [f64; 4] {
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for &part in parts {
        let (center, half) = body_box(problem, part, poses[part]);
        bounds[0] = bounds[0].min(center[0] - half[0]);
        bounds[1] = bounds[1].min(center[1] - half[1]);
        bounds[2] = bounds[2].max(center[0] + half[0]);
        bounds[3] = bounds[3].max(center[1] + half[1]);
    }
    bounds
}

/// The centre of the box around the chain's bodies.
pub fn chain_center(problem: &Problem, poses: &[Pose], chain: &[usize]) -> Point {
    let bounds = extent(problem, poses, chain);
    [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0]
}

fn snapped(problem: &Problem, part: usize, pose: Pose) -> Pose {
    Pose {
        position: crate::constraints::snap_position(problem, part, pose.position, pose.angle),
        angle: pose.angle,
    }
}

/// `parts` moved by `shift`, snapped.
pub fn translated(
    problem: &Problem,
    poses: &[Pose],
    parts: &[usize],
    shift: Point,
) -> Vec<(usize, Pose)> {
    parts
        .iter()
        .map(|&part| {
            let pose = poses[part];
            (
                part,
                snapped(
                    problem,
                    part,
                    Pose {
                        position: [pose.position[0] + shift[0], pose.position[1] + shift[1]],
                        angle: pose.angle,
                    },
                ),
            )
        })
        .collect()
}

/// `parts` turned by `turn` degrees (a quarter turn or a half) about
/// `center`, snapped; `None` when a part may not take its new angle.
pub fn rotated(
    problem: &Problem,
    poses: &[Pose],
    parts: &[usize],
    center: Point,
    turn: f64,
) -> Option<Vec<(usize, Pose)>> {
    parts
        .iter()
        .map(|&part| {
            let pose = poses[part];
            let angle = (pose.angle - turn).rem_euclid(360.0);
            let option = problem.components[part]
                .angle_options
                .iter()
                .copied()
                .find(|option| {
                    let difference = (option - angle).rem_euclid(360.0);
                    difference < 1.0e-6 || difference > 360.0 - 1.0e-6
                })?;
            let relative = rotate(
                [pose.position[0] - center[0], pose.position[1] - center[1]],
                turn,
            );
            Some((
                part,
                snapped(
                    problem,
                    part,
                    Pose {
                        position: [center[0] + relative[0], center[1] + relative[1]],
                        angle: option,
                    },
                ),
            ))
        })
        .collect()
}

/// How a bank's members are put in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reorder {
    /// Into the places the members hold now, sorted along the axis on
    /// which they spread most.
    InPlace,
    /// Side by side along x or y (0 or 1), centred where they are.
    Along(usize),
}

/// A bank's members (`group`, each with the passives that go with it in
/// `parts`) in the order of the pins they connect to outside the group,
/// along the axis: each member and its passives shifted together.
/// `margin(part)` is how far a part keeps from its neighbours beyond its
/// body (its halo and half the spacing). `None` when the group is not a
/// bank or the order and places do not change.
pub fn reordered(
    problem: &Problem,
    poses: &[Pose],
    group: &Group,
    parts: &[usize],
    how: Reorder,
    margin: &dyn Fn(usize) -> f64,
) -> Option<Vec<(usize, Pose)>> {
    if group.kind != Kind::Bank || group.chain.len() < 2 {
        return None;
    }
    // Each passive with its chain member: the nearest one.
    let chain = &group.chain;
    let centers: Vec<Point> = chain
        .iter()
        .map(|part| problem.components[*part].center(poses[*part]))
        .collect();
    let mut units: Vec<Vec<usize>> = chain.iter().map(|part| vec![*part]).collect();
    for &part in parts {
        if chain.contains(&part) {
            continue;
        }
        let at = problem.components[part].center(poses[part]);
        let nearest = (0..chain.len())
            .min_by(|a, b| {
                let distance = |index: usize| {
                    (centers[index][0] - at[0]).powi(2) + (centers[index][1] - at[1]).powi(2)
                };
                distance(*a).total_cmp(&distance(*b))
            })
            .expect("a bank has members");
        units[nearest].push(part);
    }
    let spread = |axis: usize| {
        let low = centers
            .iter()
            .map(|c| c[axis])
            .fold(f64::INFINITY, f64::min);
        let high = centers
            .iter()
            .map(|c| c[axis])
            .fold(f64::NEG_INFINITY, f64::max);
        high - low
    };
    let axis = match how {
        Reorder::InPlace => usize::from(spread(1) > spread(0)),
        Reorder::Along(axis) => axis,
    };
    // The order: by the mean coordinate along the axis of the pins on each
    // member's own bus nets that lie outside the group.
    let inside = |part: usize| parts.binary_search(&part).is_ok();
    let keys: Vec<f64> = chain
        .iter()
        .enumerate()
        .map(|(position, _)| {
            let mut sum = 0.0;
            let mut count = 0usize;
            for &(index, pin) in &group.order_pins[position] {
                if inside(index) {
                    continue;
                }
                let component = &problem.components[index];
                sum += component.pin_position(&component.pins[pin], poses[index])[axis];
                count += 1;
            }
            crate::add_work(crate::PIN_WORK * group.order_pins[position].len() as u64);
            if count == 0 {
                centers[position][axis]
            } else {
                sum / count as f64
            }
        })
        .collect();
    let mut order: Vec<usize> = (0..chain.len()).collect();
    order.sort_by(|a, b| keys[*a].total_cmp(&keys[*b]).then(a.cmp(b)));
    // The members' new centres, in that order.
    let targets: Vec<Point> = match how {
        Reorder::InPlace => {
            let mut slots: Vec<Point> = centers.clone();
            slots.sort_by(|a, b| a[axis].total_cmp(&b[axis]));
            let mut targets = vec![[0.0; 2]; chain.len()];
            for (slot, member) in order.iter().enumerate() {
                targets[*member] = slots[slot];
            }
            targets
        }
        Reorder::Along(axis) => {
            // Each unit's extent along the axis around its member's centre.
            let reach: Vec<(f64, f64)> = units
                .iter()
                .enumerate()
                .map(|(position, unit)| {
                    let mut low: f64 = 0.0;
                    let mut high: f64 = 0.0;
                    for &part in unit {
                        let (center, half) = body_box(problem, part, poses[part]);
                        let extra = margin(part);
                        low =
                            low.max(centers[position][axis] - (center[axis] - half[axis] - extra));
                        high =
                            high.max(center[axis] + half[axis] + extra - centers[position][axis]);
                    }
                    (low, high)
                })
                .collect();
            let length: f64 = reach.iter().map(|(low, high)| low + high).sum();
            let middle = centers.iter().map(|c| c[axis]).sum::<f64>() / chain.len() as f64;
            let across = 1 - axis;
            let line = centers.iter().map(|c| c[across]).sum::<f64>() / chain.len() as f64;
            let mut at = middle - length / 2.0;
            let mut targets = vec![[0.0; 2]; chain.len()];
            for &member in &order {
                let (low, high) = reach[member];
                let mut target = [0.0; 2];
                target[axis] = at + low;
                target[across] = line;
                targets[member] = target;
                at += low + high;
            }
            targets
        }
    };
    let mut moved = Vec::new();
    let mut changed = false;
    for (position, unit) in units.iter().enumerate() {
        let shift = [
            targets[position][0] - centers[position][0],
            targets[position][1] - centers[position][1],
        ];
        if shift[0].abs() > 1.0e-9 || shift[1].abs() > 1.0e-9 {
            changed = true;
        }
        moved.extend(translated(problem, poses, unit, shift));
    }
    changed.then_some(moved)
}

/// The relations involving any of `parts` (each once) and the parts'
/// hard constraints, priced as the anneal prices them (the relation
/// weight, 1000 per millimetre outside an edge or region). `parts` sorted.
pub fn held_cost(problem: &Problem, poses: &[Pose], parts: &[usize]) -> f64 {
    let constraints = &problem.constraints;
    if constraints.is_empty() {
        return 0.0;
    }
    let mut work = constraints.relations.len() as u64;
    let mut relations = 0.0;
    for relation in &constraints.relations {
        let involved = match relation {
            crate::constraints::Relation::NearAny { part, anchors, .. } => {
                work += anchors.len() as u64 / 4;
                parts.binary_search(part).is_ok()
                    || anchors
                        .iter()
                        .any(|anchor| parts.binary_search(&anchor.part()).is_ok())
            }
            other => other
                .parts()
                .iter()
                .flatten()
                .any(|part| parts.binary_search(part).is_ok()),
        };
        if involved {
            work += 8;
            relations += crate::constraints::relation_violation(problem, poses, relation);
        }
    }
    relations *= constraints.relation_weight;
    let hard: f64 = parts
        .iter()
        .map(|part| crate::constraints::hard_violation(problem, *part, poses[*part]))
        .sum();
    crate::add_work(work + 4 * parts.len() as u64);
    relations + 1000.0 * hard
}

/// What `arrange` did.
#[derive(Clone, Debug, Default)]
pub struct Arranged {
    pub banks: usize,
    pub taken: usize,
    pub cost_before: f64,
    pub cost_after: f64,
}

/// After the global placement (`Settings::arrange`): each bank tries the
/// order of its members in their places and side by side along x and
/// along y (`reordered`, each member with its passives), and takes the
/// one that lowers the weighted half-perimeter wirelength of the moved
/// parts' nets plus the crossings' weight times their crossings
/// (`Problem::crossings`) plus their relations and hard constraints (as
/// the anneal prices them). Overlap is the anneal's to resolve, as it is
/// of the global placement's: the anneal's group moves judge with it and
/// cannot gather a scattered bank over the parts in its way.
pub fn arrange(problem: &Problem, groups: &BusGroups, poses: &mut [Pose]) -> Arranged {
    let mut result = Arranged::default();
    let banks: Vec<usize> = (0..groups.groups.len())
        .filter(|index| groups.groups[*index].kind == Kind::Bank)
        .collect();
    result.banks = banks.len();
    if banks.is_empty() {
        return result;
    }
    let nets = problem.net_weights.len();
    let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nets];
    for (index, component) in problem.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            members[description.net].push((index, pin));
        }
    }
    let wirelength = |poses: &[Pose], nets: &[usize]| -> f64 {
        crate::add_work(
            crate::PIN_WORK * nets.iter().map(|net| members[*net].len()).sum::<usize>() as u64,
        );
        nets.iter()
            .filter(|net| members[**net].len() >= 2)
            .map(|net| {
                let mut bounds = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for (part, pin) in &members[*net] {
                    let component = &problem.components[*part];
                    let at = component.pin_position(&component.pins[*pin], poses[*part]);
                    bounds[0] = bounds[0].min(at[0]);
                    bounds[1] = bounds[1].min(at[1]);
                    bounds[2] = bounds[2].max(at[0]);
                    bounds[3] = bounds[3].max(at[1]);
                }
                problem.net_weights[*net] * ((bounds[2] - bounds[0]) + (bounds[3] - bounds[1]))
            })
            .sum()
    };
    let mut crossings = problem
        .crossings
        .as_deref()
        .map(|fact| crate::crossings::CrossingState::new(problem, fact, poses));
    let margin = |part: usize| problem.components[part].halo + problem.spacing / 2.0;
    let board = problem.bounds();
    for bank in banks {
        let group = &groups.groups[bank];
        let parts = &groups.attach(problem, poses)[bank];
        let mut nets: Vec<usize> = parts
            .iter()
            .flat_map(|part| problem.components[*part].pins.iter().map(|pin| pin.net))
            .collect();
        nets.sort_unstable();
        nets.dedup();
        let length_before = wirelength(poses, &nets);
        let mut best: Option<(f64, Vec<(usize, Pose)>, i64)> = None;
        let mut first = true;
        for how in [Reorder::InPlace, Reorder::Along(0), Reorder::Along(1)] {
            let Some(moved) = reordered(problem, poses, group, parts, how, &margin) else {
                continue;
            };
            let list: Vec<usize> = moved.iter().map(|(part, _)| *part).collect();
            // Not beyond the board: the anneal would have to fold it back.
            let mut after = poses.to_vec();
            for (part, pose) in &moved {
                after[*part] = *pose;
            }
            let reach = extent(problem, &after, &group.chain);
            if reach[0] < board[0]
                || reach[1] < board[1]
                || reach[2] > board[2]
                || reach[3] > board[3]
            {
                continue;
            }
            let old: Vec<(usize, Pose)> = list.iter().map(|part| (*part, poses[*part])).collect();
            let held_before = held_cost(problem, poses, &list);
            let crossings_before = crossings.as_mut().map_or(0, |state| state.before(&list));
            for (part, pose) in &moved {
                poses[*part] = *pose;
            }
            let held_after = held_cost(problem, poses, &list);
            let change = crossings.as_mut().map_or(0, |state| {
                state.after(poses) as i64 - crossings_before as i64
            });
            let weight = crossings.as_ref().map_or(0.0, |state| state.weight());
            let cost =
                wirelength(poses, &nets) - length_before + weight * change as f64 + held_after
                    - held_before;
            if std::env::var_os("PCB_GROUPS_TRACE").is_some() {
                eprintln!(
                    "groups trace: arrange {how:?}: {} parts, held {held_before:.1} -> {held_after:.1}, crossings {crossings_before} {change:+}, cost {cost:.0}",
                    list.len()
                );
            }
            if let Some(state) = crossings.as_mut() {
                // Counted, never kept: the best is kept below.
                state.settle(poses, &list, false, 0);
            }
            if first {
                result.cost_before += length_before;
                first = false;
            }
            if cost < best.as_ref().map_or(-1.0e-9, |(least, _, _)| *least) {
                best = Some((cost, moved, change));
            }
            for (part, pose) in old {
                poses[part] = pose;
            }
        }
        if let Some((cost, moved, _)) = best {
            let list: Vec<usize> = moved.iter().map(|(part, _)| *part).collect();
            for (part, pose) in &moved {
                poses[*part] = *pose;
            }
            if let Some(state) = crossings.as_mut() {
                state.kept(poses, &list);
            }
            result.taken += 1;
            result.cost_after += length_before + cost;
        } else {
            result.cost_after += length_before;
        }
    }
    result
}

/// Where the nets of `parts` pull them: the shift that brings the
/// weighted mean of their pins on nets that leave the group to the
/// weighted mean of those nets' pins outside it (the refinement's pull,
/// for a group). `None` when no net leaves the group.
pub fn pull(problem: &Problem, poses: &[Pose], parts: &[usize]) -> Option<Point> {
    let inside = |part: usize| parts.binary_search(&part).is_ok();
    let nets = problem.net_weights.len();
    let mut pins: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nets];
    let mut work = 0u64;
    for (index, component) in problem.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            pins[description.net].push((index, pin));
        }
    }
    let mut seen = vec![false; nets];
    let (mut own, mut far) = ([0.0; 2], [0.0; 2]);
    let (mut own_weight, mut far_weight) = (0.0, 0.0);
    for &part in parts {
        for description in &problem.components[part].pins {
            let net = description.net;
            if seen[net] {
                continue;
            }
            seen[net] = true;
            let members = &pins[net];
            work += members.len() as u64;
            let outside = members.iter().filter(|(other, _)| !inside(*other)).count();
            if outside == 0 || outside == members.len() {
                continue;
            }
            let weight = problem.net_weights[net] / (members.len() - 1) as f64;
            for (other, pin) in members {
                let component = &problem.components[*other];
                let at = component.pin_position(&component.pins[*pin], poses[*other]);
                if inside(*other) {
                    own[0] += weight * at[0];
                    own[1] += weight * at[1];
                    own_weight += weight;
                } else {
                    far[0] += weight * at[0];
                    far[1] += weight * at[1];
                    far_weight += weight;
                }
            }
        }
    }
    crate::add_work(crate::PIN_WORK * work + problem.components.len() as u64);
    if own_weight == 0.0 || far_weight == 0.0 {
        return None;
    }
    Some([
        far[0] / far_weight - own[0] / own_weight,
        far[1] / far_weight - own[1] / own_weight,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::{Component, Pin};

    fn part(name: &str, size: Point, nets: &[usize]) -> Component {
        Component {
            name: name.into(),
            body_center: [0.0, 0.0],
            body_size: size,
            round: false,
            halo: 0.0,
            pins: nets
                .iter()
                .enumerate()
                .map(|(index, net)| Pin {
                    offset: [index as f64 * 0.5 - size[0] / 2.0 + 0.25, 0.0],
                    net: *net,
                })
                .collect(),
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
        }
    }

    fn pose(x: f64, y: f64) -> Pose {
        Pose {
            position: [x, y],
            angle: 0.0,
        }
    }

    /// A source with a bus of four shared nets to three memories, each
    /// with its own select net (nets 4, 5, 6) back to the source, and a
    /// decoupling capacitor beside each memory.
    fn bank() -> Problem {
        let mut components = vec![{
            let mut source = part("U1", [10.0, 10.0], &[0, 1, 2, 3, 4, 5, 6]);
            source.fixed = true;
            source
        }];
        for (index, select) in [4usize, 5, 6].iter().enumerate() {
            components.push(part(
                &format!("M{index}"),
                [6.0, 4.0],
                &[0, 1, 2, 3, *select, 7],
            ));
        }
        for index in 0..3 {
            components.push(part(&format!("C{index}"), [1.6, 0.8], &[7, 8]));
        }
        let poses = vec![
            pose(0.0, 0.0),
            pose(30.0, 10.0),
            pose(30.0, 20.0),
            pose(30.0, 0.0),
            pose(30.0, 13.0),
            pose(30.0, 23.0),
            pose(40.0, 40.0),
        ];
        Problem {
            outline: vec![[-20.0, -20.0], [60.0, -20.0], [60.0, 60.0], [-20.0, 60.0]],
            net_weights: vec![1.0; 9],
            components,
            poses,
            spacing: 0.5,
            grid: 0.0,
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

    #[test]
    fn detects_a_bank_and_clusters_with_their_passives() {
        let problem = bank();
        let groups = BusGroups::detect(&problem, &Settings::default());
        let banks: Vec<&Group> = groups
            .groups
            .iter()
            .filter(|group| group.kind == Kind::Bank)
            .collect();
        assert_eq!(banks.len(), 1);
        assert_eq!(banks[0].chain, vec![1, 2, 3]);
        // Each member's own net is its order net.
        assert_eq!(banks[0].order_nets, vec![vec![4], vec![5], vec![6]]);
        // Three clusters (the fixed source leads none).
        assert_eq!(groups.groups.len(), 4);
        let parts = groups.attach(&problem, &problem.poses);
        // C0 and C1 lie within reach of M0 and M1; C2 is far from all.
        let cluster = |part: usize| groups.cluster_of[part].unwrap();
        assert_eq!(parts[cluster(1)], vec![1, 4]);
        assert_eq!(parts[cluster(2)], vec![2, 5]);
        assert_eq!(parts[cluster(3)], vec![3]);
        let bank = groups
            .groups
            .iter()
            .position(|g| g.kind == Kind::Bank)
            .unwrap();
        assert_eq!(parts[bank], vec![1, 2, 3, 4, 5]);
        // The clusters of identical memories are each other's peers.
        assert!(groups.groups[cluster(1)].peers.contains(&cluster(2)));
    }

    #[test]
    fn a_reorder_follows_the_source_pins() {
        let mut problem = bank();
        // The source's select pins along y: net 4 at the top (y 4), net 6
        // at the bottom (y -4).
        for (pin, y) in [(4usize, 4.0), (5, 0.0), (6, -4.0)] {
            problem.components[0].pins[pin].offset = [5.0, y];
        }
        let groups = BusGroups::detect(&problem, &Settings::default());
        let bank = groups
            .groups
            .iter()
            .find(|group| group.kind == Kind::Bank)
            .unwrap();
        let parts = vec![1, 2, 3, 4, 5];
        let moved = reordered(
            &problem,
            &problem.poses,
            bank,
            &parts,
            Reorder::InPlace,
            &|_| 0.25,
        )
        .expect("the order changes");
        let at = |part: usize| moved.iter().find(|(index, _)| *index == part).unwrap().1;
        // M2 (net 6, lowest pin) takes the lowest place, M0 the highest.
        assert_eq!(at(3).position, [30.0, 0.0]);
        assert_eq!(at(2).position, [30.0, 10.0]);
        assert_eq!(at(1).position, [30.0, 20.0]);
        // The capacitors go with their members.
        assert_eq!(at(4).position, [30.0, 23.0]);
        assert_eq!(at(5).position, [30.0, 13.0]);
        // Side by side along y: touching, by the order, centred.
        let along = reordered(
            &problem,
            &problem.poses,
            bank,
            &parts,
            Reorder::Along(1),
            &|_| 0.25,
        )
        .unwrap();
        let at = |part: usize| along.iter().find(|(index, _)| *index == part).unwrap().1;
        assert!(at(3).position[1] < at(2).position[1]);
        assert!(at(2).position[1] < at(1).position[1]);
    }

    #[test]
    fn a_turn_turns_the_group_about_its_centre() {
        let problem = bank();
        let turned = rotated(&problem, &problem.poses, &[1, 4], [30.0, 10.0], 90.0).unwrap();
        // The capacitor 3 mm above M0 ends 3 mm beside it, and both turn.
        let local = [0.0, 3.0];
        let expected = rotate(local, 90.0);
        assert!((turned[1].1.position[0] - (30.0 + expected[0])).abs() < 1.0e-9);
        assert!((turned[1].1.position[1] - (10.0 + expected[1])).abs() < 1.0e-9);
        assert_eq!(turned[0].1.angle, 270.0);
        // Pins keep their offsets to each other: the pin of M0 and the
        // capacitor's first pin as before, turned.
        let pin = |part: usize, pose: Pose| {
            let component = &problem.components[part];
            component.pin_position(&component.pins[0], pose)
        };
        let before = [
            pin(4, problem.poses[4])[0] - pin(1, problem.poses[1])[0],
            pin(4, problem.poses[4])[1] - pin(1, problem.poses[1])[1],
        ];
        let after = [
            pin(4, turned[1].1)[0] - pin(1, turned[0].1)[0],
            pin(4, turned[1].1)[1] - pin(1, turned[0].1)[1],
        ];
        let expected = rotate(before, 90.0);
        assert!((after[0] - expected[0]).abs() < 1.0e-9 && (after[1] - expected[1]).abs() < 1.0e-9);
    }

    #[test]
    fn a_unit_is_placed_beside_what_blocks_it_keeping_its_offsets() {
        let mut problem = bank();
        problem.grid = 0.5;
        // M0 and C0 (3 mm above it) wanted with M0 right on M2 (at 30, 0).
        let wanted = vec![(1, pose(30.0, 1.0)), (4, pose(30.0, 4.0))];
        let placed = crate::legal::place_unit(&problem, &problem.poses, &wanted, 12).unwrap();
        let at = |part: usize| placed.iter().find(|(index, _)| *index == part).unwrap().1;
        let (m0, c0) = (at(1), at(4));
        assert_eq!(c0.position[1] - m0.position[1], 3.0);
        assert_eq!(c0.position[0], m0.position[0]);
        // M0 clears M2's body (4 mm tall) with the spacing.
        assert!(m0.position[1] >= 4.5 - 1.0e-9 || m0.position[0] >= 36.5 - 1.0e-9);
        for (part, pose) in &placed {
            let mut poses = problem.poses.clone();
            for (other, other_pose) in &placed {
                poses[*other] = *other_pose;
            }
            assert!(crate::legal::is_legal(
                &problem,
                &poses,
                *part,
                *pose,
                0..problem.components.len()
            ));
        }
    }

    #[test]
    fn arrange_untwists_a_bank_by_its_crossings() {
        let mut problem = bank();
        for (pin, y) in [(4usize, 4.0), (5, 0.0), (6, -4.0)] {
            problem.components[0].pins[pin].offset = [5.0, y];
        }
        // M0 (its select pin on the source at the top) lowest, M2 highest:
        // the select lines cross.
        problem.poses[1] = pose(30.0, 0.0);
        problem.poses[3] = pose(30.0, 20.0);
        problem.poses[4] = pose(30.0, 3.0);
        problem.crossings = Some(std::sync::Arc::new(crate::crossings::Crossings {
            counted: vec![true; 9],
            weight: 3.0,
            legalise: true,
            refine: true,
            anneal: false,
            global: false,
        }));
        let fact = problem.crossings.clone().unwrap();
        let before = crate::crossings::total(&problem, &fact, &problem.poses);
        let groups = BusGroups::detect(&problem, &Settings::default());
        let mut poses = problem.poses.clone();
        let arranged = arrange(&problem, &groups, &mut poses);
        assert_eq!((arranged.banks, arranged.taken), (1, 1));
        let after = crate::crossings::total(&problem, &fact, &poses);
        assert!(after < before, "crossings {before} -> {after}");
        assert!(after <= before / 2, "crossings {before} -> {after}");
        // C0 went with M0.
        let at = |part: usize| problem.components[part].center(poses[part]);
        assert!((at(4)[1] - at(1)[1] - 3.0).abs() < 1.0e-9);
        assert!((at(4)[0] - at(1)[0]).abs() < 1.0e-9);
    }
}

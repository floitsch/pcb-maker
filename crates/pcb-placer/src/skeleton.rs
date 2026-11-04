// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The race over skeletons (docs/placer.md, "A race over skeletons
//! (2026-11-04)"): a seed's placement in two phases. First the skeleton,
//! every part but the free passives, is placed alone: the passives are
//! absent (no pins, no body, no constraint of theirs) and each skeleton
//! part's body grows by the area of the passives it owns, so that the
//! skeleton leaves them their room; a net that ran through a two-pin
//! passive on small signal nets is one net in the skeleton (a series
//! resistor keeps its two ends' parts together). Then the skeleton is held
//! (fixed at its poses) and the passives are placed around it, from the
//! skeleton's relaxation level on. The layout's race ranks the seeds'
//! placements as it ranks any.

use crate::constraints::{Anchor, Relation};
use crate::problem::{Problem, Side};
use crate::{Config, Placement, legal, place};

/// The race's settings (`Config::skeleton` with the passives named).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Factor on the area of the passives a skeleton part owns (their tight
    /// bodies, else their bodies, with the least spacing) that its body
    /// grows by while the skeleton is placed alone.
    pub room: f64,
    /// Share of the placement's work limit the skeleton's placement may
    /// take; the passives' placement takes what is left.
    pub work_share: f64,
    /// Two nets a two-pin passive joins are one net in the skeleton where
    /// both have at most this many pins and no pour carries either (0:
    /// none joined).
    pub join_pins: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            room: 1.0,
            work_share: 0.5,
            join_pins: 8,
        }
    }
}

/// What the race over skeletons did for one seed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// The movable parts placed in the skeleton.
    pub skeleton_parts: usize,
    /// Of them, those held while the passives were placed (all, or none
    /// where the skeleton left parts without a place and the seed was
    /// placed whole).
    pub held: usize,
    /// The passives left out of the skeleton.
    pub absent: usize,
    /// Nets joined into others through a passive.
    pub joined_nets: usize,
    /// Skeleton parts whose bodies grew, and the area they grew by, mm².
    pub grown: usize,
    pub room_mm2: f64,
    /// The passives' area no skeleton part could take (no net to one, or
    /// only to parts whose bodies stay as they are), mm².
    pub unowned_mm2: f64,
    /// The work of the skeleton's placement, of the passives' and of the
    /// fallback (a full placement where the passives found no room).
    pub skeleton_work: u64,
    pub passive_work: u64,
    pub fallback_work: u64,
    /// The relaxation levels the skeleton and the passives ended at.
    pub skeleton_level: usize,
    pub level: usize,
    /// The skeleton's wirelength (its own nets) when placed.
    pub skeleton_wirelength: f64,
    /// The skeleton or the passives around it left parts without a legal
    /// place and the seed was placed whole with the work left (where the
    /// passives did, the better of the two kept: `fallback_kept`).
    pub fallback: bool,
    pub fallback_kept: bool,
}

/// The passives the skeleton leaves out: `passive` parts that move, have
/// pins and that no constraint names but the tool's own decoupling
/// relations (a user's row, edge, region or near keeps the part in the
/// skeleton, where its constraint holds).
pub fn absent_parts(problem: &Problem, passive: &[bool]) -> Vec<bool> {
    let count = problem.components.len();
    let mut named = vec![false; count];
    let constraints = &problem.constraints;
    let mut name = |index: usize| {
        if index < count {
            named[index] = true;
        }
    };
    for (part, _, _) in &constraints.edges {
        name(*part);
    }
    for (part, _) in &constraints.regions {
        name(*part);
    }
    for (part, _, _) in &constraints.overhangs {
        name(*part);
    }
    for follower in &constraints.followers {
        name(follower.part);
        name(follower.leader);
    }
    for (part, _) in &constraints.starts {
        name(*part);
    }
    for (part, _) in &constraints.frames.passes {
        name(*part);
    }
    let users = constraints
        .relations
        .len()
        .saturating_sub(constraints.automatic);
    for relation in &constraints.relations[..users] {
        for part in relation_parts(relation) {
            name(part);
        }
    }
    (0..count)
        .map(|index| {
            let component = &problem.components[index];
            passive.get(index).copied().unwrap_or(false)
                && !component.fixed
                && !component.pins.is_empty()
                && !named[index]
        })
        .collect()
}

/// Every part a relation involves, anchors included.
fn relation_parts(relation: &Relation) -> Vec<usize> {
    match relation {
        Relation::NearAny { part, anchors, .. } => std::iter::once(*part)
            .chain(anchors.iter().map(Anchor::part))
            .collect(),
        other => other.parts().into_iter().flatten().collect(),
    }
}

/// Which net each net is in the skeleton: two nets a two-pin absent part
/// joins are one (the smaller index), where both have at most `join_pins`
/// pins and no pour carries either (`poured`, per net; empty: none), and
/// the part is no decoupling capacitor (the tool's relation to a supply
/// pin: its nets are a rail and ground).
pub fn joined_nets(
    problem: &Problem,
    absent: &[bool],
    poured: &[bool],
    join_pins: usize,
) -> Vec<usize> {
    let nets = problem.net_weights.len();
    let mut pins = vec![0usize; nets];
    for component in &problem.components {
        for pin in &component.pins {
            if pin.net < nets {
                pins[pin.net] += 1;
            }
        }
    }
    let mut parent: Vec<usize> = (0..nets).collect();
    fn find(parent: &mut [usize], mut net: usize) -> usize {
        while parent[net] != net {
            parent[net] = parent[parent[net]];
            net = parent[net];
        }
        net
    }
    // A capacitor the tool ties to a supply pin decouples a rail: its
    // nets are never one.
    let users = problem
        .constraints
        .relations
        .len()
        .saturating_sub(problem.constraints.automatic);
    let decoupling: std::collections::BTreeSet<usize> = problem.constraints.relations[users..]
        .iter()
        .filter_map(|relation| relation.parts()[0])
        .collect();
    let small = |net: usize| {
        net < nets && pins[net] <= join_pins && !poured.get(net).copied().unwrap_or(false)
    };
    for (index, component) in problem.components.iter().enumerate() {
        if !absent.get(index).copied().unwrap_or(false)
            || component.pins.len() != 2
            || decoupling.contains(&index)
        {
            continue;
        }
        let (a, b) = (component.pins[0].net, component.pins[1].net);
        if a == b || !small(a) || !small(b) {
            continue;
        }
        let (a, b) = (find(&mut parent, a), find(&mut parent, b));
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    }
    (0..nets).map(|net| find(&mut parent, net)).collect()
}

/// Whether a skeleton part's body may grow by its passives' room: a
/// movable part with one plain body (not hollow, with a courtyard of one
/// shape or none, not a cutout) that no edge, region, overhang or row holds to its
/// exact body.
fn growable(problem: &Problem, index: usize) -> bool {
    let component = &problem.components[index];
    let constraints = &problem.constraints;
    !component.fixed
        && !component.copper_only
        && component.side != Side::Neither
        && component.hollow.is_empty()
        && component.courtyards.len() <= 1
        && !constraints.edges.iter().any(|(part, _, _)| *part == index)
        && !constraints.regions.iter().any(|(part, _)| *part == index)
        && !constraints
            .overhangs
            .iter()
            .any(|(part, _, _)| *part == index)
        && !constraints
            .followers
            .iter()
            .any(|follower| follower.part == index || follower.leader == index)
}

/// The room each part keeps for the absent passives it owns (`room` times
/// their tight bodies with the least spacing), mm², and the
/// area no part owns. A passive a decoupling relation ties to parts is
/// owned by those parts, in equal shares; any other by the skeleton parts
/// on its net with the fewest pins that reaches one, in equal shares; a
/// passive whose nets reach only passives by the owners of those (a
/// filter's second resistor). Only growable parts take room.
pub fn owned_room(problem: &Problem, absent: &[bool], room: f64) -> (Vec<f64>, f64) {
    let count = problem.components.len();
    let is_absent = |index: usize| absent.get(index).copied().unwrap_or(false);
    let area = |index: usize| {
        // What the passive needs at the loosest level: its tight body (or
        // its body) and the least spacing; the halos and the full spacing
        // overstate it on the dense boards, which place at the tight
        // levels (the 4in1's passives with them took more than its board).
        let component = &problem.components[index];
        let size = match component.tight {
            Some(tight) => [tight[2] - tight[0], tight[3] - tight[1]],
            None if component.round => [component.body_size[0]; 2],
            None => component.body_size,
        };
        let gap = problem.min_spacing;
        room * (size[0] + gap) * (size[1] + gap)
    };
    let nets = problem.net_weights.len();
    let mut on_net: Vec<Vec<usize>> = vec![Vec::new(); nets];
    for (index, component) in problem.components.iter().enumerate() {
        for pin in &component.pins {
            if pin.net < nets && on_net[pin.net].last() != Some(&index) {
                on_net[pin.net].push(index);
            }
        }
    }
    // Per absent part, its owners (the parts among which its area goes).
    let mut owners: Vec<Vec<usize>> = vec![Vec::new(); count];
    let automatic = problem
        .constraints
        .relations
        .len()
        .saturating_sub(problem.constraints.automatic);
    for relation in &problem.constraints.relations[automatic..] {
        let parts = relation_parts(relation);
        let Some((&part, anchors)) = parts.split_first() else {
            continue;
        };
        if !is_absent(part) {
            continue;
        }
        for &anchor in anchors {
            if !is_absent(anchor) && !owners[part].contains(&anchor) {
                owners[part].push(anchor);
            }
        }
    }
    // The part's nets, the fewest pins first (ties by net index).
    let nets_of = |index: usize| {
        let mut nets: Vec<usize> = problem.components[index]
            .pins
            .iter()
            .map(|pin| pin.net)
            .filter(|net| *net < on_net.len())
            .collect();
        nets.sort_unstable_by_key(|net| (on_net[*net].len(), *net));
        nets.dedup();
        nets
    };
    for (index, owned) in owners.iter_mut().enumerate() {
        if !is_absent(index) || !owned.is_empty() {
            continue;
        }
        for net in nets_of(index) {
            let parts: Vec<usize> = on_net[net]
                .iter()
                .copied()
                .filter(|part| *part != index && !is_absent(*part))
                .collect();
            if !parts.is_empty() {
                *owned = parts;
                break;
            }
        }
    }
    // Passives among passives: their neighbours' owners (two rounds).
    for _ in 0..2 {
        let before = owners.clone();
        for (index, owned) in owners.iter_mut().enumerate() {
            if !is_absent(index) || !owned.is_empty() {
                continue;
            }
            'nets: for net in nets_of(index) {
                for &other in &on_net[net] {
                    if other != index && is_absent(other) && !before[other].is_empty() {
                        *owned = before[other].clone();
                        break 'nets;
                    }
                }
            }
        }
    }
    let mut kept = vec![0.0; count];
    let mut unowned = 0.0;
    for (index, owned) in owners.iter().enumerate() {
        if !is_absent(index) {
            continue;
        }
        let takers: Vec<usize> = owned
            .iter()
            .copied()
            .filter(|owner| growable(problem, *owner))
            .collect();
        if takers.is_empty() {
            unowned += area(index);
            continue;
        }
        let share = area(index) / takers.len() as f64;
        for owner in takers {
            kept[owner] += share;
        }
    }
    (kept, unowned)
}

/// How far a body of `size` (round: a disc of diameter `size[0]`) grows on
/// every side to take `area` more.
pub fn growth(size: [f64; 2], round: bool, area: f64) -> f64 {
    if area <= 0.0 {
        return 0.0;
    }
    if round {
        let diameter = size[0];
        ((diameter * diameter + 4.0 * area / std::f64::consts::PI).sqrt() - diameter) / 2.0
    } else {
        let sum = size[0] + size[1];
        ((sum * sum + 4.0 * area).sqrt() - sum) / 4.0
    }
}

/// The skeleton's problem: the absent parts without pins, body, halo or
/// constraint (fixed where they lie, on neither side), the nets through
/// them joined (`joined_nets`), and the skeleton parts' bodies grown by
/// their passives' room. The parts keep their indices. With the report's
/// counts filled in.
pub fn skeleton_problem(
    problem: &Problem,
    absent: &[bool],
    poured: &[bool],
    settings: &Settings,
    report: &mut Report,
) -> Problem {
    let is_absent = |index: usize| absent.get(index).copied().unwrap_or(false);
    let joined = joined_nets(problem, absent, poured, settings.join_pins);
    let (room, unowned) = owned_room(problem, absent, settings.room);
    let mut skeleton = problem.clone();
    report.joined_nets = joined
        .iter()
        .enumerate()
        .filter(|(net, root)| net != *root)
        .count();
    if report.joined_nets > 0 {
        // A joined net weighs as its lightest member (the larger nets are
        // weighed down by the lowering).
        let mut weights = problem.net_weights.clone();
        for (net, root) in joined.iter().enumerate() {
            weights[*root] = weights[*root].min(problem.net_weights[net]);
        }
        skeleton.net_weights = weights;
        let join = |counted: &[bool]| -> Vec<bool> {
            let mut joined_counted = counted.to_vec();
            for (net, root) in joined.iter().enumerate() {
                if counted.get(net).copied().unwrap_or(false) && *root < joined_counted.len() {
                    joined_counted[*root] = true;
                }
            }
            joined_counted
        };
        if let Some(crossings) = &problem.crossings {
            let mut crossings = (**crossings).clone();
            crossings.counted = join(&crossings.counted);
            skeleton.crossings = Some(std::sync::Arc::new(crossings));
        }
        if let Some(escape) = &problem.escape {
            let mut escape = (**escape).clone();
            escape.counted = join(&escape.counted);
            skeleton.escape = Some(std::sync::Arc::new(escape));
        }
    }
    // The measured escape demand routes the board as it is, passives and
    // all: the skeleton runs on the estimate.
    if let Some(escape) = &skeleton.escape
        && escape.measured.is_some()
    {
        let mut escape = (**escape).clone();
        escape.measured = None;
        skeleton.escape = Some(std::sync::Arc::new(escape));
    }
    for (index, component) in skeleton.components.iter_mut().enumerate() {
        if is_absent(index) {
            component.fixed = true;
            component.pins.clear();
            component.side = Side::Neither;
            component.halo = 0.0;
            component.slots.clear();
            component.courtless.clear();
            report.absent += 1;
            continue;
        }
        for pin in &mut component.pins {
            if let Some(root) = joined.get(pin.net) {
                pin.net = *root;
            }
        }
        if component.fixed {
            continue;
        }
        report.skeleton_parts += 1;
        let margin = growth(component.body_size, component.round, room[index]);
        if margin > 0.0 {
            report.grown += 1;
            report.room_mm2 += room[index];
            component.body_size = [
                component.body_size[0] + 2.0 * margin,
                component.body_size[1] + 2.0 * margin,
            ];
            let grow = |b: &mut [f64; 4]| {
                *b = [b[0] - margin, b[1] - margin, b[2] + margin, b[3] + margin];
            };
            if let Some(tight) = component.tight.as_mut() {
                grow(tight);
            }
            // A part without a courtyard keeps its room from others like it
            // with its courtless body.
            component.courtless.iter_mut().for_each(grow);
        }
    }
    report.unowned_mm2 = unowned;
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        // Why the skeleton's parts do not grow, by reason.
        let mut reasons: std::collections::BTreeMap<&str, usize> = Default::default();
        for (index, component) in problem.components.iter().enumerate() {
            if is_absent(index) || component.fixed {
                continue;
            }
            let constraints = &problem.constraints;
            let reason = if !component.hollow.is_empty() {
                "hollow"
            } else if component.courtyards.len() > 1 {
                "courtyards"
            } else if constraints.edges.iter().any(|(part, _, _)| *part == index) {
                "edge"
            } else if constraints.regions.iter().any(|(part, _)| *part == index) {
                "region"
            } else if constraints
                .overhangs
                .iter()
                .any(|(part, _, _)| *part == index)
            {
                "overhang"
            } else if constraints
                .followers
                .iter()
                .any(|follower| follower.part == index || follower.leader == index)
            {
                "row"
            } else if !growable(problem, index) {
                "other"
            } else {
                "growable"
            };
            *reasons.entry(reason).or_default() += 1;
        }
        eprintln!(
            "placer: skeleton: {} parts, {} absent, {} nets joined, {} grown by {:.0} mm2, {:.0} mm2 unowned; the parts by growth: {reasons:?}",
            report.skeleton_parts,
            report.absent,
            report.joined_nets,
            report.grown,
            report.room_mm2,
            unowned
        );
    }
    // The absent parts' relations go (the decoupling relations: the
    // passives' placement has them back).
    let users = skeleton
        .constraints
        .relations
        .len()
        .saturating_sub(skeleton.constraints.automatic);
    let mut kept_automatic = 0;
    let relations = std::mem::take(&mut skeleton.constraints.relations);
    for (position, relation) in relations.into_iter().enumerate() {
        if relation_parts(&relation).into_iter().any(&is_absent) {
            continue;
        }
        if position >= users {
            kept_automatic += 1;
        }
        skeleton.constraints.relations.push(relation);
    }
    skeleton.constraints.automatic = kept_automatic;
    skeleton
        .constraints
        .kept
        .retain(|kept| !is_absent(kept.part));
    skeleton.constraints.link_pairs();
    skeleton
}

/// The ranking key of a placement among its alternatives here: illegal
/// and unplaced parts, then missed constraints, then wirelength.
fn key(placement: &Placement) -> (usize, f64, f64) {
    let missed: f64 = placement
        .constraints
        .iter()
        .map(|status| status.violation)
        .sum();
    (
        placement.unplaced.len() + placement.illegal.len(),
        missed,
        placement.wirelength_final,
    )
}

/// Places `problem` as a race over skeletons does for one seed: the
/// skeleton alone (every part `passive` does not mark, and the passives a
/// constraint names), then the passives around it, held. `poured` marks
/// the nets a pour carries (no net is joined through a passive across
/// them). The placement is of `problem`: its relaxation keeps both phases
/// legal (from the skeleton's level on, each held part's halo the smaller
/// of the two phases'), its illegal parts and constraints judged on it.
/// Where the skeleton leaves parts without a legal place, the seed is
/// placed whole with the work left; where the passives find none around
/// the skeleton, too, and the better kept.
pub fn place_skeleton(
    problem: &Problem,
    config: &Config,
    passive: &[bool],
    poured: &[bool],
    settings: &Settings,
) -> Placement {
    let mut report = Report::default();
    let absent = absent_parts(problem, passive);
    if !absent.iter().any(|absent| *absent) {
        let mut placement = place(problem, config);
        report.passive_work = placement.work;
        report.level = placement.level;
        placement.skeleton = Some(report);
        return placement;
    }
    let skeleton_problem = skeleton_problem(problem, &absent, poured, settings, &mut report);
    let mut first = config.clone();
    first.work_limit = config
        .work_limit
        .map(|limit| ((limit as f64 * settings.work_share) as u64).max(1));
    let skeleton = place(&skeleton_problem, &first);
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!(
            "placer: skeleton placed: level {}, {} unplaced, {} illegal, work {:.0} s, wirelength {:.0}",
            skeleton.level,
            skeleton.unplaced.len(),
            skeleton.illegal.len(),
            skeleton.work as f64 / crate::work_per_second(),
            skeleton.wirelength_final
        );
    }
    report.skeleton_work = skeleton.work;
    report.skeleton_level = skeleton.level;
    report.skeleton_wirelength = skeleton.wirelength_final;
    let left = |used: u64| {
        config
            .work_limit
            .map(|limit| limit.saturating_sub(used).max(1))
    };
    // A skeleton with parts left without a legal place (an overfull one,
    // or one whose share of the work ran out before a level seated them):
    // the seed is placed whole with the work left. Video's skeletons at
    // full halos left one or two memory modules without a place on 5 of
    // 6 seeds, and the passives' placement with them movable from the next
    // level on failed on all 5 (docs/placer.md, "A race over skeletons").
    if skeleton.overfull || !skeleton.unplaced.is_empty() || !skeleton.illegal.is_empty() {
        let mut whole = config.clone();
        whole.work_limit = left(skeleton.work);
        let mut placement = place(problem, &whole);
        report.fallback = true;
        report.fallback_kept = true;
        report.fallback_work = placement.work;
        report.level = placement.level;
        placement.work += skeleton.work;
        placement.skeleton = Some(report);
        return placement;
    }
    let held: Vec<bool> = (0..problem.components.len())
        .map(|index| !absent[index] && !problem.components[index].fixed)
        .collect();
    report.held = held.iter().filter(|held| **held).count();
    let mut holding = problem.clone();
    for (index, held) in held.iter().enumerate() {
        if *held {
            holding.components[index].fixed = true;
            holding.poses[index] = skeleton.poses[index];
        }
    }
    let mut second = config.clone();
    second.work_limit = left(skeleton.work);
    // From the skeleton's level on: the held skeleton is legal there.
    second.first_level = skeleton.level;
    let passives = place(&holding, &second);
    report.passive_work = passives.work;
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!(
            "placer: passives placed: level {}, {} unplaced, {} illegal, work {:.0} s",
            passives.level,
            passives.unplaced.len(),
            passives.illegal.len(),
            passives.work as f64 / crate::work_per_second()
        );
    }
    let mut placement = judged(problem, passives, &skeleton, &held);
    let mut used = skeleton.work + report.passive_work;
    if !placement.unplaced.is_empty() || !placement.illegal.is_empty() {
        let mut whole = config.clone();
        whole.work_limit = left(used);
        let fallback = place(problem, &whole);
        report.fallback = true;
        report.fallback_work = fallback.work;
        used += fallback.work;
        if key(&fallback) < key(&placement) {
            report.fallback_kept = true;
            placement = fallback;
        }
    }
    report.level = placement.level;
    placement.work = used;
    let mut frames = skeleton.frames;
    frames.append(&mut placement.frames);
    placement.frames = frames;
    placement.skeleton = Some(report);
    placement
}

/// The passives' placement made a placement of `problem`: the relaxation
/// with each held part's halo the smaller of the skeleton's and the
/// passives' (the skeleton's level is never looser than the passives':
/// `Config::first_level`), the parts on the board's marks of both, and the
/// illegal parts, constraints and wirelength judged on `problem` with it.
fn judged(
    problem: &Problem,
    mut placement: Placement,
    skeleton: &Placement,
    held: &[bool],
) -> Placement {
    let halos = |relaxation: &crate::Relaxation| -> Vec<f64> {
        let mut relaxed = problem.clone();
        relaxation.apply(&mut relaxed);
        relaxed
            .components
            .iter()
            .map(|component| component.halo)
            .collect()
    };
    let passives = halos(&placement.relaxation);
    let skeletons = halos(&skeleton.relaxation);
    let merged: Vec<f64> = passives
        .iter()
        .zip(&skeletons)
        .zip(held)
        .map(|((passive, skeleton), held)| {
            if *held {
                passive.min(*skeleton)
            } else {
                *passive
            }
        })
        .collect();
    if merged != passives {
        placement.relaxation.halos = Some(std::sync::Arc::new(merged));
    }
    let mut on_marks = placement.relaxation.on_marks.clone();
    on_marks.extend(
        skeleton
            .relaxation
            .on_marks
            .iter()
            .copied()
            .filter(|part| held.get(*part).copied().unwrap_or(false)),
    );
    on_marks.sort_unstable();
    on_marks.dedup();
    placement.relaxation.on_marks = on_marks;
    let mut relaxed = problem.clone();
    placement.relaxation.apply(&mut relaxed);
    relaxed.constraints.link_pairs();
    relaxed.sync_followers(&mut placement.poses);
    placement.illegal = legal::illegal_components(&relaxed, &placement.poses);
    placement.constraints = crate::constraints::report(&relaxed, &placement.poses);
    placement.wirelength_final = problem.wirelength(&placement.poses);
    placement
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::{Component, Pin, Point, Pose};

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
                    offset: [
                        (index as f64 + 0.5) * size[0] / nets.len() as f64 - size[0] / 2.0,
                        0.0,
                    ],
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
            hole_rings: Vec::new(),
            copper_graphics: Vec::new(),
            courtless: Vec::new(),
            slots: Vec::new(),
        }
    }

    /// Two ICs on a 40 x 30 mm board: U1's nets 0-3 run through series
    /// resistors R1-R4 (nets 4-7) to U2, both on the rail (net 8) and
    /// ground (net 9) with ten more parts each on them (a header's pins),
    /// and a decoupling capacitor per IC (C1 at U1, C2 at U2, the tool's
    /// relations); R5 on a ground net alone is a pull-down of U2's net 7.
    fn board() -> (Problem, Vec<bool>) {
        let mut components = vec![
            part("U1", [5.0, 5.0], &[0, 1, 2, 3, 8, 9]),
            part("U2", [5.0, 5.0], &[4, 5, 6, 7, 8, 9]),
        ];
        for index in 0..4 {
            components.push(part(
                &format!("R{}", index + 1),
                [1.6, 0.8],
                &[index, index + 4],
            ));
        }
        components.push(part("C1", [1.6, 0.8], &[8, 9]));
        components.push(part("C2", [1.6, 0.8], &[8, 9]));
        components.push(part("R5", [1.6, 0.8], &[7, 9]));
        let mut header = part("J1", [2.5, 26.0], &[8; 10]);
        for pin in header.pins.iter_mut().skip(5) {
            pin.net = 9;
        }
        header.fixed = true;
        components.push(header);
        let count = components.len();
        let poses = vec![
            Pose {
                position: [20.0, 15.0],
                angle: 0.0,
            };
            count
        ];
        let mut poses = poses;
        poses[count - 1].position = [2.0, 15.0];
        let mut problem = Problem {
            outline: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
            components,
            net_weights: vec![1.0; 10],
            poses,
            spacing: 0.3,
            grid: 0.1,
            edge_margin: 0.5,
            min_spacing: 0.2,
            far_side_pads_only: false,
            shared_halos: false,
            escape: None,
            crossings: None,
            pieces: Vec::new(),
            constraints: Default::default(),
        };
        for (capacitor, ic) in [(6, 0), (7, 1)] {
            problem.constraints.relations.push(Relation::Near {
                part: capacitor,
                anchor: Anchor::Point(ic, [1.0, 0.0]),
                max: 2.5,
            });
        }
        problem.constraints.automatic = 2;
        let passive = (0..count)
            .map(|index| {
                let name = &problem.components[index].name;
                name.starts_with('R') || name.starts_with('C')
            })
            .collect();
        (problem, passive)
    }

    #[test]
    fn the_free_passives_are_absent_and_a_named_one_stays() {
        let (mut problem, passive) = board();
        let absent = absent_parts(&problem, &passive);
        assert_eq!(
            absent,
            [
                false, false, true, true, true, true, true, true, true, false
            ]
        );
        // A user's region on R1 keeps it in the skeleton; the tool's own
        // decoupling relations do not keep C1 and C2.
        problem
            .constraints
            .regions
            .push((2, [0.0, 0.0, 40.0, 30.0]));
        let absent = absent_parts(&problem, &passive);
        assert!(!absent[2] && absent[3] && absent[6] && absent[7]);
    }

    #[test]
    fn series_passives_join_small_nets_and_rails_stay_apart() {
        let (problem, passive) = board();
        let absent = absent_parts(&problem, &passive);
        let joined = joined_nets(&problem, &absent, &[], 8);
        // R1-R4 join U1's nets to U2's; R5's ground net (13 pins) and the
        // capacitors' rail and ground stay apart.
        assert_eq!(joined, [0, 1, 2, 3, 0, 1, 2, 3, 8, 9]);
        // A poured net is never joined, nor any with join_pins 0.
        let mut poured = vec![false; 10];
        poured[5] = true;
        let joined = joined_nets(&problem, &absent, &poured, 8);
        assert_eq!(joined[5], 5);
        assert_eq!(joined[4], 0);
        assert_eq!(
            joined_nets(&problem, &absent, &[], 0),
            (0..10).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_owners_bodies_grow_by_their_passives_room() {
        let (problem, passive) = board();
        let absent = absent_parts(&problem, &passive);
        let (room, unowned) = owned_room(&problem, &absent, 1.0);
        // Each passive with the least spacing: 1.8 x 1.0. U1 owns C1 (its
        // relation) and R1-R4 (each R's two nets have two pins, the lower
        // index first: U1's); U2 owns C2 and R5 (net 7, three pins, is
        // R5's net with the fewest).
        let one = 1.8 * 1.0;
        assert!((room[0] - 5.0 * one).abs() < 1e-9, "{}", room[0]);
        assert!((room[1] - 2.0 * one).abs() < 1e-9, "{}", room[1]);
        assert_eq!(unowned, 0.0);
        // The growth takes exactly that area.
        let margin = growth([5.0, 5.0], false, room[0]);
        assert!(((5.0 + 2.0 * margin).powi(2) - 25.0 - room[0]).abs() < 1e-9);
        let disc = growth([4.0, 4.0], true, 3.0);
        let area = std::f64::consts::PI / 4.0 * ((4.0 + 2.0 * disc).powi(2) - 16.0);
        assert!((area - 3.0).abs() < 1e-9);
        let mut report = Report::default();
        let skeleton = skeleton_problem(&problem, &absent, &[], &Settings::default(), &mut report);
        assert_eq!(report.absent, 7);
        assert_eq!(report.skeleton_parts, 2);
        assert_eq!(report.grown, 2);
        assert_eq!(report.joined_nets, 4);
        let grown = &skeleton.components[0];
        assert!((grown.body_size[0] - (5.0 + 2.0 * margin)).abs() < 1e-9);
        for absent_part in &skeleton.components[2..9] {
            assert!(absent_part.fixed && absent_part.pins.is_empty());
            assert_eq!(absent_part.side, Side::Neither);
        }
        // The decoupling relations went with their capacitors; U2's nets
        // through the resistors are U1's.
        assert!(skeleton.constraints.relations.is_empty());
        assert_eq!(skeleton.constraints.automatic, 0);
        assert_eq!(skeleton.components[1].pins[0].net, 0);
    }

    #[test]
    fn the_passives_are_placed_around_the_held_skeleton() {
        let (problem, passive) = board();
        let config = Config::new();
        let settings = Settings::default();
        let placement = place_skeleton(&problem, &config, &passive, &[], &settings);
        let report = placement.skeleton.clone().expect("a report");
        assert!(placement.unplaced.is_empty(), "{:?}", placement.unplaced);
        assert!(placement.illegal.is_empty(), "{:?}", placement.illegal);
        assert_eq!(report.held, 2);
        assert!(!report.fallback);
        assert_eq!(
            placement.work,
            report.skeleton_work + report.passive_work + report.fallback_work
        );
        assert!(report.level >= report.skeleton_level);
        // The skeleton's own placement, alone: the held parts lie where it
        // put them.
        let absent = absent_parts(&problem, &passive);
        let skeleton = skeleton_problem(&problem, &absent, &[], &settings, &mut Report::default());
        let alone = place(&skeleton, &config);
        assert_eq!(placement.poses[0], alone.poses[0]);
        assert_eq!(placement.poses[1], alone.poses[1]);
        // Legal on the whole problem at the placement's relaxation, and the
        // decoupling capacitors near their pins.
        let mut relaxed = problem.clone();
        placement.relaxation.apply(&mut relaxed);
        relaxed.constraints.link_pairs();
        assert!(legal::illegal_components(&relaxed, &placement.poses).is_empty());
        assert!(
            crate::constraints::report(&relaxed, &placement.poses)
                .iter()
                .all(|status| status.satisfied)
        );
    }

    #[test]
    fn the_better_placement_by_the_races_key_is_kept() {
        let (problem, _) = board();
        let placed = place(&problem, &Config::new());
        let mut unplaced = placed.clone();
        unplaced.unplaced = vec![3];
        unplaced.wirelength_final = 0.0;
        // Fewer parts without a place first, however short the wire.
        assert!(key(&placed) < key(&unplaced));
        let mut longer = placed.clone();
        longer.wirelength_final += 1.0;
        assert!(key(&placed) < key(&longer));
    }

    #[test]
    fn passives_without_room_fall_back_to_the_whole_placement() {
        // Every passive's room taken away (room 0) on a board just large
        // enough: the skeleton spreads, the passives may find no place;
        // whatever happens, the placement kept is the better by the race's
        // key of the two, and a board without passives places as `place`.
        let (problem, passive) = board();
        let config = Config::new();
        let none = vec![false; problem.components.len()];
        let plain = place(&problem, &config);
        let without = place_skeleton(&problem, &config, &none, &[], &Settings::default());
        assert_eq!(without.poses, plain.poses);
        assert_eq!(
            without.skeleton.as_ref().map(|report| report.absent),
            Some(0)
        );
        let mut small = problem.clone();
        small.outline = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 30.0], [0.0, 30.0]];
        small.poses[9].position = [1.5, 15.0];
        let settings = Settings {
            room: 0.0,
            ..Settings::default()
        };
        let placement = place_skeleton(&small, &config, &passive, &[], &settings);
        let report = placement.skeleton.clone().expect("a report");
        if report.fallback {
            assert!(report.fallback_work > 0);
        }
        assert_eq!(
            placement.work,
            report.skeleton_work + report.passive_work + report.fallback_work
        );
        assert!(placement.unplaced.is_empty() && placement.illegal.is_empty());
    }
}

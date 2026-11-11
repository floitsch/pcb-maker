// Copyright (C) 2026 Toit contributors.

//! The router's rules the placer keeps: the far side under vias in pads
//! and fan-outs, the room around grid arrays, and the length windows.

use super::*;
use pcb_placer as core;

/// The far side under the vias the router's via-in-pad rule puts in
/// admitted footprints' enclosed pads (`far_side_vias`): each via goes
/// through the board, so it joins its footprint's `far_side`, where parts
/// on the other side keep off it as off a through-hole pin (MIDAS-MK1:
/// C112, an 0603 on the back, under U106's B2, whose via the router then
/// refused). The box keeps the via's copper and its drill's hole clearance
/// from the other side's pads where the legality test adds the copper
/// clearance alone (the tight levels and KiCad's legality, which test
/// far-side boxes against pads).
fn far_side_of_vias_in_pads(
    pcb: &Expr,
    rules: &KiCadBoardRouterConfig,
    config: &KiCadBoardPlacerConfig,
    pad_offsets: &[BTreeMap<String, [f64; 2]>],
    problem: &mut core::Problem,
) {
    let copper = config.copper_clearance_mm.unwrap_or(0.2);
    let hole = config.hole_clearance_mm.unwrap_or(rules.hole_clearance_mm);
    for via in crate::board_router::far_side_vias(pcb, rules) {
        let Some(center) = pad_offsets
            .get(via.footprint)
            .and_then(|pads| pads.get(&via.pad))
        else {
            continue;
        };
        let half = (via.diameter / 2.0).max(via.drill / 2.0 + hole - copper);
        problem.components[via.footprint].far_side.push([
            center[0] - half,
            center[1] - half,
            center[0] + half,
            center[1] + half,
        ]);
    }
}

/// The far side under the dog-bones of the router's prepared fan-outs
/// (`bga_fanout`, `far_side_balls`): each inner ball's via sits half a
/// pitch off along both axes of the footprint, toward the nearer way out
/// (`rings_of`; both ways along an axis where both sides are as near),
/// and goes through the board as a via in a pad does. Nothing with the
/// fan-out off.
fn far_side_of_fanouts(
    pcb: &Expr,
    rules: &KiCadBoardRouterConfig,
    config: &KiCadBoardPlacerConfig,
    pad_offsets: &[BTreeMap<String, [f64; 2]>],
    problem: &mut core::Problem,
) {
    let copper = config.copper_clearance_mm.unwrap_or(0.2);
    let hole = config.hole_clearance_mm.unwrap_or(rules.hole_clearance_mm);
    for array in crate::board_router::far_side_balls(pcb, rules) {
        let Some(offsets) = pad_offsets.get(array.footprint) else {
            continue;
        };
        let balls: Vec<(&str, [f64; 2])> = array
            .balls
            .iter()
            .filter_map(|ball| offsets.get(ball).map(|at| (ball.as_str(), *at)))
            .collect();
        let local: Vec<[f64; 2]> = balls.iter().map(|(_, at)| *at).collect();
        let (_, toward) = crate::board_router::rings_of(&local, array.pitch);
        let half = (array.diameter / 2.0).max(array.drill / 2.0 + hole - copper);
        let signs = |side: f64| {
            if side == 0.0 {
                vec![-1.0, 1.0]
            } else {
                vec![side]
            }
        };
        for (position, (ball, at)) in balls.iter().enumerate() {
            if !array.inner.iter().any(|inner| inner == ball) {
                continue;
            }
            for sx in signs(toward[position][0]) {
                for sy in signs(toward[position][1]) {
                    let center = [
                        at[0] + sx * array.pitch / 2.0,
                        at[1] + sy * array.pitch / 2.0,
                    ];
                    problem.components[array.footprint].far_side.push([
                        center[0] - half,
                        center[1] - half,
                        center[0] + half,
                        center[1] + half,
                    ]);
                }
            }
        }
    }
}

/// The room an array's outer rings' surface escapes take beyond its ball
/// field on its own side (`bga_room`): the surface rings' tracks, a track
/// and a clearance each, turning along the edge, then a row of the class's
/// vias with a clearance on both sides (Glasgow's U30: 2 x 0.3 + 0.6 + 0.3
/// = 1.5 mm; its designer keeps the nearest movable part's pads 1.28 mm
/// from the field and nine in ten balls' channels 1.66 mm free).
pub(super) fn surface_band(room: &crate::board_router::ArrayRoom) -> f64 {
    room.surface_rings as f64 * (room.track + room.clearance) + room.via + 2.0 * room.clearance
}

/// The room beyond the ball field on the far side the via ring's tracks
/// take (`bga_room`): a track and a clearance per ring deeper than the
/// surface rings that holds a ball of a net (Glasgow's U30: rings 2 to 5,
/// 4 x 0.3 = 1.2 mm); none where every ball leaves on the surface, and
/// then the array keeps no room at all.
pub(super) fn far_band(room: &crate::board_router::ArrayRoom) -> f64 {
    (room.deepest + 1).saturating_sub(room.surface_rings) as f64 * (room.track + room.clearance)
}

/// The box `[min x, min y, max x, max y]` of `inner` and `outer`.
fn union_box(inner: [f64; 4], outer: [f64; 4]) -> [f64; 4] {
    [
        inner[0].min(outer[0]),
        inner[1].min(outer[1]),
        inner[2].max(outer[2]),
        inner[3].max(outer[3]),
    ]
}

/// Room around grid arrays (`bga_room`; docs/placer.md, "Room around grid
/// arrays"). Per array the router's detection finds
/// (`array_rooms`, the fan-out's `board_arrays`), in its own frame from
/// its ball field (the box of the ball centres grown by half a pitch):
///
/// - its own side: its body (and its tight body) grows to the field and
///   the surface band (`surface_band`), so no part's body comes nearer at
///   any level; the edge inset grows by as much, so the array's copper
///   keeps the edge margin it kept;
/// - the far side: the field and the far band (`far_band`) join its
///   `far_side` boxes, letting in only the array's own decoupling
///   capacitors (`Component::far_open`), which keep off the via sites
///   (the fan-out's and the vias in pads, the boxes before).
///
/// Only an array whose inner balls the fan-out serves with dog-bones in
/// the diagonal gaps (`far_side_balls`, Glasgow's U30) keeps room: one
/// whose inner balls leave by vias in their pads (MIDAS's U106, no gap
/// fits a via; the room cost MIDAS-MK1 its strict pass at one seed of
/// three) or whose every ball leaves on the surface (vd-rd's U200: 3 to 6
/// open) keeps none, nor does any array with the fan-out off.
/// Fixed parts are not moved: a free array keeps its room from them as
/// from any part, and a fixed array's report names the fixed parts in its
/// room. Returns the report lines.
fn room_around_arrays(
    pcb: &Expr,
    rules: &KiCadBoardRouterConfig,
    pad_offsets: &[BTreeMap<String, [f64; 2]>],
    references: &[String],
    problem: &mut core::Problem,
) -> Vec<String> {
    let mut report = Vec::new();
    let dog_boned: BTreeSet<usize> = crate::board_router::far_side_balls(pcb, rules)
        .iter()
        .map(|array| array.footprint)
        .collect();
    for room in crate::board_router::array_rooms(pcb, rules) {
        if !dog_boned.contains(&room.footprint) {
            continue;
        }
        let Some(offsets) = pad_offsets.get(room.footprint) else {
            continue;
        };
        let balls: Vec<[f64; 2]> = room
            .balls
            .iter()
            .filter_map(|ball| offsets.get(ball).copied())
            .collect();
        let Some(centres) = balls
            .iter()
            .map(|at| [at[0], at[1], at[0], at[1]])
            .reduce(union_box)
        else {
            continue;
        };
        let half = room.pitch / 2.0;
        let field = [
            centres[0] - half,
            centres[1] - half,
            centres[2] + half,
            centres[3] + half,
        ];
        let grow = |b: [f64; 4], by: f64| [b[0] - by, b[1] - by, b[2] + by, b[3] + by];
        let (top, far) = (surface_band(&room), far_band(&room));
        // An array whose every ball of a net leaves on the surface (vd-rd's
        // DDR3 BGA-96: each ball faces the outside or the empty middle)
        // has no via ring crowding its outer rings' escapes: no room (the
        // band cost vd-rd 3 connections and 2 findings, measured).
        if far <= 0.0 {
            continue;
        }
        let component = &mut problem.components[room.footprint];
        let body = [
            component.body_center[0] - component.body_size[0] / 2.0,
            component.body_center[1] - component.body_size[1] / 2.0,
            component.body_center[0] + component.body_size[0] / 2.0,
            component.body_center[1] + component.body_size[1] / 2.0,
        ];
        let grown = union_box(body, grow(field, top));
        let least = [
            body[0] - grown[0],
            body[1] - grown[1],
            grown[2] - body[2],
            grown[3] - body[3],
        ]
        .into_iter()
        .fold(f64::INFINITY, f64::min);
        component.body_center = [(grown[0] + grown[2]) / 2.0, (grown[1] + grown[3]) / 2.0];
        component.body_size = [grown[2] - grown[0], grown[3] - grown[1]];
        component.edge_inset += least.max(0.0);
        if let Some(tight) = component.tight {
            component.tight = Some(union_box(tight, grow(field, top)));
        }
        let from = component.far_side.len();
        component.far_side.push(grow(field, far));
        component.far_open = core::FarOpen {
            from,
            parts: room.decoupling.clone(),
        };
        let letting: Vec<String> = room
            .decoupling
            .iter()
            .map(|part| references[*part].clone())
            .collect();
        let mut line = format!(
            "{}: pitch {:.3} mm, surface rings {}, deepest ring {}; own side {:.3} mm, far side {:.3} mm beyond the field",
            references[room.footprint], room.pitch, room.surface_rings, room.deepest, top, far,
        );
        if !letting.is_empty() {
            line += &format!("; under it only its decoupling {}", letting.join(" "));
        }
        let fixed = fixed_in_room(problem, room.footprint, references);
        if !fixed.is_empty() {
            line += &format!("; fixed parts left in its room: {}", fixed.join(" "));
        }
        report.push(line);
    }
    report
}

/// For a fixed array (`room_around_arrays`), the fixed parts whose bodies
/// meet its grown body on its side or its far-side room, at their poses:
/// the room cannot move them.
fn fixed_in_room(problem: &core::Problem, array: usize, references: &[String]) -> Vec<String> {
    if !problem.components[array].fixed {
        return Vec::new();
    }
    // The legality test leaves two fixed parts alone: ask it of the array
    // as if it moved, where it lies on the board.
    let mut probe = problem.clone();
    probe.components[array].fixed = false;
    let at = probe.poses[array];
    if !core::legal::is_legal(&probe, &probe.poses, array, at, std::iter::empty()) {
        return Vec::new();
    }
    (0..problem.components.len())
        .filter(|other| *other != array && problem.components[*other].fixed)
        .filter(|other| {
            !core::legal::is_legal(&probe, &probe.poses, array, at, [*other].into_iter())
        })
        .map(|other| references[other].clone())
        .collect()
}

/// The share of a net's length maximum its pins are kept within before
/// the anneal pays for the distance (`Relation::Within`'s soft bound).
pub(super) const LENGTH_SOFT_SHARE: f64 = 0.8;

/// At most this many pins of a length-bounded net are related pairwise (a
/// bus's nets join two or three pins).
const LENGTH_MOST_PINS: usize = 16;

/// The share of a net's length minimum its pins are kept apart at least
/// (`length_minima`, `Relation::Within`'s floor): the route between two
/// pins is about their octilinear distance, and the meanders on its
/// straight tracks add at most about as much again where a bus runs at
/// its clearance (the firestarter's DVP bus on the designer's placement:
/// docs/placer.md, "Length minima").
pub(super) const LENGTH_FLOOR_SHARE: f64 = 0.5;

/// Length windows (`length_maxima`, `length_minima`): for each net of a
/// `length` rule with a maximum (`length_bounded_nets`), every two of its
/// pins on different parts, not both fixed, keep within
/// `LENGTH_SOFT_SHARE` of the maximum octilinearly, and pay steeply beyond
/// it; for each net with a minimum (`length_floor_nets`) whose pins lie on
/// two parts, they keep `LENGTH_FLOOR_SHARE` of the minimum apart at
/// least (`Relation::Within`, automatic relations: not reported as the
/// user's constraints). The part that moves in the relation repair is the
/// movable one, the smaller of two movable. Returns per net what it holds.
fn length_windows(
    rules: &KiCadBoardRouterConfig,
    config: &KiCadBoardPlacerConfig,
    net_ids: &BTreeMap<String, usize>,
    footprint_count: usize,
    problem: &mut core::Problem,
) -> Vec<LoweredLengthRule> {
    let maxima = config
        .length_maxima
        .then(|| crate::board_router::length_bounded_nets(rules))
        .flatten();
    let minima = config
        .length_minima
        .then(|| crate::board_router::length_floor_nets(rules))
        .flatten();
    let mut nets: BTreeSet<&String> = BTreeSet::new();
    nets.extend(maxima.iter().flat_map(|bounded| bounded.keys()));
    nets.extend(minima.iter().flat_map(|floors| floors.keys()));
    let mut report = Vec::new();
    let mut relations = Vec::new();
    let first = problem.constraints.relations.len();
    for net in nets {
        let max = maxima
            .as_ref()
            .and_then(|bounded| bounded.get(net).copied());
        let min = minima.as_ref().and_then(|floors| floors.get(net).copied());
        let Some(&id) = net_ids.get(net) else {
            continue;
        };
        let pins: Vec<(usize, [f64; 2])> = (0..footprint_count)
            .flat_map(|index| {
                problem.components[index]
                    .pins
                    .iter()
                    .filter(move |pin| pin.net == id)
                    .map(move |pin| (index, pin.offset))
            })
            .collect();
        let mut parts: Vec<usize> = pins.iter().map(|(index, _)| *index).collect();
        parts.dedup();
        let mut entry = LoweredLengthRule {
            net: net.clone(),
            min,
            max,
            soft: f64::INFINITY,
            parts: parts.clone(),
            ..LoweredLengthRule::default()
        };
        if pins.len() > LENGTH_MOST_PINS {
            entry.note = format!("{} pins: not held (at most {LENGTH_MOST_PINS})", pins.len());
            entry.relations = first + relations.len()..first + relations.len();
            report.push(entry);
            continue;
        }
        let (soft, max_bound) = max.map_or((f64::INFINITY, f64::INFINITY), |max| {
            (LENGTH_SOFT_SHARE * max, max)
        });
        let floor = match min {
            Some(min) if parts.len() == 2 => (LENGTH_FLOOR_SHARE * min).min(soft),
            Some(_) => {
                entry.note = format!(
                    "its pins lie on {} parts: the minimum is not held apart (two parts only)",
                    parts.len()
                );
                0.0
            }
            None => 0.0,
        };
        entry.soft = soft;
        entry.floor = floor;
        let start = first + relations.len();
        if max.is_some() || floor > 0.0 {
            for (position, &(a, pin_a)) in pins.iter().enumerate() {
                for &(b, pin_b) in &pins[position + 1..] {
                    let (fixed_a, fixed_b) =
                        (problem.components[a].fixed, problem.components[b].fixed);
                    if a == b || (fixed_a && fixed_b) {
                        continue;
                    }
                    let area = |index: usize| {
                        let size = problem.components[index].body_size;
                        size[0] * size[1]
                    };
                    let first = fixed_b || (!fixed_a && area(a) <= area(b));
                    let ((part, pin), (other, other_pin)) = if first {
                        ((a, pin_a), (b, pin_b))
                    } else {
                        ((b, pin_b), (a, pin_a))
                    };
                    relations.push(core::constraints::Relation::Within {
                        part,
                        pin,
                        other,
                        other_pin,
                        soft,
                        max: max_bound,
                        floor,
                    });
                }
            }
        }
        entry.relations = start..first + relations.len();
        if entry.relations.is_empty() && entry.note.is_empty() && parts.len() >= 2 {
            entry.note = "its parts are fixed: nothing to hold".into();
        }
        report.push(entry);
    }
    if relations.is_empty() {
        return report;
    }
    if problem.constraints.relation_weight == 0.0 {
        problem.constraints.relation_weight = config.constraint_weight;
    }
    problem.constraints.automatic += relations.len();
    problem.constraints.relations.extend(relations);
    report
}

/// board-placer.json's `length_rules`: per net the placer held
/// (`LoweredPlacement::length_rules`), its window and its pins' nearest
/// and farthest octilinear distance across parts at `poses`.
pub(crate) fn length_rule_report(
    lowered: &LoweredPlacement,
    poses: &[core::Pose],
) -> Vec<KiCadLengthRuleReport> {
    let problem = &lowered.problem;
    let round = |value: f64| (value * 1000.0).round() / 1000.0;
    lowered
        .length_rules
        .iter()
        .map(|rule| {
            let id = lowered.net_names.iter().position(|name| *name == rule.net);
            let pins: Vec<(usize, [f64; 2])> = rule
                .parts
                .iter()
                .flat_map(|&index| {
                    let component = &problem.components[index];
                    component
                        .pins
                        .iter()
                        .filter(move |pin| Some(pin.net) == id)
                        .map(move |pin| (index, component.pin_position(pin, poses[index])))
                })
                .collect();
            let (mut closest, mut farthest) = (f64::INFINITY, 0.0f64);
            for (position, &(a, at_a)) in pins.iter().enumerate() {
                for &(b, at_b) in &pins[position + 1..] {
                    if a != b {
                        let distance = core::constraints::octilinear(at_a, at_b);
                        closest = closest.min(distance);
                        farthest = farthest.max(distance);
                    }
                }
            }
            if !closest.is_finite() {
                closest = 0.0;
            }
            let held_apart = rule.floor > 0.0 && !rule.relations.is_empty();
            let held_within = rule.max.is_some() && !rule.relations.is_empty();
            let held = match (held_apart, held_within) {
                (true, true) => "both",
                (true, false) => "apart",
                (false, true) => "together",
                (false, false) => "none",
            };
            let kept = (!held_apart || closest + 1.0e-6 >= rule.floor)
                && (!held_within || farthest <= rule.soft + 1.0e-6);
            let mut note = rule.note.clone();
            if held_apart && !kept && closest + 1.0e-6 < rule.floor {
                note = format!(
                    "its pins lie {:.2} mm apart, closer than half the minimum ({:.2} mm): the route and its meanders may not make up the minimum (give the parts room, or an apart entry, docs/constraints.md)",
                    closest, rule.floor
                );
            } else if held_within && !kept && rule.max.is_some_and(|max| farthest > max) {
                note = format!(
                    "its pins lie {farthest:.2} mm apart, beyond the maximum: no route of the net meets it"
                );
            }
            KiCadLengthRuleReport {
                net: rule.net.clone(),
                min_mm: rule.min,
                max_mm: rule.max,
                apart_mm: held_apart.then(|| round(rule.floor)),
                within_mm: held_within.then(|| round(rule.soft)),
                parts: rule
                    .parts
                    .iter()
                    .map(|&index| lowered.references[index].clone())
                    .collect(),
                closest_mm: round(closest),
                farthest_mm: round(farthest),
                held: held.into(),
                kept,
                note,
            }
        })
        .collect()
}

/// Applies the router's rules the placer keeps (with `router_rules`): the
/// far side under vias in pads and fan-outs (`via_in_pad_far_side`), the
/// room around grid arrays (`bga_room`), the length windows
/// (`length_maxima`, `length_minima`). Returns the arrays' report lines
/// and the length rules held.
pub(super) fn apply_router_rules(
    context: &LowerContext,
    builder: &mut ProblemBuilder,
) -> (Vec<String>, Vec<LoweredLengthRule>) {
    let (pcb, config) = (context.pcb, context.config);
    let pad_offsets = &builder.footprints.pad_offsets;
    let problem = &mut builder.problem;
    let mut array_rooms = Vec::new();
    let mut length_rules = Vec::new();
    if let Some(rules) = &config.router_rules {
        if config.via_in_pad_far_side {
            far_side_of_vias_in_pads(pcb, rules, config, pad_offsets, problem);
            far_side_of_fanouts(pcb, rules, config, pad_offsets, problem);
        }
        if config.bga_room {
            array_rooms = room_around_arrays(pcb, rules, pad_offsets, &builder.references, problem);
        }
        if config.length_maxima || config.length_minima {
            length_rules = length_windows(
                rules,
                config,
                &builder.net_ids,
                builder.footprints.count(),
                problem,
            );
        }
    }
    (array_rooms, length_rules)
}

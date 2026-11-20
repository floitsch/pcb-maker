// Copyright (C) 2026 Toit contributors.

//! The check of the pairs' copper against everything routed
//! (`refused_groups`).

use super::*;
use pcb_router as core;

/// Other nets' copper within this of the pairs' copper is checked with it.
pub(super) const CHECK_REACH: f64 = 2.0;

/// Copper checked together (`refused_groups`): a link's run and vias.
#[derive(Clone)]
pub(super) struct Group {
    pub pair: usize,
    pub segments: [Vec<(usize, Point, Point)>; 2],
    pub vias: [Vec<Point>; 2],
    /// Per polarity: the coupled run's tracks among `segments` (the rest
    /// are the ends' stubs, jogs and crossovers), for `PCB_ROUTER_PAIRS_WHY`.
    pub run: [std::ops::Range<usize>; 2],
    /// Whether its copper is not on the router's route (`LinkCopper::moved`).
    pub moved: bool,
}

/// Per group: why its copper is refused (`None`: kept). The copper of
/// every group is checked at once against the board (`verify`: its
/// obstacles, the edge, each other) and the routes near it; a finding
/// between the two nets of one pair counts only below their gap (KiCad
/// lets them come that close: the class's diff pair rule, or the gap is no
/// less than the clearance).
#[cfg(test)]
pub(super) fn refused_groups(
    board: &core::Board,
    routed: &[core::NetRoute],
    pairs: &[Pair],
    groups: &[Group],
) -> Vec<Option<String>> {
    refused_groups_at(board, routed, pairs, groups, 0.0)
}

/// `refused_groups` with the groups' tracks `width` wide (0: their pairs'
/// width).
pub(super) fn refused_groups_at(
    board: &core::Board,
    routed: &[core::NetRoute],
    pairs: &[Pair],
    groups: &[Group],
    width: f64,
) -> Vec<Option<String>> {
    refused_groups_with_rules(board, routed, pairs, groups, width, None)
}

pub(super) fn refused_groups_with_rules(
    board: &core::Board,
    routed: &[core::NetRoute],
    pairs: &[Pair],
    groups: &[Group],
    width: f64,
    rules: Option<&core::rules::ClassRules>,
) -> Vec<Option<String>> {
    refused_groups_with_pair_gaps(board, routed, pairs, groups, width, rules, &BTreeMap::new())
}

pub(super) fn refused_groups_with_pair_gaps(
    board: &core::Board,
    routed: &[core::NetRoute],
    pairs: &[Pair],
    groups: &[Group],
    width: f64,
    rules: Option<&core::rules::ClassRules>,
    plated_gaps: &BTreeMap<(core::NetId, core::NetId), f64>,
) -> Vec<Option<String>> {
    if groups.is_empty() {
        return Vec::new();
    }
    let reach = rules.map_or(CHECK_REACH, |rules| {
        let copper = board
            .classes
            .iter()
            .flat_map(|class| [class.trace_width, class.via_diameter])
            .fold(width, f64::max);
        CHECK_REACH.max(rules.resolved.largest() + copper)
    });
    let mut routes = routes_near(board, routed, groups, reach);
    let placed = GroupsPlaced::onto(&mut routes, pairs, groups, width);
    let partner: BTreeMap<core::NetId, (core::NetId, f64)> = pairs
        .iter()
        .flat_map(|pair| {
            [
                (pair.positive, (pair.negative, pair.gap)),
                (pair.negative, (pair.positive, pair.gap)),
            ]
        })
        .collect();
    let mut refused: Vec<Option<String>> = vec![None; groups.len()];
    for violation in core::verify::verify_with_pair_gaps(
        board,
        &routes,
        rules,
        &board.attachment.verified_pair_track_gaps,
        plated_gaps,
    ) {
        // Copper at exactly its clearance, but for rounding.
        if violation.actual + 1.0e-6 >= violation.required {
            continue;
        }
        let own = match violation.segment {
            Some(segment) => placed
                .owner_of_segment
                .get(&(violation.net, segment))
                .copied(),
            None => violation
                .via
                .and_then(|index| placed.owner_of_via.get(&(violation.net, index)).copied()),
        };
        let other = violation
            .other_segment
            .and_then(|key| placed.owner_of_segment.get(&key).copied())
            .or_else(|| {
                violation
                    .other_via
                    .and_then(|key| placed.owner_of_via.get(&key).copied())
            });
        if own.is_none() && other.is_none() {
            // Between routes the router made: not the pairs'.
            continue;
        }
        let other_net = violation
            .other_segment
            .or(violation.other_via)
            .map(|(net, _)| net);
        if let (Some(other_net), Some(&(mate, gap))) = (other_net, partner.get(&violation.net))
            && other_net == mate
            && !violation.other.contains("hole")
            && violation.actual + 1.0e-6
                >= gap
                    .min(violation.required)
                    .max(scoped_peer_clearance(rules, &violation, other_net))
        {
            continue;
        }
        let neighbour = if own.is_some() {
            &violation.other
        } else {
            &board.nets[violation.net as usize].name
        };
        let why = format!(
            "its copper is {:.3} mm from {} (needs {:.3})",
            violation.actual.max(0.0),
            neighbour,
            violation.required
        );
        if debug() {
            eprintln!(
                "diff pair copper refused at {:?} layer {} (net {}, segment {:?}): {why}",
                violation.at,
                violation.layer,
                board.nets[violation.net as usize].name,
                violation.segment
            );
        }
        // Only the links' copper (a straight stub's or a meander's check
        // has no run).
        if pairs_why()
            && own
                .or(other)
                .is_some_and(|position| !groups[position].run[0].is_empty())
        {
            refusal_why(board, pairs, groups, &placed, &violation, (own, other));
        }
        // A finding between two groups refuses both, but where one of them
        // lies on the router's route and the other was moved off it
        // (`centrelines`), the moved one only.
        let culprits: Vec<usize> = match (own, other) {
            (Some(a), Some(b)) if a != b && groups[a].moved != groups[b].moved => {
                vec![if groups[a].moved { a } else { b }]
            }
            _ => own.into_iter().chain(other).collect(),
        };
        for position in culprits {
            refused[position].get_or_insert_with(|| why.clone());
        }
    }
    refused
}

fn scoped_peer_clearance(
    rules: Option<&core::rules::ClassRules>,
    violation: &core::Violation,
    other: core::NetId,
) -> f64 {
    let Some(rules) = rules else { return 0.0 };
    let a = rules.resolved.net_group[violation.net as usize];
    let b = rules.resolved.net_group[other as usize];
    let kind = |track| {
        if track {
            core::rules::TRACK
        } else {
            core::rules::VIA
        }
    };
    let own = violation.segment.is_some();
    let other = violation.other_segment.is_some();
    if own || other {
        rules
            .resolved
            .extra_on(a, kind(own), b, kind(other), violation.layer)
    } else {
        rules
            .resolved
            .extra_anywhere(a, core::rules::VIA, b, core::rules::VIA)
    }
}

/// Per net of `board` its routes of `routed` near the groups' copper
/// (within `reach` of its bounds).
fn routes_near(
    board: &core::Board,
    routed: &[core::NetRoute],
    groups: &[Group],
    reach: f64,
) -> Vec<core::NetRoute> {
    // The routes near the groups' copper.
    let mut bounds = core::geometry::Aabb {
        minimum: [f64::INFINITY; 2],
        maximum: [f64::NEG_INFINITY; 2],
    };
    let mut grow = |point: Point| {
        bounds.minimum = [
            bounds.minimum[0].min(point[0]),
            bounds.minimum[1].min(point[1]),
        ];
        bounds.maximum = [
            bounds.maximum[0].max(point[0]),
            bounds.maximum[1].max(point[1]),
        ];
    };
    for group in groups {
        for polarity in 0..2 {
            for &(_, start, end) in &group.segments[polarity] {
                grow(start);
                grow(end);
            }
            for &at in &group.vias[polarity] {
                grow(at);
            }
        }
    }
    let bounds = bounds.inflated(reach);
    let near = |a: Point, b: Point| {
        core::geometry::Aabb {
            minimum: [a[0].min(b[0]), a[1].min(b[1])],
            maximum: [a[0].max(b[0]), a[1].max(b[1])],
        }
        .intersects(bounds)
    };
    let mut routes = vec![core::NetRoute::default(); board.nets.len()];
    for (net, route) in routed.iter().enumerate().take(board.nets.len()) {
        routes[net].segments = route
            .segments
            .iter()
            .filter(|segment| near(segment.start, segment.end))
            .cloned()
            .collect();
        routes[net].vias = route
            .vias
            .iter()
            .filter(|via| near(via.at, via.at))
            .cloned()
            .collect();
    }
    routes
}

/// The groups' copper added to the routes of their nets
/// (`GroupsPlaced::onto`): per routed segment of a group its group and its
/// polarity and place in the group, and per routed via its group.
struct GroupsPlaced {
    owner_of_segment: BTreeMap<(core::NetId, usize), usize>,
    place_of_segment: BTreeMap<(core::NetId, usize), (usize, usize)>,
    owner_of_via: BTreeMap<(core::NetId, usize), usize>,
}

impl GroupsPlaced {
    /// Adds each group's tracks (`width` wide, 0: its pair's width) and
    /// vias to `routes`.
    fn onto(routes: &mut [core::NetRoute], pairs: &[Pair], groups: &[Group], width: f64) -> Self {
        let mut owner_of_segment: BTreeMap<(core::NetId, usize), usize> = BTreeMap::new();
        // Per routed segment of a group: its polarity and place in the group.
        let mut place_of_segment: BTreeMap<(core::NetId, usize), (usize, usize)> = BTreeMap::new();
        let mut owner_of_via = BTreeMap::new();
        for (position, group) in groups.iter().enumerate() {
            let pair = &pairs[group.pair];
            for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
                let route = &mut routes[net as usize];
                for (local, &(layer, start, end)) in group.segments[polarity].iter().enumerate() {
                    owner_of_segment.insert((net, route.segments.len()), position);
                    place_of_segment.insert((net, route.segments.len()), (polarity, local));
                    route.segments.push(core::Segment {
                        layer,
                        start,
                        end,
                        width: if width > 0.0 { width } else { pair.width },
                    });
                }
                for &at in &group.vias[polarity] {
                    owner_of_via.insert((net, route.vias.len()), position);
                    route.vias.push(core::Via {
                        at,
                        diameter: pair.via_diameter,
                        drill: pair.via_drill,
                        span: None,
                    });
                }
            }
        }
        GroupsPlaced {
            owner_of_segment,
            place_of_segment,
            owner_of_via,
        }
    }
}

/// `PCB_ROUTER_PAIRS_WHY`: a finding that refuses a link's copper (its
/// groups `own` and `other`): what of the pair's copper it is on (the run,
/// an end's stub, jog or crossover, a via; else the router's) and by what
/// (`[partner]`: the pair's other net).
fn refusal_why(
    board: &core::Board,
    pairs: &[Pair],
    groups: &[Group],
    placed: &GroupsPlaced,
    violation: &core::Violation,
    (own, other): (Option<usize>, Option<usize>),
) {
    // What of the pair's copper the finding is on: the run, an
    // end's stub, jog or crossover, a via; else the router's.
    let role = |net: core::NetId, segment: Option<usize>, owner: Option<usize>| {
        let Some(position) = owner else {
            return format!("the router's copper of {}", board.nets[net as usize].name);
        };
        let group = &groups[position];
        let name = &board.nets[net as usize].name;
        match segment.and_then(|segment| placed.place_of_segment.get(&(net, segment))) {
            Some(&(polarity, local)) => {
                let run = &group.run[polarity];
                let (_, a, b) = group.segments[polarity][local];
                let place = if run.is_empty() {
                    "a stub"
                } else if local < run.start {
                    "end a's stub, jog or crossover"
                } else if local < run.end {
                    "the run"
                } else {
                    "end b's stub, jog or crossover"
                };
                format!(
                    "{name}'s {place} ({:.3}, {:.3}) - ({:.3}, {:.3})",
                    a[0], a[1], b[0], b[1]
                )
            }
            None => format!("{name}'s via"),
        }
    };
    let other_text = match (violation.other_segment, violation.other_via) {
        (Some((net, segment)), _) => role(net, Some(segment), other),
        (_, Some((net, _))) if other.is_some() => role(net, None, other),
        _ => violation.other.clone(),
    };
    let pair = own.or(other).map(|position| &pairs[groups[position].pair]);
    let partner_finding = pair.is_some_and(|pair| {
        let names = [pair.positive, pair.negative].map(|net| &board.nets[net as usize].name);
        let own_pair = names.contains(&&board.nets[violation.net as usize].name);
        own_pair
            && names
                .iter()
                .any(|name| violation.other.contains(name.as_str()))
    });
    eprintln!(
        "pairs why: refused {}: at ({:.3}, {:.3}) layer {}: {} is {:.3} mm from {} (needs {:.3}){}",
        pair.map_or(String::new(), |pair| board.nets[pair.positive as usize]
            .name
            .clone()),
        violation.at[0],
        violation.at[1],
        violation.layer,
        role(violation.net, violation.segment, own),
        violation.actual,
        other_text,
        violation.required,
        if partner_finding { " [partner]" } else { "" }
    );
}

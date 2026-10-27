// Copyright (C) 2026 Toit contributors.

//! Length and skew rules of single nets and groups of nets (docs/router.md,
//! "Length and skew rules").
//!
//! The project's custom rules' `length` and `skew` constraints reach the
//! router as `KiCadLengthRule`s: per rule the nets it holds for and, per
//! net, the layers (and whether the vias) whose copper KiCad counts for it.
//! After the route (`match_lengths`, when an attempt is written, after the
//! differential pairs), each net's length is measured from its copper as
//! KiCad counts it (`kicad_length`: tracks, the pads' insides as straight
//! lines, vias at the stackup's height); a
//! net shorter than a `length` minimum, or further than a `skew` maximum
//! below the longest net of its rule's group, gets a meander on a straight
//! track of its route: bumps of at most `MAX_AMPLITUDE`, their height
//! bisected so that they add what it lacks, the longest tracks first, the
//! first placement whose copper keeps every clearance (`core::verify`
//! against everything routed and every obstacle). The meander's tracks are
//! the route's and are written as a KiCad tuning pattern
//! (`write_length_patterns`). A `length` maximum is no hard bound: its nets
//! are length-critical to the search (`route::length_bounded_nets`, the
//! core's `Config::length_bounded`), and a net longer than it is reported.
//! A layout's own routes get the same (`finish_lengths`). The verdict's
//! `lengths` lists per rule
//! and net the length as the attempt writes it, the target, the bounds and
//! what the meander added.

use super::meander::{
    Point, Tuned, Tuning, bump_adds, bump_height, bumps, copper_heights, polyline_length,
    tuning_pattern_expr,
};
use super::*;
use pcb_router as core;

/// One net of a length or skew rule after the route (the verdict's
/// `lengths`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadNetLength {
    pub net: String,
    /// The custom rule and its constraint (`length` or `skew`).
    pub rule: String,
    pub constraint: String,
    /// The net's length as KiCad counts it for the rule (the tracks on the
    /// layers the rule holds on, vias at the stackup's height), from the
    /// written board, millimetres.
    pub length_mm: f64,
    /// The length the meander aims at: a `length` rule's opt (held inside
    /// its bounds), else the middle of its bounds, else its minimum; a
    /// `skew` rule's longest net of the group less half the maximum.
    pub target_mm: f64,
    /// The bounds KiCad holds the net to: a `length` rule's min and max; a
    /// `skew` rule's longest net less the maximum, and the longest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_mm: Option<f64>,
    /// What the meanders added (0: none).
    pub meander_mm: f64,
    /// Whether the length lies within the bounds (KiCad then has no
    /// finding for it).
    pub within: bool,
    /// Why it is not matched, when it is not.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// A single net's meander, written as a KiCad tuning pattern
/// (`write_length_patterns`); its tracks are the route's.
#[derive(Clone, Debug)]
pub(crate) struct LengthPattern {
    net: core::NetId,
    width: f64,
    layer: usize,
    points: Vec<Point>,
    amplitude: f64,
    spacing: f64,
    added: f64,
    /// The target and the bounds (min, max) of the rule.
    target: [f64; 3],
}

/// A meander's bumps are at most this high (KiCad's single track default
/// `max_amplitude`), or three track widths.
const MAX_AMPLITUDE: f64 = 1.0;
/// Other copper within this of a meander is checked with it.
const CHECK_REACH: f64 = 2.0;
/// Tracks tried per net, and placements along one track.
const MAX_TRACKS: usize = 200;
const POSITIONS: usize = 12;
/// A length short of its target by less than this needs no meander.
const ENOUGH: f64 = 0.005;
/// A length this close to a bound is on it.
const ON_BOUND: f64 = 1.0e-6;

/// The settings with the length and skew rules the project has reported
/// as kept (`board-router.json`'s `rules.custom_rules`): a `skew` and a
/// `length` minimum `applied`, a `length` with a maximum `approximated`
/// (the maximum is checked: its nets are length-critical to the search, no
/// hard bound). Unchanged where the
/// project has no such rule or `length_matching` is off.
pub(crate) fn with_lengths(config: &KiCadBoardRouterConfig) -> KiCadBoardRouterConfig {
    let mut copy = config.clone();
    if config.length_rules.is_empty() || !config.length_matching_on() {
        return copy;
    }
    let used: BTreeMap<(&str, &str), bool> = config
        .length_rules
        .iter()
        .map(|rule| {
            (
                (rule.rule.as_str(), rule.constraint.as_str()),
                rule.constraint == "length" && rule.max.is_some(),
            )
        })
        .collect();
    if let Some(report) = copy.project_rules.as_mut() {
        for entry in &mut report.custom_rules {
            let Some(&bounded) = used.get(&(entry.rule.as_str(), entry.constraint.as_str())) else {
                continue;
            };
            if entry.status != "unmodelled" {
                continue;
            }
            if entry.constraint == "skew" {
                entry.status = "applied".into();
                entry.detail = "length matching (length_matching): the nets matched to the longest of the rule by meanders after the route".into();
            } else if bounded {
                entry.status = "approximated".into();
                entry.detail = "length matching (length_matching): a net below the minimum gets a meander after the route; a net under the maximum is length-critical to the search (a share of the congestion price, rerouted after the others), the maximum checked (the verdict's lengths), no hard bound".into();
            } else {
                entry.status = "applied".into();
                entry.detail = "length matching (length_matching): a net below the minimum gets a meander after the route".into();
            }
            let prefix = format!("{} ('{}'", entry.constraint, entry.rule);
            report.unmodelled.retain(|line| !line.starts_with(&prefix));
        }
    }
    copy
}

/// The target and the bounds (min, max) of a `length` rule.
fn length_target(rule: &KiCadLengthRule) -> (f64, Option<f64>, Option<f64>) {
    let (min, max) = (rule.min, rule.max);
    let target = match (rule.opt, min, max) {
        (Some(opt), _, _) => opt
            .max(min.unwrap_or(f64::NEG_INFINITY))
            .min(max.unwrap_or(f64::INFINITY)),
        (None, Some(min), Some(max)) => (min + max) / 2.0,
        (None, Some(min), None) => min,
        (None, None, Some(max)) => max,
        (None, None, None) => 0.0,
    };
    (target, min, max)
}

/// The board's copper of each net that is not the route's (the board's
/// own tracks and vias, the pairs' copper: obstacles `existing track` and
/// `existing via`), as tracks (layer, start, end) and vias.
fn fixed_copper(board: &core::Board) -> BTreeMap<core::NetId, (Vec<Track>, Vec<Point>)> {
    let mut copper: BTreeMap<core::NetId, (Vec<Track>, Vec<Point>)> = BTreeMap::new();
    for obstacle in &board.obstacles {
        let Some(net) = obstacle.net else { continue };
        match (&obstacle.shape, obstacle.label.as_str()) {
            (core::Shape::Capsule { start, end, .. }, "existing track")
                if obstacle.layers.count_ones() == 1 =>
            {
                let layer = obstacle.layers.trailing_zeros() as usize;
                copper.entry(net).or_default().0.push((layer, *start, *end));
            }
            (core::Shape::Circle { center, .. }, "existing via") => {
                copper.entry(net).or_default().1.push(*center);
            }
            _ => {}
        }
    }
    copper
}

type Track = (usize, Point, Point);

/// A pad of a net: its layers, its copper and its position (where KiCad
/// starts the net's length).
struct Pad<'a> {
    layers: core::LayerMask,
    shape: &'a core::Shape,
    at: Point,
}

/// A net's length as KiCad counts it for a rule (KiCad 10's length
/// calculation, as kicad-cli 10.0.7 measures KiCad's QA board
/// length_calculations): its tracks on the layers the rule counts
/// (`counts`), where the tracks run inside a pad that they leave once the
/// straight line from the pad's position to where they leave it instead,
/// and with `vias` each via at the height between the outermost layers of
/// what it joins (the tracks that end on it, the pads it lies in).
fn kicad_length(
    tracks: &[Track],
    vias: &[Point],
    pads: &[Pad],
    heights: &[f64],
    counts: impl Fn(usize) -> bool,
) -> f64 {
    let mut length = 0.0;
    for &(layer, start, end) in tracks {
        if counts(layer) {
            length += core::geometry::distance(start, end);
        }
    }
    // Inside the pads.
    for pad in pads {
        for layer in (0..32).filter(|&layer| pad.layers & (1 << layer) != 0 && counts(layer)) {
            let mut inside = 0.0;
            let mut exits: Vec<Point> = Vec::new();
            for &(_, start, end) in tracks.iter().filter(|track| track.0 == layer) {
                let (a, b) = (pad.shape.contains(start), pad.shape.contains(end));
                match (a, b) {
                    (true, true) => inside += core::geometry::distance(start, end),
                    (true, false) | (false, true) => {
                        let (from, to) = if a { (start, end) } else { (end, start) };
                        // Bisected to where the track leaves the pad.
                        let (mut low, mut high) = (0.0, 1.0);
                        for _ in 0..40 {
                            let middle = (low + high) / 2.0;
                            let point = [
                                from[0] + (to[0] - from[0]) * middle,
                                from[1] + (to[1] - from[1]) * middle,
                            ];
                            if pad.shape.contains(point) {
                                low = middle;
                            } else {
                                high = middle;
                            }
                        }
                        let exit = [
                            from[0] + (to[0] - from[0]) * low,
                            from[1] + (to[1] - from[1]) * low,
                        ];
                        inside += core::geometry::distance(from, exit);
                        exits.push(exit);
                    }
                    (false, false) => {}
                }
            }
            if exits.len() == 1 {
                let direct = core::geometry::distance(pad.at, exits[0]);
                if direct < inside {
                    length -= inside - direct;
                }
            }
        }
    }
    for &via in vias {
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        let mut reach = |layer: usize| {
            if let Some(&z) = heights.get(layer) {
                low = low.min(z);
                high = high.max(z);
            }
        };
        for &(layer, start, end) in tracks {
            if core::geometry::distance(start, via) < 1.0e-4
                || core::geometry::distance(end, via) < 1.0e-4
            {
                reach(layer);
            }
        }
        for pad in pads.iter().filter(|pad| pad.shape.contains(via)) {
            for layer in (0..32).filter(|&layer| pad.layers & (1 << layer) != 0) {
                reach(layer);
            }
        }
        if high > low {
            length += high - low;
        }
    }
    length
}

/// After the route, before the attempt is written: the meanders the
/// length and skew rules ask for, added to the routes (their tracks), and
/// per rule and net what came of it: the lengths of the routes so
/// finished, the bounds (a skew rule's from the longest net of its group),
/// whether each lies within them. `coupled`: the nets of pairs with a
/// coupled link, which the pairs' own pass matches (a meander on one track
/// of a coupled run would part the pair).
pub(crate) fn match_lengths(
    board: &core::Board,
    result: &mut core::RoutingResult,
    config: &KiCadBoardRouterConfig,
    heights: &[f64],
    layer_names: &[String],
    coupled: &BTreeSet<String>,
) -> (Vec<LengthPattern>, Vec<KiCadNetLength>) {
    if config.length_rules.is_empty() || !config.length_matching_on() {
        return (Vec::new(), Vec::new());
    }
    let ids: BTreeMap<&str, core::NetId> = board
        .nets
        .iter()
        .enumerate()
        .map(|(id, net)| (net.name.as_str(), id as core::NetId))
        .collect();
    let fixed = fixed_copper(board);
    let pads_of = |id: core::NetId| -> Vec<Pad> {
        let mut seen = BTreeSet::new();
        board.nets[id as usize]
            .terminals
            .iter()
            .filter(|terminal| seen.insert(terminal.pad))
            .filter_map(|terminal| {
                let obstacle = board.obstacles.get(terminal.pad)?;
                Some(Pad {
                    layers: obstacle.layers,
                    shape: &obstacle.shape,
                    at: terminal.anchor,
                })
            })
            .collect()
    };
    let mut patterns: Vec<LengthPattern> = Vec::new();
    let mut report: Vec<KiCadNetLength> = Vec::new();
    // Per rule's member: the layers it counts, by the router's index.
    let counted_of = |member: &KiCadLengthMember| -> Vec<bool> {
        layer_names
            .iter()
            .map(|name| member.layers.iter().any(|layer| layer == name))
            .collect()
    };
    let length_of = |member: &KiCadLengthMember, routes: &[core::NetRoute]| -> f64 {
        let Some(&id) = ids.get(member.net.as_str()) else {
            return 0.0;
        };
        let route = &routes[id as usize];
        let mut tracks: Vec<Track> = route
            .segments
            .iter()
            .map(|segment| (segment.layer, segment.start, segment.end))
            .collect();
        let mut vias: Vec<Point> = route.vias.iter().map(|via| via.at).collect();
        if let Some((own_tracks, own_vias)) = fixed.get(&id) {
            tracks.extend(own_tracks.iter().copied());
            vias.extend(own_vias.iter().copied());
        }
        if !member.vias {
            vias.clear();
        }
        let counted = counted_of(member);
        kicad_length(&tracks, &vias, &pads_of(id), heights, |layer| {
            counted.get(layer).copied().unwrap_or(false)
        })
    };
    // The groups a rule compares: a length rule's nets one by one, a skew
    // rule's all together (or each differential pair's two nets).
    let groups_of = |rule: &KiCadLengthRule| -> Vec<Vec<usize>> {
        if rule.constraint == "length" {
            (0..rule.nets.len())
                .map(|position| vec![position])
                .collect()
        } else if rule.within_diff_pairs {
            let mut by_base: BTreeMap<String, Vec<usize>> = BTreeMap::new();
            for (position, member) in rule.nets.iter().enumerate() {
                if let Some((_, base, _)) = crate::project_rules::diff_pair_of(&member.net) {
                    by_base.entry(base).or_default().push(position);
                }
            }
            by_base
                .into_values()
                .filter(|group| group.len() == 2)
                .collect()
        } else {
            vec![(0..rule.nets.len()).collect()]
        }
    };
    // The target and bounds of a group with these lengths.
    let bounds_of = |rule: &KiCadLengthRule, lengths: &[f64]| {
        if rule.constraint == "length" {
            length_target(rule)
        } else {
            let longest = lengths.iter().copied().fold(0.0, f64::max);
            let tolerance = rule.max.unwrap_or(0.0);
            (
                longest - tolerance / 2.0,
                Some(longest - tolerance),
                Some(longest),
            )
        }
    };
    // Length rules first: a skew then compares the lengths they made.
    let mut order: Vec<&KiCadLengthRule> = config.length_rules.iter().collect();
    order.sort_by_key(|rule| rule.constraint != "length");
    for rule in order {
        for group in groups_of(rule) {
            let lengths: Vec<f64> = group
                .iter()
                .map(|&position| length_of(&rule.nets[position], &result.routes))
                .collect();
            let (target, min, max) = bounds_of(rule, &lengths);
            for (&position, &length) in group.iter().zip(&lengths) {
                let member = &rule.nets[position];
                let mut entry = KiCadNetLength {
                    net: member.net.clone(),
                    rule: rule.rule.clone(),
                    constraint: rule.constraint.clone(),
                    length_mm: length,
                    target_mm: target,
                    min_mm: min,
                    max_mm: max,
                    ..KiCadNetLength::default()
                };
                let short = min.is_some_and(|min| length < min - ON_BOUND);
                let id = ids.get(member.net.as_str()).copied();
                if length <= 0.0 {
                    entry.note = "no copper the rule counts (not routed)".into();
                } else if short && coupled.contains(&member.net) {
                    entry.note = "a coupled pair's net: the pairs' pass matches it".into();
                } else if short
                    && id.is_some_and(|id| result.status[id as usize] != core::NetStatus::Routed)
                {
                    entry.note = "not completely routed: no meander".into();
                } else if short && let Some(id) = id {
                    let need = target - length;
                    let (added, made) = meander_net(
                        board,
                        &mut result.routes,
                        id,
                        need,
                        &counted_of(member),
                        [target, min.unwrap_or(0.0), max.unwrap_or(f64::INFINITY)],
                    );
                    entry.meander_mm = added;
                    patterns.extend(made);
                    if need - added > ENOUGH {
                        entry.note = format!(
                            "no straight track with room for all of it ({added:.3} of {need:.3} mm added)"
                        );
                    }
                }
                report.push(entry);
            }
        }
    }
    // The lengths as the routes are written, a skew rule's bounds from its
    // group's longest net then.
    for rule in &config.length_rules {
        for group in groups_of(rule) {
            let lengths: Vec<f64> = group
                .iter()
                .map(|&position| length_of(&rule.nets[position], &result.routes))
                .collect();
            let (target, min, max) = bounds_of(rule, &lengths);
            for (&position, &length) in group.iter().zip(&lengths) {
                let net = &rule.nets[position].net;
                let Some(entry) = report.iter_mut().find(|entry| {
                    &entry.net == net
                        && entry.rule == rule.rule
                        && entry.constraint == rule.constraint
                }) else {
                    continue;
                };
                entry.length_mm = length;
                entry.target_mm = target;
                entry.min_mm = min;
                entry.max_mm = max;
                entry.within = length > 0.0
                    && min.is_none_or(|min| length >= min - ON_BOUND)
                    && max.is_none_or(|max| length <= max + ON_BOUND);
                if !entry.within
                    && entry.note.is_empty()
                    && max.is_some_and(|max| length > max + ON_BOUND)
                {
                    entry.note = if rule.constraint == "length" {
                        "longer than the maximum: the search does not bound a net's length".into()
                    } else {
                        "longer than the group's longest less the skew".into()
                    };
                }
            }
        }
    }
    (patterns, report)
}

/// Adds meanders of `need` millimetres in all to the route of `net`, on
/// its straight tracks on the layers the rule counts (`counted`), the
/// longest first; returns what they add and the patterns.
fn meander_net(
    board: &core::Board,
    routes: &mut [core::NetRoute],
    net: core::NetId,
    need: f64,
    counted: &[bool],
    target: [f64; 3],
) -> (f64, Vec<LengthPattern>) {
    let clearance = board.classes[board.nets[net as usize].class].clearance;
    let mut patterns = Vec::new();
    let mut added = 0.0;
    // The straight tracks, the longest first (by their geometry: the route
    // changes as meanders split them).
    let mut candidates: Vec<core::Segment> = routes[net as usize]
        .segments
        .iter()
        .filter(|segment| counted.get(segment.layer).copied().unwrap_or(false))
        .cloned()
        .collect();
    candidates.sort_by(|x, y| {
        core::geometry::distance(y.start, y.end)
            .total_cmp(&core::geometry::distance(x.start, x.end))
    });
    // The board near the net's route (the check's obstacles).
    let mut bounds = core::geometry::Aabb {
        minimum: [f64::INFINITY; 2],
        maximum: [f64::NEG_INFINITY; 2],
    };
    for segment in &candidates {
        for point in [segment.start, segment.end] {
            bounds.minimum = [
                bounds.minimum[0].min(point[0]),
                bounds.minimum[1].min(point[1]),
            ];
            bounds.maximum = [
                bounds.maximum[0].max(point[0]),
                bounds.maximum[1].max(point[1]),
            ];
        }
    }
    if candidates.is_empty() {
        return (0.0, patterns);
    }
    let reach = MAX_AMPLITUDE.max(3.0 * candidates[0].width) + CHECK_REACH;
    let bounds = bounds.inflated(reach);
    let mut near = board.clone();
    near.obstacles = board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.shape.aabb().intersects(bounds))
        .cloned()
        .collect();
    // A work list: what is left of a track a meander split is tried again.
    let mut tries = 0;
    while let Some(segment) = (!candidates.is_empty()).then(|| candidates.remove(0)) {
        let remaining = need - added;
        tries += 1;
        if remaining < ENOUGH || tries > MAX_TRACKS {
            break;
        }
        let w = segment.width;
        let (a, b) = (segment.start, segment.end);
        let length = core::geometry::distance(a, b);
        let spacing = (w + clearance).max(2.0 * w);
        let margin = spacing;
        let along = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
        let left = [-along[1], along[0]];
        let top = MAX_AMPLITUDE.max(3.0 * w);
        let room = ((length - 2.0 * margin) / spacing + 1.0) / 2.0;
        if room < 1.0 {
            continue;
        }
        let mut placed: Option<(Vec<Point>, f64)> = None;
        'search: for cap in [top, top / 2.0, top / 4.0] {
            let most = ((remaining / bump_adds(cap, spacing)).ceil().max(1.0) as usize)
                .min(room.floor() as usize);
            // As many bumps as hold the length, else fewer (the rest on
            // other tracks).
            let mut counts = vec![most];
            for fewer in [most / 2, most / 4, 1] {
                if fewer >= 1 && !counts.contains(&fewer) {
                    counts.push(fewer);
                }
            }
            for count in counts {
                let want = remaining.min(count as f64 * bump_adds(cap, spacing));
                let height = bump_height(want, count, spacing, cap);
                let span = (2 * count - 1) as f64 * spacing;
                let free = length - 2.0 * margin - span;
                // Along the track: its start, its end, its middle, and
                // evenly between.
                let mut froms = vec![margin, margin + free, margin + free / 2.0];
                let steps = ((free / spacing).floor() as usize).min(POSITIONS);
                for step in 1..steps {
                    froms.push(margin + free * step as f64 / steps as f64);
                }
                for side in [1.0, -1.0] {
                    let outward = [left[0] * side, left[1] * side];
                    for &from in &froms {
                        let points = bumps((a, b), outward, from, count, height, spacing);
                        let (first, last) = (points[0], points[points.len() - 1]);
                        if joined_between(board, &routes[net as usize], net, &segment, first, last)
                        {
                            continue;
                        }
                        let pieces: Vec<(Point, Point)> =
                            points.windows(2).map(|pair| (pair[0], pair[1])).collect();
                        if refused(&near, routes, net, segment.layer, w, &pieces) {
                            continue;
                        }
                        placed = Some((points, height));
                        break 'search;
                    }
                }
            }
        }
        let Some((points, height)) = placed else {
            continue;
        };
        // The track splits around the meander.
        let route = &mut routes[net as usize];
        let Some(position) = route
            .segments
            .iter()
            .position(|other| other.layer == segment.layer && other.start == a && other.end == b)
        else {
            continue;
        };
        let (first, last) = (points[0], points[points.len() - 1]);
        let piece = |start: Point, end: Point| core::Segment {
            layer: segment.layer,
            start,
            end,
            width: w,
        };
        let mut replaced = vec![piece(a, first)];
        replaced.extend(points.windows(2).map(|pair| piece(pair[0], pair[1])));
        replaced.push(piece(last, b));
        route.segments.splice(position..=position, replaced);
        let gained = polyline_length(&points) - core::geometry::distance(first, last);
        added += gained;
        patterns.push(LengthPattern {
            net,
            width: w,
            layer: segment.layer,
            points,
            amplitude: height,
            spacing,
            added: gained,
            target,
        });
        // What is left of the track either side, the longest first.
        for rest in [piece(a, first), piece(last, b)] {
            let length = core::geometry::distance(rest.start, rest.end);
            let at = candidates
                .iter()
                .position(|other| core::geometry::distance(other.start, other.end) < length)
                .unwrap_or(candidates.len());
            candidates.insert(at, rest);
        }
    }
    (added, patterns)
}

/// Whether other copper of `net` joins the track `segment` between
/// `first` and `last` (a branch, a via, a pad: the meander would part it
/// from the track).
fn joined_between(
    board: &core::Board,
    route: &core::NetRoute,
    net: core::NetId,
    segment: &core::Segment,
    first: Point,
    last: Point,
) -> bool {
    let reach = segment.width;
    let touches = |point: Point| core::geometry::point_segment_distance(point, first, last) < reach;
    route.segments.iter().any(|other| {
        !(other.layer == segment.layer && other.start == segment.start && other.end == segment.end)
            && (touches(other.start) || touches(other.end))
    }) || route.vias.iter().any(|via| {
        core::geometry::point_segment_distance(via.at, first, last) < reach + via.diameter / 2.0
    }) || board.obstacles.iter().any(|obstacle| {
        obstacle.net == Some(net)
            && obstacle.layers & (1 << segment.layer) != 0
            && obstacle.shape.distance_to_segment(first, last) < segment.width / 2.0
    })
}

/// Whether a meander's tracks (`pieces` on `layer`, `width` wide, of
/// `net`) come closer than a clearance to anything (`core::verify` on the
/// board near the net and the routes near the meander).
fn refused(
    near: &core::Board,
    routes: &[core::NetRoute],
    net: core::NetId,
    layer: usize,
    width: f64,
    pieces: &[(Point, Point)],
) -> bool {
    let mut bounds = core::geometry::Aabb {
        minimum: [f64::INFINITY; 2],
        maximum: [f64::NEG_INFINITY; 2],
    };
    for &(start, end) in pieces {
        for point in [start, end] {
            bounds.minimum = [
                bounds.minimum[0].min(point[0]),
                bounds.minimum[1].min(point[1]),
            ];
            bounds.maximum = [
                bounds.maximum[0].max(point[0]),
                bounds.maximum[1].max(point[1]),
            ];
        }
    }
    let bounds = bounds.inflated(CHECK_REACH);
    let close = |a: Point, b: Point| {
        core::geometry::Aabb {
            minimum: [a[0].min(b[0]), a[1].min(b[1])],
            maximum: [a[0].max(b[0]), a[1].max(b[1])],
        }
        .intersects(bounds)
    };
    let mut nearby = vec![core::NetRoute::default(); routes.len()];
    for (index, route) in routes.iter().enumerate() {
        nearby[index].segments = route
            .segments
            .iter()
            .filter(|segment| close(segment.start, segment.end))
            .cloned()
            .collect();
        nearby[index].vias = route
            .vias
            .iter()
            .filter(|via| close(via.at, via.at))
            .cloned()
            .collect();
    }
    let base = nearby[net as usize].segments.len();
    nearby[net as usize]
        .segments
        .extend(pieces.iter().map(|&(start, end)| core::Segment {
            layer,
            start,
            end,
            width,
        }));
    core::verify(near, &nearby).iter().any(|violation| {
        violation.actual + 1.0e-6 < violation.required
            && ((violation.net == net && violation.segment.is_some_and(|index| index >= base))
                || violation
                    .other_segment
                    .is_some_and(|(other, index)| other == net && index >= base))
    })
}

/// `match_lengths` for a board written outside an attempt (the layout's
/// own routes): the stackup's heights from `pcb`, no coupled pairs (the
/// layout routes no pair as coupled runs).
pub(crate) fn finish_lengths(
    pcb: &Expr,
    board: &core::Board,
    result: &mut core::RoutingResult,
    config: &KiCadBoardRouterConfig,
    layer_names: &[String],
) -> (Vec<LengthPattern>, Vec<KiCadNetLength>) {
    if config.length_rules.is_empty() || !config.length_matching_on() {
        return (Vec::new(), Vec::new());
    }
    let heights = copper_heights(pcb);
    let heights: Vec<f64> = layer_names
        .iter()
        .map(|name| heights.get(name).copied().unwrap_or(0.0))
        .collect();
    match_lengths(
        board,
        result,
        config,
        &heights,
        layer_names,
        &BTreeSet::new(),
    )
}

/// Writes the meanders' tuning patterns into `pcb` (after the routes: the
/// patterns' members are the route's tracks of the meanders, named by the
/// uuids their written segments have).
pub(crate) fn write_length_patterns(
    pcb: &mut Expr,
    board: &core::Board,
    patterns: &[LengthPattern],
    layer_names: &[String],
) -> Result<(), String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    for pattern in patterns {
        let name = &board.nets[pattern.net as usize].name;
        let layer = layer_names[pattern.layer].clone();
        let members: Vec<Expr> = pattern
            .points
            .windows(2)
            .filter_map(|pair| {
                let segment = supplemental_segment_expr(&SupplementalSegment {
                    connection: name.clone(),
                    start: nanometres(pair[0]),
                    end: nanometres(pair[1]),
                    width: pattern.width,
                    layer: layer.clone(),
                });
                segment
                    .child("uuid")
                    .and_then(|uuid| uuid.children().get(1))
                    .and_then(Expr::atom)
                    .map(|uuid| Expr::Atom(quote(uuid)))
            })
            .collect();
        let [target, min, max] = pattern.target;
        let tuned = Tuned {
            net: name,
            width: pattern.width,
            points: &pattern.points,
            amplitude: pattern.amplitude,
            spacing: pattern.spacing,
            added: pattern.added,
            tuning: Tuning::Single {
                target,
                min,
                max: if max.is_finite() { max } else { target },
            },
        };
        items.push(tuning_pattern_expr(&tuned, &layer, members));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two nets from J1 to U1 and U2: /A 30 mm, /B 15 mm, both straight.
    const BOARD: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6)) (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 40 20) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
  (footprint "test" (layer "F.Cu") (at 4 5)
    (property "Reference" "J1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.5) (layers "F.Cu") (net "/A"))
    (pad "2" smd rect (at 0 8) (size 1 0.5) (layers "F.Cu") (net "/B")))
  (footprint "test" (layer "F.Cu") (at 34 5)
    (property "Reference" "U1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.5) (layers "F.Cu") (net "/A")))
  (footprint "test" (layer "F.Cu") (at 19 13)
    (property "Reference" "U2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.5) (layers "F.Cu") (net "/B"))))
"#;

    fn member(net: &str) -> KiCadLengthMember {
        KiCadLengthMember {
            net: net.into(),
            layers: vec!["F.Cu".into(), "B.Cu".into()],
            vias: true,
        }
    }

    /// KiCad's QA board length_calculations (kicad-cli 10.0.7 holds each
    /// case's net within 1 µm of its rule): a via in an SMD pad counts from
    /// the pad's layer, a track's wander inside a pad counts as the straight
    /// line from the pad's position to where it leaves.
    #[test]
    fn lengths_count_as_kicads_qa_board_measures_them() {
        // Copper 0.035 mm, dielectrics 5 mm: F.Cu, In1.Cu, In2.Cu, B.Cu.
        let heights = [0.0, 5.0525, 10.0875, 15.14];
        let all = |_: usize| true;
        let pad_shape = core::Shape::rectangle([0.0, 0.0], [0.5, 0.5], 0.0);
        let pad = Pad {
            layers: 1,
            shape: &pad_shape,
            at: [0.0, 0.0],
        };
        // CASE_2: the pad on F.Cu, a via in it, 13.335 mm on B.Cu.
        let tracks = [(3, [0.0, 0.0], [13.335, 0.0])];
        let length = kicad_length(
            &tracks,
            &[[0.0, 0.0]],
            std::slice::from_ref(&pad),
            &heights,
            all,
        );
        assert!((length - 28.475).abs() < 1.0e-9, "{length}");
        // CASE_7 and CASE_8: F.Cu, In1.Cu, In2.Cu and back to F.Cu (or on
        // to B.Cu), a via at each change.
        let path = |last: usize| {
            [
                (0, [0.0, 0.0], [3.0, 0.0]),
                (1, [3.0, 0.0], [6.0, 0.0]),
                (2, [6.0, 0.0], [9.0, 0.0]),
                (last, [9.0, 0.0], [13.335, 0.0]),
            ]
        };
        let vias = [[3.0, 0.0], [6.0, 0.0], [9.0, 0.0]];
        let back = kicad_length(&path(0), &vias, &[], &heights, all);
        assert!((back - 33.51).abs() < 1.0e-9, "{back}");
        let through = kicad_length(&path(3), &vias, &[], &heights, all);
        assert!((through - 28.475).abs() < 1.0e-9, "{through}");
        // CASE_5: into a 1.025 x 1.4 mm pad's centre by a jog: 13.6942 mm
        // of track, 13.547 to KiCad (whose pad is rounded: within 5 µm).
        let pad_shape = core::Shape::rectangle([135.636, 70.485], [0.5125, 0.7], 0.0);
        let pad = Pad {
            layers: 1,
            shape: &pad_shape,
            at: [135.636, 70.485],
        };
        let tracks = [
            (0, [135.382, 69.977], [122.301, 69.977]),
            (0, [135.636, 70.231], [135.382, 69.977]),
            (0, [135.636, 70.485], [135.636, 70.231]),
        ];
        let length = kicad_length(&tracks, &[], std::slice::from_ref(&pad), &heights, all);
        assert!((length - 13.547).abs() < 0.005, "{length}");
        // A rule on some layers only counts their tracks.
        let inner = kicad_length(&path(0), &[], &[], &heights, |layer| layer == 1);
        assert!((inner - 3.0).abs() < 1.0e-9);
    }

    /// The layout writes its own routes outside an attempt: the same
    /// meanders as an attempt's (`finish_lengths`), the heights from the
    /// board; and the nets a maximum bounds search near their shortest way
    /// (`length_bounded_nets`), with `length_matching` only.
    #[test]
    fn the_layouts_routes_get_the_attempts_meanders_and_bounded_nets_their_price() {
        let pcb = parse(BOARD).unwrap();
        let rule = |constraint: &str, max: Option<f64>, net: &str| KiCadLengthRule {
            rule: format!("{constraint} {net}"),
            constraint: constraint.into(),
            min: (constraint == "length").then_some(33.0),
            max,
            nets: vec![member(net)],
            ..KiCadLengthRule::default()
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            length_rules: vec![
                rule("length", Some(34.0), "A"),
                rule("skew", Some(0.5), "B"),
            ],
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, true).unwrap().board;
        let names: Vec<String> = vec!["F.Cu".into(), "B.Cu".into()];
        let mut written = core::route(&board, &core_config(&config));
        let mut attempt = written.clone();
        let (patterns, lengths) = finish_lengths(&pcb, &board, &mut written, &config, &names);
        let heights = copper_heights(&pcb);
        let heights: Vec<f64> = names
            .iter()
            .map(|name| heights.get(name).copied().unwrap_or(0.0))
            .collect();
        let (expected, expected_lengths) = match_lengths(
            &board,
            &mut attempt,
            &config,
            &heights,
            &names,
            &BTreeSet::new(),
        );
        assert_eq!(lengths, expected_lengths);
        assert_eq!(patterns.len(), expected.len());
        let a = lengths.iter().find(|entry| entry.net == "A").unwrap();
        assert!(a.within && a.meander_mm > 2.0, "{a:?}");
        // Only a `length` rule with a maximum bounds its nets.
        let bounded = crate::board_router::length_bounded_nets(&config).unwrap();
        assert_eq!(bounded.iter().collect::<Vec<_>>(), vec!["A"]);
        assert_eq!(
            core_config(&config).length_bounded.as_deref(),
            Some(&*bounded)
        );
        let off = KiCadBoardRouterConfig {
            length_matching: Some(false),
            ..config.clone()
        };
        assert!(crate::board_router::length_bounded_nets(&off).is_none());
        let (patterns, lengths) = finish_lengths(&pcb, &board, &mut written, &off, &names);
        assert!(patterns.is_empty() && lengths.is_empty());
    }

    #[test]
    fn meanders_lengthen_a_net_to_its_rule_and_the_skew_to_the_longest() {
        let mut pcb = parse(BOARD).unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            length_rules: vec![
                KiCadLengthRule {
                    rule: "long".into(),
                    constraint: "length".into(),
                    min: Some(33.0),
                    max: Some(34.0),
                    nets: vec![member("A")],
                    ..KiCadLengthRule::default()
                },
                KiCadLengthRule {
                    rule: "matched".into(),
                    constraint: "skew".into(),
                    max: Some(0.5),
                    nets: vec![member("A"), member("B")],
                    ..KiCadLengthRule::default()
                },
            ],
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, true).unwrap().board;
        let mut result = core::route(&board, &core_config(&config));
        let names: Vec<String> = vec!["F.Cu".into(), "B.Cu".into()];
        let (patterns, lengths) = match_lengths(
            &board,
            &mut result,
            &config,
            &[0.0, 1.6],
            &names,
            &BTreeSet::new(),
        );
        assert_eq!(lengths.len(), 3, "{lengths:?}");
        // /A: from about 30 mm to the middle of its bounds; then /B to
        // within the skew of /A's new length.
        let a = &lengths[0];
        assert_eq!((a.net.as_str(), a.rule.as_str()), ("A", "long"));
        assert!(a.meander_mm > 2.0, "{a:?}");
        assert!((a.length_mm - 33.5).abs() < 0.01, "{a:?}");
        let b = lengths.iter().find(|entry| entry.net == "B").unwrap();
        assert!(b.meander_mm > 10.0, "{b:?}");
        assert!((b.length_mm - (33.5 - 0.25)).abs() < 0.01, "{b:?}");
        assert!(!patterns.is_empty());
        // The meanders keep every clearance.
        assert!(core::verify(&board, &result.routes).is_empty());
        // Written: the meanders' tracks are the routes', their patterns
        // single-net tuning patterns over them; measured from the board,
        // each net is within its bounds.
        let _ = emit_routes(&mut pcb, &board, &result, &names, true).unwrap();
        write_length_patterns(&mut pcb, &board, &patterns, &names).unwrap();
        let text = encode(&pcb);
        assert!(text.contains("(tuning_mode \"single\")"));
        let uuids: BTreeSet<String> = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("segment"))
            .filter_map(|item| {
                item.child("uuid")
                    .and_then(|uuid| uuid.children().get(1))
                    .and_then(Expr::atom)
                    .map(str::to_string)
            })
            .collect();
        for pattern in pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("generated"))
        {
            let members = pattern.child("members").unwrap();
            assert!(members.children().len() > 1);
            for member in members.children().iter().skip(1) {
                assert!(uuids.contains(member.atom().unwrap()));
            }
        }
        assert!(lengths.iter().all(|entry| entry.within), "{lengths:?}");
        // Off, nothing changes.
        let off = KiCadBoardRouterConfig {
            length_matching: Some(false),
            ..config.clone()
        };
        let mut again = core::route(&board, &core_config(&config));
        let before = again.routes.clone();
        let (patterns, lengths) = match_lengths(
            &board,
            &mut again,
            &off,
            &[0.0, 1.6],
            &names,
            &BTreeSet::new(),
        );
        assert!(patterns.is_empty() && lengths.is_empty());
        assert_eq!(again.routes.len(), before.len());
    }
}

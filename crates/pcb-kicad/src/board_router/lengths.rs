// Copyright (C) 2026 Toit contributors.

//! Length and skew rules of single nets and groups of nets (docs/router.md,
//! "Length and skew rules").
//!
//! The project's custom rules' `length` and `skew` constraints reach the
//! router as `KiCadLengthRule`s: per rule the nets it holds for and, per
//! net, the layers (and whether the vias) whose copper KiCad counts for it.
//! After the route (`match_lengths`, when an attempt is written, after the
//! differential pairs), each net's length is measured from its copper
//! (tracks, vias at the stackup's height: `meander::copper_length_on`); a
//! net shorter than a `length` minimum, or further than a `skew` maximum
//! below the longest net of its rule's group, gets a meander on a straight
//! track of its route: bumps of at most `MAX_AMPLITUDE`, their height
//! bisected so that they add what it lacks, the longest tracks first, the
//! first placement whose copper keeps every clearance (`core::verify`
//! against everything routed and every obstacle). The meander's tracks are
//! the route's and are written as a KiCad tuning pattern
//! (`write_length_patterns`). A `length` maximum is not kept by the search:
//! a net longer than it is reported. The verdict's `lengths` lists per rule
//! and net the length from the written board (`measure_lengths`), the
//! target, the bounds and what the meander added.

use super::meander::{
    Point, Tuned, Tuning, bump_adds, bump_height, bumps, copper_length_on, polyline_length,
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
/// A length short of its target by less than this needs no meander.
const ENOUGH: f64 = 0.005;
/// A length this close to a bound is on it.
const ON_BOUND: f64 = 1.0e-6;

/// The settings with the length and skew rules the project has reported
/// as kept (`board-router.json`'s `rules.custom_rules`): a `skew` and a
/// `length` minimum `applied`, a `length` with a maximum `approximated`
/// (the maximum is checked, not kept by the search). Unchanged where the
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
                entry.detail = "length matching (length_matching): a net below the minimum gets a meander after the route; the maximum is checked (the verdict's lengths), not kept by the search".into();
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
fn fixed_copper(board: &core::Board) -> BTreeMap<core::NetId, (Vec<(usize, Point, Point)>, Vec<Point>)> {
    let mut copper: BTreeMap<core::NetId, (Vec<(usize, Point, Point)>, Vec<Point>)> =
        BTreeMap::new();
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

/// After the route, before the attempt is written: the meanders the
/// length and skew rules ask for, added to the routes (their tracks), and
/// per rule and net what came of it (lengths from the board as routed; the
/// written board's in `measure_lengths`). `coupled`: the nets of pairs
/// with a coupled link, which the pairs' own pass matches (a meander on
/// one track of a coupled run would part the pair).
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
    let mut patterns: Vec<LengthPattern> = Vec::new();
    let mut report: Vec<KiCadNetLength> = Vec::new();
    // Length rules first: a skew then compares the lengths they made.
    let mut order: Vec<&KiCadLengthRule> = config.length_rules.iter().collect();
    order.sort_by_key(|rule| rule.constraint != "length");
    for rule in order {
        let counted: Vec<Vec<bool>> = rule
            .nets
            .iter()
            .map(|member| {
                layer_names
                    .iter()
                    .map(|name| member.layers.iter().any(|layer| layer == name))
                    .collect()
            })
            .collect();
        let length_of = |position: usize, routes: &[core::NetRoute]| -> f64 {
            let member = &rule.nets[position];
            let Some(&id) = ids.get(member.net.as_str()) else {
                return 0.0;
            };
            let route = &routes[id as usize];
            let mut tracks: Vec<(usize, Point, Point)> = route
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
            copper_length_on(&tracks, &vias, heights, |layer| {
                counted[position].get(layer).copied().unwrap_or(false)
            })
        };
        // The groups compared: a length rule's nets one by one, a skew
        // rule's all together (or each differential pair's two nets).
        let groups: Vec<Vec<usize>> = if rule.constraint == "length" {
            (0..rule.nets.len()).map(|position| vec![position]).collect()
        } else if rule.within_diff_pairs {
            let mut by_base: BTreeMap<String, Vec<usize>> = BTreeMap::new();
            for (position, member) in rule.nets.iter().enumerate() {
                if let Some((_, base, _)) = crate::project_rules::diff_pair_of(&member.net) {
                    by_base.entry(base).or_default().push(position);
                }
            }
            by_base.into_values().filter(|group| group.len() == 2).collect()
        } else {
            vec![(0..rule.nets.len()).collect()]
        };
        for group in groups {
            let lengths: Vec<f64> = group
                .iter()
                .map(|&position| length_of(position, &result.routes))
                .collect();
            let longest = lengths.iter().copied().fold(0.0, f64::max);
            let (target, min, max) = if rule.constraint == "length" {
                length_target(rule)
            } else {
                let tolerance = rule.max.unwrap_or(0.0);
                (
                    longest - tolerance / 2.0,
                    Some(longest - tolerance),
                    Some(longest),
                )
            };
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
                        &counted[position],
                        [target, min.unwrap_or(0.0), max.unwrap_or(f64::INFINITY)],
                    );
                    entry.meander_mm = added;
                    entry.length_mm = length + added;
                    patterns.extend(made);
                    if need - added > ENOUGH {
                        entry.note = format!(
                            "no straight track with room for all of it ({:.3} of {:.3} mm added)",
                            added, need
                        );
                    }
                }
                if max.is_some_and(|max| length > max + ON_BOUND) && rule.constraint == "length"
                {
                    entry.note =
                        "longer than the maximum: the search does not bound a net's length"
                            .into();
                }
                report.push(entry);
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
    let clearance = board.classes[board.nets[net as usize].class as usize].clearance;
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
    for segment in candidates {
        let remaining = need - added;
        if remaining < ENOUGH {
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
        'placed: for cap in [top, top / 2.0, top / 4.0] {
            let room = ((length - 2.0 * margin) / spacing + 1.0) / 2.0;
            if room < 1.0 {
                break;
            }
            let count = ((remaining / bump_adds(cap, spacing)).ceil().max(1.0) as usize)
                .min(room.floor() as usize);
            let want = remaining.min(count as f64 * bump_adds(cap, spacing));
            let height = bump_height(want, count, spacing, cap);
            let span = (2 * count - 1) as f64 * spacing;
            for side in [1.0, -1.0] {
                let outward = [left[0] * side, left[1] * side];
                for from in [margin, (length - span) / 2.0, length - margin - span] {
                    let points = bumps((a, b), outward, from, count, height, spacing);
                    let (first, last) = (points[0], points[points.len() - 1]);
                    if joined_between(board, &routes[net as usize], net, &segment, first, last) {
                        continue;
                    }
                    let pieces: Vec<(Point, Point)> =
                        points.windows(2).map(|pair| (pair[0], pair[1])).collect();
                    if refused(&near, routes, net, segment.layer, w, &pieces) {
                        continue;
                    }
                    // The track splits around the meander.
                    let route = &mut routes[net as usize];
                    let Some(position) = route.segments.iter().position(|other| {
                        other.layer == segment.layer && other.start == a && other.end == b
                    }) else {
                        continue;
                    };
                    let piece = |start: Point, end: Point| core::Segment {
                        layer: segment.layer,
                        start,
                        end,
                        width: w,
                    };
                    let mut replaced = vec![piece(a, first)];
                    replaced.extend(pieces.iter().map(|&(start, end)| piece(start, end)));
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
                    break 'placed;
                }
            }
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

/// After the route: each rule's nets' lengths from the written board
/// (`lengths`, as `match_lengths` left them with their meanders and notes),
/// the bounds of a skew rule from its group's longest net measured, and
/// whether each lies within them.
pub(crate) fn measure_lengths(
    lengths: &mut [KiCadNetLength],
    config: &KiCadBoardRouterConfig,
    pcb: &Expr,
) {
    if lengths.is_empty() {
        return;
    }
    let by_name = copper_heights(pcb);
    let names: Vec<&String> = by_name.keys().collect();
    let heights: Vec<f64> = by_name.values().copied().collect();
    let mut copper: BTreeMap<String, (Vec<(usize, Point, Point)>, Vec<Point>)> = BTreeMap::new();
    for item in pcb.children() {
        let kind = item.head();
        if !matches!(kind, Some("segment" | "arc" | "via")) {
            continue;
        }
        let Some(net) = node_net(item) else { continue };
        let net = crate::kicad_nets::normalize_net(&unescape(net)).to_string();
        if kind == Some("via") {
            if let Ok(at) = form_xy(item, "at") {
                copper.entry(net).or_default().1.push(at);
            }
            continue;
        }
        let layer = form_atom(item, "layer", 1)
            .and_then(|layer| names.iter().position(|name| name.as_str() == layer))
            .unwrap_or(usize::MAX);
        let (Ok(start), Ok(end)) = (form_xy(item, "start"), form_xy(item, "end")) else {
            continue;
        };
        let entry = copper.entry(net).or_default();
        match form_xy(item, "mid") {
            Ok(mid) if kind == Some("arc") => {
                let points = crate::outline::arc_points(start, mid, end);
                for pair in points.windows(2) {
                    entry.0.push((layer, pair[0], pair[1]));
                }
            }
            _ => entry.0.push((layer, start, end)),
        }
    }
    for rule in &config.length_rules {
        let measured: Vec<(usize, f64)> = lengths
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.rule == rule.rule && entry.constraint == rule.constraint)
            .filter_map(|(index, entry)| {
                let member = rule.nets.iter().find(|member| member.net == entry.net)?;
                let length = copper.get(&entry.net).map_or(0.0, |(tracks, vias)| {
                    let vias: &[Point] = if member.vias { vias } else { &[] };
                    copper_length_on(tracks, vias, &heights, |layer| {
                        names
                            .get(layer)
                            .is_some_and(|name| member.layers.iter().any(|ruled| ruled == *name))
                    })
                });
                Some((index, length))
            })
            .collect();
        for &(index, length) in &measured {
            lengths[index].length_mm = length;
        }
        if rule.constraint == "skew" {
            // The group's longest, as measured (each pair's with
            // `within_diff_pairs`).
            let group_of = |net: &str| {
                if rule.within_diff_pairs {
                    crate::project_rules::diff_pair_of(net).map(|(_, base, _)| base)
                } else {
                    Some(String::new())
                }
            };
            let mut longest: BTreeMap<String, f64> = BTreeMap::new();
            for &(index, length) in &measured {
                if let Some(group) = group_of(&lengths[index].net) {
                    let value = longest.entry(group).or_insert(0.0);
                    *value = value.max(length);
                }
            }
            let tolerance = rule.max.unwrap_or(0.0);
            for &(index, _) in &measured {
                if let Some(top) = group_of(&lengths[index].net).and_then(|g| longest.get(&g)) {
                    lengths[index].min_mm = Some(top - tolerance);
                    lengths[index].max_mm = Some(*top);
                    lengths[index].target_mm = top - tolerance / 2.0;
                }
            }
        }
    }
    for entry in lengths.iter_mut() {
        entry.within = entry.length_mm > 0.0
            && entry
                .min_mm
                .is_none_or(|min| entry.length_mm >= min - ON_BOUND)
            && entry
                .max_mm
                .is_none_or(|max| entry.length_mm <= max + ON_BOUND);
    }
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
        let (patterns, mut lengths) =
            match_lengths(&board, &mut result, &config, &[0.0, 1.6], &names, &BTreeSet::new());
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
        measure_lengths(&mut lengths, &config, &pcb);
        assert!(lengths.iter().all(|entry| entry.within), "{lengths:?}");
        // Off, nothing changes.
        let off = KiCadBoardRouterConfig {
            length_matching: Some(false),
            ..config.clone()
        };
        let mut again = core::route(&board, &core_config(&config));
        let before = again.routes.clone();
        let (patterns, lengths) =
            match_lengths(&board, &mut again, &off, &[0.0, 1.6], &names, &BTreeSet::new());
        assert!(patterns.is_empty() && lengths.is_empty());
        assert_eq!(again.routes.len(), before.len());
    }
}

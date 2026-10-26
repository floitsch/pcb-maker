// Copyright (C) 2026 Toit contributors.

//! Meanders (docs/router.md, "Skew and length" and "Length and skew
//! rules"): the bumps that lengthen a straight track, a net's length from
//! its copper, and the KiCad tuning pattern a meander is written as.
//! Shared by the differential pairs' skew (`diff_pairs::match_skew`) and
//! the length and skew rules of single nets (`lengths`).

use super::*;
use pcb_router as core;

pub(crate) type Point = [f64; 2];

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}

fn scale(a: Point, factor: f64) -> Point {
    [a[0] * factor, a[1] * factor]
}

fn unit(a: Point) -> Point {
    let length = (a[0] * a[0] + a[1] * a[1]).sqrt();
    if length < 1.0e-9 {
        [0.0, 0.0]
    } else {
        scale(a, 1.0 / length)
    }
}

/// A meander's polyline on the track from `a` to `b`: `count` bumps of
/// `height` toward `outward`, each `spacing` wide and `spacing` apart,
/// starting `from` along the track, their corners cut at 45 degrees.
pub(crate) fn bumps(
    (a, b): (Point, Point),
    outward: Point,
    from: f64,
    count: usize,
    height: f64,
    spacing: f64,
) -> Vec<Point> {
    let u = unit(sub(b, a));
    let at = |x: f64, y: f64| add(a, add(scale(u, x), scale(outward, y)));
    let cut = (height / 2.0).min(spacing / 4.0);
    let mut points = Vec::new();
    let mut x = from;
    for _ in 0..count {
        points.extend([
            at(x, 0.0),
            at(x, height - cut),
            at(x + cut, height),
            at(x + spacing - cut, height),
            at(x + spacing, height - cut),
            at(x + spacing, 0.0),
        ]);
        x += 2.0 * spacing;
    }
    points
}

pub(crate) fn polyline_length(points: &[Point]) -> f64 {
    points
        .windows(2)
        .map(|pair| core::geometry::distance(pair[0], pair[1]))
        .sum()
}

/// What one bump of `height`, `spacing` wide, adds to a track's length.
pub(crate) fn bump_adds(height: f64, spacing: f64) -> f64 {
    polyline_length(&bumps(
        ([0.0, 0.0], [1.0, 0.0]),
        [0.0, 1.0],
        0.0,
        1,
        height,
        spacing,
    )) - spacing
}

/// The height at which `count` bumps `spacing` wide add `need` (bisected;
/// at most `cap`).
pub(crate) fn bump_height(need: f64, count: usize, spacing: f64, cap: f64) -> f64 {
    let (mut low, mut high) = (0.0, cap);
    for _ in 0..40 {
        let middle = (low + high) / 2.0;
        if count as f64 * bump_adds(middle, spacing) < need {
            low = middle;
        } else {
            high = middle;
        }
    }
    high
}

/// A net's length from its copper: its tracks (layer, start, end) and its
/// vias, each via the height between the outermost layers of the tracks
/// that end on it (`heights` per layer; a via joining no two layers counts
/// nothing).
pub(crate) fn copper_length(
    tracks: &[(usize, Point, Point)],
    vias: &[Point],
    heights: &[f64],
) -> f64 {
    let along: f64 = tracks
        .iter()
        .map(|&(_, start, end)| core::geometry::distance(start, end))
        .sum();
    let through: f64 = vias
        .iter()
        .map(|&via| {
            let zs: Vec<f64> = tracks
                .iter()
                .filter(|&&(_, start, end)| {
                    core::geometry::distance(start, via) < 1.0e-4
                        || core::geometry::distance(end, via) < 1.0e-4
                })
                .filter_map(|&(layer, ..)| heights.get(layer).copied())
                .collect();
            let (low, high) = zs
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), &z| {
                    (low.min(z), high.max(z))
                });
            if high > low { high - low } else { 0.0 }
        })
        .sum();
    along + through
}

/// What a tuning pattern tunes (KiCad's `tuning_mode`).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Tuning {
    /// A single net's length (`single`): its target and bounds.
    Single { target: f64, min: f64, max: f64 },
    /// One net of a differential pair against its partner
    /// (`diff_pair_skew`), within the pair's gap.
    PairSkew { gap: f64 },
}

/// A meander as written (`tuning_pattern_expr`).
pub(crate) struct Tuned<'a> {
    pub net: &'a str,
    pub width: f64,
    /// The meander's polyline, from where it leaves its track to where it
    /// rejoins it.
    pub points: &'a [Point],
    pub amplitude: f64,
    pub spacing: f64,
    /// The length it adds.
    pub added: f64,
    pub tuning: Tuning,
}

/// A KiCad tuning pattern (`generated`) over a meander's tracks
/// (`members`: their uuids). KiCad's gap check skips a pattern's tracks.
pub(crate) fn tuning_pattern_expr(meander: &Tuned, layer: &str, members: Vec<Expr>) -> Expr {
    let atom = |text: String| Expr::Atom(text);
    let form = |head: &str, values: Vec<Expr>| {
        let mut items = vec![atom(head.into())];
        items.extend(values);
        Expr::List(items)
    };
    let number = |value: f64| atom(format!("{value:.6}"));
    let xy = |point: Point| form("xy", vec![number(point[0]), number(point[1])]);
    let (first, last) = (meander.points[0], meander.points[meander.points.len() - 1]);
    let identity = format!(
        "pcb-maker:tuning-pattern:{}:{:.9}:{:.9}:{:.9}:{:.9}:{layer}",
        meander.net, first[0], first[1], last[0], last[1]
    );
    let text = |value: &str| atom(quote(value));
    let gap = match meander.tuning {
        Tuning::PairSkew { gap } => gap,
        Tuning::Single { .. } => 0.0,
    };
    let mut items = vec![
        form("uuid", vec![text(&deterministic_uuid_for(&identity))]),
        form("type", vec![atom("tuning_pattern".into())]),
        form("name", vec![text("Tuning Pattern")]),
        form("layer", vec![text(layer)]),
        form("base_line", vec![form("pts", vec![xy(first), xy(last)])]),
        form("corner_radius_percent", vec![atom("0".into())]),
        form("end", vec![xy(last)]),
        form("initial_side", vec![text("default")]),
        form("is_time_domain", vec![atom("no".into())]),
        form("last_diff_pair_gap", vec![number(gap)]),
        form("last_netname", vec![text(&written_net_name(meander.net))]),
        form("last_status", vec![text("tuned")]),
        form("last_track_width", vec![number(meander.width)]),
        form("last_tuning_length", vec![number(meander.added)]),
        form("max_amplitude", vec![number(meander.amplitude)]),
        form("min_amplitude", vec![number(meander.amplitude)]),
        form("min_spacing", vec![number(meander.spacing)]),
        form("origin", vec![xy(first)]),
        form("override_custom_rules", vec![atom("no".into())]),
        form("rounded", vec![atom("no".into())]),
        form("single_sided", vec![atom("yes".into())]),
    ];
    match meander.tuning {
        Tuning::PairSkew { gap } => items.extend([
            form("target_skew", vec![number(0.0)]),
            form("target_skew_max", vec![number(gap)]),
            form("target_skew_min", vec![number(-gap)]),
            form("tuning_mode", vec![text("diff_pair_skew")]),
        ]),
        Tuning::Single { target, min, max } => items.extend([
            form("target_length", vec![number(target)]),
            form("target_length_max", vec![number(max)]),
            form("target_length_min", vec![number(min)]),
            form("tuning_mode", vec![text("single")]),
        ]),
    }
    items.push(form("members", members));
    form("generated", items)
}

/// The copper layers' heights by name (millimetres from the board's top),
/// as KiCad's length with the stackup's height measures a via between two
/// of them (kicad-cli 10.0.7 on KiCad's QA board length_calculations: a
/// through via from F.Cu to B.Cu counts the whole stack, copper included;
/// to an inner layer, to its middle): the outer copper layers at the
/// board's two faces, each inner one at its middle, from the board's
/// stackup (`setup`'s `stackup`, a dielectric's sublayers summed), else
/// its thickness (`general`, 1.6 mm if none) spread evenly over the
/// copper layers it declares. A via's length is the height between the
/// layers it joins.
pub(crate) fn copper_heights(pcb: &Expr) -> BTreeMap<String, f64> {
    let mut heights = BTreeMap::new();
    if let Some(stackup) = pcb.child("setup").and_then(|setup| setup.child("stackup")) {
        let mut z = 0.0;
        // Per copper layer: its name, top and thickness.
        let mut copper: Vec<(String, f64, f64)> = Vec::new();
        for layer in stackup
            .children()
            .iter()
            .filter(|item| item.head() == Some("layer"))
        {
            let name = layer.children().get(1).and_then(Expr::atom).unwrap_or("");
            // A dielectric's sublayers (`addsublayer`) each have their own.
            let thickness: f64 = layer
                .children()
                .iter()
                .filter(|item| item.head() == Some("thickness"))
                .filter_map(|item| item.children().get(1).and_then(Expr::atom))
                .filter_map(|value| value.parse::<f64>().ok())
                .sum();
            let is_copper = layer
                .child("type")
                .and_then(|kind| kind.children().get(1))
                .and_then(Expr::atom)
                == Some("copper");
            if is_copper {
                copper.push((name.to_string(), z, thickness));
            }
            z += thickness;
        }
        let last = copper.len().saturating_sub(1);
        for (index, (name, top, thickness)) in copper.into_iter().enumerate() {
            let at = if index == 0 {
                top
            } else if index == last {
                top + thickness
            } else {
                top + thickness / 2.0
            };
            heights.insert(name, at);
        }
    }
    if heights.len() >= 2 {
        return heights;
    }
    let thickness = pcb
        .child("general")
        .and_then(|general| form_f64(general, "thickness", 1).ok())
        .unwrap_or(1.6);
    let names: Vec<String> = pcb
        .child("layers")
        .map(|layers| {
            layers
                .children()
                .iter()
                .filter_map(|layer| {
                    let name = layer.children().get(1).and_then(Expr::atom)?;
                    name.ends_with(".Cu").then(|| name.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    let last = names.len().saturating_sub(1).max(1) as f64;
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name, thickness * index as f64 / last))
        .collect()
}

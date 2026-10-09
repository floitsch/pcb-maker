// Copyright (C) 2026 Toit contributors.

//! Reference labels (silkscreen designators) moved to where they can be
//! read: off pads, off other silkscreen, off other labels, inside the
//! board. After placement the labels sit at their footprints' default
//! offsets, which on a tight board lands them on a neighbour's pads.

use super::*;

/// An axis-aligned box: [min x, min y, max x, max y].
type Box2 = [f64; 4];

fn overlaps(a: Box2, b: Box2, gap: f64) -> bool {
    a[0] < b[2] + gap && b[0] < a[2] + gap && a[1] < b[3] + gap && b[1] < a[3] + gap
}

/// Half extents of a stroke-font text (see `copper_graphic_shapes` for the
/// measurements) at angle 0: [along the text, across it].
fn text_half(text: &str, size: [f64; 2], thickness: f64) -> [f64; 2] {
    let advance = |character: char| match character {
        'i' | 'j' | 'I' | '!' | '.' | ',' | ':' | ';' | '\'' | '`' => 0.48,
        'l' => 0.52,
        't' | 'f' => 0.57,
        'r' => 0.62,
        '(' | ')' | '[' | ']' | '{' | '}' => 0.67,
        'J' | '_' | ' ' | '"' => 0.76,
        'A' => 0.86,
        'N' | 'w' => 1.05,
        'M' | 'W' | '%' => 1.14,
        '-' => 1.24,
        '@' => 1.29,
        'm' => 1.33,
        _ => 1.0,
    };
    let width = 1.08 * text.chars().map(advance).sum::<f64>() * size[1];
    // Centred on the anchor, capitals and digits are one font height tall
    // (KiCad's DRC keeps two lines of "HH" apart below 1.15 mm at size 1
    // and thickness 0.15, and scales exactly); descenders reach 0.3 below.
    let descends = text.chars().any(|c| matches!(c, 'g' | 'j' | 'p' | 'q' | 'y' | ',' | ';'));
    [(width + thickness) / 2.0, (if descends { 1.6 } else { 1.0 } * size[0] + thickness) / 2.0]
}

/// Silkscreen layer of a label and the copper layer its pads are on.
fn silk_side(layer: &str) -> Option<bool> {
    match layer {
        "F.SilkS" => Some(false),
        "B.SilkS" => Some(true),
        _ => None,
    }
}

struct Label {
    footprint: usize,
    /// Index of the `property` child inside the footprint.
    child: usize,
    back: bool,
    text: String,
    size: [f64; 2],
    thickness: f64,
    /// Board position and absolute angle now.
    at: [f64; 2],
    angle: f64,
    body: Box2,
}

/// Moves every coordinate form of a board graphic by `shift`.
fn translate(item: &mut Expr, shift: [f64; 2]) {
    let Expr::List(children) = item else {
        return;
    };
    for child in children.iter_mut() {
        match child.head() {
            // A text's `at` keeps its angle.
            Some("start" | "mid" | "end" | "center" | "xy" | "at") => {
                if let Expr::List(values) = child {
                    for (index, delta) in [(1, shift[0]), (2, shift[1])] {
                        if let Some(number) = values.get(index).and_then(Expr::atom).and_then(|atom| atom.parse::<f64>().ok()) {
                            values[index] = Expr::Atom((((number + delta) * 1e6).round() / 1e6).to_string());
                        }
                    }
                }
            }
            Some("pts") => translate(child, shift),
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadLabelReport {
    pub labels: usize,
    pub moved: usize,
    /// Labels no free spot was found for (left where they were).
    pub stuck: Vec<String>,
}

fn visible(property: &Expr) -> bool {
    let hidden = property.child("hide").is_some_and(|hide| hide.children().get(1).and_then(Expr::atom) != Some("no"))
        || property.children().iter().any(|child| child.atom() == Some("hide"))
        || property
            .child("effects")
            .is_some_and(|effects| effects.children().iter().any(|child| child.atom() == Some("hide")));
    !hidden
}

/// Moves every visible reference label to the nearest free spot around
/// its footprint.
pub(crate) fn place_labels(pcb: &mut Expr) -> Result<KiCadLabelReport, String> {
    let outline = outline::board_loops(pcb).ok().map(|loops| {
        loops.outline.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
            [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
        })
    });
    // What labels must stay off, per side: exposed copper (pads) and
    // silkscreen that is not a label.
    let mut blocked: [Vec<Box2>; 2] = [Vec::new(), Vec::new()];
    let mut labels = Vec::new();
    let footprints: Vec<&Expr> = pcb.children().iter().filter(|item| item.head() == Some("footprint")).collect();
    for (index, footprint) in footprints.iter().enumerate() {
        let at = form_at(footprint)?;
        let place = |local: [f64; 2]| {
            let offset = rotate_vector(local, -at[2]);
            [at[0] + offset[0], at[1] + offset[1]]
        };
        for pad in footprint.children().iter().filter(|child| child.head() == Some("pad")) {
            let pad_at = form_at(pad)?;
            let center = place([pad_at[0], pad_at[1]]);
            let size = form_xy(pad, "size").unwrap_or([0.0, 0.0]);
            let (sin, cos) = (-pad_at[2]).to_radians().sin_cos();
            let half = [
                (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0 + 0.05,
                (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0 + 0.05,
            ];
            let layers: Vec<&str> = pad
                .child("layers")
                .map(|layers| layers.children().iter().skip(1).filter_map(Expr::atom).collect())
                .unwrap_or_default();
            // A drill offset moves the copper away from the hole: the box
            // covers both.
            let offset = pad
                .child("drill")
                .and_then(|drill| drill.child("offset"))
                .and_then(|offset| Some([expression_coordinate(offset, 1, "pad offset x").ok()?, expression_coordinate(offset, 2, "pad offset y").ok()?]))
                .map(|offset| rotate_vector(offset, -pad_at[2]))
                .unwrap_or([0.0, 0.0]);
            let copper = [center[0] + offset[0], center[1] + offset[1]];
            let bounds = [
                center[0].min(copper[0]) - half[0],
                center[1].min(copper[1]) - half[1],
                center[0].max(copper[0]) + half[0],
                center[1].max(copper[1]) + half[1],
            ];
            if layers.iter().any(|layer| matches!(*layer, "F.Cu" | "*.Cu" | "F&B.Cu")) {
                blocked[0].push(bounds);
            }
            if layers.iter().any(|layer| matches!(*layer, "B.Cu" | "*.Cu" | "F&B.Cu")) {
                blocked[1].push(bounds);
            }
        }
        for child in footprint.children() {
            let Some(back) = form_atom(child, "layer", 1).and_then(silk_side) else {
                continue;
            };
            // The footprint's own texts (a diode's "K"): everything but the
            // labels (reference and value), which move.
            let own_text = match child.head() {
                Some("fp_text") => !matches!(child.children().get(1).and_then(Expr::atom), Some("reference" | "value")),
                Some("property") => {
                    !matches!(child.children().get(1).and_then(Expr::atom), Some("Reference" | "Value")) && visible(child)
                }
                _ => false,
            };
            if own_text {
                let text = child.children().get(2).and_then(Expr::atom).unwrap_or("");
                if !text.is_empty()
                    && let Ok(text_at) = form_at(child)
                {
                    let font = child.child("effects").and_then(|effects| effects.child("font"));
                    let size = font.and_then(|font| form_xy(font, "size").ok()).unwrap_or([1.0, 1.0]);
                    let thickness = font.and_then(|font| form_f64(font, "thickness", 1).ok()).unwrap_or(0.15);
                    let [along, across] = text_half(text, size, thickness);
                    let (sin, cos) = text_at[2].to_radians().sin_cos();
                    let half = [cos.abs() * along + sin.abs() * across, sin.abs() * along + cos.abs() * across];
                    let center = place([text_at[0], text_at[1]]);
                    blocked[back as usize].push([center[0] - half[0], center[1] - half[1], center[0] + half[0], center[1] + half[1]]);
                }
                continue;
            }
            let mut points: Vec<[f64; 2]> = outline::outline_points(child)?.into_iter().map(|point| place(point)).collect();
            if child.child("center").is_some() {
                points.push(place(form_xy(child, "center")?));
            }
            if child.head() == Some("fp_circle") {
                let (center, end) = (form_xy(child, "center")?, form_xy(child, "end")?);
                let radius = distance_squared(center, end).sqrt();
                let middle = place(center);
                points.extend([[middle[0] - radius, middle[1] - radius], [middle[0] + radius, middle[1] + radius]]);
            }
            for point in outline::pts_points(child)? {
                points.push(place(point));
            }
            if matches!(child.head(), Some("fp_line")) && points.len() == 2 {
                // A line is thin: its own box, not a box around it at an angle.
                let steps = (distance_squared(points[0], points[1]).sqrt() / 0.5).ceil().max(1.0) as usize;
                for step in 0..=steps {
                    let t = step as f64 / steps as f64;
                    let p = [points[0][0] + t * (points[1][0] - points[0][0]), points[0][1] + t * (points[1][1] - points[0][1])];
                    blocked[back as usize].push([p[0] - 0.1, p[1] - 0.1, p[0] + 0.1, p[1] + 0.1]);
                }
            } else if matches!(child.head(), Some("fp_rect" | "fp_circle" | "fp_poly" | "fp_arc")) && !points.is_empty() {
                let bounds = points.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
                    [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
                });
                if child.head() == Some("fp_rect") && child.child("fill").is_none_or(|fill| fill.children().get(1).and_then(Expr::atom) != Some("yes")) {
                    // An outline: its four sides.
                    for side in [
                        [bounds[0], bounds[1], bounds[2], bounds[1]],
                        [bounds[0], bounds[3], bounds[2], bounds[3]],
                        [bounds[0], bounds[1], bounds[0], bounds[3]],
                        [bounds[2], bounds[1], bounds[2], bounds[3]],
                    ] {
                        blocked[back as usize].push([side[0] - 0.1, side[1] - 0.1, side[2] + 0.1, side[3] + 0.1]);
                    }
                } else {
                    blocked[back as usize].push(bounds);
                }
            }
        }
        let (center, size, _) = crate::board_placer::local_body(footprint)?;
        let middle = place(center);
        let (sin, cos) = (-at[2]).to_radians().sin_cos();
        let half = [(cos.abs() * size[0] + sin.abs() * size[1]) / 2.0, (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0];
        let body = [middle[0] - half[0], middle[1] - half[1], middle[0] + half[0], middle[1] + half[1]];
        for (child_index, child) in footprint.children().iter().enumerate() {
            if child.head() != Some("property")
                || !matches!(child.children().get(1).and_then(Expr::atom), Some("Reference" | "Value"))
                || !visible(child)
            {
                continue;
            }
            let Some(back) = form_atom(child, "layer", 1).and_then(silk_side) else {
                continue;
            };
            let text = child.children().get(2).and_then(Expr::atom).unwrap_or("").to_string();
            if text.is_empty() {
                continue;
            }
            let label_at = form_at(child)?;
            let font = child.child("effects").and_then(|effects| effects.child("font"));
            labels.push(Label {
                footprint: index,
                child: child_index,
                back,
                text,
                size: font.and_then(|font| form_xy(font, "size").ok()).unwrap_or([1.0, 1.0]),
                thickness: font.and_then(|font| form_f64(font, "thickness", 1).ok()).unwrap_or(0.15),
                at: place([label_at[0], label_at[1]]),
                angle: label_at[2],
                body,
            });
        }
    }
    let mut report = KiCadLabelReport { labels: labels.len(), ..Default::default() };
    let mut taken: [Vec<Box2>; 2] = [Vec::new(), Vec::new()];
    let inside = |bounds: Box2| {
        outline.is_none_or(|board| {
            bounds[0] >= board[0] + 0.3 && bounds[1] >= board[1] + 0.3 && bounds[2] <= board[2] - 0.3 && bounds[3] <= board[3] - 0.3
        })
    };

    // Board silkscreen artwork (a logo drawn as polygons and lines) moves as
    // one piece: shapes within 0.5 mm of each other form a cluster.
    let mut graphics: Vec<(usize, bool, Box2)> = Vec::new();
    for (item_index, item) in pcb.children().iter().enumerate() {
        let Some(back) = form_atom(item, "layer", 1).and_then(silk_side) else {
            continue;
        };
        if !matches!(item.head(), Some("gr_poly" | "gr_line" | "gr_rect" | "gr_circle" | "gr_arc" | "gr_curve")) {
            continue;
        }
        let mut points = outline::outline_points(item)?;
        if item.child("center").is_some() {
            points.push(form_xy(item, "center")?);
        }
        points.extend(outline::pts_points(item)?);
        if item.head() == Some("gr_circle") && points.len() == 2 {
            let radius = distance_squared(points[0], points[1]).sqrt();
            points.push([points[0][0] - radius, points[0][1] - radius]);
            points.push([points[0][0] + radius, points[0][1] + radius]);
        }
        if points.is_empty() {
            continue;
        }
        let bounds = points.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
            [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
        });
        graphics.push((item_index, back, bounds));
    }
    let mut clusters: Vec<(Vec<usize>, bool, Box2)> = Vec::new();
    for (item_index, back, bounds) in graphics {
        let mut members = vec![item_index];
        let mut total = bounds;
        // Absorb every cluster this shape touches.
        let mut index = 0;
        while index < clusters.len() {
            if clusters[index].1 == back && overlaps(clusters[index].2, total, 0.5) {
                let (other, _, other_bounds) = clusters.remove(index);
                members.extend(other);
                total = [
                    total[0].min(other_bounds[0]),
                    total[1].min(other_bounds[1]),
                    total[2].max(other_bounds[2]),
                    total[3].max(other_bounds[3]),
                ];
                index = 0;
            } else {
                index += 1;
            }
        }
        clusters.push((members, back, total));
    }
    let mut artwork_moves: Vec<(Vec<usize>, [f64; 2])> = Vec::new();
    for (members, back, bounds) in clusters {
        let side = back as usize;
        // A frame around the whole board is decoration of the board, not a
        // piece to move.
        if outline.is_some_and(|board| (bounds[2] - bounds[0]) > 0.8 * (board[2] - board[0])) {
            continue;
        }
        let free = |bounds: Box2, taken: &[Box2]| {
            !blocked[side].iter().any(|b| overlaps(bounds, *b, 0.1)) && !taken.iter().any(|b| overlaps(bounds, *b, 0.2))
        };
        if free(bounds, &taken[side]) {
            taken[side].push(bounds);
            continue;
        }
        let mut found = None;
        'rings: for ring in 1..=60 {
            let radius = ring as f64 * 0.5;
            let directions = 8 * ring.min(4);
            for step in 0..directions {
                let angle = step as f64 / directions as f64 * std::f64::consts::TAU;
                let shift = [radius * angle.cos(), radius * angle.sin()];
                let moved = [bounds[0] + shift[0], bounds[1] + shift[1], bounds[2] + shift[0], bounds[3] + shift[1]];
                if inside(moved) && free(moved, &taken[side]) {
                    found = Some((shift, moved));
                    break 'rings;
                }
            }
        }
        match found {
            Some((shift, moved)) => {
                taken[side].push(moved);
                artwork_moves.push((members, shift));
            }
            None => {
                taken[side].push(bounds);
                report.stuck.push("silkscreen artwork".into());
            }
        }
    }

    // Board silkscreen texts (a board's name, a note) first: where they are
    // if that is free, else the nearest free spot around it.
    let mut text_moves: Vec<(usize, [f64; 2])> = Vec::new();
    for (item_index, item) in pcb.children().iter().enumerate() {
        let Some(back) = form_atom(item, "layer", 1).and_then(silk_side) else {
            continue;
        };
        if item.head() != Some("gr_text") {
            continue;
        }
        let Some(bounds) = board_router::copper_graphic_shapes(item)?
            .iter()
            .map(pcb_router::Shape::aabb)
            .reduce(pcb_router::geometry::Aabb::union)
        else {
            continue;
        };
        let bounds = [bounds.minimum[0], bounds.minimum[1], bounds.maximum[0], bounds.maximum[1]];
        let side = back as usize;
        let free = |bounds: Box2, taken: &[Box2]| {
            !blocked[side].iter().any(|b| overlaps(bounds, *b, 0.1)) && !taken.iter().any(|b| overlaps(bounds, *b, 0.2))
        };
        if free(bounds, &taken[side]) {
            taken[side].push(bounds);
            continue;
        }
        let mut found = None;
        'rings: for ring in 1..=50 {
            let radius = ring as f64 * 0.5;
            let directions = 8 * ring.min(4);
            for step in 0..directions {
                let angle = step as f64 / directions as f64 * std::f64::consts::TAU;
                let shift = [radius * angle.cos(), radius * angle.sin()];
                let moved = [bounds[0] + shift[0], bounds[1] + shift[1], bounds[2] + shift[0], bounds[3] + shift[1]];
                if inside(moved) && free(moved, &taken[side]) {
                    found = Some((shift, moved));
                    break 'rings;
                }
            }
        }
        match found {
            Some((shift, moved)) => {
                taken[side].push(moved);
                text_moves.push((item_index, shift));
            }
            None => {
                taken[side].push(bounds);
                report.stuck.push(item.children().get(1).and_then(Expr::atom).unwrap_or("").to_string());
            }
        }
    }
    let mut moves = Vec::new();
    // Big parts first: small ones have more room around them.
    let mut order: Vec<usize> = (0..labels.len()).collect();
    order.sort_by(|a, b| {
        let area = |label: &Label| (label.body[2] - label.body[0]) * (label.body[3] - label.body[1]);
        area(&labels[*b]).total_cmp(&area(&labels[*a]))
    });
    for index in order {
        let label = &labels[index];
        let side = label.back as usize;
        let free = |bounds: Box2| {
            let inside = outline.is_none_or(|board| {
                bounds[0] >= board[0] + 0.3 && bounds[1] >= board[1] + 0.3 && bounds[2] <= board[2] - 0.3 && bounds[3] <= board[3] - 0.3
            });
            inside
                && !blocked[side].iter().any(|b| overlaps(bounds, *b, 0.1))
                && !taken[side].iter().any(|b| overlaps(bounds, *b, 0.2))
        };
        let boxed = |center: [f64; 2], angle: f64, scale: f64| {
            let half = text_half(&label.text, [label.size[0] * scale, label.size[1] * scale], label.thickness * scale);
            let half = if (angle.rem_euclid(180.0) - 90.0).abs() < 1.0 { [half[1], half[0]] } else { half };
            [center[0] - half[0], center[1] - half[1], center[0] + half[0], center[1] + half[1]]
        };
        // Where it is, if that is fine.
        let current = boxed(label.at, label.angle, 1.0);
        if free(current) {
            taken[side].push(current);
            continue;
        }
        // Around the body, nearest to where the label was, horizontal first;
        // then smaller text (not below 0.8 mm, a common fab minimum).
        let b = label.body;
        let mut best: Option<(f64, [f64; 2], f64, f64, Box2)> = None;
        for scale in [1.0, 0.8] {
            if label.size[0] * scale < 0.8 - 1e-9 && scale < 1.0 {
                continue;
            }
            for angle in [0.0, 90.0] {
                let half = {
                    let h = text_half(&label.text, [label.size[0] * scale, label.size[1] * scale], label.thickness * scale);
                    if angle == 90.0 { [h[1], h[0]] } else { h }
                };
                let middle = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
                for ring in 0..4 {
                    let gap = 0.25 + ring as f64 * 0.6;
                    let candidates = [
                        [middle[0], b[1] - gap - half[1]],
                        [middle[0], b[3] + gap + half[1]],
                        [b[0] - gap - half[0], middle[1]],
                        [b[2] + gap + half[0], middle[1]],
                        [b[0] + half[0], b[1] - gap - half[1]],
                        [b[2] - half[0], b[1] - gap - half[1]],
                        [b[0] + half[0], b[3] + gap + half[1]],
                        [b[2] - half[0], b[3] + gap + half[1]],
                        middle,
                    ];
                    for center in candidates {
                        let bounds = boxed(center, angle, scale);
                        if !free(bounds) {
                            continue;
                        }
                        let cost = distance_squared(center, label.at).sqrt() + if angle == 0.0 { 0.0 } else { 1.0 } + (1.0 - scale) * 10.0;
                        if best.as_ref().is_none_or(|(best, ..)| cost < *best) {
                            best = Some((cost, center, angle, scale, bounds));
                        }
                    }
                }
            }
            if best.is_some() {
                break;
            }
        }
        match best {
            Some((_, center, angle, scale, bounds)) => {
                taken[side].push(bounds);
                moves.push((index, center, angle, scale));
            }
            None => {
                taken[side].push(current);
                report.stuck.push(label.text.clone());
            }
        }
    }

    // Write the moves: label positions are in the footprint's frame, their
    // angles absolute.
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    for (members, shift) in artwork_moves {
        for item_index in members {
            translate(&mut items[item_index], shift);
        }
        report.moved += 1;
    }
    for (item_index, shift) in text_moves {
        let at = form_at(&items[item_index])?;
        set_form_at(&mut items[item_index], [at[0] + shift[0], at[1] + shift[1], at[2]])?;
        report.moved += 1;
    }
    let mut footprint_items: Vec<&mut Expr> = items.iter_mut().filter(|item| item.head() == Some("footprint")).collect();
    for (index, center, angle, scale) in moves {
        let label = &labels[index];
        let footprint = &mut footprint_items[label.footprint];
        let at = form_at(footprint)?;
        let local = rotate_vector([center[0] - at[0], center[1] - at[1]], at[2]);
        let Expr::List(children) = footprint else {
            continue;
        };
        let property = &mut children[label.child];
        set_form_at(property, [(local[0] * 1e4).round() / 1e4, (local[1] * 1e4).round() / 1e4, angle])?;
        if scale < 1.0
            && let Expr::List(parts) = property
            && let Some(Expr::List(effects)) = parts.iter_mut().find(|part| part.head() == Some("effects"))
            && let Some(Expr::List(font)) = effects.iter_mut().find(|part| part.head() == Some("font"))
        {
            for part in font.iter_mut() {
                if let Expr::List(values) = part {
                    match values.first().and_then(Expr::atom) {
                        Some("size") if values.len() >= 3 => {
                            for value in values.iter_mut().skip(1).take(2) {
                                if let Some(number) = value.atom().and_then(|atom| atom.parse::<f64>().ok()) {
                                    *value = Expr::Atom(((number * scale * 1e3).round() / 1e3).to_string());
                                }
                            }
                        }
                        Some("thickness") if values.len() >= 2 => {
                            if let Some(number) = values[1].atom().and_then(|atom| atom.parse::<f64>().ok()) {
                                values[1] = Expr::Atom(((number * scale * 1e3).round() / 1e3).to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        report.moved += 1;
    }
    Ok(report)
}

/// Moves the board's copper texts (a title, a "+" next to a terminal) to
/// where they are clear of the copper on their layer: pads, tracks, vias
/// and the parts' courtyards. They stay where they are if that is free.
/// `routes` are the routed tracks (not yet in `pcb`), `layer_names` the
/// copper layers in the router's order.
/// The board's texts on the mask layers: (layer, words, position), the
/// twins exposed-copper artwork comes with.
pub(crate) fn mask_twins(pcb: &Expr) -> Vec<(String, String, [f64; 2])> {
    pcb.children()
        .iter()
        .filter(|item| item.head() == Some("gr_text"))
        .filter_map(|item| {
            let layer = form_atom(item, "layer", 1)?;
            let text = item.children().get(1).and_then(Expr::atom)?;
            let at = form_at(item).ok()?;
            matches!(layer, "F.Mask" | "B.Mask").then(|| (layer.to_string(), text.to_string(), [at[0], at[1]]))
        })
        .collect()
}

/// Whether a copper text on `layer_name` has a twin on its side's mask
/// layer (the same words within 0.1 mm): exposed-copper artwork, which
/// stays where its designer put it.
pub(crate) fn has_mask_twin(twins: &[(String, String, [f64; 2])], item: &Expr, layer_name: &str) -> bool {
    let mask = match layer_name {
        "F.Cu" => "F.Mask",
        "B.Cu" => "B.Mask",
        _ => return false,
    };
    let (Some(text), Ok(at)) = (item.children().get(1).and_then(Expr::atom), form_at(item)) else {
        return false;
    };
    twins.iter().any(|(twin_layer, twin_text, twin_at)| {
        twin_layer == mask && twin_text == text && (twin_at[0] - at[0]).abs() < 0.1 && (twin_at[1] - at[1]).abs() < 0.1
    })
}

pub(crate) fn place_copper_texts(
    pcb: &mut Expr,
    routes: &[pcb_router::NetRoute],
    layer_names: &[String],
) -> Result<KiCadLabelReport, String> {
    let mut report = KiCadLabelReport { labels: 0, moved: 0, stuck: Vec::new() };
    let outline = outline::board_loops(pcb).ok().map(|loops| {
        loops.outline.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
            [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
        })
    });
    let inside = |bounds: Box2| outline.is_none_or(|board| bounds[0] >= board[0] + 0.5 && bounds[1] >= board[1] + 0.5 && bounds[2] <= board[2] - 0.5 && bounds[3] <= board[3] - 0.5);
    // Per copper layer: what a text must keep off (copper), and what it
    // had better keep off (the parts' bodies).
    let mut blocked: Vec<Vec<Box2>> = vec![Vec::new(); layer_names.len()];
    let mut bodies: Vec<Vec<Box2>> = vec![Vec::new(); layer_names.len()];
    let layer_index = |name: &str| layer_names.iter().position(|layer| layer == name);
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        let at = form_at(footprint)?;
        let place = |local: [f64; 2]| {
            let offset = rotate_vector(local, -at[2]);
            [at[0] + offset[0], at[1] + offset[1]]
        };
        // The courtyard, on the part's side.
        if let Ok((center, size, _)) = crate::board_placer::local_body(footprint) {
            let corners = [[-size[0] / 2.0, -size[1] / 2.0], [size[0] / 2.0, -size[1] / 2.0], [size[0] / 2.0, size[1] / 2.0], [-size[0] / 2.0, size[1] / 2.0]]
                .map(|corner| place([center[0] + corner[0], center[1] + corner[1]]));
            let body = corners.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
                [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
            });
            let side = if form_atom(footprint, "layer", 1) == Some("B.Cu") { "B.Cu" } else { "F.Cu" };
            if let Some(index) = layer_index(side) {
                bodies[index].push(body);
            }
        }
        for pad in footprint.children().iter().filter(|child| child.head() == Some("pad")) {
            let pad_at = form_at(pad)?;
            let center = place([pad_at[0], pad_at[1]]);
            let size = form_xy(pad, "size").unwrap_or([0.0, 0.0]);
            let (sin, cos) = (-pad_at[2]).to_radians().sin_cos();
            let half = [(cos.abs() * size[0] + sin.abs() * size[1]) / 2.0, (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0];
            let bounds = [center[0] - half[0], center[1] - half[1], center[0] + half[0], center[1] + half[1]];
            let layers: Vec<&str> = pad
                .child("layers")
                .map(|layers| layers.children().iter().skip(1).filter_map(Expr::atom).collect())
                .unwrap_or_default();
            for (index, name) in layer_names.iter().enumerate() {
                if layers.iter().any(|layer| *layer == name || matches!(*layer, "*.Cu" | "F&B.Cu")) {
                    blocked[index].push(bounds);
                }
            }
        }
    }
    for route in routes {
        for segment in &route.segments {
            let half = segment.width / 2.0;
            let bounds = [
                segment.start[0].min(segment.end[0]) - half,
                segment.start[1].min(segment.end[1]) - half,
                segment.start[0].max(segment.end[0]) + half,
                segment.start[1].max(segment.end[1]) + half,
            ];
            if let Some(list) = blocked.get_mut(segment.layer) {
                list.push(bounds);
            }
        }
        for via in &route.vias {
            let half = via.diameter / 2.0;
            for list in blocked.iter_mut() {
                list.push([via.at[0] - half, via.at[1] - half, via.at[0] + half, via.at[1] + half]);
            }
        }
    }
    // The board's own copper graphics (a decorative arc, a logo) are copper
    // too: Quanta75's moved texts landed on its arcs, 2 shorts.
    for item in pcb.children().iter().filter(|item| matches!(item.head(), Some("gr_line" | "gr_arc" | "gr_rect" | "gr_circle" | "gr_poly"))) {
        let Some(layer) = form_atom(item, "layer", 1).and_then(layer_index) else {
            continue;
        };
        if let Some(bounds) = board_router::copper_graphic_shapes(item)?.iter().map(pcb_router::Shape::aabb).reduce(pcb_router::geometry::Aabb::union) {
            blocked[layer].push([bounds.minimum[0], bounds.minimum[1], bounds.maximum[0], bounds.maximum[1]]);
        }
    }
    // A copper text with a twin on its side's mask layer (the same words
    // within 0.1 mm) is exposed-copper artwork: it stays where its designer
    // put it (routing kept clear of both halves); moving the copper half
    // alone left Quanta75's mask openings over our tracks (22 bridges).
    let mask_twins = mask_twins(pcb);
    let has_mask_twin = |item: &Expr, layer_name: &str| has_mask_twin(&mask_twins, item, layer_name);
    let mut taken: Vec<Vec<Box2>> = vec![Vec::new(); layer_names.len()];
    let mut text_moves: Vec<(usize, [f64; 2], bool)> = Vec::new();
    for (item_index, item) in pcb.children().iter().enumerate() {
        if item.head() != Some("gr_text") {
            continue;
        }
        let Some(layer_name) = form_atom(item, "layer", 1) else {
            continue;
        };
        let Some(layer) = layer_index(layer_name) else {
            continue;
        };
        let Some(bounds) = board_router::copper_graphic_shapes(item)?
            .iter()
            .map(pcb_router::Shape::aabb)
            .reduce(pcb_router::geometry::Aabb::union)
        else {
            continue;
        };
        report.labels += 1;
        let bounds = [bounds.minimum[0], bounds.minimum[1], bounds.maximum[0], bounds.maximum[1]];
        if has_mask_twin(item, layer_name) {
            taken[layer].push(bounds);
            continue;
        }
        let free = |bounds: Box2, taken: &[Box2], off_bodies: bool| {
            !blocked[layer].iter().any(|b| overlaps(bounds, *b, 0.3))
                && !taken.iter().any(|b| overlaps(bounds, *b, 0.3))
                && (!off_bodies || !bodies[layer].iter().any(|b| overlaps(bounds, *b, 0.1)))
        };
        if free(bounds, &taken[layer], true) {
            taken[layer].push(bounds);
            continue;
        }
        // Off the parts if a spot exists, else only off the copper; as it
        // is, else turned by a quarter.
        let center = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
        let half = [(bounds[2] - bounds[0]) / 2.0, (bounds[3] - bounds[1]) / 2.0];
        let mut found = None;
        'passes: for (off_bodies, turned) in [(true, false), (true, true), (false, false), (false, true)] {
            let half = if turned { [half[1], half[0]] } else { half };
            let boxed = |at: [f64; 2]| [at[0] - half[0], at[1] - half[1], at[0] + half[0], at[1] + half[1]];
            if !off_bodies && !turned && free(bounds, &taken[layer], false) {
                found = Some(([0.0, 0.0], bounds, false));
                break;
            }
            for ring in 0..=80 {
                let radius = ring as f64 * 0.5;
                let directions = if ring == 0 { 1 } else { 8 * ring.min(6) };
                for step in 0..directions {
                    let angle = step as f64 / directions as f64 * std::f64::consts::TAU;
                    let shift = [radius * angle.cos(), radius * angle.sin()];
                    let moved = boxed([center[0] + shift[0], center[1] + shift[1]]);
                    if inside(moved) && free(moved, &taken[layer], off_bodies) {
                        found = Some((shift, moved, turned));
                        break 'passes;
                    }
                }
            }
        }
        match found {
            Some((shift, moved, turned)) => {
                taken[layer].push(moved);
                text_moves.push((item_index, shift, turned));
            }
            None => {
                taken[layer].push(bounds);
                if std::env::var_os("PCB_LABELS_DEBUG").is_some() {
                    eprintln!("copper text {:?} stuck: box {:.1} x {:.1} mm at {:?}", item.children().get(1).and_then(Expr::atom), bounds[2] - bounds[0], bounds[3] - bounds[1], bounds);
                }
                report.stuck.push(item.children().get(1).and_then(Expr::atom).unwrap_or("").to_string());
            }
        }
    }
    let Expr::List(items) = pcb else {
        return Ok(report);
    };
    for (index, shift, turned) in text_moves {
        translate(&mut items[index], shift);
        if turned {
            turn_quarter(&mut items[index]);
        }
        report.moved += 1;
    }
    Ok(report)
}

/// Adds a quarter turn to a text's `at`.
fn turn_quarter(item: &mut Expr) {
    let Expr::List(children) = item else {
        return;
    };
    for child in children.iter_mut() {
        if child.head() != Some("at") {
            continue;
        }
        let Expr::List(values) = child else {
            continue;
        };
        let angle = values.get(3).and_then(Expr::atom).and_then(|atom| atom.parse::<f64>().ok()).unwrap_or(0.0);
        let turned = Expr::Atom(((angle + 90.0) % 360.0).to_string());
        if values.len() > 3 {
            values[3] = turned;
        } else {
            values.push(turned);
        }
    }
}

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
    // Capitals and digits: 0.65 of the height above the baseline; the box is
    // centred on the text's anchor for centred labels.
    [(width + thickness) / 2.0, (0.75 * size[0] + thickness) / 2.0]
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
            let bounds = [center[0] - half[0], center[1] - half[1], center[0] + half[0], center[1] + half[1]];
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
            let mut points = Vec::new();
            for head in ["start", "mid", "end", "center"] {
                if child.child(head).is_some() {
                    points.push(place(form_xy(child, head)?));
                }
            }
            if child.head() == Some("fp_circle") {
                let (center, end) = (form_xy(child, "center")?, form_xy(child, "end")?);
                let radius = distance_squared(center, end).sqrt();
                let middle = place(center);
                points.extend([[middle[0] - radius, middle[1] - radius], [middle[0] + radius, middle[1] + radius]]);
            }
            for point in child.child("pts").map(Expr::children).unwrap_or_default().iter().filter(|p| p.head() == Some("xy")) {
                points.push(place([expression_coordinate(point, 1, "silk x")?, expression_coordinate(point, 2, "silk y")?]));
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
                || child.children().get(1).and_then(Expr::atom) != Some("Reference")
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

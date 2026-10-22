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

/// Silkscreen layer of a label and the copper layer its pads are on.
fn silk_side(layer: &str) -> Option<bool> {
    match layer {
        "F.SilkS" => Some(false),
        "B.SilkS" => Some(true),
        _ => None,
    }
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
                        if let Some(number) = values
                            .get(index)
                            .and_then(Expr::atom)
                            .and_then(|atom| atom.parse::<f64>().ok())
                        {
                            values[index] =
                                Expr::Atom((((number + delta) * 1e6).round() / 1e6).to_string());
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
    /// Labels no clean spot was found for (kept at the spot with the
    /// fewest findings, or hidden).
    pub stuck: Vec<String>,
    /// The board's own texts and silkscreen graphics (`board_marks`) that
    /// parts' silkscreen, pads or labels lie on after placement, each with
    /// those parts: the marks stay where the input put them
    /// (`keep_board_marks`), so they are reported rather than moved.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub overlaps: Vec<String>,
    /// The board's marks that belong to a movable part (on it at its input
    /// position): no obstacles, left where the input put them while the
    /// part moved, not reported as overlaps. The mark, its layer and
    /// anchor, and the part.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attached: Vec<String>,
    /// The fields' placement: moved, blocked, hidden, KiCad's findings
    /// predicted (`silk_fields`).
    pub silkscreen: crate::silk_fields::KiCadSilkscreenReport,
}

/// One mark's overlaps as a line of the report: the mark, then each part
/// with what of it lies there.
fn overlap_line(
    mark: &crate::board_placer::BoardMark,
    hits: &BTreeMap<String, BTreeSet<&'static str>>,
) -> String {
    format!(
        "{} ({} at {:.2}, {:.2}): {}",
        mark.what,
        mark.layer,
        mark.at[0],
        mark.at[1],
        hits.iter()
            .map(|(reference, kinds)| format!(
                "{reference}'s {}",
                kinds.iter().copied().collect::<Vec<_>>().join(" and ")
            ))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Moves every visible reference label to the nearest free spot around
/// its footprint. With `keep_marks` the board's own silkscreen texts and
/// graphics stay where they are, labels keep off them, and the parts lying
/// on them are reported (`overlaps`; not the `fixed` ones, which lie where
/// the input put them); without, they move off the parts' silkscreen and
/// pads first (before 2026-10-21).
pub(crate) fn place_labels(
    pcb: &mut Expr,
    keep_marks: bool,
    fixed: &BTreeSet<String>,
    attached: &[String],
    options: crate::silk_fields::FieldOptions,
) -> Result<KiCadLabelReport, String> {
    let outline = outline::board_loops(pcb).ok().map(|loops| {
        loops.outline.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            },
        )
    });
    // What labels must stay off, per side: exposed copper (pads) and
    // silkscreen that is not a label.
    let mut blocked: [Vec<Box2>; 2] = [Vec::new(), Vec::new()];
    // Whose each blocked box is: the footprint and what of it.
    let mut owners: [Vec<(usize, &'static str)>; 2] = [Vec::new(), Vec::new()];
    let footprints: Vec<&Expr> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .collect();
    let references: Vec<String> = footprints
        .iter()
        .map(|footprint| crate::kicad_nets::footprint_reference(footprint).unwrap_or_default())
        .collect();
    for (index, footprint) in footprints.iter().enumerate() {
        // Every box pushed for this footprint below is its silkscreen but
        // its pads'.
        let owned = [blocked[0].len(), blocked[1].len()];
        let mut pad_boxes = [0usize; 2];
        let at = form_at(footprint)?;
        let place = |local: [f64; 2]| {
            let offset = rotate_vector(local, -at[2]);
            [at[0] + offset[0], at[1] + offset[1]]
        };
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
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
                .map(|layers| {
                    layers
                        .children()
                        .iter()
                        .skip(1)
                        .filter_map(Expr::atom)
                        .collect()
                })
                .unwrap_or_default();
            // A drill offset moves the copper away from the hole: the box
            // covers both.
            let offset = pad
                .child("drill")
                .and_then(|drill| drill.child("offset"))
                .and_then(|offset| {
                    Some([
                        expression_coordinate(offset, 1, "pad offset x").ok()?,
                        expression_coordinate(offset, 2, "pad offset y").ok()?,
                    ])
                })
                .map(|offset| rotate_vector(offset, -pad_at[2]))
                .unwrap_or([0.0, 0.0]);
            let copper = [center[0] + offset[0], center[1] + offset[1]];
            let bounds = [
                center[0].min(copper[0]) - half[0],
                center[1].min(copper[1]) - half[1],
                center[0].max(copper[0]) + half[0],
                center[1].max(copper[1]) + half[1],
            ];
            if layers
                .iter()
                .any(|layer| matches!(*layer, "F.Cu" | "*.Cu" | "F&B.Cu"))
            {
                blocked[0].push(bounds);
                pad_boxes[0] += 1;
            }
            if layers
                .iter()
                .any(|layer| matches!(*layer, "B.Cu" | "*.Cu" | "F&B.Cu"))
            {
                blocked[1].push(bounds);
                pad_boxes[1] += 1;
            }
        }
        for child in footprint.children() {
            let Some(back) = form_atom(child, "layer", 1).and_then(silk_side) else {
                continue;
            };
            // The footprint's own texts (a diode's "K"): everything but the
            // labels (reference and value), which move.
            let own_text = match child.head() {
                Some("fp_text") => {
                    !matches!(
                        child.children().get(1).and_then(Expr::atom),
                        Some("reference" | "value")
                    ) && crate::silk_fields::shown(child)
                }
                Some("property") => {
                    !matches!(
                        child.children().get(1).and_then(Expr::atom),
                        Some("Reference" | "Value")
                    ) && crate::silk_fields::shown(child)
                }
                _ => false,
            };
            if own_text {
                let text = child.children().get(2).and_then(Expr::atom).unwrap_or("");
                if !text.is_empty()
                    && let Ok(text_at) = form_at(child)
                    && let Some(bounds) = crate::silk_fields::text_box(
                        text,
                        &crate::silk_fields::TextStyle::of(child),
                        place([text_at[0], text_at[1]]),
                        text_at[2],
                    )
                {
                    blocked[back as usize].push(bounds);
                }
                continue;
            }
            let mut points: Vec<[f64; 2]> = outline::outline_points(child)?
                .into_iter()
                .map(&place)
                .collect();
            if child.child("center").is_some() {
                points.push(place(form_xy(child, "center")?));
            }
            if child.head() == Some("fp_circle") {
                let (center, end) = (form_xy(child, "center")?, form_xy(child, "end")?);
                let radius = distance_squared(center, end).sqrt();
                let middle = place(center);
                points.extend([
                    [middle[0] - radius, middle[1] - radius],
                    [middle[0] + radius, middle[1] + radius],
                ]);
            }
            for point in outline::pts_points(child)? {
                points.push(place(point));
            }
            if matches!(child.head(), Some("fp_line")) && points.len() == 2 {
                // A line is thin: its own box, not a box around it at an angle.
                let steps = (distance_squared(points[0], points[1]).sqrt() / 0.5)
                    .ceil()
                    .max(1.0) as usize;
                for step in 0..=steps {
                    let t = step as f64 / steps as f64;
                    let p = [
                        points[0][0] + t * (points[1][0] - points[0][0]),
                        points[0][1] + t * (points[1][1] - points[0][1]),
                    ];
                    blocked[back as usize].push([p[0] - 0.1, p[1] - 0.1, p[0] + 0.1, p[1] + 0.1]);
                }
            } else if matches!(
                child.head(),
                Some("fp_rect" | "fp_circle" | "fp_poly" | "fp_arc")
            ) && !points.is_empty()
            {
                let bounds = points.iter().fold(
                    [
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ],
                    |b, p| {
                        [
                            b[0].min(p[0]),
                            b[1].min(p[1]),
                            b[2].max(p[0]),
                            b[3].max(p[1]),
                        ]
                    },
                );
                if child.head() == Some("fp_rect")
                    && child.child("fill").is_none_or(|fill| {
                        fill.children().get(1).and_then(Expr::atom) != Some("yes")
                    })
                {
                    // An outline: its four sides.
                    for side in [
                        [bounds[0], bounds[1], bounds[2], bounds[1]],
                        [bounds[0], bounds[3], bounds[2], bounds[3]],
                        [bounds[0], bounds[1], bounds[0], bounds[3]],
                        [bounds[2], bounds[1], bounds[2], bounds[3]],
                    ] {
                        blocked[back as usize].push([
                            side[0] - 0.1,
                            side[1] - 0.1,
                            side[2] + 0.1,
                            side[3] + 0.1,
                        ]);
                    }
                } else {
                    blocked[back as usize].push(bounds);
                }
            }
        }
        // The pads come first, then the silkscreen.
        for side in 0..2 {
            for number in owned[side]..blocked[side].len() {
                owners[side].push((
                    index,
                    if number < owned[side] + pad_boxes[side] {
                        "pads"
                    } else {
                        "silkscreen"
                    },
                ));
            }
        }
    }
    let mut report = KiCadLabelReport {
        attached: attached.to_vec(),
        ..Default::default()
    };
    let mut taken: [Vec<Box2>; 2] = [Vec::new(), Vec::new()];
    let inside = |bounds: Box2| {
        outline.is_none_or(|board| {
            bounds[0] >= board[0] + 0.3
                && bounds[1] >= board[1] + 0.3
                && bounds[2] <= board[2] - 0.3
                && bounds[3] <= board[3] - 0.3
        })
    };

    let mut artwork_moves: Vec<(Vec<usize>, [f64; 2])> = Vec::new();
    let mut text_moves: Vec<(usize, [f64; 2])> = Vec::new();
    // The board's own silkscreen texts and graphics stay; labels keep off
    // them.
    let marks: Vec<crate::board_placer::BoardMark> = if keep_marks {
        crate::board_placer::board_marks(pcb, outline)?
            .into_iter()
            .filter(|mark| !mark.copper)
            .collect()
    } else {
        Vec::new()
    };
    for mark in &marks {
        taken[mark.back as usize].extend(mark.boxes.iter().copied());
    }
    if !keep_marks {
        // Board silkscreen artwork (a logo drawn as polygons and lines) moves as
        // one piece: shapes within 0.5 mm of each other form a cluster.
        let mut graphics: Vec<(usize, bool, Box2)> = Vec::new();
        for (item_index, item) in pcb.children().iter().enumerate() {
            let Some(back) = form_atom(item, "layer", 1).and_then(silk_side) else {
                continue;
            };
            if !matches!(
                item.head(),
                Some("gr_poly" | "gr_line" | "gr_rect" | "gr_circle" | "gr_arc" | "gr_curve")
            ) {
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
            let bounds = points.iter().fold(
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ],
                |b, p| {
                    [
                        b[0].min(p[0]),
                        b[1].min(p[1]),
                        b[2].max(p[0]),
                        b[3].max(p[1]),
                    ]
                },
            );
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
        for (members, back, bounds) in clusters {
            let side = back as usize;
            // A frame around the whole board is decoration of the board, not a
            // piece to move.
            if outline.is_some_and(|board| (bounds[2] - bounds[0]) > 0.8 * (board[2] - board[0])) {
                continue;
            }
            let free = |bounds: Box2, taken: &[Box2]| {
                !blocked[side].iter().any(|b| overlaps(bounds, *b, 0.1))
                    && !taken.iter().any(|b| overlaps(bounds, *b, 0.2))
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
                    let moved = [
                        bounds[0] + shift[0],
                        bounds[1] + shift[1],
                        bounds[2] + shift[0],
                        bounds[3] + shift[1],
                    ];
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
            let bounds = [
                bounds.minimum[0],
                bounds.minimum[1],
                bounds.maximum[0],
                bounds.maximum[1],
            ];
            let side = back as usize;
            let free = |bounds: Box2, taken: &[Box2]| {
                !blocked[side].iter().any(|b| overlaps(bounds, *b, 0.1))
                    && !taken.iter().any(|b| overlaps(bounds, *b, 0.2))
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
                    let moved = [
                        bounds[0] + shift[0],
                        bounds[1] + shift[1],
                        bounds[2] + shift[0],
                        bounds[3] + shift[1],
                    ];
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
                    report.stuck.push(
                        item.children()
                            .get(1)
                            .and_then(Expr::atom)
                            .unwrap_or("")
                            .to_string(),
                    );
                }
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
        set_form_at(
            &mut items[item_index],
            [at[0] + shift[0], at[1] + shift[1], at[2]],
        )?;
        report.moved += 1;
    }
    // The fields: where KiCad's silkscreen checks find nothing (the
    // board's texts and graphics, moved or not, are obstacles).
    let fields = crate::silk_fields::place_fields(pcb, options)?;
    report.labels = fields.report.fields;
    report.moved += fields.report.moved;
    report.stuck = fields.blocked_texts.clone();
    // The parts on the board's marks: their pads, silkscreen and the
    // labels that found no free spot.
    for mark in &marks {
        let key = mark.key();
        if attached.iter().any(|line| {
            line.strip_prefix(key.as_str())
                .is_some_and(|rest| rest.starts_with(':'))
        }) {
            continue;
        }
        let side = mark.back as usize;
        let on_mark = |b: Box2| mark.boxes.iter().any(|m| overlaps(*m, b, 0.0));
        let mut hits: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
        for (b, (footprint, kind)) in blocked[side].iter().zip(&owners[side]) {
            if on_mark(*b) && !fixed.contains(&references[*footprint]) {
                hits.entry(references[*footprint].clone())
                    .or_default()
                    .insert(kind);
            }
        }
        for (footprint, back, b) in &fields.blocked_boxes {
            if *back == mark.back && on_mark(*b) && !fixed.contains(&references[*footprint]) {
                hits.entry(references[*footprint].clone())
                    .or_default()
                    .insert("label");
            }
        }
        if !hits.is_empty() {
            report.overlaps.push(overlap_line(mark, &hits));
        }
    }

    report.silkscreen = fields.report;
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
            matches!(layer, "F.Mask" | "B.Mask")
                .then(|| (layer.to_string(), text.to_string(), [at[0], at[1]]))
        })
        .collect()
}

/// Whether a copper text on `layer_name` has a twin on its side's mask
/// layer (the same words within 0.1 mm): exposed-copper artwork, which
/// stays where its designer put it.
pub(crate) fn has_mask_twin(
    twins: &[(String, String, [f64; 2])],
    item: &Expr,
    layer_name: &str,
) -> bool {
    let mask = match layer_name {
        "F.Cu" => "F.Mask",
        "B.Cu" => "B.Mask",
        _ => return false,
    };
    let (Some(text), Ok(at)) = (item.children().get(1).and_then(Expr::atom), form_at(item)) else {
        return false;
    };
    twins.iter().any(|(twin_layer, twin_text, twin_at)| {
        twin_layer == mask
            && twin_text == text
            && (twin_at[0] - at[0]).abs() < 0.1
            && (twin_at[1] - at[1]).abs() < 0.1
    })
}

///
/// With `keep_marks` they stay where the input put them (the router kept
/// clear of them); a part's pads or body or a track on one is reported
/// (`overlaps`) instead.
pub(crate) fn place_copper_texts(
    pcb: &mut Expr,
    routes: &[pcb_router::NetRoute],
    layer_names: &[String],
    keep_marks: bool,
    fixed: &BTreeSet<String>,
) -> Result<KiCadLabelReport, String> {
    let mut report = KiCadLabelReport::default();
    let outline = outline::board_loops(pcb).ok().map(|loops| {
        loops.outline.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            },
        )
    });
    let inside = |bounds: Box2| {
        outline.is_none_or(|board| {
            bounds[0] >= board[0] + 0.5
                && bounds[1] >= board[1] + 0.5
                && bounds[2] <= board[2] - 0.5
                && bounds[3] <= board[3] - 0.5
        })
    };
    // Per copper layer: what a text must keep off (copper), and what it
    // had better keep off (the parts' bodies).
    let mut blocked: Vec<Vec<Box2>> = vec![Vec::new(); layer_names.len()];
    let mut bodies: Vec<Vec<Box2>> = vec![Vec::new(); layer_names.len()];
    // Whose each pad and body box is (the reference), for the overlaps.
    let mut pad_owners: Vec<Vec<String>> = vec![Vec::new(); layer_names.len()];
    let mut body_owners: Vec<Vec<String>> = vec![Vec::new(); layer_names.len()];
    let layer_index = |name: &str| layer_names.iter().position(|layer| layer == name);
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = crate::kicad_nets::footprint_reference(footprint).unwrap_or_default();
        let at = form_at(footprint)?;
        let place = |local: [f64; 2]| {
            let offset = rotate_vector(local, -at[2]);
            [at[0] + offset[0], at[1] + offset[1]]
        };
        // The courtyard, on the part's side.
        if let Ok((center, size, _)) = crate::board_placer::local_body(footprint) {
            let corners = [
                [-size[0] / 2.0, -size[1] / 2.0],
                [size[0] / 2.0, -size[1] / 2.0],
                [size[0] / 2.0, size[1] / 2.0],
                [-size[0] / 2.0, size[1] / 2.0],
            ]
            .map(|corner| place([center[0] + corner[0], center[1] + corner[1]]));
            let body = corners.iter().fold(
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ],
                |b, p| {
                    [
                        b[0].min(p[0]),
                        b[1].min(p[1]),
                        b[2].max(p[0]),
                        b[3].max(p[1]),
                    ]
                },
            );
            let side = if form_atom(footprint, "layer", 1) == Some("B.Cu") {
                "B.Cu"
            } else {
                "F.Cu"
            };
            if let Some(index) = layer_index(side) {
                bodies[index].push(body);
                body_owners[index].push(reference.clone());
            }
        }
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            let pad_at = form_at(pad)?;
            let center = place([pad_at[0], pad_at[1]]);
            let size = form_xy(pad, "size").unwrap_or([0.0, 0.0]);
            let (sin, cos) = (-pad_at[2]).to_radians().sin_cos();
            let half = [
                (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0,
                (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0,
            ];
            let bounds = [
                center[0] - half[0],
                center[1] - half[1],
                center[0] + half[0],
                center[1] + half[1],
            ];
            let layers: Vec<&str> = pad
                .child("layers")
                .map(|layers| {
                    layers
                        .children()
                        .iter()
                        .skip(1)
                        .filter_map(Expr::atom)
                        .collect()
                })
                .unwrap_or_default();
            for (index, name) in layer_names.iter().enumerate() {
                if layers
                    .iter()
                    .any(|layer| *layer == name || matches!(*layer, "*.Cu" | "F&B.Cu"))
                {
                    blocked[index].push(bounds);
                    pad_owners[index].push(reference.clone());
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
                list.push([
                    via.at[0] - half,
                    via.at[1] - half,
                    via.at[0] + half,
                    via.at[1] + half,
                ]);
            }
        }
    }
    // The board's own copper graphics (a decorative arc, a logo) are copper
    // too: Quanta75's moved texts landed on its arcs, 2 shorts.
    for item in pcb.children().iter().filter(|item| {
        matches!(
            item.head(),
            Some("gr_line" | "gr_arc" | "gr_rect" | "gr_circle" | "gr_poly")
        )
    }) {
        let Some(layer) = form_atom(item, "layer", 1).and_then(layer_index) else {
            continue;
        };
        if let Some(bounds) = board_router::copper_graphic_shapes(item)?
            .iter()
            .map(pcb_router::Shape::aabb)
            .reduce(pcb_router::geometry::Aabb::union)
        {
            blocked[layer].push([
                bounds.minimum[0],
                bounds.minimum[1],
                bounds.maximum[0],
                bounds.maximum[1],
            ]);
        }
    }
    // A copper text with a twin on its side's mask layer (the same words
    // within 0.1 mm) is exposed-copper artwork: it stays where its designer
    // put it (routing kept clear of both halves); moving the copper half
    // alone left Quanta75's mask openings over our tracks (22 bridges).
    let mask_twins = mask_twins(pcb);
    let has_mask_twin =
        |item: &Expr, layer_name: &str| has_mask_twin(&mask_twins, item, layer_name);
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
        let bounds = [
            bounds.minimum[0],
            bounds.minimum[1],
            bounds.maximum[0],
            bounds.maximum[1],
        ];
        if has_mask_twin(item, layer_name) {
            taken[layer].push(bounds);
            continue;
        }
        if keep_marks {
            // Pads and bodies by their parts; the rest of `blocked` after
            // the pads is the routes' copper and the board's graphics.
            let mut hits: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
            for (number, b) in blocked[layer].iter().enumerate() {
                if overlaps(bounds, *b, 0.0) {
                    match pad_owners[layer].get(number) {
                        Some(reference) if fixed.contains(reference) => false,
                        Some(reference) => {
                            hits.entry(reference.clone()).or_default().insert("pads")
                        }
                        None => hits
                            .entry("the routing".into())
                            .or_default()
                            .insert("copper"),
                    };
                }
            }
            for (b, reference) in bodies[layer].iter().zip(&body_owners[layer]) {
                if overlaps(bounds, *b, 0.0) && !fixed.contains(reference) {
                    hits.entry(reference.clone()).or_default().insert("body");
                }
            }
            taken[layer].push(bounds);
            if !hits.is_empty() {
                let at = form_at(item)?;
                let text = crate::board_placer::mark_text(item).unwrap_or_default();
                report.overlaps.push(format!(
                    "copper text {text:?} ({layer_name} at {:.2}, {:.2}): {}",
                    at[0],
                    at[1],
                    hits.iter()
                        .map(|(reference, kinds)| format!(
                            "{reference}'s {}",
                            kinds.iter().copied().collect::<Vec<_>>().join(" and ")
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
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
        'passes: for (off_bodies, turned) in
            [(true, false), (true, true), (false, false), (false, true)]
        {
            let half = if turned { [half[1], half[0]] } else { half };
            let boxed = |at: [f64; 2]| {
                [
                    at[0] - half[0],
                    at[1] - half[1],
                    at[0] + half[0],
                    at[1] + half[1],
                ]
            };
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
                    eprintln!(
                        "copper text {:?} stuck: box {:.1} x {:.1} mm at {:?}",
                        item.children().get(1).and_then(Expr::atom),
                        bounds[2] - bounds[0],
                        bounds[3] - bounds[1],
                        bounds
                    );
                }
                report.stuck.push(
                    item.children()
                        .get(1)
                        .and_then(Expr::atom)
                        .unwrap_or("")
                        .to_string(),
                );
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
        let angle = values
            .get(3)
            .and_then(Expr::atom)
            .and_then(|atom| atom.parse::<f64>().ok())
            .unwrap_or(0.0);
        let turned = Expr::Atom(((angle + 90.0) % 360.0).to_string());
        if values.len() > 3 {
            values[3] = turned;
        } else {
            values.push(turned);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTIONS: crate::silk_fields::FieldOptions = crate::silk_fields::FieldOptions {
        rules: crate::silk_fields::SilkRules {
            clearance: 0.0,
            min_text_height: 0.8,
            min_text_thickness: 0.08,
        },
        shrink: true,
        hide_when_blocked: false,
    };

    /// A test pad's name and a resistor placed on it; a back text no part
    /// lies on.
    const BOARD: &str = r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
        (gr_text "VDD" (at 20 10 0) (layer "F.SilkS") (effects (font (size 0.8 0.8) (thickness 0.13))))
        (gr_text "STACK" (at 10 5 0) (layer "B.SilkS") (effects (font (size 0.8 0.8) (thickness 0.13)) (justify mirror)))
        (footprint "R" (layer "F.Cu") (at 20 10.3)
            (property "Reference" "R22" (at 0 -0.6 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (fp_line (start -1.5 -0.8) (end 1.5 -0.8) (layer "F.SilkS") (stroke (width 0.12) (type solid)))
            (fp_rect (start -1.5 -0.8) (end 1.5 0.8) (layer "F.CrtYd"))
            (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "F.Cu"))
            (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "F.Cu"))))"#;

    fn text_at(pcb: &Expr, text: &str) -> [f64; 3] {
        let item = pcb
            .children()
            .iter()
            .find(|item| {
                item.head() == Some("gr_text")
                    && item.children().get(1).and_then(Expr::atom) == Some(text)
            })
            .unwrap();
        form_at(item).unwrap()
    }

    #[test]
    fn the_polish_keeps_board_texts_and_reports_the_parts_on_them() {
        let mut pcb = parse(BOARD).unwrap();
        let report = place_labels(&mut pcb, true, &BTreeSet::new(), &[], OPTIONS).unwrap();
        assert_eq!(text_at(&pcb, "VDD"), [20.0, 10.0, 0.0]);
        assert_eq!(text_at(&pcb, "STACK"), [10.0, 5.0, 0.0]);
        assert_eq!(report.overlaps.len(), 1, "{:?}", report.overlaps);
        assert!(
            report.overlaps[0]
                .starts_with("text \"VDD\" (F.SilkS at 20.00, 10.00): R22's pads and silkscreen"),
            "{:?}",
            report.overlaps
        );
        // The reference label moved off the text.
        assert_eq!(report.moved, 1);
        // A fixed part lay there in the input: not reported.
        let mut pcb = parse(BOARD).unwrap();
        let fixed = BTreeSet::from(["R22".to_string()]);
        assert!(
            place_labels(&mut pcb, true, &fixed, &[], OPTIONS)
                .unwrap()
                .overlaps
                .is_empty()
        );
        // Before 2026-10-21 the text moved off the part.
        let mut pcb = parse(BOARD).unwrap();
        let report = place_labels(&mut pcb, false, &BTreeSet::new(), &[], OPTIONS).unwrap();
        assert_ne!(text_at(&pcb, "VDD"), [20.0, 10.0, 0.0]);
        assert!(report.overlaps.is_empty());
    }

    #[test]
    fn a_copper_text_stays_and_parts_on_it_are_reported() {
        let board = BOARD.replace(
            r#"(gr_text "VDD" (at 20 10 0) (layer "F.SilkS")"#,
            r#"(gr_text "VDD" (at 20 10 0) (layer "F.Cu")"#,
        );
        let layers = ["F.Cu".to_string(), "B.Cu".to_string()];
        let mut pcb = parse(&board).unwrap();
        let report = place_copper_texts(&mut pcb, &[], &layers, true, &BTreeSet::new()).unwrap();
        assert_eq!(text_at(&pcb, "VDD"), [20.0, 10.0, 0.0]);
        assert_eq!(
            report.overlaps,
            ["copper text \"VDD\" (F.Cu at 20.00, 10.00): R22's body and pads"]
        );
        let mut pcb = parse(&board).unwrap();
        let report = place_copper_texts(&mut pcb, &[], &layers, false, &BTreeSet::new()).unwrap();
        assert_ne!(text_at(&pcb, "VDD"), [20.0, 10.0, 0.0]);
        assert_eq!(report.moved, 1);
    }
}

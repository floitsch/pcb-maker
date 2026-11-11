// Copyright (C) 2026 Toit contributors.

//! Shapes of footprint graphics, texts and rule areas, and the board's pours.

use super::*;
use pcb_router as core;

/// A shape in a footprint's frame, placed on the board.
pub(super) fn place_shape(shape: core::Shape, at: [f64; 3]) -> core::Shape {
    let place = |point: [f64; 2]| {
        let offset = rotate_vector(point, -at[2]);
        [at[0] + offset[0], at[1] + offset[1]]
    };
    match shape {
        core::Shape::Circle { center, radius } => core::Shape::Circle {
            center: place(center),
            radius,
        },
        core::Shape::Capsule { start, end, radius } => core::Shape::Capsule {
            start: place(start),
            end: place(end),
            radius,
        },
        core::Shape::Polygon { points } => core::Shape::Polygon {
            points: points.into_iter().map(place).collect(),
        },
        core::Shape::Union { parts } => core::Shape::Union {
            parts: parts
                .into_iter()
                .map(|part| place_shape(part, at))
                .collect(),
        },
    }
}

/// Copper-layer graphics as obstacle shapes. Text becomes its (generous)
/// bounding box. Footprint graphics (`fp_*`) come out in the footprint's
/// frame; `place_shape` puts them on the board.
pub fn copper_graphic_shapes(item: &Expr) -> Result<Vec<core::Shape>, String> {
    let width = item
        .child("stroke")
        .and_then(|stroke| form_f64(stroke, "width", 1).ok())
        .or_else(|| form_f64(item, "width", 1).ok())
        .unwrap_or(0.1);
    let capsule = |start: [f64; 2], end: [f64; 2]| core::Shape::Capsule {
        start,
        end,
        radius: width / 2.0,
    };
    let shapes = match item.head() {
        Some("gr_line" | "fp_line") => {
            vec![capsule(form_xy(item, "start")?, form_xy(item, "end")?)]
        }
        // The chords lie inside the arc by up to the flattening's
        // tolerance: the stroke grows by it, so the capsules hold the
        // true arc (name-card's antenna AE1, a 10.7 mm arc: a track
        // 0.1978 mm from it under 0.2).
        Some("gr_arc" | "fp_arc") => outline::arc_points(
            form_xy(item, "start")?,
            form_xy(item, "mid")?,
            form_xy(item, "end")?,
        )
        .windows(2)
        .map(|pair| core::Shape::Capsule {
            start: pair[0],
            end: pair[1],
            radius: width / 2.0 + outline::ARC_TOLERANCE,
        })
        .collect(),
        Some("gr_rect" | "fp_rect") => {
            let (a, b) = (form_xy(item, "start")?, form_xy(item, "end")?);
            let corners = [a, [b[0], a[1]], b, [a[0], b[1]]];
            let filled = !matches!(form_atom(item, "fill", 1), None | Some("none" | "no"));
            // A filled rectangle's edges matter only with a stroke reaching
            // beyond the fill.
            let mut shapes: Vec<_> = if filled && width <= 0.0 {
                Vec::new()
            } else {
                (0..4)
                    .map(|index| capsule(corners[index], corners[(index + 1) % 4]))
                    .collect()
            };
            if filled {
                shapes.push(core::Shape::Polygon {
                    points: corners.to_vec(),
                });
            }
            shapes
        }
        Some("gr_circle" | "fp_circle") => {
            let center = form_xy(item, "center")?;
            let radius = distance_squared(center, form_xy(item, "end")?).sqrt();
            let filled = !matches!(form_atom(item, "fill", 1), None | Some("no" | "none"));
            if filled || width <= 0.0 {
                vec![core::Shape::Circle {
                    center,
                    radius: radius + width / 2.0,
                }]
            } else {
                // Unfilled: the ring only.
                outline::circle_points(center, radius)
                    .windows(2)
                    .map(|pair| core::Shape::Capsule {
                        start: pair[0],
                        end: pair[1],
                        radius: width / 2.0 + 2.0 * outline::ARC_TOLERANCE,
                    })
                    .collect()
            }
        }
        Some("gr_poly" | "fp_poly") => {
            let points = rule_area_like_points(item)?;
            // Old polygons have no fill form and are filled; the stroke
            // reaches half its width beyond the outline (SNSP's J2 mask
            // opening: a via 0.006 mm off the bare outline bridged to it).
            let filled = !matches!(form_atom(item, "fill", 1), Some("no" | "none"));
            let mut shapes: Vec<_> = if width > 0.0 || !filled {
                (0..points.len())
                    .map(|index| capsule(points[index], points[(index + 1) % points.len()]))
                    .collect()
            } else {
                Vec::new()
            };
            if filled {
                shapes.push(core::Shape::Polygon { points });
            }
            shapes
        }
        // Text in a TrueType face carries its glyphs as polygons.
        Some("gr_text")
            if item.child("render_cache").is_some_and(|cache| {
                cache
                    .children()
                    .iter()
                    .any(|child| child.head() == Some("polygon"))
            }) =>
        {
            let cache = item.child("render_cache").unwrap();
            cache
                .children()
                .iter()
                .filter(|child| child.head() == Some("polygon"))
                .map(|polygon| {
                    Ok(core::Shape::Polygon {
                        points: rule_area_like_points(polygon)?,
                    })
                })
                .collect::<Result<_, String>>()?
        }
        Some("gr_text") => vec![stroke_text_box(item)?],
        _ => Vec::new(),
    };
    // A knockout text is a copper plate with the glyphs cut out: its
    // bounding box, grown by the stroke, is the copper (Quanta75's 7 mm
    // "0bsilab" on F.Cu: tracks and pads landed inside the plate, shorts
    // and mask bridges).
    let knockout = matches!(item.head(), Some("gr_text" | "fp_text"))
        && item.child("layer").is_some_and(|layer| {
            layer
                .children()
                .iter()
                .skip(2)
                .any(|child| child.atom() == Some("knockout"))
        });
    if knockout && !shapes.is_empty() {
        let thickness = item
            .child("effects")
            .and_then(|effects| effects.child("font"))
            .and_then(|font| form_f64(font, "thickness", 1).ok())
            .unwrap_or(0.15);
        let bounds = shapes
            .iter()
            .skip(1)
            .fold(shapes[0].aabb(), |bounds, shape| bounds.union(shape.aabb()));
        let (low, high) = (bounds.minimum, bounds.maximum);
        return Ok(vec![core::Shape::Polygon {
            points: vec![
                [low[0] - thickness, low[1] - thickness],
                [high[0] + thickness, low[1] - thickness],
                [high[0] + thickness, high[1] + thickness],
                [low[0] - thickness, high[1] + thickness],
            ],
        }]);
    }
    Ok(shapes)
}

/// How far KiCad 10.0.7's plate of a knockout text reaches past its
/// glyphs: the larger of half the pen (the thickness, clamped to a quarter
/// of the smaller of the text's height and width, as KiCad clamps a stroke
/// text's pen) and a ninth of the height (pcbnew's polygons of 3150 texts
/// made for it: 7 mm at 0.6, Quanta75's '0bsilab', 0.778 mm all round; 3 by
/// 5 mm at 1.0, 0.375). It was taken as the thickness, under the ninth of
/// the height for any text thinner than a fifth of its height.
fn knockout_margin(item: &Expr) -> f64 {
    let font = item
        .child("effects")
        .and_then(|effects| effects.child("font"));
    let size = font
        .and_then(|font| form_xy(font, "size").ok())
        .unwrap_or([1.0, 1.0]);
    let thickness = font
        .and_then(|font| form_f64(font, "thickness", 1).ok())
        .unwrap_or(0.15);
    let pen = thickness.min(0.25 * size[0].min(size[1]));
    (pen / 2.0).max(size[0] / 9.0)
}

/// The shapes of a board graphic as KiCad draws it, on a mask layer for
/// its bridge test and on a copper layer for its clearance. A plain
/// (stroke font) text is its stroke box (`plain_text_box`); a cached
/// (TrueType) text the box KiCad's glyphs fill in the face it draws
/// (`drawn_text_box`, measured on pcbnew's polygons), and its cached
/// glyphs beside it: KiCad draws a cached
/// text from the font, not from the cache, and a face it does not have is
/// replaced (OpenESC's "ESC" in Tokyo on B.Mask, drawn in a stand-in 2 mm
/// wider than its cache; the nine cached mask texts of the corpus reach
/// 0.7 to 2.9 mm past their caches). A knockout text is its plate: that
/// box and a cached text's glyphs' box, each grown by KiCad's margin
/// (`knockout_margin`) in the text's frame (Quanta75's '0bsilab' in
/// "Blrrpix Tweak", drawn in Noto Sans Bold: pcbnew's plate x 81.0 to
/// 116.4 and y 82.8 to 91.9 mm where the cache's glyphs span 84.8 to 112.8
/// and 84.2 to 91.0; our I2C1-SDA track inside it, then a via). Other
/// graphics as `copper_graphic_shapes`.
pub(super) fn drawn_graphic_shapes(item: &Expr) -> Result<Vec<core::Shape>, String> {
    if item.head() != Some("gr_text") {
        return copper_graphic_shapes(item);
    }
    let text = item.children().get(1).and_then(Expr::atom).unwrap_or("");
    let cached = item.child("render_cache").is_some_and(|cache| {
        cache
            .children()
            .iter()
            .any(|child| child.head() == Some("polygon"))
    });
    let knockout = item.child("layer").is_some_and(|layer| {
        layer
            .children()
            .iter()
            .skip(2)
            .any(|child| child.atom() == Some("knockout"))
    });
    // The box of before (the stroke font's, and a knockout's plate grown
    // by the thickness) stays where it holds KiCad's drawn box: a change
    // of a text's shape moves routes (`covers`).
    let stroke = stroke_text_box_of(item, text)?;
    if knockout {
        let margin = knockout_margin(item);
        let mut plate = vec![drawn_text_box(item, text, margin)?];
        if cached {
            plate.extend(cached_glyph_box(item, margin)?);
        }
        let thickness = text_thickness(item);
        let before = copper_graphic_shapes(item)?
            .iter()
            .fold(stroke.aabb().inflated(thickness), |bounds, shape| {
                bounds.union(shape.aabb())
            });
        let before = core::Shape::Polygon {
            points: vec![
                before.minimum,
                [before.maximum[0], before.minimum[1]],
                before.maximum,
                [before.minimum[0], before.maximum[1]],
            ],
        };
        let mut glyphs = vec![drawn_text_extent(item, text, margin)?];
        if cached {
            glyphs.extend(cached_glyph_box(item, margin)?);
        }
        if covers(&before, &glyphs) {
            return Ok(vec![before]);
        }
        return Ok(plate);
    }
    if !cached {
        return Ok(vec![plain_text_box(item, text)?]);
    }
    let mut shapes = copper_graphic_shapes(item)?;
    let drawn = drawn_text_box(item, text, 0.0)?;
    shapes.push(if covers(&stroke, &[drawn_text_extent(item, text, 0.0)?]) {
        stroke
    } else {
        drawn
    });
    Ok(shapes)
}

/// `drawn_text_box` without its room on every side: where KiCad's glyphs
/// reach by the measurements (the test of `covers`).
fn drawn_text_extent(item: &Expr, text: &str, grow: f64) -> Result<core::Shape, String> {
    let height = item
        .child("effects")
        .and_then(|effects| effects.child("font"))
        .and_then(|font| form_xy(font, "size").ok())
        .map_or(1.0, |size| size[0]);
    drawn_text_box(item, text, grow - 0.02 * height)
}

/// `text` without its text variables (`${...}`).
fn without_variables(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        match rest[start..].find('}') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether `outer` holds every corner of `inner` (polygons: the text
/// boxes and plates).
fn covers(outer: &core::Shape, inner: &[core::Shape]) -> bool {
    inner.iter().all(|shape| match shape {
        core::Shape::Polygon { points } => points.iter().all(|point| outer.contains(*point)),
        _ => false,
    })
}

/// A text's thickness (KiCad's default 0.15 mm without one).
fn text_thickness(item: &Expr) -> f64 {
    item.child("effects")
        .and_then(|effects| effects.child("font"))
        .and_then(|font| form_f64(font, "thickness", 1).ok())
        .unwrap_or(0.15)
}

/// A cached text's glyphs' box in the text's frame, grown by `grow`.
fn cached_glyph_box(item: &Expr, grow: f64) -> Result<Option<core::Shape>, String> {
    let Some(cache) = item.child("render_cache") else {
        return Ok(None);
    };
    let at = form_at(item)?;
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for polygon in cache
        .children()
        .iter()
        .filter(|child| child.head() == Some("polygon"))
    {
        for point in rule_area_like_points(polygon)? {
            let local = rotate_vector([point[0] - at[0], point[1] - at[1]], at[2]);
            bounds = [
                bounds[0].min(local[0]),
                bounds[1].min(local[1]),
                bounds[2].max(local[0]),
                bounds[3].max(local[1]),
            ];
        }
    }
    if !bounds[0].is_finite() {
        return Ok(None);
    }
    let middle = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
    let offset = rotate_vector(middle, -at[2]);
    Ok(Some(core::Shape::rectangle(
        [at[0] + offset[0], at[1] + offset[1]],
        [
            (bounds[2] - bounds[0]) / 2.0 + grow,
            (bounds[3] - bounds[1]) / 2.0 + grow,
        ],
        -at[2],
    )))
}

/// A board graphic on a mask layer as KiCad's bridge test sees it
/// (`drawn_graphic_shapes`).
pub(super) fn mask_graphic_shapes(item: &Expr) -> Result<Vec<core::Shape>, String> {
    drawn_graphic_shapes(item)
}

/// A TrueType text on copper as KiCad 10.0.7 draws it: from the font
/// installed, not from the file's cache, whose glyphs it only roughly
/// follows (TomKeddie's six-line "SAOs are the property of ..." in
/// Cantarell bold, 5.8 mm: KiCad's glyphs up to 0.92 mm outside the cached
/// ones, median 0.14, tracks 0.22 mm from the cache 0.02 from KiCad's).
/// Each line of the cache is a box, in the text's frame, grown by a tenth
/// of the text's height; the lines' gaps stay open. Nothing for a stroke
/// text (its box is `copper_graphic_shapes`'s).
pub(super) fn cached_text_line_boxes(item: &Expr) -> Result<Vec<core::Shape>, String> {
    let Some(cache) = item.child("render_cache") else {
        return Ok(Vec::new());
    };
    let at = form_at(item)?;
    let height = item
        .child("effects")
        .and_then(|effects| effects.child("font"))
        .and_then(|font| form_xy(font, "size").ok())
        .map_or(1.0, |size| size[0]);
    // Each glyph polygon's extent in the text's frame (its baseline along
    // x).
    let mut extents: Vec<[f64; 4]> = cache
        .children()
        .iter()
        .filter(|child| child.head() == Some("polygon"))
        .map(|polygon| {
            let points = rule_area_like_points(polygon)?;
            Ok(points.iter().fold(
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ],
                |bounds, point| {
                    let local = rotate_vector([point[0] - at[0], point[1] - at[1]], at[2]);
                    [
                        bounds[0].min(local[0]),
                        bounds[1].min(local[1]),
                        bounds[2].max(local[0]),
                        bounds[3].max(local[1]),
                    ]
                },
            ))
        })
        .collect::<Result<_, String>>()?;
    extents.sort_by(|a, b| a[1].total_cmp(&b[1]));
    // Glyphs whose heights overlap are one line.
    let mut lines: Vec<[f64; 4]> = Vec::new();
    for extent in extents {
        match lines.last_mut() {
            Some(line) if extent[1] < line[3] => {
                *line = [
                    line[0].min(extent[0]),
                    line[1],
                    line[2].max(extent[2]),
                    line[3].max(extent[3]),
                ];
            }
            _ => lines.push(extent),
        }
    }
    let margin = 0.1 * height;
    Ok(lines
        .into_iter()
        .map(|[low_x, low_y, high_x, high_y]| {
            let corners = [
                [low_x - margin, low_y - margin],
                [high_x + margin, low_y - margin],
                [high_x + margin, high_y + margin],
                [low_x - margin, high_y + margin],
            ];
            core::Shape::Polygon {
                points: corners
                    .iter()
                    .map(|corner| {
                        let board = rotate_vector(*corner, -at[2]);
                        [at[0] + board[0], at[1] + board[1]]
                    })
                    .collect(),
            }
        })
        .collect())
}

/// The box KiCad's stroke font fills with a text, from its anchor, size,
/// thickness and justification.
fn stroke_text_box(item: &Expr) -> Result<core::Shape, String> {
    stroke_text_box_of(
        item,
        item.children().get(1).and_then(Expr::atom).unwrap_or(""),
    )
}

/// A footprint's text (`fp_text`, a KiCad 8 `property`) on a copper or a
/// mask layer, as KiCad draws it (`drawn_text_box`), in the footprint's
/// frame: its angle is relative to the footprint's (KiCad 7 and 8 alike: a
/// footprint at 90 degrees with its text at 90 draws it at 180), so
/// `place_shape` puts it on the board. `None` for a hidden text, which
/// KiCad's DRC skips. A knockout text is its plate, the box grown by
/// KiCad's margin (`knockout_margin`).
pub(super) fn footprint_text_box(item: &Expr) -> Result<Option<core::Shape>, String> {
    if text_hidden(item) {
        return Ok(None);
    }
    let text = item.children().get(2).and_then(Expr::atom).unwrap_or("");
    if text.is_empty() {
        return Ok(None);
    }
    let knockout = item.child("layer").is_some_and(|layer| {
        layer
            .children()
            .iter()
            .skip(2)
            .any(|child| child.atom() == Some("knockout"))
    });
    let face = item
        .child("effects")
        .and_then(|effects| effects.child("font"))
        .is_some_and(|font| font.child("face").is_some());
    if !knockout && !face {
        return Ok(Some(plain_text_box(item, text)?));
    }
    // As a board's text (`drawn_graphic_shapes`): the box of before where
    // it holds the drawn one.
    let stroke = stroke_text_box_of(item, text)?;
    let (before, drawn, extent) = if knockout {
        let bounds = stroke.aabb().inflated(text_thickness(item));
        let before = core::Shape::Polygon {
            points: vec![
                bounds.minimum,
                [bounds.maximum[0], bounds.minimum[1]],
                bounds.maximum,
                [bounds.minimum[0], bounds.maximum[1]],
            ],
        };
        let margin = knockout_margin(item);
        (
            before,
            drawn_text_box(item, text, margin)?,
            drawn_text_extent(item, text, margin)?,
        )
    } else {
        (
            stroke,
            drawn_text_box(item, text, 0.0)?,
            drawn_text_extent(item, text, 0.0)?,
        )
    };
    Ok(Some(if covers(&before, &[extent]) {
        before
    } else {
        drawn
    }))
}

/// The box of the stroke font's glyphs as measured on KiCad's plots in
/// 2026-10 (symmetric about the anchor's line; `drawn_text_box` is KiCad's
/// layout, measured on pcbnew's polygons since 2026-11-08). The labels and
/// the placer's marks keep it (`copper_graphic_shapes`): a change of their
/// boxes moves layouts and was not measured.
fn stroke_text_box_of(item: &Expr, text: &str) -> Result<core::Shape, String> {
    stroke_box(item, text, false)
}

/// A plain (stroke font) text as the router's copper and mask see it: the
/// stroke box, with every glyph KiCad draws below the baseline counted as
/// a descender (`/`, `\`, `#`, `$`, `@`, `_` and the brackets as well:
/// protoesp's "CH340/1" on F.Mask reached 0.165 mm below the box, and a
/// bracket 0.17 heights). Measured on pcbnew's polygons of every plain
/// copper and mask text of the designer, expansion and short tiers, the box
/// holds each within 0.08 mm (the top of a `4`, a `^`; within its 8 % on
/// top elsewhere); KiCad's own layout (`drawn_text_box`) is not used for
/// them: a change of a plain text's box moved routes on the boards that
/// carry one (DSP-ADAU1452 lost its pass to a 1-open route elsewhere,
/// Sisu 1 -> 4), where none had a finding at a text.
fn plain_text_box(item: &Expr, text: &str) -> Result<core::Shape, String> {
    stroke_box(item, text, true)
}

fn stroke_box(item: &Expr, text: &str, all_descenders: bool) -> Result<core::Shape, String> {
    let at = form_at(item)?;
    let font = item
        .child("effects")
        .and_then(|effects| effects.child("font"));
    let size = font
        .and_then(|font| form_xy(font, "size").ok())
        .unwrap_or([1.0, 1.0]);
    let thickness = font
        .and_then(|font| form_f64(font, "thickness", 1).ok())
        .unwrap_or(0.15);
    // KiCad's stroke font, measured on its own plots: glyph
    // advances in units of the font width (`i` 0.48 ... `m` 1.33);
    // capitals and digits are one font height tall, descenders
    // reach 0.3 below, and lines are 1.6 heights apart. The sum
    // gets 8 % on top so the box stays outside every measured text.
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
    let lines = text.split("\\n").count().max(1) as f64;
    let widest = text
        .split("\\n")
        .map(|line| 1.08 * line.chars().map(advance).sum::<f64>())
        .fold(0.0, f64::max);
    // Symbols outside ASCII reach as far (eurorack-pmod's "⇒⊕⊥∃⊢≢" on
    // F.Mask: 0.73 mm across the line of a 1 mm text).
    // (A line break is the two characters `\n` here, not a backslash; a
    // text variable, `${REVISION}`, is drawn as its value: its own
    // characters say nothing.)
    let drawn: String = without_variables(text).replace("\\n", "");
    let descends = text
        .chars()
        .any(|c| matches!(c, 'g' | 'j' | 'p' | 'q' | 'y' | ',' | ';') || !c.is_ascii())
        || (all_descenders
            && drawn.chars().any(|c| {
                matches!(
                    c,
                    '/' | '\\' | '#' | '$' | '@' | '_' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
                )
            }));
    let half = [
        (widest * size[1] + thickness) / 2.0,
        (lines - 1.0) * 1.62 * size[0] / 2.0
            + (if descends { 0.8 } else { 0.5 }) * size[0]
            + thickness / 2.0,
    ];
    let justify: Vec<&str> = item
        .child("effects")
        .and_then(|effects| effects.child("justify"))
        .map(|justify| {
            justify
                .children()
                .iter()
                .skip(1)
                .filter_map(Expr::atom)
                .collect()
        })
        .unwrap_or_default();
    let shift = if justify.contains(&"left") {
        half[0]
    } else if justify.contains(&"right") {
        -half[0]
    } else {
        0.0
    };
    let shift = if justify.contains(&"mirror") {
        -shift
    } else {
        shift
    };
    // Top and bottom put the anchor on that side of the box.
    // A bottom anchor is the baseline: descenders hang below it
    // (eurorack-pmod's "[apf.audio]" on F.Mask, 1 mm high: its "p" reached
    // 0.25 mm past the anchor, where a track bridged).
    let rise = if justify.contains(&"top") {
        half[1]
    } else if justify.contains(&"bottom") {
        -half[1] + if descends { 0.3 * size[0] } else { 0.0 }
    } else {
        0.0
    };
    let offset = rotate_vector([shift, rise], -at[2]);
    // KiCad's glyphs reach a little past this model across the
    // line (SNSP's bold "SNSP" on B.Cu 0.17 mm past the box of a
    // 2.5 mm text, jetson-nano's title 0.03 mm): some room on both
    // sides.
    let half = [half[0], half[1] + 0.08 * size[0]];
    Ok(core::Shape::rectangle(
        [at[0] + offset[0], at[1] + offset[1]],
        half,
        -at[2],
    ))
}

/// The box KiCad 10.0.7 fills with a text's glyphs, grown by `grow` on
/// every side in the text's frame (a knockout text's plate:
/// `knockout_margin`), from its anchor, size, thickness, face, lines and
/// justification.
fn drawn_text_box(item: &Expr, text: &str, grow: f64) -> Result<core::Shape, String> {
    let at = form_at(item)?;
    let font = item
        .child("effects")
        .and_then(|effects| effects.child("font"));
    let size = font
        .and_then(|font| form_xy(font, "size").ok())
        .unwrap_or([1.0, 1.0]);
    let thickness = font
        .and_then(|font| form_f64(font, "thickness", 1).ok())
        .unwrap_or(0.15);
    // A face (TrueType) is drawn from the font installed, else from a
    // stand-in (fontconfig's: Noto Sans here), with its own line layout.
    let true_type = font.is_some_and(|font| font.child("face").is_some());
    let italic = font.is_some_and(|font| {
        font.children().iter().any(|child| {
            child.atom() == Some("italic")
                || (child.head() == Some("italic")
                    && !matches!(
                        child.children().get(1).and_then(Expr::atom),
                        Some("no" | "false")
                    ))
        })
    });
    let height = size[0];
    // KiCad's stroke font, measured on its own plots: glyph
    // advances in units of the font width (`i` 0.48 ... `m` 1.33),
    // a symbol outside ASCII 1.2 (eurorack-pmod's "⇒⊕⊥∃⊢≢"). The sum
    // gets 8 % on top so the box stays outside every measured text.
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
        character if !character.is_ascii() => 1.2,
        _ => 1.0,
    };
    let lines = text.split("\\n").count().max(1) as f64;
    let widest = text
        .split("\\n")
        .map(|line| 1.08 * line.chars().map(advance).sum::<f64>())
        .fold(0.0, f64::max);
    // Across the line, from pcbnew's polygons of 3150 texts made for it
    // (both fonts, five sizes, every justification, one to three lines)
    // and every visible text on a copper or mask layer of the designer,
    // expansion and short tiers. The lines are a block from the first
    // line's capital tops to the last line's baseline, one height per line
    // and the pitch between them (stroke 1.61 heights, a face 1.68); a top
    // anchor is the block's top (stroke 0.045 heights below it), a bottom
    // anchor its baseline (0.126 heights above it, a face 0.17), a centred
    // one its middle (0.0425 heights below, a face 0.085). Glyphs reach
    // past the block: above the capitals the stroke font's brackets and
    // `$` 0.143 heights, a face's `b`, `l` and `f` 0.073, accented
    // capitals 0.381 and 0.322; below the baseline the descenders (`g`,
    // brackets, `/`, `#`, `$`, `@`, `J`, `Q`, a face's `,`) 0.381 and 0.337,
    // the stroke font's `Q` and `_` 0.095 otherwise. The stroke font's pen
    // adds half the thickness all round; a face's outline has no pen.
    let ascii = text.is_ascii();
    let descends = !ascii
        || text.chars().any(|character| {
            matches!(
                character,
                'g' | 'j'
                    | 'p'
                    | 'q'
                    | 'y'
                    | ','
                    | ';'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '|'
                    | '/'
                    | '\\'
                    | '#'
                    | '$'
                    | '@'
                    | '_'
                    | 'J'
                    | 'Q'
            )
        });
    let (pitch, below_top, above_bottom, below_centre) = if true_type {
        (1.68, 0.0, 0.17, 0.085)
    } else {
        (1.61, 0.045, 0.126, 0.0425)
    };
    let (above, below, pen) = if true_type {
        (
            if ascii { 0.08 } else { 0.35 },
            if descends { 0.35 } else { 0.03 },
            0.0,
        )
    } else {
        (
            if ascii { 0.15 } else { 0.4 },
            if descends { 0.4 } else { 0.1 },
            thickness / 2.0,
        )
    };
    let justify: Vec<&str> = item
        .child("effects")
        .and_then(|effects| effects.child("justify"))
        .map(|justify| {
            justify
                .children()
                .iter()
                .skip(1)
                .filter_map(Expr::atom)
                .collect()
        })
        .unwrap_or_default();
    let block = height * (1.0 + (lines - 1.0) * pitch);
    let top = if justify.contains(&"top") {
        below_top * height
    } else if justify.contains(&"bottom") {
        -above_bottom * height - block
    } else {
        -block / 2.0 - below_centre * height
    };
    // Some room on every side (a face's widest glyphs reach 0.009 mm
    // inside it at 5 mm), more along the line for a slanted text.
    let margin = 0.02 * height;
    let low = top - above * height - pen - margin - grow;
    let high = top + block + below * height + pen + margin + grow;
    let along = (widest * size[1] + thickness) / 2.0;
    let shift = if justify.contains(&"left") {
        along
    } else if justify.contains(&"right") {
        -along
    } else {
        0.0
    };
    let shift = if justify.contains(&"mirror") {
        -shift
    } else {
        shift
    };
    let slant = if italic { 0.2 * height } else { 0.0 };
    let offset = rotate_vector([shift, (low + high) / 2.0], -at[2]);
    Ok(core::Shape::rectangle(
        [at[0] + offset[0], at[1] + offset[1]],
        [along + margin + slant + grow, (high - low) / 2.0],
        -at[2],
    ))
}

/// A dimension's copper as KiCad 10 draws it (PCB_DIM_ALIGNED and
/// PCB_DIM_ORTHOGONAL's updateGeometry): two extension lines from the
/// measured points (from the extension offset to the crossbar and the
/// extension height past it), the crossbar at the height, the arrows at
/// its ends (27.5 degrees each side, the arrow length long; pointing out,
/// or with `(arrow_direction inward)` in and on tails past the ends), and
/// the text's stroke-font box where the file put it. KiCad 10.0.7's DRC
/// measures from the lines' centres, not their drawn thickness (kicad-cli
/// on a board made for it: tracks 0.19, 0.15, 0.10, 0.064 and 0.05 mm from
/// the crossbar, an extension line's start, the second extension of an
/// orthogonal dimension and an arrow, as zero-width lines predict). KiCad cuts the crossbar under a text in line with it; the
/// whole bar is kept here. A leader, centre or radial mark: a line between
/// its points and its text. KiCad counts a dimension on a copper layer as
/// copper of no net ("Dimension '10' on F.Cu": shrimp-18's GameBit,
/// shorting_items and clearance).
pub(super) fn dimension_shapes(item: &Expr) -> Result<Vec<core::Shape>, String> {
    let points = outline::pts_points(item)?;
    let style = item.child("style");
    let style_f64 = |head: &str| style.and_then(|style| form_f64(style, head, 1).ok());
    let radius = 0.0;
    let mut shapes = Vec::new();
    let mut line = |start: [f64; 2], end: [f64; 2]| {
        shapes.push(core::Shape::Capsule { start, end, radius });
    };
    let unit = |vector: [f64; 2]| {
        let length = (vector[0] * vector[0] + vector[1] * vector[1]).sqrt();
        if length > 0.0 {
            [vector[0] / length, vector[1] / length]
        } else {
            [0.0, 0.0]
        }
    };
    let along = |point: [f64; 2], direction: [f64; 2], length: f64| {
        [
            point[0] + direction[0] * length,
            point[1] + direction[1] * length,
        ]
    };
    let kind = form_atom(item, "type", 1).unwrap_or("aligned");
    if let [start, end, ..] = points[..] {
        let height = form_f64(item, "height", 1).unwrap_or(0.0);
        let offset = style_f64("extension_offset").unwrap_or(0.0);
        let overshoot = style_f64("extension_height").unwrap_or(0.0);
        let crossbar = match kind {
            "aligned" | "orthogonal" => {
                let horizontal = form_atom(item, "orientation", 1) != Some("1");
                // The extension lines' directions: perpendicular to the
                // measurement towards the crossbar.
                let (first, crossbar) = if kind == "aligned" {
                    let normal = unit([start[1] - end[1], end[0] - start[0]]);
                    let shift = [normal[0] * height, normal[1] * height];
                    (
                        [shift, shift],
                        [
                            [start[0] + shift[0], start[1] + shift[1]],
                            [end[0] + shift[0], end[1] + shift[1]],
                        ],
                    )
                } else if horizontal {
                    let bar = start[1] + height;
                    (
                        [[0.0, height], [0.0, bar - end[1]]],
                        [[start[0], bar], [end[0], bar]],
                    )
                } else {
                    let bar = start[0] + height;
                    (
                        [[height, 0.0], [bar - end[0], 0.0]],
                        [[bar, start[1]], [bar, end[1]]],
                    )
                };
                for (point, extension) in [(start, first[0]), (end, first[1])] {
                    let direction = unit(extension);
                    let length = (extension[0] * extension[0] + extension[1] * extension[1]).sqrt();
                    let from = along(point, direction, offset);
                    line(from, along(from, direction, length - offset + overshoot));
                }
                Some(crossbar)
            }
            _ => {
                line(start, end);
                None
            }
        };
        if let Some([bar_start, bar_end]) = crossbar {
            line(bar_start, bar_end);
            let arrow = style_f64("arrow_length").unwrap_or(1.27);
            let inward = style
                .and_then(|style| form_atom(style, "arrow_direction", 1))
                .is_some_and(|direction| direction == "inward");
            let direction = unit([bar_end[0] - bar_start[0], bar_end[1] - bar_start[1]]);
            let (sin, cos) = 27.5_f64.to_radians().sin_cos();
            let turn = |vector: [f64; 2], sin: f64| {
                [
                    vector[0] * cos - vector[1] * sin,
                    vector[0] * sin + vector[1] * cos,
                ]
            };
            let sign = if inward { -1.0 } else { 1.0 };
            for (tip, toward) in [
                (bar_start, direction),
                (bar_end, [-direction[0], -direction[1]]),
            ] {
                let toward = [toward[0] * sign, toward[1] * sign];
                for side in [sin, -sin] {
                    line(tip, along(tip, turn(toward, side), arrow));
                }
                if inward {
                    line(tip, along(tip, toward, arrow));
                }
            }
        }
    }
    // KiCad 5 wrote the lines themselves.
    for part in [
        "feature1", "feature2", "crossbar", "arrow1a", "arrow1b", "arrow2a", "arrow2b",
    ] {
        if let Some(form) = item.child(part)
            && let [start, end, ..] = outline::pts_points(form)?[..]
        {
            line(start, end);
        }
    }
    if let Some(text) = item.child("gr_text") {
        let value = text.children().get(1).and_then(Expr::atom).unwrap_or("");
        if !value.is_empty() {
            shapes.push(plain_text_box(text, value)?);
        }
    }
    Ok(shapes)
}

/// Whether a text or a text box is hidden: `hide`, `(hide yes)`, on the
/// item or in its effects.
pub(super) fn text_hidden(item: &Expr) -> bool {
    let hidden = |node: &Expr| {
        node.children().iter().any(|child| {
            child.atom() == Some("hide")
                || (child.head() == Some("hide")
                    && !matches!(
                        child.children().get(1).and_then(Expr::atom),
                        Some("no" | "false")
                    ))
        })
    };
    hidden(item) || item.child("effects").is_some_and(hidden)
}

/// A text box (`gr_text_box`, a footprint's `fp_text_box`) on copper: its
/// box, which holds its glyphs, filled, grown by half its stroke's width.
/// KiCad 10.0.7's DRC measures from the outline grown so with or without
/// `(border yes)` (kicad-cli: a track 0.05 mm past the stroke of either)
/// and from the glyphs inside; the filled box holds both. A footprint's
/// comes out in the footprint's frame (its corners are the footprint's
/// coordinates, as a line's are).
pub fn text_box_shapes(item: &Expr) -> Result<Vec<core::Shape>, String> {
    // A hidden text box (an `fp_text_box` as a footprint's hidden text)
    // draws nothing. Its corners are the box's: KiCad reads a text box's
    // `angle` as its text's only (the parser does not rotate the box), so
    // a box of `start` and `end` with an angle keeps those corners.
    if text_hidden(item) {
        return Ok(Vec::new());
    }
    let corners = if item.child("pts").is_some() {
        outline::pts_points(item)?
    } else {
        let (a, b) = (form_xy(item, "start")?, form_xy(item, "end")?);
        vec![a, [b[0], a[1]], b, [a[0], b[1]]]
    };
    if corners.len() < 3 {
        return Ok(Vec::new());
    }
    let width = item
        .child("stroke")
        .and_then(|stroke| form_f64(stroke, "width", 1).ok())
        .unwrap_or(0.0);
    let mut shapes = Vec::new();
    if width > 0.0 {
        shapes.extend((0..corners.len()).map(|index| core::Shape::Capsule {
            start: corners[index],
            end: corners[(index + 1) % corners.len()],
            radius: width / 2.0,
        }));
    }
    shapes.push(core::Shape::Polygon { points: corners });
    Ok(shapes)
}

/// Whether the board tents its vias on the front and on the back, as KiCad
/// 10 reads the setup: `(tenting (front yes) (back yes))` (a side left out
/// is not tented), KiCad 9's `(tenting front back)` or `(tenting none)`, and
/// before either the plot setting `(viasonmask yes)` for untented vias. A
/// board that says nothing tents them (KiCad's default).
pub(crate) fn vias_tented(pcb: &Expr) -> [bool; 2] {
    let Some(setup) = pcb.child("setup") else {
        return [true, true];
    };
    if let Some(tenting) = setup.child("tenting") {
        let rest = tenting.children().get(1..).unwrap_or_default();
        if rest.iter().any(|child| child.atom().is_some()) {
            let named = |side: &str| rest.iter().any(|child| child.atom() == Some(side));
            return [named("front"), named("back")];
        }
        let yes = |side: &str| matches!(form_atom(tenting, side, 1), Some("yes" | "true"));
        return [yes("front"), yes("back")];
    }
    match setup
        .child("pcbplotparams")
        .and_then(|plot| form_atom(plot, "viasonmask", 1))
    {
        Some("yes" | "true") => [false, false],
        _ => [true, true],
    }
}

/// A rule area that keeps zone fills out: its layer mask and polygon.
pub(super) type PourKeepout = (u32, Vec<[f64; 2]>);

/// A rule area that keeps zone fills out (`(copperpour not_allowed)`): its
/// layers and polygon. KiCad's fill does not enter it, so neither does a
/// pour's on those layers (Sisu: a 30 x 18 mm area on In3 and three on
/// In4 and B.Cu, which the pour model counted as fill).
pub(super) fn pour_keepout(
    zone: &Expr,
    layers: &LayerTable,
) -> Result<Option<PourKeepout>, String> {
    let keeps_fill_out = zone
        .child("keepout")
        .and_then(|keepout| keepout.child("copperpour"))
        .and_then(|form| form.children().get(1))
        .and_then(Expr::atom)
        == Some("not_allowed");
    if !keeps_fill_out {
        return Ok(None);
    }
    // Only copper layers matter, and an outline that is not a polygon is
    // skipped unless it is a drawn circle (TelemetryOnboard: circles on
    // F.SilkS, outlines of one arc each, refused the whole board).
    let mask = layers.mask_of_item(zone)?;
    if mask == 0 {
        return Ok(None);
    }
    if let Ok(points) = rule_area_polygon_points(zone) {
        return Ok(Some((mask, points)));
    }
    Ok(rule_area_circle(zone).map(|points| (mask, points)))
}

/// A rule area outlined by one full-circle arc (start and end equal, the
/// middle across), as a 32-gon around it; `None` for any other outline.
fn rule_area_circle(zone: &Expr) -> Option<Vec<[f64; 2]>> {
    let pts = zone.child("polygon")?.child("pts")?;
    let arcs: Vec<&Expr> = pts
        .children()
        .iter()
        .filter(|item| item.head() == Some("arc"))
        .collect();
    let [arc] = arcs.as_slice() else {
        return None;
    };
    let point = |name: &str| -> Option<[f64; 2]> {
        let form = arc.child(name)?;
        Some([
            finite_number(form.children().get(1)?.atom()?, "an arc's x").ok()?,
            finite_number(form.children().get(2)?.atom()?, "an arc's y").ok()?,
        ])
    };
    let (start, middle, end) = (point("start")?, point("mid")?, point("end")?);
    if (start[0] - end[0]).abs() > 1.0e-6 || (start[1] - end[1]).abs() > 1.0e-6 {
        return None;
    }
    let center = [(start[0] + middle[0]) / 2.0, (start[1] + middle[1]) / 2.0];
    let radius = ((start[0] - middle[0]).powi(2) + (start[1] - middle[1]).powi(2)).sqrt() / 2.0;
    (radius > 0.0).then(|| {
        (0..32)
            .map(|index| {
                let angle = index as f64 * std::f64::consts::TAU / 32.0;
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]
            })
            .collect()
    })
}

/// Whether a rule area lets pours in (`(copperpour allowed)`, KiCad's
/// default).
pub(super) fn rule_area_allows_pours(zone: &Expr) -> bool {
    zone.child("keepout")
        .and_then(|keepout| keepout.child("copperpour"))
        .and_then(|form| form.children().get(1))
        .and_then(Expr::atom)
        != Some("not_allowed")
}

/// A region where a pour stays whole: its pour net, copper layers, polygon
/// and the rule area's name.
pub(super) type KeepWholeRegion = (String, core::LayerMask, Vec<[f64; 2]>, String);

/// A rule area that marks where a pour stays whole (`keep whole <net>
/// <layer>`, written for `add_pours`' `keep_whole`): its pour net, copper
/// layers, polygon and name; `None` for any other rule area.
pub(super) fn keep_whole_region(
    zone: &Expr,
    layers: &LayerTable,
) -> Result<Option<KeepWholeRegion>, String> {
    let Some(name) = form_atom(zone, "name", 1) else {
        return Ok(None);
    };
    let Some(rest) = name
        .strip_prefix(KEEP_WHOLE_NAME)
        .and_then(|rest| rest.strip_prefix(' '))
    else {
        return Ok(None);
    };
    // The net is everything before the layer (the last word).
    let Some((net, _)) = rest.rsplit_once(' ') else {
        return Ok(None);
    };
    let mask = layers.mask_of_item(zone)?;
    if mask == 0 {
        return Ok(None);
    }
    Ok(Some((
        normalize_net(net).to_string(),
        mask,
        rule_area_polygon_points(zone)?,
        name.to_string(),
    )))
}

/// A rule area that keeps tracks or vias out, as an obstacle. Zones inside
/// footprints are stored in board coordinates too.
pub(super) fn rule_area_obstacle(
    zone: &Expr,
    layers: &LayerTable,
    fallback_label: &str,
) -> Result<Option<core::Obstacle>, String> {
    let allowed = |kind: &str| {
        zone.child("keepout")
            .and_then(|keepout| keepout.child(kind))
            .and_then(|form| form.children().get(1))
            .and_then(Expr::atom)
            != Some("not_allowed")
    };
    let (blocks_tracks, blocks_vias) = (!allowed("tracks"), !allowed("vias"));
    if !(blocks_tracks || blocks_vias) {
        return Ok(None);
    }
    Ok(Some(core::Obstacle {
        shape: core::Shape::Polygon {
            points: rule_area_polygon_points(zone)?,
        },
        layers: layers.mask_of_item(zone)?,
        kind: core::ObstacleKind::Keepout,
        net: None,
        clearance: 0.0,
        clearance_override: None,
        plated: false,
        blocks_tracks,
        blocks_vias,
        label: form_atom(zone, "name", 1)
            .unwrap_or(fallback_label)
            .to_string(),
    }))
}

fn rule_area_like_points(item: &Expr) -> Result<Vec<[f64; 2]>, String> {
    let points = outline::pts_points(item)?;
    if points.len() < 3 {
        return Err("copper polygon has fewer than three points".into());
    }
    Ok(points)
}

/// A zone is a pour when it covers at least this share of the board.
pub(super) const POUR_SHARE: f64 = 0.1;

pub(super) fn polygon_area(points: &[[f64; 2]]) -> f64 {
    (0..points.len())
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

pub struct Pour {
    pub net: String,
    pub(crate) layers: core::LayerMask,
    pub(super) priority: i64,
    /// The zone's uuid: KiCad orders zones of equal priority by it
    /// (`ZONE::HigherPriority`).
    pub(super) uuid: String,
    pub(super) clearance: f64,
    pub(super) min_thickness: f64,
    pub(super) thermal_reach: f64,
    pub(super) thermal_gap: f64,
    /// `(connect_pads yes ...)`: pads join wherever the fill touches.
    pub(super) solid: bool,
    /// `(fill (mode hatch) ...)`.
    pub(crate) hatch: Option<core::Hatch>,
    /// `(fill (island_removal_mode 0|1|2) (island_area_min A))`
    /// (`core::Plane::island_min_area`).
    pub(super) island_min_area: Option<f64>,
    pub(crate) polygon: Vec<[f64; 2]>,
}

pub fn pours(pcb: &Expr, layers: &LayerTable) -> Result<Vec<Pour>, String> {
    let names = NetNames::of(pcb);
    let mut result = Vec::new();
    for zone in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone") && !is_rule_area(item))
    {
        let Some(net) = form_atom(zone, "net_name", 1)
            .or_else(|| node_net(zone))
            .filter(|raw| names.routable(raw))
            .map(|raw| names.normalize(raw))
        else {
            continue;
        };
        if form_atom(zone, "fill", 1) == Some("no") {
            continue;
        }
        let mask = layers.mask_of_item(zone)?;
        if mask == 0 {
            continue;
        }
        result.push(Pour {
            net: net.to_string(),
            layers: mask,
            priority: form_atom(zone, "priority", 1)
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            uuid: form_atom(zone, "uuid", 1)
                .or_else(|| form_atom(zone, "tstamp", 1))
                .unwrap_or("")
                .to_ascii_lowercase(),
            clearance: zone
                .child("connect_pads")
                .and_then(|form| form_f64(form, "clearance", 1).ok())
                .unwrap_or(0.0),
            min_thickness: form_f64(zone, "min_thickness", 1).unwrap_or(0.25),
            solid: zone
                .child("connect_pads")
                .and_then(|form| form.children().get(1))
                .and_then(Expr::atom)
                == Some("yes"),
            thermal_gap: zone
                .child("fill")
                .map(|fill| form_f64(fill, "thermal_gap", 1).unwrap_or(0.5))
                .unwrap_or(0.5),
            thermal_reach: zone
                .child("fill")
                .map(|fill| {
                    form_f64(fill, "thermal_gap", 1).unwrap_or(0.5)
                        + form_f64(fill, "thermal_bridge_width", 1).unwrap_or(0.5)
                })
                .unwrap_or(1.0),
            hatch: zone
                .child("fill")
                .filter(|fill| form_atom(fill, "mode", 1) == Some("hatch"))
                .map(|fill| core::Hatch {
                    thickness: form_f64(fill, "hatch_thickness", 1).unwrap_or(1.0),
                    gap: form_f64(fill, "hatch_gap", 1).unwrap_or(1.5),
                    orientation: form_f64(fill, "hatch_orientation", 1).unwrap_or(0.0),
                    smoothing_level: form_f64(fill, "hatch_smoothing_level", 1)
                        .unwrap_or(0.0)
                        .max(0.0) as u32,
                    smoothing_value: form_f64(fill, "hatch_smoothing_value", 1).unwrap_or(0.0),
                    min_hole_area: form_f64(fill, "hatch_min_hole_area", 1).unwrap_or(0.3),
                    // `(connect_pads (clearance ...))` is thermal; `yes`,
                    // `thru_hole_only` (a via is no through-hole pad) and
                    // `no` are not.
                    via_reliefs: zone
                        .child("connect_pads")
                        .and_then(|form| form.children().get(1))
                        .and_then(Expr::atom)
                        .is_none(),
                }),
            island_min_area: match zone
                .child("fill")
                .and_then(|fill| form_atom(fill, "island_removal_mode", 1))
            {
                Some("1") => None,
                Some("2") => Some(
                    zone.child("fill")
                        .and_then(|fill| form_f64(fill, "island_area_min", 1).ok())
                        .unwrap_or(10.0),
                ),
                _ => Some(0.0),
            },
            polygon: rule_area_polygon_points(zone)?,
        });
    }
    Ok(result)
}

pub(super) fn nanometres(point: [f64; 2]) -> [f64; 2] {
    point.map(|value| (value * 1.0e6).round() / 1.0e6)
}

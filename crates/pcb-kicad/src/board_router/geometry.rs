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
pub(crate) fn copper_graphic_shapes(item: &Expr) -> Result<Vec<core::Shape>, String> {
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
        Some("gr_arc" | "fp_arc") => outline::arc_points(
            form_xy(item, "start")?,
            form_xy(item, "mid")?,
            form_xy(item, "end")?,
        )
        .windows(2)
        .map(|pair| capsule(pair[0], pair[1]))
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

/// The shapes of a board graphic on a mask layer as KiCad's bridge test
/// sees them. KiCad draws a cached text's glyphs from the font, not from
/// the cache, and a face it does not have is replaced (OpenESC's "ESC" in
/// Tokyo on B.Mask, drawn in a stand-in 2 mm wider than its cache, over a
/// track and a via that kept clear of the cached glyphs; the nine cached
/// mask texts of the corpus reach 0.7 to 2.9 mm past their caches): the
/// stroke font's box, which holds all nine within 0.06 mm, is part of the
/// opening too.
pub(super) fn mask_graphic_shapes(item: &Expr) -> Result<Vec<core::Shape>, String> {
    let mut shapes = copper_graphic_shapes(item)?;
    let cached = item.head() == Some("gr_text")
        && item.child("render_cache").is_some_and(|cache| {
            cache
                .children()
                .iter()
                .any(|child| child.head() == Some("polygon"))
        });
    if cached {
        shapes.push(stroke_text_box(item)?);
    }
    Ok(shapes)
}

/// The box KiCad's stroke font fills with a text, from its anchor, size,
/// thickness and justification.
fn stroke_text_box(item: &Expr) -> Result<core::Shape, String> {
    let at = form_at(item)?;
    let text = item.children().get(1).and_then(Expr::atom).unwrap_or("");
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
    let descends = text
        .chars()
        .any(|c| matches!(c, 'g' | 'j' | 'p' | 'q' | 'y' | ',' | ';') || !c.is_ascii());
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
            form.children().get(1)?.atom()?.parse().ok()?,
            form.children().get(2)?.atom()?.parse().ok()?,
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

pub(crate) struct Pour {
    pub(crate) net: String,
    pub(crate) layers: core::LayerMask,
    pub(super) priority: i64,
    pub(super) clearance: f64,
    pub(super) min_thickness: f64,
    pub(super) thermal_reach: f64,
    pub(super) thermal_gap: f64,
    /// `(connect_pads yes ...)`: pads join wherever the fill touches.
    pub(super) solid: bool,
    /// `(fill (mode hatch) ...)`.
    pub(crate) hatch: Option<core::Hatch>,
    pub(crate) polygon: Vec<[f64; 2]>,
}

pub(crate) fn pours(pcb: &Expr, layers: &LayerTable) -> Result<Vec<Pour>, String> {
    let mut result = Vec::new();
    for zone in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone") && !is_rule_area(item))
    {
        let Some(net) = form_atom(zone, "net_name", 1)
            .or_else(|| node_net(zone))
            .filter(|raw| routable_net(raw))
            .map(normalize_net)
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
                }),
            polygon: rule_area_polygon_points(zone)?,
        });
    }
    Ok(result)
}

pub(super) fn nanometres(point: [f64; 2]) -> [f64; 2] {
    point.map(|value| (value * 1.0e6).round() / 1.0e6)
}

// Copyright (C) 2026 Toit contributors.

//! Footprint fields on the silkscreen (the visible Reference and Value)
//! placed where KiCad's silkscreen checks find nothing: off mask-opened
//! copper (`silk_over_copper`), off other silkscreen (`silk_overlap`) and
//! the board edge (`silk_edge_clearance`), each at the project's
//! `min_silk_clearance`.
//!
//! The scorer predicts those findings from our own geometry: a field's
//! ink is KiCad's stroke font measured with pcbnew (`GLYPHS`), its
//! obstacles the pads' mask openings, every other silkscreen item (the
//! footprints' graphics and texts, the other fields, the board's texts and
//! graphics) and the outline. The polish after routing (`labels`) and
//! `repair-kicad-silkscreen` share it.

use super::*;
use std::collections::HashMap;

/// An axis-aligned box: [min x, min y, max x, max y].
pub(crate) type Box2 = [f64; 4];

/// KiCad's stroke font (KiCad 10.0.7, measured with pcbnew at size 1 and
/// pen 0.15, then the pen taken out), per printable ASCII character from
/// the space on: [advance, ink left, ink right, ink top, ink bottom], the
/// ink relative to the character's centre, y down from the text's anchor.
const GLYPHS: [[f64; 5]; 95] = [
    [0.7619, 0.0, 0.0, 0.0, 0.0],
    [0.4762, -0.0476, 0.0476, -0.5374, 0.4626],
    [0.7619, -0.1905, 0.1905, -0.5374, -0.3469],
    [1.0, -0.4048, 0.4048, -0.6326, 0.6531],
    [0.9524, -0.2857, 0.2857, -0.6802, 0.6055],
    [1.1429, -0.381, 0.381, -0.5374, 0.4626],
    [1.2381, -0.381, 0.4286, -0.5374, 0.4626],
    [0.4762, -0.0476, 0.0476, -0.5374, -0.3469],
    [0.6667, -0.0952, 0.1905, -0.6802, 0.8436],
    [0.6667, -0.1905, 0.0952, -0.6802, 0.8436],
    [0.7619, -0.2381, 0.2381, -0.5374, -0.1088],
    [1.2381, -0.381, 0.381, -0.2993, 0.4626],
    [0.4762, -0.0476, 0.0476, 0.415, 0.6055],
    [1.2381, -0.381, 0.381, 0.0817, 0.0817],
    [0.4762, -0.0476, 0.0476, 0.3674, 0.4626],
    [1.0476, -0.4286, 0.4286, -0.585, 0.7007],
    [0.9524, -0.2857, 0.2857, -0.5374, 0.4626],
    [0.9524, -0.2857, 0.2857, -0.5374, 0.4626],
    [0.9524, -0.3333, 0.2857, -0.5374, 0.4626],
    [0.9524, -0.3333, 0.2857, -0.5374, 0.4626],
    [0.9524, -0.2857, 0.3333, -0.585, 0.4626],
    [0.9524, -0.2857, 0.2857, -0.5374, 0.4626],
    [0.9524, -0.2857, 0.2857, -0.5374, 0.4626],
    [0.9524, -0.3333, 0.3333, -0.5374, 0.4626],
    [0.9524, -0.2857, 0.2857, -0.5374, 0.4626],
    [0.9524, -0.2857, 0.2857, -0.5374, 0.4626],
    [0.4762, -0.0476, 0.0476, -0.1564, 0.4626],
    [0.4762, -0.0476, 0.0476, -0.1564, 0.6055],
    [1.2381, -0.381, 0.381, -0.204, 0.3674],
    [1.2381, -0.381, 0.381, -0.0612, 0.2245],
    [1.2381, -0.381, 0.381, -0.204, 0.3674],
    [0.8571, -0.2381, 0.2381, -0.5374, 0.4626],
    [1.2857, -0.4524, 0.4524, -0.3469, 0.6055],
    [0.8571, -0.3333, 0.3333, -0.5374, 0.4626],
    [1.0, -0.2619, 0.3095, -0.5374, 0.4626],
    [1.0, -0.3095, 0.3095, -0.5374, 0.4626],
    [1.0, -0.2619, 0.3095, -0.5374, 0.4626],
    [0.9048, -0.2143, 0.2619, -0.5374, 0.4626],
    [0.8571, -0.1905, 0.2857, -0.5374, 0.4626],
    [1.0, -0.3095, 0.3095, -0.5374, 0.4626],
    [1.0476, -0.2857, 0.2857, -0.5374, 0.4626],
    [0.4762, 0.0, 0.0, -0.5374, 0.4626],
    [0.7619, -0.2381, 0.1429, -0.5374, 0.4626],
    [1.0, -0.2619, 0.3095, -0.5374, 0.4626],
    [0.8095, -0.1667, 0.3095, -0.5374, 0.4626],
    [1.1429, -0.3333, 0.3333, -0.5374, 0.4626],
    [1.0476, -0.2857, 0.2857, -0.5374, 0.4626],
    [1.0476, -0.3333, 0.3333, -0.5374, 0.4626],
    [1.0, -0.2619, 0.3095, -0.5374, 0.4626],
    [1.0476, -0.3333, 0.381, -0.5374, 0.5579],
    [1.0, -0.2619, 0.3095, -0.5374, 0.4626],
    [0.9524, -0.2857, 0.2857, -0.5374, 0.4626],
    [0.7619, -0.2857, 0.2857, -0.5374, 0.4626],
    [1.0476, -0.2857, 0.2857, -0.5374, 0.4626],
    [0.8571, -0.3333, 0.3333, -0.5374, 0.4626],
    [1.1429, -0.4286, 0.4286, -0.5374, 0.4626],
    [0.9524, -0.3333, 0.3333, -0.5374, 0.4626],
    [0.8571, -0.3333, 0.3333, -0.5374, 0.4626],
    [0.9524, -0.3333, 0.3333, -0.5374, 0.4626],
    [0.6667, -0.0476, 0.1905, -0.6326, 0.796],
    [0.6667, -0.4286, 0.4286, -0.6326, 0.6531],
    [0.6667, -0.1905, 0.0476, -0.6326, 0.796],
    [0.5714, -0.1905, 0.1905, -0.585, -0.4421],
    [0.7619, -0.381, 0.381, 0.5579, 0.5579],
    [0.381, -0.0952, 0.0476, -0.585, -0.4421],
    [0.9048, -0.2619, 0.2143, -0.204, 0.4626],
    [0.9048, -0.2143, 0.2619, -0.5374, 0.4626],
    [0.8571, -0.2381, 0.2381, -0.204, 0.4626],
    [0.9048, -0.2619, 0.2143, -0.5374, 0.4626],
    [0.8571, -0.2381, 0.2381, -0.204, 0.4626],
    [0.5714, -0.1905, 0.1905, -0.5374, 0.4626],
    [0.9048, -0.2619, 0.2143, -0.204, 0.796],
    [0.9048, -0.2143, 0.2143, -0.5374, 0.4626],
    [0.4762, -0.0476, 0.0476, -0.5374, 0.4626],
    [0.4762, -0.1905, 0.0476, -0.5374, 0.796],
    [0.8095, -0.1667, 0.2143, -0.5374, 0.4626],
    [0.5238, -0.0238, 0.119, -0.5374, 0.4626],
    [1.3333, -0.4286, 0.4286, -0.204, 0.4626],
    [0.9048, -0.2143, 0.2143, -0.204, 0.4626],
    [0.9048, -0.2619, 0.2619, -0.204, 0.4626],
    [0.9048, -0.2143, 0.2619, -0.204, 0.796],
    [0.9048, -0.2619, 0.2143, -0.204, 0.796],
    [0.619, -0.0714, 0.2143, -0.204, 0.4626],
    [0.8095, -0.2143, 0.2143, -0.204, 0.4626],
    [0.5714, -0.1905, 0.1905, -0.5374, 0.4626],
    [0.9048, -0.2143, 0.2143, -0.204, 0.4626],
    [0.7619, -0.2381, 0.2381, -0.204, 0.4626],
    [1.0476, -0.381, 0.381, -0.204, 0.4626],
    [0.8095, -0.2619, 0.2619, -0.204, 0.4626],
    [0.7619, -0.2381, 0.2381, -0.204, 0.796],
    [0.8095, -0.2619, 0.2619, -0.204, 0.4626],
    [0.6667, -0.1429, 0.1905, -0.6802, 0.8436],
    [0.9524, 0.0, 0.0, -0.6326, 0.796],
    [0.6667, -0.1905, 0.1429, -0.6802, 0.8436],
    [0.7143, -0.2619, 0.2143, -0.0136, 0.0817],
];

/// A character outside the table: as wide and tall as the widest.
const UNKNOWN_GLYPH: [f64; 5] = [1.24, -0.52, 0.52, -0.69, 0.86];

/// How far the pen widens the ink up and down (of the pen's width; the
/// font's cell is shifted up by the pen).
const PEN_UP: f64 = 0.552;
const PEN_DOWN: f64 = 0.448;

/// What the box estimate may be off by, and what a candidate keeps from
/// an obstacle beyond the clearance.
const MARGIN: f64 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Justify {
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextStyle {
    /// The file's font size: [height, width].
    pub size: [f64; 2],
    pub thickness: f64,
    pub horizontal: Justify,
    pub vertical: Justify,
    pub mirrored: bool,
}

impl TextStyle {
    /// From a text's `effects` (KiCad's defaults: size 1, pen 0.15,
    /// centred).
    pub(crate) fn of(item: &Expr) -> Self {
        let effects = item.child("effects");
        let font = effects.and_then(|effects| effects.child("font"));
        let justify: Vec<&str> = effects
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
        let size = font
            .and_then(|font| form_xy(font, "size").ok())
            .unwrap_or([1.0, 1.0]);
        Self {
            size,
            thickness: font
                .and_then(|font| form_f64(font, "thickness", 1).ok())
                .unwrap_or(0.15),
            horizontal: if justify.contains(&"left") {
                Justify::Start
            } else if justify.contains(&"right") {
                Justify::End
            } else {
                Justify::Center
            },
            vertical: if justify.contains(&"top") {
                Justify::Start
            } else if justify.contains(&"bottom") {
                Justify::End
            } else {
                Justify::Center
            },
            mirrored: justify.contains(&"mirror"),
        }
    }

    fn scaled(&self, scale: f64) -> Self {
        Self {
            size: [self.size[0] * scale, self.size[1] * scale],
            thickness: self.thickness * scale,
            ..self.clone()
        }
    }
}

/// The ink of a single-line stroke text in its own frame, relative to its
/// anchor: [along min, along max, across min, across max] (across is down
/// the glyphs). `None` for a text without ink.
pub(crate) fn text_ink(text: &str, style: &TextStyle) -> Option<[f64; 4]> {
    let [height, width] = style.size;
    let smaller = height.min(width).max(0.0);
    let pen = if style.thickness > 0.0 {
        style.thickness
    } else {
        smaller / 8.0
    }
    .min(smaller / 4.0);
    let glyph = |character: char| {
        let code = character as usize;
        if (32..127).contains(&code) {
            GLYPHS[code - 32]
        } else {
            UNKNOWN_GLYPH
        }
    };
    let total: f64 = text.chars().map(|c| glyph(c)[0] * width).sum();
    let mut cursor = -total / 2.0;
    let mut ink = [
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for character in text.chars() {
        let [advance, left, right, top, bottom] = glyph(character);
        let middle = cursor + advance * width / 2.0;
        cursor += advance * width;
        if character.is_whitespace() {
            continue;
        }
        ink = [
            ink[0].min(middle + left * width),
            ink[1].max(middle + right * width),
            ink[2].min(top * height),
            ink[3].max(bottom * height),
        ];
    }
    if !ink[0].is_finite() {
        return None;
    }
    let mut ink = [
        ink[0] - pen / 2.0,
        ink[1] + pen / 2.0,
        ink[2] - PEN_UP * pen,
        ink[3] + PEN_DOWN * pen,
    ];
    // KiCad's justified text starts (or ends) a little past the anchor.
    let along = (total / 2.0 + 0.658 * pen)
        * match style.horizontal {
            Justify::Start => 1.0,
            Justify::Center => 0.0,
            Justify::End => -1.0,
        };
    let across = (0.5 * height + 0.567 * pen)
        * match style.vertical {
            Justify::Start => 1.0,
            Justify::Center => 0.0,
            Justify::End => -1.0,
        };
    ink = [
        ink[0] + along,
        ink[1] + along,
        ink[2] + across,
        ink[3] + across,
    ];
    if style.mirrored {
        ink = [-ink[1], -ink[0], ink[2], ink[3]];
    }
    Some(ink)
}

/// The board box of a text's ink at `anchor` and absolute `angle` (KiCad
/// keeps texts upright: one at 90 to 270 degrees reads as turned half
/// way), widened by `MARGIN`.
pub(crate) fn text_box(
    text: &str,
    style: &TextStyle,
    anchor: [f64; 2],
    angle: f64,
) -> Option<Box2> {
    let ink = text_ink(text, style)?;
    let mut upright = angle.rem_euclid(360.0);
    if upright > 90.0 + 1e-9 && upright <= 270.0 + 1e-9 {
        upright -= 180.0;
    }
    let (sin, cos) = upright.to_radians().sin_cos();
    let along = [cos, -sin];
    let across = [sin, cos];
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for u in [ink[0], ink[1]] {
        for v in [ink[2], ink[3]] {
            let point = [
                anchor[0] + u * along[0] + v * across[0],
                anchor[1] + u * along[1] + v * across[1],
            ];
            bounds = [
                bounds[0].min(point[0]),
                bounds[1].min(point[1]),
                bounds[2].max(point[0]),
                bounds[3].max(point[1]),
            ];
        }
    }
    Some([
        bounds[0] - MARGIN,
        bounds[1] - MARGIN,
        bounds[2] + MARGIN,
        bounds[3] + MARGIN,
    ])
}

/// The project's silkscreen rules (its `.kicad_pro`, KiCad's defaults
/// where it has none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SilkRules {
    /// `min_silk_clearance`: silkscreen to silkscreen, to mask openings and
    /// to the board edge.
    pub clearance: f64,
    pub min_text_height: f64,
    pub min_text_thickness: f64,
}

impl Default for SilkRules {
    fn default() -> Self {
        Self {
            clearance: 0.0,
            min_text_height: 0.8,
            min_text_thickness: 0.08,
        }
    }
}

impl SilkRules {
    pub(crate) fn from_project(project: &Path) -> Self {
        let defaults = Self::default();
        let Some(file) = fs::read_to_string(project)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        else {
            return defaults;
        };
        let rules = &file["board"]["design_settings"]["rules"];
        let value = |key: &str, default: f64| rules[key].as_f64().unwrap_or(default).max(0.0);
        Self {
            clearance: value("min_silk_clearance", defaults.clearance),
            min_text_height: value("min_text_height", defaults.min_text_height),
            min_text_thickness: value("min_text_thickness", defaults.min_text_thickness),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A mask opening (a pad's, or a graphic on a mask layer): KiCad's
    /// `silk_over_copper`.
    Mask,
    /// Silkscreen: KiCad's `silk_overlap`.
    Silk,
    /// The outline: KiCad's `silk_edge_clearance`.
    Edge,
    /// A part's body (courtyard): no finding, but a label there reads as
    /// the other part's.
    Body,
}

#[derive(Clone, Debug)]
enum Shape {
    Rect(Box2),
    /// A segment with a radius (a stroke, a round or oval pad, a disc).
    Capsule([f64; 2], [f64; 2], f64),
    /// A filled polygon.
    Polygon(Vec<[f64; 2]>),
}

fn box_of(points: &[[f64; 2]]) -> Box2 {
    points.iter().fold(
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
}

fn grow(b: Box2, by: f64) -> Box2 {
    [b[0] - by, b[1] - by, b[2] + by, b[3] + by]
}

fn boxes_meet(a: Box2, b: Box2) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

fn point_box_distance(p: [f64; 2], b: Box2) -> f64 {
    let dx = (b[0] - p[0]).max(0.0).max(p[0] - b[2]);
    let dy = (b[1] - p[1]).max(0.0).max(p[1] - b[3]);
    dx.hypot(dy)
}

fn box_box_distance(a: Box2, b: Box2) -> f64 {
    let dx = (b[0] - a[2]).max(a[0] - b[2]).max(0.0);
    let dy = (b[1] - a[3]).max(a[1] - b[3]).max(0.0);
    dx.hypot(dy)
}

fn inside_box(p: [f64; 2], b: Box2) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}

fn segments_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let cross = |o: [f64; 2], p: [f64; 2], q: [f64; 2]| {
        (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0])
    };
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

/// Distance between a box and a segment (0 where they meet).
fn segment_box_distance(a: [f64; 2], b: [f64; 2], bounds: Box2) -> f64 {
    if inside_box(a, bounds) || inside_box(b, bounds) {
        return 0.0;
    }
    let corners = [
        [bounds[0], bounds[1]],
        [bounds[2], bounds[1]],
        [bounds[2], bounds[3]],
        [bounds[0], bounds[3]],
    ];
    for side in 0..4 {
        if segments_cross(a, b, corners[side], corners[(side + 1) % 4]) {
            return 0.0;
        }
    }
    let mut best = point_box_distance(a, bounds).min(point_box_distance(b, bounds));
    for corner in corners {
        best = best.min(geometry::point_segment_distance_squared(corner, a, b).sqrt());
    }
    best
}

impl Shape {
    fn bounds(&self) -> Box2 {
        match self {
            Shape::Rect(b) => *b,
            Shape::Capsule(a, b, r) => grow(box_of(&[*a, *b]), *r),
            Shape::Polygon(points) => box_of(points),
        }
    }

    fn distance(&self, bounds: Box2) -> f64 {
        match self {
            Shape::Rect(b) => box_box_distance(*b, bounds),
            Shape::Capsule(a, b, r) => (segment_box_distance(*a, *b, bounds) - r).max(0.0),
            Shape::Polygon(points) => {
                if points.len() < 3 {
                    return box_box_distance(box_of(points), bounds);
                }
                let corner = [bounds[0], bounds[1]];
                if geometry::point_in_polygon(corner, points) {
                    return 0.0;
                }
                let mut best = f64::INFINITY;
                for index in 0..points.len() {
                    let next = points[(index + 1) % points.len()];
                    best = best.min(segment_box_distance(points[index], next, bounds));
                    if best == 0.0 {
                        break;
                    }
                }
                best
            }
        }
    }
}

struct Obstacle {
    shape: Shape,
    bounds: Box2,
    kind: Kind,
    /// The footprint the item belongs to.
    owner: Option<usize>,
    /// The board item the shape is of (KiCad reports an item once).
    item: usize,
}

const CELL: f64 = 2.0;

/// The obstacles of one side, in a grid of `CELL` squares.
#[derive(Default)]
struct Side {
    obstacles: Vec<Obstacle>,
    grid: HashMap<(i64, i64), Vec<usize>>,
}

fn cells(b: Box2) -> impl Iterator<Item = (i64, i64)> {
    let x = ((b[0] / CELL).floor() as i64, (b[2] / CELL).floor() as i64);
    let y = ((b[1] / CELL).floor() as i64, (b[3] / CELL).floor() as i64);
    // A huge item (a frame) is listed in at most 256 x 256 cells.
    let x = (x.0, x.1.min(x.0 + 256));
    let y = (y.0, y.1.min(y.0 + 256));
    (x.0..=x.1).flat_map(move |i| (y.0..=y.1).map(move |j| (i, j)))
}

impl Side {
    fn add(&mut self, shape: Shape, kind: Kind, owner: Option<usize>, item: usize) {
        let bounds = shape.bounds();
        if !bounds[0].is_finite() || !bounds[2].is_finite() {
            return;
        }
        let index = self.obstacles.len();
        for cell in cells(bounds) {
            self.grid.entry(cell).or_default().push(index);
        }
        self.obstacles.push(Obstacle {
            shape,
            bounds,
            kind,
            owner,
            item,
        });
    }

    /// The obstacles whose boxes come within `reach` of `bounds`, each once.
    fn near(&self, bounds: Box2, reach: f64) -> Vec<usize> {
        let query = grow(bounds, reach);
        let mut found: Vec<usize> = cells(query)
            .filter_map(|cell| self.grid.get(&cell))
            .flatten()
            .copied()
            .filter(|index| boxes_meet(self.obstacles[*index].bounds, query))
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }
}

/// A field to place.
struct Field {
    footprint: usize,
    /// Index of the field's node among the footprint's children.
    child: usize,
    reference: String,
    name: String,
    back: bool,
    text: String,
    style: TextStyle,
    /// Board position and absolute angle now.
    anchor: [f64; 2],
    angle: f64,
    /// The footprint's body (courtyard and pads) on the board.
    body: Box2,
}

/// What KiCad will find of one field at one spot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Findings {
    pub over_copper: usize,
    pub overlap: usize,
    pub edge: usize,
}

impl Findings {
    pub(crate) fn total(&self) -> usize {
        self.over_copper + self.overlap + self.edge
    }
}

/// The scorer: every obstacle of both sides and the outline.
pub(crate) struct Scorer {
    sides: [Side; 2],
    /// The outline's pieces and cutouts (closed loops); empty without one.
    pieces: Vec<Vec<[f64; 2]>>,
    cutouts: Vec<Vec<[f64; 2]>>,
    edges: Side,
    rules: SilkRules,
    /// Items numbered so far.
    items: usize,
}

/// One silkscreen item's width (`stroke` or KiCad 6's `width`).
fn stroke_width(item: &Expr) -> f64 {
    item.child("stroke")
        .and_then(|stroke| form_atom(stroke, "width", 1))
        .or_else(|| form_atom(item, "width", 1))
        .and_then(|width| width.parse::<f64>().ok())
        .unwrap_or(0.0)
        .max(0.0)
}

fn filled(item: &Expr) -> Option<bool> {
    let fill = item.child("fill")?;
    let value = fill
        .children()
        .get(1)
        .and_then(Expr::atom)
        .or_else(|| form_atom(fill, "type", 1))?;
    Some(matches!(value, "yes" | "solid"))
}

/// The shapes of a graphic (line, rectangle, circle, arc, polygon, curve),
/// its points mapped by `place`.
fn graphic_shapes(item: &Expr, place: &dyn Fn([f64; 2]) -> [f64; 2]) -> Result<Vec<Shape>, String> {
    let radius = stroke_width(item) / 2.0;
    let kind = item.head().unwrap_or("");
    let kind = kind
        .strip_prefix("fp_")
        .or(kind.strip_prefix("gr_"))
        .unwrap_or(kind);
    let outline = |points: &[[f64; 2]], closed: bool| -> Vec<Shape> {
        let mut shapes = Vec::new();
        for pair in points.windows(2) {
            shapes.push(Shape::Capsule(pair[0], pair[1], radius));
        }
        if closed && points.len() > 2 {
            shapes.push(Shape::Capsule(points[points.len() - 1], points[0], radius));
        }
        if points.len() == 1 {
            shapes.push(Shape::Capsule(points[0], points[0], radius));
        }
        shapes
    };
    Ok(match kind {
        "line" | "arc" => {
            let points: Vec<[f64; 2]> = outline::outline_points(item)?
                .into_iter()
                .map(place)
                .collect();
            outline(&points, false)
        }
        "curve" => {
            let points: Vec<[f64; 2]> = outline::pts_points(item)?.into_iter().map(place).collect();
            outline(&points, false)
        }
        "rect" => {
            let (start, end) = (form_xy(item, "start")?, form_xy(item, "end")?);
            let corners: Vec<[f64; 2]> = [start, [end[0], start[1]], end, [start[0], end[1]]]
                .into_iter()
                .map(place)
                .collect();
            let mut shapes = outline(&corners, true);
            if filled(item) == Some(true) {
                shapes.push(Shape::Polygon(corners));
            }
            shapes
        }
        "circle" => {
            let center = form_xy(item, "center")?;
            let ring = distance_squared(center, form_xy(item, "end")?).sqrt();
            let middle = place(center);
            if filled(item) == Some(true) {
                vec![Shape::Capsule(middle, middle, ring + radius)]
            } else {
                let points: Vec<[f64; 2]> = (0..24)
                    .map(|step| {
                        let angle = step as f64 / 24.0 * std::f64::consts::TAU;
                        // The chords cut inside the ring by at most 0.9 %.
                        let reach = ring / (std::f64::consts::PI / 24.0).cos();
                        [
                            middle[0] + reach * angle.cos(),
                            middle[1] + reach * angle.sin(),
                        ]
                    })
                    .collect();
                outline(&points, true)
            }
        }
        "poly" => {
            let points: Vec<[f64; 2]> = outline::pts_points(item)?.into_iter().map(place).collect();
            let mut shapes = outline(&points, true);
            // A polygon is filled unless it says otherwise (KiCad 6 wrote
            // none).
            if filled(item) != Some(false) && points.len() > 2 {
                shapes.push(Shape::Polygon(points));
            }
            shapes
        }
        _ => Vec::new(),
    })
}

/// Whether a text, field or property is shown.
pub(crate) fn shown(item: &Expr) -> bool {
    let hidden = item
        .child("hide")
        .is_some_and(|hide| hide.children().get(1).and_then(Expr::atom) != Some("no"))
        || item
            .children()
            .iter()
            .any(|child| child.atom() == Some("hide"))
        || item.child("effects").is_some_and(|effects| {
            effects
                .children()
                .iter()
                .any(|child| child.atom() == Some("hide"))
        });
    !hidden
}

/// A footprint's field to place: its Reference or Value (a KiCad 8
/// `property` or a KiCad 6 `fp_text`), its name.
fn field_name(child: &Expr) -> Option<&'static str> {
    match child.head() {
        Some("property") => match child.children().get(1).and_then(Expr::atom) {
            Some("Reference") => Some("Reference"),
            Some("Value") => Some("Value"),
            _ => None,
        },
        Some("fp_text") => match child.children().get(1).and_then(Expr::atom) {
            Some("reference") => Some("Reference"),
            Some("value") => Some("Value"),
            _ => None,
        },
        _ => None,
    }
}

/// The text a footprint text shows: its own (a property's value), with
/// `${REFERENCE}` and `${VALUE}` replaced.
fn shown_text(child: &Expr, reference: &str, value: &str) -> String {
    let index = if child.head() == Some("property") || child.head() == Some("fp_text") {
        2
    } else {
        1
    };
    child
        .children()
        .get(index)
        .and_then(Expr::atom)
        .unwrap_or("")
        .replace("${REFERENCE}", reference)
        .replace("${VALUE}", value)
}

fn silk_layer(item: &Expr) -> Option<bool> {
    match form_atom(item, "layer", 1)? {
        "F.SilkS" => Some(false),
        "B.SilkS" => Some(true),
        _ => None,
    }
}

fn mask_layer(item: &Expr) -> Option<bool> {
    match form_atom(item, "layer", 1)? {
        "F.Mask" => Some(false),
        "B.Mask" => Some(true),
        _ => None,
    }
}

/// A pad's mask openings (front, back) and shape, `None` for a pad
/// without one.
fn pad_shapes(
    pad: &Expr,
    place: &dyn Fn([f64; 2]) -> [f64; 2],
    footprint_margin: Option<f64>,
    board_margin: f64,
) -> Result<([bool; 2], Vec<Shape>), String> {
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
    let sides = [
        layers
            .iter()
            .any(|l| matches!(*l, "F.Mask" | "*.Mask" | "F&B.Mask")),
        layers
            .iter()
            .any(|l| matches!(*l, "B.Mask" | "*.Mask" | "F&B.Mask")),
    ];
    if !sides[0] && !sides[1] {
        return Ok((sides, Vec::new()));
    }
    let margin = form_f64(pad, "solder_mask_margin", 1)
        .ok()
        .or(footprint_margin)
        .unwrap_or(board_margin);
    let pad_at = form_at(pad)?;
    let size = form_xy(pad, "size").unwrap_or([0.0, 0.0]);
    let offset = pad
        .child("drill")
        .and_then(|drill| drill.child("offset"))
        .and_then(|offset| {
            Some([
                expression_coordinate(offset, 1, "pad offset x").ok()?,
                expression_coordinate(offset, 2, "pad offset y").ok()?,
            ])
        })
        .unwrap_or([0.0, 0.0]);
    // The pad's own frame: its centre at `pad_at` in the footprint, turned
    // by the pad's absolute angle on the board.
    let center = place([pad_at[0], pad_at[1]]);
    let local = |point: [f64; 2]| {
        let turned = rotate_vector([point[0] + offset[0], point[1] + offset[1]], -pad_at[2]);
        [center[0] + turned[0], center[1] + turned[1]]
    };
    let shape = pad.children().get(3).and_then(Expr::atom).unwrap_or("rect");
    let half = [size[0] / 2.0 + margin, size[1] / 2.0 + margin];
    let mut shapes = Vec::new();
    match shape {
        "circle" => {
            let middle = local([0.0, 0.0]);
            shapes.push(Shape::Capsule(middle, middle, half[0].max(0.0)));
        }
        "oval" => {
            let (long, short) = if half[0] >= half[1] {
                ([half[0] - half[1], 0.0], half[1])
            } else {
                ([0.0, half[1] - half[0]], half[0])
            };
            shapes.push(Shape::Capsule(
                local([-long[0], -long[1]]),
                local(long),
                short.max(0.0),
            ));
        }
        _ => {
            let corners: Vec<[f64; 2]> = [
                [-half[0], -half[1]],
                [half[0], -half[1]],
                [half[0], half[1]],
                [-half[0], half[1]],
            ]
            .into_iter()
            .map(local)
            .collect();
            shapes.push(Shape::Polygon(corners));
            // A custom pad's primitives: their box.
            if shape == "custom"
                && let Some(primitives) = pad.child("primitives")
            {
                let mut points = Vec::new();
                for primitive in primitives.children() {
                    let width = stroke_width(primitive) / 2.0 + margin;
                    for point in outline::pts_points(primitive)?
                        .into_iter()
                        .chain(outline::outline_points(primitive)?)
                    {
                        let at = local(point);
                        points.push([at[0] - width, at[1] - width]);
                        points.push([at[0] + width, at[1] + width]);
                    }
                }
                if !points.is_empty() {
                    shapes.push(Shape::Rect(box_of(&points)));
                }
            }
        }
    }
    Ok((sides, shapes))
}

impl Scorer {
    /// Every obstacle of `pcb` but the fields in `movable` (footprint index,
    /// child index), which are placed one by one (`add_field`).
    fn collect(
        pcb: &Expr,
        rules: SilkRules,
        movable: &BTreeSet<(usize, usize)>,
    ) -> Result<Self, String> {
        let mut scorer = Self {
            sides: [Side::default(), Side::default()],
            pieces: Vec::new(),
            cutouts: Vec::new(),
            edges: Side::default(),
            rules,
            items: 0,
        };
        let mut item = 0;
        let board_margin = pcb
            .child("setup")
            .and_then(|setup| form_f64(setup, "pad_to_mask_clearance", 1).ok())
            .unwrap_or(0.0);
        if let Ok(loops) = outline::board_loops(pcb) {
            scorer.pieces = loops.piece_loops.clone();
            scorer.cutouts = loops.cutouts.clone();
            for polygon in loops.piece_loops.iter().chain(&loops.cutouts) {
                for index in 0..polygon.len() {
                    let next = polygon[(index + 1) % polygon.len()];
                    scorer.edges.add(
                        Shape::Capsule(polygon[index], next, 0.0),
                        Kind::Edge,
                        None,
                        0,
                    );
                }
            }
        }
        let identity = |point: [f64; 2]| point;
        for (index, footprint) in pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .enumerate()
        {
            let at = form_at(footprint)?;
            let place = |local: [f64; 2]| {
                let offset = rotate_vector(local, -at[2]);
                [at[0] + offset[0], at[1] + offset[1]]
            };
            let reference = crate::kicad_nets::footprint_reference(footprint).unwrap_or_default();
            let value = footprint
                .children()
                .iter()
                .find(|child| field_name(child) == Some("Value"))
                .map(|child| shown_text(child, "", ""))
                .unwrap_or_default();
            let footprint_margin = form_f64(footprint, "solder_mask_margin", 1).ok();
            for (child_index, child) in footprint.children().iter().enumerate() {
                match child.head() {
                    Some("pad") => {
                        let (sides, shapes) =
                            pad_shapes(child, &place, footprint_margin, board_margin)?;
                        item += 1;
                        for (side, opened) in sides.iter().enumerate() {
                            if *opened {
                                for shape in &shapes {
                                    scorer.sides[side].add(
                                        shape.clone(),
                                        Kind::Mask,
                                        Some(index),
                                        item,
                                    );
                                }
                            }
                        }
                    }
                    Some("fp_text" | "property") => {
                        if movable.contains(&(index, child_index)) || !shown(child) {
                            continue;
                        }
                        let Some(back) = silk_layer(child) else {
                            continue;
                        };
                        let Ok(text_at) = form_at(child) else {
                            continue;
                        };
                        let text = shown_text(child, &reference, &value);
                        if let Some(bounds) = text_box(
                            &text,
                            &TextStyle::of(child),
                            place([text_at[0], text_at[1]]),
                            text_at[2],
                        ) {
                            item += 1;
                            scorer.sides[back as usize].add(
                                Shape::Rect(bounds),
                                Kind::Silk,
                                Some(index),
                                item,
                            );
                        }
                    }
                    Some("fp_text_box") => {
                        if let Some(back) = silk_layer(child)
                            && let (Ok(start), Ok(end)) =
                                (form_xy(child, "start"), form_xy(child, "end"))
                        {
                            let points = [place(start), place(end)];
                            item += 1;
                            scorer.sides[back as usize].add(
                                Shape::Rect(box_of(&points)),
                                Kind::Silk,
                                Some(index),
                                item,
                            );
                        }
                    }
                    Some(
                        "fp_line" | "fp_rect" | "fp_circle" | "fp_arc" | "fp_poly" | "fp_curve",
                    ) => {
                        let (side, kind) = if let Some(back) = silk_layer(child) {
                            (back, Kind::Silk)
                        } else if let Some(back) = mask_layer(child) {
                            (back, Kind::Mask)
                        } else {
                            continue;
                        };
                        item += 1;
                        for shape in graphic_shapes(child, &place)? {
                            scorer.sides[side as usize].add(shape, kind, Some(index), item);
                        }
                    }
                    _ => {}
                }
            }
            // The body: courtyard and pads.
            let (center, size, _) = crate::board_placer::local_body(footprint)?;
            let middle = place(center);
            let (sin, cos) = (-at[2]).to_radians().sin_cos();
            let half = [
                (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0,
                (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0,
            ];
            let back = form_atom(footprint, "layer", 1) == Some("B.Cu");
            scorer.sides[back as usize].add(
                Shape::Rect([
                    middle[0] - half[0],
                    middle[1] - half[1],
                    middle[0] + half[0],
                    middle[1] + half[1],
                ]),
                Kind::Body,
                Some(index),
                0,
            );
        }
        for node in pcb.children() {
            match node.head() {
                Some("gr_text") => {
                    let Some(back) = silk_layer(node) else {
                        continue;
                    };
                    if !shown(node) {
                        continue;
                    }
                    let (Ok(at), Some(text)) =
                        (form_at(node), node.children().get(1).and_then(Expr::atom))
                    else {
                        continue;
                    };
                    item += 1;
                    for (line, offset) in text.split('\n').zip(0..) {
                        let style = TextStyle::of(node);
                        // Lines 1.62 heights apart, KiCad's interline.
                        let down =
                            rotate_vector([0.0, 1.62 * style.size[0] * offset as f64], -at[2]);
                        if let Some(bounds) =
                            text_box(line, &style, [at[0] + down[0], at[1] + down[1]], at[2])
                        {
                            scorer.sides[back as usize].add(
                                Shape::Rect(bounds),
                                Kind::Silk,
                                None,
                                item,
                            );
                        }
                    }
                }
                Some("gr_text_box") => {
                    if let Some(back) = silk_layer(node)
                        && let (Ok(start), Ok(end)) = (form_xy(node, "start"), form_xy(node, "end"))
                    {
                        item += 1;
                        scorer.sides[back as usize].add(
                            Shape::Rect(box_of(&[start, end])),
                            Kind::Silk,
                            None,
                            item,
                        );
                    }
                }
                Some("gr_line" | "gr_rect" | "gr_circle" | "gr_arc" | "gr_poly" | "gr_curve") => {
                    let (side, kind) = if let Some(back) = silk_layer(node) {
                        (back, Kind::Silk)
                    } else if let Some(back) = mask_layer(node) {
                        (back, Kind::Mask)
                    } else {
                        continue;
                    };
                    item += 1;
                    for shape in graphic_shapes(node, &identity)? {
                        scorer.sides[side as usize].add(shape, kind, None, item);
                    }
                }
                _ => {}
            }
        }
        scorer.items = item;
        Ok(scorer)
    }

    /// A placed field: silkscreen for the fields after it.
    fn add_field(&mut self, back: bool, bounds: Box2, footprint: usize) {
        self.items += 1;
        let item = self.items;
        self.sides[back as usize].add(Shape::Rect(bounds), Kind::Silk, Some(footprint), item);
    }

    fn on_board(&self, point: [f64; 2]) -> bool {
        self.pieces.is_empty()
            || (self
                .pieces
                .iter()
                .any(|piece| geometry::point_in_polygon(point, piece))
                && !self
                    .cutouts
                    .iter()
                    .any(|cutout| geometry::point_in_polygon(point, cutout)))
    }

    /// KiCad's findings of a field box on a side, and whether it lies on
    /// another part's body (and on its own).
    pub(crate) fn score(
        &self,
        back: bool,
        bounds: Box2,
        footprint: Option<usize>,
    ) -> (Findings, bool, bool) {
        // Silkscreen keeps the project's clearance from silkscreen and the
        // edge; from a mask opening it only must not overlap (KiCad 10).
        let reach = self.rules.clearance + MARGIN;
        let side = &self.sides[back as usize];
        let mut findings = Findings::default();
        let (mut on_other, mut on_own) = (false, false);
        let mut seen: Vec<usize> = Vec::new();
        for index in side.near(bounds, reach) {
            let obstacle = &side.obstacles[index];
            match obstacle.kind {
                Kind::Body => {
                    if obstacle.shape.distance(bounds) <= 0.0 {
                        if obstacle.owner == footprint {
                            on_own = true;
                        } else {
                            on_other = true;
                        }
                    }
                }
                Kind::Mask | Kind::Silk => {
                    let reach = if obstacle.kind == Kind::Mask {
                        MARGIN
                    } else {
                        reach
                    };
                    if !seen.contains(&obstacle.item) && obstacle.shape.distance(bounds) < reach {
                        seen.push(obstacle.item);
                        if obstacle.kind == Kind::Mask {
                            findings.over_copper += 1;
                        } else {
                            findings.overlap += 1;
                        }
                    }
                }
                Kind::Edge => {}
            }
        }
        let center = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
        if !self.on_board(center)
            || self
                .edges
                .near(bounds, reach)
                .into_iter()
                .any(|index| self.edges.obstacles[index].shape.distance(bounds) < reach)
        {
            findings.edge = 1;
        }
        (findings, on_other, on_own)
    }
}

/// Where a box lies against a body: above, below, left, right or inside.
fn side_of(bounds: Box2, body: Box2) -> u8 {
    let center = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
    if inside_box(center, body) {
        return 4;
    }
    let middle = [(body[0] + body[2]) / 2.0, (body[1] + body[3]) / 2.0];
    let half = [
        ((body[2] - body[0]) / 2.0).max(0.1),
        ((body[3] - body[1]) / 2.0).max(0.1),
    ];
    let along = [
        (center[0] - middle[0]) / half[0],
        (center[1] - middle[1]) / half[1],
    ];
    if along[1].abs() >= along[0].abs() {
        if along[1] < 0.0 { 0 } else { 1 }
    } else if along[0] < 0.0 {
        2
    } else {
        3
    }
}

/// What the polish and the repair may do to a field.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FieldOptions {
    pub rules: SilkRules,
    /// Shrink a field to 0.8 of its size (not below the project's minimum
    /// text height and thickness) where that finds a clean spot.
    pub shrink: bool,
    /// Hide a field no candidate is clean for (`silkscreen.hide_when_blocked`).
    pub hide_when_blocked: bool,
}

/// The fields placed.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct KiCadSilkscreenReport {
    /// Visible Reference and Value fields on the silkscreen.
    pub fields: usize,
    /// Fields moved (or turned, or shrunk).
    pub moved: usize,
    /// Fields no spot is clean for, kept at the spot with the fewest
    /// findings: the reference, the field and the findings predicted there.
    pub blocked: Vec<String>,
    /// Fields hidden because no spot is clean (`silkscreen.hide_when_blocked`).
    pub hidden: Vec<String>,
    /// KiCad's findings the fields will still have, by KiCad's type
    /// (`silk_over_copper`, `silk_overlap`, `silk_edge_clearance`), as
    /// computed from our geometry.
    pub predicted: BTreeMap<String, usize>,
}

/// The outcome: the report and the blocked fields' boxes (footprint,
/// side, box).
pub(crate) struct FieldOutcome {
    pub report: KiCadSilkscreenReport,
    pub blocked_boxes: Vec<(usize, bool, Box2)>,
    /// The references of the blocked fields.
    pub blocked_texts: Vec<String>,
}

struct Choice {
    findings: Findings,
    cost: f64,
    anchor: [f64; 2],
    angle: f64,
    scale: f64,
    bounds: Box2,
}

fn set_at_keeping_flags(node: &mut Expr, at: [f64; 3]) -> Result<(), String> {
    let unlocked = node.child("at").is_some_and(|at| {
        at.children()
            .iter()
            .any(|value| value.atom() == Some("unlocked"))
    });
    set_form_at(node, at)?;
    if unlocked
        && let Expr::List(items) = node
        && let Some(Expr::List(values)) = items.iter_mut().find(|item| item.head() == Some("at"))
    {
        values.push(Expr::Atom("unlocked".into()));
    }
    Ok(())
}

fn scale_font(node: &mut Expr, scale: f64) {
    let Expr::List(parts) = node else {
        return;
    };
    let Some(Expr::List(effects)) = parts.iter_mut().find(|part| part.head() == Some("effects"))
    else {
        return;
    };
    let Some(Expr::List(font)) = effects.iter_mut().find(|part| part.head() == Some("font")) else {
        return;
    };
    for part in font.iter_mut() {
        if let Expr::List(values) = part {
            let count = match values.first().and_then(Expr::atom) {
                Some("size") => 2,
                Some("thickness") => 1,
                _ => continue,
            };
            for value in values.iter_mut().skip(1).take(count) {
                if let Some(number) = value.atom().and_then(|atom| atom.parse::<f64>().ok()) {
                    *value = Expr::Atom(((number * scale * 1e3).round() / 1e3).to_string());
                }
            }
        }
    }
}

fn hide(node: &mut Expr) {
    let Expr::List(parts) = node else {
        return;
    };
    if parts.first().and_then(Expr::atom) == Some("fp_text") {
        // KiCad 6: a bare `hide` after the layer.
        let at = parts
            .iter()
            .position(|part| part.head() == Some("layer"))
            .map_or(parts.len(), |index| index + 1);
        parts.insert(at, Expr::Atom("hide".into()));
        return;
    }
    parts.retain(|part| part.head() != Some("hide"));
    let at = parts
        .iter()
        .position(|part| part.head() == Some("layer"))
        .map_or(parts.len(), |index| index + 1);
    parts.insert(
        at,
        Expr::List(vec![Expr::Atom("hide".into()), Expr::Atom("yes".into())]),
    );
}

/// The distances from a body a field is tried at, and how many spots along
/// a side at most.
const GAPS: [f64; 6] = [0.1, 0.3, 0.6, 1.0, 1.5, 2.2];
const ALONG: usize = 13;

/// Places every visible Reference and Value field on the silkscreen of
/// `pcb` where KiCad finds nothing: where it is if that is clean, else the
/// clean candidate around (or, for a large part, inside) the part on the
/// side the field was on, nearest to where it was; with none clean, the
/// one with the fewest findings (or hidden: `hide_when_blocked`).
pub(crate) fn place_fields(pcb: &mut Expr, options: FieldOptions) -> Result<FieldOutcome, String> {
    let mut fields = Vec::new();
    let footprints: Vec<&Expr> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .collect();
    for (index, footprint) in footprints.iter().enumerate() {
        let at = form_at(footprint)?;
        let place = |local: [f64; 2]| {
            let offset = rotate_vector(local, -at[2]);
            [at[0] + offset[0], at[1] + offset[1]]
        };
        let reference = crate::kicad_nets::footprint_reference(footprint).unwrap_or_default();
        let (center, size, _) = crate::board_placer::local_body(footprint)?;
        let middle = place(center);
        let (sin, cos) = (-at[2]).to_radians().sin_cos();
        let half = [
            (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0,
            (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0,
        ];
        let body = [
            middle[0] - half[0],
            middle[1] - half[1],
            middle[0] + half[0],
            middle[1] + half[1],
        ];
        for (child_index, child) in footprint.children().iter().enumerate() {
            let Some(name) = field_name(child) else {
                continue;
            };
            let Some(back) = silk_layer(child) else {
                continue;
            };
            let text = shown_text(child, &reference, "");
            if !shown(child) || text.trim().is_empty() {
                continue;
            }
            let field_at = form_at(child)?;
            fields.push(Field {
                footprint: index,
                child: child_index,
                reference: reference.clone(),
                name: name.to_string(),
                back,
                text,
                style: TextStyle::of(child),
                anchor: place([field_at[0], field_at[1]]),
                angle: field_at[2],
                body,
            });
        }
    }
    let movable: BTreeSet<(usize, usize)> = fields
        .iter()
        .map(|field| (field.footprint, field.child))
        .collect();
    let mut scorer = Scorer::collect(pcb, options.rules, &movable)?;
    let mut report = KiCadSilkscreenReport {
        fields: fields.len(),
        ..Default::default()
    };
    let mut blocked_boxes = Vec::new();
    let mut blocked_texts = Vec::new();
    // Big parts first: small ones have more room around them; a part's
    // Reference before its Value.
    let mut order: Vec<usize> = (0..fields.len()).collect();
    order.sort_by(|a, b| {
        let area =
            |field: &Field| (field.body[2] - field.body[0]) * (field.body[3] - field.body[1]);
        area(&fields[*b])
            .total_cmp(&area(&fields[*a]))
            .then(fields[*a].footprint.cmp(&fields[*b].footprint))
            .then(fields[*a].child.cmp(&fields[*b].child))
    });
    // (field index, anchor, angle, scale, hide)
    let mut writes: Vec<(usize, [f64; 2], f64, f64, bool)> = Vec::new();
    for index in order {
        let field = &fields[index];
        let Some(current) = text_box(&field.text, &field.style, field.anchor, field.angle) else {
            continue;
        };
        let (findings, _, _) = scorer.score(field.back, current, Some(field.footprint));
        if findings.total() == 0 {
            scorer.add_field(field.back, current, field.footprint);
            continue;
        }
        let home = side_of(current, field.body);
        let center_now = [
            (current[0] + current[2]) / 2.0,
            (current[1] + current[3]) / 2.0,
        ];
        let mut best = Choice {
            findings,
            cost: 0.0,
            anchor: field.anchor,
            angle: field.angle,
            scale: 1.0,
            bounds: current,
        };
        let mut scales = vec![1.0];
        if options.shrink {
            let scale = 0.8;
            let floor = options.rules.min_text_height.max(0.8);
            if field.style.size[0] * scale >= floor - 1e-9
                && field.style.thickness * scale >= options.rules.min_text_thickness - 1e-9
            {
                scales.push(scale);
            }
        }
        let quarter = (field.angle + 90.0).rem_euclid(360.0);
        let b = field.body;
        for &scale in &scales {
            let style = field.style.scaled(scale);
            for (angle, turned) in [(field.angle, false), (quarter, true)] {
                // The ink box relative to the anchor at this angle.
                let Some(relative) = text_box(&field.text, &style, [0.0, 0.0], angle) else {
                    continue;
                };
                let size = [relative[2] - relative[0], relative[3] - relative[1]];
                let mut boxes: Vec<(Box2, u8)> = Vec::new();
                let slide = |low: f64, high: f64, length: f64| -> Vec<f64> {
                    // Box starts from one beyond the corner to the other.
                    let (from, to) = (low - length, high);
                    let steps = (((to - from) / 0.5).ceil() as usize).clamp(1, ALONG - 1);
                    let mut starts: Vec<f64> = (0..=steps)
                        .map(|step| from + (to - from) * step as f64 / steps as f64)
                        .collect();
                    // Centred and flush with either end of the side.
                    starts.extend([(low + high - length) / 2.0, low, high - length]);
                    starts
                };
                for gap in GAPS {
                    for x in slide(b[0], b[2], size[0]) {
                        boxes.push(([x, b[1] - gap - size[1], x + size[0], b[1] - gap], 0));
                        boxes.push(([x, b[3] + gap, x + size[0], b[3] + gap + size[1]], 1));
                    }
                    for y in slide(b[1], b[3], size[1]) {
                        boxes.push(([b[0] - gap - size[0], y, b[0] - gap, y + size[1]], 2));
                        boxes.push(([b[2] + gap, y, b[2] + gap + size[0], y + size[1]], 3));
                    }
                }
                // Inside a part large enough: a grid over its body.
                if b[2] - b[0] > size[0] + 0.4 && b[3] - b[1] > size[1] + 0.4 {
                    let room = [b[2] - b[0] - size[0], b[3] - b[1] - size[1]];
                    let steps = [
                        ((room[0] / 0.5).ceil() as usize).clamp(1, 8),
                        ((room[1] / 0.5).ceil() as usize).clamp(1, 8),
                    ];
                    for i in 0..=steps[0] {
                        for j in 0..=steps[1] {
                            let x = b[0] + room[0] * i as f64 / steps[0] as f64;
                            let y = b[1] + room[1] * j as f64 / steps[1] as f64;
                            boxes.push(([x, y, x + size[0], y + size[1]], 4));
                        }
                    }
                }
                for (bounds, side) in boxes {
                    let (findings, on_other, on_own) =
                        scorer.score(field.back, bounds, Some(field.footprint));
                    if findings.total() > best.findings.total() {
                        continue;
                    }
                    let center = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
                    let cost = distance_squared(center, center_now).sqrt()
                        + if side == home { 0.0 } else { 1.5 }
                        + if turned { 0.8 } else { 0.0 }
                        + if scale < 1.0 { 3.0 } else { 0.0 }
                        + if on_other { 2.5 } else { 0.0 }
                        + if on_own && side != 4 { 1.0 } else { 0.0 };
                    if findings.total() < best.findings.total() || cost < best.cost - 1e-9 {
                        best = Choice {
                            findings,
                            cost,
                            // The anchor that puts the ink there.
                            anchor: [bounds[0] - relative[0], bounds[1] - relative[1]],
                            angle,
                            scale,
                            bounds,
                        };
                    }
                }
            }
            if best.findings.total() == 0 {
                break;
            }
        }
        let blocked = best.findings.total() > 0;
        if blocked && options.hide_when_blocked {
            report
                .hidden
                .push(format!("{} ({})", field.reference, field.name));
            writes.push((index, field.anchor, field.angle, 1.0, true));
            continue;
        }
        if blocked {
            let mut kinds = Vec::new();
            for (count, kind) in [
                (best.findings.over_copper, "silk_over_copper"),
                (best.findings.overlap, "silk_overlap"),
                (best.findings.edge, "silk_edge_clearance"),
            ] {
                if count > 0 {
                    *report.predicted.entry(kind.to_string()).or_default() += count;
                    kinds.push(format!("{count} {kind}"));
                }
            }
            report.blocked.push(format!(
                "{} ({}): {}",
                field.reference,
                field.name,
                kinds.join(", ")
            ));
            blocked_boxes.push((field.footprint, field.back, best.bounds));
            blocked_texts.push(field.text.clone());
        }
        scorer.add_field(field.back, best.bounds, field.footprint);
        if best.anchor != field.anchor || best.angle != field.angle || best.scale != 1.0 {
            writes.push((index, best.anchor, best.angle, best.scale, false));
        }
    }
    // Write: positions in the footprint's frame, angles absolute.
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut footprint_items: Vec<&mut Expr> = items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
        .collect();
    for (index, anchor, angle, scale, hidden) in writes {
        let field = &fields[index];
        let footprint = &mut footprint_items[field.footprint];
        let at = form_at(footprint)?;
        let Expr::List(children) = footprint else {
            continue;
        };
        let node = &mut children[field.child];
        if hidden {
            hide(node);
            continue;
        }
        let local = rotate_vector([anchor[0] - at[0], anchor[1] - at[1]], at[2]);
        set_at_keeping_flags(
            node,
            [
                (local[0] * 1e4).round() / 1e4,
                (local[1] * 1e4).round() / 1e4,
                angle,
            ],
        )?;
        if scale < 1.0 {
            scale_font(node, scale);
        }
        report.moved += 1;
    }
    Ok(FieldOutcome {
        report,
        blocked_boxes,
        blocked_texts,
    })
}

/// The findings KiCad will have for the fields of `pcb` as they are, by
/// field (`reference (name)`): what the scorer predicts, for tests and
/// reports.
#[cfg(test)]
pub(crate) fn predict_fields(
    pcb: &Expr,
    rules: SilkRules,
) -> Result<BTreeMap<String, Findings>, String> {
    // Score every field against everything else, every other field
    // included.
    let mut result = BTreeMap::new();
    for (index, footprint) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .enumerate()
    {
        let at = form_at(footprint)?;
        let reference = crate::kicad_nets::footprint_reference(footprint).unwrap_or_default();
        for (child_index, child) in footprint.children().iter().enumerate() {
            let Some(name) = field_name(child) else {
                continue;
            };
            let Some(back) = silk_layer(child) else {
                continue;
            };
            if !shown(child) {
                continue;
            }
            let field_at = form_at(child)?;
            let offset = rotate_vector([field_at[0], field_at[1]], -at[2]);
            let text = shown_text(child, &reference, "");
            let Some(bounds) = text_box(
                &text,
                &TextStyle::of(child),
                [at[0] + offset[0], at[1] + offset[1]],
                field_at[2],
            ) else {
                continue;
            };
            let movable = BTreeSet::from([(index, child_index)]);
            let scorer = Scorer::collect(pcb, rules, &movable)?;
            let (findings, _, _) = scorer.score(back, bounds, Some(index));
            result.insert(format!("{reference} ({name})"), findings);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(height: f64, thickness: f64) -> TextStyle {
        TextStyle {
            size: [height, height],
            thickness,
            horizontal: Justify::Center,
            vertical: Justify::Center,
            mirrored: false,
        }
    }

    fn near(a: [f64; 4], b: [f64; 4]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.002)
    }

    #[test]
    fn the_ink_is_kicads_stroke_font_as_pcbnew_measures_it() {
        // pcbnew's GetEffectiveTextShape().BBox(), KiCad 10.0.7.
        for (text, ink) in [
            ("C26", [-1.337, 1.337, -0.620, 0.530]),
            ("RN7", [-1.337, 1.432, -0.620, 0.530]),
            ("gy", [-0.718, 0.765, -0.287, 0.863]),
            ("100n", [-1.765, 1.718, -0.620, 0.530]),
            ("MCU_LINK1", [-3.956, 4.004, -0.620, 0.625]),
            ("J2 (x)", [-2.242, 2.146, -0.763, 0.911]),
        ] {
            let found = text_ink(text, &style(1.0, 0.15)).unwrap();
            assert!(near(found, ink), "{text}: {found:?} against {ink:?}");
        }
        // Pen and size: 1.27 at 0.15, a pen clamped to a quarter of the size.
        assert!(near(
            text_ink("HH", &style(1.27, 0.15)).unwrap(),
            [-1.1031, 1.1031, -0.7653, 0.6547]
        ));
        assert!(
            near(
                text_ink("H", &style(1.0, 0.3)).unwrap(),
                [-0.4107, 0.4107, -0.678, 0.572]
            ) || (text_ink("H", &style(1.0, 0.3)).unwrap()[2] - -0.678).abs() < 0.005
        );
        // Left and right justified, mirrored, upright at 180 degrees.
        let mut left = style(1.0, 0.15);
        left.horizontal = Justify::Start;
        assert!(near(
            text_ink("HH", &left).unwrap(),
            [0.2618, 2.0308, -0.6202, 0.5298]
        ));
        left.mirrored = true;
        assert!(near(
            text_ink("HH", &left).unwrap(),
            [-2.0308, -0.2618, -0.6202, 0.5298]
        ));
        left.mirrored = false;
        let at = |angle| text_box("HH", &left, [0.0, 0.0], angle).unwrap();
        assert!(near(at(180.0), at(0.0)));
        assert!(near(at(270.0), at(90.0)));
        // At 90 degrees the text reads upwards: along it is -y.
        let up = at(90.0);
        assert!(near(
            up,
            [
                -0.6202 - MARGIN,
                -2.0308 - MARGIN,
                0.5298 + MARGIN,
                -0.2618 + MARGIN
            ]
        ));
        assert!(text_ink("   ", &style(1.0, 0.15)).is_none());
    }

    /// A footprint of two mask-opened pads 1.9 mm apart (an 0805), its
    /// Reference at `field` (footprint frame, absolute angle) and `extra`
    /// items.
    fn part(reference: &str, at: [f64; 3], field: [f64; 3], extra: &str) -> String {
        format!(
            r#"(footprint "T:{reference}" (layer "F.Cu") (at {} {} {})
  (property "Reference" "{reference}" (at {} {} {}) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
  (property "Value" "x" (at 0 1.68 0) (layer "F.Fab") (effects (font (size 1 1) (thickness 0.15))))
  (attr smd)
  (fp_rect (start -1.7 -0.98) (end 1.7 0.98) (stroke (width 0.05) (type solid)) (fill no) (layer "F.CrtYd"))
  {extra}
  (pad "1" smd rect (at -0.95 0 {}) (size 1 1.45) (layers "F.Cu" "F.Mask"))
  (pad "2" smd rect (at 0.95 0 {}) (size 1 1.45) (layers "F.Cu" "F.Mask")))
"#,
            at[0], at[1], at[2], field[0], field[1], field[2], at[2], at[2]
        )
    }

    fn test_board() -> String {
        let mut board = String::from(
            r#"(kicad_pcb (version 20260206) (generator "pcbnew") (generator_version "10.0")
 (general (thickness 1.6) (legacy_teardrops no)) (paper "A4")
 (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (5 "F.SilkS" user "F.Silkscreen") (7 "B.SilkS" user "B.Silkscreen")
  (1 "F.Mask" user) (3 "B.Mask" user) (25 "Edge.Cuts" user) (31 "F.CrtYd" user "F.Courtyard") (29 "B.CrtYd" user "B.Courtyard") (35 "F.Fab" user) (33 "B.Fab" user))
 (setup (pad_to_mask_clearance 0) (allow_soldermask_bridges_in_footprints yes) (tenting (front yes) (back yes)))
 (net 0 "")
 (gr_rect (start 0 0) (end 60 40) (stroke (width 0.05) (type default)) (fill no) (layer "Edge.Cuts"))
 (gr_text "LOGO" (at 30 33.4 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
"#,
        );
        let line = |y: f64| {
            format!(
                r#"(fp_line (start -2 {y}) (end 2 {y}) (stroke (width 0.12) (type solid)) (layer "F.SilkS"))"#
            )
        };
        // Over its own pads; clean; over its own outline.
        board += &part("A1", [10.0, 10.0, 0.0], [0.0, 0.0, 0.0], "");
        board += &part("A2", [20.0, 10.0, 0.0], [0.0, -1.68, 0.0], "");
        board += &part("A3", [30.0, 10.0, 0.0], [0.0, -1.68, 0.0], &line(-1.68));
        // Over a neighbour's pad (A5's, whose own label is clear).
        board += &part("A4", [40.0, 10.0, 0.0], [0.0, -3.0, 0.0], "");
        board += &part("A5", [40.0, 6.0, 90.0], [3.5, 0.0, 0.0], "");
        // Across the edge.
        board += &part("A6", [50.0, 1.2, 0.0], [0.0, -1.68, 0.0], "");
        // Two labels on each other.
        board += &part("A7", [10.0, 25.0, 0.0], [0.0, -1.68, 0.0], "");
        board += &part("A8", [11.0, 25.0, 0.0], [0.0, -1.68, 0.0], "");
        // On copper without a mask opening: nothing.
        board += &part(
            "A9",
            [20.0, 25.0, 0.0],
            [0.0, -1.68, 0.0],
            r#"(pad "3" smd rect (at 0 -1.68 0) (size 0.5 0.5) (layers "F.Cu"))"#,
        );
        // Turned with its part, over its pads.
        board += &part("A10", [30.0, 25.0, 90.0], [0.0, 0.0, 90.0], "");
        // A visible Value on the Reference.
        board += &part("A11", [40.0, 25.0, 0.0], [0.0, -1.68, 0.0], "").replace(
            r#"(at 0 1.68 0) (layer "F.Fab")"#,
            r#"(at 0 -1.68 0) (layer "F.SilkS")"#,
        );
        // 0.15 mm from its own outline: clean at clearance 0, not at 0.2;
        // 0.05 mm from a pad: clean at both (a mask opening only must not
        // be overlapped).
        board += &part(
            "A13",
            [10.0, 35.0, 0.0],
            [0.0, -1.68, 0.0],
            &format!(
                r#"(pad "3" smd rect (at 0 -0.8 0) (size 0.6 0.6) (layers "F.Cu" "F.Mask")) {}"#,
                line(-1.68 - 0.62 - 0.15 - 0.06)
            ),
        );
        // Under the board's own text.
        board += &part("A14", [30.0, 35.0, 0.0], [0.0, -1.68, 0.0], "");
        // On the back, through-hole pads, mirrored.
        board += r#"(footprint "T:A12" (layer "B.Cu") (at 50 25 0)
  (property "Reference" "A12" (at 0 0 0) (layer "B.SilkS") (effects (font (size 1 1) (thickness 0.15)) (justify mirror)))
  (property "Value" "x" (at 0 2 0) (layer "B.Fab") (effects (font (size 1 1) (thickness 0.15)) (justify mirror)))
  (fp_rect (start -2.5 -1.2) (end 2.5 1.2) (stroke (width 0.05) (type solid)) (fill no) (layer "B.CrtYd"))
  (pad "1" thru_hole circle (at -1.27 0) (size 1.7 1.7) (drill 1) (layers "*.Cu" "*.Mask"))
  (pad "2" thru_hole circle (at 1.27 0) (size 1.7 1.7) (drill 1) (layers "*.Cu" "*.Mask")))
"#;
        board + ")\n"
    }

    /// KiCad's silkscreen findings per field (`A1 (Reference)`) by type.
    fn kicad_findings(directory: &Path, board: &str, clearance: f64) -> BTreeMap<String, Findings> {
        fs::create_dir_all(directory).unwrap();
        let pcb = directory.join("t.kicad_pcb");
        fs::write(&pcb, board).unwrap();
        fs::write(
            directory.join("t.kicad_pro"),
            format!(r#"{{"board": {{"design_settings": {{"rules": {{"min_silk_clearance": {clearance}}}}}}}}}"#),
        )
        .unwrap();
        let output = directory.join("drc.json");
        crate::native_report::run_kicad_report(
            &[
                "pcb",
                "drc",
                "--format",
                "json",
                "--severity-all",
                "--output",
            ],
            &output,
            &pcb,
        )
        .unwrap();
        let report: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&output).unwrap()).unwrap();
        let mut found: BTreeMap<String, Findings> = BTreeMap::new();
        for violation in report["violations"].as_array().unwrap() {
            let kind = violation["type"].as_str().unwrap();
            for item in violation["items"].as_array().unwrap() {
                let description = item["description"].as_str().unwrap();
                let Some((name, rest)) = description.split_once(" field of ") else {
                    continue;
                };
                let reference = rest.split_whitespace().next().unwrap();
                let entry = found.entry(format!("{reference} ({name})")).or_default();
                match kind {
                    "silk_over_copper" => entry.over_copper += 1,
                    "silk_overlap" => entry.overlap += 1,
                    "silk_edge_clearance" => entry.edge += 1,
                    _ => {}
                }
            }
        }
        found.retain(|_, findings| findings.total() > 0);
        found
    }

    fn by_type(findings: &BTreeMap<String, Findings>) -> BTreeMap<String, usize> {
        let mut total = BTreeMap::new();
        for findings in findings.values() {
            for (count, kind) in [
                (findings.over_copper, "silk_over_copper"),
                (findings.overlap, "silk_overlap"),
                (findings.edge, "silk_edge_clearance"),
            ] {
                if count > 0 {
                    *total.entry(kind.to_string()).or_default() += count;
                }
            }
        }
        total
    }

    #[test]
    fn the_scorer_predicts_kicads_silkscreen_findings_and_placing_clears_them() {
        if crate::native_report::check_kicad_cli().is_err() {
            eprintln!("kicad-cli (KiCad 10) not found: the scorer is not checked against KiCad");
            return;
        }
        let scratch =
            std::env::temp_dir().join(format!("pcb-maker-silk-fields-{}", std::process::id()));
        let board = test_board();
        for clearance in [0.0, 0.2] {
            let rules = SilkRules {
                clearance,
                ..SilkRules::default()
            };
            let mut predicted = predict_fields(&parse(&board).unwrap(), rules).unwrap();
            predicted.retain(|_, findings| findings.total() > 0);
            let kicad = kicad_findings(
                &scratch.join(format!("before-{clearance}")),
                &board,
                clearance,
            );
            assert_eq!(predicted, kicad, "clearance {clearance}");
            // The cases are what they say (a field pair counts once for
            // each field).
            let at = |key: &str| kicad.get(key).copied().unwrap_or_default();
            assert_eq!(at("A1 (Reference)").over_copper, 2);
            assert_eq!(at("A2 (Reference)").total(), 0);
            assert_eq!(at("A3 (Reference)").overlap, 1);
            assert_eq!(at("A4 (Reference)").over_copper, 1);
            assert_eq!(at("A6 (Reference)").edge, 1);
            assert_eq!(at("A7 (Reference)").overlap, 1);
            assert_eq!(at("A9 (Reference)").total(), 0);
            assert_eq!(at("A10 (Reference)").over_copper, 2);
            assert_eq!(at("A11 (Value)").overlap, 1);
            assert_eq!(at("A12 (Reference)").over_copper, 2);
            assert_eq!(at("A13 (Reference)").overlap, usize::from(clearance > 0.15));
            assert_eq!(at("A13 (Reference)").over_copper, 0);
            assert_eq!(at("A14 (Reference)").overlap, 1);
            // Placed: what is left is what the report predicts (nothing
            // here: every field has room).
            let mut pcb = parse(&board).unwrap();
            let outcome = place_fields(
                &mut pcb,
                FieldOptions {
                    rules,
                    shrink: false,
                    hide_when_blocked: false,
                },
            )
            .unwrap();
            let placed = format!("{}\n", encode(&pcb));
            let after = kicad_findings(
                &scratch.join(format!("after-{clearance}")),
                &placed,
                clearance,
            );
            assert_eq!(
                by_type(&after),
                outcome.report.predicted,
                "clearance {clearance}"
            );
            assert!(after.is_empty(), "{after:?}");
            assert_eq!(outcome.report.fields, 15);
            // A1, A3, A4, A6, one of A7 and A8, A10, one of A11's, A12, A14
            // (and A13 at 0.2).
            assert_eq!(
                outcome.report.moved,
                9 + usize::from(clearance > 0.15),
                "{:?}",
                outcome.report
            );
        }
        fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn a_field_without_a_clean_spot_is_kept_at_the_best_or_hidden() {
        // A label boxed in by a silkscreen frame on a tiny board.
        let board = r#"(kicad_pcb
 (gr_rect (start 0 0) (end 3 2.5) (layer "Edge.Cuts"))
 (footprint "R" (layer "F.Cu") (at 1.5 1.25)
  (property "Reference" "R100" (at 0 0 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
  (fp_rect (start -0.6 -0.4) (end 0.6 0.4) (layer "F.CrtYd"))
  (pad "1" smd rect (at -0.3 0) (size 0.4 0.6) (layers "F.Cu" "F.Mask"))
  (pad "2" smd rect (at 0.3 0) (size 0.4 0.6) (layers "F.Cu" "F.Mask"))))"#;
        let options = FieldOptions {
            rules: SilkRules::default(),
            shrink: true,
            hide_when_blocked: false,
        };
        let mut pcb = parse(board).unwrap();
        let outcome = place_fields(&mut pcb, options).unwrap();
        assert_eq!(outcome.report.blocked.len(), 1, "{:?}", outcome.report);
        assert!(outcome.report.blocked[0].starts_with("R100 (Reference): "));
        assert!(!outcome.report.predicted.is_empty());
        assert!(encode(&pcb).contains("\"R100\""));
        let mut pcb = parse(board).unwrap();
        let outcome = place_fields(
            &mut pcb,
            FieldOptions {
                hide_when_blocked: true,
                ..options
            },
        )
        .unwrap();
        assert_eq!(outcome.report.hidden, ["R100 (Reference)"]);
        assert!(outcome.report.predicted.is_empty());
        let footprint = pcb.child("footprint").unwrap();
        assert!(!shown(footprint.child("property").unwrap()));
    }
}

// Copyright (C) 2026 Toit contributors.

//! `outline`: the board's shape from the constraints, a rectangle sized
//! or given, a polygon, round corners, cutouts, several pieces, written
//! as Edge.Cuts graphics.

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadOutlineConstraint {
    /// Size in millimetres. Leave both out and the board is sized from its
    /// parts: `area_factor` times their total body area, at `aspect`.
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    /// Board area per unit of part area when sizing automatically.
    /// Default: 2 on two layers, 1.5 on more, the median of 616 open-source
    /// boards (1.9 and 1.4); `layout-kicad-board` grows it while the parts
    /// do not fit or the first route leaves connections open.
    #[serde(default)]
    pub area_factor: Option<f64>,
    /// Width over height when sizing automatically (default 1.5).
    #[serde(default)]
    pub aspect: Option<f64>,
    /// When sizing automatically, `layout-kicad-board` goes on shrinking the
    /// board by a fifth while a quick route still completes: the smallest
    /// board that routes.
    #[serde(default)]
    pub shrink: bool,
    /// Top-left corner in board coordinates. Default: the old outline's
    /// top-left corner, or, without one, centred on the footprints. For a
    /// polygon, the top-left corner of its points' bounding box; for
    /// pieces, of all pieces' (a piece's own `x`/`y` are its corner's
    /// offset within the set, default 0).
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    /// A polygon instead of a rectangle: its corners in millimetres,
    /// closed implicitly, moved so that their bounding box's top-left
    /// corner lies at `x`, `y`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polygon: Option<Vec<[f64; 2]>>,
    /// Every corner of the rectangle or polygon rounded with this radius
    /// (less where a side is too short for it), as lines and arcs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    /// Holes in the board (Edge.Cuts inner contours): copper keeps the
    /// edge clearance from them and no part's body enters them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cutouts: Vec<KiCadCutout>,
    /// Several pieces instead of one shape (a board with a breakaway part,
    /// boards side by side): each a rectangle or polygon with its own
    /// `x`/`y` offset, radius and cutouts. They may not overlap.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pieces: Vec<KiCadOutlineConstraint>,
}

/// A hole cut into an outline: a box (`x`, `y` ranges) or a polygon,
/// relative to its outline's (or piece's) top-left corner unless `origin`
/// is `absolute`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadCutout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polygon: Option<Vec<[f64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    /// `board` (default): relative to the outline's top-left corner;
    /// `absolute`: KiCad board coordinates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl KiCadOutlineConstraint {
    /// Sized from the parts: no size, no polygon, no pieces.
    pub fn automatic(&self) -> bool {
        self.width.is_none()
            && self.height.is_none()
            && self.polygon.is_none()
            && self.pieces.is_empty()
    }

    /// A polygon, or pieces one of which is: edge constraints then hold a
    /// part at the nearest side of the polygon facing their edge.
    pub fn polygonal(&self) -> bool {
        self.polygon.is_some() || self.pieces.iter().any(|piece| piece.polygon.is_some())
    }
}

/// Board area per unit of part area for an outline sized from the parts:
/// about what designers use (median 1.9 on two layers, 1.4 on more).
pub(crate) fn default_area_factor(pcb: &Expr) -> f64 {
    let copper = pcb
        .child("layers")
        .map(|layers| {
            layers
                .children()
                .iter()
                .filter(|layer| {
                    layer
                        .children()
                        .get(1)
                        .and_then(Expr::atom)
                        .is_some_and(|name| name.ends_with(".Cu"))
                })
                .count()
        })
        .unwrap_or(2);
    if copper > 2 { 1.5 } else { 2.0 }
}

/// Replaces the board's Edge.Cuts graphics by the constraint's rectangle.
/// `part_area` is the parts' total body area, for automatic sizing.
/// Returns the size used.
pub(crate) fn apply_outline(
    pcb: &mut Expr,
    outline: &KiCadOutlineConstraint,
    part_area: f64,
) -> Result<[f64; 2], String> {
    if outline.polygon.is_some()
        || !outline.pieces.is_empty()
        || outline.corner_radius.is_some_and(|radius| radius != 0.0)
        || !outline.cutouts.is_empty()
    {
        return apply_shaped_outline(pcb, outline, part_area);
    }
    let (width, height) = match (outline.width, outline.height) {
        (Some(width), Some(height)) => (width, height),
        (None, None) => {
            let area = part_area
                * outline
                    .area_factor
                    .unwrap_or_else(|| default_area_factor(pcb));
            let aspect = outline.aspect.unwrap_or(1.5).max(0.1);
            let mut width = (area * aspect).sqrt();
            // Both sides within 100 mm keeps the cheap fabs' flat price
            // (docs/cost.md): give up the aspect before that.
            if width > 100.0 && area <= 100.0 * 100.0 {
                width = 100.0;
            }
            // Whole half millimetres, rounded up.
            (
                (width * 2.0).ceil() / 2.0,
                (area / width * 2.0).ceil() / 2.0,
            )
        }
        _ => return Err("outline needs both width and height, or neither (automatic size)".into()),
    };
    if !(width > 0.0 && height > 0.0) {
        return Err("outline width and height must be positive".into());
    }
    let corner = new_corner(pcb, outline, width, height)?;
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    remove_edge_cuts(items);
    let rectangle = parse(&format!(
        "(gr_rect (start {} {}) (end {} {}) (stroke (width 0.05) (type solid)) (fill no) (layer \"Edge.Cuts\"))",
        corner[0],
        corner[1],
        corner[0] + width,
        corner[1] + height
    ))?;
    items.push(rectangle);
    Ok([width, height])
}

/// The top-left corner of a new outline of `width` by `height`: the
/// constraint's `x` and `y`, else the old outline's corner, else centred
/// on the footprints.
fn new_corner(
    pcb: &Expr,
    outline: &KiCadOutlineConstraint,
    width: f64,
    height: f64,
) -> Result<[f64; 2], String> {
    let old = outline::board_loops(pcb).ok().map(|loops| {
        loops
            .outline
            .iter()
            .fold([f64::INFINITY, f64::INFINITY], |corner, point| {
                [corner[0].min(point[0]), corner[1].min(point[1])]
            })
    });
    Ok(match (outline.x, outline.y, old) {
        (Some(x), Some(y), _) => [x, y],
        (x, y, Some(old)) if old[0].is_finite() => [x.unwrap_or(old[0]), y.unwrap_or(old[1])],
        (x, y, _) => {
            // Centred on the footprints.
            let origins: Vec<[f64; 3]> = pcb
                .children()
                .iter()
                .filter(|item| item.head() == Some("footprint"))
                .map(form_at)
                .collect::<Result<_, _>>()?;
            let count = origins.len().max(1) as f64;
            let center = origins.iter().fold([0.0, 0.0], |sum, at| {
                [sum[0] + at[0] / count, sum[1] + at[1] / count]
            });
            [
                x.unwrap_or(center[0] - width / 2.0),
                y.unwrap_or(center[1] - height / 2.0),
            ]
        }
    })
}

/// Drops the board's Edge.Cuts graphics.
fn remove_edge_cuts(items: &mut Vec<Expr>) {
    items.retain(|item| {
        !(matches!(
            item.head(),
            Some("gr_line" | "gr_rect" | "gr_arc" | "gr_circle" | "gr_poly" | "gr_curve")
        ) && form_atom(item, "layer", 1) == Some("Edge.Cuts"))
    });
}

/// The automatic size of an outline sized from the parts: `area_factor`
/// times their area at `aspect`, both sides within 100 mm when the area
/// allows, in whole half millimetres.
fn automatic_size(pcb: &Expr, outline: &KiCadOutlineConstraint, part_area: f64) -> (f64, f64) {
    let area = part_area
        * outline
            .area_factor
            .unwrap_or_else(|| default_area_factor(pcb));
    let aspect = outline.aspect.unwrap_or(1.5).max(0.1);
    let mut width = (area * aspect).sqrt();
    if width > 100.0 && area <= 100.0 * 100.0 {
        width = 100.0;
    }
    (
        (width * 2.0).ceil() / 2.0,
        (area / width * 2.0).ceil() / 2.0,
    )
}

/// A closed contour as written: its corners and the radius rounding
/// each of them.
#[derive(Clone, Debug, PartialEq)]
struct Contour {
    corners: Vec<[f64; 2]>,
    radius: f64,
}

/// One corner of a contour, rounded: the arc's start, middle and end (all
/// three the corner itself when it stays sharp).
fn fillet(
    previous: [f64; 2],
    corner: [f64; 2],
    next: [f64; 2],
    radius: f64,
) -> Option<[[f64; 2]; 3]> {
    let (la, lb) = (
        distance_squared(previous, corner).sqrt(),
        distance_squared(next, corner).sqrt(),
    );
    if radius <= 0.0 || la < 1.0e-9 || lb < 1.0e-9 {
        return None;
    }
    let u = [
        (previous[0] - corner[0]) / la,
        (previous[1] - corner[1]) / la,
    ];
    let v = [(next[0] - corner[0]) / lb, (next[1] - corner[1]) / lb];
    let angle = (u[0] * v[0] + u[1] * v[1]).clamp(-1.0, 1.0).acos();
    // A straight run or a spike stays as it is.
    if angle > std::f64::consts::PI - 1.0e-6 || angle < 1.0e-6 {
        return None;
    }
    let half = angle / 2.0;
    // Each side gives at most half its length to each of its corners.
    let tangent = (radius / half.tan()).min(la / 2.0).min(lb / 2.0);
    let radius = tangent * half.tan();
    if radius < 1.0e-6 {
        return None;
    }
    let start = [corner[0] + u[0] * tangent, corner[1] + u[1] * tangent];
    let end = [corner[0] + v[0] * tangent, corner[1] + v[1] * tangent];
    let w = [u[0] + v[0], u[1] + v[1]];
    let length = w[0].hypot(w[1]);
    let w = [w[0] / length, w[1] / length];
    let center_distance = radius / half.sin();
    let center = [
        corner[0] + w[0] * center_distance,
        corner[1] + w[1] * center_distance,
    ];
    let mid = [center[0] - w[0] * radius, center[1] - w[1] * radius];
    Some([start, mid, end])
}

/// A piece of a contour: `Ok` a line from point to point, `Err` an arc
/// (start, mid, end).
type ContourPiece = Result<[[f64; 2]; 2], [[f64; 2]; 3]>;

/// The contour's pieces in order: `Ok` lines from point to point, `Err`
/// arcs (start, mid, end).
fn contour_pieces(contour: &Contour) -> Vec<ContourPiece> {
    let corners = &contour.corners;
    let count = corners.len();
    let rounded: Vec<Option<[[f64; 2]; 3]>> = (0..count)
        .map(|index| {
            fillet(
                corners[(index + count - 1) % count],
                corners[index],
                corners[(index + 1) % count],
                contour.radius,
            )
        })
        .collect();
    let mut pieces = Vec::new();
    for index in 0..count {
        let end_here = rounded[index].map_or(corners[index], |arc| arc[2]);
        if let Some(arc) = rounded[index] {
            pieces.push(Err(arc));
        }
        let next = (index + 1) % count;
        let start_next = rounded[next].map_or(corners[next], |arc| arc[0]);
        if distance_squared(end_here, start_next).sqrt() > 1.0e-6 {
            pieces.push(Ok([end_here, start_next]));
        }
    }
    pieces
}

/// The contour as a polygon, arcs flattened as the outline reader does.
fn contour_points(contour: &Contour) -> Vec<[f64; 2]> {
    let mut points: Vec<[f64; 2]> = Vec::new();
    for piece in contour_pieces(contour) {
        let run = match piece {
            Ok(line) => line.to_vec(),
            Err(arc) => outline::arc_points(arc[0], arc[1], arc[2]),
        };
        for point in run {
            if points
                .last()
                .is_none_or(|last| distance_squared(*last, point) > 1.0e-18)
            {
                points.push(point);
            }
        }
    }
    if points.len() > 1 && distance_squared(points[0], *points.last().unwrap()) < 1.0e-18 {
        points.pop();
    }
    points
}

/// A coordinate as written: rounded to a nanometre, KiCad's unit.
fn coordinate(value: f64) -> String {
    let rounded = (value * 1.0e6).round() / 1.0e6;
    format!("{}", if rounded == 0.0 { 0.0 } else { rounded })
}

/// The Edge.Cuts graphics of a contour: lines and arcs.
fn contour_items(contour: &Contour) -> Result<Vec<Expr>, String> {
    let xy = |point: [f64; 2]| format!("{} {}", coordinate(point[0]), coordinate(point[1]));
    contour_pieces(contour)
        .into_iter()
        .map(|piece| {
            parse(&match piece {
                Ok([start, end]) => format!(
                    "(gr_line (start {}) (end {}) (stroke (width 0.05) (type solid)) (layer \"Edge.Cuts\"))",
                    xy(start),
                    xy(end)
                ),
                Err([start, mid, end]) => format!(
                    "(gr_arc (start {}) (mid {}) (end {}) (stroke (width 0.05) (type solid)) (layer \"Edge.Cuts\"))",
                    xy(start),
                    xy(mid),
                    xy(end)
                ),
            })
        })
        .collect()
}

pub(super) fn signed_area(points: &[[f64; 2]]) -> f64 {
    (0..points.len())
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0
}

/// Whether segments ab and cd cross or touch.
fn segments_meet(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let cross = |o: [f64; 2], p: [f64; 2], q: [f64; 2]| {
        (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0])
    };
    let on = |o: [f64; 2], p: [f64; 2], q: [f64; 2]| {
        q[0] >= o[0].min(p[0]) - 1.0e-9
            && q[0] <= o[0].max(p[0]) + 1.0e-9
            && q[1] >= o[1].min(p[1]) - 1.0e-9
            && q[1] <= o[1].max(p[1]) + 1.0e-9
    };
    let (d1, d2, d3, d4) = (
        cross(c, d, a),
        cross(c, d, b),
        cross(a, b, c),
        cross(a, b, d),
    );
    let eps = 1.0e-12;
    if ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps))
        && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
    {
        return true;
    }
    (d1.abs() <= eps && on(c, d, a))
        || (d2.abs() <= eps && on(c, d, b))
        || (d3.abs() <= eps && on(a, b, c))
        || (d4.abs() <= eps && on(a, b, d))
}

/// A polygon's corners checked: finite, three at least (a closing point
/// repeating the first is dropped), an area, and no side crossing another.
fn checked_polygon(points: &[[f64; 2]], what: &str) -> Result<Vec<[f64; 2]>, String> {
    let mut points = points.to_vec();
    if points.len() > 1 && distance_squared(points[0], *points.last().unwrap()) < 1.0e-18 {
        points.pop();
    }
    if points
        .iter()
        .any(|point| !point[0].is_finite() || !point[1].is_finite())
    {
        return Err(format!("{what}: coordinates must be numbers"));
    }
    points.dedup_by(|a, b| distance_squared(*a, *b) < 1.0e-18);
    if points.len() < 3 {
        return Err(format!("{what} needs three corners at least"));
    }
    if signed_area(&points).abs() < 1.0e-9 {
        return Err(format!("{what} has no area"));
    }
    let count = points.len();
    for i in 0..count {
        for j in i + 1..count {
            // Neighbouring sides share a corner.
            if j == i + 1 || (i == 0 && j == count - 1) {
                continue;
            }
            if segments_meet(
                points[i],
                points[(i + 1) % count],
                points[j],
                points[(j + 1) % count],
            ) {
                return Err(format!(
                    "{what} crosses itself: the side from {:?} to {:?} meets the side from {:?} to {:?}",
                    points[i],
                    points[(i + 1) % count],
                    points[j],
                    points[(j + 1) % count]
                ));
            }
        }
    }
    Ok(points)
}

fn radius_of(radius: Option<f64>, what: &str) -> Result<f64, String> {
    let radius = radius.unwrap_or(0.0);
    if !(radius >= 0.0 && radius.is_finite()) {
        return Err(format!("{what}: corner_radius must be zero or more"));
    }
    Ok(radius)
}

fn bounding_box(points: &[[f64; 2]]) -> [f64; 4] {
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

/// Whether two closed polygons share any point (a side meets a side, or
/// one lies inside the other).
fn polygons_meet(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    let (ba, bb) = (bounding_box(a), bounding_box(b));
    if ba[2] < bb[0] || bb[2] < ba[0] || ba[3] < bb[1] || bb[3] < ba[1] {
        return false;
    }
    for i in 0..a.len() {
        for j in 0..b.len() {
            if segments_meet(a[i], a[(i + 1) % a.len()], b[j], b[(j + 1) % b.len()]) {
                return true;
            }
        }
    }
    point_in_polygon(a[0], b) || point_in_polygon(b[0], a)
}

/// One piece of an outline as written: its contour (in the set's own
/// frame until placed) and its cutouts' (board coordinates where
/// `origin` is absolute, else relative to the piece's corner).
struct ShapedPiece {
    contour: Contour,
    /// The corners' bounding box in the set's frame.
    bounds: [f64; 4],
    cutouts: Vec<(Contour, bool)>,
}

fn shaped_piece(
    piece: &KiCadOutlineConstraint,
    what: &str,
    nested: bool,
    pcb: &Expr,
    part_area: f64,
) -> Result<ShapedPiece, String> {
    if nested && !piece.pieces.is_empty() {
        return Err(format!("{what}: a piece cannot have pieces"));
    }
    if (piece.shrink || piece.area_factor.is_some() || piece.aspect.is_some())
        && (nested || !piece.automatic())
    {
        return Err(format!(
            "{what}: area_factor, aspect and shrink are for an outline sized from the parts (no width, height or polygon)"
        ));
    }
    let radius = radius_of(piece.corner_radius, what)?;
    let corners = match (&piece.polygon, piece.width, piece.height) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            return Err(format!(
                "{what}: give a polygon or width and height, not both"
            ));
        }
        (Some(polygon), None, None) => checked_polygon(polygon, &format!("{what} polygon"))?,
        (None, Some(width), Some(height)) => {
            if !(width > 0.0 && height > 0.0) {
                return Err(format!("{what}: width and height must be positive"));
            }
            vec![[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]]
        }
        (None, None, None) if !nested => {
            if !piece.cutouts.is_empty() {
                return Err(format!(
                    "{what}: cutouts need a sized outline (width and height, or a polygon)"
                ));
            }
            let (width, height) = automatic_size(pcb, piece, part_area);
            vec![[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]]
        }
        (None, None, None) => return Err(format!("{what} needs width and height or a polygon")),
        _ => return Err("outline needs both width and height, or neither (automatic size)".into()),
    };
    // The corners' box moves to the piece's own offset.
    let own = bounding_box(&corners);
    let offset = if nested {
        [piece.x.unwrap_or(0.0), piece.y.unwrap_or(0.0)]
    } else {
        [0.0, 0.0]
    };
    let corners: Vec<[f64; 2]> = corners
        .iter()
        .map(|p| [p[0] - own[0] + offset[0], p[1] - own[1] + offset[1]])
        .collect();
    let mut cutouts = Vec::new();
    for (number, cutout) in piece.cutouts.iter().enumerate() {
        let name = format!("{what} cutout {}", number + 1);
        let radius = radius_of(cutout.corner_radius, &name)?;
        let absolute = match cutout.origin.as_deref() {
            None | Some("board") => false,
            Some("absolute") => true,
            Some(other) => {
                return Err(format!(
                    "unknown cutout origin {other:?} (board or absolute)"
                ));
            }
        };
        let corners = match (&cutout.polygon, cutout.x, cutout.y) {
            (Some(polygon), None, None) => checked_polygon(polygon, &name)?,
            (None, Some(x), Some(y)) => {
                let (x0, x1, y0, y1) = (
                    x[0].min(x[1]),
                    x[0].max(x[1]),
                    y[0].min(y[1]),
                    y[0].max(y[1]),
                );
                if !(x1 > x0 && y1 > y0) {
                    return Err(format!("{name} is empty"));
                }
                vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
            }
            _ => return Err(format!("{name}: give x and y ranges or a polygon")),
        };
        cutouts.push((Contour { corners, radius }, absolute));
    }
    Ok(ShapedPiece {
        bounds: bounding_box(&corners),
        contour: Contour { corners, radius },
        cutouts,
    })
}

/// Writes an outline that is not a plain rectangle (a polygon, rounded
/// corners, cutouts, several pieces) as Edge.Cuts lines and arcs,
/// replacing the board's own. Returns the size of its bounding box.
fn apply_shaped_outline(
    pcb: &mut Expr,
    outline: &KiCadOutlineConstraint,
    part_area: f64,
) -> Result<[f64; 2], String> {
    let pieces = shaped_pieces(pcb, outline, part_area)?;
    let set = pieces.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, piece| {
            [
                b[0].min(piece.bounds[0]),
                b[1].min(piece.bounds[1]),
                b[2].max(piece.bounds[2]),
                b[3].max(piece.bounds[3]),
            ]
        },
    );
    let (width, height) = (set[2] - set[0], set[3] - set[1]);
    let corner = new_corner(pcb, outline, width, height)?;
    let shift = [corner[0] - set[0], corner[1] - set[1]];
    let contours = placed_contours(&pieces, shift)?;
    let mut graphics = Vec::new();
    for contour in &contours {
        graphics.extend(contour_items(contour)?);
    }
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    remove_edge_cuts(items);
    items.extend(graphics);
    Ok([width, height])
}

/// The outline's pieces, checked: the outline itself, or each of its
/// `pieces` (the outline then gives only `x` and `y`).
fn shaped_pieces(
    pcb: &Expr,
    outline: &KiCadOutlineConstraint,
    part_area: f64,
) -> Result<Vec<ShapedPiece>, String> {
    let mut pieces = Vec::new();
    if outline.pieces.is_empty() {
        pieces.push(shaped_piece(outline, "outline", false, pcb, part_area)?);
    } else {
        if outline.polygon.is_some()
            || outline.width.is_some()
            || outline.height.is_some()
            || outline.corner_radius.is_some()
            || !outline.cutouts.is_empty()
            || outline.shrink
            || outline.area_factor.is_some()
            || outline.aspect.is_some()
        {
            return Err("outline: with pieces, each piece carries its own shape (width and height or polygon, corner_radius, cutouts); the outline only x and y".into());
        }
        for (number, piece) in outline.pieces.iter().enumerate() {
            pieces.push(shaped_piece(
                piece,
                &format!("outline piece {}", number + 1),
                true,
                pcb,
                part_area,
            )?);
        }
    }
    Ok(pieces)
}

/// `points` moved by `by`.
fn moved(points: &[[f64; 2]], by: [f64; 2]) -> Vec<[f64; 2]> {
    points
        .iter()
        .map(|p| [p[0] + by[0], p[1] + by[1]])
        .collect()
}

/// The pieces' contours and their cutouts' on the board, the pieces moved
/// by `shift`: each piece followed by its cutouts. Refuses pieces that
/// meet, a cutout not inside its piece and cutouts that meet.
fn placed_contours(pieces: &[ShapedPiece], shift: [f64; 2]) -> Result<Vec<Contour>, String> {
    let mut contours: Vec<Contour> = Vec::new();
    let mut flat_pieces: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut flat_cutouts: Vec<(Vec<[f64; 2]>, String)> = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        let name = if pieces.len() > 1 {
            format!("outline piece {}", index + 1)
        } else {
            "outline".to_string()
        };
        let contour = Contour {
            corners: moved(&piece.contour.corners, shift),
            radius: piece.contour.radius,
        };
        let flat = contour_points(&contour);
        for (other, earlier) in flat_pieces.iter().enumerate() {
            if polygons_meet(&flat, earlier) {
                return Err(format!(
                    "{name} meets outline piece {}: pieces may not overlap or touch",
                    other + 1
                ));
            }
        }
        contours.push(contour);
        // The piece's corner: where its cutouts' coordinates start.
        let piece_corner = [piece.bounds[0] + shift[0], piece.bounds[1] + shift[1]];
        for (number, (cutout, absolute)) in piece.cutouts.iter().enumerate() {
            let cutout_name = format!("{name} cutout {}", number + 1);
            let by = if *absolute { [0.0, 0.0] } else { piece_corner };
            let cutout = Contour {
                corners: moved(&cutout.corners, by),
                radius: cutout.radius,
            };
            let hole = contour_points(&cutout);
            let crosses = (0..hole.len()).any(|i| {
                (0..flat.len()).any(|j| {
                    segments_meet(
                        hole[i],
                        hole[(i + 1) % hole.len()],
                        flat[j],
                        flat[(j + 1) % flat.len()],
                    )
                })
            });
            if crosses || !point_in_polygon(hole[0], &flat) {
                return Err(format!(
                    "{cutout_name} does not lie inside its outline (coordinates are relative to the outline's top-left corner unless origin is absolute)"
                ));
            }
            for (earlier, earlier_name) in &flat_cutouts {
                if polygons_meet(&hole, earlier) {
                    return Err(format!(
                        "{cutout_name} meets {earlier_name}: cutouts may not overlap or touch"
                    ));
                }
            }
            flat_cutouts.push((hole, cutout_name));
            contours.push(cutout);
        }
        flat_pieces.push(flat);
    }
    Ok(contours)
}

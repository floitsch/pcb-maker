// Copyright (C) 2026 Toit contributors.

//! KiCad's `connection_width` check and its repair.
//!
//! KiCad's DRC (drc_test_provider_connection_width.cpp, 10.0.7) unites per
//! net and copper layer every zone fill, track, flashed via and flashed pad,
//! and reports a "neck" where two vertices of the outline that are not
//! neighbours lie closer than the board's minimum connection width (less
//! twice its epsilon: DRC epsilon, max error and the fill's extra margin)
//! with copper bulging past that width on both sides. The minimum comes
//! from the board setup's `min_connection` (0 by default: no check) or a
//! custom rule.
//!
//! On our boards the necks are KiCad's own: its refill (which also
//! rebuilds the pads' teardrops) leaves narrow joints in hatched zones that
//! its minimum-width pruning does not reach (the thermal rings, the border
//! KiCad restores around another net's teardrop, a pad's corner against a
//! hatch hole). The router's fill model knows neither the teardrops nor
//! these unions, so the repair asks KiCad: when the result's DRC reports
//! necks as errors, the board is refilled and saved by kicad-cli, the
//! net's copper is drawn on a fine raster, the thin parts that join two
//! wider parts are found (an opening by the minimum width, as KiCad's
//! filler prunes), and a short track of the net, as wide as the minimum,
//! is laid along each where it keeps every clearance. The board is checked
//! again and kept only if the necks went down and nothing else got worse.
//! When a check fails (kicad-cli crashes or times out), the board and its
//! reports go back to what they were before the repair: the result
//! directory never lacks the report of the board it holds.

use super::*;
use crate::board_router::{KiCadBoardRouterConfig, LayerTable};
use pcb_router as core;

/// Raster pitch, mm.
const PIXEL: f64 = 0.004;
/// Tile side in pixels, and the overlap kept around a tile's core.
const TILE: usize = 1024;
const MARGIN: usize = 80;
/// How far a repair track reaches past where the neck meets wide copper.
const OVERLAP: f64 = 0.04;
/// Extra clearance the repair keeps over the rules (rounding, arcs).
const SLACK: f64 = 0.003;

#[derive(Clone, Debug, Default, Serialize)]
pub struct NeckRepair {
    /// KiCad's connection_width errors before and after.
    pub before: usize,
    pub after: usize,
    /// Necks the raster found on the nets and layers KiCad named.
    pub found: usize,
    /// Repair tracks that kept every clearance (added), and whether the
    /// board with them was kept.
    pub tracks: usize,
    pub kept: bool,
    /// The kept repair tracks' total length, mm.
    pub track_mm: f64,
    pub seconds: f64,
    /// KiCad's fills of a scratch copy (a save with the zones refilled):
    /// each counts a check's work in a layout, as the DRC checks do.
    pub kicad_fills: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One connection_width error of KiCad's report.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Finding {
    pub net: String,
    pub layer: String,
    pub width: f64,
}

/// The connection_width errors in a DRC report: net (from the first item
/// that names one), layer and width (from the description).
pub(crate) fn findings(report: &serde_json::Value) -> Vec<Finding> {
    let mut result = Vec::new();
    for violation in report["violations"].as_array().into_iter().flatten() {
        if violation["type"].as_str() != Some("connection_width")
            || violation["severity"].as_str() != Some("error")
        {
            continue;
        }
        let description = violation["description"].as_str().unwrap_or("");
        let layer = description
            .rsplit_once('(')
            .and_then(|(_, tail)| tail.strip_suffix(')'))
            .unwrap_or("")
            .to_string();
        let width = description
            .split("actual ")
            .nth(1)
            .and_then(|tail| tail.split(' ').next())
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        let net = violation["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["description"].as_str())
            .find_map(|text| Some(text.split_once('[')?.1.split_once(']')?.0.to_string()))
            .unwrap_or_default();
        result.push(Finding { net, layer, width });
    }
    result
}

/// A thin part of a net's copper that joins wider parts: where it is
/// (its narrowest pixel) and the track that would cover it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Neck {
    pub at: core::Point,
    /// The passage's width there (twice the room to its edges), mm.
    pub width: f64,
    pub start: core::Point,
    pub end: core::Point,
    /// Where the track bends between `start` and `end` (a curved passage,
    /// a slot of empty space): none for a straight track.
    pub bends: Vec<core::Point>,
    /// A slot's repair: tracks across it from wall to wall, each laid
    /// where it keeps its clearances (`slots`); none for a passage of
    /// copper, whose repair is the track `start` to `end`.
    pub bridges: Vec<Vec<core::Point>>,
    /// A hole the copper closes all round (a slot's): its bridges are
    /// tried after the other repairs, holes being many and seldom KiCad's.
    pub hole: bool,
}

impl Neck {
    /// The repair track's points, `start` to `end`.
    fn path(&self) -> Vec<core::Point> {
        let mut path = vec![self.start];
        path.extend(self.bends.iter().copied());
        path.push(self.end);
        path
    }
}

/// The copper of one net on one layer: filled outlines (each a simple
/// polygon, KiCad's fractured fill) and shapes (pads, tracks, vias).
#[derive(Clone, Debug, Default)]
pub(crate) struct Copper {
    pub polygons: Vec<Vec<core::Point>>,
    pub shapes: Vec<core::Shape>,
}

impl Copper {
    fn bounds(&self) -> Option<core::geometry::Aabb> {
        let mut boxes = self
            .polygons
            .iter()
            .map(|polygon| {
                core::Shape::Polygon {
                    points: polygon.clone(),
                }
                .aabb()
            })
            .chain(self.shapes.iter().map(core::Shape::aabb));
        let first = boxes.next()?;
        Some(boxes.fold(first, core::geometry::Aabb::union))
    }
}

/// Squared Euclidean distance transform (Felzenszwalb and Huttenlocher) of
/// a grid in place: `grid` holds 0 at the targets and a large value
/// elsewhere; afterwards the squared pixel distance to the nearest target.
fn distance_transform(grid: &mut [f64], width: usize, height: usize) {
    let size = width.max(height);
    let mut f = vec![0.0; size];
    let mut d = vec![0.0; size];
    let mut v = vec![0usize; size];
    let mut z = vec![0.0; size + 1];
    let mut pass = |f: &[f64], n: usize, d: &mut [f64]| {
        let intersect = |q: usize, p: usize| {
            ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64))
        };
        let mut k = 0usize;
        v[0] = 0;
        z[0] = f64::NEG_INFINITY;
        z[1] = f64::INFINITY;
        for q in 1..n {
            let mut s = intersect(q, v[k]);
            while s <= z[k] {
                k -= 1;
                s = intersect(q, v[k]);
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f64::INFINITY;
        }
        k = 0;
        for (q, value) in d.iter_mut().enumerate().take(n) {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            let p = v[k];
            *value = (q as f64 - p as f64).powi(2) + f[p];
        }
    };
    for x in 0..width {
        for y in 0..height {
            f[y] = grid[y * width + x];
        }
        pass(&f, height, &mut d);
        for y in 0..height {
            grid[y * width + x] = d[y];
        }
    }
    for y in 0..height {
        f[..width].copy_from_slice(&grid[y * width..(y + 1) * width]);
        pass(&f, width, &mut d);
        grid[y * width..(y + 1) * width].copy_from_slice(&d[..width]);
    }
}

const FAR: f64 = 1.0e12;

/// Draws the copper on a tile whose pixel (0, 0) has its centre at
/// `origin`.
fn rasterize(copper: &Copper, origin: core::Point, width: usize, height: usize) -> Vec<bool> {
    let mut filled = vec![false; width * height];
    let top = origin[1] - PIXEL;
    let bottom = origin[1] + height as f64 * PIXEL;
    let left = origin[0] - PIXEL;
    let right = origin[0] + width as f64 * PIXEL;
    // Even-odd per outline: KiCad's fill outlines are simple polygons
    // whose slits cancel; outlines are OR-ed.
    let mut crossings: Vec<f64> = Vec::new();
    let mut outline_edges: Vec<Vec<(core::Point, core::Point)>> = Vec::new();
    for polygon in &copper.polygons {
        let mut list = Vec::new();
        for k in 0..polygon.len() {
            let (a, b) = (polygon[k], polygon[(k + 1) % polygon.len()]);
            if a[1].max(b[1]) < top || a[1].min(b[1]) > bottom {
                continue;
            }
            list.push((a, b));
        }
        if list.is_empty() {
            continue;
        }
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for polygon_point in polygon {
            low = low.min(polygon_point[0]);
            high = high.max(polygon_point[0]);
        }
        if high < left || low > right {
            continue;
        }
        outline_edges.push(list);
    }
    for list in &outline_edges {
        for row in 0..height {
            let y = origin[1] + row as f64 * PIXEL;
            crossings.clear();
            for &(a, b) in list {
                if (a[1] <= y) != (b[1] <= y) {
                    crossings.push(a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                }
            }
            if crossings.len() < 2 {
                continue;
            }
            crossings.sort_by(f64::total_cmp);
            for pair in crossings.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                let first = ((pair[0] - origin[0]) / PIXEL).ceil().max(0.0) as i64;
                let last = ((pair[1] - origin[0]) / PIXEL)
                    .floor()
                    .min(width as f64 - 1.0) as i64;
                for column in first..=last {
                    filled[row * width + column as usize] = true;
                }
            }
        }
    }
    for shape in &copper.shapes {
        let bounds = shape.aabb();
        if bounds.maximum[0] < left
            || bounds.minimum[0] > right
            || bounds.maximum[1] < top
            || bounds.minimum[1] > bottom
        {
            continue;
        }
        let x0 = (((bounds.minimum[0] - origin[0]) / PIXEL).floor().max(0.0)) as usize;
        let x1 = (((bounds.maximum[0] - origin[0]) / PIXEL)
            .ceil()
            .min(width as f64 - 1.0)) as usize;
        let y0 = (((bounds.minimum[1] - origin[1]) / PIXEL).floor().max(0.0)) as usize;
        let y1 = (((bounds.maximum[1] - origin[1]) / PIXEL)
            .ceil()
            .min(height as f64 - 1.0)) as usize;
        for row in y0..=y1 {
            for column in x0..=x1 {
                let index = row * width + column;
                if !filled[index]
                    && shape.contains([
                        origin[0] + column as f64 * PIXEL,
                        origin[1] + row as f64 * PIXEL,
                    ])
                {
                    filled[index] = true;
                }
            }
        }
    }
    filled
}

/// The necks of one tile. KiCad's check pairs vertices of one outline
/// closer than the limit with the outline bulging past it on both sides:
/// a narrow passage of copper between wider copper (repaired here), and
/// as well a narrow gap between two parts of the net's copper (a slot
/// between two lobes of a fill, a track of the net beside its fill), not
/// repaired.
#[allow(clippy::too_many_arguments)]
fn tile_necks(
    filled: &[bool],
    width: usize,
    height: usize,
    origin: core::Point,
    limit: f64,
    margin: usize,
    anchors: &[core::Point],
) -> Vec<Neck> {
    // The gaps as the saddles of the space between the copper were tried
    // and left: mostly gaps KiCad does not report (USB_Keypad: 20
    // candidates for 3 gaps, 7 to 6 necks against 7 to 3 without them).
    // The slots below are those of one piece of copper only.
    let mut necks = saddles(filled, width, height, origin, limit, margin);
    necks.extend(slots(filled, width, height, origin, limit, margin, anchors));
    necks
}

/// The slots of a net's copper: empty space narrower than `limit` between
/// two parts of one piece of copper (two lobes of a thermal ring at a
/// track, a sliver of a hatch hole, a teardrop's border), the gaps of
/// KiCad's check (`necks.py`'s "gap": a pair of vertices of one outline
/// across empty space; USB_Keypad's and WLED_Matrix's last necks,
/// 2026-10-27), which the saddles of the copper do not see. KiCad's pairs
/// hang on vertices a few nanometres apart where its union joins arcs, so
/// the raster cannot say which slot KiCad flags; it finds those that can
/// be, the empty pixels a closing by a disc of the limit's diameter fills,
/// each connected piece that is one of:
/// - a passage between two parts of the empty space (a pinch);
/// - a slit (twice the limit long, three times as long as wide);
/// - a bay or a hole as long and as wide as the limit (KiCad's outline
///   bulges past the limit in x and in y on the slot's side);
///
/// between two parts of one piece of copper (KiCad pairs vertices of one
/// outline only) that holds an item of the net (a piece without one is an
/// island of the fill, which KiCad leaves out of its connections until a
/// track touches it: USB_Keypad's trial, 2026-10-27, 2 to 11
/// unconnected). The round of a concave corner and the tip of a hatch hole
/// (one stretch of shore, narrower than the limit) are none. Each comes
/// with bridges across it, wall to wall (KiCad joins a track to a fill by
/// its ends), until every pixel lies under one.
#[allow(clippy::too_many_arguments)]
fn slots(
    filled: &[bool],
    width: usize,
    height: usize,
    origin: core::Point,
    limit: f64,
    margin: usize,
    anchors: &[core::Point],
) -> Vec<Neck> {
    let radius = limit / 2.0 / PIXEL;
    let reach = radius * radius;
    // The copper grown by the radius, then shrunk by it again.
    let mut to_copper: Vec<f64> = filled
        .iter()
        .map(|&copper| if copper { 0.0 } else { FAR })
        .collect();
    distance_transform(&mut to_copper, width, height);
    let grown: Vec<bool> = to_copper.iter().map(|&squared| squared <= reach).collect();
    let mut to_outside: Vec<f64> = grown
        .iter()
        .map(|&inside| if inside { FAR } else { 0.0 })
        .collect();
    distance_transform(&mut to_outside, width, height);
    let slot: Vec<bool> = (0..filled.len())
        .map(|index| !filled[index] && grown[index] && to_outside[index] > reach)
        .collect();
    let neighbours = |index: usize| {
        let (x, y) = ((index % width) as i64, (index / width) as i64);
        (-1..=1i64)
            .flat_map(move |dy| (-1..=1i64).map(move |dx| (dx, dy)))
            .filter(|&step| step != (0, 0))
            .filter_map(move |(dx, dy)| {
                let (nx, ny) = (x + dx, y + dy);
                (nx >= 0 && ny >= 0 && nx < width as i64 && ny < height as i64)
                    .then(|| ny as usize * width + nx as usize)
            })
    };
    // Numbers the 8-connected parts of the pixels `member` holds.
    const NONE: u32 = u32::MAX;
    let label = |member: &dyn Fn(usize) -> bool| -> (Vec<u32>, u32) {
        let mut labels = vec![NONE; filled.len()];
        let mut count = 0u32;
        for first in 0..filled.len() {
            if !member(first) || labels[first] != NONE {
                continue;
            }
            labels[first] = count;
            let mut stack = vec![first];
            while let Some(index) = stack.pop() {
                for neighbour in neighbours(index) {
                    if member(neighbour) && labels[neighbour] == NONE {
                        labels[neighbour] = count;
                        stack.push(neighbour);
                    }
                }
            }
            count += 1;
        }
        (labels, count)
    };
    // The empty regions beside the slots, and the pieces of copper.
    let (region, _) = label(&|index| !filled[index] && !slot[index]);
    let (piece_of, pieces) = label(&|index| filled[index]);
    let pixel_at = |point: core::Point| -> Option<usize> {
        let (x, y) = (
            ((point[0] - origin[0]) / PIXEL).round(),
            ((point[1] - origin[1]) / PIXEL).round(),
        );
        (x >= 0.0 && y >= 0.0 && (x as usize) < width && (y as usize) < height)
            .then(|| y as usize * width + x as usize)
    };
    let mut anchored = vec![false; pieces as usize];
    for &point in anchors {
        if let Some(index) = pixel_at(point)
            && piece_of[index] != NONE
        {
            anchored[piece_of[index] as usize] = true;
        }
    }
    let point_of = |index: usize| {
        [
            origin[0] + (index % width) as f64 * PIXEL,
            origin[1] + (index / width) as f64 * PIXEL,
        ]
    };
    // A bridge covers a pixel within this of its line (half the minimum
    // width the repair lays, which exceeds half the limit).
    let cover = limit / 2.0 + 0.001;
    const MAX_BRIDGES: usize = 16;
    let mut seen = vec![false; filled.len()];
    let mut necks = Vec::new();
    for first in 0..slot.len() {
        if !slot[first] || seen[first] {
            continue;
        }
        let mut piece = vec![first];
        seen[first] = true;
        let (mut regions, mut walls, mut shore) = (Vec::new(), Vec::new(), Vec::new());
        let mut next = 0;
        while next < piece.len() {
            let index = piece[next];
            next += 1;
            for neighbour in neighbours(index) {
                if slot[neighbour] {
                    if !seen[neighbour] {
                        seen[neighbour] = true;
                        piece.push(neighbour);
                    }
                } else if region[neighbour] != NONE {
                    if !regions.contains(&region[neighbour]) {
                        regions.push(region[neighbour]);
                    }
                    shore.push(neighbour);
                } else if piece_of[neighbour] != NONE && !walls.contains(&piece_of[neighbour]) {
                    walls.push(piece_of[neighbour]);
                }
            }
        }
        let [wall] = walls[..] else {
            continue;
        };
        if !anchored[wall as usize] {
            continue;
        }
        let points: Vec<core::Point> = piece.iter().map(|&index| point_of(index)).collect();
        let count = points.len() as f64;
        let centre = [
            points.iter().map(|point| point[0]).sum::<f64>() / count,
            points.iter().map(|point| point[1]).sum::<f64>() / count,
        ];
        let (cx, cy) = (
            ((centre[0] - origin[0]) / PIXEL).round() as i64,
            ((centre[1] - origin[1]) / PIXEL).round() as i64,
        );
        if cx < margin as i64
            || cy < margin as i64
            || cx >= (width - margin) as i64
            || cy >= (height - margin) as i64
        {
            continue;
        }
        // The stretches of shore (where the slot meets the space beyond).
        shore.sort_unstable();
        shore.dedup();
        let mut done = vec![false; shore.len()];
        let mut stretches = 0;
        for start in 0..shore.len() {
            if done[start] {
                continue;
            }
            stretches += 1;
            done[start] = true;
            let mut stack = vec![shore[start]];
            while let Some(index) = stack.pop() {
                for neighbour in neighbours(index) {
                    if let Ok(position) = shore.binary_search(&neighbour)
                        && !done[position]
                    {
                        done[position] = true;
                        stack.push(neighbour);
                    }
                }
            }
        }
        // The piece's length and breadth along its principal axis.
        let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
        for point in &points {
            let (dx, dy) = (point[0] - centre[0], point[1] - centre[1]);
            sxx += dx * dx;
            sxy += dx * dy;
            syy += dy * dy;
        }
        let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
        let axis = [angle.cos(), angle.sin()];
        let extent = |direction: [f64; 2]| {
            let (low, high) =
                points
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                        let t = (point[0] - centre[0]) * direction[0]
                            + (point[1] - centre[1]) * direction[1];
                        (low.min(t), high.max(t))
                    });
            high - low
        };
        let (length, breadth) = (extent(axis), extent([-axis[1], axis[0]]));
        let passage = regions.len() >= 2 || stretches >= 2;
        let slit = length >= 2.0 * limit && length >= 3.0 * breadth.max(PIXEL);
        let wide = length >= limit && breadth >= limit;
        if !passage && !slit && !wide {
            continue;
        }
        // Bridges: through the first pixel not yet under one, along the
        // way the space there widens (from its nearer wall to the other).
        let in_wall =
            |point: core::Point| pixel_at(point).is_some_and(|index| piece_of[index] == wall);
        let half_width = |index: usize| to_copper[index].sqrt();
        let mut bridges: Vec<Vec<core::Point>> = Vec::new();
        let mut covered = vec![false; piece.len()];
        for position in 0..piece.len() {
            if covered[position] || bridges.len() >= MAX_BRIDGES {
                continue;
            }
            covered[position] = true;
            let index = piece[position];
            let (x, y) = (index % width, index / width);
            let at = |dx: i64, dy: i64| {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                    half_width(index)
                } else {
                    half_width(ny as usize * width + nx as usize)
                }
            };
            let gradient = [at(1, 0) - at(-1, 0), at(0, 1) - at(0, -1)];
            let norm = gradient[0].hypot(gradient[1]);
            let direction = if norm > 1.0e-9 {
                [gradient[0] / norm, gradient[1] / norm]
            } else {
                [-axis[1], axis[0]]
            };
            let from = point_of(index);
            let mut ends = Vec::new();
            for sign in [-1.0, 1.0] {
                let mut travelled = 0.0;
                while travelled < 1.5 * limit {
                    travelled += PIXEL;
                    let point = [
                        from[0] + sign * direction[0] * travelled,
                        from[1] + sign * direction[1] * travelled,
                    ];
                    if in_wall(point) {
                        ends.push([
                            point[0] + sign * direction[0] * OVERLAP,
                            point[1] + sign * direction[1] * OVERLAP,
                        ]);
                        break;
                    }
                }
            }
            let [a, b] = ends[..] else {
                continue;
            };
            for (other, point) in points.iter().enumerate() {
                if !covered[other] && core::geometry::point_segment_distance(*point, a, b) <= cover
                {
                    covered[other] = true;
                }
            }
            bridges.push(vec![a, b]);
        }
        let Some(first_bridge) = bridges.first().cloned() else {
            continue;
        };
        let narrowest = piece
            .iter()
            .map(|&index| half_width(index))
            .fold(0.0f64, f64::max);
        necks.push(Neck {
            at: centre,
            width: 2.0 * narrowest * PIXEL,
            start: first_bridge[0],
            end: first_bridge[1],
            bends: Vec::new(),
            bridges,
            hole: regions.is_empty(),
        });
    }
    necks
}

/// The saddles of `filled`'s distance map: pixels of a passage narrower
/// than `limit` that leads to room wider than it both ways. Pixels are
/// taken from the widest inward (by their distance to the edge); a pixel
/// whose wider neighbours form two arcs lies in such a passage. Only those
/// in the tile's core (`margin` pixels in from each edge) are returned,
/// each with a track along the passage into the wide room on both sides.
fn saddles(
    filled: &[bool],
    width: usize,
    height: usize,
    origin: core::Point,
    limit: f64,
    margin: usize,
) -> Vec<Neck> {
    let radius = limit / 2.0 / PIXEL;
    let mut inside: Vec<f64> = filled
        .iter()
        .map(|&copper| if copper { FAR } else { 0.0 })
        .collect();
    distance_transform(&mut inside, width, height);
    // Pixel distances run between centres: the edge lies half a pixel
    // nearer.
    let room: Vec<f64> = inside
        .iter()
        .map(|&squared| (squared.sqrt() - 0.5).max(0.0))
        .collect();
    let point_of = |index: usize| {
        [
            origin[0] + (index % width) as f64 * PIXEL,
            origin[1] + (index / width) as f64 * PIXEL,
        ]
    };
    let mut necks: Vec<Neck> = Vec::new();
    // The 8 neighbours in circular order.
    const RING: [(i64, i64); 8] = [
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
    ];
    // Taken in order of room, widest first (ties by index), a pixel finds
    // its wider neighbours taken already.
    let before = |neighbour: usize, index: usize| {
        filled[neighbour]
            && (room[neighbour] > room[index]
                || (room[neighbour] == room[index] && neighbour < index))
    };
    for index in 0..filled.len() {
        if !filled[index] {
            continue;
        }
        // A passage of a pixel or two is the raster's (copper meeting at a
        // corner within a few microns), not KiCad's.
        if room[index] >= radius || room[index] < 1.0 {
            continue;
        }
        let (x, y) = ((index % width) as i64, (index / width) as i64);
        if x < margin as i64
            || y < margin as i64
            || x >= (width - margin) as i64
            || y >= (height - margin) as i64
        {
            continue;
        }
        // The arcs of neighbours already taken (wider than this pixel): a
        // pixel on an edge or a tip has one, a pixel in a passage between
        // wider copper has two, one each way along it.
        let taken: Vec<bool> = RING
            .iter()
            .map(|&(dx, dy)| {
                let (nx, ny) = (x + dx, y + dy);
                nx >= 0
                    && ny >= 0
                    && nx < width as i64
                    && ny < height as i64
                    && before(ny as usize * width + nx as usize, index)
            })
            .collect();
        let Some(gap) = (0..8).find(|&k| !taken[k]) else {
            continue;
        };
        let mut arcs: Vec<Vec<usize>> = Vec::new();
        for step in 1..=8 {
            let k = (gap + step) % 8;
            if taken[k] {
                if step == 1 || !taken[(k + 7) % 8] {
                    arcs.push(Vec::new());
                }
                arcs.last_mut().unwrap().push(k);
            }
        }
        if arcs.len() < 2 {
            continue;
        }
        let here = point_of(index);
        if necks
            .iter()
            .any(|neck| core::geometry::distance(neck.at, here) < limit)
        {
            continue;
        }
        arcs.sort_by_key(|arc| std::cmp::Reverse(arc.len()));
        let mean = |arc: &[usize]| {
            let (mut sx, mut sy) = (0.0, 0.0);
            for &k in arc {
                sx += RING[k].0 as f64;
                sy += RING[k].1 as f64;
            }
            [sx / arc.len() as f64, sy / arc.len() as f64]
        };
        let (a, b) = (mean(&arcs[0]), mean(&arcs[1]));
        let mut direction = [b[0] - a[0], b[1] - a[1]];
        let length = (direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
        if length < 1.0e-9 {
            continue;
        }
        direction = [direction[0] / length, direction[1] / length];
        // Both ways the passage must lead to copper wider than the limit
        // (KiCad: the outline bulges past the limit on both sides) within
        // half a millimetre.
        let reach = |sign: f64| -> Option<f64> {
            let mut travelled = 0.0;
            while travelled < 0.5 {
                travelled += PIXEL;
                let point = [
                    here[0] + sign * direction[0] * travelled,
                    here[1] + sign * direction[1] * travelled,
                ];
                let (px, py) = (
                    ((point[0] - origin[0]) / PIXEL).round(),
                    ((point[1] - origin[1]) / PIXEL).round(),
                );
                if px < 0.0 || py < 0.0 || px >= width as f64 || py >= height as f64 {
                    return None;
                }
                let probe = py as usize * width + px as usize;
                if !filled[probe] {
                    return None;
                }
                if room[probe] >= radius {
                    return Some(travelled);
                }
            }
            None
        };
        if let (Some(back), Some(forth)) = (reach(-1.0), reach(1.0)) {
            necks.push(Neck {
                at: here,
                width: 2.0 * (room[index] + 0.5) * PIXEL,
                start: [
                    here[0] - direction[0] * (back + OVERLAP),
                    here[1] - direction[1] * (back + OVERLAP),
                ],
                end: [
                    here[0] + direction[0] * (forth + OVERLAP),
                    here[1] + direction[1] * (forth + OVERLAP),
                ],
                bends: Vec::new(),
                bridges: Vec::new(),
                hole: false,
            });
            continue;
        }
        // A passage of a few pixels is the raster's, the straight test's
        // `room[index] < 1.0` above (copper meeting at a corner).
        if room[index] < 2.0 {
            continue;
        }
        // A passage that bends (a web between a hatch hole and a sliver of
        // one, C1.1's on USB_Keypad): a straight line leaves it, so each
        // way the walk follows its ridge, the widest neighbour onwards,
        // to the wide copper; the track bends at the narrowest pixel.
        let ridge = |heading: [f64; 2]| -> Option<core::Point> {
            let mut current = index;
            let mut travelled = 0.0;
            let mut visited = vec![index];
            while travelled < 0.5 {
                let (cx, cy) = ((current % width) as i64, (current / width) as i64);
                let from = point_of(current);
                let mut best: Option<(usize, f64)> = None;
                for &(dx, dy) in &RING {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                        continue;
                    }
                    let neighbour = ny as usize * width + nx as usize;
                    if !filled[neighbour] || visited.contains(&neighbour) {
                        continue;
                    }
                    // Onwards: away from the narrowest pixel, along the
                    // heading the passage leaves it by.
                    let to = point_of(neighbour);
                    let outward = [to[0] - here[0], to[1] - here[1]];
                    if outward[0] * heading[0] + outward[1] * heading[1] <= 0.0
                        || (to[0] - here[0]).hypot(to[1] - here[1])
                            <= (from[0] - here[0]).hypot(from[1] - here[1])
                    {
                        continue;
                    }
                    if best.is_none_or(|(_, room_there)| room[neighbour] > room_there) {
                        best = Some((neighbour, room[neighbour]));
                    }
                }
                let (neighbour, room_there) = best?;
                travelled += if neighbour % width != current % width
                    && neighbour / width != current / width
                {
                    PIXEL * std::f64::consts::SQRT_2
                } else {
                    PIXEL
                };
                visited.push(neighbour);
                current = neighbour;
                if room_there >= radius {
                    let to = point_of(current);
                    let (dx, dy) = (to[0] - here[0], to[1] - here[1]);
                    let length = dx.hypot(dy).max(1.0e-9);
                    return Some([to[0] + dx / length * OVERLAP, to[1] + dy / length * OVERLAP]);
                }
            }
            None
        };
        let heading = |arc: &[usize]| {
            let m = mean(arc);
            let length = m[0].hypot(m[1]).max(1.0e-9);
            [m[0] / length, m[1] / length]
        };
        // A passage leaves its narrowest pixel two opposite ways; the round
        // of a corner has two arcs of wider neighbours too, raster ties,
        // both inwards.
        let (first_way, second_way) = (heading(&arcs[0]), heading(&arcs[1]));
        if first_way[0] * second_way[0] + first_way[1] * second_way[1] > -0.5 {
            continue;
        }
        let (Some(start), Some(end)) = (ridge(first_way), ridge(second_way)) else {
            continue;
        };

        necks.push(Neck {
            at: here,
            width: 2.0 * (room[index] + 0.5) * PIXEL,
            start,
            end,
            bends: vec![here],
            bridges: Vec::new(),
            hole: false,
        });
    }
    necks
}

/// The necks of a net's copper on a layer, tile by tile.
pub(crate) fn find_necks(copper: &Copper, limit: f64) -> Vec<Neck> {
    let Some(bounds) = copper.bounds() else {
        return Vec::new();
    };
    let core_pixels = TILE - 2 * MARGIN;
    let span = [
        bounds.maximum[0] - bounds.minimum[0],
        bounds.maximum[1] - bounds.minimum[1],
    ];
    let tiles = [
        (span[0] / PIXEL / core_pixels as f64).ceil().max(1.0) as usize,
        (span[1] / PIXEL / core_pixels as f64).ceil().max(1.0) as usize,
    ];
    use rayon::prelude::*;
    (0..tiles[0] * tiles[1])
        .into_par_iter()
        .flat_map_iter(|tile| {
            let (tx, ty) = (tile % tiles[0], tile / tiles[0]);
            let origin = [
                bounds.minimum[0] + (tx * core_pixels) as f64 * PIXEL - MARGIN as f64 * PIXEL,
                bounds.minimum[1] + (ty * core_pixels) as f64 * PIXEL - MARGIN as f64 * PIXEL,
            ];
            let filled = rasterize(copper, origin, TILE, TILE);
            let anchors: Vec<core::Point> = copper
                .shapes
                .iter()
                .filter_map(|shape| {
                    let bounds = shape.aabb();
                    let centre = [
                        (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                        (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
                    ];
                    shape.contains(centre).then_some(centre)
                })
                .collect();
            if filled.iter().any(|&copper| copper) {
                tile_necks(&filled, TILE, TILE, origin, limit, MARGIN, &anchors)
            } else {
                Vec::new()
            }
        })
        .collect()
}

/// Other copper and holes a repair track keeps clear of.
struct Clearances<'a> {
    board: &'a core::Board,
    net: core::NetId,
    layer: usize,
    /// Other nets' zone fills on the layer (their teardrops among them).
    fills: Vec<(Vec<core::Point>, f64)>,
    /// Via drills: (centre, radius, net).
    drills: Vec<(core::Point, f64, Option<core::NetId>)>,
}

impl Clearances<'_> {
    fn track_fits(&self, start: core::Point, end: core::Point, width: f64) -> bool {
        let half = width / 2.0;
        let own = self.board.classes[self.board.nets[self.net as usize].class].clearance;
        for obstacle in &self.board.obstacles {
            if obstacle.layers & (1 << self.layer) == 0 && obstacle.kind != core::ObstacleKind::Hole
            {
                continue;
            }
            let required = match obstacle.kind {
                core::ObstacleKind::Copper => {
                    if obstacle.net == Some(self.net) {
                        continue;
                    }
                    let theirs = obstacle
                        .clearance_override
                        .or_else(|| {
                            obstacle.net.map(|net| {
                                self.board.classes[self.board.nets[net as usize].class].clearance
                            })
                        })
                        .unwrap_or(0.0);
                    own.max(theirs).max(obstacle.clearance)
                }
                // A keepout of the net's own (its pads' mask apertures)
                // lets the net's copper in.
                core::ObstacleKind::Keepout => {
                    if obstacle.net == Some(self.net) {
                        continue;
                    }
                    obstacle.clearance
                }
                core::ObstacleKind::Hole => {
                    if obstacle.net == Some(self.net) {
                        continue;
                    }
                    self.board.hole_clearance.max(obstacle.clearance)
                }
            };
            if obstacle.shape.distance_to_segment(start, end) - half < required + SLACK {
                return false;
            }
        }
        for (polygon, clearance) in &self.fills {
            let shape = core::Shape::Polygon {
                points: polygon.clone(),
            };
            if shape.distance_to_segment(start, end) - half < own.max(*clearance) + SLACK {
                return false;
            }
        }
        for &(centre, radius, net) in &self.drills {
            if net == Some(self.net) {
                continue;
            }
            if core::geometry::point_segment_distance(centre, start, end) - radius - half
                < self.board.hole_clearance + SLACK
            {
                return false;
            }
        }
        if !self.board.outline.is_empty() {
            if !core::geometry::point_in_polygon(start, &self.board.outline)
                || !core::geometry::point_in_polygon(end, &self.board.outline)
            {
                return false;
            }
            let edge = core::geometry::polygon_edges(&self.board.outline)
                .map(|(a, b)| core::geometry::segment_segment_distance(start, end, a, b))
                .fold(f64::INFINITY, f64::min);
            if edge - half < self.board.edge_clearance + SLACK {
                return false;
            }
        }
        true
    }
}

/// The repair's candidates for a neck: the track across it, then shifted
/// sideways a little either way, then shortened towards its middle.
fn candidates(neck: &Neck) -> Vec<Vec<core::Point>> {
    let length = core::geometry::distance(neck.start, neck.end).max(1.0e-9);
    let along = [
        (neck.end[0] - neck.start[0]) / length,
        (neck.end[1] - neck.start[1]) / length,
    ];
    let side = [-along[1], along[0]];
    let path = neck.path();
    let mut result = vec![path.clone()];
    for shift in [0.01, -0.01, 0.02, -0.02, 0.03, -0.03] {
        result.push(
            path.iter()
                .map(|point| [point[0] + side[0] * shift, point[1] + side[1] * shift])
                .collect(),
        );
    }
    for keep in [0.75, 0.5] {
        if neck.bends.is_empty() {
            let middle = [
                (neck.start[0] + neck.end[0]) / 2.0,
                (neck.start[1] + neck.end[1]) / 2.0,
            ];
            let half = length * keep / 2.0;
            result.push(vec![
                [middle[0] - along[0] * half, middle[1] - along[1] * half],
                [middle[0] + along[0] * half, middle[1] + along[1] * half],
            ]);
        } else {
            // The ends drawn in towards the bends next to them.
            let mut shorter = path.clone();
            let last = shorter.len() - 1;
            for (end, inner) in [(0, 1), (last, last - 1)] {
                let (a, b) = (shorter[end], path[inner]);
                shorter[end] = [b[0] + (a[0] - b[0]) * keep, b[1] + (a[1] - b[1]) * keep];
            }
            result.push(shorter);
        }
    }
    result
}

/// Zone fills of a saved board: (net, layer, outline).
fn zone_fills(pcb: &Expr) -> Vec<(String, String, Vec<core::Point>)> {
    let mut fills = Vec::new();
    for zone in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
    {
        let Some(net) = node_net(zone).map(str::to_string) else {
            continue;
        };
        for filled in zone
            .children()
            .iter()
            .filter(|child| child.head() == Some("filled_polygon"))
        {
            let Some(layer) = form_atom(filled, "layer", 1) else {
                continue;
            };
            let Some(points) = filled.child("pts") else {
                continue;
            };
            let outline: Vec<core::Point> = points
                .children()
                .iter()
                .filter(|point| point.head() == Some("xy"))
                .filter_map(|point| {
                    let fields = point.children();
                    Some([
                        fields.get(1)?.atom()?.parse().ok()?,
                        fields.get(2)?.atom()?.parse().ok()?,
                    ])
                })
                .collect();
            if outline.len() >= 3 {
                fills.push((net.clone(), layer.to_string(), outline));
            }
        }
    }
    fills
}

/// Via drills of a board: (centre, radius, net name).
fn via_drills(pcb: &Expr) -> Vec<(core::Point, f64, Option<String>)> {
    pcb.children()
        .iter()
        .filter(|item| item.head() == Some("via"))
        .filter_map(|via| {
            Some((
                form_xy(via, "at").ok()?,
                form_f64(via, "drill", 1).ok()? / 2.0,
                node_net(via).map(str::to_string),
            ))
        })
        .collect()
}

/// The board's minimum connection width (board setup `min_connection`), mm.
fn minimum_connection(directory: &Path, board_id: &str) -> f64 {
    fs::read_to_string(directory.join(format!("{board_id}.kicad_pro")))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|project| project["board"]["design_settings"]["rules"]["min_connection"].as_f64())
        .unwrap_or(0.0)
}

/// The board's max error, mm (setup `max_error`, KiCad's default 0.005).
fn max_error(pcb: &Expr) -> f64 {
    pcb.child("setup")
        .and_then(|setup| form_atom(setup, "max_error", 1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0.005)
}

fn net_matches(names: &NetNames, name: &str, wanted: &str) -> bool {
    name == wanted
        || names.normalize(wanted) == name
        || name.strip_prefix('/') == Some(wanted)
        || wanted.strip_prefix('/') == Some(name)
}

/// Repairs KiCad's connection_width errors on the routed board in
/// `directory` (see the module). `None` when the board has none or the
/// repair is switched off (`PCB_KICAD_NO_NECK_REPAIR`).
pub(crate) fn repair_connection_width(
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
    max_checks: Option<usize>,
) -> Result<Option<NeckRepair>, String> {
    if env::var_os("PCB_KICAD_NO_NECK_REPAIR").is_some() {
        return Ok(None);
    }
    let marker = directory.join("necks.json");
    if marker.exists() {
        return Ok(None);
    }
    let Ok(text) = fs::read_to_string(directory.join("drc.json")) else {
        return Ok(None);
    };
    let Ok(report) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(None);
    };
    let before = findings(&report);
    if before.is_empty() {
        return Ok(None);
    }
    let started = std::time::Instant::now();
    let mut repair = NeckRepair {
        before: before.len(),
        after: before.len(),
        ..Default::default()
    };
    // Within a work bound (a layout's work left, in checks): the fill, at
    // least one trial and the board as chosen.
    // The checks the trials and the pruning of the slots' bridges may
    // take together; the trials at most `NECK_TRIALS` of them.
    let budget = match max_checks {
        None => NECK_TRIALS + NECK_PRUNE_TRIALS,
        Some(checks) if checks >= NECK_MIN_CHECKS => checks - 2,
        Some(checks) => {
            repair.note = Some(format!(
                "not run: the work left pays for {checks} KiCad checks, the repair needs {NECK_MIN_CHECKS} (a fill, a trial, the board as chosen)"
            ));
            eprintln!(
                "connection width: {} necks in KiCad's report; the repair was not run: the work left pays for {checks} KiCad checks",
                repair.before
            );
            return Ok(Some(repair));
        }
    };
    let saved = Saved::take(directory, board_id)?;
    let outcome = repair_with(
        directory,
        board_id,
        config,
        &report,
        &before,
        budget,
        &mut repair,
    );
    if let Err(error) = outcome {
        // A check failed and the board in the directory may be a trial's
        // without its report: back to the board and the reports as they
        // were (the report of the board the directory holds).
        saved.restore()?;
        repair.tracks = 0;
        repair.track_mm = 0.0;
        repair.kept = false;
        repair.after = repair.before;
        repair.note = Some(format!(
            "{} (the board and its reports are as before the repair)",
            error.trim_end()
        ));
    }
    repair.seconds = started.elapsed().as_secs_f64();
    eprintln!(
        "connection width: {} necks in KiCad's report, {} found, {} repair tracks, {} after{}",
        repair.before,
        repair.found,
        repair.tracks,
        repair.after,
        if repair.kept {
            ""
        } else {
            " (board not changed)"
        }
    );
    if let Some(note) = &repair.note {
        eprintln!("connection width: {note}");
    }
    fs::write(
        &marker,
        serde_json::to_string_pretty(&repair).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", marker.display()))?;
    Ok(Some(repair))
}

/// The files of a result directory that a check rewrites (the board, the
/// reports, the preview), as they were: `None` for one that was not there.
struct Saved(Vec<(PathBuf, Option<Vec<u8>>)>);

impl Saved {
    fn take(directory: &Path, board_id: &str) -> Result<Self, String> {
        let names = [
            format!("{board_id}.kicad_pcb"),
            "drc.json".into(),
            "erc.json".into(),
            "verification.json".into(),
            "preview.json".into(),
            "preview.svg".into(),
            "preview-labels.json".into(),
        ];
        let mut files = Vec::new();
        for name in names {
            let path = directory.join(name);
            let bytes = match fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
            };
            files.push((path, bytes));
        }
        Ok(Self(files))
    }

    fn restore(&self) -> Result<(), String> {
        for (path, bytes) in &self.0 {
            match bytes {
                Some(bytes) => fs::write(path, bytes)
                    .map_err(|error| format!("failed to restore {}: {error}", path.display()))?,
                None => remove_if_present(path)?,
            }
        }
        Ok(())
    }
}

/// `repair_connection_width` as a command (`pcb-maker repair-kicad-necks`).
pub fn repair_kicad_necks(
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
) -> Result<Option<NeckRepair>, String> {
    repair_connection_width(directory, board_id, config, None)
}

fn error_counts(report: &serde_json::Value) -> (BTreeMap<String, usize>, usize) {
    let mut counts = BTreeMap::new();
    for violation in report["violations"].as_array().into_iter().flatten() {
        if violation["severity"].as_str() == Some("error") {
            *counts
                .entry(violation["type"].as_str().unwrap_or("").to_string())
                .or_default() += 1;
        }
    }
    (
        counts,
        report["unconnected_items"].as_array().map_or(0, Vec::len),
    )
}

/// The trials (KiCad checks of the board with some repair tracks) the
/// repair makes at most.
pub(crate) const NECK_TRIALS: usize = 6;

/// The checks that prune the slots' bridges at most (after the trials,
/// within the work left).
pub(crate) const NECK_PRUNE_TRIALS: usize = 8;

/// The KiCad checks a repair needs at least: the fill, one trial and the
/// board as chosen (`repair_connection_width`'s `max_checks`).
pub(crate) const NECK_MIN_CHECKS: usize = 3;

fn repair_with(
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
    report: &serde_json::Value,
    before: &[Finding],
    budget: usize,
    repair: &mut NeckRepair,
) -> Result<(), String> {
    let trials = budget.min(NECK_TRIALS);
    let prune_trials = NECK_PRUNE_TRIALS;
    let board_path = directory.join(format!("{board_id}.kicad_pcb"));
    let original = fs::read_to_string(&board_path)
        .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    // KiCad's own fill (with the teardrops it adds), from a scratch copy.
    let scratch = directory.with_extension("necks");
    if scratch.exists() {
        fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
    for extension in ["kicad_pcb", "kicad_pro", "kicad_dru"] {
        let file = directory.join(format!("{board_id}.{extension}"));
        if file.exists() {
            fs::copy(&file, scratch.join(format!("{board_id}.{extension}")))
                .map_err(|error| error.to_string())?;
        }
    }
    let saved = scratch.join(format!("{board_id}.kicad_pcb"));
    repair.kicad_fills += 1;
    let filled =
        native_report::run_kicad_save_board(&saved, &scratch.join("drc.json")).and_then(|()| {
            fs::read_to_string(&saved)
                .map_err(|error| format!("failed to read {}: {error}", saved.display()))
        });
    let _ = fs::remove_dir_all(&scratch);
    let filled = parse(&filled?)?;
    let pcb = parse(&original)?;
    let lowered = crate::board_router::lower(&pcb, config, true)?;
    let net_names = NetNames::of(&pcb);
    let board = &lowered.board;
    let layers = LayerTable::from_pcb(&pcb)?;
    let minimum = minimum_connection(directory, board_id);
    if minimum <= 0.0 {
        return Err("no minimum connection width in the board setup".into());
    }
    let epsilon = 2.0 * (0.0005 + max_error(&pcb) + 0.0005);
    let limit = minimum - epsilon;
    let width = (minimum * 1000.0).ceil() / 1000.0;
    let fills = zone_fills(&filled);
    let drills = via_drills(&pcb);
    let mut targets: Vec<(String, String)> = before
        .iter()
        .map(|finding| (finding.net.clone(), finding.layer.clone()))
        .collect();
    targets.sort();
    targets.dedup();
    // Per neck its repair: a track, or a few where it bends, or a slot's
    // bridges (`slot` marks those).
    let mut tracks: Vec<Vec<SupplementalSegment>> = Vec::new();
    let mut slot: Vec<bool> = Vec::new();
    let mut hole: Vec<bool> = Vec::new();
    for (net_name, layer_name) in targets {
        let Some(layer) = layers.index(&layer_name) else {
            continue;
        };
        let Some(net) = board
            .nets
            .iter()
            .position(|net| net_matches(&net_names, &net.name, &net_name))
            .map(|net| net as core::NetId)
        else {
            continue;
        };
        let mut copper = Copper::default();
        for (fill_net, fill_layer, outline) in &fills {
            if fill_layer == &layer_name && net_matches(&net_names, fill_net, &net_name) {
                copper.polygons.push(outline.clone());
            }
        }
        for obstacle in &board.obstacles {
            if obstacle.kind == core::ObstacleKind::Copper
                && obstacle.net == Some(net)
                && obstacle.layers & (1 << layer) != 0
            {
                copper.shapes.push(obstacle.shape.clone());
            }
        }
        let necks = find_necks(&copper, limit);
        if env::var_os("PCB_KICAD_NECKS_DEBUG").is_some() {
            eprintln!(
                "necks: {net_name} {layer_name}: {} fill outlines, {} shapes, limit {limit:.4}, {} necks",
                copper.polygons.len(),
                copper.shapes.len(),
                necks.len()
            );
        }
        repair.found += necks.len();
        let clearances = Clearances {
            board,
            net,
            layer,
            fills: fills
                .iter()
                .filter(|(fill_net, fill_layer, _)| {
                    fill_layer == &layer_name && !net_matches(&net_names, fill_net, &net_name)
                })
                .map(|(fill_net, _, outline)| {
                    let theirs = board
                        .nets
                        .iter()
                        .find(|net| net_matches(&net_names, &net.name, fill_net))
                        .map_or(0.0, |net| board.classes[net.class].clearance);
                    (outline.clone(), theirs)
                })
                .collect(),
            drills: drills
                .iter()
                .map(|(centre, radius, net)| {
                    (
                        *centre,
                        *radius,
                        net.as_deref().and_then(|name| {
                            board
                                .nets
                                .iter()
                                .position(|candidate| {
                                    net_matches(&net_names, &candidate.name, name)
                                })
                                .map(|net| net as core::NetId)
                        }),
                    )
                })
                .collect(),
        };
        for neck in &necks {
            if env::var_os("PCB_KICAD_NECKS_DEBUG").is_some() {
                eprintln!(
                    "neck {net_name} {layer_name} width {:.3} at ({:.3}, {:.3}) track ({:.3}, {:.3})-({:.3}, {:.3})",
                    neck.width,
                    neck.at[0],
                    neck.at[1],
                    neck.start[0],
                    neck.start[1],
                    neck.end[0],
                    neck.end[1]
                );
            }
            let round = |point: core::Point| point.map(|value| (value * 1.0e6).round() / 1.0e6);
            let segment = |pair: &[core::Point]| SupplementalSegment {
                connection: board.nets[net as usize].name.clone(),
                start: round(pair[0]),
                end: round(pair[1]),
                width,
                layer: layer_name.clone(),
            };
            if !neck.bridges.is_empty() {
                let fitting: Vec<SupplementalSegment> = neck
                    .bridges
                    .iter()
                    .filter(|bridge| clearances.track_fits(bridge[0], bridge[1], width))
                    .map(|bridge| segment(bridge))
                    .collect();
                if !fitting.is_empty() {
                    tracks.push(fitting);
                    slot.push(true);
                    hole.push(neck.hole);
                }
                continue;
            }
            if let Some(path) = candidates(neck).into_iter().find(|path| {
                path.windows(2)
                    .all(|pair| clearances.track_fits(pair[0], pair[1], width))
            }) {
                tracks.push(path.windows(2).map(segment).collect::<Vec<_>>());
                slot.push(false);
                hole.push(false);
            }
        }
    }
    if tracks.is_empty() {
        return Ok(());
    }
    // Try them all; when the board gets worse elsewhere (a track that
    // keeps an island KiCad would drop, say), try halves, keeping what
    // helps, within a few checks (each is a KiCad DRC).
    let names = unambiguous_pad_net_names(&pcb)?;
    let (old_counts, old_unconnected) = error_counts(report);
    let check = |chosen: &[usize]| -> Result<Option<usize>, String> {
        let mut repaired = pcb.clone();
        if let Expr::List(items) = &mut repaired {
            for &index in chosen {
                for segment in &tracks[index] {
                    items.push(supplemental_segment_expr(segment));
                }
            }
        }
        canonicalize_copper_net_names(&mut repaired, &names);
        fs::write(&board_path, format!("{}\n", encode(&repaired)))
            .map_err(|error| format!("failed to write {}: {error}", board_path.display()))?;
        verify_materialized_rung(directory, board_id)?;
        let text =
            fs::read_to_string(directory.join("drc.json")).map_err(|error| error.to_string())?;
        let after_report: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| error.to_string())?;
        let (new_counts, new_unconnected) = error_counts(&after_report);
        let worse: Vec<String> = new_counts
            .iter()
            .filter(|(kind, count)| {
                kind.as_str() != "connection_width"
                    && **count > old_counts.get(*kind).copied().unwrap_or(0)
            })
            .map(|(kind, count)| {
                format!(
                    "{kind} {} to {count}",
                    old_counts.get(kind).copied().unwrap_or(0)
                )
            })
            .chain(
                (new_unconnected > old_unconnected)
                    .then(|| format!("unconnected {old_unconnected} to {new_unconnected}")),
            )
            .collect();
        if env::var_os("PCB_KICAD_NECKS_DEBUG").is_some() {
            eprintln!(
                "necks: {} tracks: connection_width {}, worse: {worse:?}",
                chosen.len(),
                findings(&after_report).len()
            );
        }
        Ok(worse.is_empty().then(|| findings(&after_report).len()))
    };
    let mut kept: Vec<usize> = Vec::new();
    let mut best = before.len();
    let mut last_checked: Option<Vec<usize>> = None;
    // The holes' bridges after the rest, where the rest leaves necks.
    let (holes, rest): (Vec<usize>, Vec<usize>) = (0..tracks.len()).partition(|&index| hole[index]);
    let mut queue: std::collections::VecDeque<Vec<usize>> = [rest, holes]
        .into_iter()
        .filter(|group| !group.is_empty())
        .collect();
    let mut checks = 0;
    while let Some(group) = queue.pop_front() {
        if checks >= trials {
            break;
        }
        checks += 1;
        let mut chosen = kept.clone();
        chosen.extend(&group);
        let result = check(&chosen);
        last_checked = Some(chosen.clone());
        match result {
            Ok(Some(after)) if after < best => {
                kept = chosen;
                best = after;
                if best == 0 {
                    break;
                }
            }
            Ok(_) if group.len() > 1 => {
                let (left, right) = group.split_at(group.len() / 2);
                queue.push_back(left.to_vec());
                queue.push_back(right.to_vec());
            }
            Ok(_) => {}
            Err(error) => {
                repair.note = Some(error);
                break;
            }
        }
    }
    // The slots' bridges are many (every slot that can be KiCad's gap is
    // bridged, not only the few that are): with the checks left, halves
    // of them go again while the necks stay as few (USB_Keypad: 434 repair
    // tracks for its 4 necks before).
    // Holes first: they are many and seldom KiCad's.
    let bridged: Vec<usize> = kept
        .iter()
        .copied()
        .filter(|&index| slot[index] && hole[index])
        .chain(
            kept.iter()
                .copied()
                .filter(|&index| slot[index] && !hole[index]),
        )
        .collect();
    let pruning = prune_trials.min(budget.saturating_sub(checks));
    let mut pruned = 0;
    if bridged.len() > 1 && repair.note.is_none() {
        let (left, right) = bridged.split_at(bridged.len() / 2);
        let mut queue: std::collections::VecDeque<Vec<usize>> =
            std::collections::VecDeque::from([left.to_vec(), right.to_vec()]);
        while let Some(group) = queue.pop_front() {
            if pruned >= pruning {
                break;
            }
            pruned += 1;
            let chosen: Vec<usize> = kept
                .iter()
                .copied()
                .filter(|index| !group.contains(index))
                .collect();
            let result = check(&chosen);
            last_checked = Some(chosen.clone());
            match result {
                Ok(Some(after)) if after <= best => {
                    kept = chosen;
                    best = after;
                }
                Ok(_) if group.len() > 1 => {
                    let (left, right) = group.split_at(group.len() / 2);
                    queue.push_back(left.to_vec());
                    queue.push_back(right.to_vec());
                }
                Ok(_) => {}
                Err(error) => {
                    repair.note = Some(error);
                    break;
                }
            }
        }
    }
    repair.tracks = kept.iter().map(|&index| tracks[index].len()).sum();
    repair.track_mm = kept
        .iter()
        .flat_map(|&index| &tracks[index])
        .map(|track| (track.end[0] - track.start[0]).hypot(track.end[1] - track.start[1]))
        .sum();
    repair.after = best;
    repair.kept = !kept.is_empty();
    if last_checked.as_ref() != Some(&kept) {
        // The board as chosen (its DRC is in the cache).
        if kept.is_empty() {
            fs::write(&board_path, &original)
                .map_err(|error| format!("failed to write {}: {error}", board_path.display()))?;
            verify_materialized_rung(directory, board_id)?;
        } else {
            check(&kept)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<core::Point> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    #[test]
    fn a_thin_bridge_between_two_wide_parts_is_a_neck_a_wide_one_and_a_spike_are_not() {
        let limit = 0.115;
        // Two 1 mm squares joined by a bridge 0.06 wide and 0.3 long.
        let necked = Copper {
            polygons: vec![
                rectangle(0.0, 0.0, 1.0, 1.0),
                rectangle(1.3, 0.0, 2.3, 1.0),
                rectangle(0.95, 0.47, 1.35, 0.53),
            ],
            shapes: Vec::new(),
        };
        let necks = find_necks(&necked, limit);
        assert_eq!(necks.len(), 1, "{necks:?}");
        let neck = &necks[0];
        assert!(
            neck.at[0] > 0.95 && neck.at[0] < 1.35 && (neck.at[1] - 0.5).abs() < 0.02,
            "{neck:?}"
        );
        assert!((neck.width - 0.06).abs() < 0.01, "{neck:?}");
        // The repair runs across the bridge, from one square into the other.
        assert!(
            neck.start[0] < 1.0 && neck.end[0] > 1.3 || neck.end[0] < 1.0 && neck.start[0] > 1.3,
            "{neck:?}"
        );
        assert!(
            (neck.start[1] - 0.5).abs() < 0.02 && (neck.end[1] - 0.5).abs() < 0.02,
            "{neck:?}"
        );
        // The same bridge 0.2 wide (KiCad's hatch web) is no neck.
        let wide = Copper {
            polygons: vec![
                rectangle(0.0, 0.0, 1.0, 1.0),
                rectangle(1.3, 0.0, 2.3, 1.0),
                rectangle(0.95, 0.4, 1.35, 0.6),
            ],
            shapes: Vec::new(),
        };
        assert!(find_necks(&wide, limit).is_empty());
        // A thin spike touches wide copper once: no neck (KiCad asks for
        // copper wider than the limit on both sides).
        let spike = Copper {
            polygons: vec![
                rectangle(0.0, 0.0, 1.0, 1.0),
                rectangle(0.95, 0.47, 1.5, 0.53),
            ],
            shapes: Vec::new(),
        };
        assert!(find_necks(&spike, limit).is_empty());
        // A pad (a shape) joined to a fill by a thin strip is a neck too:
        // KiCad unites pads and fills.
        let pad = Copper {
            polygons: vec![
                rectangle(0.0, 0.0, 1.0, 1.0),
                rectangle(0.95, 0.47, 1.35, 0.53),
            ],
            shapes: vec![core::Shape::Polygon {
                points: rectangle(1.3, 0.2, 1.9, 0.8),
            }],
        };
        assert_eq!(find_necks(&pad, limit).len(), 1);
    }

    #[test]
    fn a_bent_passage_is_followed_along_its_ridge() {
        let limit = 0.115;
        // Two 1 mm squares joined by a passage 0.06 wide that bends by
        // 60 degrees half way: a straight line from its narrowest point
        // leaves it (USB_Keypad's web between a hatch hole and a sliver of
        // one at C1.1), the repair follows it into both squares.
        let (bend, end) = ([1.2, 0.5], [1.45, 0.5 + 0.25 * 3.0f64.sqrt()]);
        let bent = Copper {
            polygons: vec![
                rectangle(0.0, 0.0, 1.0, 1.0),
                rectangle(1.3, 0.85, 2.3, 1.85),
            ],
            shapes: vec![
                core::Shape::Capsule {
                    start: [0.95, 0.5],
                    end: bend,
                    radius: 0.03,
                },
                core::Shape::Capsule {
                    start: bend,
                    end,
                    radius: 0.03,
                },
            ],
        };
        let necks = find_necks(&bent, limit);
        let inside = |point: core::Point, square: [f64; 4]| {
            point[0] > square[0]
                && point[0] < square[2]
                && point[1] > square[1]
                && point[1] < square[3]
        };
        let (left, right) = ([0.0, 0.0, 1.0, 1.0], [1.3, 0.85, 2.3, 1.85]);
        assert!(
            necks.iter().any(|neck| {
                (inside(neck.start, left) && inside(neck.end, right))
                    || (inside(neck.end, left) && inside(neck.start, right))
            }),
            "{necks:?}"
        );
    }

    /// A U of copper whose arms leave a slit 0.06 wide and 0.6 long
    /// between them, `anchored` where a pad of the net lies in it.
    fn slit(anchored: bool) -> Copper {
        Copper {
            polygons: vec![
                rectangle(0.0, 0.0, 0.4, 1.0),
                rectangle(0.46, 0.0, 0.86, 1.0),
                rectangle(0.0, 0.6, 0.86, 1.0),
            ],
            shapes: if anchored {
                vec![core::Shape::Circle {
                    center: [0.43, 0.8],
                    radius: 0.1,
                }]
            } else {
                Vec::new()
            },
        }
    }

    #[test]
    fn a_slit_in_one_piece_of_copper_is_bridged_wall_to_wall() {
        let limit = 0.115;
        let necks = find_necks(&slit(true), limit);
        let bridged: Vec<&Neck> = necks
            .iter()
            .filter(|neck| !neck.bridges.is_empty())
            .collect();
        assert_eq!(bridged.len(), 1, "{necks:?}");
        let neck = bridged[0];
        // The bridges run across the slit from arm to arm, both ends in
        // copper (KiCad joins a track to a fill by its ends), and lie
        // over every part of it.
        for bridge in &neck.bridges {
            let (a, b) = (bridge[0], bridge[1]);
            assert!(a[0].min(b[0]) < 0.4 && a[0].max(b[0]) > 0.46, "{bridge:?}");
        }
        for step in 0..=10 {
            let y = 0.02 + 0.056 * step as f64;
            let point = [0.43, y];
            assert!(
                neck.bridges
                    .iter()
                    .any(|bridge| core::geometry::point_segment_distance(
                        point, bridge[0], bridge[1]
                    ) <= 0.127 / 2.0),
                "{point:?} not under a bridge: {:?}",
                neck.bridges
            );
        }
        // An island of the fill (no item of the net on it) is left alone:
        // a track on it would make KiCad want it joined.
        assert!(
            find_necks(&slit(false), limit)
                .iter()
                .all(|neck| neck.bridges.is_empty())
        );
    }

    #[test]
    fn the_tip_of_a_hatch_hole_is_no_slot() {
        let limit = 0.115;
        // A square of copper with a pad on it and a triangular hole whose
        // tips are 45 degrees (a hatch hole cut by a strand): the closing
        // fills the tips, each one stretch of shore and narrower than the
        // limit, which KiCad's check does not pair across.
        let mut outline = rectangle(0.0, 0.0, 3.0, 3.0);
        // The hole joined to the outline by a slit of no width, as KiCad
        // fractures a fill.
        outline.splice(
            1..1,
            [
                [0.5, 0.0],
                [0.5, 0.5],
                [1.5, 0.5],
                [0.5, 1.5],
                [0.5, 0.5],
                [0.5, 0.0],
            ],
        );
        let copper = Copper {
            polygons: vec![outline],
            shapes: vec![core::Shape::Circle {
                center: [2.8, 0.3],
                radius: 0.1,
            }],
        };
        let necks = find_necks(&copper, limit);
        assert!(
            necks.iter().all(|neck| neck.bridges.is_empty()),
            "{necks:?}"
        );
    }

    #[test]
    fn findings_are_read_from_kicads_report() {
        let report = serde_json::json!({"violations": [
            {"type": "connection_width", "severity": "error",
             "description": "Copper connection too narrow (board setup constraints minimum connection width 0.1270 mm; actual 0.0722 mm) (F.Cu)",
             "items": [{"description": "Pad 3 [GND] of RGB54 on F.Cu"}, {"description": "Zone [GND] on F.Cu and B.Cu, priority 0"}]},
            {"type": "connection_width", "severity": "warning", "description": "x (B.Cu)", "items": []},
            {"type": "clearance", "severity": "error", "description": "y", "items": []}
        ]});
        assert_eq!(
            findings(&report),
            vec![Finding {
                net: "GND".into(),
                layer: "F.Cu".into(),
                width: 0.0722
            }]
        );
    }

    #[test]
    fn a_repair_track_keeps_its_clearances() {
        let mut board = core::Board {
            layer_count: 2,
            outline: rectangle(-5.0, -5.0, 5.0, 5.0),
            edge_clearance: 0.3,
            hole_clearance: 0.25,
            hole_to_hole: 0.25,
            classes: vec![core::RuleClass {
                trace_width: 0.127,
                clearance: 0.127,
                via_diameter: 0.6,
                via_drill: 0.3,
                layers: 0,
            }],
            neck_width: 0.1,
            obstacles: Vec::new(),
            nets: vec![
                core::Net {
                    name: "GND".into(),
                    class: 0,
                    terminals: Vec::new(),
                },
                core::Net {
                    name: "SIG".into(),
                    class: 0,
                    terminals: Vec::new(),
                },
            ],
            planes: Vec::new(),
            solid_pads: Vec::new(),
            isolated_pads: Vec::new(),
            solid_pad_obstacles: Vec::new(),
            isolated_pad_obstacles: Vec::new(),
            pad_spokes: Default::default(),
            merged_pads: Vec::new(),
            no_via_pads: Vec::new(),
            fill_passes: Vec::new(),
            pin_row_gaps: Vec::new(),
            terminal_copper: Vec::new(),
            attachment: Default::default(),
        };
        board.obstacles.push(core::Obstacle {
            shape: core::Shape::Capsule {
                start: [-1.0, 0.3],
                end: [1.0, 0.3],
                radius: 0.1,
            },
            layers: 1,
            kind: core::ObstacleKind::Copper,
            net: Some(1),
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: "signal".into(),
        });
        let clearances = Clearances {
            board: &board,
            net: 0,
            layer: 0,
            fills: Vec::new(),
            drills: Vec::new(),
        };
        // 0.3 - 0.1 - 0.0635 = 0.1365 from the signal: fits.
        assert!(clearances.track_fits([-0.2, 0.0], [0.2, 0.0], 0.127));
        // 0.05 nearer: too close.
        assert!(!clearances.track_fits([-0.2, 0.05], [0.2, 0.05], 0.127));
        // On the other layer the signal is not in the way.
        let other = Clearances {
            board: &board,
            net: 0,
            layer: 1,
            fills: Vec::new(),
            drills: Vec::new(),
        };
        assert!(other.track_fits([-0.2, 0.05], [0.2, 0.05], 0.127));
        // A via drill of another net within the hole clearance.
        let drilled = Clearances {
            board: &board,
            net: 0,
            layer: 1,
            fills: Vec::new(),
            drills: vec![([0.0, -0.4], 0.15, Some(1))],
        };
        assert!(!drilled.track_fits([-0.2, 0.0], [0.2, 0.0], 0.127));
    }

    #[test]
    fn a_repair_the_work_left_does_not_pay_for_does_not_run() {
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-necks-bound-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("drc.json"),
            serde_json::json!({"violations": [{
                "type": "connection_width", "severity": "error",
                "description": "Connection width too small (board minimum 0.2000 mm; actual 0.1000 mm) (B.Cu)",
                "items": [{"description": "Zone [GND] on B.Cu"}]
            }]})
            .to_string(),
        )
        .unwrap();
        let config = KiCadBoardRouterConfig::default();
        let repair = repair_connection_width(&directory, "b", &config, Some(NECK_MIN_CHECKS - 1))
            .unwrap()
            .unwrap();
        assert_eq!((repair.before, repair.after, repair.kept), (1, 1, false));
        assert_eq!(repair.kicad_fills, 0);
        assert!(
            repair
                .note
                .unwrap()
                .starts_with("not run: the work left pays for 2 KiCad checks")
        );
        // Nothing written: a later call with work left still repairs.
        assert!(!directory.join("necks.json").exists());
        fs::remove_dir_all(&directory).unwrap();
    }
}

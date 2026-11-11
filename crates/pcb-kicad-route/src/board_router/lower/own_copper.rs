// Copyright (C) 2026 Toit contributors.

//! A footprint's own copper graphics where they wall its pads: an
//! antenna's feed inside the antenna's polygon, a microphone's ground pads
//! on its ground ring.
//!
//! KiCad 10.0.7's rule, established with kicad-cli on hand-made boards
//! (one footprint each, Default clearance 0.2 mm, a class of 0.5 mm for
//! the nets `A*`) and on the designer boards that have such graphics
//! (osprey's and WLED_Matrix's inverted-F antennas, reCamera's MIC1, the
//! laptop keyboard's meander antenna):
//!
//! - A copper graphic of a footprint (`fp_line`, `fp_arc`, `fp_rect`,
//!   `fp_circle`, `fp_poly`) is copper of no net, whatever it touches,
//!   unless the footprint is a net tie (`net_tie_pad_groups`, footprint.rs
//!   `net_tie_copper`). Touching a pad does not give it the pad's net, and
//!   being of the same footprint changes nothing.
//! - It never conducts: two pads of one net joined only by the graphic stay
//!   unconnected, and a track of the pad's net ending on the graphic and
//!   not on the pad reaches no pad.
//! - Against a pad of its own footprint that it touches or overlaps, KiCad
//!   reports a `clearance` finding at 0 mm (or `shorting_items`, by the
//!   order KiCad takes the pair in: osprey's PTH pad, reCamera's pads),
//!   with the larger of the pad's class clearance and Default's: the pad
//!   was born in violation, and run.py's strict rule and the verdict set
//!   such a finding apart as the footprint's own (`footprint_own`).
//! - Against a track or a via of any net, the touching pad's net included,
//!   it reports `shorting_items` where they touch and `clearance` where
//!   they come nearer than the larger of Default's clearance and the other
//!   net's class clearance. The touching pad's class does not carry over:
//!   a Default track 0.35 mm from a graphic that touches a pad of the 0.5 mm
//!   class has no finding.
//! - A graphic touching pads of two nets (an inverted-F antenna's feed and
//!   ground) is no different: a clearance finding against each pad, no
//!   finding that the two nets short, no connection between them.
//! - Its DRC measures an unfilled `fp_circle` by its centre line, whatever
//!   its stroke: a track 0.05 mm outside a 0.45 mm ring's copper has no
//!   finding, one crossing the ring a `clearance` at the distance to the
//!   centre line (reCamera's MIC1). The lowering keeps the stroke: it is
//!   copper on the board.
//!
//! So a pad that its own footprint's copper walls in (no point of it keeps
//! a track's clearance from that copper) cannot be connected without a
//! finding: the track touches the graphic. The designers' boards carry
//! that finding (osprey: shorting_items, its feed track against the
//! antenna's polygon; reCamera: shorting_items against the ring, and three
//! of MIC1's four ground pads left open; the keyboard: clearance at 0 mm
//! against the meander). The lowering keeps every such graphic copper of no
//! net, as KiCad does, and gives a walled pad exits as the designer's
//! track takes them (`Builder::own_copper_exits`): straight corridors from
//! the pad's centre through the graphic, copper of the pad's net, wide
//! enough for the track of its class with its clearance from the rest, and
//! fenced (`exit_fence`) so that the track crosses the graphic along the
//! corridor's axis and keeps its clearance from the graphic everywhere
//! else. The graphic outside the corridors stays copper of no net, every
//! via keeps clear of all of it, the footprint's own mask opening over it
//! (a ring bared for its gasket) is cut back alike, and the net's pours
//! keep out of the corridors (KiCad's fill keeps its clearance from the
//! graphic, so a pour reaches no walled pad through it). A pad with a point
//! that keeps the clearance, or with a copper layer the graphics leave
//! free, gets no corridor: nothing changes for it. A graphic touching pads
//! of two nets gets each walled pad's corridors in that pad's net, never
//! one near the other net's pad; the rest of it stays no net's, so the
//! model never joins the two nets, and KiCad finds no short between them
//! (osprey's feed and ground).

use super::footprint::Footprint;
use super::*;
use pcb_router as core;

/// How far a corridor reaches past what its track needs, at its sides and
/// beyond its outer end: room for the lattice node that ends the track, as
/// a net tie's carve leaves (footprint.rs, `NET_TIE_MARGIN`).
const EXIT_MARGIN: f64 = 0.1;

/// The directions tried from a walled pad's centre.
const EXIT_DIRECTIONS: usize = 32;

/// How far from the pad's centre a corridor may reach before it leaves the
/// graphic with its clearance (mm).
const EXIT_REACH: f64 = 3.0;

/// The step of the search along a direction and of a pad's sampling (mm).
const EXIT_STEP: f64 = 0.01;

/// At most this many corridors per pad and layer: the shortest way out and
/// others at least `EXIT_SPREAD` radians from every one taken, no longer
/// than twice the shortest and 0.2 mm (a short way out may lead into a
/// pocket of the graphic).
const EXITS_PER_PAD: usize = 4;
const EXIT_SPREAD: f64 = std::f64::consts::FRAC_PI_4;

/// One way out of a walled pad: a straight corridor from its centre,
/// copper of its net on one layer, `radius` wide on each side: the track's
/// `clearance` and `half_width` and `EXIT_MARGIN`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Exit {
    pub layer: usize,
    pub net: core::NetId,
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub radius: f64,
    pub clearance: f64,
    pub half_width: f64,
}

impl Builder {
    /// The exits of the footprint's pads that its own copper graphics wall
    /// in (the module's doc), and the graphics cut back around them: each
    /// graphic a corridor crosses keeps its place among the obstacles as
    /// what lies outside the corridors (on the corridors' layers; a copy
    /// keeps its other layers), a vias-only copy of the whole of it keeps
    /// every via clear, the footprint's own mask openings are cut back
    /// alike, and each corridor is copper of its pad's net and a pour
    /// keepout.
    pub(super) fn own_copper_exits(&mut self, ctx: &Context, footprint: &Footprint) {
        if footprint.own_copper.is_empty() {
            return;
        }
        let exits = self.walled_pad_exits(ctx, footprint);
        let debug = std::env::var_os("PCB_ROUTER_DEBUG").is_some();
        if debug {
            self.print_touching_pads(footprint);
        }
        if exits.is_empty() {
            return;
        }
        if debug {
            for exit in &exits {
                eprintln!(
                    "{} own copper: exit of net {} on layer {} from {:?} to {:?}, radius {:.3}",
                    footprint.reference, exit.net, exit.layer, exit.start, exit.end, exit.radius
                );
            }
        }
        // Each corridor's fence, from the graphics as they are: the
        // corridor lets its track through the graphic along its axis, not
        // beside it.
        let fences: Vec<Option<core::Shape>> = exits
            .iter()
            .map(|exit| {
                let walls: Vec<&core::Shape> = footprint
                    .own_copper
                    .iter()
                    .map(|&graphic| &self.obstacles[graphic])
                    .filter(|graphic| graphic.layers & (1 << exit.layer) != 0)
                    .map(|graphic| &graphic.shape)
                    .collect();
                let strips: Vec<&Exit> = exits
                    .iter()
                    .filter(|other| other.layer == exit.layer)
                    .collect();
                exit_fence(exit, &walls, &strips)
            })
            .collect();
        for &index in &footprint.own_copper {
            let graphic = &self.obstacles[index];
            let crossing: Vec<&Exit> = exits
                .iter()
                .filter(|exit| {
                    graphic.layers & (1 << exit.layer) != 0
                        && graphic.shape.distance_to_segment(exit.start, exit.end) < exit.radius
                })
                .collect();
            if crossing.is_empty() {
                continue;
            }
            let cut_layers = crossing
                .iter()
                .fold(0, |mask, exit| mask | (1 << exit.layer));
            let original = graphic.clone();
            let cuts: Vec<([f64; 2], [f64; 2], f64)> = crossing
                .iter()
                .map(|exit| (exit.start, exit.end, exit.radius))
                .collect();
            let rest = cut_corridors(&original.shape, &cuts);
            let graphic = &mut self.obstacles[index];
            graphic.layers = original.layers & cut_layers;
            match rest {
                Some(shape) => graphic.shape = shape,
                // Nothing left outside the corridors: the vias-only copy
                // below keeps the vias out, the corridors the other nets.
                None => {
                    graphic.blocks_tracks = false;
                    graphic.blocks_vias = false;
                }
            }
            if original.layers & !cut_layers != 0 {
                self.obstacles.push(core::Obstacle {
                    layers: original.layers & !cut_layers,
                    ..original.clone()
                });
            }
            self.obstacles.push(core::Obstacle {
                layers: cut_layers,
                blocks_tracks: false,
                blocks_vias: true,
                label: format!("{} (vias)", original.label),
                ..original
            });
        }
        // The footprint's own mask openings over the graphic (reCamera's
        // MIC1 bares its ring) would keep the track out of the corridor
        // too: they are cut back by the corridor and the reach a track
        // keeps from an opening (mask.rs, `place_mask_openings`).
        for opening in &mut self.mask_openings[footprint.graphic_openings.clone()] {
            let reach = (ctx.mask.pad_mask_reach + opening.margin)
                .max(ctx.mask.mask_to_copper + opening.expansion);
            let cuts: Vec<([f64; 2], [f64; 2], f64)> = exits
                .iter()
                .filter(|exit| exit.layer == opening.layer)
                .map(|exit| (exit.start, exit.end, exit.radius + reach))
                .collect();
            if cuts.is_empty() {
                continue;
            }
            opening.shapes = std::mem::take(&mut opening.shapes)
                .into_iter()
                .filter_map(|shape| {
                    let crossed = cuts.iter().any(|(start, end, radius)| {
                        shape.distance_to_segment(*start, *end) < *radius
                    });
                    if crossed {
                        cut_corridors(&shape, &cuts)
                    } else {
                        Some(shape)
                    }
                })
                .collect();
        }
        for (exit, fence) in exits.into_iter().zip(fences) {
            if let Some(fence) = fence {
                self.obstacles.push(core::Obstacle {
                    shape: fence,
                    layers: 1 << exit.layer,
                    kind: core::ObstacleKind::Keepout,
                    net: None,
                    clearance: 0.0,
                    clearance_override: None,
                    plated: false,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: format!("{} copper exit fence", footprint.reference),
                });
            }
            let shape = core::Shape::Capsule {
                start: exit.start,
                end: exit.end,
                radius: exit.radius,
            };
            self.pour_keepouts.push((
                1 << exit.layer,
                capsule_polygon(exit.start, exit.end, exit.radius),
            ));
            self.obstacles.push(core::Obstacle {
                shape,
                layers: 1 << exit.layer,
                kind: core::ObstacleKind::Copper,
                net: Some(exit.net),
                clearance: ctx.no_net_clearance,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: format!("{} copper exit", footprint.reference),
            });
        }
    }

    /// With `PCB_ROUTER_DEBUG`: one line per pad of the footprint that its
    /// own copper graphics touch, for a census of the boards.
    fn print_touching_pads(&self, footprint: &Footprint) {
        for (pad, copper) in footprint.pads.iter().zip(&footprint.copper) {
            let touching = copper.iter().any(|&index| {
                let copper = &self.obstacles[index];
                footprint.own_copper.iter().any(|&graphic| {
                    let graphic = &self.obstacles[graphic];
                    graphic.layers & copper.layers != 0
                        && graphic.shape.aabb().intersects(copper.shape.aabb())
                        && core::verify::shape_distance(&graphic.shape, &copper.shape) <= 1.0e-6
                })
            });
            if touching {
                let net = self.obstacles[copper[0]]
                    .net
                    .map(|net| self.nets[net as usize].name.clone());
                eprintln!(
                    "{} own copper: touches pad {} of net {net:?}",
                    footprint.reference, pad.number
                );
            }
        }
    }

    /// The exits of the footprint's walled pads: a pad of a net whose every
    /// copper layer the footprint's own graphics wall (`free_point` finds
    /// no point on it for its class's track), on each of those layers.
    fn walled_pad_exits(&self, ctx: &Context, footprint: &Footprint) -> Vec<Exit> {
        let mut exits = Vec::new();
        for (pad, copper) in footprint.pads.iter().zip(&footprint.copper) {
            let Some(net) = self.obstacles[copper[0]].net else {
                continue;
            };
            let class = &self.classes[self.nets[net as usize].class];
            let half_width = class.trace_width / 2.0;
            let need = class.clearance.max(ctx.no_net_clearance) + half_width;
            let mut walled: Vec<(usize, Vec<&core::Shape>)> = Vec::new();
            let mut free = false;
            'layers: for &index in copper {
                let copper = &self.obstacles[index];
                for layer in 0..ctx.layers.len() {
                    if copper.layers & (1 << layer) == 0 {
                        continue;
                    }
                    let reach = copper.shape.aabb().inflated(need + half_width);
                    let walls: Vec<&core::Shape> = footprint
                        .own_copper
                        .iter()
                        .map(|&graphic| &self.obstacles[graphic])
                        .filter(|graphic| {
                            graphic.layers & (1 << layer) != 0
                                && graphic.shape.aabb().intersects(reach)
                        })
                        .map(|graphic| &graphic.shape)
                        .collect();
                    if walls.is_empty() || free_point(&copper.shape, &walls, need, half_width) {
                        free = true;
                        break 'layers;
                    }
                    walled.push((layer, walls));
                }
            }
            if free || walled.is_empty() {
                continue;
            }
            // Another net's pad of the footprint is no way out.
            let others: Vec<&core::Shape> = footprint
                .copper
                .iter()
                .flatten()
                .map(|&index| &self.obstacles[index])
                .filter(|other| other.net != Some(net))
                .map(|other| &other.shape)
                .collect();
            for (layer, walls) in walled {
                for (start, end) in pad_exits(pad.center, &walls, &others, need) {
                    exits.push(Exit {
                        layer,
                        net,
                        start,
                        end,
                        radius: need + EXIT_MARGIN,
                        clearance: need - half_width,
                        half_width,
                    });
                }
            }
        }
        exits
    }
}

/// Whether a track of `half_width` may end on `pad` keeping `need` (its
/// clearance plus its half width) from every one of `walls`: a point within
/// `half_width` of the pad's copper (the track's end then touches it) at
/// least `need` from each wall, sampled every `EXIT_STEP` (coarser on a
/// large pad).
pub(super) fn free_point(
    pad: &core::Shape,
    walls: &[&core::Shape],
    need: f64,
    half_width: f64,
) -> bool {
    let bounds = pad.aabb().inflated(half_width);
    let width = bounds.maximum[0] - bounds.minimum[0];
    let height = bounds.maximum[1] - bounds.minimum[1];
    let step = EXIT_STEP.max(width.max(height) / 400.0);
    let columns = (width / step).ceil() as usize;
    let rows = (height / step).ceil() as usize;
    (0..=rows).any(|row| {
        let y = bounds.minimum[1] + row as f64 * step;
        (0..=columns).any(|column| {
            let point = [bounds.minimum[0] + column as f64 * step, y];
            pad.distance_to_point(point) <= half_width
                && walls
                    .iter()
                    .all(|wall| wall.distance_to_point(point) >= need - 1.0e-9)
        })
    })
}

/// The corridors out of a walled pad at `center`: for each of
/// `EXIT_DIRECTIONS` directions the first point at least `need` from every
/// one of `walls`, not nearer than `need + EXIT_MARGIN` to any of `others`
/// on the way; the shortest and up to `EXITS_PER_PAD - 1` more (the
/// constants' doc), each from the centre to `EXIT_MARGIN` beyond that
/// point.
pub(super) fn pad_exits(
    center: [f64; 2],
    walls: &[&core::Shape],
    others: &[&core::Shape],
    need: f64,
) -> Vec<([f64; 2], [f64; 2])> {
    let radius = need + EXIT_MARGIN;
    let mut ways: Vec<(f64, f64, [f64; 2])> = Vec::new();
    for direction in 0..EXIT_DIRECTIONS {
        let angle = direction as f64 * std::f64::consts::TAU / EXIT_DIRECTIONS as f64;
        let (sin, cos) = angle.sin_cos();
        let unit = [cos, sin];
        let at = |distance: f64| {
            [
                center[0] + unit[0] * distance,
                center[1] + unit[1] * distance,
            ]
        };
        let Some(out) = (0..=(EXIT_REACH / EXIT_STEP) as usize)
            .map(|step| step as f64 * EXIT_STEP)
            .find(|&distance| {
                let point = at(distance);
                walls
                    .iter()
                    .all(|wall| wall.distance_to_point(point) >= need - 1.0e-9)
            })
        else {
            continue;
        };
        let end = at(out + EXIT_MARGIN);
        if others
            .iter()
            .any(|other| other.distance_to_segment(center, end) < radius)
        {
            continue;
        }
        ways.push((out, angle, end));
    }
    ways.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let Some(&(shortest, _, _)) = ways.first() else {
        return Vec::new();
    };
    let longest = (2.0 * shortest).max(shortest + 0.2);
    let mut taken: Vec<(f64, [f64; 2])> = Vec::new();
    for (out, angle, end) in ways {
        if taken.len() == EXITS_PER_PAD || out > longest + 1.0e-9 {
            break;
        }
        let apart = taken.iter().all(|(other, _)| {
            let turn = (angle - other).rem_euclid(std::f64::consts::TAU);
            turn.min(std::f64::consts::TAU - turn) >= EXIT_SPREAD - 1.0e-9
        });
        if apart {
            taken.push((angle, end));
        }
    }
    taken.into_iter().map(|(_, end)| (center, end)).collect()
}

/// The fence of `exit` through `walls`: where its net's track would come
/// nearer than its clearance to the copper the corridor cuts out of them
/// anywhere but along the corridor's axis (a track turning inside the
/// corridor, or hugging the cut edge beyond the graphic: KiCad's clearance
/// findings against the graphic). Its cells (of 0.02 mm) within the
/// clearance of a wall and within the corridor grown by it, and wholly
/// beyond the track's reach (`EXIT_MARGIN` and its half width) of the
/// crossing part of the axis (`crossed`) of every corridor of `strips`
/// (the corridors on its layer, so that one fence never closes another's
/// axis): a keepout its track's copper may touch, not enter. Points within the clearance of the walls outside the
/// corridor are kept from the track by what is left of the walls already.
pub(super) fn exit_fence(
    exit: &Exit,
    walls: &[&core::Shape],
    strips: &[&Exit],
) -> Option<core::Shape> {
    let step = 0.02_f64;
    let half_diagonal = step * std::f64::consts::FRAC_1_SQRT_2;
    let reach = exit.radius + exit.clearance;
    let bounds = core::Shape::Capsule {
        start: exit.start,
        end: exit.end,
        radius: reach,
    }
    .aabb();
    let columns = ((bounds.maximum[0] - bounds.minimum[0]) / step).ceil() as usize;
    let rows = ((bounds.maximum[1] - bounds.minimum[1]) / step).ceil() as usize;
    let mut parts: Vec<core::Shape> = Vec::new();
    for row in 0..rows {
        let y = bounds.minimum[1] + (row as f64 + 0.5) * step;
        let mut run: Option<usize> = None;
        for column in 0..=columns {
            let kept = column < columns && {
                let point = [bounds.minimum[0] + (column as f64 + 0.5) * step, y];
                let axis = core::geometry::point_segment_distance(point, exit.start, exit.end);
                axis < reach + half_diagonal
                    && strips.iter().all(|strip| {
                        core::geometry::point_segment_distance(point, strip.start, crossed(strip))
                            >= EXIT_MARGIN + strip.half_width + half_diagonal
                    })
                    && walls
                        .iter()
                        .any(|wall| wall.distance_to_point(point) < exit.clearance + half_diagonal)
            };
            match (kept, run) {
                (true, None) => run = Some(column),
                (false, Some(first)) => {
                    let left = bounds.minimum[0] + first as f64 * step;
                    let right = bounds.minimum[0] + column as f64 * step;
                    let (top, bottom) = (y - step / 2.0, y + step / 2.0);
                    parts.push(core::Shape::Polygon {
                        points: vec![[left, top], [right, top], [right, bottom], [left, bottom]],
                    });
                    run = None;
                }
                _ => {}
            }
        }
    }
    match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(core::Shape::Union { parts }),
    }
}

/// Where `exit`'s axis has left the walls with its track's clearance: its
/// end less `EXIT_MARGIN` (`pad_exits`). Beyond it the track keeps its
/// clearance like any other: the fence spares only the crossing.
fn crossed(exit: &Exit) -> [f64; 2] {
    let along = [exit.end[0] - exit.start[0], exit.end[1] - exit.start[1]];
    let length = along[0].hypot(along[1]);
    if length <= EXIT_MARGIN {
        return exit.start;
    }
    let scale = (length - EXIT_MARGIN) / length;
    [
        exit.start[0] + along[0] * scale,
        exit.start[1] + along[1] * scale,
    ]
}

/// `shape` less the corridors `cuts` (start, end, radius): its cells (of
/// 0.02 mm, coarser for a large shape) that may touch it and lie wholly
/// outside every corridor, as rectangles per row; None when nothing is
/// left. The cells cover the shape's edge (a cell is kept when its centre
/// is within half its diagonal of the shape), so the rest is never smaller
/// than the shape outside the corridors: other nets keep their clearance
/// from all of the graphic's copper.
pub(super) fn cut_corridors(
    shape: &core::Shape,
    cuts: &[([f64; 2], [f64; 2], f64)],
) -> Option<core::Shape> {
    let bounds = shape.aabb();
    let mut step = 0.02_f64;
    let width = bounds.maximum[0] - bounds.minimum[0];
    let height = bounds.maximum[1] - bounds.minimum[1];
    while ((width / step).ceil() + 1.0) * ((height / step).ceil() + 1.0) > 250_000.0 {
        step *= 2.0;
    }
    let half_diagonal = step * std::f64::consts::FRAC_1_SQRT_2;
    // One cell more on each side: the cells cover the edge.
    let x0 = bounds.minimum[0] - step / 2.0;
    let y0 = bounds.minimum[1] - step / 2.0;
    let columns = ((width + step) / step).ceil() as usize;
    let rows = ((height + step) / step).ceil() as usize;
    let mut parts: Vec<core::Shape> = Vec::new();
    for row in 0..rows {
        let y = y0 + (row as f64 + 0.5) * step;
        let mut run: Option<usize> = None;
        for column in 0..=columns {
            let kept = column < columns && {
                let point = [x0 + (column as f64 + 0.5) * step, y];
                shape.distance_to_point(point) <= half_diagonal
                    && cuts.iter().all(|(start, end, radius)| {
                        core::geometry::point_segment_distance(point, *start, *end)
                            >= radius + half_diagonal
                    })
            };
            match (kept, run) {
                (true, None) => run = Some(column),
                (false, Some(first)) => {
                    let (left, right) = (x0 + first as f64 * step, x0 + column as f64 * step);
                    let (top, bottom) = (y - step / 2.0, y + step / 2.0);
                    parts.push(core::Shape::Polygon {
                        points: vec![[left, top], [right, top], [right, bottom], [left, bottom]],
                    });
                    run = None;
                }
                _ => {}
            }
        }
    }
    match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(core::Shape::Union { parts }),
    }
}

/// A polygon around the capsule (`start`, `end`, `radius`): each half disc
/// by the tangents at its ends and `EDGES` edges tangent to its arc, so the
/// polygon holds the whole capsule.
pub(super) fn capsule_polygon(start: [f64; 2], end: [f64; 2], radius: f64) -> Vec<[f64; 2]> {
    const EDGES: usize = 8;
    let step = std::f64::consts::PI / EDGES as f64;
    let outer = radius / (step / 2.0).cos();
    let angle = (end[1] - start[1]).atan2(end[0] - start[0]);
    let mut points = Vec::with_capacity(2 * EDGES + 4);
    let mut point = |center: [f64; 2], distance: f64, angle: f64| {
        let (sin, cos) = angle.sin_cos();
        points.push([center[0] + distance * cos, center[1] + distance * sin]);
    };
    for (center, from) in [
        (end, angle - std::f64::consts::FRAC_PI_2),
        (start, angle + std::f64::consts::FRAC_PI_2),
    ] {
        point(center, radius, from);
        for index in 0..EDGES {
            point(center, outer, from + (index as f64 + 0.5) * step);
        }
        point(center, radius, from + std::f64::consts::PI);
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-layer board of 20 x 20 mm with footprint U1 at (10, 10) of
    /// `items`, its lowering's builder (Default: 0.2 mm tracks and
    /// clearance, so a track needs 0.3 mm from no-net copper and a
    /// corridor is 0.4 mm wide each side) and the ids of the nets `/A`,
    /// `/B`.
    fn lowered(items: &str) -> (Builder, Option<core::NetId>, Option<core::NetId>) {
        let pcb = parse(&format!(
            r#"(kicad_pcb (version 20240108)
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (net 0 "") (net 1 "/A") (net 2 "/B")
              (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
              (footprint "Test" (layer "F.Cu") (at 10 10)
                (property "Reference" "U1" (at 0 0) (layer "F.Fab"))
                {items}))"#
        ))
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let ctx = Context::new(&pcb, &config, true).unwrap();
        let mut builder = Builder::default();
        builder.board_items(&ctx).unwrap();
        // The lowering's names lose the root sheet's slash.
        let a = builder.net_ids.get("A").copied();
        let b = builder.net_ids.get("B").copied();
        assert!(a.is_some() && b.is_some());
        (builder, a, b)
    }

    fn pad(number: &str, at: [f64; 2], net: &str) -> String {
        format!(
            r#"(pad "{number}" smd rect (at {} {}) (size 0.5 0.5) (layers "F.Cu") (net {} "{net}"))"#,
            at[0],
            at[1],
            if net == "/A" { 1 } else { 2 }
        )
    }

    fn poly(x0: f64, y0: f64, x1: f64, y1: f64) -> String {
        format!(
            r#"(fp_poly (pts (xy {x0} {y0}) (xy {x1} {y0}) (xy {x1} {y1}) (xy {x0} {y1}))
              (stroke (width 0) (type solid)) (fill solid) (layer "F.Cu"))"#
        )
    }

    fn exits(builder: &Builder) -> Vec<&core::Obstacle> {
        builder
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.label == "U1 copper exit")
            .collect()
    }

    fn graphic(builder: &Builder) -> &core::Obstacle {
        builder
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label == "U1 copper fp_poly")
            .unwrap()
    }

    /// Whether a track of 0.2 mm along each exit's axis keeps 0.2 mm from
    /// what is left of the graphic.
    fn exits_keep_clear(builder: &Builder) -> bool {
        let rest = &graphic(builder).shape;
        exits(builder).iter().all(|exit| {
            let core::Shape::Capsule { start, end, .. } = exit.shape else {
                return false;
            };
            (0..=20).all(|step| {
                let t = step as f64 / 20.0;
                let point = [
                    start[0] + t * (end[0] - start[0]),
                    start[1] + t * (end[1] - start[1]),
                ];
                rest.distance_to_point(point) >= 0.3 - 1.0e-9
            })
        })
    }

    #[test]
    fn a_graphic_walling_a_pad_of_one_net_gives_the_pad_exits_of_its_net() {
        // U1.1 inside the polygon, which reaches 1.5 mm to its right; U1.2
        // of /B away from it.
        let (builder, a, _) = lowered(&format!(
            "{} {} {}",
            poly(-0.4, -0.4, 1.5, 0.4),
            pad("1", [0.0, 0.0], "/A"),
            pad("2", [4.0, 0.0], "/B")
        ));
        let exits = exits(&builder);
        // Up, left and down 0.7 mm to leave it with the clearance, and a
        // diagonal 45 degrees from them; not to the right (1.8 mm).
        assert_eq!(exits.len(), 4);
        assert!(exits.iter().all(|exit| exit.net == a && exit.layers == 1));
        for exit in &exits {
            let core::Shape::Capsule { start, end, radius } = exit.shape else {
                panic!("an exit is a corridor");
            };
            assert_eq!(start, [10.0, 10.0]);
            assert!((radius - 0.4).abs() < 1.0e-9);
            assert!(end[0] <= 10.0 + 1.0e-9, "{end:?}");
        }
        // What is left stays copper of no net where it was, cut back
        // around the corridors: the track in each keeps its clearance.
        let rest = graphic(&builder);
        assert_eq!(rest.net, None);
        assert!(rest.blocks_tracks);
        assert!(!rest.shape.contains([10.0, 10.3]));
        assert!(rest.shape.contains([11.3, 10.0]));
        assert!(exits_keep_clear(&builder));
        // A copy of the whole graphic keeps every via out.
        let vias = builder
            .obstacles
            .iter()
            .find(|obstacle| obstacle.label == "U1 copper fp_poly (vias)")
            .unwrap();
        assert!(!vias.blocks_tracks && vias.blocks_vias && vias.net.is_none());
        assert!(vias.shape.contains([10.0, 10.3]));
        // Each corridor's fence keeps the track on its axis through the
        // graphic: beside the axis it is a keepout, on it not.
        let fences: Vec<&core::Obstacle> = builder
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.label == "U1 copper exit fence")
            .collect();
        assert_eq!(fences.len(), 4);
        assert!(
            fences
                .iter()
                .all(|fence| fence.kind == core::ObstacleKind::Keepout
                    && fence.net.is_none()
                    && !fence.shape.contains([10.0, 10.5])
                    && !fence.shape.contains([9.5, 10.0]))
        );
        assert!(
            fences
                .iter()
                .any(|fence| fence.shape.contains([10.3, 10.3]))
        );
        // The net's pours keep out of the corridors.
        assert_eq!(builder.pour_keepouts.len(), 4);
        assert!(core::geometry::point_in_polygon(
            [10.0, 10.5],
            &builder.pour_keepouts[0].1
        ));
    }

    #[test]
    fn a_graphic_that_walls_no_pad_stays_as_it_was() {
        // A polygon apart from U1.1, and one over the left end of U1.2,
        // whose right end keeps the clearance.
        let (builder, _, _) = lowered(&format!(
            "{} {} {} {}",
            poly(1.0, -0.4, 2.0, 0.4),
            pad("1", [0.0, 0.0], "/A"),
            poly(-0.6, 2.6, -0.1, 3.4),
            r#"(pad "2" smd rect (at 0 3) (size 1.4 0.5) (layers "F.Cu") (net 2 "/B"))"#
        ));
        assert!(exits(&builder).is_empty());
        assert!(builder.pour_keepouts.is_empty());
        let graphics: Vec<&core::Obstacle> = builder
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.label.starts_with("U1 copper"))
            .collect();
        assert_eq!(graphics.len(), 2);
        assert!(graphics.iter().all(|graphic| graphic.net.is_none()
            && matches!(&graphic.shape, core::Shape::Polygon { points } if points.len() == 4)));
    }

    #[test]
    fn a_graphic_walling_pads_of_two_nets_gives_each_its_own_exits() {
        // An inverted-F antenna's feed (/A) and ground (/B) in one polygon.
        let (builder, a, b) = lowered(&format!(
            "{} {} {}",
            poly(-0.4, -0.4, 1.9, 0.4),
            pad("1", [0.0, 0.0], "/A"),
            pad("2", [1.5, 0.0], "/B")
        ));
        let exits = exits(&builder);
        let of = |net| -> Vec<&&core::Obstacle> {
            exits.iter().filter(|exit| exit.net == net).collect()
        };
        assert!(!of(a).is_empty() && !of(b).is_empty());
        assert_eq!(of(a).len() + of(b).len(), exits.len());
        // No exit of one net comes near the other net's pad: the model
        // never joins the two nets.
        let pads: Vec<&core::Obstacle> = builder
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.label.starts_with("U1."))
            .collect();
        assert_eq!(pads.len(), 2);
        for exit in &exits {
            let core::Shape::Capsule { start, end, radius } = exit.shape else {
                panic!("an exit is a corridor");
            };
            for pad in pads.iter().filter(|pad| pad.net != exit.net) {
                assert!(pad.shape.distance_to_segment(start, end) >= radius - 1.0e-9);
            }
        }
        let rest = graphic(&builder);
        assert_eq!(rest.net, None);
        // Between the pads the polygon stays no net's.
        assert!(rest.shape.contains([10.75, 10.0]));
        assert!(exits_keep_clear(&builder));
    }

    #[test]
    fn a_capsule_polygon_holds_its_capsule() {
        let polygon = capsule_polygon([0.0, 0.0], [1.0, 1.0], 0.3);
        let capsule = core::Shape::Capsule {
            start: [0.0, 0.0],
            end: [1.0, 1.0],
            radius: 0.3,
        };
        for step in 0..64 {
            let angle = step as f64 * std::f64::consts::TAU / 64.0;
            for center in [[0.0, 0.0], [1.0, 1.0], [0.5, 0.5]] {
                let point = [
                    center[0] + 0.299 * angle.cos(),
                    center[1] + 0.299 * angle.sin(),
                ];
                assert!(capsule.contains(point));
                assert!(
                    core::geometry::point_in_polygon(point, &polygon),
                    "{point:?}"
                );
            }
        }
        assert!(!core::geometry::point_in_polygon([0.5, -0.5], &polygon));
    }
}

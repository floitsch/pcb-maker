// Copyright (C) 2026 Toit contributors.

//! A free dimension: one edge of a rectangular outline that the layout may
//! move, to find the smallest board that comes out electrically clean.
//!
//! The layout config's `free_dimension` names the edge (`bottom`, `top`,
//! `left`, `right`), the range of the board's extent along that axis and
//! the sizes to try (a step or a list), optional price thresholds
//! (`prefer_below`) and what counts as clean (`accept`). This module holds
//! the parts that do not run a layout:
//!
//! - the board and the constraints at one size (`KiCadFreeDimensionPlan`):
//!   the outline's edge moves, and so does everything within `band_mm` of
//!   it (fixed parts by their origin, board texts by their anchor, drawn
//!   shapes and zones point by point, constraint boxes by their side
//!   nearest the edge); the rest stays where it is in board coordinates.
//!   A size where a fixed part or a rule area that lay inside the outline
//!   would end outside it, or where a fixed part carried by the edge would
//!   land on one that stays, is infeasible without a run;
//! - the search over the sizes (`KiCadFreeDimensionSearch`): probes at a
//!   reduced budget from the smallest size upward, several side by side,
//!   the first clean size confirmed at the full budget; without a clean
//!   size the closest one gets the full budget instead;
//! - a probe's outcome as read from its verdict, and the choice.
//!
//! The command (`layout-kicad-board` with the key) runs each probe as a
//! layout of its own (src/agent_cli/free_dimension.rs).

use super::*;
use crate::placement_constraints::{KiCadOutlineConstraint, KiCadPlacementConstraints};

/// The `free_dimension` object of the layout config.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadFreeDimension {
    /// The edge of the rectangular outline that moves: `bottom`, `top`,
    /// `left` or `right`. The opposite edge stays where it is.
    pub edge: String,
    /// The smallest extent of the board along the free axis (its height
    /// for `top` and `bottom`, its width for `left` and `right`), in
    /// millimetres; with `max` and `step`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// The largest extent along the free axis, in millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// The sizes tried: `min`, `min + step`, ... up to `max` (and `max`
    /// itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// The sizes tried, instead of a range and a step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sizes: Vec<f64>,
    /// Price thresholds (JLCPCB's 100 mm): a size at or below one is
    /// preferred over any larger size, and is tried itself when it lies
    /// in the range.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefer_below: Vec<f64>,
    /// What counts as clean: `pass_strict` (default: the layout's verdict
    /// is complete, exit 0: KiCad finds no unconnected item and no error
    /// that counts, every part placed, every constraint kept) or `pass`
    /// (electrically good: KiCad finds no unconnected item and no new
    /// error by the count rule, every part placed; missed soft
    /// constraints allowed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<String>,
    /// What moves with the edge: everything within this distance of it
    /// (fixed parts by their origin, board texts by their anchor, drawn
    /// shapes and zones point by point, constraint boxes by their side
    /// nearest the edge). Default 0.5 mm: what lies on the edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band_mm: Option<f64>,
    /// The work budget of one probe (default a third of
    /// `confirm_seconds`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_seconds: Option<f64>,
    /// The work budget of the confirmation at the chosen size (default a
    /// third of the layout's `total_seconds`, which is the whole search's
    /// budget).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_seconds: Option<f64>,
    /// Probes run side by side (default: a quarter of the threads, at
    /// most 4, at most one per 3 GB of available memory).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel: Option<usize>,
}

/// The default band: what lies on the edge (and the rounding of its
/// coordinates) moves with it.
pub const DEFAULT_BAND_MM: f64 = 0.5;

impl KiCadFreeDimension {
    /// The axis (0: x, 1: y) and the side of the moving edge (+1: the
    /// larger coordinate, right and bottom; -1: left and top).
    pub fn axis_and_sign(&self) -> Result<(usize, f64), String> {
        match self.edge.as_str() {
            "left" => Ok((0, -1.0)),
            "right" => Ok((0, 1.0)),
            "top" => Ok((1, -1.0)),
            "bottom" => Ok((1, 1.0)),
            other => Err(input_error(format!(
                "free_dimension: edge {other:?}: one of bottom, top, left, right"
            ))),
        }
    }

    pub fn band(&self) -> f64 {
        self.band_mm.unwrap_or(DEFAULT_BAND_MM)
    }

    /// Whether a run's verdict is clean by `accept`.
    pub fn accept_rule(&self) -> Result<&str, String> {
        match self.accept.as_deref() {
            None | Some("pass_strict") => Ok("pass_strict"),
            Some("pass") => Ok("pass"),
            Some(other) => Err(input_error(format!(
                "free_dimension: accept {other:?}: pass_strict (default) or pass"
            ))),
        }
    }

    /// The sizes to try, smallest first: the range at its step (with
    /// `max`) or the list, and the price thresholds inside the range.
    pub fn ladder(&self) -> Result<Vec<f64>, String> {
        let positive = |value: f64| value.is_finite() && value > 0.0;
        let mut sizes: Vec<f64> = if !self.sizes.is_empty() {
            if self.min.is_some() || self.max.is_some() || self.step.is_some() {
                return Err(input_error(
                    "free_dimension: give sizes, or min, max and step, not both",
                ));
            }
            self.sizes.clone()
        } else {
            let (Some(min), Some(max), Some(step)) = (self.min, self.max, self.step) else {
                return Err(input_error(
                    "free_dimension: give min, max and step (mm), or a list of sizes",
                ));
            };
            if !(positive(min) && positive(max) && positive(step) && min <= max) {
                return Err(input_error(format!(
                    "free_dimension: min {min}, max {max}, step {step}: positive, min at most max"
                )));
            }
            if (max - min) / step > 200.0 {
                return Err(input_error(format!(
                    "free_dimension: {} sizes from min {min} to max {max} at step {step}: at most 200",
                    ((max - min) / step).floor() + 1.0
                )));
            }
            let mut sizes = Vec::new();
            let mut index = 0.0;
            loop {
                let size = round_mm(min + index * step);
                if size > max + 1.0e-6 {
                    break;
                }
                sizes.push(size);
                index += 1.0;
            }
            sizes.push(max);
            sizes
        };
        if let Some(bad) = sizes.iter().find(|size| !positive(**size)) {
            return Err(input_error(format!(
                "free_dimension: size {bad}: sizes are positive millimetres"
            )));
        }
        if let Some(bad) = self.prefer_below.iter().find(|size| !positive(**size)) {
            return Err(input_error(format!(
                "free_dimension: prefer_below {bad}: thresholds are positive millimetres"
            )));
        }
        let (low, high) = sizes
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), size| {
                (low.min(*size), high.max(*size))
            });
        for threshold in &self.prefer_below {
            if *threshold >= low && *threshold <= high {
                sizes.push(*threshold);
            }
        }
        sizes.sort_by(f64::total_cmp);
        sizes.dedup_by(|a, b| (*a - *b).abs() < 1.0e-6);
        Ok(sizes)
    }

    /// The price band of a size: how many thresholds lie below it (0: at
    /// or below every threshold).
    pub fn price_band(&self, size: f64) -> usize {
        self.prefer_below
            .iter()
            .filter(|threshold| size > **threshold + 1.0e-6)
            .count()
    }
}

fn round_mm(value: f64) -> f64 {
    let rounded = (value * 1.0e6).round() / 1.0e6;
    if rounded == 0.0 { 0.0 } else { rounded }
}

fn number(value: f64) -> Expr {
    Expr::Atom(round_mm(value).to_string())
}

/// A box: [min x, min y, max x, max y].
type Box4 = [f64; 4];

fn inside(inner: &Box4, outer: &Box4, tolerance: f64) -> bool {
    inner[0] >= outer[0] - tolerance
        && inner[1] >= outer[1] - tolerance
        && inner[2] <= outer[2] + tolerance
        && inner[3] <= outer[3] + tolerance
}

fn overlap(a: &Box4, b: &Box4, margin: f64) -> bool {
    a[0] < b[2] - margin && b[0] < a[2] - margin && a[1] < b[3] - margin && b[1] < a[3] - margin
}

/// The board and its constraints as given, ready to be made at any size of
/// the free dimension.
#[derive(Clone, Debug)]
pub struct KiCadFreeDimensionPlan {
    /// The board as given: the size as given writes it unchanged.
    text: String,
    pcb: Expr,
    constraints: Option<KiCadPlacementConstraints>,
    axis: usize,
    sign: f64,
    band: f64,
    /// The outline's box as given.
    outline: Box4,
    /// Whether the constraints' `outline` gives the outline (else the
    /// board's Edge.Cuts).
    from_constraints: bool,
    /// The footprints (indices among the board's children) that keep
    /// their pose: `fixed`, locked, placed exactly.
    fixed: Vec<usize>,
    /// The extent along the free axis as given.
    pub extent_mm: f64,
    pub edge: String,
}

/// The board and constraints at one size.
#[derive(Clone, Debug)]
pub struct KiCadFreeDimensionVariant {
    pub size_mm: f64,
    /// The board file's text.
    pub board: String,
    /// The constraints at this size (`None`: the layout has none).
    pub constraints: Option<KiCadPlacementConstraints>,
    /// The outline's box at this size.
    pub outline: [f64; 4],
    /// What moved with the edge: footprints, other board items, constraint
    /// entries.
    pub moved_footprints: Vec<String>,
    pub moved_items: usize,
    pub moved_constraints: usize,
}

impl KiCadFreeDimensionPlan {
    /// Reads the board (`board_text`) and its constraints (resolved, not a
    /// file) for `free`. `fixed` and `fixed_patterns` are the placer
    /// config's. Refuses an outline that is not a rectangle (input error).
    pub fn new(
        free: &KiCadFreeDimension,
        board_text: &str,
        constraints: Option<KiCadPlacementConstraints>,
        fixed: &[String],
        fixed_patterns: &[String],
    ) -> Result<Self, String> {
        let (axis, sign) = free.axis_and_sign()?;
        free.accept_rule()?;
        let band = free.band();
        if !(band.is_finite() && band >= 0.0) {
            return Err(input_error(format!(
                "free_dimension: band_mm {band}: a distance of 0 or more"
            )));
        }
        let pcb = parse(board_text)?;
        let outline_constraint = constraints
            .as_ref()
            .and_then(|constraints| constraints.outline.as_ref());
        let (outline, from_constraints) = match outline_constraint {
            Some(outline) => (constraint_rectangle(&pcb, outline)?, true),
            None => (board_rectangle(&pcb)?, false),
        };
        let extent_mm = outline[2 + axis] - outline[axis];
        if band >= extent_mm {
            return Err(input_error(format!(
                "free_dimension: band_mm {band} reaches across the board ({extent_mm} mm): everything would move"
            )));
        }
        let mut patterns: Vec<&str> = fixed
            .iter()
            .chain(fixed_patterns)
            .map(String::as_str)
            .collect();
        let mut placed_exactly = Vec::new();
        let mut move_all = false;
        if let Some(constraints) = &constraints {
            patterns.extend(constraints.fixed.iter().map(String::as_str));
            move_all = constraints.move_all;
            for place in &constraints.place {
                if place.x.is_some() && place.y.is_some() {
                    placed_exactly.push(place.part.as_str());
                }
            }
            for addition in &constraints.add {
                if addition.x.is_some() && addition.y.is_some() {
                    placed_exactly.push(addition.part.as_str());
                }
            }
        }
        let fixed = pcb
            .children()
            .iter()
            .enumerate()
            .filter(|(_, item)| item.head() == Some("footprint"))
            .filter(|(_, footprint)| {
                let reference = footprint_reference(footprint).unwrap_or_default();
                let locked = footprint.child("locked").is_some()
                    || footprint
                        .children()
                        .iter()
                        .any(|child| child.atom() == Some("locked"));
                let netless = !move_all
                    && !footprint
                        .children()
                        .iter()
                        .filter(|child| child.head() == Some("pad"))
                        .any(|pad| pad.child("net").is_some());
                locked
                    || netless
                    || placed_exactly.contains(&reference.as_str())
                    || patterns
                        .iter()
                        .any(|pattern| crate::board_placer::glob_matches(pattern, &reference))
            })
            .map(|(index, _)| index)
            .collect();
        Ok(Self {
            text: board_text.to_string(),
            pcb,
            constraints,
            axis,
            sign,
            band,
            outline,
            from_constraints,
            fixed,
            extent_mm,
            edge: free.edge.clone(),
        })
    }

    /// The moving edge's coordinate as given.
    fn edge_coordinate(&self) -> f64 {
        if self.sign > 0.0 {
            self.outline[2 + self.axis]
        } else {
            self.outline[self.axis]
        }
    }

    /// Whether a coordinate along the free axis lies within the band (or
    /// beyond the edge).
    fn in_band(&self, coordinate: f64) -> bool {
        self.sign * (self.edge_coordinate() - coordinate) <= self.band + 1.0e-6
    }

    /// The outline's box at `size`.
    pub fn outline_at(&self, size: f64) -> Box4 {
        let mut outline = self.outline;
        let shift = self.sign * (size - self.extent_mm);
        if self.sign > 0.0 {
            outline[2 + self.axis] += shift;
        } else {
            outline[self.axis] += shift;
        }
        outline
    }

    /// The board and constraints at `size`, or why that size cannot work
    /// (`Err`, each reason one line).
    pub fn variant(&self, size: f64) -> Result<KiCadFreeDimensionVariant, Vec<String>> {
        let axis = self.axis;
        let shift = round_mm(self.sign * (size - self.extent_mm));
        let new_outline = self.outline_at(size);
        let mut reasons = Vec::new();
        if size <= 0.0 {
            return Err(vec![format!("{size} mm is not a size")]);
        }
        let mut pcb = self.pcb.clone();
        let mut moved_footprints = Vec::new();
        let mut moved_items = 0;
        // Fixed footprints' bodies before and after, and whether they move.
        let mut bodies: Vec<(String, Box4, Box4, bool, Option<bool>)> = Vec::new();
        let Expr::List(items) = &mut pcb else {
            return Err(vec!["the board is not a list".into()]);
        };
        for (index, item) in items.iter_mut().enumerate() {
            let Some(head) = item.head().map(str::to_string) else {
                continue;
            };
            match head.as_str() {
                "footprint" => {
                    let Ok(at) = form_at(item) else {
                        continue;
                    };
                    let moves = shift != 0.0 && self.in_band(at[axis]);
                    let before = footprint_box(item);
                    if moves {
                        let mut position = [at[0], at[1]];
                        position[axis] += shift;
                        if crate::board_placer::write_footprint_pose(
                            item,
                            pcb_placer::problem::Pose {
                                position,
                                angle: at[2],
                            },
                        )
                        .is_err()
                        {
                            continue;
                        }
                        moved_footprints.push(footprint_reference(item).unwrap_or_default());
                    }
                    if self.fixed.contains(&index)
                        && let Some(before) = before
                    {
                        let mut after = before;
                        if moves {
                            after[axis] += shift;
                            after[axis + 2] += shift;
                        }
                        bodies.push((
                            footprint_reference(item).unwrap_or_default(),
                            before,
                            after,
                            moves,
                            footprint_side(item),
                        ));
                    }
                }
                "gr_line" | "gr_rect" | "gr_poly" | "gr_curve" | "segment" | "zone" => {
                    let before = points_box(item);
                    let mut changed = false;
                    shift_points(
                        item,
                        axis,
                        shift,
                        &|coordinate| self.in_band(coordinate),
                        &mut changed,
                    );
                    if changed {
                        moved_items += 1;
                    }
                    // A rule area that lay on the board and would not.
                    if head == "zone"
                        && item.child("keepout").is_some()
                        && let (Some(before), Some(after)) = (before, points_box(item))
                        && inside(&before, &self.outline, 0.01)
                        && !inside(&after, &new_outline, 0.01)
                    {
                        reasons.push(format!(
                            "the rule area {} would reach beyond the outline",
                            zone_name(item)
                        ));
                    }
                }
                "gr_text" | "gr_text_box" | "gr_circle" | "gr_arc" | "arc" | "via" | "image"
                | "target" | "dimension" | "table" => {
                    if shift != 0.0
                        && let Some(point) = reference_point(item)
                        && self.in_band(point[axis])
                    {
                        let mut changed = false;
                        shift_points(item, axis, shift, &|_| true, &mut changed);
                        moved_items += 1;
                    }
                }
                _ => {}
            }
        }
        // Fixed parts that end outside, or on each other.
        for (reference, before, after, _, _) in &bodies {
            if inside(before, &self.outline, 0.01) && !inside(after, &new_outline, 0.01) {
                reasons.push(format!(
                    "the fixed part {reference} would end outside the outline"
                ));
            }
        }
        for (moved, before_a, after_a, moves, side_a) in &bodies {
            if !moves {
                continue;
            }
            for (stays, before_b, after_b, other_moves, side_b) in &bodies {
                if *other_moves {
                    continue;
                }
                let colliding = match (side_a, side_b) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                };
                if colliding
                    && overlap(after_a, after_b, 0.05)
                    && !overlap(before_a, before_b, 0.05)
                {
                    reasons.push(format!(
                        "the fixed part {moved} (moving with the edge) would overlap the fixed part {stays}"
                    ));
                }
            }
        }
        // The constraints at this size.
        let mut moved_constraints = 0;
        let constraints = match &self.constraints {
            None => None,
            Some(constraints) => {
                let mut constraints = constraints.clone();
                let old_corner = [self.outline[0], self.outline[1]];
                let new_corner = [new_outline[0], new_outline[1]];
                if self.from_constraints
                    && let Some(outline) = constraints.outline.as_mut()
                {
                    outline.x = Some(round_mm(new_corner[0]));
                    outline.y = Some(round_mm(new_corner[1]));
                    outline.width = Some(round_mm(new_outline[2] - new_outline[0]));
                    outline.height = Some(round_mm(new_outline[3] - new_outline[1]));
                }
                let mut moved_box =
                    |x: &mut [f64; 2], y: &mut [f64; 2], origin: Option<&str>, what: &str| {
                        let relative = origin != Some("absolute");
                        let mut range = [*x, *y];
                        if relative {
                            for (index, range) in range.iter_mut().enumerate() {
                                range[0] += old_corner[index];
                                range[1] += old_corner[index];
                            }
                        }
                        let before = [range[0][0], range[1][0], range[0][1], range[1][1]];
                        let near = if self.sign > 0.0 {
                            range[axis][0].max(range[axis][1])
                        } else {
                            range[axis][0].min(range[axis][1])
                        };
                        if shift != 0.0 && self.in_band(near) {
                            range[axis][0] += shift;
                            range[axis][1] += shift;
                            moved_constraints += 1;
                        }
                        let after = [range[0][0], range[1][0], range[0][1], range[1][1]];
                        let ordered = |b: Box4| {
                            [
                                b[0].min(b[2]),
                                b[1].min(b[3]),
                                b[0].max(b[2]),
                                b[1].max(b[3]),
                            ]
                        };
                        if inside(&ordered(before), &self.outline, 0.01)
                            && !inside(&ordered(after), &new_outline, 0.01)
                        {
                            reasons.push(format!("the {what} box would reach beyond the outline"));
                        }
                        if relative {
                            for (index, range) in range.iter_mut().enumerate() {
                                range[0] -= new_corner[index];
                                range[1] -= new_corner[index];
                            }
                        }
                        *x = [round_mm(range[0][0]), round_mm(range[0][1])];
                        *y = [round_mm(range[1][0]), round_mm(range[1][1])];
                    };
                for keepout in &mut constraints.keepout {
                    moved_box(
                        &mut keepout.x,
                        &mut keepout.y,
                        keepout.origin.as_deref(),
                        "keepout",
                    );
                }
                for region in &mut constraints.region {
                    if let (Some(x), Some(y)) = (region.x.as_mut(), region.y.as_mut()) {
                        moved_box(x, y, region.origin.as_deref(), "region");
                    }
                }
                let mut moved_point =
                    |x: &mut Option<f64>, y: &mut Option<f64>, origin: Option<&str>, part: &str| {
                        let (Some(px), Some(py)) = (x.as_mut(), y.as_mut()) else {
                            return;
                        };
                        let relative = origin != Some("absolute");
                        let mut point = [*px, *py];
                        if relative {
                            point[0] += old_corner[0];
                            point[1] += old_corner[1];
                        }
                        let before = [point[0], point[1], point[0], point[1]];
                        if shift != 0.0 && self.in_band(point[axis]) {
                            point[axis] += shift;
                            moved_constraints += 1;
                        }
                        if inside(&before, &self.outline, 0.01)
                            && !inside(
                                &[point[0], point[1], point[0], point[1]],
                                &new_outline,
                                0.01,
                            )
                        {
                            reasons.push(format!("{part}'s place would lie outside the outline"));
                        }
                        if relative {
                            point[0] -= new_corner[0];
                            point[1] -= new_corner[1];
                        }
                        *px = round_mm(point[0]);
                        *py = round_mm(point[1]);
                    };
                for place in &mut constraints.place {
                    let part = place.part.clone();
                    moved_point(&mut place.x, &mut place.y, place.origin.as_deref(), &part);
                }
                for addition in &mut constraints.add {
                    let part = addition.part.clone();
                    moved_point(
                        &mut addition.x,
                        &mut addition.y,
                        addition.origin.as_deref(),
                        &part,
                    );
                }
                Some(constraints)
            }
        };
        if !reasons.is_empty() {
            reasons.sort();
            reasons.dedup();
            return Err(reasons);
        }
        Ok(KiCadFreeDimensionVariant {
            size_mm: size,
            board: if shift == 0.0 {
                self.text.clone()
            } else {
                crate::kicad_sexpr::encode(&pcb)
            },
            constraints,
            outline: new_outline,
            moved_footprints,
            moved_items,
            moved_constraints,
        })
    }
}

/// The rectangle a constraints `outline` gives: width and height, at `x`,
/// `y` (default as the layout takes it: the board's outline's corner,
/// else centred on the footprints).
fn constraint_rectangle(pcb: &Expr, outline: &KiCadOutlineConstraint) -> Result<Box4, String> {
    if outline.polygon.is_some() || !outline.pieces.is_empty() {
        return Err(input_error(
            "free_dimension needs a rectangular outline: the constraints' outline is a polygon or pieces",
        ));
    }
    if !outline.cutouts.is_empty() {
        return Err(input_error(
            "free_dimension: an outline with cutouts is not supported yet (the cutouts would have to move with the edge or stay)",
        ));
    }
    let (Some(width), Some(height)) = (outline.width, outline.height) else {
        return Err(input_error(
            "free_dimension needs the outline's size: give the constraints' outline a width and a height (the free one is any size of the range)",
        ));
    };
    if outline.area_factor.is_some() || outline.aspect.is_some() || outline.shrink {
        return Err(input_error(
            "free_dimension: area_factor, aspect and shrink size an outline from the parts; give width and height instead",
        ));
    }
    let corner = match (outline.x, outline.y) {
        (Some(x), Some(y)) => [x, y],
        (x, y) => {
            let old = crate::outline::board_loops(pcb).ok().map(|loops| {
                loops
                    .outline
                    .iter()
                    .fold([f64::INFINITY, f64::INFINITY], |corner, point| {
                        [corner[0].min(point[0]), corner[1].min(point[1])]
                    })
            });
            match old {
                Some(old) if old[0].is_finite() => [x.unwrap_or(old[0]), y.unwrap_or(old[1])],
                _ => {
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
            }
        }
    };
    Ok([corner[0], corner[1], corner[0] + width, corner[1] + height])
}

/// The board's own outline (its Edge.Cuts graphics, not the footprints')
/// when it is a rectangle: one `gr_rect`, or lines and polygons along the
/// sides of their box. Graphics inside the box (cutouts) are left alone.
fn board_rectangle(pcb: &Expr) -> Result<Box4, String> {
    let edges: Vec<&Expr> = pcb
        .children()
        .iter()
        .filter(|item| {
            item.head().is_some_and(|head| head.starts_with("gr_"))
                && form_atom(item, "layer", 1) == Some("Edge.Cuts")
        })
        .collect();
    if edges.is_empty() {
        return Err(input_error(
            "free_dimension needs an outline: the board has no Edge.Cuts graphics and the constraints no outline",
        ));
    }
    let mut points_of = Vec::new();
    for item in &edges {
        points_of.push(
            edge_points(item)
                .map_err(|error| input_error(format!("free_dimension: the outline: {error}")))?,
        );
    }
    let all = points_of.iter().flatten();
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for point in all {
        bounds = [
            bounds[0].min(point[0]),
            bounds[1].min(point[1]),
            bounds[2].max(point[0]),
            bounds[3].max(point[1]),
        ];
    }
    let tolerance = 1.0e-3;
    let on_side = |point: &[f64; 2]| {
        (point[0] - bounds[0]).abs() < tolerance
            || (point[0] - bounds[2]).abs() < tolerance
            || (point[1] - bounds[1]).abs() < tolerance
            || (point[1] - bounds[3]).abs() < tolerance
    };
    let refuse = |what: &str| {
        Err(input_error(format!(
            "free_dimension needs a rectangular outline (one gr_rect, or lines along the sides of a box): the board's Edge.Cuts {what}; give the outline as a rectangle in constraints.json (width, height, corner_radius for round corners)"
        )))
    };
    for (item, points) in edges.iter().zip(&points_of) {
        if !points.iter().any(on_side) {
            // A cutout inside the board: it moves as other graphics do.
            continue;
        }
        match item.head() {
            Some("gr_rect") => {}
            Some("gr_line" | "gr_poly") => {
                for pair in points.windows(2) {
                    let straight = (pair[0][0] - pair[1][0]).abs() < tolerance
                        || (pair[0][1] - pair[1][1]).abs() < tolerance;
                    if !straight || !on_side(&pair[0]) || !on_side(&pair[1]) {
                        return refuse("have a side that is not along the box");
                    }
                }
                if points.iter().any(|point| !on_side(point)) {
                    return refuse("have a corner inside the box");
                }
            }
            Some(head) => return refuse(&format!("have a {head} on the outline")),
            None => {}
        }
    }
    if !(bounds[2] > bounds[0] && bounds[3] > bounds[1]) {
        return refuse("have no area");
    }
    Ok(bounds)
}

/// The points of a board graphic on Edge.Cuts, in drawing order (a
/// polygon closed, an arc and a circle as chords).
fn edge_points(item: &Expr) -> Result<Vec<[f64; 2]>, String> {
    match item.head() {
        Some("gr_circle") => {
            let center = form_xy(item, "center")?;
            let end = form_xy(item, "end")?;
            Ok(crate::outline::circle_points(
                center,
                distance_squared(center, end).sqrt(),
            ))
        }
        Some("gr_poly") => {
            let mut points = Vec::new();
            if let Some(pts) = item.child("pts") {
                for xy in pts.children().iter().filter(|xy| xy.head() == Some("xy")) {
                    points.push([
                        expression_coordinate(xy, 1, "x")?,
                        expression_coordinate(xy, 2, "y")?,
                    ]);
                }
            }
            if let Some(first) = points.first().copied() {
                points.push(first);
            }
            Ok(points)
        }
        _ => crate::outline::outline_points(item),
    }
}

/// The board box of a footprint's body (courtyard and pads), if it has one.
fn footprint_box(footprint: &Expr) -> Option<Box4> {
    let at = form_at(footprint).ok()?;
    let (center, size, _) = crate::board_placer::local_body(footprint).ok()?;
    if !(size[0] > 0.0 && size[1] > 0.0) {
        return None;
    }
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (dx, dy) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
        let local = [center[0] + dx * size[0], center[1] + dy * size[1]];
        let offset = rotate_vector(local, -at[2]);
        let point = [at[0] + offset[0], at[1] + offset[1]];
        bounds = [
            bounds[0].min(point[0]),
            bounds[1].min(point[1]),
            bounds[2].max(point[0]),
            bounds[3].max(point[1]),
        ];
    }
    Some(bounds)
}

/// The side a footprint's body is on (`Some(true)`: the back), `None` for
/// a part with holes, which collides on both.
fn footprint_side(footprint: &Expr) -> Option<bool> {
    let holes = footprint
        .children()
        .iter()
        .filter(|child| child.head() == Some("pad"))
        .any(|pad| {
            matches!(
                pad.children().get(2).and_then(Expr::atom),
                Some("thru_hole" | "np_thru_hole")
            )
        });
    (!holes).then(|| form_atom(footprint, "layer", 1).is_some_and(|layer| layer.starts_with("B.")))
}

fn zone_name(zone: &Expr) -> String {
    form_atom(zone, "name", 1)
        .map(|name| format!("{name:?}"))
        .or_else(|| form_atom(zone, "uuid", 1).map(|uuid| format!("({uuid})")))
        .unwrap_or_default()
}

const POINT_HEADS: [&str; 6] = ["at", "start", "end", "mid", "center", "xy"];

/// Moves every point form below `node` (`at`, `start`, `end`, `mid`,
/// `center`, `xy`) whose coordinate along `axis` passes `moves` by
/// `shift` along `axis`.
fn shift_points(
    node: &mut Expr,
    axis: usize,
    shift: f64,
    moves: &dyn Fn(f64) -> bool,
    changed: &mut bool,
) {
    let Expr::List(items) = node else {
        return;
    };
    let head = items.first().and_then(Expr::atom).unwrap_or("");
    if POINT_HEADS.contains(&head) {
        if let Some(value) = items
            .get(1 + axis)
            .and_then(Expr::atom)
            .and_then(|atom| atom.parse::<f64>().ok())
            && moves(value)
        {
            items[1 + axis] = number(value + shift);
            *changed = true;
        }
        return;
    }
    for item in items.iter_mut().skip(1) {
        shift_points(item, axis, shift, moves, changed);
    }
}

/// The point that decides whether a rigid item moves: its `at`, a
/// circle's centre, an arc's middle, else its first point.
fn reference_point(item: &Expr) -> Option<[f64; 2]> {
    fn find(node: &Expr, head: &str) -> Option<[f64; 2]> {
        if node.head() == Some(head) {
            let value = |index: usize| {
                node.children()
                    .get(index)
                    .and_then(Expr::atom)
                    .and_then(|atom| atom.parse::<f64>().ok())
            };
            return Some([value(1)?, value(2)?]);
        }
        node.children()
            .iter()
            .skip(1)
            .find_map(|child| find(child, head))
    }
    ["at", "center", "mid", "xy", "start"]
        .iter()
        .find_map(|head| find(item, head))
}

/// The box of every point form below `node`.
fn points_box(node: &Expr) -> Option<Box4> {
    fn collect(node: &Expr, bounds: &mut Box4) {
        if let Some(head) = node.head()
            && POINT_HEADS.contains(&head)
        {
            let value = |index: usize| {
                node.children()
                    .get(index)
                    .and_then(Expr::atom)
                    .and_then(|atom| atom.parse::<f64>().ok())
            };
            if let (Some(x), Some(y)) = (value(1), value(2)) {
                *bounds = [
                    bounds[0].min(x),
                    bounds[1].min(y),
                    bounds[2].max(x),
                    bounds[3].max(y),
                ];
            }
            return;
        }
        for child in node.children().iter().skip(1) {
            collect(child, bounds);
        }
    }
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    collect(node, &mut bounds);
    bounds[0].is_finite().then_some(bounds)
}

/// One layout of the search (or a size that needed none), as
/// `free-dimension.json` lists it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct KiCadFreeDimensionRun {
    pub size_mm: f64,
    /// `probe` (the reduced budget), `confirm` (the full budget at the
    /// chosen size), `closest` (the full budget at the size that came
    /// closest, when no probe was clean) or `infeasible` (no run).
    pub kind: String,
    /// The run's work budget (`total_seconds`).
    pub budget_seconds: f64,
    /// The verdict's status and exit code.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    /// Clean by the search's `accept`.
    pub clean: bool,
    /// The verdict is complete (exit 0).
    pub complete: bool,
    pub kicad_unconnected: Option<usize>,
    pub open_connections: Option<usize>,
    /// KiCad errors that count (the strict rule: not the reference's at
    /// the same place), in all and by type.
    pub errors: usize,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub errors_by_type: BTreeMap<String, usize>,
    /// Errors by the count rule (`new_findings`).
    pub errors_count_rule: usize,
    /// No error counts under the strict rule.
    pub strict: bool,
    pub constraints_missed: usize,
    pub unplaced: usize,
    pub work_seconds: f64,
    pub wall_seconds: f64,
    pub chosen: bool,
    /// Why there was no run (`infeasible`), or what went wrong.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    /// The run's layout directory (its inputs beside it).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub directory: String,
}

impl KiCadFreeDimensionRun {
    /// A run's outcome from its `verdict.json` (`None`: the run left no
    /// verdict, as a crash or a kill).
    pub fn from_verdict(
        size_mm: f64,
        kind: &str,
        budget_seconds: f64,
        verdict: Option<&serde_json::Value>,
        accept: &str,
    ) -> Self {
        let mut run = Self {
            size_mm,
            kind: kind.into(),
            budget_seconds,
            ..Self::default()
        };
        let Some(verdict) = verdict else {
            run.status = "no_verdict".into();
            run.work_seconds = budget_seconds;
            run.reasons.push("the layout left no verdict".into());
            return run;
        };
        let count = |key: &str| -> BTreeMap<String, usize> {
            verdict[key]
                .as_object()
                .map(|map| {
                    map.iter()
                        .map(|(kind, count)| (kind.clone(), count.as_u64().unwrap_or(0) as usize))
                        .filter(|(_, count)| *count > 0)
                        .collect()
                })
                .unwrap_or_default()
        };
        let length = |key: &str| verdict[key].as_array().map_or(0, Vec::len);
        run.status = verdict["status"].as_str().unwrap_or("").to_string();
        run.exit_code = verdict["exit_code"].as_i64();
        run.complete = run.status == "complete";
        run.kicad_unconnected = verdict["kicad_unconnected_items"]
            .as_u64()
            .map(|count| count as usize);
        run.open_connections = verdict["open_connections"]
            .as_u64()
            .map(|count| count as usize);
        run.errors_by_type = count("new_findings_strict");
        run.errors = run.errors_by_type.values().sum();
        run.errors_count_rule = count("new_findings").values().sum();
        let checked = verdict["kicad_checked"].as_bool().unwrap_or(false);
        run.strict = checked && run.errors == 0;
        run.constraints_missed = length("constraints_missed");
        run.unplaced = length("unplaced");
        run.work_seconds = verdict["work_seconds"].as_f64().unwrap_or(0.0);
        run.wall_seconds = verdict["wall_seconds"].as_f64().unwrap_or(0.0);
        run.clean = match accept {
            "pass" => {
                checked
                    && run.kicad_unconnected == Some(0)
                    && run.errors_count_rule == 0
                    && run.unplaced == 0
                    && matches!(run.status.as_str(), "complete" | "incomplete")
            }
            _ => run.complete,
        };
        if let Some(error) = verdict["error"].as_str() {
            run.reasons
                .push(error.lines().next().unwrap_or("").to_string());
        }
        run
    }

    /// How far a run is from clean, for the closest size: parts without
    /// a place, KiCad's unconnected items, errors, missed constraints;
    /// then the smaller size, then the larger budget.
    fn distance(&self) -> (usize, usize, usize, usize, i64, i64) {
        (
            usize::from(!self.is_run()) + self.unplaced,
            self.kicad_unconnected.unwrap_or(usize::MAX / 2),
            self.errors,
            self.constraints_missed,
            (self.size_mm * 1000.0).round() as i64,
            -(self.budget_seconds * 1000.0).round() as i64,
        )
    }

    fn is_run(&self) -> bool {
        self.kind != "infeasible" && self.status != "no_verdict"
    }

    /// One line: size, budget, outcome.
    pub fn summary(&self) -> String {
        if self.kind == "infeasible" {
            return format!(
                "{} mm: infeasible ({})",
                format_mm(self.size_mm),
                self.reasons.join("; ")
            );
        }
        let unconnected = self
            .kicad_unconnected
            .map_or("KiCad did not check".to_string(), |count| {
                format!("{count} KiCad unconnected")
            });
        format!(
            "{} mm, {} at {:.0} s: {}{}, {} errors{}, {:.0} s of work, {:.0} s wall",
            format_mm(self.size_mm),
            self.kind,
            self.budget_seconds,
            if self.clean { "clean, " } else { "" },
            unconnected,
            self.errors,
            if self.constraints_missed > 0 || self.unplaced > 0 {
                format!(
                    ", {} constraints missed, {} unplaced",
                    self.constraints_missed, self.unplaced
                )
            } else {
                String::new()
            },
            self.work_seconds,
            self.wall_seconds
        )
    }
}

/// A size as the reports write it: `40`, `40.5`.
pub fn format_mm(size: f64) -> String {
    let rounded = round_mm(size);
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        rounded.to_string()
    }
}

/// What the search does next.
#[derive(Clone, Debug, PartialEq)]
pub enum KiCadFreeDimensionStep {
    /// Lay out these sizes side by side at the probe budget.
    Probe {
        sizes: Vec<f64>,
        budget: f64,
    },
    /// Lay out one size at a larger budget: `confirm` (a clean probe's
    /// size) or `closest` (no probe was clean).
    Full {
        size: f64,
        budget: f64,
        kind: &'static str,
    },
    Done,
}

/// The search over the sizes: probes from the smallest size upward, in
/// batches of `parallel`, while the budget leaves room for the full run;
/// then the full run at the first clean size, or at the size that came
/// closest. Every run's work counts against `total_seconds`.
#[derive(Clone, Debug)]
pub struct KiCadFreeDimensionSearch {
    pending: VecDeque<f64>,
    pub probe_seconds: f64,
    pub confirm_seconds: f64,
    pub total_seconds: f64,
    pub parallel: usize,
    pub runs: Vec<KiCadFreeDimensionRun>,
    full_done: bool,
}

impl KiCadFreeDimensionSearch {
    /// `sizes`: the feasible sizes; `runs`: the infeasible ones, listed.
    pub fn new(
        sizes: Vec<f64>,
        infeasible: Vec<KiCadFreeDimensionRun>,
        probe_seconds: f64,
        confirm_seconds: f64,
        total_seconds: f64,
        parallel: usize,
    ) -> Self {
        let mut sizes = sizes;
        sizes.sort_by(f64::total_cmp);
        Self {
            pending: sizes.into(),
            probe_seconds,
            confirm_seconds,
            total_seconds,
            parallel: parallel.max(1),
            runs: infeasible,
            full_done: false,
        }
    }

    /// The budgets: `confirm_seconds` defaults to a third of the total,
    /// `probe_seconds` to a third of that; a probe and the confirmation
    /// must fit the total.
    pub fn budgets(free: &KiCadFreeDimension, total_seconds: f64) -> Result<(f64, f64), String> {
        let confirm = free.confirm_seconds.unwrap_or(total_seconds / 3.0);
        let probe = free.probe_seconds.unwrap_or(confirm / 3.0);
        if !(probe > 0.0
            && confirm > 0.0
            && probe <= confirm
            && probe + confirm <= total_seconds + 1.0e-6)
        {
            return Err(input_error(format!(
                "free_dimension: probes of {probe} s and a confirmation of {confirm} s in total_seconds {total_seconds}: a probe at most the confirmation, both within the total (total_seconds is the whole search's budget)"
            )));
        }
        Ok((probe, confirm))
    }

    pub fn spent(&self) -> f64 {
        self.runs
            .iter()
            .filter(|run| run.kind != "infeasible")
            .map(|run| run.work_seconds)
            .sum()
    }

    /// Sizes the budget left untried.
    pub fn untried(&self) -> Vec<f64> {
        self.pending.iter().copied().collect()
    }

    fn first_clean_probe(&self) -> Option<&KiCadFreeDimensionRun> {
        self.runs
            .iter()
            .filter(|run| run.kind == "probe" && run.clean)
            .min_by(|a, b| a.size_mm.total_cmp(&b.size_mm))
    }

    fn closest(&self) -> Option<&KiCadFreeDimensionRun> {
        self.runs
            .iter()
            .filter(|run| run.is_run())
            .min_by_key(|run| run.distance())
    }

    pub fn next_step(&mut self) -> KiCadFreeDimensionStep {
        if self.full_done {
            return KiCadFreeDimensionStep::Done;
        }
        let left = self.total_seconds - self.spent();
        if let Some(clean) = self.first_clean_probe() {
            let budget = self.confirm_seconds.min(left);
            if budget > clean.budget_seconds + 1.0e-6 {
                return KiCadFreeDimensionStep::Full {
                    size: clean.size_mm,
                    budget,
                    kind: "confirm",
                };
            }
            return KiCadFreeDimensionStep::Done;
        }
        let room = left - self.confirm_seconds;
        let count = ((room + 1.0e-6) / self.probe_seconds).floor().max(0.0) as usize;
        let count = count.min(self.parallel).min(self.pending.len());
        if count > 0 {
            return KiCadFreeDimensionStep::Probe {
                sizes: self.pending.drain(..count).collect(),
                budget: self.probe_seconds,
            };
        }
        if let Some(closest) = self.closest() {
            let budget = self.confirm_seconds.min(left);
            if budget > closest.budget_seconds + 1.0e-6 {
                return KiCadFreeDimensionStep::Full {
                    size: closest.size_mm,
                    budget,
                    kind: "closest",
                };
            }
        }
        KiCadFreeDimensionStep::Done
    }

    pub fn record(&mut self, runs: Vec<KiCadFreeDimensionRun>) {
        if runs
            .iter()
            .any(|run| run.kind == "confirm" || run.kind == "closest")
        {
            self.full_done = true;
        }
        self.runs.extend(runs);
    }

    /// The run chosen (its index in `runs`) and why: the smallest clean
    /// size (its full run when that is clean too, else its probe); without
    /// one, the run that came closest. Marks it `chosen`.
    pub fn choose(&mut self, free: &KiCadFreeDimension) -> Option<(usize, String)> {
        let budget_order =
            |run: &KiCadFreeDimensionRun| -(run.budget_seconds * 1000.0).round() as i64;
        let clean = self
            .runs
            .iter()
            .enumerate()
            .filter(|(_, run)| run.clean && run.is_run())
            .min_by_key(|(_, run)| ((run.size_mm * 1000.0).round() as i64, budget_order(run)))
            .map(|(index, _)| index);
        let probed: Vec<&KiCadFreeDimensionRun> =
            self.runs.iter().filter(|run| run.kind == "probe").collect();
        let (index, reason) = match clean {
            Some(index) => {
                let run = &self.runs[index];
                let smaller: Vec<String> =
                    probed
                        .iter()
                        .filter(|other| other.size_mm < run.size_mm - 1.0e-6)
                        .map(|other| {
                            format!(
                                "{} mm {}",
                                format_mm(other.size_mm),
                                other.kicad_unconnected.map_or(
                                    "unchecked".into(),
                                    |count| format!("{count} open, {} errors", other.errors)
                                )
                            )
                        })
                        .collect();
                let mut reason = format!(
                    "{} mm is the smallest clean size ({}) of those probed at {:.0} s",
                    format_mm(run.size_mm),
                    if run.kind == "probe" {
                        "its probe"
                    } else {
                        "confirmed at the full budget"
                    },
                    self.probe_seconds
                );
                if !smaller.is_empty() {
                    reason.push_str(&format!("; smaller: {}", smaller.join(", ")));
                }
                if run.kind == "probe"
                    && let Some(full) = self.runs.iter().find(|other| {
                        other.kind == "confirm" && (other.size_mm - run.size_mm).abs() < 1.0e-6
                    })
                {
                    reason.push_str(&format!(
                        "; its confirmation at {:.0} s was not clean ({} open, {} errors): the probe's result is kept",
                        full.budget_seconds,
                        full.kicad_unconnected.unwrap_or(0),
                        full.errors
                    ));
                }
                let band = free.price_band(run.size_mm);
                if band > 0 {
                    let threshold = free
                        .prefer_below
                        .iter()
                        .copied()
                        .filter(|threshold| *threshold < run.size_mm)
                        .fold(f64::NEG_INFINITY, f64::max);
                    reason.push_str(&format!(
                        "; above the {} mm price threshold: no size at or below it was clean",
                        format_mm(threshold)
                    ));
                }
                (index, reason)
            }
            None => {
                let index = self
                    .runs
                    .iter()
                    .enumerate()
                    .filter(|(_, run)| run.is_run())
                    .min_by_key(|(_, run)| run.distance())
                    .map(|(index, _)| index)?;
                let run = &self.runs[index];
                let mut reason = format!(
                    "no size was clean: {} mm came closest ({} KiCad unconnected, {} errors, at {:.0} s)",
                    format_mm(run.size_mm),
                    run.kicad_unconnected
                        .map_or("unchecked".into(), |count| count.to_string()),
                    run.errors,
                    run.budget_seconds
                );
                let untried = self.untried();
                if !untried.is_empty() {
                    reason.push_str(&format!(
                        "; the budget left {} sizes untried ({} to {} mm)",
                        untried.len(),
                        format_mm(untried[0]),
                        format_mm(untried[untried.len() - 1])
                    ));
                }
                (index, reason)
            }
        };
        self.runs[index].chosen = true;
        Some((index, reason))
    }
}

/// Writes a run's project: the source project's files (an output
/// directory inside it left out) with the board at the run's size.
pub fn write_probe_project(
    source_directory: &Path,
    board_id: &str,
    destination: &Path,
    board_text: &str,
) -> Result<(), String> {
    copy_directory_tree(source_directory, destination)?;
    let board = destination.join(format!("{board_id}.kicad_pcb"));
    fs::write(&board, board_text)
        .map_err(|error| format!("failed to write {}: {error}", board.display()))
}

/// Starts a probe's process with its output in `log`, killed with this
/// process on Linux.
pub fn spawn_probe(command: &mut Command, log: &Path) -> Result<std::process::Child, String> {
    let file = fs::File::create(log)
        .map_err(|error| format!("failed to create {}: {error}", log.display()))?;
    let error_file = file.try_clone().map_err(|error| error.to_string())?;
    command.stdout(file).stderr(error_file);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: prctl is async-signal-safe and touches no memory.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
    }
    command
        .spawn()
        .map_err(|error| format!("failed to start a probe: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free(edge: &str) -> KiCadFreeDimension {
        KiCadFreeDimension {
            edge: edge.into(),
            min: Some(30.0),
            max: Some(40.0),
            step: Some(2.0),
            sizes: Vec::new(),
            prefer_below: Vec::new(),
            accept: None,
            band_mm: None,
            probe_seconds: None,
            confirm_seconds: None,
            parallel: None,
        }
    }

    /// A 50 x 30 mm board at (100, 100) with a fixed part near the bottom,
    /// a free part in the middle, a label at the bottom edge, a keepout
    /// rule area at the bottom and a pour over the whole board.
    const BOARD: &str = r#"(kicad_pcb (version 20240108)
  (footprint "R" (layer "F.Cu") (at 120 127 0)
    (property "Reference" "J1" (at 0 0 0) (layer "F.SilkS"))
    (fp_line (start -2 -1) (end 2 -1) (layer "F.CrtYd"))
    (fp_line (start 2 -1) (end 2 1) (layer "F.CrtYd"))
    (fp_line (start 2 1) (end -2 1) (layer "F.CrtYd"))
    (fp_line (start -2 1) (end -2 -1) (layer "F.CrtYd"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
  (footprint "R" (layer "F.Cu") (at 120 112 0)
    (property "Reference" "U1" (at 0 0 0) (layer "F.SilkS"))
    (fp_line (start -2 -1) (end 2 -1) (layer "F.CrtYd"))
    (fp_line (start 2 1) (end -2 1) (layer "F.CrtYd"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
  (gr_rect (start 100 100) (end 150 130) (stroke (width 0.1) (type solid)) (fill no) (layer "Edge.Cuts"))
  (gr_text "LABEL" (at 110 129.5 0) (layer "F.SilkS"))
  (gr_text "TOP" (at 110 101 0) (layer "F.SilkS"))
  (zone (net 0) (net_name "") (layer "F.Cu") (name "rule") (keepout (tracks not_allowed))
    (polygon (pts (xy 140 126) (xy 148 126) (xy 148 129) (xy 140 129))))
  (zone (net 2) (net_name "GND") (layer "B.Cu")
    (polygon (pts (xy 100 100) (xy 150 100) (xy 150 130) (xy 100 130))))
)"#;

    fn plan(
        free: &KiCadFreeDimension,
        constraints: Option<KiCadPlacementConstraints>,
    ) -> KiCadFreeDimensionPlan {
        KiCadFreeDimensionPlan::new(free, BOARD, constraints, &["J1".into()], &[]).unwrap()
    }

    fn footprint_at(board: &str, reference: &str) -> [f64; 3] {
        let pcb = parse(board).unwrap();
        let footprint = pcb
            .children()
            .iter()
            .find(|item| {
                item.head() == Some("footprint")
                    && footprint_reference(item).as_deref() == Some(reference)
            })
            .unwrap();
        form_at(footprint).unwrap()
    }

    #[test]
    fn ladder_steps_to_max_and_inserts_thresholds() {
        let mut dimension = free("bottom");
        dimension.min = Some(80.0);
        dimension.max = Some(120.0);
        dimension.step = Some(15.0);
        dimension.prefer_below = vec![100.0, 200.0];
        assert_eq!(
            dimension.ladder().unwrap(),
            vec![80.0, 95.0, 100.0, 110.0, 120.0]
        );
        dimension.step = Some(2.0);
        dimension.min = Some(34.0);
        dimension.max = Some(40.0);
        assert_eq!(dimension.ladder().unwrap(), vec![34.0, 36.0, 38.0, 40.0]);
        dimension.sizes = vec![46.0, 34.0, 40.0];
        assert!(dimension.ladder().is_err(), "sizes and a range at once");
        dimension.min = None;
        dimension.max = None;
        dimension.step = None;
        assert_eq!(dimension.ladder().unwrap(), vec![34.0, 40.0, 46.0]);
        assert_eq!(dimension.price_band(100.0), 0);
        assert_eq!(dimension.price_band(100.5), 1);
        dimension.edge = "middle".into();
        assert!(dimension.axis_and_sign().is_err());
    }

    #[test]
    fn bottom_edge_moves_the_outline_and_what_lies_in_its_band() {
        let mut dimension = free("bottom");
        dimension.band_mm = Some(4.0);
        let plan = plan(&dimension, None);
        assert_eq!(plan.extent_mm, 30.0);
        let variant = plan.variant(36.0).unwrap();
        assert_eq!(variant.outline, [100.0, 100.0, 150.0, 136.0]);
        let pcb = parse(&variant.board).unwrap();
        let rect = pcb.child("gr_rect").unwrap();
        assert_eq!(form_xy(rect, "start").unwrap(), [100.0, 100.0]);
        assert_eq!(form_xy(rect, "end").unwrap(), [150.0, 136.0]);
        // J1 (3 mm from the edge) moves, U1 (18 mm) stays.
        assert_eq!(footprint_at(&variant.board, "J1"), [120.0, 133.0, 0.0]);
        assert_eq!(footprint_at(&variant.board, "U1"), [120.0, 112.0, 0.0]);
        assert_eq!(variant.moved_footprints, vec!["J1".to_string()]);
        // The label at the edge moves, the one at the top stays.
        let texts: Vec<[f64; 3]> = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("gr_text"))
            .map(|item| form_at(item).unwrap())
            .collect();
        assert_eq!(texts, vec![[110.0, 135.5, 0.0], [110.0, 101.0, 0.0]]);
        // The rule area moves whole; the pour stretches with the board.
        let zones: Vec<Box4> = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("zone"))
            .map(|item| points_box(item).unwrap())
            .collect();
        assert_eq!(
            zones,
            vec![[140.0, 132.0, 148.0, 135.0], [100.0, 100.0, 150.0, 136.0]]
        );
    }

    #[test]
    fn the_default_band_moves_only_what_lies_on_the_edge() {
        let plan = plan(&free("bottom"), None);
        let variant = plan.variant(34.0).unwrap();
        assert_eq!(footprint_at(&variant.board, "J1"), [120.0, 127.0, 0.0]);
        let pcb = parse(&variant.board).unwrap();
        let label = pcb.child("gr_text").unwrap();
        // 0.5 mm from the edge: on it.
        assert_eq!(form_at(label).unwrap(), [110.0, 133.5, 0.0]);
        // The rule area's lower side is 1 mm off the edge: it stays.
        let rule = pcb.child("zone").unwrap();
        assert_eq!(points_box(rule).unwrap(), [140.0, 126.0, 148.0, 129.0]);
    }

    #[test]
    fn top_edge_moves_the_corner_and_re_expresses_relative_boxes() {
        let constraints: KiCadPlacementConstraints = serde_json::from_value(serde_json::json!({
            "version": 1,
            "keepout": [{"x": [5, 10], "y": [0, 2]}, {"x": [5, 10], "y": [20, 25]},
                        {"x": [105, 110], "y": [125, 128], "origin": "absolute"}],
            "region": [{"parts": ["U1"], "x": [0, 50], "y": [0, 10]}],
            "place": [{"part": "U1", "x": 20, "y": 1}],
            "edge": [{"part": "J1", "edge": "top"}]
        }))
        .unwrap();
        let mut dimension = free("top");
        dimension.band_mm = Some(3.0);
        let plan = plan(&dimension, Some(constraints));
        let variant = plan.variant(40.0).unwrap();
        assert_eq!(variant.outline, [100.0, 90.0, 150.0, 130.0]);
        let constraints = variant.constraints.unwrap();
        // At the top edge: moves with it, same place relative to the corner.
        assert_eq!(constraints.keepout[0].y, [0.0, 2.0]);
        // Farther in: stays in board coordinates, 10 mm lower relative.
        assert_eq!(constraints.keepout[1].y, [30.0, 35.0]);
        // Absolute and far from the edge: unchanged.
        assert_eq!(constraints.keepout[2].y, [125.0, 128.0]);
        // A region from the top edge: its top side is on the edge, it moves.
        assert_eq!(constraints.region[0].y, Some([0.0, 10.0]));
        assert_eq!(constraints.place[0].y, Some(1.0));
        assert_eq!(variant.moved_constraints, 3);
        let pcb = parse(&variant.board).unwrap();
        let rect = pcb.child("gr_rect").unwrap();
        assert_eq!(form_xy(rect, "start").unwrap(), [100.0, 90.0]);
        // The top label moves up with the edge.
        let top = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("gr_text"))
            .nth(1)
            .unwrap();
        assert_eq!(form_at(top).unwrap(), [110.0, 91.0, 0.0]);
    }

    #[test]
    fn a_constraints_outline_is_resized() {
        let constraints: KiCadPlacementConstraints = serde_json::from_value(serde_json::json!({
            "version": 1, "outline": {"width": 60, "height": 40, "corner_radius": 2}
        }))
        .unwrap();
        let plan = plan(&free("right"), Some(constraints));
        // The corner is the board's own outline's.
        assert_eq!(plan.outline, [100.0, 100.0, 160.0, 140.0]);
        let variant = plan.variant(70.0).unwrap();
        let outline = variant.constraints.unwrap().outline.unwrap();
        assert_eq!(
            (outline.x, outline.y, outline.width, outline.height),
            (Some(100.0), Some(100.0), Some(70.0), Some(40.0))
        );
        assert_eq!(outline.corner_radius, Some(2.0));
    }

    #[test]
    fn shapes_that_are_not_rectangles_are_refused() {
        let polygon: KiCadPlacementConstraints = serde_json::from_value(serde_json::json!({
            "version": 1, "outline": {"polygon": [[0, 0], [10, 0], [10, 10]]}
        }))
        .unwrap();
        let error = KiCadFreeDimensionPlan::new(&free("bottom"), BOARD, Some(polygon), &[], &[])
            .unwrap_err();
        assert!(error.contains("rectangular"), "{error}");
        let automatic: KiCadPlacementConstraints =
            serde_json::from_value(serde_json::json!({"version": 1, "outline": {}})).unwrap();
        let error = KiCadFreeDimensionPlan::new(&free("bottom"), BOARD, Some(automatic), &[], &[])
            .unwrap_err();
        assert!(error.contains("width and a height"), "{error}");
        let arc = BOARD.replace(
            "(gr_rect (start 100 100) (end 150 130)",
            "(gr_arc (start 100 100) (mid 125 90) (end 150 100) (layer \"Edge.Cuts\")) (gr_line (start 150 100) (end 150 130) (layer \"Edge.Cuts\")) (gr_line (start 150 130) (end 100 130) (layer \"Edge.Cuts\")) (gr_line (start 100 130) (end 100 100)",
        );
        let error = KiCadFreeDimensionPlan::new(&free("bottom"), &arc, None, &[], &[]).unwrap_err();
        assert!(error.contains("gr_arc"), "{error}");
        // Four lines along a box are a rectangle.
        let lines = BOARD.replace(
            "(gr_rect (start 100 100) (end 150 130)",
            "(gr_line (start 100 100) (end 150 100) (layer \"Edge.Cuts\")) (gr_line (start 150 100) (end 150 130) (layer \"Edge.Cuts\")) (gr_line (start 150 130) (end 100 130) (layer \"Edge.Cuts\")) (gr_line (start 100 130) (end 100 100)",
        );
        let plan = KiCadFreeDimensionPlan::new(&free("bottom"), &lines, None, &[], &[]).unwrap();
        let variant = plan.variant(32.0).unwrap();
        let pcb = parse(&variant.board).unwrap();
        let ends: Vec<[f64; 2]> = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("gr_line"))
            .map(|item| form_xy(item, "end").unwrap())
            .collect();
        assert_eq!(
            ends,
            vec![
                [150.0, 100.0],
                [150.0, 132.0],
                [100.0, 132.0],
                [100.0, 100.0]
            ]
        );
        let mut wide = free("bottom");
        wide.band_mm = Some(30.0);
        assert!(KiCadFreeDimensionPlan::new(&wide, BOARD, None, &[], &[]).is_err());
    }

    #[test]
    fn sizes_that_strand_fixed_parts_or_rule_areas_are_infeasible() {
        let plan = plan(&free("bottom"), None);
        // J1's body reaches 128 mm; at 27 mm the edge is at 127.
        let reasons = plan.variant(27.0).unwrap_err();
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("fixed part J1")),
            "{reasons:?}"
        );
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("rule area \"rule\"")),
            "{reasons:?}"
        );
        // U1 is not fixed: it is placed again.
        assert!(plan.variant(29.0).is_ok());
        // Carried by the edge onto a fixed part that stays.
        let mut carried = free("bottom");
        carried.band_mm = Some(4.0);
        let plan =
            KiCadFreeDimensionPlan::new(&carried, BOARD, None, &["J1".into(), "U1".into()], &[])
                .unwrap();
        let reasons = plan.variant(15.5).unwrap_err();
        assert!(
            reasons.iter().any(|reason| reason
                .contains("J1 (moving with the edge) would overlap the fixed part U1")),
            "{reasons:?}"
        );
        // A keepout constraint that would leave the board.
        let constraints: KiCadPlacementConstraints = serde_json::from_value(serde_json::json!({
            "version": 1, "keepout": [{"x": [5, 10], "y": [20, 25]}]
        }))
        .unwrap();
        let plan = KiCadFreeDimensionPlan::new(&free("bottom"), BOARD, Some(constraints), &[], &[])
            .unwrap();
        let reasons = plan.variant(24.0).unwrap_err();
        assert!(
            reasons.iter().any(|reason| reason.contains("keepout box")),
            "{reasons:?}"
        );
    }

    fn outcome(
        size: f64,
        kind: &str,
        budget: f64,
        open: usize,
        errors: usize,
    ) -> KiCadFreeDimensionRun {
        KiCadFreeDimensionRun {
            size_mm: size,
            kind: kind.into(),
            budget_seconds: budget,
            status: if open == 0 && errors == 0 {
                "complete"
            } else {
                "incomplete"
            }
            .into(),
            clean: open == 0 && errors == 0,
            complete: open == 0 && errors == 0,
            kicad_unconnected: Some(open),
            errors,
            work_seconds: budget,
            ..Default::default()
        }
    }

    /// Runs the search over synthetic results: `result(size, budget)`
    /// gives a run's open connections and errors.
    fn drive(
        search: &mut KiCadFreeDimensionSearch,
        result: impl Fn(f64, f64) -> (usize, usize),
    ) -> Vec<KiCadFreeDimensionStep> {
        let mut steps = Vec::new();
        loop {
            let step = search.next_step();
            steps.push(step.clone());
            match step {
                KiCadFreeDimensionStep::Probe { sizes, budget } => {
                    let runs = sizes
                        .iter()
                        .map(|size| {
                            let (open, errors) = result(*size, budget);
                            outcome(*size, "probe", budget, open, errors)
                        })
                        .collect();
                    search.record(runs);
                }
                KiCadFreeDimensionStep::Full { size, budget, kind } => {
                    let (open, errors) = result(size, budget);
                    search.record(vec![outcome(size, kind, budget, open, errors)]);
                }
                KiCadFreeDimensionStep::Done => return steps,
            }
        }
    }

    #[test]
    fn the_smallest_clean_size_wins_over_a_non_monotone_sequence() {
        // 34 open, 36 clean, 38 open again, 40 clean: not monotone.
        let mut search = KiCadFreeDimensionSearch::new(
            vec![34.0, 36.0, 38.0, 40.0, 42.0],
            Vec::new(),
            100.0,
            300.0,
            1500.0,
            4,
        );
        let steps = drive(&mut search, |size, _| match size as i64 {
            34 => (12, 0),
            36 => (0, 0),
            38 => (2, 0),
            _ => (0, 0),
        });
        assert_eq!(
            steps,
            vec![
                KiCadFreeDimensionStep::Probe {
                    sizes: vec![34.0, 36.0, 38.0, 40.0],
                    budget: 100.0
                },
                KiCadFreeDimensionStep::Full {
                    size: 36.0,
                    budget: 300.0,
                    kind: "confirm"
                },
                KiCadFreeDimensionStep::Done
            ]
        );
        let dimension = free("bottom");
        let (index, reason) = search.choose(&dimension).unwrap();
        assert_eq!(
            (search.runs[index].size_mm, search.runs[index].kind.as_str()),
            (36.0, "confirm")
        );
        assert!(
            reason.contains("36 mm is the smallest clean size"),
            "{reason}"
        );
        assert!(reason.contains("34 mm 12 open"), "{reason}");
    }

    #[test]
    fn a_failed_confirmation_keeps_the_clean_probe() {
        let mut search =
            KiCadFreeDimensionSearch::new(vec![34.0, 36.0], Vec::new(), 100.0, 300.0, 1500.0, 2);
        drive(&mut search, |size, budget| {
            if size == 36.0 && budget < 200.0 {
                (0, 0)
            } else {
                (3, 1)
            }
        });
        let (index, reason) = search.choose(&free("bottom")).unwrap();
        assert_eq!(
            (search.runs[index].size_mm, search.runs[index].kind.as_str()),
            (36.0, "probe")
        );
        assert!(reason.contains("the probe's result is kept"), "{reason}");
        assert!(search.runs[index].chosen);
    }

    #[test]
    fn without_a_clean_size_the_closest_gets_the_full_budget() {
        // Budget: 1000 in all, 400 for the full run: six probes of 100.
        let mut search = KiCadFreeDimensionSearch::new(
            (0..10).map(|step| 34.0 + 2.0 * step as f64).collect(),
            Vec::new(),
            100.0,
            400.0,
            1000.0,
            4,
        );
        let steps = drive(&mut search, |size, budget| {
            let open = match size as i64 {
                34 => 9,
                36 => 4,
                38 => 1,
                40 => 1,
                _ => 2,
            };
            (if budget > 200.0 { open - 1 } else { open }, 0)
        });
        assert_eq!(steps.len(), 4, "{steps:?}");
        assert_eq!(
            steps[1],
            KiCadFreeDimensionStep::Probe {
                sizes: vec![42.0, 44.0],
                budget: 100.0
            }
        );
        assert_eq!(
            steps[2],
            KiCadFreeDimensionStep::Full {
                size: 38.0,
                budget: 400.0,
                kind: "closest"
            }
        );
        assert_eq!(search.untried(), vec![46.0, 48.0, 50.0, 52.0]);
        let (index, reason) = search.choose(&free("bottom")).unwrap();
        // The closest's full run made it clean.
        assert_eq!(
            (search.runs[index].size_mm, search.runs[index].kind.as_str()),
            (38.0, "closest")
        );
        assert!(reason.contains("smallest clean size"), "{reason}");
        // Nothing clean at all: the closest, with the sizes left untried.
        let mut search = KiCadFreeDimensionSearch::new(
            vec![34.0, 36.0, 38.0, 40.0],
            Vec::new(),
            100.0,
            300.0,
            600.0,
            1,
        );
        drive(&mut search, |size, _| (if size == 36.0 { 2 } else { 5 }, 1));
        let (index, reason) = search.choose(&free("bottom")).unwrap();
        assert_eq!(search.runs[index].size_mm, 36.0);
        assert!(
            reason.starts_with("no size was clean: 36 mm came closest"),
            "{reason}"
        );
        assert!(reason.contains("untried"), "{reason}");
    }

    #[test]
    fn a_price_threshold_is_exceeded_only_when_nothing_below_is_clean() {
        let mut dimension = free("right");
        dimension.min = Some(90.0);
        dimension.max = Some(110.0);
        dimension.step = Some(7.0);
        dimension.prefer_below = vec![100.0];
        let ladder = dimension.ladder().unwrap();
        assert_eq!(ladder, vec![90.0, 97.0, 100.0, 104.0, 110.0]);
        // Clean at 100 and above: 100, the threshold itself.
        let mut search =
            KiCadFreeDimensionSearch::new(ladder.clone(), Vec::new(), 100.0, 300.0, 1500.0, 2);
        drive(&mut search, |size, _| (usize::from(size < 100.0), 0));
        let (index, reason) = search.choose(&dimension).unwrap();
        assert_eq!(search.runs[index].size_mm, 100.0);
        assert!(!reason.contains("threshold"), "{reason}");
        // Clean only above it: the threshold is exceeded, and said so.
        let mut search = KiCadFreeDimensionSearch::new(ladder, Vec::new(), 100.0, 300.0, 1500.0, 2);
        drive(&mut search, |size, _| (usize::from(size <= 100.0), 0));
        let (index, reason) = search.choose(&dimension).unwrap();
        assert_eq!(search.runs[index].size_mm, 104.0);
        assert!(
            reason.contains("above the 100 mm price threshold"),
            "{reason}"
        );
    }

    #[test]
    fn budgets_and_verdicts() {
        let dimension = free("bottom");
        assert_eq!(
            KiCadFreeDimensionSearch::budgets(&dimension, 4500.0).unwrap(),
            (500.0, 1500.0)
        );
        let mut tight = free("bottom");
        tight.probe_seconds = Some(400.0);
        tight.confirm_seconds = Some(300.0);
        assert!(KiCadFreeDimensionSearch::budgets(&tight, 4500.0).is_err());
        let verdict = serde_json::json!({
            "status": "incomplete", "exit_code": 1, "kicad_checked": true,
            "kicad_unconnected_items": 0, "open_connections": 0,
            "new_findings_strict": {"clearance": 2}, "new_findings": {},
            "constraints_missed": ["near C1 U1 (1.00 mm)"], "unplaced": [],
            "work_seconds": 99.5, "wall_seconds": 120.0
        });
        let strict = KiCadFreeDimensionRun::from_verdict(
            40.0,
            "probe",
            100.0,
            Some(&verdict),
            "pass_strict",
        );
        assert!(!strict.clean && !strict.strict);
        assert_eq!(
            (
                strict.errors,
                strict.constraints_missed,
                strict.work_seconds
            ),
            (2, 1, 99.5)
        );
        let pass =
            KiCadFreeDimensionRun::from_verdict(40.0, "probe", 100.0, Some(&verdict), "pass");
        assert!(
            pass.clean,
            "pass: no unconnected, no error by the count rule"
        );
        let lost = KiCadFreeDimensionRun::from_verdict(40.0, "probe", 100.0, None, "pass");
        assert!(!lost.clean);
        assert_eq!(lost.work_seconds, 100.0);
    }
}

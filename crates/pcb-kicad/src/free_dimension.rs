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
//!   it (footprints by their origin, the parts the layout holds also by
//!   their body, board texts and other rigid items by their box, drawn
//!   shapes and zones point by point, a zone that would fold cut at the
//!   new edge, arcs by their ends, constraint boxes side by side); the
//!   rest stays where it is in board coordinates. A size where a held part,
//!   a rule area or a drawn item that lay inside the outline would end
//!   outside it, a drawn shape would fold, a box would vanish, or a held
//!   part carried by the edge would land on one that stays, is infeasible
//!   without a run;
//! - the search over the sizes (`KiCadFreeDimensionSearch`): probes at a
//!   reduced budget from the smallest size upward, several side by side;
//!   then everything left goes to full runs: a smaller size that came
//!   close, else the smallest clean size again, or without a clean size
//!   the closest one;
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
    /// (footprints by their origin, held parts also by their body, board
    /// texts and other rigid items by their box, drawn shapes and zones
    /// point by point, arcs by their ends, constraint boxes side by side).
    /// Default 0.5 mm: what lies on the edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band_mm: Option<f64>,
    /// The work budget of one probe (default a third of
    /// `confirm_seconds`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_seconds: Option<f64>,
    /// The work kept for the full runs after the probes (default a third
    /// of the layout's `total_seconds`, which is the whole search's
    /// budget): probes start only while it stays. The full runs (a smaller
    /// size retried, the confirmation, the closest size) get everything
    /// left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_seconds: Option<f64>,
    /// Probes run side by side (default: a quarter of the threads, at
    /// most 4, at most one per 3 GB of available memory). Each gets this
    /// share of the threads and of the memory (`PCB_MAKER_MEMORY_MB`).
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
    /// their pose: the parts the placer holds (`held_parts`).
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
    /// file) for `free`, with the placer config `placer` (its `fixed`,
    /// `fixed_patterns`, `free`, `edge_keep_mm`: which parts are held,
    /// `held_parts`, as the layout decides it). Refuses an outline that is
    /// not a rectangle (input error).
    pub fn new(
        free: &KiCadFreeDimension,
        board_text: &str,
        constraints: Option<KiCadPlacementConstraints>,
        placer: &crate::KiCadBoardPlacerConfig,
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
        // The parts the layout holds, decided as the layout decides it.
        let mut held_config = placer.clone();
        held_config.constraints = constraints
            .clone()
            .map(crate::placement_constraints::KiCadConstraintsSource::Inline);
        let held = crate::placement_constraints::held_parts_as_given(&pcb, &held_config).map_err(
            |error| {
                input_error(format!(
                    "free_dimension: deciding which parts stay where they are: {error}"
                ))
            },
        )?;
        let fixed = pcb
            .children()
            .iter()
            .enumerate()
            .filter(|(_, item)| item.head() == Some("footprint"))
            .map(|(index, _)| index)
            .enumerate()
            .filter(|(footprint, _)| held.holds(*footprint))
            .map(|(_, index)| index)
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

    /// The references of the parts held (for the reports and tests).
    pub fn fixed_references(&self) -> Vec<String> {
        self.fixed
            .iter()
            .map(|index| footprint_reference(&self.pcb.children()[*index]).unwrap_or_default())
            .collect()
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

    /// A box's side nearest the moving edge, along the free axis.
    fn near(&self, bounds: &Box4) -> f64 {
        if self.sign > 0.0 {
            bounds[2 + self.axis]
        } else {
            bounds[self.axis]
        }
    }

    /// Whether moving the points of an item that lie in the band by
    /// `shift` would carry one past a point that stays (a shrink past the
    /// band: a polygon would fold over itself).
    fn folds(&self, coordinates: &[f64], shift: f64) -> bool {
        if self.sign * shift >= 0.0 {
            return false;
        }
        let mut nearest_moved = f64::INFINITY;
        let mut farthest_kept = f64::NEG_INFINITY;
        for coordinate in coordinates {
            if self.in_band(*coordinate) {
                nearest_moved = nearest_moved.min(self.sign * (coordinate + shift));
            } else {
                farthest_kept = farthest_kept.max(self.sign * coordinate);
            }
        }
        nearest_moved <= farthest_kept + 1.0e-6
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
        let (variant, reasons) = self.build(size)?;
        if reasons.is_empty() {
            Ok(variant)
        } else {
            Err(reasons)
        }
    }

    /// The board at `size` whatever stands in the way (a reference board
    /// moved as the input is: its findings must stay where the input's
    /// items go).
    pub fn moved_board(&self, size: f64) -> Result<String, Vec<String>> {
        self.build(size).map(|(variant, _)| variant.board)
    }

    /// The board and constraints at `size`, and the reasons that size
    /// cannot work (none: it can).
    fn build(&self, size: f64) -> Result<(KiCadFreeDimensionVariant, Vec<String>), Vec<String>> {
        let axis = self.axis;
        let shift = round_mm(self.sign * (size - self.extent_mm));
        let new_outline = self.outline_at(size);
        let new_edge = self.edge_coordinate() + shift;
        let mut reasons = Vec::new();
        if size <= 0.0 {
            return Err(vec![format!("{size} mm is not a size")]);
        }
        let mut pcb = self.pcb.clone();
        let mut moved_footprints = Vec::new();
        let mut moved_items = 0;
        // Fixed footprints' bodies before and after, and whether they move.
        let mut bodies: Vec<(String, Box4, Box4, bool, Option<bool>)> = Vec::new();
        // Zones clipped away whole.
        let mut vanished = Vec::new();
        let Expr::List(items) = &mut pcb else {
            return Err(vec!["the board is not a list".into()]);
        };
        // Whether an item that lay on the board would end beyond the
        // outline.
        let leaves = |before: Option<Box4>, after: Option<Box4>| {
            matches!((before, after), (Some(before), Some(after))
                if inside(&before, &self.outline, 0.01) && !inside(&after, &new_outline, 0.01))
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
                    let before = footprint_box(item);
                    let fixed = self.fixed.contains(&index);
                    // A fixed part moves when its origin or its body lies in
                    // the band (a centre-origin connector at the edge, a
                    // module over it); the others, which the placer places
                    // again, by their origin.
                    let anchored = self.in_band(at[axis])
                        || (fixed && before.is_some_and(|body| self.in_band(self.near(&body))));
                    let moves = shift != 0.0 && anchored;
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
                    if fixed && let Some(before) = before {
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
                "zone" => {
                    let before = points_box(item);
                    let mut changed = false;
                    // Each outline of the zone on its own: one that would
                    // fold is cut at the new edge instead (a pour's notch
                    // just past the band; KiCad clips a pour to the board
                    // anyway).
                    let mut lists = Vec::new();
                    collect_point_lists(item, &mut Vec::new(), &mut lists);
                    let folding = lists.iter().any(|(_, coordinates)| {
                        let along: Vec<f64> = coordinates.iter().map(|point| point[axis]).collect();
                        self.folds(&along, shift)
                    });
                    if folding {
                        match clip_zone(item, axis, self.sign, new_edge) {
                            Ok(true) => vanished.push(index),
                            Ok(false) => {}
                            Err(why) => reasons.push(format!("the zone {} {why}", zone_name(item))),
                        }
                        changed = true;
                    } else if shift != 0.0 {
                        shift_points(
                            item,
                            axis,
                            shift,
                            &|coordinate| self.in_band(coordinate),
                            &mut changed,
                        );
                    }
                    if changed {
                        moved_items += 1;
                    }
                    // A rule area that lay on the board and would not.
                    if item.child("keepout").is_some() && leaves(before, points_box(item)) {
                        reasons.push(format!(
                            "the rule area {} would reach beyond the outline",
                            zone_name(item)
                        ));
                    }
                }
                "gr_line" | "gr_rect" | "gr_poly" | "gr_curve" | "segment" => {
                    let before = points_box(item);
                    let mut along = Vec::new();
                    collect_axis(item, axis, &mut along);
                    // Tracks go with the layout (it strips them); drawn
                    // shapes must not fold nor leave the board.
                    let drawn = head != "segment";
                    if drawn && self.folds(&along, shift) {
                        reasons.push(format!(
                            "the {head} {} would fold: the edge would carry its points in the band past the ones beyond it",
                            item_label(item)
                        ));
                        continue;
                    }
                    let mut changed = false;
                    if shift != 0.0 {
                        shift_points(
                            item,
                            axis,
                            shift,
                            &|coordinate| self.in_band(coordinate),
                            &mut changed,
                        );
                    }
                    if changed {
                        moved_items += 1;
                    }
                    if drawn && leaves(before, points_box(item)) {
                        reasons.push(format!(
                            "the {head} {} would end beyond the outline",
                            item_label(item)
                        ));
                    }
                }
                "gr_arc" | "arc" => {
                    let before = points_box(item);
                    if shift != 0.0
                        && move_arc(item, axis, shift, &|coordinate| self.in_band(coordinate))
                    {
                        moved_items += 1;
                    }
                    if head == "gr_arc" && leaves(before, points_box(item)) {
                        reasons.push(format!(
                            "the gr_arc {} would end beyond the outline",
                            item_label(item)
                        ));
                    }
                }
                "gr_text" | "gr_text_box" | "gr_circle" | "via" | "image" | "target"
                | "dimension" | "table" => {
                    // Rigid: by the side of its box nearest the edge (a
                    // text's box, a circle's), so that what reaches into
                    // the band moves whole.
                    let Some(before) = rigid_box(item) else {
                        continue;
                    };
                    let mut after = before;
                    if shift != 0.0 && self.in_band(self.near(&before)) {
                        let mut changed = false;
                        shift_points(item, axis, shift, &|_| true, &mut changed);
                        after[axis] += shift;
                        after[axis + 2] += shift;
                        moved_items += 1;
                    }
                    if head != "via" && leaves(Some(before), Some(after)) {
                        reasons.push(format!(
                            "the {head} {} would end beyond the outline",
                            item_label(item)
                        ));
                    }
                }
                _ => {}
            }
        }
        for index in vanished.into_iter().rev() {
            items.remove(index);
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
                // A box: each side along the free axis that lies in the
                // band moves (a box at the edge moves whole, one reaching
                // from the edge across the board stretches); one whose
                // near side would pass its far side cannot work.
                let mut moved_box = |x: &mut [f64; 2],
                                     y: &mut [f64; 2],
                                     origin: Option<&str>,
                                     what: &str| {
                    let relative = origin != Some("absolute");
                    let mut range = [*x, *y];
                    if relative {
                        for (index, range) in range.iter_mut().enumerate() {
                            range[0] += old_corner[index];
                            range[1] += old_corner[index];
                        }
                    }
                    let ordered = |b: Box4| {
                        [
                            b[0].min(b[2]),
                            b[1].min(b[3]),
                            b[0].max(b[2]),
                            b[1].max(b[3]),
                        ]
                    };
                    let before = [range[0][0], range[1][0], range[0][1], range[1][1]];
                    // The side nearest the edge, and the other.
                    let near_side =
                        usize::from((self.sign > 0.0) == (range[axis][1] >= range[axis][0]));
                    let far_side = 1 - near_side;
                    if shift != 0.0 && self.in_band(range[axis][near_side]) {
                        let far_moves = self.in_band(range[axis][far_side]);
                        range[axis][near_side] += shift;
                        if far_moves {
                            range[axis][far_side] += shift;
                        }
                        moved_constraints += 1;
                        if self.sign * (range[axis][near_side] - range[axis][far_side]) <= 1.0e-6 {
                            reasons.push(format!(
                                    "the {what} box would vanish: the edge would carry its side past its far side"
                                ));
                        }
                    }
                    let after = [range[0][0], range[1][0], range[0][1], range[1][1]];
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
        reasons.sort();
        reasons.dedup();
        Ok((
            KiCadFreeDimensionVariant {
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
            },
            reasons,
        ))
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

/// The coordinates along `axis` of every point form below `node`.
fn collect_axis(node: &Expr, axis: usize, out: &mut Vec<f64>) {
    if let Some(head) = node.head()
        && POINT_HEADS.contains(&head)
    {
        if let Some(value) = node
            .children()
            .get(1 + axis)
            .and_then(Expr::atom)
            .and_then(|atom| atom.parse::<f64>().ok())
        {
            out.push(value);
        }
        return;
    }
    for child in node.children().iter().skip(1) {
        collect_axis(child, axis, out);
    }
}

/// Every `pts` list below `node` (by its path of child indices) with its
/// `xy` points.
fn collect_point_lists(
    node: &Expr,
    path: &mut Vec<usize>,
    out: &mut Vec<(Vec<usize>, Vec<[f64; 2]>)>,
) {
    if node.head() == Some("pts") {
        let points = node
            .children()
            .iter()
            .filter(|xy| xy.head() == Some("xy"))
            .filter_map(|xy| {
                let value = |index: usize| {
                    xy.children()
                        .get(index)
                        .and_then(Expr::atom)
                        .and_then(|atom| atom.parse::<f64>().ok())
                };
                Some([value(1)?, value(2)?])
            })
            .collect();
        out.push((path.clone(), points));
        return;
    }
    for (index, child) in node.children().iter().enumerate().skip(1) {
        path.push(index);
        collect_point_lists(child, path, out);
        path.pop();
    }
}

/// `polygon` cut by the half-plane `sign * (edge - point[axis]) >= 0`
/// (the board's side of the new edge), Sutherland-Hodgman.
pub(crate) fn clip_polygon(
    polygon: &[[f64; 2]],
    axis: usize,
    sign: f64,
    edge: f64,
) -> Vec<[f64; 2]> {
    let inside = |point: &[f64; 2]| sign * (edge - point[axis]) >= -1.0e-9;
    let mut out = Vec::new();
    for (index, current) in polygon.iter().enumerate() {
        let previous = polygon[(index + polygon.len() - 1) % polygon.len()];
        let crossing = |a: [f64; 2], b: [f64; 2]| {
            let t = (edge - a[axis]) / (b[axis] - a[axis]);
            let mut point = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            point[axis] = edge;
            point
        };
        match (inside(&previous), inside(current)) {
            (true, true) => out.push(*current),
            (true, false) => out.push(crossing(previous, *current)),
            (false, true) => {
                out.push(crossing(previous, *current));
                out.push(*current);
            }
            (false, false) => {}
        }
    }
    out.dedup_by(|a, b| (a[0] - b[0]).abs() < 1.0e-6 && (a[1] - b[1]).abs() < 1.0e-6);
    out
}

/// Cuts every outline of a zone (its `polygon` and its fills) at the new
/// edge. `Ok(true)`: nothing of the zone's outline is left (the zone goes);
/// `Err`: an outline with arcs, which is not cut.
fn clip_zone(zone: &mut Expr, axis: usize, sign: f64, edge: f64) -> Result<bool, String> {
    let mut lists = Vec::new();
    collect_point_lists(zone, &mut Vec::new(), &mut lists);
    for (path, _) in &lists {
        let node = path
            .iter()
            .fold(&*zone, |node, index| &node.children()[*index]);
        if node
            .children()
            .iter()
            .skip(1)
            .any(|child| child.head() != Some("xy"))
        {
            return Err(
                "would fold, and its outline has arcs: it cannot be cut at the new edge".into(),
            );
        }
    }
    let mut gone = false;
    let mut empty_fills = Vec::new();
    for (path, points) in lists {
        let clipped = clip_polygon(&points, axis, sign, edge);
        let Some(node) = path_node(zone, &path) else {
            continue;
        };
        let Expr::List(items) = node else {
            continue;
        };
        items.truncate(1);
        for point in &clipped {
            items.push(Expr::List(vec![
                Expr::Atom("xy".into()),
                number(point[0]),
                number(point[1]),
            ]));
        }
        if clipped.len() < 3 {
            // The zone's own outline gone: the zone goes; a fill gone:
            // that fill goes.
            let owner = path.first().map(|index| zone.children()[*index].head());
            if owner == Some(Some("polygon")) {
                gone = true;
            } else if let Some(first) = path.first() {
                empty_fills.push(*first);
            }
        }
    }
    if let Expr::List(items) = zone {
        empty_fills.sort_unstable();
        empty_fills.dedup();
        for index in empty_fills.into_iter().rev() {
            items.remove(index);
        }
    }
    Ok(gone)
}

fn path_node<'a>(node: &'a mut Expr, path: &[usize]) -> Option<&'a mut Expr> {
    let mut node = node;
    for index in path {
        let Expr::List(items) = node else {
            return None;
        };
        node = items.get_mut(*index)?;
    }
    Some(node)
}

/// An item for a reason: its layer and its first point.
fn item_label(item: &Expr) -> String {
    let layer = form_atom(item, "layer", 1).unwrap_or("");
    match points_box(item) {
        Some(bounds) => format!(
            "on {layer} at ({}, {})",
            format_mm(bounds[0]),
            format_mm(bounds[1])
        ),
        None => format!("on {layer}"),
    }
}

/// Moves an arc (start, mid, end) with the edge: whole when both ends lie
/// in the band, not at all when neither does, and when one does, that end
/// moves and the arc keeps its shape between its ends (its middle mapped
/// by the similarity that takes the old ends to the new), so that what is
/// joined to either end stays joined. An arc without a middle (the old
/// centre form) moves whole by its first point. Returns whether it moved.
fn move_arc(item: &mut Expr, axis: usize, shift: f64, in_band: &dyn Fn(f64) -> bool) -> bool {
    let (Ok(start), Ok(mid), Ok(end)) = (
        form_xy(item, "start"),
        form_xy(item, "mid"),
        form_xy(item, "end"),
    ) else {
        let Some(point) = reference_point(item) else {
            return false;
        };
        if !in_band(point[axis]) {
            return false;
        }
        let mut changed = false;
        shift_points(item, axis, shift, &|_| true, &mut changed);
        return changed;
    };
    let (start_moves, end_moves) = (in_band(start[axis]), in_band(end[axis]));
    if !start_moves && !end_moves {
        return false;
    }
    let mut changed = false;
    if start_moves && end_moves {
        shift_points(item, axis, shift, &|_| true, &mut changed);
        return changed;
    }
    let mut new_start = start;
    let mut new_end = end;
    if start_moves {
        new_start[axis] += shift;
    } else {
        new_end[axis] += shift;
    }
    let old = [end[0] - start[0], end[1] - start[1]];
    let new = [new_end[0] - new_start[0], new_end[1] - new_start[1]];
    let length = old[0] * old[0] + old[1] * old[1];
    if length < 1.0e-12 {
        shift_points(item, axis, shift, &|_| true, &mut changed);
        return changed;
    }
    // new / old as complex numbers.
    let ratio = [
        (new[0] * old[0] + new[1] * old[1]) / length,
        (new[1] * old[0] - new[0] * old[1]) / length,
    ];
    let offset = [mid[0] - start[0], mid[1] - start[1]];
    let new_mid = [
        new_start[0] + offset[0] * ratio[0] - offset[1] * ratio[1],
        new_start[1] + offset[0] * ratio[1] + offset[1] * ratio[0],
    ];
    for (head, point) in [("start", new_start), ("mid", new_mid), ("end", new_end)] {
        if let Some(Expr::List(form)) = item_child_mut(item, head)
            && form.len() >= 3
        {
            form[1] = number(point[0]);
            form[2] = number(point[1]);
        }
    }
    true
}

fn item_child_mut<'a>(item: &'a mut Expr, head: &str) -> Option<&'a mut Expr> {
    let Expr::List(items) = item else {
        return None;
    };
    items.iter_mut().find(|child| child.head() == Some(head))
}

/// The box of a rigid item: a text's from its font, length and
/// justification, a circle's from its radius, a via's from its size,
/// else the box of its points.
fn rigid_box(item: &Expr) -> Option<Box4> {
    match item.head() {
        Some("gr_text") => text_box(item).or_else(|| points_box(item)),
        Some("gr_circle") => {
            let center = form_xy(item, "center").ok()?;
            let end = form_xy(item, "end").ok()?;
            let radius = distance_squared(center, end).sqrt();
            Some([
                center[0] - radius,
                center[1] - radius,
                center[0] + radius,
                center[1] + radius,
            ])
        }
        Some("via") => {
            let at = form_xy(item, "at").ok()?;
            let half = form_atom(item, "size", 1)
                .and_then(|size| size.parse::<f64>().ok())
                .unwrap_or(0.0)
                / 2.0;
            Some([at[0] - half, at[1] - half, at[0] + half, at[1] + half])
        }
        _ => points_box(item),
    }
}

/// A board text's box, estimated: each character as wide as the font,
/// lines 1.6 heights apart, placed by its justification and turned by
/// its angle about its anchor.
fn text_box(item: &Expr) -> Option<Box4> {
    let at = item.child("at")?;
    let value = |index: usize| {
        at.children()
            .get(index)
            .and_then(Expr::atom)
            .and_then(|atom| atom.parse::<f64>().ok())
    };
    let (x, y, angle) = (value(1)?, value(2)?, value(3).unwrap_or(0.0));
    let text = item.children().get(1)?.atom()?;
    let effects = item.child("effects");
    let font = effects.and_then(|effects| effects.child("font"));
    let size = |index: usize| {
        font.and_then(|font| form_atom(font, "size", index))
            .and_then(|size| size.parse::<f64>().ok())
            .unwrap_or(1.0)
    };
    let (height, width) = (size(1), size(2));
    let lines: Vec<&str> = text
        .split(['\n'])
        .flat_map(|line| line.split("\\n"))
        .collect();
    let longest = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0) as f64;
    let box_width = longest * width;
    let box_height = height * (1.0 + 1.6 * (lines.len().max(1) - 1) as f64);
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
    let x0 = if justify.contains(&"left") {
        0.0
    } else if justify.contains(&"right") {
        -box_width
    } else {
        -box_width / 2.0
    };
    let y0 = if justify.contains(&"top") {
        0.0
    } else if justify.contains(&"bottom") {
        -box_height
    } else {
        -box_height / 2.0
    };
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        let local = [x0 + dx * box_width, y0 + dy * box_height];
        let offset = rotate_vector(local, -angle);
        let point = [x + offset[0], y + offset[1]];
        bounds = [
            bounds[0].min(point[0]),
            bounds[1].min(point[1]),
            bounds[2].max(point[0]),
            bounds[3].max(point[1]),
        ];
    }
    Some(bounds)
}

/// One layout of the search (or a size that needed none), as
/// `free-dimension.json` lists it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct KiCadFreeDimensionRun {
    pub size_mm: f64,
    /// `probe` (the reduced budget), `confirm` (the budget left at the
    /// smallest clean size), `retry` (the budget left at a smaller size
    /// that came close, after a clean probe: the results are not
    /// monotone), `closest` (the budget left at the size that came
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

/// The chosen run's verdict when the search found it clean by `accept`:
/// the exit follows the acceptance rule, so a size clean by `pass` (no
/// KiCad unconnected item, no new error by the count rule, every part
/// placed) whose layout says incomplete (a missed soft constraint) becomes
/// `complete`, exit 0; the layout's own status stays in
/// `free_dimension.layout_status` and its reason after the new one.
pub fn accept_verdict(verdict: &mut serde_json::Value, accept: &str, size_mm: f64) {
    if verdict["status"] == "complete" {
        return;
    }
    let layout_status = verdict["status"].clone();
    let layout_reason = verdict["reason"].as_str().unwrap_or("").to_string();
    verdict["status"] = serde_json::json!("complete");
    verdict["exit_code"] = serde_json::json!(crate::verdict::EXIT_COMPLETE);
    verdict["reason"] = serde_json::json!(format!(
        "complete by the free dimension's accept {accept:?} at {} mm; the layout's own verdict: {layout_reason}",
        format_mm(size_mm)
    ));
    if !verdict["free_dimension"].is_object() {
        verdict["free_dimension"] = serde_json::json!({});
    }
    verdict["free_dimension"]["layout_status"] = layout_status;
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

    /// Not clean, but close: every part placed, KiCad checked it, and its
    /// unconnected items and errors at most `CLOSE_FINDINGS` together.
    fn close(&self) -> bool {
        self.is_run()
            && self.unplaced == 0
            && self
                .kicad_unconnected
                .is_some_and(|open| open + self.errors <= CLOSE_FINDINGS)
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
    /// Lay out one size at the budget left: `confirm` (the smallest clean
    /// size), `retry` (a smaller size that came close) or `closest` (no
    /// probe was clean).
    Full {
        size: f64,
        budget: f64,
        kind: &'static str,
    },
    Done,
}

/// The search over the sizes: probes from the smallest size upward, in
/// batches of `parallel`, while the budget leaves room for the full run;
/// then the full runs, each with everything left of `total_seconds`: after
/// a clean probe, the largest smaller size that came close (`close`), else
/// the smallest clean size again (the confirmation); without a clean probe
/// the size that came closest. Every run's work counts against
/// `total_seconds`.
#[derive(Clone, Debug)]
pub struct KiCadFreeDimensionSearch {
    pending: VecDeque<f64>,
    pub probe_seconds: f64,
    pub confirm_seconds: f64,
    pub total_seconds: f64,
    pub parallel: usize,
    pub runs: Vec<KiCadFreeDimensionRun>,
    probing_done: bool,
}

/// A probe that was not clean "came close" when every part was placed and
/// KiCad's unconnected items and errors are at most this many together:
/// worth the full budget after a larger size was clean (the fence's two
/// layers: 38 mm 3 unconnected, 40 and 42 mm a starved thermal each, 44
/// mm clean).
pub const CLOSE_FINDINGS: usize = 4;

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
            probing_done: false,
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

    /// The smallest clean run (any kind).
    fn smallest_clean(&self) -> Option<&KiCadFreeDimensionRun> {
        self.runs
            .iter()
            .filter(|run| run.clean && run.is_run())
            .min_by(|a, b| a.size_mm.total_cmp(&b.size_mm))
    }

    fn closest(&self) -> Option<&KiCadFreeDimensionRun> {
        self.runs
            .iter()
            .filter(|run| run.is_run())
            .min_by_key(|run| run.distance())
    }

    /// Whether a size had a run at more than the probe budget.
    fn had_full_run(&self, size: f64) -> bool {
        self.runs
            .iter()
            .any(|run| run.is_run() && run.kind != "probe" && (run.size_mm - size).abs() < 1.0e-6)
    }

    pub fn next_step(&mut self) -> KiCadFreeDimensionStep {
        let left = self.total_seconds - self.spent();
        if !self.probing_done {
            if self.smallest_clean().is_none() {
                let room = left - self.confirm_seconds;
                let count = ((room + 1.0e-6) / self.probe_seconds).floor().max(0.0) as usize;
                let count = count.min(self.parallel).min(self.pending.len());
                if count > 0 {
                    return KiCadFreeDimensionStep::Probe {
                        sizes: self.pending.drain(..count).collect(),
                        budget: self.probe_seconds,
                    };
                }
            }
            self.probing_done = true;
        }
        // The final runs, each with all that is left: worth it only with
        // more than the run it repeats had.
        let worth = |run: &KiCadFreeDimensionRun| left > run.budget_seconds + 1.0e-6;
        if let Some(clean) = self.smallest_clean().cloned() {
            // A smaller size that came close, the largest first (results
            // are not monotone: it may be clean at the full budget).
            let retry = self
                .runs
                .iter()
                .filter(|run| {
                    run.kind == "probe"
                        && !run.clean
                        && run.size_mm < clean.size_mm - 1.0e-6
                        && run.close()
                        && !self.had_full_run(run.size_mm)
                })
                .max_by(|a, b| a.size_mm.total_cmp(&b.size_mm))
                .cloned();
            if let Some(retry) = retry
                && worth(&retry)
            {
                return KiCadFreeDimensionStep::Full {
                    size: retry.size_mm,
                    budget: left,
                    kind: "retry",
                };
            }
            if clean.kind == "probe" && !self.had_full_run(clean.size_mm) && worth(&clean) {
                return KiCadFreeDimensionStep::Full {
                    size: clean.size_mm,
                    budget: left,
                    kind: "confirm",
                };
            }
            return KiCadFreeDimensionStep::Done;
        }
        if let Some(closest) = self.closest().cloned()
            && !self.had_full_run(closest.size_mm)
            && !self.runs.iter().any(|run| run.kind == "closest")
            && worth(&closest)
        {
            return KiCadFreeDimensionStep::Full {
                size: closest.size_mm,
                budget: left,
                kind: "closest",
            };
        }
        KiCadFreeDimensionStep::Done
    }

    pub fn record(&mut self, runs: Vec<KiCadFreeDimensionRun>) {
        if runs.iter().any(|run| run.kind != "probe") {
            self.probing_done = true;
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
        let probed: Vec<&KiCadFreeDimensionRun> = self
            .runs
            .iter()
            .filter(|run| run.kind != "infeasible" && run.kind != "confirm")
            .collect();
        let (index, reason) = match clean {
            Some(index) => {
                let run = &self.runs[index];
                let smaller: Vec<String> =
                    probed
                        .iter()
                        .filter(|other| other.size_mm < run.size_mm - 1.0e-6)
                        .map(|other| {
                            format!(
                                "{} mm {}{}",
                                format_mm(other.size_mm),
                                if other.kind == "probe" {
                                    String::new()
                                } else {
                                    format!("{} at {:.0} s ", other.kind, other.budget_seconds)
                                },
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
                    match run.kind.as_str() {
                        "probe" => "its probe".to_string(),
                        "retry" => format!(
                            "retried at {:.0} s after a larger size was clean",
                            run.budget_seconds
                        ),
                        _ => format!("at {:.0} s", run.budget_seconds),
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
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
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

    /// A placer config fixing `fixed`.
    fn placer(fixed: &[&str]) -> crate::KiCadBoardPlacerConfig {
        crate::KiCadBoardPlacerConfig {
            fixed: fixed.iter().map(|name| name.to_string()).collect(),
            ..Default::default()
        }
    }

    fn plan(
        free: &KiCadFreeDimension,
        constraints: Option<KiCadPlacementConstraints>,
    ) -> KiCadFreeDimensionPlan {
        KiCadFreeDimensionPlan::new(free, BOARD, constraints, &placer(&["J1"])).unwrap()
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
        // A region from the top edge 10 mm down: its top side is on the
        // edge and moves, its lower side stays: it stretches.
        assert_eq!(constraints.region[0].y, Some([0.0, 20.0]));
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
        let error =
            KiCadFreeDimensionPlan::new(&free("bottom"), BOARD, Some(polygon), &placer(&[]))
                .unwrap_err();
        assert!(error.contains("rectangular"), "{error}");
        let automatic: KiCadPlacementConstraints =
            serde_json::from_value(serde_json::json!({"version": 1, "outline": {}})).unwrap();
        let error =
            KiCadFreeDimensionPlan::new(&free("bottom"), BOARD, Some(automatic), &placer(&[]))
                .unwrap_err();
        assert!(error.contains("width and a height"), "{error}");
        let arc = BOARD.replace(
            "(gr_rect (start 100 100) (end 150 130)",
            "(gr_arc (start 100 100) (mid 125 90) (end 150 100) (layer \"Edge.Cuts\")) (gr_line (start 150 100) (end 150 130) (layer \"Edge.Cuts\")) (gr_line (start 150 130) (end 100 130) (layer \"Edge.Cuts\")) (gr_line (start 100 130) (end 100 100)",
        );
        let error =
            KiCadFreeDimensionPlan::new(&free("bottom"), &arc, None, &placer(&[])).unwrap_err();
        assert!(error.contains("gr_arc"), "{error}");
        // Four lines along a box are a rectangle.
        let lines = BOARD.replace(
            "(gr_rect (start 100 100) (end 150 130)",
            "(gr_line (start 100 100) (end 150 100) (layer \"Edge.Cuts\")) (gr_line (start 150 100) (end 150 130) (layer \"Edge.Cuts\")) (gr_line (start 150 130) (end 100 130) (layer \"Edge.Cuts\")) (gr_line (start 100 130) (end 100 100)",
        );
        let plan =
            KiCadFreeDimensionPlan::new(&free("bottom"), &lines, None, &placer(&[])).unwrap();
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
        assert!(KiCadFreeDimensionPlan::new(&wide, BOARD, None, &placer(&[])).is_err());
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
            KiCadFreeDimensionPlan::new(&carried, BOARD, None, &placer(&["J1", "U1"])).unwrap();
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
        let plan =
            KiCadFreeDimensionPlan::new(&free("bottom"), BOARD, Some(constraints), &placer(&[]))
                .unwrap();
        let reasons = plan.variant(24.0).unwrap_err();
        assert!(
            reasons.iter().any(|reason| reason.contains("keepout box")),
            "{reasons:?}"
        );
    }

    /// A 50 x 30 mm board at (100, 100) with `items` on it.
    fn board_with(items: &str) -> String {
        format!(
            r#"(kicad_pcb (version 20240108)
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 100 100) (end 150 130) (stroke (width 0.1) (type solid)) (fill no) (layer "Edge.Cuts"))
  {items})"#
        )
    }

    fn item_box(board: &str, head: &str) -> Box4 {
        let pcb = parse(board).unwrap();
        let item = pcb
            .children()
            .iter()
            .find(|item| item.head() == Some(head))
            .unwrap();
        points_box(item).unwrap()
    }

    #[test]
    fn a_pour_whose_notch_lies_past_the_band_is_cut_at_the_new_edge() {
        // GND over the board with a 2 mm notch at the bottom edge: its
        // corners on the edge move, the notch's floor (2 mm in) does not.
        let text = board_with(
            r#"(zone (net 2) (net_name "GND") (layer "B.Cu") (name "gnd")
    (polygon (pts (xy 100 100) (xy 150 100) (xy 150 130) (xy 130 130) (xy 130 128) (xy 120 128) (xy 120 130) (xy 100 130)))
    (filled_polygon (layer "B.Cu") (pts (xy 101 101) (xy 149 101) (xy 149 129) (xy 101 129))))"#,
        );
        let plan = KiCadFreeDimensionPlan::new(&free("bottom"), &text, None, &placer(&[])).unwrap();
        // Growing: the edge's points move, the notch stays (it deepens).
        let grown = plan.variant(34.0).unwrap();
        assert_eq!(item_box(&grown.board, "zone"), [100.0, 100.0, 150.0, 134.0]);
        // Shrinking 4 mm: moving the corners to 126 would carry them past
        // the notch's floor at 128 (a folded polygon): cut instead.
        let shrunk = plan.variant(26.0).unwrap();
        let pcb = parse(&shrunk.board).unwrap();
        let zone = pcb.child("zone").unwrap();
        let mut lists = Vec::new();
        collect_point_lists(zone, &mut Vec::new(), &mut lists);
        let sorted = |points: &[[f64; 2]]| {
            let mut points = points.to_vec();
            points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
            points
        };
        assert_eq!(
            sorted(&lists[0].1),
            vec![
                [100.0, 100.0],
                [100.0, 126.0],
                [150.0, 100.0],
                [150.0, 126.0]
            ]
        );
        // The fill is cut too.
        assert_eq!(
            sorted(&lists[1].1),
            vec![
                [101.0, 101.0],
                [101.0, 126.0],
                [149.0, 101.0],
                [149.0, 126.0]
            ]
        );
    }

    #[test]
    fn drawn_shapes_texts_and_cutouts_past_the_band_make_a_shrink_infeasible() {
        // A silkscreen frame from 0.2 mm (in the band) to 2 mm (not) from
        // the edge: shrinking 3 mm folds it.
        let frame = board_with(
            r#"(gr_poly (pts (xy 102 128) (xy 148 128) (xy 148 129.8) (xy 102 129.8)) (layer "F.SilkS"))"#,
        );
        let plan =
            KiCadFreeDimensionPlan::new(&free("bottom"), &frame, None, &placer(&[])).unwrap();
        let reasons = plan.variant(27.0).unwrap_err();
        assert!(
            reasons[0].starts_with("the gr_poly on F.SilkS") && reasons[0].contains("fold"),
            "{reasons:?}"
        );
        // Growing stretches it.
        let grown = plan.variant(32.0).unwrap();
        assert_eq!(
            item_box(&grown.board, "gr_poly"),
            [102.0, 128.0, 148.0, 131.8]
        );
        // An inner cutout 3 mm from the edge stays, and a shrink past it
        // is infeasible.
        let cutout =
            board_with(r#"(gr_circle (center 140 126) (end 141 126) (layer "Edge.Cuts"))"#);
        let plan =
            KiCadFreeDimensionPlan::new(&free("bottom"), &cutout, None, &placer(&[])).unwrap();
        assert!(plan.variant(27.5).is_ok());
        let reasons = plan.variant(26.5).unwrap_err();
        assert!(
            reasons[0].starts_with("the gr_circle on Edge.Cuts")
                && reasons[0].ends_with("would end beyond the outline"),
            "{reasons:?}"
        );
        // A text by its box: one whose box reaches into the band moves
        // whole although its anchor is 0.8 mm from the edge; one whose box
        // ends 1 mm from it stays, and a shrink past it is infeasible.
        let texts = board_with(
            r#"(gr_text "AT EDGE" (at 110 129.2 0) (layer "F.SilkS") (effects (font (size 1.5 1.5))))
  (gr_text "INSIDE" (at 130 128.5 0) (layer "F.SilkS") (effects (font (size 1 1))))"#,
        );
        let plan =
            KiCadFreeDimensionPlan::new(&free("bottom"), &texts, None, &placer(&[])).unwrap();
        let variant = plan.variant(29.5).unwrap();
        let pcb = parse(&variant.board).unwrap();
        let anchors: Vec<[f64; 3]> = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("gr_text"))
            .map(|item| form_at(item).unwrap())
            .collect();
        assert_eq!(anchors, vec![[110.0, 128.7, 0.0], [130.0, 128.5, 0.0]]);
        let reasons = plan.variant(28.5).unwrap_err();
        assert!(
            reasons
                .iter()
                .any(|reason| reason.starts_with("the gr_text on F.SilkS at (130, 128.5)")),
            "{reasons:?}"
        );
    }

    #[test]
    fn an_arc_across_the_band_keeps_what_is_joined_to_its_ends() {
        // A track arc from the edge band (y 129.8) to (115, 125), joined
        // there to a segment that stays.
        let text = board_with(
            r#"(arc (start 110 129.8) (mid 113.5 128.4) (end 115 125) (width 0.2) (layer "F.Cu") (net 1))
  (segment (start 115 125) (end 120 120) (width 0.2) (layer "F.Cu") (net 1))
  (gr_arc (start 120 129.9) (mid 125 129.6) (end 130 129.9) (layer "F.SilkS"))"#,
        );
        let plan = KiCadFreeDimensionPlan::new(&free("bottom"), &text, None, &placer(&[])).unwrap();
        let variant = plan.variant(34.0).unwrap();
        let pcb = parse(&variant.board).unwrap();
        let arc = pcb.child("arc").unwrap();
        let (start, mid, end) = (
            form_xy(arc, "start").unwrap(),
            form_xy(arc, "mid").unwrap(),
            form_xy(arc, "end").unwrap(),
        );
        assert_eq!(start, [110.0, 133.8]);
        assert_eq!(end, [115.0, 125.0]);
        assert_eq!(
            form_xy(pcb.child("segment").unwrap(), "start").unwrap(),
            end
        );
        // The same shape between the new ends: the middle as far from
        // the chord, in proportion.
        let side = |a: [f64; 2], b: [f64; 2], p: [f64; 2]| {
            ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])) / distance_squared(a, b)
        };
        let before = side([110.0, 129.8], [115.0, 125.0], [113.5, 128.4]);
        assert!((side(start, end, mid) - before).abs() < 1.0e-5, "{mid:?}");
        // An arc wholly in the band moves whole.
        let silk = pcb.child("gr_arc").unwrap();
        assert_eq!(form_xy(silk, "mid").unwrap(), [125.0, 133.6]);
    }

    #[test]
    fn a_box_reaching_across_the_board_stretches_and_may_vanish() {
        let constraints: KiCadPlacementConstraints = serde_json::from_value(serde_json::json!({
            "version": 1,
            "keepout": [{"x": [5, 10], "y": [5, 29.8]}],
            "region": [{"parts": ["U1"], "x": [0, 50], "y": [29.9, 2]}]
        }))
        .unwrap();
        let text = board_with(
            r#"(footprint "r" (layer "F.Cu") (at 125 115 0)
    (property "Reference" "U1" (at 0 0 0) (layer "F.SilkS"))
    (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))"#,
        );
        let plan =
            KiCadFreeDimensionPlan::new(&free("bottom"), &text, Some(constraints), &placer(&[]))
                .unwrap();
        // Growing: the side at the edge moves, the far side stays.
        let grown = plan.variant(34.0).unwrap().constraints.unwrap();
        assert_eq!(grown.keepout[0].y, [5.0, 33.8]);
        // Given high to low: the same.
        assert_eq!(grown.region[0].y, Some([33.9, 2.0]));
        let shrunk = plan.variant(16.0).unwrap().constraints.unwrap();
        assert_eq!(shrunk.keepout[0].y, [5.0, 15.8]);
        // Its side at the edge would pass its far side.
        let reasons = plan.variant(5.0).unwrap_err();
        assert!(
            reasons.iter().any(|reason| reason
                == "the keepout box would vanish: the edge would carry its side past its far side"),
            "{reasons:?}"
        );
    }

    #[test]
    fn a_fixed_part_whose_body_reaches_the_edge_moves_with_it() {
        // J9's origin is 3 mm from the edge (outside the default band), its
        // body reaches the edge.
        let text = board_with(
            r#"(footprint "conn" (layer "F.Cu") (at 125 127 0)
    (property "Reference" "J9" (at 0 0 0) (layer "F.SilkS"))
    (fp_rect (start -3 -1) (end 3 3) (layer "F.CrtYd"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
  (footprint "r" (layer "F.Cu") (at 115 127 0)
    (property "Reference" "R9" (at 0 0 0) (layer "F.SilkS"))
    (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))"#,
        );
        // A band of 2 mm: R9's body (2 mm from the edge) reaches into it,
        // its origin (3 mm) does not.
        let mut dimension = free("bottom");
        dimension.band_mm = Some(2.0);
        let plan = KiCadFreeDimensionPlan::new(&dimension, &text, None, &placer(&["J9"])).unwrap();
        assert_eq!(plan.fixed_references(), vec!["J9".to_string()]);
        let grown = plan.variant(34.0).unwrap();
        assert_eq!(footprint_at(&grown.board, "J9"), [125.0, 131.0, 0.0]);
        // R9 is placed again: by its origin, it stays.
        assert_eq!(footprint_at(&grown.board, "R9"), [115.0, 127.0, 0.0]);
        assert_eq!(grown.moved_footprints, vec!["J9".to_string()]);
        // Shrinking carries it too, instead of stranding it.
        let shrunk = plan.variant(28.0).unwrap();
        assert_eq!(footprint_at(&shrunk.board, "J9"), [125.0, 125.0, 0.0]);
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
                // Everything left (1500 less four probes), not 300; 34 mm
                // (12 open) did not come close.
                KiCadFreeDimensionStep::Full {
                    size: 36.0,
                    budget: 1100.0,
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
            } else if size == 34.0 {
                (12, 1)
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
    fn a_smaller_size_that_came_close_is_retried_with_all_that_is_left() {
        // 34 open, 36 one starved thermal, 38 clean: 36 again with the
        // rest, and clean there (results are not monotone in the budget).
        let mut search = KiCadFreeDimensionSearch::new(
            vec![34.0, 36.0, 38.0, 40.0],
            Vec::new(),
            100.0,
            300.0,
            1000.0,
            2,
        );
        let steps = drive(&mut search, |size, budget| match size as i64 {
            34 => (20, 0),
            36 if budget < 200.0 => (0, 1),
            _ => (0, 0),
        });
        assert_eq!(
            steps,
            vec![
                KiCadFreeDimensionStep::Probe {
                    sizes: vec![34.0, 36.0],
                    budget: 100.0
                },
                KiCadFreeDimensionStep::Probe {
                    sizes: vec![38.0, 40.0],
                    budget: 100.0
                },
                KiCadFreeDimensionStep::Full {
                    size: 36.0,
                    budget: 600.0,
                    kind: "retry"
                },
                KiCadFreeDimensionStep::Done
            ]
        );
        let (index, reason) = search.choose(&free("bottom")).unwrap();
        assert_eq!(
            (search.runs[index].size_mm, search.runs[index].kind.as_str()),
            (36.0, "retry")
        );
        assert!(reason.contains("retried at 600 s"), "{reason}");
        // The retry not clean: the clean probe stays the result.
        let mut search = KiCadFreeDimensionSearch::new(
            vec![34.0, 36.0, 38.0, 40.0],
            Vec::new(),
            100.0,
            300.0,
            1000.0,
            2,
        );
        drive(&mut search, |size, _| match size as i64 {
            34 => (20, 0),
            36 => (0, 1),
            _ => (0, 0),
        });
        let (index, reason) = search.choose(&free("bottom")).unwrap();
        assert_eq!(
            (search.runs[index].size_mm, search.runs[index].kind.as_str()),
            (38.0, "probe")
        );
        assert!(
            reason.contains("36 mm retry at 600 s 0 open, 1 errors"),
            "{reason}"
        );
    }

    #[test]
    fn one_at_a_time_a_clean_first_probe_is_confirmed_with_the_rest() {
        let mut search = KiCadFreeDimensionSearch::new(
            vec![34.0, 36.0, 38.0],
            Vec::new(),
            100.0,
            300.0,
            1500.0,
            1,
        );
        let steps = drive(&mut search, |_, _| (0, 0));
        assert_eq!(
            steps,
            vec![
                KiCadFreeDimensionStep::Probe {
                    sizes: vec![34.0],
                    budget: 100.0
                },
                KiCadFreeDimensionStep::Full {
                    size: 34.0,
                    budget: 1400.0,
                    kind: "confirm"
                },
                KiCadFreeDimensionStep::Done
            ]
        );
        assert_eq!(search.spent(), 1500.0);
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
        // Chosen clean by pass, the search's verdict is complete (exit 0),
        // the layout's own status kept beside it.
        let mut chosen = verdict.clone();
        chosen["reason"] = serde_json::json!("incomplete: constraints missed");
        chosen["free_dimension"] = serde_json::json!({"chosen": 40.0});
        accept_verdict(&mut chosen, "pass", 40.0);
        assert_eq!(chosen["status"], "complete");
        assert_eq!(chosen["exit_code"], 0);
        assert_eq!(chosen["free_dimension"]["layout_status"], "incomplete");
        assert_eq!(chosen["free_dimension"]["chosen"], 40.0);
        assert!(
            chosen["reason"]
                .as_str()
                .unwrap()
                .ends_with("the layout's own verdict: incomplete: constraints missed")
        );
    }
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The global route inside the anneal (`AnnealConfig::guidance`, an
//! experiment, docs/global-route.md "Phase 2"): every so many accepted
//! moves the caller routes the current placement globally and returns
//! where the boundaries overflow. Until the next evaluation that field is
//! fixed: a net pays its wirelength times the mean overflow of the tiles
//! its pins' box covers (its demand where demand already exceeds
//! capacity, the local form of the total overflow), and the parts whose
//! nets cross the most overflow are proposed for moves more often.

use crate::problem::{Point, Pose};
use std::sync::Arc;

/// One evaluation of a placement's global route.
#[derive(Clone, Debug, Default)]
pub struct GuidanceField {
    /// The lower corner of tile (0, 0), in the placer's millimetres.
    pub origin: Point,
    pub tile_mm: f64,
    pub tiles_x: usize,
    pub tiles_y: usize,
    /// Per tile (row-major, `y * tiles_x + x`): tracks over capacity on
    /// its boundaries, summed over the layers.
    pub overflow: Vec<f64>,
    /// Per component: the overflow on the boundaries its nets cross,
    /// summed over those nets.
    pub attribution: Vec<f64>,
    /// The global route's total overflow (for the trace).
    pub total_overflow: f64,
    /// Placement work (`crate::work` units) the evaluation counts.
    pub work: u64,
}

pub type GuidanceFn = dyn Fn(&[Pose]) -> Option<GuidanceField> + Send + Sync;

/// The anneal's global-route guidance.
#[derive(Clone)]
pub struct Guidance {
    pub evaluate: Arc<GuidanceFn>,
    /// Millimetres of wirelength one track of mean overflow under a net's
    /// box adds per millimetre of the net.
    pub weight: f64,
    /// The evaluations may take at most this share of the anneal's own
    /// work (the first one aside).
    pub work_share: f64,
    /// At least this many accepted moves between two evaluations.
    pub min_accepted: usize,
    /// Share of the moves whose part is drawn by its attribution instead
    /// of uniformly.
    pub bias: f64,
}

impl std::fmt::Debug for Guidance {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Guidance")
            .field("weight", &self.weight)
            .field("work_share", &self.work_share)
            .field("min_accepted", &self.min_accepted)
            .field("bias", &self.bias)
            .finish()
    }
}

/// A field ready for box queries: a summed-area table of the overflow.
pub(crate) struct Prepared {
    origin: Point,
    tile_mm: f64,
    tiles_x: usize,
    tiles_y: usize,
    /// `(tiles_x + 1) * (tiles_y + 1)`, row-major.
    sums: Vec<f64>,
    pub(crate) weight: f64,
}

impl Prepared {
    pub(crate) fn new(field: &GuidanceField, weight: f64) -> Option<Self> {
        let (nx, ny) = (field.tiles_x, field.tiles_y);
        if nx == 0 || ny == 0 || field.overflow.len() != nx * ny || field.tile_mm <= 0.0 {
            return None;
        }
        let mut sums = vec![0.0; (nx + 1) * (ny + 1)];
        for y in 0..ny {
            let mut row = 0.0;
            for x in 0..nx {
                row += field.overflow[y * nx + x];
                sums[(y + 1) * (nx + 1) + x + 1] = sums[y * (nx + 1) + x + 1] + row;
            }
        }
        Some(Self {
            origin: field.origin,
            tile_mm: field.tile_mm,
            tiles_x: nx,
            tiles_y: ny,
            sums,
            weight,
        })
    }

    /// Mean overflow over the tiles the box `[x0, y0, x1, y1]` touches
    /// (clamped to the grid).
    pub(crate) fn mean(&self, bounds: [f64; 4]) -> f64 {
        let index = |value: f64, origin: f64, count: usize| {
            (((value - origin) / self.tile_mm).floor().max(0.0) as usize).min(count - 1)
        };
        let (x0, x1) = (
            index(bounds[0], self.origin[0], self.tiles_x),
            index(bounds[2], self.origin[0], self.tiles_x),
        );
        let (y0, y1) = (
            index(bounds[1], self.origin[1], self.tiles_y),
            index(bounds[3], self.origin[1], self.tiles_y),
        );
        let w = self.tiles_x + 1;
        let sum = self.sums[(y1 + 1) * w + x1 + 1]
            - self.sums[y0 * w + x1 + 1]
            - self.sums[(y1 + 1) * w + x0]
            + self.sums[y0 * w + x0];
        sum / ((x1 - x0 + 1) * (y1 - y0 + 1)) as f64
    }
}

/// Cumulative attribution over `movable` for weighted draws (`None` when
/// nothing is attributed).
pub(crate) fn cumulative(field: &GuidanceField, movable: &[usize]) -> Option<Vec<f64>> {
    let mut total = 0.0;
    let cumulative: Vec<f64> = movable
        .iter()
        .map(|index| {
            total += field
                .attribution
                .get(*index)
                .copied()
                .unwrap_or(0.0)
                .max(0.0);
            total
        })
        .collect();
    (total > 0.0).then_some(cumulative)
}

/// The position in `movable` a uniform `draw` in [0, 1) selects.
pub(crate) fn pick(cumulative: &[f64], draw: f64) -> usize {
    let target = draw * cumulative.last().copied().unwrap_or(0.0);
    cumulative
        .partition_point(|value| *value <= target)
        .min(cumulative.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mean_covers_the_tiles_a_box_touches() {
        let field = GuidanceField {
            origin: [0.0, 0.0],
            tile_mm: 1.0,
            tiles_x: 3,
            tiles_y: 2,
            overflow: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            ..Default::default()
        };
        let prepared = Prepared::new(&field, 1.0).unwrap();
        assert_eq!(prepared.mean([0.5, 0.5, 0.5, 0.5]), 1.0);
        assert_eq!(prepared.mean([0.5, 0.5, 2.5, 1.5]), 3.5);
        assert_eq!(prepared.mean([1.5, 1.2, 2.9, 1.9]), 5.5);
        // Clamped outside the grid.
        assert_eq!(prepared.mean([-5.0, -5.0, -4.0, -4.0]), 1.0);
    }

    #[test]
    fn draws_follow_the_attribution() {
        let field = GuidanceField {
            attribution: vec![0.0, 3.0, 0.0, 1.0],
            ..Default::default()
        };
        let cumulative = cumulative(&field, &[0, 1, 2, 3]).unwrap();
        assert_eq!(pick(&cumulative, 0.0), 1);
        assert_eq!(pick(&cumulative, 0.74), 1);
        assert_eq!(pick(&cumulative, 0.76), 3);
        assert_eq!(pick(&cumulative, 0.999), 3);
        let none = GuidanceField {
            attribution: vec![0.0; 4],
            ..Default::default()
        };
        assert!(super::cumulative(&none, &[0, 1, 2, 3]).is_none());
    }
}

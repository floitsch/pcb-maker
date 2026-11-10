// Copyright (C) 2026 Toit contributors.

//! Differential pairs (`diff_pairs`, docs/router.md "Differential
//! pairs").
//!
//! Nets KiCad pairs by name (`diff_pair_of`: `D+`/`D-`, `TX_P`/`TX_N`,
//! `USB_DP`/`USB_DN`) route coupled. A pair's pads sit side by side on each
//! part it joins (a *pair point*); the pair points are linked by a minimum
//! spanning tree (pair points of one part are joined by ordinary routing),
//! and each link is a net of the route itself (`plan_pairs`), negotiated
//! with every other net: its track is the pair's envelope (two tracks and
//! their gap: `2 w + g`), its via two vias side by side, from a *launch*
//! clear of one pair point's pads to one of the other's, which straight
//! stubs join to the pads: the link's own where they keep every clearance
//! (`stubs_clear`; the pads are then of the run's terminal), else the
//! router's (the pair's nets route to the run's ends). The pair's own nets lose the connections the
//! links make (their pads at a link's ends are one terminal). When the
//! route is written (`finish_pairs`), each link's centreline is offset by
//! `(w + g) / 2` to either side into the pair's two tracks, its vias into a
//! via of each net, and the stubs are added: what holds KiCad's clearances
//! (`verify`, against everything routed) is the pair's copper. What a link
//! left open (not routed, refused, a stub refused, the stubs of a crossed
//! end) is routed for the pair's nets with everything else kept (the
//! *fallback*).
//!
//! The stages, one file each:
//!
//! - Detection (detect.rs): the pairs by name, their rules, pair points and
//!   links (`find_pairs`); the settings a route with pairs uses
//!   (rules.rs, `with_pairs`).
//! - The plan before routing (plan.rs, `plan_pairs`): a class per layer
//!   set, the neighbours' launches, then per link its launches
//!   (plan_link.rs: launch.rs searches them, crossover.rs fits a crossover
//!   at a crossed end, clearance.rs and reach.rs hold the checks on the
//!   board) and its net, terminals and copper on the board; the links'
//!   give-up (give_up.rs); the plan's work model (work.rs).
//! - After routing (finish.rs, `finish_pairs`): each routed link's
//!   centreline (centreline.rs) offset into the pair's copper
//!   (link_copper.rs) as a candidate (candidates.rs), checked with every
//!   other (check.rs); the stubs a
//!   kept link lacks routed (repair.rs); the stubs straightened, joined and
//!   pruned (stubs.rs, prune.rs); the fallback (fallback.rs); the skew's
//!   meanders (skew.rs) and the return vias (return_vias.rs); the copper
//!   written (pair_copper.rs).
//! - The report (report.rs) and the uncoupled length measured on the
//!   written board (measure.rs); the diagnostics of `PCB_ROUTER_PAIRS_WHY`
//!   and `PCB_ROUTER_PAIRS_REACH` (diagnosis.rs).

use super::meander::{
    Tuned, Tuning, bumps, copper_heights, copper_length, polyline_length, tuning_pattern_expr,
};
use super::*;
use pcb_router as core;

mod candidates;
mod centreline;
mod check;
mod clearance;
mod crossover;
mod detect;
mod diagnosis;
mod fallback;
mod finish;
mod give_up;
mod launch;
mod link_copper;
mod measure;
mod pair_copper;
mod plan;
mod plan_link;
mod prune;
mod reach;
mod repair;
mod report;
mod return_vias;
mod rules;
mod skew;
mod stubs;
mod work;

pub use self::report::*;
use self::{
    candidates::*, centreline::*, check::*, clearance::*, crossover::*, detect::*, diagnosis::*,
    fallback::*, finish::*, give_up::*, launch::*, link_copper::*, pair_copper::*, plan::*,
    plan_link::*, prune::*, reach::*, repair::*, return_vias::*, skew::*, stubs::*, work::*,
};
pub(crate) use self::{
    fallback::{pairs_fallback_short, pairs_fallback_work},
    finish::finish_pairs,
    measure::measure_uncoupled,
    pair_copper::write_pair_copper,
    plan::{link_name, lower_for_route, pairs_planned, plan_pairs},
    rules::with_pairs,
};

const EPSILON: f64 = 1.0e-9;

type Point = [f64; 2];

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}
fn scale(a: Point, factor: f64) -> Point {
    [a[0] * factor, a[1] * factor]
}
fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: Point) -> Point {
    let length = norm(a);
    if length < EPSILON {
        [0.0, 0.0]
    } else {
        scale(a, 1.0 / length)
    }
}
/// The left normal of a direction.
fn left(a: Point) -> Point {
    [-a[1], a[0]]
}
/// A direction turned by `degrees`.
fn turned(a: Point, degrees: f64) -> Point {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [a[0] * cos - a[1] * sin, a[0] * sin + a[1] * cos]
}

fn debug() -> bool {
    std::env::var_os("PCB_ROUTER_DIFF_PAIRS_DEBUG").is_some()
}

/// `PCB_ROUTER_PAIRS_WHY`: per link that falls back for its stubs or its
/// copper, the geometry on stderr (`pairs why: ...`): the pads, the stubs'
/// ends, the run's start and direction, what the pad's copper reaches;
/// the finding that refuses copper and what of the pair's copper it is on.
fn pairs_why() -> bool {
    std::env::var_os("PCB_ROUTER_PAIRS_WHY").is_some()
}

/// Whether crossed ends get a crossover in their lead (`crossover`, where
/// the link carries the stubs to it), the links' paths lose their hairpins
/// (`despiked`) and the kept links' vias keep their holes' clearance in the
/// fallback: a test's override, else `CROSSOVERS_BY_DEFAULT`.
fn crossovers() -> bool {
    CROSSOVERS
        .with(|cell| cell.get())
        .unwrap_or(CROSSOVERS_BY_DEFAULT)
}

/// The crossovers' default (`crossovers`): on since 2026-11-05 (pairs6),
/// a crossover only where the link carries the stubs to it. Measured
/// (docs/router.md, "A pair's stubs in the plan"): the crossed ends whose
/// stubs the router could not cross (no room for a via between the pads
/// and the run's start) were the most of the open stubs, and with the
/// crossovers coupled links and pairs rise on the measured boards. Off
/// before (2026-10-27, "The safe crossover": a crossover whose stubs the
/// router routes walled in a pad).
const CROSSOVERS_BY_DEFAULT: bool = true;

thread_local! {
    /// A test's override of `crossovers` (this thread only).
    static CROSSOVERS: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// A copper obstacle of `net` on `layers` of `shape`, blocking tracks and
/// vias, with no clearance of its own: the pairs' plan copper and kept
/// copper.
fn net_copper(
    shape: core::Shape,
    layers: core::LayerMask,
    kind: core::ObstacleKind,
    net: core::NetId,
    label: String,
) -> core::Obstacle {
    core::Obstacle {
        shape,
        layers,
        kind,
        net: Some(net),
        clearance: 0.0,
        clearance_override: None,
        plated: false,
        blocks_tracks: true,
        blocks_vias: true,
        label,
    }
}

#[cfg(test)]
mod tests;

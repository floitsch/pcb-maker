// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Capacitated corridors (`Config::capacitated_corridors`; docs/router.md,
//! "Capacitated corridors (2026-10-24)"): the negotiation's lattice
//! searches run inside the corridors of a global route with hard
//! capacities per tile boundary (pcb-global's), and a search that fails
//! inside its corridor teaches the global route instead of searching
//! wider: the boundary it failed at loses a track and the nets through it
//! are planned again.
//!
//! pcb-global depends on this crate, so the global route reaches the
//! router through `CapacitatedPlan` and a factory pcb-global registers
//! (`register_planner`). Without one the switch does nothing.
//!
//! The counters (`CorridorTotals`) are kept for the process, every router
//! and thread, with the switch on or off: where the searches of the
//! corridors, the windows and the board went, and what the learning did.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// The tile grid of a capacitated plan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanFrame {
    /// The lower corner of tile (0, 0), in millimetres.
    pub origin: [f64; 2],
    pub tile_mm: f64,
    pub tiles_x: usize,
    pub tiles_y: usize,
    pub layers: usize,
}

impl PlanFrame {
    pub fn tiles(&self) -> usize {
        self.tiles_x * self.tiles_y
    }

    /// The tile holding the point (clamped to the grid).
    pub fn tile_of(&self, point: [f64; 2]) -> usize {
        let index = |axis: usize, count: usize| {
            (((point[axis] - self.origin[axis]) / self.tile_mm)
                .floor()
                .max(0.0) as usize)
                .min(count - 1)
        };
        index(1, self.tiles_y) * self.tiles_x + index(0, self.tiles_x)
    }
}

/// A global route with capacities the router may lower, as pcb-global
/// provides it. States are `layer * tiles + tile`; boundaries are
/// `direction * layers * tiles + layer * tiles + tile` (direction 0: the
/// boundary to the tile on the right, 1: to the tile below).
pub trait CapacitatedPlan: Send + Sync {
    fn boxed_clone(&self) -> Box<dyn CapacitatedPlan>;
    fn frame(&self) -> PlanFrame;
    /// Whether the plan has `net` at all (two or more terminal tiles).
    fn planned(&self, net: usize) -> bool;
    /// Whether every terminal of `net` is reached on the graph.
    fn routed(&self, net: usize) -> bool;
    /// The states of `net`'s route (its terminals' too).
    fn states(&self, net: usize) -> &[u32];
    /// The boundaries `net`'s route crosses.
    fn crossings(&self, net: usize) -> &[u32];
    /// The tiles of `net`'s vias.
    fn vias(&self, net: usize) -> &[u32];
    /// The tracks of the reference class one track of `net` takes.
    fn demand(&self, net: usize) -> f32;
    /// A boundary's capacity in tracks of the reference class.
    fn capacity(&self, boundary: u32) -> f32;
    /// Lowers a boundary's capacity by one track, not below `floor`.
    /// Returns whether it changed.
    fn lower(&mut self, boundary: u32, floor: f32) -> bool;
    /// Lowers a tile's via capacity by one via, not below `floor`.
    fn lower_via(&mut self, tile: u32, floor: f32) -> bool;
    /// The nets whose routes cross one of `boundaries` or have a via in
    /// one of `via_tiles`, in net order.
    fn nets_through(&self, boundaries: &[u32], via_tiles: &[u32]) -> Vec<usize>;
    /// Takes the routes of `nets` out and routes them again, in this
    /// order, against the others' usage at the negotiation's last prices.
    fn replan(&mut self, nets: &[usize]);
    /// The plan's work so far (its build, its re-plans), in units of
    /// `UNIT_EXPANSIONS` expansions.
    fn work_units(&self) -> u64;
}

/// A factory of capacitated plans for a board (pcb-global's).
pub type PlanFactory = fn(&crate::Board) -> Box<dyn CapacitatedPlan>;

static FACTORY: OnceLock<PlanFactory> = OnceLock::new();

/// Registers the factory the router builds its capacitated plans with
/// (the first registration stays).
pub fn register_planner(factory: PlanFactory) {
    let _ = FACTORY.set(factory);
}

/// The registered factory.
pub fn planner() -> Option<PlanFactory> {
    FACTORY.get().copied()
}

/// What one unit of a plan's work counts in search expansions: the
/// global route's work model (docs/global-route.md, "Phase 2": 5 units a
/// capacity distance test, 30 a state popped, at 150e6 units a second)
/// at the reference's expansion rate, so that the budgets count it.
pub const UNIT_EXPANSIONS: f64 = crate::router::REFERENCE_EXPANSIONS_PER_SECOND / 150.0e6;

/// The default of `Config::capacitated_corridors`: off, unless
/// `PCB_ROUTER_CAPACITATED_CORRIDORS` says otherwise (`0` or empty off,
/// anything else on).
pub fn default_switch() -> bool {
    match std::env::var("PCB_ROUTER_CAPACITATED_CORRIDORS") {
        Ok(value) => !(value.is_empty() || value == "0"),
        Err(_) => false,
    }
}

/// `PCB_ROUTER_CAPACITATED_CORRIDORS` when set (it overrides the
/// configuration either way).
pub fn switch_override() -> Option<bool> {
    let value = std::env::var("PCB_ROUTER_CAPACITATED_CORRIDORS").ok()?;
    Some(!(value.is_empty() || value == "0"))
}

/// The kinds of lattice search the counters tell apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchKind {
    /// Inside a corridor: the tile planner's (switch off), or the
    /// planner's inside a capacitated corridor (switch on).
    Corridor = 0,
    /// Inside the whole capacitated corridor (switch on).
    Band = 1,
    /// The plain window.
    Window = 2,
    /// The whole board.
    Board = 3,
}

const KINDS: usize = 4;

/// Per mode (negotiation, hard) and kind: searches, found, expansions.
static SEARCHES: [[[AtomicU64; 3]; KINDS]; 2] =
    [const { [const { [const { AtomicU64::new(0) }; 3] }; KINDS] }; 2];

/// The learning's counters, by `Learn`.
static LEARN: [AtomicU64; LEARN_FIELDS] = [const { AtomicU64::new(0) }; LEARN_FIELDS];

/// What the learning counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Learn {
    /// Capacitated plans built, and their build's work in expansions.
    PlansBuilt = 0,
    PlanExpansions,
    /// Searches that found nothing inside the capacitated corridor.
    Misses,
    /// Boundaries (and via sites) lowered after a miss.
    Reductions,
    /// Boundaries a miss found that their floor (the tracks routed
    /// through) kept from going lower.
    FloorHeld,
    /// Misses where no boundary of the route lay between reached and
    /// unreached tiles (the failure inside a tile: pad access).
    NoBoundary,
    /// Boundaries lowered for nets the negotiation kept in conflict.
    ConflictReductions,
    /// Global re-plans, the nets they routed again and their work in
    /// expansions.
    Replans,
    ReplannedNets,
    ReplanExpansions,
    /// Nets routed again in a new corridor after a re-plan.
    Retries,
    /// Searches that fell back to the window and the board: the net has
    /// no global path under the learned capacities; nothing could be
    /// learned from the miss; the net has no global route from the start.
    FallbackNoPath,
    FallbackNothingLearned,
    /// Nets still missing when a batch's retries ran out (they stay open
    /// until the next iteration).
    Deferred,
    /// Work of counting the tracks routed through boundaries (the
    /// floors), in expansions.
    FloorExpansions,
    /// Board-wide fallbacks refused because two had failed already
    /// (`hopeless_searches`, switch off and on).
    HopelessSkips,
}

const LEARN_FIELDS: usize = 16;

pub(crate) fn note_search(hard: bool, kind: SearchKind, found: bool, expansions: u64) {
    let slot = &SEARCHES[hard as usize][kind as usize];
    slot[0].fetch_add(1, Relaxed);
    slot[1].fetch_add(found as u64, Relaxed);
    slot[2].fetch_add(expansions, Relaxed);
}

pub(crate) fn note(field: Learn, amount: u64) {
    LEARN[field as usize].fetch_add(amount, Relaxed);
}

/// One kind of search, summed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SearchTotals {
    pub searches: u64,
    pub found: u64,
    pub expansions: u64,
}

/// The process's corridor counters (`corridor_totals`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CorridorTotals {
    /// Per kind (`SearchKind` order), the negotiation's searches
    /// (`soft`) and the hard searches' (the repair, the trouble repair).
    pub soft: [SearchTotals; KINDS],
    pub hard: [SearchTotals; KINDS],
    /// `Learn`'s counters, in its order.
    pub learn: [u64; LEARN_FIELDS],
}

impl CorridorTotals {
    pub fn get(&self, field: Learn) -> u64 {
        self.learn[field as usize]
    }
}

/// This process's corridor counters so far.
pub fn corridor_totals() -> CorridorTotals {
    let read = |mode: usize| {
        let mut totals = [SearchTotals::default(); KINDS];
        for (kind, slot) in totals.iter_mut().enumerate() {
            let counters = &SEARCHES[mode][kind];
            *slot = SearchTotals {
                searches: counters[0].load(Relaxed),
                found: counters[1].load(Relaxed),
                expansions: counters[2].load(Relaxed),
            };
        }
        totals
    };
    let mut learn = [0u64; LEARN_FIELDS];
    for (slot, counter) in learn.iter_mut().zip(&LEARN) {
        *slot = counter.load(Relaxed);
    }
    CorridorTotals {
        soft: read(0),
        hard: read(1),
        learn,
    }
}

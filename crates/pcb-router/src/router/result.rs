// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! What a run returns.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetStatus {
    /// Fewer than two terminals: nothing to do.
    Trivial,
    Routed,
    /// Some terminals could not be connected without violating a rule.
    Partial {
        unconnected_terminals: usize,
    },
    /// A terminal has no legal lattice node at all.
    Unreachable,
}

#[derive(Clone, Debug)]
pub struct RoutingResult {
    pub grid: Grid,
    /// Per lattice node: accumulated congestion history over all layers and
    /// vias. It is large where nets kept fighting for space.
    pub congestion: Vec<f32>,
    pub routes: Vec<NetRoute>,
    pub status: Vec<NetStatus>,
    pub iterations: usize,
    pub expansions: u64,
    pub searches: u64,
    /// Why the routing is not complete, when it is not.
    pub diagnostics: Diagnostics,
    /// A pad's own straight tracks into the fill island its spokes reach
    /// (`Config::island_stubs`), added by this router's finishes: they
    /// change no connection, they only answer KiCad's isolated-island
    /// rule (the verdict's `repairs.island_stubs`).
    pub island_stubs: usize,
    /// The work of this router's manual spokes (`add_manual_spokes`: the
    /// pour analyses and fill probes that repair starved thermals after
    /// the negotiation), over every finish, in seconds of an idle machine.
    /// Not in `expansions`: route mode's ladder charges it beside them.
    pub manual_spokes_work: f64,
    /// The search expansions of this router's first negotiation
    /// iteration: what a fresh router does whatever its work deadline.
    pub first_iteration_expansions: u64,
    /// Why the router's last negotiation ended: `conflict_free`,
    /// `stalled` (no progress within its patience), `work_deadline`,
    /// `negotiation_work_limit`, `probe_work_limit`, `wall_guard`,
    /// `past_wall_deadline`, `hopeless` (a quarter of the nets without any
    /// path), `iteration_cap`; empty when it did not negotiate. A record,
    /// not an input: nothing reads it to route.
    pub negotiation_stop: &'static str,
    /// The work of every pour analysis and fill probe of this router (its
    /// negotiation's pour refresh, the finishing phases' pour checks, the
    /// manual spokes'), in seconds of an idle machine: `manual_spokes_work`
    /// is a part of it. Not in `expansions`: route mode's ladder charges
    /// it beside them.
    pub pour_work: f64,
    /// The work of building this router (`Router::new`: its lattice's
    /// static maps, the nets' terminals and pour masks), modelled from
    /// its counts (`BuildCounts`), in seconds of an idle machine: the
    /// same whether it built its static maps or found them.
    pub build_work: f64,
    /// The parts of `build_work` routers of the same inputs share, keyed
    /// (`Router::shared_build`).
    pub shared_build: super::SharedBuild,
}

/// A machine-readable account of what stood in the way.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    /// Pads that no track of their net's class can enter, with the objects
    /// around them (candidates for what blocks them).
    pub dead_pads: Vec<DeadPad>,
    /// Nets still in conflict when negotiation stopped.
    pub conflicted: Vec<NetId>,
    /// The tiles where nets fought longest, most contested first.
    pub hot_spots: Vec<HotSpot>,
    /// The nets the last hard repair (`Router::finish`) took up: those
    /// incomplete or in conflict when the negotiation ended, with what the
    /// repair did with them. A record, not an input.
    pub repair: Vec<RepairEntry>,
}

/// A net as the hard repair found it (`Diagnostics::repair`).
#[derive(Clone, Debug, PartialEq)]
pub struct RepairEntry {
    pub net: NetId,
    /// Whether the negotiation had joined all its terminals (through other
    /// nets' copper where it was in conflict).
    pub complete: bool,
    /// Whether it was in conflict with other nets' copper.
    pub conflicted: bool,
    /// Why the repair did not search for it again, if it did not:
    /// `hopeless` (the attempt was judged hopeless: no hard repair),
    /// `work_deadline` (the attempt's work deadline had passed), `budget`
    /// (the repair's own budget ran out), `not_run` (the router runs no
    /// repair here); empty where it searched.
    pub skipped: &'static str,
}

#[derive(Clone, Debug)]
pub struct DeadPad {
    pub net: NetId,
    pub label: String,
    pub anchor: crate::geometry::Point,
    pub near: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct HotSpot {
    pub layer: usize,
    pub center: crate::geometry::Point,
    /// Half the tile's side in mm: the tile is `center` +- this.
    pub half_size: f64,
    pub history: f32,
}

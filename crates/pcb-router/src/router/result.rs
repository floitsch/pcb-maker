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
    /// The nets the negotiation gave up (`Board::attachment`'s `give_up`:
    /// a differential pair's links), in the order they went, with why.
    pub released: Vec<super::ReleasedNet>,
    /// Every terminal the result leaves open, with why (`OpenReason`): of
    /// a `Partial` net its unconnected terminals, of an `Unreachable` net
    /// all of them. A record, not an input.
    pub open: Vec<OpenTerminal>,
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
    /// (the repair's own budget ran out); empty where it searched.
    pub skipped: &'static str,
    /// Where it was in conflict when the negotiation ended: the first of
    /// its conflicting nodes (`Router::conflicts`), as the layer (the
    /// layer count for the via map) and the node's centre.
    pub conflict_at: Option<(usize, crate::geometry::Point)>,
    /// The nets whose copper's clearance covered its first conflicting
    /// nodes then (at most `CONFLICT_PARTNERS`, sorted). Empty in a
    /// layout's move trial, which does not look.
    pub conflicted_with: Vec<NetId>,
}

/// At most this many conflicting nodes of a net are looked up for
/// `RepairEntry::conflicted_with`, and at most this many nets named.
pub const CONFLICT_PARTNERS: usize = 4;

/// A terminal the result leaves open (`Diagnostics::open`).
#[derive(Clone, Debug, PartialEq)]
pub struct OpenTerminal {
    pub net: NetId,
    /// Its index in the net's terminals.
    pub terminal: usize,
    /// The terminal's label (`U1.3`).
    pub label: String,
    pub reason: OpenReason,
}

/// Why a terminal stays open, as the router decided it at the point it
/// left the terminal (`Diagnostics::open`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenReason {
    /// The terminal has no lattice node (a dead pad: `dead_pads` says how
    /// open it is). Decided when the net is prepared.
    NoAccess,
    /// The net was never searched: fewer than two of its terminals have a
    /// lattice node (status `Unreachable`).
    NetNotRoutable,
    /// The negotiation's board-wide search found no path to it from the
    /// net's tree, other nets' copper passable at a price: only pads, rule
    /// areas, the outline and fixed copper stop that search
    /// (`NetState::unreachable` when the last negotiation ended).
    NoPathStatic,
    /// Open when the negotiation ended and the hard repair did not search
    /// the net again: `work_deadline`, `hopeless` or `budget`
    /// (`RepairEntry::skipped`).
    RepairSkipped(&'static str),
    /// The hard repair searched, every other net's copper impassable, and
    /// found no path to it: within `corridor` (the trouble repair's
    /// victims only search corridors), `window` or the whole `board`.
    NoPathHard(&'static str),
    /// Joined after the hard repair; a later phase left it open (its name:
    /// `hard_repair` for the repair's own forced connections and links
    /// retried, `trouble_repair`, `clean_up`, `via_reduction`, `bends`,
    /// `thermal_spokes`, `straighten`, `compaction`, `pour_stitching`,
    /// `manual_spokes`).
    Lost(&'static str),
    /// The router counted it joined, but the copper it emits leaves it
    /// apart from the rest of the net (an escape stub dropped at emission:
    /// the copper check that turns a routed net partial).
    CopperCheck,
    /// None of the above was recorded for it.
    Unknown,
}

impl OpenReason {
    /// The name in the reports: the reason, then its detail after a colon
    /// (`no_path_hard:board`, `lost:clean_up`).
    pub fn name(self) -> String {
        match self {
            Self::NoAccess => "no_access".into(),
            Self::NetNotRoutable => "net_not_routable".into(),
            Self::NoPathStatic => "no_path_static".into(),
            Self::RepairSkipped(why) => format!("repair_skipped:{why}"),
            Self::NoPathHard(scope) => format!("no_path_hard:{scope}"),
            Self::Lost(phase) => format!("lost:{phase}"),
            Self::CopperCheck => "copper_check".into(),
            Self::Unknown => "unknown".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeadPad {
    pub net: NetId,
    pub label: String,
    pub anchor: crate::geometry::Point,
    pub near: Vec<String>,
    /// What walls the pad, in words (`Router::dead_pad_wall`).
    pub wall: String,
    /// How open the pad is, the most open of its layers.
    pub access: DeadPadAccess,
}

/// How a dead pad stands (`Router::dead_pad_wall`), from the most closed
/// to the most open; a pad's is the most open of its layers. Only
/// `NoLegalAccess` and `OffLattice` say that by the router's rules no
/// track can reach the pad; the others have a legal track end, and the
/// lattice or the escape's rules left the pad (KiCad, the judge, may
/// accept a track).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeadPadAccess {
    /// Not measured (a pair's launch).
    #[default]
    Unknown,
    /// No point of the pad keeps the clearances for the net's narrowest
    /// track (its neck) on any of its layers.
    NoLegalAccess,
    /// The lattice has no node within the escape's reach of the pad: the
    /// pad lies off the routed area (a part beside the board outline).
    OffLattice,
    /// A track may end on the pad, but no lattice node within the escape's
    /// reach admits the class's track.
    NoLatticeNode,
    /// Nodes admit the class's track, and items block the stubs to the
    /// nearest ones (or the board's edge does).
    StubsBlocked,
    /// A stub from the pad's best point to a node is free, but the
    /// router's stubs start elsewhere (a plain pad's centre), where it is
    /// not.
    StartBlocked,
    /// A stub from the router's own start is free too: the router did not
    /// keep the escape.
    EscapeNotKept,
}

impl DeadPadAccess {
    /// The name in the reports (`dead_pads[].access`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::NoLegalAccess => "no_legal_access",
            Self::OffLattice => "off_lattice",
            Self::NoLatticeNode => "no_lattice_node",
            Self::StubsBlocked => "stubs_blocked",
            Self::StartBlocked => "start_blocked",
            Self::EscapeNotKept => "escape_not_kept",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HotSpot {
    pub layer: usize,
    pub center: crate::geometry::Point,
    /// Half the tile's side in mm: the tile is `center` +- this.
    pub half_size: f64,
    pub history: f32,
}

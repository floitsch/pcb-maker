// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The router's settings.

#[derive(Clone, Debug)]
pub struct Config {
    /// Candidate lattice pitches; the one aligning best with the pads wins.
    pub pitches: Vec<f64>,
    /// Cost of a via, in millimetres of trace.
    pub via_cost: f64,
    /// Cost per 45 degrees of direction change, in millimetres.
    pub bend_cost: f64,
    /// Cost factor for running across a layer's preferred axis. Layers
    /// alternate between horizontal and vertical preference.
    pub against_direction: f64,
    /// The price of sharing a lattice node with another net (the present
    /// congestion factor) in the negotiation's first iteration; a reroute
    /// starts at the price the negotiation reached, at least this.
    pub present_factor: f64,
    /// Factor on the price of sharing after every negotiation iteration,
    /// up to `present_cap`.
    pub present_growth: f64,
    /// Upper bound of the present-congestion factor. Beyond it contested
    /// nodes act as walls the heuristic knows nothing about, and searches
    /// flood; history cost keeps separating the nets instead.
    pub present_cap: f64,
    /// Connect pour nets' pads to their planes first (the pour nets
    /// negotiating among themselves) and keep those stubs and vias fixed
    /// through negotiation. Off: on ColdFire the pre-pass never settles and
    /// fixed stubs left clearance violations.
    pub fixed_plane_stubs: bool,
    /// Keep other nets' tracks off exclusive planes entirely (else they pay
    /// `plane_cut_cost` there, as designers do cut planes locally).
    pub exclusive_planes: bool,
    /// Within this distance of a net's own narrow pads (narrower than its
    /// track plus clearance) the net routes at the board's neck width: the
    /// fan-out of a fine-pitch part. 0 turns it off.
    pub neck_reach: f64,
    /// Plan every net on the tile graph first and confine its lattice
    /// search to that plan's corridor; conflicts teach the plan.
    pub global_routing: bool,
    /// Iterations without fewer conflicted nets that negotiation waits
    /// once the price of sharing is at its cap.
    pub stall_at_cap: usize,
    /// At the price cap, how much fewer conflicted nets count as progress
    /// (a share of the last low; at least two nets when above zero). Zero,
    /// the default, turns it off: any new low resets the stall count, as
    /// below the cap. That can let a board crawl down one net every few
    /// iterations for hundreds of seconds (Sisu: 27-32 conflicted from
    /// iteration 6 to 38), so the layout's in-place router (its first
    /// route and the moves, which compare placements) and its placement
    /// race set 0.1 unless the router settings give `stall_drop`; the
    /// final ladder and the first route's resume keep the default.
    pub stall_drop: f64,
    /// Iterations without fewer conflicted nets after which negotiation
    /// stops below the price cap (`stall_at_cap` at it).
    pub stall_patience: usize,
    /// Stop negotiating when a quarter of the nets have no path at all
    /// after the first iteration (see `Router::hopeless`).
    pub abandon_hopeless: bool,
    /// During negotiation, check every iteration which pads of a pour net
    /// the signals have cut off from the pour's main piece, and route those
    /// to it like off-pour pads (a via into the plane, mostly), so that the
    /// supply nets negotiate with the signals instead of being repaired
    /// after them, when the room is gone (ESC mini: 41 of GND's pads
    /// stranded after the stitching). Pads whose piece rejoins lose their
    /// stub again (a sticky first version left the early iterations' stubs
    /// in the signals' way). Connect rungs with it: ESC mini 22 open
    /// against 27, Hub 123 against 135; tracks mode unchanged; it costs
    /// iterations.
    pub pour_islands: bool,
    /// A pad's thermal spoke counts only where its strip keeps the
    /// clearance from other nets' copper all the way to the fill, as
    /// KiCad's does (`analyze_pours_full`).
    pub spoke_clearance: bool,
    /// Thermal spokes as KiCad draws and counts them (`kicad_spokes`): the
    /// spoke's end at the gap and half the minimum width, its width no
    /// more than the pad's, a piece that joins two pads is no island, and
    /// a pad's own track into the fill counts as a spoke.
    pub kicad_spokes: bool,
    /// A pad that the spoke model finds short of KiCad's two spokes gets a
    /// straight track into the main piece of its pour, which KiCad's DRC
    /// counts as a spoke (`add_manual_spokes`, after the stitching).
    pub manual_spokes: bool,
    /// A pad whose spokes on a layer reach only fill pieces nothing else
    /// of the net touches (KiCad's "spokes connected to isolated island")
    /// and that neither a straight manual spoke nor a stitching via from
    /// the island to the net's main piece on another layer joins to a kept
    /// piece gets a straight track of the pad on the island's layer into
    /// the island (`add_manual_spokes`): KiCad's connectivity joins the
    /// track to the piece it overlaps, the piece has two items and is no
    /// island, and the DRC counts its spokes and the track. It changes no
    /// connection (the island was the pad's already); k30 and mackerel-30's
    /// tracks rungs, whose inner planes the signals cut into islands around
    /// through-hole pads. Off since 2026-10-21 (Florian: it only silences
    /// KiCad's isolated-island finding; the island vias stay on);
    /// `PCB_ROUTER_ISLAND_STUBS=1` turns it on.
    pub island_stubs: bool,
    /// A pad the spoke model finds with no spoke at all into a pour it
    /// lies in, and no track of its own into the fill, gets one straight
    /// track into kept fill all the same (`add_manual_spokes`): the model
    /// is least sure where it sees none (the spoke ends lie in the
    /// narrowest places), KiCad's DRC counts the track only while the pad
    /// is short, and a pad KiCad also gives no spoke is no finding with or
    /// without it. The expansion intake: 22 of 85 new starved thermals on
    /// 40 boards were pads the model saw without a spoke and KiCad with
    /// one (`PCB_ROUTER_NO_SPOKELESS_TRACKS` turns it off).
    pub spokeless_tracks: bool,
    /// Terminals of a net that no path reaches from its tree (a board-wide
    /// search failed, `NetState::unreachable`) are joined among themselves
    /// where they reach each other: a tree of their own, which the net's
    /// count still calls unconnected (KiCad finds one unconnected item
    /// between the trees, not one per terminal). jiran-ble-lite's two
    /// halves share one GND and are joined only through a frame no track
    /// crosses: the right half's 20 GND pads stayed apart, also from each
    /// other (KiCad 20 unconnected items, the designer's own board 1).
    /// `PCB_ROUTER_NO_APART_TREES` turns it off.
    pub apart_trees: bool,
    /// After the clean-up, a pad the spoke model finds starved whose
    /// failed spokes other nets' copper cuts (in the spoke's strip, or
    /// where its end's fill would be) has those nets routed again around
    /// the strips, kept when every net involved stays complete and the
    /// pour nets' starved spokes go down (`free_lost_spokes`;
    /// `PCB_ROUTER_NO_LOST_SPOKES` turns it off).
    pub lost_spoke_repair: bool,
    /// The clean-up keeps a change only if no pour net has more pads off
    /// its main piece after it.
    pub pour_aware_polish: bool,
    /// The same for a via reduction round.
    pub pour_aware_via_reduction: bool,
    /// Whether a pour net stuck in conflict is ripped up whole now and
    /// then like the other nets (`rip_up_whole`).
    pub pour_whole_rip_up: bool,
    /// Fine-pitch pads (narrower than the track plus its clearance, on one
    /// layer, in a row of their kind) get a straight stub outward fixed
    /// before negotiation, up to this long in mm (0, the default, turns it
    /// off). The stubs keep every pad's only way out open: nothing else
    /// may run along the row inside the escape zone. Tried on Tiny Tapeout
    /// (82/108 either way) and ColdFire (205 vs 208 of 209): no gain yet.
    pub escape_stub_mm: f64,
    /// History cost added to a lattice node (and its tile) for every net
    /// still in conflict there after an iteration; also the amplitude of
    /// the noise a `seed` puts into the history.
    pub history_increment: f64,
    /// Negotiation iterations at most (each reroutes the conflicted nets).
    pub max_iterations: usize,
    /// Minimum search window margin around a net's terminals.
    pub window_margin: f64,
    /// Passes of per-net shortening after the board is conflict free.
    pub cleanup_passes: usize,
    /// After the via reduction, take stair-steps and jogs out of the
    /// signal nets' paths where the clearance allows (`straighten`:
    /// shortest octilinear paths between a run's corners with fewer
    /// bends; never longer, no via moves).
    pub straighten: bool,
    /// Via cost while cleaning up. The existing route is always a fallback
    /// there, so a high value removes vias without risking completion.
    pub cleanup_via_cost: f64,
    /// Via cost when connecting to a copper pour: a short stub and a via is
    /// the preferred connection, not a long track to another pad.
    pub plane_via_cost: f64,
    /// Plan every connection on a coarse tile graph first and search the
    /// lattice only inside that corridor (widening it on failure).
    pub corridors: bool,
    /// Cost factor for a trace step on a node covered by another net's
    /// pour, and for a via through one: cutting a plane is expensive.
    pub plane_cut_cost: f64,
    /// Weight of what a cut severs (default 0: off; two-layer boards
    /// only; 1 on the fence lost the 46 mm board's completeness): a step
    /// or via on a node of
    /// another net's pour fill where the fill is a narrow corridor that
    /// joins its pieces pays `plane_cut_cost` times `1 + 20 * weight *
    /// severed`, `severed` being the share of the pour the corridor's
    /// cut would split off from its largest part (`Router::refresh_sever`,
    /// recomputed from the fill model before every batch). It also
    /// lets the pour of a net routed as tracks price the cut (its
    /// fill, as KiCad would pour it, is `covered`).
    pub pour_integrity: f64,
    /// Route every pour net first as a tree on its best-covered layer and
    /// keep that tree fixed: the pour then only adds copper, and signal
    /// routing cannot strand a pad on an island.
    pub plane_skeleton: bool,
    /// Cost multiplier for skeleton steps off the plane layer.
    pub skeleton_bias: f64,
    /// After the board is complete, renegotiate the nets that have vias with
    /// the via cost multiplied by `via_reduction_factor`, this many times,
    /// keeping a round only if nothing opens and vias go down.
    pub via_reduction_rounds: usize,
    /// Factor on the via cost per via reduction round: round k (from 0)
    /// routes at the via cost times this to the power k + 1.
    pub via_reduction_factor: f64,
    /// Heuristic weight while reducing vias (searches there are long
    /// same-layer detours; a little greed keeps them cheap).
    pub via_reduction_weight: f64,
    /// Price of sharing when a via reduction round starts.
    pub via_reduction_present: f64,
    /// Via reduction stops once it has used this many times the main
    /// negotiation's time (at least 60 s).
    pub via_reduction_budget: f64,
    /// Seconds via reduction may take at most, whatever the negotiation
    /// took (default 300 s; the KiCad ladder sets it from what is left of
    /// its budget: a 210 x 170 mm board routed clean in 130 s and then
    /// spent the runner's remaining 770 s polishing).
    pub via_reduction_seconds: f64,
    /// The via reduction keeps its budget within a round too: a round
    /// whose cost (projected from the previous round) exceeds what is left
    /// does not start, and a round's negotiation and clean-up get only
    /// what is left. Without it a round started with budget left ran to
    /// its end (k30-SBC: a 60 s budget, one round of 182 s of work and a
    /// clean-up after it; CyberKeeb2040: 186-206 s for 87-117 s).
    pub strict_via_budget: bool,
    /// The whole run's work budget (the ladder's or the layout's, in work
    /// seconds) and what was left of it when this router started (its own
    /// work since counted against it). With more than half of the run's
    /// budget unspent when the via reduction starts, it may use up to a
    /// third of what is left, and at most 300 s (k30-SBC's round of 182 s
    /// of work, the one that took it from 414 to 354 vias, fits by right
    /// then); with less, its own budget applies strictly.
    pub run_work: Option<(f64, f64)>,
    /// A via-reduction round is kept only if it starves no more thermal
    /// pads (`starved_pads`) than before. Off: the router's count is not
    /// KiCad's (mackerel-08: 0 against 5); on the quick tier it took
    /// starved thermals from 13 to 10 but cost k30-SBC its 55-via round
    /// and 5 more starved pads in KiCad's count.
    pub via_reduction_keeps_spokes: bool,
    /// Seconds the clean-up may take at most: no batch starts later than
    /// this, nor later than the negotiation took (at least 60 s). A
    /// 130 x 90 mm six-layer board spent 470 s in clean-up.
    pub cleanup_seconds: f64,
    /// Cost factor for another net's step on the ring around a pad that
    /// joins its pour by thermal spokes (copper there starves the spokes).
    pub thermal_guard_cost: f64,
    /// A negotiation that has not converged after this many seconds is
    /// handed to the resolution step as it is.
    pub negotiation_seconds: f64,
    /// Search expansions one negotiation may spend (0: no limit but the
    /// seconds). Unlike seconds, independent of the machine's load.
    pub negotiation_expansions: u64,
    /// Route spatially disjoint nets of one iteration in parallel.
    pub parallel: bool,
    /// Values above 1 trade path optimality for search speed.
    pub heuristic_weight: f64,
    /// Perturbs the net order and seeds the history with noise of one
    /// history increment, so the same board can be routed several ways
    /// (`None`: the deterministic order, or `PCB_ROUTER_SEED`'s).
    pub seed: Option<u64>,
    /// Print progress to stderr: one line per negotiation iteration and
    /// what the clean-up, the via reduction and the spoke repairs did (the
    /// KiCad router always sets it).
    pub verbose: bool,
    /// A wall-clock time by which routing returns what it has, however
    /// much work its budgets still allow: every budget counts as spent
    /// from then on (an agent waits for the result).
    pub deadline: Option<std::time::Instant>,
    /// The work (this router's own, in seconds of an idle machine) at
    /// which routing returns what it has, as at `deadline`: the caller's
    /// deadline counted in work, so that where a run stops does not depend
    /// on how busy the machine is (the layout sets it from its own budget;
    /// `deadline` stays as the guard against load).
    pub deadline_work: Option<f64>,
    /// Rules beyond the net classes (`crate::rules`): per-net width
    /// floors, clearances between sets of nets, per item type and per
    /// layer, and the same-net hole-to-hole spacing. `None` (or empty
    /// rules): the classes alone, as before.
    pub net_rules: Option<std::sync::Arc<crate::rules::NetRules>>,
    /// Share the static maps (the outline, every obstacle's nodes, the
    /// tight steps of every class) with the other routers built from the
    /// same board, lattice and classes (`hold_static_maps`): a router
    /// then builds only its own maps; what it counts is the same either
    /// way (`build_work`, with the shared parts keyed in `shared_build`
    /// for the ladder to charge once). False builds them anew and keys
    /// nothing (every router's build charged whole).
    pub share_static_maps: bool,
    /// The work deadline (`deadline_work`) and the negotiation's work
    /// limit (`negotiation_expansions`, in seconds at the reference's
    /// rate) count this router's pour analyses and fill probes beside its
    /// search, the deadline its build too: all of its work as route
    /// mode's ladder charges it. False: its search alone. The ladder sets
    /// it where it has no proportion of search to go by (its first
    /// attempt, the rung probes).
    pub limits_count_all: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pitches: vec![0.127, 0.125, 0.1],
            via_cost: 8.0,
            bend_cost: 0.1,
            against_direction: 1.6,
            present_factor: 0.5,
            present_growth: 1.5,
            present_cap: 1.0e4,
            // At the price cap, twelve iterations without a new low of
            // conflicted nets end a negotiation: after that the rest seldom
            // gains (Sisu stalled at 25 conflicted nets from iteration 21
            // and negotiated on to 47; 54 % of its expansions came after
            // iteration 20). Below the cap the rising price still moves
            // nets (a three-track channel needs more than twelve).
            stall_at_cap: 12,
            stall_drop: 0.0,
            stall_patience: 25,
            abandon_hopeless: true,
            pour_islands: true,
            spoke_clearance: true,
            kicad_spokes: true,
            manual_spokes: true,
            island_stubs: std::env::var_os("PCB_ROUTER_ISLAND_STUBS").is_some(),
            spokeless_tracks: std::env::var_os("PCB_ROUTER_NO_SPOKELESS_TRACKS").is_none(),
            apart_trees: std::env::var_os("PCB_ROUTER_NO_APART_TREES").is_none(),
            lost_spoke_repair: std::env::var_os("PCB_ROUTER_NO_LOST_SPOKES").is_none(),
            pour_aware_polish: true,
            pour_aware_via_reduction: true,
            pour_whole_rip_up: true,
            escape_stub_mm: 0.0,
            global_routing: false,
            neck_reach: 1.5,
            fixed_plane_stubs: false,
            exclusive_planes: false,
            history_increment: 0.3,
            max_iterations: 80,
            window_margin: 10.0,
            cleanup_passes: 4,
            straighten: std::env::var_os("PCB_ROUTER_NO_STRAIGHTEN").is_none(),
            cleanup_via_cost: 25.0,
            plane_via_cost: 2.0,
            corridors: true,
            parallel: true,
            via_reduction_rounds: 3,
            via_reduction_factor: 2.0,
            via_reduction_weight: 1.0,
            via_reduction_present: 0.5,
            via_reduction_budget: 3.0,
            via_reduction_seconds: 300.0,
            strict_via_budget: true,
            run_work: None,
            via_reduction_keeps_spokes: false,
            cleanup_seconds: 180.0,
            thermal_guard_cost: 10.0,
            negotiation_seconds: 900.0,
            negotiation_expansions: 0,
            plane_cut_cost: 3.0,
            pour_integrity: 0.0,
            plane_skeleton: false,
            skeleton_bias: 6.0,
            heuristic_weight: 1.0,
            seed: None,
            verbose: false,
            deadline: None,
            deadline_work: None,
            net_rules: None,
            share_static_maps: true,
            limits_count_all: false,
        }
    }
}

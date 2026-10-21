// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The phases in order, the budgets, the clocks and the results.

use super::*;

impl Router {
    /// Routes everything from the current state and produces the result.
    pub fn run(mut self) -> RoutingResult {
        self.run_in_place()
    }

    /// Like `run`, keeping the router for later `update` calls.
    pub fn run_in_place(&mut self) -> RoutingResult {
        self.run_in_place_polished(true)
    }

    /// `run_in_place`; without `polish` the clean-up and the via reduction
    /// are skipped (for trials judged on what stays open, which the polish
    /// does not change; a later `reroute(true)` polishes).
    pub fn run_in_place_polished(&mut self, polish: bool) -> RoutingResult {
        let order = self.routing_order();
        let started = std::time::Instant::now();
        self.fix_escapes(&order);
        self.route_skeletons(&order);
        self.fix_plane_stubs(&order);
        if self.config.global_routing {
            self.plan_globally(&order);
        }
        self.profile.prepare += started.elapsed().as_secs_f64();
        self.negotiate(&order, order.clone());
        self.finish_polished(&order, polish)
    }

    /// The first part of `run_in_place`: prepares the nets and negotiates
    /// for at most `seconds`, keeping every bit of state, so that `resume`
    /// can go on from here. Returns how many nets are still conflicted or
    /// incomplete (the measure a caller ranks probes by).
    pub fn probe(&mut self, seconds: f64) -> usize {
        let order = self.routing_order();
        let started = std::time::Instant::now();
        self.fix_escapes(&order);
        self.route_skeletons(&order);
        self.fix_plane_stubs(&order);
        if self.config.global_routing {
            self.plan_globally(&order);
        }
        self.profile.prepare += started.elapsed().as_secs_f64();
        let limit = (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
        );
        self.config.negotiation_seconds = seconds * GUARD;
        self.config.negotiation_expansions = (seconds * expansions_per_second()) as u64;
        self.negotiate(&order, order.clone());
        (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
        ) = limit;
        self.unfinished()
    }

    /// After `probe`: negotiates on for at most `seconds` where the probe
    /// stopped (its history and its price of sharing: starting the price
    /// low again undid the probe's progress, and a board a fresh run
    /// completes stayed at 20 conflicted nets), then finishes and polishes
    /// as `run` does.
    pub fn resume(&mut self, seconds: f64) -> RoutingResult {
        self.resume_polished(seconds, true)
    }

    /// The clean-up, the via reduction and the pour stitching on the
    /// routes as they are, without negotiating again (a renegotiation of
    /// the open nets at a short patience left jetson-nano with 28 open
    /// where it had 22).
    pub fn polish(&mut self) -> RoutingResult {
        let order = self.routing_order();
        self.finish_polished(&order, true)
    }

    /// `resume`; without `polish` as in `run_in_place_polished`.
    pub fn resume_polished(&mut self, seconds: f64, polish: bool) -> RoutingResult {
        let limit = (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
        );
        self.config.negotiation_seconds = seconds * GUARD;
        self.config.negotiation_expansions = (seconds * expansions_per_second()) as u64;
        self.hopeless = false;
        let order = self.routing_order();
        let pending: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| {
                let state = &self.nets[*net as usize];
                !state.complete || !self.conflicts(*net).is_empty()
            })
            .collect();
        let present = self.present_reached.max(self.config.present_factor as f32);
        self.negotiate_from(&order, pending, present);
        let result = self.finish_polished(&order, polish);
        (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
        ) = limit;
        result
    }

    /// The router's configuration, to change between runs (the layout
    /// trials at a short patience, the final resume at the full one).
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Search expansions so far (all phases, all threads).
    pub fn expansions(&self) -> u64 {
        self.scratch.expansions
    }

    /// The work done so far, in seconds of an idle machine (search
    /// expansions at `expansions_per_second()`): budgets measured in it do
    /// not depend on how busy the machine is.
    pub(super) fn work_seconds(&self) -> f64 {
        self.scratch.expansions as f64 / expansions_per_second()
    }

    pub(super) fn phase_mark(&self) -> PhaseMark {
        (
            self.scratch.expansions as f64 / expansions_per_second(),
            self.pour_work_seconds(),
            self.scratch
                .pour_nanos
                .load(std::sync::atomic::Ordering::Relaxed),
            std::time::Instant::now(),
            self.scratch.expansions,
        )
    }

    /// Adds what was done since `mark` to the profile's `phase`.
    pub(super) fn phase_charge(&mut self, phase: usize, mark: PhaseMark) {
        let now = self.phase_mark();
        self.profile.phase_search[phase] += now.0 - mark.0;
        self.profile.phase_pour_work[phase] += now.1 - mark.1;
        self.profile.phase_pour_wall[phase] += (now.2 - mark.2) as f64 / 1.0e9;
        self.profile.phase_wall[phase] += mark.3.elapsed().as_secs_f64();
        self.profile.phase_expansions[phase] += now.4 - mark.4;
    }

    /// The pour analyses' work so far, in seconds of an idle machine
    /// (`pour_analysis_nanos`, and the fill probes' `probe_nanos`: the
    /// reference's nanoseconds, scaled to this process's rate by
    /// `reference_time_scale`).
    pub(super) fn pour_work_seconds(&self) -> f64 {
        let nanos = self
            .scratch
            .pour_units
            .load(std::sync::atomic::Ordering::Relaxed)
            + self.probe_nanos();
        nanos as f64 / 1.0e9 * reference_time_scale()
    }

    /// The fill probes' work so far, in nanoseconds of an idle machine:
    /// their items, queries and polygon vertices.
    pub(super) fn probe_nanos(&self) -> u64 {
        let load = |counter: &std::sync::atomic::AtomicU64| {
            counter.load(std::sync::atomic::Ordering::Relaxed)
        };
        load(&self.scratch.probe_ops) * PROBE_ITEM_NANOS
            + load(&self.scratch.probe_queries) * PROBE_QUERY_NANOS
            + load(&self.scratch.probe_vertices) * PROBE_VERTEX_NANOS
    }

    /// The work the phase budgets (`clock`, `spent`) count: the search's
    /// and the pour analyses'. A clean-up on a pour board spends much of
    /// its time checking that a batch strands no pour pad (Sisu: 21-35 s
    /// of a 61-66 s budget); counted as search alone, its budget ran on in
    /// wall clock with the load, and under load the wall guard ended it.
    pub(super) fn budget_work(&self) -> f64 {
        self.work_seconds() + self.pour_work_seconds()
    }

    /// A budget's start: the work and the wall clock at this point.
    pub(super) fn clock(&self) -> (f64, std::time::Instant) {
        (self.budget_work(), std::time::Instant::now())
    }

    /// Whether `budget` (work seconds) is spent since `clock`; the wall
    /// clock stops it too, at `GUARD` times the budget and at least
    /// `BUDGET_GUARD_FLOOR` (`budget_guard_fires`).
    pub(super) fn spent(&self, clock: &(f64, std::time::Instant), budget: f64) -> bool {
        if self.budget_work() - clock.0 > budget || self.past_work_deadline() {
            return true;
        }
        if self.wall_guard() && budget_guard_fires(clock.1.elapsed().as_secs_f64(), budget) {
            note_wall_cut(
                "budget guard",
                format_args!(
                    "{:.0} s of wall clock for a budget of {budget:.0} s of work, {:.0} s of it done",
                    clock.1.elapsed().as_secs_f64(),
                    self.budget_work() - clock.0
                ),
            );
            return true;
        }
        self.past_wall_deadline()
    }

    /// Whether the wall clock guards the work budgets at `GUARD` times
    /// them: only without a caller's deadline (route mode), which is
    /// otherwise the one wall guard, so that the work alone decides the
    /// routes; and not under `PCB_ROUTER_NO_WALL_GUARD` (`wall_guard`).
    pub(super) fn wall_guard(&self) -> bool {
        self.config.deadline.is_none() && wall_guard()
    }

    /// Whether the caller's deadline has passed: its work
    /// (`deadline_work`) or, on a machine too busy for it, its wall clock.
    pub fn past_deadline(&self) -> bool {
        self.past_work_deadline() || self.past_wall_deadline()
    }

    /// Whether this router's work has reached the caller's `deadline_work`.
    pub(super) fn past_work_deadline(&self) -> bool {
        self.config
            .deadline_work
            .is_some_and(|limit| self.work_seconds() >= limit)
    }

    /// Whether the caller's wall-clock deadline has passed (noted as a cut:
    /// the work deadline, when there is one, did not stop it first).
    pub(super) fn past_wall_deadline(&self) -> bool {
        let past = self
            .config
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline);
        if past {
            note_wall_cut(
                "deadline",
                format_args!("{:.0} s of this router's work done", self.work_seconds()),
            );
        }
        past
    }

    /// The work this router has done, in seconds of an idle machine
    /// (search expansions at `expansions_per_second()`).
    pub fn work(&self) -> f64 {
        self.work_seconds()
    }

    /// Routable nets in conflict or not complete, plus the pads of pour
    /// nets that their pour does not reach yet (a pour net counts as
    /// complete while its pads touch the pour anywhere; whether the pieces
    /// they touch hang together only shows when it is stitched, and a probe
    /// that ignores it ranks a rung that ends with 120 pads stranded first).
    pub fn unfinished(&self) -> usize {
        let mut count = 0;
        for net in 0..self.nets.len() as NetId {
            let state = &self.nets[net as usize];
            if !state.routable {
                continue;
            }
            if !state.complete || !self.conflicts(net).is_empty() {
                count += 1;
            }
            if !state.plane.is_empty() {
                let (pours, mut parent, main) = self.analyze_pours(net);
                let terminal_base = pours.pieces + 1;
                count += (0..state.terminal_nodes.len())
                    .filter(|terminal| find(&mut parent, terminal_base + terminal) != main)
                    .count();
            }
        }
        count
    }

    /// `finish`, skipping the clean-up and the via reduction without
    /// `polish`.
    pub(super) fn finish_polished(&mut self, order: &[NetId], polish: bool) -> RoutingResult {
        let (cleanup, rounds) = (self.config.cleanup_passes, self.config.via_reduction_rounds);
        if !polish {
            self.config.cleanup_passes = 0;
            self.config.via_reduction_rounds = 0;
        }
        let result = self.finish(order);
        self.config.cleanup_passes = cleanup;
        self.config.via_reduction_rounds = rounds;
        result
    }

    /// Registers a hook called after every negotiation iteration with the
    /// iteration count and the routing as it stands (see `snapshot`).
    pub fn set_frame_hook(&mut self, hook: FrameHook) {
        self.frame_hook = Some(hook);
    }

    /// The routing as it stands: every net materialized as it is, conflicts
    /// and all, without cleanup, stub checks or congestion.
    pub fn snapshot(&self) -> RoutingResult {
        let routes = (0..self.board.nets.len() as NetId)
            .map(|net| self.materialize(net).0)
            .collect();
        RoutingResult {
            grid: self.grid.clone(),
            congestion: Vec::new(),
            routes,
            status: self.statuses(),
            iterations: self.iterations,
            expansions: self.scratch.expansions,
            searches: self.scratch.searches,
            diagnostics: Diagnostics::default(),
            island_stubs: self.island_stubs,
        }
    }

    /// Dead pads, the last conflicted nets and the hottest tiles.
    pub(super) fn diagnostics(&self) -> Diagnostics {
        let mut dead_pads = Vec::new();
        for (net, state) in self.nets.iter().enumerate() {
            let description = &self.board.nets[net];
            let class = self.board.classes[description.class];
            for (index, nodes) in state.terminal_nodes.iter().enumerate() {
                if !nodes.is_empty() || state.on_plane.get(index).copied().unwrap_or(false) {
                    continue;
                }
                let terminal = &description.terminals[index];
                let pad = &self.board.obstacles[terminal.pad];
                let reach = pad
                    .shape
                    .aabb()
                    .inflated(class.trace_width / 2.0 + class.clearance + self.grid.pitch);
                let mut near: Vec<String> = self
                    .board
                    .obstacles
                    .iter()
                    .filter(|other| {
                        other.net != Some(net as NetId)
                            && other.layers & terminal.layers != 0
                            && other.shape.aabb().intersects(reach)
                    })
                    .map(|other| other.label.clone())
                    .collect();
                near.sort();
                near.dedup();
                near.truncate(8);
                dead_pads.push(DeadPad {
                    net: net as NetId,
                    label: terminal.label.clone(),
                    anchor: terminal.anchor,
                    near,
                });
            }
        }
        let mut hot_spots: Vec<HotSpot> = Vec::new();
        for (layer, history) in self.tile_history.iter().enumerate() {
            for (tile, value) in history.iter().enumerate() {
                if *value > 0.0 {
                    let x = (tile % self.tiles_x) as f64 * TILE as f64 + TILE as f64 / 2.0;
                    let y = (tile / self.tiles_x) as f64 * TILE as f64 + TILE as f64 / 2.0;
                    hot_spots.push(HotSpot {
                        layer,
                        center: [
                            self.grid.origin[0] + x * self.grid.pitch,
                            self.grid.origin[1] + y * self.grid.pitch,
                        ],
                        history: *value,
                    });
                }
            }
        }
        hot_spots.sort_by(|a, b| b.history.total_cmp(&a.history));
        hot_spots.truncate(10);
        Diagnostics {
            dead_pads,
            conflicted: self.last_conflicted.clone(),
            hot_spots,
        }
    }

    pub(super) fn statuses(&self) -> Vec<NetStatus> {
        (0..self.board.nets.len())
            .map(|net| {
                let state = &self.nets[net];
                if self.board.nets[net].terminals.len() < 2 && state.plane.is_empty() {
                    NetStatus::Trivial
                } else if !state.routable {
                    NetStatus::Unreachable
                } else if state.complete {
                    NetStatus::Routed
                } else {
                    NetStatus::Partial {
                        unconnected_terminals: state
                            .connected
                            .iter()
                            .filter(|connected| !**connected)
                            .count(),
                    }
                }
            })
            .collect()
    }
}

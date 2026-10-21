// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Finishing a run: fingerprints, the hard repair's driver, forced connections.

use super::*;

impl Router {
    /// Debug: a fingerprint of every net's branches and stamps.
    pub(super) fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |value: u64| {
            hash ^= value;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        };
        for state in &self.nets {
            for branch in &state.branches {
                for node in &branch.nodes {
                    mix(((node.layer as u64) << 32) | node.cell as u64);
                }
                mix(u64::MAX);
            }
            for (map, cell) in &state.stamped {
                mix(((*map as u64) << 32) | *cell as u64);
            }
        }
        for history in &self.history {
            for value in history {
                mix(value.to_bits() as u64);
            }
        }
        if std::env::var_os("PCB_ROUTER_FINGERPRINT_ALL").is_some() {
            for map in &self.occupancy {
                for value in map {
                    mix(*value as u64);
                }
            }
            for state in &self.nets {
                for layer in &state.plane {
                    for value in layer {
                        mix(*value as u64);
                    }
                }
                for value in &state.on_plane {
                    mix(*value as u64);
                }
                mix(state.complete as u64);
                for value in &state.connected {
                    mix(*value as u64);
                }
            }
        }
        hash
    }

    pub(super) fn finish(&mut self, order: &[NetId]) -> RoutingResult {
        if std::env::var_os("PCB_ROUTER_FINGERPRINT").is_some() {
            eprintln!("fingerprint before resolve: {:016x}", self.fingerprint());
        }
        let resolve_started = std::time::Instant::now();
        let mark = self.phase_mark();
        self.resolve_remaining(order);
        self.phase_charge(1, mark);
        self.profile.resolve += resolve_started.elapsed().as_secs_f64();
        let cleanup_started = std::time::Instant::now();
        let mark = self.phase_mark();
        let improved = if self.hopeless {
            0
        } else {
            self.clean_up(order)
        };
        self.phase_charge(2, mark);
        let cleaned = cleanup_started.elapsed().as_secs_f64();
        self.profile.cleanup += cleaned;
        if std::env::var_os("PCB_ROUTER_FINGERPRINT").is_some() {
            eprintln!("fingerprint after clean-up: {:016x}", self.fingerprint());
        }
        let reduction_started = std::time::Instant::now();
        let mark = self.phase_mark();
        self.in_via_reduction = true;
        self.reduce_vias(order);
        self.in_via_reduction = false;
        self.phase_charge(3, mark);
        self.profile.via_reduction += reduction_started.elapsed().as_secs_f64();
        if std::env::var_os("PCB_ROUTER_FINGERPRINT").is_some() {
            eprintln!(
                "fingerprint after via reduction: {:016x}",
                self.fingerprint()
            );
        }
        // A repair for the final board, like the via reduction: a trial
        // (clean-up and via reduction off) skips it (PolyKybd left: 313 s
        // in the first route, mostly hard searches flooding the lattice).
        let spokes_started = std::time::Instant::now();
        let mark = self.phase_mark();
        if self.config.cleanup_passes > 0 || self.config.via_reduction_rounds > 0 {
            self.free_thermal_spokes();
            self.free_lost_spokes();
        }
        self.phase_charge(4, mark);
        self.profile.thermal_spokes += spokes_started.elapsed().as_secs_f64();
        if std::env::var_os("PCB_ROUTER_FINGERPRINT").is_some() {
            eprintln!(
                "fingerprint after thermal spokes: {:016x}",
                self.fingerprint()
            );
        }
        if self.config.verbose {
            eprintln!(
                "clean up {cleaned:.2}s, via reduction {:.2}s",
                cleanup_started.elapsed().as_secs_f64() - cleaned
            );
        }
        let plane_nets: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| !self.nets[*net as usize].plane.is_empty())
            .collect();
        for net in plane_nets {
            // The skeleton is trimmed first so that stitching only adds the
            // vias the pour pieces really need.
            let stitching_started = std::time::Instant::now();
            let mark = self.phase_mark();
            let trimmed = self.trim_skeleton(net);
            if std::env::var_os("PCB_ROUTER_FINGERPRINT").is_some() {
                let (pours, _, main, spoke_islands) = self.analyze_pours_full(net, &|_| false);
                eprintln!(
                    "fingerprint before stitching {}: {:016x}, {} pieces, main {main}, spoke islands {:?}",
                    self.board.nets[net as usize].name,
                    self.fingerprint(),
                    pours.pieces,
                    spoke_islands
                );
            }
            let stitches = self.stitch_pours(net, order);
            let tails = self.trim_dangling_plane_ends(net);
            if self.config.verbose && tails > 0 {
                eprintln!(
                    "pour {}: {tails} nodes of plane ends without fill trimmed",
                    self.board.nets[net as usize].name
                );
            }
            self.profile.stitching += stitching_started.elapsed().as_secs_f64();
            self.phase_charge(5, mark);
            if self.config.verbose {
                eprintln!(
                    "pour {}: {stitches} stitching vias, {trimmed} skeleton nodes trimmed, complete {}",
                    self.board.nets[net as usize].name, self.nets[net as usize].complete
                );
            }
        }
        if self.config.manual_spokes
            && self.config.kicad_spokes
            && (self.config.cleanup_passes > 0 || self.config.via_reduction_rounds > 0)
        {
            let started = std::time::Instant::now();
            let mark = self.phase_mark();
            let added = self.add_manual_spokes();
            self.phase_charge(6, mark);
            let now = self.phase_mark();
            self.manual_spokes_work += (now.0 - mark.0) + (now.1 - mark.1);
            self.profile.thermal_spokes += started.elapsed().as_secs_f64();
            if self.config.verbose && added > 0 {
                let now = self.phase_mark();
                eprintln!(
                    "manual spokes: {added} tracks from starved pads into their pours ({:.1} s of wall clock, {:.1} s of it pour analyses, {:.1} s of pour work)",
                    started.elapsed().as_secs_f64(),
                    (now.2 - mark.2) as f64 / 1.0e9,
                    now.1 - mark.1,
                );
            }
        }
        if self.config.verbose {
            eprintln!(
                "cleanup: {improved} improvements, {:.2}s",
                cleanup_started.elapsed().as_secs_f64()
            );
        }

        if let Some(directory) = std::env::var_os("PCB_ROUTER_POUR_DUMP") {
            static DUMPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let number = DUMPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let directory = std::path::Path::new(&directory).join(format!("finish-{number:03}"));
            if let Err(error) = self.dump_pours(&directory) {
                eprintln!("pour dump {}: {error}", directory.display());
            } else {
                let vias: usize = (0..self.nets.len())
                    .map(|net| {
                        self.nets[net]
                            .branches
                            .iter()
                            .map(|branch| {
                                branch
                                    .nodes
                                    .windows(2)
                                    .filter(|pair| {
                                        pair[0].cell == pair[1].cell
                                            && pair[0].layer != pair[1].layer
                                    })
                                    .count()
                            })
                            .sum::<usize>()
                    })
                    .sum();
                eprintln!("pour dump {}: {vias} via steps", directory.display());
            }
        }
        let emit_started = std::time::Instant::now();
        let mut status = self.statuses();
        let (mut routes, mut stubs): (Vec<NetRoute>, Vec<Vec<usize>>) = (0..self.board.nets.len()
            as NetId)
            .map(|net| self.materialize(net))
            .unzip();
        // Stubs were only checked against fixed objects; drop any that comes
        // too close to routed copper of another net, also by the rules
        // beyond the classes (tuxkeychain: a 0.3 mm GND stub along a
        // narrower pad, 0.104 mm from HPL's track and stub under a
        // 0.127 mm rule between outer-layer tracks over classes of 0.1).
        loop {
            let mut offending: Vec<(usize, usize)> = Vec::new();
            for violation in
                crate::verify::verify_with_rules(&self.board, &routes, self.class_rules.as_deref())
            {
                let involved = violation
                    .segment
                    .map(|segment| (violation.net as usize, segment))
                    .into_iter()
                    .chain(
                        violation
                            .other_segment
                            .map(|(net, segment)| (net as usize, segment)),
                    );
                for (net, segment) in involved {
                    if stubs[net].contains(&segment) && !offending.contains(&(net, segment)) {
                        offending.push((net, segment));
                    }
                }
            }
            if offending.is_empty() {
                break;
            }
            offending.sort_by(|left, right| right.cmp(left));
            for (net, segment) in offending {
                routes[net].segments.remove(segment);
                stubs[net].retain(|stub| *stub != segment);
                for stub in &mut stubs[net] {
                    if *stub > segment {
                        *stub -= 1;
                    }
                }
            }
        }
        // The emitted copper must join what the status says. A net routed
        // with tracks alone is checked as KiCad's connectivity sees it
        // (`verify::copper_components`: copper of the net overlapping on a
        // shared layer): a terminal its copper leaves apart (a stub
        // dropped above) makes it partial, so that a complete route by the
        // router's count is one KiCad finds connected. A pour net's
        // connections are the pour model's.
        for net in 0..self.board.nets.len() {
            if !matches!(status[net], NetStatus::Routed) || !self.nets[net].plane.is_empty() {
                continue;
            }
            let components =
                crate::verify::copper_components(&self.board, net as NetId, &routes[net]);
            let mut sizes: HashMap<usize, usize> = HashMap::new();
            for component in &components {
                *sizes.entry(*component).or_default() += 1;
            }
            let largest = sizes.values().copied().max().unwrap_or(0);
            let apart = components.len() - largest;
            if apart > 0 {
                if self.config.verbose || std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                    eprintln!(
                        "copper check: {} leaves {apart} of {} terminals apart",
                        self.board.nets[net].name,
                        components.len()
                    );
                }
                status[net] = NetStatus::Partial {
                    unconnected_terminals: apart,
                };
            }
        }
        let mut congestion = vec![0.0f32; self.grid.cells()];
        for layer in &self.history {
            for (total, value) in congestion.iter_mut().zip(layer) {
                *total += value;
            }
        }
        let diagnostics = self.diagnostics();
        self.profile.emit += emit_started.elapsed().as_secs_f64();
        if self.config.verbose {
            eprintln!(
                "{}",
                self.profile.line(self.search_seconds, self.stamp_seconds)
            );
            eprintln!("{}", self.profile.work_line());
            eprintln!("{}", self.profile.rate_line());
        }
        RoutingResult {
            diagnostics,
            congestion,
            routes,
            status,
            iterations: self.iterations,
            expansions: self.scratch.expansions,
            searches: self.scratch.searches,
            grid: self.grid.clone(),
            island_stubs: self.island_stubs,
            manual_spokes_work: self.manual_spokes_work,
            first_iteration_expansions: self.first_iteration_expansions,
            pour_work: self.pour_work_seconds(),
            build_work: self.build_work,
        }
    }

    /// Completes `net` by force: it is routed straight through whatever is
    /// in its way, and exactly the nets it collides with are rerouted around
    /// it. The result is kept only if every net involved ends up complete
    /// and conflict free; otherwise everything is restored.
    pub(super) fn force_connect(&mut self, net: NetId, order: &[NetId]) -> bool {
        type Saved = (NetId, Vec<Branch>, Vec<bool>, bool);
        let save = |router: &Self, net: NetId| -> Saved {
            let state = &router.nets[net as usize];
            (
                net,
                state.branches.clone(),
                state.connected.clone(),
                state.complete,
            )
        };
        let mut saved = vec![save(self, net)];
        let mut involved: Vec<NetId> = vec![net];
        self.unstamp(net);
        self.route_net_seq(net, 50.0, false, 4.0);
        self.nets[net as usize].blocked = false;
        self.stamp(net);
        let mut ok = self.nets[net as usize].complete;
        // A small negotiation among the nets this connection displaces: they
        // may push each other around, but the forced net itself stays put.
        let mut present = 2.0f32;
        for _ in 0..16 {
            if !ok {
                break;
            }
            let conflicted: Vec<NetId> = order
                .iter()
                .copied()
                .filter(|other| *other != net && !self.conflicts(*other).is_empty())
                .collect();
            if conflicted.is_empty() {
                break;
            }
            // A forced connection is a local repair; when it cascades
            // through the board the copper is simply not there.
            if involved.len() + conflicted.len() > 24 {
                ok = false;
                break;
            }
            for victim in &conflicted {
                if !involved.contains(victim) {
                    involved.push(*victim);
                    saved.push(save(self, *victim));
                }
                self.rip_up_conflicted(*victim);
                self.route_net_seq(*victim, present, false, 4.0);
                self.stamp(*victim);
            }
            present = (present * 1.5).min(self.config.present_cap as f32);
        }
        if ok {
            ok = involved.iter().all(|member| {
                self.nets[*member as usize].complete && self.conflicts(*member).is_empty()
            }) && self.conflicts(net).is_empty();
        }
        if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
            let incomplete: Vec<&str> = involved
                .iter()
                .filter(|member| !self.nets[**member as usize].complete)
                .map(|member| self.board.nets[*member as usize].name.as_str())
                .collect();
            let conflicted: Vec<&str> = involved
                .iter()
                .filter(|member| !self.conflicts(**member).is_empty())
                .map(|member| self.board.nets[*member as usize].name.as_str())
                .collect();
            eprintln!(
                "force_connect {}: ok {ok}, {} involved, incomplete {incomplete:?}, conflicted {conflicted:?}",
                self.board.nets[net as usize].name,
                involved.len()
            );
        }
        if !ok {
            for (restored, ..) in &saved {
                self.unstamp(*restored);
            }
            for (restored, branches, connected, complete) in saved {
                let state = &mut self.nets[restored as usize];
                state.branches = branches;
                state.connected = connected;
                state.complete = complete;
                self.stamp(restored);
            }
        }
        ok
    }

    /// Drops the parts of the plane skeleton that lie in solid pour copper:
    /// the pour provides that connection itself. What remains (hops to pads
    /// off the pour, runs squeezed between foreign copper) ends on the pour.
    /// Returns the number of nodes removed.
    pub(super) fn trim_skeleton(&mut self, net: NetId) -> usize {
        let fixed = self.nets[net as usize].fixed;
        if fixed == 0 {
            return 0;
        }
        let (pours, _, _) = self.analyze_pours(net);
        let grid = self.grid.clone();
        let redundant =
            |node: &Node| pours.solid_around(&grid, node.layer as usize, node.cell as usize, 1);
        self.unstamp(net);
        let state = &mut self.nets[net as usize];
        let branches = std::mem::take(&mut state.branches);
        let mut kept: Vec<Branch> = Vec::new();
        let mut removed = 0;
        for (index, branch) in branches.into_iter().enumerate() {
            if index >= fixed {
                kept.push(branch);
                continue;
            }
            let drop: Vec<bool> = branch.nodes.iter().map(redundant).collect();
            let mut start = 0;
            while start < branch.nodes.len() {
                if drop[start] {
                    start += 1;
                    continue;
                }
                let mut end = start;
                while end + 1 < branch.nodes.len() && !drop[end + 1] {
                    end += 1;
                }
                // One pour node on each trimmed side keeps the contact.
                let first = if start > 0 { start - 1 } else { 0 };
                let last = if end + 1 < branch.nodes.len() {
                    end + 1
                } else {
                    end
                };
                kept.push(Branch {
                    nodes: branch.nodes[first..=last].to_vec(),
                    start_terminal: if first == 0 {
                        branch.start_terminal
                    } else {
                        PLANE_TERMINAL
                    },
                    end_terminal: if last + 1 == branch.nodes.len() {
                        branch.end_terminal
                    } else {
                        PLANE_TERMINAL
                    },
                });
                removed += branch.nodes.len() - (last + 1 - first);
                start = last + 1;
            }
            if drop.iter().all(|flag| *flag) {
                removed += branch.nodes.len();
            }
        }
        // Junctions that sat on a removed node now sit on the pour.
        let keep = vec![true; kept.len()];
        let hosts = junction_hosts(&kept, &keep);
        for branch in &mut kept {
            if branch.start_terminal == NO_TERMINAL && !hosts.contains_key(&branch.nodes[0]) {
                branch.start_terminal = PLANE_TERMINAL;
            }
            if branch.end_terminal == NO_TERMINAL
                && !hosts.contains_key(branch.nodes.last().unwrap())
            {
                branch.end_terminal = PLANE_TERMINAL;
            }
        }
        state.branches = kept;
        state.fixed = 0;
        self.stamp(net);
        removed
    }
}

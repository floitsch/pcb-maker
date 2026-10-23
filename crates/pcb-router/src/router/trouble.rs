// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The trouble-makers' repair (`Config::trouble_repair`, on since
//! 2026-10-23; `PCB_ROUTER_TROUBLE` overrides it): after the hard repair, each net still open is routed through whatever
//! is in its way at a price, the nets it then overlaps lose the branches it
//! crosses and are routed again around it, hard; the exchange is kept when
//! the open terminals of all of them together drop
//! (docs/experiments/human-level-routing.md).

use super::*;

/// The repair's settings, from the environment (an experiment).
#[derive(Clone, Copy, Debug)]
pub(super) struct TroubleSettings {
    /// The price of sharing a node for the trouble-maker's search.
    pub present: f32,
    /// The repair's work, as a share of the negotiation's.
    pub share: f64,
    /// Rounds over the open nets.
    pub rounds: usize,
    /// The most nets one exchange may displace (at least; see
    /// `per_open`).
    pub victims: usize,
    /// The victims a trouble-maker may displace per open terminal of its
    /// own (`PCB_ROUTER_TROUBLE_PER_OPEN`): its limit is the larger of
    /// `victims` and this times its open terminals; 0 the flat limit.
    pub per_open: f64,
    /// A trade that closes as many terminals as it opens is kept when it
    /// leaves fewer of the nets involved open (`PCB_ROUTER_TROUBLE_TIES`).
    pub ties: bool,
    /// The order of the trouble-makers in a round
    /// (`PCB_ROUTER_TROUBLE_ORDER`).
    pub order: TroubleOrder,
    /// 2: a displaced net that comes out worse may displace others in
    /// turn; 1: not.
    pub depth: usize,
    /// Pour nets take part (`PCB_ROUTER_TROUBLE_POURS`, an experiment,
    /// off): as trouble-makers and as victims, their open terminals
    /// counted as the pads off their pour's main piece (the pour
    /// analysed again after the exchange).
    pub pours: bool,
}

/// The order of the trouble-makers in a round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TroubleOrder {
    /// Most open terminals first, then the routing order (`open`).
    MostOpen,
    /// Surveyed first (each net's priced search alone, undone): the
    /// fewest victims per terminal its search closes first (`few`); a
    /// net the survey refuses is not tried in that round.
    FewestVictims,
}

/// The repair's default (`Config::trouble_repair`).
pub const TROUBLE_REPAIR_DEFAULT: bool = true;

/// `PCB_ROUTER_TROUBLE`: unset, the configuration decides; `0` or empty
/// off; anything else on. It overrides the configuration either way.
pub fn trouble_override() -> Option<bool> {
    let value = std::env::var("PCB_ROUTER_TROUBLE").ok()?;
    Some(!(value.is_empty() || value == "0"))
}

impl TroubleSettings {
    /// The knobs: `PCB_ROUTER_TROUBLE_PRESENT` (20), `_SHARE` (0.5),
    /// `_ROUNDS` (3), `_VICTIMS` (12), `_PER_OPEN` (0), `_TIES` (0),
    /// `_ORDER` (`open`) and `_DEPTH` (1).
    pub(super) fn from_env() -> Self {
        fn read<T: std::str::FromStr>(name: &str, default: T) -> T {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(default)
        }
        Self {
            present: read("PCB_ROUTER_TROUBLE_PRESENT", 20.0),
            share: read("PCB_ROUTER_TROUBLE_SHARE", 0.5),
            rounds: read("PCB_ROUTER_TROUBLE_ROUNDS", 3),
            victims: read("PCB_ROUTER_TROUBLE_VICTIMS", 12),
            per_open: read("PCB_ROUTER_TROUBLE_PER_OPEN", 0.0),
            ties: read("PCB_ROUTER_TROUBLE_TIES", 0) != 0,
            order: match std::env::var("PCB_ROUTER_TROUBLE_ORDER").as_deref() {
                Ok("few") => TroubleOrder::FewestVictims,
                _ => TroubleOrder::MostOpen,
            },
            depth: read("PCB_ROUTER_TROUBLE_DEPTH", 1),
            pours: read("PCB_ROUTER_TROUBLE_POURS", 0) != 0,
        }
    }

    /// The most nets an exchange for a trouble-maker with `open` open
    /// terminals may displace.
    fn victim_limit(&self, open: usize) -> usize {
        self.victims
            .max((self.per_open * open as f64).ceil() as usize)
    }
}

/// What an exchange did.
#[derive(Clone, Copy, Debug, Default)]
struct Exchange {
    /// Kept (else restored).
    kept: bool,
    /// The open terminals it closed (0 for a tie kept).
    closed: usize,
    /// The nets it displaced.
    displaced: usize,
}

/// A net's state as an exchange may have to restore it, and its open
/// terminals then (`open_measure`).
type Saved = (NetId, Vec<Branch>, Vec<bool>, bool, usize);

impl Router {
    fn open_terminals(&self, net: NetId) -> usize {
        self.nets[net as usize]
            .connected
            .iter()
            .filter(|connected| !**connected)
            .count()
    }

    /// The open terminals the exchanges weigh: a signal net's
    /// unconnected terminals; a pour net's pads off its pour's main piece
    /// (as `unfinished` counts them).
    fn open_measure(&self, net: NetId) -> usize {
        let state = &self.nets[net as usize];
        if state.plane.is_empty() {
            return self.open_terminals(net);
        }
        let (pours, mut parent, main) = self.analyze_pours(net);
        let terminal_base = pours.pieces + 1;
        (0..state.terminal_nodes.len())
            .filter(|terminal| find(&mut parent, terminal_base + terminal) != main)
            .count()
    }

    fn save_net(&self, net: NetId) -> Saved {
        let state = &self.nets[net as usize];
        (
            net,
            state.branches.clone(),
            state.connected.clone(),
            state.complete,
            self.open_measure(net),
        )
    }

    fn restore_nets(&mut self, saved: Vec<Saved>) {
        for (net, ..) in &saved {
            self.unstamp(*net);
        }
        for (net, branches, connected, complete, _) in saved {
            let state = &mut self.nets[net as usize];
            state.branches = branches;
            state.connected = connected;
            state.complete = complete;
            self.stamp(net);
        }
    }

    /// The repair (see the module). Returns the open terminals it closed.
    pub(super) fn repair_trouble(&mut self, order: &[NetId], settings: TroubleSettings) -> usize {
        let budget = (self.negotiation_seconds * settings.share).max(30.0);
        let clock = self.clock();
        let debug = std::env::var_os("PCB_ROUTER_DEBUG").is_some();
        let why = debug || std::env::var_os("PCB_ROUTER_TROUBLE_WHY").is_some();
        let mut closed = 0;
        let (mut tried, mut kept, mut ties, mut displaced) = (0usize, 0usize, 0usize, 0usize);
        let mut out_of_work = false;
        // A net refused is tried again only once an exchange was kept
        // after its refusal: nothing else changed that it could see.
        let mut refused: HashMap<NetId, usize> = HashMap::new();
        for round in 0..settings.rounds {
            let mut trouble: Vec<NetId> = order
                .iter()
                .copied()
                .filter(|net| {
                    let state = &self.nets[*net as usize];
                    state.routable
                        && if state.plane.is_empty() {
                            !state.complete
                        } else {
                            settings.pours
                        }
                })
                .filter(|net| self.open_measure(*net) > 0)
                .collect();
            // Most open first (a bus net with nine of ten pads open), then
            // the routing order.
            trouble.sort_by_cached_key(|net| std::cmp::Reverse(self.open_measure(*net)));
            if settings.order == TroubleOrder::FewestVictims {
                let mut surveyed: Vec<(NetId, usize, usize)> = Vec::new();
                for net in trouble {
                    if refused.get(&net) == Some(&kept) {
                        continue;
                    }
                    if self.spent(&clock, budget) {
                        out_of_work = true;
                        break;
                    }
                    match self.trouble_survey(net, order, settings) {
                        Ok((victims, closes)) => surveyed.push((net, victims, closes)),
                        Err(reason) => {
                            refused.insert(net, kept);
                            if why {
                                eprintln!(
                                    "trouble survey {}: {reason}",
                                    self.board.nets[net as usize].name
                                );
                            }
                        }
                    }
                }
                // Fewest victims per terminal closed (a/b < c/d as a*d <
                // c*b), then most closed, then most open.
                surveyed.sort_by(|a, b| (a.1 * b.2).cmp(&(b.1 * a.2)).then(b.2.cmp(&a.2)));
                trouble = surveyed.into_iter().map(|entry| entry.0).collect();
            }
            let (mut closed_round, mut kept_round) = (0, 0);
            for net in trouble {
                if self.spent(&clock, budget) {
                    out_of_work = true;
                    break;
                }
                let before_net = self.open_measure(net);
                if before_net == 0 || refused.get(&net) == Some(&kept) {
                    continue;
                }
                tried += 1;
                let exchange = self.trouble_exchange(net, order, settings);
                if exchange.kept {
                    closed_round += exchange.closed;
                    kept_round += 1;
                    kept += 1;
                    if exchange.closed == 0 {
                        ties += 1;
                    }
                    displaced += exchange.displaced;
                    refused.remove(&net);
                } else {
                    refused.insert(net, kept);
                }
                if debug {
                    eprintln!(
                        "trouble {}: {} open before, closed {}, {} displaced",
                        self.board.nets[net as usize].name,
                        before_net,
                        exchange.closed,
                        exchange.displaced
                    );
                }
            }
            closed += closed_round;
            if self.config.verbose {
                eprintln!(
                    "trouble round {round}: {closed_round} open terminals closed ({tried} tried, {kept} kept, {ties} of them ties, {displaced} nets displaced so far)"
                );
            }
            // Another round only where this one changed the board (a tie
            // kept changes it too).
            if kept_round == 0 || out_of_work {
                break;
            }
        }
        if self.config.verbose {
            eprintln!(
                "trouble: {closed} open terminals closed in {tried} exchanges tried, {kept} kept ({ties} ties), {:.1} s of work{}",
                self.budget_work() - clock.0,
                if out_of_work { " (out of work)" } else { "" }
            );
        }
        closed
    }

    /// One exchange for `net`: it is routed at `settings.present` through
    /// other copper; the nets it overlaps lose their crossed branches and
    /// are routed again hard. Kept when the open terminals of all of them
    /// drop, or, with `settings.ties`, stay and fewer of the nets are open;
    /// else restored.
    fn trouble_exchange(
        &mut self,
        net: NetId,
        order: &[NetId],
        settings: TroubleSettings,
    ) -> Exchange {
        let (exchange, reason) = self.trouble_exchange_why(net, order, settings);
        if std::env::var_os("PCB_ROUTER_DEBUG").is_some()
            || std::env::var_os("PCB_ROUTER_TROUBLE_WHY").is_some()
        {
            eprintln!(
                "trouble exchange {}: {reason}",
                self.board.nets[net as usize].name
            );
        }
        exchange
    }

    /// The survey of `net` for `TroubleOrder::FewestVictims`: its priced
    /// search alone, undone; the victims it would displace and the open
    /// terminals of its own it closes, or why its exchange would be
    /// refused.
    fn trouble_survey(
        &mut self,
        net: NetId,
        order: &[NetId],
        settings: TroubleSettings,
    ) -> Result<(usize, usize), String> {
        let before_net = self.open_measure(net);
        let saved = vec![self.save_net(net)];
        self.nets[net as usize].hopeless_searches = 0;
        self.route_net_seq(net, settings.present, false, 4.0);
        self.stamp(net);
        let after_net = self.open_measure(net);
        let victims: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|other| *other != net && !self.conflicts(*other).is_empty())
            .collect();
        let limit = settings.victim_limit(before_net);
        let refusal = if after_net >= before_net {
            Some(format!(
                "the priced search closes nothing ({before_net} open)"
            ))
        } else if victims.len() > limit {
            Some(format!(
                "{} victims, more than allowed ({limit} for {before_net} open)",
                victims.len()
            ))
        } else {
            victims
                .iter()
                .find(|victim| !settings.pours && !self.nets[**victim as usize].plane.is_empty())
                .map(|pour| {
                    format!(
                        "a pour net among the victims ({})",
                        self.board.nets[*pour as usize].name
                    )
                })
        };
        self.restore_nets(saved);
        match refusal {
            Some(reason) => Err(reason),
            None => Ok((victims.len(), before_net - after_net)),
        }
    }

    fn trouble_exchange_why(
        &mut self,
        net: NetId,
        order: &[NetId],
        settings: TroubleSettings,
    ) -> (Exchange, String) {
        let (mut saved, victims) = match self.displace(net, &[], order, settings) {
            Ok(done) => done,
            Err(reason) => return (Exchange::default(), reason),
        };
        // A victim that came out worse may push others aside in turn (not
        // the net the exchange is for).
        let mut nested = 0;
        if settings.depth > 1 {
            for victim in &victims {
                let before = saved
                    .iter()
                    .find(|entry| entry.0 == *victim)
                    .map_or(0, |entry| entry.4);
                if self.open_measure(*victim) <= before {
                    continue;
                }
                if let Ok((inner, _)) = self.displace(*victim, &[net], order, settings) {
                    if open_now(self, &inner) < open_saved(&inner) && clean(self, &inner) {
                        nested += 1;
                        for entry in inner {
                            if !saved.iter().any(|kept| kept.0 == entry.0) {
                                saved.push(entry);
                            }
                        }
                    } else {
                        self.restore_nets(inner);
                    }
                }
            }
        }
        let before = open_saved(&saved);
        let after = open_now(self, &saved);
        let clean = clean(self, &saved);
        let involved = saved.len() - 1;
        // The nets open among those involved, before and after: a tie
        // that gathers the open terminals into fewer nets leaves the next
        // exchange fewer nets to close (open terminals, then open nets,
        // fall in every kept exchange: no cycle).
        let nets_before = saved.iter().filter(|entry| entry.4 > 0).count();
        let nets_after = saved
            .iter()
            .filter(|entry| self.open_measure(entry.0) > 0)
            .count();
        let tie = settings.ties && after == before && nets_after < nets_before;
        if (after < before || tie) && clean {
            (
                Exchange {
                    kept: true,
                    closed: before - after,
                    displaced: involved,
                },
                format!(
                    "kept: {before} open to {after}, {nets_before} nets open to {nets_after} ({involved} displaced, {nested} pushed on)"
                ),
            )
        } else {
            self.restore_nets(saved);
            (
                Exchange::default(),
                format!(
                    "refused: {before} open to {after}, {nets_before} nets open to {nets_after} ({involved} displaced, {nested} pushed on), clean {clean}"
                ),
            )
        }
    }

    /// Routes `net` at the settings' price through other copper, rips up
    /// the branches of the nets it then overlaps and routes those again
    /// hard. Returns the nets changed as they were (`net` first) and the
    /// victims; or, restored, why not.
    fn displace(
        &mut self,
        net: NetId,
        protected: &[NetId],
        order: &[NetId],
        settings: TroubleSettings,
    ) -> Result<(Vec<Saved>, Vec<NetId>), String> {
        let before_net = self.open_measure(net);
        let mut saved = vec![self.save_net(net)];
        self.nets[net as usize].hopeless_searches = 0;
        self.route_net_seq(net, settings.present, false, 4.0);
        self.stamp(net);
        if self.open_measure(net) >= before_net {
            self.restore_nets(saved);
            return Err(format!(
                "the priced search closes nothing ({before_net} open)"
            ));
        }
        let victims: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|other| *other != net && !self.conflicts(*other).is_empty())
            .collect();
        let limit = settings.victim_limit(before_net);
        let refusal = if victims.len() > limit {
            Some(format!(
                "{} victims, more than allowed ({limit} for {before_net} open)",
                victims.len()
            ))
        } else if victims.iter().any(|victim| protected.contains(victim)) {
            Some("it would displace the net the exchange is for".to_string())
        } else {
            victims
                .iter()
                .find(|victim| !settings.pours && !self.nets[**victim as usize].plane.is_empty())
                .map(|pour| {
                    format!(
                        "a pour net among the victims ({})",
                        self.board.nets[*pour as usize].name
                    )
                })
        };
        if let Some(reason) = refusal {
            self.restore_nets(saved);
            return Err(reason);
        }
        for victim in &victims {
            saved.push(self.save_net(*victim));
            self.rip_up_conflicted(*victim);
        }
        // What is still in its way is copper its own conflicts do not
        // show (a net of another class, whose clearance zone reaches
        // further than the net's own: the conflict is on one side only)
        // or fixed copper (a skeleton, a stub). The first goes whole.
        let mut victims = victims;
        if !self.conflicts(net).is_empty() {
            let others = self.blockers(net);
            let fresh: Vec<NetId> = others
                .iter()
                .copied()
                .filter(|other| !victims.contains(other))
                .collect();
            let refusal = if others.iter().any(|other| protected.contains(other)) {
                Some("it would displace the net the exchange is for".to_string())
            } else if victims.len() + fresh.len() > limit {
                Some(format!(
                    "{} victims with the one-sided conflicts, more than allowed ({limit} for {before_net} open)",
                    victims.len() + fresh.len()
                ))
            } else {
                others
                    .iter()
                    .find(|other| !settings.pours && !self.nets[**other as usize].plane.is_empty())
                    .map(|pour| {
                        format!(
                            "a pour net in its way ({})",
                            self.board.nets[*pour as usize].name
                        )
                    })
            };
            if let Some(reason) = refusal {
                self.restore_nets(saved);
                return Err(reason);
            }
            for other in &others {
                if !saved.iter().any(|entry| entry.0 == *other) {
                    saved.push(self.save_net(*other));
                }
                self.rip_up(*other);
            }
            victims.extend(fresh);
            if !self.conflicts(net).is_empty() {
                let names: Vec<String> = self
                    .blockers(net)
                    .iter()
                    .map(|other| self.board.nets[*other as usize].name.clone())
                    .collect();
                self.restore_nets(saved);
                return Err(format!("fixed copper in its way ({})", names.join(", ")));
            }
        }
        for victim in &victims {
            self.nets[*victim as usize].hopeless_searches = 0;
            self.route_net_seq(*victim, 0.0, true, 4.0);
            self.stamp(*victim);
        }
        Ok((saved, victims))
    }
}

impl Router {
    /// The nets whose stamps cover a node of `net` in conflict (`net`'s
    /// maps: its class, or its neck class there, and its via map).
    fn blockers(&self, net: NetId) -> Vec<NetId> {
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let cells: HashSet<(u32, u32)> = self
            .conflicts(net)
            .into_iter()
            .map(|(layer, cell)| {
                let map = if layer < layers {
                    self.map_index(self.node_class(net, cell), layer)
                } else {
                    self.map_index(class, layers)
                };
                (map as u32, cell)
            })
            .collect();
        (0..self.nets.len() as NetId)
            .filter(|other| *other != net)
            .filter(|other| {
                self.nets[*other as usize]
                    .stamped
                    .iter()
                    .any(|stamp| cells.contains(&stamp))
            })
            .collect()
    }
}

/// The open terminals of the saved nets as they were.
fn open_saved(saved: &[Saved]) -> usize {
    saved.iter().map(|entry| entry.4).sum()
}

/// The open terminals of the saved nets as they are now.
fn open_now(router: &Router, saved: &[Saved]) -> usize {
    saved.iter().map(|entry| router.open_measure(entry.0)).sum()
}

/// Whether none of the saved nets is in conflict now.
fn clean(router: &Router, saved: &[Saved]) -> bool {
    saved
        .iter()
        .all(|entry| router.conflicts(entry.0).is_empty())
}

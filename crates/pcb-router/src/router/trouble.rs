// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The trouble-makers' repair (an experiment, `PCB_ROUTER_TROUBLE`, off):
//! after the hard repair, each net still open is routed through whatever
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
    /// The most nets one exchange may displace.
    pub victims: usize,
    /// 2: a displaced net that comes out worse may displace others in
    /// turn; 1: not.
    pub depth: usize,
}

impl TroubleSettings {
    /// `PCB_ROUTER_TROUBLE` set (and not `0`): on, with
    /// `PCB_ROUTER_TROUBLE_PRESENT` (20), `_SHARE` (0.5), `_ROUNDS` (3)
    /// `_VICTIMS` (12) and `_DEPTH` (1).
    pub(super) fn from_env() -> Option<Self> {
        let on = std::env::var("PCB_ROUTER_TROUBLE").ok()?;
        if on == "0" || on.is_empty() {
            return None;
        }
        fn read<T: std::str::FromStr>(name: &str, default: T) -> T {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(default)
        }
        Some(Self {
            present: read("PCB_ROUTER_TROUBLE_PRESENT", 20.0),
            share: read("PCB_ROUTER_TROUBLE_SHARE", 0.5),
            rounds: read("PCB_ROUTER_TROUBLE_ROUNDS", 3),
            victims: read("PCB_ROUTER_TROUBLE_VICTIMS", 12),
            depth: read("PCB_ROUTER_TROUBLE_DEPTH", 1),
        })
    }
}

/// A net's state as an exchange may have to restore it.
type Saved = (NetId, Vec<Branch>, Vec<bool>, bool);

impl Router {
    fn open_terminals(&self, net: NetId) -> usize {
        self.nets[net as usize]
            .connected
            .iter()
            .filter(|connected| !**connected)
            .count()
    }

    fn save_net(&self, net: NetId) -> Saved {
        let state = &self.nets[net as usize];
        (
            net,
            state.branches.clone(),
            state.connected.clone(),
            state.complete,
        )
    }

    fn restore_nets(&mut self, saved: Vec<Saved>) {
        for (net, ..) in &saved {
            self.unstamp(*net);
        }
        for (net, branches, connected, complete) in saved {
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
        let mut closed = 0;
        let (mut tried, mut kept, mut displaced) = (0usize, 0usize, 0usize);
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
                    state.routable && !state.complete && state.plane.is_empty()
                })
                .collect();
            // Most open first (a bus net with nine of ten pads open), then
            // the routing order.
            trouble.sort_by_key(|net| std::cmp::Reverse(self.open_terminals(*net)));
            let mut closed_round = 0;
            for net in trouble {
                if self.spent(&clock, budget) {
                    out_of_work = true;
                    break;
                }
                let before_net = self.open_terminals(net);
                if before_net == 0 || refused.get(&net) == Some(&kept) {
                    continue;
                }
                tried += 1;
                let gained = self.trouble_exchange(net, order, settings);
                if gained.0 > 0 {
                    closed_round += gained.0;
                    kept += 1;
                    displaced += gained.1;
                    refused.remove(&net);
                } else {
                    refused.insert(net, kept);
                }
                if debug {
                    eprintln!(
                        "trouble {}: {} open before, closed {}, {} displaced",
                        self.board.nets[net as usize].name, before_net, gained.0, gained.1
                    );
                }
            }
            closed += closed_round;
            if self.config.verbose {
                eprintln!(
                    "trouble round {round}: {closed_round} open terminals closed ({tried} tried, {kept} kept, {displaced} nets displaced so far)"
                );
            }
            if closed_round == 0 || out_of_work {
                break;
            }
        }
        if self.config.verbose {
            eprintln!(
                "trouble: {closed} open terminals closed in {tried} exchanges tried, {kept} kept, {:.1} s of work{}",
                self.budget_work() - clock.0,
                if out_of_work { " (out of work)" } else { "" }
            );
        }
        closed
    }

    /// One exchange for `net`: it is routed at `settings.present` through
    /// other copper; the nets it overlaps lose their crossed branches and
    /// are routed again hard. Kept when the open terminals of all of them
    /// drop; returns (terminals closed, nets displaced), or (0, 0) restored.
    fn trouble_exchange(
        &mut self,
        net: NetId,
        order: &[NetId],
        settings: TroubleSettings,
    ) -> (usize, usize) {
        let (gained, displaced, reason) = self.trouble_exchange_why(net, order, settings);
        if std::env::var_os("PCB_ROUTER_DEBUG").is_some() || std::env::var_os("PCB_ROUTER_TROUBLE_WHY").is_some() {
            eprintln!(
                "trouble exchange {}: {reason}",
                self.board.nets[net as usize].name
            );
        }
        (gained, displaced)
    }

    fn trouble_exchange_why(
        &mut self,
        net: NetId,
        order: &[NetId],
        settings: TroubleSettings,
    ) -> (usize, usize, String) {
        let (mut saved, victims) = match self.displace(net, &[], order, settings) {
            Ok(done) => done,
            Err(reason) => return (0, 0, reason),
        };
        // A victim that came out worse may push others aside in turn (not
        // the net the exchange is for).
        let mut nested = 0;
        if settings.depth > 1 {
            for victim in &victims {
                let before = saved
                    .iter()
                    .find(|entry| entry.0 == *victim)
                    .map_or(0, |entry| entry.2.iter().filter(|c| !**c).count());
                if self.open_terminals(*victim) <= before {
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
        if after < before && clean {
            (
                before - after,
                involved,
                format!("kept: {before} open to {after} ({involved} displaced, {nested} pushed on)"),
            )
        } else {
            self.restore_nets(saved);
            (
                0,
                0,
                format!(
                    "refused: {before} open to {after} ({involved} displaced, {nested} pushed on), clean {clean}"
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
        let before_net = self.open_terminals(net);
        let mut saved = vec![self.save_net(net)];
        self.nets[net as usize].hopeless_searches = 0;
        self.route_net_seq(net, settings.present, false, 4.0);
        self.stamp(net);
        if self.open_terminals(net) >= before_net {
            self.restore_nets(saved);
            return Err(format!("the priced search closes nothing ({before_net} open)"));
        }
        let victims: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|other| *other != net && !self.conflicts(*other).is_empty())
            .collect();
        let refusal = if victims.len() > settings.victims {
            Some(format!("{} victims, more than allowed", victims.len()))
        } else if victims.iter().any(|victim| protected.contains(victim)) {
            Some("it would displace the net the exchange is for".to_string())
        } else {
            victims
                .iter()
                .find(|victim| !self.nets[**victim as usize].plane.is_empty())
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
            } else if victims.len() + fresh.len() > settings.victims {
                Some(format!(
                    "{} victims with the one-sided conflicts, more than allowed",
                    victims.len() + fresh.len()
                ))
            } else {
                others
                    .iter()
                    .find(|other| !self.nets[**other as usize].plane.is_empty())
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
                    .any(|stamp| cells.contains(stamp))
            })
            .collect()
    }
}

/// The open terminals of the saved nets as they were.
fn open_saved(saved: &[Saved]) -> usize {
    saved
        .iter()
        .map(|entry| entry.2.iter().filter(|connected| !**connected).count())
        .sum()
}

/// The open terminals of the saved nets as they are now.
fn open_now(router: &Router, saved: &[Saved]) -> usize {
    saved
        .iter()
        .map(|entry| router.open_terminals(entry.0))
        .sum()
}

/// Whether none of the saved nets is in conflict now.
fn clean(router: &Router, saved: &[Saved]) -> bool {
    saved
        .iter()
        .all(|entry| router.conflicts(entry.0).is_empty())
}

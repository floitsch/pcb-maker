// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Giving up on a net that stands in for others (`GiveUp`: a differential
//! pair's link) when the negotiation finds it no room: docs/router.md,
//! "Giving up on a link".

use super::*;
use crate::board::{GiveUp, NetRewrite};

/// When the negotiation gives up an optional net (`GiveUp::optional`):
/// counted in its rounds unsettled (incomplete, or complete through
/// other nets' copper), the iterations of the negotiation that ended with
/// it so.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GiveUpRule {
    /// No net is given up before it has been unsettled this many rounds.
    pub min_rounds: usize,
    /// Given up ("no room") once its conflicts have not fallen below their
    /// lowest for this many rounds.
    pub stall_rounds: usize,
    /// Given up ("no path") after this many rounds in a row with no path
    /// even through other nets' copper.
    pub pathless_rounds: usize,
    /// Given up ("no room") after this many rounds unsettled, whatever its
    /// conflicts do.
    pub max_rounds: usize,
    /// A round in conflict counts only once the price of sharing has
    /// reached this (`Config::present_factor` grown by `present_growth`
    /// each iteration): below it the negotiation is still sorting the nets
    /// out, and links that settle later are in conflict too.
    pub min_present: f32,
}

impl Default for GiveUpRule {
    fn default() -> Self {
        Self {
            // Measured on jetson-nano-baseboard and ducktop2's io boards
            // (docs/router.md, "Giving up on a link"): counted from the
            // first iteration, 4, 3 and 10 rounds gave every link but five
            // up in jetson's race probes (0 unconnected, 2 pairs coupled
            // where x1907 had 23 and 8), 8, 4 and 16 left jetson at 4 and
            // 9 but cost left_io 4 of its 6 coupled pairs; counted from
            // the price of 100, see the docs' table.
            min_rounds: 8,
            stall_rounds: 4,
            // A negotiation's search that finds no path even through other
            // nets' copper leaves its net out of the negotiation for good
            // (`NetState::blocked`): nothing is gained by waiting.
            pathless_rounds: 1,
            max_rounds: 16,
            min_present: 100.0,
        }
    }
}

/// `PCB_ROUTER_GIVE_UP` (`min,stall,pathless,max[,present]`): the rule's
/// rounds and the price they count from, an experiment's override.
pub(super) fn rule_override() -> Option<GiveUpRule> {
    let value = std::env::var("PCB_ROUTER_GIVE_UP").ok()?;
    let items: Vec<f64> = value
        .split(',')
        .map(|item| item.trim().parse().ok())
        .collect::<Option<Vec<f64>>>()?;
    let rounds = |index: usize| items[index].max(0.0) as usize;
    match items.len() {
        4 | 5 => Some(GiveUpRule {
            min_rounds: rounds(0),
            stall_rounds: rounds(1),
            pathless_rounds: rounds(2),
            max_rounds: rounds(3),
            min_present: items
                .get(4)
                .map_or(GiveUpRule::default().min_present, |value| *value as f32),
        }),
        _ => None,
    }
}

/// How a round ended for an optional net (`GiveUpRecord::round`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Standing {
    /// Complete without a conflict.
    Settled,
    /// Incomplete without a conflict: no path even through other nets'
    /// copper (or its searches gave up).
    Pathless,
    /// In conflict over this many lattice cells (complete through other
    /// nets' copper, or incomplete beside it).
    Conflicted(usize),
}

/// An optional net's record over the negotiation's rounds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct GiveUpRecord {
    unsettled: usize,
    pathless: usize,
    best: Option<usize>,
    since_best: usize,
}

/// Why a net was given up (`ReleasedNet::reason`).
/// Its conflicts stopped falling, or it stayed unsettled to the cap.
pub const NO_ROOM: &str = "no room";
/// No path even through other nets' copper, rounds in a row.
pub const NO_PATH: &str = "no path";

impl GiveUpRecord {
    /// Counts one round's `standing`; returns why the net is to be given
    /// up, if it is. A settled round starts the record afresh.
    pub(super) fn round(
        &mut self,
        standing: Standing,
        rule: &GiveUpRule,
        present: f32,
    ) -> Option<&'static str> {
        if matches!(standing, Standing::Conflicted(_)) && present < rule.min_present {
            return None;
        }
        match standing {
            Standing::Settled => {
                *self = Self::default();
                return None;
            }
            Standing::Pathless => {
                self.unsettled += 1;
                self.pathless += 1;
            }
            Standing::Conflicted(cells) => {
                self.unsettled += 1;
                self.pathless = 0;
                if self.best.is_none_or(|best| cells < best) {
                    self.best = Some(cells);
                    self.since_best = 0;
                } else {
                    self.since_best += 1;
                }
            }
        }
        if self.pathless >= rule.pathless_rounds.max(1) {
            return Some(NO_PATH);
        }
        if self.unsettled >= rule.max_rounds.max(1)
            || (self.unsettled >= rule.min_rounds && self.since_best >= rule.stall_rounds)
        {
            return Some(NO_ROOM);
        }
        None
    }
}

/// A net the negotiation gave up (`Diagnostics::released`).
#[derive(Clone, Debug, PartialEq)]
pub struct ReleasedNet {
    pub net: NetId,
    /// `NO_ROOM` or `NO_PATH`.
    pub reason: &'static str,
    /// The router's negotiation iteration it went after (counted over all
    /// its negotiations, `Router::iterations`).
    pub iteration: usize,
    /// The search work it had taken until then, in seconds.
    pub work_seconds: f64,
}

impl Router {
    /// After a negotiation iteration: each optional net (`GiveUp`) counts
    /// the round (`GiveUpRecord`); those the rule gives up lose their
    /// copper and are not routed again, and the nets they stood in for are
    /// rewritten (`GiveUp::released`) and routed from the next iteration
    /// on in `conflicted` (the nets the next iteration reroutes).
    pub(super) fn give_up_round(
        &mut self,
        order: &[NetId],
        conflicted: &mut Vec<NetId>,
        present: f32,
    ) {
        let Some(give_up) = self.board.attachment.give_up.clone() else {
            return;
        };
        let rule = self.config.give_up;
        let mut going = Vec::new();
        for &net in order {
            let state = &self.nets[net as usize];
            if state.released || !state.routable || !give_up.optional(net) {
                continue;
            }
            let cells = self.conflicts(net).len();
            let standing = if cells > 0 {
                Standing::Conflicted(cells)
            } else if state.complete {
                Standing::Settled
            } else {
                Standing::Pathless
            };
            let mut record = state.give_up;
            let verdict = record.round(standing, &rule, present);
            self.nets[net as usize].give_up = record;
            if let Some(reason) = verdict {
                going.push((net, reason));
            }
        }
        if going.is_empty() {
            return;
        }
        let rewritten = self.release(give_up.as_ref(), &going);
        conflicted.retain(|net| !self.nets[*net as usize].released);
        for net in rewritten {
            if self.nets[net as usize].routable && !conflicted.contains(&net) {
                conflicted.push(net);
            }
        }
        self.last_conflicted = conflicted.clone();
    }

    /// Gives up `nets` (with their reasons): their copper goes, they are not
    /// routed again; the nets they stood in for take their terminals as
    /// `give_up` says, lose their copper and are prepared again. Returns
    /// those nets.
    pub(super) fn release(
        &mut self,
        give_up: &dyn GiveUp,
        nets: &[(NetId, &'static str)],
    ) -> Vec<NetId> {
        for &(net, reason) in nets {
            self.strip(net);
            let state = &mut self.nets[net as usize];
            state.released = true;
            state.routable = false;
            self.released.push(ReleasedNet {
                net,
                reason,
                iteration: self.iterations,
                work_seconds: state.expansions as f64 / expansions_per_second(),
            });
            if self.config.verbose || negotiate::link_trace() {
                eprintln!(
                    "give-up: {} after iteration {} ({reason}; {:.1} s of search work)",
                    self.board.nets[net as usize].name,
                    self.iterations,
                    state.expansions as f64 / expansions_per_second()
                );
            }
        }
        let released: Vec<NetId> = self.released.iter().map(|entry| entry.net).collect();
        let rewrites = give_up.released(&self.board, &released);
        let mut changed = Vec::new();
        for rewrite in &rewrites {
            let net = rewrite.net;
            self.strip(net);
            self.board.rewrite_nets(std::slice::from_ref(rewrite));
            let old = std::mem::take(&mut self.nets[net as usize]);
            let mut fresh = self.prepare_net(net);
            fresh.reroutes = old.reroutes;
            fresh.expansions = old.expansions;
            self.nets[net as usize] = fresh;
            changed.push(net);
        }
        changed
    }

    /// In the hard repair, once the nets in conflict are ripped up: each
    /// net given up (the fewest conflicts it ever had first) is tried once
    /// more as the repair tries a link, every other net's copper fixed: the nets it stood in for lose
    /// their copper and take their terminals with it again, it is routed
    /// hard, then they are; where all of them complete (and the nets it
    /// stood in for leave no more terminals open than they did), it stands
    /// again, else everything is put back as it was. Within `budget` work
    /// seconds. Returns how many stand again.
    pub(super) fn retry_released(&mut self, budget: f64) -> usize {
        let Some(give_up) = self.board.attachment.give_up.clone() else {
            return 0;
        };
        if self.released.is_empty() || self.hopeless {
            return 0;
        }
        let clock = self.clock();
        let mut order: Vec<ReleasedNet> = self.released.clone();
        order.sort_by_key(|entry| {
            (
                entry.reason == NO_PATH,
                self.nets[entry.net as usize]
                    .give_up
                    .best
                    .unwrap_or(usize::MAX),
                entry.net,
            )
        });
        let mut restored = 0;
        for entry in order {
            if entry.reason == NO_PATH || self.spent(&clock, budget) {
                if entry.reason != NO_PATH && negotiate::link_trace() {
                    eprintln!(
                        "give-up: {} not tried again: the try's budget ({budget:.0} s) is spent",
                        self.board.nets[entry.net as usize].name
                    );
                }
                continue;
            }
            let link = entry.net;
            let others: Vec<NetId> = self
                .released
                .iter()
                .map(|released| released.net)
                .filter(|net| *net != link)
                .collect();
            // The nets the link stands in for, as they are with it.
            let standing = give_up.standing(&self.board, link, &others);
            let changed: Vec<NetId> = standing.iter().map(|rewrite| rewrite.net).collect();
            if changed.is_empty() {
                continue;
            }
            let open_before: usize = changed
                .iter()
                .map(|net| {
                    let state = &self.nets[*net as usize];
                    state.connected.iter().filter(|joined| !**joined).count()
                })
                .sum();
            // Saved to go back to: the nets' states and their terminals.
            let saved: Vec<(NetId, NetState, NetRewrite)> = changed
                .iter()
                .map(|&net| (net, self.nets[net as usize].clone(), self.net_as_is(net)))
                .collect();
            let saved_link = self.nets[link as usize].clone();
            for &net in &changed {
                self.strip(net);
            }
            self.board.rewrite_nets(&standing);
            for &net in &changed {
                let mut fresh = self.prepare_net(net);
                fresh.reroutes = self.nets[net as usize].reroutes;
                fresh.expansions = self.nets[net as usize].expansions;
                self.nets[net as usize] = fresh;
            }
            let mut fresh = self.prepare_net(link);
            fresh.expansions = saved_link.expansions;
            fresh.give_up = saved_link.give_up;
            self.nets[link as usize] = fresh;
            self.route_net_seq(link, 0.0, true, 4.0);
            self.stamp(link);
            let mut complete = self.nets[link as usize].complete;
            if complete {
                for &net in &changed {
                    if self.nets[net as usize].routable {
                        self.route_net_seq(net, 0.0, true, 4.0);
                        self.stamp(net);
                    }
                    complete &= self.nets[net as usize].complete;
                }
            }
            let open_after: usize = changed
                .iter()
                .map(|net| {
                    let state = &self.nets[*net as usize];
                    state.connected.iter().filter(|joined| !**joined).count()
                })
                .sum();
            if complete && open_after <= open_before {
                self.released.retain(|released| released.net != link);
                restored += 1;
                if self.config.verbose || negotiate::link_trace() {
                    eprintln!(
                        "give-up: {} stands again after the hard repair",
                        self.board.nets[link as usize].name
                    );
                }
                continue;
            }
            if self.config.verbose || negotiate::link_trace() {
                eprintln!(
                    "give-up: {} stays given up after the hard repair's try ({})",
                    self.board.nets[link as usize].name,
                    if !self.nets[link as usize].complete {
                        "no room for the link"
                    } else if !complete {
                        "its nets do not reach it"
                    } else {
                        "its nets would leave more open"
                    }
                );
            }
            // Back as it was.
            self.strip(link);
            for &net in &changed {
                self.strip(net);
            }
            let mut link_state = saved_link;
            link_state.stamped = Default::default();
            self.nets[link as usize] = link_state;
            for (net, mut state, rewrite) in saved {
                self.board.rewrite_nets(std::slice::from_ref(&rewrite));
                state.stamped = Default::default();
                self.nets[net as usize] = state;
                self.stamp(net);
            }
        }
        restored
    }

    /// `net` as the board has it now, as a rewrite (`NetRewrite`).
    fn net_as_is(&self, net: NetId) -> NetRewrite {
        NetRewrite {
            net,
            terminals: self.board.nets[net as usize].terminals.clone(),
            merged_pads: self
                .board
                .merged_pads
                .iter()
                .filter(|merged| merged.net == net)
                .cloned()
                .collect(),
            terminal_copper: self
                .board
                .terminal_copper
                .iter()
                .filter(|extra| extra.net == net)
                .copied()
                .collect(),
        }
    }

    /// Removes every branch of `net`, its fixed ones too.
    fn strip(&mut self, net: NetId) {
        let state = &mut self.nets[net as usize];
        state.fixed = 0;
        self.rip_up(net);
    }

    /// The board as this router has it, given `board` of the same nets:
    /// with the nets given up so far (`ReleasedNet`) given up on it too
    /// (`GiveUp::released`), so that `update` finds the same nets.
    pub(super) fn with_releases<'a>(&self, board: &'a Board) -> std::borrow::Cow<'a, Board> {
        match (&board.attachment.give_up, self.released.is_empty()) {
            (Some(give_up), false) => {
                let released: Vec<NetId> = self.released.iter().map(|entry| entry.net).collect();
                let rewrites = give_up.released(board, &released);
                let mut board = board.clone();
                board.rewrite_nets(&rewrites);
                std::borrow::Cow::Owned(board)
            }
            _ => std::borrow::Cow::Borrowed(board),
        }
    }

    /// After `update` prepared every net again: the nets given up stay so.
    pub(super) fn keep_released(&mut self) {
        for entry in &self.released {
            let state = &mut self.nets[entry.net as usize];
            state.released = true;
            state.routable = false;
        }
    }

    /// The nets given up so far, in the order they went.
    pub fn released(&self) -> &[ReleasedNet] {
        &self.released
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{MergedPad, Net, Obstacle, ObstacleKind, Terminal, TerminalCopper};
    use crate::geometry::Shape;

    /// The nets of `wall_board` as a link given up leaves them: P from pad
    /// to pad, without the run's ends.
    struct Link;

    impl GiveUp for Link {
        fn optional(&self, net: NetId) -> bool {
            net == 1
        }

        fn released(&self, board: &Board, released: &[NetId]) -> Vec<NetRewrite> {
            if released.contains(&1) {
                return vec![NetRewrite {
                    net: 0,
                    terminals: vec![pad_terminal(board, 0, "P.0"), pad_terminal(board, 1, "P.1")],
                    merged_pads: Vec::new(),
                    terminal_copper: Vec::new(),
                }];
            }
            self.standing(board, 1, released)
        }

        fn standing(&self, board: &Board, net: NetId, _released: &[NetId]) -> Vec<NetRewrite> {
            assert_eq!(net, 1);
            // As planned (`wall_board`).
            vec![NetRewrite {
                net: 0,
                terminals: vec![
                    pad_terminal(board, 0, "P.0"),
                    pad_terminal(board, 2, "pair run 1a"),
                    pad_terminal(board, 1, "P.1"),
                ],
                merged_pads: vec![MergedPad {
                    net: 0,
                    host: 1,
                    pad: 3,
                    anchor: [25.0, 10.0],
                    layers: 1,
                    padstack: false,
                }],
                terminal_copper: vec![TerminalCopper {
                    net: 0,
                    terminal: 1,
                    obstacle: 3,
                }],
            }]
        }
    }

    fn pad_terminal(board: &Board, pad: usize, label: &str) -> Terminal {
        let Shape::Circle { center, .. } = board.obstacles[pad].shape else {
            panic!("a round pad");
        };
        Terminal {
            anchor: center,
            layers: 1,
            pad,
            contact: None,
            label: label.into(),
        }
    }

    /// One layer, a wall across the board at x 15 with a gap of 0.9 mm: a
    /// track of class 0 (0.25 mm, clearance 0.2) passes, one of class 1
    /// (1.2 mm, a pair's envelope) does not. Net P has pads at x 3 and 27;
    /// net L (class 1) stands in for P between the run's ends at x 5 and
    /// 25 (P's terminal there is the two ends, merged), as a differential
    /// pair's link does; `give_up` says what P becomes without L.
    fn wall_board(give_up: bool) -> Board {
        wall_board_with_gap(give_up, 0.9)
    }

    fn wall_board_with_gap(give_up: bool, gap: f64) -> Board {
        let narrow = RuleClass {
            trace_width: 0.25,
            clearance: 0.2,
            via_diameter: 0.6,
            via_drill: 0.3,
            layers: 0,
        };
        let wide = RuleClass {
            trace_width: 1.2,
            ..narrow
        };
        let disc = |center: [f64; 2], radius: f64, net: NetId, label: &str| Obstacle {
            shape: Shape::Circle { center, radius },
            layers: 1,
            kind: ObstacleKind::Copper,
            net: Some(net),
            clearance: 0.0,
            clearance_override: None,
            plated: false,
            blocks_tracks: true,
            blocks_vias: true,
            label: label.into(),
        };
        let mut obstacles = vec![
            disc([3.0, 10.0], 0.3, 0, "P.0"),
            disc([27.0, 10.0], 0.3, 0, "P.1"),
            disc([5.0, 10.0], 0.15, 0, "pair run 1a"),
            disc([25.0, 10.0], 0.15, 0, "pair run 1b"),
            disc([6.0, 12.0], 0.6, 1, "pair launch 1a"),
            disc([24.0, 12.0], 0.6, 1, "pair launch 1b"),
        ];
        let half = (20.0 - gap) / 4.0;
        for (center, half) in [
            ([15.0, half], [0.2, half]),
            ([15.0, 20.0 - half], [0.2, half]),
        ] {
            obstacles.push(Obstacle {
                shape: Shape::rectangle(center, half, 0.0),
                layers: 1,
                kind: ObstacleKind::Keepout,
                net: None,
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: "wall".into(),
            });
        }
        let mut board = Board {
            layer_count: 1,
            outline: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
            edge_clearance: 0.1,
            hole_clearance: 0.25,
            hole_to_hole: 0.25,
            classes: vec![narrow, wide],
            neck_width: 0.25,
            obstacles,
            nets: Vec::new(),
            planes: Vec::new(),
            solid_pads: Vec::new(),
            isolated_pads: Vec::new(),
            solid_pad_obstacles: Vec::new(),
            isolated_pad_obstacles: Vec::new(),
            pad_spokes: Default::default(),
            merged_pads: vec![MergedPad {
                net: 0,
                host: 1,
                pad: 3,
                anchor: [25.0, 10.0],
                layers: 1,
                padstack: false,
            }],
            no_via_pads: Vec::new(),
            fill_passes: Vec::new(),
            pin_row_gaps: Vec::new(),
            terminal_copper: vec![TerminalCopper {
                net: 0,
                terminal: 1,
                obstacle: 3,
            }],
            attachment: Default::default(),
            via_spans: Vec::new(),
        };
        let run = pad_terminal(&board, 2, "pair run 1a");
        board.nets = vec![
            Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    pad_terminal(&board, 0, "P.0"),
                    run,
                    pad_terminal(&board, 1, "P.1"),
                ],
            },
            Net {
                name: "P / N link 1".into(),
                class: 1,
                terminals: vec![
                    pad_terminal(&board, 4, "pair launch 1a"),
                    pad_terminal(&board, 5, "pair launch 1b"),
                ],
            },
        ];
        if give_up {
            board.attachment.give_up = Some(std::sync::Arc::new(Link));
        }
        board
    }

    /// A link given up whose way opens once the others' copper is fixed
    /// stands again after the hard repair (`retry_released`): its nets
    /// take their terminals with it again and route to the run's ends.
    /// Where it has no room even then, all goes back as it was.
    #[test]
    fn a_link_given_up_stands_again_where_the_repair_finds_it_room() {
        // A gap the link's 1.2 mm track passes (with its clearance).
        let board = wall_board_with_gap(true, 2.4);
        let mut router = Router::new(&board, &config());
        let result = router.run_in_place();
        assert!(result.diagnostics.released.is_empty());
        assert_eq!(result.status[1], NetStatus::Routed);
        // Given up by hand, its net routed without it, then tried again.
        router.release(&Link, &[(1, NO_ROOM)]);
        assert_eq!(router.released().len(), 1);
        assert_eq!(router.board.nets[0].terminals.len(), 2);
        router.route_net_seq(0, 0.0, true, 4.0);
        router.stamp(0);
        assert!(router.nets[0].complete);
        assert_eq!(router.retry_released(60.0), 1);
        assert!(router.released().is_empty());
        assert!(router.nets[1].complete && router.nets[1].routable);
        assert_eq!(router.board.nets[0].terminals.len(), 3);
        assert!(router.nets[0].complete);
        // No room even with the others fixed: back as it was.
        let board = wall_board(true);
        let mut router = Router::new(&board, &config());
        router.run_in_place();
        router.released[0].reason = NO_ROOM;
        assert_eq!(router.retry_released(60.0), 0);
        assert_eq!(router.released().len(), 1);
        assert_eq!(router.board.nets[0].terminals.len(), 2);
        assert!(router.nets[0].complete && !router.nets[1].routable);
    }

    fn config() -> Config {
        Config {
            pitches: vec![0.1],
            ..Config::default()
        }
    }

    /// The link finds no path past the wall: the negotiation gives it up
    /// after the round (no path) and P, its terminals as `GiveUp` says,
    /// routes from pad to pad through the gap in the same route. Without
    /// the give-up the link stays open and P only reaches the run's ends.
    #[test]
    fn a_link_without_a_path_is_given_up_and_its_nets_route_without_it() {
        let board = wall_board(true);
        let mut router = Router::new(&board, &config());
        let result = router.run_in_place();
        assert_eq!(result.diagnostics.released.len(), 1);
        let released = &result.diagnostics.released[0];
        assert_eq!((released.net, released.reason), (1, NO_PATH));
        assert_eq!(result.status[1], NetStatus::Trivial);
        assert_eq!(result.status[0], NetStatus::Routed);
        let crosses = |result: &RoutingResult| {
            result.routes[0]
                .segments
                .iter()
                .any(|segment| (segment.start[0] - 15.0) * (segment.end[0] - 15.0) <= 0.0)
        };
        assert!(crosses(&result), "{:?}", result.routes[0].segments);
        assert_eq!(router.released().len(), 1);
        // A board of the same plan updates: the link is given up on it
        // too, so the nets are the same.
        assert!(router.update(&board).is_ok());
        assert_eq!(router.released().len(), 1);
        // Without the give-up: the link open, P not across the wall.
        let plain = wall_board(false);
        let result = Router::new(&plain, &config()).run();
        assert!(result.diagnostics.released.is_empty());
        assert_ne!(result.status[1], NetStatus::Routed);
        assert_ne!(result.status[1], NetStatus::Trivial);
        assert!(!crosses(&result));
    }

    fn run(standings: &[Standing], rule: &GiveUpRule) -> Option<(usize, &'static str)> {
        let mut record = GiveUpRecord::default();
        standings.iter().enumerate().find_map(|(round, standing)| {
            record
                .round(*standing, rule, rule.min_present)
                .map(|why| (round, why))
        })
    }

    #[test]
    fn a_link_whose_conflicts_stop_falling_is_given_up_after_the_stall() {
        let rule = GiveUpRule {
            min_rounds: 4,
            stall_rounds: 3,
            pathless_rounds: 2,
            max_rounds: 10,
            min_present: 100.0,
        };
        use Standing::*;
        // Falling conflicts: never before the cap.
        let falling: Vec<Standing> = (0..9).map(|round| Conflicted(40 - round)).collect();
        assert_eq!(run(&falling, &rule), None);
        // The cap: the tenth unsettled round.
        let mut capped = falling.clone();
        capped.push(Conflicted(1));
        assert_eq!(run(&capped, &rule), Some((9, NO_ROOM)));
        // Its lowest in round 1, then three rounds not below it: round 4.
        let stalled = [
            Conflicted(30),
            Conflicted(20),
            Conflicted(25),
            Conflicted(20),
            Conflicted(22),
        ];
        assert_eq!(run(&stalled, &rule), Some((4, NO_ROOM)));
        // Not before the minimum: a stall from round 0 waits for round 3.
        let flat = [Conflicted(9); 6];
        assert_eq!(run(&flat, &rule), Some((3, NO_ROOM)));
        // Below the price the rounds count from, a round in conflict does
        // not count (a pathless one does): the stall starts at the price.
        let mut record = GiveUpRecord::default();
        let prices = [0.5, 8.0, 64.0, 99.0, 100.0, 200.0, 400.0, 800.0];
        let went: Vec<Option<&str>> = prices
            .iter()
            .map(|present| record.round(Conflicted(9), &rule, *present))
            .collect();
        assert_eq!(went[..7], [None; 7]);
        assert_eq!(went[7], Some(NO_ROOM));
        let mut record = GiveUpRecord::default();
        assert_eq!(record.round(Pathless, &rule, 0.5), None);
        assert_eq!(record.round(Pathless, &rule, 0.75), Some(NO_PATH));
    }

    #[test]
    fn a_link_without_a_path_goes_at_once_and_a_settled_round_starts_afresh() {
        let rule = GiveUpRule::default();
        use Standing::*;
        assert_eq!(run(&[Conflicted(5), Pathless], &rule), Some((1, NO_PATH)));
        // Pathless rounds in a row where the rule asks for two: a path
        // between them, no give-up yet.
        let two = GiveUpRule {
            pathless_rounds: 2,
            ..rule
        };
        assert_eq!(
            run(&[Conflicted(5), Pathless, Pathless], &two),
            Some((2, NO_PATH))
        );
        assert_eq!(run(&[Pathless, Conflicted(5), Pathless], &two), None);
        // Settled rounds reset the count: three unsettled, settled, three
        // unsettled never reach the minimum's stall.
        let mixed = [
            Conflicted(9),
            Conflicted(9),
            Conflicted(9),
            Settled,
            Conflicted(9),
            Conflicted(9),
            Conflicted(9),
        ];
        assert_eq!(run(&mixed, &rule), None);
        // Settled throughout: never.
        assert_eq!(run(&[Settled; 20], &rule), None);
    }
}

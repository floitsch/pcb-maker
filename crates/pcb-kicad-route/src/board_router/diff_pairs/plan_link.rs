// Copyright (C) 2026 Toit contributors.

//! One link of the plan (`plan_link`): its launches at both ends (a
//! crossover at a crossed end where one fits), then its net, the pair's
//! nets' terminals at the stubs' ends and the plan's copper on the board.

use super::*;
use pcb_router as core;

/// Launches closer than this make no coupled run: the pads' stubs are the
/// whole connection.
const MINIMUM_RUN: f64 = 1.0;

/// A pair whose links are planned (`plan_pairs`): its index, its links'
/// class and that class's rule (the envelope), and the other pairs'
/// launches (`Neighbour`).
pub(super) struct PairSite<'a> {
    pub index: usize,
    pub pair: &'a Pair,
    pub class: core::ClassId,
    pub envelope: core::RuleClass,
    pub neighbours: Vec<Neighbour>,
}

/// A link's launches as chosen (`choose_launches`): per end, the side of
/// the run the positive net takes, and whether the pads' order crosses the
/// run's at one end.
struct Chosen {
    launches: [Launch; 2],
    side: f64,
    crossed: bool,
}

/// Plans link `link` of the pair at `site`: its launches, then, where they
/// make a run, its net, terminals and copper on `board`, its ends joined in
/// the pair's nets' `parents` and its plan in `links`; else the reason in
/// the pair's `entry`.
pub(super) fn plan_link(
    board: &mut core::Board,
    site: &PairSite,
    link: usize,
    entry: &mut KiCadDiffPair,
    (parents, links): (&mut Parents, &mut Vec<LinkPlan>),
) {
    let pair = site.pair;
    let (a, b) = pair.links[link];
    let (point_a, point_b) = (pair.points[a], pair.points[b]);
    let terminal =
        |net: core::NetId, terminal: usize| board.nets[net as usize].terminals[terminal].clone();
    let ends = [
        (
            terminal(pair.positive, point_a.0),
            terminal(pair.negative, point_a.1),
        ),
        (
            terminal(pair.positive, point_b.0),
            terminal(pair.negative, point_b.1),
        ),
    ];
    let chosen = match choose_launches(board, site, (link, [a, b]), &ends) {
        Ok(chosen) => chosen,
        Err(why) => {
            entry.why.push(why);
            return;
        }
    };
    let Chosen {
        launches: [launch_a, launch_b],
        side,
        crossed,
    } = chosen;
    let id = board.nets.len() as core::NetId;
    let mut copper = LinkBuild {
        id,
        link,
        side,
        terminals: Vec::new(),
        stub_ends: [[[0.0; 2]; 2]; 2],
        runs: Default::default(),
    };
    for (end, found) in [&launch_a, &launch_b].into_iter().enumerate() {
        copper.add_end(board, site, end, found);
    }
    let fixed = [launch_a.fixed, launch_b.fixed];
    if debug() {
        eprintln!(
            "diff pair link plan {} / {} link {}: crossed {crossed}, fixed {fixed:?}, layers {:#x}, side {side}, launches {:?} {:?}, crossovers {:?} {:?}",
            board.nets[pair.positive as usize].name,
            board.nets[pair.negative as usize].name,
            link + 1,
            pair.layers,
            (launch_a.start, launch_a.at, launch_a.side),
            (launch_b.start, launch_b.at, launch_b.side),
            launch_a.crossover,
            launch_b.crossover
        );
    }
    let LinkBuild {
        terminals,
        stub_ends,
        runs,
        ..
    } = copper;
    board.nets.push(core::Net {
        name: link_name(
            &board.nets[pair.positive as usize].name,
            &board.nets[pair.negative as usize].name,
            link + 1,
        ),
        class: site.class,
        terminals,
    });
    join_link_ends(board, pair, [point_a, point_b], fixed, parents);
    links.push(LinkPlan {
        net: id,
        pair: site.index,
        link,
        launches: [launch_a.at, launch_b.at],
        // Past a crossover the run starts beyond it.
        starts: [&launch_a, &launch_b].map(|found| {
            found
                .crossover
                .as_ref()
                .map_or(found.start, |crossing| crossing.run_start)
        }),
        ends: stub_ends,
        pads: [
            [ends[0].0.anchor, ends[0].1.anchor],
            [ends[1].0.anchor, ends[1].1.anchor],
        ],
        side,
        crossed,
        fixed,
        stubs: [launch_a.stubs, launch_b.stubs],
        crossovers: [launch_a.crossover, launch_b.crossover],
        runs: runs.map(|end| end.map(|run| run.expect("a run terminal per end and net"))),
    });
}

/// A link's launches at its two ends (`ends`: the pads of its pair points
/// `points`): where the stubs the link carries where there are such, else
/// (or where those leave no run) the closest; at a crossed end a
/// crossover, or room for the router's crossing; and no pin between the
/// pads of a pair point across a pin walled in. `Err`: why the link makes
/// no run.
fn choose_launches(
    board: &core::Board,
    site: &PairSite,
    (link, points): (usize, [usize; 2]),
    ends: &[(core::Terminal, core::Terminal); 2],
) -> Result<Chosen, String> {
    let pair = site.pair;
    let launch_at = |end: usize, want_fixed: bool, lead_kind: Lead| {
        let middle = scale(add(ends[1 - end].0.anchor, ends[1 - end].1.anchor), 0.5);
        launch(
            board,
            pair,
            &site.envelope,
            &ends[end].0,
            &ends[end].1,
            middle,
            want_fixed,
            lead_kind,
            &site.neighbours,
        )
    };
    // Launches whose stubs the link carries where there are such, else
    // (or where those leave no run) the closest.
    let mut launches = [
        launch_at(0, true, Lead::Straight),
        launch_at(1, true, Lead::Straight),
    ];
    let too_close = |launches: &[Option<Launch>; 2]| match launches {
        [Some(a), Some(b)] => core::geometry::distance(a.at, b.at) < MINIMUM_RUN,
        _ => false,
    };
    if too_close(&launches) && launches.iter().flatten().any(|found| found.fixed) {
        launches = [
            launch_at(0, false, Lead::Straight),
            launch_at(1, false, Lead::Straight),
        ];
    }
    let close = too_close(&launches);
    let [Some(mut launch_a), Some(mut launch_b)] = launches else {
        return Err(format!("link {}: no launch clear of the pads", link + 1));
    };
    if close {
        return Err(format!(
            "link {}: the launches are too close for a coupled run",
            link + 1
        ));
    }
    // Along the run from the first end the positive net keeps the
    // side its pad has there; at the second end the run arrives
    // against that launch's outward direction, so the pads' order
    // matches the run's where the positive pad is on the other side
    // of that direction. Where it is not (crossed), the stubs cross
    // at one end, which the router routes (with a via): the second
    // end, unless only the second end's stubs are straight.
    let crossed = launch_b.side == launch_a.side;
    let mut side = if crossed && !launch_a.fixed && launch_b.fixed {
        -launch_b.side
    } else {
        launch_a.side
    };
    let mut crossing = None;
    if crossed && crossovers() {
        crossing = crossover_end(&launch_at, &launch_a, &launch_b);
    }
    if let Some((end, found)) = crossing {
        if end == 1 {
            side = launch_a.side;
            launch_b = found;
        } else {
            side = -launch_a.side;
            launch_a = found;
        }
    } else if crossed {
        // The crossed end's stubs are routed: its closest launch.
        if side == launch_a.side {
            launch_b = launch_at(1, false, Lead::Crossing).unwrap_or(launch_b);
            launch_b.fixed = false;
        } else {
            launch_a = launch_at(0, false, Lead::Crossing).unwrap_or(launch_a);
            launch_a.fixed = false;
        }
        if core::geometry::distance(launch_a.at, launch_b.at) < MINIMUM_RUN {
            return Err(format!(
                "link {}: the launches are too close for a coupled run",
                link + 1
            ));
        }
    }
    // At a pair point across a pin the pin must keep a way out past
    // the stubs and the run's lead (`walled_pin`).
    if let Some(pin) = [(&launch_a, 0usize), (&launch_b, 1)]
        .into_iter()
        .filter(|(_, end)| pair.kinds[points[*end]] == PointKind::Pin)
        .find_map(|(found, end)| {
            walled_pin(
                board,
                pair,
                &site.envelope,
                (&ends[end].0, &ends[end].1),
                found,
            )
        })
    {
        if pairs_why() {
            eprintln!(
                "pairs why: walled {}: link {}, {pin}",
                board.nets[pair.positive as usize].name,
                link + 1
            );
        }
        return Err(format!(
            "link {}: the pin between its pads ({pin}) would be walled in by its stubs",
            link + 1
        ));
    }
    Ok(Chosen {
        launches: [launch_a, launch_b],
        side,
        crossed,
    })
}

/// At a crossed end a crossover in the lead (`crossover`), where one
/// fits: at the second end, else at the first; one whose stubs the link
/// carries first. The end and its launch.
fn crossover_end(
    launch_at: &dyn Fn(usize, bool, Lead) -> Option<Launch>,
    launch_a: &Launch,
    launch_b: &Launch,
) -> Option<(usize, Launch)> {
    let at_b = launch_at(1, true, Lead::Crossover);
    let at_a = launch_at(0, true, Lead::Crossover);
    let far = |found: &Launch, other: &Launch| {
        core::geometry::distance(found.at, other.at) >= MINIMUM_RUN
    };
    // Only where the link carries the stubs to it: a crossover
    // whose stubs the router routes can wall in a pad with its
    // copper (a dead pad, the stub's net open; pairs6).
    let options = [
        at_b.filter(|found| found.fixed && far(found, launch_a))
            .map(|found| (1, found)),
        at_a.filter(|found| found.fixed && far(found, launch_b))
            .map(|found| (0, found)),
    ];
    let fixed_at = |end: usize, found: &Launch| found.fixed && [launch_a, launch_b][1 - end].fixed;
    // Both ends' stubs the link's first.
    options
        .iter()
        .flatten()
        .find(|(end, found)| fixed_at(*end, found))
        .or(options.iter().flatten().next())
        .cloned()
}

/// A link's net and the pair's nets' terminals at its ends as they are
/// added to the board (`LinkBuild::add_end`).
struct LinkBuild {
    /// The link's net and number, the positive net's side.
    id: core::NetId,
    link: usize,
    side: f64,
    /// The link's net's terminals (one per end).
    terminals: Vec<core::Terminal>,
    /// Per end and polarity: where the stub ends, and its terminal.
    stub_ends: [[Point; 2]; 2],
    runs: [[Option<core::Terminal>; 2]; 2],
}

impl LinkBuild {
    /// The link's end `end` at `found` on `board`: the launch's terminal
    /// and lead, the stubs' ends of the pair's nets, the straight stubs the
    /// link carries and a crossover's copper.
    fn add_end(&mut self, board: &mut core::Board, site: &PairSite, end: usize, found: &Launch) {
        let (pair, link) = (site.pair, self.link);
        let label = format!("pair launch {}{}", link + 1, ["a", "b"][end]);
        // The lead past the stubs' ends: the terminal's copper; the
        // search starts close to the launch.
        self.terminals.push(core::Terminal {
            anchor: found.at,
            layers: found.layers,
            pad: board.obstacles.len(),
            contact: Some(core::Shape::Circle {
                center: found.at,
                radius: LAUNCH_CONTACT,
            }),
            label: label.clone(),
        });
        // Where the link carries the stubs its copper begins at the
        // run's start, so that no other net comes between the run's
        // two tracks' beginnings and their clearance. It is the
        // terminal's pad ("pair launch": the router does not neck the
        // link down near it, `Router::prepare_net`).
        board.obstacles.push(net_copper(
            core::Shape::Capsule {
                start: if found.fixed && found.crossover.is_none() {
                    found.start
                } else {
                    found.reserved
                },
                end: found.at,
                radius: site.envelope.trace_width / 2.0,
            },
            found.layers,
            core::ObstacleKind::Copper,
            self.id,
            label,
        ));
        self.add_stub_ends(board, pair, end, found);
        // Straight stubs the link carries: copper of the pair's nets
        // (other nets keep clear of it); the pads are of one
        // terminal with the run's ends.
        if found.fixed {
            for (polarity, net) in [pair.positive, pair.negative].into_iter().enumerate() {
                board.obstacles.push(net_copper(
                    core::Shape::Capsule {
                        start: found.stubs[polarity],
                        end: self.stub_ends[end][polarity],
                        radius: pair.width / 2.0,
                    },
                    found.layers,
                    core::ObstacleKind::Copper,
                    net,
                    format!("pair stub {}{}", link + 1, ["a", "b"][end]),
                ));
            }
        }
        // A crossover's copper: of the pair's nets (other nets keep
        // clear of it).
        if let Some(crossing) = &found.crossover {
            add_crossover_copper(board, pair, crossing, (link, end));
        }
    }

    /// The stubs' ends at `end`: a terminal of each of the pair's nets,
    /// its copper the run's track up to the link's (which the other nets
    /// keep clear of).
    fn add_stub_ends(&mut self, board: &mut core::Board, pair: &Pair, end: usize, found: &Launch) {
        let (side, link) = (self.side, self.link);
        let across = if end == 0 {
            left(found.outward)
        } else {
            scale(left(found.outward), -1.0)
        };
        let reach = found.ends;
        for (polarity, (net, sign)) in [(pair.positive, side), (pair.negative, -side)]
            .into_iter()
            .enumerate()
        {
            // At a crossover the stubs' ends are on the pads' sides.
            let at = if found.crossover.is_some() {
                found.stub_ends[polarity]
            } else {
                add(found.start, scale(across, sign * pair.ends_offset()))
            };
            self.stub_ends[end][polarity] = at;
            let run = core::Terminal {
                anchor: at,
                layers: found.layers,
                pad: board.obstacles.len(),
                contact: Some(core::Shape::Circle {
                    center: at,
                    radius: LAUNCH_CONTACT,
                }),
                label: format!("pair run {}{}", link + 1, ["a", "b"][end]),
            };
            board.nets[net as usize].terminals.push(run.clone());
            self.runs[end][polarity] = Some(run);
            board.obstacles.push(net_copper(
                core::Shape::Capsule {
                    start: at,
                    end: add(at, reach),
                    radius: pair.width / 2.0,
                },
                found.layers,
                core::ObstacleKind::Copper,
                net,
                format!("pair run {}{}", link + 1, ["a", "b"][end]),
            ));
        }
    }
}

/// A crossover's tracks and vias (copper and hole) at end `end` of link
/// `link`, as copper of the pair's nets on `board`.
fn add_crossover_copper(
    board: &mut core::Board,
    pair: &Pair,
    crossing: &Crossover,
    (link, end): (usize, usize),
) {
    for (polarity, net) in [pair.positive, pair.negative].into_iter().enumerate() {
        for &(layer, from, to) in &crossing.segments[polarity] {
            board.obstacles.push(net_copper(
                core::Shape::Capsule {
                    start: from,
                    end: to,
                    radius: pair.width / 2.0,
                },
                1 << layer,
                core::ObstacleKind::Copper,
                net,
                format!("pair crossover {}{}", link + 1, ["a", "b"][end]),
            ));
        }
        for &via in &crossing.vias[polarity] {
            for (kind, radius) in [
                (core::ObstacleKind::Copper, pair.via_diameter / 2.0),
                (core::ObstacleKind::Hole, pair.via_drill / 2.0),
            ] {
                board.obstacles.push(net_copper(
                    core::Shape::Circle {
                        center: via,
                        radius,
                    },
                    board.all_layers(),
                    kind,
                    net,
                    format!("pair crossover {}{}", link + 1, ["a", "b"][end]),
                ));
            }
        }
    }
}

/// The run's two ends of each of the pair's nets (the last two of its
/// terminals) joined with each other and with the pads at the pair points
/// `points` whose stubs the link carries (`fixed`), in the nets' `parents`.
fn join_link_ends(
    board: &core::Board,
    pair: &Pair,
    points: [(usize, usize); 2],
    fixed: [bool; 2],
    parents: &mut Parents,
) {
    for (polarity, net) in [pair.positive, pair.negative].into_iter().enumerate() {
        let count = board.nets[net as usize].terminals.len();
        let parent = parents.entry(net).or_default();
        parent.extend(parent.len()..count);
        // The run's two ends, the last two pairs of terminals, and
        // the pads whose stubs the link carries.
        let pad_of = |point: (usize, usize)| [point.0, point.1][polarity];
        let mut joined = vec![count - 2, count - 1];
        for (end, point) in points.into_iter().enumerate() {
            if fixed[end] {
                joined.push(pad_of(point));
            }
        }
        for item in &joined[1..] {
            let (x, y) = (root(parent, joined[0]), root(parent, *item));
            if x != y {
                parent[x.max(y)] = x.min(y);
            }
        }
    }
}

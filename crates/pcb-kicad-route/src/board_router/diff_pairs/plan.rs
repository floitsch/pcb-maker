// Copyright (C) 2026 Toit contributors.

//! The plan before routing (`plan_pairs`): each link between a pair's pair
//! points a net of the route, a class per layer set for them, and the plan
//! kept on the board (`Board::attachment`) for `finish_pairs`.

use super::*;
use pcb_router as core;

/// The name of a pair's coupled link (a net of its own on the lowered
/// board), the `number`-th of the pair of `positive` and `negative`.
pub(crate) fn link_name(positive: &str, negative: &str, number: usize) -> String {
    format!("{positive} / {negative} link {number}")
}

/// A link routed as a net of its own (`PairPlan::links`).
#[derive(Clone, Debug)]
pub(super) struct LinkPlan {
    pub net: core::NetId,
    pub pair: usize,
    pub link: usize,
    /// Per end: the launch (where the link's net starts) and where the run
    /// starts.
    pub launches: [Point; 2],
    pub starts: [Point; 2],
    /// Per end and polarity: where the stubs end, and the pads they start
    /// from.
    pub ends: [[Point; 2]; 2],
    pub pads: [[Point; 2]; 2],
    /// The side of the run the positive net takes (1 left, -1 right),
    /// along the run from the first end.
    pub side: f64,
    /// Whether the pads' order at one end crosses the run's (the stubs
    /// cross there, routed by the router).
    pub crossed: bool,
    /// Per end: whether the link carries the stubs there, straight from
    /// the pads to the stubs' ends (`stubs_clear`; the pads are of the
    /// run's terminal), or the router routes them.
    pub fixed: [bool; 2],
    /// Per end and polarity: where those stubs start on the pads.
    pub stubs: [[Point; 2]; 2],
    /// Per end: the crossover in its lead (at a crossed end, where one
    /// fits): the run starts past it (`starts`), the stubs' ends are on the
    /// pads' sides.
    pub crossovers: [Option<Crossover>; 2],
    /// Per end and polarity: the terminal of the pair's net at the stub's
    /// end (`plan_pairs`), for the nets as they are with the link given up
    /// (`GiveUp::released`: without it).
    pub runs: [[core::Terminal; 2]; 2],
}

impl LinkPlan {
    /// The copper of the stub end of `polarity` at `end`: from where the
    /// stub ends a track width along the lead (`plan_pairs`).
    pub(super) fn stub_end(&self, end: usize, polarity: usize, width: f64) -> (Point, Point) {
        let at = self.ends[end][polarity];
        let along = unit(sub(self.launches[end], self.starts[end]));
        (at, add(at, scale(along, width)))
    }
}

/// A board whose pairs' links are nets of the route (`plan_pairs`), as
/// its `Board::attachment` keeps it for `finish_pairs`.
#[derive(Clone)]
pub(super) struct PairPlan {
    /// The board's nets, obstacles and classes before the links' (which
    /// follow).
    pub nets: usize,
    pub obstacles: usize,
    pub classes: usize,
    /// The pairs' nets and the board's merged pads and terminal copper as
    /// lowered, before the links' ends were added and merged.
    pub originals: Vec<(core::NetId, core::Net)>,
    pub merged_pads: Vec<core::MergedPad>,
    pub terminal_copper: Vec<core::TerminalCopper>,
    pub pairs: Vec<Pair>,
    pub entries: Vec<KiCadDiffPair>,
    pub links: Vec<LinkPlan>,
    /// The plan's work (`plan_work_of` its obstacle tests), booked by the
    /// stage that lowered the board (`planned_work`).
    pub work_seconds: f64,
}

/// Per pair net: its terminals joined by the links (union-find parents),
/// the stubs' ends added.
pub(super) type Parents = BTreeMap<core::NetId, Vec<usize>>;

/// Makes each link between a pair's pair points a net of the route:
/// launches at both ends (`launch`), the run's lead as its copper (other
/// nets keep clear of it), a class for the pairs of one layer set (the
/// widest envelope and via of them, kept to the layers); the pair's nets
/// get a terminal beside each end of the run (their stubs from the pads
/// end there: ordinary routing), the two ends of a link one terminal. The
/// plan goes into `board.attachment` for `finish_pairs`.
pub(crate) fn plan_pairs(board: &mut core::Board, config: &KiCadBoardRouterConfig) {
    let tests_before = tests_so_far();
    let started = std::time::Instant::now();
    let pairs = find_pairs(board, config);
    let mut entries = pair_entries(board, &pairs);
    let mut plan = PairPlan {
        nets: board.nets.len(),
        obstacles: board.obstacles.len(),
        classes: board.classes.len(),
        originals: Vec::new(),
        merged_pads: board.merged_pads.clone(),
        terminal_copper: board.terminal_copper.clone(),
        pairs: Vec::new(),
        entries: Vec::new(),
        links: Vec::new(),
        work_seconds: 0.0,
    };
    let class_of = pair_classes(board, &pairs, &mut entries);
    // With crossovers: every link end's launch on the board as lowered (no
    // link's copper yet), the room a crossover leaves its neighbours
    // (`neighbour_room`).
    let mut launches_before: Vec<Neighbour> = Vec::new();
    if crossovers() {
        launches_before = neighbour_launches(board, &pairs, &entries, &class_of);
    }
    let mut parents: Parents = BTreeMap::new();
    for (index, pair) in pairs.iter().enumerate() {
        let Some(&class) = class_of.get(&pair.layers) else {
            continue;
        };
        if !entries[index].why.is_empty() {
            continue;
        }
        let site = PairSite {
            index,
            pair,
            class,
            envelope: board.classes[class],
            neighbours: launches_before
                .iter()
                .filter(|neighbour| neighbour.pair != index)
                .cloned()
                .collect(),
        };
        for link in 0..pair.links.len() {
            plan_link(
                board,
                &site,
                link,
                &mut entries[index],
                (&mut parents, &mut plan.links),
            );
        }
    }
    merge_linked_terminals(board, &mut plan.originals, &mut parents);
    if debug() {
        eprintln!(
            "diff pairs: {} found, {} links as nets of the route",
            pairs.len(),
            plan.links.len()
        );
    }
    plan.pairs = pairs;
    plan.entries = entries;
    plan.work_seconds = plan_work_since(tests_before);

    if debug() {
        let [scans, near] = tests_so_far();
        eprintln!(
            "diff pairs plan: {} obstacles scanned, {} tested, {:.4} s, {:.4} s of work modelled (rate scale {:.3})",
            scans - tests_before[0],
            near - tests_before[1],
            started.elapsed().as_secs_f64(),
            plan.work_seconds,
            core::router::reference_time_scale()
        );
    }
    if std::env::var_os("PCB_ROUTER_PAIRS_REACH").is_some() {
        plan_reach_check(board, &plan, config);
    }
    let plan = std::sync::Arc::new(plan);
    board.attachment = core::Attachment {
        data: Some(plan.clone()),
        give_up: give_up_links(config).then_some(plan),
    };
}

/// The report's entry of each pair as found: its nets, width, gap and
/// links, and why it has none.
fn pair_entries(board: &core::Board, pairs: &[Pair]) -> Vec<KiCadDiffPair> {
    pairs
        .iter()
        .map(|pair| KiCadDiffPair {
            positive: board.nets[pair.positive as usize].name.clone(),
            negative: board.nets[pair.negative as usize].name.clone(),
            width_mm: pair.width,
            gap_mm: pair.gap,
            source: pair.source.clone(),
            links: pair.links.len(),
            why: if pair.links.is_empty() {
                vec!["fewer than two parts with the pair's pads side by side".into()]
            } else {
                Vec::new()
            },
            ..KiCadDiffPair::default()
        })
        .collect()
}

/// A class per layer set: the widest envelope, clearance and via of its
/// pairs (each class builds static maps of the whole board), added to the
/// board. A pair with no layer for its width and gap, or with kept copper
/// on its nets, gets its why instead.
fn pair_classes(
    board: &mut core::Board,
    pairs: &[Pair],
    entries: &mut [KiCadDiffPair],
) -> BTreeMap<core::LayerMask, core::ClassId> {
    let mut class_of: BTreeMap<core::LayerMask, core::ClassId> = BTreeMap::new();
    for (index, pair) in pairs.iter().enumerate() {
        if pair.links.is_empty() {
            continue;
        }
        if pair.layers == 0 {
            entries[index]
                .why
                .push("no layer holds the pair's width and gap".into());
            continue;
        }
        // Kept copper on the pair's nets (a board routed in part) keeps its
        // terminals as they are.
        if board
            .terminal_copper
            .iter()
            .any(|extra| extra.net == pair.positive || extra.net == pair.negative)
        {
            entries[index]
                .why
                .push("the pair's nets have kept copper".into());
            continue;
        }
        if class_of.contains_key(&pair.layers) {
            continue;
        }
        let members: Vec<&Pair> = pairs
            .iter()
            .filter(|other| other.layers == pair.layers && !other.links.is_empty())
            .collect();
        let widest = |measure: &dyn Fn(&Pair) -> f64| {
            members.iter().map(|pair| measure(pair)).fold(0.0, f64::max)
        };
        board.classes.push(core::RuleClass {
            trace_width: widest(&|pair| pair.envelope()),
            clearance: widest(&|pair| pair.clearance),
            // Two vias side by side, as far apart as at a right-angle turn
            // (`link_copper`).
            via_diameter: widest(&|pair| 2.0 * pair.via_offset() / VIA_TURN + pair.via_diameter),
            via_drill: widest(&|pair| 2.0 * pair.via_offset() / VIA_TURN + pair.via_drill),
            layers: if pair.layers == board.all_layers() {
                0
            } else {
                pair.layers
            },
        });
        class_of.insert(pair.layers, board.classes.len() - 1);
    }
    class_of
}

/// Every link end's launch on the board as lowered (no link's copper yet),
/// as a neighbour of the other pairs' crossovers (`Neighbour`).
fn neighbour_launches(
    board: &core::Board,
    pairs: &[Pair],
    entries: &[KiCadDiffPair],
    class_of: &BTreeMap<core::LayerMask, core::ClassId>,
) -> Vec<Neighbour> {
    let mut launches_before: Vec<Neighbour> = Vec::new();
    for (index, pair) in pairs.iter().enumerate() {
        let Some(&class) = class_of.get(&pair.layers) else {
            continue;
        };
        if !entries[index].why.is_empty() {
            continue;
        }
        let envelope = board.classes[class];
        for &(a, b) in &pair.links {
            let points = [pair.points[a], pair.points[b]];
            let pads = points.map(|(p, n)| {
                (
                    &board.nets[pair.positive as usize].terminals[p],
                    &board.nets[pair.negative as usize].terminals[n],
                )
            });
            for end in 0..2 {
                let (p, n) = pads[end];
                let (q, m) = pads[1 - end];
                let toward = scale(add(q.anchor, m.anchor), 0.5);
                let Some(found) = launch(
                    board,
                    pair,
                    &envelope,
                    p,
                    n,
                    toward,
                    true,
                    Lead::Straight,
                    &[],
                ) else {
                    continue;
                };
                let half = pair.width / 2.0;
                let along = sub(found.reserved, found.start);
                let lead_of = |end: Point| (end, add(end, along), half, pair.clearance);
                launches_before.push(Neighbour {
                    pair: index,
                    layers: found.layers,
                    copper: [
                        (found.stubs[0], found.stub_ends[0], half, pair.clearance),
                        (found.stubs[1], found.stub_ends[1], half, pair.clearance),
                        lead_of(found.stub_ends[0]),
                        lead_of(found.stub_ends[1]),
                        (
                            found.start,
                            found.at,
                            envelope.trace_width / 2.0 + LAUNCH_MARGIN,
                            envelope.clearance,
                        ),
                    ],
                });
            }
        }
    }
    launches_before
}

/// Each pair net's terminals merged as the links join them (`parents`);
/// its terminals as lowered, without the stubs' ends, kept in `originals`.
fn merge_linked_terminals(
    board: &mut core::Board,
    originals: &mut Vec<(core::NetId, core::Net)>,
    parents: &mut Parents,
) {
    for (&net, parent) in parents {
        let mut original = board.nets[net as usize].clone();
        original
            .terminals
            .retain(|terminal| !terminal.label.starts_with("pair run "));
        originals.push((net, original));
        let host_of: Vec<usize> = (0..parent.len()).map(|t| root(parent, t)).collect();
        merge_terminals(board, net, &host_of);
    }
}

/// Whether a route with `config` plans the pairs (`plan_pairs`): the
/// router's `diff_pairs` on, and not the topological engine.
pub fn pairs_planned(config: &KiCadBoardRouterConfig) -> bool {
    config.diff_pairs_on() && config.engine.as_deref() != Some("topological")
}

/// The modelled work of the pairs' plan on `board` (`plan_pairs`), zero
/// where it has none.
pub(crate) fn planned_work(board: &core::Board) -> f64 {
    plan_of(board).map_or(0.0, |plan| plan.work_seconds)
}

/// The pairs' plan on `board` (`plan_pairs`), if it has one.
pub(super) fn plan_of(board: &core::Board) -> Option<std::sync::Arc<PairPlan>> {
    board
        .attachment
        .data
        .as_ref()
        .and_then(|any| any.clone().downcast::<PairPlan>().ok())
}

/// A layout's lowering for a stage that routes the board: `lower`, and
/// where the router plans the pairs (`pairs_planned`) their plan, so that
/// every route of a layout (the first route, the placement race's probes,
/// the moves, the outline's sizing) negotiates the pairs' links as the
/// final ladder's attempts do. Returns the board and the plan's work
/// (`planned_work`), which the stage books.
pub fn lower_for_route(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    connect: bool,
) -> Result<(core::Board, f64), String> {
    let mut board = lower(pcb, config, connect)?.board;
    if !pairs_planned(config) {
        return Ok((board, 0.0));
    }
    plan_pairs(&mut board, config);
    let work = planned_work(&board);
    Ok((board, work))
}

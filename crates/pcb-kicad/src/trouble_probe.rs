// Copyright (C) 2026 Toit contributors.

//! The trouble-maker probe (`pcb-maker probe-trouble-placement`, a
//! research command; docs/placer.md, "The trouble-maker probe
//! (2026-10-26)"): a placed board's difficulty measured by routing only
//! the nets that made trouble (the nets in conflict when a previous run's
//! negotiation ended, the nets it left open; or, cold, the nets with the
//! most pins times the largest box), first and alone, with a small work
//! budget, against a background for the other nets: none, a fixed escape
//! stub out of each of their pads, or the global route's use of the tiles
//! by them as a cost. Deterministic: the budget is work.

use super::*;
use crate::global_route::lower_placed;
use pcb_router as core;

/// Where the other nets' copper is assumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadTroubleBackground {
    /// The set alone on the board.
    None,
    /// A fixed stub out of every pad of the other nets
    /// (`Router::stamp_background_stubs`).
    Stubs,
    /// The global route's tile usage by the other nets as a cost
    /// (`Router::add_background_cost`).
    Global,
    /// No set: the race's whole-board probe (`Router::probe`), for
    /// comparison.
    Full,
}

impl KiCadTroubleBackground {
    pub fn parse(text: &str) -> Result<Self, String> {
        Ok(match text {
            "none" => Self::None,
            "stubs" => Self::Stubs,
            "global" => Self::Global,
            "full" => Self::Full,
            _ => {
                return Err(format!(
                    "--background: none, stubs, global or full, not {text}"
                ));
            }
        })
    }
}

/// Where the trouble set comes from.
#[derive(Clone, Debug)]
pub enum KiCadTroubleSource {
    /// A previous run: a layout or route directory, its board-router.json,
    /// or a previous probe's report (its `new_trouble`).
    From(PathBuf),
    /// Net names, one per line (`--trouble <file>`).
    Names(PathBuf),
    /// Cold: the nets with the largest pin count times bounding box
    /// half-perimeter (pour nets left out).
    Auto,
}

#[derive(Clone, Debug)]
pub struct KiCadTroubleProbeConfig {
    pub sources: Vec<KiCadTroubleSource>,
    /// Nets taken from each source; `None`: 15 % of the board's routable
    /// nets of two or more pads, at least 8.
    pub nets: Option<usize>,
    pub background: KiCadTroubleBackground,
    /// The work budget: `share` of `total_seconds`, or `work_seconds`.
    pub share: f64,
    pub total_seconds: f64,
    pub work_seconds: Option<f64>,
    /// The stubs' length (`Stubs`).
    pub stub_mm: f64,
    /// The cost at a tile the other nets fill (`Global`): `weight` times
    /// their tracks over its capacity (at most twice).
    pub global_weight: f64,
}

impl Default for KiCadTroubleProbeConfig {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            nets: None,
            background: KiCadTroubleBackground::None,
            share: 0.05,
            total_seconds: 1500.0,
            work_seconds: None,
            stub_mm: 1.0,
            global_weight: 2.0,
        }
    }
}

/// One net of the set after the probe.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadTroubleNet {
    pub net: String,
    pub open_terminals: usize,
    pub conflict_cells: usize,
    pub complete: bool,
    pub reroutes: usize,
    /// Its terminals' rectilinear spanning tree, mm.
    pub manhattan_mm: f64,
    /// Search work spent on it, seconds.
    pub work_seconds: f64,
    /// Work per millimetre of `manhattan_mm`, milliseconds.
    pub work_ms_per_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadTroubleProbeReport {
    pub board_id: String,
    pub background: KiCadTroubleBackground,
    /// Routable nets of two or more pads.
    pub board_nets: usize,
    /// The set probed (empty for `full`), in the routing order's terms:
    /// its names as chosen.
    pub trouble: Vec<String>,
    /// Names of the set the board does not have or cannot route.
    pub missing: Vec<String>,
    pub budget_seconds: f64,
    /// The set's (or for `full`, the board's) open terminals plus one for
    /// every complete net in conflict; nets unfinished; nets and cells in
    /// conflict.
    pub open_terminals: usize,
    pub unfinished: usize,
    pub conflicted: usize,
    pub conflict_cells: usize,
    pub iterations: usize,
    pub stop: String,
    /// Connections of the set (pads less one per net) and their
    /// spanning trees.
    pub connections: usize,
    pub manhattan_mm: f64,
    /// The negotiation's search work per connection (s) and per mm of the
    /// spanning trees (ms).
    pub work_per_connection: f64,
    pub work_ms_per_mm: f64,
    /// Searches outside their corridor (the window or the board) and the
    /// corridor searches that found nothing.
    pub corridor_searches: u64,
    pub corridor_misses: u64,
    pub wider_searches: u64,
    /// Background stubs fixed (`stubs`).
    pub stubs: usize,
    /// Work (s, at the pinned rates): the negotiation's search, the
    /// router's build, the global route's (`global`), all of it.
    pub search_work_seconds: f64,
    pub build_work_seconds: f64,
    pub global_work_seconds: f64,
    pub work_seconds: f64,
    pub wall_seconds: f64,
    /// The set's nets still open or in conflict (most open terminals,
    /// then conflict cells), then the others by work per mm: the next
    /// set's candidates.
    pub new_trouble: Vec<KiCadTroubleNet>,
}

/// The rectilinear spanning tree of `points` (Prim), mm.
fn manhattan_tree(points: &[[f64; 2]]) -> f64 {
    if points.len() < 2 {
        return 0.0;
    }
    let mut best = vec![f64::INFINITY; points.len()];
    let mut done = vec![false; points.len()];
    let mut total = 0.0;
    let mut current = 0;
    done[0] = true;
    for _ in 1..points.len() {
        let mut next = None;
        for index in 0..points.len() {
            if done[index] {
                continue;
            }
            let distance = (points[index][0] - points[current][0]).abs()
                + (points[index][1] - points[current][1]).abs();
            best[index] = best[index].min(distance);
            if next.is_none_or(|other: usize| best[index] < best[other]) {
                next = Some(index);
            }
        }
        let Some(next) = next else { break };
        done[next] = true;
        total += best[next];
        current = next;
    }
    total
}

/// The nets a source names, most trouble first.
fn source_nets(
    source: &KiCadTroubleSource,
    board: &core::Board,
    candidates: &[usize],
) -> Result<Vec<String>, String> {
    let read = |path: &Path| -> Result<serde_json::Value, String> {
        serde_json::from_str(
            &fs::read_to_string(path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?,
        )
        .map_err(|error| format!("{}: {error}", path.display()))
    };
    let span = |net: usize| -> f64 {
        let terminals = &board.nets[net].terminals;
        let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for terminal in terminals {
            for axis in 0..2 {
                low[axis] = low[axis].min(terminal.anchor[axis]);
                high[axis] = high[axis].max(terminal.anchor[axis]);
            }
        }
        terminals.len() as f64 * ((high[0] - low[0]) + (high[1] - low[1]))
    };
    let by_name: BTreeMap<&str, usize> = candidates
        .iter()
        .map(|net| (board.nets[*net].name.as_str(), *net))
        .collect();
    let span_of = |name: &str| by_name.get(name).map_or(0.0, |net| span(*net));
    match source {
        KiCadTroubleSource::Auto => {
            let pours: BTreeSet<usize> = board
                .planes
                .iter()
                .map(|plane| plane.net as usize)
                .collect();
            let mut nets: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|net| !pours.contains(net))
                .collect();
            nets.sort_by(|a, b| span(*b).total_cmp(&span(*a)).then(a.cmp(b)));
            Ok(nets
                .into_iter()
                .map(|net| board.nets[net].name.clone())
                .collect())
        }
        KiCadTroubleSource::Names(path) => Ok(fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()),
        KiCadTroubleSource::From(path) => {
            let file = if path.is_dir() {
                [
                    "result/board-router.json",
                    "layout/result/board-router.json",
                    "board-router.json",
                ]
                .iter()
                .map(|name| path.join(name))
                .find(|candidate| candidate.exists())
                .ok_or_else(|| format!("{}: no board-router.json in it", path.display()))?
            } else {
                path.clone()
            };
            let value = read(&file)?;
            if let Some(trouble) = value["new_trouble"].as_array() {
                // A previous probe: its new trouble-makers in its order.
                return Ok(trouble
                    .iter()
                    .filter(|net| {
                        net["open_terminals"].as_u64().unwrap_or(0) > 0
                            || net["conflict_cells"].as_u64().unwrap_or(0) > 0
                    })
                    .filter_map(|net| net["net"].as_str().map(str::to_string))
                    .collect());
            }
            // A run: open terminals at the end, then in conflict when the
            // negotiation ended (or named at a hot spot), then the span.
            let mut score: BTreeMap<String, (u64, u64)> = BTreeMap::new();
            for net in value["nets"].as_array().into_iter().flatten() {
                let Some(name) = net["connection"].as_str() else {
                    continue;
                };
                let open = net["unconnected_terminals"].as_u64().unwrap_or(0)
                    + u64::from(net["status"].as_str() == Some("unreachable"));
                if open > 0 {
                    score.entry(name.to_string()).or_default().0 += open;
                }
            }
            let diagnostics = &value["diagnostics"];
            for name in diagnostics["conflicted_nets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|name| name.as_str())
            {
                score.entry(name.to_string()).or_default().1 += 2;
            }
            for spot in diagnostics["hot_spots"].as_array().into_iter().flatten() {
                for name in spot["conflicted_nets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|name| name.as_str())
                {
                    score.entry(name.to_string()).or_default().1 += 1;
                }
            }
            let mut nets: Vec<(String, (u64, u64))> = score.into_iter().collect();
            nets.sort_by(|a, b| {
                (b.1)
                    .cmp(&a.1)
                    .then(span_of(&b.0).total_cmp(&span_of(&a.0)))
                    .then(a.0.cmp(&b.0))
            });
            Ok(nets.into_iter().map(|(name, _)| name).collect())
        }
    }
}

/// Probes the placed board `board_path` (a `.kicad_pcb`; tracks are
/// stripped, planned pours added) by its trouble-makers.
pub fn probe_trouble_placement(
    board_path: &Path,
    router_config: &KiCadBoardRouterConfig,
    config: &KiCadTroubleProbeConfig,
) -> Result<KiCadTroubleProbeReport, String> {
    let started = Instant::now();
    let board_id = board_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("{}: no board id", board_path.display()))?
        .to_string();
    let text = fs::read_to_string(board_path)
        .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let pcb = crate::canonical_order::parse_board(&text)?;
    let board = lower_placed(&pcb, router_config)?;
    let mut core_config = crate::congestion_samples::race_core_config(router_config);
    core_config.verbose = false;
    let budget = config
        .work_seconds
        .unwrap_or(config.share * config.total_seconds)
        .max(0.0);
    let mut router = core::router::Router::new(&board, &core_config);
    let candidates: Vec<usize> = (0..board.nets.len())
        .filter(|net| board.nets[*net].terminals.len() >= 2)
        .collect();
    let per_source = config
        .nets
        .unwrap_or(((candidates.len() as f64 * 0.15).ceil() as usize).max(8));
    let index: BTreeMap<&str, usize> = board
        .nets
        .iter()
        .enumerate()
        .map(|(net, description)| (description.name.as_str(), net))
        .collect();
    // Pour nets are left out: their trouble is the pour's connectivity,
    // which the other nets' copper decides and the set alone cannot show
    // (protoesp's GND: 24, 1 and 38 pads off the pour in the three
    // backgrounds of one placement).
    let pours: BTreeSet<usize> = board
        .planes
        .iter()
        .map(|plane| plane.net as usize)
        .collect();
    let mut trouble: Vec<String> = Vec::new();
    if config.background != KiCadTroubleBackground::Full {
        for source in &config.sources {
            let mut taken = 0;
            for name in source_nets(source, &board, &candidates)? {
                if taken >= per_source {
                    break;
                }
                if index
                    .get(name.as_str())
                    .is_some_and(|net| pours.contains(net))
                {
                    continue;
                }
                if !trouble.contains(&name) {
                    trouble.push(name);
                }
                taken += 1;
            }
        }
        if trouble.is_empty() {
            return Err("no trouble set: give --trouble-from, --trouble or --auto".into());
        }
    }
    let mut set: Vec<core::NetId> = Vec::new();
    let mut missing = Vec::new();
    for name in &trouble {
        match index.get(name.as_str()) {
            Some(net) if board.nets[*net].terminals.len() >= 2 => set.push(*net as core::NetId),
            _ => missing.push(name.clone()),
        }
    }
    let corridors_before = KiCadCorridors::now();
    let rate = core::router::expansions_per_second();
    let mut stubs = 0;
    let mut global_work_seconds = 0.0;
    match config.background {
        KiCadTroubleBackground::Stubs => {
            stubs = router.stamp_background_stubs(&set, config.stub_mm);
        }
        KiCadTroubleBackground::Global => {
            let route = pcb_global::route(&board, &pcb_global::GlobalConfig::default());
            // pcb-global's work model (docs/router.md, "Capacitated
            // corridors"): 5 units a capacity test, 30 a state popped,
            // 150e6 units a second.
            global_work_seconds =
                (route.capacity.work as f64 * 5.0 + route.pops as f64 * 30.0) / 150.0e6;
            let frame = &route.frame;
            let tiles = frame.tiles();
            let layers = frame.layers;
            let reference =
                route.capacity.reference.trace_width + route.capacity.reference.clearance;
            let mut usage = route.usage.clone();
            let in_set: BTreeSet<usize> = set.iter().map(|net| *net as usize).collect();
            let per = layers * tiles;
            for net_route in route.nets.iter().filter(|r| in_set.contains(&r.net)) {
                let class = board.classes[board.nets[net_route.net].class];
                let demand = ((class.trace_width + class.clearance) / reference) as f32;
                for &edge in &net_route.crossings {
                    let edge = edge as usize;
                    usage[edge / per][edge % per] -= demand;
                }
            }
            // Per layer and tile: the mean fill of its boundaries by the
            // other nets.
            let mut fill = vec![0.0f32; per];
            for layer in 0..layers {
                for tile in 0..tiles {
                    let (x, y) = (tile % frame.tiles_x, tile / frame.tiles_x);
                    let mut edges = vec![(0, tile), (1, tile)];
                    if x > 0 {
                        edges.push((0, tile - 1));
                    }
                    if y > 0 {
                        edges.push((1, tile - frame.tiles_x));
                    }
                    let (mut sum, mut count) = (0.0f32, 0);
                    for (direction, at) in edges {
                        let capacity = route.capacity.edges[direction][layer * tiles + at];
                        if capacity > 0.0 {
                            sum +=
                                (usage[direction][layer * tiles + at].max(0.0) / capacity).min(2.0);
                            count += 1;
                        }
                    }
                    if count > 0 {
                        fill[layer * tiles + tile] = sum / count as f32;
                    }
                }
            }
            let weight = config.global_weight as f32;
            let frame = frame.clone();
            router.add_background_cost(&|layer, point| {
                if layer >= frame.layers {
                    return 0.0;
                }
                weight * fill[layer * frame.tiles() + frame.tile_of(point)]
            });
        }
        KiCadTroubleBackground::None | KiCadTroubleBackground::Full => {}
    }
    let mut report = KiCadTroubleProbeReport {
        board_id,
        background: config.background,
        board_nets: candidates.len(),
        trouble: trouble.clone(),
        missing,
        budget_seconds: budget,
        open_terminals: 0,
        unfinished: 0,
        conflicted: 0,
        conflict_cells: 0,
        iterations: 0,
        stop: String::new(),
        connections: 0,
        manhattan_mm: 0.0,
        work_per_connection: 0.0,
        work_ms_per_mm: 0.0,
        corridor_searches: 0,
        corridor_misses: 0,
        wider_searches: 0,
        stubs,
        search_work_seconds: 0.0,
        build_work_seconds: router.build_work(),
        global_work_seconds,
        work_seconds: 0.0,
        wall_seconds: 0.0,
        new_trouble: Vec::new(),
    };
    let before = router.expansions();
    if config.background == KiCadTroubleBackground::Full {
        router.probe(budget);
        let standing = router.probe_standing();
        report.open_terminals = standing.open_terminals;
        report.unfinished = standing.unfinished;
        report.conflicted = standing.conflicted;
        for net in &candidates {
            report.connections += board.nets[*net].terminals.len() - 1;
            let points: Vec<[f64; 2]> = board.nets[*net]
                .terminals
                .iter()
                .map(|terminal| terminal.anchor)
                .collect();
            report.manhattan_mm += manhattan_tree(&points);
        }
    } else {
        let standing = router.probe_subset(&set, budget);
        report.open_terminals = standing.open_terminals;
        report.unfinished = standing.unfinished;
        report.conflicted = standing.conflicted;
        report.conflict_cells = standing.conflict_cells;
        report.iterations = standing.iterations;
        report.stop = standing.stop.to_string();
        let mut nets: Vec<KiCadTroubleNet> = standing
            .nets
            .iter()
            .map(|net| {
                let description = &board.nets[net.net as usize];
                let points: Vec<[f64; 2]> = description
                    .terminals
                    .iter()
                    .map(|terminal| terminal.anchor)
                    .collect();
                let manhattan_mm = manhattan_tree(&points);
                let work_seconds = net.expansions as f64 / rate;
                KiCadTroubleNet {
                    net: description.name.clone(),
                    open_terminals: net.open_terminals,
                    conflict_cells: net.conflict_cells,
                    complete: net.complete,
                    reroutes: net.reroutes,
                    manhattan_mm,
                    work_seconds,
                    work_ms_per_mm: 1000.0 * work_seconds / manhattan_mm.max(0.1),
                }
            })
            .collect();
        for (net, description) in nets.iter().zip(&standing.nets) {
            report.connections += board.nets[description.net as usize].terminals.len() - 1;
            report.manhattan_mm += net.manhattan_mm;
        }
        nets.sort_by(|a, b| {
            let trouble = |net: &KiCadTroubleNet| net.open_terminals > 0 || net.conflict_cells > 0;
            trouble(b)
                .cmp(&trouble(a))
                .then(b.open_terminals.cmp(&a.open_terminals))
                .then(b.conflict_cells.cmp(&a.conflict_cells))
                .then(b.work_ms_per_mm.total_cmp(&a.work_ms_per_mm))
                .then(a.net.cmp(&b.net))
        });
        report.new_trouble = nets;
    }
    let search = router.expansions().saturating_sub(before) as f64 / rate;
    report.search_work_seconds = search;
    report.work_per_connection = search / report.connections.max(1) as f64;
    report.work_ms_per_mm = 1000.0 * search / report.manhattan_mm.max(0.1);
    let corridors = KiCadCorridors::now();
    let negotiation = |kinds: &KiCadSearchKinds| {
        (
            kinds.corridor.searches,
            kinds.corridor.found,
            kinds.window.searches + kinds.board.searches,
        )
    };
    let (searches, found, wider) = negotiation(&corridors.negotiation);
    let (searches_before, found_before, wider_before) = negotiation(&corridors_before.negotiation);
    report.corridor_searches = searches - searches_before;
    report.corridor_misses = (searches - found).saturating_sub(searches_before - found_before);
    report.wider_searches = wider - wider_before;
    report.work_seconds =
        search + report.build_work_seconds + router.pour_work() + report.global_work_seconds;
    report.wall_seconds = started.elapsed().as_secs_f64();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::manhattan_tree;

    #[test]
    fn the_spanning_tree_joins_each_pad_to_the_nearest() {
        assert_eq!(manhattan_tree(&[[0.0, 0.0]]), 0.0);
        // An L of three pads: 3 + 4, not the 7 + 3 of a chain in order.
        assert_eq!(manhattan_tree(&[[0.0, 0.0], [3.0, 4.0], [3.0, 0.0]]), 7.0);
    }
}

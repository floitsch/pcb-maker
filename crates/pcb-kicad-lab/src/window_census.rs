// Copyright (C) 2026 Florian Loitsch.

//! `census-kicad-window`: the window census of a routed board
//! (experiments/window-census, docs/experiments/window-census.md). The
//! copper of every net with tracks or vias in a window is cut back to the
//! window's edge; the board without it is lowered as the open repairs lower
//! a routed board (`lower_for_local`: the copper outside kept, the pads it
//! joins one terminal, a piece on no pad a terminal of its own), and the
//! router's lattice of the window, grown by a margin, is written out
//! (`Router::window_census`) with, per net, the terminals that must end
//! joined: those the cut copper joined, and for the nets of the window's
//! open items the two sides of each item. A SAT model is built from it
//! offline; the cut board is where a solution's copper goes.

use crate::*;
use pcb_kicad_route::open_repair::lower_for_local;
use pcb_router as core;

/// What to census.
#[derive(Clone, Debug, Deserialize)]
pub struct KiCadWindowCensusConfig {
    /// The window (mm): x0, y0, x1, y1. Copper in it is rerouted.
    pub window: [f64; 4],
    /// How far beyond the window the lattice reaches (mm): the cut
    /// copper's ends lie on the window's edge, their nodes around it.
    #[serde(default = "default_margin")]
    pub margin: f64,
    /// The open items: the net (KiCad's name) and the two ends.
    #[serde(default)]
    pub items: Vec<KiCadWindowItem>,
    #[serde(default = "default_slack")]
    pub slack_steps: f64,
    #[serde(default = "default_ratio")]
    pub slack_ratio: f64,
    #[serde(default = "default_via_steps")]
    pub via_steps: f64,
    /// The budget of the nets that are not the items' (default: the
    /// items' budget).
    pub other_slack_steps: Option<f64>,
    pub other_slack_ratio: Option<f64>,
    /// Nets whose copper stays as it is although it lies in the window
    /// (differential pairs, which the census does not model as pairs, and
    /// nets the lattice cannot join again).
    #[serde(default)]
    pub fixed_nets: Vec<String>,
    /// Every track, arc and via of the board removed first (a diagnosis:
    /// whether a walled terminal is walled by copper or by the pads,
    /// graphics and rule areas alone).
    #[serde(default)]
    pub strip_all: bool,
    /// The most nodes the connections may span together (else the window
    /// is too big for the offline solve).
    #[serde(default = "default_max_nodes")]
    pub max_nodes: usize,
    /// A layer step allowed when either end's search may take it.
    #[serde(default)]
    pub either_end: bool,
    /// The lattice the routed board's attempt used (board-router.json).
    pub pitch: Option<f64>,
    pub pin_lines: Option<bool>,
}

fn default_margin() -> f64 {
    0.4
}
fn default_slack() -> f64 {
    20.0
}
fn default_ratio() -> f64 {
    0.5
}
fn default_via_steps() -> f64 {
    3.0
}
fn default_max_nodes() -> usize {
    150_000
}

#[derive(Clone, Debug, Deserialize)]
pub struct KiCadWindowItem {
    pub net: String,
    pub ends: Vec<[f64; 2]>,
}

/// A terminal, its label and its ways in: (node, escape or -1).
type TerminalOut = (usize, String, Vec<(usize, i64)>);

#[derive(Serialize)]
struct NetOut {
    name: String,
    raw: String,
    net_id: usize,
    class: usize,
    target: bool,
    /// How the item's sides were found: "ends", or "all" when an end lies
    /// on no copper of the net (every piece in the region joined).
    target_sides: String,
    via: [f64; 2],
    /// [layer, x mm, y mm] per node.
    nodes: Vec<(usize, f64, f64)>,
    widths: Vec<f64>,
    edges: Vec<(usize, usize, bool)>,
    vias: Vec<(f64, f64, Vec<usize>)>,
    /// [terminal, label, [[node, escape or -1], ...]].
    terminals: Vec<TerminalOut>,
    escapes: Vec<(usize, usize, f64, Vec<[f64; 2]>)>,
    requirements: Vec<Vec<usize>>,
    /// [a, b, distance, [nodes]].
    connections: Vec<(usize, usize, f64, Vec<usize>)>,
    separated: Vec<(usize, usize)>,
    /// [kind, index, [stamp keys], [read keys]].
    elements: Vec<(u8, usize, Vec<u32>, Vec<u32>)>,
    own_via_conflicts: Vec<(usize, usize)>,
    /// The cut copper's nodes and vias (phases, not constraints).
    hint_nodes: Vec<usize>,
    hint_vias: Vec<usize>,
}

#[derive(Serialize)]
struct CensusOut {
    board: String,
    clipped_board: String,
    window: [f64; 4],
    region: [f64; 4],
    pitch: f64,
    warped: bool,
    layers: Vec<String>,
    keys: usize,
    removed: [usize; 3],
    remnants: usize,
    kept_arcs: usize,
    /// Fixed nets with copper in the window, kept as they are.
    fixed_in_window: Vec<String>,
    nets: Vec<NetOut>,
    seconds: f64,
}

/// Liang-Barsky: the parameter interval of segment `a`-`b` inside `rect`.
fn clip(a: [f64; 2], b: [f64; 2], rect: [f64; 4]) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let d = [b[0] - a[0], b[1] - a[1]];
    for (p, q) in [
        (-d[0], a[0] - rect[0]),
        (d[0], rect[2] - a[0]),
        (-d[1], a[1] - rect[1]),
        (d[1], rect[3] - a[1]),
    ] {
        if p.abs() < 1.0e-12 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    (t0 < t1 - 1.0e-9).then_some((t0, t1))
}

fn inside(point: [f64; 2], rect: [f64; 4]) -> bool {
    point[0] >= rect[0] && point[0] <= rect[2] && point[1] >= rect[1] && point[1] <= rect[3]
}

fn number(value: f64) -> Expr {
    Expr::Atom(
        format!("{:.6}", value)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string(),
    )
}

/// `item` with its start, end and uuid replaced.
fn remnant(item: &Expr, start: [f64; 2], end: [f64; 2], tag: &str) -> Expr {
    let parts = item
        .children()
        .iter()
        .map(|part| match part.head() {
            Some("start") => Expr::List(vec![
                Expr::Atom("start".into()),
                number(start[0]),
                number(start[1]),
            ]),
            Some("end") => Expr::List(vec![
                Expr::Atom("end".into()),
                number(end[0]),
                number(end[1]),
            ]),
            Some("uuid") => {
                let old = part.children().get(1).and_then(Expr::atom).unwrap_or("");
                Expr::List(vec![
                    Expr::Atom("uuid".into()),
                    Expr::Atom(quote(&deterministic_uuid_for(&format!("{old}#{tag}")))),
                ])
            }
            _ => part.clone(),
        })
        .collect();
    Expr::List(parts)
}

/// The shapes of each terminal of `net` (its pad or contact, the pads
/// merged into it, the copper it reaches) with their layers.
fn terminal_shapes(board: &core::Board, net: usize) -> Vec<Vec<(core::Shape, u64)>> {
    let description = &board.nets[net];
    let mut shapes: Vec<Vec<(core::Shape, u64)>> = description
        .terminals
        .iter()
        .map(|terminal| {
            let pad = &board.obstacles[terminal.pad];
            vec![(pad.shape.clone(), pad.layers as u64)]
        })
        .collect();
    for merged in board
        .merged_pads
        .iter()
        .filter(|merged| merged.net as usize == net)
    {
        if let Some(list) = shapes.get_mut(merged.host) {
            let pad = &board.obstacles[merged.pad];
            list.push((pad.shape.clone(), pad.layers as u64));
        }
    }
    for extra in board
        .terminal_copper
        .iter()
        .filter(|extra| extra.net as usize == net)
    {
        if let Some(list) = shapes.get_mut(extra.terminal) {
            let copper = &board.obstacles[extra.obstacle];
            list.push((copper.shape.clone(), copper.layers as u64));
        }
    }
    shapes
}

/// The terminal of `shapes` whose copper holds `point` (on `layers`).
fn holder(
    shapes: &[Vec<(core::Shape, u64)>],
    point: [f64; 2],
    layers: u64,
    tolerance: f64,
) -> Option<usize> {
    let mut best: Option<(f64, usize)> = None;
    for (terminal, list) in shapes.iter().enumerate() {
        for (shape, on) in list {
            if on & layers == 0 {
                continue;
            }
            let distance = shape.distance_to_point(point);
            if distance <= tolerance && best.is_none_or(|(least, _)| distance < least) {
                best = Some((distance, terminal));
            }
        }
    }
    best.map(|(_, terminal)| terminal)
}

pub fn census_kicad_window(
    board_path: &Path,
    config: &KiCadWindowCensusConfig,
    clipped_path: &Path,
    output: &Path,
) -> Result<(), String> {
    let started = Instant::now();
    let text = fs::read_to_string(board_path)
        .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let mut pcb = crate::canonical_order::parse_board(&text)?;
    if config.strip_all
        && let Expr::List(items) = &mut pcb
    {
        items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    }
    let pcb = pcb;
    let mut rules =
        project_rules::resolve_project_rules(&board_path.with_extension("kicad_pro"), board_path)?;
    if let Some(pitch) = config.pitch {
        rules.tuning.pitches = Some(vec![pitch]);
    }
    if config.pin_lines.is_some() {
        rules.pin_lines = config.pin_lines;
    }
    let names = NetNames::of(&pcb);
    let window = config.window;
    let targets: BTreeSet<String> = config.items.iter().map(|item| item.net.clone()).collect();
    // The window's nets: tracks or vias in the window, and the items'.
    let mut window_nets: BTreeSet<String> = targets.clone();
    let mut removed = [0usize; 3];
    let mut fixed_in_window: BTreeSet<String> = BTreeSet::new();
    for item in pcb.children() {
        let Some(net) = node_net(item) else {
            continue;
        };
        let hit = match item.head() {
            Some("segment") => {
                let (Ok(a), Ok(b)) = (form_xy(item, "start"), form_xy(item, "end")) else {
                    continue;
                };
                clip(a, b, window).is_some()
            }
            Some("via") => form_xy(item, "at").is_ok_and(|at| inside(at, window)),
            _ => false,
        };
        if hit && !net.is_empty() {
            if config.fixed_nets.iter().any(|fixed| fixed == net) {
                fixed_in_window.insert(net.to_string());
            } else {
                window_nets.insert(net.to_string());
            }
        }
    }
    // The board with their copper cut back to the window's edge.
    let mut clipped_items = Vec::new();
    let (mut remnants, mut kept_arcs) = (0, 0);
    // The cut copper, for the solver's phases: (net, layer, start, end)
    // and (net, via centre).
    let mut cut_tracks: Vec<(String, String, [f64; 2], [f64; 2])> = Vec::new();
    let mut cut_vias: Vec<(String, [f64; 2])> = Vec::new();
    for item in pcb.children() {
        let net = node_net(item).unwrap_or("");
        if !window_nets.contains(net) {
            clipped_items.push(item.clone());
            continue;
        }
        match item.head() {
            Some("segment") => {
                let (a, b) = (form_xy(item, "start")?, form_xy(item, "end")?);
                let Some((t0, t1)) = clip(a, b, window) else {
                    clipped_items.push(item.clone());
                    continue;
                };
                removed[0] += 1;
                let at = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                if let Some(layer) = form_atom(item, "layer", 1) {
                    cut_tracks.push((net.to_string(), layer.to_string(), at(t0), at(t1)));
                }
                let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                if t0 * length > 1.0e-3 {
                    clipped_items.push(remnant(item, a, at(t0), "a"));
                    remnants += 1;
                }
                if (1.0 - t1) * length > 1.0e-3 {
                    clipped_items.push(remnant(item, at(t1), b, "b"));
                    remnants += 1;
                }
            }
            Some("via") if form_xy(item, "at").is_ok_and(|at| inside(at, window)) => {
                removed[1] += 1;
                cut_vias.push((net.to_string(), form_xy(item, "at")?));
            }
            Some("arc") => {
                let points: Vec<[f64; 2]> = ["start", "mid", "end"]
                    .iter()
                    .filter_map(|head| form_xy(item, head).ok())
                    .collect();
                if points.iter().all(|point| inside(*point, window)) {
                    removed[2] += 1;
                } else {
                    if points.iter().any(|point| inside(*point, window)) {
                        kept_arcs += 1;
                    }
                    clipped_items.push(item.clone());
                }
            }
            _ => clipped_items.push(item.clone()),
        }
    }
    let clipped = Expr::List(clipped_items);
    fs::write(clipped_path, encode(&clipped))
        .map_err(|error| format!("failed to write {}: {error}", clipped_path.display()))?;
    let (rules_c, board_c) = lower_for_local(&clipped, rules.clone())?;
    let (_, board_o) = lower_for_local(&pcb, rules)?;
    let mut core_config = crate::board_layout::race_core_config(&rules_c);
    core_config.verbose = false;
    let router = core::router::Router::try_new(&board_c, &core_config)?;
    let grid = router.grid().clone();
    let region = core::geometry::Aabb {
        minimum: [window[0] - config.margin, window[1] - config.margin],
        maximum: [window[2] + config.margin, window[3] + config.margin],
    };
    let region_rect = [
        region.minimum[0],
        region.minimum[1],
        region.maximum[0],
        region.maximum[1],
    ];
    let mut requests = Vec::new();
    let mut extra: Vec<(String, String, bool, String)> = Vec::new();
    for raw in &window_nets {
        let name = names.normalize(raw);
        let Some(net_c) = board_c.nets.iter().position(|net| net.name == name) else {
            continue;
        };
        let Some(net_o) = board_o.nets.iter().position(|net| net.name == name) else {
            continue;
        };
        let shapes_o = terminal_shapes(&board_o, net_o);
        let shapes_c = terminal_shapes(&board_c, net_c);
        // The clipped terminals near the region, by original piece.
        let mut pieces: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let mut fresh = shapes_o.len();
        for (terminal, list) in shapes_c.iter().enumerate() {
            let near = list
                .iter()
                .any(|(shape, _)| shape.aabb().intersects(region));
            if !near {
                continue;
            }
            let description = &board_c.nets[net_c].terminals[terminal];
            let piece = holder(
                &shapes_o,
                description.anchor,
                description.layers as u64,
                1.0e-4,
            )
            .unwrap_or_else(|| {
                fresh += 1;
                fresh
            });
            pieces.entry(piece).or_default().push(terminal);
        }
        let target = targets.contains(raw);
        let mut sides = String::new();
        if target {
            sides = "ends".into();
            for item in config.items.iter().filter(|item| &item.net == raw) {
                let ends: Vec<Option<usize>> = item
                    .ends
                    .iter()
                    .map(|end| holder(&shapes_o, *end, u64::MAX, 0.05))
                    .collect();
                let found: Vec<usize> = ends.iter().flatten().copied().collect();
                let joinable = found.len() == 2
                    && found[0] != found[1]
                    && found.iter().all(|piece| pieces.contains_key(piece));
                if joinable {
                    let moved = pieces.remove(&found[1]).unwrap_or_default();
                    pieces.entry(found[0]).or_default().extend(moved);
                } else {
                    sides = "all".into();
                }
            }
            if sides == "all" {
                let all: Vec<usize> = pieces.values().flatten().copied().collect();
                pieces = BTreeMap::from([(0, all)]);
            }
        }
        let requirements: Vec<Vec<usize>> = pieces
            .into_values()
            .filter(|terminals| terminals.len() >= 2)
            .collect();
        if requirements.is_empty() {
            continue;
        }
        let limits = if target {
            core::router::CensusLimits {
                slack_steps: config.slack_steps,
                slack_ratio: config.slack_ratio,
                via_steps: config.via_steps,
                either_end: config.either_end,
            }
        } else {
            core::router::CensusLimits {
                slack_steps: config.other_slack_steps.unwrap_or(config.slack_steps),
                slack_ratio: config.other_slack_ratio.unwrap_or(config.slack_ratio),
                via_steps: config.via_steps,
                either_end: config.either_end,
            }
        };
        requests.push(core::router::CensusRequest {
            net: net_c as core::NetId,
            requirements,
            limits,
        });
        extra.push((name.to_string(), raw.clone(), target, sides));
    }
    let model = router.window_census(region, &requests, config.max_nodes)?;
    let layers = board_router::LayerTable::from_pcb(&pcb)?.names;
    // The cut copper snapped to the lattice: each net's nodes and vias
    // under it (phases for the solver; not a constraint).
    let hints = |census: &core::router::CensusNet, raw: &str| -> (Vec<usize>, Vec<usize>) {
        let index: BTreeMap<(usize, usize, usize), usize> = census
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (*node, index))
            .collect();
        let mut nodes = BTreeSet::new();
        for (net, layer, a, b) in cut_tracks.iter().filter(|track| track.0 == raw) {
            let _ = net;
            let Some(layer) = layers.iter().position(|name| name == layer) else {
                continue;
            };
            let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let steps = ((length / (grid.pitch / 4.0)).ceil() as usize).max(1);
            for step in 0..=steps {
                let t = step as f64 / steps as f64;
                let point = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                if let Some(cell) = grid.nearest_node(point) {
                    let (x, y) = grid.xy(cell);
                    if let Some(node) = index.get(&(layer, x, y)) {
                        nodes.insert(*node);
                    }
                }
            }
        }
        let mut vias = BTreeSet::new();
        for (_, at) in cut_vias.iter().filter(|via| via.0 == raw) {
            if let Some(cell) = grid.nearest_node(*at)
                && let Some(via) = census.vias.iter().position(|(here, _)| *here == cell)
            {
                vias.insert(via);
                nodes.extend(census.vias[via].1.iter().copied());
            }
        }
        (nodes.into_iter().collect(), vias.into_iter().collect())
    };
    let mut nets = Vec::new();
    for ((census, request), (name, raw, target, sides)) in
        model.nets.into_iter().zip(&requests).zip(extra)
    {
        let (hint_nodes, hint_vias) = hints(&census, &raw);
        let center = |x: usize, y: usize| grid.center(x, y);
        let terminals = census
            .terminals
            .iter()
            .map(|(terminal, access)| {
                (
                    *terminal,
                    board_c.nets[census.net as usize].terminals[*terminal]
                        .label
                        .clone(),
                    access
                        .iter()
                        .map(|way| (way.node, way.escape.map_or(-1, |escape| escape as i64)))
                        .collect(),
                )
            })
            .collect();
        nets.push(NetOut {
            name,
            raw,
            net_id: census.net as usize,
            class: census.class,
            target,
            target_sides: sides,
            via: census.via,
            nodes: census
                .nodes
                .iter()
                .map(|&(layer, x, y)| {
                    let at = center(x, y);
                    (layer, at[0], at[1])
                })
                .collect(),
            widths: census.widths,
            edges: census.edges,
            vias: census
                .vias
                .iter()
                .map(|(cell, here)| {
                    let at = grid.center_of(*cell);
                    (at[0], at[1], here.clone())
                })
                .collect(),
            terminals,
            escapes: census
                .escapes
                .iter()
                .map(|escape| {
                    (
                        escape.terminal,
                        escape.node,
                        escape.width,
                        escape.polyline.clone(),
                    )
                })
                .collect(),
            requirements: request.requirements.clone(),
            connections: census
                .connections
                .into_iter()
                .map(|connection| {
                    (
                        connection.a,
                        connection.b,
                        connection.distance,
                        connection.nodes,
                    )
                })
                .collect(),
            separated: census.separated,
            elements: census
                .elements
                .into_iter()
                .map(|element| (element.kind, element.index, element.stamps, element.reads))
                .collect(),
            own_via_conflicts: census.own_via_conflicts,
            hint_nodes,
            hint_vias,
        });
    }
    let out = CensusOut {
        board: board_path.display().to_string(),
        clipped_board: clipped_path.display().to_string(),
        window,
        region: region_rect,
        pitch: model.pitch,
        warped: model.warped,
        layers,
        keys: model.keys,
        removed,
        remnants,
        kept_arcs,
        fixed_in_window: fixed_in_window.into_iter().collect(),
        nets,
        seconds: started.elapsed().as_secs_f64(),
    };
    let file = fs::File::create(output)
        .map_err(|error| format!("failed to write {}: {error}", output.display()))?;
    serde_json::to_writer(std::io::BufWriter::new(file), &out).map_err(|error| error.to_string())
}

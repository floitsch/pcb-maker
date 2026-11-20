// Copyright (C) 2026 Florian Loitsch.

//! Add one separately routed path between two already-connected pads.

use super::*;
use pcb_router as core;
use std::time::Duration;

const SEARCH_SECONDS: u64 = 20;
const CHECK_SECONDS: u64 = 60;
const MAX_CELLS: f64 = 1_000_000.0;
const MAX_SEGMENTS: usize = 128;
const NM: f64 = 0.000_001;

#[derive(Serialize)]
pub struct ParallelFeed {
    pub status: &'static str,
    pub reason: String,
    pub pads: [String; 2],
    pub net: Option<String>,
    pub layer: String,
    pub width_mm: f64,
    pub added_segments: usize,
    pub added_length_mm: f64,
    pub pitch_mm: Option<f64>,
    pub expanded_nodes: u64,
    pub search_stop: Option<&'static str>,
    pub native_checks_directory: Option<String>,
    pub native_checks: usize,
    pub native_added_finding_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_publication: Option<ParallelFeedFill>,
    pub predecessor_report: Option<String>,
}

#[derive(Serialize)]
pub struct ParallelFeedFill {
    pub status: &'static str,
    pub reason: Option<String>,
    pub native_checks: usize,
    pub filled_zones: usize,
    pub filled_polygons: usize,
}

/// Writes verified stored-fill data and its native report to a private
/// candidate. The assembly layer supplies the existing fill handoff.
pub type ParallelFeedPublisher =
    fn(&Path, &Value, &Path, Instant) -> Result<ParallelFeedFill, String>;

/// Add a separate straight-segment feed on one selected layer. The board
/// under test supplies all obstacles, including retained copper of the net.
pub fn add_kicad_parallel_feed(
    project: &Path,
    board_id: &str,
    output: &Path,
    pads: [&str; 2],
    layer: &str,
    width: f64,
    publish_fills: ParallelFeedPublisher,
) -> Result<ParallelFeed, String> {
    check_board_id(board_id).map_err(exit::input_error)?;
    if !width.is_finite() || width < NM || (width - round_mm(width)).abs() > 1e-12 {
        return Err(exit::input_error(
            "width must be a positive number of whole nanometres",
        ));
    }
    if !pads.iter().all(|pad| {
        pad.split_once(':')
            .is_some_and(|(reference, pin)| !reference.is_empty() && !pin.is_empty())
    }) {
        return Err(exit::input_error("select each pad as REF:PAD"));
    }
    if !project.join(format!("{board_id}.kicad_sch")).is_file() {
        return Err(exit::input_error(
            "a parallel feed requires the current KiCad schematic",
        ));
    }
    in_repair_copy(project, output, |copy| {
        let mut report = ParallelFeed {
            status: "unchanged",
            reason: String::new(),
            pads: pads.map(str::to_owned),
            net: None,
            layer: layer.to_owned(),
            width_mm: width,
            added_segments: 0,
            added_length_mm: 0.0,
            pitch_mm: None,
            expanded_nodes: 0,
            search_stop: None,
            native_checks_directory: None,
            native_checks: 0,
            native_added_finding_types: Vec::new(),
            fill_publication: None,
            predecessor_report: None,
        };
        match try_feed(copy, board_id, &mut report, publish_fills) {
            Ok(()) => {
                report.reason = if report.fill_publication.is_some() {
                    "KiCad accepted the feed after refill and verified the reopened stored fill"
                } else {
                    "stored and refilled KiCad checks accepted the additive feed"
                }
                .into();
                report.status = "added";
            }
            Err(Rejection::Unchanged(reason)) => report.reason = reason,
            Err(Rejection::Error(error)) => return Err(error),
        }
        let manifest = copy.join("parallel-feed.json");
        if manifest.is_file() {
            let directory = reserve_directory(copy, "parallel-feed-predecessor")?;
            let previous = format!("{directory}/parallel-feed.json");
            fs::rename(&manifest, copy.join(&previous)).map_err(|error| error.to_string())?;
            report.predecessor_report = Some(previous);
        }
        write_typed_json(&manifest, &report)?;
        Ok(report)
    })
}

enum Rejection {
    Unchanged(String),
    Error(String),
}

impl From<String> for Rejection {
    fn from(error: String) -> Self {
        Self::Error(error)
    }
}

fn unchanged(reason: &str) -> Rejection {
    Rejection::Unchanged(reason.into())
}

fn round_mm(value: f64) -> f64 {
    (value * 1e6).round() / 1e6
}

fn try_feed(
    project: &Path,
    board_id: &str,
    report: &mut ParallelFeed,
    publish_fills: ParallelFeedPublisher,
) -> Result<(), Rejection> {
    let file = project.join(format!("{board_id}.kicad_pcb"));
    let original = fs::read_to_string(&file).map_err(|error| error.to_string())?;
    let pcb = parse(&original)?;
    let codes = net_codes(&pcb);
    let [from_net, to_net] = report
        .pads
        .each_ref()
        .map(|pad| planner::pad_net(&pcb, &codes, pad));
    let Some(net) =
        from_net.filter(|name| to_net.as_ref() == Some(name) && report.pads[0] != report.pads[1])
    else {
        return Err(unchanged(
            "choose two distinct unique pads on the same named net",
        ));
    };
    report.net = Some(net.clone());
    let layers = LayerTable::from_pcb(&pcb)?;
    let layer = layers
        .index(&report.layer)
        .ok_or_else(|| unchanged("the requested copper layer does not exist on this board"))?;
    let bit = 1 << layer;
    let mut exact_pads = Vec::with_capacity(2);
    for pad in &report.pads {
        let (footprint, form) = planner::find_pad(&pcb, pad).expect("validated unique pad");
        if !matches!(
            form.children().get(2).and_then(Expr::atom),
            Some("smd" | "thru_hole")
        ) || !matches!(
            form.children().get(3).and_then(Expr::atom),
            Some("rect" | "circle" | "oval" | "roundrect")
        ) || form.child("padstack").is_some()
            || form_atom(form, "remove_unused_layers", 1) == Some("yes")
            || form
                .child("drill")
                .is_some_and(|drill| drill.child("offset").is_some())
            || layers.pad_mask(form)? & bit == 0
        {
            return Err(unchanged(
                "both endpoints must expose supported pad copper on the requested layer",
            ));
        }
        exact_pads.push(lower_pad(form, form_at(footprint)?)?);
    }
    let names = note_net_names(&pcb);
    let normalized = names.normalize(&net);
    if has_unmodeled_zone_contact(&pcb, &codes, &layers, bit, normalized, &names)? {
        return Err(unchanged(
            "a same-net or unassigned copper zone on the selected layer prevents proving a separate feed",
        ));
    }
    if has_teardrops(&pcb, &codes, &layers, bit, &net, &file)? {
        return Err(unchanged(
            "enabled teardrops on the selected net or project track junctions prevent proving a separate feed",
        ));
    }
    let mut rules = resolve_project_rules(&file.with_extension("kicad_pro"), &file)?;
    if rules.diff_pair_rules.contains_key(normalized)
        || rules
            .length_rules
            .iter()
            .any(|rule| rule.nets.iter().any(|net| net.net == normalized))
    {
        return Err(unchanged(
            "the net has a differential or length requirement",
        ));
    }
    rules.via_in_pad = Some(board_router::KiCadViaInPad::All(false));
    rules.bga_fanout = Some(false);
    rules.tuning.sub_board_nets = Some(false);
    let mut board = board_router::lower(&pcb, &rules, false)?.board;
    let mut net_rules = rules.net_rules.as_deref().cloned().unwrap_or_default();
    let labels = report.pads.each_ref().map(|pad| pad.replacen(':', ".", 1));
    let mut matching = board.nets.iter().enumerate().filter_map(|(i, net)| {
        if net.name != normalized {
            return None;
        }
        let [from, to] = [0, 1].map(|index| {
            let pad = &exact_pads[index];
            let shape = pad.geometry.router_shape();
            let mut matching = net.terminals.iter().filter(|terminal| {
                terminal.label == labels[index]
                    && terminal.layers & bit != 0
                    && terminal.anchor == pad.center
                    && board.obstacles[terminal.pad].shape == shape
            });
            matching
                .next()
                .cloned()
                .filter(|_| matching.next().is_none())
        });
        Some((i, [from?, to?]))
    });
    let Some((focus, terminals)) = matching.next().filter(|_| matching.next().is_none()) else {
        return Err(unchanged(
            "the selected pad pair has no common supported router-layer access",
        ));
    };
    let retained = focus_layer(
        &mut board,
        &mut net_rules,
        layer,
        focus,
        terminals.clone(),
        report.width_mm,
    )?;
    let segments = search_path(
        &board,
        net_rules,
        focus,
        terminals.each_ref().map(|terminal| terminal.anchor),
        &retained,
        report,
    )?;
    let net_codes: Vec<_> = codes
        .iter()
        .filter(|(_, name)| *name == &net)
        .map(|(code, _)| code)
        .collect();
    let legacy_code = match net_codes.as_slice() {
        [] if codes.is_empty() => None,
        [code] => Some(*code),
        _ => return Err(unchanged("the source has no unique net code for this feed")),
    };
    let mut additions = Vec::new();
    let mut ids = BTreeSet::new();
    for segment in &segments {
        let segment = SupplementalSegment {
            connection: net.clone(),
            start: segment.start,
            end: segment.end,
            width: report.width_mm,
            layer: report.layer.clone(),
        };
        let id = supplemental_segment_id(&segment);
        if !ids.insert(id.clone()) || original.contains(&id) {
            return Err(unchanged("a generated track identity is already in use"));
        }
        let mut object = supplemental_segment_expr(&segment);
        if let (Some(code), Expr::List(fields)) = (legacy_code, &mut object) {
            for field in fields
                .iter_mut()
                .filter(|field| field.head() == Some("net"))
            {
                *field = Expr::List(vec![Expr::Atom("net".into()), Expr::Atom(code.clone())]);
            }
        }
        additions.push(object);
    }
    accept_feed(&file, &original, &pcb, &additions, report, publish_fills)?;
    report.added_segments = segments.len();
    report.added_length_mm = segments
        .iter()
        .map(|track| core::geometry::distance(track.start, track.end))
        .sum();
    Ok(())
}

fn has_unmodeled_zone_contact(
    pcb: &Expr,
    codes: &BTreeMap<String, String>,
    layers: &LayerTable,
    bit: u32,
    net: &str,
    names: &NetNames,
) -> Result<bool, String> {
    let footprint_zones = pcb
        .children()
        .iter()
        .filter(|form| matches!(form.head(), Some("footprint" | "module")))
        .flat_map(Expr::children);
    for zone in pcb
        .children()
        .iter()
        .chain(footprint_zones)
        .filter(|form| form.head() == Some("zone") && !is_rule_area(form))
    {
        let owner = item_net(zone, codes).or_else(|| form_atom(zone, "net_name", 1));
        if layers.mask_of_item(zone)? & bit != 0
            && owner.is_none_or(|owner| owner.is_empty() || names.normalize(owner) == net)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn has_teardrops(
    pcb: &Expr,
    codes: &BTreeMap<String, String>,
    layers: &LayerTable,
    bit: u32,
    net: &str,
    file: &Path,
) -> Result<bool, String> {
    let enabled = |object: &Expr| {
        item_net(object, codes) == Some(net)
            && object.child("teardrops").is_some_and(|form| {
                form_atom(form, "enabled", 1) == Some("yes")
                    || form
                        .children()
                        .iter()
                        .skip(1)
                        .any(|flag| flag.atom() == Some("enabled"))
            })
    };
    for object in pcb.children() {
        if object.head() == Some("via") && enabled(object) && layers.via_mask(object) & bit != 0 {
            return Ok(true);
        }
        if matches!(object.head(), Some("footprint" | "module")) {
            for pad in object
                .children()
                .iter()
                .filter(|item| item.head() == Some("pad"))
            {
                if enabled(pad) && layers.pad_mask(pad)? & bit != 0 {
                    return Ok(true);
                }
            }
        }
    }
    let project = file.with_extension("kicad_pro");
    if !project.is_file() {
        return Ok(false);
    }
    let project: Value = read_typed_json(&project)?;
    Ok(project["board"]["design_settings"]["teardrop_options"]
        .as_array()
        .is_some_and(|options| options.iter().any(|option| option["td_ontrackend"] == true)))
}

/// Keep unrelated copper as static obstacles, including copper of the
/// selected net. Reindex all layer masks for a one-layer search: via moves
/// and shortcuts through the existing network are unavailable.
fn focus_layer(
    board: &mut core::Board,
    rules: &mut core::rules::NetRules,
    layer: usize,
    focus: usize,
    terminals: [core::Terminal; 2],
    width: f64,
) -> Result<Vec<core::Shape>, Rejection> {
    let bit = 1 << layer;
    let conn = &board.nets[focus];
    let name = conn.name.clone();
    let mut class = board.classes[conn.class];
    if class.layers != 0 && class.layers & bit == 0 {
        return Err(unchanged(
            "the net class does not permit the selected layer",
        ));
    }
    let bound = rules
        .layer_widths
        .get(&name)
        .and_then(|bounds| bounds.get(layer))
        .copied()
        .unwrap_or(core::rules::LayerWidth::ANY);
    let floor = rules
        .width_floor
        .get(&name)
        .copied()
        .unwrap_or(board.neck_width);
    if width < bound.min.max(floor) || width > bound.max {
        return Err(unchanged(
            "the requested width is outside the project width limits for this net and layer",
        ));
    }
    if board
        .planes
        .iter()
        .any(|plane| plane.layer == layer && plane.exclusive && plane.net as usize != focus)
    {
        return Err(unchanged(
            "the requested layer is reserved for another net's plane",
        ));
    }
    for bounds in rules.layer_widths.values_mut() {
        *bounds = vec![
            bounds
                .get(layer)
                .copied()
                .unwrap_or(core::rules::LayerWidth::ANY),
        ];
    }
    rules.layer_widths.insert(
        name.clone(),
        vec![core::rules::LayerWidth {
            opt: Some(width),
            ..bound
        }],
    );
    rules.width_floor.insert(name, width);
    rules.clearances.retain_mut(|rule| {
        let enabled = rule.layers == 0 || rule.layers & bit != 0;
        rule.layers = 0;
        enabled
    });
    class.layers = 1;
    class.trace_width = width;
    board.nets[focus].class = board.classes.len();
    board.classes.push(class);
    board.layer_count = 1;
    board.neck_width = width;
    board.planes.clear();
    board.via_spans.clear();
    board.terminal_copper.clear();
    board.merged_pads.clear();
    board.pin_row_gaps.clear();
    for conn in &mut board.nets {
        conn.terminals.clear();
    }
    let endpoint_shapes = terminals
        .each_ref()
        .map(|terminal| board.obstacles[terminal.pad].shape.clone());
    let mut independent = Vec::new();
    for (index, obstacle) in board.obstacles.iter_mut().enumerate() {
        obstacle.layers = u32::from(obstacle.layers & bit != 0);
        if obstacle.layers == 0 || obstacle.net != Some(focus as core::NetId) {
            continue;
        }
        if terminals.iter().any(|terminal| {
            terminal.pad == index
                || obstacle.kind == core::ObstacleKind::Hole
                    && obstacle.label == format!("{} plated hole", terminal.label)
        }) {
            continue;
        }
        if obstacle.kind == core::ObstacleKind::Copper {
            let shape = match obstacle.label.as_str() {
                "existing track" | "existing via" => {
                    outside_pads(&obstacle.shape, &endpoint_shapes)
                }
                _ => Some(obstacle.shape.clone()),
            };
            match shape {
                Some(shape) => {
                    independent.push(shape.clone());
                    obstacle.shape = shape;
                }
                None => {
                    obstacle.layers = 0;
                    continue;
                }
            }
        }
        obstacle.net = None;
    }
    board.nets[focus].terminals = terminals
        .into_iter()
        .map(|mut terminal| {
            terminal.layers = 1;
            terminal
        })
        .collect();
    Ok(independent)
}

/// A capsule is the sweep of its end discs. Trim only their sweep that lies
/// strictly inside a convex endpoint pad. The original copper remains in the
/// board. The untrimmed remainder independently blocks any external tap.
fn outside_pads(shape: &core::Shape, pads: &[core::Shape; 2]) -> Option<core::Shape> {
    let inset = 0.01;
    match *shape {
        core::Shape::Circle { center, radius } => (!pads
            .iter()
            .any(|pad| pad.contains_disc(center, radius + inset)))
        .then(|| shape.clone()),
        core::Shape::Capsule {
            mut start,
            mut end,
            radius,
        } => {
            for pad in pads {
                if !matches!(
                    pad,
                    core::Shape::Circle { .. }
                        | core::Shape::Capsule { .. }
                        | core::Shape::Polygon { .. }
                ) {
                    continue;
                }
                let inside = |point| pad.contains_disc(point, radius + inset);
                if inside(start) && inside(end) {
                    return None;
                }
                start = inside_prefix(start, end, inside);
                end = inside_prefix(end, start, inside);
            }
            Some(core::Shape::Capsule { start, end, radius })
        }
        _ => Some(shape.clone()),
    }
}

fn inside_prefix(
    start: core::Point,
    end: core::Point,
    inside: impl Fn(core::Point) -> bool,
) -> core::Point {
    if !inside(start) {
        return start;
    }
    let mut best = start;
    let mut limit = end;
    for _ in 0..24 {
        let halfway = [(best[0] + limit[0]) * 0.5, (best[1] + limit[1]) * 0.5];
        if inside(halfway) {
            best = halfway;
        } else {
            limit = halfway;
        }
    }
    best
}

fn search_path(
    board: &core::Board,
    rules: core::rules::NetRules,
    focus: usize,
    terminals: [core::Point; 2],
    retained: &[core::Shape],
    report: &mut ParallelFeed,
) -> Result<Vec<core::Segment>, Rejection> {
    let size = [0.075, 0.10, 0.127, 0.20]
        .into_iter()
        .map(|pitch| core::grid::LatticeSize::of(board, pitch))
        .find(|size| size.check().is_ok() && size.cells() <= MAX_CELLS)
        .ok_or_else(|| unchanged("the selected layer exceeds the bounded search lattice"))?;
    report.pitch_mm = Some(size.pitch);
    let maps_mb = size.maps_bytes_floor(board.layer_count, board.classes.len()) / 1_048_576.0;
    if board_router::MemoryLimits::read(None)
        .too_small_for(maps_mb, 1)
        .is_some()
    {
        return Err(unchanged(
            "the selected search lattice exceeds available router memory",
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(SEARCH_SECONDS);
    let config = core::router::Config {
        pitches: vec![size.pitch],
        bend_cost: 0.05,
        against_direction: 1.0,
        neck_reach: 0.0,
        max_iterations: 1,
        cleanup_passes: 0,
        bend_round: false,
        via_reduction_rounds: 0,
        trouble_repair: false,
        lost_spoke_repair: false,
        parallel: false,
        pin_lines: core::pin_lines::Lines::Off,
        share_static_maps: false,
        deadline: Some(deadline),
        deadline_work: Some(8.0),
        limits_count_all: true,
        net_rules: Some(std::sync::Arc::new(rules)),
        ..core::router::Config::default()
    };
    let result = core::router::route(board, &config);
    report.expanded_nodes = result.expansions;
    let mut route = result.routes.into_iter().nth(focus).expect("routed net");
    if Instant::now() >= deadline
        || matches!(
            result.negotiation_stop,
            "work_deadline" | "wall_guard" | "past_wall_deadline"
        )
    {
        report.search_stop = Some("work_limit");
        return Err(unchanged("the feed search reached its work limit"));
    }
    if result.status[focus] != core::router::NetStatus::Routed || route.segments.is_empty() {
        report.search_stop = Some("no_path");
        return Err(unchanged(
            "no bounded, via-free path was found at the requested width",
        ));
    }
    if !route.vias.is_empty() || route.segments.len() > MAX_SEGMENTS {
        report.search_stop = Some("unsupported_path");
        return Err(unchanged("a candidate exceeds the via-free segment limit"));
    }
    for segment in &mut route.segments {
        segment.start = segment.start.map(round_mm);
        segment.end = segment.end.map(round_mm);
    }
    if !separate_path(&route.segments, terminals, report.width_mm, retained) {
        report.search_stop = Some("unproven_path");
        return Err(unchanged(
            "the route could not prove a continuous pad-to-pad feed without an external tap",
        ));
    }
    report.search_stop = Some("path_found");
    Ok(route.segments)
}

fn separate_path(
    segments: &[core::Segment],
    terminals: [core::Point; 2],
    width: f64,
    retained: &[core::Shape],
) -> bool {
    let joint = |point: core::Point| point.map(|coordinate| (coordinate * 1e6).round() as i64);
    let [start, end] = terminals.map(joint);
    if start == end {
        return false;
    }
    let mut links = BTreeMap::<_, Vec<_>>::new();
    for segment in segments {
        let a = joint(segment.start);
        let b = joint(segment.end);
        let capsule = core::Shape::Capsule {
            start: segment.start,
            end: segment.end,
            radius: width * 0.5,
        };
        if a == b
            || segment.layer != 0
            || (segment.width - width).abs() > NM * 0.1
            || retained
                .iter()
                .any(|shape| core::verify::shape_distance(&capsule, shape) <= NM)
        {
            return false;
        }
        links.entry(a).or_default().push(b);
        links.entry(b).or_default().push(a);
    }
    let mut current = start;
    let mut previous = None;
    for _ in 0..segments.len() {
        let Some(next) = links.remove(&current) else {
            return false;
        };
        let expected = if previous.is_some() { 2 } else { 1 };
        if next.len() != expected {
            return false;
        }
        let Some(follow) = next.into_iter().find(|joint| Some(*joint) != previous) else {
            return false;
        };
        previous = Some(current);
        current = follow;
    }
    current == end && links.len() == 1 && links[&end] == [previous.expect("nonempty path")]
}

fn accept_feed(
    file: &Path,
    original: &str,
    pcb: &Expr,
    additions: &[Expr],
    report: &mut ParallelFeed,
    publish_fills: ParallelFeedPublisher,
) -> Result<(), Rejection> {
    let project = file.parent().expect("project file");
    let directory = reserve_directory(project, "parallel-feed-checks")?;
    let reports = project.join(&directory);
    report.native_checks_directory = Some(directory);
    let deadline = Instant::now() + Duration::from_secs(CHECK_SECONDS);
    let check = |label: &str, refill| {
        native_report::check_kept_route_input(
            file,
            &reports.join(format!("{label}.json")),
            true,
            refill,
            || Instant::now() >= deadline,
        )
        .map_err(|reason| Rejection::Unchanged(at_native_check(&reports, label, &reason)))
    };
    let mut baseline = Vec::new();
    for (i, mode) in MODES.iter().enumerate() {
        let before = check(&format!("before-{mode}"), i != 0)?;
        report.native_checks += 1;
        check_baseline(&reports, mode, &before).map_err(Rejection::Unchanged)?;
        baseline.push(before);
    }
    crate::open_repair::write_additions(file, original, additions)?;
    let trial = fs::read_to_string(file).map_err(|error| error.to_string())?;
    let decision = (|| {
        let mut needs_fill = false;
        let mut checked_refill = None;
        for (i, (mode, before)) in MODES.iter().zip(&baseline).enumerate() {
            let trial_name = format!("trial-0-{mode}");
            let after = check(&trial_name, i != 0)?;
            report.native_checks += 1;
            report.native_added_finding_types = violation_types(before, &after);
            if let Err(error) = compare_trial(before, &after) {
                if i == 0 && only_stale_fill_clearance(pcb, additions, before, &after)? {
                    needs_fill = true;
                } else {
                    return Err(Rejection::Unchanged(at_native_check(
                        &reports,
                        &trial_name,
                        &error,
                    )));
                }
            }
            if i != 0 {
                checked_refill = Some(after);
            }
        }
        if fs::read_to_string(file).map_err(|error| error.to_string())? != trial {
            return Err(Rejection::Error(
                "native check modified the trial board".into(),
            ));
        }
        if !needs_fill {
            return Ok(None);
        }
        let path = reports.join("trial-0-published-stored.json");
        let fill = publish_fills(
            file,
            &checked_refill.expect("checked the refill"),
            &path,
            deadline,
        )?;
        let status = fill.status;
        let reason = fill.reason.clone();
        report.native_checks += fill.native_checks;
        report.fill_publication = Some(fill);
        if status != "saved" {
            return Err(Rejection::Unchanged(reason.unwrap_or_else(|| {
                "could not verify the reopened filled board".into()
            })));
        }
        Ok(Some(path))
    })();
    match decision {
        Ok(saved) => record_kept_checks(project, &reports, 0, saved.as_deref())?,
        Err(error) => {
            fs::write(file, original).map_err(|error| error.to_string())?;
            return Err(error);
        }
    }
    Ok(())
}

/// A candidate may invalidate a cached polygon while keeping its authored
/// zone. Only violations naming that zone and newly added copper qualify.
pub(super) fn only_stale_fill_clearance(
    pcb: &Expr,
    additions: &[Expr],
    before: &Value,
    after: &Value,
) -> Result<bool, String> {
    if check_complete(after).is_err()
        || parity(before) != parity(after)
        || !crate::open_repair::same_check_policy(before, after)
    {
        return Ok(false);
    }
    use crate::open_repair::item_id;
    let zones: BTreeSet<_> = copper_zone_definitions(pcb)
        .filter(|zone| zone.child("filled_polygon").is_some())
        .filter_map(item_id)
        .collect();
    let added: BTreeMap<_, _> = additions
        .iter()
        .filter_map(|item| Some((item_id(item)?, item.head()?)))
        .collect();
    let counts = finding_counts(before, "violations")?;
    let mut found = false;
    for (key, count) in finding_counts(after, "violations")? {
        if counts.get(&key).is_some_and(|previous| *previous >= count) {
            continue;
        }
        let finding: Value = serde_json::from_str(&key).map_err(|error| error.to_string())?;
        let kind = finding["type"].as_str();
        if !matches!(kind, Some("clearance" | "hole_clearance")) || finding["excluded"] == true {
            return Ok(false);
        }
        let Some([a, b]) = finding["items"].as_array().map(Vec::as_slice) else {
            return Ok(false);
        };
        let ids = [a, b].map(|item| item["uuid"].as_str().map(str::to_ascii_lowercase));
        let [Some(a), Some(b)] = ids else {
            return Ok(false);
        };
        if ![(&a, &b), (&b, &a)].into_iter().any(|(zone, item)| {
            zones.contains(zone)
                && added
                    .get(item)
                    .is_some_and(|shape| kind == Some("clearance") || *shape == "via")
        }) {
            return Ok(false);
        }
        found = true;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refill_fallback_requires_exact_new_track_and_saved_zone_witnesses() {
        let board = parse(
            r#"(kicad_pcb
                (zone (layer "F.Cu") (uuid zone) (filled_polygon (layer "F.Cu")))
                (zone (layer "F.Cu") (uuid unfilled)))"#,
        )
        .unwrap();
        let added = [parse("(segment (uuid new-track))").unwrap()];
        let report = |findings| {
            serde_json::json!({
                "violations": findings,
                "schematic_parity": [],
                "unconnected_items": [],
            })
        };
        let violation = |kind, a, b| {
            serde_json::json!({
                "type": kind,
                "items": [{"uuid": a}, {"uuid": b}],
            })
        };
        let before = report(vec![]);
        let valid = violation("clearance", "zone", "new-track");
        let apply =
            |after| only_stale_fill_clearance(&board, &added, &before, &report(after)).unwrap();
        assert!(apply(vec![valid.clone()]));
        assert!(!apply(vec![violation("clearance", "zone", "old-track")]));
        assert!(!apply(vec![violation(
            "hole_clearance",
            "zone",
            "new-track"
        )]));
        let via = [parse("(via (uuid new-via))").unwrap()];
        assert!(
            only_stale_fill_clearance(
                &board,
                &via,
                &before,
                &report(vec![violation("hole_clearance", "zone", "new-via")]),
            )
            .unwrap()
        );
        assert!(
            !only_stale_fill_clearance(
                &board,
                &via,
                &before,
                &report(vec![violation("hole_clearance", "old-pad", "new-via")]),
            )
            .unwrap()
        );
        assert!(!apply(vec![violation(
            "clearance",
            "unfilled",
            "new-track"
        )]));
        assert!(!apply(vec![
            valid,
            violation("starved_thermal", "pad", "zone")
        ]));
    }

    #[test]
    fn old_track_geometry_remains_a_blocker_after_its_pad_prefix_is_trimmed() {
        let pads = [
            core::Shape::Circle {
                center: [0.0, 0.0],
                radius: 1.0,
            },
            core::Shape::Circle {
                center: [8.0, 0.0],
                radius: 1.0,
            },
        ];
        let original = core::Shape::Capsule {
            start: [0.0, 0.0],
            end: [8.0, 0.0],
            radius: 0.25,
        };
        let Some(core::Shape::Capsule { start, end, radius }) = outside_pads(&original, &pads)
        else {
            panic!("the old track outside both pads must remain a blocker")
        };
        assert!(pads[0].contains_disc(start, radius));
        assert!(pads[1].contains_disc(end, radius));
        assert!((0.73..0.75).contains(&start[0]));
        assert!((7.25..7.27).contains(&end[0]));
    }

    #[test]
    fn the_new_chain_must_reach_both_pad_centres_without_contacting_old_copper() {
        let segment = |start, end| core::Segment {
            start,
            end,
            layer: 0,
            width: 0.5,
        };
        let anchors = [[1.0, 3.0], [3.0, 5.0]];
        let bend = [3.0, 3.0];
        let good = [segment(anchors[0], bend), segment(bend, anchors[1])];
        assert!(separate_path(&good, anchors, 0.5, &[]));
        let tap = core::Shape::Circle {
            center: [2.0, 3.45],
            radius: 0.25,
        };
        assert!(!separate_path(&good, anchors, 0.5, &[tap]));
        let disconnected = [good[0].clone(), segment([2.95, 3.0], anchors[1])];
        assert!(!separate_path(&disconnected, anchors, 0.5, &[]));
        let fork = [good[0].clone(), good[1].clone(), segment(bend, [4.0, 3.0])];
        assert!(!separate_path(&fork, anchors, 0.5, &[]));
    }
}

// Copyright (C) 2026 Toit contributors.

//! One routing attempt: its run, its board written and checked.

use super::*;
use pcb_router as core;

/// Writes a routing result as copper into `pcb` and reports per net.
/// `keep_existing`: the board's own tracks and vias stay as they are
/// (route mode, `kept`); otherwise they are replaced.
pub(crate) fn emit_routes(
    pcb: &mut Expr,
    board: &core::Board,
    result: &core::RoutingResult,
    layer_names: &[String],
    keep_existing: bool,
) -> Result<Vec<KiCadBoardRouterNet>, String> {
    let copper_net_names = unambiguous_pad_net_names(pcb)?;
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    if !keep_existing {
        items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    }
    // The vias the via-in-pad rule put in pads, fixed copper of their net
    // that the routes join: written with them, filled and capped.
    let mut pad_vias = vec![0usize; board.nets.len()];
    for (net, at, diameter, drill) in pad_vias_of(board) {
        pad_vias[net as usize] += 1;
        items.push(pad_via_expr(&SupplementalVia {
            connection: board.nets[net as usize].name.clone(),
            at: nanometres(at),
            size: diameter,
            drill,
            layers: ["F.Cu".into(), "B.Cu".into()],
            kind: None,
        }));
    }
    // The prepared fan-outs (`bga_fanout`) the routes use: stub and via
    // written as the net's copper; one not used is dropped.
    for fanout in fanouts_of(board) {
        let net = fanout.net as usize;
        if !result
            .routes
            .get(net)
            .is_some_and(|route| fanout_used(board, &fanout, route))
        {
            continue;
        }
        pad_vias[net] += 1;
        items.push(supplemental_segment_expr(&SupplementalSegment {
            connection: board.nets[net].name.clone(),
            start: nanometres(fanout.start),
            end: nanometres(fanout.end),
            width: fanout.width,
            layer: layer_names[fanout.layer].clone(),
        }));
        let (layers, kind) = via_layers_and_kind(fanout.span, board.layer_count, layer_names);
        items.push(supplemental_via_expr(&SupplementalVia {
            connection: board.nets[net].name.clone(),
            at: nanometres(fanout.end),
            size: fanout.diameter,
            drill: fanout.drill,
            layers,
            kind,
        }));
    }
    let mut nets = Vec::new();
    for (net, route) in result.routes.iter().enumerate() {
        let description = &board.nets[net];
        let dropped = graphic_stubs(board, net, route);
        for (_, segment) in route
            .segments
            .iter()
            .enumerate()
            .filter(|(index, _)| !dropped.contains(index))
        {
            items.push(supplemental_segment_expr(&SupplementalSegment {
                connection: description.name.clone(),
                start: nanometres(segment.start),
                end: nanometres(segment.end),
                width: segment.width,
                layer: layer_names[segment.layer].clone(),
            }));
        }
        for via in &route.vias {
            // A via of a span (blind, buried, micro) on its layers.
            let (layers, kind) = via_layers_and_kind(via.span, board.layer_count, layer_names);
            items.push(supplemental_via_expr(&SupplementalVia {
                connection: description.name.clone(),
                at: nanometres(via.at),
                size: via.diameter,
                drill: via.drill,
                layers,
                kind,
            }));
        }
        let (status, unconnected_terminals) = match result.status[net] {
            core::NetStatus::Trivial => ("trivial", 0),
            core::NetStatus::Routed => ("routed", 0),
            core::NetStatus::Partial {
                unconnected_terminals,
            } => ("partial", unconnected_terminals),
            core::NetStatus::Unreachable => {
                ("unreachable", description.terminals.len().saturating_sub(1))
            }
        };
        nets.push(KiCadBoardRouterNet {
            connection: description.name.clone(),
            terminals: description.terminals.len(),
            status: status.into(),
            unconnected_terminals,
            vias: route.vias.len() + pad_vias[net],
            length_mm: route
                .segments
                .iter()
                .map(|segment| distance_squared(segment.start, segment.end).sqrt())
                .sum(),
        });
    }
    canonicalize_copper_net_names(pcb, &copper_net_names);
    Ok(nets)
}

/// A via's `layers` and kind token as KiCad writes them: `F.Cu B.Cu` and
/// none for a through via; a span's first and last layers and `micro`,
/// `blind` (an outer layer to an inner one) or `buried` (inner layers).
pub(crate) fn via_layers_and_kind(
    span: Option<core::ViaSpan>,
    layers: usize,
    layer_names: &[String],
) -> ([String; 2], Option<String>) {
    match span {
        None => (["F.Cu".into(), "B.Cu".into()], None),
        Some(span) => {
            let kind = match span.kind(layers) {
                core::ViaKind::Through => None,
                core::ViaKind::Micro => Some("micro"),
                core::ViaKind::Blind => Some("blind"),
                core::ViaKind::Buried => Some("buried"),
            };
            (
                [
                    layer_names[span.top].clone(),
                    layer_names[span.bottom].clone(),
                ],
                kind.map(String::from),
            )
        }
    }
}

/// The router's stubs into a copper graphic of their net that KiCad's
/// dangling check flags: a track from the graphic's anchor (the terminal
/// the search reached) to a node inside it, nothing else at the anchor.
/// KiCad 10 passes over tracks inside a pad (`track_dangling` ignores
/// them) but not inside a graphic: such a track is copper of the graphic
/// that ends on nothing (kicad-cli 10.0.7 on WLED_Matrix's
/// `Net-(AE1-A)` rectangle, 2026-10-27: the stub flagged, the track that
/// reaches the node from outside not). The graphic stays joined through
/// the node's own track, which ends inside it. Indices into the route's
/// segments.
pub(crate) fn graphic_stubs(board: &core::Board, net: usize, route: &core::NetRoute) -> Vec<usize> {
    let mut dropped = Vec::new();
    let near = |a: core::Point, b: core::Point| distance_squared(a, b) < 1.0e-12;
    for terminal in &board.nets[net].terminals {
        let Some(graphic) = board.obstacles.get(terminal.pad) else {
            continue;
        };
        if graphic.net != Some(net as core::NetId) || !graphic.label.starts_with("copper ") {
            continue;
        }
        let anchor = terminal.anchor;
        for (index, segment) in route.segments.iter().enumerate() {
            if graphic.layers & (1 << segment.layer) == 0 {
                continue;
            }
            let other = if near(segment.start, anchor) {
                segment.end
            } else if near(segment.end, anchor) {
                segment.start
            } else {
                continue;
            };
            if !graphic.shape.contains(anchor) || !graphic.shape.contains(other) {
                continue;
            }
            let alone = route.vias.iter().all(|via| !near(via.at, anchor))
                && route
                    .segments
                    .iter()
                    .enumerate()
                    .all(|(other_index, track)| {
                        other_index == index
                            || track.layer != segment.layer
                            || (!near(track.start, anchor) && !near(track.end, anchor))
                    });
            if alone {
                dropped.push(index);
            }
        }
    }
    dropped
}

/// A fresh numbered subdirectory of `root` for one routing attempt's frames.
fn attempt_frame_directory(root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("failed to create {}: {error}", root.display()))?;
    let attempts = fs::read_dir(root)
        .map_err(|error| format!("failed to read {}: {error}", root.display()))?
        .count();
    let directory = root.join(format!("attempt-{attempts:02}"));
    fs::create_dir_all(&directory)
        .map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
    Ok(directory)
}

pub(super) fn route_kicad_board_once(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
) -> Result<KiCadBoardRouterResult, String> {
    route_kicad_board_lowered(
        source_directory,
        board_id,
        output_directory,
        config,
        connect_pours,
        None,
    )
}

/// `route_kicad_board_once`, on `lowered` (the board as `lower` gives it
/// for `config` and `connect_pours`, with its lowering's work) when the
/// caller lowered it already: the ladder lowers a rung's board for its
/// hold and routes that board (one lowering an attempt).
pub(super) fn route_kicad_board_lowered(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
    lowered: Option<(core::Board, f64)>,
) -> Result<KiCadBoardRouterResult, String> {
    let started = std::time::Instant::now();
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let pcb = crate::canonical_order::parse_board(&source)?;
    // The board's own tracks and vias: what they join is connected (a
    // board the ladder lowered had them kept there: the runner's `kept`
    // reports them).
    let (board, lowering_work, kept) = match lowered {
        Some((board, work)) => (board, work, KiCadKeptCopper::default()),
        None => {
            let Lowered {
                mut board,
                expressions,
                ..
            } = lower(&pcb, config, connect_pours)?;
            let kept = keep_existing_copper(&mut board, false);
            if config.diff_pairs_on() && config.engine.as_deref() != Some("topological") {
                plan_pairs(&mut board, config);
            }
            (board, lowering_work_of(expressions), kept)
        }
    };
    let layer_names = LayerTable::from_pcb(&pcb)?.names;
    let lowering_seconds = started.elapsed().as_secs_f64();
    let routing_started = std::time::Instant::now();
    // The router's lattice maps, for sizing routers side by side.
    let mut maps_mb = 0.0;
    // A board the residue completed by the router's count: KiCad's check
    // before its polish, where the attempt is checked (`confirm_residue`).
    let check = (config.residue_check || !config.skip_native_verification).then(|| ResidueCheck {
        pcb: &pcb,
        board: &board,
        layer_names: &layer_names,
        source_directory,
        board_id,
        directory: residue_check_directory(output_directory),
        config,
    });
    let result = match &config.frame_directory {
        _ if config.engine.as_deref() == Some("topological") => {
            let mut topological = pcb_topo::Config {
                iterations: config.topological_rounds.unwrap_or(30),
                seed: config.first_seed.unwrap_or(1).max(1),
                verbose: true,
                ..pcb_topo::Config::default()
            };
            config.topological.apply(&mut topological);
            pcb_topo::route(&board, &topological)
        }
        None if config.seeds.unwrap_or(1) > 1 || config.first_seed.unwrap_or(0) > 0 => {
            let (result, maps) = route_seeds(
                &board,
                &core_config(config),
                config.first_seed.unwrap_or(0),
                config.seeds.unwrap_or(1),
                config.seeds_side_by_side.unwrap_or(usize::MAX),
                check.as_ref(),
            );
            maps_mb = maps;
            result
        }
        None => {
            let (result, maps) = route_measured(&board, &core_config(config), check.as_ref());
            maps_mb = maps;
            result
        }
        Some(root) => {
            let directory = attempt_frame_directory(root)?;
            let mut router = core::router::Router::new(&board, &core_config(config));
            let frame_pcb = pcb.clone();
            let frame_board = board.clone();
            let frame_layers = layer_names.clone();
            router.set_frame_hook(std::sync::Arc::new(move |iteration, snapshot| {
                let mut frame = frame_pcb.clone();
                let written = emit_routes(&mut frame, &frame_board, snapshot, &frame_layers, true)
                    .and_then(|_| {
                        let path = directory.join(format!("frame-{iteration:04}.kicad_pcb"));
                        fs::write(&path, format!("{}\n", encode(&frame)))
                            .map_err(|error| format!("failed to write {}: {error}", path.display()))
                    });
                if let Err(error) = written {
                    eprintln!("frame {iteration}: {error}");
                }
            }));
            router.run()
        }
    };
    drop(check);
    let routing_seconds = routing_started.elapsed().as_secs_f64();
    let mut written = write_attempt(
        pcb,
        &board,
        result,
        &layer_names,
        source_directory,
        board_id,
        output_directory,
        config,
        [lowering_seconds, routing_seconds],
    )?;
    written.maps_mb = maps_mb;
    written.lowering_work_seconds = lowering_work;
    if !kept.nets.is_empty() {
        written.kept_copper = Some(kept);
    }
    Ok(written)
}

/// Routes `board` as `core::route` does (`Router::new` and `run`), with
/// the router's lattice maps in megabytes (its memory report; 0 when the
/// report does not say). A board the residue completed by the router's
/// count is polished only as `confirm_residue` says (`check`).
pub(super) fn route_measured(
    board: &core::Board,
    config: &core::Config,
    check: Option<&ResidueCheck>,
) -> (core::RoutingResult, f64) {
    let mut router = core::router::Router::new(board, config);
    let maps_mb = maps_mb_of(&router.memory_report()).unwrap_or(0.0);
    let routed = router.run_in_place();
    (confirm_residue(&mut router, routed, check), maps_mb)
}

/// Where a residue check of the attempt in `output` writes its board:
/// beside it (the attempt's directory is written new after the check),
/// removed after the check.
pub(super) fn residue_check_directory(output: &Path) -> PathBuf {
    let mut name = output.file_name().unwrap_or_default().to_os_string();
    name.push(".residue-check");
    output.with_file_name(name)
}

/// KiCad's checks of boards the residue completed (`confirm_residue`), to
/// be charged as the ladder's checks (`Ladder::judge`).
pub(super) static RESIDUE_CHECKS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// What becomes of a board the router's residue completed by its own
/// count (router `Diagnostics::residue_polish_deferred`): the router's
/// count is optimistic (the pours' stitching at the end can leave pads
/// cut off, and KiCad sees what the router's model does not), so the
/// board is judged as the ladder judges an attempt (KiCad's check where
/// the ladder checks) before the polish runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResidueVerdict {
    /// KiCad finds nothing open: the polish runs.
    Confirmed,
    /// The router's own count at the end of its run (after the pours'
    /// stitching) has terminals open: no polish.
    OpenByRouter(usize),
    /// KiCad finds items open: no polish.
    OpenByKiCad(usize),
    /// No KiCad check: complete by the router's count only, which can be
    /// optimistic; no polish.
    Unchecked,
}

/// The verdict on a board the residue completed, with the router's open
/// terminals at the end of its run and KiCad's open count (the ladder's
/// measure; None: not checked).
pub(crate) fn residue_verdict(router_open: usize, kicad_open: Option<usize>) -> ResidueVerdict {
    if router_open > 0 {
        return ResidueVerdict::OpenByRouter(router_open);
    }
    match kicad_open {
        None => ResidueVerdict::Unchecked,
        Some(0) => ResidueVerdict::Confirmed,
        Some(open) => ResidueVerdict::OpenByKiCad(open),
    }
}

/// The open terminals of a routing by the router's own count (its nets'
/// status; route_seeds ranks by it with the copper check on top).
pub(super) fn routed_open(result: &core::RoutingResult) -> usize {
    result
        .status
        .iter()
        .map(|status| match status {
            core::NetStatus::Partial {
                unconnected_terminals,
            } => *unconnected_terminals,
            core::NetStatus::Unreachable => 1,
            _ => 0,
        })
        .sum()
}

/// How a board the residue completed is checked: written as the attempt
/// writes it into `directory` and checked by KiCad as the ladder checks
/// an attempt.
#[derive(Clone)]
pub(super) struct ResidueCheck<'a> {
    pub pcb: &'a Expr,
    pub board: &'a core::Board,
    pub layer_names: &'a [String],
    pub source_directory: &'a Path,
    pub board_id: &'a str,
    pub directory: PathBuf,
    pub config: &'a KiCadBoardRouterConfig,
}

impl ResidueCheck<'_> {
    /// The same check in a directory of its own beside this one's
    /// (`name` appended).
    pub(super) fn at(&self, name: &str) -> Self {
        let mut check = self.clone();
        let mut directory = self.directory.clone().into_os_string();
        directory.push(format!("-{name}"));
        check.directory = PathBuf::from(directory);
        check
    }

    /// The ladder's open count of `routed` (the router's unconnected
    /// terminals and internal violations and KiCad's unconnected items),
    /// its board written and checked in `directory`, which is removed
    /// after.
    fn open(&self, routed: &core::RoutingResult) -> Result<usize, String> {
        let mut config = self.config.clone();
        config.skip_native_verification = true;
        if self.directory.exists() {
            fs::remove_dir_all(&self.directory).map_err(|error| error.to_string())?;
        }
        // The board is written into a new directory, its parent there.
        if let Some(parent) = self.directory.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        let checked = write_attempt(
            self.pcb.clone(),
            self.board,
            routed.clone(),
            self.layer_names,
            self.source_directory,
            self.board_id,
            &self.directory,
            &config,
            [0.0, 0.0],
        )
        .and_then(|mut written| {
            check_attempt(&mut written, &self.directory, self.board_id)?;
            Ok(router_open(&written)
                + written
                    .native
                    .as_ref()
                    .map_or(0, |native| native.selected_net_unconnected_items))
        });
        let _ = fs::remove_dir_all(&self.directory);
        checked
    }
}

/// A board the residue completed by the router's count
/// (`Diagnostics::residue_polish_deferred`) is polished (`Router::polish`)
/// only when `check` confirms it complete (`residue_verdict`); otherwise
/// `routed` stays as it is, unpolished. Any other routing passes as it is.
pub(super) fn confirm_residue(
    router: &mut core::router::Router,
    routed: core::RoutingResult,
    check: Option<&ResidueCheck>,
) -> core::RoutingResult {
    if !routed.diagnostics.residue_polish_deferred {
        return routed;
    }
    let router_open = routed_open(&routed);
    let kicad_open = match check.filter(|_| router_open == 0) {
        Some(check) => {
            RESIDUE_CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match check.open(&routed) {
                Ok(open) => Some(open),
                Err(error) => {
                    eprintln!("residue: KiCad's check of the board failed: {error}");
                    None
                }
            }
        }
        None => None,
    };
    let verdict = residue_verdict(router_open, kicad_open);
    eprintln!(
        "residue: the board it completed by the router's count {}",
        match verdict {
            ResidueVerdict::Confirmed => "is complete by KiCad's check too: polished".to_string(),
            ResidueVerdict::OpenByRouter(open) => format!(
                "has {open} open terminals by the router's count after the pours' stitching: not polished"
            ),
            ResidueVerdict::OpenByKiCad(open) =>
                format!("has {open} open by KiCad's check: not polished"),
            ResidueVerdict::Unchecked => "is not checked by KiCad (the router's count only, which can be optimistic): not polished".to_string(),
        }
    );
    if verdict == ResidueVerdict::Confirmed {
        router.polish()
    } else {
        routed
    }
}

/// Moves every `(layer "X")` that names a layer the board's layer table
/// does not define to `Cmts.User`. Some boards carry such items (a
/// `Rescue` layer from an old KiCad, a user layer removed later); KiCad's
/// editor asks what to do with them and kicad-cli refuses the board.
/// Returns how many were moved.
pub(super) fn move_undefined_layers(pcb: &mut Expr) -> usize {
    let defined: std::collections::HashSet<String> = pcb
        .child("layers")
        .map(|layers| {
            layers
                .children()
                .iter()
                .skip(1)
                .flat_map(|entry| {
                    entry
                        .children()
                        .iter()
                        .skip(1)
                        .take(3)
                        .filter_map(Expr::atom)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    if defined.is_empty() {
        return 0;
    }
    fn walk(node: &mut Expr, defined: &std::collections::HashSet<String>, top: bool) -> usize {
        let Expr::List(items) = node else {
            return 0;
        };
        // The layer table itself and the stackup name layers by design.
        if !top
            && matches!(
                items.first().and_then(Expr::atom),
                Some("layers" | "stackup")
            )
        {
            return 0;
        }
        let mut moved = 0;
        if items.len() == 2 && items[0].atom() == Some("layer") {
            if let Some(name) = items[1].atom()
                && !defined.contains(name)
                && !name.contains('*')
                && !name.contains('&')
            {
                items[1] = Expr::Atom("\"Cmts.User\"".into());
                moved += 1;
            }
            return moved;
        }
        for item in items.iter_mut() {
            moved += walk(item, defined, false);
        }
        moved
    }
    walk(pcb, &defined, true)
}

/// Writes a routed board: the optional tightening, the copper into the
/// board file, and the checks (`finish_routed_board`).
#[allow(clippy::too_many_arguments)]
pub(super) fn write_attempt(
    mut pcb: Expr,
    board: &core::Board,
    result: core::RoutingResult,
    layer_names: &[String],
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
    seconds: [f64; 2],
) -> Result<KiCadBoardRouterResult, String> {
    crate::mem_profile::phase("write-back");
    let [lowering_seconds, mut routing_seconds] = seconds;
    // Read before the items on undefined layers move away.
    let warnings = input_warnings(&pcb);
    let moved = move_undefined_layers(&mut pcb);
    if moved > 0 {
        eprintln!(
            "{moved} items on layers the board does not define moved to Cmts.User (KiCad would not load the board)"
        );
    }
    let mut result = result;
    if config.tighten && config.engine.as_deref() != Some("topological") {
        let tightening = std::time::Instant::now();
        let (routes, report) = pcb_topo::tighten::tighten(board, &result.routes);
        eprintln!(
            "tighten: {} of {} pieces pulled tight, {:.1} mm -> {:.1} mm, {:.2}s",
            report.tightened,
            report.pieces,
            report.length_before,
            report.length_after,
            tightening.elapsed().as_secs_f64()
        );
        result.routes = routes;
        routing_seconds += tightening.elapsed().as_secs_f64();
    }
    // Differential pairs: the links' routes into the pairs' copper (written
    // first, so that the nets' names are made canonical with the rest), the
    // board as lowered.
    let heights = copper_heights(&pcb);
    let heights: Vec<f64> = layer_names
        .iter()
        .map(|name| heights.get(name).copied().unwrap_or(0.0))
        .collect();
    let paired = finish_pairs(board, &mut result, config, &heights);
    let board = paired.as_ref().map_or(board, |(restored, _, _)| restored);
    if let Some((_, copper, _)) = &paired {
        write_pair_copper(&mut pcb, copper, layer_names)?;
    }
    // The length and skew rules: meanders on the routes of the nets that
    // lack length (the pairs' coupled nets are the pairs' own).
    let coupled: BTreeSet<String> = paired
        .iter()
        .flat_map(|(_, _, pairs)| pairs.pairs.iter())
        .filter(|pair| pair.links_coupled > 0)
        .flat_map(|pair| [pair.positive.clone(), pair.negative.clone()])
        .collect();
    let (patterns, lengths) =
        match_lengths(board, &mut result, config, &heights, layer_names, &coupled);
    let nets = emit_routes(&mut pcb, board, &result, layer_names, true)?;
    write_length_patterns(&mut pcb, board, &patterns, layer_names)?;
    let mut report = finish_routed_board(
        &pcb,
        board,
        &result,
        nets,
        layer_names,
        source_directory,
        board_id,
        output_directory,
        config,
        [lowering_seconds, routing_seconds],
    )?;
    report.warnings = warnings;
    report.diff_pairs = paired.map(|(_, _, pairs)| pairs);
    report.lengths = lengths;
    Ok(report)
}

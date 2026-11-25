// Copyright (C) 2026 Toit contributors.

//! One routing attempt: its run, its board written and checked.

use super::*;
use pcb_router as core;

mod retained_segments;
use self::retained_segments::RetainedSegments;

/// Writes a routing result as copper into `pcb` and reports per net.
/// `keep_existing`: the board's own tracks and vias stay as they are
/// (route mode, `kept`); otherwise they are replaced.
pub fn emit_routes(
    pcb: &mut Expr,
    board: &core::Board,
    result: &core::RoutingResult,
    layer_names: &[String],
    keep_existing: bool,
) -> Result<Vec<KiCadBoardRouterNet>, String> {
    emit_routes_preserving_tuning(
        pcb,
        board,
        result,
        layer_names,
        keep_existing,
        &BTreeSet::new(),
        &BTreeSet::new(),
    )
}

fn emit_routes_preserving_tuning(
    pcb: &mut Expr,
    board: &core::Board,
    result: &core::RoutingResult,
    layer_names: &[String],
    keep_existing: bool,
    tuned: &BTreeSet<&str>,
    native_closed_planes: &BTreeSet<String>,
) -> Result<Vec<KiCadBoardRouterNet>, String> {
    let copper_net_names = unambiguous_pad_net_names(pcb)?;
    let mut retained = if keep_existing {
        RetainedSegments::of(pcb)
    } else {
        RetainedSegments::default()
    };
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    if !keep_existing {
        items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    }
    let reusable_net = |connection: &str| {
        if tuned.contains(connection) {
            None
        } else {
            copper_net_names.get(connection).and_then(Expr::atom)
        }
    };
    // The vias the via-in-pad rule put in pads, fixed copper of their net
    // that the routes join: written with them, filled and capped.
    let mut pad_vias = vec![0usize; board.nets.len()];
    for (net, at, diameter, drill) in pad_vias_of(board) {
        if native_closed_planes.contains(&board.nets[net as usize].name) {
            continue;
        }
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
        if native_closed_planes.contains(&board.nets[net].name) {
            continue;
        }
        if !result
            .routes
            .get(net)
            .is_some_and(|route| fanout_used(board, &fanout, route))
        {
            continue;
        }
        pad_vias[net] += 1;
        let connection = &board.nets[net].name;
        retained.emit(
            items,
            reusable_net(connection),
            SupplementalSegment {
                connection: connection.clone(),
                start: nanometres(fanout.start),
                end: nanometres(fanout.end),
                width: fanout.width,
                layer: layer_names[fanout.layer].clone(),
            },
        );
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
    // The names the router keeps several nets of (a panel's copies).
    let copies = copy_indices(
        &board
            .nets
            .iter()
            .take(result.routes.len())
            .map(|net| (!native_closed_planes.contains(&net.name)).then_some(net.name.as_str()))
            .collect::<Vec<_>>(),
    );
    for (net, route) in result.routes.iter().enumerate() {
        let description = &board.nets[net];
        if native_closed_planes.contains(&description.name) {
            continue;
        }
        let native_net = reusable_net(&description.name);
        let dropped = graphic_stubs(board, net, route);
        let mut covered_length = 0.0;
        for (_, segment) in route
            .segments
            .iter()
            .enumerate()
            .filter(|(index, _)| !dropped.contains(index))
        {
            covered_length += retained.emit(
                items,
                native_net,
                SupplementalSegment {
                    connection: description.name.clone(),
                    start: nanometres(segment.start),
                    end: nanometres(segment.end),
                    width: segment.width,
                    layer: layer_names[segment.layer].clone(),
                },
            );
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
        let open = matches!(status, "partial" | "unreachable");
        let copy = copies[net];
        nets.push(KiCadBoardRouterNet {
            open: if open {
                open_terminals_of(
                    &result.diagnostics,
                    net as core::NetId,
                    unconnected_terminals,
                )
            } else {
                Vec::new()
            },
            repair: open
                .then(|| {
                    net_repair_of(
                        &result.diagnostics,
                        &board.nets,
                        board.layer_count,
                        net as core::NetId,
                        layer_names,
                    )
                })
                .flatten(),
            open_repair: None,
            copy,
            pads: if copy.is_some() {
                description
                    .terminals
                    .iter()
                    .map(|terminal| terminal.label.clone())
                    .collect()
            } else {
                Vec::new()
            },
            connection: description.name.clone(),
            terminals: description.terminals.len(),
            status: status.into(),
            unconnected_terminals,
            vias: route.vias.len() + pad_vias[net],
            length_mm: (route
                .segments
                .iter()
                .map(|segment| distance_squared(segment.start, segment.end).sqrt())
                .sum::<f64>()
                - covered_length)
                .max(0.0),
        });
    }
    canonicalize_copper_net_names(pcb, &copper_net_names);
    Ok(nets)
}

/// Per net of `names` (None: a net not reported), where several reported
/// nets share its name (a panel's copies the lowering split,
/// `outline_pieces::split_router_nets`), its index among them in net
/// order; None for a name of one net.
pub(crate) fn copy_indices(names: &[Option<&str>]) -> Vec<Option<usize>> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for name in names.iter().flatten() {
        *counts.entry(name).or_default() += 1;
    }
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    names
        .iter()
        .map(|name| {
            let name = (*name)?;
            (counts[name] > 1).then(|| {
                let index = seen.entry(name).or_default();
                *index += 1;
                *index - 1
            })
        })
        .collect()
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
            let kept = super::kept::keep_for_route(&mut board, &pcb, config);
            super::native_kept::protect_planes(&mut board, config);
            if config.diff_pairs_on() && config.tuning.engine.as_deref() != Some("topological") {
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
    let result = match &config.tuning.frame_directory {
        _ if config.tuning.engine.as_deref() == Some("topological") => {
            let mut topological = pcb_topo::Config {
                iterations: config.tuning.topological_rounds.unwrap_or(30),
                seed: config.tuning.first_seed.unwrap_or(1).max(1),
                verbose: true,
                ..pcb_topo::Config::default()
            };
            config.tuning.topological.apply(&mut topological);
            pcb_topo::route(&board, &topological)
        }
        None if config.seeds.unwrap_or(1) > 1 || config.tuning.first_seed.unwrap_or(0) > 0 => {
            let (result, maps) = route_seeds(
                &board,
                &core_config(config),
                config.tuning.first_seed.unwrap_or(0),
                config.seeds.unwrap_or(1),
                config.seeds_side_by_side.unwrap_or(usize::MAX),
            );
            maps_mb = maps;
            result
        }
        None => {
            let (result, maps) = route_measured(&board, &core_config(config));
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
    if !kept.nets.is_empty() || kept.retained_pad_tips.is_some() {
        written.kept_copper = Some(kept);
    }
    Ok(written)
}

/// Routes `board` as `core::route` does (`Router::new` and `run`), with
/// the router's lattice maps in megabytes (its memory report; 0 when the
/// report does not say).
pub(super) fn route_measured(
    board: &core::Board,
    config: &core::Config,
) -> (core::RoutingResult, f64) {
    let router = core::router::Router::new(board, config);
    let maps_mb = maps_mb_of(&router.memory_report()).unwrap_or(0.0);
    (router.run(), maps_mb)
}

/// Moves items on layers KiCad cannot resolve to `Cmts.User`. The usual
/// case is a `Rescue` layer from an old KiCad. Declared names and KiCad's
/// implicit numbered user layers are retained. Returns how many moved.
pub(super) fn move_undefined_layers(pcb: &mut Expr) -> usize {
    let mut defined: std::collections::HashSet<String> = pcb
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
    // KiCad 10 also resolves these canonical names when the layer table
    // omits them, including in footprints copied from a library.
    defined.extend((1..=45).map(|index| format!("User.{index}")));
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
    if !config.route_reuse_preserve_input_layers {
        let moved = move_undefined_layers(&mut pcb);
        if moved > 0 {
            eprintln!(
                "{moved} items on unresolved layers moved to Cmts.User (KiCad would not load the board)"
            );
        }
    }
    let mut result = result;
    if config.tuning.tighten && config.tuning.engine.as_deref() != Some("topological") {
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
    // Tuning patterns refer to emitted segment UUIDs. Their routed geometry
    // also feeds the length report, so preserve those nets' emission.
    let tuned: BTreeSet<&str> = lengths
        .iter()
        .map(|length| length.net.as_str())
        .chain(coupled.iter().map(String::as_str))
        .collect();
    let nets = emit_routes_preserving_tuning(
        &mut pcb,
        board,
        &result,
        layer_names,
        true,
        &tuned,
        &config.native_closed_planes,
    )?;
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
        false,
    )?;
    report.warnings = warnings;
    report.diff_pairs = paired.map(|(_, _, pairs)| pairs);
    report.lengths = lengths;
    Ok(report)
}

#[cfg(test)]
mod layer_tests {
    use super::*;

    #[test]
    fn implicit_kicad_user_layers_keep_footprint_artwork() {
        let mut board = parse(
            r#"(kicad_pcb
                (layers (0 "F.Cu" signal) (19 "Cmts.User" user))
                (footprint "Library:Part" (layer "F.Cu") (at 1 1)
                    (fp_poly (layer "User.1"))
                    (fp_poly (layer "User.45")))
                (gr_line (layer "Rescue"))
                (gr_line (layer "User.46"))
                (gr_line (layer "User.01")))"#,
        )
        .unwrap();

        assert_eq!(move_undefined_layers(&mut board), 3);
        let footprint = board.child("footprint").unwrap();
        let artwork: Vec<_> = footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("fp_poly"))
            .map(|item| form_atom(item, "layer", 1).unwrap())
            .collect();
        assert_eq!(artwork, ["User.1", "User.45"]);
        assert!(
            board
                .children()
                .iter()
                .filter(|item| item.head() == Some("gr_line"))
                .all(|item| form_atom(item, "layer", 1) == Some("Cmts.User"))
        );
    }
}

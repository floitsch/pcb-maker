// Copyright (C) 2026 Toit contributors.

//! Route mode's kept copper: a board's own tracks and vias (a designer's
//! or an agent's pre-routed nets) stay exactly as they are.
//!
//! The lowering makes them fixed copper of their net, which other nets keep
//! clear of (`existing track`, `existing via`). Here the pads they join
//! become one terminal of their net (the others are merged into it, as
//! pads of one number that overlap are), so the router counts those
//! connections made and routes only what is open; it never owns a kept
//! track, so it never rips one up. A piece of kept copper that no pad of
//! its net touches is a terminal of its own (KiCad's connectivity wants it
//! joined to the rest), unless the net has a pour, whose fill joins it (a
//! stitching via). `emit_routes` with `keep_existing` writes the board's
//! own items back untouched beside the router's.
//!
//! Joined means what KiCad's connectivity joins without pours: copper of
//! the net overlapping on a layer both have (a custom pad through its
//! contact only), as `pcb_router::verify::copper_components` counts it.

use super::*;
use pcb_router as core;

/// What route mode kept of the board's own copper (`kept_copper` in
/// `board-router.json`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadKeptCopper {
    /// The board's own track segments, arcs and vias, kept as they are.
    pub segments: usize,
    pub arcs: usize,
    pub vias: usize,
    /// The nets they carry.
    pub nets: Vec<String>,
    /// Pads that kept copper joins to another pad of their net: the
    /// connections made before routing.
    pub connections_made: usize,
    /// Nets whose pads kept copper joins all: nothing left to route.
    pub nets_complete: Vec<String>,
    /// Pieces of kept copper on no pad of their net (and on a net without
    /// a pour): terminals the router connects.
    pub floating: usize,
}

/// The board's top-level track segments, arcs and vias.
pub(crate) fn count_tracks(pcb: &Expr) -> [usize; 3] {
    let mut counts = [0; 3];
    for item in pcb.children() {
        match item.head() {
            Some("segment") => counts[0] += 1,
            Some("arc") => counts[1] += 1,
            Some("via") => counts[2] += 1,
            _ => {}
        }
    }
    counts
}

/// Removes the board's track segments, arcs and vias (route mode's
/// `strip`); returns how many. Zones and graphics stay.
pub(crate) fn strip_tracks(pcb: &mut Expr) -> usize {
    let Expr::List(items) = pcb else {
        return 0;
    };
    let before = items.len();
    items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    before - items.len()
}

/// Whether an obstacle is a track or via of the board itself.
fn is_kept(obstacle: &core::Obstacle) -> bool {
    obstacle.kind == core::ObstacleKind::Copper
        && obstacle.net.is_some()
        && (obstacle.label == "existing track" || obstacle.label == "existing via")
}

fn root(parent: &mut [usize], mut item: usize) -> usize {
    while parent[item] != item {
        parent[item] = parent[parent[item]];
        item = parent[item];
    }
    item
}

/// Joins `a` and `b`; the root stays the smaller index.
fn join(parent: &mut [usize], a: usize, b: usize) {
    let (a, b) = (root(parent, a), root(parent, b));
    if a != b {
        parent[a.max(b)] = a.min(b);
    }
}

/// Where connectivity counts two shapes as touching (`verify`'s).
const TOUCH: f64 = 1.0e-5;

/// Merges the pads that the board's own tracks and vias join into one
/// terminal per piece of copper and makes pieces on no pad terminals of
/// their own (see the module's comment). A board without tracks and vias
/// is left as it is. The report's track counts are the caller's
/// (`count_tracks`). With `whole_pieces` (differential pairs on), a
/// piece's terminal reaches every item of the piece
/// (`Board::terminal_copper`), not only its first: a pair's coupled run is
/// joined to the pads at both of its ends.
pub(crate) fn keep_existing_copper(board: &mut core::Board, whole_pieces: bool) -> KiCadKeptCopper {
    let mut report = KiCadKeptCopper::default();
    let mut kept_by_net: BTreeMap<core::NetId, Vec<usize>> = BTreeMap::new();
    for (index, obstacle) in board.obstacles.iter().enumerate() {
        if is_kept(obstacle) {
            kept_by_net
                .entry(obstacle.net.unwrap_or_default())
                .or_default()
                .push(index);
        }
    }
    for (net, kept) in kept_by_net {
        let description = &board.nets[net as usize];
        report.nets.push(description.name.clone());
        let terminals = description.terminals.len();
        // Elements: the terminals (their contact or pad), the pads merged
        // into them, the kept items.
        let mut shapes: Vec<(core::Shape, u64)> = description
            .terminals
            .iter()
            .map(|terminal| {
                let pad = &board.obstacles[terminal.pad];
                (
                    terminal
                        .contact
                        .clone()
                        .unwrap_or_else(|| pad.shape.clone()),
                    pad.layers as u64,
                )
            })
            .collect();
        let mut hosts: Vec<(usize, usize)> = Vec::new();
        for merged in board.merged_pads.iter().filter(|merged| merged.net == net) {
            let pad = &board.obstacles[merged.pad];
            hosts.push((shapes.len(), merged.host));
            shapes.push((pad.shape.clone(), pad.layers as u64));
        }
        let first_kept = shapes.len();
        for &index in &kept {
            let obstacle = &board.obstacles[index];
            shapes.push((obstacle.shape.clone(), obstacle.layers as u64));
        }
        let mut parent: Vec<usize> = (0..shapes.len()).collect();
        for &(element, host) in &hosts {
            if host < terminals {
                join(&mut parent, element, host);
            }
        }
        // Pairs by a sweep over the boxes' x ranges; only pairs with kept
        // copper join (pads that touch each other are the lowering's).
        let boxes: Vec<core::geometry::Aabb> =
            shapes.iter().map(|(shape, _)| shape.aabb()).collect();
        let mut order: Vec<usize> = (0..shapes.len()).collect();
        order.sort_by(|a, b| {
            boxes[*a].minimum[0]
                .total_cmp(&boxes[*b].minimum[0])
                .then(a.cmp(b))
        });
        for (position, &a) in order.iter().enumerate() {
            for &b in &order[position + 1..] {
                if boxes[b].minimum[0] > boxes[a].maximum[0] + TOUCH {
                    break;
                }
                if (a < first_kept && b < first_kept)
                    || shapes[a].1 & shapes[b].1 == 0
                    || !boxes[a].inflated(TOUCH).intersects(boxes[b])
                    || root(&mut parent, a) == root(&mut parent, b)
                {
                    continue;
                }
                if core::verify::shape_distance(&shapes[a].0, &shapes[b].0) <= TOUCH {
                    join(&mut parent, a, b);
                }
            }
        }
        // The root of a piece holding terminals is its first terminal (the
        // smaller index wins), which stays; the piece's other terminals
        // are merged into it.
        let host_of: Vec<usize> = (0..terminals)
            .map(|terminal| root(&mut parent, terminal))
            .collect();
        let mut renumbered = vec![usize::MAX; terminals];
        let mut next = 0;
        for terminal in 0..terminals {
            if host_of[terminal] == terminal {
                renumbered[terminal] = next;
                next += 1;
            }
        }
        let mut merged = Vec::new();
        for terminal in 0..terminals {
            if host_of[terminal] != terminal {
                let description = &board.nets[net as usize].terminals[terminal];
                merged.push(core::MergedPad {
                    net,
                    host: renumbered[host_of[terminal]],
                    pad: description.pad,
                    anchor: description.anchor,
                    layers: board.obstacles[description.pad].layers,
                    padstack: false,
                });
            }
        }
        report.connections_made += merged.len();
        if !merged.is_empty() {
            // The pads merged before keep their host, renumbered.
            for existing in board
                .merged_pads
                .iter_mut()
                .filter(|existing| existing.net == net && existing.host < terminals)
            {
                // A padstack's shape whose pad was merged into another
                // terminal is another pad of that terminal.
                existing.padstack &= host_of[existing.host] == existing.host;
                existing.host = renumbered[host_of[existing.host]];
            }
            let mut index = 0;
            board.nets[net as usize].terminals.retain(|_| {
                index += 1;
                host_of[index - 1] == index - 1
            });
            board.merged_pads.extend(merged);
        }
        if terminals >= 2 && next == 1 {
            report
                .nets_complete
                .push(board.nets[net as usize].name.clone());
        }
        // With `whole_pieces`, a terminal reaches the kept copper of its
        // piece: a route to it may end anywhere on that copper.
        if whole_pieces {
            for (offset, &index) in kept.iter().enumerate() {
                let piece = root(&mut parent, first_kept + offset);
                if piece < terminals {
                    board.terminal_copper.push(core::TerminalCopper {
                        net,
                        terminal: renumbered[piece],
                        obstacle: index,
                    });
                }
            }
        }
        // Pieces on no terminal: a terminal each where no pour joins them.
        if board.planes.iter().any(|plane| plane.net == net) {
            continue;
        }
        let mut seen = std::collections::BTreeSet::new();
        for (offset, &index) in kept.iter().enumerate() {
            let element = first_kept + offset;
            let piece = root(&mut parent, element);
            if piece < first_kept || !seen.insert(piece) {
                continue;
            }
            let obstacle = &board.obstacles[index];
            let anchor = match &obstacle.shape {
                core::Shape::Capsule { start, end, .. } => {
                    [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0]
                }
                shape => {
                    let bounds = shape.aabb();
                    [
                        (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                        (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
                    ]
                }
            };
            report.floating += 1;
            if whole_pieces {
                let terminal = board.nets[net as usize].terminals.len();
                for (other_offset, &other) in kept.iter().enumerate() {
                    if other != index && root(&mut parent, first_kept + other_offset) == piece {
                        board.terminal_copper.push(core::TerminalCopper {
                            net,
                            terminal,
                            obstacle: other,
                        });
                    }
                }
            }
            board.nets[net as usize].terminals.push(core::Terminal {
                anchor,
                layers: obstacle.layers,
                pad: index,
                contact: None,
                label: format!("{} {}", obstacle.label, report.floating),
            });
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three pads of one net in a row, A and B joined by a track and a via
    /// (as the fixture `plated-branch` is, less its last track), and a net
    /// of two pads without copper.
    const BOARD: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6)) (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 20 20) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
  (footprint "test" (layer "F.Cu") (at 2 10)
    (property "Reference" "J1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/A")))
  (footprint "test" (layer "F.Cu") (at 8 10)
    (property "Reference" "J2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 0.8) (layers "*.Cu" "*.Mask") (net "/A")))
  (footprint "test" (layer "F.Cu") (at 18 10)
    (property "Reference" "J3" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/A")))
  (footprint "test" (layer "F.Cu") (at 2 4)
    (property "Reference" "J4" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/B")))
  (footprint "test" (layer "F.Cu") (at 18 4)
    (property "Reference" "J5" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/B")))
  (segment (start 2 10) (end 7.8 10) (width 0.25) (layer "F.Cu") (net "/A"))
  (segment (start 12 16) (end 14 16) (width 0.25) (layer "B.Cu") (net "/A")))
"#;

    fn lowered(text: &str) -> core::Board {
        let pcb = parse(text).unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.25,
                clearance_mm: 0.2,
                via_size_mm: 0.7,
                via_drill_mm: 0.35,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        lower(&pcb, &config, true).unwrap().board
    }

    fn net<'a>(board: &'a core::Board, name: &str) -> (core::NetId, &'a core::Net) {
        board
            .nets
            .iter()
            .enumerate()
            .find(|(_, net)| net.name == name)
            .map(|(index, net)| (index as core::NetId, net))
            .unwrap()
    }

    #[test]
    fn pads_a_kept_track_joins_are_one_terminal_and_a_loose_track_is_another() {
        let mut board = lowered(BOARD);
        assert_eq!(net(&board, "A").1.terminals.len(), 3);
        let report = keep_existing_copper(&mut board, false);
        let (a, description) = net(&board, "A");
        let labels: Vec<&str> = description
            .terminals
            .iter()
            .map(|terminal| terminal.label.as_str())
            .collect();
        // J1 and J2 are one terminal (J2 merged into J1), J3 stays, and the
        // track on no pad is a terminal of its own.
        assert_eq!(labels.len(), 3, "{labels:?}");
        assert!(labels[0].starts_with("J1"), "{labels:?}");
        assert!(labels[1].starts_with("J3"), "{labels:?}");
        assert_eq!(labels[2], "existing track 1");
        let merged: Vec<&core::MergedPad> = board
            .merged_pads
            .iter()
            .filter(|merged| merged.net == a)
            .collect();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].host, 0);
        assert_eq!(merged[0].anchor, [8.0, 10.0]);
        assert_eq!(net(&board, "B").1.terminals.len(), 2);
        assert_eq!(report.connections_made, 1);
        assert_eq!(report.nets, vec!["A".to_string()]);
        assert!(report.nets_complete.is_empty());
        assert_eq!(report.floating, 1);
    }

    #[test]
    fn a_net_its_tracks_join_whole_has_nothing_left_to_route() {
        let text = BOARD.replace(
            "(segment (start 12 16) (end 14 16) (width 0.25) (layer \"B.Cu\") (net \"/A\"))",
            "(segment (start 8.2 10) (end 12 10) (width 0.25) (layer \"B.Cu\") (net \"/A\"))
  (via (at 12 10) (size 0.7) (drill 0.35) (layers \"F.Cu\" \"B.Cu\") (net \"/A\"))
  (arc (start 12 10) (mid 15 8.5) (end 18 10) (width 0.25) (layer \"F.Cu\") (net \"/A\"))",
        );
        let pcb = parse(&text).unwrap();
        assert_eq!(count_tracks(&pcb), [2, 1, 1]);
        let mut board = lowered(&text);
        let report = keep_existing_copper(&mut board, false);
        assert_eq!(net(&board, "A").1.terminals.len(), 1);
        assert_eq!(report.connections_made, 2);
        assert_eq!(report.nets_complete, vec!["A".to_string()]);
        assert_eq!(report.floating, 0);
        // The router has nothing to do on A and routes B.
        let routed = core::route(&board, &core::Config::default());
        let (a, _) = net(&board, "A");
        let (b, _) = net(&board, "B");
        assert!(routed.routes[a as usize].segments.is_empty());
        assert!(!routed.routes[b as usize].segments.is_empty());
        assert_eq!(routed.status[b as usize], core::NetStatus::Routed);
        // The other net's route keeps clear of the kept copper.
        assert!(core::verify(&board, &routed.routes).is_empty());
        let mut stripped = pcb.clone();
        assert_eq!(strip_tracks(&mut stripped), 4);
        assert_eq!(count_tracks(&stripped), [0, 0, 0]);
    }

    #[test]
    fn a_board_without_tracks_is_left_as_it_is() {
        let text = BOARD
            .replace(
                "(segment (start 2 10) (end 7.8 10) (width 0.25) (layer \"F.Cu\") (net \"/A\"))",
                "",
            )
            .replace(
                "(segment (start 12 16) (end 14 16) (width 0.25) (layer \"B.Cu\") (net \"/A\"))",
                "",
            );
        let mut board = lowered(&text);
        let before: Vec<usize> = board.nets.iter().map(|net| net.terminals.len()).collect();
        let merged = board.merged_pads.clone();
        assert_eq!(
            keep_existing_copper(&mut board, false),
            KiCadKeptCopper::default()
        );
        let after: Vec<usize> = board.nets.iter().map(|net| net.terminals.len()).collect();
        assert_eq!(before, after);
        assert_eq!(board.merged_pads, merged);
    }
}

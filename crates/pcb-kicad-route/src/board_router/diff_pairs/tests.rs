// Copyright (C) 2026 Toit contributors.

//! Tests of the differential pairs: detection, the plan, the coupled
//! copper, the stubs, skew, return vias and the fallback.

use super::*;

/// A USB pair from J1 (two rows: two pair points of one part) to U2,
/// past U1, whose D+ and D- pads have a GND pin between them.
const BOARD: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6)) (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 40 20) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
  (footprint "test" (layer "F.Cu") (at 3 10)
    (property "Reference" "J1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "3" smd rect (at 0 2) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "4" smd rect (at 0 2.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 10)
    (property "Reference" "U1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/GND"))
    (pad "3" smd rect (at 0 1) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 34 10)
    (property "Reference" "U2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)
    (property "Reference" "J5" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/GND"))))
"#;

fn lowered() -> (core::Board, KiCadBoardRouterConfig) {
    let pcb = parse(BOARD).unwrap();
    let config = KiCadBoardRouterConfig {
        default_rules: Some(KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }),
        ..KiCadBoardRouterConfig::default()
    };
    let board = lower(&pcb, &config, true).unwrap().board;
    (board, config)
}

fn terminals(board: &core::Board, name: &str) -> Vec<String> {
    let net = board
        .nets
        .iter()
        .find(|net| net.name.trim_start_matches('/') == name)
        .unwrap();
    net.terminals
        .iter()
        .map(|terminal| terminal.label.clone())
        .collect()
}

#[test]
fn pair_points_are_pads_side_by_side_and_links_join_parts() {
    let (board, config) = lowered();
    let pairs = find_pairs(&board, &config);
    assert_eq!(pairs.len(), 1);
    let pair = &pairs[0];
    let p = &board.nets[pair.positive as usize];
    let labels: Vec<String> = pair
        .points
        .iter()
        .map(|&(a, _)| p.terminals[a].label.clone())
        .collect();
    // U1's pads have its GND pin between them: no pair point there.
    assert_eq!(labels, ["J1.1", "J1.3", "U2.1"]);
    // J1's two rows are joined by ordinary routing: one link, J1 to U2.
    assert_eq!(pair.links.len(), 1);
    let (a, b) = pair.links[0];
    let ends = [labels[a].as_str(), labels[b].as_str()];
    assert!(ends.contains(&"U2.1"), "{ends:?}");
}

/// Two pairs whose pads lie side by side on one part only (U2): S
/// across a ground pin on another (U1, an HDMI connector's or an ESD
/// part's pins), T apart on two series parts (R1, R2, `apart` mm).
fn one_point_board(apart: f64) -> String {
    format!(
        r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6)) (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 40 24) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
  (footprint "test" (layer "F.Cu") (at 18 10)
    (property "Reference" "U1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/S+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/GND"))
    (pad "3" smd rect (at 0 1) (size 1 0.3) (layers "F.Cu") (net "/S-")))
  (footprint "test" (layer "F.Cu") (at 34 10)
    (property "Reference" "U2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/S+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/S-"))
    (pad "3" smd rect (at 0 4) (size 1 0.3) (layers "F.Cu") (net "/T+"))
    (pad "4" smd rect (at 0 4.5) (size 1 0.3) (layers "F.Cu") (net "/T-")))
  (footprint "test" (layer "F.Cu") (at 10 14)
    (property "Reference" "R1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/T+"))
    (pad "2" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/A")))
  (footprint "test" (layer "F.Cu") (at 10 {})
    (property "Reference" "R2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/T-"))
    (pad "2" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/B")))
  (footprint "test" (layer "F.Cu") (at 3 22)
    (property "Reference" "J5" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/GND"))))
"#,
        14.0 + apart
    )
}

fn one_point_pairs(apart: f64, extended: bool) -> (core::Board, KiCadBoardRouterConfig, Vec<Pair>) {
    let pcb = parse(&one_point_board(apart)).unwrap();
    let (_, config) = lowered();
    let board = lower(&pcb, &config, true).unwrap().board;
    EXTENDED.with(|cell| cell.set(Some(extended)));
    let pairs = find_pairs(&board, &config);
    EXTENDED.with(|cell| cell.set(None));
    (board, config, pairs)
}

fn pair_named<'a>(board: &core::Board, pairs: &'a [Pair], name: &str) -> &'a Pair {
    pairs
        .iter()
        .find(|pair| board.nets[pair.positive as usize].name.ends_with(name))
        .unwrap()
}

/// A pair with no link otherwise (one part with its pads side by side)
/// takes a part's pads with a ground pin between them as a pair point:
/// the run goes from U2 to U1.
#[test]
fn a_pair_without_a_link_takes_pads_across_a_pin_as_a_pair_point() {
    let (board, _, pairs) = one_point_pairs(4.0, false);
    assert!(pair_named(&board, &pairs, "S+").links.is_empty());
    let (board, _, pairs) = one_point_pairs(4.0, true);
    let pair = pair_named(&board, &pairs, "S+");
    assert_eq!(pair.kinds, [PointKind::Side, PointKind::Pin]);
    assert_eq!(pair.links.len(), 1);
    let p = &board.nets[pair.positive as usize];
    let labels: Vec<&str> = pair
        .points
        .iter()
        .map(|&(a, _)| p.terminals[a].label.as_str())
        .collect();
    assert_eq!(labels, ["U2.1", "U1.1"]);
    // A pair with a link keeps its points: the original board's U1 is
    // no pair point (`pair_points_are_pads_side_by_side_and_links_join_parts`).
    // T's other pads lie on two series parts 4 mm apart: still no link
    // (a split end there was built and measured, and left out).
    let pair = pair_named(&board, &pairs, "T+");
    assert_eq!(pair.kinds, [PointKind::Side]);
    assert!(pair.links.is_empty());
}

/// A pin between a pair point's pads must keep a way out past the
/// stubs and the run's lead: U1's ground pin (1 mm long pads, room on
/// its far side) does, U9's (a 0.4 mm pitch row with other nets' pins
/// beside and behind it, MSign's USB muxes) does not, and that pair
/// point makes no run.
#[test]
fn a_pin_point_whose_pin_would_be_walled_in_makes_no_run() {
    let (mut board, config, _) = one_point_pairs(4.0, true);
    EXTENDED.with(|cell| cell.set(Some(true)));
    plan_pairs(&mut board, &config);
    let plan = plan_of(&board).unwrap();
    let entry = plan
        .entries
        .iter()
        .find(|entry| entry.positive.ends_with("S+"))
        .unwrap();
    assert!(entry.why.is_empty(), "{entry:?}");
    assert_eq!(entry.links, 1);
    let walled = one_point_board(4.0).replace(
        r#"  (footprint "test" (layer "F.Cu") (at 18 10)
    (property "Reference" "U1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/S+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/GND"))
    (pad "3" smd rect (at 0 1) (size 1 0.3) (layers "F.Cu") (net "/S-")))"#,
        r#"  (footprint "test" (layer "F.Cu") (at 18 10)
    (property "Reference" "U9" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at -0.4 0) (size 0.2 0.6) (layers "F.Cu") (net "/X1"))
    (pad "2" smd rect (at 0 0) (size 0.2 0.6) (layers "F.Cu") (net "/S+"))
    (pad "3" smd rect (at 0.4 0) (size 0.2 0.6) (layers "F.Cu") (net "/GND"))
    (pad "4" smd rect (at 0.8 0) (size 0.2 0.6) (layers "F.Cu") (net "/S-"))
    (pad "5" smd rect (at 1.2 0) (size 0.2 0.6) (layers "F.Cu") (net "/X2"))
    (pad "6" smd rect (at 0.4 -0.75) (size 1.8 0.6) (layers "F.Cu") (net "/X3")))"#,
    );
    assert_ne!(walled, one_point_board(4.0));
    let pcb = parse(&walled).unwrap();
    let mut board = lower(&pcb, &config, true).unwrap().board;
    let pair = pair_named(&board, &find_pairs(&board, &config), "S+").clone();
    assert_eq!(pair.kinds, [PointKind::Side, PointKind::Pin]);
    plan_pairs(&mut board, &config);
    EXTENDED.with(|cell| cell.set(None));
    let plan = plan_of(&board).unwrap();
    let entry = plan
        .entries
        .iter()
        .find(|entry| entry.positive.ends_with("S+"))
        .unwrap();
    assert!(
        entry
            .why
            .iter()
            .any(|why| why.contains("U9.3") && why.contains("walled in")),
        "{entry:?}"
    );
    assert!(
        !plan
            .links
            .iter()
            .any(|link| plan.pairs[link.pair].positive == pair.positive)
    );
}

/// Two two-pad parts side by side on the pair (its AC-coupling
/// capacitors) are a pair point; two whose other pads are far apart
/// are not.
#[test]
fn series_parts_side_by_side_are_a_pair_point() {
    let caps = BOARD.replace(
        r#"  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        r#"  (footprint "test" (layer "F.Cu") (at 26 10)
    (property "Reference" "C1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/X+")))
  (footprint "test" (layer "F.Cu") (at 26 11)
    (property "Reference" "C2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/X-")))
  (footprint "test" (layer "F.Cu") (at 10 5)
    (property "Reference" "C3" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/Y")))
  (footprint "test" (layer "F.Cu") (at 9.5 6)
    (property "Reference" "C4" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 3.5) (size 0.5 0.5) (layers "F.Cu") (net "/Z")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
    );
    let pcb = parse(&caps).unwrap();
    let (_, config) = lowered();
    let board = lower(&pcb, &config, true).unwrap().board;
    let pairs = find_pairs(&board, &config);
    let pair = pairs
        .iter()
        .find(|pair| board.nets[pair.positive as usize].name.ends_with("D+"))
        .unwrap();
    let (p, n) = (
        &board.nets[pair.positive as usize],
        &board.nets[pair.negative as usize],
    );
    let labels: Vec<(String, String)> = pair
        .points
        .iter()
        .map(|&(a, b)| (p.terminals[a].label.clone(), n.terminals[b].label.clone()))
        .collect();
    assert!(
        labels.contains(&("C1.1".to_string(), "C2.1".to_string())),
        "{labels:?}"
    );
    // C3's and C4's other pads are 4 mm apart.
    assert!(!labels.iter().any(|(a, _)| a == "C3.1"), "{labels:?}");
    // J1's rows and U2 as before, and the capacitors: three links.
    assert_eq!(labels.len(), 4, "{labels:?}");
    assert_eq!(pair.links.len(), 2);
}

/// A pair row's side launches out of the parts, away from their other
/// pads, wherever the link's far end lies: J1 north-east of the
/// capacitors' D side (the source of right_io's HDMI pairs beside their
/// row), whose normal pointing "toward" it went north through the
/// bodies past the X pair's pads.
#[test]
fn a_series_pair_point_launches_away_from_its_parts() {
    let board_text = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6)) (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 40 30) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
  (footprint "test" (layer "F.Cu") (at 30 9.5)
    (property "Reference" "J1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 0.3 1) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0.5 0) (size 0.3 1) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 20 20) (property "Reference" "C1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0.48) (size 0.62 0.56) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 -0.48) (size 0.62 0.56) (layers "F.Cu") (net "/X+")))
  (footprint "test" (layer "F.Cu") (at 20.93 20) (property "Reference" "C2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0.48) (size 0.62 0.56) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 -0.48) (size 0.62 0.56) (layers "F.Cu") (net "/X-")))
  (footprint "test" (layer "F.Cu") (at 10 10)
    (property "Reference" "J2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 0.3 1) (layers "F.Cu") (net "/X+"))
    (pad "2" smd rect (at 0.5 0) (size 0.3 1) (layers "F.Cu") (net "/X-"))))
"#;
    let pcb = parse(board_text).unwrap();
    let (_, config) = lowered();
    let mut board = lower(&pcb, &config, true).unwrap().board;
    plan_pairs(&mut board, &config);
    let plan = board
        .attachment
        .data
        .clone()
        .unwrap()
        .downcast::<PairPlan>()
        .unwrap();
    // The D pair's link: its end at the capacitors' D pads (y 20.48)
    // launches south, out of the row; the X pair's (y 19.52) north.
    let mut seen = 0;
    for link in &plan.links {
        for end in 0..2 {
            let pads = link.pads[end];
            let middle = scale(add(pads[0], pads[1]), 0.5);
            if (middle[0] - 20.465).abs() > 0.01 {
                continue;
            }
            let outward = sub(link.starts[end], middle);
            let south = middle[1] > 20.0;
            assert!(
                if south {
                    outward[1] > 0.0
                } else {
                    outward[1] < 0.0
                },
                "the end at {middle:?} launches along {outward:?}"
            );
            seen += 1;
        }
    }
    assert_eq!(seen, 2);
}

#[test]
fn a_link_is_a_net_with_launches_and_the_pairs_nets_end_beside_its_run() {
    let (mut board, config) = lowered();
    let (nets, obstacles) = (board.nets.len(), board.obstacles.len());
    plan_pairs(&mut board, &config);
    // One link: a net of its own at the envelope, kept to no layer.
    assert_eq!(board.nets.len(), nets + 1);
    let link = &board.nets[nets];
    let class = board.classes[link.class];
    assert!((class.trace_width - 0.6).abs() < 1.0e-9, "{class:?}");
    assert_eq!(class.layers, 0);
    assert_eq!(link.terminals.len(), 2);
    // The pair's nets: the link carries straight stubs at both ends
    // (the pads have room), so the link's two pads and the run's two
    // ends are one terminal (the link's first pad, the others merged
    // pads a route may end on): of four pads three terminals.
    for name in ["D+", "D-"] {
        let labels = terminals(&board, name);
        assert!(
            !labels.iter().any(|label| label.starts_with("pair run")),
            "{labels:?}"
        );
        assert_eq!(labels.len(), 3, "{labels:?}");
        assert!(!labels.contains(&"U2.1".to_string()), "{labels:?}");
    }
    assert!(board.obstacles.len() > obstacles);
    assert_eq!(
        board.obstacles[obstacles..]
            .iter()
            .filter(|obstacle| obstacle.label.starts_with("pair stub"))
            .count(),
        4
    );
    // The launches lie outside the pads, the run's ends beside them.
    let plan = board
        .attachment
        .data
        .clone()
        .unwrap()
        .downcast::<PairPlan>()
        .unwrap();
    let link_plan = &plan.links[0];
    assert_eq!(link_plan.fixed, [true, true]);
    for end in 0..2 {
        let [p, n] = link_plan.ends[end];
        let apart = core::geometry::distance(p, n);
        assert!(apart >= 0.2 + 0.2 - 1.0e-9, "the ends {apart} apart");
        let pads = link_plan.pads[end];
        for (end_point, pad) in [(p, pads[0]), (n, pads[1])] {
            assert!(core::geometry::distance(end_point, pad) > 0.5);
        }
    }
}

/// The links may be given up (`core::GiveUp`, with the settings'
/// `diff_pair_give_up`; off by default): with nothing released
/// a pair's net is as planned, with its link released it is the net as
/// lowered (its pads, no run ends, nothing merged), and a route of the
/// planned board whose link finds no room couples nothing but leaves
/// no connection of the pair open.
#[test]
fn a_link_given_up_gives_the_pairs_nets_their_pads_back() {
    use pcb_router::GiveUp;
    let (lowered_board, config) = lowered();
    // Off by default: the plan offers no link to give up.
    let mut board = lowered_board.clone();
    plan_pairs(&mut board, &config);
    assert!(board.attachment.give_up.is_none());
    let config = KiCadBoardRouterConfig {
        diff_pair_give_up: Some(true),
        ..config
    };
    let mut board = lowered_board.clone();
    plan_pairs(&mut board, &config);
    let plan = plan_of(&board).unwrap();
    assert!(board.attachment.give_up.is_some());
    let link = plan.links[0].net;
    assert!(plan.optional(link));
    assert!(!plan.optional(plan.pairs[0].positive));
    let labels = |terminals: &[core::Terminal]| -> Vec<String> {
        terminals
            .iter()
            .map(|terminal| terminal.label.clone())
            .collect()
    };
    for polarity in 0..2 {
        let net = [plan.pairs[0].positive, plan.pairs[0].negative][polarity];
        let planned = plan
            .net_with_links(&board, 0, polarity, &BTreeSet::new())
            .unwrap();
        assert_eq!(
            labels(&planned.terminals),
            labels(&board.nets[net as usize].terminals)
        );
        let of_net = |merged: &[core::MergedPad]| -> Vec<(usize, usize)> {
            merged
                .iter()
                .filter(|merged| merged.net == net)
                .map(|merged| (merged.host, merged.pad))
                .collect()
        };
        assert_eq!(of_net(&planned.merged_pads), of_net(&board.merged_pads));
        let copper = |extras: &[core::TerminalCopper]| -> Vec<core::TerminalCopper> {
            extras
                .iter()
                .filter(|extra| extra.net == net)
                .copied()
                .collect()
        };
        assert_eq!(
            copper(&planned.terminal_copper),
            copper(&board.terminal_copper)
        );
        assert!(!planned.merged_pads.is_empty());
    }
    // Standing again (`retry_released`): the nets as planned.
    let standing = plan.standing(&board, link, &[link]);
    assert_eq!(standing.len(), 2);
    for rewrite in &standing {
        assert_eq!(
            labels(&rewrite.terminals),
            labels(&board.nets[rewrite.net as usize].terminals)
        );
    }
    let rewrites = plan.released(&board, &[link]);
    assert_eq!(rewrites.len(), 2);
    for rewrite in &rewrites {
        assert_eq!(
            labels(&rewrite.terminals),
            labels(&lowered_board.nets[rewrite.net as usize].terminals)
        );
        assert!(rewrite.merged_pads.is_empty());
        assert!(rewrite.terminal_copper.is_empty());
    }
}

/// A layout's routed stages lower with `lower_for_route`: with the
/// router's `diff_pairs` the board carries the pairs' plan (the link a
/// net of the route) and the plan's work is booked by the stage; off
/// (a layout's default) the board is `lower`'s alone, no work; the
/// topological engine plans none.
#[test]
fn a_layouts_routed_stages_lower_with_the_pairs_plan_and_book_its_work() {
    let pcb = parse(BOARD).unwrap();
    let (plain, config) = lowered();
    let (board, work) = lower_for_route(&pcb, &config, true).unwrap();
    assert_eq!(board.nets.len(), plain.nets.len());
    assert!(plan_of(&board).is_none());
    assert_eq!(work, 0.0);
    let on = KiCadBoardRouterConfig {
        diff_pairs: Some(true),
        ..config.clone()
    };
    assert!(pairs_planned(&on));
    let (board, work) = lower_for_route(&pcb, &on, true).unwrap();
    assert_eq!(board.nets.len(), plain.nets.len() + 1);
    let plan = plan_of(&board).unwrap();
    assert_eq!(plan.links.len(), 1);
    assert!(work > 0.0, "{work}");
    assert_eq!(work, planned_work(&board));
    // The same board plans the same work (a model of its tests, not
    // the clock).
    let (_, again) = lower_for_route(&pcb, &on, true).unwrap();
    assert_eq!(again, work);
    let topological = KiCadBoardRouterConfig {
        tuning: KiCadRouterTuning {
            engine: Some("topological".into()),
            ..on.tuning.clone()
        },
        ..on
    };
    assert!(!pairs_planned(&topological));
    assert_eq!(lower_for_route(&pcb, &topological, true).unwrap().1, 0.0);
}

/// A move that changes the pairs' plan (here a pair point gone: U2's
/// pads apart, and U1's, so that no pair point across a pin takes its
/// place)
/// changes the nets the router holds: `Router::update`
/// refuses the board with `UPDATE_NEEDS_SAME_NETS`, which a layout's
/// moves take for a move not to make (not an error of the layout).
#[test]
fn a_move_that_changes_the_pairs_plan_is_refused_by_the_routers_update() {
    let (_, config) = lowered();
    let on = KiCadBoardRouterConfig {
        diff_pairs: Some(true),
        ..config
    };
    let (board, _) = lower_for_route(&parse(BOARD).unwrap(), &on, true).unwrap();
    let apart = BOARD
        .replace(
            r#"(pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
            r#"(pad "2" smd rect (at 0 5.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        )
        .replace(
            r#"(pad "3" smd rect (at 0 1) (size 1 0.3) (layers "F.Cu") (net "/D-")))"#,
            r#"(pad "3" smd rect (at 0 6) (size 1 0.3) (layers "F.Cu") (net "/D-")))"#,
        );
    assert_ne!(apart, BOARD);
    let (moved, _) = lower_for_route(&parse(&apart).unwrap(), &on, true).unwrap();
    assert_ne!(moved.nets.len(), board.nets.len());
    let mut router = core::router::Router::new(&board, &core_config(&on));
    assert_eq!(
        router.update(&moved).unwrap_err(),
        core::router::UPDATE_NEEDS_SAME_NETS
    );
    // The same plan updates.
    assert!(router.update(&board).is_ok());
}

/// A rule disallowing tracks on In1.Cu for the pair's nets (ducktop2's
/// ground layers), and one bounding their width on In2.Cu below the
/// width they route at: the pair's nets (their stubs, the fallback) and
/// the link keep off both, as the coupled run does; other nets do not.
#[test]
fn a_pairs_nets_keep_off_the_layers_a_rule_disallows() {
    let four = BOARD.replace(
        r#"(layers (0 "F.Cu" signal) (31 "B.Cu" signal)"#,
        r#"(layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (31 "B.Cu" signal)"#,
    );
    let pcb = parse(&four).unwrap();
    let mut config = KiCadBoardRouterConfig {
        default_rules: Some(KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }),
        ..KiCadBoardRouterConfig::default()
    };
    let names: Vec<String> = lower(&pcb, &config, true)
        .unwrap()
        .board
        .nets
        .iter()
        .map(|net| net.name.clone())
        .filter(|name| name.ends_with("D+") || name.ends_with("D-"))
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    for name in &names {
        config.diff_pair_rules.insert(
            name.clone(),
            crate::board_router::KiCadDiffPairRule {
                layers: vec![(0.2, 0.2), (0.2, 0.2), (0.1, 0.2), (0.2, 0.2)],
                disallowed: vec![false, true, false, false],
                width_ruled: vec![false, false, true, false],
                // In2.Cu: at most 0.1 mm (the nets route at 0.2).
                width_bounds: vec![
                    (0.0, f64::INFINITY),
                    (0.0, f64::INFINITY),
                    (0.0, 0.1),
                    (0.0, f64::INFINITY),
                ],
                sources: vec!["rule 'pairs'".into(); 4],
                ..Default::default()
            },
        );
    }
    let class_layers = |board: &core::Board, name: &str| {
        let net = board.nets.iter().find(|net| net.name == name).unwrap();
        board.classes[net.class].layers
    };
    // Off: the pair's nets go anywhere, as before.
    config.diff_pairs = Some(false);
    let board = lower(&pcb, &config, true).unwrap().board;
    assert_eq!(class_layers(&board, &names[0]), 0);
    // On: F.Cu and B.Cu (In1.Cu is bit 1, In2.Cu bit 2); GND anywhere.
    config.diff_pairs = Some(true);
    let mut board = lower(&pcb, &config, true).unwrap().board;
    assert_eq!(board.layer_count, 4);
    for name in &names {
        assert_eq!(class_layers(&board, name), 0b1001, "{name}");
    }
    let gnd = board
        .nets
        .iter()
        .position(|net| net.name.ends_with("GND"))
        .unwrap();
    assert_eq!(board.classes[board.nets[gnd].class].layers, 0);
    let nets = board.nets.len();
    plan_pairs(&mut board, &config);
    assert_eq!(board.nets.len(), nets + 1);
    let link = board.classes[board.nets[nets].class].layers;
    assert!(link != 0 && link & 0b110 == 0, "{link:b}");
    // Routed: no track of the pair's nets or the link on In1.Cu or
    // In2.Cu.
    let routed = core::route(&board, &core_config(&config));
    let pair_nets: Vec<usize> = board
        .nets
        .iter()
        .enumerate()
        .filter(|(index, net)| *index >= nets || names.contains(&net.name))
        .map(|(index, _)| index)
        .collect();
    for net in pair_nets {
        assert!(
            routed.routes[net]
                .segments
                .iter()
                .all(|segment| segment.layer == 0 || segment.layer == 3),
            "{} has a track on an inner layer",
            board.nets[net].name
        );
    }
}

/// A pair whose two nets prefer the same layers by a layer-limited
/// length or skew rule (`length_rule_layers`, trial 7's Sisu_SSE-9:
/// RP_USB off In1.Cu and In4.Cu as the designer runs it): its coupled
/// links prefer them too, under the name the lowering gives them;
/// nets preferring different layers leave the links free.
#[test]
fn a_pairs_links_prefer_the_layers_both_its_nets_prefer() {
    let pcb = parse(BOARD).unwrap();
    let mut config = KiCadBoardRouterConfig {
        default_rules: Some(KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }),
        diff_pairs: Some(true),
        ..KiCadBoardRouterConfig::default()
    };
    let mut board = lower(&pcb, &config, true).unwrap().board;
    let names: Vec<String> = board
        .nets
        .iter()
        .map(|net| net.name.clone())
        .filter(|name| name.ends_with("D+") || name.ends_with("D-"))
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    let nets = board.nets.len();
    plan_pairs(&mut board, &config);
    assert_eq!(board.nets.len(), nets + 1);
    let link = board.nets[nets].name.clone();
    let prefer = |config: &mut KiCadBoardRouterConfig, masks: [core::LayerMask; 2]| {
        for (name, mask) in names.iter().zip(masks) {
            config.length_layer_preferences.insert(
                name.clone(),
                crate::board_router::KiCadLayerPreference {
                    mask,
                    ..Default::default()
                },
            );
        }
    };
    let link_mask = |config: &KiCadBoardRouterConfig| {
        core_config(config)
            .layer_preferences
            .and_then(|masks| masks.get(&link).copied())
    };
    prefer(&mut config, [0b10, 0b10]);
    assert_eq!(link_mask(&config), Some(0b10));
    prefer(&mut config, [0b10, 0b01]);
    assert_eq!(link_mask(&config), None);
}

/// U2's pads in the other order (D- above D+): the link is crossed at
/// one end, which gets a crossover in its lead (the negative net dives
/// under the positive one or the other way round): the link couples,
/// its stubs straight, no stub crossing another.
#[test]
fn a_crossed_end_crosses_over_in_its_lead() {
    let crossed = BOARD.replace(
        r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D+")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
    );
    assert_ne!(crossed, BOARD);
    let pcb = parse(&crossed).unwrap();
    let config = KiCadBoardRouterConfig {
        default_rules: Some(KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }),
        ..KiCadBoardRouterConfig::default()
    };
    let plan_of = |board: &core::Board| {
        board
            .attachment
            .data
            .clone()
            .unwrap()
            .downcast::<PairPlan>()
            .unwrap()
    };
    // Off by default: the crossed end's stubs are the router's.
    CROSSOVERS.with(|cell| cell.set(Some(false)));
    let mut board = lower(&pcb, &config, true).unwrap().board;
    plan_pairs(&mut board, &config);
    assert!(
        plan_of(&board).links[0]
            .crossovers
            .iter()
            .all(Option::is_none)
    );
    CROSSOVERS.with(|cell| cell.set(Some(true)));
    let mut board = lower(&pcb, &config, true).unwrap().board;
    plan_pairs(&mut board, &config);
    let plan = board
        .attachment
        .data
        .clone()
        .unwrap()
        .downcast::<PairPlan>()
        .unwrap();
    let link_plan = &plan.links[0];
    assert!(link_plan.crossed);
    let crossing = link_plan
        .crossovers
        .iter()
        .flatten()
        .next()
        .expect("a crossover");
    // One net dives: two vias, a track on B.Cu.
    let vias: usize = crossing.vias.iter().map(Vec::len).sum();
    assert_eq!(vias, 2);
    assert!(
        crossing
            .segments
            .iter()
            .flatten()
            .any(|(layer, ..)| *layer == 1)
    );
    // At each end the stubs' ends keep the pads' order: the stubs do
    // not cross.
    for end in 0..2 {
        let [p, n] = link_plan.pads[end];
        let [end_p, end_n] = link_plan.ends[end];
        assert!(
            core::geometry::segment_segment_distance(p, end_p, n, end_n) > 0.2,
            "end {end}: stubs cross"
        );
    }
    let mut result = core::route(&board, &core_config(&config));
    let (_, copper, report) = finish_pairs(&board, &mut result, &config, &[0.0, 1.6]).unwrap();
    assert_eq!(report.links_coupled, 1, "{:?}", report.pairs);
    assert_eq!(report.crossovers, 1);
    // The diving net's two vias (1.6 mm each between F.Cu and B.Cu) and
    // its detour make the pair's lengths differ by more than its gap:
    // the shorter net gets a meander of what it lacks, its tracks a
    // tuning pattern.
    assert_eq!(report.meanders, 1, "{:?}", report.pairs);
    let entry = report.pairs.iter().find(|pair| pair.links > 0).unwrap();
    assert!(entry.meander_mm > 0.2, "{entry:?}");
    assert_eq!(copper.patterns.len(), 1);
    let meander = &copper.patterns[0];
    assert!((meander.added - entry.meander_mm).abs() < 1.0e-9);
    assert!(meander.amplitude <= 3.0 * 0.2 + 1.0e-9);
}

/// Without a crossover a crossed end's stubs cross on their way to the
/// run, which the router does with a via: the end's start keeps room
/// for it beyond the pads (`CROSSING_ROOM`), not the closest clear
/// start (pairs6: left_io's J22 launched 0.65 mm from its pads, the
/// stubs with no path in the negotiation).
#[test]
fn a_crossed_end_without_a_crossover_leaves_room_for_the_crossing() {
    let crossed = BOARD.replace(
        r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D+")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
    );
    let pcb = parse(&crossed).unwrap();
    let config = KiCadBoardRouterConfig {
        default_rules: Some(KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }),
        ..KiCadBoardRouterConfig::default()
    };
    CROSSOVERS.with(|cell| cell.set(Some(false)));
    let mut board = lower(&pcb, &config, true).unwrap().board;
    plan_pairs(&mut board, &config);
    CROSSOVERS.with(|cell| cell.set(None));
    let plan = board
        .attachment
        .data
        .clone()
        .unwrap()
        .downcast::<PairPlan>()
        .unwrap();
    let link_plan = plan
        .links
        .iter()
        .find(|link| link.crossed)
        .expect("a crossed link");
    let pair = &plan.pairs[link_plan.pair];
    let end = (0..2)
        .find(|&end| end_crossed(link_plan, end))
        .expect("a crossed end");
    assert!(!link_plan.fixed[end]);
    let outward = unit(sub(link_plan.launches[end], link_plan.starts[end]));
    let room = CROSSING_ROOM * (pair.via_diameter + 2.0 * pair.clearance + pair.width);
    for stub in link_plan.stubs[end] {
        let along = dot(sub(link_plan.starts[end], stub), outward);
        assert!(along >= room - 1.0e-9, "{along} from a pad, {room} wanted");
    }
}

/// A crossover is planned only where the link carries the stubs from
/// the pads to it: at U2, whose stubs a GND pad between the pads' exits
/// keeps from going straight, the router's stubs would have to reach
/// round the crossover's copper (pairs6: the ch32v003 board's D+ pad
/// walled in, its net open), so U2's end gets none.
#[test]
fn a_crossover_is_planned_only_where_the_link_carries_its_stubs() {
    let crossed = BOARD.replace(
        r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D+")))
  (footprint "test" (layer "F.Cu") (at 33.25 10.25)
    (property "Reference" "R9" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 0.15 0.15) (layers "F.Cu") (net "/GND")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
    );
    assert_ne!(crossed, BOARD);
    let pcb = parse(&crossed).unwrap();
    let config = KiCadBoardRouterConfig {
        default_rules: Some(KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }),
        ..KiCadBoardRouterConfig::default()
    };
    CROSSOVERS.with(|cell| cell.set(Some(true)));
    let mut board = lower(&pcb, &config, true).unwrap().board;
    plan_pairs(&mut board, &config);
    CROSSOVERS.with(|cell| cell.set(None));
    let plan = board
        .attachment
        .data
        .clone()
        .unwrap()
        .downcast::<PairPlan>()
        .unwrap();
    let link_plan = plan
        .links
        .iter()
        .find(|link| link.pads.iter().any(|pads| pads[0][0] > 33.0))
        .expect("a link to U2");
    assert!(link_plan.crossed);
    let u2 = (0..2)
        .find(|&end| link_plan.pads[end][0][0] > 33.0)
        .unwrap();
    assert!(!link_plan.fixed[u2], "U2's stubs are the router's");
    assert!(link_plan.crossovers[u2].is_none(), "a crossover at U2");
    for end in 0..2 {
        if link_plan.crossovers[end].is_some() {
            assert!(
                link_plan.fixed[end],
                "a crossover at end {end} without its stubs"
            );
        }
    }
}

/// A crossed pair whose link falls back (its run not routed, or its
/// copper refused) leaves no crossover copper: no via or track of the
/// crossover in the pairs' nets' routes or in the written pair copper,
/// nothing of the pair's nets dangling, and the fallback joins the pads.
#[test]
fn a_crossed_pair_that_falls_back_leaves_no_crossover_copper() {
    let crossed = BOARD.replace(
        r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D+")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
    );
    let pcb = parse(&crossed).unwrap();
    let config = KiCadBoardRouterConfig {
        default_rules: Some(KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }),
        ..KiCadBoardRouterConfig::default()
    };
    CROSSOVERS.with(|cell| cell.set(Some(true)));
    for how in ["not routed", "refused"] {
        let mut board = lower(&pcb, &config, true).unwrap().board;
        plan_pairs(&mut board, &config);
        let plan = board
            .attachment
            .data
            .clone()
            .unwrap()
            .downcast::<PairPlan>()
            .unwrap();
        let link_plan = plan.links[0].clone();
        let pair = plan.pairs[link_plan.pair].clone();
        let crossing = link_plan
            .crossovers
            .iter()
            .flatten()
            .next()
            .expect("a crossover")
            .clone();
        let mut result = core::route(&board, &core_config(&config));
        let link = link_plan.net as usize;
        assert_eq!(result.status[link], core::NetStatus::Routed);
        match how {
            "not routed" => {
                result.routes[link] = core::NetRoute::default();
                result.status[link] = core::NetStatus::Partial {
                    unconnected_terminals: 1,
                };
            }
            _ => {
                // Another net's track across the middle of the run, on
                // both layers: the copper check refuses the link.
                let segments = &result.routes[link].segments;
                let longest = segments
                    .iter()
                    .max_by(|a, b| {
                        core::geometry::distance(a.start, a.end)
                            .total_cmp(&core::geometry::distance(b.start, b.end))
                    })
                    .unwrap()
                    .clone();
                let middle = scale(add(longest.start, longest.end), 0.5);
                let across = left(unit(sub(longest.end, longest.start)));
                let ground = board
                    .nets
                    .iter()
                    .position(|net| net.name.trim_start_matches('/') == "GND")
                    .unwrap();
                for layer in 0..2 {
                    result.routes[ground].segments.push(core::Segment {
                        layer,
                        start: add(middle, scale(across, -1.0)),
                        end: add(middle, scale(across, 1.0)),
                        width: 0.2,
                    });
                }
            }
        }
        let (restored, copper, report) =
            finish_pairs(&board, &mut result, &config, &[0.0, 1.6]).unwrap();
        assert_eq!(report.links_coupled, 0, "{how}: {:?}", report.pairs);
        assert_eq!(report.crossovers, 0, "{how}");
        assert!(copper.items.is_empty(), "{how}: pair copper written");
        assert!(
            !restored
                .obstacles
                .iter()
                .any(|obstacle| obstacle.label.starts_with("pair ")),
            "{how}: the plan's copper stayed on the board"
        );
        let same = |a: Point, b: Point| core::geometry::distance(a, b) < 1.0e-6;
        for net in [pair.positive, pair.negative] {
            let route = &result.routes[net as usize];
            for via in &route.vias {
                assert!(
                    !crossing.vias.iter().flatten().any(|&at| same(at, via.at)),
                    "{how}: a crossover via stayed"
                );
            }
            for segment in &route.segments {
                assert!(
                    !crossing
                        .segments
                        .iter()
                        .flatten()
                        .any(|&(layer, from, to)| {
                            layer == segment.layer
                                && ((same(from, segment.start) && same(to, segment.end))
                                    || (same(to, segment.start) && same(from, segment.end)))
                        }),
                    "{how}: a crossover track stayed"
                );
            }
            // Nothing of the pair's nets joins nothing.
            let mut pruned = route.clone();
            prune_dangling(&restored, &mut pruned, net);
            assert_eq!(
                (pruned.segments.len(), pruned.vias.len()),
                (route.segments.len(), route.vias.len()),
                "{how}: copper of {} dangles",
                restored.nets[net as usize].name
            );
            assert_eq!(
                result.status[net as usize],
                core::NetStatus::Routed,
                "{how}: the fallback left {} open",
                restored.nets[net as usize].name
            );
        }
    }
    CROSSOVERS.with(|cell| cell.set(None));
}

/// The room a crossover leaves a neighbouring pair's launch: its
/// tracks, vias and the run's lead past it against the neighbour's
/// stubs and lead, each with its clearance; negative where it reaches
/// in (`launch` then skips that crossover), infinite with no neighbour.
#[test]
fn a_crossover_keeps_room_beside_a_neighbours_launch() {
    let (board, config) = lowered();
    let pair = find_pairs(&board, &config)
        .into_iter()
        .find(|pair| !pair.links.is_empty())
        .unwrap();
    let (w, c) = (pair.width, pair.clearance);
    // A crossover along x at y = 0: a track on F.Cu, a via at (1, 0.4).
    let found = Crossover {
        run_start: [2.0, 0.0],
        segments: [vec![(0, [0.0, 0.0], [2.0, 0.0])], Vec::new()],
        vias: [Vec::new(), vec![[1.0, 0.4]]],
    };
    let lead = ([2.0, 0.0], [3.0, 0.0], w);
    let neighbour_at = |y: f64| Neighbour {
        pair: 1,
        layers: 1,
        copper: [([0.0, y], [3.0, y], w / 2.0, c); 5],
    };
    assert_eq!(neighbour_room(&found, lead, 1, &pair, &[]), f64::INFINITY);
    // Far: the via's side at y = 0.4 plus its radius and the clearance.
    let need = (pair.via_diameter / 2.0 + c).max(pair.via_drill / 2.0 + pair.hole_clearance);
    let far = neighbour_at(0.4 + need + w / 2.0 + 0.1);
    let room = neighbour_room(&found, lead, 1, &pair, &[&far]);
    assert!((room - 0.1).abs() < 1.0e-9, "{room}");
    // Close: the neighbour's stub inside the via's clearance.
    let close = neighbour_at(0.4 + need + w / 2.0 - 0.05);
    assert!(neighbour_room(&found, lead, 1, &pair, &[&close]) < 0.0);
    // On another layer the tracks and the lead do not count, the via
    // (on every layer) does.
    let other = Neighbour {
        layers: 2,
        ..neighbour_at(-(w + c + 0.01))
    };
    assert!(neighbour_room(&found, lead, 1, &pair, &[&other]) > 0.0);
}

/// A via of a coupled run gets a return via of GND beside it where GND
/// has a pour there (on two layers), its ring within `RETURN_GAP` of the pair via's and
/// clear of the pair's copper; without a pour of GND, none.
#[test]
fn a_pair_via_gets_a_return_via_of_ground_beside_it() {
    let crossed = BOARD.replace(
        r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D+")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
    );
    let poured = crossed.replace(
        "(gr_rect (start 0 0)",
        r#"(zone (net "/GND") (net_name "/GND") (layers "F.Cu" "B.Cu") (hatch edge 0.5) (connect_pads (clearance 0.2)) (min_thickness 0.2) (fill yes (thermal_gap 0.2) (thermal_bridge_width 0.2)) (polygon (pts (xy 0.5 0.5) (xy 39.5 0.5) (xy 39.5 19.5) (xy 0.5 19.5))))
  (gr_rect (start 0 0)"#,
    );
    assert_ne!(poured, crossed);
    let config = KiCadBoardRouterConfig {
        default_rules: Some(KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }),
        ..KiCadBoardRouterConfig::default()
    };
    CROSSOVERS.with(|cell| cell.set(Some(true)));
    let routed = |text: &str| {
        let pcb = parse(text).unwrap();
        let mut board = lower(&pcb, &config, true).unwrap().board;
        plan_pairs(&mut board, &config);
        let mut result = core::route(&board, &core_config(&config));
        let (restored, _, report) =
            finish_pairs(&board, &mut result, &config, &[0.0, 1.6]).unwrap();
        (restored, result, report)
    };
    let (board, result, report) = routed(&poured);
    CROSSOVERS.with(|cell| cell.set(None));
    assert_eq!(report.links_coupled, 1, "{:?}", report.pairs);
    // The crossover's two vias of the diving net.
    assert_eq!(report.pair_vias, 2, "{report:?}");
    assert!(report.return_vias >= 1, "{report:?}");
    assert_eq!(report.return_net.trim_start_matches('/'), "GND");
    let ground = board
        .nets
        .iter()
        .position(|net| net.name == report.return_net)
        .unwrap();
    // They follow the router's own vias of GND (into its pour).
    let all = &result.routes[ground].vias;
    let returns = &all[all.len() - report.return_vias..];
    let pair_vias: Vec<Point> = board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.label == "existing via")
        .filter_map(|obstacle| match obstacle.shape {
            core::Shape::Circle { center, .. } => Some(center),
            _ => None,
        })
        .collect();
    for via in returns {
        let nearest = pair_vias
            .iter()
            .map(|at| core::geometry::distance(*at, via.at))
            .fold(f64::INFINITY, f64::min);
        assert!(nearest - 0.6 <= RETURN_GAP + 1.0e-9, "{nearest}");
        assert!(nearest - 0.6 >= 0.2 - 1.0e-9, "{nearest}");
    }
    assert!(core::verify(&board, &result.routes).is_empty());
    // No pour of GND: no return via.
    CROSSOVERS.with(|cell| cell.set(Some(true)));
    let (_, _, report) = routed(&crossed);
    CROSSOVERS.with(|cell| cell.set(None));
    assert_eq!(report.return_vias, 0);
    assert_eq!(report.pair_vias, 2);
}

/// The skew from the written copper: the tracks, and a via at the
/// stackup's height between the layers its tracks join.
#[test]
fn the_skew_counts_tracks_and_vias_at_the_stackups_height() {
    let pcb = parse(
        r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "core") (thickness 1.5))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (segment (start 0 0) (end 10 0) (width 0.2) (layer "F.Cu") (net "/D+"))
  (via (at 10 0) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "/D+"))
  (segment (start 10 0) (end 15 0) (width 0.2) (layer "B.Cu") (net "/D+"))
  (segment (start 0 0.4) (end 15 0.4) (width 0.2) (layer "F.Cu") (net "/D-")))"#,
    )
    .unwrap();
    // Production callers note the board's net names before measuring;
    // the test does the same (`normalize_net` asserts it in debug).
    crate::note_net_names(&pcb);
    // A via from F.Cu to B.Cu counts the whole stack, its copper
    // included (KiCad 10, `copper_heights`).
    let heights = copper_heights(&pcb);
    assert!(
        (heights["B.Cu"] - heights["F.Cu"] - 1.57).abs() < 1.0e-9,
        "{heights:?}"
    );
    let mut report = KiCadDiffPairs {
        pairs: vec![KiCadDiffPair {
            positive: "D+".into(),
            negative: "D-".into(),
            links: 1,
            links_coupled: 1,
            skew_target_mm: 0.2,
            ..KiCadDiffPair::default()
        }],
        ..KiCadDiffPairs::default()
    };
    measure_uncoupled(&mut report, &pcb);
    let skew = &report.skews[0];
    assert!((skew.lengths_mm[0] - 16.57).abs() < 1.0e-9, "{skew:?}");
    assert!((skew.lengths_mm[1] - 15.0).abs() < 1.0e-9, "{skew:?}");
    assert!((report.skew_mm - 1.57).abs() < 1.0e-9);
}

/// A fallen-back pair's leftovers: what joins nothing at one end goes
/// (a track to nowhere, a via on no copper), what joins the two pads
/// stays.
/// A link's route that turns back on itself (a hairpin) or jogs aside
/// for less than its offset: the offset tracks cross or near each
/// other there (`own_copper_clear` says so), the despiked centreline's
/// keep the pair's gap; `finish_pairs` takes the first clean one
/// (pairs5: 0.000 mm between a pair's own nets refused jetson's,
/// AMLing2's and the atmega board's runs).
#[test]
fn a_hairpin_in_a_links_route_is_straightened_before_its_tracks_are_offset() {
    let (board, config) = lowered();
    let pair = find_pairs(&board, &config).remove(0);
    let d = pair.offset();
    let path: Vec<(Point, usize)> = [
        [0.0, 0.0],
        [1.0, 0.0],
        [6.0, 0.0],
        // Back by 1 mm and aside: a hairpin.
        [5.0, 0.4],
        [5.0, 5.0],
        [5.0, 6.0],
    ]
    .into_iter()
    .map(|point| (point, 0))
    .collect();
    let crossing = link_copper(&path, &pair, 1.0, 1.0).unwrap();
    assert!(!own_copper_clear(&crossing, &pair));
    let straightened = despiked(&path, 2.5 * d);
    assert!(straightened.len() < path.len(), "{straightened:?}");
    let clean = link_copper(&straightened, &pair, 1.0, 1.0).unwrap();
    assert!(own_copper_clear(&clean, &pair));
    // A short jog aside (shorter than the offset) between two turns
    // the same way: its inner offset folds back toward the partner.
    let jog: Vec<(Point, usize)> = [
        [0.0, 0.0],
        [1.0, 0.0],
        [5.0, 0.0],
        [5.0 + 0.3 * d, 0.3 * d],
        [5.0 + 0.3 * d, 5.0],
        [5.0 + 0.3 * d, 6.0],
    ]
    .into_iter()
    .map(|point| (point, 0))
    .collect();
    let merged = despiked(&jog, 2.5 * d);
    let clean = link_copper(&merged, &pair, 1.0, 1.0).unwrap();
    assert!(own_copper_clear(&clean, &pair));
}

/// A link's route that changes layer in a turn: the vias stand on a
/// straight piece once the change moves back along the segment before
/// it (`vias_on_straights`), and the two nets keep their gap.
#[test]
fn a_links_layer_change_in_a_turn_moves_onto_a_straight_piece() {
    let (board, config) = lowered();
    let pair = find_pairs(&board, &config).remove(0);
    for turn in [[0.0, 1.0], [0.0, -1.0], [1.0, 1.0], [1.0, -1.0]] {
        let corner = [5.0, 0.0];
        let far = add(corner, scale(unit(turn), 3.0));
        let path: Vec<(Point, usize)> = vec![
            ([0.0, 0.0], 0),
            ([1.0, 0.0], 0),
            (corner, 0),
            (corner, 1),
            (far, 1),
            (add(far, scale(unit(turn), 1.0)), 1),
        ];
        let shift = 2.0 * pair.via_offset() + pair.width;
        let moved = vias_on_straights(&path, shift);
        assert_eq!(moved.len(), path.len() + 1, "{moved:?}");
        // The layer change is on the straight piece before the turn.
        assert!((moved[2].0[0] - (5.0 - shift)).abs() < 1.0e-9, "{moved:?}");
        assert_eq!((moved[2].1, moved[3].1, moved[4].1), (0, 1, 1));
        let copper = link_copper(&moved, &pair, 1.0, 1.0).unwrap();
        assert!(own_copper_clear(&copper, &pair), "turn {turn:?}");
        assert_eq!(copper.vias[0].len(), 1);
    }
}

/// The run of AMLing2's drone carrier's USB2 pair as the router routed
/// it (pairs5): a layer change followed by a 0.14 mm diagonal and a
/// turn. Offset as it is, the two nets' vias overlap; of the
/// centrelines `finish_pairs` tries one keeps the pair's gap.
#[test]
fn a_routed_run_with_a_via_before_a_short_diagonal_gets_a_clean_centreline() {
    let (board, config) = lowered();
    let mut pair = find_pairs(&board, &config).remove(0);
    (pair.width, pair.gap, pair.clearance) = (0.21, 0.13, 0.149);
    (pair.via_diameter, pair.via_drill) = (0.45, 0.25);
    let path: Vec<(Point, usize)> = [
        ([158.033, 121.016], 0),
        ([157.818, 120.644], 0),
        ([157.498, 120.2], 0),
        ([157.198, 119.9], 0),
        ([157.198, 118.9], 0),
        ([160.398, 118.9], 0),
        ([160.398, 118.9], 1),
        ([160.498, 119.0], 1),
        ([160.498, 127.8], 1),
        ([159.598, 128.7], 1),
        ([159.598, 132.8], 1),
        ([160.398, 133.6], 1),
        ([160.398, 133.6], 0),
        ([161.298, 134.5], 0),
        ([164.498, 134.5], 0),
        ([165.498, 135.5], 0),
        ([165.608, 135.939], 0),
        ([165.608, 136.37], 0),
    ]
    .into_iter()
    .collect();
    let raw = link_copper(&path, &pair, -1.0, 1.0).unwrap();
    assert!(!own_copper_clear(&raw, &pair));
    let clean: Vec<Vec<(Point, usize)>> = centrelines(&path, &pair)
        .into_iter()
        .filter(|line| {
            link_copper(line, &pair, -1.0, 1.0)
                .is_some_and(|copper| own_copper_clear(&copper, &pair))
        })
        .collect();
    assert!(!clean.is_empty());
    // The layer changes stay two.
    for line in &clean {
        let changes = line
            .windows(2)
            .filter(|step| step[0].1 != step[1].1)
            .count();
        assert_eq!(changes, 2, "{line:?}");
    }
}

/// A stub that reaches the run through another pad of its net on the
/// way (an ESD diode's pin between the connector and the run: right_io's
/// ETH_MDI pairs through U501 and U502) reaches it; through the pad of
/// the link's other end it does not.
#[test]
fn a_stub_through_another_pad_of_its_net_reaches_the_run() {
    let pad = |x: f64| core::Obstacle {
        shape: core::Shape::Circle {
            center: [x, 0.0],
            radius: 0.3,
        },
        layers: 0b11,
        kind: core::ObstacleKind::Copper,
        net: Some(0),
        clearance: 0.0,
        clearance_override: None,
        plated: false,
        blocks_tracks: true,
        blocks_vias: true,
        label: "pad".into(),
    };
    let mut board = lowered().0;
    // The pad, an ESD pin on the way, the link's other end's pad.
    board.obstacles = vec![pad(0.0), pad(2.0), pad(9.0)];
    let segment = |start: Point, end: Point| core::Segment {
        layer: 0,
        start,
        end,
        width: 0.2,
    };
    let route = core::NetRoute {
        segments: vec![
            segment([0.0, 0.0], [1.8, 0.0]),
            segment([2.2, 0.0], [4.0, 0.0]),
        ],
        vias: Vec::new(),
    };
    let lead = ([4.0, 0.0], [4.2, 0.0]);
    assert!(stub_joined(
        &board,
        &route,
        0,
        ([0.0, 0.0], [9.0, 0.0]),
        lead,
        0.2
    ));
    // Without the ESD pin's copper the two tracks do not meet.
    board.obstacles.remove(1);
    assert!(!stub_joined(
        &board,
        &route,
        0,
        ([0.0, 0.0], [9.0, 0.0]),
        lead,
        0.2
    ));
    // Through the other end's pad: not this end's stub.
    board.obstacles = vec![pad(0.0), pad(2.0)];
    assert!(!stub_joined(
        &board,
        &route,
        0,
        ([0.0, 0.0], [2.0, 0.0]),
        lead,
        0.2
    ));
}

/// A link whose copper holds but whose pad's stub the attempt left out
/// (the router joined the pad another way, or left it open): the stub
/// is routed after the attempt from the pad to its end of the run, the
/// link is kept, and the pad's old way, now a loop, goes.
#[test]
fn a_stub_the_attempt_left_out_is_routed_after_it() {
    let (mut board, config) = lowered();
    plan_pairs(&mut board, &config);
    let plan = plan_of(&board).unwrap();
    // The test board's link carries straight stubs at both ends: make
    // the second end's the router's.
    let mut link = plan.links[0].clone();
    assert_eq!(link.fixed, [true, true]);
    let mut result = core::route(&board, &core_config(&config));
    let (_, _, before) = finish_pairs(&board, &mut result.clone(), &config, &[0.0, 1.6]).unwrap();
    assert_eq!(before.links_coupled, 1);
    // The pads' stubs at the second end, dropped from a routed result:
    // a route from the far pad straight to the first end's pad instead.
    link.fixed = [true, false];
    let pair = &plan.pairs[link.pair];
    let mut changed = (*plan).clone();
    changed.links[0] = link.clone();
    let mut board = board.clone();
    board.attachment.data = Some(std::sync::Arc::new(changed));
    // D+ round below the board's middle, D- round above it.
    let detours: Vec<Vec<Point>> = (0..2)
        .map(|polarity| {
            let [a, b] = [link.pads[0][polarity], link.pads[1][polarity]];
            let (x, y) = if polarity == 0 {
                (35.0, 7.0)
            } else {
                (35.5, 14.0)
            };
            let left = if polarity == 0 { a[0] } else { 2.0 };
            vec![b, [x, b[1]], [x, y], [left, y], [left, a[1]], a]
        })
        .collect();
    for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
        result.routes[net as usize].segments = detours[polarity]
            .windows(2)
            .filter(|step| core::geometry::distance(step[0], step[1]) > 1.0e-9)
            .map(|step| core::Segment {
                layer: 0,
                start: step[0],
                end: step[1],
                width: 0.2,
            })
            .collect();
        result.routes[net as usize].vias.clear();
    }
    let (_, _, report) = finish_pairs(&board, &mut result, &config, &[0.0, 1.6]).unwrap();
    assert_eq!(report.links_coupled, 1, "{:?}", report.pairs);
    assert_eq!(report.stubs_routed, 2, "{:?}", report.pairs);
    // The old way round is gone: each net's tracks end at the run.
    for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
        let far = detours[polarity][2];
        assert!(
            !result.routes[net as usize]
                .segments
                .iter()
                .any(
                    |segment| core::geometry::distance(segment.start, far) < 1.0e-6
                        || core::geometry::distance(segment.end, far) < 1.0e-6
                ),
            "{:?}",
            result.routes[net as usize].segments
        );
    }
}

/// Two links' copper that meet: both are refused, but where one lies
/// on the router's route and the other was moved off it
/// (`centrelines`), the moved one only (pairs5: on jetson a link's
/// moved via took a coupled neighbour down with it).
#[test]
fn copper_moved_off_the_route_is_refused_alone_where_it_meets_a_link_on_its_route() {
    let (board, config) = lowered();
    let pairs = find_pairs(&board, &config);
    let group = |segments: [Vec<(usize, Point, Point)>; 2], moved: bool| Group {
        pair: 0,
        segments,
        vias: Default::default(),
        run: [0..1, 0..1],
        moved,
    };
    let routes = vec![core::NetRoute::default(); board.nets.len()];
    let across = || [vec![(0, [20.0, 15.0], [24.0, 15.0])], Vec::new()];
    let along = || [Vec::new(), vec![(0, [22.0, 13.0], [22.0, 17.0])]];
    for (moved, refused) in [
        ([false, false], [true, true]),
        ([true, false], [true, false]),
        ([false, true], [false, true]),
        ([true, true], [true, true]),
    ] {
        let groups = [group(across(), moved[0]), group(along(), moved[1])];
        let why = refused_groups(&board, &routes, &pairs, &groups);
        assert_eq!(
            why.iter().map(Option::is_some).collect::<Vec<_>>(),
            refused,
            "{moved:?}: {why:?}"
        );
    }
}

#[test]
fn what_joins_nothing_is_pruned() {
    let pad = |x: f64| core::Obstacle {
        shape: core::Shape::Circle {
            center: [x, 0.0],
            radius: 0.3,
        },
        layers: 0b11,
        kind: core::ObstacleKind::Copper,
        net: Some(0),
        clearance: 0.0,
        clearance_override: None,
        plated: false,
        blocks_tracks: true,
        blocks_vias: true,
        label: "pad".into(),
    };
    let mut board = lowered().0;
    board.obstacles = vec![pad(0.0), pad(10.0)];
    let segment = |layer: usize, start: Point, end: Point| core::Segment {
        layer,
        start,
        end,
        width: 0.2,
    };
    let mut route = core::NetRoute {
        segments: vec![
            segment(0, [0.0, 0.0], [5.0, 0.0]),
            segment(0, [5.0, 0.0], [10.0, 0.0]),
            // A spur off the middle, to a via, on to nothing on B.Cu.
            segment(0, [5.0, 0.0], [5.0, 2.0]),
            segment(1, [5.0, 2.0], [7.0, 2.0]),
            // A short piece at the pad's edge, its far end loose.
            segment(0, [10.0, 0.0], [10.0, 0.6]),
        ],
        vias: vec![
            core::Via {
                at: [5.0, 2.0],
                diameter: 0.6,
                drill: 0.3,
                span: None,
            },
            // A via that joins nothing.
            core::Via {
                at: [8.0, 5.0],
                diameter: 0.6,
                drill: 0.3,
                span: None,
            },
        ],
    };
    prune_dangling(&board, &mut route, 0);
    // The B.Cu track, the lone via and the piece at the pad go; a via
    // with copper on one layer stays (a zone or a teardrop may hold it),
    // and so the spur to it.
    assert_eq!(route.segments.len(), 3, "{:?}", route.segments);
    assert_eq!(route.vias.len(), 1);
    assert!(route.segments.iter().all(|segment| segment.layer == 0));
    // A short piece whose end only overlaps the next track (KiCad: that
    // end dangles) stays where it joins two tracks, goes where it is a
    // nub off a track.
    let mut route = core::NetRoute {
        segments: vec![
            segment(0, [0.0, 0.0], [5.0, 0.0]),
            segment(0, [5.0, 0.0], [5.15, 0.0]),
            segment(0, [5.3, 0.0], [10.0, 0.0]),
            segment(0, [2.0, 0.0], [2.0, 0.15]),
        ],
        vias: Vec::new(),
    };
    prune_dangling(&board, &mut route, 0);
    assert_eq!(route.segments.len(), 3, "{:?}", route.segments);
    assert!(route.segments.iter().all(|segment| segment.end[1] == 0.0));
    assert!(
        route
            .segments
            .iter()
            .all(|segment| segment.layer == 0 && segment.start[1] == 0.0)
    );
}

/// Why a link was not routed comes from the router's diagnostics, the
/// negotiation's give-up first, then a dead launch, then the hard repair's
/// record (`not_routed_cause`).
#[test]
fn a_links_cause_not_routed_is_read_from_the_diagnostics() {
    let entry = |skipped: &'static str, complete: bool, conflicted: bool| core::RepairEntry {
        net: 7,
        complete,
        conflicted,
        skipped,
    };
    let mut diagnostics = core::Diagnostics::default();
    assert_eq!(not_routed_cause(&diagnostics, 7), "lost after the repair");
    for (repair, cause) in [
        (
            entry("hopeless", false, false),
            "no hard repair: the attempt was judged hopeless",
        ),
        (
            entry("budget", false, true),
            "the hard repair's budget ran out before it",
        ),
        (entry("", false, false), "no path in the negotiation"),
        (entry("", true, true), "the hard repair found no room"),
    ] {
        diagnostics.repair = vec![repair];
        assert_eq!(not_routed_cause(&diagnostics, 7), cause);
    }
    diagnostics.dead_pads.push(core::DeadPad {
        net: 7,
        label: "pair launch 1a".into(),
        anchor: [0.0, 0.0],
        near: Vec::new(),
        wall: String::new(),
    });
    assert_eq!(
        not_routed_cause(&diagnostics, 7),
        "a launch has no legal node"
    );
    diagnostics.released.push(core::router::ReleasedNet {
        net: 7,
        reason: core::router::NO_PATH,
        iteration: 3,
        work_seconds: 0.0,
    });
    assert_eq!(
        not_routed_cause(&diagnostics, 7),
        "given up in the negotiation: no path"
    );
    // Another net's record is not the link's.
    assert_eq!(not_routed_cause(&diagnostics, 8), "lost after the repair");
}

/// A pair rule's first allowed layer sets the pair's width and gap; the
/// layers with the same values (and allowed) are the pair's; without a
/// rule the net class's width and the pair's clearance hold everywhere.
#[test]
fn a_rule_sets_a_pairs_width_gap_and_layers_from_its_first_allowed_layer() {
    let (board, mut config) = lowered();
    let all = board.all_layers();
    let (width, gap, layers, source) = pair_geometry(&board, &config, "/D+", 0.2, 0.25);
    assert_eq!((width, gap, layers), (0.2, 0.25, all));
    assert_eq!(source, "net class width and clearance");
    config.diff_pair_rules.insert(
        "/D+".into(),
        KiCadDiffPairRule {
            layers: vec![(0.1, 0.1), (0.15, 0.12)],
            disallowed: vec![true, false],
            sources: vec!["rule 'top'".into(), "rule 'bottom'".into()],
            ..KiCadDiffPairRule::default()
        },
    );
    let (width, gap, layers, source) = pair_geometry(&board, &config, "/D+", 0.2, 0.25);
    assert_eq!((width, gap, layers), (0.15, 0.12, 0b10 & all));
    assert_eq!(source, "rule 'bottom'");
}

/// The skew's meander adds what is needed: as few bumps as hold the
/// length at most the cap high, as high as needed, `2 n - 1` spacings
/// long.
#[test]
fn a_meander_shape_adds_what_is_needed_with_bumps_no_higher_than_the_cap() {
    let (board, config) = lowered();
    let pair = find_pairs(&board, &config).remove(0);
    let (cap, spacing) = (0.6, 0.4);
    let added = |shape: &MeanderShape| {
        shape.count as f64
            * (polyline_length(&bumps(
                ([0.0, 0.0], [1.0, 0.0]),
                [0.0, 1.0],
                0.0,
                1,
                shape.height,
                spacing,
            )) - spacing)
    };
    for need in [0.1, 1.0, 5.0] {
        let shape = MeanderShape::of(need, cap, spacing, &pair);
        assert!(shape.height <= cap + 1.0e-12, "{need}: {}", shape.height);
        assert!(
            (added(&shape) - need).abs() < 1.0e-6,
            "{need}: {}",
            added(&shape)
        );
        assert!((shape.span - (2 * shape.count - 1) as f64 * spacing).abs() < 1.0e-12);
        assert!(shape.margin >= spacing && shape.margin >= 2.0 * pair.offset());
    }
    // One bump while it holds; more where one at the cap does not.
    assert_eq!(MeanderShape::of(0.1, cap, spacing, &pair).count, 1);
    assert!(MeanderShape::of(5.0, cap, spacing, &pair).count > 1);
}

/// A routed link's run starts at the run's start (the stubs' ends beside
/// it) and runs half the lead straight along the launch's direction; the
/// route's own stub from the launch goes.
#[test]
fn a_links_run_path_starts_and_ends_beside_the_stubs_ends() {
    let (mut board, config) = lowered();
    plan_pairs(&mut board, &config);
    let plan = plan_of(&board).unwrap();
    let link = &plan.links[0];
    let [at_a, at_b] = link.launches;
    let [start_a, start_b] = link.starts;
    let along = |share: f64| add(at_a, scale(sub(at_b, at_a), share));
    let (first, second) = (along(1.0 / 3.0), along(2.0 / 3.0));
    let segment = |start: Point, end: Point| core::Segment {
        layer: 0,
        start,
        end,
        width: 0.6,
    };
    let route = core::NetRoute {
        segments: vec![
            segment(at_a, first),
            segment(first, second),
            segment(second, at_b),
        ],
        vias: Vec::new(),
    };
    let path = run_path(&route, link).expect("a path");
    let points: Vec<Point> = path.iter().map(|(point, _)| *point).collect();
    let half = |start: Point, at: Point| scale(add(start, at), 0.5);
    assert_eq!(
        points,
        [
            start_a,
            half(start_a, at_a),
            first,
            second,
            half(start_b, at_b),
            start_b
        ]
    );
    assert!(path.iter().all(|(_, layer)| *layer == 0));
}

/// A stub's repair routes from the pad's terminal, or from the pad merged
/// into another terminal (its own terminal there).
#[test]
fn a_merged_pad_is_its_own_terminal_for_a_stub_repair() {
    let (mut board, _) = lowered();
    let net = board
        .nets
        .iter()
        .position(|net| net.name.trim_start_matches('/') == "D+")
        .unwrap() as core::NetId;
    let terminal = board.nets[net as usize].terminals[1].clone();
    let found = pad_terminal(&board, net, terminal.anchor).expect("its terminal");
    assert_eq!(found.label, terminal.label);
    assert!(pad_terminal(&board, net, [100.0, 100.0]).is_none());
    board.nets[net as usize].terminals.remove(1);
    board.merged_pads.push(core::MergedPad {
        net,
        host: 0,
        pad: terminal.pad,
        anchor: terminal.anchor,
        layers: terminal.layers,
        padstack: false,
    });
    let merged = pad_terminal(&board, net, terminal.anchor).expect("the merged pad");
    assert_eq!(merged.pad, terminal.pad);
    assert_eq!(merged.anchor, terminal.anchor);
    assert_eq!(merged.label, board.obstacles[terminal.pad].label);
    assert!(merged.contact.is_none());
}

/// A failed link's stub is pruned from its end along the tracks that join
/// nothing else, through a via that joins one track on another layer.
#[test]
fn a_failed_stub_is_pruned_through_its_via() {
    let mut board = lowered().0;
    board.obstacles.clear();
    let segment = |layer: usize, start: Point, end: Point| core::Segment {
        layer,
        start,
        end,
        width: 0.2,
    };
    let via = core::Via {
        at: [1.0, 0.0],
        diameter: 0.6,
        drill: 0.3,
        span: None,
    };
    let mut route = core::NetRoute {
        segments: vec![
            segment(0, [0.0, 0.0], [1.0, 0.0]),
            segment(1, [1.0, 0.0], [2.0, 0.0]),
        ],
        vias: vec![via.clone()],
    };
    prune_stub(&board, &mut route, 0, ([0.0, 0.0], [0.2, 0.0]), 0.2);
    assert!(route.segments.is_empty(), "{:?}", route.segments);
    assert!(route.vias.is_empty());
    // A via that joins two tracks on the other layer stays, and the chain
    // stops there.
    let mut route = core::NetRoute {
        segments: vec![
            segment(0, [0.0, 0.0], [1.0, 0.0]),
            segment(1, [1.0, 0.0], [2.0, 0.0]),
            segment(1, [1.0, 0.0], [1.0, 1.0]),
        ],
        vias: vec![via],
    };
    prune_stub(&board, &mut route, 0, ([0.0, 0.0], [0.2, 0.0]), 0.2);
    assert_eq!(route.segments.len(), 2);
    assert_eq!(route.vias.len(), 1);
}

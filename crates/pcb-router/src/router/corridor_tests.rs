// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A pad that leaves its part only through a corridor between the part's
//! own pads (docs/router.md, "Corridors of pocketed pads"): OpenFC's U10,
//! an X2SON-6 with custom pads, whose +3.3V pad 5 sits in a pocket
//! between the corner pads; the way out is a 0.3 mm gap that the class's
//! 0.09 mm track with 0.09 mm clearance passes only within 15 um of its
//! centre line.

use super::*;
use crate::board::{Net, Obstacle, ObstacleKind, RuleClass, Terminal};
use crate::geometry::{Point, Shape};

/// A custom pad as pcb-kicad lowers it: its anchor (a 0.01 mm disc, the
/// only place KiCad joins it, `Terminal::contact`), its polygon and the
/// polygon's 0.1 mm stroke as one capsule per edge.
fn custom_pad(at: Point, polygon: &[Point], net: NetId, label: &str) -> (Obstacle, Shape) {
    let points: Vec<Point> = polygon
        .iter()
        .map(|point| [at[0] + point[0], at[1] + point[1]])
        .collect();
    let anchor = Shape::Circle {
        center: at,
        radius: 0.005,
    };
    let mut parts = vec![
        anchor.clone(),
        Shape::Polygon {
            points: points.clone(),
        },
    ];
    for index in 0..points.len() {
        parts.push(Shape::Capsule {
            start: points[index],
            end: points[(index + 1) % points.len()],
            radius: 0.05,
        });
    }
    (
        Obstacle {
            shape: Shape::Union { parts },
            layers: 1,
            kind: ObstacleKind::Copper,
            net: Some(net),
            clearance: 0.0,
            clearance_override: None,
            plated: false,
            blocks_tracks: true,
            blocks_vias: true,
            label: label.into(),
        },
        anchor,
    )
}

/// One layer, 0.09 mm tracks and clearance. U10 (OpenFC's
/// X2SON-6_L1.0-W0.8 at orientation 0, its pads and their primitives
/// as in the board file) at (5, 5 + `dy`); its pad 5 joins P1 at (2, 5),
/// the other pads are single-pad nets. A row of ten narrow pads at y = 2
/// (0.2 mm across y, on the 0.1 mm lattice's phase 0) outvotes U10's
/// anchors for the lattice's phase across y, so that `dy` moves U10's
/// corridors against the lattice: they run along x at y = 5 + `dy`
/// (between pads 3 and 4, 15 um of play) and 5.015 + `dy` (pads 1 and 6,
/// 30 um).
pub(super) fn pocketed_part(dy: f64) -> Board {
    let class = RuleClass {
        trace_width: 0.09,
        clearance: 0.09,
        via_diameter: 0.35,
        via_drill: 0.2,
        layers: 0,
    };
    let centre = [5.0, 5.0 + dy];
    let at = |x: f64, y: f64| [centre[0] + x, centre[1] + y];
    // (number, position, polygon), from the footprint.
    let pads: [(&str, Point, Vec<Point>); 6] = [
        (
            "1",
            at(-0.35, -0.35),
            vec![
                [-0.1, -0.15],
                [0.16, -0.15],
                [0.16, -0.08],
                [-0.07, 0.15],
                [-0.1, 0.15],
            ],
        ),
        (
            "2",
            at(0.0, -0.22),
            vec![[-0.15, 0.12], [0.15, 0.12], [0.0, -0.13]],
        ),
        (
            "3",
            at(0.35, -0.35),
            vec![
                [0.1, -0.15],
                [0.1, 0.15],
                [0.05, 0.15],
                [-0.13, -0.03],
                [-0.13, -0.15],
            ],
        ),
        (
            "4",
            at(0.35, 0.35),
            vec![
                [0.1, 0.15],
                [-0.2, 0.15],
                [-0.2, 0.1],
                [0.05, -0.15],
                [0.1, -0.15],
            ],
        ),
        (
            "5",
            at(0.0, 0.22),
            vec![[-0.15, -0.13], [0.15, -0.13], [0.0, 0.12]],
        ),
        (
            "6",
            at(-0.35, 0.35),
            vec![
                [-0.1, 0.15],
                [0.2, 0.15],
                [0.2, 0.12],
                [-0.04, -0.12],
                [-0.1, -0.12],
            ],
        ),
    ];
    let mut obstacles = Vec::new();
    let mut nets = vec![Net {
        name: "+3.3V".into(),
        class: 0,
        terminals: Vec::new(),
    }];
    for (number, position, polygon) in &pads {
        let net = if *number == "5" {
            0
        } else {
            nets.push(Net {
                name: format!("U10-{number}"),
                class: 0,
                terminals: Vec::new(),
            });
            nets.len() as NetId - 1
        };
        let label = format!("U10.{number}");
        let (obstacle, anchor) = custom_pad(*position, polygon, net, &label);
        obstacles.push(obstacle);
        nets[net as usize].terminals.push(Terminal {
            anchor: *position,
            layers: 1,
            pad: obstacles.len() - 1,
            contact: Some(anchor),
            label,
        });
    }
    let plain = |center: Point, half: [f64; 2], net: NetId, label: String| Obstacle {
        shape: Shape::rectangle(center, half, 0.0),
        layers: 1,
        kind: ObstacleKind::Copper,
        net: Some(net),
        clearance: 0.0,
        clearance_override: None,
        plated: false,
        blocks_tracks: true,
        blocks_vias: true,
        label,
    };
    obstacles.push(plain([2.0, 5.0], [0.3, 0.3], 0, "P1.1".into()));
    nets[0].terminals.push(Terminal {
        anchor: [2.0, 5.0],
        layers: 1,
        pad: obstacles.len() - 1,
        contact: None,
        label: "P1.1".into(),
    });
    for index in 0..10 {
        let center = [1.5 + 0.8 * index as f64, 2.0];
        let label = format!("J1.{}", index + 1);
        nets.push(Net {
            name: format!("J1-{index}"),
            class: 0,
            terminals: Vec::new(),
        });
        let net = nets.len() as NetId - 1;
        obstacles.push(plain(center, [0.3, 0.1], net, label.clone()));
        nets[net as usize].terminals.push(Terminal {
            anchor: center,
            layers: 1,
            pad: obstacles.len() - 1,
            contact: None,
            label,
        });
    }
    Board {
        layer_count: 1,
        outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        edge_clearance: 0.1,
        hole_clearance: 0.1,
        hole_to_hole: 0.1,
        classes: vec![class],
        neck_width: 0.09,
        obstacles,
        nets,
        planes: Vec::new(),
        solid_pads: Vec::new(),
        isolated_pads: Vec::new(),
        solid_pad_obstacles: Vec::new(),
        isolated_pad_obstacles: Vec::new(),
        pad_spokes: Default::default(),
        merged_pads: Vec::new(),
        no_via_pads: Vec::new(),
        fill_passes: Vec::new(),
        pin_row_gaps: Vec::new(),
        terminal_copper: Vec::new(),
        attachment: Default::default(),
        via_spans: Vec::new(),
    }
}

#[test]
fn a_pocketed_pad_routes_on_every_lattice_offset() {
    // With corridor stubs U10.5 connects whatever the part's offset
    // against the lattice; without them only where a node row falls in a
    // corridor and the pocket's nodes reach it.
    let open_at = |corridor_escapes: bool| {
        let config = Config {
            pitches: vec![0.1],
            corridor_escapes,
            ..Config::default()
        };
        let mut open = Vec::new();
        for step in 0..10 {
            let dy = step as f64 * 0.01;
            let board = pocketed_part(dy);
            let result = crate::route(&board, &config);
            let violations = crate::verify(&board, &result.routes);
            assert!(violations.is_empty(), "dy {dy}: {violations:?}");
            if result.status[0] != NetStatus::Routed {
                open.push((dy, result.grid.origin));
            }
        }
        open
    };
    let open = open_at(true);
    assert!(
        open.is_empty(),
        "U10.5 open at (dy, lattice origin) {open:?}"
    );
    assert!(!open_at(false).is_empty());
}

/// Hub's X2SON-4 regulator (PCM_Package_SON_AKL:Texas_X2SON-4_1x1mm_P0.65mm,
/// U2, U3 and U6 on its board) at `centre`, in its own frame: four custom
/// corner pads, each a 0.148 mm anchor at (+-0.43, +-0.325) and a 0.46 x
/// 0.31 mm polygon whose corner towards the middle is cut to 0.12 mm from
/// the centre pad, and the centre pad (a 0.58 mm square turned by 45
/// degrees) of GND with pad 2. One layer, the board's Default class (0.2
/// mm tracks and clearance) with a 0.127 mm neck. Each corner pad's net
/// joins a plain pad 2 mm out along its diagonal. The anchors' centres lie
/// 0.244 mm from the centre pad: within the clearance plus half a neck
/// (0.2635 mm), so no track may end there.
pub(super) fn x2son4(centre: Point) -> Board {
    let class = RuleClass {
        trace_width: 0.2,
        clearance: 0.2,
        via_diameter: 0.6,
        via_drill: 0.3,
        layers: 0,
    };
    let at = |x: f64, y: f64| [centre[0] + x, centre[1] + y];
    // (number, anchor, polygon around the anchor), from the board file.
    let pads: [(&str, Point, Vec<Point>); 4] = [
        (
            "1",
            [-0.43, 0.325],
            vec![
                [0.23, 0.054289],
                [0.020711, -0.155],
                [-0.23, -0.155],
                [-0.23, 0.155],
                [0.23, 0.155],
            ],
        ),
        (
            "2",
            [-0.43, -0.325],
            vec![
                [0.23, -0.054289],
                [0.23, -0.155],
                [-0.23, -0.155],
                [-0.23, 0.155],
                [0.020711, 0.155],
            ],
        ),
        (
            "3",
            [0.43, -0.325],
            vec![
                [0.23, -0.155],
                [-0.23, -0.155],
                [-0.23, -0.054289],
                [-0.020711, 0.155],
                [0.23, 0.155],
            ],
        ),
        (
            "4",
            [0.43, 0.325],
            vec![
                [0.23, -0.155],
                [-0.020711, -0.155],
                [-0.23, 0.054289],
                [-0.23, 0.155],
                [0.23, 0.155],
            ],
        ),
    ];
    let mut obstacles = Vec::new();
    let mut nets: Vec<Net> = ["+3V3", "GND", "EN", "+5V"]
        .iter()
        .map(|name| Net {
            name: (*name).into(),
            class: 0,
            terminals: Vec::new(),
        })
        .collect();
    let copper = |shape: Shape, net: NetId, label: String| Obstacle {
        shape,
        layers: 1,
        kind: ObstacleKind::Copper,
        net: Some(net),
        clearance: 0.0,
        clearance_override: None,
        plated: false,
        blocks_tracks: true,
        blocks_vias: true,
        label,
    };
    for (index, (number, anchor, polygon)) in pads.iter().enumerate() {
        let net = index as NetId;
        let position = at(anchor[0], anchor[1]);
        let contact = Shape::Circle {
            center: position,
            radius: 0.074246,
        };
        let shape = Shape::Union {
            parts: vec![
                contact.clone(),
                Shape::Polygon {
                    points: polygon
                        .iter()
                        .map(|point| [position[0] + point[0], position[1] + point[1]])
                        .collect(),
                },
            ],
        };
        let label = format!("U2.{number}");
        obstacles.push(copper(shape, net, label.clone()));
        nets[index].terminals.push(Terminal {
            anchor: position,
            layers: 1,
            pad: obstacles.len() - 1,
            contact: Some(contact),
            label,
        });
        // Its partner, 2 mm out along the diagonal.
        let out = [
            position[0] + 2.0 * anchor[0].signum(),
            position[1] + 2.0 * anchor[1].signum(),
        ];
        let label = format!("P{number}.1");
        obstacles.push(copper(
            Shape::rectangle(out, [0.3, 0.3], 0.0),
            net,
            label.clone(),
        ));
        nets[index].terminals.push(Terminal {
            anchor: out,
            layers: 1,
            pad: obstacles.len() - 1,
            contact: None,
            label,
        });
    }
    obstacles.push(copper(
        Shape::Polygon {
            points: vec![at(0.41, 0.0), at(0.0, 0.41), at(-0.41, 0.0), at(0.0, -0.41)],
        },
        1,
        "U2.5".into(),
    ));
    nets[1].terminals.push(Terminal {
        anchor: centre,
        layers: 1,
        pad: obstacles.len() - 1,
        contact: None,
        label: "U2.5".into(),
    });
    Board {
        layer_count: 1,
        outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        edge_clearance: 0.1,
        hole_clearance: 0.1,
        hole_to_hole: 0.1,
        classes: vec![class],
        neck_width: 0.127,
        obstacles,
        nets,
        planes: Vec::new(),
        solid_pads: Vec::new(),
        isolated_pads: Vec::new(),
        solid_pad_obstacles: Vec::new(),
        isolated_pad_obstacles: Vec::new(),
        pad_spokes: Default::default(),
        merged_pads: Vec::new(),
        no_via_pads: Vec::new(),
        fill_passes: Vec::new(),
        pin_row_gaps: Vec::new(),
        terminal_copper: Vec::new(),
        attachment: Default::default(),
        via_spans: Vec::new(),
    }
}

#[test]
fn a_custom_pad_whose_anchor_lies_in_a_neighbours_clearance_leaves_from_its_far_side() {
    // Hub's X2SON-4 corner pads: their anchors' centres lie within the
    // centre pad's clearance, so the escape stubs start on the anchor's
    // far side, where the designer's tracks end. Every offset of the part
    // against the 0.1 mm lattice.
    let config = Config {
        pitches: vec![0.1],
        ..Config::default()
    };
    for step in 0..5 {
        let offset = step as f64 * 0.023;
        let board = x2son4([5.0 + offset, 5.0 + offset / 2.0]);
        let result = crate::route(&board, &config);
        let violations = crate::verify(&board, &result.routes);
        assert!(violations.is_empty(), "offset {offset}: {violations:?}");
        for (net, status) in result.status.iter().enumerate() {
            assert_eq!(
                *status,
                NetStatus::Routed,
                "offset {offset}: {} open",
                board.nets[net].name
            );
        }
    }
}

/// WLED_Matrix's LDO1 pad 2 (a SOT-23's 1.07 x 0.6 mm pad, net Vin) at
/// (5, 5), with a second SOT-23 of the same reference and no nets over it:
/// its pad 3 at (5.015, 5.23), the same size, overlaps all of pad 2 but a
/// 0.23 mm strip along pad 2's top edge. One layer, 0.127 mm tracks and
/// clearance (the board's). Vin joins a plain pad at (7, 3.5). The pad's
/// centre lies inside the other pad; a track may end only in the strip's
/// top 40 um (`cover`: the no-net pad's offset along y, 0.23 mm; at 0.6
/// it covers the whole pad). Everything moved by `offset`.
fn overlapped_pad(cover: f64, offset: Point) -> Board {
    let at = |x: f64, y: f64| [x + offset[0], y + offset[1]];
    let class = RuleClass {
        trace_width: 0.127,
        clearance: 0.127,
        via_diameter: 0.6,
        via_drill: 0.4,
        layers: 0,
    };
    let copper = |shape: Shape, net: Option<NetId>, label: &str| Obstacle {
        shape,
        layers: 1,
        kind: ObstacleKind::Copper,
        net,
        clearance: 0.0,
        clearance_override: None,
        plated: false,
        blocks_tracks: true,
        blocks_vias: true,
        label: label.into(),
    };
    let obstacles = vec![
        copper(
            Shape::rectangle(at(5.0, 5.0), [1.07, 0.6], 0.0),
            Some(0),
            "LDO1.2",
        ),
        copper(
            Shape::rectangle(at(7.0, 3.5), [0.6, 0.6], 0.0),
            Some(0),
            "C7.1",
        ),
        copper(
            Shape::rectangle(at(5.015, 5.0 + cover), [1.07, 0.6], 0.0),
            None,
            "LDO1.3",
        ),
    ];
    let terminal = |anchor: Point, pad: usize, label: &str| Terminal {
        anchor,
        layers: 1,
        pad,
        contact: None,
        label: label.into(),
    };
    Board {
        layer_count: 1,
        outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        edge_clearance: 0.1,
        hole_clearance: 0.1,
        hole_to_hole: 0.1,
        classes: vec![class],
        neck_width: 0.127,
        obstacles,
        nets: vec![Net {
            name: "Vin".into(),
            class: 0,
            terminals: vec![
                terminal(at(5.0, 5.0), 0, "LDO1.2"),
                terminal(at(7.0, 3.5), 1, "C7.1"),
            ],
        }],
        planes: Vec::new(),
        solid_pads: Vec::new(),
        isolated_pads: Vec::new(),
        solid_pad_obstacles: Vec::new(),
        isolated_pad_obstacles: Vec::new(),
        pad_spokes: Default::default(),
        merged_pads: Vec::new(),
        no_via_pads: Vec::new(),
        fill_passes: Vec::new(),
        pin_row_gaps: Vec::new(),
        terminal_copper: Vec::new(),
        attachment: Default::default(),
        via_spans: Vec::new(),
    }
}

#[test]
fn a_pad_whose_centre_lies_in_another_items_copper_leaves_from_its_free_part() {
    // Every offset of the pad against the 0.1 mm lattice: the free strip
    // holds no node, and the centre lies inside the no-net pad; the stub
    // starts in the strip, where a track keeps the clearance.
    let config = Config {
        pitches: vec![0.1],
        ..Config::default()
    };
    for step in 0..5 {
        let offset = step as f64 * 0.023;
        let board = overlapped_pad(0.23, [offset, offset / 2.0]);
        let result = crate::route(&board, &config);
        let violations = crate::verify(&board, &result.routes);
        assert!(violations.is_empty(), "offset {offset}: {violations:?}");
        assert_eq!(result.status[0], NetStatus::Routed, "offset {offset}");
    }
}

#[test]
fn a_dead_pad_names_what_walls_it() {
    // The no-net pad covers all of LDO1.2: no point of it keeps the
    // clearance, and the diagnostic names the pad of no net that walls it.
    let config = Config {
        pitches: vec![0.1],
        ..Config::default()
    };
    let board = overlapped_pad(0.0, [0.0, 0.0]);
    let result = crate::route(&board, &config);
    let dead = &result.diagnostics.dead_pads;
    assert_eq!(dead.len(), 1, "{dead:?}");
    assert_eq!(dead[0].label, "LDO1.2");
    assert!(
        dead[0]
            .wall
            .contains("no point of the pad keeps the clearances")
            && dead[0].wall.contains("LDO1.3 (no net"),
        "{}",
        dead[0].wall
    );
}

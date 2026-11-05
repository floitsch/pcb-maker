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
fn the_lattice_passes_a_pockets_corridor_on_every_offset() {
    // U10's corridors run along x between its corner pads: pads 3 and 4
    // leave 15 um of play either side of y = 5 + dy, pads 1 and 6 30 um
    // of y = 5.015 + dy. Whatever the part's offset, a node row of the
    // chosen lattice lies within one of them (the row of narrow pads
    // votes for phase 0).
    for step in 0..10 {
        let dy = step as f64 * 0.01;
        let grid = Grid::choose(&pocketed_part(dy), &[0.1]);
        let row = |y: f64| {
            let offset = (y - grid.origin[1]).rem_euclid(grid.pitch);
            offset.min(grid.pitch - offset)
        };
        assert!(
            row(5.0 + dy) <= 0.015 + 1.0e-9 || row(5.015 + dy) <= 0.03 + 1.0e-9,
            "dy {dy}: lattice origin {:?}",
            grid.origin
        );
    }
}

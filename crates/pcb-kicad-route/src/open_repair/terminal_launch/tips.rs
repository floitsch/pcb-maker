// Copyright (C) 2026 Florian Loitsch.

use super::{Expr, KiCadBoardRouterConfig, Pad, Point, TerminalLaunch, item_id};
use pcb_kicad_file::kicad_sexpr::{form_at, form_xy};
use pcb_router::geometry::{point_in_polygon, point_segment_distance, polygon_edges};
use pcb_router::verify::shape_distance;
use pcb_router::{Board, ObstacleKind, Shape, Terminal};

const NM: f64 = 0.000_001;
const TIP_LIMIT_MM: f64 = 2.5;

pub(super) fn normal(pad: &Expr, part: &Expr, center: Point) -> Option<(Point, f64)> {
    if !matches!(
        pad.children().get(3)?.atom()?,
        "rect" | "roundrect" | "oval"
    ) {
        return None;
    }
    let [x, y] = form_xy(pad, "size").ok()?;
    let [_, _, angle] = form_at(pad).ok()?;
    let [px, py, _] = form_at(part).ok()?;
    let (sin, cos) = angle.to_radians().sin_cos();
    let direction = if x > y { [cos, -sin] } else { [sin, cos] };
    let projection = (center[0] - px) * direction[0] + (center[1] - py) * direction[1];
    (x > 0.0 && y > 0.0 && (x - y).abs() > NM && projection.abs() >= 0.05)
        .then(|| (direction.map(|v| v * projection.signum()), x.max(y) / 2.0))
}

pub(super) fn outward_tip(
    pcb: &Expr,
    board: &Board,
    terminal: &Terminal,
    pad: &Pad,
    width: f64,
    config: &KiCadBoardRouterConfig,
    previous: &[TerminalLaunch],
) -> Option<Point> {
    let (part, model_pad) = pcb.children().iter().find_map(|part| {
        (part.head() == Some("footprint")).then_some(())?;
        let model_pad = part.children().iter().find(|item| {
            item.head() == Some("pad") && item_id(item).is_some_and(|id| id == pad.id)
        })?;
        Some((part, model_pad))
    })?;
    let start = terminal.anchor;
    if board
        .merged_pads
        .iter()
        .any(|merged| board.nets[merged.net as usize].terminals[merged.host].pad == terminal.pad)
    {
        return None;
    }
    let (direction, half_length) = normal(model_pad, part, start)?;
    let project = config.project_rules.as_ref()?;
    let clearance = project.minimums.as_ref()?.clearance_mm;
    let via_radius = project.via_minimums[0] / 2.0;
    if !clearance.is_finite() || clearance < 0.0 || !via_radius.is_finite() || via_radius <= 0.0 {
        return None;
    }
    let layer = if terminal.layers == 1 { "F.Cu" } else { "B.Cu" };
    let nearby: Vec<_> = board
        .obstacles
        .iter()
        .enumerate()
        .filter(|(index, obstacle)| {
            *index != terminal.pad
                && obstacle.shape.distance_to_point(start)
                    <= TIP_LIMIT_MM
                        + via_radius.max(width / 2.0)
                        + clearance.max(board.hole_clearance).max(obstacle.clearance)
        })
        .map(|(_, obstacle)| obstacle)
        .collect();
    (0..20)
        .map(|step| half_length + 0.2 + f64::from(step) * 0.05)
        .take_while(|length| *length <= TIP_LIMIT_MM)
        .map(|length| {
            std::array::from_fn(|i| ((start[i] + direction[i] * length) / NM).round() * NM)
        })
        .find(|&end| {
            let trace = Shape::Capsule {
                start,
                end,
                radius: width / 2.0,
            };
            let landing = Shape::Circle {
                center: end,
                radius: via_radius,
            };
            let fits = |shape: &Shape, obstacle: &Shape, clearance: f64| {
                let distance = shape_distance(shape, obstacle);
                distance > 0.0 && distance + NM >= clearance
            };
            point_in_polygon(end, &board.outline)
                && polygon_edges(&board.outline).all(|(a, b)| {
                    point_segment_distance(end, a, b) + NM >= via_radius + board.edge_clearance
                })
                && nearby.iter().all(|obstacle| {
                    let minimum = match obstacle.kind {
                        ObstacleKind::Copper => clearance,
                        ObstacleKind::Hole => board.hole_clearance,
                        ObstacleKind::Keepout => obstacle.clearance,
                    };
                    (!obstacle.blocks_vias || fits(&landing, &obstacle.shape, minimum))
                        && (!obstacle.blocks_tracks
                            || obstacle.layers & terminal.layers == 0
                            || fits(&trace, &obstacle.shape, minimum))
                })
                && previous
                    .iter()
                    .filter(|other| other.layer == layer)
                    .all(|other| {
                        let earlier = Shape::Capsule {
                            start: other.start_mm,
                            end: other.end_mm,
                            radius: other.width_mm / 2.0,
                        };
                        fits(&trace, &earlier, clearance)
                    })
        })
}

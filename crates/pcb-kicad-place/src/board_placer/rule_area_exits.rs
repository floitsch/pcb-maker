// Copyright (C) 2026 Florian Loitsch.

//! Local placement repairs for SMD pads fully enclosed by track-and-via
//! keepouts authored on the board.

use super::measured::{component_of, lowered_at, reference_index, routed_base};
use super::*;
use pcb_placer as core;
use pcb_router as router;
use std::sync::Arc;

/// One surface pad of a connected net.
struct Pad {
    net: usize,
    index: usize,
    layer: usize,
}

/// Keep same-layer pours and directly overlapping same-net pads out of
/// the proof: either could join the pad without a track or a via.
fn trapped(
    board: &router::Board,
    pad: &Pad,
    areas: &[router::Obstacle],
    starts_blocked: bool,
    checks: &mut u64,
) -> Option<usize> {
    let net = &board.nets[pad.net];
    let terminal = &net.terminals[pad.index];
    let copper = &board.obstacles[terminal.pad].shape;
    let bounds = copper.aabb();
    let layer_mask = 1 << pad.layer;
    let center = [
        (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
        (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
    ];
    let radius = router::geometry::distance(center, bounds.minimum);
    let reach = 0.5 * board.classes[net.class].trace_width;
    let area = areas.iter().position(|area| {
        *checks += 1;
        area.layers & layer_mask != 0
            && if starts_blocked {
                // An originally trapped pad must clear the rule area's
                // boundary with room for the track reaching its copper.
                area.shape.aabb().intersects(bounds.inflated(reach))
                    && router::verify::shape_distance(copper, &area.shape) <= reach
            } else {
                area.shape.aabb().intersects(bounds) && area.shape.contains_disc(center, radius)
            }
    })?;
    if net.terminals.iter().enumerate().any(|(index, other)| {
        *checks += 1;
        index != pad.index
            && other.layers & layer_mask != 0
            && bounds.intersects(board.obstacles[other.pad].shape.aabb())
    }) || board.planes.iter().any(|plane| {
        *checks += 1;
        plane.layer == pad.layer
            && plane.net as usize == pad.net
            && router::verify::shape_distance(
                copper,
                &router::Shape::Polygon {
                    points: plane.polygon.clone(),
                },
            ) <= reach
    }) {
        return None;
    }
    Some(area)
}

pub(super) fn factory(
    pcb: &Expr,
    references: &[String],
    router_config: &KiCadBoardRouterConfig,
    keep_board_marks: bool,
) -> Result<Option<Arc<core::exits::ExitFactory>>, String> {
    let areas = crate::board_router::board_track_via_keepouts(pcb)?;
    if areas.is_empty() {
        return Ok(None);
    }
    let (base, connect) = routed_base(pcb, router_config, keep_board_marks)?;
    let lowering = super::exits::lowering_units(&base);
    let base = Arc::new(base);
    let areas = Arc::new(areas);
    let index = Arc::new(reference_index(references));
    let reference_count = references.len();
    let router_config = router_config.clone();
    let factory: Arc<core::exits::ExitFactory> = Arc::new(move || {
        let (base, areas, index) = (base.clone(), areas.clone(), index.clone());
        let router_config = router_config.clone();
        let mut state = None;
        let mut initially_blocked = BTreeSet::new();
        Box::new(move |poses, parts| {
            let mut work = 0;
            let initial = state.is_none();
            if initial {
                let board = lowered_at(&base, poses, &router_config, connect)?;
                let mut pads: Vec<Vec<Pad>> = (0..reference_count).map(|_| Vec::new()).collect();
                for (net, record) in board.nets.iter().enumerate() {
                    for (pad, terminal) in record.terminals.iter().enumerate() {
                        if record.terminals.len() >= 2
                            && terminal.layers.count_ones() == 1
                            && let Some(part) = component_of(&terminal.label, &index)
                        {
                            pads[part].push(Pad {
                                net,
                                index: pad,
                                layer: terminal.layers.trailing_zeros() as usize,
                            });
                        }
                    }
                }
                let copper = crate::part_motion::part_copper(&board, &index, reference_count);
                work += lowering;
                state = Some((board, copper, poses.to_vec(), pads));
            }
            let (board, copper, last, pads) = state.as_mut()?;
            let moved: Vec<usize> = (0..reference_count)
                .filter(|&part| last[part] != poses[part])
                .collect();
            if !moved.is_empty() {
                crate::part_motion::apply(board, copper, last, poses, &moved);
                last.copy_from_slice(poses);
            }
            let mut closed = Vec::new();
            let mut checks = 0;
            for &part in parts {
                for pad in &pads[part] {
                    let identity = (pad.net, pad.index);
                    if let Some(area) = trapped(
                        board,
                        pad,
                        &areas,
                        initially_blocked.contains(&identity),
                        &mut checks,
                    ) {
                        if initial {
                            initially_blocked.insert(identity);
                        }
                        let net = &board.nets[pad.net];
                        closed.push(core::exits::ClosedPad {
                            part,
                            pad: net.terminals[pad.index].label.clone(),
                            net: net.name.clone(),
                            layer: pad.layer,
                            blocked_by: vec![areas[area].label.clone()],
                            blockers: Vec::new(),
                            own_only: false,
                        });
                    }
                }
            }
            Some((closed, work + checks * 80))
        })
    });
    Ok(Some(factory))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trapped_surface_pad_must_clear_the_keepout_with_room_for_its_track() {
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
          (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
          (footprint "Test" (property "Reference" "C1") (at 5 5) (layer "F.Cu")
            (pad "1" smd circle (at 0 0) (size 0.8 0.8) (layers "F.Cu") (net "A")))
          (footprint "Test" (property "Reference" "J1") (at 15 5) (layer "F.Cu")
            (pad "1" smd circle (at 0 0) (size 0.8 0.8) (layers "F.Cu") (net "A")))
          (zone (net "") (layer "F.Cu") (name "cover")
            (keepout (tracks not_allowed) (vias not_allowed))
            (polygon (pts (xy 4 4) (xy 6 4) (xy 6 6) (xy 4 6)))))"#,
        )
        .unwrap();
        let references = ["C1".into(), "J1".into()];
        let rules = KiCadBoardRouterConfig {
            default_rules: Some(
                pcb_kicad_file::connection_rules::KiCadConnectionRoutingRules {
                    trace_width_mm: 0.2,
                    clearance_mm: 0.2,
                    via_size_mm: 0.6,
                    via_drill_mm: 0.3,
                },
            ),
            ..KiCadBoardRouterConfig::default()
        };
        let mut probe = factory(&pcb, &references, &rules, false).unwrap().unwrap()();
        let pose = |x| core::Pose {
            position: [x, 5.0],
            angle: 0.0,
        };
        let problem = |x| [pose(x), pose(15.0)];
        let (blocked, _) = probe(&problem(5.0), &[0, 1]).unwrap();
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].blocked_by, ["cover"]);
        assert_eq!(blocked[0].pad, "C1.1");
        assert_eq!(probe(&problem(6.1), &[0]).unwrap().0.len(), 1);
        assert!(probe(&problem(7.0), &[0]).unwrap().0.is_empty());
    }
}

// Copyright (C) 2026 Florian Loitsch.

//! Use the same retained tip geometry in layout's in-memory route.

use super::{
    KiCadBoardRouterConfig, KiCadExistingTracks, KiCadKeptCopper, count_tracks,
    kept::{RetainedPadTip, RetainedPadTips, keep_for_route},
};
use pcb_kicad_file::{
    copper_emit::{SupplementalSegment, supplemental_segment_expr, supplemental_segment_id},
    kicad_nets::note_net_names,
    kicad_sexpr::Expr,
};
use pcb_router::{Board, ObstacleKind, Shape};

pub(super) fn lower(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    connect: bool,
) -> Result<Board, String> {
    if count_tracks(pcb) != [0, 0, 0] {
        return Err("prepared row escapes need a copper-free placement".into());
    }
    let plan = crate::open_repair::plan_row_escapes(pcb, config, &config.prepared_row_escape_pads)?;
    let mut staged = pcb.clone();
    let Expr::List(items) = &mut staged else {
        return Err("prepared row escape PCB is not a board list".into());
    };
    items.extend(plan.iter().map(|(_, segment)| segment.clone()));
    let mut retained = config.clone();
    retained.existing_tracks = KiCadExistingTracks::Keep;
    retained.retained_pad_tips = config.prepared_row_escape_pads.clone();
    let mut board = super::lower(&staged, &retained, connect)?.board;
    let kept = keep_for_route(&mut board, &staged, &retained);
    if !kept.retained_pad_tips.is_some_and(|tips| {
        tips.ignored.is_empty() && tips.used.len() == retained.retained_pad_tips.len()
    }) {
        return Err("the router could not use every prepared pad tip".into());
    }
    for (escape, _) in plan {
        eprintln!(
            "prepared row escape: {} on {} at {:.3} mm width",
            escape.pad, escape.layer, escape.width_mm,
        );
    }
    Ok(board)
}

/// Write the prepared tracks of the selected layout's actual lowered
/// board. The placement artifact and unselected poses never receive them.
pub fn emit_layout_row_escapes(
    pcb: &mut Expr,
    board: &Board,
    config: &KiCadBoardRouterConfig,
    layer_names: &[String],
) -> Result<Option<KiCadKeptCopper>, String> {
    if config.prepared_row_escape_pads.is_empty() {
        return Ok(None);
    }
    let mut tracks = Vec::new();
    for pad in &config.prepared_row_escape_pads {
        let Some((reference, number)) = pad.split_once(':') else {
            continue;
        };
        let label = format!("{reference}.{number}");
        for (index, net) in board.nets.iter().enumerate() {
            let Some(terminal) = net
                .terminals
                .iter()
                .find(|terminal| terminal.label == label)
            else {
                continue;
            };
            let obstacle = &board.obstacles[terminal.pad];
            if obstacle.label != "existing track"
                || obstacle.kind != ObstacleKind::Copper
                || obstacle.net != Some(index as pcb_router::NetId)
                || obstacle.layers.count_ones() != 1
                || terminal.contact.is_none()
            {
                continue;
            }
            let Shape::Capsule { start, end, radius } = obstacle.shape else {
                return Err("prepared row escape lost its straight copper shape".into());
            };
            let layer = obstacle.layers.trailing_zeros() as usize;
            let segment = SupplementalSegment {
                connection: net.name.clone(),
                layer: layer_names[layer].clone(),
                start,
                end,
                width: radius * 2.0,
            };
            tracks.push((pad.clone(), terminal.anchor, segment));
        }
    }
    if tracks.is_empty() {
        return Ok(None);
    }
    if tracks.len() != config.prepared_row_escape_pads.len() {
        return Err("prepared row escape copper no longer matches the selected pad tips".into());
    }
    note_net_names(pcb);
    let Expr::List(items) = pcb else {
        return Err("prepared row escape PCB is not a board list".into());
    };
    let mut result = KiCadKeptCopper {
        segments: tracks.len(),
        ..Default::default()
    };
    let mut tips = RetainedPadTips::default();
    for (pad, at_mm, segment) in tracks {
        let track_uuid = supplemental_segment_id(&segment);
        items.push(supplemental_segment_expr(&segment));
        result.nets.push(segment.connection.clone());
        tips.used.push(RetainedPadTip {
            pad,
            net: segment.connection,
            layer: segment.layer,
            at_mm,
            track_uuid,
        });
    }
    result.retained_pad_tips = Some(tips);
    Ok(Some(result))
}

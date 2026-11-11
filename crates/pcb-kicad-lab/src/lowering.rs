// Copyright (C) 2026 Toit contributors.

//! A placed board lowered as the layout routes a placement (tracks
//! stripped, planned pours added, copper texts kept as the layout keeps
//! them): what the placement ablation and the trouble and coarse probes
//! measure.

use crate::board_router::{KiCadPourMode, LayerTable, add_pour_zones, lower, planned_pours, pours};
use crate::labels::without_copper_texts;
use crate::*;
use pcb_router as core;

/// The board as the layout routes a placement.
pub(crate) fn lower_placed(
    pcb: &Expr,
    router_config: &KiCadBoardRouterConfig,
) -> Result<core::Board, String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut routed = Expr::List(
        items
            .iter()
            .filter(|item| !matches!(item.head(), Some("segment" | "arc" | "via")))
            .cloned()
            .collect(),
    );
    let planned = planned_pours(&routed, router_config)?;
    add_pour_zones(&mut routed, &planned)?;
    let layers = LayerTable::from_pcb(&routed)?;
    let connect =
        !pours(&routed, &layers)?.is_empty() && router_config.pours != KiCadPourMode::Tracks;
    // Copper texts stay, as the layout keeps them by default
    // (`keep_board_marks`).
    Ok(lower(&without_copper_texts(&routed, true), router_config, connect)?.board)
}

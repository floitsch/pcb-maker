// Copyright (C) 2026 Toit contributors.

//! Whole-board routing through the in-memory `pcb-router` core.
//!
//! The board is parsed once, lowered once, routed entirely in memory, checked
//! by the core's exact verifier, and written once. Native KiCad verification
//! is a single final gate, never part of the routing loop.

use super::*;

mod attempt;
mod bga_fanout;
mod budget;
mod config;
mod diff_pairs;
mod geometry;
mod kept;
mod ladder;
mod layer_preference;
mod lengths;
mod lower;
mod meander;
mod memory;
mod native_kept;
mod pour_plan;
mod pour_quality;
mod result;
mod route;
mod route_open_repair;
mod route_reuse;
mod row_escapes;
#[cfg(test)]
mod tests;
mod tools;
mod via_in_pad;
mod via_spans;
mod walled_pads;

pub(crate) use self::attempt::via_layers_and_kind;
pub use self::bga_fanout::bga_fanout_on;
pub use self::bga_fanout::bga_second_ring_open_pads;
pub use self::bga_fanout::{ArrayRoom, array_rooms, far_side_balls, rings_of};
pub(crate) use self::bga_fanout::{fanout_used, fanouts_of};
pub use self::diff_pairs::{
    finish_pairs, lower_for_route, measure_fallback_lengths, measure_uncoupled,
    pairs_fallback_short, pairs_fallback_work, pairs_planned, with_pairs, write_pair_copper,
};
pub(crate) use self::diff_pairs::{isolated_pair_plan_failure, plan_pairs};
pub(crate) use self::kept::{count_tracks, keep_existing_copper, strip_tracks};
pub use self::layer_preference::admit_length_rule_layers;
pub(crate) use self::layer_preference::core_layer_preferences;
pub use self::lengths::{finish_lengths, match_lengths, with_lengths, write_length_patterns};
pub use self::meander::copper_heights;
pub use self::native_kept::KiCadInputConnectivity;
pub use self::route_open_repair::{KiCadNetOpenRepair, KiCadRouteOpenRepair, account_open_repair};
pub use self::route_reuse::{
    KiCadRouteReuse, KiCadRouteReuseFinding, KiCadRouteReuseFindingItem, KiCadRouteReusePartialNet,
    KiCadRouteReuseSource, RouteReuseIdentities, check_reused_copper, check_route_reuse,
    prepare_route_reuse, prepare_route_reuse_for_project, prepare_route_reuse_from,
    read_route_reuse, route_kicad_board_reusing, route_reuse_trial_budget,
};
pub use self::row_escapes::emit_layout_row_escapes;
pub use self::via_in_pad::{CLOSED_PADS_WHY, far_side_vias};
pub(crate) use self::via_in_pad::{PlannedVia, designer_entries, pad_vias_of};
pub use self::via_in_pad::{admits_any as admits_vias_in_pads, plan as plan_vias_in_pads};
pub(crate) use self::via_spans::{allowed_spans, designer_spans, project_spans};
pub use self::walled_pads::{
    KiCadWalledGraphic, KiCadWalledPad, walled_pads_of, walled_pads_of_file,
};
pub use self::{attempt::*, budget::*, geometry::*, ladder::*, lower::*, memory::*};
pub use self::{
    config::*,
    diff_pairs::{
        KiCadDiffPair, KiCadDiffPairs, KiCadExcludedDiffPair, KiCadFallbackConductor,
        KiCadFallbackPairLength, KiCadPairEndpointSpacing,
    },
    kept::KiCadKeptCopper,
    lengths::KiCadNetLength,
    pour_plan::*,
    pour_quality::*,
    result::*,
    route::*,
    tools::*,
    via_in_pad::{
        KiCadClosedPads, KiCadViaInPad, KiCadViaInPadEntry, KiCadViaInPadFootprint,
        KiCadViaInPadReport, admit_reference_vias_in_pads,
    },
    via_spans::{
        KiCadMicroViaMinimums, KiCadViaSpanEntry, admit_reference_via_spans, blind_vias_override,
    },
};

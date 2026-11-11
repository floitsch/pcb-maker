// Copyright (C) 2026 Toit contributors.

//! What the placer weighs beyond wirelength: the escape demand and the
//! ratsnest crossings.

use super::*;

/// Adds to the problem the routing demand the placer weighs: the escape
/// demand around dense parts (`escape_demand`, not where the fixed parts'
/// share reaches its guard) and the ratsnest crossings
/// (`ratsnest_crossings`).
pub(super) fn add_routing_demand(
    context: &PlaceContext,
    pcb: &Expr,
    lowered: &mut LoweredPlacement,
) -> Result<(), String> {
    let config = &context.config;
    let fixed = lowered
        .problem
        .components
        .iter()
        .filter(|component| component.fixed)
        .count();
    let fixed_share = fixed as f64 / lowered.problem.components.len().max(1) as f64;
    if let Some(escape) = &config.tuning.escape_demand
        && let Some(guard) = escape.fixed_share
        && fixed_share >= guard
    {
        eprintln!(
            "placer: escape demand not applied: {fixed} of {} footprints fixed ({:.2}, the guard {guard})",
            lowered.problem.components.len(),
            fixed_share
        );
    } else if let Some(escape) = &config.tuning.escape_demand {
        // The measured version lowers the board as the layout routes it,
        // with the layout's router configuration (its rules): a placement
        // alone has none and runs on the estimate.
        let measured = match (&config.measure_router, escape.measured) {
            (Some(router_config), true) => Some(measured::measured_factory(
                pcb,
                &lowered.references,
                router_config,
                config.keep_board_marks,
                escape.threshold,
            )?),
            _ => None,
        };
        lowered.problem.escape = Some(std::sync::Arc::new(escape_fact(
            escape,
            config.tuning.track_pitch_mm,
            lowered,
            measured,
        )));
    }
    if let Some(crossings) = &config.tuning.ratsnest_crossings
        && (crossings.legalise || crossings.refine || crossings.anneal)
    {
        lowered.problem.crossings = Some(std::sync::Arc::new(core::crossings::Crossings {
            counted: crossing_nets(lowered),
            weight: if crossings.per_layer {
                crossings.weight * (2.0 / lowered.routing_layers.max(1.0)).min(1.0)
            } else {
                crossings.weight
            },
            legalise: crossings.legalise,
            refine: crossings.refine,
            anneal: crossings.anneal,
        }));
    }
    Ok(())
}

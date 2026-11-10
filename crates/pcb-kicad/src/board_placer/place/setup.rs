// Copyright (C) 2026 Toit contributors.

//! What the placer weighs beyond wirelength (the escape demand, the
//! ratsnest crossings) and the placer's configuration for the board.

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
    if let Some(escape) = &config.escape_demand
        && let Some(guard) = escape.fixed_share
        && fixed_share >= guard
    {
        eprintln!(
            "placer: escape demand not applied: {fixed} of {} footprints fixed ({:.2}, the guard {guard})",
            lowered.problem.components.len(),
            fixed_share
        );
    } else if let Some(escape) = &config.escape_demand {
        // The measured version lowers the board as the layout routes it,
        // with the layout's router configuration (its rules): a placement
        // alone has none and runs on the estimate.
        let measured = match (&config.guidance_router, escape.measured) {
            (Some(router_config), true) => Some(guidance::measured_factory(
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
            config.track_pitch_mm,
            lowered,
            measured,
        )));
    }
    if let Some(crossings) = &config.ratsnest_crossings
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

/// The placer's configuration for the board: the settings, the work
/// limit, the warm start, and the router's tests where they are on (the
/// global guidance, the pad exits).
pub(super) fn placer_config(
    context: &PlaceContext,
    pcb: &Expr,
    lowered: &LoweredPlacement,
) -> Result<core::Config, String> {
    let config = &context.config;
    let mut placer_config = core::Config::new();
    placer_config.global.whitespace_fill = config.whitespace_fill;
    placer_config.maximum_utilization = config.maximum_utilization;
    placer_config.global.seed = config.seed;
    placer_config.anneal.deadline = config.deadline;
    placer_config.work_limit = config
        .work_seconds
        .map(|seconds| (seconds.max(0.0) * core::work_per_second()) as u64);
    placer_config.overlap_far_side_pads = config.overlap_far_side_pads;
    placer_config.orientation_search = config.orientation_search;
    placer_config.bus_groups = config.bus_groups.as_ref().map(KiCadBusGroups::settings);
    placer_config.kicad_last_level = true;
    placer_config.start = warm_start(lowered)?.map(std::sync::Arc::new);
    placer_config.skip_anneal = std::env::var_os("PCB_PLACER_SKIP_ANNEAL").is_some();
    if let Some(guidance) = &config.global_guidance {
        let router_config = config.guidance_router.clone().unwrap_or_default();
        placer_config.anneal.guidance = Some(core::guidance::Guidance {
            evaluate: guidance::evaluator(
                pcb,
                &lowered.references,
                &router_config,
                config.keep_board_marks,
                guidance.threshold,
            )?,
            weight: guidance.weight,
            work_share: guidance.work_share,
            min_accepted: guidance.min_accepted,
            bias: guidance.bias,
        });
    }
    // Pads without a way out (`pad_exits`): the router's test after each
    // seed's placement, with the router's configuration the layout and the
    // place check give.
    if let Some(settings) = exits::pad_exits_settings(config) {
        match &config.guidance_router {
            Some(router_config) => {
                placer_config.pad_exits = Some(core::exits::PadExits {
                    factory: exits::exit_factory(
                        pcb,
                        &lowered.references,
                        router_config,
                        config.keep_board_marks,
                    )?,
                    reach: settings.reach_mm,
                    candidates: settings.candidates,
                });
            }
            None => eprintln!(
                "placer: pad exits: no router configuration (the layout and the place check give one): not tested"
            ),
        }
    }
    Ok(placer_config)
}

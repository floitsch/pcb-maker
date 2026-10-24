// Copyright (C) 2026 Toit contributors.

//! `place_kicad_board`: the whole placement of a board.

use super::*;
use pcb_placer as core;

pub fn place_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardPlacerConfig,
) -> Result<KiCadBoardPlacerResult, String> {
    let started = std::time::Instant::now();
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    // Errors up to the lowering are the input's (`input_error`).
    let source = fs::read_to_string(&source_board).map_err(|error| {
        crate::verdict::input_error(format!(
            "failed to read {}: {error}",
            source_board.display()
        ))
    })?;
    let mut pcb =
        crate::canonical_order::parse_board(&source).map_err(crate::verdict::input_error)?;
    let mut config =
        resolve_constraints(config, source_directory).map_err(crate::verdict::input_error)?;
    // Parts added by the import (`add`) stay where the entry puts them.
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &mut config.constraints {
        crate::placement_constraints::expand_additions(constraints, &pcb)
            .map_err(crate::verdict::input_error)?;
    }
    if config.copper_clearance_mm.is_none()
        || config.copper_edge_clearance_mm.is_none()
        || config.hole_clearance_mm.is_none()
        || (config.mask_openings && config.solder_mask_to_copper_mm.is_none())
    {
        let rules = project_rules::resolve_project_rules(
            &source_directory.join(format!("{board_id}.kicad_pro")),
            &source_board,
        )
        .ok();
        if config.copper_clearance_mm.is_none() {
            config.copper_clearance_mm = rules.as_ref().map(largest_clearance);
        }
        if config.copper_edge_clearance_mm.is_none() {
            config.copper_edge_clearance_mm = rules.as_ref().map(|rules| rules.edge_clearance_mm);
        }
        if config.hole_clearance_mm.is_none() {
            config.hole_clearance_mm = rules.as_ref().map(|rules| rules.hole_clearance_mm);
        }
        if config.solder_mask_to_copper_mm.is_none() {
            config.solder_mask_to_copper_mm = rules
                .as_ref()
                .and_then(|rules| rules.solder_mask_to_copper_mm);
        }
    }
    if config.tight_bodies.is_none() {
        config.tight_bodies = Some(courtyards_may_overlap(
            &source_directory.join(format!("{board_id}.kicad_pro")),
        ));
    }
    if config.holes_in_courtyards_error.is_none() {
        config.holes_in_courtyards_error = Some(holes_in_courtyards_error(
            &source_directory.join(format!("{board_id}.kicad_pro")),
        ));
    }
    let edges_chosen = choose_any_edges(source_directory, board_id, &mut config)
        .map_err(crate::verdict::input_error)?;
    let config = &config;
    let mut outline_size = None;
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &config.constraints
        && let Some(outline) = &constraints.outline
    {
        let mut part_area = 0.0;
        for footprint in pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
        {
            let (_, size, _) = local_body(footprint).map_err(crate::verdict::input_error)?;
            part_area += (size[0] + config.spacing_mm) * (size[1] + config.spacing_mm);
        }
        outline_size =
            Some(apply_outline(&mut pcb, outline, part_area).map_err(crate::verdict::input_error)?);
    }
    let mut keepout_warnings = Vec::new();
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &config.constraints {
        let held = if constraints.keepout.is_empty() {
            HeldParts::default()
        } else {
            held_parts(&pcb, config).map_err(crate::verdict::input_error)?
        };
        keepout_warnings = apply_keepouts(&mut pcb, &constraints.keepout, &held)
            .map_err(crate::verdict::input_error)?;
        apply_sides(&mut pcb, constraints).map_err(crate::verdict::input_error)?;
    }
    let mut lowered = lower_placement(&pcb, config).map_err(crate::verdict::input_error)?;
    lowered.constraint_warnings.extend(keepout_warnings);
    if let Some(designer) = std::env::var_os("PCB_PLACER_DESIGNER") {
        diagnose_designer(&lowered, Path::new(&designer))?;
    }
    for warning in &lowered.constraint_warnings {
        eprintln!("warning: {warning}");
    }
    if let Some(escape) = &config.escape_demand {
        // The measured version lowers the board as the layout routes it,
        // with the layout's router configuration (its rules): a placement
        // alone has none and runs on the estimate.
        let measured = match (&config.guidance_router, escape.measured) {
            (Some(router_config), true) => Some(self::guidance::measured_factory(
                &pcb,
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
            &lowered,
            measured,
        )));
    }
    if let Some(crossings) = &config.ratsnest_crossings
        && (crossings.legalise || crossings.refine || crossings.anneal || crossings.global)
    {
        lowered.problem.crossings = Some(std::sync::Arc::new(core::crossings::Crossings {
            counted: crossing_nets(&lowered),
            weight: crossings.weight,
            legalise: crossings.legalise,
            refine: crossings.refine,
            anneal: crossings.anneal,
            global: crossings.global,
        }));
    }
    let mut placer_config = core::Config::new();
    placer_config.global.whitespace_fill = config.whitespace_fill;
    placer_config.global.routing_demand = config.routing_demand;
    placer_config.global.routing_layers = lowered.routing_layers;
    placer_config.global.track_pitch = config.track_pitch_mm;
    placer_config.maximum_utilization = config.maximum_utilization;
    placer_config.global.seed = config.seed;
    placer_config.anneal.deadline = config.deadline;
    placer_config.work_limit = config
        .work_seconds
        .map(|seconds| (seconds.max(0.0) * core::work_per_second()) as u64);
    placer_config.overlap_far_side_pads = config.overlap_far_side_pads;
    placer_config.anneal.legal_far_side = config.anneal_far_side_gaps;
    placer_config.routing_room = routing_room(config, &lowered);
    placer_config.orientation_search = config.orientation_search;
    if let Some(guidance) = &config.global_guidance {
        let router_config = config.guidance_router.clone().unwrap_or_default();
        placer_config.anneal.guidance = Some(core::guidance::Guidance {
            evaluate: self::guidance::evaluator(
                &pcb,
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
    // Several seeds, in parallel; the placement with the fewest illegal
    // parts, then the least missed constraints, then the least wirelength
    // wins. Placement takes seconds; the constraints are what the user asked
    // for.
    let seeds = config.placement_seeds.max(1) as u64;
    // The seeds share the placement's work budget: seed k is kept only
    // while the seeds before it, in seed order, used less than the budget
    // (three seeds at least). They run in a pool (`race_seeds`), as many
    // at once as the process has search threads; which seeds run and
    // which are kept depends on their work alone, not on the threads or
    // the wall clock (SmartSpin2k: sixteen seeds took 1218 s of placement
    // on four cores and left the route no time).
    let minimum_seeds = seeds.min(3);
    let threads = rayon::current_num_threads().max(1);
    // The kept seeds' work as the clock of the reference machine sees it
    // (`REFERENCE_THREADS` at once), and in all; the seeds run, the kept
    // seed.
    let pool_work = std::cell::Cell::new(0u64);
    let seeds_work = std::cell::Cell::new(0u64);
    let seeds_run = std::cell::Cell::new(0u64);
    let place_seeds = |problem: &core::Problem| -> Vec<(u64, core::Placement)> {
        let race = race_seeds(
            seeds,
            minimum_seeds,
            placer_config.work_limit,
            threads,
            |placement: &core::Placement| placement.work,
            |offset| {
                let mut seeded = placer_config.clone();
                seeded.global.seed = config.seed + offset;
                seeded.anneal.seed = seeded.anneal.seed.wrapping_add(offset);
                core::place(problem, &seeded)
            },
        );
        pool_work.set(pool_work.get() + race.pool_work);
        seeds_work.set(
            seeds_work.get()
                + race
                    .kept
                    .iter()
                    .map(|(_, placement)| placement.work)
                    .sum::<u64>(),
        );
        seeds_run.set(seeds_run.get() + race.run);
        let mut placements: Vec<(u64, core::Placement)> = race
            .kept
            .into_iter()
            .map(|(offset, placement)| (config.seed + offset, placement))
            .collect();
        // A placement that needed courtyards touching (the last relaxation
        // level) is a last resort: it ranks below every legal one that did
        // not, however short its wires.
        let key = |placement: &core::Placement| {
            let missed: f64 = placement
                .constraints
                .iter()
                .map(|status| status.violation)
                .sum();
            (
                placement.unplaced.len() + placement.illegal.len(),
                placement.relaxation.courtyard_spacing as usize,
                placement.relaxation.on_marks.len(),
                placement.routing_room_failed as usize,
                missed,
                placement.wirelength_final,
            )
        };
        // Stable: a tie goes to the earlier seed.
        placements.sort_by(|(_, a), (_, b)| {
            key(a)
                .partial_cmp(&key(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        placements
    };
    let placements = place_seeds(&lowered.problem);
    let (kept_seeds, mut placements): (Vec<u64>, Vec<core::Placement>) =
        placements.into_iter().unzip();
    if let Some(trace) = std::env::var_os("PCB_PLACER_TRACE") {
        trace_placements(
            &lowered,
            &placements,
            &placer_config.anneal,
            Path::new(&trace),
        )?;
    }
    let work_seconds = placements
        .iter()
        .map(|placement| placement.work)
        .max()
        .unwrap_or(0) as f64
        / core::work_per_second();
    let seeds_placed = placements.len();
    let placement = placements.remove(0);
    // The other legal placements, for layout to race when the best one by
    // wirelength does not route: wirelength is not routability.
    let alternatives = placements
        .into_iter()
        .filter(|other| other.unplaced.is_empty() && other.illegal.is_empty())
        .map(|other| {
            let missed = other
                .constraints
                .iter()
                .map(|status| status.violation)
                .sum();
            (other.poses, other.relaxation, missed)
        })
        .collect();

    let Expr::List(items) = &mut pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    let mut index = 0;
    let mut footprints = Vec::new();
    for footprint in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        let pose = placement.poses[index];
        let source_at = lowered.source_at[index];
        let at = write_footprint_pose(footprint, pose)?;
        footprints.push(KiCadPlacedFootprint {
            reference: lowered.references[index].clone(),
            fixed: lowered.problem.components[index].fixed,
            halo_mm: lowered.problem.components[index].halo,
            body_center: lowered.problem.components[index].center(pose),
            body_half: lowered.problem.components[index].half_extent(pose.angle),
            source_at,
            at,
        });
        index += 1;
    }

    // The written board must put every pad where the placer believed it is.
    index = 0;
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let footprint_at = form_at(footprint)?;
        let component = &lowered.problem.components[index];
        let mut pin = 0;
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if !node_net(pad).is_some_and(placer_net) {
                continue;
            }
            let written = lower_pad(pad, footprint_at)?.center;
            // A part a row carries has no pins of its own: its pads follow
            // its own pose.
            let expected = match component.pins.get(pin) {
                Some(own) => component.pin_position(own, placement.poses[index]),
                None => {
                    let local = form_at(pad)?;
                    component.pin_position(
                        &core::Pin {
                            offset: [local[0], local[1]],
                            net: 0,
                        },
                        placement.poses[index],
                    )
                }
            };
            if distance_squared(written, expected).sqrt() > 1.0e-5 {
                return Err(format!(
                    "internal error: pad of {} written at {written:?}, expected {expected:?}",
                    lowered.references[index]
                ));
            }
            pin += 1;
        }
        index += 1;
    }

    if output_directory.exists() {
        return Err(format!(
            "output directory {} already exists",
            output_directory.display()
        ));
    }
    copy_directory_tree(source_directory, output_directory)?;
    let output_board = output_directory.join(format!("{board_id}.kicad_pcb"));
    fs::write(&output_board, format!("{}\n", encode(&pcb)))
        .map_err(|error| format!("failed to write {}: {error}", output_board.display()))?;
    let html = core::view::playback_html(
        &lowered.problem,
        &placement.frames,
        &format!("{board_id} placement"),
    );
    let html_path = output_directory.join("placement.html");
    fs::write(&html_path, html)
        .map_err(|error| format!("failed to write {}: {error}", html_path.display()))?;

    let names = |indices: &[usize]| -> Vec<String> {
        indices
            .iter()
            .map(|index| lowered.references[*index].clone())
            .collect()
    };
    // How full each side is, and what to change when parts did not fit.
    let problem = &lowered.problem;
    let outline_area = {
        let outline = &problem.outline;
        (0..outline.len())
            .map(|index| {
                let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            .abs()
            / 2.0
    };
    let mut used = [0.0f64; 2];
    for (index, component) in problem.components.iter().enumerate() {
        let half = component.half_extent(placement.poses[index].angle);
        let area = if component.hollow.is_empty() {
            (2.0 * half[0] + problem.spacing) * (2.0 * half[1] + problem.spacing)
        } else {
            component.blocking_area()
        };
        match component.side {
            core::Side::Front => used[0] += area,
            core::Side::Back => used[1] += area,
            core::Side::Both => {
                used[0] += area;
                used[1] += area;
            }
            core::Side::Neither => {}
        }
    }
    let utilization = used.map(|used| (used / outline_area.max(1.0e-9) * 1000.0).round() / 1000.0);
    let mut hints = Vec::new();
    if !placement.unplaced.is_empty() || !placement.illegal.is_empty() {
        for (side, share) in ["front", "back"].iter().zip(utilization) {
            if share > 0.7 {
                hints.push(format!(
                    "the {side} side is {:.0} % full: enlarge the outline, move parts to the other side, or fix fewer parts",
                    share * 100.0
                ));
            }
        }
        let constraints = &problem.constraints;
        // Fixed parts are only illegal where a part that found no place
        // lies on them; the moving parts are the ones to talk about.
        for index in placement
            .unplaced
            .iter()
            .chain(&placement.illegal)
            .filter(|index| !problem.components[**index].fixed)
        {
            let name = &lowered.references[*index];
            if constraints.edges.iter().any(|(part, _, _)| part == index)
                || constraints.regions.iter().any(|(part, _)| part == index)
                || constraints
                    .overhangs
                    .iter()
                    .any(|(part, _, _)| part == index)
            {
                hints.push(format!(
                    "{name} is held by an edge, region or overhang constraint: loosen it (a larger max_mm or region)"
                ));
            }
        }
        if placement.overfull {
            hints.push(
                "even the tightest bodies need more area than a side has: no placement fits every part; room.suggested_outline_mm is an outline that holds them"
                    .into(),
            );
        }
        if hints.is_empty() {
            hints.push(
                "no free spot fits the part's courtyard with the copper clearance around it: give the board more room"
                    .into(),
            );
        }
        hints.sort();
        hints.dedup();
    }
    let room = crate::placement_room::room(
        problem,
        &lowered.references,
        !placement.unplaced.is_empty() || !placement.illegal.is_empty(),
    );
    let result = KiCadBoardPlacerResult {
        alternatives,
        tight_bodies: placement.relaxation.tight,
        spacing_mm: placement.relaxation.spacing,
        grid_mm: placement.relaxation.grid,
        halo_scale: placement.relaxation.halo_scale,
        small_halo_scale: placement.relaxation.small_halo_scale,
        edge_inset: placement.relaxation.edge_inset,
        edge_copper: placement.relaxation.edge_copper,
        edge_rule_mm: placement.relaxation.edge_rule,
        courtyard_spacing: placement.relaxation.courtyard_spacing,
        parts_on_marks: placement
            .relaxation
            .on_marks
            .iter()
            .map(|index| lowered.references[*index].clone())
            .collect(),
        on_marks: placement.relaxation.on_marks.clone(),
        attached_marks: lowered.attached_report.clone(),
        attached_mark_pairs: lowered.attached_marks.clone(),
        halos: placement.relaxation.halos.clone(),
        utilization,
        hints,
        room,
        overfull: placement.overfull,
        board_id: board_id.into(),
        components: lowered.problem.components.len(),
        movable: lowered
            .problem
            .components
            .iter()
            .filter(|component| !component.fixed)
            .count(),
        nets: lowered.problem.net_weights.len(),
        global_iterations: placement.global_iterations,
        global_overflow: placement.global_overflow,
        wirelength_source_mm: placement.wirelength_initial,
        wirelength_global_mm: placement.wirelength_global,
        wirelength_legal_mm: placement.wirelength_legal,
        wirelength_final_mm: placement.wirelength_final,
        unplaced: names(&placement.unplaced),
        illegal: names(&placement.illegal),
        constraints: constraint_report(&lowered.problem, &placement.poses, &lowered.references),
        constraint_warnings: lowered.constraint_warnings.clone(),
        outline_mm: outline_size,
        edges_chosen,
        seconds: started.elapsed().as_secs_f64(),
        work_seconds,
        pool_work_seconds: pool_work.get() as f64 / core::work_per_second(),
        seeds_work_seconds: seeds_work.get() as f64 / core::work_per_second(),
        seeds: seeds_placed,
        seeds_run: seeds_run.get() as usize,
        seed_kept: kept_seeds[0],
        footprints,
    };
    let report_path = output_directory.join("board-placer.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(result)
}

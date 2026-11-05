// Copyright (C) 2026 Toit contributors.

//! `place_kicad_board`: the whole placement of a board.

use super::*;
use pcb_placer as core;

/// A board read, its constraints resolved and its project's rules taken,
/// and lowered as the placer places it (`prepare_board`).
pub(crate) struct PreparedBoard {
    pub pcb: Expr,
    pub config: KiCadBoardPlacerConfig,
    pub lowered: LoweredPlacement,
    pub outline_size: Option<[f64; 2]>,
    pub edges_chosen: BTreeMap<String, String>,
}

/// Reads `<board_id>.kicad_pcb` from `source_directory`, resolves the
/// constraints and the project's rules into `config`, applies the outline,
/// keepout and side constraints, and lowers the board: everything
/// `place_kicad_board` does before it places (also the legality census,
/// `legality_kicad_board`).
pub(crate) fn prepare_board(
    source_directory: &Path,
    board_id: &str,
    config: &KiCadBoardPlacerConfig,
    poses_from: Option<&Expr>,
) -> Result<PreparedBoard, String> {
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
    if let Some(from) = poses_from {
        take_poses(&mut pcb, from)?;
    }
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
    // The pairs' widths and gaps, for the pair rows' pitch (the layout's
    // and the place check's configuration has them).
    if config.pair_rows_on() && config.pair_geometry.is_empty() {
        config.pair_geometry = project_rules::resolve_project_rules(
            &source_directory.join(format!("{board_id}.kicad_pro")),
            &source_board,
        )
        .map(|rules| pair_geometry(&rules))
        .unwrap_or_default();
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
    if config.silk_over_parts_error.is_none() {
        config.silk_over_parts_error = Some(silk_over_parts_error(
            &source_directory.join(format!("{board_id}.kicad_pro")),
        ));
    }
    let edges_chosen = choose_any_edges(source_directory, board_id, &mut config)
        .map_err(crate::verdict::input_error)?;
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
            held_parts(&pcb, &config).map_err(crate::verdict::input_error)?
        };
        keepout_warnings = apply_keepouts(&mut pcb, &constraints.keepout, &held)
            .map_err(crate::verdict::input_error)?;
        apply_sides(&mut pcb, constraints).map_err(crate::verdict::input_error)?;
    }
    let mut lowered = lower_placement(&pcb, &config).map_err(crate::verdict::input_error)?;
    lowered.constraint_warnings.extend(keepout_warnings);
    Ok(PreparedBoard {
        pcb,
        config,
        lowered,
        outline_size,
        edges_chosen,
    })
}

pub fn place_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardPlacerConfig,
) -> Result<KiCadBoardPlacerResult, String> {
    let started = std::time::Instant::now();
    let PreparedBoard {
        mut pcb,
        config,
        mut lowered,
        outline_size,
        edges_chosen,
    } = prepare_board(source_directory, board_id, config, None)?;
    let config = &config;
    if let Some(designer) = std::env::var_os("PCB_PLACER_DESIGNER") {
        diagnose_designer(&lowered, Path::new(&designer))?;
    }
    for warning in &lowered.constraint_warnings {
        eprintln!("warning: {warning}");
    }
    // Panel copies (`panel_copies`): each seed places the first copy and
    // replicates it (`panel::place_copies`).
    let copies = if config.panel_copies {
        let footprint_count = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .count();
        match self::panel::detect(&lowered.problem, footprint_count) {
            Ok(copies) => {
                eprintln!(
                    "placer: panel copies: {} copies of {} parts ({}, regions from {})",
                    copies.members.len(),
                    copies.members[0].len(),
                    copies.detected_by,
                    copies.regions_from
                );
                if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
                    eprintln!(
                        "placer: panel copies: regions {:?}",
                        copies
                            .regions
                            .iter()
                            .map(|r| r.map(|v| (v * 100.0).round() / 100.0))
                            .collect::<Vec<_>>()
                    );
                }
                Some(copies)
            }
            Err(why) => {
                if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
                    eprintln!("placer: panel copies: none ({why})");
                }
                None
            }
        }
    } else {
        None
    };
    // Sub-boards of a panel of different boards (`sub_boards`): where no
    // copies were found (`sub_boards::detect_for_placer`), before the
    // escape demand and the crossings read the problem's nets.
    let mut sub_boards_report = None;
    // The sub-boards when the placement holds their parts in them: each
    // seed places them one by one (`sub_boards::place_sub_boards`).
    let mut held_sub_boards = None;
    crate::sub_boards::note_designed(None);
    // A panel of copies whose cells share net names (the PolyKybd
    // molecules: every cell's 27 nets, no copper between cells): the
    // router splits those nets per cell and KiCad's items across cells
    // are not counted (`sub_boards::note_cells`).
    if let Some(copies) = &copies
        && copies.detected_by == "repeated references"
    {
        let shared = crate::sub_boards::repeated_reference_nets(&pcb);
        if !shared.is_empty() {
            eprintln!(
                "placer: panel copies: {} nets of one name in several cells: one net per cell",
                shared.len()
            );
            crate::sub_boards::note_designed(Some(shared));
            crate::sub_boards::note_cells(Some(copies.regions.clone()));
        }
    }
    if config.sub_boards {
        let footprint_count = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .count();
        let reference = match &config.sub_board_reference {
            Some(path) => {
                let text = fs::read_to_string(path).map_err(|error| {
                    crate::verdict::input_error(format!(
                        "failed to read the reference board {}: {error}",
                        path.display()
                    ))
                })?;
                Some(
                    crate::canonical_order::parse_board(&text)
                        .map_err(crate::verdict::input_error)?,
                )
            }
            None => None,
        };
        let detected = crate::sub_boards::detect_for_placer(
            &pcb,
            &lowered.problem,
            footprint_count,
            reference.as_ref(),
        )?;
        // No part on any piece (a netlist import's stack without
        // `--reference`): the pieces, tabs and frames from the outline
        // alone are reported, nothing else changes.
        if copies.is_none()
            && let Some(sub) = &detected
            && sub.count(&lowered.problem) == 0
            && !sub.has_frames()
        {
            let report = crate::sub_boards::KiCadSubBoardsReport::of_outline(
                sub,
                &lowered.problem,
                &lowered.references,
            );
            eprintln!(
                "placer: sub-boards: the outline falls into {} pieces (cut by {}; {} frames or rails by their shape) and no part lies on any: {} movable parts without a board (give --reference or a region per board)",
                report.pieces,
                report.cut_by.join(", "),
                report.frames.len(),
                report.unassigned
            );
            sub_boards_report = Some(report);
        }
        let sub = detected.filter(|sub| sub.count(&lowered.problem) >= 2 || sub.has_frames());
        if let (Some(copies), Some(sub)) = (&copies, &sub) {
            // A panel of copies keeps its parts off its frames and tabs
            // too, each copy off the images of every copy's (one
            // placement replicates to all), but for connectors and the
            // designer's overhangs (`sub_boards::keep_off_frames`); the
            // copies are placed as copies, nothing held by sub-board, no
            // net split.
            let frames = crate::sub_boards::CopyFrames {
                transforms: copies
                    .transforms
                    .iter()
                    .map(|transform| (transform.angle, transform.offset))
                    .collect(),
                within: copies_surroundings(copies, sub),
                held: copies_held_bands(copies, &lowered.problem),
            };
            let names = lowered.references.clone();
            let mut applied = crate::sub_boards::PlacerApplied::default();
            crate::sub_boards::keep_off_frames(
                sub,
                &mut lowered.problem,
                &mut lowered.references,
                &mut lowered.source_at,
                &mut applied,
                true,
                Some(&frames),
            );
            let mut report = crate::sub_boards::KiCadSubBoardsReport::of(
                sub,
                &lowered.problem,
                &lowered.references,
                &names,
                &applied,
            );
            report.copies = true;
            eprintln!(
                "placer: sub-boards: a panel of {} copies, {} frames, cut by {}, {} keepouts over the tabs and frames ({:.0} mm2, each copy's in every copy; margin {}, grown by 1.5 mm they take {:.2} of the first copy's surroundings), {} connectors and {} designer's overhangs may reach over them",
                copies.members.len(),
                report.frames.len(),
                report.cut_by.join(", "),
                applied.tabs,
                applied.kept_clear_mm2,
                match applied.margin_mm {
                    Some(margin) => format!("{margin} mm"),
                    None => "none: the copies are too small for it, nothing kept off".into(),
                },
                applied.copy_kept_share,
                applied.connectors,
                applied.designer_overhangs
            );
            sub_boards_report = Some(report);
        }
        if copies.is_none()
            && let Some(sub) = sub
        {
            // A board beside its frame alone (one sub-board) has no nets on
            // several by design.
            if sub.count(&lowered.problem) >= 2 {
                crate::sub_boards::note_designed(Some(crate::sub_boards::designed_nets(
                    &pcb,
                    &sub,
                    &lowered.net_names,
                )));
            }
            let names = lowered.references.clone();
            let applied = crate::sub_boards::apply_to_placer(
                &sub,
                &mut lowered.problem,
                &mut lowered.references,
                &mut lowered.source_at,
                &mut lowered.poured,
                config.net_weight,
                &lowered.net_names,
            );
            let report = crate::sub_boards::KiCadSubBoardsReport::of(
                &sub,
                &lowered.problem,
                &lowered.references,
                &names,
                &applied,
            );
            eprintln!(
                "placer: sub-boards: {} ({}), {} nets on several, {} parts without one, {} held in their region, {} frames, cut by {}, {} keepouts over the tabs and frames ({:.0} mm2), {} connectors and {} designer's overhangs may reach over them",
                report.count,
                report.from.join(", "),
                report.split_nets,
                report.unassigned,
                applied.held,
                report.frames.len(),
                report.cut_by.join(", "),
                applied.tabs,
                applied.kept_clear_mm2,
                applied.connectors,
                applied.designer_overhangs
            );
            sub_boards_report = Some(report);
            if applied.held > 0 {
                held_sub_boards = Some(sub);
            }
        }
    }
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
            weight: if crossings.per_layer {
                crossings.weight * (2.0 / lowered.routing_layers.max(1.0)).min(1.0)
            } else {
                crossings.weight
            },
            legalise: crossings.legalise,
            refine: crossings.refine,
            anneal: crossings.anneal,
            global: crossings.global,
        }));
    }
    let mut placer_config = core::Config::new();
    placer_config.global.whitespace_fill = config.whitespace_fill;
    placer_config.maximum_utilization = config.maximum_utilization;
    placer_config.global.seed = config.seed;
    placer_config.anneal.deadline = config.deadline;
    placer_config.work_limit = config
        .work_seconds
        .map(|seconds| (seconds.max(0.0) * core::work_per_second()) as u64);
    placer_config.overlap_far_side_pads = config.overlap_far_side_pads;
    placer_config.anneal.legal_far_side = config.anneal_far_side_gaps;
    placer_config.orientation_search = config.orientation_search;
    placer_config.bus_groups = config.bus_groups.as_ref().map(KiCadBusGroups::settings);
    placer_config.kicad_legality = config.legality == KiCadPlacerLegality::Kicad;
    placer_config.kicad_last_level = config.legality != KiCadPlacerLegality::Strict;
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
    // Pads without a way out (`pad_exits`): the router's test after each
    // seed's placement, with the router's configuration the layout and the
    // place check give.
    if let Some(settings) = self::exits::pad_exits_settings(config) {
        match &config.guidance_router {
            Some(router_config) => {
                placer_config.pad_exits = Some(core::exits::PadExits {
                    factory: self::exits::exit_factory(
                        &pcb,
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
    // The race over skeletons: the passives by the class rule.
    let skeleton_race = config
        .skeleton_race
        .as_ref()
        .filter(|_| copies.is_none() && held_sub_boards.is_none())
        .map(|race| {
            (
                race.settings(),
                passive_mask(&pcb, lowered.problem.components.len()),
            )
        });
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
                // The pad-exits repair is a whole board's (its test
                // names parts by the board's indices): not inside a
                // panel's copies or a held sub-board.
                if copies.is_some() || held_sub_boards.is_some() {
                    seeded.pad_exits = None;
                }
                match (&copies, &held_sub_boards) {
                    (Some(copies), _) => self::panel::place_copies(problem, copies, &seeded),
                    (None, Some(sub)) => crate::sub_boards::place_sub_boards(problem, sub, &seeded),
                    (None, None) => match &skeleton_race {
                        Some((settings, passive)) => core::skeleton::place_skeleton(
                            problem,
                            &seeded,
                            passive,
                            &lowered.poured,
                            settings,
                        ),
                        None => core::place(problem, &seeded),
                    },
                }
            },
        );
        // The pad-exits repair's work (`pad_exits`), booked apart from the
        // seeds' (the seeds kept do not depend on it): on the clock as the
        // reference machine's pool runs it beside them.
        let exits_works: Vec<u64> = race
            .kept
            .iter()
            .filter_map(|(_, placement)| placement.exits.as_ref().map(|outcome| outcome.work))
            .collect();
        pool_work.set(
            pool_work.get()
                + race.pool_work
                + self::seed_race::pool_makespan(&exits_works, REFERENCE_THREADS),
        );
        seeds_work.set(
            seeds_work.get()
                + race
                    .kept
                    .iter()
                    .map(|(_, placement)| placement.work)
                    .sum::<u64>()
                + exits_works.iter().sum::<u64>(),
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
            // Pads another part or the edge closes (`pad_exits`; none
            // with it off) count after the illegal parts.
            let closed = placement.exits.as_ref().map_or(0, |outcome| {
                outcome.closed.iter().filter(|pad| !pad.own_only).count()
            });
            (
                placement.unplaced.len() + placement.illegal.len(),
                closed,
                placement.relaxation.courtyard_spacing as usize,
                placement.relaxation.on_marks.len(),
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
    let skeleton_race_report = skeleton_race.as_ref().map(|(_, passive)| {
        KiCadSkeletonRaceReport::of(
            passive,
            &placements[0],
            &placements.iter().collect::<Vec<_>>(),
        )
    });
    let placement = placements.remove(0);
    let panel_copies = copies
        .as_ref()
        .map(|copies| KiCadPanelCopiesReport::of(&lowered.problem, copies, &placement));
    // The other legal placements, for layout to race when the best one by
    // wirelength does not route: wirelength is not routability.
    let (alternatives, alternative_exits): (Vec<_>, Vec<_>) = placements
        .into_iter()
        .filter(|other| other.unplaced.is_empty() && other.illegal.is_empty())
        .map(|other| {
            let missed = other
                .constraints
                .iter()
                .map(|status| status.violation)
                .sum();
            let exits = other
                .exits
                .as_ref()
                .map(|outcome| self::exits::exits_report(outcome, &lowered.references, &pcb));
            ((other.poses, other.relaxation, missed), exits)
        })
        .unzip();

    let Expr::List(items) = &mut pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    let mut index = 0;
    let mut footprints = Vec::new();
    let mut uuids = Vec::new();
    for footprint in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        uuids.push(crate::kicad_nets::footprint_uuid(footprint));
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
    // How full each side is, in the room's numbers (`placement_room::room`:
    // the movable parts' bodies with spacing over the usable area, the
    // outline less its edge band, its cutouts and the fixed parts' bodies),
    // and what to change when parts did not fit. Until 2026-11-04 the hint
    // and `utilization` counted every component (fixed parts, the board's
    // marks, rule areas, a panel's kept-clear frames, each with spacing)
    // over the outline's gross area: trial 4's ULK panel was "99 % full"
    // beside room.utilization 0.135.
    let problem = &lowered.problem;
    let room = crate::placement_room::room(
        problem,
        &lowered.references,
        !placement.unplaced.is_empty() || !placement.illegal.is_empty(),
    );
    let utilization = room.utilization;
    let repeated_references = {
        let mut seen = BTreeSet::new();
        let mut repeated = BTreeSet::new();
        for reference in &lowered.references[..uuids.len().min(lowered.references.len())] {
            if !seen.insert(reference.as_str()) {
                repeated.insert(reference.clone());
            }
        }
        repeated
    };
    let unplaced_parts = unplaced_parts(
        problem,
        &lowered.references,
        &uuids,
        &repeated_references,
        &placement,
    );
    let mut hints = Vec::new();
    if !placement.unplaced.is_empty() || !placement.illegal.is_empty() {
        for (side, (share, usable)) in ["front", "back"]
            .iter()
            .zip(room.utilization.iter().zip(room.usable_mm2))
        {
            if *share > crate::placement_room::TARGET_UTILIZATION {
                hints.push(room_hint(side, *share, usable));
            }
        }
        // Fixed parts are only illegal where a part that found no place
        // lies on them; the moving parts are the ones to talk about, each
        // with the constraints that hold it (a repeated reference with
        // its uuid).
        for part in &unplaced_parts {
            if !part.held_by.is_empty() {
                hints.push(format!(
                    "{} is held by {}: loosen it (a larger max_mm or region)",
                    part.label,
                    part.held_by.join(", ")
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
    let result = KiCadBoardPlacerResult {
        alternatives,
        alternative_exits,
        tight_bodies: placement.relaxation.tight,
        spacing_mm: placement.relaxation.spacing,
        grid_mm: placement.relaxation.grid,
        halo_scale: placement.relaxation.halo_scale,
        small_halo_scale: placement.relaxation.small_halo_scale,
        edge_inset: placement.relaxation.edge_inset,
        edge_copper: placement.relaxation.edge_copper,
        edge_rule_mm: placement.relaxation.edge_rule,
        courtyard_spacing: placement.relaxation.courtyard_spacing,
        pad_clearance: placement.relaxation.pad_clearance,
        parts_on_marks: placement
            .relaxation
            .on_marks
            .iter()
            .map(|index| lowered.references[*index].clone())
            .collect(),
        on_marks: placement.relaxation.on_marks.clone(),
        attached_marks: lowered.attached_report.clone(),
        array_rooms: lowered.array_rooms.clone(),
        length_rules: super::lower::length_rule_report(&lowered, &placement.poses),
        attached_mark_pairs: lowered.attached_marks.clone(),
        halos: placement.relaxation.halos.clone(),
        utilization,
        hints,
        room,
        overfull: placement.overfull,
        board_id: board_id.into(),
        components: lowered.problem.components.len(),
        movable: placed_count(&lowered.problem),
        nets: lowered.problem.net_weights.len(),
        global_iterations: placement.global_iterations,
        global_overflow: placement.global_overflow,
        wirelength_source_mm: placement.wirelength_initial,
        wirelength_global_mm: placement.wirelength_global,
        wirelength_legal_mm: placement.wirelength_legal,
        wirelength_final_mm: placement.wirelength_final,
        unplaced: names(&placement.unplaced),
        illegal: names(&placement.illegal),
        unplaced_parts,
        constraints: constraint_report(&lowered.problem, &placement.poses, &lowered.references)
            .into_iter()
            .chain(pair_row_statuses(
                &lowered.pair_rows,
                &lowered.problem,
                &placement.poses,
                &lowered.references,
            ))
            .collect(),
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
        bus_groups: config.bus_groups.as_ref().map(|bus_groups| {
            KiCadBusGroupsReport::of(
                &lowered.problem,
                &bus_groups.settings(),
                &placement.poses,
                &lowered.references,
            )
        }),
        panel_copies,
        sub_boards: sub_boards_report,
        pad_exits: placement
            .exits
            .as_ref()
            .map(|outcome| self::exits::exits_report(outcome, &lowered.references, &pcb)),
        skeleton_race: skeleton_race_report,
    };
    let report_path = output_directory.join("board-placer.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(result)
}

/// The placement hint for a side fuller than the room's target
/// (`placement_room::TARGET_UTILIZATION`), in the room's terms: `share`
/// is `room.utilization` of the side, `usable` its `room.usable_mm2`.
pub(crate) fn room_hint(side: &str, share: f64, usable: f64) -> String {
    if usable <= 0.0 {
        return format!(
            "the {side} side has no usable area left (room.usable_mm2 0: the edge band, the cutouts and the fixed parts' bodies cover the outline): enlarge the outline, move parts to the other side, or fix fewer parts"
        );
    }
    format!(
        "the {side} side is {:.0} % full (room.utilization {share:.3}: the movable parts' bodies with spacing over its {usable:.0} mm2 usable, the outline less its edge band, its cutouts and the fixed parts): enlarge the outline, move parts to the other side, or fix fewer parts",
        share * 100.0
    )
}

/// The movable parts without a legal place, each once (unplaced first),
/// with their uuids and the constraints that hold them.
/// Per copy of a panel, where its own parts may lie: its region, cut to
/// the box of the outline pieces its parts lie on (`sub_boards`; the
/// region alone where none does) grown by the tabs' margin: the area
/// whose keepouts every copy takes over (`Pieces::copies_keepouts`).
fn copies_surroundings(
    copies: &self::panel::PanelCopies,
    sub: &crate::sub_boards::PlacerSubBoards,
) -> Vec<[f64; 4]> {
    let margin = crate::sub_boards::TAB_MARGIN_MM;
    copies
        .members
        .iter()
        .zip(&copies.regions)
        .map(|(members, region)| {
            let pieces: BTreeSet<usize> = members
                .iter()
                .filter_map(|part| sub.part_piece.get(*part).copied().flatten())
                .collect();
            let Some(around) =
                pieces
                    .iter()
                    .map(|piece| sub.pieces.boxes[*piece])
                    .reduce(|a, b| {
                        [
                            a[0].min(b[0]),
                            a[1].min(b[1]),
                            a[2].max(b[2]),
                            a[3].max(b[3]),
                        ]
                    })
            else {
                return *region;
            };
            [
                region[0].max(around[0] - margin),
                region[1].max(around[1] - margin),
                region[2].min(around[2] + margin),
                region[3].min(around[3] + margin),
            ]
        })
        .collect()
}

/// Per copy of a panel, the bands along the board's edges where one of
/// its movable parts is held (an edge or overhang constraint): as deep as
/// the part's longer side and its edge distance (`max_mm`; the overhang's
/// box), a millimetre more. The other copies' keepouts are not carried
/// there (`sub_boards::CopyFrames::held`).
fn copies_held_bands(
    copies: &self::panel::PanelCopies,
    problem: &core::Problem,
) -> Vec<Vec<[f64; 4]>> {
    let board = problem.bounds();
    let band = |edge: core::constraints::Edge, depth: f64| match edge {
        core::constraints::Edge::Left => [board[0], board[1], board[0] + depth, board[3]],
        core::constraints::Edge::Right => [board[2] - depth, board[1], board[2], board[3]],
        core::constraints::Edge::Top => [board[0], board[1], board[2], board[1] + depth],
        core::constraints::Edge::Bottom => [board[0], board[3] - depth, board[2], board[3]],
    };
    copies
        .members
        .iter()
        .map(|members| {
            let mut bands = Vec::new();
            for &part in members {
                let component = &problem.components[part];
                if component.fixed {
                    continue;
                }
                let longer = component.body_size[0].max(component.body_size[1]);
                for (held, edge, max) in &problem.constraints.edges {
                    if *held == part {
                        bands.push(band(*edge, longer + max + 1.0));
                    }
                }
                for (held, edge, _) in &problem.constraints.overhangs {
                    if *held == part {
                        bands.push(band(*edge, longer + 1.0));
                    }
                }
            }
            bands
        })
        .collect()
}

fn unplaced_parts(
    problem: &core::Problem,
    references: &[String],
    uuids: &[Option<String>],
    repeated: &BTreeSet<String>,
    placement: &core::Placement,
) -> Vec<KiCadUnplacedPart> {
    let constraints = &problem.constraints;
    let mut seen = BTreeSet::new();
    let mut parts = Vec::new();
    // The illegal parts' reasons, judged at the placement's level.
    let relaxed = (!placement.illegal.is_empty()).then(|| {
        let mut relaxed = problem.clone();
        placement.relaxation.apply(&mut relaxed);
        relaxed
    });
    let reason = |index: usize| -> Option<String> {
        let relaxed = relaxed.as_ref()?;
        let poses = &placement.poses;
        // The pair tests book no placement work (`is_legal` would).
        Some(
            (0..relaxed.components.len())
                .filter(|other| *other != index)
                .find_map(|other| {
                    core::legal::conflict_kind(relaxed, poses, index, poses[index], other)
                        .map(|kind| format!("{kind} with {}", references[other]))
                })
                .unwrap_or_else(|| "the outline or a hard constraint".into()),
        )
    };
    for (index, status) in placement
        .unplaced
        .iter()
        .map(|index| (*index, "unplaced"))
        .chain(placement.illegal.iter().map(|index| (*index, "illegal")))
    {
        // Fixed parts and the board's marks are illegal only where a part
        // that found no place lies on them.
        if problem.components[index].fixed || index >= uuids.len() || !seen.insert(index) {
            continue;
        }
        let mut held_by = Vec::new();
        for (part, edge, max) in &constraints.edges {
            if *part == index {
                held_by.push(format!("edge {} within {max:.2} mm", edge.name()));
            }
        }
        for (part, region) in &constraints.regions {
            if *part == index {
                held_by.push(format!(
                    "region x {:.2}..{:.2}, y {:.2}..{:.2}",
                    region[0], region[2], region[1], region[3]
                ));
            }
        }
        for (part, edge, _) in &constraints.overhangs {
            if *part == index {
                held_by.push(format!("overhang {}", edge.name()));
            }
        }
        let pose = placement.poses[index];
        let reference = &references[index];
        let label = match &uuids[index] {
            Some(uuid) if repeated.contains(reference) => format!("{reference} (uuid {uuid})"),
            _ => reference.clone(),
        };
        parts.push(KiCadUnplacedPart {
            reference: reference.clone(),
            label,
            uuid: uuids[index].clone(),
            status,
            at: [pose.position[0], pose.position[1], pose.angle],
            held_by,
            reason: if status == "illegal" {
                reason(index)
            } else {
                None
            },
        });
    }
    parts
}

/// The parts the placer placed: the free ones and a row's other parts,
/// which move with its first (fixed in the problem only because their
/// leader carries them; trial 5's right_io counted its six row followers
/// as "kept where they are", 17 where 11 were fixed).
pub(crate) fn placed_count(problem: &core::Problem) -> usize {
    let free = problem
        .components
        .iter()
        .filter(|component| !component.fixed)
        .count();
    let carried = problem
        .constraints
        .followers
        .iter()
        .filter(|follower| {
            problem
                .components
                .get(follower.leader)
                .is_some_and(|leader| !leader.fixed)
        })
        .count();
    free + carried
}

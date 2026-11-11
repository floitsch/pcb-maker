// Copyright (C) 2026 Toit contributors.

//! Before the placement: the board read, its constraints resolved, its
//! project's rules taken, the outline, keepout and side constraints
//! applied and the board lowered (`prepare_board`).

use super::*;

/// A board read, its constraints resolved and its project's rules taken,
/// and lowered as the placer places it (`prepare_board`).
pub struct PreparedBoard {
    pub pcb: Expr,
    pub config: KiCadBoardPlacerConfig,
    pub lowered: LoweredPlacement,
    pub outline_size: Option<[f64; 2]>,
    pub edges_chosen: BTreeMap<String, String>,
}

/// Moves every footprint of `pcb` to the pose of its footprint in `from`:
/// the one with its uuid, else the one with its reference (the n-th of
/// a repeated reference to the n-th). Footprints `from` lacks stay.
pub(super) fn take_poses(pcb: &mut Expr, from: &Expr) -> Result<usize, String> {
    let mut by_uuid: BTreeMap<String, Vec<[f64; 3]>> = BTreeMap::new();
    let mut by_reference: BTreeMap<String, Vec<[f64; 3]>> = BTreeMap::new();
    for footprint in from
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let at = form_at(footprint)?;
        if let Some(uuid) = crate::kicad_nets::footprint_uuid(footprint) {
            by_uuid.entry(uuid).or_default().push(at);
        }
        by_reference
            .entry(footprint_reference(footprint).unwrap_or_default())
            .or_default()
            .push(at);
    }
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    // A uuid the copies of a panel share goes to its n-th footprint.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut seen_uuid: BTreeMap<String, usize> = BTreeMap::new();
    let mut moved = 0;
    for footprint in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let nth = seen.entry(reference.clone()).or_default();
        let at = crate::kicad_nets::footprint_uuid(footprint)
            .and_then(|uuid| {
                let nth = seen_uuid.entry(uuid.clone()).or_default();
                *nth += 1;
                by_uuid
                    .get(&uuid)
                    .and_then(|ats| ats.get(*nth - 1).copied())
            })
            .or_else(|| {
                by_reference
                    .get(&reference)
                    .and_then(|ats| ats.get(*nth).copied())
            });
        *nth += 1;
        if let Some(at) = at {
            write_footprint_pose(
                footprint,
                core::Pose {
                    position: [at[0], at[1]],
                    angle: at[2],
                },
            )?;
            moved += 1;
        }
    }
    Ok(moved)
}

/// Reads `<board_id>.kicad_pcb` from `source_directory`, resolves the
/// constraints and the project's rules into `config`, applies the outline,
/// keepout and side constraints, and lowers the board: everything
/// `place_kicad_board` does before it places (also the legality census,
/// `pcb_kicad_lab::legality_kicad_board`, which takes the poses of
/// `poses_from`).
pub fn prepare_board(
    source_directory: &Path,
    board_id: &str,
    config: &KiCadBoardPlacerConfig,
    poses_from: Option<&Expr>,
) -> Result<PreparedBoard, String> {
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    // Errors up to the lowering are the input's (`input_error`).
    let source = fs::read_to_string(&source_board).map_err(|error| {
        crate::exit::input_error(format!(
            "failed to read {}: {error}",
            source_board.display()
        ))
    })?;
    let mut pcb = crate::canonical_order::parse_board(&source).map_err(crate::exit::input_error)?;
    if let Some(from) = poses_from {
        take_poses(&mut pcb, from)?;
    }
    let mut config =
        resolve_constraints(config, source_directory).map_err(crate::exit::input_error)?;
    // Parts added by the import (`add`) stay where the entry puts them.
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &mut config.constraints {
        crate::placement_constraints::expand_additions(constraints, &pcb)
            .map_err(crate::exit::input_error)?;
    }
    take_project_rules(&mut config, source_directory, board_id, &source_board);
    let edges_chosen = choose_any_edges(source_directory, board_id, &mut config)
        .map_err(crate::exit::input_error)?;
    let (outline_size, keepout_warnings) = apply_board_constraints(&mut pcb, &config)?;
    let mut lowered = lower_placement(&pcb, &config).map_err(crate::exit::input_error)?;
    lowered.constraint_warnings.extend(keepout_warnings);
    Ok(PreparedBoard {
        pcb,
        config,
        lowered,
        outline_size,
        edges_chosen,
    })
}

/// Fills the settings `config` leaves open from the board's project: the
/// clearances of its rules, the pairs' geometry for the pair rows, and
/// the severities of the courtyard, hole and silk checks.
fn take_project_rules(
    config: &mut KiCadBoardPlacerConfig,
    source_directory: &Path,
    board_id: &str,
    source_board: &Path,
) {
    if config.copper_clearance_mm.is_none()
        || config.copper_edge_clearance_mm.is_none()
        || config.hole_clearance_mm.is_none()
        || (config.tuning.mask_openings && config.solder_mask_to_copper_mm.is_none())
    {
        let rules = project_rules::resolve_project_rules(
            &source_directory.join(format!("{board_id}.kicad_pro")),
            source_board,
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
            source_board,
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
}

/// Applies the outline, keepout and side constraints to the board: the
/// outline's size where one is drawn, and the keepouts' warnings.
fn apply_board_constraints(
    pcb: &mut Expr,
    config: &KiCadBoardPlacerConfig,
) -> Result<(Option<[f64; 2]>, Vec<String>), String> {
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
            let (_, size, _) = local_body(footprint).map_err(crate::exit::input_error)?;
            part_area +=
                (size[0] + config.tuning.spacing_mm) * (size[1] + config.tuning.spacing_mm);
        }
        outline_size =
            Some(apply_outline(pcb, outline, part_area).map_err(crate::exit::input_error)?);
    }
    let mut keepout_warnings = Vec::new();
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &config.constraints {
        let held = if constraints.keepout.is_empty() {
            HeldParts::default()
        } else {
            held_parts(pcb, config).map_err(crate::exit::input_error)?
        };
        keepout_warnings =
            apply_keepouts(pcb, &constraints.keepout, &held).map_err(crate::exit::input_error)?;
        apply_sides(pcb, constraints).map_err(crate::exit::input_error)?;
    }
    Ok((outline_size, keepout_warnings))
}

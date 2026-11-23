// Copyright (C) 2026 Florian Loitsch.

//! A single alternate assignment when an automatic rear choice strands a part.

use super::*;

/// The one failed movable footprint that this run automatically put on
/// the back. Required rear sides and fixed parts are outside this retry.
fn stranded_rear<'a>(
    lowered: &'a LoweredPlacement,
    placement: &core::Placement,
) -> Option<&'a str> {
    let stranded: BTreeSet<_> = placement
        .unplaced
        .iter()
        .chain(&placement.illegal)
        .copied()
        .filter(|&part| {
            let component = &lowered.problem.components[part];
            !component.fixed
                && component.side == core::Side::Back
                && lowered
                    .back_side_assignments
                    .contains(&lowered.references[part])
        })
        .collect();
    if stranded.len() != 1 {
        return None;
    }
    let reference = lowered.references[*stranded.first()?].as_str();
    (!reference.is_empty()
        && !reference.contains(['*', '?', '\\'])
        && lowered
            .references
            .iter()
            .filter(|other| *other == reference)
            .count()
            == 1)
        .then_some(reference)
}

pub(super) fn recover(
    context: &PlaceContext,
    pcb: &mut Expr,
    lowered: &mut LoweredPlacement,
    picked: &mut Picked,
    layout_work_available: Option<f64>,
) -> Option<KiCadAutoRearRetry> {
    use KiCadAutoRearRetryStatus::{Accepted, Rejected, Skipped};

    let reference = stranded_rear(lowered, &picked.placement)?.to_owned();
    let mut report = KiCadAutoRearRetry {
        front_part: reference,
        status: Skipped,
        reason: None,
        work_seconds: 0.0,
    };
    let Some(available) = layout_work_available else {
        report.reason = Some("standalone_placement".into());
        return Some(report);
    };
    let pool_seconds = picked.work.pool as f64 / core::work_per_second();
    let remaining = available - pool_seconds.max(picked.work_seconds);
    let budget = (remaining * 0.4).min(35.0);
    if !available.is_finite() || budget < 10.0 {
        report.reason = Some("insufficient_remaining_work".into());
        return Some(report);
    }
    let expired = || {
        context
            .config
            .deadline
            .is_some_and(|d| d <= std::time::Instant::now())
    };
    if expired() {
        report.reason = Some("deadline".into());
        return Some(report);
    }
    eprintln!(
        "placer: retrying with {} on the front (up to {budget:.1} modeled seconds)",
        report.front_part
    );
    let (new_pcb, new_lowering, trial) = match place_front(
        context,
        lowered,
        &picked.placement.poses,
        &report.front_part,
        // Legalization can finish the current bounded step after its
        // own work deadline. Leave room for that step in the retry slice.
        0.8 * budget,
    ) {
        Ok(trial) => trial,
        Err(error) => {
            report.status = Rejected;
            report.reason = Some(error);
            return Some(report);
        }
    };
    report.work_seconds = trial.work_seconds;
    let pool_seconds = trial.work.pool as f64 / core::work_per_second();
    let reason = if expired() {
        Some("deadline")
    } else if pool_seconds.max(trial.work_seconds) > remaining {
        Some("layout_work_limit")
    } else if !trial.placement.unplaced.is_empty() || !trial.placement.illegal.is_empty() {
        Some("no_legal_placement")
    } else if !constraint_report(
        &new_lowering.problem,
        &trial.placement.poses,
        &new_lowering.references,
    )
    .into_iter()
    .chain(pair_row_statuses(
        &new_lowering.pair_rows,
        &new_lowering.problem,
        &trial.placement.poses,
        &new_lowering.references,
    ))
    .all(|status| status.requirement_satisfied())
    {
        Some("placement_constraints_missed")
    } else {
        None
    };
    let work_seconds = picked.work_seconds + trial.work_seconds;
    let work = seeds::SeedWork {
        pool: picked.work.pool + trial.work.pool,
        seeds: picked.work.seeds + trial.work.seeds,
        run: picked.work.run + trial.work.run,
        composition: picked.work.composition + trial.work.composition,
    };
    if let Some(reason) = reason {
        report.status = Rejected;
        report.reason = Some(reason.into());
    } else {
        report.status = Accepted;
        *pcb = new_pcb;
        *lowered = new_lowering;
        *picked = trial;
    }
    picked.work_seconds = work_seconds;
    picked.work = work;
    Some(report)
}

fn place_front(
    original: &PlaceContext,
    lowered: &LoweredPlacement,
    poses: &[core::Pose],
    reference: &str,
    budget: f64,
) -> Result<(Expr, LoweredPlacement, Picked), String> {
    let mut config = original.config.clone();
    config.work_seconds = Some(budget);
    let PreparedBoard {
        pcb,
        config,
        lowered: mut candidate,
        outline_size,
        edges_chosen,
    } = prepare::prepare_board_with_front_retry(
        original.source_directory,
        original.board_id,
        &config,
        None,
        Some(reference),
    )?;
    let context = PlaceContext {
        source_directory: original.source_directory,
        board_id: original.board_id,
        output_directory: original.output_directory,
        config,
        outline_size,
        // The original context has already resolved "any" edges.
        edges_chosen: if edges_chosen.is_empty() {
            original.edges_chosen.clone()
        } else {
            edges_chosen
        },
        started: original.started,
    };
    let pieces = place_in_pieces(&context, &pcb, &mut candidate)?;
    let is_on_front = candidate
        .references
        .iter()
        .position(|other| other == reference)
        .is_some_and(|index| candidate.problem.components[index].side == core::Side::Front);
    if !pieces.is_whole()
        || !is_on_front
        || candidate.references != lowered.references
        || candidate.problem.components.len() != poses.len()
        || context.edges_chosen != original.edges_chosen
        || context.outline_size != original.outline_size
        || candidate.problem.outline != lowered.problem.outline
        || candidate.problem.constraints.edges != lowered.problem.constraints.edges
        || candidate.problem.constraints.sides != lowered.problem.constraints.sides
        || candidate.problem.constraints.solid_cutouts != lowered.problem.constraints.solid_cutouts
        || candidate.problem.constraints.pad_rule_areas
            != lowered.problem.constraints.pad_rule_areas
        || lowered
            .problem
            .components
            .iter()
            .enumerate()
            .any(|(index, old)| {
                let new = &candidate.problem.components[index];
                old.fixed
                    && (!new.fixed
                        || new.side != old.side
                        || lowered.problem.poses[index] != candidate.problem.poses[index])
            })
    {
        return Err("placement geometry changed during rear retry".into());
    }
    add_routing_demand(&context, &pcb, &mut candidate)?;
    let mut settings = placer_config(&context, &pcb, &candidate)?;
    settings.start = Some(std::sync::Arc::new(core::Start {
        poses: poses
            .iter()
            .map(|&pose| {
                let [x, y, angle] = written_footprint_pose(pose);
                core::Pose {
                    position: [x, y],
                    angle,
                }
            })
            .collect(),
        parts: vec![true; poses.len()],
        stage: core::StartStage::Legalize,
    }));
    let race = skeleton_race(&context, &pcb, &candidate, &pieces);
    let seeds = place_seeds(&context, &candidate, &settings, &pieces, race.as_ref());
    let trial = pick(&context, &pcb, &candidate, &pieces, race.as_ref(), seeds);
    Ok((pcb, candidate, trial))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"(kicad_pcb (version 20240108)
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (net 1 "A") (net 2 "B")
        (gr_rect (start 0 0) (end 30 20) (layer "Edge.Cuts"))
        (footprint "Lib:IC" (layer "F.Cu") (at 16 10)
            (property "Reference" "U1")
            (fp_rect (start -3 -2) (end 3 2) (layer "F.CrtYd"))
            (pad "1" smd rect (at -1 1) (size 1 1) (layers "F.Cu") (net 1 "A"))
            (pad "2" smd rect (at 1 -1) (size 1 1) (layers "F.Cu") (net 2 "B")))
        (footprint "Lib:SW" (layer "B.Cu") (at 5 5) (locked yes)
            (property "Reference" "SW1")
            (fp_rect (start -2 -2) (end 2 2) (layer "B.CrtYd"))
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu") (net 1 "A"))))"#;

    #[test]
    fn retry_is_budgeted_and_only_changes_the_automatic_assignment() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pcb-maker-rear-retry-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("test.kicad_pcb");
        fs::write(&source, BOARD).unwrap();
        let config = KiCadBoardPlacerConfig {
            allow_back: vec!["U*".into(), "SW*".into()],
            work_seconds: Some(1.0),
            constraints: Some(KiCadConstraintsSource::Inline(
                serde_json::from_value(serde_json::json!({
                    "version": 1, "move_all": true, "back": ["SW1"], "fixed": ["SW1"],
                }))
                .unwrap(),
            )),
            ..Default::default()
        };
        let started = std::time::Instant::now();
        let mut board = crate::canonical_order::parse_board(BOARD).unwrap();
        if let Expr::List(forms) = &mut board {
            let footprint = forms
                .iter_mut()
                .find(|form| {
                    form.head() == Some("footprint")
                        && footprint_reference(form).as_deref() == Some("U1")
                })
                .unwrap();
            pcb_kicad_file::flip::flip_footprint(footprint).unwrap();
        }
        let mut lowered = lower_placement(&board, &config).unwrap();
        let part = lowered.references.iter().position(|r| r == "U1").unwrap();
        let switch = lowered.references.iter().position(|r| r == "SW1").unwrap();
        assert!(lowered.problem.components[switch].fixed);
        lowered.back_side_assignments.push("U1".into());
        let context = PlaceContext {
            source_directory: &root,
            board_id: "test",
            output_directory: &root,
            config,
            outline_size: None,
            edges_chosen: BTreeMap::new(),
            started,
        };
        let pieces = place_in_pieces(&context, &board, &mut lowered).unwrap();
        let settings = placer_config(&context, &board, &lowered).unwrap();
        let seeds = place_seeds(&context, &lowered, &settings, &pieces, None);
        let mut picked = pick(&context, &board, &lowered, &pieces, None, seeds);
        assert!(recover(&context, &mut board, &mut lowered, &mut picked, Some(100.0)).is_none());
        let work = picked.work_seconds;
        let held = picked.placement.poses[switch];
        picked.placement.unplaced.push(part);
        picked.placement.illegal.push(switch);
        assert_eq!(stranded_rear(&lowered, &picked.placement), Some("U1"));
        lowered.references[switch] = "U1".into();
        assert_eq!(stranded_rear(&lowered, &picked.placement), None);
        lowered.references[switch] = "SW1".into();
        let result = recover(&context, &mut board, &mut lowered, &mut picked, Some(0.0)).unwrap();
        assert_eq!(result.status, KiCadAutoRearRetryStatus::Skipped);
        assert_eq!(
            result.reason.as_deref(),
            Some("insufficient_remaining_work")
        );
        assert_eq!(picked.work_seconds, work);
        let result = recover(&context, &mut board, &mut lowered, &mut picked, Some(100.0)).unwrap();
        assert_eq!(
            result.status,
            KiCadAutoRearRetryStatus::Accepted,
            "{result:?}"
        );
        assert_eq!(result.front_part, "U1");
        assert!(result.work_seconds > 0.0);
        assert_eq!(picked.work_seconds, work + result.work_seconds);
        assert!(lowered.back_side_assignments.is_empty());
        assert_eq!(lowered.problem.components[part].side, core::Side::Front);
        assert_eq!(lowered.problem.components[switch].side, core::Side::Back);
        assert_eq!(picked.placement.poses[switch], held);
        assert_eq!(fs::read_to_string(&source).unwrap(), BOARD);
        fs::remove_dir_all(root).unwrap();
    }
}

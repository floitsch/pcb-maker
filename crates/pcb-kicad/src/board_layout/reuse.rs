// Copyright (C) 2026 Florian Loitsch.

//! A previous board supplies one tentative placement and a bounded route trial.

use super::setup::{LayoutContext, WorkLedger};
use super::*;
use crate::board_placer::LoweredPlacement;
use crate::board_router::{
    KiCadRouteReuse, RouteReuseIdentities, check_reused_copper, prepare_route_reuse_for_project,
    read_route_reuse, route_reuse_trial_budget,
};

pub(super) struct Warmed {
    pub pcb: Expr,
    pub problem: LoweredPlacement,
    pub first_route: report::FirstRoute,
    pub labels: crate::labels::KiCadLabelReport,
    pub routed: KiCadBoardRouterResult,
    pub prepublished_fill: Option<crate::FillPublication>,
}

pub(super) struct Trial {
    pub accepted: Option<Warmed>,
    pub evidence: KiCadRouteReuse,
    pub placement: KiCadRouteReusePlacement,
}

struct OfferedPlacement {
    pcb: Expr,
    problem: LoweredPlacement,
    moved: usize,
    relocated: usize,
}

pub(super) fn write_evidence(
    directory: &Path,
    routed: &mut KiCadBoardRouterResult,
    evidence: KiCadRouteReuse,
) -> Result<(), String> {
    routed.route_reuse = Some(evidence);
    let report = serde_json::to_vec_pretty(routed).map_err(|error| error.to_string())?;
    write_atomically(&directory.join("board-router.json"), &report)
}

/// Try previous coordinates for unheld footprints with the same footprint
/// name and side. Keep new footprints and the current board outline as placed.
/// Recheck legality and every original placement request before using any
/// prior pose. The current placement is available if the candidate fails.
fn offered_placement(
    previous: &Expr,
    current: &Expr,
    original: &LoweredPlacement,
    config: &KiCadBoardPlacerConfig,
    relaxation: &placer::Relaxation,
    repair_work: u64,
) -> Result<Option<OfferedPlacement>, String> {
    let footprints = |board: &Expr| {
        let mut by_reference = BTreeMap::new();
        let mut ambiguous = BTreeSet::new();
        for footprint in board
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
        {
            let Some(reference) = footprint_reference(footprint) else {
                continue;
            };
            let identity = (
                footprint
                    .children()
                    .get(1)
                    .and_then(Expr::atom)
                    .unwrap_or("")
                    .to_string(),
                form_atom(footprint, "layer", 1).unwrap_or("").to_string(),
            );
            if by_reference
                .insert(reference.clone(), (identity, form_at(footprint)))
                .is_some()
            {
                ambiguous.insert(reference);
            }
        }
        by_reference.retain(|reference, _| !ambiguous.contains(reference));
        by_reference
    };
    let identities = RouteReuseIdentities::of(previous, current);
    let previous = footprints(previous);
    let current_footprints = footprints(current);
    let attached: BTreeSet<_> = original
        .attached_marks
        .iter()
        .map(|&(_, part)| part)
        .collect();
    let mut poses = original.problem.poses.clone();
    let mut moved = 0;
    for (index, reference) in original.references.iter().enumerate() {
        let component = &original.problem.components[index];
        if component.fixed || attached.contains(&index) {
            continue;
        }
        let (Some((old_id, Ok([x, y, angle]))), Some((id, _))) = (
            identities
                .previous_reference(reference)
                .and_then(|old| previous.get(old)),
            current_footprints.get(reference),
        ) else {
            continue;
        };
        let angle_matches = |a: f64| ((a - angle + 180.0).rem_euclid(360.0) - 180.0).abs() <= 1e-6;
        if old_id != id || !component.angle_options.iter().copied().any(angle_matches) {
            continue;
        }
        let pose = &mut poses[index];
        if pose.position != [*x, *y] || !angle_matches(pose.angle) {
            *pose = placer::Pose {
                position: [*x, *y],
                angle: *angle,
            };
            moved += 1;
        }
    }
    if moved == 0 {
        return Ok(None);
    }
    let illegal = placer::legal::illegal_components(&original.problem, &poses);
    let mut relocated = 0;
    if !illegal.is_empty() {
        let (mut repair, mut keep) = (Vec::new(), Vec::new());
        for (index, reference) in original.references.iter().enumerate() {
            if illegal.contains(&index)
                && !original.problem.components[index].fixed
                && !attached.contains(&index)
                && identities.is_new(reference)
                && current_footprints.contains_key(reference)
            {
                repair.push(index);
            } else {
                keep.push(index);
            }
        }
        if !repair.is_empty() && repair_work > 0 {
            let mut repaired = poses.clone();
            let failed = placer::legal::legalize_keeping_bounded(
                &original.problem,
                &mut repaired,
                &keep,
                repair_work,
            );
            if failed.is_empty() {
                relocated = repair.iter().filter(|&&i| repaired[i] != poses[i]).count();
                poses = repaired;
            }
        }
    }
    let candidate = placement::posed(current, &poses, &[])?;
    let mut lowered = lower_placement(&candidate, config)?;
    if original.references != lowered.references {
        return Err(
            "the previous poses could not be reconciled with the current footprint order".into(),
        );
    }
    // A re-lowering at new coordinates can make an off-board footprint
    // fixed by default. Check the poses against the original movable parts,
    // outline and constraints, using the positions as they will be written.
    validate_placement(original, &lowered.problem.poses)?;
    relaxation.apply(&mut lowered.problem);
    for (component, initial) in lowered
        .problem
        .components
        .iter_mut()
        .zip(&original.problem.components)
    {
        component.fixed = initial.fixed;
    }
    Ok(Some(OfferedPlacement {
        pcb: candidate,
        problem: lowered,
        moved,
        relocated,
    }))
}

fn validate_placement(lowered: &LoweredPlacement, poses: &[placer::Pose]) -> Result<(), String> {
    let missed: Vec<_> = report::constraints_at(lowered, poses, None)
        .into_iter()
        .filter(|status| !status.requirement_satisfied())
        .map(|status| format!("{} {}", status.kind, status.part))
        .collect();
    if !missed.is_empty() {
        return Err(format!(
            "previous poses miss current constraints ({})",
            missed.join(", ")
        ));
    }
    let illegal = placer::legal::illegal_components(&lowered.problem, poses);
    if !illegal.is_empty() {
        let parts = illegal
            .iter()
            .map(|&index| lowered.references[index].as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "previous poses conflict with the current board geometry ({parts})"
        ));
    }
    Ok(())
}

/// At most one trial consumes a fixed, small part of the remaining work.
/// Current rules and the native comparison apply before any result is published.
#[allow(
    clippy::too_many_arguments,
    reason = "a trial needs the current layout and placer state"
)]
pub(super) fn try_reuse(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    prior_path: &Path,
    pcb: &Expr,
    problem: &LoweredPlacement,
    placer_config: &KiCadBoardPlacerConfig,
    relaxation: &placer::Relaxation,
    attached_marks: &[String],
) -> Result<Trial, String> {
    let previous = read_route_reuse(prior_path).map_err(crate::exit::input_error)?;
    let mut placement = KiCadRouteReusePlacement {
        moved_to_previous: 0,
        relocated_new_footprints: 0,
        saved_routable_nets: None,
        reason: None,
        work_seconds: 0.0,
    };
    let pose_work = placer::work();
    let repair_work = (route_reuse_trial_budget(ledger.left(ctx.work_deadline)).min(0.5)
        * placer::work_per_second()) as u64;
    let offer = offered_placement(
        &previous.board,
        pcb,
        problem,
        placer_config,
        relaxation,
        repair_work,
    );
    placement.work_seconds =
        placer::work().saturating_sub(pose_work) as f64 / placer::work_per_second();
    ledger.work += placement.work_seconds;
    let offered = match offer {
        Ok(offer) => offer,
        Err(reason) => {
            placement.reason = Some(reason);
            None
        }
    };
    let (mut candidate, problem) = if let Some(offer) = offered {
        placement.moved_to_previous = offer.moved;
        placement.relocated_new_footprints = offer.relocated;
        (offer.pcb, offer.problem)
    } else {
        let mut problem = lower_placement(pcb, placer_config)?;
        relaxation.apply(&mut problem.problem);
        (pcb.clone(), problem)
    };
    crate::board_router::refresh_pour_regions(&mut candidate, &ctx.router_config.add_pours)?;
    let fixed = write::fixed_parts(&problem);
    let labels = write::place_part_labels(ctx, &mut candidate, &fixed, attached_marks)?;
    let (borrowed, mut evidence) = prepare_route_reuse_for_project(
        &previous,
        &candidate,
        Some(&ctx.placed_board),
        &ctx.router_config,
    )?;
    evidence.remaining_budget_seconds = ledger.left(ctx.work_deadline).max(0.0);
    let started = std::time::Instant::now();
    let budget = route_reuse_trial_budget(evidence.remaining_budget_seconds);
    let mut trial = Trial {
        accepted: None,
        evidence,
        placement,
    };
    let Some(borrowed) = borrowed else {
        return Ok(trial);
    };
    let reject = |trial: &mut Trial, reason: String| {
        trial.evidence.status = "fallback".into();
        trial.evidence.reason = Some(reason);
    };
    if ctx.router_config.skip_native_verification {
        reject(&mut trial, "native KiCad verification is disabled".into());
        return Ok(trial);
    }
    if ctx.router_config.existing_tracks == KiCadExistingTracks::Strip {
        reject(
            &mut trial,
            "the router settings request stripping all tracks".into(),
        );
        return Ok(trial);
    }
    if budget <= 3.0 * ctx.check_seconds + 1.0 || ctx.wall_passed() {
        reject(
            &mut trial,
            "too little layout budget for routing and native checks".into(),
        );
        return Ok(trial);
    }
    if let Err(reason) = validate_placement(&problem, &problem.problem.poses) {
        reject(&mut trial, reason);
        return Ok(trial);
    }
    let work_before_trial = ledger.work;
    let scratch = ctx.output_directory.join("reuse");
    let baseline = scratch.join("baseline");
    let baseline_board = baseline.join(format!("{}.kicad_pcb", ctx.board_id));
    let baseline_report = scratch.join("baseline-drc.json");
    let trial_report = scratch.join("trial-drc.json");
    let staged = scratch.join("input");
    let routed_dir = scratch.join("trial");
    let staged_board = staged.join(format!("{}.kicad_pcb", ctx.board_id));
    let routed_board = routed_dir.join(format!("{}.kicad_pcb", ctx.board_id));
    let stage_baseline = || -> Result<(), String> {
        fs::create_dir(&scratch)
            .map_err(|error| format!("create reuse trial directory: {error}"))?;
        copy_directory_tree(&ctx.placed_directory, &baseline)?;
        fs::write(&baseline_board, format!("{}\n", encode(&candidate))).map_err(|e| e.to_string())
    };
    if let Err(reason) = stage_baseline() {
        reject(&mut trial, format!("cannot stage route reuse: {reason}"));
        return Ok(trial);
    }
    ledger.work += ledger.checks.add(1);
    if let Err(reason) = trial
        .evidence
        .write_drc(&baseline, ctx.board_id, &baseline_report)
        .and_then(|()| trial.evidence.check_baseline_parity(&baseline_report))
        .and_then(|()| {
            if ctx.wall_passed() {
                Err("the layout's wall guard elapsed during the input check".into())
            } else {
                Ok(())
            }
        })
        .and_then(|()| {
            copy_directory_tree(&ctx.placed_directory, &staged)
                .and_then(|()| {
                    fs::write(&staged_board, format!("{}\n", encode(&borrowed)))
                        .map_err(|e| e.to_string())
                })
                .map_err(|reason| format!("cannot stage route reuse: {reason}"))
        })
    {
        reject(&mut trial, reason);
        trial.evidence.trial_work_seconds = ledger.work - work_before_trial;
        trial.evidence.remaining_budget_seconds = ledger.left(ctx.work_deadline).max(0.0);
        return Ok(trial);
    }
    let mut router = ctx.router_config.clone();
    // KiCad loaded the exact staged input artwork. Preserve valid auxiliary
    // layers even when those layers are absent from its enabled layer table.
    router.route_reuse_preserve_input_layers = true;
    router.route_reuse_capture_refill = ctx.config.save_zone_fills
        && !ctx.config.stop_after_first_route
        && ledger.left(ctx.work_deadline) > budget + 2.0 * ctx.check_seconds;
    // The standalone ladder allows a 5% work tolerance. Keep its full
    // bound within the trial's share of this layout's remaining budget.
    router.ladder_budget_seconds = Some((budget - 2.0 * ctx.check_seconds) / 1.05);
    let mut routed = match route_kicad_board(&staged, ctx.board_id, &routed_dir, &router) {
        Ok(routed) => routed,
        Err(reason) => {
            ledger.work += budget - 2.0 * ctx.check_seconds;
            trial.evidence.trial_work_seconds = ledger.work - work_before_trial;
            trial.evidence.remaining_budget_seconds = ledger.left(ctx.work_deadline).max(0.0);
            reject(&mut trial, reason);
            return Ok(trial);
        }
    };
    ledger.checks.add(routed.kicad_checks_charged);
    let review = if crate::board_router::router_open(&routed) != 0 || ctx.wall_passed() {
        Err("the reuse trial did not route every connection within the layout budget".into())
    } else {
        check_reused_copper(&borrowed, &routed_board)
    }
    .and_then(|()| {
        if !trial
            .evidence
            .copy_closed_drc(&routed, &routed_dir, ctx.board_id, &trial_report)
        {
            ledger.work += ledger.checks.add(1);
            trial
                .evidence
                .write_drc(&routed_dir, ctx.board_id, &trial_report)?;
        }
        trial.evidence.compare_drc(&baseline_report, &trial_report)
    });
    trial.evidence.trial_work_seconds = ledger.work - work_before_trial + routed.work_seconds;
    trial.evidence.remaining_budget_seconds =
        (ledger.left(ctx.work_deadline) - routed.work_seconds).max(0.0);
    let reason = match review {
        Err(reason) => Some(reason),
        Ok(()) if !trial.evidence.new_findings.is_empty() => {
            Some("the reuse trial adds native findings".into())
        }
        Ok(_) if !routed.cut.is_empty() || ctx.wall_passed() => {
            Some("the layout's wall guard ended the reuse trial".into())
        }
        Ok(_) => None,
    };
    if let Some(reason) = reason {
        ledger.work += routed.work_seconds;
        reject(&mut trial, reason);
        return Ok(trial);
    }
    trial.evidence.status = "accepted".into();
    trial.evidence.reason = None;
    let scored = match crate::quality::score_kicad_board(&routed_dir, ctx.board_id) {
        Ok(scored) => scored,
        Err(reason) => {
            ledger.work += routed.work_seconds;
            reject(&mut trial, format!("cannot score reused routes: {reason}"));
            return Ok(trial);
        }
    };
    let first_route = report::FirstRoute {
        seconds: started.elapsed().as_secs_f64(),
        work_seconds: ledger.clock() + routed.work_seconds,
        open: 0,
        vias: scored.economy.vias,
        length_mm: scored.economy.track_length_mm,
    };
    trial.placement.saved_routable_nets =
        inspect_kicad_routable_connections(&routed_dir.join(format!("{}.kicad_pcb", ctx.board_id)))
            .ok()
            .map(|nets| nets.len());
    write_evidence(&routed_dir, &mut routed, trial.evidence.clone())?;
    // The routed directory still has the original checked parent, so
    // the proof can be used without rebasing any relative dependencies.
    let prepublished_fill = routed
        .closed_reuse_drc
        .as_ref()
        .and_then(|proof| proof.refill_if_current(&routed_dir, ctx.board_id))
        .and_then(|refill| {
            report::publish_fill(
                ctx,
                ledger,
                &routed,
                ctx.work_deadline,
                &routed_dir,
                Some(&refill),
            )
        });
    let result = ctx.output_directory.join("result");
    fs::rename(&routed_dir, &result).map_err(|error| format!("publish reuse trial: {error}"))?;
    trial.accepted = Some(Warmed {
        pcb: candidate,
        problem,
        first_route,
        labels,
        routed,
        prepublished_fill,
    });
    Ok(trial)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_pose_must_keep_the_current_pad_distance() {
        let board = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
          (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 30 20) (layer "Edge.Cuts"))
          (footprint "cap" (layer "F.Cu") (at 12 10)
            (property "Reference" "C1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "VCC")))
          (footprint "ic" (layer "F.Cu") (at 7 10)
            (property "Reference" "U1")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "VCC")))
          (footprint "cap" (layer "F.Cu") (at 20 10)
            (property "Reference" "C2")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "VCC"))))"#;
        let current = parse(board).unwrap();
        let old = |x: &str| {
            let mut pcb = parse(&board.replace("(at 12 10)", &format!("(at {x} 10)"))).unwrap();
            let Expr::List(items) = &mut pcb else {
                unreachable!()
            };
            items.retain(|item| footprint_reference(item).as_deref() != Some("C2"));
            pcb
        };
        let config = KiCadBoardPlacerConfig {
            constraints: Some(KiCadConstraintsSource::Inline(
                serde_json::from_str(
                    r#"{
                "version": 1,
                "near": [{"part":"C1","pad":"1","pin_of":"U1:1","max_mm":6}]
            }"#,
                )
                .unwrap(),
            )),
            ..Default::default()
        };
        let lowered = lower_placement(&current, &config).unwrap();
        let relaxation = placer::Relaxation {
            spacing: lowered.problem.spacing,
            grid: lowered.problem.grid,
            halo_scale: 1.0,
            small_halo_scale: 1.0,
            edge_inset: true,
            tight: false,
            edge_copper: false,
            edge_rule: 0.0,
            courtyard_spacing: false,
            pad_clearance: true,
            halos: None,
            on_marks: Vec::new(),
        };
        let proposed = |previous: &Expr| {
            offered_placement(previous, &current, &lowered, &config, &relaxation, 0)
        };
        assert!(matches!(proposed(&old("13.005814")), Err(reason) if reason.contains("near C1")));
        let off_board = parse(&encode(&old("-2")).replace("(at 7 10)", "(at -8 10)")).unwrap();
        assert!(matches!(proposed(&off_board), Err(reason) if reason.contains("board geometry")));
        let proposal = proposed(&old("13")).unwrap().unwrap();
        let accepted = proposal.problem;
        assert_eq!(proposal.moved, 1);
        assert_eq!(proposal.relocated, 0);
        assert_eq!(accepted.problem.poses[0].position, [13.0, 10.0]);
        assert_eq!(accepted.problem.poses[2].position, [20.0, 10.0]);
        assert_eq!(lowered.problem.poses[0].position, [12.0, 10.0]);

        let crowded = parse(&board.replace("(at 20 10)", "(at 13.5 10)")).unwrap();
        let lowered = lower_placement(&crowded, &config).unwrap();
        let proposal = offered_placement(
            &old("13"),
            &crowded,
            &lowered,
            &config,
            &relaxation,
            100_000,
        )
        .unwrap()
        .unwrap();
        let accepted = proposal.problem;
        assert_eq!(proposal.moved, 1);
        assert_eq!(proposal.relocated, 1);
        assert_eq!(accepted.problem.poses[0].position, [13.0, 10.0]);
        assert_eq!(accepted.problem.poses[1].position, [7.0, 10.0]);
        assert_ne!(accepted.problem.poses[2].position, [13.5, 10.0]);
        assert_eq!(lowered.problem.poses[2].position, [13.5, 10.0]);

        let linked = |reference_1, reference_2| {
            board
                .replace(
                    r#"(property "Reference" "C1")"#,
                    r#"(path "/11111111") (property "Reference" "FIRST_REF")"#,
                )
                .replace(
                    r#"(property "Reference" "C2")"#,
                    r#"(path "/22222222") (property "Reference" "SECOND_REF")"#,
                )
                .replace("FIRST_REF", reference_1)
                .replace("SECOND_REF", reference_2)
        };
        let current = parse(&linked("C1", "C2")).unwrap();
        let previous_text = linked("C2", "C1").replace("(at 12 10)", "(at 13 10)");
        let previous = parse(&previous_text).unwrap();
        let lowered = lower_placement(&current, &config).unwrap();
        let proposal = offered_placement(&previous, &current, &lowered, &config, &relaxation, 0)
            .unwrap()
            .unwrap();
        assert_eq!(proposal.moved, 1);
        assert_eq!(proposal.problem.references, lowered.references);
        assert_eq!(proposal.problem.problem.poses[0].position, [13.0, 10.0]);
        let conflicting = parse(&previous_text.replace("/11111111", "/aaaaaaaa")).unwrap();
        assert!(
            offered_placement(&conflicting, &current, &lowered, &config, &relaxation, 0)
                .unwrap()
                .is_none()
        );
    }
}

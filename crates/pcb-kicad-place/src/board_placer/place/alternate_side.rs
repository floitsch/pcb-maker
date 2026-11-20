// Copyright (C) 2026 Florian Loitsch.

//! Assign user-permitted surface-mount bodies to the less crowded side.

use super::*;
use crate::kicad_nets::glob_matches;

/// A part with holes, internal or rear copper, a footprint rule area, or
/// only an access pad needs its existing side. Ordinary front-side SMD
/// pad stacks have an unambiguous KiCad mirror.
fn front_smd(footprint: &Expr) -> bool {
    if form_atom(footprint, "layer", 1) != Some("F.Cu") || footprint.child("zone").is_some() {
        return false;
    }
    let mut pad_numbers = BTreeSet::new();
    for pad in footprint
        .children()
        .iter()
        .filter(|form| form.head() == Some("pad"))
    {
        let fields = pad.children();
        let layers = pad.child("layers").map(|form| &form.children()[1..]);
        let Some(layers) = layers else { return false };
        if fields.get(2).and_then(Expr::atom) != Some("smd")
            || pad.child("drill").is_some()
            || pad.child("padstack").is_some()
            || layers.is_empty()
            || layers
                .iter()
                .any(|layer| !matches!(layer.atom(), Some("F.Cu" | "F.Paste" | "F.Mask")))
        {
            return false;
        }
        let number = fields.get(1).and_then(Expr::atom).unwrap_or_default();
        if layers.iter().any(|layer| layer.atom() == Some("F.Cu")) {
            if !number.is_empty() {
                pad_numbers.insert(number);
            }
        } else if !number.is_empty() || pad.child("net").is_some() {
            return false;
        }
    }
    pad_numbers.len() >= 2
}

fn movable(problem: &core::Problem, index: usize) -> bool {
    let part = &problem.components[index];
    let constraints = &problem.constraints;
    !part.fixed
        && part.side == core::Side::Front
        && part.far_courtyard.is_empty()
        && part.far_side.is_empty()
        && part.far_copper.is_empty()
        && !constraints.kept.iter().any(|held| held.part == index)
        && !constraints.edges.iter().any(|(part, ..)| *part == index)
        && !constraints
            .overhangs
            .iter()
            .any(|(part, ..)| *part == index)
        && !constraints
            .followers
            .iter()
            .any(|follower| follower.part == index || follower.leader == index)
        && !constraints.relations[..constraints.relations.len() - constraints.automatic]
            .iter()
            .any(|relation| relation.involves(index))
}

/// Try a single stranded surface-mount footprint on the rear without moving
/// any other footprint. A privately proven fit can inform the next action,
/// but adopting it requires an explicit `allow_back` permission.
pub(super) fn fit_stranded_part(
    context: &PlaceContext,
    pcb: &mut Expr,
    lowered: &mut LoweredPlacement,
    picked: &mut Picked,
) -> Option<String> {
    let placement = &picked.placement;
    let problem = &lowered.problem;
    let stranded: BTreeSet<_> = placement
        .unplaced
        .iter()
        .chain(&placement.illegal)
        .copied()
        .filter(|&index| !problem.components[index].fixed)
        .collect();
    let index = *stranded.first()?;
    if stranded.len() != 1
        || !movable(problem, index)
        || !problem.components[index].slots.is_empty()
    {
        return None;
    }
    let reference = lowered.references[index].clone();
    let may_move = |patterns: &[String]| {
        patterns
            .iter()
            .any(|pattern| glob_matches(pattern, &reference))
    };
    let config = &context.config;
    let candidate_form = pcb
        .children()
        .iter()
        .enumerate()
        .filter(|(_, form)| form.head() == Some("footprint"))
        .nth(index)
        .filter(|(_, form)| front_smd(form))?
        .0;
    if lowered
        .references
        .iter()
        .filter(|r| **r == reference)
        .count()
        != 1
        || lowered
            .attached_marks
            .iter()
            .any(|&(_, part)| part == index)
        || matches!(&config.constraints, Some(KiCadConstraintsSource::Inline(brief)) if may_move(&brief.front) || may_move(&brief.hollow))
    {
        return None;
    }
    let seconds = config
        .work_seconds
        .map_or(1.0, |budget| (budget - picked.work_seconds).clamp(0.0, 1.0));
    if seconds <= 0.0
        || config
            .deadline
            .is_some_and(|d| d <= std::time::Instant::now())
    {
        return None;
    }

    let mut board = pcb.clone();
    let Expr::List(items) = &mut board else {
        return None;
    };
    pcb_kicad_file::flip::flip_footprint(&mut items[candidate_form]).ok()?;
    let mut candidate = lower_placement(&board, config).ok()?;
    if candidate.references != lowered.references {
        return None;
    }
    let mut relaxed = candidate.problem.clone();
    placement.relaxation.apply(&mut relaxed);
    let mut poses = placement.poses.clone();
    poses[index].angle = -poses[index].angle;
    let keep: Vec<_> = (0..poses.len()).filter(|&part| part != index).collect();
    let start = core::work();
    let unplaced = core::legal::legalize_keeping_bounded(
        &relaxed,
        &mut poses,
        &keep,
        (seconds * core::work_per_second()) as u64,
    );
    let legal = unplaced.is_empty()
        && keep
            .iter()
            .all(|&part| poses[part] == placement.poses[part])
        && core::legal::illegal_components(&relaxed, &poses).is_empty()
        && constraint_report(&candidate.problem, &poses, &candidate.references)
            .into_iter()
            .chain(pair_row_statuses(
                &candidate.pair_rows,
                &candidate.problem,
                &poses,
                &candidate.references,
            ))
            .all(|status| status.requirement_satisfied());
    let work = core::work().saturating_sub(start);
    picked.work_seconds += work as f64 / core::work_per_second();
    picked.work.pool += work;
    picked.work.seeds += work;
    if !legal
        || config
            .deadline
            .is_some_and(|d| d <= std::time::Instant::now())
    {
        return None;
    }
    if !may_move(&config.allow_back) {
        let value = serde_json::json!([reference]);
        return Some(format!(
            "{reference} fits on the back with the other parts held. If rear mounting is allowed, add {value} to the placer's allow_back setting and retry."
        ));
    }

    candidate
        .constraint_warnings
        .extend(lowered.constraint_warnings.iter().cloned());
    candidate.constraint_warnings.sort();
    candidate.constraint_warnings.dedup();
    candidate
        .back_side_assignments
        .clone_from(&lowered.back_side_assignments);
    candidate.back_side_assignments.push(reference);
    candidate.back_side_assignments.sort();
    let placement = &mut picked.placement;
    placement.unplaced.clear();
    placement.illegal.clear();
    placement.overfull = false;
    placement.constraints = core::constraints::report(&candidate.problem, &poses);
    placement.wirelength_final = candidate.problem.wirelength(&poses);
    placement.work += work;
    placement.frames.push(core::Frame {
        iteration: placement
            .frames
            .last()
            .map_or(0, |frame| frame.iteration + 1),
        overflow: 0.0,
        wirelength: placement.wirelength_final,
        poses: poses.clone(),
    });
    placement.stage_poses.push(("back_side_fit", poses.clone()));
    placement.poses = poses;
    placement.exits = None;
    picked.alternatives.clear();
    picked.alternative_seed_metrics.clear();
    picked.alternative_exits.clear();
    *pcb = board;
    *lowered = candidate;
    None
}

pub(super) fn move_to_back(
    pcb: &mut Expr,
    lowered: &LoweredPlacement,
    allowed: &[String],
    exclude_from_automatic_back: Option<&str>,
) -> Result<Vec<String>, String> {
    if allowed.is_empty() {
        return Ok(Vec::new());
    }
    let problem = &lowered.problem;
    let room = crate::placement_room::room(problem, &lowered.references, false);
    let pair = core::legal::region_pair_conflict(problem);
    if room.utilization[0] <= room.target_utilization && pair.is_none() {
        return Ok(Vec::new());
    }
    let mut candidates: Vec<_> = rear_candidates(pcb, lowered)
        .filter(|(index, _, _)| {
            let reference = lowered.references[*index].as_str();
            Some(reference) != exclude_from_automatic_back && permitted(allowed, reference)
        })
        .collect();
    let front_target = room.usable_mm2[0] * room.target_utilization;
    let back_target = room.usable_mm2[1] * room.target_utilization;
    let mut selected = Vec::new();
    let mut transferred = 0.0;
    if let Some(pair) = pair {
        let Some(position) = [pair.region_part, pair.other_part]
            .into_iter()
            .find_map(|wanted| {
                candidates.iter().position(|(index, _, area)| {
                    *index == wanted && room.parts_mm2[1] + area <= back_target
                })
            })
        else {
            // Moving unrelated parts cannot resolve the proven pair conflict.
            return Ok(Vec::new());
        };
        let candidate = candidates.swap_remove(position);
        transferred += candidate.2;
        selected.push(candidate);
    }
    // Move the fewest permitted bodies that cover the estimated spill.
    // Placement and routing still check the resulting geometry normally.
    candidates.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    for candidate @ (_, _, area) in candidates {
        if room.parts_mm2[0] - transferred <= front_target {
            break;
        }
        if room.parts_mm2[1] + transferred + area <= back_target {
            transferred += area;
            selected.push(candidate);
        }
    }
    let mut moved = Vec::new();
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    for (index, item_index, _) in selected {
        pcb_kicad_file::flip::flip_footprint(&mut items[item_index])?;
        moved.push(lowered.references[index].clone());
    }
    moved.sort();
    Ok(moved)
}

fn permitted(patterns: &[String], reference: &str) -> bool {
    patterns
        .iter()
        .any(|pattern| glob_matches(pattern, reference))
}

/// Footprint and PCB-form indices and the modeled area of each movable
/// front-side SMD footprint, prior to checking rear-placement permission.
fn rear_candidates<'a>(
    pcb: &'a Expr,
    lowered: &'a LoweredPlacement,
) -> impl Iterator<Item = (usize, usize, f64)> + 'a {
    let problem = &lowered.problem;
    pcb.children()
        .iter()
        .enumerate()
        .filter(|(_, form)| form.head() == Some("footprint"))
        .enumerate()
        .filter_map(move |(index, (item_index, footprint))| {
            let reference = &lowered.references[index];
            if !movable(problem, index)
                || !problem.components[index].hollow.is_empty()
                || !front_smd(footprint)
                || lowered
                    .references
                    .iter()
                    .filter(|other| *other == reference)
                    .count()
                    != 1
            {
                return None;
            }
            let [width, height] = problem.components[index].body_size;
            let area = (width + problem.spacing) * (height + problem.spacing);
            Some((index, item_index, area))
        })
}

/// Candidate permissions for a crowded front side. These are based on
/// geometry and area estimates, without trying a placement or granting
/// permission. A proven single-part recovery is reported separately.
pub(super) fn rear_recovery(
    pcb: &Expr,
    lowered: &LoweredPlacement,
    room: &crate::placement_room::KiCadRoom,
    placement: &core::Placement,
    allowed: &[String],
) -> Option<KiCadRearPlacementRecovery> {
    let problem = &lowered.problem;
    let affected: BTreeSet<_> = placement
        .unplaced
        .iter()
        .chain(&placement.illegal)
        .copied()
        .filter(|&index| !problem.components[index].fixed)
        .collect();
    let rear_headroom_mm2 = room.usable_mm2[1] * room.target_utilization - room.parts_mm2[1];
    if affected.len() < 2
        || room.utilization[0] <= room.target_utilization
        || rear_headroom_mm2 <= 0.0
    {
        return None;
    }
    let passives = passive_mask(pcb, problem.components.len());
    let mut eligible: Vec<_> = rear_candidates(pcb, lowered)
        .filter(|&(index, _, area)| {
            passives[index]
                && area <= rear_headroom_mm2
                && problem.components[index].slots.is_empty()
                && !lowered
                    .attached_marks
                    .iter()
                    .any(|&(_, part)| part == index)
                && !lowered.references[index].is_empty()
                && !lowered.references[index].contains(['*', '?'])
                && !permitted(allowed, &lowered.references[index])
        })
        .map(|(index, _, _)| index)
        .collect();
    if !eligible.iter().any(|index| affected.contains(index))
        || core::legal::region_pair_conflict(problem).is_some_and(|pair| {
            !eligible.contains(&pair.region_part) && !eligible.contains(&pair.other_part)
        })
    {
        return None;
    }
    eligible.sort_by_key(|&index| (!affected.contains(&index), &lowered.references[index]));
    Some(KiCadRearPlacementRecovery {
        allow_back_additions: eligible
            .into_iter()
            .map(|index| lowered.references[index].clone())
            .collect(),
        affected_movable: affected.len(),
        rear_headroom_mm2: (rear_headroom_mm2 * 10.0).round() / 10.0,
    })
}

/// Passive SMD footprints whose modeled body area could be moved off a
/// crowded front face. The layout verdict uses this evidence when routing
/// left native opens. A placement on the other face still needs a trial.
pub(super) fn rear_routing_candidates(
    pcb: &Expr,
    lowered: &LoweredPlacement,
    room: &crate::placement_room::KiCadRoom,
    placement: &core::Placement,
    config: &KiCadBoardPlacerConfig,
) -> Option<KiCadRearRoutingCandidates> {
    let rear_headroom_mm2 = room.usable_mm2[1] * room.target_utilization - room.parts_mm2[1];
    if !placement.unplaced.is_empty()
        || !placement.illegal.is_empty()
        || room.utilization[0] <= room.target_utilization
        || rear_headroom_mm2 <= 0.0
    {
        return None;
    }
    let passives = passive_mask(pcb, lowered.problem.components.len());
    let mut candidates: Vec<_> = rear_candidates(pcb, lowered)
        .filter_map(|(index, _, area)| {
            let reference = &lowered.references[index];
            let constrained_front = matches!(
                &config.constraints,
                Some(KiCadConstraintsSource::Inline(brief))
                    if permitted(&brief.front, reference) || permitted(&brief.hollow, reference)
            );
            (passives[index]
                && area.is_finite()
                && area > 0.0
                && lowered.problem.components[index].slots.is_empty()
                && !lowered
                    .attached_marks
                    .iter()
                    .any(|&(_, part)| part == index)
                && !reference.is_empty()
                && !reference.contains(['*', '?'])
                && !permitted(&config.allow_back, reference)
                && !constrained_front)
                .then_some((reference, area))
        })
        .collect();
    candidates.sort_by(|(a, a_area), (b, b_area)| b_area.total_cmp(a_area).then(a.cmp(b)));
    let need = room.parts_mm2[0] - room.usable_mm2[0] * room.target_utilization;
    let mut moved_area = 0.0;
    let mut refs = Vec::new();
    for (reference, area) in candidates {
        if moved_area >= need {
            break;
        }
        if moved_area + area <= rear_headroom_mm2 {
            refs.push(reference.clone());
            moved_area += area;
        }
    }
    (!refs.is_empty()).then(|| {
        refs.sort();
        KiCadRearRoutingCandidates {
            allow_back_additions: refs,
            front_body_area_mm2: (moved_area * 10.0).round() / 10.0,
            rear_headroom_mm2: (rear_headroom_mm2 * 10.0).round() / 10.0,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"(kicad_pcb (version 20240108)
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (net 1 "SIG") (net 2 "GND")
        (gr_rect (start 0 0) (end 22 20) (layer "Edge.Cuts"))
        (footprint "Lib:MCU" (layer "F.Cu") (at 11 10)
            (property "Reference" "U1")
            (fp_rect (start -4 -4) (end 4 4) (layer "F.CrtYd"))
            (pad "1" smd rect (at -1 1) (size 1 1) (layers "F.Cu" "F.Paste" "F.Mask") (net 1 "SIG"))
            (pad "2" smd rect (at 1 -1) (size 1 1) (layers "F.Cu" "F.Paste" "F.Mask") (net 2 "GND"))
            (pad "" smd rect (at 1 -1) (size 1 1) (layers "F.Paste")))
        (footprint "Lib:Socket" (layer "F.Cu") (at 14 14)
            (property "Reference" "J1")
            (fp_rect (start -4.5 -4.5) (end 4.5 4.5) (layer "F.CrtYd"))
            (pad "1" thru_hole circle (at -1 0) (size 1 1) (drill 0.5) (layers "*.Cu" "*.Mask") (net 1 "SIG"))
            (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 2 "GND"))))"#;

    fn inputs(front: Vec<String>) -> (Expr, KiCadBoardPlacerConfig) {
        let constraints = serde_json::json!({
            "version": 1,
            "move_all": true,
            "front": front,
            "region": [{"parts": ["U1"], "origin": "absolute", "x": [7, 15], "y": [6, 14]}],
        });
        (
            parse(BOARD).unwrap(),
            KiCadBoardPlacerConfig {
                constraints: Some(KiCadConstraintsSource::Inline(
                    serde_json::from_value(constraints).unwrap(),
                )),
                ..Default::default()
            },
        )
    }

    #[test]
    fn crowded_front_advice_names_only_eligible_passives_not_already_permitted() {
        let (mut board, mut config) = inputs(vec!["C2".into()]);
        let Some(KiCadConstraintsSource::Inline(brief)) = &mut config.constraints else {
            unreachable!()
        };
        brief.region.clear();
        brief.fixed.push("C3".into());
        let model = encode(board.child("footprint").unwrap());
        let Expr::List(items) = &mut board else {
            unreachable!()
        };
        for reference in ["C1", "C2", "C3", "R1"] {
            items.push(parse(&model.replace("\"U1\"", &format!("\"{reference}\""))).unwrap());
        }
        let lowered = lower_placement(&board, &config).unwrap();
        let mut place_config = core::Config::new();
        place_config.skip_anneal = true;
        let mut placement = core::place(&lowered.problem, &place_config);
        placement.unplaced = vec![0, 1, 2, 3, 4, 5];
        placement.illegal.clear();
        let mut room = crate::placement_room::KiCadRoom {
            utilization: [0.95, 0.15],
            target_utilization: 0.7,
            usable_mm2: [1000.0, 1000.0],
            parts_mm2: [950.0, 150.0],
            ..Default::default()
        };
        let advice = rear_recovery(&board, &lowered, &room, &placement, &["R*".into()]).unwrap();
        assert_eq!(advice.allow_back_additions, ["C1"]);
        assert_eq!(advice.affected_movable, 5);
        assert_eq!(advice.rear_headroom_mm2, 550.0);
        config.allow_back = vec!["R*".into()];
        assert!(rear_routing_candidates(&board, &lowered, &room, &placement, &config).is_none());
        let unplaced = std::mem::take(&mut placement.unplaced);
        let advice = rear_routing_candidates(&board, &lowered, &room, &placement, &config).unwrap();
        assert_eq!(advice.allow_back_additions, ["C1"]);
        assert_eq!(advice.rear_headroom_mm2, 550.0);
        // No new permission remains. The permanently constrained C2/C3
        // still must not be suggested.
        config.allow_back.push("C1".into());
        assert!(rear_routing_candidates(&board, &lowered, &room, &placement, &config).is_none());
        config.allow_back.clear();
        let full_room = crate::placement_room::KiCadRoom {
            parts_mm2: [950.0, 700.0],
            ..room.clone()
        };
        assert!(
            rear_routing_candidates(&board, &lowered, &full_room, &placement, &config).is_none()
        );
        placement.unplaced = unplaced;
        assert!(rear_recovery(&board, &lowered, &room, &placement, &["*".into()]).is_none());
        room.parts_mm2[1] = 700.0;
        assert!(rear_recovery(&board, &lowered, &room, &placement, &[]).is_none());
    }

    #[test]
    fn a_stranded_part_needs_permission_and_keeps_its_neighbours_in_place() {
        let (_, mut config) = inputs(Vec::new());
        let mut board =
            parse(&BOARD.replace(r#"(fp_rect (start -4 -4) (end 4 4) (layer "F.CrtYd"))"#, ""))
                .unwrap();
        if let Some(KiCadConstraintsSource::Inline(brief)) = &mut config.constraints {
            brief.fixed.push("J1".into());
        }
        let mut context = PlaceContext {
            source_directory: Path::new("."),
            output_directory: Path::new("."),
            board_id: "test",
            config,
            outline_size: None,
            edges_chosen: BTreeMap::new(),
            started: std::time::Instant::now(),
        };
        let mut lowered = lower_placement(&board, &context.config).unwrap();
        assert!(!lowered.problem.components[0].hollow.is_empty());
        assert!(lowered.problem.components[1].fixed);
        let mut config = core::Config::new();
        config.skip_anneal = true;
        config.work_limit = Some((0.2 * core::work_per_second()) as u64);
        let mut picked = Picked {
            placement: core::place(&lowered.problem, &config),
            seed_kept: 0,
            work_seconds: 0.0,
            seeds_placed: 1,
            work: super::seeds::SeedWork {
                pool: 0,
                seeds: 0,
                run: 1,
                composition: 0,
            },
            skeleton_race: None,
            panel_copies: None,
            alternatives: Vec::new(),
            alternative_seed_metrics: Vec::new(),
            pad_objective_composition: None,
            alternative_exits: Vec::new(),
        };
        picked.placement.poses.clone_from(&lowered.problem.poses);
        picked.placement.unplaced = vec![0];
        picked.placement.illegal = vec![0, 1];
        // The mirrored part must clear the fixed neighbour's through-hole pad.
        let poses = picked.placement.poses.clone();
        let hint = fit_stranded_part(&context, &mut board, &mut lowered, &mut picked).unwrap();
        assert!(hint.contains("[\"U1\"]"), "{hint}");
        assert_eq!(picked.placement.poses, poses);
        assert_eq!(
            form_atom(board.child("footprint").unwrap(), "layer", 1),
            Some("F.Cu")
        );

        context.config.allow_back = vec!["U*".into()];
        assert!(fit_stranded_part(&context, &mut board, &mut lowered, &mut picked).is_none());
        assert_eq!(lowered.back_side_assignments, ["U1"]);
        assert!(picked.placement.unplaced.is_empty());
        assert!(picked.placement.illegal.is_empty());
        assert_eq!(picked.placement.poses[1], poses[1]);
        assert_eq!(
            form_atom(board.child("footprint").unwrap(), "layer", 1),
            Some("B.Cu")
        );
    }

    #[test]
    fn a_permitted_region_part_can_move_to_the_back_with_its_pads() {
        let (mut board, config) = inputs(Vec::new());
        let original = lower_placement(&board, &config).unwrap();
        assert!(core::legal::region_pair_conflict(&original.problem).is_some());
        let approved = vec!["U*".into(), "J1".into()];
        assert_eq!(
            move_to_back(&mut board, &original, &approved, None).unwrap(),
            ["U1"]
        );
        let after = lower_placement(&board, &config).unwrap();
        assert!(core::legal::region_pair_conflict(&after.problem).is_none());
        assert_eq!(
            after.problem.constraints.regions,
            original.problem.constraints.regions
        );
        assert_eq!(after.problem.components[0].side, core::Side::Back);
        assert_eq!(after.problem.components[1].side, core::Side::Front);
        let u1 = board.child("footprint").unwrap();
        let pad = u1.child("pad").unwrap();
        assert_eq!(form_atom(pad, "layers", 1), Some("B.Cu"));
        assert_eq!(form_atom(pad, "layers", 2), Some("B.Paste"));
        assert_eq!(form_atom(pad, "layers", 3), Some("B.Mask"));
        assert_eq!(form_f64(pad, "at", 2).unwrap(), -1.0);
        assert_eq!(form_atom(pad, "net", 2), Some("SIG"));
        let paste = u1
            .children()
            .iter()
            .find(|form| form.head() == Some("pad") && form.children()[1].atom() == Some(""))
            .unwrap();
        assert_eq!(form_atom(paste, "layers", 1), Some("B.Paste"));
    }

    #[test]
    fn a_required_front_side_and_a_drilled_socket_cannot_move() {
        let (mut board, config) = inputs(vec!["U1".into()]);
        let original = lower_placement(&board, &config).unwrap();
        let approved = vec!["*".into()];
        assert!(
            move_to_back(&mut board, &original, &approved, None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(encode(&board), encode(&parse(BOARD).unwrap()));
    }

    #[test]
    fn rear_permission_can_relieve_crowding_below_full_area() {
        let (_, config) = inputs(Vec::new());
        let config = KiCadBoardPlacerConfig {
            constraints: Some(KiCadConstraintsSource::Inline(
                serde_json::from_value(serde_json::json!({
                    "version": 1,
                    "move_all": true,
                    "front": ["R1"],
                }))
                .unwrap(),
            )),
            ..config
        };
        let footprints: String = (1..=5)
            .map(|index| {
                format!(
                    r#"(footprint "Lib:R" (layer "F.Cu") (at 9 9)
                        (property "Reference" "R{index}")
                        (fp_rect (start -5 -5) (end 5 5) (layer "F.CrtYd"))
                        (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net 1 "SIG"))
                        (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 2 "GND")))"#
                )
            })
            .collect();
        let outline = |width| {
            parse(&format!(
                r#"(kicad_pcb (version 20240108) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
                    (net 1 "SIG") (net 2 "GND")
                    (gr_rect (start 0 0) (end {width} 24) (layer "Edge.Cuts")) {footprints})"#
            ))
            .unwrap()
        };
        let mut dense = outline(28);
        let lowered = lower_placement(&dense, &config).unwrap();
        let room = crate::placement_room::room(&lowered.problem, &lowered.references, false);
        assert!(room.utilization[0] > room.target_utilization);
        assert!(room.utilization[0] < 1.0);
        let allowed = ["R*".into()];
        let moved = move_to_back(&mut dense, &lowered, &allowed, None).unwrap();
        assert!(!moved.is_empty());
        assert!(!moved.iter().any(|reference| reference == "R1"));
        let lowered = lower_placement(&dense, &config).unwrap();
        let room = crate::placement_room::room(&lowered.problem, &lowered.references, false);
        assert!(room.utilization[0] <= room.target_utilization);

        let mut roomy = outline(40);
        let lowered = lower_placement(&roomy, &config).unwrap();
        assert!(
            move_to_back(&mut roomy, &lowered, &allowed, None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(encode(&roomy), encode(&outline(40)));
    }
}

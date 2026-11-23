// Copyright (C) 2026 Toit contributors.

//! Unit tests of the placement's stages, each alone.

use super::output::{check_written_pads, write_poses};
use super::prepare::take_poses;
use super::report::{placement_hints, repeated_references, unplaced_parts};
use super::seeds::{rank, sort_by_rank};
use super::*;

/// Two fixed connectors and a resistor between them, on a 40 x 20 board.
const BOARD: &str = r#"(kicad_pcb (version 20240108)
    (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
    (net 0 "") (net 1 "A") (net 2 "B")
    (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
    (footprint "Lib:J" (layer "F.Cu") (at 3 10) (locked yes)
        (uuid "j1")
        (property "Reference" "J1")
        (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd"))
        (pad "1" smd rect (at 0 0) (size 1.5 1.5) (layers "F.Cu") (net 1 "A")))
    (footprint "Lib:J" (layer "F.Cu") (at 37 10) (locked yes)
        (uuid "j2")
        (property "Reference" "J2")
        (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd"))
        (pad "1" smd rect (at 0 0) (size 1.5 1.5) (layers "F.Cu") (net 2 "B")))
    (footprint "Lib:R" (layer "F.Cu") (at 20 3)
        (uuid "r1")
        (property "Reference" "R1")
        (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "F.CrtYd"))
        (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 1 "A"))
        (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 2 "B")))
    (segment (start 3 10) (end 19.2 3) (width 0.25) (layer "F.Cu") (net 1)))"#;

/// The board lowered and placed by one plain seed.
fn placed() -> (Expr, LoweredPlacement, core::Placement) {
    let pcb = parse(BOARD).unwrap();
    let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default()).unwrap();
    let mut config = core::Config::new();
    config.work_limit = Some((0.2 * core::work_per_second()) as u64);
    let placement = core::place(&lowered.problem, &config);
    (pcb, lowered, placement)
}

fn footprint_at(pcb: &Expr, reference: &str) -> [f64; 3] {
    let footprint = pcb
        .children()
        .iter()
        .find(|item| footprint_reference(item).as_deref() == Some(reference))
        .unwrap();
    form_at(footprint).unwrap()
}

#[test]
fn poses_are_taken_by_uuid_else_by_reference() {
    let mut pcb = parse(BOARD).unwrap();
    // R1 by its uuid under another reference; J2 by its reference under
    // another uuid; J1 is missing and stays.
    let from = parse(
        r#"(kicad_pcb
        (footprint "Lib:R" (layer "F.Cu") (at 11 12 90) (uuid "r1") (property "Reference" "R9"))
        (footprint "Lib:J" (layer "F.Cu") (at 30 15) (uuid "other") (property "Reference" "J2")))"#,
    )
    .unwrap();
    assert_eq!(take_poses(&mut pcb, &from).unwrap(), 2);
    assert_eq!(footprint_at(&pcb, "R1"), [11.0, 12.0, 90.0]);
    assert_eq!(footprint_at(&pcb, "J2")[..2], [30.0, 15.0]);
    assert_eq!(footprint_at(&pcb, "J1")[..2], [3.0, 10.0]);
}

#[test]
fn seeds_rank_by_illegal_parts_then_touching_courtyards_then_wirelength() {
    let (_, _, placement) = placed();
    let with = |illegal: usize, courtyards: bool, wirelength: f64| {
        let mut placement = placement.clone();
        placement.illegal = (0..illegal).collect();
        placement.relaxation.courtyard_spacing = courtyards;
        placement.wirelength_final = wirelength;
        placement
    };
    let mut placements = vec![
        (1, with(1, false, 10.0)),
        (2, with(0, true, 10.0)),
        (3, with(0, false, 30.0)),
        (4, with(0, false, 20.0)),
        // A tie goes to the earlier seed.
        (5, with(0, false, 20.0)),
    ];
    assert!(rank(&placements[1].1) > rank(&placements[2].1));
    sort_by_rank(&mut placements);
    let seeds: Vec<u64> = placements.iter().map(|(seed, _)| *seed).collect();
    assert_eq!(seeds, [4, 5, 3, 2, 1]);
}

#[test]
fn seeds_rank_materially_closer_pad_objectives_before_wirelength() {
    let (_, _, placement) = placed();
    let with = |distance: f64, wirelength: f64| {
        let mut placement = placement.clone();
        placement.constraints.push(core::constraints::Status {
            kind: "pad_near_objective",
            part: 2,
            other: Some(0),
            satisfied: true,
            violation: 0.0,
            distance: Some(distance),
            pad_near: None,
        });
        placement.wirelength_final = wirelength;
        placement
    };
    let mut placements = [
        (1, with(5.0, 30.0)),
        (2, with(5.0002, 25.0)),
        (3, with(5.05, 20.0)),
    ];
    sort_by_rank(&mut placements);
    assert_eq!(placements.map(|(seed, _)| seed), [2, 1, 3]);
}

#[test]
fn seeds_keep_a_finite_pad_limit_before_minimizing_wirelength() {
    let (_, mut lowered, placement) = placed();
    let problem = &mut lowered.problem;
    problem
        .constraints
        .relations
        .push(core::constraints::Relation::PadNear {
            source: core::constraints::Anchor::Point(2, [0.0, 0.0]),
            anchor: core::constraints::Anchor::Point(0, [0.0, 0.0]),
            max: Some(6.0),
            pad_numbers: None,
        });
    let with = |distance, wirelength| {
        let mut placement = placement.clone();
        placement.poses[2].position = [3.0 + distance, 10.0];
        placement.constraints = core::constraints::report(problem, &placement.poses);
        let last = placement.constraints.last().unwrap();
        assert!(last.satisfied && last.violation == 0.0);
        placement.wirelength_final = wirelength;
        placement
    };
    let mut placements = [
        (1, with(6.005814, 10.0)),
        (2, with(6.0, 20.0)),
        (3, with(6.0000012, 30.0)),
        (4, with(6.0000008, 15.0)),
    ];
    sort_by_rank(&mut placements);
    assert_eq!(placements.map(|(seed, _)| seed), [4, 2, 3, 1]);
}

#[test]
fn written_poses_put_every_pad_where_the_placer_believed_it_is() {
    let (mut pcb, lowered, placement) = placed();
    let written = write_poses(&mut pcb, &lowered, &placement).unwrap();
    assert_eq!(
        written.uuids,
        [Some("j1".into()), Some("j2".into()), Some("r1".into())]
    );
    assert_eq!(written.footprints.len(), 3);
    assert_eq!(written.footprints[2].reference, "R1");
    assert_eq!(written.footprints[2].source_at[..2], [20.0, 3.0]);
    // The tracks are gone.
    assert!(
        pcb.children()
            .iter()
            .all(|item| item.head() != Some("segment"))
    );
    let r1 = placement.poses[2];
    assert_eq!(footprint_at(&pcb, "R1")[..2], r1.position);
    check_written_pads(&pcb, &lowered, &placement.poses).unwrap();
    // Poses the board does not hold fail the check.
    let mut moved = placement.poses.clone();
    moved[2].position[0] += 0.5;
    let error = check_written_pads(&pcb, &lowered, &moved).unwrap_err();
    assert!(
        error.starts_with("internal error: pad of R1 written at"),
        "{error}"
    );
}

#[test]
fn a_repeated_reference_is_one_more_than_one_footprint_carries() {
    let references: Vec<String> = ["R1", "R2", "R1", "C1", "C1"]
        .iter()
        .map(|reference| reference.to_string())
        .collect();
    // The fifth part is no footprint (a board's mark, say).
    let repeated = repeated_references(&references, 4);
    assert_eq!(repeated.into_iter().collect::<Vec<_>>(), ["R1"]);
    assert_eq!(repeated_references(&references, 9).len(), 2);
}

#[test]
fn hints_name_the_constraint_that_holds_a_part_without_a_place() {
    let (_, mut lowered, mut placement) = placed();
    let room = crate::placement_room::room(&lowered.problem, &lowered.references, true);
    let references = lowered.references.clone();
    let uuids = vec![None, None, Some("r1".to_string())];
    // A legal placement has no hints.
    assert!(placement_hints(&placement, &room, &[]).is_empty());
    // An illegal part without an area shortage reports observed conflicts.
    placement.illegal = vec![2];
    let parts = unplaced_parts(
        &lowered.problem,
        &references,
        &uuids,
        &BTreeSet::new(),
        &placement,
        &BTreeSet::new(),
    );
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].status, "illegal");
    assert!(parts[0].reason.is_some());
    let hints = placement_hints(&placement, &room, &parts);
    assert_eq!(hints.len(), 1);
    assert!(hints[0].contains("local_diagnosis"), "{hints:?}");
    // Held at an edge, with its uuid where its reference repeats.
    lowered
        .problem
        .constraints
        .edges
        .push((2, core::constraints::Edge::Top, 1.0));
    let repeated = BTreeSet::from(["R1".to_string()]);
    let parts = unplaced_parts(
        &lowered.problem,
        &references,
        &uuids,
        &repeated,
        &placement,
        &BTreeSet::new(),
    );
    assert_eq!(parts[0].label, "R1 (uuid r1)");
    placement.overfull = true;
    let hints = placement_hints(&placement, &room, &parts);
    assert_eq!(hints.len(), 2, "{hints:?}");
    assert!(
        hints.contains(&"R1 (uuid r1) is held by edge top within 1.00 mm: loosen it (a larger max_mm or region)".to_string()),
        "{hints:?}"
    );
    assert!(
        hints
            .iter()
            .any(|hint| hint.starts_with("even the tightest bodies"))
    );
}

#[test]
fn automatic_module_holds_are_grouped_and_keep_board_piece_provenance() {
    let (_, mut lowered, mut placement) = placed();
    let problem = &mut lowered.problem;
    let uuids = vec![None; 3];
    for index in 0..3 {
        problem.components[index].fixed = false;
        problem
            .constraints
            .regions
            .push((index, [1.0, 1.0, 19.0, 19.0]));
        problem.constraints.automatic_regions.insert(index);
    }
    placement.unplaced = vec![0, 1, 2];
    let parts = unplaced_parts(
        problem,
        &lowered.references,
        &uuids,
        &BTreeSet::new(),
        &placement,
        &BTreeSet::from([0, 1]),
    );
    assert_eq!(
        parts
            .iter()
            .map(|row| row.local_diagnosis.region.as_ref().unwrap().origin)
            .collect::<Vec<_>>(),
        [
            KiCadPlacementRegionOrigin::BreakawayModule,
            KiCadPlacementRegionOrigin::BreakawayModule,
            KiCadPlacementRegionOrigin::BoardPiece,
        ]
    );
    let room = crate::placement_room::room(problem, &lowered.references, true);
    let hints = placement_hints(&placement, &room, &parts);
    assert_eq!(
        hints
            .iter()
            .filter(|hint| hint.contains("placer.keep_in_modules: false"))
            .count(),
        1
    );
    assert!(
        hints
            .iter()
            .any(|hint| hint.contains("Automatic breakaway module regions hold 2 parts"))
    );
    assert!(
        hints
            .iter()
            .any(|hint| hint.contains("Automatic board-piece regions hold 1 part"))
    );
    assert!(!hints.iter().any(|hint| hint.contains("loosen it")));
}

#[test]
fn an_unplaced_part_reports_the_footprint_that_blocks_its_whole_region() {
    let (_, mut lowered, mut placement) = placed();
    let uuids = vec![Some("j1".into()), Some("j2".into()), Some("r1".into())];
    let problem = &mut lowered.problem;
    problem.constraints.regions.push((2, [1.4, 8.4, 4.6, 11.6]));
    placement.unplaced = vec![2];
    placement.illegal = vec![0, 2];
    placement.poses[2] = core::Pose {
        position: [3.0, 10.0],
        angle: 0.0,
    };
    let parts = unplaced_parts(
        problem,
        &lowered.references,
        &uuids,
        &BTreeSet::new(),
        &placement,
        &BTreeSet::new(),
    );
    let [part] = &parts[..] else {
        panic!("{parts:?}")
    };
    assert_eq!(part.status, "unplaced");
    let diagnosis = &part.local_diagnosis;
    assert_eq!(diagnosis.side, "front");
    let [blocker] = &diagnosis.conflicts[..] else {
        panic!("{diagnosis:?}")
    };
    assert_eq!(blocker.other, "J1");
    assert_eq!(blocker.uuid.as_deref(), Some("j1"));
    assert!(blocker.fixed);
    let region = diagnosis.region.as_ref().unwrap();
    assert_eq!(region.bounds_mm, [1.4, 8.4, 4.6, 11.6]);
    assert_eq!(region.blocked_by.as_deref(), Some("J1"));
    assert!(
        region
            .by_rotation
            .iter()
            .all(|row| row.center_bounds_mm.is_some())
    );
    assert!(
        diagnosis
            .next_action
            .as_ref()
            .unwrap()
            .contains("R1's region or the constraint that fixes J1")
    );
    assert!(
        placement_hints(
            &placement,
            &crate::placement_room::room(problem, &lowered.references, true),
            &parts
        )
        .iter()
        .all(|hint| !hint.contains("loosen it"))
    );

    // A region and a collision at one pose alone do not prove a blocked region.
    problem.constraints.regions[0].1 = [1.4, 8.4, 12.0, 17.0];
    let parts = unplaced_parts(
        problem,
        &lowered.references,
        &uuids,
        &BTreeSet::new(),
        &placement,
        &BTreeSet::new(),
    );
    assert!(
        parts[0]
            .local_diagnosis
            .region
            .as_ref()
            .unwrap()
            .blocked_by
            .is_none()
    );
    assert!(parts[0].local_diagnosis.next_action.is_none());
}

#[test]
fn a_region_covered_by_fixed_footprints_reports_the_joint_obstruction() {
    let (_, mut lowered, mut placement) = placed();
    let uuids = vec![Some("j1".into()), Some("j2".into()), Some("r1".into())];
    let problem = &mut lowered.problem;
    problem
        .constraints
        .regions
        .push((2, [2.0, 6.0, 12.0, 14.0]));
    problem.constraints.automatic_regions.insert(2);
    for index in 0..2 {
        problem.components[index].fixed = true;
        problem.components[index].body_size = [4.0, 8.0];
        problem.components[index].body_center = [0.0; 2];
        placement.poses[index].position = [4.0 + index as f64 * 6.0, 10.0];
        placement.poses[index].angle = 0.0;
    }
    problem.components[2].fixed = false;
    problem.components[2].body_size = [3.0, 3.0];
    problem.components[2].body_center = [0.0; 2];
    problem.components[2].angle_options = vec![0.0, 90.0];
    placement.relaxation.tight = false;
    placement.unplaced = vec![2];
    placement.poses[2].position = [4.0, 10.0];
    let report = |problem: &core::Problem, placement: &core::Placement| {
        unplaced_parts(
            problem,
            &lowered.references,
            &uuids,
            &BTreeSet::new(),
            placement,
            &BTreeSet::from([2]),
        )
    };
    let parts = report(problem, &placement);
    let diagnosis = &parts[0].local_diagnosis;
    assert_eq!(diagnosis.conflicts.len(), 1, "{diagnosis:?}");
    let region = diagnosis.region.as_ref().unwrap();
    assert_eq!(region.origin, KiCadPlacementRegionOrigin::BreakawayModule);
    assert_eq!(
        region
            .fixed_region_cover
            .iter()
            .map(|blocker| (blocker.part.as_str(), blocker.uuid.as_str()))
            .collect::<Vec<_>>(),
        [("J1", "j1"), ("J2", "j2")]
    );
    assert!(
        diagnosis
            .next_action
            .as_deref()
            .unwrap()
            .contains("for R1 on front")
    );
    let room = crate::placement_room::room(problem, &lowered.references, true);
    assert!(
        placement_hints(&placement, &room, &parts)
            .iter()
            .all(|hint| !hint.contains("keep_in_modules"))
    );

    // A continuous gap has clearance from the actual fixed bodies, even
    // if the failed pose still overlaps one of them.
    placement.poses[1].position[0] += 2.0;
    let parts = report(problem, &placement);
    assert!(
        parts[0]
            .local_diagnosis
            .region
            .as_ref()
            .unwrap()
            .fixed_region_cover
            .is_empty()
    );
    placement.poses[1].position[0] -= 2.0;

    // Do not project an opposite-face hold across the board or assume an
    // axis-aligned box represents a part at arbitrary angles.
    problem.components[1].side = core::Side::Back;
    let parts = report(problem, &placement);
    assert!(
        parts[0]
            .local_diagnosis
            .region
            .as_ref()
            .unwrap()
            .fixed_region_cover
            .is_empty()
    );
    problem.components[1].side = core::Side::Front;
    problem.components[2].angle_options.push(45.0);
    let parts = report(problem, &placement);
    assert!(
        parts[0]
            .local_diagnosis
            .region
            .as_ref()
            .unwrap()
            .fixed_region_cover
            .is_empty()
    );
}

#[test]
fn a_certified_pair_points_to_side_or_region_instead_of_moving_the_blocker() {
    let (_, mut lowered, mut placement) = placed();
    let uuids = vec![Some("j1".into()), Some("j2".into()), Some("r1".into())];
    let problem = &mut lowered.problem;
    problem.components[0].fixed = false;
    problem.components[0].body_size = [18.0, 18.0];
    problem.components[0].angle_options = vec![0.0, 90.0];
    problem.components[2].body_size = [12.0, 10.0];
    problem.components[2].angle_options = vec![0.0, 90.0];
    problem
        .constraints
        .regions
        .push((2, [14.0, 4.0, 26.0, 16.0]));
    placement.relaxation.tight = false;
    placement.unplaced = vec![2];
    placement.poses[2] = core::Pose {
        position: [20.0, 10.0],
        angle: 0.0,
    };
    let parts = unplaced_parts(
        problem,
        &lowered.references,
        &uuids,
        &BTreeSet::new(),
        &placement,
        &BTreeSet::new(),
    );
    let diagnosis = &parts[0].local_diagnosis;
    let packing = diagnosis
        .region
        .as_ref()
        .unwrap()
        .packing_conflict
        .as_ref()
        .unwrap();
    assert_eq!(packing.other, "J1");
    assert_eq!(packing.other_minimum_mm, [18.0, 18.0]);
    assert!(
        packing.available_mm.iter().all(|span| *span < 18.0),
        "{packing:?}"
    );
    let action = diagnosis.next_action.as_ref().unwrap();
    assert!(
        action.contains("Change an assigned side if mounting and access permit it"),
        "{action}"
    );
    assert!(
        action.contains("revise R1's region if the enclosure allows"),
        "{action}"
    );

    // A round full body may become a rectangular tight body. The pair
    // proof must still see the full shape and decline to certify it.
    problem.components[0].round = true;
    problem.components[0].tight = Some([-9.0, -8.9, 9.0, 8.9]);
    placement.relaxation.tight = true;
    let parts = unplaced_parts(
        problem,
        &lowered.references,
        &uuids,
        &BTreeSet::new(),
        &placement,
        &BTreeSet::new(),
    );
    assert!(
        parts[0]
            .local_diagnosis
            .region
            .as_ref()
            .unwrap()
            .packing_conflict
            .is_none()
    );
}

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
    // An illegal part without a constraint: give the board more room.
    placement.illegal = vec![2];
    let parts = unplaced_parts(
        &lowered.problem,
        &references,
        &uuids,
        &BTreeSet::new(),
        &placement,
    );
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].status, "illegal");
    assert!(parts[0].reason.is_some());
    let hints = placement_hints(&placement, &room, &parts);
    assert_eq!(hints.len(), 1);
    assert!(hints[0].starts_with("no free spot fits"), "{hints:?}");
    // Held at an edge, with its uuid where its reference repeats.
    lowered
        .problem
        .constraints
        .edges
        .push((2, core::constraints::Edge::Top, 1.0));
    let repeated = BTreeSet::from(["R1".to_string()]);
    let parts = unplaced_parts(&lowered.problem, &references, &uuids, &repeated, &placement);
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

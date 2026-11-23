// Copyright (C) 2026 Florian Loitsch.

use super::*;

pub(super) type OutlineSourcePoses = BTreeMap<usize, [f64; 3]>;

/// A newly imported connector may supply an open span of the perimeter
/// or a closed cutout assembled from lines and arcs. Establish that
/// geometry at explicit absolute holds before lowering the outline.
pub(super) fn place_outline_anchors(
    pcb: &mut Expr,
    constraints: &crate::placement_constraints::KiCadPlacementConstraints,
) -> Result<OutlineSourcePoses, String> {
    let mut previous = BTreeMap::new();
    if constraints.place.is_empty()
        || !pcb.children().iter().any(|item| {
            item.head().is_some_and(|head| head.starts_with("gr_"))
                && form_atom(item, "layer", 1) == Some("Edge.Cuts")
        })
    {
        return Ok(previous);
    }

    let mut anchors = BTreeMap::<usize, Vec<_>>::new();
    let footprints: Vec<_> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .map(|item| {
            (
                item,
                footprint_reference(item).unwrap_or_default(),
                kicad_nets::footprint_uuid(item),
            )
        })
        .collect();
    for entry in &constraints.place {
        let (None, Some(x), Some(y), Some("absolute")) =
            (&entry.at, entry.x, entry.y, entry.origin.as_deref())
        else {
            continue;
        };
        let mut matches = footprints
            .iter()
            .enumerate()
            .filter(|(_, (_, reference, own_uuid))| {
                reference == &entry.part
                    && entry.uuid.as_deref().is_none_or(|uuid| {
                        own_uuid
                            .as_deref()
                            .is_some_and(|own| own.eq_ignore_ascii_case(uuid))
                    })
            });
        let Some((index, (footprint, _, _))) = matches.next() else {
            continue;
        };
        if matches.next().is_some()
            || !footprint.children().iter().any(|item| {
                matches!(item.head(), Some("fp_line" | "fp_arc" | "fp_curve"))
                    && form_atom(item, "layer", 1) == Some("Edge.Cuts")
            })
        {
            continue;
        }
        anchors
            .entry(index)
            .or_default()
            .push(([x, y], entry.angle));
    }
    if anchors.is_empty() {
        return Ok(BTreeMap::new());
    }
    let needs_perimeter = outline::board_loops(pcb).is_err();
    let mut closed_loops = 0;
    if !needs_perimeter {
        anchors.retain(|index, _| {
            let count = closed_edge_loops(footprints[*index].0);
            closed_loops += count;
            count != 0
        });
        if anchors.is_empty() {
            return Ok(previous);
        }
    }

    let mut staged = pcb.clone();
    apply_sides(&mut staged, constraints)?;
    let Expr::List(items) = &mut staged else {
        return Err("PCB root is not a list".into());
    };
    for (index, footprint) in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
        .enumerate()
    {
        let Some(entries) = anchors.get(&index) else {
            continue;
        };
        let at = form_at(footprint)?;
        previous.insert(index, at);
        let mut angle = at[2];
        let mut position = [at[0], at[1]];
        for &(next_position, next_angle) in entries {
            position = next_position;
            angle = next_angle.unwrap_or(angle).rem_euclid(360.0);
        }
        write_footprint_pose(footprint, core::Pose { position, angle })?;
    }
    let loops = match outline::board_loops_strict(&staged) {
        Ok(loops) => loops,
        Err(error) if needs_perimeter => {
            return Err(format!(
                "the absolute connector poses leave an invalid Edge.Cuts outline: {error}"
            ));
        }
        Err(_) => return Ok(BTreeMap::new()),
    };
    let mut perimeter = staged.clone();
    if let Expr::List(items) = &mut perimeter {
        let mut index = 0;
        items.retain(|item| {
            if item.head() != Some("footprint") {
                return true;
            }
            let anchored = anchors.contains_key(&index);
            index += 1;
            // For perimeter recovery, only held anchors may complete the
            // outside perimeter. For a cutout, the outside perimeter must
            // still exist with the anchored footprint removed.
            anchored == needs_perimeter
        });
    }
    if !outline::board_loops_strict(&perimeter).is_ok_and(|fixed| {
        fixed.piece_loops == loops.piece_loops
            && (needs_perimeter || fixed.cutouts.len() + closed_loops == loops.cutouts.len())
    }) {
        if needs_perimeter {
            return Err("the Edge.Cuts perimeter needs an absolute place constraint for each footprint that forms it".into());
        }
        return Ok(BTreeMap::new());
    }
    *pcb = staged;
    Ok(previous)
}

fn closed_edge_loops(footprint: &Expr) -> usize {
    let mut local = vec![Expr::Atom("footprint".into())];
    local.extend(
        footprint
            .children()
            .iter()
            .filter(|item| form_atom(item, "layer", 1) == Some("Edge.Cuts"))
            .cloned(),
    );
    let board = Expr::List(vec![Expr::Atom("kicad_pcb".into()), Expr::List(local)]);
    outline::board_loops_strict(&board)
        .map_or(0, |loops| loops.piece_loops.len() + loops.cutouts.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incomplete_outline() -> Expr {
        parse(
            r#"(kicad_pcb
            (gr_line (start 0 0) (end 3 0) (layer "Edge.Cuts"))
            (gr_line (start 7 0) (end 10 0) (layer "Edge.Cuts"))
            (gr_line (start 10 0) (end 10 6) (layer "Edge.Cuts"))
            (gr_line (start 10 6) (end 0 6) (layer "Edge.Cuts"))
            (gr_line (start 0 6) (end 0 0) (layer "Edge.Cuts"))
            (footprint "Test:Socket" (layer "F.Cu") (at 50 50)
              (property "Reference" "J1")
              (fp_line (start 0 -2) (end 0 2) (layer "Edge.Cuts"))))"#,
        )
        .unwrap()
    }

    #[test]
    fn an_absolute_pose_closes_the_footprints_perimeter_span() {
        let mut board = incomplete_outline();
        let brief = serde_json::from_str(
            r#"{
            "version": 1,
            "place": [{"part": "J1", "x": 5, "y": 0, "angle": 90, "origin": "absolute"}]
        }"#,
        )
        .unwrap();
        assert!(outline::board_loops_strict(&board).is_err());
        let prior = place_outline_anchors(&mut board, &brief).unwrap();
        assert_eq!(prior[&0], [50.0, 50.0, 0.0]);
        assert_eq!(
            outline::board_loops_strict(&board).unwrap().pieces,
            vec![[0.0, 0.0, 10.0, 6.0]]
        );
        let footprint = board.child("footprint").unwrap();
        assert_eq!(form_at(footprint).unwrap(), [5.0, 0.0, 90.0]);
        assert_eq!(
            form_xy(footprint.child("fp_line").unwrap(), "start").unwrap(),
            [0.0, -2.0]
        );
    }

    #[test]
    fn relative_or_misaligned_connector_poses_do_not_repair_the_outline() {
        for (origin, x) in [("board", 5.0), ("absolute", 5.1)] {
            let mut board = incomplete_outline();
            let brief = serde_json::from_value(serde_json::json!({
                "version": 1, "place": [{"part": "J1", "x": x, "y": 0, "angle": 90, "origin": origin}]
            })).unwrap();
            if origin == "board" {
                assert!(
                    place_outline_anchors(&mut board, &brief)
                        .unwrap()
                        .is_empty()
                );
            } else {
                assert!(place_outline_anchors(&mut board, &brief).is_err());
            }
            assert_eq!(
                form_at(board.child("footprint").unwrap()).unwrap(),
                [50.0, 50.0, 0.0]
            );
        }
    }

    fn board_with_closed_connector_cutout() -> Expr {
        parse(
            r#"(kicad_pcb
            (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
            (footprint "Test:Socket" (layer "F.Cu") (at 50 50)
              (property "Reference" "J1")
              (fp_line (start -1 -1) (end 1 -1) (layer "Edge.Cuts"))
              (fp_arc (start 1 -1) (mid 2 0) (end 1 1) (layer "Edge.Cuts"))
              (fp_line (start 1 1) (end -1 1) (layer "Edge.Cuts"))
              (fp_line (start -1 1) (end -1 -1) (layer "Edge.Cuts"))))"#,
        )
        .unwrap()
    }

    #[test]
    fn an_absolute_back_pose_seats_a_closed_cutout_before_lowering() {
        let mut board = board_with_closed_connector_cutout();
        let before = outline::board_loops_strict(&board).unwrap();
        assert_eq!(before.pieces.len(), 1);
        assert!(before.cutouts.is_empty());
        let brief = serde_json::from_str(
            r#"{
            "version": 1, "back": ["J1"],
            "place": [{"part": "J1", "x": 8, "y": 5, "angle": 90, "origin": "absolute"}]
        }"#,
        )
        .unwrap();
        let prior = place_outline_anchors(&mut board, &brief).unwrap();
        assert_eq!(prior[&0], [50.0, 50.0, 0.0]);
        let loops = outline::board_loops_strict(&board).unwrap();
        assert_eq!(loops.pieces, vec![[0.0, 0.0, 20.0, 10.0]]);
        assert_eq!(loops.cutouts.len(), 1);
        let footprint = board.child("footprint").unwrap();
        assert_eq!(form_atom(footprint, "layer", 1), Some("B.Cu"));
        assert_eq!(form_at(footprint).unwrap(), [8.0, 5.0, 90.0]);
        assert_eq!(
            form_xy(footprint.child("fp_line").unwrap(), "start").unwrap(),
            [-1.0, 1.0]
        );
    }

    #[test]
    fn an_absolute_pose_outside_the_board_does_not_register_a_new_cutout() {
        let mut board = board_with_closed_connector_cutout();
        let original = board.clone();
        let brief = serde_json::from_str(
            r#"{
            "version": 1,
            "place": [{"part": "J1", "x": 25, "y": 5, "angle": 90, "origin": "absolute"}]
        }"#,
        )
        .unwrap();
        assert!(
            place_outline_anchors(&mut board, &brief)
                .unwrap()
                .is_empty()
        );
        assert_eq!(board, original);
    }
}

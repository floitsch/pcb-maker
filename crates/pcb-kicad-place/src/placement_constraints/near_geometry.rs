// Copyright (C) 2026 Florian Loitsch.

//! Explain missed numeric near requests without changing placement scoring.

use super::*;
use crate::board_placer::LoweredPlacement;

pub fn near_geometry_spacing(relaxation: &core::Relaxation) -> Option<f64> {
    (!relaxation.tight
        && !relaxation.courtyard_spacing
        && !relaxation.pad_clearance
        && !relaxation.edge_copper)
        .then_some(relaxation.spacing)
}

/// Retain the authored pin label only when the compiled relation points
/// to a uniquely identifiable pad centre. The solver's legacy first-pad
/// choice for body-to-pin relations remains unchanged.
pub(crate) fn body_near_pin_labels(
    pcb: &Expr,
    authored: Option<&KiCadPlacementConstraints>,
    problem: &core::Problem,
    references: &[String],
    first_near: usize,
) -> BTreeMap<usize, String> {
    let mut labels = BTreeMap::new();
    let Some(authored) = authored else {
        return labels;
    };
    for (index, entry) in authored.near.iter().enumerate() {
        let index = first_near + index;
        let Some(Relation::Near {
            part,
            anchor: Anchor::Point(target, _),
            ..
        }) = problem.constraints.relations.get(index)
        else {
            continue;
        };
        let Some((pin_of, (reference, number))) = entry
            .pin_of
            .as_ref()
            .and_then(|pin| Some((pin, pin.split_once(':')?)))
        else {
            continue;
        };
        if references.get(*part) != Some(&entry.part)
            || references.get(*target).is_none_or(|name| name != reference)
        {
            continue;
        }
        let mut footprints = pcb.children().iter().filter(|item| {
            item.head() == Some("footprint")
                && footprint_reference(item).as_deref() == Some(reference)
        });
        let Some(footprint) = footprints.next() else {
            continue;
        };
        if footprints.next().is_some() {
            continue;
        }
        let mut positions = footprint
            .children()
            .iter()
            .filter(|item| {
                item.head() == Some("pad")
                    && item.children().get(1).and_then(Expr::atom) == Some(number)
            })
            .map(form_at);
        let Some(Ok([x, y, _])) = positions.next() else {
            continue;
        };
        if positions.all(|value| value.is_ok_and(|at| [at[0], at[1]] == [x, y])) {
            labels.insert(index, pin_of.clone());
        }
    }
    labels
}

pub fn constraint_report_at(
    lowered: &LoweredPlacement,
    poses: &[core::Pose],
    diagnostic_spacing: Option<f64>,
) -> Vec<KiCadConstraintStatus> {
    let problem = &lowered.problem;
    let constraints = &problem.constraints;
    let mut rows = constraint_report(problem, poses, &lowered.references);
    if diagnostic_spacing.is_some() || !lowered.authored_near.is_empty() {
        let start =
            constraints.edges.len() + constraints.overhangs.len() + constraints.regions.len();
        let authored = constraints
            .relations
            .len()
            .saturating_sub(constraints.automatic);
        for (index, (row, relation)) in rows[start..]
            .iter_mut()
            .zip(&constraints.relations[..authored])
            .enumerate()
        {
            if lowered.authored_near.contains(&index)
                && let Relation::Near {
                    part,
                    anchor: anchor @ Anchor::Point(_, _),
                    max,
                } = relation
            {
                let distance =
                    core::constraints::body_near_distance(problem, poses, *part, *anchor);
                row.satisfied = core::constraints::near_limit_kept(distance, *max);
                row.distance_mm = Some(distance);
                row.violation_mm = if row.satisfied { 0.0 } else { distance - max };
                if !row.satisfied
                    && let Some(pin_of) = lowered.body_near_pin_of.get(&index)
                {
                    row.body_near_pin = Some(KiCadBodyNearPinStatus {
                        pin_of: pin_of.clone(),
                        distance_mm: distance,
                        max_mm: *max,
                    });
                }
            }
            if row.kind != "near" || row.requirement_satisfied() {
                continue;
            }
            let Some(spacing) = diagnostic_spacing else {
                continue;
            };
            let (pin_of, pad) = match &row.pad_near {
                Some(pair) => (&pair.pin_of, Some(pair.pad.clone())),
                None => match lowered.body_near_pin_of.get(&index) {
                    Some(pin) => (pin, None),
                    None => continue,
                },
            };
            let Some(bound) =
                core::constraints::near_geometry_at(problem, poses, relation, spacing)
            else {
                continue;
            };
            let [_, Some(target)] = relation.parts() else {
                continue;
            };
            let pose = poses[target];
            row.near_geometry = Some(KiCadNearGeometry {
                pin_of: pin_of.clone(),
                pad,
                measured_mm: bound.distance,
                max_mm: bound.limit,
                minimum_at_target_pose_mm: bound.minimum,
                body_spacing_mm: spacing,
                target_at: [pose.position[0], pose.position[1], pose.angle],
                faces_without_room: bound
                    .faces_without_room
                    .iter()
                    .map(|name| (*name).to_string())
                    .collect(),
            });
        }
    }
    if let Some(confirmed) = &lowered.confirmed_sub_boards {
        rows.extend(confirmed.report(problem, poses, &lowered.references));
    }
    rows
}

#[derive(Clone, Debug, Serialize)]
pub struct BodyNearGridStep {
    #[serde(skip)]
    pub part_index: usize,
    pub part: String,
    pub pin_of: String,
    pub before_at: [f64; 3],
    pub at: [f64; 3],
    pub before_mm: f64,
    pub distance_mm: f64,
    pub max_mm: f64,
    pub grid_mm: f64,
    pub used_courtyard_spacing: bool,
}

/// Adjacent legal translations on the selected placement grid. The caller
/// still has to check native copper contact and DRC.
pub fn body_near_grid_steps(
    lowered: &LoweredPlacement,
    allow_courtyard_spacing: bool,
) -> Vec<BodyNearGridStep> {
    use pcb_placer::constraints::{body_near_distance, near_limit_kept, snap_position};

    let problem = &lowered.problem;
    let poses = &problem.poses;
    let grid = problem.grid;
    if !(0.0 < grid && grid <= 0.25) {
        return Vec::new();
    }
    let Some((index, pin)) = lowered.body_near_pin_of.iter().find(|(index, _)| {
        matches!(problem.constraints.relations[**index], Relation::Near { part, anchor, max }
            if !near_limit_kept(body_near_distance(problem, poses, part, anchor), max))
    }) else {
        return Vec::new();
    };
    let Relation::Near { part, anchor, max } = problem.constraints.relations[*index] else {
        return Vec::new();
    };
    let body = &problem.components[part];
    let before = poses[part];
    let before_mm = body_near_distance(problem, poses, part, anchor);
    if body.fixed
        || body.has_holes()
        || body.body_size.into_iter().any(|size| size > 6.0)
        || before_mm - max > grid
        || lowered
            .attached_marks
            .iter()
            .any(|&(_, owner)| owner == part)
        || problem
            .constraints
            .followers
            .iter()
            .any(|member| [member.part, member.leader].contains(&part))
        || lowered
            .references
            .iter()
            .filter(|name| *name == &lowered.references[part])
            .count()
            != 1
    {
        return Vec::new();
    }
    let [x, y] = before.position;
    let angle = before.angle;
    let mut trial = poses.clone();
    let mut steps = Vec::new();
    let mut courtyard_problem = None;
    for [dx, dy] in [[-grid, 0.0], [grid, 0.0], [0.0, -grid], [0.0, grid]] {
        let [next_x, next_y] = snap_position(problem, part, [x + dx, y + dy], angle);
        let displacement = (next_x - x).hypot(next_y - y);
        if displacement <= 1.0e-6 || displacement > grid + 1.0e-6 {
            continue;
        }
        let pose = core::Pose {
            position: [next_x, next_y],
            angle,
        };
        trial[part] = pose;
        let distance_mm = body_near_distance(problem, &trial, part, anchor);
        if !near_limit_kept(distance_mm, max) {
            continue;
        }
        let ordinary_legal = core::legal::is_legal(problem, poses, part, pose, 0..poses.len());
        if !ordinary_legal {
            if !allow_courtyard_spacing || problem.constraints.courtyard_spacing {
                continue;
            }
            let courtyard = courtyard_problem.get_or_insert_with(|| {
                let mut alternate = problem.clone();
                alternate.constraints.courtyard_spacing = true;
                alternate
            });
            if !core::legal::is_legal(courtyard, poses, part, pose, 0..poses.len()) {
                continue;
            }
        }
        steps.push(BodyNearGridStep {
            part_index: part,
            part: lowered.references[part].clone(),
            pin_of: pin.clone(),
            before_at: [x, y, angle],
            at: [next_x, next_y, angle],
            before_mm,
            distance_mm,
            max_mm: max,
            grid_mm: grid,
            used_courtyard_spacing: !ordinary_legal,
        });
    }
    steps.sort_by(|a, b| {
        a.used_courtyard_spacing
            .cmp(&b.used_courtyard_spacing)
            .then_with(|| a.distance_mm.total_cmp(&b.distance_mm))
    });
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board_placer::{KiCadBoardPlacerConfig, lower_placement};

    const BOARD: &str = r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
        (gr_rect (start 0 0) (end 30 20) (layer "Edge.Cuts"))
        (footprint "Target" (layer "F.Cu") (at 6.8 2.9)
          (property "Reference" "U1")
          (fp_rect (start -6.8 -8.5) (end 6.8 8.5) (layer "F.CrtYd"))
          (pad "3" smd rect (at -5.6 0.3) (size 0.3 0.5) (layers "F.Cu")))
        (footprint "Source" (layer "F.Cu") (at 1.2 12.6)
          (property "Reference" "C1")
          (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
          (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu"))))"#;

    fn fixture(board: &str, entries: serde_json::Value, back: bool) -> LoweredPlacement {
        let constraints = serde_json::from_value(serde_json::json!({
            "version": 1,
            "move_all": true,
            "fixed": ["U1"],
            "back": if back { vec!["C1"] } else { Vec::new() },
            "near": entries
        }))
        .unwrap();
        let mut board = parse(board).unwrap();
        apply_sides(&mut board, &constraints).unwrap();
        lower_placement(
            &board,
            &KiCadBoardPlacerConfig {
                constraints: Some(KiCadConstraintsSource::Inline(constraints)),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn request(body: f64, pad: f64) -> serde_json::Value {
        serde_json::json!([
            {"part":"C1","pin_of":"U1:3","max_mm":body},
            {"part":"C1","pad":"1","pin_of":"U1:3","max_mm":pad}
        ])
    }

    #[test]
    fn a_buried_pin_at_two_board_edges_has_a_conditional_minimum() {
        let placement = fixture(BOARD, request(6.0, 7.0), false);
        let report = constraint_report_at(&placement, &placement.problem.poses, Some(0.2));
        let bounds: Vec<_> = report
            .iter()
            .filter_map(|row| row.near_geometry.as_ref())
            .collect();
        assert_eq!(bounds.len(), 2);
        for (bound, expected) in bounds.iter().zip([8.4, 9.4]) {
            assert_eq!(&bound.pin_of, "U1:3");
            assert_eq!(bound.faces_without_room, ["left", "top"]);
            assert!((bound.minimum_at_target_pose_mm - expected).abs() <= 0.000_002);
            assert!((bound.measured_mm - expected).abs() <= 0.000_002);
        }
        assert_eq!(bounds[0].pad, None);
        assert_eq!(bounds[1].pad.as_deref(), Some("1"));
        let without_bound = constraint_report_at(&placement, &placement.problem.poses, None);
        let body = without_bound
            .iter()
            .find_map(|row| row.body_near_pin.as_ref())
            .unwrap();
        assert_eq!(body.pin_of, "U1:3");
        assert!((body.distance_mm - 8.4).abs() < 2.0e-6);
        assert_eq!(body.max_mm, 6.0);
    }

    #[test]
    fn a_feasible_boundary_and_an_opposite_side_have_no_bound_diagnostic() {
        let equal = fixture(BOARD, request(8.4, 9.4), false);
        let boundaries = constraint_report_at(&equal, &equal.problem.poses, Some(0.2));
        assert!(
            boundaries
                .iter()
                .filter(|row| row.kind == "near")
                .all(|row| row.requirement_satisfied())
        );
        assert!(boundaries.iter().all(|row| row.near_geometry.is_none()));

        let opposite = fixture(BOARD, request(6.0, 7.0), true);
        let reports = constraint_report_at(&opposite, &opposite.problem.poses, Some(0.2));
        assert!(reports.iter().any(|row| !row.requirement_satisfied()));
        assert!(reports.iter().all(|row| row.near_geometry.is_none()));
        assert!(reports.iter().any(|row| row.body_near_pin.is_some()));
    }

    #[test]
    fn authored_body_near_reports_the_literal_limit_with_nanometre_allowance() {
        for (excess, expected) in [
            (0.000_000_5, true),
            (0.000_001_5, false),
            (0.007_501, false),
        ] {
            let limit = 1.5;
            let y = 3.2 + 1.0 + limit + excess;
            let board = BOARD.replace("(at 1.2 12.6)", &format!("(at 1.2 {y})"));
            let entries = serde_json::json!([{"part":"C1","pin_of":"U1:3","max_mm":limit}]);
            let placement = fixture(&board, entries, false);
            let poses = &placement.problem.poses;
            let near = &placement.problem.constraints.relations[placement.authored_near.start];
            assert_eq!(
                core::constraints::relation_violation(&placement.problem, poses, near),
                0.0
            );
            let report = constraint_report_at(&placement, poses, None);
            let row = report.iter().find(|row| row.kind == "near").unwrap();
            assert_eq!(row.requirement_satisfied(), expected);
            assert!((row.distance_mm.unwrap() - limit - excess).abs() < 1.0e-9);
            if expected {
                assert_eq!(row.violation_mm, 0.0);
            } else {
                assert!((row.violation_mm - excess).abs() < 1.0e-9);
                assert_eq!(row.body_near_pin.as_ref().unwrap().pin_of, "U1:3");
            }
        }
    }

    #[test]
    fn a_micro_retry_stays_on_the_selected_pitch_and_preserves_angle() {
        let board = r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
            (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
            (footprint "Target" (layer "F.Cu") (at 5 5)
              (property "Reference" "U1")
              (fp_rect (start -0.5 -0.5) (end 0.5 0.5) (layer "F.CrtYd"))
              (pad "3" smd rect (at 0 0) (size 0.3 0.5) (layers "F.Cu")))
            (footprint "Source" (layer "F.Cu") (at 8.1 5 90)
              (property "Reference" "C1")
              (fp_rect (start -0.5 -0.5) (end 0.5 0.5) (layer "F.CrtYd"))
              (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu"))))"#;
        let entries = serde_json::json!([{"part":"C1","pin_of":"U1:3","max_mm":2.55}]);
        let mut placement = fixture(board, entries, false);
        placement.problem.grid = 0.1;
        let steps = body_near_grid_steps(&placement, false);
        assert_eq!(steps.len(), 1);
        let step = &steps[0];
        assert_eq!((&*step.part, &*step.pin_of), ("C1", "U1:3"));
        assert_eq!(step.at, [8.0, 5.0, 90.0]);
        assert!((step.distance_mm - 2.5).abs() < 1.0e-9);
        let source = step.part_index;
        placement.problem.components[source].fixed = true;
        assert!(body_near_grid_steps(&placement, false).is_empty());
        placement.problem.components[source].fixed = false;
        placement.problem.spacing = 2.1;
        assert!(body_near_grid_steps(&placement, false).is_empty());
        let courtyard_steps = body_near_grid_steps(&placement, true);
        assert_eq!(courtyard_steps.len(), 1);
        assert!(courtyard_steps[0].used_courtyard_spacing);
        assert_eq!(courtyard_steps[0].at, step.at);
        placement.attached_marks.push((0, source));
        assert!(body_near_grid_steps(&placement, true).is_empty());
        placement.attached_marks.clear();
        placement.problem.grid = 0.635;
        assert!(body_near_grid_steps(&placement, true).is_empty());
    }

    #[test]
    fn authored_body_to_body_and_group_keep_their_existing_tolerance() {
        let board = BOARD.replace("(at 1.2 12.6)", "(at 1.2 13.907501)");
        let entries = serde_json::json!([{"part":"C1","part_of":"U1","max_mm":1.5}]);
        let mut placement = fixture(&board, entries, false);
        // A group lowers to the same relation immediately after authored near.
        let near = placement.problem.constraints.relations[placement.authored_near.start].clone();
        let end = placement.authored_near.end;
        placement.problem.constraints.relations.insert(end, near);
        let report = constraint_report_at(&placement, &placement.problem.poses, None);
        let mut rows = report.iter().filter(|row| row.kind == "near");
        let body_near = rows.next().unwrap();
        assert!(body_near.requirement_satisfied());
        assert_eq!(body_near.violation_mm, 0.0);
        assert!(body_near.body_near_pin.is_none());
        assert!(rows.next().unwrap().requirement_satisfied());
    }

    #[test]
    fn a_legacy_body_request_with_ambiguous_pad_centres_keeps_its_old_report() {
        let duplicate = BOARD.replace("(pad \"3\" smd rect (at -5.6 0.3) (size 0.3 0.5) (layers \"F.Cu\"))",
            "(pad \"3\" smd rect (at -5.6 0.3) (size 0.3 0.5) (layers \"F.Cu\")) (pad \"3\" smd rect (at -4 0.3) (size 0.3 0.5) (layers \"F.Cu\"))");
        let body_only = serde_json::json!([{"part":"C1","pin_of":"U1:3","max_mm":6}]);
        let placement = fixture(&duplicate, body_only, false);
        let reports = constraint_report_at(&placement, &placement.problem.poses, Some(0.2));
        assert!(reports.iter().any(|row| !row.requirement_satisfied()));
        assert!(
            reports
                .iter()
                .all(|row| row.near_geometry.is_none() && row.body_near_pin.is_none())
        );
    }
}

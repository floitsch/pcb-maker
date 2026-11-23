// Copyright (C) 2026 Florian Loitsch.

//! Observed overlap of a footprint's antenna area and a retained board area.

use crate::board_router::LayerTable;
use crate::kicad_nets::{footprint_reference, footprint_uuid};
use crate::{Expr, KiCadBoardPlacerConfig, KiCadRunFacts, form_at, form_atom, parse};
use pcb_kicad_file::{geometry::rotate_vector, outline::board_loops_strict};
use pcb_kicad_place::{
    board_placer::lower_placement,
    placement_constraints::{constraint_report_at, resolve_constraints},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

type Point = [f64; 2];
type EdgeSignature = BTreeSet<[[i64; 2]; 2]>;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadReferenceKeepoutOverlap {
    pub reference_board: PathBuf,
    pub footprint: String,
    pub antenna_zone: String,
    pub retained_zone: String,
    pub layers: Vec<String>,
    pub antenna_area_mm2: f64,
    pub reference_overlap_mm2: f64,
    pub saved_overlap_mm2: f64,
    pub reference_pose: KiCadKeepoutPose,
    pub saved_pose: KiCadKeepoutPose,
    /// The reference pose passed the current placement model with other parts held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_pose: Option<KiCadKeepoutPose>,
    #[serde(skip)]
    review_alternative: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct KiCadKeepoutPose {
    pub x_mm: f64,
    pub y_mm: f64,
    pub angle_degrees: f64,
}

impl From<[f64; 3]> for KiCadKeepoutPose {
    fn from([x_mm, y_mm, angle]: [f64; 3]) -> Self {
        let angle_degrees = (angle + 180.0).rem_euclid(360.0) - 180.0;
        Self {
            x_mm,
            y_mm,
            angle_degrees,
        }
    }
}

fn edges(points: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
}

fn cross(a: Point, b: Point, p: Point) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn signed_area(points: &[Point]) -> f64 {
    let Some(origin) = points.first() else {
        return 0.0;
    };
    edges(points).fold(0.0, |sum, (a, b)| sum + cross(*origin, a, b)) * 0.5
}

fn convex_contour(zone: &Expr) -> Option<Vec<Point>> {
    let mut polygons = zone
        .children()
        .iter()
        .filter(|item| item.head() == Some("polygon"));
    let (Some(boundary), None) = (polygons.next(), polygons.next()) else {
        return None;
    };
    let points: Vec<Point> = boundary
        .child("pts")?
        .children()
        .iter()
        .skip(1)
        .map(|point| {
            let [_, x, y] = point.children() else {
                return None;
            };
            (point.head() == Some("xy")).then_some(())?;
            let xy = [
                x.atom()?.parse::<f64>().ok()?,
                y.atom()?.parse::<f64>().ok()?,
            ];
            xy.iter().all(|value| value.is_finite()).then_some(xy)
        })
        .collect::<Option<_>>()?;
    if points.len() < 3 || points.len() > 128 {
        return None;
    }
    let area = signed_area(&points);
    (area.abs() > 1.0e-6
        && edges(&points).all(|(a, b)| {
            a != b
                && points
                    .iter()
                    .all(|point| area.signum() * cross(a, b, *point) >= -1.0e-9)
        }))
    .then_some(points)
}

fn intersection_area(subject: &[Point], clip: &[Point]) -> f64 {
    let orientation = signed_area(clip).signum();
    let mut vertices = subject.to_vec();
    for (a, b) in edges(clip) {
        let mut next = Vec::new();
        for (start, end) in edges(&vertices) {
            let start_side = orientation * cross(a, b, start);
            let end_side = orientation * cross(a, b, end);
            if (start_side >= 0.0) != (end_side >= 0.0) {
                let t = start_side / (start_side - end_side);
                next.push([
                    start[0] + t * (end[0] - start[0]),
                    start[1] + t * (end[1] - start[1]),
                ]);
            }
            if end_side >= 0.0 {
                next.push(end);
            }
        }
        vertices = next;
    }
    signed_area(&vertices).abs()
}

fn edge_signature(points: &[Point], units_per_mm: f64) -> EdgeSignature {
    let point = |p: Point| p.map(|value| (value * units_per_mm).round() as i64);
    edges(points)
        .filter_map(|(a, b)| {
            let mut ends = [point(a), point(b)];
            ends.sort_unstable();
            (ends[0] != ends[1]).then_some(ends)
        })
        .collect()
}

fn outline_signature(pcb: &Expr) -> Option<Vec<EdgeSignature>> {
    let outline = board_loops_strict(pcb).ok()?;
    let mut paths: Vec<_> = outline
        .piece_loops
        .iter()
        .chain(
            outline
                .cutouts
                .iter()
                .zip(outline.board_cutouts)
                .filter_map(|(path, board)| board.then_some(path)),
        )
        .map(|path| edge_signature(path, 1_000.0))
        .collect();
    paths.sort();
    Some(paths)
}

fn unique_items<'a>(
    items: impl Iterator<Item = (String, &'a Expr)>,
) -> BTreeMap<String, Option<&'a Expr>> {
    let mut unique = BTreeMap::new();
    for (key, item) in items {
        unique
            .entry(key)
            .and_modify(|value| *value = None)
            .or_insert(Some(item));
    }
    unique
}

fn zones(item: &Expr) -> BTreeMap<String, Option<&Expr>> {
    unique_items(
        item.children()
            .iter()
            .filter(|child| child.head() == Some("zone"))
            .filter_map(|zone| Some((footprint_uuid(zone)?, zone))),
    )
}

fn footprints(pcb: &Expr) -> BTreeMap<String, Option<&Expr>> {
    unique_items(
        pcb.children()
            .iter()
            .filter(|child| matches!(child.head(), Some("footprint" | "module")))
            .filter_map(|footprint| Some((footprint_reference(footprint)?, footprint))),
    )
}

fn copper_keepout(zone: &Expr, stack: &LayerTable) -> Option<(u32, bool, Vec<Point>)> {
    let keepout = zone.child("keepout")?;
    if !["tracks", "vias", "pads", "copperpour"]
        .iter()
        .all(|kind| form_atom(keepout, kind, 1) == Some("not_allowed"))
    {
        return None;
    }
    let mask = stack.mask_of_item(zone).ok()?;
    let blocks_parts = form_atom(keepout, "footprints", 1) == Some("not_allowed");
    (mask != 0).then_some((mask, blocks_parts, convex_contour(zone)?))
}

fn local_outline(polygon: &[Point], pose: [f64; 3]) -> EdgeSignature {
    let points: Vec<_> = polygon
        .iter()
        .map(|p| rotate_vector([p[0] - pose[0], p[1] - pose[1]], pose[2]))
        .collect();
    edge_signature(&points, 1_000.0)
}

fn compare(
    reference: &Expr,
    saved: &Expr,
    board_path: &Path,
) -> Option<Vec<KiCadReferenceKeepoutOverlap>> {
    let stack = LayerTable::from_pcb(reference).ok()?;
    if stack.names != LayerTable::from_pcb(saved).ok()?.names
        || outline_signature(reference)? != outline_signature(saved)?
    {
        return None;
    }
    let blocks = zones(reference);
    let saved_blocks = zones(saved);
    let saved_footprints = footprints(saved);
    let mut rows = Vec::new();
    for (footprint, original) in footprints(reference) {
        let (Some(original), Some(Some(current))) = (original, saved_footprints.get(&footprint))
        else {
            continue;
        };
        let side = form_atom(original, "layer", 1);
        if !matches!(side, Some("F.Cu" | "B.Cu"))
            || side != form_atom(current, "layer", 1)
            || original.children().get(1)?.atom()? != current.children().get(1)?.atom()?
        {
            continue;
        }
        let (Ok(before_pose), Ok(after_pose)) = (form_at(original), form_at(current)) else {
            continue;
        };
        let current_zones = zones(current);
        for (antenna_zone, before) in zones(original) {
            let (Some(before), Some(Some(after))) = (before, current_zones.get(&antenna_zone))
            else {
                continue;
            };
            let named = |zone| {
                form_atom(zone, "name", 1)
                    .is_some_and(|name| name.eq_ignore_ascii_case("antenna keepout"))
            };
            if !named(before) || !named(after) {
                continue;
            }
            let (Some((layers, pre_parts, before)), Some((saved_layers, post_parts, after))) = (
                copper_keepout(before, &stack),
                copper_keepout(after, &stack),
            ) else {
                continue;
            };
            if layers != saved_layers
                || pre_parts != post_parts
                || local_outline(&before, before_pose) != local_outline(&after, after_pose)
            {
                continue;
            }
            let total = signed_area(&before).abs();
            for (retained_zone, area) in &blocks {
                let (Some(before_area), Some(Some(after_area))) =
                    (area, saved_blocks.get(retained_zone))
                else {
                    continue;
                };
                let (
                    Some((board_layers, old_parts, strip)),
                    Some((new_layers, new_parts, saved_strip)),
                ) = (
                    copper_keepout(before_area, &stack),
                    copper_keepout(after_area, &stack),
                )
                else {
                    continue;
                };
                let common = board_layers & layers;
                if common == 0
                    || board_layers != new_layers
                    || old_parts != new_parts
                    || edge_signature(&strip, 1_000_000.0)
                        != edge_signature(&saved_strip, 1_000_000.0)
                {
                    continue;
                }
                let reference_overlap_mm2 = intersection_area(&before, &strip);
                let saved_overlap_mm2 = intersection_area(&after, &saved_strip);
                if reference_overlap_mm2 >= total * 0.5 && saved_overlap_mm2 <= 1.0e-6 {
                    rows.push(KiCadReferenceKeepoutOverlap {
                        reference_board: board_path.into(),
                        footprint: footprint.clone(),
                        antenna_zone: antenna_zone.clone(),
                        retained_zone: retained_zone.clone(),
                        layers: stack
                            .names
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| common & (1 << index) != 0)
                            .map(|(_, name)| name.clone())
                            .collect(),
                        antenna_area_mm2: total,
                        reference_overlap_mm2,
                        saved_overlap_mm2,
                        reference_pose: before_pose.into(),
                        saved_pose: after_pose.into(),
                        trial_pose: None,
                        review_alternative: false,
                    });
                }
            }
        }
    }
    Some(rows)
}

fn test_reference_poses(
    rows: &mut [KiCadReferenceKeepoutOverlap],
    saved: &Expr,
    config: &KiCadBoardPlacerConfig,
    source: &Path,
) {
    let Ok(config) = resolve_constraints(config, source) else {
        return;
    };
    let Ok(lowered) = lower_placement(saved, &config) else {
        return;
    };
    let problem = &lowered.problem;
    let same_angle = |a: f64, b: f64| ((a - b + 180.0).rem_euclid(360.0) - 180.0).abs() < 1.0e-6;
    let footprint_count = saved
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .count();
    if !problem
        .poses
        .iter()
        .zip(&lowered.source_at)
        .take(footprint_count)
        .all(|(pose, at)| {
            (pose.position[0] - at[0]).abs() < 1.0e-6
                && (pose.position[1] - at[1]).abs() < 1.0e-6
                && same_angle(pose.angle, at[2])
        })
    {
        return;
    }
    for row in rows {
        let Some(index) = lowered
            .references
            .iter()
            .position(|name| name == &row.footprint)
        else {
            continue;
        };
        let at = row.reference_pose;
        let component = &problem.components[index];
        if component.fixed
            || !component
                .angle_options
                .iter()
                .any(|angle| same_angle(*angle, at.angle_degrees))
        {
            continue;
        }
        let pose = pcb_placer::Pose {
            position: [at.x_mm, at.y_mm],
            angle: at.angle_degrees,
        };
        let mut poses = problem.poses.clone();
        poses[index] = pose;
        if !constraint_report_at(&lowered, &poses, None)
            .iter()
            .all(|row| row.requirement_satisfied())
        {
            continue;
        }
        row.review_alternative = true;
        if pcb_placer::legal::is_legal(problem, &poses, index, pose, 0..poses.len()) {
            row.trial_pose = Some(at);
        }
    }
}

pub fn note_reference_keepouts(
    facts: &mut KiCadRunFacts,
    reference: &Path,
    board: &Path,
    config: &KiCadBoardPlacerConfig,
    source: &Path,
) {
    let read = |path| parse(&fs::read_to_string(path).ok()?).ok();
    let (Some(reference_board), Some(saved)) = (read(reference), read(board)) else {
        return;
    };
    if let Some(mut rows) = compare(&reference_board, &saved, reference) {
        if !rows.is_empty() {
            test_reference_poses(&mut rows, &saved, config, source);
        }
        facts.reference_keepout_overlaps = rows;
    }
}

pub(crate) fn actions(rows: &[KiCadReferenceKeepoutOverlap]) -> Vec<String> {
    rows.iter().take(4).map(|row| {
        let trial = if let Some(pose) = row.trial_pose { format!(
            " The reference pose ({:.3}, {:.3}) mm at {}° passed the current placement checks with other poses held. Try that pose on a copy and reroute if you want to restore the overlap.",
            pose.x_mm, pose.y_mm, pose.angle_degrees)
        } else if row.review_alternative {
            format!(" If you want to restore the overlap, review the available regions and orientations for {} in the current brief and verify a rerouted trial.", row.footprint)
        } else {
            String::new()
        };
        format!("{}'s antenna keepout overlapped {:.3} mm² of retained area {} on {} in the reference. The saved board has no measurable overlap. This is a reference observation.{trial}",
                row.footprint, row.reference_overlap_mm2, row.retained_zone, row.layers.join(" and "))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(pose: [f64; 3]) -> String {
        let [x, y, angle] = pose;
        let points = [[-7.7, 7.7], [7.7, 7.7], [7.7, 12.8], [-7.7, 12.8]];
        let contour: String = points
            .iter()
            .map(|&point| {
                let [dx, dy] = rotate_vector(point, -angle);
                format!("(xy {:.6} {:.6})", x + dx, y + dy)
            })
            .collect();
        format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
            (gr_rect (start 86 57) (end 214 143) (layer "Edge.Cuts"))
            (zone (uuid "retained") (layers "F&B.Cu")
                (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints allowed))
                (polygon (pts (xy 208.1 57.5) (xy 207.8 142) (xy 213.9 142.2) (xy 213.9 57.5))))
            (footprint "Radio:Module" (layer "F.Cu") (at {x} {y} {angle}) (property "Reference" "U10")
                (fp_rect (start -2 -2) (end 2 2) (stroke (width 0.05) (type default)) (fill none) (layer "F.CrtYd"))
                (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu" "F.Mask"))
                (zone (uuid "local") (name "antenna keepout") (layers "*.Cu")
                    (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints not_allowed))
                    (polygon (pts {contour})))))"#
        )
    }

    #[test]
    fn a_slanted_strip_loses_overlap_only_when_the_same_antenna_moves_away() {
        let reference = parse(&fixture([200.18, 99.5, 90.0])).unwrap();
        let moved_file = fixture([184.785, 71.12, -90.0]);
        let moved = parse(&moved_file).unwrap();
        let measure = |after: &Expr| {
            compare(&reference, after, Path::new("reference.kicad_pcb")).unwrap_or_default()
        };
        let rows = measure(&moved);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].layers, ["F.Cu", "B.Cu"]);
        assert!((rows[0].reference_overlap_mm2 - 77.4483325).abs() < 0.000_02);
        assert_eq!(rows[0].saved_overlap_mm2, 0.0);
        assert!(rows[0].trial_pose.is_none());
        let alternate = parse(&fixture([200.025, 71.755, 90.0])).unwrap();
        assert!(measure(&alternate).is_empty());
        assert!(measure(&reference).is_empty());
        assert!(
            compare(&moved, &reference, Path::new("reference.kicad_pcb"))
                .unwrap()
                .is_empty()
        );
        for edited in [
            moved_file.replace("\"retained\"", "\"other\""),
            moved_file.replace("208.1 57.5", "208.5 57.5"),
            moved_file.replace("\"F&B.Cu\"", "\"B.Cu\""),
            moved_file.replace("(layer \"F.Cu\") (at", "(layer \"B.Cu\") (at"),
            moved_file.replace("antenna keepout", "generic keepout"),
        ] {
            assert!(measure(&parse(&edited).unwrap()).is_empty());
        }
    }

    #[test]
    fn a_pose_trial_requires_the_current_arrangement_to_allow_it() {
        let reference = parse(&fixture([200.18, 99.5, 90.0])).unwrap();
        let saved = parse(&fixture([184.785, 71.12, -90.0])).unwrap();
        let path = Path::new("reference.kicad_pcb");
        let config = |brief| KiCadBoardPlacerConfig {
            constraints: Some(crate::KiCadConstraintsSource::Inline(
                serde_json::from_value(brief).unwrap(),
            )),
            ..Default::default()
        };
        let mut compatible = compare(&reference, &saved, path).unwrap();
        test_reference_poses(
            &mut compatible,
            &saved,
            &config(serde_json::json!({"version":1,"move_all":true,"front":["U10"]})),
            Path::new("."),
        );
        assert_eq!(compatible[0].trial_pose, Some(compatible[0].reference_pose));

        let mut held = compare(&reference, &saved, path).unwrap();
        test_reference_poses(
            &mut held,
            &saved,
            &config(serde_json::json!({"version":1,"move_all":true,"place":[{
                "part":"U10", "x":184.785, "y":71.12, "angle":-90, "origin":"absolute"
            }]})),
            Path::new("."),
        );
        assert!(held[0].trial_pose.is_none());
        assert!(!actions(&held)[0].contains("Try that pose"));
        assert!(!actions(&held)[0].contains("review the available regions"));
    }
}

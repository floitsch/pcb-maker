// Copyright (C) 2026 Florian Loitsch.

//! Correct a narrow contact between a new local track and a kept track.

use super::*;
use pcb_router::geometry::{distance, point_segment_distance};

type Point = [i64; 2];

const NM_PER_MM: f64 = 1.0e6;

fn millimeters(point: Point) -> [f64; 2] {
    point.map(|coord| coord as f64 / NM_PER_MM)
}

fn point(item: &Expr, form: &str) -> Option<Point> {
    let at = form_xy(item, form).ok()?;
    at.iter()
        .all(|coord| coord.is_finite() && coord.abs() < 1.0e6)
        .then(|| at.map(|coord| (coord * NM_PER_MM).round() as i64))
}

struct Track<'a> {
    points: [Point; 2],
    layer: &'a str,
    width: f64,
}

impl<'a> Track<'a> {
    fn read(item: &'a Expr) -> Option<Self> {
        let points = [point(item, "start")?, point(item, "end")?];
        let width = form_f64(item, "width", 1).ok()?;
        (item.head() == Some("segment")
            && points[0] != points[1]
            && width.is_finite()
            && width > 0.0)
            .then_some(Self {
                points,
                layer: form_atom(item, "layer", 1)?,
                width,
            })
    }
}

/// A point exactly on the kept centerline on KiCad's integer nanometre grid.
fn centerline(at: Point, [a, b]: [Point; 2]) -> Point {
    let delta = [b[0] - a[0], b[1] - a[1]];
    let [mut gcd, mut remainder] = delta.map(i64::abs);
    while remainder != 0 {
        (gcd, remainder) = (remainder, gcd % remainder);
    }
    let step = delta.map(|coord| coord / gcd);
    let dot = |u: Point, v: Point| {
        i128::from(u[0]) * i128::from(v[0]) + i128::from(u[1]) * i128::from(v[1])
    };
    let along = dot([at[0] - a[0], at[1] - a[1]], step) as f64 / dot(step, step) as f64;
    let index = (along.round() as i64).clamp(0, gcd);
    [a[0] + index * step[0], a[1] + index * step[1]]
}

/// At most two orthogonal/45° pieces. When possible, the piece approaching
/// the joint runs on the direction of the track it meets.
fn run(from: Point, to: Point, [a, b]: [Point; 2]) -> Vec<Point> {
    let delta = [to[0] - from[0], to[1] - from[1]];
    let small = delta[0].abs().min(delta[1].abs());
    if small == 0 || delta[0].abs() == delta[1].abs() {
        return vec![from, to];
    }
    let diagonal = delta.map(|coord| coord.signum() * small);
    let diagonal_last = i128::from(diagonal[0]) * i128::from(b[1] - a[1])
        == i128::from(diagonal[1]) * i128::from(b[0] - a[0]);
    let corner = if diagonal_last {
        [to[0] - diagonal[0], to[1] - diagonal[1]]
    } else {
        [from[0] + diagonal[0], from[1] + diagonal[1]]
    };
    vec![from, corner, to]
}

fn is_leaf<'a>(at: Point, track: &Track, mut others: impl Iterator<Item = &'a Expr>) -> bool {
    let at = millimeters(at);
    others.all(|item| match item.head() {
        Some("segment") => Track::read(item).is_some_and(|other| {
            other.layer != track.layer
                || point_segment_distance(
                    at,
                    millimeters(other.points[0]),
                    millimeters(other.points[1]),
                ) > (track.width + other.width) / 2.0
        }),
        Some("via") => point(item, "at")
            .zip(form_f64(item, "size", 1).ok())
            .is_some_and(|(center, diameter)| {
                distance(at, millimeters(center)) > (track.width + diameter) / 2.0
            }),
        _ => false,
    })
}

#[derive(Serialize)]
pub struct TrackContact {
    pub kept_track: String,
    pub replaced_track: String,
    pub from_mm: [f64; 2],
    pub to_mm: [f64; 2],
}

#[derive(Serialize)]
pub struct ContactRefinement {
    pub original_report: String,
    pub contacts: Vec<TrackContact>,
}

pub struct Adjusted {
    pub objects: Vec<Expr>,
    pub contacts: Vec<TrackContact>,
}

/// Use the same exact contact correction for an automatically proposed
/// additive branch accompanying a separate, already bounded relocation.
pub fn refine_generated_contact(
    pcb: &Expr,
    added: &[Expr],
    net: &str,
    before: &Value,
    trial: &Value,
) -> Result<Option<Adjusted>, String> {
    refine(pcb, &[], added, net, before, trial)
}

/// Rebuild only emitted leaf tracks identified by KiCad's new width
/// findings. A different finding, route geometry, or junction retains the
/// original native rejection. The caller verifies the replacement anew.
pub(super) fn refine(
    pcb: &Expr,
    prior: &[Expr],
    added: &[Expr],
    net: &str,
    before: &Value,
    trial: &Value,
) -> Result<Option<Adjusted>, String> {
    if report_capped(before)
        || report_capped(trial)
        || !same_check_policy(before, trial)
        || adds_findings(before, trial, "schematic_parity")?
    {
        return Ok(None);
    }
    let mut old_findings = finding_counts(before, "violations")?;
    let mut findings = Vec::new();
    for finding in trial["violations"].as_array().into_iter().flatten() {
        let mut normalized = finding.clone();
        if let Some(items) = normalized["items"].as_array_mut() {
            items.sort_by_cached_key(Value::to_string);
        }
        if let Some(count) = old_findings.get_mut(&normalized.to_string())
            && *count > 0
        {
            *count -= 1;
            continue;
        }
        if finding["type"] != "connection_width" || finding["excluded"] == true {
            return Ok(None);
        }
        findings.push(finding);
    }
    if findings.is_empty() {
        return Ok(None);
    }
    let codes = net_codes(pcb);
    let names = note_net_names(pcb);
    let old_ids: BTreeMap<_, _> = pcb
        .children()
        .iter()
        .filter(|item| route_item(item))
        .filter_map(|item| Some((item_id(item)?, item)))
        .collect();
    let new_ids: BTreeMap<_, _> = added
        .iter()
        .enumerate()
        .filter_map(|(index, item)| Some((item_id(item)?, index)))
        .collect();
    let mut edits = BTreeMap::new();
    let mut contacts = Vec::new();
    for finding in findings {
        let patch = (|| {
            let description = finding["description"].as_str()?;
            let (limit, _) = description.split_once(" mm; actual ")?;
            let minimum: f64 = limit.rsplit_once(' ')?.1.parse().ok()?;
            if !minimum.is_finite() || minimum <= 0.0 {
                return None;
            }
            let [a, b] = finding["items"].as_array()?.as_slice() else {
                return None;
            };
            let a = a["uuid"].as_str()?.to_ascii_lowercase();
            let b = b["uuid"].as_str()?.to_ascii_lowercase();
            let (old, new) = if old_ids.contains_key(&a) {
                (a, b)
            } else {
                (b, a)
            };
            let kept = old_ids.get(&old)?;
            let index = *new_ids.get(&new)?;
            let item = &added[index];
            let (source, active) = (Track::read(kept)?, Track::read(item)?);
            if source.layer != active.layer
                || source.width.min(active.width) < minimum
                || item_net(kept, &codes) != Some(net)
                || item_net(item, &codes) != Some(net)
                || edits.contains_key(&index)
            {
                return None;
            }
            let endpoint = |at| {
                let to = centerline(at, source.points);
                let displacement = distance(millimeters(at), millimeters(to));
                (displacement > 0.0
                    && displacement <= (source.width + active.width) / 2.0
                    && is_leaf(
                        at,
                        &active,
                        added
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| *i != index)
                            .map(|(_, item)| item),
                    ))
                .then_some(to)
            };
            let [a, b] = active.points;
            let (from, tip, to) = match (endpoint(a), endpoint(b)) {
                (Some(to), None) => (b, a, to),
                (None, Some(to)) => (a, b, to),
                _ => return None,
            };
            if from == to {
                return None;
            }
            let form = item.child("net")?;
            let pieces = run(from, to, source.points)
                .windows(2)
                .map(|edge| {
                    let mut shape = supplemental_segment_expr(&SupplementalSegment {
                        connection: names.normalize(net).to_string(),
                        start: millimeters(edge[0]),
                        end: millimeters(edge[1]),
                        width: active.width,
                        layer: active.layer.to_string(),
                    });
                    if let Expr::List(children) = &mut shape {
                        for child in children
                            .iter_mut()
                            .filter(|child| child.head() == Some("net"))
                        {
                            *child = form.clone();
                        }
                    }
                    shape
                })
                .collect::<Vec<_>>();
            Some((
                index,
                pieces,
                TrackContact {
                    kept_track: old,
                    replaced_track: new,
                    from_mm: millimeters(tip),
                    to_mm: millimeters(to),
                },
            ))
        })();
        let Some((index, pieces, contact)) = patch else {
            return Ok(None);
        };
        edits.insert(index, pieces);
        contacts.push(contact);
    }
    let revised: Vec<_> = added
        .iter()
        .enumerate()
        .flat_map(|(index, item)| edits.remove(&index).unwrap_or_else(|| vec![item.clone()]))
        .collect();
    let mut ids = item_ids(pcb);
    ids.extend(prior.iter().filter_map(item_id));
    if revised.len() > MAX_NET_ITEMS
        || !revised
            .iter()
            .all(|item| item_id(item).is_some_and(|id| ids.insert(id)))
    {
        return Ok(None);
    }
    Ok(Some(Adjusted {
        objects: revised,
        contacts,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_new_leaf_can_extend_to_the_kept_center_without_moving_the_other_end() {
        let pcb = parse(r#"(kicad_pcb
            (segment (start 1.1 0) (end 2 0) (width 0.15) (layer "F.Cu") (net "SIG") (uuid "kept")))"#).unwrap();
        let added = parse(r#"(segment (start 1 0.05) (end 0.75 0.3) (width 0.15) (layer "F.Cu") (net "SIG") (uuid "leaf"))"#).unwrap();
        let via = parse(r#"(via (at 0.75 0.3) (size 0.45) (drill 0.25) (layers "F.Cu" "B.Cu") (net "SIG") (uuid "via"))"#).unwrap();
        let baseline = json!({"violations": []});
        let warning = json!({"type": "connection_width", "severity": "warning",
            "description": "Copper connection too narrow (minimum connection width 0.1270 mm; actual 0.1009 mm) (F.Cu)",
            "items": [{"uuid": "kept"}, {"uuid": "leaf"}],
        });
        let after = json!({"violations": [warning]});
        let adjusted = refine(
            &pcb,
            &[],
            &[added.clone(), via.clone()],
            "SIG",
            &baseline,
            &after,
        )
        .unwrap()
        .unwrap();
        assert_eq!(adjusted.contacts[0].to_mm, [1.1, 0.0]);
        let [a, b, retained_via] = adjusted.objects.as_slice() else {
            panic!("two track pieces and the original via")
        };
        assert_eq!(form_xy(a, "start").unwrap(), [0.75, 0.3]);
        assert_eq!(form_xy(a, "end").unwrap(), [1.05, 0.0]);
        assert_eq!(form_xy(b, "end").unwrap(), [1.1, 0.0]);
        assert_eq!(form_f64(b, "width", 1).unwrap(), 0.15);
        assert_eq!(*retained_via, via);

        let branch = parse(r#"(segment (start 1 0.05) (end 1 -0.2) (width 0.15) (layer "F.Cu") (net "SIG") (uuid "branch"))"#).unwrap();
        assert!(
            refine(
                &pcb,
                &[],
                &[added.clone(), branch],
                "SIG",
                &baseline,
                &after
            )
            .unwrap()
            .is_none()
        );
        let mut impossible = after.clone();
        impossible["violations"][0]["description"] = json!(
            "Copper connection too narrow (minimum connection width 0.18 mm; actual 0.1009 mm) (F.Cu)"
        );
        assert!(
            refine(
                &pcb,
                &[],
                std::slice::from_ref(&added),
                "SIG",
                &baseline,
                &impossible,
            )
            .unwrap()
            .is_none()
        );
        let mut unrelated = after.clone();
        unrelated["violations"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"clearance"}));
        assert!(
            refine(&pcb, &[], &[added], "SIG", &baseline, &unrelated)
                .unwrap()
                .is_none()
        );
    }
}

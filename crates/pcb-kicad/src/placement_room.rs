// Copyright (C) 2026 Toit contributors.

//! Room as numbers: how full each side of a board is, how much area is
//! missing, and an outline that holds the parts, so that an agent whose
//! placement found no room enlarges the board in one step rather than by
//! guessing. Reported as `room` in `board-placer.json` and in the verdict.
//!
//! Also the quick tests of constraints against the outline: a row longer
//! than the board, or parts held at one edge that need more length than
//! the edge has, cannot be placed at any relaxation level; they are input
//! errors at once, as a region smaller than its part is.

use pcb_placer as core;
use pcb_placer::constraints::Edge;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// The share of a side's usable area the parts' bodies (with spacing) may
/// take before placement gets hard: above it the hints call a side full,
/// and the outlines suggested here are sized to it.
pub const TARGET_UTILIZATION: f64 = 0.7;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KiCadRoom {
    /// The outline's bounding box, width and height.
    pub outline_mm: [f64; 2],
    /// Per side (front, back): the outline's area less the band along its
    /// edge that bodies keep free (the edge margin) and less the fixed
    /// parts' bodies there.
    pub usable_mm2: [f64; 2],
    /// Per side: the movable parts' bodies (courtyards, or the body box
    /// without one) with the spacing around them; a row is one body.
    pub parts_mm2: [f64; 2],
    /// `parts_mm2` over `usable_mm2`, per side.
    pub utilization: [f64; 2],
    pub target_utilization: f64,
    /// Per side: the usable area missing for the parts at the target.
    pub missing_mm2: [f64; 2],
    /// The smallest outline (at the current one's aspect, unless large
    /// parts need another) whose usable area holds every side's parts at
    /// the target, whose strips across it hold its large parts side by
    /// side as full as the target, and that holds the largest part, every
    /// row and every edge's parts (as they may turn). An estimate from
    /// sizes: the placement may need more.
    pub smallest_outline_mm: [f64; 2],
    /// An outline to try next, when parts found no place or the outline
    /// cannot hold a row or an edge's parts: the larger of the current
    /// outline and `smallest_outline_mm` per dimension, or, where area is
    /// not what is missing, the current one with a quarter more area.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_outline_mm: Option<[f64; 2]>,
    /// What bounds the outline from below besides area: rows, edges,
    /// parts larger than the board.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<String>,
}

/// The room a pre-check found missing (`note_room`), for the verdict of a
/// run that ended in an input error.
static NOTED: Mutex<Option<KiCadRoom>> = Mutex::new(None);

pub fn note_room(room: KiCadRoom) {
    if let Ok(mut noted) = NOTED.lock() {
        *noted = Some(room);
    }
}

/// The room the last failed pre-check noted, if any.
pub fn noted_room() -> Option<KiCadRoom> {
    NOTED.lock().ok().and_then(|noted| noted.clone())
}

fn polygon_area(outline: &[core::Point]) -> f64 {
    (0..outline.len())
        .map(|index| {
            let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

fn polygon_perimeter(outline: &[core::Point]) -> f64 {
    (0..outline.len())
        .map(|index| {
            let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
            ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
        })
        .sum()
}

/// The box sizes (width, height) a part takes at the angles it may turn
/// to, `tight` taking the body without the courtyard's margin where it
/// has one (the most it can shrink).
fn sizes(component: &core::Component, tight: bool) -> Vec<[f64; 2]> {
    let mut size = component.body_size;
    if tight && let Some(box_) = component.tight {
        size = [size[0].min(box_[2] - box_[0]), size[1].min(box_[3] - box_[1])];
    }
    let mut out: Vec<[f64; 2]> = component
        .angle_options
        .iter()
        .map(|angle| {
            let turns = (angle.rem_euclid(360.0) / 90.0).round() as i64 % 2;
            let quarter = ((angle.rem_euclid(360.0) / 90.0) - (angle.rem_euclid(360.0) / 90.0).round()).abs() < 1.0e-6;
            if !quarter {
                // Any angle: the box fits either way round.
                [size[0].min(size[1]), size[0].min(size[1])]
            } else if turns == 0 {
                size
            } else {
                [size[1], size[0]]
            }
        })
        .collect();
    if out.is_empty() {
        out.push(size);
    }
    out
}

/// The copper's (pads') extent of a part at each of its angles, as `sizes`.
fn copper_sizes(component: &core::Component) -> Vec<[f64; 2]> {
    let mut extent = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for pad in &component.pads {
        extent = [extent[0].min(pad[0]), extent[1].min(pad[1]), extent[2].max(pad[2]), extent[3].max(pad[3])];
    }
    let size = if extent[0].is_finite() { [extent[2] - extent[0], extent[3] - extent[1]] } else { [0.0, 0.0] };
    let mut copy = component.clone();
    copy.body_size = size;
    copy.tight = None;
    sizes(&copy, false)
}

/// Parts held at one edge on one side, and the length along the edge they
/// need at least.
pub struct EdgeNeed {
    pub edge: Edge,
    pub side: &'static str,
    pub parts: Vec<usize>,
    pub length: f64,
}

fn along(edge: Edge, size: [f64; 2]) -> (f64, f64) {
    match edge {
        Edge::Top | Edge::Bottom => (size[0], size[1]),
        Edge::Left | Edge::Right => (size[1], size[0]),
    }
}

/// For each edge and side, the movable parts an edge constraint holds
/// there and the least length along the edge they need. Only parts that
/// cannot stand two deep within their reach of the edge count (every
/// orientation deeper than the reach): they lie in single file. Each
/// counts its shortest orientation, by its tight body; the two that could
/// overhang the edge's ends most (the last relaxation level keeps only the
/// copper of parts held at an edge on the board) count only their copper.
/// A lower bound: a set that needs more than its edge cannot be placed.
pub fn edge_needs(problem: &core::Problem) -> Vec<EdgeNeed> {
    let constraints = &problem.constraints;
    let mut needs = Vec::new();
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        for (side, name) in [(core::Side::Front, "front"), (core::Side::Back, "back")] {
            let mut parts: Vec<usize> = Vec::new();
            let mut length = 0.0;
            let mut slack: Vec<f64> = Vec::new();
            for (part, held, reach) in &constraints.edges {
                let component = &problem.components[*part];
                // A hollow part blocks with its pads alone.
                if *held != edge
                    || component.fixed
                    || component.copper_only
                    || !component.hollow.is_empty()
                    || !component.side.collides(side)
                    || parts.contains(part)
                    || constraints.overhangs.iter().any(|(other, _, _)| other == part)
                {
                    continue;
                }
                let bodies = sizes(component, true);
                if bodies.iter().any(|size| along(edge, *size).1 <= *reach) {
                    continue;
                }
                let copper = copper_sizes(component);
                length += bodies.iter().map(|size| along(edge, *size).0).fold(f64::INFINITY, f64::min);
                slack.push(
                    bodies
                        .iter()
                        .zip(&copper)
                        .map(|(body, copper)| (along(edge, *body).0 - along(edge, *copper).0).max(0.0))
                        .fold(0.0, f64::max),
                );
                parts.push(*part);
            }
            if parts.is_empty() {
                continue;
            }
            slack.sort_by(|a, b| b.total_cmp(a));
            length -= slack.iter().take(2).sum::<f64>();
            needs.push(EdgeNeed { edge, side: name, parts, length: length.max(0.0) });
        }
    }
    needs
}

fn round_up(value: f64) -> f64 {
    (value * 2.0 - 1.0e-9).ceil() / 2.0
}

fn edge_name(edge: Edge) -> &'static str {
    match edge {
        Edge::Left => "left",
        Edge::Right => "right",
        Edge::Top => "top",
        Edge::Bottom => "bottom",
    }
}

/// The room on `problem`'s board (its bodies unrelaxed, as lowered).
/// `wanting`: parts found no place or the outline failed a pre-check, so
/// a next outline is suggested.
pub fn room(problem: &core::Problem, references: &[String], wanting: bool) -> KiCadRoom {
    let bounds = problem.bounds();
    let outline = [bounds[2] - bounds[0], bounds[3] - bounds[1]];
    let margin = problem.edge_margin.max(0.0);
    // Holes the user cut into the outline hold no part.
    let holes: f64 = problem
        .constraints
        .solid_cutouts
        .iter()
        .map(|index| polygon_area(&problem.components[*index].cutout_outline))
        .sum();
    let inner = (polygon_area(&problem.outline) - polygon_perimeter(&problem.outline) * margin - holes).max(0.0);
    let mut fixed = [0.0f64; 2];
    let mut parts = [0.0f64; 2];
    let sides = |side: core::Side| -> &'static [usize] {
        match side {
            core::Side::Front => &[0],
            core::Side::Back => &[1],
            core::Side::Both => &[0, 1],
            core::Side::Neither => &[],
        }
    };
    for component in &problem.components {
        if component.copper_only {
            continue;
        }
        let area = if !component.hollow.is_empty() {
            component.blocking_area()
        } else if component.fixed {
            component.body_size[0] * component.body_size[1]
        } else {
            (component.body_size[0] + problem.spacing) * (component.body_size[1] + problem.spacing)
        };
        for side in sides(component.side) {
            if component.fixed {
                fixed[*side] += area;
            } else {
                parts[*side] += area;
            }
        }
    }
    let usable = fixed.map(|fixed| (inner - fixed).max(0.0));
    let ratio = |side: usize| if usable[side] > 0.0 { parts[side] / usable[side] } else if parts[side] > 0.0 { 99.0 } else { 0.0 };
    let utilization = [ratio(0), ratio(1)].map(|share: f64| (share.min(99.0) * 1000.0).round() / 1000.0);
    let missing = [0, 1].map(|side| ((parts[side] / TARGET_UTILIZATION - usable[side]).max(0.0) * 10.0).round() / 10.0);

    // The smallest outline at this aspect: its area less the margin band
    // holds each side's fixed parts and its parts at the target.
    let aspect = if outline[1] > 0.0 { (outline[0] / outline[1]).max(0.05) } else { 1.0 };
    let need = (0..2).map(|side| parts[side] / TARGET_UTILIZATION + fixed[side]).fold(0.0, f64::max);
    // aspect h^2 - 2 (aspect + 1) margin h - need = 0
    let b = 2.0 * (aspect + 1.0) * margin;
    let height = (b + (b * b + 4.0 * aspect * need).sqrt()) / (2.0 * aspect);
    let mut smallest = [aspect * height, height];
    // Then large parts side by side (`strips_hold`): two 88 x 44 mm key
    // grids need 88 mm twice along one axis, whatever the area says (95 x
    // 90 mm at least for urchin's two halves, 118 x 107.5 at the target). By
    // their bodies, hollow ones too (their pads alone block, but a
    // suggestion that counts on interleaving them would mislead).
    let strip_parts: Vec<Vec<[f64; 2]>> = [core::Side::Front, core::Side::Back]
        .iter()
        .map(|side| {
            problem
                .components
                .iter()
                .enumerate()
                .filter(|(index, component)| {
                    !component.fixed
                        && !component.copper_only
                        && component.side.collides(*side)
                        && component.side != core::Side::Neither
                        && !problem.constraints.edges.iter().any(|(part, _, _)| part == index)
                        && problem.constraints.overhang(*index).is_none()
                })
                .map(|(_, component)| core::least_extents(component).map(|extent| extent + problem.spacing))
                .collect()
        })
        .collect();
    // At the target, as the area: the strips' length is at most that full.
    let slack = TARGET_UTILIZATION.sqrt();
    let holds = |outline: [f64; 2]| {
        strip_parts
            .iter()
            .all(|parts| core::strips_hold(parts, [(outline[0] - 2.0 * margin) * slack, (outline[1] - 2.0 * margin) * slack]))
    };
    if !holds(smallest) {
        // Growing either side (two grids side by side or one above the
        // other), in steps of 1 %: of the sizes within a tenth of the least
        // area, the one nearest the current aspect.
        let steps: Vec<f64> = (0..=300).map(|step| 1.01f64.powi(step)).collect();
        let candidates: Vec<[f64; 2]> = steps
            .iter()
            .filter_map(|grow| {
                let width = smallest[0] * grow;
                steps.iter().map(|grow| smallest[1] * grow).find(|height| holds([width, *height])).map(|height| [width, height])
            })
            .collect();
        let least = candidates.iter().map(|size| size[0] * size[1]).fold(f64::INFINITY, f64::min);
        if let Some(best) = candidates
            .iter()
            .filter(|size| size[0] * size[1] <= 1.1 * least)
            .min_by(|a, b| ((a[0] / a[1]) / aspect).ln().abs().total_cmp(&((b[0] / b[1]) / aspect).ln().abs()))
        {
            smallest = *best;
        }
    }
    let mut limits = Vec::new();
    // Every movable part (a row is one) must fit in some orientation,
    // margins included; grow the dimension that needs least.
    for (index, component) in problem.components.iter().enumerate() {
        if component.fixed || component.copper_only || component.side == core::Side::Neither {
            continue;
        }
        let options: Vec<[f64; 2]> =
            sizes(component, true).into_iter().map(|size| [size[0] + 2.0 * margin, size[1] + 2.0 * margin]).collect();
        let fits = |outline: [f64; 2]| options.iter().any(|size| size[0] <= outline[0] + 1.0e-6 && size[1] <= outline[1] + 1.0e-6);
        if !fits(outline) {
            let size = sizes(component, true)[0];
            let name = references.get(index).map_or("?", String::as_str);
            let row = problem.constraints.followers.iter().filter(|follower| follower.leader == index).count();
            limits.push(if row > 0 {
                format!("the row of {name} ({} parts) is {:.1} x {:.1} mm", row + 1, size[0], size[1])
            } else {
                format!("{name} is {:.1} x {:.1} mm", size[0], size[1])
            });
        }
        if !fits(smallest) {
            let grown = options
                .iter()
                .map(|size| [smallest[0].max(size[0]), smallest[1].max(size[1])])
                .min_by(|a, b| (a[0] * a[1]).total_cmp(&(b[0] * b[1])))
                .unwrap_or(smallest);
            smallest = grown;
        }
    }
    // Every edge's parts in single file along it.
    for need in edge_needs(problem) {
        let (axis, current) = match need.edge {
            Edge::Top | Edge::Bottom => (0, outline[0]),
            Edge::Left | Edge::Right => (1, outline[1]),
        };
        let length = need.length + 2.0 * margin;
        if length > current + 1.0e-6 {
            limits.push(format!(
                "the parts held at the {} edge on the {} ({}) need {:.1} mm along it",
                edge_name(need.edge),
                need.side,
                need.parts.iter().map(|part| references.get(*part).map_or("?", String::as_str)).collect::<Vec<_>>().join(", "),
                need.length
            ));
        }
        smallest[axis] = smallest[axis].max(length);
    }
    let smallest = smallest.map(round_up);
    let suggested = wanting.then(|| {
        let larger = [outline[0].max(smallest[0]), outline[1].max(smallest[1])];
        if larger[0] > outline[0] + 0.25 || larger[1] > outline[1] + 0.25 {
            larger
        } else {
            // Area is not what is missing (a part held somewhere crowded):
            // a quarter more area, as the layout's automatic sizing grows.
            outline.map(|side| round_up(side * 1.25f64.sqrt()))
        }
    });
    KiCadRoom {
        outline_mm: outline.map(|side| (side * 100.0).round() / 100.0),
        usable_mm2: usable.map(|area| (area * 10.0).round() / 10.0),
        parts_mm2: parts.map(|area| (area * 10.0).round() / 10.0),
        utilization,
        target_utilization: TARGET_UTILIZATION,
        missing_mm2: missing,
        smallest_outline_mm: smallest,
        suggested_outline_mm: suggested,
        limits,
    }
}

/// The outline tests of the constraints, after they were applied: a
/// user's row that fits the outline in no orientation it may take, and
/// parts held at an edge that need more length than it has (`edge_needs`).
/// Errors name what does not fit and by how much.
pub fn outline_holds(problem: &core::Problem, references: &[String], rows: &[usize]) -> Vec<String> {
    let mut errors = Vec::new();
    if problem.outline.len() < 3 {
        return errors;
    }
    let bounds = problem.bounds();
    let outline = [bounds[2] - bounds[0], bounds[3] - bounds[1]];
    for leader in rows {
        let component = &problem.components[*leader];
        // A row held at an edge needs only its copper on the board at the
        // last level, and an overhanging one reaches beyond it.
        let constraints = &problem.constraints;
        if constraints.edges.iter().any(|(part, _, _)| part == leader) || constraints.overhang(*leader).is_some() {
            continue;
        }
        let options = sizes(component, false);
        if !options.iter().any(|size| size[0] <= outline[0] + 1.0e-6 && size[1] <= outline[1] + 1.0e-6) {
            let members = 1 + problem.constraints.followers.iter().filter(|follower| follower.leader == *leader).count();
            errors.push(format!(
                "the row of {} ({members} parts) cannot fit the outline: the row is {:.2} x {:.2} mm (turned as it may be), the outline {:.2} x {:.2} mm: give the board more room (room.suggested_outline_mm in verdict.json) or the row a smaller pitch or more lines",
                references[*leader], component.body_size[0], component.body_size[1], outline[0], outline[1]
            ));
        }
    }
    for need in edge_needs(problem) {
        let length = match need.edge {
            Edge::Top | Edge::Bottom => outline[0],
            Edge::Left | Edge::Right => outline[1],
        };
        if need.length > length + 1.0e-6 {
            errors.push(format!(
                "the parts held at the {} edge on the {} ({}) cannot fit it: they need {:.2} mm along it in single file, the edge is {:.2} mm: a larger outline, a larger max_mm, or some of them on another edge",
                edge_name(need.edge),
                need.side,
                need.parts.iter().map(|part| references[*part].as_str()).collect::<Vec<_>>().join(", "),
                need.length,
                length
            ));
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(size: [f64; 2]) -> core::Component {
        core::Component {
            name: String::new(),
            body_center: [0.0, 0.0],
            body_size: size,
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side: core::Side::Front,
            fixed: false,
            angle_options: vec![0.0, 90.0, 180.0, 270.0],
            far_side: Vec::new(),
            far_copper: Vec::new(),
            far_courtyard: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: vec![[-size[0] / 2.0, -size[1] / 2.0, size[0] / 2.0, size[1] / 2.0]],
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
        }
    }

    fn board(width: f64, height: f64, components: Vec<core::Component>) -> core::Problem {
        let count = components.len();
        core::Problem {
            outline: vec![[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]],
            components,
            net_weights: Vec::new(),
            poses: vec![core::Pose { position: [0.0, 0.0], angle: 0.0 }; count],
            spacing: 0.0,
            grid: 0.5,
            edge_margin: 0.0,
            min_spacing: 0.0,
            far_side_pads_only: false,
            shared_halos: false,
            pieces: Vec::new(),
            constraints: Default::default(),
        }
    }

    #[test]
    fn room_says_how_much_area_is_missing_and_suggests_an_outline() {
        // Ten 10 x 10 mm parts (1000 mm2) on 40 x 20 mm (800 mm2): at 0.7
        // they need 1428.6 mm2, 628.6 more.
        let problem = board(40.0, 20.0, (0..10).map(|_| part([10.0, 10.0])).collect());
        let references: Vec<String> = (1..=10).map(|n| format!("U{n}")).collect();
        let room = room(&problem, &references, true);
        assert_eq!(room.outline_mm, [40.0, 20.0]);
        assert_eq!(room.parts_mm2, [1000.0, 0.0]);
        assert_eq!(room.utilization, [1.25, 0.0]);
        assert_eq!(room.missing_mm2, [628.6, 0.0]);
        // By area at the aspect 2: 2 h^2 = 1428.6, h = 26.7, w = 53.5. But
        // two strips of 10 mm hold the parts only 50 mm long, 59.8 at the
        // target (as full as 0.7 in area: 0.837 in length).
        assert_eq!(room.smallest_outline_mm, [60.5, 27.0]);
        assert_eq!(room.suggested_outline_mm, Some([60.5, 27.0]));
        // Nothing wanting, nothing suggested.
        assert_eq!(super::room(&problem, &references, false).suggested_outline_mm, None);
    }

    #[test]
    fn a_part_larger_than_the_board_bounds_the_outline() {
        let mut long = part([50.0, 4.0]);
        long.angle_options = vec![0.0, 180.0];
        let problem = board(40.0, 40.0, vec![long]);
        let room = room(&problem, &["J1".to_string()], true);
        // 50 mm long, 59.8 at the target; the height as the area says.
        assert_eq!(room.smallest_outline_mm, [60.0, 17.0]);
        assert_eq!(room.suggested_outline_mm, Some([60.0, 40.0]));
        assert_eq!(room.limits, vec!["J1 is 50.0 x 4.0 mm".to_string()]);
    }

    #[test]
    fn parts_held_at_one_edge_need_its_length() {
        // Three 15 x 6 mm connectors flush on the top edge of a 40 mm
        // board need 3 x 6 = 18 mm, turned (their copper fills their
        // bodies: none may overhang the edge's ends).
        let mut problem = board(40.0, 30.0, (0..3).map(|_| part([15.0, 6.0])).collect());
        for index in 0..3 {
            problem.constraints.edges.push((index, Edge::Top, 1.0e-3));
        }
        let references: Vec<String> = (1..=3).map(|n| format!("J{n}")).collect();
        let needs = edge_needs(&problem);
        assert_eq!(needs.len(), 1);
        assert!((needs[0].length - 18.0).abs() < 1.0e-9);
        assert!(outline_holds(&problem, &references, &[]).is_empty());
        // Seven need 42 mm of a 40 mm edge.
        let mut problem = board(40.0, 30.0, (0..7).map(|_| part([15.0, 6.0])).collect());
        for index in 0..7 {
            problem.constraints.edges.push((index, Edge::Top, 1.0e-3));
        }
        let references: Vec<String> = (1..=7).map(|n| format!("J{n}")).collect();
        let errors = outline_holds(&problem, &references, &[]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("need 42.00 mm along it"), "{errors:?}");
        // With a reach deeper than a part, two may stand one behind the
        // other: no quick answer.
        for edge in &mut problem.constraints.edges {
            edge.2 = 10.0;
        }
        assert!(outline_holds(&problem, &references, &[]).is_empty());
    }
}

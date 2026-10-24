// Copyright (C) 2026 Toit contributors.

//! Placement ablation (docs/placer.md "Placement ablation (2026-10-24)"):
//! which of our placement's decisions cost routability. From the designer's
//! board and our placed board of the same project it writes hybrids: the
//! designer's placement with one group of parts where we put them, and our
//! placement with that group where the designer put it. Groups: each large
//! footprint (more than `large_pads` pads) alone, the same with its
//! passives within `passive_mm`, the connectors, and the board's quadrants.
//!
//! Every hybrid starts from our placed board (the task's preparation: its
//! tracks stripped, its pours and rule areas, its labels), so the variants
//! differ in the footprints' poses alone. A moved group lands on parts of
//! the other placement; with `repair` the parts whose bodies it lands on
//! move to the nearest spot where their body meets no other (`repair`),
//! everything else staying. The report lists them.
//!
//! Beside the boards, the signals a placer could have read before routing,
//! per variant: the wirelength, escape demand's channel overflow
//! (`pcb_placer::escape`, the placer's own estimate), and the global
//! route's overflow and soft overflow at 0.5 (`pcb_global`, as the layout
//! lowers a placement); and per part of the two placements what its
//! escape looks like (demand per side, the part it faces and the channel
//! between them, the board edge, its connections' distance).

use super::*;
use crate::board_layout::footprint_items;
use crate::board_placer::{
    KiCadBoardPlacerConfig, attribution, courtyards_may_overlap, holes_in_courtyards_error,
    largest_clearance, lower_placement, reference_index, write_footprint_pose,
};
use crate::placement_constraints::{KiCadConstraintsSource, resolve_constraints};
use pcb_placer as placer;
use pcb_placer::escape;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadAblationConfig {
    /// A footprint with more pads than this is large: a group of its own.
    pub large_pads: usize,
    /// Parts of at most this many pads count as passives.
    pub passive_pads: usize,
    /// A passive sharing a net with a large part, its body centre within
    /// this distance of the large part's body (in either placement), moves
    /// with it in the large part's cluster group.
    pub passive_mm: f64,
    /// Move the parts a group lands on to the nearest legal spot.
    pub repair: bool,
    /// Only these groups (by name; empty: all).
    pub groups: Vec<String>,
    /// Groups of one's own, by name (`extra-<name>`): the parts by
    /// reference (two parts that trade places belong in one group).
    pub extra: BTreeMap<String, Vec<String>>,
    /// The global route's excess threshold (share of a boundary's
    /// capacity) for the per-part attribution.
    pub threshold: f64,
    /// Route every variant globally (seconds per variant on a large board).
    pub global_route: bool,
    /// Null variants per placement (`d-null-1`, `o-null-1`, ...): one
    /// passive nudged by `null_nudge_mm` where that is legal, nothing else
    /// changed. Their routes show how much a route moves from a change
    /// that should cost nothing: the noise floor of an attribution.
    pub nulls: usize,
    pub null_nudge_mm: f64,
    /// Replicates of every variant written (`<variant>-r1`, ...), the
    /// baselines' among them: each with one more passive nudged by
    /// `null_nudge_mm` (another one per replicate, away from the moved
    /// and displaced parts), so that a group's cost is a mean over routes
    /// that differ by noise alone.
    pub replicates: usize,
}

impl Default for KiCadAblationConfig {
    fn default() -> Self {
        Self {
            large_pads: 20,
            passive_pads: 4,
            passive_mm: 3.0,
            repair: true,
            groups: Vec::new(),
            extra: BTreeMap::new(),
            threshold: 0.5,
            global_route: true,
            nulls: 2,
            null_nudge_mm: 0.1,
            replicates: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadAblationGroup {
    pub name: String,
    /// `large`, `cluster`, `connectors` or `quadrant`.
    pub kind: String,
    /// The large part of a `large` or `cluster` group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    pub members: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadDisplaced {
    pub reference: String,
    pub distance_mm: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadGlobalSignals {
    pub total_overflow: f64,
    pub soft_overflow_05: f64,
    pub overflowed_nets: usize,
    pub detour_fraction: f64,
    /// The excess at `threshold` summed over the tiles.
    pub excess: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadPlacementSignals {
    /// Half-perimeter wirelength over the nets, mm (unweighted).
    pub wirelength_mm: f64,
    /// Escape demand's channel overflow summed over the channels, tracks.
    pub channel_overflow: f64,
    /// Channels with overflow.
    pub channels_over: usize,
    /// Exits that face another part's body within a channel's reach
    /// (`escape::CHANNEL_REACH`), summed over the parts.
    pub blocked_exits: usize,
    /// Parts illegal in the placer's tightest model.
    pub illegal: usize,
    /// Pairs of parts that collide (`conflict`: pads within 0.1 mm, or one
    /// body mostly on another): a hybrid's left-over collisions beside the
    /// placement's own.
    pub overlaps: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<KiCadGlobalSignals>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadAblationVariant {
    pub name: String,
    /// `designer` or `ours`: the placement the variant starts from.
    pub base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub moved: usize,
    pub displaced: Vec<KiCadDisplaced>,
    /// Displaced parts without a legal spot (left where they were).
    pub unplaced: Vec<String>,
    pub signals: KiCadPlacementSignals,
    /// Where the variant's project was written.
    pub directory: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadSideEscape {
    /// Nets leaving through this side.
    pub demand: usize,
    /// Distance from the body to the board's bounding box on this side.
    pub edge_mm: f64,
    /// The nearest body this side faces within a channel's reach, its
    /// gap, the channel's capacity in tracks and its overflow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub faces: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub faced_demand: Option<usize>,
    pub overflow: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPartEscape {
    pub reference: String,
    pub pads: usize,
    pub side: String,
    pub at: [f64; 3],
    pub body_half: [f64; 2],
    /// Sides -x, +x, -y, +y of the body box at its pose.
    pub sides: Vec<KiCadSideEscape>,
    /// Mean, over its counted nets, of the distance from its pin to the
    /// net's nearest pin on another part, mm.
    pub connection_mm: f64,
    /// The channel overflow attributed to it, tracks.
    pub channel_overflow: f64,
    /// The global route's excess on its nets (`threshold`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_attribution: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadAblationReport {
    pub board_id: String,
    pub footprints: usize,
    pub movable: usize,
    /// Footprints at the same pose in both placements (fixed by the task).
    pub same_pose: usize,
    /// Footprints on another side in our placement.
    pub flipped: Vec<String>,
    pub track_pitch_mm: f64,
    pub routing_layers: f64,
    pub groups: Vec<KiCadAblationGroup>,
    pub variants: Vec<KiCadAblationVariant>,
    /// Every part that moves between the two placements, in both.
    pub parts_designer: Vec<KiCadPartEscape>,
    pub parts_ours: Vec<KiCadPartEscape>,
}

/// One footprint of the base board: its index, reference, pads, and the
/// designer's and our poses and sides.
struct Part {
    reference: String,
    pads: usize,
    connector: bool,
    designer: [f64; 3],
    designer_back: bool,
    ours_back: bool,
}

fn footprint_uuid(footprint: &Expr) -> Option<String> {
    form_atom(footprint, "uuid", 1).map(str::to_string)
}

fn footprint_back(footprint: &Expr) -> bool {
    form_atom(footprint, "layer", 1) == Some("B.Cu")
}

fn footprint_library(footprint: &Expr) -> String {
    footprint
        .children()
        .get(1)
        .and_then(Expr::atom)
        .unwrap_or_default()
        .to_string()
}

fn is_connector(reference: &str, library: &str) -> bool {
    let prefix: String = reference
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let library = library.to_ascii_lowercase();
    matches!(prefix.as_str(), "J" | "P" | "CN" | "CON" | "USB" | "X")
        && !library.contains("crystal")
        || library.starts_with("connector")
        || library.contains(":connector")
}

/// The placement problem the hybrids are judged in: the base board lowered
/// as the placer lowers it (the project's rules, the task's constraints),
/// then at its tightest (no halos, the smallest spacing), with escape
/// demand's fact.
fn ablation_problem(
    pcb: &Expr,
    directory: &Path,
    board_id: &str,
) -> Result<(crate::board_placer::LoweredPlacement, escape::Escape), String> {
    let mut config = KiCadBoardPlacerConfig::default();
    if directory.join("constraints.json").exists() {
        config.constraints = Some(KiCadConstraintsSource::File("constraints.json".into()));
    }
    let mut config = resolve_constraints(&config, directory)?;
    let project = directory.join(format!("{board_id}.kicad_pro"));
    let rules = project_rules::resolve_project_rules(
        &project,
        &directory.join(format!("{board_id}.kicad_pcb")),
    )
    .ok();
    config.copper_clearance_mm = rules.as_ref().map(largest_clearance);
    config.copper_edge_clearance_mm = rules.as_ref().map(|rules| rules.edge_clearance_mm);
    config.hole_clearance_mm = rules.as_ref().map(|rules| rules.hole_clearance_mm);
    config.solder_mask_to_copper_mm = rules
        .as_ref()
        .and_then(|rules| rules.solder_mask_to_copper_mm);
    config.tight_bodies = Some(courtyards_may_overlap(&project));
    config.holes_in_courtyards_error = Some(holes_in_courtyards_error(&project));
    let mut lowered = lower_placement(pcb, &config)?;
    lowered.problem.spacing = lowered.problem.min_spacing;
    for component in &mut lowered.problem.components {
        component.halo = 0.0;
    }
    let mut pins = vec![0usize; lowered.problem.net_weights.len()];
    for component in &lowered.problem.components {
        for pin in &component.pins {
            pins[pin.net] += 1;
        }
    }
    let fact = escape::Escape {
        pitch: config.track_pitch_mm,
        layers: lowered.routing_layers.max(1.0),
        counted: pins
            .iter()
            .zip(&lowered.poured)
            .map(|(pins, poured)| *pins >= 2 && !poured)
            .collect(),
        weight: 2.0,
        inflate: 1.0,
        turn_tolerance: 0.05,
        measured: None,
        measure_every: 20,
    };
    Ok((lowered, fact))
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Distance from `point` to the axis-aligned box (centre, half extent).
fn box_distance(point: [f64; 2], center: [f64; 2], half: [f64; 2]) -> f64 {
    let dx = ((point[0] - center[0]).abs() - half[0]).max(0.0);
    let dy = ((point[1] - center[1]).abs() - half[1]).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// Escape demand's exits of every part at `poses`.
fn all_exits(
    problem: &placer::Problem,
    fact: &escape::Escape,
    members: &[escape::NetPins],
    poses: &[placer::Pose],
    count: usize,
) -> Vec<Vec<escape::Exit>> {
    (0..count)
        .map(|index| {
            let mut out = Vec::new();
            escape::exits(problem, fact, members, poses, index, &mut out);
            out
        })
        .collect()
}

/// Per part the channel overflow attributed to it, the channels with
/// overflow, and the total.
fn channel_overflow(
    problem: &placer::Problem,
    fact: &escape::Escape,
    poses: &[placer::Pose],
    exits: &[Vec<escape::Exit>],
) -> (Vec<f64>, usize, f64) {
    let count = exits.len();
    let mut per = vec![0.0; count];
    let mut channels = 0;
    let mut total = 0.0;
    let boxes: Vec<_> = (0..count)
        .map(|index| escape::body_box(problem, index, poses[index]))
        .collect();
    for a in 0..count {
        if exits[a].is_empty() {
            continue;
        }
        for b in a + 1..count {
            if exits[b].is_empty() {
                continue;
            }
            let ((ca, ha), (cb, hb)) = (boxes[a], boxes[b]);
            let gx = (cb[0] - ca[0]).abs() - ha[0] - hb[0];
            let gy = (cb[1] - ca[1]).abs() - ha[1] - hb[1];
            if gx > escape::CHANNEL_REACH || gy > escape::CHANNEL_REACH {
                continue;
            }
            let over = escape::channel(
                problem, fact, a, poses[a], &exits[a], b, poses[b], &exits[b],
            );
            if over > 0.0 {
                per[a] += over;
                per[b] += over;
                channels += 1;
                total += over;
            }
        }
    }
    (per, channels, total)
}

/// Per side of part `index`, the nearest body it faces within a channel's
/// reach: (part, gap).
fn facing(
    problem: &placer::Problem,
    poses: &[placer::Pose],
    count: usize,
    index: usize,
) -> [Option<(usize, f64)>; escape::SIDES] {
    let mut best: [Option<(usize, f64)>; escape::SIDES] = [None; escape::SIDES];
    let (center, half) = escape::body_box(problem, index, poses[index]);
    let side_of = problem.components[index].side;
    for (other, pose) in poses.iter().enumerate().take(count) {
        if other == index || !side_of.collides(problem.components[other].side) {
            continue;
        }
        let (oc, oh) = escape::body_box(problem, other, *pose);
        if oh[0] <= 0.0 && oh[1] <= 0.0 {
            continue;
        }
        let gx = (oc[0] - center[0]).abs() - half[0] - oh[0];
        let gy = (oc[1] - center[1]).abs() - half[1] - oh[1];
        let (side, gap) = if gx >= 0.0 && gy < 0.0 {
            (usize::from(oc[0] > center[0]), gx)
        } else if gy >= 0.0 && gx < 0.0 {
            (2 + usize::from(oc[1] > center[1]), gy)
        } else {
            continue;
        };
        if gap > escape::CHANNEL_REACH {
            continue;
        }
        if best[side].is_none_or(|(_, known)| gap < known) {
            best[side] = Some((other, gap));
        }
    }
    best
}

/// A placement's signals, its parts' exits, their channel overflow and
/// their global route attribution (when routed).
type Measured = (
    KiCadPlacementSignals,
    Vec<Vec<escape::Exit>>,
    Vec<f64>,
    Option<Vec<f64>>,
);

/// The signals of one placement.
#[allow(clippy::too_many_arguments)]
fn signals(
    problem: &placer::Problem,
    fact: &escape::Escape,
    members: &[escape::NetPins],
    poses: &[placer::Pose],
    count: usize,
    pcb: Option<&Expr>,
    router_config: &KiCadBoardRouterConfig,
    references: &[String],
    threshold: f64,
) -> Result<Measured, String> {
    let exits = all_exits(problem, fact, members, poses, count);
    let (per, channels_over, channel_total) = channel_overflow(problem, fact, poses, &exits);
    let mut blocked = 0;
    for (index, part_exits) in exits.iter().enumerate().take(count) {
        let faces = facing(problem, poses, count, index);
        for exit in part_exits {
            if faces[exit.side].is_some() {
                blocked += 1;
            }
        }
    }
    let mut boxes = vec![
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY
        ];
        problem.net_weights.len()
    ];
    for (component, pose) in problem.components.iter().zip(poses).take(count) {
        for pin in &component.pins {
            let at = component.pin_position(pin, *pose);
            let bounds = &mut boxes[pin.net];
            bounds[0] = bounds[0].min(at[0]);
            bounds[1] = bounds[1].min(at[1]);
            bounds[2] = bounds[2].max(at[0]);
            bounds[3] = bounds[3].max(at[1]);
        }
    }
    let wirelength: f64 = boxes
        .iter()
        .filter(|bounds| bounds[0].is_finite())
        .map(|bounds| (bounds[2] - bounds[0]) + (bounds[3] - bounds[1]))
        .sum();
    let illegal = placer::legal::illegal_components(problem, poses)
        .into_iter()
        .filter(|index| *index < count && !problem.components[*index].fixed)
        .count();
    let mut overlaps = 0;
    for a in 0..count {
        for b in a + 1..count {
            if conflict(problem, a, poses[a], b, poses[b]) {
                overlaps += 1;
            }
        }
    }
    let mut global = None;
    let mut global_parts = None;
    if let Some(pcb) = pcb {
        let board = crate::global_route::lower_placed(pcb, router_config)?;
        let route = pcb_global::route(&board, &pcb_global::GlobalConfig::default());
        let index = reference_index(references);
        let (parts, tiles) = attribution(&board, &route, &index, references.len(), threshold);
        let soft = pcb_global::SOFT_ALPHAS
            .iter()
            .position(|alpha| (*alpha - 0.5).abs() < 1.0e-9)
            .and_then(|at| route.score.soft_overflow.get(at).copied())
            .unwrap_or(0.0);
        global = Some(KiCadGlobalSignals {
            total_overflow: route.score.total_overflow,
            soft_overflow_05: soft,
            overflowed_nets: route.score.overflowed_nets,
            detour_fraction: route.score.detour_fraction,
            excess: tiles.iter().sum(),
        });
        global_parts = Some(parts);
    }
    Ok((
        KiCadPlacementSignals {
            wirelength_mm: wirelength,
            channel_overflow: channel_total,
            channels_over,
            blocked_exits: blocked,
            illegal,
            overlaps,
            global,
        },
        exits,
        per,
        global_parts,
    ))
}

/// What one part's escape looks like at `poses`.
#[allow(clippy::too_many_arguments)]
fn part_escape(
    problem: &placer::Problem,
    fact: &escape::Escape,
    members: &[escape::NetPins],
    poses: &[placer::Pose],
    count: usize,
    exits: &[Vec<escape::Exit>],
    per: &[f64],
    global: Option<&Vec<f64>>,
    parts: &[Part],
    references: &[String],
    index: usize,
) -> KiCadPartEscape {
    let component = &problem.components[index];
    let pose = poses[index];
    let (center, half) = escape::body_box(problem, index, pose);
    let bounds = problem.bounds();
    let edges = [
        center[0] - half[0] - bounds[0],
        bounds[2] - center[0] - half[0],
        center[1] - half[1] - bounds[1],
        bounds[3] - center[1] - half[1],
    ];
    let demand = escape::demand(&exits[index]);
    let faces = facing(problem, poses, count, index);
    let sides = (0..escape::SIDES)
        .map(|side| {
            let mut entry = KiCadSideEscape {
                demand: demand[side],
                edge_mm: edges[side],
                faces: None,
                gap_mm: None,
                capacity: None,
                faced_demand: None,
                overflow: 0.0,
            };
            if let Some((other, gap)) = faces[side] {
                let opposite = side ^ 1;
                entry.faces = Some(references[other].clone());
                entry.gap_mm = Some(gap);
                entry.capacity = Some(gap.max(0.0) / fact.track_room());
                entry.faced_demand = Some(escape::demand(&exits[other])[opposite]);
                entry.overflow = escape::channel(
                    problem,
                    fact,
                    index,
                    pose,
                    &exits[index],
                    other,
                    poses[other],
                    &exits[other],
                );
            }
            entry
        })
        .collect();
    let mut sum = 0.0;
    let mut nets = 0usize;
    let mut seen = BTreeSet::new();
    for pin in &component.pins {
        if !fact.counted.get(pin.net).copied().unwrap_or(false) || !seen.insert(pin.net) {
            continue;
        }
        let at = component.pin_position(pin, pose);
        let nearest = members[pin.net]
            .iter()
            .filter(|(other, _)| *other != index)
            .map(|(other, other_pin)| {
                let part = &problem.components[*other];
                distance(at, part.pin_position(&part.pins[*other_pin], poses[*other]))
            })
            .fold(f64::INFINITY, f64::min);
        if nearest.is_finite() {
            sum += nearest;
            nets += 1;
        }
    }
    let side = if parts[index].ours_back {
        "back"
    } else {
        "front"
    };
    KiCadPartEscape {
        reference: references[index].clone(),
        pads: parts[index].pads,
        side: side.into(),
        at: [pose.position[0], pose.position[1], pose.angle],
        body_half: half,
        sides,
        connection_mm: if nets > 0 { sum / nets as f64 } else { 0.0 },
        channel_overflow: per[index],
        global_attribution: global.map(|parts| parts[index]),
    }
}

/// Pads of two parts closer than this (mm) conflict: copper that would
/// short or leave no clearance.
const PAD_GAP_MM: f64 = 0.1;
/// Bodies overlapping by more than this share of the smaller body's area
/// conflict: one part on top of another. Less is the courtyard overlap
/// designers allow (parts under a socket, courtyards that touch), which
/// leaves the copper routable.
const BODY_SHARE: f64 = 0.25;

/// Whether parts `a` and `b` at their poses collide on a side both occupy:
/// their pads come within `PAD_GAP_MM`, or their body boxes overlap by
/// more than `BODY_SHARE` of the smaller one.
fn conflict(
    problem: &placer::Problem,
    a: usize,
    pose_a: placer::Pose,
    b: usize,
    pose_b: placer::Pose,
) -> bool {
    let (first, second) = (&problem.components[a], &problem.components[b]);
    if !first.side.collides(second.side) {
        return false;
    }
    let (ca, ha) = escape::body_box(problem, a, pose_a);
    let (cb, hb) = escape::body_box(problem, b, pose_b);
    let reach = 1.0;
    let ox = ha[0] + hb[0] - (ca[0] - cb[0]).abs();
    let oy = ha[1] + hb[1] - (ca[1] - cb[1]).abs();
    if ox <= -reach || oy <= -reach {
        return false;
    }
    if ox > 0.0 && oy > 0.0 {
        let smaller = (4.0 * ha[0] * ha[1]).min(4.0 * hb[0] * hb[1]);
        if ox.min(2.0 * ha[0].min(hb[0])) * oy.min(2.0 * ha[1].min(hb[1])) > BODY_SHARE * smaller {
            return true;
        }
    }
    let pads_b = second.pad_boxes(pose_b);
    first.pad_boxes(pose_a).iter().any(|(pa, sa)| {
        pads_b.iter().any(|(pb, sb)| {
            (pa[0] - pb[0]).abs() - sa[0] - sb[0] < PAD_GAP_MM
                && (pa[1] - pb[1]).abs() - sa[1] - sb[1] < PAD_GAP_MM
        })
    })
}

/// Moves the parts whose bodies the moved `group` lands on (by `conflict`,
/// at `poses`) to the nearest spot, at their own angle, where their body
/// lies inside the outline and conflicts with no other part; everything
/// else stays.
/// Larger parts first. The placer's own legality is not the test: at its
/// tightest it still finds most of a designer's parts illegal (courtyards
/// that touch, parts under parts). Returns the displaced parts with their
/// distance and those left without a spot (where they were).
fn repair(
    problem: &placer::Problem,
    poses: &mut [placer::Pose],
    group: &BTreeSet<usize>,
    count: usize,
) -> (Vec<(usize, f64)>, Vec<usize>) {
    let total = problem.components.len();
    let mut displaced: Vec<usize> = (0..count)
        .filter(|index| {
            !group.contains(index)
                && !problem.components[*index].fixed
                && group
                    .iter()
                    .any(|member| conflict(problem, *index, poses[*index], *member, poses[*member]))
        })
        .collect();
    displaced.sort_by(|a, b| {
        let area = |index: usize| {
            let size = problem.components[index].body_size;
            size[0] * size[1]
        };
        area(*b).total_cmp(&area(*a)).then(a.cmp(b))
    });
    let pending: BTreeSet<usize> = displaced.iter().copied().collect();
    let mut placed_again: BTreeSet<usize> = BTreeSet::new();
    let mut moved = Vec::new();
    let mut unplaced = Vec::new();
    let bounds = problem.bounds();
    let step = 0.25;
    let rings = (((bounds[2] - bounds[0]).max(bounds[3] - bounds[1])) / step).ceil() as i64;
    for &index in &displaced {
        let start = poses[index];
        let free = |pose: placer::Pose| {
            placer::legal::body_inside_outline(problem, index, pose)
                && (0..total).all(|other| {
                    other == index
                        || (pending.contains(&other) && !placed_again.contains(&other))
                        || !conflict(problem, index, pose, other, poses[other])
                })
        };
        let mut found = None;
        'search: for ring in 1..=rings {
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    if dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let pose = placer::Pose {
                        position: [
                            start.position[0] + dx as f64 * step,
                            start.position[1] + dy as f64 * step,
                        ],
                        angle: start.angle,
                    };
                    if free(pose) {
                        let better = found.is_none_or(|(_, known): (placer::Pose, f64)| {
                            distance(pose.position, start.position) < known
                        });
                        if better {
                            found = Some((pose, distance(pose.position, start.position)));
                        }
                    }
                }
            }
            // A ring holds spots up to sqrt(2) times its radius: stop once
            // no farther ring can hold a nearer one.
            if let Some((_, known)) = found
                && known <= ring as f64 * step
            {
                break 'search;
            }
        }
        match found {
            Some((pose, moved_by)) => {
                poses[index] = pose;
                moved.push((index, moved_by));
            }
            None => unplaced.push(index),
        }
        placed_again.insert(index);
    }
    (moved, unplaced)
}

/// Nudges one passive of `poses` by `nudge` mm (the first direction where
/// it meets no part it did not meet before), the `slot`-th of `slots`
/// spread over the candidates (movable passives outside `avoid`), and
/// returns it; None when no candidate can move.
#[allow(clippy::too_many_arguments)]
fn nudge_passive(
    problem: &placer::Problem,
    poses: &mut [placer::Pose],
    movable: &[usize],
    parts: &[Part],
    passive_pads: usize,
    avoid: &BTreeSet<usize>,
    slot: usize,
    slots: usize,
    nudge: f64,
) -> Option<usize> {
    let candidates: Vec<usize> = movable
        .iter()
        .copied()
        .filter(|index| {
            (2..=passive_pads).contains(&parts[*index].pads)
                && !parts[*index].connector
                && !avoid.contains(index)
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let total = problem.components.len();
    let start = (slot * candidates.len() / (slots + 1) + candidates.len() / (2 * (slots + 1)))
        % candidates.len();
    for offset in 0..candidates.len() {
        let index = candidates[(start + offset) % candidates.len()];
        let before: Vec<bool> = (0..total)
            .map(|other| {
                other != index && conflict(problem, index, poses[index], other, poses[other])
            })
            .collect();
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let pose = placer::Pose {
                position: [
                    poses[index].position[0] + dx * nudge,
                    poses[index].position[1] + dy * nudge,
                ],
                angle: poses[index].angle,
            };
            let clean = placer::legal::body_inside_outline(problem, index, pose)
                && (0..total).all(|other| {
                    other == index
                        || before[other]
                        || !conflict(problem, index, pose, other, poses[other])
                });
            if clean {
                poses[index] = pose;
                return Some(index);
            }
        }
    }
    None
}

/// `pcb` (our placed board) with its footprints at `poses` (those whose
/// pose differs from ours), flipped where the side differs.
fn board_at(
    pcb: &Expr,
    poses: &[placer::Pose],
    ours: &[placer::Pose],
    backs: &[bool],
    parts: &[Part],
) -> Result<Expr, String> {
    let mut board = pcb.clone();
    let mut footprints = footprint_items(&mut board)?;
    for (index, footprint) in footprints.iter_mut().enumerate() {
        if backs[index] != parts[index].ours_back {
            crate::flip::flip_footprint(footprint)?;
        }
        if poses[index] != ours[index] || backs[index] != parts[index].ours_back {
            write_footprint_pose(footprint, poses[index])?;
        }
    }
    Ok(board)
}

/// Writes the hybrids of `designer_board` and our placed board
/// `<ours_directory>/<board_id>.kicad_pcb` into
/// `<output_directory>/<variant>/` (each a copy of our directory with its
/// board replaced) and reports the groups, the variants and their signals.
pub fn ablate_kicad_placement(
    designer_board: &Path,
    ours_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadAblationConfig,
    router_config: &KiCadBoardRouterConfig,
) -> Result<KiCadAblationReport, String> {
    let ours_path = ours_directory.join(format!("{board_id}.kicad_pcb"));
    let ours_text = fs::read_to_string(&ours_path)
        .map_err(|error| format!("failed to read {}: {error}", ours_path.display()))?;
    let pcb = crate::canonical_order::parse_board(&ours_text)?;
    let designer_text = fs::read_to_string(designer_board)
        .map_err(|error| format!("failed to read {}: {error}", designer_board.display()))?;
    let designer_pcb = crate::canonical_order::parse_board(&designer_text)?;
    // The designer's footprints by uuid, and by reference in order.
    let mut by_uuid: BTreeMap<String, ([f64; 3], bool)> = BTreeMap::new();
    let mut by_reference: BTreeMap<String, Vec<([f64; 3], bool)>> = BTreeMap::new();
    for footprint in designer_pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let entry = (form_at(footprint)?, footprint_back(footprint));
        if let Some(uuid) = footprint_uuid(footprint) {
            by_uuid.insert(uuid, entry);
        }
        by_reference
            .entry(footprint_reference(footprint).unwrap_or_default())
            .or_default()
            .push(entry);
    }
    let mut parts = Vec::new();
    let mut nth: BTreeMap<String, usize> = BTreeMap::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let seen = nth.entry(reference.clone()).or_default();
        let found = footprint_uuid(footprint)
            .and_then(|uuid| by_uuid.get(&uuid).copied())
            .or_else(|| {
                by_reference
                    .get(&reference)
                    .and_then(|entries| entries.get(*seen).copied())
            });
        *seen += 1;
        let ours_at = form_at(footprint)?;
        let (designer, designer_back) = found.unwrap_or((ours_at, footprint_back(footprint)));
        let pads = footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
            .count();
        parts.push(Part {
            connector: is_connector(&reference, &footprint_library(footprint)),
            reference,
            pads,
            designer,
            designer_back,
            ours_back: footprint_back(footprint),
        });
    }
    let count = parts.len();
    let (lowered, fact) = ablation_problem(&pcb, ours_directory, board_id)?;
    let problem = &lowered.problem;
    let references = &lowered.references;
    let members = escape::members(problem);
    let ours: Vec<placer::Pose> = problem.poses.clone();
    let mut designer = ours.clone();
    for (index, part) in parts.iter().enumerate() {
        designer[index] = placer::Pose {
            position: [part.designer[0], part.designer[1]],
            angle: part.designer[2],
        };
    }
    let designer_backs: Vec<bool> = parts.iter().map(|part| part.designer_back).collect();
    let ours_backs: Vec<bool> = parts.iter().map(|part| part.ours_back).collect();
    let flipped: Vec<String> = parts
        .iter()
        .filter(|part| part.designer_back != part.ours_back)
        .map(|part| part.reference.clone())
        .collect();
    let moves = |index: usize| {
        !problem.components[index].fixed
            && (distance(designer[index].position, ours[index].position) > 1.0e-3
                || (designer[index].angle - ours[index].angle).abs() > 1.0e-3
                || parts[index].designer_back != parts[index].ours_back)
    };
    let movable: Vec<usize> = (0..count).filter(|index| moves(*index)).collect();
    let same_pose = count - movable.len();

    // The groups.
    let mut groups: Vec<(KiCadAblationGroup, BTreeSet<usize>)> = Vec::new();
    let named = |indices: &BTreeSet<usize>| -> Vec<String> {
        indices
            .iter()
            .map(|index| parts[*index].reference.clone())
            .collect()
    };
    let mut large: Vec<usize> = movable
        .iter()
        .copied()
        .filter(|index| parts[*index].pads > config.large_pads)
        .collect();
    large.sort_by(|a, b| parts[*b].pads.cmp(&parts[*a].pads).then(a.cmp(b)));
    for &index in &large {
        let alone: BTreeSet<usize> = [index].into();
        groups.push((
            KiCadAblationGroup {
                name: format!("large-{}", parts[index].reference),
                kind: "large".into(),
                anchor: Some(parts[index].reference.clone()),
                members: named(&alone),
            },
            alone,
        ));
    }
    for &index in &large {
        let nets: BTreeSet<usize> = problem.components[index]
            .pins
            .iter()
            .map(|pin| pin.net)
            .collect();
        let mut cluster: BTreeSet<usize> = [index].into();
        for &other in &movable {
            if parts[other].pads > config.passive_pads
                || parts[other].pads == 0
                || parts[other].connector
                || !problem.components[other]
                    .pins
                    .iter()
                    .any(|pin| nets.contains(&pin.net))
            {
                continue;
            }
            let near = [&designer, &ours].iter().any(|poses| {
                let (center, half) = escape::body_box(problem, index, poses[index]);
                let (other_center, _) = escape::body_box(problem, other, poses[other]);
                box_distance(other_center, center, half) <= config.passive_mm
            });
            if near {
                cluster.insert(other);
            }
        }
        if cluster.len() > 1 {
            groups.push((
                KiCadAblationGroup {
                    name: format!("cluster-{}", parts[index].reference),
                    kind: "cluster".into(),
                    anchor: Some(parts[index].reference.clone()),
                    members: named(&cluster),
                },
                cluster,
            ));
        }
    }
    let connectors: BTreeSet<usize> = movable
        .iter()
        .copied()
        .filter(|index| parts[*index].connector)
        .collect();
    if !connectors.is_empty() {
        groups.push((
            KiCadAblationGroup {
                name: "connectors".into(),
                kind: "connectors".into(),
                anchor: None,
                members: named(&connectors),
            },
            connectors,
        ));
    }
    let bounds = problem.bounds();
    let middle = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
    // KiCad's y grows downwards: north is the low y.
    for (name, east, south) in [
        ("nw", false, false),
        ("ne", true, false),
        ("sw", false, true),
        ("se", true, true),
    ] {
        let quadrant: BTreeSet<usize> = movable
            .iter()
            .copied()
            .filter(|index| {
                let center = problem.components[*index].center(designer[*index]);
                (center[0] >= middle[0]) == east && (center[1] >= middle[1]) == south
            })
            .collect();
        if !quadrant.is_empty() {
            groups.push((
                KiCadAblationGroup {
                    name: format!("quadrant-{name}"),
                    kind: "quadrant".into(),
                    anchor: None,
                    members: named(&quadrant),
                },
                quadrant,
            ));
        }
    }
    for (name, references_wanted) in &config.extra {
        let mut members = BTreeSet::new();
        for wanted in references_wanted {
            let found: Vec<usize> = movable
                .iter()
                .copied()
                .filter(|index| &parts[*index].reference == wanted)
                .collect();
            if found.is_empty() {
                return Err(format!(
                    "extra group {name}: {wanted} is not a part that moves"
                ));
            }
            members.extend(found);
        }
        groups.push((
            KiCadAblationGroup {
                name: format!("extra-{name}"),
                kind: "extra".into(),
                anchor: None,
                members: named(&members),
            },
            members,
        ));
    }
    if !config.groups.is_empty() {
        groups.retain(|(group, _)| config.groups.contains(&group.name));
    }

    if output_directory.exists() {
        return Err(format!(
            "output directory {} already exists",
            output_directory.display()
        ));
    }
    fs::create_dir_all(output_directory)
        .map_err(|error| format!("failed to create {}: {error}", output_directory.display()))?;
    let write_variant = |name: &str, board: &Expr| -> Result<String, String> {
        let directory = output_directory.join(name);
        crate::fs_util::copy_directory_tree(ours_directory, &directory)?;
        // Our placement's own reports do not describe the hybrid.
        for stale in ["placement.html", "board-placer.json"] {
            let _ = fs::remove_file(directory.join(stale));
        }
        let path = directory.join(format!("{board_id}.kicad_pcb"));
        fs::write(&path, format!("{}\n", encode(board)))
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        Ok(directory.display().to_string())
    };

    let measure = |poses: &[placer::Pose], board: &Expr| {
        signals(
            problem,
            &fact,
            &members,
            poses,
            count,
            config.global_route.then_some(board),
            router_config,
            references,
            config.threshold,
        )
    };
    let mut variants = Vec::new();
    let mut described = Vec::new();
    for (name, poses, backs) in [
        ("designer", &designer, &designer_backs),
        ("ours", &ours, &ours_backs),
    ] {
        let board = board_at(&pcb, poses, &ours, backs, &parts)?;
        let directory = write_variant(name, &board)?;
        let (signals, exits, per, global) = measure(poses, &board)?;
        eprintln!(
            "ablation: {name}: wirelength {:.0} mm, channel overflow {:.1}, soft overflow {:.1}",
            signals.wirelength_mm,
            signals.channel_overflow,
            signals
                .global
                .as_ref()
                .map_or(0.0, |global| global.soft_overflow_05)
        );
        // Every part that moves between the two placements.
        let interesting: BTreeSet<usize> = movable
            .iter()
            .copied()
            .filter(|index| parts[*index].pads > 0)
            .collect();
        described.push(
            interesting
                .iter()
                .map(|index| {
                    part_escape(
                        problem,
                        &fact,
                        &members,
                        poses,
                        count,
                        &exits,
                        &per,
                        global.as_ref(),
                        &parts,
                        references,
                        *index,
                    )
                })
                .collect::<Vec<_>>(),
        );
        variants.push(KiCadAblationVariant {
            name: name.into(),
            base: name.into(),
            group: None,
            moved: 0,
            displaced: Vec::new(),
            unplaced: Vec::new(),
            signals,
            directory,
        });
        for replicate in 1..=config.replicates {
            let mut nudged = poses.clone();
            let Some(index) = nudge_passive(
                problem,
                &mut nudged,
                &movable,
                &parts,
                config.passive_pads,
                &BTreeSet::new(),
                replicate - 1,
                config.replicates,
                config.null_nudge_mm,
            ) else {
                continue;
            };
            let replicate_name = format!("{name}-r{replicate}");
            let board = board_at(&pcb, &nudged, &ours, backs, &parts)?;
            let directory = write_variant(&replicate_name, &board)?;
            let (signals, _, _, _) = measure(&nudged, &board)?;
            variants.push(KiCadAblationVariant {
                name: replicate_name,
                base: name.into(),
                group: Some(format!("null:{}", parts[index].reference)),
                moved: 1,
                displaced: Vec::new(),
                unplaced: Vec::new(),
                signals,
                directory,
            });
        }
    }
    // The null variants: a passive nudged, legal where it lands, spread
    // over the board (every `step`-th candidate).
    for (base, from, from_backs, prefix) in [
        ("designer", &designer, &designer_backs, "d"),
        ("ours", &ours, &ours_backs, "o"),
    ] {
        let candidates: Vec<usize> = movable
            .iter()
            .copied()
            .filter(|index| {
                (2..=config.passive_pads).contains(&parts[*index].pads)
                    && !parts[*index].connector
                    && placer::legal::is_legal(
                        problem,
                        from,
                        *index,
                        from[*index],
                        0..problem.components.len(),
                    )
            })
            .collect();
        let step = (candidates.len() / (config.nulls + 1)).max(1);
        let mut made = 0;
        let mut at = step;
        while made < config.nulls && at < candidates.len() {
            let index = candidates[at];
            at += step;
            let mut poses = from.clone();
            let nudged = [0, 1, 2, 3].into_iter().find_map(|direction| {
                let (dx, dy) = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)][direction];
                let pose = placer::Pose {
                    position: [
                        from[index].position[0] + dx * config.null_nudge_mm,
                        from[index].position[1] + dy * config.null_nudge_mm,
                    ],
                    angle: from[index].angle,
                };
                placer::legal::is_legal(problem, from, index, pose, 0..problem.components.len())
                    .then_some(pose)
            });
            let Some(pose) = nudged else {
                continue;
            };
            poses[index] = pose;
            made += 1;
            let name = format!("{prefix}-null-{made}");
            let board = board_at(&pcb, &poses, &ours, from_backs, &parts)?;
            let directory = write_variant(&name, &board)?;
            let (signals, _, _, _) = measure(&poses, &board)?;
            eprintln!(
                "ablation: {name}: {} nudged by {} mm",
                parts[index].reference, config.null_nudge_mm
            );
            variants.push(KiCadAblationVariant {
                name,
                base: base.into(),
                group: Some(format!("null:{}", parts[index].reference)),
                moved: 1,
                displaced: Vec::new(),
                unplaced: Vec::new(),
                signals,
                directory,
            });
        }
    }
    for (group, indices) in &groups {
        for (base, from, from_backs, to, to_backs, prefix) in [
            (
                "designer",
                &designer,
                &designer_backs,
                &ours,
                &ours_backs,
                "d",
            ),
            ("ours", &ours, &ours_backs, &designer, &designer_backs, "o"),
        ] {
            let mut poses = from.clone();
            let mut backs = from_backs.clone();
            for index in indices {
                poses[*index] = to[*index];
                backs[*index] = to_backs[*index];
            }
            let (displaced, unplaced) = if config.repair {
                repair(problem, &mut poses, indices, count)
            } else {
                (Vec::new(), Vec::new())
            };
            let name = format!("{prefix}-{}", group.name);
            let board = board_at(&pcb, &poses, &ours, &backs, &parts)?;
            let directory = write_variant(&name, &board)?;
            let (signals, _, _, _) = measure(&poses, &board)?;
            eprintln!(
                "ablation: {name}: {} moved, {} displaced, wirelength {:.0} mm, channel overflow {:.1}, soft overflow {:.1}",
                indices.len(),
                displaced.len(),
                signals.wirelength_mm,
                signals.channel_overflow,
                signals
                    .global
                    .as_ref()
                    .map_or(0.0, |global| global.soft_overflow_05)
            );
            variants.push(KiCadAblationVariant {
                name: name.clone(),
                base: base.into(),
                group: Some(group.name.clone()),
                moved: indices.len(),
                displaced: displaced
                    .iter()
                    .map(|(index, moved)| KiCadDisplaced {
                        reference: references[*index].clone(),
                        distance_mm: (*moved * 1000.0).round() / 1000.0,
                    })
                    .collect(),
                unplaced: unplaced
                    .iter()
                    .map(|index| references[*index].clone())
                    .collect(),
                signals,
                directory,
            });
            let mut avoid: BTreeSet<usize> = indices.clone();
            avoid.extend(displaced.iter().map(|(index, _)| *index));
            for replicate in 1..=config.replicates {
                let mut nudged = poses.clone();
                if nudge_passive(
                    problem,
                    &mut nudged,
                    &movable,
                    &parts,
                    config.passive_pads,
                    &avoid,
                    replicate - 1,
                    config.replicates,
                    config.null_nudge_mm,
                )
                .is_none()
                {
                    continue;
                }
                let replicate_name = format!("{name}-r{replicate}");
                let board = board_at(&pcb, &nudged, &ours, &backs, &parts)?;
                let directory = write_variant(&replicate_name, &board)?;
                let (signals, _, _, _) = measure(&nudged, &board)?;
                variants.push(KiCadAblationVariant {
                    name: replicate_name,
                    base: base.into(),
                    group: Some(group.name.clone()),
                    moved: indices.len(),
                    displaced: Vec::new(),
                    unplaced: Vec::new(),
                    signals,
                    directory,
                });
            }
        }
    }
    let parts_ours = described.pop().unwrap_or_default();
    let parts_designer = described.pop().unwrap_or_default();
    Ok(KiCadAblationReport {
        board_id: board_id.into(),
        footprints: count,
        movable: movable.len(),
        same_pose,
        flipped,
        track_pitch_mm: fact.pitch,
        routing_layers: fact.layers,
        groups: groups.into_iter().map(|(group, _)| group).collect(),
        variants,
        parts_designer,
        parts_ours,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connectors_by_reference_or_library() {
        assert!(is_connector(
            "J3",
            "Connector_PinHeader_2.54mm:PinHeader_1x04"
        ));
        assert!(is_connector("P1", "Local:Something"));
        assert!(is_connector("CN2", "Local:Something"));
        assert!(is_connector("U9", "Connector_USB:USB_C_Receptacle"));
        assert!(!is_connector(
            "X1",
            "Crystal:Crystal_SMD_3225-4Pin_3.2x2.5mm"
        ));
        assert!(!is_connector("U1", "Package_QFP:LQFP-64"));
        assert!(!is_connector("R4", "Resistor_SMD:R_0603"));
    }

    #[test]
    fn box_distance_is_zero_inside_and_euclidean_outside() {
        assert_eq!(box_distance([0.5, 0.5], [0.0, 0.0], [1.0, 1.0]), 0.0);
        assert!((box_distance([4.0, 0.0], [0.0, 0.0], [1.0, 1.0]) - 3.0).abs() < 1.0e-12);
        assert!((box_distance([4.0, 5.0], [0.0, 0.0], [1.0, 1.0]) - 5.0).abs() < 1.0e-12);
    }
}

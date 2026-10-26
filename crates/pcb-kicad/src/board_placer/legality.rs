// Copyright (C) 2026 Toit contributors.

//! The legality census (`legality-kicad-board`): a board's own placement
//! (a designer's) judged by the placer's model at every relaxation level,
//! each illegal part with what it meets and why, beside what KiCad's DRC
//! report of the same board says of the same parts.

use super::*;
use pcb_placer as core;
use serde::Serialize;

/// Below this (mm) a gap or an overlap is a touch.
const TOUCH: f64 = 0.01;

/// The DRC types that are a placement's doing between two footprints (or
/// one footprint and the edge).
const PLACEMENT_TYPES: &[&str] = &[
    "courtyards_overlap",
    "clearance",
    "solder_mask_bridge",
    "hole_clearance",
    "hole_to_hole",
    "holes_co_located",
    "pth_inside_courtyard",
    "npth_inside_courtyard",
    "shorting_items",
    "copper_edge_clearance",
    "edge_clearance",
];

#[derive(Clone, Debug, Serialize)]
pub struct KiCadLegalityReport {
    pub board_id: String,
    /// Movable and fixed footprints (the constraints' and the lowering's).
    pub movable: usize,
    pub fixed: usize,
    pub rules: KiCadLegalityRules,
    /// Per level, loosest first, the movable parts illegal at the board's
    /// own poses.
    pub levels: Vec<KiCadLegalityLevel>,
    /// Every movable part illegal at some level, with what it meets at
    /// the tightest level where it is illegal.
    pub parts: Vec<KiCadLegalityPart>,
    /// What the DRC report says, when one was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kicad: Option<KiCadLegalityDrc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadLegalityRules {
    /// The bodies' least spacing: the largest copper clearance.
    pub clearance_mm: f64,
    pub copper_edge_mm: f64,
    pub edge_margin_mm: f64,
    /// The project's severity of `courtyards_overlap` (KiCad's default:
    /// error).
    pub courtyards_overlap: String,
    pub tight_bodies: bool,
    pub holes_in_courtyards_error: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadLegalityLevel {
    pub index: usize,
    pub halo_scale: f64,
    pub small_halo_scale: f64,
    pub spacing_mm: f64,
    pub grid_mm: f64,
    pub edge_inset: bool,
    pub tight: bool,
    pub edge_copper: bool,
    pub edge_rule_mm: f64,
    pub courtyard_spacing: bool,
    /// KiCad's legality (`legality: "kicad"`) at this level.
    pub pad_clearance: bool,
    pub illegal: usize,
    /// Illegal parts with a KiCad error against what they meet (the model
    /// is right there), and those without (the model is stricter than
    /// KiCad); only with a DRC report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kicad_agrees: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_stricter: Option<usize>,
    /// Illegal parts per cause (a part counts once for each cause it has).
    pub causes: BTreeMap<String, usize>,
    /// The same, only the parts without a KiCad error.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub stricter_causes: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadLegalityPart {
    /// The reference, with the uuid where it repeats on the board.
    pub label: String,
    pub side: String,
    pub at: [f64; 3],
    /// The levels at which it is illegal.
    pub illegal_levels: Vec<usize>,
    /// The level its conflicts are reported at (the tightest of them).
    pub level: usize,
    pub conflicts: Vec<KiCadLegalityConflict>,
    /// KiCad errors on the part against what it meets, if any
    /// (`kicad_agrees`), with a DRC report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kicad_agrees: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadLegalityConflict {
    /// The part or obstacle met, or `outline`.
    pub other: String,
    pub other_fixed: bool,
    /// The placer's test that turns the pose down.
    pub test: String,
    /// Why, measured on the footprints' own shapes.
    pub cause: String,
    /// Courtyard to courtyard (negative: overlap depth), mm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub courtyard_gap_mm: Option<f64>,
    /// The model's bodies (no halo), mm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_gap_mm: Option<f64>,
    /// The nearest two pads of different nets, and the clearance the
    /// project's classes ask of them, mm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad_gap_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad_clearance_mm: Option<f64>,
    /// Pads to the board edge (outline conflicts), mm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_gap_mm: Option<f64>,
    /// KiCad's findings between the two (errors; warnings apart).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub kicad_errors: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub kicad_warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadLegalityDrc {
    /// Errors per type, every type.
    pub errors: BTreeMap<String, usize>,
    /// The placement's errors (`PLACEMENT_TYPES`) on movable footprints.
    pub placement_errors: BTreeMap<String, usize>,
}

/// One footprint's own shapes on the board at its pose: courtyard pieces
/// and pads (with their nets) as quadrilaterals.
struct Shapes {
    courtyard: Vec<[[f64; 2]; 4]>,
    pads: Vec<Pad>,
    bounds: [f64; 4],
}

/// A pad's copper: its box less the corner radius, the radius (a disc's,
/// an oval's or a rounded rectangle's), its net and copper layers (front
/// 1, back 2, inner 4).
struct Pad {
    core: [[f64; 2]; 4],
    radius: f64,
    net: Option<String>,
    layers: u8,
}

fn quad(local: [f64; 4], pose: core::Pose) -> [[f64; 2]; 4] {
    [
        [local[0], local[1]],
        [local[2], local[1]],
        [local[2], local[3]],
        [local[0], local[3]],
    ]
    .map(|point| {
        let turned = core::problem::rotate(point, -pose.angle);
        [pose.position[0] + turned[0], pose.position[1] + turned[1]]
    })
}

fn quad_bounds(quad: &[[f64; 2]; 4]) -> [f64; 4] {
    quad.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    )
}

fn axis_aligned(quad: &[[f64; 2]; 4]) -> bool {
    (quad[0][0] - quad[1][0]).abs() < 1.0e-9 || (quad[0][1] - quad[1][1]).abs() < 1.0e-9
}

/// The gap between two convex quadrilaterals: their distance where they
/// are apart (exact for boxes square to the board, else the separating
/// axes' bound), the negative depth of their overlap where they overlap.
fn gap(a: &[[f64; 2]; 4], b: &[[f64; 2]; 4]) -> f64 {
    if axis_aligned(a) && axis_aligned(b) {
        let (p, q) = (quad_bounds(a), quad_bounds(b));
        let dx = (q[0] - p[2]).max(p[0] - q[2]);
        let dy = (q[1] - p[3]).max(p[1] - q[3]);
        return if dx > 0.0 && dy > 0.0 {
            dx.hypot(dy)
        } else {
            dx.max(dy)
        };
    }
    let mut best = f64::NEG_INFINITY;
    for quad in [a, b] {
        for edge in 0..4 {
            let (p, q) = (quad[edge], quad[(edge + 1) % 4]);
            let length = (q[0] - p[0]).hypot(q[1] - p[1]);
            if length < 1.0e-12 {
                continue;
            }
            let axis = [-(q[1] - p[1]) / length, (q[0] - p[0]) / length];
            let project = |quad: &[[f64; 2]; 4]| {
                quad.iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), point| {
                        let value = point[0] * axis[0] + point[1] * axis[1];
                        (lo.min(value), hi.max(value))
                    })
            };
            let ((alo, ahi), (blo, bhi)) = (project(a), project(b));
            best = best.max((blo - ahi).max(alo - bhi));
        }
    }
    best
}

fn bounds_gap(a: [f64; 4], b: [f64; 4]) -> f64 {
    (b[0] - a[2])
        .max(a[0] - b[2])
        .max((b[1] - a[3]).max(a[1] - b[3]))
}

fn point_segment(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let length = d[0] * d[0] + d[1] * d[1];
    let t = if length < 1.0e-18 {
        0.0
    } else {
        (((point[0] - a[0]) * d[0] + (point[1] - a[1]) * d[1]) / length).clamp(0.0, 1.0)
    };
    (point[0] - a[0] - t * d[0]).hypot(point[1] - a[1] - t * d[1])
}

/// The pads' least distance to the outline (negative where a pad's
/// corner lies outside it).
fn edge_gap(pads: &[Pad], outline: &[[f64; 2]]) -> Option<f64> {
    if pads.is_empty() || outline.len() < 3 {
        return None;
    }
    let mut least = f64::INFINITY;
    for pad in pads {
        let quad = &pad.core;
        let outside = quad
            .iter()
            .any(|corner| !core::problem::point_in_polygon(*corner, outline));
        let mut near = f64::INFINITY;
        for index in 0..outline.len() {
            let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
            for corner in quad {
                near = near.min(point_segment(*corner, a, b));
            }
            for end in [a, b] {
                for side in 0..4 {
                    near = near.min(point_segment(end, quad[side], quad[(side + 1) % 4]));
                }
            }
        }
        least = least.min(if outside { -near } else { near } - pad.radius);
    }
    Some(least)
}

/// Each footprint's own shapes at its pose on the board.
fn footprint_shapes(pcb: &Expr, lowered: &LoweredPlacement) -> Result<Vec<Shapes>, String> {
    let mut shapes = Vec::new();
    for (index, footprint) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .enumerate()
    {
        let component = &lowered.problem.components[index];
        let pose = lowered.problem.poses[index];
        let back = component.side == core::Side::Back;
        let (own, other) = if back { ("B.", "F.") } else { ("F.", "B.") };
        let has = |prefix: &str| {
            footprint.children().iter().any(|child| {
                form_atom(child, "layer", 1)
                    .is_some_and(|layer| layer.starts_with(prefix) && layer.ends_with(".CrtYd"))
            })
        };
        let skip = (has(own) && has(other)).then_some(other);
        let mut slabs = courtyard_slabs(footprint, skip)?;
        if slabs.is_empty() {
            // Not closed, or arcs: the box of its courtyard graphics.
            slabs = footprint
                .children()
                .iter()
                .filter(|child| {
                    form_atom(child, "layer", 1).is_some_and(|layer| {
                        layer.ends_with(".CrtYd")
                            && !skip.is_some_and(|side| layer.starts_with(side))
                    })
                })
                .map(graphic_box)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .reduce(|a, b| {
                    [
                        a[0].min(b[0]),
                        a[1].min(b[1]),
                        a[2].max(b[2]),
                        a[3].max(b[3]),
                    ]
                })
                .into_iter()
                .collect();
        }
        let courtyard: Vec<_> = slabs.into_iter().map(|slab| quad(slab, pose)).collect();
        let mut pads = Vec::new();
        for (local, pad) in component.pads.iter().zip(
            footprint
                .children()
                .iter()
                .filter(|child| child.head() == Some("pad")),
        ) {
            let kind = pad.children().get(2).and_then(Expr::atom).unwrap_or("");
            if kind == "np_thru_hole" {
                continue;
            }
            let layers = if kind == "thru_hole" {
                7
            } else {
                pad.child("layers")
                    .map(|layers| {
                        layers
                            .children()
                            .iter()
                            .skip(1)
                            .filter_map(Expr::atom)
                            .map(|layer| match layer.trim_matches('"') {
                                "F.Cu" => 1,
                                "B.Cu" => 2,
                                "F&B.Cu" => 3,
                                "*.Cu" => 7,
                                layer if layer.ends_with(".Cu") => 4,
                                _ => 0,
                            })
                            .fold(0u8, |a, b| a | b)
                    })
                    .unwrap_or(0)
            };
            if layers == 0 {
                continue;
            }
            let size = form_xy(pad, "size").unwrap_or([0.0, 0.0]);
            let least = size[0].min(size[1]).max(0.0);
            let radius = match pad.children().get(3).and_then(Expr::atom).unwrap_or("") {
                "circle" | "oval" => least / 2.0,
                "roundrect" => {
                    form_f64(pad, "roundrect_rratio", 1)
                        .unwrap_or(0.25)
                        .clamp(0.0, 0.5)
                        * least
                }
                _ => 0.0,
            };
            let radius = radius
                .min((local[2] - local[0]) / 2.0)
                .min((local[3] - local[1]) / 2.0)
                .max(0.0);
            pads.push(Pad {
                core: quad(
                    [
                        local[0] + radius,
                        local[1] + radius,
                        local[2] - radius,
                        local[3] - radius,
                    ],
                    pose,
                ),
                radius,
                net: node_net(pad)
                    .filter(|net| !net.is_empty())
                    .map(str::to_string),
                layers,
            });
        }
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for (quad, grow) in courtyard
            .iter()
            .map(|quad| (quad, 0.0))
            .chain(pads.iter().map(|pad| (&pad.core, pad.radius)))
        {
            let b = quad_bounds(quad);
            let b = [b[0] - grow, b[1] - grow, b[2] + grow, b[3] + grow];
            bounds = [
                bounds[0].min(b[0]),
                bounds[1].min(b[1]),
                bounds[2].max(b[2]),
                bounds[3].max(b[3]),
            ];
        }
        shapes.push(Shapes {
            courtyard,
            pads,
            bounds,
        });
    }
    Ok(shapes)
}

/// KiCad's findings by footprint pair (indices, the lower first; a
/// single footprint as `(i, i)`), errors and warnings.
type Findings = BTreeMap<(usize, usize), (BTreeSet<String>, BTreeSet<String>)>;

fn read_drc(
    path: &Path,
    pcb: &Expr,
    references: &[String],
    footprints: usize,
    movable: &[bool],
) -> Result<(Findings, BTreeSet<usize>, KiCadLegalityDrc), String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let report: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    // Every uuid of a footprint and its pads (a panel's copies may share
    // them: those are left to the references).
    let mut by_uuid: BTreeMap<String, Option<usize>> = BTreeMap::new();
    let mut note = |uuid: String, index: usize| {
        by_uuid
            .entry(uuid)
            .and_modify(|known| {
                if *known != Some(index) {
                    *known = None;
                }
            })
            .or_insert(Some(index));
    };
    for (index, footprint) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .enumerate()
    {
        if let Some(uuid) = crate::kicad_nets::footprint_uuid(footprint) {
            note(uuid, index);
        }
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if let Some(uuid) = crate::kicad_nets::footprint_uuid(pad) {
                note(uuid, index);
            }
        }
    }
    let mut by_reference: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, reference) in references.iter().take(footprints).enumerate() {
        by_reference.entry(reference).or_default().push(index);
    }
    let footprint_of = |item: &serde_json::Value| -> Option<usize> {
        if let Some(known) = item["uuid"].as_str().and_then(|uuid| by_uuid.get(uuid)) {
            return *known;
        }
        let description = item["description"].as_str()?;
        let reference = description
            .strip_prefix("Footprint ")
            .or_else(|| description.split(" of ").nth(1))?
            .split_whitespace()
            .next()?;
        match by_reference.get(reference).map(Vec::as_slice) {
            Some([index]) => Some(*index),
            _ => None,
        }
    };
    let mut findings = Findings::new();
    let mut unbuilt = BTreeSet::new();
    let mut errors = BTreeMap::new();
    let mut placement_errors = BTreeMap::new();
    for violation in report["violations"].as_array().into_iter().flatten() {
        let kind = violation["type"].as_str().unwrap_or("").to_string();
        let error = violation["severity"].as_str() == Some("error");
        if kind == "malformed_courtyard" {
            unbuilt.extend(
                violation["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(footprint_of),
            );
            continue;
        }
        if error {
            *errors.entry(kind.clone()).or_insert(0usize) += 1;
        }
        if !PLACEMENT_TYPES.contains(&kind.as_str()) {
            continue;
        }
        let mut parts: Vec<usize> = violation["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(footprint_of)
            .collect();
        parts.sort_unstable();
        parts.dedup();
        let pairs: Vec<(usize, usize)> = match parts.as_slice() {
            [] => Vec::new(),
            [one] => vec![(*one, *one)],
            many => {
                let mut pairs = Vec::new();
                for (k, a) in many.iter().enumerate() {
                    for b in &many[k + 1..] {
                        pairs.push((*a, *b));
                    }
                }
                pairs
            }
        };
        if error && parts.iter().any(|part| movable[*part]) {
            *placement_errors.entry(kind.clone()).or_insert(0usize) += 1;
        }
        for pair in pairs {
            let entry = findings.entry(pair).or_default();
            if error {
                entry.0.insert(kind.clone());
            } else {
                entry.1.insert(kind.clone());
            }
        }
    }
    Ok((
        findings,
        unbuilt,
        KiCadLegalityDrc {
            errors,
            placement_errors,
        },
    ))
}

/// The clearance the project's classes ask between two nets.
fn pair_clearance(rules: Option<&KiCadBoardRouterConfig>, a: &str, b: &str, fallback: f64) -> f64 {
    let Some(rules) = rules else {
        return fallback;
    };
    let of = |net: &str| {
        rules
            .connection_rules
            .get(crate::kicad_nets::normalize_net(net))
            .or(rules.default_rules.as_ref())
            .map(|rules| rules.clearance_mm)
            .unwrap_or(fallback)
    };
    let pad = rules
        .project_rules
        .as_ref()
        .and_then(|report| report.pad_clearance_mm)
        .unwrap_or(0.0);
    of(a).max(of(b)).max(pad)
}

/// The nearest pads of different nets of two footprints and their
/// clearance.
fn pad_gap(
    a: &Shapes,
    b: &Shapes,
    rules: Option<&KiCadBoardRouterConfig>,
    fallback: f64,
) -> Option<(f64, f64)> {
    if bounds_gap(a.bounds, b.bounds) > 2.0 {
        return None;
    }
    let mut best: Option<(f64, f64)> = None;
    for pa in &a.pads {
        let ba = quad_bounds(&pa.core);
        for pb in &b.pads {
            if (pa.net.is_some() && pa.net == pb.net) || pa.layers & pb.layers == 0 {
                continue;
            }
            if bounds_gap(ba, quad_bounds(&pb.core)) > 2.0 + pa.radius + pb.radius {
                continue;
            }
            let distance = gap(&pa.core, &pb.core) - pa.radius - pb.radius;
            let clearance = pair_clearance(
                rules,
                pa.net.as_deref().unwrap_or(""),
                pb.net.as_deref().unwrap_or(""),
                fallback,
            );
            // The pair that falls shortest of its clearance.
            if best.is_none_or(|(d, c)| distance - clearance < d - c) {
                best = Some((distance, clearance));
            }
        }
    }
    best
}

fn courtyard_gap(a: &Shapes, b: &Shapes) -> Option<f64> {
    if a.courtyard.is_empty() || b.courtyard.is_empty() {
        return None;
    }
    let mut least = f64::INFINITY;
    for qa in &a.courtyard {
        for qb in &b.courtyard {
            least = least.min(gap(qa, qb));
        }
    }
    Some(least)
}

fn body_quad(component: &core::Component, pose: core::Pose) -> [[f64; 2]; 4] {
    let (c, s) = (component.body_center, component.body_size);
    quad(
        [
            c[0] - s[0] / 2.0,
            c[1] - s[1] / 2.0,
            c[0] + s[0] / 2.0,
            c[1] + s[1] / 2.0,
        ],
        pose,
    )
}

/// Moves every footprint of `pcb` to the pose of its footprint in `from`:
/// the one with its uuid, else the one with its reference (the n-th of
/// a repeated reference to the n-th). Footprints `from` lacks stay.
pub(super) fn take_poses(pcb: &mut Expr, from: &Expr) -> Result<usize, String> {
    let mut by_uuid: BTreeMap<String, Vec<[f64; 3]>> = BTreeMap::new();
    let mut by_reference: BTreeMap<String, Vec<[f64; 3]>> = BTreeMap::new();
    for footprint in from
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let at = form_at(footprint)?;
        if let Some(uuid) = crate::kicad_nets::footprint_uuid(footprint) {
            by_uuid.entry(uuid).or_default().push(at);
        }
        by_reference
            .entry(footprint_reference(footprint).unwrap_or_default())
            .or_default()
            .push(at);
    }
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    // A uuid the copies of a panel share goes to its n-th footprint.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut seen_uuid: BTreeMap<String, usize> = BTreeMap::new();
    let mut moved = 0;
    for footprint in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let nth = seen.entry(reference.clone()).or_default();
        let at = crate::kicad_nets::footprint_uuid(footprint)
            .and_then(|uuid| {
                let nth = seen_uuid.entry(uuid.clone()).or_default();
                *nth += 1;
                by_uuid
                    .get(&uuid)
                    .and_then(|ats| ats.get(*nth - 1).copied())
            })
            .or_else(|| {
                by_reference
                    .get(&reference)
                    .and_then(|ats| ats.get(*nth).copied())
            });
        *nth += 1;
        if let Some(at) = at {
            write_footprint_pose(
                footprint,
                core::Pose {
                    position: [at[0], at[1]],
                    angle: at[2],
                },
            )?;
            moved += 1;
        }
    }
    Ok(moved)
}

/// Judges the board's own placement (or, with `poses_from`, the poses of
/// that board's footprints: a designer's on a task's prepared board) by
/// the placer's model at every relaxation level (`level_problems`), with
/// the board's constraints and project rules as `place_kicad_board` takes
/// them; `drc`: KiCad's report of the board whose poses are judged, to
/// tell where KiCad agrees.
pub fn legality_kicad_board(
    source_directory: &Path,
    board_id: &str,
    config: &KiCadBoardPlacerConfig,
    poses_from: Option<&Path>,
    drc: Option<&Path>,
) -> Result<KiCadLegalityReport, String> {
    let from = match poses_from {
        Some(path) => Some(crate::canonical_order::parse_board(
            &fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?,
        )?),
        None => None,
    };
    // The directory's constraints, as a layout.json beside them would ask.
    let mut config = config.clone();
    if config.constraints.is_none() && source_directory.join("constraints.json").exists() {
        config.constraints = Some(KiCadConstraintsSource::File("constraints.json".into()));
    }
    let prepared = prepare_board(source_directory, board_id, &config, from.as_ref())?;
    let (pcb, config, lowered) = (&prepared.pcb, &prepared.config, &prepared.lowered);
    let project = source_directory.join(format!("{board_id}.kicad_pro"));
    let router_rules = project_rules::resolve_project_rules(
        &project,
        &source_directory.join(format!("{board_id}.kicad_pcb")),
    )
    .ok();
    let problem = &lowered.problem;
    let footprints = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .count();
    let count = problem.components.len();
    let movable: Vec<bool> = problem.components.iter().map(|c| !c.fixed).collect();
    let mut placer_config = core::Config::new();
    placer_config.maximum_utilization = config.maximum_utilization;
    placer_config.overlap_far_side_pads = config.overlap_far_side_pads;
    placer_config.kicad_legality = config.legality == KiCadPlacerLegality::Kicad;
    let levels = core::level_problems(problem, &placer_config);
    let poses = problem.poses.clone();
    let shapes = footprint_shapes(pcb, lowered)?;
    let findings = match drc {
        Some(path) => Some(read_drc(
            path,
            pcb,
            &lowered.references,
            footprints,
            &movable,
        )?),
        None => None,
    };
    let mut repeated: BTreeMap<&str, usize> = BTreeMap::new();
    for reference in lowered.references.iter().take(footprints) {
        *repeated.entry(reference).or_default() += 1;
    }
    let uuids: Vec<Option<String>> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .map(crate::kicad_nets::footprint_uuid)
        .collect();
    let label = |index: usize| -> String {
        let reference = &lowered.references[index];
        match uuids.get(index).cloned().flatten() {
            Some(uuid) if repeated.get(reference.as_str()).copied().unwrap_or(0) > 1 => {
                format!("{reference} (uuid {uuid})")
            }
            _ => reference.clone(),
        }
    };
    let severity = fs::read_to_string(&project)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|project| {
            project["board"]["design_settings"]["rule_severities"]["courtyards_overlap"]
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "error".into());
    let clearance = problem.min_spacing;
    // The conflicts of part `index` at a level.
    let conflicts_at = |relaxed: &core::Problem, index: usize| -> Vec<KiCadLegalityConflict> {
        let pose = poses[index];
        let component = &relaxed.components[index];
        let mut out = Vec::new();
        let kicad_of = |other: usize| -> (Vec<String>, Vec<String>) {
            let Some((findings, _, _)) = &findings else {
                return (Vec::new(), Vec::new());
            };
            let key = (index.min(other), index.max(other));
            findings
                .get(&key)
                .map(|(errors, warnings)| {
                    (
                        errors.iter().cloned().collect(),
                        warnings.iter().cloned().collect(),
                    )
                })
                .unwrap_or_default()
        };
        if !core::legal::is_legal(relaxed, &poses, index, pose, std::iter::empty()) {
            let edge = (index < footprints)
                .then(|| edge_gap(&shapes[index].pads, &relaxed.outline))
                .flatten();
            let copper_edge = relaxed.constraints.copper_edge;
            let cause = if !core::constraints::hard_ok(relaxed, index, pose) {
                "a constraint of the task (edge, region, near)".to_string()
            } else if edge.is_some_and(|gap| gap < copper_edge - TOUCH) {
                "pads closer to the edge than the copper-to-edge clearance".to_string()
            } else if !core::legal::body_inside_outline(relaxed, index, pose) {
                "body over the edge, copper keeps the copper-to-edge clearance".to_string()
            } else {
                "body within the edge margin, copper keeps the copper-to-edge clearance".to_string()
            };
            let (errors, warnings) = kicad_of(index);
            out.push(KiCadLegalityConflict {
                other: "outline".into(),
                other_fixed: true,
                test: "outline or hard constraint".into(),
                cause,
                courtyard_gap_mm: None,
                body_gap_mm: None,
                pad_gap_mm: None,
                pad_clearance_mm: None,
                edge_gap_mm: edge,
                kicad_errors: errors
                    .into_iter()
                    .filter(|kind| kind.contains("edge"))
                    .collect(),
                kicad_warnings: warnings
                    .into_iter()
                    .filter(|kind| kind.contains("edge"))
                    .collect(),
            });
        }
        let reach = 12.0 + component.halo + relaxed.spacing;
        let own_half = component.half_extent(pose.angle);
        let own_center = component.center(pose);
        for other in 0..count {
            if other == index {
                continue;
            }
            let theirs = &relaxed.components[other];
            let half = theirs.half_extent(poses[other].angle);
            let center = theirs.center(poses[other]);
            if (own_center[0] - center[0]).abs() > own_half[0] + half[0] + reach
                || (own_center[1] - center[1]).abs() > own_half[1] + half[1] + reach
            {
                continue;
            }
            let Some(test) = core::legal::conflict_kind(relaxed, &poses, index, pose, other) else {
                continue;
            };
            let obstacle = other >= footprints;
            let (courtyard, pads) = if obstacle || index >= footprints {
                (None, None)
            } else {
                (
                    courtyard_gap(&shapes[index], &shapes[other]),
                    pad_gap(
                        &shapes[index],
                        &shapes[other],
                        router_rules.as_ref(),
                        clearance,
                    ),
                )
            };
            let body = (!obstacle).then(|| {
                gap(
                    &body_quad(component, pose),
                    &body_quad(theirs, poses[other]),
                )
            });
            let cause = if obstacle {
                format!("meets a {}", lowered.references[other])
            } else if test == "far side" {
                "far side: a hole, lead or courtyard under a part on the other side".into()
            } else if test == "pads in a cutout or pad keepout" {
                test.to_string()
            } else if pads.is_some_and(|(gap, clearance)| gap < clearance - TOUCH) {
                "pads closer than their clearance".into()
            } else if courtyard.is_some_and(|gap| gap < -TOUCH)
                && findings.as_ref().is_some_and(|(_, unbuilt, _)| {
                    unbuilt.contains(&index) || unbuilt.contains(&other)
                })
            {
                "courtyards overlap, one KiCad cannot build (malformed: not checked)".into()
            } else if courtyard.is_some_and(|gap| gap < -TOUCH) {
                "courtyards overlap".into()
            } else if pads.is_some_and(|(gap, _)| gap < clearance - TOUCH) {
                "pads closer than the largest clearance, not their own".into()
            } else if body.is_some_and(|gap| gap < -TOUCH) {
                if courtyard.is_none() {
                    "bodies overlap, a part without courtyard".into()
                } else {
                    "bodies overlap, courtyards do not (pads, mask or copper past the courtyard, or its box)"
                        .into()
                }
            } else if test == "shaped or hollow boxes" {
                "shaped or hollow boxes within the spacing, courtyards apart".into()
            } else if courtyard.is_none() {
                "a part without courtyard closer than the spacing, pads keep their clearance".into()
            } else if relaxed.constraints.pad_clearance {
                "pads within the clearance as boxes (their corners), not by their shapes".into()
            } else if relaxed.constraints.courtyard_spacing {
                "courtyards apart but closer than their copper's least inset allows, pads keep their clearance"
                    .into()
            } else if courtyard.is_some_and(|gap| gap >= relaxed.spacing - TOUCH) {
                "bodies past their courtyards (pads, mask or copper) closer than the spacing, courtyards keep it"
                    .into()
            } else {
                "courtyards apart but closer than the spacing, pads keep their clearance".into()
            };
            let (errors, warnings) = if obstacle {
                (Vec::new(), Vec::new())
            } else {
                kicad_of(other)
            };
            out.push(KiCadLegalityConflict {
                other: if obstacle {
                    lowered.references[other].clone()
                } else {
                    label(other)
                },
                other_fixed: theirs.fixed,
                test: test.to_string(),
                cause,
                courtyard_gap_mm: courtyard,
                body_gap_mm: body,
                pad_gap_mm: pads.map(|(gap, _)| gap),
                pad_clearance_mm: pads.map(|(_, clearance)| clearance),
                edge_gap_mm: None,
                kicad_errors: errors,
                kicad_warnings: warnings,
            });
        }
        out
    };
    let mut census = Vec::new();
    let mut illegal_levels: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut reported: Vec<Option<(usize, Vec<KiCadLegalityConflict>)>> = vec![None; count];
    for (level, (shape, relaxed)) in levels.iter().enumerate() {
        let mut causes = BTreeMap::new();
        let mut stricter_causes = BTreeMap::new();
        let (mut illegal, mut agrees) = (0, 0);
        for index in 0..footprints.min(count) {
            if !movable[index]
                || core::legal::is_legal(relaxed, &poses, index, poses[index], 0..count)
            {
                continue;
            }
            illegal += 1;
            illegal_levels[index].push(level);
            let conflicts = conflicts_at(relaxed, index);
            let kicad = conflicts
                .iter()
                .any(|conflict| !conflict.kicad_errors.is_empty());
            agrees += kicad as usize;
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            for conflict in &conflicts {
                if seen.insert(&conflict.cause) {
                    *causes.entry(conflict.cause.clone()).or_insert(0usize) += 1;
                    if !kicad {
                        *stricter_causes
                            .entry(conflict.cause.clone())
                            .or_insert(0usize) += 1;
                    }
                }
            }
            reported[index] = Some((level, conflicts));
        }
        census.push(KiCadLegalityLevel {
            index: level,
            halo_scale: shape.halo_scale,
            small_halo_scale: shape.small_halo_scale,
            spacing_mm: shape.spacing,
            grid_mm: shape.grid,
            edge_inset: shape.edge_inset,
            tight: shape.tight,
            edge_copper: shape.edge_copper,
            edge_rule_mm: shape.edge_rule,
            courtyard_spacing: shape.courtyard_spacing,
            pad_clearance: shape.pad_clearance,
            illegal,
            kicad_agrees: findings.as_ref().map(|_| agrees),
            model_stricter: findings.as_ref().map(|_| illegal - agrees),
            causes,
            stricter_causes: if findings.is_some() {
                stricter_causes
            } else {
                BTreeMap::new()
            },
        });
    }
    let parts = (0..footprints.min(count))
        .filter_map(|index| {
            let (level, conflicts) = reported[index].take()?;
            let component = &problem.components[index];
            Some(KiCadLegalityPart {
                label: label(index),
                side: format!("{:?}", component.side),
                at: [
                    poses[index].position[0],
                    poses[index].position[1],
                    poses[index].angle,
                ],
                illegal_levels: std::mem::take(&mut illegal_levels[index]),
                level,
                kicad_agrees: findings.as_ref().map(|_| {
                    conflicts
                        .iter()
                        .any(|conflict| !conflict.kicad_errors.is_empty())
                }),
                conflicts,
            })
        })
        .collect();
    Ok(KiCadLegalityReport {
        board_id: board_id.to_string(),
        movable: movable.iter().take(footprints).filter(|m| **m).count(),
        fixed: movable.iter().take(footprints).filter(|m| !**m).count(),
        rules: KiCadLegalityRules {
            clearance_mm: clearance,
            copper_edge_mm: problem.constraints.copper_edge,
            edge_margin_mm: problem.edge_margin,
            courtyards_overlap: severity,
            tight_bodies: config.tight_bodies == Some(true),
            holes_in_courtyards_error: config.holes_in_courtyards_error == Some(true),
        },
        levels: census,
        parts,
        kicad: findings.map(|(_, _, drc)| drc),
    })
}

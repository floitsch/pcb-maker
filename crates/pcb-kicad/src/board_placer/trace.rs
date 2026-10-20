// Copyright (C) 2026 Toit contributors.

//! Diagnostics: the placement's trace per seed and the designer's poses
//! judged by the placer.

use super::*;
use pcb_placer as core;

/// Debugging aid (`PCB_PLACER_DESIGNER=<designer's .kicad_pcb>`): the
/// designer's poses judged by the placer's model at its loosest level, with
/// what each illegal movable part meets.
/// The movable parts' poses in the designer's board (by reference, in
/// order for repeated references); fixed parts keep their own.
fn designer_poses(lowered: &LoweredPlacement, designer: &Path) -> Result<Vec<core::Pose>, String> {
    let text =
        fs::read_to_string(designer).map_err(|error| format!("{}: {error}", designer.display()))?;
    let pcb = crate::canonical_order::parse_board(&text)?;
    let mut at_by_reference: BTreeMap<String, Vec<[f64; 3]>> = BTreeMap::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        at_by_reference
            .entry(footprint_reference(footprint).unwrap_or_default())
            .or_default()
            .push(form_at(footprint)?);
    }
    let mut poses = lowered.problem.poses.clone();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (index, reference) in lowered.references.iter().enumerate() {
        if lowered.problem.components[index].fixed {
            continue;
        }
        let nth = seen.entry(reference.clone()).or_default();
        if let Some(at) = at_by_reference.get(reference).and_then(|ats| ats.get(*nth)) {
            poses[index] = core::Pose {
                position: [at[0], at[1]],
                angle: at[2],
            };
        }
        *nth += 1;
    }
    Ok(poses)
}

/// Measurement aid (`PCB_PLACER_TRACE=<file>`): where the wirelength goes
/// between the placer's stages, for every seed's placement, as JSON: the
/// weighted wirelength after global placement, annealing, legalization
/// and refinement (and at the designer's poses with
/// `PCB_PLACER_DESIGNER`), the relaxation level, how crowded the global
/// placement is at that level, and per movable part how far each stage
/// moved it and what it met where the annealer left it.
pub(super) fn trace_placements(
    lowered: &LoweredPlacement,
    placements: &[core::Placement],
    anneal: &core::anneal::AnnealConfig,
    path: &Path,
) -> Result<(), String> {
    let designer = match std::env::var_os("PCB_PLACER_DESIGNER") {
        Some(designer) => Some(designer_poses(lowered, Path::new(&designer))?),
        None => None,
    };
    let base = &lowered.problem;
    let count = base.components.len();
    let nets = base.net_weights.len();
    let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nets];
    for (index, component) in base.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            members[description.net].push((index, pin));
        }
    }
    let net_lengths = |poses: &[core::Pose]| -> Vec<f64> {
        members
            .iter()
            .enumerate()
            .map(|(net, pins)| {
                if pins.len() < 2 {
                    return 0.0;
                }
                let mut b = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for (component, pin) in pins {
                    let c = &base.components[*component];
                    let at = c.pin_position(&c.pins[*pin], poses[*component]);
                    b = [
                        b[0].min(at[0]),
                        b[1].min(at[1]),
                        b[2].max(at[0]),
                        b[3].max(at[1]),
                    ];
                }
                base.net_weights[net] * ((b[2] - b[0]) + (b[3] - b[1]))
            })
            .collect()
    };
    let incident: Vec<Vec<usize>> = base
        .components
        .iter()
        .map(|component| {
            let mut nets: Vec<usize> = component.pins.iter().map(|pin| pin.net).collect();
            nets.sort_unstable();
            nets.dedup();
            nets
        })
        .collect();
    let mut seeds = Vec::new();
    for placement in placements {
        let mut relaxed = base.clone();
        placement.relaxation.apply(&mut relaxed);
        let stages: Vec<(&str, &Vec<core::Pose>)> = placement
            .stage_poses
            .iter()
            .map(|(name, poses)| (*name, poses))
            .collect();
        let lengths: Vec<(&str, Vec<f64>)> = stages
            .iter()
            .map(|(name, poses)| (*name, net_lengths(poses)))
            .collect();
        let designer_lengths = designer.as_ref().map(|poses| net_lengths(poses));
        // Crowding: the share of movable parts that meet another part or
        // leave the board at each stage, at the level the placement ended
        // with; and the pairwise overlap of the inflated boxes.
        let mut crowding = serde_json::Map::new();
        for (name, poses) in &stages {
            let mut conflicted = 0;
            let mut overlap = 0.0;
            let mut area = 0.0;
            for index in 0..count {
                let component = &relaxed.components[index];
                if component.fixed {
                    continue;
                }
                let half = component.half_extent(poses[index].angle);
                let margin = component.halo + relaxed.spacing / 2.0;
                let grown = [half[0] + margin, half[1] + margin];
                area += 4.0 * grown[0] * grown[1];
                let center = component.center(poses[index]);
                for other in 0..count {
                    if other == index
                        || relaxed.components[other].copper_only
                        || component.copper_only
                    {
                        continue;
                    }
                    let theirs = &relaxed.components[other];
                    if !component.side.collides(theirs.side) {
                        continue;
                    }
                    let other_half = theirs.half_extent(poses[other].angle);
                    let other_margin = if theirs.fixed {
                        relaxed.spacing / 2.0
                    } else {
                        theirs.halo + relaxed.spacing / 2.0
                    };
                    let other_center = theirs.center(poses[other]);
                    let w = grown[0] + other_half[0] + other_margin
                        - (center[0] - other_center[0]).abs();
                    let h = grown[1] + other_half[1] + other_margin
                        - (center[1] - other_center[1]).abs();
                    if w > 0.0 && h > 0.0 {
                        // Movable pairs count once.
                        overlap += if theirs.fixed { w * h } else { w * h / 2.0 };
                    }
                }
                if !core::legal::is_legal(&relaxed, poses, index, poses[index], 0..count) {
                    conflicted += 1;
                }
            }
            crowding.insert(
                name.to_string(),
                serde_json::json!({"conflicted": conflicted, "overlap_share": overlap / area.max(1.0e-9)}),
            );
        }
        let stage_of = |name: &str| stages.iter().position(|(stage, _)| *stage == name);
        // What the annealer itself sees there (a conflict it does not see
        // is a mismatch of the two models).
        let anneal_overlaps = stage_of("annealed")
            .map(|stage| core::anneal::overlaps(&relaxed, stages[stage].1, anneal));
        let mut parts = Vec::new();
        for index in 0..count {
            let component = &relaxed.components[index];
            if component.fixed {
                continue;
            }
            let centers: Vec<[f64; 2]> = stages
                .iter()
                .map(|(_, poses)| component.center(poses[index]))
                .collect();
            let cost =
                |lengths: &[f64]| -> f64 { incident[index].iter().map(|net| lengths[*net]).sum() };
            let mut entry = serde_json::json!({
                "reference": lowered.references[index],
                "pins": component.pins.len(),
                "body": [component.body_size[0], component.body_size[1]],
                "halo": component.halo,
                "side": format!("{:?}", component.side),
                "edge": relaxed.constraints.edges.iter().any(|(part, _, _)| *part == index),
                "centers": centers,
                "angles": stages.iter().map(|(_, poses)| poses[index].angle).collect::<Vec<_>>(),
                "net_cost": lengths.iter().map(|(_, lengths)| cost(lengths)).collect::<Vec<_>>(),
            });
            if let (Some(poses), Some(lengths)) = (&designer, &designer_lengths) {
                entry["designer_center"] = serde_json::json!(component.center(poses[index]));
                entry["designer_net_cost"] = serde_json::json!(cost(lengths));
            }
            // What the part met where the annealer left it.
            if let Some(annealed) = stage_of("annealed").map(|stage| stages[stage].1) {
                let mut met = Vec::new();
                if !core::legal::is_legal(
                    &relaxed,
                    annealed,
                    index,
                    annealed[index],
                    std::iter::empty(),
                ) {
                    met.push("outline or hard constraint".to_string());
                }
                for other in 0..count {
                    if other == index {
                        continue;
                    }
                    if let Some(kind) = core::legal::conflict_kind(
                        &relaxed,
                        annealed,
                        index,
                        annealed[index],
                        other,
                    ) {
                        met.push(format!(
                            "{}{}: {kind}",
                            lowered.references[other],
                            if relaxed.components[other].fixed {
                                " (fixed)"
                            } else {
                                ""
                            }
                        ));
                    }
                }
                entry["met_annealed"] = serde_json::json!(met);
                entry["anneal_overlap"] =
                    serde_json::json!(anneal_overlaps.as_ref().map(|overlaps| overlaps[index]));
            }
            parts.push(entry);
        }
        let relaxation = &placement.relaxation;
        seeds.push(serde_json::json!({
            "stages": stages.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            "wirelength": lengths.iter().map(|(_, lengths)| lengths.iter().sum::<f64>()).collect::<Vec<_>>(),
            "designer_wirelength": designer_lengths.as_ref().map(|lengths| lengths.iter().sum::<f64>()),
            "unplaced": placement.unplaced.len(),
            "illegal": placement.illegal.len(),
            "work": placement.work,
            "global_iterations": placement.global_iterations,
            "global_overflow": placement.global_overflow,
            "relaxation": {
                "spacing": relaxation.spacing, "grid": relaxation.grid, "halo_scale": relaxation.halo_scale,
                "small_halo_scale": relaxation.small_halo_scale, "edge_inset": relaxation.edge_inset,
                "tight": relaxation.tight, "courtyard_spacing": relaxation.courtyard_spacing,
            },
            "crowding": crowding,
            "parts": parts,
        }));
    }
    let text = serde_json::to_string(&serde_json::json!({"seeds": seeds}))
        .map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
}

pub(super) fn diagnose_designer(lowered: &LoweredPlacement, designer: &Path) -> Result<(), String> {
    let mut problem = lowered.problem.clone();
    let poses = designer_poses(lowered, designer)?;
    eprintln!(
        "designer: wirelength {:.1} mm at these poses",
        problem.wirelength(&poses)
    );
    let level = std::env::var("PCB_PLACER_DESIGNER_LEVEL").unwrap_or_default();
    if level != "full" {
        problem.spacing = problem.min_spacing;
        problem.grid = problem.grid.min(0.1);
        problem.edge_margin = problem
            .edge_margin
            .min((problem.constraints.copper_edge + 0.05).min(problem.edge_margin));
        problem.constraints.courtyard_spacing = level == "touch";
        problem.constraints.courtyard_spacing = level == "touch";
        for component in &mut problem.components {
            component.halo = 0.0;
            if level == "tight" || level == "touch" {
                component.use_tight_body();
            }
        }
    }
    let count = problem.components.len();
    let bounds = problem.bounds();
    eprintln!(
        "designer: outline bounds {bounds:?}, {} pieces, edge margin {}, {} points: {:?}",
        problem.pieces.len(),
        problem.edge_margin,
        problem.outline.len(),
        problem
            .outline
            .iter()
            .step_by((problem.outline.len() / 40).max(1))
            .collect::<Vec<_>>()
    );
    let mut illegal = 0;
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for index in 0..count {
        if problem.components[index].fixed {
            continue;
        }
        let mut reasons = Vec::new();
        let outside =
            !core::legal::is_legal(&problem, &poses, index, poses[index], std::iter::empty());
        if outside {
            reasons.push("outline or hard constraint".to_string());
            *kinds.entry("outline or hard constraint").or_insert(0usize) += 1;
        }
        for other in (0..count).filter(|other| !outside && *other != index) {
            if let Some(kind) =
                core::legal::conflict_kind(&problem, &poses, index, poses[index], other)
            {
                reasons.push(format!(
                    "{} ({:?}): {kind}",
                    lowered.references[other], problem.components[other].side
                ));
                *kinds.entry(kind).or_insert(0usize) += 1;
            }
        }
        if !reasons.is_empty() {
            illegal += 1;
            let component = &problem.components[index];
            eprintln!(
                "designer: {} ({:?}, body {:.2} x {:.2}, at {:?}) meets {}",
                lowered.references[index],
                component.side,
                component.body_size[0],
                component.body_size[1],
                poses[index],
                reasons.join(", ")
            );
        }
    }
    eprintln!(
        "designer: {illegal} movable parts illegal at the designer's poses (level {level:?}); conflicts by kind {kinds:?}"
    );
    Ok(())
}

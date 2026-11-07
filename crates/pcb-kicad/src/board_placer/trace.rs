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

/// The placer's objective split by term (`core::anneal::objective`), as
/// JSON.
fn objective_json(objective: &core::anneal::Objective) -> serde_json::Value {
    serde_json::json!({
        "wirelength": objective.wirelength,
        "half_perimeter": objective.half_perimeter,
        "overlap": objective.overlap,
        "overlapping": objective.overlapping,
        "relations": objective.relations,
        "hard": objective.hard,
        "crossings_anneal": objective.crossings_anneal,
        "crossings_all": objective.crossings_all,
        "crossing_weight": objective.crossing_weight,
    })
}

/// A pose as `[x, y, angle]` (the footprint's origin, degrees).
fn pose_json(pose: core::Pose) -> serde_json::Value {
    serde_json::json!([pose.position[0], pose.position[1], pose.angle])
}

/// Diagnostic start (`PCB_PLACER_START=<designer's .kicad_pcb>`, docs/placer.md
/// "Where the arrangement is lost (2026-11-04)"): the placer's stages from
/// `PCB_PLACER_START_STAGE` (`global`, the default; `anneal`; `legalize`)
/// on run from the designer's poses. With `global`,
/// `PCB_PLACER_START_PARTS=<file>` (a JSON list of `REF#n`, the n-th
/// footprint of that reference in board order, as partialcopy.py's
/// `*.pinned.json`) names the parts that start there; the others start
/// from the placer's own start. `None` without `PCB_PLACER_START`.
pub(super) fn warm_start(lowered: &LoweredPlacement) -> Result<Option<core::Start>, String> {
    let Some(designer) = std::env::var_os("PCB_PLACER_START") else {
        return Ok(None);
    };
    let stage = std::env::var("PCB_PLACER_START_STAGE").ok();
    let parts = std::env::var_os("PCB_PLACER_START_PARTS");
    start_from(
        lowered,
        Path::new(&designer),
        stage.as_deref(),
        parts.as_deref().map(Path::new),
    )
    .map(Some)
}

/// `warm_start` from its settings: the designer's board, the stage
/// (`None`: `global`), the file naming the parts (`None`: every part).
pub(super) fn start_from(
    lowered: &LoweredPlacement,
    designer: &Path,
    stage: Option<&str>,
    parts: Option<&Path>,
) -> Result<core::Start, String> {
    let poses = designer_poses(lowered, designer)?;
    let stage = match stage {
        None | Some("global") => core::StartStage::Global,
        Some("anneal") => core::StartStage::Anneal,
        Some("legalize") => core::StartStage::Legalize,
        Some(other) => {
            return Err(format!(
                "PCB_PLACER_START_STAGE: {other:?} is not global, anneal or legalize"
            ));
        }
    };
    let count = lowered.references.len();
    let parts = match parts {
        None => vec![true; count],
        Some(file) => {
            if stage != core::StartStage::Global {
                return Err(
                    "PCB_PLACER_START_PARTS: only with PCB_PLACER_START_STAGE=global".to_string(),
                );
            }
            let text =
                fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?;
            let names: BTreeSet<String> = serde_json::from_str(&text)
                .map_err(|error| format!("{}: {error}", file.display()))?;
            let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
            let parts: Vec<bool> = lowered
                .references
                .iter()
                .map(|reference| {
                    let nth = seen.entry(reference.as_str()).or_default();
                    let named = names.contains(&format!("{reference}#{nth}"));
                    *nth += 1;
                    named
                })
                .collect();
            let found = parts.iter().filter(|part| **part).count();
            if found != names.len() {
                return Err(format!(
                    "{}: {} parts named, {found} of them on the board",
                    file.display(),
                    names.len(),
                ));
            }
            parts
        }
    };
    Ok(core::Start {
        poses,
        parts,
        stage,
    })
}

/// Measurement aid (`PCB_PLACER_TRACE=<file>`): where the wirelength goes
/// between the placer's stages, for every seed's placement, as JSON: the
/// weighted wirelength after global placement, annealing, legalization
/// and refinement (and at the designer's poses with
/// `PCB_PLACER_DESIGNER`), the relaxation level, how crowded the global
/// placement is at that level, the placer's objective at each stage split
/// by term (at the level, and with no halos: `objective`, `objective_bare`),
/// the global placement's density overflow at each stage's poses
/// (at the placement's level, without halos, and at the global
/// placement's own: `global_overflow_at`, `_bare`, `_global`),
/// and per movable part each stage's pose, how far each stage moved it and
/// what it met where the annealer left it. Each seed says its seed
/// number and whether it is the placement kept (the first). The stages
/// are the placement's (`core::Placement::stage_poses`: the global
/// placement's `start` first, the kept skeleton's `skeleton_*` before
/// them under the race over skeletons), and `global_history` holds each
/// global placement's descent every ten iterations (`iteration`,
/// `overflow`, `lambda`, `gamma`, the wire gradient's and the weighted
/// density gradient's absolute sums, the wirelength). A seed of the race
/// over skeletons says in `skeleton` its phases' levels and work, whether
/// it fell back to the plain placement and which parts the skeleton or the
/// passives left without a legal place; `walk` the relaxation levels the
/// placement ran (the passives' around a held skeleton), each with the
/// parts it left without room.
pub(super) fn trace_placements(
    lowered: &LoweredPlacement,
    placements: &[core::Placement],
    seeds: &[u64],
    config: &core::Config,
    path: &Path,
) -> Result<(), String> {
    // The global placement's own problem (the fitted halos of its level).
    let global_problem = core::global_problem(&lowered.problem, config);
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
    let mut traced = Vec::new();
    for (rank, placement) in placements.iter().enumerate() {
        let mut relaxed = base.clone();
        placement.relaxation.apply(&mut relaxed);
        // The level without halos: what the arrangement costs where the
        // routing room is not counted.
        let mut bare = relaxed.clone();
        for component in &mut bare.components {
            component.halo = 0.0;
        }
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
        let objectives = |problem: &core::Problem| -> serde_json::Map<String, serde_json::Value> {
            let mut result: serde_json::Map<String, serde_json::Value> = stages
                .iter()
                .map(|(name, poses)| {
                    (
                        name.to_string(),
                        objective_json(&core::anneal::objective(problem, poses)),
                    )
                })
                .collect();
            if let Some(poses) = &designer {
                result.insert(
                    "designer".to_string(),
                    objective_json(&core::anneal::objective(problem, poses)),
                );
            }
            result
        };
        // The global placement's density overflow (its stop test, 0.04)
        // at each stage's poses and the designer's, at the level and
        // without halos: what the global's model calls a legal placement.
        let overflows = |problem: &core::Problem| -> serde_json::Map<String, serde_json::Value> {
            let at = |poses: &[core::Pose]| -> f64 {
                let mut placed = problem.clone();
                placed.poses = poses.to_vec();
                let config = core::GlobalConfig {
                    max_iterations: 1,
                    frame_interval: 0,
                    warm: Some(std::sync::Arc::new(vec![true; count])),
                    ..core::GlobalConfig::default()
                };
                core::global::global_place(&placed, &config).overflow
            };
            let mut result: serde_json::Map<String, serde_json::Value> = stages
                .iter()
                .map(|(name, poses)| (name.to_string(), serde_json::json!(at(poses))))
                .collect();
            if let Some(poses) = &designer {
                result.insert("designer".to_string(), serde_json::json!(at(poses)));
            }
            result
        };
        let global_overflow = overflows(&relaxed);
        let global_overflow_global = overflows(&global_problem);
        let global_overflow_bare = overflows(&bare);
        let objective = objectives(&relaxed);
        let objective_bare = objectives(&bare);
        let stage_of = |name: &str| stages.iter().position(|(stage, _)| *stage == name);
        // What the annealer itself sees there (a conflict it does not see
        // is a mismatch of the two models).
        let anneal_overlaps =
            stage_of("annealed").map(|stage| core::anneal::overlaps(&relaxed, stages[stage].1));
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
                "poses": stages.iter().map(|(_, poses)| pose_json(poses[index])).collect::<Vec<_>>(),
                "net_cost": lengths.iter().map(|(_, lengths)| cost(lengths)).collect::<Vec<_>>(),
            });
            if let (Some(poses), Some(lengths)) = (&designer, &designer_lengths) {
                entry["designer_center"] = serde_json::json!(component.center(poses[index]));
                entry["designer_pose"] = pose_json(poses[index]);
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
        traced.push(serde_json::json!({
            "seed": seeds.get(rank),
            "kept": rank == 0,
            "objective": objective,
            "objective_bare": objective_bare,
            "global_overflow_at": global_overflow,
            "global_overflow_at_bare": global_overflow_bare,
            "global_overflow_at_global": global_overflow_global,
            "stages": stages.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            "wirelength": lengths.iter().map(|(_, lengths)| lengths.iter().sum::<f64>()).collect::<Vec<_>>(),
            "designer_wirelength": designer_lengths.as_ref().map(|lengths| lengths.iter().sum::<f64>()),
            "unplaced": placement.unplaced.len(),
            "illegal": placement.illegal.len(),
            "work": placement.work,
            "walk": walk_json(&placement.walk, &lowered.references),
            "global_iterations": placement.global_iterations,
            "skeleton": placement.skeleton.as_ref().map(|report| {
                let names = |parts: &[usize]| -> Vec<&str> {
                    parts.iter().map(|part| lowered.references[*part].as_str()).collect()
                };
                serde_json::json!({
                    "skeleton_level": report.skeleton_level,
                    "level": report.level,
                    "fitting_level": report.fitting_level,
                    "skeleton_work": report.skeleton_work,
                    "passive_work": report.passive_work,
                    "fallback_work": report.fallback_work,
                    "fallback": report.fallback,
                    "fallback_kept": report.fallback_kept,
                    "skeleton_left_out": names(&report.skeleton_left_out),
                    "passives_left_out": names(&report.passives_left_out),
                    "skeleton_walk": walk_json(&report.skeleton_walk, &lowered.references),
                })
            }),
            "global_overflow": placement.global_overflow,
            "global_history": placement
                .global_histories
                .iter()
                .map(|(name, steps)| {
                    (
                        name.to_string(),
                        serde_json::json!(steps
                            .iter()
                            .map(|step| [
                                step.iteration as f64,
                                step.overflow,
                                step.lambda,
                                step.gamma,
                                step.wire,
                                step.density,
                                step.wirelength,
                            ])
                            .collect::<Vec<_>>()),
                    )
                })
                .collect::<serde_json::Map<String, serde_json::Value>>(),
            "relaxation": {
                "spacing": relaxation.spacing, "grid": relaxation.grid, "halo_scale": relaxation.halo_scale,
                "small_halo_scale": relaxation.small_halo_scale, "edge_inset": relaxation.edge_inset,
                "tight": relaxation.tight, "courtyard_spacing": relaxation.courtyard_spacing,
            },
            "crowding": crowding,
            "parts": parts,
        }));
    }
    let text = serde_json::to_string(&serde_json::json!({"seeds": traced}))
        .map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
}

/// A placement's level walk (`core::Placement::walk`) as JSON: per level
/// run, `[level, [the parts it left without room], the work when it ended
/// in seconds]`.
fn walk_json(walk: &[(usize, Vec<usize>, u64)], references: &[String]) -> Vec<serde_json::Value> {
    walk.iter()
        .map(|(level, failed, work)| {
            let names: Vec<&str> = failed
                .iter()
                .map(|part| references[*part].as_str())
                .collect();
            serde_json::json!([level, names, *work as f64 / core::work_per_second()])
        })
        .collect()
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

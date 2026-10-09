// Copyright (C) 2026 Toit contributors.

//! Training samples for the congestion model (`docs/congestion-model.md`):
//! a board under several placements, each rasterized on the router's tiles
//! (`pcb_congestion::features`) and probed by the router for a fixed amount
//! of work, as the layout's placement race probes its candidates; the
//! probe's state on the tiles is the label.
//!
//! Each sample is `<name>.json` (header: board, tile grid, channel names,
//! scalars, and per array its offset, shape and type) and `<name>.bin`
//! (the arrays, little-endian `f32`).

use super::*;
use crate::board_layout::{footprint_items, without_copper_texts};
use crate::board_placer::{largest_clearance, lower_placement, write_footprint_pose};
use crate::board_router::{KiCadPourMode, LayerTable, add_pour_zones, core_config, lower, planned_pours, pours};
use crate::placement_constraints::resolve_constraints;
use pcb_placer as placer;
use pcb_router as core;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadCongestionJob {
    /// Work each probe may do, in seconds of an idle machine.
    pub probe_seconds: f64,
    /// Placements to sample, in order; a variant may start from an earlier
    /// one (`base`).
    pub variants: Vec<KiCadCongestionVariant>,
    /// The layout configuration whose placer places the `placer_seed`
    /// variants (constraints relative to the source directory).
    pub layout: Option<KiCadBoardLayoutConfig>,
    /// Placement work a placer variant may do, in seconds of an idle
    /// machine.
    pub placement_seconds: f64,
    /// Write the placed boards next to the samples.
    pub keep_boards: bool,
}

impl Default for KiCadCongestionJob {
    fn default() -> Self {
        Self {
            probe_seconds: 20.0,
            variants: vec![KiCadCongestionVariant { name: "source".into(), ..Default::default() }],
            layout: None,
            placement_seconds: 60.0,
            keep_boards: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadCongestionVariant {
    pub name: String,
    /// The variant this one starts from (default: the source placement).
    pub base: Option<String>,
    /// Placed by our placer with this seed (from the source, with the
    /// job's layout configuration).
    pub placer_seed: Option<u64>,
    /// Random legal moves, swaps and turns of movable parts.
    pub perturb: Option<KiCadPerturbation>,
    /// Every movable part at a random spot, then legalized: a bad
    /// placement on purpose.
    pub shuffle: Option<u64>,
    /// Features only (no probe, no labels).
    pub skip_probe: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadPerturbation {
    pub moves: usize,
    pub swaps: usize,
    pub rotations: usize,
    /// Largest move, in millimetres.
    pub max_mm: f64,
    pub seed: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadCongestionSampleReport {
    pub name: String,
    pub unfinished: Option<usize>,
    pub wirelength_mm: f64,
    pub seconds_lower: f64,
    pub seconds_features: f64,
    pub seconds_probe: f64,
    pub error: Option<String>,
}

/// A small xorshift generator: samples must be reproducible from their seed.
struct Random(u64);

impl Random {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, count: usize) -> usize {
        (self.next() % count.max(1) as u64) as usize
    }
}

/// The placement problem perturbations are judged in: the source's bodies
/// without halos at the smallest spacing (a designer packs tighter than
/// the placer's comfort margins).
fn perturbation_problem(pcb: &Expr, placer_config: &KiCadBoardPlacerConfig) -> Result<placer::Problem, String> {
    let mut problem = lower_placement(pcb, placer_config, &[])?.problem;
    problem.spacing = problem.min_spacing;
    for component in &mut problem.components {
        component.halo = 0.0;
    }
    Ok(problem)
}

fn movable(problem: &placer::Problem) -> Vec<usize> {
    (0..problem.components.len())
        .filter(|index| !problem.components[*index].fixed && !problem.components[*index].pins.is_empty())
        .collect()
}

fn legal_at(problem: &placer::Problem, poses: &[placer::Pose], index: usize, pose: placer::Pose) -> bool {
    placer::legal::is_legal(problem, poses, index, pose, 0..problem.components.len())
}

/// Random legal moves, swaps and quarter turns.
fn perturb(problem: &placer::Problem, poses: &mut [placer::Pose], spec: &KiCadPerturbation) {
    let parts = movable(problem);
    if parts.is_empty() {
        return;
    }
    let mut random = Random::new(spec.seed.wrapping_add(17));
    let tries = 40;
    for _ in 0..spec.moves {
        for _ in 0..tries {
            let index = parts[random.below(parts.len())];
            let radius = spec.max_mm * random.unit().sqrt();
            let angle = random.unit() * std::f64::consts::TAU;
            let mut pose = poses[index];
            pose.position = [pose.position[0] + radius * angle.cos(), pose.position[1] + radius * angle.sin()];
            if legal_at(problem, poses, index, pose) {
                poses[index] = pose;
                break;
            }
        }
    }
    for _ in 0..spec.rotations {
        for _ in 0..tries {
            let index = parts[random.below(parts.len())];
            let options = &problem.components[index].angle_options;
            if options.len() < 2 {
                continue;
            }
            let mut pose = poses[index];
            let angle = options[random.below(options.len())];
            if (angle - pose.angle).rem_euclid(360.0) < 1.0e-6 {
                continue;
            }
            // Turn about the body centre, not the footprint origin.
            let center = problem.components[index].center(pose);
            pose.angle = angle;
            pose.position = problem.components[index].position_for_center(center, angle);
            if legal_at(problem, poses, index, pose) {
                poses[index] = pose;
                break;
            }
        }
    }
    for _ in 0..spec.swaps {
        for _ in 0..tries {
            let a = parts[random.below(parts.len())];
            let b = parts[random.below(parts.len())];
            let (first, second) = (&problem.components[a], &problem.components[b]);
            if a == b || first.side != second.side {
                continue;
            }
            let area = |component: &placer::Component| component.body_size[0] * component.body_size[1];
            let ratio = area(first) / area(second).max(1.0e-9);
            if !(0.5..=2.0).contains(&ratio) {
                continue;
            }
            let (center_a, center_b) = (first.center(poses[a]), second.center(poses[b]));
            let pose_a = placer::Pose { position: first.position_for_center(center_b, poses[a].angle), angle: poses[a].angle };
            let pose_b = placer::Pose { position: second.position_for_center(center_a, poses[b].angle), angle: poses[b].angle };
            let mut trial = poses.to_vec();
            trial[a] = pose_a;
            trial[b] = pose_b;
            if legal_at(problem, &trial, a, pose_a) && legal_at(problem, &trial, b, pose_b) {
                poses[a] = pose_a;
                poses[b] = pose_b;
                break;
            }
        }
    }
}

/// Every movable part at a random spot of the board's bounds, then
/// legalized (parts that find no spot stay where they landed).
fn shuffle(problem: &placer::Problem, poses: &mut [placer::Pose], seed: u64) {
    let mut random = Random::new(seed.wrapping_add(91));
    let bounds = problem.bounds();
    for index in movable(problem) {
        let options = &problem.components[index].angle_options;
        let angle = if options.is_empty() { poses[index].angle } else { options[random.below(options.len())] };
        let center = [
            bounds[0] + random.unit() * (bounds[2] - bounds[0]),
            bounds[1] + random.unit() * (bounds[3] - bounds[1]),
        ];
        poses[index] = placer::Pose { position: problem.components[index].position_for_center(center, angle), angle };
    }
    let mut relaxed = problem.clone();
    relaxed.grid = relaxed.grid.min(0.1);
    placer::legal::legalize(&relaxed, poses);
}

/// The source board without routed copper (tracks, arcs, vias).
fn stripped(pcb: &Expr) -> Expr {
    let Expr::List(items) = pcb else {
        return pcb.clone();
    };
    Expr::List(items.iter().filter(|item| !matches!(item.head(), Some("segment" | "arc" | "via"))).cloned().collect())
}

fn with_poses(pcb: &Expr, poses: &[placer::Pose]) -> Result<Expr, String> {
    let mut placed = pcb.clone();
    let mut footprints = footprint_items(&mut placed)?;
    if footprints.len() != poses.len() {
        return Err(format!("{} footprints, {} poses", footprints.len(), poses.len()));
    }
    for (footprint, pose) in footprints.iter_mut().zip(poses) {
        write_footprint_pose(footprint, *pose)?;
    }
    Ok(placed)
}

/// Arrays of one sample, written after a JSON header.
struct Blob {
    arrays: Vec<(String, Vec<usize>, Vec<f32>)>,
}

impl Blob {
    fn add(&mut self, name: &str, shape: Vec<usize>, values: Vec<f32>) {
        debug_assert_eq!(shape.iter().product::<usize>(), values.len());
        self.arrays.push((name.into(), shape, values));
    }

    fn write(&self, path: &Path, header: &mut serde_json::Value) -> Result<(), String> {
        let mut bytes = Vec::new();
        let mut index = serde_json::Map::new();
        for (name, shape, values) in &self.arrays {
            index.insert(
                name.clone(),
                serde_json::json!({"offset": bytes.len(), "shape": shape, "dtype": "<f4"}),
            );
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        header["arrays"] = serde_json::Value::Object(index);
        fs::write(path.with_extension("bin"), &bytes).map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        fs::write(path.with_extension("json"), serde_json::to_string(header).map_err(|error| error.to_string())?)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))
    }
}

/// The router configuration the layout's placement race probes with.
pub(crate) fn race_core_config(router_config: &KiCadBoardRouterConfig) -> core::Config {
    let mut core_config = core_config(router_config);
    if router_config.stall_drop.is_none() {
        core_config.stall_drop = 0.1;
    }
    if router_config.stall_at_cap.is_none() {
        core_config.stall_at_cap = core_config.stall_at_cap.min(10);
    }
    if router_config.present_growth.is_none() {
        core_config.present_growth = core_config.present_growth.max(2.0);
    }
    core_config.stall_patience = core_config.stall_patience.min(8);
    core_config
}

/// Writes one sample per variant of the job to `output_directory`.
pub fn export_congestion_samples(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    job: &KiCadCongestionJob,
    router_config: &KiCadBoardRouterConfig,
) -> Result<Vec<KiCadCongestionSampleReport>, String> {
    fs::create_dir_all(output_directory).map_err(|error| format!("failed to create {}: {error}", output_directory.display()))?;
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&source_board).map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let source = stripped(&parse(&text)?);
    let layout = job.layout.clone().unwrap_or_default();
    let mut placer_config = resolve_constraints(&layout.placer, source_directory)?;
    if placer_config.copper_clearance_mm.is_none() {
        placer_config.copper_clearance_mm = Some(largest_clearance(router_config));
    }
    placer_config.edge_margin_mm = placer_config.edge_margin_mm.max(router_config.edge_clearance_mm);
    if placer_config.copper_edge_clearance_mm.is_none() {
        placer_config.copper_edge_clearance_mm = Some(router_config.edge_clearance_mm);
    }
    // Perturbations move the designer's parts too: only locked parts,
    // parts without nets and edge connectors stay.
    let perturb_config = KiCadBoardPlacerConfig { constraints: None, ..KiCadBoardPlacerConfig::default() };
    let mut core_config = race_core_config(router_config);
    core_config.verbose = false;
    let mut placements: BTreeMap<String, Expr> = BTreeMap::new();
    let mut reports = Vec::new();
    for variant in &job.variants {
        let started = std::time::Instant::now();
        let outcome = (|| -> Result<KiCadCongestionSampleReport, String> {
            let base = match &variant.base {
                Some(name) => placements.get(name).cloned().ok_or_else(|| format!("no variant {name:?} before this one"))?,
                None => source.clone(),
            };
            let mut pcb = if let Some(seed) = variant.placer_seed {
                let directory = output_directory.join(format!(".place-{}", variant.name));
                if directory.exists() {
                    fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
                }
                let config = KiCadBoardPlacerConfig {
                    seed,
                    placement_seeds: 1,
                    work_seconds: Some(job.placement_seconds),
                    ..placer_config.clone()
                };
                let placed = place_kicad_board(source_directory, board_id, &directory, &config);
                let board = directory.join(format!("{board_id}.kicad_pcb"));
                let text = fs::read_to_string(&board);
                let _ = fs::remove_dir_all(&directory);
                let placed = placed?;
                if !placed.unplaced.is_empty() || !placed.illegal.is_empty() {
                    eprintln!(
                        "{}: placement not legal ({} unplaced, {} illegal); sampled anyway",
                        variant.name,
                        placed.unplaced.len(),
                        placed.illegal.len()
                    );
                }
                stripped(&parse(&text.map_err(|error| error.to_string())?)?)
            } else {
                base
            };
            if variant.perturb.is_some() || variant.shuffle.is_some() {
                let problem = perturbation_problem(&pcb, &perturb_config)?;
                let mut poses = problem.poses.clone();
                if let Some(seed) = variant.shuffle {
                    shuffle(&problem, &mut poses, seed);
                }
                if let Some(spec) = &variant.perturb {
                    perturb(&problem, &mut poses, spec);
                }
                pcb = with_poses(&pcb, &poses)?;
            }
            placements.insert(variant.name.clone(), pcb.clone());
            let wirelength_mm = {
                let problem = lower_placement(&pcb, &perturb_config, &[])?.problem;
                problem.wirelength(&problem.poses)
            };
            // As the layout routes a placement: planned pours added.
            let mut routed = pcb.clone();
            let planned = planned_pours(&routed, router_config)?;
            add_pour_zones(&mut routed, &planned)?;
            let layers = LayerTable::from_pcb(&routed)?;
            let connect = !pours(&routed, &layers)?.is_empty() && router_config.pours != KiCadPourMode::Tracks;
            let lowered_at = std::time::Instant::now();
            let board = lower(&without_copper_texts(&routed), router_config, connect)?.board;
            let seconds_lower = lowered_at.elapsed().as_secs_f64();
            let features_at = std::time::Instant::now();
            let frame = core::router::congestion::tile_frame(&board, &core_config);
            let maps = pcb_congestion::rasterize(&board, &frame);
            let seconds_features = features_at.elapsed().as_secs_f64();
            let tiles = [frame.tiles_y, frame.tiles_x];
            let mut blob = Blob { arrays: Vec::new() };
            blob.add("features", vec![pcb_congestion::channel_count(), tiles[0], tiles[1]], maps.data.clone());
            let mut header = serde_json::json!({
                "board_id": board_id,
                "source": source_directory,
                "variant": variant,
                "layers": board.layer_count,
                "layer_names": layers.names,
                "frame": {
                    "origin": frame.origin, "pitch": frame.pitch, "nx": frame.nx, "ny": frame.ny,
                    "tile": frame.tile, "tiles_x": frame.tiles_x, "tiles_y": frame.tiles_y,
                },
                "channels": pcb_congestion::channel_names(),
                "nets": board.nets.len(),
                "terminals": board.nets.iter().map(|net| net.terminals.len()).sum::<usize>(),
                "wirelength_mm": wirelength_mm,
                "probe_seconds": job.probe_seconds,
                "seconds_lower": seconds_lower,
                "seconds_features": seconds_features,
            });
            let mut unfinished = None;
            let mut seconds_probe = 0.0;
            if !variant.skip_probe {
                let probe_at = std::time::Instant::now();
                let mut router = core::router::Router::new(&board, &core_config);
                let probed = router.probe(job.probe_seconds);
                let labels = router.congestion_labels();
                drop(router);
                seconds_probe = probe_at.elapsed().as_secs_f64();
                if labels.frame.tiles_x != frame.tiles_x
                    || labels.frame.tiles_y != frame.tiles_y
                    || (labels.frame.pitch - frame.pitch).abs() > 1.0e-12
                {
                    return Err(format!("the router's tiles differ from the features' ({:?} against {:?})", labels.frame, frame));
                }
                let planes = board.layer_count + 1;
                blob.add("conflict", vec![planes, tiles[0], tiles[1]], labels.conflict);
                blob.add("history", vec![planes, tiles[0], tiles[1]], labels.history);
                blob.add("tile_history", vec![planes, tiles[0], tiles[1]], labels.tile_history);
                blob.add("usage", vec![planes, tiles[0], tiles[1]], labels.usage);
                blob.add("claimed", vec![board.layer_count, tiles[0], tiles[1]], labels.claimed);
                blob.add("routable", vec![board.layer_count, tiles[0], tiles[1]], labels.routable);
                blob.add("open_terminals", vec![1, tiles[0], tiles[1]], labels.open_terminals);
                let flags = |values: &[bool]| values.iter().map(|value| *value as u8 as f32).collect::<Vec<f32>>();
                let nets = labels.net_routable.len();
                blob.add("net_routable", vec![nets], flags(&labels.net_routable));
                blob.add("net_complete", vec![nets], flags(&labels.net_complete));
                blob.add("net_conflicted", vec![nets], flags(&labels.net_conflicted));
                let routable = labels.net_routable.iter().filter(|value| **value).count();
                let incomplete = (0..nets).filter(|net| labels.net_routable[*net] && !labels.net_complete[*net]).count();
                let conflicted = (0..nets).filter(|net| labels.net_routable[*net] && labels.net_conflicted[*net]).count();
                header["probe"] = serde_json::json!({
                    "unfinished": probed,
                    "routable_nets": routable,
                    "incomplete_nets": incomplete,
                    "conflicted_nets": conflicted,
                    "iterations": labels.iterations,
                    "expansions": labels.expansions,
                    "seconds": seconds_probe,
                });
                header["net_names"] = serde_json::json!(board.nets.iter().map(|net| net.name.clone()).collect::<Vec<_>>());
                unfinished = Some(probed);
            }
            blob.write(&output_directory.join(&variant.name), &mut header)?;
            if job.keep_boards {
                fs::write(output_directory.join(format!("{}.kicad_pcb", variant.name)), format!("{}\n", encode(&pcb)))
                    .map_err(|error| error.to_string())?;
            }
            Ok(KiCadCongestionSampleReport {
                name: variant.name.clone(),
                unfinished,
                wirelength_mm,
                seconds_lower,
                seconds_features,
                seconds_probe,
                error: None,
            })
        })();
        let report = outcome.unwrap_or_else(|error| KiCadCongestionSampleReport {
            name: variant.name.clone(),
            unfinished: None,
            wirelength_mm: 0.0,
            seconds_lower: 0.0,
            seconds_features: 0.0,
            seconds_probe: 0.0,
            error: Some(error),
        });
        eprintln!(
            "sample {}: {} ({:.1} s)",
            report.name,
            match (&report.error, report.unfinished) {
                (Some(error), _) => format!("error: {error}"),
                (None, Some(unfinished)) => format!("{unfinished} unfinished"),
                (None, None) => "features only".into(),
            },
            started.elapsed().as_secs_f64()
        );
        reports.push(report);
    }
    Ok(reports)
}

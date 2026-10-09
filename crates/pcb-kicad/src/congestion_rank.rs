// Copyright (C) 2026 Toit contributors.

//! Ranks placements of one board with the congestion model
//! (`docs/congestion-model.md`): each is lowered as the router would see
//! it, rasterized on the router's tiles, and scored by the network.

use super::*;
use crate::board_layout::{footprint_items, without_copper_texts};
use crate::board_placer::write_footprint_pose;
use crate::board_router::lower;
use pcb_placer as placer;
use pcb_router as core;

pub(crate) struct Ranking {
    /// Per placement, lower is better.
    pub scores: Vec<f64>,
    /// Placement indices, best first (ties to the earlier one).
    pub order: Vec<usize>,
    pub seconds: f64,
    pub lower_seconds: f64,
    pub feature_seconds: f64,
    pub inference_seconds: f64,
}

/// One placement's score: `open` (predicted unfinished nets) or
/// `overflow` (predicted overflow summed over the board).
/// `rudy`: RUDY demand above capacity (no model); `open+rudy`: the sum of
/// the two scores' ranks among the placements, done by `rank_placements`.
pub(crate) fn score_board(
    board: &core::Board,
    model: Option<&pcb_congestion::CongestionModel>,
    core_config: &core::Config,
    score: &str,
) -> Result<(f64, f64, f64, f64), String> {
    let started = std::time::Instant::now();
    let frame = core::router::congestion::tile_frame(board, core_config);
    let maps = pcb_congestion::rasterize(board, &frame);
    let rudy = pcb_congestion::rudy_overflow(&maps, 0.25) as f64;
    let features = started.elapsed().as_secs_f64();
    let inferred = std::time::Instant::now();
    let value = match (score, model) {
        ("rudy", _) => rudy,
        (_, None) => return Err(format!("congestion score {score:?} needs a congestion_model")),
        ("overflow", Some(model)) => model.predict(&maps)?.overflow_sum() as f64,
        (_, Some(model)) => model.predict(&maps)?.open as f64,
    };
    Ok((value, rudy, features, inferred.elapsed().as_secs_f64()))
}

/// Per value, its rank among `values` (0 for the lowest; ties share the
/// lower rank).
fn ranks(values: &[f64]) -> Vec<f64> {
    values.iter().map(|value| values.iter().filter(|other| **other < *value).count() as f64).collect()
}

pub(crate) fn rank_placements(
    pcb: &Expr,
    placements: &[(Vec<placer::Pose>, placer::Relaxation)],
    model: Option<&pcb_congestion::CongestionModel>,
    router_config: &KiCadBoardRouterConfig,
    core_config: &core::Config,
    connect: bool,
    score: &str,
) -> Result<Ranking, String> {
    let started = std::time::Instant::now();
    let (mut lower_seconds, mut feature_seconds, mut inference_seconds) = (0.0, 0.0, 0.0);
    let mut scores = Vec::with_capacity(placements.len());
    let mut rudies = Vec::with_capacity(placements.len());
    for (poses, _) in placements {
        let lowered_at = std::time::Instant::now();
        let mut candidate = pcb.clone();
        {
            let mut footprints = footprint_items(&mut candidate)?;
            for (footprint, pose) in footprints.iter_mut().zip(poses) {
                write_footprint_pose(footprint, *pose)?;
            }
        }
        let board = lower(&without_copper_texts(&candidate), router_config, connect)?.board;
        lower_seconds += lowered_at.elapsed().as_secs_f64();
        let (value, rudy, features, inference) = score_board(&board, model, core_config, score)?;
        feature_seconds += features;
        inference_seconds += inference;
        scores.push(value);
        rudies.push(rudy);
    }
    if score == "open+rudy" {
        let (model_ranks, rudy_ranks) = (ranks(&scores), ranks(&rudies));
        scores = model_ranks.iter().zip(&rudy_ranks).map(|(a, b)| a + b).collect();
    }
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|a, b| scores[*a].total_cmp(&scores[*b]).then(a.cmp(b)));
    Ok(Ranking { scores, order, seconds: started.elapsed().as_secs_f64(), lower_seconds, feature_seconds, inference_seconds })
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadCongestionScore {
    pub open: f64,
    pub overflow: f64,
    pub tiles: [usize; 2],
    pub lower_seconds: f64,
    pub feature_seconds: f64,
    /// The first inference builds the plan for the board's size.
    pub first_inference_seconds: f64,
    /// Later inferences on the same size.
    pub inference_seconds: f64,
}

/// Scores a board as placed (tracks ignored) with a congestion model, and
/// how long each step takes.
pub fn score_kicad_congestion(
    source_directory: &Path,
    board_id: &str,
    model_path: &Path,
    router_config: &KiCadBoardRouterConfig,
) -> Result<KiCadCongestionScore, String> {
    let board_path = source_directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&board_path).map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let pcb = parse(&text)?;
    let Expr::List(items) = &pcb else {
        return Err("PCB root is not a list".into());
    };
    let pcb = Expr::List(items.iter().filter(|item| !matches!(item.head(), Some("segment" | "arc" | "via"))).cloned().collect());
    let layers = crate::board_router::LayerTable::from_pcb(&pcb)?;
    let connect = !crate::board_router::pours(&pcb, &layers)?.is_empty();
    let model = pcb_congestion::CongestionModel::load(model_path)?;
    let core_config = crate::congestion_samples::race_core_config(router_config);
    let lowered_at = std::time::Instant::now();
    let board = lower(&without_copper_texts(&pcb), router_config, connect)?.board;
    let lower_seconds = lowered_at.elapsed().as_secs_f64();
    let started = std::time::Instant::now();
    let frame = core::router::congestion::tile_frame(&board, &core_config);
    let maps = pcb_congestion::rasterize(&board, &frame);
    let feature_seconds = started.elapsed().as_secs_f64();
    let first_at = std::time::Instant::now();
    model.predict(&maps)?;
    let first_inference_seconds = first_at.elapsed().as_secs_f64();
    let again = std::time::Instant::now();
    let runs = 5;
    let mut prediction = None;
    for _ in 0..runs {
        prediction = Some(model.predict(&maps)?);
    }
    let prediction = prediction.unwrap();
    Ok(KiCadCongestionScore {
        open: prediction.open as f64,
        overflow: prediction.overflow_sum() as f64,
        tiles: [frame.tiles_x, frame.tiles_y],
        lower_seconds,
        feature_seconds,
        first_inference_seconds,
        inference_seconds: again.elapsed().as_secs_f64() / runs as f64,
    })
}

/// Predicts on a sample's stored features (`export-congestion-sample`),
/// to compare tract's output with PyTorch's.
pub fn score_congestion_sample(sample_json: &Path, model_path: &Path) -> Result<(f64, f64, Vec<f64>), String> {
    let header: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(sample_json).map_err(|error| format!("failed to read {}: {error}", sample_json.display()))?,
    )
    .map_err(|error| error.to_string())?;
    let entry = &header["arrays"]["features"];
    let shape: Vec<usize> = entry["shape"].as_array().ok_or("no features")?.iter().map(|v| v.as_u64().unwrap_or(0) as usize).collect();
    let offset = entry["offset"].as_u64().unwrap_or(0) as usize;
    let bytes = fs::read(sample_json.with_extension("bin")).map_err(|error| error.to_string())?;
    let count: usize = shape.iter().product();
    let data: Vec<f32> = bytes[offset..offset + 4 * count]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    let model = pcb_congestion::CongestionModel::load(model_path)?;
    let prediction = model.predict(&pcb_congestion::FeatureMaps { tiles_x: shape[2], tiles_y: shape[1], data })?;
    let size = prediction.tiles_x * prediction.tiles_y;
    let planes = (0..prediction.planes).map(|plane| prediction.overflow[plane * size..(plane + 1) * size].iter().map(|v| *v as f64).sum()).collect();
    Ok((prediction.open as f64, prediction.overflow_sum() as f64, planes))
}

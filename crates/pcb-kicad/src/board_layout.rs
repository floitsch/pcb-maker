// Copyright (C) 2026 Toit contributors.

//! Coupled placement and routing.
//!
//! The router is the judge of a placement, and its congestion history says
//! where the placement starved it of room. Footprints sitting in congested
//! regions get larger routing halos and the board is placed again, until it
//! routes completely or the rounds are used up. The best round wins.

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadBoardLayoutConfig {
    pub rounds: usize,
    /// Halo growth, in millimetres, for the most congested footprint of a
    /// round; others grow in proportion to their congestion.
    pub halo_growth_mm: f64,
    /// Every round may also claim this much more of the free board.
    pub utilization_growth: f64,
    /// Negotiation iterations granted to the router while probing.
    pub probe_iterations: usize,
    pub placer: KiCadBoardPlacerConfig,
}

impl Default for KiCadBoardLayoutConfig {
    fn default() -> Self {
        Self {
            rounds: 4,
            halo_growth_mm: 2.0,
            utilization_growth: 0.1,
            probe_iterations: 40,
            placer: KiCadBoardPlacerConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardLayoutRound {
    pub round: usize,
    pub wirelength_mm: f64,
    pub routed_connections: usize,
    pub unconnected_terminals: usize,
    pub vias: usize,
    pub length_mm: f64,
    pub placement_seconds: f64,
    pub routing_seconds: f64,
    pub most_congested: Vec<(String, f64)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardLayoutResult {
    pub board_id: String,
    pub selected_round: usize,
    pub rounds: Vec<KiCadBoardLayoutRound>,
    pub routed: KiCadBoardRouterResult,
}

/// Mean congestion under each footprint's body plus a margin.
fn footprint_congestion(
    placement: &KiCadBoardPlacerResult,
    routed: &KiCadBoardRouterResult,
    margin: f64,
) -> Vec<(String, f64)> {
    let pitch = routed.grid_pitch_mm;
    let [nx, ny] = routed.grid_nodes;
    placement
        .footprints
        .iter()
        .map(|footprint| {
            let range = |axis: usize, size: usize| {
                let low = footprint.body_center[axis] - footprint.body_half[axis] - margin;
                let high = footprint.body_center[axis] + footprint.body_half[axis] + margin;
                let first = ((low - routed.grid_origin[axis]) / pitch).ceil().max(0.0) as usize;
                let last = (((high - routed.grid_origin[axis]) / pitch).floor().max(0.0) as usize)
                    .min(size.saturating_sub(1));
                (first, last)
            };
            let (x0, x1) = range(0, nx);
            let (y0, y1) = range(1, ny);
            let mut total = 0.0;
            let mut nodes = 0usize;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    total += routed.congestion[y * nx + x] as f64;
                    nodes += 1;
                }
            }
            (
                footprint.reference.clone(),
                if nodes > 0 { total / nodes as f64 } else { 0.0 },
            )
        })
        .collect()
}

/// Places and routes `<source>/<board_id>.kicad_pcb`. Every round is kept
/// under `output/round-N`; the selected result is copied to `output/result`.
pub fn layout_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardLayoutConfig,
    router: &KiCadBoardRouterConfig,
) -> Result<KiCadBoardLayoutResult, String> {
    if output_directory.exists() {
        return Err(format!(
            "output directory {} already exists",
            output_directory.display()
        ));
    }
    fs::create_dir_all(output_directory)
        .map_err(|error| format!("failed to create {}: {error}", output_directory.display()))?;
    let mut placer = config.placer.clone();
    let mut probe = router.clone();
    probe.skip_native_verification = true;
    probe.maximum_iterations = Some(config.probe_iterations);

    let mut rounds = Vec::new();
    let mut best: Option<(usize, (usize, usize, f64))> = None;
    for round in 0..config.rounds.max(1) {
        let round_directory = output_directory.join(format!("round-{round}"));
        let placed_directory = round_directory.join("placed");
        let routed_directory = round_directory.join("routed");
        fs::create_dir_all(&round_directory)
            .map_err(|error| format!("failed to create {}: {error}", round_directory.display()))?;
        let placement = place_kicad_board(source_directory, board_id, &placed_directory, &placer)?;
        if !placement.unplaced.is_empty() || !placement.illegal.is_empty() {
            return Err(format!(
                "round {round}: placement is not legal (unplaced {:?}, illegal {:?})",
                placement.unplaced, placement.illegal
            ));
        }
        let routed = route_kicad_board(&placed_directory, board_id, &routed_directory, &probe)?;
        let mut congestion = footprint_congestion(&placement, &routed, 2.0);
        let score = (
            routed.unconnected_terminals + routed.internal_violations.len(),
            routed.vias,
            routed.length_mm,
        );
        let complete = score.0 == 0;
        if best.as_ref().is_none_or(|(_, best)| {
            (score.0, score.2 + 10.0 * score.1 as f64) < (best.0, best.2 + 10.0 * best.1 as f64)
        }) {
            best = Some((round, score));
        }
        let worst = congestion
            .iter()
            .map(|(_, value)| *value)
            .fold(0.0, f64::max);
        congestion.sort_by(|left, right| right.1.total_cmp(&left.1));
        rounds.push(KiCadBoardLayoutRound {
            round,
            wirelength_mm: placement.wirelength_final_mm,
            routed_connections: routed.routed_connections,
            unconnected_terminals: routed.unconnected_terminals,
            vias: routed.vias,
            length_mm: routed.length_mm,
            placement_seconds: placement.seconds,
            routing_seconds: routed.routing_seconds,
            most_congested: congestion.iter().take(5).cloned().collect(),
        });
        if complete && round > 0 {
            break;
        }
        if worst <= 0.0 {
            break;
        }
        // Congested footprints ask for more room next round.
        for footprint in &placement.footprints {
            let value = congestion
                .iter()
                .find(|(reference, _)| reference == &footprint.reference)
                .map_or(0.0, |(_, value)| *value);
            let halo = (footprint.halo_mm + config.halo_growth_mm * value / worst)
                .min(placer.maximum_halo_mm.max(footprint.halo_mm));
            placer
                .halo_overrides_mm
                .insert(footprint.reference.clone(), halo);
        }
        placer.maximum_utilization = (placer.maximum_utilization + config.utilization_growth).min(0.95);
    }

    let (selected_round, _) = best.expect("at least one round ran");
    let result_directory = output_directory.join("result");
    let placed_directory = output_directory.join(format!("round-{selected_round}/placed"));
    let routed = route_kicad_board(&placed_directory, board_id, &result_directory, router)?;
    let result = KiCadBoardLayoutResult {
        board_id: board_id.into(),
        selected_round,
        rounds,
        routed,
    };
    let report_path = output_directory.join("board-layout.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(result)
}

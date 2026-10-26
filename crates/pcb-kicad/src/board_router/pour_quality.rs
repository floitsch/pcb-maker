// Copyright (C) 2026 Toit contributors.

//! How whole the pours stayed on a routed board: per zone and layer, the
//! pieces the fill keeps, the largest one's share, the copper's coverage
//! and the other nets' tracks that cut it, from the router's own pour
//! model (`Router::pour_report`) on the board as written. The verdict
//! carries it (`pours`), board-router.json as `pour_quality`.

use super::*;
use pcb_router as core;

/// One pour (a zone's net on one layer) on a routed board.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct KiCadPourQuality {
    pub net: String,
    pub layer: String,
    /// The fill's pieces that hold copper of the net (KiCad removes the
    /// others as islands), slivers under `SLIVER_MM2` left out.
    pub pieces: usize,
    /// The largest piece's area over the pour's area on the layer (inside
    /// its outline and the board's, less the pours of a higher priority).
    pub largest_share: f64,
    /// Copper of the kept pieces over the pour's area.
    pub coverage: f64,
    /// The largest piece, the kept pieces and the pour's area, in mm².
    pub largest_mm2: f64,
    pub copper_mm2: f64,
    pub area_mm2: f64,
    /// Pieces without copper of the net, which KiCad removes.
    pub islands_removed: usize,
    /// The length of other nets' tracks on the layer (score-kicad-board's
    /// `cut_by_mm`): what cuts the pour.
    pub cut_mm: f64,
}

impl KiCadPourQuality {
    /// Whether the pour fell apart: more than `MANY_PIECES` pieces, or
    /// its largest piece holds less than half of its copper.
    pub fn in_many_pieces(&self) -> bool {
        self.pieces > MANY_PIECES
            || (self.copper_mm2 > 0.0 && self.largest_mm2 < 0.5 * self.copper_mm2)
    }

    /// One line: `GND on B.Cu: 37 pieces, the largest 41 % of the pour,
    /// copper on 27 %, cut by 2088 mm of other nets' tracks`.
    pub fn line(&self) -> String {
        format!(
            "{} on {}: {} pieces, the largest {:.0} % of the pour, copper on {:.0} %, cut by {:.0} mm of other nets' tracks",
            self.net,
            self.layer,
            self.pieces,
            self.largest_share * 100.0,
            self.coverage * 100.0,
            self.cut_mm
        )
    }
}

/// Past this many pieces a pour is "in many pieces" for the verdict's
/// next action (the fence's 4-layer GND plane: 3 pieces).
pub const MANY_PIECES: usize = 5;

/// Pieces smaller than this (mm²) are left out of the count.
pub const SLIVER_MM2: f64 = 0.25;

/// What a piece beyond the first adds to `pour_loss` (a share of a pour).
pub const PIECE_LOSS: f64 = 0.01;

/// The step `pour_rank` rounds `pour_loss` to: attempts closer than this
/// on their pours are ranked by their vias and copper.
pub const POUR_RANK_STEP: f64 = 0.02;

/// How much of their pours a board lost, summed over the pours: the share
/// of each pour's area outside its largest piece, and `PIECE_LOSS` for
/// every piece beyond the first. 0 for pours in one piece each that fill
/// their zone.
pub fn pour_loss(pours: &[KiCadPourQuality]) -> f64 {
    pours
        .iter()
        .map(|pour| {
            (1.0 - pour.largest_share).max(0.0) + PIECE_LOSS * pour.pieces.saturating_sub(1) as f64
        })
        .sum()
}

/// `pour_loss` in steps of `POUR_RANK_STEP`, for ranking attempts (0 when
/// not measured).
pub fn pour_rank(loss: Option<f64>) -> u32 {
    loss.map_or(0, |loss| (loss / POUR_RANK_STEP).round() as u32)
}

/// The pours of the routed board `directory/<board_id>.kicad_pcb`,
/// measured by the pour model: the board lowered with its pours connected
/// and its tracks and vias as copper, a router built on it (no routing).
/// `Ok(empty)` for a board without pours.
pub fn measure_pours(
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
) -> Result<Vec<KiCadPourQuality>, String> {
    measure_board_pours(directory, board_id, config).map(|(pours, _)| pours)
}

/// A pad the pour model finds starved on the routed board: KiCad's DRC
/// would report a starved thermal there (`Router::pour_report_with_starved`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct KiCadPredictedStarved {
    /// `REF.NUMBER`.
    pub pad: String,
    pub net: String,
    pub layer: String,
    /// Spokes the DRC counts, spokes into isolated islands, and the pad's
    /// tracks it counts as spokes.
    pub spokes: usize,
    pub islands: usize,
    pub tracks: usize,
}

/// `measure_pours` and the pads the same analyses find starved
/// (`KiCadPredictedStarved`), from one lowering and one router.
pub fn measure_board_pours(
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
) -> Result<(Vec<KiCadPourQuality>, Vec<KiCadPredictedStarved>), String> {
    let path = directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let pcb = crate::canonical_order::parse_board(&text)?;
    let layers = LayerTable::from_pcb(&pcb)?;
    if pours(&pcb, &layers)?.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let Lowered { board, .. } = lower(&pcb, config, true)?;
    // Other nets' tracks per layer (arcs by their chord).
    let mut cut: BTreeMap<(String, String), f64> = BTreeMap::new();
    let mut tracks: Vec<(String, Option<String>, f64)> = Vec::new();
    for item in pcb.children() {
        if !matches!(item.head(), Some("segment" | "arc")) {
            continue;
        }
        let (Ok(start), Ok(end)) = (form_xy(item, "start"), form_xy(item, "end")) else {
            continue;
        };
        tracks.push((
            form_atom(item, "layer", 1).unwrap_or("").to_string(),
            node_net(item).map(|raw| normalize_net(raw).to_string()),
            distance_squared(start, end).sqrt(),
        ));
    }
    // The parse is done with before the router is built (a big board's
    // parse is ten times its file).
    drop(pcb);
    drop(text);
    let mut router = core::router::Router::new(&board, &core_config(config));
    router.adopt_existing_copper();
    let (report, starved) = router.pour_report_with_starved();
    drop(router);
    let starved: Vec<KiCadPredictedStarved> = starved
        .into_iter()
        .map(|pad| KiCadPredictedStarved {
            pad: pad.label,
            net: board.nets[pad.net as usize].name.clone(),
            layer: layers.names[pad.layer].clone(),
            spokes: pad.spokes,
            islands: pad.islands,
            tracks: pad.tracks,
        })
        .collect();
    let mut result = Vec::new();
    for entry in report {
        let net = board.nets[entry.net as usize].name.clone();
        let layer = layers.names[entry.layer].clone();
        let cut_mm = *cut.entry((net.clone(), layer.clone())).or_insert_with(|| {
            tracks
                .iter()
                .filter(|(on, other, _)| *on == layer && other.as_ref().is_some_and(|o| *o != net))
                .map(|(_, _, length)| length)
                .sum()
        });
        // Slivers of a few lattice nodes are the lattice's, not the fill's
        // (the fence at 46 mm: nine of 0.1-0.2 mm² that KiCad's fill does
        // not have).
        let pieces: Vec<f64> = entry
            .pieces_mm2
            .iter()
            .copied()
            .filter(|area| *area >= SLIVER_MM2)
            .collect();
        let copper: f64 = pieces.iter().sum();
        let largest = pieces.first().copied().unwrap_or(0.0);
        let area = entry.inside_mm2.max(1.0e-9);
        // (A sum of nothing is -0.0.)
        let round = |value: f64, digits: i32| {
            let scale = 10f64.powi(digits);
            (value * scale).round() / scale + 0.0
        };
        result.push(KiCadPourQuality {
            net,
            layer,
            pieces: pieces.len(),
            largest_share: round((largest / area).min(1.0), 3),
            coverage: round((copper / area).min(1.0), 3),
            largest_mm2: round(largest, 1),
            copper_mm2: round(copper, 1),
            area_mm2: round(entry.inside_mm2, 1),
            islands_removed: entry.islands,
            cut_mm: round(cut_mm, 1),
        });
    }
    Ok((result, starved))
}

/// What the pour model measured on a board as written
/// (`measured_board`).
#[derive(Clone, Debug, Default)]
pub(crate) struct MeasuredBoard {
    pub pours: Vec<KiCadPourQuality>,
    /// The pads the model finds starved on the board as left.
    pub starved: Vec<KiCadPredictedStarved>,
    /// The pads it connected solid for them (`solidify`).
    pub solidified: Vec<String>,
}

/// `measure_board_pours` on `directory/<board_id>.kicad_pcb`, its error
/// in the log rather than the run's (the measure is a report, not a step
/// that can fail a board). With `solidify` (a board KiCad does not check,
/// whose project allows solid pads: `solid_starved_thermals`), the pads
/// the model finds starved get KiCad's solid connection, as
/// `solidify_starved_thermals` gives the pads KiCad finds starved, and
/// the board is measured again: the change is kept when the model then
/// finds fewer starved pads, else the board is restored. Counted as no
/// work, as the measure.
pub(crate) fn measured_board(
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
    solidify: bool,
) -> MeasuredBoard {
    let started = std::time::Instant::now();
    let mut measured = match measure_board_pours(directory, board_id, config) {
        Ok((pours, starved)) => MeasuredBoard {
            pours,
            starved,
            solidified: Vec::new(),
        },
        Err(error) => {
            eprintln!("pours not measured: {error}");
            return MeasuredBoard::default();
        }
    };
    if solidify && !measured.starved.is_empty() {
        match solidify_predicted(directory, board_id, config, &measured) {
            Ok(Some(better)) => measured = better,
            Ok(None) => {}
            Err(error) => eprintln!("predicted starved pads not connected solid: {error}"),
        }
    }
    for pour in &measured.pours {
        eprintln!("pour: {}", pour.line());
    }
    for pad in &measured.starved {
        eprintln!(
            "pour model: {} ({}) starved on {}: {} spokes, {} into islands, {} tracks",
            pad.pad, pad.net, pad.layer, pad.spokes, pad.islands, pad.tracks
        );
    }
    if !measured.pours.is_empty() {
        eprintln!(
            "pours measured in {:.1} s ({} pads the model finds starved)",
            started.elapsed().as_secs_f64(),
            measured.starved.len()
        );
    }
    measured
}

/// The predicted starved pads of `measured` solid in the board file, the
/// board measured again: the new measure when it has fewer starved pads
/// (the board left solid), else None (the board restored).
fn solidify_predicted(
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
    measured: &MeasuredBoard,
) -> Result<Option<MeasuredBoard>, String> {
    let pads: Vec<(String, String)> = measured
        .starved
        .iter()
        .filter_map(|pad| {
            let (reference, number) = pad.pad.split_once('.')?;
            Some((reference.to_string(), number.to_string()))
        })
        .collect();
    let path = directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut pcb = parse(&text)?;
    let changed = super::route::set_pads_solid(&mut pcb, &pads);
    if changed == 0 {
        return Ok(None);
    }
    fs::write(&path, format!("{}\n", encode(&pcb)))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    let (pours, starved) = measure_board_pours(directory, board_id, config)?;
    if starved.len() < measured.starved.len() {
        eprintln!(
            "{changed} pads the pour model finds starved connected solid: {} left",
            starved.len()
        );
        let mut solidified: Vec<String> = measured
            .starved
            .iter()
            .map(|pad| pad.pad.clone())
            .collect();
        solidified.sort();
        solidified.dedup();
        Ok(Some(MeasuredBoard {
            pours,
            starved,
            solidified,
        }))
    } else {
        fs::write(&path, text)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        eprintln!(
            "{changed} pads the pour model finds starved connected solid: not kept ({} starved after, {} before)",
            starved.len(),
            measured.starved.len()
        );
        Ok(None)
    }
}


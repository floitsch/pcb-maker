// Copyright (C) 2026 Toit contributors.

//! What a placement reports.

use super::*;
use pcb_placer as core;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPlacedFootprint {
    pub reference: String,
    pub fixed: bool,
    /// Halo requested for this footprint, before fitting to the board.
    pub halo_mm: f64,
    /// Final body centre and half extents on the board.
    pub body_center: [f64; 2],
    pub body_half: [f64; 2],
    pub source_at: [f64; 3],
    pub at: [f64; 3],
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardPlacerResult {
    pub board_id: String,
    pub components: usize,
    pub movable: usize,
    pub nets: usize,
    pub global_iterations: usize,
    pub global_overflow: f64,
    pub wirelength_source_mm: f64,
    pub wirelength_global_mm: f64,
    pub wirelength_legal_mm: f64,
    pub wirelength_final_mm: f64,
    pub unplaced: Vec<String>,
    pub illegal: Vec<String>,
    /// Every placement constraint with whether the placement keeps it.
    pub constraints: Vec<KiCadConstraintStatus>,
    pub constraint_warnings: Vec<String>,
    /// The board size a constraints outline set (automatic or given).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_mm: Option<[f64; 2]>,
    /// Share of the board the parts' bodies (with spacing) take, front and
    /// back.
    pub utilization: [f64; 2],
    /// Whether the placement needed bodies without their courtyard margin.
    pub tight_bodies: bool,
    /// The spacing, grid and halo factor the placement ended with, and
    /// whether bodies came closer to the edge than the full margin.
    pub spacing_mm: f64,
    pub grid_mm: f64,
    pub halo_scale: f64,
    /// The factor on the halos of parts with few pins (`halo_scale` is the
    /// many-pin parts').
    #[serde(default)]
    pub small_halo_scale: f64,
    pub edge_inset: bool,
    /// Parts held at an edge were placed by their copper alone (their
    /// courtyard may overhang the outline, as a card edge's does its tab).
    pub edge_copper: bool,
    /// Distance movable bodies kept from the edge (the placement's margin,
    /// or the rules' copper-to-edge clearance when nothing else fit).
    pub edge_rule_mm: f64,
    /// Courtyards came as close as their copper allows (touching).
    #[serde(default)]
    pub courtyard_spacing: bool,
    /// The parts that lie on the board's own texts or silkscreen graphics,
    /// having found no room off them at the placement's level
    /// (`core::Relaxation::on_marks`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts_on_marks: Vec<String>,
    /// The same parts by their index in the placement problem.
    #[serde(skip)]
    pub on_marks: Vec<usize>,
    /// The board's marks that belong to a movable part (on it at its input
    /// position, `attach_marks`) and so are no obstacles: the mark, its
    /// layer and anchor, and the part.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attached_marks: Vec<String>,
    /// The same, as (the mark's place among the board's marks, the part).
    #[serde(skip)]
    pub attached_mark_pairs: Vec<(usize, usize)>,
    /// Per footprint, the halo the placement gave it where routing room
    /// set the halos (`core::Relaxation::halos`).
    #[serde(skip)]
    pub halos: Option<std::sync::Arc<Vec<f64>>>,
    /// The other seeds' legal placements (poses, relaxation and how far
    /// they miss the constraints in all), next best first.
    #[serde(skip)]
    pub alternatives: Vec<(Vec<core::Pose>, core::Relaxation, f64)>,
    /// What to change when parts found no legal place.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    /// Room as numbers: utilization per side against the usable area, the
    /// area missing, the smallest outline that holds the parts and, when
    /// parts found no place, an outline to try (`placement_room`).
    pub room: crate::placement_room::KiCadRoom,
    /// Even the tightest bodies need more area than a side has: the
    /// placer said no at once.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub overfull: bool,
    /// How many seeds were kept (the work budget may stop them before
    /// `placement_seeds`).
    #[serde(default)]
    pub seeds: usize,
    /// The edges parts on `"edge": "any"` were given, by part.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub edges_chosen: BTreeMap<String, String>,
    pub seconds: f64,
    /// The most placement work a seed did, in seconds of an idle machine.
    #[serde(default)]
    pub work_seconds: f64,
    /// The kept seeds' work as the clock sees it: as the reference machine
    /// runs them, `REFERENCE_THREADS` at once in a pool (`race_seeds`), a
    /// seed starting as a thread frees. It depends on the seeds' work
    /// alone, not on the threads or the load.
    #[serde(default)]
    pub pool_work_seconds: f64,
    /// The kept seeds' work in all (as one after the other).
    #[serde(default)]
    pub seeds_work_seconds: f64,
    /// How many seeds ran: the kept ones and those a batch ran past the
    /// budget (they count for nothing).
    #[serde(default)]
    pub seeds_run: usize,
    /// The seed of the placement kept (`seed` plus its offset).
    #[serde(default)]
    pub seed_kept: u64,
    pub footprints: Vec<KiCadPlacedFootprint>,
}

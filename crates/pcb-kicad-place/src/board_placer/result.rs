// Copyright (C) 2026 Toit contributors.

//! What a placement reports.

use super::*;
use pcb_placer as core;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPlacedFootprint {
    pub reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    /// Mounting face as written in the board's footprint layer.
    pub side: &'static str,
    pub fixed: bool,
    /// Halo requested for this footprint, before fitting to the board.
    pub halo_mm: f64,
    /// Final body centre and half extents on the board.
    pub body_center: [f64; 2],
    pub body_half: [f64; 2],
    pub source_at: [f64; 3],
    pub at: [f64; 3],
}

/// Solver metrics attached to one seed's placement before any layout moves.
#[derive(Clone, Debug)]
pub struct KiCadPlacementSeedMetrics {
    pub seed: u64,
    pub global_iterations: usize,
    pub global_overflow: f64,
    pub wirelength_source_mm: f64,
    pub wirelength_global_mm: f64,
    pub wirelength_legal_mm: f64,
    pub skeleton_race: Option<KiCadSkeletonRaceReport>,
    pub panel_copies: Option<KiCadPanelCopiesReport>,
    pub pad_objective_composition: Option<KiCadPadObjectiveComposition>,
    pub sparse_legalization: Option<KiCadSparseLegalization>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct KiCadSparseLegalization {
    pub seated: usize,
    pub work_seconds: f64,
    #[serde(skip_serializing_if = "zero_usize")]
    pub moved_others: usize,
}

fn zero_usize(value: &usize) -> bool {
    *value == 0
}

impl From<core::SparseLegalization> for KiCadSparseLegalization {
    fn from(recovery: core::SparseLegalization) -> Self {
        Self {
            seated: recovery.seated,
            work_seconds: recovery.work as f64 / core::work_per_second(),
            moved_others: recovery.moved_others,
        }
    }
}

/// Provenance of a placement candidate made by combining legal seed poses.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadPadObjectiveComposition {
    pub parent_search: &'static str,
    pub parent_seed: u64,
    pub transfers: Vec<KiCadPadObjectiveTransfer>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPadObjectiveTransfer {
    pub part: String,
    pub anchor: String,
    pub donor_seed: u64,
}

/// A movable part the placement found no legal place for, said so that
/// an agent can act on it: a repeated reference (a panel's copies, a
/// keyboard's cells) is told apart by the footprint's uuid, which
/// constraints.json takes too.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadUnplacedPart {
    pub reference: String,
    /// The part in a message (the verdict's reason): its reference, with
    /// its uuid where the reference repeats on the board.
    pub label: String,
    /// The footprint's uuid (absent where the board has none).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    /// `unplaced` (no spot found) or `illegal` (left where it overlaps).
    pub status: &'static str,
    /// Where the placement left it (its origin and angle).
    pub at: [f64; 3],
    /// The constraints that hold it (`edge left within 0.52 mm`, `region`,
    /// `overhang top`): loosening one gives it more room.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub held_by: Vec<String>,
    /// For an illegal part, why, at the placement's level: what it meets
    /// first and the test that turns it down (`bodies overlap (no
    /// courtyard) with SW15`, `outline or hard constraint`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Geometry and legality tests at the retained pose and relaxation.
    pub local_diagnosis: KiCadPlacementLocalDiagnostic,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPlacementLocalDiagnostic {
    pub side: &'static str,
    /// [min x, min y, max x, max y] without the routing halo.
    pub body_bounds_mm: [f64; 4],
    /// Pair conflicts at the retained pose, including other unplaced parts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<KiCadPlacementLocalConflict>,
    #[serde(skip_serializing_if = "is_zero")]
    pub more_conflicts: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outside_outline_mm: Option<[f64; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hard_constraint_violation_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<KiCadPlacementRegionDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPlacementLocalConflict {
    pub kind: &'static str,
    pub other: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    pub fixed: bool,
    pub side: &'static str,
    pub at: [f64; 3],
    /// The neighbour's bounding box without its halo, for locating it.
    pub body_bounds_mm: [f64; 4],
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPlacementRegionDiagnostic {
    pub bounds_mm: [f64; 4],
    pub automatic: bool,
    pub origin: KiCadPlacementRegionOrigin,
    pub by_rotation: Vec<KiCadPlacementRegionRotation>,
    /// This footprint box blocks every feasible window at its retained pose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
    /// Held footprints whose combined bodies cover the entire centre domain.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fixed_region_cover: Vec<KiCadPlacementFixedBlocker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packing_conflict: Option<KiCadPlacementRegionPackingConflict>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPlacementFixedBlocker {
    pub part: String,
    pub uuid: String,
    pub at: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadPlacementRegionOrigin {
    Authored,
    BreakawayModule,
    BoardPiece,
}

/// An optimistic fit against the board bounds at the permitted angles.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadPlacementRegionPackingConflict {
    pub other: String,
    /// Largest possible body widths to the left, above, right and below.
    pub available_mm: [f64; 4],
    /// Lower bounds on the other footprint's projected width and height.
    pub other_minimum_mm: [f64; 2],
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPlacementRegionRotation {
    pub angle_deg: f64,
    pub body_size_mm: [f64; 2],
    /// [min x, min y, max x, max y] of centres admitted by the region.
    /// Null when the body does not fit at this angle.
    pub center_bounds_mm: Option<[f64; 4]>,
}

/// The bus groups of a placement (`bus_groups` in the placer's
/// configuration): the groups that moved more than one part at the
/// placement kept.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadBusGroupsReport {
    /// Groups with more than one part: banks and clusters.
    pub groups: usize,
    /// The banks: their members and the parts they move with their
    /// passives.
    pub banks: Vec<KiCadBusBank>,
    /// Clusters (a part and its passives) with more than one part.
    pub clusters: usize,
    /// Parts per group, the largest first.
    pub sizes: Vec<usize>,
}

/// A bank of `KiCadBusGroupsReport`.
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadBusBank {
    pub parts: Vec<String>,
    pub moved: usize,
}

impl KiCadBusGroupsReport {
    pub fn of(
        problem: &core::Problem,
        settings: &core::groups::Settings,
        poses: &[core::Pose],
        references: &[String],
    ) -> Self {
        let groups = core::groups::BusGroups::detect(problem, settings);
        let mut report = KiCadBusGroupsReport::default();
        for (kind, chain, moved) in groups.summary(problem, poses) {
            report.groups += 1;
            report.sizes.push(moved);
            match kind {
                core::groups::Kind::Bank => report.banks.push(KiCadBusBank {
                    parts: chain.iter().map(|part| references[*part].clone()).collect(),
                    moved,
                }),
                core::groups::Kind::Cluster => report.clusters += 1,
            }
        }
        report.sizes.sort_unstable_by(|a, b| b.cmp(a));
        report
    }
}

/// A net of a `length` rule as the placement left it (board-placer.json's
/// `length_rules`; `length_maxima`, `length_minima`): the window the
/// placer held its pins in and where they ended.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadLengthRuleReport {
    pub net: String,
    /// The rule's bounds, millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_mm: Option<f64>,
    /// What the placer held: its pins at least `apart_mm` apart (half the
    /// minimum) and within `within_mm` (0.8 of the maximum), octilinearly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apart_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within_mm: Option<f64>,
    /// The parts its pins lie on.
    pub parts: Vec<String>,
    /// The nearest and the farthest two of its pins on different parts
    /// after the placement, octilinearly.
    pub closest_mm: f64,
    pub farthest_mm: f64,
    /// `apart` (held apart for the minimum), `together` (within for the
    /// maximum), `both`, or `none` (see `note`).
    pub held: String,
    /// Whether the pins lie where the placer held them.
    pub kept: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Optional rear-placement permissions suggested by a failed placement.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadRearPlacementRecovery {
    /// Exact references to append to `placer.allow_back` only when the
    /// assembly permits rear mounting. No individual fit has been proven.
    pub allow_back_additions: Vec<String>,
    /// Movable footprints without a legal place, on either face.
    pub affected_movable: usize,
    /// Estimated spare rear area at the placement's utilization target.
    pub rear_headroom_mm2: f64,
}

/// Candidate permissions for rerouting a legally placed, crowded board.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadRearRoutingCandidates {
    pub allow_back_additions: Vec<String>,
    /// Modeled area of the candidates' bodies with placement spacing.
    pub front_body_area_mm2: f64,
    pub rear_headroom_mm2: f64,
}

/// Result of excluding one stranded automatic rear assignment and retrying
/// placement from the failed run's poses.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadAutoRearRetry {
    /// Add this exact reference to the placement brief's `front` list to
    /// request the same side choice on a fresh run.
    pub front_part: String,
    pub status: KiCadAutoRearRetryStatus,
    pub work_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadAutoRearRetryStatus {
    Skipped,
    Rejected,
    Accepted,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadSignalAnchorSeed {
    pub seed: u64,
    pub reference: String,
    pub estimated_xy_mm: [f64; 2],
    pub projected_xy_mm: [f64; 2],
    pub projected_angle_deg: f64,
    pub work_seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPerimeterContactRecovery {
    pub reference: String,
    pub status: &'static str,
    pub misplaced_before: usize,
    pub misplaced_after: usize,
    pub work_seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardPlacerResult {
    /// Initial pose offered to one seed. The reported final pose may differ.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_anchor_seed: Option<KiCadSignalAnchorSeed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perimeter_contact_recovery: Option<KiCadPerimeterContactRecovery>,
    /// How a compact parent and local pad poses were combined, when kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad_objective_composition: Option<KiCadPadObjectiveComposition>,
    /// Work spent seating the remaining parts on a sparse grid within the seed's budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sparse_legalization: Option<KiCadSparseLegalization>,
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
    /// The movable parts of `unplaced` and `illegal`, each once, with its
    /// uuid and what holds it (`KiCadUnplacedPart`); fixed parts and the
    /// board's marks a part was left on are not in it.
    pub unplaced_parts: Vec<KiCadUnplacedPart>,
    /// Every placement constraint with whether the placement keeps it.
    pub constraints: Vec<KiCadConstraintStatus>,
    pub constraint_warnings: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub placement_courtyard: Vec<crate::placement_constraints::KiCadPlacementCourtyard>,
    /// References put on the back using the optional `allow_back` setting.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub back_side_assignments: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rear_placement_recovery: Option<KiCadRearPlacementRecovery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rear_routing_candidates: Option<KiCadRearRoutingCandidates>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automatic_rear_retry: Option<KiCadAutoRearRetry>,
    /// The placement kept the edge geometry of the input PCB.
    pub outline_from_input: bool,
    /// The board size a constraints outline set (automatic or given).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_mm: Option<[f64; 2]>,
    /// How full each side is, front and back: `room.utilization` (the
    /// movable parts' bodies with spacing over the usable area); the hints
    /// call a side full above `room.target_utilization`.
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
    /// KiCad's legality held at the placement's level (`legality`:
    /// courtyards touching, pads the copper clearance apart).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pad_clearance: bool,
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
    /// The room kept around grid arrays (`bga_room`): per array its bands
    /// beyond the ball field on its side and the far side, the parts let
    /// under it, and fixed parts left in its room.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub array_rooms: Vec<String>,
    /// The nets of the project's `length` rules the placer held
    /// (`length_maxima`, `length_minima`): their window and where their
    /// pins ended.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub length_rules: Vec<KiCadLengthRuleReport>,
    /// The same, as (the mark's place among the board's marks, the part).
    #[serde(skip)]
    pub attached_mark_pairs: Vec<(usize, usize)>,
    /// The exact pair-row definitions used to place the board.
    #[serde(skip)]
    pub placed_pair_rows: Vec<super::PairRow>,
    /// Per footprint, the halo the placement gave it where the skeleton
    /// race merged the halos (`core::Relaxation::halos`).
    #[serde(skip)]
    pub halos: Option<std::sync::Arc<Vec<f64>>>,
    /// The other seeds' legal placements (poses, relaxation and how far
    /// they miss the constraints in all), next best first.
    #[serde(skip)]
    pub alternatives: Vec<(Vec<core::Pose>, core::Relaxation, f64)>,
    /// The solver metrics for the corresponding alternatives.
    #[serde(skip)]
    pub alternative_seed_metrics: Vec<KiCadPlacementSeedMetrics>,
    /// Per alternative, its pads without a way out (`pad_exits`; `None`
    /// off).
    #[serde(skip)]
    pub alternative_exits: Vec<Option<super::exits::KiCadPadExitsReport>>,
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
    /// The longest seed plus any additional serial placement work.
    #[serde(default)]
    pub work_seconds: f64,
    /// The kept seeds' work as the clock sees it: as the reference machine
    /// runs them, `REFERENCE_THREADS` at once in a pool (`race_seeds`), a
    /// seed starting as a thread frees. It depends on the seeds' work
    /// alone, not on the threads or the load.
    #[serde(default)]
    pub pool_work_seconds: f64,
    /// Modeled work of the extra pad objective search, included in work_seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad_objective_search_work_seconds: Option<f64>,
    /// The kept seeds' work in all (as one after the other).
    #[serde(default)]
    pub seeds_work_seconds: f64,
    /// Seeds attempted, including work-limit overflow and experimental drafts.
    #[serde(default)]
    pub seeds_run: usize,
    /// The seed of the placement kept (`seed` plus its offset).
    #[serde(default)]
    pub seed_kept: u64,
    pub footprints: Vec<KiCadPlacedFootprint>,
    /// The bus groups (`bus_groups` on): absent when off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bus_groups: Option<KiCadBusGroupsReport>,
    /// The panel's copies (`panel_copies` on and copies found): absent
    /// otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_copies: Option<KiCadPanelCopiesReport>,
    /// The sub-boards of a panel of different boards (`sub_boards` on,
    /// an outline of two pieces or more without its tabs with parts on
    /// two): absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_boards: Option<crate::sub_boards::KiCadSubBoardsReport>,
    /// The breakaway modules of one board (`keep_in_modules` on, two
    /// modules or more): absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modules: Option<crate::modules::KiCadModulesReport>,
    /// Pads without a way out (`pad_exits`, off by default): the pads of
    /// the kept placement that no straight way leaves on the statics, what
    /// closes each, and the parts the repair moved. Absent when off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad_exits: Option<super::exits::KiCadPadExitsReport>,
    /// The race over skeletons (`skeleton_race` on): absent when off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_race: Option<KiCadSkeletonRaceReport>,
}

// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Deterministic, replaceable initial-placement policies.
//!
//! This module deliberately stops at rigid component poses. Pads are stable
//! component-local anchors, not independent particles, and copper routing is
//! not performed here. Every proposal, including future embedding-guided
//! proposals, passes through the same hard-constraint projection and
//! validation path before it can reach the routing pipeline.

use std::collections::{BTreeMap, HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use layout_trace_model::model::{
    BoardEdge, CopperShape, Movement, PinRef, PlacementAnchor, PlacementConstraint, Problem, Rect,
    Rotation, Vec2, angular_distance_degrees,
};

const FEASIBILITY_TOLERANCE_MM: f64 = 1.0e-6;

#[path = "coupled_legalization.rs"]
mod coupled_legalization;
#[path = "fixed_obstacle_seed.rs"]
mod fixed_obstacle_seed;
#[path = "placement_geometry.rs"]
mod geometry;
pub use coupled_legalization::{CoupledLegalizationConfig, CoupledLegalizationFrame};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InitialPlacementPolicy {
    /// Preserve the input poses, while still checking every represented hard
    /// constraint through the common projector.
    #[default]
    Declared,
    /// Deterministic ID-ordered row packing. This is intentionally cheap; it
    /// is a seed generator, not a packing proof.
    GridPacking {
        #[serde(default = "default_grid_gap_mm")]
        gap_mm: f64,
    },
    /// Constraint-feasible pseudo-random proposals. Randomness is local to a
    /// component ID, so input-vector permutations do not alter the result.
    Random {
        #[serde(default = "default_random_attempts")]
        attempts: usize,
    },
    /// Synchronous net-weighted barycentric relaxation of rigid bodies.
    /// This minimizes a connectivity-distance surrogate only and makes no
    /// claim about routability or non-overlap.
    ConnectivityBarycentric {
        #[serde(default = "default_barycentric_iterations")]
        iterations: usize,
        #[serde(default = "default_barycentric_attraction")]
        attraction: f64,
    },
    /// Sparse fixed-boundary harmonic embedding followed by a bounded rigid
    /// quarter-turn choice using physical pad directions. This is only a pose
    /// seed: it neither routes copper nor preserves a prior corridor word.
    /// Attraction uses width times tension_weight; component bounds and axis
    /// locks participate in every relaxation step.
    HarmonicPorts {
        #[serde(default = "default_harmonic_iterations")]
        iterations: usize,
        #[serde(default = "default_legalization_sweeps")]
        legalization_sweeps: usize,
        #[serde(default = "default_maximum_pair_checks")]
        maximum_pair_checks: usize,
        /// Optional nearest 0.25 mm sampled center legal against fixed bodies,
        /// using the same pair-check budget as subsequent legalization.
        #[serde(default, skip_serializing_if = "bool_is_false")]
        fixed_obstacle_seed_projection: bool,
        /// Insert largest complete footprint bounding rectangles first, making
        /// already inserted movable bodies part of the sampled obstacle union.
        #[serde(default, skip_serializing_if = "bool_is_false")]
        largest_first_seed_insertion: bool,
        /// Skip certified colliding lattice intervals without changing sample order.
        #[serde(default, skip_serializing_if = "bool_is_false")]
        skip_collision_intervals: bool,
        /// Transient extra copper-pair gap during seed insertion only. This
        /// reserves routing space without changing bodies or native rules.
        #[serde(default, skip_serializing_if = "f64_is_zero")]
        seed_extra_pad_gap_mm: f64,
        /// Optional bounded joint contact recovery after ordinary legalization.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        coupled_legalization: Option<CoupledLegalizationConfig>,
        /// Historical layout-trace heuristic. This radial support-distance
        /// floor is stronger than exact oriented-body non-overlap, so it is
        /// selectable for controlled ablations rather than correctness law.
        #[serde(default = "default_connected_pair_spacing_floor")]
        connected_pair_spacing_floor: bool,
        /// Seed policy for movable Laplacian blocks with fewer than two
        /// distinct positional anchors. The historical behavior retains the
        /// declaration; alternatives make those otherwise unconstrained
        /// blocks an explicit portfolio dimension.
        #[serde(default)]
        underanchored_seed: UnderanchoredSeedPolicy,
    },
    /// Deterministic local mutations around one base-policy proposal. This is
    /// the reviewed subset of the predecessor's population search: it creates
    /// cheap archive diversity without inheriting routed copper or selecting
    /// by a placement surrogate alone.
    LocalMutation {
        base: Box<InitialPlacementPolicy>,
        #[serde(default = "default_local_mutation_attempts")]
        attempts: usize,
        #[serde(default = "default_local_mutation_step_mm")]
        step_mm: f64,
        #[serde(default = "default_local_mutations_per_proposal")]
        mutations_per_proposal: usize,
        #[serde(default = "default_allow_rotation")]
        allow_rotation: bool,
    },
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UnderanchoredSeedPolicy {
    #[default]
    Declared,
    GridPacking {
        #[serde(default = "default_grid_gap_mm")]
        gap_mm: f64,
    },
    Random {
        #[serde(default = "default_random_attempts")]
        attempts: usize,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InitialPlacementConfig {
    #[serde(default)]
    pub policy: InitialPlacementPolicy,
    #[serde(default)]
    pub seed: u64,
    #[serde(default = "default_projection_sweeps")]
    pub projection_sweeps: usize,
    /// Optional refinement after the common placement projector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation_refinement: Option<crate::OrientationRefinementConfig>,
}

impl InitialPlacementConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.projection_sweeps == 0 {
            return Err("initial placement projection_sweeps must be positive".into());
        }
        if let Some(config) = &self.orientation_refinement {
            config.check()?;
        }
        match &self.policy {
            InitialPlacementPolicy::Declared => {}
            InitialPlacementPolicy::GridPacking { gap_mm } => {
                if !gap_mm.is_finite() || *gap_mm < 0.0 {
                    return Err(
                        "initial placement grid gap_mm must be finite and non-negative".into(),
                    );
                }
            }
            InitialPlacementPolicy::Random { attempts } => {
                if *attempts == 0 {
                    return Err("initial placement random attempts must be positive".into());
                }
            }
            InitialPlacementPolicy::ConnectivityBarycentric {
                iterations,
                attraction,
            } => {
                if *iterations == 0 {
                    return Err("initial placement barycentric iterations must be positive".into());
                }
                if !attraction.is_finite() || !(0.0..=1.0).contains(attraction) {
                    return Err(
                        "initial placement barycentric attraction must be between zero and one"
                            .into(),
                    );
                }
            }
            InitialPlacementPolicy::HarmonicPorts {
                iterations,
                legalization_sweeps,
                maximum_pair_checks,
                coupled_legalization,
                underanchored_seed,
                fixed_obstacle_seed_projection,
                largest_first_seed_insertion,
                skip_collision_intervals,
                seed_extra_pad_gap_mm,
                ..
            } => {
                if *iterations == 0 || *legalization_sweeps == 0 || *maximum_pair_checks == 0 {
                    return Err(
                        "initial placement harmonic and legalization budgets must be positive"
                            .into(),
                    );
                }
                if *largest_first_seed_insertion && !*fixed_obstacle_seed_projection {
                    return Err(
                        "largest-first seed insertion requires fixed-obstacle seed projection"
                            .into(),
                    );
                }
                if *skip_collision_intervals && !*fixed_obstacle_seed_projection {
                    return Err(
                        "collision interval skipping requires fixed-obstacle seed projection"
                            .into(),
                    );
                }
                if !seed_extra_pad_gap_mm.is_finite() || *seed_extra_pad_gap_mm < 0.0 {
                    return Err("seed_extra_pad_gap_mm must be finite and nonnegative".into());
                }
                if *seed_extra_pad_gap_mm > 0.0 && !*largest_first_seed_insertion {
                    return Err("seed extra pad gap requires largest-first seed insertion".into());
                }
                check_underanchored_seed_policy(underanchored_seed)?;
                if let Some(coupled) = coupled_legalization {
                    coupled.check()?;
                }
            }
            InitialPlacementPolicy::LocalMutation {
                base,
                attempts,
                step_mm,
                mutations_per_proposal,
                ..
            } => {
                if matches!(base.as_ref(), InitialPlacementPolicy::LocalMutation { .. }) {
                    return Err("nested initial-placement local mutation is not supported".into());
                }
                check_initial_placement_policy(base)?;
                if *attempts == 0 || *mutations_per_proposal == 0 {
                    return Err(
                        "local mutation attempts and mutations_per_proposal must be positive"
                            .into(),
                    );
                }
                if !step_mm.is_finite() || *step_mm <= 0.0 {
                    return Err("local mutation step_mm must be finite and positive".into());
                }
            }
        }
        Ok(())
    }
}

fn check_initial_placement_policy(policy: &InitialPlacementPolicy) -> Result<(), String> {
    InitialPlacementConfig {
        policy: policy.clone(),
        seed: 0,
        orientation_refinement: None,
        projection_sweeps: 1,
    }
    .check()
}

fn check_underanchored_seed_policy(policy: &UnderanchoredSeedPolicy) -> Result<(), String> {
    match policy {
        UnderanchoredSeedPolicy::Declared => Ok(()),
        UnderanchoredSeedPolicy::GridPacking { gap_mm } => {
            if !gap_mm.is_finite() || *gap_mm < 0.0 {
                return Err(
                    "under-anchored grid seed gap_mm must be finite and non-negative".into(),
                );
            }
            Ok(())
        }
        UnderanchoredSeedPolicy::Random { attempts } => {
            if *attempts == 0 {
                return Err("under-anchored random seed attempts must be positive".into());
            }
            Ok(())
        }
    }
}

impl Default for InitialPlacementConfig {
    fn default() -> Self {
        Self {
            policy: InitialPlacementPolicy::Declared,
            seed: 0,
            projection_sweeps: default_projection_sweeps(),
            orientation_refinement: None,
        }
    }
}

fn default_grid_gap_mm() -> f64 {
    1.0
}

fn default_random_attempts() -> usize {
    16
}

fn default_barycentric_iterations() -> usize {
    12
}

fn default_barycentric_attraction() -> f64 {
    0.35
}

fn default_harmonic_iterations() -> usize {
    128
}

fn default_local_mutation_attempts() -> usize {
    16
}

fn default_local_mutation_step_mm() -> f64 {
    0.5
}

fn default_local_mutations_per_proposal() -> usize {
    1
}

fn default_allow_rotation() -> bool {
    true
}

fn default_legalization_sweeps() -> usize {
    64
}

fn default_maximum_pair_checks() -> usize {
    1_000_000
}

fn default_connected_pair_spacing_floor() -> bool {
    true
}

fn default_projection_sweeps() -> usize {
    256
}

fn bool_is_false(value: &bool) -> bool {
    !value
}

fn f64_is_zero(value: &f64) -> bool {
    *value == 0.0
}

fn usize_is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlacementPose {
    pub component: String,
    pub position: Vec2,
    pub rotation_degrees: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InitialPlacementEvidence {
    pub policy: String,
    pub seed: u64,
    /// Number of complete proposals passed to the common projector.
    pub proposal_attempts: usize,
    pub projection_sweeps: usize,
    /// Relational hard-constraint checks performed by the common projector.
    pub hard_constraint_checks: usize,
    /// Exact oriented-body pair checks performed by the common projector.
    pub body_pair_checks: usize,
    pub moved_components: usize,
    /// Components whose center changed from the declared pose.
    pub translated_components: usize,
    /// Components whose orientation changed from the declared pose.
    pub rotated_components: usize,
    pub connectivity_distance_before_mm: f64,
    pub connectivity_distance_after_mm: f64,
    /// Cost-independent straight-ratsnest and raster-demand diagnostics. They
    /// are placement guidance only; neither field claims routability.
    pub demand: PlacementDemandEvidence,
    pub feasible: bool,
    /// The initializer only emits rigid-body poses. This explicit negative
    /// claim prevents a favorable surrogate score from being mistaken for a
    /// corridor or copper certificate.
    pub routability_claimed: bool,
    /// Reserved integration point for a later abstract-embedding policy.
    pub embedding_guidance_used: bool,
    /// Connected movable blocks with fewer than two distinct positional
    /// anchors receive no harmonic displacement. The later common legality
    /// projection may still move them to remove an overlap or satisfy a hard
    /// constraint. This makes a partial harmonic seed distinguishable from
    /// whole-board harmonic success.
    #[serde(skip_serializing_if = "usize_is_zero")]
    pub underanchored_blocks_retained: usize,
    /// Under-anchored blocks given an explicit non-declared seed. This is a
    /// proposal count, not evidence that the resulting placement routes.
    #[serde(skip_serializing_if = "usize_is_zero")]
    pub underanchored_blocks_reseeded: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation_refinement: Option<crate::OrientationRefinementEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlacementDemandEvidence {
    pub grid_size: [usize; 2],
    pub segment_count: usize,
    pub cross_net_proper_crossings: usize,
    pub occupied_cells: usize,
    pub peak_cell_demand: f64,
    pub squared_cell_demand: f64,
    pub deposited_demand: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InitialPlacementResult {
    pub poses: Vec<PlacementPose>,
    pub evidence: InitialPlacementEvidence,
}

/// Outcome of one proposal in a bounded placement archive.
///
/// Rejected and duplicate attempts remain visible instead of disappearing
/// behind the first legal proposal. Only retained attempts are intended for
/// an expensive router/native-verifier finalist stage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InitialPlacementArchiveDisposition {
    Retained,
    FeasibleNotRetained,
    Duplicate,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InitialPlacementArchiveAttempt {
    pub ordinal: usize,
    pub disposition: InitialPlacementArchiveDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of_ordinal: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placement: Option<InitialPlacementResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InitialPlacementArchive {
    pub policy: String,
    pub seed: u64,
    pub maximum_proposals: usize,
    pub maximum_retained: usize,
    pub evaluated_proposals: usize,
    pub feasible_unique_proposals: usize,
    pub duplicate_proposals: usize,
    pub rejected_proposals: usize,
    pub retained_proposals: usize,
    pub retained_attempt_ordinals: Vec<usize>,
    pub ranking_rule: String,
    pub attempts: Vec<InitialPlacementArchiveAttempt>,
}

/// Common proposal interface for built-in policies and future experimental
/// policies (for example one consuming embedding-derived port orders).
/// Proposals are never trusted: [`run_initial_placement_with_generator`]
/// projects and validates them before returning them.
pub trait InitialPlacementGenerator {
    fn name(&self) -> &'static str;
    fn maximum_attempts(&self) -> usize {
        1
    }
    fn orientation_refinement(&self) -> Option<&crate::OrientationRefinementConfig> {
        None
    }
    fn underanchored_blocks_retained(&self, _problem: &Problem) -> Result<usize, String> {
        Ok(0)
    }
    fn underanchored_blocks_reseeded(&self, _problem: &Problem) -> Result<usize, String> {
        Ok(0)
    }
    fn validate_projected(
        &self,
        _problem: &Problem,
        _poses: &[PlacementPose],
    ) -> Result<(), String> {
        Ok(())
    }
    fn propose(
        &self,
        problem: &Problem,
        seed: u64,
        attempt: usize,
    ) -> Result<Vec<PlacementPose>, String>;
}

struct BuiltinGenerator<'a>(
    &'a InitialPlacementPolicy,
    Option<&'a crate::OrientationRefinementConfig>,
    Option<&'a dyn Fn(PlacementLegalizationFrame)>,
);

impl InitialPlacementGenerator for BuiltinGenerator<'_> {
    fn orientation_refinement(&self) -> Option<&crate::OrientationRefinementConfig> {
        self.1
    }
    fn name(&self) -> &'static str {
        match self.0 {
            InitialPlacementPolicy::Declared => "declared",
            InitialPlacementPolicy::GridPacking { .. } => "grid_packing",
            InitialPlacementPolicy::Random { .. } => "random",
            InitialPlacementPolicy::ConnectivityBarycentric { .. } => "connectivity_barycentric",
            InitialPlacementPolicy::HarmonicPorts {
                underanchored_seed: UnderanchoredSeedPolicy::GridPacking { .. },
                ..
            } => "harmonic_ports_underanchored_grid",
            InitialPlacementPolicy::HarmonicPorts {
                underanchored_seed: UnderanchoredSeedPolicy::Random { .. },
                ..
            } => "harmonic_ports_underanchored_random",
            InitialPlacementPolicy::HarmonicPorts {
                connected_pair_spacing_floor: true,
                ..
            } => "harmonic_ports",
            InitialPlacementPolicy::HarmonicPorts {
                connected_pair_spacing_floor: false,
                ..
            } => "harmonic_ports_without_connected_pair_floor",
            InitialPlacementPolicy::LocalMutation { .. } => "local_mutation_archive",
        }
    }

    fn maximum_attempts(&self) -> usize {
        match self.0 {
            InitialPlacementPolicy::Random { attempts } => *attempts,
            InitialPlacementPolicy::HarmonicPorts {
                underanchored_seed: UnderanchoredSeedPolicy::Random { attempts },
                ..
            } => *attempts,
            InitialPlacementPolicy::LocalMutation { attempts, .. } => *attempts,
            _ => 1,
        }
    }

    fn underanchored_blocks_retained(&self, problem: &Problem) -> Result<usize, String> {
        match self.0 {
            InitialPlacementPolicy::HarmonicPorts {
                underanchored_seed: UnderanchoredSeedPolicy::Declared,
                ..
            } => harmonic_underanchored_blocks(problem),
            InitialPlacementPolicy::LocalMutation { base, .. } => {
                BuiltinGenerator(base, None, self.2).underanchored_blocks_retained(problem)
            }
            _ => Ok(0),
        }
    }

    fn underanchored_blocks_reseeded(&self, problem: &Problem) -> Result<usize, String> {
        match self.0 {
            InitialPlacementPolicy::HarmonicPorts {
                underanchored_seed, ..
            } if !matches!(underanchored_seed, UnderanchoredSeedPolicy::Declared) => {
                harmonic_underanchored_blocks(problem)
            }
            InitialPlacementPolicy::LocalMutation { base, .. } => {
                BuiltinGenerator(base, None, self.2).underanchored_blocks_reseeded(problem)
            }
            _ => Ok(0),
        }
    }

    fn validate_projected(&self, problem: &Problem, poses: &[PlacementPose]) -> Result<(), String> {
        match self.0 {
            InitialPlacementPolicy::HarmonicPorts { .. } => {
                validate_placement_exclusions(problem, poses)
            }
            InitialPlacementPolicy::LocalMutation { base, .. } => {
                BuiltinGenerator(base, None, self.2).validate_projected(problem, poses)
            }
            _ => Ok(()),
        }
    }

    fn propose(
        &self,
        problem: &Problem,
        seed: u64,
        attempt: usize,
    ) -> Result<Vec<PlacementPose>, String> {
        match self.0 {
            InitialPlacementPolicy::Declared => Ok(declared_poses(problem)),
            InitialPlacementPolicy::GridPacking { gap_mm } => Ok(grid_poses(problem, *gap_mm)),
            InitialPlacementPolicy::Random { .. } => Ok(random_poses(problem, seed, attempt)),
            InitialPlacementPolicy::ConnectivityBarycentric {
                iterations,
                attraction,
            } => Ok(barycentric_poses(problem, *iterations, *attraction)),
            InitialPlacementPolicy::HarmonicPorts {
                iterations,
                legalization_sweeps,
                maximum_pair_checks,
                coupled_legalization,
                connected_pair_spacing_floor,
                underanchored_seed,
                fixed_obstacle_seed_projection,
                largest_first_seed_insertion,
                skip_collision_intervals,
                seed_extra_pad_gap_mm,
            } => harmonic_port_poses(
                problem,
                *iterations,
                *legalization_sweeps,
                *maximum_pair_checks,
                coupled_legalization.as_ref(),
                *connected_pair_spacing_floor,
                underanchored_seed,
                *fixed_obstacle_seed_projection,
                *largest_first_seed_insertion,
                *skip_collision_intervals,
                *seed_extra_pad_gap_mm,
                seed,
                attempt,
                self.2,
            ),
            InitialPlacementPolicy::LocalMutation {
                base,
                step_mm,
                mutations_per_proposal,
                allow_rotation,
                ..
            } => {
                let base_generator = BuiltinGenerator(base, None, self.2);
                let base_poses = base_generator.propose(problem, seed, 0)?;
                local_mutation_poses(
                    problem,
                    base_poses,
                    seed,
                    attempt,
                    *step_mm,
                    *mutations_per_proposal,
                    *allow_rotation,
                )
            }
        }
    }
}

pub fn run_initial_placement(
    problem: &Problem,
    config: &InitialPlacementConfig,
) -> Result<InitialPlacementResult, String> {
    problem.check_schema()?;
    config.check()?;
    let embedding_guidance_used = policy_uses_embedding(&config.policy);
    run_initial_placement_with_generator(
        problem,
        config.seed,
        config.projection_sweeps,
        &BuiltinGenerator(&config.policy, config.orientation_refinement.as_ref(), None),
        embedding_guidance_used,
    )
}

/// Observe harmonic legalization without changing proposals or admission.
/// Frames include the initial state and every completed legalization sweep,
/// even when the proposal ultimately fails. Other policy phases are not traced.
/// The observer owns storage/rendering; the placement library performs no I/O.
pub fn run_initial_placement_traced(
    problem: &Problem,
    config: &InitialPlacementConfig,
    observer: &dyn Fn(PlacementLegalizationFrame),
) -> Result<InitialPlacementResult, String> {
    problem.check_schema()?;
    config.check()?;
    run_initial_placement_with_generator(
        problem,
        config.seed,
        config.projection_sweeps,
        &BuiltinGenerator(
            &config.policy,
            config.orientation_refinement.as_ref(),
            Some(observer),
        ),
        policy_uses_embedding(&config.policy),
    )
}

pub fn run_initial_placement_with_generator(
    problem: &Problem,
    seed: u64,
    projection_sweeps: usize,
    generator: &dyn InitialPlacementGenerator,
    embedding_guidance_used: bool,
) -> Result<InitialPlacementResult, String> {
    if projection_sweeps == 0 {
        return Err("initial placement projection_sweeps must be positive".into());
    }
    let before = connectivity_distance(problem, &declared_poses(problem))?;
    let maximum_attempts = generator.maximum_attempts().max(1);
    let mut last_error = None;
    for attempt in 0..maximum_attempts {
        let proposal = match generator.propose(problem, seed, attempt) {
            Ok(proposal) => proposal,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        match evaluate_initial_placement_proposal(
            problem,
            proposal,
            seed,
            attempt,
            projection_sweeps,
            generator,
            embedding_guidance_used,
            before,
        ) {
            Ok(result) => return Ok(result),
            Err(error) => last_error = Some(error),
        }
    }
    Err(format!(
        "initial placement policy {} exhausted {maximum_attempts} proposal(s): {}",
        generator.name(),
        last_error.unwrap_or_else(|| "no proposal was produced".into())
    ))
}

/// Generates every bounded proposal exposed by one placement policy, retains
/// rejected and duplicate outcomes, and marks a cheap diverse finalist set.
///
/// The archive ranking is deliberately not a routing claim. Callers should
/// route and independently verify every retained proposal before selecting a
/// board. [`run_initial_placement`] keeps its historical first-feasible
/// behavior; experiments must opt into this archive explicitly.
pub fn run_initial_placement_archive(
    problem: &Problem,
    config: &InitialPlacementConfig,
    maximum_retained: usize,
) -> Result<InitialPlacementArchive, String> {
    problem.check_schema()?;
    config.check()?;
    let embedding_guidance_used = policy_uses_embedding(&config.policy);
    run_initial_placement_archive_with_generator(
        problem,
        config.seed,
        config.projection_sweeps,
        &BuiltinGenerator(&config.policy, config.orientation_refinement.as_ref(), None),
        embedding_guidance_used,
        maximum_retained,
    )
}

pub fn run_initial_placement_archive_with_generator(
    problem: &Problem,
    seed: u64,
    projection_sweeps: usize,
    generator: &dyn InitialPlacementGenerator,
    embedding_guidance_used: bool,
    maximum_retained: usize,
) -> Result<InitialPlacementArchive, String> {
    if projection_sweeps == 0 {
        return Err("initial placement projection_sweeps must be positive".into());
    }
    if maximum_retained == 0 {
        return Err("initial placement archive maximum_retained must be positive".into());
    }
    let before = connectivity_distance(problem, &declared_poses(problem))?;
    let maximum_proposals = generator.maximum_attempts().max(1);
    let mut attempts = Vec::with_capacity(maximum_proposals);
    let mut first_ordinal_by_pose = HashMap::<String, usize>::new();
    let mut feasible_indices = Vec::new();
    let mut duplicates = 0;
    let mut rejected = 0;

    for ordinal in 0..maximum_proposals {
        let evaluated = generator
            .propose(problem, seed, ordinal)
            .and_then(|proposal| {
                evaluate_initial_placement_proposal(
                    problem,
                    proposal,
                    seed,
                    ordinal,
                    projection_sweeps,
                    generator,
                    embedding_guidance_used,
                    before,
                )
            });
        match evaluated {
            Ok(placement) => {
                let fingerprint = placement_pose_fingerprint(&placement.poses);
                if let Some(duplicate_of_ordinal) = first_ordinal_by_pose.get(&fingerprint).copied()
                {
                    duplicates += 1;
                    attempts.push(InitialPlacementArchiveAttempt {
                        ordinal,
                        disposition: InitialPlacementArchiveDisposition::Duplicate,
                        duplicate_of_ordinal: Some(duplicate_of_ordinal),
                        placement: None,
                        error: None,
                    });
                } else {
                    first_ordinal_by_pose.insert(fingerprint, ordinal);
                    let index = attempts.len();
                    feasible_indices.push(index);
                    attempts.push(InitialPlacementArchiveAttempt {
                        ordinal,
                        disposition: InitialPlacementArchiveDisposition::FeasibleNotRetained,
                        duplicate_of_ordinal: None,
                        placement: Some(placement),
                        error: None,
                    });
                }
            }
            Err(error) => {
                rejected += 1;
                attempts.push(InitialPlacementArchiveAttempt {
                    ordinal,
                    disposition: InitialPlacementArchiveDisposition::Rejected,
                    duplicate_of_ordinal: None,
                    placement: None,
                    error: Some(error),
                });
            }
        }
    }

    feasible_indices.sort_by(|left, right| {
        let left_ordinal = attempts[*left].ordinal;
        let right_ordinal = attempts[*right].ordinal;
        let left_placement = attempts[*left]
            .placement
            .as_ref()
            .expect("feasible archive index has placement");
        let right_placement = attempts[*right]
            .placement
            .as_ref()
            .expect("feasible archive index has placement");
        left_placement
            .evidence
            .demand
            .cross_net_proper_crossings
            .cmp(&right_placement.evidence.demand.cross_net_proper_crossings)
            .then_with(|| {
                left_placement
                    .evidence
                    .demand
                    .squared_cell_demand
                    .total_cmp(&right_placement.evidence.demand.squared_cell_demand)
            })
            .then_with(|| {
                left_placement
                    .evidence
                    .demand
                    .peak_cell_demand
                    .total_cmp(&right_placement.evidence.demand.peak_cell_demand)
            })
            .then_with(|| {
                left_placement
                    .evidence
                    .connectivity_distance_after_mm
                    .total_cmp(&right_placement.evidence.connectivity_distance_after_mm)
            })
            .then_with(|| left_ordinal.cmp(&right_ordinal))
    });
    let retained_indices = feasible_indices
        .iter()
        .copied()
        .take(maximum_retained)
        .collect::<Vec<_>>();
    let retained_attempt_ordinals = retained_indices
        .iter()
        .map(|index| attempts[*index].ordinal)
        .collect::<Vec<_>>();
    for index in retained_indices {
        attempts[index].disposition = InitialPlacementArchiveDisposition::Retained;
    }

    Ok(InitialPlacementArchive {
        policy: generator.name().into(),
        seed,
        maximum_proposals,
        maximum_retained,
        evaluated_proposals: attempts.len(),
        feasible_unique_proposals: feasible_indices.len(),
        duplicate_proposals: duplicates,
        rejected_proposals: rejected,
        retained_proposals: retained_attempt_ordinals.len(),
        retained_attempt_ordinals,
        ranking_rule: "cross-net proper crossings, squared cell demand, peak cell demand, connectivity distance, proposal ordinal; routing/native validation intentionally deferred"
            .into(),
        attempts,
    })
}

#[allow(clippy::too_many_arguments)]
fn evaluate_initial_placement_proposal(
    problem: &Problem,
    proposal: Vec<PlacementPose>,
    seed: u64,
    attempt: usize,
    projection_sweeps: usize,
    generator: &dyn InitialPlacementGenerator,
    embedding_guidance_used: bool,
    before: f64,
) -> Result<InitialPlacementResult, String> {
    let (mut poses, used_sweeps, hard_constraint_checks, body_pair_checks) =
        project_and_validate(problem, proposal, projection_sweeps)?;
    generator.validate_projected(problem, &poses)?;
    let orientation_refinement = if let Some(config) = generator.orientation_refinement() {
        let result = crate::refine_placement_orientations(problem, &poses, config)?;
        poses = result.poses;
        generator.validate_projected(problem, &poses)?;
        Some(result.evidence)
    } else {
        None
    };
    let after = connectivity_distance(problem, &poses)?;
    let demand = placement_demand_evidence(problem, &poses, [32, 32])?;
    let declared = declared_poses(problem)
        .into_iter()
        .map(|pose| (pose.component.clone(), pose))
        .collect::<HashMap<_, _>>();
    let translated_components = poses
        .iter()
        .filter(|pose| {
            let input = &declared[&pose.component];
            distance(input.position, pose.position) > FEASIBILITY_TOLERANCE_MM
        })
        .count();
    let rotated_components = poses
        .iter()
        .filter(|pose| {
            let input = &declared[&pose.component];
            angle_distance(input.rotation_degrees, pose.rotation_degrees) > FEASIBILITY_TOLERANCE_MM
        })
        .count();
    let moved_components = poses
        .iter()
        .filter(|pose| {
            let input = &declared[&pose.component];
            distance(input.position, pose.position) > FEASIBILITY_TOLERANCE_MM
                || angle_distance(input.rotation_degrees, pose.rotation_degrees)
                    > FEASIBILITY_TOLERANCE_MM
        })
        .count();
    Ok(InitialPlacementResult {
        poses,
        evidence: InitialPlacementEvidence {
            orientation_refinement,
            policy: generator.name().into(),
            seed,
            proposal_attempts: attempt + 1,
            projection_sweeps: used_sweeps,
            hard_constraint_checks,
            body_pair_checks,
            moved_components,
            translated_components,
            rotated_components,
            connectivity_distance_before_mm: before,
            connectivity_distance_after_mm: after,
            demand,
            feasible: true,
            routability_claimed: false,
            embedding_guidance_used,
            underanchored_blocks_retained: generator.underanchored_blocks_retained(problem)?,
            underanchored_blocks_reseeded: generator.underanchored_blocks_reseeded(problem)?,
        },
    })
}

fn placement_pose_fingerprint(poses: &[PlacementPose]) -> String {
    poses
        .iter()
        .map(|pose| {
            format!(
                "{}:{:016x}:{:016x}:{:016x}",
                pose.component,
                pose.position.x.to_bits(),
                pose.position.y.to_bits(),
                pose.rotation_degrees.to_bits()
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn policy_uses_embedding(policy: &InitialPlacementPolicy) -> bool {
    match policy {
        InitialPlacementPolicy::HarmonicPorts { .. } => true,
        InitialPlacementPolicy::LocalMutation { base, .. } => policy_uses_embedding(base),
        _ => false,
    }
}

fn local_mutation_poses(
    problem: &Problem,
    mut poses: Vec<PlacementPose>,
    seed: u64,
    attempt: usize,
    step_mm: f64,
    mutations_per_proposal: usize,
    allow_rotation: bool,
) -> Result<Vec<PlacementPose>, String> {
    // Proposal zero is the exact base control, matching the predecessor's
    // population convention. Later proposals are independent mutations of
    // that same base rather than a warm chain.
    if attempt == 0 {
        return Ok(poses);
    }
    let components = sorted_components(problem);
    let movable = components
        .iter()
        .enumerate()
        .filter(|(_, component)| component.constraints.movement != Movement::Fixed)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if movable.is_empty() {
        return Ok(poses);
    }
    let pose_by_component = poses
        .iter()
        .enumerate()
        .map(|(index, pose)| (pose.component.clone(), index))
        .collect::<HashMap<_, _>>();
    for mutation in 0..mutations_per_proposal {
        let random = mix64(
            seed ^ (attempt as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ (mutation as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9),
        );
        let component_index = movable[random as usize % movable.len()];
        let component = components[component_index];
        let pose_index = *pose_by_component
            .get(&component.id)
            .ok_or_else(|| format!("local mutation is missing pose {}", component.id))?;
        let mut dx = ((random >> 16) % 3) as i32 - 1;
        let mut dy = ((random >> 18) % 3) as i32 - 1;
        match component.constraints.movement {
            Movement::Fixed => unreachable!("fixed components were filtered"),
            Movement::Horizontal => dy = 0,
            Movement::Vertical => dx = 0,
            Movement::Free => {}
        }
        if dx == 0 && dy == 0 {
            match component.constraints.movement {
                Movement::Horizontal => dx = if random & 1 == 0 { -1 } else { 1 },
                Movement::Vertical => dy = if random & 1 == 0 { -1 } else { 1 },
                Movement::Free => dx = if random & 1 == 0 { -1 } else { 1 },
                Movement::Fixed => unreachable!("fixed components were filtered"),
            }
        }
        poses[pose_index].position.x += dx as f64 * step_mm;
        poses[pose_index].position.y += dy as f64 * step_mm;
        if allow_rotation
            && component.constraints.rotation == Rotation::Free
            && (random >> 20).is_multiple_of(4)
        {
            let quarter_turns = ((random >> 24) % 3 + 1) as f64;
            poses[pose_index].rotation_degrees =
                (poses[pose_index].rotation_degrees + 90.0 * quarter_turns).rem_euclid(360.0);
        }
    }
    Ok(poses)
}

fn sorted_components(problem: &Problem) -> Vec<&layout_trace_model::model::Component> {
    let mut components = problem.components.iter().collect::<Vec<_>>();
    components.sort_by(|first, second| first.id.cmp(&second.id));
    components
}

fn declared_poses(problem: &Problem) -> Vec<PlacementPose> {
    sorted_components(problem)
        .into_iter()
        .map(|component| PlacementPose {
            component: component.id.clone(),
            position: component.position,
            rotation_degrees: component.rotation_degrees,
        })
        .collect()
}

fn grid_poses(problem: &Problem, gap_mm: f64) -> Vec<PlacementPose> {
    let components = sorted_components(problem);
    let movable = components
        .iter()
        .filter(|component| component.constraints.movement != Movement::Fixed)
        .copied()
        .collect::<Vec<_>>();
    if movable.is_empty() {
        return declared_poses(problem);
    }
    let maximum_width = movable
        .iter()
        .map(|component| component.size.x)
        .fold(0.0_f64, f64::max);
    let maximum_height = movable
        .iter()
        .map(|component| component.size.y)
        .fold(0.0_f64, f64::max);
    let columns = ((problem.board.bounds.width() + gap_mm) / (maximum_width + gap_mm))
        .floor()
        .max(1.0) as usize;
    let slots = movable
        .iter()
        .enumerate()
        .map(|(index, component)| {
            let column = index % columns;
            let row = index / columns;
            let proposed = Vec2::new(
                problem.board.bounds.min.x
                    + maximum_width * 0.5
                    + column as f64 * (maximum_width + gap_mm),
                problem.board.bounds.min.y
                    + maximum_height * 0.5
                    + row as f64 * (maximum_height + gap_mm),
            );
            (component.id.as_str(), proposed)
        })
        .collect::<HashMap<_, _>>();
    components
        .into_iter()
        .map(|component| PlacementPose {
            component: component.id.clone(),
            position: slots
                .get(component.id.as_str())
                .copied()
                .unwrap_or(component.position),
            rotation_degrees: component.rotation_degrees,
        })
        .collect()
}

fn random_poses(problem: &Problem, seed: u64, attempt: usize) -> Vec<PlacementPose> {
    sorted_components(problem)
        .into_iter()
        .map(|component| {
            if component.constraints.movement == Movement::Fixed {
                return PlacementPose {
                    component: component.id.clone(),
                    position: component.position,
                    rotation_degrees: component.rotation_degrees,
                };
            }
            let base = mix64(seed ^ stable_string_hash(&component.id) ^ mix64(attempt as u64));
            let x = unit_float(mix64(base ^ 0x2d35_8dcc_aa6c_78a5));
            let y = unit_float(mix64(base ^ 0x8bb8_4b93_962e_acc9));
            let rotation = if component.constraints.rotation == Rotation::Fixed {
                component.rotation_degrees
            } else {
                let quarter_turn = (mix64(base ^ 0x4f1b_bcdd_7d63_219d) & 3) as f64;
                (component.rotation_degrees + quarter_turn * 90.0).rem_euclid(360.0)
            };
            PlacementPose {
                component: component.id.clone(),
                position: Vec2::new(
                    problem.board.bounds.min.x + x * problem.board.bounds.width(),
                    problem.board.bounds.min.y + y * problem.board.bounds.height(),
                ),
                rotation_degrees: rotation,
            }
        })
        .collect()
}

#[derive(Clone)]
struct ConnectivityEdge {
    first_component: String,
    first_pin: String,
    second_component: String,
    second_pin: String,
    weight: f64,
}

fn connectivity_edges(problem: &Problem) -> Vec<ConnectivityEdge> {
    let mut edges = Vec::new();
    for net in &problem.nets {
        if net.from.component != net.to.component {
            edges.push(ConnectivityEdge {
                first_component: net.from.component.clone(),
                first_pin: net.from.pin.clone(),
                second_component: net.to.component.clone(),
                second_pin: net.to.pin.clone(),
                weight: net.width.max(FEASIBILITY_TOLERANCE_MM),
            });
        }
    }
    for net in &problem.electrical_nets {
        for first in 0..net.terminals.len() {
            for second in first + 1..net.terminals.len() {
                let first_ref = &net.terminals[first];
                let second_ref = &net.terminals[second];
                if first_ref.component != second_ref.component {
                    edges.push(ConnectivityEdge {
                        first_component: first_ref.component.clone(),
                        first_pin: first_ref.pin.clone(),
                        second_component: second_ref.component.clone(),
                        second_pin: second_ref.pin.clone(),
                        weight: net.width.max(FEASIBILITY_TOLERANCE_MM),
                    });
                }
            }
        }
    }
    edges.sort_by(|first, second| {
        (
            &first.first_component,
            &first.first_pin,
            &first.second_component,
            &first.second_pin,
        )
            .cmp(&(
                &second.first_component,
                &second.first_pin,
                &second.second_component,
                &second.second_pin,
            ))
    });
    edges
}

fn barycentric_poses(problem: &Problem, iterations: usize, attraction: f64) -> Vec<PlacementPose> {
    let components = sorted_components(problem);
    let component_indexes = components
        .iter()
        .enumerate()
        .map(|(index, component)| (component.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut poses = declared_poses(problem);
    let edges = connectivity_edges(problem);
    for _ in 0..iterations {
        let mut sums = vec![Vec2::ZERO; components.len()];
        let mut weights = vec![0.0; components.len()];
        for edge in &edges {
            let first = component_indexes[edge.first_component.as_str()];
            let second = component_indexes[edge.second_component.as_str()];
            let first_arm = rotated(
                pin_offset(components[first], &edge.first_pin),
                poses[first].rotation_degrees,
            );
            let second_arm = rotated(
                pin_offset(components[second], &edge.second_pin),
                poses[second].rotation_degrees,
            );
            sums[first] = add(
                sums[first],
                scale(
                    sub(add(poses[second].position, second_arm), first_arm),
                    edge.weight,
                ),
            );
            sums[second] = add(
                sums[second],
                scale(
                    sub(add(poses[first].position, first_arm), second_arm),
                    edge.weight,
                ),
            );
            weights[first] += edge.weight;
            weights[second] += edge.weight;
        }
        let previous = poses.clone();
        for index in 0..poses.len() {
            if weights[index] == 0.0 || components[index].constraints.movement == Movement::Fixed {
                continue;
            }
            let target = scale(sums[index], 1.0 / weights[index]);
            let delta = scale(sub(target, previous[index].position), attraction);
            match components[index].constraints.movement {
                Movement::Fixed => {}
                Movement::Horizontal => poses[index].position.x += delta.x,
                Movement::Vertical => poses[index].position.y += delta.y,
                Movement::Free => poses[index].position = add(poses[index].position, delta),
            }
        }
    }
    poses
}

#[derive(Clone, Copy)]
struct HarmonicNeighbor {
    node: usize,
    weight: f64,
}

fn add_harmonic_edge(
    neighbors: &mut [Vec<HarmonicNeighbor>],
    first: usize,
    second: usize,
    weight: f64,
) {
    neighbors[first].push(HarmonicNeighbor {
        node: second,
        weight,
    });
    neighbors[second].push(HarmonicNeighbor {
        node: first,
        weight,
    });
}

/// Construct a sparse component/net incidence graph. Multi-terminal nets use
/// one virtual node rather than a quadratic terminal clique.
fn harmonic_neighbors(
    problem: &Problem,
    component_indexes: &HashMap<&str, usize>,
) -> Vec<Vec<HarmonicNeighbor>> {
    let component_count = component_indexes.len();
    let mut neighbors = vec![Vec::new(); component_count];

    let mut legacy = problem.nets.iter().collect::<Vec<_>>();
    legacy.sort_by(|first, second| first.id.cmp(&second.id));
    for net in legacy {
        if net.tension_weight == 0.0 {
            continue;
        }
        let first = component_indexes[net.from.component.as_str()];
        let second = component_indexes[net.to.component.as_str()];
        if first != second {
            add_harmonic_edge(
                &mut neighbors,
                first,
                second,
                net.width.max(FEASIBILITY_TOLERANCE_MM) * net.tension_weight,
            );
        }
    }

    let mut electrical = problem.electrical_nets.iter().collect::<Vec<_>>();
    electrical.sort_by(|first, second| first.id.cmp(&second.id));
    for net in electrical {
        if net.tension_weight == 0.0 {
            continue;
        }
        let mut terminals = net.terminals.iter().collect::<Vec<_>>();
        terminals.sort_by(|first, second| {
            (&first.component, &first.pin).cmp(&(&second.component, &second.pin))
        });
        let mut terminal_components = terminals
            .into_iter()
            .map(|terminal| component_indexes[terminal.component.as_str()])
            .collect::<Vec<_>>();
        terminal_components.dedup();
        if terminal_components.len() < 2 {
            continue;
        }
        let virtual_node = neighbors.len();
        neighbors.push(Vec::new());
        let weight = net.width.max(FEASIBILITY_TOLERANCE_MM) * net.tension_weight;
        for component in terminal_components {
            add_harmonic_edge(&mut neighbors, component, virtual_node, weight);
        }
    }
    for values in &mut neighbors {
        values.sort_by_key(|neighbor| neighbor.node);
    }
    neighbors
}

fn board_edge_order(edge: BoardEdge) -> u8 {
    match edge {
        BoardEdge::North => 0,
        BoardEdge::East => 1,
        BoardEdge::South => 2,
        BoardEdge::West => 3,
    }
}

/// Produce deterministic Dirichlet anchors from immovable components and
/// positional board-edge constraints. Contradictory anchors are deliberately
/// left for the common exact projector to reject.
fn harmonic_anchor_poses(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
) -> Result<(Vec<PlacementPose>, Vec<bool>), String> {
    let indexes = components
        .iter()
        .enumerate()
        .map(|(index, component)| (component.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut poses = declared_poses(problem);
    let mut anchors = components
        .iter()
        .map(|component| component.constraints.movement == Movement::Fixed)
        .collect::<Vec<_>>();
    let mut edge_constraints = problem
        .placement_constraints
        .iter()
        .filter_map(|constraint| match constraint {
            PlacementConstraint::FaceBoardEdge {
                component,
                edge,
                local_facing_direction_degrees,
                maximum_distance,
                ..
            } => Some((
                component.as_str(),
                *edge,
                *local_facing_direction_degrees,
                *maximum_distance,
            )),
            PlacementConstraint::MaximumDistance { .. } => None,
        })
        .collect::<Vec<_>>();
    edge_constraints.sort_by(|first, second| {
        (first.0, board_edge_order(first.1), first.2.to_bits()).cmp(&(
            second.0,
            board_edge_order(second.1),
            second.2.to_bits(),
        ))
    });

    for &(component_id, edge, local_facing, maximum_distance) in &edge_constraints {
        let index = indexes[component_id];
        let component = components[index];
        if component.constraints.rotation == Rotation::Free {
            poses[index].rotation_degrees =
                (edge.outward_angle_degrees() - local_facing).rem_euclid(360.0);
        }
        enforce_component_bounds(problem, component, &mut poses[index])?;
        let Some(maximum_distance) = maximum_distance else {
            continue;
        };
        let extent = geometry::body_extent(component, poses[index].rotation_degrees);
        match edge {
            BoardEdge::West => {
                poses[index].position.x = problem.board.bounds.min.x + extent.x + maximum_distance
            }
            BoardEdge::East => {
                poses[index].position.x = problem.board.bounds.max.x - extent.x - maximum_distance
            }
            BoardEdge::South => {
                poses[index].position.y = problem.board.bounds.min.y + extent.y + maximum_distance
            }
            BoardEdge::North => {
                poses[index].position.y = problem.board.bounds.max.y - extent.y - maximum_distance
            }
        }
        enforce_component_bounds(problem, component, &mut poses[index])?;
        anchors[index] = true;
    }
    Ok((poses, anchors))
}

fn harmonic_centers(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    iterations: usize,
) -> Result<(Vec<PlacementPose>, usize, Vec<bool>), String> {
    let component_indexes = components
        .iter()
        .enumerate()
        .map(|(index, component)| (component.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let neighbors = harmonic_neighbors(problem, &component_indexes);
    let (mut poses, component_anchors) = harmonic_anchor_poses(problem, components)?;
    let mut positions = vec![Vec2::ZERO; neighbors.len()];
    for (index, pose) in poses.iter().enumerate() {
        positions[index] = pose.position;
    }
    for node in components.len()..neighbors.len() {
        let values = &neighbors[node];
        if !values.is_empty() {
            positions[node] = scale(
                values.iter().fold(Vec2::ZERO, |sum, neighbor| {
                    add(sum, positions[neighbor.node])
                }),
                1.0 / values.len() as f64,
            );
        }
    }

    // A free Laplacian block is singular, and a singly anchored block simply
    // collapses toward that anchor. Leave both unchanged. This is a bounded
    // seed policy, not an invented boundary condition.
    let mut visited = vec![false; neighbors.len()];
    let mut active = vec![false; neighbors.len()];
    let mut underanchored_blocks_retained = 0;
    for first in 0..neighbors.len() {
        if visited[first] {
            continue;
        }
        let mut queue = VecDeque::from([first]);
        let mut block = Vec::new();
        visited[first] = true;
        while let Some(node) = queue.pop_front() {
            block.push(node);
            for neighbor in &neighbors[node] {
                if !visited[neighbor.node] {
                    visited[neighbor.node] = true;
                    queue.push_back(neighbor.node);
                }
            }
        }
        let anchors = block
            .iter()
            .copied()
            .filter(|&node| node < components.len() && component_anchors[node])
            .collect::<Vec<_>>();
        let has_distinct_boundary = anchors.iter().enumerate().any(|(index, &first_anchor)| {
            anchors[index + 1..].iter().any(|&second_anchor| {
                distance(positions[first_anchor], positions[second_anchor])
                    > FEASIBILITY_TOLERANCE_MM
            })
        });
        if !has_distinct_boundary {
            if block.iter().any(|&node| {
                node < components.len() && components[node].constraints.movement != Movement::Fixed
            }) {
                underanchored_blocks_retained += 1;
            }
            continue;
        }
        let centroid = scale(
            anchors
                .iter()
                .fold(Vec2::ZERO, |sum, &node| add(sum, positions[node])),
            1.0 / anchors.len() as f64,
        );
        for &node in &block {
            active[node] = true;
            if node >= components.len() || !component_anchors[node] {
                positions[node] = centroid;
                if node < components.len() {
                    poses[node].position = positions[node];
                    enforce_component_bounds(problem, components[node], &mut poses[node])?;
                    positions[node] = poses[node].position;
                }
            }
        }
    }

    for _ in 0..iterations {
        let previous = positions.clone();
        for node in 0..neighbors.len() {
            if !active[node]
                || (node < components.len() && component_anchors[node])
                || neighbors[node].is_empty()
            {
                continue;
            }
            let (weighted_sum, total_weight) =
                neighbors[node]
                    .iter()
                    .fold((Vec2::ZERO, 0.0), |(sum, weight), neighbor| {
                        (
                            add(sum, scale(previous[neighbor.node], neighbor.weight)),
                            weight + neighbor.weight,
                        )
                    });
            let target = scale(weighted_sum, 1.0 / total_weight);
            if node >= components.len() {
                positions[node] = target;
                continue;
            }
            match components[node].constraints.movement {
                Movement::Fixed => {}
                Movement::Horizontal => positions[node].x = target.x,
                Movement::Vertical => positions[node].y = target.y,
                Movement::Free => positions[node] = target,
            }
            // Bounds must participate in the solve: clamping only after the
            // iterations leaves connected neighbors at the unclamped location.
            poses[node].position = positions[node];
            enforce_component_bounds(problem, components[node], &mut poses[node])?;
            positions[node] = poses[node].position;
        }
    }
    for (index, pose) in poses.iter_mut().enumerate() {
        pose.position = positions[index];
    }
    Ok((
        poses,
        underanchored_blocks_retained,
        active[..components.len()].to_vec(),
    ))
}

fn harmonic_underanchored_blocks(problem: &Problem) -> Result<usize, String> {
    let components = sorted_components(problem);
    Ok(harmonic_centers(problem, &components, 0)?.1)
}

#[derive(Clone)]
struct PortAttraction {
    component: usize,
    pin: String,
    target: Vec2,
    weight: f64,
    layers: Vec<String>,
}

fn net_layers(primary: &str, allowed: &[String]) -> Vec<String> {
    let mut layers = allowed.to_vec();
    layers.push(primary.to_owned());
    layers.sort();
    layers.dedup();
    layers
}

fn port_attractions(
    problem: &Problem,
    component_indexes: &HashMap<&str, usize>,
    poses: &[PlacementPose],
) -> Vec<PortAttraction> {
    let mut result = Vec::new();
    for net in &problem.nets {
        if net.tension_weight == 0.0 {
            continue;
        }
        let first = component_indexes[net.from.component.as_str()];
        let second = component_indexes[net.to.component.as_str()];
        if first == second {
            continue;
        }
        let layers = net_layers(&net.layer, &net.allowed_layers);
        let weight = net.width.max(FEASIBILITY_TOLERANCE_MM) * net.tension_weight;
        result.push(PortAttraction {
            component: first,
            pin: net.from.pin.clone(),
            target: poses[second].position,
            weight,
            layers: layers.clone(),
        });
        result.push(PortAttraction {
            component: second,
            pin: net.to.pin.clone(),
            target: poses[first].position,
            weight,
            layers,
        });
    }
    for net in &problem.electrical_nets {
        if net.tension_weight == 0.0 {
            continue;
        }
        let mut terminals = net.terminals.iter().collect::<Vec<_>>();
        terminals.sort_by(|first, second| {
            (&first.component, &first.pin).cmp(&(&second.component, &second.pin))
        });
        let centers = terminals
            .iter()
            .map(|terminal| poses[component_indexes[terminal.component.as_str()]].position)
            .collect::<Vec<_>>();
        let total = centers.iter().copied().fold(Vec2::ZERO, add);
        let layers = net_layers(&net.layer, &net.allowed_layers);
        let weight = net.width.max(FEASIBILITY_TOLERANCE_MM) * net.tension_weight;
        for (terminal, own_center) in terminals.into_iter().zip(centers) {
            if net.terminals.len() < 2 {
                continue;
            }
            result.push(PortAttraction {
                component: component_indexes[terminal.component.as_str()],
                pin: terminal.pin.clone(),
                target: scale(
                    sub(total, own_center),
                    1.0 / (net.terminals.len() - 1) as f64,
                ),
                weight,
                layers: layers.clone(),
            });
        }
    }
    result.sort_by(|first, second| {
        (
            first.component,
            &first.pin,
            first.target.x.to_bits(),
            first.target.y.to_bits(),
        )
            .cmp(&(
                second.component,
                &second.pin,
                second.target.x.to_bits(),
                second.target.y.to_bits(),
            ))
    });
    result
}

fn physical_port_offsets(
    component: &layout_trace_model::model::Component,
    pin_id: &str,
    layers: &[String],
) -> Vec<Vec2> {
    let pin = component
        .pins
        .iter()
        .find(|pin| pin.id == pin_id)
        .expect("problem schema validated pin reference");
    let mut offsets = pin
        .pads
        .iter()
        .filter(|pad| layers.binary_search(&pad.layer).is_ok())
        .map(|pad| pin.pad_local_center(pad))
        .collect::<Vec<_>>();
    if offsets.is_empty() {
        offsets.push(pin.offset);
    }
    offsets
}

fn choose_port_orientations(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    harmonic_active: &[bool],
    poses: &mut [PlacementPose],
) {
    let component_indexes = components
        .iter()
        .enumerate()
        .map(|(index, component)| (component.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let attractions = port_attractions(problem, &component_indexes, poses);
    let edge_constrained = problem
        .placement_constraints
        .iter()
        .filter_map(|constraint| match constraint {
            PlacementConstraint::FaceBoardEdge { component, .. } => Some(component.as_str()),
            PlacementConstraint::MaximumDistance { .. } => None,
        })
        .collect::<std::collections::HashSet<_>>();
    for (index, component) in components.iter().enumerate() {
        if component.constraints.rotation == Rotation::Fixed
            || edge_constrained.contains(component.id.as_str())
            || !harmonic_active[index]
        {
            continue;
        }
        let incident = attractions
            .iter()
            .filter(|attraction| attraction.component == index)
            .collect::<Vec<_>>();
        if incident.is_empty() {
            continue;
        }
        let candidates = [0.0, 90.0, 180.0, 270.0]
            .map(|turn| (component.rotation_degrees + turn).rem_euclid(360.0));
        let mut best = (f64::INFINITY, candidates[0]);
        for candidate in candidates {
            let score = incident.iter().fold(0.0, |score, attraction| {
                let target_delta = sub(attraction.target, poses[index].position);
                let port_cost =
                    physical_port_offsets(component, &attraction.pin, &attraction.layers)
                        .into_iter()
                        .map(|offset| squared_length(sub(rotated(offset, candidate), target_delta)))
                        .fold(f64::INFINITY, f64::min);
                score + attraction.weight * port_cost
            });
            if score < best.0 {
                best = (score, candidate);
            }
        }
        poses[index].rotation_degrees = best.1;
    }
}

/// Pair discovery is deliberately isolated so a uniform-grid or sweep broad
/// phase can replace this bounded quadratic CPU implementation.
trait ComponentPairEnumerator {
    fn enumerate(&self, component_count: usize) -> Vec<(usize, usize)>;
}

struct AllComponentPairs;

impl ComponentPairEnumerator for AllComponentPairs {
    fn enumerate(&self, component_count: usize) -> Vec<(usize, usize)> {
        let mut pairs = Vec::with_capacity(component_count.saturating_mul(component_count) / 2);
        for first in 0..component_count {
            for second in first + 1..component_count {
                pairs.push((first, second));
            }
        }
        pairs
    }
}

fn connected_component_pairs(
    problem: &Problem,
    component_indexes: &HashMap<&str, usize>,
) -> Vec<(usize, usize)> {
    let mut pairs = std::collections::BTreeSet::new();
    for net in &problem.nets {
        let first = component_indexes[net.from.component.as_str()];
        let second = component_indexes[net.to.component.as_str()];
        if first != second {
            pairs.insert((first.min(second), first.max(second)));
        }
    }
    for net in &problem.electrical_nets {
        let mut components = net
            .terminals
            .iter()
            .map(|terminal| component_indexes[terminal.component.as_str()])
            .collect::<Vec<_>>();
        components.sort_unstable();
        components.dedup();
        for adjacent in components.windows(2) {
            pairs.insert((adjacent[0], adjacent[1]));
        }
    }
    pairs.into_iter().collect()
}

fn oriented_axes(rotation_degrees: f64) -> [Vec2; 2] {
    [
        rotated(Vec2::new(1.0, 0.0), rotation_degrees),
        rotated(Vec2::new(0.0, 1.0), rotation_degrees),
    ]
}

fn body_support(
    component: &layout_trace_model::model::Component,
    pose: &PlacementPose,
    axis: Vec2,
) -> f64 {
    let (_, size) = placement_envelope(component);
    let axes = oriented_axes(pose.rotation_degrees);
    0.5 * size.x * dot(axis, axes[0]).abs() + 0.5 * size.y * dot(axis, axes[1]).abs()
}

/// Conservative local rectangle used only for inter-component placement
/// exclusion. A component body is not always its full physical extent: pads
/// on imported modules can project beyond it. Ignoring those pads lets a
/// body-feasible placement become an immediate native copper short.
fn placement_envelope(component: &layout_trace_model::model::Component) -> (Vec2, Vec2) {
    let mut minimum = Vec2::new(-component.size.x / 2.0, -component.size.y / 2.0);
    let mut maximum = Vec2::new(component.size.x / 2.0, component.size.y / 2.0);
    for pin in &component.pins {
        for pad in &pin.pads {
            let center = pin.pad_local_center(pad);
            let extent = match pad.shape {
                CopperShape::Circle { diameter } => Vec2::new(diameter / 2.0, diameter / 2.0),
                CopperShape::Rect {
                    size,
                    rotation_degrees,
                } => {
                    let radians = rotation_degrees.to_radians();
                    let cosine = radians.cos().abs();
                    let sine = radians.sin().abs();
                    Vec2::new(
                        0.5 * (size.x * cosine + size.y * sine),
                        0.5 * (size.x * sine + size.y * cosine),
                    )
                }
            };
            minimum.x = minimum.x.min(center.x - extent.x);
            minimum.y = minimum.y.min(center.y - extent.y);
            maximum.x = maximum.x.max(center.x + extent.x);
            maximum.y = maximum.y.max(center.y + extent.y);
        }
    }
    (scale(add(minimum, maximum), 0.5), sub(maximum, minimum))
}

fn placement_envelope_center(
    component: &layout_trace_model::model::Component,
    pose: &PlacementPose,
) -> Vec2 {
    let (local_center, _) = placement_envelope(component);
    add(pose.position, rotated(local_center, pose.rotation_degrees))
}

fn stable_pair_direction(first: &str, second: &str) -> Vec2 {
    let hash = stable_string_hash(&format!("{}\0{}", first.min(second), first.max(second)));
    let angle = unit_float(mix64(hash)) * std::f64::consts::TAU;
    Vec2::new(angle.cos(), angle.sin())
}

/// Minimum separating translation for two oriented component bodies,
/// including the board clearance. The normal points from `second` to `first`.
fn body_overlap_correction(
    first: &layout_trace_model::model::Component,
    first_pose: &PlacementPose,
    second: &layout_trace_model::model::Component,
    second_pose: &PlacementPose,
    clearance: f64,
) -> Option<(Vec2, f64)> {
    if first.placement_geometry.is_some() || second.placement_geometry.is_some() {
        return geometry::overlap_correction(first, first_pose, second, second_pose, clearance);
    }
    let delta = sub(
        placement_envelope_center(first, first_pose),
        placement_envelope_center(second, second_pose),
    );
    let stable = stable_pair_direction(&first.id, &second.id);
    let first_axes = oriented_axes(first_pose.rotation_degrees);
    let second_axes = oriented_axes(second_pose.rotation_degrees);
    let mut minimum = (Vec2::ZERO, f64::INFINITY);
    for mut axis in [first_axes[0], first_axes[1], second_axes[0], second_axes[1]] {
        if dot(delta, axis) < -FEASIBILITY_TOLERANCE_MM
            || (dot(delta, axis).abs() <= FEASIBILITY_TOLERANCE_MM && dot(stable, axis) < 0.0)
        {
            axis = scale(axis, -1.0);
        }
        let penetration = body_support(first, first_pose, axis)
            + body_support(second, second_pose, axis)
            + clearance
            - dot(delta, axis).abs();
        if penetration <= FEASIBILITY_TOLERANCE_MM {
            return None;
        }
        if penetration < minimum.1 {
            minimum = (axis, penetration);
        }
    }
    Some(minimum)
}

fn apply_separation(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    anchors: &[bool],
    poses: &mut [PlacementPose],
    pair: (usize, usize),
    correction: (Vec2, f64),
) -> Result<(), String> {
    let (first, second) = pair;
    let (normal, residual) = correction;
    if residual <= FEASIBILITY_TOLERANCE_MM {
        return Ok(());
    }
    let first_gradient = if anchors[first] {
        Vec2::ZERO
    } else {
        movement_projection(components[first].constraints.movement, normal)
    };
    let second_gradient = if anchors[second] {
        Vec2::ZERO
    } else {
        movement_projection(components[second].constraints.movement, scale(normal, -1.0))
    };
    let denominator = squared_length(first_gradient) + squared_length(second_gradient);
    if denominator <= f64::EPSILON {
        return Ok(());
    }
    let correction = residual / denominator;
    poses[first].position = add(poses[first].position, scale(first_gradient, correction));
    poses[second].position = add(poses[second].position, scale(second_gradient, correction));
    enforce_component_bounds(problem, components[first], &mut poses[first])?;
    enforce_component_bounds(problem, components[second], &mut poses[second])?;
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct PlacementContact {
    pub first: String,
    pub second: String,
    /// Direction of the correction applied to the first component.
    pub normal: Vec2,
    /// Separating translation requested by the geometry model, not area.
    pub residual_mm: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PlacementSeedBroadPhaseWork {
    /// Cheap cached projection-certificate comparisons, separate from exact queries.
    pub comparisons: usize,
    pub rejected_disjoint: usize,
    #[serde(skip_serializing_if = "usize_is_zero")]
    pub interval_preparations: usize,
    #[serde(skip_serializing_if = "usize_is_zero")]
    pub row_certificates: usize,
    #[serde(skip_serializing_if = "usize_is_zero")]
    pub skipped_samples: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlacementLegalizationFrame {
    pub attempt: usize,
    /// Zero is the bounded/oriented seed before any legalization sweep.
    pub sweep: usize,
    pub pair_checks: usize,
    pub poses: Vec<PlacementPose>,
    /// Contacts recomputed simultaneously at the saved poses. They can differ
    /// from the contacts encountered during sequential corrections. This list
    /// excludes the optional connected-pair spacing floor and relational rows.
    pub contacts: Vec<PlacementContact>,
    pub maximum_encountered_residual_mm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coupled: Option<CoupledLegalizationFrame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed_projection: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed_projection_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed_projection_broad_phase: Option<PlacementSeedBroadPhaseWork>,
}

#[allow(clippy::too_many_arguments)]
fn observe_legalization(
    observer: Option<&dyn Fn(PlacementLegalizationFrame)>,
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    poses: &[PlacementPose],
    attempt: usize,
    sweep: usize,
    pair_checks: usize,
    maximum_encountered_residual_mm: f64,
) {
    let Some(observer) = observer else { return };
    let contacts = AllComponentPairs
        .enumerate(components.len())
        .into_iter()
        .filter_map(|(first, second)| {
            body_overlap_correction(
                components[first],
                &poses[first],
                components[second],
                &poses[second],
                problem.rules.clearance,
            )
            .map(|(normal, residual_mm)| PlacementContact {
                first: components[first].id.clone(),
                second: components[second].id.clone(),
                normal,
                residual_mm,
            })
        })
        .collect();
    observer(PlacementLegalizationFrame {
        attempt,
        sweep,
        pair_checks,
        poses: poses.to_vec(),
        contacts,
        maximum_encountered_residual_mm,
        coupled: None,
        seed_projection: None,
        seed_projection_seconds: None,
        seed_projection_broad_phase: None,
    });
}

#[allow(clippy::too_many_arguments)]
fn legalize_harmonic_poses(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    poses: &mut [PlacementPose],
    maximum_sweeps: usize,
    maximum_pair_checks: usize,
    initial_pair_checks: usize,
    coupled_config: Option<&CoupledLegalizationConfig>,
    connected_pair_spacing_floor: bool,
    attempt: usize,
    observer: Option<&dyn Fn(PlacementLegalizationFrame)>,
) -> Result<(), String> {
    let indexes = components
        .iter()
        .enumerate()
        .map(|(index, component)| (component.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let (_, anchors) = harmonic_anchor_poses(problem, components)?;
    let connected_pairs = if connected_pair_spacing_floor {
        connected_component_pairs(problem, &indexes)
    } else {
        Vec::new()
    };
    let body_pairs = AllComponentPairs.enumerate(components.len());
    let checks_per_sweep = connected_pairs.len() + body_pairs.len();
    let mut pair_checks = initial_pair_checks;
    observe_legalization(
        observer,
        problem,
        components,
        poses,
        attempt,
        0,
        pair_checks,
        0.0,
    );
    for sweep in 0..maximum_sweeps {
        if pair_checks.saturating_add(checks_per_sweep) > maximum_pair_checks {
            return Err(format!(
                "harmonic placement legalization exhausted its {maximum_pair_checks} pair-check budget"
            ));
        }
        pair_checks += checks_per_sweep;
        let mut maximum_residual = 0.0_f64;

        // A legality floor only: connected components that are already far
        // enough are never attracted or globally spread.
        for &(first, second) in &connected_pairs {
            let delta = sub(poses[first].position, poses[second].position);
            let current = vector_length(delta);
            let normal = if current > FEASIBILITY_TOLERANCE_MM {
                scale(delta, 1.0 / current)
            } else {
                stable_pair_direction(&components[first].id, &components[second].id)
            };
            let minimum = body_support(components[first], &poses[first], normal)
                + body_support(components[second], &poses[second], normal)
                + problem.rules.clearance;
            let residual = minimum - current;
            maximum_residual = maximum_residual.max(residual);
            apply_separation(
                problem,
                components,
                &anchors,
                poses,
                (first, second),
                (normal, residual),
            )?;
        }

        for &(first, second) in &body_pairs {
            if let Some((normal, residual)) = body_overlap_correction(
                components[first],
                &poses[first],
                components[second],
                &poses[second],
                problem.rules.clearance,
            ) {
                maximum_residual = maximum_residual.max(residual);
                apply_separation(
                    problem,
                    components,
                    &anchors,
                    poses,
                    (first, second),
                    (normal, residual),
                )?;
            }
        }
        observe_legalization(
            observer,
            problem,
            components,
            poses,
            attempt,
            sweep + 1,
            pair_checks,
            maximum_residual,
        );
        if maximum_residual <= FEASIBILITY_TOLERANCE_MM {
            return Ok(());
        }
        if let Some(coupled) = coupled_config
            && sweep + 1 == coupled.start_sweep
        {
            let observe =
                |candidate: &[PlacementPose], checks, evidence: CoupledLegalizationFrame| {
                    if let Some(observer) = observer {
                        observe_legalization(
                            Some(&|mut frame: PlacementLegalizationFrame| {
                                frame.coupled = Some(evidence.clone());
                                observer(frame);
                            }),
                            problem,
                            components,
                            candidate,
                            attempt,
                            sweep + 1,
                            checks,
                            0.0,
                        );
                    }
                };
            if coupled_legalization::propose(
                problem,
                components,
                &anchors,
                poses,
                &connected_pairs,
                coupled,
                &mut pair_checks,
                maximum_pair_checks,
                &observe,
            ) {
                return Ok(());
            }
        }
    }
    validate_placement_exclusions(problem, poses).map_err(|error| {
        format!(
            "harmonic placement legalization did not converge in {maximum_sweeps} sweeps: {error}"
        )
    })
}

pub(crate) fn validate_placement_exclusions(
    problem: &Problem,
    poses: &[PlacementPose],
) -> Result<(), String> {
    validate_placement_exclusions_with_clearance(problem, poses, false)
}

/// Reject overlap of component placement envelopes. Copper-bearing component
/// pairs also receive board-rule clearance; unlike initial-placement
/// legalization, non-copper fixtures receive no additional clearance. This is
/// the import gate for externally generated placements and pressure
/// transactions.
pub fn validate_physical_placement_exclusions(
    problem: &Problem,
    poses: &[PlacementPose],
) -> Result<(), String> {
    validate_placement_exclusions_with_clearance(problem, poses, true)
}

fn validate_placement_exclusions_with_clearance(
    problem: &Problem,
    poses: &[PlacementPose],
    copper_bearing_pairs_only: bool,
) -> Result<(), String> {
    let components = sorted_components(problem);
    if components.len() != poses.len() {
        return Err("placement exclusion validation requires every component pose".into());
    }
    for (first, second) in AllComponentPairs.enumerate(components.len()) {
        let both_have_copper = components[first]
            .pins
            .iter()
            .any(|pin| !pin.pads.is_empty())
            && components[second]
                .pins
                .iter()
                .any(|pin| !pin.pads.is_empty());
        let clearance = if !copper_bearing_pairs_only || both_have_copper {
            problem.rules.clearance
        } else {
            0.0
        };
        if body_overlap_correction(
            components[first],
            &poses[first],
            components[second],
            &poses[second],
            clearance,
        )
        .is_some()
        {
            return Err(format!(
                "component placement envelopes {} and {} violate exclusion",
                components[first].id, components[second].id
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn harmonic_port_poses(
    problem: &Problem,
    iterations: usize,
    legalization_sweeps: usize,
    maximum_pair_checks: usize,
    coupled_config: Option<&CoupledLegalizationConfig>,
    connected_pair_spacing_floor: bool,
    underanchored_seed: &UnderanchoredSeedPolicy,
    fixed_obstacle_seed_projection: bool,
    largest_first_seed_insertion: bool,
    skip_collision_intervals: bool,
    seed_extra_pad_gap_mm: f64,
    seed: u64,
    attempt: usize,
    observer: Option<&dyn Fn(PlacementLegalizationFrame)>,
) -> Result<Vec<PlacementPose>, String> {
    let components = sorted_components(problem);
    let (mut poses, _, harmonic_active) = harmonic_centers(problem, &components, iterations)?;
    let fallback = match underanchored_seed {
        UnderanchoredSeedPolicy::Declared => None,
        UnderanchoredSeedPolicy::GridPacking { gap_mm } => Some(grid_poses(problem, *gap_mm)),
        UnderanchoredSeedPolicy::Random { .. } => Some(random_poses(problem, seed, attempt)),
    };
    if let Some(fallback) = fallback {
        for index in 0..components.len() {
            if !harmonic_active[index] && components[index].constraints.movement != Movement::Fixed
            {
                poses[index] = fallback[index].clone();
            }
        }
    }
    choose_port_orientations(problem, &components, &harmonic_active, &mut poses);
    let mut seed_pair_checks = 0;
    if fixed_obstacle_seed_projection {
        let observe_seed = |stage,
                            poses: &[PlacementPose],
                            pair_checks,
                            seconds,
                            broad: &PlacementSeedBroadPhaseWork| {
            if let Some(observer) = observer {
                observe_legalization(
                    Some(&|mut frame: PlacementLegalizationFrame| {
                        frame.seed_projection = Some(stage);
                        frame.seed_projection_seconds = Some(seconds);
                        frame.seed_projection_broad_phase = Some(broad.clone());
                        observer(frame);
                    }),
                    problem,
                    &components,
                    poses,
                    attempt,
                    0,
                    pair_checks,
                    0.0,
                );
            }
        };
        observe_seed(
            if largest_first_seed_insertion {
                "before largest-first seed insertion"
            } else {
                "before fixed-obstacle projection"
            },
            &poses,
            0,
            0.0,
            &PlacementSeedBroadPhaseWork::default(),
        );
        let mut broad_phase_work = PlacementSeedBroadPhaseWork::default();
        let started = std::time::Instant::now();
        let result = fixed_obstacle_seed::project_with_pad_gap(
            problem,
            &components,
            &mut poses,
            &mut seed_pair_checks,
            maximum_pair_checks,
            largest_first_seed_insertion,
            skip_collision_intervals,
            seed_extra_pad_gap_mm,
            &mut broad_phase_work,
        );
        observe_seed(
            match (largest_first_seed_insertion, result.is_ok()) {
                (true, true) => "after largest-first seed insertion",
                (true, false) => "largest-first seed insertion incomplete",
                (false, true) => "after fixed-obstacle projection",
                (false, false) => "fixed-obstacle projection incomplete",
            },
            &poses,
            seed_pair_checks,
            started.elapsed().as_secs_f64(),
            &broad_phase_work,
        );
        result?;
    }
    legalize_harmonic_poses(
        problem,
        &components,
        &mut poses,
        legalization_sweeps,
        maximum_pair_checks,
        seed_pair_checks,
        coupled_config,
        connected_pair_spacing_floor,
        attempt,
        observer,
    )?;
    Ok(poses)
}

/// Validate externally supplied rigid-body poses without projecting or
/// repairing them. This is the fail-closed import path for serialized exact
/// solutions: fixed/axis-limited motion, fixed rotation, board/region bounds,
/// and relational placement constraints must already hold.
pub fn validate_serialized_placement_poses(
    problem: &Problem,
    poses: &[PlacementPose],
) -> Result<(), String> {
    problem.check_schema()?;
    let components = sorted_components(problem);
    let component_indexes = components
        .iter()
        .enumerate()
        .map(|(index, component)| (component.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    if poses.len() != components.len() {
        return Err("serialized placement must contain exactly one pose per component".into());
    }
    let mut supplied = BTreeMap::new();
    for pose in poses {
        if !component_indexes.contains_key(pose.component.as_str()) {
            return Err(format!(
                "serialized placement contains unknown component {}",
                pose.component
            ));
        }
        if !pose.position.x.is_finite()
            || !pose.position.y.is_finite()
            || !pose.rotation_degrees.is_finite()
        {
            return Err(format!(
                "serialized placement for component {} is not finite",
                pose.component
            ));
        }
        if supplied.insert(pose.component.as_str(), pose).is_some() {
            return Err(format!(
                "serialized placement contains component {} more than once",
                pose.component
            ));
        }
    }
    let ordered = components
        .iter()
        .map(|component| {
            supplied
                .get(component.id.as_str())
                .copied()
                .cloned()
                .ok_or_else(|| format!("serialized placement omits component {}", component.id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_feasibility(problem, &components, &component_indexes, &ordered)?;
    validate_physical_placement_exclusions(problem, &ordered)
}

fn project_and_validate(
    problem: &Problem,
    proposal: Vec<PlacementPose>,
    maximum_sweeps: usize,
) -> Result<(Vec<PlacementPose>, usize, usize, usize), String> {
    let components = sorted_components(problem);
    let component_indexes = components
        .iter()
        .enumerate()
        .map(|(index, component)| (component.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut proposed = BTreeMap::new();
    for pose in proposal {
        if !component_indexes.contains_key(pose.component.as_str()) {
            return Err(format!(
                "initial placement proposed unknown component {}",
                pose.component
            ));
        }
        if proposed.insert(pose.component.clone(), pose).is_some() {
            return Err("initial placement proposed a component more than once".into());
        }
    }
    let mut poses = components
        .iter()
        .map(|component| {
            proposed
                .remove(&component.id)
                .ok_or_else(|| format!("initial placement omitted component {}", component.id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let fixed_anchors = components
        .iter()
        .map(|component| component.constraints.movement == Movement::Fixed)
        .collect::<Vec<_>>();
    let body_pairs = AllComponentPairs.enumerate(components.len());
    for (component, pose) in components.iter().zip(&mut poses) {
        if !pose.position.x.is_finite()
            || !pose.position.y.is_finite()
            || !pose.rotation_degrees.is_finite()
        {
            return Err(format!(
                "initial placement proposed a non-finite pose for {}",
                component.id
            ));
        }
        if component.constraints.rotation == Rotation::Fixed {
            pose.rotation_degrees = component.rotation_degrees;
        } else {
            pose.rotation_degrees = pose.rotation_degrees.rem_euclid(360.0);
        }
        enforce_component_bounds(problem, component, pose)?;
    }

    let mut used_sweeps = 0;
    let mut hard_constraint_checks = 0;
    let mut body_pair_checks = 0;
    for sweep in 0..maximum_sweeps {
        used_sweeps = sweep + 1;
        let mut maximum_residual = 0.0_f64;
        for constraint in &problem.placement_constraints {
            hard_constraint_checks += 1;
            match constraint {
                PlacementConstraint::MaximumDistance {
                    first,
                    second,
                    maximum,
                } => {
                    let first_index = component_indexes[first.component.as_str()];
                    let second_index = component_indexes[second.component.as_str()];
                    let first_point =
                        anchor_point(components[first_index], &poses[first_index], first);
                    let second_point =
                        anchor_point(components[second_index], &poses[second_index], second);
                    let delta = sub(first_point, second_point);
                    let current_distance = vector_length(delta);
                    let residual = current_distance - *maximum;
                    maximum_residual = maximum_residual.max(residual);
                    if residual <= FEASIBILITY_TOLERANCE_MM
                        || current_distance <= FEASIBILITY_TOLERANCE_MM
                        || first_index == second_index
                    {
                        continue;
                    }
                    let normal = scale(delta, 1.0 / current_distance);
                    let first_gradient =
                        movement_projection(components[first_index].constraints.movement, normal);
                    let second_gradient = movement_projection(
                        components[second_index].constraints.movement,
                        scale(normal, -1.0),
                    );
                    let denominator =
                        squared_length(first_gradient) + squared_length(second_gradient);
                    if denominator <= f64::EPSILON {
                        continue;
                    }
                    let correction = residual / denominator;
                    poses[first_index].position = sub(
                        poses[first_index].position,
                        scale(first_gradient, correction),
                    );
                    poses[second_index].position = sub(
                        poses[second_index].position,
                        scale(second_gradient, correction),
                    );
                    enforce_component_bounds(
                        problem,
                        components[first_index],
                        &mut poses[first_index],
                    )?;
                    enforce_component_bounds(
                        problem,
                        components[second_index],
                        &mut poses[second_index],
                    )?;
                }
                PlacementConstraint::FaceBoardEdge {
                    component,
                    edge,
                    local_facing_direction_degrees,
                    angular_tolerance_degrees,
                    maximum_distance,
                } => {
                    let index = component_indexes[component.as_str()];
                    let record = components[index];
                    let rotation_correction = edge_rotation_correction_degrees(
                        poses[index].rotation_degrees,
                        *local_facing_direction_degrees,
                        *edge,
                        *angular_tolerance_degrees,
                    );
                    maximum_residual = maximum_residual.max(rotation_correction.abs());
                    if record.constraints.rotation == Rotation::Free {
                        poses[index].rotation_degrees =
                            (poses[index].rotation_degrees + rotation_correction).rem_euclid(360.0);
                    }
                    enforce_component_bounds(problem, record, &mut poses[index])?;
                    if let Some(maximum_distance) = maximum_distance {
                        let current = component_edge_distance(
                            *edge,
                            problem.board.bounds,
                            poses[index].position,
                            record,
                            poses[index].rotation_degrees,
                        );
                        let residual = current - maximum_distance;
                        maximum_residual = maximum_residual.max(residual);
                        if residual > FEASIBILITY_TOLERANCE_MM {
                            let correction = movement_projection(
                                record.constraints.movement,
                                edge_inward_translation(*edge, residual),
                            );
                            poses[index].position = add(poses[index].position, correction);
                            enforce_component_bounds(problem, record, &mut poses[index])?;
                        }
                    }
                }
            }
        }
        // Relational projection may create a placement overlap even when the
        // proposal was initially legal. Alternate exact oriented-body
        // separation in the common loop so every policy receives the same
        // feasibility treatment.
        for &(first, second) in &body_pairs {
            body_pair_checks += 1;
            if let Some((normal, residual)) = body_overlap_correction(
                components[first],
                &poses[first],
                components[second],
                &poses[second],
                problem.rules.clearance,
            ) {
                maximum_residual = maximum_residual.max(residual);
                apply_separation(
                    problem,
                    &components,
                    &fixed_anchors,
                    &mut poses,
                    (first, second),
                    (normal, residual),
                )?;
            }
        }
        if maximum_residual <= FEASIBILITY_TOLERANCE_MM {
            break;
        }
    }
    validate_feasibility(problem, &components, &component_indexes, &poses)?;
    validate_placement_exclusions(problem, &poses)?;
    Ok((poses, used_sweeps, hard_constraint_checks, body_pair_checks))
}

fn enforce_component_bounds(
    problem: &Problem,
    component: &layout_trace_model::model::Component,
    pose: &mut PlacementPose,
) -> Result<(), String> {
    match component.constraints.movement {
        Movement::Fixed => pose.position = component.position,
        Movement::Horizontal => pose.position.y = component.position.y,
        Movement::Vertical => pose.position.x = component.position.x,
        Movement::Free => {}
    }
    let allowed = if let Some(region) = component.constraints.region {
        layout_trace_model::model::Rect {
            min: Vec2::new(
                problem.board.bounds.min.x.max(region.min.x),
                problem.board.bounds.min.y.max(region.min.y),
            ),
            max: Vec2::new(
                problem.board.bounds.max.x.min(region.max.x),
                problem.board.bounds.max.y.min(region.max.y),
            ),
        }
    } else {
        problem.board.bounds
    };
    let (minimum, maximum) = geometry::position_limits(problem, component, pose, allowed);
    if minimum.x > maximum.x + FEASIBILITY_TOLERANCE_MM
        || minimum.y > maximum.y + FEASIBILITY_TOLERANCE_MM
    {
        return Err(format!(
            "component {} cannot fit in its allowed region",
            component.id
        ));
    }
    if matches!(
        component.constraints.movement,
        Movement::Horizontal | Movement::Free
    ) {
        pose.position.x = pose.position.x.clamp(minimum.x, maximum.x.max(minimum.x));
    }
    if matches!(
        component.constraints.movement,
        Movement::Vertical | Movement::Free
    ) {
        pose.position.y = pose.position.y.clamp(minimum.y, maximum.y.max(minimum.y));
    }
    Ok(())
}

fn validate_feasibility(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    component_indexes: &HashMap<&str, usize>,
    poses: &[PlacementPose],
) -> Result<(), String> {
    for (component, pose) in components.iter().zip(poses) {
        let bounds = component
            .constraints
            .region
            .map_or(problem.board.bounds, |region| {
                layout_trace_model::model::Rect {
                    min: Vec2::new(
                        problem.board.bounds.min.x.max(region.min.x),
                        problem.board.bounds.min.y.max(region.min.y),
                    ),
                    max: Vec2::new(
                        problem.board.bounds.max.x.min(region.max.x),
                        problem.board.bounds.max.y.min(region.max.y),
                    ),
                }
            });
        let (minimum, maximum) = geometry::position_limits(problem, component, pose, bounds);
        if pose.position.x < minimum.x - FEASIBILITY_TOLERANCE_MM
            || pose.position.x > maximum.x + FEASIBILITY_TOLERANCE_MM
            || pose.position.y < minimum.y - FEASIBILITY_TOLERANCE_MM
            || pose.position.y > maximum.y + FEASIBILITY_TOLERANCE_MM
        {
            return Err(format!(
                "component {} violates its board/region bounds",
                component.id
            ));
        }
        match component.constraints.movement {
            Movement::Fixed
                if distance(pose.position, component.position) > FEASIBILITY_TOLERANCE_MM =>
            {
                return Err(format!("fixed component {} moved", component.id));
            }
            Movement::Horizontal
                if (pose.position.y - component.position.y).abs() > FEASIBILITY_TOLERANCE_MM =>
            {
                return Err(format!(
                    "horizontal component {} moved vertically",
                    component.id
                ));
            }
            Movement::Vertical
                if (pose.position.x - component.position.x).abs() > FEASIBILITY_TOLERANCE_MM =>
            {
                return Err(format!(
                    "vertical component {} moved horizontally",
                    component.id
                ));
            }
            _ => {}
        }
        if component.constraints.rotation == Rotation::Fixed
            && angle_distance(pose.rotation_degrees, component.rotation_degrees)
                > FEASIBILITY_TOLERANCE_MM
        {
            return Err(format!("fixed-rotation component {} rotated", component.id));
        }
    }
    for constraint in &problem.placement_constraints {
        match constraint {
            PlacementConstraint::MaximumDistance {
                first,
                second,
                maximum,
            } => {
                let first_index = component_indexes[first.component.as_str()];
                let second_index = component_indexes[second.component.as_str()];
                let actual = distance(
                    anchor_point(components[first_index], &poses[first_index], first),
                    anchor_point(components[second_index], &poses[second_index], second),
                );
                if actual > maximum + FEASIBILITY_TOLERANCE_MM {
                    return Err(format!(
                        "maximum-distance constraint {} -> {} remains infeasible ({actual:.6} > {maximum:.6} mm)",
                        first.component, second.component
                    ));
                }
            }
            PlacementConstraint::FaceBoardEdge {
                component,
                edge,
                local_facing_direction_degrees,
                angular_tolerance_degrees,
                maximum_distance,
            } => {
                let index = component_indexes[component.as_str()];
                let angular_error = edge_facing_error_degrees(
                    poses[index].rotation_degrees,
                    *local_facing_direction_degrees,
                    *edge,
                );
                if angular_error > angular_tolerance_degrees + FEASIBILITY_TOLERANCE_MM {
                    return Err(format!(
                        "component {component} does not face the {edge:?} edge"
                    ));
                }
                if let Some(maximum_distance) = maximum_distance {
                    let actual = component_edge_distance(
                        *edge,
                        problem.board.bounds,
                        poses[index].position,
                        components[index],
                        poses[index].rotation_degrees,
                    );
                    if actual > maximum_distance + FEASIBILITY_TOLERANCE_MM {
                        return Err(format!(
                            "component {component} exceeds its maximum {edge:?} edge distance"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn anchor_point(
    component: &layout_trace_model::model::Component,
    pose: &PlacementPose,
    anchor: &PlacementAnchor,
) -> Vec2 {
    let offset = anchor
        .pin
        .as_deref()
        .map_or(Vec2::ZERO, |pin| pin_offset(component, pin));
    add(pose.position, rotated(offset, pose.rotation_degrees))
}

fn pin_offset(component: &layout_trace_model::model::Component, pin: &str) -> Vec2 {
    component
        .pins
        .iter()
        .find(|candidate| candidate.id == pin)
        .expect("problem schema validated pin reference")
        .offset
}

fn pin_ref_point(
    components: &HashMap<&str, &layout_trace_model::model::Component>,
    poses: &HashMap<&str, &PlacementPose>,
    reference: &PinRef,
) -> Vec2 {
    let component = components[reference.component.as_str()];
    let pose = poses[reference.component.as_str()];
    add(
        pose.position,
        rotated(pin_offset(component, &reference.pin), pose.rotation_degrees),
    )
}

fn connectivity_distance(problem: &Problem, poses: &[PlacementPose]) -> Result<f64, String> {
    let components = problem
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<HashMap<_, _>>();
    let poses = poses
        .iter()
        .map(|pose| (pose.component.as_str(), pose))
        .collect::<HashMap<_, _>>();
    if poses.len() != problem.components.len() {
        return Err("connectivity metric requires exactly one pose per component".into());
    }
    let mut total = 0.0;
    for net in &problem.nets {
        total += net.width.max(FEASIBILITY_TOLERANCE_MM)
            * distance(
                pin_ref_point(&components, &poses, &net.from),
                pin_ref_point(&components, &poses, &net.to),
            );
    }
    for net in &problem.electrical_nets {
        for first in 0..net.terminals.len() {
            for second in first + 1..net.terminals.len() {
                total += net.width.max(FEASIBILITY_TOLERANCE_MM)
                    * distance(
                        pin_ref_point(&components, &poses, &net.terminals[first]),
                        pin_ref_point(&components, &poses, &net.terminals[second]),
                    );
            }
        }
    }
    Ok(total)
}

#[derive(Clone)]
struct PlacementDemandSegment {
    connection: String,
    start: Vec2,
    finish: Vec2,
    width: f64,
}

/// Rasterize straight ratsnest demand into a flat scalar field.
///
/// The field deliberately contains no obstacle or layer assumptions. It is a
/// cheap placement diagnostic and a future CPU/GPU backend boundary, not a
/// routing certificate. Legacy branches define the demand graph when present;
/// electrical nets absent from that graph use a deterministic rooted star.
pub fn placement_demand_evidence(
    problem: &Problem,
    poses: &[PlacementPose],
    grid_size: [usize; 2],
) -> Result<PlacementDemandEvidence, String> {
    if grid_size[0] == 0 || grid_size[1] == 0 {
        return Err("placement demand grid dimensions must be positive".into());
    }
    let cell_count = grid_size[0]
        .checked_mul(grid_size[1])
        .ok_or_else(|| "placement demand grid dimensions overflow".to_string())?;
    let components = problem
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<HashMap<_, _>>();
    let poses = poses
        .iter()
        .map(|pose| (pose.component.as_str(), pose))
        .collect::<HashMap<_, _>>();
    if poses.len() != problem.components.len() {
        return Err("placement demand requires exactly one pose per component".into());
    }

    let mut segments = Vec::new();
    let mut represented_connections = std::collections::BTreeSet::new();
    for net in &problem.nets {
        let connection = net
            .electrical_net
            .as_deref()
            .unwrap_or(net.id.as_str())
            .to_string();
        represented_connections.insert(connection.clone());
        segments.push(PlacementDemandSegment {
            connection,
            start: pin_ref_point(&components, &poses, &net.from),
            finish: pin_ref_point(&components, &poses, &net.to),
            width: net.width.max(FEASIBILITY_TOLERANCE_MM),
        });
    }
    for net in &problem.electrical_nets {
        if represented_connections.contains(&net.id) || net.terminals.len() < 2 {
            continue;
        }
        let root = pin_ref_point(&components, &poses, &net.terminals[0]);
        for terminal in &net.terminals[1..] {
            segments.push(PlacementDemandSegment {
                connection: net.id.clone(),
                start: root,
                finish: pin_ref_point(&components, &poses, terminal),
                width: net.width.max(FEASIBILITY_TOLERANCE_MM),
            });
        }
    }

    let cross_net_proper_crossings = segments
        .iter()
        .enumerate()
        .map(|(first_index, first)| {
            segments[first_index + 1..]
                .iter()
                .filter(|second| {
                    first.connection != second.connection
                        && segments_properly_cross(
                            first.start,
                            first.finish,
                            second.start,
                            second.finish,
                        )
                })
                .count()
        })
        .sum();

    let bounds = problem.board.bounds;
    let cell_width = bounds.width() / grid_size[0] as f64;
    let cell_height = bounds.height() / grid_size[1] as f64;
    if !cell_width.is_finite()
        || !cell_height.is_finite()
        || cell_width <= 0.0
        || cell_height <= 0.0
    {
        return Err("placement demand requires finite positive board bounds".into());
    }
    let mut field = vec![0.0_f64; cell_count];
    for segment in &segments {
        let dx_cells = (segment.finish.x - segment.start.x).abs() / cell_width;
        let dy_cells = (segment.finish.y - segment.start.y).abs() / cell_height;
        let steps = dx_cells.max(dy_cells).ceil().max(1.0) as usize;
        let length = distance(segment.start, segment.finish);
        let deposit = segment.width * length / steps as f64;
        for step in 0..steps {
            let t = (step as f64 + 0.5) / steps as f64;
            let point = add(segment.start, scale(sub(segment.finish, segment.start), t));
            let x = (((point.x - bounds.min.x) / cell_width).floor() as isize)
                .clamp(0, grid_size[0] as isize - 1) as usize;
            let y = (((point.y - bounds.min.y) / cell_height).floor() as isize)
                .clamp(0, grid_size[1] as isize - 1) as usize;
            field[y * grid_size[0] + x] += deposit;
        }
    }
    Ok(PlacementDemandEvidence {
        grid_size,
        segment_count: segments.len(),
        cross_net_proper_crossings,
        occupied_cells: field.iter().filter(|demand| **demand > 0.0).count(),
        peak_cell_demand: field.iter().copied().fold(0.0, f64::max),
        squared_cell_demand: field.iter().map(|demand| demand * demand).sum(),
        deposited_demand: field.iter().sum(),
    })
}

fn segments_properly_cross(first: Vec2, second: Vec2, third: Vec2, fourth: Vec2) -> bool {
    fn orientation(first: Vec2, second: Vec2, third: Vec2) -> f64 {
        (second.x - first.x) * (third.y - first.y) - (second.y - first.y) * (third.x - first.x)
    }
    let first_orientation = orientation(first, second, third);
    let second_orientation = orientation(first, second, fourth);
    let third_orientation = orientation(third, fourth, first);
    let fourth_orientation = orientation(third, fourth, second);
    first_orientation * second_orientation < -1.0e-9
        && third_orientation * fourth_orientation < -1.0e-9
}

fn movement_projection(movement: Movement, vector: Vec2) -> Vec2 {
    match movement {
        Movement::Fixed => Vec2::ZERO,
        Movement::Horizontal => Vec2::new(vector.x, 0.0),
        Movement::Vertical => Vec2::new(0.0, vector.y),
        Movement::Free => vector,
    }
}

pub(crate) fn edge_facing_error_degrees(
    rotation_degrees: f64,
    local_facing_direction_degrees: f64,
    edge: BoardEdge,
) -> f64 {
    angular_distance_degrees(
        rotation_degrees + local_facing_direction_degrees,
        edge.outward_angle_degrees(),
    )
}

pub(crate) fn edge_rotation_correction_degrees(
    rotation_degrees: f64,
    local_facing_direction_degrees: f64,
    edge: BoardEdge,
    tolerance_degrees: f64,
) -> f64 {
    let current = rotation_degrees + local_facing_direction_degrees;
    let desired = edge.outward_angle_degrees();
    let signed_delta = (desired - current + 180.0).rem_euclid(360.0) - 180.0;
    if signed_delta.abs() <= tolerance_degrees {
        0.0
    } else {
        signed_delta.signum() * (signed_delta.abs() - tolerance_degrees)
    }
}

pub(crate) fn component_edge_distance(
    edge: BoardEdge,
    bounds: Rect,
    position: Vec2,
    component: &layout_trace_model::model::Component,
    rotation_degrees: f64,
) -> f64 {
    let extent = geometry::body_extent(component, rotation_degrees);
    match edge {
        BoardEdge::West => position.x - extent.x - bounds.min.x,
        BoardEdge::East => bounds.max.x - position.x - extent.x,
        BoardEdge::South => position.y - extent.y - bounds.min.y,
        BoardEdge::North => bounds.max.y - position.y - extent.y,
    }
}

pub(crate) fn edge_inward_translation(edge: BoardEdge, distance: f64) -> Vec2 {
    match edge {
        BoardEdge::West => Vec2::new(-distance, 0.0),
        BoardEdge::East => Vec2::new(distance, 0.0),
        BoardEdge::South => Vec2::new(0.0, -distance),
        BoardEdge::North => Vec2::new(0.0, distance),
    }
}

fn rotated_extent(size: Vec2, degrees: f64) -> Vec2 {
    let radians = degrees.to_radians();
    let cosine = radians.cos().abs();
    let sine = radians.sin().abs();
    Vec2::new(
        (cosine * size.x + sine * size.y) * 0.5,
        (sine * size.x + cosine * size.y) * 0.5,
    )
}

fn rotated(vector: Vec2, degrees: f64) -> Vec2 {
    let radians = degrees.to_radians();
    Vec2::new(
        radians.cos() * vector.x - radians.sin() * vector.y,
        radians.sin() * vector.x + radians.cos() * vector.y,
    )
}

fn add(first: Vec2, second: Vec2) -> Vec2 {
    Vec2::new(first.x + second.x, first.y + second.y)
}

fn sub(first: Vec2, second: Vec2) -> Vec2 {
    Vec2::new(first.x - second.x, first.y - second.y)
}

fn scale(vector: Vec2, scalar: f64) -> Vec2 {
    Vec2::new(vector.x * scalar, vector.y * scalar)
}

fn squared_length(vector: Vec2) -> f64 {
    vector.x * vector.x + vector.y * vector.y
}

fn dot(first: Vec2, second: Vec2) -> f64 {
    first.x * second.x + first.y * second.y
}

fn vector_length(vector: Vec2) -> f64 {
    squared_length(vector).sqrt()
}

fn distance(first: Vec2, second: Vec2) -> f64 {
    vector_length(sub(first, second))
}

fn angle_distance(first: f64, second: f64) -> f64 {
    ((first - second + 180.0).rem_euclid(360.0) - 180.0).abs()
}

fn stable_string_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ byte as u64).wrapping_mul(0x1000_0000_01b3)
    })
}

fn mix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn unit_float(value: u64) -> f64 {
    (value >> 11) as f64 * (1.0 / ((1_u64 << 53) as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(
        components: serde_json::Value,
        nets: serde_json::Value,
        constraints: serde_json::Value,
    ) -> Problem {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "board": {"bounds": {"min": {"x": 0.0, "y": 0.0}, "max": {"x": 100.0, "y": 60.0}}},
            "rules": {"clearance": 0.2},
            "components": components,
            "nets": nets,
            "placement_constraints": constraints
        }))
        .unwrap()
    }

    fn component(id: &str, x: f64, y: f64, constraints: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "position": {"x": x, "y": y},
            "size": {"x": 4.0, "y": 2.0},
            "constraints": constraints,
            "pins": [{"id": "1", "offset": {"x": 0.0, "y": 0.0}}]
        })
    }

    #[test]
    fn legalization_observer_preserves_results_and_retains_failed_final_state() {
        let config: InitialPlacementConfig = serde_json::from_value(serde_json::json!({
            "policy": {"kind": "harmonic_ports", "legalization_sweeps": 2,
                       "connected_pair_spacing_floor": false}
        }))
        .unwrap();
        for movement in ["fixed", "free"] {
            let problem = problem(
                serde_json::json!([
                    component(
                        "A",
                        20.0,
                        20.0,
                        serde_json::json!({"movement": movement, "rotation": "fixed"})
                    ),
                    component(
                        "B",
                        21.0,
                        20.0,
                        serde_json::json!({"movement": movement, "rotation": "fixed"})
                    )
                ]),
                serde_json::json!([]),
                serde_json::json!([]),
            );
            let ordinary = run_initial_placement(&problem, &config);
            let frames = std::cell::RefCell::new(Vec::new());
            let traced = run_initial_placement_traced(&problem, &config, &|frame| {
                frames.borrow_mut().push(frame)
            });
            assert_eq!(
                serde_json::to_value(&ordinary).unwrap(),
                serde_json::to_value(&traced).unwrap()
            );
            let frames = frames.into_inner();
            assert_eq!(frames[0].sweep, 0);
            assert_eq!(frames[0].contacts.len(), 1);
            assert_eq!(frames[0].contacts[0].first, "A");
            assert!(frames[0].contacts[0].residual_mm > 0.0);
            let last = frames.last().unwrap();
            if movement == "fixed" {
                assert!(traced.is_err());
                assert_eq!(last.sweep, 2);
                assert_eq!(last.poses, frames[0].poses);
                assert_eq!(last.contacts.len(), 1);
            } else {
                assert!(traced.is_ok());
                assert!(last.contacts.is_empty());
                assert_ne!(last.poses, frames[0].poses);
            }
        }
    }

    #[test]
    fn generator_proposal_errors_consume_attempts_instead_of_aborting_the_budget() {
        struct SecondAttemptGenerator;

        impl InitialPlacementGenerator for SecondAttemptGenerator {
            fn name(&self) -> &'static str {
                "second_attempt"
            }

            fn maximum_attempts(&self) -> usize {
                2
            }

            fn propose(
                &self,
                problem: &Problem,
                _seed: u64,
                attempt: usize,
            ) -> Result<Vec<PlacementPose>, String> {
                if attempt == 0 {
                    Err("deliberate first proposal failure".into())
                } else {
                    Ok(declared_poses(problem))
                }
            }
        }

        let problem = problem(
            serde_json::json!([component(
                "FIXED",
                10.0,
                10.0,
                serde_json::json!({"movement": "fixed", "rotation": "fixed"})
            )]),
            serde_json::json!([]),
            serde_json::json!([]),
        );
        let result =
            run_initial_placement_with_generator(&problem, 0, 8, &SecondAttemptGenerator, false)
                .unwrap();
        assert_eq!(result.evidence.proposal_attempts, 2);
        assert_eq!(result.evidence.policy, "second_attempt");
    }

    #[test]
    fn placement_archive_retains_unique_finalists_and_exposes_rejections_and_duplicates() {
        struct ArchiveGenerator;

        impl InitialPlacementGenerator for ArchiveGenerator {
            fn name(&self) -> &'static str {
                "archive_control"
            }

            fn maximum_attempts(&self) -> usize {
                4
            }

            fn propose(
                &self,
                problem: &Problem,
                _seed: u64,
                attempt: usize,
            ) -> Result<Vec<PlacementPose>, String> {
                if attempt == 0 {
                    return Err("deliberate rejected proposal".into());
                }
                let mut poses = declared_poses(problem);
                if attempt == 3 {
                    poses[0].position.x = 20.0;
                }
                Ok(poses)
            }
        }

        let problem = problem(
            serde_json::json!([component("MOVABLE", 10.0, 10.0, serde_json::json!({}))]),
            serde_json::json!([]),
            serde_json::json!([]),
        );
        let archive = run_initial_placement_archive_with_generator(
            &problem,
            0,
            8,
            &ArchiveGenerator,
            false,
            2,
        )
        .unwrap();

        assert_eq!(archive.evaluated_proposals, 4);
        assert_eq!(archive.feasible_unique_proposals, 2);
        assert_eq!(archive.rejected_proposals, 1);
        assert_eq!(archive.duplicate_proposals, 1);
        assert_eq!(archive.retained_proposals, 2);
        assert_eq!(archive.retained_attempt_ordinals, [1, 3]);
        assert_eq!(
            archive.attempts[0].disposition,
            InitialPlacementArchiveDisposition::Rejected
        );
        assert_eq!(
            archive.attempts[2].disposition,
            InitialPlacementArchiveDisposition::Duplicate
        );
        assert_eq!(archive.attempts[2].duplicate_of_ordinal, Some(1));
        assert!(archive.attempts[1].placement.is_some());
        assert!(archive.attempts[3].placement.is_some());
    }

    #[test]
    fn placement_archive_is_opt_in_and_first_feasible_behavior_is_unchanged() {
        let problem = problem(
            serde_json::json!([component("MOVABLE", 10.0, 10.0, serde_json::json!({}))]),
            serde_json::json!([]),
            serde_json::json!([]),
        );
        let config = InitialPlacementConfig {
            policy: InitialPlacementPolicy::Random { attempts: 4 },
            seed: 19,
            orientation_refinement: None,
            projection_sweeps: 8,
        };
        let first = run_initial_placement(&problem, &config).unwrap();
        let archive = run_initial_placement_archive(&problem, &config, 4).unwrap();
        let first_attempt = archive
            .attempts
            .iter()
            .find(|attempt| attempt.ordinal + 1 == first.evidence.proposal_attempts)
            .and_then(|attempt| attempt.placement.as_ref())
            .unwrap();
        assert_eq!(first.poses, first_attempt.poses);
    }

    #[test]
    fn local_mutation_archive_keeps_base_and_independent_legal_variants() {
        let problem = problem(
            serde_json::json!([component("MOVABLE", 50.0, 30.0, serde_json::json!({}))]),
            serde_json::json!([]),
            serde_json::json!([]),
        );
        let config = InitialPlacementConfig {
            policy: InitialPlacementPolicy::LocalMutation {
                base: Box::new(InitialPlacementPolicy::Declared),
                attempts: 6,
                step_mm: 0.5,
                mutations_per_proposal: 1,
                allow_rotation: true,
            },
            seed: 7,
            orientation_refinement: None,
            projection_sweeps: 16,
        };
        let archive = run_initial_placement_archive(&problem, &config, 3).unwrap();
        assert_eq!(archive.maximum_proposals, 6);
        assert_eq!(archive.retained_proposals, 3);
        assert!(archive.feasible_unique_proposals >= 3);
        let base = archive
            .attempts
            .iter()
            .find(|attempt| attempt.ordinal == 0)
            .and_then(|attempt| attempt.placement.as_ref())
            .unwrap();
        assert_eq!(base.poses[0].position, Vec2::new(50.0, 30.0));
        assert!(archive.attempts.iter().all(|attempt| {
            attempt
                .placement
                .as_ref()
                .is_none_or(|placement| placement.evidence.feasible)
        }));
    }

    #[test]
    fn local_mutation_configuration_is_bounded_and_cannot_nest() {
        let valid = InitialPlacementConfig {
            policy: InitialPlacementPolicy::LocalMutation {
                base: Box::new(InitialPlacementPolicy::Declared),
                attempts: 4,
                step_mm: 0.5,
                mutations_per_proposal: 1,
                allow_rotation: false,
            },
            seed: 0,
            orientation_refinement: None,
            projection_sweeps: 8,
        };
        valid.check().unwrap();
        let mut invalid_step = valid.clone();
        let InitialPlacementPolicy::LocalMutation { step_mm, .. } = &mut invalid_step.policy else {
            unreachable!()
        };
        *step_mm = 0.0;
        assert!(invalid_step.check().is_err());
        let nested = InitialPlacementConfig {
            policy: InitialPlacementPolicy::LocalMutation {
                base: Box::new(valid.policy),
                attempts: 2,
                step_mm: 0.5,
                mutations_per_proposal: 1,
                allow_rotation: false,
            },
            seed: 0,
            orientation_refinement: None,
            projection_sweeps: 8,
        };
        assert!(nested.check().is_err());
    }

    #[test]
    fn serialized_pose_validation_includes_pad_extended_exclusion() {
        let extended = |id: &str, x: f64, pad_x: f64| {
            serde_json::json!({
                "id": id,
                "position": {"x": x, "y": 10.0},
                "size": {"x": 4.0, "y": 2.0},
                "constraints": {"movement": "free", "rotation": "fixed"},
                "pins": [{
                    "id": "1",
                    "offset": {"x": pad_x, "y": 0.0},
                    "pads": [{
                        "id": "pad",
                        "local_center": {"x": pad_x, "y": 0.0},
                        "layer": "top",
                        "shape": {"kind": "circle", "diameter": 1.0}
                    }]
                }]
            })
        };
        let problem = problem(
            serde_json::json!([extended("A", 10.0, 2.0), extended("B", 14.5, -2.0)]),
            serde_json::json!([]),
            serde_json::json!([]),
        );
        let error = validate_serialized_placement_poses(&problem, &declared_poses(&problem))
            .expect_err("pad-extended envelopes overlap despite separated nominal bodies");

        assert!(error.contains("component placement envelopes A and B violate exclusion"));
    }

    #[test]
    fn placement_demand_field_counts_cross_net_pressure() {
        let fixed = serde_json::json!({"movement": "fixed", "rotation": "fixed"});
        let problem = problem(
            serde_json::json!([
                component("A", 10.0, 10.0, fixed.clone()),
                component("B", 90.0, 50.0, fixed.clone()),
                component("C", 10.0, 50.0, fixed.clone()),
                component("D", 90.0, 10.0, fixed)
            ]),
            serde_json::json!([
                {"id": "AB", "width": 0.25, "from": {"component": "A", "pin": "1"}, "to": {"component": "B", "pin": "1"}},
                {"id": "CD", "width": 0.25, "from": {"component": "C", "pin": "1"}, "to": {"component": "D", "pin": "1"}}
            ]),
            serde_json::json!([]),
        );
        let poses = declared_poses(&problem);
        let evidence = placement_demand_evidence(&problem, &poses, [10, 6]).unwrap();

        assert_eq!(evidence.segment_count, 2);
        assert_eq!(evidence.cross_net_proper_crossings, 1);
        assert!(evidence.occupied_cells > 0);
        assert!(evidence.peak_cell_demand > 0.0);
        assert!(evidence.squared_cell_demand > 0.0);
        assert!(
            (evidence.deposited_demand - connectivity_distance(&problem, &poses).unwrap()).abs()
                < 1.0e-9
        );
    }

    #[test]
    fn random_projection_respects_fixed_axis_region_distance_and_rotation_constraints() {
        let fixed_header = component(
            "RPI_HEADER",
            15.0,
            10.0,
            serde_json::json!({"movement": "fixed", "rotation": "fixed"}),
        );
        let horizontal = component(
            "H",
            45.0,
            22.0,
            serde_json::json!({
                "movement": "horizontal", "rotation": "fixed",
                "region": {"min": {"x": 30.0, "y": 20.0}, "max": {"x": 70.0, "y": 24.0}}
            }),
        );
        let free = component(
            "F",
            55.0,
            22.0,
            serde_json::json!({
                "movement": "free", "region": {"min": {"x": 30.0, "y": 15.0}, "max": {"x": 75.0, "y": 35.0}}
            }),
        );
        let problem = problem(
            serde_json::json!([fixed_header, horizontal, free]),
            serde_json::json!([]),
            serde_json::json!([{
                "kind": "maximum_distance",
                "first": {"component": "H"},
                "second": {"component": "F"},
                "maximum": 12.0
            }]),
        );
        problem.check_schema().unwrap();
        let result = run_initial_placement(
            &problem,
            &InitialPlacementConfig {
                policy: InitialPlacementPolicy::Random { attempts: 8 },
                seed: 91,
                orientation_refinement: None,
                projection_sweeps: 256,
            },
        )
        .unwrap();
        let poses = result
            .poses
            .iter()
            .map(|pose| (pose.component.as_str(), pose))
            .collect::<HashMap<_, _>>();
        assert_eq!(poses["RPI_HEADER"].position, Vec2::new(15.0, 10.0));
        assert_eq!(poses["RPI_HEADER"].rotation_degrees, 0.0);
        assert_eq!(poses["H"].position.y, 22.0);
        assert_eq!(poses["H"].rotation_degrees, 0.0);
        assert!(
            distance(poses["H"].position, poses["F"].position) <= 12.0 + FEASIBILITY_TOLERANCE_MM
        );
        assert!(result.evidence.feasible);
        assert!(!result.evidence.routability_claimed);
    }

    #[test]
    fn infeasible_fixed_relationship_fails_closed() {
        let problem = problem(
            serde_json::json!([
                component("A", 10.0, 10.0, serde_json::json!({"movement": "fixed"})),
                component("B", 50.0, 10.0, serde_json::json!({"movement": "fixed"}))
            ]),
            serde_json::json!([]),
            serde_json::json!([{
                "kind": "maximum_distance",
                "first": {"component": "A"}, "second": {"component": "B"}, "maximum": 5.0
            }]),
        );
        assert!(run_initial_placement(&problem, &InitialPlacementConfig::default()).is_err());
    }

    #[test]
    fn grid_and_random_are_stable_under_input_permutation() {
        let components = vec![
            component("C", 70.0, 20.0, serde_json::json!({})),
            component("A", 20.0, 20.0, serde_json::json!({})),
            component("B", 40.0, 20.0, serde_json::json!({})),
        ];
        let forward = problem(
            serde_json::Value::Array(components.clone()),
            serde_json::json!([]),
            serde_json::json!([]),
        );
        let reverse = problem(
            serde_json::Value::Array(components.into_iter().rev().collect()),
            serde_json::json!([]),
            serde_json::json!([]),
        );
        for policy in [
            InitialPlacementPolicy::GridPacking { gap_mm: 1.5 },
            InitialPlacementPolicy::Random { attempts: 4 },
        ] {
            let config = InitialPlacementConfig {
                policy,
                seed: 31337,
                orientation_refinement: None,
                projection_sweeps: 32,
            };
            assert_eq!(
                run_initial_placement(&forward, &config).unwrap().poses,
                run_initial_placement(&reverse, &config).unwrap().poses
            );
        }
    }

    #[test]
    fn harmonic_region_and_locked_axis_feed_back_to_neighbors() {
        let edge = |id, first, second| {
            serde_json::json!({
                "id": id, "width": 0.25,
                "from": {"component": first, "pin": "1"},
                "to": {"component": second, "pin": "1"}
            })
        };
        let problem = problem(
            serde_json::json!([
                component("A", 10.0, 30.0, serde_json::json!({"movement": "fixed"})),
                component("B", 90.0, 30.0, serde_json::json!({"movement": "fixed"})),
                component(
                    "C",
                    50.0,
                    10.0,
                    serde_json::json!({
                        "region": {"min": {"x": 40.0, "y": 5.0}, "max": {"x": 60.0, "y": 15.0}}
                    })
                ),
                component("D", 50.0, 50.0, serde_json::json!({})),
                component(
                    "H",
                    60.0,
                    8.0,
                    serde_json::json!({"movement": "horizontal"})
                )
            ]),
            serde_json::json!([
                edge("AC", "A", "C"),
                edge("BC", "B", "C"),
                edge("CD", "C", "D"),
                edge("AH", "A", "H"),
                edge("BH", "B", "H")
            ]),
            serde_json::json!([]),
        );
        let components = sorted_components(&problem);
        for iterations in [0, 128] {
            let (poses, _, _) = harmonic_centers(&problem, &components, iterations).unwrap();
            let position = |id| {
                poses
                    .iter()
                    .find(|pose| pose.component == id)
                    .unwrap()
                    .position
            };
            assert_eq!(
                position("H").y,
                8.0,
                "even initialization must preserve the locked axis"
            );
            assert!(position("C").y <= 14.0);
            if iterations > 0 {
                assert!(
                    (position("D").y - 14.0).abs() < 1.0e-8,
                    "the free leaf must follow its constrained neighbor before legalization"
                );
            }
        }
    }

    #[test]
    fn harmonic_tension_weights_control_attraction_but_not_physical_demand() {
        for explicit in [false, true] {
            let mut problem = problem(
                serde_json::json!([
                    component("A", 10.0, 30.0, serde_json::json!({"movement": "fixed"})),
                    component("B", 90.0, 30.0, serde_json::json!({"movement": "fixed"})),
                    component("M", 60.0, 30.0, serde_json::json!({}))
                ]),
                serde_json::json!([]),
                serde_json::json!([]),
            );
            let mut nets = Vec::new();
            for (id, anchor, weight) in [("LOCAL", "A", 1.0), ("GND", "B", 0.05)] {
                let mut net =
                    serde_json::json!({"id": id, "width": 0.25, "tension_weight": weight});
                if explicit {
                    net["terminals"] = serde_json::json!([
                        {"component": anchor, "pin": "1"}, {"component": "M", "pin": "1"}
                    ]);
                } else {
                    net["from"] = serde_json::json!({"component": anchor, "pin": "1"});
                    net["to"] = serde_json::json!({"component": "M", "pin": "1"});
                }
                nets.push(net);
            }
            if explicit {
                problem.electrical_nets = serde_json::from_value(serde_json::json!(nets)).unwrap();
            } else {
                problem.nets = serde_json::from_value(serde_json::json!(nets)).unwrap();
            }
            problem.check_schema().unwrap();
            let components = sorted_components(&problem);
            let indexes = components
                .iter()
                .enumerate()
                .map(|(i, c)| (c.id.as_str(), i))
                .collect();
            let (poses, _, _) = harmonic_centers(&problem, &components, 512).unwrap();
            let moving = poses.iter().find(|p| p.component == "M").unwrap();
            assert!((moving.position.x - (10.0 + 90.0 * 0.05) / 1.05).abs() < 1.0e-8);
            let ports = port_attractions(&problem, &indexes, &poses);
            let to_ground = ports
                .iter()
                .find(|p| p.component == indexes["M"] && p.target.x == 90.0)
                .unwrap();
            assert!((to_ground.weight - 0.0125).abs() < 1.0e-12);

            let demand = placement_demand_evidence(&problem, &poses, [10, 6]).unwrap();
            // A zero shortening weight removes attraction, not electrical or
            // geometric demand. It must not manufacture a second anchor either.
            let mut zero_problem = problem.clone();
            if explicit {
                zero_problem.electrical_nets[1].tension_weight = 0.0;
            } else {
                zero_problem.nets[1].tension_weight = 0.0;
            }
            let components = sorted_components(&zero_problem);
            let (zero_poses, retained, active) =
                harmonic_centers(&zero_problem, &components, 512).unwrap();
            assert_eq!(retained, 1);
            assert!(!active[indexes["M"]]);
            assert_eq!(zero_poses[indexes["M"]].position.x, 60.0);
            assert_eq!(port_attractions(&zero_problem, &indexes, &poses).len(), 2);
            let zero_demand = placement_demand_evidence(&zero_problem, &poses, [10, 6]).unwrap();
            assert_eq!(demand.deposited_demand, zero_demand.deposited_demand);
            assert_eq!(demand.segment_count, zero_demand.segment_count);
        }
    }

    #[test]
    fn harmonic_policy_is_stable_under_component_net_and_terminal_permutation() {
        let components = vec![
            component("WEST", 8.0, 30.0, serde_json::json!({"movement": "fixed"})),
            component("M1", 65.0, 45.0, serde_json::json!({})),
            component("M2", 35.0, 15.0, serde_json::json!({})),
            component("EAST", 92.0, 30.0, serde_json::json!({"movement": "fixed"})),
        ];
        let nets = vec![
            serde_json::json!({
                "id": "N1", "width": 0.25,
                "from": {"component": "WEST", "pin": "1"},
                "to": {"component": "M1", "pin": "1"}
            }),
            serde_json::json!({
                "id": "N2", "width": 0.4,
                "from": {"component": "M2", "pin": "1"},
                "to": {"component": "EAST", "pin": "1"}
            }),
        ];
        let mut forward = problem(
            serde_json::Value::Array(components.clone()),
            serde_json::Value::Array(nets.clone()),
            serde_json::json!([]),
        );
        forward.electrical_nets = serde_json::from_value(serde_json::json!([{
            "id": "BUS", "width": 0.3,
            "terminals": [
                {"component": "M1", "pin": "1"},
                {"component": "M2", "pin": "1"},
                {"component": "EAST", "pin": "1"}
            ]
        }]))
        .unwrap();
        let mut reverse = problem(
            serde_json::Value::Array(components.into_iter().rev().collect()),
            serde_json::Value::Array(nets.into_iter().rev().collect()),
            serde_json::json!([]),
        );
        reverse.electrical_nets = serde_json::from_value(serde_json::json!([{
            "id": "BUS", "width": 0.3,
            "terminals": [
                {"component": "EAST", "pin": "1"},
                {"component": "M2", "pin": "1"},
                {"component": "M1", "pin": "1"}
            ]
        }]))
        .unwrap();
        forward.check_schema().unwrap();
        reverse.check_schema().unwrap();
        let config = InitialPlacementConfig {
            policy: InitialPlacementPolicy::HarmonicPorts {
                iterations: 64,
                legalization_sweeps: 64,
                maximum_pair_checks: 100_000,
                coupled_legalization: None,
                connected_pair_spacing_floor: true,
                underanchored_seed: UnderanchoredSeedPolicy::Declared,
                fixed_obstacle_seed_projection: false,
                largest_first_seed_insertion: false,
                skip_collision_intervals: false,
                seed_extra_pad_gap_mm: 0.0,
            },
            seed: 7,
            orientation_refinement: None,
            projection_sweeps: 64,
        };
        assert_eq!(
            run_initial_placement(&forward, &config).unwrap().poses,
            run_initial_placement(&reverse, &config).unwrap().poses
        );
    }

    #[test]
    fn harmonic_port_choice_rotates_one_rigid_component_toward_its_neighbors() {
        let fixed = |id: &str, x: f64| {
            serde_json::json!({
                "id": id, "position": {"x": x, "y": 30.0},
                "size": {"x": 2.0, "y": 2.0},
                "constraints": {"movement": "fixed", "rotation": "fixed"},
                "pins": [{"id": "1", "offset": {"x": 0.0, "y": 0.0}}]
            })
        };
        let movable = serde_json::json!({
            "id": "U", "position": {"x": 50.0, "y": 30.0},
            "size": {"x": 8.0, "y": 4.0},
            "constraints": {"movement": "free", "rotation": "free"},
            "pins": [
                {"id": "LEFT", "offset": {"x": 3.0, "y": 0.0}, "pads": [
                    {"id": "left-pad", "local_center": {"x": 3.0, "y": 0.0}, "layer": "top", "shape": {"kind": "circle", "diameter": 0.5}}
                ]},
                {"id": "RIGHT", "offset": {"x": 0.0, "y": 0.0}}
            ]
        });
        let problem = problem(
            serde_json::json!([fixed("A", 10.0), movable, fixed("B", 90.0)]),
            serde_json::json!([
                {"id": "LEFT_NET", "width": 1.0, "from": {"component": "A", "pin": "1"}, "to": {"component": "U", "pin": "LEFT"}},
                {"id": "RIGHT_NET", "width": 1.0, "from": {"component": "U", "pin": "RIGHT"}, "to": {"component": "B", "pin": "1"}}
            ]),
            serde_json::json!([]),
        );
        let result = run_initial_placement(
            &problem,
            &InitialPlacementConfig {
                policy: InitialPlacementPolicy::HarmonicPorts {
                    iterations: 32,
                    legalization_sweeps: 64,
                    maximum_pair_checks: 100_000,
                    coupled_legalization: None,
                    connected_pair_spacing_floor: true,
                    underanchored_seed: UnderanchoredSeedPolicy::Declared,
                    fixed_obstacle_seed_projection: false,
                    largest_first_seed_insertion: false,
                    skip_collision_intervals: false,
                    seed_extra_pad_gap_mm: 0.0,
                },
                seed: 0,
                orientation_refinement: None,
                projection_sweeps: 32,
            },
        )
        .unwrap();
        let pose = result
            .poses
            .iter()
            .find(|pose| pose.component == "U")
            .unwrap();
        assert_eq!(pose.rotation_degrees, 180.0);
        assert!(result.evidence.embedding_guidance_used);
        assert!(!result.evidence.routability_claimed);
    }

    #[test]
    fn harmonic_legalizer_separates_a_collapsed_passive_bundle() {
        let fixed = |id: &str, x: f64| {
            component(
                id,
                x,
                30.0,
                serde_json::json!({"movement": "fixed", "rotation": "fixed"}),
            )
        };
        let movable = |id: &str| component(id, 50.0, 30.0, serde_json::json!({}));
        let problem = problem(
            serde_json::json!([
                fixed("WEST", 10.0),
                movable("P1"),
                movable("P2"),
                movable("P3"),
                fixed("EAST", 90.0)
            ]),
            serde_json::json!([
                {"id": "N1", "width": 0.25, "from": {"component": "WEST", "pin": "1"}, "to": {"component": "P1", "pin": "1"}},
                {"id": "N2", "width": 0.25, "from": {"component": "WEST", "pin": "1"}, "to": {"component": "P2", "pin": "1"}},
                {"id": "N3", "width": 0.25, "from": {"component": "WEST", "pin": "1"}, "to": {"component": "P3", "pin": "1"}},
                {"id": "N4", "width": 0.25, "from": {"component": "P1", "pin": "1"}, "to": {"component": "EAST", "pin": "1"}},
                {"id": "N5", "width": 0.25, "from": {"component": "P2", "pin": "1"}, "to": {"component": "EAST", "pin": "1"}},
                {"id": "N6", "width": 0.25, "from": {"component": "P3", "pin": "1"}, "to": {"component": "EAST", "pin": "1"}}
            ]),
            serde_json::json!([]),
        );
        let result = run_initial_placement(
            &problem,
            &InitialPlacementConfig {
                policy: InitialPlacementPolicy::HarmonicPorts {
                    iterations: 64,
                    legalization_sweeps: 128,
                    maximum_pair_checks: 100_000,
                    coupled_legalization: None,
                    connected_pair_spacing_floor: true,
                    underanchored_seed: UnderanchoredSeedPolicy::Declared,
                    fixed_obstacle_seed_projection: false,
                    largest_first_seed_insertion: false,
                    skip_collision_intervals: false,
                    seed_extra_pad_gap_mm: 0.0,
                },
                seed: 0,
                orientation_refinement: None,
                projection_sweeps: 32,
            },
        )
        .unwrap();
        validate_placement_exclusions(&problem, &result.poses).unwrap();
        assert_eq!(result.evidence.underanchored_blocks_retained, 0);
    }

    #[test]
    fn harmonic_policy_reports_and_retains_underanchored_blocks() {
        let problem = problem(
            serde_json::json!([
                component(
                    "ANCHOR",
                    10.0,
                    30.0,
                    serde_json::json!({"movement": "fixed", "rotation": "fixed"})
                ),
                component("PASSIVE", 70.0, 30.0, serde_json::json!({}))
            ]),
            serde_json::json!([{
                "id": "N", "width": 0.25,
                "from": {"component": "ANCHOR", "pin": "1"},
                "to": {"component": "PASSIVE", "pin": "1"}
            }]),
            serde_json::json!([]),
        );
        let result = run_initial_placement(
            &problem,
            &InitialPlacementConfig {
                policy: InitialPlacementPolicy::HarmonicPorts {
                    iterations: 64,
                    legalization_sweeps: 16,
                    maximum_pair_checks: 1_000,
                    coupled_legalization: None,
                    connected_pair_spacing_floor: true,
                    underanchored_seed: UnderanchoredSeedPolicy::Declared,
                    fixed_obstacle_seed_projection: false,
                    largest_first_seed_insertion: false,
                    skip_collision_intervals: false,
                    seed_extra_pad_gap_mm: 0.0,
                },
                seed: 0,
                orientation_refinement: None,
                projection_sweeps: 32,
            },
        )
        .unwrap();
        let passive = result
            .poses
            .iter()
            .find(|pose| pose.component == "PASSIVE")
            .unwrap();
        assert_eq!(passive.position, Vec2::new(70.0, 30.0));
        assert_eq!(passive.rotation_degrees, 0.0);
        assert_eq!(result.evidence.underanchored_blocks_retained, 1);
        assert_eq!(result.evidence.underanchored_blocks_reseeded, 0);
    }

    #[test]
    fn harmonic_policy_can_grid_reseed_underanchored_blocks() {
        let problem = problem(
            serde_json::json!([
                component(
                    "ANCHOR",
                    10.0,
                    30.0,
                    serde_json::json!({"movement": "fixed", "rotation": "fixed"})
                ),
                component("PASSIVE", 70.0, 30.0, serde_json::json!({}))
            ]),
            serde_json::json!([{
                "id": "N", "width": 0.25,
                "from": {"component": "ANCHOR", "pin": "1"},
                "to": {"component": "PASSIVE", "pin": "1"}
            }]),
            serde_json::json!([]),
        );
        let result = run_initial_placement(
            &problem,
            &InitialPlacementConfig {
                policy: InitialPlacementPolicy::HarmonicPorts {
                    iterations: 64,
                    legalization_sweeps: 16,
                    maximum_pair_checks: 1_000,
                    coupled_legalization: None,
                    connected_pair_spacing_floor: true,
                    underanchored_seed: UnderanchoredSeedPolicy::GridPacking { gap_mm: 1.0 },
                    fixed_obstacle_seed_projection: false,
                    largest_first_seed_insertion: false,
                    skip_collision_intervals: false,
                    seed_extra_pad_gap_mm: 0.0,
                },
                seed: 0,
                orientation_refinement: None,
                projection_sweeps: 32,
            },
        )
        .unwrap();
        let passive = result
            .poses
            .iter()
            .find(|pose| pose.component == "PASSIVE")
            .unwrap();
        assert_ne!(passive.position, Vec2::new(70.0, 30.0));
        assert_eq!(result.evidence.policy, "harmonic_ports_underanchored_grid");
        assert_eq!(result.evidence.underanchored_blocks_retained, 0);
        assert_eq!(result.evidence.underanchored_blocks_reseeded, 1);
        validate_placement_exclusions(&problem, &result.poses).unwrap();
    }

    #[test]
    fn old_harmonic_json_defaults_to_declared_underanchored_seed() {
        let config: InitialPlacementConfig = serde_json::from_value(serde_json::json!({
            "policy": {
                "kind": "harmonic_ports",
                "iterations": 8,
                "legalization_sweeps": 8,
                "maximum_pair_checks": 100,
                "connected_pair_spacing_floor": true
            },
            "seed": 0,
            "projection_sweeps": 8
        }))
        .unwrap();
        assert!(matches!(
            config.policy,
            InitialPlacementPolicy::HarmonicPorts {
                underanchored_seed: UnderanchoredSeedPolicy::Declared,
                ..
            }
        ));
    }

    #[test]
    fn underanchored_random_seed_requires_a_positive_attempt_budget() {
        let config = InitialPlacementConfig {
            policy: InitialPlacementPolicy::HarmonicPorts {
                iterations: 8,
                legalization_sweeps: 8,
                maximum_pair_checks: 100,
                coupled_legalization: None,
                connected_pair_spacing_floor: true,
                underanchored_seed: UnderanchoredSeedPolicy::Random { attempts: 0 },
                fixed_obstacle_seed_projection: false,
                largest_first_seed_insertion: false,
                skip_collision_intervals: false,
                seed_extra_pad_gap_mm: 0.0,
            },
            seed: 0,
            orientation_refinement: None,
            projection_sweeps: 8,
        };
        assert!(config.check().unwrap_err().contains("attempts"));
    }

    #[test]
    fn harmonic_policy_fails_closed_when_fixed_bodies_cannot_be_separated() {
        let problem = problem(
            serde_json::json!([
                component(
                    "A",
                    50.0,
                    30.0,
                    serde_json::json!({"movement": "fixed", "rotation": "fixed"})
                ),
                component(
                    "B",
                    50.0,
                    30.0,
                    serde_json::json!({"movement": "fixed", "rotation": "fixed"})
                )
            ]),
            serde_json::json!([{
                "id": "N", "width": 0.25,
                "from": {"component": "A", "pin": "1"},
                "to": {"component": "B", "pin": "1"}
            }]),
            serde_json::json!([]),
        );
        let error = run_initial_placement(
            &problem,
            &InitialPlacementConfig {
                policy: InitialPlacementPolicy::HarmonicPorts {
                    iterations: 8,
                    legalization_sweeps: 2,
                    maximum_pair_checks: 16,
                    coupled_legalization: None,
                    connected_pair_spacing_floor: true,
                    underanchored_seed: UnderanchoredSeedPolicy::Declared,
                    fixed_obstacle_seed_projection: false,
                    largest_first_seed_insertion: false,
                    skip_collision_intervals: false,
                    seed_extra_pad_gap_mm: 0.0,
                },
                seed: 0,
                orientation_refinement: None,
                projection_sweeps: 8,
            },
        )
        .unwrap_err();
        assert!(error.contains("did not converge") || error.contains("overlap"));
    }

    #[test]
    fn underanchored_esp_slices_skip_harmonic_motion_then_legalize() {
        for input in [
            include_str!(
                "../../../benchmarks/imported/layout-trace/esp32-slices/power-west-capacitor-interaction.json"
            ),
            include_str!(
                "../../../benchmarks/imported/layout-trace/esp32-slices/west-six-final-geometry.json"
            ),
        ] {
            let problem: Problem = serde_json::from_str(input).unwrap();
            let declared = declared_poses(&problem);
            let components = sorted_components(&problem);
            let (coarse, underanchored, active) =
                harmonic_centers(&problem, &components, 128).unwrap();
            assert!(underanchored > 0);
            assert!(active.iter().any(|active| !active));
            for ((coarse, declared), active) in coarse.iter().zip(&declared).zip(&active) {
                if !active {
                    assert_eq!(coarse, declared);
                }
            }
            let result = run_initial_placement(
                &problem,
                &InitialPlacementConfig {
                    policy: InitialPlacementPolicy::HarmonicPorts {
                        iterations: 128,
                        legalization_sweeps: 128,
                        maximum_pair_checks: 1_000_000,
                        coupled_legalization: None,
                        connected_pair_spacing_floor: true,
                        underanchored_seed: UnderanchoredSeedPolicy::Declared,
                        fixed_obstacle_seed_projection: false,
                        largest_first_seed_insertion: false,
                        skip_collision_intervals: false,
                        seed_extra_pad_gap_mm: 0.0,
                    },
                    seed: 0,
                    orientation_refinement: None,
                    projection_sweeps: 256,
                },
            )
            .unwrap();
            // Under-anchored components retain the harmonic seed above. The
            // common hard-constraint projector may still move them slightly;
            // retaining a seed is not permission to skip relational rules.
            assert!(result.evidence.underanchored_blocks_retained > 0);
            validate_placement_exclusions(&problem, &result.poses).unwrap();
        }
    }

    fn planted_grid_problem(side: usize) -> Problem {
        let mut components = Vec::new();
        for row in 0..side {
            for column in 0..side {
                let boundary = row == 0 || column == 0 || row + 1 == side || column + 1 == side;
                let (declared_row, declared_column) = if boundary {
                    (row, column)
                } else {
                    (side - 1 - row, side - 1 - column)
                };
                components.push(serde_json::json!({
                    "id": format!("C{row:02}_{column:02}"),
                    "position": {
                        "x": 5.0 + declared_column as f64 * 10.0,
                        "y": 5.0 + declared_row as f64 * 5.5
                    },
                    "size": {"x": 3.0, "y": 2.0},
                    "constraints": if boundary {
                        serde_json::json!({"movement": "fixed", "rotation": "fixed"})
                    } else {
                        serde_json::json!({"movement": "free", "rotation": "fixed"})
                    },
                    "pins": [{"id": "1", "offset": {"x": 0.0, "y": 0.0}}]
                }));
            }
        }
        let mut nets = Vec::new();
        for row in 0..side {
            for column in 0..side {
                for (next_row, next_column) in [(row + 1, column), (row, column + 1)] {
                    if next_row >= side || next_column >= side {
                        continue;
                    }
                    nets.push(serde_json::json!({
                        "id": format!("N{row:02}_{column:02}_{next_row:02}_{next_column:02}"),
                        "width": 0.25,
                        "from": {"component": format!("C{row:02}_{column:02}"), "pin": "1"},
                        "to": {"component": format!("C{next_row:02}_{next_column:02}"), "pin": "1"}
                    }));
                }
            }
        }
        problem(
            serde_json::Value::Array(components),
            serde_json::Value::Array(nets),
            serde_json::json!([]),
        )
    }

    fn orientation(first: Vec2, second: Vec2, third: Vec2) -> f64 {
        (second.x - first.x) * (third.y - first.y) - (second.y - first.y) * (third.x - first.x)
    }

    fn proper_segment_crossing(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
        let first = orientation(a, b, c);
        let second = orientation(a, b, d);
        let third = orientation(c, d, a);
        let fourth = orientation(c, d, b);
        first * second < -1.0e-9 && third * fourth < -1.0e-9
    }

    fn straight_crossings_and_overlaps(
        problem: &Problem,
        result: &InitialPlacementResult,
    ) -> usize {
        let poses = result
            .poses
            .iter()
            .map(|pose| (pose.component.as_str(), pose.position))
            .collect::<HashMap<_, _>>();
        let mut violations = 0;
        for first in 0..problem.nets.len() {
            for second in first + 1..problem.nets.len() {
                let a = &problem.nets[first];
                let b = &problem.nets[second];
                if [a.from.component.as_str(), a.to.component.as_str()]
                    .iter()
                    .any(|component| *component == b.from.component || *component == b.to.component)
                {
                    continue;
                }
                if proper_segment_crossing(
                    poses[a.from.component.as_str()],
                    poses[a.to.component.as_str()],
                    poses[b.from.component.as_str()],
                    poses[b.to.component.as_str()],
                ) {
                    violations += 1;
                }
            }
        }
        for first in 0..problem.components.len() {
            for second in first + 1..problem.components.len() {
                let a = &problem.components[first];
                let b = &problem.components[second];
                let delta = sub(poses[a.id.as_str()], poses[b.id.as_str()]);
                if delta.x.abs() < (a.size.x + b.size.x) * 0.5
                    && delta.y.abs() < (a.size.y + b.size.y) * 0.5
                {
                    violations += 1;
                }
            }
        }
        violations
    }

    #[test]
    fn harmonic_seed_beats_random_and_connectivity_on_planted_boundary_grid() {
        let problem = planted_grid_problem(10);
        let run = |policy| {
            run_initial_placement(
                &problem,
                &InitialPlacementConfig {
                    policy,
                    seed: 0x51,
                    orientation_refinement: None,
                    projection_sweeps: 64,
                },
            )
        };
        let harmonic = run(InitialPlacementPolicy::HarmonicPorts {
            iterations: 512,
            legalization_sweeps: 64,
            maximum_pair_checks: 1_000_000,
            coupled_legalization: None,
            connected_pair_spacing_floor: true,
            underanchored_seed: UnderanchoredSeedPolicy::Declared,
            fixed_obstacle_seed_projection: false,
            largest_first_seed_insertion: false,
            skip_collision_intervals: false,
            seed_extra_pad_gap_mm: 0.0,
        })
        .unwrap();
        let connectivity = run(InitialPlacementPolicy::ConnectivityBarycentric {
            iterations: default_barycentric_iterations(),
            attraction: default_barycentric_attraction(),
        });
        let random = run(InitialPlacementPolicy::Random { attempts: 1 });
        let harmonic_violations = straight_crossings_and_overlaps(&problem, &harmonic);
        // A control which cannot produce a legal placement is strictly worse
        // than a legal harmonic seed for this comparison.
        let connectivity_violations = connectivity.as_ref().map_or(usize::MAX, |result| {
            straight_crossings_and_overlaps(&problem, result)
        });
        let random_violations = random.as_ref().map_or(usize::MAX, |result| {
            straight_crossings_and_overlaps(&problem, result)
        });
        assert_eq!(harmonic_violations, 0);
        assert!(
            harmonic_violations < connectivity_violations,
            "harmonic={harmonic_violations}, connectivity={connectivity_violations}"
        );
        assert!(
            harmonic_violations < random_violations,
            "harmonic={harmonic_violations}, random={random_violations}"
        );
    }

    #[test]
    fn connectivity_policy_improves_distance_without_claiming_routability() {
        let problem = problem(
            serde_json::json!([
                component("A", 10.0, 30.0, serde_json::json!({"movement": "fixed"})),
                component("B", 85.0, 30.0, serde_json::json!({}))
            ]),
            serde_json::json!([{
                "id": "N", "width": 0.25, "from": {"component": "A", "pin": "1"}, "to": {"component": "B", "pin": "1"}
            }]),
            serde_json::json!([]),
        );
        let result = run_initial_placement(
            &problem,
            &InitialPlacementConfig {
                policy: InitialPlacementPolicy::ConnectivityBarycentric {
                    iterations: 8,
                    attraction: 0.5,
                },
                seed: 0,
                orientation_refinement: None,
                projection_sweeps: 32,
            },
        )
        .unwrap();
        assert!(
            result.evidence.connectivity_distance_after_mm
                < result.evidence.connectivity_distance_before_mm
        );
        assert!(!result.evidence.routability_claimed);
    }

    #[test]
    fn free_component_is_rotated_and_moved_to_selected_board_edge() {
        let problem = problem(
            serde_json::json!([component(
                "ANTENNA",
                50.0,
                30.0,
                serde_json::json!({"movement": "free", "rotation": "free"})
            )]),
            serde_json::json!([]),
            serde_json::json!([{
                "kind": "face_board_edge",
                "component": "ANTENNA",
                "edge": "west",
                "angular_tolerance_degrees": 0.0,
                "maximum_distance": 1.0
            }]),
        );
        let result = run_initial_placement(&problem, &InitialPlacementConfig::default()).unwrap();
        let pose = &result.poses[0];
        assert!(edge_facing_error_degrees(pose.rotation_degrees, 0.0, BoardEdge::West) < 1.0e-9);
        assert!(
            component_edge_distance(
                BoardEdge::West,
                problem.board.bounds,
                pose.position,
                &problem.components[0],
                pose.rotation_degrees,
            ) <= 1.0 + FEASIBILITY_TOLERANCE_MM
        );
    }

    #[test]
    fn fixed_rotation_that_cannot_face_edge_is_rejected() {
        let problem = problem(
            serde_json::json!([component(
                "FIXED",
                50.0,
                30.0,
                serde_json::json!({"movement": "fixed", "rotation": "fixed"})
            )]),
            serde_json::json!([]),
            serde_json::json!([{
                "kind": "face_board_edge",
                "component": "FIXED",
                "edge": "west",
                "angular_tolerance_degrees": 5.0
            }]),
        );
        assert!(
            problem
                .check_schema()
                .unwrap_err()
                .contains("fixed-rotation component FIXED")
        );
    }

    #[test]
    fn dual_esp32_antennas_declare_opposite_edge_facing() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/dual-esp32-benchmark.json"
        ))
        .unwrap();
        problem.check_schema().unwrap();
        for (id, edge) in [("ESP_W", BoardEdge::West), ("ESP_E", BoardEdge::East)] {
            let component = problem
                .components
                .iter()
                .find(|component| component.id == id)
                .unwrap();
            assert!(
                problem
                    .placement_constraints
                    .iter()
                    .any(|constraint| matches!(
                        constraint,
                        PlacementConstraint::FaceBoardEdge { component, edge: declared, .. }
                            if component == id && *declared == edge
                    ))
            );
            assert!(edge_facing_error_degrees(component.rotation_degrees, 270.0, edge) < 1.0e-9);
            assert!(
                component_edge_distance(
                    edge,
                    problem.board.bounds,
                    component.position,
                    component,
                    component.rotation_degrees,
                ) <= FEASIBILITY_TOLERANCE_MM
            );
        }
    }

    #[test]
    fn transient_seed_pad_gap_is_explicit_bounded_and_omitted_at_zero() {
        let mut config: InitialPlacementConfig = serde_json::from_value(serde_json::json!({
            "policy":{"kind":"harmonic_ports","fixed_obstacle_seed_projection":true,
                      "largest_first_seed_insertion":true}
        }))
        .unwrap();
        let before = serde_json::to_value(&config).unwrap();
        assert!(before["policy"].get("seed_extra_pad_gap_mm").is_none());
        for value in [f64::NAN, f64::INFINITY, -0.1, 0.0, 0.6] {
            if let InitialPlacementPolicy::HarmonicPorts {
                seed_extra_pad_gap_mm,
                ..
            } = &mut config.policy
            {
                *seed_extra_pad_gap_mm = value;
            }
            assert_eq!(config.check().is_ok(), value.is_finite() && value >= 0.0);
        }
        if let InitialPlacementPolicy::HarmonicPorts {
            largest_first_seed_insertion,
            ..
        } = &mut config.policy
        {
            *largest_first_seed_insertion = false;
        }
        assert!(config.check().is_err());
    }
}

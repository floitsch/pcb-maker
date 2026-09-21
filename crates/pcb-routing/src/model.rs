use layout_trace_model::Vec2;
use pcb_validate::{CandidateArtifact, ExactValidationAssessment};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteOrder {
    Input,
    #[default]
    WidthDescending,
    LongestFirst,
}

/// How much geometric uncertainty the node raster reserves around obstacles.
/// Both modes still pass the resulting continuous segments through exact DRC.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RasterSafety {
    /// Match the predecessor's centerline grid: reserve physical trace radius
    /// and clearance, but do not charge a whole cell for discretization.
    #[default]
    ExactCenterline,
    /// Every segment incident to a legal grid point is conservatively clear.
    /// This is safer during search but can erase narrow valid channels.
    ConservativeCells,
}

/// How generated branches for a multi-terminal electrical net acquire their
/// source. Explicit two-terminal `nets` are never rewritten by this policy.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiTerminalRoutingPolicy {
    /// Compile a Euclidean terminal MST and route each terminal-to-terminal
    /// edge independently.
    #[default]
    TerminalMst,
    /// After the first edge, grow an unconnected terminal from the closest
    /// legal vertex on already-routed same-net copper.
    SharedCopperTree,
}

/// Objective used only when a shared-tree growth step evaluates more than one
/// attachment source. Keeping it explicit lets experiments compare physical
/// copper quality with the router's internal search objective.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeAttachmentPortfolioObjective {
    /// Minimize retained physical branch length, then vias and bends.
    #[default]
    RetainedLength,
    /// Match the predecessor terminal-junction ordering: vias, then length,
    /// then bends.
    ViaCountThenLength,
    /// Minimize the selected A* cost before retained physical tie-breakers.
    RouterCost,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DutGridRoutingConfig {
    pub grid_mm: f64,
    /// Optional progressively finer grids tried after an exact `no_path`.
    /// Budget exhaustion does not trigger refinement because it is not proof
    /// that the current discretization has no path.
    #[serde(default)]
    pub retry_grid_mm: Vec<f64>,
    pub max_expansions_per_search: u32,
    pub maximum_exact_edge_retries: usize,
    pub straight_cost: u32,
    pub diagonal_cost: u32,
    pub bend_cost: u32,
    pub via_cost: u32,
    pub allow_vias: bool,
    #[serde(default)]
    pub route_order: RouteOrder,
    /// Stable branch IDs forced to the front before the selected base order.
    /// This is the semantic hook for bounded reorder/rip-up experiments.
    #[serde(default)]
    pub priority_branches: Vec<String>,
    #[serde(default)]
    pub raster_safety: RasterSafety,
    #[serde(default)]
    pub multi_terminal_routing: MultiTerminalRoutingPolicy,
    /// Enables predecessor-style reroutes from already-connected terminals.
    /// `None` disables the experiment; `Some(slack)` admits a terminal branch
    /// up to `slack` millimetres longer than ordinary tree attachment.
    #[serde(default)]
    pub terminal_junction_slack_mm: Option<f64>,
    #[serde(default = "default_maximum_terminal_junction_searches")]
    pub maximum_terminal_junction_searches: usize,
    /// Number of distance-ranked existing-tree/target pairs evaluated for
    /// each shared-tree growth step. One preserves the predecessor's greedy
    /// behavior; larger values are an explicit route-portfolio experiment.
    #[serde(default = "default_maximum_tree_attachment_searches")]
    pub maximum_tree_attachment_searches: usize,
    /// Optional geometric diversity radius for the distance-ranked tree
    /// attachment portfolio. Later sources for the same target and layer
    /// must be at least this far from already retained sources.
    #[serde(default)]
    pub tree_attachment_minimum_spacing_mm: f64,
    #[serde(default)]
    pub tree_attachment_portfolio_objective: TreeAttachmentPortfolioObjective,
}

impl DutGridRoutingConfig {
    pub fn check(&self) -> Result<(), String> {
        if !self.grid_mm.is_finite() || self.grid_mm <= 0.0 {
            return Err("routing grid_mm must be finite and positive".into());
        }
        let mut previous = self.grid_mm;
        for &grid in &self.retry_grid_mm {
            if !grid.is_finite() || grid <= 0.0 {
                return Err("routing retry_grid_mm values must be finite and positive".into());
            }
            if grid >= previous {
                return Err("routing retry_grid_mm values must be strictly decreasing".into());
            }
            previous = grid;
        }
        if self.max_expansions_per_search == 0 {
            return Err("routing max_expansions_per_search must be positive".into());
        }
        if self.straight_cost == 0 || self.diagonal_cost == 0 {
            return Err("routing movement costs must be positive".into());
        }
        let mut priorities = std::collections::BTreeSet::new();
        for branch in &self.priority_branches {
            if branch.is_empty() || !priorities.insert(branch) {
                return Err("priority_branches must contain unique non-empty IDs".into());
            }
        }
        if self
            .terminal_junction_slack_mm
            .is_some_and(|slack| !slack.is_finite() || slack < 0.0)
        {
            return Err("terminal_junction_slack_mm must be finite and non-negative".into());
        }
        if self.maximum_terminal_junction_searches == 0 {
            return Err("maximum_terminal_junction_searches must be positive".into());
        }
        if self.maximum_tree_attachment_searches == 0 {
            return Err("maximum_tree_attachment_searches must be positive".into());
        }
        if !self.tree_attachment_minimum_spacing_mm.is_finite()
            || self.tree_attachment_minimum_spacing_mm < 0.0
        {
            return Err(
                "tree_attachment_minimum_spacing_mm must be finite and non-negative".into(),
            );
        }
        if self.terminal_junction_slack_mm.is_some()
            && self.multi_terminal_routing != MultiTerminalRoutingPolicy::SharedCopperTree
        {
            return Err("terminal-junction preference requires shared_copper_tree routing".into());
        }
        if (self.maximum_tree_attachment_searches > 1
            || self.tree_attachment_minimum_spacing_mm > 0.0)
            && self.multi_terminal_routing != MultiTerminalRoutingPolicy::SharedCopperTree
        {
            return Err("tree-attachment portfolios require shared_copper_tree routing".into());
        }
        Ok(())
    }
}

impl Default for DutGridRoutingConfig {
    fn default() -> Self {
        Self {
            grid_mm: 0.5,
            retry_grid_mm: Vec::new(),
            max_expansions_per_search: 1_000_000,
            maximum_exact_edge_retries: 64,
            straight_cost: 1_000,
            diagonal_cost: 1_414,
            bend_cost: 100,
            via_cost: 8_000,
            allow_vias: true,
            route_order: RouteOrder::WidthDescending,
            priority_branches: Vec::new(),
            raster_safety: RasterSafety::ExactCenterline,
            multi_terminal_routing: MultiTerminalRoutingPolicy::TerminalMst,
            terminal_junction_slack_mm: None,
            maximum_terminal_junction_searches: 32,
            maximum_tree_attachment_searches: 1,
            tree_attachment_minimum_spacing_mm: 0.0,
            tree_attachment_portfolio_objective: TreeAttachmentPortfolioObjective::RetainedLength,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchRoutingStatus {
    Found,
    NoPath,
    BudgetExhausted,
    ExactGeometryRejected,
    TreeUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingBlockerKind {
    BoardEdge,
    ComponentBody,
    Keepout,
    Pad,
    RoutedTrace,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RoutingBlockerEvidence {
    pub kind: RoutingBlockerKind,
    /// Stable semantic object identity: component ID, branch ID, or `board`.
    pub object: String,
    pub frontier_hits: usize,
    /// Mean physical location of blocked grid cells attributed to this object.
    /// Coordinators can derive directional pressure without depending on the
    /// router's grid representation.
    pub frontier_centroid: Vec2,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GridRoutingAttemptEvidence {
    pub grid_mm: f64,
    pub status: BranchRoutingStatus,
    pub terminal_layer_searches: usize,
    pub expansions: u64,
    pub exact_edge_retries: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TreeAttachmentEvidence {
    pub target_terminal: String,
    pub searched_source_branch: String,
    pub searched_point_index: usize,
    pub searched_position: Vec2,
    pub searched_layer: String,
    pub source_branch: String,
    pub point_index: usize,
    pub node: String,
    pub position: Vec2,
    pub layer: String,
    pub interior: bool,
    pub existing_segment_split: bool,
    pub new_segment_split: bool,
    pub trimmed_prefix_points: usize,
    pub terminal_preference_selected: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TerminalJunctionAttemptEvidence {
    pub source_terminal: String,
    pub status: BranchRoutingStatus,
    pub searches: usize,
    pub expansions: u64,
    pub retained_length_mm: Option<f64>,
    pub via_count: Option<usize>,
    pub bend_count: Option<usize>,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TreeAttachmentAttemptEvidence {
    pub target_terminal: String,
    pub source_branch: String,
    pub point_index: usize,
    pub position: Vec2,
    pub layer: String,
    pub status: BranchRoutingStatus,
    pub searches: usize,
    pub expansions: u64,
    pub committed_position: Option<Vec2>,
    pub trimmed_prefix_points: usize,
    pub retained_length_mm: Option<f64>,
    pub via_count: Option<usize>,
    pub bend_count: Option<usize>,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BranchRoutingEvidence {
    pub branch: String,
    pub electrical_net: String,
    pub width: f64,
    pub status: BranchRoutingStatus,
    pub terminal_layer_searches: usize,
    pub expansions: u64,
    pub exact_edge_retries: usize,
    pub selected_cost: Option<u32>,
    pub selected_grid_mm: Option<f64>,
    pub grid_attempts: Vec<GridRoutingAttemptEvidence>,
    pub point_count: usize,
    pub via_count: usize,
    pub blockers: Vec<RoutingBlockerEvidence>,
    pub tree_attachment: Option<TreeAttachmentEvidence>,
    pub tree_attachment_attempts: Vec<TreeAttachmentAttemptEvidence>,
    pub tree_attachment_searches_truncated: bool,
    pub terminal_junction_attempts: Vec<TerminalJunctionAttemptEvidence>,
    pub terminal_junction_searches_truncated: bool,
}

fn default_maximum_terminal_junction_searches() -> usize {
    32
}

fn default_maximum_tree_attachment_searches() -> usize {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DutGridRoutingEvidence {
    pub strategy: String,
    pub config: DutGridRoutingConfig,
    pub branch_count: usize,
    pub routed_branches: usize,
    pub failed_branches: usize,
    pub searches: usize,
    pub expansions: u64,
    pub branches: Vec<BranchRoutingEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DutGridRoutingResult {
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    pub evidence: DutGridRoutingEvidence,
}

impl DutGridRoutingResult {
    pub fn complete(&self) -> bool {
        self.validation.complete && self.evidence.failed_branches == 0
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NegotiatedDutGridRoutingConfig {
    pub routing: DutGridRoutingConfig,
    pub maximum_passes: usize,
    pub congestion_cost: u32,
    pub history_cost: u32,
}

impl NegotiatedDutGridRoutingConfig {
    pub fn check(&self) -> Result<(), String> {
        self.routing.check()?;
        if self.maximum_passes == 0 {
            return Err("negotiated routing maximum_passes must be positive".into());
        }
        if self.congestion_cost == 0 || self.history_cost == 0 {
            return Err("negotiated routing congestion/history costs must be positive".into());
        }
        if !self.routing.retry_grid_mm.is_empty() {
            return Err(
                "negotiated routing currently requires one grid; adaptive history remapping is not yet defined"
                    .into(),
            );
        }
        Ok(())
    }
}

impl Default for NegotiatedDutGridRoutingConfig {
    fn default() -> Self {
        Self {
            routing: DutGridRoutingConfig::default(),
            maximum_passes: 8,
            congestion_cost: 8_000,
            history_cost: 1_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NegotiatedPassEvidence {
    pub pass: usize,
    pub routed_branches: usize,
    pub failed_branches: usize,
    pub congested_cells: usize,
    pub exact_violations: usize,
    pub expansions: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NegotiatedDutGridRoutingEvidence {
    pub strategy: String,
    pub config: NegotiatedDutGridRoutingConfig,
    pub selected_pass: usize,
    pub completed_passes: usize,
    pub total_expansions: u64,
    pub passes: Vec<NegotiatedPassEvidence>,
    pub routing: DutGridRoutingEvidence,
}

#[derive(Clone, Debug, Serialize)]
pub struct NegotiatedDutGridRoutingResult {
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    pub evidence: NegotiatedDutGridRoutingEvidence,
    pub attempts: Vec<DutGridRoutingResult>,
}

impl NegotiatedDutGridRoutingResult {
    pub fn complete(&self) -> bool {
        self.validation.complete
            && self.evidence.routing.failed_branches == 0
            && self.evidence.passes[self.evidence.selected_pass].congested_cells == 0
    }
}

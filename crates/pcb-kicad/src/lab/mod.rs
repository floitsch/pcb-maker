// Copyright (C) 2026 Toit contributors.

//! Measurement tools: what the research and benchmark commands run, kept
//! apart from the product (place, route, lay out, check, export). Nothing
//! here runs in `layout-kicad-board`, `route-kicad-board` or the other
//! agent commands; the scripts under `benchmarks/` and `experiments/` call
//! these through the binary's research commands.
//!
//! - `placement_ablation`: `ablate-kicad-placement`, the designer's and our
//!   placement mixed group by group (benchmarks/agent-tasks/ablation.py).
//! - `trouble_probe`, `coarse_probe`: `probe-trouble-placement` and
//!   `probe-coarse-placement`, placements judged by partial or coarse
//!   routes (experiments/trouble-probe, experiments/coarse-probe).
//! - `legality`: `legality-kicad-board`, a placement judged by the
//!   placer's legality model at every relaxation level beside KiCad's DRC
//!   (benchmarks/agent-tasks/legality_census.py).
//! - `global_route`: `global-route-kicad-board`, the coarse global route as
//!   a routability measure beside RUDY and the race's probe
//!   (experiments/global-route), and the placed board's lowering the
//!   others share.
//! - `shrink`: `shrink-kicad-board`, boards shrunk for the corpus's
//!   strength rows (benchmarks/corpus/run.py).
//! - `footprint_links`: `normalize-kicad-footprint-links`, a benchmark
//!   board's schematic links rewritten (benchmarks/real/*/prepare.sh).
//!
//! In a module of the crate rather than a crate of their own: they lower
//! boards with the crate's private lowering and geometry (the placer's
//! `lower_placement` and `prepare_board`, the router's `lower` and pour
//! planning, the S-expression tree, the footprint pose writers, the
//! courtyard and body shapes, the constraint resolution: some thirty
//! crate-private items), which a separate crate would need made public.

mod coarse_probe;
mod footprint_links;
mod global_route;
mod legality;
mod placement_ablation;
mod shrink;
mod trouble_probe;

pub use coarse_probe::{KiCadCoarseProbeConfig, KiCadCoarseProbeReport, probe_coarse_placement};
pub use footprint_links::{
    KiCadFootprintLinkNormalizationReport, KiCadFootprintLinkUpdate,
    KiCadSchematicLinkNormalization, normalize_kicad_footprint_links,
};
pub use global_route::{KiCadGlobalRouteReport, KiCadProbeReport, global_route_kicad_board};
pub use legality::{KiCadLegalityReport, legality_kicad_board};
pub use placement_ablation::{
    KiCadAblationConfig, KiCadAblationGroup, KiCadAblationReport, KiCadAblationVariant,
    ablate_kicad_placement,
};
pub use shrink::{KiCadShrinkReport, shrink_kicad_board};
pub use trouble_probe::{
    KiCadTroubleBackground, KiCadTroubleNet, KiCadTroubleProbeConfig, KiCadTroubleProbeReport,
    KiCadTroubleSource, probe_trouble_placement,
};

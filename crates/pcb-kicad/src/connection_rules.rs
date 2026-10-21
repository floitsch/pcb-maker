// Copyright (C) 2026 Toit contributors.

use super::*;

/// Resolved per-connection geometry. Keys in the owning map use the native
/// adapter's connection IDs (one leading hierarchy slash removed). This is
/// compiled input, not a net-class pattern matcher or a custom-rule engine.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadConnectionRoutingRules {
    pub trace_width_mm: f64,
    pub clearance_mm: f64,
    pub via_size_mm: f64,
    pub via_drill_mm: f64,
}

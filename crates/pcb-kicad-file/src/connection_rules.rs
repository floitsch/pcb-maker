// Copyright (C) 2026 Toit contributors.

use super::*;

/// Resolved per-connection geometry. Keys in the owning map use the native
/// adapter's connection IDs (one leading hierarchy slash removed). This is
/// compiled input, not a net-class pattern matcher or a custom-rule engine.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadConnectionRoutingRules {
    /// Track width of the connection's net (raised to the width floor;
    /// the narrow-signal width replaces it on the attempts that use it).
    pub trace_width_mm: f64,
    /// Copper clearance the net's tracks and vias keep from other nets.
    pub clearance_mm: f64,
    /// Diameter of the net's vias.
    pub via_size_mm: f64,
    /// Drill of the net's vias.
    pub via_drill_mm: f64,
}

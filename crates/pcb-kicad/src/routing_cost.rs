// Copyright (C) 2026 Toit contributors.

use super::KiCadGridRouteConfig;

impl KiCadGridRouteConfig {
    /// Called after configuration validation. Round upward so conversion to
    /// integer search costs never discounts the requested physical penalty.
    pub(super) fn via_cost_units(&self) -> u32 {
        self.via_cost_mm.map_or(self.via_cost, |mm| {
            (mm * f64::from(self.straight_cost) / self.resolution_mm).ceil() as u32
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate_grid_route_config;

    #[test]
    fn physical_via_cost_preserves_track_equivalence_across_grids() {
        for (resolution_mm, expected) in [(0.25, 800), (0.05, 4000), (0.02, 10000)] {
            let config = KiCadGridRouteConfig {
                resolution_mm,
                via_cost_mm: Some(2.0),
                ..Default::default()
            };
            validate_grid_route_config(&config).unwrap();
            assert_eq!(config.via_cost_units(), expected);
            let legacy = KiCadGridRouteConfig {
                via_cost_mm: None,
                ..config
            };
            assert_eq!(legacy.via_cost_units(), 800);
        }
        let config = KiCadGridRouteConfig {
            resolution_mm: 0.3,
            straight_cost: 7,
            via_cost_mm: Some(0.1),
            ..Default::default()
        };
        assert_eq!(config.via_cost_units(), 3);
    }

    #[test]
    fn invalid_physical_costs_are_rejected_before_integer_conversion() {
        for mm in [-1.0, f64::NAN, f64::INFINITY, f64::MAX] {
            let config = KiCadGridRouteConfig {
                via_cost_mm: Some(mm),
                ..Default::default()
            };
            assert!(validate_grid_route_config(&config).is_err());
        }
        let config = KiCadGridRouteConfig {
            via_cost_mm: Some(0.0),
            ..Default::default()
        };
        validate_grid_route_config(&config).unwrap();
        assert_eq!(config.via_cost_units(), 0);
    }
}

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

pub(super) fn validate(config: &KiCadGridRouteConfig) -> Result<(), String> {
    for (connection, rules) in &config.connection_rules {
        let values = [
            rules.trace_width_mm,
            rules.clearance_mm,
            rules.via_size_mm,
            rules.via_drill_mm,
        ];
        if connection.is_empty()
            || values.iter().any(|x| !x.is_finite() || *x < 0.0)
            || rules.trace_width_mm == 0.0
            || rules.via_drill_mm == 0.0
            || rules.via_drill_mm >= rules.via_size_mm
        {
            return Err(format!("invalid resolved routing rules for {connection:?}"));
        }
    }
    Ok(())
}

pub(super) fn resolve(
    config: &KiCadGridRouteConfig,
    connection: &str,
) -> Result<KiCadGridRouteConfig, String> {
    let mut resolved = config.clone();
    if config.connection_rules.is_empty() {
        return Ok(resolved);
    }
    let rules = config
        .connection_rules
        .get(connection)
        .ok_or_else(|| format!("resolved routing rules omit target connection {connection:?}"))?;
    resolved.trace_width_mm = rules.trace_width_mm;
    resolved.clearance_mm = rules.clearance_mm;
    resolved.via_size_mm = rules.via_size_mm;
    resolved.via_drill_mm = rules.via_drill_mm;
    Ok(resolved)
}

pub(super) fn apply(
    model: &mut KiCadRoutingModel,
    config: &KiCadGridRouteConfig,
) -> Result<(), String> {
    if config.connection_rules.is_empty() {
        return Ok(());
    }
    validate(config)?;
    for obstacle in &mut model.obstacles {
        let Some(net) = obstacle.net.as_deref().filter(|net| !net.is_empty()) else {
            continue;
        };
        let connection = normalize_net(net);
        let rules = config.connection_rules.get(connection).ok_or_else(|| {
            format!("resolved routing rules omit obstacle connection {connection:?}")
        })?;
        // Query radii and broad-phase boxes already account for a local
        // clearance larger than the target's. Preserve pad/footprint floors.
        obstacle.local_clearance_mm = obstacle.local_clearance_mm.max(rules.clearance_mm);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rules(width: f64, clearance: f64) -> KiCadConnectionRoutingRules {
        KiCadConnectionRoutingRules {
            trace_width_mm: width,
            clearance_mm: clearance,
            via_size_mm: 1.6,
            via_drill_mm: 0.6,
        }
    }

    #[test]
    fn resolved_geometry_is_per_target_and_complete() {
        let mut config = KiCadGridRouteConfig::default();
        config
            .connection_rules
            .insert("GND".into(), rules(0.8, 0.28));
        config
            .connection_rules
            .insert("SIGNAL".into(), rules(0.5, 0.25));
        let ground = resolve(&config, "GND").unwrap();
        let signal = resolve(&ground, "SIGNAL").unwrap();
        assert_eq!((ground.trace_width_mm, ground.clearance_mm), (0.8, 0.28));
        assert_eq!((signal.trace_width_mm, signal.clearance_mm), (0.5, 0.25));
        assert_eq!(signal.via_size_mm, 1.6);
        assert!(resolve(&config, "MISSING").is_err());
        config.connection_rules.get_mut("GND").unwrap().clearance_mm = f64::NAN;
        assert!(validate(&config).is_err());
    }

    #[test]
    fn foreign_class_clearance_reaches_exact_and_broad_phase_checks() {
        let pcb = parse(r#"(kicad_pcb
          (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
          (footprint "A" (at 2 2) (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/SIGNAL")))
          (footprint "B" (at 18 2) (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/SIGNAL")))
          (segment (start 10 5) (end 10 15) (width 0.8) (layer "F.Cu") (net "GND")))"#).unwrap();
        let mut model = KiCadRoutingModel::from_pcb(&pcb, "SIGNAL").unwrap();
        let mut config = KiCadGridRouteConfig {
            trace_width_mm: 0.5,
            clearance_mm: 0.25,
            ..Default::default()
        };
        assert!(route_segment_is_clear(
            [10.91, 6.0],
            [10.91, 14.0],
            0,
            &model,
            &config
        ));
        config
            .connection_rules
            .insert("SIGNAL".into(), rules(0.5, 0.25));
        assert!(apply(&mut model, &config).is_err());
        config
            .connection_rules
            .insert("GND".into(), rules(0.8, 0.28));
        apply(&mut model, &config).unwrap();
        assert!(!route_segment_is_clear(
            [10.91, 6.0],
            [10.91, 14.0],
            0,
            &model,
            &config
        ));
        assert!(route_segment_is_clear(
            [10.94, 6.0],
            [10.94, 14.0],
            0,
            &model,
            &config
        ));
        let grid = KiCadObstacleGrid::new(&model, 0.5, config.clearance_mm);
        assert!(grid.any_intersects_segment(&model, 0, [10.91, 6.0], [10.91, 14.0], 0.5));
        assert!(!grid.any_intersects_segment(&model, 0, [10.94, 6.0], [10.94, 14.0], 0.5));
    }
}

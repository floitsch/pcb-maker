// Copyright (C) 2026 Florian Loitsch.

//! A controlled via-cost comparison for established multilayer routes.

use super::*;
use crate::{KiCadBoardRouterConfig, KiCadBoardRouterResult, board_router::LayerTable};

const TRIAL_COST: f64 = 0.5;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadViaCostRetry {
    pub copper_layers: usize,
    pub native_open_nets: usize,
    pub routed_vias: usize,
    pub current_router_config: String,
    pub current_via_cost: f64,
    pub router_config_patch: serde_json::Value,
}

impl KiCadVerdict {
    /// Attach advice only where the ordinary search used vias and KiCad
    /// still found many unresolved nets across a multilayer placement.
    pub fn note_via_cost_retry(
        &mut self,
        facts: &KiCadRunFacts,
        result: &KiCadBoardRouterResult,
        board: &Path,
        config: &KiCadBoardRouterConfig,
        config_name: Option<&str>,
    ) {
        if self.status != KiCadRunStatus::Incomplete
            || self.kicad_unconnected_items.is_none_or(|opens| opens < 12)
            || !self.constraints_missed.is_empty()
            || facts.boards_mixed().is_some()
            || facts.panel_copies.is_some()
            || config.tuning.via_cost.is_some()
            || result.routed_connections < 50
            || result.vias < 8
        {
            return;
        }
        let native_open_nets = facts
            .native
            .as_ref()
            .map_or(0, |native| native.unconnected_nets.len());
        if native_open_nets < 8 {
            return;
        }
        let Some(copper_layers) = fs::read_to_string(board)
            .ok()
            .and_then(|text| crate::parse(&text).ok())
            .and_then(|pcb| LayerTable::from_pcb(&pcb).ok())
            .map(|table| table.len())
            .filter(|layers| *layers >= 4)
        else {
            return;
        };
        let current_router_config = config_name.unwrap_or("auto").to_owned();
        let current_via_cost = pcb_router::router::Config::default().via_cost;
        let mut router_config_patch = serde_json::json!({"tuning":{"via_cost":TRIAL_COST}});
        if current_router_config == "auto"
            && let Some(seeds) = config.seeds
        {
            // Auto mode can get its seed count from PCB_ROUTER_SEEDS.
            // An explicit router JSON needs to carry the same value.
            router_config_patch["seeds"] = serde_json::json!(seeds);
        }
        self.via_cost_retry = Some(KiCadViaCostRetry {
            copper_layers,
            native_open_nets,
            routed_vias: result.vias,
            current_router_config,
            current_via_cost,
            router_config_patch,
        });
        self.next_actions.push(format!(
            "KiCad found opens on {native_open_nets} nets across a {copper_layers}-layer board. For a matched retry, set `tuning.via_cost` to {TRIAL_COST} in a copy of the current router JSON, keeping its other keys, placement and project rules. If the run used `auto`, supply `via_cost_retry.router_config_patch` as the router JSON so project rules are still loaded. Cheaper layer changes may add vias. Compare KiCad on the saved board and after refill, and check via count, track length and findings."
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_a_config_patch_only_for_a_verified_multilayer_run_using_default_cost() {
        let path = crate::fs_util::temporary_next_to(
            &std::env::temp_dir().join("via-retry-board.kicad_pcb"),
        );
        let four = r#"(kicad_pcb (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal)))"#;
        fs::write(&path, four).unwrap();
        let mut facts = KiCadRunFacts::new("route-kicad-board");
        facts.routable_connections = 70;
        facts.routed_connections = 55;
        facts.native = Some(DrcTally {
            unconnected: 14,
            unconnected_nets: (0..8).map(|index| (format!("S{index}"), 1)).collect(),
            ..Default::default()
        });
        let result = KiCadBoardRouterResult {
            routed_connections: 55,
            vias: 11,
            ..Default::default()
        };
        let mut config = KiCadBoardRouterConfig::default();
        let mut verdict = judge(&facts);
        verdict.note_via_cost_retry(&facts, &result, &path, &config, Some("configured.json"));
        let retry = verdict.via_cost_retry.unwrap();
        assert_eq!((retry.copper_layers, retry.native_open_nets), (4, 8));
        assert_eq!(retry.current_router_config, "configured.json");
        assert_eq!(
            retry.router_config_patch,
            serde_json::json!({"tuning":{"via_cost":0.5}})
        );

        config.seeds = Some(3);
        let mut verdict = judge(&facts);
        verdict.note_via_cost_retry(&facts, &result, &path, &config, Some("auto"));
        let retry = verdict.via_cost_retry.unwrap();
        assert_eq!(
            retry.router_config_patch,
            serde_json::json!({"seeds":3,"tuning":{"via_cost":0.5}})
        );

        config.tuning.via_cost = Some(4.0);
        let mut verdict = judge(&facts);
        verdict.note_via_cost_retry(&facts, &result, &path, &config, None);
        assert!(verdict.via_cost_retry.is_none());
        config.tuning.via_cost = None;
        fs::write(
            &path,
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal)))"#,
        )
        .unwrap();
        verdict.note_via_cost_retry(&facts, &result, &path, &config, None);
        assert!(verdict.via_cost_retry.is_none());
        fs::write(&path, four).unwrap();
        facts.native = None;
        let mut verdict = judge(&facts);
        verdict.note_via_cost_retry(&facts, &result, &path, &config, None);
        assert!(verdict.via_cost_retry.is_none());
        fs::remove_file(path).unwrap();
    }
}

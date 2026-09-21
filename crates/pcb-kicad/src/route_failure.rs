// Copyright (C) 2026 Toit contributors.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadRouteSearchFailureKind {
    SearchBudgetExhausted,
    GridDisconnected,
}

/// Search-only terminal context; reached branches have not received native admission.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct KiCadRouteTerminalContext {
    pub root_mm: [f64; 2],
    pub reached_mm: Vec<[f64; 2]>,
    pub pending_mm: [f64; 2],
}

/// Search evidence emitted by the router itself, before formatting a CLI
/// error. Grid disconnection describes the selected discrete search model,
/// not physical-board infeasibility.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct KiCadRouteSearchFailure {
    pub kind: KiCadRouteSearchFailureKind,
    pub connection: String,
    pub branch: usize,
    pub astar_expansions: u32,
    pub reachability_expansions: u32,
    pub heuristic_expansions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_context: Option<KiCadRouteTerminalContext>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum KiCadRouteError {
    Search(KiCadRouteSearchFailure),
    Other(String),
}

impl KiCadRouteError {
    pub fn search_failure(&self) -> Option<&KiCadRouteSearchFailure> {
        match self {
            Self::Search(failure) => Some(failure),
            Self::Other(_) => None,
        }
    }
}

impl From<String> for KiCadRouteError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

impl From<&str> for KiCadRouteError {
    fn from(message: &str) -> Self {
        Self::Other(message.into())
    }
}

impl fmt::Display for KiCadRouteError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Other(message) => output.write_str(message),
            Self::Search(failure) => {
                let reason = match failure.kind {
                    KiCadRouteSearchFailureKind::SearchBudgetExhausted => {
                        "exhausted its search budget"
                    }
                    KiCadRouteSearchFailureKind::GridDisconnected => {
                        "found no path on the routing grid"
                    }
                };
                write!(
                    output,
                    "router {reason} for {:?} branch {} after {} aggregate expansions; reachability preflight used {} expansions",
                    failure.connection,
                    failure.branch,
                    failure.astar_expansions,
                    failure.reachability_expansions
                )?;
                if let Some(expansions) = failure.heuristic_expansions {
                    write!(
                        output,
                        "; obstacle-distance preparation used {expansions} expansions"
                    )?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for KiCadRouteError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_branch_identifies_the_remaining_pad_after_partial_search_progress() {
        use crate::{
            KiCadGridHeuristic, KiCadGridRouteConfig, KiCadMultiTerminalRoutingPolicy,
            KiCadTerminalContactPolicy, route_materialized_connection_detailed,
        };
        use std::{
            env, fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let source = r#"(kicad_pcb
          (gr_rect (start 0 0) (end 10 8) (layer "Edge.Cuts"))
          (footprint "A" (at 1 2) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.4) (layers "*.Cu" "*.Mask") (net "/N")))
          (footprint "B" (at 3 2) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.4) (layers "*.Cu" "*.Mask") (net "/N")))
          (footprint "C" (at 8 2) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.4) (layers "*.Cu" "*.Mask") (net "/N")))
          (footprint "WALL" (at 5 4) (pad "1" thru_hole rect (at 0 0) (size 1 10) (drill 0.4) (layers "*.Cu" "*.Mask") (net "/BLOCK"))))"#;
        let path = env::temp_dir().join(format!(
            "pcb-failure-context-{}.kicad_pcb",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, source).unwrap();
        for heuristic in [
            KiCadGridHeuristic::Geometric,
            KiCadGridHeuristic::ObstacleDistances,
        ] {
            let config = KiCadGridRouteConfig {
                resolution_mm: 0.5,
                trace_width_mm: 0.2,
                clearance_mm: 0.1,
                edge_clearance_mm: 0.1,
                heuristic,
                reachability_preflight: true,
                multi_terminal_routing: KiCadMultiTerminalRoutingPolicy::SharedCopperTree,
                terminal_contact_policy: KiCadTerminalContactPolicy::FirstPadContact,
                ..Default::default()
            };
            let error = route_materialized_connection_detailed(&path, "N", &config).unwrap_err();
            let failure = error.search_failure().unwrap();
            assert_eq!(failure.kind, KiCadRouteSearchFailureKind::GridDisconnected);
            assert_eq!(failure.branch, 1);
            let context = failure.terminal_context.as_ref().unwrap();
            assert_eq!(context.root_mm, [3.0, 2.0]);
            assert_eq!(context.reached_mm, vec![[3.0, 2.0], [1.0, 2.0]]);
            assert_eq!(context.pending_mm, [8.0, 2.0]);
            let mut legacy = serde_json::to_value(failure).unwrap();
            legacy.as_object_mut().unwrap().remove("terminal_context");
            let restored: KiCadRouteSearchFailure = serde_json::from_value(legacy).unwrap();
            assert!(restored.terminal_context.is_none());
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn failure_evidence_preserves_work_and_legacy_cli_message() {
        let error = KiCadRouteError::Search(KiCadRouteSearchFailure {
            kind: KiCadRouteSearchFailureKind::SearchBudgetExhausted,
            connection: "NET".into(),
            branch: 2,
            astar_expansions: 101,
            reachability_expansions: 300,
            heuristic_expansions: Some(400),
            terminal_context: None,
        });
        assert_eq!(
            error.to_string(),
            "router exhausted its search budget for \"NET\" branch 2 after 101 aggregate expansions; reachability preflight used 300 expansions; obstacle-distance preparation used 400 expansions"
        );
        assert_eq!(
            error.search_failure().unwrap().heuristic_expansions,
            Some(400)
        );
        // Message content must never promote an unrelated error into a
        // machine-actionable search failure.
        assert!(
            KiCadRouteError::from(error.to_string())
                .search_failure()
                .is_none()
        );
    }
}

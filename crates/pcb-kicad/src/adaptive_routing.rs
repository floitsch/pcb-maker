// Copyright (C) 2026 Toit contributors.

//! Bounded native routing with automatically generated demand and learned order.
use super::*;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadAdaptiveInitialOrder {
    #[default]
    Discovery,
    ElectricalTerminalCountDescending,
}

impl KiCadAdaptiveInitialOrder {
    fn is_discovery(&self) -> bool {
        *self == Self::Discovery
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadAdaptiveRoutingConfig {
    pub sequential: KiCadSequentialRouterConfig,
    /// Resolve once from source discovery; later passes retain their learned order.
    #[serde(skip_serializing_if = "KiCadAdaptiveInitialOrder::is_discovery")]
    pub initial_order: KiCadAdaptiveInitialOrder,
    /// Generate cold forecasts before the first pass and reserve future routing
    /// space without changing that pass's configured order or attachment policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_demand_strength: Option<f64>,
    pub maximum_passes: usize,
    pub demand_strengths: Vec<f64>,
    pub demand_shoulder_mm: f64,
    pub forecast_via_costs_mm: Vec<Option<f64>>,
    pub maximum_forecasts_per_net: usize,
    /// Try one exact-parent, failure-first cold order before independent forecasts.
    #[serde(skip_serializing_if = "disabled")]
    pub retry_observed_failures_before_forecasts: bool,
    /// Continue trying policy/order alternatives after native connectivity is
    /// complete. Ordinary layout generation stops routing at that point;
    /// further copper/via optimization is an explicit separate objective.
    pub optimize_after_routing_complete: bool,
    /// Permit only unchanged source annotation findings during provisional
    /// routing. Final complete still requires the full native gate.
    pub allow_existing_annotation_findings: bool,
}

impl Default for KiCadAdaptiveRoutingConfig {
    fn default() -> Self {
        Self {
            sequential: Default::default(),
            initial_order: Default::default(),
            initial_demand_strength: None,
            maximum_passes: 6,
            demand_strengths: vec![1.0],
            demand_shoulder_mm: 1.0,
            forecast_via_costs_mm: vec![None, Some(20.0)],
            maximum_forecasts_per_net: 2,
            retry_observed_failures_before_forecasts: false,
            optimize_after_routing_complete: false,
            allow_existing_annotation_findings: false,
        }
    }
}

fn disabled(value: &bool) -> bool {
    !value
}

impl KiCadAdaptiveRoutingConfig {
    pub fn check(&self) -> Result<(), String> {
        sequential_router::validate_sequential_config(&self.sequential)?;
        if !self.initial_order.is_discovery() && !self.sequential.connection_order.is_empty() {
            return Err("automatic adaptive initial_order conflicts with explicit sequential connection_order".into());
        }
        if !(1..=64).contains(&self.maximum_passes)
            || !(1..=64).contains(&self.maximum_forecasts_per_net)
            || self.demand_strengths.len() > 8
            || self.forecast_via_costs_mm.is_empty()
            || self.forecast_via_costs_mm.len() > 8
            || !self.demand_shoulder_mm.is_finite()
            || self.demand_shoulder_mm <= 0.0
            || self
                .initial_demand_strength
                .is_some_and(|v| !v.is_finite() || v <= 0.0 || v > 1e6)
            || self
                .demand_strengths
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v > 1e6)
            || self
                .forecast_via_costs_mm
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || *v < 0.0)
            || self
                .sequential
                .routing_portfolio
                .iter()
                .any(|c| c.routing_demand.is_some())
        {
            return Err("invalid adaptive routing bounds; forecasts are generated internally, so supplied routing_demand must be absent".into());
        }
        Ok(())
    }
}

fn initial_connection_order(
    discovered: &[KiCadConnectionSummary],
    config: &KiCadAdaptiveRoutingConfig,
) -> Vec<String> {
    if !config.sequential.connection_order.is_empty() {
        return config.sequential.connection_order.clone();
    }
    let mut ordered = discovered.iter().collect::<Vec<_>>();
    if config.initial_order == KiCadAdaptiveInitialOrder::ElectricalTerminalCountDescending {
        // Stable sorting preserves source discovery order for equal electrical counts.
        ordered.sort_by_key(|net| std::cmp::Reverse(net.electrical_terminal_count()));
    }
    ordered
        .into_iter()
        .map(|net| net.connection.clone())
        .collect()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadRoutingForecastEvidence {
    pub connection: String,
    pub attempts: Vec<serde_json::Value>,
    pub distinct_alternatives: usize,
    pub independent_quality: Option<KiCadRouteQuality>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadObservedRoutingDifficulty {
    pub connection: String,
    pub previous_ordinal: usize,
    pub status: String,
    pub additional_cost_mm: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadAdaptiveRoutingPass {
    pub ordinal: usize,
    pub parent_pass: Option<usize>,
    pub reason: String,
    pub directory: PathBuf,
    pub config: KiCadSequentialRouterConfig,
    pub sequential: Option<KiCadSequentialRouterResult>,
    pub native: Option<VerificationReport>,
    pub error: Option<String>,
    pub fixed_design_preserved: bool,
    pub native_progress_admissible: bool,
    pub score_mm: Option<f64>,
    pub observed_difficulty: Vec<KiCadObservedRoutingDifficulty>,
    pub selected: bool,
    pub elapsed_micros: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadAdaptiveRoutingResult {
    pub schema_version: u32,
    pub complete: bool,
    pub routing_complete: bool,
    pub outstanding_annotation_findings: usize,
    pub termination: String,
    pub source_directory: PathBuf,
    pub source_board_sha256: String,
    pub source_unchanged: bool,
    pub board_id: String,
    pub config: KiCadAdaptiveRoutingConfig,
    pub forecasts: Vec<KiCadRoutingForecastEvidence>,
    pub forecast_elapsed_micros: u64,
    /// Whether independent forecasts were generated or unnecessary after the
    /// first native-admitted pass. Empty evidence never means a failed forecast.
    pub forecast_generation: String,
    pub passes: Vec<KiCadAdaptiveRoutingPass>,
    pub selected_pass: Option<usize>,
    pub result_directory: PathBuf,
    pub result_statistics: KiCadBoardStatistics,
    pub result_verification: VerificationReport,
    pub remaining_queued_trials: usize,
    pub contract: String,
}

struct PendingPass {
    parent: Option<usize>,
    reason: String,
    config: KiCadSequentialRouterConfig,
}

fn score(quality: &KiCadRouteQuality, penalty: f64) -> f64 {
    quality.length_mm + quality.vias as f64 * penalty
}

#[cfg(test)]
fn design_clean(native: &VerificationReport) -> bool {
    native.erc_violations == 0
        && native.drc_design_violations == 0
        && native.schematic_parity_issues == 0
}

fn annotation_finding(finding: &serde_json::Value) -> bool {
    matches!(
        finding["type"].as_str(),
        Some("silk_overlap" | "silk_over_copper" | "silk_edge_clearance")
    )
}

pub(super) fn unchanged_finding_subset(
    baseline: Vec<&serde_json::Value>,
    current: Vec<&serde_json::Value>,
) -> bool {
    let fingerprint = |finding: &serde_json::Value| {
        let mut normalized = finding.clone();
        if let Some(items) = normalized["items"].as_array_mut() {
            items.sort_by_key(|v| v.to_string());
        }
        normalized.to_string()
    };
    let mut counts = BTreeMap::<String, usize>::new();
    for finding in baseline {
        *counts.entry(fingerprint(finding)).or_default() += 1;
    }
    for finding in current {
        match counts.get_mut(&fingerprint(finding)) {
            Some(count) if *count > 0 => *count -= 1,
            _ => return false,
        }
    }
    true
}

// This is a provisional routing gate. It never rewrites native completion or
// waives a newly introduced finding, even one with the same type/count.
pub(super) fn native_progress_admissible(
    native: &VerificationReport,
    drc: &serde_json::Value,
    baseline_drc: &serde_json::Value,
    allow_annotations: bool,
) -> Result<bool, String> {
    let findings = drc_design_issues(drc);
    if native.erc_violations != 0
        || native.schematic_parity_issues != 0
        || findings.len() != native.drc_design_violations
    {
        return Ok(false);
    }
    if findings.is_empty() {
        return Ok(true);
    }
    Ok(allow_annotations
        && findings.iter().all(|f| annotation_finding(f))
        && unchanged_finding_subset(drc_design_issues(baseline_drc), findings))
}

/// Missing connections precede route quality. A malformed or failed native
/// evaluation cannot displace a retained board, even if its copper is shorter.
fn improves(
    native: &VerificationReport,
    candidate_score: f64,
    incumbent: &VerificationReport,
    incumbent_score: f64,
    admissible: bool,
) -> bool {
    admissible
        && !(incumbent.complete && !native.complete)
        && ((native.complete && !incumbent.complete)
            || native.selected_net_unconnected_items < incumbent.selected_net_unconnected_items
            || (native.selected_net_unconnected_items == incumbent.selected_net_unconnected_items
                && candidate_score + 1e-6 < incumbent_score))
}

fn observed_difficulty(
    order: &[String],
    steps: &[KiCadSequentialRouteStep],
    forecasts: &[KiCadRoutingForecastEvidence],
    penalty: f64,
) -> Vec<KiCadObservedRoutingDifficulty> {
    // A later recovery may replace an earlier net. Learn from its final
    // geometry, not the route that existed before that recovery.
    let mut final_quality = BTreeMap::new();
    for step in steps {
        if let Some(quality) = step
            .attempts
            .iter()
            .find(|a| a.selected)
            .and_then(|a| a.quality.as_ref())
        {
            final_quality.insert(step.connection.as_str(), quality);
        }
        if let Some(repair) = step.repair.as_ref().filter(|r| r.selected) {
            if let Some(quality) = &repair.target_quality {
                final_quality.insert(step.connection.as_str(), quality);
            }
            if let (Some(connection), Some(quality)) =
                (&repair.yielded_connection, &repair.yielded_quality)
            {
                final_quality.insert(connection.as_str(), quality);
            }
            for (connection, receipt) in &repair.changed_routes {
                final_quality.insert(connection.as_str(), &receipt.quality);
            }
        }
    }
    let mut result = Vec::new();
    for (ordinal, connection) in order.iter().enumerate() {
        let step = steps.iter().find(|step| step.connection == *connection);
        let selected = final_quality.get(connection.as_str()).copied();
        let baseline = forecasts
            .iter()
            .find(|f| f.connection == *connection)
            .and_then(|f| f.independent_quality.as_ref());
        let (status, additional_cost_mm) = match (step, selected, baseline) {
            (None, _, _) => ("not_attempted", None),
            (Some(_), None, _) => ("failed", None),
            (_, Some(_), None) => ("routed_without_forecast", None),
            (_, Some(actual), Some(baseline)) => {
                let delta = score(actual, penalty) - score(baseline, penalty);
                ("routed", Some(if delta.abs() < 1e-6 { 0.0 } else { delta }))
            }
        };
        result.push(KiCadObservedRoutingDifficulty {
            connection: connection.clone(),
            previous_ordinal: ordinal,
            status: status.into(),
            additional_cost_mm,
        });
    }
    let priority = |status: &str| match status {
        "failed" => 0,
        "not_attempted" => 1,
        "routed_without_forecast" => 2,
        _ => 3,
    };
    result.sort_by(|a, b| {
        priority(&a.status)
            .cmp(&priority(&b.status))
            .then_with(|| {
                b.additional_cost_mm
                    .unwrap_or(0.0)
                    .total_cmp(&a.additional_cost_mm.unwrap_or(0.0))
            })
            .then_with(|| a.previous_ordinal.cmp(&b.previous_ordinal))
    });
    result
}

fn observed_retry_config(
    parent: &KiCadSequentialRouterConfig,
    order: &[String],
    steps: &[KiCadSequentialRouteStep],
    penalty: f64,
) -> Option<KiCadSequentialRouterConfig> {
    let observed = observed_difficulty(order, steps, &[], penalty);
    if !observed
        .iter()
        .any(|d| matches!(d.status.as_str(), "failed" | "not_attempted"))
    {
        return None;
    }
    let order = observed
        .into_iter()
        .map(|d| d.connection)
        .collect::<Vec<_>>();
    if order == parent.connection_order {
        return None;
    }
    let mut retry = parent.clone();
    retry.connection_order = order;
    Some(retry)
}

fn router_cost(mut config: KiCadSequentialRouterConfig) -> KiCadSequentialRouterConfig {
    for routing in &mut config.routing_portfolio {
        if routing.multi_terminal_routing == KiCadMultiTerminalRoutingPolicy::SharedCopperTree {
            routing.tree_attachment_objective = KiCadTreeAttachmentObjective::RouterCost;
        }
    }
    config
}

fn sequential_policy(config: &KiCadAdaptiveRoutingConfig) -> KiCadSequentialRouterConfig {
    let mut sequential = config.sequential.clone();
    if let Some(repair) = &mut sequential.ripup {
        repair.allow_existing_annotation_findings = config.allow_existing_annotation_findings;
    }
    sequential
}

fn upfront_forecast_generation(config: &KiCadAdaptiveRoutingConfig) -> Option<&'static str> {
    if config.initial_demand_strength.is_some() {
        Some("eager_for_initial_demand")
    } else if config.optimize_after_routing_complete {
        Some("eager_for_optimization")
    } else {
        None
    }
}

fn initial_pass(
    config: &KiCadAdaptiveRoutingConfig,
    order: Vec<String>,
    alternatives: &[KiCadDemandAlternative],
) -> PendingPass {
    let mut sequential = sequential_policy(config);
    sequential.connection_order = order;
    let mut reason = "original configured policy".to_string();
    // Missing forecasts are unknown demand, not proof of empty routing space.
    // With no alternatives or no foreign net, preserve the original policy.
    if let Some(strength) = config.initial_demand_strength
        && !alternatives.is_empty()
        && sequential.connection_order.len() > 1
    {
        for routing in &mut sequential.routing_portfolio {
            routing.routing_demand = Some(KiCadRoutingDemand {
                strength,
                shoulder_mm: config.demand_shoulder_mm,
                alternatives: alternatives.to_vec(),
            });
        }
        reason = format!("original configured policy: initial demand {strength}");
    }
    PendingPass {
        parent: None,
        reason,
        config: sequential,
    }
}

fn enqueue(
    queue: &mut VecDeque<PendingPass>,
    seen: &mut BTreeSet<String>,
    pass: PendingPass,
) -> Result<(), String> {
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&pass.config).map_err(|e| e.to_string())?)
    );
    if seen.insert(fingerprint) {
        queue.push_back(pass);
    }
    Ok(())
}

fn enqueue_order(
    queue: &mut VecDeque<PendingPass>,
    seen: &mut BTreeSet<String>,
    config: &KiCadAdaptiveRoutingConfig,
    alternatives: &[KiCadDemandAlternative],
    order: Vec<String>,
    parent: Option<usize>,
    reason: &str,
) -> Result<(), String> {
    let mut base = router_cost(sequential_policy(config));
    base.connection_order = order;
    enqueue(
        queue,
        seen,
        PendingPass {
            parent,
            reason: format!("{reason}: router-cost control"),
            config: base.clone(),
        },
    )?;
    // With no forecasts (or no foreign net), a demand policy has exactly the
    // control's cost field. Do not spend a pass on a serialized no-op.
    if alternatives.is_empty() || base.connection_order.len() < 2 {
        return Ok(());
    }
    for strength in &config.demand_strengths {
        let mut guided = base.clone();
        for routing in &mut guided.routing_portfolio {
            routing.routing_demand = Some(KiCadRoutingDemand {
                strength: *strength,
                shoulder_mm: config.demand_shoulder_mm,
                alternatives: alternatives.to_vec(),
            });
        }
        enqueue(
            queue,
            seen,
            PendingPass {
                parent,
                reason: format!("{reason}: demand {strength}"),
                config: guided,
            },
        )?;
    }
    Ok(())
}

fn forecast(
    source: &Path,
    board_id: &str,
    output: &Path,
    order: &[String],
    config: &KiCadAdaptiveRoutingConfig,
) -> Result<
    (
        Vec<KiCadDemandAlternative>,
        Vec<KiCadRoutingForecastEvidence>,
    ),
    String,
> {
    let directory = output.join("forecasts");
    fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let board = source.join(format!("{board_id}.kicad_pcb"));
    let mut alternatives = Vec::new();
    let mut evidence = Vec::new();
    for (ordinal, connection) in order.iter().enumerate() {
        let mut row = KiCadRoutingForecastEvidence {
            connection: connection.clone(),
            attempts: Vec::new(),
            distinct_alternatives: 0,
            independent_quality: None,
        };
        let mut found = BTreeMap::new();
        for (config_index, routing) in router_cost(config.sequential.clone())
            .routing_portfolio
            .iter()
            .enumerate()
        {
            // These entries recover a failed search on retained copper. An
            // independent forecast has no such trigger; keep its original
            // portfolio and budget rather than running retries unconditionally.
            if config
                .sequential
                .obstacle_distance_fallbacks
                .contains_key(&config_index)
            {
                continue;
            }
            for via_cost in &config.forecast_via_costs_mm {
                if row.attempts.len() >= config.maximum_forecasts_per_net {
                    break;
                }
                let mut routing = routing.clone();
                routing.via_cost_mm = *via_cost;
                let attempt = directory.join(format!("net-{ordinal:03}-{:02}", row.attempts.len()));
                copy_directory_tree(source, &attempt)?;
                // The forecast changes copper but does not run the native
                // gate. Do not leave the copied cold-board reports beside it
                // looking like certificates for this provisional geometry.
                for report in ["verification.json", "drc.json", "erc.json"] {
                    remove_if_present(&attempt.join(report))?;
                }
                write_typed_json(&attempt.join("routing-config.json"), &routing)?;
                let started = Instant::now();
                let candidate = route_materialized_connection(&board, connection, &routing);
                let elapsed_micros = elapsed_micros(started);
                match candidate {
                    Ok(candidate) => {
                        let path = attempt.join("candidate.json");
                        write_typed_json(&path, &candidate)?;
                        apply_route_candidate(&board, &path, &attempt.join(format!("{board_id}.kicad_pcb")))?;
                        let quality = route_candidate_quality(&candidate)?;
                        if row.independent_quality.as_ref().is_none_or(|best| score(&quality, config.sequential.via_penalty_mm) < score(best, config.sequential.via_penalty_mm)) {
                            row.independent_quality = Some(quality.clone());
                        }
                        let paths = candidate.branches.iter().map(|b| b.path.clone()).collect::<Vec<_>>();
                        let fingerprint = serde_json::to_string(&paths).map_err(|e| e.to_string())?;
                        found.entry(fingerprint).or_insert(KiCadDemandAlternative { connection: connection.clone(), weight: 0.0,
                            trace_width_mm: candidate.config.trace_width_mm, clearance_mm: candidate.config.clearance_mm,
                            via_size_mm: candidate.config.via_size_mm, paths });
                        row.attempts.push(serde_json::json!({"directory":attempt,"found":true,"quality":quality,"elapsed_micros":elapsed_micros}));
                    }
                    Err(error) => row.attempts.push(serde_json::json!({"directory":attempt,"found":false,"error":error,"elapsed_micros":elapsed_micros})),
                }
                let _ = verification_preview::capture(&attempt, board_id, || {
                    Err("Provisional independent forecast; native verification was not run for this forecast.".into())
                });
            }
        }
        row.distinct_alternatives = found.len();
        for (_, mut alternative) in found {
            alternative.weight = 1.0 / row.distinct_alternatives as f64;
            alternatives.push(alternative);
        }
        evidence.push(row);
        write_typed_json(&output.join("forecast-evidence.json"), &evidence)?;
    }
    write_typed_json(&output.join("forecast-alternatives.json"), &alternatives)?;
    Ok((alternatives, evidence))
}

fn elapsed_micros(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

fn write_forecast_generation(
    output: &Path,
    status: &str,
    elapsed_micros: u64,
) -> Result<(), String> {
    write_typed_json(
        &output.join("forecast-generation.json"),
        &serde_json::json!({"status":status,"elapsed_micros":elapsed_micros}),
    )
}

fn progress_page(
    output: &Path,
    passes: &[KiCadAdaptiveRoutingPass],
    selected: &Path,
    finished: bool,
) -> Result<(), String> {
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let relative = |path: &Path| {
        path.strip_prefix(output)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    };
    let mut page = String::from(
        "<!doctype html><meta charset=\"utf-8\"><title>Adaptive native routing</title><style>body{font:17px system-ui;margin:24px}table{border-collapse:collapse}td,th{padding:8px;border:1px solid #aaa}.pair{display:grid;grid-template-columns:1fr 1fr;gap:20px}img{width:100%}input{width:75%}</style><h1>Adaptive native routing</h1>",
    );
    page.push_str(if finished { "<p>Bounded run finished. The retained result is shown below; inspect completion in the native report.</p>" } else { "<p>Routing is in progress. Refresh to see completed passes. Frames show committed routing steps, not a force simulation.</p>" });
    page.push_str("<table><tr><th>Pass</th><th>Proposal</th><th>Native opens</th><th>Native complete</th><th>Design findings</th><th>Score mm</th><th>Retained</th></tr>");
    let mut frames = Vec::new();
    for pass in passes {
        let opens = pass
            .native
            .as_ref()
            .map_or("verification failed".into(), |n| {
                n.selected_net_unconnected_items.to_string()
            });
        page.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            pass.ordinal,
            escape(&pass.reason),
            opens,
            pass.native.as_ref().is_some_and(|n| n.complete),
            pass.native.as_ref().map_or("unknown".into(), |n| n.drc_design_violations.to_string()),
            pass.score_mm.map_or("—".into(), |s| format!("{s:.3}")),
            pass.selected
        ));
        if let Some(sequential) = &pass.sequential {
            for step in &sequential.steps {
                if let Some(admission) = step.selected_admission() {
                    let repaired = step.repair.as_ref().is_some_and(|r| r.selected);
                    frames.push(serde_json::json!({"src":relative(&admission.directory.join("preview.svg")), "label":format!("Pass {} · net {} · {}{}", pass.ordinal, step.ordinal+1, step.connection, if repaired { " · rip-up recovery" } else { "" })}));
                }
            }
        }
    }
    page.push_str(&format!("</table><div class=\"pair\"><div><h2>Retained board</h2><img src=\"{}/preview.svg\"></div><div><h2>Routing sequence</h2><button id=\"play\">Play / pause</button> <input id=\"frame\" type=\"range\" min=\"0\" max=\"{}\" value=\"0\"><p id=\"label\"></p><img id=\"sequence\" src=\"source/preview.svg\"></div></div><p><a href=\"adaptive-routing.json\">Final evidence</a> · <a href=\"passes.json\">Pass evidence</a> · <a href=\"forecast-evidence.json\">Forecast coverage</a></p>", escape(&relative(selected)), frames.len().saturating_sub(1)));
    let frames = serde_json::to_string(&frames)
        .map_err(|e| e.to_string())?
        .replace('<', "\\u003c");
    page.push_str(&format!("<script>const frames={frames};const slider=document.getElementById('frame');let timer=null;function show(){{const f=frames[Number(slider.value)];if(f){{document.getElementById('sequence').src=f.src;document.getElementById('label').textContent=f.label;}}}}slider.oninput=show;document.getElementById('play').onclick=()=>{{if(timer){{clearInterval(timer);timer=null;}}else if(frames.length){{timer=setInterval(()=>{{slider.value=(Number(slider.value)+1)%frames.length;show();}},800);}}}};show();</script>"));
    fs::write(output.join("index.html"), page).map_err(|e| e.to_string())
}

/// Forecasts come only from this zero-copper source. Every attempt reroutes
/// from the same source. Routing stops at native connectivity completion unless
/// further optimization is requested; full layout admission still requires all
/// native checks. Complete incumbents survive failed optimization passes.
pub fn route_kicad_board_adaptively(
    source: &Path,
    board_id: &str,
    output: &Path,
    config: &KiCadAdaptiveRoutingConfig,
) -> Result<KiCadAdaptiveRoutingResult, String> {
    config.check()?;
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let output = output
        .parent()
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(output.file_name().ok_or("output needs a directory name")?);
    if output.exists() || output.starts_with(&source) {
        return Err("adaptive output must be fresh and outside source".into());
    }
    let board_name = format!("{board_id}.kicad_pcb");
    let source_board = source.join(&board_name);
    let source_hash = file_sha256(&source_board)?;
    let stats = inspect_kicad_board(&source_board)?;
    if stats.segments != 0
        || stats.arcs != 0
        || stats.vias != 0
        || stats.copper_zones != 0
        || stats.copper_graphics != 0
    {
        return Err("adaptive routing requires an explicitly prepared zero-copper source".into());
    }
    let discovered_connections = inspect_kicad_routable_connections(&source_board)?;
    let order = initial_connection_order(&discovered_connections, config);
    let discovered = discovered_connections
        .iter()
        .map(|n| &n.connection)
        .collect::<Vec<_>>();
    if order.is_empty()
        || order.len() != discovered.len()
        || order.iter().collect::<BTreeSet<_>>() != discovered.into_iter().collect::<BTreeSet<_>>()
    {
        return Err("adaptive order must contain every discovered connection exactly once".into());
    }
    if config.sequential.maximum_connections < order.len() {
        return Err("adaptive maximum_connections must allow a whole-board pass".into());
    }
    fs::create_dir(&output).map_err(|e| e.to_string())?;
    write_typed_json(&output.join("config.json"), config)?;
    let baseline = output.join("source");
    copy_directory_tree(&source, &baseline)?;
    let baseline_native = verify_materialized_rung(&baseline, board_id)?;
    let baseline_drc = read_json(&baseline.join("drc.json"))?;
    if !native_progress_admissible(
        &baseline_native,
        &baseline_drc,
        &baseline_drc,
        config.allow_existing_annotation_findings,
    )? {
        return Err("adaptive cold source must have no ERC, physical DRC or schematic parity issues; existing annotation findings require explicit provisional routing opt-in".into());
    }
    let fixed = coupled_vias::fixed_design(parse(
        &fs::read_to_string(&source_board).map_err(|e| e.to_string())?,
    )?);
    let project_bytes = fs::read(source.join(format!("{board_id}.kicad_pro"))).ok();
    let erc_inputs = kicad_erc_input_sha256(&source)?;
    let mut alternatives = Vec::new();
    let mut forecasts = Vec::new();
    let mut forecast_elapsed_micros = 0;
    let mut forecast_generation = "deferred_until_initial_pass";
    // Defaults defer forecasts until another pass needs them. Explicit initial
    // demand and optimization share one upfront batch, even for a one-pass run.
    if let Some(generation) = upfront_forecast_generation(config) {
        let started = Instant::now();
        (alternatives, forecasts) = forecast(&baseline, board_id, &output, &order, config)?;
        forecast_elapsed_micros = elapsed_micros(started);
        forecast_generation = generation;
    } else {
        write_typed_json(&output.join("forecast-evidence.json"), &forecasts)?;
        write_typed_json(&output.join("forecast-alternatives.json"), &alternatives)?;
    }
    write_forecast_generation(&output, forecast_generation, forecast_elapsed_micros)?;
    let mut queue = VecDeque::new();
    let mut seen = BTreeSet::new();
    enqueue(
        &mut queue,
        &mut seen,
        initial_pass(config, order.clone(), &alternatives),
    )?;
    if upfront_forecast_generation(config).is_some() {
        enqueue_order(
            &mut queue,
            &mut seen,
            config,
            &alternatives,
            order.clone(),
            None,
            "initial order",
        )?;
    }
    let mut passes = Vec::new();
    let mut selected_pass = None;
    let mut selected_directory = baseline.clone();
    let mut selected_native = baseline_native;
    let mut selected_score = 0.0;
    while passes.len() < config.maximum_passes {
        let Some(pending) = queue.pop_front() else {
            break;
        };
        let started = Instant::now();
        let ordinal = passes.len();
        let directory = output.join(format!("pass-{ordinal:03}"));
        let sequential =
            route_kicad_board_sequentially(&baseline, board_id, &directory, &pending.config);
        let mut pass = KiCadAdaptiveRoutingPass {
            ordinal,
            parent_pass: pending.parent,
            reason: pending.reason,
            directory: directory.clone(),
            config: pending.config,
            sequential: None,
            native: None,
            error: None,
            fixed_design_preserved: false,
            native_progress_admissible: false,
            score_mm: None,
            observed_difficulty: Vec::new(),
            selected: false,
            elapsed_micros: 0,
        };
        match sequential {
            Err(error) => {
                pass.error = Some(error);
            }
            Ok(sequential) => {
                let result_board = &sequential.result_board;
                pass.fixed_design_preserved = fixed
                    == coupled_vias::fixed_design(parse(
                        &fs::read_to_string(result_board).map_err(|e| e.to_string())?,
                    )?)
                    && fs::read(
                        sequential
                            .result_directory
                            .join(format!("{board_id}.kicad_pro")),
                    )
                    .ok()
                        == project_bytes
                    && kicad_erc_input_sha256(&sequential.result_directory)? == erc_inputs;
                pass.score_mm = Some(
                    sequential
                        .final_statistics
                        .physical_copper
                        .physical_centerline_length_mm
                        + sequential.final_statistics.vias as f64
                            * config.sequential.via_penalty_mm,
                );
                match verify_materialized_rung(&sequential.result_directory, board_id) {
                    Ok(native) => {
                        let drc = read_json(&sequential.result_directory.join("drc.json"))?;
                        pass.native_progress_admissible = native_progress_admissible(
                            &native,
                            &drc,
                            &baseline_drc,
                            config.allow_existing_annotation_findings,
                        )?;
                        if pass.fixed_design_preserved
                            && improves(
                                &native,
                                pass.score_mm.unwrap(),
                                &selected_native,
                                selected_score,
                                pass.native_progress_admissible,
                            )
                        {
                            selected_directory = sequential.result_directory.clone();
                            selected_score = pass.score_mm.unwrap();
                            selected_native = native.clone();
                            selected_pass = Some(ordinal);
                        }
                        pass.native = Some(native);
                    }
                    Err(error) => pass.error = Some(error),
                }
                pass.sequential = Some(sequential);
            }
        }
        let mut deferred_forecast_micros = 0;
        if matches!(
            forecast_generation,
            "deferred_until_initial_pass" | "deferred_for_observed_retry"
        ) {
            let mut retry_queued = false;
            if config.retry_observed_failures_before_forecasts
                && ordinal == 0
                && ordinal + 1 < config.maximum_passes
                && pass.error.is_none()
                && pass.fixed_design_preserved
                && pass.native_progress_admissible
                && pass
                    .native
                    .as_ref()
                    .is_some_and(|native| native.selected_net_unconnected_items > 0)
                && let Some(sequential) = &pass.sequential
                && let Some(retry) = observed_retry_config(
                    &pass.config,
                    &sequential.connection_order,
                    &sequential.steps,
                    config.sequential.via_penalty_mm,
                )
            {
                let before = queue.len();
                enqueue(
                    &mut queue,
                    &mut seen,
                    PendingPass {
                        parent: Some(ordinal),
                        reason: "observed failure before independent forecasts".into(),
                        config: retry,
                    },
                )?;
                retry_queued = queue.len() > before;
            }
            if selected_native.selected_net_unconnected_items == 0 {
                forecast_generation = "skipped_routing_complete";
            } else if ordinal + 1 >= config.maximum_passes {
                forecast_generation = "skipped_pass_limit";
            } else if retry_queued {
                forecast_generation = "deferred_for_observed_retry";
            } else {
                // Forecast generation can consume the remaining process
                // budget. Persist the already-verified first pass before
                // starting it, so a timeout retains an explicit incumbent.
                if file_sha256(&source_board)? != source_hash {
                    return Err("adaptive source changed during routing".into());
                }
                pass.selected = selected_pass == Some(ordinal);
                pass.elapsed_micros = elapsed_micros(started);
                let mut returned_passes = passes.clone();
                returned_passes.push(pass.clone());
                for returned in &mut returned_passes {
                    returned.selected = selected_pass == Some(returned.ordinal);
                }
                write_typed_json(&output.join("passes.json"), &returned_passes)?;
                write_typed_json(
                    &output.join("progress.json"),
                    &serde_json::json!({"completed_passes":returned_passes.len(),"queued_passes":queue.len(),"selected_pass":selected_pass,
                    "selected_directory":selected_directory,"selected_native":selected_native,"selected_score_mm":selected_score,
                    "phase":"generating_forecasts"}),
                )?;
                progress_page(&output, &returned_passes, &selected_directory, false)?;
                forecast_generation = if ordinal == 0 {
                    "generating_after_initial_pass"
                } else {
                    "generating_after_observed_retry"
                };
                write_forecast_generation(&output, forecast_generation, 0)?;
                let forecast_started = Instant::now();
                (alternatives, forecasts) = forecast(&baseline, board_id, &output, &order, config)?;
                forecast_elapsed_micros = elapsed_micros(forecast_started);
                deferred_forecast_micros = forecast_elapsed_micros;
                forecast_generation = if ordinal == 0 {
                    "generated_after_initial_pass"
                } else {
                    "generated_after_observed_retry"
                };
                // Preserve the eager planner's queue exactly: original-order
                // variants precede variants learned from this pass. This also
                // applies when the first sequential invocation returned Err.
                enqueue_order(
                    &mut queue,
                    &mut seen,
                    config,
                    &alternatives,
                    order.clone(),
                    None,
                    "initial order",
                )?;
                // The early retry deferred the first pass's forecast-informed
                // proposals. Restore those in chronological order now.
                for prior in &mut passes {
                    if let Some(sequential) = &prior.sequential {
                        prior.observed_difficulty = observed_difficulty(
                            &sequential.connection_order,
                            &sequential.steps,
                            &forecasts,
                            config.sequential.via_penalty_mm,
                        );
                        if prior.fixed_design_preserved {
                            enqueue_order(
                                &mut queue,
                                &mut seen,
                                config,
                                &alternatives,
                                prior
                                    .observed_difficulty
                                    .iter()
                                    .map(|d| d.connection.clone())
                                    .collect(),
                                Some(prior.ordinal),
                                "observed failure and cost inflation",
                            )?;
                        }
                    }
                }
            }
            write_forecast_generation(&output, forecast_generation, forecast_elapsed_micros)?;
        }
        if let Some(sequential) = &pass.sequential {
            pass.observed_difficulty = observed_difficulty(
                &sequential.connection_order,
                &sequential.steps,
                &forecasts,
                config.sequential.via_penalty_mm,
            );
            if pass.fixed_design_preserved
                && !forecast_generation.starts_with("skipped_")
                && forecast_generation != "deferred_for_observed_retry"
            {
                let learned_order = pass
                    .observed_difficulty
                    .iter()
                    .map(|d| d.connection.clone())
                    .collect();
                enqueue_order(
                    &mut queue,
                    &mut seen,
                    config,
                    &alternatives,
                    learned_order,
                    Some(ordinal),
                    "observed failure and cost inflation",
                )?;
            }
        }
        pass.elapsed_micros = elapsed_micros(started).saturating_sub(deferred_forecast_micros);
        passes.push(pass);
        for p in &mut passes {
            p.selected = selected_pass == Some(p.ordinal);
        }
        write_typed_json(&output.join("passes.json"), &passes)?;
        write_typed_json(
            &output.join("progress.json"),
            &serde_json::json!({"completed_passes":passes.len(),"queued_passes":queue.len(),"selected_pass":selected_pass,
            "selected_directory":selected_directory,"selected_native":selected_native,"selected_score_mm":selected_score}),
        )?;
        progress_page(&output, &passes, &selected_directory, false)?;
        if file_sha256(&source_board)? != source_hash {
            return Err("adaptive source changed during routing".into());
        }
        if selected_native.selected_net_unconnected_items == 0
            && !config.optimize_after_routing_complete
        {
            break;
        }
    }
    let result_directory = output.join("result");
    copy_directory_tree(&selected_directory, &result_directory)?;
    let result_statistics = inspect_kicad_board(&result_directory.join(&board_name))?;
    let result_verification = verify_materialized_rung(&result_directory, board_id)?;
    if result_verification != selected_native {
        return Err("selected adaptive result did not reproduce native verification".into());
    }
    let final_drc = read_json(&result_directory.join("drc.json"))?;
    if !native_progress_admissible(
        &result_verification,
        &final_drc,
        &baseline_drc,
        config.allow_existing_annotation_findings,
    )? {
        return Err("retained result failed the provisional native gate".into());
    }
    let result = KiCadAdaptiveRoutingResult { schema_version: 4, complete: result_verification.complete,
        routing_complete: result_verification.selected_net_unconnected_items == 0,
        outstanding_annotation_findings: drc_design_issues(&final_drc).iter().filter(|f| annotation_finding(f)).count(),
        termination: if result_verification.selected_net_unconnected_items == 0 && !config.optimize_after_routing_complete {
            "routing_complete"
        } else if forecast_generation == "skipped_pass_limit" { "maximum_passes"
        } else if queue.is_empty() { "proposal_queue_exhausted" } else { "maximum_passes" }.into(),
        source_directory: source, source_board_sha256: source_hash.clone(), source_unchanged: file_sha256(&source_board)? == source_hash,
        board_id: board_id.into(), config: config.clone(), forecasts, forecast_elapsed_micros,
        forecast_generation: forecast_generation.into(), passes, selected_pass,
        result_directory, result_statistics, result_verification, remaining_queued_trials: queue.len(),
        contract: "independent zero-copper forecasts generated only when another pass is needed, or eagerly for explicit initial demand or optimization; initial demand preserves the configured first-pass order and attachment policy; bounded distinct policy/order passes; promote failures and observed cost inflation; original physical rules and fixed design preserved; provisional admission allows only explicitly opted-in, unchanged source annotations; stop at routing completion unless further optimization is explicitly requested; final completion still requires a clean native board; fewer opens precede physical length plus explicit via penalty; retain incumbent through later failures".into() };
    write_typed_json(&output.join("adaptive-routing.json"), &result)?;
    progress_page(&output, &result.passes, &result.result_directory, true)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn improves(
        native: &VerificationReport,
        score: f64,
        incumbent: &VerificationReport,
        previous: f64,
    ) -> bool {
        super::improves(native, score, incumbent, previous, design_clean(native))
    }
    fn native(opens: usize) -> VerificationReport {
        VerificationReport {
            board_id: "test".into(),
            complete: opens == 0,
            erc_violations: 0,
            drc_design_violations: 0,
            schematic_parity_issues: 0,
            selected_net_unconnected_items: opens,
            intentional_no_connect_groups: 0,
            library_metadata_warnings: 7,
        }
    }
    #[test]
    fn existing_configs_default_to_completion_before_optional_optimization() {
        let config: KiCadAdaptiveRoutingConfig =
            serde_json::from_value(serde_json::json!({"maximum_passes": 4})).unwrap();
        assert!(!config.optimize_after_routing_complete);
        assert!(!config.retry_observed_failures_before_forecasts);
        assert!(config.initial_demand_strength.is_none());
        assert!(
            serde_json::to_value(&config)
                .unwrap()
                .get("initial_demand_strength")
                .is_none()
        );
        assert_eq!(config.initial_order, KiCadAdaptiveInitialOrder::Discovery);
        assert!(
            serde_json::to_value(&config)
                .unwrap()
                .get("initial_order")
                .is_none()
        );
        assert!(
            serde_json::to_value(&config)
                .unwrap()
                .get("retry_observed_failures_before_forecasts")
                .is_none()
        );
        let retry: KiCadAdaptiveRoutingConfig = serde_json::from_value(serde_json::json!({
            "retry_observed_failures_before_forecasts": true
        }))
        .unwrap();
        assert!(retry.retry_observed_failures_before_forecasts);
        let optimization: KiCadAdaptiveRoutingConfig =
            serde_json::from_value(serde_json::json!({"optimize_after_routing_complete": true}))
                .unwrap();
        assert!(optimization.optimize_after_routing_complete);
    }

    #[test]
    fn initial_demand_requires_valid_strength_and_runs_upfront_even_with_one_pass() {
        let mut config = KiCadAdaptiveRoutingConfig::default();
        assert_eq!(upfront_forecast_generation(&config), None);
        config.maximum_passes = 1;
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY, 1_000_001.0] {
            config.initial_demand_strength = Some(invalid);
            assert!(config.check().is_err());
        }
        config.initial_demand_strength = Some(1.0);
        config.check().unwrap();
        assert_eq!(
            upfront_forecast_generation(&config),
            Some("eager_for_initial_demand")
        );
        config.optimize_after_routing_complete = true;
        assert_eq!(
            upfront_forecast_generation(&config),
            Some("eager_for_initial_demand")
        );
        config.initial_demand_strength = None;
        assert_eq!(
            upfront_forecast_generation(&config),
            Some("eager_for_optimization")
        );
    }

    #[test]
    fn first_pass_reserves_forecasts_without_changing_order_or_attachment_policy() {
        let mut config = KiCadAdaptiveRoutingConfig::default();
        config.initial_demand_strength = Some(1.0);
        config.sequential.routing_portfolio[0].tree_attachment_objective =
            KiCadTreeAttachmentObjective::LengthThenVias;
        let order = vec!["B".into(), "A".into()];
        let alternatives = vec![KiCadDemandAlternative {
            connection: "A".into(),
            weight: 1.0,
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            paths: vec![vec![
                KiCadRoutePoint {
                    at: [0.0, 0.0],
                    layer: "F.Cu".into(),
                },
                KiCadRoutePoint {
                    at: [1.0, 0.0],
                    layer: "F.Cu".into(),
                },
            ]],
        }];
        let guided = initial_pass(&config, order.clone(), &alternatives);
        assert_eq!(guided.config.connection_order, order);
        assert_eq!(
            guided.config.routing_portfolio[0].tree_attachment_objective,
            KiCadTreeAttachmentObjective::LengthThenVias
        );
        let mut stripped = guided.config.clone();
        for route in &mut stripped.routing_portfolio {
            let demand = route.routing_demand.take().unwrap();
            assert_eq!(demand.strength, 1.0);
            assert_eq!(
                serde_json::to_value(demand.alternatives).unwrap(),
                serde_json::to_value(&alternatives).unwrap()
            );
        }
        let mut expected = sequential_policy(&config);
        expected.connection_order = order.clone();
        assert_eq!(
            serde_json::to_value(stripped).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let empty = initial_pass(&config, order.clone(), &[]);
        assert_eq!(
            serde_json::to_value(empty.config).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let single = initial_pass(&config, vec!["A".into()], &alternatives);
        assert!(
            single
                .config
                .routing_portfolio
                .iter()
                .all(|r| r.routing_demand.is_none())
        );
        config.initial_demand_strength = None;
        assert_eq!(
            serde_json::to_value(initial_pass(&config, order, &alternatives).config).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }

    #[test]
    fn guided_first_pass_keeps_an_unguided_fallback_without_duplicate_guided_trials() {
        let mut config = KiCadAdaptiveRoutingConfig::default();
        config.initial_demand_strength = Some(1.0);
        config.sequential = router_cost(config.sequential);
        let order = vec!["A".into(), "B".into()];
        let alternatives = vec![KiCadDemandAlternative {
            connection: "B".into(),
            weight: 1.0,
            trace_width_mm: 0.2,
            clearance_mm: 0.2,
            via_size_mm: 0.6,
            paths: vec![],
        }];
        let mut queue = VecDeque::new();
        let mut seen = BTreeSet::new();
        enqueue(
            &mut queue,
            &mut seen,
            initial_pass(&config, order.clone(), &alternatives),
        )
        .unwrap();
        enqueue_order(
            &mut queue,
            &mut seen,
            &config,
            &alternatives,
            order,
            None,
            "initial order",
        )
        .unwrap();
        assert_eq!(queue.len(), 2);
        assert!(
            queue[0]
                .config
                .routing_portfolio
                .iter()
                .all(|r| r.routing_demand.is_some())
        );
        assert!(
            queue[1]
                .config
                .routing_portfolio
                .iter()
                .all(|r| r.routing_demand.is_none())
        );
    }

    #[test]
    fn initial_electrical_order_preserves_ties_and_distinct_layer_contacts() {
        let discovered = vec![
            KiCadConnectionSummary {
                connection: "first-tie".into(),
                distinct_pad_centers: 3,
                electrical_terminal_count: None,
            },
            KiCadConnectionSummary {
                connection: "coincident-layers".into(),
                distinct_pad_centers: 2,
                electrical_terminal_count: Some(4),
            },
            KiCadConnectionSummary {
                connection: "second-tie".into(),
                distinct_pad_centers: 3,
                electrical_terminal_count: None,
            },
        ];
        let mut config = KiCadAdaptiveRoutingConfig::default();
        assert_eq!(
            initial_connection_order(&discovered, &config),
            ["first-tie", "coincident-layers", "second-tie"]
        );
        config.initial_order = KiCadAdaptiveInitialOrder::ElectricalTerminalCountDescending;
        assert_eq!(
            initial_connection_order(&discovered, &config),
            ["coincident-layers", "first-tie", "second-tie"]
        );
        let serialized = serde_json::to_value(&config).unwrap();
        assert_eq!(
            serialized["initial_order"],
            "electrical_terminal_count_descending"
        );
        assert_eq!(
            serde_json::from_value::<KiCadAdaptiveRoutingConfig>(serialized)
                .unwrap()
                .initial_order,
            config.initial_order
        );
    }

    #[test]
    fn automatic_initial_order_rejects_explicit_order_before_routing() {
        let mut config = KiCadAdaptiveRoutingConfig::default();
        config.sequential.connection_order = vec!["explicit".into()];
        assert!(config.check().is_ok());
        assert_eq!(initial_connection_order(&[], &config), ["explicit"]);
        config.initial_order = KiCadAdaptiveInitialOrder::ElectricalTerminalCountDescending;
        assert!(
            config
                .check()
                .unwrap_err()
                .contains("conflicts with explicit")
        );
    }
    #[test]
    fn provisional_annotations_never_hide_new_findings_or_complete_a_board() {
        let finding = |kind: &str, id: &str| {
            serde_json::json!({
            "type":kind,"severity":"error","description":"test finding",
            "items":[{"uuid":id,"description":"test item","pos":{"x":1.0,"y":2.0}}]})
        };
        let base = serde_json::json!({"violations":[finding("silk_overlap","old")]});
        let mut report = native(0);
        report.complete = false;
        report.drc_design_violations = 1;
        assert!(!native_progress_admissible(&report, &base, &base, false).unwrap());
        assert!(native_progress_admissible(&report, &base, &base, true).unwrap());
        assert!(!report.complete);
        let changed = serde_json::json!({"violations":[finding("silk_overlap","new")]});
        assert!(!native_progress_admissible(&report, &changed, &base, true).unwrap());
        let physical = serde_json::json!({"violations":[finding("clearance","old")]});
        assert!(!native_progress_admissible(&report, &physical, &physical, true).unwrap());
        assert!(!super::improves(&report, 0.0, &native(0), 100.0, true));
        assert!(super::improves(&native(0), 100.0, &report, 0.0, true));
        report.erc_violations = 1;
        assert!(!native_progress_admissible(&report, &base, &base, true).unwrap());
        report.erc_violations = 0;
        report.schematic_parity_issues = 1;
        assert!(!native_progress_admissible(&report, &base, &base, true).unwrap());
        assert!(!super::improves(&report, 0.0, &native(4), 100.0, false));
    }

    #[test]
    fn incumbent_rejects_short_incomplete_or_invalid_boards() {
        assert!(!improves(&native(1), 1.0, &native(0), 100.0));
        assert!(improves(&native(0), 200.0, &native(1), 1.0));
        assert!(improves(&native(0), 99.0, &native(0), 100.0));
        let mut invalid = native(0);
        invalid.drc_design_violations = 1;
        assert!(!improves(&invalid, 1.0, &native(0), 100.0));
        assert!(!improves(&native(0), 100.0 - 1e-8, &native(0), 100.0));
    }
    #[test]
    fn equivalent_policy_orders_are_not_repeated() {
        let mut queue = VecDeque::new();
        let mut seen = BTreeSet::new();
        let config = KiCadAdaptiveRoutingConfig::default();
        let mut original = config.sequential.clone();
        original.connection_order = vec!["A".into(), "B".into()];
        enqueue(
            &mut queue,
            &mut seen,
            PendingPass {
                parent: None,
                reason: "original".into(),
                config: original,
            },
        )
        .unwrap();
        enqueue_order(
            &mut queue,
            &mut seen,
            &config,
            &[],
            vec!["A".into(), "B".into()],
            Some(0),
            "learned",
        )
        .unwrap();
        assert_eq!(queue.len(), 1); // Rooted-star has no separate attachment objective.
        enqueue_order(
            &mut queue,
            &mut seen,
            &config,
            &[],
            vec!["B".into(), "A".into()],
            Some(0),
            "learned",
        )
        .unwrap();
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn failed_and_unattempted_nets_precede_observed_cost_inflation() {
        let quality = |length_mm| KiCadRouteQuality {
            length_mm,
            segments: 1,
            stored_track_length_mm: length_mm,
            stored_segments: 1,
            overlapping_track_length_mm: 0.0,
            vias: 0,
            branch_via_transitions: 0,
            maximum_branch_vias: 0,
            via_cluster_threshold_mm: 1.0,
            minimum_via_spacing_mm: None,
            close_via_pairs: 0,
            clustered_vias: 0,
            branch_bends: 0,
            branch_points: 2,
        };
        let order = ["easy", "costly", "failed", "unattempted"].map(String::from);
        let forecasts = order
            .iter()
            .map(|name| KiCadRoutingForecastEvidence {
                connection: name.clone(),
                attempts: Vec::new(),
                distinct_alternatives: 1,
                independent_quality: Some(quality(10.0)),
            })
            .collect::<Vec<_>>();
        let mut steps = order[..3]
            .iter()
            .enumerate()
            .map(|(i, name)| KiCadSequentialRouteStep {
                ordinal: i,
                connection: name.clone(),
                artifact_directory: PathBuf::new(),
                attempts: if i == 2 {
                    vec![]
                } else {
                    vec![KiCadSequentialRouteAttempt {
                        config_index: 0,
                        candidate: None,
                        quality: Some(quality(if i == 0 { 10.0 + 1e-8 } else { 30.0 })),
                        score_mm: None,
                        expansions: 0,
                        error: None,
                        route_failure: None,
                        skip_reason: None,
                        native_admission: None,
                        selected: true,
                    }]
                },
                selected_config_index: None,
                repair: None,
                repair_skip_reason: None,
                board_sha256_after: None,
                elapsed_micros: 0,
            })
            .collect::<Vec<_>>();
        let ranked = observed_difficulty(&order, &steps, &forecasts, 5.0);
        assert_eq!(
            ranked
                .iter()
                .map(|r| r.connection.as_str())
                .collect::<Vec<_>>(),
            vec!["failed", "unattempted", "costly", "easy"]
        );
        assert_eq!(ranked[2].additional_cost_mm, Some(20.0));
        assert_eq!(ranked[3].additional_cost_mm, Some(0.0));
        let mut parent = KiCadSequentialRouterConfig::default();
        parent.connection_order = order.to_vec();
        parent.routing_portfolio[0].tree_attachment_objective =
            KiCadTreeAttachmentObjective::LengthThenVias;
        let retry = observed_retry_config(&parent, &order, &steps, 5.0).unwrap();
        assert_eq!(
            retry.connection_order,
            ["failed", "unattempted", "easy", "costly"].map(String::from)
        );
        let mut expected = parent.clone();
        expected.connection_order = retry.connection_order.clone();
        assert_eq!(
            serde_json::to_value(&retry).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert!(observed_retry_config(&retry, &retry.connection_order, &steps, 5.0).is_none());
        // Sparse sweep: an earlier failure does not hide a later committed net.
        let mut sparse_parent = parent.clone();
        sparse_parent.continue_after_routing_failure = true;
        let mut sparse_steps = steps.clone();
        sparse_steps[0].attempts.clear();
        sparse_steps[2].attempts = steps[0].attempts.clone();
        sparse_steps[2].selected_config_index = Some(0);
        let sparse = observed_difficulty(&order, &sparse_steps, &[], 5.0);
        assert_eq!(
            sparse
                .iter()
                .map(|s| (s.connection.as_str(), s.status.as_str()))
                .collect::<Vec<_>>(),
            [
                ("easy", "failed"),
                ("unattempted", "not_attempted"),
                ("costly", "routed_without_forecast"),
                ("failed", "routed_without_forecast")
            ]
        );
        let sparse_retry =
            observed_retry_config(&sparse_parent, &order, &sparse_steps, 5.0).unwrap();
        assert!(sparse_retry.continue_after_routing_failure);
        assert_eq!(
            sparse_retry.connection_order,
            ["easy", "unattempted", "costly", "failed"].map(String::from)
        );
        let mut outer = KiCadAdaptiveRoutingConfig::default();
        outer.sequential = sparse_parent;
        outer.check().unwrap();
        assert!(sequential_policy(&outer).continue_after_routing_failure);
        assert!(!improves(&native(3), 0.0, &native(2), 100.0));

        let mut queue = VecDeque::new();
        let mut seen = BTreeSet::new();
        for _ in 0..2 {
            enqueue(
                &mut queue,
                &mut seen,
                PendingPass {
                    parent: Some(0),
                    reason: "observed failure before independent forecasts".into(),
                    config: retry.clone(),
                },
            )
            .unwrap();
        }
        assert_eq!(queue.len(), 1);
        steps[2].repair = Some(KiCadSequentialRipupEvidence {
            selected: true,
            target_quality: Some(quality(20.0)),
            yielded_connection: Some("easy".into()),
            yielded_quality: Some(quality(50.0)),
            ..Default::default()
        });
        let repaired = observed_difficulty(&order, &steps, &forecasts, 5.0);
        assert_eq!(
            repaired
                .iter()
                .map(|r| r.connection.as_str())
                .collect::<Vec<_>>(),
            vec!["unattempted", "easy", "costly", "failed"]
        );
        assert_eq!(repaired[1].additional_cost_mm, Some(40.0));
        assert_eq!(repaired[3].status, "routed");
        assert!(steps[2].committed());
        steps[2].repair.as_mut().unwrap().changed_routes.insert(
            "costly".into(),
            KiCadChangedRouteReceipt {
                candidate: PathBuf::from("nested.json"),
                candidate_sha256: "test".into(),
                quality: quality(80.0),
            },
        );
        let nested = observed_difficulty(&order, &steps, &forecasts, 5.0);
        assert_eq!(
            nested
                .iter()
                .find(|r| r.connection == "costly")
                .unwrap()
                .additional_cost_mm,
            Some(70.0)
        );
    }
}

// Copyright (C) 2026 Toit contributors.

//! Bounded append-only repair selected from native electrical islands.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadIslandBridgeConfig {
    /// Search policy. Physical dimensions are replaced by native compiled rules.
    pub routing: KiCadGridRouteConfig,
    pub maximum_attempts: usize,
    pub maximum_pair_candidates: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pair {
    connection: String,
    start: KiCadPadReference,
    finish: KiCadPadReference,
}

#[derive(Deserialize)]
struct Islands {
    schema_version: u32,
    board_sha256: String,
    source_unchanged: bool,
    discovery_complete: bool,
    proposals: Vec<Pair>,
}

#[derive(Debug, Serialize)]
pub struct KiCadIslandBridgeResult {
    pub schema_version: u32,
    pub status: String,
    pub reason: Option<String>,
    pub source_board_sha256: String,
    pub source_unchanged: bool,
    pub selected_directory: PathBuf,
    pub selected_native: VerificationReport,
    pub attempts: Vec<serde_json::Value>,
    pub admitted_bridges: usize,
    pub contract: &'static str,
}

fn publish(output: &Path, result: &KiCadIslandBridgeResult) -> Result<(), String> {
    let temporary = output.join(".island-bridges.next.json");
    write_typed_json(&temporary, result)?;
    fs::rename(temporary, output.join("island-bridges.json")).map_err(|e| e.to_string())
}

fn run_python(script: &Path, args: &[&Path], log: &Path) -> Result<(), String> {
    let stream = fs::File::create(log).map_err(|e| e.to_string())?;
    let status = Command::new("python3")
        .arg(script)
        .args(args)
        .stdout(stream.try_clone().map_err(|e| e.to_string())?)
        .stderr(stream)
        .status()
        .map_err(|e| format!("native island helper: {e}"))?;
    if !status.success() {
        return Err(format!(
            "native island helper failed ({status}); see {}",
            log.display()
        ));
    }
    Ok(())
}

fn physical_clean(native: &VerificationReport) -> bool {
    native.drc_design_violations == 0
        && native.erc_violations == 0
        && native.schematic_parity_issues == 0
}

fn design_inputs(directory: &Path, board_name: &str) -> Result<BTreeMap<PathBuf, String>, String> {
    let mut files = Vec::new();
    collect_kicad_cache_inputs(directory, directory, &mut files)?;
    files
        .into_iter()
        .filter(|(relative, _)| relative != Path::new(board_name))
        .map(|(relative, absolute)| Ok((relative, file_sha256(&absolute)?)))
        .collect()
}

fn admission(before: &VerificationReport, after: &VerificationReport, preserved: bool) -> bool {
    preserved
        && physical_clean(after)
        && after.selected_net_unconnected_items < before.selected_net_unconnected_items
        && after.intentional_no_connect_groups == before.intentional_no_connect_groups
        && after.library_metadata_warnings <= before.library_metadata_warnings
}

/// Every original copper item must remain byte-equivalent after S-expression
/// parsing, and all other board nodes must remain exact and in the same order.
fn append_only(before: &Expr, after: &Expr) -> bool {
    let copper = |v: &&Expr| matches!(v.head(), Some("segment" | "via"));
    let old_fixed: Vec<_> = before.children().iter().filter(|v| !copper(v)).collect();
    let new_fixed: Vec<_> = after.children().iter().filter(|v| !copper(v)).collect();
    if old_fixed != new_fixed {
        return false;
    }
    let mut old = BTreeMap::new();
    let mut new = BTreeMap::new();
    for (board, items) in [(before, &mut old), (after, &mut new)] {
        for item in board.children().iter().filter(copper) {
            let Some(id) = item
                .child("uuid")
                .and_then(|u| u.children().get(1))
                .and_then(Expr::atom)
            else {
                return false;
            };
            if items.insert(id, item).is_some() {
                return false;
            }
        }
    }
    old.iter().all(|(id, item)| new.get(id) == Some(item))
}

pub fn bridge_kicad_open_connections(
    source_directory: &Path,
    board_id: &str,
    config: &KiCadIslandBridgeConfig,
    output: &Path,
) -> Result<KiCadIslandBridgeResult, String> {
    if config.maximum_attempts == 0 || config.maximum_pair_candidates == 0 {
        return Err("island bridges require positive attempt and pair bounds".into());
    }
    validate_grid_route_config(&config.routing)?;
    if !matches!(
        Path::new(board_id)
            .components()
            .collect::<Vec<_>>()
            .as_slice(),
        [std::path::Component::Normal(_)]
    ) {
        return Err("island bridge board-id must be a filename stem".into());
    }
    let source = fs::canonicalize(source_directory).map_err(|e| e.to_string())?;
    let absolute = env::current_dir().map_err(|e| e.to_string())?.join(output);
    let parent = fs::canonicalize(absolute.parent().ok_or("output requires parent")?)
        .map_err(|e| e.to_string())?;
    let output = parent.join(absolute.file_name().ok_or("output requires name")?);
    if output.exists() || output.starts_with(&source) {
        return Err("island bridge output must be new and outside source".into());
    }
    let board_name = format!("{board_id}.kicad_pcb");
    let source_board = source.join(&board_name);
    let source_hash = file_sha256(&source_board)?;
    let source_design_inputs = design_inputs(&source, &board_name)?;
    let source_pcb = parse(&fs::read_to_string(&source_board).map_err(|e| e.to_string())?)?;
    fs::create_dir(&output).map_err(|e| e.to_string())?;
    write_typed_json(&output.join("config.json"), config)?;
    let baseline = output.join("baseline");
    copy_directory_tree(&source, &baseline)?;
    let cache = output.join("verification-cache");
    let native = verify_materialized_rung_with_cache(&baseline, board_id, &cache)?;
    if file_sha256(&baseline.join(&board_name))? != source_hash
        || design_inputs(&baseline, &board_name)? != source_design_inputs
    {
        return Err("baseline verification changed the source design copy".into());
    }
    let mut result = KiCadIslandBridgeResult {
        schema_version: 1,
        status: "running".into(),
        reason: None,
        source_board_sha256: source_hash.clone(),
        source_unchanged: true,
        selected_directory: baseline,
        selected_native: native,
        attempts: Vec::new(),
        admitted_bridges: 0,
        contract: "Native island discovery and deterministic existing-pad pairs; native-compiled physical rules; bounded append-only proposals; strict native open reduction and exact original copper/design preservation before atomic selection. Unsupported and unattempted islands remain incomplete. No automatic ripup, placement or source mutation.",
    };
    publish(&output, &result)?;
    let work = (|| -> Result<(), String> {
        if let Some(reason) = route_obstructions::unsupported_board_geometry(&source_pcb) {
            result.status = "unsupported".into();
            result.reason = Some(reason);
            return Ok(());
        }
        if !physical_clean(&result.selected_native) {
            result.status = "rejected".into();
            result.reason = Some(
                "source native DRC/ERC/parity preflight failed (including annotations)".into(),
            );
            return Ok(());
        }
        if result.selected_native.complete {
            result.status = "complete".into();
            return Ok(());
        }
        // The private baseline has just passed full native verification and
        // exact source-design checks. Reuse only its ERC result; every bridge
        // candidate still checks this input hash and runs native PCB DRC,
        // schematic parity, and preview capture before selection.
        let baseline_erc_input_sha256 = kicad_erc_input_sha256(&result.selected_directory)?;
        let baseline_erc = read_json(&result.selected_directory.join("erc.json"))?;
        let tooling = output.join("tooling");
        fs::create_dir(&tooling).map_err(|e| e.to_string())?;
        let inspector = tooling.join("inspect_native_islands.py");
        fs::write(
            &inspector,
            include_str!("../../../experiments/whole-board/inspect_native_islands.py"),
        )
        .map_err(|e| e.to_string())?;
        let compiler = tooling.join("compile_net_classes.py");
        fs::write(
            &compiler,
            include_str!("../../../experiments/whole-board/compile_net_classes.py"),
        )
        .map_err(|e| e.to_string())?;
        let seed = output.join("routing-seed.json");
        write_typed_json(
            &seed,
            &serde_json::json!({"routing_portfolio": [config.routing.clone()]}),
        )?;
        let compiled = output.join("native-rules");
        run_python(
            &compiler,
            &[
                &result.selected_directory.join(&board_name),
                &seed,
                &compiled,
            ],
            &output.join("compile-rules.log"),
        )?;
        let evidence = read_json(&compiled.join("net-class-evidence.json"))?;
        let mut routing: KiCadGridRouteConfig = serde_json::from_value(
            read_json(&compiled.join("sequential-config.json"))?["routing_portfolio"][0].clone(),
        )
        .map_err(|e| e.to_string())?;
        routing.edge_clearance_mm = evidence["global_rules"]["edge_clearance_mm"]
            .as_f64()
            .ok_or("missing native edge rule")?;
        routing.hole_to_hole_clearance_mm = evidence["global_rules"]["hole_to_hole_clearance_mm"]
            .as_f64()
            .ok_or("missing native drill rule")?;
        validate_grid_route_config(&routing)?;
        write_typed_json(&output.join("resolved-routing.json"), &routing)?;
        let mut considered = BTreeSet::new();
        let mut inspected_hash = None;
        let mut pending = VecDeque::new();
        while result.attempts.len() < config.maximum_attempts && !result.selected_native.complete {
            let index = result.attempts.len();
            let selected_board = result.selected_directory.join(&board_name);
            let hash = file_sha256(&selected_board)?;
            if inspected_hash.as_ref() != Some(&hash) {
                let inspection = output.join(format!("islands-{index:03}.json"));
                let bound = config.maximum_pair_candidates.to_string();
                run_python(
                    &inspector,
                    &[
                        &selected_board,
                        &inspection,
                        Path::new("--maximum-pairs"),
                        Path::new(&bound),
                    ],
                    &output.join(format!("islands-{index:03}.log")),
                )?;
                let islands: Islands =
                    serde_json::from_value(read_json(&inspection)?).map_err(|e| e.to_string())?;
                if islands.schema_version != 1
                    || !islands.source_unchanged
                    || islands.board_sha256 != hash
                    || file_sha256(&selected_board)? != hash
                {
                    return Err("native island inspection did not preserve/bind its source".into());
                }
                if !islands.discovery_complete
                    || islands.proposals.len() > config.maximum_pair_candidates
                {
                    return Err("native island inventory unsupported or exceeded pair bound; see inspection report".into());
                }
                pending = islands.proposals.into_iter().collect();
                inspected_hash = Some(hash.clone());
            }
            let pair = std::iter::from_fn(|| pending.pop_front()).find(|p| {
                considered.insert(format!("{hash}:{}", serde_json::to_string(p).unwrap()))
            });
            let Some(pair) = pair else {
                break;
            };
            let proposal_dir = output.join(format!("proposal-{index:03}"));
            let request = KiCadPadPairRoutingConfig {
                connection: pair.connection.clone(),
                start: pair.start.clone(),
                finish: pair.finish.clone(),
                routing: routing.clone(),
            };
            let proposal = propose_kicad_pad_pair_route(&selected_board, &request, &proposal_dir)?;
            let mut attempt = serde_json::json!({"pair":pair, "source_board_sha256":hash, "proposal_directory":proposal_dir, "route_found":proposal.route_found, "error":proposal.error, "admitted":false, "preserved":null});
            if proposal.route_found && proposal.source_unchanged {
                let candidate = output.join(format!("candidate-{index:03}"));
                copy_directory_tree(&result.selected_directory, &candidate)?;
                fs::copy(
                    proposal_dir.join("preview.kicad_pcb"),
                    candidate.join(&board_name),
                )
                .map_err(|e| e.to_string())?;
                let candidate_native = verify_materialized_route_only(
                    &candidate,
                    board_id,
                    &baseline_erc_input_sha256,
                    &baseline_erc,
                )?;
                let candidate_pcb = parse(
                    &fs::read_to_string(candidate.join(&board_name)).map_err(|e| e.to_string())?,
                )?;
                let prior_pcb =
                    parse(&fs::read_to_string(&selected_board).map_err(|e| e.to_string())?)?;
                let preserved = append_only(&prior_pcb, &candidate_pcb)
                    && append_only(&source_pcb, &candidate_pcb)
                    && design_inputs(&candidate, &board_name)? == source_design_inputs
                    && file_sha256(&selected_board)? == hash
                    && file_sha256(&source_board)? == source_hash
                    && design_inputs(&source, &board_name)? == source_design_inputs;
                let accepted = admission(&result.selected_native, &candidate_native, preserved);
                attempt["preserved"] = preserved.into();
                attempt["native"] =
                    serde_json::to_value(&candidate_native).map_err(|e| e.to_string())?;
                attempt["candidate_directory"] = serde_json::json!(candidate);
                attempt["admitted"] = accepted.into();
                if accepted {
                    result.selected_directory = candidate;
                    result.selected_native = candidate_native;
                    result.admitted_bridges += 1;
                }
            }
            result.attempts.push(attempt);
            publish(&output, &result)?;
        }
        result.status = if result.selected_native.complete {
            "complete"
        } else {
            "incomplete"
        }
        .into();
        Ok(())
    })();
    if let Err(error) = work {
        result.status = "error".into();
        result.reason = Some(error);
    }
    result.source_unchanged = file_sha256(&source_board)? == source_hash
        && design_inputs(&source, &board_name)? == source_design_inputs;
    if !result.source_unchanged {
        result.status = "error".into();
        result.reason = Some("source changed during bridge operation".into());
    }
    publish(&output, &result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn append_only_rejects_original_mutation_duplicate_and_removal() {
        let a = parse("(kicad_pcb (footprint F (at 1 2)) (segment (start 1 2) (uuid a)))").unwrap();
        let good = parse("(kicad_pcb (footprint F (at 1 2)) (segment (start 1 2) (uuid a)) (via (at 2 3) (uuid b)))").unwrap();
        assert!(append_only(&a, &good));
        for text in [
            "(kicad_pcb (footprint F (at 1 3)) (segment (start 1 2) (uuid a)))",
            "(kicad_pcb (footprint F (at 1 2)))",
            "(kicad_pcb (footprint F (at 1 2)) (segment (start 1 3) (uuid a)))",
            "(kicad_pcb (footprint F (at 1 2)) (segment (start 1 2) (uuid a)) (via (uuid a)))",
        ] {
            assert!(!append_only(&a, &parse(text).unwrap()));
        }
    }
    #[test]
    fn admission_requires_preserved_design_and_strict_native_progress() {
        let before = VerificationReport {
            board_id: "test".into(),
            complete: false,
            erc_violations: 0,
            drc_design_violations: 0,
            schematic_parity_issues: 0,
            selected_net_unconnected_items: 2,
            intentional_no_connect_groups: 0,
            library_metadata_warnings: 0,
        };
        let mut after = before.clone();
        assert!(!admission(&before, &after, true));
        after.selected_net_unconnected_items = 1;
        assert!(admission(&before, &after, true));
        assert!(!admission(&before, &after, false));
        after.drc_design_violations = 1;
        assert!(!admission(&before, &after, true));
    }
}

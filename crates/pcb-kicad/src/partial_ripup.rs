// Copyright (C) 2026 Toit contributors.

//! Admission for inserting an unrouted target while restoring one earlier net.
use super::*;

fn copper_on(node: &Expr, connections: &[&str], names: &BTreeMap<String, String>) -> bool {
    matches!(node.head(), Some("segment" | "arc" | "via" | "zone"))
        && node_net(node).is_some_and(|net| {
            let name = names.get(net).map_or(net, String::as_str);
            connections
                .iter()
                .any(|c| normalize_net(c) == normalize_net(name))
        })
}

pub(super) fn net_names(pcb: &Expr) -> BTreeMap<String, String> {
    pcb.children()
        .iter()
        .filter(|n| n.head() == Some("net") && n.children().len() >= 3)
        .filter_map(|n| {
            Some((
                n.children()[1].atom()?.into(),
                n.children()[2].atom()?.into(),
            ))
        })
        .collect()
}

pub(super) fn expected_opens(
    board: &Path,
    target: &str,
    baseline: &VerificationReport,
) -> Result<usize, String> {
    let pcb = parse(&fs::read_to_string(board).map_err(|e| e.to_string())?)?;
    let names = net_names(&pcb);
    if pcb
        .children()
        .iter()
        .any(|n| copper_on(n, &[target], &names))
    {
        return Err("partial rip-up requires an unrouted target with no existing copper".into());
    }
    let summary = inspect_kicad_routable_connections(board)?
        .into_iter()
        .find(|s| normalize_net(&s.connection) == normalize_net(target))
        .ok_or_else(|| {
            "partial rip-up target is not a discovered routable connection".to_string()
        })?;
    baseline
        .selected_net_unconnected_items
        .checked_sub(summary.electrical_terminal_count() - 1)
        .ok_or_else(|| "partial rip-up target connectivity accounting underflowed".into())
}

fn unchanged_board(mut pcb: Expr, changed: &[&str]) -> Expr {
    let names = net_names(&pcb);
    if let Expr::List(items) = &mut pcb {
        items.retain(|n| !copper_on(n, changed, &names));
    }
    pcb
}

pub(super) fn preserved(
    source: &Path,
    current: &Path,
    board_id: &str,
    target: &str,
    yielding: &str,
) -> Result<bool, String> {
    preserved_connections(source, current, board_id, &[target, yielding])
}

pub(super) fn preserved_connections(
    source: &Path,
    current: &Path,
    board_id: &str,
    changed: &[&str],
) -> Result<bool, String> {
    let board = format!("{board_id}.kicad_pcb");
    let read = |directory: &Path| {
        parse(&fs::read_to_string(directory.join(&board)).map_err(|e| e.to_string())?)
    };
    Ok(
        unchanged_board(read(source)?, changed) == unchanged_board(read(current)?, changed)
            && fs::read(source.join(format!("{board_id}.kicad_pro"))).map_err(|e| e.to_string())?
                == fs::read(current.join(format!("{board_id}.kicad_pro")))
                    .map_err(|e| e.to_string())?
            && kicad_erc_input_sha256(source)? == kicad_erc_input_sha256(current)?,
    )
}

pub(super) fn final_admissible(
    native: &VerificationReport,
    drc: &serde_json::Value,
    baseline: &serde_json::Value,
    expected_opens: usize,
    allow_annotations: bool,
) -> Result<bool, String> {
    let opens = |report: &serde_json::Value| -> Result<Vec<serde_json::Value>, String> {
        report["unconnected_items"]
            .as_array()
            .cloned()
            .ok_or_else(|| "missing native unconnected items".into())
    };
    let before = opens(baseline)?;
    let after = opens(drc)?;
    Ok(
        adaptive_routing::native_progress_admissible(native, drc, baseline, allow_annotations)?
            && native.selected_net_unconnected_items == expected_opens
            && after.len() == expected_opens
            && adaptive_routing::unchanged_finding_subset(
                before.iter().collect(),
                after.iter().collect(),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn partial_repair_cannot_trade_an_old_open_for_a_new_disconnection() {
        let finding = |id| {
            json!({"type":"unconnected_items","severity":"error","description":"unconnected",
            "items":[{"uuid":id,"description":"pad","pos":{"x":1.0,"y":2.0}}]})
        };
        let baseline =
            json!({"violations":[],"unconnected_items":[finding("target"),finding("other")]});
        let mut native = VerificationReport {
            board_id: "test".into(),
            complete: false,
            erc_violations: 0,
            drc_design_violations: 0,
            schematic_parity_issues: 0,
            selected_net_unconnected_items: 1,
            intentional_no_connect_groups: 0,
            library_metadata_warnings: 0,
        };
        let good = json!({"violations":[],"unconnected_items":[finding("other")]});
        let lost = json!({"violations":[],"unconnected_items":[finding("newly-broken-net")]});
        assert!(final_admissible(&native, &good, &baseline, 1, false).unwrap());
        assert!(!final_admissible(&native, &lost, &baseline, 1, false).unwrap());
        assert!(!final_admissible(&native, &good, &baseline, 0, false).unwrap());
        native.drc_design_violations = 1;
        assert!(!final_admissible(&native, &good, &baseline, 1, true).unwrap());
    }

    #[test]
    fn partial_repair_preserves_every_other_copper_object_and_fixed_design() {
        let source = parse(
            r#"(kicad_pcb (net 1 "target") (net 2 "other") (footprint (at 1 2))
            (segment (net 1) (start 0 0)) (segment (net 2) (start 3 4)))"#,
        )
        .unwrap();
        let changed_target = parse(
            r#"(kicad_pcb (net 1 "target") (net 2 "other") (footprint (at 1 2))
            (segment (net 1) (start 9 9)) (segment (net 2) (start 3 4)))"#,
        )
        .unwrap();
        let changed_other = parse(
            r#"(kicad_pcb (net 1 "target") (net 2 "other") (footprint (at 1 2))
            (segment (net 1) (start 0 0)) (segment (net 2) (start 9 9)))"#,
        )
        .unwrap();
        let moved = parse(
            r#"(kicad_pcb (net 1 "target") (net 2 "other") (footprint (at 9 9))
            (segment (net 1) (start 0 0)) (segment (net 2) (start 3 4)))"#,
        )
        .unwrap();
        let expected = unchanged_board(source, &["target"]);
        assert_eq!(expected, unchanged_board(changed_target, &["target"]));
        assert_ne!(expected, unchanged_board(changed_other, &["target"]));
        assert_ne!(expected, unchanged_board(moved, &["target"]));
    }
}

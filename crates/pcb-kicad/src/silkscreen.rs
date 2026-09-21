// Copyright (C) 2026 Toit contributors.

use super::*;

fn editable(node: &Expr) -> bool {
    matches!(
        node.head(),
        Some("fp_line" | "fp_circle" | "fp_arc" | "fp_poly" | "fp_rect" | "fp_text" | "property")
    ) && form_atom(node, "layer", 1) == Some("F.SilkS")
}

fn protected_board(mut board: Expr) -> Expr {
    if let Expr::List(items) = &mut board {
        for footprint in items
            .iter_mut()
            .filter(|item| item.head() == Some("footprint"))
        {
            if let Expr::List(children) = footprint {
                children.retain(|child| !editable(child));
            }
        }
    }
    board
}

fn labels(board: &Expr) -> BTreeMap<String, Expr> {
    let mut result = BTreeMap::new();
    for footprint in board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        for field in footprint
            .children()
            .iter()
            .filter(|item| editable(item) && matches!(item.head(), Some("property" | "fp_text")))
        {
            let Some(identity) = form_atom(field, "uuid", 1) else {
                continue;
            };
            let Expr::List(items) = field else {
                unreachable!()
            };
            result.insert(
                identity.to_string(),
                Expr::List(
                    items
                        .iter()
                        .filter(|item| !matches!(item.head(), Some("at" | "render_cache")))
                        .cloned()
                        .collect(),
                ),
            );
        }
    }
    result
}

fn marks_preserved(before: &Expr, after: &Expr, report: &serde_json::Value) -> bool {
    let strokes = |board: &Expr| {
        board
            .children()
            .iter()
            .filter(|fp| fp.head() == Some("footprint"))
            .flat_map(|fp| fp.children())
            .filter(|s| editable(s) && s.head() == Some("fp_line"))
            .filter_map(|s| form_atom(s, "uuid", 1).map(|id| (id.to_string(), s.clone())))
            .collect::<BTreeMap<_, _>>()
    };
    let (before, after) = (strokes(before), strokes(after));
    let Some(groups) = report["protected_marks"].as_array() else {
        return false;
    };
    for group in groups {
        let Some(ids) = group["uuids"].as_array() else {
            return false;
        };
        if ids.len() != 2 {
            return false;
        }
        let mut translation: Option<[f64; 2]> = None;
        for id in ids {
            let Some(id) = id.as_str() else {
                return false;
            };
            let (Some(a), Some(b)) = (before.get(id), after.get(id)) else {
                return false;
            };
            let style = |s: &Expr| {
                s.children()
                    .iter()
                    .filter(|n| !matches!(n.head(), Some("start" | "end")))
                    .cloned()
                    .collect::<Vec<_>>()
            };
            if style(a) != style(b) {
                return false;
            }
            for endpoint in ["start", "end"] {
                let (Ok(a), Ok(b)) = (form_xy(a, endpoint), form_xy(b, endpoint)) else {
                    return false;
                };
                let delta = [b[0] - a[0], b[1] - a[1]];
                if delta.iter().any(|v| !v.is_finite()) {
                    return false;
                }
                if let Some(expected) = translation {
                    if (0..2).any(|i| (delta[i] - expected[i]).abs() > 0.000002) {
                        return false;
                    }
                } else {
                    translation = Some(delta);
                }
            }
        }
    }
    true
}

/// Repair annotations in a fresh project copy. Copper, pads, placement, rules,
/// and all non-front-silk records are protected by independent structural
/// comparison. Open connections are allowed: this can run before routing.
pub fn repair_kicad_silkscreen(
    source: &Path,
    board_id: &str,
    output: &Path,
) -> Result<serde_json::Value, String> {
    if board_id.is_empty()
        || board_id == "."
        || board_id == ".."
        || !board_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
    {
        return Err("invalid silkscreen board ID".into());
    }
    let source = source
        .canonicalize()
        .map_err(|error| format!("source project: {error}"))?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent
        .canonicalize()
        .map_err(|error| format!("output parent: {error}"))?;
    let output = parent.join(
        output
            .file_name()
            .ok_or("output must name a fresh directory")?,
    );
    if output.starts_with(&source) {
        return Err("silkscreen output must be outside the source project".into());
    }
    let source_board = source.join(format!("{board_id}.kicad_pcb"));
    let before_text = fs::read_to_string(&source_board).map_err(|e| e.to_string())?;
    let before = parse(&before_text)?;
    let project = source.join(format!("{board_id}.kicad_pro"));
    let project_bytes = if project.exists() {
        Some(fs::read(&project).map_err(|e| e.to_string())?)
    } else {
        None
    };
    copy_directory_tree(&source, &output)?;
    let result_board = output.join(format!("{board_id}.kicad_pcb"));
    let proposal = output.join("silkscreen-proposal.json");
    remove_if_present(&proposal)?;
    let process = Command::new("python3")
        .arg("-c")
        .arg(include_str!("silkscreen.py"))
        .arg(&source_board)
        .arg(&result_board)
        .arg(&proposal)
        .output()
        .map_err(|e| e.to_string())?;
    fs::write(output.join("silkscreen.stdout.log"), &process.stdout).map_err(|e| e.to_string())?;
    fs::write(output.join("silkscreen.stderr.log"), &process.stderr).map_err(|e| e.to_string())?;
    // Always verify and render the retained result, including worker failures.
    let verification = verify_materialized_rung(&output, board_id)?;
    let after = parse(&fs::read_to_string(&result_board).map_err(|e| e.to_string())?)?;
    let protected_unchanged = protected_board(before.clone()) == protected_board(after.clone());
    let labels_unchanged = labels(&before) == labels(&after);
    let result_project = output.join(format!("{board_id}.kicad_pro"));
    let project_unchanged = match project_bytes {
        Some(bytes) => fs::read(&result_project).map_err(|e| e.to_string())? == bytes,
        None => !result_project.exists(),
    };
    let source_unchanged =
        fs::read_to_string(&source_board).map_err(|e| e.to_string())? == before_text;
    let drc = read_json(&output.join("drc.json"))?;
    let remaining = drc["violations"]
        .as_array()
        .ok_or("native DRC omitted violations")?
        .iter()
        .filter(|v| {
            v["type"].as_str().is_some_and(|kind| {
                kind.starts_with("silk_") || matches!(kind, "text_height" | "text_thickness")
            })
        })
        .count();
    let proposal_report = if proposal.exists() {
        Some(read_json(&proposal)?)
    } else {
        None
    };
    let reference_policy_complete = proposal_report
        .as_ref()
        .and_then(|report| report["unresolved_references"].as_array())
        .is_some_and(|items| items.is_empty());
    let marks_unchanged = proposal_report
        .as_ref()
        .is_some_and(|report| marks_preserved(&before, &after, report));
    let complete = process.status.success()
        && reference_policy_complete
        && marks_unchanged
        && proposal_report
            .as_ref()
            .and_then(|report| report["unresolved_marks"].as_array())
            .is_some_and(|items| items.is_empty())
        && protected_unchanged
        && labels_unchanged
        && project_unchanged
        && source_unchanged
        && remaining == 0;
    let report = serde_json::json!({
        "schema_version": 1, "source": source_board, "result": result_board,
        "worker_exit_code": process.status.code(), "protected_non_silk_unchanged": protected_unchanged,
        "label_text_style_visibility_unchanged": labels_unchanged,
        "recognized_mark_shape_style_identity_preserved": marks_unchanged,
        "project_unchanged": project_unchanged, "source_unchanged": source_unchanged,
        "remaining_annotation_findings": remaining, "silkscreen_complete": complete,
        "native": verification, "reference_policy_complete": reference_policy_complete, "proposal": proposal_report,
        "scope": "Front annotation repair only. Full native completion is reported separately; open cold boards are allowed."
    });
    write_typed_json(&output.join("silkscreen-repair.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_guard_rejects_shortening_and_separate_arm_motion() {
        let source = parse(r#"(kicad_pcb (footprint
            (fp_line (start -0.5 0) (end 0.5 0) (stroke (width 0.12)) (layer "F.SilkS") (uuid "a"))
            (fp_line (start 0 -0.5) (end 0 0.5) (stroke (width 0.12)) (layer "F.SilkS") (uuid "b"))))"#).unwrap();
        let report = serde_json::json!({"protected_marks": [{"uuids": ["a", "b"]}]});
        assert!(marks_preserved(&source, &source, &report));
        let moved_text = encode(&source)
            .replace("(start -0.5 0)", "(start 0.5 2)")
            .replace("(end 0.5 0)", "(end 1.5 2)")
            .replace("(start 0 -0.5)", "(start 1 1.5)")
            .replace("(end 0 0.5)", "(end 1 2.5)");
        let moved = parse(&moved_text).unwrap();
        assert!(marks_preserved(&source, &moved, &report));
        for changed in [
            moved_text.replace("(end 1 2.5)", "(end 1 2.1)"),
            moved_text
                .replace("(start 1 1.5)", "(start 2 1.5)")
                .replace("(end 1 2.5)", "(end 2 2.5)"),
            moved_text.replace("(width 0.12)", "(width 0.1)"),
        ] {
            assert!(!marks_preserved(
                &source,
                &parse(&changed).unwrap(),
                &report
            ));
        }
    }

    #[test]
    fn annotation_guard_protects_copper_placement_and_label_content() {
        let source = parse(r#"(kicad_pcb (footprint (at 2 3) (pad "1" thru_hole circle (at 1 0))
            (property "Reference" "R1" (at 0 0) (layer "F.SilkS") (uuid "label") (effects (font (size 1 1))))
            (fp_line (start 0 0) (end 1 1) (layer "F.SilkS"))) (segment (start 2 3) (end 5 6)))"#).unwrap();
        let moved = parse(&encode(&source).replace("(at 0 0)", "(at 2 2)")).unwrap();
        assert_eq!(
            protected_board(source.clone()),
            protected_board(moved.clone())
        );
        assert_eq!(labels(&source), labels(&moved));
        for (from, to) in [
            ("(at 2 3)", "(at 2 4)"),
            ("(end 5 6)", "(end 5 7)"),
            ("(at 1 0)", "(at 1 1)"),
        ] {
            let changed = parse(&encode(&source).replace(from, to)).unwrap();
            assert_ne!(protected_board(source.clone()), protected_board(changed));
        }
        let renamed = parse(&encode(&source).replace("\"R1\"", "\"R2\"")).unwrap();
        assert_ne!(labels(&source), labels(&renamed));
        let hidden = parse(&encode(&source).replace("(effects", "(hide yes) (effects")).unwrap();
        assert_ne!(labels(&source), labels(&hidden));
    }
}

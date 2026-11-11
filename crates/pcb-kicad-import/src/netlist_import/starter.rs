// Copyright (C) 2026 Toit contributors.

//! The starter constraints the import writes beside the board: the plug-in
//! connectors on an edge, the key matrix, the decoupling `near` entries.

use super::*;

/// Starter constraints: every part placed, the outline sized from the
/// parts, the plug-in connectors on an edge (facing out when the
/// footprint shows which way it opens). Returns the `edge` entries and the
/// report's lines.
pub(super) fn edge_entries(board: &Expr) -> Result<(Vec<serde_json::Value>, Vec<String>), String> {
    let mut edges = Vec::new();
    let mut edge_connectors = Vec::new();
    for footprint in board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let name = footprint
            .children()
            .get(1)
            .and_then(Expr::atom)
            .unwrap_or("");
        let reference = footprint
            .children()
            .iter()
            .find(|item| {
                item.head() == Some("property")
                    && item.children().get(1).and_then(Expr::atom) == Some("Reference")
            })
            .and_then(|item| item.children().get(2).and_then(Expr::atom))
            .unwrap_or("");
        let wired = footprint
            .children()
            .iter()
            .any(|item| item.head() == Some("pad") && item.child("net").is_some());
        if reference.is_empty() || !wired || !edge_connector(name) {
            continue;
        }
        let mut edge = serde_json::json!({"part": reference, "edge": "any", "flush": true});
        if crate::footprint_bodies::connector_mouth(footprint)?.is_some() {
            edge["opening_outwards"] = true.into();
        }
        edges.push(edge);
        edge_connectors.push(format!("{reference} ({name})"));
    }
    Ok((edges, edge_connectors))
}

/// A key matrix: the starter constraints for it, beside the agent's
/// file (rewritten on every import; constraints.json never is). `None`
/// where no part looks like a key switch.
pub(super) fn key_matrix_starter(
    board: &Expr,
    matrix_parts: &[crate::key_matrix::MatrixPart],
    starter: &serde_json::Value,
    missing_parts: usize,
    output: &Path,
) -> Result<Option<crate::key_matrix::KiCadStarterReport>, String> {
    let Some(matrix) = crate::key_matrix::infer_key_matrix(matrix_parts) else {
        return Ok(None);
    };
    let mut bodies = BTreeMap::new();
    for footprint in board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let Some(reference) = footprint_reference(footprint) else {
            continue;
        };
        let id = footprint
            .children()
            .get(1)
            .and_then(Expr::atom)
            .unwrap_or("")
            .to_string();
        let size =
            crate::board_footprints::body_size(footprint).map_or([0.0, 0.0], |(size, _)| size);
        bodies.insert(reference, (id, size));
    }
    let (keys, mut report) =
        crate::key_matrix::starter_constraints(&matrix, &bodies, starter.clone());
    if matrix.groups.is_empty() || report.switches == 0 {
        report.why = Some(no_matrix_why(matrix.unassigned.len(), missing_parts));
    } else {
        let path = output.join("constraints.starter.json");
        fs::write(&path, crate::key_matrix::constraints_text(&keys))
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        report.file = Some(path);
    }
    Ok(Some(report))
}

/// With `--near-decoupling`: the `near` entries of the template for the
/// board written so far go into `starter`; returns their number.
pub(super) fn add_near_entries(
    output: &Path,
    board_id: &str,
    starter: &mut serde_json::Value,
) -> Result<usize, String> {
    let template = crate::constraints_template::constraints_template(output, board_id)?;
    let near = template["near"].as_array().cloned().unwrap_or_default();
    let count = near.len();
    if !near.is_empty() {
        starter["near"] = near.into();
        let comments: Vec<serde_json::Value> = template["comment"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|line| line.as_str().is_some_and(|line| line.starts_with("near:")))
            .cloned()
            .collect();
        starter["comment"] = comments.into();
    }
    Ok(count)
}

/// Writes the starter files that are new and notes the ones kept.
pub(super) struct StarterFiles<'a> {
    output: &'a Path,
    /// The files that existed and were kept as they are.
    pub kept: Vec<String>,
}

impl<'a> StarterFiles<'a> {
    pub fn new(output: &'a Path) -> Self {
        StarterFiles {
            output,
            kept: Vec::new(),
        }
    }

    /// `<output>/<name>` with `value` as pretty JSON, never over an
    /// existing file (a second import keeps the agent's edits).
    pub fn write_new(
        &mut self,
        name: &str,
        value: serde_json::Value,
    ) -> Result<Option<PathBuf>, String> {
        let path = self.output.join(name);
        if path.exists() {
            self.kept.push(name.to_string());
            return Ok(Some(path));
        }
        fs::write(
            &path,
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())? + "\n",
        )
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        Ok(Some(path))
    }
}

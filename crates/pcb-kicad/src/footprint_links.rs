// Copyright (C) 2026 Toit contributors.

//! Schematic footprint links rewritten to the board's footprint libraries.

use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadFootprintLinkUpdate {
    #[serde(default)]
    pub schematic_path: String,
    pub reference: String,
    pub schematic_before: String,
    pub board_link: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadSchematicLinkNormalization {
    pub schematic_path: String,
    pub before_sha256: String,
    pub after_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadFootprintLinkNormalizationReport {
    pub schema_version: u32,
    pub board_id: String,
    pub updated_references: Vec<KiCadFootprintLinkUpdate>,
    pub schematic_before_sha256: String,
    pub schematic_after_sha256: String,
    pub board_sha256: String,
    pub copper_geometry_sha256: String,
    pub component_placement_sha256: String,
    #[serde(default)]
    pub schematic_files: Vec<KiCadSchematicLinkNormalization>,
}

/// Synchronizes footprint links throughout the referenced schematic hierarchy
/// to the embedded footprint links in an already materialized board. This is
/// intended for copied, pinned legacy projects whose library nicknames were
/// renamed after the board was laid out. It changes no board geometry,
/// placement, net, or copper decision.
pub fn normalize_kicad_footprint_links(
    directory: &Path,
    board_id: &str,
) -> Result<KiCadFootprintLinkNormalizationReport, String> {
    let board_path = directory.join(format!("{board_id}.kicad_pcb"));
    let schematic_path = directory.join(format!("{board_id}.kicad_sch"));
    let board_source = fs::read_to_string(&board_path)
        .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let board = parse(&board_source)
        .map_err(|error| format!("failed to parse {}: {error}", board_path.display()))?;
    let board_statistics = board_statistics(&board)?;
    let mut board_links = BTreeMap::new();
    for footprint in board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let Some(reference) = footprint_reference(footprint)
            .filter(|value| !value.is_empty() && !value.contains('*') && !value.starts_with('#'))
        else {
            continue;
        };
        let link = footprint
            .children()
            .get(1)
            .and_then(Expr::atom)
            .ok_or_else(|| format!("board footprint {reference:?} has no library link"))?
            .to_string();
        if let Some(previous) = board_links.insert(reference.clone(), link.clone())
            && previous != link
        {
            return Err(format!(
                "board has conflicting footprint links for reference {reference:?}"
            ));
        }
    }

    let project_root = fs::canonicalize(directory)
        .map_err(|error| format!("failed to resolve {}: {error}", directory.display()))?;
    let root_schematic = fs::canonicalize(&schematic_path)
        .map_err(|error| format!("failed to resolve {}: {error}", schematic_path.display()))?;
    let mut pending = VecDeque::from([root_schematic.clone()]);
    let mut visited = BTreeSet::new();
    let mut updates = BTreeMap::new();
    let mut schematic_files = Vec::new();
    while let Some(current_path) = pending.pop_front() {
        if !current_path.starts_with(&project_root) {
            return Err(format!(
                "referenced schematic {} escapes project {}",
                current_path.display(),
                project_root.display()
            ));
        }
        if !visited.insert(current_path.clone()) {
            continue;
        }
        let relative = current_path
            .strip_prefix(&project_root)
            .expect("project containment was checked")
            .to_string_lossy()
            .into_owned();
        let before_sha256 = file_sha256(&current_path)?;
        let schematic_source = fs::read_to_string(&current_path)
            .map_err(|error| format!("failed to read {}: {error}", current_path.display()))?;
        let mut schematic = parse(&schematic_source)
            .map_err(|error| format!("failed to parse {}: {error}", current_path.display()))?;
        let Expr::List(items) = &mut schematic else {
            return Err(format!("{} root is not a list", current_path.display()));
        };
        if root_head(items) != Some("kicad_sch") {
            return Err(format!(
                "{} is not a kicad_sch document",
                current_path.display()
            ));
        }
        for sheet_file in items
            .iter()
            .filter(|item| item.head() == Some("sheet"))
            .filter_map(|sheet| schematic_property(sheet, "Sheetfile"))
        {
            let child = Path::new(&sheet_file);
            if child.is_absolute() {
                return Err(format!(
                    "referenced schematic path {sheet_file:?} is absolute"
                ));
            }
            let child_path =
                fs::canonicalize(current_path.parent().unwrap().join(child)).map_err(|error| {
                    format!(
                        "failed to resolve child schematic {sheet_file:?} from {}: {error}",
                        current_path.display()
                    )
                })?;
            if !child_path.starts_with(&project_root) {
                return Err(format!(
                    "referenced schematic {} escapes project {}",
                    child_path.display(),
                    project_root.display()
                ));
            }
            pending.push_back(child_path);
        }
        let mut changed = false;
        for symbol in items
            .iter_mut()
            .filter(|item| item.head() == Some("symbol"))
        {
            let Some(reference) = schematic_property(symbol, "Reference") else {
                continue;
            };
            let Some(board_link) = board_links.get(&reference) else {
                continue;
            };
            let Some(before) = schematic_property(symbol, "Footprint") else {
                continue;
            };
            if before == *board_link {
                continue;
            }
            let Expr::List(symbol_items) = symbol else {
                unreachable!("symbol form must be a list")
            };
            let property = symbol_items
                .iter_mut()
                .find(|item| {
                    item.head() == Some("property")
                        && item.children().get(1).and_then(Expr::atom) == Some("Footprint")
                })
                .expect("Footprint property was read above");
            let Expr::List(property_items) = property else {
                unreachable!("property form must be a list")
            };
            property_items[2] = Expr::Atom(quote(board_link));
            changed = true;
            updates.insert(
                (relative.clone(), reference.clone()),
                KiCadFootprintLinkUpdate {
                    schematic_path: relative.clone(),
                    reference,
                    schematic_before: before,
                    board_link: board_link.clone(),
                },
            );
        }
        if changed {
            fs::write(&current_path, format!("{}\n", encode(&schematic))).map_err(|error| {
                format!(
                    "failed to write normalized {}: {error}",
                    current_path.display()
                )
            })?;
        }
        schematic_files.push(KiCadSchematicLinkNormalization {
            schematic_path: relative,
            before_sha256,
            after_sha256: file_sha256(&current_path)?,
        });
    }
    let root_audit = schematic_files
        .iter()
        .find(|audit| root_schematic == project_root.join(&audit.schematic_path))
        .expect("root schematic is always audited");
    Ok(KiCadFootprintLinkNormalizationReport {
        schema_version: 2,
        board_id: board_id.to_string(),
        updated_references: updates.into_values().collect(),
        schematic_before_sha256: root_audit.before_sha256.clone(),
        schematic_after_sha256: root_audit.after_sha256.clone(),
        board_sha256: file_sha256(&board_path)?,
        copper_geometry_sha256: board_statistics.copper_geometry_sha256,
        component_placement_sha256: board_statistics.component_placement_sha256,
        schematic_files,
    })
}

fn schematic_property(node: &Expr, name: &str) -> Option<String> {
    node.children().iter().find_map(|item| {
        (item.head() == Some("property") && item.children().get(1)?.atom()? == name)
            .then(|| item.children().get(2)?.atom().map(str::to_owned))
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footprint_link_normalization_changes_only_schematic_library_links() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-footprint-link-normalization-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        let schematic = directory.join("board.kicad_sch");
        fs::write(
            &board,
            r#"(kicad_pcb
                (footprint "Local:R" (at 4 5 90)
                  (property "Reference" "R1")
                  (pad "1" thru_hole circle (at 0 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask")))
                (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        fs::write(
            &schematic,
            r#"(kicad_sch
                (symbol (lib_id "Device:R") (at 1 2 0)
                  (property "Reference" "R1")
                  (property "Footprint" "Device:R")))"#,
        )
        .unwrap();
        let board_before = fs::read(&board).unwrap();

        let report = normalize_kicad_footprint_links(&directory, "board").unwrap();

        assert_eq!(report.updated_references.len(), 1);
        assert_eq!(report.schema_version, 2);
        assert_eq!(report.schematic_files.len(), 1);
        assert_eq!(
            report.updated_references[0].schematic_path,
            "board.kicad_sch"
        );
        assert_eq!(report.updated_references[0].reference, "R1");
        assert_eq!(report.updated_references[0].schematic_before, "Device:R");
        assert_eq!(report.updated_references[0].board_link, "Local:R");
        assert_ne!(
            report.schematic_before_sha256,
            report.schematic_after_sha256
        );
        assert_eq!(fs::read(&board).unwrap(), board_before);
        assert!(
            fs::read_to_string(&schematic)
                .unwrap()
                .contains("(property \"Footprint\" \"Local:R\")")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn footprint_link_normalization_traverses_referenced_schematic_hierarchy() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-hierarchical-footprint-link-normalization-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("board.kicad_pcb"),
            r#"(kicad_pcb
                (footprint "Local:R" (at 4 5) (property "Reference" "R1"))
                (footprint "Local:C" (at 6 5) (property "Reference" "C2"))
                (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        fs::write(
            directory.join("board.kicad_sch"),
            r#"(kicad_sch
                (symbol (property "Reference" "R1") (property "Footprint" "Device:R"))
                (sheet (property "Sheetfile" "child.kicad_sch")))"#,
        )
        .unwrap();
        fs::write(
            directory.join("child.kicad_sch"),
            r#"(kicad_sch
                (symbol (property "Reference" "C2") (property "Footprint" "Device:C")))"#,
        )
        .unwrap();
        let board_before = fs::read(directory.join("board.kicad_pcb")).unwrap();

        let report = normalize_kicad_footprint_links(&directory, "board").unwrap();

        assert_eq!(report.updated_references.len(), 2);
        assert_eq!(report.schematic_files.len(), 2);
        assert_eq!(
            fs::read(directory.join("board.kicad_pcb")).unwrap(),
            board_before
        );
        assert!(
            fs::read_to_string(directory.join("child.kicad_sch"))
                .unwrap()
                .contains("(property \"Footprint\" \"Local:C\")")
        );
        fs::remove_dir_all(directory).unwrap();
    }
}

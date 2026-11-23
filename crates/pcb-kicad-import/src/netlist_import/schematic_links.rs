// Copyright (C) 2026 Florian Loitsch.

//! Keep the copied schematic's footprint IDs aligned with its board instances.

use super::*;

struct Link {
    reference: String,
    id: String,
    changed: bool,
}

type Links = BTreeMap<String, Vec<Link>>;
type Pending = BTreeSet<(String, String)>;

struct Sheet {
    document: Expr,
    source: String,
    atom_spans: Vec<std::ops::Range<usize>>,
    paths: Vec<String>,
}

impl Sheet {
    fn updated_text(&self) -> Result<String, String> {
        let mut remaining = vec![&self.document];
        let mut spans = self.atom_spans.iter();
        let mut text = String::with_capacity(self.source.len());
        let mut cursor = 0;
        while let Some(form) = remaining.pop() {
            match form {
                Expr::List(items) => remaining.extend(items.iter().rev()),
                Expr::Atom(atom) => {
                    let span = spans.next().ok_or("footprint link update added an atom")?;
                    if self.source[span.clone()] != *atom {
                        text.push_str(&self.source[cursor..span.start]);
                        text.push_str(atom);
                        cursor = span.end;
                    }
                }
            }
        }
        text.push_str(&self.source[cursor..]);
        if spans.next().is_some() || parse(&text)? != self.document {
            return Err("footprint link update changed the schematic structure".into());
        }
        Ok(text)
    }
}

/// Updates only the linked symbols in the copied project. KiCad 6 can
/// record different footprints for instances of one shared symbol. Modern
/// shared sheet symbols have one Footprint property for all instances.
pub(super) fn update(
    project: &Path,
    output: &Path,
    board_id: &str,
    components: &[Expr],
    board: &Expr,
) -> Result<Vec<String>, String> {
    let board_ids: BTreeMap<_, _> = board
        .children()
        .iter()
        .filter_map(|part| {
            let id = part.children().get(1)?.atom()?;
            Some((footprint_reference(part)?, id))
        })
        .collect();
    let mut links = Links::new();
    let mut pending = Pending::new();
    for component in components.iter().filter(|item| item.head() == Some("comp")) {
        let Some(reference) = text_of(component, "ref") else {
            continue;
        };
        let original = form_atom(component, "footprint", 1).unwrap_or_default();
        let id = board_ids.get(&reference).copied().unwrap_or(original);
        let changed = id != original;
        let sheet = component
            .child("sheetpath")
            .and_then(|sheet| form_atom(sheet, "tstamps", 1))
            .unwrap_or_default()
            .trim_end_matches('/');
        let stamps = form_atom(component, "tstamps", 1)
            .or_else(|| form_atom(component, "tstamp", 1))
            .unwrap_or_default();
        if changed && stamps.trim().is_empty() {
            pending.insert((reference.clone(), String::new()));
        }
        for stamp in stamps.split_whitespace().collect::<BTreeSet<_>>() {
            let path = format!("{sheet}/{stamp}");
            if changed {
                pending.insert((reference.clone(), path.clone()));
            }
            links.entry(path).or_default().push(Link {
                reference: reference.clone(),
                id: id.into(),
                changed,
            });
        }
    }
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let output = fs::canonicalize(output).map_err(|error| error.to_string())?;
    let in_place = fs::canonicalize(project).is_ok_and(|project| project == output);
    if !in_place {
        let root = output.join(format!("{board_id}.kicad_sch"));
        for (file, mut sheet) in read_sheets(&output, root)? {
            if update_sheet(&mut sheet, &links, &mut pending) {
                write_atomically(&file, sheet.updated_text()?.as_bytes())?;
            }
        }
    }
    Ok(pending
        .into_iter()
        .map(|(reference, path)| {
            let reason = if in_place {
                "the schematic is in the source directory"
            } else {
                "no independently editable footprint link for this symbol"
            };
            format!("{reference} ({path}): {reason}")
        })
        .collect())
}

fn read_sheets(output: &Path, root: PathBuf) -> Result<BTreeMap<PathBuf, Sheet>, String> {
    let mut sheets = BTreeMap::<PathBuf, Sheet>::new();
    let mut queue = vec![(root, String::new(), Vec::new())];
    while let Some((file, path, mut ancestors)) = queue.pop() {
        let Ok(file) = fs::canonicalize(file) else {
            continue;
        };
        if !file.starts_with(output) || ancestors.contains(&file) {
            continue;
        }
        let sheet = match sheets.entry(file.clone()) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let source = fs::read_to_string(&file)
                    .map_err(|error| format!("{}: {error}", file.display()))?;
                let (document, atom_spans) = parse_with_atom_spans(&source)
                    .map_err(|error| format!("{}: {error}", file.display()))?;
                entry.insert(Sheet {
                    document,
                    source,
                    atom_spans,
                    paths: Vec::new(),
                })
            }
        };
        ancestors.push(file.clone());
        for child in sheet
            .document
            .children()
            .iter()
            .filter(|item| item.head() == Some("sheet"))
        {
            let Some(stamp) = form_atom(child, "uuid", 1) else {
                continue;
            };
            let file_name = property(child, "Sheetfile").or_else(|| property(child, "Sheet file"));
            if let (Some(parent), Some(name)) = (file.parent(), file_name) {
                queue.push((
                    parent.join(name),
                    format!("{path}/{stamp}"),
                    ancestors.clone(),
                ));
            }
        }
        sheet.paths.push(path);
    }
    Ok(sheets)
}

/// Positions of uniquely located symbols in the board's schematic hierarchy.
pub(crate) fn schematic_positions(
    directory: &Path,
    board_id: &str,
) -> BTreeMap<String, (String, [f64; 2])> {
    let Ok(directory) = fs::canonicalize(directory) else {
        return BTreeMap::new();
    };
    let Ok(root) = fs::canonicalize(directory.join(format!("{board_id}.kicad_sch"))) else {
        return BTreeMap::new();
    };
    let Ok(sheets) = read_sheets(&directory, root.clone()) else {
        return BTreeMap::new();
    };
    let Some(root_sheet) = sheets.get(&root) else {
        return BTreeMap::new();
    };
    let root_id = form_atom(&root_sheet.document, "uuid", 1).map(str::to_owned);
    let mut positions = BTreeMap::new();
    for (file, sheet) in sheets {
        let instances: BTreeSet<_> = sheet
            .paths
            .iter()
            .filter_map(|path| root_id.as_deref().map(|root| format!("/{root}{path}")))
            .collect();
        for symbol in sheet
            .document
            .children()
            .iter()
            .filter(|item| item.head() == Some("symbol"))
        {
            let Ok(at) = form_xy(symbol, "at") else {
                continue;
            };
            let paths = symbol
                .child("instances")
                .into_iter()
                .flat_map(Expr::children)
                .filter(|project| project.head() == Some("project"))
                .flat_map(Expr::children)
                .filter(|path| path.head() == Some("path"));
            for path in paths {
                let (Some(instance), Some(reference)) = (
                    path.children().get(1).and_then(Expr::atom),
                    form_atom(path, "reference", 1),
                ) else {
                    continue;
                };
                if root_id.is_some() && !instances.contains(instance) {
                    continue;
                }
                let position = (format!("{}\n{instance}", file.display()), at);
                positions
                    .entry(reference.to_owned())
                    .and_modify(|seen: &mut Option<_>| {
                        if seen.as_ref() != Some(&position) {
                            *seen = None;
                        }
                    })
                    .or_insert(Some(position));
            }
        }
    }
    positions
        .into_iter()
        .filter_map(|(reference, position)| Some((reference, position?)))
        .collect()
}

fn property<'a>(form: &'a Expr, name: &str) -> Option<&'a str> {
    form.children().iter().find_map(|item| {
        let fields = item.children();
        if item.head() == Some("property") && fields.get(1)?.atom()? == name {
            return fields.get(2)?.atom();
        }
        None
    })
}

fn set_link(form: &mut Expr, head: &str, key: Option<&str>, id: &str) -> Option<bool> {
    let Expr::List(items) = form else { return None };
    let fields = items.iter_mut().find_map(|item| match item {
        Expr::List(fields)
            if fields.first()?.atom()? == head
                && key.is_none_or(|key| fields.get(1).and_then(Expr::atom) == Some(key)) =>
        {
            Some(fields)
        }
        _ => None,
    })?;
    let value = fields.get_mut(if key.is_some() { 2 } else { 1 })?;
    if value.atom()? == id {
        return Some(false);
    }
    *value = Expr::Atom(requote(id));
    Some(true)
}

fn update_sheet(sheet: &mut Sheet, links: &Links, pending: &mut Pending) -> bool {
    let Expr::List(items) = &mut sheet.document else {
        return false;
    };
    let mut changed = false;
    for item in items {
        match item.head() {
            Some("symbol") => changed |= update_symbol(item, &sheet.paths, links, pending),
            Some("symbol_instances") => {
                let Expr::List(instances) = item else {
                    continue;
                };
                for instance in instances
                    .iter_mut()
                    .filter(|item| item.head() == Some("path"))
                {
                    let Some(path) = instance
                        .children()
                        .get(1)
                        .and_then(Expr::atom)
                        .map(str::to_owned)
                    else {
                        continue;
                    };
                    let Some(reference) = form_atom(instance, "reference", 1) else {
                        continue;
                    };
                    let Some([link]) = links.get(&path).map(Vec::as_slice) else {
                        continue;
                    };
                    if link.changed
                        && link.reference == reference
                        && let Some(edited) = set_link(instance, "footprint", None, &link.id)
                    {
                        pending.remove(&(link.reference.clone(), path));
                        changed |= edited;
                    }
                }
            }
            _ => (),
        }
    }
    changed
}

fn update_symbol(
    symbol: &mut Expr,
    sheet_paths: &[String],
    links: &Links,
    pending: &mut Pending,
) -> bool {
    let Some(uuid) = form_atom(symbol, "uuid", 1) else {
        return false;
    };
    let Some(current) = property(symbol, "Footprint") else {
        return false;
    };
    let mut targets = BTreeSet::new();
    let mut matching = Vec::new();
    for sheet in sheet_paths {
        let path = format!("{sheet}/{uuid}");
        if let Some([link]) = links.get(&path).map(Vec::as_slice) {
            targets.insert(link.id.as_str());
            if link.changed {
                matching.push((link.reference.clone(), path));
            }
        } else {
            targets.insert(current);
        }
    }
    let Some(target) = targets
        .pop_first()
        .filter(|_| targets.is_empty() && !matching.is_empty())
    else {
        return false;
    };
    // End the borrow of the current property before editing this symbol.
    let target = target.to_owned();
    let Some(edited) = set_link(symbol, "property", Some("Footprint"), &target) else {
        return false;
    };
    for key in matching {
        pending.remove(&key);
    }
    edited
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(id: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pcb-maker-schematic-links-{id}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(dir.join("output")).unwrap();
        dir
    }

    fn board() -> Expr {
        parse(
            r#"(kicad_pcb (footprint "board:First" (property "Reference" "SW1"))
            (footprint "board:Second" (property "Reference" "SW2")))"#,
        )
        .unwrap()
    }

    #[test]
    fn local_and_legacy_instance_footprints_follow_their_own_symbol() {
        let project = fixture("legacy");
        let output = project.join("output");
        let path = output.join("test.kicad_sch");
        let source = r#"(kicad_sch
            (lib_symbols (symbol "lib:Switch" (property "Footprint" "lib:Original")))
            (symbol (uuid first) (property "Footprint"     "lib:Original"))
            (symbol (uuid second) (property "Footprint" "lib:Original"))
            (symbol (uuid other) (property "Footprint" "lib:Original"))
            (symbol_instances (path "/first" (reference "SW1") (footprint "lib:Original"))
              (path "/second" (reference "SW2") (footprint "lib:Original"))))"#;
        fs::write(&path, source).unwrap();
        let components = parse(
            r#"(components
            (comp (ref "SW1") (footprint "lib:Original") (tstamps "first"))
            (comp (ref "SW2") (footprint "lib:Original") (tstamps "second")))"#,
        )
        .unwrap();
        let conflicts = update(&project, &output, "test", components.children(), &board()).unwrap();
        assert!(conflicts.is_empty());
        let updated = fs::read_to_string(path).unwrap();
        assert_eq!(
            updated
                .replace("board:First", "lib:Original")
                .replace("board:Second", "lib:Original"),
            source
        );
        let document = parse(&updated).unwrap();
        let expected = [
            Some("board:First"),
            Some("board:Second"),
            Some("lib:Original"),
        ];
        let symbols = document
            .children()
            .iter()
            .filter(|item| item.head() == Some("symbol"));
        assert_eq!(
            symbols
                .map(|symbol| property(symbol, "Footprint"))
                .collect::<Vec<_>>(),
            expected
        );
        let library = document
            .child("lib_symbols")
            .unwrap()
            .child("symbol")
            .unwrap();
        assert_eq!(property(library, "Footprint"), Some("lib:Original"));
        let instances = &document.child("symbol_instances").unwrap().children()[1..];
        assert_eq!(
            form_atom(&instances[0], "footprint", 1),
            Some("board:First")
        );
        assert_eq!(
            form_atom(&instances[1], "footprint", 1),
            Some("board:Second")
        );
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn conflicting_modern_shared_sheet_links_are_reported_without_changing_the_template() {
        let project = fixture("shared");
        let output = project.join("output");
        fs::write(
            output.join("test.kicad_sch"),
            r#"(kicad_sch
            (sheet (uuid one) (property "Sheetfile" "shared.kicad_sch"))
            (sheet (uuid two) (property "Sheetfile" "shared.kicad_sch")))"#,
        )
        .unwrap();
        let shared = r#"(kicad_sch (symbol (uuid switch) (property "Footprint" "lib:Original")))"#;
        let path = output.join("shared.kicad_sch");
        fs::write(&path, shared).unwrap();
        let components = parse(r#"(components
            (comp (ref "SW1") (footprint "lib:Original") (sheetpath (tstamps "/one/")) (tstamps "switch"))
            (comp (ref "SW2") (footprint "lib:Original") (sheetpath (tstamps "/two/")) (tstamps "switch")))"#).unwrap();
        let conflicts = update(&project, &output, "test", components.children(), &board()).unwrap();
        assert_eq!(conflicts.len(), 2);
        assert!(conflicts[0].starts_with("SW1 (/one/switch)"));
        assert!(conflicts[1].starts_with("SW2 (/two/switch)"));
        assert_eq!(fs::read_to_string(path).unwrap(), shared);
        fs::remove_dir_all(project).unwrap();
    }
}

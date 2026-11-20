// Copyright (C) 2026 Florian Loitsch.

//! Legacy footprint table entries are monolithic `.mod` files. Replace a
//! copied entry only when neither project can resolve its original file.

use super::*;
use std::io::ErrorKind;

#[derive(Default)]
pub(super) struct LegacyLibraries {
    pub(super) protected: BTreeSet<String>,
    pub(super) replacements: BTreeMap<String, MissingLegacyLibrary>,
    pub(super) staged: BTreeMap<String, PathBuf>,
    pub(super) in_place: bool,
}

pub(super) struct MissingLegacyLibrary {
    entry: Expr,
    source_table: PathBuf,
    source_target: PathBuf,
    copied_target: PathBuf,
}

fn read_table(path: &Path) -> Option<Expr> {
    let table = parse(&fs::read_to_string(path).ok()?).ok()?;
    (table.head() == Some("fp_lib_table")).then_some(table)
}

fn entries<'a>(table: &'a Expr, nickname: &'a str) -> impl Iterator<Item = &'a Expr> {
    table.children().iter().filter(move |item| {
        item.head() == Some("lib") && form_atom(item, "name", 1) == Some(nickname)
    })
}

fn missing(path: &Path) -> bool {
    fs::symlink_metadata(path).is_err_and(|error| error.kind() == ErrorKind::NotFound)
}

fn target(entry: &Expr, project: &Path, system: &str) -> Option<PathBuf> {
    let path = expand(form_atom(entry, "uri", 1)?, project, system);
    let target = PathBuf::from(&path);
    (!path.contains('$') && target.is_absolute()).then_some(target)
}

impl LegacyLibraries {
    pub fn read(project: &Path, output: &Path, system: &str) -> Self {
        let mut result = Self::default();
        let (Ok(project), Ok(output)) = (fs::canonicalize(project), fs::canonicalize(output))
        else {
            return result;
        };
        let source_table = project.join("fp-lib-table");
        let copied = read_table(&output.join("fp-lib-table"));
        if let Some(copied) = &copied {
            result.protected.extend(
                legacy_entries(copied)
                    .filter_map(|entry| form_atom(entry, "name", 1).map(str::to_owned)),
            );
        }
        let Some(source) = read_table(&source_table) else {
            return result;
        };
        for entry in legacy_entries(&source) {
            let Some(nickname) = form_atom(entry, "name", 1) else {
                continue;
            };
            result.protected.insert(nickname.into());
            if project == output {
                result.in_place = true;
                continue;
            }
            if entries(&source, nickname).count() != 1
                || form_atom(entry, "options", 1).is_some_and(|value| !value.is_empty())
            {
                continue;
            }
            let Some(source_target) = target(entry, &project, system) else {
                continue;
            };
            let Some(copied_target) = target(entry, &output, system) else {
                continue;
            };
            if source_target.extension().is_none_or(|ext| ext != "mod")
                || !missing(&source_target)
                || !missing(&copied_target)
            {
                continue;
            }
            let mut copied_entries = copied.iter().flat_map(|table| entries(table, nickname));
            let first = copied_entries.next();
            if copied_entries.next().is_some() {
                continue;
            }
            match first {
                None => {}
                Some(working) if working == entry => {}
                Some(working) if form_atom(working, "type", 1) == Some("KiCad") => {
                    let Some(path) = target(working, &output, system)
                        .and_then(|path| super::libraries::staged_library(&path, &output))
                    else {
                        continue;
                    };
                    result.staged.insert(nickname.into(), path);
                    result.protected.remove(nickname);
                    continue;
                }
                Some(_) => continue,
            }
            result.replacements.insert(
                nickname.into(),
                MissingLegacyLibrary {
                    entry: entry.clone(),
                    source_table: source_table.clone(),
                    source_target,
                    copied_target,
                },
            );
            result.protected.remove(nickname);
        }
        result
    }
}

fn legacy_entries(table: &Expr) -> impl Iterator<Item = &Expr> {
    table.children().iter().filter(|entry| {
        entry.head() == Some("lib") && form_atom(entry, "type", 1) == Some("Legacy")
    })
}

impl MissingLegacyLibrary {
    /// Preserve a changed entry or an original `.mod` restored during a run.
    pub fn still_missing(&self, nickname: &str, entry: Option<&Expr>) -> bool {
        (entry.is_none() || entry == Some(&self.entry))
            && missing(&self.source_target)
            && missing(&self.copied_target)
            && read_table(&self.source_table).is_some_and(|table| {
                let mut found = entries(&table, nickname);
                found.next() == Some(&self.entry) && found.next().is_none()
            })
    }
}

// Copyright (C) 2026 Toit contributors.

//! The census of custom rules (moved here from pcb-kicad's project_rules
//! tests, where it was an ignored test that asserted nothing): the
//! lowering of the .kicad_dru files of the expansion intake's boards, per
//! constraint how many rules and boards end in each status (`applied`,
//! `approximated`, `no_effect`, `not_routing`, `unmodelled`, `skipped`).
//!
//!   cargo run --release --manifest-path experiments/project-rules/rules-census/Cargo.toml -- \
//!       benchmarks/expansion/data/manifest.json
//!
//! The manifest's paths are relative to the repository root, four levels
//! above it.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

fn main() {
    let Some(manifest) = std::env::args().nth(1) else {
        eprintln!("usage: rules-census <root>/benchmarks/expansion/data/manifest.json");
        std::process::exit(2);
    };
    let manifest = fs::canonicalize(&manifest).unwrap_or_else(|error| {
        eprintln!("{manifest}: {error}");
        std::process::exit(2);
    });
    let root = manifest.ancestors().nth(4).unwrap();
    let boards: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    let mut counts = BTreeMap::<(String, String), (usize, BTreeSet<String>)>::new();
    let mut failed = 0;
    let mut with_rules = 0;
    for board in boards["boards"].as_array().unwrap() {
        let Some(rules) = board["custom_rules_path"].as_str() else {
            continue;
        };
        with_rules += 1;
        let pcb = root.join(board["board_path"].as_str().unwrap());
        let project = root.join(rules).with_extension("kicad_pro");
        let name = board["name"].as_str().unwrap().to_string();
        match pcb_kicad::resolve_project_rules(&project, Path::new(&pcb)) {
            Ok(config) => {
                let report = config.project_rules.unwrap_or_default();
                if let Some(error) = &report.custom_rules_error {
                    println!("{name}: {error}");
                }
                for rule in report.custom_rules {
                    let entry = counts.entry((rule.constraint, rule.status)).or_default();
                    entry.0 += 1;
                    entry.1.insert(name.clone());
                }
            }
            Err(error) => {
                failed += 1;
                println!("{name}: {error}");
            }
        }
    }
    println!("{with_rules} boards with a .kicad_dru, {failed} not resolved");
    println!("| constraint | status | rules | boards |");
    println!("| --- | --- | ---: | ---: |");
    for ((constraint, status), (rules, boards)) in counts {
        println!("| {constraint} | {status} | {rules} | {} |", boards.len());
    }
}

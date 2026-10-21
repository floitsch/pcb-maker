// Copyright (C) 2026 Toit contributors.

//! docs/config-reference.md and the hand-over's environment index are
//! generated from the code (tools/config-reference): this fails while the
//! committed files differ from what the generator makes of the tree, or
//! while an environment variable lacks its line (or `--help` a user-facing
//! one). `cargo run -p config-reference` rewrites both files.

use std::path::Path;

#[test]
fn the_config_reference_and_the_environment_index_are_current() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut stale = Vec::new();
    for (path, generated) in config_reference::generate(root).unwrap() {
        let committed = std::fs::read_to_string(root.join(&path)).unwrap_or_default();
        if committed != generated {
            let line = committed
                .lines()
                .zip(generated.lines())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| committed.lines().count().min(generated.lines().count()));
            let first = |text: &str| text.lines().nth(line).unwrap_or("(end)").to_string();
            stale.push(format!(
                "{path} is stale from line {}:\n  committed: {}\n  generated: {}",
                line + 1,
                first(&committed),
                first(&generated)
            ));
        }
    }
    let problems = config_reference::problems(root).unwrap();
    assert!(
        stale.is_empty() && problems.is_empty(),
        "{}{}\nrun: cargo run -p config-reference",
        stale.join("\n"),
        problems
            .iter()
            .map(|problem| format!("\nproblem: {problem}"))
            .collect::<String>()
    );
}

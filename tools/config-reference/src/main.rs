// Copyright (C) 2026 Toit contributors.

//! Rewrites docs/config-reference.md and the hand-over's environment index
//! from the code (see the library), and lists what else is out of step.
//!
//! ```text
//! cargo run -p config-reference            # rewrite
//! cargo run -p config-reference -- --check # exit 1 when stale
//! ```

use std::process::ExitCode;

fn main() -> ExitCode {
    let check = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--check") => true,
        Some(other) => {
            eprintln!("usage: config-reference [--check] (unknown argument {other:?})");
            return ExitCode::from(2);
        }
    };
    let root = config_reference::repository_root();
    let generated = match config_reference::generate(&root) {
        Ok(generated) => generated,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut ok = true;
    for (path, text) in generated {
        let current = std::fs::read_to_string(root.join(&path)).unwrap_or_default();
        if current == text {
            continue;
        }
        if check {
            eprintln!("{path} is stale: cargo run -p config-reference");
            ok = false;
        } else if let Err(error) = std::fs::write(root.join(&path), text) {
            eprintln!("error: failed to write {path}: {error}");
            return ExitCode::FAILURE;
        } else {
            println!("wrote {path}");
        }
    }
    match config_reference::problems(&root) {
        Ok(problems) => {
            for problem in &problems {
                eprintln!("problem: {problem}");
            }
            ok &= problems.is_empty();
        }
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

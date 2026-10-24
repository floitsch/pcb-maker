// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The repository tracks no symbolic link. scripts/worktree.sh links the
//! main checkout's `benchmarks/real/external`, `build` and `artifacts`
//! into every worktree; a branch that commits one of those links (as a
//! `git add -A` in a worktree once did, because `.gitignore` patterns
//! ending in `/` match directories only) replaces the real directory
//! with a link to itself when main is fast-forwarded onto it. The
//! harvested boards were lost that way on 2026-10-24.

use std::process::Command;

#[test]
fn the_repository_tracks_no_symbolic_link() {
    let root = env!("CARGO_MANIFEST_DIR");
    let output = Command::new("git")
        .args(["ls-files", "-s"])
        .current_dir(root)
        .output()
        .expect("git ls-files");
    if !output.status.success() {
        // Not a git checkout (a source archive): nothing to check.
        return;
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let links: Vec<&str> = listing
        .lines()
        .filter(|line| line.starts_with("120000 "))
        .collect();
    assert!(links.is_empty(), "tracked symbolic links: {links:?}");
    for linked in ["artifacts", "benchmarks/real/external", "build"] {
        assert!(
            !listing.lines().any(|line| line.ends_with(&format!("\t{linked}"))),
            "{linked} is a worktree link and must not be tracked"
        );
    }
}

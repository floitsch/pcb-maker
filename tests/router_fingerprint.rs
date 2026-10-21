// Copyright (C) 2026 Toit contributors.

//! The router's result does not depend on its thread count: route mode on
//! a fence board of the project's own (`benchmarks/fence/v4-2layer/fence`)
//! at a short budget (2 s of ladder work, 6 negotiation iterations), with
//! `PCB_ROUTER_FINGERPRINT=1` (a hash of the routing state at each phase, on
//! stderr), at `RAYON_NUM_THREADS` 1 and 4: the fingerprints must be the
//! same, compared as a multiset (attempts side by side print in any order).
//! KiCad's check is skipped (`skip_native_verification`), so no kicad-cli
//! is needed, and the wall guard is off (`PCB_ROUTER_NO_WALL_GUARD`), so
//! that the machine's load cannot cut the route. About 15 s a run.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const ROUTER: &str =
    r#"{"ladder_budget_seconds": 2, "maximum_iterations": 6, "skip_native_verification": true}"#;

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pcb-maker-router-fingerprint-{}-{label}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// The sorted fingerprint lines of one route at `threads` search threads.
fn fingerprints(threads: usize) -> Vec<String> {
    let root = scratch(&threads.to_string());
    let router = root.join("router.json");
    fs::write(&router, ROUTER).unwrap();
    let board = Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/fence/v4-2layer/fence");
    let output = Command::new(env!("CARGO_BIN_EXE_pcb-maker"))
        .arg("route-kicad-board")
        .arg(&board)
        .arg("fence")
        .arg(root.join("out"))
        .arg(&router)
        .args(["--reference", "none"])
        .env("PCB_ROUTER_FINGERPRINT", "1")
        .env("PCB_ROUTER_NO_WALL_GUARD", "1")
        .env("RAYON_NUM_THREADS", threads.to_string())
        .env_remove("PCB_ROUTER_FINGERPRINT_ALL")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    // 0: complete, 1: connections left open (this board at this budget);
    // anything else is a failure of the route itself.
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "route-kicad-board at {threads} threads exited with {:?}:\n{stderr}",
        output.status.code()
    );
    let mut lines: Vec<String> = stderr
        .lines()
        .filter(|line| line.starts_with("fingerprint "))
        .map(str::to_string)
        .collect();
    lines.sort();
    let _ = fs::remove_dir_all(&root);
    lines
}

#[test]
fn route_fingerprints_do_not_depend_on_the_thread_count() {
    let one = fingerprints(1);
    assert!(
        !one.is_empty(),
        "no fingerprint printed (PCB_ROUTER_FINGERPRINT)"
    );
    let four = fingerprints(4);
    assert_eq!(one, four, "fingerprints at 1 and 4 threads differ");
}

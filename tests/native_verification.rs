// Copyright (C) 2026 Toit contributors.

#![cfg(unix)]

use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "pcb-maker-native-verification-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("bin")).unwrap();
        fs::create_dir(root.join("board")).unwrap();
        fs::write(root.join("board/board.kicad_pcb"), "(kicad_pcb)\n").unwrap();
        fs::write(root.join("board/board.kicad_sch"), "(kicad_sch)\n").unwrap();
        Self(root)
    }

    fn cli(&self, body: &str) {
        let path = self.0.join("bin/kicad-cli");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn verify(&self, cache: bool) -> Output {
        let mut paths = vec![self.0.join("bin")];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let mut command = Command::new(env!("CARGO_BIN_EXE_pcb-maker"));
        command
            .args(["verify-kicad-rung"])
            .arg(self.0.join("board"))
            .arg("board")
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env_remove("PCB_MAKER_KICAD_VERIFICATION_CACHE")
            .env_remove("PCB_MAKER_KICAD_VERIFICATION_CACHE_EVENTS");
        if cache {
            command.env("PCB_MAKER_KICAD_VERIFICATION_CACHE", self.0.join("cache"));
        }
        command.output().unwrap()
    }

    fn successful_cli(&self) {
        self.cli(r#"if [ "$1" = "version" ]; then echo 10.0.6; exit 0; fi
kind="$1"
if [ "$2" = "export" ]; then
    if [ -e "$0.fail-render" ]; then exit 1; fi
    kind=svg
fi
while [ "$#" -gt 0 ]; do
    if [ "$1" = "--output" ]; then
        shift
        if [ "$kind" = "svg" ]; then
            printf '%s\n' '<svg xmlns="http://www.w3.org/2000/svg"><circle r="1"/></svg>' > "$1"
        elif [ "$kind" = "sch" ]; then
            printf '%s\n' '{"source":"board.kicad_sch","date":"2026-09-07T12:00:00","kicad_version":"10.0.6","sheets":[]}' > "$1"
        else
            printf '%s\n' '{"source":"board.kicad_pcb","date":"2026-09-07T12:00:00","kicad_version":"10.0.6","coordinate_units":"mm","violations":[],"unconnected_items":[],"schematic_parity":[]}' > "$1"
        fi
        exit 0
    fi
    shift
done
exit 99"#);
    }

    fn json(&self, name: &str) -> Value {
        serde_json::from_str(&fs::read_to_string(self.0.join(name)).unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn failed_reverification_removes_the_prior_complete_verdict() {
    let fixture = Fixture::new();
    fixture.successful_cli();
    assert!(fixture.verify(false).status.success());
    assert_eq!(fixture.json("board/verification.json")["complete"], true);
    assert!(fixture.0.join("board/preview.svg").exists());
    assert_eq!(
        fixture.json("board/preview.json")["source"],
        "board.kicad_pcb"
    );
    fixture.cli("exit 1");
    let failed = fixture.verify(false);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("kicad-cli failed"));
    assert!(!fixture.0.join("board/verification.json").exists());
    assert!(!fixture.0.join("board/erc.json").exists());
    assert!(!fixture.0.join("board/preview.svg").exists());
    assert!(fixture.json("board/preview.json")["verification_error"].is_string());
    assert!(fixture.json("board/preview.json")["render_error"].is_string());
}

#[test]
fn cached_assessment_must_match_the_validated_native_reports() {
    let fixture = Fixture::new();
    fixture.successful_cli();
    assert!(fixture.verify(true).status.success());
    assert_eq!(
        fixture.json("board/verification-cache.json")["status"],
        "miss"
    );
    fs::remove_file(fixture.0.join("board/preview.svg")).unwrap();
    assert!(fixture.verify(true).status.success());
    assert_eq!(
        fixture.json("board/verification-cache.json")["status"],
        "hit"
    );
    assert!(fixture.0.join("board/preview.svg").exists());
    assert_eq!(
        fixture.json("board/preview.json")["verification"]["complete"],
        true
    );
    let trace = fixture.json("board/verification-cache.json");
    let entry_path = PathBuf::from(trace["cache_entry"].as_str().unwrap()).join("cache-entry.json");
    let mut entry: Value = serde_json::from_str(&fs::read_to_string(&entry_path).unwrap()).unwrap();
    entry["report"]["complete"] = json!(false);
    fs::write(&entry_path, entry.to_string()).unwrap();
    let rejected = fixture.verify(true);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("inconsistent assessment"));
    assert!(!fixture.0.join("board/verification.json").exists());
    // Rejected cache admission still leaves a fresh visual record.
    assert!(fixture.0.join("board/preview.svg").exists());
    assert!(fixture.json("board/preview.json")["verification_error"].is_string());
}

#[test]
fn preview_failure_does_not_change_native_admission_or_retain_an_old_image() {
    let fixture = Fixture::new();
    fixture.successful_cli();
    assert!(fixture.verify(false).status.success());
    assert!(fixture.0.join("board/preview.svg").exists());
    fs::write(fixture.0.join("bin/kicad-cli.fail-render"), "").unwrap();
    assert!(fixture.verify(false).status.success());
    assert_eq!(fixture.json("board/verification.json")["complete"], true);
    assert!(!fixture.0.join("board/preview.svg").exists());
    assert!(fixture.json("board/preview.json")["render_error"].is_string());
}

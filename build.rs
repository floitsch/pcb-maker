// Copyright (C) 2026 Toit contributors.

//! Embeds the commit and the build date for `pcb-maker --version` and the
//! verdict's `environment` (compile-time variables `BUILD_COMMIT`,
//! `BUILD_DATE`).
//! Without git (a source archive) the commit is "unknown". The build is
//! redone when HEAD moves (a commit, a checkout), not on every edit, so a
//! binary built from a dirty tree names the commit it started from; the
//! date is UTC, from SOURCE_DATE_EPOCH when set (reproducible builds).

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn git(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git").args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// The civil date (UTC) of `seconds` since the epoch, as YYYY-MM-DD
/// (Howard Hinnant's days-to-civil algorithm).
fn date(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn main() {
    // Rerun when HEAD or the branch it names moves (paths that work in a
    // worktree too, where .git is a file).
    for path in ["HEAD", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=build.rs");

    let commit = git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let seconds = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs())
        });
    let built = seconds.map(date).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=BUILD_DATE={built}");
}

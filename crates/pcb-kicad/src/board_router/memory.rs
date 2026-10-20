// Copyright (C) 2026 Toit contributors.

//! Memory measures that decide how many routers run side by side.

use super::*;

/// The process's resident size now, in megabytes (Linux; elsewhere 0).
pub(crate) fn resident_mb() -> f64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("VmRSS:"))?
                .split_whitespace()
                .nth(1)?
                .parse::<f64>()
                .ok()
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

/// The process's peak resident size and the machine's available memory,
/// in megabytes (Linux; elsewhere unknown).
/// The machine's memory, in megabytes: a fixed share of it decides what
/// the ladder may attempt, so that the attempts do not depend on what else
/// runs (the memory available decides only how many run at once).
pub(crate) fn memory_total_mb() -> Option<f64> {
    fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find(|line| line.starts_with("MemTotal:"))?
        .split_whitespace()
        .nth(1)?
        .parse::<f64>()
        .ok()
        .map(|kb| kb / 1024.0)
}

pub(crate) fn memory_mb() -> Option<(f64, f64)> {
    let field = |text: &str, name: &str| -> Option<f64> {
        text.lines()
            .find(|line| line.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse::<f64>()
            .ok()
            .map(|kb| kb / 1024.0)
    };
    let peak = field(&fs::read_to_string("/proc/self/status").ok()?, "VmHWM:")?;
    let available = field(&fs::read_to_string("/proc/meminfo").ok()?, "MemAvailable:")?;
    Some((peak, available))
}

// Copyright (C) 2026 Toit contributors.

//! The machine's and a process's memory as Linux's /proc reports it
//! (elsewhere unknown): what the ledger, the ladder and the layout's race
//! measure their reservations against.

use std::fs;

/// The process's resident size now, in megabytes (Linux; elsewhere 0).
pub fn resident_mb() -> f64 {
    status_field("VmRSS:").unwrap_or(0.0)
}

/// Process `pid`'s resident size now, in megabytes (0 when it is gone or
/// off Linux).
pub(crate) fn resident_mb_of(pid: u32) -> f64 {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|text| kilobytes_field(&text, "VmRSS:"))
        .unwrap_or(0.0)
}

/// The machine's available memory (MemAvailable), in megabytes (Linux;
/// elsewhere None).
pub(crate) fn memory_available_mb() -> Option<f64> {
    kilobytes_field(&fs::read_to_string("/proc/meminfo").ok()?, "MemAvailable:")
}

/// A field of /proc/self/status in megabytes (Linux; elsewhere None).
pub fn status_field(name: &str) -> Option<f64> {
    kilobytes_field(&fs::read_to_string("/proc/self/status").ok()?, name)
}

/// The value of a `Name: N kB` line of a /proc file, in megabytes.
pub fn kilobytes_field(text: &str, name: &str) -> Option<f64> {
    text.lines()
        .find(|line| line.starts_with(name))?
        .split_whitespace()
        .nth(1)?
        .parse::<f64>()
        .ok()
        .map(|kb| kb / 1024.0)
}

/// The machine's memory, in megabytes: a fixed share of it decides what
/// the ladder may attempt, so that the attempts do not depend on what else
/// runs (the memory available decides only how many run at once).
pub fn memory_total_mb() -> Option<f64> {
    kilobytes_field(&fs::read_to_string("/proc/meminfo").ok()?, "MemTotal:")
}

// Copyright (C) 2026 Toit contributors.

//! The settings a route with differential pairs uses.

use super::*;

/// The settings a route with differential pairs uses: a pair's nets at the
/// pair's width where a custom rule sets it (KiCad checks it on all their
/// tracks: the stubs, a pair that falls back; the net class's diff pair
/// width only the coupled runs take), and the project's diff pair rules
/// the pairs take reported `applied` (custom rules on `inDiffPair()` and
/// the `diff_pair_gap` and `diff_pair_uncoupled` constraints).
pub fn with_pairs(config: &KiCadBoardRouterConfig) -> KiCadBoardRouterConfig {
    let mut copy = config.clone();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for (net, rule) in &config.diff_pair_rules {
        let first = (0..rule.layers.len())
            .find(|layer| !rule.disallowed.get(*layer).copied().unwrap_or(false));
        if let (Some(rules), Some((width, _)), true) = (
            copy.connection_rules.get_mut(net),
            first.and_then(|layer| rule.layers.get(layer)),
            first.is_some_and(|layer| rule.width_ruled.get(layer).copied().unwrap_or(false)),
        ) {
            rules.trace_width_mm = *width;
            // The narrow-signal step keeps it (KiCad checks it).
            copy.ruled_widths.insert(net.clone());
        }
        used.extend(rule.rules.iter().cloned());
    }
    if let Some(report) = copy.project_rules.as_mut() {
        for entry in &mut report.custom_rules {
            if entry.status == "unmodelled" && used.contains(&entry.rule) {
                entry.status = "applied".into();
                entry.detail = "differential pairs (diff_pairs): the pairs' width and gap".into();
                let prefix = format!("{} ('{}'", entry.constraint, entry.rule);
                report.unmodelled.retain(|line| !line.starts_with(&prefix));
            }
        }
    }
    copy
}

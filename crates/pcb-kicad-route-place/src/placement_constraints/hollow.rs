// Copyright (C) 2026 Toit contributors.

//! `hollow`: parts whose pads alone block other parts, which may sit inside
//! their courtyard (a shield's outline around its headers, a module
//! mounted above parts).

use super::*;

/// Applies `hollow`: each part a glob matches blocks with its pads' boxes
/// only. Refuses a glob that matches nothing and a part without pads.
pub(super) fn apply_hollow(
    problem: &mut core::Problem,
    parts: &Parts,
    patterns: &[String],
) -> Result<(), String> {
    for pattern in patterns {
        let matched = parts.matching(pattern);
        if matched.is_empty() {
            return Err(format!("hollow constraint {pattern:?} matches no part"));
        }
        for index in matched {
            if parts.pad_boxes[index].is_empty() {
                return Err(format!(
                    "hollow part {} has no pads",
                    parts.references[index]
                ));
            }
            problem.components[index].hollow = parts.pad_boxes[index].clone();
        }
    }
    Ok(())
}

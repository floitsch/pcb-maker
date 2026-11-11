// Copyright (C) 2026 Toit contributors.

//! The report's lines: missing footprints folded by footprint, and why no
//! key matrix was written.

use super::*;

/// The import report's `starter.why` where no key matrix is found among
/// The missing parts' lines (`<reference> (<why>)`, one per part) folded
/// by their reason, which names the footprint: `<n> parts: <references>
/// (<why>)` where several parts share one, in the order of their first
/// part (trials 4 and 6: the ULK's 90 lines were two footprints,
/// eurorack-pmod's 19 three).
pub(super) fn fold_missing(lines: &[String]) -> Vec<String> {
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in lines {
        let (reference, why) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match groups.iter_mut().find(|(other, _)| *other == why) {
            Some((_, references)) => references.push(reference),
            None => groups.push((why, vec![reference])),
        }
    }
    groups
        .into_iter()
        .map(|(why, references)| match references.as_slice() {
            [one] => format!("{one} {why}"),
            many => format!("{} parts: {} {why}", many.len(), many.join(", ")),
        })
        .collect()
}

/// `switches` switch-like parts on the board, `missing` parts left out of
/// it: the matrix is found from the parts on the board, so with parts
/// missing it says that first (trial 4's ULK: 88 of 92 footprints missing,
/// and the report said "no key matrix (keys wired directly to pins, or no
/// keyboard)").
pub(super) fn no_matrix_why(switches: usize, missing: usize) -> String {
    let found = format!(
        "{switches} switch-like parts, but no two share a row and a column net: no key matrix"
    );
    if missing > 0 {
        format!(
            "{found} among the parts on the board; {missing} parts are missing from it (missing): import again with their footprints and the starter is written from the whole matrix"
        )
    } else {
        format!("{found} (keys wired directly to pins, or no keyboard)")
    }
}

/// The report's `drifted_footprints`: per footprint the sizes and up to six
/// of its parts' references.
pub(super) fn drifted_lines(
    drift: BTreeMap<(String, String), (String, Vec<String>)>,
) -> Vec<String> {
    drift
        .into_iter()
        .map(|((id, _), (sizes, references))| {
            let shown: Vec<&str> = references.iter().take(6).map(String::as_str).collect();
            let more = if references.len() > shown.len() {
                ", ..."
            } else {
                ""
            };
            format!(
                "{id} ({} parts: {}{more}): {sizes}",
                references.len(),
                shown.join(", ")
            )
        })
        .collect()
}

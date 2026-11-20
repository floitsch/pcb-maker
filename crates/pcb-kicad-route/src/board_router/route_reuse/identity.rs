// Copyright (C) 2026 Florian Loitsch.

//! Persistent schematic identities for route contacts and tentative poses.

use super::*;

#[derive(Default)]
pub struct RouteReuseIdentities {
    current_to_previous: BTreeMap<String, String>,
    previous_to_current: BTreeMap<String, String>,
    unsafe_previous: BTreeSet<String>,
    unsafe_current: BTreeSet<String>,
    pub(super) has_schematic_links: bool,
}

impl RouteReuseIdentities {
    pub fn of(previous: &Expr, current: &Expr) -> Self {
        let old = Links::of(previous);
        let new = Links::of(current);
        let mut result = Self {
            has_schematic_links: old.has_schematic_links || new.has_schematic_links,
            ..Self::default()
        };
        // Resolve all path matches before considering any name. A name
        // exchanged with another symbol must not take that symbol's poses
        // or contacts merely because it still appears in both files.
        for (reference, footprint) in &new.references {
            let Some(path) = footprint.path.as_deref() else {
                continue;
            };
            let Some(previous) = old.unique_path(path) else {
                continue;
            };
            let donor = &old.references[previous];
            if new.usable(reference)
                && old.usable(previous)
                && (previous == reference
                    || (footprint.library.is_some() && footprint.library == donor.library))
            {
                result.pair(previous, reference);
            }
        }
        // Legacy KiCad boards can lack a schematic link. Use the previous
        // reference fallback only if neither side has a conflicting path
        // claim elsewhere in the other board.
        for (reference, footprint) in &new.references {
            let Some(previous) = old.references.get(reference) else {
                continue;
            };
            if !result.current_to_previous.contains_key(reference)
                && !result.previous_to_current.contains_key(reference)
                && new.usable(reference)
                && old.usable(reference)
                && (footprint.path.is_none() || previous.path.is_none())
                && footprint
                    .path
                    .as_ref()
                    .is_none_or(|path| !old.paths.contains_key(path))
                && previous
                    .path
                    .as_ref()
                    .is_none_or(|path| !new.paths.contains_key(path))
            {
                result.pair(reference, reference);
            }
        }
        result.unsafe_previous = old.conflicts(&new, &result.previous_to_current);
        result.unsafe_current = new.conflicts(&old, &result.current_to_previous);
        result
    }

    fn pair(&mut self, previous: &str, current: &str) {
        self.current_to_previous
            .insert(current.into(), previous.into());
        self.previous_to_current
            .insert(previous.into(), current.into());
    }

    /// The prior footprint whose path or compatible legacy name matches.
    pub fn previous_reference(&self, current: &str) -> Option<&str> {
        self.current_to_previous.get(current).map(String::as_str)
    }

    /// Use the new reference in old pad contact keys. A truly disappeared
    /// footprint retains its old key so the existing leaf-removal check
    /// can consider it. Conflicts are ineligible as a whole.
    pub(super) fn prior_contact_reference<'a>(&'a self, previous: &'a str) -> Option<&'a str> {
        if self.unsafe_previous.contains(previous) {
            return None;
        }
        Some(
            self.previous_to_current
                .get(previous)
                .map_or(previous, String::as_str),
        )
    }

    pub(super) fn current_contact_allowed(&self, current: &str) -> bool {
        !self.unsafe_current.contains(current)
    }

    pub fn renamed(&self) -> BTreeMap<String, String> {
        self.previous_to_current
            .iter()
            .filter(|(previous, current)| previous != current)
            .map(|(previous, current)| (previous.clone(), current.clone()))
            .collect()
    }

    /// New-part relocation must never treat a reused or ambiguous name as
    /// a freshly added footprint, nor move one we found by path.
    pub fn is_new(&self, current: &str) -> bool {
        self.current_contact_allowed(current) && self.previous_reference(current).is_none()
    }
}

#[derive(Default)]
struct Links<'a> {
    references: BTreeMap<String, LinkedFootprint<'a>>,
    paths: BTreeMap<String, Vec<String>>,
    has_schematic_links: bool,
}

struct LinkedFootprint<'a> {
    path: Option<String>,
    library: Option<&'a str>,
    ambiguous: bool,
}

impl<'a> Links<'a> {
    fn of(pcb: &'a Expr) -> Self {
        let mut links = Self::default();
        for footprint in pcb
            .children()
            .iter()
            .filter(|item| matches!(item.head(), Some("footprint" | "module")))
        {
            let reference = footprint_reference(footprint).unwrap_or_default();
            let mut ambiguous = reference.trim().is_empty();
            let mut path = None;
            for (index, form) in footprint
                .children()
                .iter()
                .filter(|form| form.head() == Some("path"))
                .enumerate()
            {
                ambiguous |= index != 0;
                links.has_schematic_links |= index != 0;
                let value = match form.children() {
                    [_, value] => value.atom(),
                    _ => None,
                };
                match value {
                    Some("" | "/") => {}
                    Some(value) => {
                        links.has_schematic_links = true;
                        if let Some(claim) = parse_instance_path(value) {
                            links
                                .paths
                                .entry(claim.clone())
                                .or_default()
                                .push(reference.clone());
                            if index == 0 {
                                path = Some(claim);
                            }
                        } else {
                            ambiguous = true;
                        }
                    }
                    None => {
                        ambiguous = true;
                        links.has_schematic_links = true;
                    }
                }
            }
            let library = footprint.children().get(1).and_then(Expr::atom);
            links
                .references
                .entry(reference)
                .and_modify(|known| known.ambiguous = true)
                .or_insert(LinkedFootprint {
                    path,
                    library,
                    ambiguous,
                });
        }
        links
    }

    fn unique_path(&self, path: &str) -> Option<&str> {
        let [reference] = self.paths.get(path)?.as_slice() else {
            return None;
        };
        Some(reference)
    }

    fn usable(&self, reference: &str) -> bool {
        self.references.get(reference).is_some_and(|footprint| {
            !footprint.ambiguous
                && footprint
                    .path
                    .as_ref()
                    .is_none_or(|path| self.unique_path(path) == Some(reference))
        })
    }

    fn conflicts(&self, other: &Self, matched: &BTreeMap<String, String>) -> BTreeSet<String> {
        self.references
            .iter()
            .filter(|(reference, footprint)| {
                !self.usable(reference)
                    || (!matched.contains_key(*reference)
                        && (other.references.contains_key(*reference)
                            || footprint
                                .path
                                .as_ref()
                                .is_some_and(|path| other.paths.contains_key(path))))
            })
            .map(|(reference, _)| reference.clone())
            .collect()
    }
}

fn parse_instance_path(path: &str) -> Option<String> {
    let path = path.to_ascii_lowercase();
    let mut canonical = String::with_capacity(path.len());
    for part in path.strip_prefix('/')?.split('/') {
        canonical.push('/');
        if !native_uuid(part) {
            if part.len() != 8
                || part == "00000000"
                || !part.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return None;
            }
            // KiCad puts the old 32-bit timestamp in the final eight hex
            // digits of a nil UUID. Apply this to both sheet and symbol IDs.
            canonical.push_str("00000000-0000-0000-0000-0000");
        }
        canonical.push_str(part);
    }
    Some(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcb(parts: &[(&str, &str)]) -> Expr {
        let forms = parts
            .iter()
            .map(|(reference, path)| {
                format!("(footprint \"lib:C\" (property \"Reference\" \"{reference}\") {path})")
            })
            .collect::<Vec<_>>()
            .join(" ");
        parse(&format!("(kicad_pcb {forms})")).unwrap()
    }

    #[test]
    fn exact_paths_precede_reused_names_in_swaps() {
        let previous = pcb(&[
            ("C2", r#"(path "/aaaaaaaa/bbbbbbbb")"#),
            ("C3", r#"(path "/aaaaaaaa/cccccccc")"#),
        ]);
        let current = pcb(&[
            ("C2", r#"(path "/AAAAAAAA/CCCCCCCC")"#),
            ("C3", r#"(path "/aaaaaaaa/bbbbbbbb")"#),
        ]);
        let identities = RouteReuseIdentities::of(&previous, &current);
        assert_eq!(identities.previous_reference("C2"), Some("C3"));
        assert_eq!(identities.prior_contact_reference("C2"), Some("C3"));
        assert_eq!(
            identities.renamed(),
            BTreeMap::from([("C2".into(), "C3".into()), ("C3".into(), "C2".into()),])
        );
    }

    #[test]
    fn legacy_timestamp_segments_match_their_kicad_uuid_forms_in_hierarchical_paths() {
        let previous = pcb(&[(
            "C2",
            r#"(path "/ABC00DEF/bd244025-5710-4dd3-a661-fc57e3d57afd/5EB84F3A")"#,
        )]);
        let current = pcb(&[(
            "C3",
            r#"(path "/00000000-0000-0000-0000-0000abc00def/bd244025-5710-4dd3-a661-fc57e3d57afd/00000000-0000-0000-0000-00005eb84f3a")"#,
        )]);
        let identities = RouteReuseIdentities::of(&previous, &current);
        assert_eq!(identities.previous_reference("C3"), Some("C2"));
        assert_eq!(identities.prior_contact_reference("C2"), Some("C3"));
        assert_eq!(
            parse_instance_path("/00000001-0000-0000-0000-00005EB84F3A").as_deref(),
            Some("/00000001-0000-0000-0000-00005eb84f3a")
        );
        for invalid in ["/00000000", "/5eb84f3a/", "/5eb84f3z"] {
            assert_eq!(parse_instance_path(invalid), None);
        }
    }

    #[test]
    fn aliases_that_claim_the_same_path_still_make_both_references_unsafe() {
        let previous = pcb(&[("C2", r#"(path "/5EB84F3A")"#)]);
        let current = pcb(&[
            ("C2", r#"(path "/00000000-0000-0000-0000-00005eb84f3a")"#),
            ("C3", r#"(path "/5eb84f3a")"#),
        ]);
        let identities = RouteReuseIdentities::of(&previous, &current);
        for reference in ["C2", "C3"] {
            assert_eq!(identities.previous_reference(reference), None);
            assert!(!identities.current_contact_allowed(reference));
        }
        assert_eq!(identities.prior_contact_reference("C2"), None);
    }

    #[test]
    fn repeated_sheet_instances_share_a_leaf_uuid_but_keep_distinct_paths() {
        let previous = pcb(&[
            (
                "C1",
                r#"(path "/00000000-0000-0000-0001-aaaaaaaaaaaa/00000000-0000-0000-0001-bbbbbbbbbbbb")"#,
            ),
            (
                "C2",
                r#"(path "/00000000-0000-0000-0001-cccccccccccc/00000000-0000-0000-0001-bbbbbbbbbbbb")"#,
            ),
        ]);
        let current = pcb(&[
            ("C1", ""),
            (
                "C8",
                r#"(path "/00000000-0000-0000-0001-cccccccccccc/00000000-0000-0000-0001-bbbbbbbbbbbb")"#,
            ),
            (
                "C9",
                r#"(path "/00000000-0000-0000-0001-aaaaaaaaaaaa/00000000-0000-0000-0001-bbbbbbbbbbbb")"#,
            ),
        ]);
        let identities = RouteReuseIdentities::of(&previous, &current);
        assert_eq!(identities.previous_reference("C8"), Some("C2"));
        assert_eq!(identities.previous_reference("C9"), Some("C1"));
        assert_eq!(identities.previous_reference("C1"), None);
        assert!(!identities.current_contact_allowed("C1"));
    }

    #[test]
    fn conflicting_path_forms_also_poison_their_secondary_claim() {
        let previous = pcb(&[
            ("C1", r#"(path "/11111111") (path "/22222222")"#),
            ("C2", r#"(path "/22222222")"#),
        ]);
        let current = pcb(&[("C2", r#"(path "/22222222")"#)]);
        let identities = RouteReuseIdentities::of(&previous, &current);
        assert_eq!(identities.previous_reference("C2"), None);
        assert_eq!(identities.prior_contact_reference("C2"), None);
        assert!(!identities.current_contact_allowed("C2"));
    }

    #[test]
    fn recycled_and_ambiguous_paths_do_not_fall_back_by_name() {
        let previous = pcb(&[
            ("C1", r#"(path "/11111111")"#),
            ("C2", r#"(path "/22222222")"#),
            ("C3", r#"(path "/33333333")"#),
            ("C4", ""),
            ("C5", r#"(path "/55555555")"#),
            ("C6", r#"(path "/66666666")"#),
        ]);
        let current = pcb(&[
            ("C1", r#"(path "/aaaaaaaa")"#),
            ("C2", r#"(path "/22222222")"#),
            ("C3", r#"(path "/22222222")"#),
            ("C4", ""),
            ("C5", ""),
            ("C7", r#"(path "/77777777")"#),
        ]);
        let identities = RouteReuseIdentities::of(&previous, &current);
        for reference in ["C1", "C2", "C3"] {
            assert_eq!(identities.previous_reference(reference), None);
            assert_eq!(identities.prior_contact_reference(reference), None);
            assert!(!identities.current_contact_allowed(reference));
        }
        assert_eq!(identities.previous_reference("C4"), Some("C4"));
        assert_eq!(identities.previous_reference("C5"), Some("C5"));
        assert_eq!(identities.prior_contact_reference("C6"), Some("C6"));
        assert!(identities.is_new("C7"));
    }

    #[test]
    fn a_name_taken_by_a_path_match_cannot_be_reused_by_a_different_donor() {
        let prior = pcb(&[("C1", r#"(path "/11111111")"#), ("C2", "")]);
        let current = pcb(&[("C2", r#"(path "/11111111")"#), ("C1", "")]);
        let identities = RouteReuseIdentities::of(&prior, &current);
        assert_eq!(identities.previous_reference("C2"), Some("C1"));
        assert_eq!(identities.previous_reference("C1"), None);
        assert_eq!(identities.prior_contact_reference("C1"), Some("C2"));
        assert_eq!(identities.prior_contact_reference("C2"), None);
        assert!(identities.current_contact_allowed("C2"));
        assert!(!identities.current_contact_allowed("C1"));
    }
}

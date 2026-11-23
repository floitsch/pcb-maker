// Copyright (C) 2026 Florian Loitsch.

//! Compare footprint-local KiCad group membership independently of item IDs.

use pcb_kicad_file::kicad_sexpr::Expr;
use std::collections::BTreeMap;

type Subtrees = BTreeMap<(String, Vec<usize>), usize>;

pub(super) fn equivalent(
    first: &[Expr],
    second: &[Expr],
    label: impl Fn(&Expr) -> Option<String>,
) -> bool {
    let mut subtrees = Subtrees::new();
    let Some(first) = forest(first, &label, &mut subtrees) else {
        return false;
    };
    forest(second, &label, &mut subtrees) == Some(first)
}

fn identity(item: &Expr, group: bool) -> Option<Option<String>> {
    let mut fields = item.children().iter().filter(|field| {
        matches!(field.head(), Some("uuid" | "tstamp")) || (group && field.head() == Some("id"))
    });
    let Some(field) = fields.next() else {
        return Some(None);
    };
    let ident = field.children().get(1)?.atom()?;
    (fields.next().is_none() && field.children().len() == 2 && !ident.is_empty())
        .then(|| Some(ident.to_ascii_lowercase()))
}

fn forest(
    items: &[Expr],
    label: &impl Fn(&Expr) -> Option<String>,
    subtrees: &mut Subtrees,
) -> Option<Vec<usize>> {
    let mut ids = BTreeMap::new();
    let mut groups = Vec::new();
    let mut labels = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let group = item.head() == Some("group");
        if let Some(id) = identity(item, group)? {
            if ids.insert(id, index).is_some() {
                return None;
            }
        } else if group {
            return None;
        }
        if group {
            let mut fields = item
                .children()
                .iter()
                .filter(|field| field.head() == Some("members"));
            let members = fields.next()?.children();
            if fields.next().is_some() {
                return None;
            }
            groups.push((index, &members[1..]));
            let mut group = item.children().to_vec();
            group.retain(|field| !matches!(field.head(), Some("members" | "id")));
            labels.push(Some(label(&Expr::List(group))?));
        } else {
            labels.push(label(item));
        }
    }

    let mut parent = vec![None; items.len()];
    let mut children_left = vec![0; items.len()];
    for (group, members) in groups {
        for member in members {
            let index = *ids.get(&member.atom()?.to_ascii_lowercase())?;
            // A KiCad item has one parent group. Ambiguous ownership, a
            // repeated member, or a member erased by the model projection
            // cannot prove equivalence.
            if labels[index].is_none() || parent[index].replace(group).is_some() {
                return None;
            }
            children_left[group] += 1;
        }
    }

    // Intern bottom-up, with both footprints sharing the same subtree IDs.
    // Sibling IDs are a multiset. Two physically identical items remain
    // distinct members, and their placement in the tree still matters.
    let mut children = vec![Vec::new(); items.len()];
    let mut pending: Vec<_> = (0..items.len())
        .filter(|&index| children_left[index] == 0 && labels[index].is_some())
        .collect();
    let mut roots = Vec::new();
    let mut remaining = labels.iter().flatten().count();
    while let Some(index) = pending.pop() {
        remaining -= 1;
        let mut members = std::mem::take(&mut children[index]);
        members.sort_unstable();
        let key = (labels[index].take()?, members);
        let next = subtrees.len();
        let id = *subtrees.entry(key).or_insert(next);
        if let Some(parent) = parent[index] {
            children[parent].push(id);
            children_left[parent] -= 1;
            if children_left[parent] == 0 {
                pending.push(parent);
            }
        } else {
            roots.push(id);
        }
    }
    roots.sort_unstable();
    (remaining == 0).then_some(roots) // A cycle has no leaf to start from.
}

#[cfg(test)]
mod tests {
    use crate::board_footprints::{
        same_library_model, same_library_model_with_instance_documentation,
    };
    use pcb_kicad_file::kicad_sexpr::parse;

    const MODEL: &str = r#"(footprint "model"
        (pad "1" smd rect (size 1 1) (uuid "p1"))
        (pad "1" smd rect (size 1 1) (uuid "p2"))
        (pad "2" smd rect (size 1 1) (uuid "p3"))
        (group "inner" (uuid "g1") (members "p1" "p2"))
        (group "outer" (uuid "g2") (members "g1" "p3")))"#;

    #[test]
    fn nested_groups_compare_by_appearance_names_and_membership() {
        let first = parse(MODEL).unwrap();
        let second = parse(
            r#"(footprint "model"
                (group "outer" (id "g4") (members "p6" "g3"))
                (pad "2" smd rect (size 1 1) (tstamp "p6"))
                (pad "1" smd rect (size 1 1) (tstamp "p5"))
                (group "inner" (id "g3") (members "p5" "p4"))
                (pad "1" smd rect (size 1 1) (tstamp "p4")))"#,
        )
        .unwrap();
        assert!(same_library_model(&first, &second));

        for changed in [
            MODEL.replace("\"inner\"", "\"other\""),
            MODEL
                .replace("\"p1\" \"p2\"", "\"p1\" \"p3\"")
                .replace("\"g1\" \"p3\"", "\"g1\" \"p2\""),
            MODEL.replace("\"p1\" \"p2\"", "\"p1\""),
            MODEL.replace("(size 1 1)", "(size 2 1)"),
        ] {
            assert!(!same_library_model(&first, &parse(&changed).unwrap()));
        }
    }

    #[test]
    fn ambiguous_or_invalid_group_ownership_does_not_prove_equivalence() {
        for malformed in [
            MODEL.replace("(members \"p1\" \"p2\")", "(members \"missing\" \"p2\")"),
            MODEL.replace("(uuid \"p2\")", "(uuid \"P1\")"),
            MODEL.replace("(members \"p1\" \"p2\")", "(members \"p1\" \"p1\")"),
            MODEL.replace("(members \"g1\" \"p3\")", "(members \"g1\" \"p1\")"),
            MODEL.replace("(members \"p1\" \"p2\")", "(members \"g2\")"),
        ] {
            let model = parse(&malformed).unwrap();
            assert!(!same_library_model(&model, &model), "{malformed}");
        }
    }

    #[test]
    fn model_projection_cannot_remove_a_group_member() {
        let first = parse(
            r#"(footprint "model"
                (fp_rect (start 0 0) (end 1 1) (layer "Dwgs.User") (uuid "r"))
                (group "drawing" (uuid "g") (members "r")))"#,
        )
        .unwrap();
        assert!(same_library_model(&first, &first));
        assert!(!same_library_model_with_instance_documentation(
            &first, &first
        ));
        let projected =
            parse(r#"(footprint "model" (group "drawing" (uuid "g") (members "r")))"#).unwrap();
        assert!(!same_library_model(&first, &projected));
    }
}

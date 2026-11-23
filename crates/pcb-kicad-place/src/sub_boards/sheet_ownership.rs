// Copyright (C) 2026 Florian Loitsch.

//! Source-sheet evidence for batch placement constraints on a panel.

use super::*;
use crate::kicad_nets::glob_matches;
use crate::kicad_sexpr::{form_atom, unescape};
use crate::placement_constraints::KiCadRegionConstraint;

#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct SheetOwnershipPreview {
    pub pieces: Vec<SheetPiece>,
    pub groups: Vec<SheetGroup>,
    pub unclassified_parts: Vec<String>,
    pub suggested_parts: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_sub_boards: Vec<SheetSubBoard>,
    pub ambiguous_nets_excluded_from_assignment: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct SheetSubBoard {
    pub sheet_file: String,
    pub instance_path: String,
    pub piece: usize,
    pub witnesses: Vec<SheetWitness>,
    pub constraint: KiCadSubBoardConstraint,
}

#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct SheetPiece {
    pub piece: usize,
    pub box_mm: [f64; 4],
    pub assigned_parts: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct SheetGroup {
    pub sheet_file: String,
    pub instance_path: String,
    pub witnesses: Vec<SheetWitness>,
    pub assigned_per_piece: BTreeMap<usize, usize>,
    pub unassigned: usize,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_region: Option<KiCadRegionConstraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_parts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct SheetWitness {
    pub part: String,
    pub piece: usize,
}

#[derive(Default)]
struct Group {
    witnesses: Vec<SheetWitness>,
    assigned: BTreeMap<usize, usize>,
    unresolved: Vec<String>,
    unresolved_fixed: bool,
    includes_descendants: bool,
}

impl Group {
    /// The group has two fixed electrical witnesses on the same piece
    /// and no assigned member on another piece.
    fn witnessed_piece(&self) -> Option<usize> {
        let piece = self.witnesses.first()?.piece;
        (!self.unresolved_fixed
            && self.witnesses.len() >= 2
            && self.witnesses.iter().all(|witness| witness.piece == piece)
            && self.assigned.keys().all(|known| *known == piece))
        .then_some(piece)
    }
}

fn descends_from(instance: &str, ancestor: &str) -> bool {
    instance == ancestor
        || instance
            .strip_prefix(ancestor)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn aggregate_subtrees(
    groups: &BTreeMap<(String, String), Group>,
) -> Vec<(&str, &str, Group, usize)> {
    let mut file_counts = BTreeMap::<_, usize>::new();
    for (_, path) in groups.keys() {
        *file_counts.entry(path).or_default() += 1;
    }
    let mut parents = groups
        .keys()
        .filter(|(_, path)| path != "/")
        .collect::<Vec<_>>();
    parents.sort_by_key(|(_, path)| (path.matches('/').count(), path.as_str()));
    parents
        .into_iter()
        .filter_map(|(file, path)| {
            let mut combined = Group::default();
            for ((_, child), group) in groups {
                if !descends_from(child, path) {
                    continue;
                }
                // A sheet instance has one source file.
                if file_counts[child] != 1 {
                    return None;
                }
                combined.includes_descendants |= child != path;
                combined.witnesses.extend(group.witnesses.iter().cloned());
                combined.unresolved.extend(group.unresolved.iter().cloned());
                combined.unresolved_fixed |= group.unresolved_fixed;
                for (&piece, &count) in &group.assigned {
                    *combined.assigned.entry(piece).or_default() += count;
                }
            }
            let piece = combined.witnessed_piece()?;
            Some((file.as_str(), path.as_str(), combined, piece))
        })
        .collect()
}

fn key(footprint: &Expr) -> Option<(String, String)> {
    let file = form_atom(footprint, "sheetfile", 1).filter(|file| !file.is_empty())?;
    let path = form_atom(footprint, "path", 1)?.strip_prefix('/')?;
    let (instance, symbol) = path.rsplit_once('/').unwrap_or(("", path));
    if symbol.is_empty() {
        return None;
    }
    // One leaf file may be instantiated repeatedly, including where its
    // human-readable sheet name is the same in more than one project.
    Some((unescape(file).into_owned(), format!("/{instance}")))
}

/// When the user has confirmed source sheets on separate pieces, a net
/// name shared across sheets or with untagged parts gives no ownership
/// evidence. This test does not split or remove such nets.
pub(super) fn ambiguous_nets(
    footprints: &[&Expr],
    problem: &pcb_placer::Problem,
    assigned: &[Option<usize>],
    explicit: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    if explicit.is_empty() {
        return BTreeSet::new();
    }
    let keys: Vec<_> = footprints
        .iter()
        .take(assigned.len())
        .map(|item| key(item))
        .collect();
    let mut known = BTreeMap::<_, BTreeMap<_, usize>>::new();
    for (part, (key, piece)) in keys.iter().zip(assigned).enumerate() {
        if let (Some(key), Some(piece)) = (key, piece) {
            *known.entry(key).or_default().entry(*piece).or_default() +=
                usize::from(explicit.contains(&part));
        }
    }
    let agreed: BTreeSet<_> = known
        .values()
        .filter_map(|pieces| {
            let (&piece, &confirmed) = pieces.first_key_value()?;
            (pieces.len() == 1 && confirmed >= 2).then_some(piece)
        })
        .collect();
    if agreed.len() < 2 {
        return BTreeSet::new();
    }
    let mut on = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    for (part, key) in keys.iter().enumerate() {
        for pin in &problem.components[part].pins {
            let first = on.entry(pin.net).or_insert(key.as_ref());
            if *first != key.as_ref() {
                ambiguous.insert(pin.net);
            }
        }
    }
    ambiguous
}

fn exact_pattern(reference: &str) -> String {
    let mut pattern = String::new();
    for c in reference.chars() {
        if matches!(c, '*' | '?' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern
}

pub(crate) fn preview(
    pcb: &Expr,
    sub: &PlacerSubBoards,
    problem: &pcb_placer::Problem,
    references: &[String],
) -> Option<SheetOwnershipPreview> {
    let mut groups = BTreeMap::<_, Group>::new();
    let mut unclassified_parts = Vec::new();
    let mut assigned = vec![0; sub.pieces.count()];
    let parts = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"));
    for (part, (item, piece)) in parts.zip(&sub.part_piece).enumerate() {
        let component = &problem.components[part];
        if component.pins.is_empty() {
            continue;
        }
        let piece = piece.map(|piece| piece + 1);
        if let Some(piece) = piece {
            assigned[piece - 1] += 1;
        }
        let reference = &references[part];
        let Some(identity) = key(item) else {
            if piece.is_none() && !component.fixed {
                unclassified_parts.push(reference.clone());
            }
            continue;
        };
        let group = groups.entry(identity).or_default();
        if let Some(piece) = piece {
            *group.assigned.entry(piece).or_default() += 1;
        } else if !component.fixed {
            group.unresolved.push(reference.clone());
        }
        if component.fixed
            && let Some(Some(piece)) = sub
                .pieces
                .piece_of_box(body_box(component, problem.poses[part]))
        {
            group.witnesses.push(SheetWitness {
                part: reference.clone(),
                piece: piece + 1,
            });
        } else if component.fixed {
            group.unresolved_fixed = true;
        }
    }
    if groups.len() < 2 {
        return None;
    }

    // A sheet label alone is not evidence that the board was panelized.
    // Only offer patches where independently witnessed groups occupy at
    // least two different pieces, and let the agent confirm the patches.
    let eligible = groups
        .values()
        .filter_map(Group::witnessed_piece)
        .collect::<BTreeSet<_>>()
        .len()
        >= 2;
    let mut suggested_sub_boards: Vec<SheetSubBoard> = Vec::new();
    let owners = references
        .iter()
        .take(sub.part_piece.len())
        .enumerate()
        .fold(
            BTreeMap::<_, Vec<_>>::new(),
            |mut owners, (part, reference)| {
                owners.entry(reference.as_str()).or_default().push(part);
                owners
            },
        );
    let subtrees = aggregate_subtrees(&groups);
    if subtrees
        .iter()
        .map(|(_, _, _, piece)| piece)
        .collect::<BTreeSet<_>>()
        .len()
        >= 2
    {
        for (file, path, mut group, piece) in subtrees {
            if group.unresolved.is_empty()
                || (!group.includes_descendants
                    && region_piece(&sub.pieces, sub.pieces.boxes[piece - 1]) == Some(piece - 1))
                || suggested_sub_boards
                    .iter()
                    .any(|suggested| descends_from(path, &suggested.instance_path))
                || group.unresolved.iter().any(|part| {
                    owners
                        .get(part.as_str())
                        .is_none_or(|parts| parts.len() != 1)
                })
            {
                continue;
            }
            let Some(anchor) = group.witnesses.iter().find_map(|witness| {
                let [index] = owners.get(witness.part.as_str())?.as_slice() else {
                    return None;
                };
                let component = &problem.components[*index];
                let pose = problem.poses[*index];
                std::iter::once(component.center(pose))
                    .chain(
                        component
                            .pins
                            .iter()
                            .map(|pin| component.pin_position(pin, pose)),
                    )
                    .map(|point| point.map(|value| (value * 1000.0).round() / 1000.0))
                    .find(|&point| sub.pieces.piece_at(point) == Some(piece - 1))
            }) else {
                continue;
            };
            group.unresolved.sort();
            suggested_sub_boards.push(SheetSubBoard {
                sheet_file: file.into(),
                instance_path: path.into(),
                piece,
                witnesses: group.witnesses,
                constraint: KiCadSubBoardConstraint {
                    piece_at_mm: anchor,
                    parts: group.unresolved,
                },
            });
        }
    }
    let round = |b: &[f64; 4]| b.map(|v| (v * 1000.0).round() / 1000.0);
    let pieces = sub
        .pieces
        .boxes
        .iter()
        .map(round)
        .zip(assigned)
        .enumerate()
        .map(|(i, (box_mm, assigned_parts))| SheetPiece {
            piece: i + 1,
            box_mm,
            assigned_parts,
        })
        .collect::<Vec<_>>();
    let mut suggested_parts = suggested_sub_boards
        .iter()
        .map(|suggestion| suggestion.constraint.parts.len())
        .sum();
    let groups = groups
        .into_iter()
        .map(|((sheet_file, instance_path), group)| {
            let unassigned = group.unresolved.len();
            let witnessed = group.witnessed_piece();
            let mut unresolved_parts = group.unresolved;
            let mut suggested_region = None;
            let status = match (unassigned, witnessed) {
                (0, _) => "already_assigned",
                (_, _)
                    if suggested_sub_boards.iter().any(|suggested| {
                        descends_from(&instance_path, &suggested.instance_path)
                    }) =>
                {
                    unresolved_parts.clear();
                    "witnessed_sub_board"
                }
                (_, Some(piece)) if eligible => {
                    let [x0, y0, x1, y1] = pieces[piece - 1].box_mm;
                    let patterns = unresolved_parts
                        .iter()
                        .map(|reference| exact_pattern(reference))
                        .collect::<Vec<_>>();
                    if !patterns.iter().all(|pattern| {
                        references
                            .iter()
                            .take(sub.part_piece.len())
                            .filter(|reference| glob_matches(pattern, reference))
                            .count()
                            == 1
                    }) {
                        "ambiguous_references"
                    } else if region_piece(&sub.pieces, pieces[piece - 1].box_mm) != Some(piece - 1)
                    {
                        "ambiguous_piece_box"
                    } else {
                        let region = KiCadRegionConstraint {
                            parts: patterns,
                            x: Some([x0, x1]),
                            y: Some([y0, y1]),
                            at: None,
                            origin: Some("absolute".into()),
                        };
                        suggested_region = Some(region);
                        unresolved_parts.clear();
                        suggested_parts += unassigned;
                        "witnessed"
                    }
                }
                (_, Some(_)) => "insufficient_panel_evidence",
                _ if group.witnesses.is_empty() => "no_fixed_witness",
                _ if group.witnesses.len() == 1 && group.assigned.len() <= 1 => "one_fixed_witness",
                _ => "conflicting_witnesses",
            };
            SheetGroup {
                sheet_file,
                instance_path,
                witnesses: group.witnesses,
                assigned_per_piece: group.assigned,
                unassigned,
                status: status.into(),
                suggested_region,
                unresolved_parts,
            }
        })
        .collect();
    Some(SheetOwnershipPreview {
        pieces,
        groups,
        unclassified_parts,
        suggested_parts,
        suggested_sub_boards,
        ambiguous_nets_excluded_from_assignment: sub.ambiguous_sheet_nets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_repeated_sheet_instances_and_root_files_separately() {
        let footprint = |path, sheet| {
            parse(&format!(
                r#"(footprint "test" (path "{path}") (sheetfile "{sheet}"))"#
            ))
            .unwrap()
        };
        let first = key(&footprint("/top/one/symbol", "cell.kicad_sch")).unwrap();
        let second = key(&footprint("/top/two/symbol", "cell.kicad_sch")).unwrap();
        assert_eq!(first.1, "/top/one");
        assert_ne!(first, second);
        assert_eq!(
            key(&footprint("/top/one/another-symbol", "cell.kicad_sch")),
            Some(first)
        );
        assert_ne!(
            key(&footprint("/symbol", "left.kicad_sch")),
            key(&footprint("/symbol", "right.kicad_sch"))
        );
        assert_eq!(key(&footprint("/", "cell.kicad_sch")), None);
    }

    #[test]
    fn ambiguous_evidence_never_nominates_a_piece() {
        let witness = |part: &str, piece| SheetWitness {
            part: part.into(),
            piece,
        };
        let mut group = Group {
            witnesses: vec![witness("R1", 1), witness("C1", 1)],
            ..Default::default()
        };
        assert_eq!(group.witnessed_piece(), Some(1));
        group.assigned.insert(2, 1);
        assert_eq!(group.witnessed_piece(), None);
        group.assigned.clear();
        group.witnesses[1].piece = 2;
        assert_eq!(group.witnessed_piece(), None);
        group.witnesses.pop();
        assert_eq!(group.witnessed_piece(), None);
        assert_eq!(exact_pattern(r"R**?\C"), r"R\*\*\?\\C");
    }

    #[test]
    fn only_the_actual_instance_prefix_can_supply_a_childs_missing_witnesses() {
        let held = |prefix: &str, piece| Group {
            witnesses: ["R", "C"]
                .map(|part| SheetWitness {
                    part: format!("{prefix}{part}"),
                    piece,
                })
                .to_vec(),
            ..Group::default()
        };
        let key = |file: &str, path: &str| (file.to_owned(), path.to_owned());
        let mut groups = BTreeMap::from([
            (key("board", "/left"), held("L", 1)),
            (key("board", "/left_other"), held("R", 2)),
            (
                key("flash", "/left/flash"),
                Group {
                    unresolved: vec!["U1".into()],
                    ..Group::default()
                },
            ),
        ]);
        let choices = aggregate_subtrees(&groups);
        let (_, _, group, piece) = choices
            .iter()
            .find(|(_, path, _, _)| *path == "/left")
            .unwrap();
        assert!(group.includes_descendants);
        assert_eq!(group.unresolved, ["U1"]);
        assert_eq!(*piece, 1);
        groups.insert(key("foreign", "/left/mixed"), held("M", 2));
        assert!(
            aggregate_subtrees(&groups)
                .iter()
                .all(|(_, path, _, _)| *path != "/left")
        );
        groups.remove(&key("foreign", "/left/mixed"));
        groups.insert(key("another_file", "/left/flash"), Group::default());
        assert!(
            aggregate_subtrees(&groups)
                .iter()
                .all(|(_, path, _, _)| *path != "/left")
        );
    }
}

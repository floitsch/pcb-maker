// Copyright (C) 2026 Toit contributors.

//! Routed branches as a graph: vertices inserted wherever same-layer routes
//! touch, so physical edges deduplicate.

use super::*;

#[derive(Clone, Copy, Debug)]
struct SelectedRouteSegment {
    branch_index: usize,
    segment_index: usize,
    start: [f64; 2],
    end: [f64; 2],
}

const ROUTE_CONTACT_EPSILON_MM: f64 = 1.0e-9;

fn route_cross(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[1] - left[1] * right[0]
}

fn route_subtract(left: [f64; 2], right: [f64; 2]) -> [f64; 2] {
    [left[0] - right[0], left[1] - right[1]]
}

fn route_point_on_segment(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> bool {
    let direction = route_subtract(end, start);
    let offset = route_subtract(point, start);
    let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
    if length_squared <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM {
        return distance_squared(point, start)
            <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM;
    }
    let scale = length_squared.sqrt().max(1.0);
    if route_cross(direction, offset).abs() > ROUTE_CONTACT_EPSILON_MM * scale {
        return false;
    }
    let projection = offset[0] * direction[0] + offset[1] * direction[1];
    projection >= -ROUTE_CONTACT_EPSILON_MM
        && projection <= length_squared + ROUTE_CONTACT_EPSILON_MM
}

fn route_segment_parameter(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let direction = route_subtract(end, start);
    let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
    if length_squared == 0.0 {
        0.0
    } else {
        let offset = route_subtract(point, start);
        (offset[0] * direction[0] + offset[1] * direction[1]) / length_squared
    }
}

/// Returns the physical contact points shared by two same-layer segments.
///
/// Collinear overlap is represented by its endpoint vertices. Proper
/// crossings get one computed point, while endpoint/T contacts retain the
/// exact stored endpoint coordinate so later bitwise grouping is stable.
fn route_segment_contacts(
    first_start: [f64; 2],
    first_end: [f64; 2],
    second_start: [f64; 2],
    second_end: [f64; 2],
) -> Vec<[f64; 2]> {
    let mut contacts = Vec::new();
    for point in [first_start, first_end, second_start, second_end] {
        if route_point_on_segment(point, first_start, first_end)
            && route_point_on_segment(point, second_start, second_end)
            && !contacts.iter().any(|existing| {
                distance_squared(*existing, point)
                    <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM
            })
        {
            contacts.push(point);
        }
    }
    if !contacts.is_empty() {
        return contacts;
    }

    let first_direction = route_subtract(first_end, first_start);
    let second_direction = route_subtract(second_end, second_start);
    let denominator = route_cross(first_direction, second_direction);
    let scale = (distance_squared(first_start, first_end)
        * distance_squared(second_start, second_end))
    .sqrt()
    .max(1.0);
    if denominator.abs() <= ROUTE_CONTACT_EPSILON_MM * scale {
        return contacts;
    }
    let between_starts = route_subtract(second_start, first_start);
    let first_parameter = route_cross(between_starts, second_direction) / denominator;
    let second_parameter = route_cross(between_starts, first_direction) / denominator;
    let contact_range = -ROUTE_CONTACT_EPSILON_MM..=1.0 + ROUTE_CONTACT_EPSILON_MM;
    if contact_range.contains(&first_parameter) && contact_range.contains(&second_parameter) {
        contacts.push([
            first_start[0] + first_parameter * first_direction[0],
            first_start[1] + first_parameter * first_direction[1],
        ]);
    }
    contacts
}

/// Inserts explicit vertices wherever selected same-layer routes physically
/// touch. The operation preserves the initial copper geometry but turns
/// collinear overlap, T contacts, and crossings into a graph that the generic
/// shared-particle compiler can represent without duplicate physical edges.
pub(crate) fn normalize_selected_route_graph(
    branches: &mut [KiCadRouteBranch],
    selected: &[usize],
) {
    normalize_selected_route_graph_against(branches, selected, selected)
}

fn normalize_selected_route_graph_against(
    branches: &mut [KiCadRouteBranch],
    selected: &[usize],
    context: &[usize],
) {
    let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
    let original_paths = context
        .iter()
        .map(|&branch_index| (branch_index, branches[branch_index].path.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut segments = Vec::new();
    let mut cuts = BTreeMap::<(usize, usize), Vec<[f64; 2]>>::new();
    for (&branch_index, path) in &original_paths {
        for (segment_index, pair) in path.windows(2).enumerate() {
            if pair[0].layer != pair[1].layer || pair[0].at == pair[1].at {
                continue;
            }
            segments.push(SelectedRouteSegment {
                branch_index,
                segment_index,
                start: pair[0].at,
                end: pair[1].at,
            });
            if selected_set.contains(&branch_index) {
                cuts.insert((branch_index, segment_index), vec![pair[0].at, pair[1].at]);
            }
        }
    }

    for first in 0..segments.len() {
        for second in (first + 1)..segments.len() {
            let first_segment = segments[first];
            let second_segment = segments[second];
            let first_path = &original_paths[&first_segment.branch_index];
            let second_path = &original_paths[&second_segment.branch_index];
            if first_path[first_segment.segment_index].layer
                != second_path[second_segment.segment_index].layer
            {
                continue;
            }
            let contacts = route_segment_contacts(
                first_segment.start,
                first_segment.end,
                second_segment.start,
                second_segment.end,
            );
            // Ordinary adjacent segments only share their existing vertex.
            // A backtrack shares an interval: split its endpoints just like
            // overlaps between different branches so physical edges dedupe.
            if first_segment.branch_index == second_segment.branch_index
                && first_segment
                    .segment_index
                    .abs_diff(second_segment.segment_index)
                    == 1
                && contacts.len() <= 1
            {
                continue;
            }
            for contact in contacts {
                if let Some(segment_cuts) =
                    cuts.get_mut(&(first_segment.branch_index, first_segment.segment_index))
                {
                    segment_cuts.push(contact);
                }
                if let Some(segment_cuts) =
                    cuts.get_mut(&(second_segment.branch_index, second_segment.segment_index))
                {
                    segment_cuts.push(contact);
                }
            }
        }
    }

    for &branch_index in selected {
        let original = &original_paths[&branch_index];
        // A terminal already reached by the tree can have a single contact
        // point and no new segment. Preserve that connectivity evidence.
        if original.len() < 2 {
            continue;
        }
        let mut normalized = Vec::new();
        for (segment_index, pair) in original.windows(2).enumerate() {
            let Some(segment_cuts) = cuts.get(&(branch_index, segment_index)) else {
                if normalized.is_empty() {
                    normalized.push(pair[0].clone());
                }
                normalized.push(pair[1].clone());
                continue;
            };
            let mut segment_cuts = segment_cuts.clone();
            segment_cuts.sort_by(|left, right| {
                route_segment_parameter(*left, pair[0].at, pair[1].at)
                    .total_cmp(&route_segment_parameter(*right, pair[0].at, pair[1].at))
                    .then_with(|| left[0].total_cmp(&right[0]))
                    .then_with(|| left[1].total_cmp(&right[1]))
            });
            segment_cuts.dedup_by(|left, right| {
                distance_squared(*left, *right)
                    <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM
            });
            for (cut_index, at) in segment_cuts.into_iter().enumerate() {
                if segment_index > 0 && cut_index == 0 {
                    continue;
                }
                normalized.push(KiCadRoutePoint {
                    at,
                    layer: pair[0].layer.clone(),
                });
            }
        }
        branches[branch_index].path = normalized;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_graph_normalization_splits_collinear_overlap_and_interior_contacts() {
        let point = |at| KiCadRoutePoint {
            at,
            layer: "F.Cu".into(),
        };
        let branch = |path: Vec<[f64; 2]>| KiCadRouteBranch {
            start_terminal: path[0],
            finish_terminal: *path.last().unwrap(),
            cost: 0,
            expansions: 0,
            path: path.into_iter().map(point).collect(),
        };
        let mut branches = vec![
            branch(vec![[0.0, 0.0], [4.0, 0.0], [5.0, 1.0]]),
            branch(vec![[0.0, 0.0], [2.0, 0.0], [5.0, 2.0]]),
            branch(vec![[-1.0, -1.0], [1.0, 0.0], [1.0, 1.0]]),
        ];

        normalize_selected_route_graph(&mut branches, &[0, 1, 2]);

        assert_eq!(
            branches[0]
                .path
                .iter()
                .map(|point| point.at)
                .collect::<Vec<_>>(),
            vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [4.0, 0.0], [5.0, 1.0]]
        );
        assert_eq!(
            branches[1]
                .path
                .iter()
                .map(|point| point.at)
                .collect::<Vec<_>>(),
            vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [5.0, 2.0]]
        );
    }

    #[test]
    fn route_normalization_preserves_an_already_connected_terminal() {
        let mut branches = vec![KiCadRouteBranch {
            start_terminal: [2.0, 3.0],
            finish_terminal: [2.0, 3.0],
            cost: 0,
            expansions: 0,
            path: vec![KiCadRoutePoint {
                at: [2.0, 3.0],
                layer: "F.Cu".into(),
            }],
        }];
        normalize_selected_route_graph(&mut branches, &[0]);
        assert_eq!(branches[0].path.len(), 1);
        assert_eq!(branches[0].path[0].at, [2.0, 3.0]);
        assert_eq!(branches[0].path[0].layer, "F.Cu");
    }
}

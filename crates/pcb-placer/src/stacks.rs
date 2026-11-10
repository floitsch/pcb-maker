// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Stacks in the netlist's start (docs/placer.md, "The global placement's
//! start" and "Banks on a wide bus glued through the descent").
//! The quadratic solve (`initial::quadratic_start`) puts parts on the same
//! nets on one spot: the bound-to-bound model cannot tell them apart (the
//! OpenESC 4in1-mini's 22 bulk capacitors on the battery rail and ground
//! within 0.2 mm, video's eight memory modules on one bus within 1 mm). A
//! stack here is a group of movable parts on one side whose nets have a share
//! and a count in common (`shared`, `common`) and whose solved bodies, halos
//! included, overlap (`find_stacks`); each is spread into a bank of cells at
//! its pitch around its centroid (`bank_columns`, `bank_centers`).
//!
//! A bank at the start alone is undone by the global placement's descent
//! wherever no wire holds its parts (capacitors on a rail and ground); glued
//! (`global::Stacks::Glue`), it moves as one body through the descent. The
//! default (`global::Stacks::BANKS`) glues banks of three or more parts on
//! one wide bus: nine tenths of their nets and 24 at least in common.

use crate::problem::Point;

/// Each part's key: its nets, sorted and without repeats, where it has at
/// least `least` of them, and two at least (a part on one net, or none, is
/// no member of a stack: it has no wire that sets it on one spot with
/// others). Parts with one key share every net.
pub fn net_keys(nets: &[Vec<usize>], least: usize) -> Vec<Option<Vec<usize>>> {
    nets.iter()
        .map(|nets| {
            let mut key = nets.clone();
            key.sort_unstable();
            key.dedup();
            (key.len() >= least.max(2)).then_some(key)
        })
        .collect()
}

/// The nets two keys (sorted, without repeats) have in common.
pub fn common(a: &[usize], b: &[usize]) -> usize {
    let (mut i, mut j, mut both) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                both += 1;
                i += 1;
                j += 1;
            }
        }
    }
    both
}

/// The share of their nets two keys (sorted, without repeats) have in
/// common: the nets on both over the nets on either (1: the same nets).
pub fn shared(a: &[usize], b: &[usize]) -> f64 {
    let both = common(a, b);
    let either = a.len() + b.len() - both;
    if either == 0 {
        0.0
    } else {
        both as f64 / either as f64
    }
}

/// The stacks: groups of parts with a key (`Some`) and one density
/// `field` whose keys have `least_share` of their nets and `least_common`
/// nets at least in common and whose bodies of half extents `half` (halos
/// included) at `centers` overlap, chained (a part meets the group where it
/// meets one member so). Groups of fewer than `least_members` parts (two at
/// least) are left out. Each group is in index order, the groups by their
/// first member; parts in no group are left out.
pub fn find_stacks(
    centers: &[Point],
    half: &[Point],
    field: &[usize],
    keys: &[Option<Vec<usize>>],
    least_share: f64,
    least_common: usize,
    least_members: usize,
) -> Vec<Vec<usize>> {
    let count = centers.len();
    let mut parent: Vec<usize> = (0..count).collect();
    fn find(parent: &mut [usize], mut part: usize) -> usize {
        while parent[part] != part {
            parent[part] = parent[parent[part]];
            part = parent[part];
        }
        part
    }
    // Swept by x: a part meets only those within its reach along x.
    let mut order: Vec<usize> = (0..count).filter(|part| keys[*part].is_some()).collect();
    order.sort_by(|a, b| centers[*a][0].total_cmp(&centers[*b][0]).then(a.cmp(b)));
    let reach = order.iter().map(|part| half[*part][0]).fold(0.0, f64::max);
    for (position, &a) in order.iter().enumerate() {
        for &b in &order[position + 1..] {
            if centers[b][0] - centers[a][0] > half[a][0] + reach {
                break;
            }
            let overlap = |axis: usize| {
                (centers[a][axis] - centers[b][axis]).abs() < half[a][axis] + half[b][axis]
            };
            let alike = match (&keys[a], &keys[b]) {
                (Some(key_a), Some(key_b)) => {
                    shared(key_a, key_b) >= least_share && common(key_a, key_b) >= least_common
                }
                _ => false,
            };
            if field[a] == field[b] && alike && overlap(0) && overlap(1) {
                let (root_a, root_b) = (find(&mut parent, a), find(&mut parent, b));
                if root_a != root_b {
                    parent[root_a.max(root_b)] = root_a.min(root_b);
                }
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for part in order {
        let root = find(&mut parent, part);
        groups.entry(root).or_default().push(part);
    }
    groups
        .into_values()
        .filter(|group| group.len() >= least_members.max(2))
        .map(|mut group| {
            group.sort_unstable();
            group
        })
        .collect()
}

/// The columns of a bank of `count` cells of size `cell`: the bank is
/// `columns` cells wide along x and as many rows as it takes along y. The
/// wire it adds is priced per net of the stack: `spans` holds each net's
/// weight and the extent of its other pins (not the stack's) along x and
/// y, and a bank longer than a net's extent lengthens that net by the
/// difference. A bank longer than `room` (the board's usable size) costs
/// more than any wire. Of equal costs the most compact bank (the shorter
/// longer side), then the fewer columns.
pub fn bank_columns(count: usize, cell: Point, spans: &[(f64, Point)], room: Point) -> usize {
    let weight: f64 = spans.iter().map(|(weight, _)| weight).sum::<f64>().max(1.0);
    let cost = |columns: usize| -> (f64, f64) {
        let rows = count.div_ceil(columns);
        let extent = [columns as f64 * cell[0], rows as f64 * cell[1]];
        let mut wire = 0.0;
        for (weight, span) in spans {
            for axis in 0..2 {
                wire += weight * (extent[axis] - span[axis]).max(0.0);
            }
        }
        let beyond: f64 = (0..2)
            .map(|axis| (extent[axis] - room[axis]).max(0.0))
            .sum();
        (wire + 1.0e3 * weight * beyond, extent[0].max(extent[1]))
    };
    let mut best = (1, cost(1));
    for columns in 2..=count.max(1) {
        let value = cost(columns);
        // A tenth of a micrometre of wire is a tie.
        let better = value.0 < best.1.0 - 1.0e-4
            || (value.0 <= best.1.0 + 1.0e-4 && value.1 < best.1.1 - 1.0e-9);
        if better {
            best = (columns, value);
        }
    }
    best.0
}

/// The cells' centres of a bank of `count` cells of size `cell`,
/// `columns` wide, row by row from the lowest x and y, with their centroid
/// at `center` (a short last row too).
pub fn bank_centers(count: usize, cell: Point, columns: usize, center: Point) -> Vec<Point> {
    let columns = columns.clamp(1, count.max(1));
    let rows = count.div_ceil(columns);
    let mut at: Vec<Point> = (0..count)
        .map(|index| {
            let (row, column) = (index / columns, index % columns);
            [
                (column as f64 - (columns as f64 - 1.0) / 2.0) * cell[0],
                (row as f64 - (rows as f64 - 1.0) / 2.0) * cell[1],
            ]
        })
        .collect();
    let mean = [
        at.iter().map(|point| point[0]).sum::<f64>() / count.max(1) as f64,
        at.iter().map(|point| point[1]).sum::<f64>() / count.max(1) as f64,
    ];
    for point in &mut at {
        *point = [
            center[0] + point[0] - mean[0],
            center[1] + point[1] - mean[1],
        ];
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_that_share_every_net_have_one_key() {
        let keys = net_keys(
            &[
                vec![3, 1],    // a capacitor on the rail (3) and ground (1)
                vec![1, 3],    // another, turned
                vec![1, 3, 3], // a part with two pins on the rail
                vec![5, 3],    // a gate resistor: one net shared
                vec![3, 1, 5], // a MOSFET: the resistor's net and two more
                vec![4],       // a part on one net
                vec![],
            ],
            2,
        );
        assert_eq!(keys[0], Some(vec![1, 3]));
        assert_eq!(keys[0], keys[1]);
        assert_eq!(keys[0], keys[2]);
        assert_ne!(keys[3], keys[4]);
        assert_ne!(keys[0], keys[4]);
        assert_eq!(keys[5], None);
        assert_eq!(keys[6], None);
        // Parts with three nets or more only: the MOSFET.
        let keys = net_keys(&[vec![3, 1], vec![3, 1, 5]], 3);
        assert_eq!(keys, vec![None, Some(vec![1, 3, 5])]);
    }

    #[test]
    fn a_stack_is_overlapping_parts_of_one_key_on_one_side() {
        let key = Some(vec![1, 3]);
        let other = Some(vec![1, 4]);
        let centers = [
            [10.0, 10.0],
            [10.1, 10.0],
            [10.0, 10.05],
            [11.9, 10.0], // overlaps the second only: chained
            [30.0, 30.0], // alone
            [10.0, 10.0], // another key
            [10.0, 10.0], // the other side
            [14.0, 10.0], // 2.1 mm right of the fourth: clear of it
        ];
        let half = [[1.0, 0.5]; 8];
        let field = [0, 0, 0, 0, 0, 0, 1, 0];
        let keys = [
            key.clone(),
            key.clone(),
            key.clone(),
            key.clone(),
            key.clone(),
            other,
            key.clone(),
            key,
        ];
        let stacks = find_stacks(&centers, &half, &field, &keys, 1.0, 0, 2);
        assert_eq!(stacks, vec![vec![0, 1, 2, 3]]);
    }

    #[test]
    fn the_shared_share_is_the_nets_on_both_over_the_nets_on_either() {
        assert_eq!(shared(&[1, 3], &[1, 3]), 1.0);
        // A MOSFET and its gate resistor: one net of four.
        assert_eq!(shared(&[1, 3, 5], &[5, 7]), 0.25);
        // Two memory modules on one bus, each with its own select net: 50
        // of 52.
        let bus: Vec<usize> = (0..50).collect();
        let a: Vec<usize> = bus.iter().copied().chain([100]).collect();
        let b: Vec<usize> = bus.iter().copied().chain([101]).collect();
        assert!((shared(&a, &b) - 50.0 / 52.0).abs() < 1.0e-12);
        assert_eq!(shared(&[], &[]), 0.0);
    }

    #[test]
    fn modules_that_differ_in_one_net_of_many_stack_at_nine_tenths() {
        let bus: Vec<usize> = (0..50).collect();
        let key = |own: usize| Some(bus.iter().copied().chain([100 + own]).collect::<Vec<_>>());
        let keys = [key(0), key(1), key(2)];
        let centers = [[50.0, 50.0], [50.2, 50.1], [50.1, 49.9]];
        let half = [[58.0, 4.0]; 3];
        assert_eq!(
            find_stacks(&centers, &half, &[0; 3], &keys, 0.9, 0, 2),
            vec![vec![0, 1, 2]]
        );
        assert!(find_stacks(&centers, &half, &[0; 3], &keys, 1.0, 0, 2).is_empty());
    }

    #[test]
    fn a_bus_stack_shares_its_lines() {
        // Two memories on one bus (28 nets, each with its own select) and
        // two MOSFETs on the same three nets.
        let bus: Vec<usize> = (0..27).collect();
        let memory = |own: usize| Some(bus.iter().copied().chain([100 + own]).collect::<Vec<_>>());
        let keys = [
            memory(0),
            memory(1),
            Some(vec![200, 201, 202]),
            Some(vec![200, 201, 202]),
        ];
        let centers = [[10.0, 10.0], [10.2, 10.1], [40.0, 40.0], [40.1, 40.0]];
        let half = [[6.0, 9.0], [6.0, 9.0], [1.0, 1.0], [1.0, 1.0]];
        assert_eq!(
            find_stacks(&centers, &half, &[0; 4], &keys, 0.9, 0, 2),
            vec![vec![0, 1], vec![2, 3]]
        );
        // 24 nets or more in common: the memories alone.
        assert_eq!(
            find_stacks(&centers, &half, &[0; 4], &keys, 0.9, 24, 2),
            vec![vec![0, 1]]
        );
        // Three members at least: neither pair.
        assert!(find_stacks(&centers, &half, &[0; 4], &keys, 0.9, 24, 3).is_empty());
        assert_eq!(common(&[1, 2, 3], &[2, 3, 4]), 2);
    }

    #[test]
    fn parts_a_body_apart_are_no_stack() {
        let key = Some(vec![1, 2]);
        let centers = [[10.0, 10.0], [12.0, 10.0], [10.0, 11.0]];
        let stacks = find_stacks(
            &centers,
            &[[1.0, 0.5]; 3],
            &[0; 3],
            &[key.clone(), key.clone(), key],
            1.0,
            0,
            2,
        );
        assert!(stacks.is_empty(), "{stacks:?}");
    }

    #[test]
    fn a_bank_on_nets_that_span_the_board_is_compact() {
        // 22 cells of 2 x 1 mm on two nets spanning 20 mm both ways: no
        // shape adds wire; the most compact is 3 columns (6 x 8 mm).
        let spans = [(1.0, [20.0, 20.0]); 2];
        let columns = bank_columns(22, [2.0, 1.0], &spans, [20.0, 20.0]);
        assert_eq!(columns, 3);
    }

    #[test]
    fn a_bank_lies_along_its_nets() {
        // Two nets spanning 10 mm along x and nothing along y: a row.
        let spans = [(1.0, [10.0, 0.0]); 2];
        assert_eq!(bank_columns(4, [2.0, 1.0], &spans, [50.0, 50.0]), 4);
        // Along y: a column.
        let spans = [(1.0, [0.0, 10.0]); 2];
        assert_eq!(bank_columns(4, [2.0, 1.0], &spans, [50.0, 50.0]), 1);
    }

    #[test]
    fn a_bank_fits_the_board() {
        // Eight 116 x 8 mm modules on a 150 x 100 mm board: one column (the
        // nets would rather have a row, which does not fit).
        let spans = [(1.0, [200.0, 0.0])];
        assert_eq!(bank_columns(8, [116.0, 8.0], &spans, [150.0, 100.0]), 1);
    }

    #[test]
    fn the_bank_keeps_the_centroid() {
        let at = bank_centers(5, [2.0, 1.0], 3, [10.0, 20.0]);
        assert_eq!(at.len(), 5);
        let mean = [
            at.iter().map(|p| p[0]).sum::<f64>() / 5.0,
            at.iter().map(|p| p[1]).sum::<f64>() / 5.0,
        ];
        assert!(
            (mean[0] - 10.0).abs() < 1.0e-9 && (mean[1] - 20.0).abs() < 1.0e-9,
            "{at:?}"
        );
        // At the pitch: neighbours in a row 2 mm apart, rows 1 mm apart.
        assert!((at[1][0] - at[0][0] - 2.0).abs() < 1.0e-9);
        assert!((at[3][1] - at[0][1] - 1.0).abs() < 1.0e-9);
        // No two cells overlap.
        for a in 0..5 {
            for b in a + 1..5 {
                assert!(
                    (at[a][0] - at[b][0]).abs() >= 2.0 - 1.0e-9
                        || (at[a][1] - at[b][1]).abs() >= 1.0 - 1.0e-9
                );
            }
        }
    }
}

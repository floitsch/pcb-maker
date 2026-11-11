// Copyright (C) 2026 Toit contributors.

//! The parts a board of several pieces, or a panel of copies, keeps on
//! their own piece or copy (`keep_on_pieces`, `keep_copies`).

use super::*;
use pcb_placer as core;

/// On a board of several pieces (two boards side by side), no copper
/// joins one piece to another: a net lies on one piece. The parts a net
/// joins (directly or through other parts) stay on the piece of the fixed
/// parts among them, as a region constraint: zpn_devboard's movable parts
/// spread over its second piece, which holds one connector, and left 129
/// connections open after the first route (the designer's placement: 1).
pub(in crate::board_placer) fn keep_on_pieces(problem: &mut core::Problem, footprints: usize) {
    if problem.pieces.len() < 2 {
        return;
    }
    // Parts joined by nets, by union-find.
    let mut parent: Vec<usize> = (0..footprints).collect();
    fn root(parent: &mut [usize], mut part: usize) -> usize {
        while parent[part] != part {
            parent[part] = parent[parent[part]];
            part = parent[part];
        }
        part
    }
    let mut first_on_net: BTreeMap<usize, usize> = BTreeMap::new();
    for part in 0..footprints {
        for pin in &problem.components[part].pins {
            match first_on_net.get(&pin.net) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, part), root(&mut parent, other));
                    parent[a] = b;
                }
                None => {
                    first_on_net.insert(pin.net, part);
                }
            }
        }
    }
    // The pieces the fixed parts of each group sit on.
    let piece_of = |problem: &core::Problem, part: usize| {
        let center = problem.components[part].center(problem.poses[part]);
        problem.pieces.iter().position(|b| {
            center[0] >= b[0] && center[0] <= b[2] && center[1] >= b[1] && center[1] <= b[3]
        })
    };
    let mut anchors: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for part in 0..footprints {
        if problem.components[part].fixed && !problem.components[part].pins.is_empty() {
            let group = root(&mut parent, part);
            if let Some(piece) = piece_of(problem, part) {
                anchors.entry(group).or_default().insert(piece);
            }
        }
    }
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!(
            "pieces: {} pieces, net groups anchored on them {:?}",
            problem.pieces.len(),
            anchors
                .values()
                .map(|pieces| pieces.len())
                .collect::<Vec<_>>()
        );
    }
    let constrained: BTreeSet<usize> = problem
        .constraints
        .regions
        .iter()
        .map(|(part, _)| *part)
        .collect();
    for part in 0..footprints {
        if problem.components[part].fixed
            || problem.components[part].pins.is_empty()
            || constrained.contains(&part)
        {
            continue;
        }
        let group = root(&mut parent, part);
        if let Some(pieces) = anchors.get(&group)
            && pieces.len() == 1
        {
            let piece = problem.pieces[*pieces.iter().next().expect("one piece")];
            problem.constraints.regions.push((part, piece));
            problem.constraints.automatic_regions.insert(part);
        }
    }
}

/// A panel of copies joined by tabs is one outline piece, so
/// `keep_on_pieces` cannot tell the copies apart; their nets can: each
/// copy's parts form a group no net leaves (KiKit names a copy's nets
/// `Board_N-...`), and the copies' groups hold the same references. Where
/// two or more such groups each have fixed parts and the boxes of those
/// do not overlap, every movable part of a copy stays in its copy's
/// region: the board's box, cut between the copy's fixed parts and every
/// other copy's, on the axis they are farther apart (halfway, moved by
/// half the difference of the board's margins beyond all copies' fixed
/// parts), and starts there (global placement begins at its centre). On the
/// 0xCB pcb-panel (four copies, one piece) 15-22 parts of one copy landed
/// on another's area. Boards whose nets form one group (most boards, and
/// panels whose copies share net names) are left alone. Returns the
/// number of copies confined.
pub(super) fn keep_copies_apart(problem: &mut core::Problem, footprints: usize) -> usize {
    if problem.pieces.len() >= 2 || footprints < 4 {
        return 0;
    }
    let mut parent: Vec<usize> = (0..footprints).collect();
    fn root(parent: &mut [usize], mut part: usize) -> usize {
        while parent[part] != part {
            parent[part] = parent[parent[part]];
            part = parent[part];
        }
        part
    }
    let mut first_on_net: BTreeMap<usize, usize> = BTreeMap::new();
    for part in 0..footprints {
        for pin in &problem.components[part].pins {
            match first_on_net.get(&pin.net) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, part), root(&mut parent, other));
                    parent[a] = b;
                }
                None => {
                    first_on_net.insert(pin.net, part);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for part in 0..footprints {
        if !problem.components[part].pins.is_empty() {
            groups
                .entry(root(&mut parent, part))
                .or_default()
                .push(part);
        }
    }
    if groups.len() < 2 {
        return 0;
    }
    // A copy: a group whose references (with pin counts) another group
    // repeats.
    let signature = |members: &[usize]| -> Vec<(String, usize)> {
        let mut parts: Vec<(String, usize)> = members
            .iter()
            .map(|part| {
                (
                    problem.components[*part].name.clone(),
                    problem.components[*part].pins.len(),
                )
            })
            .collect();
        parts.sort();
        parts
    };
    let mut kinds: BTreeMap<Vec<(String, usize)>, usize> = BTreeMap::new();
    for members in groups.values() {
        *kinds.entry(signature(members)).or_default() += 1;
    }
    let body_box = |problem: &core::Problem, part: usize| {
        let component = &problem.components[part];
        let (center, half) = (
            component.center(problem.poses[part]),
            component.half_extent(problem.poses[part].angle),
        );
        [
            center[0] - half[0],
            center[1] - half[1],
            center[0] + half[0],
            center[1] + half[1],
        ]
    };
    // Each copy's fixed parts' box, and its movable parts.
    let mut copies: Vec<([f64; 4], Vec<usize>)> = Vec::new();
    for members in groups.values() {
        if kinds[&signature(members)] < 2 {
            continue;
        }
        let mut anchor = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut movable = Vec::new();
        for &part in members {
            if problem.components[part].fixed {
                let b = body_box(problem, part);
                anchor = [
                    anchor[0].min(b[0]),
                    anchor[1].min(b[1]),
                    anchor[2].max(b[2]),
                    anchor[3].max(b[3]),
                ];
            } else {
                movable.push(part);
            }
        }
        if anchor[0].is_finite() && !movable.is_empty() {
            copies.push((anchor, movable));
        }
    }
    if copies.len() < 2 {
        return 0;
    }
    // The copies' fixed parts must keep apart: interleaved copies have no
    // region of their own.
    let gap = |a: &[f64; 4], b: &[f64; 4]| -> [f64; 2] {
        [
            (b[0] - a[2]).max(a[0] - b[2]),
            (b[1] - a[3]).max(a[1] - b[3]),
        ]
    };
    for (index, (a, _)) in copies.iter().enumerate() {
        for (b, _) in &copies[index + 1..] {
            let [x, y] = gap(a, b);
            if x <= 0.0 && y <= 0.0 {
                return 0;
            }
        }
    }
    let bounds = problem.bounds();
    let constrained: BTreeSet<usize> = problem
        .constraints
        .regions
        .iter()
        .map(|(part, _)| *part)
        .collect();
    // The copies repeat one layout, so each copy's region lies around its
    // fixed parts as the board lies around all copies' fixed parts: a cut
    // halfway between two copies' fixed parts moves by half the difference
    // of the board's margins beyond them (kept between the two). Halfway
    // alone cut SmartSpin2k's copies (fixed parts at the top of each copy,
    // 2.3 mm from the board's top edge, 40.5 mm from its bottom) through
    // their own parts: 162 of the designer's 630 movable parts lay outside
    // their copy's region, none with the shift.
    let all = copies.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, (a, _)| {
            [
                b[0].min(a[0]),
                b[1].min(a[1]),
                b[2].max(a[2]),
                b[3].max(a[3]),
            ]
        },
    );
    let shift = [
        ((bounds[2] - all[2]) - (all[0] - bounds[0])) / 2.0,
        ((bounds[3] - all[3]) - (all[1] - bounds[1])) / 2.0,
    ];
    let cut = |before: f64, after: f64, axis: usize| {
        ((before + after) / 2.0 + shift[axis]).clamp(before, after)
    };
    let mut regions = Vec::new();
    for (index, (anchor, _)) in copies.iter().enumerate() {
        let mut region = bounds;
        for (other, (theirs, _)) in copies.iter().enumerate() {
            if other == index {
                continue;
            }
            let [x, y] = gap(anchor, theirs);
            if x >= y {
                if theirs[0] >= anchor[2] {
                    region[2] = region[2].min(cut(anchor[2], theirs[0], 0));
                } else {
                    region[0] = region[0].max(cut(theirs[2], anchor[0], 0));
                }
            } else if theirs[1] >= anchor[3] {
                region[3] = region[3].min(cut(anchor[3], theirs[1], 1));
            } else {
                region[1] = region[1].max(cut(theirs[3], anchor[1], 1));
            }
        }
        regions.push(region);
    }
    // Each copy's parts start in their own region (its centre): from the
    // common start (a netlist's stack, the fixed pins' centroid) global
    // placement clamped the other copies' parts against their regions'
    // near borders, where they piled up (SmartSpin2k: 57 parts without a
    // legal place).
    for ((_, movable), region) in copies.iter().zip(&regions) {
        let centre = [(region[0] + region[2]) / 2.0, (region[1] + region[3]) / 2.0];
        for &part in movable {
            if !constrained.contains(&part) {
                problem.constraints.regions.push((part, *region));
                problem.constraints.starts.push((part, centre));
            }
        }
    }
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!(
            "copies: {} copies held apart ({} movable parts), regions {:?}",
            copies.len(),
            copies
                .iter()
                .map(|(_, movable)| movable.len())
                .sum::<usize>(),
            regions
                .iter()
                .map(|r| r.map(|v| (v * 10.0).round() / 10.0))
                .collect::<Vec<_>>()
        );
    }
    copies.len()
}

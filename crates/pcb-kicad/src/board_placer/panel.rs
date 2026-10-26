// Copyright (C) 2026 Toit contributors.

//! Panel copies (`panel_copies`): a panel holds several copies of one
//! circuit in one outline (KiKit's tabbed copies, a keyboard's repeated
//! cells, numbered sheets side by side). Designers place one copy and
//! replicate it; placing every part independently interleaves or drifts
//! the copies (docs/placer.md, "Panel copies (2026-10-26)").
//!
//! `detect` finds the copies: groups of parts with the same footprints
//! (references with pin counts, bodies and pin offsets) whose nets map one
//! to one (a net inside one copy to a net inside the other, a net several
//! copies share to itself), told apart by their net islands (KiKit names a
//! copy's nets `Board_N-...`), by numbered references (U101, U201, U301)
//! or, where references repeat, by the fixed part nearest each copy's
//! anchor. The copies' fixed parts give each copy's transform from the
//! first (a translation and a quarter turn), and each copy's region: an
//! outline piece of its own, else the board cut between the copies' fixed
//! parts, narrowed so that every copy's image of the first copy's region
//! lies inside its own; without fixed parts, slices of the board's box
//! along its longer axis.
//!
//! `place_copies` places the first copy with the ordinary placer inside
//! its region (the other copies' and the shared movable parts take no
//! room and pull no nets there), replicates the placement to every copy
//! by its transform, and places what is left with the ordinary placer:
//! the replicated parts that are not legal where they land (a copy's
//! surroundings differ: a rail, a fiducial, the board edge), each in its
//! copy's region, and the parts in no copy (shared connectors, rails);
//! the legal replicated parts stay where they are.

use super::*;
use pcb_placer as core;

/// A copy's place relative to the first: a turn by `angle` degrees (the
/// placer's convention: a pose's angle grows by it) and then `offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CopyTransform {
    pub angle: f64,
    pub offset: core::problem::Point,
}

impl CopyTransform {
    const IDENTITY: CopyTransform = CopyTransform {
        angle: 0.0,
        offset: [0.0, 0.0],
    };

    /// The transform that takes `from` to `to` (two poses of one
    /// footprint), where the turn is a quarter turn.
    fn between(from: core::Pose, to: core::Pose) -> Option<CopyTransform> {
        let turn = (to.angle - from.angle).rem_euclid(360.0);
        let quarter = (turn / 90.0).round();
        if (turn - quarter * 90.0).abs() > 0.01 {
            return None;
        }
        let angle = (quarter * 90.0).rem_euclid(360.0);
        let turned = core::problem::rotate(from.position, -angle);
        Some(CopyTransform {
            angle,
            offset: [to.position[0] - turned[0], to.position[1] - turned[1]],
        })
    }

    fn point(&self, point: core::problem::Point) -> core::problem::Point {
        let turned = core::problem::rotate(point, -self.angle);
        [turned[0] + self.offset[0], turned[1] + self.offset[1]]
    }

    fn inverse_point(&self, point: core::problem::Point) -> core::problem::Point {
        core::problem::rotate(
            [point[0] - self.offset[0], point[1] - self.offset[1]],
            self.angle,
        )
    }

    fn pose(&self, pose: core::Pose) -> core::Pose {
        core::Pose {
            position: self.point(pose.position),
            angle: normal_angle(pose.angle + self.angle),
        }
    }

    fn region(&self, region: [f64; 4]) -> [f64; 4] {
        boxed(corners(region).map(|corner| self.point(corner)))
    }

    fn inverse_region(&self, region: [f64; 4]) -> [f64; 4] {
        boxed(corners(region).map(|corner| self.inverse_point(corner)))
    }
}

fn normal_angle(angle: f64) -> f64 {
    let angle = angle.rem_euclid(360.0);
    if (360.0 - angle).abs() < 1.0e-9 {
        0.0
    } else {
        angle
    }
}

fn corners(region: [f64; 4]) -> [core::problem::Point; 4] {
    [
        [region[0], region[1]],
        [region[2], region[1]],
        [region[2], region[3]],
        [region[0], region[3]],
    ]
}

fn boxed(points: [core::problem::Point; 4]) -> [f64; 4] {
    points.iter().fold(EMPTY_BOX, |b, p| {
        [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ]
    })
}

const EMPTY_BOX: [f64; 4] = [
    f64::INFINITY,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::NEG_INFINITY,
];

fn intersect(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

fn grow(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn empty(a: [f64; 4]) -> bool {
    !(a[0] < a[2] && a[1] < a[3])
}

/// The copies a panel holds (`detect`).
#[derive(Clone, Debug)]
pub(crate) struct PanelCopies {
    /// Per copy, its parts; the r-th part of every copy is the same part
    /// of the circuit (`members[c][r]` is copy c's counterpart of
    /// `members[0][r]`).
    pub members: Vec<Vec<usize>>,
    /// Per copy, the transform from the first copy (the identity first).
    pub transforms: Vec<CopyTransform>,
    /// Per copy, its region: the first copy's region moved by the copy's
    /// transform.
    pub regions: Vec<[f64; 4]>,
    /// How the copies were told apart: `net islands`, `numbered
    /// references` or `repeated references`.
    pub detected_by: &'static str,
    /// Where the regions came from: `fixed parts`, `outline pieces` or
    /// `slices`.
    pub regions_from: &'static str,
}

impl PanelCopies {
    /// The movable parts of the copies.
    fn movable(&self, problem: &core::Problem) -> Vec<usize> {
        self.members
            .iter()
            .flatten()
            .copied()
            .filter(|part| !problem.components[*part].fixed)
            .collect()
    }
}

/// A footprint's identity as the lowering sees it: its side, its pins'
/// count and offsets, its body.
fn fingerprint(component: &core::Component) -> Vec<i64> {
    let round = |value: f64| (value * 100.0).round() as i64;
    let mut print = vec![
        component.pins.len() as i64,
        round(component.body_size[0]),
        round(component.body_size[1]),
        round(component.body_center[0]),
        round(component.body_center[1]),
        match component.side {
            core::Side::Front => 0,
            core::Side::Back => 1,
            core::Side::Both => 2,
            core::Side::Neither => 3,
        },
    ];
    for pin in &component.pins {
        print.push(round(pin.offset[0]));
        print.push(round(pin.offset[1]));
    }
    print
}

/// The parts that take part in copies: footprints (not the board's marks
/// and keepouts) with pins.
fn candidates(problem: &core::Problem, footprints: usize) -> Vec<usize> {
    (0..footprints.min(problem.components.len()))
        .filter(|part| !problem.components[*part].pins.is_empty())
        .collect()
}

type Key = (String, Vec<i64>);

fn key(problem: &core::Problem, part: usize) -> Key {
    (
        problem.components[part].name.clone(),
        fingerprint(&problem.components[part]),
    )
}

/// Copies told apart by their nets: each a group of parts no net leaves,
/// with the same references and footprints as the others.
fn by_islands(problem: &core::Problem, parts: &[usize]) -> Option<Vec<Vec<usize>>> {
    let mut parent: BTreeMap<usize, usize> = parts.iter().map(|part| (*part, *part)).collect();
    fn root(parent: &mut BTreeMap<usize, usize>, mut part: usize) -> usize {
        while parent[&part] != part {
            let up = parent[&parent[&part]];
            parent.insert(part, up);
            part = up;
        }
        part
    }
    let mut first_on_net: BTreeMap<usize, usize> = BTreeMap::new();
    for &part in parts {
        for pin in &problem.components[part].pins {
            match first_on_net.get(&pin.net) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, part), root(&mut parent, other));
                    parent.insert(a, b);
                }
                None => {
                    first_on_net.insert(pin.net, part);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &part in parts {
        groups
            .entry(root(&mut parent, part))
            .or_default()
            .push(part);
    }
    let mut kinds: BTreeMap<Vec<Key>, Vec<Vec<usize>>> = BTreeMap::new();
    for mut members in groups.into_values() {
        members.sort_by_key(|part| (key(problem, *part), *part));
        let signature: Vec<Key> = members.iter().map(|part| key(problem, *part)).collect();
        kinds.entry(signature).or_default().push(members);
    }
    best_class(kinds.into_values())
}

/// The class of equal copies with the most parts (two copies at least,
/// four parts each), its copies in board order.
fn best_class(classes: impl Iterator<Item = Vec<Vec<usize>>>) -> Option<Vec<Vec<usize>>> {
    let mut best = classes
        .filter(|copies| copies.len() >= 2 && copies[0].len() >= 4)
        .max_by_key(|copies| (copies.len() * copies[0].len(), copies.len()))?;
    best.sort_by_key(|members| members.iter().min().copied());
    Some(best)
}

/// Copies told apart by numbered references: U101, C110 in the first,
/// U201, C210 in the second (the hundreds are the copy). Sheets of
/// variants (OpenRX's panel: one with a U.FL connector where another has
/// a chip antenna) are copies in the parts they share, where those are
/// nine in ten of each; the others are parts in no copy.
fn by_numbers(problem: &core::Problem, parts: &[usize]) -> Option<Vec<Vec<usize>>> {
    type NumberKey = (String, u64, Vec<i64>);
    let mut copies: BTreeMap<u64, BTreeMap<NumberKey, usize>> = BTreeMap::new();
    for &part in parts {
        let name = &problem.components[part].name;
        let digits = name.len() - name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        if digits < 3 || digits == name.len() {
            continue;
        }
        let (prefix, number) = name.split_at(name.len() - digits);
        let Ok(number) = number.parse::<u64>() else {
            continue;
        };
        if number < 100 {
            continue;
        }
        let key = (
            prefix.to_string(),
            number % 100,
            fingerprint(&problem.components[part]),
        );
        copies.entry(number / 100).or_default().insert(key, part);
    }
    let copies: Vec<BTreeMap<NumberKey, usize>> = copies
        .into_values()
        .filter(|members| members.len() >= 4)
        .collect();
    // Around each copy, the copies sharing nine in ten of its parts and
    // of their own; the parts all of them share.
    let mut best: Option<Vec<Vec<usize>>> = None;
    for base in &copies {
        let alike: Vec<&BTreeMap<NumberKey, usize>> = copies
            .iter()
            .filter(|other| {
                let shared = base.keys().filter(|key| other.contains_key(*key)).count();
                shared * 10 >= base.len() * 9 && shared * 10 >= other.len() * 9
            })
            .collect();
        let common: Vec<&NumberKey> = base
            .keys()
            .filter(|key| alike.iter().all(|other| other.contains_key(*key)))
            .collect();
        if alike.len() < 2 || common.len() < 4 {
            continue;
        }
        let members: Vec<Vec<usize>> = alike
            .iter()
            .map(|copy| common.iter().map(|key| copy[*key]).collect())
            .collect();
        let size = |copies: &Vec<Vec<usize>>| (copies.len() * copies[0].len(), copies.len());
        if best.as_ref().is_none_or(|best| size(&members) > size(best)) {
            best = Some(members);
        }
    }
    let mut best = best?;
    best.sort_by_key(|members| members.iter().min().copied());
    Some(best)
}

/// Copies whose references repeat (a keyboard's cells, a panel copied
/// in the editor): the references every copy has (the count most parts
/// repeat), each instance in the copy of the anchor its fixed instance
/// lies nearest (the anchor: the largest footprint whose instances are all
/// fixed), the movable instances in turn (their nets do not tell copies
/// apart where they are shared; where they do, the isomorphism check
/// refuses the copies).
fn by_repeats(problem: &core::Problem, parts: &[usize]) -> Option<Vec<Vec<usize>>> {
    let mut classes: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
    for &part in parts {
        classes.entry(key(problem, part)).or_default().push(part);
    }
    // The count most parts repeat.
    let mut by_count: BTreeMap<usize, usize> = BTreeMap::new();
    for instances in classes.values() {
        if instances.len() >= 2 {
            *by_count.entry(instances.len()).or_default() += instances.len();
        }
    }
    let (&count, _) = by_count
        .iter()
        .max_by_key(|(count, parts)| (**parts, **count))?;
    let classes: Vec<Vec<usize>> = classes
        .into_values()
        .filter(|instances| instances.len() == count)
        .collect();
    let area = |part: usize| {
        let size = problem.components[part].body_size;
        size[0] * size[1]
    };
    let anchor_class = classes
        .iter()
        .filter(|instances| instances.iter().all(|part| problem.components[*part].fixed))
        .max_by(|a, b| area(a[0]).total_cmp(&area(b[0])))?;
    let centre = |part: usize| problem.components[part].center(problem.poses[part]);
    let anchors: Vec<core::problem::Point> =
        anchor_class.iter().map(|part| centre(*part)).collect();
    let nearest = |part: usize| {
        let at = centre(part);
        (0..anchors.len())
            .min_by(|a, b| {
                distance_squared(at, anchors[*a]).total_cmp(&distance_squared(at, anchors[*b]))
            })
            .expect("anchors")
    };
    let mut copies: Vec<Vec<usize>> = vec![Vec::new(); count];
    for instances in &classes {
        let mut taken: Vec<Option<usize>> = vec![None; count];
        let mut clash = false;
        for &part in instances {
            if problem.components[part].fixed {
                let copy = nearest(part);
                if taken[copy].is_some() {
                    clash = true;
                    break;
                }
                taken[copy] = Some(part);
            }
        }
        if clash {
            continue;
        }
        let mut free = instances
            .iter()
            .copied()
            .filter(|part| !problem.components[*part].fixed);
        for slot in taken.iter_mut() {
            if slot.is_none() {
                *slot = free.next();
            }
        }
        for (copy, part) in taken.into_iter().enumerate() {
            copies[copy].push(part.expect("one instance per copy"));
        }
    }
    (copies[0].len() >= 4).then_some(copies)
}

/// Whether the copies' nets map one to one onto the first copy's: a net
/// inside the first copy onto one inside the copy, a net that reaches
/// beyond the first copy onto itself; 95 % of the pins at least.
fn isomorphic(problem: &core::Problem, members: &[Vec<usize>]) -> bool {
    let copy_of: BTreeMap<usize, usize> = members
        .iter()
        .enumerate()
        .flat_map(|(copy, parts)| parts.iter().map(move |part| (*part, copy)))
        .collect();
    // Per net, the copies (or none: a part in no copy) it reaches.
    let mut reach: BTreeMap<usize, BTreeSet<Option<usize>>> = BTreeMap::new();
    for (part, component) in problem.components.iter().enumerate() {
        for pin in &component.pins {
            reach
                .entry(pin.net)
                .or_default()
                .insert(copy_of.get(&part).copied());
        }
    }
    let inside =
        |net: usize, copy: usize| reach[&net].len() == 1 && reach[&net].contains(&Some(copy));
    let (mut pins, mut wrong) = (0usize, 0usize);
    for (copy, parts) in members.iter().enumerate().skip(1) {
        let mut map: BTreeMap<usize, usize> = BTreeMap::new();
        let mut image: BTreeMap<usize, usize> = BTreeMap::new();
        for (rank, &part) in parts.iter().enumerate() {
            let first = &problem.components[members[0][rank]];
            let this = &problem.components[part];
            for (a, b) in first.pins.iter().zip(&this.pins) {
                pins += 1;
                let fits = if inside(a.net, 0) {
                    inside(b.net, copy)
                } else {
                    a.net == b.net
                };
                let consistent = *map.entry(a.net).or_insert(b.net) == b.net
                    && *image.entry(b.net).or_insert(a.net) == a.net;
                if !fits || !consistent {
                    wrong += 1;
                }
            }
        }
    }
    pins > 0 && wrong * 20 <= pins
}

/// Copy `copy`'s transform from the fixed parts both copies hold: of the
/// three largest as anchors, the one that the most others agree with
/// (four in five at least).
fn transform_of(
    problem: &core::Problem,
    members: &[Vec<usize>],
    copy: usize,
) -> Option<CopyTransform> {
    let pairs: Vec<(usize, usize)> = members[0]
        .iter()
        .zip(&members[copy])
        .map(|(a, b)| (*a, *b))
        .filter(|(a, b)| problem.components[*a].fixed && problem.components[*b].fixed)
        .collect();
    if pairs.is_empty() {
        return None;
    }
    let area = |part: usize| {
        let size = problem.components[part].body_size;
        size[0] * size[1]
    };
    let mut anchors = pairs.clone();
    anchors.sort_by(|a, b| area(b.0).total_cmp(&area(a.0)));
    let agree = |transform: &CopyTransform| {
        pairs
            .iter()
            .filter(|(a, b)| {
                let image = transform.pose(problem.poses[*a]);
                let pose = problem.poses[*b];
                distance_squared(image.position, pose.position) < 0.05 * 0.05
                    && ((normal_angle(image.angle) - normal_angle(pose.angle)).abs() < 0.01
                        || (normal_angle(image.angle) - normal_angle(pose.angle)).abs() > 359.99)
            })
            .count()
    };
    let (transform, agreeing) = anchors
        .iter()
        .take(3)
        .filter_map(|(a, b)| CopyTransform::between(problem.poses[*a], problem.poses[*b]))
        .map(|transform| (transform, agree(&transform)))
        .max_by_key(|(_, agreeing)| *agreeing)?;
    (agreeing * 5 >= pairs.len() * 4).then_some(transform)
}

/// The box of parts' bodies.
fn bodies_box(problem: &core::Problem, parts: impl Iterator<Item = usize>) -> [f64; 4] {
    parts.fold(EMPTY_BOX, |b, part| {
        let component = &problem.components[part];
        let pose = problem.poses[part];
        let (center, half) = (component.center(pose), component.half_extent(pose.angle));
        grow(
            b,
            [
                center[0] - half[0],
                center[1] - half[1],
                center[0] + half[0],
                center[1] + half[1],
            ],
        )
    })
}

/// Each copy's region from its fixed parts' box (`anchors`): an outline
/// piece of its own where the board has pieces, else the board's box cut
/// between the copy's fixed parts and every other copy's (and the fixed
/// parts in no copy, where they keep apart from it), on the axis they are
/// farther apart, as `keep_copies_apart` cuts.
fn cut_regions(
    problem: &core::Problem,
    anchors: &[[f64; 4]],
    others: Option<[f64; 4]>,
) -> Option<(Vec<[f64; 4]>, &'static str)> {
    if problem.pieces.len() >= 2 {
        let pieces: Vec<Option<usize>> = anchors
            .iter()
            .map(|anchor| {
                let centre = [(anchor[0] + anchor[2]) / 2.0, (anchor[1] + anchor[3]) / 2.0];
                problem.pieces.iter().position(|piece| {
                    centre[0] >= piece[0]
                        && centre[0] <= piece[2]
                        && centre[1] >= piece[1]
                        && centre[1] <= piece[3]
                })
            })
            .collect();
        let distinct: BTreeSet<usize> = pieces.iter().flatten().copied().collect();
        if distinct.len() == anchors.len() && pieces.iter().all(Option::is_some) {
            return Some((
                pieces
                    .iter()
                    .map(|piece| problem.pieces[piece.expect("piece")])
                    .collect(),
                "outline pieces",
            ));
        }
    }
    let gap = |a: &[f64; 4], b: &[f64; 4]| -> [f64; 2] {
        [
            (b[0] - a[2]).max(a[0] - b[2]),
            (b[1] - a[3]).max(a[1] - b[3]),
        ]
    };
    for (index, a) in anchors.iter().enumerate() {
        for b in &anchors[index + 1..] {
            let [x, y] = gap(a, b);
            if x <= 0.0 && y <= 0.0 {
                return None;
            }
        }
    }
    let bounds = problem.bounds();
    // The copies repeat one layout, so each copy's region lies around its
    // fixed parts as the board lies around all of them: a cut halfway
    // between two copies' fixed parts moves by half the difference of the
    // board's margins beyond them (`keep_copies_apart`).
    let all = anchors.iter().fold(EMPTY_BOX, |b, a| grow(b, *a));
    let shift = [
        ((bounds[2] - all[2]) - (all[0] - bounds[0])) / 2.0,
        ((bounds[3] - all[3]) - (all[1] - bounds[1])) / 2.0,
    ];
    let between = |before: f64, after: f64, axis: usize| {
        ((before + after) / 2.0 + shift[axis]).clamp(before, after)
    };
    let cut = |region: &mut [f64; 4], anchor: &[f64; 4], theirs: &[f64; 4]| {
        let [x, y] = gap(anchor, theirs);
        if x >= y {
            if theirs[0] >= anchor[2] {
                region[2] = region[2].min(between(anchor[2], theirs[0], 0));
            } else {
                region[0] = region[0].max(between(theirs[2], anchor[0], 0));
            }
        } else if theirs[1] >= anchor[3] {
            region[3] = region[3].min(between(anchor[3], theirs[1], 1));
        } else {
            region[1] = region[1].max(between(theirs[3], anchor[1], 1));
        }
    };
    let mut regions = Vec::new();
    for (index, anchor) in anchors.iter().enumerate() {
        let mut region = bounds;
        for (other, theirs) in anchors.iter().enumerate() {
            if other != index {
                cut(&mut region, anchor, theirs);
            }
        }
        if let Some(theirs) = others {
            let [x, y] = gap(anchor, &theirs);
            if x > 0.0 || y > 0.0 {
                cut(&mut region, anchor, &theirs);
            }
        }
        regions.push(region);
    }
    Some((regions, "fixed parts"))
}

/// The copies of a panel, or why there are none (for the debug line).
pub(crate) fn detect(problem: &core::Problem, footprints: usize) -> Result<PanelCopies, String> {
    let parts = candidates(problem, footprints);
    let mut tried = Vec::new();
    for (detected_by, found) in [
        ("net islands", by_islands(problem, &parts)),
        ("numbered references", by_numbers(problem, &parts)),
        ("repeated references", by_repeats(problem, &parts)),
    ] {
        let Some(members) = found else {
            tried.push(format!("{detected_by}: none"));
            continue;
        };
        match copies_from(problem, members, detected_by) {
            Ok(copies) => return Ok(copies),
            Err(why) => tried.push(format!("{detected_by}: {why}")),
        }
    }
    Err(tried.join("; "))
}

fn copies_from(
    problem: &core::Problem,
    members: Vec<Vec<usize>>,
    detected_by: &'static str,
) -> Result<PanelCopies, String> {
    let count = members.len();
    let movable = members
        .iter()
        .flatten()
        .filter(|part| !problem.components[**part].fixed)
        .count();
    if movable < count {
        return Err(format!("{movable} movable parts in {count} copies"));
    }
    if !isomorphic(problem, &members) {
        return Err("the nets do not map one to one".into());
    }
    let fixed_anywhere = members
        .iter()
        .flatten()
        .any(|part| problem.components[*part].fixed);
    // Copies that share nets and hold no fixed part need not lie apart:
    // an LED matrix numbered D101-D199 and D201-D299 is one circuit, not a
    // panel (PixelWave). Without fixed parts only copies no net joins
    // (KiKit's) are taken to be a panel.
    if !fixed_anywhere && detected_by != "net islands" {
        return Err("no fixed parts say where the copies lie".into());
    }
    let bounds = problem.bounds();
    let (transforms, regions, regions_from) = if fixed_anywhere {
        let mut transforms = vec![CopyTransform::IDENTITY];
        for copy in 1..count {
            transforms.push(
                transform_of(problem, &members, copy)
                    .ok_or_else(|| format!("copy {copy}'s fixed parts give no transform"))?,
            );
        }
        let anchors: Vec<[f64; 4]> = members
            .iter()
            .map(|parts| {
                bodies_box(
                    problem,
                    parts
                        .iter()
                        .copied()
                        .filter(|part| problem.components[*part].fixed),
                )
            })
            .collect();
        let in_copies: BTreeSet<usize> = members.iter().flatten().copied().collect();
        let others = bodies_box(
            problem,
            (0..problem.components.len()).filter(|part| {
                let component = &problem.components[*part];
                component.fixed
                    && !component.pins.is_empty()
                    && !component.copper_only
                    && !in_copies.contains(part)
            }),
        );
        let (cuts, from) = cut_regions(problem, &anchors, (!empty(others)).then_some(others))
            .ok_or("the copies' fixed parts overlap")?;
        // The first copy's region: where every copy's image of it lies in
        // that copy's own region.
        let mut first = cuts[0];
        for copy in 1..count {
            first = intersect(first, transforms[copy].inverse_region(cuts[copy]));
        }
        if empty(first) {
            return Err("the copies' regions do not match".into());
        }
        let regions: Vec<[f64; 4]> = transforms.iter().map(|t| t.region(first)).collect();
        (transforms, regions, from)
    } else {
        // No fixed part says where the copies go: slices of the board's
        // box along its longer axis.
        let axis = if bounds[2] - bounds[0] >= bounds[3] - bounds[1] {
            0
        } else {
            1
        };
        let width = (bounds[axis + 2] - bounds[axis]) / count as f64;
        let mut first = bounds;
        first[axis + 2] = bounds[axis] + width;
        let transforms: Vec<CopyTransform> = (0..count)
            .map(|copy| {
                let mut offset = [0.0, 0.0];
                offset[axis] = copy as f64 * width;
                CopyTransform { angle: 0.0, offset }
            })
            .collect();
        let regions = transforms.iter().map(|t| t.region(first)).collect();
        (transforms, regions, "slices")
    };
    // The regions keep apart.
    for (index, a) in regions.iter().enumerate() {
        for b in &regions[index + 1..] {
            let overlap = intersect(*a, *b);
            if (overlap[2] - overlap[0]) > 1.0e-6 && (overlap[3] - overlap[1]) > 1.0e-6 {
                return Err("the copies' regions overlap".into());
            }
        }
    }
    Ok(PanelCopies {
        members,
        transforms,
        regions,
        detected_by,
        regions_from,
    })
}

/// `polygon` clipped to `region` (Sutherland and Hodgman).
fn clip(polygon: &[core::problem::Point], region: [f64; 4]) -> Vec<core::problem::Point> {
    let mut points = polygon.to_vec();
    for (axis, bound, keep_below) in [
        (0, region[0], false),
        (0, region[2], true),
        (1, region[1], false),
        (1, region[3], true),
    ] {
        if points.is_empty() {
            break;
        }
        let inside = |point: &core::problem::Point| {
            if keep_below {
                point[axis] <= bound
            } else {
                point[axis] >= bound
            }
        };
        let crossing = |a: core::problem::Point, b: core::problem::Point| {
            let t = (bound - a[axis]) / (b[axis] - a[axis]);
            let mut point = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            point[axis] = bound;
            point
        };
        let mut clipped = Vec::new();
        for index in 0..points.len() {
            let (current, next) = (points[index], points[(index + 1) % points.len()]);
            match (inside(&current), inside(&next)) {
                (true, true) => clipped.push(next),
                (true, false) => clipped.push(crossing(current, next)),
                (false, true) => {
                    clipped.push(crossing(current, next));
                    clipped.push(next);
                }
                (false, false) => {}
            }
        }
        points = clipped;
    }
    points
}

/// A part that takes no room and pulls no net.
fn make_inert(component: &mut core::Component) {
    component.fixed = true;
    component.side = core::Side::Neither;
    component.pins.clear();
    component.far_side.clear();
    component.far_copper.clear();
    component.far_courtyard.clear();
    component.hollow.clear();
    component.pads.clear();
    component.courtyards.clear();
    component.tight_hollow.clear();
    component.hole_rings.clear();
    component.copper_only = false;
    component.cutout_outline.clear();
    component.halo = 0.0;
}

/// The first copy's problem: its outline is the board's inside the first
/// copy's region; its movable parts are the first copy's, held in that
/// region; the other movable parts take no room and pull no net; fixed
/// parts away from the region keep their bodies but pull no net (the other
/// copies' pins of a shared net would pull the copy across the board).
fn reference_problem(problem: &core::Problem, copies: &PanelCopies) -> core::Problem {
    let region = copies.regions[0];
    let active: BTreeSet<usize> = copies.members[0]
        .iter()
        .copied()
        .filter(|part| !problem.components[*part].fixed)
        .collect();
    let first: BTreeSet<usize> = copies.members[0].iter().copied().collect();
    let mut sub = problem.clone();
    for (index, component) in sub.components.iter_mut().enumerate() {
        if active.contains(&index) {
            continue;
        }
        if !component.fixed {
            make_inert(component);
            continue;
        }
        if !first.contains(&index) && !component.pins.is_empty() {
            let pose = problem.poses[index];
            let (center, half) = (component.center(pose), component.half_extent(pose.angle));
            let body = [
                center[0] - half[0],
                center[1] - half[1],
                center[0] + half[0],
                center[1] + half[1],
            ];
            if empty(intersect(body, region)) {
                component.pins.clear();
            }
        }
    }
    let pulls = |part: usize| active.contains(&part) || !sub.components[part].pins.is_empty();
    let pulling: Vec<bool> = (0..sub.components.len()).map(pulls).collect();
    sub.outline = clip(&problem.outline, region);
    sub.pieces.clear();
    let constraints = &mut sub.constraints;
    constraints
        .regions
        .retain(|(part, _)| active.contains(part));
    constraints
        .starts
        .retain(|(part, _)| !active.contains(part));
    constraints
        .edges
        .retain(|(part, _, _)| active.contains(part));
    constraints
        .overhangs
        .retain(|(part, _, _)| active.contains(part));
    constraints.kept.retain(|kept| active.contains(&kept.part));
    constraints
        .followers
        .retain(|follower| active.contains(&follower.leader));
    // Relations among the parts that still count; the automatic ones are
    // the last.
    let user = constraints.relations.len() - constraints.automatic;
    let mut relations = Vec::new();
    let mut automatic = 0;
    for (index, relation) in constraints.relations.drain(..).enumerate() {
        let relation = match relation {
            core::constraints::Relation::NearAny { part, anchors, max } => {
                let anchors: Vec<_> = anchors
                    .into_iter()
                    .filter(|anchor| pulling[anchor.part()])
                    .collect();
                if anchors.is_empty() || !active.contains(&part) {
                    continue;
                }
                core::constraints::Relation::NearAny { part, anchors, max }
            }
            other => {
                let parts = other.parts();
                if !parts.iter().flatten().all(|part| pulling[*part])
                    || !parts.iter().flatten().any(|part| active.contains(part))
                {
                    continue;
                }
                other
            }
        };
        if index >= user {
            automatic += 1;
        }
        relations.push(relation);
    }
    constraints.relations = relations;
    constraints.automatic = automatic;
    constraints.linked.clear();
    constraints.linked_partners.clear();
    constraints.link_pairs();
    let centre = [(region[0] + region[2]) / 2.0, (region[1] + region[3]) / 2.0];
    for &part in &active {
        constraints.regions.push((part, region));
        constraints.starts.push((part, centre));
    }
    // The measured escape correction routes the whole board: the other
    // copies' parts would lie in a heap where the stack was.
    if let Some(escape) = &sub.escape
        && escape.measured.is_some()
    {
        let mut estimate = (**escape).clone();
        estimate.measured = None;
        sub.escape = Some(std::sync::Arc::new(estimate));
    }
    sub
}

/// The looser of two relaxations, field by field.
fn loosest(a: &core::Relaxation, b: &core::Relaxation) -> core::Relaxation {
    let mut on_marks: Vec<usize> = a.on_marks.iter().chain(&b.on_marks).copied().collect();
    on_marks.sort_unstable();
    on_marks.dedup();
    core::Relaxation {
        spacing: a.spacing.min(b.spacing),
        grid: a.grid.min(b.grid),
        halo_scale: a.halo_scale.min(b.halo_scale),
        small_halo_scale: a.small_halo_scale.min(b.small_halo_scale),
        edge_inset: a.edge_inset || b.edge_inset,
        tight: a.tight || b.tight,
        edge_copper: a.edge_copper || b.edge_copper,
        edge_rule: a.edge_rule.min(b.edge_rule),
        courtyard_spacing: a.courtyard_spacing || b.courtyard_spacing,
        pad_clearance: a.pad_clearance || b.pad_clearance,
        halos: a.halos.clone().or_else(|| b.halos.clone()),
        on_marks,
    }
}

/// The first copy's poses (the copies' movable parts' only) moved to every
/// copy.
fn replicate(
    problem: &core::Problem,
    copies: &PanelCopies,
    from: &[core::Pose],
) -> Vec<core::Pose> {
    let mut poses = problem.poses.clone();
    for (copy, parts) in copies.members.iter().enumerate() {
        for (rank, &part) in parts.iter().enumerate() {
            if !problem.components[part].fixed {
                poses[part] = copies.transforms[copy].pose(from[copies.members[0][rank]]);
            }
        }
    }
    problem.sync_followers(&mut poses);
    poses
}

/// The share of the copies' movable parts above which replicated parts
/// not legal where they land send the board to a placement as a whole.
const MOST_ILLEGAL_SHARE: f64 = 0.1;

/// The board placed as a whole, with the seed's whole budget (the first
/// copy's work spent on top of it; the copies' placement is not used).
fn place_whole(
    problem: &core::Problem,
    config: &core::Config,
    started: u64,
    why: &str,
) -> core::Placement {
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!("panel copies: {why}: placed as a whole");
    }
    let mut whole = core::place(problem, config);
    whole.work = core::work() - started;
    whole
}

/// Places a panel by its copies (the module's comment): one seed's
/// placement of the whole board.
pub(crate) fn place_copies(
    problem: &core::Problem,
    copies: &PanelCopies,
    config: &core::Config,
) -> core::Placement {
    let debug = std::env::var_os("PCB_PLACER_DEBUG").is_some();
    let started = core::work();
    // The first copy, with three quarters of the budget.
    let sub = reference_problem(problem, copies);
    let mut first_config = config.clone();
    first_config.work_limit = config.work_limit.map(|limit| limit / 4 * 3);
    let first = core::place(&sub, &first_config);
    // Even the tightest bodies of one copy need more area than its region
    // has: no replication seats them; the board is placed as a whole.
    if first.overfull {
        return place_whole(
            problem,
            config,
            started,
            "the first copy does not fit its region",
        );
    }
    let replicated = replicate(problem, copies, &first.poses);
    // The replicated parts that are not legal where they land, at the
    // first copy's level (the parts in no copy are placed after).
    // The first copy's parts that lie on the board's marks (having found
    // no room off them) lie on their copies' marks in every copy.
    let mut first_relaxation = first.relaxation.clone();
    let rank_in_first: BTreeMap<usize, usize> = copies.members[0]
        .iter()
        .enumerate()
        .map(|(rank, part)| (*part, rank))
        .collect();
    for part in &first.relaxation.on_marks {
        if let Some(rank) = rank_in_first.get(part) {
            for members in &copies.members[1..] {
                first_relaxation.on_marks.push(members[*rank]);
            }
        }
    }
    first_relaxation.on_marks.sort_unstable();
    first_relaxation.on_marks.dedup();
    let mut check = problem.clone();
    first_relaxation.apply(&mut check);
    let in_copies: BTreeSet<usize> = copies.members.iter().flatten().copied().collect();
    let shared: Vec<usize> = (0..problem.components.len())
        .filter(|part| !problem.components[*part].fixed && !in_copies.contains(part))
        .collect();
    let shared_set: BTreeSet<usize> = shared.iter().copied().collect();
    let others: Vec<usize> = (0..problem.components.len())
        .filter(|part| !shared_set.contains(part))
        .collect();
    let first_unplaced: BTreeSet<usize> = first.unplaced.iter().copied().collect();
    let copy_of: BTreeMap<usize, (usize, usize)> = copies
        .members
        .iter()
        .enumerate()
        .flat_map(|(copy, parts)| {
            parts
                .iter()
                .enumerate()
                .map(move |(rank, part)| (*part, (copy, rank)))
        })
        .collect();
    let illegal: Vec<usize> = copies
        .movable(problem)
        .into_iter()
        .filter(|part| {
            let (_, rank) = copy_of[part];
            let pose = replicated[*part];
            let options = &problem.components[*part].angle_options;
            let turn_allowed = options.is_empty()
                || options.iter().any(|option| {
                    let difference = (normal_angle(*option) - pose.angle).abs();
                    difference < 0.01 || difference > 359.99
                });
            first_unplaced.contains(&copies.members[0][rank])
                || !turn_allowed
                || !core::legal::is_legal(&check, &replicated, *part, pose, others.iter().copied())
        })
        .collect();
    if debug {
        // Why the replicated parts are not legal: the first few.
        for &part in illegal.iter().take(12) {
            let pose = replicated[part];
            let mut met: Vec<String> = Vec::new();
            if !core::legal::is_legal(&check, &replicated, part, pose, std::iter::empty()) {
                met.push("outline or hard constraint".into());
            }
            for &other in &others {
                if other != part
                    && let Some(kind) =
                        core::legal::conflict_kind(&check, &replicated, part, pose, other)
                {
                    met.push(format!("{} {kind}", check.components[other].name));
                }
            }
            eprintln!(
                "panel copies: {} (copy {}) not legal: {}",
                check.components[part].name,
                copy_of[&part].0,
                met.join(", ")
            );
        }
        eprintln!(
            "panel copies: first copy placed ({} unplaced, wirelength {:.0}), {} of {} replicated parts not legal, {} parts in no copy",
            first.unplaced.len(),
            first.wirelength_final,
            illegal.len(),
            copies.movable(problem).len(),
            shared.len()
        );
    }
    // Many replicated parts not legal where they land: the copies'
    // surroundings differ (the molecules' cells: an edge constraint on the
    // outer cells' connectors, the board's marks; 37 of 144 parts of the
    // 4x4, which then left 55 parts without a place against 31 placed as
    // a whole): the board is placed as a whole.
    if illegal.len() as f64 > MOST_ILLEGAL_SHARE * copies.movable(problem).len() as f64 {
        return place_whole(
            problem,
            config,
            started,
            "many replicated parts are not legal where they land",
        );
    }
    let (poses, relaxation, unplaced, overfull, rest_work) =
        if illegal.is_empty() && shared.is_empty() {
            (
                replicated.clone(),
                first_relaxation.clone(),
                Vec::new(),
                false,
                0,
            )
        } else {
            // The rest with the ordinary placer: the legal replicated parts
            // stay (fixed), the others start where their copy put them, each
            // in its copy's region.
            let illegal_set: BTreeSet<usize> = illegal.iter().copied().collect();
            let mut rest = problem.clone();
            rest.poses = replicated.clone();
            for part in copies.movable(problem) {
                if !illegal_set.contains(&part) {
                    rest.components[part].fixed = true;
                }
            }
            rest.constraints
                .starts
                .retain(|(part, _)| !illegal_set.contains(part));
            for &part in &illegal {
                let (copy, _) = copy_of[&part];
                rest.constraints.regions.push((part, copies.regions[copy]));
                let centre = rest.components[part].center(replicated[part]);
                rest.constraints.starts.push((part, centre));
            }
            let mut rest_config = config.clone();
            let used = core::work() - started;
            rest_config.work_limit = config
                .work_limit
                .map(|limit| limit.saturating_sub(used).max(limit / 10));
            let mark = core::work();
            let placed = core::place(&rest, &rest_config);
            let relaxation = loosest(&first_relaxation, &placed.relaxation);
            let unplaced = placed.unplaced.clone();
            (
                placed.poses,
                relaxation,
                unplaced,
                placed.overfull,
                core::work() - mark,
            )
        };
    let mut fin = problem.clone();
    relaxation.apply(&mut fin);
    let illegal_final = core::legal::illegal_components(&fin, &poses);
    let constraints = core::constraints::report(&fin, &poses);
    let wirelength_final = problem.wirelength(&poses);
    if debug {
        eprintln!(
            "panel copies: the rest placed with {:.1} s of work, {} unplaced, {} illegal, wirelength {:.0}",
            rest_work as f64 / core::work_per_second(),
            unplaced.len(),
            illegal_final.len(),
            wirelength_final
        );
    }
    let global = first
        .stage_poses
        .iter()
        .find(|(stage, _)| *stage == "global")
        .map(|(_, poses)| problem.wirelength(&replicate(problem, copies, poses)))
        .unwrap_or(first.wirelength_global);
    let mut frames = first.frames.clone();
    frames.push(core::global::Frame {
        iteration: first.global_iterations + 3,
        overflow: first.global_overflow,
        wirelength: wirelength_final,
        poses: poses.clone(),
    });
    core::Placement {
        frames,
        global_iterations: first.global_iterations,
        global_overflow: first.global_overflow,
        wirelength_initial: problem.wirelength(&problem.poses),
        wirelength_global: global,
        wirelength_legal: problem.wirelength(&replicated),
        wirelength_final,
        unplaced,
        illegal: illegal_final,
        constraints,
        relaxation,
        work: core::work() - started,
        stage_poses: vec![("replicated", replicated), ("final", poses.clone())],
        routing_room_failed: first.routing_room_failed,
        overfull: first.overfull || overfull,
        poses,
    }
}

/// The copies as `board-placer.json` and the verdict say them
/// (`panel_copies`).
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadPanelCopiesReport {
    /// How many copies.
    pub count: usize,
    /// Parts per copy (those with pins), and of them the movable ones.
    pub parts_per_copy: usize,
    pub movable_per_copy: usize,
    /// `net islands`, `numbered references` or `repeated references`.
    pub detected_by: String,
    /// `fixed parts`, `outline pieces` or `slices`.
    pub regions_from: String,
    /// Whether the placement kept was replicated from the first copy
    /// (false: the first copy did not fit its region even with the
    /// tightest bodies, and the board was placed as a whole).
    pub replicated: bool,
    /// Movable parts in no copy, placed after the copies.
    pub shared_parts: usize,
    /// Movable parts of the copies the placement kept that do not lie
    /// where the first copy's placement put them (not legal there, or
    /// moved by the placement of the rest).
    pub moved_after_replication: usize,
    /// Each copy: its region, its turn and offset from the first, and its
    /// parts' references in the first copy's order (the r-th reference of
    /// every copy is the same part of the circuit).
    pub copies: Vec<KiCadPanelCopy>,
}

#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadPanelCopy {
    pub region: [f64; 4],
    pub rotation: f64,
    pub offset: [f64; 2],
    pub references: Vec<String>,
}

impl KiCadPanelCopiesReport {
    pub(crate) fn of(
        problem: &core::Problem,
        copies: &PanelCopies,
        placement: &core::Placement,
    ) -> KiCadPanelCopiesReport {
        let round = |value: f64| (value * 1000.0).round() / 1000.0;
        let replicated = placement
            .stage_poses
            .iter()
            .find(|(stage, _)| *stage == "replicated")
            .map(|(_, poses)| poses);
        let movable = copies.movable(problem);
        let moved = replicated.map_or(0, |replicated| {
            movable
                .iter()
                .filter(|part| {
                    let (a, b) = (replicated[**part], placement.poses[**part]);
                    distance_squared(a.position, b.position) > 1.0e-8
                        || (normal_angle(a.angle) - normal_angle(b.angle)).abs() > 1.0e-6
                })
                .count()
        });
        let in_copies: BTreeSet<usize> = copies.members.iter().flatten().copied().collect();
        KiCadPanelCopiesReport {
            replicated: replicated.is_some(),
            count: copies.members.len(),
            parts_per_copy: copies.members[0].len(),
            movable_per_copy: movable.len() / copies.members.len(),
            detected_by: copies.detected_by.into(),
            regions_from: copies.regions_from.into(),
            shared_parts: (0..problem.components.len())
                .filter(|part| !problem.components[*part].fixed && !in_copies.contains(part))
                .count(),
            moved_after_replication: moved,
            copies: copies
                .members
                .iter()
                .zip(&copies.transforms)
                .zip(&copies.regions)
                .map(|((parts, transform), region)| KiCadPanelCopy {
                    region: region.map(round),
                    rotation: transform.angle,
                    offset: transform.offset.map(round),
                    references: parts
                        .iter()
                        .map(|part| problem.components[*part].name.clone())
                        .collect(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(
        name: &str,
        at: [f64; 2],
        fixed: bool,
        nets: &[usize],
    ) -> (core::Component, core::Pose) {
        (
            core::Component {
                name: name.into(),
                body_center: [0.0, 0.0],
                body_size: [2.0, 1.0],
                round: false,
                halo: 0.0,
                pins: nets
                    .iter()
                    .enumerate()
                    .map(|(index, net)| core::Pin {
                        offset: [index as f64 - 0.5, 0.0],
                        net: *net,
                    })
                    .collect(),
                side: core::Side::Front,
                fixed,
                angle_options: vec![0.0, 90.0, 180.0, 270.0],
                far_side: Vec::new(),
                far_copper: Vec::new(),
                far_courtyard: Vec::new(),
                hollow: Vec::new(),
                tight: None,
                edge_inset: 0.0,
                courtyards: Vec::new(),
                holes_inside: false,
                pads: Vec::new(),
                copper_only: false,
                cutout_outline: Vec::new(),
                tight_hollow: Vec::new(),
                hole_rings: Vec::new(),
            },
            core::Pose {
                position: at,
                angle: 0.0,
            },
        )
    }

    /// Two copies side by side (the second turned half a turn), each a
    /// fixed J1 and four movable parts on the copy's own nets.
    fn panel(shared_nets: bool) -> core::Problem {
        let mut parts = Vec::new();
        for copy in 0..2usize {
            let net = |n: usize| if shared_nets { n } else { n + copy * 10 };
            let (x, angle) = if copy == 0 {
                (10.0, 0.0)
            } else {
                (50.0, 180.0)
            };
            let (mut component, mut pose) = part("J1", [x, 10.0], true, &[net(0), net(1)]);
            pose.angle = angle;
            component.body_size = [4.0, 2.0];
            parts.push((component, pose));
            for (index, name) in ["R1", "R2", "C1", "C2"].iter().enumerate() {
                parts.push(part(
                    name,
                    [30.0, 30.0],
                    false,
                    &[net(index % 2), net(2 + index)],
                ));
            }
        }
        let (components, poses): (Vec<_>, Vec<_>) = parts.into_iter().unzip();
        core::Problem {
            outline: vec![[0.0, 0.0], [60.0, 0.0], [60.0, 20.0], [0.0, 20.0]],
            net_weights: vec![1.0; 20],
            components,
            poses,
            spacing: 0.5,
            grid: 0.1,
            edge_margin: 0.5,
            min_spacing: 0.1,
            constraints: Default::default(),
            far_side_pads_only: false,
            shared_halos: false,
            pieces: Vec::new(),
            escape: None,
            crossings: None,
        }
    }

    #[test]
    fn islands_found_with_their_turn_and_regions() {
        let problem = panel(false);
        let copies = detect(&problem, problem.components.len()).expect("copies");
        assert_eq!(copies.detected_by, "net islands");
        assert_eq!(copies.members.len(), 2);
        assert_eq!(copies.transforms[1].angle, 180.0);
        // J1 at x 10 turned half a turn about the origin and moved: x 50.
        let image = copies.transforms[1].pose(problem.poses[0]);
        assert!((image.position[0] - 50.0).abs() < 1.0e-9);
        // The regions are each other's image and do not overlap.
        let [a, b] = [copies.regions[0], copies.regions[1]];
        assert!(a[2] <= b[0] + 1.0e-9 || b[2] <= a[0] + 1.0e-9);
        assert!(a[0] <= 10.0 && a[2] >= 10.0);
    }

    #[test]
    fn shared_nets_found_by_repeats() {
        let problem = panel(true);
        let copies = detect(&problem, problem.components.len()).expect("copies");
        assert_eq!(copies.detected_by, "repeated references");
        assert_eq!(copies.members[0].len(), 5);
    }

    #[test]
    fn different_circuits_are_no_copies() {
        let mut problem = panel(false);
        // The second copy's R2 joins another net of its own: still a copy
        // by references, but its nets no longer map one to one.
        problem.components[7].pins[1].net = 19;
        problem.components[8].pins[1].net = 19;
        problem.components[9].pins[1].net = 19;
        assert!(detect(&problem, problem.components.len()).is_err());
    }

    #[test]
    fn replicated_placement_is_the_first_copys() {
        let problem = panel(false);
        let copies = detect(&problem, problem.components.len()).expect("copies");
        let mut config = core::Config::new();
        config.global.seed = 1;
        let placement = place_copies(&problem, &copies, &config);
        assert!(placement.unplaced.is_empty() && placement.illegal.is_empty());
        for rank in 1..5 {
            let (a, b) = (copies.members[0][rank], copies.members[1][rank]);
            let image = copies.transforms[1].pose(placement.poses[a]);
            assert!(distance_squared(image.position, placement.poses[b].position) < 1.0e-12);
            let region = copies.regions[1];
            let at = placement.poses[b].position;
            assert!(at[0] >= region[0] && at[0] <= region[2]);
        }
    }

    #[test]
    fn on_by_default_and_off_by_the_key() {
        let config: KiCadBoardPlacerConfig = serde_json::from_str("{}").unwrap();
        assert!(config.panel_copies);
        let config: KiCadBoardPlacerConfig =
            serde_json::from_str(r#"{"panel_copies": false}"#).unwrap();
        assert!(!config.panel_copies);
    }

    #[test]
    fn clip_cuts_a_square() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let clipped = clip(&square, [5.0, -1.0, 20.0, 20.0]);
        let b = boxed([clipped[0], clipped[1], clipped[2], clipped[3]]);
        assert_eq!(clipped.len(), 4);
        assert_eq!(b, [5.0, 0.0, 10.0, 10.0]);
    }
}

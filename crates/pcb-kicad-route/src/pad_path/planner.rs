// Copyright (C) 2026 Florian Loitsch.

//! A direct connection across a leaf with known pad and tee contacts.

use super::*;
use pcb_router::{Point, Shape, verify::shape_distance};
use std::collections::VecDeque;

const MAX_TRACKS: usize = 256;
const MAX_PATH: usize = 24;
const IMPROVEMENT_MM: f64 = 0.10;
const NM: f64 = 0.000_001;

type Joint = [i64; 2];

fn joint(at: Point) -> Joint {
    at.map(|coordinate| (coordinate / NM).round() as i64)
}

fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

struct Pad {
    label: String,
    at: Point,
    shape: Shape,
    layers: u32,
    face: Option<usize>,
}

struct Track {
    item: usize,
    at: [Point; 2],
    joint: [Joint; 2],
    layer: usize,
    width: f64,
}

impl Track {
    fn shape(&self) -> Shape {
        Shape::Capsule {
            start: self.at[0],
            end: self.at[1],
            radius: self.width / 2.0,
        }
    }

    fn other(&self, node: Joint) -> usize {
        usize::from(self.joint[0] == node)
    }
}

pub(super) struct Shortcut {
    pub(super) remove: BTreeSet<usize>,
    pub(super) chord: SupplementalSegment,
    pub(super) before_mm: f64,
    pub(super) after_mm: f64,
    pub(super) delta_mm: f64,
    pub(super) kept_tee: bool,
}

fn pad_contacts(
    board: &Expr,
    codes: &BTreeMap<String, String>,
    net: &str,
    layers: &LayerTable,
) -> Option<Vec<Pad>> {
    let mut pads = Vec::new();
    for footprint in board
        .children()
        .iter()
        .filter(|form| matches!(form.head(), Some("footprint" | "module")))
    {
        let reference = footprint_reference(footprint)?;
        let face = form_atom(footprint, "layer", 1).and_then(|name| layers.index(name));
        for pad in footprint
            .children()
            .iter()
            .filter(|pad| pad.head() == Some("pad"))
        {
            if item_net(pad, codes) != Some(net) {
                continue;
            }
            if pad.child("padstack").is_some()
                || form_atom(pad, "remove_unused_layers", 1) == Some("yes")
                || pad
                    .child("drill")
                    .is_some_and(|form| form.child("offset").is_some())
                || !matches!(pad.children().get(2)?.atom()?, "thru_hole" | "smd")
                || !matches!(
                    pad.children().get(3)?.atom()?,
                    "rect" | "circle" | "oval" | "roundrect"
                )
            {
                return None;
            }
            let mask = layers.pad_mask(pad).ok()?;
            if mask == 0 || (mask.count_ones() > 1 && pad.children()[2].atom() != Some("thru_hole"))
            {
                return None;
            }
            let lowered = lower_pad(pad, form_at(footprint).ok()?).ok()?;
            pads.push(Pad {
                label: format!("{reference}:{}", pad.children().get(1)?.atom()?),
                at: lowered.center,
                shape: lowered.geometry.router_shape(),
                layers: mask,
                face,
            });
        }
    }
    Some(pads)
}

fn net_tracks(
    board: &Expr,
    codes: &BTreeMap<String, String>,
    net: &str,
    layers: &LayerTable,
) -> Option<Vec<Track>> {
    let owns = |item: &Expr| {
        item_net(item, codes) == Some(net) || form_atom(item, "net_name", 1) == Some(net)
    };
    let mut tracks = Vec::new();
    for (index, item) in board.children().iter().enumerate() {
        if matches!(item.head(), Some("footprint" | "module")) {
            if item
                .children()
                .iter()
                .any(|form| form.head() != Some("pad") && owns(form))
            {
                return None;
            }
        } else if owns(item) {
            if matches!(item.head(), Some("net")) {
                continue;
            }
            if item.head() != Some("segment")
                || item_net(item, codes) != Some(net)
                || tracks.len() >= MAX_TRACKS
            {
                return None;
            }
            let at = [form_xy(item, "start").ok()?, form_xy(item, "end").ok()?];
            let joints = at.map(joint);
            let width = form_f64(item, "width", 1).ok()?;
            if width <= 0.0 || joints[0] == joints[1] {
                return None;
            }
            tracks.push(Track {
                item: index,
                at,
                joint: joints,
                layer: layers.index(form_atom(item, "layer", 1)?)?,
                width,
            });
        }
    }
    Some(tracks)
}

/// The first point on a chord outside the copper of its starting pad.
fn edge(pad: &Pad, from: Point, to: Point) -> Point {
    let (mut inside, mut outside) = (0.0, 1.0);
    let at = |t| {
        [
            from[0] + t * (to[0] - from[0]),
            from[1] + t * (to[1] - from[1]),
        ]
    };
    for _ in 0..28 {
        let mid = (inside + outside) * 0.5;
        if pad.shape.contains(at(mid)) {
            inside = mid;
        } else {
            outside = mid;
        }
    }
    at(inside)
}

fn one_pad<'a>(label: &str, pads: &'a [Pad]) -> Option<&'a Pad> {
    let mut matching = pads.iter().filter(|pad| pad.label == label);
    let pad = matching.next()?;
    matching.next().is_none().then_some(pad)
}

pub(super) fn pad_net(
    board: &Expr,
    codes: &BTreeMap<String, String>,
    label: &str,
) -> Option<String> {
    let (_, pad) = find_pad(board, label)?;
    let net = item_net(pad, codes)?;
    (!net.is_empty()).then(|| net.to_string())
}

pub(super) fn find_pad<'a>(board: &'a Expr, label: &str) -> Option<(&'a Expr, &'a Expr)> {
    let (reference, number) = label.split_once(':')?;
    let mut matching = board.children().iter().filter(|fp| {
        matches!(fp.head(), Some("footprint" | "module"))
            && footprint_reference(fp).as_deref() == Some(reference)
    });
    let footprint = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    let mut matching = footprint.children().iter().filter(|pad| {
        pad.head() == Some("pad") && pad.children().get(1).and_then(Expr::atom) == Some(number)
    });
    let pad = matching.next()?;
    matching.next().is_none().then_some((footprint, pad))
}

fn joining_track(leaf: &Pad, tracks: &[Track]) -> Option<(usize, usize)> {
    let mut contacts = tracks.iter().enumerate().filter_map(|(index, track)| {
        if leaf.layers & (1 << track.layer) == 0 {
            return None;
        }
        let inside = track.at.map(|at| leaf.shape.contains(at));
        (inside[0] != inside[1]).then_some((index, usize::from(inside[0])))
    });
    let found = contacts.next()?;
    contacts.next().is_none().then_some(found)
}

/// Walk the same-layer centreline to the other pad. Before the first tee,
/// there must be just one continuation. The connection from the other
/// pad back to the tee is left intact.
fn leaf_path(
    first: (usize, usize),
    target: &Pad,
    tracks: &[Track],
) -> Option<(Vec<(usize, usize)>, usize)> {
    let layer = tracks[first.0].layer;
    let joins: BTreeMap<_, Vec<_>> = tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| track.layer == layer)
        .flat_map(|(i, track)| track.joint.into_iter().map(move |node| (node, i)))
        .fold(BTreeMap::new(), |mut map, (node, index)| {
            map.entry(node).or_default().push(index);
            map
        });
    let mut queue = VecDeque::from([(vec![first], None)]);
    let mut seen = BTreeSet::from([first.0]);
    while let Some((path, tee)) = queue.pop_front() {
        let &(index, end) = path.last()?;
        let track = &tracks[index];
        if target.layers & (1 << layer) != 0 && target.shape.contains(track.at[end]) {
            let remove = tee.unwrap_or(path.len());
            return Some((path, remove));
        }
        if path.len() >= MAX_PATH {
            continue;
        }
        let joint = track.joint[end];
        let neighbors = &joins[&joint];
        let tee = tee.or_else(|| (neighbors.len() > 2).then_some(path.len()));
        for &next in neighbors {
            if seen.insert(next) {
                let mut path = path.clone();
                path.push((next, tracks[next].other(joint)));
                queue.push_back((path, tee));
            }
        }
    }
    None
}

/// A track's real copper can touch a branch that its centreline misses.
/// Only the named pad contacts and the exact surviving tee may touch
/// either end of the pruned leaf chain.
fn isolated_leaf(
    prefix: &[(usize, usize)],
    tracks: &[Track],
    pads: &[Pad],
    source: &Pad,
    target: &Pad,
    tee: Option<Joint>,
) -> Result<(), String> {
    for &(index, _) in prefix {
        let track = &tracks[index];
        let copper = track.shape();
        let on_layer = |pad: &Pad| pad.layers & (1 << track.layer) != 0;
        if let Some(pad) = pads.iter().find(|pad| {
            on_layer(pad)
                && pad.label != source.label
                && pad.label != target.label
                && shape_distance(&copper, &pad.shape) <= NM
        }) {
            return Err(format!("the leaf copper touches pad {}", pad.label));
        }
        for (other, retained) in tracks.iter().enumerate() {
            if retained.layer == track.layer
                && prefix.iter().all(|(removed, _)| *removed != other)
                && ![source, target]
                    .iter()
                    .any(|pad| retained.at.iter().all(|&at| pad.shape.contains(at)))
                && !tee.is_some_and(|point| {
                    track.joint.contains(&point) && retained.joint.contains(&point)
                })
                && shape_distance(&copper, &retained.shape()) <= NM
            {
                return Err(
                    "the leaf copper touches retained copper away from its endpoint tee".into(),
                );
            }
        }
    }
    Ok(())
}

fn proposal(
    board: &Expr,
    net: &str,
    source: &Pad,
    target: &Pad,
    pads: &[Pad],
    tracks: &[Track],
    layers: &LayerTable,
) -> Result<Vec<Shortcut>, String> {
    let first =
        joining_track(source, tracks).ok_or("the source pad has no single track leaving it")?;
    let (path, remove) = leaf_path(first, target, tracks)
        .ok_or("no same-layer path of at most 24 segments reaches the target at exact endpoints")?;
    let (patch, tee_end) = path[remove - 1];
    let keep_tee = remove < path.len();
    let tee = keep_tee.then_some(tracks[patch].joint[tee_end]);
    let common_layers = source.layers & target.layers;
    let width = tracks[first.0].width;
    let old_layer = tracks[first.0].layer;
    if !path.iter().all(|(part, _)| tracks[*part].width == width) {
        return Err("the path changes track width".into());
    }
    isolated_leaf(&path[..remove], tracks, pads, source, target, tee)?;
    if let Some(pad) = pads.iter().find(|pad| {
        pad.layers & (1 << old_layer) != 0
            && pad.label != source.label
            && pad.label != target.label
            && path[remove..]
                .iter()
                .any(|&(index, _)| shape_distance(&tracks[index].shape(), &pad.shape) <= NM)
    }) {
        return Err(format!(
            "the path crosses another pad {} after its tee",
            pad.label
        ));
    }
    let mut span = 0.0;
    let mut leaf_span = 0.0;
    for (position, &(index, end)) in path.iter().enumerate() {
        let [a, b] = [tracks[index].at[1 - end], tracks[index].at[end]];
        let from = if position == 0 { edge(source, a, b) } else { a };
        let to = if position == path.len() - 1 {
            edge(target, b, a)
        } else {
            b
        };
        let track_span = distance(from, to);
        span += track_span;
        if position < remove {
            leaf_span += track_span;
        }
    }
    let [start, end] = [
        edge(source, source.at, target.at),
        edge(target, target.at, source.at),
    ];
    let after = distance(start, end);
    // Any existing path from the isolated leaf traverses this chain. From
    // a tee the remaining path cannot be shorter than its geometric gap
    // to the target pad, even if the selected same-layer route detours.
    let remaining_gap = if keep_tee {
        target.shape.distance_to_point(tracks[patch].at[tee_end])
    } else {
        0.0
    };
    if leaf_span + remaining_gap - after < IMPROVEMENT_MM
        || source.shape.contains(end)
        || target.shape.contains(start)
    {
        return Err("the direct chord does not shorten the provable span by 0.1 mm".into());
    }
    let pad_inset = |at: Point, center: Point| {
        let length = distance(at, center);
        let blend = (width.min(0.2) / length).min(0.5);
        std::array::from_fn(|axis| at[axis] + blend * (center[axis] - at[axis]))
    };
    let [start, end] = [pad_inset(start, source.at), pad_inset(end, target.at)];
    let removed: BTreeSet<_> = path[..remove]
        .iter()
        .map(|(part, _)| tracks[*part].item)
        .collect();
    let groups: BTreeSet<_> = board
        .children()
        .iter()
        .filter(|item| item.head() == Some("group"))
        .filter_map(|item| item.child("members"))
        .flat_map(|form| form.children().iter().skip(1).filter_map(Expr::atom))
        .map(str::to_ascii_lowercase)
        .collect();
    if removed.iter().any(|index| {
        let track = &board.children()[*index];
        track
            .children()
            .iter()
            .any(|part| part.head() == Some("locked") || part.atom() == Some("locked"))
            || ["uuid", "tstamp"].iter().any(|key| {
                form_atom(track, key, 1).is_some_and(|id| groups.contains(&id.to_ascii_lowercase()))
            })
    }) {
        return Err("the leaf would remove a locked or grouped track".into());
    }
    let delta = distance(start, end)
        - path[..remove]
            .iter()
            .map(|(part, _)| distance(tracks[*part].at[0], tracks[*part].at[1]))
            .sum::<f64>();
    let face = source.face.filter(|face| target.face == Some(*face));
    let mut options = Vec::new();
    for layer in face
        .into_iter()
        .chain((face != Some(old_layer)).then_some(old_layer))
    {
        if common_layers & (1 << layer) != 0 {
            options.push(Shortcut {
                remove: removed.clone(),
                chord: SupplementalSegment {
                    connection: net.into(),
                    layer: layers.names[layer].clone(),
                    start,
                    end,
                    width,
                },
                before_mm: span,
                after_mm: after,
                delta_mm: delta,
                kept_tee: keep_tee,
            });
        }
    }
    if options.is_empty() {
        return Err("the pads do not share a supported copper layer for the chord".into());
    }
    Ok(options)
}

pub(super) fn plan(
    board: &Expr,
    codes: &BTreeMap<String, String>,
    net: &str,
    layers: &LayerTable,
    pair: [&str; 2],
) -> Result<Vec<Shortcut>, String> {
    let pads = pad_contacts(board, codes, net, layers)
        .ok_or("the net contains unsupported pad geometry")?;
    let tracks = net_tracks(board, codes, net, layers)
        .ok_or("the net contains zones, arcs, vias, or unsupported copper")?;
    let a = one_pad(pair[0], &pads).ok_or("the first pad has no unique modeled copper contact")?;
    let b = one_pad(pair[1], &pads).ok_or("the second pad has no unique modeled copper contact")?;
    proposal(board, net, a, b, &pads, &tracks, layers).or_else(|forward| {
        proposal(board, net, b, a, &pads, &tracks, layers)
            .map_err(|reverse| format!("{}: {forward}. {}: {reverse}.", pair[0], pair[1]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
        (footprint "Pad" (layer "F.Cu") (at 10 10)
          (property "Reference" "P1")
          (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1) (layers "*.Cu") (net "VCC")))
        (footprint "Pad" (layer "F.Cu") (at 14 10)
          (property "Reference" "P2")
          (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1) (layers "*.Cu") (net "VCC")))
        (footprint "Pad" (layer "F.Cu") (at 12 17)
          (property "Reference" "P3")
          (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1) (layers "*.Cu") (net "VCC")))
        (segment (start 10 10) (end 10 13) (width 0.2) (layer "B.Cu") (net "VCC") (uuid "tip"))
        (segment (start 10 13) (end 12 13) (width 0.2) (layer "B.Cu") (net "VCC"))
        (segment (start 12 13) (end 14 10) (width 0.2) (layer "B.Cu") (net "VCC") (uuid "tail"))
        (segment (start 12 13) (end 12 17) (width 0.2) (layer "B.Cu") (net "VCC") (uuid "trunk")))"#;

    fn candidate(board: &str) -> Result<Vec<Shortcut>, String> {
        let board = parse(board).unwrap();
        plan(
            &board,
            &net_codes(&board),
            "VCC",
            &LayerTable::from_pcb(&board).unwrap(),
            ["P1:1", "P2:1"],
        )
    }

    #[test]
    fn a_shorter_pad_chord_keeps_the_original_tail_to_a_shared_tee() {
        let board = parse(BOARD).unwrap();
        let paths = candidate(BOARD).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].chord.layer, "F.Cu");
        assert_eq!(paths[0].chord.width, 0.2);
        assert_eq!(paths[0].remove.len(), 2);
        assert!(paths[0].kept_tee);
        assert!(paths[0].before_mm - paths[0].after_mm > 2.0);
        for (index, track) in board.children().iter().enumerate() {
            if matches!(form_atom(track, "uuid", 1), Some("tail" | "trunk")) {
                assert!(!paths[0].remove.contains(&index));
            }
        }
    }

    #[test]
    fn a_contact_with_retained_copper_inside_the_leaf_refuses_the_candidate() {
        let board = BOARD.replace("(start 12 13) (end 12 17)", "(start 11.5 13) (end 12 17)");
        assert!(
            matches!(candidate(&board), Err(reason) if reason.contains("away from its endpoint tee"))
        );
    }

    #[test]
    fn a_group_refuses_to_remove_its_member() {
        let grouped = format!(
            "{} (group (uuid \"group\") (members \"TIP\" \"TAIL\")))",
            BOARD.trim_end_matches(')').to_owned() + "))"
        );
        let result = candidate(&grouped);
        assert!(matches!(result, Err(reason) if reason.contains("locked or grouped")));
    }
}

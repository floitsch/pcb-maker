// Copyright (C) 2026 Toit contributors.

//! How a board's tracks look (docs/quality.md, "Aesthetics"): bends per
//! connection, stair-steps and tracks that pass between the adjacent pins
//! of a part. The complaint about autorouters is traces that "snake
//! through SMD pins" and bend "in many small steps"; these count both, the
//! same way on a designer's board and on ours.
//!
//! A track's path is a chain of segments on one layer and of one net
//! between terminals: a pad of the net, a via, a junction of three or more
//! segments, or a free end. Along a path a direction change of 5 degrees
//! or more is a turn; turns of one sense less than 1 mm apart whose sum is
//! at most 90 degrees are one bend (a corner cut at 45 degrees, an arc).
//! A stair-step is two bends within 2 mm of each other (the straight run
//! between them) that return to the direction before the first. A pin-row
//! crossing is a track of another net crossing the line between the
//! centres of two adjacent SMD pads of one part (pads of a part of three
//! or more, at most 1.25 times the closer one's nearest-neighbour distance
//! apart), counted once per pad pair and net.

use serde::Serialize;
use std::collections::{BTreeSet, HashMap};

/// Below this a direction change is noise (degrees).
const TURN_DEGREES: f64 = 5.0;
/// Turns of one sense closer than this are one bend (mm).
const MERGE_MM: f64 = 1.0;
/// Two bends within this that return to the first direction are a
/// stair-step (mm).
const STAIR_MM: f64 = 2.0;
/// Adjacent pads: at most this times the nearest-neighbour distance.
const ADJACENT: f64 = 1.25;

#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadAesthetics {
    /// The track paths (chains between terminals).
    pub paths: usize,
    /// The board's connections: per net of two pads or more, its pads less
    /// one (the same for any routing of one netlist).
    pub connections: usize,
    pub bends: usize,
    pub bends_per_connection: Option<f64>,
    pub stair_steps: usize,
    pub pin_row_crossings: usize,
}

pub(crate) struct AestheticTrack {
    pub start: [f64; 2],
    pub end: [f64; 2],
    /// An arc's middle point.
    pub mid: Option<[f64; 2]>,
    pub layer: String,
    pub net: Option<String>,
}

pub(crate) struct AestheticPad {
    pub center: [f64; 2],
    pub half: [f64; 2],
    pub net: Option<String>,
    /// The copper layer of an SMD pad; None for a through-hole pad (every
    /// layer).
    pub layer: Option<String>,
    pub part: usize,
}

/// A uniform bucket grid of boxes.
struct Buckets {
    cell: f64,
    map: HashMap<(i64, i64), Vec<usize>>,
}

impl Buckets {
    fn new(cell: f64) -> Self {
        Self {
            cell,
            map: HashMap::new(),
        }
    }

    fn range(&self, low: [f64; 2], high: [f64; 2]) -> impl Iterator<Item = (i64, i64)> {
        let (x0, y0) = (
            (low[0] / self.cell).floor() as i64,
            (low[1] / self.cell).floor() as i64,
        );
        let (x1, y1) = (
            (high[0] / self.cell).floor() as i64,
            (high[1] / self.cell).floor() as i64,
        );
        (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
    }

    fn insert(&mut self, index: usize, low: [f64; 2], high: [f64; 2]) {
        let keys: Vec<_> = self.range(low, high).collect();
        for key in keys {
            self.map.entry(key).or_default().push(index);
        }
    }

    fn query(&self, low: [f64; 2], high: [f64; 2]) -> BTreeSet<usize> {
        self.range(low, high)
            .filter_map(|key| self.map.get(&key))
            .flatten()
            .copied()
            .collect()
    }
}

fn key(point: [f64; 2]) -> (i64, i64) {
    (
        (point[0] * 1000.0).round() as i64,
        (point[1] * 1000.0).round() as i64,
    )
}

fn length(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The signed turn from direction a to direction b, in degrees (-180, 180].
fn turn(a: [f64; 2], b: [f64; 2]) -> f64 {
    let cross = a[0] * b[1] - a[1] * b[0];
    let dot = a[0] * b[0] + a[1] * b[1];
    cross.atan2(dot).to_degrees()
}

/// The bends of a polyline and its stair-steps.
pub(crate) fn bends_of(points: &[[f64; 2]]) -> (usize, usize) {
    let mut points: Vec<[f64; 2]> = points.to_vec();
    points.dedup_by(|b, a| length(*a, *b) < 1.0e-6);
    // (angle, where it starts, where it ends) along the path.
    let mut bends: Vec<(f64, f64, f64)> = Vec::new();
    let mut along = 0.0;
    for index in 1..points.len().saturating_sub(1) {
        along += length(points[index - 1], points[index]);
        let a = [
            points[index][0] - points[index - 1][0],
            points[index][1] - points[index - 1][1],
        ];
        let b = [
            points[index + 1][0] - points[index][0],
            points[index + 1][1] - points[index][1],
        ];
        let angle = turn(a, b);
        if angle.abs() < TURN_DEGREES {
            continue;
        }
        match bends.last_mut() {
            Some(last)
                if last.0.signum() == angle.signum()
                    && along - last.2 < MERGE_MM
                    && (last.0 + angle).abs() <= 90.0 + TURN_DEGREES =>
            {
                last.0 += angle;
                last.2 = along;
            }
            _ => bends.push((angle, along, along)),
        }
    }
    let mut stairs = 0;
    let mut index = 1;
    while index < bends.len() {
        let (first, second) = (bends[index - 1], bends[index]);
        if first.0.signum() != second.0.signum()
            && (first.0 + second.0).abs() < TURN_DEGREES
            && second.1 - first.2 < STAIR_MM
        {
            stairs += 1;
            index += 2;
        } else {
            index += 1;
        }
    }
    (bends.len(), stairs)
}

fn on_layer(pad: &AestheticPad, layer: &str) -> bool {
    pad.layer.as_deref().is_none_or(|own| own == layer)
}

fn inside(pad: &AestheticPad, point: [f64; 2]) -> bool {
    (point[0] - pad.center[0]).abs() <= pad.half[0] + 1.0e-6
        && (point[1] - pad.center[1]).abs() <= pad.half[1] + 1.0e-6
}

/// Whether segments ab and cd cross (properly, or touching).
fn crosses(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let side = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let (d1, d2) = (side(a, b, c), side(a, b, d));
    let (d3, d4) = (side(c, d, a), side(c, d, b));
    d1 * d2 <= 0.0 && d3 * d4 <= 0.0 && (d1 != 0.0 || d2 != 0.0)
}

pub(crate) fn aesthetics(
    tracks: &[AestheticTrack],
    vias: &[[f64; 2]],
    pads: &[AestheticPad],
) -> KiCadAesthetics {
    let mut pad_buckets = Buckets::new(2.0);
    for (index, pad) in pads.iter().enumerate() {
        pad_buckets.insert(
            index,
            [pad.center[0] - pad.half[0], pad.center[1] - pad.half[1]],
            [pad.center[0] + pad.half[0], pad.center[1] + pad.half[1]],
        );
    }
    let via_keys: BTreeSet<(i64, i64)> = vias.iter().map(|via| key(*via)).collect();

    // Paths and their bends, per net and layer.
    let mut groups: HashMap<(&str, &str), Vec<Vec<[f64; 2]>>> = HashMap::new();
    for track in tracks {
        let Some(net) = track.net.as_deref() else {
            continue;
        };
        let polyline = match track.mid {
            Some(mid) => vec![track.start, mid, track.end],
            None => vec![track.start, track.end],
        };
        if length(track.start, track.end) < 1.0e-6 && track.mid.is_none() {
            continue;
        }
        groups
            .entry((net, track.layer.as_str()))
            .or_default()
            .push(polyline);
    }
    let (mut paths, mut bends, mut stairs) = (0, 0, 0);
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    for ((net, layer), pieces) in groups {
        let mut ends: HashMap<(i64, i64), Vec<(usize, bool)>> = HashMap::new();
        for (index, piece) in pieces.iter().enumerate() {
            ends.entry(key(piece[0])).or_default().push((index, false));
            ends.entry(key(*piece.last().unwrap()))
                .or_default()
                .push((index, true));
        }
        let terminal = |point: [f64; 2]| {
            let at = key(point);
            ends.get(&at).map_or(0, Vec::len) != 2
                || via_keys.contains(&at)
                || pad_buckets
                    .query(point, point)
                    .into_iter()
                    .any(|index| {
                        let pad = &pads[index];
                        pad.net.as_deref() == Some(net) && on_layer(pad, layer) && inside(pad, point)
                    })
        };
        let mut used = vec![false; pieces.len()];
        let walk = |first: usize, reversed: bool, used: &mut Vec<bool>| -> Vec<[f64; 2]> {
            let mut points: Vec<[f64; 2]> = Vec::new();
            let (mut index, mut backwards) = (first, reversed);
            loop {
                used[index] = true;
                let mut piece = pieces[index].clone();
                if backwards {
                    piece.reverse();
                }
                if points.is_empty() {
                    points.extend(&piece);
                } else {
                    points.extend(&piece[1..]);
                }
                let end = *points.last().unwrap();
                if terminal(end) {
                    break;
                }
                let next = ends[&key(end)]
                    .iter()
                    .find(|(other, _)| !used[*other])
                    .copied();
                match next {
                    Some((other, at_end)) => {
                        index = other;
                        backwards = at_end;
                    }
                    None => break,
                }
            }
            points
        };
        for index in 0..pieces.len() {
            for at_end in [false, true] {
                if used[index] {
                    continue;
                }
                let point = if at_end {
                    *pieces[index].last().unwrap()
                } else {
                    pieces[index][0]
                };
                if terminal(point) {
                    let points = walk(index, at_end, &mut used);
                    let (b, s) = bends_of(&points);
                    paths += 1;
                    bends += b;
                    stairs += s;
                }
            }
        }
        // Loops without a terminal.
        for index in 0..pieces.len() {
            if !used[index] {
                let points = walk(index, false, &mut used);
                let (b, s) = bends_of(&points);
                paths += 1;
                bends += b;
                stairs += s;
            }
        }
    }

    // Connections: per net, its pads less one.
    let mut net_pads: HashMap<&str, usize> = HashMap::new();
    for pad in pads {
        if let Some(net) = &pad.net {
            *net_pads.entry(net.as_str()).or_default() += 1;
        }
    }
    let connections: usize = net_pads.values().map(|count| count.saturating_sub(1)).sum();

    // Pin-row crossings.
    let mut track_buckets = Buckets::new(2.0);
    let segments: Vec<([f64; 2], [f64; 2], &str, &str)> = tracks
        .iter()
        .filter_map(|track| {
            let net = track.net.as_deref()?;
            Some(match track.mid {
                Some(mid) => vec![
                    (track.start, mid, net, track.layer.as_str()),
                    (mid, track.end, net, track.layer.as_str()),
                ],
                None => vec![(track.start, track.end, net, track.layer.as_str())],
            })
        })
        .flatten()
        .collect();
    for (index, (a, b, _, _)) in segments.iter().enumerate() {
        track_buckets.insert(
            index,
            [a[0].min(b[0]), a[1].min(b[1])],
            [a[0].max(b[0]), a[1].max(b[1])],
        );
    }
    let mut by_part: HashMap<usize, Vec<usize>> = HashMap::new();
    for (index, pad) in pads.iter().enumerate() {
        if pad.layer.is_some() {
            by_part.entry(pad.part).or_default().push(index);
        }
    }
    let mut crossings = BTreeSet::new();
    for members in by_part.values() {
        if members.len() < 3 {
            continue;
        }
        let nearest: Vec<f64> = members
            .iter()
            .map(|a| {
                members
                    .iter()
                    .filter(|b| *b != a && pads[**b].layer == pads[*a].layer)
                    .map(|b| length(pads[*a].center, pads[*b].center))
                    .filter(|d| *d > 1.0e-6)
                    .fold(f64::INFINITY, f64::min)
            })
            .collect();
        for (i, a) in members.iter().enumerate() {
            for (j, b) in members.iter().enumerate().skip(i + 1) {
                let (pa, pb) = (&pads[*a], &pads[*b]);
                if pa.layer != pb.layer {
                    continue;
                }
                let gap = length(pa.center, pb.center);
                if gap < 1.0e-6 || gap > ADJACENT * nearest[i].min(nearest[j]) {
                    continue;
                }
                let layer = pa.layer.as_deref().unwrap();
                let (c, d) = (pa.center, pb.center);
                for index in track_buckets.query(
                    [c[0].min(d[0]), c[1].min(d[1])],
                    [c[0].max(d[0]), c[1].max(d[1])],
                ) {
                    let (s, e, net, track_layer) = segments[index];
                    if track_layer != layer
                        || pa.net.as_deref() == Some(net)
                        || pb.net.as_deref() == Some(net)
                    {
                        continue;
                    }
                    if crosses(c, d, s, e) {
                        crossings.insert((*a, *b, net));
                    }
                }
            }
        }
    }

    KiCadAesthetics {
        paths,
        connections,
        bends,
        bends_per_connection: (connections > 0)
            .then(|| (bends as f64 / connections as f64 * 1000.0).round() / 1000.0),
        stair_steps: stairs,
        pin_row_crossings: crossings.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(start: [f64; 2], end: [f64; 2], net: &str) -> AestheticTrack {
        AestheticTrack {
            start,
            end,
            mid: None,
            layer: "F.Cu".into(),
            net: Some(net.into()),
        }
    }

    #[test]
    fn a_corner_cut_at_45_degrees_is_one_bend_and_a_jog_is_a_stair_step() {
        // A 90-degree corner of two 45-degree turns 0.5 mm apart.
        let corner = [[0.0, 0.0], [5.0, 0.0], [5.35, 0.35], [5.35, 5.0]];
        assert_eq!(bends_of(&corner), (1, 0));
        // The same turns 3 mm apart are two bends.
        let wide = [[0.0, 0.0], [5.0, 0.0], [7.0, 2.0], [7.0, 5.0]];
        assert_eq!(bends_of(&wide), (2, 0));
        // A jog: 45 degrees up and back within 2 mm.
        let jog = [[0.0, 0.0], [5.0, 0.0], [5.5, 0.5], [10.0, 0.5]];
        assert_eq!(bends_of(&jog), (2, 1));
        // Collinear pieces are no bend.
        assert_eq!(bends_of(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]), (0, 0));
        // A staircase of four jogs.
        let mut stairs = vec![[0.0, 0.0]];
        for step in 0..4 {
            let x = step as f64 * 1.5;
            let y = step as f64 * 0.3;
            stairs.push([x + 1.0, y]);
            stairs.push([x + 1.3, y + 0.3]);
        }
        stairs.push([10.0, 1.2]);
        let (bends, steps) = bends_of(&stairs);
        assert_eq!((bends, steps), (8, 4));
    }

    #[test]
    fn paths_end_at_pads_vias_and_junctions_and_tracks_between_pins_count() {
        let pad = |x: f64, net: &str, part: usize| AestheticPad {
            center: [x, 0.0],
            half: [0.15, 0.5],
            net: Some(net.into()),
            layer: Some("F.Cu".into()),
            part,
        };
        // A row of three pins at 0.5 mm, a fourth pad elsewhere.
        let pads = vec![
            pad(0.0, "A", 0),
            pad(0.5, "B", 0),
            pad(1.0, "C", 0),
            pad(10.0, "A", 1),
        ];
        // A leaves its pin of the row and reaches its other pad with two
        // bends (none at the pads).
        let tracks = vec![
            track([0.0, 0.0], [0.0, 2.0], "A"),
            track([0.0, 2.0], [10.0, 2.0], "A"),
            track([10.0, 2.0], [10.0, 0.0], "A"),
            // D crosses between B and C.
            track([0.75, -2.0], [0.75, 2.0], "D"),
        ];
        let report = aesthetics(&tracks, &[], &pads);
        assert_eq!(report.paths, 2);
        assert_eq!(report.bends, 2);
        assert_eq!(report.connections, 1);
        assert_eq!(report.pin_row_crossings, 1);
        // A via at the first corner splits A's path there.
        let report = aesthetics(&tracks, &[[0.0, 2.0]], &pads);
        assert_eq!((report.paths, report.bends), (3, 1));
    }
}

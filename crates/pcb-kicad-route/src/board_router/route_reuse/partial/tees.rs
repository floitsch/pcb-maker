// Copyright (C) 2026 Florian Loitsch.

//! Model exact tee contacts while preserving the original trunk items.

use super::*;

const MAX_TREE_EDGES: usize = 2 * MAX_SEGMENTS;

pub(super) fn subdivide(segments: Vec<Segment>) -> Option<Vec<Segment>> {
    let mut tree = Vec::with_capacity(segments.len());
    for (index, trunk) in segments.iter().enumerate() {
        let mut junctions = BTreeMap::new();
        for (other, branch) in segments.iter().enumerate() {
            if other == index || branch.layer != trunk.layer {
                continue;
            }
            for (point, node) in branch.points.into_iter().zip(branch.nodes) {
                if interior(trunk.points, point)
                    && (!matches!(node, Node::End(..)) || junctions.insert(point, node).is_some())
                {
                    return None;
                }
            }
        }
        if junctions.is_empty() {
            tree.push(trunk.clone());
            continue;
        }
        let [mut start, mut end] = trunk.points;
        let [mut from, mut to] = trunk.nodes;
        if start > end {
            std::mem::swap(&mut start, &mut end);
            std::mem::swap(&mut from, &mut to);
        }
        for (point, node) in junctions.into_iter().chain([(end, to)]) {
            tree.push(Segment {
                source: None,
                points: [start, point],
                at: [start, point].map(|point| point.map(|nm| f64::from(nm) * MM_PER_NM)),
                nodes: [from, node],
                ..*trunk
            });
            start = point;
            from = node;
        }
        if tree.len() > MAX_TREE_EDGES {
            return None;
        }
    }
    (tree.len() <= MAX_TREE_EDGES).then_some(tree)
}

fn interior([first, last]: [Point; 2], point: Point) -> bool {
    let low = first.min(last);
    let high = first.max(last);
    if point <= low || point >= high {
        return false;
    }
    let delta = |b: Point| [0, 1].map(|axis| i128::from(b[axis]) - i128::from(low[axis]));
    let [x, y] = delta(high);
    let [u, v] = delta(point);
    x * v == y * u
}

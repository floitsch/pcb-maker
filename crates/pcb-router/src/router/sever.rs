// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! What a cut of a pour severs (`Config::pour_integrity`). A track of
//! another net across a pour's fill costs `plane_cut_cost` a step
//! wherever it runs; a nick in a wide area leaves the fill whole, a cut
//! across a narrow corridor that joins two parts of it splits the pour.
//! Per pour and layer the fill as the pour model sees it now (the pour's
//! nodes free of other nets' copper) is split into wide cores (its
//! opening by a disk of `SEVER_HALF_WIDTH_MM`) and narrow corridors, the
//! corridors cut into chunks of `SEVER_CHUNK_MM` (what one crossing takes
//! away); on the graph of cores and chunks a chunk that is an
//! articulation point severs the parts beyond it, and its nodes get the
//! share of the pour that would no longer be in the largest remaining
//! part (`Router::sever`, 0 to 255). Recomputed before a batch of the
//! negotiation or the clean-up when copper changed and the search since
//! the last refresh was at least ten times what a refresh costs.

use super::*;

/// Half the width below which a part of a pour's fill is a corridor, in
/// mm: a corridor narrower than 1.2 mm is closed by one or two tracks
/// and their clearances.
const SEVER_HALF_WIDTH_MM: f64 = 0.6;

/// The side of the chunks a corridor is cut into, in mm: about what one
/// track crossing the corridor takes away.
const SEVER_CHUNK_MM: f64 = 1.2;

/// A step on a node whose cut severs share `s` of the pour costs
/// `1 + SEVER_COST * pour_integrity * s` times its cut cost.
pub(super) const SEVER_COST: f32 = 20.0;

/// The work of a refresh per lattice node of a pour layer, in
/// nanoseconds of an idle machine (two distance passes, two labellings
/// and the masks: 56-66 ns a node on the fence at 46 mm under a load of
/// 46, whose wall clock stretches about 1.7 times).
pub const SEVER_NODE_NANOS: u64 = 30;

/// The most of the search since the last refresh a refresh may cost: it
/// waits for more search otherwise (deterministic: both are counts).
const SEVER_SHARE: f64 = 0.1;

/// `pour_integrity` as it applies to `board`: on two layers only. With
/// inner planes the pours stay whole on the rungs that keep signals off
/// them, and pricing the planes for pour nets routed as tracks cost the
/// expansion tier 197 more unconnected items (x1570: MSign 69 to 137,
/// ZSWatch 3 to 63) and the fingerprints' jetson 20 to 85.
pub(super) fn integrity(config: &Config, board: &Board) -> f64 {
    if board.layer_count == 2 {
        config.pour_integrity.max(0.0)
    } else {
        0.0
    }
}

impl Router {
    /// Recomputes `sever` from the fill as the routes left it, when copper
    /// changed since the last refresh (`copper_changes`). Counted as pour
    /// work (`SEVER_NODE_NANOS` a node of every pour layer refreshed).
    pub(super) fn refresh_sever(&mut self) {
        if integrity(&self.config, &self.board) <= 0.0 || self.sever_at == Some(self.copper_changes)
        {
            return;
        }
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        // A refresh costs at most `SEVER_SHARE` of the search since the
        // last one (a big board's pour layers take tens of milliseconds;
        // the negotiation's batches come every few).
        if self.sever_at.is_some() {
            let pours = self
                .board
                .planes
                .iter()
                .filter(|plane| !self.exclusive[plane.layer])
                .count() as f64;
            let cost = pours * cells as f64 * SEVER_NODE_NANOS as f64;
            let searched = (self.scratch.expansions - self.sever_expansions) as f64
                / REFERENCE_EXPANSIONS_PER_SECOND
                * 1.0e9;
            if cost > SEVER_SHARE * searched {
                return;
            }
        }
        self.sever_at = Some(self.copper_changes);
        self.sever_expansions = self.scratch.expansions;
        let started = std::time::Instant::now();
        let (nx, ny) = (self.grid.nx, self.grid.ny);
        let half = ((SEVER_HALF_WIDTH_MM / self.grid.pitch).round() as u32).max(1);
        let chunk = ((SEVER_CHUNK_MM / self.grid.pitch).round() as usize).max(2);
        let mut nodes = 0u64;
        for layer in 0..layers {
            if self.covered[layer].is_empty() || self.exclusive[layer] {
                if !self.sever[layer].is_empty() {
                    self.sever[layer] = Vec::new();
                }
                continue;
            }
            let mut owners: Vec<u32> = self
                .board
                .planes
                .iter()
                .filter(|plane| plane.layer == layer)
                .map(|plane| crate::grid::owner(plane.net))
                .collect();
            owners.sort_unstable();
            owners.dedup();
            let mut level = vec![0u8; cells];
            let mut any = false;
            for owner in owners {
                let net = owner - 1;
                let Some(class) = self
                    .board
                    .planes
                    .iter()
                    .find(|plane| plane.layer == layer && plane.net == net)
                    .map(|plane| plane.class)
                else {
                    continue;
                };
                let covered = &self.covered[layer];
                let zone = covered
                    .iter()
                    .filter(|value| **value as u32 == owner)
                    .count();
                if zone == 0 {
                    continue;
                }
                nodes += cells as u64;
                let map = self.map_index(class, layer);
                let own = OwnStamps::new(&self.nets[net as usize].stamped, &[map], cells);
                let occupancy = &self.occupancy;
                let free: Vec<bool> = (0..cells)
                    .map(|cell| {
                        covered[cell] as u32 == owner
                            && occupancy.get(map, cell) as usize == own.contains(map, cell) as usize
                    })
                    .collect();
                for (cell, value) in sever_levels(nx, ny, &free, half, chunk, zone)
                    .into_iter()
                    .enumerate()
                {
                    if value > level[cell] {
                        level[cell] = value;
                        any = true;
                    }
                }
            }
            self.sever[layer] = if any { level } else { Vec::new() };
        }
        self.scratch.pour_units.fetch_add(
            nodes * SEVER_NODE_NANOS,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.scratch.pour_nanos.fetch_add(
            started.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        if std::env::var_os("PCB_ROUTER_SEVER_TRACE").is_some() {
            let marked: Vec<usize> = self
                .sever
                .iter()
                .map(|layer| layer.iter().filter(|value| **value != 0).count())
                .collect();
            eprintln!(
                "sever: {nodes} nodes in {} ns ({:.1} ns a node), marked per layer {marked:?}",
                started.elapsed().as_nanos(),
                started.elapsed().as_nanos() as f64 / nodes.max(1) as f64
            );
        }
    }

    /// The factor on a cut at `cell` of `layer`: 1 where cutting severs
    /// nothing.
    #[inline]
    pub(super) fn sever_factor(&self, layer: usize, cell: usize) -> f32 {
        match self.sever[layer].get(cell) {
            Some(&level) if level != 0 => 1.0 + self.sever_scale * level as f32,
            _ => 1.0,
        }
    }
}

/// Chamfer distances (3 a side step, 4 a diagonal one) of every node to
/// the nearest node where `source` is false (`invert`: true), the grid's
/// border counting as such a node.
fn chamfer(nx: usize, ny: usize, source: &[bool], invert: bool) -> Vec<u16> {
    let inside = |cell: usize| source[cell] != invert;
    let mut distance: Vec<u16> = (0..nx * ny)
        .map(|cell| if inside(cell) { u16::MAX } else { 0 })
        .collect();
    let at = |distance: &Vec<u16>, x: i64, y: i64| -> u16 {
        if x < 0 || y < 0 || x >= nx as i64 || y >= ny as i64 {
            if invert { u16::MAX } else { 0 }
        } else {
            distance[y as usize * nx + x as usize]
        }
    };
    for y in 0..ny as i64 {
        for x in 0..nx as i64 {
            let cell = y as usize * nx + x as usize;
            if distance[cell] == 0 {
                continue;
            }
            let best = [
                at(&distance, x - 1, y).saturating_add(3),
                at(&distance, x, y - 1).saturating_add(3),
                at(&distance, x - 1, y - 1).saturating_add(4),
                at(&distance, x + 1, y - 1).saturating_add(4),
            ]
            .into_iter()
            .min()
            .unwrap_or(u16::MAX);
            distance[cell] = distance[cell].min(best);
        }
    }
    for y in (0..ny as i64).rev() {
        for x in (0..nx as i64).rev() {
            let cell = y as usize * nx + x as usize;
            if distance[cell] == 0 {
                continue;
            }
            let best = [
                at(&distance, x + 1, y).saturating_add(3),
                at(&distance, x, y + 1).saturating_add(3),
                at(&distance, x + 1, y + 1).saturating_add(4),
                at(&distance, x - 1, y + 1).saturating_add(4),
            ]
            .into_iter()
            .min()
            .unwrap_or(u16::MAX);
            distance[cell] = distance[cell].min(best);
        }
    }
    distance
}

/// Per node of the fill `free` (an `nx` by `ny` lattice): how much of the
/// pour (`zone` nodes) a cut there severs, 0 to 255 (255: all of it).
/// Wide cores are the fill's opening by a disk of `half` nodes; the rest
/// are corridors, cut into chunks of `chunk` nodes a side. A chunk that
/// is an articulation point of the graph of cores and chunks severs the
/// parts its removal leaves outside the largest; a core severs nothing.
pub(super) fn sever_levels(
    nx: usize,
    ny: usize,
    free: &[bool],
    half: u32,
    chunk: usize,
    zone: usize,
) -> Vec<u8> {
    let cells = nx * ny;
    let mut level = vec![0u8; cells];
    if !free.iter().any(|value| *value) {
        return level;
    }
    // The opening: nodes at least `half` from the fill's edge, grown back
    // by `half`.
    let reach = (3 * half).min(u16::MAX as u32) as u16;
    let inner = chamfer(nx, ny, free, false);
    let eroded: Vec<bool> = inner.iter().map(|value| *value >= reach).collect();
    let grown = chamfer(nx, ny, &eroded, true);
    // Labels: cores (4-connected open nodes) first, then corridor chunks
    // (4-connected narrow nodes of one chunk).
    const NONE: u32 = u32::MAX;
    let mut label = vec![NONE; cells];
    let open = |cell: usize| free[cell] && grown[cell] < reach;
    let mut weights: Vec<u64> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut fill = |label: &mut Vec<u32>,
                    weights: &mut Vec<u64>,
                    start: usize,
                    same: &dyn Fn(usize, usize) -> bool| {
        let id = weights.len() as u32;
        let mut weight = 0;
        label[start] = id;
        stack.push(start);
        while let Some(cell) = stack.pop() {
            weight += 1;
            let (x, y) = (cell % nx, cell / nx);
            let mut visit = |other: usize| {
                if label[other] == NONE && same(cell, other) {
                    label[other] = id;
                    stack.push(other);
                }
            };
            if x > 0 {
                visit(cell - 1);
            }
            if x + 1 < nx {
                visit(cell + 1);
            }
            if y > 0 {
                visit(cell - nx);
            }
            if y + 1 < ny {
                visit(cell + nx);
            }
        }
        weights.push(weight);
    };
    for cell in 0..cells {
        if label[cell] == NONE && open(cell) {
            fill(&mut label, &mut weights, cell, &|_, other| open(other));
        }
    }
    let cores = weights.len();
    let chunk_of = |cell: usize| ((cell % nx) / chunk, (cell / nx) / chunk);
    let narrow = |cell: usize| free[cell] && !open(cell);
    for cell in 0..cells {
        if label[cell] == NONE && narrow(cell) {
            fill(&mut label, &mut weights, cell, &|from, other| {
                narrow(other) && chunk_of(other) == chunk_of(from)
            });
        }
    }
    if weights.len() == cores {
        return level;
    }
    // The graph: an edge where two labels' nodes are side by side.
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for cell in 0..cells {
        let a = label[cell];
        if a == NONE || (a as usize) < cores {
            continue;
        }
        let (x, y) = (cell % nx, cell / nx);
        for other in [
            (x > 0).then(|| cell - 1),
            (x + 1 < nx).then(|| cell + 1),
            (y > 0).then(|| cell - nx),
            (y + 1 < ny).then(|| cell + nx),
        ]
        .into_iter()
        .flatten()
        {
            let b = label[other];
            if b != NONE && b != a {
                edges.push((a.min(b), a.max(b)));
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();
    let vertices = weights.len();
    let mut adjacency: Vec<Vec<u32>> = vec![Vec::new(); vertices];
    for (a, b) in &edges {
        adjacency[*a as usize].push(*b);
        adjacency[*b as usize].push(*a);
    }
    // Articulation points with the weight each one's removal cuts off
    // (Tarjan, iterative): per vertex the parts it separates.
    let severed = articulation_severs(&adjacency, &weights);
    let zone = zone.max(1) as f64;
    let vertex_level: Vec<u8> = severed
        .iter()
        .enumerate()
        .map(|(vertex, lost)| {
            if vertex < cores || *lost == 0 {
                0
            } else {
                ((*lost as f64 / zone) * 255.0).ceil().min(255.0) as u8
            }
        })
        .collect();
    for cell in 0..cells {
        let vertex = label[cell];
        if vertex != NONE {
            level[cell] = vertex_level[vertex as usize];
        }
    }
    level
}

/// Per vertex of the graph `adjacency` (vertex weights `weights`): the
/// weight its removal leaves outside the largest remaining part of its
/// connected component (0 for a vertex that is no articulation point).
fn articulation_severs(adjacency: &[Vec<u32>], weights: &[u64]) -> Vec<u64> {
    let vertices = adjacency.len();
    const UNSEEN: u32 = u32::MAX;
    let mut discovered = vec![UNSEEN; vertices];
    let mut low = vec![0u32; vertices];
    let mut subtree = vec![0u64; vertices];
    // Per vertex: the separated children's subtree weights.
    let mut parts: Vec<Vec<u64>> = vec![Vec::new(); vertices];
    let mut result = vec![0u64; vertices];
    let mut time = 0u32;
    for root in 0..vertices {
        if discovered[root] != UNSEEN {
            continue;
        }
        let mut members: Vec<usize> = Vec::new();
        // (vertex, parent, next neighbour index)
        let mut stack: Vec<(usize, usize, usize)> = vec![(root, usize::MAX, 0)];
        discovered[root] = time;
        low[root] = time;
        time += 1;
        subtree[root] = weights[root];
        members.push(root);
        while let Some(&mut (vertex, parent, ref mut next)) = stack.last_mut() {
            if *next < adjacency[vertex].len() {
                let other = adjacency[vertex][*next] as usize;
                *next += 1;
                if discovered[other] == UNSEEN {
                    discovered[other] = time;
                    low[other] = time;
                    time += 1;
                    subtree[other] = weights[other];
                    members.push(other);
                    stack.push((other, vertex, 0));
                } else if other != parent {
                    low[vertex] = low[vertex].min(discovered[other]);
                }
            } else {
                stack.pop();
                if parent != usize::MAX {
                    low[parent] = low[parent].min(low[vertex]);
                    subtree[parent] += subtree[vertex];
                    if low[vertex] >= discovered[parent] {
                        parts[parent].push(subtree[vertex]);
                    }
                }
            }
        }
        let total = subtree[root];
        for vertex in members {
            let mut pieces = std::mem::take(&mut parts[vertex]);
            if vertex == root {
                // The root separates its children only when it has two or
                // more of them.
                if pieces.len() < 2 {
                    continue;
                }
            } else {
                if pieces.is_empty() {
                    continue;
                }
                let separated: u64 = pieces.iter().sum();
                pieces.push(total - weights[vertex] - separated);
            }
            let largest = pieces.iter().copied().max().unwrap_or(0);
            result[vertex] = pieces.iter().sum::<u64>() - largest;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two 20 x 20 squares joined by a corridor 2 nodes wide and 10 long.
    fn dumbbell() -> (usize, usize, Vec<bool>) {
        let (nx, ny) = (52, 22);
        let mut free = vec![false; nx * ny];
        for y in 1..21 {
            for x in 1..21 {
                free[y * nx + x] = true;
                free[y * nx + x + 30] = true;
            }
        }
        for y in 10..12 {
            for x in 21..31 {
                free[y * nx + x] = true;
            }
        }
        (nx, ny, free)
    }

    #[test]
    fn a_corridor_between_two_halves_severs_half() {
        let (nx, ny, free) = dumbbell();
        let zone = free.iter().filter(|value| **value).count();
        let level = sever_levels(nx, ny, &free, 3, 4, zone);
        // The corridor's middle: cutting it leaves one square apart.
        let middle = level[10 * nx + 26];
        assert!(middle > 100, "{middle}");
        // A square's middle and corner: wide, nothing severed.
        assert_eq!(level[10 * nx + 10], 0);
        assert_eq!(level[nx + 1], 0);
    }

    #[test]
    fn a_dead_end_severs_only_itself() {
        let (nx, ny) = (40, 22);
        let mut free = vec![false; nx * ny];
        for y in 1..21 {
            for x in 1..21 {
                free[y * nx + x] = true;
            }
        }
        // A spur 2 wide and 16 long off the square's side.
        for y in 10..12 {
            for x in 21..37 {
                free[y * nx + x] = true;
            }
        }
        let zone = free.iter().filter(|value| **value).count();
        let level = sever_levels(nx, ny, &free, 3, 4, zone);
        // Cutting near the root severs the spur (a few % of the pour);
        // its tip severs nothing.
        assert!(level[10 * nx + 23] > 0 && level[10 * nx + 23] < 40);
        assert_eq!(level[10 * nx + 36], 0);
    }

    #[test]
    fn two_corridors_in_parallel_sever_nothing() {
        let (nx, ny, mut free) = dumbbell();
        for y in 3..5 {
            for x in 21..31 {
                free[y * nx + x] = true;
            }
        }
        let zone = free.iter().filter(|value| **value).count();
        let level = sever_levels(nx, ny, &free, 3, 4, zone);
        assert_eq!(level[10 * nx + 26], 0);
        assert_eq!(level[3 * nx + 26], 0);
    }
}

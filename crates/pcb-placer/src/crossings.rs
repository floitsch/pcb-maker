// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Ratsnest crossings (`Problem::crossings`, docs/placer.md "Ratsnest
//! crossings (2026-10-24)"): per counted net the minimum spanning tree of
//! its pins (Prim's, straight distance), and the pairs of tree lines of
//! different nets that properly cross. Wirelength cannot see two orders of
//! the same parts along a bus; the crossings can, and on a board of few
//! layers each one is a via pair or a detour.
//!
//! The count is the placement ablation's (`pcb-maker
//! ablate-kicad-placement`, which calls `spanning_tree` and
//! `lines_cross` here). `CrossingState` keeps it incrementally: a move or
//! turn of a part changes only the trees of the nets on its pins, so a
//! move is judged by the crossings of those nets' lines, counted against
//! the others through a bucket grid the lines are walked through (each
//! line in every cell it passes): exact, and independent of the order the
//! lines were added in.

use crate::problem::{Point, Pose, Problem};

/// The lowering's crossings fact and the term's weight.
#[derive(Clone, Debug)]
pub struct Crossings {
    /// Per net: whether its tree counts (two pins or more, not carried by
    /// a pour: escape demand's nets).
    pub counted: Vec<bool>,
    /// Millimetres of wirelength one crossing costs in the legaliser and
    /// the refinement.
    pub weight: f64,
    /// The legaliser pays for the crossings a spot adds.
    pub legalise: bool,
    /// The refinement pays for the crossings of a move.
    pub refine: bool,
}

/// The minimum spanning tree of `points` by straight distance (Prim's
/// from the first point; a tie to the earlier point), as pairs of indices
/// in the order the tree takes them.
pub fn spanning_tree(points: &[Point]) -> Vec<(usize, usize)> {
    let count = points.len();
    let mut edges = Vec::with_capacity(count.saturating_sub(1));
    if count < 2 {
        return edges;
    }
    let distance = |a: Point, b: Point| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let mut in_tree = vec![false; count];
    let mut best = vec![(f64::INFINITY, 0usize); count];
    in_tree[0] = true;
    for other in 1..count {
        best[other] = (distance(points[0], points[other]), 0);
    }
    for _ in 1..count {
        let Some(next) = (0..count)
            .filter(|index| !in_tree[*index])
            .min_by(|a, b| best[*a].0.total_cmp(&best[*b].0))
        else {
            break;
        };
        in_tree[next] = true;
        edges.push((best[next].1, next));
        for other in 0..count {
            if !in_tree[other] {
                let d = distance(points[next], points[other]);
                if d < best[other].0 {
                    best[other] = (d, next);
                }
            }
        }
    }
    edges
}

/// Whether the lines `a`-`b` and `c`-`d` properly cross (bounding boxes
/// first, then the orientation test; touching or collinear lines do not).
pub fn lines_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    if c[0].max(d[0]) < a[0].min(b[0])
        || c[0].min(d[0]) > a[0].max(b[0])
        || c[1].max(d[1]) < a[1].min(b[1])
        || c[1].min(d[1]) > a[1].max(b[1])
    {
        return false;
    }
    let orient = |p: Point, q: Point, r: Point| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let d1 = orient(a, b, c);
    let d2 = orient(a, b, d);
    let d3 = orient(c, d, a);
    let d4 = orient(c, d, b);
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// The board's crossings at `poses`, counted from scratch.
pub fn total(problem: &Problem, crossings: &Crossings, poses: &[Pose]) -> u64 {
    CrossingState::new(problem, crossings, poses).total()
}

/// One tree line.
#[derive(Clone, Copy, Debug)]
struct Line {
    net: usize,
    a: Point,
    b: Point,
}

/// Lines by the cells of a uniform grid they pass through. A point
/// beyond the grid falls into its border cells, so two lines that meet
/// share the cell of their meeting point.
struct LineGrid {
    origin: Point,
    cell: f64,
    columns: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

/// Slack around a line's passage through a column: a meeting point on a
/// cell boundary is in the cells of both lines.
const SLACK: f64 = 1.0e-6;

impl LineGrid {
    fn new(bounds: [f64; 4], lines: usize) -> Self {
        let width = (bounds[2] - bounds[0]).max(1.0);
        let height = (bounds[3] - bounds[1]).max(1.0);
        // About two lines a cell, between 1 and 20 mm, at most 2^16 cells.
        let mut cell = (2.0 * width * height / lines.max(1) as f64)
            .sqrt()
            .clamp(1.0, 20.0);
        while (width / cell).ceil() * (height / cell).ceil() > 65536.0 {
            cell *= 1.5;
        }
        let columns = ((width / cell).ceil() as usize).max(1);
        let rows = ((height / cell).ceil() as usize).max(1);
        LineGrid {
            origin: [bounds[0], bounds[1]],
            cell,
            columns,
            rows,
            cells: vec![Vec::new(); columns * rows],
        }
    }

    fn column(&self, x: f64) -> usize {
        let at = ((x - self.origin[0]) / self.cell).floor();
        if at.is_nan() || at < 0.0 {
            0
        } else {
            (at as usize).min(self.columns - 1)
        }
    }

    fn row(&self, y: f64) -> usize {
        let at = ((y - self.origin[1]) / self.cell).floor();
        if at.is_nan() || at < 0.0 {
            0
        } else {
            (at as usize).min(self.rows - 1)
        }
    }

    /// Calls `visit` with every cell the line from `a` to `b` passes
    /// (with `SLACK`); returns how many.
    fn walk(&self, a: Point, b: Point, mut visit: impl FnMut(usize)) -> usize {
        let (low, high) = if a[0] <= b[0] { (a, b) } else { (b, a) };
        let first = self.column(low[0] - SLACK);
        let last = self.column(high[0] + SLACK);
        let dx = high[0] - low[0];
        let mut visited = 0;
        for column in first..=last {
            // The column's stretch of x (the border columns reach on).
            let left = if column == 0 {
                f64::NEG_INFINITY
            } else {
                self.origin[0] + column as f64 * self.cell
            };
            let right = if column + 1 == self.columns {
                f64::INFINITY
            } else {
                self.origin[0] + (column + 1) as f64 * self.cell
            };
            let x0 = (low[0] - SLACK).max(left);
            let x1 = (high[0] + SLACK).min(right);
            let (y0, y1) = if dx <= 1.0e-12 {
                (low[1].min(high[1]), low[1].max(high[1]))
            } else {
                let at = |x: f64| {
                    let t = ((x - low[0]) / dx).clamp(0.0, 1.0);
                    low[1] + t * (high[1] - low[1])
                };
                let (p, q) = (at(x0), at(x1));
                (p.min(q), p.max(q))
            };
            for row in self.row(y0 - SLACK)..=self.row(y1 + SLACK) {
                visit(row * self.columns + column);
                visited += 1;
            }
        }
        visited
    }

    fn insert(&mut self, id: u32, line: &Line) {
        let mut cells = Vec::new();
        self.walk(line.a, line.b, |cell| cells.push(cell));
        for cell in cells {
            self.cells[cell].push(id);
        }
    }

    fn remove(&mut self, id: u32, line: &Line) {
        let mut cells = Vec::new();
        self.walk(line.a, line.b, |cell| cells.push(cell));
        for cell in cells {
            let list = &mut self.cells[cell];
            if let Some(position) = list.iter().position(|other| *other == id) {
                list.swap_remove(position);
            }
        }
    }
}

/// The crossings term of the legaliser and the refinement: every counted
/// net's tree lines at the poses last kept, in a grid, and their
/// crossings.
pub struct CrossingState<'a> {
    problem: &'a Problem,
    crossings: &'a Crossings,
    /// Per net, its pins as (component, pin).
    members: Vec<Vec<(usize, usize)>>,
    /// Per part, the counted nets on its pins.
    incident: Vec<Vec<usize>>,
    /// Per net, its lines' ids.
    net_lines: Vec<Vec<u32>>,
    lines: Vec<Line>,
    free: Vec<u32>,
    grid: LineGrid,
    /// Per line, the stamp of the query that saw it last.
    seen: Vec<u32>,
    stamp: u32,
    /// Per line, the stamp of the query it is a subject of.
    subject: Vec<u32>,
    /// Per net, the stamp of the query that touches it.
    touched: Vec<u32>,
    query: u32,
    total: u64,
    points: Vec<Point>,
    /// The poses the lines were kept at.
    poses: Vec<Pose>,
}

impl<'a> CrossingState<'a> {
    pub fn new(problem: &'a Problem, crossings: &'a Crossings, poses: &[Pose]) -> Self {
        let nets = problem.net_weights.len();
        let mut members = vec![Vec::new(); nets];
        for (index, component) in problem.components.iter().enumerate() {
            for (pin, description) in component.pins.iter().enumerate() {
                members[description.net].push((index, pin));
            }
        }
        let counted = |net: usize| crossings.counted.get(net).copied().unwrap_or(false);
        let incident = problem
            .components
            .iter()
            .map(|component| {
                let mut nets: Vec<usize> = component
                    .pins
                    .iter()
                    .map(|pin| pin.net)
                    .filter(|net| counted(*net))
                    .collect();
                nets.sort_unstable();
                nets.dedup();
                nets
            })
            .collect();
        let line_count: usize = members
            .iter()
            .enumerate()
            .filter(|(net, pins)| counted(*net) && pins.len() >= 2)
            .map(|(_, pins)| pins.len() - 1)
            .sum();
        // The grid covers the pins where they are now and the board.
        let mut bounds = problem.bounds();
        for (component, pose) in problem.components.iter().zip(poses) {
            for pin in &component.pins {
                let at = component.pin_position(pin, *pose);
                bounds[0] = bounds[0].min(at[0]);
                bounds[1] = bounds[1].min(at[1]);
                bounds[2] = bounds[2].max(at[0]);
                bounds[3] = bounds[3].max(at[1]);
            }
        }
        if !bounds[0].is_finite() {
            bounds = [0.0, 0.0, 1.0, 1.0];
        }
        let mut state = CrossingState {
            problem,
            crossings,
            members,
            incident,
            net_lines: vec![Vec::new(); nets],
            lines: Vec::new(),
            free: Vec::new(),
            grid: LineGrid::new(bounds, line_count),
            seen: Vec::new(),
            stamp: 0,
            subject: Vec::new(),
            touched: vec![0; nets],
            query: 0,
            total: 0,
            points: Vec::new(),
            poses: poses.to_vec(),
        };
        let all: Vec<usize> = (0..nets).filter(|net| counted(*net)).collect();
        for &net in &all {
            let ids = state.add_tree(poses, net);
            state.net_lines[net] = ids;
        }
        // Each crossing once: every line against the lines after it.
        let ids: Vec<u32> = all
            .iter()
            .flat_map(|net| state.net_lines[*net].iter().copied())
            .collect();
        state.total = state.count(&ids, &[]);
        state
    }

    /// The board's crossings at the poses last kept.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// The counted nets on the pins of `parts`, sorted, each once.
    fn nets_of(&self, parts: &[usize]) -> Vec<usize> {
        let mut nets: Vec<usize> = parts
            .iter()
            .flat_map(|part| self.incident[*part].iter().copied())
            .collect();
        nets.sort_unstable();
        nets.dedup();
        nets
    }

    /// The tree of `net` at `poses`, its lines added to the grid; their
    /// ids.
    fn add_tree(&mut self, poses: &[Pose], net: usize) -> Vec<u32> {
        let mut points = std::mem::take(&mut self.points);
        points.clear();
        for (part, pin) in &self.members[net] {
            let component = &self.problem.components[*part];
            points.push(component.pin_position(&component.pins[*pin], poses[*part]));
        }
        let edges = spanning_tree(&points);
        let pins = points.len() as u64;
        crate::add_work(crate::PIN_WORK * pins + pins * pins);
        let mut ids = Vec::with_capacity(edges.len());
        for (from, to) in edges {
            let line = Line {
                net,
                a: points[from],
                b: points[to],
            };
            let id = match self.free.pop() {
                Some(id) => {
                    self.lines[id as usize] = line;
                    id
                }
                None => {
                    self.lines.push(line);
                    self.seen.push(0);
                    self.subject.push(0);
                    (self.lines.len() - 1) as u32
                }
            };
            self.grid.insert(id, &line);
            ids.push(id);
        }
        self.points = points;
        ids
    }

    fn drop_lines(&mut self, ids: &[u32]) {
        for &id in ids {
            let line = self.lines[id as usize];
            self.grid.remove(id, &line);
            self.free.push(id);
        }
    }

    fn next_stamp(stamp: &mut u32, marks: &mut [u32]) -> u32 {
        *stamp = stamp.wrapping_add(1);
        if *stamp == 0 {
            marks.iter_mut().for_each(|mark| *mark = 0);
            *stamp = 1;
        }
        *stamp
    }

    /// The crossings of the lines `ids` with every line in the grid but
    /// the lines of the nets `hidden` that are not among `ids` (a moved
    /// part's nets' trees as kept, while `ids` holds their new ones), each
    /// pair once.
    fn count(&mut self, ids: &[u32], hidden: &[usize]) -> u64 {
        let query = Self::next_stamp(&mut self.query, &mut self.subject);
        if query == 1 {
            self.touched.iter_mut().for_each(|mark| *mark = 0);
        }
        for &net in hidden {
            self.touched[net] = query;
        }
        for &id in ids {
            self.subject[id as usize] = query;
        }
        let mut found = 0u64;
        let mut work = 0u64;
        let mut near: Vec<u32> = Vec::new();
        for &id in ids {
            let line = self.lines[id as usize];
            let stamp = Self::next_stamp(&mut self.stamp, &mut self.seen);
            near.clear();
            let (grid, seen) = (&self.grid, &mut self.seen);
            work += grid.walk(line.a, line.b, |cell| {
                for &other in &grid.cells[cell] {
                    if seen[other as usize] != stamp {
                        seen[other as usize] = stamp;
                        near.push(other);
                    }
                }
            }) as u64;
            work += near.len() as u64;
            for &other in &near {
                let theirs = self.lines[other as usize];
                if theirs.net == line.net {
                    continue;
                }
                if self.subject[other as usize] == query {
                    // Both subjects: counted from the earlier one.
                    if other <= id {
                        continue;
                    }
                } else if self.touched[theirs.net] == query {
                    continue;
                }
                if lines_cross(line.a, line.b, theirs.a, theirs.b) {
                    found += 1;
                }
            }
        }
        crate::add_work(work);
        found
    }

    /// The crossings of the trees of the nets on the pins of `moved` with
    /// all lines, the parts where `poses` has them; the lines kept are
    /// unchanged.
    pub fn involving(&mut self, poses: &[Pose], moved: &[usize]) -> u64 {
        let nets = self.nets_of(moved);
        if nets.is_empty() {
            return 0;
        }
        let mut fresh = Vec::new();
        for &net in &nets {
            let ids = self.add_tree(poses, net);
            fresh.extend(ids);
        }
        let found = self.count(&fresh, &nets);
        self.drop_lines(&fresh);
        found
    }

    /// After a kept move of `moved` (to where `poses` has them): their
    /// nets' trees again, and the total.
    pub fn kept(&mut self, poses: &[Pose], moved: &[usize]) {
        for &part in moved {
            self.poses[part] = poses[part];
        }
        let nets = self.nets_of(moved);
        if nets.is_empty() {
            return;
        }
        let old: Vec<u32> = nets
            .iter()
            .flat_map(|net| self.net_lines[*net].iter().copied())
            .collect();
        let before = self.count(&old, &[]);
        self.drop_lines(&old);
        let mut fresh = Vec::new();
        for &net in &nets {
            let ids = self.add_tree(poses, net);
            fresh.extend(ids.iter().copied());
            self.net_lines[net] = ids;
        }
        let after = self.count(&fresh, &[]);
        self.total = self.total + after - before;
    }

    /// Every part's trees again at `poses` where a part moved since they
    /// were kept (moves made by others between two refinements).
    pub fn resync(&mut self, poses: &[Pose]) {
        let moved: Vec<usize> = (0..poses.len().min(self.poses.len()))
            .filter(|index| self.poses[*index] != poses[*index])
            .collect();
        if !moved.is_empty() {
            self.kept(poses, &moved);
        }
    }

    /// The weight: millimetres of wirelength per crossing.
    pub fn weight(&self) -> f64 {
        self.crossings.weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::{Component, Pin, Side};

    fn part(name: &str, pins: Vec<Pin>) -> Component {
        Component {
            name: name.into(),
            body_center: [0.0, 0.0],
            body_size: [1.0, 1.0],
            round: false,
            halo: 0.0,
            pins,
            side: Side::Front,
            fixed: false,
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
        }
    }

    fn pose(x: f64, y: f64) -> Pose {
        Pose {
            position: [x, y],
            angle: 0.0,
        }
    }

    fn problem(components: Vec<Component>, nets: usize, poses: Vec<Pose>) -> Problem {
        Problem {
            outline: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            components,
            net_weights: vec![1.0; nets],
            poses,
            spacing: 0.2,
            grid: 0.1,
            edge_margin: 0.5,
            min_spacing: 0.2,
            constraints: Default::default(),
            far_side_pads_only: false,
            shared_halos: false,
            pieces: Vec::new(),
            escape: None,
            crossings: None,
        }
    }

    /// Every pair of lines of different nets, counted directly.
    fn brute(problem: &Problem, crossings: &Crossings, poses: &[Pose]) -> u64 {
        let mut lines: Vec<(usize, Point, Point)> = Vec::new();
        for net in 0..problem.net_weights.len() {
            if !crossings.counted[net] {
                continue;
            }
            let mut points = Vec::new();
            for (component, pose) in problem.components.iter().zip(poses) {
                for pin in &component.pins {
                    if pin.net == net {
                        points.push(component.pin_position(pin, *pose));
                    }
                }
            }
            for (a, b) in spanning_tree(&points) {
                lines.push((net, points[a], points[b]));
            }
        }
        let mut count = 0;
        for (index, first) in lines.iter().enumerate() {
            for second in &lines[index + 1..] {
                if first.0 != second.0 && lines_cross(first.1, first.2, second.1, second.2) {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    fn the_tree_is_prims_with_ties_to_the_earlier_point() {
        let points = [[0.0, 0.0], [10.0, 0.0], [1.0, 0.0], [5.0, 0.0]];
        assert_eq!(spanning_tree(&points), vec![(0, 2), (2, 3), (3, 1)]);
        assert!(spanning_tree(&points[..1]).is_empty());
    }

    #[test]
    fn touching_and_collinear_lines_do_not_cross() {
        assert!(lines_cross([0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0]));
        // An end on the other line.
        assert!(!lines_cross([0.0, 0.0], [2.0, 2.0], [1.0, 1.0], [2.0, 0.0]));
        assert!(!lines_cross([0.0, 0.0], [2.0, 0.0], [1.0, 0.0], [3.0, 0.0]));
        assert!(!lines_cross([0.0, 0.0], [1.0, 1.0], [2.0, 0.0], [3.0, 1.0]));
    }

    /// Two parts in a row along a bus of two nets to a third: swapped,
    /// their lines cross.
    #[test]
    fn a_swapped_order_along_a_bus_crosses() {
        let components = vec![
            part(
                "A",
                vec![Pin {
                    offset: [0.0, 0.0],
                    net: 0,
                }],
            ),
            part(
                "B",
                vec![Pin {
                    offset: [0.0, 0.0],
                    net: 1,
                }],
            ),
            part(
                "U",
                vec![
                    Pin {
                        offset: [0.0, 0.0],
                        net: 0,
                    },
                    Pin {
                        offset: [5.0, 0.0],
                        net: 1,
                    },
                ],
            ),
        ];
        let poses = vec![pose(10.0, 10.0), pose(15.0, 10.0), pose(10.0, 20.0)];
        let problem = problem(components, 2, poses.clone());
        let crossings = Crossings {
            counted: vec![true; 2],
            weight: 1.0,
            legalise: true,
            refine: true,
        };
        let mut state = CrossingState::new(&problem, &crossings, &poses);
        assert_eq!(state.total(), 0);
        let mut swapped = poses.clone();
        swapped.swap(0, 1);
        assert_eq!(state.involving(&swapped, &[0, 1]), 1);
        // Asking does not change the state.
        assert_eq!(state.total(), 0);
        assert_eq!(state.involving(&poses, &[0]), 0);
        state.kept(&swapped, &[0, 1]);
        assert_eq!(state.total(), 1);
        assert_eq!(total(&problem, &crossings, &swapped), 1);
    }

    /// The refinement with the term on swaps two equal parts whose lines
    /// to a fixed part cross.
    #[test]
    fn the_refinement_uncrosses_a_bus() {
        let mut components = vec![
            part(
                "A",
                vec![Pin {
                    offset: [0.0, 0.0],
                    net: 0,
                }],
            ),
            part(
                "B",
                vec![Pin {
                    offset: [0.0, 0.0],
                    net: 1,
                }],
            ),
            part(
                "U",
                vec![
                    Pin {
                        offset: [0.0, 0.0],
                        net: 0,
                    },
                    Pin {
                        offset: [5.0, 0.0],
                        net: 1,
                    },
                ],
            ),
        ];
        components[2].fixed = true;
        components[2].body_size = [8.0, 2.0];
        components[2].body_center = [2.5, 0.0];
        let mut poses = vec![pose(55.0, 40.0), pose(50.0, 40.0), pose(50.0, 60.0)];
        let mut problem = problem(components, 2, poses.clone());
        let crossings = Crossings {
            counted: vec![true; 2],
            weight: 3.0,
            legalise: true,
            refine: true,
        };
        assert_eq!(total(&problem, &crossings, &poses), 1);
        problem.crossings = Some(std::sync::Arc::new(crossings.clone()));
        crate::legal::refine(&problem, &mut poses, 3);
        assert_eq!(total(&problem, &crossings, &poses), 0);
    }

    /// Random parts and nets, random moves and turns: the incremental
    /// count equals the direct one after every kept move, and `involving`
    /// gives the change a move makes.
    #[test]
    fn the_incremental_count_is_exact() {
        let mut seed = 12345u64;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let nets = 14;
        let mut components = Vec::new();
        let mut poses = Vec::new();
        for index in 0..24 {
            let pins = 1 + (random() * 5.0) as usize;
            let pins = (0..pins)
                .map(|_| Pin {
                    offset: [random() * 4.0 - 2.0, random() * 4.0 - 2.0],
                    net: (random() * nets as f64) as usize,
                })
                .collect();
            components.push(part(&format!("P{index}"), pins));
            // Some beyond the board: the grid's border cells.
            poses.push(pose(random() * 130.0 - 15.0, random() * 120.0 - 10.0));
        }
        let problem = problem(components, nets, poses.clone());
        let mut counted = vec![true; nets];
        counted[3] = false;
        let crossings = Crossings {
            counted,
            weight: 1.0,
            legalise: true,
            refine: true,
        };
        let mut state = CrossingState::new(&problem, &crossings, &poses);
        assert_eq!(state.total(), brute(&problem, &crossings, &poses));
        for step in 0..300 {
            let a = (random() * 24.0) as usize;
            let b = (random() * 24.0) as usize;
            let mut moved = vec![a];
            let mut next = poses.clone();
            if step % 3 == 0 && a != b {
                next.swap(a, b);
                moved.push(b);
            } else {
                next[a] = Pose {
                    position: [random() * 110.0 - 5.0, random() * 110.0 - 5.0],
                    angle: [0.0, 90.0, 180.0, 270.0][(random() * 4.0) as usize],
                };
            }
            let before = state.involving(&poses, &moved);
            let after = state.involving(&next, &moved);
            let expected = brute(&problem, &crossings, &next);
            assert_eq!(
                state.total() as i64 + after as i64 - before as i64,
                expected as i64
            );
            if random() < 0.7 {
                state.kept(&next, &moved);
                poses = next;
                assert_eq!(state.total(), expected);
            }
        }
    }
}

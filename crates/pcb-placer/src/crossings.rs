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
    /// The anneal pays for the crossings of a move, and a swap's partner
    /// is often a part like it (`anneal::peers`).
    pub anneal: bool,
    /// After the global placement, the parts with many pins trade places
    /// and turn where that saves wirelength plus crossings (`reorder`).
    pub global: bool,
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

/// Profiling of the count's work against its time
/// (`PCB_PLACER_CROSSINGS_PROFILE` with `PCB_PLACER_DEBUG`: the placer's
/// phase lines say per phase the wall time spent in the count, the work
/// it booked and what it did, docs/placer.md "Ratsnest crossings").
pub mod profile {
    use std::cell::Cell;

    /// What the count did on this thread since the process started.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct Tally {
        /// Wall time inside the count's calls, nanoseconds.
        pub nanos: u64,
        /// The work those calls booked.
        pub booked: u64,
        /// Calls (outermost).
        pub calls: u64,
        /// Pins of the trees built.
        pub pins: u64,
        /// Prim's k^2 over the trees built.
        pub prim: u64,
        /// Tree lines built.
        pub lines: u64,
        /// Cells walked by the queries.
        pub cells: u64,
        /// Lines met in those cells (each once a query line).
        pub met: u64,
        /// Lines met that were tested for a crossing.
        pub tested: u64,
        /// Cells walked to insert or remove lines.
        pub grid_cells: u64,
        /// Entries scanned in the cells' lists to remove lines.
        pub scanned: u64,
    }

    thread_local! {
        static TALLY: Cell<Tally> = const { Cell::new(Tally {
            nanos: 0, booked: 0, calls: 0, pins: 0, prim: 0, lines: 0, cells: 0,
            met: 0, tested: 0, grid_cells: 0, scanned: 0,
        }) };
        static DEPTH: Cell<u32> = const { Cell::new(0) };
    }

    /// Whether profiling is on (`PCB_PLACER_CROSSINGS_PROFILE`).
    pub fn enabled() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("PCB_PLACER_CROSSINGS_PROFILE").is_some())
    }

    /// This thread's tally so far.
    pub fn tally() -> Tally {
        TALLY.with(|tally| tally.get())
    }

    pub(crate) fn add(change: impl FnOnce(&mut Tally)) {
        if enabled() {
            TALLY.with(|cell| {
                let mut tally = cell.get();
                change(&mut tally);
                cell.set(tally);
            });
        }
    }

    /// Times a call of the count from its creation to its drop (the
    /// outermost one only).
    pub(crate) struct Timed(Option<(std::time::Instant, u64)>);

    impl Timed {
        pub(crate) fn start() -> Self {
            if !enabled() {
                return Timed(None);
            }
            let depth = DEPTH.with(|depth| {
                let now = depth.get();
                depth.set(now + 1);
                now
            });
            Timed((depth == 0).then(|| (std::time::Instant::now(), crate::work())))
        }
    }

    impl Drop for Timed {
        fn drop(&mut self) {
            if !enabled() {
                return;
            }
            DEPTH.with(|depth| depth.set(depth.get() - 1));
            if let Some((started, work)) = self.0 {
                let nanos = started.elapsed().as_nanos() as u64;
                let booked = crate::work() - work;
                add(|tally| {
                    tally.nanos += nanos;
                    tally.booked += booked;
                    tally.calls += 1;
                });
            }
        }
    }

    /// `after` minus `before`, field by field.
    pub fn since(before: Tally, after: Tally) -> Tally {
        Tally {
            nanos: after.nanos - before.nanos,
            booked: after.booked - before.booked,
            calls: after.calls - before.calls,
            pins: after.pins - before.pins,
            prim: after.prim - before.prim,
            lines: after.lines - before.lines,
            cells: after.cells - before.cells,
            met: after.met - before.met,
            tested: after.tested - before.tested,
            grid_cells: after.grid_cells - before.grid_cells,
            scanned: after.scanned - before.scanned,
        }
    }
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
    /// The cells of the line being inserted or removed.
    scratch: Vec<usize>,
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
            scratch: Vec::new(),
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
        let mut cells = std::mem::take(&mut self.scratch);
        cells.clear();
        self.walk(line.a, line.b, |cell| cells.push(cell));
        profile::add(|tally| tally.grid_cells += cells.len() as u64);
        for &cell in &cells {
            self.cells[cell].push(id);
        }
        self.scratch = cells;
    }

    fn remove(&mut self, id: u32, line: &Line) {
        let mut cells = std::mem::take(&mut self.scratch);
        cells.clear();
        self.walk(line.a, line.b, |cell| cells.push(cell));
        let mut scanned = 0;
        let walked = cells.len() as u64;
        for &cell in &cells {
            let list = &mut self.cells[cell];
            if let Some(position) = list.iter().position(|other| *other == id) {
                scanned += position as u64 + 1;
                list.swap_remove(position);
            }
        }
        self.scratch = cells;
        profile::add(|tally| {
            tally.grid_cells += walked;
            tally.scanned += scanned;
        });
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
    /// A proposed move (`before`, `after`, `settle`): the moved parts'
    /// nets, their new lines per net, and whether those were built.
    pending_nets: Vec<usize>,
    pending_lines: Vec<(usize, Vec<u32>)>,
    pending_built: bool,
    /// Whether the pending lines are in the grid (more than
    /// `DIRECT_LINES` of them) or were counted against each other
    /// directly.
    pending_inserted: bool,
    /// Buffers of `count` and `before`.
    near: Vec<u32>,
    ids: Vec<u32>,
}

/// A pending move's new lines up to this many are counted against each
/// other directly and enter the grid only when the move is kept: the
/// anneal rejects most moves, and a rejected one then never touches the
/// grid's lists.
const DIRECT_LINES: usize = 48;

impl<'a> CrossingState<'a> {
    pub fn new(problem: &'a Problem, crossings: &'a Crossings, poses: &[Pose]) -> Self {
        let _timed = profile::Timed::start();
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
            pending_nets: Vec::new(),
            pending_lines: Vec::new(),
            pending_built: false,
            pending_inserted: false,
            near: Vec::new(),
            ids: Vec::new(),
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
        self.tree(poses, net, true)
    }

    /// The tree of `net` at `poses`, its lines in the grid where `insert`;
    /// their ids.
    fn tree(&mut self, poses: &[Pose], net: usize, insert: bool) -> Vec<u32> {
        let mut points = std::mem::take(&mut self.points);
        points.clear();
        for (part, pin) in &self.members[net] {
            let component = &self.problem.components[*part];
            points.push(component.pin_position(&component.pins[*pin], poses[*part]));
        }
        let edges = spanning_tree(&points);
        let pins = points.len() as u64;
        crate::add_work(crate::PIN_WORK * pins + pins * pins);
        profile::add(|tally| {
            tally.pins += pins;
            tally.prim += pins * pins;
            tally.lines += edges.len() as u64;
        });
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
            if insert {
                self.grid.insert(id, &line);
            }
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
        let mut tested = 0u64;
        let mut near_total = 0u64;
        let mut near = std::mem::take(&mut self.near);
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
            near_total += near.len() as u64;
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
                tested += 1;
                if lines_cross(line.a, line.b, theirs.a, theirs.b) {
                    found += 1;
                }
            }
        }
        self.near = near;
        crate::add_work(work);
        profile::add(|tally| {
            tally.cells += work - near_total;
            tally.met += near_total;
            tally.tested += tested;
        });
        found
    }

    /// The crossings of the trees of the nets on the pins of `moved` with
    /// all lines, the parts where `poses` has them; the lines kept are
    /// unchanged.
    pub fn involving(&mut self, poses: &[Pose], moved: &[usize]) -> u64 {
        let _timed = profile::Timed::start();
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
        let _timed = profile::Timed::start();
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
        let _timed = profile::Timed::start();
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

    /// A move of `moved` in three steps, for the anneal, which judges
    /// moves by the hundred thousand: the crossings of the moved parts'
    /// nets as kept (their lines are in the grid already: no tree is
    /// built, and a bound on how much the move can save), then `after`
    /// at the new poses, then `settle` keeps or drops them. `before` minus
    /// `after` is what `involving` gives at the old and the new poses.
    pub fn before(&mut self, moved: &[usize]) -> u64 {
        debug_assert!(self.pending_nets.is_empty() && !self.pending_built);
        self.pending_nets = self.nets_of(moved);
        if self.pending_nets.is_empty() {
            return 0;
        }
        let mut old = std::mem::take(&mut self.ids);
        old.clear();
        for &net in &self.pending_nets {
            old.extend(self.net_lines[net].iter().copied());
        }
        let found = self.count(&old, &[]);
        self.ids = old;
        found
    }

    /// The crossings of the moved parts' nets (since `before`) at
    /// `poses`, their new lines held until `settle`.
    pub fn after(&mut self, poses: &[Pose]) -> u64 {
        debug_assert!(!self.pending_built);
        self.pending_built = true;
        if self.pending_nets.is_empty() {
            return 0;
        }
        let nets = std::mem::take(&mut self.pending_nets);
        let mut fresh = std::mem::take(&mut self.ids);
        fresh.clear();
        let mut lines = Vec::with_capacity(nets.len());
        for &net in &nets {
            let ids = self.tree(poses, net, false);
            fresh.extend(ids.iter().copied());
            lines.push((net, ids));
        }
        let inserted = fresh.len() > DIRECT_LINES;
        let mut found;
        if inserted {
            for &id in &fresh {
                let line = self.lines[id as usize];
                self.grid.insert(id, &line);
            }
            found = self.count(&fresh, &nets);
        } else {
            // Against the kept lines through the grid, among themselves
            // directly.
            found = self.count(&fresh, &nets);
            let mut work = 0u64;
            for (index, &first) in fresh.iter().enumerate() {
                let one = self.lines[first as usize];
                for &second in &fresh[index + 1..] {
                    let other = self.lines[second as usize];
                    work += 1;
                    if one.net != other.net && lines_cross(one.a, one.b, other.a, other.b) {
                        found += 1;
                    }
                }
            }
            crate::add_work(work);
        }
        self.ids = fresh;
        self.pending_nets = nets;
        self.pending_lines = lines;
        self.pending_inserted = inserted;
        found
    }

    /// Ends the move `before` began: with `keep` (after `after`, the
    /// counts it and `before` gave as `change`) the new lines replace the
    /// kept ones and the parts' poses are `poses`'; otherwise the new
    /// lines go.
    pub fn settle(&mut self, poses: &[Pose], moved: &[usize], keep: bool, change: i64) {
        let lines = std::mem::take(&mut self.pending_lines);
        if keep {
            assert!(self.pending_built, "a kept move was counted at its poses");
            for &part in moved {
                self.poses[part] = poses[part];
            }
            for (net, ids) in lines {
                if !self.pending_inserted {
                    for &id in &ids {
                        let line = self.lines[id as usize];
                        self.grid.insert(id, &line);
                    }
                }
                let old = std::mem::replace(&mut self.net_lines[net], ids);
                self.drop_lines(&old);
            }
            self.total = (self.total as i64 + change) as u64;
        } else {
            for (_, ids) in lines {
                if self.pending_inserted {
                    self.drop_lines(&ids);
                } else {
                    self.free.extend(ids);
                }
            }
        }
        self.pending_inserted = false;
        self.pending_nets.clear();
        self.pending_built = false;
    }
}

/// The least share of a part's pins its trading partner in `reorder`
/// has (QFP-128 and QFP-160: 0.8).
pub const REORDER_PIN_SHARE: f64 = 0.5;

/// Parts with at least this many pins take part in `reorder`.
pub const REORDER_PINS: usize = 16;

/// Passes of `reorder` at most.
const REORDER_PASSES: usize = 4;

/// What `reorder` did.
#[derive(Clone, Debug, Default)]
pub struct Reordered {
    pub parts: usize,
    pub swaps: usize,
    pub turns: usize,
    pub crossings_before: u64,
    pub crossings_after: u64,
}

/// After the global placement: the movable parts with many pins
/// (`REORDER_PINS`) trade places (each to the other's body centre, angles
/// kept) and turn (about their body centre) where that lowers the
/// weighted half-perimeter wirelength of their nets plus `weight` times
/// their nets' crossings: each candidate in turn, taken when it improves,
/// in passes until one improves nothing (at most `REORDER_PASSES`; trades
/// only between parts on one side, the one with fewer pins having at
/// least `REORDER_PIN_SHARE` of the other's). The order
/// of large parts along a bus (MegaDrive's QFPs) is a decision the anneal
/// starts from cold and the legaliser and the refinement can only repair
/// locally; here it is a handful of parts against each other. Overlaps
/// the trade makes are the anneal's to resolve; a trade or turn that
/// breaks a placement constraint or relation of the part is not taken.
/// Measured on MegaDrive and Glasgow it changed little (docs/placer.md,
/// "Ratsnest crossings where the order is decided"); off by default.
pub fn reorder(problem: &Problem, crossings: &Crossings, poses: &mut [Pose]) -> Reordered {
    let large: Vec<usize> = (0..problem.components.len())
        .filter(|index| {
            let component = &problem.components[*index];
            !component.fixed && component.pins.len() >= REORDER_PINS
        })
        .collect();
    let mut result = Reordered {
        parts: large.len(),
        ..Default::default()
    };
    if large.is_empty() {
        return result;
    }
    let nets = problem.net_weights.len();
    let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nets];
    for (index, component) in problem.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            members[description.net].push((index, pin));
        }
    }
    let incident: Vec<Vec<usize>> = problem
        .components
        .iter()
        .map(|component| {
            let mut nets: Vec<usize> = component.pins.iter().map(|pin| pin.net).collect();
            nets.sort_unstable();
            nets.dedup();
            nets
        })
        .collect();
    let wirelength = |poses: &[Pose], nets: &[usize]| -> f64 {
        crate::add_work(
            crate::PIN_WORK * nets.iter().map(|net| members[*net].len()).sum::<usize>() as u64,
        );
        nets.iter()
            .filter(|net| members[**net].len() >= 2)
            .map(|net| {
                let mut bounds = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for (part, pin) in &members[*net] {
                    let component = &problem.components[*part];
                    let at = component.pin_position(&component.pins[*pin], poses[*part]);
                    bounds[0] = bounds[0].min(at[0]);
                    bounds[1] = bounds[1].min(at[1]);
                    bounds[2] = bounds[2].max(at[0]);
                    bounds[3] = bounds[3].max(at[1]);
                }
                problem.net_weights[*net] * ((bounds[2] - bounds[0]) + (bounds[3] - bounds[1]))
            })
            .sum()
    };
    // A part's constraints, kept or not: a trade or turn may not make them
    // worse.
    let held = |poses: &[Pose], part: usize| -> f64 {
        if problem.constraints.is_empty() {
            0.0
        } else {
            crate::constraints::relation_penalty(problem, poses, Some(part))
                + 1000.0 * crate::constraints::hard_violation(problem, part, poses[part])
        }
    };
    let mut state = CrossingState::new(problem, crossings, poses);
    result.crossings_before = state.total();
    let weight = crossings.weight;
    // Whether `changes` lower the cost; taken if so.
    let mut try_change = |poses: &mut [Pose], changes: &[(usize, Pose)]| -> bool {
        let parts: Vec<usize> = changes.iter().map(|(part, _)| *part).collect();
        let mut nets: Vec<usize> = parts
            .iter()
            .flat_map(|part| incident[*part].iter().copied())
            .collect();
        nets.sort_unstable();
        nets.dedup();
        let old: Vec<(usize, Pose)> = parts.iter().map(|part| (*part, poses[*part])).collect();
        let held_before: f64 = parts.iter().map(|part| held(poses, *part)).sum();
        let length_before = wirelength(poses, &nets);
        let crossings_before = state.before(&parts);
        for (part, pose) in changes {
            poses[*part] = *pose;
        }
        let held_after: f64 = parts.iter().map(|part| held(poses, *part)).sum();
        let mut better = false;
        let mut change = 0;
        if held_after <= held_before + 1.0e-9 {
            let crossings_after = state.after(poses);
            change = crossings_after as i64 - crossings_before as i64;
            let delta = wirelength(poses, &nets) - length_before + weight * change as f64;
            better = delta < -1.0e-9;
        }
        state.settle(poses, &parts, better, change);
        if !better {
            for (part, pose) in old {
                poses[part] = pose;
            }
        }
        better
    };
    for _ in 0..REORDER_PASSES {
        let mut improved = false;
        for (position, &a) in large.iter().enumerate() {
            if crate::out_of_work(0) {
                break;
            }
            let first = &problem.components[a];
            // Turns about the body centre.
            for &angle in &first.angle_options {
                if (angle - poses[a].angle).abs() < 1.0e-9 {
                    continue;
                }
                let center = first.center(poses[a]);
                let pose = Pose {
                    position: first.position_for_center(center, angle),
                    angle,
                };
                if try_change(poses, &[(a, pose)]) {
                    result.turns += 1;
                    improved = true;
                }
            }
            for &b in &large[position + 1..] {
                let second = &problem.components[b];
                let first = &problem.components[a];
                let (fewer, more) = (
                    first.pins.len().min(second.pins.len()),
                    first.pins.len().max(second.pins.len()),
                );
                if first.side != second.side || (fewer as f64) < REORDER_PIN_SHARE * more as f64 {
                    continue;
                }
                let (pose_a, pose_b) = (poses[a], poses[b]);
                let traded = [
                    (
                        a,
                        Pose {
                            position: first
                                .position_for_center(second.center(pose_b), pose_a.angle),
                            angle: pose_a.angle,
                        },
                    ),
                    (
                        b,
                        Pose {
                            position: second
                                .position_for_center(first.center(pose_a), pose_b.angle),
                            angle: pose_b.angle,
                        },
                    ),
                ];
                if try_change(poses, &traded) {
                    result.swaps += 1;
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
    result.crossings_after = state.total();
    result
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
            anneal: false,
            global: false,
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
            anneal: false,
            global: false,
        };
        assert_eq!(total(&problem, &crossings, &poses), 1);
        problem.crossings = Some(std::sync::Arc::new(crossings.clone()));
        crate::legal::refine(&problem, &mut poses, 3);
        assert_eq!(total(&problem, &crossings, &poses), 0);
    }

    /// The anneal with the term on: two equal parts whose lines to a
    /// fixed part cross trade places (peers), and the anneal's own count
    /// stays exact through its moves (checked by the debug assertions of
    /// the three steps and the final total below).
    #[test]
    fn the_anneal_uncrosses_a_bus() {
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
            legalise: false,
            refine: false,
            anneal: true,
            global: false,
        };
        assert_eq!(total(&problem, &crossings, &poses), 1);
        problem.crossings = Some(std::sync::Arc::new(crossings.clone()));
        let config = crate::anneal::AnnealConfig {
            stages: 20,
            moves_per_part: 20,
            ..Default::default()
        };
        crate::anneal::anneal(&problem, &mut poses, &config);
        assert_eq!(total(&problem, &crossings, &poses), 0);
    }

    /// Random parts and nets, random moves and turns: the incremental
    /// count equals the direct one after every kept move, and `involving`
    /// gives the change a move makes.
    #[test]
    fn the_incremental_count_is_exact() {
        // Few lines a move (counted directly) and many (through the grid).
        exactness(24, 14, 12345);
        exactness(60, 8, 777);
    }

    fn exactness(parts: usize, nets: usize, seed: u64) {
        let mut seed = seed;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut components = Vec::new();
        let mut poses = Vec::new();
        for index in 0..parts {
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
            anneal: false,
            global: false,
        };
        let mut state = CrossingState::new(&problem, &crossings, &poses);
        assert_eq!(state.total(), brute(&problem, &crossings, &poses));
        for step in 0..300 {
            let a = (random() * parts as f64) as usize;
            let b = (random() * parts as f64) as usize;
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
            // The anneal's three steps give the same change.
            let early = state.before(&moved);
            assert_eq!(early, before);
            if random() < 0.2 {
                // Rejected by the bound: no new trees.
                state.settle(&next, &moved, false, 0);
                continue;
            }
            let late = state.after(&next);
            assert_eq!(late, after);
            let keep = random() < 0.7;
            state.settle(&next, &moved, keep, late as i64 - early as i64);
            if keep {
                poses = next;
                assert_eq!(state.total(), expected);
            } else {
                assert_eq!(state.total(), brute(&problem, &crossings, &poses));
            }
            if random() < 0.3 {
                // The other way of keeping a move, between the steps.
                let mut again = poses.clone();
                again[a] = Pose {
                    position: [random() * 110.0 - 5.0, random() * 110.0 - 5.0],
                    angle: poses[a].angle,
                };
                state.kept(&again, &[a]);
                poses = again;
                assert_eq!(state.total(), brute(&problem, &crossings, &poses));
            }
        }
    }

    /// Two parts of 16 pins whose buses to a fixed connector cross trade
    /// places after the global placement.
    #[test]
    fn reorder_trades_two_large_parts_along_a_bus() {
        let pins = |first_net: usize| -> Vec<Pin> {
            (0..16)
                .map(|index| Pin {
                    offset: [index as f64 * 0.5 - 3.75, 0.0],
                    net: first_net + index,
                })
                .collect()
        };
        let mut connector = part(
            "J",
            (0..32)
                .map(|index| Pin {
                    offset: [index as f64 * 1.0, 0.0],
                    net: index,
                })
                .collect(),
        );
        connector.fixed = true;
        let components = vec![part("A", pins(0)), part("B", pins(16)), connector];
        // A (nets 0-15) above the connector's right half, B (16-31) above
        // its left: every line of A crosses B's.
        let mut poses = vec![pose(24.0, 40.0), pose(8.0, 40.0), pose(0.0, 20.0)];
        let problem = problem(components, 32, poses.clone());
        let crossings = Crossings {
            counted: vec![true; 32],
            weight: 3.0,
            legalise: false,
            refine: false,
            anneal: false,
            global: true,
        };
        let before = total(&problem, &crossings, &poses);
        assert!(before > 0);
        let result = reorder(&problem, &crossings, &mut poses);
        assert_eq!(result.swaps, 1);
        assert_eq!(result.crossings_before, before);
        assert_eq!(result.crossings_after, total(&problem, &crossings, &poses));
        assert!(result.crossings_after < before);
        assert!(poses[0].position[0] < poses[1].position[0]);
    }
}

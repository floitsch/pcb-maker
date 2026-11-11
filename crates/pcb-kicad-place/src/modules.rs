// Copyright (C) 2026 Toit contributors.

//! Breakaway modules of one board (docs/placer.md, "Modules of one
//! board"): an outline that slots and rows of holes divide into sections
//! joined by narrow tabs the designer runs a few tracks across (protoesp:
//! eight modules, tabs of 3 to 7.5 mm carrying one to three tracks), or
//! separate outline loops of one board (the Modular Hub's three boards).
//! The placement's costs are straight-line and know nothing of the
//! modules, so parts land in the wrong one and the nets that then cross
//! the tabs overflow them.
//!
//! - `detect`: the modules: the outline's pieces with every perforation
//!   chained at `PERFORATION_PITCH_MM` and cut whole
//!   (`Cuts::perforation`), or its loops; per pair of neighbours, the
//!   tracks their tab's gaps pass.
//! - `assign`: each movable part's module from the netlist: a k-way
//!   partition seeded by the parts that stay where they are, the parts a
//!   region holds and the edges the constraints name, each module's parts
//!   kept within its area on each side, minimising the tab crossings of
//!   the nets (the pours' nets and the nets on every module left out)
//!   and their overflow over the tabs' tracks: a greedy start by
//!   attachment, then Fiduccia-Mattheyses passes.
//! - `describe_modules`: the research command `modules-kicad-board`.

use crate::kicad_sexpr::Expr;
use crate::outline_pieces::{Cuts, PERFORATION_PITCH_MM, Pieces};
use pcb_placer as core;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

/// A gap between two holes of a perforation passes one track per this
/// much clear width (mm): protoesp's 1.0 mm gaps between 0.5 mm holes
/// carry one signal, its 2.0 mm gaps one power track.
const TRACK_PASS_MM: f64 = 0.9;

/// How far beside a gap the modules on its two sides are looked for
/// (mm): beyond the opening's fringe (`NECK_RADIUS_MM`, 1 mm).
const SIDE_OFFSETS_MM: [f64; 3] = [1.5, 2.5, 3.5];

/// The share of a module's area (on each side) its parts' bodies may
/// take in the assignment.
const MOST_FILL: f64 = 0.7;

/// A module's box reaches a side of the board when it lies within this
/// of it (mm): a part held at that edge may go there.
const EDGE_REACH_MM: f64 = 2.0;

/// What a track over a tab beyond its capacity costs, in tab crossings.
const OVERFLOW_COST: f64 = 2.0;

/// At most this many Fiduccia-Mattheyses passes from one start.
const MOST_PASSES: usize = 12;

/// The perturbations of the best assignment refined again.
const RESTARTS: usize = 8;

/// How a point finds its module.
#[derive(Clone, Debug)]
enum ModuleMap {
    /// The outline's pieces with the perforations cut whole.
    Pieces(std::sync::Arc<Pieces>),
    /// The outline's loops.
    Loops(Vec<Vec<[f64; 2]>>),
}

/// A board's modules (`detect`).
#[derive(Clone, Debug)]
pub(crate) struct BoardModules {
    map: ModuleMap,
    /// Per module: its box and its area (mm²).
    pub boxes: Vec<[f64; 4]>,
    pub areas: Vec<f64>,
    /// Neighbouring modules and the tracks the gaps of the perforation
    /// between them pass (a and b with a < b; 0 where no gap is wide
    /// enough).
    pub tabs: Vec<(usize, usize, usize)>,
    /// `perforations` or `outline loops`.
    pub kind: &'static str,
}

impl BoardModules {
    pub(crate) fn count(&self) -> usize {
        self.boxes.len()
    }

    /// The module `point` lies on; None off the board or on a tab.
    pub(crate) fn module_at(&self, point: [f64; 2]) -> Option<usize> {
        match &self.map {
            ModuleMap::Pieces(pieces) => pieces.piece_at(point),
            ModuleMap::Loops(loops) => loops.iter().position(|polygon| inside(polygon, point)),
        }
    }
}

fn inside(polygon: &[[f64; 2]], point: [f64; 2]) -> bool {
    let mut result = false;
    for index in 0..polygon.len() {
        let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
        {
            result = !result;
        }
    }
    result
}

fn polygon_area(polygon: &[[f64; 2]]) -> f64 {
    let mut twice = 0.0;
    for index in 0..polygon.len() {
        let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
        twice += a[0] * b[1] - b[0] * a[1];
    }
    twice.abs() / 2.0
}

/// The modules of a board (module comment), None where it has fewer than
/// two or where its outline is a panel of different boards
/// (`outline_pieces::Pieces::of_board`: the sub-boards').
pub(crate) fn detect(pcb: &Expr) -> Result<Option<BoardModules>, String> {
    let loops = crate::outline::board_loops(pcb)?;
    if Pieces::of_loops_cached(&loops.piece_loops, &loops.cutouts, &Cuts::of_board(pcb)).is_some() {
        return Ok(None);
    }
    if loops.piece_loops.len() >= 2 {
        let boxes = loops
            .piece_loops
            .iter()
            .map(|polygon| {
                polygon.iter().fold(
                    [
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ],
                    |b, p| {
                        [
                            b[0].min(p[0]),
                            b[1].min(p[1]),
                            b[2].max(p[0]),
                            b[3].max(p[1]),
                        ]
                    },
                )
            })
            .collect();
        let areas = loops.piece_loops.iter().map(|p| polygon_area(p)).collect();
        return Ok(Some(BoardModules {
            map: ModuleMap::Loops(loops.piece_loops.clone()),
            boxes,
            areas,
            tabs: Vec::new(),
            kind: "outline loops",
        }));
    }
    let (cuts, gaps) = Cuts::perforation(pcb, PERFORATION_PITCH_MM);
    if gaps.is_empty() {
        return Ok(None);
    }
    let Some(pieces) = Pieces::of_loops(&loops.piece_loops, &loops.cutouts, &cuts) else {
        return Ok(None);
    };
    let mut tracks: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for gap in &gaps {
        let [a, b] = gap.ends;
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        if length <= 0.0 {
            continue;
        }
        let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let normal = [(b[1] - a[1]) / length, -(b[0] - a[0]) / length];
        for offset in SIDE_OFFSETS_MM {
            let one = pieces.piece_at([
                middle[0] + offset * normal[0],
                middle[1] + offset * normal[1],
            ]);
            let other = pieces.piece_at([
                middle[0] - offset * normal[0],
                middle[1] - offset * normal[1],
            ]);
            if let (Some(one), Some(other)) = (one, other) {
                if one != other {
                    *tracks.entry((one.min(other), one.max(other))).or_default() +=
                        (gap.clear / TRACK_PASS_MM).floor().max(0.0) as usize;
                }
                break;
            }
        }
    }
    Ok(Some(BoardModules {
        boxes: pieces.boxes.clone(),
        areas: pieces.areas.clone(),
        map: ModuleMap::Pieces(std::sync::Arc::new(pieces)),
        tabs: tracks.into_iter().map(|((a, b), n)| (a, b, n)).collect(),
        kind: "perforations",
    }))
}

/// Each part's module (`assign`).
#[derive(Clone, Debug, Default)]
pub(crate) struct ModuleAssignment {
    /// Per footprint (the problem's first components): its module, None
    /// where nothing says (no pins, or only nets the partition leaves out).
    pub module: Vec<Option<usize>>,
    /// Per footprint: whether its module was given (a part that stays
    /// where it is, or one a constraint holds in one module).
    pub seeded: Vec<bool>,
    /// The nets the partition counts, the module crossings of the
    /// assignment (per net the modules it spans less one), its tab
    /// crossings (per net the tabs a tree over its modules crosses) and
    /// the tracks over the tabs' capacities.
    pub nets_counted: usize,
    pub crossings: usize,
    pub tab_crossings: usize,
    pub overflow: usize,
    /// The modules whose parts take more than `MOST_FILL` of their area on
    /// a side (nothing else would fit them anywhere).
    pub overfull: usize,
    pub passes: usize,
}

/// The cost of a set of modules: tab crossings of a tree over them (the
/// shortest paths of the tab graph joined as a minimum spanning tree;
/// modules no tab joins cost one crossing per module) and the tabs used.
struct Trees {
    /// Per pair: the hops between them, the next module on the way.
    hops: Vec<Vec<u32>>,
    next: Vec<Vec<usize>>,
    tab_index: BTreeMap<(usize, usize), usize>,
    unreachable: u32,
    cache: HashMap<u64, (u32, Vec<usize>)>,
}

impl Trees {
    fn new(modules: &BoardModules) -> Trees {
        let count = modules.count();
        let far = u32::MAX / 4;
        let mut hops = vec![vec![far; count]; count];
        let mut next = vec![vec![usize::MAX; count]; count];
        let mut tab_index = BTreeMap::new();
        for (index, &(a, b, tracks)) in modules.tabs.iter().enumerate() {
            tab_index.insert((a, b), index);
            if tracks > 0 {
                hops[a][b] = 1;
                hops[b][a] = 1;
                next[a][b] = b;
                next[b][a] = a;
            }
        }
        for module in 0..count {
            hops[module][module] = 0;
            next[module][module] = module;
        }
        for k in 0..count {
            for i in 0..count {
                for j in 0..count {
                    if hops[i][k] + hops[k][j] < hops[i][j] {
                        hops[i][j] = hops[i][k] + hops[k][j];
                        next[i][j] = next[i][k];
                    }
                }
            }
        }
        Trees {
            hops,
            next,
            tab_index,
            unreachable: count as u32,
            cache: HashMap::new(),
        }
    }

    fn tree(&mut self, mask: u64) -> (u32, Vec<usize>) {
        if let Some(found) = self.cache.get(&mask) {
            return found.clone();
        }
        let members: Vec<usize> = (0..64).filter(|m| mask & (1 << m) != 0).collect();
        let mut cost = 0u32;
        let mut used: BTreeSet<usize> = BTreeSet::new();
        if members.len() >= 2 {
            let far = u32::MAX / 4;
            let mut joined = vec![members[0]];
            let mut left: Vec<usize> = members[1..].to_vec();
            while !left.is_empty() {
                let mut best = (u32::MAX, 0usize, 0usize);
                for (index, &b) in left.iter().enumerate() {
                    for &a in &joined {
                        let hops = if self.hops[a][b] >= far {
                            self.unreachable
                        } else {
                            self.hops[a][b]
                        };
                        if hops < best.0 {
                            best = (hops, index, a);
                        }
                    }
                }
                let (hops, index, from) = best;
                let to = left.remove(index);
                cost += hops;
                if self.hops[from][to] < far {
                    let mut at = from;
                    while at != to {
                        let step = self.next[at][to];
                        if let Some(tab) = self.tab_index.get(&(at.min(step), at.max(step))) {
                            used.insert(*tab);
                        }
                        at = step;
                    }
                }
                joined.push(to);
            }
        }
        let found = (cost, used.into_iter().collect::<Vec<usize>>());
        self.cache.insert(mask, found.clone());
        found
    }
}

/// A part's body area on each side (front, back).
fn side_area(component: &core::Component) -> [f64; 2] {
    let area = component.body_size[0] * component.body_size[1];
    match component.side {
        core::Side::Front => [area, 0.0],
        core::Side::Back => [0.0, area],
        core::Side::Both => [area, area],
        core::Side::Neither => [0.0, 0.0],
    }
}

/// The partition's state: the assignment, each counted net's pins per
/// module, each module's load per side and each tab's tracks.
struct Partition<'a> {
    modules: &'a BoardModules,
    trees: Trees,
    module: Vec<Option<usize>>,
    /// Per counted net: the parts on it (each once) and its pins per
    /// module.
    net_parts: Vec<Vec<usize>>,
    net_counts: Vec<Vec<u32>>,
    /// Per part: its counted nets.
    part_nets: Vec<Vec<usize>>,
    load: Vec<[f64; 2]>,
    room: Vec<[f64; 2]>,
    area: Vec<[f64; 2]>,
    allowed: Vec<Option<BTreeSet<usize>>>,
    tab_load: Vec<i64>,
}

impl Partition<'_> {
    fn mask(&self, net: usize) -> u64 {
        self.net_counts[net]
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 0)
            .fold(0u64, |mask, (module, _)| mask | (1 << module))
    }

    fn fits(&self, part: usize, module: usize) -> bool {
        if let Some(allowed) = &self.allowed[part]
            && !allowed.contains(&module)
        {
            return false;
        }
        (0..2).all(|side| {
            self.area[part][side] <= 0.0
                || self.load[module][side] + self.area[part][side] <= self.room[module][side]
        })
    }

    fn allowed(&self, part: usize, module: usize) -> bool {
        self.allowed[part]
            .as_ref()
            .is_none_or(|allowed| allowed.contains(&module))
    }

    /// The change of the cost when `part` moves to `to` (negative: better).
    fn delta(&mut self, part: usize, to: usize) -> f64 {
        let Some(from) = self.module[part] else {
            return 0.0;
        };
        let mut hops = 0i64;
        let mut tabs: BTreeMap<usize, i64> = BTreeMap::new();
        for index in 0..self.part_nets[part].len() {
            let net = self.part_nets[part][index];
            let before = self.mask(net);
            let mut after = before;
            if self.net_counts[net][from] == 1 {
                after &= !(1 << from);
            }
            after |= 1 << to;
            if after == before {
                continue;
            }
            let (old_hops, old_tabs) = self.trees.tree(before);
            let (new_hops, new_tabs) = self.trees.tree(after);
            hops += new_hops as i64 - old_hops as i64;
            for tab in old_tabs {
                *tabs.entry(tab).or_default() -= 1;
            }
            for tab in new_tabs {
                *tabs.entry(tab).or_default() += 1;
            }
        }
        let mut overflow = 0i64;
        for (tab, change) in tabs {
            let capacity = self.modules.tabs[tab].2 as i64;
            let load = self.tab_load[tab];
            overflow += (load + change - capacity).max(0) - (load - capacity).max(0);
        }
        hops as f64 + OVERFLOW_COST * overflow as f64
    }

    fn apply(&mut self, part: usize, to: usize) {
        let from = self.module[part];
        for index in 0..self.part_nets[part].len() {
            let net = self.part_nets[part][index];
            let (_, old_tabs) = self.trees.tree(self.mask(net));
            if let Some(from) = from {
                self.net_counts[net][from] -= 1;
            }
            self.net_counts[net][to] += 1;
            let (_, new_tabs) = self.trees.tree(self.mask(net));
            for tab in old_tabs {
                self.tab_load[tab] -= 1;
            }
            for tab in new_tabs {
                self.tab_load[tab] += 1;
            }
        }
        for side in 0..2 {
            if let Some(from) = from {
                self.load[from][side] -= self.area[part][side];
            }
            self.load[to][side] += self.area[part][side];
        }
        self.module[part] = Some(to);
    }

    /// Fiduccia-Mattheyses passes over `movable` (module comment): each
    /// moves every part once, the best move first (a worse one too), and
    /// keeps the best prefix; until a pass gains nothing. The passes run.
    fn refine(&mut self, movable: &[usize], count: usize) -> usize {
        let mut passes = 0;
        for _ in 0..MOST_PASSES {
            passes += 1;
            let mut locked: BTreeSet<usize> = BTreeSet::new();
            let mut history: Vec<(usize, usize)> = Vec::new();
            let mut total = 0.0f64;
            let mut best_total = 0.0f64;
            let mut best_length = 0usize;
            while locked.len() < movable.len() {
                let mut best: Option<(f64, usize, usize)> = None;
                for &part in movable {
                    if locked.contains(&part) {
                        continue;
                    }
                    let from = self.module[part].expect("assigned");
                    for module in 0..count {
                        if module == from || !self.fits(part, module) {
                            continue;
                        }
                        let delta = self.delta(part, module);
                        if best.is_none_or(|(b, _, _)| delta < b) {
                            best = Some((delta, part, module));
                        }
                    }
                }
                let Some((delta, part, module)) = best else {
                    break;
                };
                let from = self.module[part].expect("assigned");
                self.apply(part, module);
                locked.insert(part);
                total += delta;
                history.push((part, from));
                if total < best_total - 1.0e-9 {
                    best_total = total;
                    best_length = history.len();
                }
            }
            for &(part, from) in history[best_length..].iter().rev() {
                self.apply(part, from);
            }
            if best_length == 0 {
                break;
            }
        }
        passes
    }

    /// Puts the parts of `movable` back to `module`.
    fn restore(&mut self, movable: &[usize], module: &[Option<usize>]) {
        for &part in movable {
            if let Some(to) = module[part]
                && self.module[part] != Some(to)
            {
                self.apply(part, to);
            }
        }
    }

    /// What the passes minimise: the tab crossings and the overflow.
    fn objective(&mut self) -> f64 {
        let (_, hops, overflow) = self.cost();
        hops as f64 + OVERFLOW_COST * overflow as f64
    }

    /// The measures of the assignment into `result`.
    fn finish(&mut self, result: &mut ModuleAssignment) {
        let (crossings, hops, overflow) = self.cost();
        result.nets_counted = self.net_counts.len();
        result.crossings = crossings;
        result.tab_crossings = hops;
        result.overflow = overflow;
        result.overfull = (0..self.modules.count())
            .filter(|m| (0..2).any(|side| self.load[*m][side] > self.room[*m][side] + 1.0e-9))
            .count();
        result.module = self.module.clone();
    }

    fn cost(&mut self) -> (usize, usize, usize) {
        let mut crossings = 0usize;
        let mut hops = 0usize;
        for net in 0..self.net_counts.len() {
            let mask = self.mask(net);
            crossings += (mask.count_ones() as usize).saturating_sub(1);
            hops += self.trees.tree(mask).0 as usize;
        }
        let overflow = self
            .tab_load
            .iter()
            .zip(&self.modules.tabs)
            .map(|(load, tab)| (*load - tab.2 as i64).max(0) as usize)
            .sum();
        (crossings, hops, overflow)
    }
}

/// The module of a part where it is: the one most of its pins lie on, by
/// its body's centre where none does.
pub(crate) fn module_of_part(
    modules: &BoardModules,
    component: &core::Component,
    pose: core::Pose,
) -> Option<usize> {
    let mut votes: BTreeMap<usize, usize> = BTreeMap::new();
    for pin in &component.pins {
        if let Some(module) = modules.module_at(component.pin_position(pin, pose)) {
            *votes.entry(module).or_default() += 1;
        }
    }
    votes
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(module, _)| *module)
        .or_else(|| modules.module_at(component.center(pose)))
}

/// The partition with its seeds placed (`assign`), the seeds, and the
/// movable parts it assigns; None where the board has more modules than
/// it handles.
fn seeded_partition<'a>(
    modules: &'a BoardModules,
    problem: &core::Problem,
    footprints: usize,
    poured: &[bool],
) -> Option<(Partition<'a>, ModuleAssignment, Vec<usize>)> {
    let footprints = footprints.min(problem.components.len());
    let count = modules.count();
    let mut result = ModuleAssignment {
        module: vec![None; footprints],
        seeded: vec![false; footprints],
        ..Default::default()
    };
    if !(2..=64).contains(&count) {
        return None;
    }
    let with_pins = |part: usize| !problem.components[part].pins.is_empty();
    // The modules a part may go to: a region's (where it lies on one
    // module, that one), an edge's (the modules that reach it).
    let mut allowed: Vec<Option<BTreeSet<usize>>> = vec![None; footprints];
    let restrict = |allowed: &mut Option<BTreeSet<usize>>, set: BTreeSet<usize>| {
        *allowed = Some(match allowed.take() {
            Some(old) => {
                let both: BTreeSet<usize> = old.intersection(&set).copied().collect();
                if both.is_empty() { old } else { both }
            }
            None => set,
        });
    };
    for (part, region) in &problem.constraints.regions {
        if *part >= footprints {
            continue;
        }
        let centre = [(region[0] + region[2]) / 2.0, (region[1] + region[3]) / 2.0];
        let set: BTreeSet<usize> = match modules.module_at(centre) {
            Some(module) => BTreeSet::from([module]),
            None => (0..count)
                .filter(|m| {
                    let b = modules.boxes[*m];
                    b[0] < region[2] && region[0] < b[2] && b[1] < region[3] && region[1] < b[3]
                })
                .collect(),
        };
        if !set.is_empty() {
            restrict(&mut allowed[*part], set);
        }
    }
    let board = modules.boxes.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        },
    );
    let reaches = |module: usize, edge: core::constraints::Edge| {
        let b = modules.boxes[module];
        match edge {
            core::constraints::Edge::Left => b[0] - board[0] <= EDGE_REACH_MM,
            core::constraints::Edge::Right => board[2] - b[2] <= EDGE_REACH_MM,
            core::constraints::Edge::Top => b[1] - board[1] <= EDGE_REACH_MM,
            core::constraints::Edge::Bottom => board[3] - b[3] <= EDGE_REACH_MM,
        }
    };
    let edges = problem
        .constraints
        .edges
        .iter()
        .map(|(part, edge, _)| (*part, *edge))
        .chain(
            problem
                .constraints
                .overhangs
                .iter()
                .map(|(part, edge, _)| (*part, *edge)),
        );
    for (part, edge) in edges {
        if part >= footprints {
            continue;
        }
        let set: BTreeSet<usize> = (0..count).filter(|m| reaches(*m, edge)).collect();
        if !set.is_empty() {
            restrict(&mut allowed[part], set);
        }
    }
    // The seeds: parts that stay where they are, parts held in one module.
    for (part, allowed) in allowed.iter().enumerate() {
        let component = &problem.components[part];
        if !with_pins(part) {
            continue;
        }
        if component.fixed {
            result.module[part] = module_of_part(modules, component, problem.poses[part]);
        } else if let Some(set) = allowed
            && set.len() == 1
        {
            result.module[part] = set.iter().next().copied();
        }
        result.seeded[part] = result.module[part].is_some();
    }
    // The counted nets: not poured, on two parts or more, not already on
    // every module by the seeds.
    let mut parts_of: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for part in 0..footprints {
        if !with_pins(part) || (problem.components[part].fixed && !result.seeded[part]) {
            continue;
        }
        for pin in &problem.components[part].pins {
            if !poured.get(pin.net).copied().unwrap_or(false) {
                parts_of.entry(pin.net).or_default().insert(part);
            }
        }
    }
    let mut net_parts: Vec<Vec<usize>> = Vec::new();
    for parts in parts_of.into_values() {
        if parts.len() < 2 {
            continue;
        }
        let seeded: BTreeSet<usize> = parts.iter().filter_map(|p| result.module[*p]).collect();
        if seeded.len() == count {
            continue;
        }
        net_parts.push(parts.into_iter().collect());
    }
    let mut part_nets: Vec<Vec<usize>> = vec![Vec::new(); footprints];
    for (net, parts) in net_parts.iter().enumerate() {
        for part in parts {
            part_nets[*part].push(net);
        }
    }
    let area: Vec<[f64; 2]> = (0..footprints)
        .map(|part| side_area(&problem.components[part]))
        .collect();
    let mut room: Vec<[f64; 2]> = modules
        .areas
        .iter()
        .map(|a| [a * MOST_FILL, a * MOST_FILL])
        .collect();
    for (part, area) in area.iter().enumerate() {
        // Every part that stays where it is takes room, pins or not.
        if problem.components[part].fixed
            && let Some(module) =
                result.module.get(part).copied().flatten().or_else(|| {
                    modules.module_at(problem.components[part].center(problem.poses[part]))
                })
        {
            for side in 0..2 {
                room[module][side] -= area[side];
            }
        }
    }
    let mut partition = Partition {
        modules,
        trees: Trees::new(modules),
        module: vec![None; footprints],
        net_counts: vec![vec![0; count]; net_parts.len()],
        net_parts,
        part_nets,
        load: vec![[0.0; 2]; count],
        room,
        area,
        allowed,
        tab_load: vec![0; modules.tabs.len()],
    };
    for part in 0..footprints {
        if result.seeded[part]
            && let Some(module) = result.module[part]
        {
            let keep = partition.area[part];
            if problem.components[part].fixed {
                // Its room is taken already.
                partition.area[part] = [0.0; 2];
            }
            partition.apply(part, module);
            partition.area[part] = keep;
        }
    }
    let movable: Vec<usize> = (0..footprints)
        .filter(|part| {
            !result.seeded[*part]
                && !problem.components[*part].fixed
                && !partition.part_nets[*part].is_empty()
        })
        .collect();
    Some((partition, result, movable))
}

/// Each movable part's module (module comment). `poured`: per net of the
/// problem, whether a pour carries it.
pub(crate) fn assign(
    modules: &BoardModules,
    problem: &core::Problem,
    footprints: usize,
    poured: &[bool],
) -> ModuleAssignment {
    let count = modules.count();
    let Some((mut partition, mut result, movable)) =
        seeded_partition(modules, problem, footprints, poured)
    else {
        return ModuleAssignment::default();
    };
    // The start: the part most attached to the assigned ones goes to the
    // module it is most attached to, where it fits.
    let mut todo: BTreeSet<usize> = movable.iter().copied().collect();
    loop {
        let mut best: Option<(f64, usize, usize)> = None;
        for &part in &todo {
            let mut pull = vec![0.0f64; count];
            for &net in &partition.part_nets[part] {
                let size = partition.net_parts[net].len();
                for (module, pins) in partition.net_counts[net].iter().enumerate() {
                    if *pins > 0 {
                        pull[module] += 1.0 / (size - 1) as f64;
                    }
                }
            }
            for (module, pull) in pull.iter().enumerate() {
                if *pull > 0.0
                    && partition.fits(part, module)
                    && best.is_none_or(|(b, _, _)| *pull > b)
                {
                    best = Some((*pull, part, module));
                }
            }
        }
        let Some((_, part, module)) = best.or_else(|| {
            // Nothing fits where it is pulled: the most pulled part to the
            // allowed module with the most room.
            let part = todo.iter().copied().find(|part| {
                partition.part_nets[*part]
                    .iter()
                    .any(|net| partition.net_counts[*net].iter().any(|pins| *pins > 0))
            })?;
            let module = (0..count)
                .filter(|module| partition.allowed(part, *module))
                .max_by(|a, b| {
                    let free = |m: usize| {
                        (partition.room[m][0] - partition.load[m][0])
                            .min(partition.room[m][1] - partition.load[m][1])
                    };
                    free(*a).total_cmp(&free(*b)).then(b.cmp(a))
                })?;
            Some((0.0, part, module))
        }) else {
            break;
        };
        partition.apply(part, module);
        todo.remove(&part);
    }
    // Fiduccia-Mattheyses passes from the start, then from perturbations
    // of the best assignment so far (a part in five moved to another
    // module where it fits, by a fixed sequence): the best kept.
    let movable: Vec<usize> = movable
        .into_iter()
        .filter(|part| partition.module[*part].is_some())
        .collect();
    result.passes += partition.refine(&movable, count);
    let mut best = (partition.objective(), partition.module.clone());
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        state >> 33
    };
    for _ in 0..RESTARTS {
        partition.restore(&movable, &best.1);
        for &part in &movable {
            let (draw, module) = (next(), next() as usize % count);
            if draw % 5 == 0
                && partition.module[part] != Some(module)
                && partition.fits(part, module)
            {
                partition.apply(part, module);
            }
        }
        result.passes += partition.refine(&movable, count);
        let value = partition.objective();
        if value < best.0 - 1.0e-9 {
            best = (value, partition.module.clone());
        }
    }
    partition.restore(&movable, &best.1);
    partition.finish(&mut result);
    result
}

/// The partition's measures of a given assignment of the movable parts
/// (`module`, per footprint), the seeds as `assign` takes them.
pub(crate) fn evaluate(
    modules: &BoardModules,
    problem: &core::Problem,
    footprints: usize,
    poured: &[bool],
    module: &[Option<usize>],
) -> ModuleAssignment {
    let Some((mut partition, mut result, movable)) =
        seeded_partition(modules, problem, footprints, poured)
    else {
        return ModuleAssignment::default();
    };
    for part in movable {
        if let Some(Some(to)) = module.get(part) {
            partition.apply(part, *to);
        }
    }
    partition.finish(&mut result);
    result
}

/// The modules as `board-placer.json` says them (`keep_in_modules` on and
/// a board of two modules or more).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadModulesReport {
    /// How many modules, and what tells them apart: `perforations` (rows
    /// of holes and slots in one outline) or `outline loops`.
    pub modules: usize,
    pub kind: String,
    /// The movable parts held in their module's region, and those the
    /// netlist gives none (only poured nets, or nets to no part with a
    /// module): placed where the placement puts them.
    pub held: usize,
    pub unassigned: usize,
    /// The assignment's module crossings (per net not poured, the modules
    /// it spans less one), its tab crossings and the tracks over the
    /// tabs' capacities.
    pub crossings: usize,
    pub tab_crossings: usize,
    pub overflow: usize,
}

/// Holds each movable part in its module (`detect`, `assign`): its
/// module's box a region the tool gives (`automatic_regions`, as
/// `keep_on_pieces` gives one: the race over skeletons still leaves the
/// passives out). A part a region, an edge or an overhang holds already
/// (protoesp's CN36, held at the bottom edge, found no place on its
/// module's stretch of it), or one too large for its module's box, stays
/// as it is; no part gets a start (a start names a passive, and the race
/// then has no skeleton). None where the board has no modules.
pub(crate) fn hold_in_modules(
    pcb: &Expr,
    problem: &mut core::Problem,
    footprints: usize,
    poured: &[bool],
) -> Result<Option<KiCadModulesReport>, String> {
    let Some(modules) = detect(pcb)? else {
        return Ok(None);
    };
    let footprints = footprints.min(problem.components.len());
    let assignment = assign(&modules, problem, footprints, poured);
    if assignment.module.is_empty() {
        return Ok(None);
    }
    let constraints = &problem.constraints;
    let held: BTreeSet<usize> = constraints
        .regions
        .iter()
        .map(|(part, _)| *part)
        .chain(constraints.edges.iter().map(|(part, _, _)| *part))
        .chain(constraints.overhangs.iter().map(|(part, _, _)| *part))
        .collect();
    let mut report = KiCadModulesReport {
        modules: modules.count(),
        kind: modules.kind.to_string(),
        crossings: assignment.crossings,
        tab_crossings: assignment.tab_crossings,
        overflow: assignment.overflow,
        ..Default::default()
    };
    for part in 0..footprints {
        let component = &problem.components[part];
        if component.fixed || component.pins.is_empty() {
            continue;
        }
        let Some(module) = assignment.module[part] else {
            report.unassigned += 1;
            continue;
        };
        if held.contains(&part) {
            continue;
        }
        let region = modules.boxes[module];
        let size = [region[2] - region[0], region[3] - region[1]];
        let body = component.body_size;
        let fits = (body[0] <= size[0] && body[1] <= size[1])
            || (body[1] <= size[0] && body[0] <= size[1]);
        if !fits {
            report.unassigned += 1;
            continue;
        }
        problem.constraints.regions.push((part, region));
        problem.constraints.automatic_regions.insert(part);
        report.held += 1;
    }
    Ok(Some(report))
}

/// The crossings of an assignment (`ModuleAssignment::crossings`, per
/// net the modules it spans less one) over the nets that are not poured:
/// the census's measure (docs/placer.md, "Modules of one board").
pub(crate) fn crossings(
    problem: &core::Problem,
    module: &[Option<usize>],
    poured: &[bool],
) -> usize {
    let mut on: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (part, module) in module.iter().enumerate() {
        let Some(module) = module else { continue };
        for pin in &problem.components[part].pins {
            if !poured.get(pin.net).copied().unwrap_or(false) {
                on.entry(pin.net).or_default().insert(*module);
            }
        }
    }
    on.values().map(|set| set.len().saturating_sub(1)).sum()
}

/// `pcb-maker modules-kicad-board`: a board's modules and the assignment
/// of its parts, as JSON; with the designer's board (`designer`), each
/// movable part's module there against the assignment's (research; the
/// census of docs/placer.md "Modules of one board").
pub fn describe_modules(
    source_directory: &Path,
    board_id: &str,
    config: &crate::board_placer::KiCadBoardPlacerConfig,
    designer: Option<&Path>,
) -> Result<serde_json::Value, String> {
    let prepared = crate::board_placer::prepare_board(source_directory, board_id, config, None)?;
    let pcb = &prepared.pcb;
    let lowered = &prepared.lowered;
    let problem = &lowered.problem;
    let footprints = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .count()
        .min(problem.components.len());
    let Some(modules) = detect(pcb)? else {
        return Ok(serde_json::json!({ "modules": 0 }));
    };
    let started = std::time::Instant::now();
    let assignment = assign(&modules, problem, footprints, &lowered.poured);
    let seconds = started.elapsed().as_secs_f64();
    let passive = crate::board_placer::passive_mask(pcb, problem.components.len());
    let movable: Vec<usize> = (0..footprints)
        .filter(|part| {
            let component = &problem.components[*part];
            !component.fixed && !component.pins.is_empty()
        })
        .collect();
    let round = |b: [f64; 4]| b.map(|v| (v * 100.0).round() / 100.0);
    let mut report = serde_json::json!({
        "kind": modules.kind,
        "modules": modules.count(),
        "boxes": modules.boxes.iter().map(|b| round(*b)).collect::<Vec<_>>(),
        "areas": modules.areas.iter().map(|a| a.round()).collect::<Vec<_>>(),
        "tabs": modules.tabs,
        "movable": movable.len(),
        "movable_active": movable.iter().filter(|p| !passive[**p]).count(),
        "seeded_movable": movable.iter().filter(|p| assignment.seeded[**p]).count(),
        "unassigned": movable.iter().filter(|p| assignment.module[**p].is_none()).count(),
        "nets_counted": assignment.nets_counted,
        "crossings": assignment.crossings,
        "crossings_all": crossings(problem, &assignment.module, &lowered.poured),
        "tab_crossings": assignment.tab_crossings,
        "overflow": assignment.overflow,
        "overfull": assignment.overfull,
        "passes": assignment.passes,
        "seconds": (seconds * 1000.0).round() / 1000.0,
        "fixed_per_module": (0..modules.count()).map(|m| (0..footprints).filter(|p| problem.components[*p].fixed && assignment.module[*p] == Some(m)).count()).collect::<Vec<_>>(),
        "assigned_per_module": (0..modules.count()).map(|m| movable.iter().filter(|p| assignment.module[**p] == Some(m)).count()).collect::<Vec<_>>(),
        "unassigned_parts": movable.iter().filter(|p| assignment.module[**p].is_none()).map(|p| lowered.references[*p].clone()).collect::<Vec<_>>(),
        "assignment": (0..footprints).filter(|p| assignment.module[*p].is_some()).map(|p| (lowered.references[p].clone(), assignment.module[p])).collect::<Vec<_>>(),
    });
    let Some(designer) = designer else {
        return Ok(report);
    };
    // The designer's places, by reference (the n-th footprint of a
    // repeated reference to the n-th).
    let text = std::fs::read_to_string(designer)
        .map_err(|error| format!("failed to read {}: {error}", designer.display()))?;
    let designer_pcb = crate::canonical_order::parse_board(&text)?;
    let mut places: BTreeMap<String, Vec<[f64; 3]>> = BTreeMap::new();
    for footprint in designer_pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        places
            .entry(crate::kicad_nets::footprint_reference(footprint).unwrap_or_default())
            .or_default()
            .push(crate::kicad_sexpr::form_at(footprint)?);
    }
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    let mut truth: Vec<Option<usize>> = vec![None; footprints];
    for (part, slot) in truth.iter_mut().enumerate() {
        let reference = lowered.references[part].as_str();
        let nth = seen.entry(reference).or_default();
        if let Some(at) = places.get(reference).and_then(|ats| ats.get(*nth)) {
            *slot = module_of_part(
                &modules,
                &problem.components[part],
                core::Pose {
                    position: [at[0], at[1]],
                    angle: at[2],
                },
            );
        }
        *nth += 1;
    }
    // A part whose counted nets reach the designer's module and ours is a
    // bridge (a net tie, a jumper on a tab): either module serves it.
    let mut right = 0;
    let mut wrong = 0;
    let mut unassigned = 0;
    let mut right_active = 0;
    let mut wrong_active = 0;
    let mut unassigned_active = 0;
    let mut wrong_parts = Vec::new();
    for &part in &movable {
        let active = !passive[part];
        match (assignment.module[part], truth[part]) {
            (None, _) => {
                unassigned += 1;
                unassigned_active += active as usize;
            }
            (Some(ours), Some(theirs)) if ours == theirs => {
                right += 1;
                right_active += active as usize;
            }
            (Some(ours), theirs) => {
                wrong += 1;
                wrong_active += active as usize;
                wrong_parts.push(serde_json::json!([
                    lowered.references[part],
                    theirs,
                    ours,
                    if active { "active" } else { "passive" }
                ]));
            }
        }
    }
    let mut designer_module = truth.clone();
    for (part, module) in designer_module.iter_mut().enumerate() {
        if problem.components[part].fixed {
            *module = assignment.module[part];
        }
    }
    let measured = evaluate(&modules, problem, footprints, &lowered.poured, &truth);
    let object = report.as_object_mut().expect("an object");
    object.insert(
        "designer".into(),
        serde_json::json!({
            "crossings_all": crossings(problem, &designer_module, &lowered.poured),
            "crossings": measured.crossings,
            "tab_crossings": measured.tab_crossings,
            "overflow": measured.overflow,
            "overfull": measured.overfull,
            "right": right,
            "wrong": wrong,
            "unassigned": unassigned,
            "right_active": right_active,
            "wrong_active": wrong_active,
            "unassigned_active": unassigned_active,
            "wrong_parts": wrong_parts,
            "assignment": (0..footprints).filter(|p| designer_module[*p].is_some()).map(|p| (lowered.references[p].clone(), designer_module[p])).collect::<Vec<_>>(),
            "movable_per_module": (0..modules.count()).map(|m| movable.iter().filter(|p| truth[**p] == Some(m)).count()).collect::<Vec<_>>(),
        }),
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board_placer::{KiCadBoardPlacerConfig, lower_placement};

    /// Two halves of 19.5 x 20 mm joined by a tab 4 mm wide, with three
    /// 0.5 mm holes across it 1.5 mm apart (two gaps of 1 mm, one track
    /// each, as protoesp's): J1 on the left, J2 on the right; U1 joins J1
    /// by two nets, U2 joins J2 by two, X joins U1 and U2. `holes`: the
    /// perforation's footprint, or none.
    fn halves(holes: bool) -> Expr {
        let bite = if holes {
            r#"(footprint "mouse" (layer "F.Cu") (at 20 10)
                (pad "" np_thru_hole circle (at 0 -1.5) (size 0.5 0.5) (drill 0.5) (layers "*.Cu" "*.Mask"))
                (pad "" np_thru_hole circle (at 0 0) (size 0.5 0.5) (drill 0.5) (layers "*.Cu" "*.Mask"))
                (pad "" np_thru_hole circle (at 0 1.5) (size 0.5 0.5) (drill 0.5) (layers "*.Cu" "*.Mask")))"#
        } else {
            ""
        };
        let part = |reference: &str, x: f64, nets: [(usize, &str); 3]| {
            format!(
                r#"(footprint "SOIC" (layer "F.Cu") (at {x} 10) (property "Reference" "{reference}")
                (fp_rect (start -2 -2) (end 2 2) (layer "F.CrtYd"))
                (pad "1" smd rect (at -1 0) (size 0.6 0.7) (layers "F.Cu") (net {} "{}"))
                (pad "2" smd rect (at 0 0) (size 0.6 0.7) (layers "F.Cu") (net {} "{}"))
                (pad "3" smd rect (at 1 0) (size 0.6 0.7) (layers "F.Cu") (net {} "{}")))"#,
                nets[0].0, nets[0].1, nets[1].0, nets[1].1, nets[2].0, nets[2].1
            )
        };
        let text = format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (net 0 "") (net 1 "A1") (net 2 "A2") (net 3 "B1") (net 4 "B2") (net 5 "X") (net 6 "GND")
            (gr_poly (pts (xy 0 0) (xy 19.5 0) (xy 19.5 8) (xy 20.5 8) (xy 20.5 0) (xy 40 0)
                (xy 40 20) (xy 20.5 20) (xy 20.5 12) (xy 19.5 12) (xy 19.5 20) (xy 0 20))
                (layer "Edge.Cuts"))
            {bite}
            {}
            {}
            {}
            {})"#,
            part("J1", 5.0, [(1, "A1"), (2, "A2"), (6, "GND")]),
            part("J2", 35.0, [(3, "B1"), (4, "B2"), (6, "GND")]),
            // Both start stacked on the right half.
            part("U1", 30.0, [(1, "A1"), (2, "A2"), (5, "X")]),
            part("U2", 30.0, [(3, "B1"), (4, "B2"), (5, "X")]),
        );
        crate::kicad_sexpr::parse(&text).unwrap()
    }

    fn lowered(pcb: &Expr) -> crate::board_placer::LoweredPlacement {
        let config = KiCadBoardPlacerConfig {
            fixed: vec!["J1".into(), "J2".into()],
            ..KiCadBoardPlacerConfig::default()
        };
        lower_placement(pcb, &config).unwrap()
    }

    #[test]
    fn a_perforated_tab_between_two_halves_makes_two_modules() {
        let pcb = halves(true);
        // A mouse bite's pitch does not chain holes 1.5 mm apart: the
        // panel's pieces see one board.
        let loops = crate::outline::board_loops(&pcb).unwrap();
        assert!(
            Pieces::of_loops(&loops.piece_loops, &loops.cutouts, &Cuts::of_board(&pcb)).is_none()
        );
        let modules = detect(&pcb).unwrap().expect("two modules");
        assert_eq!(modules.count(), 2);
        assert_eq!(modules.kind, "perforations");
        // One tab, two gaps of 1 mm: two tracks.
        assert_eq!(modules.tabs, vec![(0, 1, 2)]);
        let left = modules.module_at([5.0, 10.0]).unwrap();
        let right = modules.module_at([35.0, 10.0]).unwrap();
        assert_ne!(left, right);
        // Without the holes the tab is board: one module.
        assert!(detect(&halves(false)).unwrap().is_none());
    }

    #[test]
    fn the_partition_puts_each_part_with_the_fixed_part_it_shares_nets_with() {
        let pcb = halves(true);
        let modules = detect(&pcb).unwrap().unwrap();
        let mut lowered = lowered(&pcb);
        let index = |name: &str| {
            lowered
                .references
                .iter()
                .position(|reference| reference == name)
                .unwrap()
        };
        let (u1, u2) = (index("U1"), index("U2"));
        let left = modules.module_at([5.0, 10.0]);
        let right = modules.module_at([35.0, 10.0]);
        let assignment = assign(&modules, &lowered.problem, 5, &lowered.poured);
        assert_eq!(assignment.module[u1], left);
        assert_eq!(assignment.module[u2], right);
        // X alone crosses the tab.
        assert_eq!(assignment.crossings, 1);
        assert_eq!(assignment.tab_crossings, 1);
        assert_eq!(assignment.overflow, 0);
        // Held: each in its module's box, as a region the tool gave.
        let report = hold_in_modules(&pcb, &mut lowered.problem, 5, &lowered.poured)
            .unwrap()
            .unwrap();
        assert_eq!(report.held, 2);
        let constraints = &lowered.problem.constraints;
        for (part, module) in [(u1, left), (u2, right)] {
            let region = constraints
                .regions
                .iter()
                .find(|(held, _)| *held == part)
                .map(|(_, region)| *region)
                .unwrap();
            assert_eq!(region, modules.boxes[module.unwrap()]);
            assert!(constraints.automatic_regions.contains(&part));
        }
        assert!(constraints.starts.is_empty());
    }
}

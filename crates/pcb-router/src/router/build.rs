// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Lowering the board into the router's maps.

use super::*;

impl Router {
    /// `board` with its classes split by the rules beyond the net classes
    /// (`Config::net_rules`, `crate::rules::ClassRules::split`) and a neck
    /// variant of every class wider than its neck (for routing next to
    /// fine-pitch pads), per class its neck class, and the split's rules
    /// (`None` without rules: the board's classes as they are).
    pub(super) fn with_neck_classes(
        board: &Board,
        config: &Config,
    ) -> (Board, Vec<Option<usize>>, Option<Arc<ClassRules>>) {
        let (mut board, mut class_rules) = match config
            .net_rules
            .as_deref()
            .filter(|rules| !rules.is_empty())
        {
            Some(rules) => {
                let (board, class_rules) = ClassRules::split(board, rules);
                (board, Some(class_rules))
            }
            None => (board.clone(), None),
        };
        let mut neck_of = vec![None; board.classes.len()];
        if config.neck_reach > 0.0 {
            for (class, neck_class) in neck_of.iter_mut().enumerate() {
                let rules = board.classes[class];
                let neck_width = class_rules
                    .as_ref()
                    .map_or(board.neck_width, |split| split.neck_width(&board, class));
                if rules.trace_width > neck_width + 1.0e-9 {
                    let neck = RuleClass {
                        trace_width: neck_width,
                        ..rules
                    };
                    let index = match &mut class_rules {
                        None => board
                            .classes
                            .iter()
                            .position(|existing| *existing == neck)
                            .unwrap_or_else(|| {
                                board.classes.push(neck);
                                board.classes.len() - 1
                            }),
                        Some(split) => {
                            let (group, floor) =
                                (split.class_group[class], split.class_floor[class]);
                            (0..board.classes.len())
                                .find(|&existing| {
                                    board.classes[existing] == neck
                                        && split.class_group[existing] == group
                                        && split.class_floor[existing] == floor
                                })
                                .unwrap_or_else(|| {
                                    board.classes.push(neck);
                                    split.class_group.push(group);
                                    split.class_floor.push(floor);
                                    board.classes.len() - 1
                                })
                        }
                    };
                    *neck_class = Some(index);
                }
            }
        }
        neck_of.resize(board.classes.len(), None);
        (board, neck_of, class_rules.map(Arc::new))
    }

    /// `new`, or the error of a board the router's maps cannot hold
    /// (`grid::check_capacity`: more than `MAX_NETS` nets or `MAX_LAYERS`
    /// copper layers).
    pub fn try_new(board: &Board, config: &Config) -> Result<Self, String> {
        crate::grid::check_capacity(board)?;
        Ok(Self::new(board, config))
    }

    /// Builds the router for `board`. Panics on a board the maps cannot
    /// hold (`try_new` returns that as an error).
    pub fn new(board: &Board, config: &Config) -> Self {
        let started = std::time::Instant::now();
        let router = Self::build(board, config);
        BUILD_WORK_NANOS.fetch_add(
            (router.build_work * 1.0e9) as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        BUILD_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        BUILD_NANOS.fetch_add(
            started.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        router
    }

    pub(super) fn build(board: &Board, config: &Config) -> Self {
        let built = std::time::Instant::now();
        let (board, neck_of, class_rules) = Self::with_neck_classes(board, config);
        let board = &board;
        let grid = Grid::choose(board, &config.pitches);
        if config.verbose {
            eprintln!(
                "lattice: pitch {} mm, origin {:?}, {} x {} nodes",
                grid.pitch, grid.origin, grid.nx, grid.ny
            );
        }
        let cells = grid.cells();
        let layers = board.layer_count;
        // The search records (`Hot`) name nets in 16 bits as the static
        // maps do; until 2026-10-21 in 15, a bit kept for a hard guard
        // flag that no longer exists.
        if let Err(message) = crate::grid::check_capacity(board) {
            panic!("{message} (Router::try_new returns it as an error)");
        }
        let searched = searched_classes(board, &neck_of);
        let (statics, statics_shared, statics_key) = if config.share_static_maps {
            shared_statics(board, &grid, &searched, class_rules.as_deref())
        } else {
            (
                build_statics(board, &grid, &searched, class_rules.as_deref()),
                false,
                None,
            )
        };
        let statics_built = built.elapsed().as_secs_f64();
        let maps = board.classes.len() * (layers + 1) + board.classes.len() * layers * 2;
        let step = grid.pitch * std::f64::consts::SQRT_2;
        let variants = class_rules
            .as_ref()
            .map_or(1, |split| split.resolved.variants);
        let stamps = (0..board.classes.len())
            .map(|own_class| {
                let own = &board.classes[own_class];
                (0..variants)
                    .flat_map(|variant| {
                        (0..board.classes.len()).map(move |other_class| (variant, other_class))
                    })
                    .map(|(variant, other_class)| {
                        let other = &board.classes[other_class];
                        let clearance = own.clearance.max(other.clearance);
                        // The rules beyond the classes, per pair of item
                        // kinds (a via meets the other item on every layer).
                        let rule = |own_kind: usize, other_kind: usize| {
                            class_rules.as_ref().map_or(clearance, |split| {
                                let (a, b) =
                                    (split.class_group[own_class], split.class_group[other_class]);
                                let extra = if own_kind == crate::rules::VIA
                                    && other_kind == crate::rules::VIA
                                {
                                    split.resolved.extra_anywhere(a, own_kind, b, other_kind)
                                } else {
                                    split.resolved.extra(a, own_kind, b, other_kind, variant)
                                };
                                clearance.max(extra)
                            })
                        };
                        use crate::rules::{TRACK, VIA};
                        let trace_clearance = rule(TRACK, TRACK);
                        // Exact: a node at the radius is legal, and the
                        // steps between nodes are covered apart (see
                        // `diagonal_extra`; the search checks the cells a
                        // diagonal step cuts).
                        let with_margin = |radius: f64| radius;
                        let snap = grid.pitch * 0.75 + step * step / (4.0 * grid.pitch);
                        let trace_radius =
                            own.trace_width / 2.0 + other.trace_width / 2.0 + trace_clearance;
                        // Copper keeps the copper clearance from a via's
                        // ring and the hole clearance from its drill;
                        // with a thin ring the hole rule is the larger
                        // (the laptop motherboard: 154 KiCad findings).
                        let hole = board.hole_clearance;
                        let via_radius =
                            (own.trace_width / 2.0 + other.via_diameter / 2.0 + rule(TRACK, VIA))
                                .max(own.trace_width / 2.0 + other.via_drill / 2.0 + hole);
                        let own_via_radius =
                            (own.via_diameter / 2.0 + other.trace_width / 2.0 + rule(VIA, TRACK))
                                .max(own.via_drill / 2.0 + other.trace_width / 2.0 + hole);
                        let via_clearance = rule(VIA, VIA);
                        Stamps {
                            diagonal_to_trace: diagonal_extra(trace_radius, grid.pitch),
                            diagonal_to_via: diagonal_extra(via_radius, grid.pitch),
                            diagonal_block: diagonal_block(trace_radius, grid.pitch),
                            // This net's via against the querying class's
                            // diagonal steps: its ring and their trace (Sisu:
                            // 0.3 power tracks passed 0.4 vias at 0.145).
                            diagonal_block_via: diagonal_block(own_via_radius, grid.pitch),
                            stub_diagonal_block: diagonal_block(trace_radius + snap, grid.pitch),
                            stub_to_trace: disc(
                                with_margin(
                                    own.trace_width / 2.0
                                        + other.trace_width / 2.0
                                        + trace_clearance,
                                ) + snap,
                                grid.pitch,
                            ),
                            stub_to_via: disc(with_margin(via_radius) + snap, grid.pitch),
                            trace_to_trace: disc(
                                with_margin(
                                    own.trace_width / 2.0
                                        + other.trace_width / 2.0
                                        + trace_clearance,
                                ),
                                grid.pitch,
                            ),
                            trace_to_via: disc(with_margin(via_radius), grid.pitch),
                            via_to_trace: disc(with_margin(own_via_radius), grid.pitch),
                            via_to_via: disc(
                                (own.via_diameter / 2.0 + other.via_diameter / 2.0 + via_clearance)
                                    .max(
                                        own.via_drill / 2.0
                                            + other.via_drill / 2.0
                                            + board.hole_to_hole,
                                    )
                                    .max(own.via_drill / 2.0 + other.via_diameter / 2.0 + hole)
                                    .max(own.via_diameter / 2.0 + other.via_drill / 2.0 + hole),
                                grid.pitch,
                            ),
                            fresh: Fresh::default(),
                        }
                    })
                    .map(|mut stamps| {
                        stamps.fresh = Fresh::of(&stamps);
                        stamps
                    })
                    .collect()
            })
            .collect();
        let direction_cost = (0..layers)
            .map(|layer| {
                let against = config.against_direction as f32;
                let (horizontal, vertical) = if layer % 2 == 0 {
                    (1.0, against)
                } else {
                    (against, 1.0)
                };
                let diagonal = (horizontal + vertical) / 2.0 * std::f32::consts::SQRT_2;
                let mut costs = [0.0; 8];
                for (direction, (dx, dy)) in DIRECTIONS.iter().enumerate() {
                    costs[direction] = match (dx != &0, dy != &0) {
                        (true, true) => diagonal,
                        (true, false) => horizontal,
                        _ => vertical,
                    } * grid.pitch as f32;
                }
                costs
            })
            .collect();
        let states = cells * layers;
        let tiles_x = grid.nx.div_ceil(TILE);
        let tiles_y = grid.ny.div_ceil(TILE);
        let mut tile_routable = vec![vec![0u32; tiles_x * tiles_y]; board.classes.len() * layers];
        for class in 0..board.classes.len() {
            for layer in 0..layers {
                let counts = &mut tile_routable[class * layers + layer];
                for (cell, value) in statics[class].trace[layer].iter().enumerate() {
                    if *value != crate::grid::BLOCKED {
                        counts[(cell / grid.nx / TILE) * tiles_x + (cell % grid.nx) / TILE] += 1;
                    }
                }
            }
        }
        let mut router = Self {
            board: board.clone(),
            config: config.clone(),
            statics,
            statics_shared,
            statics_key,
            occupancy: vec![vec![0; cells]; maps],
            stamp_mark: vec![0; cells.div_ceil(64) * maps],
            history: vec![vec![0.0; cells]; layers + 1],
            stamps,
            nets: Vec::new(),
            direction_cost,
            cleanup: false,
            tiles_x,
            tiles_y,
            tile_claimed: vec![vec![0; tiles_x * tiles_y]; maps],
            tile_routable,
            global: None,
            tile_history: vec![vec![0.0; tiles_x * tiles_y]; layers + 1],
            guard: vec![Vec::new(); layers],
            via_marks: Vec::new(),
            via_cover: Vec::new(),
            covered: vec![Vec::new(); layers],
            hot: Vec::new(),
            tile_covered: vec![vec![0.0; tiles_x * tiles_y]; layers],
            layer_cut: vec![1.0; layers],
            neck_of,
            class_rules,
            searched,
            exclusive: (0..layers)
                .map(|layer| {
                    config.exclusive_planes
                        && board
                            .planes
                            .iter()
                            .any(|plane| plane.layer == layer && plane.exclusive)
                })
                .collect(),
            layer_bias: Vec::new(),
            scratch: Scratch::new(states, cells),
            profile: Profile::default(),
            in_via_reduction: false,
            island_stubs: 0,
            manual_spokes_work: 0.0,
            build_work: 0.0,
            first_iteration_expansions: 0,
            frame_hook: None,
            search_seconds: 0.0,
            negotiation_seconds: 0.0,
            present_reached: 0.0,
            hopeless: false,
            stamp_seconds: 0.0,
            iterations: 0,
            last_conflicted: Vec::new(),
            probe_shapes: Arc::default(),
            grid,
        };
        if let Some(seed) = config.seed.or_else(experiment_seed) {
            let mut state = seed;
            let amplitude = config.history_increment as f32;
            for layer in router
                .history
                .iter_mut()
                .chain(router.tile_history.iter_mut())
            {
                for value in layer.iter_mut() {
                    *value = amplitude * unit_noise(&mut state) as f32;
                }
            }
        }
        let nets_started = built.elapsed().as_secs_f64();
        router.nets = (0..board.nets.len())
            .map(|net| router.prepare_net(net as NetId))
            .collect();
        let nets_built = built.elapsed().as_secs_f64();
        router.guard = router.thermal_guards();
        router.mark_covered();
        router.mark_via_cells();
        router.build_hot();
        let counts = router.build_counts();
        router.build_work = counts.work_seconds();
        if std::env::var_os("PCB_ROUTER_BUILD_TRACE").is_some() {
            // For calibrating `BUILD_*_NANOS`: the counts beside the wall
            // clock, and where it went.
            let total = built.elapsed().as_secs_f64();
            let statics_tests: u64 = router.statics.iter().map(|maps| maps.tests).sum();
            let statics_edges: u64 = router.statics.iter().map(|maps| maps.vertex_tests).sum();
            eprintln!(
                "build trace: map_nodes {} tests {} edges {} statics_tests {statics_tests} statics_edges {statics_edges} tight {} hot_nodes {} work {:.0} wall {:.0} statics {:.0} stamps {:.0} nets {:.0} rest {:.0} cells {cells} layers {layers} classes {} searched {} obstacles {} netcount {} planes {} shared {}",
                counts.map_nodes + counts.shared_map_nodes,
                counts.tests,
                counts.edges,
                counts.step_edges,
                cells * layers * router.searched.iter().filter(|searched| **searched).count(),
                counts.work_nanos(),
                total * 1.0e9,
                statics_built * 1.0e9,
                (nets_started - statics_built) * 1.0e9,
                (nets_built - nets_started) * 1.0e9,
                (total - nets_built) * 1.0e9,
                board.classes.len(),
                router.searched.iter().filter(|searched| **searched).count(),
                board.obstacles.len(),
                board.nets.len(),
                board.planes.len(),
                router.statics_shared,
            );
        }
        if config.verbose {
            eprintln!(
                "{} (built in {:.1}s)",
                router.memory_report(),
                built.elapsed().as_secs_f64()
            );
        }
        router
    }

    /// What building this router took, counted (`BuildCounts`).
    pub(super) fn build_counts(&self) -> BuildCounts {
        let layers = self.board.layer_count as u64;
        let prepared = |index: usize| -> u64 {
            self.nets
                .iter()
                .map(|state| state.prepare_counts[index])
                .sum()
        };
        // Static maps another router built count nothing here (`shared_statics`).
        let statics = |count: fn(&StaticMaps) -> u64| -> u64 {
            if self.statics_shared {
                0
            } else {
                self.statics.iter().map(count).sum()
            }
        };
        let map_nodes = self.grid.cells() as u64 * layers * self.statics.len() as u64;
        BuildCounts {
            map_nodes: if self.statics_shared { 0 } else { map_nodes },
            shared_map_nodes: if self.statics_shared { map_nodes } else { 0 },
            tests: statics(|maps| maps.tests) + prepared(0),
            edges: statics(|maps| maps.vertex_tests) + prepared(1),
            step_edges: statics(|maps| maps.tight_tests),
        }
    }

    /// What the router's lattice maps hold, in megabytes per part, and in
    /// bytes per node (`cells`, one layer) for the whole: where a board's
    /// memory goes. The per-thread search scratches of the parallel
    /// batches come on top, one per worker thread that routed a batch.
    pub fn memory_report(&self) -> String {
        fn bytes<T>(maps: &[Vec<T>]) -> usize {
            maps.iter()
                .map(|map| map.capacity() * std::mem::size_of::<T>())
                .sum()
        }
        let cells = self.grid.cells().max(1);
        let statics: usize = self.statics.iter().map(StaticMaps::bytes).sum();
        let occupancy = bytes(&self.occupancy);
        let stamp_mark = self.stamp_mark.capacity() * 8;
        let hot = bytes(&self.hot);
        let pour = bytes(&self.history)
            + bytes(&self.guard)
            + bytes(&self.covered)
            + self.via_marks.capacity()
            + 4 * self.via_cover.capacity();
        let planes: usize = self
            .nets
            .iter()
            .map(|state| bytes(&state.plane) + bytes(&state.plane_target))
            .sum();
        let scratch = self.scratch.bytes();
        let tiles = bytes(&self.tile_claimed)
            + bytes(&self.tile_routable)
            + bytes(&self.tile_history)
            + bytes(&self.tile_covered);
        let total = statics + occupancy + stamp_mark + hot + pour + planes + scratch + tiles;
        let mb = |value: usize| value as f64 / (1024.0 * 1024.0);
        format!(
            "memory: {:.0} MB in maps ({:.0} bytes a node; {} x {} nodes, {} layers, {} classes, {} searched, {} track geometries): statics {:.0}, occupancy {:.0}, stamp marks {:.0}, hot {:.0}, history/guard/cover {:.0}, pour masks {:.0}, scratch {:.0} (+{:.0} per worker thread), tiles {:.0}",
            mb(total),
            total as f64 / cells as f64,
            self.grid.nx,
            self.grid.ny,
            self.board.layer_count,
            self.board.classes.len(),
            self.searched.iter().filter(|searched| **searched).count(),
            {
                let mut geometries: Vec<(u64, u64)> = (0..self.board.classes.len())
                    .filter(|class| self.searched[*class])
                    .map(|class| {
                        (
                            self.board.classes[class].trace_width.to_bits(),
                            self.board.classes[class].clearance.to_bits(),
                        )
                    })
                    .collect();
                geometries.sort();
                geometries.dedup();
                geometries.len()
            },
            mb(statics),
            mb(occupancy),
            mb(stamp_mark),
            mb(hot),
            mb(pour),
            mb(planes),
            mb(scratch),
            mb(self.scratch.bytes()),
            mb(tiles),
        )
    }

    /// Packs the static map, the guard, the pour cover and the history
    /// per class and layer (see `hot`); after any of them changed.
    pub(super) fn build_hot(&mut self) {
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        self.hot = (0..self.statics.len() * layers)
            .map(|index| {
                let (class, layer) = (index / layers, index % layers);
                if !self.searched[class] {
                    return Vec::new();
                }
                let trace = &self.statics[class].trace[layer];
                let guard = &self.guard[layer];
                let covered = &self.covered[layer];
                let history = &self.history[layer];
                (0..cells)
                    .map(|cell| Hot {
                        trace: trace[cell],
                        guard: guard.get(cell).map_or(0, |&value| value as u16),
                        covered: covered.get(cell).map_or(0, |&value| value as u16),
                        history: history[cell],
                    })
                    .collect()
            })
            .collect();
    }

    /// The neck width of `class`: the board's, or the floor of the class's
    /// nets where that is wider (`crate::rules::NetRules::width_floor`).
    pub(super) fn neck_width_of(&self, class: usize) -> f64 {
        self.class_rules
            .as_ref()
            .map_or(self.board.neck_width, |split| {
                split.neck_width(&self.board, class)
            })
    }

    /// The clearance copper of `class` (an item of `kind`, `crate::rules`)
    /// keeps from copper `obstacle` on `layer`: the classes' and the
    /// rules beyond them.
    pub(super) fn obstacle_clearance(
        &self,
        class: usize,
        kind: usize,
        obstacle: &crate::board::Obstacle,
        layer: usize,
    ) -> f64 {
        let clearance = self
            .board
            .copper_clearance(&self.board.classes[class], obstacle);
        match &self.class_rules {
            None => clearance,
            Some(split) => clearance.max(split.obstacle_extra(class, kind, obstacle, layer)),
        }
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub(super) fn state(&self, node: Node) -> usize {
        node.layer as usize * self.grid.cells() + node.cell as usize
    }

    #[inline(always)]
    pub(super) fn map_index(&self, class: usize, layer: usize) -> usize {
        class * (self.board.layer_count + 1) + layer
    }

    /// The map counting the nets whose copper a diagonal step of `class`
    /// on `layer` starting at a cell would pass too closely (`orientation`
    /// 0: the step towards +x +y, 1: towards +x -y; a step towards -x is
    /// the same step from its other end). A diagonal step passes the nodes
    /// on its bisector closer than its ends do; those nodes stamp here.
    #[inline(always)]
    pub(super) fn diagonal_map(&self, class: usize, layer: usize, orientation: usize) -> usize {
        let layers = self.board.layer_count;
        self.board.classes.len() * (layers + 1) + (class * layers + layer) * 2 + orientation
    }

    /// Sets `via_marks` from `covered` and `guard`.
    pub(super) fn mark_via_cells(&mut self) {
        let mut marks = vec![0u8; self.grid.cells()];
        for (bit, maps) in [(1u8, &self.covered), (2u8, &self.guard)] {
            for map in maps.iter().filter(|map| !map.is_empty()) {
                for (mark, value) in marks.iter_mut().zip(map) {
                    if *value != 0 {
                        *mark |= bit;
                    }
                }
            }
        }
        self.via_marks = marks;
        let mut cover = vec![0u32; self.grid.cells()];
        for (layer, map) in self.covered.iter().enumerate() {
            if map.is_empty() {
                continue;
            }
            for (summary, &owner) in cover.iter_mut().zip(map) {
                if owner == 0 || *summary == VIA_COVER_MIXED {
                    continue;
                }
                if layer >= 15 || owner > 0xffff || (*summary != 0 && *summary & 0xffff != owner) {
                    *summary = VIA_COVER_MIXED;
                } else {
                    *summary = owner | (*summary & !0xffff) | 1 << (16 + layer);
                }
            }
        }
        self.via_cover = cover;
    }

    /// Marks the nodes and tiles under pours, per layer.
    pub(super) fn mark_covered(&mut self) {
        let tiles = self.tiles_x * self.tiles_y;
        for plane in &self.board.planes {
            let layer = plane.layer;
            if self.covered[layer].is_empty() {
                self.covered[layer] = vec![0; self.grid.cells()];
            }
            let mask = &self.nets[plane.net as usize].plane;
            if mask.is_empty() || mask[layer].is_empty() {
                continue;
            }
            let mut per_tile = vec![0u32; tiles];
            for (cell, inside) in mask[layer].iter().enumerate() {
                if *inside {
                    self.covered[layer][cell] = crate::grid::owner(plane.net);
                    per_tile[(cell / self.grid.nx / TILE) * self.tiles_x
                        + (cell % self.grid.nx) / TILE] += 1;
                }
            }
            for (tile, count) in per_tile.iter().enumerate() {
                let share = *count as f32 / (TILE * TILE) as f32;
                self.tile_covered[layer][tile] = self.tile_covered[layer][tile].max(share.min(1.0));
            }
        }
        let share: Vec<f32> = (0..self.board.layer_count)
            .map(|layer| {
                if self.covered[layer].is_empty() {
                    0.0
                } else {
                    self.covered[layer]
                        .iter()
                        .filter(|owner| **owner != 0)
                        .count() as f32
                        / self.grid.cells() as f32
                }
            })
            .collect();
        let least = share.iter().copied().fold(1.0f32, f32::min);
        for (layer, layer_share) in share.iter().enumerate() {
            let relative = (layer_share - least).max(0.0) / (1.0 - least).max(1.0e-3);
            self.layer_cut[layer] = 1.0 + (self.config.plane_cut_cost as f32 - 1.0) * relative;
        }
    }
}

/// What building a router took, counted: the work `Router::new` charges
/// (`RoutingResult::build_work`) at `BUILD_*_NANOS` each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildCounts {
    /// Lattice nodes of every class's maps (cells times layers times
    /// classes).
    pub map_nodes: u64,
    /// The same of a router whose static maps another built
    /// (`shared_statics`): its own maps only (the occupancy, the search
    /// records, the tile counts).
    pub shared_map_nodes: u64,
    /// Nodes tested: against the outline and every obstacle in the
    /// static maps, inside the nets' pads and pour boxes.
    pub tests: u64,
    /// The shape edges those tests measured (one for a circle or a
    /// capsule, the corners of a polygon measured edge by edge, a pour's
    /// corners for a node it may fill, the outline's per row).
    pub edges: u64,
    /// The edges the nodes near an obstacle measure their eight steps
    /// against (`mark_tight_edges`: a segment against each edge).
    pub step_edges: u64,
}

impl BuildCounts {
    /// The build's work in nanoseconds of the reference machine.
    pub fn work_nanos(&self) -> f64 {
        self.map_nodes as f64 * BUILD_MAP_NANOS
            + self.shared_map_nodes as f64 * BUILD_SHARED_MAP_NANOS
            + self.tests as f64 * BUILD_TEST_NANOS
            + self.edges as f64 * BUILD_EDGE_NANOS
            + self.step_edges as f64 * BUILD_STEP_NANOS
    }

    /// The build's work in seconds at this process's rate (the
    /// reference's nanoseconds scaled as the pour model's are,
    /// `reference_time_scale`).
    pub fn work_seconds(&self) -> f64 {
        self.work_nanos() / 1.0e9 * reference_time_scale()
    }
}

/// The static maps of the routers built last, by what they are built from
/// (`statics_key`): the most recent kept, the others while a router holds
/// them. The rung probes, the probed rung's continuation, the rungs of a
/// pitch, the seeds and the steps lower one board with one lattice and
/// one set of classes again and again, and the static maps (the outline,
/// every obstacle's nodes, the tight steps: about half of a build) are a
/// function of those alone.
type StaticsCache = Vec<(u128, std::sync::Weak<Vec<StaticMaps>>)>;

static STATICS_CACHE: std::sync::Mutex<(StaticsCache, Option<Arc<Vec<StaticMaps>>>)> =
    std::sync::Mutex::new((Vec::new(), None));

/// What the static maps are built from, hashed (128 bits): the outline,
/// the clearances, the classes and the nets' classes, the obstacles, the
/// lattice, which classes are searched and the rules beyond the classes,
/// each as its exact debug form (floats print as they parse back).
fn statics_key(board: &Board, grid: &Grid, searched: &[bool], rules: Option<&ClassRules>) -> u128 {
    use std::fmt::Write as _;
    use std::hash::Hasher as _;
    struct Feed([std::collections::hash_map::DefaultHasher; 2]);
    impl std::fmt::Write for Feed {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            for hasher in &mut self.0 {
                hasher.write(text.as_bytes());
            }
            Ok(())
        }
    }
    let mut feed = Feed([Default::default(), Default::default()]);
    feed.0[1].write_u64(0x5eed_5eed_5eed_5eed);
    let _ = write!(
        feed,
        "{} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
        board.layer_count,
        board.outline,
        (
            board.edge_clearance,
            board.hole_clearance,
            board.hole_to_hole,
            board.neck_width
        ),
        board.classes,
        board.nets.iter().map(|net| net.class).collect::<Vec<_>>(),
        (grid.pitch, grid.origin, grid.nx, grid.ny),
        searched,
        rules,
        // The pads no via may enter shape the via maps
        // (`Board::no_via_pads`).
        board.no_via_pads,
    );
    for obstacle in &board.obstacles {
        let _ = write!(feed, "{obstacle:?}");
    }
    ((feed.0[0].finish() as u128) << 64) | feed.0[1].finish() as u128
}

/// The static maps of every class: another router's of the same inputs
/// (`STATICS_CACHE`, and true), or built (false). Not shared under
/// `PCB_ROUTER_NO_SHARED_STATICS` (for attributing).
fn shared_statics(
    board: &Board,
    grid: &Grid,
    searched: &[bool],
    rules: Option<&ClassRules>,
) -> (Arc<Vec<StaticMaps>>, bool, Option<u128>) {
    let build = || build_statics(board, grid, searched, rules);
    if std::env::var_os("PCB_ROUTER_NO_SHARED_STATICS").is_some() {
        return (build(), false, None);
    }
    let key = statics_key(board, grid, searched, rules);
    let found = {
        let mut cache = STATICS_CACHE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        cache.0.retain(|(_, maps)| maps.strong_count() > 0);
        cache
            .0
            .iter()
            .find(|(other, _)| *other == key)
            .and_then(|(_, maps)| maps.upgrade())
    };
    if let Some(maps) = found {
        return (maps, true, Some(key));
    }
    // Built outside the lock: routers side by side of other inputs build
    // theirs at the same time (two of the same inputs may both build).
    let maps = build();
    let mut cache = STATICS_CACHE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(existing) = cache
        .0
        .iter()
        .find(|(other, _)| *other == key)
        .and_then(|(_, maps)| maps.upgrade())
    {
        return (existing, true, Some(key));
    }
    cache.0.push((key, Arc::downgrade(&maps)));
    cache.1 = Some(maps.clone());
    (maps, false, Some(key))
}

/// Every class's static maps, built.
fn build_statics(
    board: &Board,
    grid: &Grid,
    searched: &[bool],
    rules: Option<&ClassRules>,
) -> Arc<Vec<StaticMaps>> {
    Arc::new(
        (0..board.classes.len())
            .map(|class| StaticMaps::build_with_rules(board, grid, class, searched[class], rules))
            .collect(),
    )
}

/// Static maps held for the routers about to be built (`hold_static_maps`).
pub struct StaticMapsHold {
    _maps: Arc<Vec<StaticMaps>>,
}

/// The static maps a router of `board` and `config` would build: found
/// (another router's of the same inputs) or built now, and held while the
/// returned value lives, with the work of building them (0 when found):
/// what a router sharing them does not count (`BUILD_MAP_NANOS` less
/// `BUILD_SHARED_MAP_NANOS` per node, and their tests). The ladder holds
/// them before it starts routers side by side, so that one builds them in
/// a known order and each router counts the same work whichever starts
/// first.
pub fn hold_static_maps(board: &Board, config: &Config) -> (StaticMapsHold, f64) {
    let (board, neck_of, class_rules) = Router::with_neck_classes(board, config);
    let grid = Grid::choose(&board, &config.pitches);
    let searched = searched_classes(&board, &neck_of);
    let (maps, shared, key) = shared_statics(&board, &grid, &searched, class_rules.as_deref());
    // The connected pours' masks of every net, for the routers of these
    // maps and pours (`Router::prepared_pour_masks`).
    let mut mask_counts = [0u64; 2];
    if let Some(key) = key {
        let key = super::prepare::pour_masks_key(key, &board);
        let mut cache = super::prepare::POUR_MASKS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !cache.iter().any(|(other, _)| *other == key) {
            if cache.len() >= super::prepare::POUR_MASK_KEYS {
                cache.remove(0);
            }
            let mut entry = (key, Vec::new());
            let nets: Vec<NetId> = board
                .planes
                .iter()
                .filter(|plane| plane.connect)
                .map(|plane| plane.net)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            for net in nets {
                let (plane, plane_class, counts) =
                    super::prepare::compute_pour_masks(&board, &maps, &grid, net, true);
                mask_counts[0] += counts[0];
                mask_counts[1] += counts[1];
                entry.1.push((net, Arc::new((plane, plane_class))));
            }
            cache.push(entry);
        }
    }
    let masks = (mask_counts[0] as f64 * BUILD_TEST_NANOS
        + mask_counts[1] as f64 * BUILD_EDGE_NANOS)
        / 1.0e9
        * reference_time_scale();
    let work = masks
        + if shared {
            0.0
        } else {
            let nodes = (grid.cells() * board.layer_count * maps.len()) as f64;
            let tests: u64 = maps.iter().map(|maps| maps.tests).sum();
            let edges: u64 = maps.iter().map(|maps| maps.vertex_tests).sum();
            let steps: u64 = maps.iter().map(|maps| maps.tight_tests).sum();
            (nodes * (BUILD_MAP_NANOS - BUILD_SHARED_MAP_NANOS)
                + tests as f64 * BUILD_TEST_NANOS
                + edges as f64 * BUILD_EDGE_NANOS
                + steps as f64 * BUILD_STEP_NANOS)
                / 1.0e9
                * reference_time_scale()
        };
    (StaticMapsHold { _maps: maps }, work)
}

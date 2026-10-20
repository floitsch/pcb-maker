// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Lowering the board into the router's maps.

use super::*;

impl Router {
    /// `board` with a neck variant of every class wider than the neck (for
    /// routing next to fine-pitch pads), and per class its neck class.
    pub(super) fn with_neck_classes(board: &Board, config: &Config) -> (Board, Vec<Option<usize>>) {
        let mut board = board.clone();
        let mut neck_of = vec![None; board.classes.len()];
        if config.neck_reach > 0.0 {
            for (class, neck_class) in neck_of.iter_mut().enumerate() {
                let rules = board.classes[class];
                if rules.trace_width > board.neck_width + 1.0e-9 {
                    let neck = RuleClass {
                        trace_width: board.neck_width,
                        ..rules
                    };
                    let index = board
                        .classes
                        .iter()
                        .position(|existing| *existing == neck)
                        .unwrap_or_else(|| {
                            board.classes.push(neck);
                            board.classes.len() - 1
                        });
                    *neck_class = Some(index);
                }
            }
        }
        neck_of.resize(board.classes.len(), None);
        (board, neck_of)
    }

    pub fn new(board: &Board, config: &Config) -> Self {
        let started = std::time::Instant::now();
        let router = Self::build(board, config);
        BUILD_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        BUILD_NANOS.fetch_add(
            started.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        router
    }

    pub(super) fn build(board: &Board, config: &Config) -> Self {
        let built = std::time::Instant::now();
        let (board, neck_of) = Self::with_neck_classes(board, config);
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
        assert!(
            board.nets.len() + 1 < 1 << 15,
            "{} nets: the search records name nets in 15 bits",
            board.nets.len()
        );
        let searched = searched_classes(board, &neck_of);
        let statics: Vec<_> = (0..board.classes.len())
            .map(|class| StaticMaps::build(board, &grid, class, searched[class]))
            .collect();
        let maps = board.classes.len() * (layers + 1) + board.classes.len() * layers * 2;
        let step = grid.pitch * std::f64::consts::SQRT_2;
        let stamps = board
            .classes
            .iter()
            .map(|own| {
                board
                    .classes
                    .iter()
                    .map(|other| {
                        let clearance = own.clearance.max(other.clearance);
                        // Exact: a node at the radius is legal, and the
                        // steps between nodes are covered apart (see
                        // `diagonal_extra`; the search checks the cells a
                        // diagonal step cuts).
                        let with_margin = |radius: f64| radius;
                        let snap = grid.pitch * 0.75 + step * step / (4.0 * grid.pitch);
                        let trace_radius =
                            own.trace_width / 2.0 + other.trace_width / 2.0 + clearance;
                        // Copper keeps the copper clearance from a via's
                        // ring and the hole clearance from its drill;
                        // with a thin ring the hole rule is the larger
                        // (the laptop motherboard: 154 KiCad findings).
                        let hole = board.hole_clearance;
                        let via_radius =
                            (own.trace_width / 2.0 + other.via_diameter / 2.0 + clearance)
                                .max(own.trace_width / 2.0 + other.via_drill / 2.0 + hole);
                        let own_via_radius =
                            (own.via_diameter / 2.0 + other.trace_width / 2.0 + clearance)
                                .max(own.via_drill / 2.0 + other.trace_width / 2.0 + hole);
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
                                    own.trace_width / 2.0 + other.trace_width / 2.0 + clearance,
                                ) + snap,
                                grid.pitch,
                            ),
                            stub_to_via: disc(with_margin(via_radius) + snap, grid.pitch),
                            trace_to_trace: disc(
                                with_margin(
                                    own.trace_width / 2.0 + other.trace_width / 2.0 + clearance,
                                ),
                                grid.pitch,
                            ),
                            trace_to_via: disc(with_margin(via_radius), grid.pitch),
                            via_to_trace: disc(with_margin(own_via_radius), grid.pitch),
                            via_to_via: disc(
                                (own.via_diameter / 2.0 + other.via_diameter / 2.0 + clearance)
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
            statics: Arc::new(statics),
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
            frame_hook: None,
            search_seconds: 0.0,
            negotiation_seconds: 0.0,
            present_reached: 0.0,
            hopeless: false,
            stamp_seconds: 0.0,
            iterations: 0,
            last_conflicted: Vec::new(),
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
        router.nets = (0..board.nets.len())
            .map(|net| router.prepare_net(net as NetId))
            .collect();
        router.guard = router.thermal_guards();
        router.mark_covered();
        router.mark_via_cells();
        router.build_hot();
        if config.verbose {
            eprintln!(
                "{} (built in {:.1}s)",
                router.memory_report(),
                built.elapsed().as_secs_f64()
            );
        }
        router
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

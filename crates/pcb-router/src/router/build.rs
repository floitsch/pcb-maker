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
        Grid::choose(board, &config.pitches).check_size()?;
        Ok(Self::new(board, config))
    }

    /// Builds the router for `board`. Panics on a board the maps cannot
    /// hold (`try_new` returns that as an error).
    pub fn new(board: &Board, config: &Config) -> Self {
        let started = std::time::Instant::now();
        let router = Self::build(board, config);
        BUILD_WORK_NANOS.fetch_add(
            (router.physical_build_work() * 1.0e9) as u64,
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
        // Before any map is allocated (pcb-kicad's lowering refuses a
        // board whose lattice does not fit the memory first).
        if let Err(message) = grid.check_size() {
            panic!("{message} (Router::try_new returns it as an error)");
        }
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
        let tiles_x = grid.nx.div_ceil(TILE);
        let tiles_y = grid.ny.div_ceil(TILE);
        let mut tile_routable = vec![vec![0u32; tiles_x * tiles_y]; board.classes.len() * layers];
        for class in 0..board.classes.len() {
            for layer in 0..layers {
                let counts = &mut tile_routable[class * layers + layer];
                for (cell, value) in statics[class].trace[layer].iter().enumerate() {
                    if *value != crate::grid::BLOCKED16 {
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
            hot_pours: Vec::new(),
            tile_covered: vec![vec![0.0; tiles_x * tiles_y]; layers],
            layer_cut: vec![1.0; layers],
            sever: vec![Vec::new(); layers],
            sever_scale: sever::SEVER_COST * sever::integrity(config, board) as f32 / 255.0,
            sever_at: None,
            sever_expansions: 0,
            copper_changes: 0,
            cover_mask_counts: [0; 2],
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
            scratch: Scratch::default(),
            spare_scratches: SpareScratches::default(),
            profile: Profile::default(),
            in_via_reduction: false,
            in_probe: false,
            cut_iteration: None,
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
                counts.map_nodes,
                counts.tests,
                counts.edges,
                counts.step_edges,
                cells * layers * router.searched.iter().filter(|searched| **searched).count(),
                router.physical_build_work() * 1.0e9 / reference_time_scale(),
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

    /// What building this router took, counted (`BuildCounts`): its maps,
    /// every class's static maps and its nets' preparation with their pour
    /// masks, as a router that built them all itself counts them. The same
    /// whether it built its static maps and masks or found them
    /// (`shared_statics`, `POUR_MASKS`): the canonical price, so that what
    /// a router counts does not depend on what else the process holds
    /// (which followed how many routers ran side by side, and so the free
    /// memory). What a router of the same maps already paid is
    /// `shared_build`; what this process did is `physical_build_work`.
    pub(super) fn build_counts(&self) -> BuildCounts {
        let layers = self.board.layer_count as u64;
        let prepared = |index: usize| -> u64 {
            self.nets
                .iter()
                .map(|state| state.prepare_counts[index] + state.mask_counts[index])
                .sum()
        };
        let statics =
            |count: fn(&StaticMaps) -> u64| -> u64 { self.statics.iter().map(count).sum() };
        BuildCounts {
            map_nodes: self.grid.cells() as u64 * layers * self.statics.len() as u64,
            tests: statics(|maps| maps.tests) + prepared(0) + self.cover_mask_counts[0],
            edges: statics(|maps| maps.vertex_tests) + prepared(1) + self.cover_mask_counts[1],
            step_edges: statics(|maps| maps.tight_tests),
        }
    }

    /// The parts of `build_work` routers of the same inputs share
    /// (`SharedBuild`): the static maps beyond a router's own maps and the
    /// connected pours' masks, each with its key; none when the maps are
    /// not shared (`Config::share_static_maps`, `PCB_ROUTER_NO_SHARED_STATICS`).
    pub fn shared_build(&self) -> SharedBuild {
        let Some(key) = self.statics_key else {
            return SharedBuild::default();
        };
        let masks: [u64; 2] = [0, 1].map(|index| {
            self.nets
                .iter()
                .map(|state| state.mask_counts[index])
                .sum::<u64>()
        });
        SharedBuild {
            statics: Some((
                key,
                statics_part_work(&self.statics, &self.grid, self.board.layer_count),
            )),
            masks: (masks != [0, 0]).then(|| {
                (
                    super::prepare::pour_masks_key(key, &self.board),
                    masks_work(masks),
                )
            }),
        }
    }

    /// The work this process did building this router: `build_work` less
    /// the static maps and masks it found (another router's or a hold's).
    pub(super) fn physical_build_work(&self) -> f64 {
        let shared = self.shared_build();
        let masks_found = self.nets.iter().any(|state| state.masks_found);
        self.build_work
            - if self.statics_shared {
                shared.statics.map_or(0.0, |(_, work)| work)
            } else {
                0.0
            }
            - if masks_found {
                shared.masks.map_or(0.0, |(_, work)| work)
            } else {
                0.0
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
        let scratch = Scratch::bytes_for(cells * self.board.layer_count, cells);
        let tiles = bytes(&self.tile_claimed)
            + bytes(&self.tile_routable)
            + bytes(&self.tile_history)
            + bytes(&self.tile_covered)
            + bytes(&self.sever);
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
            mb(scratch),
            mb(tiles),
        )
    }

    /// Packs the static map, the guard, the pour cover and the history
    /// per class and layer (see `hot`); after any of them changed. In
    /// place where the records exist: a second set beside the first was
    /// the peak of big boards (katia: 1.1 GB more while the lost-spokes
    /// step set and restored its guards).
    pub(super) fn build_hot(&mut self) {
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        let mut hot = std::mem::take(&mut self.hot);
        hot.resize_with(self.statics.len() * layers, Vec::new);
        let mut pours: Vec<(u16, u16)> = vec![(0, 0)];
        let mut pour_index: FastMap<(u16, u16), u16> = FastMap::default();
        let mut pour_of = vec![0u16; cells];
        for layer in 0..layers {
            // The pair index per node, the same for every class (runs of
            // one pair are the rule: one lookup a run).
            let guard = &self.guard[layer];
            let covered = &self.covered[layer];
            let mut last = ((0u16, 0u16), 0u16);
            for (cell, pour) in pour_of.iter_mut().enumerate() {
                let pair = (
                    guard.get(cell).map_or(0, |&value| value as u16),
                    covered.get(cell).map_or(0, |&value| value as u16),
                );
                *pour = if pair == (0, 0) {
                    0
                } else if pair == last.0 {
                    last.1
                } else {
                    let index = *pour_index.entry(pair).or_insert_with(|| {
                        if pours.len() < HOT_POURS_MAPS as usize {
                            pours.push(pair);
                            (pours.len() - 1) as u16
                        } else {
                            HOT_POURS_MAPS
                        }
                    });
                    last = (pair, index);
                    index
                };
            }
            let history = &self.history[layer];
            for class in 0..self.statics.len() {
                let records = &mut hot[class * layers + layer];
                if !self.searched[class] {
                    *records = Vec::new();
                    continue;
                }
                if records.len() != cells {
                    *records = vec![Hot::default(); cells];
                }
                let trace = &self.statics[class].trace[layer];
                for (cell, record) in records.iter_mut().enumerate() {
                    *record = Hot {
                        history: history[cell],
                        trace: trace[cell],
                        pour: pour_of[cell],
                    };
                }
            }
        }
        self.hot = hot;
        self.hot_pours = pours;
    }

    /// The (guard, cover) pair of a `Hot::pour` other than 0, at `cell` of
    /// `layer`.
    #[inline(always)]
    pub(super) fn hot_pour(&self, pour: u16, layer: usize, cell: usize) -> (u16, u16) {
        if pour == HOT_POURS_MAPS {
            (
                self.guard[layer].get(cell).map_or(0, |&value| value as u16),
                self.covered[layer]
                    .get(cell)
                    .map_or(0, |&value| value as u16),
            )
        } else {
            self.hot_pours[pour as usize]
        }
    }

    /// The scratches for a parallel batch's jobs: this router's own
    /// (first) and its spares (`ScratchPool`), given back by
    /// `return_scratches`.
    pub(super) fn lend_scratches(&mut self) -> ScratchPool {
        let mut scratches = std::mem::take(&mut self.spare_scratches.0);
        if !self.scratch.nodes.is_empty() {
            scratches.push(self.scratch.take_arrays());
        }
        ScratchPool(std::sync::Mutex::new(scratches))
    }

    /// Takes back the scratches of `lend_scratches`: one as this router's
    /// own, the rest as spares.
    pub(super) fn return_scratches(&mut self, pool: ScratchPool) {
        let mut scratches = pool
            .0
            .into_inner()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(own) = scratches.pop() {
            self.scratch.restore_arrays(own);
        }
        self.spare_scratches.0 = scratches;
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
    /// With `pour_integrity`, a pour not connected (its net routed as
    /// tracks) covers the nodes KiCad's fill would take as well: cutting
    /// it costs what cutting a connected one costs.
    pub(super) fn mark_covered(&mut self) {
        let tiles = self.tiles_x * self.tiles_y;
        self.sever_at = None;
        self.cover_mask_counts = [0; 2];
        // The masks of the pours not connected, per net (computed once).
        let mut unconnected: Vec<(NetId, Vec<Vec<bool>>)> = Vec::new();
        if sever::integrity(&self.config, &self.board) > 0.0 {
            for plane in &self.board.planes {
                let own = &self.nets[plane.net as usize].plane;
                if (own.is_empty() || own[plane.layer].is_empty())
                    && !unconnected.iter().any(|(net, _)| *net == plane.net)
                {
                    let (mask, _, counts) = super::prepare::compute_pour_masks(
                        &self.board,
                        &self.statics,
                        &self.grid,
                        plane.net,
                        false,
                    );
                    self.cover_mask_counts[0] += counts[0];
                    self.cover_mask_counts[1] += counts[1];
                    unconnected.push((plane.net, mask));
                }
            }
        }
        for plane in &self.board.planes {
            let layer = plane.layer;
            if self.covered[layer].is_empty() {
                self.covered[layer] = vec![0; self.grid.cells()];
            }
            let mut mask = &self.nets[plane.net as usize].plane;
            if mask.is_empty() || mask[layer].is_empty() {
                match unconnected.iter().find(|(net, _)| *net == plane.net) {
                    Some((_, other)) => mask = other,
                    None => continue,
                }
            }
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

/// The parts of a router's build (`RoutingResult::build_work`) that
/// routers of the same inputs share, each with the key of what it is
/// built from: the static maps beyond the router's own maps (every
/// class's nodes at `BUILD_MAP_NANOS` less `BUILD_SHARED_MAP_NANOS`, their
/// tests, edges and tight steps; `statics_key`) and the connected pours'
/// masks (`pour_masks_key`). Route mode's ladder charges each key once a
/// phase, in its rungs' order, and holds the maps for the phase
/// (`hold_static_maps`), so that what it charges is what was built and
/// does not depend on how many routers ran side by side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SharedBuild {
    pub statics: Option<(u128, f64)>,
    pub masks: Option<(u128, f64)>,
}

impl SharedBuild {
    /// The parts, keyed.
    pub fn parts(&self) -> impl Iterator<Item = (u128, f64)> {
        self.statics.into_iter().chain(self.masks)
    }

    /// The parts' work together.
    pub fn total(&self) -> f64 {
        self.parts().map(|(_, work)| work).sum()
    }

    /// The static maps' key (the parts' group).
    pub fn key(&self) -> Option<u128> {
        self.statics.map(|(key, _)| key)
    }
}

/// The static maps' part of a build (`SharedBuild::statics`): every
/// class's nodes beyond a router's own maps, and the maps' counts.
fn statics_part_work(maps: &[StaticMaps], grid: &Grid, layers: usize) -> f64 {
    let nodes = (grid.cells() * layers * maps.len()) as f64;
    let tests: u64 = maps.iter().map(|maps| maps.tests).sum();
    let edges: u64 = maps.iter().map(|maps| maps.vertex_tests).sum();
    let steps: u64 = maps.iter().map(|maps| maps.tight_tests).sum();
    (nodes * (BUILD_MAP_NANOS - BUILD_SHARED_MAP_NANOS)
        + tests as f64 * BUILD_TEST_NANOS
        + edges as f64 * BUILD_EDGE_NANOS
        + steps as f64 * BUILD_STEP_NANOS)
        / 1.0e9
        * reference_time_scale()
}

/// The pour masks' part of a build (`SharedBuild::masks`).
fn masks_work(counts: [u64; 2]) -> f64 {
    (counts[0] as f64 * BUILD_TEST_NANOS + counts[1] as f64 * BUILD_EDGE_NANOS) / 1.0e9
        * reference_time_scale()
}

/// The static maps routers and holds of this process hold, by what they
/// are built from (`statics_key`): weak entries, so that nothing outlives
/// its holders. The rung probes, the probed rung's continuation, the rungs
/// of a pitch, the seeds and the steps lower one board with one lattice
/// and one set of classes again and again, and the static maps (the
/// outline, every obstacle's nodes, the tight steps: about half of a
/// build) are a function of those alone. Until 2026-10-21 the last maps
/// built were also kept for the process's life (a router's worth of
/// memory after a fine rung, and across a layout's move routers); route
/// mode's ladder holds what a phase shares instead (`hold_static_maps`)
/// and releases it when the phase ends.
type StaticsCache = Vec<(u128, std::sync::Weak<Vec<StaticMaps>>)>;

static STATICS_CACHE: std::sync::Mutex<StaticsCache> = std::sync::Mutex::new(Vec::new());

/// The bytes of the static maps and of the pour masks this process holds
/// (`STATICS_CACHE`, `POUR_MASKS`: every live router's and hold's), for
/// the ladder's memory accounting.
pub fn held_maps_bytes() -> (usize, usize) {
    let statics = {
        let mut cache = STATICS_CACHE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        cache.retain(|(_, maps)| maps.strong_count() > 0);
        cache
            .iter()
            .filter_map(|(_, maps)| maps.upgrade())
            .map(|maps| maps.iter().map(StaticMaps::bytes).sum::<usize>())
            .sum()
    };
    let masks = {
        let mut cache = super::prepare::POUR_MASKS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        cache.retain(|(_, masks)| masks.strong_count() > 0);
        cache
            .iter()
            .filter_map(|(_, masks)| masks.upgrade())
            .map(|masks| super::prepare::mask_set_bytes(&masks))
            .sum()
    };
    (statics, masks)
}

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
        "{} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
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
        // Keepouts the fill passes shape the fill trace.
        board.fill_passes,
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
        cache.retain(|(_, maps)| maps.strong_count() > 0);
        cache
            .iter()
            .find(|(other, _)| *other == key)
            .and_then(|(_, maps)| maps.upgrade())
    };
    if let Some(maps) = found {
        return (maps, true, Some(key));
    }
    // Built outside the lock: routers side by side of other inputs build
    // theirs at the same time (two of the same inputs may both build; the
    // second takes the first's maps, but did the work: not shared).
    let maps = build();
    let mut cache = STATICS_CACHE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(existing) = cache
        .iter()
        .find(|(other, _)| *other == key)
        .and_then(|(_, maps)| maps.upgrade())
    {
        return (existing, false, Some(key));
    }
    cache.push((key, Arc::downgrade(&maps)));
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

/// Static maps (and the connected pours' masks) held for the routers
/// about to be built (`hold_static_maps`).
pub struct StaticMapsHold {
    maps: Arc<Vec<StaticMaps>>,
    masks: Option<Arc<super::prepare::PourMaskSet>>,
    key: Option<u128>,
    built: f64,
}

impl StaticMapsHold {
    /// The static maps' key (None when they are not shared).
    pub fn key(&self) -> Option<u128> {
        self.key
    }

    /// The work this hold did: the static maps beyond a router's own and
    /// the masks it built (0 where it found them).
    pub fn built_work(&self) -> f64 {
        self.built
    }

    /// The bytes it holds (the static maps and the masks).
    pub fn bytes(&self) -> usize {
        self.maps.iter().map(StaticMaps::bytes).sum::<usize>()
            + self
                .masks
                .as_ref()
                .map_or(0, |masks| super::prepare::mask_set_bytes(masks))
    }
}

/// The inputs of the static maps of a router of `board` and `config`: the
/// board with its rule and neck classes, the lattice, which classes are
/// searched.
fn statics_inputs(
    board: &Board,
    config: &Config,
) -> (Board, Grid, Vec<bool>, Option<Arc<ClassRules>>) {
    let (board, neck_of, class_rules) = Router::with_neck_classes(board, config);
    let grid = Grid::choose(&board, &config.pitches);
    let searched = searched_classes(&board, &neck_of);
    (board, grid, searched, class_rules)
}

/// The key of the static maps a router of `board` and `config` would
/// build (`statics_key`; None when they are not shared).
pub fn static_maps_key(board: &Board, config: &Config) -> Option<u128> {
    if !config.share_static_maps || std::env::var_os("PCB_ROUTER_NO_SHARED_STATICS").is_some() {
        return None;
    }
    let (board, grid, searched, class_rules) = statics_inputs(board, config);
    Some(statics_key(
        &board,
        &grid,
        &searched,
        class_rules.as_deref(),
    ))
}

/// The static maps a router of `board` and `config` would build, and the
/// masks of its connected pours: found (another router's or hold's of the
/// same inputs) or built now, and held while the returned value lives. The
/// ladder holds them before it starts routers side by side and for the
/// rest of a phase, so that one builds them in a known order and the work
/// it charges (`SharedBuild`, once a key a phase) is the work done. What
/// the hold built is added to the process's modelled lattice work
/// (`ProcessAccounting::build_work_seconds`).
pub fn hold_static_maps(board: &Board, config: &Config) -> StaticMapsHold {
    let (board, grid, searched, class_rules) = statics_inputs(board, config);
    let (maps, shared, key) = shared_statics(&board, &grid, &searched, class_rules.as_deref());
    // The connected pours' masks of every net, for the routers of these
    // maps and pours (`Router::prepared_pour_masks`).
    let mut built = if shared {
        0.0
    } else {
        statics_part_work(&maps, &grid, board.layer_count)
    };
    let mut masks = None;
    if let Some(key) = key {
        let key = super::prepare::pour_masks_key(key, &board);
        let mut cache = super::prepare::POUR_MASKS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        cache.retain(|(_, masks)| masks.strong_count() > 0);
        masks = cache
            .iter()
            .find(|(other, _)| *other == key)
            .and_then(|(_, masks)| masks.upgrade());
        if masks.is_none() {
            let nets: Vec<NetId> = board
                .planes
                .iter()
                .filter(|plane| plane.connect)
                .map(|plane| plane.net)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            let mut set = Vec::new();
            let mut counts = [0u64; 2];
            for net in nets {
                let mask = super::prepare::compute_pour_masks(&board, &maps, &grid, net, true);
                counts[0] += mask.2[0];
                counts[1] += mask.2[1];
                set.push((net, Arc::new(mask)));
            }
            built += masks_work(counts);
            let set = Arc::new(set);
            cache.push((key, Arc::downgrade(&set)));
            masks = Some(set);
        }
    }
    BUILD_WORK_NANOS.fetch_add((built * 1.0e9) as u64, std::sync::atomic::Ordering::Relaxed);
    StaticMapsHold {
        maps,
        masks,
        key,
        built,
    }
}

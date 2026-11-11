// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Building a class's static maps (`StaticMaps::build_with_rules`) in
//! stages: the nodes inside the outline, the outline's edge zones, then
//! every obstacle in order (its reaches, the vias it keeps out, the tracks
//! it keeps out), then the fill's record and the per-node layer bits.

use super::*;

/// The class's geometry as the build measures it.
struct ClassGeometry<'a> {
    rules: crate::board::RuleClass,
    half_width: f64,
    /// A class with widths per layer (`ClassRules::class_widths`): its
    /// half width per layer (`half_width` is then the widest); else empty.
    half_widths: Vec<f64>,
    via_radius: f64,
    /// The longest step (lines at the pins may lie farther apart than
    /// the pitch): the sagitta margins follow it.
    longest: f64,
    /// The longest step's diagonal.
    step: f64,
    net_rules: Option<&'a crate::rules::ClassRules>,
}

impl ClassGeometry<'_> {
    /// The half width on `layer`.
    fn half_on(&self, layer: usize) -> f64 {
        self.half_widths
            .get(layer)
            .copied()
            .unwrap_or(self.half_width)
    }
}

/// The maps while they are built, and what building them took.
struct Building {
    trace: Vec<Vec<u16>>,
    edges: Vec<EdgeMap>,
    via_blocked: Vec<bool>,
    via_layers: Vec<u32>,
    via_owner: Vec<u16>,
    /// Where `via_owner` holds a via inside its net's own copper (a
    /// touch electrode's, an exposed pad's, at its own plated hole),
    /// not beside a pad: only there does a touch electrode's mask
    /// opening leave the via to its net (`own_opening`). Built only on
    /// a board with such an opening.
    via_inside: Vec<bool>,
    fill_trace: Option<FillTrace>,
    /// Per layer (or one for all layers), the outline's edge zone.
    edge_zone: Vec<Vec<bool>>,
    tests: u64,
    vertex_tests: u64,
    tight_tests: u64,
}

/// How far one obstacle keeps this class's tracks and vias.
struct Reaches {
    /// The fill's clearance from copper, and how far it keeps a track's
    /// centre for it.
    fill_clearance: Option<f64>,
    fill_reach: Option<f64>,
    /// A track's centre (the largest over the layers with rules per
    /// layer), and per layer where those differ.
    trace_reach: f64,
    ruled_trace: Option<Vec<f64>>,
    via_reach: f64,
    /// The same-net hole-to-hole spacing, and how far it keeps a via's
    /// centre from its own net's drill.
    same_net_hole: f64,
    own_hole_reach: f64,
    /// The sagitta margin of `trace_reach`.
    margin: f64,
}

impl StaticMaps {
    /// `build` with the rules beyond the net classes (`crate::rules`):
    /// a pad, an existing track or via keeps what the rules ask of this
    /// class's tracks (per layer) and vias, and a via keeps the same-net
    /// hole-to-hole spacing from its own net's plated holes.
    pub fn build_with_rules(
        board: &Board,
        grid: &Grid,
        class: ClassId,
        searched: bool,
        net_rules: Option<&crate::rules::ClassRules>,
    ) -> Self {
        // Input-reachable only through a caller that skips the check
        // (`check_capacity`; the router's build checks it first).
        assert!(
            board.nets.len() <= MAX_NETS,
            "{} nets: the static maps name nets in 16 bits (check_capacity)",
            board.nets.len()
        );
        let rules = board.classes[class];
        let half_width = rules.trace_width / 2.0;
        let half_widths: Vec<f64> = match net_rules {
            Some(split) if !split.class_widths[class].is_empty() => split.class_widths[class]
                .iter()
                .map(|width| width / 2.0)
                .collect(),
            _ => Vec::new(),
        };
        let longest = grid.steps().1;
        let geometry = ClassGeometry {
            rules,
            half_width,
            half_widths,
            via_radius: rules.via_diameter / 2.0,
            longest,
            step: longest * std::f64::consts::SQRT_2,
            net_rules,
        };
        let mut building = Self::outline_maps(board, grid, &geometry, searched);
        Self::block_obstacles(board, grid, &geometry, class, searched, &mut building);
        Self::finish(board, grid, &geometry, searched, building)
    }

    /// The maps with the outline alone: outside it and within its edge
    /// zones nothing goes.
    fn outline_maps(
        board: &Board,
        grid: &Grid,
        geometry: &ClassGeometry,
        searched: bool,
    ) -> Building {
        let cells = grid.cells();
        // What the build does, counted (`tests`, `vertex_tests`): the
        // outline's crossings per row, its edges' neighbourhoods, every
        // node tested against an obstacle and the edges each test measures.
        let outline_edges = polygon_edges(&board.outline).count() as u64;
        let (mut tests, mut vertex_tests) = (cells as u64, grid.ny as u64 * outline_edges);
        let inside = inside_outline(board, grid);

        let mut base_trace: Vec<u16> = inside
            .iter()
            .map(|inside| if *inside { FREE16 } else { BLOCKED16 })
            .collect();
        let mut via_blocked: Vec<bool> = inside.iter().map(|inside| !inside).collect();
        // With via spans (`Board::via_spans`): per node, the layers on which
        // an obstacle keeps a via of the class out, whatever its net.
        let all_layers = board.all_layers();
        let spans = searched && !board.via_spans.is_empty() && board.layer_count > 2;
        let mut via_layers: Vec<u32> = if spans {
            inside
                .iter()
                .map(|inside| if *inside { 0 } else { all_layers })
                .collect()
        } else {
            Vec::new()
        };
        let via_owner: Vec<u16> = if searched { vec![0; cells] } else { Vec::new() };
        let via_inside: Vec<bool> = if searched
            && board
                .obstacles
                .iter()
                .any(|obstacle| obstacle.net.is_some() && own_opening_label(obstacle))
        {
            vec![false; cells]
        } else {
            Vec::new()
        };
        let (half_widths, step, longest) = (&geometry.half_widths, geometry.step, geometry.longest);
        let trace_edge = geometry.half_width + board.edge_clearance + SAFETY;
        let via_edge = geometry.via_radius + board.edge_clearance + SAFETY;
        // A unit step between two legal nodes can cut an outline corner by
        // at most the sagitta of that chord.
        let edge_margin = step * step / (8.0 * trace_edge.max(longest));
        // Per layer, where the widths differ: the edge distance of every
        // node near the outline, for the layers' own zones below.
        let mut edge_distance: Vec<(usize, f64)> = Vec::new();
        for (a, b) in polygon_edges(&board.outline) {
            let reach = trace_edge.max(via_edge) + edge_margin;
            let bounds = Aabb {
                minimum: [a[0].min(b[0]), a[1].min(b[1])],
                maximum: [a[0].max(b[0]), a[1].max(b[1])],
            }
            .inflated(reach);
            let Some((x0, y0, x1, y1)) = grid.node_range(bounds) else {
                continue;
            };
            let nodes = ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;
            tests += nodes;
            vertex_tests += nodes;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let distance = crate::geometry::point_segment_distance(grid.center(x, y), a, b);
                    let index = grid.index(x, y);
                    if !half_widths.is_empty() {
                        edge_distance.push((index, distance));
                    } else if distance < trace_edge + edge_margin {
                        base_trace[index] = BLOCKED16;
                    }
                    if distance < via_edge {
                        via_blocked[index] = true;
                        if let Some(layers) = via_layers.get_mut(index) {
                            *layers = all_layers;
                        }
                    }
                }
            }
        }

        // A pad may straddle the outline (card-edge fingers, connector
        // shells) but a track centreline on it still keeps the edge
        // clearance: pad cells inside the edge zone stay blocked.
        let mut trace = vec![base_trace; board.layer_count];
        // Each layer's edge zone at its own width.
        for (layer, map) in trace.iter_mut().enumerate() {
            if half_widths.is_empty() {
                break;
            }
            let trace_edge = geometry.half_on(layer) + board.edge_clearance + SAFETY;
            let edge_margin = step * step / (8.0 * trace_edge.max(longest));
            for &(index, distance) in &edge_distance {
                if distance < trace_edge + edge_margin {
                    map[index] = BLOCKED16;
                }
            }
        }
        drop(edge_distance);
        let edge_zone: Vec<Vec<bool>> = trace
            .iter()
            .take(if half_widths.is_empty() {
                1
            } else {
                board.layer_count
            })
            .map(|map| map.iter().map(|value| *value == BLOCKED16).collect())
            .collect();
        Building {
            trace,
            edges: vec![EdgeMap::new(cells); board.layer_count],
            via_blocked,
            via_layers,
            via_owner,
            via_inside,
            fill_trace: None,
            edge_zone,
            tests,
            vertex_tests,
            tight_tests: 0,
        }
    }

    /// Blocks around every obstacle in order: the others first, then the
    /// pads with a smaller clearance for the fill, then what keeps tracks
    /// out but lets the fill in (from the first of those on, the fill's
    /// own map differs where it may cover more: `FillTrace`).
    fn block_obstacles(
        board: &Board,
        grid: &Grid,
        geometry: &ClassGeometry,
        class: ClassId,
        searched: bool,
        building: &mut Building,
    ) {
        let rules = &geometry.rules;
        let cells = grid.cells();
        // Solder mask openings keep tracks out but not a pour's fill: they
        // come last, and the trace map before them is the pours' (MokyaLora:
        // a motor's shield pad under a mask opening looked cut off from its
        // ground pour, which KiCad fills there).
        let mask_opening = |obstacle: &&crate::board::Obstacle| {
            obstacle.kind == ObstacleKind::Keepout
                && obstacle.label.starts_with("solder mask opening")
        };
        // So do the rule areas that keep tracks out but let pours in
        // (`Board::fill_passes`: a plane layer's `no tracks` area), which
        // hid the plane's fill from the pour model before 2026-10-21.
        let passes = |index: usize, obstacle: &&crate::board::Obstacle| {
            mask_opening(obstacle) || board.fill_passes.binary_search(&index).is_ok()
        };
        // A pad's own clearance override replaces the zone's clearance in
        // KiCad's fill also where it is smaller than this class's (which
        // the tracks keep): such pads come after the others, and the fill
        // keeps only the override from them (`Board::fill_clearance`).
        let overridden = |obstacle: &&crate::board::Obstacle| {
            obstacle.kind == ObstacleKind::Copper
                && !mask_opening(obstacle)
                && board.fill_clearance(rules, obstacle)
                    < board.copper_clearance(rules, obstacle) - 1.0e-9
        };
        let ordered: Vec<(usize, &crate::board::Obstacle)> = board
            .obstacles
            .iter()
            .enumerate()
            .filter(|(index, obstacle)| !passes(*index, obstacle) && !overridden(obstacle))
            .chain(
                board
                    .obstacles
                    .iter()
                    .enumerate()
                    .filter(|(_, obstacle)| overridden(obstacle)),
            )
            .chain(
                board
                    .obstacles
                    .iter()
                    .enumerate()
                    .filter(|(index, obstacle)| passes(*index, obstacle)),
            )
            .collect();
        let first_apart = ordered
            .iter()
            .position(|(index, obstacle)| passes(*index, obstacle) || overridden(obstacle));
        for (position, (obstacle_index, obstacle)) in ordered.into_iter().enumerate() {
            if Some(position) == first_apart {
                building.fill_trace = Some(FillTrace::new(board.layer_count, cells));
            }
            // From the first pad apart on: where the fill keeps away too
            // (within the fill's reach of an overridden pad; any other
            // copper), a remembered value is claimed as well.
            let opening = mask_opening(&obstacle);
            let reaches = Self::reaches(board, geometry, class, obstacle, opening);
            let bounds = obstacle.shape.aabb();

            // A polygon of many corners (a board cutout, a rule area) is
            // measured once per node from its edges' neighbourhoods
            // (`PolygonField`), not edge by edge for every node of its box
            // (ULK's 454-corner cutout: 15 s a class, 60-190 s a lattice).
            let field = match &obstacle.shape {
                Shape::Polygon { points } if points.len() >= FIELD_CORNERS => {
                    let via_range = obstacle
                        .blocks_vias
                        .then(|| grid.node_range(bounds.inflated(reaches.via_reach)))
                        .flatten();
                    let trace_range = obstacle
                        .blocks_tracks
                        .then(|| {
                            grid.node_range(bounds.inflated(reaches.trace_reach + reaches.margin))
                        })
                        .flatten();
                    [via_range, trace_range]
                        .into_iter()
                        .flatten()
                        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
                        .map(|range| {
                            let field = PolygonField::new(
                                grid,
                                points,
                                range,
                                reaches.via_reach.max(reaches.trace_reach + reaches.margin),
                            );
                            building.tests += field.distances.len() as u64;
                            building.vertex_tests += field.measured;
                            field
                        })
                }
                _ => None,
            };
            let corners = if field.is_some() {
                0
            } else {
                shape_corners(&obstacle.shape)
            };
            let distance_at = |x: usize, y: usize, center: Point| match &field {
                Some(field) => field.distance(x, y),
                None => obstacle.shape.distance_to_point(center),
            };
            if obstacle.blocks_vias {
                Self::block_vias(
                    board,
                    grid,
                    geometry,
                    searched,
                    building,
                    (obstacle_index, obstacle),
                    &reaches,
                    corners,
                    &distance_at,
                );
            }
            if !obstacle.blocks_tracks {
                continue;
            }
            let tight = Self::block_tracks(
                board,
                grid,
                geometry,
                building,
                obstacle,
                &reaches,
                corners,
                &distance_at,
            );
            // A tight node measures its eight steps against the shape.
            building.tight_tests += tight * 8 * shape_corners(&obstacle.shape);
        }
    }

    /// How far `obstacle` keeps this class's tracks and vias (`opening`:
    /// whether it is a solder mask opening).
    fn reaches(
        board: &Board,
        geometry: &ClassGeometry,
        class: ClassId,
        obstacle: &crate::board::Obstacle,
        opening: bool,
    ) -> Reaches {
        let (rules, half_width, via_radius) =
            (&geometry.rules, geometry.half_width, geometry.via_radius);
        let (half_widths, step, net_rules) =
            (&geometry.half_widths, geometry.step, geometry.net_rules);
        let fill_clearance = (obstacle.kind == ObstacleKind::Copper && !opening)
            .then(|| board.fill_clearance(rules, obstacle));
        let fill_reach = fill_clearance.map(|clearance| half_width + clearance + SAFETY);
        let (trace_clearance, via_reach) = match obstacle.kind {
            ObstacleKind::Copper => {
                let clearance = board.copper_clearance(rules, obstacle);
                // Copper keeps the hole clearance from a via's drill too.
                (
                    clearance,
                    (via_radius + clearance).max(rules.via_drill / 2.0 + board.hole_clearance),
                )
            }
            ObstacleKind::Keepout => (obstacle.clearance, via_radius + obstacle.clearance),
            ObstacleKind::Hole => {
                let clearance = board.hole_clearance.max(obstacle.clearance);
                (
                    clearance,
                    (via_radius + clearance).max(rules.via_drill / 2.0 + board.hole_to_hole),
                )
            }
        };
        let trace_reach = half_width + trace_clearance + SAFETY;
        let via_reach = via_reach + SAFETY;
        // The rules beyond the classes: per layer for tracks, for vias
        // the largest over the obstacle's layers; and a class's widths
        // per layer.
        let mut ruled_trace: Option<Vec<f64>> = None;
        let mut via_reach = via_reach;
        if !half_widths.is_empty() {
            ruled_trace = Some(
                (0..board.layer_count)
                    .map(|layer| geometry.half_on(layer) + trace_clearance + SAFETY)
                    .collect(),
            );
        }
        if let Some(split) = net_rules
            && obstacle.kind == ObstacleKind::Copper
        {
            let per_layer: Vec<f64> = (0..board.layer_count)
                .map(|layer| {
                    let own = ruled_trace.as_ref().map_or(trace_reach, |per| per[layer]);
                    (geometry.half_on(layer)
                        + split.obstacle_extra(class, crate::rules::TRACK, obstacle, layer)
                        + SAFETY)
                        .max(own)
                })
                .collect();
            if !half_widths.is_empty() || per_layer.iter().any(|reach| *reach > trace_reach) {
                ruled_trace = Some(per_layer);
            }
            via_reach = via_reach.max(
                via_radius + split.obstacle_extra_via(class, obstacle, board.layer_count) + SAFETY,
            );
        }
        let trace_reach = ruled_trace.as_ref().map_or(trace_reach, |per_layer| {
            per_layer.iter().copied().fold(trace_reach, f64::max)
        });
        // A via of the hole's own net keeps the same-net hole-to-hole
        // spacing from its drill (KiCad's hole_to_hole holds within a
        // net: nuclone's VSS via 0.464 mm from J3's VSS pad hole).
        let same_net_hole = net_rules
            .map_or(0.0, |split| split.resolved.same_net_hole_to_hole)
            .max(0.0);
        let own_hole_reach = rules.via_drill / 2.0 + same_net_hole + SAFETY;
        let margin = step * step / (8.0 * trace_reach);
        Reaches {
            fill_clearance,
            fill_reach,
            trace_reach,
            ruled_trace,
            via_reach,
            same_net_hole,
            own_hole_reach,
            margin,
        }
    }

    /// Blocks vias of this class within `via_reach` of the obstacle,
    /// leaving them to its own net where its own copper or hole allows
    /// (`via_owner`).
    #[allow(clippy::too_many_arguments)]
    fn block_vias(
        board: &Board,
        grid: &Grid,
        geometry: &ClassGeometry,
        searched: bool,
        building: &mut Building,
        (obstacle_index, obstacle): (usize, &crate::board::Obstacle),
        reaches: &Reaches,
        corners: u64,
        distance_at: &impl Fn(usize, usize, Point) -> f64,
    ) {
        let via_radius = geometry.via_radius;
        let (via_reach, same_net_hole, own_hole_reach) = (
            reaches.via_reach,
            reaches.same_net_hole,
            reaches.own_hole_reach,
        );
        let all_layers = board.all_layers();
        let Some((x0, y0, x1, y1)) = grid.node_range(obstacle.shape.aabb().inflated(via_reach))
        else {
            return;
        };
        let Building {
            via_blocked,
            via_owner,
            via_inside,
            via_layers,
            ..
        } = building;
        // A pad no via may overlap (`Board::no_via_pads`: an SMD
        // pad) holds no via of its own net either, but that net's
        // vias may touch it from outside: beside it they need no
        // clearance from their own pad.
        let no_via = board.no_via_pads.binary_search(&obstacle_index).is_ok();
        let beside = obstacle.net.filter(|_| no_via);
        // Another surface pad of a net may hold that net's own vias
        // where the whole via lies inside its copper.
        let own_vias = obstacle.net.filter(|_| {
            !no_via && obstacle.kind == ObstacleKind::Copper && obstacle.layers.count_ones() == 1
        });
        // A routed pad's plated hole keeps other nets' vias at the
        // hole clearance from the drill (KiCad measures a via's
        // copper to the hole: jetson's 0.5 via 0.244 from J2's
        // 0.55 drill in 0.75 copper, 0.25 required). Its own net
        // is exempt, as in KiCad (tracks end on the drill), and
        // keeps the pad copper's rule.
        let own_hole = obstacle.net.filter(|_| obstacle.kind == ObstacleKind::Hole);
        // A region where a pour stays whole (`keep whole <net>
        // <layer>`) keeps other nets' vias out; its net's pass.
        let keep_whole = obstacle.net.filter(|_| {
            obstacle.kind == ObstacleKind::Keepout && obstacle.label.starts_with("keep whole ")
        });
        // A touch electrode's solder mask opening over its own
        // pad (d20_pcb's J2: the opening is the whole die) leaves
        // the pad's net its vias inside the pad, as KiCad does,
        // which bridges only copper of different nets. Other
        // openings keep every via out (an exposed pad's mask
        // windows too: Sisu's TPS63020 went from 23 to 29
        // unconnected with its vias let in there).
        let own_opening = obstacle.net.filter(|_| own_opening_label(obstacle));
        let nodes = ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;
        building.tests += nodes;
        building.vertex_tests += nodes * corners;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let center = grid.center(x, y);
                let distance = distance_at(x, y, center);
                if distance < via_reach {
                    let index = grid.index(x, y);
                    if searched {
                        let (owner, inside) = match (own_vias, own_hole) {
                            // A node an earlier obstacle left to the
                            // net (its pin's `beside` arm) stays the
                            // net's: the map does not depend on
                            // whether a QFN's pins come before its
                            // exposed pad.
                            (Some(net), _)
                                if (!via_blocked[index] || via_owner[index] == owner16(net))
                                    && obstacle
                                        .shape
                                        .contains_disc(center, via_radius + SAFETY) =>
                            {
                                (owner16(net), true)
                            }
                            (_, Some(net))
                                if (!via_blocked[index] || via_owner[index] == owner16(net))
                                    && !(same_net_hole > 0.0 && distance < own_hole_reach) =>
                            {
                                (owner16(net), true)
                            }
                            _ => match (beside, keep_whole) {
                                (Some(net), _)
                                    if (!via_blocked[index]
                                        || via_owner[index] == owner16(net))
                                        && distance >= via_radius + SAFETY =>
                                {
                                    (owner16(net), false)
                                }
                                (None, Some(net))
                                    if !via_blocked[index] || via_owner[index] == owner16(net) =>
                                {
                                    (owner16(net), false)
                                }
                                _ => match own_opening {
                                    Some(net)
                                        if via_blocked[index]
                                            && via_owner[index] == owner16(net)
                                            && via_inside[index] =>
                                    {
                                        (owner16(net), true)
                                    }
                                    _ => (0, false),
                                },
                            },
                        };
                        via_owner[index] = owner;
                        if let Some(flag) = via_inside.get_mut(index) {
                            *flag = inside;
                        }
                    }
                    via_blocked[index] = true;
                    if let Some(layers) = via_layers.get_mut(index) {
                        *layers |= obstacle.layers & all_layers;
                    }
                }
            }
        }
    }

    /// Claims the trace map within the obstacle's reach on each of its
    /// layers for its net (outside the edge zone; with the fill's record
    /// from the first pad apart on) and marks the steps of the nodes just
    /// beyond it that pass too close; returns how many such tight nodes.
    #[allow(clippy::too_many_arguments)]
    fn block_tracks(
        board: &Board,
        grid: &Grid,
        geometry: &ClassGeometry,
        building: &mut Building,
        obstacle: &crate::board::Obstacle,
        reaches: &Reaches,
        corners: u64,
        distance_at: &impl Fn(usize, usize, Point) -> f64,
    ) -> u64 {
        let (half_widths, step) = (&geometry.half_widths, geometry.step);
        let Some((x0, y0, x1, y1)) = grid.node_range(
            obstacle
                .shape
                .aabb()
                .inflated(reaches.trace_reach + reaches.margin),
        ) else {
            return 0;
        };
        let mut tight = 0u64;
        for layer in 0..board.layer_count {
            if obstacle.layers & (1 << layer) == 0 {
                continue;
            }
            let (trace_reach, margin) = match &reaches.ruled_trace {
                None => (reaches.trace_reach, reaches.margin),
                Some(per_layer) => (per_layer[layer], step * step / (8.0 * per_layer[layer])),
            };
            let fill_reach = match reaches.fill_clearance {
                Some(clearance) if !half_widths.is_empty() => {
                    Some(geometry.half_on(layer) + clearance + SAFETY)
                }
                _ => reaches.fill_reach,
            };
            let edge_zone = &building.edge_zone[layer.min(building.edge_zone.len() - 1)];
            let nodes = ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;
            building.tests += nodes;
            building.vertex_tests += nodes * corners;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let index = grid.index(x, y);
                    let center = grid.center(x, y);
                    let distance = distance_at(x, y, center);
                    if distance < trace_reach {
                        if !edge_zone[index] {
                            if let Some(fill) = &mut building.fill_trace {
                                match fill_reach {
                                    Some(reach) if distance < reach => {
                                        fill.claim(layer, index, obstacle.net)
                                    }
                                    _ => fill.remember(
                                        layer,
                                        index,
                                        widen16(building.trace[layer][index]),
                                    ),
                                }
                            }
                            claim16(&mut building.trace[layer][index], obstacle.net);
                        }
                    } else if distance < trace_reach + margin {
                        tight += 1;
                        mark_tight_edges(
                            grid,
                            &obstacle.shape,
                            obstacle.net,
                            trace_reach,
                            index,
                            &mut building.edges[layer],
                        );
                    }
                }
            }
        }
        tight
    }

    /// The finished maps: the fill's record trimmed, a class's other
    /// layers closed, the per-node bits packed.
    fn finish(
        board: &Board,
        grid: &Grid,
        geometry: &ClassGeometry,
        searched: bool,
        building: Building,
    ) -> Self {
        let Building {
            mut trace,
            edges,
            via_blocked,
            via_layers,
            via_owner,
            mut fill_trace,
            tests,
            vertex_tests,
            tight_tests,
            ..
        } = building;
        let cells = grid.cells();
        // Unchanged cells need no record (an opening that claimed nothing).
        if let Some(fill) = &mut fill_trace {
            let values = std::mem::take(&mut fill.values);
            for ((layer, index), value) in values {
                if widen16(trace[layer as usize][index as usize]) == value {
                    fill.differs[layer as usize][index as usize / 64] &= !(1 << (index % 64));
                } else {
                    fill.values.insert((layer, index), value);
                }
            }
            fill.values.shrink_to_fit();
        }
        assert!(
            board.layer_count <= MAX_LAYERS,
            "{} copper layers: free_layers holds 32 (check_capacity)",
            board.layer_count
        );
        // A class kept to some layers (`RuleClass::layers`) has no track
        // on the others.
        let rules = &geometry.rules;
        if rules.layers != 0 {
            for (layer, map) in trace.iter_mut().enumerate() {
                if rules.layers & (1 << layer) == 0 {
                    map.fill(BLOCKED16);
                }
            }
        }
        let mut free_layers = vec![0u32; if searched { cells } else { 0 }];
        let via_blocked = NodeBits::from_bools(&via_blocked);
        let via_layers = LayerBits::new(&via_layers, board.layer_count);
        if searched {
            for (layer, map) in trace.iter().enumerate() {
                for (cell, value) in map.iter().enumerate() {
                    if *value == FREE16 {
                        free_layers[cell] |= 1 << layer;
                    }
                }
            }
        }
        Self {
            trace,
            edges,
            via_blocked,
            via_owner,
            via_layers,
            free_layers: LayerBits::new(&free_layers, board.layer_count),
            fill_trace,
            tests,
            vertex_tests,
            tight_tests,
        }
    }
}

/// Per node, whether it lies inside the board's outline (by each row's
/// crossings of the outline).
fn inside_outline(board: &Board, grid: &Grid) -> Vec<bool> {
    let mut inside = vec![false; grid.cells()];
    for y in 0..grid.ny {
        let line = grid.coordinate(1, y as i64);
        let mut crossings: Vec<f64> = polygon_edges(&board.outline)
            .filter(|(a, b)| (a[1] > line) != (b[1] > line))
            .map(|(a, b)| a[0] + (line - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
            .collect();
        crossings.sort_by(f64::total_cmp);
        for span in crossings.as_chunks::<2>().0 {
            if let Some(warp) = &grid.warp {
                let lines = &warp.lines[0];
                let x0 = lines.partition_point(|line| *line < span[0]);
                let x1 = lines.partition_point(|line| *line <= span[1]);
                for x in x0..x1 {
                    inside[grid.index(x, y)] = true;
                }
                continue;
            }
            let x0 = ((span[0] - grid.origin[0]) / grid.pitch).ceil().max(0.0) as usize;
            let x1 = ((span[1] - grid.origin[0]) / grid.pitch).floor();
            if x1 < 0.0 {
                continue;
            }
            for x in x0..=(x1 as usize).min(grid.nx - 1) {
                inside[grid.index(x, y)] = true;
            }
        }
    }
    inside
}

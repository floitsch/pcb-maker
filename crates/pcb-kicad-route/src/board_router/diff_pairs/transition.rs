// Copyright (C) 2026 Florian Loitsch.

//! A paired layer change near two surface pads and a coupled run to two
//! plated through-hole pads on the opposite outer copper layer.

use super::plan_link::{Chosen, PairSite};
use super::*;
use pcb_router as core;

/// Clearance-tested straight run between the stubs and the search terminal.
const LEAD: f64 = 0.85;
const CONNECTOR_STEP: f64 = 0.10;
const CONNECTOR_STEPS: i32 = 45;

pub(super) fn choose(
    board: &core::Board,
    site: &PairSite,
    ends: &[(core::Terminal, core::Terminal); 2],
    plated_gap: bool,
) -> Option<Chosen> {
    let mut pair = site.pair.clone();
    pair.clearance = [pair.positive, pair.negative]
        .into_iter()
        .map(|id| board.classes[board.nets[id as usize].class].clearance)
        .fold(0.0, f64::max);
    let pair = &pair;
    if pair.links.len() != 1 || board.layer_count < 2 {
        return None;
    }
    let front = 1;
    let back = 1 << (board.layer_count - 1);
    let surface_end = ends.iter().position(|(p, n)| {
        p.layers == n.layers
            && [front, back].contains(&p.layers)
            && !board.obstacles[p.pad].plated
            && !board.obstacles[n.pad].plated
    })?;
    let (p, n) = &ends[surface_end];
    let (far_p, far_n) = &ends[1 - surface_end];
    let layer = if p.layers == front { back } else { front };
    if (far_p.layers & far_n.layers & pair.layers & layer) == 0
        || p.layers & pair.layers == 0
        || !board.obstacles[far_p.pad].plated
        || !board.obstacles[far_n.pad].plated
    {
        return None;
    }
    let midpoint = |p: &core::Terminal, n: &core::Terminal| scale(add(p.anchor, n.anchor), 0.5);
    let middle = midpoint(p, n);
    let toward = midpoint(far_p, far_n);
    let class = core::RuleClass {
        clearance: pair.clearance,
        layers: layer,
        ..site.envelope
    };
    let mut outward = left(unit(sub(n.anchor, p.anchor)));
    if dot(outward, sub(toward, middle)) < 0.0 {
        outward = scale(outward, -1.0);
    }
    let local_reach = 8.0
        + site
            .clearance
            .map_or(pair.clearance, core::rules::Resolved::largest);
    let near = obstacles_near(board, middle, local_reach);
    let far_near = obstacles_near(board, toward, local_reach);
    // With a shared outer layer, fixed plated approaches can avoid a
    // crossover and its additional independently routed stubs.
    // A plated-pad retry first keeps the existing outer-layer assignment.
    let planar = || {
        let layer = p.layers & pair.layers & far_p.layers & far_n.layers;
        if layer.count_ones() != 1 {
            return None;
        }
        let class = core::RuleClass {
            layers: layer,
            ..class
        };
        let surface = launch(
            board,
            pair,
            &class,
            p,
            n,
            toward,
            true,
            Lead::Straight,
            &site.neighbours,
        )
        .filter(|launch| launch.fixed)?;
        let connector = EndSite {
            board,
            pair,
            envelope: &class,
            clearance: site.clearance,
            near: &far_near,
            pads: [far_p, far_n],
            center: toward,
            outward: unit(sub(surface.start, toward)),
            side: -surface.side,
            layer,
            plated_gap,
        };
        let far = connector.connector_launch()?;
        let launches = if surface_end == 0 {
            [surface, far]
        } else {
            [far, surface]
        };
        Some(Chosen {
            side: launches[0].side,
            launches,
            crossed: false,
            run_class: Some(class),
        })
    };
    if plated_gap && let Some(chosen) = planar() {
        return Some(chosen);
    }
    for angle in [0.0, 15.0, -15.0, 30.0, -30.0, 45.0, -45.0] {
        let outward = turned(outward, angle);
        let side = if dot(left(outward), sub(p.anchor, n.anchor)) > 0.0 {
            1.0
        } else {
            -1.0
        };
        let at = EndSite {
            board,
            pair,
            envelope: &class,
            clearance: site.clearance,
            near: &near,
            pads: [p, n],
            center: middle,
            outward,
            side,
            layer,
            plated_gap: false,
        };
        if let Some(surface) = (5..=40).find_map(|step| at.surface_at(step as f64 * 0.10)) {
            let connector = EndSite {
                board,
                pair,
                envelope: &class,
                clearance: site.clearance,
                near: &far_near,
                pads: [far_p, far_n],
                center: toward,
                outward: unit(sub(surface.start, toward)),
                side: -side,
                layer,
                plated_gap,
            };
            let Some(far) = connector.connector_launch() else {
                continue;
            };
            if debug() {
                eprintln!(
                    "paired layer transition {} / {} on layer {}: {:?}",
                    p.label,
                    n.label,
                    layer.trailing_zeros(),
                    surface.crossover
                );
            }
            let launches = if surface_end == 0 {
                [surface, far]
            } else {
                [far, surface]
            };
            return Some(Chosen {
                side: launches[0].side,
                launches,
                crossed: false,
                run_class: Some(class),
            });
        }
    }
    planar()
}

struct EndSite<'a> {
    board: &'a core::Board,
    pair: &'a Pair,
    envelope: &'a core::RuleClass,
    clearance: Option<&'a core::rules::Resolved>,
    near: &'a [usize],
    pads: [&'a core::Terminal; 2],
    center: Point,
    outward: Point,
    side: f64,
    layer: core::LayerMask,
    plated_gap: bool,
}

impl EndSite<'_> {
    /// Find a short, balanced pair of pad stubs with a clear run past them.
    fn connector_launch(&self) -> Option<Launch> {
        let mut best: Option<(f64, Launch)> = None;
        for x in -CONNECTOR_STEPS..=CONNECTOR_STEPS {
            for y in -CONNECTOR_STEPS..=CONNECTOR_STEPS {
                let start = add(
                    self.center,
                    [x as f64 * CONNECTOR_STEP, y as f64 * CONNECTOR_STEP],
                );
                for turn in 0..8 {
                    let along = turned([1.0, 0.0], f64::from(turn) * 45.0);
                    let forward = dot(along, self.outward);
                    if forward < -0.15 {
                        continue;
                    }
                    if self
                        .pads
                        .iter()
                        .any(|pad| dot(along, sub(pad.anchor, start)) > 0.0)
                    {
                        continue;
                    }
                    if let Some((cost, launch)) = self.connector_at(start, along) {
                        let score = cost + (1.0 - forward);
                        if best.as_ref().is_none_or(|(known, _)| score < *known) {
                            best = Some((score, launch));
                        }
                    }
                }
            }
        }
        best.map(|(_, launch)| launch)
    }

    fn connector_at(&self, start: Point, outward: Point) -> Option<(f64, Launch)> {
        let (pair, layer) = (self.pair, self.layer);
        let at = add(start, scale(outward, LEAD));
        if !self.lead_clear(start, at) {
            return None;
        }
        let pads = self.pads.map(|pad| pad.anchor);
        let ends = [self.side, -self.side]
            .map(|sign| add(start, scale(left(outward), sign * pair.offset())));
        let paths = |polarity: usize| {
            let (from, to) = (pads[polarity], ends[polarity]);
            let [x, y] = sub(to, from);
            let diag = x.abs().min(y.abs());
            let diagonal = [diag.copysign(x), diag.copysign(y)];
            let possibilities = [
                from,
                [from[0], to[1]],
                [to[0], from[1]],
                add(from, diagonal),
                sub(to, diagonal),
            ];
            let straight = sub(sub(to, from), diagonal);
            let by = |ratio| add(from, scale(straight, ratio));
            // A compact escape can join the run with two good clearances
            // but unequal leg lengths. Also try a dogleg on either side
            // of the chord, and compare both conductors together below.
            let chord = sub(to, from);
            let normal = left(unit(chord));
            let doglegs = [0.45, -0.45, 0.75, -0.75, 1.05, -1.05, 1.35, -1.35]
                .into_iter()
                .filter(|_| self.plated_gap)
                .map(|depth| {
                    let jog = scale(normal, depth);
                    let inset = scale(chord, 0.20);
                    [
                        from,
                        add(add(from, inset), jog),
                        add(sub(to, inset), jog),
                        to,
                    ]
                });
            possibilities
                .into_iter()
                .map(|bend| [from, bend, bend, to])
                .chain(
                    [0.25, 0.5, 0.75].map(|ratio| [from, by(ratio), add(by(ratio), diagonal), to]),
                )
                .chain(doglegs)
                .filter_map(|points| {
                    let index = layer.trailing_zeros() as usize;
                    let segments: Vec<_> = points
                        .windows(2)
                        .filter(|step| core::geometry::distance(step[0], step[1]) > EPSILON)
                        .map(|step| (index, step[0], step[1]))
                        .collect();
                    (!segments.is_empty()
                        && segments.iter().all(|&(_, a, b)| {
                            self.track_clear((a, b), Some(polarity), layer, pair.width / 2.0)
                        }))
                    .then_some(segments)
                })
                .collect::<Vec<_>>()
        };
        let [p_paths, n_paths] = [paths(0), paths(1)];
        let leads = ends.map(|end| add(end, scale(outward, LEAD)));
        let mut best: Option<(f64, Crossover)> = None;
        for p in &p_paths {
            for n in &n_paths {
                let segments = [p.clone(), n.clone()];
                let lengths = segments.each_ref().map(|path| {
                    path.iter()
                        .map(|&(_, a, b)| core::geometry::distance(a, b))
                        .sum::<f64>()
                });
                let length = lengths[0] + lengths[1] + 2.0 * (lengths[0] - lengths[1]).abs();
                let mut copper = LinkCopper {
                    segments: segments.clone(),
                    ..LinkCopper::default()
                };
                for (segments, (end, lead)) in
                    copper.segments.iter_mut().zip(ends.into_iter().zip(leads))
                {
                    segments.push((layer.trailing_zeros() as usize, end, lead));
                }
                if best.as_ref().is_none_or(|(cost, _)| length < *cost)
                    && own_copper_clear(&copper, pair)
                {
                    best = Some((
                        length,
                        Crossover {
                            run_start: start,
                            segments,
                            vias: Default::default(),
                        },
                    ));
                }
            }
        }
        let (length, lead) = best?;
        Some((
            length,
            Launch {
                at,
                start,
                reserved: start,
                ends: scale(outward, pair.width),
                outward,
                layers: self.layer,
                side: self.side,
                fixed: true,
                stubs: pads,
                stub_ends: pads,
                crossover: Some(lead),
                room: f64::INFINITY,
            },
        ))
    }

    fn lead_clear(&self, start: Point, at: Point) -> bool {
        let half = self.envelope.trace_width / 2.0;
        self.track_clear((at, at), None, self.layer, half + LAUNCH_MARGIN)
            && self.track_clear((start, at), None, self.layer, half)
    }

    fn track_clear(
        &self,
        (start, at): (Point, Point),
        polarity: Option<usize>,
        layer: core::LayerMask,
        half: f64,
    ) -> bool {
        tested(self.board.outline.len() + self.near.len());
        let own = polarity.map(|index| [self.pair.positive, self.pair.negative][index]);
        core::geometry::point_in_polygon(at, &self.board.outline)
            && core::geometry::polygon_edges(&self.board.outline).all(|(a, b)| {
                core::geometry::segment_segment_distance(start, at, a, b)
                    >= half + self.board.edge_clearance
            })
            && self.near.iter().all(|&index| {
                let obstacle = &self.board.obstacles[index];
                obstacle.layers & layer == 0
                    || own.is_some() && obstacle.net == own
                    || obstacle.kind == core::ObstacleKind::Keepout
                        && obstacle.label.starts_with("solder mask opening")
                        && obstacle.shape.contains(start)
                        && own.is_some()
                    || obstacle.shape.distance_to_segment(start, at)
                        >= half + self.keep(obstacle, layer, polarity, core::rules::TRACK) - EPSILON
            })
    }

    fn keep(
        &self,
        obstacle: &core::Obstacle,
        layers: core::LayerMask,
        polarity: Option<usize>,
        kind: usize,
    ) -> f64 {
        let mut base = keep_from(self.board, obstacle, self.envelope.clearance);
        if self.plated_gap
            && kind == core::rules::TRACK
            && core::rules::obstacle_kind(obstacle) == Some(core::rules::PLATED)
            && polarity.is_some_and(|own| {
                obstacle.net == Some([self.pair.negative, self.pair.positive][own])
            })
        {
            base = self
                .board
                .pair_copper_clearance(self.envelope, obstacle, self.pair.gap);
        }
        let (Some(rules), Some(other_kind)) =
            (self.clearance, core::rules::obstacle_kind(obstacle))
        else {
            return base;
        };
        if obstacle
            .net
            .is_some_and(|id| id as usize >= rules.net_group.len())
        {
            return base.max(self.pair.clearance);
        }
        let other = rules.obstacle_group(obstacle);
        let mut keep = base;
        for (index, id) in [self.pair.positive, self.pair.negative]
            .into_iter()
            .enumerate()
        {
            if polarity.is_some_and(|only| only != index) {
                continue;
            }
            for layer in 0..self.board.layer_count {
                if layers & obstacle.layers & (1 << layer) == 0 {
                    continue;
                }
                keep = keep.max(rules.extra_on(
                    rules.net_group[id as usize],
                    kind,
                    other,
                    other_kind,
                    layer,
                ));
            }
        }
        keep
    }

    fn surface_at(&self, reach: f64) -> Option<Launch> {
        let pair = self.pair;
        let [p, n] = self.pads;
        let vias = self
            .pads
            .map(|pad| add(pad.anchor, scale(self.outward, reach)));
        if !vias
            .iter()
            .enumerate()
            .all(|(polarity, &via)| self.via_clear(via, polarity))
        {
            return None;
        }
        let pads = [p.anchor, n.anchor];
        if !(0..2).all(|polarity| {
            self.track_clear(
                (pads[polarity], vias[polarity]),
                Some(polarity),
                p.layers,
                pair.width / 2.0,
            )
        }) {
            return None;
        }
        let pad_ends = pads.map(|at| add(at, scale(self.outward, 0.05)));
        if !self
            .pads
            .iter()
            .zip(pad_ends)
            .all(|(pad, at)| self.board.obstacles[pad.pad].shape.contains(at))
        {
            return None;
        }
        let start = add(self.center, scale(self.outward, reach + 0.65));
        let at = add(start, scale(self.outward, LEAD));
        let end_layers = [
            p.layers.trailing_zeros() as usize,
            self.layer.trailing_zeros() as usize,
        ];
        let ends = [self.side, -self.side]
            .map(|sign| add(start, scale(left(self.outward), sign * pair.offset())));
        if !vias
            .iter()
            .zip(ends)
            .enumerate()
            .all(|(polarity, (&via, end))| {
                self.track_clear((via, end), Some(polarity), self.layer, pair.width / 2.0)
            })
        {
            return None;
        }
        if !self.lead_clear(start, at) {
            return None;
        }
        let segments = [0, 1].map(|polarity| {
            vec![
                (end_layers[0], pad_ends[polarity], vias[polarity]),
                (end_layers[1], vias[polarity], ends[polarity]),
            ]
        });
        let vias = vias.map(|via| vec![via]);
        if !own_copper_clear(
            &LinkCopper {
                segments: segments.clone(),
                vias: vias.clone(),
                ..Default::default()
            },
            pair,
        ) {
            return None;
        }
        Some(Launch {
            at,
            start,
            reserved: start,
            ends: scale(self.outward, pair.width),
            outward: self.outward,
            layers: self.layer,
            side: self.side,
            fixed: true,
            stubs: pads,
            stub_ends: pad_ends,
            crossover: Some(Crossover {
                run_start: start,
                segments,
                vias,
            }),
            room: f64::INFINITY,
        })
    }

    fn via_clear(&self, at: Point, polarity: usize) -> bool {
        let pair = self.pair;
        let board = self.board;
        let radius = pair.via_diameter / 2.0;
        let drill = pair.via_drill / 2.0;
        let net = [pair.positive, pair.negative][polarity];
        let partner = [pair.negative, pair.positive][polarity];
        if !core::geometry::point_in_polygon(at, &board.outline)
            || core::geometry::polygon_edges(&board.outline).any(|(a, b)| {
                core::geometry::point_segment_distance(at, a, b) < radius + board.edge_clearance
            })
        {
            return false;
        }
        tested(board.obstacles.len());
        board.obstacles.iter().enumerate().all(|(index, obstacle)| {
            if !obstacle.blocks_vias {
                return true;
            }
            let need = match obstacle.kind {
                core::ObstacleKind::Copper if obstacle.net == Some(net) => {
                    if board.no_via_pads.binary_search(&index).is_ok() {
                        radius
                    } else {
                        0.0
                    }
                }
                core::ObstacleKind::Copper => {
                    let clearance = if obstacle.net == Some(partner)
                        && matches!(
                            core::rules::obstacle_kind(obstacle),
                            Some(core::rules::TRACK | core::rules::VIA)
                        ) {
                        pair.gap
                    } else {
                        self.keep(
                            obstacle,
                            board.all_layers(),
                            Some(polarity),
                            core::rules::VIA,
                        )
                    };
                    (radius + clearance).max(drill + board.hole_clearance)
                }
                core::ObstacleKind::Hole => drill + board.hole_to_hole.max(obstacle.clearance),
                core::ObstacleKind::Keepout => radius + obstacle.clearance,
            };
            need <= EPSILON || obstacle.shape.distance_to_point(at) >= need - EPSILON
        })
    }
}

// Copyright (C) 2026 Toit contributors.

//! Prepared fan-outs for fine-pitch rows (docs/router.md, "Prepared
//! fan-outs for fine-pitch rows (2026-11-07)"; router settings
//! `row_fanout`, `PCB_ROUTER_ROW_FANOUT`).
//!
//! A pad of a 0.4 to 0.5 mm row leaves between its neighbours' tips: the
//! search finds its way out per net, in negotiation order, on a lattice
//! whose phase the row does not share, and on the designer tier's boards
//! 69 of the open items were such pads (docs/router-ceiling.md, design 2):
//! no straight way out at an exact fit (0.2 mm track and clearance at
//! 0.4 mm), every straight way taken by a neighbour's via or track, or a
//! crossing pair (two neighbours whose targets lie on the other's side).
//!
//! The plan (`plan`), a function of the lowered board and the settings
//! alone, like the BGA fan-out's (board_router/bga_fanout.rs): per row of a
//! part (SMD pads on one layer whose long axes point out of one side,
//! adjacent pads closer than `FINE_PITCH`), every pad of a signal net gets
//! its exit at exact geometry, jointly with its neighbours: a stub from the
//! pad's centre straight out beyond the row's tips to a landing where the
//! net's track is legal with room for any lattice node near it (`RHO`).
//! Where the pitch is narrower than two tracks and a clearance with that
//! room, the exits fan: from the row's middle out, each pad's stub turns
//! by 45 degrees away from the middle by as much as the spacing needs, the
//! outer pads first, so that every landing keeps its neighbours' copper at
//! the clearance plus the lattice's room. Pour nets' pads keep a straight
//! corridor of their own (checked, not copper): their spokes leave there.
//!
//! The order: along the row, the pads' targets (the nearest pad of the net
//! on another part, as an angle from the part's centre) against the pads'
//! order; the longest run of pads whose targets keep the order stays on the
//! layer, the others dive: their stub reaches beyond the neighbours'
//! landings to a via (the largest of the class's and the project's sizes
//! that keeps every clearance on every layer), taken only where a straight
//! run of the net's track leaves it on another layer it may route on (its
//! landing, checked before). A row any of whose pads the plan cannot serve
//! (no clear exit, no landing, no via site) is left to the search whole:
//! no partial plan (the removed fan-out step's reservations moved which pad
//! of a crossing pair stayed open).
//!
//! The lowering makes each planned exit fixed copper of the pad's net
//! (`<pad> row exit` segments, a `<pad> row landing` disc at its end, which
//! the pad's terminal reaches: `Board::terminal_copper`; a diver's via as
//! the BGA dog-bone's, its ring the terminal on the other layers).
//! `emit_routes` writes an exit only where the net's route uses it (a
//! route segment ends at the landing, or the via is used off the pad's
//! layer), joining the route's end to the landing where KiCad would not
//! see them touch; an exit the routes did not take is dropped, its room
//! was only kept from other nets.

use super::*;
use pcb_router as core;

/// `PCB_ROUTER_ROW_FANOUT` when set: overrides `row_fanout` (`0` or empty
/// off, anything else on).
pub fn row_fanout_override() -> Option<bool> {
    let value = std::env::var("PCB_ROUTER_ROW_FANOUT").ok()?;
    Some(!(value.is_empty() || value == "0"))
}

/// Whether the lowering plans row fan-outs: the override, else the
/// setting, else off.
pub(crate) fn row_fanout_on(config: &KiCadBoardRouterConfig) -> bool {
    row_fanout_override().unwrap_or(config.row_fanout.unwrap_or(false))
}

/// Whether divers take part (`PCB_ROUTER_ROW_FANOUT_DIVE`, on unless `0`):
/// an experiment's switch, rows planned on the surface alone without it.
fn dives_on() -> bool {
    std::env::var("PCB_ROUTER_ROW_FANOUT_DIVE").map_or(true, |value| value != "0")
}

/// Adjacent pads closer than this (centre to centre) make a row
/// fine-pitch (router/fanout.rs `FINE_PITCH`, the census's).
const FINE_PITCH: f64 = 0.65;
/// Adjacent pads up to this many times `FINE_PITCH` apart stay in one row
/// (a 0.65 mm part's pads, centres a nanometre either side of it, are one
/// row, and not a fine-pitch one).
const ROW_SLACK: f64 = 1.05;
/// A part with fewer SMD pads has no row.
const MIN_PART_PADS: usize = 4;
/// The room a landing keeps beyond its track for the lattice node the
/// search starts from: half the diagonal of the coarsest default lattice
/// (0.127 mm), so some node within it lies inside the landing's legal disc
/// whatever the lattice's phase.
const RHO: f64 = 0.09;
/// How far an exit may come short of a clearance: floating point noise
/// (the router's `STUB_FIT`): an exact fit passes, as in KiCad.
const FIT: f64 = 1.0e-6;
/// What a planned via keeps beyond the rule (the BGA plan's).
const SAFETY: f64 = 1.0e-5;
/// The landing's last straight piece beyond a turn (mm).
const LAND: f64 = 0.05;
/// How far beyond the row's tips an exit reaches at least (mm): the way
/// out it keeps from the neighbours' vias and turns (the census's ways
/// out were taken within about this of the tips).
const MIN_EXIT: f64 = 0.25;
/// How far beyond the row's tips the turns may lie (mm), and the step.
const REACH: f64 = 1.5;
const STEP: f64 = 0.02;
/// How far beyond its landing a diver's via may lie, and the step (mm).
const VIA_REACH: f64 = 2.0;
const VIA_STEP: f64 = 0.05;
/// The straight run of the net's track that must leave a diver's via on
/// another layer, beyond the via's ring (mm).
const LANDING_RUN: f64 = 0.5;

/// A footprint's SMD pads as the lowering made them (`lower_board` collects
/// them with the plan on).
pub(crate) struct RowPart {
    pub reference: String,
    /// The footprint's rotation (KiCad's degrees).
    pub angle: f64,
    /// Its SMD pads' copper obstacles, the exposed pad left out.
    pub pads: Vec<usize>,
}

/// A diver's via.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RowVia {
    pub at: [f64; 2],
    pub diameter: f64,
    pub drill: f64,
    /// Its span where it is not a through via.
    pub span: Option<core::ViaSpan>,
}

/// One planned exit: the path from the pad's centre to its landing (or its
/// via), at `width`, on the pad's layer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlannedExit {
    pub net: core::NetId,
    /// The pad's copper obstacle.
    pub pad: usize,
    pub label: String,
    pub layer: usize,
    pub path: Vec<[f64; 2]>,
    pub width: f64,
    pub via: Option<RowVia>,
}

/// A row's plan, for the report (`PCB_ROUTER_ROW_FANOUT_WHY`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RowSummary {
    pub reference: String,
    pub layer: usize,
    /// The pads' labels, in the row's order.
    pub pads: Vec<String>,
    pub pitch: f64,
    /// Pads that get an exit (signal nets).
    pub planned: usize,
    /// Whether the exits fan (the pitch narrower than the landings need).
    pub fan: bool,
    pub divers: Vec<String>,
    /// Divers no via site serves: they stay on the surface.
    pub stayed: Vec<String>,
    /// Why the row was left to the search.
    pub refused: Option<String>,
}

/// The plan: the exits, the rows' summaries, the work (distance tests and
/// obstacles visited, about a lowering's expression each).
#[derive(Default)]
pub(crate) struct RowPlan {
    pub exits: Vec<PlannedExit>,
    pub rows: Vec<RowSummary>,
    pub work: u64,
}

/// Planned copper (and landing zones, checked against copper only).
#[derive(Clone, Debug)]
struct Planned {
    net: core::NetId,
    layers: core::LayerMask,
    shape: core::Shape,
    /// A landing's legal disc: no copper; another net's copper keeps its
    /// clearance from it, another zone does not.
    zone: bool,
    /// A via's drill radius (holes keep the hole rules).
    hole: Option<f64>,
}

/// The legality tests' context.
struct Context<'a> {
    obstacles: &'a [core::Obstacle],
    nets: &'a [core::Net],
    classes: &'a [core::RuleClass],
    /// Sorted: SMD pads no via may overlap.
    no_via_pads: &'a [usize],
    hole_clearance: f64,
    hole_to_hole: f64,
    outline: &'a [[f64; 2]],
    edge_clearance: f64,
}

impl Context<'_> {
    fn clearance_of(&self, net: core::NetId) -> f64 {
        self.classes[self.nets[net as usize].class].clearance
    }

    /// `core::Board::copper_clearance` before the board exists.
    fn clearance(&self, net: core::NetId, obstacle: &core::Obstacle) -> f64 {
        let other = obstacle.clearance_override.unwrap_or_else(|| {
            let class = obstacle.net.map_or(0.0, |other| self.clearance_of(other));
            obstacle.clearance.max(class)
        });
        self.clearance_of(net).max(other)
    }

    /// Whether a track of `net` from `a` to `b`, `width` wide, on `layer`
    /// keeps every rule against the obstacles `near` (indices) and the
    /// board's edge. `start`: the exit's start (the pad's own mask
    /// opening is passed, as the escape stubs pass it).
    #[allow(clippy::too_many_arguments)]
    fn track_clear(
        &self,
        net: core::NetId,
        layer: usize,
        a: [f64; 2],
        b: [f64; 2],
        width: f64,
        start: [f64; 2],
        near: &[usize],
        work: &mut u64,
    ) -> bool {
        *work += near.len() as u64;
        let half = width / 2.0;
        near.iter().all(|&index| {
            let obstacle = &self.obstacles[index];
            if obstacle.net == Some(net)
                || !obstacle.blocks_tracks
                || obstacle.layers & (1 << layer) == 0
            {
                return true;
            }
            if obstacle.kind == core::ObstacleKind::Keepout
                && obstacle.label.starts_with("solder mask opening")
                && obstacle.shape.contains(start)
            {
                return true;
            }
            let required = half - FIT
                + match obstacle.kind {
                    core::ObstacleKind::Copper => self.clearance(net, obstacle),
                    core::ObstacleKind::Keepout => obstacle.clearance,
                    core::ObstacleKind::Hole => self.hole_clearance.max(obstacle.clearance),
                };
            obstacle.shape.distance_to_segment(a, b) >= required
        }) && self.keeps_edge(a, b, half)
    }

    /// The first of `near` that a track of `net` from `a` to `b` meets (for
    /// the report), or the edge.
    #[allow(clippy::too_many_arguments)]
    fn blocker(
        &self,
        net: core::NetId,
        layer: usize,
        a: [f64; 2],
        b: [f64; 2],
        width: f64,
        start: [f64; 2],
        near: &[usize],
    ) -> String {
        let mut work = 0;
        near.iter()
            .find(|&&index| !self.track_clear(net, layer, a, b, width, start, &[index], &mut work))
            .map_or_else(
                || "the board's edge".to_string(),
                |&index| self.obstacles[index].label.clone(),
            )
    }

    /// Whether a segment `half` wide lies inside the outline and keeps the
    /// edge clearance (no outline: anywhere).
    fn keeps_edge(&self, a: [f64; 2], b: [f64; 2], half: f64) -> bool {
        if self.outline.len() < 3 {
            return true;
        }
        if !core::geometry::point_in_polygon(a, self.outline)
            || !core::geometry::point_in_polygon(b, self.outline)
        {
            return false;
        }
        let required = half + self.edge_clearance - FIT;
        (0..self.outline.len()).all(|index| {
            let c = self.outline[index];
            let d = self.outline[(index + 1) % self.outline.len()];
            core::geometry::segment_segment_distance(a, b, c, d) >= required
        })
    }

    /// Whether a via of `net` at `at` (radius, drill radius, its layers)
    /// keeps every rule against the obstacles `near` (the BGA plan's test).
    #[allow(clippy::too_many_arguments)]
    fn via_clear(
        &self,
        net: core::NetId,
        at: [f64; 2],
        radius: f64,
        hole: f64,
        layers: core::LayerMask,
        near: &[usize],
        work: &mut u64,
    ) -> bool {
        *work += near.len() as u64;
        near.iter().all(|&index| {
            let obstacle = &self.obstacles[index];
            if !obstacle.blocks_vias
                || (layers != core::LayerMask::MAX && obstacle.layers & layers == 0)
            {
                return true;
            }
            let gap = obstacle.shape.distance_to_point(at);
            match obstacle.kind {
                core::ObstacleKind::Hole => {
                    gap >= hole + self.hole_to_hole + SAFETY
                        && (obstacle.net == Some(net)
                            || gap >= radius + self.hole_clearance + SAFETY)
                }
                core::ObstacleKind::Keepout => gap >= radius + obstacle.clearance + SAFETY,
                core::ObstacleKind::Copper if obstacle.net == Some(net) => {
                    self.no_via_pads.binary_search(&index).is_err() || gap >= radius + SAFETY
                }
                core::ObstacleKind::Copper => {
                    gap >= radius + self.clearance(net, obstacle) + SAFETY
                        && gap >= hole + self.hole_clearance + SAFETY
                }
            }
        }) && self.keeps_edge(at, at, radius)
    }

    /// Whether `candidate` (copper or a zone of `net`) keeps its rules
    /// against the planned copper of other nets (`planned`).
    fn clear_of_planned(
        &self,
        net: core::NetId,
        candidate: &Planned,
        planned: &[Planned],
        work: &mut u64,
    ) -> bool {
        *work += planned.len() as u64;
        planned.iter().all(|other| {
            if other.layers & candidate.layers == 0 {
                return true;
            }
            let distance = core::verify::shape_distance(&candidate.shape, &other.shape);
            // Holes: the hole-to-hole spacing whatever the nets.
            if let (Some(a), Some(b)) = (candidate.hole, other.hole) {
                let centres = |shape: &core::Shape| match shape {
                    core::Shape::Circle { center, .. } => *center,
                    _ => [f64::NAN; 2],
                };
                let apart =
                    core::geometry::distance(centres(&candidate.shape), centres(&other.shape));
                if apart < a + b + self.hole_to_hole + SAFETY {
                    return false;
                }
            }
            if other.net == net || (candidate.zone && other.zone) {
                return true;
            }
            let clearance = self.clearance_of(net).max(self.clearance_of(other.net));
            distance >= clearance - FIT
        })
    }
}

/// One pad of a part, in the part's frame.
#[derive(Clone, Debug)]
struct RowPad {
    /// The pad's copper obstacle.
    obstacle: usize,
    centre: [f64; 2],
    layer: usize,
    /// Its outward axis (an index of `SIDES`).
    side: usize,
    /// Its coordinates in the side's frame: `u` outward, `v` along the
    /// row; and its half extent outward.
    u: f64,
    v: f64,
    reach: f64,
}

/// The four sides of a part's frame: (axis, sign).
const SIDES: [(usize, f64); 4] = [(0, 1.0), (1, 1.0), (0, -1.0), (1, -1.0)];

/// What a pad of a row takes from the plan.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    /// A signal net's pad: an exit of copper.
    Signal,
    /// A pour net's pad: a straight corridor kept from the others.
    Corridor,
    /// Nothing (no net, a net of one pad, a via in the pad).
    Skip,
}

/// The longest strictly increasing subsequence of `values`: their indices.
fn longest_increasing(values: &[f64]) -> Vec<usize> {
    // Patience sorting with predecessors.
    let mut tails: Vec<usize> = Vec::new();
    let mut previous = vec![usize::MAX; values.len()];
    for (index, value) in values.iter().enumerate() {
        let position = tails.partition_point(|&tail| values[tail] < *value);
        if position > 0 {
            previous[index] = tails[position - 1];
        }
        if position == tails.len() {
            tails.push(index);
        } else {
            tails[position] = index;
        }
    }
    let mut chain = Vec::new();
    let mut at = tails.last().copied().unwrap_or(usize::MAX);
    while at != usize::MAX {
        chain.push(at);
        at = previous[at];
    }
    chain.reverse();
    chain
}

/// Where a pad's net leads: the nearest pad of the net on another part,
/// else none (a net whose other pads are all the part's own leaves inward
/// or along the row, not out of it).
fn target_of(nets: &[core::Net], net: core::NetId, pad: usize, part: &str) -> Option<[f64; 2]> {
    let description = &nets[net as usize];
    let at = description
        .terminals
        .iter()
        .find(|terminal| terminal.pad == pad)?
        .anchor;
    description
        .terminals
        .iter()
        .filter(|terminal| terminal.pad != pad)
        .filter(|terminal| {
            terminal
                .label
                .rsplit_once('.')
                .map_or(terminal.label.as_str(), |(part, _)| part)
                != part
        })
        .map(|terminal| terminal.anchor)
        .min_by(|a, b| {
            core::geometry::distance(*a, at).total_cmp(&core::geometry::distance(*b, at))
        })
}

/// The plan of the lowered board: per fine-pitch row of `parts`, its exits
/// (see the module's comment). `holding`: pads that hold a via already;
/// `planes`: the pours (their nets' pads keep corridors); `spans`: the via
/// spans the board allows.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan(
    parts: &[RowPart],
    obstacles: &[core::Obstacle],
    nets: &[core::Net],
    classes: &[core::RuleClass],
    no_via_pads: &[usize],
    holding: &BTreeSet<usize>,
    spans: (&[core::ViaSpan], &[core::Plane]),
    outline: &[[f64; 2]],
    layer_count: usize,
    config: &KiCadBoardRouterConfig,
) -> RowPlan {
    let (spans, planes) = spans;
    let mut sorted_no_via = no_via_pads.to_vec();
    sorted_no_via.sort_unstable();
    let context = Context {
        obstacles,
        nets,
        classes,
        no_via_pads: &sorted_no_via,
        hole_clearance: config.hole_clearance_mm,
        hole_to_hole: config.hole_to_hole_clearance_mm,
        outline,
        edge_clearance: config.edge_clearance_mm + outline::ARC_TOLERANCE,
    };
    let neck = config
        .neck_width_mm
        .unwrap_or_else(|| {
            classes
                .iter()
                .map(|class| class.trace_width)
                .fold(f64::INFINITY, f64::min)
        })
        .max(config.width_floor());
    let poured: BTreeSet<core::NetId> = planes.iter().map(|plane| plane.net).collect();
    let names: BTreeMap<&str, usize> = nets
        .iter()
        .enumerate()
        .map(|(index, net)| (net.name.as_str(), index))
        .collect();
    let paired = |net: core::NetId| {
        crate::project_rules::diff_pair_of(&nets[net as usize].name)
            .is_some_and(|(partner, _, _)| names.contains_key(partner.as_str()))
    };
    let dives = dives_on();
    let mut plan = RowPlan::default();
    // The planned copper of every row so far (other rows keep clear of it).
    let mut planned: Vec<Planned> = Vec::new();
    for part in parts {
        if part.pads.len() < MIN_PART_PADS {
            continue;
        }
        let axes = [
            crate::geometry::rotate_vector([1.0, 0.0], -part.angle),
            crate::geometry::rotate_vector([0.0, 1.0], -part.angle),
        ];
        let centres: Vec<[f64; 2]> = part
            .pads
            .iter()
            .map(|&index| {
                let bounds = obstacles[index].shape.aabb();
                [
                    (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                    (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
                ]
            })
            .collect();
        let n = centres.len() as f64;
        let middle = [
            centres.iter().map(|c| c[0]).sum::<f64>() / n,
            centres.iter().map(|c| c[1]).sum::<f64>() / n,
        ];
        let local = |point: [f64; 2]| {
            let d = [point[0] - middle[0], point[1] - middle[1]];
            [
                d[0] * axes[0][0] + d[1] * axes[0][1],
                d[0] * axes[1][0] + d[1] * axes[1][1],
            ]
        };
        // The box of the pads' centres in the frame (the sides of a
        // square pad's part).
        let mut span = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
        for centre in &centres {
            let at = local(*centre);
            for axis in 0..2 {
                span[axis][0] = span[axis][0].min(at[axis]);
                span[axis][1] = span[axis][1].max(at[axis]);
            }
        }
        let mut work = 0u64;
        let mut pads: Vec<RowPad> = Vec::new();
        for (position, &index) in part.pads.iter().enumerate() {
            let obstacle = &obstacles[index];
            if obstacle.layers.count_ones() != 1 {
                continue;
            }
            let centre = centres[position];
            let at = local(centre);
            let extent = |axis: usize| {
                let direction = axes[axis];
                bga_fanout::half_extent(&obstacle.shape, centre, direction, 5.0).max(
                    bga_fanout::half_extent(
                        &obstacle.shape,
                        centre,
                        [-direction[0], -direction[1]],
                        5.0,
                    ),
                )
            };
            let halves = [extent(0), extent(1)];
            work += 120;
            // Outward: along the pad's long axis away from the middle; a
            // square pad by the side of the centres' box it lies on.
            let axis = if halves[0] >= 1.2 * halves[1] {
                Some(0)
            } else if halves[1] >= 1.2 * halves[0] {
                Some(1)
            } else {
                let room = |axis: usize| {
                    let half = (span[axis][1] - span[axis][0]) / 2.0;
                    if half <= 1.0e-6 {
                        0.0
                    } else {
                        at[axis].abs() / half
                    }
                };
                match room(0).total_cmp(&room(1)) {
                    std::cmp::Ordering::Greater => Some(0),
                    std::cmp::Ordering::Less => Some(1),
                    std::cmp::Ordering::Equal => None,
                }
            };
            let Some(axis) = axis else {
                continue;
            };
            if at[axis].abs() < 1.0e-3 {
                continue;
            }
            let sign = at[axis].signum();
            let side = SIDES
                .iter()
                .position(|&(a, s)| a == axis && s == sign)
                .unwrap_or(0);
            pads.push(RowPad {
                obstacle: index,
                centre,
                layer: obstacle.layers.trailing_zeros() as usize,
                side,
                u: sign * at[axis],
                v: at[1 - axis] * if axis == 0 { sign } else { -sign },
                reach: halves[axis],
            });
        }
        // Rows: per layer and side, by `v`, split where adjacent pads lie
        // farther apart than the fine pitch allows or not in line.
        let mut groups: BTreeMap<(usize, usize), Vec<RowPad>> = BTreeMap::new();
        for pad in pads {
            groups.entry((pad.layer, pad.side)).or_default().push(pad);
        }
        for ((layer, side), mut members) in groups {
            members.sort_by(|a, b| a.v.total_cmp(&b.v));
            let mut rows: Vec<Vec<RowPad>> = Vec::new();
            for pad in members {
                match rows.last_mut() {
                    Some(row)
                        if pad.v - row.last().unwrap().v < FINE_PITCH * ROW_SLACK
                            && (pad.u - row.last().unwrap().u).abs() < 0.1 =>
                    {
                        row.push(pad)
                    }
                    _ => rows.push(vec![pad]),
                }
            }
            for row in rows {
                if row.len() < 2 {
                    continue;
                }
                let (axis, sign) = SIDES[side];
                let outward = [axes[axis][0] * sign, axes[axis][1] * sign];
                // `v` grows along `along` (the outward axis turned a
                // quarter counter-clockwise in the frame).
                let other = 1 - axis;
                let along_sign = if axis == 0 { sign } else { -sign };
                let along = [axes[other][0] * along_sign, axes[other][1] * along_sign];
                let to_board = |u: f64, v: f64| {
                    [
                        middle[0] + outward[0] * u + along[0] * v,
                        middle[1] + outward[1] * u + along[1] * v,
                    ]
                };
                let summary = plan_row(
                    &context,
                    &row,
                    layer,
                    (&part.reference, middle),
                    (outward, along, &to_board),
                    (neck, &poured, holding, &paired, dives),
                    (spans, planes, layer_count, config),
                    &mut planned,
                    &mut plan.exits,
                    &mut work,
                );
                if let Some(summary) = summary {
                    plan.rows.push(summary);
                }
            }
        }
        if plan
            .rows
            .iter()
            .any(|row| row.reference == part.reference && row.refused.is_none())
        {
            plan.work += work;
        }
    }
    plan
}

/// One pad's exit while a row is planned.
#[derive(Clone, Debug)]
struct Exit {
    pad: usize,
    net: core::NetId,
    role: Role,
    /// The track of the exit and the class's (the landing's).
    width: f64,
    class_width: f64,
    clearance: f64,
    /// The lateral shift of the fan and the depth of its turn.
    shift: f64,
    turn: f64,
}

/// Plans one row: its exits into `exits` and their copper into `planned`
/// when every pad is served; the summary (None: no signal pad).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn plan_row(
    context: &Context,
    row: &[RowPad],
    layer: usize,
    part: (&str, [f64; 2]),
    frame: ([f64; 2], [f64; 2], &dyn Fn(f64, f64) -> [f64; 2]),
    rules: (
        f64,
        &BTreeSet<core::NetId>,
        &BTreeSet<usize>,
        &dyn Fn(core::NetId) -> bool,
        bool,
    ),
    board: (
        &[core::ViaSpan],
        &[core::Plane],
        usize,
        &KiCadBoardRouterConfig,
    ),
    planned: &mut Vec<Planned>,
    exits: &mut Vec<PlannedExit>,
    work: &mut u64,
) -> Option<RowSummary> {
    let (reference, middle) = part;
    let (outward, along, to_board) = frame;
    let (neck, poured, holding, paired, dives) = rules;
    let (spans, planes, layer_count, config) = board;
    let obstacles = context.obstacles;
    let pitch = row
        .windows(2)
        .map(|pair| pair[1].v - pair[0].v)
        .fold(f64::INFINITY, f64::min);
    if pitch >= FINE_PITCH - 0.005 {
        return None;
    }
    let mut summary = RowSummary {
        reference: reference.to_string(),
        layer,
        pads: row
            .iter()
            .map(|pad| obstacles[pad.obstacle].label.clone())
            .collect(),
        pitch,
        ..RowSummary::default()
    };
    // The roles.
    let mut members: Vec<Exit> = Vec::new();
    let mut targets: Vec<Option<[f64; 2]>> = Vec::new();
    for (position, pad) in row.iter().enumerate() {
        let Some(net) = obstacles[pad.obstacle].net else {
            continue;
        };
        let description = &context.nets[net as usize];
        let terminal = description
            .terminals
            .iter()
            .find(|terminal| terminal.pad == pad.obstacle);
        // A pour net's pads join by spokes; a custom pad joins at its
        // anchor; a pair's nets launch as a pair: none is the plan's to
        // serve, each keeps a straight corridor out.
        let role = if description.terminals.len() < 2 || holding.contains(&pad.obstacle) {
            Role::Skip
        } else if poured.contains(&net)
            || terminal.is_none_or(|terminal| terminal.contact.is_some())
            || paired(net)
        {
            Role::Corridor
        } else {
            match target_of(context.nets, net, pad.obstacle, reference) {
                Some(_) => Role::Signal,
                None => Role::Skip,
            }
        };
        if role == Role::Skip {
            continue;
        }
        let class = context.classes[description.class];
        // A pad narrower than its net's track and clearance lies in a neck
        // zone (the router's `neck_reach` about it): the search starts there
        // at the neck width.
        // (The router's test: the pad's box, `prepare_net`.)
        let bounds = obstacles[pad.obstacle].shape.aabb();
        let narrow =
            (bounds.maximum[0] - bounds.minimum[0]).min(bounds.maximum[1] - bounds.minimum[1]);
        let landing = if narrow < class.trace_width + class.clearance {
            class.trace_width.min(neck)
        } else {
            class.trace_width
        };
        members.push(Exit {
            pad: position,
            net,
            role,
            width: class.trace_width,
            class_width: landing,
            clearance: class.clearance,
            shift: 0.0,
            turn: 0.0,
        });
        targets.push(if role == Role::Signal {
            target_of(context.nets, net, pad.obstacle, reference)
        } else {
            None
        });
    }
    summary.planned = members.iter().filter(|m| m.role == Role::Signal).count();
    if summary.planned == 0 {
        return None;
    }
    let tip = row
        .iter()
        .map(|pad| pad.u + pad.reach)
        .fold(f64::NEG_INFINITY, f64::max);
    // The obstacles near the row: its pads' box beyond the reach.
    let corners: Vec<[f64; 2]> = row
        .iter()
        .flat_map(|pad| {
            [
                to_board(pad.u - pad.reach, pad.v),
                to_board(tip + REACH + VIA_REACH + 1.0, pad.v),
            ]
        })
        .collect();
    let mut region = core::geometry::Aabb {
        minimum: corners[0],
        maximum: corners[0],
    };
    for corner in &corners {
        region = region.union(core::geometry::Aabb {
            minimum: *corner,
            maximum: *corner,
        });
    }
    let region = region.inflated(2.0);
    let near: Vec<usize> = obstacles
        .iter()
        .enumerate()
        .filter(|(_, obstacle)| obstacle.shape.aabb().intersects(region))
        .map(|(index, _)| index)
        .collect();
    *work += obstacles.len() as u64 / 8;
    // Each exit's width: the class's where its straight stub to beyond the
    // tips clears the pads, else the neck.
    for member in &mut members {
        let pad = &row[member.pad];
        let start = pad.centre;
        let end = to_board(tip + RHO, pad.v);
        if !context.track_clear(
            member.net,
            layer,
            start,
            end,
            member.width,
            start,
            &near,
            work,
        ) && neck < member.width
        {
            member.width = neck;
        }
    }
    // The fan: the spacing each pair of adjacent exits needs at their
    // landings, and the lateral shifts that give it, from the row's middle.
    let count = members.len();
    let need: Vec<f64> = (0..count.saturating_sub(1))
        .map(|k| {
            let (a, b) = (&members[k], &members[k + 1]);
            let gap = row[b.pad].v - row[a.pad].v;
            let clearance = a.clearance.max(b.clearance);
            a.class_width / 2.0 + clearance + b.class_width / 2.0 + RHO - gap
        })
        .collect();
    summary.fan = need.iter().any(|extra| *extra > 1.0e-9);
    let centre = (0..count)
        .min_by(|a, b| {
            row[members[*a].pad]
                .v
                .abs()
                .total_cmp(&row[members[*b].pad].v.abs())
        })
        .unwrap_or(0);
    for k in centre + 1..count {
        members[k].shift = members[k - 1].shift + need[k - 1].max(0.0);
    }
    for k in (0..centre).rev() {
        members[k].shift = members[k + 1].shift - need[k].max(0.0);
    }
    // The turns: the outer pads first; an inner neighbour that turns too
    // turns later by as much as two 45-degree diagonals need to keep their
    // clearance.
    let diagonal_step = |a: &Exit, b: &Exit| {
        let gap = (row[b.pad].v - row[a.pad].v).abs();
        // The landings lie on the diagonals' ends: the lattice's room too.
        let required = a.class_width.max(a.width) / 2.0
            + a.clearance.max(b.clearance)
            + b.class_width.max(b.width) / 2.0
            + RHO;
        (required * std::f64::consts::SQRT_2 - gap).max(0.0) + 1.0e-4
    };
    let mut offsets = vec![0.0; count];
    for k in (centre + 1..count.saturating_sub(1)).rev() {
        if members[k].shift.abs() > 1.0e-9 {
            offsets[k] = offsets[k + 1] + diagonal_step(&members[k], &members[k + 1]);
        }
    }
    for k in 1..centre {
        if members[k].shift.abs() > 1.0e-9 {
            offsets[k] = offsets[k - 1] + diagonal_step(&members[k - 1], &members[k]);
        }
    }
    for (member, offset) in members.iter_mut().zip(&offsets) {
        member.turn = *offset;
    }
    // A pad that does not turn lands beyond every bend of the fan, and
    // beyond its neighbours' turns by as much as its landing's room needs
    // past their corners.
    let corner = (0..count.saturating_sub(1))
        .map(|k| {
            let (a, b) = (&members[k], &members[k + 1]);
            let gap = row[b.pad].v - row[a.pad].v;
            let spacing = a.class_width / 2.0 + a.clearance.max(b.clearance) + b.width / 2.0 + RHO;
            (spacing * spacing - gap * gap).max(0.0).sqrt()
        })
        .fold(0.0, f64::max);
    let deepest = members
        .iter()
        .map(|member| member.turn + member.shift.abs())
        .chain(members.iter().map(|member| member.turn + corner))
        .fold(0.0, f64::max);
    if !summary.fan {
        for member in &mut members {
            member.shift = 0.0;
            member.turn = 0.0;
        }
    }
    // The paths for a base depth `base` (beyond the tips) of the turns.
    let path_of = |member: &Exit, base: f64| -> Vec<[f64; 2]> {
        let pad = &row[member.pad];
        let start = pad.centre;
        if member.shift.abs() <= 1.0e-9 {
            let end = if summary.fan {
                base + deepest + LAND
            } else {
                base
            };
            return vec![start, to_board(end, pad.v)];
        }
        let turn = base + member.turn;
        let bend = turn + member.shift.abs();
        vec![
            start,
            to_board(turn, pad.v),
            to_board(bend, pad.v + member.shift),
            to_board(bend + LAND, pad.v + member.shift),
        ]
    };
    // The first base (in steps beyond the tips) at which every exit, its
    // landing and every pair keep their rules.
    let mut chosen: Option<(f64, Vec<Vec<[f64; 2]>>)> = None;
    let mut base = tip + MIN_EXIT;
    let mut refusal = String::from("no base depth clears the row");
    while base <= tip + MIN_EXIT + REACH + 1.0e-9 {
        let paths: Vec<Vec<[f64; 2]>> = members.iter().map(|m| path_of(m, base)).collect();
        let mut ok = true;
        let mut trial: Vec<Planned> = Vec::new();
        for (member, path) in members.iter().zip(&paths) {
            let start = path[0];
            for piece in path.windows(2) {
                if member.role == Role::Signal
                    && !context.track_clear(
                        member.net,
                        layer,
                        piece[0],
                        piece[1],
                        member.width,
                        start,
                        &near,
                        work,
                    )
                {
                    ok = false;
                    refusal = format!(
                        "{}'s exit meets {}",
                        context.obstacles[row[member.pad].obstacle].label,
                        context.blocker(
                            member.net,
                            layer,
                            piece[0],
                            piece[1],
                            member.width,
                            start,
                            &near
                        )
                    );
                    break;
                }
            }
            if !ok {
                break;
            }
            // The landing: the class's track legal with the lattice's room
            // (a corridor's is the pour's business).
            let end = *path.last().unwrap();
            let zone = core::Shape::Circle {
                center: end,
                radius: member.class_width / 2.0 + RHO,
            };
            if member.role == Role::Signal
                && !context.track_clear(
                    member.net,
                    layer,
                    end,
                    end,
                    member.class_width + 2.0 * RHO,
                    start,
                    &near,
                    work,
                )
            {
                ok = false;
                refusal = format!(
                    "{}'s landing meets {}",
                    context.obstacles[row[member.pad].obstacle].label,
                    context.blocker(
                        member.net,
                        layer,
                        end,
                        end,
                        member.class_width + 2.0 * RHO,
                        start,
                        &near
                    )
                );
                break;
            }
            let copper: Vec<Planned> = path
                .windows(2)
                .map(|piece| Planned {
                    net: member.net,
                    layers: 1 << layer,
                    shape: core::Shape::Capsule {
                        start: piece[0],
                        end: piece[1],
                        radius: member.width / 2.0,
                    },
                    zone: false,
                    hole: None,
                })
                .chain((member.role == Role::Signal).then_some(Planned {
                    net: member.net,
                    layers: 1 << layer,
                    shape: zone,
                    zone: true,
                    hole: None,
                }))
                .collect();
            for piece in &copper {
                if !context.clear_of_planned(member.net, piece, planned, work)
                    || !context.clear_of_planned(member.net, piece, &trial, work)
                {
                    ok = false;
                    refusal = format!(
                        "{}'s exit meets another exit",
                        context.obstacles[row[member.pad].obstacle].label
                    );
                    break;
                }
            }
            if !ok {
                break;
            }
            trial.extend(copper);
        }
        if ok {
            chosen = Some((base, paths));
            break;
        }
        base += STEP;
    }
    let Some((_, mut paths)) = chosen else {
        summary.refused = Some(refusal);
        return Some(summary);
    };
    // The order: the signal pads whose targets keep the row's order stay;
    // the others dive (the longest increasing run of their targets' angles
    // seen from the part's centre, measured from the outward axis).
    let signal: Vec<usize> = (0..count)
        .filter(|k| members[*k].role == Role::Signal)
        .collect();
    let angles: Vec<f64> = signal
        .iter()
        .map(|k| {
            let target = targets[*k].unwrap_or(middle);
            let d = [target[0] - middle[0], target[1] - middle[1]];
            (d[0] * along[0] + d[1] * along[1]).atan2(d[0] * outward[0] + d[1] * outward[1])
        })
        .collect();
    let stay: BTreeSet<usize> = longest_increasing(&angles)
        .into_iter()
        .map(|position| signal[position])
        .collect();
    // Only a pad whose target crosses its signal neighbour's in the row
    // dives: crossings farther apart the routes resolve themselves.
    let crosses_neighbour = |position: usize| {
        (position > 0 && angles[position - 1] > angles[position])
            || (position + 1 < angles.len() && angles[position] > angles[position + 1])
    };
    let divers: Vec<usize> = if dives {
        signal
            .iter()
            .enumerate()
            .filter(|(position, k)| !stay.contains(k) && crosses_neighbour(*position))
            .map(|(_, k)| *k)
            .collect()
    } else {
        Vec::new()
    };
    // The row's copper so far (every exit, the divers' landings dropped as
    // their vias come).
    let mut row_planned: Vec<Planned> = Vec::new();
    for (k, member) in members.iter().enumerate() {
        let path = &paths[k];
        for piece in path.windows(2) {
            row_planned.push(Planned {
                net: member.net,
                layers: 1 << layer,
                shape: core::Shape::Capsule {
                    start: piece[0],
                    end: piece[1],
                    radius: member.width / 2.0,
                },
                zone: false,
                hole: None,
            });
        }
        if member.role == Role::Signal {
            row_planned.push(Planned {
                net: member.net,
                layers: 1 << layer,
                shape: core::Shape::Circle {
                    center: *path.last().unwrap(),
                    radius: member.class_width / 2.0 + RHO,
                },
                zone: true,
                hole: None,
            });
        }
    }
    // The divers' vias: beyond the landing along the outward axis, the
    // first site where a via keeps every rule and a run of the net's track
    // leaves it on another layer it may route on.
    let mut vias: BTreeMap<usize, RowVia> = BTreeMap::new();
    for &k in &divers {
        let member = &members[k];
        let class = context.classes[context.nets[member.net as usize].class];
        let span = bga_fanout::fanout_span(spans, layer, member.net, &class, planes);
        let sizes = bga_fanout::via_sizes(config, &class);
        let landing = *paths[k].last().unwrap();
        let mut found: Option<RowVia> = None;
        let mut extension = VIA_STEP;
        'sites: while extension <= VIA_REACH + 1.0e-9 {
            let at = [
                landing[0] + outward[0] * extension,
                landing[1] + outward[1] * extension,
            ];
            for &[diameter, drill] in &sizes {
                let layers = span.map_or(core::LayerMask::MAX, |span| span.mask());
                if !context.via_clear(
                    member.net,
                    at,
                    diameter / 2.0,
                    drill / 2.0,
                    layers,
                    &near,
                    work,
                ) || !context.track_clear(
                    member.net,
                    layer,
                    landing,
                    at,
                    member.width,
                    row[member.pad].centre,
                    &near,
                    work,
                ) {
                    continue;
                }
                let ring = Planned {
                    net: member.net,
                    layers,
                    shape: core::Shape::Circle {
                        center: at,
                        radius: diameter / 2.0,
                    },
                    zone: false,
                    hole: Some(drill / 2.0),
                };
                let stub = Planned {
                    net: member.net,
                    layers: 1 << layer,
                    shape: core::Shape::Capsule {
                        start: landing,
                        end: at,
                        radius: member.width / 2.0,
                    },
                    zone: false,
                    hole: None,
                };
                if !context.clear_of_planned(member.net, &ring, planned, work)
                    || !context.clear_of_planned(member.net, &ring, &row_planned, work)
                    || !context.clear_of_planned(member.net, &stub, planned, work)
                    || !context.clear_of_planned(member.net, &stub, &row_planned, work)
                {
                    continue;
                }
                // Its landing: a straight run of the net's track off the
                // via's ring on a layer the net may route on, not another
                // net's exclusive plane.
                let reach = diameter / 2.0 + LANDING_RUN;
                let lands = (0..layer_count)
                    .filter(|other| {
                        *other != layer
                            && layers & (1 << other) != 0
                            && (class.layers == 0 || class.layers & (1 << other) != 0)
                            && !planes.iter().any(|plane| {
                                plane.layer == *other && plane.net != member.net && plane.exclusive
                            })
                    })
                    .any(|other| {
                        (0..8).any(|direction| {
                            let angle = direction as f64 * std::f64::consts::FRAC_PI_4;
                            let end = [at[0] + reach * angle.cos(), at[1] + reach * angle.sin()];
                            let run = Planned {
                                net: member.net,
                                layers: 1 << other,
                                shape: core::Shape::Capsule {
                                    start: at,
                                    end,
                                    radius: class.trace_width / 2.0,
                                },
                                zone: false,
                                hole: None,
                            };
                            context.track_clear(
                                member.net,
                                other,
                                at,
                                end,
                                class.trace_width,
                                at,
                                &near,
                                work,
                            ) && context.clear_of_planned(member.net, &run, planned, work)
                                && context.clear_of_planned(member.net, &run, &row_planned, work)
                        })
                    });
                if !lands {
                    continue;
                }
                row_planned.push(ring);
                row_planned.push(stub);
                found = Some(RowVia {
                    at,
                    diameter,
                    drill,
                    span,
                });
                break 'sites;
            }
            extension += VIA_STEP;
        }
        match found {
            Some(via) => {
                paths[k].push(via.at);
                vias.insert(k, via);
                summary
                    .divers
                    .push(context.obstacles[row[member.pad].obstacle].label.clone());
            }
            None => {
                // No site: the pad stays on the surface, its exit served.
                summary
                    .stayed
                    .push(context.obstacles[row[member.pad].obstacle].label.clone());
            }
        }
    }
    // Every pad served: the row's copper stands (corridors are checks only).
    for (k, member) in members.iter().enumerate() {
        if member.role != Role::Signal {
            continue;
        }
        exits.push(PlannedExit {
            net: member.net,
            pad: row[member.pad].obstacle,
            label: context.obstacles[row[member.pad].obstacle].label.clone(),
            layer,
            path: paths[k].clone(),
            width: member.width,
            via: vias.remove(&k),
        });
    }
    planned.extend(row_planned);
    Some(summary)
}

/// Prints each distinct row's plan once a process
/// (`PCB_ROUTER_ROW_FANOUT_WHY`).
pub(crate) fn report(plan: &RowPlan) {
    if std::env::var_os("PCB_ROUTER_ROW_FANOUT_WHY").is_none() {
        return;
    }
    static SEEN: std::sync::Mutex<BTreeSet<String>> = std::sync::Mutex::new(BTreeSet::new());
    for row in &plan.rows {
        let line = format!(
            "row fan-out {} layer {}: {} pads at {:.3} mm ({} .. {}), exits {}, {}, divers {:?}, no via site {:?}{}",
            row.reference,
            row.layer,
            row.pads.len(),
            row.pitch,
            row.pads.first().map_or("", String::as_str),
            row.pads.last().map_or("", String::as_str),
            row.planned,
            if row.fan { "fanned" } else { "straight" },
            row.divers,
            row.stayed,
            row.refused
                .as_ref()
                .map_or(String::new(), |why| format!("; left to the search: {why}"))
        );
        if SEEN.lock().is_ok_and(|mut seen| seen.insert(line.clone())) {
            eprintln!("{line}");
        }
    }
}

/// A planned exit as the lowered board holds it (`exits_of`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExitCopper {
    pub net: core::NetId,
    pub layer: usize,
    /// The exit's pieces in order: from the pad's centre to its end.
    pub pieces: Vec<([f64; 2], [f64; 2])>,
    pub width: f64,
    /// Its landing (the end of its last piece on the surface).
    pub landing: [f64; 2],
    /// The via (diameter, drill, span), a diver's.
    pub via: Option<([f64; 2], f64, f64, Option<core::ViaSpan>)>,
}

/// The exits the lowering placed (the obstacles it labels `<pad> row exit`,
/// `<pad> row landing`, `... row via ring` and `... row hole`), by pad.
pub(crate) fn exits_of(board: &core::Board) -> Vec<ExitCopper> {
    let mut exits: BTreeMap<&str, ExitCopper> = BTreeMap::new();
    let mut rings: BTreeMap<&str, f64> = BTreeMap::new();
    for obstacle in &board.obstacles {
        match (&obstacle.shape, obstacle.kind) {
            (core::Shape::Capsule { start, end, radius }, core::ObstacleKind::Copper) => {
                if let (Some(pad), Some(net)) =
                    (obstacle.label.strip_suffix(" row exit"), obstacle.net)
                {
                    let exit = exits.entry(pad).or_insert_with(|| ExitCopper {
                        net,
                        layer: obstacle.layers.trailing_zeros() as usize,
                        pieces: Vec::new(),
                        width: 2.0 * radius,
                        landing: *end,
                        via: None,
                    });
                    exit.pieces.push((*start, *end));
                }
            }
            (core::Shape::Circle { center, radius }, core::ObstacleKind::Copper) => {
                if let Some(pad) = obstacle.label.strip_suffix(" row landing")
                    && let Some(exit) = exits.get_mut(pad)
                {
                    exit.landing = *center;
                }
                if let Some(pad) = obstacle.label.strip_suffix(" row via ring") {
                    rings.insert(pad, 2.0 * radius);
                }
            }
            _ => {}
        }
    }
    for obstacle in &board.obstacles {
        if obstacle.kind != core::ObstacleKind::Hole {
            continue;
        }
        let Some(pad) = obstacle.label.strip_suffix(" row hole") else {
            continue;
        };
        let core::Shape::Circle { center, radius } = &obstacle.shape else {
            continue;
        };
        if let (Some(exit), Some(diameter)) = (exits.get_mut(pad), rings.get(pad)) {
            let span = (obstacle.layers != board.all_layers())
                .then(|| {
                    board
                        .via_spans
                        .iter()
                        .find(|span| span.mask() == obstacle.layers)
                        .copied()
                })
                .flatten();
            exit.via = Some((*center, *diameter, 2.0 * radius, span));
        }
    }
    exits.into_values().collect()
}

/// How the net's route uses an exit: (on the surface: the route's segment
/// end at the landing, if any; through the via off the pad's layer).
pub(crate) fn exit_use(
    board: &core::Board,
    exit: &ExitCopper,
    route: &core::NetRoute,
) -> (Option<[f64; 2]>, bool) {
    const TOUCH: f64 = 1.0e-6;
    // A route ends on the landing within the reach the router's terminal
    // nodes have (`joins_copper`: the track's half width less 0.02 mm from
    // the landing's copper).
    let surface = route
        .segments
        .iter()
        .filter(|segment| segment.layer == exit.layer)
        .flat_map(|segment| {
            let reach = exit.width / 2.0 + segment.width / 2.0 + TOUCH;
            [segment.start, segment.end]
                .into_iter()
                .filter(move |end| core::geometry::distance(*end, exit.landing) <= reach)
        })
        .min_by(|a, b| {
            core::geometry::distance(*a, exit.landing)
                .total_cmp(&core::geometry::distance(*b, exit.landing))
        });
    let through = exit.via.is_some_and(|(at, diameter, _, span)| {
        let radius = diameter / 2.0;
        let on = |layer: usize| span.is_none_or(|span| span.contains(layer));
        let mask = span.map_or(core::LayerMask::MAX, |span| span.mask());
        route.segments.iter().any(|segment| {
            segment.layer != exit.layer
                && on(segment.layer)
                && core::geometry::point_segment_distance(at, segment.start, segment.end)
                    <= radius + segment.width / 2.0 + TOUCH
        }) || route.vias.iter().any(|via| {
            via.mask(board.layer_count) & mask != 0
                && core::geometry::distance(via.at, at) <= radius + via.diameter / 2.0 + TOUCH
        }) || board.planes.iter().any(|plane| {
            plane.net == exit.net
                && plane.layer != exit.layer
                && on(plane.layer)
                && core::geometry::point_in_polygon(at, &plane.polygon)
        })
    });
    (surface, through)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class() -> core::RuleClass {
        core::RuleClass {
            trace_width: 0.2,
            clearance: 0.2,
            via_diameter: 0.6,
            via_drill: 0.3,
            layers: 0,
        }
    }

    fn pad(
        center: [f64; 2],
        size: [f64; 2],
        net: Option<core::NetId>,
        label: &str,
    ) -> core::Obstacle {
        let (hx, hy) = (size[0] / 2.0, size[1] / 2.0);
        core::Obstacle {
            shape: core::Shape::Polygon {
                points: vec![
                    [center[0] - hx, center[1] - hy],
                    [center[0] + hx, center[1] - hy],
                    [center[0] + hx, center[1] + hy],
                    [center[0] - hx, center[1] + hy],
                ],
            },
            layers: 1,
            kind: core::ObstacleKind::Copper,
            net,
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: label.into(),
        }
    }

    /// A part U1 of `count` pads in a row along x at `pitch`, 0.25 x 0.8 mm,
    /// leaving toward -y (the part's middle lies at y = 2 by a mirror row
    /// of pads of no net), each pad of its own net whose other pad lies
    /// far away at the x of `targets[i]`, 10 mm out.
    fn row(
        pitch: f64,
        width: f64,
        targets: &[f64],
    ) -> (Vec<RowPart>, Vec<core::Obstacle>, Vec<core::Net>) {
        let mut obstacles = Vec::new();
        let mut nets = Vec::new();
        let mut pads = Vec::new();
        for (index, target) in targets.iter().enumerate() {
            let x = index as f64 * pitch;
            let net = nets.len() as core::NetId;
            let at = obstacles.len();
            obstacles.push(pad(
                [x, 0.0],
                [width, 0.8],
                Some(net),
                &format!("U1.{}", index + 1),
            ));
            pads.push(at);
            let far = obstacles.len();
            obstacles.push(pad(
                [*target, -10.0],
                [1.0, 1.0],
                Some(net),
                &format!("J1.{}", index + 1),
            ));
            nets.push(core::Net {
                name: format!("N{}", index + 1),
                class: 0,
                terminals: vec![
                    core::Terminal {
                        anchor: [x, 0.0],
                        layers: 1,
                        pad: at,
                        contact: None,
                        label: format!("U1.{}", index + 1),
                    },
                    core::Terminal {
                        anchor: [*target, -10.0],
                        layers: 1,
                        pad: far,
                        contact: None,
                        label: format!("J1.{}", index + 1),
                    },
                ],
            });
        }
        // The mirror row (no net): the part's middle at y = 2.
        for index in 0..targets.len() {
            let at = obstacles.len();
            obstacles.push(pad(
                [index as f64 * pitch, 4.0],
                [width, 0.8],
                None,
                &format!("U1.{}", targets.len() + index + 1),
            ));
            pads.push(at);
        }
        (
            vec![RowPart {
                reference: "U1".into(),
                angle: 0.0,
                pads,
            }],
            obstacles,
            nets,
        )
    }

    fn config() -> KiCadBoardRouterConfig {
        KiCadBoardRouterConfig {
            hole_clearance_mm: 0.2,
            hole_to_hole_clearance_mm: 0.25,
            ..KiCadBoardRouterConfig::default()
        }
    }

    fn plan_of(parts: &[RowPart], obstacles: &[core::Obstacle], nets: &[core::Net]) -> RowPlan {
        super::plan(
            parts,
            obstacles,
            nets,
            &[class()],
            &[],
            &BTreeSet::new(),
            (&[], &[]),
            &[],
            2,
            &config(),
        )
    }

    /// Every pair of exits of different nets keeps the clearance.
    fn assert_clear(plan: &RowPlan) {
        for (i, a) in plan.exits.iter().enumerate() {
            for b in &plan.exits[i + 1..] {
                for pa in a.path.windows(2) {
                    for pb in b.path.windows(2) {
                        let d =
                            core::geometry::segment_segment_distance(pa[0], pa[1], pb[0], pb[1]);
                        assert!(
                            d >= (a.width + b.width) / 2.0 + 0.2 - 1.0e-6,
                            "{} and {}: {d}",
                            a.label,
                            b.label
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_row_with_straight_ways_gets_straight_exits_beyond_its_tips() {
        // 0.65 > 0.6 mm: two tracks, a clearance and the lattice's room fit.
        let (parts, obstacles, nets) = row(0.6, 0.25, &[0.0, 0.6, 1.2, 1.8, 2.4]);
        let plan = plan_of(&parts, &obstacles, &nets);
        assert_eq!(plan.rows.len(), 1, "{:?}", plan.rows);
        let summary = &plan.rows[0];
        assert!(summary.refused.is_none(), "{summary:?}");
        assert!(!summary.fan && summary.divers.is_empty());
        assert_eq!(plan.exits.len(), 5);
        for exit in &plan.exits {
            // Straight down from the centre, beyond the tip (y = -0.4).
            assert_eq!(exit.path.len(), 2);
            assert!((exit.path[0][0] - exit.path[1][0]).abs() < 1.0e-9);
            assert!(exit.path[1][1] <= -0.4 - MIN_EXIT + 1.0e-6);
            assert!(exit.via.is_none());
            assert_eq!(exit.width, 0.2);
        }
        assert_clear(&plan);
        assert!(plan.work > 0);
    }

    #[test]
    fn an_exact_fit_row_fans_its_exits_out_to_room_for_the_lattice() {
        // 0.4 mm: 0.2 track and clearance an exact fit between the pads.
        let (parts, obstacles, nets) = row(0.4, 0.2, &[0.0, 0.4, 0.8, 1.2, 1.6, 2.0, 2.4]);
        let plan = plan_of(&parts, &obstacles, &nets);
        let summary = &plan.rows[0];
        assert!(summary.refused.is_none(), "{summary:?}");
        assert!(summary.fan);
        assert_eq!(plan.exits.len(), 7);
        assert_clear(&plan);
        // The landings keep two half tracks, a clearance and the room apart.
        let mut landings: Vec<[f64; 2]> =
            plan.exits.iter().map(|e| *e.path.last().unwrap()).collect();
        landings.sort_by(|a, b| a[0].total_cmp(&b[0]));
        for pair in landings.windows(2) {
            assert!(
                pair[1][0] - pair[0][0] >= 0.4 + RHO - 1.0e-6,
                "{landings:?}"
            );
        }
    }

    #[test]
    fn a_crossing_pair_sends_one_pad_down_a_via_beyond_the_row() {
        // N2 and N3 swap sides: N3's target lies left of N2's.
        let (parts, obstacles, nets) = row(0.5, 0.25, &[0.0, 1.5, 0.5, 2.0]);
        let plan = plan_of(&parts, &obstacles, &nets);
        let summary = &plan.rows[0];
        assert!(summary.refused.is_none(), "{summary:?}");
        assert_eq!(summary.divers.len(), 1, "{summary:?}");
        let diver = plan.exits.iter().find(|exit| exit.via.is_some()).unwrap();
        assert!(diver.label == "U1.2" || diver.label == "U1.3");
        let via = diver.via.as_ref().unwrap();
        // Beyond the row's landings: below every other exit's end.
        for exit in plan.exits.iter().filter(|e| e.via.is_none()) {
            assert!(via.at[1] < exit.path.last().unwrap()[1]);
        }
        assert_eq!(*diver.path.last().unwrap(), via.at);
        assert_clear(&plan);
    }

    #[test]
    fn a_diver_whose_via_has_no_landing_stays_on_the_surface() {
        let (parts, mut obstacles, nets) = row(0.5, 0.25, &[0.0, 1.5, 0.5, 2.0]);
        // A copper plate of no net on B.Cu under the whole region: no via
        // leaves on the other layer (vias may pass it).
        obstacles.push(core::Obstacle {
            shape: core::Shape::Polygon {
                points: vec![[-3.0, -6.0], [5.0, -6.0], [5.0, -0.5], [-3.0, -0.5]],
            },
            layers: 2,
            kind: core::ObstacleKind::Copper,
            net: None,
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: false,
            label: "plate".into(),
        });
        let plan = plan_of(&parts, &obstacles, &nets);
        let summary = &plan.rows[0];
        assert!(summary.refused.is_none(), "{summary:?}");
        assert!(summary.divers.is_empty());
        assert_eq!(summary.stayed.len(), 1, "{summary:?}");
        assert_eq!(plan.exits.len(), 4);
        assert!(plan.exits.iter().all(|exit| exit.via.is_none()));
    }

    #[test]
    fn a_row_whose_landing_is_blocked_is_left_to_the_search() {
        let (parts, mut obstacles, nets) = row(0.5, 0.25, &[0.0, 0.5, 1.0, 1.5]);
        // A part of no net just beyond the row's tips across its whole
        // width: no exit lands.
        obstacles.push(pad([0.75, -1.0], [3.0, 0.6], None, "R1.1"));
        let plan = plan_of(&parts, &obstacles, &nets);
        let summary = &plan.rows[0];
        assert!(
            summary
                .refused
                .as_deref()
                .is_some_and(|why| why.contains("R1.1")),
            "{summary:?}"
        );
        assert!(plan.exits.is_empty());
        assert_eq!(plan.work, 0);
    }

    #[test]
    fn nothing_is_planned_where_no_row_is_fine_pitch() {
        let (parts, obstacles, nets) = row(0.8, 0.25, &[0.0, 0.8, 1.6, 2.4]);
        let plan = plan_of(&parts, &obstacles, &nets);
        assert!(plan.rows.is_empty());
        assert!(plan.exits.is_empty());
        assert_eq!(plan.work, 0);
    }

    /// A board: U1's row of six 0.25 x 0.8 mm pads at 0.5 mm on F.Cu
    /// (a mirror row of no net 3 mm south makes the part's middle), each
    /// net's other pad on J1 10 mm north; tracks and clearance 0.2 mm.
    fn row_board() -> (Expr, KiCadBoardRouterConfig) {
        let mut pads = String::new();
        let mut ends = String::new();
        for index in 0..6 {
            let x = (index as f64 - 2.5) * 0.5;
            pads.push_str(&format!(
                r#"(pad "{n}" smd rect (at {x} 0) (size 0.25 0.8) (layers "F.Cu" "F.Mask") (net "N{n}"))
                   (pad "{m}" smd rect (at {x} 3) (size 0.25 0.8) (layers "F.Cu" "F.Mask"))"#,
                n = index + 1,
                m = index + 7,
            ));
            ends.push_str(&format!(
                r#"(pad "{n}" smd rect (at {} 0) (size 1 1) (layers "F.Cu") (net "N{n}"))"#,
                index as f64 * 2.0,
                n = index + 1,
            ));
        }
        let pcb = parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
                 (gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts"))
                 (footprint "ROW" (at 15 20) (property "Reference" "U1") {pads})
                 (footprint "J" (at 10 8) (property "Reference" "J1") {ends}))"#
        ))
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            hole_clearance_mm: 0.2,
            hole_to_hole_clearance_mm: 0.25,
            row_fanout: Some(true),
            ..KiCadBoardRouterConfig::default()
        };
        (pcb, config)
    }

    #[test]
    fn the_lowering_makes_an_exit_the_pads_terminal_copper_and_drops_one_its_route_does_not_use() {
        let (mut pcb, config) = row_board();
        let off = KiCadBoardRouterConfig {
            row_fanout: Some(false),
            ..config.clone()
        };
        let unset = KiCadBoardRouterConfig {
            row_fanout: None,
            ..config.clone()
        };
        assert_eq!(
            row_fanout_on(&unset),
            row_fanout_override().unwrap_or(false)
        );
        // Off: nothing planned, nothing charged, no terminal copper.
        let plain = lower(&pcb, &off, false).unwrap();
        assert!(exits_of(&plain.board).is_empty());
        assert!(plain.board.terminal_copper.is_empty());
        let lowered = lower(&pcb, &config, false).unwrap();
        // The plan's own work, charged as lowering work.
        assert!(lowered.fanout_work > plain.fanout_work);
        let board = lowered.board;
        let exits = exits_of(&board);
        // Six exits north (toward J1), straight: 0.5 mm leaves room.
        assert_eq!(exits.len(), 6);
        assert_eq!(board.terminal_copper.len(), 6);
        for exit in &exits {
            assert_eq!(exit.pieces.len(), 1);
            assert!(exit.landing[1] < 20.0 - 0.4 - MIN_EXIT + 1.0e-6);
            assert!(exit.via.is_none());
        }
        let net = |name: &str| board.nets.iter().position(|net| net.name == name).unwrap();
        let exit = |name: &str| {
            exits
                .iter()
                .find(|exit| exit.net as usize == net(name))
                .unwrap()
                .clone()
        };
        let (n1, n2) = (exit("N1"), exit("N2"));
        // N1's route starts 0.1 mm beyond its landing (a lattice node the
        // landing reaches); N2's leaves its pad to the south.
        let mut result = core::route(&board, &core_config(&off));
        let from = [n1.landing[0], n1.landing[1] - 0.15];
        result.routes[net("N1")] = core::NetRoute {
            segments: vec![core::Segment {
                layer: 0,
                start: from,
                end: [from[0], from[1] - 5.0],
                width: 0.2,
            }],
            vias: Vec::new(),
        };
        let start = n2.pieces[0].0;
        result.routes[net("N2")] = core::NetRoute {
            segments: vec![core::Segment {
                layer: 0,
                start,
                end: [start[0], start[1] + 0.3],
                width: 0.2,
            }],
            vias: Vec::new(),
        };
        assert_eq!(
            exit_use(&board, &n1, &result.routes[net("N1")]),
            (Some(from), false)
        );
        assert_eq!(exit_use(&board, &n2, &result.routes[net("N2")]).0, None);
        let names: Vec<String> = vec!["F.Cu".into(), "B.Cu".into()];
        emit_routes(&mut pcb, &board, &result, &names, false).unwrap();
        let segments: Vec<(String, [f64; 2], [f64; 2])> = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("segment"))
            .map(|item| {
                let point = |head: &str| {
                    [
                        form_f64(item, head, 1).unwrap(),
                        form_f64(item, head, 2).unwrap(),
                    ]
                };
                let net = form_atom(item, "net", 1)
                    .unwrap_or("")
                    .trim_matches('"')
                    .to_string();
                (net, point("start"), point("end"))
            })
            .collect();
        let near =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1.0e-6 && (a[1] - b[1]).abs() < 1.0e-6;
        // N1: the exit, the joint from the route's end to the landing, the route.
        assert!(
            segments
                .iter()
                .any(|(_, a, b)| near(*a, n1.pieces[0].0) && near(*b, n1.landing))
        );
        assert!(
            segments
                .iter()
                .any(|(_, a, b)| near(*a, from) && near(*b, n1.landing))
        );
        // N2's exit is dropped.
        assert!(!segments.iter().any(|(_, _, b)| near(*b, n2.landing)));
    }

    /// A QFN-56 (14 pads a side at 0.4 mm, 0.2 x 0.875 mm, centres 3.4375
    /// mm out), every pad its own net to a pad 10 mm straight out: every
    /// side fans, and the four fans keep clear of each other at the
    /// corners.
    #[test]
    fn the_four_sides_of_a_qfn_fan_without_meeting_at_the_corners() {
        let mut obstacles = Vec::new();
        let mut nets = Vec::new();
        let mut pads = Vec::new();
        for side in 0..4 {
            for index in 0..14 {
                let along = (index as f64 - 6.5) * 0.4;
                let (center, size, far) = match side {
                    0 => ([3.4375, along], [0.875, 0.2], [13.0, along]),
                    1 => ([-along, 3.4375], [0.2, 0.875], [-along, 13.0]),
                    2 => ([-3.4375, -along], [0.875, 0.2], [-13.0, -along]),
                    _ => ([along, -3.4375], [0.2, 0.875], [along, -13.0]),
                };
                let net = nets.len() as core::NetId;
                let label = format!("U1.{}", side * 14 + index + 1);
                let at = obstacles.len();
                obstacles.push(pad(center, size, Some(net), &label));
                pads.push(at);
                let end = obstacles.len();
                obstacles.push(pad(far, [0.5, 0.5], Some(net), "J"));
                nets.push(core::Net {
                    name: label.clone(),
                    class: 0,
                    terminals: vec![
                        core::Terminal {
                            anchor: center,
                            layers: 1,
                            pad: at,
                            contact: None,
                            label: label.clone(),
                        },
                        core::Terminal {
                            anchor: far,
                            layers: 1,
                            pad: end,
                            contact: None,
                            label: format!("J{net}.1"),
                        },
                    ],
                });
            }
        }
        let parts = vec![RowPart {
            reference: "U1".into(),
            angle: 0.0,
            pads,
        }];
        let plan = plan_of(&parts, &obstacles, &nets);
        for row in &plan.rows {
            assert!(row.refused.is_none(), "{row:?}");
        }
        assert_eq!(plan.exits.len(), 56);
        assert_clear(&plan);
    }

    #[test]
    fn the_longest_increasing_run_keeps_the_most_pads() {
        assert_eq!(longest_increasing(&[0.0, 3.0, 1.0, 2.0]), vec![0, 2, 3]);
        assert_eq!(longest_increasing(&[]), Vec::<usize>::new());
        assert_eq!(longest_increasing(&[2.0, 1.0]).len(), 1);
    }
}

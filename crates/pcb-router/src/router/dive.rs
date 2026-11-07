// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The crossing census (`PCB_ROUTER_TROUBLE_WHY`, router/trouble.rs):
//! after the trouble repair, every open dense pad of a signal net still
//! open (a pad of the fan-out plan, `fanout_plan`), its straight ways out
//! and what closes each (a neighbour's via or track), and whether that
//! neighbour's pad in the row has a target on the other side along the
//! row: a crossing pair, where on one layer the two nets cannot both
//! leave and one of them has to dive (docs/router.md, "Fan-out before
//! negotiation (2026-10-28)", the diagnosis). Analysis only: nothing is
//! changed and no work is charged. The crossing dive that acted on the
//! pairs (`crossing_dive`, off) was removed on 2026-11-04 (docs/handover.md,
//! "Removed experiments (2026-11-04)").

use super::fanout::{DensePad, FanoutPlan};
use super::*;

/// A row neighbour lies at most this far along the row from the pad
/// (centre to centre), mm: two or three pads of a 0.4 to 0.5 mm row.
const ROW_REACH: f64 = 1.5;

/// The fan-out plan (the dense pads, their ways out and via sites, on the
/// statics) and its pads by net and by row, for the census.
pub(super) struct DivePlan {
    pub(super) plan: FanoutPlan,
    by_net: HashMap<NetId, Vec<usize>>,
    by_row: HashMap<(String, usize, usize), Vec<usize>>,
}

impl DivePlan {
    pub(super) fn new(plan: FanoutPlan) -> Self {
        let mut by_net: HashMap<NetId, Vec<usize>> = HashMap::new();
        let mut by_row: HashMap<(String, usize, usize), Vec<usize>> = HashMap::new();
        for (index, pad) in plan.pads.iter().enumerate() {
            by_net.entry(pad.net).or_default().push(index);
            if let Some((part, outward)) = &pad.row {
                by_row
                    .entry((part.clone(), pad.layer, *outward))
                    .or_default()
                    .push(index);
            }
        }
        Self {
            plan,
            by_net,
            by_row,
        }
    }
}

/// The coordinate along a row that leaves along `outward` (an index of
/// `DIRECTIONS`).
fn along_row(outward: usize, point: crate::geometry::Point) -> f64 {
    let (dx, dy) = DIRECTIONS[outward];
    point[0] * -(dy as f64) + point[1] * dx as f64
}

impl Router {
    fn pad_anchor(&self, pad: &DensePad) -> crate::geometry::Point {
        self.board.nets[pad.net as usize].terminals[pad.terminal].anchor
    }

    /// Whether `pad`'s terminal is open: its net is not complete and no
    /// branch of the net ends at it (a net's first terminal counts as
    /// connected in its flags even where no route left it).
    fn pad_open(&self, pad: &DensePad) -> bool {
        let state = &self.nets[pad.net as usize];
        let terminal = pad.terminal as u16;
        !state.complete
            && !state
                .branches
                .iter()
                .any(|branch| branch.start_terminal == terminal || branch.end_terminal == terminal)
    }

    fn pad_label(&self, pad: &DensePad) -> &str {
        &self.board.nets[pad.net as usize].terminals[pad.terminal].label
    }

    /// What closes `pad`'s straight ways out (its runs along the axes):
    /// per run, the nets of `nets` whose copper comes
    /// closer to the run than both half widths and the clearance (a via by
    /// its radius), and whether by a via; into `blocks` as (run, net, via).
    /// Returns the nodes looked at (the work).
    fn tip_blocks(
        &self,
        pad: &DensePad,
        nets: &mut dyn Iterator<Item = NetId>,
        blocks: &mut Vec<(usize, NetId, bool)>,
    ) -> u64 {
        let runs: Vec<(usize, &super::fanout::ExitRun)> = pad.runs.iter().enumerate().collect();
        if runs.is_empty() {
            return 0;
        }
        // The runs' box, grown by the furthest reach (a via of 1 mm).
        let grow = pad.half_width + pad.clearance.max(0.5) + 1.0;
        let (mut x0, mut y0, mut x1, mut y1) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for (_, run) in &runs {
            x0 = x0.min(run.start[0].min(run.end[0]) - grow);
            y0 = y0.min(run.start[1].min(run.end[1]) - grow);
            x1 = x1.max(run.start[0].max(run.end[0]) + grow);
            y1 = y1.max(run.start[1].max(run.end[1]) + grow);
        }
        let mut work = 0u64;
        for net in nets {
            if net == pad.net {
                continue;
            }
            let class = self.board.nets[net as usize].class;
            let rules = self.board.classes[class];
            let clearance = pad.clearance.max(rules.clearance) - 1.0e-9;
            for branch in &self.nets[net as usize].branches {
                for (index, node) in branch.nodes.iter().enumerate() {
                    work += 1;
                    let is_via = index > 0
                        && branch.nodes[index - 1].cell == node.cell
                        && branch.nodes[index - 1].layer != node.layer;
                    if !is_via && node.layer as usize != pad.layer {
                        continue;
                    }
                    let point = self.grid.center_of(node.cell as usize);
                    if point[0] < x0 || point[0] > x1 || point[1] < y0 || point[1] > y1 {
                        continue;
                    }
                    let reach = pad.half_width
                        + clearance
                        + if is_via {
                            rules.via_diameter / 2.0
                        } else {
                            self.width_on(self.node_class(net, node.cell), pad.layer) / 2.0
                        };
                    for (run_index, run) in &runs {
                        if crate::geometry::point_segment_distance(point, run.start, run.end)
                            < reach
                        {
                            match blocks
                                .iter_mut()
                                .find(|(r, n, _)| r == run_index && *n == net)
                            {
                                Some(entry) => entry.2 |= is_via,
                                None => blocks.push((*run_index, net, is_via)),
                            }
                        }
                    }
                }
            }
        }
        work
    }

    /// The pads of `dive`'s row of pad `index` within `ROW_REACH` along the
    /// row, of other nets, nearest first.
    fn row_neighbours(&self, dive: &DivePlan, index: usize) -> Vec<usize> {
        let pad = &dive.plan.pads[index];
        let Some((part, outward)) = &pad.row else {
            return Vec::new();
        };
        let Some(members) = dive.by_row.get(&(part.clone(), pad.layer, *outward)) else {
            return Vec::new();
        };
        let here = along_row(*outward, self.pad_anchor(pad));
        let mut near: Vec<(f64, usize)> = members
            .iter()
            .copied()
            .filter(|other| dive.plan.pads[*other].net != pad.net)
            .map(|other| {
                let at = along_row(*outward, self.pad_anchor(&dive.plan.pads[other]));
                ((at - here).abs(), other)
            })
            .filter(|(distance, _)| *distance <= ROW_REACH)
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        near.into_iter().map(|(_, other)| other).collect()
    }

    /// Whether the targets of the pads `a` and `b` of one row lie on
    /// opposite sides of each other along the row (their tracks, leaving
    /// on one layer side by side, would cross).
    fn targets_cross(&self, a: &DensePad, b: &DensePad) -> bool {
        let (Some((_, outward)), Some(target_a), Some(target_b)) = (&a.row, a.target, b.target)
        else {
            return false;
        };
        let pads =
            along_row(*outward, self.pad_anchor(a)) - along_row(*outward, self.pad_anchor(b));
        let targets = along_row(*outward, target_a) - along_row(*outward, target_b);
        pads * targets < -1.0e-9
    }

    /// `PCB_ROUTER_TROUBLE_WHY`: every open dense pad (`pad_open`) of a signal net still open,
    /// its straight ways out and what closes each (the net, by a via or a
    /// track, and whether that net has a pad in the row whose target
    /// crosses this one's: a crossing pair); open nets without a dense pad
    /// are named too. Analysis only: no work is charged.
    pub(super) fn crossing_census(&self, dive: &DivePlan, order: &[NetId], label: &str) {
        let (mut open_nets, mut pads, mut boxed, mut paired, mut no_way) = (0, 0, 0, 0, 0);
        let mut without = 0;
        for &net in order {
            let state = &self.nets[net as usize];
            if !state.routable || !state.plane.is_empty() || state.complete {
                continue;
            }
            open_nets += 1;
            let name = &self.board.nets[net as usize].name;
            let indices = dive.by_net.get(&net).map_or(&[][..], Vec::as_slice);
            if !indices
                .iter()
                .any(|index| self.pad_open(&dive.plan.pads[*index]))
            {
                without += 1;
                eprintln!("crossing census {label}: {name}: open, no open dense pad");
                continue;
            }
            for &index in indices {
                let pad = &dive.plan.pads[index];
                if !self.pad_open(pad) {
                    continue;
                }
                pads += 1;
                let ways: Vec<usize> = (0..pad.runs.len()).collect();
                if ways.is_empty() {
                    no_way += 1;
                    eprintln!(
                        "crossing census {label}: {} ({name}) layer {}: no straight way out",
                        self.pad_label(pad),
                        pad.layer
                    );
                    continue;
                }
                let mut blocks = Vec::new();
                self.tip_blocks(
                    pad,
                    &mut (0..self.nets.len() as NetId).filter(|other| {
                        let other = &self.nets[*other as usize];
                        other.plane.is_empty() || !other.branches.is_empty()
                    }),
                    &mut blocks,
                );
                let closed = ways
                    .iter()
                    .filter(|way| blocks.iter().any(|(run, _, _)| run == *way))
                    .count();
                if closed == ways.len() {
                    boxed += 1;
                }
                let neighbours = self.row_neighbours(dive, index);
                let mut nets: Vec<NetId> = blocks.iter().map(|(_, net, _)| *net).collect();
                nets.sort_unstable();
                nets.dedup();
                let mut pair = false;
                let named: Vec<String> = nets
                    .iter()
                    .map(|other| {
                        let via = blocks.iter().any(|(_, net, via)| net == other && *via);
                        let in_row = neighbours
                            .iter()
                            .find(|neighbour| dive.plan.pads[**neighbour].net == *other);
                        let row = match in_row {
                            Some(neighbour) => {
                                let crossing = self.targets_cross(pad, &dive.plan.pads[*neighbour]);
                                pair |= crossing;
                                format!(
                                    ", row {}{}",
                                    self.pad_label(&dive.plan.pads[*neighbour]),
                                    if crossing { ", crossing" } else { "" }
                                )
                            }
                            None => String::new(),
                        };
                        format!(
                            "{} {}{row}",
                            self.board.nets[*other as usize].name,
                            if via { "via" } else { "track" }
                        )
                    })
                    .collect();
                if pair {
                    paired += 1;
                }
                eprintln!(
                    "crossing census {label}: {} ({name}) layer {}: {closed} of {} ways closed by [{}]{}",
                    self.pad_label(pad),
                    pad.layer,
                    ways.len(),
                    named.join("; "),
                    if pair { "; a crossing pair" } else { "" }
                );
            }
        }
        eprintln!(
            "crossing census {label}: {open_nets} signal nets open ({without} without an open dense pad), {pads} open dense pads of theirs, {boxed} with every straight way closed, {paired} in a crossing pair, {no_way} without a straight way out"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Net, Obstacle, ObstacleKind, RuleClass, Terminal};
    use crate::geometry::Shape;

    fn obstacle(
        shape: Shape,
        net: Option<NetId>,
        kind: ObstacleKind,
        layers: u32,
        label: &str,
    ) -> Obstacle {
        Obstacle {
            shape,
            layers,
            kind,
            net,
            clearance: 0.0,
            clearance_override: None,
            plated: false,
            blocks_tracks: true,
            blocks_vias: true,
            label: label.into(),
        }
    }

    fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
        Shape::Polygon {
            points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        }
    }

    /// The pad of the row whose net is open (U1.2, net N2) and its crossing
    /// neighbour's (U1.3, net N3).
    const OPEN: NetId = 2;
    const OTHER: NetId = 3;

    /// Two layers, a row of six 0.3 mm pads at 0.5 mm pitch on the top
    /// (U1.0 to U1.5 at x 4.0 to 6.5, nets N0 to N5), each net to a pad at
    /// the top wall in the row's order but N2 and N3, whose targets
    /// cross (N2's at x 6.5, N3's at 4.0). Walls on both layers below the
    /// row and above the far pads: every net's track splits the space
    /// between them, so N2 and N3 cannot both stay on the top.
    fn crossing_row() -> Router {
        let class = RuleClass {
            trace_width: 0.2,
            clearance: 0.2,
            via_diameter: 0.6,
            via_drill: 0.3,
            layers: 0,
        };
        let targets = [1.0, 2.5, 6.5, 4.0, 8.0, 9.5];
        let mut obstacles = Vec::new();
        let mut nets = Vec::new();
        let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal {
            anchor: at,
            layers: 1,
            pad,
            contact: None,
            label: label.into(),
        };
        for (index, far) in targets.iter().enumerate() {
            let x = 4.0 + 0.5 * index as f64;
            let net = index as NetId;
            obstacles.push(obstacle(
                rectangle(x - 0.15, 4.5, x + 0.15, 5.5),
                Some(net),
                ObstacleKind::Copper,
                1,
                &format!("U1.{index}"),
            ));
            let row = obstacles.len() - 1;
            obstacles.push(obstacle(
                Shape::Circle {
                    center: [*far, 9.0],
                    radius: 0.2,
                },
                Some(net),
                ObstacleKind::Copper,
                1,
                &format!("J{index}.1"),
            ));
            nets.push(Net {
                name: format!("N{index}"),
                class: 0,
                terminals: vec![
                    terminal(row, [x, 5.0], &format!("U1.{index}")),
                    terminal(row + 1, [*far, 9.0], &format!("J{index}.1")),
                ],
            });
        }
        // The part's other row, under the wall (no routes there): the part's
        // centre lies below the row, so the row leaves upwards.
        for index in 0..6 {
            let x = 4.0 + 0.5 * index as f64;
            obstacles.push(obstacle(
                rectangle(x - 0.15, 0.5, x + 0.15, 1.5),
                Some(nets.len() as NetId),
                ObstacleKind::Copper,
                1,
                &format!("U1.{}", index + 6),
            ));
            nets.push(Net {
                name: format!("D{index}"),
                class: 0,
                terminals: vec![terminal(
                    obstacles.len() - 1,
                    [x, 1.0],
                    &format!("U1.{}", index + 6),
                )],
            });
        }
        let walls = [(0.0, 0.0, 11.0, 4.3), (0.0, 9.4, 11.0, 10.0)];
        for (x0, y0, x1, y1) in walls {
            obstacles.push(obstacle(
                rectangle(x0, y0, x1, y1),
                None,
                ObstacleKind::Keepout,
                3,
                "wall",
            ));
        }
        let board = Board {
            layer_count: 2,
            outline: vec![[0.0, 0.0], [11.0, 0.0], [11.0, 10.0], [0.0, 10.0]],
            edge_clearance: 0.1,
            hole_clearance: 0.25,
            hole_to_hole: 0.25,
            classes: vec![class],
            neck_width: 0.2,
            obstacles,
            nets,
            planes: Vec::new(),
            solid_pads: Vec::new(),
            isolated_pads: Vec::new(),
            solid_pad_obstacles: Vec::new(),
            isolated_pad_obstacles: Vec::new(),
            pad_spokes: Default::default(),
            merged_pads: Vec::new(),
            no_via_pads: Vec::new(),
            fill_passes: Vec::new(),
            pin_row_gaps: Vec::new(),
            terminal_copper: Vec::new(),
            attachment: Default::default(),
            via_spans: Vec::new(),
        };
        let config = Config {
            pitches: vec![0.1],
            ..Config::default()
        };
        Router::new(&board, &config)
    }

    /// Lattice nodes on the top from `points[0]` through the others, by
    /// axis and diagonal steps.
    fn walk(router: &Router, points: &[[f64; 2]]) -> Vec<Node> {
        let mut at = router.grid.xy(router.grid.nearest_node(points[0]).unwrap());
        let mut nodes = vec![Node {
            layer: 0,
            cell: router.grid.index(at.0, at.1) as u32,
        }];
        for point in &points[1..] {
            let to = router.grid.xy(router.grid.nearest_node(*point).unwrap());
            while at != to {
                at = (
                    (at.0 as i64 + (to.0 as i64 - at.0 as i64).signum()) as usize,
                    (at.1 as i64 + (to.1 as i64 - at.1 as i64).signum()) as usize,
                );
                nodes.push(Node {
                    layer: 0,
                    cell: router.grid.index(at.0, at.1) as u32,
                });
            }
        }
        nodes
    }

    /// The row routed as the negotiation might leave it: N3 leaves U1.3 up
    /// and turns left across U1.2's tip (0.4 mm beyond it) to its far pad,
    /// the outer nets are routed hard, and N2, boxed in, stays open.
    fn routed_with_n2_open(router: &mut Router) {
        let nodes = walk(
            router,
            &[[5.5, 5.0], [5.5, 5.9], [5.0, 5.9], [4.0, 6.9], [4.0, 9.0]],
        );
        let branch = Branch {
            nodes,
            start_terminal: 0,
            end_terminal: 1,
        };
        assert!(router.branch_is_legal(OTHER, &branch));
        let state = &mut router.nets[OTHER as usize];
        state.branches = vec![branch];
        state.connected = vec![true, true];
        state.complete = true;
        router.stamp(OTHER);
        for net in [1, 0, 4, 5, OPEN] {
            router.route_net_seq(net, 0.0, true, 4.0);
            router.stamp(net);
        }
        for net in [0, 1, 3, 4, 5] {
            assert!(router.nets[net].complete, "N{net} routed");
        }
        assert!(!router.nets[OPEN as usize].complete, "N2 boxed in");
        for net in 0..6 {
            assert!(router.conflicts(net).is_empty());
        }
    }

    /// N2 and N3 cross at the row: the census's facts name the pair. N2's
    /// pad is open, N3's pad is its row neighbour, their targets cross, and
    /// N3's track closes one of N2's straight ways out at its tip.
    #[test]
    fn the_census_finds_the_crossing_pair() {
        let mut router = crossing_row();
        routed_with_n2_open(&mut router);
        let plan = DivePlan::new(router.fanout_plan());
        router.crossing_census(&plan, &[0, 1, 2, 3, 4, 5], "test");
        let open: Vec<usize> = plan.by_net[&OPEN]
            .iter()
            .copied()
            .filter(|index| router.pad_open(&plan.plan.pads[*index]))
            .collect();
        assert_eq!(open.len(), 1);
        let pad = &plan.plan.pads[open[0]];
        let crossing: Vec<usize> = router
            .row_neighbours(&plan, open[0])
            .into_iter()
            .filter(|other| plan.plan.pads[*other].net == OTHER)
            .filter(|other| router.targets_cross(pad, &plan.plan.pads[*other]))
            .collect();
        assert_eq!(crossing.len(), 1);
        let mut blocks = Vec::new();
        router.tip_blocks(pad, &mut std::iter::once(OTHER), &mut blocks);
        assert!(!blocks.is_empty(), "N3 closes a way out of U1.2");
    }
}

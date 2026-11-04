// Copyright (C) 2026 Toit contributors.

//! Pads without a way out (`Config::pad_exits`, off by default;
//! docs/placer.md "Pads without a way out (2026-11-04)"): a pad whose
//! every straight way out is closed by fixed copper, a keepout or the
//! board edge around it can be connected by no route. Where another
//! part's copper or the edge closes it, the placement is to blame and only
//! moving or turning a part opens it; where the pad's own part alone
//! walls it in, the footprint is (no placement opens it).
//!
//! The test is the router's (pcb-router's `ExitProbe`, the fan-out's ways
//! out on the statics at the class width): the placer asks it through a
//! session (`ExitSession`) of the caller, which lowers the placement as
//! the layout routes it. After a seed's placement, `repair` asks it of
//! every part; for each part with a pad another part or the edge closes,
//! it tries the part and the movable parts that close the pad at their
//! other orientations and at nearby legal spots, the cheapest in
//! wirelength first, and keeps the first that leaves fewer closed pads
//! around it and the part's relations no worse.

use crate::problem::{Pose, Problem};
use std::sync::Arc;

/// A pad with no way out, as the session reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct ClosedPad {
    /// The pad's part (component index) and its label (`REF.PAD`), net and
    /// copper layer (the board's index).
    pub part: usize,
    pub pad: String,
    pub net: String,
    pub layer: usize,
    /// What closes its ways, other parts' first (labels: `REF.PAD`, a
    /// keepout's, `the board edge`).
    pub blocked_by: Vec<String>,
    /// The other parts among them (component indices).
    pub blockers: Vec<usize>,
    /// Only the pad's own part closes it (a footprint's doing).
    pub own_only: bool,
}

/// One session's answer for `parts` at `poses`: their closed pads and the
/// placement work it took. `None`: the session cannot answer (a lowering
/// that failed); the placement is then left as it is.
pub type ExitSession = Box<dyn FnMut(&[Pose], &[usize]) -> Option<(Vec<ClosedPad>, u64)> + Send>;
/// Opens a session (each keeps its own lowered board).
pub type ExitFactory = dyn Fn() -> ExitSession + Send + Sync;

/// The switch's settings.
#[derive(Clone)]
pub struct PadExits {
    pub factory: Arc<ExitFactory>,
    /// How far (mm) a part may move from its spot to open a pad.
    pub reach: f64,
    /// Candidate poses a part asks the session about, the cheapest first.
    pub candidates: usize,
}

impl std::fmt::Debug for PadExits {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PadExits")
            .field("reach", &self.reach)
            .field("candidates", &self.candidates)
            .finish()
    }
}

/// A part `repair` moved, and the pads that opened.
#[derive(Clone, Debug, PartialEq)]
pub struct ExitMove {
    pub part: usize,
    pub from: Pose,
    pub to: Pose,
    pub opened: Vec<String>,
}

/// What `repair` found and did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExitOutcome {
    /// The pads still closed (`own_only` ones included).
    pub closed: Vec<ClosedPad>,
    pub moves: Vec<ExitMove>,
    /// The pads closed before the repair.
    pub closed_before: usize,
    /// The repair's work in all (placement units: the session's, the
    /// candidates' legality), booked apart from the seed's (`place` takes
    /// it back from the seed's count), and the candidates tried.
    pub work: u64,
    pub tried: usize,
}

/// The closed pads another part or the edge closes.
fn placement_closed(pads: &[ClosedPad]) -> usize {
    pads.iter().filter(|pad| !pad.own_only).count()
}

/// The weighted half-perimeter of the nets of `part` (`members`: per
/// net, its (component, pin)).
fn part_wirelength(
    problem: &Problem,
    members: &[Vec<(usize, usize)>],
    nets: &[usize],
    poses: &[Pose],
) -> f64 {
    nets.iter()
        .map(|&net| {
            let mut bounds = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for &(component, pin) in &members[net] {
                let component_ref = &problem.components[component];
                let at = component_ref.pin_position(&component_ref.pins[pin], poses[component]);
                bounds[0] = bounds[0].min(at[0]);
                bounds[1] = bounds[1].min(at[1]);
                bounds[2] = bounds[2].max(at[0]);
                bounds[3] = bounds[3].max(at[1]);
            }
            problem.net_weights.get(net).copied().unwrap_or(1.0)
                * ((bounds[2] - bounds[0]) + (bounds[3] - bounds[1]))
        })
        .sum()
}

/// Placement work a candidate's legality and wirelength take, a pin.
const CANDIDATE_PIN_WORK: u64 = 40;

/// Asks the session about every placed part, then moves or turns parts
/// so that fewer pads stay closed (see the module). `unplaced` parts are
/// neither asked about nor moved. `None`: the session could not answer.
pub fn repair(
    problem: &Problem,
    poses: &mut [Pose],
    unplaced: &[usize],
    exits: &PadExits,
) -> Option<ExitOutcome> {
    let count = problem.components.len();
    let mut session = (exits.factory)();
    let placed: Vec<usize> = (0..count).filter(|part| !unplaced.contains(part)).collect();
    let (closed, work) = session(poses, &placed)?;
    crate::add_work(work);
    let mut outcome = ExitOutcome {
        closed_before: closed.len(),
        ..Default::default()
    };
    // Per part, its closed pads now.
    let mut by_part: std::collections::BTreeMap<usize, Vec<ClosedPad>> = Default::default();
    for pad in closed {
        by_part.entry(pad.part).or_default().push(pad);
    }
    let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); problem.net_weights.len()];
    for (component, part) in problem.components.iter().enumerate() {
        for (pin, record) in part.pins.iter().enumerate() {
            if let Some(entry) = members.get_mut(record.net) {
                entry.push((component, pin));
            }
        }
    }
    let followers = &problem.constraints.followers;
    let free = |part: usize| {
        !problem.components[part].fixed
            && !unplaced.contains(&part)
            && !followers
                .iter()
                .any(|follower| follower.part == part || follower.leader == part)
    };
    let defective: Vec<usize> = by_part
        .iter()
        .filter(|(_, pads)| placement_closed(pads) > 0)
        .map(|(part, _)| *part)
        .collect();
    for part in defective {
        let Some(pads) = by_part.get(&part) else {
            continue;
        };
        if placement_closed(pads) == 0 {
            continue;
        }
        // The movers: the part, then the other parts that close its pads.
        let mut movers: Vec<usize> = Vec::new();
        if free(part) {
            movers.push(part);
        }
        for pad in pads.iter().filter(|pad| !pad.own_only) {
            for &blocker in &pad.blockers {
                if blocker != part && free(blocker) && !movers.contains(&blocker) {
                    movers.push(blocker);
                }
            }
        }
        // Candidates of every mover: (cost, distance, mover, pose).
        let mut candidates: Vec<(f64, f64, usize, Pose)> = Vec::new();
        for &mover in &movers {
            let component = &problem.components[mover];
            let nets: Vec<usize> = {
                let mut nets: Vec<usize> = component.pins.iter().map(|pin| pin.net).collect();
                nets.sort_unstable();
                nets.dedup();
                nets
            };
            let pins: u64 = nets.iter().map(|net| members[*net].len() as u64).sum();
            let original = poses[mover];
            let before = part_wirelength(problem, &members, &nets, poses);
            let penalty_before = crate::legal::relation_penalty_counted(problem, poses, mover);
            let center = component.center(original);
            let step = problem.grid.max(0.05);
            let rings = (exits.reach / step).ceil().max(0.0) as i64;
            let angles: Vec<f64> = if component.angle_options.is_empty() {
                vec![original.angle]
            } else {
                component.angle_options.clone()
            };
            let mut seen: Vec<Pose> = Vec::new();
            for &angle in &angles {
                for ring in 0..=rings {
                    for dy in -ring..=ring {
                        for dx in -ring..=ring {
                            if dx.abs().max(dy.abs()) != ring {
                                continue;
                            }
                            let moved_center =
                                [center[0] + dx as f64 * step, center[1] + dy as f64 * step];
                            let position = crate::constraints::snap_position(
                                problem,
                                mover,
                                component.position_for_center(moved_center, angle),
                                angle,
                            );
                            let pose = Pose { position, angle };
                            if pose == original || seen.contains(&pose) {
                                continue;
                            }
                            seen.push(pose);
                            if !crate::legal::is_legal(problem, poses, mover, pose, 0..count) {
                                continue;
                            }
                            crate::add_work(CANDIDATE_PIN_WORK * pins);
                            poses[mover] = pose;
                            let after = part_wirelength(problem, &members, &nets, poses);
                            let penalty =
                                crate::legal::relation_penalty_counted(problem, poses, mover);
                            poses[mover] = original;
                            if penalty > penalty_before + 1.0e-9 {
                                continue;
                            }
                            let distance = (dx.abs() + dy.abs()) as f64 * step;
                            // To a micrometre of wire: equal costs go to the
                            // nearest pose, not to rounding.
                            let cost = ((after - before) * 1.0e3).round() / 1.0e3;
                            candidates.push((cost, distance, mover, pose));
                        }
                    }
                }
            }
        }
        candidates.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        });
        // The cheapest pose of each mover and orientation first (a turn
        // is tried even where many cheaper shifts are: a pad facing a
        // neighbour stays closed under small shifts), then the rest.
        let mut firsts = Vec::new();
        let mut rest = Vec::new();
        let mut seen_turns: Vec<(usize, i64)> = Vec::new();
        for candidate in candidates {
            let key = (
                candidate.2,
                candidate.3.angle.rem_euclid(360.0).round() as i64,
            );
            if seen_turns.contains(&key) {
                rest.push(candidate);
            } else {
                seen_turns.push(key);
                firsts.push(candidate);
            }
        }
        firsts.extend(rest);
        let candidates = firsts;
        for &(_, _, mover, pose) in candidates.iter().take(exits.candidates) {
            outcome.tried += 1;
            let original = poses[mover];
            // The parts whose pads the move can open or close: the part,
            // the mover and the placed parts near either of its spots.
            let reach = exits.reach + 3.0;
            let spots = [
                problem.components[mover].center(original),
                problem.components[mover].center(pose),
            ];
            let near: Vec<usize> = placed
                .iter()
                .copied()
                .filter(|other| {
                    *other == part
                        || *other == mover
                        || spots.iter().any(|spot| {
                            let component = &problem.components[*other];
                            let at = component.center(poses[*other]);
                            let half = component.half_extent(poses[*other].angle);
                            (at[0] - spot[0]).abs() <= half[0] + reach
                                && (at[1] - spot[1]).abs() <= half[1] + reach
                        })
                })
                .collect();
            let before: usize = near
                .iter()
                .filter_map(|other| by_part.get(other))
                .map(|pads| placement_closed(pads))
                .sum();
            poses[mover] = pose;
            let Some((answer, work)) = session(poses, &near) else {
                poses[mover] = original;
                break;
            };
            crate::add_work(work);
            let after = placement_closed(&answer);
            if after < before {
                let still: Vec<&str> = answer.iter().map(|pad| pad.pad.as_str()).collect();
                let opened: Vec<String> = near
                    .iter()
                    .filter_map(|other| by_part.get(other))
                    .flatten()
                    .filter(|pad| !pad.own_only && !still.contains(&pad.pad.as_str()))
                    .map(|pad| pad.pad.clone())
                    .collect();
                for other in &near {
                    by_part.remove(other);
                }
                for pad in answer {
                    by_part.entry(pad.part).or_default().push(pad);
                }
                outcome.moves.push(ExitMove {
                    part: mover,
                    from: original,
                    to: pose,
                    opened,
                });
                break;
            }
            poses[mover] = original;
        }
    }
    outcome.closed = by_part.into_values().flatten().collect();
    Some(outcome)
}

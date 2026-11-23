// Copyright (C) 2026 Florian Loitsch.

//! A projected starting pose for one movable component in a signal graph.

use super::{CG_ITERATIONS, Point, Problem, conjugate_gradients};
use crate::problem::{Pin, Pose, Side};
use std::collections::{BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug)]
pub struct SignalAnchorSeed {
    pub part: usize,
    pub estimate: Point,
    pub pose: Pose,
}

struct Contact<'a> {
    part: usize,
    pin: &'a Pin,
}

struct Link<'a> {
    contacts: [&'a Contact<'a>; 2],
    weight: f64,
}

/// Estimate one well-connected part's origin and orientation from the
/// sparse signal graph. The core's same-board legalizer then seats it
/// against held bodies and the outline. The result is an initial pose
/// only. The caller runs the full placement with the part still movable.
pub fn signal_anchor_seed(problem: &Problem, budget: u64) -> (Option<SignalAnchorSeed>, u64) {
    let count = problem.components.len();
    if budget == 0 || count > 1024 {
        return (None, 0);
    }
    let mut nets: Vec<Vec<Contact<'_>>> =
        (0..problem.net_weights.len()).map(|_| Vec::new()).collect();
    for (part, component) in problem.components.iter().enumerate() {
        for pin in &component.pins {
            nets[pin.net].push(Contact { part, pin });
        }
    }
    let mut signal_degree = vec![0; count];
    let mut links = Vec::new();
    for contacts in nets
        .iter()
        .filter(|contacts| (2..=8).contains(&contacts.len()))
    {
        let parts: BTreeSet<_> = contacts.iter().map(|contact| contact.part).collect();
        if parts.len() < 2 {
            continue;
        }
        for part in parts {
            signal_degree[part] += 1;
        }
        let weight = 1.0 / (contacts.len() - 1) as f64;
        for (index, a) in contacts.iter().enumerate() {
            links.extend(
                contacts[..index]
                    .iter()
                    .filter(|b| a.part != b.part)
                    .map(|b| Link {
                        contacts: [a, b],
                        weight,
                    }),
            );
        }
    }
    let mut work = (count + links.len()) as u64;
    let mut adjacent = vec![Vec::new(); count];
    for link in &links {
        let [a, b] = link.contacts.map(|contact| contact.part);
        adjacent[a].push(b);
        adjacent[b].push(a);
    }
    let mut anchored: Vec<_> = problem.components.iter().map(|part| part.fixed).collect();
    let mut queue: VecDeque<_> = (0..count).filter(|&part| anchored[part]).collect();
    while let Some(part) = queue.pop_front() {
        for &next in &adjacent[part] {
            if !anchored[next] {
                anchored[next] = true;
                queue.push_back(next);
            }
        }
    }
    let candidate = (0..count)
        .filter(|&part| {
            let component = &problem.components[part];
            anchored[part]
                && !component.fixed
                && signal_degree[part] >= 16
                && matches!(component.side, Side::Front | Side::Back)
                && !problem
                    .constraints
                    .followers
                    .iter()
                    .any(|follower| follower.part == part || follower.leader == part)
        })
        .max_by_key(|&part| {
            (
                signal_degree[part],
                problem.components[part].pins.len(),
                std::cmp::Reverse(part),
            )
        });
    let Some(part) = candidate else {
        return (None, work);
    };
    let links: Vec<_> = links
        .into_iter()
        .filter(|link| anchored[link.contacts[0].part])
        .collect();
    let mut diagonal = vec![0.0; count];
    let free = |owner: usize| anchored[owner] && !problem.components[owner].fixed;
    let mut edges = Vec::new();
    for link in &links {
        let [a, b] = link.contacts.map(|contact| contact.part);
        if free(a) {
            diagonal[a] += link.weight;
        }
        if free(b) {
            diagonal[b] += link.weight;
        }
        if free(a) && free(b) {
            edges.push((a, b, link.weight));
        }
    }
    for (owner, element) in diagonal.iter_mut().enumerate() {
        if !free(owner) {
            *element = 1.0;
        }
    }
    let component = &problem.components[part];
    // One initial matrix product and at most CG_ITERATIONS more per axis.
    let solve_ceiling = (2 * (CG_ITERATIONS + 1) * (count + 2 * edges.len())) as u64;
    let mut estimates = Vec::new();
    for &angle in component.angle_options.iter().take(4) {
        if solve_ceiling > budget.saturating_sub(work) {
            break;
        }
        let offset = |contact: &Contact<'_>| -> Point {
            let owner = contact.part;
            if problem.components[owner].fixed {
                problem.components[owner].pin_position(contact.pin, problem.poses[owner])
            } else if owner == part {
                component.pin_position(
                    contact.pin,
                    Pose {
                        position: [0.0, 0.0],
                        angle,
                    },
                )
            } else {
                [0.0, 0.0]
            }
        };
        let mut solved = [vec![0.0; count], vec![0.0; count]];
        for (axis, values) in solved.iter_mut().enumerate() {
            let mut rhs = vec![0.0; count];
            for link in &links {
                let [a, b] = link.contacts;
                let delta = offset(b)[axis] - offset(a)[axis];
                if free(a.part) {
                    rhs[a.part] += link.weight * delta;
                }
                if free(b.part) {
                    rhs[b.part] -= link.weight * delta;
                }
            }
            work += conjugate_gradients(&diagonal, &edges, &rhs, values);
        }
        let residual = links
            .iter()
            .map(|link| {
                let [a, b] = link.contacts;
                let (a_offset, b_offset) = (offset(a), offset(b));
                let delta: Point = std::array::from_fn(|axis| {
                    solved[axis][a.part] + a_offset[axis] - solved[axis][b.part] - b_offset[axis]
                });
                link.weight * (delta[0] * delta[0] + delta[1] * delta[1])
            })
            .sum::<f64>();
        let estimate = [solved[0][part], solved[1][part]];
        if residual.is_finite() && estimate.iter().all(|value| value.is_finite()) {
            estimates.push((
                residual,
                Pose {
                    position: estimate,
                    angle,
                },
            ));
        }
    }
    estimates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let absent: Vec<_> = (0..count)
        .map(|owner| owner != part && !problem.components[owner].fixed)
        .collect();
    let mut geometry = crate::skeleton::skeleton_problem(
        problem,
        &absent,
        &[],
        &crate::skeleton::Settings {
            room: 0.0,
            join_pins: 0,
            ..Default::default()
        },
        &mut crate::skeleton::Report::default(),
    );
    // Project to the actual body/clearance model at its fine grid. Halos
    // and the additional spacing are routing preferences, which the full
    // placement will apply to the original problem again. The projection
    // has no other movable bodies to assess escape demand or crossings.
    geometry.grid = geometry.grid.min(0.1);
    geometry.spacing = geometry.min_spacing;
    geometry.escape = None;
    geometry.crossings = None;
    for component in &mut geometry.components {
        component.halo = 0.0;
    }
    // Test each angle's nearest legal pose without rotating it to a
    // different, unscored orientation during the projection.
    let max_displacement = component.body_size[0].max(component.body_size[1]);
    for (_, pose) in estimates {
        let remaining = budget.saturating_sub(work);
        if remaining == 0 {
            break;
        }
        geometry.components[part].angle_options = vec![pose.angle];
        let mut poses = geometry.poses.clone();
        poses[part] = pose;
        let started = crate::work();
        let unplaced =
            crate::legal::legalize_keeping_bounded(&geometry, &mut poses, &[], remaining);
        work += crate::work().saturating_sub(started);
        let projected = poses[part];
        let legal = !unplaced.contains(&part)
            && !crate::legal::illegal_components(&geometry, &poses).contains(&part);
        let displacement = (projected.position[0] - pose.position[0])
            .hypot(projected.position[1] - pose.position[1]);
        if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
            eprintln!(
                "signal seed {}: {:?} to {:?}, legal {legal}, displacement {displacement:.3} mm",
                component.name, pose, projected,
            );
        }
        if legal && displacement <= max_displacement {
            return (
                Some(SignalAnchorSeed {
                    part,
                    estimate: pose.position,
                    pose: projected,
                }),
                work,
            );
        }
    }
    (None, work)
}

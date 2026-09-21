// Copyright (C) 2026 Toit contributors.

//! Bounded joint translation proposals from observed separation constraints.
//! A contact branch can be inconsistent even when another placement is legal.
//! Such failures roll back; they never establish placement infeasibility.
use super::*;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoupledLegalizationConfig {
    pub start_sweep: usize,
    pub maximum_rounds: usize,
    pub maximum_linear_solves: usize,
    pub maximum_rows: usize,
    pub maximum_active_rows: usize,
    /// Additional contact-direction proposals; zero preserves the original solve.
    #[serde(skip_serializing_if = "usize_is_zero")]
    pub maximum_branch_trials: usize,
}

impl Default for CoupledLegalizationConfig {
    fn default() -> Self {
        Self {
            start_sweep: 128,
            maximum_rounds: 16,
            maximum_linear_solves: 4096,
            maximum_rows: 2048,
            maximum_active_rows: 256,
            maximum_branch_trials: 0,
        }
    }
}

impl CoupledLegalizationConfig {
    pub(super) fn check(&self) -> Result<(), String> {
        if self.start_sweep == 0
            || self.maximum_rounds == 0
            || self.maximum_linear_solves == 0
            || self.maximum_rows == 0
            || self.maximum_active_rows == 0
        {
            return Err("coupled legalization budgets must be positive".into());
        }
        if self.maximum_branch_trials > 256 {
            return Err("coupled contact branch trials must not exceed 256".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CoupledLegalizationFrame {
    pub round: usize,
    pub status: &'static str,
    pub rows: usize,
    pub active_rows: usize,
    pub linear_solves: usize,
    #[serde(skip_serializing_if = "usize_is_zero")]
    pub branch_trial: usize,
}

#[derive(Clone)]
struct Row {
    terms: Vec<(usize, f64)>,
    rhs: f64,
}

fn row_dot(row: &Row, x: &[f64]) -> f64 {
    row.terms.iter().map(|&(i, value)| value * x[i]).sum()
}

fn gram(a: &Row, b: &Row) -> f64 {
    a.terms
        .iter()
        .map(|&(i, x)| {
            b.terms
                .iter()
                .filter(|&&(j, _)| j == i)
                .map(|&(_, y)| x * y)
                .sum::<f64>()
        })
        .sum()
}

fn solve(mut matrix: Vec<Vec<f64>>, mut rhs: Vec<f64>) -> Option<Vec<f64>> {
    let n = rhs.len();
    for k in 0..n {
        let pivot = (k..n).max_by(|&i, &j| matrix[i][k].abs().total_cmp(&matrix[j][k].abs()))?;
        if matrix[pivot][k].abs() < 1e-10 {
            return None;
        }
        matrix.swap(k, pivot);
        rhs.swap(k, pivot);
        for i in k + 1..n {
            let factor = matrix[i][k] / matrix[k][k];
            for j in k + 1..n {
                matrix[i][j] -= factor * matrix[k][j];
            }
            rhs[i] -= factor * rhs[k];
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        x[i] = (rhs[i] - (i + 1..n).map(|j| matrix[i][j] * x[j]).sum::<f64>()) / matrix[i][i];
        if !x[i].is_finite() {
            return None;
        }
    }
    Some(x)
}

/// Active-set dual of min ||x||²/2 subject to A*x >= b. Rows remain sparse;
/// only the bounded active Gram matrix is dense. Verified dependent normals
/// permit a bounded dual pivot; uncertain numerical systems are rejected.
#[cfg(test)]
fn project(
    rows: &[Row],
    dimensions: usize,
    config: &CoupledLegalizationConfig,
    solves: &mut usize,
) -> Result<(Vec<f64>, usize), &'static str> {
    project_with_conflicts(rows, dimensions, config, solves, &mut Vec::new())
}

fn project_with_conflicts(
    rows: &[Row],
    dimensions: usize,
    config: &CoupledLegalizationConfig,
    solves: &mut usize,
    conflicting: &mut Vec<usize>,
) -> Result<(Vec<f64>, usize), &'static str> {
    conflicting.clear();
    let mut multipliers = vec![0.0; rows.len()];
    let mut active: Vec<usize> = Vec::new();
    let mut x = vec![0.0; dimensions];
    for _ in 0..config.maximum_linear_solves {
        x.fill(0.0);
        for &i in &active {
            for &(j, value) in &rows[i].terms {
                x[j] += multipliers[i] * value;
            }
        }
        let violations: Vec<_> = rows.iter().map(|row| row.rhs - row_dot(row, &x)).collect();
        if violations.iter().all(|&v| v <= 1e-8) {
            return Ok((x, active.len()));
        }
        let entering = (0..rows.len())
            .filter(|i| !active.contains(i))
            .max_by(|&i, &j| violations[i].total_cmp(&violations[j]).then(i.cmp(&j)))
            .ok_or("inconsistent_active_rows")?;
        if violations[entering] <= 1e-8 {
            return Err("inconsistent_active_rows");
        }
        if active.len() >= config.maximum_active_rows {
            return Err("active_row_budget");
        }
        active.push(entering);
        loop {
            if *solves >= config.maximum_linear_solves {
                return Err("linear_solve_budget");
            }
            *solves += 1;
            let matrix = active
                .iter()
                .map(|&i| active.iter().map(|&j| gram(&rows[i], &rows[j])).collect())
                .collect();
            let Some(z) = solve(matrix, active.iter().map(|&i| rows[i].rhs).collect()) else {
                conflicting.clone_from(&active);
                // A dependent entering normal does not make the inequalities
                // infeasible: its equality can replace an older active row.
                // The extra dependency solve charges the same shared budget;
                // every pivot is followed by another charged active solve.
                if *solves >= config.maximum_linear_solves {
                    return Err("linear_solve_budget");
                }
                *solves += 1;
                let (&last, previous) = active.split_last().unwrap();
                let matrix = previous
                    .iter()
                    .map(|&i| previous.iter().map(|&j| gram(&rows[i], &rows[j])).collect())
                    .collect();
                let products = previous
                    .iter()
                    .map(|&i| gram(&rows[i], &rows[last]))
                    .collect();
                let weights =
                    solve(matrix, products).ok_or("singular_or_inconsistent_active_rows")?;
                // Check the normal identity directly, not through subtracting
                // nearly equal squared Gram norms. Uncertain dependence is a
                // numerical failure, never an infeasibility certificate.
                let mut residual = vec![0.0; dimensions];
                let mut scale = vec![0.0; dimensions];
                for &(j, value) in &rows[last].terms {
                    residual[j] += value;
                    scale[j] += value.abs();
                }
                for (&i, &weight) in previous.iter().zip(&weights) {
                    for &(j, value) in &rows[i].terms {
                        let contribution = weight * value;
                        residual[j] -= contribution;
                        scale[j] += contribution.abs();
                    }
                }
                if residual.iter().zip(&scale).any(|(&value, &scale)| {
                    !value.is_finite()
                        || !scale.is_finite()
                        || value.abs() > 128.0 * f64::EPSILON * (1.0 + scale)
                }) {
                    return Err("singular_or_inconsistent_active_rows");
                }
                *conflicting = previous
                    .iter()
                    .zip(&weights)
                    .filter_map(|(&i, &weight)| (weight != 0.0).then_some(i))
                    .chain(std::iter::once(last))
                    .collect();
                let mut slope = rows[last].rhs;
                let mut slope_scale = slope.abs();
                for (&i, &weight) in previous.iter().zip(&weights) {
                    let contribution = weight * rows[i].rhs;
                    slope -= contribution;
                    slope_scale += contribution.abs();
                }
                if !slope.is_finite()
                    || !slope_scale.is_finite()
                    || slope <= 1e-8 + 128.0 * f64::EPSILON * (1.0 + slope_scale)
                {
                    return Err("singular_or_inconsistent_active_rows");
                }
                // Null dual direction (-weights, 1) preserves the primal x.
                // Advance until an old nonnegative multiplier reaches zero.
                let leaving = previous
                    .iter()
                    .zip(&weights)
                    .enumerate()
                    .filter_map(|(j, (&i, &weight))| {
                        (weight > 0.0).then(|| (multipliers[i] / weight, j))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                let Some((step, leaving)) = leaving else {
                    // With no decreasing multiplier the verified null direction
                    // proves this linear contact branch incompatible. Another
                    // contact direction or placement can still be feasible.
                    return Err("infeasible_active_inequalities");
                };
                if !step.is_finite() || step < 0.0 {
                    return Err("invalid_dual_step");
                }
                for (&i, &weight) in previous.iter().zip(&weights) {
                    let change = step * weight;
                    let value = multipliers[i] - change;
                    let guard = 128.0 * f64::EPSILON * (1.0 + multipliers[i].abs() + change.abs());
                    if !value.is_finite() || value < -guard {
                        return Err("invalid_dual_step");
                    }
                    multipliers[i] = value.max(0.0);
                }
                multipliers[last] += step;
                if !multipliers[last].is_finite() {
                    return Err("invalid_dual_step");
                }
                multipliers[active.remove(leaving)] = 0.0;
                continue;
            };
            if z.iter().all(|&value| value > 0.0) {
                multipliers.fill(0.0);
                for (&i, value) in active.iter().zip(z) {
                    multipliers[i] = value;
                }
                break;
            }
            let leaving = active
                .iter()
                .enumerate()
                .filter_map(|(j, &i)| {
                    (z[j] <= 0.0 && multipliers[i] - z[j] > 0.0)
                        .then(|| (multipliers[i] / (multipliers[i] - z[j]), j))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            if let Some((alpha, leaving)) = leaving {
                for (j, &i) in active.iter().enumerate() {
                    multipliers[i] += alpha * (z[j] - multipliers[i]);
                }
                multipliers[active.remove(leaving)] = 0.0;
            } else {
                let leaving = z
                    .iter()
                    .position(|&value| value <= 0.0)
                    .ok_or("invalid_dual_step")?;
                multipliers[active.remove(leaving)] = 0.0;
            }
        }
    }
    Err("dual_iteration_budget")
}

type ContactKey = (u8, usize, usize, i64, i64);
type Branches = BTreeMap<(usize, usize), (i64, i64)>;

fn axis_key(axis: Vec2) -> (i64, i64) {
    ((axis.x * 1e8).round() as i64, (axis.y * 1e8).round() as i64)
}

fn branch_axis(key: (i64, i64)) -> Vec2 {
    let axis = Vec2::new(key.0 as f64, key.1 as f64);
    scale(axis, 1.0 / vector_length(axis))
}

#[allow(clippy::too_many_arguments)]
fn contact_row(
    a: usize,
    b: usize,
    normal: Vec2,
    residual: f64,
    variables: &[[Option<usize>; 2]],
    candidate: &[PlacementPose],
    original: &[PlacementPose],
) -> Row {
    let mut terms = Vec::new();
    let mut rhs = residual + 2.0 * FEASIBILITY_TOLERANCE_MM;
    for (i, direction) in [(a, normal), (b, scale(normal, -1.0))] {
        for (axis, coefficient) in [direction.x, direction.y].into_iter().enumerate() {
            if coefficient.abs() <= 1e-15 {
                continue;
            }
            if let Some(variable) = variables[i][axis] {
                terms.push((variable, coefficient));
                let delta = sub(candidate[i].position, original[i].position);
                rhs += coefficient * if axis == 0 { delta.x } else { delta.y };
            }
        }
    }
    Row { terms, rhs }
}

#[derive(Clone)]
struct BranchProposal {
    choices: Branches,
    // A ranking surrogate, not a bound on the coupled system's true cost.
    cost: f64,
}

#[allow(clippy::too_many_arguments)]
fn enqueue_branches(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    anchors: &[bool],
    original: &[PlacementPose],
    choices: &Branches,
    conflicting: &[ContactKey],
    pending: &mut Vec<BranchProposal>,
    visited: &std::collections::BTreeSet<Branches>,
    remaining: usize,
    costs: &mut BTreeMap<ContactKey, Option<f64>>,
    pair_checks: &mut usize,
    maximum_pair_checks: usize,
) {
    for &(kind, a, b, old_x, old_y) in conflicting {
        if kind != 0 {
            continue;
        }
        let old = branch_axis((old_x, old_y));
        let axes = [
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            oriented_axes(original[a].rotation_degrees)[0],
            oriented_axes(original[a].rotation_degrees)[1],
            oriented_axes(original[b].rotation_degrees)[0],
            oriented_axes(original[b].rotation_degrees)[1],
            old,
            Vec2::new(-old.y, old.x),
        ];
        for axis in axes {
            for sign in [1.0, -1.0] {
                let direction = axis_key(scale(axis, sign));
                if direction == (old_x, old_y) || choices.get(&(a, b)) == Some(&direction) {
                    continue;
                }
                let key = (0, a, b, direction.0, direction.1);
                if !costs.contains_key(&key) {
                    if *pair_checks >= maximum_pair_checks {
                        return;
                    }
                    *pair_checks += 1;
                    let normal = branch_axis(direction);
                    let mut norm = 0.0;
                    for i in [a, b] {
                        if anchors[i] {
                            continue;
                        }
                        norm += match components[i].constraints.movement {
                            Movement::Fixed => 0.0,
                            Movement::Horizontal => normal.x * normal.x,
                            Movement::Vertical => normal.y * normal.y,
                            Movement::Free => 1.0,
                        };
                    }
                    let cost = if norm <= 1e-15 {
                        None
                    } else {
                        geometry::separation_along_axis(
                            components[a],
                            &original[a],
                            components[b],
                            &original[b],
                            problem.rules.clearance,
                            normal,
                        )
                        .map(|r| (r + 2.0 * FEASIBILITY_TOLERANCE_MM).max(0.0).powi(2) / norm)
                    };
                    costs.insert(key, cost);
                }
                if costs[&key].is_none() {
                    continue;
                }
                let mut next = choices.clone();
                next.insert((a, b), direction);
                if visited.contains(&next) || pending.iter().any(|p| p.choices == next) {
                    continue;
                }
                let cost = next
                    .iter()
                    .map(|(&(a, b), &(x, y))| costs[&(0, a, b, x, y)].unwrap())
                    .sum();
                pending.push(BranchProposal {
                    choices: next,
                    cost,
                });
                pending.sort_by(|a, b| a.cost.total_cmp(&b.cost).then(a.choices.cmp(&b.choices)));
                pending.truncate(remaining);
            }
        }
    }
}

/// First try the unchanged joint solve. Only a failed active contact system
/// permits alternate directional branches. Bounds, fixed anchors, pair checks,
/// linear solves and successful joint rounds are shared across every trial.
#[allow(clippy::too_many_arguments)]
pub(super) fn propose(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    anchors: &[bool],
    poses: &mut [PlacementPose],
    connected_pairs: &[(usize, usize)],
    config: &CoupledLegalizationConfig,
    pair_checks: &mut usize,
    maximum_pair_checks: usize,
    observer: &dyn Fn(&[PlacementPose], usize, CoupledLegalizationFrame),
) -> bool {
    let original = poses.to_vec();
    let mut solves = 0;
    let mut rounds = 0;
    let mut conflicting = Vec::new();
    let mut choices = Branches::new();
    let mut pending = Vec::new();
    let mut visited = std::collections::BTreeSet::new();
    let mut costs = BTreeMap::new();
    for trial in 0..=config.maximum_branch_trials {
        if pair_checks.saturating_add(choices.len()) > maximum_pair_checks {
            break;
        }
        *pair_checks += choices.len();
        visited.insert(choices.clone());
        let observe = |candidate: &[PlacementPose], checks, mut frame: CoupledLegalizationFrame| {
            frame.branch_trial = trial;
            observer(candidate, checks, frame);
        };
        if propose_once(
            problem,
            components,
            anchors,
            poses,
            connected_pairs,
            config,
            pair_checks,
            maximum_pair_checks,
            &observe,
            &choices,
            &mut solves,
            &mut rounds,
            &mut conflicting,
        ) {
            return true;
        }
        debug_assert_eq!(poses, original);
        if trial == config.maximum_branch_trials
            || solves >= config.maximum_linear_solves
            || rounds >= config.maximum_rounds
            || *pair_checks >= maximum_pair_checks
        {
            break;
        }
        enqueue_branches(
            problem,
            components,
            anchors,
            &original,
            &choices,
            &conflicting,
            &mut pending,
            &visited,
            config.maximum_branch_trials - trial,
            &mut costs,
            pair_checks,
            maximum_pair_checks,
        );
        if pending.is_empty() {
            break;
        }
        choices = pending.remove(0).choices;
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn propose_once(
    problem: &Problem,
    components: &[&layout_trace_model::model::Component],
    anchors: &[bool],
    poses: &mut [PlacementPose],
    connected_pairs: &[(usize, usize)],
    config: &CoupledLegalizationConfig,
    pair_checks: &mut usize,
    maximum_pair_checks: usize,
    observer: &dyn Fn(&[PlacementPose], usize, CoupledLegalizationFrame),
    overrides: &BTreeMap<(usize, usize), (i64, i64)>,
    solves: &mut usize,
    rounds: &mut usize,
    conflicting: &mut Vec<ContactKey>,
) -> bool {
    conflicting.clear();
    let original = poses.to_vec();
    let mut candidate = original.clone();
    let mut variables = vec![[None; 2]; components.len()];
    let mut dimensions = 0;
    let mut rows = Vec::new();
    for (i, component) in components.iter().enumerate() {
        let mut allowed = problem.board.bounds;
        if let Some(region) = component.constraints.region {
            allowed.min.x = allowed.min.x.max(region.min.x);
            allowed.min.y = allowed.min.y.max(region.min.y);
            allowed.max.x = allowed.max.x.min(region.max.x);
            allowed.max.y = allowed.max.y.min(region.max.y);
        }
        let (low, high) = geometry::position_limits(problem, component, &poses[i], allowed);
        for axis in 0..2 {
            let enabled = !anchors[i]
                && match component.constraints.movement {
                    Movement::Free => true,
                    Movement::Horizontal => axis == 0,
                    Movement::Vertical => axis == 1,
                    Movement::Fixed => false,
                };
            if !enabled {
                continue;
            }
            let (lo, hi, position) = if axis == 0 {
                (low.x, high.x, poses[i].position.x)
            } else {
                (low.y, high.y, poses[i].position.y)
            };
            variables[i][axis] = Some(dimensions);
            rows.push(Row {
                terms: vec![(dimensions, 1.0)],
                rhs: lo - position,
            });
            rows.push(Row {
                terms: vec![(dimensions, -1.0)],
                rhs: position - hi,
            });
            dimensions += 1;
        }
    }
    let mut contacts = BTreeMap::new();
    for (&(a, b), &direction) in overrides {
        let normal = branch_axis(direction);
        if let Some(residual) = geometry::separation_along_axis(
            components[a],
            &original[a],
            components[b],
            &original[b],
            problem.rules.clearance,
            normal,
        ) {
            contacts.insert(
                (0, a, b, direction.0, direction.1),
                contact_row(a, b, normal, residual, &variables, &original, &original),
            );
        }
    }
    let mut active_rows = 0;
    let mut failure = "round_budget";
    let checks = components
        .len()
        .saturating_mul(components.len().saturating_sub(1))
        / 2
        + connected_pairs.len()
        + overrides.len();
    for _ in 0..=config.maximum_rounds {
        if pair_checks.saturating_add(checks) > maximum_pair_checks {
            failure = "pair_check_budget";
            break;
        }
        *pair_checks += checks;
        let mut found = Vec::new();
        for (a, b) in AllComponentPairs.enumerate(components.len()) {
            if let Some((normal, residual)) = body_overlap_correction(
                components[a],
                &candidate[a],
                components[b],
                &candidate[b],
                problem.rules.clearance,
            ) {
                found.push((0, a, b, normal, residual));
            }
        }
        for &(a, b) in connected_pairs {
            let delta = sub(candidate[a].position, candidate[b].position);
            let length = vector_length(delta);
            let normal = if length > FEASIBILITY_TOLERANCE_MM {
                scale(delta, 1.0 / length)
            } else {
                stable_pair_direction(&components[a].id, &components[b].id)
            };
            let residual = body_support(components[a], &candidate[a], normal)
                + body_support(components[b], &candidate[b], normal)
                + problem.rules.clearance
                - length;
            if residual > FEASIBILITY_TOLERANCE_MM {
                found.push((1, a, b, normal, residual));
            }
        }
        if found.is_empty() {
            let indexes = components
                .iter()
                .enumerate()
                .map(|(i, c)| (c.id.as_str(), i))
                .collect();
            if validate_feasibility(problem, components, &indexes, &candidate).is_err() {
                failure = "hard_constraints";
                break;
            }
            poses.clone_from_slice(&candidate);
            observer(
                poses,
                *pair_checks,
                CoupledLegalizationFrame {
                    round: *rounds,
                    status: "accepted",
                    rows: rows.len() + contacts.len(),
                    active_rows,
                    linear_solves: *solves,
                    branch_trial: 0,
                },
            );
            return true;
        }
        if *rounds == config.maximum_rounds {
            break;
        }
        for (kind, a, b, mut normal, mut residual) in found {
            if kind == 0
                && let Some(&direction) = overrides.get(&(a, b))
            {
                normal = branch_axis(direction);
                residual = geometry::separation_along_axis(
                    components[a],
                    &candidate[a],
                    components[b],
                    &candidate[b],
                    problem.rules.clearance,
                    normal,
                )
                .expect("an observed contact has interacting primitives");
            }
            let direction = axis_key(normal);
            contacts.insert(
                (kind, a, b, direction.0, direction.1),
                contact_row(a, b, normal, residual, &variables, &candidate, &original),
            );
        }
        if rows.len() + contacts.len() > config.maximum_rows {
            failure = "row_budget";
            break;
        }
        let all_rows: Vec<_> = rows
            .iter()
            .cloned()
            .chain(contacts.values().cloned())
            .collect();
        let mut failed_rows = Vec::new();
        let (displacement, count) =
            match project_with_conflicts(&all_rows, dimensions, config, solves, &mut failed_rows) {
                Ok(result) => result,
                Err(reason) => {
                    *conflicting = failed_rows
                        .into_iter()
                        .filter_map(|i| {
                            i.checked_sub(rows.len())
                                .and_then(|j| contacts.keys().nth(j))
                                .copied()
                        })
                        .collect();
                    failure = reason;
                    break;
                }
            };
        *rounds += 1;
        active_rows = count;
        candidate.clone_from_slice(&original);
        for i in 0..components.len() {
            if let Some(j) = variables[i][0] {
                candidate[i].position.x += displacement[j];
            }
            if let Some(j) = variables[i][1] {
                candidate[i].position.y += displacement[j];
            }
        }
        observer(
            &candidate,
            *pair_checks,
            CoupledLegalizationFrame {
                round: *rounds,
                status: "proposal",
                rows: all_rows.len(),
                active_rows,
                linear_solves: *solves,
                branch_trial: 0,
            },
        );
    }
    observer(
        &original,
        *pair_checks,
        CoupledLegalizationFrame {
            round: *rounds,
            status: failure,
            rows: rows.len() + contacts.len(),
            active_rows,
            linear_solves: *solves,
            branch_trial: 0,
        },
    );
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_projection(rows: &[Row], expected: &[f64]) -> usize {
        let mut solves = 0;
        let (actual, _) = project(rows, expected.len(), &Default::default(), &mut solves).unwrap();
        for (actual, expected) in actual.iter().zip(expected) {
            assert!((actual - expected).abs() <= 1e-8, "{actual} != {expected}");
        }
        assert!(
            rows.iter()
                .all(|row| row_dot(row, &actual) >= row.rhs - 1e-8)
        );
        solves
    }

    #[test]
    fn redundant_dependent_row_releases_its_old_multiplier() {
        // The diagonal enters first but is slack at the true optimum.
        for sign in [-1.0, 1.0] {
            let rows = vec![
                Row {
                    terms: vec![(0, 1.0)],
                    rhs: 2.0,
                },
                Row {
                    terms: vec![(1, sign)],
                    rhs: 2.0,
                },
                Row {
                    terms: vec![(0, 1.0), (1, sign)],
                    rhs: 3.0,
                },
            ];
            for order in [
                [0, 1, 2],
                [0, 2, 1],
                [1, 0, 2],
                [1, 2, 0],
                [2, 0, 1],
                [2, 1, 0],
            ] {
                let reordered: Vec<_> = order.iter().map(|&i| rows[i].clone()).collect();
                assert_eq!(assert_projection(&reordered, &[2.0, sign * 2.0]), 5);
            }
        }
    }

    #[test]
    fn dependent_contact_chains_generalize_to_independent_groups() {
        let mut rows = Vec::new();
        for offset in [0, 3] {
            rows.extend([
                Row {
                    terms: vec![(offset, 1.0), (offset + 1, -1.0)],
                    rhs: 2.0,
                },
                Row {
                    terms: vec![(offset + 1, 1.0), (offset + 2, -1.0)],
                    rhs: 2.0,
                },
                Row {
                    terms: vec![(offset, 1.0), (offset + 2, -1.0)],
                    rhs: 3.0,
                },
            ]);
        }
        assert_projection(&rows[..3], &[2.0, 0.0, -2.0]);
        assert_projection(&rows, &[2.0, 0.0, -2.0, 2.0, 0.0, -2.0]);
    }

    #[test]
    fn dependent_incompatible_inequalities_keep_failure_and_support() {
        let rows = vec![
            Row {
                terms: vec![(0, 1.0)],
                rhs: 2.0,
            },
            Row {
                terms: vec![(0, -1.0)],
                rhs: -1.0,
            },
        ];
        let mut solves = 0;
        let mut support = Vec::new();
        assert_eq!(
            project_with_conflicts(&rows, 1, &Default::default(), &mut solves, &mut support),
            Err("infeasible_active_inequalities")
        );
        assert_eq!(support, vec![0, 1]);
        assert_eq!(solves, 3);
        let chain = vec![
            Row {
                terms: vec![(0, 1.0), (1, -1.0)],
                rhs: 2.0,
            },
            Row {
                terms: vec![(1, 1.0), (2, -1.0)],
                rhs: 2.0,
            },
            Row {
                terms: vec![(0, -1.0), (2, 1.0)],
                rhs: -3.0,
            },
        ];
        assert_eq!(
            project(&chain, 3, &Default::default(), &mut 0),
            Err("infeasible_active_inequalities")
        );
    }

    #[test]
    fn uncertain_normal_dependency_is_not_claimed_infeasible() {
        // This feasible near-dependent system has a small independent normal
        // coordinate which the Gram pivot threshold cannot resolve reliably.
        let rows = vec![
            Row {
                terms: vec![(0, 1.0)],
                rhs: 2.0,
            },
            Row {
                terms: vec![(0, -1.0), (1, 1e-6)],
                rhs: -1.0,
            },
        ];
        assert_eq!(
            project(&rows, 2, &Default::default(), &mut 0),
            Err("singular_or_inconsistent_active_rows")
        );
    }

    #[test]
    fn dependency_pivot_charges_the_existing_solve_budget() {
        let rows = vec![
            Row {
                terms: vec![(0, 1.0)],
                rhs: 2.0,
            },
            Row {
                terms: vec![(1, 1.0)],
                rhs: 2.0,
            },
            Row {
                terms: vec![(0, 1.0), (1, 1.0)],
                rhs: 3.0,
            },
        ];
        for limit in [3, 4] {
            let config = CoupledLegalizationConfig {
                maximum_linear_solves: limit,
                ..Default::default()
            };
            let mut solves = 0;
            assert_eq!(
                project(&rows, 2, &config, &mut solves),
                Err("linear_solve_budget")
            );
            assert_eq!(solves, limit);
        }
        assert_eq!(assert_projection(&rows, &[2.0, 2.0]), 5);
        // Independent successful systems keep their existing exact result/work.
        assert_eq!(assert_projection(&rows[..2], &[2.0, 2.0]), 2);
    }

    #[test]
    fn alternate_directions_escape_two_independent_fixed_gaps() {
        let mut data = serde_json::json!({
            "schema_version":1,
            "board":{"bounds":{"min":{"x":0,"y":0},"max":{"x":40,"y":40}}},
            "rules":{"clearance":0.2},"components":[],"nets":[]
        });
        for (id, x, y, size, movement) in [
            ("A", 10.0, 10.0, 4.0, "fixed"),
            ("B", 15.0, 10.0, 4.0, "fixed"),
            ("C", 10.0, 25.0, 4.0, "fixed"),
            ("D", 15.0, 25.0, 4.0, "fixed"),
            ("M", 12.5, 10.0, 2.0, "free"),
            ("N", 12.5, 25.0, 2.0, "free"),
        ] {
            data["components"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "id":id,"position":{"x":x,"y":y},"size":{"x":size,"y":size},
                    "constraints":{"movement":movement,"rotation":"fixed"},"pins":[]
                }));
        }
        let problem: Problem = serde_json::from_value(data).unwrap();
        let components = sorted_components(&problem);
        let original = declared_poses(&problem);
        let anchors = [true, true, true, true, false, false];
        for trials in [0, 1] {
            let mut poses = original.clone();
            let config = CoupledLegalizationConfig {
                maximum_branch_trials: trials,
                ..Default::default()
            };
            assert!(!propose(
                &problem,
                &components,
                &anchors,
                &mut poses,
                &[],
                &config,
                &mut 0,
                5000,
                &|_, _, _| {}
            ));
            assert_eq!(poses, original);
        }
        let config = CoupledLegalizationConfig {
            maximum_branch_trials: 16,
            ..Default::default()
        };
        let mut poses = original.clone();
        let mut checks = 0;
        let frames = std::cell::RefCell::new(Vec::new());
        let accepted = propose(
            &problem,
            &components,
            &anchors,
            &mut poses,
            &[],
            &config,
            &mut checks,
            5000,
            &|_, _, frame| {
                assert!(frame.linear_solves <= config.maximum_linear_solves);
                assert!(frame.round <= config.maximum_rounds);
                assert!(frame.branch_trial <= config.maximum_branch_trials);
                frames.borrow_mut().push(frame);
            },
        );
        assert!(accepted, "{:#?}", frames.borrow());
        assert_eq!(poses[..4], original[..4]);
        assert!(checks <= 5000);
        validate_placement_exclusions(&problem, &poses).unwrap();
        assert!((poses[4].position.y - original[4].position.y).abs() > 3.1);
        assert!((poses[5].position.y - original[5].position.y).abs() > 3.1);

        // Moving only along the trapped axis cannot evade either fixed gap.
        let mut locked = problem.clone();
        for component in &mut locked.components[4..] {
            component.constraints.movement = Movement::Horizontal;
            component.constraints.region = Some(Rect {
                min: Vec2::new(11.0, component.position.y - 2.0),
                max: Vec2::new(14.0, component.position.y + 2.0),
            });
        }
        let mut rejected = original.clone();
        assert!(!propose(
            &locked,
            &sorted_components(&locked),
            &anchors,
            &mut rejected,
            &[],
            &config,
            &mut 0,
            5000,
            &|_, _, _| {}
        ));
        assert_eq!(rejected, original);
    }

    fn fixture() -> Problem {
        serde_json::from_value(serde_json::json!({
            "schema_version":1,"board":{"bounds":{"min":{"x":0,"y":0},"max":{"x":20,"y":12}}},
            "rules":{"clearance":0.2},"components":[
                {"id":"A","position":{"x":2,"y":5},"size":{"x":4,"y":2},
                 "constraints":{"movement":"fixed","rotation":"fixed"},"pins":[]},
                {"id":"B","position":{"x":5,"y":5},"size":{"x":4,"y":2},
                 "constraints":{"movement":"horizontal","rotation":"fixed"},"pins":[]},
                {"id":"C","position":{"x":9.2,"y":5},"size":{"x":4,"y":2},
                 "constraints":{"movement":"horizontal","rotation":"fixed"},"pins":[]}
            ],"nets":[]
        }))
        .unwrap()
    }

    #[test]
    fn new_contacts_join_the_solve_and_limited_trials_roll_back() {
        let problem = fixture();
        let components = sorted_components(&problem);
        let original = declared_poses(&problem);
        let mut candidate = original.clone();
        let anchors = [true, false, false];
        let config = CoupledLegalizationConfig {
            maximum_rounds: 1,
            ..Default::default()
        };
        assert!(!propose(
            &problem,
            &components,
            &anchors,
            &mut candidate,
            &[],
            &config,
            &mut 0,
            100,
            &|_, _, _| {}
        ));
        assert_eq!(candidate, original);
        assert!(propose(
            &problem,
            &components,
            &anchors,
            &mut candidate,
            &[],
            &Default::default(),
            &mut 0,
            100,
            &|_, _, _| {}
        ));
        assert_eq!(candidate[0], original[0]);
        assert!(candidate.iter().all(|pose| pose.position.y == 5.0));
        assert!(candidate[1].position.x > 6.19 && candidate[2].position.x > 10.39);
        validate_placement_exclusions(&problem, &candidate).unwrap();
    }

    #[test]
    fn board_bounds_and_pair_budget_cannot_be_overridden() {
        let mut problem = fixture();
        problem.board.bounds.max.x = 11.0;
        let components = sorted_components(&problem);
        let original = declared_poses(&problem);
        let mut candidate = original.clone();
        assert!(!propose(
            &problem,
            &components,
            &[true, false, false],
            &mut candidate,
            &[],
            &Default::default(),
            &mut 0,
            100,
            &|_, _, _| {}
        ));
        assert_eq!(candidate, original);
        let mut checks = 0;
        assert!(!propose(
            &problem,
            &components,
            &[true, false, false],
            &mut candidate,
            &[],
            &Default::default(),
            &mut checks,
            2,
            &|_, _, _| {}
        ));
        assert_eq!(candidate, original);
        assert_eq!(checks, 0);
    }

    #[test]
    fn chain_moves_together_and_respects_a_fixed_boundary() {
        let mut rows = vec![Row {
            terms: vec![(0, 1.0)],
            rhs: 0.0,
        }];
        rows.extend((0..31).map(|i| Row {
            terms: vec![(i + 1, 1.0), (i, -1.0)],
            rhs: 1.0,
        }));
        let (x, _) = project(&rows, 32, &Default::default(), &mut 0).unwrap();
        for (i, value) in x.iter().enumerate() {
            assert!((value - i as f64).abs() < 1e-8);
        }
    }

    #[test]
    fn active_set_discards_a_constraint_with_negative_multiplier() {
        let rows = vec![
            Row {
                terms: vec![(0, 2.0)],
                rhs: 1.2,
            },
            Row {
                terms: vec![(0, 0.8), (1, 0.6)],
                rhs: 1.1,
            },
        ];
        let (x, _) = project(&rows, 2, &Default::default(), &mut 0).unwrap();
        assert!((x[0] - 0.88).abs() < 1e-8 && (x[1] - 0.66).abs() < 1e-8);
    }

    #[test]
    fn inconsistent_branch_and_work_limits_fail_without_a_solution() {
        let rows = vec![
            Row {
                terms: vec![(0, 1.0)],
                rhs: 1.0,
            },
            Row {
                terms: vec![(0, -1.0)],
                rhs: 1.0,
            },
        ];
        assert!(project(&rows, 1, &Default::default(), &mut 0).is_err());
        let config = CoupledLegalizationConfig {
            maximum_linear_solves: 1,
            ..Default::default()
        };
        let rows = vec![
            Row {
                terms: vec![(0, 1.0)],
                rhs: 1.0,
            },
            Row {
                terms: vec![(1, 1.0)],
                rhs: 1.0,
            },
        ];
        assert!(project(&rows, 2, &config, &mut 0).is_err());
    }
}

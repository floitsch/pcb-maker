// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The start of `place`: the problem it places (`prepare`), what every
//! later stage reads (`Placing`) and the global placement
//! (`Placing::global_stage`).

use crate::{
    Config, Problem, Start, StartStage, anneal, fit_halos, global, groups, mark, orient,
    phase_line, work,
};

/// The problem `place` places: the halos fitted to the board.
pub(crate) struct Fitted<'a> {
    pub(crate) problem: Problem,
    /// The halos' factors (`fit_halos`): the parts with few pins', the
    /// parts with many pins'.
    pub(crate) fit_scale: [f64; 2],
    /// The wirelength at the board's poses.
    pub(crate) wirelength_initial: f64,
    /// A diagnostic start that fits the problem (`Config::start`).
    pub(crate) start: Option<&'a Start>,
}

/// The halos fitted to the board, the linked pairs found, the parts at
/// their canonical angles (`Config::orientation_search`) and at a
/// diagnostic start's poses.
pub(crate) fn prepare<'a>(problem: &Problem, config: &'a Config) -> Fitted<'a> {
    let (mut fitted, fit_scale) = fit_halos(problem, config.maximum_utilization);
    fitted.constraints.link_pairs();
    let wirelength_initial = fitted.wirelength(&fitted.poses);
    if config.orientation_search {
        orient::canonical_start(&mut fitted);
    }
    // A diagnostic start: its parts at its poses, angles included (a
    // panel's copy or a sub-board is a problem of its own: none there).
    let start = config
        .start
        .as_deref()
        .filter(|start| start.poses.len() == fitted.components.len());
    if let Some(start) = start {
        for index in 0..fitted.components.len() {
            let all = start.stage != StartStage::Global;
            if !fitted.components[index].fixed && (all || start.parts[index]) {
                fitted.poses[index] = start.poses[index];
            }
        }
    }
    Fitted {
        problem: fitted,
        fit_scale,
        wirelength_initial,
        start,
    }
}

/// What every stage of `place` reads: the fitted problem, the settings,
/// the stages a diagnostic start skips, the budget and the bus groups.
pub(crate) struct Placing<'a> {
    pub(super) problem: &'a Problem,
    pub(super) config: &'a Config,
    pub(super) fit_scale: [f64; 2],
    pub(super) start: Option<&'a Start>,
    pub(crate) skip_global: bool,
    pub(crate) skip_anneal: bool,
    pub(super) debug: bool,
    pub(super) started: std::time::Instant,
    /// The thread's work when the placement began, and where its budget
    /// ends (`Config::work_limit`).
    pub(super) work_started: u64,
    pub(super) stop_at_work: Option<u64>,
    /// The anneal's settings: the budget's stop and the bus groups.
    pub(crate) anneal_config: anneal::AnnealConfig,
    pub(super) bus_groups: Option<std::sync::Arc<groups::BusGroups>>,
}

impl<'a> Placing<'a> {
    /// The placement of `fitted`'s problem: its budget from now on, and
    /// the bus groups found (once, on the fitted problem: the levels
    /// change halos and spacing, not nets).
    pub(crate) fn new(fitted: &'a Fitted<'a>, config: &'a Config) -> Self {
        let start = fitted.start;
        // The stages a diagnostic start skips.
        let skip_global = start.is_some_and(|start| start.stage != StartStage::Global);
        let skip_anneal =
            config.skip_anneal || start.is_some_and(|start| start.stage == StartStage::Legalize);
        let problem = &fitted.problem;
        let debug = std::env::var_os("PCB_PLACER_DEBUG").is_some();
        let started = std::time::Instant::now();
        let work_started = work();
        let stop_at_work = config.work_limit.map(|limit| work_started + limit);
        let mut anneal_config = config.anneal.clone();
        anneal_config.stop_at_work = stop_at_work.or(anneal_config.stop_at_work);
        // Bus groups: found once, on the fitted problem (the levels change
        // halos and spacing, not nets).
        let bus_groups = config.bus_groups.as_ref().map(|settings| {
            let groups = std::sync::Arc::new(groups::BusGroups::detect(problem, settings));
            if debug {
                let banks: Vec<String> = groups
                    .groups
                    .iter()
                    .filter(|group| group.kind == groups::Kind::Bank)
                    .map(|group| {
                        group
                            .chain
                            .iter()
                            .map(|part| problem.components[*part].name.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect();
                eprintln!(
                    "placer: bus groups: {} banks [{}], {} clusters",
                    banks.len(),
                    banks.join("; "),
                    groups.groups.len() - banks.len()
                );
            }
            groups
        });
        anneal_config.bus_groups = bus_groups.clone();
        Placing {
            problem,
            config,
            fit_scale: fitted.fit_scale,
            start,
            skip_global,
            skip_anneal,
            debug,
            started,
            work_started,
            stop_at_work,
            anneal_config,
            bus_groups,
        }
    }

    /// Whether the caller's deadline or the work budget has come.
    pub(super) fn out_of_budget(&self) -> bool {
        self.config
            .anneal
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
            || self
                .anneal_config
                .stop_at_work
                .is_some_and(|stop| work() >= stop)
    }

    /// The global placement (warm from a diagnostic start's parts, or
    /// skipped for a later one: the problem's poses), then the bus groups'
    /// banks ordered and arranged (`groups::arrange`).
    pub(crate) fn global_stage(&self) -> global::GlobalResult {
        let (problem, config, debug) = (self.problem, self.config, self.debug);
        let global_mark = mark();
        let mut global = match self.start {
            Some(start) if start.stage == StartStage::Global => {
                let mut warm = config.global.clone();
                warm.warm = Some(std::sync::Arc::new(start.parts.clone()));
                global::global_place(problem, &warm)
            }
            _ if self.skip_global => global::GlobalResult {
                poses: problem.poses.clone(),
                frames: Vec::new(),
                iterations: 0,
                overflow: 0.0,
                start: problem.poses.clone(),
                history: Vec::new(),
                stacks: Vec::new(),
            },
            _ => global::global_place(problem, &config.global),
        };
        phase_line(debug, "global", global_mark);
        if debug {
            eprintln!(
                "placer: global {:.1}s",
                self.started.elapsed().as_secs_f64()
            );
        }
        // Bus groups: the banks' order and arrangement (`groups::arrange`).
        if let Some(groups) = self
            .bus_groups
            .as_deref()
            .filter(|groups| groups.settings.arrange && !self.skip_global)
        {
            let arrange_mark = mark();
            let arranged = groups::arrange(problem, groups, &mut global.poses);
            phase_line(debug, "arrange", arrange_mark);
            if debug {
                eprintln!(
                    "placer: arrange: {} banks, {} arranged, cost {:.0} -> {:.0}",
                    arranged.banks, arranged.taken, arranged.cost_before, arranged.cost_after
                );
            }
        }
        global
    }
}

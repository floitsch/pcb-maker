// Copyright (C) 2026 Toit contributors.

//! The machine's work rates: how much work a second of a budget stands for.
//!
//! Every budget is work (search expansions, placement units), so that the
//! same input gives the same board however busy the machine is. The rates
//! that turn seconds into work were the reference's (the 5950X idle:
//! `REFERENCE_EXPANSIONS_PER_SECOND`, `REFERENCE_WORK_PER_SECOND`), so a
//! slower machine (a laptop, a CI runner, the macOS snapshot's, this one
//! under load) spent longer than its budget on the wall clock and the
//! layout's wall guard cut it (exit 3). At its start a command that places
//! or routes measures this machine's rates instead (`calibrate`): a fixed
//! search on a canned lattice on one thread, and a fixed placement, as
//! many copies at once as the placement race's first batch (3), on the
//! process's threads. Each rate is the reference's scaled by the canned
//! workload's speed here against its speed on the reference, so a second
//! of work is about a second of this machine's wall clock.
//!
//! The work counters stay as they are: a run is byte-identical for given
//! rates, and its results depend on the measured rates unless they are
//! pinned (`PCB_WORK_RATE=<expansions per second>[,<placer units per
//! second>]`, or the router settings `work_rate` and `placer_work_rate`),
//! as the tiers pin them at the reference's. Every verdict records the
//! rates applied and measured (docs/router.md, "Determinism and the wall
//! clock").

use pcb_placer as placer;
use pcb_router as core;
use serde::Serialize;

/// The most the wall-clock guards stretch for a pinned rate faster than
/// the machine (`KiCadWorkRate::wall_stretch`): a machine at a quarter of
/// the pinned rate still ends its runs where the work does; a slower one
/// is cut as before, and a run that hangs still ends.
pub const MAX_WALL_STRETCH: f64 = 4.0;

/// The variable that sets another cap than `MAX_WALL_STRETCH` (1: no
/// stretch, the guards as before 2026-10-21).
pub const WALL_STRETCH_VARIABLE: &str = "PCB_WALL_STRETCH_MAX";

/// The variable that pins the rates.
pub const WORK_RATE_VARIABLE: &str = "PCB_WORK_RATE";

/// The canned search's expansions per second on the reference (the 5950X,
/// one thread pinned to a core, load 12, as the search's rate was
/// calibrated: load under 10 at the start, 10 to 17 while it ran):
/// `calibrate-work-rate` 2026-10-21, 6.78 to 7.03 M (docs/changelog.md,
/// "Committed on 2026-10-21 (runtime calibration)"). The canned lattice
/// (240 k nodes, about 50 MB of maps) is beyond the caches, as a board's
/// is: the rate falls with the memory traffic of other jobs, 3.3 to 5.7 M
/// on eight cores at load 18.
pub const REFERENCE_CANNED_EXPANSIONS_PER_SECOND: f64 = 6.9e6;

/// The canned placement's units per second of wall clock with
/// `PLACER_COPIES` copies at once on the reference (all 32 threads, load
/// 12): 151 to 156 M (164 M on three pinned cores, 56 M on one), measured
/// with `REFERENCE_CANNED_EXPANSIONS_PER_SECOND`.
pub const REFERENCE_CANNED_PLACER_PER_SECOND: f64 = 155.0e6;

/// The placement copies run at once: the placement race's first batch
/// (`minimum_seeds`, three seeds that always run side by side as far as
/// the process's threads allow). The reference's clock counts them as
/// running at once (`REFERENCE_THREADS`), so a process with fewer threads
/// than this measures the placer's rate at the share a seed gets.
pub const PLACER_COPIES: usize = 3;

/// The canned search's work limit, in expansions (it ends after the
/// iteration that reaches it: about 600 k, 0.09 s on the reference), and
/// how often it runs (the fastest round counts: `fastest`).
const SEARCH_EXPANSIONS: u64 = 400_000;
const SEARCH_ROUNDS: usize = 5;
/// The canned placement's work limit, in units (about 0.1 s on the
/// reference, a third of it in the global placement, which runs
/// `PLACER_GLOBAL_ITERATIONS` only: the rest legalises and anneals), and
/// how often its copies run.
const PLACER_UNITS: u64 = 15_000_000;
const PLACER_GLOBAL_ITERATIONS: usize = 20;
const PLACER_ROUNDS: usize = 3;

/// What `measure` found.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadMeasuredRate {
    /// The search's rate this machine has against the reference's
    /// (`REFERENCE_EXPANSIONS_PER_SECOND` scaled by the canned search's).
    pub expansions_per_second: f64,
    /// The placer's (`REFERENCE_WORK_PER_SECOND` scaled by the canned
    /// placement's).
    pub placer_work_per_second: f64,
    /// The canned workloads' own rates.
    pub canned_expansions_per_second: f64,
    pub canned_placer_per_second: f64,
    /// The process's threads the placement copies ran on.
    pub threads: usize,
    /// The wall clock the measurement took.
    pub seconds: f64,
}

/// The rates a process applies, where they came from, and what the
/// calibration measured.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadWorkRate {
    /// Search expansions a second of work stands for.
    pub expansions_per_second: f64,
    /// Placement units a second of work stands for.
    pub placer_work_per_second: f64,
    /// `measured`, `PCB_WORK_RATE`, `config` (the router settings
    /// `work_rate` and `placer_work_rate`) or `reference` (nothing
    /// calibrated: a library call, a test).
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured: Option<KiCadMeasuredRate>,
}

impl KiCadWorkRate {
    /// The reference's rates, uncalibrated.
    pub fn reference() -> Self {
        Self {
            expansions_per_second: core::router::REFERENCE_EXPANSIONS_PER_SECOND,
            placer_work_per_second: placer::REFERENCE_WORK_PER_SECOND,
            source: "reference".into(),
            measured: None,
        }
    }

    /// How many times longer than the applied rates say this machine
    /// takes a second of work: the applied rate over the measured one where
    /// the rates are pinned (`PCB_WORK_RATE`, the router settings) faster
    /// than the machine measured at the start, at least 1 and at most
    /// `MAX_WALL_STRETCH` (`PCB_WALL_STRETCH_MAX`); 1 with the rates
    /// measured. The search's rate, and with `placement` the larger of it
    /// and the placer's. The wall-clock guards (route mode's
    /// `deadline_seconds`, the layout's `wall_guard`, `GUARD`) stretch by
    /// it: they are a margin for load and for what the work does not
    /// count, and a pinned rate the machine does not reach is neither (the
    /// tiers pin the reference's; at load 60 this machine measures 0.3 to
    /// 0.4 of it, and the 90 s guard of a 30 s budget cut every board that
    /// used its budget).
    pub fn wall_stretch(&self, placement: bool) -> f64 {
        let Some(measured) = &self.measured else {
            return 1.0;
        };
        if self.source == "measured" || self.source == "reference" {
            return 1.0;
        }
        let ratio = |applied: f64, measured: f64| {
            if measured > 0.0 && applied.is_finite() {
                applied / measured
            } else {
                1.0
            }
        };
        let search = ratio(self.expansions_per_second, measured.expansions_per_second);
        let stretch = if placement {
            search.max(ratio(
                self.placer_work_per_second,
                measured.placer_work_per_second,
            ))
        } else {
            search
        };
        stretch.clamp(1.0, wall_stretch_cap())
    }

    /// One line for the log.
    pub fn line(&self) -> String {
        let measured = match &self.measured {
            Some(measured) => format!(
                "; measured {:.4e} expansions/s, {:.4e} placer units/s in {:.2} s (canned search {:.4e}/s on one thread, canned placement {:.4e}/s with {PLACER_COPIES} copies on {} threads; {:.3} and {:.3} of the reference)",
                measured.expansions_per_second,
                measured.placer_work_per_second,
                measured.seconds,
                measured.canned_expansions_per_second,
                measured.canned_placer_per_second,
                measured.threads,
                measured.expansions_per_second / core::router::REFERENCE_EXPANSIONS_PER_SECOND,
                measured.placer_work_per_second / placer::REFERENCE_WORK_PER_SECOND,
            ),
            None => String::new(),
        };
        let stretch = if self.wall_stretch(true) > 1.0 {
            format!(
                "; the wall-clock guards stretch {:.2} times (a layout's {:.2}: the pinned rate over the measured, at most {})",
                self.wall_stretch(false),
                self.wall_stretch(true),
                wall_stretch_cap()
            )
        } else {
            String::new()
        };
        format!(
            "work rate: {:.4e} expansions/s, {:.4e} placer units/s ({}){measured}{stretch}",
            self.expansions_per_second, self.placer_work_per_second, self.source
        )
    }
}

/// Reads a pinned rate: `<expansions per second>` or `<expansions per
/// second>,<placer units per second>` (`3.15e6,150e6` is the reference's).
pub fn parse_work_rate(text: &str) -> Result<(f64, Option<f64>), String> {
    let number = |part: &str| -> Result<f64, String> {
        let value: f64 = part
            .trim()
            .parse()
            .map_err(|_| format!("{WORK_RATE_VARIABLE}: {part:?} is not a number"))?;
        if value.is_finite() && value > 0.0 {
            Ok(value)
        } else {
            Err(format!(
                "{WORK_RATE_VARIABLE}: {part:?} is not a positive rate"
            ))
        }
    };
    let mut parts = text.split(',');
    let expansions = number(parts.next().unwrap_or(""))?;
    let placer = parts.next().map(number).transpose()?;
    if parts.next().is_some() {
        return Err(format!(
            "{WORK_RATE_VARIABLE}: {text:?}: at most two rates (expansions and placer units a second)"
        ));
    }
    Ok((expansions, placer))
}

/// The placer's rate for a pinned search rate without one: the
/// reference's, scaled as the search's is.
fn placer_rate_for(expansions: f64, placer: Option<f64>) -> f64 {
    placer.unwrap_or(
        placer::REFERENCE_WORK_PER_SECOND * expansions
            / core::router::REFERENCE_EXPANSIONS_PER_SECOND,
    )
}

/// The rates to apply: `PCB_WORK_RATE` (`variable`) first, then the router
/// settings (`config`), then the measurement, else the reference's.
pub(crate) fn resolve(
    variable: Option<&str>,
    config: Option<(f64, Option<f64>)>,
    measured: Option<KiCadMeasuredRate>,
) -> Result<KiCadWorkRate, String> {
    let pinned = match variable {
        Some(text) => Some((parse_work_rate(text)?, WORK_RATE_VARIABLE)),
        None => match config {
            Some((expansions, placer)) => {
                let valid = |rate: f64| rate.is_finite() && rate > 0.0;
                if !valid(expansions) || !placer.is_none_or(valid) {
                    return Err(format!(
                        "work_rate {expansions} and placer_work_rate {placer:?} must be positive rates"
                    ));
                }
                Some(((expansions, placer), "config"))
            }
            None => None,
        },
    };
    Ok(match (pinned, measured) {
        (Some(((expansions, placer), source)), measured) => KiCadWorkRate {
            expansions_per_second: expansions,
            placer_work_per_second: placer_rate_for(expansions, placer),
            source: source.into(),
            measured,
        },
        (None, Some(measured)) => KiCadWorkRate {
            expansions_per_second: measured.expansions_per_second,
            placer_work_per_second: measured.placer_work_per_second,
            source: "measured".into(),
            measured: Some(measured),
        },
        (None, None) => KiCadWorkRate::reference(),
    })
}

static APPLIED: std::sync::OnceLock<KiCadWorkRate> = std::sync::OnceLock::new();

/// Measures this machine's rates and applies them, or the pinned ones
/// (`PCB_WORK_RATE`, else `config`: the router settings `work_rate` and
/// `placer_work_rate`), for the rest of the process, before any work.
/// Once per process: a later call returns what the first applied.
pub fn calibrate(config: Option<(f64, Option<f64>)>) -> Result<KiCadWorkRate, String> {
    if let Some(applied) = APPLIED.get() {
        return Ok(applied.clone());
    }
    let variable = std::env::var(WORK_RATE_VARIABLE).ok();
    // A bad pinned rate is the input's error, before a second is spent.
    resolve(variable.as_deref(), config, None).map_err(crate::verdict::input_error)?;
    let rate = resolve(variable.as_deref(), config, Some(measure()))
        .map_err(crate::verdict::input_error)?;
    core::router::set_expansions_per_second(rate.expansions_per_second);
    placer::set_work_per_second(rate.placer_work_per_second);
    core::router::set_wall_stretch(rate.wall_stretch(false));
    let applied = APPLIED.get_or_init(|| rate);
    eprintln!("{}", applied.line());
    Ok(applied.clone())
}

/// The rates this process applies (the reference's when nothing was
/// calibrated).
pub fn work_rate() -> KiCadWorkRate {
    APPLIED
        .get()
        .cloned()
        .unwrap_or_else(KiCadWorkRate::reference)
}

/// Measures the canned search on one thread and the canned placement in
/// `PLACER_COPIES` copies at once on the process's threads (about a second
/// on the reference), each the fastest of a few rounds. The workloads are fixed: their work is the same on
/// every machine, only their wall clock differs.
pub fn measure() -> KiCadMeasuredRate {
    let started = std::time::Instant::now();
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .expect("a thread for the calibration");
    let canned_search = one.install(|| {
        fastest(
            (0..SEARCH_ROUNDS)
                .map(|_| {
                    let mut router = core::router::Router::new(&canned_board(), &canned_config());
                    let before = router.expansions();
                    let clock = std::time::Instant::now();
                    router.probe(SEARCH_EXPANSIONS as f64 / core::router::expansions_per_second());
                    rate(router.expansions() - before, clock.elapsed().as_secs_f64())
                })
                .collect(),
        )
    });
    let threads = rayon::current_num_threads().max(1);
    let problem = canned_placement();
    let canned_placer = fastest(
        (0..PLACER_ROUNDS)
            .map(|_| {
                use rayon::prelude::*;
                let clock = std::time::Instant::now();
                let works: Vec<u64> = (0..PLACER_COPIES)
                    .into_par_iter()
                    .map(|_| placer::place(&problem, &canned_placer_config()).work)
                    .collect();
                // The copies' work over the wall clock of all of them: a
                // seed's rate in the race's first batch.
                rate(works[0], clock.elapsed().as_secs_f64())
            })
            .collect(),
    );
    KiCadMeasuredRate {
        expansions_per_second: core::router::REFERENCE_EXPANSIONS_PER_SECOND * canned_search
            / REFERENCE_CANNED_EXPANSIONS_PER_SECOND,
        placer_work_per_second: placer::REFERENCE_WORK_PER_SECOND * canned_placer
            / REFERENCE_CANNED_PLACER_PER_SECOND,
        canned_expansions_per_second: canned_search,
        canned_placer_per_second: canned_placer,
        threads,
        seconds: started.elapsed().as_secs_f64(),
    }
}

/// `MAX_WALL_STRETCH`, or `PCB_WALL_STRETCH_MAX` when it is a number of
/// at least 1.
fn wall_stretch_cap() -> f64 {
    std::env::var(WALL_STRETCH_VARIABLE)
        .ok()
        .and_then(|text| text.trim().parse::<f64>().ok())
        .filter(|cap| cap.is_finite() && *cap >= 1.0)
        .unwrap_or(MAX_WALL_STRETCH)
}

fn rate(work: u64, seconds: f64) -> f64 {
    work as f64 / seconds.max(1.0e-9)
}

/// The fastest of the rounds' rates: the machine's speed rather than a
/// moment's load. Another job's burst on the core or its sibling halves a
/// round's rate for a moment (the canned search pinned to one core at load
/// 16: 5.4, 5.0, 5.8 and 2.96 M a second, one process after the other);
/// the median of five rounds followed such bursts, and a layout's
/// minutes average them out.
fn fastest(values: Vec<f64>) -> f64 {
    values.into_iter().fold(0.0, f64::max)
}

/// A deterministic stream of numbers in [0, 1).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn canned_placer_config() -> placer::Config {
    let mut config = placer::Config::new();
    config.global.max_iterations = PLACER_GLOBAL_ITERATIONS;
    config.work_limit = Some(PLACER_UNITS);
    config
}

fn canned_config() -> core::Config {
    core::Config {
        pitches: vec![0.1],
        ..core::Config::default()
    }
}

/// The canned search's board: 40 x 30 mm, two layers, 0.1 mm lattice;
/// fourteen nets across and ten up and down between pads on the top
/// layer (every pair crosses: vias and negotiation), and keepouts in the
/// way.
fn canned_board() -> core::Board {
    use core::{Board, Net, Obstacle, ObstacleKind, RuleClass, Shape, Terminal};
    let mut random = Lcg(0x5eed_ca11_b4a7_e000);
    let mut obstacles = Vec::new();
    let mut nets = Vec::new();
    let pad = |obstacles: &mut Vec<Obstacle>, at: [f64; 2], net: usize, label: String| {
        obstacles.push(Obstacle {
            shape: Shape::Circle {
                center: at,
                radius: 0.35,
            },
            layers: 1,
            kind: ObstacleKind::Copper,
            net: Some(net as core::NetId),
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: label.clone(),
        });
        Terminal {
            anchor: at,
            layers: 1,
            pad: obstacles.len() - 1,
            contact: None,
            label,
        }
    };
    let mut ends = Vec::new();
    for row in 0..14 {
        let y = 2.5 + 1.8 * row as f64 + 0.4 * random.next();
        ends.push([[1.5, y], [38.5, 30.0 - y]]);
    }
    for column in 0..10 {
        let x = 4.0 + 3.3 * column as f64 + 0.4 * random.next();
        ends.push([[x, 1.2], [40.0 - x, 28.8]]);
    }
    for (net, [a, b]) in ends.into_iter().enumerate() {
        let terminals = vec![
            pad(&mut obstacles, a, net, format!("N{net}.1")),
            pad(&mut obstacles, b, net, format!("N{net}.2")),
        ];
        nets.push(Net {
            name: format!("N{net}"),
            class: 0,
            terminals,
        });
    }
    for index in 0..10 {
        let center = [6.0 + 28.0 * random.next(), 5.0 + 20.0 * random.next()];
        obstacles.push(Obstacle {
            shape: Shape::Circle {
                center,
                radius: 0.8 + 0.6 * random.next(),
            },
            layers: if index % 2 == 0 { 1 } else { 3 },
            kind: ObstacleKind::Keepout,
            net: None,
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: format!("K{index}"),
        });
    }
    Board {
        layer_count: 2,
        outline: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
        edge_clearance: 0.2,
        hole_clearance: 0.25,
        hole_to_hole: 0.25,
        classes: vec![RuleClass {
            trace_width: 0.2,
            clearance: 0.2,
            via_diameter: 0.6,
            via_drill: 0.3,
            layers: 0,
        }],
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
    }
}

/// The canned placement: two fixed connectors and forty two-pin parts in
/// a chain with cross links, piled in a corner of 60 x 40 mm.
fn canned_placement() -> placer::Problem {
    use placer::{Component, Pin, Pose, Problem, Side};
    let part = |name: String, size: [f64; 2], pins: Vec<Pin>, fixed: bool| Component {
        name,
        body_center: [0.0, 0.0],
        body_size: size,
        round: false,
        halo: 0.0,
        pins,
        side: if fixed { Side::Both } else { Side::Front },
        fixed,
        angle_options: if fixed {
            vec![0.0]
        } else {
            vec![0.0, 90.0, 180.0, 270.0]
        },
        far_side: Vec::new(),
        far_copper: Vec::new(),
        far_courtyard: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
    };
    let parts = 40;
    let pin = |offset: [f64; 2], net: usize| Pin { offset, net };
    let mut components = vec![
        part("J1".into(), [4.0, 10.0], vec![pin([0.0, 0.0], 0)], true),
        part("J2".into(), [4.0, 10.0], vec![pin([0.0, 0.0], parts)], true),
    ];
    let mut poses = vec![
        Pose {
            position: [3.0, 20.0],
            angle: 0.0,
        },
        Pose {
            position: [57.0, 20.0],
            angle: 0.0,
        },
    ];
    let mut random = Lcg(0x91ac_e5ee_d000_0001);
    for index in 0..parts {
        let mut pins = vec![pin([-1.5, 0.0], index), pin([1.5, 0.0], index + 1)];
        // A third pin on every fourth part links back across the chain.
        if index % 4 == 3 {
            pins.push(pin([0.0, 1.0], (index * 7) % parts));
        }
        components.push(part(format!("R{index}"), [4.0, 2.5], pins, false));
        poses.push(Pose {
            position: [8.0 + 4.0 * random.next(), 32.0 + 4.0 * random.next()],
            angle: 0.0,
        });
    }
    Problem {
        outline: vec![[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]],
        components,
        net_weights: vec![1.0; parts + 1],
        poses,
        spacing: 0.5,
        grid: 0.5,
        edge_margin: 0.0,
        min_spacing: 0.0,
        far_side_pads_only: false,
        shared_halos: false,
        escape: None,
        pieces: Vec::new(),
        constraints: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured() -> KiCadMeasuredRate {
        KiCadMeasuredRate {
            expansions_per_second: 2.5e6,
            placer_work_per_second: 100e6,
            canned_expansions_per_second: 3.0e6,
            canned_placer_per_second: 90e6,
            threads: 4,
            seconds: 1.2,
        }
    }

    #[test]
    fn a_pinned_rate_reads_one_or_two_numbers() {
        assert_eq!(parse_work_rate("3.15e6"), Ok((3.15e6, None)));
        assert_eq!(parse_work_rate("3.15e6,150e6"), Ok((3.15e6, Some(150.0e6))));
        assert_eq!(parse_work_rate(" 2e6 , 1e8 "), Ok((2.0e6, Some(1.0e8))));
        for bad in [
            "", "fast", "0", "-1e6", "inf", "NaN", "1e6,", "1e6,0", "1,2,3",
        ] {
            assert!(parse_work_rate(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_variable_wins_over_the_config_and_the_config_over_the_measurement() {
        let rate = resolve(Some("3.15e6,150e6"), Some((1.0e6, None)), Some(measured())).unwrap();
        assert_eq!(rate.expansions_per_second, 3.15e6);
        assert_eq!(rate.placer_work_per_second, 150.0e6);
        assert_eq!(rate.source, "PCB_WORK_RATE");
        // The measurement is still recorded beside a pinned rate.
        assert_eq!(rate.measured, Some(measured()));
        let rate = resolve(None, Some((1.0e6, Some(7.0e7))), Some(measured())).unwrap();
        assert_eq!(
            (rate.expansions_per_second, rate.placer_work_per_second),
            (1.0e6, 7.0e7)
        );
        assert_eq!(rate.source, "config");
        let rate = resolve(None, None, Some(measured())).unwrap();
        assert_eq!(
            (rate.expansions_per_second, rate.placer_work_per_second),
            (2.5e6, 100e6)
        );
        assert_eq!(rate.source, "measured");
        assert_eq!(
            resolve(None, None, None).unwrap(),
            KiCadWorkRate::reference()
        );
        assert!(resolve(Some("slow"), None, None).is_err());
        assert!(resolve(None, Some((0.0, None)), None).is_err());
        assert!(resolve(None, Some((1.0e6, Some(-1.0))), None).is_err());
    }

    #[test]
    fn the_guards_stretch_for_a_pinned_rate_faster_than_the_machine() {
        // Measured 2.5e6 and 100e6 against the reference's 3.15e6 and 150e6.
        let pinned = resolve(Some("3.15e6,150e6"), None, Some(measured())).unwrap();
        assert!((pinned.wall_stretch(false) - 1.26).abs() < 1e-9);
        assert!((pinned.wall_stretch(true) - 1.5).abs() < 1e-9);
        // A machine at a tenth: at most MAX_WALL_STRETCH.
        let slow = KiCadMeasuredRate {
            expansions_per_second: 3.15e5,
            ..measured()
        };
        let pinned = resolve(Some("3.15e6"), None, Some(slow)).unwrap();
        assert_eq!(pinned.wall_stretch(false), MAX_WALL_STRETCH);
        // A pinned rate slower than the machine, the rates measured, or
        // nothing measured: no stretch.
        let pinned = resolve(Some("1e6,50e6"), None, Some(measured())).unwrap();
        assert_eq!(pinned.wall_stretch(true), 1.0);
        let applied = resolve(None, None, Some(measured())).unwrap();
        assert_eq!(applied.wall_stretch(true), 1.0);
        assert_eq!(KiCadWorkRate::reference().wall_stretch(true), 1.0);
    }

    #[test]
    fn a_search_rate_alone_scales_the_placer_rate_alike() {
        // Half the reference's search rate: half its placer rate.
        let rate = resolve(Some("1.575e6"), None, None).unwrap();
        assert_eq!(rate.placer_work_per_second, 75.0e6);
        // The reference's search rate alone is the reference exactly.
        let rate = resolve(Some("3.15e6"), None, None).unwrap();
        assert_eq!(
            rate.placer_work_per_second,
            placer::REFERENCE_WORK_PER_SECOND
        );
    }

    #[test]
    fn the_canned_workloads_are_fixed() {
        // The same work on every run (only the wall clock may differ), so
        // that a rate measures the machine and nothing else.
        let run = || {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .unwrap();
            pool.install(|| {
                let mut router = core::router::Router::new(&canned_board(), &canned_config());
                router.probe(
                    SEARCH_EXPANSIONS as f64 / core::router::REFERENCE_EXPANSIONS_PER_SECOND,
                );
                (
                    router.expansions(),
                    placer::place(&canned_placement(), &canned_placer_config()).work,
                )
            })
        };
        let first = run();
        assert!(first.0 >= SEARCH_EXPANSIONS, "{first:?}");
        assert!(first.1 >= PLACER_UNITS, "{first:?}");
        assert_eq!(run(), first);
    }

    #[test]
    fn the_line_names_the_rates_and_their_source() {
        let rate = resolve(None, None, Some(measured())).unwrap();
        let line = rate.line();
        assert!(
            line.starts_with(
                "work rate: 2.5000e6 expansions/s, 1.0000e8 placer units/s (measured); measured"
            ),
            "{line}"
        );
        assert!(line.contains("0.794 and 0.667 of the reference"), "{line}");
        assert_eq!(
            KiCadWorkRate::reference().line(),
            "work rate: 3.1500e6 expansions/s, 1.5000e8 placer units/s (reference)"
        );
    }
}

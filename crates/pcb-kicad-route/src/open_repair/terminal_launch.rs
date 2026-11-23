// Copyright (C) 2026 Florian Loitsch.

//! Retry a fixed placement with early access only on diagnosed open pads.

use super::plane_escape::geometry::{Pad, contact, contact_on_net, pads};
use super::plane_escape::{open_count, rejection, snapshot};
use super::*;
use pcb_router::Point;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

mod detour;
mod row;
mod tips;

pub(crate) use row::plan_row_escapes;
pub use row::{PreparedRowEscape, PreparedRowEscapeAdvice, prepared_row_escape_advice};

#[derive(Clone, Debug, Serialize)]
pub struct TerminalLaunch {
    pub pad: String,
    pub net: String,
    pub layer: String,
    pub start_mm: Point,
    pub end_mm: Point,
    pub width_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalLaunchAdvice {
    pub board_sha256: String,
    pub layout_config_patch: BTreeMap<&'static str, Vec<String>>,
    pub required_cli_args: [&'static str; 2],
    pub minimum_reserve_seconds_to_start: u64,
    pub proposed_launches: Vec<TerminalLaunch>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalLocalTrial {
    pub net: String,
    pub opens_before: usize,
    pub opens_after: usize,
    pub decision: &'static str,
    pub report: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalLaunchRetry {
    pub status: &'static str,
    pub reason: String,
    pub opens_before: usize,
    pub opens_after: usize,
    pub budget_seconds: f64,
    pub work_seconds: f64,
    pub reroute_work_seconds: f64,
    pub native_checks: usize,
    pub reroute_native_checks: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub requested_pads: Vec<String>,
    pub launches: Vec<TerminalLaunch>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub local_trials: Vec<TerminalLocalTrial>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_detour: Option<detour::BlockingTrackDetour>,
    #[serde(skip)]
    pub accepted: Option<(PathBuf, KiCadBoardRouterResult)>,
}

impl TerminalLaunchRetry {
    pub fn skipped(budget_seconds: f64, reason: &str, requested: &[String]) -> Self {
        Self {
            status: "skipped",
            reason: reason.into(),
            opens_before: 0,
            opens_after: 0,
            budget_seconds,
            work_seconds: 0.0,
            reroute_work_seconds: 0.0,
            native_checks: 0,
            reroute_native_checks: 0,
            requested_pads: requested.to_vec(),
            launches: Vec::new(),
            local_trials: Vec::new(),
            track_detour: None,
            accepted: None,
        }
    }

    fn check(&mut self, board: &Path, report: &Path, check_seconds: f64) -> Result<Value, String> {
        if self.work_seconds + check_seconds > self.budget_seconds {
            return Err("remaining retry work does not cover a native check".into());
        }
        self.native_checks += 1;
        self.work_seconds += check_seconds;
        native_check(board, report, true, true)
    }
}

const MAX_PADS: usize = 2;
const MIN_ROUTE_SECONDS: f64 = 40.0;
const REROUTE_CHECK_RESERVE: usize = 8;
const TRIAL_CHECKS: usize = 4 + REROUTE_CHECK_RESERVE;

fn native_pad_hits<'a>(
    before: &Value,
    pads: &'a [Pad],
    accepts: impl Fn(&Pad) -> bool,
) -> Option<Vec<&'a Pad>> {
    let hits: Vec<_> = before["unconnected_items"]
        .as_array()?
        .iter()
        .map(|finding| {
            let ids: BTreeSet<_> = finding["items"]
                .as_array()?
                .iter()
                .filter_map(|item| Some(item["uuid"].as_str()?.to_ascii_lowercase()))
                .collect();
            let mut matches = pads
                .iter()
                .filter(|pad| ids.contains(&pad.id) && accepts(pad));
            let pad = matches.next()?;
            matches.next().is_none().then_some(pad)
        })
        .collect::<Option<_>>()?;
    let first = hits.first()?;
    (hits.iter().all(|pad| pad.reference == first.reference)
        && hits
            .iter()
            .map(|pad| &pad.id)
            .collect::<BTreeSet<_>>()
            .len()
            == hits.len())
    .then_some(hits)
}

pub fn valid_terminal_launch_pads(requested: &[String]) -> bool {
    (1..=MAX_PADS).contains(&requested.len())
        && requested.iter().collect::<BTreeSet<_>>().len() == requested.len()
        && requested.iter().all(|key| {
            key.split_once(':')
                .is_some_and(|(reference, pad)| !reference.is_empty() && !pad.is_empty())
        })
}

fn outward(rows: &[Point], center: Point, pitch: f64) -> Option<Point> {
    let coordinates = rows
        .iter()
        .map(|row| row.map(|value| (1.0e6 * value).round() as i64))
        .collect::<BTreeSet<_>>();
    let x = coordinates
        .iter()
        .map(|row| row[0])
        .collect::<BTreeSet<_>>();
    let y = coordinates
        .iter()
        .map(|row| row[1])
        .collect::<BTreeSet<_>>();
    let axes = [&x, &y];
    let across = if x.len() == 2 && y.len() >= 3 {
        0
    } else if y.len() == 2 && x.len() >= 3 {
        1
    } else {
        return None;
    };
    if coordinates.len() != rows.len() || x.len() * y.len() != rows.len() {
        return None;
    }
    let along = 1 - across;
    let coordinate = center.map(|value| (1.0e6 * value).round() as i64);
    let column_separation = (*axes[across].last()? - *axes[across].first()?) as f64 / 1.0e6;
    let row_span = (*axes[along].last()? - *axes[along].first()?) as f64 / 1.0e6;
    if !(0.4..=1.5).contains(&column_separation) || row_span > 3.0 {
        return None;
    }
    // End beyond a row separation, with half a cell of room for landing.
    // At the row's ends there is a free outward direction along the row.
    let axis = if [axes[along].first(), axes[along].last()].contains(&Some(&coordinate[along])) {
        along
    } else {
        across
    };
    let sign = if Some(&coordinate[axis]) == axes[axis].last() {
        1.0
    } else {
        -1.0
    };
    let mut tip = center;
    tip[axis] += sign * (column_separation + pitch / 2.0);
    Some(tip)
}

fn plan(
    initial: &Expr,
    before: &Value,
    diagnosed: &[board_router::KiCadDeadPad],
    config: &KiCadBoardRouterConfig,
    requested: &[String],
) -> Result<(Vec<TerminalLaunch>, Vec<Expr>), String> {
    let explicit = !requested.is_empty();
    let mut rules = config.clone();
    rules.bga_fanout = Some(false);
    rules.via_in_pad = Some(KiCadViaInPad::All(false));
    rules.apply_width_floor();
    let lower = board_router::lower(initial, &rules, true)?;
    let board = &lower.board;
    let names = crate::kicad_nets::note_net_names(initial);
    let lookup = |pad: &Pad| {
        if explicit {
            contact_on_net(board, &pad.label, names.normalize(&pad.net))
        } else {
            contact(board, pad)
        }
    };
    let pads = pads(initial);
    let explicit_ids: BTreeSet<_> = requested
        .iter()
        .filter_map(|key| {
            let (reference, pin) = key.split_once(':')?;
            let mut matches = pads.iter().filter(|pad| {
                pad.reference == reference
                    && pad
                        .label
                        .strip_prefix(reference)
                        .and_then(|name| name.strip_prefix('.'))
                        == Some(pin)
            });
            let first = matches.next()?;
            matches.next().is_none().then_some(first.id.as_str())
        })
        .collect();
    if explicit_ids.len() != requested.len() {
        return Ok((Vec::new(), Vec::new()));
    }
    let write_net = net_codes(initial);
    let Some(hits) = native_pad_hits(before, &pads, |pad| {
        if explicit {
            explicit_ids.contains(pad.id.as_str())
        } else {
            diagnosed
                .iter()
                .any(|dead| dead.pad == pad.label && dead.net == pad.net)
        }
    }) else {
        return Ok((Vec::new(), Vec::new()));
    };
    if explicit && hits.len() != requested.len() {
        return Ok((Vec::new(), Vec::new()));
    }
    let first = hits[0];
    let Some((_, anchor)) = lookup(first) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let row: Vec<_> = pads
        .iter()
        .filter(|pad| pad.reference == first.reference)
        .filter_map(lookup)
        .filter(|(_, terminal)| terminal.layers == anchor.layers)
        .map(|(_, terminal)| terminal.anchor)
        .collect();
    let mut launches = Vec::new();
    let mut additions = Vec::new();
    for pad in hits {
        let start = || {
            let (index, terminal) = lookup(pad)?;
            let layer = terminal.layers;
            if layer != anchor.layers {
                return None;
            }
            let class = board.classes[board.nets[index].class];
            if class.layers != 0 && class.layers & layer == 0 {
                return None;
            }
            let width = board.neck_width.min(class.trace_width);
            if width <= 0.0 || !width.is_finite() {
                return None;
            }
            let start = terminal.anchor;
            let end = if explicit {
                tips::outward_tip(initial, board, terminal, pad, width, config, &launches)?
            } else {
                let pitch = config
                    .tuning
                    .pitches
                    .as_ref()
                    .and_then(|pitches| pitches.first())
                    .copied()
                    .unwrap_or(0.1);
                outward(&row, start, pitch)?
            };
            let layer = if layer == 1 { "F.Cu" } else { "B.Cu" };
            Some(TerminalLaunch {
                pad: pad.label.clone(),
                net: pad.net.clone(),
                layer: layer.into(),
                start_mm: start,
                end_mm: end,
                width_mm: width,
            })
        };
        let Some(launch) = start() else {
            return Ok((Vec::new(), Vec::new()));
        };
        let [ax, ay] = launch.start_mm;
        let [bx, by] = launch.end_mm;
        let layer = quote(&launch.layer);
        let width = launch.width_mm;
        let net = encode(&source_net_field(&write_net, &pad.net));
        let fields = format!(
            "(start {ax:.6} {ay:.6}) (end {bx:.6} {by:.6}) (width {width:.6}) (layer {layer}) {net}"
        );
        let id = deterministic_uuid_for(&format!("terminal launch {} {fields}", pad.id));
        additions.push(parse(&format!("(segment {fields} (uuid \"{id}\"))"))?);
        launches.push(launch);
    }
    Ok((launches, additions))
}

fn native_seed_ok(before: &Value, mut seeded: Value, added: &[Expr]) -> Result<bool, String> {
    if !before["schematic_parity"]
        .as_array()
        .is_some_and(Vec::is_empty)
        || !seeded["schematic_parity"]
            .as_array()
            .is_some_and(Vec::is_empty)
        || !same_check_policy(before, &seeded)
        || !finding_types_at_cap(before, "violations").is_empty()
        || !finding_types_at_cap(&seeded, "violations").is_empty()
    {
        return Ok(false);
    }
    let ids: BTreeSet<_> = added.iter().filter_map(item_id).collect();
    if let Some(findings) = seeded["violations"].as_array_mut() {
        findings.retain(|finding| {
            let Some([item]) = finding["items"].as_array().map(Vec::as_slice) else {
                return true;
            };
            !(finding["type"] == "track_dangling"
                && finding["severity"] == "warning"
                && item["uuid"].as_str().is_some_and(|id| ids.contains(id)))
        });
    }
    Ok(!adds_findings(before, &seeded, "violations")?)
}

fn bare_placement(mut board: Expr) -> Expr {
    if let Expr::List(children) = &mut board {
        children.retain(|item| !route_item(item));
        for item in children
            .iter_mut()
            .filter(|item| item.head() == Some("zone"))
        {
            if let Expr::List(forms) = item {
                forms.retain(|form| {
                    !matches!(
                        form.head(),
                        Some("filled_polygon" | "fill_segments" | "filled_areas_thickness")
                    )
                });
            }
        }
    }
    board
}

pub fn advise_terminal_launches(
    placed_board: &Path,
    selected_board: &Path,
    native: &verification::VerificationReport,
    diagnosed: &[board_router::KiCadDeadPad],
    config: &KiCadBoardRouterConfig,
    check_seconds: f64,
) -> Option<TerminalLaunchAdvice> {
    if diagnosed.is_empty()
        || !native.schematic_parity_checked
        || native.schematic_parity_issues != 0
        || !(1..=MAX_PADS).contains(&native.selected_net_unconnected_items)
    {
        return None;
    }
    let before =
        native_report::read_kicad_report(&selected_board.with_file_name("drc.json")).ok()?;
    if report_capped(&before)
        || before["coordinate_units"] != "mm"
        || open_count(&before) != native.selected_net_unconnected_items
        || !before["schematic_parity"]
            .as_array()
            .is_some_and(Vec::is_empty)
        || !selected_board.with_extension("kicad_sch").is_file()
    {
        return None;
    }
    let (_, placed) = read_board(placed_board).ok()?;
    let (text, selected) = read_board(selected_board).ok()?;
    if placed.children().iter().any(route_item) || Pieces::of_board(&selected).ok()?.is_some() {
        return None;
    }
    let initial = bare_placement(selected);
    let names = crate::kicad_nets::note_net_names(&initial);
    let pads = pads(&initial);
    let mut requested: Vec<_> = native_pad_hits(&before, &pads, |pad| {
        diagnosed.iter().any(|dead| {
            dead.pad == pad.label && names.normalize(&pad.net) == names.normalize(&dead.net)
        })
    })?
    .into_iter()
    .filter_map(|pad| {
        let pin = pad.label.strip_prefix(&pad.reference)?.strip_prefix('.')?;
        Some(format!("{}:{pin}", pad.reference))
    })
    .collect();
    requested.sort();
    let (launches, _) = plan(&initial, &before, diagnosed, config, &requested).ok()?;
    if launches.len() != native.selected_net_unconnected_items {
        return None;
    }
    // This covers the required checks and the first detailed routing step.
    // A useful reroute can require more work than the startup threshold.
    let minimum = (TRIAL_CHECKS + 2) as f64 * check_seconds
        + MIN_ROUTE_SECONDS
        + 4.0
        + board_router::lowering_work(&initial);
    Some(TerminalLaunchAdvice {
        board_sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
        layout_config_patch: BTreeMap::from([("terminal_launch_pads", requested)]),
        required_cli_args: ["--terminal-launch-budget", "SECONDS"],
        minimum_reserve_seconds_to_start: minimum.ceil() as u64,
        proposed_launches: launches,
    })
}

struct Trial<'a> {
    selected: &'a Path,
    placed_board: &'a Path,
    output: &'a Path,
    board_id: &'a str,
    incumbent: &'a KiCadBoardRouterResult,
    config: &'a KiCadBoardRouterConfig,
    check_seconds: f64,
    requested: &'a [String],
}

#[allow(
    clippy::too_many_arguments,
    reason = "reuse the layout's work and native check costs"
)]
pub fn retry_terminal_launches(
    selected: &Path,
    placed_board: &Path,
    output: &Path,
    board_id: &str,
    incumbent: &KiCadBoardRouterResult,
    config: &KiCadBoardRouterConfig,
    available: f64,
    check_seconds: f64,
    requested: &[String],
) -> TerminalLaunchRetry {
    let mut report =
        TerminalLaunchRetry::skipped(available, "no_native_blocked_terminal", requested);
    let trial = Trial {
        selected,
        placed_board,
        output,
        board_id,
        incumbent,
        config,
        check_seconds,
        requested,
    };
    if let Err(error) = trial.run(&mut report) {
        report.status = "failed";
        report.reason = error;
    }
    report
}

impl Trial<'_> {
    fn check(
        &self,
        report: &mut TerminalLaunchRetry,
        board: &Path,
        output: &Path,
    ) -> Result<Value, String> {
        if self.requested.is_empty() {
            return report.check(board, output, self.check_seconds);
        }
        if report.work_seconds + self.check_seconds > report.budget_seconds {
            return Err("remaining retry work does not cover a native check".into());
        }
        report.native_checks += 1;
        report.work_seconds += self.check_seconds;
        let limit = Instant::now() + Duration::from_secs(60);
        let deadline = self
            .config
            .deadline
            .map_or(limit, |deadline| deadline.min(limit));
        native_report::check_kept_route_input(board, output, true, true, || {
            Instant::now() >= deadline
        })
    }

    fn local_trial(
        &self,
        source: &Path,
        output: &Path,
        net: &str,
        config: &KiCadBoardRouterConfig,
        report: &mut TerminalLaunchRetry,
    ) -> Result<Option<usize>, String> {
        let mut budget = LayoutRepairBudget {
            router: config,
            available_seconds: report.budget_seconds - report.work_seconds - self.check_seconds,
            check_seconds: self.check_seconds,
            native_checks: 0,
            search_work_seconds: 0.0,
            branch_fill_reserve_checks: None,
        };
        if budget.search_limit() <= 0.0 || !budget.can_check_trial() {
            report.reason = "insufficient_remaining_work_after_reroute".into();
            return Ok(None);
        }
        let repair = repair_layout_opens_on_nets(
            source,
            self.board_id,
            output,
            &mut budget,
            &[net.to_string()],
        );
        report.native_checks += budget.native_checks;
        report.work_seconds += budget.work_seconds();
        let repair = repair?;
        let accepted = repair.status == "improved" && repair.opens_after < repair.opens_before;
        let decision = if accepted {
            "accepted"
        } else {
            repair
                .trials
                .first()
                .map_or(repair.status, |trial| trial.status)
        };
        let proof_path = output.join("open-repair.json");
        let relative_to = self.output.parent().unwrap_or_else(|| Path::new("."));
        report.local_trials.push(TerminalLocalTrial {
            net: net.to_string(),
            opens_before: repair.opens_before,
            opens_after: repair.opens_after,
            decision,
            report: proof_path
                .strip_prefix(relative_to)
                .unwrap_or(&proof_path)
                .into(),
        });
        if !accepted {
            report.reason = format!("local_route_rejected: {}", repair.note.unwrap_or(decision));
        }
        Ok(accepted.then_some(repair.opens_after))
    }

    fn run(&self, report: &mut TerminalLaunchRetry) -> Result<(), String> {
        let explicit = !self.requested.is_empty();
        if explicit && !valid_terminal_launch_pads(self.requested) {
            report.reason = "invalid_terminal_launch_pads".into();
            return Ok(());
        }
        let diagnosed = &self.incumbent.diagnostics.dead_pads;
        if explicit && let Some(native) = self.incumbent.native.as_ref() {
            report.opens_before = native.selected_net_unconnected_items;
            report.opens_after = report.opens_before;
        }
        let Some(native) = self.incumbent.native.as_ref().filter(|native| {
            (1..=MAX_PADS).contains(&native.selected_net_unconnected_items)
                && native.schematic_parity_issues == 0
        }) else {
            if explicit {
                report.reason =
                    "selected_tip_retry_requires_one_or_two_native_opens_and_clean_parity".into();
            }
            return Ok(());
        };
        report.opens_before = native.selected_net_unconnected_items;
        report.opens_after = report.opens_before;
        if explicit && self.requested.len() != report.opens_before {
            report.reason = "selected_pads_must_match_all_native_opens".into();
            return Ok(());
        }
        if !explicit && diagnosed.is_empty() {
            return Ok(());
        }
        // Both baselines, the seeded placement and the final check, plus
        // the reroute's own native checks and up to three fresh checks for
        // each local repair after the reroute.
        let checks = TRIAL_CHECKS + if explicit { 0 } else { 3 * report.opens_before };
        let reserved_checks = checks as f64 * self.check_seconds;
        let route_allowance = report.budget_seconds - reserved_checks - 4.0;
        if route_allowance < MIN_ROUTE_SECONDS {
            report.reason = "insufficient_remaining_work".into();
            return Ok(());
        }
        let filename = format!("{}.kicad_pcb", self.board_id);
        let (_, placed) = read_board(self.placed_board)?;
        if placed.children().iter().any(route_item) {
            report.reason = "placed_board_has_authored_routes".into();
            return Ok(());
        }
        let selected_board = self.selected.join(&filename);
        let (source, original) = read_board(&selected_board)?;
        if !selected_board.with_extension("kicad_sch").is_file() {
            report.reason = "requires_native_schematic_parity".into();
            return Ok(());
        }
        if Pieces::of_board(&original)?.is_some() {
            report.reason = "requires_single_board".into();
            return Ok(());
        }
        fs::create_dir(self.output).map_err(|error| error.to_string())?;
        let baseline = self.output.join("baseline");
        copy_directory_tree(self.selected, &baseline)?;
        let baseline = baseline.join(&filename);
        report.status = "no_improvement";
        let before = self.check(
            report,
            &baseline,
            &self.output.join("drc-before-launch.json"),
        )?;
        if report_capped(&before)
            || before["coordinate_units"] != "mm"
            || open_count(&before) != report.opens_before
            || !before["schematic_parity"]
                .as_array()
                .is_some_and(Vec::is_empty)
            || fs::read_to_string(&baseline).map_err(|error| error.to_string())? != source
        {
            report.reason = "baseline_native_attestation_failed".into();
            return Ok(());
        }
        // Keep the selected placement's labels, zones and pad-level
        // settings as well as its physical geometry. The verified retry
        // replaces only the copper generated during the first route.
        let initial = bare_placement(original.clone());
        let lowering_work = board_router::lowering_work(&initial);
        if explicit && lowering_work + MIN_ROUTE_SECONDS > route_allowance {
            report.reason = "insufficient_remaining_work_after_planning".into();
            return Ok(());
        }
        report.work_seconds += lowering_work;
        let (launches, additions) =
            plan(&initial, &before, diagnosed, self.config, self.requested)?;
        if additions.len() != report.opens_before {
            report.reason = if explicit {
                "selected_native_open_pads_do_not_have_compatible_free_tips"
            } else {
                "no_compatible_native_pad_rows"
            }
            .into();
            return Ok(());
        }
        report.launches = launches;
        let seeded = self.output.join("seed");
        copy_directory_tree(self.selected, &seeded)?;
        native_report::remove_stale_board_run_reports(&seeded)?;
        let input = seeded.join(&filename);
        let bare = encode(&initial);
        fs::write(&input, &bare).map_err(|error| error.to_string())?;
        let before_seed = self.check(
            report,
            &input,
            &self.output.join("drc-before-launch-seed.json"),
        )?;
        if fs::read_to_string(&input).map_err(|error| error.to_string())? != bare {
            report.reason = "native_check_changed_seed_baseline".into();
            return Ok(());
        }
        write_additions(&input, &bare, &additions)?;
        let with_launches = fs::read_to_string(&input).map_err(|error| error.to_string())?;
        let evidence = self.check(report, &input, &self.output.join("drc-launch-seed.json"))?;
        if fs::read_to_string(&input).map_err(|error| error.to_string())? != with_launches
            || !native_seed_ok(&before_seed, evidence, &additions)?
        {
            report.reason = "native_check_rejected_launches".into();
            return Ok(());
        }

        let mut config = self.config.clone();
        config.seeds = Some(1);
        config.existing_tracks = KiCadExistingTracks::Keep;
        if explicit {
            config.retained_pad_tips = self.requested.to_vec();
        }
        config.deadline_work = None;
        config.kicad_kept = None;
        // Keep the standalone reroute's probe schedule. Charge the checks
        // it actually runs to this stage's separate native reserve.
        config.kicad_check_work = Some(0.0);
        config.provisional_native_check_limit = Some(REROUTE_CHECK_RESERVE);
        config.native_candidate_ranker = None;
        let route_allowance = route_allowance - lowering_work;
        let nominal = route_allowance / 1.05;
        let nominal = if explicit { nominal } else { nominal.min(90.0) };
        config.ladder_budget_seconds = Some(
            config
                .ladder_budget_seconds
                .map_or(nominal, |cap| cap.min(nominal)),
        );
        config.ladder_work_ceiling = Some(route_allowance);
        let mut directory = self.output.join("route");
        let mut route = match route_kicad_board(&seeded, self.board_id, &directory, &config) {
            Ok(route) => route,
            Err(error) => {
                report.reroute_work_seconds =
                    route_allowance + REROUTE_CHECK_RESERVE as f64 * self.check_seconds;
                report.work_seconds += report.reroute_work_seconds;
                return Err(error);
            }
        };
        report.reroute_native_checks = route.kicad_checks_charged;
        let check_work = report.reroute_native_checks as f64 * self.check_seconds;
        route.kicad_check_work_seconds = check_work;
        report.reroute_work_seconds = route.work_seconds + check_work;
        report.work_seconds += report.reroute_work_seconds;
        let Some(native) = route.native.as_ref() else {
            report.reason = "reroute_native_attestation_missing".into();
            return Ok(());
        };
        let mut opens = native.selected_net_unconnected_items;
        if opens > report.opens_before {
            report.reason = "reroute_added_opens".into();
            return Ok(());
        }
        if explicit {
            let ids: BTreeSet<_> = additions.iter().filter_map(item_id).collect();
            let used = route
                .kept_copper
                .as_ref()
                .and_then(|kept| kept.retained_pad_tips.as_ref());
            if !used.is_some_and(|tips| {
                tips.used.len() == ids.len()
                    && tips.used.iter().all(|tip| ids.contains(&tip.track_uuid))
            }) {
                report.reason = "detailed_route_did_not_use_all_selected_free_tips".into();
                return Ok(());
            }
            if opens != 0 {
                report.reason = "selected_tip_reroute_left_native_opens".into();
                return Ok(());
            }
        }
        // Use the diagnosed net identities in the same stable order as
        // standalone local repair. Nearby endpoints of other open nets
        // can move when zone connectivity and native witness order change.
        let mut targets: Vec<_> = report
            .launches
            .iter()
            .map(|launch| launch.net.clone())
            .collect();
        targets.sort_unstable();
        for (index, net) in targets.iter().enumerate() {
            if opens == 0 {
                break;
            }
            let retry = self.output.join(format!("local-{}", index + 1));
            if let Some(after) = self.local_trial(&directory, &retry, net, &config, report)? {
                opens = after;
                directory = retry;
                route.model_precedes_additive_repair = true;
            }
        }
        if opens == 1
            && let Some(launch) = report
                .local_trials
                .iter()
                .find(|trial| trial.decision == "no_local_path")
                .and_then(|trial| {
                    report
                        .launches
                        .iter()
                        .find(|launch| launch.net == trial.net)
                })
                .cloned()
            && let Some(detoured) = detour::try_one(self, &directory, &launch, &config, report)?
        {
            let retry = self.output.join("detour-local");
            if let Some(after) =
                self.local_trial(&detoured, &retry, &launch.net, &config, report)?
            {
                opens = after;
                directory = retry;
                route.model_precedes_additive_repair = true;
            }
        }
        if opens > 0 {
            if report.reason == "no_native_blocked_terminal" {
                report.reason = "retry_left_native_opens".into();
            }
            return Ok(());
        }
        if self.check_seconds + report.work_seconds > report.budget_seconds {
            report.reason = "insufficient_remaining_work_for_final_check".into();
            return Ok(());
        }
        let candidate = directory.join(&filename);
        let (text, final_board) = read_board(&candidate)?;
        if snapshot(&final_board) != snapshot(&original) {
            report.reason = "retry_changed_placement_or_rules".into();
            return Ok(());
        }
        let final_path = self.output.join("drc-after-launch.json");
        let after = self.check(report, &candidate, &final_path)?;
        if fs::read_to_string(&candidate).map_err(|error| error.to_string())? != text {
            report.reason = "trial_native_attestation_failed".into();
            return Ok(());
        }
        if let Some(reason) = rejection(&before, &after)? {
            report.reason = reason.into();
            return Ok(());
        }
        if route.model_precedes_additive_repair {
            route.native = Some(verification::record_verified_additive_board(
                &directory,
                self.board_id,
                true,
            )?);
            route.saved_copper = Some(pcb_kicad_file::board_stats::inspect_kicad_saved_copper(
                &candidate,
            )?);
        }
        route.closed_reuse_drc = None;
        write_typed_json(&directory.join("board-router.json"), &route)?;
        report.accepted = Some((directory, route));
        report.opens_after = open_count(&after);
        report.reason = "all_opens_closed_without_new_native_findings".into();
        report.status = "accepted";
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axial_launches_follow_the_row_at_its_end_and_point_across_the_middle() {
        let pads = [
            [1.0, 1.0],
            [1.0, 1.35],
            [1.0, 1.7],
            [1.8, 1.0],
            [1.8, 1.35],
            [1.8, 1.7],
        ];
        assert_eq!(
            outward(&pads, pads[1], 0.1)
                .unwrap()
                .map(|v| (v * 100.0).round() as i32),
            [15, 135]
        );
        assert_eq!(
            outward(&pads, pads[5], 0.1)
                .unwrap()
                .map(|v| (v * 100.0).round() as i32),
            [180, 255]
        );
        assert!(outward(&pads[..5], pads[3], 0.1).is_none());
    }

    #[test]
    fn seed_checks_tolerate_only_the_provisional_launch_warning() {
        use serde_json::json;

        let added = parse(r#"(segment (uuid "launch"))"#).unwrap();
        let bare_zone_warning = json!({
            "type": "isolated_copper",
            "severity": "warning",
            "items": [{"uuid":"bare-zone"}]
        });
        let dangling = |uuid| {
            json!({
                "type": "track_dangling",
                "severity": "warning",
                "items": [{"uuid":uuid}]
            })
        };
        let bare = json!({"schematic_parity": [], "violations":[bare_zone_warning]});
        let with_launch = |uuid| {
            json!({
                "schematic_parity": [],
                "violations": [bare_zone_warning, dangling(uuid)]
            })
        };
        assert!(
            native_seed_ok(&bare, with_launch("launch"), std::slice::from_ref(&added)).unwrap()
        );
        assert!(
            !native_seed_ok(&bare, with_launch("other"), std::slice::from_ref(&added)).unwrap()
        );
        let routed = json!({"schematic_parity": [], "violations":[]});
        assert!(!native_seed_ok(&routed, with_launch("launch"), &[added]).unwrap());
    }

    #[test]
    fn explicit_exit_needs_the_selected_native_open_pad_and_clear_landing() {
        use crate::project_rules::{KiCadProjectMinimums, KiCadProjectRulesReport};
        use serde_json::json;

        let board = |obstacle| {
            parse(&format!(
                r#"(kicad_pcb (version 20241229)
                (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
                (gr_rect (start 0 0) (end 10 10) (stroke (width 0.05) (type solid))
                         (fill none) (layer "Edge.Cuts"))
                (footprint "T:one" (layer "F.Cu") (at 5 5 90) (property "Reference" "U1")
                    (pad "1" smd rect (at 1 0 90) (size 0.6 0.2) (layers "F.Cu")
                         (net "/signal") (uuid "11111111-1111-1111-1111-111111111111")))
                (footprint "T:two" (layer "F.Cu") (at 7 4) (property "Reference" "J1")
                    (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu")
                         (net "/signal") (uuid "22222222-2222-2222-2222-222222222222")))
                {obstacle})"#
            ))
            .unwrap()
        };
        let report = json!({"unconnected_items": [{"items": [
            {"uuid": "11111111-1111-1111-1111-111111111111"},
            {"uuid": "22222222-2222-2222-2222-222222222222"}
        ]}]});
        let config = KiCadBoardRouterConfig {
            default_rules: Some(crate::KiCadConnectionRoutingRules {
                trace_width_mm: 0.15,
                clearance_mm: 0.15,
                via_size_mm: 0.45,
                via_drill_mm: 0.2,
            }),
            project_rules: Some(KiCadProjectRulesReport {
                minimums: Some(KiCadProjectMinimums {
                    track_width_mm: 0.15,
                    track_width_source: String::new(),
                    clearance_mm: 0.15,
                    clearance_source: String::new(),
                }),
                via_minimums: [0.45, 0.2, 0.1],
                ..Default::default()
            }),
            ..Default::default()
        };
        let selected = vec!["U1:1".to_string()];
        let (launches, _) = plan(&board(""), &report, &[], &config, &selected).unwrap();
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].start_mm, [5.0, 4.0]);
        assert!(pcb_router::geometry::distance(launches[0].end_mm, [5.0, 3.5]) < 1e-6);
        let blocked = board(
            r#"(footprint "T:obstacle" (layer "B.Cu") (at 5 3) (property "Reference" "D1")
                (pad "1" smd rect (at 0 0) (size 0.7 2.0) (layers "B.Cu")))"#,
        );
        assert!(
            plan(&blocked, &report, &[], &config, &selected)
                .unwrap()
                .0
                .is_empty()
        );
        let wrong_open = json!({"unconnected_items":[{"items":[{"uuid":"another-pad"}]}]});
        assert!(
            plan(&board(""), &wrong_open, &[], &config, &selected)
                .unwrap()
                .0
                .is_empty()
        );
    }
}
